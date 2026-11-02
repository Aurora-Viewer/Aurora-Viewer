//! Live modal backdrop: scene and lower UI layers are blurred on the GPU;
//! the modal and higher layers are painted afterwards at full resolution.
//! This presentation is an explicit Aurora UI choice, requested by the user.

use crate::pipeline_cache::CachedPipelines;
use std::sync::Arc;

struct Boundary;

/// Place this before painting a modal on its layer. The marker survives
/// tessellation, so even meshes with the same texture cannot straddle it.
pub fn blurred_backdrop(ctx: &egui::Context, layer: egui::LayerId) {
    ctx.layer_painter(layer).add(egui::PaintCallback {
        rect: ctx.content_rect(),
        callback: Arc::new(Boundary),
    });
}

pub(crate) fn take_boundary(primitives: &mut Vec<egui::ClippedPrimitive>) -> Option<usize> {
    let at = primitives
        .iter()
        .position(|p| matches!(&p.primitive, egui::epaint::Primitive::Callback(c) if c.callback.is::<Boundary>()))?;
    primitives.remove(at);
    Some(at)
}

/// Step between two taps of the shader, in UI points.
const STEP: f32 = 3.0;
/// Farthest tap of the shader, in steps (shaders/ui_blur.wgsl).
const LAST_TAP: f32 = 3.230_769_2;

/// The rectangles of a blur limited to a part of the image, in pixels as
/// [x, y, width, height].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Part {
    /// Of the source: what the horizontal pass reads.
    pub source: [u32; 4],
    /// Of the half-size image: what the horizontal pass writes for the
    /// vertical one.
    half: [u32; 4],
    output: [u32; 4],
}

/// The rectangles to blur `area` of an image of `size` alone (`area` inside
/// the image, not empty): each pass writes what the next one reads, a tap
/// and the neighbors of the linear filter around.
fn part(area: [u32; 4], size: [u32; 2], scale: f32) -> Part {
    // farthest pixel of the source a pass reads from the one it writes: the
    // last tap, and the neighbor the filter mixes across the half-size image
    let reach = (LAST_TAP * STEP * scale).ceil() as u32 + 2;
    let grow = |r: [u32; 4], by: [u32; 2]| {
        let min = [r[0].saturating_sub(by[0]), r[1].saturating_sub(by[1])];
        let max = [(r[0] + r[2] + by[0]).min(size[0]), (r[1] + r[3] + by[1]).min(size[1])];
        [min[0], min[1], max[0] - min[0], max[1] - min[1]]
    };
    // the vertical pass reads the half-size image above and below
    let read = grow(area, [2, reach]);
    let min = [read[0] / 2, read[1] / 2];
    let max = [(read[0] + read[2]).div_ceil(2), (read[1] + read[3]).div_ceil(2)];
    let half = [min[0], min[1], max[0] - min[0], max[1] - min[1]];
    // the horizontal pass reads the source left and right
    let source = grow(half.map(|n| n * 2), [reach, 2]);
    Part {
        source,
        half,
        output: area,
    }
}

pub(crate) struct Blur {
    pub source: wgpu::Texture,
    pub view: wgpu::TextureView,
    size: [u32; 2],
    scale: f32,
    intermediate: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    groups: [wgpu::BindGroup; 2],
}

impl Blur {
    pub fn matches(&self, size: [u32; 2], scale: f32) -> bool {
        self.size == size && self.scale == scale
    }

    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2], scale: f32) -> Self {
        use wgpu::util::DeviceExt;
        let texture = |label, size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let source = texture("UI backdrop", size);
        let view = source.create_view(&Default::default());
        let intermediate = texture("UI backdrop horizontal", size.map(|n| n.div_ceil(2))).create_view(&Default::default());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UI backdrop"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("UI backdrop"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Constant radius in UI points, independent of window size or DPI.
        let groups = [
            (&view, [STEP * scale / size[0] as f32, 0.0, 0.0, 0.0]),
            (&intermediate, [0.0, STEP * scale / size[1] as f32, 0.0, 0.0]),
        ]
        .map(|(source, direction)| {
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("UI backdrop direction"),
                contents: bytemuck::cast_slice(&direction),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("UI backdrop"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            })
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI backdrop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ui_blur.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UI backdrop"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("UI backdrop"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            source,
            view,
            size,
            scale,
            intermediate,
            pipeline,
            groups,
        }
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView) {
        self.passes(encoder, output, None);
    }

    /// The rectangles to blur `area` of the output alone ([x, y, width,
    /// height] in pixels, inside the image and not empty).
    pub fn part(&self, area: [u32; 4]) -> Part {
        part(area, self.size, self.scale)
    }

    /// `encode` for a part of the output: `part.source` of the source holds
    /// the image to blur, the rest of the output keeps what it had.
    pub fn encode_part(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView, part: &Part) {
        self.passes(encoder, output, Some([part.half, part.output]));
    }

    fn passes(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView, scissors: Option<[[u32; 4]; 2]>) {
        let targets = [&self.intermediate, output].into_iter().zip(&self.groups);
        for (pass_index, (view, group)) in targets.enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI backdrop blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // a part: nothing is read outside what it writes
                        load: match scissors {
                            Some(_) => wgpu::LoadOp::Load,
                            None => wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            if let Some(scissors) = scissors {
                let [x, y, width, height] = scissors[pass_index];
                pass.set_scissor_rect(x, y, width, height);
            }
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tessellation_keeps_lower_layers_before_the_blur_and_modal_layers_after_it() {
        let ctx = egui::Context::default();
        let modal = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("modal"));
        let mut output = ctx.run_ui(Default::default(), |_| {
            // Call order deliberately differs from layer order.
            blurred_backdrop(&ctx, modal);
            ctx.layer_painter(modal).rect_filled(
                egui::Rect::from_min_size(egui::pos2(20.0, 0.0), egui::vec2(20.0, 20.0)),
                0.0,
                egui::Color32::GREEN,
            );
            ctx.layer_painter(egui::LayerId::background()).rect_filled(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 20.0)),
                0.0,
                egui::Color32::RED,
            );
        });
        // No GPU renderer in this test: discard texture uploads explicitly.
        output.textures_delta.clear();
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let boundary = take_boundary(&mut primitives).expect("blur marker survives tessellation");
        let colors = |jobs: &[egui::ClippedPrimitive]| {
            jobs.iter()
                .flat_map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
                    _ => Vec::new(),
                })
                .collect::<Vec<_>>()
        };
        assert!(colors(&primitives[..boundary]).contains(&egui::Color32::RED));
        assert!(!colors(&primitives[..boundary]).contains(&egui::Color32::GREEN));
        assert!(colors(&primitives[boundary..]).contains(&egui::Color32::GREEN));
        assert!(!colors(&primitives[boundary..]).contains(&egui::Color32::RED));
    }

    /// [x, y, width, height] to (left, top, right, bottom).
    fn edges(r: [u32; 4]) -> [u32; 4] {
        [r[0], r[1], r[0] + r[2], r[1] + r[3]]
    }

    fn contains(outer: [u32; 4], inner: [u32; 4]) -> bool {
        let (o, i) = (edges(outer), edges(inner));
        o[0] <= i[0] && o[1] <= i[1] && o[2] >= i[2] && o[3] >= i[3]
    }

    #[test]
    fn a_partial_blur_writes_all_that_its_next_pass_reads() {
        let size = [1921, 1081];
        let image = [0, 0, size[0], size[1]];
        for scale in [1.0, 1.5, 2.0] {
            // a tap, and the texel the linear filter mixes with it
            let tap = (LAST_TAP * STEP * scale).ceil() as u32 + 1;
            for area in [[300, 200, 180, 40], [0, 0, 64, 20], [1800, 1040, 121, 41], image] {
                let p = part(area, size, scale);
                assert_eq!(p.output, area);
                assert!(contains([0, 0, size[0].div_ceil(2), size[1].div_ceil(2)], p.half), "{p:?}");
                assert!(contains(image, p.source), "{p:?}");
                assert!(p.half[2] > 0 && p.half[3] > 0);
                // the vertical pass reads above and below the area, unless
                // the image ends there
                let a = edges(area);
                let read = [a[0], a[1].saturating_sub(tap), a[2], (a[3] + tap).min(size[1])];
                let w = edges(p.half.map(|n| n * 2));
                assert!(
                    w[0] <= read[0] && w[1] <= read[1] && w[2] >= read[2] && w[3] >= read[3],
                    "{scale} {area:?}"
                );
                // the horizontal pass reads left and right of what it writes
                let s = edges(p.source);
                let read = [w[0].saturating_sub(tap), w[1], (w[2] + tap).min(size[0]), w[3].min(size[1])];
                assert!(
                    s[0] <= read[0] && s[1] <= read[1] && s[2] >= read[2] && s[3] >= read[3],
                    "{scale} {area:?}"
                );
            }
        }
    }

    #[test]
    fn the_blur_of_a_small_part_reads_little_more_than_it() {
        let p = part([300, 200, 180, 40], [1920, 1080], 1.0);
        assert!(p.source[2] < 240 && p.source[3] < 80, "{p:?}");
    }
}
