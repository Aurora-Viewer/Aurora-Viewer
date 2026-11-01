//! Aurora Viewer renderer: wgpu on Vulkan (Metal on macOS), bindless textures,
//! sub-allocated geometry and multi-draw-indirect submission.
//!
//! Two halves: `main_thread` (the `Renderer` the scene uses: its stores,
//! the frame packet) and `renderer` (the passes, run by the render thread
//! of `render_thread`); see main_thread.rs for who owns what.

pub mod adapter_choice;
pub mod arena;
pub mod bc7;
mod bloom;
mod contact_shadows;
mod cutout;
// (Vulkan only: Metal textures stay with wgpu's allocator)
#[cfg_attr(target_os = "macos", path = "dedicated_none.rs")]
mod dedicated;
mod device_lost;
mod dof;
mod drawn_pick;
mod exposure;
pub mod frame_priority;
pub mod gpu_alloc;
pub mod gpu_cull;
mod gtao;
mod helpers;
mod histogram;
pub mod hud;
mod label_depth;
pub mod light_tiles;
pub mod looks;
pub mod main_thread;
mod motion_debug;
mod motion_history;
mod occlusion;
pub mod pacing;
mod packet;
mod pick;
#[cfg(test)]
mod portal_tests;
pub mod present;
pub mod preview;
pub mod probes;
mod render_thread;
mod renderer;
mod rlva;
mod selection;
pub mod shadow_cull;
mod shadow_merge;
#[cfg(test)]
mod specular_aa;
pub mod stars;
pub mod stereo_view;
#[cfg(test)]
mod terrain_tests;
#[cfg(test)]
mod test_gpu;
pub mod tex_anim;
pub mod textures;
pub mod thread_cpu;
pub mod types;
mod ui_blur;
mod ui_frame;
mod ui_glass;
mod ui_world;
pub mod upload;
mod vr_control;
pub mod vram;
pub mod vulkan_drivers;
pub mod vulkan_layers;
pub mod water_effects;
mod water_optics;
mod water_wake;
pub use vr_control::VrState;
#[cfg(target_os = "linux")]
mod vr_mirror;
pub mod writes;
#[cfg(target_os = "linux")]
mod xr_device;
#[cfg(target_os = "linux")]
mod xr_frame;
pub use water_effects::{WaterEffects, WaterEvent, WaterGround};

pub use arena::MeshAlloc;
// What a tile of a photo draws around what it keeps, for the bloom, the
// depth of field and the contact shadows (snapshot tiles in the viewer).
pub use bloom::tile_reach as bloom_tile_reach;
pub use contact_shadows::tile_reach as contact_tile_reach;
pub use dof::tile_reach as dof_tile_reach;
pub use drawn_pick::{Drawn, DrawnFace, DrawnMode};
pub use gpu_cull::set_whole_chunk_uploads;
pub use histogram::HISTOGRAM_BINS;
pub use hud::HudView;
pub use label_depth::{world_label_at, world_line_at};
pub use main_thread::{GpuInfo, Renderer};
pub use renderer::{CASCADES, RenderError};
pub use textures::{MASK_CUTOFF, MipLevel, alpha_coverage, build_mips, build_mips_alpha_weighted, preserve_alpha_coverage};
pub use types::*;
pub use ui_blur::blurred_backdrop;
pub use ui_frame::{EguiFrame, EguiHandOver, vr_hint_marker};
pub use ui_glass::{glass_panel, glass_panel_faded, glass_region};
pub use ui_world::end_world_labels;
pub use upload::{StagedMesh, StagedTexture, StagingPool, TexelFormat};

pub use egui;
pub use egui_wgpu;
pub use wgpu;

/// Native graphics API for this platform.
pub const BACKENDS: wgpu::Backends = if cfg!(target_os = "macos") {
    wgpu::Backends::METAL
} else {
    wgpu::Backends::VULKAN
};

pub(crate) const COMMON_SHADER: &str = concat!(
    include_str!("shaders/common.wgsl"),
    "\n",
    include_str!("shaders/water_spectrum.wgsl")
);

/// Offline material iteration only. Normal viewers always use their baked shaders.
pub(crate) fn water_shader_source(files: &[&str], baked: &'static str) -> std::borrow::Cow<'static, str> {
    if std::env::var_os("AURORA_DEMO").is_none() || std::env::var("AURORA_DEMO_WATER_SHADERS").as_deref() != Ok("1") {
        return baked.into();
    }
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
    let mut source = String::new();
    for file in files {
        match std::fs::read_to_string(directory.join(file)) {
            Ok(text) => {
                source.push_str(&text);
                source.push('\n');
            }
            Err(error) => {
                log::warn!("offline water shader {file}: {error}; using baked source");
                return baked.into();
            }
        }
    }
    source.into()
}

pub(crate) fn common_shader_source() -> std::borrow::Cow<'static, str> {
    water_shader_source(&["common.wgsl", "water_spectrum.wgsl"], COMMON_SHADER)
}

#[cfg(test)]
mod shader_tests {
    #[test]
    fn rlva_sphere_shader_is_valid() {
        validate(include_str!("shaders/rlva.wgsl"));
        validate(include_str!("shaders/rlva_overlay.wgsl"));
        validate(include_str!("shaders/rlva_resolution.wgsl"));
    }
    fn validate(src: &str) {
        let module = match naga::front::wgsl::parse_str(src) {
            Ok(m) => m,
            Err(e) => panic!("{}", e.emit_to_string(src)),
        };
        let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(src));
        }
    }

    #[test]
    fn motion_history_shader_is_valid() {
        validate(include_str!("shaders/motion_history.wgsl"));
    }

    #[test]
    fn water_compute_shader_is_valid() {
        validate(include_str!("shaders/water_sim.wgsl"));
        validate(include_str!("shaders/water_wake.wgsl"));
    }

    #[test]
    fn refracted_water_caustics_shader_is_valid() {
        validate(concat!(
            include_str!("shaders/water_spectrum.wgsl"),
            "\n",
            include_str!("shaders/water_caustics.wgsl")
        ));
    }

    #[test]
    fn water_receivers_specialize_for_mask_and_blend_pipelines() {
        let source = format!(
            "{}\n{}\n{}",
            crate::COMMON_SHADER,
            include_str!("shaders/object.wgsl"),
            include_str!("shaders/env.wgsl")
        );
        let module = naga::front::wgsl::parse_str(&source).expect("scene shader parses");
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .expect("scene shader validates");
        for entry in ["fs_mask", "fs_mask_g", "fs_blend"] {
            for coverage in [0.0, 1.0] {
                for early_discard in [0.0, 1.0] {
                    let mut constants = naga::back::PipelineConstants::default();
                    constants.insert("ALPHA_TO_COVERAGE".into(), coverage);
                    constants.insert("BLEND_EARLY_DISCARD".into(), early_discard);
                    let result = naga::back::pipeline_constants::process_overrides(
                        &module,
                        &info,
                        Some((naga::ShaderStage::Fragment, entry)),
                        &constants,
                    );
                    if let Err(error) = result {
                        panic!("{entry}, coverage {coverage}, early discard {early_discard}: {error:#?}");
                    }
                }
            }
        }
    }

    #[test]
    fn asset_preview_shader_is_valid() {
        validate(include_str!("shaders/preview.wgsl"));
    }

    #[test]
    fn scene_shader_is_valid() {
        let src = format!(
            "{}\n{}\n{}",
            crate::COMMON_SHADER,
            include_str!("shaders/object.wgsl"),
            include_str!("shaders/env.wgsl")
        );
        validate(&src);
    }

    /// Use the actual water projection/depth entry points with a constant
    /// material. Real triangles must survive the near plane from both sides,
    /// preserve rolled coverage and still fail the test behind opaque depth.
    #[test]
    fn water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu() {
        // The Windows checkout can use CRLF. Check entry extraction even
        // without an adapter, and keep its source independent of line endings.
        let env = include_str!("shaders/env.wgsl").replace("\r\n", "\n");
        let vertex =
            &env[env.find("@vertex\nfn vs_water(").expect("water vertex")..env.find("fn analytic_wave(").expect("end of water vertices")];
        let fragment = &env
            [env.find("@fragment\nfn fs_water(").expect("water fragment")..env.find("// Event history").expect("end of water fragments")];
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let features = adapter.features() & wgpu::Features::DEPTH_CLIP_CONTROL;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }))
        .expect("GPU device");
        let source = format!(
            r#"
struct ProbeFrame {{view_proj:mat4x4<f32>}};
@group(0) @binding(0) var<uniform> frame:ProbeFrame;
struct VsIn {{@builtin(vertex_index) vertex:u32,@builtin(instance_index) layer:u32}};
struct WaterOut {{@builtin(position) clip:vec4<f32>,@location(0) world_pos:vec3<f32>,@location(1) level:f32}};
struct VsOut {{@builtin(position) clip:vec4<f32>,@location(0) world_pos:vec3<f32>}};
fn water_vertex(input:VsIn)->WaterOut {{
    let corners=array<vec2<f32>,6>(vec2<f32>(-100.0,-100.0),vec2<f32>(100.0,-100.0),vec2<f32>(100.0,100.0),
        vec2<f32>(-100.0,-100.0),vec2<f32>(100.0,100.0),vec2<f32>(-100.0,100.0));
    let world=vec3<f32>(corners[input.vertex],20.0-f32(input.layer)*0.01);
    return WaterOut(frame.view_proj*vec4<f32>(world,1.0),world,20.0);
}}
fn water_pixel(surface:WaterOut)->vec4<f32> {{return vec4<f32>(0.0,0.0,1.0,1.0);}}
@vertex fn vs_exclusion(input:VsIn)->VsOut {{let water=water_vertex(input);return VsOut(water.clip,water.world_pos);}}
{vertex}
{fragment}
"#
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water mesh clamp"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 8 * 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (texture, view) = crate::renderer::make_view(
            &device,
            "mesh coverage",
            8,
            8,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        for samples in [1, 4] {
            let msaa = (samples > 1).then(|| {
                crate::renderer::make_view(
                    &device,
                    "mesh MSAA",
                    8,
                    8,
                    wgpu::TextureFormat::Rgba8Unorm,
                    samples,
                    wgpu::TextureUsages::RENDER_ATTACHMENT,
                )
            });
            let (_, depth) = crate::renderer::make_view(
                &device,
                "opaque depth",
                8,
                8,
                wgpu::TextureFormat::Depth32Float,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            for hardware in [false, true] {
                if hardware && features.is_empty() {
                    continue;
                }
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("water mesh depth"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(if hardware { "vs_water" } else { "vs_water_unclipped" }),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        unclipped_depth: hardware,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::Greater),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: samples,
                        ..Default::default()
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(if hardware { "fs_water" } else { "fs_water_unclipped" }),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                });
                for delta in [0.01f32, -0.01, 0.3, -0.3] {
                    for roll in [0.0, 0.35] {
                        let eye = glam::Vec3::new(0.0, 0.0, 20.0 + delta);
                        let up = glam::Quat::from_axis_angle(glam::Vec3::Y, roll) * glam::Vec3::Z;
                        let camera = glam::camera::rh::view::look_at_mat4(eye, eye + glam::Vec3::Y, up);
                        let projection =
                            glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_3, 1.5, 0.1);
                        queue.write_buffer(&uniform, 0, bytemuck::cast_slice(&(projection * camera).to_cols_array()));
                        for occluder in [0.0, 0.75] {
                            let mut encoder = device.create_command_encoder(&Default::default());
                            {
                                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: msaa.as_ref().map_or(&view, |(_, v)| v),
                                        depth_slice: None,
                                        resolve_target: msaa.as_ref().map(|_| &view),
                                        ops: wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                                r: 1.0,
                                                g: 0.0,
                                                b: 0.0,
                                                a: 1.0,
                                            }),
                                            store: wgpu::StoreOp::Store,
                                        },
                                    })],
                                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                                        view: &depth,
                                        depth_ops: Some(wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(occluder),
                                            store: wgpu::StoreOp::Store,
                                        }),
                                        stencil_ops: None,
                                    }),
                                    ..Default::default()
                                });
                                pass.set_pipeline(&pipeline);
                                pass.set_bind_group(0, &group, &[]);
                                pass.draw(0..6, 0..1);
                            }
                            encoder.copy_texture_to_buffer(
                                wgpu::TexelCopyTextureInfo {
                                    texture: &texture,
                                    mip_level: 0,
                                    origin: Default::default(),
                                    aspect: wgpu::TextureAspect::All,
                                },
                                wgpu::TexelCopyBufferInfo {
                                    buffer: &readback,
                                    layout: wgpu::TexelCopyBufferLayout {
                                        offset: 0,
                                        bytes_per_row: Some(256),
                                        rows_per_image: Some(8),
                                    },
                                },
                                wgpu::Extent3d {
                                    width: 8,
                                    height: 8,
                                    depth_or_array_layers: 1,
                                },
                            );
                            queue.submit([encoder.finish()]);
                            readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("water mesh readback"));
                            device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
                            let pixels = readback.get_mapped_range(..).expect("water mesh pixels");
                            for y in [0usize, 7] {
                                let covered =
                                    ((delta > 0.0 && y == 7) || (delta < 0.0 && y == 0)) && (occluder == 0.0 || delta.abs() < 0.1);
                                for x in 0..8 {
                                    assert_eq!(
                                        &pixels[y * 256 + x * 4..y * 256 + x * 4 + 4],
                                        if covered { &[0, 0, 255, 255] } else { &[255, 0, 0, 255] },
                                        "eye {delta}, roll {roll}, hardware {hardware}, samples {samples}, occluder {occluder}, pixel {x},{y}"
                                    );
                                }
                            }
                            drop(pixels);
                            readback.unmap();
                        }
                    }
                }
            }
        }
        // Two real water sheets project to the same pixels. A near-clamped
        // depth would merge them before near and make the last draw win.
        let (interface_tex, interface) = crate::renderer::make_view(
            &device,
            "water interface ordering",
            8,
            8,
            wgpu::TextureFormat::Depth32Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let interface_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("production water interface ordering"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_water_unclipped"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_water_interface"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &interface_pipe.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        for delta in [0.01f32, 0.3] {
            let eye = glam::Vec3::new(0.0, 0.0, 20.0 + delta);
            let view = glam::camera::rh::view::look_at_mat4(eye, eye + glam::Vec3::Y, glam::Vec3::Z);
            let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_3, 1.5, 0.1);
            queue.write_buffer(&uniform, 0, bytemuck::cast_slice(&(projection * view).to_cols_array()));
            let mut reference = None;
            for order in [[0u32, 1], [1, 0]] {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("overlapping wave sheets"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &interface,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&interface_pipe);
                    pass.set_bind_group(0, &group, &[]);
                    for layer in order {
                        pass.draw(0..6, layer..layer + 1);
                    }
                }
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &interface_tex,
                        mip_level: 0,
                        origin: Default::default(),
                        aspect: wgpu::TextureAspect::DepthOnly,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(8),
                        },
                    },
                    wgpu::Extent3d {
                        width: 8,
                        height: 8,
                        depth_or_array_layers: 1,
                    },
                );
                queue.submit([encoder.finish()]);
                readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("interface order readback"));
                device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
                let pixels = readback.get_mapped_range(..).expect("interface order samples");
                let data: &[f32] = bytemuck::cast_slice(&pixels);
                let expected = 1.0 / (1.0 + (eye.z - 20.0) / (0.875 * (std::f32::consts::FRAC_PI_3 * 0.5).tan()));
                let row = data[7 * 64..7 * 64 + 8].to_vec();
                if let Some(previous) = &reference {
                    assert_eq!(&row, previous, "interface selection must be independent of draw order");
                }
                reference = Some(row);
                for x in 0..8 {
                    assert_eq!(data[x], 0.0, "air must not write an interface");
                    // Clipping a 200 m triangle to this tiny target has sub-mm
                    // interpolation error, well below the 1 cm sheet spacing.
                    assert!(
                        (data[7 * 64 + x] - expected).abs() < 0.0002,
                        "the nearest interface must win in either order, including before near: delta {delta}, order {order:?}, got {} expected {expected}",
                        data[7 * 64 + x]
                    );
                }
                drop(pixels);
                readback.unmap();
            }
        }
    }

    #[test]
    fn water_exclusion_mask_orders_real_surfaces_on_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let env = include_str!("shaders/env.wgsl");
        let depth = &env[env.find("fn water_interface_depth(").expect("depth")..env.find("// Event history").expect("end")];
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water exclusion regression"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    r#"
struct Frame {{view_proj:mat4x4<f32>}};
@group(0) @binding(0) var<uniform> frame:Frame;
struct WaterOut {{@builtin(position) clip:vec4<f32>,@location(0) world_pos:vec3<f32>}};
struct VsOut {{@builtin(position) clip:vec4<f32>,@location(0) world_pos:vec3<f32>}};
@vertex fn vertex(@builtin(vertex_index) v:u32,@builtin(instance_index) layer:u32)->WaterOut {{
    let corners=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    let w=1.0+f32(layer);
    let world=vec3<f32>(corners[v]*w,w);
    return WaterOut(vec4<f32>(corners[v],0.5,1.0),world);
}}
{depth}
"#
                )
                .into(),
            ),
        });
        let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water mask test"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&group_layout)],
            immediate_size: 0,
        });
        let make_pipe = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::R8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let water = make_pipe("fs_water_mask");
        let exclusion = make_pipe("fs_water_exclusion");
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut projection = glam::Mat4::IDENTITY;
        projection.z_axis.w = 1.0;
        projection.w_axis.w = 0.0;
        queue.write_buffer(&uniform, 0, bytemuck::cast_slice(&projection.to_cols_array()));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let (mask_texture, mask) = crate::renderer::make_view(
            &device,
            "exclusion mask",
            8,
            8,
            wgpu::TextureFormat::R8Unorm,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let (_, depth_view) = crate::renderer::make_view(
            &device,
            "exclusion depth",
            8,
            8,
            wgpu::TextureFormat::Depth32Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 8 * 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for exclusion_layer in [0, 2] {
            for reverse in [false, true] {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &mask,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_bind_group(0, &group, &[]);
                    let draws = [(&water, 1), (&exclusion, exclusion_layer)];
                    for i in if reverse { [1, 0] } else { [0, 1] } {
                        let (pipe, layer) = draws[i];
                        pass.set_pipeline(pipe);
                        pass.draw(0..3, layer..layer + 1);
                    }
                }
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &mask_texture,
                        mip_level: 0,
                        origin: Default::default(),
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(8),
                        },
                    },
                    wgpu::Extent3d {
                        width: 8,
                        height: 8,
                        depth_or_array_layers: 1,
                    },
                );
                queue.submit([encoder.finish()]);
                readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("mask readback"));
                device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
                let pixels = readback.get_mapped_range(..).expect("mask pixels");
                for y in 0..8 {
                    assert!(
                        pixels[y * 256..y * 256 + 8]
                            .iter()
                            .all(|p| *p == if exclusion_layer == 0 { 0 } else { 255 }),
                        "only an exclusion in front of water wins, in either draw order"
                    );
                }
                drop(pixels);
                readback.unmap();
            }
        }
    }

    /// Execute the production fog function, not a Rust mirror: distant targets
    /// beyond the same water exit must have identical attenuation/scattering.
    #[test]
    fn underwater_fog_stops_at_the_air_boundary_on_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter; water fog regression skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let common = include_str!("shaders/common.wgsl");
        let start = common.find("fn camera_water_level(").expect("production waterline function");
        let function = &common[start..];
        let end = function.find("\n// LLDrawPoolWaterExclusion").expect("end of camera/fog functions");
        let optics = include_str!("shaders/env.wgsl");
        let dielectric = &optics[optics.find("fn water_dielectric(").expect("dielectric helper")
            ..optics.find("fn water_underside(").expect("end of dielectric")];
        let source = format!(
            r#"
struct FogFrame {{camera_pos:vec4<f32>,params:vec4<f32>,water_params:vec4<f32>,water_fog:vec4<f32>,
    sun_color:vec4<f32>,sun_dir:vec4<f32>,sky_zenith:vec4<f32>,cam_up:vec4<f32>}};
var<private> enhanced:bool;
var<private> fine_wake:f32;
var<private> reflection:bool;
fn water_fx_on(bit:u32)->bool {{return (enhanced && bit==256u) || (fine_wake!=0.0 && bit==16u);}}
fn in_reflection_pass()->bool {{return reflection;}}
fn water_fx_bed(p:vec2<f32>)->vec2<f32> {{return vec2<f32>(0.0);}}
fn water_wake_sample(p:vec2<f32>,level:f32)->vec4<f32> {{return vec4<f32>(0.0,0.0,fine_wake,0.0);}}
var<private> frame:FogFrame;
@group(0) @binding(0) var<storage,read_write> result:array<vec4<f32>>;
{}
{}
@compute @workgroup_size(1)
fn probe(@builtin(global_invocation_id) id:vec3<u32>) {{
    frame.camera_pos=vec4<f32>(0.0,0.0,-2.0,0.0);
    frame.params=vec4<f32>(0.0);
    frame.cam_up=vec4<f32>(0.0);
    fine_wake=select(0.0,0.3,id.x==15u);
    reflection=id.x==14u;
    frame.water_params=vec4<f32>(1.0);
    frame.water_fog=vec4<f32>(0.05,0.12,0.18,1.0);
    frame.sun_color=vec4<f32>(select(1.0,0.0,id.x==9u));
    frame.sun_dir=vec4<f32>(0.0,0.0,1.0,0.0);
    frame.sky_zenith=vec4<f32>(select(0.3,0.0,id.x==9u));
    enhanced=id.x!=8u;
    if(id.x==10u) {{frame.camera_pos.z=-0.001;}}
    if(id.x==11u) {{frame.camera_pos.z=2.0;}}
    if(id.x==12u || id.x==14u) {{frame.camera_pos.z=0.1;frame.cam_up.w=0.35;}}
    if(id.x==13u) {{frame.camera_pos.z=-0.1;frame.cam_up.w=-0.35;}}
    if(id.x==15u) {{frame.camera_pos.z=0.1;}}
    if(id.x==18u) {{frame.camera_pos.z=0.001;}}
    if(id.x==19u) {{frame.camera_pos.z=2.0;}}
    let targets=array<vec3<f32>,20>(vec3<f32>(0.0,0.0,2.0),vec3<f32>(0.0,0.0,200.0),
        vec3<f32>(2.0,0.0,2.0),vec3<f32>(200.0,0.0,200.0),
        vec3<f32>(0.0,0.0,-2.0),vec3<f32>(0.0,0.0,-20.0),vec3<f32>(0.0),vec3<f32>(0.0,0.0,-2.0),
        vec3<f32>(0.0,0.0,-20.0),vec3<f32>(0.0,0.0,-2.0),vec3<f32>(0.0,0.0,-2.0),vec3<f32>(0.0,0.0,-4.0),
        vec3<f32>(0.0,0.0,-20.0),vec3<f32>(0.0,0.0,-20.0),vec3<f32>(0.0,0.0,-20.0),vec3<f32>(0.0,0.0,-20.0),
        vec3<f32>(0.0,0.0,2.0),vec3<f32>(0.0,0.0,-2.0),vec3<f32>(0.0,0.0,-2.0),vec3<f32>(0.0,0.0,2.0));
    var fog=water_fog_view(targets[id.x],select(2.0,0.0,id.x==7u));
    // A real wave-interface hit is authoritative even when a local flat
    // boundary would incorrectly shorten that optical segment.
    if(id.x==16u || id.x==17u) {{fog=water_fog_segment(targets[id.x],2.0,0.001,0.0);}}
    result[id.x*2u]=vec4<f32>(fog.transmission,select(0.0,1.0,camera_underwater()));
    let angles=array<f32,7>(0.0,20.0,41.0,42.0,45.0,60.0,90.0);
    result[id.x*2u+1u]=vec4<f32>(fog.color,water_dielectric(sin(angles[min(id.x,6u)]*0.01745329252)));
}}
"#,
            &function[..end],
            dielectric
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water fog boundary regression"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("water fog boundary regression"),
            layout: None,
            module: &shader,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog samples"),
            size: 640,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog sample readback"),
            size: 640,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(20, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 640);
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("fog sample readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let mapped = readback.get_mapped_range(..).expect("mapped fog samples");
        let samples: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        for (a, b) in [(0, 1), (2, 3)] {
            for component in 0..2 {
                for c in 0..3 {
                    assert!(
                        (samples[a * 2 + component][c] - samples[b * 2 + component][c]).abs() < 1e-5,
                        "air distance must not add water fog: {samples:?}"
                    );
                }
            }
        }
        for (c, _) in samples[8][..3].iter().enumerate() {
            assert!(samples[10][c] < samples[8][c], "longer submerged paths attenuate each wavelength");
            assert!(samples[19][c] < samples[9][c], "night scattering must not glow like daylight");
            assert_eq!(samples[12][c], 1.0, "zero path is clear");
            assert_eq!(samples[13][c], 0.0);
            assert_eq!(samples[14][c], 1.0, "zero density is clear");
            assert_eq!(samples[15][c], 0.0);
            assert!(
                (samples[20][c] - samples[8][c]).abs() < 1e-5,
                "water path attenuation must not fade with eye depth"
            );
            for index in [22, 32, 34] {
                assert!(
                    (samples[index][c] - samples[8][c]).abs() < 1e-5,
                    "real interface segments must not be shortened by a local air plane"
                );
            }
            assert!(
                (samples[36][c] - samples[20][c]).abs() < 5e-4,
                "submerged endpoints retain attenuation continuously across the waterline"
            );
            assert_eq!(samples[38][c], 1.0, "an all-air ray remains clear");
            assert_eq!(samples[39][c], 0.0);
        }
        assert!(
            samples[8][0] < samples[8][1] && samples[8][1] < samples[8][2],
            "warm wavelengths must disappear first"
        );
        assert_eq!(
            samples[16][0], samples[16][1],
            "disabled absorption uses Legacy scalar transmission"
        );
        assert_eq!(samples[16][1], samples[16][2]);
        assert_eq!(samples[24][3], 1.0, "a crest submerges a camera above the native plane");
        assert!(
            samples[24][0] < samples[24][1] && samples[24][1] < samples[24][2],
            "crest fog uses coloured extinction"
        );
        assert_eq!(samples[26][3], 0.0, "a trough exposes a camera below the native plane");
        assert_eq!(samples[28][3], 0.0, "reflections never enter the underwater path");
        assert_eq!(samples[30][3], 1.0, "GPU-only interaction displacement also submerges the camera");
        assert_eq!(samples[1][3], 1.0, "a horizontal underwater ray has total internal reflection");
        assert_eq!(samples[3][3], 1.0, "grazing underwater rays retain physical total reflection");
        assert!(samples[9][3] < 0.15, "a 45-degree upward ray must mostly transmit the exterior");
        assert!(samples[11][3] < 0.03, "a 60-degree upward ray must not become a bottom mirror");
        assert!(
            (samples[13][3] - 0.02037).abs() < 0.0001,
            "normal-incidence water reflection is about two percent"
        );
        assert!(samples.iter().flatten().all(|v| v.is_finite() && *v >= 0.0));
    }

    /// The statements of vertex entry point `entry` that lead to its
    /// position: the model matrix, the placed vertex and the clip position.
    fn position_steps<'a>(src: &'a str, entry: &str) -> Vec<&'a str> {
        let start = src.find(&format!("fn {entry}(")).unwrap_or_else(|| panic!("{entry} not found"));
        let body = &src[start..];
        let end = body.find("\n}").unwrap_or(body.len());
        body[..end]
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("let model =") || l.starts_with("let ep =") || l.starts_with("out.clip ="))
            .collect()
    }

    /// The depth prepass draws the terrain with `vs_depth`, and without
    /// multisampling the scene pass tests `vs_terrain` against that depth
    /// (`GreaterEqual`): both must reach the position by the same
    /// operations. When only the prepass placed its vertices relative to
    /// the eye, patches of terrain failed the test and showed the sky.
    #[test]
    fn terrain_position_is_the_depth_prepass_position() {
        let prepass = position_steps(include_str!("shaders/object.wgsl"), "vs_depth");
        let scene = position_steps(include_str!("shaders/env.wgsl"), "vs_terrain");
        assert_eq!(prepass.len(), 3, "{prepass:?}");
        assert_eq!(prepass, scene);
        // the opaque PBR terrain shares that prepass
        assert_eq!(position_steps(include_str!("shaders/env.wgsl"), "vs_terrain_pbr"), scene);
    }

    /// The same for the objects: the depth prepasses draw the opaque faces
    /// with `vs_depth` (the single-sample one, and with MSAA the one of
    /// the scene pass's own depth) and the scene pass tests `vs_main`
    /// against that depth. Plain, worn, rigged and anchored faces all go
    /// through `eye_model` in both.
    #[test]
    fn object_position_is_the_depth_prepass_position() {
        let src = include_str!("shaders/object.wgsl");
        let prepass = position_steps(src, "vs_depth");
        let scene = position_steps(src, "vs_main");
        assert_eq!(prepass.len(), 3, "{prepass:?}");
        assert_eq!(prepass, scene);
        // the prepass that draws GTAO's mesh normals as well (#993)
        assert_eq!(position_steps(src, "vs_depth_normal"), scene);
        // and its masked, portal-clipped and contact faces
        assert_eq!(position_steps(src, "vs_depth_mask_normal"), scene);
    }

    /// Equal operations give equal depths only if the compiler may not
    /// rearrange them differently in each shader: the three positions are
    /// declared invariant.
    #[test]
    fn prepass_and_scene_positions_are_invariant() {
        let object = include_str!("shaders/object.wgsl");
        let env = include_str!("shaders/env.wgsl");
        for (src, name) in [
            (object, "DepthOut"),
            (object, "DepthNormalOut"),
            (object, "DepthMaskOut"),
            (object, "VsOut"),
            (env, "TerrainOut"),
            (env, "TerrainPbrOut"),
        ] {
            let start = src.find(&format!("struct {name} {{")).unwrap_or_else(|| panic!("{name} not found"));
            let body = &src[start..];
            let body = &body[..body.find('}').unwrap_or(body.len())];
            assert!(body.contains("@invariant @builtin(position) clip: vec4<f32>"), "{name}: {body}");
        }
    }

    #[test]
    fn ssao_shader_is_valid() {
        validate(include_str!("shaders/ssao.wgsl"));
    }

    #[test]
    fn contact_shadows_shader_is_valid() {
        validate(include_str!("shaders/contact_shadows.wgsl"));
        validate(include_str!("shaders/contact_shadows_filter.wgsl"));
        // a photo's, up to the most samples
        validate(&crate::contact_shadows::shader_source(192, 13));
        let (count, hard) = crate::contact_shadows::samples(100.0);
        validate(&crate::contact_shadows::shader_source(count, hard));
    }

    #[test]
    fn gtao_shader_is_valid() {
        validate(include_str!("shaders/gtao.wgsl"));
    }

    #[test]
    fn ssr_shader_is_valid() {
        validate(&format!("{}\n{}", crate::COMMON_SHADER, include_str!("shaders/ssr.wgsl")));
    }

    #[test]
    fn post_shader_is_valid() {
        validate(include_str!("shaders/post.wgsl"));
    }

    #[test]
    fn cutout_shader_is_valid() {
        validate(include_str!("shaders/cutout.wgsl"));
    }

    #[test]
    fn exposure_shader_is_valid() {
        validate(include_str!("shaders/exposure.wgsl"));
    }

    #[test]
    fn selection_overlay_shader_is_valid() {
        validate(include_str!("shaders/selection.wgsl"));
    }

    #[test]
    fn probe_filter_shader_is_valid() {
        validate(include_str!("shaders/probe_filter.wgsl"));
    }

    #[test]
    fn smaa_shader_is_valid() {
        validate(include_str!("shaders/smaa.wgsl"));
    }

    #[test]
    fn occlusion_shader_is_valid() {
        validate(&format!(
            "{}
{}",
            include_str!("shaders/hiz_test.wgsl"),
            include_str!("shaders/occlusion.wgsl")
        ));
    }

    #[test]
    fn cull_shader_is_valid() {
        validate(&format!(
            "{}
{}",
            include_str!("shaders/hiz_test.wgsl"),
            include_str!("shaders/cull.wgsl")
        ));
    }

    #[test]
    fn glow_shader_is_valid() {
        validate(include_str!("shaders/glow.wgsl"));
    }

    #[test]
    fn bloom_shader_is_valid() {
        validate(include_str!("shaders/bloom.wgsl"));
    }

    #[test]
    fn ui_blur_shader_is_valid() {
        validate(include_str!("shaders/ui_blur.wgsl"));
    }

    #[test]
    fn taa_shader_is_valid() {
        validate(include_str!("shaders/taa.wgsl"));
    }
}
