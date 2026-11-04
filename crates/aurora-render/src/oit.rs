//! Order-independent transparency: the nearest layers of each pixel, kept
//! by sorted insertion.
//!
//! Blended faces are drawn back to front object by object, so two objects
//! that cross or interleave (hair over a dress, glass in glass, nested
//! particles of a build) show the wrong one in front wherever their order
//! flips inside a pixel. With this on, the scene's blended faces are drawn
//! twice into per-pixel arrays instead (object.wgsl): a key pass inserts
//! each fragment's depth into its pixel's `layers` keys, sorted nearest
//! first by a chain of atomic maxima, and a payload pass shades each
//! fragment once and stores its colour by its key, or sums it into the
//! pixel's tail when it is farther than the kept layers. A compute pass
//! (oit.wgsl) then composites each pixel of the tiles that got layers, and
//! two indirect draws put those tiles over the scene, the MSAA edge pixels
//! sample by sample. Nothing is sorted after the capture, nothing comes
//! from a shared pool, and no frame is drawn again: the work per fragment
//! is bounded by `layers`, the memory by the screen, and a still view gives
//! the same image every frame.
//!
//! A new Aurora feature: the Legacy Viewer sorts its alpha by object
//! (LLDrawPoolAlpha). Past `layers` layers in a pixel, the nearest are kept
//! and sorted, the farther ones folded into a weighted average behind them
//! (the tail: their colour weighted by their alpha, over the product of
//! their transparencies). Off, on a GPU without writable storage in
//! fragment shaders, or with MSAA 8x (a layer keeps 4 coverage bits), the
//! renderer draws exactly as before; turning it off frees the arrays
//! ([`Oit::release`]).
//!
//! Limit: where the device cannot force the capture's depth test before its
//! shader (`SHADER_EARLY_DEPTH_TEST`, Vulkan and GLES only in wgpu: not Metal
//! nor DX12), the fragments hidden by the opaque scene of an MSAA frame also
//! run both passes.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Layers kept per pixel ("Layers per pixel" in the preferences).
pub const DEFAULT_LAYERS: u32 = 16;
pub const MIN_LAYERS: u32 = 2;
/// (object.wgsl and oit.wgsl bound their loops by it)
pub const MAX_LAYERS: u32 = 24;
/// MSAA samples a layer's coverage holds (4 bits): beyond, the renderer
/// draws in the per-object order.
pub const MAX_SAMPLES: u32 = 4;
/// The whole OIT never takes more video memory than this: the keys, the
/// payloads, the tail, the composite texture and the tile list.
pub const MAX_BYTES: u64 = 512 << 20;
/// A layer's key (its depth) and payload (colour, alpha, coverage), per pixel.
const KEY_BYTES: u64 = 4;
const PAYLOAD_BYTES: u64 = 8;
/// The tail's five sums per pixel.
const TAIL_BYTES: u64 = 20;
/// The composite texture (`rgba16float`), per pixel.
const COMPOSITE_BYTES: u64 = 8;
/// Per screen tile: its two list entries, and its flag after the tail.
const TILE_LIST_BYTES: u64 = 8;
const TILE_FLAG_BYTES: u64 = 4;
/// Side of the resolve's screen tiles (oit.wgsl `TILE`, its workgroups).
const TILE: u32 = 8;
/// Storage buffers the capture's fragment stage reads: the five of every
/// blended face (main_thread.rs `FRAGMENT_STORAGE_BUFFERS`), the keys, the
/// payloads and the tail.
const CAPTURE_STORAGE_BUFFERS: u32 = 8;

/// oit.wgsl `OitParams` (object.wgsl declares the same words).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
struct Params {
    width: u32,
    height: u32,
    /// the screen's pixels: the stride between the layers' slices
    pixels: u32,
    layers: u32,
    samples: u32,
    tiles_x: u32,
    tiles: u32,
    _pad: u32,
}

/// Whether this device can capture: storage written from fragment shaders,
/// with three more storage buffers than the blended faces already read.
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

/// `AURORA_OIT_LAYERS=<n>`: the layers kept per pixel for this launch,
/// whatever the setting (docs/TESTING.md).
pub(crate) fn layers_override() -> Option<u32> {
    static OVERRIDE: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| std::env::var("AURORA_OIT_LAYERS").ok()?.trim().parse().ok())
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

/// oit.wgsl, its line ends normalised.
pub(crate) fn resolve_source() -> String {
    include_str!("shaders/oit.wgsl").replace("\r\n", "\n")
}

/// Video memory that does not depend on the layer count: the tail, the
/// composite texture, the tile list and the tiles' flags of a `width` x
/// `height` screen.
pub fn fixed_bytes(width: u32, height: u32) -> u64 {
    let pixels = u64::from(width.max(1)) * u64::from(height.max(1));
    let tiles = u64::from(width.max(1).div_ceil(TILE)) * u64::from(height.max(1).div_ceil(TILE));
    pixels * (TAIL_BYTES + COMPOSITE_BYTES) + tiles * (TILE_LIST_BYTES + TILE_FLAG_BYTES)
}

/// Bytes of the keys and payloads of `layers` per pixel.
pub fn layer_bytes(width: u32, height: u32, layers: u32) -> u64 {
    u64::from(width.max(1)) * u64::from(height.max(1)) * u64::from(layers) * (KEY_BYTES + PAYLOAD_BYTES)
}

/// The layers per pixel a screen gets: `wanted` (the setting), within
/// `MIN_LAYERS..=MAX_LAYERS`, reduced so that the whole OIT stays within
/// [`MAX_BYTES`] once what does not depend on it ([`fixed_bytes`]) comes
/// off, and so that each buffer fits `max_binding`. `None` when fewer than
/// `MIN_LAYERS` fit or the screen exceeds `max_dim`.
pub fn plan(width: u32, height: u32, wanted: u32, max_binding: u64, max_dim: u32) -> Option<u32> {
    if width > max_dim || height > max_dim {
        return None;
    }
    let pixels = u64::from(width.max(1)) * u64::from(height.max(1));
    let budget = MAX_BYTES.saturating_sub(fixed_bytes(width, height));
    let by_budget = budget / (pixels * (KEY_BYTES + PAYLOAD_BYTES));
    let by_binding = max_binding / (pixels * PAYLOAD_BYTES);
    let layers = u64::from(wanted.clamp(MIN_LAYERS, MAX_LAYERS)).min(by_budget).min(by_binding);
    (layers >= u64::from(MIN_LAYERS)).then_some(layers as u32)
}

struct Buffers {
    width: u32,
    height: u32,
    samples: u32,
    layers: u32,
    tiles_x: u32,
    tiles: u32,
    /// layer i of pixel p at `i * pixels + p`: its depth's bits (0: none;
    /// the top bit once its payload is stored, so a coplanar fragment's
    /// claim of the slot is seen by the others and by the resolve)
    keys: wgpu::Buffer,
    /// its colour as rgb9e5, then its alpha and its coverage
    payloads: wgpu::Buffer,
    /// five sums per pixel, then one flag per tile
    tail: wgpu::Buffer,
    /// each pixel's composite (an edge pixel's alpha: -1)
    color: wgpu::TextureView,
    /// two DrawIndirectArgs (plain tiles, edge tiles), then the tile lists
    args: wgpu::Buffer,
    tile_list: wgpu::Buffer,
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
    /// The capture's group 3 layout (both passes).
    pub fn capture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.capture_layout
    }

    /// `fs_glow_oit`'s group 3 layout (the layers read).
    pub fn glow_layout(&self) -> &wgpu::BindGroupLayout {
        &self.glow_layout
    }
}

pub(crate) struct Oit {
    shared: Shared,
    params: wgpu::Buffer,
    last_params: Option<Params>,
    buffers: Option<Buffers>,
    /// buffers that could not be allocated are tried again later
    retry: crate::gpu_alloc::Backoff,
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
        entry(9, stage, read),
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
    Resolve {
        samples,
        classify,
        composite: draw("oit composite", "fs_composite"),
        edge: ms.then(|| draw("oit composite msaa edges", "fs_edge")),
    }
}

impl Oit {
    /// `None` when the device cannot capture ([`supported`]).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> Option<Self> {
        if !supported(device) {
            return None;
        }
        use wgpu::BufferBindingType as B;
        let frag = wgpu::ShaderStages::FRAGMENT;
        let uniform = entry(22, frag, buffer(B::Uniform));
        // group 3 of the capture (object.wgsl bindings 20..23)
        let rw = buffer(B::Storage { read_only: false });
        let capture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit capture"),
            entries: &[entry(20, frag, rw), entry(21, frag, rw), uniform, entry(23, frag, rw)],
        });
        // The glow reads the layers in the pass that composites them, where
        // the composite binds them read only: a read-write binding of the
        // same buffer there would invalidate the whole pass (wgpu usage
        // scopes), so the glow has read-only bindings of its own.
        let read_only = buffer(B::Storage { read_only: true });
        let glow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit glow"),
            entries: &[entry(25, frag, read_only), entry(26, frag, read_only), uniform],
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
            buffers: None,
            retry: Default::default(),
        })
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// Makes the arrays ready for this frame. `depth_msaa`: the scene's
    /// MSAA depth (`None` without MSAA). `false` when they cannot be made
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
        layers: u32,
        depth_ss: &wgpu::TextureView,
        depth_msaa: Option<&wgpu::TextureView>,
    ) -> bool {
        let limits = device.limits();
        let max_binding = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let Some(layers) = plan(width, height, layers, max_binding, limits.max_texture_dimension_2d) else {
            if self.buffers.take().is_some() {
                log::warn!("oit: no room for {MIN_LAYERS} layers per pixel at {width}x{height}: blended faces sorted by object");
            }
            return false;
        };
        let stale = self
            .buffers
            .as_ref()
            .is_none_or(|b| b.width != width || b.height != height || b.samples != samples || b.layers != layers);
        if stale {
            self.buffers = None;
            let now = std::time::Instant::now();
            if !self.retry.ready(now) {
                return false;
            }
            match self.make_buffers(device, width, height, samples, layers) {
                Ok(buffers) => {
                    self.buffers = Some(buffers);
                    self.retry.succeeded();
                }
                Err(e) => {
                    log::warn!("oit: no buffers for {width}x{height} ({e}): blended faces sorted by object for now");
                    self.retry.failed(now);
                    return false;
                }
            }
        }
        let Some(buffers) = &self.buffers else {
            return false;
        };
        let depth = depth_msaa.unwrap_or(depth_ss);
        if buffers.groups.is_none() || buffers.groups_depth.as_ref() != Some(depth) {
            let groups = self.make_groups(device, buffers, depth_msaa);
            if let Some(buffers) = &mut self.buffers {
                buffers.groups = Some(groups);
                buffers.groups_depth = Some(depth.clone());
            }
        }
        let Some(buffers) = &self.buffers else {
            return false;
        };
        let params = Params {
            width,
            height,
            pixels: width * height,
            layers: buffers.layers,
            samples,
            tiles_x: buffers.tiles_x,
            tiles: buffers.tiles,
            _pad: 0,
        };
        if self.last_params != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
            self.last_params = Some(params);
        }
        // no tile listed yet: 6 vertices and no instance each; the edge
        // tiles' list starts at `tiles` (their first_instance)
        let args: [u32; 8] = [6, 0, 0, 0, 6, 0, 0, buffers.tiles];
        queue.write_buffer(&buffers.args, 0, bytemuck::cast_slice(&args));
        true
    }

    /// The arrays and the composite texture, or why one could not be made:
    /// every allocation checked (`gpu_alloc`), installed only whole.
    fn make_buffers(&self, device: &wgpu::Device, width: u32, height: u32, samples: u32, layers: u32) -> Result<Buffers, String> {
        let pixels = u64::from(width) * u64::from(height);
        let (tiles_x, tiles_y) = (width.div_ceil(TILE), height.div_ceil(TILE));
        let tiles = tiles_x * tiles_y;
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
        let keys = buffer("oit keys", pixels * u64::from(layers) * KEY_BYTES, U::STORAGE)?;
        let payloads = buffer("oit payloads", pixels * u64::from(layers) * PAYLOAD_BYTES, U::STORAGE)?;
        let tail = buffer("oit tail", pixels * TAIL_BYTES + u64::from(tiles) * TILE_FLAG_BYTES, U::STORAGE)?;
        let args = buffer("oit tile draws", 32, U::STORAGE | U::INDIRECT)?;
        let tile_list = buffer("oit tiles", u64::from(tiles) * TILE_LIST_BYTES, U::STORAGE)?;
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
            pixels * COMPOSITE_BYTES,
        )?;
        log::info!(
            "oit: {layers} layers per pixel ({} MB of keys and colours, {} MB in all) for {width}x{height}",
            layer_bytes(width, height, layers) >> 20,
            (layer_bytes(width, height, layers) + fixed_bytes(width, height)) >> 20
        );
        let capture = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit capture"),
            layout: &self.shared.capture_layout,
            entries: &[
                bind(20, keys.as_entire_binding()),
                bind(21, payloads.as_entire_binding()),
                bind(22, self.params.as_entire_binding()),
                bind(23, tail.as_entire_binding()),
            ],
        });
        let glow = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit glow"),
            layout: &self.shared.glow_layout,
            entries: &[
                bind(25, keys.as_entire_binding()),
                bind(26, payloads.as_entire_binding()),
                bind(22, self.params.as_entire_binding()),
            ],
        });
        Ok(Buffers {
            width,
            height,
            samples,
            layers,
            tiles_x,
            tiles,
            keys,
            payloads,
            tail,
            color,
            args,
            tile_list,
            capture,
            glow,
            groups: None,
            groups_depth: None,
        })
    }

    fn make_groups(
        &self,
        device: &wgpu::Device,
        b: &Buffers,
        depth_msaa: Option<&wgpu::TextureView>,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let ms = usize::from(depth_msaa.is_some());
        let mut classify = vec![
            bind(0, b.keys.as_entire_binding()),
            bind(1, b.payloads.as_entire_binding()),
            bind(2, self.params.as_entire_binding()),
            bind(9, b.tail.as_entire_binding()),
        ];
        let mut draws = classify.clone();
        if let Some(depth) = depth_msaa {
            classify.push(bind(3, wgpu::BindingResource::TextureView(depth)));
            draws.push(bind(3, wgpu::BindingResource::TextureView(depth)));
        }
        classify.push(bind(4, wgpu::BindingResource::TextureView(&b.color)));
        classify.push(bind(5, b.args.as_entire_binding()));
        classify.push(bind(6, b.tile_list.as_entire_binding()));
        draws.push(bind(7, wgpu::BindingResource::TextureView(&b.color)));
        draws.push(bind(8, b.tile_list.as_entire_binding()));
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

    /// The capture's group 3, once [`Oit::prepare`] made the arrays.
    pub fn capture_group(&self) -> Option<&wgpu::BindGroup> {
        self.buffers.as_ref().map(|b| &b.capture)
    }

    /// The glow's group 3 (the same layers, read only).
    pub fn glow_group(&self) -> Option<&wgpu::BindGroup> {
        self.buffers.as_ref().map(|b| &b.glow)
    }

    /// Empties the keys, the tail and the tiles' flags (before the capture);
    /// a payload is written for every kept key, so they need no clearing.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(b) = &self.buffers {
            encoder.clear_buffer(&b.keys, 0, None);
            encoder.clear_buffer(&b.tail, 0, None);
        }
    }

    /// Frees the arrays and the composite texture (up to 512 MB): the
    /// renderer calls it while OIT is off (as `Bloom::release`), since
    /// [`Oit::prepare`] runs only while it is on.
    pub fn release(&mut self) {
        if self.buffers.take().is_some() {
            log::info!("oit: off, layers freed");
        }
        self.last_params = None;
    }

    /// Composites each pixel of the tiles that got layers and lists those
    /// tiles (after the capture).
    pub fn classify(&self, encoder: &mut wgpu::CommandEncoder, r: &Resolve) {
        if let Some(b) = &self.buffers
            && let Some((group, _)) = &b.groups
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("oit classify"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&r.classify);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(b.tiles_x, b.tiles / b.tiles_x, 1);
        }
    }

    /// Draws the listed tiles over the scene (after [`Oit::classify`], in a
    /// pass with the scene colour and its depth read only): once per pixel,
    /// then the MSAA edge pixels sample by sample.
    pub fn resolve(&self, pass: &mut wgpu::RenderPass, r: &Resolve) {
        if let Some(b) = &self.buffers
            && let Some((_, group)) = &b.groups
        {
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(&r.composite);
            pass.draw_indirect(&b.args, 0);
            if let Some(edge) = &r.edge {
                pass.set_pipeline(edge);
                pass.draw_indirect(&b.args, 16);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: u64 = u64::MAX;

    /// Everything the OIT holds for a plan.
    fn total(layers: u32, w: u32, h: u32) -> u64 {
        layer_bytes(w, h, layers) + fixed_bytes(w, h)
    }

    /// The tail's rule, as oit.wgsl `tail_of` reads what
    /// `fs_blend_payloads` sums: the colours weighted by their alphas over a
    /// coverage that is the product of the transparencies, the same
    /// whatever the order, and what the old resolve folded.
    fn tail(fragments: &[([f32; 3], f32)]) -> ([f32; 3], f32) {
        let (mut sum, mut a, mut logt) = ([0.0f32; 3], 0.0f32, 0.0f32);
        for (rgb, alpha) in fragments {
            for i in 0..3 {
                sum[i] += rgb[i] * alpha;
            }
            a += alpha;
            logt += -(1.0 - alpha.min(254.0 / 255.0)).ln();
        }
        let cov = 1.0 - (-logt).exp();
        (sum.map(|s| s / a * cov), cov)
    }

    #[test]
    fn the_tail_is_the_weighted_average_behind_the_kept_layers() {
        let a = [([1.0, 0.0, 0.0], 0.5), ([0.0, 1.0, 0.0], 0.25), ([0.0, 0.0, 1.0], 0.75)];
        let b = [a[2], a[0], a[1]];
        let (ca, cova) = tail(&a);
        let (cb, covb) = tail(&b);
        assert!((cova - covb).abs() < 1e-6 && ca.iter().zip(cb).all(|(x, y)| (x - y).abs() < 1e-6));
        // the coverage is one minus the product of the transparencies
        assert!((cova - (1.0 - 0.5 * 0.75 * 0.25)).abs() < 1e-5, "{cova}");
        // a single fragment is itself
        let (c, cov) = tail(&[([0.2, 0.4, 0.6], 0.5)]);
        assert!((cov - 0.5).abs() < 1e-6 && (c[0] - 0.1).abs() < 1e-6 && (c[2] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn the_layers_follow_the_setting_within_the_budget() {
        // the default at 1600x900: 16 layers, 302 MB in all (binary megabytes,
        // like every size the viewer logs)
        assert_eq!(plan(1600, 900, 16, BIG, 16384), Some(16));
        let mb = total(16, 1600, 900) >> 20;
        assert!((300..=304).contains(&mb), "{mb} MB");
        // asked for more: within the bounds
        assert_eq!(plan(1600, 900, 100, BIG, 16384), Some(MAX_LAYERS));
        assert_eq!(plan(1600, 900, 0, BIG, 16384), Some(MIN_LAYERS));
        // larger screens get fewer, and the whole never passes the cap
        for (w, h, at_least) in [(1920, 1080, 12), (2560, 1440, 9), (3840, 2160, 2)] {
            let layers = plan(w, h, 16, BIG, 16384).expect("fits");
            assert!(layers >= at_least && layers <= 16, "{w}x{h}: {layers}");
            assert!(total(layers, w, h) <= MAX_BYTES, "{w}x{h}: {} MB", total(layers, w, h) >> 20);
            assert!(total(layers + 1, w, h) > MAX_BYTES || layers == 16, "{w}x{h}: one more would fit");
        }
        // a binding limit below the payloads wins
        assert_eq!(plan(1600, 900, 16, 1600 * 900 * 8 * 4, 16384), Some(4));
        // a screen larger than the textures, or too large for two layers
        assert_eq!(plan(5000, 100, 8, BIG, 4096), None);
        assert_eq!(plan(16384, 16384, 8, BIG, 16384), None);
    }

    #[test]
    fn the_shaders_are_valid() {
        let src = resolve_source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(&src));
        }
    }
}
