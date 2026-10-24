//! Session-only placement of static Blender sources. Attachment point frames
//! follow LLViewerJointAttachment; dropping follows LLSelectMgr::sendDropAttachment
//! (indra/newview/llviewerjointattachment.cpp, llselectmgr.cpp).
use super::{Live, SourceState};
use crate::world::{World, objects::PREVIEW_REGION};
use glam::{Quat, Vec3};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    World,
    Attachment(u8),
    Detached,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build::{BuildSettings, BuildTool},
        scene::Scene,
    };
    use aurora_net::{NetCommand, build::BuildCmd};

    fn fixture() -> (Live, World, Uuid, usize, aurora_net::objects::ObjectUpdate) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let mut live = Live::default();
        let source = Uuid::new_v4();
        let size = Vec3::new(0.5, 0.4, 0.3);
        let placement = live.ensure_placement(source, size, &world);
        let mut object = crate::demo::hud::object(31, false);
        object.full_id = Uuid::new_v4();
        object.local_id = 1;
        object.parent_id = 0;
        object.state = 0;
        object.position = placement.position;
        object.rotation = Quat::IDENTITY;
        object.scale = size;
        object.update_flags |= crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY;
        let idx = world.objects.upsert_preview(None, object.clone());
        live.instances.insert((source, 0), object.full_id);
        live.objects.push(object.full_id);
        live.world_origin = world.main_origin();
        (live, world, source, idx, object)
    }

    #[test]
    fn blender_object_normal_edit_tools_keep_placement_and_never_address_a_simulator() {
        let (mut live, mut world, source, idx, mut update) = fixture();
        let root_key = world.objects.get(idx).expect("root").key;
        update.local_id = 2;
        update.full_id = Uuid::new_v4();
        update.position = Vec3::ZERO;
        let second = world.objects.upsert_preview(Some(root_key), update.clone());
        live.instances.insert((source, 1), update.full_id);
        live.objects.push(update.full_id);
        let mut touched = Vec::new();
        world.objects.take_touched(&mut touched);
        for _ in 0..60 {
            live.capture_placements(&mut world);
        }
        touched.clear();
        world.objects.take_touched(&mut touched);
        assert!(touched.is_empty(), "idle placement capture must not dirty draw objects");
        assert!(crate::build::selectable(&world, idx));
        let settings = BuildSettings::default();
        let mut build = BuildTool::default();
        build.click_select(&mut world, &settings, Some(idx), false);
        let position = Vec3::new(130.0, 125.0, 25.0);
        let rotation = Quat::from_rotation_z(0.6);
        build.set_transform(
            &mut world,
            &settings,
            Some(position),
            Some(Vec3::new(1.0, 0.8, 0.6)),
            Some(rotation),
            true,
        );
        for command in &build.out {
            assert!(live.local_command(command, &world));
        }
        live.capture_placements(&mut world);
        let placement = live.placements[&source];
        assert!(placement.position.abs_diff_eq(position, 1e-5));
        assert!(placement.rotation.abs_diff_eq(rotation, 1e-5));
        assert!(placement.scale.abs_diff_eq(Vec3::splat(2.0), 1e-5));
        assert!(
            world
                .objects
                .get(second)
                .expect("second render batch")
                .scale
                .abs_diff_eq(Vec3::new(1.0, 0.8, 0.6), 1e-5)
        );
        assert!(
            live.ensure_placement(source, Vec3::splat(0.6), &world)
                .scale
                .abs_diff_eq(Vec3::splat(2.0), 1e-5)
        );
        let before = Scene::object_transform(&world, idx, std::time::Instant::now(), 0).expect("placed");
        world.agent.position += Vec3::X * 5.0;
        assert_eq!(
            Scene::object_transform(&world, idx, std::time::Instant::now(), 0).expect("placed"),
            before
        );
        assert!(!live.local_command(
            &NetCommand::Build(BuildCmd::Select {
                handle: world.main_region.expect("region"),
                local_ids: vec![1]
            }),
            &world
        ));
    }

    #[test]
    fn blender_object_body_and_hud_reparent_without_losing_identity_or_selection() {
        let (mut live, mut world, source, idx, mut update) = fixture();
        let parent = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|i| world.objects.get(i))
            .expect("avatar")
            .key;
        let original_key = world.objects.get(idx).expect("object").key;
        for point in [6u8, 31] {
            live.place_object(source, Action::Attach(point), &world);
            update.state = point.rotate_left(4);
            update.position = Vec3::new(0.0, 0.15, 0.0);
            let again = world.objects.upsert_preview(Some(parent), update.clone());
            assert_eq!(again, idx);
            assert_eq!(world.objects.get(idx).expect("object").key, original_key);
            assert!(Scene::object_transform(&world, idx, std::time::Instant::now(), 0).is_some_and(|(_, _, hud)| hud == (point == 31)));
            assert!(crate::build::selectable(&world, idx));
        }
        live.place_object(source, Action::Detach, &world);
        live.capture_placements(&mut world);
        assert_eq!(live.placements[&source].location, Location::Detached);
        live.place_object(source, Action::Rez, &world);
        assert_eq!(live.placements[&source].location, Location::World);
    }

    #[test]
    fn blender_object_world_origin_changes_keep_global_position_and_drop_uses_rotated_bounds() {
        let (mut live, mut world, source, idx, _) = fixture();
        let original = world.objects.get(idx).expect("object").position;
        let (x, y) = world.main_origin().expect("origin");
        world.main_region = Some(((x as u64 + 256) << 32) | y as u64);
        live.capture_placements(&mut world);
        assert!(live.placements[&source].position.abs_diff_eq(original - Vec3::X * 256.0, 1e-5));
        let position = Vec3::new(10.0, 12.0, 30.0);
        if let Some(object) = world.objects.get_mut(idx) {
            object.position = position;
            object.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        }
        live.place_object(source, Action::Drop, &world);
        let placement = live.placements[&source];
        assert!(placement.position.truncate().abs_diff_eq(position.truncate(), 1e-5));
        assert!((placement.position.z - (world.ground_height(position).unwrap_or(world.agent.position.z - 1.0) + 0.25)).abs() < 1e-4);
        live.capture_placements(&mut world);
        assert_eq!(
            live.placements[&source].position, placement.position,
            "pending staging cannot overwrite the requested placement"
        );
    }

    #[test]
    fn blender_object_numeric_edits_and_authored_center_changes_preserve_mesh_and_user_scale() {
        let (mut live, mut world, source, idx, _) = fixture();
        let volume = world.objects.get(idx).expect("object").volume;
        let position = Vec3::new(120.0, 125.0, 26.0);
        let rotation = Quat::from_rotation_z(0.5);
        live.place_object(
            source,
            Action::Transform {
                position,
                rotation,
                scale: Vec3::splat(2.0),
            },
            &world,
        );
        live.apply_transform(source, &mut world);
        let object = world.objects.get(idx).expect("object");
        assert_eq!(object.volume, volume);
        assert_eq!(object.position, position);
        assert!(object.scale.abs_diff_eq(Vec3::new(1.0, 0.8, 0.6), 1e-5));
        let bounds = live.update_bounds(source, Vec3::ONE, Vec3::X, &world);
        assert!(bounds.position.abs_diff_eq(position + rotation * Vec3::X * 2.0, 1e-5));
        assert!(bounds.scale.abs_diff_eq(Vec3::splat(2.0), 1e-5));
        assert_eq!(
            live.update_bounds(source, Vec3::ONE, Vec3::X, &world).position,
            bounds.position,
            "render batches must not apply the shift twice"
        );
    }
}

#[derive(Clone, Copy)]
pub struct Placement {
    /// Linked sources retain their own geometry and editable local transform.
    pub parent: Option<Uuid>,
    pub offset: Vec3,
    pub location: Location,
    pub position: Vec3,
    pub rotation: Quat,
    /// User scale multiplier, independent of subsequent Blender geometry edits.
    pub scale: Vec3,
    pub size: Vec3,
    pub center: Vec3,
}

#[derive(Clone, Copy)]
pub enum Action {
    Rez,
    Drop,
    Derez,
    Attach(u8),
    Detach,
    Edit,
    Transform { position: Vec3, rotation: Quat, scale: Vec3 },
}

impl Live {
    pub fn source_for_object(&self, id: Uuid) -> Option<Uuid> {
        self.instances
            .iter()
            .find_map(|(&(source, _), &object)| (object == id).then_some(source))
    }

    pub fn root(&self, source: Uuid) -> Option<Uuid> {
        self.instances.get(&(source, 0)).copied()
    }

    /// Capture manipulator edits before a revision replaces the asset. Rebase
    /// world previews when the viewer changes its rendering origin at a border.
    pub fn capture_placements(&mut self, world: &mut World) {
        let origin = world.main_origin();
        let shift = match (self.world_origin, origin) {
            (Some((x, y)), Some((nx, ny))) => Vec3::new((x as f64 - nx as f64) as f32, (y as f64 - ny as f64) as f32, 0.0),
            _ => Vec3::ZERO,
        };
        self.world_origin = origin;
        let mut resized = Vec::new();
        for (&source, placement) in &mut self.placements {
            if self.placement_dirty.contains(&source) {
                if placement.parent.is_none() && placement.location == Location::World {
                    placement.position += shift;
                }
                continue;
            }
            let root = self.instances.get(&(source, 0)).and_then(|id| world.objects.index_of_uuid(id));
            if let Some(index) = root {
                if placement.parent.is_none()
                    && placement.location == Location::World
                    && shift != Vec3::ZERO
                    && let Some(object) = world.objects.get_mut(index)
                {
                    object.position += shift;
                    object.render.needs_records = true;
                }
                let Some(object) = world.objects.get(index) else { continue };
                let moved = placement.parent.is_some()
                    && (!placement.position.abs_diff_eq(object.position, 1e-6) || !placement.rotation.abs_diff_eq(object.rotation, 1e-6));
                placement.position = object.position;
                placement.rotation = object.rotation;
                let scale = object.scale / placement.size;
                if moved || !placement.scale.abs_diff_eq(scale, 1e-6) {
                    resized.push(source);
                }
                placement.scale = scale;
            } else if placement.parent.is_none() && placement.location == Location::World {
                placement.position += shift;
            }
        }
        for source in resized {
            self.apply_transform(source, world);
        }
    }

    pub fn ensure_placement(&mut self, source: Uuid, size: Vec3, world: &World) -> Placement {
        let placement = self.placements.entry(source).or_insert_with(|| {
            let mut position = world.agent.position + world.agent.body_rotation() * Vec3::new(2.0, 0.0, 0.0);
            position.z = world.ground_height(position).unwrap_or(world.agent.position.z - 1.0) + size.z * 0.5;
            Placement {
                parent: None,
                offset: Vec3::ZERO,
                location: Location::World,
                position,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                size,
                center: Vec3::ZERO,
            }
        });
        placement.size = size;
        *placement
    }

    pub fn update_bounds(&mut self, source: Uuid, size: Vec3, center: Vec3, world: &World) -> Placement {
        let existed = self.placements.contains_key(&source);
        let mut placement = self.ensure_placement(source, size, world);
        if existed {
            placement.position += placement.rotation * ((center - placement.center) * placement.scale);
        }
        placement.center = center;
        self.placements.insert(source, placement);
        placement
    }

    /// Numeric edits only update transforms; keep mesh bytes, decoded geometry
    /// and GPU resources intact, exactly as the normal manipulators do.
    pub fn apply_transform(&mut self, source: Uuid, world: &mut World) {
        let Some(placement) = self.placements.get(&source) else { return };
        for (&(id, part), object) in &self.instances {
            if id != source && self.placements.get(&id).is_none_or(|p| p.parent != Some(source)) {
                continue;
            }
            let placement = self.placements.get(&id).unwrap_or(placement);
            if let Some(object) = world.objects.index_of_uuid(object).and_then(|idx| world.objects.get_mut(idx)) {
                if part == 0 || placement.parent.is_some() {
                    object.position = placement.position;
                    object.rotation = placement.rotation;
                }
                object.scale = placement.size * placement.scale;
                object.render.needs_records = true;
            }
        }
        self.placement_dirty.remove(&source);
    }

    pub fn place_object(&mut self, source: Uuid, action: Action, world: &World) {
        let source = if matches!(action, Action::Transform { .. } | Action::Edit) {
            source
        } else {
            self.linked_root(source)
        };
        if let Action::Transform { scale, position, rotation } = action
            && let Some(old) = self.placements.get(&source)
            && scale.is_finite()
            && scale.min_element() > 0.0
            && position.is_finite()
            && rotation.is_finite()
            && rotation.length_squared() >= 1e-12
        {
            let ratio = scale / old.scale;
            for child in self.placements.values_mut().filter(|p| p.parent == Some(source)) {
                child.position *= ratio;
                child.scale *= ratio;
            }
        }
        let Some(placement) = self.placements.get_mut(&source) else {
            return;
        };
        match action {
            Action::Rez | Action::Drop => {
                if matches!(action, Action::Drop)
                    && let Some(idx) = self.instances.get(&(source, 0)).and_then(|id| world.objects.index_of_uuid(id))
                    && let Some((position, rotation, hud)) = crate::scene::Scene::object_transform(world, idx, std::time::Instant::now(), 0)
                    && !hud
                {
                    placement.position = position;
                    placement.rotation = rotation;
                } else {
                    placement.position = world.agent.position + world.agent.body_rotation() * Vec3::new(2.0, 0.0, 0.0);
                    placement.rotation = Quat::IDENTITY;
                }
                placement.location = Location::World;
                self.states.insert(source, SourceState::Visible);
            }
            Action::Derez | Action::Detach => placement.location = Location::Detached,
            Action::Attach(point) => {
                if !world.avatar_lib.attach_points.contains_key(&point) {
                    return;
                }
                placement.location = Location::Attachment(point);
                placement.position = Vec3::ZERO;
                placement.rotation = Quat::IDENTITY;
                self.states.insert(source, SourceState::Visible);
            }
            Action::Transform { position, rotation, scale } => {
                if !position.is_finite()
                    || !rotation.is_finite()
                    || rotation.length_squared() < 1e-12
                    || !scale.is_finite()
                    || scale.min_element() <= 0.0
                {
                    return;
                }
                placement.position = position;
                placement.rotation = rotation.normalize();
                placement.scale = scale;
            }
            Action::Edit => return,
        }
        if matches!(action, Action::Rez | Action::Drop) {
            let offset = self.floor_offset(source);
            if let Some(placement) = self.placements.get_mut(&source) {
                placement.position.z = world.ground_height(placement.position).unwrap_or(world.agent.position.z - 1.0) + offset;
            }
        }
        let location = self.placements[&source].location;
        for child in self.placements.values_mut().filter(|p| p.parent == Some(source)) {
            child.location = location;
        }
        self.placement_dirty.insert(source);
        self.revision = None;
    }

    /// Consume local tool commands before the demo simulator or network sees
    /// them. The namespace can never resolve to a real simulator connection.
    pub fn local_command(&mut self, command: &aurora_net::NetCommand, world: &World) -> bool {
        use aurora_net::{NetCommand, build::BuildCmd};
        let (handle, ids, action) = match command {
            NetCommand::Build(BuildCmd::Select { handle, .. } | BuildCmd::Deselect { handle, .. } | BuildCmd::Transform { handle, .. }) => {
                return *handle == PREVIEW_REGION;
            }
            NetCommand::Build(BuildCmd::DeRez { handle, local_ids, .. }) => (*handle, local_ids.clone(), Action::Derez),
            NetCommand::Build(BuildCmd::Attach { handle, objects, point }) => {
                (*handle, objects.iter().map(|(id, _)| *id).collect(), Action::Attach(*point & 0x7f))
            }
            NetCommand::Build(BuildCmd::DropAttachment { handle, local_ids }) => (*handle, local_ids.clone(), Action::Drop),
            _ => return false,
        };
        if handle != PREVIEW_REGION {
            return false;
        }
        for id in ids {
            let key = crate::world::objects::ObjKey {
                region: handle,
                local_id: id,
            };
            if let Some(source) = world
                .objects
                .index_of(&key)
                .and_then(|idx| world.objects.get(idx))
                .and_then(|o| self.source_for_object(o.full_id))
            {
                self.place_object(source, action, world);
            }
        }
        true
    }
}
