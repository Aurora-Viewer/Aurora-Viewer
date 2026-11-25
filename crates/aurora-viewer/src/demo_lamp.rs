//! A lamp 6 m ahead of the start view, 2.5 m above the ground, that a
//! script would switch or carry, for what keeps a history of the light and
//! must let it go when a lamp changes (the traced indirect light and the
//! lamps' traced shadows, aurora-render `rt_light_changes.rs`):
//!
//! - AURORA_DEMO_LAMP_BLINK=<frames>: lit for `frames` frames (240 by
//!   default), dark for as many, and so on from the start;
//! - AURORA_DEMO_LAMP_MOVE=<frames>[,<stop>]: carried from 3 m on one side
//!   to 3 m on the other in `frames` frames (120 by default) and back, and
//!   so on; from the frame `stop` it stays where it is. A post stands
//!   1.5 m before it, whose shadow crosses the floor the other way.
//!
//! Both fall on whole frames, so a capture can be taken a given number of
//! frames after a switch, or on the frame the lamp stops and long after;
//! the log names each switch and the stop.

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::params::*;
use aurora_prim::{ExtraParams, LightParams};
use glam::{Quat, Vec3};

const LOCAL_ID: u32 = 81_600;
/// How far to each side the lamp is carried (m).
const SIDE: f32 = 3.0;

pub struct LampDemo {
    blink: Option<u64>,
    /// The frames of a crossing, and the frame the lamp stops at.
    carried: Option<(u64, Option<u64>)>,
    /// The lamp's place when it is not carried, and the way it is carried.
    place: Option<(Vec3, Vec3)>,
    shown: Option<(bool, f32)>,
}

/// Whether the lamp is lit at `frame`, switched every `every` frames.
fn lit_at(frame: u64, every: u64) -> bool {
    (frame / every).is_multiple_of(2)
}

/// How far to the side the lamp is at `frame` (m), a crossing taking
/// `crossing` frames.
fn side_at(frame: u64, crossing: u64) -> f32 {
    let along = (frame % (2 * crossing)) as f32 / crossing as f32;
    SIDE * (1.0 - 2.0 * (along - 1.0).abs())
}

fn frames(value: &str, default: u64) -> u64 {
    value.trim().parse::<u64>().ok().filter(|n| *n > 0).unwrap_or(default)
}

impl LampDemo {
    pub fn from_env() -> Option<Self> {
        let blink = std::env::var("AURORA_DEMO_LAMP_BLINK").ok().map(|v| frames(&v, 240));
        let carried = std::env::var("AURORA_DEMO_LAMP_MOVE").ok().map(|v| {
            let mut values = v.split(',');
            let crossing = frames(values.next().unwrap_or(""), 120);
            (crossing, values.next().and_then(|stop| stop.trim().parse::<u64>().ok()))
        });
        (blink.is_some() || carried.is_some()).then_some(LampDemo {
            blink,
            carried,
            place: None,
            shown: None,
        })
    }

    /// Once a frame, with the camera's position and direction.
    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World, eye: Vec3, forward: Vec3) {
        let lit = self.blink.is_none_or(|every| lit_at(frame, every));
        let side = self
            .carried
            .map_or(0.0, |(crossing, stop)| side_at(stop.map_or(frame, |s| frame.min(s)), crossing));
        if self.shown == Some((lit, side)) {
            return;
        }
        let first = self.place.is_none();
        let (place, across) = *self.place.get_or_insert_with(|| {
            let ahead = Vec3::new(forward.x, forward.y, 0.0).try_normalize().unwrap_or(Vec3::X);
            let at = eye + ahead * 6.0;
            (Vec3::new(at.x, at.y, height(at.x, at.y) + 2.5), Vec3::new(-ahead.y, ahead.x, 0.0))
        });
        if first && self.carried.is_some() {
            let ahead = Vec3::new(across.y, -across.x, 0.0);
            world.apply(NetEvent::ObjectUpdates {
                handle: HANDLE,
                objects: vec![prim(
                    LOCAL_ID + 1,
                    place - ahead * 1.5 - Vec3::Z * 1.55,
                    Quat::IDENTITY,
                    Vec3::new(0.3, 0.3, 1.5),
                    shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                    te([0.5, 0.5, 0.55, 1.0], 0, false, 0.0),
                    ExtraParams::default(),
                    "",
                )],
            });
        }
        world.apply(NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![prim(
                LOCAL_ID,
                place + across * side,
                Quat::IDENTITY,
                Vec3::splat(0.3),
                shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                if lit {
                    te([1.0, 0.85, 0.6, 1.0], 0, true, 0.2)
                } else {
                    te([0.2, 0.17, 0.12, 1.0], 0, false, 0.0)
                },
                ExtraParams {
                    light: lit.then_some(LightParams {
                        color: [1.0, 0.8, 0.55, 1.0],
                        radius: 10.0,
                        cutoff: 0.0,
                        falloff: 0.75,
                    }),
                    ..Default::default()
                },
                "",
            )],
        });
        if self.shown.is_none_or(|(was, _)| was != lit) {
            log::info!("demo lamp: {} at frame {frame}", if lit { "lit" } else { "dark" });
        }
        if self.carried.is_some_and(|(_, stop)| stop == Some(frame)) {
            log::info!("demo lamp: stopped at frame {frame}, {side:.2} m to the side");
        }
        self.shown = Some((lit, side));
    }
}

#[cfg(test)]
mod tests {
    use super::{SIDE, lit_at, side_at};

    #[test]
    fn the_lamp_switches_on_whole_frames() {
        assert!(lit_at(0, 240));
        assert!(lit_at(239, 240));
        assert!(!lit_at(240, 240));
        assert!(!lit_at(479, 240));
        assert!(lit_at(480, 240));
    }

    #[test]
    fn the_lamp_is_carried_from_side_to_side_and_back() {
        assert_eq!(side_at(0, 120), -SIDE);
        assert_eq!(side_at(60, 120), 0.0);
        assert_eq!(side_at(120, 120), SIDE);
        assert_eq!(side_at(180, 120), 0.0);
        assert_eq!(side_at(240, 120), -SIDE);
        // the same step at every frame: 6 m in 120 frames
        assert!((side_at(31, 120) - side_at(30, 120) - 0.05).abs() < 1e-5);
    }
}
