//! Engine roots the port is validated on, and how to start each one's dedicated server.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{env, fs};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// Xash3D FWGS, native amd64 (`xash` dedicated, `xash3d` client).
    Fwgs,
    /// ReHLDS on pre-anniversary HLDS (steam_legacy branch).
    ReHlds,
    /// Stock Valve HLDS, post-anniversary (HL25) branch.
    Hlds25,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub triple: &'static str,
    pub os: Os,
    pub platform: &'static str,
    /// Installed names. FWGS on 64-bit takes the gamedll name from gameinfo.txt minus any _i?86 and
    /// appends _amd64: dlls/ts_i386.so -> ts_amd64.so on Linux, dlls\mp.dll -> mp_amd64.dll on Windows.
    pub server_name: &'static str,
    pub client_name: &'static str,
}

impl Target {
    pub const LINUX_I686: Target = Target {
        triple: "i686-unknown-linux-gnu",
        os: Os::Linux,
        platform: "linux-i686",
        server_name: "ts_i386.so",
        client_name: "client.so",
    };
    pub const WINDOWS_I686: Target = Target {
        triple: "i686-pc-windows-gnu",
        os: Os::Windows,
        platform: "windows-i686",
        server_name: "mp.dll",
        client_name: "client.dll",
    };
    pub const LINUX_X86_64: Target = Target {
        triple: "x86_64-unknown-linux-gnu",
        os: Os::Linux,
        platform: "linux-x86_64",
        server_name: "ts_amd64.so",
        client_name: "client_amd64.so",
    };
    pub const WINDOWS_X86_64: Target = Target {
        triple: "x86_64-pc-windows-gnu",
        os: Os::Windows,
        platform: "windows-x86_64",
        server_name: "mp_amd64.dll",
        client_name: "client_amd64.dll",
    };
    pub const ALL: [Target; 4] = [Self::LINUX_I686, Self::WINDOWS_I686, Self::LINUX_X86_64, Self::WINDOWS_X86_64];
}

#[derive(Clone)]
pub struct Root {
    pub name: &'static str,
    pub os: Os,
    pub engine: Engine,
    pub path: PathBuf,
    /// The libraries it loads.
    pub target: Target,
}

fn home() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap())
}

static PARALLEL: AtomicBool = AtomicBool::new(false);

/// Checks are running on several roots at once (see `Root::cleanup`).
pub fn set_parallel(on: bool) {
    PARALLEL.store(on, Ordering::Relaxed);
}

/// Stops every Wine process in the shared prefix.
pub fn wine_cleanup() {
    let _ = Command::new("wineserver").env("WINEPREFIX", wineprefix()).arg("-k").status();
}

pub fn wineprefix() -> PathBuf {
    env::var_os("WINEPREFIX").map(PathBuf::from).unwrap_or_else(|| home().join("games/wineprefix"))
}

impl Root {
    pub fn all() -> Vec<Root> {
        let mk = |name: &'static str, target: Target, engine, dir: &str| {
            let var = format!("TS_ROOT_{}", name.to_ascii_uppercase().replace('-', "_"));
            let path = env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home().join("games").join(dir));
            Root { name, os: target.os, engine, path, target }
        };
        vec![
            mk("fwgs", Target::LINUX_X86_64, Engine::Fwgs, "xash"),
            mk("fwgs-win", Target::WINDOWS_I686, Engine::Fwgs, "xash_win"),
            mk("fwgs-win64", Target::WINDOWS_X86_64, Engine::Fwgs, "xash_win64"),
            mk("rehlds-linux", Target::LINUX_I686, Engine::ReHlds, "hlds_linux"),
            mk("rehlds-win", Target::WINDOWS_I686, Engine::ReHlds, "hlds_win"),
            mk("hlds25-linux", Target::LINUX_I686, Engine::Hlds25, "hlds25_linux"),
            mk("hlds25-win", Target::WINDOWS_I686, Engine::Hlds25, "hlds25_win"),
        ]
    }

    pub fn exists(&self) -> bool {
        self.path
            .join(match (self.engine, self.os) {
                (Engine::Fwgs, Os::Linux) => "xash",
                (Engine::Fwgs, Os::Windows) => "xash3d.exe",
                (_, Os::Linux) => "hlds_linux",
                (_, Os::Windows) => "hlds.exe",
            })
            .exists()
    }

    pub fn target(&self) -> Target {
        self.target
    }

    /// Where the console log ends up (Windows hlds writes it with -condebug).
    pub fn log_path(&self) -> PathBuf {
        match self.engine {
            Engine::Fwgs => self.path.join("engine.log"),
            _ => self.path.join("qconsole.log"),
        }
    }

    pub fn server_command(&self, map: &str, port: u16, bots: u32) -> Command {
        let common = |c: &mut Command| {
            c.current_dir(&self.path).args(["-game", "ts", "-port", &port.to_string(), "+maxplayers", "8", "+sv_lan", "1"]);
            // Game cvars don't exist yet when command-line `+cvar` runs on GoldSrc, so set them
            // from the per-map server config instead (the engine runs it after GameInit).
            // `addbot` checks that a dedicated server adds a bot for it (a port deviation: TS refuses).
            let cfg = format!("// Written by cargo xtask server.\nexec server.cfg\nts_bot_quota {bots}\naddbot\n");
            std::fs::write(self.path.join("ts").join("xtask_server.cfg"), cfg).unwrap();
            // FWGS applies command-line cvars after GameInit, GoldSrc reads the config: pass both.
            c.args(["+ts_bot_quota", &bots.to_string(), "+servercfgfile", "xtask_server.cfg", "+map", map]);
        };
        match (self.engine, self.os) {
            (Engine::Fwgs, Os::Windows) => {
                // FWGS 0.21 win32 (as shipped with xash3d-ts) or win32-amd64 (the FWGS continuous
                // build): one exe, -dedicated for servers.
                let mut c = Command::new("wine");
                c.env("WINEPREFIX", wineprefix()).env("WINEDEBUG", "-all");
                c.arg("xash3d.exe").args(["-dedicated", "-dev", "1", "-log"]);
                common(&mut c);
                c
            }
            (Engine::Fwgs, Os::Linux) => {
                let mut c = Command::new(self.path.join("xash"));
                c.arg("-dev").arg("1");
                common(&mut c);
                c
            }
            (_, Os::Linux) => {
                let mut c = Command::new(self.path.join("hlds_linux"));
                c.env("LD_LIBRARY_PATH", &self.path).arg("-condebug");
                common(&mut c);
                c
            }
            (_, Os::Windows) => {
                let mut c = Command::new("wine");
                c.env("WINEPREFIX", wineprefix()).env("WINEDEBUG", "-all");
                c.arg("hlds.exe").args(["-console", "-condebug"]);
                common(&mut c);
                c
            }
        }
    }

    /// Windows servers run under Wine: make sure nothing is left behind. While checks run in
    /// parallel this waits for [`wine_cleanup`] at the end (`wineserver -k` would kill the other
    /// roots' Wine processes too).
    pub fn cleanup(&self) {
        if self.os == Os::Windows && !PARALLEL.load(Ordering::Relaxed) {
            wine_cleanup();
        }
    }

    /// The engine console log plus the captured stdout.
    pub fn read_log(&self, repo: &Path) -> String {
        let mut s = fs::read(self.log_path()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        if let Ok(b) = fs::read(repo.join("target").join(format!("server-{}.log", self.name))) {
            s.push_str(&String::from_utf8_lossy(&b));
        }
        s.replace('\r', "")
    }
}
