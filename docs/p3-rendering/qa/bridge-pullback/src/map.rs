//! Offscreen transition fixtures using the viewer's extracted visibility WGSL.
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use serde_json::{Value, json};
use wgpu::util::DeviceExt;

include!("production_fit.rs");
fn tile() -> u32 {
    std::env::var("MAP_SIZE")
        .unwrap_or_else(|_| "1024".into())
        .parse()
        .unwrap()
}
type V4 = [f32; 4];

#[repr(C)]
#[derive(Pod, Zeroable, Copy, Clone)]
struct Frame {
    base: [[f32; 16]; 3],
    detail: [f32; 16],
    detail_params: V4,
    splits: V4,
    flags: [u32; 4],
    controls: [u32; 4],
    eye: V4,
    params: V4,
    misc: V4,
    plane: V4,
    portrait: [f32; 16],
    portrait_depth: V4,
}
#[repr(C)]
#[derive(Pod, Zeroable, Copy, Clone)]
struct Probe {
    position: V4,
    normal: V4,
}
pub(super) struct Gpu {
    dev: wgpu::Device,
    q: wgpu::Queue,
    raster: wgpu::RenderPipeline,
    lookup: wgpu::ComputePipeline,
    atlas: wgpu::TextureView,
    atlas_base: wgpu::TextureView,
    remote: wgpu::TextureView,
    portrait: wgpu::TextureView,
    detail: wgpu::TextureView,
    sampler: wgpu::Sampler,
}
impl Gpu {
    pub(super) fn new() -> (Self, Value) {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(desc);
        let a = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let info = a.get_info();
        let mut device_info = json!({"adapter":info.name,"driver":info.driver,"backend":format!("{:?}",info.backend),"type":format!("{:?}",info.device_type)});
        let limits = a.limits();
        let mut requested = wgpu::DeviceDescriptor::default();
        requested.required_limits.max_texture_dimension_2d = limits.max_texture_dimension_2d;
        device_info["atlas_dims"] = json!([
            2 * tile(),
            (2 * tile() + tile() / 2).min(limits.max_texture_dimension_2d)
        ]);
        let (dev, q) = pollster::block_on(a.request_device(&requested)).unwrap();
        let m = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orthographic caster fixture"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
                @group(0) @binding(0) var<uniform> vp:mat4x4<f32>;
                struct V { @builtin(position) p:vec4<f32>, @location(0) w:vec4<f32>, };
                @vertex fn vs(@location(0) v:vec4<f32>) -> V {
                    return V(vp*vec4<f32>(v.xyz,1.0),v);
                }
                @fragment fn fs(i:V) {
                    if(i.w.w>0.0 && abs(i.w.x)<i.w.w){discard;}
                }
            "#
                .into(),
            ),
        });
        let raster = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("curved receiver and opaque metal bridge"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &m,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &m,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        let cm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production shadow_factor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lookup.wgsl").into()),
        });
        let lookup = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("read visibility"),
            layout: None,
            module: &cm,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
        let depth = |w, h, levels| {
            dev.create_texture(&wgpu::TextureDescriptor {
                label: Some("shadow depth"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let atlas_texture = depth(
            2 * tile(),
            (2 * tile() + tile() / 2).min(limits.max_texture_dimension_2d),
            2,
        );
        let portrait = depth(PORTRAIT_SIZE, PORTRAIT_SIZE, 1).create_view(&Default::default());
        let atlas = atlas_texture.create_view(&Default::default());
        let single_level = |level| {
            atlas_texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let atlas_base = single_level(0);
        let remote = single_level(1);
        let detail = depth(tile(), tile(), 1).create_view(&Default::default());
        let sampler = dev.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bilinear shadow compare"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        (
            Self {
                dev,
                q,
                raster,
                lookup,
                atlas,
                atlas_base,
                remote,
                portrait,
                detail,
                sampler,
            },
            device_info,
        )
    }
    fn buffer(&self, b: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.dev
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: b,
                usage,
            })
    }
    fn run(&self, f: &Frame, verts: &[V4], probes: &[Probe]) -> Vec<V4> {
        let matrices = [f.detail, f.base[0], f.base[1], f.base[2], f.portrait];
        let local_cap: f32 = 2.;
        let padding: f32 = 400.;
        let overlap: f32 = 1.;
        let split_for = |m: &[f32; 16], remote: bool| {
            let scale = Vec3::new(m[2], m[6], m[10]).length();
            ((padding - local_cap + if remote { overlap } else { 0. }) * scale)
                .clamp(0.000001, 0.999999)
        };
        let groups = |remote: bool| {
            matrices.map(|mut m| {
                {
                    let split = split_for(&m, remote);
                    for i in 0..4 {
                        let z = 4 * i + 2;
                        m[z] = if remote {
                            m[z] / split
                        } else {
                            (m[z] - split * m[z + 1]) / (1. - split)
                        };
                    }
                }
                let uniform = self.buffer(bytemuck::cast_slice(&m), wgpu::BufferUsages::UNIFORM);
                self.dev.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.raster.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                })
            })
        };
        let local_groups = groups(false);
        let far_groups = groups(true);
        let vb = self.buffer(bytemuck::cast_slice(verts), wgpu::BufferUsages::VERTEX);
        let mut enc = self.dev.create_command_encoder(&Default::default());
        let mut render = |target: &wgpu::TextureView,
                          bindings: &[wgpu::BindGroup; 5],
                          slots: &[(usize, u32, u32)],
                          size: u32,
                          distant: bool| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("raster actual clipped caster depths"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                multiview_mask: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.raster);
            pass.set_vertex_buffer(0, vb.slice(..));
            for &(index, x, y) in slots {
                let split = split_for(&matrices[index], distant);
                let (depth_min, depth_max) = if distant { (0., split) } else { (split, 1.) };
                pass.set_viewport(
                    (x * size) as f32,
                    (y * size) as f32,
                    size as f32,
                    size as f32,
                    depth_min,
                    depth_max,
                );
                pass.set_scissor_rect(x * size, y * size, size, size);
                pass.set_bind_group(0, &bindings[index], &[]);
                pass.draw(0..verts.len() as u32, 0..1);
            }
        };
        render(&self.detail, &local_groups, &[(0, 0, 0)], tile(), false);
        render(
            &self.portrait,
            &local_groups,
            &[(4, 0, 0)],
            PORTRAIT_SIZE,
            false,
        );
        render(
            &self.atlas_base,
            &local_groups,
            &[(1, 0, 0), (2, 1, 0), (3, 0, 1)],
            tile(),
            false,
        );
        render(
            &self.remote,
            &far_groups,
            &[(1, 0, 0), (2, 1, 0), (3, 0, 1), (0, 1, 1)],
            tile() / 2,
            true,
        );
        let mut uniform_frame = *f;
        uniform_frame.portrait_depth[0] = split_for(&f.detail, false);
        let angle: f32 = std::env::var("SUN_DIAMETER_DEG").unwrap().parse().unwrap();
        uniform_frame.detail_params[3] = (angle.to_radians() * 0.5).tan();

        let uniform = self.buffer(
            bytemuck::bytes_of(&uniform_frame),
            wgpu::BufferUsages::UNIFORM,
        );
        let input = self.buffer(bytemuck::cast_slice(probes), wgpu::BufferUsages::STORAGE);
        let output = self.buffer(
            bytemuck::cast_slice(&vec![[-123f32; 4]; probes.len()]),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let read = self.dev.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (probes.len() * 16) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entries = [
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&self.atlas),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 29,
                resource: wgpu::BindingResource::TextureView(&self.detail),
            },
            wgpu::BindGroupEntry {
                binding: 30,
                resource: wgpu::BindingResource::TextureView(&self.portrait),
            },
        ];
        let bg = self.dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.lookup.get_bind_group_layout(0),
            entries: &entries,
        });
        {
            let mut cp = enc.begin_compute_pass(&Default::default());
            cp.set_pipeline(&self.lookup);
            cp.set_bind_group(0, &bg, &[]);
            cp.dispatch_workgroups((probes.len() as u32).div_ceil(64), 1, 1);
        }
        enc.copy_buffer_to_buffer(&output, 0, &read, 0, (probes.len() * 16) as u64);
        self.q.submit([enc.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        self.dev.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let result = bytemuck::allocation::pod_collect_to_vec::<u8, V4>(
            &read.slice(..).get_mapped_range().unwrap(),
        );
        read.unmap();
        assert!(
            result
                .iter()
                .flatten()
                .all(|x| x.is_finite() && (0.0..=1.0).contains(x)),
            "each probe must return finite in-range visibility"
        );
        result
    }
}

// The fitted extent uses the same sphere radius and 1/16-m quantization as
// the viewer. This fixture uses a 16:10 camera and 60° vertical FOV.
// Viewer culling, draw routing and terrain are outside this reference fit.

fn snapped(center: Vec3, radius: f32, sun: Vec3) -> Mat4 {
    let pad: f32 = 400.;
    let up = if sun.z.abs() > 0.99 { Vec3::Y } else { Vec3::Z };
    let light = glam::camera::rh::view::look_at_mat4(center + sun * (radius + pad), center, up);
    let metres = 2. * radius / tile() as f32;
    let origin = light.transform_point3((center / radius).floor() * radius);
    let diff = Vec3::new(
        (origin.x / metres).round() * metres - origin.x,
        (origin.y / metres).round() * metres - origin.y,
        0.,
    );
    glam::camera::rh::proj::directx::orthographic(
        -radius,
        radius,
        -radius,
        radius,
        0.1,
        2. * (radius + pad),
    ) * Mat4::from_translation(diff)
        * light
}

impl Gpu {
    pub(super) fn evaluate_normals(
        &self,
        eye: Vec3,
        focus: Vec3,
        proj: Mat4,
        viewport: (u32, u32),
        sun: Vec3,
        ranges: (f32, f32),
        verts: &[[f32; 4]],
        points: &[Vec3],
        normals: &[Vec3],
        contact: &[u8],
    ) -> Vec<[f32; 4]> {
        let (detail, far) = ranges;
        let ratio = (far / detail).cbrt();
        let splits = [detail * ratio, far / ratio, far];
        let direction = (focus - eye).normalize();
        let th = 1. / proj.y_axis.y;
        let tw = 1. / proj.x_axis.x;
        let corner_cos = 1. / (1. + tw * tw + th * th).sqrt();
        let mut maps = [[0.; 16]; 3];
        let mut close_texel = 0.;
        for i in 0..3 {
            let start = match i {
                0 => 0.01,
                1 => 0.85 * splits[0] * corner_cos,
                _ => (splits[1] - 0.15 * (splits[1] - splits[0])) * corner_cos,
            };
            let end = splits[i];
            let center = eye + direction * ((start + end) * 0.5);
            let radius =
                (((end * tw).powi(2) + (end * th).powi(2) + ((end - start) * 0.5).powi(2))
                    .sqrt()
                    .max(1.)
                    * 16.)
                    .ceil()
                    / 16.;
            let (center, radius) = if i == 0 && tile() <= 2048 {
                let (axis, r) = graded_detail_bounds(proj, viewport, end, tile(), 0.10);
                (eye + direction * axis, (r * 16.).ceil() / 16.)
            } else {
                (center, radius)
            };
            maps[i] = snapped(center, radius, sun).to_cols_array();
            if i == 0 {
                close_texel = 2. * radius / tile() as f32;
            }
        }
        let (axis, radius) = graded_detail_bounds(
            proj,
            viewport,
            detail,
            tile(),
            (0.5 * close_texel).min(0.10),
        );
        let radius = (radius * 16.).ceil() / 16.;
        let parent = snapped(eye + direction * axis, radius, sun);
        let portrait = portrait_projection(
            parent,
            tile(),
            glam::camera::rh::view::look_at_mat4(eye, eye + direction, Vec3::Y),
        )
        .unwrap_or(Mat4::ZERO);
        let frame = Frame {
            base: maps,
            detail: parent.to_cols_array(),
            detail_params: [detail, 1., 0., 0.],
            splits: [splits[0], splits[1], splits[2], 1.],
            flags: [0; 4],
            controls: [0, 32, 0, 1],
            eye: eye.extend(1.).to_array(),
            params: [1.; 4],
            misc: [0.; 4],
            plane: [0.; 4],
            portrait: portrait.to_cols_array(),
            portrait_depth: [0.; 4],
        };
        let probes: Vec<_> = points
            .iter()
            .zip(normals)
            .zip(contact)
            .map(|((p, n), contact)| Probe {
                position: p.extend(f32::from(*contact) / 255.).to_array(),
                normal: n.extend(1.).to_array(),
            })
            .collect();
        self.run(&frame, verts, &probes)
    }
}
