//! Resolve all VR entry controls before requesting or probing a runtime.

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Toggle {
    Disabled,
    Request,
    Restart,
    Unavailable,
}

pub(super) fn toggle(support: bool, available: bool, probe: impl FnOnce() -> bool) -> Toggle {
    if !support {
        Toggle::Disabled
    } else if available {
        Toggle::Request
    } else if probe() {
        Toggle::Restart
    } else {
        Toggle::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openxr_disabled_toggle_neither_requests_nor_probes_runtime() {
        for available in [false, true] {
            assert_eq!(toggle(false, available, || panic!("disabled VR probed runtime")), Toggle::Disabled);
        }
    }

    #[test]
    fn openxr_enabled_toggle_preserves_headset_and_restart_paths() {
        assert_eq!(toggle(true, true, || panic!("available VR need not probe")), Toggle::Request);
        assert_eq!(toggle(true, false, || true), Toggle::Restart);
        assert_eq!(toggle(true, false, || false), Toggle::Unavailable);
    }
}
