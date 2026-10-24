//! Third-party Vulkan layers kept out of the viewer's process.
//!
//! Overlay and capture programs register implicit Vulkan layers that the
//! Vulkan loader injects into every Vulkan application, inside
//! `vkCreateInstance`. Some of them crash there: issue #901, a crash one
//! second after start in `wgpu::Instance::new`, with Overwolf's overlay
//! layer (`OWClient`) calling into a capture hook (`graphics-hook64`)
//! before Aurora had drawn anything. Nothing of those overlays works in the
//! viewer anyway (they target games), so their layers are disabled through
//! the variable each layer's manifest declares (`disable_environment`),
//! which every loader version honours, unlike `VK_LOADER_LAYERS_DISABLE`
//! (loader 1.3.234 and later). OBS Studio's own game capture layer
//! (`VK_LAYER_OBS_HOOK`) is kept: streamers capture the viewer with it.
//!
//! The loader reads these variables from the process environment each time
//! it looks for layers, so setting them at the start of `main`, before any
//! thread, is enough. They are inherited by the child processes (media
//! plugins, crash watcher), which use no Vulkan overlay either.
//! `AURORA_VULKAN_LAYERS=keep` leaves the environment as it is; a variable
//! the user already set is left untouched.

/// The layers disabled, with the variable their manifest declares and the
/// value that disables them.
pub const DISABLED: &[Layer] = &[
    // Overwolf's in-game overlay (owclient.dll, ow-vulkan-overlay64.json)
    Layer {
        name: "VK_LAYER_OW_OVERLAY",
        variable: "DISABLE_VULKAN_OW_OVERLAY_LAYER",
        value: "1",
    },
    // Overwolf's capture hook (ow-graphics-vulkan.dll, ow-graphics-vulkan64.json)
    Layer {
        name: "VK_LAYER_OW_OBS_HOOK",
        variable: "DISABLE_VULKAN_OW_OBS_CAPTURE",
        value: "1",
    },
];

/// One implicit layer and the environment variable that disables it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    pub name: &'static str,
    pub variable: &'static str,
    pub value: &'static str,
}

/// The layers to disable: none with `AURORA_VULKAN_LAYERS=keep` (`keep`),
/// and none whose variable is already set (`is_set`).
pub fn to_disable(keep: Option<&str>, is_set: impl Fn(&str) -> bool) -> Vec<Layer> {
    if keep.is_some_and(|v| v.trim().eq_ignore_ascii_case("keep")) {
        return Vec::new();
    }
    DISABLED.iter().copied().filter(|l| !is_set(l.variable)).collect()
}

/// Sets the disabling variables in the process environment and returns the
/// layers disabled, for the log (written once logging is up).
///
/// # Safety
///
/// Must be called before any other thread of the process starts (the
/// first lines of `main`): it changes the environment.
pub unsafe fn disable() -> Vec<Layer> {
    let keep = std::env::var("AURORA_VULKAN_LAYERS").ok();
    let layers = to_disable(keep.as_deref(), |v| std::env::var_os(v).is_some());
    for l in &layers {
        // SAFETY: the caller guarantees that no other thread runs yet, so
        // nothing reads the environment while it changes.
        unsafe { std::env::set_var(l.variable, l.value) };
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disables_the_overwolf_layers_by_default() {
        let names: Vec<_> = to_disable(None, |_| false).iter().map(|l| l.name).collect();
        assert_eq!(names, ["VK_LAYER_OW_OVERLAY", "VK_LAYER_OW_OBS_HOOK"]);
        // OBS Studio's game capture stays
        assert!(DISABLED.iter().all(|l| !l.name.contains("VK_LAYER_OBS_HOOK")));
    }

    #[test]
    fn keep_and_user_variables_are_respected() {
        assert!(to_disable(Some("keep"), |_| false).is_empty());
        assert!(to_disable(Some(" KEEP "), |_| false).is_empty());
        assert_eq!(to_disable(Some("other"), |_| false).len(), 2);
        let left = to_disable(None, |v| v == "DISABLE_VULKAN_OW_OVERLAY_LAYER");
        assert_eq!(left.iter().map(|l| l.name).collect::<Vec<_>>(), ["VK_LAYER_OW_OBS_HOOK"]);
    }
}
