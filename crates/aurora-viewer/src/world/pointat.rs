//! LLHUDEffectPointAt (indra/newview/llhudeffectpointat.cpp, originally
//! LGPL 2.1). Editing is a local procedural motion driven by this effect;
//! no AgentAnimation request is sent for ANIM_AGENT_EDITING.

use super::lookat::{Own, Target};
use glam::DVec3;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

pub const SELECT: u8 = 1;
pub const GRAB: u8 = 2;
pub const CLEAR: u8 = 3;

pub struct PointAts {
    pub own: Own,
    pub remote: HashMap<Uuid, Target>,
}

impl Default for PointAts {
    fn default() -> Self {
        Self {
            own: Own::new(),
            remote: HashMap::new(),
        }
    }
}

impl PointAts {
    pub fn set(&mut self, target: Option<Target>, now: Instant) {
        if let Some(t) = target {
            self.own.set_point_at(t.kind, t.object, t.offset, now);
        } else if self.own.target.kind != 0 {
            self.own.set_point_at(CLEAR, None, DVec3::ZERO, now);
        }
    }

    pub fn receive(&mut self, effect: Uuid, source: Uuid, target: Uuid, offset: [f64; 3], kind: u8) {
        if effect == self.own.effect || !offset.iter().all(|v| v.is_finite()) || kind > CLEAR {
            return;
        }
        if kind == 0 || kind == CLEAR {
            self.remote.remove(&source);
        } else {
            self.remote.insert(
                source,
                Target {
                    kind,
                    object: (!target.is_nil()).then_some(target),
                    offset: DVec3::from_array(offset),
                },
            );
        }
    }

    pub fn forget(&mut self, exists: impl Fn(&Uuid) -> bool) {
        self.remote
            .retain(|source, target| exists(source) && target.object.is_none_or(|id| exists(&id)));
        if self.own.target.object.is_some_and(|id| !exists(&id)) {
            self.set(None, Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn selection_targets_throttle_offsets_and_clear_once() {
        let now = Instant::now();
        let mut points = PointAts::default();
        let target = Target {
            kind: SELECT,
            object: Some(Uuid::new_v4()),
            offset: DVec3::ZERO,
        };
        points.set(Some(target), now);
        assert!(points.own.needs_send);
        points.own.sent(now);
        let moved = Target {
            offset: DVec3::X,
            ..target
        };
        points.set(Some(moved), now + Duration::from_millis(100));
        assert!(!points.own.needs_send);
        assert_eq!(points.own.target, moved, "local motion follows unthrottled");
        points.set(Some(moved), now + Duration::from_millis(260));
        assert!(points.own.needs_send);
        points.own.sent(now);
        points.set(None, now);
        assert_eq!(points.own.target, Target::NONE);
        assert!(points.own.needs_send);
        points.own.sent(now);
        points.set(None, now);
        assert!(!points.own.needs_send);
    }

    #[test]
    fn remote_targets_clear_and_leave_with_their_objects() {
        let mut p = PointAts::default();
        let source = Uuid::new_v4();
        let object = Uuid::new_v4();
        let effect = Uuid::new_v4();
        p.receive(effect, source, object, [0.0; 3], GRAB);
        assert_eq!(p.remote[&source].kind, GRAB);
        p.forget(|id| *id == source);
        assert!(p.remote.is_empty());
        p.receive(effect, source, Uuid::nil(), [1.0; 3], SELECT);
        p.receive(effect, source, Uuid::nil(), [0.0; 3], CLEAR);
        assert!(p.remote.is_empty());
        p.receive(effect, source, object, [f64::NAN; 3], SELECT);
        assert!(p.remote.is_empty());
    }
}
