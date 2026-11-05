//! Third-party Vulkan layers kept out of the viewer's process.
//!
//! Overlay and capture programs register implicit Vulkan layers that the
//! Vulkan loader injects into every Vulkan application, inside
//! `vkCreateInstance`, whether the program runs or not. They hook the
//! presentation (`vkQueuePresentKHR`, the swapchain) and are the first
//! cause of the crashes and lost images that testers meet with overlays:
//! issue #901, a crash one second after start in `wgpu::Instance::new`,
//! with Overwolf's overlay layer (`OWClient`) calling into a capture hook
//! (`graphics-hook64`) before Aurora had drawn anything; RivaTuner's layer
//! crashing wgpu's device teardown elsewhere. Each layer is disabled
//! through the variable its manifest declares (`disable_environment`),
//! which every loader version honours, unlike `VK_LOADER_LAYERS_DISABLE`
//! (loader 1.3.234 and later). Two groups:
//!
//! - [`CRASHING`]: layers known to crash the viewer, always kept out;
//! - [`OVERLAYS`]: the other overlays' layers, kept out unless Preferences
//!   › Graphics › Overlays allows them (an FPS counter, the Steam overlay,
//!   OBS Studio's game capture).
//!
//! OBS Studio's game capture layer (`VK_LAYER_OBS_HOOK`, Wallpaper Engine's
//! copy declares the same variable) joined the overlays after #2443: its
//! manifest loads `graphics-hook64.dll` into every Vulkan program as soon as
//! OBS is installed, running or not, and it was loaded in 61 % of the GPU
//! device-lost reports of 0.8.5 to 0.9.1 (33 of 54) against 10 % of the
//! other crash reports (3 of 30), most of them hangs the driver reset with
//! the GPU frames well under its time limit (#2443). OBS's Window Capture
//! needs no layer and keeps working; a streamer who wants Game Capture
//! allows the overlays.
//!
//! Kept in any case: ReShade's layer (installed on purpose for a program),
//! and the GPU drivers' own layers (`VK_LAYER_NV_optimus`,
//! `VK_LAYER_NV_present`, AMD's switchable graphics), which choose the GPU
//! and present.
//!
//! The loader reads these variables from the process environment each time
//! it looks for layers, so setting them at the start of `main`, before any
//! thread, is enough. They are inherited by the child processes (media
//! plugins, crash watcher), which use no Vulkan overlay either.
//! `AURORA_VULKAN_LAYERS=keep` leaves the environment as it is; a variable
//! the user already set is left untouched. Overlays that inject a DLL into
//! the running process instead (Discord) cannot be kept out this way: the
//! viewer names them in its log (aurora-viewer's `overlay_hooks.rs`).

/// Layers known to crash the viewer: always disabled.
pub const CRASHING: &[Layer] = &[
    // Overwolf's in-game overlay (owclient.dll, ow-vulkan-overlay64.json), #901
    Layer {
        program: "Overwolf",
        name: "VK_LAYER_OW_OVERLAY",
        variable: "DISABLE_VULKAN_OW_OVERLAY_LAYER",
    },
    // Overwolf's capture hook (ow-graphics-vulkan.dll, ow-graphics-vulkan64.json), #901
    Layer {
        program: "Overwolf",
        name: "VK_LAYER_OW_OBS_HOOK",
        variable: "DISABLE_VULKAN_OW_OBS_CAPTURE",
    },
    // Bandicam's 64-bit manifest deliberately uses a different variable
    // from the 32-bit one. #2080 faults in bdcamvk64 during vkCreateInstance.
    Layer {
        program: "Bandicam",
        name: "VK_LAYER_bandicam_helper",
        variable: "VK_LAYER_bandicam_helper_DEBUG_1",
    },
];

/// The other overlays' layers: disabled unless the user allows overlays.
/// Names and variables from each program's manifest.
pub const OVERLAYS: &[Layer] = &[
    // obs-vulkan64.json (OBS Studio's game capture, graphics-hook64.dll),
    // whose variable Wallpaper Engine's copy (VK_LAYER_WEOBS_HOOK) shares;
    // in most GPU device-lost reports (#2443, module comment)
    Layer {
        program: "OBS Studio game capture",
        name: "VK_LAYER_OBS_HOOK",
        variable: "DISABLE_VULKAN_OBS_CAPTURE",
    },
    // SteamOverlayVulkanLayer64.json; only active for programs Steam starts
    Layer {
        program: "Steam overlay",
        name: "VK_LAYER_VALVE_steam_overlay",
        variable: "DISABLE_VK_LAYER_VALVE_steam_overlay_1",
    },
    // RTSSVkLayer64.json (RivaTuner Statistics Server, MSI Afterburner's OSD)
    Layer {
        program: "RivaTuner Statistics Server",
        name: "VK_LAYER_RTSS",
        variable: "DISABLE_RTSS_LAYER",
    },
    // EOSOverlayVkLayer-Win64.json (Epic Online Services overlay)
    Layer {
        program: "Epic Games overlay",
        name: "VK_LAYER_EOS_Overlay",
        variable: "EOS_OVERLAY_DISABLE_VULKAN_WIN64",
    },
    Layer {
        program: "Twitch Studio",
        name: "VK_LAYER_Twitch_Overlay",
        variable: "DISABLE_TWITCH_VULKAN_OVERLAY",
    },
    Layer {
        program: "FPS Monitor",
        name: "VK_LAYER_fpsmon",
        variable: "DISABLE_FPSMON_LAYER",
    },
    Layer {
        program: "PlayClaw",
        name: "VK_LAYER_playclaw",
        variable: "DISABLE_PLAYCLAW_LAYER",
    },
];

/// One implicit layer, its program and the environment variable that
/// disables it (the loader disables the layer when the variable is set,
/// whatever its value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    pub program: &'static str,
    pub name: &'static str,
    pub variable: &'static str,
}

/// The layers to disable: none with `AURORA_VULKAN_LAYERS=keep` (`keep`),
/// the crashing ones only when overlays are allowed, and none whose
/// variable is already set (`is_set`).
pub fn to_disable(keep: Option<&str>, allow_overlays: bool, is_set: impl Fn(&str) -> bool) -> Vec<Layer> {
    if keep.is_some_and(|v| v.trim().eq_ignore_ascii_case("keep")) {
        return Vec::new();
    }
    let overlays = if allow_overlays { &[][..] } else { OVERLAYS };
    CRASHING.iter().chain(overlays).copied().filter(|l| !is_set(l.variable)).collect()
}

/// The names of the Vulkan layers the loader offers an instance now: the
/// implicit ones not disabled (the drivers' own, the overlays when allowed) and the explicit ones installed, for the crash report. Only
/// names that look like a layer's (`VK_LAYER_` and word characters) are
/// kept, at most 16, sorted. Empty without a Vulkan loader or on macOS.
#[cfg(not(target_os = "macos"))]
pub fn loaded_names() -> Vec<String> {
    // SAFETY: loading the system's Vulkan library, as the XR start does.
    let Ok(entry) = (unsafe { ash::Entry::load() }) else {
        return Vec::new();
    };
    // SAFETY: a plain query of the loader, with a valid entry.
    let Ok(layers) = (unsafe { entry.enumerate_instance_layer_properties() }) else {
        return Vec::new();
    };
    let names = layers.iter().filter_map(|l| l.layer_name_as_c_str().ok()?.to_str().ok());
    clean_names(names)
}

#[cfg(target_os = "macos")]
pub fn loaded_names() -> Vec<String> {
    Vec::new()
}

/// Layer names fit for a public report: `VK_LAYER_` and word characters,
/// 48 at most, sorted and unique, 16 at most.
pub fn clean_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = names
        .filter(|n| n.len() <= 48 && n.starts_with("VK_LAYER_") && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    out.truncate(16);
    out
}

/// Sets the disabling variables in the process environment and returns the
/// layers disabled, for the log (written once logging is up).
///
/// # Safety
///
/// Must be called before any other thread of the process starts (the
/// first lines of `main`): it changes the environment.
pub unsafe fn disable(allow_overlays: bool) -> Vec<Layer> {
    let keep = std::env::var("AURORA_VULKAN_LAYERS").ok();
    let layers = to_disable(keep.as_deref(), allow_overlays, |v| std::env::var_os(v).is_some());
    for l in &layers {
        // SAFETY: the caller guarantees that no other thread runs yet, so
        // nothing reads the environment while it changes.
        unsafe { std::env::set_var(l.variable, "1") };
    }
    layers
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_layer_names_reach_the_report() {
        let names = [
            "VK_LAYER_OBS_HOOK",
            "VK_LAYER_NV_optimus",
            "VK_LAYER_OBS_HOOK",
            "/home/someone",
            "VK_LAYER_a b",
            "other",
        ];
        assert_eq!(super::clean_names(names.into_iter()), ["VK_LAYER_NV_optimus", "VK_LAYER_OBS_HOOK"]);
    }

    use super::*;

    fn names(layers: &[Layer]) -> Vec<&'static str> {
        layers.iter().map(|l| l.name).collect()
    }

    #[test]
    fn crashing_and_overlay_layers_are_disabled_by_default() {
        let names = names(&to_disable(None, false, |_| false));
        assert_eq!(&names[..2], ["VK_LAYER_OW_OVERLAY", "VK_LAYER_OW_OBS_HOOK"]);
        for overlay in [
            "VK_LAYER_OBS_HOOK",
            "VK_LAYER_VALVE_steam_overlay",
            "VK_LAYER_RTSS",
            "VK_LAYER_EOS_Overlay",
        ] {
            assert!(names.contains(&overlay), "{overlay}");
        }
        assert_eq!(names.len(), CRASHING.len() + OVERLAYS.len());
    }

    #[test]
    fn obs_capture_is_an_overlay_with_its_manifest_variable() {
        let obs = OVERLAYS.iter().find(|l| l.name == "VK_LAYER_OBS_HOOK").expect("OBS layer");
        // obs-vulkan64.json's disable_environment; Wallpaper Engine's copy
        // declares the same one, so one variable keeps both out
        assert_eq!(obs.variable, "DISABLE_VULKAN_OBS_CAPTURE");
        assert!(CRASHING.iter().all(|l| l.variable != obs.variable));
        assert!(to_disable(None, true, |_| false).iter().all(|l| l.name != obs.name));
    }

    #[test]
    fn reshade_and_driver_layers_are_never_disabled() {
        for kept in [
            "VK_LAYER_reshade",
            "VK_LAYER_NV_optimus",
            "VK_LAYER_NV_present",
            "VK_LAYER_AMD_switchable_graphics",
        ] {
            assert!(CRASHING.iter().chain(OVERLAYS).all(|l| l.name != kept), "{kept}");
        }
    }

    #[test]
    fn allowed_overlays_still_keep_the_crashing_layers_out() {
        assert_eq!(
            names(&to_disable(None, true, |_| false)),
            ["VK_LAYER_OW_OVERLAY", "VK_LAYER_OW_OBS_HOOK", "VK_LAYER_bandicam_helper"]
        );
    }

    #[test]
    fn keep_and_user_variables_are_respected() {
        for allow in [false, true] {
            assert!(to_disable(Some("keep"), allow, |_| false).is_empty());
            assert!(to_disable(Some(" KEEP "), allow, |_| false).is_empty());
        }
        assert_eq!(to_disable(Some("other"), true, |_| false).len(), CRASHING.len());
        let left = to_disable(None, true, |v| v == "DISABLE_VULKAN_OW_OVERLAY_LAYER");
        assert_eq!(names(&left), ["VK_LAYER_OW_OBS_HOOK", "VK_LAYER_bandicam_helper"]);
        let left = to_disable(None, false, |v| v == "DISABLE_RTSS_LAYER");
        assert!(!names(&left).contains(&"VK_LAYER_RTSS"));
    }
}
