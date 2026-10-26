//! Shared GPU disc and procedural aperture texture, depth-tested and sorted
//! with world transparency. No simulator objects, physics or per-frame uploads.
use crate::{
    theme::Palette,
    world::{
        World,
        portals::{self, HEIGHT, WIDTH},
    },
};
use aurora_render::{DrawCmd, DrawRecord, MeshAlloc, MipLevel, Renderer, Vertex, build_mips, types::flags};
use glam::{Mat4, Quat, Vec3};

#[derive(Default)]
pub struct Portals {
    quad: Option<MeshAlloc>,
    texture: Option<u32>,
    records: Vec<u32>,
    pub cmds: Vec<(f32, DrawCmd)>,
}

impl Portals {
    pub fn clear(&mut self, renderer: &mut Renderer) {
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
            let color = if portals::allowed(world, entry.owner, portal.access) {
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
