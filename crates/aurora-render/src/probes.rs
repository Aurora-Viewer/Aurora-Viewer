//! Reflection probes, after LLReflectionMapManager / LLReflectionMap and
//! class3/deferred/reflectionProbeF.glsl of the Second Life / Firestorm viewer
//! (originally LGPL 2.1, Copyright (C) 2022-2024 Linden Research, Inc. and the Firestorm
//! project).
//!
//! The viewer decides which probes exist and which cube face to capture each
//! frame ([`crate::ProbeFrame`]); this module owns the GPU side:
//! - a cube map array (one cube per slot, slot 0 = the default sky probe),
//!   prefiltered for GGX roughness along its mips (radianceGenF);
//! - a scratch cube the faces are captured into, with its mip chain
//!   (reflectionmipF), which the radiance pass filters into the probe's slot;
//! - the probe list the scene shaders read (binding 9 of the frame group).

use bytemuck::{Pod, Zeroable};

use crate::renderer::{make_pipeline_simple, make_view};
use crate::types::ProbeGpu;

/// RenderReflectionProbeResolution.
pub const PROBE_RES: u32 = 128;
/// Mips 128 .. 2 (LL: log2(resolution)); the shaders use max lod 6.
pub const PROBE_MIPS: u32 = 7;
/// Probes the shaders can see per frame (the default probe included).
pub const MAX_PROBES: usize = 64;
pub const PROBE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Faces are captured at twice the probe resolution and downsampled (LL
/// super-samples x4 and blurs before its mip chain).
pub const CAPTURE_RES: u32 = PROBE_RES * 2;
const CAPTURE_MIPS: u32 = PROBE_MIPS + 1;
/// LL_IRRADIANCE_MAP_RESOLUTION.
pub const IRRADIANCE_RES: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ProbeHeader {
    count: u32,
    max_lod: f32,
    enabled: u32,
    _pad: u32,
}

pub(crate) struct Probes {
    pub slots: u32,
    /// Radiance render targets: [slot * 6 + face][mip].
    array_targets: Vec<Vec<wgpu::TextureView>>,
    /// Cube array view sampled by the scene shaders.
    pub array_view: wgpu::TextureView,
    /// Irradiance cubes (ambient of non classic skies), same slots.
    pub irradiance_view: wgpu::TextureView,
    /// Irradiance render targets: [slot * 6 + face].
    irradiance_targets: Vec<wgpu::TextureView>,
    scratch: wgpu::Texture,
    /// Render targets of the capture and mip passes: [face][mip].
    scratch_targets: Vec<Vec<wgpu::TextureView>>,
    pub depth: wgpu::TextureView,
    pub buffer: wgpu::Buffer,
    /// Trilinear sampler of the probe array (scene shaders, binding 11).
    pub sampler: wgpu::Sampler,
    mip_pipeline: wgpu::RenderPipeline,
    radiance_pipeline: wgpu::RenderPipeline,
    irradiance_pipeline: wgpu::RenderPipeline,
    /// Mip pass sources: all faces of scratch mip `i` (index i - 1 for target i).
    mip_bgs: Vec<wgpu::BindGroup>,
    radiance_bg: wgpu::BindGroup,
}

impl Probes {
    pub fn new(device: &wgpu::Device, slots: u32) -> Probes {
        let slots = slots.clamp(1, MAX_PROBES as u32);
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let mk_cubes = |label: &str, layers: u32, res: u32, mips: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: res,
                    height: res,
                    depth_or_array_layers: layers,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PROBE_FORMAT,
                usage,
                view_formats: &[],
            })
        };
        let array = mk_cubes("reflection probes", slots * 6, PROBE_RES, PROBE_MIPS);
        let array_targets = (0..slots * 6)
            .map(|layer| {
                (0..PROBE_MIPS)
                    .map(|mip| {
                        array.create_view(&wgpu::TextureViewDescriptor {
                            label: Some("probe radiance target"),
                            dimension: Some(wgpu::TextureViewDimension::D2),
                            base_mip_level: mip,
                            mip_level_count: Some(1),
                            base_array_layer: layer,
                            array_layer_count: Some(1),
                            ..Default::default()
                        })
                    })
                    .collect()
            })
            .collect();
        let irradiance = mk_cubes("reflection probe irradiance", slots * 6, IRRADIANCE_RES, 1);
        let irradiance_view = irradiance.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection probe irradiance"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let irradiance_targets = (0..slots * 6)
            .map(|layer| {
                irradiance.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("probe irradiance target"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let array_view = array.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection probes"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let scratch = mk_cubes("reflection probe capture", 6, CAPTURE_RES, CAPTURE_MIPS);
        let scratch_targets = (0..6)
            .map(|face| {
                (0..CAPTURE_MIPS)
                    .map(|mip| {
                        scratch.create_view(&wgpu::TextureViewDescriptor {
                            label: Some("probe capture face"),
                            dimension: Some(wgpu::TextureViewDimension::D2),
                            base_mip_level: mip,
                            mip_level_count: Some(1),
                            base_array_layer: face,
                            array_layer_count: Some(1),
                            ..Default::default()
                        })
                    })
                    .collect()
            })
            .collect();
        let depth = make_view(
            device,
            "probe capture depth",
            CAPTURE_RES,
            CAPTURE_RES,
            wgpu::TextureFormat::Depth32Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
        .1;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reflection probe list"),
            size: (std::mem::size_of::<ProbeHeader>() + MAX_PROBES * std::mem::size_of::<ProbeGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("probe filter"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let sampler_entry = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let tex_entry = |binding: u32, dim: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe mip"),
            entries: &[tex_entry(0, wgpu::TextureViewDimension::D2Array), sampler_entry],
        });
        let radiance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe radiance"),
            entries: &[tex_entry(1, wgpu::TextureViewDimension::Cube), sampler_entry],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("probe filter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/probe_filter.wgsl").into()),
        });
        let mip_pipeline = make_pipeline_simple(device, &module, &mip_layout, "probe mip", "vs_filter", "fs_mip", PROBE_FORMAT);
        let radiance_pipeline = make_pipeline_simple(
            device,
            &module,
            &radiance_layout,
            "probe radiance",
            "vs_filter",
            "fs_radiance",
            PROBE_FORMAT,
        );
        let irradiance_pipeline = make_pipeline_simple(
            device,
            &module,
            &radiance_layout,
            "probe irradiance",
            "vs_filter",
            "fs_irradiance",
            PROBE_FORMAT,
        );
        let mip_bgs = (0..CAPTURE_MIPS - 1)
            .map(|mip| {
                let v = scratch.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("probe mip source"),
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("probe mip"),
                    layout: &mip_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&v),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                })
            })
            .collect();
        let cube = scratch.create_view(&wgpu::TextureViewDescriptor {
            label: Some("probe capture cube"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let radiance_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("probe radiance"),
            layout: &radiance_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&cube),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Probes {
            slots,
            array_targets,
            array_view,
            irradiance_view,
            irradiance_targets,
            scratch,
            scratch_targets,
            depth,
            buffer,
            sampler,
            mip_pipeline,
            radiance_pipeline,
            irradiance_pipeline,
            mip_bgs,
            radiance_bg,
        }
    }

    /// Render target of a captured face (mip 0 of the scratch cube).
    pub fn capture_target(&self, face: u32) -> &wgpu::TextureView {
        &self.scratch_targets[(face % 6) as usize][0]
    }

    /// Upload the probe list (index 0 = default probe).
    pub fn upload(&self, queue: &wgpu::Queue, enabled: bool, probes: &[ProbeGpu]) {
        let n = probes.len().min(MAX_PROBES);
        let header = ProbeHeader {
            count: n as u32,
            max_lod: (PROBE_MIPS - 1) as f32,
            enabled: u32::from(enabled && n > 0),
            _pad: 0,
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&header));
        if n > 0 {
            queue.write_buffer(
                &self.buffer,
                std::mem::size_of::<ProbeHeader>() as u64,
                bytemuck::cast_slice(&probes[..n]),
            );
        }
    }

    /// After the six faces are captured: mip chain of the scratch cube
    /// (reflectionmipF), then the GGX prefiltered radiance into `slot`.
    pub fn filter(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, slot: u32) {
        let slot = slot.min(self.slots - 1);
        let pass = |encoder: &mut wgpu::CommandEncoder,
                    target: &wgpu::TextureView,
                    pipe: &wgpu::RenderPipeline,
                    bg: &wgpu::BindGroup,
                    inst: u32| {
            let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("probe filter"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(pipe);
            p.set_bind_group(0, bg, &[]);
            p.draw(0..3, inst..inst + 1);
        };
        for mip in 1..CAPTURE_MIPS {
            for face in 0..6u32 {
                pass(
                    encoder,
                    &self.scratch_targets[face as usize][mip as usize],
                    &self.mip_pipeline,
                    &self.mip_bgs[(mip - 1) as usize],
                    face,
                );
            }
        }
        for mip in 0..PROBE_MIPS {
            for face in 0..6u32 {
                let target = &self.array_targets[(slot * 6 + face) as usize][mip as usize];
                pass(encoder, target, &self.radiance_pipeline, &self.radiance_bg, mip * 6 + face);
            }
        }
        for face in 0..6u32 {
            let target = &self.irradiance_targets[(slot * 6 + face) as usize];
            pass(encoder, target, &self.irradiance_pipeline, &self.radiance_bg, face);
        }
        let _ = (&self.scratch, device);
    }
}

/// View matrix of cube face `face` seen from `origin` (Vulkan face order and
/// orientation, z-up world): x right = increasing s, y up = decreasing t.
pub fn face_view(origin: glam::Vec3, face: u32) -> glam::Mat4 {
    use glam::Vec3;
    // (forward, s axis, t axis) per face, see face_dir in probe_filter.wgsl
    let (f, r, d) = match face % 6 {
        0 => (Vec3::X, -Vec3::Z, -Vec3::Y),
        1 => (-Vec3::X, Vec3::Z, -Vec3::Y),
        2 => (Vec3::Y, Vec3::X, Vec3::Z),
        3 => (-Vec3::Y, Vec3::X, -Vec3::Z),
        4 => (Vec3::Z, Vec3::X, -Vec3::Y),
        _ => (-Vec3::Z, -Vec3::X, -Vec3::Y),
    };
    let u = -d;
    let b = -f;
    glam::Mat4::from_cols(
        glam::Vec4::new(r.x, u.x, b.x, 0.0),
        glam::Vec4::new(r.y, u.y, b.y, 0.0),
        glam::Vec4::new(r.z, u.z, b.z, 0.0),
        glam::Vec4::new(-r.dot(origin), -u.dot(origin), -b.dot(origin), 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4Swizzles};

    /// A point straight ahead of each face lands at the image center, the
    /// s axis to the right and the t axis downward (Vulkan convention).
    #[test]
    fn face_views_follow_the_cube_convention() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_2, 1.0, 0.1);
        let o = Vec3::new(10.0, 20.0, 30.0);
        // face -> (major axis, s direction, t direction) as in face_dir
        let faces = [
            (Vec3::X, -Vec3::Z, -Vec3::Y),
            (-Vec3::X, Vec3::Z, -Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::Z),
            (-Vec3::Y, Vec3::X, -Vec3::Z),
            (Vec3::Z, Vec3::X, -Vec3::Y),
            (-Vec3::Z, -Vec3::X, -Vec3::Y),
        ];
        for (i, (f, s, t)) in faces.into_iter().enumerate() {
            let vp = proj * face_view(o, i as u32);
            let ndc = |p: Vec3| {
                let c = vp * p.extend(1.0);
                c.xy() / c.w
            };
            let center = ndc(o + f);
            assert!(center.length() < 1e-4, "face {i} center {center:?}");
            let right = ndc(o + f + s * 0.5);
            assert!((right.x - 0.5).abs() < 1e-4 && right.y.abs() < 1e-4, "face {i} s {right:?}");
            let down = ndc(o + f + t * 0.5);
            assert!((down.y + 0.5).abs() < 1e-4 && down.x.abs() < 1e-4, "face {i} t {down:?}");
        }
    }
}
