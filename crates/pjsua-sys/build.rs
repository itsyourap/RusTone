//! Links the prebuilt pjproject static library and, when the `regen`
//! feature is enabled, regenerates the committed bindgen output.
//!
//! `PJPROJECT_DIR` points at:
//! * Windows: the pjproject source tree after the MSVC build (`lib/`,
//!   `third_party/lib/`, `<module>/include`), with the dependencies in
//!   `RUSTONE_DEPS_DIR` (`include/`, `lib/`, `bin/`);
//! * Linux and macOS: the `make install` prefix (`include/`, `lib/`,
//!   `lib/pkgconfig/libpjproject.pc`); the dependencies are system
//!   libraries, so `RUSTONE_DEPS_DIR` is optional.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Targets with committed bindings in `src/bindings/<target>.rs`.
const TARGETS: &[&str] = &["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "aarch64-apple-darwin"];

/// What the shim and bindgen compile against, and what to link.
struct Pj {
    includes: Vec<PathBuf>,
    defines: Vec<(String, Option<String>)>,
}

fn main() {
    let target = env::var("TARGET").unwrap();
    if !TARGETS.contains(&target.as_str()) {
        panic!("pjsua-sys has no bindings for {target} (supported: {})", TARGETS.join(", "));
    }
    let bindings = format!("src/bindings/{}.rs", target.replace('-', "_"));

    let pj_dir = dir_from_env("PJPROJECT_DIR");
    let pj = if target.contains("windows") { windows::setup(&pj_dir) } else { unix::setup(&pj_dir) };

    #[cfg(feature = "regen")]
    regen::run(&pj, &bindings);

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    if !manifest.join(&bindings).exists() {
        panic!("{bindings} is missing; run `cargo build -p pjsua-sys --features regen`");
    }

    build_shim(&pj);
}

fn build_shim(pj: &Pj) {
    println!("cargo:rerun-if-changed=shim");
    let mut build = cc::Build::new();
    build
        .file("shim/jf_shim.c")
        .file("shim/jf_debug.c")
        .file("shim/jf_video.c")
        .includes(&pj.includes)
        .define("NDEBUG", None)
        .static_crt(true)
        .warnings(true);
    for (name, value) in &pj.defines {
        build.define(name, value.as_deref());
    }
    build.compile("jf_shim");
}

fn dir_from_env(var: &str) -> PathBuf {
    println!("cargo:rerun-if-env-changed={var}");
    let dir = PathBuf::from(env::var(var).unwrap_or_else(|_| panic!("{var} is not set")));
    if !dir.is_dir() {
        panic!("{var}={} is not a directory", dir.display());
    }
    dir
}

mod windows {
    use super::*;

    const PJ_LIB: &str = "libpjproject-x86_64-x64-vc14-Release-Static";

    /// Libraries that pjproject needs and doesn't already request with
    /// `#pragma comment(lib, ...)`.
    const SYSTEM_LIBS: &[&str] = &[
        "ws2_32", "mswsock", "iphlpapi", "ole32", "oleaut32", "uuid", "strmiids", "winmm", "dsound", "advapi32", "user32",
        "gdi32", "crypt32", "bcrypt", "secur32", "shlwapi",
    ];

    /// DLLs the linked pjproject imports (dumpbin /dependents): SDL2 for
    /// video windows and OpenSSL for TLS/SRTP. OpenH264 and AMR are linked
    /// statically.
    const RUNTIME_DLLS: &[&str] = &["SDL2.dll", "libcrypto-3-x64.dll", "libssl-3-x64.dll"];

    pub fn setup(pj_dir: &Path) -> Pj {
        let deps_dir = dir_from_env("RUSTONE_DEPS_DIR");
        let pj_lib_dir = pj_dir.join("lib");
        println!("cargo:rerun-if-changed={}", pj_lib_dir.join(format!("{PJ_LIB}.lib")).display());

        // pjproject objects carry `#pragma comment(lib, ...)` directives for
        // OpenSSL, OpenH264, SDL2, etc., which link.exe resolves from these
        // paths.
        for dir in [pj_lib_dir, pj_dir.join("third_party").join("lib"), deps_dir.join("lib")] {
            println!("cargo:rustc-link-search=native={}", dir.display());
        }
        println!("cargo:rustc-link-lib=static={PJ_LIB}");
        for lib in SYSTEM_LIBS {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
        copy_runtime_dlls(&deps_dir);

        let mut includes: Vec<PathBuf> =
            ["pjlib", "pjlib-util", "pjnath", "pjmedia", "pjsip"].iter().map(|m| pj_dir.join(m).join("include")).collect();
        includes.push(deps_dir.join("include"));
        Pj { includes, defines: Vec::new() }
    }

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
}

mod unix {
    use super::*;

    /// Reads `libpjproject.pc` with `pkg-config --static`. The paths in it
    /// are where pjproject was installed when it was built, so the search
    /// and include paths always come from `PJPROJECT_DIR` instead, which
    /// lets the installed tree move.
    pub fn setup(pj_dir: &Path) -> Pj {
        let lib_dir = pj_dir.join("lib");
        let pc_dir = lib_dir.join("pkgconfig");
        println!("cargo:rerun-if-changed={}", pc_dir.join("libpjproject.pc").display());
        println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

        let mut search = std::env::var_os("PKG_CONFIG_PATH").map(|p| env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
        search.insert(0, pc_dir.clone());
        let output = Command::new("pkg-config")
            .args(["--static", "--cflags", "--libs", "libpjproject"])
            .env("PKG_CONFIG_PATH", env::join_paths(search).unwrap())
            .output()
            .unwrap_or_else(|e| panic!("cannot run pkg-config: {e}"));
        if !output.status.success() {
            panic!("pkg-config libpjproject failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        let flags = String::from_utf8(output.stdout).unwrap();

        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        let mut pj = Pj { includes: vec![pj_dir.join("include")], defines: Vec::new() };
        let mut tokens = flags.split_whitespace();
        while let Some(token) = tokens.next() {
            if let Some(dir) = token.strip_prefix("-L") {
                // Dependencies' own directories (e.g. Homebrew's); pjproject's
                // install path is replaced by `lib_dir` above.
                if Path::new(dir).is_dir() && Path::new(dir) != lib_dir {
                    println!("cargo:rustc-link-search=native={dir}");
                }
            } else if let Some(name) = token.strip_prefix("-l") {
                // pjproject asks for GNU's C++ library (for OpenH264);
                // Apple's is libc++.
                let name = if name == "stdc++" && env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") { "c++" } else { name };
                // pjproject's libraries (and its bundled third-party ones) are
                // static archives in `lib_dir`; everything else is a system
                // shared library.
                let kind = if lib_dir.join(format!("lib{name}.a")).is_file() { "static" } else { "dylib" };
                println!("cargo:rustc-link-lib={kind}={name}");
            } else if token == "-framework" {
                let name = tokens.next().expect("-framework without a name");
                println!("cargo:rustc-link-lib=framework={name}");
            } else if let Some(def) = token.strip_prefix("-D") {
                let (name, value) = match def.split_once('=') {
                    Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
                    None => (def.to_owned(), None),
                };
                pj.defines.push((name, value));
            } else if let Some(dir) = token.strip_prefix("-I") {
                if Path::new(dir).is_dir() && !Path::new(dir).starts_with(pj_dir) {
                    pj.includes.push(PathBuf::from(dir));
                }
            } else if token == "-pthread" {
                println!("cargo:rustc-link-arg=-pthread");
            }
        }
        pj
    }
}

#[cfg(feature = "regen")]
mod regen {
    use super::Pj;
    use std::path::Path;

    pub fn run(pj: &Pj, bindings: &str) {
        println!("cargo:rerun-if-changed=wrapper.h");
        let mut args: Vec<String> = pj.includes.iter().map(|dir| format!("-I{}", dir.display())).collect();
        args.push("-DNDEBUG".into());
        for (name, value) in &pj.defines {
            args.push(match value {
                Some(v) => format!("-D{name}={v}"),
                None => format!("-D{name}"),
            });
        }

        let generated = bindgen::Builder::default()
            .header("wrapper.h")
            .clang_args(args)
            .allowlist_function("pjsua_.*")
            .allowlist_function("pj_str|pj_strerror|pj_thread_register|pj_thread_is_registered|pj_log_.*")
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

        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join(bindings);
        std::fs::write(&out, signed_enums(&generated.to_string())).expect("writing bindings failed");
    }

    /// MSVC gives every C enum the type `int`; clang elsewhere makes enums
    /// without negative values `unsigned int`. Size and calling convention
    /// are the same, so declare every enum as `c_int` and the constants
    /// (`PJ_SUCCESS`, status codes, …) have one type on all targets.
    fn signed_enums(code: &str) -> String {
        // With `EnumVariation::Consts` an enum is a type alias plus
        // `pub const VARIANT: Enum = value;` items; macros get plain
        // integer types, so aliases used by constants are the enums.
        let enums: std::collections::HashSet<&str> = code
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub const ")?.split_once(": ")?.1.split_once(" = ").map(|(t, _)| t))
            .collect();
        code.lines()
            .map(|line| {
                let alias = line.trim().strip_prefix("pub type ").and_then(|l| l.strip_suffix(" = ::std::os::raw::c_uint;"));
                match alias {
                    Some(name) if enums.contains(name) => line.replace("::std::os::raw::c_uint;", "::std::os::raw::c_int;"),
                    _ => line.to_owned(),
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
}
