//! Local circular podium and an ocean plane beneath the inspection avatar.
//! Private GPU allocations, never simulator objects or inventory assets.

use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, ParticleInstance, Renderer, Vertex, flags, particle_flags};
use glam::{Mat4, Quat, Vec3};

#[derive(Default)]
pub struct Stage {
    parts: Vec<(MeshAlloc, u32)>,
}

pub const WATER_OFFSET: f32 = -0.18;

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
    const SEGMENTS: u16 = 64;
    let mut vertices = vec![Vertex::new([0.0; 3], Vec3::Z.to_array(), [0.5; 2])];
    let mut indices = Vec::with_capacity(SEGMENTS as usize * 9);
    for i in 0..=SEGMENTS {
        let angle = f32::from(i) / f32::from(SEGMENTS) * std::f32::consts::TAU;
        let normal = Vec3::new(angle.cos(), angle.sin(), 0.0);
        let top = normal * 1.15;
        vertices.push(Vertex::new(
            top.to_array(),
            Vec3::Z.to_array(),
            [normal.x * 0.5 + 0.5, normal.y * 0.5 + 0.5],
        ));
        vertices.push(Vertex::new(
            top.to_array(),
            normal.to_array(),
            [f32::from(i) / f32::from(SEGMENTS), 0.0],
        ));
        vertices.push(Vertex::new(
            (top - Vec3::Z * 0.35).to_array(),
            normal.to_array(),
            [f32::from(i) / f32::from(SEGMENTS), 1.0],
        ));
        if i < SEGMENTS {
            let a = 1 + i * 3;
            indices.extend([0, a, a + 3, a + 1, a + 2, a + 5, a + 1, a + 5, a + 4]);
        }
    }
    (vertices, indices)
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
                    base_color: [0.18, 0.18, 0.2, 1.0],
                    flags: [0, 0, 0, flags::STUDIO_STAGE],
                    ..Default::default()
                },
            );
            let draws = if i == 0 { &mut lists.opaque } else { &mut lists.water };
            draws.push(DrawCmd {
                index_count: mesh.index_count,
                first_index: mesh.index_offset,
                base_vertex: mesh.vertex_offset as i32,
                record: *record,
                bounds: [0.0; 4],
            });
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
