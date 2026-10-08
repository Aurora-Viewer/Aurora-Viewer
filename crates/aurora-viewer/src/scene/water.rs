//! Water surfaces: one quad per known region at that region's own water
//! height, plus "void" water at the agent region's height around them
//! (LLVOWater / LLVOVoidWater).

use crate::world::World;
use aurora_render::{DrawCmd, DrawRecord, MeshAlloc, Renderer, Vertex};
use glam::{Mat4, Vec3};

/// Void water: region-sized cells within this many regions of the main
/// one, then four large slabs out to `VOID_EXTENT`.
const VOID_CELLS: i32 = 4;
const VOID_EXTENT: f32 = 16384.0;

#[derive(Default)]
pub struct WaterGpu {
    quad: Option<MeshAlloc>,
    records: Vec<u32>,
    key: u64,
}

impl WaterGpu {
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for r in self.records.drain(..) {
            renderer.records.free(r);
        }
        if let Some(q) = self.quad.take() {
            renderer.free_mesh(q);
        }
        self.key = 0;
    }

    /// Rebuild the water records when regions, sizes or heights change.
    pub fn sync(&mut self, renderer: &mut Renderer, world: &World) {
        let Some(main) = world.main_region else {
            return;
        };
        // (offset, size, height) of every region with known water
        let mut regions: Vec<(Vec3, (u32, u32), f32)> = Vec::new();
        for (h, r) in &world.regions {
            let (Some(info), Some(offset)) = (r.info.as_ref(), world.region_offset(*h)) else {
                continue;
            };
            regions.push((offset, (info.size_x.max(256), info.size_y.max(256)), info.water_height));
        }
        regions.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.0.y.total_cmp(&b.0.y)));
        let main_height = world.main_water_height();
        let mut key = main ^ 0x5157_4154_4552;
        for (o, s, hgt) in &regions {
            key = key.rotate_left(7).wrapping_mul(0x100_0000_01B3)
                ^ (o.x.to_bits() as u64)
                ^ ((o.y.to_bits() as u64) << 32)
                ^ ((s.0 as u64) << 16)
                ^ (hgt.to_bits() as u64).rotate_left(40);
        }
        key ^= (main_height.to_bits() as u64) << 3;
        if key == self.key && self.quad.is_some() {
            return;
        }
        for r in self.records.drain(..) {
            renderer.records.free(r);
        }
        if self.quad.is_none() {
            let v = [
                Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]),
                Vertex::new([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0]),
                Vertex::new([1.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0]),
                Vertex::new([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0]),
            ];
            self.quad = renderer.upload_mesh(&v, &[0, 1, 2, 0, 2, 3]);
        }
        if self.quad.is_none() {
            return;
        }
        self.key = key;
        let mut add = |renderer: &mut Renderer, x: f32, y: f32, w: f32, h: f32, z: f32| {
            let model = Mat4::from_translation(Vec3::new(x, y, z)) * Mat4::from_scale(Vec3::new(w, h, 1.0));
            self.records.push(renderer.records.alloc(DrawRecord {
                model: model.to_cols_array_2d(),
                ..Default::default()
            }));
        };
        for (o, s, hgt) in &regions {
            add(renderer, o.x, o.y, s.0 as f32, s.1 as f32, *hgt);
        }
        // void water cells not covered by a region
        let covered = |x: f32, y: f32| {
            regions
                .iter()
                .any(|(o, s, _)| x >= o.x && x < o.x + s.0 as f32 && y >= o.y && y < o.y + s.1 as f32)
        };
        for j in -VOID_CELLS..=VOID_CELLS {
            for i in -VOID_CELLS..=VOID_CELLS {
                let (x, y) = (i as f32 * 256.0, j as f32 * 256.0);
                if !covered(x + 128.0, y + 128.0) {
                    add(renderer, x, y, 256.0, 256.0, main_height);
                }
            }
        }
        let lo = -VOID_CELLS as f32 * 256.0;
        let hi = (VOID_CELLS + 1) as f32 * 256.0;
        let e = VOID_EXTENT;
        add(renderer, -e, hi, 2.0 * e, e - hi, main_height); // north
        add(renderer, -e, -e, 2.0 * e, e + lo, main_height); // south
        add(renderer, -e, lo, e + lo, hi - lo, main_height); // west
        add(renderer, hi, lo, e - hi, hi - lo, main_height); // east
    }

    pub fn draws(&self) -> impl Iterator<Item = DrawCmd> + '_ {
        let q = self.quad;
        self.records.iter().filter_map(move |r| {
            q.map(|a| DrawCmd {
                index_count: a.index_count,
                first_index: a.index_offset,
                base_vertex: a.vertex_offset as i32,
                record: *r,
                bounds: [0.0; 4],
            })
        })
    }
}
