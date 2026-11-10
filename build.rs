//! Build helper for the windows-gnu Rust toolchain.
//!
//! egui's link-opening part (webbrowser) asks the linker for "shlwapi", but the
//! MinGW files that come with Rust don't include that import library. The
//! `windows_x86_64_gnu` crate (already a dependency) ships one import library that
//! covers every Windows function, so a copy of it named libshlwapi.a is made here
//! and handed to the linker. Nothing happens with the MSVC toolchain.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target = out.join("libshlwapi.a");
    if !target.is_file() {
        if let Some(src) = find_windows_import_lib() {
            let _ = std::fs::copy(src, &target);
        }
    }
    if target.is_file() {
        println!("cargo:rustc-link-search=native={}", out.display());
    }
}

fn find_windows_import_lib() -> Option<PathBuf> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".cargo")))?;
    let src = home.join("registry").join("src");
    let mut best: Option<PathBuf> = None;
    for index in std::fs::read_dir(src).ok()?.flatten() {
        for krate in std::fs::read_dir(index.path()).ok()?.flatten() {
            let name = krate.file_name().to_string_lossy().to_string();
            if !name.starts_with("windows_x86_64_gnu-0.5") {
                continue;
            }
            let lib = krate.path().join("lib");
            for f in std::fs::read_dir(&lib).ok()?.flatten() {
                let n = f.file_name().to_string_lossy().to_string();
                if n.starts_with("libwindows.") && n.ends_with(".a") {
                    if best.as_ref().map(|b| f.path() > *b).unwrap_or(true) {
                        best = Some(f.path());
                    }
                }
            }
        }
    }
    best
}
