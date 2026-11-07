//! Subject eligibility is checked every frame, independently of the picker.
use super::*;
use crate::{
    ui::menu,
    world::{bridge::QueryState, poser_consent},
};

pub(super) fn eligibility_reason(world: &World, id: Uuid) -> Option<&'static str> {
    if id == world.agent_id {
        return id.is_nil().then_some("not-owned");
    }
    let Some(index) = world.objects.index_of_uuid(&id) else {
        return Some("not-owned");
    };
    let Some(object) = world.objects.get(index) else {
        return Some("not-owned");
    };
    if object.is_avatar() {
        return (!world.poser_consent.can_pose(world, id)).then_some("no-consent");
    }
    // the simulator leaves OwnerID empty in most object updates and fills
    // it only with ObjectProperties (a selection): the update flags say
    // "you own this" for every prim, as permYouOwner (llviewerobject.cpp)
    let you_own = object.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0;
    if object.owner_id != world.agent_id && !(you_own && object.owner_id.is_nil()) {
        return Some("not-owned");
    }
    let Some(own) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i)) else {
        return Some("not-owned");
    };
    if object.key.region != own.key.region || Some(object.key.region) != world.main_region {
        return Some("other-region");
    }
    if world.mutes.is_muted_id(&id) {
        return Some("muted");
    }
    if !Scene::object_transform(world, index, Instant::now(), 0)
        .zip(
            world
                .objects
                .index_of_uuid(&world.agent_id)
                .and_then(|i| Scene::object_transform(world, i, Instant::now(), 0)),
        )
        .is_some_and(|(a, b)| a.0.distance(b.0) <= 50.0)
    {
        return Some("too-far");
    }
    if !Scene::skeleton_owner(world, index).is_some_and(|(owner, root)| owner == id && root == index) {
        return Some("not-root");
    }
    // no pose of its own yet is fine: a still animesh is posed only once
    // it animates, and the poser does not need that pose to start
    None
}
pub(super) fn allowed(world: &World, id: Uuid) -> bool {
    eligibility_reason(world, id).is_none()
}
/// Legacy Viewer names owned animesh through `FSFloaterPoser::getControlAvatarName`
/// (indra/newview/fsfloaterposer.cpp), preferring its worn inventory item.
fn subject_name(world: &World, names: &HashMap<Uuid, String>, id: Uuid) -> String {
    if id == world.agent_id {
        return aurora_i18n::tr!("poser-you").to_string();
    }
    if let Some(object) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i))
        && !object.is_avatar()
    {
        if let Some(item) = object.attachment_item_id().and_then(|item| world.inventory.items.get(&item)) {
            return item.name.clone();
        }
        return names
            .get(&id)
            .cloned()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| aurora_i18n::tr!("poser-animesh").to_string());
    }
    poser_consent::name(world, id)
}
impl Editor {
    pub(in crate::ik_edit) fn editor_frame(&self, world: &World) -> Option<Frame> {
        let id = if self.poser.active && !self.poser.subject.is_nil() {
            self.poser.subject
        } else {
            world.agent_id
        };
        Frame::subject(world, id)
    }
    pub fn pose_layers(&mut self, world: &World) -> std::collections::HashMap<Uuid, Arc<SubjectPose>> {
        if !self.poser.active {
            return Default::default();
        }
        self.poser.sessions.retain(|id, _| allowed(world, *id));
        if !allowed(world, self.poser.subject) {
            self.poser.subject = world.agent_id;
            self.selected = None;
            self.refresh = true;
        }
        if !self.poser.sessions.contains_key(&self.poser.subject)
            && let Some(frame) = Frame::subject(world, self.poser.subject)
            && let Some(pose) = SubjectPose::capture(&world.avatar_lib.rig, &frame)
        {
            self.poser.sessions.insert(self.poser.subject, data::Session::new(pose));
        }
        self.poser
            .sessions
            .iter()
            .filter(|(_, s)| s.enabled && s.pose.valid(&world.avatar_lib.rig))
            .map(|(id, s)| (*id, s.pose.clone()))
            .collect()
    }
    pub(super) fn pick_subject(&mut self, world: &World, id: Uuid) {
        if !allowed(world, id) {
            return;
        }
        self.poser.subject = id;
        self.selected = None;
        self.refresh = true;
        self.drag = None;
        self.gizmo.cancel();
        self.pose_layers(world);
    }
    pub(in crate::ik_edit) fn commit_poser_manipulation(&mut self, rig: &Rig, frame: &Frame, rotate: bool) -> bool {
        let Some(bone) = self.selected else { return false };
        let rotation = Quat::from_euler(
            EulerRot::ZYX,
            self.degrees.z.to_radians(),
            self.degrees.y.to_radians(),
            self.degrees.x.to_radians(),
        );
        if !self.position.is_finite() || self.position.abs().max_element() > 100.0 || !rotation.is_finite() {
            return true;
        }
        let Some(session) = self.poser.sessions.get_mut(&self.poser.subject) else {
            return false;
        };
        if !session.enabled {
            return true;
        }
        let before = session.pose.resolved(rig, frame.model);
        let mut edited = (*session.pose).clone();
        edited.rotations.clone_from(&before.rotations);
        edited.positions.clone_from(&before.positions);
        edited.locks.remove(&bone);
        if rotate {
            let matrices = rig.world_with(&before.rotations, Vec3::ZERO, &before.positions, &edited.scales);
            let parent_q = rig.parent[bone].map_or(Quat::IDENTITY, |p| matrices[p].to_scale_rotation_translation().1);
            edited.rotations[bone] = (parent_q.inverse() * rotation).normalize();
        } else if !edited.move_joint(rig, bone, self.position) {
            return true;
        }
        if self.symmetry {
            let rotations = edited.rotations.clone();
            for (changed, rotation) in rotations.into_iter().enumerate() {
                if rotation != before.rotations[changed]
                    && let Some(other) = self.mirrors.counterpart(changed)
                {
                    edited.rotations[other] = data::mirrored(rig, rotation, changed, other);
                    edited.origin[other] = data::mirrored(rig, edited.origin[changed], changed, other);
                    edited.locks.remove(&other);
                }
            }
        }
        if !self.poser.drag_checkpoint {
            session.checkpoint();
            self.poser.drag_checkpoint = true;
        }
        session.pose = Arc::new(edited);
        true
    }
    /// The subject button: picture, name and kind, opening the subject menu
    /// (you and your animesh, then the nearby avatars with their consent
    /// action; the consent itself is the app's, through `Request`).
    pub(super) fn subject_button(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        let name = subject_name(world, &self.poser.subject_names, self.poser.subject);
        let kind = if self.poser.subject == world.agent_id {
            world.own_name()
        } else if world
            .objects
            .index_of_uuid(&self.poser.subject)
            .and_then(|i| world.objects.get(i))
            .is_some_and(|o| o.is_avatar())
        {
            aurora_i18n::tr!("poser-kind-consent").to_string()
        } else {
            aurora_i18n::tr!("poser-kind-animesh").to_string()
        };
        let font = egui::FontId::proportional(13.0);
        let small = egui::FontId::proportional(11.0);
        let text_w = ui
            .painter()
            .layout_no_wrap(name.clone(), font.clone(), p.ink)
            .size()
            .x
            .max(ui.painter().layout_no_wrap(kind.clone(), small.clone(), p.muted).size().x)
            .min(180.0);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(4.0 + 26.0 + 8.0 + text_w + 8.0 + 14.0 + 10.0, 36.0), Sense::click());
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
        let face = if open || response.hovered() {
            p.raised.lerp_to_gamma(p.ink, 0.12)
        } else {
            p.raised
        };
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
        let disc = egui::Rect::from_center_size(egui::pos2(rect.left() + 4.0 + 13.0, rect.center().y), egui::Vec2::splat(26.0));
        // your own initials, not those of "You"
        let initials = if self.poser.subject == world.agent_id { &kind } else { &name };
        widgets::avatar_disc(ui, p, disc, None, initials, false);
        let left = disc.right() + 8.0;
        widgets::paint_elided(ui, &name, egui::pos2(left, rect.center().y - 7.0), text_w, font, p.ink);
        widgets::paint_elided(ui, &kind, egui::pos2(left, rect.center().y + 8.0), text_w, small, p.muted);
        if let Some(chevron) = crate::ui::icons::global("chevron-down") {
            let at = egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0 - 7.0, rect.center().y), egui::Vec2::splat(14.0));
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(chevron.id(), at, uv, p.muted);
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        // AURORA_DEMO_POSER=consent-* / bridge-*: the menu opens by itself, as a click
        let demo = egui::Id::unique("poser-subjects-open");
        if ui.ctx().data_mut(|d| d.remove_temp::<bool>(demo)).unwrap_or(false) {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
        }
        crate::ui::menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(340.0);
            self.subject_menu(ui, p, world);
        });
    }
    fn subject_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        self.poser
            .subject_name_requested
            .retain(|_, at| at.elapsed() < std::time::Duration::from_secs(300));
        self.poser
            .subject_skip_logged
            .retain(|_, at| at.elapsed() < std::time::Duration::from_secs(300));
        use crate::world::poser_consent::Status;
        let edited = |editor: &Self, id: Uuid| {
            editor
                .poser
                .sessions
                .get(&id)
                .map_or(0, |s| (0..world.avatar_lib.rig.len()).filter(|b| s.edited(*b)).count())
        };
        menu::section(ui, p, aurora_i18n::text("poser-subject-hint"));
        let mut pick = None;
        let you = aurora_i18n::tr!("poser-edited-count", count = edited(self, world.agent_id)).to_string();
        if menu::item_detail(ui, p, "user-filled", aurora_i18n::text("poser-you"), &you) {
            pick = Some(world.agent_id);
        }
        for (_, object) in world.objects.iter() {
            let id = object.full_id;
            if id == world.agent_id || object.is_avatar() {
                continue;
            }
            match eligibility_reason(world, id) {
                None => {
                    if !self.poser.subject_names.contains_key(&id)
                        && self
                            .poser
                            .subject_name_requested
                            .get(&id)
                            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(30))
                    {
                        self.poser.subject_name_requested.insert(id, Instant::now());
                        self.poser.requests.push(Request::ObjectName(id));
                    }
                    let count = aurora_i18n::tr!("poser-edited-count", count = edited(self, object.full_id)).to_string();
                    if menu::item_detail(
                        ui,
                        p,
                        "accessible-filled",
                        &subject_name(world, &self.poser.subject_names, id),
                        &count,
                    ) {
                        pick = Some(object.full_id);
                    }
                }
                Some(reason)
                    if object.extra.extended_mesh_flags.unwrap_or(0) & aurora_prim::extra::EXTENDED_MESH_ANIMATED != 0
                        && self
                            .poser
                            .subject_skip_logged
                            .get(&id)
                            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(30)) =>
                {
                    log::debug!("poser animesh {} not offered: {reason}", object.key.local_id);
                    self.poser.subject_skip_logged.insert(id, Instant::now());
                }
                _ => {}
            }
        }
        menu::separator(ui, p);
        menu::section(ui, p, aurora_i18n::text("poser-nearby-section"));
        let mut any = false;
        for (_, object) in world.objects.avatars() {
            let peer = object.full_id;
            if peer == world.agent_id || !poser_consent::nearby(world, peer) {
                continue;
            }
            any = true;
            let status = world.poser_consent.status(peer);
            ui.horizontal(|ui| {
                ui.set_min_height(30.0);
                ui.add_space(8.0);
                ui.label(poser_consent::name(world, peer));
                if let Some(distance) = poser_consent::distance(world, peer) {
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("poser-distance", distance = format!("{distance:.0}")))
                            .size(11.0)
                            .color(p.muted_dim),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    let size = egui::vec2(0.0, 24.0);
                    match status {
                        Status::Ask => {
                            if widgets::action_button_if(
                                ui,
                                p,
                                "user-plus",
                                aurora_i18n::text("poser-consent-ask"),
                                widgets::ActionKind::Flat,
                                size,
                                self.poser.bridge == QueryState::Ready,
                            )
                            .clicked()
                            {
                                self.poser.requests.push(Request::Ask(peer));
                            }
                        }
                        Status::Pending => {
                            if widgets::flat_button(ui, p, aurora_i18n::text("poser-consent-pending")).clicked() {
                                self.poser.requests.push(Request::Cancel(peer));
                            }
                        }
                        Status::Accepted => {
                            if widgets::action_button(
                                ui,
                                p,
                                "check",
                                aurora_i18n::text("poser-consent-ready"),
                                widgets::ActionKind::Primary,
                                size,
                            )
                            .clicked()
                            {
                                pick = Some(peer);
                            }
                        }
                        Status::Refused | Status::NoAnswer => {
                            let key = if status == Status::Refused {
                                "poser-consent-refused"
                            } else {
                                "poser-consent-no-answer"
                            };
                            ui.label(egui::RichText::new(aurora_i18n::text(key)).size(11.0).color(p.muted));
                        }
                    }
                });
            });
        }
        if !any {
            menu::info(ui, p, "users", aurora_i18n::text("poser-nearby-none"));
        } else {
            // Say why Ask waits: a worn bridge whose query script is silent
            // needs Repair, not "wear it".
            let hint = match self.poser.bridge {
                QueryState::Ready => None,
                QueryState::Starting => Some(("hourglass-filled", "poser-bridge-starting")),
                QueryState::Silent => Some(("alert-triangle-filled", "poser-bridge-silent")),
                QueryState::Failed => Some(("alert-triangle-filled", "poser-bridge-failed")),
                QueryState::Detached => Some(("unlink", "poser-bridge-needed")),
                QueryState::Off => Some(("unlink", "poser-bridge-disabled")),
            };
            if let Some((icon, key)) = hint {
                menu::info(ui, p, icon, aurora_i18n::text(key));
            }
            if matches!(self.poser.bridge, QueryState::Silent | QueryState::Failed)
                && menu::item(ui, p, "refresh", aurora_i18n::text("bridge-repair"))
            {
                self.poser.requests.push(Request::RepairBridge);
            }
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "lock-filled", aurora_i18n::text("poser-privacy")) {
            self.poser.requests.push(Request::OpenPrivacy);
        }
        if let Some(id) = pick {
            self.pick_subject(world, id);
            ui.close();
        }
    }
}
