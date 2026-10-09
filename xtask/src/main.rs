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
