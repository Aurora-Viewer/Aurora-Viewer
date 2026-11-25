//! Subject eligibility is checked every frame, independently of the picker.
use super::*;
use crate::{
    ui::menu,
    world::{bridge::QueryState, poser_consent},
};

/// Where you stand, for the subjects' distance: computed once per menu
/// frame by the caller rather than once per object.
pub(super) fn own_position(world: &World) -> Option<Vec3> {
    let index = world.objects.index_of_uuid(&world.agent_id)?;
    Scene::object_transform(world, index, Instant::now(), 0).map(|t| t.0)
}
/// An animesh's own skeleton root (`animesh::owner`): a linkset root, rezzed
/// or worn, flagged animated. A plain test for the cheap flag comes first.
fn animesh_root(world: &World, index: usize, object: &crate::world::objects::Object) -> bool {
    object.extra.extended_mesh_flags.unwrap_or(0) & aurora_prim::extra::EXTENDED_MESH_ANIMATED != 0
        && Scene::skeleton_owner(world, index).is_some_and(|(owner, root)| owner == object.full_id && root == index)
}
pub(super) fn eligibility_reason(world: &World, id: Uuid) -> Option<&'static str> {
    eligibility_reason_from(world, id, own_position(world))
}
/// `eligibility_reason` with your position already known (`own_position`):
/// the cheap tests first, the object's own transform last.
pub(super) fn eligibility_reason_from(world: &World, id: Uuid, own_at: Option<Vec3>) -> Option<&'static str> {
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
    if !animesh_root(world, index, object) {
        return Some("not-root");
    }
    if !owned(world, object) {
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
    if !distance_from(world, index, own_at).is_some_and(|d| d <= 50.0) {
        return Some("too-far");
    }
    None
}
/// Whether an object is yours to pose. The simulator leaves OwnerID empty
/// in most object updates and fills it only with ObjectProperties (a
/// selection), while the update flags say "you own this" for every prim:
/// the Legacy Viewer's `permYouOwner` (llviewerobject.cpp) reads that flag
/// alone. A group-owned animesh you may manage carries the flag with its
/// group as owner, so the flag is trusted for a nil owner and for one of
/// your groups. Stricter than the Legacy Viewer: an owner that is known and
/// is neither you nor one of your groups wins over the flag.
fn owned(world: &World, object: &crate::world::objects::Object) -> bool {
    let owner = object.owner_id;
    if owner == world.agent_id {
        return true;
    }
    object.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
        && (owner.is_nil() || world.groups.groups.iter().any(|g| g.id == owner))
}
/// How far an object is from you (`own_position`), in metres.
fn distance_from(world: &World, index: usize, own_at: Option<Vec3>) -> Option<f32> {
    Scene::object_transform(world, index, Instant::now(), 0)
        .zip(own_at)
        .map(|(a, b)| a.0.distance(b))
}
/// Whether the poser can open on this subject now: an animesh whose
/// skeleton is not posed yet (its rigged mesh still loading) has no frame,
/// and the poser window would show nothing for it.
pub(super) fn has_frame(world: &World, id: Uuid) -> bool {
    id == world.agent_id || Frame::subject(world, id).is_some()
}
pub(super) fn allowed(world: &World, id: Uuid) -> bool {
    eligibility_reason(world, id).is_none()
}
/// A subject's name: "You", a resident's name, or an animesh's worn item
/// name, else its object name from ObjectProperties (`names`), else
/// "Animesh". The Legacy Viewer (`FSFloaterPoser::getControlAvatarName`,
/// fsfloaterposer.cpp) also takes the worn item's name, but names a rezzed
/// animesh "AO_<uuid>" (`LLControlAvatar::getFullname`); the object name is
/// a deliberate difference.
pub(super) fn subject_name(world: &World, names: &HashMap<Uuid, String>, id: Uuid) -> String {
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
/// The short name of a subject, for a narrow place (the compact window's
/// head, a picture's initials): a resident's display name alone, without
/// the account that `subject_name` adds; an animesh's name as there.
pub(super) fn subject_short_name(world: &World, names: &HashMap<Uuid, String>, id: Uuid) -> String {
    if id == world.agent_id {
        return world.own_name();
    }
    let avatars = &world.social.avatar_names;
    avatars
        .get(&id)
        .map(|n| n.display(&avatars.options))
        .unwrap_or_else(|| subject_name(world, names, id))
}
impl Editor {
    pub(in crate::ik_edit) fn editor_frame(&self, world: &World) -> Option<Frame> {
        let id = if self.poser.active && !self.poser.subject.is_nil() {
            self.poser.subject
        } else {
            // couple mode: the person the animation editor works on
            self.edited_subject(world)
        };
        Frame::subject(world, id)
    }
    pub fn pose_layers(&mut self, world: &World) -> std::collections::HashMap<Uuid, Arc<SubjectPose>> {
        self.pose_layers_with_base(world, |_| None)
    }

    pub fn pose_layers_with_base(
        &mut self,
        world: &World,
        mut base_for: impl FnMut(Uuid) -> Option<Arc<crate::scene::anim::SkeletonBase>>,
    ) -> std::collections::HashMap<Uuid, Arc<SubjectPose>> {
        if !self.poser.active {
            return Default::default();
        }
        self.poser.sessions.retain(|id, _| allowed(world, *id));
        if !allowed(world, self.poser.subject) {
            self.poser.subject = world.agent_id;
            self.selected = None;
            // a click beside on them must not leave you with none
            self.poser.unselected = false;
            self.poser.together.clear();
            self.refresh = true;
        }
        if !self.poser.sessions.contains_key(&self.poser.subject)
            && let Some(frame) = Frame::subject(world, self.poser.subject)
            && let Some(pose) = SubjectPose::capture(&world.avatar_lib.rig, &frame)
        {
            self.poser.sessions.insert(self.poser.subject, data::Session::new(pose));
        }
        for (id, session) in &mut self.poser.sessions {
            if let Some(base) = base_for(*id) {
                session.rebase(base);
            }
        }
        self.poser
            .sessions
            .iter()
            .filter(|(_, s)| s.enabled && s.pose.valid(&world.avatar_lib.rig))
            .map(|(id, s)| (*id, s.pose.clone()))
            .collect()
    }
    /// Who else a click can pose: those already posed, and the nearby
    /// avatars who accepted, before they are chosen once in the menu.
    pub(super) fn poseable_others(&self, world: &World) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self.poser.sessions.keys().copied().collect();
        ids.extend(
            world
                .objects
                .avatars()
                .map(|(_, o)| o.full_id)
                .filter(|id| *id != world.agent_id && allowed(world, *id)),
        );
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|id| *id != self.poser.subject);
        ids
    }
    pub(super) fn pick_subject(&mut self, world: &World, id: Uuid) {
        // without a frame the poser window shows nothing (`show_poser`)
        if !allowed(world, id) || !has_frame(world, id) {
            return;
        }
        // a Ctrl + click in the world names it as the menu does
        self.request_subject_name(world, id);
        self.poser.subject = id;
        self.selected = None;
        self.poser.unselected = false;
        self.poser.together.clear();
        self.refresh = true;
        self.drag = None;
        self.gizmo.cancel();
        self.pose_layers(world);
    }
    /// Asks the simulator for an animesh's object name (`Request::ObjectName`)
    /// when it has none yet, at most once every 30 seconds: from the subject
    /// menu and from a pick in the world. A resident is named elsewhere, and a
    /// worn animesh by its inventory item.
    pub(super) fn request_subject_name(&mut self, world: &World, id: Uuid) {
        let Some(object) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i)) else {
            return;
        };
        let worn_name = object
            .attachment_item_id()
            .is_some_and(|item| world.inventory.items.contains_key(&item));
        if id == world.agent_id || object.is_avatar() || worn_name || self.poser.subject_names.contains_key(&id) {
            return;
        }
        if self
            .poser
            .subject_name_requested
            .get(&id)
            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(30))
        {
            self.poser.subject_name_requested.insert(id, Instant::now());
            self.poser.requests.push(Request::ObjectName(id));
        }
    }
    pub(in crate::ik_edit) fn commit_poser_manipulation(&mut self, rig: &Rig, frame: &Frame, rotate: bool) -> bool {
        let Some(bone) = self.selected else { return false };
        let followers = self.followers();
        let selected = self.selected;
        let rotation = Quat::from_euler(
            EulerRot::ZYX,
            self.degrees.z.to_radians(),
            self.degrees.y.to_radians(),
            self.degrees.x.to_radians(),
        );
        if !self.position.is_finite() || self.position.abs().max_element() > 100.0 || !rotation.is_finite() {
            return true;
        }
        let move_joint_only = self.poser.move_joint_only;
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
        } else if !edited.move_joint(rig, bone, self.position, move_joint_only) {
            return true;
        }
        // the joints this drag changed itself (the leader, the followers and
        // the chains their moves bend): Mirror reflects them onto their
        // counterparts but never writes over one of them, so a selected pair
        // keeps its own solves while an unpaired follower still mirrors, as
        // with the sliders
        let mut dragged = vec![false; rig.len()];
        if !followers.is_empty() {
            let mut start = (*session.pose).clone();
            start.rotations.clone_from(&before.rotations);
            start.positions.clone_from(&before.positions);
            let change = if rotate { together::Change::Rotate } else { together::Change::Move };
            together::follow(rig, &start, &mut edited, bone, &followers, change, !rotate && move_joint_only);
            for (joint, dragged) in dragged.iter_mut().enumerate() {
                *dragged = edited.rotations[joint] != before.rotations[joint]
                    || edited.positions[joint] != before.positions[joint]
                    || joint == bone
                    || followers.contains(&joint);
            }
        }
        if self.symmetry {
            // a joint the drag moved directly (the leader or a follower with
            // "Move this joint only", or a joint whose parent is a spine joint
            // even without it): only its position changed, so the rotation
            // mirror below would miss it (FSPoserAnimator::setJointPosition
            // also moves the opposite joint in its MIRROR styles)
            let positions = edited.positions.clone();
            for (changed, position) in positions.into_iter().enumerate() {
                if position != before.positions[changed]
                    && let Some(other) = self.mirrors.counterpart(changed)
                    && Some(other) != selected
                    && !dragged[other]
                {
                    edited.positions[other] =
                        session.base.positions[other] + crate::ik_edit::rig_controls::reflect(position - session.base.positions[changed]);
                    edited.locks.remove(&other);
                }
            }
            let rotations = edited.rotations.clone();
            for (changed, rotation) in rotations.into_iter().enumerate() {
                // a joint selected with the others takes their change, not a reflection
                if rotation != before.rotations[changed]
                    && let Some(other) = self.mirrors.counterpart(changed)
                    && Some(other) != selected
                    && !dragged[other]
                {
                    edited.rotations[other] = data::mirrored(rig, rotation, changed, other);
                    edited.origin[other] = data::mirrored(rig, edited.origin[changed], changed, other);
                    edited.hold_rotation[other] = edited.hold_rotation[changed];
                    edited.locks.remove(&other);
                }
            }
        }
        if !self.poser.drag_checkpoint {
            session.checkpoint();
            self.poser.drag_checkpoint = true;
        }
        session.pose = Arc::new(edited);
        if self.clip.active && self.clip.document.as_ref().is_some_and(|document| document.auto_keyframe) {
            self.clip.modified.insert(bone);
            if !rotate {
                self.clip.translations.insert(bone);
            }
            self.clip.auto_key_pending = true;
        }
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
            self.subject_menu(ui, p, world);
        });
    }
    /// The subject menu, in the People list's look (its rows: the round
    /// picture, the name over the account name, the face lit under the
    /// pointer): you and your animesh, then the nearby avatars with their
    /// distance and a square button for the consent (Ask, Asked, Accepted,
    /// Refused), and the privacy setting.
    pub(super) fn subject_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        self.poser
            .subject_name_requested
            .retain(|_, at| at.elapsed() < std::time::Duration::from_secs(300));
        self.poser
            .subject_skip_logged
            .retain(|_, at| at.elapsed() < std::time::Duration::from_secs(300));
        // the names of objects that are gone
        self.poser.subject_names.retain(|id, _| world.objects.index_of_uuid(id).is_some());
        use crate::world::poser_consent::Status;
        ui.set_width(MENU_W);
        let now = ui.input(|i| i.time);
        let edited = |editor: &Self, id: Uuid| {
            editor
                .poser
                .sessions
                .get(&id)
                .map_or(0, |s| (0..world.avatar_lib.rig.len()).filter(|b| s.edited(*b)).count())
        };
        menu::section(ui, p, aurora_i18n::text("poser-subject-hint"));
        let mut pick = None;
        let own = world.own_name();
        let you = aurora_i18n::tr!("poser-edited-count", count = edited(self, world.agent_id)).to_string();
        let picture = self.picture(world.agent_id, now);
        let current = self.poser.subject == world.agent_id;
        let you = Detail { text: you, end: None };
        if person_row(
            ui,
            p,
            world.agent_id,
            Avatar::Picture(picture.as_ref(), &own),
            aurora_i18n::text("poser-you"),
            &you,
            current,
            None,
        )
        .0
        {
            pick = Some(world.agent_id);
        }
        // your position once per frame, not once per object
        let own_at = own_position(world);
        for (index, object) in world.objects.iter() {
            let id = object.full_id;
            // an animesh is an animated linkset root: every other prim
            // (most of what you own) stops at these two plain tests
            if id == world.agent_id || object.is_avatar() || !animesh_root(world, index, object) {
                continue;
            }
            // a rigged mesh still loading has no pose yet: the poser would
            // open on nothing (`Frame::subject`), so it waits in the log
            let reason = eligibility_reason_from(world, id, own_at).or_else(|| (!has_frame(world, id)).then_some("no-pose"));
            match reason {
                None => {
                    self.request_subject_name(world, id);
                    let count = aurora_i18n::tr!("poser-edited-count", count = edited(self, id)).to_string();
                    let current = self.poser.subject == id;
                    let name = subject_name(world, &self.poser.subject_names, id);
                    // the distance, as the nearby residents' rows: it tells
                    // apart several animesh whose names are not known yet
                    let end = distance_from(world, index, own_at).map(|d| {
                        let away = aurora_i18n::tr!("poser-distance", distance = aurora_i18n::number(f64::from(d), 1)).to_string();
                        (away, range_color(p, d))
                    });
                    let count = Detail { text: count, end };
                    if person_row(ui, p, id, Avatar::Icon("accessible-filled"), &name, &count, current, None).0 {
                        pick = Some(id);
                    }
                }
                Some(reason)
                    if self
                        .poser
                        .subject_skip_logged
                        .get(&id)
                        .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(30)) =>
                {
                    log::debug!("poser animesh {} not offered: {reason}", object.key.local_id);
                    self.poser.subject_skip_logged.insert(id, Instant::now());
                }
                Some(_) => {}
            }
        }
        ui.add_space(4.0);
        menu::section(ui, p, aurora_i18n::text("poser-nearby-section"));
        let mut peers: Vec<(Uuid, f32)> = world
            .objects
            .avatars()
            .map(|(_, o)| o.full_id)
            .filter(|id| *id != world.agent_id && poser_consent::nearby(world, *id))
            .map(|id| (id, poser_consent::distance(world, id).unwrap_or(f32::MAX)))
            .collect();
        // nearest first, as Near you
        peers.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (peer, distance) in &peers {
            let peer = *peer;
            let status = world.poser_consent.status(peer);
            let names = &world.social.avatar_names;
            let name = names
                .get(&peer)
                .map_or_else(|| poser_consent::name(world, peer), |n| n.display(&names.options));
            let account = names.get(&peer).map(|n| n.username.clone()).filter(|u| *u != name);
            let away = aurora_i18n::tr!("poser-distance", distance = aurora_i18n::number(f64::from(*distance), 1)).to_string();
            let detail = Detail {
                text: account.unwrap_or_default(),
                end: Some((away, range_color(p, *distance))),
            };
            let ready = self.poser.bridge == QueryState::Ready;
            let consent = match status {
                Status::Ask => Consent {
                    icon: "user-plus",
                    label: aurora_i18n::text("poser-consent-ask"),
                    tip: aurora_i18n::text("poser-ask-tip"),
                    accent: false,
                    enabled: ready,
                },
                Status::Pending => Consent {
                    icon: "hourglass-filled",
                    label: aurora_i18n::text("poser-asked"),
                    tip: aurora_i18n::text("poser-consent-pending"),
                    accent: false,
                    enabled: true,
                },
                Status::Accepted => Consent {
                    icon: "check",
                    label: aurora_i18n::text("poser-accepted"),
                    tip: aurora_i18n::text("poser-consent-ready"),
                    accent: true,
                    enabled: true,
                },
                Status::Refused => Consent {
                    icon: "x",
                    label: aurora_i18n::text("poser-refused"),
                    tip: aurora_i18n::text("poser-consent-refused"),
                    accent: false,
                    enabled: false,
                },
                Status::NoAnswer => Consent {
                    icon: "clock-filled",
                    label: aurora_i18n::text("poser-no-answer"),
                    tip: aurora_i18n::text("poser-consent-no-answer"),
                    accent: false,
                    enabled: false,
                },
            };
            let picture = self.picture(peer, now);
            let current = self.poser.subject == peer;
            let (row, button) = person_row(
                ui,
                p,
                peer,
                Avatar::Picture(picture.as_ref(), &name),
                &name,
                &detail,
                current,
                Some(consent),
            );
            match status {
                Status::Ask if button => self.poser.requests.push(Request::Ask(peer)),
                Status::Pending if button => self.poser.requests.push(Request::Cancel(peer)),
                Status::Accepted if button || row => pick = Some(peer),
                _ => {}
            }
        }
        if peers.is_empty() {
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
        if let Some(id) = pick {
            self.pick_subject(world, id);
            ui.close();
        }
    }
}

/// The subject menu's width: a People row's.
pub(super) const MENU_W: f32 = 270.0;
/// A row's height and the gap between two, as Near you's rich rows.
const ROW_H: f32 = 42.0;
const ROW_GAP: f32 = 3.0;
/// The consent button: square-ish, over both lines of the row.
const CONSENT: egui::Vec2 = egui::vec2(58.0, 34.0);

/// What a row's left shows: a resident's picture (initials without it) or
/// an object's symbol.
enum Avatar<'a> {
    Picture(Option<&'a egui::TextureHandle>, &'a str),
    Icon(&'a str),
}

/// A row's second line: the account name (or the joints edited), and the
/// distance kept whole at its right end, the text cut before it.
struct Detail {
    text: String,
    end: Option<(String, egui::Color32)>,
}

/// The distance's colour, as Near you's figures (`people::nearby::Band`):
/// turquoise within a whisper, green within the chat, amber within a shout.
fn range_color(p: &Palette, distance: f32) -> egui::Color32 {
    match distance {
        d if d <= 10.0 => p.teal,
        d if d <= 20.0 => p.success,
        d if d <= 100.0 => p.amber,
        _ => p.muted,
    }
}

/// The consent button of a nearby avatar's row.
struct Consent<'a> {
    icon: &'a str,
    label: &'a str,
    tip: &'a str,
    /// Accepted: the accent's face.
    accent: bool,
    enabled: bool,
}

/// One row in People's look (`ui::people::rows`: the face, the 30 px
/// picture, the semibold name over the muted detail); `current` gets the
/// accent's wash of a selected row. Returns (row clicked, button clicked).
#[allow(clippy::too_many_arguments)]
fn person_row(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    avatar: Avatar,
    name: &str,
    detail: &Detail,
    current: bool,
    consent: Option<Consent>,
) -> (bool, bool) {
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), ROW_H));
    ui.add_space(ROW_GAP - ui.spacing().item_spacing.y);
    let row = ui.interact(rect, ui.scope_id().with(("poser-person-row", id)), Sense::click());
    let button_rect = consent
        .as_ref()
        .map(|_| egui::Rect::from_center_size(egui::pos2(rect.right() - 4.0 - CONSENT.x * 0.5, rect.center().y), CONSENT));
    let button = button_rect.map(|r| ui.interact(r, ui.scope_id().with(("poser-consent", id)), Sense::click()));
    let on_button = button.as_ref().is_some_and(|b| b.hovered());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = if current {
        p.violet.gamma_multiply(0.18)
    } else if row.hovered() && !on_button {
        p.raised.gamma_multiply(0.45)
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, radius, fill);
    let disc = egui::Rect::from_center_size(egui::pos2(rect.left() + 8.0 + 15.0, rect.center().y), egui::Vec2::splat(30.0));
    match avatar {
        Avatar::Picture(pic, initials) => widgets::avatar_disc(ui, p, disc, pic, initials, false),
        Avatar::Icon(icon) => {
            ui.painter().circle_filled(disc.center(), 15.0, p.raised);
            crate::ui::viewfinder::icon(
                ui.painter(),
                icon,
                egui::Rect::from_center_size(disc.center(), egui::Vec2::splat(16.0)),
                p.muted,
            );
        }
    }
    let left = disc.right() + 10.0;
    let mut right = button_rect.map_or(rect.right() - 8.0, |b| b.left() - 8.0);
    // the distance, centred on the row at its right as Near you's
    if let Some((end, color)) = &detail.end {
        let galley = ui.painter().layout_job(egui::text::LayoutJob::single_section(
            end.clone(),
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 13.0, *color),
        ));
        let width = galley.size().x;
        widgets::paint_row_text(ui.painter(), galley, right - width, rect.center().y, *color);
        right -= width + 10.0;
    }
    let width = (right - left).max(0.0);
    let title = egui::FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()));
    let cut = widgets::paint_elided(ui, name, egui::pos2(left, rect.center().y - 8.0), width, title, p.ink);
    let small = egui::FontId::proportional(11.0);
    let detail_cut = widgets::paint_elided(ui, &detail.text, egui::pos2(left, rect.center().y + 9.0), width, small, p.muted);
    let mut clicked_button = false;
    if let (Some(consent), Some(b), Some(at)) = (consent, button, button_rect) {
        widgets::paint_tile_button(ui, p, at, consent.icon, consent.label, consent.accent, consent.enabled, b.hovered());
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, consent.enabled, consent.label));
        clicked_button = consent.enabled && b.on_hover_text(consent.tip).clicked();
    }
    row.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, current, name));
    // a cut name or account shows whole under the pointer
    let row = if cut || detail_cut {
        row.on_hover_text(format!("{name}\n{}", detail.text))
    } else {
        row
    };
    (row.clicked() && !on_button, clicked_button)
}
