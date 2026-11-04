//! Compare the production Mask fragment against its former order on the GPU.
use crate::pipeline_cache::CachedPipelines;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a GPU"]
fn mask_rejection_preserves_gpu_colour_and_sample_depth() {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let mut fragments = String::new();
    for name in ["mask_color", "fs_mask", "fs_mask_g"] {
        let start = object.find(&format!("fn {name}(")).expect("production function");
        let end = start + object[start..].find("\n}").expect("function end") + 2;
        fragments.push_str(
            &object[start..end]
                .replace("@builtin(front_facing) ", "")
                .replace("@location(0) ", ""),
        );
        fragments.push('\n');
    }
    let source = format!(
        r#"
override ALPHA_TO_COVERAGE:bool=false;
override REFERENCE:bool=false;
override GBUFFER:bool=false;
struct DrawRecord {{params:vec4<f32>}};
@group(0) @binding(0) var<storage,read> records:array<DrawRecord>;
struct VsOut {{@builtin(position) clip:vec4<f32>,@location(0) uv:vec2<f32>,
    @location(1) @interpolate(flat) record:u32}};
struct Material {{color:vec4<f32>}};
struct Shaded {{color:vec4<f32>}};
struct GOut {{@location(0) color:vec4<f32>,@location(1) gbuf:vec4<f32>}};
fn eval_material(v:VsOut,front:bool)->Material {{
    let r=records[v.record];
    let a=mix(r.params.x,r.params.y,clamp(v.uv.x,0.0,1.0));
    return Material(vec4<f32>(0.2+0.4*v.uv.x,0.3,0.7,a));
}}
fn shade_material(v:VsOut,m:Material)->Shaded {{return Shaded(vec4<f32>(m.color.rgb*0.7+vec3<f32>(0.03),m.color.a));}}
fn shade(v:VsOut,front:bool)->Shaded {{return shade_material(v,eval_material(v,front));}}
fn gbuffer(s:Shaded)->vec4<f32> {{return vec4<f32>(s.color.r,0.5,0.25,1.0);}}
{fragments}
fn vertex(i:u32,inst:u32)->VsOut {{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0))[i];
    return VsOut(vec4<f32>(p,0.25,1.0),p*0.5+0.5,inst);
}}
@vertex fn vs(@builtin(vertex_index) i:u32,@builtin(instance_index) inst:u32)->VsOut {{return vertex(i,inst);}}
@vertex fn vs_back(@builtin(vertex_index) i:u32)->VsOut {{
    var v=vertex(i,0u);v.clip.z=0.75;return v;
}}
@fragment fn foreground(v:VsOut,@builtin(front_facing) front:bool)->GOut {{
    if (REFERENCE) {{
        let s=shade(v,front);
        return GOut(mask_color(records[v.record],s.color),select(vec4<f32>(0.5),gbuffer(s),GBUFFER));
    }}
    if (GBUFFER) {{return fs_mask_g(v,front);}}
    return GOut(fs_mask(v,front),vec4<f32>(0.5));
}}
// Drawn behind the tested face: different per-sample depth writes become
// visible in the resolved colour, including partial alpha-to-coverage.
@fragment fn background(v:VsOut)->GOut {{return GOut(vec4<f32>(0.05,0.8,0.1,1.0),vec4<f32>(0.0,1.0,0.0,1.0));}}
"#
    );
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("Mask validation requires a GPU");
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("Mask GPU device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Mask A/B sample coverage"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let mut records = Vec::new();
    for cutoff in [0.0_f32, 0.33, 0.5, 1.0] {
        for (left, right) in [(0.0, 0.0), (0.0, 1.0), (cutoff, cutoff), (1.0, 1.0)] {
            records.push([left, right, 0.0, cutoff]);
        }
    }
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Alpha ramps and cutoff boundaries"),
        contents: bytemuck::cast_slice(&records),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let width = 64;
    let height = records.len() as u32 * 16;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let make_pipeline = |entry, samples, reference: bool, gbuffer: bool, background: bool| {
        device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(if background { "vs_back" } else { "vs" }),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                alpha_to_coverage_enabled: samples > 1 && !background,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[
                        ("ALPHA_TO_COVERAGE", f64::from(samples > 1)),
                        ("REFERENCE", f64::from(reference)),
                        ("GBUFFER", f64::from(gbuffer)),
                    ],
                    ..Default::default()
                },
                targets: &[Some(format.into()), Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    for samples in [1, 4] {
        for gbuffer in [false, true] {
            let mut images = Vec::new();
            for reference in [true, false] {
                let pipeline = make_pipeline("foreground", samples, reference, gbuffer, false);
                let background = make_pipeline("background", samples, reference, gbuffer, true);
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                let (_, depth) = crate::renderer::make_view(
                    &device,
                    "Mask depth",
                    width,
                    height,
                    wgpu::TextureFormat::Depth32Float,
                    samples,
                    wgpu::TextureUsages::RENDER_ATTACHMENT,
                );
                let targets = [0, 1].map(|_| {
                    crate::renderer::make_view(
                        &device,
                        "Mask colour",
                        width,
                        height,
                        format,
                        samples,
                        wgpu::TextureUsages::RENDER_ATTACHMENT
                            | if samples == 1 {
                                wgpu::TextureUsages::COPY_SRC
                            } else {
                                wgpu::TextureUsages::empty()
                            },
                    )
                });
                let resolves = [0, 1].map(|_| {
                    crate::renderer::make_view(
                        &device,
                        "Mask resolve",
                        width,
                        height,
                        format,
                        1,
                        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    )
                });
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(height) * 256 * 2,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let attachments = [0, 1].map(|i| {
                        Some(wgpu::RenderPassColorAttachment {
                            view: &targets[i].1,
                            depth_slice: None,
                            resolve_target: (samples > 1).then_some(&resolves[i].1),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })
                    });
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &attachments,
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    for row in 0..records.len() as u32 {
                        pass.set_viewport(0.0, row as f32 * 16.0, width as f32, 16.0, 0.0, 1.0);
                        pass.draw(0..3, row..row + 1);
                    }
                    pass.set_pipeline(&background);
                    pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                    pass.draw(0..3, 0..1);
                }
                for i in 0..2 {
                    let texture = if samples > 1 { &resolves[i].0 } else { &targets[i].0 };
                    encoder.copy_texture_to_buffer(
                        texture.as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: i as u64 * u64::from(height) * 256,
                                bytes_per_row: Some(256),
                                rows_per_image: None,
                            },
                        },
                        texture.size(),
                    );
                }
                queue.submit([encoder.finish()]);
                readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("Mask readback"));
                device.poll(wgpu::PollType::wait_indefinitely()).expect("Mask GPU completion");
                images.push(readback.get_mapped_range(..).expect("Mask pixels").to_vec());
            }
            assert_eq!(
                images[0], images[1],
                "Mask colour/gbuffer/sample depth: samples={samples}, gbuffer={gbuffer}"
            );
            assert!(
                images[0].as_chunks::<4>().0.iter().any(|p| p[1] > 180),
                "transparent pixels reveal the background"
            );
            assert!(
                images[0].as_chunks::<4>().0.iter().any(|p| p[2] > 100),
                "covered pixels remain drawn"
            );
        }
    }
}
