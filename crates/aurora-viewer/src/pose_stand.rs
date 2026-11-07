//! The pose stand (toolbar `pose_stand`): our avatar held in one of the
//! Legacy Viewer's stand poses while attachments are adjusted. Port of
//! FSFloaterPoseStand and FSPose (indra/newview/fsfloaterposestand.cpp,
//! fspose.cpp, app_settings/posestand.xml; settings FSPoseStandLock and
//! FSPoseStandLastSelectedPose in app_settings/settings.xml).
//!
//! The poses are grid animations started and stopped through the
//! simulator (AgentAnimation, `LLAgent::sendAnimationRequest`), which plays
//! them back to every viewer, ours included. This module holds the rules;
//! `app/pose_stand.rs` carries out the [`Effect`]s in their order.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a closed stand waits for a ground-sit that was asked and not yet
/// confirmed, to stand up again.
const RELEASE_WAIT: Duration = Duration::from_secs(10);

/// FSPoseStandLock (off) and FSPoseStandLastSelectedPose (empty), both
/// saved across sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Ground-sit while the stand is open, so that the avatar stays put.
    pub lock: bool,
    /// The pose chosen last, as the Legacy Viewer keeps it: the UUID text.
    pub last_pose: String,
}

/// One pose of posestand.xml: its animation and the key of its name.
pub struct Pose {
    pub id: Uuid,
    key: &'static str,
}

impl Pose {
    /// Its name in the interface's language (the PS_* strings).
    pub fn label(&self) -> &'static str {
        aurora_i18n::text(self.key)
    }
}

const fn pose(id: u128, key: &'static str) -> Pose {
    Pose {
        id: Uuid::from_u128(id),
        key,
    }
}

/// app_settings/posestand.xml, in its file order (the UUIDs' order, as an
/// LLSD map iterates).
pub const POSES: [Pose; 11] = [
    pose(0x0b4ee516_5dfd_ef4b_4ab6_300bb44cb045, "pose-stand-arms-down-legs-together"),
    pose(0x2029d88f_4efc_d72d_18d1_274caf9e9bc0, "pose-stand-arms-downward-legs-together"),
    pose(0x23f3163c_183b_dac0_0cdd_7811bcc3b5f1, "pose-stand-arms-forward-legs-together"),
    pose(0x2cfeed25_04f9_caff_f3d2_776cddea5771, "pose-stand-arms-upward-legs-apart"),
    pose(0x3481ccd5_cc6c_65ca_c466_dc9df1bf3944, "pose-stand-t-pose"),
    pose(0x6e83f685_2f53_278f_6758_b529327cce09, "pose-stand-arms-upward-legs-together"),
    pose(0xb23c52ef_0b15_11f2_852d_8fbd33417847, "pose-stand-arms-down-sitting"),
    pose(0xb2f312aa_2c1e_27b8_50fe_a0fa4ff3df4e, "pose-stand-arms-straight-legs-apart"),
    pose(0xc17677b8_9fb2_8d6e_bbbe_219edce64ef4, "pose-stand-arms-straight-sitting"),
    pose(0xcff5559e_c915_0b5e_d587_f789b5cc299a, "pose-stand-arms-forward-legs-apart"),
    pose(0xe14fee5f_3735_a236_2e26_94f92f366d81, "pose-stand-arms-downward-legs-apart"),
];

/// The pose of a first opening: LLComboBox::add selects the first item
/// added, posestand.xml's first key, and sortByName keeps that selection.
pub const DEFAULT_POSE: Uuid = POSES[0].id;

/// The list as the window shows it: sorted by name in the interface's
/// language (FSFloaterPoseStand::loadPoses, LLComboBox::sortByName).
pub fn sorted() -> Vec<&'static Pose> {
    let mut list: Vec<&Pose> = POSES.iter().collect();
    list.sort_by_cached_key(|p| p.label().to_lowercase());
    list
}

/// The pose an opening selects: the one saved last when it is still in
/// the list (setSelectedByValue), else the default.
pub fn initial(last: &str) -> Uuid {
    Uuid::parse_str(last)
        .ok()
        .filter(|id| POSES.iter().any(|p| p.id == *id))
        .unwrap_or(DEFAULT_POSE)
}

/// What the stand asks of the viewer, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// UseAO off for the time of the stand.
    PauseAo,
    /// UseAO back on.
    ResumeAo,
    /// LLAgent::sitDown: AGENT_CONTROL_SIT_ON_GROUND (RLVa may refuse it).
    SitOnGround,
    /// LLAgent::standUp: AGENT_CONTROL_STAND_UP (RLVa may refuse it).
    StandUp,
    /// LLAgent::stopCurrentAnimations(true): every animation stopped, the
    /// scripts keep their animation permissions.
    StopAnimations,
    /// AgentAnimation start / stop of one pose.
    Start(Uuid),
    Stop(Uuid),
}

/// What the opening needs to know of the avatar.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts {
    /// UseAO is on.
    pub ao_on: bool,
    /// We sit, on an object or on the ground.
    pub sitting: bool,
    /// RLVa @sit is in force (RLV_BHVR_SIT).
    pub rlv_sit: bool,
}

/// The stand while its window is open.
#[derive(Debug, Default)]
pub struct Stand {
    /// The window is open (its opening has run, its closing not yet).
    pub open: bool,
    /// FSPose::mCurrentPose.
    current: Option<Uuid>,
    /// FSFloaterPoseStand::mPoseStandLock: we sat down for the lock.
    locked: bool,
    /// mAOPaused: the opening turned the AO off.
    ao_paused: bool,
    /// The window closed before the ground-sit it asked for was confirmed:
    /// stand up as soon as we sit, until this time.
    release_until: Option<Instant>,
    /// The server seated us since the lock asked (seen by `late_release`).
    sit_confirmed: bool,
}

impl Stand {
    /// The lock is on: we sat down for it and have not let go.
    #[cfg(test)]
    fn holds_lock(&self) -> bool {
        self.open && self.locked
    }

    /// Each frame while the stand is open: whether the ground lock was lost
    /// from outside. The lock sat us down and the server confirmed it; if we
    /// no longer sit (the shape editor's stand-up, a script unsit, the user
    /// standing, a teleport), the lock is dropped and this returns true, so
    /// that the checkbox and the saved setting follow what the avatar does.
    /// The Legacy Viewer stands the avatar up the same way and leaves its
    /// checkbox ticked; following the real state is an Aurora clarity fix.
    /// The stand's own moves never trigger it: `set_lock` clears the
    /// confirmation, and `close` releases the lock itself. Nothing sits the
    /// avatar again.
    pub fn lock_lost(&mut self, sitting: bool) -> bool {
        if !self.open || !self.locked {
            return false;
        }
        if sitting {
            self.sit_confirmed = true;
            return false;
        }
        if self.sit_confirmed {
            self.locked = false;
            self.sit_confirmed = false;
            return true;
        }
        false
    }

    /// Each frame after a closing: the ground-sit asked by the lock may be
    /// confirmed after the window closed (`close` found us still standing).
    /// Stand up then, so that the lock does not outlive the stand.
    pub fn late_release(&mut self, sitting: bool, now: Instant) -> Vec<Effect> {
        if self.locked && sitting {
            self.sit_confirmed = true;
        }
        let Some(until) = self.release_until else {
            return Vec::new();
        };
        if self.open || now >= until {
            self.release_until = None;
            return Vec::new();
        }
        if sitting {
            self.release_until = None;
            return vec![Effect::StandUp];
        }
        Vec::new()
    }

    /// The pose playing.
    pub fn current(&self) -> Option<Uuid> {
        self.current
    }

    /// FSFloaterPoseStand::onOpen: the AO paused, the lock taken if asked
    /// and we stand without @sit, every animation stopped, then the pose.
    pub fn open(&mut self, facts: Facts, settings: &Settings) -> Vec<Effect> {
        let mut out = Vec::new();
        self.open = true;
        self.release_until = None;
        // set at each opening: the Legacy Viewer keeps an old mAOPaused when
        // the AO was turned on again meanwhile, and a later closing could
        // then turn on an AO that was off when the stand opened
        self.ao_paused = facts.ao_on;
        if facts.ao_on {
            out.push(Effect::PauseAo);
        }
        if settings.lock && !facts.sitting && !facts.rlv_sit {
            out.extend(self.set_lock(true));
        }
        out.push(Effect::StopAnimations);
        out.extend(self.select(initial(&settings.last_pose)));
        out
    }

    /// FSPose::setPose: the pose before stopped, this one started (again,
    /// when it is the same).
    pub fn select(&mut self, pose: Uuid) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(before) = self.current {
            out.push(Effect::Stop(before));
        }
        self.current = Some(pose);
        out.push(Effect::Start(pose));
        out
    }

    /// FSFloaterPoseStand::setLock: sit down or stand up. The lock is
    /// noted as asked even when RLVa refuses the move, as there.
    fn set_lock(&mut self, on: bool) -> Vec<Effect> {
        self.locked = on;
        self.sit_confirmed = false;
        vec![if on { Effect::SitOnGround } else { Effect::StandUp }]
    }

    /// The lock switched while the stand is open (handleSetPoseStandLock):
    /// sit down or stand up, then the pose sent again (onCommitCombo).
    pub fn lock_changed(&mut self, on: bool) -> Vec<Effect> {
        let mut out = self.set_lock(on);
        if let Some(pose) = self.current {
            out.extend(self.select(pose));
        }
        out
    }

    /// FSFloaterPoseStand::onClose: stand up when the lock sat us down, the
    /// pose and every animation stopped, the AO back on if we paused it.
    pub fn close(&mut self, sitting: bool, ao_on: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        self.open = false;
        if self.locked && sitting {
            out.extend(self.set_lock(false));
        } else if self.locked && !self.sit_confirmed {
            // the ground-sit is still on its way: release it when it lands
            self.locked = false;
            self.release_until = Some(Instant::now() + RELEASE_WAIT);
        }
        if let Some(pose) = self.current.take() {
            out.push(Effect::Stop(pose));
        }
        out.push(Effect::StopAnimations);
        if self.ao_paused && !ao_on {
            out.push(Effect::ResumeAo);
            self.ao_paused = false;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outside_stand_up_drops_the_lock_but_the_stands_own_release_does_not() {
        let lock = Settings {
            lock: true,
            last_pose: String::new(),
        };
        let mut stand = Stand::default();
        stand.open(Facts::default(), &lock);
        // the ground-sit is not confirmed yet: standing is no loss
        assert!(!stand.lock_lost(false));
        assert!(stand.holds_lock());
        assert!(!stand.lock_lost(true));
        // the editor, a script or the user stands us up
        assert!(stand.lock_lost(false));
        assert!(!stand.holds_lock());
        assert!(!stand.lock_lost(false), "reported once");

        // the stand's own release: unticking sits no more, nothing to report
        let mut stand = Stand::default();
        stand.open(Facts::default(), &lock);
        stand.lock_lost(true);
        stand.lock_changed(false);
        assert!(!stand.lock_lost(false));
        // locking again waits for its own confirmation
        stand.lock_changed(true);
        assert!(!stand.lock_lost(false));
        assert!(stand.holds_lock());

        // closing releases it without a report
        stand.lock_lost(true);
        stand.close(true, false);
        assert!(!stand.lock_lost(false));
    }

    #[test]
    fn settings_default_like_the_legacy_viewer() {
        let s = Settings::default();
        assert!(!s.lock, "FSPoseStandLock is off");
        assert!(s.last_pose.is_empty(), "FSPoseStandLastSelectedPose is empty");
        let read: Settings = serde_json::from_str("{}").expect("empty settings");
        assert_eq!(read, s);
    }

    #[test]
    fn the_list_is_posestand_xml() {
        assert_eq!(POSES.len(), 11);
        // the map's order: the UUID texts sorted
        let ids: Vec<String> = POSES.iter().map(|p| p.id.to_string()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids);
        assert_eq!(DEFAULT_POSE.to_string(), "0b4ee516-5dfd-ef4b-4ab6-300bb44cb045");
        assert_eq!(sorted().len(), POSES.len());
    }

    #[test]
    fn the_first_opening_takes_the_default_and_a_saved_pose_comes_back() {
        assert_eq!(initial(""), DEFAULT_POSE);
        assert_eq!(initial("not a uuid"), DEFAULT_POSE);
        // a UUID that is no pose of the list
        assert_eq!(initial("2408fe9e-df1d-1d7d-f4ff-1384fa7b350f"), DEFAULT_POSE);
        let t_pose = POSES[4].id;
        assert_eq!(initial(&t_pose.to_string()), t_pose);
    }

    #[test]
    fn opening_pauses_the_ao_stops_everything_then_poses() {
        let mut stand = Stand::default();
        let facts = Facts {
            ao_on: true,
            ..Facts::default()
        };
        let out = stand.open(facts, &Settings::default());
        assert_eq!(out, vec![Effect::PauseAo, Effect::StopAnimations, Effect::Start(DEFAULT_POSE)]);
        assert!(stand.open);
        assert_eq!(stand.current(), Some(DEFAULT_POSE));
        // the AO still off: it comes back
        let out = stand.close(false, false);
        assert_eq!(out, vec![Effect::Stop(DEFAULT_POSE), Effect::StopAnimations, Effect::ResumeAo]);
        assert!(!stand.open);
        assert_eq!(stand.current(), None);
    }

    #[test]
    fn choosing_a_pose_stops_the_one_before() {
        let mut stand = Stand::default();
        stand.open(Facts::default(), &Settings::default());
        let next = POSES[3].id;
        assert_eq!(stand.select(next), vec![Effect::Stop(DEFAULT_POSE), Effect::Start(next)]);
        // the same one again is sent again, as setPose does
        assert_eq!(stand.select(next), vec![Effect::Stop(next), Effect::Start(next)]);
    }

    #[test]
    fn the_lock_sits_down_only_standing_and_without_rlva_sit() {
        let lock = Settings {
            lock: true,
            last_pose: String::new(),
        };
        let mut stand = Stand::default();
        let out = stand.open(Facts::default(), &lock);
        assert_eq!(out[0], Effect::SitOnGround);
        assert_eq!(stand.close(true, false)[0], Effect::StandUp);

        for facts in [
            Facts {
                sitting: true,
                ..Facts::default()
            },
            Facts {
                rlv_sit: true,
                ..Facts::default()
            },
        ] {
            let mut stand = Stand::default();
            assert!(!stand.open(facts, &lock).contains(&Effect::SitOnGround));
            assert!(!stand.close(true, false).contains(&Effect::StandUp));
        }
    }

    #[test]
    fn a_ground_sit_confirmed_after_closing_is_released() {
        let lock = Settings {
            lock: true,
            last_pose: String::new(),
        };
        let mut stand = Stand::default();
        stand.open(Facts::default(), &lock);
        assert!(stand.holds_lock());
        assert!(stand.late_release(false, Instant::now()).is_empty());
        // closed before the server seated us
        assert!(!stand.close(false, false).contains(&Effect::StandUp));
        assert!(!stand.holds_lock());
        let now = Instant::now();
        assert!(stand.late_release(false, now).is_empty(), "not seated yet");
        assert_eq!(stand.late_release(true, now), vec![Effect::StandUp]);
        assert!(stand.late_release(true, now).is_empty(), "once");
        // never seated: the wait ends
        let mut stand = Stand::default();
        stand.open(Facts::default(), &lock);
        stand.close(false, false);
        assert!(stand.late_release(true, now + RELEASE_WAIT * 2).is_empty());
        // a new opening drops the old wait
        let mut stand = Stand::default();
        stand.open(Facts::default(), &lock);
        stand.close(false, false);
        stand.open(Facts::default(), &Settings::default());
        assert!(stand.late_release(true, now).is_empty());
    }

    #[test]
    fn a_lock_switched_while_open_moves_and_sends_the_pose_again() {
        let mut stand = Stand::default();
        stand.open(Facts::default(), &Settings::default());
        assert_eq!(
            stand.lock_changed(true),
            vec![Effect::SitOnGround, Effect::Stop(DEFAULT_POSE), Effect::Start(DEFAULT_POSE)]
        );
        // stood up meanwhile: closing does not stand up again
        assert_eq!(stand.close(false, false), vec![Effect::Stop(DEFAULT_POSE), Effect::StopAnimations]);
    }

    #[test]
    fn an_ao_turned_on_meanwhile_is_left_alone() {
        let mut stand = Stand::default();
        stand.open(
            Facts {
                ao_on: true,
                ..Facts::default()
            },
            &Settings::default(),
        );
        assert!(!stand.close(false, true).contains(&Effect::ResumeAo));
        // an AO off from the start is not turned on
        let mut stand = Stand::default();
        stand.open(Facts::default(), &Settings::default());
        assert!(!stand.close(false, false).contains(&Effect::ResumeAo));
    }
}
