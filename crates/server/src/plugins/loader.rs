//! Which `tsmod_*` files to load (docs/implementation/plugin-api.md §1.2). The FFI (opening the
//! library, calling its `TSRS_PluginLoad`) is `ts-server-dll`'s; everything decided before and
//! after it is here, so it can be tested without an engine.
//!
//! The naming rule: in the server's `dlls/` directory, `tsmod_<name>` followed by the server
//! library's own platform suffix (`ts_plugin_api::library_suffix`): `tsmod_<name>.dll` next to
//! `mp.dll`, `tsmod_<name>_amd64.dll` next to `mp_amd64.dll`, `tsmod_<name>_i386.so` next to
//! `ts_i386.so` and `tsmod_<name>_amd64.so` next to `ts_amd64.so`. `<name>` is letters, digits,
//! `_` and `-`. A `tsmod_` file with another platform's suffix, or whose header says it was built
//! for another platform or architecture, is skipped with a warning. Files load in name order.

use std::ffi::c_int;
use std::path::{Path, PathBuf};
use ts_plugin_api::{LIBRARY_PREFIX, OK, Plugin, VERSION, library_suffix};

/// A platform and architecture the port builds the server library for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub windows: bool,
    pub x86_64: bool,
}

impl Target {
    /// The platform this server library was built for.
    pub const HOST: Target = Target { windows: cfg!(windows), x86_64: cfg!(target_arch = "x86_64") };
    const ALL: [Target; 4] = [
        Target { windows: true, x86_64: false },
        Target { windows: true, x86_64: true },
        Target { windows: false, x86_64: false },
        Target { windows: false, x86_64: true },
    ];

    pub fn suffix(self) -> &'static str {
        library_suffix(self.windows, self.x86_64)
    }

    /// For messages: "win32", "linux-amd64", ...
    pub fn describe(self) -> &'static str {
        match (self.windows, self.x86_64) {
            (true, false) => "win32",
            (true, true) => "win64",
            (false, false) => "linux-i386",
            (false, true) => "linux-amd64",
        }
    }
}

/// What a file in `dlls/` is to the loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not a plugin file at all (another prefix, or no library suffix): ignored silently.
    NotPlugin,
    /// A plugin `name` for this platform: load it.
    Load(String),
    /// A plugin built for another platform or architecture (by its suffix).
    OtherTarget(Target),
    /// `tsmod_` with this platform's suffix, but no usable name between them.
    BadName,
}

/// [`Verdict`] of the file name `file` for a server built for `host`. The longest suffix that
/// matches decides, so `tsmod_x_amd64.dll` is a win64 plugin, not a win32 one named `x_amd64`.
/// Windows compares names case-insensitively, as its file system does.
pub fn classify(file: &str, host: Target) -> Verdict {
    let fold = |s: &str| if host.windows { s.to_ascii_lowercase() } else { s.to_owned() };
    let f = fold(file);
    if !f.starts_with(LIBRARY_PREFIX) {
        return Verdict::NotPlugin;
    }
    let Some(t) = Target::ALL.iter().filter(|t| f.ends_with(t.suffix())).max_by_key(|t| t.suffix().len()).copied() else {
        return Verdict::NotPlugin;
    };
    if t != host {
        return Verdict::OtherTarget(t);
    }
    let name = &file[LIBRARY_PREFIX.len()..file.len() - t.suffix().len()];
    if valid_name(name) { Verdict::Load(name.to_owned()) } else { Verdict::BadName }
}

/// A plugin name: 1 to 32 ASCII letters, digits, `_` or `-`.
pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The platform and architecture an ELF or PE image was built for, from its first bytes (at
/// least the ELF header, or the DOS header and the PE header it points to): `None` if it is
/// neither, or for a machine the port doesn't build for.
pub fn image_target(bytes: &[u8]) -> Option<Target> {
    if bytes.starts_with(b"\x7fELF") {
        // e_ident[EI_DATA] must be little-endian; e_machine is the u16 at 0x12.
        if bytes.get(5) != Some(&1) {
            return None;
        }
        let machine = u16::from_le_bytes([*bytes.get(0x12)?, *bytes.get(0x13)?]);
        return match (bytes.get(4), machine) {
            (Some(1), 3) => Some(Target { windows: false, x86_64: false }), // ELFCLASS32, EM_386
            (Some(2), 62) => Some(Target { windows: false, x86_64: true }), // ELFCLASS64, EM_X86_64
            _ => None,
        };
    }
    if bytes.starts_with(b"MZ") {
        let at = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
        if bytes.get(at..at + 4)? != b"PE\0\0" {
            return None;
        }
        let machine = u16::from_le_bytes(bytes.get(at + 4..at + 6)?.try_into().ok()?);
        return match machine {
            0x14c => Some(Target { windows: true, x86_64: false }), // IMAGE_FILE_MACHINE_I386
            0x8664 => Some(Target { windows: true, x86_64: true }), // IMAGE_FILE_MACHINE_AMD64
            _ => None,
        };
    }
    None
}

/// How many bytes of a file [`image_target`] needs at most (the PE header may sit past the DOS
/// stub; MinGW and MSVC put it below 0x200).
pub const HEADER_BYTES: usize = 4096;

/// A file the loader looked at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub file: String,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Load it as plugin `name`.
    Load(String),
    /// Skip it, with this warning.
    Skip(String),
}

/// The plugin files in `dir` for `host`, in name order (files that aren't plugins left out). A
/// matching name is checked against the file's header too, so a library renamed for the wrong
/// platform is skipped before the system loader sees it.
pub fn scan(dir: &Path, host: Target) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|(file, path)| decide(&file, &path, host, read_header).map(|outcome| Found { path, file, outcome }))
        .collect()
}

fn read_header(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(HEADER_BYTES);
    std::fs::File::open(path).ok()?.take(HEADER_BYTES as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// One file's [`Outcome`]; `None` for a file that isn't a plugin. `header` reads the file's first
/// bytes.
pub fn decide(file: &str, path: &Path, host: Target, header: impl FnOnce(&Path) -> Option<Vec<u8>>) -> Option<Outcome> {
    match classify(file, host) {
        Verdict::NotPlugin => None,
        Verdict::BadName => {
            Some(Outcome::Skip(format!("{file}: not a plugin name (tsmod_<name>{}, <name> of letters, digits, _ and -)", host.suffix())))
        }
        Verdict::OtherTarget(t) => {
            Some(Outcome::Skip(format!("{file}: a {} library (this server is {}), skipped", t.describe(), host.describe())))
        }
        Verdict::Load(name) => Some(match header(path).as_deref().map(image_target) {
            None => Outcome::Skip(format!("{file}: can't be read, skipped")),
            Some(None) => Outcome::Skip(format!("{file}: not a library for this server ({}), skipped", host.describe())),
            Some(Some(t)) if t != host => {
                Outcome::Skip(format!("{file}: built for {} (this server is {}), skipped", t.describe(), host.describe()))
            }
            Some(Some(_)) => Outcome::Load(name),
        }),
    }
}

/// The checks after the plugin's `TSRS_PluginLoad` returned `ret` and filled `desc`: it must have
/// returned `OK`, kept the descriptor's size and built for this API version.
pub fn check_loaded(ret: c_int, desc: &Plugin) -> Result<(), String> {
    if ret != OK {
        return Err(format!("its TSRS_PluginLoad refused to load ({ret})"));
    }
    if desc.size as usize != size_of::<Plugin>() {
        return Err(format!("its plugin descriptor is {} bytes, this server's is {}", desc.size, size_of::<Plugin>()));
    }
    if desc.api_version != VERSION {
        return Err(format!("built for plugin API version {}, this server has version {VERSION}", desc.api_version));
    }
    Ok(())
}

/// The descriptor the server hands a plugin to fill: its size set, nothing else (a plugin that
/// doesn't say which API version it was built for isn't loaded).
pub fn blank_descriptor() -> Plugin {
    Plugin { api_version: 0, ..Plugin::EMPTY }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W32: Target = Target { windows: true, x86_64: false };
    const W64: Target = Target { windows: true, x86_64: true };
    const L32: Target = Target { windows: false, x86_64: false };
    const L64: Target = Target { windows: false, x86_64: true };

    #[test]
    fn discovery_follows_the_server_library_naming() {
        assert_eq!(classify("tsmod_rcbot.dll", W32), Verdict::Load("rcbot".into()));
        assert_eq!(classify("tsmod_rcbot_amd64.dll", W64), Verdict::Load("rcbot".into()));
        assert_eq!(classify("tsmod_rcbot_i386.so", L32), Verdict::Load("rcbot".into()));
        assert_eq!(classify("tsmod_rcbot_amd64.so", L64), Verdict::Load("rcbot".into()));
        assert_eq!(classify("tsmod_test_badver_i386.so", L32), Verdict::Load("test_badver".into()));
        // Windows file names are case-insensitive; the name keeps its case.
        assert_eq!(classify("TSMOD_RcBot.DLL", W32), Verdict::Load("RcBot".into()));
        assert_eq!(classify("TSMOD_x_i386.so", L32), Verdict::NotPlugin, "Linux is case-sensitive");
    }

    #[test]
    fn other_platforms_are_recognised_and_skipped() {
        // The longest suffix wins: a win64 plugin isn't a win32 one named `x_amd64`.
        assert_eq!(classify("tsmod_x_amd64.dll", W32), Verdict::OtherTarget(W64));
        assert_eq!(classify("tsmod_x.dll", W64), Verdict::OtherTarget(W32));
        assert_eq!(classify("tsmod_x_i386.so", W32), Verdict::OtherTarget(L32));
        assert_eq!(classify("tsmod_x_amd64.so", L32), Verdict::OtherTarget(L64));
        assert_eq!(classify("tsmod_x_i386.so", L64), Verdict::OtherTarget(L32));
        assert_eq!(classify("tsmod_x.dll", L64), Verdict::OtherTarget(W32));
    }

    #[test]
    fn other_files_are_not_plugins() {
        for f in ["mp.dll", "ts_i386.so", "tsmod_readme.txt", "tsmod_x.so", "libtsmod_x_i386.so", "tsmod_"] {
            assert_eq!(classify(f, L32), Verdict::NotPlugin, "{f}");
        }
        assert_eq!(classify("tsmod__i386.so", L32), Verdict::BadName, "empty name");
        assert_eq!(classify("tsmod_a b_i386.so", L32), Verdict::BadName);
        assert_eq!(classify("tsmod_a.b_i386.so", L32), Verdict::BadName);
    }

    fn elf(class: u8, machine: u16) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = class;
        b[5] = 1;
        b[0x12..0x14].copy_from_slice(&machine.to_le_bytes());
        b
    }

    fn pe(machine: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x100];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        b
    }

    #[test]
    fn headers_name_the_architecture() {
        assert_eq!(image_target(&elf(1, 3)), Some(L32));
        assert_eq!(image_target(&elf(2, 62)), Some(L64));
        assert_eq!(image_target(&elf(2, 183)), None, "aarch64");
        assert_eq!(image_target(&elf(1, 62)), None, "class and machine disagree");
        assert_eq!(image_target(&pe(0x14c)), Some(W32));
        assert_eq!(image_target(&pe(0x8664)), Some(W64));
        assert_eq!(image_target(&pe(0xaa64)), None, "arm64");
        assert_eq!(image_target(b"MZ"), None, "truncated");
        assert_eq!(image_target(b"hello world"), None);
        assert_eq!(image_target(&[]), None);
    }

    #[test]
    fn wrong_architecture_is_skipped() {
        let p = Path::new("dlls/tsmod_x_amd64.so");
        // Named for linux-amd64 but an i386 ELF inside.
        let o = decide("tsmod_x_amd64.so", p, L64, |_| Some(elf(1, 3))).unwrap();
        assert!(matches!(&o, Outcome::Skip(w) if w.contains("built for linux-i386")), "{o:?}");
        assert_eq!(decide("tsmod_x_amd64.so", p, L64, |_| Some(elf(2, 62))), Some(Outcome::Load("x".into())));
        // A PE image named as a Linux plugin, garbage, an unreadable file.
        assert!(matches!(decide("tsmod_x_amd64.so", p, L64, |_| Some(pe(0x8664))), Some(Outcome::Skip(_))));
        assert!(matches!(decide("tsmod_x_amd64.so", p, L64, |_| Some(b"not a library".to_vec())), Some(Outcome::Skip(_))));
        assert!(matches!(decide("tsmod_x_amd64.so", p, L64, |_| None), Some(Outcome::Skip(_))));
        // Another platform's name is skipped without reading it.
        let o = decide("tsmod_x.dll", p, L64, |_| panic!("not read")).unwrap();
        assert!(matches!(&o, Outcome::Skip(w) if w.contains("a win32 library")), "{o:?}");
        assert_eq!(decide("ts_amd64.so", p, L64, |_| panic!("not read")), None);
    }

    #[test]
    fn scan_sorts_and_filters_a_directory() {
        let dir = std::env::temp_dir().join(format!("tsmod-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tsmod_dir_amd64.so")).unwrap();
        let host_header = |t: Target| {
            if t.windows { pe(if t.x86_64 { 0x8664 } else { 0x14c }) } else { elf(1 + t.x86_64 as u8, if t.x86_64 { 62 } else { 3 }) }
        };
        let host = Target::HOST;
        let other = Target { windows: !host.windows, ..host };
        std::fs::write(dir.join(format!("tsmod_b{}", host.suffix())), host_header(host)).unwrap();
        std::fs::write(dir.join(format!("tsmod_a{}", host.suffix())), host_header(host)).unwrap();
        std::fs::write(dir.join(format!("tsmod_c{}", other.suffix())), host_header(other)).unwrap();
        std::fs::write(dir.join(format!("tsmod_junk{}", host.suffix())), b"junk").unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        let found = scan(&dir, host);
        let _ = std::fs::remove_dir_all(&dir);
        let summary: Vec<(String, bool)> = found.iter().map(|f| (f.file.clone(), matches!(f.outcome, Outcome::Load(_)))).collect();
        assert_eq!(
            summary,
            [
                (format!("tsmod_a{}", host.suffix()), true),
                (format!("tsmod_b{}", host.suffix()), true),
                (format!("tsmod_c{}", other.suffix()), false),
                (format!("tsmod_junk{}", host.suffix()), false),
            ]
        );
        assert!(scan(Path::new("/nonexistent/tsmod"), host).is_empty());
    }

    #[test]
    fn a_bad_descriptor_or_refusal_is_rejected() {
        let good = Plugin { ..Plugin::EMPTY };
        assert_eq!(check_loaded(OK, &good), Ok(()));
        assert!(check_loaded(1, &good).unwrap_err().contains("refused"));
        assert!(check_loaded(-2, &good).is_err());
        let small = Plugin { size: 8, ..Plugin::EMPTY };
        assert!(check_loaded(OK, &small).unwrap_err().contains("descriptor"));
        let old = Plugin { api_version: 2, ..Plugin::EMPTY };
        assert!(check_loaded(OK, &old).unwrap_err().contains("version 2"));
        // The server's blank descriptor isn't loadable as it is: the plugin must say its version.
        assert!(check_loaded(OK, &blank_descriptor()).is_err());
        assert_eq!(blank_descriptor().size as usize, size_of::<Plugin>());
    }
}
