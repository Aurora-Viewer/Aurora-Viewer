//! Right-click menus: opening the world menu on what is under the cursor
//! (LLToolPie::handleRightClickPick), what its entries depend on, and the
//! actions chosen in any menu of the interface (ui/context.rs).
use super::*;
use crate::interaction::{self, Action};
use crate::ui::context::{CtxAction, Facts, ObjectFacts, Target};

impl App {
    pub(super) fn open_context_menu(&mut self) {
        let Some(g) = self.gfx.as_mut() else {
            return;
        };
        let (x, y) = self.cursor_pos;
        let ray = g.renderer.cursor_ray(x, y);
        let hit = self.scene.action_point(
            &self.world,
            ray,
            g.renderer.pick_world(x, y),
            self.build.open,
            self.settings.draw_distance,
        );
        let now = Instant::now();
        // an avatar name tag opens its avatar's menu, as a click on the
        // avatar; its point is the avatar, not the air at the tag ("Zoomer"
        // frames the avatar, like handle_look_at_selection)
        let (picked, point) = if let Some((idx, point, _)) = self.scene.hud_pick(&self.world, (x, y), true) {
            (Some(idx), point)
        } else {
            match self.pick_name_tag(hit, ray) {
                Some((idx, p)) => (
                    Some(idx),
                    Scene::object_transform(&self.world, idx, now, 0).map_or(p, |(pos, _, _)| pos),
                ),
                None => {
                    let Some(p) = hit else {
                        return;
                    };
                    (self.scene.interaction_at(&self.world, p, now, self.build.open), p)
                }
            }
        };
        // the previous menu's selection goes, as deselectUnused would
        self.build.release_menu_selection(&self.world);
        let me = self.world.agent_id;
        let target = match picked.and_then(|i| self.world.objects.get(i).map(|o| (i, o))) {
            Some((_, o)) if o.is_avatar() => Target::Avatar {
                id: o.full_id,
                own: o.full_id == me,
                attachment: None,
            },
            Some((idx, o)) => {
                let (key, full_id) = (o.key, o.full_id);
                match ui::context::wearer(&self.world, idx) {
                    Some(w) if w == me => Target::Attachment { key },
                    // someone's attachment: their avatar's menu (menu_attachment_other.xml)
                    Some(w) => Target::Avatar {
                        id: w,
                        own: false,
                        attachment: Some(full_id),
                    },
                    None => {
                        let offset = Scene::object_transform(&self.world, idx, now, 0)
                            .map(|(pos, rot, _)| rot.inverse() * (point - pos))
                            .unwrap_or(Vec3::ZERO);
                        // selected and outlined while the menu is open
                        // (LLToolSelect::handleObjectSelection, temp_select)
                        self.build.select_for_menu(&mut self.world, &self.settings.build, idx);
                        self.flush_build();
                        Target::Object { key, full_id, offset }
                    }
                }
            }
            None => Target::Ground,
        };
        let ppp = self.egui_ctx.pixels_per_point().max(0.1);
        self.world.ui_sounds.push(UiSound::PieMenuAppear);
        self.context_menu = Some(ui::context::ContextMenu {
            pos: egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp),
            point,
            target,
        });
    }

    /// What the open menu's entries depend on, this frame.
    pub(super) fn context_facts(&self) -> Facts {
        let w = &self.world;
        let mut f = Facts {
            me: w.agent_id,
            seated: w.agent.is_sitting(),
            flying: w.agent.flying,
            edit_linked: self.settings.build.edit_linked,
            ..Facts::default()
        };
        match self.context_menu.as_ref().map(|m| &m.target) {
            Some(Target::Object { key, .. } | Target::Attachment { key }) => {
                if let Some(idx) = w.objects.index_of(key) {
                    f.object = self.object_facts(idx);
                }
            }
            Some(Target::Avatar { id, .. }) => {
                f.complexity = w
                    .objects
                    .index_of_uuid(id)
                    .and_then(|idx| Some((*self.scene.avatar_complexity.get(id)?, self.scene.too_complex.contains(&idx))));
                f.render = self.settings.render_exception(id);
                f.blocked = w.is_avatar_blocked(id);
                f.friend = w.social.friends.iter().any(|fr| fr.id == *id);
            }
            _ => {}
        }
        f
    }

    fn object_facts(&self, idx: usize) -> ObjectFacts {
        let w = &self.world;
        let Some(o) = w.objects.get(idx) else {
            return ObjectFacts::default();
        };
        let flags_of = |i: usize| w.objects.get(i).map_or(0, |o| o.update_flags);
        let parent = w
            .objects
            .parent_of(o)
            .filter(|&p| w.objects.get(p).is_some_and(|po| !po.is_avatar()));
        let root = linkset_root(w, idx);
        let family = crate::build::family(w, root);
        let family_flags: Vec<u32> = family.iter().map(|&i| flags_of(i)).collect();
        // we sit on one of its prims
        let sitting_on = w
            .objects
            .index_of_uuid(&w.agent_id)
            .and_then(|a| w.objects.get(a))
            .and_then(|a| w.objects.parent_of(a))
            .is_some_and(|seat| family.contains(&seat));
        let root_id = w.objects.get(root).map(|r| r.full_id).unwrap_or_default();
        let props = self.build.props.get(&root_id).or_else(|| self.interactions.props.get(&root_id));
        let for_sale = props.is_some_and(|p| p.sale_type != 0);
        let mut f = ObjectFacts::from_flags(
            o.update_flags,
            parent.map_or(0, flags_of),
            flags_of(root),
            &family_flags,
            sitting_on,
            for_sale,
        );
        f.open = interaction::allow_open(w, idx);
        f.attachment_item = w.objects.get(root).and_then(|o| o.attachment_item_id());
        f.blocked = w.mutes.is_muted_id(&root_id);
        f.name = props.map(|p| p.name.clone()).unwrap_or_default();
        f
    }

    /// FSFloaterPlaceDetails::showPlaceDetails: a standalone window when
    /// FSUseStandalonePlaceDetailsFloater is on, else the profile in « Lieux »
    /// (LLFloaterSidePanelContainer::showPanel("places", key)).
    pub(super) fn show_place_profile(&mut self, source: crate::world::place_details::Source) {
        let now = Instant::now();
        let w = &mut self.world;
        if self.settings.standalone_place_details {
            let serial = w.place_details.open_window(source, &w.map, &mut w.landmarks, now);
            self.place_ui.raise(serial);
        } else {
            w.place_details.open_panel(source, &w.map, &mut w.landmarks, now);
            self.panels.places = true;
            self.places_ui.raise();
        }
    }

    pub(super) fn on_ctx_action(&mut self, act: CtxAction) {
        match act {
            CtxAction::AttachmentInventory(item) => self.apply_appearance_action(crate::world::appearance::Action::ShowOriginal(item)),
            CtxAction::Detach(key) => {
                if let Some(idx) = self.world.objects.index_of(&key)
                    && ui::context::wearer(&self.world, idx) == Some(self.world.agent_id)
                    && let Some(item) = self
                        .world
                        .objects
                        .get(linkset_root(&self.world, idx))
                        .and_then(|o| o.attachment_item_id())
                {
                    self.release_object_hold();
                    self.apply_appearance_action(crate::world::appearance::Action::Remove(item));
                }
            }
            CtxAction::Touch(local_id) => self.send(NetCommand::Touch { local_id }),
            CtxAction::Sit { target, offset } => {
                if let Some(idx) = self.world.objects.index_of_uuid(&target)
                    && let Some(object) = self.world.objects.get(idx)
                {
                    self.send(NetCommand::RequestSit {
                        handle: object.key.region,
                        target,
                        offset,
                    });
                }
            }
            CtxAction::Zoom(p) => self.camera.zoom_to(p, &mut self.world.agent, &self.settings.camera),
            CtxAction::AboutLand(point) => {
                let (gx, gy) = ui::minimap::to_global(&self.world, point);
                ui::land::LandUi::select_at_global(&mut self.world, gx, gy);
                self.panels.about_land = true;
            }
            CtxAction::DisplayName => self.display_name_ui.open(),
            CtxAction::OpenAppearance { tab, editing } => {
                self.panels.appearance = true;
                self.appearance_ui.open(tab, editing);
            }
            CtxAction::StandUp => self.send(NetCommand::OneShotControl(control::STAND_UP)),
            CtxAction::SitGround => self.send(NetCommand::OneShotControl(control::SIT_ON_GROUND)),
            CtxAction::ToggleFly => self.toggle_fly(),
            CtxAction::ResetCamera => self.reset_camera_view(),
            CtxAction::Im(id) => {
                // UISndStartIM (LLAvatarActions::startIM)
                self.world.ui_sounds.push(UiSound::StartIm);
                self.world.social.session_mut(id);
                self.world.social.focus_im = Some(id);
                self.panels.chat = true;
            }
            CtxAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
            CtxAction::ToggleBlock(id) => {
                let name = self.world.legacy_name(&id).unwrap_or_else(|| self.world.social.name_of(&id));
                if let Err(e) = self.world.toggle_block_avatar(id, &name) {
                    self.world.system_message(e);
                }
            }
            CtxAction::ToggleBlockObject { id, name } => {
                // the whole object is blocked: its root (LLMute of getRootEdit)
                let root = self
                    .world
                    .objects
                    .index_of_uuid(&id)
                    .and_then(|i| self.world.objects.get(linkset_root(&self.world, i)))
                    .map_or(id, |r| r.full_id);
                let name = if name.is_empty() {
                    format!("Objet {}", &root.to_string()[..8])
                } else {
                    name
                };
                if self.world.mutes.is_muted_id(&root) {
                    self.world.unblock(root, &name, 0);
                } else if let Err(e) = self.world.block(root, &name, crate::world::mutes::MuteType::Object) {
                    self.world.system_message(e);
                }
            }
            CtxAction::Edit(id) => {
                // pie menu "Edit": select the object, build floater on Edit (handle_object_edit)
                if let Some(idx) = self.world.objects.index_of_uuid(&id) {
                    self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                    self.build.open_build(crate::build::Tool::Edit);
                    self.build.edit_mode = crate::build::EditMode::Move;
                    self.flush_build();
                }
            }
            CtxAction::Take(id) | CtxAction::TakeCopy(id) => {
                let copy = matches!(act, CtxAction::TakeCopy(_));
                if let Some(idx) = self.world.objects.index_of_uuid(&id) {
                    self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                    self.build.take(&mut self.world, copy);
                    if !self.build.open {
                        self.build.deselect_all(&self.world);
                    }
                    self.flush_build();
                }
            }
            CtxAction::Delete(id) => {
                let Some(idx) = self.world.objects.index_of_uuid(&id) else {
                    return;
                };
                let family = crate::build::family(&self.world, linkset_root(&self.world, idx));
                let prim_flags: Vec<u32> = family
                    .iter()
                    .filter_map(|&i| self.world.objects.get(i).map(|o| o.update_flags))
                    .collect();
                match ui::context::delete_warning(&prim_flags) {
                    Some(text) => self.delete_confirm = Some((id, text)),
                    None => self.on_ctx_action(CtxAction::DeleteConfirmed(id)),
                }
            }
            CtxAction::DeleteConfirmed(id) => {
                if let Some(idx) = self.world.objects.index_of_uuid(&id) {
                    self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                    self.build.delete(&mut self.world);
                    self.flush_build();
                }
            }
            CtxAction::Pay(id) | CtxAction::Buy(id) | CtxAction::Open(id) => {
                let action = match act {
                    CtxAction::Pay(_) => Action::Pay,
                    CtxAction::Buy(_) => Action::Buy,
                    _ => Action::Open,
                };
                if let Some(idx) = self.world.objects.index_of_uuid(&id)
                    && let Some(t) = interaction::menu_target(&self.world, idx, action)
                {
                    self.activate_object_action(t, idx, Vec3::ZERO, None);
                }
            }
            CtxAction::Unlink(id) => {
                if let Some(idx) = self.world.objects.index_of_uuid(&id) {
                    self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                    self.build.unlink(&mut self.world);
                    if !self.build.open {
                        self.build.deselect_all(&self.world);
                    }
                    self.flush_build();
                }
            }
            CtxAction::EditLinkedParts(on) => {
                self.build.set_edit_linked(&mut self.world, &mut self.settings.build, on);
                self.settings.save();
                self.flush_build();
            }
            CtxAction::Build => self.build.open_build(crate::build::Tool::Create),
            CtxAction::EditTerrain => self.build.open_build(crate::build::Tool::Land),
            CtxAction::SetRender(id, mode) => {
                self.settings.set_render_exception(id, mode);
                self.settings.save();
            }
            CtxAction::OfferTeleport(id) => {
                self.send(NetCommand::OfferTeleport {
                    to: id,
                    message: "Rejoins-moi !".into(),
                });
                self.world.system_message("Offre de téléportation envoyée.");
            }
            CtxAction::OpenPeople(tab) => {
                self.panels.people = true;
                self.panels.people_tab = tab;
            }
            CtxAction::ZoomAvatar(id) => {
                if let Some(p) = ui::context::avatar_position(&self.world, &id) {
                    self.camera.zoom_to(p, &mut self.world.agent, &self.settings.camera);
                }
            }
            CtxAction::TeleportToAvatar(id) | CtxAction::ShowOnMap(id) => {
                let Some(p) = ui::context::avatar_position(&self.world, &id) else {
                    return;
                };
                let (x, y) = ui::minimap::to_global(&self.world, p);
                let teleport = matches!(act, CtxAction::TeleportToAvatar(_));
                self.world.map.track_location(x, y, p.z, teleport);
                if !teleport {
                    self.panels.world_map = true;
                }
            }
            CtxAction::ShowPlaceInfo(region, pos) => self.show_place_profile(crate::world::place_details::Source::Link { region, pos }),
            CtxAction::ShowPlace(region, pos) => {
                self.world.map.track_region(&region, pos);
                self.panels.world_map = true;
            }
            CtxAction::TeleportToPlace(region, pos) => self.place_confirm = Some((region, pos)),
            CtxAction::OpenUrl(url) => {
                use crate::link_trust::Trust;
                match crate::link_trust::classify(&url) {
                    Trust::Dangerous(why) => self.url_confirm = Some((url, false, Some(why))),
                    Trust::Unknown if self.settings.warn_external_links => self.url_confirm = Some((url, false, None)),
                    _ => self.egui_ctx.open_url(egui::OpenUrl::new_tab(url)),
                }
            }
            CtxAction::TeleportToPlaceConfirmed(region, pos) => self.teleport_to_location(region, pos),
            CtxAction::GrantRights { friend, rights } => {
                let cmd = NetCommand::GrantUserRights { friend, rights };
                self.world.profile_command(&cmd);
                self.send(cmd);
            }
            CtxAction::AddFriend(id) => self.contacts_ui.ask_friendship(id),
            CtxAction::RemoveFriend(id) => self.contacts_ui.ask_remove_friend(id),
            CtxAction::RequestTeleport(id) => self.contacts_ui.ask_teleport_request(id),
            CtxAction::ActivateGroup(id) => self.send(NetCommand::ActivateGroup(id)),
            CtxAction::PinGroup(id) => self.world.groups.toggle_favorite(id),
            CtxAction::LeaveGroup(id) => self.contacts_ui.ask_leave_group(id),
            CtxAction::GroupChat(id) => {
                self.world.ui_sounds.push(UiSound::StartIm);
                self.world.start_group_chat(id);
                self.panels.chat = true;
            }
            CtxAction::GroupChatBlocked(id, on) => self.world.set_group_chat_blocked(id, on),
        }
    }
}

/// The root prim of object `idx`'s linkset (not the avatar it is worn by).
fn linkset_root(world: &World, idx: usize) -> usize {
    let mut i = idx;
    for _ in 0..64 {
        let Some(o) = world.objects.get(i) else { break };
        match world.objects.parent_of(o) {
            Some(p) if world.objects.get(p).is_some_and(|po| !po.is_avatar()) => i = p,
            _ => break,
        }
    }
    i
}
