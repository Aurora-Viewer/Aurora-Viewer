//! Ban lines (LLViewerParcelMgr::renderCollisionSegments): striped walls on
//! the borders of the parcel the agent may not enter, fading out within
//! 13 m of the agent, 5000 m high when banned (or when the region blocks
//! fly-over), 50 m otherwise; green "pass" stripes when the parcel sells
//! passes and we are only missing from its access list.
//!
//! Ported from Firestorm's llglsandbox.cpp / llviewerparcelmgr.cpp
//! (originally LGPL 2.1, Linden Research, Inc.). The stripe textures are drawn here.

use crate::world::World;
use aurora_render::types::flags;
use aurora_render::{DrawCmd, DrawRecord, MeshAlloc, MipLevel, Renderer, Vertex, build_mips};
use glam::{Mat4, Vec3, Vec4};

/// PARCEL_GRID_STEP_METERS.
const STEP: f32 = 4.0;
/// BAN_HEIGHT / PARCEL_HEIGHT (llparcel.h).
const BAN_HEIGHT: f32 = 5000.0;
const PARCEL_HEIGHT: f32 = 50.0;
const REGION_FLAGS_BLOCK_FLYOVER: u32 = 1 << 27;
const MAX_ALPHA: f32 = 0.95;
const DIST_OFFSET: f32 = 5.0;
const MIN_DIST_SQ: f32 = DIST_OFFSET * DIST_OFFSET;
const MAX_DIST_SQ: f32 = 169.0;
const SOUTH: u8 = 1;
const WEST: u8 = 2;

#[derive(Default)]
pub struct BanLines {
    quad: Option<MeshAlloc>,
    /// Stripe textures: blocked (yellow), pass (green).
    tex: Option<[u32; 2]>,
    records: Vec<u32>,
    /// Blended draws with their distance to the camera (sorted with the
    /// other alpha faces).
    pub cmds: Vec<(f32, DrawCmd)>,
}

/// 256² band of diagonal stripes in the middle rows, transparent around
/// (world/NoEntryLines.png layout: the texture repeats every 2 m).
fn stripes(base: [u8; 3], light: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; 256 * 256 * 4];
    for y in 116..140usize {
        for x in 0..256usize {
            let c = if (x + y) % 32 < 12 { light } else { base };
            let i = (y * 256 + x) * 4;
            px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 235]);
        }
    }
    px
}

fn upload(renderer: &mut Renderer, rgba: Vec<u8>) -> Option<u32> {
    let mips = build_mips(256, 256, rgba, 9);
    let levels: Vec<MipLevel> = mips
        .iter()
        .map(|(w, h, d)| MipLevel {
            width: *w,
            height: *h,
            data: d,
        })
        .collect();
    renderer.create_texture(&levels)
}

impl BanLines {
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for r in self.records.drain(..) {
            renderer.records.free(r);
        }
        self.cmds.clear();
        if let Some(q) = self.quad.take() {
            renderer.free_mesh(q);
        }
        if let Some(t) = self.tex.take() {
            for s in t {
                renderer.free_texture(s);
            }
        }
    }

    /// Rebuild the wall draws for this frame (`mode`: ShowBanLines).
    pub fn update(&mut self, renderer: &mut Renderer, world: &World, mode: u8, eye: Vec3) {
        self.cmds.clear();
        let mut used = 0;
        if world.map.ban_lines_visible(mode) {
            used = self.build(renderer, world, eye);
        }
        for r in self.records.drain(used..) {
            renderer.records.free(r);
        }
    }

    fn build(&mut self, renderer: &mut Renderer, world: &World, eye: Vec3) -> usize {
        let Some(c) = world.map.collision.as_ref() else {
            return 0;
        };
        let (Some(region), Some(offset)) = (world.regions.get(&c.handle), world.region_offset(c.handle)) else {
            return 0;
        };
        if self.quad.is_none() {
            // unit quad (s along the border, t up), both windings
            let v = [
                Vertex::new([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0]),
                Vertex::new([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0]),
                Vertex::new([0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0, 1.0]),
                Vertex::new([1.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 1.0]),
            ];
            self.quad = renderer.upload_mesh(&v, &[0, 1, 3, 0, 3, 2, 0, 3, 1, 0, 2, 3]);
        }
        if self.tex.is_none() {
            let blocked = upload(renderer, stripes([222, 214, 98], [236, 232, 170]));
            let pass = upload(renderer, stripes([96, 176, 72], [150, 206, 128]));
            if let (Some(a), Some(b)) = (blocked, pass) {
                self.tex = Some([a, b]);
            }
        }
        let (Some(quad), Some(tex)) = (self.quad, self.tex) else {
            return 0;
        };
        let hm = &region.heightmap;
        let n = hm.size_x / STEP as u32;
        // writeSegmentsFromBitmap: borders between set and clear cells
        let stride = (n + 1) as usize;
        let mut seg = vec![0u8; stride * stride];
        for y in 0..n {
            for x in 0..n {
                if c.cell(x, y, n) {
                    let o = x as usize + y as usize * stride;
                    seg[o] ^= SOUTH;
                    seg[o + stride] ^= SOUTH;
                    seg[o] ^= WEST;
                    seg[o + 1] ^= WEST;
                }
            }
        }
        let flyover = region
            .info
            .as_ref()
            .is_some_and(|i| i.region_flags & REGION_FLAGS_BLOCK_FLYOVER != 0);
        let height = if c.kind == 1 || flyover { BAN_HEIGHT } else { PARCEL_HEIGHT };
        let slot = if c.use_pass && c.kind == 3 { tex[1] } else { tex[0] };
        let me = world.agent.position - offset;
        let border = hm.size_x as f32 - 0.1;
        let ground = |x: f32, y: f32| hm.sample(x.min(border), y.min(border));
        let alpha_of = |d2: f32| {
            if d2 < MIN_DIST_SQ {
                MAX_ALPHA
            } else if d2 > MAX_DIST_SQ {
                0.0
            } else {
                (30.0 / d2).clamp(0.0, MAX_ALPHA)
            }
        };
        let mut used = 0;
        for y in 0..stride {
            for x in 0..stride {
                let m = seg[x + y * stride];
                for dir in [SOUTH, WEST] {
                    if m & dir == 0 {
                        continue;
                    }
                    let (x1, y1) = (x as f32 * STEP, y as f32 * STEP);
                    let (x2, y2, d2) = if dir == SOUTH {
                        let dy = (me.y - y1) + DIST_OFFSET;
                        let dx = if me.x < x1 {
                            me.x - x1
                        } else if me.x > x1 + STEP {
                            me.x - x1 - STEP
                        } else {
                            0.0
                        };
                        (x1 + STEP, y1, dx * dx + dy * dy)
                    } else {
                        let dx = (me.x - x1) + DIST_OFFSET;
                        let dy = if me.y < y1 {
                            me.y - y1
                        } else if me.y > y1 + STEP {
                            me.y - y1 - STEP
                        } else {
                            0.0
                        };
                        (x1, y1 + STEP, dx * dx + dy * dy)
                    };
                    let alpha = alpha_of(d2);
                    if alpha <= 0.0 {
                        continue;
                    }
                    // a vertical quad from the lower corner (the terrain hides
                    // what is under the ground) to the higher one + height, so
                    // the stripes (t = z * 0.5) stay level from one segment to
                    // the next; + 0.1 m against z-fighting with property lines
                    let (z1, z2) = (ground(x1, y1), ground(x2, y2));
                    let (zlo, zhi) = (z1.min(z2), z1.max(z2));
                    let tall = zhi - zlo + height;
                    let p1 = offset + Vec3::new(x1 + 0.1, y1 + 0.1, zlo);
                    let along = Vec3::new(x2 - x1, y2 - y1, 0.0);
                    let side = Vec3::new(-(y2 - y1), x2 - x1, 0.0).normalize_or_zero();
                    let model = Mat4::from_cols(along.extend(0.0), side.extend(0.0), Vec4::new(0.0, 0.0, tall, 0.0), p1.extend(1.0));
                    // texture: s = position along the border * 0.5, t = z * 0.5
                    let start = if dir == SOUTH { x1 } else { y1 };
                    let (su, sv) = (STEP * 0.5, tall * 0.5);
                    let (ou, ov) = (start * 0.5 + 0.5, zlo * 0.5);
                    let rec = DrawRecord {
                        model: model.to_cols_array_2d(),
                        base_color: [1.0, 1.0, 1.0, alpha],
                        // ll_uv scales around 0.5: shift so that uv' = uv * s + o
                        uv_st: [su, sv, ou + 0.5 * su - 0.5, ov + 0.5 * sv - 0.5],
                        tex: [
                            slot,
                            aurora_render::textures::FLAT_NORMAL,
                            aurora_render::textures::WHITE,
                            aurora_render::textures::WHITE,
                        ],
                        flags: [flags::FULLBRIGHT | flags::ALPHA_BLEND, 0, 0, 0],
                        ..Default::default()
                    };
                    let record = match self.records.get(used) {
                        Some(&r) => {
                            renderer.records.set(r, rec);
                            r
                        }
                        None => {
                            let r = renderer.records.alloc(rec);
                            self.records.push(r);
                            r
                        }
                    };
                    used += 1;
                    // sort point: the wall at the eye's height
                    let mid = p1 + along * 0.5 + Vec3::Z * (eye.z.clamp(p1.z, p1.z + tall) - p1.z);
                    self.cmds.push((
                        eye.distance(mid),
                        DrawCmd {
                            index_count: quad.index_count,
                            first_index: quad.index_offset,
                            base_vertex: quad.vertex_offset as i32,
                            record,
                            bounds: [0.0; 4],
                        },
                    ));
                }
            }
        }
        used
    }
}
