//! Selection and give beams: the line from an avatar's hand to what it
//! edits, grabs or gives (LLHUDEffectSpiral of type LL_HUD_EFFECT_BEAM,
//! indra/newview/llhudeffecttrail.cpp; LLVOAvatarSelf::idleUpdateTractorBeam,
//! indra/newview/llvoavatarself.cpp; LLGiveInventory and
//! LLToolDragAndDrop, which fire one at the avatar or object given to;
//! Linden Research and the Legacy Viewer, originally LGPL 2.1).
//!
//! Each beam lives for the duration its sender gives (0.7 s for the
//! selection beam, sent again every 0.25 s while editing; 1 s for a give)
//! and fades out after. Ours are drawn here at once, the others' as their
//! ViewerEffect messages come. `ui::beams` draws them.

use glam::DVec3;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// LLHUDObject::addHUDEffect: a beam's duration when its sender gives none.
pub const SELECTION_DURATION: f32 = 0.7;
/// lggBeamMaps::setUpAndGetDuration without a beam shape: how often our
/// selection beam is sent again while we edit (s).
pub const RESEND: f32 = 0.25;
/// LL_HUD_DUR_SHORT: the beam fired at what we give or drop into.
pub const GIVE_DURATION: f32 = 1.0;
/// A sender's duration is kept to this (s): a beam only lives while its
/// sender keeps sending it. The Legacy Viewer keeps any duration; a longer
/// one only leaves a stale beam behind.
const MAX_DURATION: f32 = 30.0;
/// How long a beam takes to fade out once its time is up (s).
pub const FADE_OUT: f32 = 0.3;

/// The look of the beams (Preferences › Aurora).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Style {
    /// A glowing thread from the hand, the halo of the look-at targets on
    /// the point aimed at and the avatar's name in a pill.
    #[default]
    Aurora,
    /// LLViewerPartSourceBeam: a stream of particles from the hand.
    Legacy,
}

/// Beam preferences, the Legacy Viewer's with its defaults.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// ShowSelectionBeam: draw the beams, ours and the others' (they are
    /// sent either way).
    pub show: bool,
    /// EffectColor (Preferences › Colors): the colour of our beams, as the
    /// others see them too. White by default (the skin's EffectColor).
    pub color: [u8; 4],
    pub style: Style,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show: true,
            color: [255; 4],
            style: Style::Aurora,
        }
    }
}

/// One beam. The target is an object, else a global position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beam {
    pub source: Uuid,
    pub target: Option<Uuid>,
    pub position: DVec3,
    pub color: [u8; 4],
    /// When it appeared: it fades in from there.
    pub born: Instant,
    /// When its time is up: it fades out from there.
    pub until: Instant,
}

impl Beam {
    /// 0..1: how far it has faded in, then out.
    pub fn alpha(&self, now: Instant, fade_in: f32) -> f32 {
        let fade_in = (now.saturating_duration_since(self.born).as_secs_f32() / fade_in).min(1.0);
        let out = 1.0 - (now.saturating_duration_since(self.until).as_secs_f32() / FADE_OUT).min(1.0);
        fade_in * out
    }
}

/// Our selection beam while we edit or grab: one effect, sent again every
/// `RESEND`; a new one when editing starts again (a dead mBeam is
/// recreated).
#[derive(Debug, Default)]
pub struct Own {
    effect: Option<Uuid>,
    last_sent: Option<Instant>,
}

/// What to send of our own beam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Send {
    pub effect: Uuid,
    pub target: Uuid,
    pub position: DVec3,
    pub color: [u8; 4],
    pub duration: f32,
}

#[derive(Debug, Default)]
pub struct Beams {
    /// Every beam shown, ours included, by effect id.
    pub shown: HashMap<Uuid, Beam>,
    pub own: Own,
}

fn secs(s: f32) -> Duration {
    Duration::from_secs_f32(s.clamp(0.0, MAX_DURATION))
}

impl Beams {
    /// LLHUDManager::processViewerEffect then LLHUDEffectSpiral::unpackData:
    /// a new effect id starts a beam, a known one is sent again and lives on.
    /// The effect dies when its source or its target object is not known
    /// here (`known`), or when its time is zero; our own echo is ignored.
    #[allow(clippy::too_many_arguments)]
    pub fn receive(
        &mut self,
        effect: Uuid,
        source: Uuid,
        target: Uuid,
        position: [f64; 3],
        color: [u8; 4],
        duration: f32,
        now: Instant,
        known: impl Fn(&Uuid) -> bool,
    ) {
        if self.own.effect == Some(effect) {
            return;
        }
        let target = (!target.is_nil()).then_some(target);
        let aimed = target.is_some() || position.iter().any(|v| *v != 0.0);
        if !duration.is_finite() || duration <= 0.0 || !known(&source) || target.is_some_and(|t| !known(&t)) || !aimed {
            self.shown.remove(&effect);
            return;
        }
        let born = self.shown.get(&effect).map_or(now, |b| b.born);
        self.shown.insert(
            effect,
            Beam {
                source,
                target,
                position: DVec3::from_array(position),
                color,
                born,
                until: now + secs(duration),
            },
        );
    }

    /// LLVOAvatarSelf::idleUpdateTractorBeam, once a frame: `aim` is where
    /// our beam points (a global position), none when we neither edit nor
    /// grab. Our beam is shown at once and follows `aim` every frame; what
    /// to send comes back every `RESEND`.
    pub fn update_own(&mut self, me: Uuid, aim: Option<DVec3>, color: [u8; 4], now: Instant) -> Option<Send> {
        let Some(position) = aim else {
            // mBeam = NULL: the last one fades out on its own
            self.own = Own::default();
            return None;
        };
        let effect = *self.own.effect.get_or_insert_with(Uuid::new_v4);
        let beam = self.shown.entry(effect).or_insert(Beam {
            source: me,
            target: None,
            position,
            color,
            born: now,
            until: now,
        });
        beam.position = position;
        beam.color = color;
        // triggerLocal every frame
        beam.until = now + secs(SELECTION_DURATION);
        let due = self
            .own
            .last_sent
            .is_none_or(|t| now.saturating_duration_since(t).as_secs_f32() > RESEND);
        due.then(|| {
            self.own.last_sent = Some(now);
            Send {
                effect,
                target: Uuid::nil(),
                position,
                color,
                duration: SELECTION_DURATION,
            }
        })
    }

    /// The beam fired at the avatar or object we give to or drop into, or
    /// at the global position where we rez (LLGiveInventory::
    /// doGiveInventoryItem, LLToolDragAndDrop::dropInventory / dropObject):
    /// shown here and sent once.
    pub fn give(&mut self, me: Uuid, target: Uuid, position: DVec3, color: [u8; 4], now: Instant) -> Send {
        let effect = Uuid::new_v4();
        self.shown.insert(
            effect,
            Beam {
                source: me,
                target: (!target.is_nil()).then_some(target),
                position,
                color,
                born: now,
                until: now + secs(GIVE_DURATION),
            },
        );
        Send {
            effect,
            target,
            position,
            color,
            duration: GIVE_DURATION,
        }
    }

    /// Drop the beams faded out, and those whose source or target left.
    pub fn forget(&mut self, now: Instant, exists: impl Fn(&Uuid) -> bool) {
        let gone = Duration::from_secs_f32(FADE_OUT);
        self.shown
            .retain(|_, b| now.saturating_duration_since(b.until) < gone && exists(&b.source) && b.target.is_none_or(|t| exists(&t)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn a_received_beam_lives_while_sent_again_then_fades_and_goes() {
        let now = Instant::now();
        let mut beams = Beams::default();
        let known = |u: &Uuid| *u == id(1) || *u == id(2);
        beams.receive(id(9), id(1), id(2), [0.0; 3], [1, 2, 3, 255], 0.7, now, known);
        let born = beams.shown[&id(9)].born;
        assert_eq!(beams.shown[&id(9)].target, Some(id(2)));
        // sent again: the same beam, born when it first came
        let later = now + Duration::from_millis(250);
        beams.receive(id(9), id(1), id(2), [0.0; 3], [1, 2, 3, 255], 0.7, later, known);
        assert_eq!(beams.shown[&id(9)].born, born);
        assert_eq!(beams.shown[&id(9)].until, later + secs(0.7));
        let b = beams.shown[&id(9)];
        assert!(b.alpha(later, 0.25) > 0.99);
        // its time up, it fades out, then goes
        let fading = later + Duration::from_millis(850);
        assert!(b.alpha(fading, 0.25) < 1.0 && b.alpha(fading, 0.25) > 0.0);
        beams.forget(fading, known);
        assert_eq!(beams.shown.len(), 1);
        beams.forget(later + Duration::from_secs(2), known);
        assert!(beams.shown.is_empty());
    }

    #[test]
    fn a_beam_from_or_to_an_unknown_object_or_of_no_time_dies() {
        let now = Instant::now();
        let mut beams = Beams::default();
        let known = |u: &Uuid| *u == id(1);
        // target unknown
        beams.receive(id(9), id(1), id(2), [0.0; 3], [255; 4], 0.7, now, known);
        assert!(beams.shown.is_empty());
        // source unknown
        beams.receive(id(9), id(3), Uuid::nil(), [10.0, 10.0, 20.0], [255; 4], 0.7, now, known);
        assert!(beams.shown.is_empty());
        // aimed at a position
        beams.receive(id(9), id(1), Uuid::nil(), [10.0, 10.0, 20.0], [255; 4], 0.7, now, known);
        assert_eq!(beams.shown[&id(9)].position, DVec3::new(10.0, 10.0, 20.0));
        // no time: gone at once
        beams.receive(id(9), id(1), Uuid::nil(), [10.0, 10.0, 20.0], [255; 4], 0.0, now, known);
        assert!(beams.shown.is_empty());
        // aimed nowhere
        beams.receive(id(9), id(1), Uuid::nil(), [0.0; 3], [255; 4], 0.7, now, known);
        assert!(beams.shown.is_empty());
        // a huge duration is kept short
        beams.receive(id(9), id(1), Uuid::nil(), [1.0; 3], [255; 4], f32::MAX, now, known);
        assert_eq!(beams.shown[&id(9)].until, now + secs(MAX_DURATION));
        // a beam whose source leaves goes with it
        beams.forget(now, |_| false);
        assert!(beams.shown.is_empty());
    }

    #[test]
    fn our_beam_shows_at_once_is_sent_every_quarter_second_and_ignores_its_echo() {
        let now = Instant::now();
        let me = id(1);
        let mut beams = Beams::default();
        let aim = DVec3::new(128.0, 128.0, 25.0);
        let first = beams.update_own(me, Some(aim), [9, 9, 9, 255], now).expect("sent at once");
        assert_eq!(first.target, Uuid::nil());
        assert_eq!(first.position, aim);
        assert_eq!(first.duration, SELECTION_DURATION);
        assert_eq!(beams.shown[&first.effect].source, me);
        let soon = now + Duration::from_millis(100);
        assert!(beams.update_own(me, Some(aim + DVec3::X), [9, 9, 9, 255], soon).is_none());
        assert_eq!(beams.shown[&first.effect].position, aim + DVec3::X, "follows every frame");
        let again = beams
            .update_own(me, Some(aim), [9, 9, 9, 255], now + Duration::from_millis(260))
            .expect("sent again");
        assert_eq!(again.effect, first.effect);
        // our echo does not replace it
        beams.receive(first.effect, id(5), Uuid::nil(), [1.0; 3], [255; 4], 0.7, now, |_| true);
        assert_eq!(beams.shown[&first.effect].source, me);
        // editing stops: nothing sent, the beam fades out; a new one next time
        assert!(beams.update_own(me, None, [9, 9, 9, 255], now).is_none());
        let next = beams.update_own(me, Some(aim), [9, 9, 9, 255], now).expect("a new beam");
        assert_ne!(next.effect, first.effect);
    }

    #[test]
    fn a_give_beam_is_shown_and_sent_once_for_a_second() {
        let now = Instant::now();
        let mut beams = Beams::default();
        let send = beams.give(id(1), id(7), DVec3::ZERO, [255; 4], now);
        assert_eq!((send.target, send.duration), (id(7), GIVE_DURATION));
        assert_eq!(beams.shown[&send.effect].until, now + secs(GIVE_DURATION));
        assert_eq!(beams.shown[&send.effect].target, Some(id(7)));
        // a rez: at the point dropped on
        let at = DVec3::new(300.0, 20.0, 22.0);
        let rez = beams.give(id(1), Uuid::nil(), at, [255; 4], now);
        assert_eq!((rez.target, rez.position), (Uuid::nil(), at));
        assert_eq!(beams.shown[&rez.effect].target, None);
    }
}
