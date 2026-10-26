//! Shared GPU disc and procedural aperture texture, depth-tested and sorted
//! with world transparency. No simulator objects, physics or per-frame uploads.
use crate::{
    theme::Palette,
    world::{
        World,
        portals::{HEIGHT, WIDTH},
    },
};
use aurora_render::{DrawCmd, DrawRecord, MeshAlloc, MipLevel, Renderer, Vertex, build_mips, types::flags};
use glam::{Mat4, Quat, Vec3};
use std::collections::{HashMap, HashSet};
mod clipping;

fn object_plane(gpu: &super::ObjGpu, index: usize, planes: &HashMap<usize, [f32; 4]>) -> Option<[f32; 4]> {
    if gpu.hud {
        return None;
    }
    gpu.avatar(index).and_then(|owner| planes.get(&owner)).copied()
}

#[derive(Default)]
pub struct Portals {
    pub planes: HashMap<usize, [f32; 4]>,
    clipped: HashSet<usize>,
    clipping: clipping::Clipping,
    quad: Option<MeshAlloc>,
    texture: Option<u32>,
    records: Vec<u32>,
    pub cmds: Vec<(f32, DrawCmd)>,
}

impl Portals {
    pub fn clear(&mut self, renderer: &mut Renderer) {
        self.planes.clear();
        self.clipped.clear();
        self.clipping.clear();
        for record in self.records.drain(..) {
            renderer.records.free(record);
        }
        if let Some(mesh) = self.quad.take() {
            renderer.free_mesh(mesh);
        }
        if let Some(texture) = self.texture.take() {
            renderer.free_texture(texture);
        }
        self.cmds.clear();
    }

    pub fn update(&mut self, renderer: &mut Renderer, world: &World, eye: Vec3, palette: &Palette, time: f32, animate: bool) {
        self.cmds.clear();
        if !world.portals.entries.is_empty() && self.quad.is_none() {
            let mut vertices = vec![Vertex::new([0.0; 3], [0.0, -1.0, 0.0], [0.5, 0.5])];
            let mut indices = Vec::with_capacity(64 * 6);
            for i in 0..64u16 {
                let angle = i as f32 * std::f32::consts::TAU / 64.0;
                let (z, x) = angle.sin_cos();
                vertices.push(Vertex::new([x, 0.0, z], [0.0, -1.0, 0.0], [x * 0.5 + 0.5, 0.5 - z * 0.5]));
                let next = (i + 1) % 64 + 1;
                indices.extend_from_slice(&[0, i + 1, next, 0, next, i + 1]);
            }
            // A disc keeps rotated UVs inside the texture's inscribed circle;
            // the ordinary repeating world sampler cannot repeat its corners.
            self.quad = renderer.upload_mesh(&vertices, &indices);
        }
        if !world.portals.entries.is_empty() && self.texture.is_none() {
            let size = 128;
            let mut pixels = Vec::with_capacity(size * size * 4);
            for y in 0..size {
                for x in 0..size {
                    let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                    let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                    let radius = u.hypot(v);
                    let ring = (1.0 - (radius - 0.91).abs() / 0.065).clamp(0.0, 1.0);
                    let swirl = ((v.atan2(u) * 4.0 + radius * 17.0).sin() * 0.5 + 0.5) * 0.25;
                    let alpha = if radius < 0.94 { 0.18 + swirl + ring * 0.6 } else { ring * 0.9 };
                    pixels.extend_from_slice(&[255, 255, 255, (alpha.clamp(0.0, 1.0) * 255.0) as u8]);
                }
            }
            let mips = build_mips(size as u32, size as u32, pixels, 8);
            let levels: Vec<_> = mips
                .iter()
                .map(|(w, h, data)| MipLevel {
                    width: *w,
                    height: *h,
                    data,
                })
                .collect();
            self.texture = renderer.create_texture(&levels);
        }
        let (Some(quad), Some(texture)) = (self.quad, self.texture) else {
            return;
        };
        let mut used = 0;
        for entry in &world.portals.entries {
            let portal = &entry.portal;
            let center = Vec3::from(portal.center);
            let color = if entry.allowed(world) {
                palette.violet_light
            } else {
                palette.muted
            };
            let record = DrawRecord {
                model: Mat4::from_scale_rotation_translation(
                    Vec3::new(WIDTH, 1.0, HEIGHT),
                    Quat::from_rotation_z(portal.yaw - std::f32::consts::FRAC_PI_2),
                    center,
                )
                .to_cols_array_2d(),
                base_color: color.to_srgba_unmultiplied().map(|value| value as f32 / 255.0),
                params: [if animate { time * 0.15 } else { 0.0 }, 0.0, 0.8, 0.5],
                tex: [
                    texture,
                    aurora_render::textures::FLAT_NORMAL,
                    aurora_render::textures::WHITE,
                    aurora_render::textures::WHITE,
                ],
                flags: [flags::FULLBRIGHT | flags::ALPHA_BLEND, 0, 0, 0],
                ..Default::default()
            };
            let slot = if let Some(&slot) = self.records.get(used) {
                renderer.records.set(slot, record);
                slot
            } else {
                let slot = renderer.records.alloc(record);
                self.records.push(slot);
                slot
            };
            used += 1;
            self.cmds.push((
                eye.distance(center),
                DrawCmd {
                    index_count: quad.index_count,
                    first_index: quad.index_offset,
                    base_vertex: quad.vertex_offset as i32,
                    record: slot,
                    bounds: [0.0; 4],
                },
            ));
        }
        for slot in self.records.drain(used..) {
            renderer.records.free(slot);
        }
    }
}

impl super::Scene {
    pub fn portal_keeps_point(&self, avatar: usize, point: Vec3) -> bool {
        self.portals
            .planes
            .get(&avatar)
            .is_none_or(|plane| Vec3::from_slice(&plane[..3]).dot(point) + plane[3] >= -0.0001)
    }

    pub fn update_portal_clipping(&mut self, renderer: &mut Renderer, world: &World) {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled = *ENABLED.get_or_init(|| std::env::var("AURORA_PORTAL_CLIP").as_deref() != Ok("0"));
        if !enabled || world.portals.entries.is_empty() {
            self.portals.clipping.clear();
            self.portals.planes.clear();
        } else {
            let portals: Vec<_> = world
                .portals
                .entries
                .iter()
                .map(|entry| clipping::Aperture {
                    key: clipping::Key(entry.owner, entry.portal.id, entry.portal.region),
                    center: Vec3::from(entry.portal.center),
                    normal: Vec3::new(entry.portal.yaw.cos(), entry.portal.yaw.sin(), 0.0),
                })
                .collect();
            let mut avatars = Vec::new();
            let mut owners = HashMap::new();
            for (index, _) in self.list_index.with(super::list_index::AVATAR) {
                if let Some(object) = world.objects.get(index)
                    && let Some((root, rotation, false)) = super::Scene::object_transform(world, index, std::time::Instant::now(), 0)
                {
                    owners.insert(index, avatars.len());
                    avatars.push(clipping::Avatar {
                        id: object.full_id,
                        index,
                        root,
                        forward: rotation * Vec3::X,
                        bounds: Vec::new(),
                    });
                }
            }
            // The byte index selects only bodies and world attachments. Posed
            // spheres include skinning and rigid attachment-point animation.
            for (index, _) in self.list_index.with(super::list_index::AVATAR_PART) {
                if let Some(gpu) = self.gpu.get(index)
                    && let Some(owner) = gpu.avatar(index).and_then(|owner| owners.get(&owner))
                {
                    avatars[*owner].bounds.push(gpu.sphere());
                }
            }
            self.portals.planes = self.portals.clipping.update(&portals, &avatars);
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
                renderer.records.set_portal_clip(face.record, plane);
            }
            next.insert(index);
        }
        for index in self.portals.clipped.difference(&next) {
            if let Some(gpu) = self.gpu.get(*index) {
                for face in &gpu.faces {
                    renderer.records.set_portal_clip(face.record, [0.0; 4]);
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
    fn portal_plane_is_shared_by_body_rigid_and_skinned_attachments_but_never_huds_or_world_objects() {
        let plane = [1.0, 0.0, 0.0, -12.0];
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
