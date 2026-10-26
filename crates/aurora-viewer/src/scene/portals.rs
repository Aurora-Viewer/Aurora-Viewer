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

fn object_plane(gpu: &super::ObjGpu, index: usize, planes: &HashMap<usize, PortalClip>) -> Option<PortalClip> {
    if gpu.hud {
        return None;
    }
    gpu.avatar(index).and_then(|owner| planes.get(&owner)).copied()
}

#[derive(Default)]
pub struct Portals {
    pub planes: HashMap<usize, PortalClip>,
    clipped: HashSet<usize>,
    clipping: clipping::Clipping,
    quad: Option<MeshAlloc>,
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
        self.cmds.clear();
    }

    pub fn update(&mut self, renderer: &mut Renderer, world: &World, eye: Vec3, palette: &Palette, _time: f32, animate: bool) {
        self.cmds.clear();
        if world.portals.visible(world).next().is_some() && self.quad.is_none() {
            let vertices = [
                Vertex::new([-1.0, 0.0, -1.0], [0.0, -1.0, 0.0], [0.0, 1.0]),
                Vertex::new([1.0, 0.0, -1.0], [0.0, -1.0, 0.0], [1.0, 1.0]),
                Vertex::new([1.0, 0.0, 1.0], [0.0, -1.0, 0.0], [1.0, 0.0]),
                Vertex::new([-1.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0, 0.0]),
            ];
            // Two windings for the ordinary back-face-culled blend/glow passes.
            // The shader keeps the actual opening at r=1 and fades its corona
            // outside it; the corona never enlarges avatar clipping.
            let indices = [0, 1, 2, 0, 2, 3, 2, 1, 0, 3, 2, 0];
            self.quad = renderer.upload_mesh(&vertices, &indices);
        }
        let Some(quad) = self.quad else {
            return;
        };
        let mut used = 0;
        for entry in world.portals.visible(world) {
            let portal = &entry.portal;
            let center = Vec3::from(portal.center);
            let allowed = entry.allowed(world);
            let rgba = |color: egui::Color32| color.to_srgba_unmultiplied().map(|value| value as f32 / 255.0);
            let accent = |color| rgba(if allowed { color } else { palette.muted });
            let record = DrawRecord {
                model: Mat4::from_scale_rotation_translation(
                    Vec3::new(
                        WIDTH * aurora_render::PORTAL_SURFACE_SCALE,
                        1.0,
                        HEIGHT * aurora_render::PORTAL_SURFACE_SCALE,
                    ),
                    Quat::from_rotation_z(portal.yaw - std::f32::consts::FRAC_PI_2),
                    center,
                )
                .to_cols_array_2d(),
                base_color: accent(palette.violet),
                emissive: accent(palette.teal),
                spec_color: accent(palette.indigo_light),
                legacy: rgba(palette.navy),
                params: [0.0, if animate { 1.0 } else { 0.0 }, if allowed { 1.0 } else { 0.35 }, 0.5],
                flags: [
                    flags::FULLBRIGHT | flags::ALPHA_BLEND | flags::DOUBLE_SIDED | flags::PORTAL_EFFECT,
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
        self.portals.planes.get(&avatar).is_none_or(|plane| plane.keeps_point(point))
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
