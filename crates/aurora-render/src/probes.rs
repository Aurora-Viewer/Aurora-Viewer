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
use glam::{Mat4, Vec3};

use crate::light_tiles::{self, LightSets};
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
/// The irradiance capture of a sky with a probe ambiance (its own pass, as
/// LLReflectionMapManager's irradiance pass). irradianceGenF only reads
/// the smallest mip of a capture (its level is clamped at 2 x 2), but each
/// mip stored the way the Legacy Viewer stores them darkens and yellows it a
/// little (LEGACY_FORMAT of probe_filter.wgsl): drawn at 128 x 128, its
/// chain has as many steps down to 2 x 2 as the Legacy Viewer's (a capture
/// at 512 filtered into mips 128 .. 2).
pub const IRR_CAPTURE_RES: u32 = CAPTURE_RES >> 1;
const IRR_CAPTURE_MIPS: u32 = CAPTURE_MIPS - 1;
/// `@workgroup_size` of the filter shader, in x and y.
const WORKGROUP: u32 = 8;

/// How the filtering stores each stage of a probe (probe_filter.wgsl).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Store {
    /// As computed (skies without a probe ambiance).
    Plain = 0,
    /// GL_R11F_G11F_B10F, as the Legacy Viewer (LEGACY_FORMAT).
    Legacy = 1,
    /// GL_RGB8, the Legacy Viewer without HDR (LDR_FORMAT).
    Ldr = 2,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ProbeHeader {
    count: u32,
    max_lod: f32,
    enabled: u32,
    _pad: u32,
    /// The box probes of the list: bit `i` of word `i / 32` (`box_mask`).
    boxes: [u32; 4],
    /// The probes of the main view's fragments ([`tiles`]).
    sets: LightSets,
}

/// How a fragment walks the probes (AURORA_PROBE_WALK; the image is the
/// same).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeWalk {
    /// The probes of its screen tile and depth ([`tiles`]), once for both
    /// its lookups (the default; `sample_probes_pair` in common.wgsl).
    Tiles,
    /// The whole list, once for both its lookups (`list`).
    List,
    /// The whole list twice per lookup, as at first (`0`).
    PerLookup,
    /// [`ProbeWalk::Tiles`] or [`ProbeWalk::List`], each frame choosing
    /// with [`crate::FrameParams::probe_walk_list`] (`alt`).
    Alternate,
    /// [`ProbeWalk::Tiles`], and the shaders paint magenta any fragment
    /// that a probe of the list covers without being in its sets
    /// (`check`): none, ever.
    Check,
}

impl ProbeWalk {
    pub fn from_env() -> ProbeWalk {
        static MODE: std::sync::OnceLock<ProbeWalk> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| match std::env::var("AURORA_PROBE_WALK").as_deref().map(str::trim) {
            Ok("0") => ProbeWalk::PerLookup,
            Ok("list") => ProbeWalk::List,
            Ok("alt") => ProbeWalk::Alternate,
            Ok("check") => ProbeWalk::Check,
            _ => ProbeWalk::Tiles,
        })
    }

    /// Whether the fragments of a frame find their probes by screen tile
    /// and depth; `list`: what [`crate::FrameParams::probe_walk_list`] asks
    /// of this frame.
    pub fn tiles(self, list: bool) -> bool {
        match self {
            ProbeWalk::Tiles | ProbeWalk::Check => true,
            ProbeWalk::List | ProbeWalk::PerLookup => false,
            ProbeWalk::Alternate => !list,
        }
    }

    /// The walk as the shaders read it (`frame.light_mode.w`,
    /// `PROBE_WALK_*` in common.wgsl). The views other than the main one
    /// keep 0, the whole list. `check`: the sets are checked wherever they
    /// are used (Preferences › Debug).
    pub fn shader_mode(self, list: bool, check: bool) -> u32 {
        match self {
            ProbeWalk::PerLookup => 1,
            ProbeWalk::Check => 3,
            _ if self.tiles(list) => 2 + u32::from(check),
            _ => 0,
        }
    }
}

// (the sets of the lights hold as many members as there are probes)
const _: () = assert!(MAX_PROBES == light_tiles::LIGHTS);

/// A sphere around what a probe covers: its influence sphere or, for a box
/// probe, the sphere through the corners of its box.
pub fn extent(p: &ProbeGpu) -> (Vec3, f32) {
    if p.params[3] >= 0.0 {
        return (Vec3::from_slice(&p.center_radius[..3]), p.center_radius[3]);
    }
    let to_world = Mat4::from_cols_array_2d(&p.box_inv).inverse();
    let centre = to_world.transform_point3(Vec3::ZERO);
    let mut radius = 0.0f32;
    for i in 0..8 {
        let side = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
        let corner = to_world.transform_point3(Vec3::new(side(1), side(2), side(4)));
        // (NaN, the corner of a box that cannot be turned back: everywhere)
        radius = if corner.is_finite() {
            radius.max(corner.distance(centre))
        } else {
            f32::INFINITY
        };
    }
    if centre.is_finite() {
        (centre, radius)
    } else {
        (Vec3::ZERO, f32::INFINITY)
    }
}

/// The probes of the main view's fragments by screen tile and depth, as
/// the lights' (light_tiles.rs): `probes` in the order of the frame's list,
/// the screen `size` in pixels, the camera at `eye` (the position the
/// shaders measure depths from).
///
/// A fragment used to walk the whole list (32 probes by default, up to 64)
/// to find the few that cover it; the walk, not the samples of those few,
/// was what the probes cost. Same image: a probe left out covers none of
/// the fragments drawn there, and those that stay are added in the same
/// order. The default probe, first of the list, is sampled whatever its
/// sphere says: it is in every set.
pub fn tiles(view: Mat4, proj: Mat4, eye: Vec3, size: [u32; 2], probes: &[ProbeGpu]) -> LightSets {
    let mut sets = light_tiles::bin_spheres(view, proj, eye, size, probes.iter().map(extent));
    sets.hold_everywhere(0);
    sets
}

/// The box probes of a list (kind below zero), which keep the automatic
/// probes out of what they enclose: the shaders test these first. The
/// default probe, first of the list, is never one.
pub fn box_mask(probes: &[ProbeGpu]) -> [u32; 4] {
    let mut mask = [0u32; 4];
    for (i, p) in probes.iter().enumerate().take(MAX_PROBES).skip(1) {
        if p.params[3] < 0.0 {
            mask[i / 32] |= 1 << (i % 32);
        }
    }
    mask
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
    /// Faces of the irradiance capture's cube (mip 0), and its depth.
    irr_capture_targets: Vec<wgpu::TextureView>,
    pub irr_depth: wgpu::TextureView,
    pub buffer: wgpu::Buffer,
    /// Trilinear sampler of the probe array (scene shaders, binding 11).
    pub sampler: wgpu::Sampler,
    /// [plain, as the Legacy Viewer stores its probes, in 8 bits without
    /// HDR] (`Store`: LEGACY_FORMAT, LDR_FORMAT of probe_filter.wgsl)
    mip_pipeline: [wgpu::ComputePipeline; 3],
    radiance_pipeline: [wgpu::ComputePipeline; 3],
    /// [plain, in 8 bits]
    irradiance_pipeline: [wgpu::ComputePipeline; 2],
    /// Scratch mip `i` into scratch mip `i + 1`.
    mip_bgs: Vec<wgpu::BindGroup>,
    /// The scratch cube into mip `m` of a slot's radiance: [slot][m].
    radiance_bgs: Vec<Vec<wgpu::BindGroup>>,
    /// The scratch cube into a slot's irradiance: [slot].
    irradiance_bgs: Vec<wgpu::BindGroup>,
    /// Irradiance capture mip `i` into mip `i + 1`.
    irr_mip_bgs: Vec<wgpu::BindGroup>,
    /// [stored as the Legacy Viewer does, in 8 bits]
    irr_source_pipeline: [wgpu::ComputePipeline; 2],
    /// The irradiance capture into a slot's irradiance: [slot].
    irr_source_bgs: Vec<wgpu::BindGroup>,
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
        let irr_scratch = mk_cubes(
            "reflection probe irradiance capture",
            6,
            IRR_CAPTURE_RES,
            IRR_CAPTURE_MIPS,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let face_targets = |texture: &wgpu::Texture| -> Vec<wgpu::TextureView> {
            (0..6)
                .map(|face| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        label: Some("probe capture face"),
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_mip_level: 0,
                        mip_level_count: Some(1),
                        base_array_layer: face,
                        array_layer_count: Some(1),
                        ..Default::default()
                    })
                })
                .collect()
        };
        let capture_targets = face_targets(&scratch);
        let irr_capture_targets = face_targets(&irr_scratch);
        let depth_of = |label: &str, res: u32| {
            make_view(
                device,
                label,
                res,
                res,
                wgpu::TextureFormat::Depth32Float,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .1
        };
        let depth = depth_of("probe capture depth", CAPTURE_RES);
        let irr_depth = depth_of("probe irradiance capture depth", IRR_CAPTURE_RES);
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
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str, store: Store| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            let flag = |on: bool| if on { 1.0 } else { 0.0 };
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[
                        ("LEGACY_FORMAT", flag(store == Store::Legacy)),
                        ("LDR_FORMAT", flag(store == Store::Ldr)),
                    ],
                    ..Default::default()
                },
                cache: None,
            })
        };
        let all = [Store::Plain, Store::Legacy, Store::Ldr];
        let mip_pipeline = all.map(|store| pipeline(&mip_layout, "cs_mip", store));
        let radiance_pipeline = all.map(|store| pipeline(&radiance_layout, "cs_radiance", store));
        let irradiance_pipeline = [Store::Plain, Store::Ldr].map(|store| pipeline(&radiance_layout, "cs_irradiance", store));
        let irr_source_pipeline = [Store::Legacy, Store::Ldr].map(|store| pipeline(&radiance_layout, "cs_irradiance_capture", store));
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
        let mip_chain = |texture: &wgpu::Texture, mips: u32| -> Vec<wgpu::BindGroup> {
            (1..mips)
                .map(|mip| {
                    let source = faces(texture, "probe mip source", mip - 1, 0);
                    let target = faces(texture, "probe mip target", mip, 0);
                    group(&mip_layout, "probe mip", (0, &source), &target)
                })
                .collect()
        };
        let mip_bgs = mip_chain(&scratch, CAPTURE_MIPS);
        let irr_mip_bgs = mip_chain(&irr_scratch, IRR_CAPTURE_MIPS);
        let cube_of = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("probe capture cube"),
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            })
        };
        let cube = cube_of(&scratch);
        let irr_cube = cube_of(&irr_scratch);
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
        let irradiance_from = |source: &wgpu::TextureView| -> Vec<wgpu::BindGroup> {
            (0..slots)
                .map(|slot| {
                    let target = faces(&irradiance, "probe irradiance target", 0, slot * 6);
                    group(&radiance_layout, "probe irradiance", (1, source), &target)
                })
                .collect()
        };
        let irradiance_bgs = irradiance_from(&cube);
        let irr_source_bgs = irradiance_from(&irr_cube);
        Probes {
            slots,
            array_view,
            irradiance_view,
            capture_targets,
            depth,
            irr_capture_targets,
            irr_depth,
            buffer,
            sampler,
            mip_pipeline,
            radiance_pipeline,
            irradiance_pipeline,
            mip_bgs,
            radiance_bgs,
            irradiance_bgs,
            irr_mip_bgs,
            irr_source_pipeline,
            irr_source_bgs,
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

    /// Render target of a face of the irradiance capture.
    pub fn irr_capture_target(&self, face: u32) -> &wgpu::TextureView {
        &self.irr_capture_targets[(face % 6) as usize]
    }

    /// Upload the probe list (index 0 = default probe) and the sets the
    /// main view's fragments find their probes in ([`tiles`]; only read
    /// when the frame says so, `ProbeWalk::shader_mode`).
    pub fn upload(&self, queue: &wgpu::Queue, enabled: bool, probes: &[ProbeGpu], sets: &LightSets) {
        let n = probes.len().min(MAX_PROBES);
        let header = ProbeHeader {
            count: n as u32,
            max_lod: (PROBE_MIPS - 1) as f32,
            enabled: u32::from(enabled && n > 0),
            _pad: 0,
            boxes: box_mask(&probes[..n]),
            sets: *sets,
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
    /// irradiance into `slot`; with `irr_capture` (a sky with a probe
    /// ambiance, the six faces of the irradiance capture drawn too), the
    /// irradiance comes from the irradiance capture and every stage is
    /// stored as the Legacy Viewer stores its probes. Without `hdr` every
    /// stage is stored in 8 bits, whatever the sky.
    ///
    /// One compute pass, a dispatch per mip for its six faces. As render
    /// passes, one per face and mip, there were 90 of them, and wgpu pays
    /// for each when the encoder is finished: the frame of a sixth face
    /// spent 1 ms there instead of 0.25.
    pub fn filter(&self, encoder: &mut wgpu::CommandEncoder, slot: u32, irr_capture: bool, hdr: bool) {
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
        let store = match (hdr, irr_capture) {
            (false, _) => Store::Ldr,
            (true, true) => Store::Legacy,
            (true, false) => Store::Plain,
        };
        let ldr = usize::from(store == Store::Ldr);
        pass.set_pipeline(&self.mip_pipeline[store as usize]);
        for (mip, bg) in (1..CAPTURE_MIPS).zip(&self.mip_bgs) {
            dispatch(&mut pass, bg, CAPTURE_RES >> mip);
        }
        pass.set_pipeline(&self.radiance_pipeline[store as usize]);
        for (mip, bg) in (0..PROBE_MIPS).zip(&self.radiance_bgs[slot]) {
            dispatch(&mut pass, bg, PROBE_RES >> mip);
        }
        if irr_capture {
            pass.set_pipeline(&self.mip_pipeline[store as usize]);
            for (mip, bg) in (1..IRR_CAPTURE_MIPS).zip(&self.irr_mip_bgs) {
                dispatch(&mut pass, bg, IRR_CAPTURE_RES >> mip);
            }
            pass.set_pipeline(&self.irr_source_pipeline[ldr]);
            dispatch(&mut pass, &self.irr_source_bgs[slot], IRRADIANCE_RES);
        } else {
            pass.set_pipeline(&self.irradiance_pipeline[ldr]);
            dispatch(&mut pass, &self.irradiance_bgs[slot], IRRADIANCE_RES);
        }
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

    /// The box probes of a list, as the scene shaders look for them: bit
    /// by bit in the header, which the shader's structure mirrors.
    #[test]
    fn box_probes_are_found_through_their_mask() {
        let probe = |kind: f32| ProbeGpu {
            center_radius: [0.0; 4],
            params: [1.0, 1.0, 1.0, kind],
            slot: [0; 4],
            box_inv: [[0.0; 4]; 4],
        };
        // default, automatic, box, manual sphere, box
        let mut list = vec![probe(0.0), probe(0.0), probe(-1.0), probe(1.0), probe(-1.0)];
        assert_eq!(box_mask(&list), [0b10100, 0, 0, 0]);
        // (the default probe is never one, whatever it says)
        list[0] = probe(-1.0);
        assert_eq!(box_mask(&list), [0b10100, 0, 0, 0]);
        list.resize(40, probe(0.0));
        list[39] = probe(-1.0);
        list.resize(MAX_PROBES + 6, probe(-1.0));
        let mask = box_mask(&list);
        assert_eq!(mask[1] & 0xff, 0b1000_0000, "probe 39, in the second word");
        assert_eq!(mask[1] >> 8, 0x00ff_ffff, "then every probe up to the last the shaders see");
        assert_eq!([mask[2], mask[3]], [0, 0]);
        assert_eq!(std::mem::size_of::<ProbeHeader>(), 32 + std::mem::size_of::<LightSets>());
        let src = include_str!("shaders/common.wgsl");
        let set = &src[src.find("struct ProbeSet {").expect("ProbeSet")..];
        // the header, the mask, the sets (laid out as LightSets), the probes
        let fields = [
            "_pad: u32,",
            "boxes: vec4<u32>,",
            "tile_grid: vec4<f32>,",
            "tile_forward: vec4<f32>,",
            "tile_mode: vec4<u32>,",
            "tile_columns: array<vec4<u32>, 16>,",
            "tile_rows: array<vec4<u32>, 16>,",
            "tile_slices: array<vec4<u32>, 16>,",
            "probes: array<Probe, 64>,",
        ];
        let at: Vec<usize> = fields.iter().map(|f| set.find(f).unwrap_or_else(|| panic!("{f}"))).collect();
        assert!(at.is_sorted(), "{at:?}");
        assert_eq!(light_tiles::TILES, 32);
        assert!(src.contains("probe_set.boxes.xy & held"));
        // nearer than this the shaders walk the whole list: the sets hold
        // what is further than the plane the extents are cut by
        assert!(src.contains("const PROBE_TILES_NEAREST: f32 = 0.01;"));
        const { assert!(light_tiles::NEAREST < 0.01) };
    }

    /// The walks as the shaders name them.
    #[test]
    fn walks_match_the_shaders() {
        let src = include_str!("shaders/common.wgsl");
        for (walk, list, name) in [
            (ProbeWalk::List, false, "LIST"),
            (ProbeWalk::Alternate, true, "LIST"),
            (ProbeWalk::PerLookup, false, "PER_LOOKUP"),
            (ProbeWalk::Tiles, false, "TILES"),
            (ProbeWalk::Alternate, false, "TILES"),
            (ProbeWalk::Check, false, "CHECK"),
        ] {
            let line = format!("const PROBE_WALK_{name}: u32 = {}u;", walk.shader_mode(list, false));
            assert!(src.contains(&line), "{line}");
            assert_eq!(walk.tiles(list), walk.shader_mode(list, false) >= 2);
        }
        // the check asked from the interface: where the sets are used, and
        // only there (the whole list has nothing to check)
        let check = |walk: ProbeWalk, list| walk.shader_mode(list, true);
        assert_eq!(check(ProbeWalk::Tiles, false), ProbeWalk::Check.shader_mode(false, false));
        assert_eq!(check(ProbeWalk::Alternate, false), 3);
        assert_eq!(
            (
                check(ProbeWalk::List, false),
                check(ProbeWalk::Alternate, true),
                check(ProbeWalk::PerLookup, false)
            ),
            (0, 0, 1)
        );
    }

    fn probe(center: Vec3, radius: f32, kind: f32, box_inv: Mat4) -> ProbeGpu {
        ProbeGpu {
            center_radius: center.extend(radius).to_array(),
            params: [1.0, 1.0, 1.0, kind],
            slot: [0; 4],
            box_inv: box_inv.to_cols_array_2d(),
        }
    }

    /// Whether a probe covers `p`, as `sample_probes_pair` decides it.
    fn covers(i: usize, probe: &ProbeGpu, p: Vec3) -> bool {
        if i == 0 {
            return true;
        }
        if probe.params[3] < 0.0 {
            let lp = Mat4::from_cols_array_2d(&probe.box_inv) * p.extend(1.0);
            return lp.truncate().abs().max_element() <= 1.0;
        }
        let delta = p - Vec3::from_slice(&probe.center_radius[..3]);
        delta.dot(delta) <= probe.center_radius[3] * probe.center_radius[3]
    }

    /// What the sets must never lose: a fragment a probe covers finds the
    /// probe. Checked along the view rays through the centre of every
    /// eighth pixel, at depths from the near plane to beyond the probes,
    /// for sphere and box probes all around the camera, from a camera high
    /// above a region too.
    #[test]
    fn a_fragment_finds_every_probe_that_covers_it() {
        const SIZE: [u32; 2] = [1280, 720];
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, SIZE[0] as f32 / SIZE[1] as f32, 0.1);
        for (mut seed, eye) in [(4242u32, Vec3::ZERO), (99, Vec3::new(200.0, 131.0, 3050.0))] {
            let mut rnd = move || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / (1 << 24) as f32
            };
            // the default probe, whose sphere is far from the view here
            let mut probes = vec![probe(eye + Vec3::new(-9000.0, 0.0, 0.0), 10.0, 0.0, Mat4::ZERO)];
            for i in 1..MAX_PROBES {
                let c = eye + Vec3::new(rnd() * 90.0 - 20.0, rnd() * 80.0 - 40.0, rnd() * 30.0 - 15.0);
                if i % 3 == 0 {
                    // a box of any size, turned any way
                    let turn = glam::Quat::from_euler(glam::EulerRot::ZYX, rnd() * 6.0, rnd() * 6.0, rnd() * 6.0);
                    let half = Vec3::new(1.0 + rnd() * 14.0, 1.0 + rnd() * 9.0, 0.5 + rnd() * 6.0);
                    let to_world = Mat4::from_scale_rotation_translation(half, turn, c);
                    probes.push(probe(c + Vec3::splat(0.3), 0.0, -1.0, to_world.inverse()));
                } else {
                    probes.push(probe(c, 2.0 + rnd() * 14.0, (i % 2) as f32, Mat4::ZERO));
                }
            }
            let view = glam::camera::rh::view::look_at_mat4(eye, eye + Vec3::X, Vec3::Z);
            let sets = tiles(view, proj, eye, SIZE, &probes);
            let forward = Vec3::from_slice(&sets.forward[..3]);
            let inv = (proj * view).inverse();
            let (mut covered, mut walked, mut fragments) = (0u32, 0u32, 0u32);
            for y in (0..SIZE[1]).step_by(8) {
                for x in (0..SIZE[0]).step_by(8) {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let ndc = Vec3::new(fx / SIZE[0] as f32 * 2.0 - 1.0, 1.0 - fy / SIZE[1] as f32 * 2.0, 0.5);
                    let dir = (inv.project_point3(ndc) - eye).normalize();
                    let mut depth = 0.1;
                    while depth < 130.0 {
                        let p = eye + dir * (depth / dir.dot(forward));
                        let found = sets.at(fx, fy, forward.dot(p - eye));
                        fragments += 1;
                        walked += found.count_ones();
                        for (i, probe) in probes.iter().enumerate() {
                            if covers(i, probe, p) {
                                covered += 1;
                                assert!(
                                    found & (1 << i) != 0,
                                    "probe {i} ({probe:?}) missing at pixel {x},{y}, depth {depth}"
                                );
                            }
                        }
                        depth *= 1.07;
                    }
                }
            }
            // and the sets do narrow the list of 64
            let (mean, mean_covered) = (walked as f32 / fragments as f32, covered as f32 / fragments as f32);
            assert!(
                covered > 100_000 && mean < 16.0,
                "{mean_covered} probes cover a fragment, {mean} walked"
            );
        }
    }

    #[test]
    fn a_probe_extent_holds_its_box() {
        // a sphere probe: its own sphere
        let sphere = probe(Vec3::new(3.0, 4.0, 5.0), 9.0, 1.0, Mat4::ZERO);
        assert_eq!(extent(&sphere), (Vec3::new(3.0, 4.0, 5.0), 9.0));
        // a box of half sizes 3, 4, 12 around (10, 20, 30), whatever the
        // origin of its probe: the sphere through its corners
        let to_world = Mat4::from_scale_rotation_translation(
            Vec3::new(3.0, 4.0, 12.0),
            glam::Quat::from_rotation_z(0.7),
            Vec3::new(10.0, 20.0, 30.0),
        );
        let (c, r) = extent(&probe(Vec3::new(11.0, 20.0, 33.0), 1.0, -1.0, to_world.inverse()));
        assert!(
            c.distance(Vec3::new(10.0, 20.0, 30.0)) < 1e-3 && (r - 13.0).abs() < 1e-3,
            "{c:?} {r}"
        );
        // a box that cannot be turned back to the world covers everything
        let (_, r) = extent(&probe(Vec3::ZERO, 1.0, -1.0, Mat4::ZERO));
        assert_eq!(r, f32::INFINITY);
        let eye = Vec3::new(100.0, 100.0, 25.0);
        let view = glam::camera::rh::view::look_at_mat4(eye, eye + Vec3::X, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.5, 0.1);
        let far = probe(eye + Vec3::new(-9000.0, 0.0, 0.0), 10.0, 0.0, Mat4::ZERO);
        let sets = tiles(view, proj, eye, [1200, 800], &[far, far, probe(Vec3::ZERO, 1.0, -1.0, Mat4::ZERO)]);
        for (x, y, depth) in [(0.5, 0.5, 0.2), (600.0, 400.0, 30.0), (1199.5, 799.5, 4000.0)] {
            assert_eq!(sets.at(x, y, depth), 0b101, "pixel {x},{y}, depth {depth}");
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
        // IRR_CAPTURE_LOD_OFFSET: mip m of the irradiance capture is mip m + 1 of the capture
        assert_eq!(IRR_CAPTURE_RES, CAPTURE_RES >> 1);
        assert_eq!(src.matches(&format!("@workgroup_size({WORKGROUP}, {WORKGROUP}, 1)")).count(), 4);
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
        capture_into(encoder, colors, |face| probes.capture_target(face));
    }

    /// The same into the faces `target` gives.
    fn capture_into<'a>(encoder: &mut wgpu::CommandEncoder, colors: [[f64; 3]; 6], target: impl Fn(u32) -> &'a wgpu::TextureView) {
        for (face, c) in colors.into_iter().enumerate() {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target(face as u32),
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
        let _gpu = crate::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let probes = Probes::new(&device, 8);
        let close = |a: [f32; 4], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k] as f32).abs() <= 2e-3 * (1.0 + b[k] as f32)) && a[3] == 1.0;
        // a cube of one color stays that color at every roughness
        let plain = [0.5, 2.0, 0.125];
        let mut encoder = device.create_command_encoder(&Default::default());
        capture(&probes, &mut encoder, [plain; 6]);
        probes.filter(&mut encoder, 3, false, true);
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
        probes.filter(&mut encoder, 1, false, true);
        queue.submit([encoder.finish()]);
        for (face, color) in FACES.into_iter().enumerate() {
            let sharp = read(&device, &queue, &probes.array, 0, 6 + face as u32);
            let center = sharp[(PROBE_RES / 2 * PROBE_RES + PROBE_RES / 2) as usize];
            assert!(close(center, color), "face {face}: {center:?}");
            let rough = read(&device, &queue, &probes.array, PROBE_MIPS - 1, 6 + face as u32)[0];
            assert!(!close(rough, color), "face {face} at full roughness: {rough:?}");
        }
        // with its irradiance capture, a probe's irradiance comes from it and
        // its radiance from the radiance capture, both stored as the Legacy
        // Viewer stores its probes: 0.7 truncated to 6 bits of mantissa in
        // red and green, 5 in blue
        let grey = [0.7, 0.7, 0.7];
        let stored = [0.6953125, 0.6953125, 0.6875];
        let mut encoder = device.create_command_encoder(&Default::default());
        capture(&probes, &mut encoder, [plain; 6]);
        capture_into(&mut encoder, [grey; 6], |face| probes.irr_capture_target(face));
        probes.filter(&mut encoder, 5, true, true);
        queue.submit([encoder.finish()]);
        for face in 0..6 {
            let radiance = read(&device, &queue, &probes.array, 0, 5 * 6 + face);
            assert!(radiance.iter().all(|t| close(*t, plain)), "radiance face {face}");
            let irradiance = read(&device, &queue, &probes.irradiance, 0, 5 * 6 + face);
            let off = irradiance.iter().position(|t| !close(*t, stored));
            assert_eq!(off, None, "irradiance face {face}: {:?}", off.map(|i| irradiance[i]));
        }
        // without HDR every stage is 8 bits: clamped to 1, in 1 / 255 steps
        // (0.5 -> 128 / 255, 0.125 -> 32 / 255), with or without the
        // irradiance capture
        let ldr = [128.0 / 255.0, 1.0, 32.0 / 255.0];
        for (slot, irr_capture) in [(6, false), (7, true)] {
            let mut encoder = device.create_command_encoder(&Default::default());
            capture(&probes, &mut encoder, [plain; 6]);
            capture_into(&mut encoder, [plain; 6], |face| probes.irr_capture_target(face));
            probes.filter(&mut encoder, slot, irr_capture, false);
            queue.submit([encoder.finish()]);
            for face in 0..6 {
                for (texture, mip) in [(&probes.array, 0), (&probes.array, PROBE_MIPS - 1), (&probes.irradiance, 0)] {
                    let texels = read(&device, &queue, texture, mip, slot * 6 + face);
                    let off = texels.iter().position(|t| !close(*t, ldr));
                    assert_eq!(off, None, "slot {slot} face {face} mip {mip}: {:?}", off.map(|i| texels[i]));
                }
            }
        }
    }
}
