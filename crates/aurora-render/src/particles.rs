//! Scripted blend factors and ordered particle batches. Wire values and glow
//! suppression follow LLDrawPoolAlpha::renderAlpha (indra/newview/lldrawpoolalpha.cpp,
//! originally LGPL 2.1). Never regroup particles across their back-to-front order.

use crate::ParticleInstance;
use std::ops::Range;

const FACTORS: [u8; 8] = [7, 9, 0, 1, 2, 3, 4, 5];
pub(crate) const BLEND_COUNT: usize = 64;

/// The wire protocol excludes destination-alpha factors. Invalid external values
/// fall back independently to the ordinary alpha source/destination factors.
pub fn blend_index(source: u8, destination: u8) -> u32 {
    let src = FACTORS.iter().position(|&f| f == source).unwrap_or(0);
    let dst = FACTORS.iter().position(|&f| f == destination).unwrap_or(1);
    ((src * 8 + dst) ^ 1) as u32
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

#[cfg(test)]
mod tests {
    use super::*;

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
