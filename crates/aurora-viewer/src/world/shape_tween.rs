//! Progressive shape changes: when another avatar's shape changes (a new
//! AvatarAppearance that is not its first), its visual parameters glide
//! from the old values to the new ones over 0.65 s on a "bouncy" curve
//! that overshoots a little and settles, instead of snapping. Port of
//! LLVOAvatar::processAvatarAppearance (setAnimationTarget),
//! startAppearanceAnimation, idleUpdateAppearanceAnimation, calcMorphAmount
//! and calc_bouncy_animation (indra/newview/llvoavatar.cpp,
//! APPEARANCE_MORPH_TIME) and LLVisualParam::animate
//! (indra/llcharacter/llvisualparam.cpp), originally LGPL 2.1.
//!
//! The Legacy Viewer animates every avatar but its own; Aurora only those
//! within [`RANGE`] of the agent (a farther one snaps, as its change is
//! hardly seen there): each step rebuilds the avatar's skeleton and the
//! morphed geometry of its visible system body parts, about 0.1 ms of the
//! main thread per avatar and frame during the glide (21 avatars at once
//! in AURORA_DEMO_SHAPE_MORPH: +1.5 ms on the seconds they glide, dev
//! build, worst frames unchanged).

use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// APPEARANCE_MORPH_TIME (llvoavatar.cpp).
pub const MORPH_SECS: f32 = 0.65;
/// Avatars farther than this from the agent (m) take a new shape at once.
pub const RANGE: f32 = 32.0;

/// calc_bouncy_animation (llvoavatar.cpp): 0 at 0 and 1 at 1; on the way
/// it first steps back a little (-12 % near 0.2), then past the target
/// (+12 % near 0.6), then settles.
pub fn bouncy(x: f32) -> f32 {
    -(x * std::f32::consts::PI * 2.5 - std::f32::consts::FRAC_PI_2).cos() * (0.4 - x * 0.1) + x * 1.3
}

/// The parameters `t` seconds into a change from `from` to `to`, kept in
/// the parameters' range (LLVisualParam::setWeight clamps to it).
pub fn blend(from: &[u8], to: &[u8], t: f32, out: &mut Vec<u8>) {
    let f = bouncy((t / MORPH_SECS).clamp(0.0, 1.0));
    out.clear();
    out.extend(to.iter().enumerate().map(|(i, &b)| {
        let a = f32::from(from.get(i).copied().unwrap_or(b));
        (a + (f32::from(b) - a) * f).round().clamp(0.0, 255.0) as u8
    }));
}

struct Tween {
    from: Vec<u8>,
    to: Vec<u8>,
    started: Instant,
    shown: Vec<u8>,
}

/// The avatars whose shape is gliding, and a counter per avatar that moves
/// at each step (added to its appearance generation, so the scene rebuilds
/// its skeleton and morphs).
#[derive(Default)]
pub struct ShapeTweens {
    tweens: HashMap<Uuid, Tween>,
    steps: HashMap<Uuid, u64>,
}

impl ShapeTweens {
    /// A change of `avatar`'s shape from `from` (what it shows now) to `to`.
    pub fn start(&mut self, avatar: Uuid, from: Vec<u8>, to: Vec<u8>, now: Instant) {
        if from == to || from.len() != to.len() {
            self.tweens.remove(&avatar);
            return;
        }
        self.tweens.insert(
            avatar,
            Tween {
                shown: from.clone(),
                from,
                to,
                started: now,
            },
        );
        *self.steps.entry(avatar).or_default() += 1;
    }

    /// What `avatar` shows while it glides.
    pub fn shown(&self, avatar: &Uuid) -> Option<&[u8]> {
        self.tweens.get(avatar).map(|t| t.shown.as_slice())
    }

    pub fn generation(&self, avatar: &Uuid) -> u64 {
        self.steps.get(avatar).copied().unwrap_or(0)
    }

    pub fn active(&self) -> bool {
        !self.tweens.is_empty()
    }

    /// One frame: every gliding avatar moves on; the finished ones show
    /// their new shape.
    pub fn step(&mut self, now: Instant) {
        let steps = &mut self.steps;
        let mut next = Vec::new();
        self.tweens.retain(|id, t| {
            let secs = now.saturating_duration_since(t.started).as_secs_f32();
            if secs >= MORPH_SECS {
                *steps.entry(*id).or_default() += 1;
                return false;
            }
            blend(&t.from, &t.to, secs, &mut next);
            if next != t.shown {
                std::mem::swap(&mut t.shown, &mut next);
                *steps.entry(*id).or_default() += 1;
            }
            true
        });
    }

    /// A change that lands at once: no glide left to show.
    pub fn cancel(&mut self, avatar: &Uuid) {
        self.tweens.remove(avatar);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_bouncy_curve_starts_overshoots_and_settles_as_the_legacy_one() {
        assert!(bouncy(0.0).abs() < 1e-6);
        assert!((bouncy(1.0) - 1.0).abs() < 1e-5);
        let peak = (0..=100).map(|i| bouncy(i as f32 / 100.0)).fold(0.0f32, f32::max);
        assert!(peak > 1.05 && peak < 1.2, "{peak}");
    }

    #[test]
    fn a_change_glides_then_lands_on_the_new_shape() {
        let id = Uuid::from_u128(1);
        let t0 = Instant::now();
        let mut tw = ShapeTweens::default();
        tw.start(id, vec![0, 100, 255], vec![200, 100, 0], t0);
        assert_eq!(tw.shown(&id), Some(&[0u8, 100, 255][..]));
        let g0 = tw.generation(&id);
        tw.step(t0 + Duration::from_millis(200));
        let mid = tw.shown(&id).expect("gliding").to_vec();
        assert!(mid[0] > 0 && mid[0] < 255 && mid[1] == 100 && mid[2] < 255);
        assert!(tw.generation(&id) > g0);
        // the overshoot stays in the parameters' range
        for ms in (0..650).step_by(10) {
            tw.step(t0 + Duration::from_millis(ms));
        }
        let g1 = tw.generation(&id);
        tw.step(t0 + Duration::from_millis(700));
        assert_eq!(tw.shown(&id), None, "done: the appearance's own values");
        assert!(tw.generation(&id) > g1, "the last step is read again");
        assert!(!tw.active());
        // no change, or a different parameter count: no glide
        tw.start(id, vec![1, 2], vec![1, 2], t0);
        tw.start(id, vec![1, 2], vec![1, 2, 3], t0);
        assert!(!tw.active());
    }
}
