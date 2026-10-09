//! Build script: tells the viewer which cargo profile built it, for the
//! window title ("Aurora Viewer (Dev)"): cargo only exposes `debug` /
//! `release` (PROFILE), while the profile's target directory names the
//! custom ones (`debugging`, `ci`).

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // OUT_DIR = <target>/<profile dir>/build/<crate>-<hash>/out
    let out = std::env::var("OUT_DIR").unwrap_or_default();
    let parts: Vec<String> = Path::new(&out)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let dir = parts
        .iter()
        .rposition(|p| p == "build")
        .and_then(|i| i.checked_sub(1))
        .map(|i| parts[i].clone())
        .unwrap_or_default();
    println!("cargo:rustc-env=AURORA_BUILD_PROFILE={dir}");
}
