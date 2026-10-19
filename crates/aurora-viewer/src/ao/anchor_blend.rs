//! Observer-side easing coverage for the native anchor. A P4 clip whose
//! weight is below one still exposes the P3 ground-sit underneath it
//! (LLJointStateBlender::blendJointStates, indra/llcharacter/llpose.cpp).
use crate::world::PlayingAnimation;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DELIVERY_MARGIN: Duration = Duration::from_millis(150);
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pose {
    Anchored,
    Standing,
    Moving,
}

#[derive(Default)]
pub struct Blend {
    /// One fully weighted group, independent of the next selection's timer.
    cover: Vec<Uuid>,
    held: bool,
    until: Option<Instant>,
    target: Vec<Uuid>,
}
impl Blend {
    pub fn prepared(&self) -> bool {
        self.held && !self.cover.is_empty()
    }
    /// Extra UUIDs to retain in the ordinary AO owner, never a second sender.
    pub fn update(
        &mut self,
        pose: Pose,
        candidate: &[Uuid],
        active: &[PlayingAnimation],
        raw: &[PlayingAnimation],
        now: Instant,
        mut ease_in: impl FnMut(&Uuid) -> Option<Duration>,
    ) -> &[Uuid] {
        let hold = pose == Pose::Anchored;
        if !hold && !self.held && self.cover.is_empty() {
            return &self.cover;
        }
        let warm = !candidate.is_empty()
            && candidate.iter().all(|id| {
                let Some(playing) = active.iter().find(|a| a.id == *id) else {
                    return false;
                };
                // A simulator echo proves the reliable start was processed before
                // asking it to sit. Include authored easing after that receipt.
                let Some(echo) = raw.iter().find(|a| a.id == *id) else {
                    return false;
                };
                let Some(ease) = ease_in(id) else { return false };
                now.saturating_duration_since(playing.continuous_start.max(echo.continuous_start)) >= ease + DELIVERY_MARGIN
            });
        if hold {
            self.until = None;
            if warm {
                self.cover = candidate.to_vec();
            }
        } else if self.held {
            self.until = Some(now + HANDOVER_TIMEOUT);
            self.target.clear();
        }
        if !hold && !self.cover.is_empty() {
            if pose == Pose::Moving && self.target != candidate {
                self.target = candidate.to_vec();
                let fade = candidate.iter().filter_map(&mut ease_in).max().unwrap_or_default();
                self.until = Some(now + HANDOVER_TIMEOUT + fade);
            }
            if warm && pose == Pose::Standing {
                // A stand-up acknowledgement can precede the first walking
                // signal. Keep a full group across that interval too.
                self.cover = candidate.to_vec();
            }
            if (warm && pose == Pose::Moving) || self.until.is_some_and(|until| now >= until) {
                self.cover.clear();
                self.until = None;
            }
        }
        self.held = hold;
        &self.cover
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signal(id: u128, start: Instant) -> PlayingAnimation {
        PlayingAnimation::from_signal(Uuid::from_u128(id), 1, start, None)
    }
    #[test]
    fn native_anchor_waits_for_echo_and_authored_easing_before_sitting() {
        let t = Instant::now();
        let a = signal(1, t);
        let echo = signal(1, t + Duration::from_millis(80));
        let mut b = Blend::default();
        let fade = |_: &Uuid| Some(Duration::from_millis(400));
        assert!(
            b.update(Pose::Anchored, &[a.id], &[a], &[], t + Duration::from_secs(5), fade)
                .is_empty()
        );
        assert!(!b.prepared(), "local animation time alone is insufficient");
        assert!(
            b.update(Pose::Anchored, &[a.id], &[a], &[echo], t + Duration::from_millis(629), fade)
                .is_empty()
        );
        assert_eq!(
            b.update(Pose::Anchored, &[a.id], &[a], &[echo], t + Duration::from_millis(630), fade),
            [a.id]
        );
        assert!(b.prepared());
    }
    #[test]
    fn native_anchor_keeps_one_full_group_through_cycles_and_movement() {
        let t = Instant::now();
        let a = signal(1, t);
        let mut b = Blend::default();
        b.update(Pose::Anchored, &[a.id], &[a], &[a], t + Duration::from_secs(1), |_| {
            Some(Duration::ZERO)
        });
        for i in 2..100 {
            let next = signal(i, t + Duration::from_secs(2));
            assert_eq!(
                b.update(
                    Pose::Anchored,
                    &[next.id],
                    &[a, next],
                    &[a, next],
                    t + Duration::from_secs(2),
                    |_| Some(Duration::from_secs(1))
                ),
                [a.id]
            );
        }
        let walk = signal(100, t + Duration::from_secs(3));
        assert_eq!(
            b.update(
                Pose::Moving,
                &[walk.id],
                &[a, walk],
                &[a, walk],
                t + Duration::from_millis(3500),
                |_| Some(Duration::from_millis(800))
            ),
            [a.id]
        );
        assert!(!b.prepared());
        assert!(
            b.update(
                Pose::Moving,
                &[walk.id],
                &[a, walk],
                &[a, walk],
                t + Duration::from_millis(3950),
                |_| Some(Duration::from_millis(800))
            )
            .is_empty()
        );
    }
    #[test]
    fn native_anchor_missing_replacement_expires_and_the_ordinary_ao_has_no_extra_lookups() {
        let t = Instant::now();
        let a = signal(1, t);
        let mut b = Blend::default();
        b.update(Pose::Moving, &[a.id], &[a], &[a], t, |_| {
            panic!("ordinary AO must not query easing")
        });
        b.update(Pose::Anchored, &[a.id], &[a], &[a], t + Duration::from_secs(1), |_| {
            Some(Duration::ZERO)
        });
        assert_eq!(
            b.update(Pose::Moving, &[], &[a], &[a], t + Duration::from_secs(2), |_| None),
            [a.id]
        );
        assert!(
            b.update(Pose::Moving, &[], &[a], &[], t + Duration::from_secs(4), |_| None)
                .is_empty()
        );
    }

    /// Raw simulator animations, including P3 sit/stand-up, go into a separate
    #[test]
    fn native_anchor_keeps_cover_if_standing_is_acknowledged_before_walking() {
        let t = Instant::now();
        let a = signal(1, t);
        let mut b = Blend::default();
        b.update(Pose::Anchored, &[a.id], &[a], &[a], t + Duration::from_secs(1), |_| {
            Some(Duration::ZERO)
        });
        assert_eq!(
            b.update(Pose::Standing, &[a.id], &[a], &[a], t + Duration::from_millis(1500), |_| Some(
                Duration::ZERO
            )),
            [a.id]
        );
        let walk = signal(2, t + Duration::from_millis(1600));
        assert_eq!(
            b.update(
                Pose::Moving,
                &[walk.id],
                &[a, walk],
                &[a, walk],
                t + Duration::from_millis(1600),
                |_| Some(Duration::from_millis(400))
            ),
            [a.id]
        );
        assert!(
            b.update(
                Pose::Moving,
                &[walk.id],
                &[a, walk],
                &[a, walk],
                t + Duration::from_millis(2150),
                |_| Some(Duration::from_millis(400))
            )
            .is_empty()
        );
    }

    /// Raw simulator animations, including P3 sit/stand-up, go into a separate
    /// production motion controller. No owner-side AO pose filtering is used.
    fn observer_minimum_pelvis(continuous: bool) -> f32 {
        use crate::scene::anim::{BoundAnim, Controller, PALETTE_JOINTS, Rig};
        use aurora_assets::{Animation, JointMotion, Skeleton, anim::PosKey};
        use glam::{Mat4, Vec3};
        use std::{collections::HashMap, sync::Arc};
        let xml = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/character/avatar_skeleton.xml")).expect("skeleton");
        let rig = Rig::new(&Skeleton::parse(&xml).expect("parse skeleton"));
        let clip = |priority, z, ease_in, ease_out, looping| {
            let a = Animation {
                base_priority: priority,
                duration: 1.2,
                emote_name: String::new(),
                loop_in: 0.0,
                loop_out: 1.2,
                looping,
                ease_in,
                ease_out,
                hand_pose: 0,
                joints: vec![JointMotion {
                    joint_name: "mPelvis".into(),
                    priority: -1,
                    rot_keys: Vec::new(),
                    pos_keys: vec![PosKey {
                        time: 0.0,
                        position: Vec3::new(0.0, 0.0, z),
                    }],
                }],
            };
            Arc::new(BoundAnim::bind(a, &rig))
        };
        let mut clips = HashMap::new();
        clips.insert(Uuid::from_u128(1), clip(4, 0.0, 0.4, 0.1, true));
        clips.insert(Uuid::from_u128(2), clip(4, 0.0, 0.8, 0.2, true));
        clips.insert(Uuid::from_u128(3), clip(3, -0.6, 0.0, 0.1, true));
        clips.insert(Uuid::from_u128(4), clip(3, -0.6, 0.0, 0.2, false));
        let t = Instant::now();
        let at = |ms| t + Duration::from_millis(ms);
        let mut blend = Blend::default();
        let mut active = Vec::<PlayingAnimation>::new();
        let mut observer = Controller::default();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        observer.evaluate(&rig, t, None, &mut out);
        let rest = Mat4::from_cols_array_2d(&out[rig.pelvis]).w_axis.z;
        let mut min = 0.0f32;
        for ms in (0..=3400).step_by(20) {
            let now = at(ms);
            let standing = ms < 2000 && (continuous || ms >= 1500);
            let mut desired = if standing {
                vec![Uuid::from_u128(1)]
            } else if ms >= 2000 {
                vec![Uuid::from_u128(2)]
            } else {
                Vec::new()
            };
            if continuous {
                let extra = blend.update(
                    if ms < 2000 { Pose::Anchored } else { Pose::Moving },
                    &desired,
                    &active,
                    &active,
                    now,
                    |id| clips.get(id).map(|a| Duration::from_secs_f32(a.anim.ease_in)),
                );
                for id in extra {
                    if !desired.contains(id) {
                        desired.push(*id);
                    }
                }
                if ms == 1500 {
                    assert!(blend.prepared(), "pose must be warm before the sit request");
                }
            }
            let previous = std::mem::take(&mut active);
            active = desired
                .into_iter()
                .map(|id| PlayingAnimation::from_signal(id, 1, now, previous.iter().find(|a| a.id == id)))
                .collect();
            let mut raw = active.clone();
            if (1500..2000).contains(&ms) {
                raw.push(signal(3, at(1500)));
            }
            if (2000..3200).contains(&ms) {
                raw.push(signal(4, at(2000)));
            }
            observer.sync(&raw, now, |id| clips.get(id).cloned());
            observer.evaluate(&rig, now, None, &mut out);
            min = min.min(Mat4::from_cols_array_2d(&out[rig.pelvis]).w_axis.z - rest);
        }
        min
    }
    #[test]
    fn native_anchor_observer_has_no_pelvis_dip_during_entry_and_movement_handover() {
        let old = observer_minimum_pelvis(false);
        let covered = observer_minimum_pelvis(true);
        assert!(old < -0.3, "reproduce the observer's downward pose: {old}");
        assert!(covered > -0.0001, "full P4 coverage must hide the P3 native poses: {covered}");
    }
}
