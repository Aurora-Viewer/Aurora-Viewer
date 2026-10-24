//! Build script: tells the viewer which cargo profile built it, for the
//! window title ("Aurora Viewer <version> (Dev)"): cargo only exposes `debug` /
//! `release` (PROFILE), while the profile's target directory names the
//! custom ones (`debugging`, `ci`). On Windows it also embeds the resources
//! of aurora-viewer.exe: its icon and its version block.

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

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
        // The hybrid-GPU hints of main.rs (gpu_hints) in the exe's export
        // table, where the NVIDIA and AMD drivers look for them: a Rust
        // executable exports nothing on its own
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            for symbol in ["NvOptimusEnablement", "AmdPowerXpressRequestHighPerformance"] {
                println!("cargo:rustc-link-arg-bin=aurora-viewer=/EXPORT:{symbol},DATA");
            }
        }
    }
}

/// The window sets its icon at run time (app.rs), but the file itself needs
/// an icon resource: without one, Explorer, shortcuts and the taskbar pin
/// show the generic executable icon. The version block comes with it:
/// Properties > Details, and the name Windows gives the program (Task
/// Manager, volume mixer, firewall prompts) is its FileDescription.
///
/// The version is the one aurora-version resolved from the Git tag (its
/// `links` metadata), never CARGO_PKG_VERSION. A failure stops the build: a
/// release must not ship without its icon again.
fn windows_resources() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("Cargo supplies CARGO_MANIFEST_DIR");
    let icon = Path::new(&manifest).join("../../assets/branding/aurora.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    let display = std::env::var("DEP_AURORA_VERSION_DISPLAY").expect("aurora-version's build script supplies the version");
    let numeric = std::env::var("DEP_AURORA_VERSION_NUMERIC").expect("aurora-version's build script supplies the version");
    // "major.minor.patch.distance" packed as four 16-bit fields
    let packed = numeric
        .split('.')
        .chain(std::iter::repeat("0"))
        .take(4)
        .fold(0u64, |acc, part| (acc << 16) | part.parse::<u64>().unwrap_or(0).min(0xFFFF));

    let mut res = winresource::WindowsResource::new();
    res.set_icon(&icon.to_string_lossy())
        .set("ProductName", "Aurora Viewer")
        .set("FileDescription", "Aurora Viewer")
        .set("OriginalFilename", "aurora-viewer.exe")
        .set("ProductVersion", &display)
        .set("FileVersion", &display)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed)
        .set_version_info(winresource::VersionInfo::FILEVERSION, packed);
    // the viewer only: the test binaries of this crate need no resources
    if let Err(error) = res.compile_for("aurora-viewer") {
        panic!("Cannot embed the icon and version of aurora-viewer.exe (rc.exe of the Windows SDK): {error}");
    }
}
