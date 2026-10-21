//! AURORA_DEMO_COLLISIONS: three residents bump the agent, push it with a
//! script and hit it with objects, a few seconds apart, and the "Bumps,
//! pushes & hits" window opens on the list. `chat` also turns on the
//! nearby chat lines for this run (Preferences › Interface).

use super::{DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use crate::world::collisions::Settings;
use aurora_net::{MeanCollision, NetEvent};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_COLLISIONS").ok()
}

/// The saved options, with the nearby chat lines for `chat`.
pub fn settings(saved: Settings) -> Settings {
    let chat = mode().is_some_and(|m| m.trim().eq_ignore_ascii_case("chat"));
    Settings {
        in_chat: saved.in_chat || chat,
        ..saved
    }
}

/// The frame the window opens at.
pub const OPEN: u64 = 600;

fn alert(blocks: &[(Uuid, u8, f32)]) -> Vec<NetEvent> {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
    let list = blocks
        .iter()
        .map(|&(perp, kind, magnitude)| MeanCollision {
            perp,
            time,
            magnitude,
            kind,
        })
        .collect();
    vec![NetEvent::MeanCollisions(list)]
}

/// The events of `frame`, at 60 frames a second.
pub fn events(frame: u64) -> Vec<NetEvent> {
    match frame {
        640 => alert(&[(DEMO_NOVA, 1, 4.2)]),
        760 => alert(&[(DEMO_TESS, 2, 38.0)]),
        880 => alert(&[(DEMO_LOUP, 4, 12.6), (DEMO_LOUP, 5, 9.1)]),
        // the first resident again: that row comes back to the top with
        // the new time; a type the viewer does not know is left out
        1000 => alert(&[(DEMO_NOVA, 1, 6.0), (DEMO_TESS, 9, 1.0), (DEMO_TESS, 3, 3.3)]),
        _ => Vec::new(),
    }
}
