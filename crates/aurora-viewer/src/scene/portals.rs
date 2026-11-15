//! Shared GPU aperture with procedural Aurora flow, depth-tested and sorted
//! with world transparency/glow. No texture generation or particle uploads.
use crate::{
    theme::Palette,
    world::{
        World,
        portals::{HEIGHT, WIDTH},
    },
};
use aurora_render::{DrawCmd, DrawRecord, MeshAlloc, PortalClip, Renderer, Vertex, types::flags};
use glam::{Mat4, Quat, Vec3};
use std::collections::{HashMap, HashSet};
mod clipping;

type Identity = (uuid::Uuid, uuid::Uuid, u64);

fn opening_birth(births: &mut HashMap<Identity, u32>, key: Identity, now: u32, animate: bool) -> u32 {
    let birth = births.entry(key).or_insert(now);
    // Settling once also prevents a partial opening from coming back when
    // reduced motion is switched off. Do not rewrite settled clocks each frame.
    if !animate && now.wrapping_sub(*birth) < 8000 {
        *birth = now.wrapping_sub(8000);
    }
    *birth
}

// One shared upload: aperture, energy rim and a closed storm-cloud shell.
// UV.x in [2, 3] selects the rim; [4, 5] selects the cloud in the shader.
fn portal_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let scale = aurora_render::PORTAL_SURFACE_SCALE;
    let mut vertices = vec![
        Vertex::new([-WIDTH * scale, 0.0, -HEIGHT * scale], [0.0, -1.0, 0.0], [0.0, 1.0]),
        Vertex::new([WIDTH * scale, 0.0, -HEIGHT * scale], [0.0, -1.0, 0.0], [1.0, 1.0]),
        Vertex::new([WIDTH * scale, 0.0, HEIGHT * scale], [0.0, -1.0, 0.0], [1.0, 0.0]),
        Vertex::new([-WIDTH * scale, 0.0, HEIGHT * scale], [0.0, -1.0, 0.0], [0.0, 0.0]),
    ];
    // Portal records are double-sided. A second coincident quad would
    // blend the thin opening twice, including its glow, from either side.
    let mut indices = vec![0, 1, 2, 0, 2, 3];
    // The shader supplies the fine filaments. The cloud only needs a
    // convex proxy enclosing the analytic density field, including at its poles.
    const AROUND: usize = 64;
    const TUBE: usize = 8;
    for i in 0..=AROUND {
        let u = i as f32 / AROUND as f32;
        let angle = u * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        let center = Vec3::new(WIDTH * cos, 0.0, HEIGHT * sin);
        let radial = Vec3::new(cos / WIDTH, 0.0, sin / HEIGHT).normalize();
        for j in 0..=TUBE {
            let v = j as f32 / TUBE as f32;
            let (sin, cos) = (v * std::f32::consts::TAU).sin_cos();
            let normal = radial * cos + Vec3::Y * sin;
            vertices.push(Vertex::new((center + normal * 0.075).to_array(), normal.to_array(), [2.0 + u, v]));
        }
    }
    for i in 0..AROUND {
        for j in 0..TUBE {
            let a = (4 + i * (TUBE + 1) + j) as u16;
            let b = a + (TUBE + 1) as u16;
            indices.extend_from_slice(&[a, b + 1, b, a, a + 1, b + 1]);
        }
    }
    let first = vertices.len();
    const LATITUDE: usize = 12;
    for i in 0..=AROUND {
        let u = i as f32 / AROUND as f32;
        let (sin, cos) = (u * std::f32::consts::TAU).sin_cos();
        for j in 0..=LATITUDE {
            let v = j as f32 / LATITUDE as f32;
            let (ring, depth) = (v * std::f32::consts::PI).sin_cos();
            let unit = Vec3::new(cos * ring, depth, sin * ring);
            let size = Vec3::new(1.5, 0.6, 2.0);
            vertices.push(Vertex::new(
                (unit * size).to_array(),
                (unit / size).normalize().to_array(),
                [4.0 + u, v],
            ));
        }
    }
    for i in 0..AROUND {
        for j in 0..LATITUDE {
            let a = (first + i * (LATITUDE + 1) + j) as u16;
            let b = a + (LATITUDE + 1) as u16;
            indices.extend_from_slice(&[a, b, b + 1, a, b + 1, a + 1]);
        }
    }
    (vertices, indices)
}

fn object_plane(gpu: &super::ObjGpu, index: usize, planes: &HashMap<usize, PortalClip>) -> Option<PortalClip> {
    if gpu.hud {
        return None;
    }
    gpu.avatar(index).and_then(|owner| planes.get(&owner)).copied()
}

fn blocks_avatar(center: Vec3, normal: Vec3, eye: Vec3, avatar: Vec3) -> bool {
    let from = eye - center;
    let ray = avatar - eye;
    let along = ray.dot(normal);
    if along.abs() < 0.0001 {
        return false;
    }
    let t = -from.dot(normal) / along;
    if !(0.0..=1.0).contains(&t) {
        return false;
    }
    let at = from + ray * t;
    let tangent = Vec3::new(-normal.y, normal.x, 0.0);
    (at.dot(tangent) / WIDTH).powi(2) + (at.z / HEIGHT).powi(2) <= 1.0
}

#[derive(Default)]
pub struct Portals {
    pub planes: HashMap<usize, PortalClip>,
    pub(super) clipped: HashSet<usize>,
    clipping: clipping::Clipping,
    mesh: Option<MeshAlloc>,
    // First local sighting, not announcement refresh or record order. The
    // GPU clock drives the opening once without per-frame record writes.
    births: HashMap<Identity, u32>,
    records: Vec<u32>,
    pub cmds: Vec<(Vec3, DrawCmd)>,
}

impl Portals {
    pub fn arrived(&mut self, avatar: uuid::Uuid) {
        self.clipping.forget(avatar);
    }

    pub fn clear(&mut self, renderer: &mut Renderer) {
        self.planes.clear();
        self.clipped.clear();
        self.clipping.clear();
        self.births.clear();
        for record in self.records.drain(..) {
            renderer.records.free(record);
        }
        if let Some(mesh) = self.mesh.take() {
            renderer.free_mesh(mesh);
        }
        self.cmds.clear();
    }

    pub fn update(&mut self, renderer: &mut Renderer, world: &World, eye: Vec3, palette: &Palette, follow_avatar: bool, animate: bool) {
        self.cmds.clear();
        self.births.retain(|&(owner, id, region), _| {
            world
                .portals
                .entries
                .iter()
                .any(|entry| entry.owner == owner && entry.portal.id == id && entry.portal.region == region)
        });
        if (world.portals.visible(world).next().is_some() || !world.portals.arrivals.is_empty()) && self.mesh.is_none() {
            let (vertices, indices) = portal_mesh();
            self.mesh = renderer.upload_mesh(&vertices, &indices);
        }
        let Some(mesh) = self.mesh else {
            return;
        };
        let mut used = 0;
        for entry in world.portals.visible(world) {
            let portal = &entry.portal;
            let center = Vec3::from(portal.center);
            // Keep the source mist/rim when it lies between the follow
            // camera and its avatar, without hiding their body and path.
            let normal = Vec3::new(portal.yaw.cos(), portal.yaw.sin(), 0.0);
            let core = !(follow_avatar && blocks_avatar(center, normal, eye, world.agent.position));
            let allowed = entry.allowed(world);
            let birth = opening_birth(
                &mut self.births,
                (entry.owner, portal.id, portal.region),
                renderer.anim_clock().ms(std::time::Instant::now()),
                animate,
            );
            let rgba = |color: egui::Color32| color.to_srgba_unmultiplied().map(|value| value as f32 / 255.0);
            let accent = |color| rgba(if allowed { color } else { palette.muted });
            let record = DrawRecord {
                model: Mat4::from_rotation_translation(Quat::from_rotation_z(portal.yaw - std::f32::consts::FRAC_PI_2), center)
                    .to_cols_array_2d(),
                base_color: accent(palette.violet),
                emissive: accent(palette.teal),
                spec_color: accent(palette.indigo_light),
                legacy: rgba(palette.navy),
                params: [
                    0.0,
                    if animate { 1.0 } else { 0.0 },
                    if allowed { 1.0 } else { 0.35 },
                    if core { 1.0 } else { 0.0 },
                ],
                flags: [
                    flags::FULLBRIGHT | flags::ALPHA_BLEND | flags::DOUBLE_SIDED | flags::PORTAL_EFFECT,
                    0,
                    (portal.id.as_u128() as u16) as u32,
                    birth,
                ],
                // the OIT tie-break: a namespace of their own and the portal's
                // place (not its rank among the visible ones, which changes
                // as portals come and go)
                identity: [
                    aurora_render::oit_tie(
                        *b"aurora-portal-fx",
                        portal.center[0].to_bits()
                            ^ portal.center[1].to_bits().rotate_left(11)
                            ^ portal.center[2].to_bits().rotate_left(22),
                    ),
                    0,
                    0,
                    0,
                ],
                ..Default::default()
            };
            let slot = if let Some(&slot) = self.records.get(used) {
                renderer.records.set_if_changed(slot, record);
                slot
            } else {
                let slot = renderer.records.alloc(record);
                self.records.push(slot);
                slot
            };
            used += 1;
            self.cmds.push((
                center,
                DrawCmd {
                    index_count: mesh.index_count,
                    first_index: mesh.index_offset,
                    base_vertex: mesh.vertex_offset as i32,
                    record: slot,
                    bounds: [0.0; 4],
                },
            ));
        }
        let now = std::time::Instant::now();
        if !world.photo_isolation.applied() {
            for effect in world.portals.arrivals.values() {
                let scale = if animate { effect.scale(now) } else { 1.0 };
                if effect.finished(now) || scale <= 0.001 {
                    continue;
                }
                let center = effect.center();
                let rgba = |color: egui::Color32| color.to_srgba_unmultiplied().map(|v| v as f32 / 255.0);
                let record = DrawRecord {
                    model: Mat4::from_scale_rotation_translation(
                        Vec3::splat(scale),
                        Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                        center,
                    )
                    .to_cols_array_2d(),
                    base_color: rgba(palette.violet),
                    emissive: rgba(palette.teal),
                    spec_color: rgba(palette.indigo_light),
                    legacy: rgba(palette.navy),
                    params: [0.0, if animate { 1.0 } else { 0.0 }, 1.0, 1.0],
                    flags: [
                        flags::FULLBRIGHT | flags::ALPHA_BLEND | flags::DOUBLE_SIDED | flags::PORTAL_EFFECT,
                        0,
                        effect.event.id.as_u128() as u16 as u32,
                        renderer.anim_clock().ms(now).wrapping_sub(8000),
                    ],
                    // Simultaneous arrivals keep a stable transparency order,
                    // independent of the subjects' HashMap iteration order.
                    identity: [aurora_render::oit_tie(*effect.event.id.as_bytes(), 0), 0, 0, 0],
                    ..Default::default()
                };
                let slot = if let Some(&slot) = self.records.get(used) {
                    renderer.records.set_if_changed(slot, record);
                    slot
                } else {
                    let slot = renderer.records.alloc(record);
                    self.records.push(slot);
                    slot
                };
                used += 1;
                self.cmds.push((
                    center,
                    DrawCmd {
                        index_count: mesh.index_count,
                        first_index: mesh.index_offset,
                        base_vertex: mesh.vertex_offset as i32,
                        record: slot,
                        bounds: [0.0; 4],
                    },
                ));
            }
        }
        for slot in self.records.drain(used..) {
            renderer.records.free(slot);
        }
    }
}

impl super::Scene {
    fn portal_mesh_bounds(&self, renderer: &Renderer, gpu: &super::ObjGpu) -> Vec<clipping::Bounds> {
        let Some(record) = gpu
            .faces
            .iter()
            .find(|f| !f.fully_transparent())
            .and_then(|face| renderer.records.get(face.record))
        else {
            return Vec::new();
        };
        let skinned = record.flags[0] & flags::SKINNED != 0;
        let anchored = record.flags[0] & flags::ANCHORED != 0;
        let model = if anchored {
            let slot = record.flags[1] as usize + if skinned { aurora_render::SKELETON_ANCHOR as usize } else { 0 };
            let Some(anchor) = self.palettes.get(slot) else { return Vec::new() };
            let anchor = Mat4::from_cols_array_2d(anchor);
            if skinned {
                anchor
            } else {
                anchor * Mat4::from_cols_array_2d(&record.model)
            }
        } else {
            Mat4::from_cols_array_2d(&record.model)
        };
        let geometry = gpu.geom.and_then(|key| self.geom_ready(&key));
        if skinned {
            let skin_box = |joint: &super::animesh::JointBounds| {
                let bind = self.skin_binds.list().get(record.flags[2] as usize + joint.joint as usize)?;
                let posed = self.palettes.get(record.flags[1] as usize + bind.joint[0] as usize)?;
                let matrix = model * Mat4::from_cols_array_2d(posed) * Mat4::from_cols_array_2d(&bind.inverse_bind);
                clipping::Bounds::new(joint.min, joint.max, matrix)
            };
            let mut bounds = Vec::new();
            if gpu.is_avatar {
                // Only currently drawn system parts count, including their
                // morphed geometry. Hidden bakes cannot delay completion.
                let keys = (0..self.avatar_lib.parts.len())
                    .map(|p| super::GeomKey::AvatarPart(p as u8))
                    .chain(gpu.morph_geoms.iter().copied());
                for key in keys {
                    let Some(geom) = self.geom_ready(&key) else { continue };
                    if geom.faces.iter().flatten().any(|alloc| {
                        gpu.faces
                            .iter()
                            .any(|f| !f.fully_transparent() && f.cmd.base_vertex == alloc.vertex_offset as i32)
                    }) {
                        bounds.extend(geom.joint_bounds.iter().filter_map(skin_box));
                    }
                }
            } else if let Some(geom) = geometry {
                bounds.extend(geom.joint_bounds.iter().filter_map(skin_box));
            }
            return bounds;
        }
        geometry
            .and_then(|g| clipping::Bounds::new(g.min, g.max, model))
            .into_iter()
            .collect()
    }

    pub fn portal_keeps_point(&self, avatar: usize, point: Vec3) -> bool {
        self.portals.planes.get(&avatar).is_none_or(|plane| plane.keeps_point(point))
    }

    pub fn update_portal_clipping(&mut self, renderer: &mut Renderer, world: &World) {
        let now = std::time::Instant::now();
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled = *ENABLED.get_or_init(|| std::env::var("AURORA_PORTAL_CLIP").as_deref() != Ok("0"));
        if !enabled || (world.portals.entries.is_empty() && world.portals.arrivals.is_empty() && !world.portals.awaiting_floor) {
            self.portals.clipping.clear();
            self.portals.planes.clear();
        } else {
            // A portal the setting hides cuts no body either.
            let portals: Vec<_> = world
                .portals
                .shown(world)
                .map(|entry| clipping::Aperture {
                    key: clipping::Key(entry.owner, entry.portal.id, entry.portal.region),
                    center: Vec3::from(entry.portal.center),
                    normal: Vec3::new(entry.portal.yaw.cos(), entry.portal.yaw.sin(), 0.0),
                    local_ready: entry.allowed(world)
                        && (world.portals.departure == Some(entry.portal.id) || entry.ready_to_cross(now) || entry.reveals_arrival()),
                })
                .collect();
            let mut avatars = Vec::new();
            let mut owners = HashMap::new();
            for (index, _) in self.list_index.with(super::list_index::AVATAR) {
                if let Some(object) = world.objects.get(index)
                    && let Some((root, rotation, false)) = super::Scene::object_transform(world, index, std::time::Instant::now(), 0)
                {
                    let physical = super::Scene::object_transform_unshifted(world, index, std::time::Instant::now()).map(|x| x.0);
                    if world
                        .portals
                        .arrivals
                        .get(&object.full_id)
                        .is_some_and(|e| physical.is_some_and(|p| e.matches(p)))
                    {
                        self.portals.clipping.forget(object.full_id);
                        continue;
                    }
                    owners.insert(index, avatars.len());
                    avatars.push(clipping::Avatar {
                        id: object.full_id,
                        own: object.full_id == world.agent_id,
                        in_transit: world.portals.departure.is_some(),
                        index,
                        root,
                        forward: rotation * Vec3::X,
                        placement_serial: object.placement_serial,
                        bounds: Vec::new(),
                    });
                }
            }
            // Spheres only shortlist nearby travelers. Posed joint/rigid boxes
            // decide when the whole mesh has passed, without a fixed world mask.
            for (index, _) in self.list_index.with(super::list_index::AVATAR_PART) {
                if let Some(gpu) = self.gpu.get(index)
                    && let Some(owner) = gpu.avatar(index).and_then(|owner| owners.get(&owner))
                {
                    let avatar = &mut avatars[*owner];
                    if self.portals.clipping.active(avatar.id) || portals.iter().any(|p| p.inside(avatar.root) && p.near(gpu.sphere())) {
                        avatar.bounds.extend(self.portal_mesh_bounds(renderer, gpu));
                    }
                }
            }
            self.portals.planes = self.portals.clipping.update(&portals, &avatars);
            if world.portals.awaiting_floor
                && let Some(index) = world.objects.index_of_uuid(&world.agent_id)
            {
                self.portals.clipping.forget(world.agent_id);
                self.portals.planes.insert(index, PortalClip::hidden());
            }
            if !world.portals.reduced_motion {
                let now = std::time::Instant::now();
                for (index, _) in self.list_index.with(super::list_index::AVATAR) {
                    if let Some(object) = world.objects.get(index)
                        && let Some(effect) = world.portals.arrivals.get(&object.full_id)
                        && let Some((root, _, _)) = super::Scene::object_transform_unshifted(world, index, now)
                        && effect.matches(root)
                        && let Some(clip) = effect.clip(now)
                    {
                        self.portals.planes.insert(index, clip);
                    }
                }
            }
        }
        if self.portals.planes.is_empty() && self.portals.clipped.is_empty() {
            return;
        }
        let mut next = HashSet::new();
        for (index, _) in self.list_index.with(super::list_index::AVATAR_PART) {
            let Some(gpu) = self.gpu.get(index) else { continue };
            let Some(plane) = object_plane(gpu, index, &self.portals.planes) else {
                continue;
            };
            for face in &gpu.faces {
                renderer.records.set_portal_clip(face.record, Some(plane));
            }
            next.insert(index);
        }
        for index in self.portals.clipped.difference(&next) {
            if let Some(gpu) = self.gpu.get(*index) {
                for face in &gpu.faces {
                    renderer.records.set_portal_clip(face.record, None);
                }
            }
        }
        self.portals.clipped = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_aperture_on_the_camera_to_avatar_segment_blocks_the_follow_view() {
        for yaw in [0.0, std::f32::consts::FRAC_PI_2, 2.3] {
            let normal = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
            let tangent = Vec3::new(-normal.y, normal.x, 0.0);
            let center = Vec3::new(128.0, 90.0, 31.0);
            for side in [-1.0, 1.0] {
                let eye = center + normal * (side * 3.0) + Vec3::Z;
                let avatar = center - normal * (side * 0.15);
                assert!(blocks_avatar(center, normal, eye, avatar));
                assert!(!blocks_avatar(center, normal, eye, center + normal * side));
                assert!(!blocks_avatar(center, normal, eye, center + normal * (side * 4.0)));
                assert!(!blocks_avatar(center, normal, eye + tangent * 3.0, avatar + tangent * 3.0));
                assert!(!blocks_avatar(center, normal, eye + Vec3::Z * 3.0, avatar + Vec3::Z * 3.0));
                assert!(!blocks_avatar(center, normal, avatar, avatar + Vec3::Z));
            }
        }
    }

    #[test]
    fn portal_opening_survives_reordering_motion_changes_and_clock_wrap() {
        use uuid::Uuid;
        let a = (Uuid::from_u128(1), Uuid::from_u128(2), 256);
        let b = (Uuid::from_u128(3), Uuid::from_u128(4), 256);
        let mut births = HashMap::new();
        assert_eq!(opening_birth(&mut births, a, 1000, true), 1000);
        assert_eq!(opening_birth(&mut births, b, 2000, true), 2000);
        assert_eq!(opening_birth(&mut births, a, 3000, true), 1000);
        let settled = opening_birth(&mut births, a, 3500, false);
        assert_eq!(3500_u32.wrapping_sub(settled), 8000);
        assert_eq!(opening_birth(&mut births, a, 4000, false), settled);
        assert_eq!(opening_birth(&mut births, a, 4500, true), settled);
        assert_eq!(births[&b], 2000);
        births.clear();
        let origin = u32::MAX - 100;
        assert_eq!(opening_birth(&mut births, a, origin, true), origin);
        assert_eq!(opening_birth(&mut births, a, 200, true), origin);
        let settled = opening_birth(&mut births, a, 400, false);
        assert_eq!(400_u32.wrapping_sub(settled), 8000);
        assert_eq!(opening_birth(&mut births, a, 500, true), settled);
    }
    #[test]
    fn portal_mesh_has_real_thickness_and_outward_rim_winding() {
        let (vertices, indices) = portal_mesh();
        assert_eq!(WIDTH, aurora_render::PORTAL_HALF_WIDTH);
        assert_eq!(HEIGHT, aurora_render::PORTAL_HALF_HEIGHT);
        assert!(vertices.iter().any(|v| v.pos[1] > 0.07));
        assert!(vertices.iter().any(|v| v.pos[1] < -0.07));
        let triangles = indices.as_chunks::<3>().0;
        assert_eq!(triangles.iter().filter(|tri| vertices[tri[0] as usize].uv[0] <= 1.0).count(), 2);
        let rim_triangles = triangles
            .iter()
            .filter(|tri| (2.0..=3.0).contains(&vertices[tri[0] as usize].uv[0]));
        assert!(rim_triangles.clone().next().is_some());
        for triangle in rim_triangles {
            let v = [triangle[0], triangle[1], triangle[2]].map(|i| vertices[i as usize]);
            let [a, b, c] = v.map(|v| Vec3::from(v.pos));
            let normal = Vec3::new(v[0].normal[0] as f32, v[0].normal[1] as f32, v[0].normal[2] as f32);
            assert!((b - a).cross(c - a).dot(normal) > 0.0);
            assert!(v.iter().all(|v| (2.0..=3.0).contains(&v.uv[0])));
        }
        let cloud: Vec<_> = vertices.iter().filter(|v| v.uv[0] >= 4.0).collect();
        assert!(cloud.iter().any(|v| v.pos[1] > 0.59));
        assert!(cloud.iter().any(|v| v.pos[1] < -0.59));
        assert!(cloud.iter().any(|v| v.pos[2] > HEIGHT + 0.4));
    }
    #[test]
    fn portal_plane_is_shared_by_body_rigid_and_skinned_attachments_but_never_huds_or_world_objects() {
        let plane = PortalClip::new(Vec3::new(12.0, 0.0, 0.0), Vec3::X).unwrap();
        let planes = HashMap::from([(7, plane)]);
        let body = super::super::ObjGpu {
            is_avatar: true,
            ..Default::default()
        };
        assert_eq!(object_plane(&body, 7, &planes), Some(plane));
        let mut worn = super::super::ObjGpu {
            owner_avatar: Some(7),
            ..Default::default()
        };
        assert_eq!(object_plane(&worn, 20, &planes), Some(plane));
        worn.rigged = true;
        assert_eq!(object_plane(&worn, 21, &planes), Some(plane));
        worn.hud = true;
        assert_eq!(object_plane(&worn, 22, &planes), None);
        assert_eq!(object_plane(&Default::default(), 23, &planes), None);
        assert_eq!(object_plane(&body, 8, &planes), None);
    }
}
