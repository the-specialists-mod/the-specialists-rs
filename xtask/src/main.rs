//! Build, package, install and validate the TS port.
//!
//! ```text
//! cargo xtask build   [--release]                 build all four targets
//! cargo xtask dist    [--release]                 dist/<platform>/ts/: libraries + game files
//! cargo xtask install [--release] <root>... | all assemble <root>/ts and install libraries
//! cargo xtask server  [--release] <root> [--map M] [--bots N] [--seconds S] [--port P]
//!                                                 run a dedicated server and check it
//! cargo xtask test    [--release] [<root>...]     install + server check on every root
//! cargo xtask client  [--release] <server-root> [--map M] [--port P]
//!                                                 FWGS client (Rust client library) joins a dedicated
//!                                                 server on <server-root>; screenshot in target/
//! cargo xtask listen  [--release] [<fwgs-root>...] [--map M]
//!                                                 listen server on FWGS: buy a loadout, click to
//!                                                 spawn, add/kick bots; screenshots in target/
//! cargo xtask dojo    [--release] [<fwgs-root>...]
//!                                                 gameplay on ts_dojo: power-up, grenade vs
//!                                                 breakable, kill/death/respawn, bots fighting
//! cargo xtask roots                               list engine roots and whether they exist
//! ```
//!
//! Engine roots default to ~/games/<dir> and can be overridden with `TS_ROOT_<NAME>`
//! (e.g. `TS_ROOT_REHLDS_LINUX=/srv/hlds`). TS content comes from `TS_DIR` (default ../ts):
//! the installed game directory links to its maps, models and sounds without modifying it.

mod a2s;
mod roots;

use roots::{Engine, Os, Root, Target};
use std::path::{Path, PathBuf};
use std::process::{Command, exit};
use std::time::{Duration, Instant};
use std::{env, fs};

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let release = take_flag(&mut args, "--release");
    let Some(cmd) = (!args.is_empty()).then(|| args.remove(0)) else { usage() };
    let ctx = Ctx { release, repo: repo_root() };
    match cmd.as_str() {
        "build" => ctx.build(&Target::ALL),
        "dist" => ctx.dist(),
        "install" => {
            for r in select_roots(&args) {
                ctx.install(&r);
            }
        }
        "server" => {
            let map = take_opt(&mut args, "--map").unwrap_or_else(|| "ts_arena".into());
            let bots: u32 = take_opt(&mut args, "--bots").map_or(0, |v| v.parse().unwrap());
            let secs: u64 = take_opt(&mut args, "--seconds").map_or(20, |v| v.parse().unwrap());
            let port: u16 = take_opt(&mut args, "--port").map_or(27115, |v| v.parse().unwrap());
            let root = select_roots(&args).into_iter().next().unwrap_or_else(|| usage());
            if !ctx.server(&root, &map, bots, secs, port) {
                exit(1);
            }
        }
        "test" => {
            let map = take_opt(&mut args, "--map").unwrap_or_else(|| "ts_arena".into());
            let mut failed = Vec::new();
            for (i, r) in select_roots(&args).into_iter().enumerate() {
                if !r.exists() {
                    println!("== {}: skipped ({} missing)", r.name, r.path.display());
                    continue;
                }
                ctx.install(&r);
                if !ctx.server(&r, &map, 2, 25, 27115 + i as u16) {
                    failed.push(r.name);
                }
            }
            if !failed.is_empty() {
                eprintln!("FAILED: {failed:?}");
                exit(1);
            }
            println!("all server checks passed");
        }
        "client" => {
            let map = take_opt(&mut args, "--map").unwrap_or_else(|| "ts_awaken".into());
            let port: u16 = take_opt(&mut args, "--port").map_or(27150, |v| v.parse().unwrap());
            let roots = select_roots(&args);
            let fwgs = Root::all().into_iter().find(|r| r.name == "fwgs").unwrap();
            let mut failed = Vec::new();
            for (i, r) in roots.into_iter().filter(|r| r.exists()).enumerate() {
                ctx.install(&r);
                if !ctx.client(&r, &fwgs, &map, port + i as u16) {
                    failed.push(r.name);
                }
            }
            if !failed.is_empty() {
                eprintln!("FAILED: {failed:?}");
                exit(1);
            }
        }
        "listen" => {
            let map = take_opt(&mut args, "--map").unwrap_or_else(|| "ts_lobby".into());
            let names = if args.is_empty() { vec!["fwgs".to_owned(), "fwgs-win".to_owned()] } else { args.clone() };
            let mut failed = Vec::new();
            for r in select_roots(&names) {
                if r.engine != Engine::Fwgs {
                    println!("== {}: skipped (listen servers need an FWGS client)", r.name);
                    continue;
                }
                if !r.exists() {
                    println!("== {}: skipped ({} missing)", r.name, r.path.display());
                    continue;
                }
                ctx.install(&r);
                if !ctx.listen(&r, &map) {
                    failed.push(r.name);
                }
            }
            if !failed.is_empty() {
                eprintln!("FAILED: {failed:?}");
                exit(1);
            }
        }
        "dojo" => {
            let names = if args.is_empty() { vec!["fwgs".to_owned(), "fwgs-win".to_owned()] } else { args.clone() };
            let mut failed = Vec::new();
            for r in select_roots(&names) {
                if r.engine != Engine::Fwgs || !r.exists() {
                    println!("== {}: skipped (needs an installed FWGS client)", r.name);
                    continue;
                }
                ctx.install(&r);
                if !ctx.dojo(&r) {
                    failed.push(r.name);
                }
            }
            if !failed.is_empty() {
                eprintln!("FAILED: {failed:?}");
                exit(1);
            }
        }
        "roots" => {
            for r in Root::all() {
                println!("{:14} {:7} {:?} {} {}", r.name, format!("{:?}", r.os), r.engine, r.path.display(),
                         if r.exists() { "ok" } else { "missing" });
            }
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("usage: cargo xtask [--release] <build|dist|install|server|test|roots> ... (see xtask/src/main.rs)");
    exit(2)
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != flag);
    args.len() != before
}

fn take_opt(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

fn select_roots(names: &[String]) -> Vec<Root> {
    if names.is_empty() || names.iter().any(|n| n == "all") {
        return Root::all();
    }
    names
        .iter()
        .map(|n| Root::all().into_iter().find(|r| r.name == n).unwrap_or_else(|| {
            eprintln!("unknown root {n}; see `cargo xtask roots`");
            exit(2)
        }))
        .collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn ts_dir(repo: &Path) -> PathBuf {
    env::var_os("TS_DIR").map(PathBuf::from).unwrap_or_else(|| repo.join("../ts"))
}

struct Ctx {
    release: bool,
    repo: PathBuf,
}

impl Ctx {
    fn profile(&self) -> &'static str {
        if self.release { "release" } else { "debug" }
    }

    fn build(&self, targets: &[Target]) {
        for t in targets {
            let mut c = Command::new(env::var("CARGO").unwrap_or("cargo".into()));
            c.current_dir(&self.repo).args(["build", "-p", "ts-server", "-p", "ts-client", "--target", t.triple]);
            if self.release {
                c.arg("--release");
            }
            println!("== cargo build --target {}", t.triple);
            if !c.status().expect("cargo").success() {
                exit(1);
            }
        }
    }

    fn artifact(&self, t: &Target, server: bool) -> PathBuf {
        let dir = self.repo.join("target").join(t.triple).join(self.profile());
        let name = match (t.os, server) {
            (Os::Linux, true) => "libts.so",
            (Os::Linux, false) => "libclient.so",
            (Os::Windows, true) => "ts.dll",
            (Os::Windows, false) => "client.dll",
        };
        dir.join(name)
    }

    /// Copies the libraries for `t` into `gamedir` under the names each engine loads.
    fn install_libs(&self, t: &Target, gamedir: &Path) {
        copy(&self.artifact(t, true), &gamedir.join("dlls").join(t.server_name));
        // Stock HLDS (8684 and HL25) strips "_i386" from gamedll_linux and loads dlls/ts.so;
        // ReHLDS and old engines load the name as written. Install both.
        if t.server_name == "ts_i386.so" {
            copy(&self.artifact(t, true), &gamedir.join("dlls/ts.so"));
        }
        copy(&self.artifact(t, false), &gamedir.join("cl_dlls").join(t.client_name));
        if t.os == Os::Windows {
            msvc_style_export_section(&gamedir.join("dlls").join(t.server_name));
            msvc_style_export_section(&gamedir.join("cl_dlls").join(t.client_name));
        }
    }

    fn dist(&self) {
        self.build(&Target::ALL);
        for t in &Target::ALL {
            let gamedir = self.repo.join("dist").join(t.platform).join("ts");
            copy_dir(&self.repo.join("game/ts"), &gamedir);
            self.install_libs(t, &gamedir);
        }
        println!("dist/ ready: copy dist/<platform>/ts over a TS 3.0 install's ts/ directory");
    }

    /// Assembles `<root>/ts`: links TS content, copies writable config files, installs our game
    /// files and libraries.
    fn install(&self, root: &Root) {
        if !root.exists() {
            eprintln!("== {}: {} missing, skipped", root.name, root.path.display());
            return;
        }
        let t = root.target();
        self.build(&[t]);
        let gamedir = root.path.join("ts");
        link_ts_content(&ts_dir(&self.repo), &gamedir);
        copy_dir(&self.repo.join("game/ts"), &gamedir);
        self.install_libs(&t, &gamedir);
        println!("== {}: installed into {}", root.name, gamedir.display());
    }

    /// Starts a dedicated server, waits for it to answer A2S_INFO, optionally checks that `bots`
    /// bots joined, watches the log for panics, then stops it.
    fn server(&self, root: &Root, map: &str, bots: u32, secs: u64, port: u16) -> bool {
        println!("== {}: dedicated server, map {map}, {bots} bots, {secs}s, port {port}", root.name);
        let log = root.log_path();
        let _ = fs::remove_file(&log);
        let mut cmd = root.server_command(map, port, bots);
        let stdout = fs::File::create(self.repo.join("target").join(format!("server-{}.log", root.name))).unwrap();
        cmd.stdout(stdout.try_clone().unwrap()).stderr(stdout);
        let mut child = cmd.spawn().expect("start server");
        let start = Instant::now();
        let mut info = None;
        // Windows servers run under Wine and take noticeably longer to come up.
        let limit = Duration::from_secs(if root.os == Os::Windows { 120 } else { 60 });
        while start.elapsed() < limit {
            std::thread::sleep(Duration::from_millis(500));
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            if let Ok(i) = a2s::info("127.0.0.1", port) {
                info = Some(i);
                break;
            }
        }
        let mut ok = true;
        match &info {
            Some(i) => println!("   up after {:.1}s: {i}", start.elapsed().as_secs_f32()),
            None => {
                println!("   FAIL: no A2S_INFO reply within {}s", limit.as_secs());
                ok = false;
            }
        }
        if ok {
            std::thread::sleep(Duration::from_secs(secs));
            // HL25 rate-limits status queries per address: retry a few times before failing.
            let mut reply = a2s::info("127.0.0.1", port);
            for _ in 0..5 {
                if reply.is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
                reply = a2s::info("127.0.0.1", port);
            }
            match reply {
                Ok(i) => {
                    println!("   after {secs}s: {i}");
                    if let Ok(pl) = a2s::players("127.0.0.1", port) {
                        let list: Vec<String> = pl.iter().map(|(n, f)| format!("{n}={f}")).collect();
                        println!("   scores: {}", list.join(", "));
                    }
                    if i.map != map {
                        println!("   FAIL: map is {:?}, expected {map:?}", i.map);
                        ok = false;
                    }
                    if i.players < bots as u8 {
                        // The Xash `info` reply (old FWGS) doesn't count fake clients: use the log.
                        let added = root.read_log(&self.repo).matches("[ts] bots: added").count();
                        if i.game.is_empty() && added >= bots as usize {
                            println!("   {added} bots joined (from the log; this engine's query doesn't count bots)");
                        } else {
                            println!("   FAIL: {} players, expected {bots} bots", i.players);
                            ok = false;
                        }
                    }
                }
                Err(e) => {
                    println!("   FAIL: server stopped answering: {e}");
                    ok = false;
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        root.cleanup();
        let text = root.read_log(&self.repo);
        for line in text.lines().filter(|l| l.contains("[ts]") || l.contains("PANIC") || l.contains("FATAL")) {
            println!("   | {}", line.trim_end());
        }
        if text.contains("PANIC") || text.contains("FATAL ERROR") || text.contains("Segmentation") {
            println!("   FAIL: panic or fatal error in the log");
            ok = false;
        }
        if !text.contains("[ts] GameInit") {
            println!("   FAIL: the Rust server library didn't load (no GameInit line)");
            ok = false;
        }
        println!("   {}", if ok { "PASS" } else { "FAIL" });
        ok
    }
}

impl Ctx {
    /// Runs `server_root`'s dedicated server (2 bots) on a thread and joins it with the FWGS
    /// client from `client_root`. Passes when the client connects and the server check passes.
    fn client(&self, server_root: &Root, client_root: &Root, map: &str, port: u16) -> bool {
        self.install(client_root);
        let srv = Ctx { release: self.release, repo: self.repo.clone() };
        let (sr, m) = (server_root.clone(), map.to_owned());
        let server = std::thread::spawn(move || srv.server(&sr, &m, 2, 60, port));
        let start = Instant::now();
        while a2s::info("127.0.0.1", port).is_err() && start.elapsed() < Duration::from_secs(120) {
            std::thread::sleep(Duration::from_millis(500));
        }
        std::thread::sleep(Duration::from_secs(3));

        // GoldSrc servers need the GoldSrc protocol; FWGS servers speak FWGS's own.
        let proto = if server_root.engine == Engine::Fwgs { "" } else { " gs" };
        let w = "wait;".repeat(20);
        let waits = format!("{w}\n").repeat(30);
        let cfg = format!("connect 127.0.0.1:{port}{proto}\n{waits}echo XTASK: in game\nscreenshot\n{}quit\n", format!("{w}\n").repeat(3));
        let gamedir = client_root.path.join("ts");
        fs::write(gamedir.join("xtask_client.cfg"), cfg).unwrap();
        let _ = fs::remove_dir_all(gamedir.join("scrshots"));
        let log_path = self.repo.join("target").join(format!("client-{}.log", server_root.name));
        let log = fs::File::create(&log_path).unwrap();
        let mut child = Command::new(client_root.path.join("xash3d"))
            .current_dir(&client_root.path)
            .args(["-game", "ts", "-dev", "2", "-windowed", "-width", "1024", "-height", "768", "+exec", "xtask_client.cfg"])
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .expect("start client");
        let t = Instant::now();
        while child.try_wait().ok().flatten().is_none() && t.elapsed() < Duration::from_secs(90) {
            std::thread::sleep(Duration::from_millis(500));
        }
        let _ = child.kill();
        let _ = child.wait();

        let text = String::from_utf8_lossy(&fs::read(&log_path).unwrap_or_default()).replace('\r', "");
        let connected = text.contains("client connected");
        let in_game = text.contains("XTASK: in game");
        let panicked = text.contains("PANIC");
        let shot = fs::read_dir(gamedir.join("scrshots")).ok().and_then(|d| d.flatten().next()).map(|e| e.path());
        if let Some(p) = &shot {
            let dest = self.repo.join("target").join(format!("client-{}.png", server_root.name));
            let _ = fs::copy(p, &dest);
            println!("   client screenshot: {}", dest.display());
        }
        // `echo` output isn't always mirrored to stdout, so the screenshot is the in-game evidence.
        let ok = connected && !panicked && shot.is_some();
        println!("   client: connected={connected} in_game={in_game} panic={panicked} screenshot={}", shot.is_some());
        let server_ok = server.join().unwrap();
        println!("   {} (client + server)", if ok && server_ok { "PASS" } else { "FAIL" });
        ok && server_ok
    }
}

/// Script helpers for listen-server tests. `wait` is one frame and the scripts pin `fps_max 100`;
/// `script_start` defines `w10` (10 frames) and `w100` (1 s). Plain `wait` chains would overflow
/// the engine's 32 KB command buffer on long scripts.
fn secs(n: f32) -> String {
    let frames = (n * 100.0).round() as usize;
    "w100;".repeat(frames / 100) + &"w10;".repeat(frames % 100 / 10) + &"wait;".repeat(frames % 10) + "\n"
}

fn shot(tag: &str) -> String {
    format!("echo XTASK: {tag}\nscreenshot\n")
}

/// Script prologue: cheats for the dev commands, scripted input past the client's anti-script
/// gate (`ts_dev_scriptinput 1`), then the map.
fn script_start(map: &str) -> String {
    format!(
        "// Written by cargo xtask.\nalias w10 \"{}\"\nalias w100 \"{}\"\nfps_max 100\nsv_cheats 1\nts_dev_scriptinput 1\nmaxplayers 8\nmap {map}\n",
        "wait;".repeat(10),
        "w10;".repeat(10)
    )
}

/// Join flow: wait for the buy menu (open on join with a random first loadout), Reset -> Buy ->
/// Rifles -> AK47 -> scope -> add -> back -> done, then click `+attack` after the respawn wait.
fn script_join_and_spawn() -> String {
    let mut cfg = secs(8.0) + &shot("joined");
    for key in [4, 2, 3, 2, 5, 1, 10, 1] {
        cfg += &format!("slot{key}\n{}", secs(1.0));
    }
    cfg += &shot("bought");
    cfg += &secs(5.0);
    cfg += &click();
    cfg + &secs(3.0)
}

fn click() -> String {
    "echo XTASK: click\n+attack\nwait;wait;wait;wait;wait\n-attack\n".to_owned()
}

/// `ts_status` dump tagged so checks can find it.
fn status(tag: &str) -> String {
    format!("echo XTASK: status {tag}\nts_status\n")
}

/// One player line of a `ts_status` dump.
#[derive(Debug, Clone)]
struct PlayerStatus {
    idx: i32,
    name: String,
    state: String,
    frags: i32,
    deaths: i32,
    held: String,
    active: String,
    slow_target: f32,
    org: [f32; 3],
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = &line[line.find(key)? + key.len()..];
    Some(rest.split_whitespace().next().unwrap_or(""))
}

fn parse_status_line(line: &str) -> Option<PlayerStatus> {
    let rest = line.split("[ts]  #").nth(1)?;
    let (idx, rest) = rest.split_once(' ')?;
    let state_at = [" alive ", " dead ", " observer "].iter().filter_map(|s| rest.find(s).map(|i| (i, *s))).min()?;
    let name = rest[..state_at.0].trim().to_owned();
    let org = line.split("org=(").nth(1)?.trim_end_matches(')');
    let o: Vec<f32> = org.split_whitespace().filter_map(|v| v.trim_end_matches(')').parse().ok()).collect();
    Some(PlayerStatus {
        idx: idx.parse().ok()?,
        name,
        state: state_at.1.trim().to_owned(),
        frags: field(line, "frags=")?.parse().ok()?,
        deaths: field(line, "deaths=")?.parse().ok()?,
        held: field(line, "held=")?.split('/').next()?.to_owned(),
        active: field(line, "active=")?.to_owned(),
        slow_target: field(line, "slow=")?.split("->").nth(1)?.parse().ok()?,
        org: [*o.first()?, *o.get(1)?, *o.get(2)?],
    })
}

/// The `ts_status` dumps of a run, in order: (tag, players).
fn status_dumps(text: &str) -> Vec<(String, Vec<PlayerStatus>)> {
    let mut out: Vec<(String, Vec<PlayerStatus>)> = Vec::new();
    for line in text.lines() {
        if let Some(tag) = line.split("XTASK: status ").nth(1) {
            out.push((tag.trim().to_owned(), Vec::new()));
        } else if let (Some(last), Some(p)) = (out.last_mut(), parse_status_line(line)) {
            last.1.push(p);
        }
    }
    out
}

struct Run {
    text: String,
    shots: Vec<PathBuf>,
    /// The game exited on its own (crash, or a script `quit`).
    exited: bool,
    /// The script's last line (`echo XTASK: done`) ran.
    done: bool,
}

impl Ctx {
    /// Runs the FWGS client of `root` as a listen server with `cfg` and stops it 10 s after the
    /// script's final `echo XTASK: done`. Server commands the game library queues (`kick` from
    /// the bot quota) wait behind the script in the console buffer, so the script must end
    /// rather than `quit`.
    fn run_script(&self, root: &Root, tag: &str, cfg: &str, limit: Duration) -> Run {
        let gamedir = root.path.join("ts");
        let cfg_name = format!("xtask_{tag}.cfg");
        fs::write(gamedir.join(&cfg_name), cfg).unwrap();
        let _ = fs::remove_dir_all(gamedir.join("scrshots"));
        let log_path = self.repo.join("target").join(format!("{tag}-{}.log", root.name));
        let log = fs::File::create(&log_path).unwrap();
        let args = ["-game", "ts", "-dev", "2", "-log", "-windowed", "-width", "1024", "-height", "768", "+exec", &cfg_name];
        let mut cmd = if root.os == Os::Windows {
            let mut c = Command::new("wine");
            c.env("WINEPREFIX", roots::wineprefix()).env("WINEDEBUG", "-all").arg("xash3d.exe");
            c
        } else {
            Command::new(root.path.join("xash3d"))
        };
        let _ = fs::remove_file(root.log_path());
        let mut child = cmd.current_dir(&root.path).args(args).stdout(log.try_clone().unwrap()).stderr(log).spawn().expect("start client");
        let t = Instant::now();
        let (mut exited, mut done_at) = (false, None);
        while t.elapsed() < limit {
            if child.try_wait().ok().flatten().is_some() {
                exited = true;
                break;
            }
            if done_at.is_none() && fs::read(root.log_path()).is_ok_and(|b| String::from_utf8_lossy(&b).contains("XTASK: done")) {
                done_at = Some(Instant::now());
            }
            if done_at.is_some_and(|d| d.elapsed() > Duration::from_secs(10)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let _ = child.kill();
        let _ = child.wait();
        root.cleanup();

        // engine.log (-log) has everything on both builds; stdout is the fallback. Not both:
        // the checks count lines.
        let engine_log = String::from_utf8_lossy(&fs::read(root.log_path()).unwrap_or_default()).into_owned();
        let text = if engine_log.contains("XTASK:") { engine_log } else { String::from_utf8_lossy(&fs::read(&log_path).unwrap_or_default()).into_owned() };
        let text = text.replace('\r', "");
        let mut shots: Vec<PathBuf> = fs::read_dir(gamedir.join("scrshots")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        shots.sort();
        let shots = shots
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let dest = self.repo.join("target").join(format!("{tag}-{}-{}.png", root.name, i + 1));
                let _ = fs::copy(p, &dest);
                dest
            })
            .collect();
        Run { text, shots, exited, done: done_at.is_some() }
    }

    fn report(&self, root: &Root, tag: &str, run: &Run, checks: &[(&str, bool)]) -> bool {
        for p in &run.shots {
            println!("   screenshot: {}", p.display());
        }
        let mut ok = true;
        for (what, pass) in checks {
            println!("   {} {what}", if *pass { "ok  " } else { "FAIL" });
            ok &= *pass;
        }
        let log = self.repo.join("target").join(format!("{tag}-{}.log", root.name));
        println!("   {} (full log: {}; engine log: {})", if ok { "PASS" } else { "FAIL" }, log.display(), root.log_path().display());
        ok
    }

    /// A listen server played the way a person joins: the buy menu opens on join, the script
    /// resets the random first loadout, picks an AK47 with a scope and confirms (`slotN`, as the
    /// number keys do), clicks `+attack` after the respawn wait to spawn in, then adds bots,
    /// kicks one and lets the bot quota add and remove more.
    fn listen(&self, root: &Root, map: &str) -> bool {
        println!("== {}: listen server on {map}", root.name);
        let mut cfg = script_start(map) + &script_join_and_spawn() + &shot("spawned");
        // `addbot` raises the quota to cover its bot; at 0 the quota would kick it right away.
        cfg += &format!("addbot\n{}addbot\n{}ts_bot_quota\n", secs(2.0), secs(8.0));
        // Kicking a bot left a dropped client the bot code kept driving (pfnSetClientMaxspeed crash).
        cfg += &format!("kick \"Bot Neo\"\n{}", secs(10.0));
        cfg += &format!("ts_bot_quota 3\n{}", secs(6.0));
        cfg += &shot("bots");
        cfg += "ts_bot_quota 0\necho XTASK: done\n";
        let run = self.run_script(root, "listen", &cfg, Duration::from_secs(300));
        let text = &run.text;
        for line in text.lines().filter(|l| l.contains("XTASK:") || l.contains("spawned (#") || l.contains("kicked")) {
            println!("   | {}", line.trim_end());
        }
        let spawn = text.lines().find(|l| l.contains("[ts] ") && l.contains("spawned (#1)"));
        let checks = [
            ("the host spawned in by clicking", spawn.is_some()),
            ("with the bought AK47 (id 13)", spawn.is_some_and(|l| l.split_once(" with ").is_some_and(|(_, w)| w.contains("13")))),
            ("bots were added", text.matches("[ts] bots: added").count() >= 2),
            ("addbot raised the quota to 2", text.contains("\"ts_bot_quota\" is \"2\"")),
            ("a bot was kicked", text.contains("was kicked")),
            ("the quota removed bots", text.matches("was kicked").count() >= 2),
            ("the script finished", run.done),
            ("the game kept running to the end", !run.exited),
            ("no panic or crash", !crashed(text)),
            ("screenshots", run.shots.len() >= 4),
        ];
        self.report(root, "listen", &run, &checks)
    }

    /// Gameplay on ts_dojo, on a listen server:
    /// 1. join, buy an AK47, click to spawn;
    /// 2. with a stopped bot (`ts_bot_stop 1`) in view, step onto the slow-motion power-up
    ///    (ts_dojo `0 -672 48`, pwuptype 1) and double-tap `+use` to activate it: the host runs
    ///    at 0.6 and slows the players it sees to 0.35;
    /// 3. take an M61, aim at a 1-HP wooden pillar (`*15`) and throw: the explosion breaks it;
    /// 4. the stopped bot is put in front of the host, who shoots it dead;
    /// 5. the host is killed (`ts_hurt`), clicks through the death cam and clicks to respawn;
    /// 6. the host suicides and stays observer while four bots (`ts_bot_stop 0`) play for a
    ///    minute: they move around and kill each other.
    fn dojo(&self, root: &Root) -> bool {
        println!("== {}: gameplay on ts_dojo", root.name);
        let mut cfg = script_start("ts_dojo") + &script_join_and_spawn() + &status("spawned");

        // A stopped bot (no movement, no fire) to be slowed and shot. It joins as an observer
        // and respawns after the 5 s wait.
        cfg += &format!("ts_bot_stop 1\naddbot\n{}", secs(9.0));

        // 2. power-up, with the bot standing in view
        cfg += &format!("ts_tp 1 0 -672 60 90 0\n{}ts_tp 2 0 -480 60 270 0\n{}", secs(0.5), secs(1.5));
        cfg += &status("picked up");
        cfg += "+use\nwait;wait;wait;wait;wait\n-use\nwait;wait;wait;wait;wait\n+use\nwait;wait;wait;wait;wait\n-use\n";
        cfg += &secs(1.5);
        cfg += &status("power-up used");
        cfg += &shot("slow motion");
        cfg += &secs(4.0);

        // 3. grenade
        cfg += &format!("ts_give 24\n{}weapon_24\n{}", secs(0.5), secs(1.5));
        cfg += &format!("ts_tp 1 -332 -420 60 90 0\n{}ts_face 1 -332 -164 60\n{}", secs(1.0), secs(0.5));
        cfg += &shot("aim grenade");
        cfg += "echo XTASK: throw\n+attack\nw10\n-attack\n";
        cfg += &secs(1.0);
        cfg += &format!("ts_tp 1 0 -1344 60 90 0\n{}", secs(6.0));
        cfg += &shot("after grenade");

        // 4. the host shoots the stopped bot
        cfg += &format!("weapon_13\n{}ts_tp 2 0 -1200 60 270 0\n{}", secs(1.5), secs(0.5));
        cfg += "echo XTASK: shoot\n+attack\n";
        for _ in 0..15 {
            cfg += &format!("ts_face 1 2\n{}", secs(0.2));
        }
        cfg += "-attack\n";
        cfg += &status("bot shot");
        cfg += &secs(1.0);

        // 5. the host dies and respawns
        cfg += "echo XTASK: host killed\nts_hurt 1 500 2 2\n";
        cfg += &secs(2.0);
        cfg += &click();
        cfg += &secs(1.0);
        cfg += &status("host dead");
        cfg += &secs(6.0);
        cfg += &click();
        cfg += &secs(2.0);
        cfg += &status("host respawned");
        cfg += &shot("respawned");

        // 6. bots on their own; the host suicides and stays observer (no click)
        cfg += &format!("kill\nts_bot_stop 0\n{}", secs(5.0));
        cfg += "ts_bot_quota 4\n";
        for i in 0..7 {
            cfg += &secs(10.0);
            cfg += &status(&format!("bots {i}"));
        }
        cfg += &shot("bots");
        cfg += "echo XTASK: done\n";

        let run = self.run_script(root, "dojo", &cfg, Duration::from_secs(600));
        let text = &run.text;
        for line in text.lines().filter(|l| {
            l.contains("XTASK:") || l.contains("spawned (#1)") || l.contains("func_breakable") || l.contains(" killed ") || l.contains("ts_tp")
        }) {
            println!("   | {}", line.trim_end());
        }

        let dumps = status_dumps(text);
        let dump = |tag: &str| dumps.iter().find(|d| d.0 == tag).map(|d| d.1.clone()).unwrap_or_default();
        let host = |tag: &str| dump(tag).into_iter().find(|p| p.idx == 1);
        let host_name = host("spawned").map(|p| p.name).unwrap_or_default();
        let spawns: Vec<&str> = text.lines().filter(|l| l.contains("[ts] ") && l.contains("spawned (#1)")).collect();

        // Power-up: held after the pickup, then active with the slow-motion time scales.
        let picked = host("picked up").is_some_and(|p| p.held == "0x1");
        let used = host("power-up used").is_some_and(|p| p.active == "0x1" && (p.slow_target - 0.6).abs() < 0.01);
        let bot_slowed = dump("power-up used").iter().any(|p| p.idx != 1 && (p.slow_target - 0.35).abs() < 0.01);
        // Grenade: a breakable broken by the host's grenade (inflictor class "grenade", blast damage).
        let grenade_break = text.lines().any(|l| l.contains("func_breakable") && l.contains(&format!("broken by {host_name} (grenade")));
        // The host's kill and death.
        let host_kill = text.lines().any(|l| l.contains(&format!("{host_name} killed Bot ")));
        let host_frag = host("bot shot").is_some_and(|p| p.frags >= 1);
        let host_died = host("host dead").is_some_and(|p| p.deaths >= 1 && p.state != "alive");
        let respawned = spawns.len() >= 2 && host("host respawned").is_some_and(|p| p.state == "alive");
        // Bots: positions in consecutive dumps of the same life, and bot-on-bot kills.
        let bot_dumps: Vec<&Vec<PlayerStatus>> = dumps.iter().filter(|d| d.0.starts_with("bots ")).map(|d| &d.1).collect();
        let mut moved: std::collections::BTreeMap<String, f32> = Default::default();
        for w in bot_dumps.windows(2) {
            for a in w[0].iter().filter(|p| p.name.starts_with("Bot ") && p.state == "alive") {
                if let Some(b) = w[1].iter().find(|b| b.idx == a.idx && b.name == a.name && b.deaths == a.deaths && b.state == "alive") {
                    let d = ((a.org[0] - b.org[0]).powi(2) + (a.org[1] - b.org[1]).powi(2)).sqrt();
                    let m = moved.entry(a.name.clone()).or_default();
                    *m = m.max(d);
                }
            }
        }
        println!("   bots' largest move between dumps: {moved:?}");
        let bots_moved = moved.len() >= 3 && moved.values().filter(|&&d| d > 64.0).count() >= moved.len() - 1;
        let bot_kills = text.lines().filter(|l| l.contains("Bot ") && l.contains(" killed Bot ")).count();
        println!("   bot-on-bot kills: {bot_kills}");
        let last_bots = bot_dumps.last().map(|d| d.iter().filter(|p| p.name.starts_with("Bot ")).count()).unwrap_or(0);

        let checks = [
            ("the host spawned in by clicking", !spawns.is_empty()),
            ("picked up the slow-motion power-up", picked),
            ("double-tap +use activated it (host time scale 0.6)", used),
            ("slow motion slowed the bot in view (0.35)", bot_slowed),
            ("the thrown grenade broke a func_breakable", grenade_break),
            ("the host shot a bot dead", host_kill && host_frag),
            ("the host was killed", host_died),
            ("the host clicked to respawn", respawned),
            ("four bots played", last_bots >= 4),
            ("bots moved around", bots_moved),
            ("bots killed each other", bot_kills >= 2),
            ("the script finished", run.done),
            ("the game kept running to the end", !run.exited),
            ("no panic or crash", !crashed(text)),
        ];
        self.report(root, "dojo", &run, &checks)
    }
}

fn crashed(text: &str) -> bool {
    ["PANIC", "Sys_Crash", "Segmentation", "FATAL ERROR"].iter().any(|m| text.contains(m))
}

/// Links TS content into `gamedir`. Directories and binary assets are symlinked; small text
/// files the engine may rewrite (configs) are copied, so the source install is never modified.
fn link_ts_content(ts: &Path, gamedir: &Path) {
    const DIRS: &[&str] = &["events", "gfx", "logos", "maps", "models", "overviews", "resource", "sound", "sprites"];
    const LINK_EXT: &[&str] = &["wad", "tga", "bmp", "tsa", "scr", "ico", "res"];
    const COPY_EXT: &[&str] = &["cfg", "txt", "rc"];
    fs::create_dir_all(gamedir).unwrap();
    let Ok(entries) = fs::read_dir(ts) else {
        eprintln!("TS content not found at {} (set TS_DIR)", ts.display());
        exit(1)
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name_s = name.to_string_lossy();
        let dest = gamedir.join(&name);
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let ext = Path::new(&*name_s).extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if is_dir && DIRS.iter().any(|d| d.eq_ignore_ascii_case(&name_s)) || !is_dir && LINK_EXT.contains(&ext.as_str()) {
            if fs::symlink_metadata(&dest).is_err() {
                std::os::unix::fs::symlink(e.path(), &dest).unwrap();
            }
        } else if !is_dir && COPY_EXT.contains(&ext.as_str()) && !dest.exists() {
            fs::copy(e.path(), &dest).unwrap();
        }
    }
}

/// Steam's engine (HL25 `swds.dll`, and the GoldSrc engines it inherits the code from) reads the
/// game DLL's export names from the file itself: it finds the section *named* `.rdata` and
/// translates the export directory's RVAs with that section's mapping, which is where MSVC puts
/// exports. MinGW's ld emits a separate `.edata` section, so the translation lands on garbage and
/// the engine crashes (or, by luck, reads junk) while loading the library. The Windows loader
/// ignores section names, so rename `.rdata` -> `.rodata` and `.edata` -> `.rdata`.
fn msvc_style_export_section(path: &Path) {
    let mut pe = fs::read(path).unwrap();
    let rd = |b: &[u8], o: usize, n: usize| -> usize { b[o..o + n].iter().rev().fold(0, |a, &x| (a << 8) | x as usize) };
    let nt = rd(&pe, 0x3c, 4);
    assert_eq!(&pe[nt..nt + 4], b"PE\0\0", "{} is not a PE file", path.display());
    let count = rd(&pe, nt + 6, 2);
    let first = nt + 24 + rd(&pe, nt + 20, 2);
    let names: Vec<[u8; 8]> = (0..count).map(|i| pe[first + 40 * i..first + 40 * i + 8].try_into().unwrap()).collect();
    if !names.iter().any(|n| n == b".edata\0\0") {
        return; // already MSVC-style
    }
    for (i, n) in names.iter().enumerate() {
        let new: Option<&[u8; 8]> = match n {
            b".rdata\0\0" => Some(b".rodata\0"),
            b".edata\0\0" => Some(b".rdata\0\0"),
            _ => None,
        };
        if let Some(new) = new {
            pe[first + 40 * i..first + 40 * i + 8].copy_from_slice(new);
        }
    }
    fs::write(path, pe).unwrap();
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to.parent().unwrap()).unwrap();
    // Remove first so a running engine keeps its mapped copy instead of crashing.
    let _ = fs::remove_file(to);
    fs::copy(from, to).unwrap_or_else(|e| panic!("{} -> {}: {e}", from.display(), to.display()));
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            copy(&entry.path(), &dest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ts_status_lines() {
        let line = "[ts]  #2 Bot Neo          alive hp=100 ar=0 frags=3 deaths=1 cash=1200 team='' held=0x1/3s active=0x1 slow=0.35->0.35 award=0 spree=0 org=(0 -480 36)";
        let p = parse_status_line(line).unwrap();
        assert_eq!((p.idx, p.name.as_str(), p.state.as_str(), p.frags, p.deaths), (2, "Bot Neo", "alive", 3, 1));
        assert_eq!((p.held.as_str(), p.active.as_str(), p.slow_target, p.org), ("0x1", "0x1", 0.35, [0.0, -480.0, 36.0]));
        let text = "XTASK: status a\n[ts] status: Dm\n".to_owned() + line + "\nXTASK: status b\n";
        let dumps = status_dumps(&text);
        assert_eq!((dumps.len(), dumps[0].1.len(), dumps[1].1.len()), (2, 1, 0));
    }

    #[test]
    fn secs_uses_the_aliases() {
        assert_eq!(secs(2.35), "w100;w100;w10;w10;w10;wait;wait;wait;wait;wait;\n");
    }
}
