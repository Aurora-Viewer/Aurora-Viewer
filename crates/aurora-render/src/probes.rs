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

use crate::renderer::make_view;
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
/// `@workgroup_size` of the filter shader, in x and y.
const WORKGROUP: u32 = 8;

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
    /// Cube array view sampled by the scene shaders.
    pub array_view: wgpu::TextureView,
    /// Irradiance cubes (ambient of non classic skies), same slots.
    pub irradiance_view: wgpu::TextureView,
    /// Faces of the scratch cube the capture passes render into (mip 0).
    capture_targets: Vec<wgpu::TextureView>,
    pub depth: wgpu::TextureView,
    pub buffer: wgpu::Buffer,
    /// Trilinear sampler of the probe array (scene shaders, binding 11).
    pub sampler: wgpu::Sampler,
    mip_pipeline: wgpu::ComputePipeline,
    radiance_pipeline: wgpu::ComputePipeline,
    irradiance_pipeline: wgpu::ComputePipeline,
    /// Scratch mip `i` into scratch mip `i + 1`.
    mip_bgs: Vec<wgpu::BindGroup>,
    /// The scratch cube into mip `m` of a slot's radiance: [slot][m].
    radiance_bgs: Vec<Vec<wgpu::BindGroup>>,
    /// The scratch cube into a slot's irradiance: [slot].
    irradiance_bgs: Vec<wgpu::BindGroup>,
    /// Read back by the GPU test.
    #[cfg(test)]
    array: wgpu::Texture,
    #[cfg(test)]
    irradiance: wgpu::Texture,
}

impl Probes {
    pub fn new(device: &wgpu::Device, slots: u32) -> Probes {
        let slots = slots.clamp(1, MAX_PROBES as u32);
        let mk_cubes = |label: &str, layers: u32, res: u32, mips: u32, usage: wgpu::TextureUsages| {
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
        // sampled by the scene, written by the filtering
        let mut filtered = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
        if cfg!(test) {
            filtered |= wgpu::TextureUsages::COPY_SRC;
        }
        let array = mk_cubes("reflection probes", slots * 6, PROBE_RES, PROBE_MIPS, filtered);
        let irradiance = mk_cubes("reflection probe irradiance", slots * 6, IRRADIANCE_RES, 1, filtered);
        let irradiance_view = irradiance.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection probe irradiance"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let array_view = array.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection probes"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let scratch = mk_cubes(
            "reflection probe capture",
            6,
            CAPTURE_RES,
            CAPTURE_MIPS,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let capture_targets = (0..6)
            .map(|face| {
                scratch.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("probe capture face"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: 0,
                    mip_level_count: Some(1),
                    base_array_layer: face,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
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
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let source_entry = |binding: u32, dim: wgpu::TextureViewDimension| {
            entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: dim,
                    multisampled: false,
                },
            )
        };
        let sampler_entry = entry(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering));
        // the six faces of the mip being written
        let target_entry = entry(
            3,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: PROBE_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2Array,
            },
        );
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe mip"),
            entries: &[source_entry(0, wgpu::TextureViewDimension::D2Array), sampler_entry, target_entry],
        });
        let radiance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe radiance"),
            entries: &[source_entry(1, wgpu::TextureViewDimension::Cube), sampler_entry, target_entry],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("probe filter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/probe_filter.wgsl").into()),
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let mip_pipeline = pipeline(&mip_layout, "cs_mip");
        let radiance_pipeline = pipeline(&radiance_layout, "cs_radiance");
        let irradiance_pipeline = pipeline(&radiance_layout, "cs_irradiance");
        // the six faces of one mip of a cube, `first` being its first layer
        let faces = |texture: &wgpu::Texture, label: &str, mip: u32, first: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: mip,
                mip_level_count: Some(1),
                base_array_layer: first,
                array_layer_count: Some(6),
                ..Default::default()
            })
        };
        let group = |layout: &wgpu::BindGroupLayout, label: &str, source: (u32, &wgpu::TextureView), target: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: source.0,
                        resource: wgpu::BindingResource::TextureView(source.1),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(target),
                    },
                ],
            })
        };
        let mip_bgs = (1..CAPTURE_MIPS)
            .map(|mip| {
                let source = faces(&scratch, "probe mip source", mip - 1, 0);
                let target = faces(&scratch, "probe mip target", mip, 0);
                group(&mip_layout, "probe mip", (0, &source), &target)
            })
            .collect();
        let cube = scratch.create_view(&wgpu::TextureViewDescriptor {
            label: Some("probe capture cube"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let radiance_bgs = (0..slots)
            .map(|slot| {
                (0..PROBE_MIPS)
                    .map(|mip| {
                        let target = faces(&array, "probe radiance target", mip, slot * 6);
                        group(&radiance_layout, "probe radiance", (1, &cube), &target)
                    })
                    .collect()
            })
            .collect();
        let irradiance_bgs = (0..slots)
            .map(|slot| {
                let target = faces(&irradiance, "probe irradiance target", 0, slot * 6);
                group(&radiance_layout, "probe irradiance", (1, &cube), &target)
            })
            .collect();
        Probes {
            slots,
            array_view,
            irradiance_view,
            capture_targets,
            depth,
            buffer,
            sampler,
            mip_pipeline,
            radiance_pipeline,
            irradiance_pipeline,
            mip_bgs,
            radiance_bgs,
            irradiance_bgs,
            #[cfg(test)]
            array,
            #[cfg(test)]
            irradiance,
        }
    }

    /// Render target of a captured face (mip 0 of the scratch cube).
    pub fn capture_target(&self, face: u32) -> &wgpu::TextureView {
        &self.capture_targets[(face % 6) as usize]
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
    /// (reflectionmipF), then the GGX prefiltered radiance and the
    /// irradiance into `slot`.
    ///
    /// One compute pass, a dispatch per mip for its six faces. As render
    /// passes, one per face and mip, there were 90 of them, and wgpu pays
    /// for each when the encoder is finished: the frame of a sixth face
    /// spent 1 ms there instead of 0.25.
    pub fn filter(&self, encoder: &mut wgpu::CommandEncoder, slot: u32) {
        let slot = slot.min(self.slots - 1) as usize;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("probe filter"),
            timestamp_writes: None,
        });
        let dispatch = |pass: &mut wgpu::ComputePass, bg: &wgpu::BindGroup, res: u32| {
            let groups = res.div_ceil(WORKGROUP);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(groups, groups, 6);
        };
        pass.set_pipeline(&self.mip_pipeline);
        for (mip, bg) in (1..CAPTURE_MIPS).zip(&self.mip_bgs) {
            dispatch(&mut pass, bg, CAPTURE_RES >> mip);
        }
        pass.set_pipeline(&self.radiance_pipeline);
        for (mip, bg) in (0..PROBE_MIPS).zip(&self.radiance_bgs[slot]) {
            dispatch(&mut pass, bg, PROBE_RES >> mip);
        }
        pass.set_pipeline(&self.irradiance_pipeline);
        dispatch(&mut pass, &self.irradiance_bgs[slot], IRRADIANCE_RES);
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

    /// What the filter shader repeats of the sizes above.
    #[test]
    fn shader_constants_match() {
        let src = include_str!("shaders/probe_filter.wgsl");
        assert!(src.contains(&format!("const PROBE_RES: u32 = {PROBE_RES}u;")));
        assert!(src.contains(&format!("const MAX_PROBE_LOD: f32 = {}.0;", PROBE_MIPS - 1)));
        // CAPTURE_LOD_OFFSET: mip 1 of the capture is mip 0 of the probe
        assert_eq!(CAPTURE_RES, PROBE_RES * 2);
        assert_eq!(src.matches(&format!("@workgroup_size({WORKGROUP}, {WORKGROUP}, 1)")).count(), 3);
    }

    /// Value of a half float.
    fn half(h: u16) -> f32 {
        let (e, m) = ((h >> 10) & 0x1f, (h & 0x3ff) as f32);
        let v = match e {
            0 => m * 2f32.powi(-24),
            31 => f32::INFINITY,
            _ => (1.0 + m / 1024.0) * 2f32.powi(e as i32 - 15),
        };
        if h & 0x8000 != 0 { -v } else { v }
    }

    /// One layer of one mip of an Rgba16Float texture, row by row.
    fn read(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, mip: u32, layer: u32) -> Vec<[f32; 4]> {
        let res = (texture.width() >> mip).max(1);
        let row = (res * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * res) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: mip,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: res,
                height: res,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let data = buffer.get_mapped_range(..).expect("mapped");
        let mut out = Vec::with_capacity((res * res) as usize);
        for y in 0..res {
            let line = &data[(y * row) as usize..(y * row + res * 8) as usize];
            for px in line.as_chunks::<8>().0 {
                let c = |i: usize| half(u16::from_le_bytes([px[i * 2], px[i * 2 + 1]]));
                out.push([c(0), c(1), c(2), c(3)]);
            }
        }
        out
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return None;
        };
        Some(
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .expect("GPU device"),
        )
    }

    /// The six faces of the capture cube, each of one color.
    fn capture(probes: &Probes, encoder: &mut wgpu::CommandEncoder, colors: [[f64; 3]; 6]) {
        for (face, c) in colors.into_iter().enumerate() {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: probes.capture_target(face as u32),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: c[0],
                            g: c[1],
                            b: c[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
    }

    const FACES: [[f64; 3]; 6] = [
        [4.0, 0.25, 0.0],
        [0.0, 1.0, 0.5],
        [0.125, 0.0, 2.0],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.5, 3.0, 0.75],
    ];

    /// The filtering writes every texel of every mip of its slot, each face
    /// in its layer, and nothing else. Needs a GPU:
    /// `cargo test -p aurora-render probe_filter -- --ignored`.
    #[test]
    #[ignore]
    fn gpu_probe_filter_fills_its_slot() {
        let Some((device, queue)) = device() else {
            return;
        };
        let probes = Probes::new(&device, 8);
        let close = |a: [f32; 4], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k] as f32).abs() <= 2e-3 * (1.0 + b[k] as f32)) && a[3] == 1.0;
        // a cube of one color stays that color at every roughness
        let plain = [0.5, 2.0, 0.125];
        let mut encoder = device.create_command_encoder(&Default::default());
        capture(&probes, &mut encoder, [plain; 6]);
        probes.filter(&mut encoder, 3);
        queue.submit([encoder.finish()]);
        for face in 0..6 {
            for mip in 0..PROBE_MIPS {
                let texels = read(&device, &queue, &probes.array, mip, 3 * 6 + face);
                assert_eq!(texels.len(), ((PROBE_RES >> mip) * (PROBE_RES >> mip)) as usize);
                let off = texels.iter().position(|t| !close(*t, plain));
                assert_eq!(off, None, "radiance face {face} mip {mip}: {:?}", off.map(|i| texels[i]));
            }
            let texels = read(&device, &queue, &probes.irradiance, 0, 3 * 6 + face);
            let off = texels.iter().position(|t| !close(*t, plain));
            assert_eq!(off, None, "irradiance face {face}: {:?}", off.map(|i| texels[i]));
            // the slots around it are left alone
            for slot in [2, 4] {
                assert!(
                    read(&device, &queue, &probes.array, 0, slot * 6 + face)
                        .iter()
                        .all(|t| *t == [0.0; 4])
                );
                assert!(
                    read(&device, &queue, &probes.irradiance, 0, slot * 6 + face)
                        .iter()
                        .all(|t| *t == [0.0; 4])
                );
            }
        }
        // mirror-like mip: each face shows its own color, the rough ones mix them
        let mut encoder = device.create_command_encoder(&Default::default());
        capture(&probes, &mut encoder, FACES);
        probes.filter(&mut encoder, 1);
        queue.submit([encoder.finish()]);
        for (face, color) in FACES.into_iter().enumerate() {
            let sharp = read(&device, &queue, &probes.array, 0, 6 + face as u32);
            let center = sharp[(PROBE_RES / 2 * PROBE_RES + PROBE_RES / 2) as usize];
            assert!(close(center, color), "face {face}: {center:?}");
            let rough = read(&device, &queue, &probes.array, PROBE_MIPS - 1, 6 + face as u32)[0];
            assert!(!close(rough, color), "face {face} at full roughness: {rough:?}");
        }
    }
}
