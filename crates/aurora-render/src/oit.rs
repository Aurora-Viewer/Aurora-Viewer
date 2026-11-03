//! Order-independent transparency: a slot array plus per-pixel linked lists.
//!
//! Blended faces are drawn back to front object by object, so two objects
//! that cross or interleave (hair over a dress, glass in glass, nested
//! particles of a build) show the wrong one in front wherever their order
//! flips inside a pixel. With this on, the scene's blended faces are drawn
//! once (`fs_blend_oit`, object.wgsl): each pixel's first `slots` fragments
//! in front of the opaque scene go to slot k of a texture array (layer k
//! holds every pixel's slot k, so neighbours write side by side and these
//! fragments never touch the pool's shared counter); the following ones
//! become nodes of the pixel's linked list in the pool, so the result stays
//! exact. A compute pass (oit.wgsl) then sorts each pixel's layers once
//! (the tiles the capture flagged only) and lists the screen tiles that
//! hold some; two indirect draws composite those tiles over the scene, the
//! MSAA edge pixels sample by sample.
//!
//! A new Aurora feature: the Legacy Viewer sorts its alpha by object
//! (LLDrawPoolAlpha). What does not fit changes nothing: when the pool is
//! full, the classify skips the composite and the blended faces are drawn
//! again exactly as without OIT, rigged ones by their `RiggedBlendDepth`
//! mode, its depth copy included (all decided on the GPU: the gated draws
//! read copies of their arguments, oit_gate.rs, no wait on the CPU); past
//! `max_layers` layers in one pixel, the nearest are kept and sorted, the
//! farthest folded into a weighted average behind them. Off, or when the
//! pool cannot be allocated, the renderer draws exactly as before; turning
//! it off frees the pool ([`Oit::release`]).
//!
//! Limit: where the device cannot force the capture's depth test before its
//! shader (`SHADER_EARLY_DEPTH_TEST`, Vulkan and GLES only in wgpu: not Metal
//! nor DX12), the fragments hidden by the opaque scene of an MSAA frame also
//! reach the pool, which then fills faster.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Blended fragments per screen pixel the pool holds on average.
pub const DEFAULT_NODES_PER_PIXEL: u32 = 8;
pub const MIN_NODES_PER_PIXEL: u32 = 2;
pub const MAX_NODES_PER_PIXEL: u32 = 32;
/// Layers sorted exactly in one pixel (oit.wgsl sorts up to 24: `resolve_24`).
pub const DEFAULT_SORTED_LAYERS: u32 = 24;
pub const MAX_SORTED_LAYERS: u32 = 24;
/// The whole OIT never takes more video memory than this: the slot array,
/// the node pool, the heads, the composite texture and the tile list.
pub const MAX_POOL_BYTES: u64 = 512 << 20;
/// Bytes per pixel of the heads (a fragment count and a list head, 2 words).
const HEADS_BYTES: u64 = 8;
/// Bytes per pixel of the composite texture (`rgba16float`).
const COMPOSITE_BYTES: u64 = 8;
/// Bytes per screen tile of the tile list (a plain and an edge entry).
const TILE_LIST_BYTES: u64 = 8;
/// Bytes per screen tile of its flag in the heads (set by the capture).
const TILE_FLAG_BYTES: u64 = 4;
/// A slot (a texel of the array) has the size of a node.
const SLOT_BYTES: u64 = 16;
/// Slots per pixel at most: past that the array costs more than it saves.
const MAX_SLOTS: u32 = 8;
/// A node: premultiplied colour, alpha and coverage, depth, next.
const NODE_BYTES: u64 = 16;
/// Side of the resolve's screen tiles (oit.wgsl `TILE`, its workgroups).
const TILE: u32 = 8;

/// Storage buffers the capture's fragment stage reads: the five of every
/// blended face (main_thread.rs `FRAGMENT_STORAGE_BUFFERS`), the heads and
/// the nodes.
const CAPTURE_STORAGE_BUFFERS: u32 = 7;

/// oit.wgsl `OitParams` (object.wgsl declares the same words).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
struct Params {
    node_cap: u32,
    width: u32,
    height: u32,
    max_layers: u32,
    samples: u32,
    tiles_x: u32,
    tiles: u32,
    /// fragments per pixel kept in the slot array (the rest go to the list)
    slots: u32,
    /// first word of the tiles' flags in the heads (one per tile, set by
    /// the capture: the classify skips the tiles without any)
    tile_base: u32,
    _pad: [u32; 3],
}

/// The pool's `flags` buffer: the DrawIndirectArgs of the replay's depth
/// copy (one triangle when the frame is replayed, none otherwise, set by the
/// classify), whose instance count also tells oit_gate.wgsl which gated
/// draws keep their instances; then the overflow of the frame and the
/// hold-off that follows one (oit_gate.wgsl), kept across the frames.
const DEPTH_COPY_ARGS: u64 = 0;
const FLAGS_BYTES: u64 = 32;

/// Whether this device can capture: storage written from fragment shaders,
/// with two more storage buffers than the blended faces already read. The
/// overflow's replay and the composite are gated with plain indirect draws
/// (oit_gate.rs), so Metal, which has no counted draws, captures too.
pub(crate) fn supported(device: &wgpu::Device) -> bool {
    device.limits().max_storage_buffers_per_shader_stage >= CAPTURE_STORAGE_BUFFERS
        && device.limits().max_storage_textures_per_shader_stage >= 1
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

/// `AURORA_OIT_NODES=<n>`: the pool's layers per pixel for this launch,
/// whatever the setting (docs/TESTING.md). At its lowest (2) a scene with
/// many layers fills the pool, to check the in-order replay.
pub(crate) fn nodes_override() -> Option<u32> {
    static OVERRIDE: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| std::env::var("AURORA_OIT_NODES").ok()?.trim().parse().ok())
}

/// object.wgsl for this device: the capture's depth test forced before its
/// shader when the device can (`Features::SHADER_EARLY_DEPTH_TEST`; the
/// attribute is refused otherwise, and with it the whole module).
pub(crate) fn object_source(device: &wgpu::Device, src: &str) -> String {
    if early_depth(device) {
        src.replace("/*OIT_EARLY_DEPTH*/", "@early_depth_test(force)")
    } else {
        src.to_string()
    }
}

/// The capture's depth test can be forced before its shader.
pub(crate) fn early_depth(device: &wgpu::Device) -> bool {
    device.features().contains(wgpu::Features::SHADER_EARLY_DEPTH_TEST)
}

/// oit.wgsl with its sort generated for 8 and 24 layers: two array sizes
/// that stay in registers, a pixel taking the larger only when it needs it.
pub(crate) fn resolve_source() -> String {
    let src = include_str!("shaders/oit.wgsl").replace("\r\n", "\n");
    let (Some(start), Some(end)) = (src.find("// <resolve_N>"), src.find("// </resolve_N>")) else {
        return src;
    };
    let template = &src[start..end];
    let sorts: String = ["8", "24"].iter().map(|n| template.replace("__N__", n)).collect();
    format!("{}{}{}", &src[..start], sorts, &src[end..])
}

/// How a pixel budget (`nodes_per_pixel`, the "Layers kept per pixel"
/// setting) is split between the slot array and the node pool.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Plan {
    /// slot-array layers (fragments per pixel stored without the pool)
    pub slots: u32,
    /// nodes in the pool
    pub node_cap: u32,
}

/// Video memory that does not depend on the layer counts: the heads (with
/// the tiles' flags), the composite texture and the tile list of a `width`
/// x `height` screen.
pub fn fixed_bytes(width: u32, height: u32) -> u64 {
    let pixels = u64::from(width.max(1)) * u64::from(height.max(1));
    let tiles = u64::from(width.max(1).div_ceil(TILE)) * u64::from(height.max(1).div_ceil(TILE));
    pixels * (HEADS_BYTES + COMPOSITE_BYTES) + tiles * (TILE_LIST_BYTES + TILE_FLAG_BYTES) + FLAGS_BYTES
}

/// Splits the budget: half of the per-pixel count goes to the slots (1 to
/// [`MAX_SLOTS`]), the rest stays in the pool on average. The whole OIT stays
/// within `MAX_POOL_BYTES`: what the heads and the composite take
/// ([`fixed_bytes`]) comes off first, the slots are reduced when they alone
/// would take more than half of what is left (4K screens) and by the
/// device's layer limit; the pool gets the rest of the budget within
/// `max_binding`. `None` when no slot fits or the screen exceeds `max_dim`.
pub fn plan(width: u32, height: u32, nodes_per_pixel: u32, max_binding: u64, max_layers: u32, max_dim: u32) -> Option<Plan> {
    if width > max_dim || height > max_dim {
        return None;
    }
    let pixels = u64::from(width.max(1)) * u64::from(height.max(1));
    let per_pixel = nodes_per_pixel.clamp(MIN_NODES_PER_PIXEL, MAX_NODES_PER_PIXEL);
    let budget = MAX_POOL_BYTES.saturating_sub(fixed_bytes(width, height));
    let by_budget = u32::try_from((budget / 2) / (pixels * SLOT_BYTES)).unwrap_or(u32::MAX);
    let slots = (per_pixel / 2).clamp(1, MAX_SLOTS).min(by_budget).min(max_layers);
    if slots == 0 {
        return None;
    }
    let slot_bytes = pixels * SLOT_BYTES * u64::from(slots);
    let wanted = pixels * u64::from(per_pixel - slots);
    let bytes = (wanted * NODE_BYTES).min(budget.saturating_sub(slot_bytes)).min(max_binding);
    let node_cap = u32::try_from(bytes / NODE_BYTES).unwrap_or(u32::MAX);
    Some(Plan { slots, node_cap })
}

struct Pool {
    width: u32,
    height: u32,
    node_cap: u32,
    slots: u32,
    nodes_per_pixel: u32,
    tiles_x: u32,
    tiles: u32,
    /// heads[0] counts the nodes; pixel p: heads[1 + 2p] counts its
    /// fragments, heads[2 + 2p] is its list's head + 1 (0: none)
    heads: wgpu::Buffer,
    /// the first `slots` fragments of each pixel (layer k: slot k of all)
    slots_view: wgpu::TextureView,
    /// keeps the slot array alive
    _slots_tex: wgpu::Texture,
    nodes: wgpu::Buffer,
    /// each pixel's composite (an edge pixel's alpha: -1)
    color: wgpu::TextureView,
    /// two DrawIndirectArgs (plain tiles, edge tiles), then the tile lists
    args: wgpu::Buffer,
    tile_list: wgpu::Buffer,
    /// the draw counts at REPLAY_FLAG and COMPOSITED_FLAG, set by the classify
    flags: wgpu::Buffer,
    capture: wgpu::BindGroup,
    glow: wgpu::BindGroup,
    /// the classify's and the draws' groups, and the MSAA depth they read
    groups: Option<(wgpu::BindGroup, wgpu::BindGroup)>,
    groups_depth: Option<wgpu::TextureView>,
}

/// The resolve's pipelines for one sample count, compiled with the scene's
/// (renderer.rs `make_pipelines`, on the pipeline worker).
pub(crate) struct Resolve {
    samples: u32,
    classify: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    /// per sample, over the edge tiles (MSAA only)
    edge: Option<wgpu::RenderPipeline>,
    /// the scene depth into another depth target, when the pool overflowed
    /// ([`Oit::copy_depth_if_overflowed`])
    depth_copy: wgpu::RenderPipeline,
}

impl Resolve {
    pub fn samples(&self) -> u32 {
        self.samples
    }
}

/// What the pipelines are made from, handed to the pipeline worker.
#[derive(Clone)]
pub(crate) struct Shared {
    capture_layout: wgpu::BindGroupLayout,
    glow_layout: wgpu::BindGroupLayout,
    /// the classify's and the draws', single-sample then MSAA
    classify_layouts: [wgpu::BindGroupLayout; 2],
    draw_layouts: [wgpu::BindGroupLayout; 2],
    module: wgpu::ShaderModule,
    hdr_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
}

impl Shared {
    /// The capture's group 3 layout (the slot array written).
    pub fn capture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.capture_layout
    }

    /// `fs_glow_oit`'s group 3 layout (the slot array read).
    pub fn glow_layout(&self) -> &wgpu::BindGroupLayout {
        &self.glow_layout
    }
}

pub(crate) struct Oit {
    shared: Shared,
    params: wgpu::Buffer,
    last_params: Option<Params>,
    pool: Option<Pool>,
    /// a pool that could not be allocated is tried again later
    retry: crate::gpu_alloc::Backoff,
    /// the gated draws' copies of their arguments (oit_gate.rs)
    gate: crate::oit_gate::Gate,
}

fn entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

fn buffer(ty: wgpu::BufferBindingType) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

/// The slot array read as a sampled texture (loads only).
fn slots_texture() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Uint,
        view_dimension: wgpu::TextureViewDimension::D2Array,
        multisampled: false,
    }
}

fn bind(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

/// The resolve's group layout (oit.wgsl bindings): `classify` the compute
/// step's, otherwise the draws'; `ms` with the scene's MSAA depth.
fn resolve_layout(device: &wgpu::Device, classify: bool, ms: bool) -> wgpu::BindGroupLayout {
    use wgpu::ShaderStages as S;
    let read = buffer(wgpu::BufferBindingType::Storage { read_only: true });
    let stage = if classify { S::COMPUTE } else { S::FRAGMENT };
    let mut entries = vec![
        entry(0, stage, read),
        entry(1, stage, read),
        entry(2, S::VERTEX_FRAGMENT | S::COMPUTE, buffer(wgpu::BufferBindingType::Uniform)),
    ];
    if ms {
        entries.push(entry(
            3,
            stage,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: true,
            },
        ));
    }
    // the slot array (oit.wgsl `slots_r`)
    entries.push(entry(10, stage, slots_texture()));
    if classify {
        let rw = buffer(wgpu::BufferBindingType::Storage { read_only: false });
        entries.push(entry(
            4,
            S::COMPUTE,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        ));
        entries.push(entry(5, S::COMPUTE, rw));
        entries.push(entry(6, S::COMPUTE, rw));
        entries.push(entry(9, S::COMPUTE, rw));
    } else {
        entries.push(entry(
            7,
            S::FRAGMENT,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        ));
        entries.push(entry(8, S::VERTEX, read));
        if !ms {
            entries.push(entry(
                11,
                S::FRAGMENT,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            ));
        }
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(match (classify, ms) {
            (true, false) => "oit classify",
            (true, true) => "oit classify msaa",
            (false, false) => "oit composite",
            (false, true) => "oit composite msaa",
        }),
        entries: &entries,
    })
}

/// The resolve's pipelines for `samples` (called by the pipeline worker).
pub(crate) fn make_resolve(device: &wgpu::Device, shared: &Shared, samples: u32) -> Resolve {
    let ms = samples > 1;
    let pl = |label, layout: &wgpu::BindGroupLayout| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        })
    };
    let classify = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("oit classify"),
        layout: Some(&pl("oit classify", &shared.classify_layouts[usize::from(ms)])),
        module: &shared.module,
        entry_point: Some(if ms { "cs_classify_ms" } else { "cs_classify" }),
        compilation_options: Default::default(),
        cache: None,
    });
    let draw_layout = pl("oit composite", &shared.draw_layouts[usize::from(ms)]);
    let draw = |label, fs| {
        device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&draw_layout),
            vertex: wgpu::VertexState {
                module: &shared.module,
                entry_point: Some("vs_tile"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shared.module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: shared.hdr_format,
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
                format: shared.depth_format,
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
    };
    let depth_copy = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("oit replay depth copy"),
        layout: Some(&draw_layout),
        vertex: wgpu::VertexState {
            module: &shared.module,
            entry_point: Some("vs_full"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shared.module,
            entry_point: Some(if ms { "fs_copy_depth_ms" } else { "fs_copy_depth" }),
            compilation_options: Default::default(),
            targets: &[],
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: shared.depth_format,
            depth_write_enabled: Some(true),
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
    });
    Resolve {
        samples,
        classify,
        composite: draw("oit composite", "fs_composite"),
        edge: ms.then(|| draw("oit composite msaa edges", "fs_edge")),
        depth_copy,
    }
}

impl Oit {
    /// `None` when the device cannot capture ([`supported`]).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> Option<Self> {
        if !supported(device) {
            return None;
        }
        use wgpu::BufferBindingType as B;
        // group 3 of the capture (object.wgsl bindings 20..23) and of the
        // glow (20..22 and 24): a write-only storage texture cannot be read,
        // so the slot array has a layout entry for each use
        let frag = wgpu::ShaderStages::FRAGMENT;
        let common = [
            entry(20, frag, buffer(B::Storage { read_only: false })),
            entry(21, frag, buffer(B::Storage { read_only: false })),
            entry(22, frag, buffer(B::Uniform)),
        ];
        let capture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit capture"),
            entries: &[
                common[0],
                common[1],
                common[2],
                entry(
                    23,
                    frag,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                ),
            ],
        });
        // The glow reads the lists in the pass that composites them, where
        // the composite binds them read only: a read-write binding of the
        // same buffer there would invalidate the whole pass (wgpu usage
        // scopes), so the glow has read-only bindings of its own.
        let read_only = buffer(B::Storage { read_only: true });
        let glow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit glow"),
            entries: &[
                entry(25, frag, read_only),
                entry(26, frag, read_only),
                common[2],
                entry(24, frag, slots_texture()),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oit resolve"),
            source: wgpu::ShaderSource::Wgsl(resolve_source().into()),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit parameters"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Oit {
            shared: Shared {
                capture_layout,
                glow_layout,
                classify_layouts: [resolve_layout(device, true, false), resolve_layout(device, true, true)],
                draw_layouts: [resolve_layout(device, false, false), resolve_layout(device, false, true)],
                module,
                hdr_format,
                depth_format,
            },
            params,
            last_params: None,
            pool: None,
            retry: Default::default(),
            gate: crate::oit_gate::Gate::new(device),
        })
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// Makes the pool ready for this frame. `depth_msaa`: the scene's MSAA
    /// depth (`None` without MSAA). `false` when the pool cannot be made
    /// (video memory full: tried again after a while, `gpu_alloc::Backoff`):
    /// the faces are then blended as before.
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
        depth_ss: &wgpu::TextureView,
        depth_msaa: Option<&wgpu::TextureView>,
        indirect: &wgpu::Buffer,
        gated: [(u64, u32); crate::oit_gate::SPANS],
    ) -> bool {
        let limits = device.limits();
        let max_binding = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let nodes_per_pixel = nodes_per_pixel.clamp(MIN_NODES_PER_PIXEL, MAX_NODES_PER_PIXEL);
        let stale = self
            .pool
            .as_ref()
            .is_none_or(|p| p.width != width || p.height != height || p.nodes_per_pixel != nodes_per_pixel);
        if stale {
            self.pool = None;
            let now = std::time::Instant::now();
            if !self.retry.ready(now) {
                return false;
            }
            match self.make_pool(device, width, height, nodes_per_pixel, max_binding) {
                Ok(pool) => {
                    self.pool = Some(pool);
                    self.retry.succeeded();
                }
                Err(e) => {
                    log::warn!("oit: no pool for {width}x{height} ({e}): blended faces sorted by object for now");
                    self.retry.failed(now);
                    return false;
                }
            }
        }
        let Some(pool) = &self.pool else {
            return false;
        };
        let depth = depth_msaa.unwrap_or(depth_ss);
        if pool.groups.is_none() || pool.groups_depth.as_ref() != Some(depth) {
            let groups = self.make_groups(device, pool, depth_ss, depth_msaa);
            if let Some(pool) = &mut self.pool {
                pool.groups = Some(groups);
                pool.groups_depth = Some(depth.clone());
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
            samples,
            tiles_x: pool.tiles_x,
            tiles: pool.tiles,
            slots: pool.slots,
            tile_base: 1 + 2 * width * height,
            _pad: [0; 3],
        };
        if self.last_params != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
            self.last_params = Some(params);
        }
        // no tile listed yet: 6 vertices and no instance each; the edge
        // tiles' list starts at `tiles` (their first_instance)
        let args: [u32; 8] = [6, 0, 0, 0, 6, 0, 0, pool.tiles];
        queue.write_buffer(&pool.args, 0, bytemuck::cast_slice(&args));
        let flags = pool.flags.clone();
        self.gate.prepare(device, queue, indirect, &flags, gated)
    }

    /// The pool's buffers and texture, or why one could not be made: every
    /// allocation checked (`gpu_alloc`), the pool installed only whole.
    fn make_pool(&self, device: &wgpu::Device, width: u32, height: u32, nodes_per_pixel: u32, max_binding: u64) -> Result<Pool, String> {
        let limits = device.limits();
        let Some(Plan { slots, node_cap }) = plan(
            width,
            height,
            nodes_per_pixel,
            max_binding,
            limits.max_texture_array_layers,
            limits.max_texture_dimension_2d,
        ) else {
            return Err("larger than the device's textures".to_owned());
        };
        let (tiles_x, tiles_y) = (width.div_ceil(TILE), height.div_ceil(TILE));
        let tiles = tiles_x * tiles_y;
        // the node counter, two words per pixel (its fragment count and
        // its list's head), then one flag per tile
        let heads_bytes = (1 + 2 * u64::from(width) * u64::from(height)) * 4 + u64::from(tiles) * TILE_FLAG_BYTES;
        if node_cap == 0 || heads_bytes > max_binding {
            return Err("larger than the device's storage buffers".to_owned());
        }
        // (COPY_DST on all: `try_buffer` checks a buffer by clearing it)
        let buffer = |label, size, usage| {
            crate::gpu_alloc::try_buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            )
        };
        use wgpu::BufferUsages as U;
        let heads = buffer("oit heads", heads_bytes, U::STORAGE)?;
        let nodes = buffer("oit nodes", u64::from(node_cap) * NODE_BYTES, U::STORAGE)?;
        let args = buffer("oit tile draws", 32, U::STORAGE | U::INDIRECT)?;
        let tile_list = buffer("oit tiles", u64::from(tiles) * 2 * 4, U::STORAGE)?;
        let flags = buffer("oit draw flags", FLAGS_BYTES, U::STORAGE | U::INDIRECT)?;
        let (_, color) = crate::gpu_alloc::try_texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("oit composite"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            &Default::default(),
            u64::from(width) * u64::from(height) * 8,
        )?;
        let slots_bytes = u64::from(width) * u64::from(height) * u64::from(slots) * SLOT_BYTES;
        let (slots_tex, slots_view) = crate::gpu_alloc::try_texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("oit slots"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: slots,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Uint,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            // an array view even for one layer (the default would be plain 2D)
            &wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            },
            slots_bytes,
        )?;
        log::info!(
            "oit: {slots} slots ({} MB) + {node_cap} nodes ({} MB) for {width}x{height}, {nodes_per_pixel} per pixel",
            slots_bytes >> 20,
            (u64::from(node_cap) * NODE_BYTES) >> 20
        );
        let capture = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit capture"),
            layout: &self.shared.capture_layout,
            entries: &[
                bind(20, heads.as_entire_binding()),
                bind(21, nodes.as_entire_binding()),
                bind(22, self.params.as_entire_binding()),
                bind(23, wgpu::BindingResource::TextureView(&slots_view)),
            ],
        });
        let glow = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit glow"),
            layout: &self.shared.glow_layout,
            entries: &[
                bind(25, heads.as_entire_binding()),
                bind(26, nodes.as_entire_binding()),
                bind(22, self.params.as_entire_binding()),
                bind(24, wgpu::BindingResource::TextureView(&slots_view)),
            ],
        });
        Ok(Pool {
            width,
            height,
            node_cap,
            slots,
            nodes_per_pixel,
            tiles_x,
            tiles,
            heads,
            slots_view,
            _slots_tex: slots_tex,
            nodes,
            color,
            args,
            tile_list,
            flags,
            capture,
            glow,
            groups: None,
            groups_depth: None,
        })
    }

    fn make_groups(
        &self,
        device: &wgpu::Device,
        pool: &Pool,
        depth_ss: &wgpu::TextureView,
        depth_msaa: Option<&wgpu::TextureView>,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let ms = usize::from(depth_msaa.is_some());
        let mut classify = vec![
            bind(0, pool.heads.as_entire_binding()),
            bind(1, pool.nodes.as_entire_binding()),
            bind(2, self.params.as_entire_binding()),
        ];
        classify.push(bind(10, wgpu::BindingResource::TextureView(&pool.slots_view)));
        let mut draws = classify.clone();
        if let Some(depth) = depth_msaa {
            classify.push(bind(3, wgpu::BindingResource::TextureView(depth)));
            draws.push(bind(3, wgpu::BindingResource::TextureView(depth)));
        }
        classify.push(bind(4, wgpu::BindingResource::TextureView(&pool.color)));
        classify.push(bind(5, pool.args.as_entire_binding()));
        classify.push(bind(6, pool.tile_list.as_entire_binding()));
        classify.push(bind(9, pool.flags.as_entire_binding()));
        draws.push(bind(7, wgpu::BindingResource::TextureView(&pool.color)));
        draws.push(bind(8, pool.tile_list.as_entire_binding()));
        if depth_msaa.is_none() {
            draws.push(bind(11, wgpu::BindingResource::TextureView(depth_ss)));
        }
        let group = |label, layout, entries: &[wgpu::BindGroupEntry<'_>]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries,
            })
        };
        (
            group("oit classify", &self.shared.classify_layouts[ms], &classify),
            group("oit composite", &self.shared.draw_layouts[ms], &draws),
        )
    }

    /// The capture's group 3, once [`Oit::prepare`] made the pool.
    pub fn capture_group(&self) -> Option<&wgpu::BindGroup> {
        self.pool.as_ref().map(|p| &p.capture)
    }

    /// The glow's group 3 (the same lists, the slot array read).
    pub fn glow_group(&self) -> Option<&wgpu::BindGroup> {
        self.pool.as_ref().map(|p| &p.glow)
    }

    /// Empties the lists (before the capture).
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(pool) = &self.pool {
            encoder.clear_buffer(&pool.heads, 0, None);
        }
    }

    /// Frees the pool, its heads and its composite texture (up to 512 MB):
    /// the renderer calls it while OIT is off (as `Bloom::release`), since
    /// [`Oit::prepare`] runs, and so drops a stale pool, only while it is on.
    pub fn release(&mut self) {
        if self.pool.take().is_some() {
            log::info!("oit: off, pool freed");
        }
        self.last_params = None;
    }

    /// Before the capture: the copy of its gated draws, without instances
    /// while the hold-off after an overflow runs (oit_gate.rs).
    pub fn gate_capture(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.pool.is_some() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("oit capture gate"),
                timestamp_writes: None,
            });
            self.gate.dispatch_capture(&mut pass);
        }
    }

    /// Sorts the lists and lists the tiles to composite (after the capture);
    /// when the pool overflowed, or the hold-off after one skipped the
    /// capture, lists none and sets the flags for the replay.
    pub fn classify(&self, encoder: &mut wgpu::CommandEncoder, r: &Resolve) {
        if let Some(pool) = &self.pool
            && let Some((group, _)) = &pool.groups
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("oit classify"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&r.classify);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(pool.tiles_x, pool.tiles / pool.tiles_x, 1);
            // the gated draws' arguments, from the flags it just set
            self.gate.dispatch(&mut pass);
        }
    }

    /// Where a gated draw reads its arguments: the draws `offset..` of the
    /// indirect buffer (`count` of them) in the copy drawn when the pool
    /// overflowed (`Replay`) or when it did not (`Composited`). `None`: not a
    /// gated range this frame, nothing to draw.
    pub fn gated(&self, copy: crate::oit_gate::Copy, offset: u64, count: u32) -> Option<(&wgpu::Buffer, u64)> {
        self.gate.args(copy, offset, count)
    }

    /// When the pool overflowed (decided on the GPU by [`Oit::classify`]),
    /// the scene depth into `target` (same size and samples): the replay of
    /// the rigged faces' `Layered` and `Layers` modes draws on that copy, as
    /// they do without OIT after a texture copy. Otherwise nothing is drawn.
    pub fn copy_depth_if_overflowed(&self, encoder: &mut wgpu::CommandEncoder, r: &Resolve, target: &wgpu::TextureView) {
        if let Some(pool) = &self.pool
            && let Some((_, group)) = &pool.groups
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("oit replay depth copy"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&r.depth_copy);
            pass.set_bind_group(0, group, &[]);
            pass.draw_indirect(&pool.flags, DEPTH_COPY_ARGS);
        }
    }

    /// Composites the listed tiles over the scene (after [`Oit::classify`],
    /// in a pass with the scene colour and its depth read only): once per
    /// pixel, then the MSAA edge pixels sample by sample.
    pub fn resolve(&self, pass: &mut wgpu::RenderPass, r: &Resolve) {
        if let Some(pool) = &self.pool
            && let Some((_, group)) = &pool.groups
        {
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(&r.composite);
            pass.draw_indirect(&pool.args, 0);
            if let Some(edge) = &r.edge {
                pass.set_pipeline(edge);
                pass.draw_indirect(&pool.args, 16);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: u64 = u64::MAX;

    /// Everything the OIT holds for a plan: slots, nodes, heads, composite,
    /// tile list (what `make_pool` allocates).
    fn total(p: Plan, w: u32, h: u32) -> u64 {
        u64::from(w) * u64::from(h) * u64::from(p.slots) * SLOT_BYTES + u64::from(p.node_cap) * NODE_BYTES + fixed_bytes(w, h)
    }

    /// The selection rule of oit.wgsl `resolve_N`, on the layers' depths in
    /// draw order (reverse-Z: the nearest has the largest depth): the indices
    /// of the `keep` nearest, and of the others, which the shader folds into
    /// the tail. A layer as far as the farthest kept goes to the tail.
    fn nearest(depths: &[f32], keep: usize) -> (Vec<usize>, Vec<usize>) {
        let (mut kept, mut tail): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        for (i, &z) in depths.iter().enumerate() {
            if kept.len() < keep {
                kept.push(i);
                continue;
            }
            let far = (0..kept.len()).fold(0, |far, k| if depths[kept[k]] < depths[kept[far]] { k } else { far });
            if z > depths[kept[far]] {
                tail.push(kept[far]);
                kept[far] = i;
            } else {
                tail.push(i);
            }
        }
        (kept, tail)
    }

    #[test]
    fn the_nearest_layers_are_kept_whatever_the_draw_order() {
        // 30 layers drawn nearest first, then farthest first, then shuffled
        let ascending: Vec<f32> = (1..=30).map(|i| i as f32 / 31.0).collect();
        let descending: Vec<f32> = ascending.iter().rev().copied().collect();
        let shuffled: Vec<f32> = (0..30).map(|i| ascending[(i * 7) % 30]).collect();
        for depths in [ascending, descending, shuffled] {
            let (kept, tail) = nearest(&depths, 24);
            let mut kept_z: Vec<f32> = kept.iter().map(|&i| depths[i]).collect();
            kept_z.sort_by(f32::total_cmp);
            assert_eq!(kept_z.len(), 24);
            let farthest_kept = kept_z[0];
            assert!(
                tail.iter().all(|&i| depths[i] < farthest_kept),
                "nothing in the tail is nearer than a kept layer"
            );
            assert_eq!(tail.len(), 6);
        }
    }

    #[test]
    fn one_sorted_layer_is_the_nearest_not_the_first_or_the_last() {
        let depths = [0.5, 0.9, 0.2, 0.7];
        let (kept, tail) = nearest(&depths, 1);
        assert_eq!(kept, vec![1]);
        assert_eq!(tail.len(), 3);
        // equal depths: the first stays
        assert_eq!(nearest(&[0.4, 0.4], 1).0, vec![0]);
        // fewer layers than the limit: all kept, no tail
        let (kept, tail) = nearest(&[0.1, 0.2], 24);
        assert_eq!((kept.len(), tail.len()), (2, 0));
    }

    #[test]
    fn the_budget_splits_between_slots_and_pool() {
        // 1080p, 8 per pixel: 4 slots and 4 nodes per pixel
        let p = plan(1920, 1080, 8, BIG, 2048, 16384).unwrap();
        assert_eq!(p.slots, 4);
        assert_eq!(p.node_cap, 1920 * 1080 * 4);
        // the default at 1600x900: 198 MB in all, heads and composite included
        let p = plan(1600, 900, 8, BIG, 2048, 16384).unwrap();
        assert_eq!((p.slots, p.node_cap), (4, 1600 * 900 * 4));
        let mb = total(p, 1600, 900) >> 20;
        assert!((197..=199).contains(&mb), "{mb} MB");
    }

    #[test]
    fn the_heads_and_the_composite_count_in_the_budget() {
        // 16 bytes per pixel before the first slot, plus the tile list and
        // the tiles' flags
        assert_eq!(fixed_bytes(1600, 900), 1600 * 900 * 16 + 200 * 113 * 12 + FLAGS_BYTES);
        // however large the budget wanted, the total never passes the cap
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160), (7680, 4320)] {
            for per_pixel in [2, 8, 16, 32] {
                if let Some(p) = plan(w, h, per_pixel, BIG, 2048, 16384) {
                    assert!(
                        total(p, w, h) <= MAX_POOL_BYTES,
                        "{w}x{h} at {per_pixel}: {} MB",
                        total(p, w, h) >> 20
                    );
                }
            }
        }
    }

    #[test]
    fn a_4k_screen_reduces_the_slots_and_stays_within_the_budget() {
        let p = plan(3840, 2160, 8, BIG, 2048, 16384).unwrap();
        assert_eq!(p.slots, 1, "2 slots and the heads would pass half of what is left");
        assert!(total(p, 3840, 2160) <= MAX_POOL_BYTES);
        assert!(total(p, 3840, 2160) > (500 << 20), "the rest of the budget goes to the pool");
        let p = plan(3840, 2160, 32, BIG, 2048, 16384).unwrap();
        assert!(p.slots >= 1 && total(p, 3840, 2160) <= MAX_POOL_BYTES);
    }

    #[test]
    fn the_slot_count_stays_in_its_bounds() {
        assert_eq!(plan(100, 100, 2, BIG, 2048, 16384).unwrap().slots, 1);
        assert_eq!(plan(100, 100, 0, BIG, 2048, 16384).unwrap().slots, 1);
        assert_eq!(plan(100, 100, 32, BIG, 2048, 16384).unwrap().slots, MAX_SLOTS);
        assert_eq!(plan(100, 100, 1000, BIG, 2048, 16384).unwrap().slots, MAX_SLOTS);
        // the device's layer limit
        assert_eq!(plan(100, 100, 32, BIG, 3, 16384).unwrap().slots, 3);
        // the smallest budget keeps one node per pixel in the pool
        assert_eq!(plan(100, 100, 2, BIG, 2048, 16384).unwrap().node_cap, 100 * 100);
    }

    #[test]
    fn the_device_limits_are_respected() {
        // a binding limit below the pool wins
        let p = plan(1920, 1080, 8, 64 << 20, 2048, 16384).unwrap();
        assert_eq!(p.node_cap, (64 << 20) / 16);
        // a screen larger than the textures: no plan
        assert_eq!(plan(5000, 100, 8, BIG, 2048, 4096), None);
        // no layer allowed
        assert_eq!(plan(100, 100, 8, BIG, 0, 16384), None);
    }

    #[test]
    fn the_shaders_are_valid() {
        let src = resolve_source();
        assert!(
            src.contains("fn resolve_8(") && src.contains("fn resolve_24("),
            "both sorts generated"
        );
        assert!(!src.contains("__N__"));
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(&src));
        }
    }
}
