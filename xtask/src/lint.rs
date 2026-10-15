//! Source checks that the compiler can't do for us.
//!
//! The game crates are `#![deny(unsafe_code)]`, but that lint doesn't fire on `unsafe` blocks
//! that come from another crate's macro, so `goldsrc`'s engine-call macros (`eng!`, `cl_eng!`,
//! `ev!`, `efx!`, `tri!`) would compile there and smuggle `unsafe` past the boundary
//! (AGENTS.md, rule 2). [`engine_macro_uses`] finds them; `cargo xtask build` and the unit test
//! below fail on any hit. Each game crate's `clippy.toml` also lists the macros under
//! `disallowed-macros`, a second guard that sees through renamed imports and local macros.
//!
//! The gameplay plugins keep TS 2.1 apart from the 3.0 code (issue #100,
//! `docs/implementation/gameplay-plugins.md`): [`ruleset_leaks`] finds 2.1 identifiers outside
//! `crates/ts21/`, other than the two libraries' registration of the plugin, and any other crate
//! that depends on it. `cargo xtask lint` also builds and tests the libraries with one ruleset
//! each ([`SINGLE_RULESET`]), so each ruleset builds and passes on its own.
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
pub const GAME_CRATES: [&str; 10] = [
    "crates/server",
    "crates/client",
    "crates/shared",
    "crates/gameplay",
    "crates/ts21",
    "crates/ts21/server",
    "crates/ts21/client",
    "crates/ts30",
    "crates/ts30/server",
    "crates/ts30/client",
];

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

/// The TS 2.1 plugin's directory (`ts21-gameplay` and its `server/` and `client/` packages): the
/// only one that may name 2.1.
pub const RULESET_CRATE: &str = "crates/ts21";

/// The libraries that register the plugin (behind their `ts21` feature): they may name the
/// plugin's crate and the feature, nothing else of 2.1.
pub const REGISTERING_CRATES: [&str; 2] = ["crates/server-dll", "crates/client-dll"];

/// Identifier fragments that name TS 2.1 or a 2.1 mode (lower case, matched after a
/// non-alphanumeric character, so `rules_v21` counts and `tests21` or `dev21` don't).
pub const RULESET_TOKENS: [&str; 7] = ["ts21", "ts_21", "ts2_1", "ts_2_1", "v21", "v2_1", "gp21"];

/// Every `file:line: text` outside [`RULESET_CRATE`] (sources, manifests and configs under
/// `crates/`, and xtask's manifest) that names TS 2.1 ([`RULESET_TOKENS`]), except the registering
/// libraries' references to the plugin crate and its feature. A crate that depends on the
/// plugin has to name it in its manifest, so this also finds those.
pub fn ruleset_leaks(repo: &Path) -> Vec<String> {
    let mut files = vec![repo.join("xtask/Cargo.toml")];
    text_files(&repo.join("crates"), &mut files);
    files.sort();
    let mut hits = Vec::new();
    for f in files {
        let rel = f.strip_prefix(repo).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        if rel.starts_with(&format!("{RULESET_CRATE}/")) || rel.contains("/target/") {
            continue;
        }
        let registering = REGISTERING_CRATES.iter().any(|c| rel.starts_with(&format!("{c}/")));
        let Ok(text) = fs::read_to_string(&f) else { continue };
        for (i, line) in text.lines().enumerate() {
            if names_ruleset(line, registering) {
                hits.push(format!("{rel}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    hits
}

/// True if `line` names TS 2.1 ([`RULESET_TOKENS`]). With `registering`, the references a
/// library needs to register the plugin are allowed: the plugin's packages (`ts21_gameplay::ID`,
/// `ts21-server`, `ts21_client::PLUGIN`, ..., their paths `../ts21`, `../ts21/server` and the doc
/// `ts21.md`) and the `ts21` feature (`"ts21"`, `` `ts21` ``, `ts21 = [...]`).
fn names_ruleset(line: &str, registering: bool) -> bool {
    let l = line.to_ascii_lowercase();
    RULESET_TOKENS.iter().any(|t| {
        l.match_indices(t).any(|(at, _)| {
            if l[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric()) {
                return false;
            }
            !(registering && *t == "ts21" && registration(&l[..at], &l[at + t.len()..]))
        })
    })
}

/// Whether a `ts21` between `before` and `after` is one of a registering library's references
/// ([`names_ruleset`]).
fn registration(before: &str, after: &str) -> bool {
    let package = ["gameplay", "server", "client"]
        .iter()
        .any(|p| after.strip_prefix(['_', '-']).is_some_and(|a| a.starts_with(p)) || before.ends_with(&format!("ts21_{p}::")));
    let path = (before.ends_with("../") && (after.starts_with('"') || after.starts_with('/'))) || after.starts_with(".md");
    let quoted = |q: char| before.ends_with(q) && after.starts_with(q);
    let feature = quoted('"') || quoted('`') || (before.trim().is_empty() && after.trim_start().starts_with('='));
    package || path || feature
}

/// The one-ruleset checks: the two libraries with a single gameplay ruleset feature each (the
/// first is the default plugin then) build, and their tests and the core crates' and that
/// ruleset's pass. Each entry is the ruleset's name and its packages.
pub const SINGLE_RULESET: [(&str, [&str; 3]); 2] =
    [("ts30", ["ts30-gameplay", "ts30-server", "ts30-client"]), ("ts21", ["ts21-gameplay", "ts21-server", "ts21-client"])];

/// The `cargo test` arguments for one [`SINGLE_RULESET`] entry.
pub fn single_ruleset_args(feature: &str, packages: &[&str]) -> Vec<String> {
    let mut a = Vec::new();
    for p in ["ts-server-dll", "ts-client-dll", "ts-server", "ts-client", "shared", "ts-gameplay"].iter().chain(packages) {
        a.extend(["-p".to_owned(), (*p).to_owned()]);
    }
    a.extend(["--no-default-features".to_owned(), "--features".to_owned()]);
    a.push(format!("ts-server-dll/{feature},ts-client-dll/{feature}"));
    a
}

/// `cargo xtask lint`: the engine-macro and ruleset-isolation checks, `cargo test` of the core
/// crates and libraries with each ruleset alone ([`SINGLE_RULESET`]), then `cargo clippy
/// --workspace --all-targets -- -D warnings` for every target. A workspace build unifies
/// `goldsrc`'s features, so both its `client` and `server` halves are linted. `xtask` is host
/// tooling (it uses `std::os::unix`), so it's linted on the Linux targets only. Returns false on
/// any failure.
pub fn run(repo: &Path) -> bool {
    let hits = engine_macro_uses(repo);
    for h in &hits {
        eprintln!("engine-call macro in a game crate: {h}");
    }
    let leaks = ruleset_leaks(repo);
    for h in &leaks {
        eprintln!("TS 2.1 named outside crates/ts21 (keep 2.1 in its plugin): {h}");
    }
    let mut ok = hits.is_empty() && leaks.is_empty();
    for (feature, packages) in SINGLE_RULESET {
        let args = single_ruleset_args(feature, &packages);
        crate::say!("== cargo test {} (the {feature} ruleset alone)", args.join(" "));
        let mut c = Command::new(std::env::var("CARGO").unwrap_or("cargo".into()));
        ok &= c.current_dir(repo).arg("test").args(&args).status().expect("cargo test").success();
    }
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

/// The sources, manifests and configs under `dir` (`.rs`, `.toml`).
fn text_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            text_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "toml") {
            out.push(p);
        }
    }
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

    #[test]
    fn detects_ruleset_names() {
        for bad in ["use ts21_gameplay::TS21;", "if v21 {", "let gp21 = 1;", "fn rules_ts_2_1() {}", "// a TS21 rule"] {
            assert!(names_ruleset(bad, false), "{bad}");
        }
        for bad in ["if v21 {", "const TS21_SPEED: f32 = 310.0;", "let ts21 = true;"] {
            assert!(names_ruleset(bad, true), "registering crate: {bad}");
        }
        for ok in [
            "ts21-gameplay = { path = \"../ts21\", optional = true }",
            "ts21-server = { path = \"../ts21/server\", optional = true }",
            "ts21 = [\"dep:ts21-gameplay\", \"dep:ts21-client\"]",
            "    ts_client::gameplay::register(ts21_gameplay::ID, &ts21_client::PLUGIN);",
            "default = [\"ts21\"]",
            "    #[cfg(feature = \"ts21\")]",
            "    ts_server::gameplay::register(ts21_gameplay::ID, &ts21_gameplay::TS21);",
        ] {
            assert!(!names_ruleset(ok, true), "registering crate: {ok}");
            assert!(names_ruleset(ok, false), "other crate: {ok}");
        }
        for ok in ["fn tests21() {}", "let dev21 = 0;", "x = 2.1;", "assert_eq!(n, 21);"] {
            assert!(!names_ruleset(ok, false), "{ok}");
        }
    }

    /// Issue #100: TS 2.1 lives only in its plugin crate.
    #[test]
    fn ruleset_stays_in_its_plugin() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let hits = ruleset_leaks(repo);
        assert!(hits.is_empty(), "TS 2.1 named outside crates/ts21 (keep 2.1 in its plugin):\n{}", hits.join("\n"));
    }

    /// AGENTS.md rule 2: game crates reach the engine only through goldsrc's safe wrappers.
    #[test]
    fn game_crates_do_not_use_engine_macros() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let hits = engine_macro_uses(repo);
        assert!(hits.is_empty(), "engine-call macros in game crates (add a safe wrapper to goldsrc):\n{}", hits.join("\n"));
    }
}
