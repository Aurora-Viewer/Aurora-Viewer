//! Optional simulator ground-sit anchor. SIT_ON_GROUND / STAND_UP follow
//! FSSelfForceSit (indra/newview/llviewermenu.cpp); no seat object is created.
//! Ownership is separate from the actual seated state: never stand up a chair
//! or a ground sit requested by the user, and never fake a simulator flag.
use crate::world::bridge::{LockMode, Preferences};
use aurora_net::{NetCommand, control};
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(250);
const RESPONSE: Duration = Duration::from_secs(4);
const RETRY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Off,
    Waiting,
    Moving,
    Seated,
    Suspended,
    Water,
    Airborne,
    Travel,
    NeedsStand,
    Anchoring,
    Anchored,
    Releasing,
    Timeout,
}
impl Status {
    pub fn short_label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Off => "movelock-short-off",
            Self::Waiting => "movelock-short-waiting",
            Self::Moving => "movelock-short-moving",
            Self::Seated | Self::Suspended | Self::Water | Self::Airborne | Self::Travel => "movelock-short-paused",
            Self::NeedsStand => "movelock-short-ao",
            Self::Anchoring => "movelock-short-anchoring",
            Self::Anchored => "movelock-short-anchored",
            Self::Releasing => "movelock-short-releasing",
            Self::Timeout => "movelock-short-timeout",
        })
    }
    pub fn label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Off => "movelock-off",
            Self::Waiting => "movelock-sit-waiting",
            Self::Moving => "movelock-sit-moving",
            Self::Seated => "movelock-sit-seated",
            Self::Suspended => "movelock-sit-suspended",
            Self::Water => "movelock-sit-water",
            Self::Airborne => "movelock-sit-airborne",
            Self::Travel => "movelock-sit-travel",
            Self::NeedsStand => "movelock-sit-needs-stand",
            Self::Anchoring => "movelock-sit-anchoring",
            Self::Anchored => "movelock-sit-anchored",
            Self::Releasing => "movelock-sit-releasing",
            Self::Timeout => "movelock-sit-timeout",
        })
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Travel {
    #[default]
    Ready,
    Water,
    Airborne,
    Other,
}
impl Travel {
    fn status(self, anchored: bool) -> Status {
        match self {
            // A confirmed ground sit is authoritative. Its changed physical
            // height or an old jump/fall animation cannot release the anchor.
            Self::Ready => Status::Waiting,
            Self::Water | Self::Airborne if anchored => Status::Waiting,
            Self::Water => Status::Water,
            Self::Airborne => Status::Airborne,
            Self::Other => Status::Travel,
        }
    }
}

/// Animation signals can outlive physical landing (or accompany a newer
/// standing signal). Use contact and completed transition duration to avoid
/// waiting forever on a stale fall/landing, without sitting during a jump.
pub fn airborne(
    raw: &[crate::world::PlayingAnimation],
    flying: bool,
    supported: bool,
    vertical_speed: f32,
    now: Instant,
    mut duration: impl FnMut(&uuid::Uuid) -> Option<Duration>,
) -> bool {
    use crate::ao::State;
    if flying {
        return false;
    }
    let primary = raw
        .iter()
        .filter(|a| !crate::ao::STAND_VARIANTS.contains(&a.id))
        .filter_map(|a| State::from_motion(a.id).filter(|s| *s != State::Typing).map(|s| (s, a)))
        .max_by_key(|(s, a)| (a.continuous_start, *s as usize));
    match primary {
        Some((State::Falling | State::Jumping, _)) => !supported || vertical_speed.abs() > 0.5,
        Some((State::PreJumping | State::Landing | State::SoftLanding, a)) => {
            !supported
                || vertical_speed.abs() > 0.5
                || now.saturating_duration_since(a.continuous_start) < duration(&a.id).unwrap_or(RESPONSE)
        }
        _ => false,
    }
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub requested: bool,
    pub region: Option<u64>,
    pub available: bool,
    pub voluntary: bool,
    /// Residual simulator/drawn motion after input release, before acquiring.
    pub moving: bool,
    pub travel: Travel,
    pub pose_ready: bool,
    /// The standing group was echoed by the simulator and fully eased in.
    pub pose_warm: bool,
    pub ground_sit: bool,
    pub on_object: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Frame {
        Frame {
            requested: true,
            region: Some(1),
            available: true,
            voluntary: false,
            moving: false,
            travel: Travel::Ready,
            pose_ready: true,
            pose_warm: true,
            ground_sit: false,
            on_object: false,
        }
    }
    fn anchored() -> (Controller, Frame, Instant) {
        let mut c = Controller::default();
        let mut f = frame();
        let start = Instant::now();
        assert_eq!(c.update(f, start), None);
        let now = start + SETTLE;
        assert_eq!(c.update(f, now), Some(control::SIT_ON_GROUND));
        f.ground_sit = true;
        assert_eq!(c.update(f, now), None);
        assert_eq!(c.status, Status::Anchored);
        (c, f, now)
    }
    #[test]
    fn movelock_sit_waits_for_rest_and_confirmation_without_keepalive() {
        let (mut c, f, now) = anchored();
        for i in 0..600 {
            assert_eq!(c.update(f, now + Duration::from_millis(i * 16)), None);
            assert_eq!(c.status, Status::Anchored);
        }
        assert_eq!(c.standing_since(true), Some(now - SETTLE));
        assert_eq!(c.standing_since(false), None);
    }
    #[test]
    fn movelock_sit_routes_missing_idle_to_bridge_through_motion_and_seat_transitions() {
        let (mut c, mut f, now) = anchored();
        let selected = Preferences {
            enabled: true,
            indicator: true,
            mode: LockMode::NativeSit,
        };
        assert_eq!(c.bridge_preferences(selected), selected);
        f.pose_ready = false;
        assert_eq!(c.update(f, now), Some(control::STAND_UP));
        assert!(c.owns_sit(), "keep the verified pose through stand-up acknowledgement");
        assert!(c.bridge_fallback());
        assert_eq!(c.bridge_preferences(selected).mode, LockMode::Bridge);
        f.ground_sit = false;
        c.update(f, now + SETTLE);
        assert!(!c.owns_sit());
        for (voluntary, moving, on_object, travel) in [
            (false, false, false, Travel::Ready),
            (true, true, false, Travel::Ready),
            (false, false, true, Travel::Ready),
            (false, false, false, Travel::Water),
        ] {
            f.voluntary = voluntary;
            f.moving = moving;
            f.on_object = on_object;
            f.travel = travel;
            assert_eq!(c.update(f, now + SETTLE * 2), None);
            let effective = c.bridge_preferences(selected);
            assert_eq!(effective.mode, LockMode::Bridge);
            assert!(effective.enabled && effective.indicator);
        }
        f = frame();
        f.pose_warm = false;
        c.update(f, now + SETTLE * 3);
        assert!(!c.bridge_fallback(), "animation warm-up uses native preparation");
        assert_eq!(c.bridge_preferences(selected), selected);
        assert_eq!(c.update(f, now + SETTLE * 4), None);
        f.pose_warm = true;
        assert_eq!(c.update(f, now + SETTLE * 5), Some(control::SIT_ON_GROUND));
        f.requested = false;
        f.pose_ready = false;
        c.update(f, now + SETTLE * 6);
        assert!(!c.bridge_fallback(), "turning Movelock off cannot enable its fallback");
        assert!(
            !c.bridge_preferences(Preferences {
                enabled: false,
                ..selected
            })
            .enabled
        );
        assert_eq!(selected.mode, LockMode::NativeSit, "never change the saved method");
    }
    #[test]
    fn movelock_sit_prepares_the_same_standing_clock_before_requesting_native_sit() {
        let mut c = Controller::default();
        let mut f = frame();
        f.pose_warm = false;
        let t = Instant::now();
        c.update(f, t);
        assert_eq!(c.standing_since(false), Some(t), "publish the standing pose before sitting");
        assert_eq!(
            c.update(f, t + Duration::from_secs(2)),
            None,
            "no sit while its animation is easing or unconfirmed"
        );
        f.pose_warm = true;
        assert_eq!(c.update(f, t + Duration::from_secs(2)), Some(control::SIT_ON_GROUND));
        assert_eq!(c.standing_since(false), Some(t), "request/echo cannot restart the AO cycle");
        f.ground_sit = true;
        c.update(f, t + Duration::from_secs(2));
        assert_eq!(c.standing_since(true), Some(t));
        f.voluntary = true;
        assert_eq!(c.update(f, t + Duration::from_secs(2)), Some(control::STAND_UP));
    }
    #[test]
    fn movelock_sit_movement_cancels_preparation_without_ever_sitting() {
        let mut c = Controller::default();
        let mut f = frame();
        f.pose_warm = false;
        let now = Instant::now();
        c.update(f, now);
        f.voluntary = true;
        assert_eq!(c.update(f, now + SETTLE), None);
        assert!(!c.owns_sit());
        assert_eq!(c.standing_since(false), None);
        assert_eq!(c.status, Status::Moving);
    }
    #[test]
    fn movelock_sit_coasting_defers_idle_and_restarts_preparation_after_a_short_tap() {
        let mut c = Controller::default();
        let mut f = frame();
        f.voluntary = true;
        f.moving = true;
        let t = Instant::now();
        c.update(f, t);
        f.voluntary = false;
        for ms in [16, 250, 500, 2000, 5000] {
            assert_eq!(c.update(f, t + Duration::from_millis(ms)), None);
            assert_eq!(c.standing_since(false), None, "input release alone cannot start idle");
            assert_eq!(c.status, Status::Moving);
        }
        f.moving = false;
        let rest = t + Duration::from_secs(6);
        f.pose_warm = false;
        c.update(f, rest);
        assert_eq!(c.standing_since(false), Some(rest));
        f.voluntary = true;
        c.update(f, rest + SETTLE);
        assert_eq!(c.standing_since(false), None);
        f.voluntary = false;
        c.update(f, rest + SETTLE * 2);
        assert_eq!(c.standing_since(false), Some(rest + SETTLE * 2));
        assert_eq!(c.update(f, rest + SETTLE * 3), None, "still wait for observer preparation");
        f.pose_warm = true;
        assert_eq!(c.update(f, rest + SETTLE * 3), Some(control::SIT_ON_GROUND));
    }
    #[test]
    fn movelock_sit_external_motion_does_not_release_an_owned_anchor() {
        let (mut c, mut f, now) = anchored();
        f.moving = true;
        assert_eq!(c.update(f, now + RESPONSE), None);
        assert_eq!(c.status, Status::Anchored);
        assert!(c.standing_since(true).is_some());
        f.voluntary = true;
        assert_eq!(
            c.update(f, now + RESPONSE),
            Some(control::STAND_UP),
            "input still releases immediately"
        );
    }
    #[test]
    fn movelock_sit_acquisition_checks_cannot_release_a_confirmed_sit() {
        for travel in [Travel::Water, Travel::Airborne] {
            let (mut c, mut f, now) = anchored();
            f.travel = travel;
            for i in 0..600 {
                assert_eq!(c.update(f, now + Duration::from_millis(i * 16)), None);
                assert_eq!(c.status, Status::Anchored);
                assert!(c.owns_sit());
            }
            f.voluntary = true;
            assert_eq!(c.update(f, now + RESPONSE * 3), Some(control::STAND_UP));
        }
    }
    #[test]
    fn movelock_sit_landed_animation_rearms_at_activation_and_after_walking() {
        use crate::{ao::State, world::PlayingAnimation};
        let now = Instant::now();
        for state in [
            State::Falling,
            State::Jumping,
            State::PreJumping,
            State::Landing,
            State::SoftLanding,
        ] {
            let raw = [PlayingAnimation::from_signal(state.motion(), 1, now, None)];
            let duration = |_: &uuid::Uuid| Some(Duration::from_millis(400));
            let rest = now + Duration::from_secs(1);
            assert!(
                airborne(&raw, false, false, -2.0, rest, duration),
                "real airborne motion must pause"
            );
            assert!(
                !airborne(&raw, false, true, 0.0, rest, duration),
                "physical landing ends stale signals"
            );
            let mut f = frame();
            f.travel = if airborne(&raw, false, true, 0.0, rest, duration) {
                Travel::Airborne
            } else {
                Travel::Ready
            };
            let mut c = Controller::default();
            assert_eq!(c.update(f, rest), None);
            assert_eq!(c.status, Status::Waiting);
            assert_eq!(c.update(f, rest + SETTLE), Some(control::SIT_ON_GROUND));
            f.ground_sit = true;
            c.update(f, rest + SETTLE);
            f.voluntary = true;
            assert_eq!(c.update(f, rest + SETTLE * 2), Some(control::STAND_UP));
            f.ground_sit = false;
            c.update(f, rest + SETTLE * 3);
            f.voluntary = false;
            c.update(f, rest + SETTLE * 4);
            assert_eq!(c.update(f, rest + SETTLE * 5), Some(control::SIT_ON_GROUND));
        }
    }
    #[test]
    fn movelock_sit_preserves_fresh_jump_and_landing_transitions() {
        use crate::{ao::State, world::PlayingAnimation};
        let now = Instant::now();
        for state in [State::PreJumping, State::Landing, State::SoftLanding] {
            let raw = [PlayingAnimation::from_signal(state.motion(), 1, now, None)];
            assert!(airborne(&raw, false, true, 0.0, now + SETTLE, |_| Some(Duration::from_millis(400))));
            assert!(airborne(&raw, false, true, 0.0, now + SETTLE, |_| None));
        }
        let old = PlayingAnimation::from_signal(State::Falling.motion(), 1, now, None);
        let standing = PlayingAnimation::from_signal(State::Standing.motion(), 1, now + SETTLE, None);
        let variant = PlayingAnimation::from_signal(crate::ao::STAND_VARIANTS[0], 1, now + SETTLE, None);
        assert!(
            airborne(&[old, variant], false, false, 0.0, now + SETTLE, |_| None),
            "a secondary standing variant cannot hide airborne motion at the jump apex"
        );
        assert!(
            !airborne(&[standing, old], false, false, 0.0, now + SETTLE, |_| None),
            "a newer standing signal supersedes an old falling signal regardless of packet order"
        );
        assert!(
            airborne(&[old], false, true, -3.0, now + SETTLE, |_| None),
            "contact must not hide a real jump/fall"
        );
        assert!(
            !airborne(&[old], true, false, 0.0, now + SETTLE, |_| None),
            "hovering remains eligible"
        );
    }
    #[test]
    fn movelock_sit_reports_the_actual_suspension_and_resumes_after_travel() {
        for (travel, status) in [
            (Travel::Water, Status::Water),
            (Travel::Airborne, Status::Airborne),
            (Travel::Other, Status::Travel),
        ] {
            let mut c = Controller::default();
            let mut f = frame();
            let now = Instant::now();
            f.travel = travel;
            assert_eq!(c.update(f, now), None);
            assert_eq!(c.status, status);
            assert_eq!(c.update(f, now + RESPONSE), None);
            f.travel = Travel::Ready;
            c.update(f, now + RESPONSE);
            assert_eq!(c.update(f, now + RESPONSE + SETTLE), Some(control::SIT_ON_GROUND));
        }
    }
    #[test]
    fn movelock_sit_releases_before_movement_and_reanchors_after_rest() {
        let (mut c, mut f, now) = anchored();
        f.voluntary = true;
        assert_eq!(c.update(f, now), Some(control::STAND_UP));
        f.ground_sit = false;
        assert_eq!(c.update(f, now + SETTLE), None);
        assert_eq!(c.update(f, now + SETTLE * 2), None);
        assert!(!c.owns_sit());
        f.voluntary = false;
        assert_eq!(c.update(f, now + SETTLE * 3), None);
        assert_eq!(c.update(f, now + SETTLE * 4), Some(control::SIT_ON_GROUND));
    }
    #[test]
    fn movelock_sit_needs_a_compatible_pose_and_suspends_for_other_travel() {
        for field in 0..5 {
            let (mut c, mut f, now) = anchored();
            match field {
                0 => f.pose_ready = false,
                1 => f.travel = Travel::Other,
                2 => f.available = false,
                3 => f.requested = false,
                _ => f.voluntary = true,
            }
            assert_eq!(c.update(f, now), Some(control::STAND_UP));
            let mut fresh = Controller::default();
            f.ground_sit = false;
            assert_eq!(fresh.update(f, now), None);
            assert_eq!(fresh.update(f, now + RESPONSE), None);
            assert!(!fresh.owns_sit());
        }
    }
    #[test]
    fn movelock_sit_never_unsits_a_manual_seat_or_a_new_region() {
        for object in [false, true] {
            let mut c = Controller::default();
            let mut f = frame();
            f.on_object = object;
            f.ground_sit = !object;
            let now = Instant::now();
            assert_eq!(c.update(f, now), None);
            f.voluntary = true;
            assert_eq!(c.update(f, now + RESPONSE), None);
            assert!(!c.owns_sit());
        }
        let (mut c, mut f, now) = anchored();
        f.region = Some(2);
        assert_eq!(c.update(f, now), None);
        assert!(!c.owns_sit());
        let (mut c, mut f, now) = anchored();
        f.on_object = true;
        assert_eq!(c.update(f, now), None);
        assert!(!c.owns_sit());
    }
    #[test]
    fn movelock_sit_user_controls_take_priority_and_generated_controls_do_not_pause() {
        let (mut c, mut f, now) = anchored();
        let generated = c.command(control::SIT_ON_GROUND);
        assert_eq!(c.observe(&generated, false, now), None);
        assert_eq!(c.update(f, now), None);
        assert_eq!(c.status, Status::Anchored);
        assert_eq!(c.observe(&NetCommand::OneShotControl(control::STAND_UP), false, now), None);
        f.ground_sit = false;
        assert_eq!(c.update(f, now + SETTLE), None);
        assert_eq!(c.update(f, now + RESPONSE), None);
        assert_eq!(c.status, Status::Seated);
        assert!(!c.owns_sit());
        f.voluntary = true;
        c.update(f, now + RESPONSE);
        f.voluntary = false;
        c.update(f, now + RESPONSE + SETTLE);
        assert_eq!(c.update(f, now + RESPONSE + SETTLE * 2), Some(control::SIT_ON_GROUND));
    }
    #[test]
    fn movelock_sit_hands_an_owned_anchor_to_a_requested_object_seat() {
        let (mut c, mut f, now) = anchored();
        let seat = NetCommand::RequestSit {
            handle: 1,
            target: uuid::Uuid::from_u128(17),
            offset: glam::Vec3::ZERO,
        };
        assert_eq!(c.observe(&seat, false, now), Some(control::STAND_UP));
        assert!(!c.owns_sit());
        let release = c.command(control::STAND_UP);
        assert_eq!(c.observe(&release, false, now), None);
        assert_eq!(c.update(f, now), None, "a pending seat request must not be stolen");
        f.on_object = true;
        f.ground_sit = false;
        assert_eq!(c.update(f, now + RESPONSE), None);
        assert_eq!(c.release(true, now + RESPONSE), None, "disable never stands up the chair");
    }
    #[test]
    fn movelock_sit_allows_autopilot_through_its_own_stand_up_but_preserves_manual_seats() {
        let (mut c, f, now) = anchored();
        assert!(!c.blocks_walk(false, f.ground_sit));
        let mut walk = crate::autopilot::AutoPilot::start(glam::Vec3::ZERO, glam::Vec3::X * 8.0, false, now);
        assert!(walk.step(glam::Vec3::ZERO, 0.0, 0.016).flags & control::AT_POS != 0);
        assert_eq!(c.release(false, now), Some(control::STAND_UP));
        assert!(!c.blocks_walk(false, true), "the pending sit echo must not cancel the walk");
        assert!(c.blocks_walk(true, true), "a chair always blocks walking");
        let manual = c.observe(&NetCommand::OneShotControl(control::SIT_ON_GROUND), false, now);
        assert_eq!(manual, None);
        assert!(c.blocks_walk(false, true), "a user ground sit remains authoritative");
    }
    #[test]
    fn movelock_sit_cancels_a_late_reply_after_movement_cancelled_the_request() {
        let mut c = Controller::default();
        let mut f = frame();
        let now = Instant::now();
        c.update(f, now);
        assert_eq!(c.update(f, now + SETTLE), Some(control::SIT_ON_GROUND));
        f.voluntary = true;
        assert_eq!(c.update(f, now + SETTLE), Some(control::STAND_UP));
        c.update(f, now + SETTLE * 2);
        assert!(!c.owns_sit());
        f.ground_sit = true;
        assert_eq!(c.update(f, now + SETTLE * 3), Some(control::STAND_UP));
        assert!(c.owns_sit());
    }
    #[test]
    fn movelock_sit_a_manual_stand_cancels_a_late_unconfirmed_sit() {
        let mut c = Controller::default();
        let mut f = frame();
        let now = Instant::now();
        c.update(f, now);
        assert_eq!(c.update(f, now + SETTLE), Some(control::SIT_ON_GROUND));
        c.observe(&NetCommand::OneShotControl(control::STAND_UP), false, now + SETTLE);
        c.update(f, now + SETTLE * 2);
        assert!(!c.owns_sit());
        f.ground_sit = true;
        assert_eq!(c.update(f, now + SETTLE * 3), Some(control::STAND_UP));
    }
    #[test]
    fn movelock_sit_retries_are_bounded_and_a_timeout_can_be_rearmed() {
        let (mut c, mut f, now) = anchored();
        f.voluntary = true;
        let mut releases = 0;
        for i in 0..600 {
            releases += usize::from(c.update(f, now + Duration::from_millis(i * 16)) == Some(control::STAND_UP));
        }
        assert_eq!(releases, 4);
        assert_eq!(c.status, Status::Timeout);
        f.ground_sit = false;
        f.requested = false;
        f.voluntary = false;
        c.update(f, now + RESPONSE * 3);
        f.requested = true;
        c.update(f, now + RESPONSE * 3);
        assert_eq!(c.update(f, now + RESPONSE * 3 + SETTLE), Some(control::SIT_ON_GROUND));
    }
}

#[derive(Clone, Copy, Default)]
enum Phase {
    #[default]
    Idle,
    Request(Instant),
    Held,
    Release {
        since: Instant,
        last: Instant,
        tries: u8,
    },
    /// Keep ownership of an unanswered request to cancel a late sit echo.
    Failed {
        owned: bool,
    },
}

#[derive(Default)]
pub struct Controller {
    phase: Phase,
    region: Option<u64>,
    requested: bool,
    pose_ready: bool,
    manual: bool,
    idle_since: Option<Instant>,
    hold_start: Option<Instant>,
    cancel_until: Option<Instant>,
    internal: Option<u32>,
    ground_sit: bool,
    pub status: Status,
}
impl Controller {
    pub fn bridge_fallback(&self) -> bool {
        self.requested && !self.pose_ready
    }
    /// Route only the physical lock; the saved choice and bridge services stay intact.
    pub fn bridge_preferences(&self, mut prefs: Preferences) -> Preferences {
        if prefs.mode == LockMode::NativeSit && self.bridge_fallback() {
            prefs.mode = LockMode::Bridge;
        }
        prefs
    }
    pub fn owns_sit(&self) -> bool {
        matches!(
            self.phase,
            Phase::Request(_) | Phase::Held | Phase::Release { .. } | Phase::Failed { owned: true }
        )
    }
    pub fn standing_since(&self, ground_sit: bool) -> Option<Instant> {
        match self.phase {
            Phase::Idle if self.status == Status::Waiting => self.idle_since,
            Phase::Request(_) => self.hold_start,
            _ if ground_sit && self.owns_sit() => self.hold_start,
            _ => None,
        }
    }
    pub fn blocks_walk(&self, on_object: bool, ground_sit: bool) -> bool {
        on_object || (ground_sit && !self.owns_sit())
    }
    pub fn command(&mut self, flags: u32) -> NetCommand {
        self.internal = Some(flags);
        NetCommand::OneShotControl(flags)
    }
    /// Called by the common send path, so every menu, keyboard and object sit
    /// has priority. Generated controls are identified only for this send.
    pub fn observe(&mut self, command: &NetCommand, on_object: bool, now: Instant) -> Option<u32> {
        if let NetCommand::OneShotControl(flags) = command
            && self.internal.take() == Some(*flags)
        {
            return None;
        }
        match command {
            NetCommand::RequestSit { .. } => {
                let release = self.owns_sit() && !on_object;
                self.phase = Phase::Idle;
                self.manual = true;
                self.idle_since = None;
                self.cancel_until = None;
                release.then_some(control::STAND_UP)
            }
            NetCommand::OneShotControl(flags) if flags & control::SIT_ON_GROUND != 0 => {
                self.phase = Phase::Idle;
                self.manual = true;
                self.idle_since = None;
                self.cancel_until = None;
                None
            }
            NetCommand::OneShotControl(flags) if flags & control::STAND_UP != 0 => {
                self.manual = true;
                self.idle_since = None;
                if self.owns_sit() && !on_object {
                    if matches!(self.phase, Phase::Request(_)) {
                        self.cancel_until = Some(now + RESPONSE);
                    }
                    self.phase = Phase::Release {
                        since: now,
                        last: now,
                        tries: 1,
                    };
                }
                None
            }
            _ => None,
        }
    }
    pub fn release(&mut self, on_object: bool, now: Instant) -> Option<u32> {
        self.idle_since = None;
        if on_object {
            self.phase = Phase::Idle;
            return None;
        }
        if !self.owns_sit() || matches!(self.phase, Phase::Release { .. }) {
            return None;
        }
        if matches!(self.phase, Phase::Request(_) | Phase::Failed { .. }) {
            self.cancel_until = Some(now + RESPONSE);
        }
        self.phase = Phase::Release {
            since: now,
            last: now,
            tries: 1,
        };
        Some(control::STAND_UP)
    }
    pub fn update(&mut self, frame: Frame, now: Instant) -> Option<u32> {
        if frame.region != self.region {
            *self = Self {
                region: frame.region,
                requested: frame.requested,
                ..Default::default()
            };
        }
        let late_sit = frame.ground_sit && !self.ground_sit;
        self.pose_ready = frame.pose_ready;
        self.ground_sit = frame.ground_sit;
        if frame.requested != self.requested {
            self.requested = frame.requested;
            self.manual = false;
            if frame.requested && !frame.ground_sit && matches!(self.phase, Phase::Failed { .. }) {
                self.phase = Phase::Idle;
            }
        }
        if frame.voluntary {
            self.manual = false;
        }
        self.status = if !frame.requested {
            Status::Off
        } else if frame.on_object || self.manual || (frame.ground_sit && !self.owns_sit()) {
            Status::Seated
        } else if frame.voluntary || (frame.moving && !self.owns_sit()) {
            Status::Moving
        } else if !frame.available {
            Status::Suspended
        } else if frame.travel.status(self.owns_sit() && frame.ground_sit) != Status::Waiting {
            frame.travel.status(self.owns_sit() && frame.ground_sit)
        } else if !frame.pose_ready {
            Status::NeedsStand
        } else {
            Status::Waiting
        };
        if frame.on_object {
            self.phase = Phase::Idle;
            self.idle_since = None;
            self.cancel_until = None;
            return None;
        }
        if late_sit && !self.owns_sit() && self.cancel_until.is_some_and(|until| now < until) {
            self.phase = Phase::Release {
                since: now,
                last: now,
                tries: 1,
            };
            self.status = Status::Releasing;
            return Some(control::STAND_UP);
        }
        let acquire = self.status == Status::Waiting;
        if !acquire && !matches!(self.phase, Phase::Release { .. } | Phase::Failed { .. }) {
            if let Some(flags) = self.release(false, now) {
                self.status = Status::Releasing;
                return Some(flags);
            }
            self.idle_since = None;
        }
        match self.phase {
            Phase::Request(_) if frame.ground_sit => {
                self.phase = Phase::Held;
                self.status = Status::Anchored;
            }
            Phase::Request(since) if now.saturating_duration_since(since) >= RESPONSE => {
                self.phase = Phase::Failed { owned: true };
                self.cancel_until = Some(now + RESPONSE);
                self.status = Status::Timeout;
                return Some(control::STAND_UP);
            }
            Phase::Request(_) => self.status = Status::Anchoring,
            Phase::Held if frame.ground_sit => self.status = Status::Anchored,
            Phase::Held => {
                // A simulator/script stand is authoritative; avoid a sit fight.
                self.phase = Phase::Idle;
                self.manual = true;
                self.idle_since = None;
                self.status = Status::Seated;
            }
            Phase::Release { since, last, tries } => {
                self.status = Status::Releasing;
                if !frame.ground_sit && now.saturating_duration_since(since) >= SETTLE {
                    self.phase = Phase::Idle;
                    self.idle_since = None;
                } else if now.saturating_duration_since(since) >= RESPONSE {
                    self.phase = Phase::Failed { owned: true };
                    self.status = Status::Timeout;
                } else if tries < 4 && now.saturating_duration_since(last) >= RETRY {
                    self.phase = Phase::Release {
                        since,
                        last: now,
                        tries: tries + 1,
                    };
                    return Some(control::STAND_UP);
                }
            }
            Phase::Failed { owned } => {
                self.status = Status::Timeout;
                if owned && late_sit {
                    return Some(control::STAND_UP);
                }
                if owned && !frame.ground_sit && !late_sit && frame.voluntary {
                    self.phase = Phase::Idle;
                }
                if !owned && frame.voluntary {
                    self.phase = Phase::Idle;
                }
            }
            Phase::Idle if acquire => {
                let since = *self.idle_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= SETTLE && frame.pose_warm {
                    self.hold_start = Some(since);
                    self.phase = Phase::Request(now);
                    self.status = Status::Anchoring;
                    return Some(control::SIT_ON_GROUND);
                }
            }
            Phase::Idle => {}
        }
        None
    }
}
