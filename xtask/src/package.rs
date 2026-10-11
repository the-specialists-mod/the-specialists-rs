//! `cargo xtask package-xash-win`: the Windows (Xash3D FWGS) release archive.
//!
//! ```text
//! ts-xash-win/                 $XASH_WIN_DIR (default ../xash3d-ts): the engine and valve/,
//!                              symlinks followed, without its ts/, .git and engine.log
//! ts-xash-win/ts/              $TS_ORIGINAL_DIR (default ../ts_original): stock TS 3.0, symlinks
//!                              followed, without screenshots, logs, caches of the last game and
//!                              the Linux server libraries
//!            + dist/windows-i686/ts/ (`dist --release`): the port's libraries and game files
//! ts-xash-win/README-RUST-PORT.txt   (CRLF) the commit it was built from
//! ```
//!
//! written to `$TS_PACKAGE_OUT` (default `~/ts-xash-win-release.tar.gz`) with the system `tar`.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};
use std::{env, fs};

/// The archive's top directory.
pub const TOP: &str = "ts-xash-win";

/// Where the package's inputs and output are.
#[derive(Debug, PartialEq)]
pub struct Paths {
    pub xash_win: PathBuf,
    pub ts_original: PathBuf,
    pub out: PathBuf,
}

impl Paths {
    /// From the environment (`XASH_WIN_DIR`, `TS_ORIGINAL_DIR`, `TS_PACKAGE_OUT`), else the
    /// directories next to the checkout (found from `repo` up, so worktrees work too) and `~`.
    pub fn from_env(repo: &Path) -> Paths {
        Paths::resolve(repo, |k| env::var_os(k).map(PathBuf::from))
    }

    pub fn resolve(repo: &Path, var: impl Fn(&str) -> Option<PathBuf>) -> Paths {
        let home = var("HOME").unwrap_or_else(|| PathBuf::from("."));
        Paths {
            xash_win: var("XASH_WIN_DIR").unwrap_or_else(|| sibling(repo, "xash3d-ts")),
            ts_original: var("TS_ORIGINAL_DIR").unwrap_or_else(|| sibling(repo, "ts_original")),
            out: var("TS_PACKAGE_OUT").unwrap_or_else(|| home.join("ts-xash-win-release.tar.gz")),
        }
    }
}

/// `<dir>/<name>` for the nearest ancestor `dir` of `repo` that has it (`../<name>` for a
/// checkout, further up for a worktree under `.claude/worktrees`); `repo/../<name>` if none does.
pub fn sibling(repo: &Path, name: &str) -> PathBuf {
    repo.ancestors().skip(1).map(|d| d.join(name)).find(|p| p.is_dir()).unwrap_or_else(|| repo.join("..").join(name))
}

/// Paths (relative to the engine directory) left out of the package.
pub fn skip_engine(rel: &Path) -> bool {
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
    matches!(first.as_str(), "ts" | ".git") || rel == Path::new("engine.log")
}

/// Paths (relative to the TS content directory) left out of the package: the last game's
/// screenshots, xtask's scripts and markers, console dumps, the spray cache, the voice ban list,
/// and the Linux server libraries (the package has the port's `mp.dll`).
pub fn skip_ts(rel: &Path) -> bool {
    let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let top = rel.components().count() == 1;
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
    first == "scrshots"
        || top && (name.starts_with("xtask_") || name.starts_with("condump") && name.ends_with(".txt"))
        || top && matches!(name.as_str(), "custom.hpk" | "voice_ban.dt")
        || rel == Path::new("dlls/ts_i386.so")
        || rel == Path::new("dlls/ts_i686.so")
}

/// The package's README, with CRLF line ends. `overlay` lists the files the port provides, relative
/// to `ts/`.
pub fn readme(commit: &str, date: &str, overlay: &[String]) -> String {
    let libs = "ts/dlls/mp.dll and ts/cl_dlls/client.dll are the port's server and client libraries;";
    let others: Vec<String> =
        overlay.iter().filter(|f| !matches!(f.as_str(), "dlls/mp.dll" | "cl_dlls/client.dll")).map(|f| format!("ts/{f}")).collect();
    let others = match others.len() {
        0 => String::new(),
        1 => others[0].clone(),
        n => format!("{} and {}", others[..n - 1].join(", "), others[n - 1]),
    };
    let text = format!(
        "The Specialists 3.0 - Rust port, Windows (Xash3D FWGS) release build\n\
         ====================================================================\n\
         \n\
         Built from rust-hlmod-the-specialists commit {commit} {date} (cargo xtask dist --release, windows-i686),\n\
         on the stock TS 3.0 content.\n\
         \n\
         Run:   run_ts.cmd   (xash3d.exe -game ts -console)\n\
         \n\
         {libs}\n\
         {others} come from the port too. Everything else\n\
         (engine, valve/, TS 3.0 maps, models, sounds) is the stock content.\n\
         \n\
         Source: https://github.com/the-specialists-mod/the-specialists-rs\n"
    );
    text.replace('\n', "\r\n")
}

/// Copies `from` into `to` following symlinks, leaving out what `skip` (given the path relative to
/// the top of the copy) says. Returns the files copied, relative.
pub fn copy_tree(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    copy_rec(from, to, Path::new(""), skip, &mut out);
    out
}

fn copy_rec(from: &Path, to: &Path, rel: &Path, skip: &dyn Fn(&Path) -> bool, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(from) else {
        eprintln!("{} not found", from.display());
        exit(1)
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let r = rel.join(e.file_name());
        if skip(&r) {
            continue;
        }
        // `fs::metadata` follows symlinks: a linked directory is copied as a directory.
        let Ok(meta) = fs::metadata(e.path()) else {
            eprintln!("warning: {} is a broken link, skipped", e.path().display());
            continue;
        };
        let dest = to.join(e.file_name());
        if meta.is_dir() {
            fs::create_dir_all(&dest).unwrap();
            copy_rec(&e.path(), &dest, &r, skip, out);
        } else {
            let _ = fs::remove_file(&dest);
            fs::copy(e.path(), &dest).unwrap_or_else(|err| panic!("{} -> {}: {err}", e.path().display(), dest.display()));
            out.push(r.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Stages `ts-xash-win/` under `stage` from the engine, the stock content and the port's `dist`
/// overlay (`dist_ts`), writes the README and archives it to `paths.out` with `tar`.
pub fn package(paths: &Paths, dist_ts: &Path, stage: &Path, commit: &str, date: &str) {
    let top = stage.join(TOP);
    let _ = fs::remove_dir_all(&top);
    fs::create_dir_all(top.join("ts")).unwrap();
    println!("== package: engine from {}", paths.xash_win.display());
    copy_tree(&paths.xash_win, &top, &skip_engine);
    println!("== package: TS 3.0 content from {}", paths.ts_original.display());
    copy_tree(&paths.ts_original, &top.join("ts"), &skip_ts);
    println!("== package: the port from {}", dist_ts.display());
    let overlay = copy_tree(dist_ts, &top.join("ts"), &|_| false);
    fs::write(top.join("README-RUST-PORT.txt"), readme(commit, date, &overlay)).unwrap();
    if let Some(dir) = paths.out.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    println!("== package: tar -czf {}", paths.out.display());
    let ok = Command::new("tar").arg("-czf").arg(&paths.out).arg("-C").arg(stage).arg(TOP).status().is_ok_and(|s| s.success());
    if !ok {
        eprintln!("tar failed");
        exit(1);
    }
    println!("== package: {} ready", paths.out.display());
}

/// `git rev-parse --short HEAD` and the commit's date (`%cs`), "unknown" outside a checkout.
pub fn commit(repo: &Path) -> (String, String) {
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_else(|| "unknown".into())
    };
    (git(&["rev-parse", "--short", "HEAD"]), git(&["log", "-1", "--format=%cs"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_from_env_or_defaults() {
        let repo = Path::new("/nonexistent/dev/hl/repo");
        let p = Paths::resolve(repo, |k| (k == "HOME").then(|| PathBuf::from("/home/u")));
        assert_eq!(p.xash_win, repo.join("../xash3d-ts"));
        assert_eq!(p.ts_original, repo.join("../ts_original"));
        assert_eq!(p.out, PathBuf::from("/home/u/ts-xash-win-release.tar.gz"));
        let p = Paths::resolve(repo, |k| match k {
            "XASH_WIN_DIR" => Some("/e".into()),
            "TS_ORIGINAL_DIR" => Some("/c".into()),
            "TS_PACKAGE_OUT" => Some("/o.tar.gz".into()),
            _ => None,
        });
        assert_eq!(p, Paths { xash_win: "/e".into(), ts_original: "/c".into(), out: "/o.tar.gz".into() });
    }

    #[test]
    fn sibling_found_from_a_worktree() {
        let root = std::env::temp_dir().join(format!("xtask-sibling-{}", std::process::id()));
        let wt = root.join("repo/.claude/worktrees/agent");
        fs::create_dir_all(&wt).unwrap();
        fs::create_dir_all(root.join("ts_original")).unwrap();
        assert_eq!(sibling(&wt, "ts_original"), root.join("ts_original"));
        assert_eq!(sibling(&root.join("repo"), "ts_original"), root.join("ts_original"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn package_exclusions() {
        for p in ["ts", ".git", "engine.log", ".git/config", "ts/maps/a.bsp"] {
            assert!(skip_engine(Path::new(p)), "{p}");
        }
        for p in ["xash3d.exe", "valve/liblist.gam", "valve/ts", "run_ts.cmd"] {
            assert!(!skip_engine(Path::new(p)), "{p}");
        }
        for p in [
            "scrshots",
            "scrshots/a.bmp",
            "xtask_play.cfg",
            "xtask_play_done",
            "condump000.txt",
            "custom.hpk",
            "voice_ban.dt",
            "dlls/ts_i386.so",
            "dlls/ts_i686.so",
        ] {
            assert!(skip_ts(Path::new(p)), "{p}");
        }
        for p in ["dlls/mp.dll", "maps/ts_dojo.bsp", "bots.txt", "maps/xtask_x.bsp", "condump.cfg", "models/custom.hpk"] {
            assert!(!skip_ts(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn copy_follows_links_and_skips() {
        let root = std::env::temp_dir().join(format!("xtask-copy-{}", std::process::id()));
        let (src, dst) = (root.join("src"), root.join("dst"));
        fs::create_dir_all(src.join("dlls")).unwrap();
        fs::create_dir_all(root.join("elsewhere")).unwrap();
        fs::write(root.join("elsewhere/a.bsp"), "map").unwrap();
        std::os::unix::fs::symlink(root.join("elsewhere"), src.join("maps")).unwrap();
        fs::write(src.join("dlls/ts_i386.so"), "x").unwrap();
        fs::write(src.join("dlls/mp.dll"), "x").unwrap();
        fs::write(src.join("custom.hpk"), "x").unwrap();
        fs::create_dir_all(&dst).unwrap();
        let files = copy_tree(&src, &dst, &skip_ts);
        assert_eq!(files, ["dlls/mp.dll", "maps/a.bsp"]);
        assert!(!fs::symlink_metadata(dst.join("maps")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(dst.join("maps/a.bsp")).unwrap(), "map");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn readme_names_the_commit_with_crlf() {
        let r = readme(
            "abc1234",
            "2026-10-12",
            &["cl_dlls/client.dll".into(), "delta.lst".into(), "dlls/mp.dll".into(), "gameinfo.txt".into(), "liblist.gam".into()],
        );
        assert!(r.starts_with("The Specialists 3.0 - Rust port, Windows (Xash3D FWGS) release build\r\n"));
        assert!(r.contains("commit abc1234 2026-10-12 (cargo xtask dist --release, windows-i686),\r\n"));
        assert!(r.contains("Run:   run_ts.cmd   (xash3d.exe -game ts -console)\r\n"));
        assert!(r.contains("ts/delta.lst, ts/gameinfo.txt and ts/liblist.gam come from the port too."));
        assert!(r.ends_with("Source: https://github.com/the-specialists-mod/the-specialists-rs\r\n"));
        assert!(!r.replace("\r\n", "").contains('\n'), "every line ends in CRLF");
    }
}
