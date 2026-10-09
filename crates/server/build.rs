fn main() {
    // i686 Windows (MinGW): GiveFnptrsToDll is stdcall, decorated `_GiveFnptrsToDll@8`, but the
    // engine looks up the plain name. Let ld map the plain export to the decorated symbol without
    // printing a note each build.
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") == "windows" && var("CARGO_CFG_TARGET_ENV") == "gnu" && var("CARGO_CFG_TARGET_ARCH") == "x86" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,--enable-stdcall-fixup");
    }
}
