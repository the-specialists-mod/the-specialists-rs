# Client plugins: packages, downloads and approval

Status: draft specification, 2026-10-17. Nothing here is built yet.

The client library has no plugin loading today. The server loads `tsmod_*` libraries from
`dlls/` ([plugin-api.md](plugin-api.md) §1.2), but nothing equivalent exists on the client. This
document adds two things:

* **Client plugins:** native libraries that `client.dll` loads through the client extension API.
  That API is specified in `the-specialists-mod/tsmod-plugin-webview-rs` `docs/specifications.md`
  §4.1, and will move here when it's built.
* **Packages and approval:** how plugins are installed, downloaded from a server, and approved by
  the player. Every file of an approved plugin is hashed, and nothing loads unless every hash
  still matches.

The first plugin is the webview overlay (`tsmod-plugin-webview-rs`).

## 1. Goals and non-goals

Goals:

* **Nothing loads without the player's approval.** That applies to a plugin the player copied in
  by hand, one the launcher installed, and one a server offered.
* **An approval covers exact bytes.** It records a SHA-256 of every file in the plugin. If any
  file changes, disappears or appears, for any reason, the plugin doesn't load and the player is
  asked again, and told what changed.
* **Servers can offer plugins.** The player sees the details before anything is downloaded
  (name, publisher, version, size, signature, what it says it does) and approves or refuses.
  A server can mark a plugin as required; it can't install one.
* **Works on every client the port supports:** Steam HL25 and FWGS, Linux and Windows, 32- and
  64-bit.

Non-goals:

* **Sandboxing.** A plugin is native code running in the game process with the player's
  permissions. Approval is consent, not containment, and the prompts say so plainly. The
  manifest's `declares` (§3.2) is informational only.
* **Protection against malware already on the computer.** Something that can write to the game
  directory can also edit the approval store. The store protects against servers and against
  silent changes, not against a local attacker.
* **Unloading.** Once loaded, a plugin stays loaded until the game exits (§9).
* macOS.

## 2. Overview

```
        server (port)                              client (port client.dll)
┌──────────────────────────┐   TSPkg offer    ┌──────────────────────────────────────────┐
│ plugin_offers.toml       │ ───────────────► │ 1. fetch manifest + signature (KB)       │
│ sv_plugin_offer, API     │                  │ 2. offer card: details, size, signature  │
│                          │ ◄─────────────── │ 3. player approves → download package    │
│ FastDL / HTTPS host      │   tspkg <state>  │ 4. verify: signature, package hash,      │
│  plugins/<id>-<v>.tspkg  │ ◄── HTTP(S) ──── │    every file's hash                     │
└──────────────────────────┘                  │ 5. install to cl_plugins/<id>/<version>/ │
                                              │ 6. record approval with every file hash  │
                                              │ 7. load through the client extension API │
                                              └──────────────────────────────────────────┘
 At every start: hash every installed plugin's files and compare them with approved.toml.
 A match loads; anything else prompts and doesn't load until it's approved.
```

## 3. Packages

### 3.1 Format

A package is a zip file named `<id>-<version>.tspkg`, holding:

```
manifest.toml
manifest.sig          ed25519 signature of manifest.toml's exact bytes (absent when unsigned)
files/...             the plugin's files, paths as in the manifest
```

* The manifest and its signature are also served next to the package, as
  `<id>-<version>.manifest.toml` and `.manifest.sig`, so the client can show details before it
  downloads anything (§6.2).
* The `.tspkg` extension isn't on either engine's list of blocked download extensions. The port
  downloads packages itself, though, not through the engine (§6.3).

### 3.2 Manifest

```toml
format = 1
id = "webview"                     # [a-z0-9_-]{1,32}; the library is tsmod_<id><suffix>
name = "Web overlay"
version = "0.1.0"                  # semver
description = "A Chromium overlay that servers and the player can open."
homepage = "https://github.com/the-specialists-mod/tsmod-plugin-webview-rs"
license = "MIT"
client_ext_api = 1                 # the extension API version it was built for (exact)

[publisher]
name = "The Specialists Mod"
key = "ed25519:BASE64..."          # the key manifest.sig verifies with

[platforms]                         # the library to load per client platform
linux-i686    = "lib/linux-i686/tsmod_webview.so"
linux-x86_64  = "lib/linux-x86_64/tsmod_webview.so"
windows-i686  = "lib/windows-i686/tsmod_webview.dll"
windows-x86_64 = "lib/windows-x86_64/tsmod_webview.dll"

[declares]                          # informational; shown on the card, not enforced
processes = ["bin/tsmod-webview-host"]   # executables the plugin starts
network = true
data = true                         # writes to its data directory (§4)

[[files]]                           # every file in the package, no exceptions
path = "lib/linux-x86_64/tsmod_webview.so"
size = 2412544
sha256 = "9f2c..."
platform = "linux-x86_64"           # optional: only installed on that platform
# ...
```

* **Every file is listed.** A file in `files/` without an entry, or an entry without a file,
  rejects the package.
* **Paths** are relative, use `/`, and may not contain `..`, a drive letter, or a leading `/`.
  Symlinks and other special zip entries reject the package.
* **Platforms:** a package that has no library for the client's platform is reported as
  `unsupported` (§6.5) and is never offered to the player.

### 3.3 Signatures and trust

| Level | Meaning | Shown as |
|---|---|---|
| **Verified** | `manifest.sig` checks against `publisher.key`, and that key is countersigned by a key built into the port (the-specialists-mod's release key; the list is `crates/client/src/plugins/trust.rs`) | "Verified publisher: The Specialists Mod" |
| **Known publisher** | the signature checks, and the player has approved a plugin with this key before | "Signed by <name>, approved before on <date>" |
| **New publisher** | the signature checks, the key is new | "Signed by an unknown publisher" plus the key's fingerprint |
| **Unsigned** | no `manifest.sig`, or a library dropped in with no manifest (§5.3) | red: "Unsigned: nobody vouches for this file" |

* Countersignatures ship as `publisher.cert` next to `manifest.sig`, signed by a built-in key.
* **Key changes:** a plugin id that was approved under one key and now comes with another is
  treated as a different plugin, with a warning that the publisher changed.
* **Unsigned code:** unsigned packages from servers are refused unless the player has set
  `cl_plugins_allow_unsigned 1`. Hand-installed unsigned libraries can still be approved through
  the start-up prompt, with the red warning (that's how developers work, §11).

## 4. Install layout (client)

```
ts/cl_plugins/approved.toml                 the approval store (§5)
ts/cl_plugins/<id>/<version>/               an installed package: manifest.toml, manifest.sig, files
ts/cl_plugins/staging/                      downloads in progress; never loaded
ts/cl_plugins_data/<id>/                    the plugin's own writable data (caches, profiles, settings)
ts/cl_dlls/tsmod_<name><suffix>             hand-installed libraries without a package (§5.3)
```

* A plugin must write only to its data directory, never to its package directory. Writing there
  changes a hash and triggers a prompt at the next start, so this is self-enforcing.
* One version of a plugin is active at a time. Older version directories are kept until the new
  one is approved, then deleted.

## 5. Approval store and verification

### 5.1 `approved.toml`

```toml
format = 1

[[plugin]]
id = "webview"
version = "0.1.0"
state = "allowed"                 # allowed | blocked
scope = "always"                  # always | server
servers = ["203.0.113.5:27015"]   # for scope = "server"
source = "server 203.0.113.5:27015"   # or "launcher", "manual"
publisher_key = "ed25519:BASE64..."   # absent for unsigned
trust = "verified"                # as shown at approval time (§3.3)
approved_at = "2026-10-17T10:42:00Z"
manifest_sha256 = "41ab..."
platform = "linux-x86_64"

[plugin.files]                    # every file of the installed package, for this platform
"manifest.toml" = "41ab..."
"lib/linux-x86_64/tsmod_webview.so" = "9f2c..."
"bin/tsmod-webview-host" = "77d0..."
# ... every other file
```

* The store is written only by the approval dialogs (§7) and the launcher. Nothing a server
  sends can write it.
* `state = "blocked"` remembers a "Never" choice, so the plugin isn't offered or prompted for
  again. `cl_plugins_menu` (§10) can undo it.
* The store is written atomically (write to a temporary file, then rename). A store that fails
  to parse is set aside as `approved.toml.bad`, and every plugin is prompted for again.

### 5.2 Verification at every start

At `Initialize`, before any plugin is mapped:

1. List the candidates: every `cl_plugins/<id>/<version>/` with a manifest, and every
   `cl_dlls/tsmod_*<suffix>` (§5.3).
2. For each, hash every file under its package directory, in parallel on worker threads.
3. Compare the results with the store. The plugin loads only if all of these hold:
   * an entry with `state = "allowed"` exists for this id, version and platform;
   * the set of files is exactly the set in the entry (no missing files, no extra files);
   * every hash matches, and `manifest_sha256` matches;
   * the signature still verifies, and the key is still the entry's `publisher_key`.
4. Anything else is **changed**, and the plugin doesn't load; it goes to the start-up prompt
   (§7.1). Changed means:
   * a file's contents differ;
   * a file is missing, or there's an extra file;
   * the manifest changed;
   * a different version or key;
   * a plugin seen for the first time.

* **Time budget:** SHA-256 runs at about 1–2 GB/s per core with SHA extensions. The webview
  package (about 300 MB unpacked, mostly CEF) takes about 0.2 s on four threads. The port logs
  the time, and `cl_plugins_verify_log 1` logs each file.
* **No caching on size or mtime.** Every file is hashed in full at every start, because a cache
  keyed on metadata would miss a change that preserved it.
* **Executables started later:** a plugin that starts a helper process does it through the
  extension API's `verified_spawn(path, args)`. That call re-hashes the file against the
  approval immediately before starting it, and refuses if the file has changed or isn't listed.
  The webview plugin's `tsmod-webview-host` and CEF's subprocesses (the same executable) start
  this way. Libraries the helper loads itself (`libcef`) were hashed at start; re-checking them
  is open question 5.

### 5.3 Hand-installed libraries

A `cl_dlls/tsmod_<name><suffix>` with no package is treated as a one-file unsigned plugin:
- **id:** `manual:<name>`
- **files:** the library alone, plus any `tsmod_<name>.files` list next to it naming more files

It goes through the start-up prompt with the unsigned warning. The approval covers that one hash,
so every rebuild prompts again. Developers can pre-approve a build instead (§11).

## 6. Server offers

### 6.1 Server side

* **`ts/plugin_offers.toml`** in the server's game directory:

  ```toml
  [[offer]]
  id = "webview"
  version = "0.1.0"
  url = "plugins/webview-0.1.0.tspkg"   # relative to sv_downloadurl, or an absolute https:// URL
  sha256 = "c3d1..."                    # of the .tspkg
  size = 131072000
  required = false                      # true: the server may kick players who refuse (below)
  note = "Needed for the stats and vote pages."   # ≤ 64 bytes, shown on the card
  ```

  `xtask` gets `cargo xtask plugin-offer <tspkg>`, which prints the entry for a built package.
* **When offers are sent:** on `ClientPutInServer`, only to port clients (§12). Offers also go
  out on demand, with `sv_plugin_offer <#userid|name|all> <id>` or the plugin API's
  `offer_client_plugin(edict, id)` (the next plugin-api version; tsmod-amxx-rs can expose it as a
  native).
* **Required plugins:** with `sv_plugin_required_kick 1` (default 0), a player who answers
  `denied`, `unsupported` or `failed` to a required offer is kicked. The kick reason names the
  plugin and its homepage. Without it, the plugin API reports the state and plugins decide.
* **What the server learns:** the client's state for each offer, through `tspkg` (§6.5) and the
  plugin API event `client_plugin_state(edict, id, state)`.

### 6.2 The `TSPkg` user message

The port registers `TSPkg` after TS's 64 messages, so TS's registration order is unchanged
(design.md §2).

```
byte    op            1 offer, 2 withdraw
string  id            ≤ 32
string  version       ≤ 16
long    size          package bytes
byte×32 sha256        of the .tspkg
byte    flags         1 required
string  url           ≤ 96; relative to sv_downloadurl, or absolute https://
string  note          ≤ 64
```

That is at most 249 bytes, over GoldSrc's 192-byte user message limit. So `note` is sent only
when `url` is relative and short enough; the client checks the total fits. Open question 1
covers chunking it instead.

### 6.3 Client flow

1. **Receive the offer.** The client checks it first:
   * it ignores offers when `cl_plugins_download 0`;
   * it ignores offers for an id blocked in the store, for this server or always;
   * it answers `have` when the same id, version and package hash is already approved, and loads
     it if it isn't loaded yet (scope permitting).
2. **Fetch the manifest** and its signature (`<url without .tspkg>.manifest.toml` / `.sig`, at
   most 64 KB), on a worker thread. Then check:
   * the signature (§3.3);
   * that the id and version match the offer;
   * that there's a library for this platform; if not, answer `unsupported`.
3. **Show the offer card** (§7.2) and answer `offered`. A card waits in a queue while another
   card is open; one server can queue at most 4 offers per connection.
4. **On approval:**
   * Download the package into `staging/`, with progress on the card and the game still
     playable. Answer `downloading`.
   * The size must match the offer, and the download stops if it goes past it.
   * The SHA-256 must match the offer.
5. **Verify the package:**
   * its manifest is byte-identical to the one approved on the card;
   * the zip entries are exactly the manifest's files (§3.2), and every file's hash matches.
6. **Install:** unpack to `cl_plugins/<id>/<version>/`, re-hash the unpacked files, write the
   store entry with those hashes, and delete the staging file.
7. **Load** (§9). Answer `loaded`, or `installed` when the scope means it shouldn't load yet.
   Any failure answers `failed <reason>` and leaves nothing behind but the log line.

* **HTTP:** a worker thread in `client.dll` does the fetching, with `ureq` and `rustls` and
  redirects limited to 3. HTTPS is required for absolute URLs. A relative URL uses
  `sv_downloadurl` as it is, which is often plain HTTP. That's acceptable because integrity
  comes from the offer's hash and the signature, not the transport. Unsigned packages over plain
  HTTP are always refused. The cost is a TLS stack in `client.dll` (open question 2).
* **Cancel:** disconnecting or pressing Cancel stops the download and deletes the staging file.

### 6.4 Updates

An offer for a newer version of an approved plugin shows the update card (§7.2):
- the old and new versions;
- whether the publisher key is the same;
- the `declares` that changed;
- how many files changed.

It's approved like any offer, so every change prompts. An offer for an older version than the
approved one is shown as a downgrade, with a warning, and "Allow" isn't the default.

### 6.5 Client replies: `tspkg`

```
tspkg <id> <version> <state> [reason]
state: have | offered | downloading | installed | loaded | denied | blocked | unsupported | failed
```

* `denied` covers both "Not now" and "Never". The server isn't told which.
* The server rate-limits `tspkg` (16 per 10 seconds) and ignores ids it didn't offer.

## 7. Approval dialogs

### 7.1 At start-up

Changed or new plugins from §5.2 are shown before the main menu, with `SDL_ShowMessageBox`.
`client.dll` looks it up in the SDL2 the engine has already loaded (`goldsrc::platform`'s
lookup, as the port already does for mouse and joystick calls). Both HL25 and FWGS use SDL2.

* **Contents:**
  * plugin name, version and id;
  * publisher and trust level (§3.3);
  * source;
  * what changed: "new plugin", or the list of changed, missing and extra files (up to 10 lines,
    then "and N more"), or "manifest changed", "version 0.1.0 → 0.2.0", or "publisher key
    changed";
  * the line: "This is a program that runs inside the game with your account's permissions."
* **Buttons:** "Allow", "Not now" (doesn't load; asks again at the next start) and "Never"
  (`state = "blocked"`). The default button and Escape are "Not now".
* **Several plugins:** one box each, in id order. "Not now for all" appears when there are more
  than three.
* **Launcher:** when the game starts from tsmod-launcher, the launcher runs the same check
  first (`tsmod-launcher` reads the same store and manifests) and shows a richer dialog with the
  full file list. Once it has approved, the in-game box doesn't appear.

### 7.2 Server offer and update cards

These are drawn in game as a port VGUI panel (`crates/client/src/menus/`, next to the MOTD and
team menus).

* **Contents:**
  * the server's name and address;
  * the plugin's name, version, publisher, trust level and homepage;
  * the description;
  * download size and installed size;
  * `declares` in plain words ("starts programs: tsmod-webview-host", "uses the network",
    "stores data");
  * "Required by this server" and the server's note;
  * the native-code line from §7.1.
* **Buttons:**
  * "Install and allow";
  * "Allow on this server only": `scope = "server"`. The plugin loads only while connected to
    this address and stays loaded until the game exits (§9).
  * "Not now";
  * "Never from this server": the id is blocked for that address. Holding Shift turns it into
    "Never": the id is blocked everywhere.

### 7.3 Rules that keep a server from approving for the player

* **No console command approves.** The store is written only from a click in a dialog or from
  the launcher, so a server can't approve through `stufftext`, `alias` or binds.
* **Clicks only:**
  * the approve buttons take only a mouse click, never Enter, Space or a bound key;
  * they stay disabled for the first 2 seconds a card is shown, and after any card change, so a
    click aimed at something else doesn't land on them;
  * the default is always "Not now".
* **No flooding:** at most 4 offers per server per connection. A refused offer isn't shown again
  for the same id in that connection. "Never" stops it for good.
* **No cover:** while a card is open, the server can't draw over it. It's drawn last, above HUD
  and VGUI, and the webview overlay and other extensions get no `redraw` over it.
* **Clear source:** a card always says which server is asking.

## 8. Cvars and commands (client)

| cvar | default | meaning |
|---|---|---|
| `cl_plugins` | 1 | 0 loads no plugins at all, approved or not |
| `cl_plugins_download` | 1 | 0 ignores every server offer (answers nothing) |
| `cl_plugins_allow_unsigned` | 0 | 1 lets unsigned server packages reach the card |
| `cl_plugins_verify_log` | 0 | log each file hash at start |

* `cl_plugins_list`: installed plugins, version, state, scope, trust, loaded or not.
* `cl_plugins_menu`: a panel listing plugins. From it the player can revoke an approval, unblock,
  delete a plugin, and see every file and hash. Revoking takes effect at the next start (§9).
* `cl_plugins_verify`: re-hashes every plugin now and prints the result. It never approves.

None of these can approve. A server that stuffs `cl_plugins 0` or `cl_plugins_menu` can, at
worst, disable plugins for the next start or open the panel. Both are harmless and visible.

## 9. Loading

* **Approved at start:** loaded at `Initialize` through the client extension API, as specified
  in the webview spec §4.1.
* **Approved later** (a server offer or update): loaded at once, late. The port calls the
  extension's `init`, then `vid_init` if a map is loaded. Extensions must accept that order.
  `client_ext_api` must match exactly; on a mismatch, the reply is `failed api`.
* **`scope = "server"`:** loaded on connecting to a listed address. After that it stays loaded
  until the game exits, even on other servers, because unloading native code isn't safe. The
  card says so. The extension gets a `server_changed(address)` callback, and the webview plugin
  closes its server surface on it.
* **Update while loaded:** the new version is installed and approved, but loads at the next
  start. The card says "restart to finish".
* **Revoked while loaded:** stays loaded until exit.

## 10. Hashing and file rules (shared)

The pure parts go in a new crate, `crates/plugin-pkg`: no unsafe, shared by the client and
tsmod-launcher.
- manifest parsing and path rules
- zip checks
- the store format
- SHA-256 over files
- the comparison in §5.2, which produces a `Change` list
- signature and countersignature checks (`ed25519-dalek`)

## 11. Development and tests

* **Developers:** `cargo xtask plugin-approve <dir|tspkg>` writes a store entry for a local build
  on the developer's own machine. That's the same choice a player makes in a dialog, made at the
  terminal. The e2e roots use it, so runs don't stop at a prompt.
* **Unit tests (`plugin-pkg`):**
  * manifest and path rules (`..`, absolute paths, drive letters, symlinks, duplicate paths);
  * a zip with an extra or missing file;
  * every `Change` variant (changed, missing, extra, manifest, version, key, platform);
  * the store round trip and the `.bad` fallback;
  * signature, countersignature and key change;
  * downgrade detection.
* **Port unit tests:**
  * the offer state machine and the offer queue and limits;
  * the `TSPkg` encoding within 192 bytes;
  * `tspkg` parsing and rate limits on the server.
* **e2e (`cargo xtask e2e`, FWGS root with a dedicated server):**
  1. Approve a stub plugin with `plugin-approve`, start, and check it loads. Change one byte, add
     an extra file, delete a file; each must stop it loading, and the log must name the change.
     The prompt is answered by a test-only SDL stub that records what it was shown.
  2. Serve a signed stub package from the root's FastDL and offer it. The test answers the card
     through a test-only hook, which is compiled only into `--features e2e` builds and is never a
     console command. Check the reply sequence `offered`, `downloading`, `loaded`, the installed
     files and the store hashes.
  3. Offer a package whose size or hash doesn't match the offer, a manifest whose signature
     fails, and an unsigned package. Each must answer `failed` or never reach the card, and leave
     nothing in `cl_plugins/`.
  4. An original TS 3.0 client on the same server must not receive `TSPkg` and must play
     normally.
  5. Steam HL25: manual for v1 (validation.md §5, Steam status invisible while testing).

## 12. Wire compatibility

* `TSPkg` (and the webview spec's `TSExt`) are new user messages. TS 3.0's client doesn't hook
  them, and a GoldSrc client can fail on a user message it hasn't hooked ("UserMsg: Not Present
  on Client"). So the server sends them only to clients known to be the port: those whose
  userinfo carries `_tsrs` (`shared::protocol::port::USERINFO_KEY`), which the server already
  reads in `ClientConnect`. The client takes `TSPkg` support from the port server's version
  (`SrvSett` keys 5–7), so neither side needs a new discovery channel.

  Original clients never get either message, and design.md §2's contract holds.
* The registration order of TS's 64 messages doesn't change. New messages are appended after
  them.
* This client on an original TS server gets no offers, because nothing sends them.

## 13. Open questions

1. `TSPkg` can exceed 192 bytes with a long URL and note. Should it chunk, as the webview `TSExt`
   framing does, or move the note into the manifest?
2. **The TLS stack:** `ureq` and `rustls` add about 1.5–2 MB to each `client.dll`. The
   alternative is the engine's own download (`precache_generic` plus `sv_downloadurl`), which
   accepts `.tspkg` names. But it runs at connect time, blocks joining for large packages, and
   can't show a card before downloading.
3. Does `SDL_ShowMessageBox` show correctly over a fullscreen window at `Initialize`, on both
   engines and on Wine? If not, the start-up prompt waits for the first frame and uses the VGUI
   card, and plugins awaiting approval load late (§9).
4. The countersigning key: who holds it, and how a publisher asks for verification. Probably a
   release-repo workflow in the-specialists-mod.
5. Libraries a helper process loads on its own (`libcef.so`, `libcef.dll`) are hashed at start,
   but they could be swapped between that check and the helper loading them. Should
   `verified_spawn` re-hash all of a declared process's files, at a cost of about 0.2 s per
   start of the helper?
6. Should the launcher be able to install packages from the mod's own release feed, with the
   same cards, so players needn't wait for a server to offer them?

## 14. Milestones (port issues)

1. `plugin-pkg` crate: manifest, store, hashing, comparison, signatures, with the unit tests.
2. Client: the loader with start-up verification and the SDL prompt, hand-installed libraries,
   `cl_plugins_*` commands, `xtask plugin-approve`. Depends on the client extension API.
3. Server: `TSPkg` (sent only to `_tsrs` clients), `plugin_offers.toml`, `sv_plugin_offer`, `tspkg` replies
   and the plugin API additions.
4. Client: the offer flow, downloads, the VGUI cards and their safeguards (§7.3), late loading,
   e2e.
5. tsmod-launcher: the same verification and approval UI, using `plugin-pkg`.
