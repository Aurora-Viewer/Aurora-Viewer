//! AURORA_DEMO_VOICE_CONTROLS: the toolbar's Voice controls window opens
//! on the nearby channel with three residents in it: Nova talks in bursts,
//! Tess is silent, and Loup leaves the channel for good every 40 s (listed as
//! "Left" for 10 s, then removed, and back 20 s later). Set
//! AURORA_DEMO_CALL=conference (or group, p2p) as well for the window on a
//! call's channel: the call's own participants replace these.

use super::{DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use std::collections::HashMap;
use uuid::Uuid;

pub fn enabled() -> bool {
    std::env::var_os("AURORA_DEMO_VOICE_CONTROLS").is_some()
}

/// The frame the window opens at.
pub const OPEN: u64 = 600;

/// The residents in the nearby channel at `time`: (raw level, speaking).
pub fn voice(levels: &mut HashMap<Uuid, (f32, bool)>, time: f64) {
    if !enabled() {
        return;
    }
    let burst = time.rem_euclid(6.0) < 3.5;
    let level = if burst { 0.4 + 0.2 * (time * 3.0).sin().abs() as f32 } else { 0.0 };
    levels.insert(DEMO_NOVA, (level, burst));
    levels.insert(DEMO_TESS, (0.0, false));
    // Loup talks in turn with Nova while he is in the channel
    if time.rem_euclid(40.0) < 20.0 {
        levels.insert(DEMO_LOUP, if burst { (0.0, false) } else { (0.5, true) });
    } else {
        levels.remove(&DEMO_LOUP);
    }
}
