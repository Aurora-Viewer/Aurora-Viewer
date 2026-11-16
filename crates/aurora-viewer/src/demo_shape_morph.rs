//! Progressive shape changes (AURORA_DEMO_SHAPE_MORPH=1, docs/TESTING.md):
//! from frame 600, every 200 frames, the Wolf and the crowd of
//! AURORA_DEMO_CROWD take the other of two shapes, tall and thin then short
//! and broad. The first one lands at once (a first appearance); each change
//! after it glides over 0.65 s, stepping back a little, past the new shape,
//! then settling.

use super::{DEMO_LOUP, crowd_count};
use aurora_net::NetEvent;
use std::sync::OnceLock;
use uuid::Uuid;

pub fn active() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_DEMO_SHAPE_MORPH").is_ok_and(|v| !v.is_empty() && v != "0"))
}

const START: u64 = 600;
const PERIOD: u64 = 200;

/// (parameter, weight): height 33, body thickness 34, body fat 637.
const TALL: [(i32, f32); 3] = [(33, 1.8), (34, -0.6), (637, 0.0)];
const BROAD: [(i32, f32); 3] = [(33, -1.8), (34, 1.4), (637, 0.8)];

/// The avatars that change: the Wolf and the crowd.
fn avatars() -> Vec<Uuid> {
    let crowd = crowd_count().unwrap_or(0);
    std::iter::once(DEMO_LOUP)
        .chain((0..crowd).map(|k| Uuid::from_u128(0xA0E0_C0D0_0000_0000_0000_0000_0000_0000 | k as u128)))
        .collect()
}

/// The appearances sent at `frame`, if it is a change's.
pub fn tick(frame: u64, shapes: &crate::scene::shape::ShapeParams) -> Vec<NetEvent> {
    if !active() || frame < START || !(frame - START).is_multiple_of(PERIOD) {
        return Vec::new();
    }
    let broad = ((frame - START) / PERIOD).is_multiple_of(2);
    // the classic avatars' bakes, so that the crowd shows a baked body
    let bakes = super::classic_events(shapes).into_iter().find_map(|ev| match ev {
        NetEvent::Appearance(a) if a.avatar_id == DEMO_LOUP => a.texture_entry,
        _ => None,
    });
    let values = shapes.values_with(if broad { &BROAD } else { &TALL });
    avatars()
        .into_iter()
        .map(|avatar_id| {
            NetEvent::Appearance(aurora_net::AvatarAppearance {
                avatar_id,
                texture_entry: bakes.clone(),
                visual_params: values.clone(),
                cof_version: 1,
                appearance_version: 1,
                hover_height: 0.0,
                attachments: Vec::new(),
            })
        })
        .collect()
}
