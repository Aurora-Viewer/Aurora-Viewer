//! When an AgentUpdate leaves: port of the rule of `send_agent_update`
//! (indra/newview/llviewermessage.cpp, originally LGPL 2.1).
//!
//! A change of the control flags (a key pressed or released) is sent at
//! once, then confirmed by one more update; the simulator only stops some
//! motions when told twice. Everything else (camera, body and head
//! rotation, render state) is sent when it moved past Firestorm's
//! thresholds, at most 125 times a second, and one update a second keeps
//! the agent alive.

use crate::AgentControls;
use std::time::{Duration, Instant};

/// MAX_AGENT_UPDATES_PER_SECOND = 125: "derived experimentally to avoid
/// input delays".
pub(crate) const MIN_PERIOD: Duration = Duration::from_millis(8);
/// MIN_AGENT_UPDATES_PER_SECOND = 1: the keep-alive.
pub(crate) const MAX_PERIOD: Duration = Duration::from_secs(1);

/// TRANSLATE_THRESHOLD (m).
const CAMERA_MOVE: f32 = 0.01;
/// CAMERA_AT_THRESHOLD: 0.2 × RADIANS_PER_DEGREE, Firestorm's constant being
/// π / 360; compared with the length of the difference of the at axes.
const CAMERA_TURN: f32 = 0.2 * std::f32::consts::PI / 360.0;
/// MIN_HEAD_ROT_QDOT (about 2.5°, also used for the body) and
/// MAX_HEAD_ROT_QDOT (about 0.5°): a head rotation between the two is sent
/// once it has waited long enough, the threshold growing from the first to
/// the second over THRESHOLD_GROWTH_PERIOD, for fine aiming.
const ROTATION_FAR: f32 = 0.9997;
const HEAD_CLOSE: f32 = 0.99999;
const HEAD_GROWTH: f32 = 0.5;

impl AgentControls {
    /// Whether these controls moved past one of the thresholds that make
    /// `send_agent_update` send, since `prev`. The head rotation counts
    /// here only beyond its far threshold: the band below is a matter of
    /// time, see [`AgentUpdateRule`].
    pub fn update_worthy(&self, prev: &AgentControls) -> bool {
        self.control_flags != prev.control_flags
            || self.alt_or_ctrl != prev.alt_or_ctrl
            || self.state != prev.state
            || self.flags != prev.flags
            || self.camera_center.distance(prev.camera_center) > CAMERA_MOVE
            || (self.camera_at - prev.camera_at).length() > CAMERA_TURN
            || self.body_rotation.dot(prev.body_rotation).abs() < ROTATION_FAR
            || self.head_rotation.dot(prev.head_rotation).abs() < ROTATION_FAR
    }
}

/// State of `send_agent_update` between two calls (its `static` locals).
pub(crate) struct AgentUpdateRule {
    last: AgentControls,
    last_send: Instant,
    /// The flags changed with the last update: send one more.
    follow_up: bool,
    forced: bool,
}

impl AgentUpdateRule {
    pub(crate) fn new(now: Instant) -> Self {
        AgentUpdateRule {
            last: AgentControls::default(),
            last_send: now,
            follow_up: false,
            forced: false,
        }
    }

    /// Control flags of the last update sent.
    pub(crate) fn sent_flags(&self) -> u32 {
        self.last.control_flags
    }

    /// Send at the next evaluation whatever changed (`force_send`).
    pub(crate) fn force(&mut self) {
        self.forced = true;
    }

    /// Whether `c` must be sent at `now`. A yes records it as sent.
    pub(crate) fn should_send(&mut self, c: &AgentControls, now: Instant) -> bool {
        let since = now.saturating_duration_since(self.last_send);
        // "any change in control_flags should be sent ASAP"
        let flags_changed = c.control_flags != self.last.control_flags;
        let force = self.forced || flags_changed;
        if !force && since < MIN_PERIOD {
            return false;
        }
        let send = force || since > MAX_PERIOD || self.follow_up || self.moved(c, since);
        if send {
            self.follow_up = flags_changed;
            self.forced = false;
            self.last = *c;
            self.last_send = now;
        }
        send
    }

    fn moved(&self, c: &AgentControls, since: Duration) -> bool {
        let last = &self.last;
        if c.state != last.state
            || c.flags != last.flags
            || c.camera_center.distance(last.camera_center) > CAMERA_MOVE
            || (c.camera_at - last.camera_at).length() > CAMERA_TURN
            || c.body_rotation.dot(last.body_rotation).abs() < ROTATION_FAR
        {
            return true;
        }
        let head = c.head_rotation.dot(last.head_rotation).abs();
        if head > HEAD_CLOSE {
            return false;
        }
        head < ROTATION_FAR + since.as_secs_f32() * (HEAD_CLOSE - ROTATION_FAR) / HEAD_GROWTH
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control;
    use glam::{Quat, Vec3};

    fn rule() -> (AgentUpdateRule, Instant) {
        let now = Instant::now();
        let mut rule = AgentUpdateRule::new(now);
        // the first update of a session, so that the tests start from "sent"
        assert!(rule.should_send(&AgentControls::default(), now + MAX_PERIOD * 2));
        (rule, now + MAX_PERIOD * 2)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_key_leaves_at_once_and_is_confirmed_once() {
        let (mut rule, t) = rule();
        let walking = AgentControls {
            control_flags: control::AT_POS | control::FAST_AT,
            ..Default::default()
        };
        // pressed 1 ms after an update: no wait for the rate limit
        assert!(rule.should_send(&walking, t + ms(1)));
        // the follow-up waits for the rate limit, then leaves once
        assert!(!rule.should_send(&walking, t + ms(3)));
        assert!(rule.should_send(&walking, t + ms(10)));
        assert!(!rule.should_send(&walking, t + ms(30)));
        // released 2 ms later: at once again, and confirmed
        let stopped = AgentControls::default();
        assert!(rule.should_send(&stopped, t + ms(32)));
        assert!(rule.should_send(&stopped, t + ms(41)));
        assert!(!rule.should_send(&stopped, t + ms(60)));
    }

    #[test]
    fn a_tap_shorter_than_the_rate_limit_sends_both_edges() {
        let (mut rule, t) = rule();
        let jump = AgentControls {
            control_flags: control::UP_POS,
            ..Default::default()
        };
        assert!(rule.should_send(&jump, t + ms(1)));
        assert!(rule.should_send(&AgentControls::default(), t + ms(2)));
    }

    #[test]
    fn the_rest_follows_firestorms_thresholds_at_125_a_second_at_most() {
        let (mut rule, t) = rule();
        let camera = |x: f32| AgentControls {
            camera_center: Vec3::new(x, 0.0, 0.0),
            ..Default::default()
        };
        // under the threshold: nothing, however long (below the keep-alive)
        assert!(!rule.should_send(&camera(0.009), t + ms(500)));
        // past it, but too soon after the last update
        assert!(rule.should_send(&camera(0.02), t + ms(501)));
        assert!(!rule.should_send(&camera(0.05), t + ms(505)));
        assert!(rule.should_send(&camera(0.05), t + ms(509)));
        // a camera that turns, a body that turns
        let turned = AgentControls {
            camera_center: Vec3::new(0.05, 0.0, 0.0),
            camera_at: Quat::from_rotation_z(0.3_f32.to_radians()) * Vec3::X,
            ..Default::default()
        };
        assert!(rule.should_send(&turned, t + ms(520)));
        let body = |degrees: f32| AgentControls {
            body_rotation: Quat::from_rotation_z(degrees.to_radians()),
            ..turned
        };
        assert!(!rule.should_send(&body(2.0), t + ms(540)));
        assert!(rule.should_send(&body(3.0), t + ms(560)));
        // (a moving camera at 1,000 frames a second: 125 updates, not 1,000)
        let (mut rule, t) = super::tests::rule();
        let sent = (1..=1000)
            .filter(|&i| rule.should_send(&camera(i as f32 * 0.02), t + ms(i)))
            .count();
        assert_eq!(sent, 125);
    }

    #[test]
    fn a_small_head_turn_is_sent_once_it_has_waited() {
        let (mut rule, t) = rule();
        let head = |degrees: f32| AgentControls {
            head_rotation: Quat::from_rotation_y(degrees.to_radians()),
            ..Default::default()
        };
        // 0.2°: close enough, never sent on its own
        assert!(!rule.should_send(&head(0.2), t + ms(900)));
        // 1.5°: between the two thresholds, sent after a part of the 0.5 s
        assert!(!rule.should_send(&head(1.5), t + ms(20)));
        assert!(rule.should_send(&head(1.5), t + ms(400)));
        // 4°: far, at once (after the rate limit)
        assert!(rule.should_send(&head(5.5), t + ms(409)));
    }

    #[test]
    fn keep_alive_and_forced_updates() {
        let (mut rule, t) = rule();
        let idle = AgentControls::default();
        assert!(!rule.should_send(&idle, t + ms(999)));
        assert!(rule.should_send(&idle, t + ms(1001)));
        // an arrival: sent whatever the time since the last one, once
        rule.force();
        assert!(rule.should_send(&idle, t + ms(1002)));
        assert!(!rule.should_send(&idle, t + ms(1003)));
        // the draw distance travels with the next update, it does not cause one
        let far = AgentControls { far: 512.0, ..idle };
        assert!(!rule.should_send(&far, t + ms(1500)));
    }

    #[test]
    fn update_worthy_matches_the_rule_outside_the_head_band() {
        let prev = AgentControls::default();
        assert!(!prev.update_worthy(&prev));
        assert!(
            AgentControls {
                control_flags: control::FLY,
                ..prev
            }
            .update_worthy(&prev)
        );
        assert!(
            AgentControls {
                camera_center: Vec3::splat(0.01),
                ..prev
            }
            .update_worthy(&prev)
        );
        assert!(
            !AgentControls {
                head_rotation: Quat::from_rotation_y(1.5_f32.to_radians()),
                ..prev
            }
            .update_worthy(&prev)
        );
        assert!(!AgentControls { far: 512.0, ..prev }.update_worthy(&prev));
    }
}
