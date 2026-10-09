use std::{env, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // FWGS_DIR, or the nearest `xash3d-fwgs` directory next to any ancestor of this crate (works
    // from the main checkout and from git worktrees).
    let fwgs = env::var("FWGS_DIR").map(PathBuf::from).unwrap_or_else(|_| {
        manifest
            .ancestors()
            .map(|a| a.join("xash3d-fwgs"))
            .find(|p| p.join("common/xash3d_types.h").exists())
            .expect("FWGS headers not found: set FWGS_DIR to an xash3d-fwgs checkout")
    });
    let target = env::var("TARGET").unwrap();

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_args(["common", "engine", "pm_shared", "public", "3rdparty/library_suffix/include"].map(|d| format!("-I{}", fwgs.join(d).display())))
        .clang_arg(format!("--target={target}")) // layouts differ between i686 and amd64: generate per target
        .allowlist_type("(enginefuncs_t|DLL_FUNCTIONS|NEW_DLL_FUNCTIONS|globalvars_t|edict_t|entvars_t|KeyValueData|TraceResult|cvar_t)")
        .allowlist_type("(cl_entity_t|cl_enginefunc_t|playermove_t|usercmd_t|entity_state_t|clientdata_t|weapon_data_t|ref_params_t|local_state_t)")
        .allowlist_var("(INTERFACE_VERSION|NEW_DLL_FUNCTIONS_VERSION|CLDLL_INTERFACE_VERSION)")
        .allowlist_var("(FL_|EF_|MOVETYPE_|SOLID_|DAMAGE_|DEAD_|IN_|MSG_|SVC_|CHAN_|ATTN_|PITCH_|kRender|ENTITY_|PM_|CONTENTS_).*")
        .allowlist_type("ALERT_TYPE")
        .rustified_enum("(ALERT_TYPE|PRINT_TYPE)")
        .derive_default(true)
        .generate()
        .expect("bindgen");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(out).unwrap();
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-env-changed=FWGS_DIR");
}
