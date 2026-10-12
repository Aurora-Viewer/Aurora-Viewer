//! Avatar impostors, after LLVOAvatar::updateImpostor / needsImpostorUpdate
//! of the Second Life / Firestorm viewer (originally LGPL 2.1, Copyright (C) 2001-2024
//! Linden Research, Inc. and the Firestorm project): avatars beyond the
//! nearest RenderAvatarMaxNonImpostors are drawn as a card showing a picture
//! of them, taken from the camera and refreshed now and then (more often when
//! near, at once when the view turns or the avatar moves).

use aurora_render::{DrawCmd, IMPOSTOR_CAPTURES, IMPOSTOR_FULLBRIGHT, IMPOSTOR_TILE, IMPOSTOR_TILES, ImpostorCapture, ImpostorSprite};
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

/// An avatar to draw as an impostor.
#[derive(Debug, Clone, Copy)]
pub struct Far {
    /// Object index.
    pub idx: usize,
    pub center: Vec3,
    pub distance: f32,
    pub in_view: bool,
    /// Over the complexity limit: its picture is its fullbright silhouette.
    pub silhouette: bool,
}

/// Pixels an object of radius `radius` (m) covers in the picture of its
/// avatar: its textures are of no use finer than that.
pub fn picture_pixels(radius: f32) -> f32 {
    radius / HALF * IMPOSTOR_TILE as f32
}

#[derive(Debug, Clone, Copy)]
struct State {
    tile: u32,
    /// Picture taken: when, from which direction, of which center.
    taken: Option<(Instant, Vec3, Vec3)>,
    /// Card axes of the picture (half extents).
    right: Vec3,
    up: Vec3,
    /// The picture shows the silhouette (`Far::silhouette` when taken).
    silhouette: bool,
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
    /// `avatars`: the avatars to draw as impostors.
    pub fn plan(&mut self, avatars: &[Far], eye: Vec3, now: Instant) -> Plan {
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
        for a in avatars {
            let Far { idx, center, distance, .. } = *a;
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
                        silhouette: a.silhouette,
                        seen: now,
                    },
                );
            }
            let Some(s) = self.states.get_mut(&idx) else {
                continue;
            };
            s.seen = now;
            if !a.in_view {
                continue;
            }
            let dir = (center - eye).normalize_or(Vec3::X);
            let stale = match s.taken {
                None => f32::INFINITY,
                // it went over the complexity limit, or back under it: its
                // picture shows what it no longer is
                Some(_) if s.silhouette != a.silhouette => f32::INFINITY,
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
            let Some(a) = avatars.iter().find(|a| a.idx == idx) else {
                continue;
            };
            let Some(s) = self.states.get_mut(&idx) else {
                continue;
            };
            let center = a.center;
            let dir = (center - eye).normalize_or(Vec3::X);
            let world_up = if dir.z.abs() > 0.95 { Vec3::Y } else { Vec3::Z };
            let right = dir.cross(world_up).normalize_or(Vec3::X);
            let up = right.cross(dir);
            let eye_c = center - dir * (HALF + 1.0);
            s.taken = Some((now, dir, center));
            s.right = right * HALF;
            s.up = up * HALF;
            s.silhouette = a.silhouette;
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
        for a in avatars {
            let Some(s) = self.states.get(&a.idx) else {
                continue;
            };
            if a.in_view && s.taken.is_some() {
                plan.sprites.push(ImpostorSprite {
                    center: a.center.to_array(),
                    tile: s.tile | if s.silhouette { IMPOSTOR_FULLBRIGHT } else { 0 },
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn far(idx: usize, silhouette: bool) -> Far {
        Far {
            idx,
            center: Vec3::new(20.0, 0.0, 1.0),
            distance: 20.0,
            in_view: true,
            silhouette,
        }
    }

    #[test]
    fn a_picture_is_taken_again_when_the_avatar_becomes_a_silhouette() {
        let mut imp = Impostors::default();
        let t0 = Instant::now();
        let plan = imp.plan(&[far(7, false)], Vec3::ZERO, t0);
        assert_eq!(plan.capture.get(&7), Some(&0));
        assert_eq!(plan.sprites[0].tile & IMPOSTOR_FULLBRIGHT, 0);
        // a moment later, well within its period: the picture is kept
        let t1 = t0 + Duration::from_millis(50);
        assert!(imp.plan(&[far(7, false)], Vec3::ZERO, t1).captures.is_empty());
        // over the complexity limit: pictured at once, and its card is
        // marked fullbright with the picture that shows the silhouette
        let plan = imp.plan(&[far(7, true)], Vec3::ZERO, t1);
        assert_eq!(plan.captures.len(), 1);
        assert_eq!(plan.sprites[0].tile & IMPOSTOR_FULLBRIGHT, IMPOSTOR_FULLBRIGHT);
        assert_eq!(plan.sprites[0].tile & !IMPOSTOR_FULLBRIGHT, plan.captures[0].tile);
        // and back under the limit
        let plan = imp.plan(&[far(7, false)], Vec3::ZERO, t1);
        assert_eq!(plan.captures.len(), 1);
        assert_eq!(plan.sprites[0].tile & IMPOSTOR_FULLBRIGHT, 0);
    }

    #[test]
    fn a_card_keeps_the_flag_of_its_picture_until_the_next_one() {
        let mut imp = Impostors::default();
        let t0 = Instant::now();
        // more avatars turn into silhouettes than pictures are taken in a frame
        let n = IMPOSTOR_CAPTURES + 1;
        let before: Vec<Far> = (0..n).map(|i| far(i, false)).collect();
        let mut t = t0;
        while imp.plan(&before, Vec3::ZERO, t).sprites.len() < n {
            t += Duration::from_millis(1);
        }
        let after: Vec<Far> = (0..n).map(|i| far(i, true)).collect();
        let plan = imp.plan(&after, Vec3::ZERO, t);
        assert_eq!(plan.captures.len(), IMPOSTOR_CAPTURES);
        let marked = plan.sprites.iter().filter(|s| s.tile & IMPOSTOR_FULLBRIGHT != 0).count();
        assert_eq!(marked, IMPOSTOR_CAPTURES, "the avatar not pictured yet still shows its old picture");
    }

    #[test]
    fn textures_are_wanted_as_fine_as_the_picture_shows_them() {
        // the framed square is a tile wide
        assert_eq!(picture_pixels(HALF), IMPOSTOR_TILE as f32);
        assert_eq!(picture_pixels(HALF / 4.0), IMPOSTOR_TILE as f32 / 4.0);
    }
}
