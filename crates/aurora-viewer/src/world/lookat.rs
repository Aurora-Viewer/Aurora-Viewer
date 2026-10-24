//! Look-at targets (where avatars look), port of `LLHUDEffectLookAt`
//! (indra/newview/llhudeffectlookat.cpp, Linden Research and Firestorm,
//! originally LGPL 2.1): our own target with its priorities and timeouts, sent to the
//! simulator as a ViewerEffect, and the targets other avatars send us.

use glam::{DVec3, Quat, Vec3};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const NONE: u8 = 0;
pub const IDLE: u8 = 1;
pub const AUTO_LISTEN: u8 = 2;
pub const FREELOOK: u8 = 3;
pub const RESPOND: u8 = 4;
/// Set by the cursor over the world: an Aurora choice, Firestorm defines
/// the type without a caller (see `cursor_look`).
pub const HOVER: u8 = 5;
pub const CONVERSATION: u8 = 6;
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

/// Look-at preferences, those of Firestorm with its defaults: Privacy ›
/// LookAt (panel_preferences_privacy.xml) and Firestorm › Avatar
/// (panel_preferences_firestorm.xml).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// DebugLookAt: crosshairs where the avatars look.
    pub show_targets: bool,
    /// DebugLookAtShowNames: name over each crosshair (0 none, 1 complete
    /// name, 2 display name, 3 legacy name, 4 username).
    pub show_names: u8,
    /// DebugLookAtHideOwn: not our own crosshair.
    pub hide_own: bool,
    /// ExodusLookAtLines: a line from the crosshair to its avatar.
    pub lines: bool,
    /// PrivateLookAtTarget: the others get a dull target instead of ours.
    pub private: bool,
    /// PrivateLocalLookAtTarget: no target at all, our head stays straight.
    pub private_local: bool,
    /// PrivatePointAtTarget: suppress selection hints and the editing arm.
    pub private_point_at: bool,
    /// FSLookAtTargetLimitDistance / FSLookAtTargetMaxDistance (m): a
    /// farther target is brought back to this distance from our head.
    pub limit_distance: bool,
    pub max_distance: f32,
    /// YawFromMousePosition / PitchFromMousePosition: how far (degrees,
    /// across the whole window) the head follows the cursor.
    pub head_yaw: f32,
    pub head_pitch: f32,
    /// FSStaticEyes: an animation that stops the random eye movements.
    pub static_eyes: bool,
    /// How the targets are drawn: Aurora's halo, or Firestorm's wire
    /// crosshair (Preferences › Aurora).
    pub style: Style,
    /// Where the head looks while the cursor moves (Preferences › Aurora).
    pub aim: Aim,
}

/// Where our head looks while the cursor moves (`cursor_look`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Aim {
    /// The very point under the cursor (LOOKAT_TARGET_HOVER), Aurora's own.
    #[default]
    Point,
    /// Firestorm: only toward the cursor's side (FREELOOK).
    Direction,
}

/// The look of the look-at targets (`ui::lookat_targets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Style {
    /// A dot with a soft halo, theme colours, the name in a pill.
    #[default]
    Aurora,
    /// LLHUDEffectLookAt::render: wire crosshair, attention colours.
    Legacy,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_targets: false,
            show_names: 1,
            hide_own: true,
            lines: false,
            private: false,
            private_local: false,
            private_point_at: false,
            limit_distance: false,
            max_distance: 1.0,
            head_yaw: 90.0,
            head_pitch: 90.0,
            static_eyes: false,
            style: Style::Aurora,
            aim: Aim::Point,
        }
    }
}

/// FSLookAtTargetMaxDistance slider (m).
pub const MAX_DISTANCE: std::ops::RangeInclusive<f32> = 1.0..=32.0;
/// Yaw / PitchFromMousePosition sliders (degrees).
pub const HEAD_RANGE: std::ops::RangeInclusive<f32> = 0.0..=120.0;
/// FSStaticEyesUUID: the priority 2 animation that holds the eyes still.
pub const STATIC_EYES: Uuid = uuid::uuid!("592a9fb4-5e02-77bb-e1e5-da4d17d27336");

/// What the simulator was asked about `STATIC_EYES` (update_static_eyes in
/// llstartup.cpp and its setting's listener in llviewercontrol.cpp). The
/// animation is the simulator's: a teleport or a crossing lands in a
/// simulator that does not play it, so every arrival asks again
/// (gAgent.addRegionChangedCallback(update_static_eyes)), local teleports
/// too, which may unseat the avatar and reset its animations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StaticEyes {
    sent: Option<bool>,
}

impl StaticEyes {
    /// The request to send for the setting `want` (true: start, false:
    /// stop), if any. Nothing is sent to stop what was never started.
    pub fn request(&mut self, want: bool) -> Option<bool> {
        match self.sent {
            Some(sent) if sent == want => None,
            None if !want => {
                self.sent = Some(false);
                None
            }
            _ => {
                self.sent = Some(want);
                Some(want)
            }
        }
    }

    /// An arrival (login, teleport, crossing): the simulator plays nothing
    /// of ours any more.
    pub fn on_arrival(&mut self) {
        self.sent = None;
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Settings {
        let clamp = |v: f32, r: &std::ops::RangeInclusive<f32>, d: f32| if v.is_finite() { v.clamp(*r.start(), *r.end()) } else { d };
        self.max_distance = clamp(self.max_distance, &MAX_DISTANCE, 1.0);
        self.head_yaw = clamp(self.head_yaw, &HEAD_RANGE, 90.0);
        self.head_pitch = clamp(self.head_pitch, &HEAD_RANGE, 90.0);
        self.show_names = self.show_names.min(4);
        self
    }

    /// The former single "Look-at tracking" switch and its range: off meant
    /// nothing shown nor sent, a range under `former_unlimited` a limit.
    pub fn from_former(on: Option<bool>, range: Option<f32>, former_unlimited: f32) -> Settings {
        let mut s = Settings::default();
        if let Some(on) = on {
            s.private = !on;
            s.private_local = !on;
        }
        if let Some(r) = range.filter(|r| r.is_finite()) {
            s.limit_distance = r < former_unlimited;
            if s.limit_distance {
                s.max_distance = r.clamp(*MAX_DISTANCE.start(), *MAX_DISTANCE.end());
            }
        }
        s
    }
}

/// What the others get of our target (LLHUDEffectLookAt::packData): nothing
/// when both privacy settings are on; with PrivateLookAtTarget alone, a
/// dull target 2.5 m ahead of us ("auto listen"), unless ours already is an
/// auto-listen one, which goes out as it is.
pub fn outgoing(target: Target, me: Uuid, private: bool, private_local: bool) -> Option<Target> {
    if private && private_local {
        None
    } else if private && target.kind != AUTO_LISTEN {
        Some(Target {
            kind: AUTO_LISTEN,
            object: Some(me),
            offset: DVec3::new(2.5, 0.0, 0.0),
        })
    } else {
        Some(target)
    }
}

/// A global target brought back within `range` of the head
/// (LLHUDEffectLookAt::setLookAt, FIRE-23524).
pub fn within(head: DVec3, target: DVec3, range: f64) -> DVec3 {
    let d = target - head;
    let len = d.length();
    if len > range && len > 0.0 {
        head + d * (range / len)
    } else {
        target
    }
}

/// Where the head looks when the cursor moves (LLAgentCamera::updateLookAt,
/// third person): the camera's view turned by the cursor's offset from the
/// middle of the window (`x` from -0.5 on the left to 0.5 on the right, `y`
/// from -0.5 at the bottom to 0.5 at the top), `yaw` / `pitch` degrees
/// across the whole window (YawFromMousePosition, PitchFromMousePosition).
/// LLCoordFrame::yaw then pitch turn the camera's own frame: about its up
/// axis, then about its left axis. A direction from the head (FREELOOK).
pub fn cursor_direction(forward: Vec3, x: f32, y: f32, yaw: f32, pitch: f32) -> Vec3 {
    let left = Vec3::Z.cross(forward).normalize_or(Vec3::Y);
    let up = forward.cross(left).normalize_or(Vec3::Z);
    Quat::from_axis_angle(up, -x * yaw.to_radians()) * Quat::from_axis_angle(left, -y * pitch.to_radians()) * forward
}

/// What the head looks at while the cursor moves (third person). Firestorm
/// only turns the head toward the cursor's direction (FREELOOK,
/// `cursor_direction`): the crosshair stands 1 m from the head, never on
/// what the cursor points at, and LOOKAT_TARGET_HOVER has no caller. Aurora
/// looks at the very point under the cursor when there is one (the hover
/// pick of the cursor icon): an object with the point in its frame, or a
/// point of the ground (`hover`); over the sky, a window or ourselves, the
/// direction of Firestorm. Its priority and its 1 s timeout are those of
/// the attentions table: focus and selection still win, free look and
/// idle give way. `Aim::Direction` keeps Firestorm's free look everywhere.
pub fn cursor_look(aim: Aim, hover: Option<(Option<Uuid>, DVec3)>, me: Uuid, free: Vec3) -> (u8, Option<Uuid>, DVec3) {
    match (aim, hover) {
        (Aim::Point, Some((object, offset))) => (HOVER, object, offset),
        _ => (FREELOOK, Some(me), free.as_dvec3()),
    }
}

/// A point of the world in the frame of what it lies on (`at`, `rot`): the
/// offset a look-at on that object carries (calcTargetPosition turns it
/// back with the object).
pub fn local_offset(point: Vec3, at: Vec3, rot: Quat) -> DVec3 {
    (rot.inverse() * (point - at)).as_dvec3()
}

/// Colour of a type in Firestorm's look (BOY_ATTS / GIRL_ATTS, the same
/// for both), in 0..1.
pub fn legacy_color(kind: u8) -> [f32; 3] {
    const COLORS: [[f32; 3]; NUM as usize] = [
        [0.3, 0.3, 0.3],
        [0.5, 0.5, 0.5],
        [0.5, 0.5, 0.5],
        [0.5, 0.5, 0.9],
        [0.0, 0.0, 0.0],
        [0.5, 0.9, 0.5],
        [0.1, 0.1, 0.5],
        [0.9, 0.5, 0.5],
        [0.9, 0.5, 0.9],
        [0.9, 0.9, 0.5],
        [1.0, 1.0, 1.0],
    ];
    COLORS[kind.min(NUM - 1) as usize]
}

/// Types whose crosshair gets a line to its avatar (ExodusLookAtLines).
pub fn has_line(kind: u8) -> bool {
    !matches!(kind, NONE | IDLE | AUTO_LISTEN)
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
    pub(super) fn new() -> Own {
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
        self.set_with(kind, object, offset, now, &PRIORITY, &TIMEOUT)
    }

    pub(super) fn set_point_at(&mut self, kind: u8, object: Option<Uuid>, offset: DVec3, now: Instant) -> bool {
        let max = f32::MAX / 4.0;
        self.set_with(kind, object, offset, now, &[0, 1, 2, 3], &[max, max, max, 0.0])
    }

    fn set_with(&mut self, kind: u8, object: Option<Uuid>, offset: DVec3, now: Instant, priority: &[u8], timeout: &[f32]) -> bool {
        if !offset.is_finite() || kind as usize >= priority.len() || priority[kind as usize] < priority[self.target.kind as usize] {
            return false;
        }
        let moved = (offset - self.last_sent_offset).length_squared() > 0.05 * 0.05
            && self.last_send.is_none_or(|t| now.duration_since(t).as_secs_f32() > 0.25);
        if kind != self.target.kind || object != self.target.object || moved {
            self.last_sent_offset = offset;
            self.duration = timeout[kind as usize];
            self.needs_send = true;
        }
        self.target = if kind as usize == priority.len() - 1 {
            Target::NONE
        } else {
            Target { kind, object, offset }
        };
        self.kill = (self.duration < timeout[0]).then(|| now + Duration::from_secs_f32(self.duration));
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

/// Look-at targets: ours, and the last one each avatar sent. A received one
/// has no timeout (LLHUDEffectLookAt::unpackData never sets mKillTime: its
/// sender's own timeout sends "none"); it lasts as long as its avatar is in
/// the world (`forget`).
pub struct LookAts {
    pub own: Own,
    pub remote: HashMap<Uuid, Target>,
    /// Look-at requests made by the world (heard chat), applied by the app
    /// through the privacy settings.
    pub requests: Vec<(u8, Option<Uuid>, DVec3)>,
    /// Last avatar heard in local chat (LLAgent::mLastChatterID).
    pub last_chatter: Option<Uuid>,
    /// Our avatar looks straight ahead, head and eyes: no target at all
    /// (PrivateLocalLookAtTarget), set by the app each frame (`gaze`).
    pub own_ahead: bool,
}

impl Default for LookAts {
    fn default() -> Self {
        LookAts {
            own: Own::new(),
            remote: HashMap::new(),
            requests: Vec::new(),
            last_chatter: None,
            own_ahead: false,
        }
    }
}

/// How an avatar's head and eyes take its look-at target this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gaze {
    /// No look-at known for it: the motions keep what they had.
    Unknown,
    /// Toward the target.
    At(Target),
    /// "None" (clearLookAtTarget): the head stops following (HEAD_ROT
    /// stops), the eyes keep the last point (LookAtPoint stays set).
    Cleared,
    /// Straight ahead, head and eyes.
    Ahead,
}

/// The gaze of an avatar from its look-at target (`None`: none received).
/// `ahead`: our own avatar with PrivateLocalLookAtTarget. The Legacy Viewer
/// only turns our target into "none" there, so the head stops and the eyes
/// stay on the last point until a restart; Aurora brings both back straight
/// ahead, as the setting says (the head eases back, the random eye
/// movements go on). Deliberate difference, the human's report of
/// 2026-10-24.
pub fn gaze(target: Option<Target>, ahead: bool) -> Gaze {
    match target {
        _ if ahead => Gaze::Ahead,
        None => Gaze::Unknown,
        Some(t) if t.kind == NONE => Gaze::Cleared,
        Some(t) => Gaze::At(t),
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

    /// LLHUDEffectLookAt::update once objects left the world (`exists`
    /// tells which are still in it): the look-at of an avatar that is gone
    /// dies with it (markDead: killed, out of view, logged off, its region
    /// closed), and a target object that is gone clears the look-at
    /// (clearLookAtTarget: "none", the head stops following it), ours too,
    /// without telling the others. Run where objects are removed, not each
    /// frame.
    pub fn forget(&mut self, exists: impl Fn(&Uuid) -> bool) {
        self.remote.retain(|source, _| exists(source));
        for t in self.remote.values_mut().chain(std::iter::once(&mut self.own.target)) {
            if t.object.is_some_and(|o| !exists(&o)) {
                *t = Target::NONE;
            }
        }
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
    fn static_eyes_are_asked_again_after_every_arrival() {
        let mut eyes = StaticEyes::default();
        // the setting off: nothing to stop
        assert_eq!(eyes.request(false), None);
        // on: started once
        assert_eq!(eyes.request(true), Some(true));
        assert_eq!(eyes.request(true), None);
        // a teleport (seated or not) or a crossing: the new simulator does
        // not play it, so it is asked again while the setting stays on
        eyes.on_arrival();
        assert_eq!(eyes.request(true), Some(true));
        assert_eq!(eyes.request(true), None);
        // turned off: stopped once; an arrival with it off sends nothing
        assert_eq!(eyes.request(false), Some(false));
        eyes.on_arrival();
        assert_eq!(eyes.request(false), None);
    }

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

    /// Turning "no look-at target, even for me" on: our target is cleared
    /// at once and the others are told (LOOKAT_TARGET_CLEAR), even over a
    /// focus that never times out, and our head and eyes look ahead instead
    /// of staying on the last point. Off again: the cursor is followed.
    #[test]
    fn no_local_target_clears_ours_and_looks_ahead() {
        let t0 = Instant::now();
        let me = Uuid::from_u128(1);
        let mut l = LookAts::default();
        let cursor = Target {
            kind: FOCUS,
            object: None,
            offset: DVec3::new(130.0, 120.0, 25.0),
        };
        assert!(l.own.set(cursor.kind, cursor.object, cursor.offset, t0));
        l.own.sent(t0);
        assert_eq!(gaze(Some(l.own.target), l.own_ahead), Gaze::At(cursor));
        // the setting turns on: "none" alone cannot replace a focus
        assert!(!l.own.set(NONE, Some(me), DVec3::ZERO, t0));
        assert!(l.own.set(CLEAR, None, DVec3::ZERO, t0));
        l.own_ahead = true;
        assert_eq!(l.own.target, Target::NONE);
        assert!(l.own.needs_send);
        assert_eq!(outgoing(l.own.target, me, false, true), Some(Target::NONE));
        // then "none" holds frame after frame, head and eyes ahead
        assert!(l.own.set(NONE, Some(me), DVec3::ZERO, t0));
        assert_eq!(gaze(Some(l.own.target), l.own_ahead), Gaze::Ahead);
        // off again: the cursor's target is taken and followed
        l.own_ahead = false;
        assert!(l.own.set(FREELOOK, Some(me), DVec3::X, t0));
        assert!(matches!(gaze(Some(l.own.target), l.own_ahead), Gaze::At(t) if t.kind == FREELOOK));
        // another avatar's "none" keeps the Legacy Viewer's way
        assert_eq!(gaze(Some(Target::NONE), false), Gaze::Cleared);
        assert_eq!(gaze(None, false), Gaze::Unknown);
    }

    #[test]
    fn a_look_at_lasts_as_long_as_its_avatar_and_its_target() {
        let (gone, stays, prim, me) = (Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(7), Uuid::from_u128(1));
        let mut l = LookAts::default();
        let at = |object| Target {
            kind: FOCUS,
            object,
            offset: DVec3::X,
        };
        l.receive(Uuid::from_u128(20), gone, me, [0.0; 3], CONVERSATION);
        l.receive(Uuid::from_u128(30), stays, prim, [1.0, 0.0, 0.0], FOCUS);
        let point = Uuid::from_u128(4);
        l.receive(Uuid::from_u128(40), point, Uuid::nil(), [1.0, 0.0, 0.0], FOCUS);
        l.own.set(FOCUS, Some(prim), DVec3::X, Instant::now());
        // a received look-at has no timeout of its own
        assert_eq!(l.remote.len(), 3);
        // its source gone, it goes (markDead); toward an object gone, it
        // becomes "none" (clearLookAtTarget), ours too; a global point stays
        l.forget(|id| ![gone, prim].contains(id));
        assert!(!l.remote.contains_key(&gone));
        assert_eq!(l.remote.get(&stays), Some(&Target::NONE));
        assert_eq!(l.remote.get(&point), Some(&at(None)));
        assert_eq!(l.own.target, Target::NONE);
        // nothing gone: nothing changes
        l.receive(Uuid::from_u128(30), stays, me, [0.0; 3], FOCUS);
        l.forget(|_| true);
        assert_eq!(l.remote.get(&stays).map(|t| t.object), Some(Some(me)));
    }

    #[test]
    fn what_the_others_get_follows_the_privacy_settings() {
        let me = Uuid::from_u128(1);
        let focus = Target {
            kind: FOCUS,
            object: None,
            offset: DVec3::new(130.0, 120.0, 25.0),
        };
        assert_eq!(outgoing(focus, me, false, false), Some(focus));
        // the local switch alone: the target is already "none", sent as it is
        assert_eq!(outgoing(Target::NONE, me, false, true), Some(Target::NONE));
        let dull = Target {
            kind: AUTO_LISTEN,
            object: Some(me),
            offset: DVec3::new(2.5, 0.0, 0.0),
        };
        assert_eq!(outgoing(focus, me, true, false), Some(dull));
        // an auto-listen target is not replaced (packData)
        let listen = Target {
            kind: AUTO_LISTEN,
            object: Some(Uuid::from_u128(2)),
            offset: DVec3::ZERO,
        };
        assert_eq!(outgoing(listen, me, true, false), Some(listen));
        assert_eq!(outgoing(focus, me, true, true), None);
    }

    #[test]
    fn far_targets_are_brought_back_to_the_range() {
        let head = DVec3::new(10.0, 10.0, 21.0);
        let far = DVec3::new(20.0, 10.0, 21.0);
        assert!((within(head, far, 2.0) - DVec3::new(12.0, 10.0, 21.0)).length() < 1e-9);
        let near = DVec3::new(10.5, 10.0, 21.0);
        assert_eq!(within(head, near, 2.0), near);
        assert_eq!(within(head, head, 2.0), head);
    }

    #[test]
    fn the_former_switch_keeps_what_the_user_had() {
        let off = Settings::from_former(Some(false), Some(10.0), 64.0);
        assert!(off.private && off.private_local);
        assert!(off.limit_distance && off.max_distance == 10.0);
        let on = Settings::from_former(Some(true), Some(64.0), 64.0);
        assert!(!on.private && !on.private_local && !on.limit_distance);
        assert_eq!(Settings::from_former(Some(true), Some(50.0), 64.0).max_distance, 32.0);
        assert_eq!(Settings::from_former(None, None, 64.0), Settings::default());
    }

    #[test]
    fn the_head_turns_toward_the_cursor() {
        let close = |a: Vec3, b: Vec3| (a - b).length() < 1e-5;
        let h = std::f32::consts::FRAC_1_SQRT_2;
        // middle of the window: the camera's view
        assert!(close(cursor_direction(Vec3::X, 0.0, 0.0, 90.0, 90.0), Vec3::X));
        // left edge, 90° across the window: 45° to the left (+Y)
        assert!(close(cursor_direction(Vec3::X, -0.5, 0.0, 90.0, 90.0), Vec3::new(h, h, 0.0)));
        assert!(close(cursor_direction(Vec3::X, 0.5, 0.0, 90.0, 90.0), Vec3::new(h, -h, 0.0)));
        // top edge: 45° up; bottom: down
        assert!(close(cursor_direction(Vec3::X, 0.0, 0.5, 90.0, 90.0), Vec3::new(h, 0.0, h)));
        assert!(close(cursor_direction(Vec3::X, 0.0, -0.5, 90.0, 90.0), Vec3::new(h, 0.0, -h)));
        // 0°: the head does not follow
        assert!(close(cursor_direction(Vec3::X, 0.4, -0.3, 0.0, 0.0), Vec3::X));
        // a camera looking 30° down: the yaw turns about the camera's own up
        // axis, so the direction stays 45° away from the view
        let down = Vec3::new(30f32.to_radians().cos(), 0.0, -30f32.to_radians().sin());
        let d = cursor_direction(down, -0.5, 0.0, 90.0, 0.0);
        assert!((d.dot(down) - h).abs() < 1e-5 && d.y > 0.0);
        // and the pitch lifts it back to the horizon
        assert!(close(cursor_direction(down, 0.0, 0.5, 0.0, 60.0), Vec3::X));
    }

    #[test]
    fn the_cursor_looks_at_what_it_is_over() {
        let me = Uuid::from_u128(1);
        let prim = Uuid::from_u128(7);
        let free = Vec3::new(0.0, 1.0, 0.0);
        // over an object: hover, on the object
        let on = (Some(prim), DVec3::new(0.2, 0.0, 0.5));
        assert_eq!(cursor_look(Aim::Point, Some(on), me, free), (HOVER, Some(prim), on.1));
        // over the ground: hover, a global point
        let ground = (None, DVec3::new(256.0 + 30.0, 40.0, 21.5));
        assert_eq!(cursor_look(Aim::Point, Some(ground), me, free), (HOVER, None, ground.1));
        // over the sky: Firestorm's free look
        assert_eq!(cursor_look(Aim::Point, None, me, free), (FREELOOK, Some(me), free.as_dvec3()));
        // a hover target is a point in the world: the distance limit applies
        assert!(clamped(HOVER) && !clamped(FREELOOK));
    }

    #[test]
    fn firestorm_aim_only_turns_toward_the_cursor() {
        let me = Uuid::from_u128(1);
        let free = Vec3::new(0.0, 1.0, 0.0);
        let on = (Some(Uuid::from_u128(7)), DVec3::new(0.2, 0.0, 0.5));
        // over an object as over the sky: the direction, never the point
        for hover in [Some(on), Some((None, DVec3::new(30.0, 40.0, 21.5))), None] {
            assert_eq!(cursor_look(Aim::Direction, hover, me, free), (FREELOOK, Some(me), free.as_dvec3()));
        }
        // Aurora's point by default, also in a file saved before the setting
        assert_eq!(Settings::default().aim, Aim::Point);
        let old: Settings = serde_json::from_str(r#"{"show_targets":true}"#).expect("former look-at settings");
        assert_eq!(old.aim, Aim::Point);
    }

    #[test]
    fn a_point_is_kept_in_the_frame_of_its_object() {
        let rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let at = Vec3::new(10.0, 10.0, 20.0);
        let point = Vec3::new(10.0, 11.0, 20.5);
        let off = local_offset(point, at, rot);
        assert!((off - DVec3::new(1.0, 0.0, 0.5)).length() < 1e-5);
        // turned back with the object, it is the point again
        assert!((at + rot * off.as_vec3() - point).length() < 1e-5);
    }

    #[test]
    fn hover_gives_way_to_focus_and_selection_and_beats_free_look() {
        let t0 = Instant::now();
        let x = DVec3::X;
        let mut o = Own::new();
        assert!(o.set(FREELOOK, None, x, t0));
        assert!(o.set(HOVER, None, x, t0));
        assert!(!o.set(FREELOOK, None, x, t0));
        assert!(!o.set(IDLE, None, x, t0));
        assert_eq!(o.target.kind, HOVER);
        // a hover lasts 1 s, then the head goes back to idle / free look
        o.update(t0 + Duration::from_secs_f32(1.2));
        assert_eq!(o.target.kind, NONE);
        for strong in [FOCUS, SELECT] {
            let mut o = Own::new();
            assert!(o.set(strong, None, x, t0));
            assert!(!o.set(HOVER, None, x, t0));
            assert_eq!(o.target.kind, strong);
        }
    }

    #[test]
    fn firestorm_defaults() {
        let s = Settings::default();
        assert!(!s.show_targets && s.show_names == 1 && s.hide_own && !s.lines);
        assert!(!s.private && !s.private_local && !s.limit_distance);
        assert_eq!((s.max_distance, s.head_yaw, s.head_pitch, s.static_eyes), (1.0, 90.0, 90.0, false));
        // Aurora's look, Firestorm's kept as the legacy one
        assert_eq!(s.style, Style::Aurora);
        let legacy = Settings {
            style: Style::Legacy,
            ..Settings::default()
        };
        let saved = serde_json::to_value(legacy).expect("serialize");
        assert_eq!(saved["style"], "Legacy");
        let read: Settings = serde_json::from_value(saved).expect("read");
        assert_eq!(read.style, Style::Legacy);
        // a file without it: Aurora's
        let read: Settings = serde_json::from_value(serde_json::json!({ "lines": true })).expect("read");
        assert_eq!(read.style, Style::Aurora);
    }
}
