//! Look-at targets (where avatars look), port of `LLHUDEffectLookAt`
//! (indra/newview/llhudeffectlookat.cpp, Linden Research and Firestorm,
//! originally LGPL 2.1): our own target with its priorities and timeouts, sent to the
//! simulator as a ViewerEffect, and the targets other avatars send us.

use glam::DVec3;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const NONE: u8 = 0;
pub const IDLE: u8 = 1;
pub const AUTO_LISTEN: u8 = 2;
pub const FREELOOK: u8 = 3;
pub const RESPOND: u8 = 4;
#[allow(dead_code)] // set by object hovering (no caller in Firestorm either)
pub const HOVER: u8 = 5;
pub const CONVERSATION: u8 = 6;
#[allow(dead_code)] // set by selection / editing (not ported yet)
pub const SELECT: u8 = 7;
pub const FOCUS: u8 = 8;
pub const MOUSELOOK: u8 = 9;
pub const CLEAR: u8 = 10;
const NUM: u8 = 11;

/// "Never" (MAX_TIMEOUT = F32_MAX / 2).
pub const MAX_TIMEOUT: f32 = f32::MAX / 2.0;

/// Attention table (attentions.xml): priority and timeout per type.
const PRIORITY: [u8; NUM as usize] = [0, 1, 3, 2, 3, 4, 0, 6, 6, 7, 8];
const TIMEOUT: [f32; NUM as usize] = [
    MAX_TIMEOUT,
    3.0,
    4.0,
    2.0,
    4.0,
    1.0,
    MAX_TIMEOUT,
    MAX_TIMEOUT,
    MAX_TIMEOUT,
    MAX_TIMEOUT,
    0.0,
];

/// Types whose target is about the avatar itself, never clamped by the
/// look-at range (FSLookAtTargetLimitDistance).
pub fn clamped(kind: u8) -> bool {
    !matches!(kind, NONE | IDLE | RESPOND | CONVERSATION | FREELOOK | AUTO_LISTEN)
}

/// A look-at target: an object and an offset in its frame (MOUSELOOK /
/// FREELOOK: a world-space direction), or a global position when the object
/// is none.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub kind: u8,
    pub object: Option<Uuid>,
    pub offset: DVec3,
}

impl Target {
    pub const NONE: Target = Target {
        kind: NONE,
        object: None,
        offset: DVec3::ZERO,
    };
}

/// Our own look-at effect.
pub struct Own {
    pub effect: Uuid,
    pub target: Target,
    kill: Option<Instant>,
    pub duration: f32,
    last_sent_offset: DVec3,
    last_send: Option<Instant>,
    pub needs_send: bool,
}

impl Own {
    fn new() -> Own {
        Own {
            effect: Uuid::new_v4(),
            target: Target::NONE,
            kill: None,
            duration: MAX_TIMEOUT,
            last_sent_offset: DVec3::ZERO,
            last_send: None,
            needs_send: false,
        }
    }

    /// LLHUDEffectLookAt::setLookAt: a target of the same or a higher
    /// priority replaces the current one; it lasts its type's timeout.
    pub fn set(&mut self, kind: u8, object: Option<Uuid>, offset: DVec3, now: Instant) -> bool {
        if kind >= NUM || PRIORITY[kind as usize] < PRIORITY[self.target.kind as usize] {
            return false;
        }
        let moved = (offset - self.last_sent_offset).length_squared() > 0.05 * 0.05
            && self.last_send.is_none_or(|t| now.duration_since(t).as_secs_f32() > 0.25);
        if kind != self.target.kind || object != self.target.object || moved {
            self.last_sent_offset = offset;
            self.duration = TIMEOUT[kind as usize];
            self.needs_send = true;
        }
        self.target = if kind == CLEAR {
            Target::NONE
        } else {
            Target { kind, object, offset }
        };
        self.kill = (self.duration < MAX_TIMEOUT).then(|| now + Duration::from_secs_f32(self.duration));
        true
    }

    /// Expire the target after its timeout (LLHUDEffectLookAt::update).
    pub fn update(&mut self, now: Instant) {
        if self.target.kind != NONE && self.kill.is_some_and(|k| now > k) {
            self.target = Target::NONE;
            self.needs_send = true;
        }
    }

    pub fn sent(&mut self, now: Instant) {
        self.needs_send = false;
        self.last_send = Some(now);
    }
}

/// Look-at targets: ours, and the last one each avatar sent (they never
/// expire locally, as in LLHUDManager).
pub struct LookAts {
    pub own: Own,
    pub remote: HashMap<Uuid, Target>,
    /// Look-at requests made by the world (heard chat), applied by the app
    /// through the privacy settings.
    pub requests: Vec<(u8, Option<Uuid>, DVec3)>,
    /// Last avatar heard in local chat (LLAgent::mLastChatterID).
    pub last_chatter: Option<Uuid>,
}

impl Default for LookAts {
    fn default() -> Self {
        LookAts {
            own: Own::new(),
            remote: HashMap::new(),
            requests: Vec::new(),
            last_chatter: None,
        }
    }
}

impl LookAts {
    /// A look-at received from the simulator (LLHUDEffectLookAt::unpackData).
    pub fn receive(&mut self, effect: Uuid, source: Uuid, target: Uuid, offset: [f64; 3], kind: u8) {
        if effect == self.own.effect {
            return;
        }
        let kind = if kind < NUM { kind } else { NONE };
        self.remote.insert(
            source,
            Target {
                kind,
                object: (!target.is_nil()).then_some(target),
                offset: DVec3::from_array(offset),
            },
        );
    }

    /// LLAgent::heardChat: half the time, look at whoever spoke before.
    pub fn heard_chat(&mut self, speaker: Uuid, coin: bool) {
        if coin && let Some(prev) = self.last_chatter {
            self.requests.push((AUTO_LISTEN, Some(prev), DVec3::ZERO));
        }
        self.last_chatter = Some(speaker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priorities_and_timeouts() {
        let t0 = Instant::now();
        let mut o = Own::new();
        assert!(o.set(FREELOOK, None, DVec3::X, t0));
        assert!(o.needs_send);
        o.sent(t0);
        // focus beats free look and holds against it
        assert!(o.set(FOCUS, None, DVec3::new(10.0, 0.0, 0.0), t0));
        assert!(!o.set(FREELOOK, None, DVec3::X, t0));
        assert_eq!(o.target.kind, FOCUS);
        // clear, then free look again; it times out after 2 s
        assert!(o.set(CLEAR, None, DVec3::ZERO, t0));
        assert_eq!(o.target.kind, NONE);
        assert!(o.set(FREELOOK, None, DVec3::X, t0));
        o.update(t0 + Duration::from_secs_f32(1.0));
        assert_eq!(o.target.kind, FREELOOK);
        o.update(t0 + Duration::from_secs_f32(2.5));
        assert_eq!(o.target.kind, NONE);
    }
}
