//! AURORA_DEMO_SIM_VERSION=1|on: the region's simulator version changes
//! during the run, as after a teleport to a region on another release.
//! `1` follows Preferences › Confirmations › Simulator version change (off
//! by default: no notice); `on` turns the notice on for the run without
//! saving it, so that it shows once.

use aurora_net::NetEvent;
use aurora_net::estate::EstateEvent;

/// The scenario asked, if any: `Some(true)` for `on`.
pub fn mode() -> Option<bool> {
    let mode = std::env::var("AURORA_DEMO_SIM_VERSION").ok()?;
    (!mode.is_empty() && mode != "0").then_some(mode == "on")
}

/// The notice is on for the run (`on`).
pub fn forced() -> bool {
    mode() == Some(true)
}

/// The events of `frame`, at 60 frames a second: the version of the arrival
/// (never told: the first one), then another one.
pub fn events(frame: u64, handle: aurora_net::RegionHandle) -> Vec<NetEvent> {
    let version = match frame {
        640 => "Second Life Server 2026-10-02.4893",
        700 => "Second Life Server 2026-10-23.5012",
        _ => return Vec::new(),
    };
    vec![NetEvent::Estate(EstateEvent::SimVersion {
        handle,
        version: version.into(),
    })]
}
