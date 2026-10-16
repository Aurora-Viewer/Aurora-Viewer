//! The object menu entries that take more than one message: "Return",
//! "Drop", "Put on", the script actions of a selection, "Block particle
//! owner", the favorites of our attachments, "Take multiple" and "Link",
//! and the right click's own surface for "Touch" (menu_object.xml,
//! menu_attachment_self.xml; handlers of indra/newview/llviewermenu.cpp,
//! originally LGPL 2.1).
use super::context_menu::linkset_root;
use super::*;
use crate::autopilot::{Arrival, AutoPilot};
use crate::build::script_queue::{self, QueueKind, ScriptQueue};
use crate::ui::context::{AttachChoice, ObjectFacts, Question, flags};
use crate::world::notifications::{Data, Kind};
use crate::world::objects::ObjKey;
use aurora_net::build::{BuildCmd, derez};
use uuid::Uuid;

/// ATTACHMENT_ADD: added to the point, the attachment goes on beside what
/// the point holds.
const ATTACHMENT_ADD: u8 = 0x80;
/// LL_PCODE_VOLUME: only prims are worn (object_selected_and_point_valid).
const PCODE_VOLUME: u8 = 9;
/// The attachment limit when the account benefits do not give one.
const DEFAULT_ATTACHMENT_LIMIT: usize = 38;

/// What the object menus wait for between frames.
#[derive(Default)]
pub(super) struct ObjectMenu {
    /// The question asked before an action, and its "ignore" box.
    confirm: Option<(Confirm, bool)>,
    /// The script action under way and its window.
    queue: Option<ScriptQueue>,
    /// Picked with the right click: the owner of the particle in front
    /// (LLPickInfo::mParticleOwnerID) and the touched surface of the object
    /// (the SurfaceInfo of send_ObjectGrab_message).
    pub particle_owner: Option<Uuid>,
    touch: Option<(ObjKey, aurora_net::TouchSurface)>,
}

enum Confirm {
    /// The objects to return, saved when asked (LLObjectReturn keeps
    /// mReturnableObjects: the dialog drops the selection).
    Return {
        handle: aurora_net::RegionHandle,
        local_ids: Vec<u32>,
    },
    Drop(ObjKey),
    Scripts(QueueKind, Vec<(Uuid, String)>),
}

impl App {
    /// The right click opened the menu on `target`: what the entries of
    /// the object menus read from the pick itself.
    pub(super) fn object_menu_pick(&mut self, picked: Option<usize>, point: Vec3) {
        let ray = self
            .gfx
            .as_ref()
            .and_then(|g| g.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1));
        let m = &mut self.object_menu;
        m.particle_owner = None;
        m.touch = None;
        let Some((origin, dir)) = ray else {
            return;
        };
        // a particle nearer than what was hit (LLPickInfo::fetchResults)
        let max = if picked.is_some() || point != Vec3::ZERO {
            (point - origin).length()
        } else {
            self.settings.draw_distance
        };
        m.particle_owner = self
            .scene
            .particles
            .pick(origin, dir, max)
            .and_then(|source| self.world.objects.index_of_uuid(&source))
            .and_then(|i| self.world.objects.get(i))
            .map(|o| o.owner_id)
            .filter(|owner| !owner.is_nil());
        if let Some(idx) = picked
            && let Some(o) = self.world.objects.get(idx)
            && !o.is_avatar()
        {
            m.touch = self.scene.touch_surface(&self.world, idx, (origin, dir)).map(|s| (o.key, s));
        }
    }

    /// The facts of the object menus beyond the clicked linkset's flags:
    /// the selection kept by the right click, wearing, scripts, favorites,
    /// dropping, an animesh skeleton.
    pub(super) fn object_menu_facts(&self, idx: usize, f: &mut ObjectFacts) {
        let w = &self.world;
        if w.objects.get(idx).is_none() {
            return;
        }
        let root = linkset_root(w, idx);
        let flags_of = |i: usize| w.objects.get(i).map_or(0, |o| o.update_flags);
        let root_id = w.objects.get(root).map(|r| r.full_id).unwrap_or_default();
        if let Some(props) = self.build.props.get(&root_id).or_else(|| self.interactions.props.get(&root_id)) {
            f.touch_name = props.touch_name.clone();
            f.sit_name = props.sit_name.clone();
        }
        // the selection a right click on a selected object keeps
        let edit_linked = self.settings.build.edit_linked;
        let kept = crate::build::BuildTool::unit_of(w, idx, edit_linked).is_some_and(|u| self.build.is_selected(&u));
        let roots = if kept { self.build.roots(w) } else { vec![root] };
        f.roots = roots.len().max(1);
        if roots.len() > 1 {
            // Take: one root we may take (enable_take); Take copy: all
            // copyable (enable_object_take_copy); Delete: one deletable
            let each: Vec<ObjectFacts> = roots
                .iter()
                .map(|&r| {
                    let family: Vec<u32> = crate::build::family(w, r).into_iter().map(flags_of).collect();
                    ObjectFacts::from_flags(flags_of(r), 0, flags_of(r), &family, f.stand, false)
                })
                .collect();
            f.take = !f.stand && each.iter().any(|e| e.take);
            f.take_copy = each.iter().all(|e| e.take_copy);
            f.delete = each.iter().any(|e| e.delete);
        }
        f.link = !edit_linked
            && roots.len() >= 2
            && roots
                .iter()
                .any(|&r| flags_of(r) & flags::OBJECT_MODIFY != 0 && flags_of(r) & flags::AFFECTS_NAVMESH == 0);
        // EditableSelected: a prim we may modify among those selected
        let prims: Vec<usize> = if kept {
            self.build.highlighted(w).into_iter().map(|(i, _)| i).collect()
        } else if edit_linked {
            vec![idx]
        } else {
            crate::build::family(w, root)
        };
        f.scripts = prims.iter().any(|&i| flags_of(i) & flags::OBJECT_MODIFY != 0);
        let wearer = crate::ui::context::wearer(w, idx);
        // object_is_wearable / object_selected_and_point_valid
        let r = flags_of(root);
        // an avatar seated on it is one of its children
        let seated_on = crate::build::family(w, root)
            .iter()
            .any(|&i| w.objects.get(i).is_some_and(|c| c.is_avatar()));
        f.wear = wearer.is_none()
            && roots.len() == 1
            && w.objects.get(root).is_some_and(|o| o.pcode == PCODE_VOLUME)
            && r & flags::OBJECT_YOU_OWNER != 0
            && r & flags::OBJECT_MOVE != 0
            && r & flags::AFFECTS_NAVMESH == 0
            && !seated_on
            && !f.stand
            && w.worn_attachment_points().len() < self.attachment_limit();
        if wearer == Some(w.agent_id) {
            let item = f.attachment_item.map(|id| linked_item(w, id));
            f.favorite = item.and_then(|id| w.inventory.items.get(&id)).map(|it| it.favorite);
            // Attachment.EnableDrop: the item is in our inventory and we may build here
            f.drop = item.is_some_and(|id| w.inventory.items.contains_key(&id)) && self.agent_can_build();
        }
        f.animesh = wearer
            .is_none()
            .then(|| crate::scene::animesh::owner(&w.objects, idx))
            .flatten()
            .map(|(id, _)| id);
    }

    /// LLAgentBenefits::getAttachmentLimit, from the login's account benefits.
    fn attachment_limit(&self) -> usize {
        self.world
            .login
            .as_ref()
            .map(|l| l.raw["account_level_benefits"]["attachment_limit"].as_i32())
            .filter(|n| *n > 0)
            .map_or(DEFAULT_ATTACHMENT_LIMIT, |n| n as usize)
    }

    /// allowAgentBuild on the parcel we stand on.
    fn agent_can_build(&self) -> bool {
        match (self.world.main().and_then(|r| r.info.as_ref()), self.world.parcel.as_deref()) {
            (Some(region), Some(parcel)) => ui::parcel_icons::agent_can_build(&ui::parcel_icons::ParcelState {
                region_flags: region.region_flags,
                parcel,
                agent_id: self.world.agent_id,
                group: self.world.groups.group(&parcel.group_id),
                health: self.world.health,
            }),
            // nothing known yet: the simulator decides
            _ => true,
        }
    }

    /// "Put on ▸ Attach" and "Attach HUD": every point of avatar_lad.xml
    /// under its localized name (LLVOAvatarSelf::buildMenus).
    pub(super) fn attach_choices(&self) -> Vec<AttachChoice> {
        let lib = &self.world.avatar_lib;
        let mut out: Vec<AttachChoice> = lib
            .attach_points
            .iter()
            .map(|(&id, ap)| AttachChoice {
                id,
                label: lib
                    .attach_names
                    .get(&id)
                    .map(|n| crate::ui::appearance::items::point_name(n).to_string())
                    .unwrap_or_default(),
                group: ap.group,
                pie_slice: ap.pie_slice,
            })
            .collect();
        out.sort_by_key(|c| (c.group, c.pie_slice, c.id));
        out
    }

    /// Object.Touch: ObjectGrab then ObjectDeGrab with the surface the
    /// right click picked (handle_object_touch).
    pub(super) fn menu_touch(&mut self, local_id: u32) {
        let Some((key, surface)) = self.object_menu.touch.filter(|(k, _)| k.local_id == local_id) else {
            self.send(NetCommand::Touch { local_id });
            return;
        };
        self.send(NetCommand::ObjectGrab {
            handle: key.region,
            local_id,
            offset: Vec3::ZERO,
            surface,
        });
        self.send(NetCommand::ObjectRelease {
            handle: key.region,
            local_id,
            surface,
        });
    }

    /// Object.Return: the roots of the selection, asked first.
    pub(super) fn ask_return(&mut self, id: Uuid) {
        let Some(idx) = self.world.objects.index_of_uuid(&id) else { return };
        self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
        self.flush_build();
        // get_derezzable_objects(DRD_RETURN_TO_OWNER): roots that are not
        // attachments, all in one region (AcquireErrorObjectSpan)
        let roots: Vec<(aurora_net::RegionHandle, u32)> = self
            .build
            .roots(&self.world)
            .into_iter()
            .filter(|&r| crate::ui::context::wearer(&self.world, r).is_none())
            .filter_map(|r| self.world.objects.get(r).map(|o| (o.key.region, o.key.local_id)))
            .collect();
        let Some(&(handle, _)) = roots.first() else { return };
        if roots.iter().any(|(h, _)| *h != handle) {
            self.world.notifications.push(
                Kind::Alert,
                aurora_i18n::tr!("ui-context-return"),
                aurora_i18n::tr!("ui-context-return-span-regions"),
                Data::None,
            );
            return;
        }
        let local_ids = roots.into_iter().map(|(_, l)| l).collect();
        let ask = Confirm::Return { handle, local_ids };
        if self.settings.confirm_return_to_owner {
            self.object_menu.confirm = Some((ask, false));
        } else {
            self.object_menu_confirmed(ask);
        }
    }

    /// Attachment.Drop: asked first.
    pub(super) fn ask_drop(&mut self, key: ObjKey) {
        if self.settings.confirm_attachment_drop {
            self.object_menu.confirm = Some((Confirm::Drop(key), false));
        } else {
            self.object_menu_confirmed(Confirm::Drop(key));
        }
    }

    /// Tools.SelectedScriptAction: the scripted prims of the selection we
    /// may modify (queue_actions), asked first, then the queue.
    pub(super) fn ask_scripts(&mut self, id: Uuid, kind: QueueKind) {
        let Some(idx) = self.world.objects.index_of_uuid(&id) else { return };
        self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
        self.flush_build();
        let prims: Vec<(Uuid, u32, String)> = self
            .build
            .highlighted(&self.world)
            .into_iter()
            .filter_map(|(i, _)| self.world.objects.get(i))
            .map(|o| {
                let name = self
                    .build
                    .props
                    .get(&o.full_id)
                    .map(|p| p.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| aurora_i18n::tr!("common-object").to_string());
                (o.full_id, o.update_flags, name)
            })
            .collect();
        match script_queue::queue_objects(&prims) {
            Err(scripted) => {
                self.world
                    .notifications
                    .push(Kind::Alert, kind.title(), kind.refusal(scripted), Data::None);
            }
            Ok(objects) if self.settings.confirm_script_modify => {
                self.object_menu.confirm = Some((Confirm::Scripts(kind, objects), false));
            }
            Ok(objects) => self.object_menu_confirmed(Confirm::Scripts(kind, objects)),
        }
    }

    fn object_menu_confirmed(&mut self, c: Confirm) {
        match c {
            Confirm::Return { handle, local_ids } => {
                // derez_objects(DRD_RETURN_TO_OWNER, null): one transaction,
                // the agent's active group
                self.build.send(BuildCmd::DeRez {
                    handle,
                    local_ids,
                    destination: derez::RETURN_TO_OWNER,
                    destination_id: Uuid::nil(),
                    group_id: self.world.groups.active,
                });
                self.world.ui_sounds.push(UiSound::ObjectRezOut);
                if !self.build.open {
                    self.build.deselect_all(&self.world);
                }
                self.flush_build();
            }
            Confirm::Drop(key) => self.drop_attachment(key),
            Confirm::Scripts(kind, objects) => self.object_menu.queue = Some(ScriptQueue::new(kind, objects)),
        }
    }

    /// callback_attachment_drop: ObjectDrop of the attachment's root.
    fn drop_attachment(&mut self, key: ObjKey) {
        let Some(idx) = self.world.objects.index_of(&key) else { return };
        if ui::context::wearer(&self.world, idx) != Some(self.world.agent_id) {
            return;
        }
        let Some(root) = self.world.objects.get(linkset_root(&self.world, idx)).map(|o| o.key) else {
            return;
        };
        self.release_object_hold();
        self.build.send(BuildCmd::DropAttachment {
            handle: root.region,
            local_ids: vec![root.local_id],
        });
        self.build.deselect_all(&self.world);
        self.flush_build();
    }

    /// Object.AttachToAvatar / AttachAddToAvatar: walk to the object
    /// (confirmReplaceAttachment), then onNearAttachObject attaches it.
    /// "Wear" replaces on the object's own point; "Add" and the points of
    /// the submenus add (LLSelectMgr::sendAttach).
    pub(super) fn wear_object(&mut self, id: Uuid, point: u8, add: bool) {
        let Some(idx) = self.world.objects.index_of_uuid(&id) else { return };
        let root = linkset_root(&self.world, idx);
        let Some(o) = self.world.objects.get(root) else { return };
        let wire = if point == 0 && !add { 0 } else { point | ATTACHMENT_ADD };
        let object = o.full_id;
        let Some((pos, _, _)) = Scene::object_transform(&self.world, root, Instant::now(), 0) else {
            return;
        };
        // MIN_STOP_DISTANCE 1 m, ARM_LENGTH 0.5 m, SCALE_FUDGE 1.5
        let stop = (1.5 * o.scale.max_element() + 0.5).max(1.0);
        let agent = &self.world.agent;
        // in front of the object, at the avatar's height (Aurora's walk
        // does not fly: flying or seated, it attaches from here)
        let mut spot = pos - (pos - agent.position).normalize_or_zero() * 0.5;
        spot.z = agent.position.z;
        let walk = AutoPilot::start_to(agent.position, spot, stop, Arrival::Attach { object, point: wire }, Instant::now());
        if agent.flying || agent.is_sitting() || walk.reached(agent.position) {
            self.attach_object(object, wire);
        } else {
            self.stop_autopilot(true);
            self.autopilot = Some(walk);
        }
    }

    /// The finish callback of a walk (LLAgent::stopAutoPilot).
    pub(super) fn autopilot_arrived(&mut self, arrival: Arrival, success: bool, user_cancel: bool) {
        match arrival {
            Arrival::Attach { object, point } => {
                if success {
                    self.attach_object(object, point);
                } else if user_cancel {
                    // CancelledAttach (a notifytip)
                    self.world
                        .notifications
                        .push(Kind::Info, aurora_i18n::tr!("ui-context-attach-cancelled"), "", Data::None);
                }
            }
        }
    }

    /// LLSelectMgr::sendAttach: ObjectAttach of the root with its rotation;
    /// out of build mode the selection goes (the simulator stops sending
    /// its properties).
    fn attach_object(&mut self, object: Uuid, point: u8) {
        let Some(idx) = self.world.objects.index_of_uuid(&object) else {
            return;
        };
        let Some(o) = self.world.objects.get(linkset_root(&self.world, idx)) else {
            return;
        };
        self.build.send(BuildCmd::Attach {
            handle: o.key.region,
            point,
            objects: vec![(o.key.local_id, o.rotation)],
        });
        if !self.build.open {
            self.build.deselect_all(&self.world);
        }
        self.flush_build();
    }

    /// Particle.Mute: the particle owner blocked, or unblocked when they
    /// already are; a new block shows the block list (showPanelAndSelect).
    pub(super) fn block_particle_owner(&mut self, owner: Uuid) {
        let name = self.world.legacy_name(&owner).unwrap_or_else(|| self.world.social.name_of(&owner));
        let blocked = self.world.mutes.is_muted_id(&owner);
        match self.world.toggle_block_avatar(owner, &name) {
            Err(e) => self.world.system_message(e),
            Ok(()) if !blocked => {
                self.panels.people = true;
                self.panels.people_tab = crate::ui::people::TAB_BLOCKED;
            }
            Ok(()) => {}
        }
    }

    /// Object.SetFavorite on the item our attachment comes from.
    pub(super) fn attachment_favorite(&mut self, item: Uuid) {
        let item = linked_item(&self.world, item);
        self.apply_appearance_action(crate::world::appearance::Action::Favorite(item));
    }

    /// Tools.Link on the selection the right click kept.
    pub(super) fn menu_link(&mut self, id: Uuid) {
        if let Some(idx) = self.world.objects.index_of_uuid(&id) {
            self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
            self.build.link(&mut self.world);
            self.flush_build();
        }
    }

    /// Object.TakeSeparate / TakeSeparateCopy: each root its own item.
    pub(super) fn take_separate(&mut self, id: Uuid, copy: bool) {
        if let Some(idx) = self.world.objects.index_of_uuid(&id) {
            self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
            self.build.take_separate(&mut self.world, copy);
            if !self.build.open {
                self.build.deselect_all(&self.world);
            }
            self.flush_build();
        }
    }

    /// Each frame: the question, the script queue and its window.
    pub(super) fn object_menu_ui(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if let Some((confirm, dont_ask)) = self.object_menu.confirm.as_mut() {
            let (id, title, text, yes, no) = match confirm {
                Confirm::Return { .. } => (
                    "object_return_confirm",
                    aurora_i18n::tr!("ui-context-return"),
                    format!(
                        "{}\n\n{}",
                        aurora_i18n::tr!("ui-context-return-to-owner-question"),
                        aurora_i18n::tr!("ui-context-return-to-owner-warning")
                    ),
                    aurora_i18n::tr!("common-ok"),
                    aurora_i18n::tr!("common-cancel"),
                ),
                Confirm::Drop(_) => (
                    "attachment_drop_confirm",
                    aurora_i18n::tr!("ui-context-drop"),
                    aurora_i18n::tr!("ui-context-drop-question").to_string(),
                    aurora_i18n::tr!("common-yes"),
                    aurora_i18n::tr!("common-no"),
                ),
                Confirm::Scripts(kind, _) => (
                    "script_modify_confirm",
                    kind.title(),
                    aurora_i18n::tr!("ui-context-script-modify-question").to_string(),
                    aurora_i18n::tr!("common-ok"),
                    aurora_i18n::tr!("common-cancel"),
                ),
            };
            let q = Question {
                id,
                title,
                text: &text,
                yes,
                no,
            };
            if let Some(yes) = ui::context::ask(ctx, p, &q, dont_ask)
                && let Some((confirm, dont_ask)) = self.object_menu.confirm.take()
                && yes
            {
                if dont_ask {
                    let s = &mut self.settings;
                    match confirm {
                        Confirm::Return { .. } => s.confirm_return_to_owner = false,
                        Confirm::Drop(_) => s.confirm_attachment_drop = false,
                        Confirm::Scripts(..) => s.confirm_script_modify = false,
                    }
                    s.save();
                }
                self.object_menu_confirmed(confirm);
            }
        }
        if let Some(queue) = self.object_menu.queue.as_mut() {
            queue.step(&mut self.build, &self.world, Instant::now());
            let open = ui::script_queue::show(ctx, p, queue);
            if !open {
                self.object_menu.queue = None;
            }
            self.flush_build();
        }
    }
}

/// gInventory.getLinkedItemID: the item a link points to.
fn linked_item(world: &World, id: Uuid) -> Uuid {
    /// AT_LINK.
    const AT_LINK: i32 = 24;
    match world.inventory.items.get(&id) {
        Some(it) if it.asset_type == AT_LINK && !it.asset_id.is_nil() => it.asset_id,
        _ => id,
    }
}
