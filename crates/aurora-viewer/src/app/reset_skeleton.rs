//! "Reset skeleton", "Reset skeleton and animations" and "Stop animations"
//! (LLAvatarResetSkeleton, LLAvatarResetSkeletonAndAnimations,
//! LLToolsStopAllAnimations in indra/newview/llviewermenu.cpp,
//! LLHUDEffectResetSkeleton, LLAgent::stopCurrentAnimations in
//! indra/newview/llagent.cpp, originally LGPL 2.1).
//!
//! Another avatar is reset on our screen only. Ours is reset here and the
//! others are told with a ResetSkeleton ViewerEffect, which their viewers
//! follow when an avatar asks it for itself; with the animations, ours are
//! stopped on the simulator too.
use super::App;
use crate::world::body::ANIM_SIT_GROUND_CONSTRAINED;
use crate::world::status::ANIM_DO_NOT_DISTURB;
use aurora_net::{NetCommand, control};
use uuid::Uuid;

/// ANIM_AGENT_STAND (llanimationstates.cpp).
const ANIM_STAND: Uuid = Uuid::from_u128(0x2408fe9e_df1d_1d7d_f4ff_1384fa7b350f);
/// SCRIPT_PERMISSION_TRIGGER_ANIMATION | SCRIPT_PERMISSION_OVERRIDE_ANIMATIONS,
/// the only bits the simulator accepts in RevokePermissions.
const ANIMATION_PERMISSIONS: u32 = 0x10 | 0x8000;

impl App {
    pub(super) fn reset_skeleton(&mut self, avatar: Uuid, animations: bool) {
        if avatar == self.world.agent_id {
            // LLHUDEffectResetSkeleton: sent, and applied at once since it
            // originated here (LLHUDEffectResetSkeleton::update)
            self.send(NetCommand::ResetSkeleton {
                effect: Uuid::new_v4(),
                target: avatar,
                animations,
            });
            if animations {
                self.stop_current_animations();
            }
        }
        self.scene.reset_skeleton(avatar, animations);
        log::info!("skeleton reset: {avatar}{}", if animations { ", with its animations" } else { "" });
    }

    /// The resets other avatars asked for themselves (ViewerEffect).
    pub(super) fn apply_skeleton_resets(&mut self) {
        for (avatar, animations) in std::mem::take(&mut self.world.skeleton_resets) {
            self.scene.reset_skeleton(avatar, animations);
        }
    }

    /// LLAgent::stopCurrentAnimations, with Firestorm's defaults
    /// (RevokePermsOnStopAnimation on, FSPlayDefaultBentoAnimation off):
    /// every animation of ours but do-not-disturb and the ground sit stops,
    /// here at once and on the simulator, its animation overrides are
    /// cleared, the scripts of the region lose their animation permissions
    /// (sitting, we stand up: the seat's own permission is revoked too), and
    /// the stand starts again unless we sat on the ground.
    pub(super) fn stop_current_animations(&mut self) {
        self.stop_animations(true);
    }

    /// LLAgent::stopCurrentAnimations(force_keep_script_perms): with
    /// `revoke` off (Avatar health's "Stop avatar animations",
    /// Tools.StopAllAnimations "stop"), the scripts keep their animation
    /// permissions and a seated avatar stays seated.
    pub(super) fn stop_animations(&mut self, revoke: bool) {
        let me = self.world.agent_id;
        let playing: Vec<Uuid> = self
            .world
            .animations
            .get(&me)
            .map(|list| list.iter().map(|p| p.id).collect())
            .unwrap_or_default();
        let ground_sit = playing.contains(&ANIM_SIT_GROUND_CONSTRAINED);
        let stop = stopped_animations(&playing);
        if let Some(list) = self.world.animations.get_mut(&me) {
            list.retain(|p| !stop.contains(&p.id));
        }
        self.send(NetCommand::AgentAnimations { anims: stop, start: false });
        self.send(NetCommand::AnimationStateReset);
        let region = self
            .world
            .main_region
            .and_then(|h| self.world.regions.get(&h))
            .and_then(|r| r.info.as_ref())
            .map(|i| i.region_id);
        if revoke && let Some(region) = region.filter(|r| !r.is_nil()) {
            self.send(NetCommand::RevokePermissions {
                target: region,
                permissions: ANIMATION_PERMISSIONS,
            });
            if self.world.agent.is_sitting() {
                self.send(NetCommand::OneShotControl(control::STAND_UP));
            }
        }
        if !ground_sit {
            self.send(NetCommand::AgentAnimation {
                anim: ANIM_STAND,
                start: true,
            });
        }
    }
}

/// What LLAgent::stopCurrentAnimations stops: all but do-not-disturb and
/// the ground sit (viewers tell from it that we sit).
fn stopped_animations(playing: &[Uuid]) -> Vec<Uuid> {
    playing
        .iter()
        .copied()
        .filter(|id| *id != ANIM_DO_NOT_DISTURB && *id != ANIM_SIT_GROUND_CONSTRAINED)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_keeps_do_not_disturb_and_the_ground_sit() {
        let dance = Uuid::from_u128(42);
        let playing = [dance, ANIM_DO_NOT_DISTURB, ANIM_SIT_GROUND_CONSTRAINED, ANIM_STAND];
        assert_eq!(stopped_animations(&playing), vec![dance, ANIM_STAND]);
        assert!(stopped_animations(&[]).is_empty());
    }
}
