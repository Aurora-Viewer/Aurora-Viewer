//! Startup policy shared by desktop and headset rendering.

/// Explicit off wins over the saved preference and offline XR experiments.
pub fn should_load_openxr(setting: bool, env_xr: Option<&str>, demo_xr: bool) -> bool {
    match env_xr {
        Some("off") => false,
        Some("on") => true,
        _ => setting || demo_xr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openxr_requires_opt_in_and_explicit_off_always_wins() {
        for setting in [false, true] {
            for demo in [false, true] {
                assert!(!should_load_openxr(setting, Some("off"), demo));
                assert!(should_load_openxr(setting, Some("on"), demo));
                for env in [None, Some("invalid"), Some("")] {
                    assert_eq!(should_load_openxr(setting, env, demo), setting || demo);
                }
            }
        }
    }
}
