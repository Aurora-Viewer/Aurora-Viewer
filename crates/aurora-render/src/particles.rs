//! Scripted blend factors and ordered particle batches. Wire values and glow
//! suppression follow LLDrawPoolAlpha::renderAlpha (indra/newview/lldrawpoolalpha.cpp,
//! originally LGPL 2.1). Never regroup particles across their back-to-front order.

use crate::ParticleInstance;
use std::ops::Range;

const FACTORS: [u8; 8] = [7, 9, 0, 1, 2, 3, 4, 5];
pub(crate) const BLEND_COUNT: usize = 64;

/// The wire protocol excludes destination-alpha factors. Invalid external values
/// fall back independently to the ordinary alpha source/destination factors.
#[inline]
pub fn blend_index(source: u8, destination: u8) -> u32 {
    const fn lookup(default: u8) -> [u8; 256] {
        let mut table = [default; 256];
        let mut i = 0;
        while i < FACTORS.len() {
            table[FACTORS[i] as usize] = i as u8;
            i += 1;
        }
        table
    }
    const SOURCE: [u8; 256] = lookup(0);
    const DESTINATION: [u8; 256] = lookup(1);
    (u32::from(SOURCE[source as usize]) * 8 + u32::from(DESTINATION[destination as usize])) ^ 1
}

pub(crate) fn blend_state(index: usize) -> wgpu::BlendState {
    let factor = |wire| match wire {
        0 => wgpu::BlendFactor::One,
        1 => wgpu::BlendFactor::Zero,
        2 => wgpu::BlendFactor::Dst,
        3 => wgpu::BlendFactor::Src,
        4 => wgpu::BlendFactor::OneMinusDst,
        5 => wgpu::BlendFactor::OneMinusSrc,
        7 => wgpu::BlendFactor::SrcAlpha,
        _ => wgpu::BlendFactor::OneMinusSrcAlpha,
    };
    let pair = index.min(BLEND_COUNT - 1) ^ 1;
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: factor(FACTORS[pair / 8]),
            dst_factor: factor(FACTORS[pair % 8]),
            operation: wgpu::BlendOperation::Add,
        },
        // Scene alpha holds glow, independently of scripted colour blending.
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Batch {
    pub instances: Range<u32>,
    pub blend: usize,
    pub glow: bool,
}

/// Like the reference alpha pool, add a batch's glow after its colour, then
/// allow nearer batches to suppress it. Bindless textures stay in one draw
/// unless glow needs that per-texture boundary. No allocation or global regroup.
pub(crate) fn batches(particles: &[ParticleInstance], glow_on: bool) -> impl Iterator<Item = Batch> + '_ {
    let has_glow = |p: &ParticleInstance| p.glow > 0.0 || p.end_glow > 0.0;
    let glow_on = glow_on && particles.iter().any(has_glow);
    let key = move |p: &ParticleInstance| (p.blend.min((BLEND_COUNT - 1) as u32), if glow_on { p.texture } else { 0 });
    let mut from = 0;
    std::iter::from_fn(move || {
        let first = particles.get(from)?;
        let state = key(first);
        let mut end = from + 1;
        let mut glow = glow_on && has_glow(first);
        while let Some(p) = particles.get(end) {
            if key(p) != state {
                break;
            }
            glow |= glow_on && has_glow(p);
            end += 1;
        }
        let batch = Batch {
            instances: from as u32..end as u32,
            blend: state.0 as usize,
            glow,
        };
        from = end;
        Some(batch)
    })
}

/// Camera clipping only; simulation, picking, texture priority and sort keys
/// remain untouched. Planes are the exact (possibly jittered/tiled) GPU matrix.
pub(crate) struct Frustum([glam::Vec4; 6]);

impl Frustum {
    pub fn new(matrix: glam::Mat4) -> Self {
        let rows = matrix.transpose();
        let planes = [
            rows.w_axis + rows.x_axis,
            rows.w_axis - rows.x_axis,
            rows.w_axis + rows.y_axis,
            rows.w_axis - rows.y_axis,
            rows.z_axis,
            rows.w_axis - rows.z_axis,
        ];
        Self(planes.map(|p| {
            let length = p.truncate().length();
            if length > 0.0 && length.is_finite() {
                p / length
            } else {
                glam::Vec4::ZERO
            }
        }))
    }

    pub fn visible(&self, particle: &ParticleInstance) -> bool {
        use glam::Vec3;
        let mut center = Vec3::from_array(particle.pos);
        let radius = if particle.flags & crate::particle_flags::AXIS != 0 {
            let end = Vec3::from_array(particle.end_pos);
            let half = (end - center) * 0.5;
            center += half;
            half.length()
                + (Vec3::from_array(particle.axis).length() * particle.size[0].abs())
                    .max(Vec3::from_array(particle.end_axis).length() * particle.end_width.abs())
                    * 0.5
        } else {
            glam::Vec2::from_array(particle.size).length() * 0.5
        };
        if !center.is_finite() || !radius.is_finite() {
            return true;
        }
        // Keep borderline geometry despite CPU/GPU rounding and large heights.
        let margin = 0.001 + center.abs().max_element() * 1e-5;
        self.0
            .iter()
            .all(|plane| plane.truncate().dot(center) + plane.w >= -radius - margin)
    }
}

pub(crate) fn glow_enabled(enabled: bool, cutout: bool, scene_glow: bool, particles: &[ParticleInstance]) -> bool {
    enabled && !cutout && (scene_glow || particles.iter().any(has_glow))
}

pub(crate) fn has_glow(p: &ParticleInstance) -> bool {
    p.glow > 0.0 || p.end_glow > 0.0
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Draw {
    pub color: Range<u32>,
    pub glow: Range<u32>,
    pub blend: usize,
}

#[derive(Default)]
pub(crate) struct Plan {
    pub draws: Vec<Draw>,
    pub stats: crate::ParticleDrawStats,
}

impl Plan {
    /// Trim only invisible ends of each original batch. In particular, an
    /// invisible intervening batch must not merge two colour/glow groups:
    /// nearer colour still suppresses the preceding group's glow.
    pub fn prepare(&mut self, particles: &[ParticleInstance], glow_on: bool, frustum: &Frustum) {
        self.draws.clear();
        self.stats = crate::ParticleDrawStats::default();
        for batch in batches(particles, glow_on) {
            self.stats.batches += 1;
            let from = batch.instances.start as usize;
            let to = batch.instances.end as usize;
            let slice = &particles[from..to];
            let Some(first) = slice.iter().position(|p| frustum.visible(p)) else {
                continue;
            };
            let last = if first + 1 == slice.len() {
                first
            } else {
                slice.iter().rposition(|p| frustum.visible(p)).unwrap_or(first)
            };
            let color = (from + first) as u32..(from + last + 1) as u32;
            let glow = if batch.glow {
                let p = &particles[color.start as usize..color.end as usize];
                let first = p
                    .iter()
                    .enumerate()
                    .position(|(i, item)| has_glow(item) && (i == 0 || i + 1 == p.len() || frustum.visible(item)));
                let last = p
                    .iter()
                    .enumerate()
                    .rposition(|(i, item)| has_glow(item) && (i == 0 || i + 1 == p.len() || frustum.visible(item)));
                match (first, last) {
                    (Some(a), Some(b)) => color.start + a as u32..color.start + b as u32 + 1,
                    _ => 0..0,
                }
            } else {
                0..0
            };
            self.stats.color_draws += 1;
            self.stats.glow_draws += u32::from(!glow.is_empty());
            self.stats.color_instances += color.len() as u32;
            self.stats.glow_instances += glow.len() as u32;
            // A preceding group with no glow can share this colour draw:
            // its colour was already before this group's colour and glow.
            // A non-empty preceding glow range is an absolute barrier.
            if let Some(previous) = self.draws.last_mut()
                && previous.blend == batch.blend
                && previous.glow.is_empty()
                && previous.color.end == color.start
            {
                previous.color.end = color.end;
                previous.glow = glow;
                self.stats.color_draws -= 1;
            } else {
                self.draws.push(Draw {
                    color,
                    glow,
                    blend: batch.blend,
                });
            }
        }
        self.stats.culled = particles.len() as u32 - self.stats.color_instances;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_particle_glow_does_not_require_a_glowing_object() {
        let p = [ParticleInstance {
            glow: 0.5,
            ..Default::default()
        }];
        assert!(glow_enabled(true, false, false, &p));
        assert!(!glow_enabled(false, false, true, &p));
        assert!(!glow_enabled(true, true, true, &p));
        assert!(!glow_enabled(true, false, false, &[]));
        assert!(glow_enabled(true, false, true, &[]));
    }

    #[test]
    fn conservative_visibility_keeps_large_billboards_parent_ends_and_invalid_data() {
        let view = Frustum::new(glam::Mat4::IDENTITY);
        let mut p = ParticleInstance {
            pos: [1.1, 0.0, 0.5],
            size: [0.4, 0.4],
            ..Default::default()
        };
        assert!(view.visible(&p), "a centre outside does not reject a visible edge");
        p.pos[0] = 1.4;
        assert!(!view.visible(&p));
        p.flags = crate::particle_flags::AXIS;
        p.pos = [3.0, 0.0, 0.5];
        p.end_pos = [0.0, 0.0, 0.5];
        p.end_axis = [0.0, 2.0, 0.0];
        p.end_width = 0.25;
        assert!(view.visible(&p), "the parent end remains visible");
        p.end_pos[0] = 4.0;
        assert!(!view.visible(&p));
        p.pos[0] = f32::NAN;
        assert!(view.visible(&p), "invalid bounds are never used for rejection");
    }

    #[test]
    fn culling_an_intervening_batch_does_not_merge_glow_groups() {
        let mut p = [ParticleInstance {
            pos: [0.0, 0.0, 0.5],
            size: [0.2; 2],
            texture: 1,
            glow: 0.5,
            ..Default::default()
        }; 3];
        p[1].pos[0] = 5.0;
        p[1].blend = blend_index(1, 7);
        let mut plan = Plan::default();
        plan.prepare(&p, true, &Frustum::new(glam::Mat4::IDENTITY));
        assert_eq!(plan.draws.iter().map(|d| d.color.clone()).collect::<Vec<_>>(), [0..1, 2..3]);
        assert_eq!(plan.draws.iter().map(|d| d.glow.clone()).collect::<Vec<_>>(), [0..1, 2..3]);
        assert_eq!(
            (plan.stats.batches, plan.stats.color_draws, plan.stats.glow_draws, plan.stats.culled),
            (3, 2, 2, 1)
        );
    }

    #[test]
    fn only_non_emitting_glow_boundaries_can_be_coalesced() {
        let mut p = [ParticleInstance {
            pos: [0.0, 0.0, 0.5],
            size: [0.2; 2],
            ..Default::default()
        }; 4];
        for (i, p) in p.iter_mut().enumerate() {
            p.texture = i as u32 + 1;
        }
        p[2].glow = 0.5;
        let mut plan = Plan::default();
        plan.prepare(&p, true, &Frustum::new(glam::Mat4::IDENTITY));
        assert_eq!(plan.draws.iter().map(|d| d.color.clone()).collect::<Vec<_>>(), [0..3, 3..4]);
        assert_eq!(plan.draws[0].glow, 2..3);
        assert!(plan.draws[1].glow.is_empty());
        assert_eq!((plan.stats.batches, plan.stats.color_draws, plan.stats.glow_draws), (4, 2, 1));
    }

    #[test]
    fn zero_alpha_colour_survives_while_glow_only_range_is_trimmed() {
        let mut p = [ParticleInstance {
            pos: [0.0, 0.0, 0.5],
            size: [0.2; 2],
            texture: 1,
            color: [255, 0, 0, 0],
            flags: crate::particle_flags::KEEP_ZERO_ALPHA,
            ..Default::default()
        }; 5];
        p[1].glow = 0.5;
        p[3].end_glow = 0.25;
        let mut plan = Plan::default();
        plan.prepare(&p, true, &Frustum::new(glam::Mat4::IDENTITY));
        assert_eq!(plan.draws.len(), 1);
        assert_eq!(plan.draws[0].color, 0..5);
        assert_eq!(plan.draws[0].glow, 1..4);
        plan.prepare(&p, false, &Frustum::new(glam::Mat4::IDENTITY));
        assert_eq!(plan.draws[0].color, 0..5);
        assert!(plan.draws[0].glow.is_empty());
        plan.prepare(&[], true, &Frustum::new(glam::Mat4::IDENTITY));
        assert!(plan.draws.is_empty());
        assert_eq!(plan.stats.color_instances, 0);
    }

    #[test]
    fn every_supported_wire_pair_reaches_its_actual_fixed_function_blend() {
        let mut indices = std::collections::HashSet::new();
        for src in FACTORS {
            for dst in FACTORS {
                assert!(indices.insert(blend_index(src, dst)));
            }
        }
        assert_eq!(indices.len(), BLEND_COUNT);
        assert_eq!(blend_index(7, 9), 0);
        assert_eq!(blend_index(6, 8), 0);
        let mask = blend_state(blend_index(1, 7) as usize);
        assert_eq!(mask.color.src_factor, wgpu::BlendFactor::Zero);
        assert_eq!(mask.color.dst_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(blend_state(blend_index(3, 4) as usize).color.src_factor, wgpu::BlendFactor::Src);
        assert_eq!(
            blend_state(blend_index(3, 4) as usize).color.dst_factor,
            wgpu::BlendFactor::OneMinusDst
        );
    }

    #[test]
    fn glow_batches_preserve_interleaved_depth_order_and_suppression() {
        let mut p = [ParticleInstance::default(); 5];
        p[0].texture = 2;
        p[0].glow = 0.5;
        p[1].texture = 2;
        p[2].texture = 3;
        p[2].end_glow = 0.25;
        p[3].texture = 2;
        p[4].texture = 2;
        p[4].blend = blend_index(1, 7);
        let b: Vec<_> = batches(&p, true).collect();
        assert_eq!(b.iter().map(|b| b.instances.clone()).collect::<Vec<_>>(), [0..2, 2..3, 3..4, 4..5]);
        assert_eq!(b.iter().map(|b| b.glow).collect::<Vec<_>>(), [true, true, false, false]);
        assert_eq!(batches(&p, false).map(|b| b.instances).collect::<Vec<_>>(), [0..4, 4..5]);
    }
}
