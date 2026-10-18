//! Links the prebuilt pjproject static library and, when the `regen`
//! feature is enabled, regenerates the committed bindgen output.

use std::env;
use std::path::{Path, PathBuf};

const TARGET: &str = "x86_64-pc-windows-msvc";
const BINDINGS: &str = "src/bindings/x86_64_pc_windows_msvc.rs";
const PJ_LIB: &str = "libpjproject-x86_64-x64-vc14-Release-Static";

/// Libraries that pjproject needs and doesn't already request with
/// `#pragma comment(lib, ...)`.
const SYSTEM_LIBS: &[&str] = &[
    "ws2_32", "mswsock", "iphlpapi", "ole32", "oleaut32", "uuid", "strmiids", "winmm",
    "dsound", "advapi32", "user32", "gdi32", "crypt32", "bcrypt", "secur32", "shlwapi",
];

fn main() {
    let target = env::var("TARGET").unwrap();
    if target != TARGET {
        panic!("pjsua-sys currently supports only {TARGET} (Linux/macOS come in Phase 10)");
    }

    let pj_dir = dir_from_env("PJPROJECT_DIR");
    let deps_dir = dir_from_env("RUSTONE_DEPS_DIR");

    #[cfg(feature = "regen")]
    regen::run(&pj_dir, &deps_dir);

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    if !manifest.join(BINDINGS).exists() {
        panic!("{BINDINGS} is missing; run `cargo build -p pjsua-sys --features regen`");
    }

    build_shim(&pj_dir, &deps_dir);
    link(&pj_dir, &deps_dir);
    copy_runtime_dlls(&deps_dir);
}

/// DLLs the linked pjproject imports (dumpbin /dependents): SDL2 for video
/// windows and OpenSSL for TLS/SRTP. OpenH264 and AMR are linked statically.
const RUNTIME_DLLS: &[&str] = &["SDL2.dll", "libcrypto-3-x64.dll", "libssl-3-x64.dll"];

/// Copies the runtime DLLs next to the binaries Cargo builds
/// (target/<profile>/ and target/<profile>/deps/ for tests), so the exe
/// starts without PATH changes.
fn copy_runtime_dlls(deps_dir: &Path) {
    // OUT_DIR is target/<profile>/build/pjsua-sys-<hash>/out.
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let Some(profile_dir) = out.ancestors().nth(3) else { return };
    for name in RUNTIME_DLLS {
        let Some(src) = ["bin", "lib"].iter().map(|d| deps_dir.join(d).join(name)).find(|p| p.is_file()) else {
            println!("cargo:warning={name} not found in {}\\{{bin,lib}}", deps_dir.display());
            continue;
        };
        println!("cargo:rerun-if-changed={}", src.display());
        for dir in [profile_dir.to_path_buf(), profile_dir.join("deps")] {
            let dst = dir.join(name);
            let same = std::fs::metadata(&dst)
                .ok()
                .zip(std::fs::metadata(&src).ok())
                .is_some_and(|(d, s)| d.len() == s.len() && d.modified().ok() >= s.modified().ok());
            if !same && let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::copy(&src, &dst).map(drop)) {
                println!("cargo:warning=cannot copy {name} to {}: {e}", dir.display());
            }
        }
    }
}

fn include_dirs(pj_dir: &Path, deps_dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["pjlib", "pjlib-util", "pjnath", "pjmedia", "pjsip"]
        .iter()
        .map(|m| pj_dir.join(m).join("include"))
        .collect();
    dirs.push(deps_dir.join("include"));
    dirs
}

fn build_shim(pj_dir: &Path, deps_dir: &Path) {
    println!("cargo:rerun-if-changed=shim");
    cc::Build::new()
        .file("shim/jf_shim.c")
        .includes(include_dirs(pj_dir, deps_dir))
        .define("NDEBUG", None)
        .static_crt(true)
        .warnings(true)
        .compile("jf_shim");
}

fn dir_from_env(var: &str) -> PathBuf {
    println!("cargo:rerun-if-env-changed={var}");
    let dir = PathBuf::from(env::var(var).unwrap_or_else(|_| panic!("{var} is not set")));
    if !dir.is_dir() {
        panic!("{var}={} is not a directory", dir.display());
    }
    dir
}

fn link(pj_dir: &Path, deps_dir: &Path) {
    let pj_lib_dir = pj_dir.join("lib");
    println!("cargo:rerun-if-changed={}", pj_lib_dir.join(format!("{PJ_LIB}.lib")).display());

    // pjproject objects carry `#pragma comment(lib, ...)` directives for
    // OpenSSL, OpenH264, SDL2, etc., which link.exe resolves from these paths.
    for dir in [pj_lib_dir, pj_dir.join("third_party").join("lib"), deps_dir.join("lib")] {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    println!("cargo:rustc-link-lib=static={PJ_LIB}");
    for lib in SYSTEM_LIBS {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}

#[cfg(feature = "regen")]
mod regen {
    use std::path::Path;

    pub fn run(pj_dir: &Path, deps_dir: &Path) {
        println!("cargo:rerun-if-changed=wrapper.h");
        let mut args: Vec<String> = super::include_dirs(pj_dir, deps_dir)
            .iter()
            .map(|dir| format!("-I{}", dir.display()))
            .collect();
        args.push("-DNDEBUG".into());

        let bindings = bindgen::Builder::default()
            .header("wrapper.h")
            .clang_args(args)
            .allowlist_function("pjsua_.*")
            .allowlist_function(
                "pj_str|pj_strerror|pj_thread_register|pj_thread_is_registered|pj_log_.*",
            )
            .allowlist_function("pjmedia_aud_dev_refresh|pjmedia_vid_dev_refresh|pjmedia_tonegen_.*|pjmedia_port_destroy|pj_pool_release|pjmedia_wav_player_set_eof_cb2")
            .allowlist_type("pjsua_.*|pj_constants_|pjsip_status_code|pjsip_inv_state")
            .allowlist_type("pjsip_cfg_t|pjsip_cred_data_type|pj_log_decoration|pjmedia_tone_digit|pjmedia_tone_desc|pjmedia_file_player_option|pjmedia_dir")
            .allowlist_var("PJMEDIA_TONEGEN_.*|PJMEDIA_RTP_EREMNORFC2833|PJSIP_EBUSY")
            .allowlist_var("PJSUA_.*|PJ_INVALID_SOCKET|PJ_LOG_.*")
            .default_enum_style(bindgen::EnumVariation::Consts)
            .prepend_enum_name(false)
            .derive_default(true)
            .generate_comments(false)
            .layout_tests(true)
            .generate()
            .expect("bindgen failed");

        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join(super::BINDINGS);
        bindings.write_to_file(&out).expect("writing bindings failed");
    }
}
