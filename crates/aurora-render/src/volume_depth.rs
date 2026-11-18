//! Final scene depth for local volumes under MSAA. The ordinary prepass is
//! opaque/masked only; rigged blended depth is written later to the MSAA
//! attachment. A volume must stop there, not at the background behind it.
use crate::pipeline_cache::CachedPipelines;

const SHADER: &str = r#"
@group(0) @binding(0) var scene_depth:texture_depth_multisampled_2d;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}
@fragment fn fs(@builtin(position) pixel:vec4<f32>)->@builtin(frag_depth) f32 {
    var nearest=0.0;
    // Reverse Z: max selects the nearest covered sample. Averaging depth
    // creates a fictitious surface and leaks shafts around silhouettes.
    for (var i=0u;i<textureNumSamples(scene_depth);i++) {
        nearest=max(nearest,textureLoad(scene_depth,vec2<i32>(pixel.xy),i32(i)));
    }
    return nearest;
}
"#;

pub(crate) struct Resolve {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
}

impl Resolve {
    pub fn new(device: &wgpu::Device, source: &wgpu::TextureView, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("final MSAA volume depth"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("final MSAA volume depth"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("final MSAA volume depth"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            }],
        });
        Self { pipeline, group }
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("resolve final scene depth for volumes"),
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_depth_includes_late_blended_faces_and_nearest_msaa_sample_on_gpu() {
        let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
        let mut functions = String::new();
        for name in ["water_clip", "depth_contact", "fs_depth_contact"] {
            let start = object.find(&format!("fn {name}(")).expect("production depth function");
            let end = start + object[start..].find("\n}").expect("function end") + 2;
            functions.push_str(&object[start..end].replace("@builtin(front_facing) ", ""));
            functions.push('\n');
        }
        let source = format!(
            r#"
const FLAG_PORTAL_EFFECT:u32=0u;
const FLAG_DOUBLE_SIDED:u32=1u;
const PORTAL_SURFACE_SCALE:f32=1.16;
const DEPTH_CONTACT_ALPHA:f32=0.5;
const WATER_CLIP:u32=0u;
struct DrawRecord {{flags:vec4<u32>,tex:vec4<u32>,base_color:vec4<f32>}};
const records=array<DrawRecord,4>(
    DrawRecord(vec4<u32>(1u),vec4<u32>(0u),vec4<f32>(0.0)),
    DrawRecord(vec4<u32>(1u),vec4<u32>(0u),vec4<f32>(0.4)),
    DrawRecord(vec4<u32>(1u),vec4<u32>(0u),vec4<f32>(0.8)),
    DrawRecord(vec4<u32>(1u),vec4<u32>(0u),vec4<f32>(1.0)));
struct Frame {{params:vec4<f32>,camera_pos:vec4<f32>}};
const frame=Frame(vec4<f32>(0.0),vec4<f32>(0.0));
struct VsOut {{@builtin(position) clip:vec4<f32>,@location(0) @interpolate(flat) record:u32,@location(1) uv:vec2<f32>,@location(2) world_pos:vec3<f32>}};
fn sample_tex(slot:u32,uv:vec2<f32>)->vec4<f32> {{return vec4<f32>(1.0);}}
fn portal_clip(rec:DrawRecord,world:vec3<f32>) {{}}
{functions}
@vertex fn vs(@builtin(vertex_index) index:u32,@builtin(instance_index) instance:u32)->VsOut {{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return VsOut(vec4<f32>(p[index],0.8,1.0),instance,vec2<f32>(0.5),vec3<f32>(0.0));
}}
@fragment fn alpha(input:VsOut,@builtin(front_facing) front:bool) {{fs_depth_contact(input,front);}}
@group(0) @binding(0) var depth:texture_depth_2d;
@group(0) @binding(1) var<storage,read_write> result:array<f32>;
@compute @workgroup_size(1) fn before(@builtin(global_invocation_id) i:vec3<u32>) {{result[i.x]=textureLoad(depth,vec2<i32>(i32(i.x)*4+2,2),0);}}
@compute @workgroup_size(1) fn after(@builtin(global_invocation_id) i:vec3<u32>) {{result[4u+i.x]=textureLoad(depth,vec2<i32>(i32(i.x)*4+2,2),0);}}
"#
        );
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("volume depth device");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production rigged blended depth"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        for samples in [1, 4] {
            let depth = |label, sample_count| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 16,
                        height: 4,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            };
            let single = depth("prepass depth", 1);
            let single_view = single.create_view(&Default::default());
            let multi = (samples > 1).then(|| depth("final scene depth", samples));
            let multi_view = multi.as_ref().map(|t| t.create_view(&Default::default()));
            let scene = multi_view.as_ref().unwrap_or(&single_view);
            let resolver = multi_view
                .as_ref()
                .map(|source| Resolve::new(&device, source, wgpu::TextureFormat::Depth32Float));
            let alpha_pipeline = |mask| {
                device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                    label: Some("late blended depth"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("alpha"),
                        compilation_options: Default::default(),
                        targets: &[],
                    }),
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Greater),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: samples,
                        mask,
                        alpha_to_coverage_enabled: false,
                    },
                    multiview_mask: None,
                    cache: None,
                })
            };
            let full = alpha_pipeline(!0);
            let one = alpha_pipeline(1);
            let probes = ["before", "after"].map(|entry| {
                device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: None,
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 32,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: crate::readback::readback_size(32),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let groups = probes.each_ref().map(|pipeline| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&single_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: output.as_entire_binding(),
                        },
                    ],
                })
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            if samples > 1 {
                // The separate opaque prepass does not contain the later body.
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("opaque prepass"),
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &single_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.05),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("late rigged faces"),
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: scene,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.05),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                for column in 0..4 {
                    pass.set_pipeline(if column == 3 { &one } else { &full });
                    pass.set_viewport(column as f32 * 4.0, 0.0, 4.0, 4.0, 0.0, 1.0);
                    pass.set_scissor_rect(column * 4, 0, 4, 4);
                    pass.draw(0..3, column..column + 1);
                }
            }
            for stage in 0..2 {
                if stage == 1
                    && let Some(resolve) = &resolver
                {
                    resolve.encode(&mut encoder, &single_view);
                }
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&probes[stage]);
                pass.set_bind_group(0, &groups[stage], &[]);
                pass.dispatch_workgroups(4, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 32);
            queue.submit([encoder.finish()]);
            readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("depth readback"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("depth GPU completion");
            let mapped = readback.get_mapped_range(..).expect("depth samples");
            let got: &[f32] = bytemuck::cast_slice(&mapped);
            let expected = [0.05, 0.05, 0.8, 0.8];
            for (i, want) in expected.into_iter().enumerate() {
                let old = if samples > 1 { 0.05 } else { want };
                assert!((got[i] - old).abs() < 1e-6, "{samples}x pre-resolve pixel{i}: {}", got[i]);
                assert!((got[4 + i] - want).abs() < 1e-6, "{samples}x final depth pixel{i}: {}", got[4 + i]);
            }
        }
    }
}
