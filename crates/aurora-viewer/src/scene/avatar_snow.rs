//! Aurora Weather: snow gathering on avatars. Out in falling snow an
//! avatar slowly whitens on what faces up (shoulders, a hood, hair, a hat:
//! the object shader, `eval_material`); under cover, in the rain, or once
//! the snow stops, it melts away again. Under cover means something solid
//! above the head (a roof, an umbrella held up): a ray straight up, now and
//! then, through the picking's triangles.
//!
//! The amount goes to the renderer in the avatar's joint palette, one
//! matrix after its live morph weights (`anim::AVATAR_SNOW`, read by
//! `vs_main`), so what it wears takes it too.

use super::Scene;
use crate::world::World;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The palette entry carrying `amount` of snow (0..1) to the shader, with
/// the mark it needs to read it as snow (`aurora_render::AVATAR_SNOW_MARK`).
/// No snow (none gathered, or "Snow on avatars" off) is an empty entry
/// without the mark: the shader leaves the mesh as it is.
pub fn palette_entry(amount: f32) -> [[f32; 4]; 4] {
    if amount <= 0.0 {
        return [[0.0; 4]; 4];
    }
    [
        [amount, 0.0, 0.0, 0.0],
        [0.0; 4],
        [0.0; 4],
        [0.0, 0.0, 0.0, aurora_render::AVATAR_SNOW_MARK],
    ]
}

/// Only avatars this near the camera gather snow (m).
const REACH: f32 = 64.0;
/// How often each looks up for cover.
const LOOK: Duration = Duration::from_millis(500);
/// Cover counts within this far above the head (m).
const COVER: f32 = 60.0;

#[derive(Clone, Copy, Debug)]
struct Snowed {
    amount: f32,
    covered: bool,
    next_look: Instant,
}

#[derive(Default)]
pub struct AvatarSnow {
    avatars: HashMap<Uuid, Snowed>,
    last: Option<Instant>,
}

/// How the snow on an avatar changes over `dt` s: it gathers in falling
/// snow (`snow`, 0..1) out in the open, in two or three minutes when it
/// falls hard; it melts in rain in about a minute, under cover in two, and
/// out in the open once it stops in five.
fn step(amount: f32, dt: f32, snow: f32, rain: f32, covered: bool) -> f32 {
    let next = if snow > 0.0 && !covered {
        amount + dt * (0.2 + snow) / 150.0
    } else if rain > 0.0 {
        amount - dt / 60.0
    } else if covered {
        amount - dt / 120.0
    } else {
        amount - dt / 300.0
    };
    next.clamp(0.0, 1.0)
}

impl AvatarSnow {
    /// One frame: `snow` how hard it snows (0 when it does not), `rain` how
    /// hard it rains, near `eye`.
    pub fn update(&mut self, scene: &Scene, world: &World, now: Instant, snow: f32, rain: f32, eye: Vec3) {
        let dt = self.last.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32().min(1.0));
        self.last = Some(now);
        let mut seen = HashSet::new();
        for (idx, o) in world.objects.avatars() {
            let id = o.full_id;
            let Some((p, _, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if (p - eye).length() > REACH {
                continue;
            }
            seen.insert(id);
            let s = self.avatars.entry(id).or_insert(Snowed {
                amount: 0.0,
                covered: false,
                next_look: now,
            });
            // nothing to do while none falls and none lies
            if snow <= 0.0 && s.amount <= 0.0 {
                continue;
            }
            if now >= s.next_look {
                s.next_look = now + LOOK;
                let head = scene.head_world(world, &id, now).unwrap_or(p + Vec3::Z * 0.7);
                s.covered = scene.covered_above(world, head + Vec3::Z * 0.35, COVER);
            }
            s.amount = step(s.amount, dt, snow, rain, s.covered);
        }
        self.avatars.retain(|id, s| seen.contains(id) || s.amount > 0.0);
    }

    /// No avatar has snow left on it: with none falling, nothing to update.
    pub fn is_clear(&self) -> bool {
        self.avatars.values().all(|s| s.amount <= 0.0)
    }

    /// How much snow is on an avatar (0..1).
    pub fn amount(&self, id: &Uuid) -> f32 {
        self.avatars.get(id).map_or(0.0, |s| s.amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader reads snow only from an entry the viewer marked: the
    /// identity a palette starts with (1.0 where the amount goes) and a
    /// joint's matrix are no snow; the amount sits where it reads it.
    #[test]
    fn only_a_marked_entry_is_snow() {
        let identity = glam::Mat4::IDENTITY.to_cols_array_2d();
        assert_ne!(identity[3][3], aurora_render::AVATAR_SNOW_MARK);
        let e = palette_entry(0.4);
        assert_eq!(e[3][3], aurora_render::AVATAR_SNOW_MARK);
        assert_eq!(e[0][0], 0.4);
        // no snow (or "Snow on avatars" off): unmarked, left alone
        assert_ne!(palette_entry(0.0)[3][3], aurora_render::AVATAR_SNOW_MARK);
    }

    #[test]
    fn snow_gathers_in_the_open_and_melts_under_cover_or_in_rain() {
        let mut a = 0.0;
        // three minutes of heavy snow out in the open: well covered
        for _ in 0..180 {
            a = step(a, 1.0, 1.0, 0.0, false);
        }
        assert!(a > 0.9, "{a}");
        // under a roof while it snows: none gathers, it melts
        let before = a;
        a = step(a, 10.0, 1.0, 0.0, true);
        assert!(a < before);
        // in rain, gone within about a minute
        for _ in 0..70 {
            a = step(a, 1.0, 0.0, 0.5, false);
        }
        assert_eq!(a, 0.0);
    }
}
