//! Order-independent transparency: per-pixel linked lists.
//!
//! Blended faces are drawn back to front object by object, so two objects
//! that cross or interleave (hair over a dress, glass in glass, nested
//! particles of a build) show the wrong one in front wherever their order
//! flips inside a pixel. With this on, the scene's blended faces are drawn
//! once into a pool of nodes (`fs_blend_oit`, object.wgsl): each fragment in
//! front of the opaque scene becomes a node of its pixel's list. A
//! full-screen pass (oit.wgsl) then sorts each pixel's layers, each MSAA
//! sample on its own, and composites them over the scene.
//!
//! A new Aurora feature: the Legacy Viewer sorts its alpha by object
//! (LLDrawPoolAlpha). What does not fit changes nothing: when the pool is
//! full, the fragments left are blended in the drawing order as before; past
//! `max_layers` layers in one pixel, the farthest are folded into a weighted
//! average. Off, the renderer draws exactly as before.

use bytemuck::{Pod, Zeroable};

/// Blended fragments per screen pixel the pool holds on average.
pub const DEFAULT_NODES_PER_PIXEL: u32 = 8;
pub const MIN_NODES_PER_PIXEL: u32 = 2;
pub const MAX_NODES_PER_PIXEL: u32 = 32;
/// Layers sorted exactly in one pixel (the shader's arrays hold
/// `MAX_SORTED_LAYERS`, oit.wgsl `MAX_LAYERS`).
pub const DEFAULT_SORTED_LAYERS: u32 = 24;
pub const MAX_SORTED_LAYERS: u32 = 32;
/// The node pool never takes more video memory than this.
pub const MAX_POOL_BYTES: u64 = 512 << 20;
/// A node: premultiplied colour, alpha and coverage, depth, next.
const NODE_BYTES: u64 = 16;

/// Storage buffers the capture's fragment stage reads: the five of every
/// blended face (main_thread.rs `FRAGMENT_STORAGE_BUFFERS`), the heads and
/// the nodes.
const CAPTURE_STORAGE_BUFFERS: u32 = 7;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
struct Params {
    node_cap: u32,
    width: u32,
    height: u32,
    max_layers: u32,
}

/// Whether this device can capture: storage written from fragment shaders,
/// with two more storage buffers than the blended faces already read. Off on
/// macOS for now: not yet checked on Apple GPUs.
pub(crate) fn supported(device: &wgpu::Device) -> bool {
    !cfg!(target_os = "macos") && device.limits().max_storage_buffers_per_shader_stage >= CAPTURE_STORAGE_BUFFERS
}

/// `AURORA_OIT=0|1` turns it off or on whatever the setting (before / after
/// captures, docs/TESTING.md).
pub(crate) fn env_override() -> Option<bool> {
    static OVERRIDE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| match std::env::var("AURORA_OIT").ok()?.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    })
}

/// Nodes the pool holds for a `width` × `height` screen: `nodes_per_pixel`
/// per pixel, within `MAX_POOL_BYTES` and the device's buffer limits.
pub fn node_capacity(width: u32, height: u32, nodes_per_pixel: u32, max_binding: u64) -> u32 {
    let wanted =
        u64::from(width.max(1)) * u64::from(height.max(1)) * u64::from(nodes_per_pixel.clamp(MIN_NODES_PER_PIXEL, MAX_NODES_PER_PIXEL));
    let bytes = (wanted * NODE_BYTES).min(MAX_POOL_BYTES).min(max_binding);
    u32::try_from(bytes / NODE_BYTES).unwrap_or(u32::MAX)
}

struct Pool {
    width: u32,
    height: u32,
    node_cap: u32,
    nodes_per_pixel: u32,
    /// heads[0] counts the nodes, then one head per pixel (0: empty)
    heads: wgpu::Buffer,
    nodes: wgpu::Buffer,
    capture: wgpu::BindGroup,
    /// the resolve's group, and the MSAA depth it reads
    resolve: wgpu::BindGroup,
    resolve_depth: Option<wgpu::TextureView>,
}

pub(crate) struct Oit {
    capture_layout: wgpu::BindGroupLayout,
    /// single-sample, then MSAA (with the scene depth)
    resolve_layouts: [wgpu::BindGroupLayout; 2],
    resolve_pipeline_layouts: [wgpu::PipelineLayout; 2],
    module: wgpu::ShaderModule,
    /// the resolve for one sample count
    resolve: Option<(u32, wgpu::RenderPipeline)>,
    params: wgpu::Buffer,
    last_params: Option<Params>,
    pool: Option<Pool>,
    hdr_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Oit {
    /// `None` when the device cannot capture ([`supported`]).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> Option<Self> {
        if !supported(device) {
            return None;
        }
        // group 3 of the capture (object.wgsl bindings 20..22)
        let capture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit capture"),
            entries: &[storage(20, false), storage(21, false), uniform(22)],
        });
        let resolve_ss = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit resolve"),
            entries: &[storage(0, true), storage(1, true), uniform(2)],
        });
        let resolve_ms = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit resolve msaa"),
            entries: &[
                storage(0, true),
                storage(1, true),
                uniform(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                    count: None,
                },
            ],
        });
        let pl = |label, layout: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            })
        };
        let resolve_pipeline_layouts = [pl("oit resolve", &resolve_ss), pl("oit resolve msaa", &resolve_ms)];
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oit resolve"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/oit.wgsl").into()),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit parameters"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Oit {
            capture_layout,
            resolve_layouts: [resolve_ss, resolve_ms],
            resolve_pipeline_layouts,
            module,
            resolve: None,
            params,
            last_params: None,
            pool: None,
            hdr_format,
            depth_format,
        })
    }

    /// The capture pipeline's group 3 layout.
    pub fn capture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.capture_layout
    }

    fn resolve_pipeline(&self, device: &wgpu::Device, samples: u32) -> wgpu::RenderPipeline {
        let ms = samples > 1;
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(if ms { "oit resolve msaa" } else { "oit resolve" }),
            layout: Some(&self.resolve_pipeline_layouts[usize::from(ms)]),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_full"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(if ms { "fs_resolve_ms" } else { "fs_resolve" }),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.hdr_format,
                    // premultiplied over the scene; the glow in the scene
                    // alpha dimmed by the coverage (LLDrawPoolAlpha)
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            // the scene depth stays attached, read only
            depth_stencil: Some(wgpu::DepthStencilState {
                format: self.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
    }

    /// Makes the pool and the resolve ready for this frame. `depth_msaa`:
    /// the scene's MSAA depth (`None` without MSAA). `false` when the pool
    /// cannot be made: the faces are then blended as before.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        samples: u32,
        nodes_per_pixel: u32,
        max_layers: u32,
        depth_msaa: Option<&wgpu::TextureView>,
    ) -> bool {
        if self.resolve.as_ref().is_none_or(|(n, _)| *n != samples) {
            self.resolve = Some((samples, self.resolve_pipeline(device, samples)));
        }
        let limits = device.limits();
        let max_binding = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let nodes_per_pixel = nodes_per_pixel.clamp(MIN_NODES_PER_PIXEL, MAX_NODES_PER_PIXEL);
        let stale = self
            .pool
            .as_ref()
            .is_none_or(|p| p.width != width || p.height != height || p.nodes_per_pixel != nodes_per_pixel);
        if stale {
            self.pool = None;
            let node_cap = node_capacity(width, height, nodes_per_pixel, max_binding);
            let heads_bytes = (1 + u64::from(width) * u64::from(height)) * 4;
            if node_cap == 0 || heads_bytes > max_binding {
                return false;
            }
            let heads = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("oit heads"),
                size: heads_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let nodes = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("oit nodes"),
                size: u64::from(node_cap) * NODE_BYTES,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            log::info!(
                "oit: {node_cap} nodes ({} MB) for {width}x{height}, {nodes_per_pixel} per pixel",
                (u64::from(node_cap) * NODE_BYTES) >> 20
            );
            let capture = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("oit capture"),
                layout: &self.capture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 20,
                        resource: heads.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 21,
                        resource: nodes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 22,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            });
            let resolve = self.resolve_group(device, &heads, &nodes, depth_msaa);
            self.pool = Some(Pool {
                width,
                height,
                node_cap,
                nodes_per_pixel,
                heads,
                nodes,
                capture,
                resolve,
                resolve_depth: depth_msaa.cloned(),
            });
        }
        let Some(pool) = &self.pool else {
            return false;
        };
        if pool.resolve_depth.as_ref() != depth_msaa {
            let resolve = self.resolve_group(device, &pool.heads, &pool.nodes, depth_msaa);
            if let Some(pool) = &mut self.pool {
                pool.resolve = resolve;
                pool.resolve_depth = depth_msaa.cloned();
            }
        }
        let Some(pool) = &self.pool else {
            return false;
        };
        let params = Params {
            node_cap: pool.node_cap,
            width,
            height,
            max_layers: max_layers.clamp(1, MAX_SORTED_LAYERS),
        };
        if self.last_params != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
            self.last_params = Some(params);
        }
        true
    }

    /// The capture's group 3, once [`Oit::prepare`] made the pool.
    pub fn capture_group(&self) -> Option<&wgpu::BindGroup> {
        self.pool.as_ref().map(|p| &p.capture)
    }

    fn resolve_group(
        &self,
        device: &wgpu::Device,
        heads: &wgpu::Buffer,
        nodes: &wgpu::Buffer,
        depth_msaa: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: heads.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: nodes.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: self.params.as_entire_binding(),
            },
        ];
        if let Some(depth) = depth_msaa {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(depth),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit resolve"),
            layout: &self.resolve_layouts[usize::from(depth_msaa.is_some())],
            entries: &entries,
        })
    }

    /// Empties the lists (before the capture).
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(pool) = &self.pool {
            encoder.clear_buffer(&pool.heads, 0, None);
        }
    }

    /// Composites the lists over the scene (after the capture, in a pass
    /// with the scene colour and its depth read only).
    pub fn resolve(&self, pass: &mut wgpu::RenderPass) {
        if let (Some((_, pipe)), Some(pool)) = (&self.resolve, &self.pool) {
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &pool.resolve, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_follows_the_screen_and_stays_within_its_budget() {
        // 1080p, 8 per pixel: 16.6 M nodes would be 253 MB
        assert_eq!(node_capacity(1920, 1080, 8, u64::MAX), 1920 * 1080 * 8);
        // 4K at 8 per pixel would be 1 GB: capped to 512 MB
        assert_eq!(u64::from(node_capacity(3840, 2160, 8, u64::MAX)) * NODE_BYTES, MAX_POOL_BYTES);
        // a device binding limit below the budget wins
        assert_eq!(node_capacity(1920, 1080, 8, 128 << 20), (128 << 20) / 16);
        // the per-pixel count is kept in its range
        assert_eq!(node_capacity(100, 100, 0, u64::MAX), 100 * 100 * MIN_NODES_PER_PIXEL);
        assert_eq!(node_capacity(100, 100, 1000, u64::MAX), 100 * 100 * MAX_NODES_PER_PIXEL);
    }

    #[test]
    fn the_shaders_are_valid() {
        let src = include_str!("shaders/oit.wgsl");
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(src));
        }
    }
}
