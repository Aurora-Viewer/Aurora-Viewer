//! Check that every possible standing cycle masks the actual constrained-sit
//! channels for observers too. Never stop that system motion: it carries the
//! simulator's ground-sit state. Priority alone cannot replace missing joints.
use super::{Animation, Set, State};
use crate::scene::anim::BoundAnim;
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

#[derive(Default)]
pub struct StandCheck {
    reference: Option<Arc<BoundAnim>>,
    masks: HashMap<Uuid, u128>,
}
impl StandCheck {
    pub fn ready(&mut self, set: Option<&Set>, mut get: impl FnMut(&Uuid) -> Option<Arc<BoundAnim>>) -> bool {
        let Some(config) = set.and_then(|s| s.animation_config(State::Standing)) else {
            return false;
        };
        let Some(reference) = get(&State::GroundSit.motion()) else {
            return false;
        };
        if self.reference.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &reference)) {
            self.masks.clear();
            self.reference = Some(reference.clone());
        }
        // Two channels per joint, at most 64 joints in the stock body pose.
        // Refuse an unexpected reference rather than silently miss a channel.
        if reference.anim.joints.is_empty() || reference.anim.joints.len() > 64 {
            return false;
        }
        let mut full = 0u128;
        for (i, joint) in reference.anim.joints.iter().enumerate() {
            if !joint.rot_keys.is_empty() {
                full |= 1 << (i * 2);
            }
            if !joint.pos_keys.is_empty() {
                full |= 1 << (i * 2 + 1);
            }
        }
        if full == 0 {
            return false;
        }
        let mut loaded = true;
        let mut coverage = |a: &Animation| {
            if a.asset.is_nil() {
                return 0;
            }
            if let Some(mask) = self.masks.get(&a.asset) {
                return *mask;
            }
            let Some(clip) = get(&a.asset) else {
                loaded = false;
                return 0;
            };
            let mask = mask(&clip.anim, &reference.anim);
            self.masks.insert(a.asset, mask);
            mask
        };
        // Parallel tracks can cycle independently; only channels present in
        // every member of a track are guaranteed for all combinations.
        let tracks = config.tracks.iter().fold(0, |mask, track| {
            mask | track.members.iter().map(&mut coverage).reduce(|a, b| a & b).unwrap_or(0)
        });
        let complete = if config.steps.is_empty() {
            tracks & full == full
        } else {
            config.steps.iter().all(|step| {
                let group = step.members.iter().fold(tracks, |mask, a| mask | coverage(a));
                group & full == full
            })
        };
        loaded && complete
    }
}

fn priority(joint: i32, base: i32) -> i32 {
    if joint < 0 { base } else { joint }
}
fn mask(clip: &aurora_assets::Animation, reference: &aurora_assets::Animation) -> u128 {
    if clip.loop_range().is_none() {
        return 0;
    }
    let mut mask = 0;
    for (i, original) in reference.joints.iter().enumerate() {
        let p = priority(original.priority, reference.base_priority);
        for joint in &clip.joints {
            if joint.joint_name == original.joint_name && priority(joint.priority, clip.base_priority) > p {
                if !joint.rot_keys.is_empty() {
                    mask |= 1 << (i * 2);
                }
                if !joint.pos_keys.is_empty() {
                    mask |= 1 << (i * 2 + 1);
                }
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_assets::anim::{JointMotion, PosKey, RotKey};
    use glam::{Quat, Vec3};
    fn reference() -> aurora_assets::Animation {
        let mut a = crate::demo::idle_animation();
        a.base_priority = 3;
        a.joints = vec![JointMotion {
            joint_name: "mPelvis".into(),
            priority: -1,
            rot_keys: vec![RotKey {
                time: 0.0,
                rotation: Quat::IDENTITY,
            }],
            pos_keys: vec![PosKey {
                time: 0.0,
                position: Vec3::ZERO,
            }],
        }];
        a
    }
    #[test]
    fn native_anchor_checks_each_channel_and_joint_priority_not_only_the_header() {
        let reference = reference();
        let mut a = reference.clone();
        assert_eq!(mask(&a, &reference), 0, "equal priority cannot guarantee the observer's pose");
        a.base_priority = 4;
        assert_eq!(mask(&a, &reference), 3);
        a.joints[0].priority = 2;
        assert_eq!(mask(&a, &reference), 0);
        a.joints[0].priority = 5;
        a.base_priority = 0;
        assert_eq!(mask(&a, &reference), 3);
        a.joints[0].pos_keys.clear();
        assert_eq!(mask(&a, &reference), 1, "missing pelvis translation is not a complete override");
        a.looping = false;
        assert_eq!(mask(&a, &reference), 0, "an exhausted one-shot cannot hold standing indefinitely");
    }
    #[test]
    fn native_anchor_checks_all_cycles_and_parallel_groups_and_waits_for_assets() {
        let (_, mut set) = super::super::tests::fixture();
        let rig = crate::scene::avatar::AvatarLibrary::load().rig;
        let reference = reference();
        let mut full = reference.clone();
        full.base_priority = 4;
        let bind = |a| Arc::new(BoundAnim::bind(a, &rig));
        let mut clips = HashMap::from([(State::GroundSit.motion(), bind(reference))]);
        for step in &set.states[&State::Standing].steps {
            for a in &step.members {
                clips.insert(a.asset, bind(full.clone()));
            }
        }
        let mut check = StandCheck::default();
        assert!(check.ready(Some(&set), |id| clips.get(id).cloned()));
        let second = set.states[&State::Standing].steps[1].members[0].clone();
        clips.remove(&second.asset);
        assert!(!StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()));
        let mut partial = full.clone();
        partial.joints[0].pos_keys.clear();
        clips.insert(second.asset, bind(partial));
        assert!(
            !StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()),
            "all cycles must be safe"
        );
        let mut partner = second.clone();
        partner.asset = Uuid::from_u128(991);
        full.joints[0].rot_keys.clear();
        clips.insert(partner.asset, bind(full));
        set.states.get_mut(&State::Standing).expect("standing").steps[1]
            .members
            .push(partner.clone());
        assert!(
            StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()),
            "parallel members can complete coverage"
        );
        let config = set.states.get_mut(&State::Standing).expect("standing");
        config.steps[1].members.pop();
        config.tracks.push(super::super::Track {
            id: Uuid::from_u128(992),
            name: "Pelvis".into(),
            cycle: super::super::Cycle::default(),
            members: vec![partner.clone()],
        });
        assert!(StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()));
        partner.asset = Uuid::from_u128(993);
        clips.insert(partner.asset, bind(crate::demo::idle_animation()));
        set.states.get_mut(&State::Standing).expect("standing").tracks[0]
            .members
            .push(partner);
        assert!(
            !StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()),
            "an independently cycling track must always cover its channels"
        );
    }
    #[test]
    fn native_anchor_requires_at_least_one_compatible_standing_idle() {
        let (_, mut set) = super::super::tests::fixture();
        let rig = crate::scene::avatar::AvatarLibrary::load().rig;
        let mut idle = reference();
        idle.base_priority = 4;
        let config = set.states.get_mut(&State::Standing).expect("standing");
        config.steps.truncate(1);
        config.tracks.clear();
        let asset = config.steps[0].members[0].asset;
        let clips = HashMap::from([
            (State::GroundSit.motion(), Arc::new(BoundAnim::bind(reference(), &rig))),
            (asset, Arc::new(BoundAnim::bind(idle, &rig))),
        ]);
        assert!(StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()));
        set.states.get_mut(&State::Standing).expect("standing").steps.clear();
        assert!(!StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()));
        set.states.remove(&State::Standing);
        assert!(!StandCheck::default().ready(Some(&set), |id| clips.get(id).cloned()));
        assert!(!StandCheck::default().ready(None, |id| clips.get(id).cloned()));
    }
}
