//! Enforce outgoing actions at the application's common send boundary.

use super::super::World;
use aurora_net::{NetCommand, control};
use glam::Vec3;
use uuid::Uuid;

/// What RLVa makes of a double click that walks or teleports to a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleClickRlva {
    Go,
    /// @interact: LLViewerWindow::cursorIntersect dropped the pick, the
    /// land's included, so there is nowhere to go and nothing is said.
    Ignored,
    /// RlvActions::canTeleportToLocal refuses the point: the Legacy tool
    /// says Blocked::AutoPilot.
    Blocked,
}

impl World {
    /// Resident and group payments follow RLV `@pay`; object purchases use
    /// the independent `@buy` restriction.
    pub fn rlva_can_pay(&self) -> bool {
        !self.rlva.has("pay")
    }

    pub fn rlva_allows_im(&self, behaviour: &str, target: Uuid) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let distance = self
            .objects
            .index_of_uuid(&target)
            .and_then(|i| crate::scene::Scene::object_transform_unshifted(self, i, std::time::Instant::now()))
            .map(|(p, _, _)| p.distance(self.agent.position));
        distance.map_or_else(
            || self.rlva.allows_im(behaviour, target),
            |d| self.rlva.allows_im_at(behaviour, target, Some(d)),
        )
    }
    pub fn rlva_can_touch(&self, handle: aurora_net::RegionHandle, local_id: u32) -> bool {
        self.rlva_can_touch_at(handle, local_id, None)
    }

    /// `position` is in the viewer's region-relative world, like a picking
    /// ray. Measure the clicked surface, not the object's bounding-box centre.
    pub fn rlva_can_touch_at(&self, handle: aurora_net::RegionHandle, local_id: u32, position: Option<Vec3>) -> bool {
        let r = &self.rlva;
        if !r.enabled() {
            return true;
        }
        let Some(index) = self.objects.index_of(&super::super::objects::ObjKey { region: handle, local_id }) else {
            return false;
        };
        if !self.rlva_object_visible(index) {
            return false;
        }
        let mut root = index;
        let mut wearer = None;
        for _ in 0..64 {
            let Some(object) = self.objects.get(root) else {
                return false;
            };
            let Some(parent) = self.objects.parent_of(object).and_then(|i| self.objects.get(i).map(|o| (i, o))) else {
                break;
            };
            if parent.1.is_avatar() {
                wearer = Some(parent.1.full_id);
                break;
            }
            root = parent.0;
        }
        let Some(object) = self.objects.get(root) else {
            return false;
        };
        let id = object.full_id.to_string();
        let hud = wearer == Some(self.agent_id) && self.avatar_lib.attach_points.get(&object.attachment_point()).is_some_and(|p| p.hud);
        // @interact is enforced when the object is picked
        // (LLViewerWindow::cursorIntersect, llviewerwindow.cpp), before
        // RlvActions::canTouch and its @touchme override are asked. A HUD's
        // pick is dropped there too, by `rlva_drops_picks` where the click
        // is read: this rule also answers the Touch entry of a worn object,
        // which needs no pick and keeps a HUD's touch (handle_attachment_touch,
        // llviewermenu.cpp).
        if !hud && r.has("interact") {
            return false;
        }
        if r.rules_for("touchme", Uuid::nil()).any(|(source, _)| {
            let Some(mut i) = self.objects.index_of_uuid(&source) else {
                return false;
            };
            for _ in 0..64 {
                if i == root {
                    return true;
                }
                let Some(parent) = self.objects.get(i).and_then(|o| self.objects.parent_of(o)) else {
                    break;
                };
                i = parent;
            }
            false
        }) {
            return true;
        }
        if r.targets("touchthis", &id) || (!hud && r.has("touchall")) {
            return false;
        }
        if hud {
            return r.allows("touchhud", &id);
        }
        match wearer {
            None if !r.allows("touchworld", &id) => return false,
            Some(w) if w == self.agent_id => {
                if !r.allows("touchattach", &id) || (r.has("touchattachself") && !r.targets("touchattach", &id)) {
                    return false;
                }
            }
            Some(w)
                if ((!r.allows("touchattach", &id) || r.has("touchattachother"))
                    && !r.targets("touchattach", &w.to_string())
                    && !r.targets("touchattach", &id))
                    || r.targets("touchattachother", &w.to_string()) =>
            {
                return false;
            }
            _ => {}
        }
        if wearer != Some(self.agent_id) {
            let max = r.distance_limit("fartouch", 1.5, Uuid::nil());
            if max.is_finite()
                && !position
                    .filter(|p| p.is_finite())
                    .or_else(|| crate::scene::Scene::object_transform_unshifted(self, index, std::time::Instant::now()).map(|(p, _, _)| p))
                    .is_some_and(|p| p.distance(self.agent.position) <= max)
            {
                return false;
            }
        }
        true
    }

    /// RlvActions::isLocalTp, canTeleportToLocal and canTeleportToLocation:
    /// within a region's width (flat) a teleport is local, limited by @sittp
    /// and @tplocal measured in 3D so that going straight up does not get
    /// around them; a bare @tplocal allows 1.5 m. Farther, only @tploc.
    fn rlva_teleport_delta_allowed(&self, delta: Vec3, actor: Uuid) -> bool {
        let r = &self.rlva;
        if delta.truncate().length_squared() < 256.0 * 256.0 {
            self.rlva_local_teleport_allowed(delta.length(), actor)
        } else {
            !(self.agent.is_sitting() && r.has_except("unsit", actor)) && !r.has_except("tploc", actor)
        }
    }

    /// RlvActions::canTeleportToLocal: the agent may stand up, and the
    /// place is within what @sittp and @tplocal leave, measured in 3D.
    /// Whether a distance applies leaves out the issuer's own restriction
    /// (hasBehaviourExcept), but the distance is the global modifier's, the
    /// smallest of every object's (RlvBehaviourModifier::getValue), the
    /// issuer's own included.
    fn rlva_local_teleport_allowed(&self, distance: f32, actor: Uuid) -> bool {
        let r = &self.rlva;
        !(self.agent.is_sitting() && r.has_except("unsit", actor))
            && (!r.has_except("sittp", actor) || distance < r.distance_limit("sittp", 1.5, Uuid::nil()))
            && (!r.has_except("tplocal", actor) || distance < r.distance_limit("tplocal", 1.5, Uuid::nil()).min(256.0))
    }

    /// RlvActions::canTeleportToLocal asked alone, whatever the distance:
    /// what the area search shows "Teleport to" on
    /// (FSPanelAreaSearchList::onContextMenuItemVisibleRLV,
    /// indra/newview/fsareasearch.cpp). Farther than a region's width the
    /// entry is hidden under @sittp or @tplocal, and shown under @tploc
    /// alone, which refuses the teleport once asked.
    pub fn rlva_can_teleport_local(&self, global: glam::DVec3) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let distance = self.agent_global().map_or(f32::INFINITY, |here| (global - here).as_vec3().length());
        self.rlva_local_teleport_allowed(distance, Uuid::nil())
    }
    /// A double click walking or teleporting to `global`
    /// (LLToolPie::walkToClickedLocation and teleportToClickedLocation,
    /// lltoolpie.cpp): @interact leaves no pick; otherwise the point must
    /// be one a local teleport may reach (@tplocal, @sittp, @unsit while
    /// seated), however far it is, and never @tploc: the teleport itself
    /// asks that one (LLAgent::teleportViaLocationLookAt).
    pub fn rlva_double_click(&self, global: Option<glam::DVec3>) -> DoubleClickRlva {
        if self.rlva_drops_picks() {
            DoubleClickRlva::Ignored
        } else if global.is_some_and(|g| !self.rlva_can_teleport_local(g)) {
            DoubleClickRlva::Blocked
        } else {
            DoubleClickRlva::Go
        }
    }

    /// What the interface greys: the same rules as the send boundary, so a
    /// greyed entry and a refused click never disagree.
    pub fn rlva_can_teleport_home(&self) -> bool {
        self.rlva_allows_command(&NetCommand::TeleportHome, Uuid::nil())
    }

    /// World › Teleport Home (LLWorldEnableTeleportHome, llviewermenu.cpp):
    /// the menu entry is greyed under either @tplm or @tploc, although the
    /// teleport is refused only under both. The Home buttons of the
    /// navigation bar and of the map follow the teleport's rule
    /// (RlvUIEnabler::onToggleTp, LLFloaterWorldMap).
    pub fn rlva_home_entry_enabled(&self) -> bool {
        self.rlva_can_teleport_home() && !self.rlva.has("tplm") && !self.rlva.has("tploc")
    }

    /// RlvActions::canTeleportToLocation (rlvactions.cpp), what the Legacy
    /// Viewer's place windows grey their Teleport button with, whatever the
    /// distance (LLPanelPlaces::updateVerbs, LLTeleportHistoryPanel,
    /// FSFloaterPlaceDetails): no @tploc, and free to stand up.
    pub fn rlva_can_teleport_location(&self) -> bool {
        !self.rlva.enabled() || (!self.rlva.has("tploc") && !(self.agent.is_sitting() && self.rlva.has("unsit")))
    }

    /// The Home buttons of the navigation bar and of the map
    /// (RlvUIEnabler::onToggleTp, rlvui.cpp; LLFloaterWorldMap::draw): greyed
    /// only under both @tplm and @tploc, seated under @unsit or not.
    pub fn rlva_home_button_enabled(&self) -> bool {
        !self.rlva.enabled() || !(self.rlva.has("tplm") && self.rlva.has("tploc"))
    }

    /// A landmark's Teleport button (LLLandmarksPanel::isActionEnabled,
    /// llpanellandmarks.cpp; LLPanelPlaces::updateVerbs): greyed under @tplm.
    pub fn rlva_landmark_button_enabled(&self) -> bool {
        !self.rlva.enabled() || !self.rlva.has("tplm")
    }

    pub fn rlva_can_teleport_landmark(&self) -> bool {
        self.rlva_allows_command(&NetCommand::TeleportLandmark(Uuid::nil()), Uuid::nil())
    }

    /// A teleport to a global position (LLAgent::teleportViaLocation), local
    /// or not; unknown where we stand, it counts as a distant one. The
    /// windows now grey as the Legacy ones do (rlva_can_teleport_location),
    /// so only the tests still ask it.
    #[cfg(test)]
    pub fn rlva_can_teleport_global(&self, global: glam::DVec3) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let delta = self.agent_global().map_or(Vec3::X * 1.0e6, |here| (global - here).as_vec3());
        self.rlva_teleport_delta_allowed(delta, Uuid::nil())
    }

    pub fn rlva_allows_command(&self, command: &NetCommand, actor: Uuid) -> bool {
        let r = &self.rlva;
        if !r.enabled() {
            return true;
        }
        if !actor.is_nil() && r.controllers.blocked(actor) {
            return false;
        }
        let blocked = |name| r.has_except(name, actor);
        match command {
            NetCommand::AgentAnimation { anim, start: true } if *anim == super::super::radar::ANIM_AGENT_TYPE => r.can_send_typing_start(),
            NetCommand::CreateInventoryItem {
                kind: aurora_net::inventory::operations::NewItem::Landmark,
                ..
            } => !r.has("showloc"),
            NetCommand::OfferTeleport { to, .. } => to
                .iter()
                .all(|id| self.rlva_answers_teleport_request(*id) || self.rlva_can_offer_teleport(*id)),
            NetCommand::PickUpdate(pick) if r.has("showloc") => self
                .profiles
                .picks
                .get(&pick.id)
                .is_some_and(|old| old.pos_global == pick.pos_global),
            NetCommand::ClassifiedUpdate(classified) if r.has("showloc") => self
                .profiles
                .classifieds
                .get(&classified.id)
                .is_some_and(|old| old.pos_global == classified.pos_global),
            NetCommand::EditInventory { request, change } => {
                if actor == super::offers::ACTOR {
                    r.offers.permits(*request, change)
                } else {
                    self.rlva_inventory_mutation_allowed(change, actor)
                }
            }
            NetCommand::CopyInventoryItems(copies) => copies
                .iter()
                .all(|copy| self.rlva_inventory_move_allowed(copy.item, copy.parent, actor)),
            NetCommand::PreviewInventoryItem(item) => match item.asset_type {
                0 => !r.has("viewtexture"),
                7 => !r.has("viewnote"),
                10 => !r.has("viewscript"),
                _ => true,
            },
            // LLTaskNotecardBridge / LLTaskLSLBridge::openItem
            // (llpanelobjectinventory.cpp): nor those of a locked attachment.
            NetCommand::PreviewTaskNotecard { object, .. } => {
                !r.has("viewnote") && !self.objects.index_of_uuid(object).is_some_and(|i| self.rlva_locked_attachment(i))
            }
            NetCommand::LoadScript { target, .. } => {
                !r.has("viewscript")
                    && !self
                        .objects
                        .index_of_uuid(&target.object)
                        .is_some_and(|i| self.rlva_locked_attachment(i))
            }
            NetCommand::SendIm { to, .. } => self.rlva_allows_im("sendim", *to),
            NetCommand::SendImDialog { to, dialog: 0, .. } => self.rlva_allows_im("sendim", *to),
            NetCommand::SendImDialog { to, dialog: 17, .. } => self.rlva_can_send_session(*to),
            NetCommand::SendImDialog { to, dialog: 4, .. } | NetCommand::OfferInventory { to, .. } => r.allows("share", &to.to_string()),
            // A dialog shown before @sendchat (process_script_dialog_response,
            // llviewermessage.cpp): its public-chat answer is not sent.
            NetCommand::ScriptDialogReply { channel: 0, .. } => !r.has("sendchat"),
            NetCommand::ActivateGroup(group) => {
                !blocked("setgroup") || (actor == super::groups::ACTOR && r.has("setgroup") && r.groups.restore_allowed(*group))
            }
            NetCommand::GroupTitleUpdate { .. } => !blocked("setgroup"),
            // LLGroupActions::leave (llgroupactions.cpp): leaving the active
            // group would change it, which @setgroup forbids.
            NetCommand::LeaveGroup(group) => *group != self.groups.active || !blocked("setgroup"),
            // LLAgent::teleportViaLandmark: home stays allowed unless both
            // @tplm and @tploc restrict it
            NetCommand::TeleportHome => !(blocked("tplm") && blocked("tploc")) && !(self.agent.is_sitting() && blocked("unsit")),
            NetCommand::TeleportLandmark(_) => !blocked("tplm") && !(self.agent.is_sitting() && blocked("unsit")),
            NetCommand::TeleportTo { handle, position, .. } => {
                if r.stand_return == Some((*handle, *position)) {
                    return true;
                }
                let Some(main) = self.main_region else {
                    return !blocked("tploc") && !(self.agent.is_sitting() && blocked("unsit"));
                };
                let (x, y) = aurora_net::handle_to_origin(*handle);
                let (ox, oy) = aurora_net::handle_to_origin(main);
                let shift = Vec3::new((x as f64 - ox as f64) as f32, (y as f64 - oy as f64) as f32, 0.0);
                self.rlva_teleport_delta_allowed(*position + shift - self.agent.position, actor)
            }
            NetCommand::TeleportToRegion { name, position, .. } => {
                let handle = self
                    .regions
                    .values()
                    .find(|r| r.name.eq_ignore_ascii_case(name))
                    .map(|r| r.handle)
                    .or_else(|| {
                        self.map
                            .sims
                            .iter()
                            .find(|(_, r)| r.name.eq_ignore_ascii_case(name))
                            .map(|(&(x, y), _)| aurora_net::origin_to_handle(x * 256, y * 256))
                    });
                if let Some(handle) = handle
                    && let Some(main) = self.main_region
                {
                    let (x, y) = aurora_net::handle_to_origin(handle);
                    let (ox, oy) = aurora_net::handle_to_origin(main);
                    self.rlva_teleport_delta_allowed(
                        *position + Vec3::new((x as f64 - ox as f64) as f32, (y as f64 - oy as f64) as f32, 0.0) - self.agent.position,
                        actor,
                    )
                } else {
                    // RlvForceHandler<RLV_BHVR_TPTO> (rlvhandler.cpp) asks
                    // the map first and decides nothing before it answers:
                    // the teleport it resolves to is checked then
                    // (RlvHandler::onTeleportCallback, the TeleportTo the
                    // reply becomes, `App::send_deferred_teleport`).
                    true
                }
            }
            NetCommand::RequestSit { target, offset, .. } => {
                let limit = r.distance_limit("sittp", 1.5, actor).min(r.distance_limit("fartouch", 1.5, actor));
                !r.has("sit")
                    && self
                        .objects
                        .index_of_uuid(target)
                        .and_then(|i| self.objects.get(i))
                        .is_some_and(|o| o.pcode == aurora_prim::params::LL_PCODE_VOLUME)
                    && !(self.agent.is_sitting() && r.has("standtp"))
                    && !(self.agent.is_sitting() && r.has("unsit"))
                    && (!actor.is_nil()
                        || !limit.is_finite()
                        || self
                            .objects
                            .index_of_uuid(target)
                            .and_then(|i| crate::scene::Scene::object_transform_unshifted(self, i, std::time::Instant::now()))
                            // RlvActions::canSit adds the picked world-axis
                            // offset directly, including for a rotated child.
                            .is_some_and(|(pos, _, _)| (pos + *offset).distance(self.agent.position) < limit))
            }
            NetCommand::OneShotControl(flags) => {
                (flags & control::STAND_UP == 0 || !blocked("unsit"))
                    && (flags & control::SIT_ON_GROUND == 0
                        || (*flags == control::SIT_ON_GROUND && r.sitting.ground_released(actor))
                        || (!blocked("sit") && !(self.agent.is_sitting() && blocked("unsit"))))
            }
            // One message for every kind of run (LLAgent::sendWalkRun): it is
            // refused only when both restrictions leave nothing that may run.
            // Each setter checks its own (set_always_run, walk_tap, running).
            NetCommand::SetAlwaysRun(true) => !(blocked("alwaysrun") && blocked("temprun")),
            NetCommand::BuyObject { .. } => !blocked("buy"),
            NetCommand::PayObject { .. } => !blocked("buy"),
            NetCommand::PayResident { .. } => !blocked("pay"),
            // LLToolPlacer::placeObject and LLToolDragAndDrop::dropObject.
            NetCommand::Build(aurora_net::build::BuildCmd::Add(_) | aurora_net::build::BuildCmd::RezInventory(_)) => {
                !blocked("rez") && !blocked("interact")
            }
            NetCommand::Build(command) => self.rlva_allows_build(command),
            NetCommand::ScriptAnswer { task_id, questions, .. } => self.rlva_script_permissions(*task_id, *questions).0 == *questions,
            // lure_callback (llviewermessage.cpp) asks RLVa nothing when the
            // offer is accepted: @tplure and @unsit were asked when it came
            // (LLIMProcessing::processNewMessage), and refused it there.
            NetCommand::AcceptLure { lure_id, godlike } => {
                *godlike
                    || self
                        .teleport_invites
                        .list
                        .iter()
                        .any(|v| matches!(v.kind, super::super::teleport_invites::InviteKind::Offer { lure, .. } if lure == *lure_id))
            }
            // The viewer's own bridge comes off whatever the locks
            // (FSLSLBridge::canDetach in
            // LLVOAvatarSelf::detachAttachmentIntoInventory,
            // indra/newview/llvoavatarself.cpp).
            NetCommand::DetachAttachments(items) => items.iter().all(|item| {
                super::super::bridge::viewer_bridge(self, *item, super::super::bridge::POINT) || self.rlva_can_remove(*item, actor)
            }),
            NetCommand::DetachObject { handle, local_id } => self
                .objects
                .index_of(&super::super::objects::ObjKey {
                    region: *handle,
                    local_id: *local_id,
                })
                .and_then(|i| self.objects.get(i))
                .is_some_and(|object| {
                    self.rlva_own_attachment_root(object.full_id) == Some(object.full_id)
                        && self.rlva_can_remove(
                            object.attachment_item_id().filter(|id| !id.is_nil()).unwrap_or(object.full_id),
                            actor,
                        )
                }),
            // Rez is sent as ADD while RLVa is enabled. The watchdog replaces
            // only removable predecessors once the actual point is known.
            // The bridge goes back on Center 2 past them too (the fRlvForce
            // of LLAttachmentsMgr::addAttachmentRequest,
            // indra/newview/llattachmentsmgr.cpp).
            NetCommand::RezAttachments(items) => {
                let worn = self.rlva_worn_attachment_points();
                items.iter().all(|item| {
                    super::super::bridge::viewer_bridge(self, item.item_id, item.point) || self.rlva_rez_allowed(item, &worn, actor)
                })
            }
            NetCommand::UpdateOutfit { change, .. } => {
                let Some(cof) = super::super::appearance::cof(&self.inventory) else {
                    return true;
                };
                let old = super::super::appearance::folder_links(&self.inventory, cof);
                if change.folder == cof {
                    self.rlva_outfit_allowed(&old, &change.links, actor)
                } else {
                    change
                        .cof
                        .as_ref()
                        .is_none_or(|(_, links)| self.rlva_outfit_allowed(&old, links, actor))
                }
            }
            NetCommand::Touch { local_id } => self.main_region.is_some_and(|h| self.rlva_can_touch(h, *local_id)),
            NetCommand::ObjectGrab {
                handle, local_id, surface, ..
            } => self.rlva_can_touch_at(
                *handle,
                *local_id,
                (surface.face >= 0)
                    .then(|| self.region_offset(*handle).map(|shift| surface.position + shift))
                    .flatten(),
            ),
            NetCommand::ObjectGrabUpdate { object, surface, .. } => self
                .objects
                .index_of_uuid(object)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|o| {
                    self.rlva_can_touch_at(
                        o.key.region,
                        o.key.local_id,
                        (surface.face >= 0)
                            .then(|| self.region_offset(o.key.region).map(|shift| surface.position + shift))
                            .flatten(),
                    )
                }),
            _ => true,
        }
    }

    /// One attachment of a rez request: it may go on its point, does not
    /// replace a locked attachment there (RLVaWearReplaceUnlocked off), and a
    /// worn one is not moved off a point it is locked on.
    fn rlva_rez_allowed(&self, item: &aurora_net::AttachRequest, worn: &std::collections::HashMap<Uuid, u8>, actor: Uuid) -> bool {
        let watchdog = actor == super::watchdog::ACTOR;
        let at = if item.point == 0 {
            self.inventory.items.get(&item.item_id).map_or(0, |it| (it.flags & 0xff) as u8)
        } else {
            item.point
        };
        self.rlva_can_add(item.item_id, item.point, actor)
            && (item.add
                || self.rlva.preferences.replace_unlocked
                || watchdog
                || !worn
                    .iter()
                    .any(|(old, point)| *old != item.item_id && at != 0 && *point == at && !self.rlva_can_remove(*old, actor)))
            && (watchdog
                || worn
                    .get(&item.item_id)
                    .is_none_or(|old| at == 0 || at == *old || self.rlva_can_remove(item.item_id, actor)))
    }

    /// LLAgentWearables::userRemoveMultipleAttachments (llagentwearables.cpp)
    /// and LLAttachmentsMgr::addAttachmentRequest (llattachmentsmgr.cpp) with
    /// RLVa: a lock takes its own attachments out of a detach or attach
    /// request and says nothing, the others go through. The outfit sync sends
    /// such requests on its own (what is worn and not in the outfit, what is
    /// in the outfit and not worn): refusing the whole request left the
    /// unlocked attachments on and posted a notice for nothing the user did.
    /// Returns false when nothing is left to send. The watchdog's own
    /// requests are exact and stay whole (`rlva_allows_command`).
    pub fn rlva_prepare_attachments(&self, command: &mut NetCommand, actor: Uuid) -> bool {
        if !self.rlva.enabled() || actor == super::watchdog::ACTOR || (!actor.is_nil() && self.rlva.controllers.blocked(actor)) {
            return true;
        }
        match command {
            NetCommand::DetachAttachments(items) => {
                items.retain(|item| self.rlva_can_remove(*item, actor));
                !items.is_empty()
            }
            NetCommand::RezAttachments(items) => {
                let worn = self.rlva_worn_attachment_points();
                items.retain(|item| self.rlva_rez_allowed(item, &worn, actor));
                !items.is_empty()
            }
            _ => true,
        }
    }

    /// RlvUIEnabler::onToggleMovement (rlvui.cpp): when @fly, @alwaysrun or
    /// @temprun comes or goes, a flight under way lands under @fly. Only
    /// then: a flight the issuer of @fly forced itself (@fly:true=force)
    /// goes on while its @fly=n holds.
    pub fn rlva_movement_tick(&mut self) {
        let rules = ["fly", "alwaysrun", "temprun"]
            .iter()
            .enumerate()
            .filter(|(_, name)| self.rlva.has(name))
            .fold(0u8, |bits, (i, _)| bits | (1 << i));
        if std::mem::replace(&mut self.rlva.movement_rules, rules) != rules && rules & 1 != 0 {
            self.agent.flying = false;
        }
    }

    /// Queries requiring world state and force commands are consumed in their
    /// original order. The issuing object may bypass only its own locks.
    pub fn rlva_pending(&mut self) -> Vec<(Uuid, NetCommand)> {
        self.rlva_relative_tick(std::time::Instant::now());
        let mut commands = Vec::new();
        if self.rlva.appearance_busy || !self.rlva.appearance.is_empty() {
            return commands;
        }
        let mut pending = std::mem::take(&mut self.rlva.pending).into_iter();
        while let Some((source, c)) = pending.next() {
            if !self.rlva.enabled() {
                break;
            }
            // RlvHandler::processCommand (rlvhandler.cpp) runs a force or a
            // query when it is received: it reads the rules of that moment,
            // whatever was released while it waited here. A lock set since
            // still holds too, at the send boundary or just above: stricter
            // than the Legacy Viewer there, never looser.
            // A force with an effect of its own, sent nowhere (the sky, a
            // take-off), has no boundary after it: the lock another object
            // set since refuses it here.
            if c.param == super::Param::Force
                && ((c.behaviour.starts_with("setenv_") && self.rlva.has_except("setenv", source))
                    || (c.behaviour == "fly" && super::option_bool(&c.option) != Some(false) && self.rlva.has_except("fly", source)))
            {
                continue;
            }
            let live = self.rlva.rewind(&c);
            let go_on = self.rlva_run_queued(source, c, &mut pending, &mut commands);
            self.rlva.resume(live);
            if !go_on {
                break;
            }
        }
        commands
    }

    /// One queued command. False stops the queue: the commands after it wait
    /// for what this one started.
    fn rlva_run_queued(
        &mut self,
        source: Uuid,
        c: super::Command,
        pending: &mut std::vec::IntoIter<(Uuid, super::Command)>,
        commands: &mut Vec<(Uuid, NetCommand)>,
    ) -> bool {
        if !self.rlva.controllers.accepts(source, c.param) {
            return true;
        }
        if self.rlva_environment_command(source, &c) {
            return true;
        }
        if c.behaviour.starts_with("getcam_")
            || c.behaviour.starts_with("setcam_")
            || c.behaviour.starts_with("getdebug_")
            || c.behaviour.starts_with("setdebug_")
            || matches!(c.behaviour.as_str(), "adjustheight" | "getheightoffset")
        {
            if self.rlva.viewer.len() < 256 {
                self.rlva.viewer.push((source, c));
            }
            return true;
        }
        if let super::Param::Reply(channel) = c.param {
            // (RlvHandler::processReplyCommand reads no option for
            // @getgroup and @getsitid: one given changes nothing)
            let text = match c.behaviour.as_str() {
                "getgroup" => Some(
                    self.groups
                        .groups
                        .iter()
                        .find(|g| g.id == self.groups.active)
                        .map_or("none", |g| g.name.as_str())
                        .to_owned(),
                ),
                "getsitid" => Some(self.rlva_sit_id().to_string()),
                name if super::QUERIES.contains(&name) => self.rlva_inventory_query(&c, source),
                _ => Some(String::new()),
            };
            if let Some(text) = text {
                self.rlva.reply(channel, &c.behaviour, text);
            } else {
                self.rlva.pending.push((source, c));
                self.rlva.pending.extend(pending.by_ref());
                return false;
            }
            return true;
        }
        if c.behaviour == "detachme"
            && c.option.is_empty()
            && let Some(item) = self.rlva_attachment_item(source)
            && let Some(key) = self.rlva_temporary_attachment(item)
        {
            if self.rlva_can_remove(item, source) {
                commands.push((
                    source,
                    NetCommand::DetachObject {
                        handle: key.region,
                        local_id: key.local_id,
                    },
                ));
            }
            return true;
        }
        if super::inventory::force_kind(&c.behaviour).is_some() {
            if !self.rlva_inventory_force(source, &c) {
                self.rlva.pending.push((source, c));
            }
            // The next force command of the same object joins this one's
            // COF update (RlvForceWear::done runs once per batch); any
            // other command waits for the update, as before.
            let joins = pending
                .as_slice()
                .first()
                .is_some_and(|(next, c)| *next == source && super::inventory::force_kind(&c.behaviour).is_some());
            if !self.rlva.pending.is_empty() || (!self.rlva.appearance.is_empty() && !joins) {
                self.rlva.pending.extend(pending.by_ref());
                return false;
            }
            return true;
        }
        if c.behaviour == "setgroup" {
            commands.extend(self.rlva_group_force(source, &c, std::time::Instant::now()));
            return true;
        }
        let command = match c.behaviour.as_str() {
            "unsit" if c.option.is_empty() && self.agent.is_sitting() => Some(NetCommand::OneShotControl(control::STAND_UP)),
            // RlvForceHandler<RLV_BHVR_SITGROUND>::onCommand reads no
            // option; RlvForceHandler<RLV_BHVR_SIT> (rlvhandler.cpp) hands
            // it the null key, which no option is too (LLUUID::set).
            "sitground" => self.rlva_force_ground_sit(source),
            "sit" if c.option.is_empty() => self.rlva_force_ground_sit(source),
            "sit" => match Uuid::parse_str(&c.option) {
                Ok(id) if id.is_nil() => self.rlva_force_ground_sit(source),
                Ok(id) => self
                    .objects
                    .index_of_uuid(&id)
                    .and_then(|i| self.objects.get(i))
                    .map(|o| NetCommand::RequestSit {
                        handle: o.key.region,
                        target: o.full_id,
                        offset: Vec3::ZERO,
                    }),
                Err(_) => None,
            },
            "setrot" => {
                if let Ok(angle) = c.option.parse::<f32>()
                    && angle.is_finite()
                {
                    self.agent.yaw = std::f32::consts::FRAC_PI_2 - angle;
                }
                None
            }
            "fly" => {
                let flying = if c.option.is_empty() {
                    Some(true)
                } else {
                    super::option_bool(&c.option)
                };
                // RlvForceHandler<RLV_BHVR_FLY> (rlvhandler.cpp) takes off
                // through LLAgent::setFlying (llagent.cpp), which refuses a
                // takeoff seated or where the land allows no flight
                // (LLAgent::canFly: no parcel known yet is no flight), and
                // lets a flight under way land.
                if let Some(flying) = flying
                    && flying != self.agent.flying
                    && (!flying
                        || (!self.rlva.has_except("fly", source)
                            && !self.agent.is_sitting()
                            && crate::autopilot::land_allows_flight(self, true, false)))
                {
                    self.agent.flying = flying;
                }
                None
            }
            "tpto" => teleport(&c.option),
            _ => None,
        };
        if let Some(mut command) = command
            && self.rlva_normalize_teleport(&mut command)
        {
            if self.rlva_allows_command(&command, source) {
                // LLAgent::teleportViaLocation: a teleport forced by @tpto
                // to a position cannot be cancelled
                // (RlvHandler::setCanCancelTp(false)). One to a region name
                // starts from RlvHandler::onTeleportCallback, once the map
                // answered, where no command is current: it can be.
                if c.behaviour == "tpto" && matches!(command, NetCommand::TeleportTo { .. }) {
                    self.tp_cancel.lock_for_rlva();
                }
                commands.push((source, command));
            } else if c.behaviour == "tpto" {
                // LLAgent::teleportViaLocation(LookAt): a refused teleport
                // says so (RlvUtil::notifyBlocked, Blocked::Teleport).
                self.system_notice(
                    aurora_i18n::tr!("rlva-blocked-teleport"),
                    super::super::notice_topics::Topic::Warning,
                );
            }
        }
        true
    }
}

fn teleport(option: &str) -> Option<NetCommand> {
    // RlvForceHandler<TPTO> retains double precision until the region origin
    // is removed and rotates the look vector by the optional radian angle.
    let mut options = option.split(';');
    let destination = options.next()?;
    let look_at = match options.next() {
        Some(angle) => {
            let angle = angle.parse::<f32>().ok()?;
            if !angle.is_finite() {
                return None;
            }
            let (sin, cos) = angle.sin_cos();
            Vec3::new(cos, sin, 0.0)
        }
        None => Vec3::ZERO,
    };
    let parts: Vec<_> = destination.split('/').collect();
    let (name, coordinates) = match parts.as_slice() {
        [x, y, z] => (None, [*x, *y, *z]),
        [region, x, y, z] if !region.is_empty() => (Some(*region), [*x, *y, *z]),
        _ => return None,
    };
    let p = glam::DVec3::new(
        coordinates[0].parse().ok()?,
        coordinates[1].parse().ok()?,
        coordinates[2].parse().ok()?,
    );
    if !p.is_finite() {
        return None;
    }
    if let Some(name) = name {
        Some(NetCommand::TeleportToRegion {
            request: Uuid::new_v4(),
            name: name.to_owned(),
            position: p.as_vec3().is_finite().then_some(p.as_vec3())?,
            look_at,
        })
    } else {
        if p.x < 0.0 || p.y < 0.0 || p.x >= u32::MAX as f64 || p.y >= u32::MAX as f64 || p.z.abs() > f32::MAX as f64 {
            return None;
        }
        let x = (p.x as u32 / 256) * 256;
        let y = (p.y as u32 / 256) * 256;
        Some(NetCommand::TeleportTo {
            handle: aurora_net::origin_to_handle(x, y),
            position: Vec3::new((p.x - x as f64) as f32, (p.y - y as f64) as f32, p.z as f32),
            look_at,
        })
    }
}

#[cfg(test)]
mod teleport_tests {
    use super::*;

    /// LLWorldEnableTeleportHome greys the menu entry under either lock;
    /// the teleport and the Home buttons give way only under both.
    #[test]
    fn rlva_teleport_home_entry_is_greyed_under_either_teleport_lock() {
        for (rules, entry, teleport) in [
            ("@clear", true, true),
            ("@tplm=n", false, true),
            ("@tploc=n", false, true),
            ("@tplm=n,tploc=n", false, false),
        ] {
            let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            assert!(w.rlva_home_entry_enabled() && w.rlva_can_teleport_home(), "RLVa off");
            w.rlva_configure(true);
            assert!(w.rlva.receive(&aurora_net::ChatMessage {
                from_name: "Collar".to_owned(),
                source_id: Uuid::from_u128(9),
                owner_id: Uuid::nil(),
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                region: None,
                position: Vec3::ZERO,
                message: rules.to_owned(),
            }));
            assert_eq!(w.rlva_home_entry_enabled(), entry, "{rules}: the menu entry");
            assert_eq!(w.rlva_can_teleport_home(), teleport, "{rules}: the teleport");
        }
    }

    #[test]
    fn rlva_tpto_preserves_global_fractions_and_named_region_heading() {
        let Some(NetCommand::TeleportTo { handle, position, look_at }) = teleport("10000000.125/20000000.75/42.5;1.57079632679") else {
            panic!("global teleport")
        };
        let (x, y) = aurora_net::handle_to_origin(handle);
        assert_eq!(x as f64 + position.x as f64, 10000000.125);
        assert_eq!(y as f64 + position.y as f64, 20000000.75);
        assert_eq!(position.z, 42.5);
        assert!(look_at.abs_diff_eq(Vec3::Y, 1e-6));
        assert!(
            matches!(teleport("region/1.25/2.5/40;-1.57079632679"), Some(NetCommand::TeleportToRegion { position, look_at, .. }) if position == Vec3::new(1.25, 2.5, 40.0) && look_at.abs_diff_eq(Vec3::NEG_Y, 1e-6))
        );
        assert!(matches!(teleport("1/2/3"), Some(NetCommand::TeleportTo { look_at, .. }) if look_at == Vec3::ZERO));
        assert!(
            matches!(teleport("region/-1.25/300.5/40"), Some(NetCommand::TeleportToRegion { position, .. }) if position == Vec3::new(-1.25, 300.5, 40.0))
        );
        for invalid in [
            "1/2/3;bad",
            "1/2/3;",
            "1/2/3;nan",
            "region/1/2/3;inf",
            "nan/2/3",
            "1/inf/3",
            "4294967296/2/3",
            "region/1/2/1e100",
            "region/1/2",
        ] {
            assert!(teleport(invalid).is_none(), "{invalid}");
        }
    }
}
