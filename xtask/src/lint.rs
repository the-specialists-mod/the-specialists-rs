//! Source checks that the compiler can't do for us.
//!
//! The game crates are `#![deny(unsafe_code)]`, but that lint doesn't fire on `unsafe` blocks
//! that come from another crate's macro, so `goldsrc`'s engine-call macros (`eng!`, `cl_eng!`,
//! `ev!`, `efx!`, `tri!`) would compile there and smuggle `unsafe` past the boundary
//! (AGENTS.md, rule 2). [`engine_macro_uses`] finds them; `cargo xtask build` and the unit test
//! below fail on any hit. Each game crate's `clippy.toml` also lists the macros under
//! `disallowed-macros`, a second guard that sees through renamed imports and local macros.
//!
//! `cargo xtask lint` runs the whole clippy set with warnings as errors on every build target,
//! including the `undocumented_unsafe_blocks` lint that `goldsrc` and the `*-dll` crates enable
//! (AGENTS.md, rule 4). The Windows-only code (`goldsrc::platform::win`, the WinMM joystick) is
//! only compiled, and so only linted, for a Windows target.

use crate::roots::{Os, Target};
use std::fs;
use std::path::Path;
use std::process::Command;

/// `goldsrc`'s exported macros that expand to an `unsafe` engine call.
pub const ENGINE_MACROS: [&str; 5] = ["eng", "cl_eng", "ev", "efx", "tri"];

/// The crates that must reach the engine only through `goldsrc`'s safe API.
pub const GAME_CRATES: [&str; 3] = ["crates/server", "crates/client", "crates/shared"];

/// Every `file:line: text` in the game crates' sources that invokes (`tri!(...)`) or names
/// (`use goldsrc::tri;`, `goldsrc::eng!`) an engine-call macro. Line comments are skipped.
pub fn engine_macro_uses(repo: &Path) -> Vec<String> {
    let mut hits = Vec::new();
    for krate in GAME_CRATES {
        let mut files = Vec::new();
        rust_files(&repo.join(krate).join("src"), &mut files);
        files.sort();
        for f in files {
            let Ok(text) = fs::read_to_string(&f) else { continue };
            for (i, line) in text.lines().enumerate() {
                if uses_engine_macro(line) {
                    let rel = f.strip_prefix(repo).unwrap_or(&f);
                    hits.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
                }
            }
        }
    }
    hits
}

/// `cargo xtask lint`: the engine-macro check, then `cargo clippy --workspace --all-targets
/// -- -D warnings` for every target. A workspace build unifies `goldsrc`'s features, so both its
/// `client` and `server` halves are linted. `xtask` is host tooling (it uses `std::os::unix`), so
/// it's linted on the Linux targets only. Returns false on any failure.
pub fn run(repo: &Path) -> bool {
    let hits = engine_macro_uses(repo);
    for h in &hits {
        eprintln!("engine-call macro in a game crate: {h}");
    }
    let mut ok = hits.is_empty();
    for t in &Target::ALL {
        crate::say!("== cargo clippy --workspace --all-targets --target {} -- -D warnings", t.triple);
        let mut c = Command::new(std::env::var("CARGO").unwrap_or("cargo".into()));
        c.current_dir(repo).args(["clippy", "--workspace", "--all-targets", "--target", t.triple]);
        if t.os == Os::Windows {
            c.args(["--exclude", "xtask"]);
        }
        ok &= c.args(["--", "-D", "warnings"]).status().expect("cargo clippy").success();
    }
    ok
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// True if the code part of `line` (before any `//`) has `name!`, `goldsrc::name` or
/// `goldsrc::{.., name, ..}` for an engine-call macro `name`, as a whole identifier. (A use-group
/// split over several lines isn't caught, but invoking the macro is.)
fn uses_engine_macro(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or("");
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    ENGINE_MACROS.iter().any(|m| {
        code.match_indices(m).any(|(at, _)| {
            let before = code[..at].chars().next_back();
            let after = code[at + m.len()..].chars().next();
            if before.is_some_and(ident) || after.is_some_and(ident) {
                return false;
            }
            let bang = after == Some('!') && !code[at + m.len()..].starts_with("!=");
            // `use goldsrc::{tri, ...}`: a top-level item of a goldsrc use-group, not a path.
            let grouped = code.contains("goldsrc::{")
                && matches!(code[..at].trim_end().chars().next_back(), Some('{' | ','))
                && !code[at + m.len()..].starts_with("::");
            bang || grouped || code[..at].ends_with("goldsrc::")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_engine_macros() {
        for bad in [
            "    goldsrc::eng!(pfnCVarSetString(a, b));",
            "tri!(End());",
            "let x = cl_eng!(GetMaxClients());",
            "use goldsrc::tri;",
            "use goldsrc::efx as fx;",
            "use goldsrc::{sys::*, tri as t};",
            "    goldsrc::ev!(EV_PopPMStates());",
        ] {
            assert!(uses_engine_macro(bad), "{bad}");
        }
        for ok in [
            "use goldsrc::ext::tri;",
            "tri::begin(TRI_QUADS as i32);",
            "let eng = engine();",
            "eng.time()",
            "if ev!=0 {}",
            "// don't call tri!(End()) here",
            "geng!(x)",
            "use goldsrc::ext::render::{self as r, Vec3};",
            "use goldsrc::{ext::tri, sys::*};",
        ] {
            assert!(!uses_engine_macro(ok), "{ok}");
        }
    }

    /// AGENTS.md rule 2: game crates reach the engine only through goldsrc's safe wrappers.
    #[test]
    fn game_crates_do_not_use_engine_macros() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let hits = engine_macro_uses(repo);
        assert!(hits.is_empty(), "engine-call macros in game crates (add a safe wrapper to goldsrc):\n{}", hits.join("\n"));
    }
}
