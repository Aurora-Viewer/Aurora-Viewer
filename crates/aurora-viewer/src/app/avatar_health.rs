//! Me › Avatar health and the customise animation.
//!
//! - "Undeform avatar" (FSToolsUndeform in indra/newview/llviewermenu.cpp,
//!   FSPose::setPose in indra/newview/fspose.cpp): our skeleton is reset
//!   with its animations (LLVOAvatar::resetSkeleton(true), which stops them
//!   like "Stop animations"), then the undeform animation (FSUndeformUUID)
//!   is started once on the simulator, for everyone; it is never stopped
//!   (setPose without saving the state).
//! - "Refresh attachments" (LLAttachmentsMgr::refreshAttachments in
//!   indra/newview/llattachmentsmgr.cpp): every object we wear from the
//!   inventory is asked again on its point, added (ATTACHMENT_ADD), which
//!   brings back the ones the simulator lost track of; temporary ones and
//!   items asked for less than MIN_RETRY_REQUEST_TIME ago are left alone.
//! - The customise animation (LLAgentCamera::changeCameraToCustomizeAvatar
//!   in indra/newview/llagentcamera.cpp, LLAgent::endAnimationUpdateUI in
//!   indra/newview/llagent.cpp): when the camera frames our avatar for
//!   editing its appearance (AppearanceCameraMovement, on by default; the
//!   shape editor always does), our avatar stands up and
//!   ANIM_AGENT_CUSTOMIZE plays; leaving stops it and plays
//!   ANIM_AGENT_CUSTOMIZE_DONE.
//!
//! All originally LGPL 2.1.
use super::App;
use crate::world::inventory::Inventory;
use aurora_net::{AttachRequest, NetCommand};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// ANIM_AGENT_CUSTOMIZE / ANIM_AGENT_CUSTOMIZE_DONE (llanimationstates.cpp).
pub(crate) const ANIM_CUSTOMIZE: Uuid = Uuid::from_u128(0x038fcec9_5ebd_8a8e_0e2e_6e71a0a1ac53);
pub(crate) const ANIM_CUSTOMIZE_DONE: Uuid = Uuid::from_u128(0x6883a61a_b27b_5914_a61e_dda118a9ee2c);
/// FSUndeformUUID's default (app_settings/settings.xml).
const ANIM_UNDEFORM: Uuid = Uuid::from_u128(0x44e98907_3764_119f_1c13_cba9945d2ff4);
/// ANIM_AGENT_CUSTOMIZE's turn (1.35 s) with the round trip of its start to
/// the simulator; the Legacy Viewer waits for it too before its camera
/// settles (CUSTOMIZE_AVATAR_CAMERA_ANIM_SLOP after the motion).
const TURN_TIME: Duration = Duration::from_millis(2000);
/// MIN_RETRY_REQUEST_TIME of LLAttachmentsMgr's attachment requests.
const RETRY_AFTER: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct AvatarHealth {
    /// ANIM_AGENT_CUSTOMIZE asked for (LLAgent::mCustomAnim).
    customizing: bool,
    /// The shape editor frames our avatar turned around (customize_turns).
    turned: bool,
    /// When to frame it again, on the pose the turn ends with.
    turn_ends: Option<Instant>,
    /// When each item was last asked again (LLAttachmentsMgr::mAttachmentRequests).
    refreshed: HashMap<Uuid, Instant>,
}

/// An object our avatar wears: the item it was worn from (AttachItemID),
/// its point, and whether it is a temporary attachment (no item of ours:
/// LLViewerObject::isTempAttachment, its id is its item id).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Worn {
    item: Uuid,
    point: u8,
    temporary: bool,
}

impl App {
    /// Once per frame: the customise animation follows the appearance
    /// editors, and the `AURORA_DEMO_AVATAR_HEALTH` scenario. The physics
    /// layer editor does not count: its wearable type keeps the camera
    /// (LLWearableType WT_PHYSICS, disable_camera_switch), so the Legacy
    /// Viewer never enters customise mode for it. With the customise
    /// animation turned off (an Aurora choice), our avatar keeps its pose.
    pub(super) fn update_avatar_health(&mut self) {
        let editing = (self.shape_ui.open || self.wearable_ui.open) && !self.world.agent_id.is_nil() && self.settings.customize_animation;
        if editing && !self.avatar_health.customizing {
            // changeCameraToCustomizeAvatar: "force stand up" first
            // (LLAgent::standUp, AGENT_CONTROL_STAND_UP)
            self.send(NetCommand::OneShotControl(aurora_net::control::STAND_UP));
        }
        for (anim, start) in customize_changes(&mut self.avatar_health.customizing, editing) {
            log::info!("customise animation: {} {anim}", if start { "start" } else { "stop" });
            self.own_animation(anim, start);
        }
        let turned = editing && self.customize_turns();
        if turned != self.avatar_health.turned {
            self.avatar_health.turned = turned;
            self.avatar_health.turn_ends = turned.then(|| Instant::now() + TURN_TIME);
            if self.shape_ui.open {
                // frame our avatar again from the side it now faces
                self.shape_ui.camera_pending = true;
            }
            self.wearable_ui.camera_pending |= self.wearable_ui.open;
        }
        if self.avatar_health.turn_ends.is_some_and(|at| Instant::now() >= at) {
            self.avatar_health.turn_ends = None;
            self.shape_ui.camera_pending |= self.shape_ui.open;
            self.wearable_ui.camera_pending |= self.wearable_ui.open;
        }
        if self.demo && self.frame_count == 600 {
            match std::env::var("AURORA_DEMO_AVATAR_HEALTH").as_deref() {
                Ok("undeform") => self.undeform_avatar(),
                Ok("refresh") => self.refresh_attachments(),
                _ => {}
            }
        }
    }

    /// Whether the shape editor finds our avatar turned around: the
    /// customise animation is chosen and its motion is loaded (none in the
    /// demo, which has no built-in animation assets; a first download takes
    /// a moment). Asking for it also starts its loading.
    pub(super) fn customize_turns(&mut self) -> bool {
        self.settings.customize_animation && self.scene.anims.get(&ANIM_CUSTOMIZE).is_some()
    }

    /// The customise animation is not asked for again after a logout.
    pub(super) fn reset_avatar_health(&mut self) {
        self.avatar_health = AvatarHealth::default();
    }

    pub(super) fn undeform_avatar(&mut self) {
        let me = self.world.agent_id;
        if me.is_nil() {
            return;
        }
        self.scene.reset_skeleton(me, true);
        self.stop_current_animations();
        self.own_animation(ANIM_UNDEFORM, true);
        log::info!("undeform: skeleton and animations reset, undeform animation started");
    }

    pub(super) fn refresh_attachments(&mut self) {
        let now = Instant::now();
        let refreshed = &mut self.avatar_health.refreshed;
        refreshed.retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
        let worn = worn_objects(&self.world);
        let requests = refresh_requests(&self.world.inventory, self.world.agent_id, &worn, refreshed);
        for r in &requests {
            refreshed.insert(r.item_id, now);
        }
        log::info!("refresh attachments: {} of {} worn objects asked again", requests.len(), worn.len());
        if !requests.is_empty() {
            self.send(NetCommand::RezAttachments(requests));
        }
    }

    /// One of our own animations started or stopped on the simulator; the
    /// demo has none, so it plays here at once (as the gestures' do).
    pub(super) fn own_animation(&mut self, anim: Uuid, start: bool) {
        if self.demo {
            let motions = self.world.animations.entry(self.world.agent_id).or_default();
            motions.retain(|a| a.id != anim);
            if start {
                motions.push(crate::world::PlayingAnimation::from_signal(anim, 0, Instant::now(), None));
            }
        } else {
            self.send(NetCommand::AgentAnimation { anim, start });
        }
    }
}

/// What entering or leaving appearance editing asks for: the customise
/// animation started, or stopped and followed by its "done" animation
/// (LLAgent::endAnimationUpdateUI).
fn customize_changes(customizing: &mut bool, editing: bool) -> Vec<(Uuid, bool)> {
    if *customizing == editing {
        return Vec::new();
    }
    *customizing = editing;
    if editing {
        vec![(ANIM_CUSTOMIZE, true)]
    } else {
        vec![(ANIM_CUSTOMIZE, false), (ANIM_CUSTOMIZE_DONE, true)]
    }
}

/// The objects our avatar wears (LLVOAvatar::mAttachmentPoints).
fn worn_objects(world: &crate::world::World) -> Vec<Worn> {
    let Some(me) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i)) else {
        return Vec::new();
    };
    world
        .objects
        .children_of(&me.key)
        .iter()
        .filter_map(|i| world.objects.get(*i))
        .filter_map(|o| {
            let item = o.attachment_item_id().filter(|id| !id.is_nil())?;
            Some(Worn {
                item,
                point: o.attachment_point(),
                temporary: o.full_id == item,
            })
        })
        .collect()
}

/// LLAttachmentsMgr::refreshAttachments then requestAttachments: each worn
/// object again on its point, added, unless it is temporary, was asked for
/// recently, or its item is not in our inventory.
fn refresh_requests(inv: &Inventory, agent: Uuid, worn: &[Worn], recent: &HashMap<Uuid, Instant>) -> Vec<AttachRequest> {
    worn.iter()
        .filter(|w| !w.temporary && !recent.contains_key(&w.item))
        .filter_map(|w| {
            let Some(it) = inv.items.get(&w.item) else {
                log::warn!("refresh attachments: worn item {} not in the inventory", w.item);
                return None;
            };
            Some(AttachRequest {
                item_id: it.id,
                owner_id: if it.owner.is_nil() { agent } else { it.owner },
                point: w.point,
                add: true,
                flags: it.flags,
                group_mask: it.group_mask,
                everyone_mask: it.everyone_mask,
                next_owner_mask: it.next_owner_mask,
                name: it.name.clone(),
                desc: it.desc.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_starts_the_customise_animation_and_leaving_plays_its_end() {
        let mut customizing = false;
        assert!(customize_changes(&mut customizing, false).is_empty());
        assert_eq!(customize_changes(&mut customizing, true), vec![(ANIM_CUSTOMIZE, true)]);
        assert!(customize_changes(&mut customizing, true).is_empty());
        assert_eq!(
            customize_changes(&mut customizing, false),
            vec![(ANIM_CUSTOMIZE, false), (ANIM_CUSTOMIZE_DONE, true)]
        );
        assert!(!customizing);
    }

    #[test]
    fn refresh_asks_again_for_worn_items_on_their_points() {
        let agent = Uuid::from_u128(1);
        let (hat, ring, temp, recent_item, unknown) = (
            Uuid::from_u128(10),
            Uuid::from_u128(11),
            Uuid::from_u128(12),
            Uuid::from_u128(13),
            Uuid::from_u128(14),
        );
        let mut inv = Inventory::default();
        for (id, name) in [(hat, "Hat"), (ring, "Ring"), (temp, "Temp"), (recent_item, "Recent")] {
            inv.items.insert(
                id,
                aurora_net::inventory::InvItem {
                    id,
                    name: name.into(),
                    asset_type: 6,
                    ..Default::default()
                },
            );
        }
        let worn = [
            Worn {
                item: hat,
                point: 2,
                temporary: false,
            },
            Worn {
                item: ring,
                point: 39,
                temporary: false,
            },
            Worn {
                item: temp,
                point: 5,
                temporary: true,
            },
            Worn {
                item: recent_item,
                point: 6,
                temporary: false,
            },
            Worn {
                item: unknown,
                point: 7,
                temporary: false,
            },
        ];
        let recent = HashMap::from([(recent_item, Instant::now())]);
        let asked = refresh_requests(&inv, agent, &worn, &recent);
        let got: Vec<_> = asked.iter().map(|r| (r.item_id, r.point, r.add, r.owner_id)).collect();
        assert_eq!(got, vec![(hat, 2, true, agent), (ring, 39, true, agent)]);
        assert_eq!(asked[0].name, "Hat");
    }
}
