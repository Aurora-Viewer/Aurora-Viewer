//! Vulkan drivers kept out of the viewer's process.
//!
//! Dozen (`vulkan_dzn.dll`, Mesa's Vulkan on Direct3D 12) comes with
//! Microsoft's "OpenCL, OpenGL, and Vulkan Compatibility Pack", installed by
//! some PC makers and programs. The Vulkan loader loads it beside the GPU's
//! own driver, and while it lists its devices Dozen opens every Direct3D 12
//! adapter: an old AMD Direct3D 12 driver (`amdxc64`, 2023) crashed there,
//! inside `vkCreateInstance`, before Aurora had chosen any GPU (crash report
//! of 2026-10-26: `vulkan-1` → `vulkan_dzn` → `d3d12core` → `amdxc64`).
//! Dozen only translates Vulkan to the Direct3D 12 driver of a GPU that
//! already has its own Vulkan driver, with fewer features, so the viewer
//! never wants it there.
//!
//! The loader skips the drivers whose manifest file name matches a glob of
//! `VK_LOADER_DRIVERS_DISABLE` (loader 1.3.234 and later; an older loader
//! ignores it and nothing changes), read again at each instance it creates
//! and applied to every manifest it finds, the compatibility pack's
//! (`dzn_icd.<arch>.json`, package `Microsoft.D3DMappingLayers`) included
//! (`loader_icd_scan`, loader/loader.c). [`disable`] sets it at the start of
//! `main`, before any thread, unless the user chose the drivers themself.
//! On a PC where Dozen is the only Vulkan driver (no native one for the GPU),
//! the renderer finds no adapter without it and lets it back in with
//! [`allow_again`] before trying once more. `AURORA_VULKAN_DRIVERS=keep`
//! leaves the environment as it is.

use std::sync::atomic::{AtomicBool, Ordering};

/// The loader's filter of the drivers to skip.
pub const DISABLE_VARIABLE: &str = "VK_LOADER_DRIVERS_DISABLE";

/// Dozen's manifest: `dzn_icd.x86_64.json` in Mesa's build and in the
/// compatibility pack.
pub const DOZEN_GLOB: &str = "*dzn*";

/// The variables through which the user picks the drivers themself: when one
/// is set, the viewer adds nothing (Dozen may be what they asked for).
const USER_CHOICE: &[&str] = &[
    DISABLE_VARIABLE,
    "VK_LOADER_DRIVERS_SELECT",
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
];

/// Set when [`disable`] put the filter in the environment.
static DISABLED: AtomicBool = AtomicBool::new(false);

/// Whether to keep Dozen out: not with `AURORA_VULKAN_DRIVERS=keep` (`keep`),
/// nor when the user set one of the loader's driver variables (`is_set`).
pub fn should_disable(keep: Option<&str>, is_set: impl Fn(&str) -> bool) -> bool {
    !keep.is_some_and(|v| v.trim().eq_ignore_ascii_case("keep")) && !USER_CHOICE.iter().any(|v| is_set(v))
}

/// Keeps Dozen out of the process and returns whether it did, for the log
/// (written once logging is up).
///
/// # Safety
///
/// Must be called before any other thread of the process starts (the
/// first lines of `main`): it changes the environment.
pub unsafe fn disable() -> bool {
    let keep = std::env::var("AURORA_VULKAN_DRIVERS").ok();
    if !should_disable(keep.as_deref(), |v| std::env::var_os(v).is_some()) {
        return false;
    }
    // SAFETY: the caller guarantees that no other thread runs yet, so
    // nothing reads the environment while it changes.
    unsafe { std::env::set_var(DISABLE_VARIABLE, DOZEN_GLOB) };
    DISABLED.store(true, Ordering::Relaxed);
    true
}

/// Lets Dozen back in when [`disable`] kept it out, for a renderer that found
/// no adapter without it; returns whether it did (the next instance then
/// sees it). Once only: later calls return false.
pub fn allow_again() -> bool {
    if !DISABLED.swap(false, Ordering::Relaxed) {
        return false;
    }
    #[cfg(windows)]
    {
        // SAFETY: changing the environment is always sound on Windows, with
        // other threads running too (std::env::remove_var's documentation):
        // the process environment is behind the system's own lock.
        unsafe { std::env::remove_var(DISABLE_VARIABLE) };
        true
    }
    #[cfg(not(windows))]
    {
        // Other threads may read the environment through libc here: leave
        // it (Dozen is a Windows driver anyway).
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dozen_is_kept_out_by_default() {
        assert!(should_disable(None, |_| false));
        assert!(should_disable(Some("other"), |_| false));
    }

    #[test]
    fn keep_leaves_the_drivers_alone() {
        assert!(!should_disable(Some("keep"), |_| false));
        assert!(!should_disable(Some(" KEEP "), |_| false));
    }

    #[test]
    fn a_driver_choice_of_the_user_is_respected() {
        for v in USER_CHOICE {
            assert!(!should_disable(None, |name| name == *v), "{v}");
        }
        assert!(should_disable(None, |name| name == "VK_LOADER_DEBUG"));
    }

    #[test]
    fn the_glob_matches_dozens_manifest_only() {
        // the loader's glob: `*` around a part of the file name
        let part = DOZEN_GLOB.trim_matches('*');
        for name in ["dzn_icd.x86_64.json", "dzn_icd.aarch64.json", "dzn_icd.json"] {
            assert!(name.contains(part), "{name}");
        }
        for name in ["nv-vk64.json", "amd-vulkan64.json", "igvk64.json", "amdvlk64.json"] {
            assert!(!name.contains(part), "{name}");
        }
    }
}
