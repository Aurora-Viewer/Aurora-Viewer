//! Right-click menus: opening the world menu on what is under the cursor
//! (LLToolPie::handleRightClickPick), what its entries depend on, and the
//! actions chosen in any menu of the interface (ui/context.rs).
use super::*;
use crate::interaction::{self, Action};
use crate::ui::context::{CtxAction, Facts, ObjectFacts, Target};
use uuid::Uuid;

impl App {
    /// Right click on the world: the menu of what is under the cursor.
    pub(super) fn open_context_menu(&mut self) {
        self.click(click_pick::Click::Menu);
    }

    /// What `open_context_menu` does, run by `click`: again from the start
    /// when the depth under the cursor has come back (app/click_pick.rs).
    /// What a right click designates under the cursor: the object (an
    /// avatar's name tag counts as the avatar) and the point clicked; None
    /// while the depth is asked for (the click is run again), or on the sky.
    pub(super) fn menu_pick(&mut self) -> Option<(Option<usize>, Vec3)> {
        let (x, y) = self.cursor_pos;
        let now = Instant::now();
        // an avatar name tag opens its avatar's menu, as a click on the
        // avatar; its point is the avatar, not the air at the tag ("Zoom"
        // frames the avatar, like handle_look_at_selection)
        if let Some((idx, point, _)) = self.scene.hud_edit_pick(&self.world, (x, y)) {
            // (a HUD does not need the depth of the world behind it)
            let ray = self.gfx.as_ref().and_then(|g| g.renderer.cursor_ray(x, y));
            self.object_menu_pick(Some(idx), ray, Vec::new(), false);
            return Some((Some(idx), point));
        }
        let picked = self.click_pick()?;
        let (ray, depth) = (picked.ray, picked.depth);
        let (hit, target) = self.scene.selection_pick(
            &self.world,
            ray,
            depth,
            self.build.open,
            self.settings.build.select_probes,
            self.settings.draw_distance,
        );
        let surface = match self.pick_name_tag(hit, ray) {
            Some((idx, p)) => Some((
                Some(idx),
                Scene::object_transform(&self.world, idx, now, 0).map_or(p, |(pos, _, _)| pos),
            )),
            // what is shown under the pixel: a rigged mesh where the GPU
            // poses it, an attachment in front of its wearer
            // (LLToolPie::handleRightMouseDown picks rigged meshes and
            // transparent faces)
            None => match self.designate(crate::scene::drawn::Gesture::Menu, &picked, hit) {
                Some(d) => Some((Some(d.idx), d.hit.point)),
                None => hit.map(|p| (target, p)),
            },
        };
        let particles: Vec<_> = picked
            .particles
            .as_deref()
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|p| ray.is_some_and(|ray| p.before_surface(ray, surface.map(|(_, point)| point))))
            .collect();
        let particle_only = surface.is_none();
        let (object, point) = surface.or_else(|| particles.first().map(|p| (None, p.point)))?;
        self.object_menu_pick(object, ray, particles, particle_only);
        Some((object, point))
    }

    pub(super) fn world_menu(&mut self) {
        if self.gfx.is_none() {
            return;
        }
        let now = Instant::now();
        let Some((picked, point)) = self.menu_pick() else {
            return;
        };
        if let Some(source) = picked
            .and_then(|idx| self.world.objects.get(idx))
            .filter(|object| object.is_editable_preview())
            .and_then(|object| self.blender_live.source_for_object(object.full_id))
        {
            self.blender_live.selected = Some(if self.settings.build.edit_linked {
                source
            } else {
                self.blender_live.linked_root(source)
            });
        }
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
                    Some(w) if w == me => {
                        self.build.select_for_menu(&mut self.world, &self.settings.build, idx);
                        self.flush_build();
                        Target::Attachment { key }
                    }
                    // someone's attachment: their avatar's entries and its own
                    // (menu_attachment_other.xml), outlined by `menu_outline`
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
        if let Target::Avatar { id, own: false, .. } = target {
            self.moderation_menu_opened(id);
        }
        self.request_particle_source_names();
        if self.clicks.scripted() || self.demo && std::env::var_os("AURORA_DEMO_ACTIONS").is_some() {
            log::info!("demo menu: {target:?}");
        }
        let ppp = self.egui_ctx.pixels_per_point().max(0.1);
        self.world.ui_sounds.push(UiSound::PieMenuAppear);
        self.context_menu = Some(ui::context::ContextMenu {
            pos: egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp),
            point,
            target,
            pie_path: Vec::new(),
            pie_input_frame: None,
        });
    }

    /// The prims of the object another avatar wears that the open menu is
    /// about: Firestorm selects it while the menu is open
    /// (LLToolSelect::handleObjectSelection) and shows its outline. What we
    /// wear ourselves is selected by the build tools, like an object.
    pub(super) fn menu_outline(&self) -> Vec<usize> {
        let idx = match self.context_menu.as_ref().map(|m| &m.target) {
            Some(Target::Avatar { attachment: Some(id), .. }) => self.world.objects.index_of_uuid(id),
            _ => None,
        };
        match idx.filter(|i| self.scene.gpu.get(*i).is_some_and(|g| !g.hud)) {
            Some(idx) => crate::build::family(&self.world, linkset_root(&self.world, idx)),
            None => Vec::new(),
        }
    }

    /// What the open menu's entries depend on, this frame.
    pub(super) fn context_facts(&self) -> Facts {
        let w = &self.world;
        let mut f = Facts {
            worn_shape: crate::shape_editor::worn_shape(&self.world),
            me: w.agent_id,
            seated: w.agent.is_sitting(),
            flying: w.agent.flying,
            // Account options remain accessible while region support loads.
            hover_height: w.login.is_some(),
            edit_linked: self.settings.build.edit_linked,
            property_lines: self.settings.world_overlays.property_lines,
            voice: self.settings.audio.voice_enabled,
            particle_sources: self.particle_menu_sources(),
            particle_only: self.object_menu.particle_only,
            ..Facts::default()
        };
        match self.context_menu.as_ref().map(|m| &m.target) {
            Some(Target::Object { key, .. } | Target::Attachment { key }) => {
                if let Some(idx) = w.objects.index_of(key) {
                    f.object = self.object_facts(idx);
                }
                if matches!(self.context_menu.as_ref().map(|m| &m.target), Some(Target::Object { .. })) {
                    f.attach_points = self.attach_choices();
                }
            }
            Some(Target::Avatar { id, .. }) => {
                f.avatar_travel = !self.native_anchor.blocks_walk(w.agent.seated, w.agent.ground_sit)
                    && crate::autopilot::avatar_position(w, *id).is_some();
                f.following = w.following_avatar == Some(*id);
                f.complexity = w
                    .objects
                    .index_of_uuid(id)
                    .and_then(|idx| Some((*self.scene.avatar_complexity.get(id)?, self.scene.too_complex.contains(&idx))));
                f.render = self.settings.render_exception(id);
                f.blocked = w.is_avatar_blocked(id);
                f.particles_blocked = self.settings.particle_filters.blocked(*id, true);
                f.sound_blocks = w.derender.owner_sound_flags(*id);
                f.contact_sets = w.contact_sets.set_names();
                f.friend = w.social.friends.iter().any(|fr| fr.id == *id);
                f.voice_volume = crate::ui::context::voice_volume_of(w, *id);
                f.worn = !super::inspect::worn_roots(w, id).is_empty();
                f.moderation = self.moderation_rights(*id);
            }
            _ => {}
        }
        if self
            .context_menu
            .as_ref()
            .is_some_and(|m| matches!(m.target, Target::Attachment { .. } | Target::Avatar { own: true, .. }))
        {
            f.attach_points = self.attach_choices();
            f.removable_clothing = crate::world::appearance::worn_clothing(&w.inventory)
                .into_iter()
                .filter(|(id, _)| w.rlva_can_remove(*id, Uuid::nil()))
                .map(|(_, kind)| kind)
                .collect();
            f.removable_clothing.sort_unstable();
            f.removable_clothing.dedup();
            f.removable_points = w
                .worn_attachment_points()
                .into_iter()
                .filter(|(id, _)| w.rlva_can_remove(*id, Uuid::nil()))
                .map(|(_, point)| point)
                .collect();
            f.removable_points.sort_unstable();
            f.removable_points.dedup();
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
        f.edit = crate::build::selectable(w, idx);
        f.id = o.full_id;
        f.attachment_item = w.objects.get(root).and_then(|o| o.attachment_item_id());
        f.blocked = w.mutes.is_muted_id(&root_id);
        f.name = props.map(|p| p.name.clone()).unwrap_or_default();
        if let Some(root) = w.hud_root(idx).and_then(|i| w.objects.get(i)) {
            f.local_hud_scale = Some(w.hud_scales.scale(root.full_id));
            f.saved_hud_scale = w.hud_scales.has_profile(root.full_id);
            f.hud_move_locked = w.hud_locks.locked(root.full_id);
        }
        self.object_menu_facts(idx, &mut f);
        f.local_preview = o.is_editable_preview();
        if f.local_preview {
            f.wear = true;
            f.drop = true;
        }
        f
    }

    /// FSFloaterPlaceDetails::showPlaceDetails: a standalone window when
    /// FSUseStandalonePlaceDetailsFloater is on, else the profile in "Places"
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

    pub(super) fn open_object_profile(&mut self, id: Uuid) {
        let Some(index) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        let root = linkset_root(&self.world, index);
        let Some(object) = self.world.objects.get(root) else { return };
        let (id, key, local) = (object.full_id, object.key, object.is_client_preview());
        if self.object_profile.object != Some(id)
            && let Some(previous) = self.object_profile.object.take()
        {
            self.release_object_profile_selection(previous);
        }
        let props = self
            .interactions
            .props
            .get(&id)
            .filter(|props| props.full)
            .or_else(|| self.build.props.get(&id))
            .cloned();
        self.object_profile.open(id, key, props);
        if !local {
            self.send(NetCommand::Build(aurora_net::build::BuildCmd::Select {
                handle: key.region,
                local_ids: vec![key.local_id],
            }));
        }
    }

    fn release_object_profile_selection(&mut self, id: Uuid) {
        if let Some(index) = self.world.objects.index_of_uuid(&id)
            && let Some(object) = self.world.objects.get(index)
            && !object.is_client_preview()
            && !self.build.highlighted(&self.world).iter().any(|(selected, _)| *selected == index)
        {
            self.send(NetCommand::Build(aurora_net::build::BuildCmd::Deselect {
                handle: object.key.region,
                local_ids: vec![object.key.local_id],
            }));
        }
    }

    pub(super) fn object_profile_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(id) = self.object_profile.object else { return };
        let facts = self
            .world
            .objects
            .index_of_uuid(&id)
            .map_or_else(Default::default, |index| self.object_facts(index));
        self.object_profile.show(ctx, p, &mut self.world, &facts);
        if self.object_profile.object.is_none() {
            self.release_object_profile_selection(id);
        }
        let (commands, requests, properties) = self.object_profile.take_output();
        if !commands.is_empty() {
            self.interactions.on_properties(&properties);
            self.build.on_properties(properties);
        }
        for command in commands {
            self.send(command);
        }
        for request in requests {
            self.on_build_request(request);
        }
    }

    pub(super) fn on_ctx_action(&mut self, act: CtxAction) {
        match act {
            CtxAction::ObjectProfile(id) => self.open_object_profile(id),
            CtxAction::Pathfinding { id, characters } => {
                if let Some(index) = self.world.objects.index_of_uuid(&id)
                    && let Some(object) = self.world.objects.get(linkset_root(&self.world, index))
                {
                    let kind = if characters {
                        aurora_net::pathfinding::Kind::Characters
                    } else {
                        aurora_net::pathfinding::Kind::Objects
                    };
                    let command = self.pathfinding_ui.open(object.key.region, kind, object.full_id);
                    if self.demo {
                        self.pathfinding_ui.demo(&self.world, command);
                    } else {
                        self.send(command);
                    }
                }
            }
            CtxAction::ResetMeshLod(id) => {
                let roots = super::inspect::worn_roots(&self.world, &id);
                self.scene.reset_mesh_lods(&mut self.world, &roots, Instant::now());
            }
            CtxAction::ShowAvatarTextures(id) => self.avatar_textures.avatar = Some(id),
            CtxAction::SitAt(point) => self.sit_at(point),
            CtxAction::SitStand => self.on_ctx_action(if self.world.agent.is_sitting() {
                CtxAction::StandUp
            } else {
                CtxAction::SitGround
            }),
            CtxAction::LookAtAvatar(id) => {
                if let Some(point) = ui::context::avatar_position(&self.world, &id) {
                    let object = self
                        .world
                        .objects
                        .index_of_uuid(&id)
                        .and_then(|idx| crate::camera::pick_focus_object(&self.world, idx, Instant::now()));
                    self.camera
                        .set_script_focus(self.camera.position, point, object, &mut self.world.agent, &self.settings.camera);
                }
            }
            CtxAction::EditPbr(id) => {
                self.on_ctx_action(CtxAction::Edit(id));
                self.build.tab = 3;
                self.build.ui.tex_tab = 1;
            }
            CtxAction::TakeOffClothing(kind) => {
                let items = crate::world::appearance::worn_clothing(&self.world.inventory)
                    .into_iter()
                    .filter(|(id, k)| *k == kind && self.world.rlva_can_remove(*id, Uuid::nil()))
                    .map(|(id, _)| id)
                    .collect();
                self.apply_appearance_action(crate::world::appearance::Action::RemoveItems(items));
            }
            CtxAction::DetachAt(_) | CtxAction::DetachAll => {
                let point = if let CtxAction::DetachAt(point) = act { Some(point) } else { None };
                let items = self
                    .world
                    .worn_attachment_points()
                    .into_iter()
                    .filter(|(id, p)| point.is_none_or(|point| point == *p) && self.world.rlva_can_remove(*id, Uuid::nil()))
                    .map(|(id, _)| id)
                    .collect();
                self.apply_appearance_action(crate::world::appearance::Action::RemoveItems(items));
            }
            CtxAction::CustomizeMenu(menu) => {
                self.context_menu = Some(*menu.clone());
                let facts = self.context_facts();
                self.context_customization
                    .edit(&self.egui_ctx, &menu, &facts, &mut self.settings.world_context_menus);
                self.context_menu = Some(*menu);
            }
            CtxAction::ResetHudScale(key) => self.reset_hud_scale(key, false),
            CtxAction::ForgetHudScale(key) => self.reset_hud_scale(key, true),
            CtxAction::SetHudMoveLock(key, locked) => {
                self.world.refresh_hud_scales();
                if let Some(id) = self
                    .world
                    .objects
                    .index_of(&key)
                    .and_then(|idx| self.world.hud_root(idx))
                    .and_then(|root| self.world.objects.get(root))
                    .map(|o| o.full_id)
                {
                    self.world.hud_locks.set(id, locked);
                }
            }
            CtxAction::EditAttachment(key) => {
                if let Some(idx) = self.world.objects.index_of(&key)
                    && crate::build::selectable(&self.world, idx)
                    && let Some(id) = self.world.objects.get(idx).map(|o| o.full_id)
                {
                    self.release_object_hold();
                    self.on_ctx_action(CtxAction::Edit(id));
                }
            }
            CtxAction::NewPhysics => self.new_physics(),
            CtxAction::Freeze(id) => self.ask_moderation(id, false),
            CtxAction::Eject(id) => self.ask_moderation(id, true),
            CtxAction::EditWorn(item) => {
                if self.world.inventory.items.get(&item).is_some_and(crate::wearable_editor::supported) {
                    self.open_wearable_editor(item);
                    return;
                }
                if Some(item) == crate::shape_editor::worn_shape(&self.world) {
                    self.open_shape_editor(item);
                    return;
                }
                if Some(item) == crate::physics_editor::worn_physics(&self.world) {
                    self.open_physics_editor(item);
                    return;
                }
                if let Some(key) = self
                    .world
                    .worn_attachment(&item)
                    .and_then(|i| self.world.objects.get(i))
                    .map(|o| o.key)
                {
                    self.on_ctx_action(CtxAction::EditAttachment(key));
                }
            }
            CtxAction::TouchWorn(item) => {
                // the root prim, when it handles touches (enable_attachment_touch)
                if let Some(o) = self.world.worn_attachment(&item).and_then(|i| self.world.objects.get(i))
                    && o.update_flags & ui::context::flags::HANDLE_TOUCH != 0
                {
                    let local_id = o.key.local_id;
                    self.send(NetCommand::Touch { local_id });
                }
            }
            CtxAction::AttachmentInventory(item) => self.apply_appearance_action(crate::world::appearance::Action::ShowOriginal(item)),
            CtxAction::Detach(key) => {
                if let Some(source) = self
                    .world
                    .objects
                    .index_of(&key)
                    .and_then(|idx| self.world.objects.get(idx))
                    .and_then(|o| self.blender_live.source_for_object(o.full_id))
                {
                    self.blender_live.object_action = Some((source, crate::blender_live::objects::Action::Detach));
                    return;
                }
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
            CtxAction::Touch(local_id) => self.menu_touch(local_id),
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
            CtxAction::Zoom(p, id) => {
                let object = id.and_then(|id| self.zoom_focus_object(&id));
                self.camera.zoom_to(p, object, &mut self.world.agent, &self.settings.camera);
            }
            CtxAction::GoToAvatar(id) => self.travel_to_avatar(id, false),
            CtxAction::FollowAvatar(id) => self.travel_to_avatar(id, true),
            CtxAction::StopFollowing => self.stop_autopilot(true),
            CtxAction::AboutLand(point) => {
                let (gx, gy) = ui::minimap::to_global(&self.world, point);
                ui::land::LandUi::select_at_global(&mut self.world, gx, gy);
                self.panels.about_land = true;
            }
            // handle_buy_land on the parcel LLToolPie selected
            CtxAction::BuyLand(point) => {
                let (gx, gy) = ui::minimap::to_global(&self.world, point);
                ui::land::LandUi::select_at_global(&mut self.world, gx, gy);
                self.open_buy_land(buy_land::BuyLandSource::Parcel, false);
            }
            CtxAction::BuyPass(point) => {
                let (gx, gy) = ui::minimap::to_global(&self.world, point);
                self.land_ui.buy_pass_at(&mut self.world, gx, gy);
            }
            CtxAction::SetPropertyLines(on) => {
                self.settings.world_overlays.property_lines = on;
                self.settings.save();
            }
            CtxAction::DisplayName => self.display_name_ui.open(),
            CtxAction::HoverHeight => self.hover_height_ui.open = true,
            CtxAction::ReportAbuse(target) => self.open_report_abuse(target),
            CtxAction::CreateLandmark => self.open_create_landmark(None),
            CtxAction::OpenAppearance { tab, editing } => {
                self.panels.appearance = true;
                self.appearance_ui.open(tab, editing);
            }
            CtxAction::StandUp => self.send(NetCommand::OneShotControl(control::STAND_UP)),
            CtxAction::SitGround => self.send(NetCommand::OneShotControl(control::SIT_ON_GROUND)),
            CtxAction::ToggleFly => self.toggle_fly(),
            CtxAction::ResetCamera => self.reset_camera_view(),
            CtxAction::Im(id) => {
                self.open_conversation_to_type(id);
            }
            CtxAction::Call(id) => {
                if self.settings.audio.voice_enabled {
                    self.world.start_call(id);
                    if let Some(c) = self.world.calls.active.as_ref() {
                        self.chat_ui.open(c.conversation());
                    }
                    self.show_conversations();
                }
            }
            CtxAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
            CtxAction::ToggleBlock(id) => {
                let name = self.world.legacy_name(&id).unwrap_or_else(|| self.world.social.name_of(&id));
                if let Err(e) = self.world.toggle_block_avatar(id, &name) {
                    self.world.system_message(e);
                }
            }
            // the volumes of the call's participants: applied live, saved
            // once the slider rests
            CtxAction::SetVoiceVolume(id, v) => {
                self.world.calls.set_volume(id, v);
                self.volumes_save_at = Some(Instant::now() + std::time::Duration::from_millis(500));
            }
            // LLInspectAvatar::onClickMuteVolume: only the voice flag
            CtxAction::ToggleVoiceMute(id) => {
                use crate::world::mutes::{MuteType, flag};
                let name = self.world.legacy_name(&id).unwrap_or_else(|| self.world.social.name_of(&id));
                if self.world.voice_blocked(&id) {
                    self.world.unblock(id, &name, flag::VOICE_CHAT);
                } else if let Err(e) = self.world.block_flags(id, &name, MuteType::Agent, flag::VOICE_CHAT) {
                    self.world.system_message(e);
                }
            }
            CtxAction::ModerateVoice(target, mute) => {
                use aurora_net::voice_moderation::Target;
                let target = target.map_or(Target::Everyone, Target::Resident);
                self.send(NetCommand::ModerateVoice { target, mute });
            }
            CtxAction::ToggleOwnerParticleFilter(id) => {
                let blocked = !self.settings.particle_filters.blocked(id, true);
                if self.settings.particle_filters.set(id, true, blocked) {
                    self.settings.save();
                }
            }
            CtxAction::SetOwnerSoundBlock { owner, flag, blocked } => self.set_owner_sound_block(owner, flag, blocked),
            CtxAction::InviteToGroup(id) => self.group_profile_ui.invite.choose_group(vec![id]),
            CtxAction::GiveCallingCard(id) => self.world.offer_calling_card(id),
            CtxAction::ToggleBlockObject { id, name } => {
                // the whole object is blocked: its root (LLMute of getRootEdit)
                let root = self
                    .world
                    .objects
                    .index_of_uuid(&id)
                    .and_then(|i| self.world.objects.get(linkset_root(&self.world, i)))
                    .map_or(id, |r| r.full_id);
                let name = if name.is_empty() {
                    aurora_i18n::tr!("common-object-2", value0 = root.to_string()[..8].to_string())
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
            CtxAction::Inspect(id) => self.inspect_object(id),
            CtxAction::InspectAvatar(id) => self.inspect_avatar(id),
            CtxAction::Derender { id, permanent } => self.derender(id, permanent),

            CtxAction::ScriptInfo(id) => {
                let root = self
                    .world
                    .objects
                    .index_of_uuid(&id)
                    .map(|idx| linkset_root(&self.world, idx))
                    .and_then(|idx| self.world.objects.get(idx))
                    .map_or(id, |o| o.full_id);
                self.inspect_scripts(root);
            }
            CtxAction::ObjectInfo(id) => self.inspect_object_information(id),
            CtxAction::RefreshTextures(id) => self.refresh_object_textures(id),
            CtxAction::RefreshAvatarTextures(id) => self.refresh_avatar_textures(id),
            CtxAction::Rebake => self.rebake(),
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
                if let Some(source) = self.blender_live.source_for_object(id) {
                    self.blender_live.object_action = Some((source, crate::blender_live::objects::Action::Derez));
                    return;
                }
                let Some(idx) = self.world.objects.index_of_uuid(&id) else {
                    return;
                };
                let family = crate::build::family(&self.world, linkset_root(&self.world, idx));
                let prim_flags: Vec<u32> = family
                    .iter()
                    .filter_map(|&i| self.world.objects.get(i).map(|o| o.update_flags))
                    .collect();
                match ui::context::delete_warning(&prim_flags) {
                    Some(text) => self.delete_confirm = Some((vec![id], text)),
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
            CtxAction::PayResident(id) => self.pay_ui.open(id, false),
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
            CtxAction::Link(id) => self.menu_link(id),
            CtxAction::TakeSeparate { id, copy } => self.take_separate(id, copy),
            CtxAction::Return(id) => self.ask_return(id),
            CtxAction::Wear { id, point, add } => self.wear_object(id, point, add),
            CtxAction::Scripts { id, action } => self.ask_scripts(id, action),
            CtxAction::Drop(key) => {
                if let Some(source) = self
                    .world
                    .objects
                    .index_of(&key)
                    .and_then(|idx| self.world.objects.get(idx))
                    .and_then(|o| self.blender_live.source_for_object(o.full_id))
                {
                    self.blender_live.object_action = Some((source, crate::blender_live::objects::Action::Drop));
                } else {
                    self.ask_drop(key);
                }
            }
            CtxAction::SetParticleBlock { id, owner, blocked } => {
                if self.settings.particle_filters.set(id, owner, blocked) {
                    self.settings.save();
                }
            }
            CtxAction::AttachmentFavorite(item) => self.attachment_favorite(item),
            CtxAction::EditLinkedParts(on) => {
                self.build.set_edit_linked(&mut self.world, &mut self.settings.build, on);
                self.settings.save();
                self.flush_build();
            }
            CtxAction::Build => self.build.open_build(crate::build::Tool::Create),
            CtxAction::EditTerrain => self.build.open_build(crate::build::Tool::Land),
            CtxAction::SetRender(id, mode) => {
                self.set_avatar_render_settings(&[id], mode);
            }
            CtxAction::ResetSkeleton { avatar, animations } => self.reset_skeleton(avatar, animations),
            CtxAction::StopAnimations => self.stop_current_animations(),
            CtxAction::StopAnimationsKeepPermissions => self.stop_animations(false),
            CtxAction::Undeform => self.undeform_avatar(),
            CtxAction::RefreshAttachments => self.refresh_attachments(),
            CtxAction::AnswerTeleport { invite, accept } => {
                for c in self.world.answer_teleport_invite(invite, accept) {
                    if matches!(c, NetCommand::AcceptLure { .. }) {
                        self.request_user_teleport(c, String::new(), false);
                    } else {
                        self.send(c);
                    }
                }
            }
            CtxAction::AnswerInventoryOffer { offer, keep } => {
                for c in self.world.answer_inventory_offer(offer, keep) {
                    self.send(c);
                }
            }
            CtxAction::OfferTeleport(id) => {
                if self.world.rlva_can_offer_teleport(id) {
                    self.contacts_ui.ask_teleport_offer(vec![id], &self.world.rlva_region_label());
                }
            }
            CtxAction::OpenPeople(tab) => {
                self.panels.people = true;
                self.panels.people_tab = tab;
            }
            CtxAction::ZoomAvatar(id) => {
                // a drawn avatar is followed; one only known by its map
                // position is a fixed point (handle_zoom_to_object)
                if let Some(avatar) = self.zoom_focus_object(&id).filter(|o| o.1.avatar) {
                    self.camera.zoom_avatar(avatar, &mut self.world.agent, &self.settings.camera);
                } else if let Some(p) = ui::context::avatar_position(&self.world, &id) {
                    self.camera.zoom_to(p, None, &mut self.world.agent, &self.settings.camera);
                }
            }
            CtxAction::TeleportToAvatar(id) | CtxAction::ShowOnMap(id) => {
                let teleport = matches!(act, CtxAction::TeleportToAvatar(_));
                if self.world.locate_avatar(id, teleport) && !teleport {
                    ui::widgets::open_floater(&self.egui_ctx, "worldmap", &mut self.panels.world_map);
                }
            }
            CtxAction::ShowPlaceInfo(region, pos) => {
                if let Some(region) = self.grid_region(&region) {
                    self.show_place_profile(crate::world::place_details::Source::Link { region, pos });
                }
            }
            CtxAction::ShowPlace(region, pos) => {
                let Some(region) = self.grid_region(&region) else {
                    return;
                };
                self.world.map.track_region(&region, pos);
                ui::widgets::open_floater(&self.egui_ctx, "worldmap", &mut self.panels.world_map);
            }
            CtxAction::TeleportToPlace(region, pos) if self.settings.confirm_teleports => self.place_confirm = Some((region, pos, false)),
            CtxAction::TeleportToPlace(region, pos) => self.teleport_to_location(region, pos),
            CtxAction::AppLink(link) => self.run_app_link(link),
            CtxAction::OpenUrl(url) => {
                use crate::link_trust::Trust;
                // LLWeb::loadURLInternal: a Marketplace page opens in the
                // Marketplace window (Firestorm's default browser choice keeps
                // the Second Life sites inside the viewer)
                let marketplace = self.marketplace_home();
                if marketplace.is_some_and(|home| ui::marketplace::is_marketplace_url(&url, &home))
                    && !matches!(crate::link_trust::classify(&url), Trust::Dangerous(_))
                {
                    self.marketplace_ui.open(Some(url));
                } else {
                    match crate::link_trust::classify(&url) {
                        Trust::Dangerous(why) => self.url_confirm = Some((url, false, Some(why))),
                        Trust::Unknown if self.settings.warn_external_links => self.url_confirm = Some((url, false, None)),
                        _ => self.egui_ctx.open_url(egui::OpenUrl::new_tab(url)),
                    }
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
            CtxAction::RemoveFriends(ids) => self.contacts_ui.ask_remove_friends(ids),
            CtxAction::Conference(ids) => {
                self.world.start_text_conference(ids);
                self.show_conversations();
            }
            CtxAction::ConferenceCall(ids) => {
                if self.settings.audio.voice_enabled {
                    self.world.start_conference(ids);
                    self.show_conversations();
                }
            }
            CtxAction::OfferTeleports(ids) => {
                let ids = ids.into_iter().filter(|id| self.world.rlva_can_offer_teleport(*id)).collect();
                self.contacts_ui.ask_teleport_offer(ids, &self.world.rlva_region_label());
            }
            CtxAction::RequestTeleport(id) => self.contacts_ui.ask_teleport_request(id, self.world.status.cfg.reject_teleports),
            CtxAction::ActivateGroup(id) => self.send(NetCommand::ActivateGroup(id)),
            CtxAction::PinGroup(id) => self.world.groups.toggle_favorite(id),
            CtxAction::LeaveGroup(id) => self.contacts_ui.ask_leave_group(id),
            CtxAction::GroupChat(id) => {
                self.world.ui_sounds.push(UiSound::StartIm);
                self.world.start_group_chat(id);
                self.show_conversations();
            }
            CtxAction::GroupChatBlocked(id, on) => self.world.set_group_chat_blocked(id, on),
            CtxAction::ContactSet(edit) => crate::ui::contacts::apply_set_edit(&mut self.world, edit),
            CtxAction::NewContactSet(ids) => self.contacts_ui.ask_new_set_for(ids),
            CtxAction::ShowHistory(conversation) => {
                let (title, file) = self.world.transcript_of(conversation);
                self.history_ui.show_file(title, file);
            }
        }
    }

    /// The focus object a Zoom keeps the camera on (an attachment gives its
    /// avatar), when `id` is in view.
    fn zoom_focus_object(&self, id: &uuid::Uuid) -> Option<(uuid::Uuid, crate::camera::FocusObject)> {
        let idx = self.world.objects.index_of_uuid(id)?;
        crate::camera::pick_focus_object(&self.world, idx, Instant::now())
    }
}

/// The root prim of object `idx`'s linkset (not the avatar it is worn by).
pub(super) fn linkset_root(world: &World, idx: usize) -> usize {
    let mut i = idx;
    for _ in 0..64 {
        let Some(o) = world.objects.get(i) else { break };
        // An avatar's script statistics concern the avatar, including while
        // seated. Its seat is a transform parent, not its script-info root.
        if o.is_avatar() {
            break;
        }
        match world.objects.parent_of(o) {
            Some(p) if world.objects.get(p).is_some_and(|po| !po.is_avatar()) => i = p,
            _ => break,
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_information_of_a_seated_avatar_does_not_inspect_its_seat() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut avatar = crate::demo::action_avatar(false);
        avatar.local_id = 99101;
        avatar.full_id = crate::demo::action_id(99101);
        avatar.parent_id = 99100;
        let mut seat = avatar.clone();
        seat.local_id = 99100;
        seat.full_id = crate::demo::action_id(99100);
        seat.parent_id = 0;
        seat.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        let mut attachment = seat.clone();
        attachment.local_id = 99102;
        attachment.full_id = crate::demo::action_id(99102);
        attachment.parent_id = 99101;
        world.apply(aurora_net::NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![seat, avatar, attachment],
        });
        for local in [99100, 99101, 99102] {
            let index = world.objects.index_of_uuid(&crate::demo::action_id(local)).expect("fixture object");
            assert_eq!(linkset_root(&world, index), index, "local {local}");
        }
    }
}
