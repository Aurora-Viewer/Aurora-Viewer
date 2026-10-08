//! Avatar impostors, after LLVOAvatar::updateImpostor / needsImpostorUpdate
//! of the Second Life / Firestorm viewer (originally LGPL 2.1, Copyright (C) 2001-2024
//! Linden Research, Inc. and the Firestorm project): avatars beyond the
//! nearest RenderAvatarMaxNonImpostors are drawn as a card showing a picture
//! of them, taken from the camera and refreshed now and then (more often when
//! near, at once when the view turns or the avatar moves).

use aurora_render::{DrawCmd, IMPOSTOR_CAPTURES, IMPOSTOR_TILES, ImpostorCapture, ImpostorSprite};
use glam::Vec3;
use std::collections::HashMap;
use std::time::Instant;

/// Half size of the framed square around an avatar (m): body, hair, worn
/// objects.
const HALF: f32 = 1.4;
/// View turned by more than this since the picture: take a new one.
const MAX_ANGLE_COS: f32 = 0.9976; // 4°
/// The avatar moved by more than this: take a new one.
const MAX_MOVE: f32 = 0.3;

#[derive(Debug, Clone, Copy)]
struct State {
    tile: u32,
    /// Picture taken: when, from which direction, of which center.
    taken: Option<(Instant, Vec3, Vec3)>,
    /// Card axes of the picture (half extents).
    right: Vec3,
    up: Vec3,
    seen: Instant,
}

#[derive(Default)]
pub struct Impostors {
    states: HashMap<usize, State>,
    free: Vec<u32>,
    initialized: bool,
}

/// This frame's impostor work: avatars to picture (slot in the captures)
/// and the cards to draw.
pub struct Plan {
    /// Avatar object index -> capture slot.
    pub capture: HashMap<usize, usize>,
    pub captures: Vec<ImpostorCapture>,
    pub sprites: Vec<ImpostorSprite>,
}

/// Picture period: near avatars more often (LL refreshes impostors on a
/// period that grows with distance).
fn period(distance: f32) -> f32 {
    (0.15 + distance / 60.0).min(1.0)
}

impl Impostors {
    /// `avatars`: (object index, center, distance, in view) of the avatars to
    /// draw as impostors.
    pub fn plan(&mut self, avatars: &[(usize, Vec3, f32, bool)], eye: Vec3, now: Instant) -> Plan {
        if !self.initialized {
            self.initialized = true;
            self.free = (0..IMPOSTOR_TILES).rev().collect();
        }
        // forget avatars not seen as impostors for a while
        let free = &mut self.free;
        self.states.retain(|_, s| {
            let keep = now.duration_since(s.seen).as_secs_f32() < 5.0;
            if !keep {
                free.push(s.tile);
            }
            keep
        });
        let mut wanted: Vec<(f32, usize)> = Vec::new();
        for &(idx, center, distance, in_view) in avatars {
            if !self.states.contains_key(&idx) {
                let Some(tile) = self.free.pop() else {
                    continue; // atlas full: not drawn
                };
                self.states.insert(
                    idx,
                    State {
                        tile,
                        taken: None,
                        right: Vec3::X,
                        up: Vec3::Z,
                        seen: now,
                    },
                );
            }
            let Some(s) = self.states.get_mut(&idx) else {
                continue;
            };
            s.seen = now;
            if !in_view {
                continue;
            }
            let dir = (center - eye).normalize_or(Vec3::X);
            let stale = match s.taken {
                None => f32::INFINITY,
                Some((at, d, c)) => {
                    let age = now.duration_since(at).as_secs_f32();
                    if dir.dot(d) < MAX_ANGLE_COS || center.distance(c) > MAX_MOVE {
                        age + 10.0
                    } else if age > period(distance) {
                        age
                    } else {
                        continue;
                    }
                }
            };
            wanted.push((stale, idx));
        }
        // the stalest first
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut plan = Plan {
            capture: HashMap::new(),
            captures: Vec::new(),
            sprites: Vec::new(),
        };
        for (_, idx) in wanted.into_iter().take(IMPOSTOR_CAPTURES) {
            let Some(&(_, center, _, _)) = avatars.iter().find(|a| a.0 == idx) else {
                continue;
            };
            let Some(s) = self.states.get_mut(&idx) else {
                continue;
            };
            let dir = (center - eye).normalize_or(Vec3::X);
            let world_up = if dir.z.abs() > 0.95 { Vec3::Y } else { Vec3::Z };
            let right = dir.cross(world_up).normalize_or(Vec3::X);
            let up = right.cross(dir);
            let eye_c = center - dir * (HALF + 1.0);
            s.taken = Some((now, dir, center));
            s.right = right * HALF;
            s.up = up * HALF;
            plan.capture.insert(idx, plan.captures.len());
            plan.captures.push(ImpostorCapture {
                tile: s.tile,
                eye: eye_c,
                view: glam::camera::rh::view::look_at_mat4(eye_c, center, up),
                half: HALF,
                depth: 2.0 * HALF + 2.0,
                opaque: Vec::new(),
                blend: Vec::new(),
            });
        }
        // cards of the pictured avatars in view, following them
        for &(idx, center, _, in_view) in avatars {
            let Some(s) = self.states.get(&idx) else {
                continue;
            };
            if in_view && s.taken.is_some() {
                plan.sprites.push(ImpostorSprite {
                    center: center.to_array(),
                    tile: s.tile,
                    right: s.right.to_array(),
                    up: s.up.to_array(),
                });
            }
        }
        plan
    }
}

/// Draws of a pictured avatar, by pass.
pub fn add_draw(capture: &mut ImpostorCapture, cmd: DrawCmd, blend: bool) {
    if blend {
        capture.blend.push(cmd);
    } else {
        capture.opaque.push(cmd);
    }
}
