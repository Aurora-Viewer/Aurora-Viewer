//! When an AgentUpdate leaves: port of the rule of `send_agent_update`
//! (indra/newview/llviewermessage.cpp, originally LGPL 2.1).
//!
//! A change of the control flags (a key pressed or released) is sent at
//! once, then confirmed by one more update; the simulator only stops some
//! motions when told twice. Everything else (camera, body and head
//! rotation, render state) is sent when it moved past Firestorm's
//! thresholds, at most 125 times a second, and one update a second keeps
//! the agent alive. The draw distance it asks for (`Far`) follows the same
//! function's interest list steps, [`FarStep`].

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

/// TeleportArrivalDelay (settings.xml, 2 s): how long TELEPORT_ARRIVING
/// lasts after AgentMovementComplete (llviewerdisplay.cpp).
const TELEPORT_ARRIVAL_DELAY: Duration = Duration::from_secs(2);
/// The interest list never asks for less than this on arrival.
const ARRIVAL_FAR_MIN: f32 = 50.0;
/// After an arrival, `Far` grows by a tenth of the draw distance...
const FAR_GROWTH: f32 = 0.1;
/// ...at most once a second.
const FAR_GROWTH_PERIOD: Duration = Duration::from_secs(1);

/// The draw distance AgentUpdate asks for (`Far`), port of the interest
/// list steps of `send_agent_update` (its `last_draw_disatance_step`). The
/// simulator connects the agent to the neighbouring regions within this
/// distance (EnableSimulator) and sends the objects within it. While
/// logging in and during TELEPORT_ARRIVING it asks for half the draw
/// distance (50 m at least), "to prioritize closer area": the simulator
/// does not sort the attachments by distance. Then it grows by a tenth of
/// the draw distance a second up to it; a smaller draw distance applies at
/// once. (FSDrawDistanceVRAMOptimization, off by default, is not ported.)
pub(crate) struct FarStep {
    step: f32,
    raised_at: Instant,
    /// TELEPORT_ARRIVING (or logging in) until then.
    arriving_until: Option<Instant>,
    /// A TeleportFinish came: its AgentMovementComplete starts arriving.
    teleported: bool,
}

impl FarStep {
    pub(crate) fn new(now: Instant) -> Self {
        FarStep {
            // (`static F32 last_draw_disatance_step = 1024`)
            step: 1024.0,
            raised_at: now,
            arriving_until: None,
            teleported: false,
        }
    }

    /// TeleportFinish: the agent is on its way to another region.
    pub(crate) fn teleported(&mut self) {
        self.teleported = true;
    }

    /// AgentMovementComplete. `login`: the first region of the session
    /// (LLStartUp's state before STATE_MISC); a teleport's arrival then
    /// lasts TeleportArrivalDelay; a region crossing changes nothing.
    pub(crate) fn movement_complete(&mut self, login: bool, now: Instant) {
        let hold = if login {
            Some(Duration::ZERO)
        } else {
            std::mem::take(&mut self.teleported).then_some(TELEPORT_ARRIVAL_DELAY)
        };
        if let Some(hold) = hold {
            self.arriving_until = Some(now + hold);
        }
    }

    /// `Far` of an update leaving at `now`, for the viewer's draw distance.
    pub(crate) fn far(&mut self, draw_distance: f32, now: Instant) -> f32 {
        if let Some(until) = self.arriving_until {
            if now >= until {
                self.arriving_until = None;
            }
            self.step = (draw_distance / 2.0).max(ARRIVAL_FAR_MIN).min(draw_distance);
        } else if self.step < draw_distance {
            if now.saturating_duration_since(self.raised_at) > FAR_GROWTH_PERIOD {
                self.raised_at = now;
                self.step = (self.step + draw_distance * FAR_GROWTH).min(draw_distance);
            }
        } else {
            self.step = draw_distance;
        }
        self.step
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
    fn far_is_the_draw_distance_after_half_of_it_on_arrival() {
        let t = Instant::now();
        // #2810: the session asked for 128 m whatever the draw distance, so
        // the simulator connected only the neighbours within 128 m
        let mut far = FarStep::new(t);
        far.movement_complete(true, t);
        assert_eq!(far.far(290.0, t), 145.0);
        // then a tenth of the draw distance a second, up to it
        assert_eq!(far.far(290.0, t + ms(500)), 145.0);
        assert_eq!(far.far(290.0, t + ms(1100)), 174.0);
        assert_eq!(far.far(290.0, t + ms(1500)), 174.0);
        let mut at = t + ms(1100);
        for _ in 0..4 {
            at += ms(1001);
            far.far(290.0, at);
        }
        assert_eq!(far.far(290.0, at), 290.0);
        // a smaller draw distance at once, a larger one step by step again
        assert_eq!(far.far(64.0, at + ms(10)), 64.0);
        assert_eq!(far.far(512.0, at + ms(20)), 64.0);
        assert!((far.far(512.0, at + ms(1100)) - 115.2).abs() < 1e-3);
        // never under 50 m on arrival, nor over a short draw distance
        let mut short = FarStep::new(t);
        short.movement_complete(true, t);
        assert_eq!(short.far(64.0, t), 50.0);
        let mut shorter = FarStep::new(t);
        shorter.movement_complete(true, t);
        assert_eq!(shorter.far(32.0, t), 32.0);
    }

    #[test]
    fn a_teleport_arrival_holds_half_the_draw_distance_for_two_seconds() {
        let t = Instant::now();
        let mut far = FarStep::new(t);
        far.movement_complete(true, t);
        assert_eq!(far.far(256.0, t), 128.0);
        let mut at = t;
        for _ in 0..6 {
            at += ms(1001);
            far.far(256.0, at);
        }
        assert_eq!(far.far(256.0, at), 256.0);
        // a region crossing keeps the draw distance
        far.movement_complete(false, at);
        assert_eq!(far.far(256.0, at), 256.0);
        // a teleport: half of it until TeleportArrivalDelay has passed
        far.teleported();
        let arrived = at + ms(5000);
        far.movement_complete(false, arrived);
        assert_eq!(far.far(256.0, arrived), 128.0);
        assert_eq!(far.far(256.0, arrived + ms(1900)), 128.0);
        assert_eq!(far.far(256.0, arrived + ms(2000)), 128.0);
        assert!((far.far(256.0, arrived + ms(2100)) - 153.6).abs() < 1e-3);
        // the teleport was used up: a later crossing changes nothing
        far.movement_complete(false, arrived + ms(3000));
        assert!((far.far(256.0, arrived + ms(3000)) - 153.6).abs() < 1e-3);
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
