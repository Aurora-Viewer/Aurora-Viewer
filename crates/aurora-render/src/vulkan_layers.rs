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
//! manifest registers `graphics-hook64.dll` as an implicit layer, which
//! loads into Vulkan programs as soon as OBS is installed, running or not.
//! It was loaded in 61 % of the GPU device-lost reports of 0.8.5 to 0.9.1
//! (33 of 54) against 10 % of the other crash reports (3 of 30), most of
//! them hangs the driver reset with the GPU frames well under its time limit
//! (#2443): modules loaded, in a small sample of crashes, not proof that the
//! layer was active or caused a loss. OBS's Window Capture needs no layer
//! and keeps working; a streamer who wants Game Capture allows the overlays.
//!
//! Kept in any case during normal startup: ReShade's layer (installed on
//! purpose for a program), and the GPU drivers' own layers
//! (`VK_LAYER_NV_optimus`, `VK_LAYER_NV_present`, AMD's switchable graphics),
//! which choose the GPU and present. On Windows only, if normal enumeration
//! returns no adapter, Aurora retries first with AMD's switchable graphics
//! layer disabled, then lets Dozen back in. Some AMD layer versions
//! return `VK_INCOMPLETE` during physical-device enumeration; this fallback
//! preserves the normal layer path unless it prevents the viewer from starting
//! (#2366).
//!
//! The loader reads these variables from the process environment each time
//! it creates an instance. Overlay and Dozen settings are set at the start of
//! `main`, before any thread; Windows permits the AMD fallback to set its
//! manifest variable later, after enumeration fails. They are inherited by
//! the child processes (media plugins, crash watcher), which use no Vulkan
//! overlay either.
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

/// AMD's implicit switchable-graphics layer, left enabled except for the
/// Windows fallback after adapter enumeration fails.
pub const AMD_SWITCHABLE: Layer = Layer {
    program: "AMD switchable graphics",
    name: "VK_LAYER_AMD_switchable_graphics",
    variable: "DISABLE_LAYER_AMD_SWITCHABLE_GRAPHICS_1",
};
/// AMD's implicit Optimus-style switchable graphics layer on Windows.
pub const AMD_SWITCHABLE_LAYER: &str = AMD_SWITCHABLE.name;
/// The `disable_environment` value in AMD's switchable graphics manifest.
pub const AMD_SWITCHABLE_DISABLE_VARIABLE: &str = AMD_SWITCHABLE.variable;

/// Whether Aurora may temporarily disable AMD's implicit layer for an
/// adapter-enumeration retry.
pub fn amd_retry_allowed(keep: Option<&str>, is_set: bool) -> bool {
    !is_set && !keep.is_some_and(|value| value.trim().eq_ignore_ascii_case("keep"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumerationRetry {
    AmdSwitchableOff,
    DozenIn,
}

/// Retry AMD's layer first, then let Dozen back in if Aurora disabled it.
pub fn retry_plan(adapters_empty: bool, amd_allowed: bool, dozen_was_disabled: bool) -> Vec<EnumerationRetry> {
    if !adapters_empty {
        return Vec::new();
    }
    let mut retries = Vec::with_capacity(2);
    if amd_allowed {
        retries.push(EnumerationRetry::AmdSwitchableOff);
    }
    if dozen_was_disabled {
        retries.push(EnumerationRetry::DozenIn);
    }
    retries
}

/// Disable AMD's switchable layer only for the first empty-adapter retry.
/// Returns false on other platforms or when the user owns the relevant
/// environment setting.
pub fn disable_amd_switchable_for_retry(adapters_empty: bool) -> bool {
    #[cfg(windows)]
    {
        let keep = std::env::var("AURORA_VULKAN_LAYERS").ok();
        let user_set = std::env::var_os(AMD_SWITCHABLE_DISABLE_VARIABLE).is_some();
        if !adapters_empty || !amd_retry_allowed(keep.as_deref(), user_set) {
            return false;
        }
        // SAFETY: on Windows, std documents environment mutation as safe while
        // other threads run because it uses the operating system's process
        // environment lock.
        unsafe { std::env::set_var(AMD_SWITCHABLE_DISABLE_VARIABLE, "1") };
        true
    }
    #[cfg(not(windows))]
    {
        let _ = adapters_empty;
        false
    }
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

/// The names of the installed Vulkan layers enumerated by the loader. The
/// list can include layers disabled by our environment variables; it does
/// not say which layers an instance enabled. Only names that look like a
/// layer's (`VK_LAYER_` and word characters) are kept, at most 16, sorted.
/// Empty without a Vulkan loader or on macOS.
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

/// The loader's offered layer names with their Vulkan and implementation
/// versions, for a startup log that can identify stale vendor layers.
#[cfg(not(target_os = "macos"))]
pub fn offered_descriptions() -> Vec<String> {
    // SAFETY: loading the system's Vulkan library, as the XR start does.
    let Ok(entry) = (unsafe { ash::Entry::load() }) else {
        return Vec::new();
    };
    // SAFETY: a plain query of the loader, with a valid entry.
    let Ok(layers) = (unsafe { entry.enumerate_instance_layer_properties() }) else {
        return Vec::new();
    };
    let mut descriptions: Vec<_> = layers
        .iter()
        .filter_map(|layer| {
            let name = layer.layer_name_as_c_str().ok()?.to_str().ok()?;
            layer_description(name, layer.spec_version, layer.implementation_version)
        })
        .collect();
    descriptions.sort();
    descriptions.dedup();
    descriptions.truncate(16);
    descriptions
}

/// Vulkan instance version offered by the loader.
#[cfg(not(target_os = "macos"))]
pub fn loader_version() -> String {
    // SAFETY: loading the system Vulkan loader.
    let Ok(entry) = (unsafe { ash::Entry::load() }) else {
        return "unavailable (loader could not be loaded)".to_owned();
    };
    // SAFETY: plain loader version query.
    match unsafe { entry.try_enumerate_instance_version() } {
        Ok(Some(version)) => format!(
            "{}.{}.{}",
            ash::vk::api_version_major(version),
            ash::vk::api_version_minor(version),
            ash::vk::api_version_patch(version)
        ),
        Ok(None) => "1.0.0".to_owned(),
        Err(error) => format!("unavailable ({error:?})"),
    }
}

#[cfg(target_os = "macos")]
pub fn loader_version() -> String {
    "not applicable (Metal)".to_owned()
}

/// Vulkan loader and Aurora Vulkan override variables set in this process.
/// Path values redact the home-directory prefix before entering a log.
pub fn loader_environment() -> Vec<String> {
    const VARIABLES: &[&str] = &[
        "VK_DRIVER_FILES",
        "VK_ICD_FILENAMES",
        "VK_ADD_DRIVER_FILES",
        "VK_LOADER_DRIVERS_SELECT",
        "VK_LOADER_DRIVERS_DISABLE",
        "VK_LOADER_LAYERS_ENABLE",
        "VK_LOADER_LAYERS_DISABLE",
        "VK_LOADER_LAYERS_ALLOW",
        "VK_INSTANCE_LAYERS",
        "VK_LAYER_PATH",
        "VK_ADD_LAYER_PATH",
        "VK_IMPLICIT_LAYER_PATH",
        "VK_LOADER_DEVICE_ID_FILTER",
        "VK_LOADER_VENDOR_ID_FILTER",
        "VK_LOADER_DRIVER_ID_FILTER",
        "VK_LOADER_DEBUG",
        AMD_SWITCHABLE_DISABLE_VARIABLE,
        "DISABLE_LAYER_NV_OPTIMUS_1",
        "AURORA_VULKAN_LAYERS",
        "AURORA_VULKAN_DRIVERS",
    ];
    let mut values: Vec<_> = VARIABLES
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| format!("{name}={}", sanitize_log_value(&value.to_string_lossy()))))
        .collect();
    for (name, value) in std::env::vars_os() {
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("AURORA_VULKAN_") {
            continue;
        }
        let value = value.to_string_lossy();
        if !values.iter().any(|entry| entry.starts_with(&format!("{name}="))) {
            values.push(format!("{name}={}", sanitize_log_value(&value)));
        }
    }
    values.sort();
    values
}

fn sanitize_log_value(value: &str) -> String {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    redact_home(value, &home).chars().filter(|ch| !ch.is_control()).take(256).collect()
}

/// `value` with every `home` in it replaced by `<home>`, whatever the ASCII
/// case: Windows paths ignore it (`c:\users\name` is `C:\Users\Name`), and
/// on Linux a wider match only hides more. ASCII lowercasing keeps every
/// byte offset, so the matches found in the lowercase copy cut the value at
/// the same character boundaries.
fn redact_home(value: &str, home: &str) -> String {
    if home.is_empty() {
        return value.to_owned();
    }
    let (lower, needle) = (value.to_ascii_lowercase(), home.to_ascii_lowercase());
    let mut out = String::with_capacity(value.len());
    let mut rest = 0;
    for (at, _) in lower.match_indices(&needle) {
        out.push_str(&value[rest..at]);
        out.push_str("<home>");
        rest = at + needle.len();
    }
    out.push_str(&value[rest..]);
    out
}

/// Use ash directly to preserve the count-call VkResult that wgpu-hal logs
/// only as a translated error. Implicit layers remain active unless disabled
/// through their own environment setting.
#[cfg(not(target_os = "macos"))]
pub fn physical_device_count_probe() -> String {
    use ash::vk;
    use std::ptr;

    // SAFETY: load the system Vulkan loader.
    let entry = match unsafe { ash::Entry::load() } {
        Ok(entry) => entry,
        Err(error) => return format!("loader unavailable ({error})"),
    };
    // Ask for the loader's highest instance version, matching the viewer's
    // Vulkan version as closely as possible.
    // SAFETY: a plain version query on the valid loader entry.
    let api_version = unsafe { entry.try_enumerate_instance_version() }
        .ok()
        .flatten()
        .unwrap_or(vk::API_VERSION_1_0);
    let app = vk::ApplicationInfo::default().api_version(api_version);
    let create = vk::InstanceCreateInfo::default().application_info(&app);
    // SAFETY: no extensions, layers or pointers beyond the local application
    // info are requested; the loader validates the installed implicit layers.
    let instance = match unsafe { entry.create_instance(&create, None) } {
        Ok(instance) => instance,
        Err(error) => return format!("vkCreateInstance failed ({error:?})"),
    };
    let mut count = 0u32;
    // SAFETY: Vulkan specifies null pPhysicalDevices as the count query.
    let result = unsafe { (instance.fp_v1_0().enumerate_physical_devices)(instance.handle(), &mut count, ptr::null_mut()) };
    let report = format_count_result(result, count);
    // SAFETY: no child Vulkan objects outlive this instance.
    unsafe { instance.destroy_instance(None) };
    report
}

#[cfg(not(target_os = "macos"))]
fn format_count_result(result: ash::vk::Result, count: u32) -> String {
    format!("{result:?} (count {count})")
}

#[cfg(target_os = "macos")]
pub fn loaded_names() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "macos")]
pub fn offered_descriptions() -> Vec<String> {
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
fn layer_description(name: &str, spec_version: u32, implementation_version: u32) -> Option<String> {
    let name = clean_names(std::iter::once(name)).pop()?;
    Some(format!(
        "{name} (Vulkan {}.{}.{}, implementation {})",
        ash::vk::api_version_major(spec_version),
        ash::vk::api_version_minor(spec_version),
        ash::vk::api_version_patch(spec_version),
        implementation_version
    ))
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

/// Known layers that are installed and are not excluded by Aurora's
/// per-manifest disable variable or the loader's explicit disable filter.
/// This is eligibility only; it does not imply an instance enabled them.
pub fn eligible_known_names(installed: &[String], is_disabled: impl Fn(&str) -> bool, loader_filter: Option<&str>) -> Vec<String> {
    CRASHING
        .iter()
        .chain(OVERLAYS)
        .filter(|layer| installed.iter().any(|name| name == layer.name))
        .filter(|layer| !is_disabled(layer.variable))
        .filter(|layer| !loader_filter.is_some_and(|filter| filter.split(',').any(|item| layer_filter_matches(item.trim(), layer.name))))
        .map(|layer| layer.name.to_owned())
        .collect()
}

fn layer_filter_matches(filter: &str, name: &str) -> bool {
    filter == "*"
        || filter == name
        || filter
            .strip_suffix('*')
            .is_some_and(|prefix| !prefix.is_empty() && name.starts_with(prefix))
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn startup_layer_details_keep_only_safe_names_and_versions() {
        assert_eq!(
            layer_description("VK_LAYER_AMD_switchable_graphics", ash::vk::API_VERSION_1_3, 7).as_deref(),
            Some("VK_LAYER_AMD_switchable_graphics (Vulkan 1.3.0, implementation 7)")
        );
        assert!(layer_description("C:\\Users\\private\\layer.json", ash::vk::API_VERSION_1_3, 7).is_none());
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
    fn the_home_folder_is_hidden_whatever_its_case() {
        let home = "C:\\Users\\Someone";
        assert_eq!(
            redact_home("c:\\users\\someone\\layers;C:\\Users\\Someone\\b", home),
            "<home>\\layers;<home>\\b"
        );
        assert_eq!(redact_home("D:\\Vulkan\\nvidia.json", home), "D:\\Vulkan\\nvidia.json");
        assert_eq!(redact_home("C:\\Users\\Someone\\é\\x", home), "<home>\\é\\x");
        assert_eq!(redact_home("anything", ""), "anything");
    }

    #[test]
    fn amd_switchable_retry_respects_keep_and_user_override() {
        assert!(amd_retry_allowed(None, false));
        assert!(amd_retry_allowed(Some("other"), false));
        assert!(!amd_retry_allowed(Some("keep"), false));
        assert!(!amd_retry_allowed(Some(" KEEP "), false));
        assert!(!amd_retry_allowed(None, true));
        assert!(!CRASHING.iter().any(|layer| layer.name == AMD_SWITCHABLE.name));
        assert!(!OVERLAYS.iter().any(|layer| layer.name == AMD_SWITCHABLE.name));
    }

    #[test]
    fn empty_enumeration_retries_amd_before_dozen() {
        use EnumerationRetry::{AmdSwitchableOff, DozenIn};

        assert!(retry_plan(false, true, true).is_empty());
        assert_eq!(retry_plan(true, true, true), [AmdSwitchableOff, DozenIn]);
        assert_eq!(retry_plan(true, false, true), [DozenIn]);
        assert_eq!(retry_plan(true, true, false), [AmdSwitchableOff]);
        assert!(retry_plan(true, false, false).is_empty());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn raw_count_probe_keeps_the_vk_result_and_count() {
        assert_eq!(format_count_result(ash::vk::Result::INCOMPLETE, 0), "INCOMPLETE (count 0)");
        assert_eq!(format_count_result(ash::vk::Result::SUCCESS, 2), "SUCCESS (count 2)");
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

    #[test]
    fn eligibility_uses_known_disable_variables_and_loader_filters_without_claiming_activation() {
        let installed = ["VK_LAYER_OBS_HOOK", "VK_LAYER_RTSS", "VK_LAYER_NV_present"].map(str::to_owned);
        let eligible = eligible_known_names(&installed, |name| name == "DISABLE_RTSS_LAYER", Some("VK_LAYER_OBS_*"));
        assert!(eligible.is_empty());
        let eligible = eligible_known_names(&installed, |_| false, None);
        assert_eq!(eligible, ["VK_LAYER_OBS_HOOK", "VK_LAYER_RTSS"]);
        assert!(!eligible.iter().any(|name| name == "VK_LAYER_NV_present"));
    }
}
