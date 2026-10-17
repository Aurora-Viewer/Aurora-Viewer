//! Mesh joint overrides, following LLVOAvatar::addAttachmentOverridesForObject
//! (indra/newview/llvoavatar.cpp), LLVector3OverrideMap (lljoint.cpp) and
//! LLAvatarAppearance::hasPelvisFixup (llavatarappearance.cpp), originally LGPL 2.1.

use super::anim::Rig;
use aurora_assets::SkinInfo;
use glam::Vec3;
use uuid::Uuid;

pub(crate) struct JointOverrides {
    positions: Vec<Option<(Uuid, Vec3)>>,
    // Scale overrides are independent: a losing position override can still
    // lock the scale. Every such override requests the skeleton's default.
    lock_scale: Vec<bool>,
    pelvis_fixup: Option<(Uuid, f32)>,
}

impl JointOverrides {
    pub fn new(rig: &Rig) -> Self {
        Self {
            positions: vec![None; rig.len()],
            lock_scale: vec![false; rig.len()],
            pelvis_fixup: None,
        }
    }

    pub fn add(&mut self, rig: &Rig, mesh: Uuid, skin: &SkinInfo) {
        if mesh.is_nil() || skin.alt_inverse_bind.is_empty() || skin.alt_inverse_bind.len() != skin.joint_names.len() {
            return;
        }
        for (name, matrix) in skin.joint_names.iter().zip(&skin.alt_inverse_bind) {
            let Some(&joint) = rig.names.get(name) else { continue };
            let position = matrix.w_axis.truncate();
            // LLJoint::aboveJointPosThreshold: strictly more than 0.1 mm
            // from the XML default, including mPelvis (not the shaped rest).
            const THRESHOLD: f32 = 0.0001;
            if !position.is_finite() || (position - rig.local_pos[joint]).length_squared() <= THRESHOLD * THRESHOLD {
                continue;
            }
            let active = &mut self.positions[joint];
            if active.is_none_or(|(id, _)| mesh >= id) {
                *active = Some((mesh, position));
            }
            self.lock_scale[joint] |= skin.lock_scale_if_joint_position;
        }
        if skin.pelvis_offset.is_finite() && skin.pelvis_offset != 0.0 && self.pelvis_fixup.is_none_or(|(id, _)| mesh >= id) {
            self.pelvis_fixup = Some((mesh, skin.pelvis_offset));
        }
    }

    pub fn apply(&self, rig: &Rig, positions: &mut [Vec3], scales: &mut [Vec3]) {
        for (joint, active) in self.positions.iter().enumerate() {
            if let Some((_, position)) = active {
                // Legacy's pelvis is relative to mRoot, initially zero.
                // Our matrices use feet-origin skeleton space instead.
                positions[joint] = *position + if joint == rig.pelvis { rig.local_pos[joint] } else { Vec3::ZERO };
            }
            if self.lock_scale[joint] {
                scales[joint] = rig.scale[joint];
            }
        }
    }

    pub fn pelvis_fixup(&self) -> f32 {
        self.pelvis_fixup.map_or(0.0, |(_, offset)| offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Scene, anchors, anim, avatar::AvatarLibrary, shape};
    use crate::world::World;
    use aurora_net::NetEvent;
    use glam::{Mat4, Quat};
    use std::{sync::Arc, time::Instant};

    fn skin(joints: &[(&str, Vec3)], lock: bool, pelvis_offset: f32) -> SkinInfo {
        SkinInfo {
            joint_names: joints.iter().map(|(name, _)| (*name).into()).collect(),
            inverse_bind: vec![Mat4::IDENTITY; joints.len()],
            alt_inverse_bind: joints.iter().map(|(_, pos)| Mat4::from_translation(*pos)).collect(),
            lock_scale_if_joint_position: lock,
            pelvis_offset,
            ..Default::default()
        }
    }

    fn fixture() -> (Scene, World, usize) {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in crate::demo::events().into_iter().chain(crate::demo::worn::events()) {
            world.apply(event);
        }
        let wearer = world.objects.index_of_uuid(&crate::demo::DEMO_AGENT).expect("wearer");
        let scene = Scene::new(std::path::PathBuf::new(), lib);
        (scene, world, wearer)
    }

    fn mesh(world: &mut World, local: u32, asset: u128) -> usize {
        let mut object = crate::demo::worn::root();
        object.local_id = local;
        object.full_id = Uuid::from_u128(u128::from(local));
        let sculpt = aurora_prim::params::SculptParams {
            texture: Uuid::from_u128(asset),
            sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
        };
        object.volume.sculpt = Some(sculpt);
        object.extra.sculpt = Some(sculpt);
        let id = object.full_id;
        world.apply(NetEvent::ObjectUpdates {
            handle: world
                .objects
                .get(world.objects.index_of_uuid(&crate::demo::DEMO_AGENT).expect("wearer"))
                .expect("avatar")
                .key
                .region,
            objects: vec![object],
        });
        world.objects.index_of_uuid(&id).expect("mesh object")
    }

    #[test]
    fn attachment_parity_mesh_uuid_priority_and_scale_locks_are_independent() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let hip = rig.names["mHipLeft"];
        let low = skin(&[("mHipLeft", Vec3::new(0.1, 0.12, -0.1))], true, 0.25);
        let high = skin(&[("mHipLeft", Vec3::new(-0.2, 0.15, -0.2))], false, -0.125);
        for reverse in [false, true] {
            let mut overrides = JointOverrides::new(rig);
            let mut meshes = [(Uuid::from_u128(1), &low), (Uuid::from_u128(2), &high)];
            if reverse {
                meshes.reverse();
            }
            for (id, skin) in meshes {
                overrides.add(rig, id, skin);
            }
            let mut pos = rig.local_pos.clone();
            let mut scale = vec![Vec3::splat(1.7); rig.len()];
            overrides.apply(rig, &mut pos, &mut scale);
            assert_eq!(pos[hip], Vec3::new(-0.2, 0.15, -0.2));
            assert_eq!(scale[hip], rig.scale[hip], "the losing mesh still locks scale");
            assert_eq!(overrides.pelvis_fixup(), -0.125);
        }
    }

    #[test]
    fn attachment_parity_threshold_invalid_skins_and_zero_fixups_do_not_override() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let hip = rig.names["mHipLeft"];
        let mut overrides = JointOverrides::new(rig);
        let valid = skin(&[("mHipLeft", rig.local_pos[hip] + Vec3::X * 0.0002)], false, 0.15);
        overrides.add(rig, Uuid::from_u128(1), &valid);
        let tiny = skin(&[("mHipLeft", rig.local_pos[hip] + Vec3::X * 0.00005)], true, 0.0);
        overrides.add(rig, Uuid::from_u128(4), &tiny);
        let nan = skin(&[("mHipLeft", Vec3::splat(f32::NAN))], true, f32::NAN);
        overrides.add(rig, Uuid::from_u128(5), &nan);
        let mut invalid = valid.clone();
        invalid.joint_names.push("mHipRight".into());
        overrides.add(rig, Uuid::from_u128(6), &invalid);
        overrides.add(rig, Uuid::nil(), &valid);
        let mut positions = rig.local_pos.clone();
        let mut scales = vec![Vec3::splat(1.4); rig.len()];
        overrides.apply(rig, &mut positions, &mut scales);
        assert_eq!(positions[hip], rig.local_pos[hip] + Vec3::X * 0.0002);
        assert_eq!(scales[hip], Vec3::splat(1.4));
        assert_eq!(overrides.pelvis_fixup(), 0.15);
    }

    #[test]
    fn attachment_parity_late_assets_detach_and_animated_objects_rebuild_the_right_skeleton() {
        let (mut scene, mut world, wearer) = fixture();
        let owner = crate::demo::DEMO_AGENT;
        let hip = world.avatar_lib.rig.names["mHipLeft"];
        mesh(&mut world, 9400, 10);
        let b = mesh(&mut world, 9401, 20);
        let now = Instant::now();
        let (initial, _) = scene.skeleton_of(&world, owner, wearer, now);
        let first = Vec3::new(0.1, 0.12, -0.1);
        scene
            .meshes
            .insert_demo_skin(Uuid::from_u128(10), skin(&[("mHipLeft", first)], true, 0.0));
        let (loaded, _) = scene.skeleton_of(&world, owner, wearer, now);
        assert!(!Arc::ptr_eq(&initial, &loaded));
        assert_eq!(loaded.local_pos[hip], first);
        let second = Vec3::new(-0.3, 0.2, 0.05);
        scene
            .meshes
            .insert_demo_skin(Uuid::from_u128(20), skin(&[("mHipLeft", second)], false, 0.0));
        let (conflict, _) = scene.skeleton_of(&world, owner, wearer, now);
        assert_eq!(conflict.local_pos[hip], second);
        world.objects.get_mut(b).expect("mesh").extra.extended_mesh_flags = Some(1);
        let (isolated, _) = scene.skeleton_of(&world, owner, wearer, now);
        assert_eq!(isolated.local_pos[hip], first);
        let (control, _) = scene.skeleton_of(&world, Uuid::from_u128(9401), b, now);
        assert_eq!(control.local_pos[hip], second);
        // A simulator removal drops the root; its source mesh stays cached.
        world.apply(NetEvent::ObjectsKilled {
            handle: world.objects.get(wearer).expect("wearer").key.region,
            local_ids: vec![9400],
        });
        let (removed, _) = scene.skeleton_of(&world, owner, wearer, now);
        assert_eq!(removed.local_pos, initial.local_pos);
    }

    #[test]
    fn attachment_parity_pelvis_translation_and_fixup_follow_standing_and_seated_roots() {
        let (mut scene, mut world, wearer) = fixture();
        let owner = crate::demo::DEMO_AGENT;
        let rig = world.avatar_lib.rig.clone();
        mesh(&mut world, 9400, 10);
        let offset = Vec3::new(0.23, -0.17, 0.11);
        scene
            .meshes
            .insert_demo_skin(Uuid::from_u128(10), skin(&[("mPelvis", offset)], true, 0.27));
        let now = Instant::now();
        let (base, dz) = scene.skeleton_of(&world, owner, wearer, now);
        assert_eq!(base.local_pos[rig.pelvis], rig.local_pos[rig.pelvis] + offset);
        let (foot, height) = shape::body_size(&rig, &base.local_pos, &base.scale);
        let hover = world.avatar_lib.shape_params.shape(&[], &rig).hover;
        assert!((dz - (hover + 0.27 - 0.5 * height + foot)).abs() < 1e-6);
        world.objects.get_mut(wearer).expect("avatar").parent_id = crate::demo::worn::ROOT;
        let (seated, dz) = scene.skeleton_of(&world, owner, wearer, now);
        assert_eq!(seated.local_pos[rig.pelvis], base.local_pos[rig.pelvis]);
        assert_eq!(dz, 0.0, "a seated root has no standing/fixup height correction");
    }

    #[test]
    fn attachment_parity_bone_scale_changes_the_point_offset_without_resizing_geometry() {
        let library = AvatarLibrary::load();
        let mut point = library.attach_points[&22];
        let position = Vec3::new(0.3, -0.1, 0.7);
        let rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.3, 0.4);
        let scale = Vec3::new(2.0, 0.4, 1.3);
        point.rest_joint = Mat4::from_scale_rotation_translation(scale, rotation, position);
        point.local_position = Vec3::new(0.11, -0.07, 0.13);
        point.rotation = Quat::from_rotation_y(0.8);
        let (actual_position, actual_rotation) = point.frame(None);
        assert!(actual_position.abs_diff_eq(position + rotation * (scale * point.local_position), 1e-6));
        let actual = Mat4::from_rotation_translation(actual_rotation, actual_position);
        let expected = Mat4::from_rotation_translation(rotation * point.rotation, position + rotation * (scale * point.local_position));
        assert!(actual.abs_diff_eq(expected, 1e-6));
        assert!((actual.x_axis.truncate().length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn attachment_parity_only_ground_animesh_gets_its_own_world_vertical_pelvis_fixup() {
        let (mut scene, mut world, _) = fixture();
        let index = mesh(&mut world, 9400, 10);
        let rig = world.avatar_lib.rig.clone();
        scene.meshes.insert_demo_skin(
            Uuid::from_u128(10),
            skin(&[("mHipLeft", rig.local_pos[rig.names["mHipLeft"]])], false, 0.3),
        );
        world.objects.get_mut(index).expect("animesh").extra.extended_mesh_flags = Some(1);
        world.avatar_shape_dz.insert(Uuid::from_u128(9400), 0.3);
        let position = Vec3::new(1.0, 2.0, 3.0);
        let rotation = Quat::from_rotation_x(0.6);
        let worn = anchors::skeleton_model(&world, &scene.meshes, index, position, rotation, 1.0);
        let expected = Mat4::from_rotation_translation(rotation, position) * Mat4::from_translation(-world.avatar_lib.pelvis);
        assert!(worn.abs_diff_eq(expected, 1e-6));
        world.objects.get_mut(index).expect("animesh").parent_id = 0;
        let ground = anchors::skeleton_model(&world, &scene.meshes, index, position, rotation, 1.0);
        let expected = Mat4::from_translation(Vec3::Z * 0.3) * expected;
        assert!(ground.abs_diff_eq(expected, 1e-6));
    }

    // Independent LLXformMatrix oracle: compose positions with the parent's
    // own scale and rotation; never inherit that scale in the child's matrix.
    fn legacy_joints(rig: &Rig, base: &anim::SkeletonBase, rotations: &[Quat]) -> Vec<(Vec3, Quat)> {
        let mut joints = vec![(Vec3::ZERO, Quat::IDENTITY); rig.len()];
        for &joint in &rig.order {
            let position = base.local_pos[joint];
            joints[joint] = match rig.parent[joint] {
                Some(parent) => (
                    joints[parent].0 + joints[parent].1 * (base.scale[parent] * position),
                    (joints[parent].1 * rotations[joint]).normalize(),
                ),
                None => (position, rotations[joint]),
            };
        }
        joints
    }

    #[test]
    fn attachment_parity_all_55_points_keep_linkset_position_rotation_and_size() {
        let (mut scene, mut world, wearer) = fixture();
        let lib = world.avatar_lib.clone();
        let rig = &lib.rig;
        let now = Instant::now();
        let (base, _) = scene.skeleton_of(&world, crate::demo::DEMO_AGENT, wearer, now);
        let mut base = (*base).clone();
        let mut rotations = rig.local_rot.clone();
        base.local_pos[rig.pelvis] += Vec3::new(0.12, -0.08, 0.19);
        for (joint, rotation) in rotations.iter_mut().enumerate() {
            *rotation = (*rotation * Quat::from_euler(glam::EulerRot::XYZ, 0.05, -0.07, joint as f32 * 0.001)).normalize();
            base.scale[joint] *= Vec3::new(1.3, 0.8, 1.15);
        }
        let joints = legacy_joints(rig, &base, &rotations);
        let absolute = rig.world_with(&rotations, Vec3::ZERO, &base.local_pos, &base.scale);
        world.avatar_poses.insert(
            crate::demo::DEMO_AGENT,
            absolute.iter().zip(&rig.default_world_inv).map(|(a, b)| *a * *b).collect(),
        );
        let root = world.objects.index_of_uuid(&crate::demo::worn::item_id()).expect("root");
        let child = world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == crate::demo::worn::CHILD)
            .map(|(i, _)| i)
            .expect("child");
        world.objects.get_mut(root).expect("root").scale = Vec3::new(2.0, 0.3, 4.0);
        world.objects.get_mut(child).expect("child").scale = Vec3::new(0.15, 0.7, 0.23);
        let (avatar_pos, avatar_rot, _) = Scene::object_transform(&world, wearer, now, 0).expect("avatar");
        for (&point, ap) in &lib.attach_points {
            world.objects.get_mut(root).expect("root").state = point.rotate_left(4);
            let (point_pos, point_rot) = if ap.hud {
                let mut position = ap.position;
                position.y *= world.hud_aspect;
                (position, ap.rotation)
            } else if let Some(joint) = ap.joint {
                (
                    avatar_pos + avatar_rot * (joints[joint].0 + joints[joint].1 * (base.scale[joint] * ap.local_position) - lib.pelvis),
                    avatar_rot * joints[joint].1 * ap.rotation,
                )
            } else {
                // Avatar Center is on mRoot, outside the animated skeleton.
                (avatar_pos + avatar_rot * ap.position, avatar_rot * ap.rotation)
            };
            let root_object = world.objects.get(root).expect("root");
            let child_object = world.objects.get(child).expect("child");
            let root_pos = point_pos + point_rot * root_object.position;
            let root_rot = point_rot * root_object.rotation;
            for (index, position, rotation) in [
                (root, root_pos, root_rot),
                (child, root_pos + root_rot * child_object.position, root_rot * child_object.rotation),
            ] {
                let object = world.objects.get(index).expect("prim");
                let expected = Mat4::from_scale_rotation_translation(object.scale, rotation, position);
                let (p, r, hud) = Scene::object_transform(&world, index, now, 0).expect("placed");
                let actual = Mat4::from_scale_rotation_translation(object.scale, r, p);
                assert_eq!(hud, ap.hud);
                assert!(
                    actual.abs_diff_eq(expected, 0.0001),
                    "point {point}, prim {index}: {actual:?} != {expected:?}"
                );
                if !hud {
                    let (_, local_pos, local_rot) = anchors::anchored(&world, object, |_, slot| Some(u32::from(slot))).expect("GPU anchor");
                    let f = anchors::attach_frame(&world, world.objects.get(wearer).expect("wearer"), point);
                    let gpu = Mat4::from_rotation_translation(avatar_rot * f.rot, avatar_pos + avatar_rot * f.pos)
                        * Mat4::from_scale_rotation_translation(object.scale, local_rot, local_pos);
                    assert!(gpu.abs_diff_eq(expected, 0.0001), "GPU point {point}, prim {index}");
                }
            }
        }
    }
}
