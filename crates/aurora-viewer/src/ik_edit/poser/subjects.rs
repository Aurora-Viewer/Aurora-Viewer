//! Subject eligibility is checked every frame, independently of the picker.
use super::*;
use crate::world::poser_consent;

pub(super) fn allowed(world: &World, id: Uuid) -> bool {
    if id == world.agent_id {
        return !id.is_nil();
    }
    let Some(index) = world.objects.index_of_uuid(&id) else {
        return false;
    };
    let Some(object) = world.objects.get(index) else { return false };
    if object.is_avatar() {
        return world.poser_consent.can_pose(world, id);
    }
    let Some(own) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i)) else {
        return false;
    };
    object.owner_id == world.agent_id
        && object.key.region == own.key.region
        && Some(object.key.region) == world.main_region
        && Scene::object_transform(world, index, Instant::now(), 0)
            .zip(
                world
                    .objects
                    .index_of_uuid(&world.agent_id)
                    .and_then(|i| Scene::object_transform(world, i, Instant::now(), 0)),
            )
            .is_some_and(|(a, b)| a.0.distance(b.0) <= 50.0)
        && !world.mutes.is_muted_id(&id)
        && Scene::skeleton_owner(world, index).is_some_and(|(owner, root)| owner == id && root == index)
}
fn subject_name(world: &World, id: Uuid) -> String {
    if id == world.agent_id {
        return aurora_i18n::tr!("poser-you").to_string();
    }
    if let Some(object) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i))
        && !object.is_avatar()
    {
        return format!("{} {}", aurora_i18n::text("poser-animesh"), object.key.local_id);
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
                    let relative = rig.local_rot[changed].inverse() * rotation;
                    edited.rotations[other] =
                        (rig.local_rot[other] * Quat::from_xyzw(-relative.x, relative.y, -relative.z, relative.w)).normalize();
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
    pub(super) fn subject_picker(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        let current = subject_name(world, self.poser.subject);
        egui::ComboBox::from_id_salt("poser-subject")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.poser.subject == world.agent_id, aurora_i18n::tr!("poser-you"))
                    .clicked()
                {
                    self.pick_subject(world, world.agent_id);
                }
                for (_, object) in world.objects.iter() {
                    if object.full_id != world.agent_id && allowed(world, object.full_id) {
                        let name = subject_name(world, object.full_id);
                        if ui.selectable_label(self.poser.subject == object.full_id, name).clicked() {
                            self.pick_subject(world, object.full_id);
                        }
                    }
                }
            });
        ui.colored_label(p.muted, aurora_i18n::tr!("poser-subject-hint"));
    }
}
