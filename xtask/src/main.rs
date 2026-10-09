//! `cargo xtask install [--release] [--game <dir>]`: build both libraries and install them,
//! plus `game/mymod/*`, into an Xash3D FWGS mod directory under their engine-expected names.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};
use std::{env, fs};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("install") {
        eprintln!("usage: cargo xtask install [--release] [--game <dir>]");
        exit(2);
    }
    let release = args.iter().any(|a| a == "--release");
    let game = args
        .iter()
        .position(|a| a == "--game")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var("HOME").unwrap()).join("games/xash/mymod"));

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut build = Command::new(env::var("CARGO").unwrap_or("cargo".into()));
    build.current_dir(root).args(["build", "-p", "mymod-server", "-p", "mymod-client"]);
    if release {
        build.arg("--release");
    }
    if !build.status().expect("cargo build").success() {
        exit(1);
    }

    // FWGS appends the arch suffix on non-x86 targets (library-naming.md).
    let suffix = match env::consts::ARCH {
        "x86_64" => "_amd64",
        "aarch64" => "_arm64",
        _ => "",
    };
    let out = root.join("target").join(if release { "release" } else { "debug" });
    copy_dir(&root.join("game/mymod"), &game);
    copy(&out.join("libmymod.so"), &game.join(format!("dlls/mymod{suffix}.so")));
    copy(&out.join("libclient.so"), &game.join(format!("cl_dlls/client{suffix}.so")));
    println!("installed to {}", game.display());
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to.parent().unwrap()).unwrap();
    // Remove first so a running engine keeps its mapped copy instead of crashing.
    let _ = fs::remove_file(to);
    fs::copy(from, to).unwrap_or_else(|e| panic!("{} -> {}: {e}", from.display(), to.display()));
    println!("  {}", to.display());
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            copy(&entry.path(), &dest);
        }
    }
}
