//! Local circular podium and an ocean plane beneath the inspection avatar.
//! Private GPU allocations, never simulator objects or inventory assets.

use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, ParticleInstance, Renderer, ShadowCaster, Vertex, flags, particle_flags};
use glam::{Mat4, Quat, Vec3};

#[derive(Default)]
pub struct Stage {
    parts: Vec<(MeshAlloc, u32)>,
}

pub const WATER_OFFSET: f32 = -0.18;
const PODIUM_RADIUS: f32 = 1.15;
const PODIUM_DEPTH: f32 = 0.35;
const PODIUM_BEVEL: f32 = 0.055;

/// T-pose follows the shaped rest skeleton, as reset-joint posing does
/// (FSPoserAnimator::resetJoint, indra/newview/fsposeranimator.cpp).
/// Aurora's A-pose lowers each straight arm 45 degrees. Apply only to the
/// final displayed palette, never to animation signals or held blend state.
pub(super) fn inspection_pose(
    rig: &super::anim::Rig,
    base: &super::anim::SkeletonBase,
    pose: crate::avatar_studio::Pose,
) -> Option<Vec<Mat4>> {
    use crate::avatar_studio::Pose;
    if pose == Pose::Animated {
        return None;
    }
    let mut rotations = rig.local_rot.clone();
    if pose == Pose::A {
        for (name, angle) in [
            ("mShoulderLeft", -std::f32::consts::FRAC_PI_4),
            ("mShoulderRight", std::f32::consts::FRAC_PI_4),
        ] {
            if let Some(&joint) = rig.names.get(name) {
                rotations[joint] *= Quat::from_rotation_x(angle);
            }
        }
    }
    Some(rig.world_with(&rotations, Vec3::ZERO, &base.local_pos, &base.scale))
}

/// Applied on the existing pose worker, after ordinary animation and physics.
pub(super) fn write_inspection_pose(
    rig: &super::anim::Rig,
    base: &super::anim::SkeletonBase,
    pose: crate::avatar_studio::Pose,
    ground: Option<(Mat4, f32)>,
    out: &mut [[[f32; 4]; 4]],
) {
    if let Some(mut pose) = inspection_pose(rig, base, pose) {
        if let Some((model, floor)) = ground {
            ground_inspection_pose(rig, &mut pose, model, floor);
        }
        for (dst, matrix) in out.iter_mut().zip(pose) {
            *dst = matrix.to_cols_array_2d();
        }
    }
}

/// The Studio's hover height as a translation in the avatar's skeleton
/// space, or `None` when there is nothing to lift. Whoever reads the stored
/// pose back as an authored one (the pose and animation editors' `Frame`)
/// takes the same translation out again.
pub fn display_lift(model: Mat4, height: f32) -> Option<Vec3> {
    if !model.is_finite() || model.determinant().abs() <= 1e-8 || !height.is_finite() || height.abs() < f32::EPSILON {
        return None;
    }
    let local_lift = model.inverse().transform_vector3(Vec3::Z * height);
    local_lift.is_finite().then_some(local_lift)
}

/// Lift the Studio's displayed skeleton and worn attachment anchors without
/// changing its object transform or any animation data.
pub(super) fn lift_display_pose(model: Option<Mat4>, height: f32, out: &mut [[[f32; 4]; 4]]) {
    let Some(local_lift) = model.and_then(|model| display_lift(model, height)) else {
        return;
    };
    for matrix in out.iter_mut().take(super::anim::ANCHOR_BASE) {
        let mut transform = Mat4::from_cols_array_2d(matrix);
        transform.w_axis += local_lift.extend(0.0);
        *matrix = transform.to_cols_array_2d();
    }
}

#[cfg(test)]
mod display_lift_tests {
    use super::*;

    #[test]
    fn studio_hover_lifts_joint_and_attachment_palettes_without_touching_other_slots() {
        let model = Mat4::from_rotation_y(0.7) * Mat4::from_translation(Vec3::new(2.0, -1.0, 4.0));
        let mut palette = vec![Mat4::IDENTITY.to_cols_array_2d(); super::super::anim::ANCHOR_BASE + 1];
        let before = palette.clone();

        lift_display_pose(Some(model), 0.4, &mut palette);

        for index in [0, super::super::anim::ANCHOR_BASE - 1] {
            let lifted = Mat4::from_cols_array_2d(&palette[index]);
            let original = Mat4::from_cols_array_2d(&before[index]);
            let world_delta = model.transform_vector3(lifted.w_axis.truncate() - original.w_axis.truncate());
            assert!(world_delta.abs_diff_eq(Vec3::Z * 0.4, 1e-5));
            assert_eq!(lifted.x_axis, original.x_axis);
            assert_eq!(lifted.y_axis, original.y_axis);
            assert_eq!(lifted.z_axis, original.z_axis);
        }
        assert_eq!(palette[super::super::anim::ANCHOR_BASE], before[super::super::anim::ANCHOR_BASE]);
    }
}

/// Normalize only the displayed preset, including attachment joints. A mesh
/// pelvis offset or the AO's entry pose must not lift it above the fixed stage.
fn ground_inspection_pose(rig: &super::anim::Rig, pose: &mut [Mat4], model: Mat4, floor: f32) {
    let feet = ["mFootLeft", "mFootRight"]
        .into_iter()
        .filter_map(|name| rig.names.get(name).and_then(|&j| pose.get(j)))
        .map(|joint| model.transform_point3(joint.w_axis.truncate()).z)
        .reduce(f32::min);
    if let Some(feet) = feet.filter(|z| z.is_finite())
        && floor.is_finite()
        && model.is_finite()
        && model.determinant().abs() > 1e-8
    {
        let shift = model
            .inverse()
            .transform_vector3(Vec3::Z * (floor + crate::avatar_studio::SOLE_HEIGHT - feet));
        if shift.is_finite() {
            for joint in pose {
                joint.w_axis += shift.extend(0.0);
            }
        }
    }
}

/// The water passes use separate terrain lists, including chunks outside
/// the main camera's frustum. Hide them with the main-view terrain so the
/// local ocean cannot reflect or refract the region's invisible land.
pub fn hide_region_terrain(lists: &mut DrawLists) {
    lists.terrain.clear();
    lists.terrain_pbr.clear();
    lists.terrain_mask.clear();
    lists.terrain_shadow_casters.clear();
    lists.reflection_terrain.clear();
    lists.reflection_terrain_pbr.clear();
}

/// One bounded burst, evaluated analytically from its age. Reuse the normal
/// GPU billboard path and the draw list's allocation; no emitters, assets,
/// world particles or simulation state are created or changed.
pub fn reveal_particles(out: &mut Vec<ParticleInstance>, floor: Vec3, height: f32, age: f32, colors: [[u8; 3]; 3], limit: usize) {
    if !age.is_finite() || !(0.0..1.2).contains(&age) {
        return;
    }
    let random = |i: u32, channel: u32| {
        let mut v = i.wrapping_mul(747796405).wrapping_add(channel.wrapping_mul(2891336453));
        v = ((v >> ((v >> 28) + 4)) ^ v).wrapping_mul(277803737);
        ((v ^ (v >> 22)) & 65535) as f32 / 65535.0
    };
    for i in 0..limit.min(96) as u32 {
        let life = 0.7 + random(i, 1) * 0.5;
        let fade = (1.0 - age / life).max(0.0);
        if fade <= 0.0 {
            continue;
        }
        let angle = random(i, 2) * std::f32::consts::TAU;
        let radius = 0.25 + random(i, 3) * 0.2 + age * (0.3 + random(i, 4) * 0.6);
        let position = floor + Vec3::new(angle.cos() * radius, angle.sin() * radius, 0.1 + height * random(i, 5) + age * 0.35);
        let color = colors[i as usize % colors.len()];
        let size = 0.025 + random(i, 6) * 0.035 + age * 0.025;
        out.push(ParticleInstance {
            pos: position.to_array(),
            size: [size; 2],
            flags: particle_flags::EMISSIVE | particle_flags::ADDITIVE,
            color: [color[0], color[1], color[2], (fade * fade * 180.0).round() as u8],
            glow: fade * 0.4,
            ..Default::default()
        });
    }
}

fn podium() -> (Vec<Vertex>, Vec<u16>) {
    const SEGMENTS: u16 = 96;
    const STRIDE: u16 = SEGMENTS + 1;
    let mut vertices = Vec::with_capacity(10 * usize::from(STRIDE) + 2);
    let mut indices = Vec::with_capacity(usize::from(SEGMENTS) * 48);
    // Quarter-circle shoulders carry real curved normals: the existing PBR
    // lights catch the edges without a painted highlight or a new texture.
    for ring in 0..8u16 {
        let angle = if ring < 4 { f32::from(ring) } else { f32::from(ring - 1) } * std::f32::consts::FRAC_PI_6;
        let radius = PODIUM_RADIUS - PODIUM_BEVEL + PODIUM_BEVEL * angle.sin();
        let z = if ring < 4 { -PODIUM_BEVEL } else { -PODIUM_DEPTH + PODIUM_BEVEL } + PODIUM_BEVEL * angle.cos();
        for i in 0..=SEGMENTS {
            let u = f32::from(i) / f32::from(SEGMENTS);
            let theta = u * std::f32::consts::TAU;
            let radial = Vec3::new(theta.cos(), theta.sin(), 0.0);
            vertices.push(Vertex::new(
                (radial * radius + Vec3::Z * z).to_array(),
                (radial * angle.sin() + Vec3::Z * angle.cos()).to_array(),
                [u, -z / PODIUM_DEPTH],
            ));
            if ring < 7 && i < SEGMENTS {
                let a = ring * STRIDE + i;
                let b = a + STRIDE;
                indices.extend([a, b, b + 1, a, b + 1, a + 1]);
            }
        }
    }
    // Separate planar cap UVs and a closed underside, including when the
    // pedestal is seen by the water's virtual camera or refraction pass.
    for bottom in [false, true] {
        let center = vertices.len() as u16;
        let z = if bottom { -PODIUM_DEPTH } else { 0.0 };
        let normal = if bottom { -Vec3::Z } else { Vec3::Z };
        vertices.push(Vertex::new([0.0, 0.0, z], normal.to_array(), [0.5; 2]));
        for i in 0..=SEGMENTS {
            let theta = f32::from(i) / f32::from(SEGMENTS) * std::f32::consts::TAU;
            let radial = Vec3::new(theta.cos(), theta.sin(), 0.0);
            vertices.push(Vertex::new(
                (radial * (PODIUM_RADIUS - PODIUM_BEVEL) + Vec3::Z * z).to_array(),
                normal.to_array(),
                [radial.x * 0.5 + 0.5, radial.y * 0.5 + 0.5],
            ));
            if i < SEGMENTS {
                let a = center + 1 + i;
                indices.extend(if bottom { [center, a + 1, a] } else { [center, a, a + 1] });
            }
        }
    }
    (vertices, indices)
}

fn append_podium(lists: &mut DrawLists, cmd: DrawCmd, floor: Vec3) {
    let center = floor - Vec3::Z * (PODIUM_DEPTH * 0.5);
    let radius = PODIUM_RADIUS.hypot(PODIUM_DEPTH * 0.5);
    let cmd = cmd.with_bounds(center, radius);
    lists.opaque.push(cmd);
    // CPU supplemental draws are consumed alongside the GPU avatar draws in
    // both water captures. Omitting the solid here exposes reflected soles.
    lists.reflection.push(ShadowCaster {
        cmd,
        center,
        radius,
        alpha: false,
    });
}

fn ocean() -> (Vec<Vertex>, Vec<u16>) {
    let vertices = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| Vertex::new([x, y, 0.0], Vec3::Z.to_array(), [x, y]));
    (vertices.to_vec(), vec![0, 1, 2, 0, 2, 3])
}

impl Stage {
    pub fn append(&mut self, renderer: &mut Renderer, lists: &mut DrawLists, floor: Vec3, yaw: f32) {
        if self.parts.is_empty() {
            for (vertices, indices) in [podium(), ocean()] {
                let Some(mesh) = renderer.upload_mesh(&vertices, &indices) else {
                    self.clear(renderer);
                    log::warn!("avatar studio: stage allocation unavailable");
                    return;
                };
                self.parts.push((mesh, renderer.records.alloc(DrawRecord::default())));
            }
        }
        let rotation = Quat::from_rotation_z(yaw);
        for (i, (mesh, record)) in self.parts.iter().enumerate() {
            let (scale, center) = if i == 0 {
                (Vec3::ONE, floor)
            } else {
                (Vec3::new(16384.0, 16384.0, 1.0), floor + Vec3::Z * WATER_OFFSET)
            };
            renderer.records.set(
                *record,
                DrawRecord {
                    model: Mat4::from_scale_rotation_translation(scale, rotation, center).to_cols_array_2d(),
                    base_color: [0.22, 0.24, 0.28, 1.0],
                    params: [0.0, 0.65, 0.38, 0.5],
                    flags: [if i == 0 { flags::PBR } else { 0 }, 0, 0, flags::STUDIO_STAGE],
                    ..Default::default()
                },
            );
            let cmd = DrawCmd {
                index_count: mesh.index_count,
                first_index: mesh.index_offset,
                base_vertex: mesh.vertex_offset as i32,
                record: *record,
                bounds: [0.0; 4],
            };
            if i == 0 {
                append_podium(lists, cmd, floor);
            } else {
                lists.water.push(cmd);
            }
        }
    }

    pub fn clear(&mut self, renderer: &mut Renderer) {
        for (mesh, record) in self.parts.drain(..) {
            renderer.free_mesh(mesh);
            renderer.records.free(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_pelvis_offsets_and_entry_height_do_not_float_inspection_poses() {
        use crate::avatar_studio::{Pose, SOLE_HEIGHT};
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = &lib.rig;
        for pelvis_offset in [-1.5, 0.0, 2.3] {
            let mut base = super::super::anim::SkeletonBase {
                local_pos: rig.local_pos.clone(),
                scale: rig.scale.clone(),
            };
            base.local_pos[rig.pelvis].z += pelvis_offset;
            let model = Mat4::from_rotation_translation(Quat::from_euler(glam::EulerRot::XYZ, 0.1, -0.2, 0.7), Vec3::new(12.0, 20.0, 30.0))
                * Mat4::from_translation(-lib.pelvis);
            let floor = 28.4;
            for selected in [Pose::T, Pose::A] {
                let mut pose = inspection_pose(rig, &base, selected).expect("preset");
                let before = pose.clone();
                ground_inspection_pose(rig, &mut pose, model, floor);
                let feet = ["mFootLeft", "mFootRight"].map(|name| model.transform_point3(pose[rig.names[name]].w_axis.truncate()).z);
                assert!((feet[0].min(feet[1]) - SOLE_HEIGHT - floor).abs() < 1e-5);
                let shift = pose[rig.pelvis].w_axis - before[rig.pelvis].w_axis;
                for (original, grounded) in before.iter().zip(&pose) {
                    assert!((grounded.w_axis - original.w_axis).abs_diff_eq(shift, 1e-5));
                    assert_eq!(grounded.x_axis, original.x_axis);
                    assert_eq!(grounded.y_axis, original.y_axis);
                    assert_eq!(grounded.z_axis, original.z_axis);
                }
            }
        }
    }

    #[test]
    fn inspection_poses_keep_shaped_feet_and_lower_both_arms_symmetrically() {
        use crate::avatar_studio::Pose;
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = &lib.rig;
        let mut base = super::super::anim::SkeletonBase {
            local_pos: rig.local_pos.clone(),
            scale: rig.scale.clone(),
        };
        base.local_pos[rig.names["mShoulderLeft"]].y += 0.03;
        base.local_pos[rig.names["mShoulderRight"]].y -= 0.03;
        let t = inspection_pose(rig, &base, Pose::T).expect("T-pose");
        let a = inspection_pose(rig, &base, Pose::A).expect("A-pose");
        assert!(inspection_pose(rig, &base, Pose::Animated).is_none());
        for side in ["Left", "Right"] {
            let shoulder = rig.names[&format!("mShoulder{side}")];
            let wrist = rig.names[&format!("mWrist{side}")];
            let t_arm = t[wrist].w_axis.truncate() - t[shoulder].w_axis.truncate();
            let a_arm = a[wrist].w_axis.truncate() - a[shoulder].w_axis.truncate();
            assert!(a_arm.z < t_arm.z - 0.2, "{side} arm points down");
            assert!((t_arm.length() - a_arm.length()).abs() < 1e-5);
            let foot = rig.names[&format!("mFoot{side}")];
            assert!(a[foot].abs_diff_eq(t[foot], 1e-6));
        }
    }

    fn first_hit(vertices: &[Vertex], indices: &[u16], origin: Vec3, direction: Vec3) -> Option<f32> {
        indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|triangle| {
                let [a, b, c] = triangle.map(|i| Vec3::from_array(vertices[usize::from(i)].pos));
                let edge1 = b - a;
                let edge2 = c - a;
                let p = direction.cross(edge2);
                let determinant = edge1.dot(p);
                if determinant.abs() < 0.000001 {
                    return None;
                }
                let delta = origin - a;
                let u = delta.dot(p) / determinant;
                let q = delta.cross(edge1);
                let v = direction.dot(q) / determinant;
                let distance = edge2.dot(q) / determinant;
                (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && distance > 0.0).then_some(distance)
            })
            .reduce(f32::min)
    }

    #[test]
    fn solid_podium_blocks_water_rays_to_the_soles_and_is_in_reflection_draws() {
        let (vertices, indices) = podium();
        let below = Vec3::new(0.1, 0.0, -1.0);
        let hit = first_hit(&vertices, &indices, below, Vec3::Z).expect("closed bottom cap");
        assert!((below.z + hit + PODIUM_DEPTH).abs() < 0.00001);
        let sole = Vec3::new(0.0, 0.0, 0.03);
        for x in [1.25, 1.6, 2.0, 3.0] {
            let surface = Vec3::new(x, 0.0, WATER_OFFSET);
            let ray = sole - surface;
            let hit = first_hit(&vertices, &indices, surface, ray.normalize()).expect("pedestal occludes sole");
            assert!(hit < ray.length());
        }
        let mut lists = DrawLists::default();
        let cmd = DrawCmd {
            index_count: indices.len() as u32,
            first_index: 0,
            base_vertex: 0,
            record: 17,
            bounds: [0.0; 4],
        };
        let floor = Vec3::new(10.0, 20.0, 30.0);
        append_podium(&mut lists, cmd, floor);
        assert_eq!(lists.opaque[0].record, cmd.record);
        let reflected = &lists.reflection[0];
        assert_eq!(reflected.cmd.record, cmd.record);
        assert!(!reflected.alpha);
        for vertex in &vertices {
            assert!((floor + Vec3::from_array(vertex.pos)).distance(reflected.center) <= reflected.radius + 0.00001);
        }
    }

    #[test]
    fn reveal_burst_is_bounded_fades_and_ends_without_retained_particles() {
        let colors = [[120, 220, 220]; 3];
        let mut start = Vec::new();
        reveal_particles(&mut start, Vec3::ZERO, 1.8, 0.0, colors, 10000);
        assert_eq!(start.len(), 96);
        let mut later = Vec::new();
        reveal_particles(&mut later, Vec3::ZERO, 1.8, 0.5, colors, 96);
        assert_eq!(later.len(), start.len());
        for (a, b) in start.iter().zip(&later) {
            assert!(Vec3::from_array(b.pos).is_finite());
            assert!(b.color[3] < a.color[3]);
            assert!(b.pos[2] > a.pos[2]);
        }
        let mut done = Vec::new();
        for age in [1.2, 10.0, -1.0, f32::NAN] {
            reveal_particles(&mut done, Vec3::ZERO, 1.8, age, colors, 96);
        }
        reveal_particles(&mut done, Vec3::ZERO, 1.8, 0.0, colors, 0);
        assert!(done.is_empty());
    }

    #[test]
    fn studio_hides_classic_and_pbr_land_from_water_passes_too() {
        let land = DrawCmd {
            index_count: 6,
            first_index: 0,
            base_vertex: 0,
            record: 17,
            bounds: [0.0; 4],
        };
        let avatar = DrawCmd { record: 23, ..land };
        let reflected = aurora_render::ShadowCaster {
            cmd: avatar,
            center: Vec3::ZERO,
            radius: 1.0,
            alpha: false,
        };
        let mut lists = DrawLists {
            terrain: vec![land],
            terrain_pbr: vec![land],
            terrain_mask: vec![land],
            terrain_shadow_casters: vec![aurora_render::ShadowCaster { cmd: land, ..reflected }],
            // These also contain land outside the main view and are shared
            // by the water reflection and air-side refraction passes.
            reflection_terrain: vec![land],
            reflection_terrain_pbr: vec![land],
            opaque: vec![avatar],
            reflection: vec![reflected],
            hud_opaque: vec![avatar],
            ..Default::default()
        };
        hide_region_terrain(&mut lists);
        assert!(lists.terrain.is_empty());
        assert!(lists.terrain_pbr.is_empty());
        assert!(lists.terrain_mask.is_empty());
        assert!(lists.terrain_shadow_casters.is_empty());
        assert!(lists.reflection_terrain.is_empty());
        assert!(lists.reflection_terrain_pbr.is_empty());
        assert_eq!(lists.opaque[0].record, avatar.record);
        assert_eq!(lists.reflection[0].cmd.record, avatar.record);
        assert_eq!(lists.hud_opaque[0].record, avatar.record);
    }

    #[test]
    fn stage_faces_outward_and_water_stays_below_the_feet() {
        for (vertices, indices) in [podium(), ocean()] {
            for triangle in indices.as_chunks::<3>().0 {
                let a = Vec3::from_array(vertices[triangle[0] as usize].pos);
                let b = Vec3::from_array(vertices[triangle[1] as usize].pos);
                let c = Vec3::from_array(vertices[triangle[2] as usize].pos);
                let packed = vertices[triangle[0] as usize].normal;
                let normal = Vec3::new(f32::from(packed[0]), f32::from(packed[1]), f32::from(packed[2]));
                assert!((b - a).cross(c - a).dot(normal) > 0.0);
            }
        }
        let (vertices, _) = podium();
        let top = vertices.iter().map(|v| v.pos[2]).reduce(f32::max).expect("podium vertices");
        let bottom = vertices.iter().map(|v| v.pos[2]).reduce(f32::min).expect("podium vertices");
        assert!(bottom < WATER_OFFSET && WATER_OFFSET < top);
    }
}
