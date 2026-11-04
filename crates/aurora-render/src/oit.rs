//! Order-independent transparency: a slot array plus per-pixel linked
//! lists, captured in bands of rows planned from a count of the fragments.
//!
//! Blended faces are drawn back to front object by object, so two objects
//! that cross or interleave (hair over a dress, glass in glass, nested
//! particles of a build) show the wrong one in front wherever their order
//! flips inside a pixel. With this on, the scene's blended faces are first
//! rasterised unshaded to count each pixel's fragments in front of the
//! opaque scene (`fs_count_oit`, object.wgsl), and a compute pass cuts the
//! screen into bands of tile rows whose fragments fit the node pool
//! (oit.wgsl `cs_band_plan`, on the GPU: no wait). Then, band by band, the
//! blended faces are drawn once on the band's depth mask (a copy of the
//! scene depth with the nearest depth outside the band's rows, oit.wgsl
//! `fs_band_depth`: the hardware rejects the other rows before the shader,
//! and the geometry is not touched, which clipping it to the rows would,
//! doubling or losing the pixels on the triangles' shared edges), and each
//! pixel's first `slots` fragments go to slot k of a texture array (layer
//! k holds every pixel's slot k, so neighbours write side by side and
//! these fragments never touch the pool's shared counter); the following
//! ones become nodes of the pixel's linked list in the pool. A compute
//! pass (oit.wgsl) sorts each pixel's layers once and lists the band's
//! 8 x 8 tiles that hold some; an indirect draw composites those tiles over
//! the scene, once per pixel, at the depth of the pixel's nearest layer.
//! The pool is reused by each band, so the memory is bounded whatever the
//! depth complexity, no frame overflows it and every fragment is captured,
//! sorted and composited exactly, each face shaded once. No sample mask is
//! read anywhere: with MSAA a layer counts where it is in front of at
//! least half of its pixel's samples, the composite's depth test keeps it
//! off the samples an opaque edge covers in front of every layer, and the
//! transparent edges take their anti-aliasing from the post-process one.
//!
//! A new Aurora feature: the Legacy Viewer sorts its alpha by object
//! (LLDrawPoolAlpha). Past `max_layers` layers in one pixel, the nearest
//! are kept and sorted, the farthest folded into a weighted average behind
//! them. Layers are sorted by depth then by a tie-break stored with each
//! (24 bits of the face record's identity, `types::oit_tie`: the object's
//! id and the face's index hashed), so coplanar faces keep one order from
//! frame to frame and from launch to launch, whatever order their fragments
//! arrive in. Off, or when the pool cannot be allocated, the renderer draws
//! exactly as before; turning it off frees the pool ([`Oit::release`]).
//!
//! Limits: where the device cannot force the capture's depth test before
//! its shader (`SHADER_EARLY_DEPTH_TEST`, Vulkan and GLES only in wgpu: not
//! Metal nor DX12), the fragments hidden by the opaque scene of an MSAA
//! frame also reach the pool, which then needs more bands. A frame needing
//! more than [`MAX_BANDS`] bands has the rows left over merged into its
//! last band, and a single tile row needing more than the pool is a band of
//! its own: in both the capture is bounded by the pool and the fragments
//! past it are left out (the `oit layers:` line of `AURORA_OIT_STATS` tells).

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Blended fragments per screen pixel the slot array and the pool hold on
/// average: half of it as slots, the rest the pool's size when
/// `AURORA_OIT_NODES` sets it (the pool takes the rest of the budget
/// otherwise, [`plan`]).
pub const DEFAULT_NODES_PER_PIXEL: u32 = 8;
pub const MIN_NODES_PER_PIXEL: u32 = 2;
pub const MAX_NODES_PER_PIXEL: u32 = 32;
/// Layers sorted exactly in one pixel (oit.wgsl sorts up to 24: `resolve_24`).
pub const DEFAULT_SORTED_LAYERS: u32 = 24;
pub const MAX_SORTED_LAYERS: u32 = 24;
/// The whole OIT never takes more video memory than this: the slot array,
/// the node pool, the heads, the counts, the composite texture, the band
/// depth mask and the tile list.
pub const MAX_POOL_BYTES: u64 = 512 << 20;
/// Bands a frame may be captured in: each a pass over the blended faces on
/// its depth mask, so each has its mask, draws, classify and composite
/// encoded whether the plan uses it or not.
pub const MAX_BANDS: u32 = 16;
/// Bytes per pixel of the heads (a fragment count and a list head, 2 words).
const HEADS_BYTES: u64 = 8;
/// Bytes per pixel of the count pass's layer count.
const COUNT_BYTES: u64 = 4;
/// Bytes per pixel of the composite texture (`rgba16float`) and of the
/// depth of its nearest layer (`r32float`).
const COMPOSITE_BYTES: u64 = 12;
/// Bytes per sample of the band depth mask.
const MASK_BYTES: u64 = 4;
/// Bytes per screen tile of the tile list.
const TILE_LIST_BYTES: u64 = 4;
/// Bytes per tile row of the band plan's row needs.
const ROW_NEED_BYTES: u64 = 4;
/// A slot (a texel of the array) has the size of a node.
const SLOT_BYTES: u64 = 16;
/// Slots per pixel at most: past that the array costs more than it saves.
const MAX_SLOTS: u32 = 8;
/// A node: premultiplied colour (rgb9e5), alpha and 24-bit tie-break, depth,
/// next.
const NODE_BYTES: u64 = 16;
/// Per node, the stamp of the capture that claimed it (object.wgsl
/// `oit_claims`).
const CLAIM_BYTES: u64 = 4;
/// Side of the resolve's screen tiles (oit.wgsl `TILE`, its workgroups).
const TILE: u32 = 8;
/// Bytes of the band table: a header and one entry per band (oit.wgsl
/// `bands`).
const BANDS_BYTES: u64 = 16 * (1 + MAX_BANDS as u64);
/// Bytes of each band's classify dispatch, composite draw and depth mask
/// draw arguments.
const BAND_ARGS_BYTES: u64 = (12 + 16 + 16) * MAX_BANDS as u64;
/// Stride of the band index uniform (one word per band at a dynamic
/// offset: the minimum alignment wgpu guarantees).
const BAND_UNIFORM_STRIDE: u64 = 256;

/// Storage buffers the capture's fragment stage reads: the five of every
/// blended face (main_thread.rs `FRAGMENT_STORAGE_BUFFERS`), the heads, the
/// nodes, the claims and the counts.
const CAPTURE_STORAGE_BUFFERS: u32 = 9;

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
    /// this frame's stamp, never 0: what the capture claims a node with
    /// (shifted by the band's index in the shader)
    stamp: u32,
    max_bands: u32,
    _pad: [u32; 2],
}

/// Whether this device can capture: storage written from fragment shaders,
/// with the storage buffers the capture binds beyond the blended faces'.
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

/// `AURORA_OIT_NODES=<n>`: the slot array's and the pool's layers per pixel
/// for this launch, whatever the setting (docs/TESTING.md). At its lowest
/// (2) a scene with many layers needs many bands, to check the banding.
pub(crate) fn nodes_override() -> Option<u32> {
    static OVERRIDE: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| std::env::var("AURORA_OIT_NODES").ok()?.trim().parse().ok())
}

/// `AURORA_OIT_STATS=1`: the count pass's layers per pixel, the band plan
/// and the first band's capture read back about once a second and logged
/// as `oit layers:` (docs/TESTING.md).
pub(crate) fn stats_wanted() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_OIT_STATS").is_ok_and(|v| v.trim() == "1"))
}

/// object.wgsl for this device: the capture's depth test forced before its
/// shader when the device can (`Features::SHADER_EARLY_DEPTH_TEST`; the
/// attribute is refused otherwise, and with it the whole module), and its
/// nodes allocated once per subgroup when it has subgroups
/// (`Features::SUBGROUP`; the subgroup built-ins are refused otherwise).
pub(crate) fn object_source(device: &wgpu::Device, src: &str) -> String {
    object_source_with(src, early_depth(device), subgroups(device))
}

/// [`object_source`] for the given device abilities: the markers of
/// object.wgsl's capture turned into the early depth test and the subgroup
/// allocation (`oit_alloc`: its scalar version between the
/// `OIT_ALLOC_SCALAR` markers, its subgroup version commented out between
/// the `OIT_ALLOC_SUBGROUP` ones, so that the file itself is the scalar
/// variant).
pub(crate) fn object_source_with(src: &str, early_depth: bool, subgroups: bool) -> String {
    let mut out = if early_depth {
        src.replace("/*OIT_EARLY_DEPTH*/", "@early_depth_test(force)")
    } else {
        src.to_string()
    };
    if subgroups
        && let (Some(start), Some(end)) = (out.find("// <OIT_ALLOC_SCALAR>"), out.find("// </OIT_ALLOC_SCALAR>"))
        && start < end
    {
        let after = end + "// </OIT_ALLOC_SCALAR>".len();
        out = format!("{}{}", &out[..start], &out[after..])
            .replace("/* <OIT_ALLOC_SUBGROUP>", "")
            .replace("</OIT_ALLOC_SUBGROUP> */", "");
    }
    out
}

/// The capture's depth test can be forced before its shader.
pub(crate) fn early_depth(device: &wgpu::Device) -> bool {
    device.features().contains(wgpu::Features::SHADER_EARLY_DEPTH_TEST)
}

/// The capture can allocate its nodes once per subgroup.
pub(crate) fn subgroups(device: &wgpu::Device) -> bool {
    device.features().contains(wgpu::Features::SUBGROUP)
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

/// How the slot array and the node pool are sized.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Plan {
    /// slot-array layers (fragments per pixel stored without the pool)
    pub slots: u32,
    /// nodes in the pool
    pub node_cap: u32,
}

/// Video memory that does not depend on the layer counts: the heads, the
/// counts, the composite texture, the band depth mask (`samples` per
/// pixel), the tile list and the band plan of a `width` x `height` screen.
pub fn fixed_bytes(width: u32, height: u32, samples: u32) -> u64 {
    let pixels = u64::from(width.max(1)) * u64::from(height.max(1));
    let tile_rows = u64::from(height.max(1).div_ceil(TILE));
    let tiles = u64::from(width.max(1).div_ceil(TILE)) * tile_rows;
    pixels * (HEADS_BYTES + COUNT_BYTES + COMPOSITE_BYTES + MASK_BYTES * u64::from(samples.max(1)))
        + tiles * TILE_LIST_BYTES
        + tile_rows * ROW_NEED_BYTES
        + BANDS_BYTES
        + BAND_ARGS_BYTES
        + BAND_UNIFORM_STRIDE * u64::from(MAX_BANDS)
}

/// The least the pool may hold: under it the bands would be too many.
const MIN_POOL_BYTES: u64 = 16 << 20;

/// Sizes the slot array and the pool: half of the per-pixel count goes to
/// the slots (1 to [`MAX_SLOTS`]), reduced to what takes no more than half
/// of what the fixed buffers ([`fixed_bytes`]) leave of `MAX_POOL_BYTES`
/// (4K screens; one slot at least) and by the device's layer limit. The
/// pool takes the rest of the budget within `max_binding` (fewer bands), or
/// `pool_per_pixel` nodes per pixel when `AURORA_OIT_NODES` sets it (the
/// bands then do the rest). `None` when the pool would be under
/// [`MIN_POOL_BYTES`] or the screen exceeds `max_dim`.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    width: u32,
    height: u32,
    samples: u32,
    nodes_per_pixel: u32,
    pool_per_pixel: Option<u32>,
    max_binding: u64,
    max_layers: u32,
    max_dim: u32,
) -> Option<Plan> {
    if width > max_dim || height > max_dim {
        return None;
    }
    let pixels = u64::from(width.max(1)) * u64::from(height.max(1));
    let per_pixel = nodes_per_pixel.clamp(MIN_NODES_PER_PIXEL, MAX_NODES_PER_PIXEL);
    let budget = MAX_POOL_BYTES.saturating_sub(fixed_bytes(width, height, samples));
    let by_budget = u32::try_from((budget / 2) / (pixels * SLOT_BYTES)).unwrap_or(u32::MAX);
    let slots = (per_pixel / 2).clamp(1, MAX_SLOTS).min(by_budget.max(1)).min(max_layers);
    if slots == 0 {
        return None;
    }
    let slot_bytes = pixels * SLOT_BYTES * u64::from(slots);
    let per_node = NODE_BYTES + CLAIM_BYTES;
    let mut bytes = budget.saturating_sub(slot_bytes).min(max_binding);
    if bytes < MIN_POOL_BYTES {
        return None;
    }
    if let Some(n) = pool_per_pixel {
        bytes = bytes.min(pixels * u64::from(n.saturating_sub(slots).max(1)) * per_node);
    }
    let node_cap = u32::try_from(bytes / per_node).unwrap_or(u32::MAX);
    Some(Plan { slots, node_cap })
}

struct Pool {
    width: u32,
    height: u32,
    samples: u32,
    node_cap: u32,
    slots: u32,
    nodes_per_pixel: u32,
    tiles_x: u32,
    tiles: u32,
    /// heads[0] counts the nodes; pixel p: heads[1 + 2p] counts its
    /// fragments, heads[2 + 2p] is its list's head + 1 (0: none)
    heads: wgpu::Buffer,
    /// the count pass's layers per pixel
    counts: wgpu::Buffer,
    /// the first `slots` fragments of each pixel (layer k: slot k of all)
    slots_view: wgpu::TextureView,
    /// keeps the slot array alive
    _slots_tex: wgpu::Texture,
    nodes: wgpu::Buffer,
    /// per node, the stamp of the capture that claimed it (the capture's
    /// group keeps it alive; kept here with the pool)
    _claims: wgpu::Buffer,
    /// each pixel's composite, and the depth of its nearest layer
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// the band plan: the nodes each tile row needs (the plan's group
    /// keeps it alive), the bands (oit.wgsl `bands`), each band's classify
    /// dispatch, composite draw and depth mask draw arguments, the band
    /// index uniform
    _row_need: wgpu::Buffer,
    bands: wgpu::Buffer,
    dispatch_args: wgpu::Buffer,
    draw_args: wgpu::Buffer,
    mask_args: wgpu::Buffer,
    band_index: wgpu::Buffer,
    tile_list: wgpu::Buffer,
    /// the band's depth mask: the scene depth inside its rows, the nearest
    /// depth outside, the capture's and the glow's depth attachment
    band_depth: wgpu::TextureView,
    _band_depth_tex: wgpu::Texture,
    capture: wgpu::BindGroup,
    glow: wgpu::BindGroup,
    /// the band plan's group
    plan: wgpu::BindGroup,
    /// the classify's and the draws' groups, and the scene depth they read
    groups: Option<(wgpu::BindGroup, wgpu::BindGroup)>,
    groups_depth: Option<wgpu::TextureView>,
}

/// The resolve's pipelines for one sample count, compiled with the scene's
/// (renderer.rs `make_pipelines`, on the pipeline worker).
pub(crate) struct Resolve {
    samples: u32,
    classify: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    /// the band's depth mask (oit.wgsl `fs_band_depth`)
    band_depth: wgpu::RenderPipeline,
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

/// The band plan's pipelines (oit.wgsl `cs_band_rows`, `cs_band_plan`):
/// no sample count in them, made with the module.
struct BandPlan {
    layout: wgpu::BindGroupLayout,
    rows: wgpu::ComputePipeline,
    plan: wgpu::ComputePipeline,
}

pub(crate) struct Oit {
    shared: Shared,
    band_plan: BandPlan,
    params: wgpu::Buffer,
    last_params: Option<Params>,
    pool: Option<Pool>,
    /// a pool that could not be allocated is tried again later
    retry: crate::gpu_alloc::Backoff,
    /// the blended draws' copies per band (oit_gate.rs)
    gate: crate::oit_gate::Gate,
    /// the frame stamp of the capture's node claims (never 0)
    stamp: u32,
    /// `AURORA_OIT_STATS`: the counts, the bands and the first band's heads
    /// read back for the `oit layers:` line
    sample: std::sync::Mutex<Option<Sample>>,
}

/// The read back for the log (`AURORA_OIT_STATS`): the counts and the band
/// plan copied after the plan, the heads after the first band's capture,
/// mapped after the submit, counted once the GPU is done (no wait, the
/// line comes a few frames later).
struct Sample {
    buf: wgpu::Buffer,
    /// the counts' bytes (the bands follow them, then the heads)
    count_bytes: u64,
    width: u32,
    height: u32,
    slots: u32,
    node_cap: u32,
    /// SAMPLE_IDLE, _ARMED (the counts and the bands copied), _ENCODED
    /// (the heads too), _WAITING (mapping), _MAPPED or _FAILED
    state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    last: std::time::Instant,
}

const SAMPLE_IDLE: u8 = 0;
const SAMPLE_ENCODED: u8 = 1;
const SAMPLE_WAITING: u8 = 2;
const SAMPLE_MAPPED: u8 = 3;
const SAMPLE_FAILED: u8 = 4;
const SAMPLE_ARMED: u8 = 5;
/// Seconds between two read backs.
const SAMPLE_PERIOD: f32 = 1.0;

/// The `oit layers:` line from the count pass of one frame (`width` x
/// `height` pixels, `slots` per pixel in the slot array, a pool of
/// `node_cap`), its band plan (`bands`, oit.wgsl's table) and the heads
/// after its first band's capture: the covered pixels, the layers per
/// covered pixel (mean and percentiles), the pixels past the slots, past
/// the sort's 24 and past a walk's bound, the nodes the frame needed
/// against the pool, the bands used, and what the first band stored.
fn layers_line(counts: &[u32], bands: &[u32], heads: &[u32], width: u32, height: u32, slots: u32, node_cap: u32) -> String {
    let pixels = (width as usize) * (height as usize);
    let mut counts: Vec<u32> = counts.iter().take(pixels).copied().filter(|&c| c > 0).collect();
    counts.sort_unstable();
    let covered = counts.len();
    let fragments: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    let needed: u64 = counts.iter().map(|&c| u64::from(c.saturating_sub(slots))).sum();
    let pct = |n: usize| 100.0 * n as f64 / pixels.max(1) as f64;
    let at = |q: f64| {
        counts
            .get(((covered as f64 - 1.0) * q).round().max(0.0) as usize)
            .copied()
            .unwrap_or(0)
    };
    let over = |n: u32| counts.iter().filter(|&&c| c > n).count();
    let word = |i: usize| bands.get(i).copied().unwrap_or(0);
    let used = word(0);
    let rows: Vec<String> = (0..used.min(MAX_BANDS) as usize)
        .map(|b| format!("{}-{}", word(4 * (1 + b)), word(4 * (1 + b) + 1)))
        .collect();
    // the first band's capture: the fragments its heads count, its nodes
    let head = |p: usize| heads.get(1 + 2 * p).copied().unwrap_or(0);
    let stored: u64 = (0..pixels).map(|p| u64::from(head(p))).sum();
    let stored_px = (0..pixels).filter(|&p| head(p) > 0).count();
    let nodes_used = heads.first().copied().unwrap_or(0);
    format!(
        "oit layers: {covered} of {pixels} pixels covered ({:.1} %), {fragments} fragments, per covered pixel mean {:.1} p50 {} p95 {} max {}; over the {slots} slots {} ({:.1} %), over 24 {} ({:.1} %), over 1024 {}; nodes needed {needed} of {node_cap} ({:.0} %): {used} band(s) of rows {}{}{}; band 0 stored {stored} fragments in {stored_px} pixels, {nodes_used} nodes",
        pct(covered),
        fragments as f64 / covered.max(1) as f64,
        at(0.5),
        at(0.95),
        counts.last().copied().unwrap_or(0),
        over(slots),
        pct(over(slots)),
        over(24),
        pct(over(24)),
        over(1024),
        100.0 * needed as f64 / u64::from(node_cap.max(1)) as f64,
        rows.join(" "),
        if word(2) != 0 { ", rows merged into the last" } else { "" },
        if word(1) != 0 {
            ", a band passed the pool: fragments left out"
        } else {
            ""
        }
    )
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

/// The band index uniform: one word per band, bound at a dynamic offset.
fn band_uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: true,
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

/// The scene depth read as a texture, with or without MSAA.
fn depth_texture(ms: bool) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Depth,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: ms,
    }
}

fn bind(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

/// A binding of one word of the band index uniform (at a dynamic offset).
fn bind_band_index(buffer: &wgpu::Buffer) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: 0,
        size: wgpu::BufferSize::new(16),
    })
}

/// The resolve's group layout (oit.wgsl bindings): `classify` the compute
/// step's, otherwise the draws' (the composite and the band depth mask);
/// `ms` with the scene's MSAA depth.
fn resolve_layout(device: &wgpu::Device, classify: bool, ms: bool) -> wgpu::BindGroupLayout {
    use wgpu::ShaderStages as S;
    let read = buffer(wgpu::BufferBindingType::Storage { read_only: true });
    let rw = buffer(wgpu::BufferBindingType::Storage { read_only: false });
    let stage = if classify { S::COMPUTE } else { S::FRAGMENT };
    let mut entries = vec![entry(2, S::VERTEX_FRAGMENT | S::COMPUTE, buffer(wgpu::BufferBindingType::Uniform))];
    // the scene depth: the classify's MSAA rule, the band depth mask
    if ms || !classify {
        entries.push(entry(if ms { 3 } else { 11 }, stage, depth_texture(ms)));
    }
    if classify {
        entries.push(entry(0, stage, read));
        entries.push(entry(1, stage, read));
        // the slot array (oit.wgsl `slots_r`)
        entries.push(entry(10, stage, slots_texture()));
        entries.push(entry(
            4,
            S::COMPUTE,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        ));
        entries.push(entry(
            12,
            S::COMPUTE,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::R32Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        ));
        entries.push(entry(5, S::COMPUTE, rw));
        entries.push(entry(6, S::COMPUTE, rw));
        entries.push(entry(15, S::COMPUTE, rw));
        entries.push(entry(17, S::COMPUTE, band_uniform()));
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
        entries.push(entry(
            13,
            S::FRAGMENT,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        ));
        entries.push(entry(8, S::VERTEX, read));
        // the band depth mask: the bands (declared read-write by the
        // shader, for the classify) and the band
        entries.push(entry(15, S::FRAGMENT, rw));
        entries.push(entry(17, S::FRAGMENT, band_uniform()));
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
    let composite = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("oit composite"),
        layout: Some(&draw_layout),
        vertex: wgpu::VertexState {
            module: &shared.module,
            entry_point: Some("vs_tile"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shared.module,
            entry_point: Some("fs_composite"),
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
        // the scene depth stays attached, read only: the composite is
        // tested at the depth of its nearest layer (fs_composite), so
        // with MSAA an opaque edge in front of every layer keeps its
        // samples
        depth_stencil: Some(wgpu::DepthStencilState {
            format: shared.depth_format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Greater),
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
    let band_depth = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("oit band depth"),
        layout: Some(&draw_layout),
        vertex: wgpu::VertexState {
            module: &shared.module,
            entry_point: Some("vs_full"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shared.module,
            entry_point: Some(if ms { "fs_band_depth_ms" } else { "fs_band_depth" }),
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
        composite,
        band_depth,
    }
}

impl Oit {
    /// `None` when the device cannot capture ([`supported`]).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> Option<Self> {
        if !supported(device) {
            return None;
        }
        use wgpu::BufferBindingType as B;
        use wgpu::ShaderStages as S;
        // group 3 of the capture (object.wgsl bindings 20..23, 27, 28, 30)
        // and of the glow (22, 24..26, 30): a write-only storage texture
        // cannot be read, so the slot array has a layout entry for each use
        let frag = S::FRAGMENT;
        let params_entry = entry(22, frag, buffer(B::Uniform));
        // the band index (the capture's node claims)
        let band_entry = entry(30, frag, band_uniform());
        let capture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit capture"),
            entries: &[
                entry(20, frag, buffer(B::Storage { read_only: false })),
                entry(21, frag, buffer(B::Storage { read_only: false })),
                params_entry,
                entry(27, frag, buffer(B::Storage { read_only: false })),
                entry(28, frag, buffer(B::Storage { read_only: false })),
                entry(
                    23,
                    frag,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                ),
                band_entry,
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
                params_entry,
                entry(24, frag, slots_texture()),
                band_entry,
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oit resolve"),
            source: wgpu::ShaderSource::Wgsl(resolve_source().into()),
        });
        // the band plan's group (oit.wgsl bindings 2, 14, 15, 16, 18, 5, 19)
        let comp = S::COMPUTE;
        let rw = buffer(B::Storage { read_only: false });
        let plan_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit band plan"),
            entries: &[
                entry(2, comp, buffer(B::Uniform)),
                entry(14, comp, read_only),
                entry(15, comp, rw),
                entry(16, comp, rw),
                entry(18, comp, rw),
                entry(5, comp, rw),
                entry(19, comp, rw),
            ],
        });
        let plan_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oit band plan"),
            bind_group_layouts: &[Some(&plan_layout)],
            immediate_size: 0,
        });
        let compute = |label, entry_point| {
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&plan_pipeline_layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let band_plan = BandPlan {
            rows: compute("oit band rows", "cs_band_rows"),
            plan: compute("oit band plan", "cs_band_plan"),
            layout: plan_layout,
        };
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
            band_plan,
            params,
            last_params: None,
            pool: None,
            retry: Default::default(),
            gate: crate::oit_gate::Gate::new(device),
            stamp: 0,
            sample: std::sync::Mutex::new(None),
        })
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// Makes the pool ready for this frame. `depth_ss`, `depth_msaa`: the
    /// scene's depth, single-sample and MSAA (`None` without MSAA), the band
    /// depth masks are made from; `blend`: the blend list's range of the
    /// indirect buffer (byte offset, draws), drawn once per band. `false`
    /// when the pool cannot be made (video memory full: tried again after a
    /// while, `gpu_alloc::Backoff`): the faces are then blended as before.
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
        blend: (u64, u32),
    ) -> bool {
        let limits = device.limits();
        let max_binding = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let nodes_per_pixel = nodes_per_pixel.clamp(MIN_NODES_PER_PIXEL, MAX_NODES_PER_PIXEL);
        let stale = self
            .pool
            .as_ref()
            .is_none_or(|p| p.width != width || p.height != height || p.samples != samples || p.nodes_per_pixel != nodes_per_pixel);
        if stale {
            self.pool = None;
            let now = std::time::Instant::now();
            if !self.retry.ready(now) {
                return false;
            }
            match self.make_pool(device, width, height, samples, nodes_per_pixel, max_binding) {
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
            // a new stamp each frame (the claims of the last one are void),
            // never 0 (what a cleared claim holds); the capture shifts it
            // by the band's index (object.wgsl), so 28 bits of it count
            stamp: self.stamp.wrapping_add(1).max(1),
            max_bands: MAX_BANDS,
            _pad: [0; 2],
        };
        self.stamp = params.stamp;
        if self.last_params != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
            self.last_params = Some(params);
        }
        let bands = pool.bands.clone();
        self.gate.prepare(device, queue, indirect, &bands, blend, MAX_BANDS)
    }

    /// The pool's buffers and textures, or why one could not be made: every
    /// allocation checked (`gpu_alloc`), the pool installed only whole.
    fn make_pool(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        samples: u32,
        nodes_per_pixel: u32,
        max_binding: u64,
    ) -> Result<Pool, String> {
        let limits = device.limits();
        let Some(Plan { slots, node_cap }) = plan(
            width,
            height,
            samples,
            nodes_per_pixel,
            nodes_override().map(|_| nodes_per_pixel),
            max_binding,
            limits.max_texture_array_layers,
            limits.max_texture_dimension_2d,
        ) else {
            return Err("larger than the device's textures".to_owned());
        };
        // two words per pixel: its fragment count and its list's head
        let pixels = u64::from(width) * u64::from(height);
        let heads_bytes = (1 + 2 * pixels) * 4;
        if node_cap == 0 || heads_bytes > max_binding {
            return Err("larger than the device's storage buffers".to_owned());
        }
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
        // (COPY_SRC: read back with AURORA_OIT_STATS)
        let heads = buffer("oit heads", heads_bytes, U::STORAGE | U::COPY_SRC)?;
        let counts = buffer("oit counts", pixels * COUNT_BYTES, U::STORAGE | U::COPY_SRC)?;
        let nodes = buffer("oit nodes", u64::from(node_cap) * NODE_BYTES, U::STORAGE)?;
        let claims = buffer("oit node claims", u64::from(node_cap) * CLAIM_BYTES, U::STORAGE)?;
        let row_need = buffer("oit band rows", u64::from(tiles_y) * ROW_NEED_BYTES, U::STORAGE)?;
        let bands = buffer("oit bands", BANDS_BYTES, U::STORAGE | U::COPY_SRC)?;
        let dispatch_args = buffer("oit band classify", 12 * u64::from(MAX_BANDS), U::STORAGE | U::INDIRECT)?;
        let draw_args = buffer("oit band composite", 16 * u64::from(MAX_BANDS), U::STORAGE | U::INDIRECT)?;
        let mask_args = buffer("oit band depth mask", 16 * u64::from(MAX_BANDS), U::STORAGE | U::INDIRECT)?;
        let tile_list = buffer("oit tiles", u64::from(tiles) * TILE_LIST_BYTES, U::STORAGE)?;
        // the band index uniform: band b's word at b * 256
        let band_index = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit band index"),
            size: BAND_UNIFORM_STRIDE * u64::from(MAX_BANDS),
            usage: U::UNIFORM,
            mapped_at_creation: true,
        });
        match band_index.slice(..).get_mapped_range_mut() {
            Ok(mut view) => {
                let mut words = vec![0u8; (BAND_UNIFORM_STRIDE * u64::from(MAX_BANDS)) as usize];
                for b in 0..MAX_BANDS {
                    let at = (u64::from(b) * BAND_UNIFORM_STRIDE) as usize;
                    words[at..at + 4].copy_from_slice(&b.to_le_bytes());
                }
                view.copy_from_slice(&words);
            }
            Err(e) => return Err(format!("band index uniform: {e}")),
        }
        band_index.unmap();
        let texture = |label, format, sample_count, usage, bytes| {
            crate::gpu_alloc::try_texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                },
                &Default::default(),
                bytes,
            )
        };
        let storage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let (_, color) = texture("oit composite", wgpu::TextureFormat::Rgba16Float, 1, storage, pixels * 8)?;
        let (_, depth) = texture("oit composite depth", wgpu::TextureFormat::R32Float, 1, storage, pixels * 4)?;
        let (band_depth_tex, band_depth) = texture(
            "oit band depth",
            self.shared.depth_format,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            pixels * MASK_BYTES * u64::from(samples),
        )?;
        let slots_bytes = pixels * u64::from(slots) * SLOT_BYTES;
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
                usage: storage,
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
            "oit: {slots} slots ({} MB) + {node_cap} nodes ({} MB) for {width}x{height}, up to {MAX_BANDS} bands",
            slots_bytes >> 20,
            (u64::from(node_cap) * (NODE_BYTES + CLAIM_BYTES)) >> 20
        );
        let capture = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit capture"),
            layout: &self.shared.capture_layout,
            entries: &[
                bind(20, heads.as_entire_binding()),
                bind(21, nodes.as_entire_binding()),
                bind(22, self.params.as_entire_binding()),
                bind(27, claims.as_entire_binding()),
                bind(28, counts.as_entire_binding()),
                bind(23, wgpu::BindingResource::TextureView(&slots_view)),
                bind(30, bind_band_index(&band_index)),
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
                bind(30, bind_band_index(&band_index)),
            ],
        });
        let plan = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit band plan"),
            layout: &self.band_plan.layout,
            entries: &[
                bind(2, self.params.as_entire_binding()),
                bind(14, counts.as_entire_binding()),
                bind(15, bands.as_entire_binding()),
                bind(16, row_need.as_entire_binding()),
                bind(18, dispatch_args.as_entire_binding()),
                bind(5, draw_args.as_entire_binding()),
                bind(19, mask_args.as_entire_binding()),
            ],
        });
        Ok(Pool {
            width,
            height,
            samples,
            node_cap,
            slots,
            nodes_per_pixel,
            tiles_x,
            tiles,
            heads,
            counts,
            slots_view,
            _slots_tex: slots_tex,
            nodes,
            _claims: claims,
            color,
            depth,
            _row_need: row_need,
            bands,
            dispatch_args,
            draw_args,
            mask_args,
            band_index,
            tile_list,
            band_depth,
            _band_depth_tex: band_depth_tex,
            capture,
            glow,
            plan,
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
            bind(10, wgpu::BindingResource::TextureView(&pool.slots_view)),
        ];
        if let Some(depth) = depth_msaa {
            classify.push(bind(3, wgpu::BindingResource::TextureView(depth)));
        }
        classify.push(bind(4, wgpu::BindingResource::TextureView(&pool.color)));
        classify.push(bind(12, wgpu::BindingResource::TextureView(&pool.depth)));
        classify.push(bind(5, pool.draw_args.as_entire_binding()));
        classify.push(bind(6, pool.tile_list.as_entire_binding()));
        classify.push(bind(15, pool.bands.as_entire_binding()));
        classify.push(bind(17, bind_band_index(&pool.band_index)));
        let draws = [
            bind(2, self.params.as_entire_binding()),
            bind(7, wgpu::BindingResource::TextureView(&pool.color)),
            bind(13, wgpu::BindingResource::TextureView(&pool.depth)),
            bind(8, pool.tile_list.as_entire_binding()),
            bind(
                if depth_msaa.is_some() { 3 } else { 11 },
                wgpu::BindingResource::TextureView(depth_msaa.unwrap_or(depth_ss)),
            ),
            bind(15, pool.bands.as_entire_binding()),
            bind(17, bind_band_index(&pool.band_index)),
        ];
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

    /// The capture's group 3, once [`Oit::prepare`] made the pool; bound at
    /// [`Oit::band_offset`] of the band being drawn.
    pub fn capture_group(&self) -> Option<&wgpu::BindGroup> {
        self.pool.as_ref().map(|p| &p.capture)
    }

    /// The glow's group 3 (the same lists, the slot array read), bound at
    /// [`Oit::band_offset`] of the band being drawn.
    pub fn glow_group(&self) -> Option<&wgpu::BindGroup> {
        self.pool.as_ref().map(|p| &p.glow)
    }

    /// The dynamic offset of band `band` in the band index uniform (group
    /// 3 of the capture and the glow).
    pub fn band_offset(band: u32) -> u32 {
        band * BAND_UNIFORM_STRIDE as u32
    }

    /// The depth attachment of each band's capture and glow, once
    /// [`Oit::band_mask`] wrote it for the band.
    pub fn band_depth(&self) -> Option<&wgpu::TextureView> {
        self.pool.as_ref().map(|p| &p.band_depth)
    }

    /// Empties the counts (before the count pass).
    pub fn clear_counts(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(pool) = &self.pool {
            encoder.clear_buffer(&pool.counts, 0, None);
        }
    }

    /// Empties the lists (before each band's capture).
    pub fn clear_heads(&self, encoder: &mut wgpu::CommandEncoder) {
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

    /// The band plan from the count pass: the nodes each tile row needs,
    /// the bands, their arguments and the blended draws' copies per band
    /// (after the count pass, before the first band).
    pub fn plan_bands(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(pool) = &self.pool {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("oit band plan"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.band_plan.rows);
            pass.set_bind_group(0, &pool.plan, &[]);
            pass.dispatch_workgroups(pool.height.div_ceil(TILE), 1, 1);
            pass.set_pipeline(&self.band_plan.plan);
            pass.dispatch_workgroups(1, 1, 1);
            self.gate.dispatch(&mut pass);
        }
    }

    /// Writes band `band`'s depth mask: the scene depth inside its rows,
    /// the nearest depth outside (nothing drawn for a band without rows).
    pub fn band_mask(&self, encoder: &mut wgpu::CommandEncoder, r: &Resolve, band: u32) {
        if let Some(pool) = &self.pool
            && let Some((_, group)) = &pool.groups
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("oit band depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &pool.band_depth,
                    depth_ops: Some(wgpu::Operations {
                        // (every texel is written when the band has rows)
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&r.band_depth);
            pass.set_bind_group(0, group, &[Self::band_offset(band)]);
            pass.draw_indirect(&pool.mask_args, 16 * u64::from(band));
        }
    }

    /// Sorts the lists of band `band` and lists its tiles to composite
    /// (after the band's capture).
    pub fn classify(&self, encoder: &mut wgpu::CommandEncoder, r: &Resolve, band: u32) {
        if let Some(pool) = &self.pool
            && let Some((group, _)) = &pool.groups
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("oit classify"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&r.classify);
            pass.set_bind_group(0, group, &[Self::band_offset(band)]);
            pass.dispatch_workgroups_indirect(&pool.dispatch_args, 12 * u64::from(band));
        }
    }

    /// The copy and the offset of band `band`'s draws `offset..` of the
    /// indirect buffer (`count` of them), the blend list's capture. `None`:
    /// not in the blend list, nothing to draw.
    pub fn band_draws(&self, band: u32, offset: u64, count: u32) -> Option<(&wgpu::Buffer, u64)> {
        self.gate.args(band, offset, count)
    }

    /// Composites the listed tiles of band `band` over the scene (after
    /// [`Oit::classify`], in a pass with the scene colour and its depth read
    /// only), once per pixel, at the depth of each pixel's nearest layer.
    pub fn resolve(&self, pass: &mut wgpu::RenderPass, r: &Resolve, band: u32) {
        if let Some(pool) = &self.pool
            && let Some((_, group)) = &pool.groups
        {
            pass.set_bind_group(0, group, &[Self::band_offset(band)]);
            pass.set_pipeline(&r.composite);
            pass.draw_indirect(&pool.draw_args, 16 * u64::from(band));
        }
    }

    /// `AURORA_OIT_STATS`: copies the counts and the bands after the plan,
    /// about once a second, for the `oit layers:` line
    /// ([`Oit::layers_log`]). Nothing otherwise.
    pub fn sample_layers(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if !stats_wanted() {
            return;
        }
        let Some(pool) = &self.pool else {
            return;
        };
        let Ok(mut sample) = self.sample.lock() else {
            return;
        };
        let count_bytes = pool.counts.size();
        let now = std::time::Instant::now();
        let stale = sample.as_ref().is_none_or(|s| s.count_bytes != count_bytes);
        if stale {
            *sample = Some(Sample {
                buf: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("oit stats readback"),
                    size: count_bytes + BANDS_BYTES + pool.heads.size(),
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                count_bytes,
                width: pool.width,
                height: pool.height,
                slots: pool.slots,
                node_cap: pool.node_cap,
                state: Default::default(),
                last: now - std::time::Duration::from_secs(10),
            });
        }
        let Some(s) = sample.as_mut() else {
            return;
        };
        use std::sync::atomic::Ordering;
        if s.state.load(Ordering::Acquire) != SAMPLE_IDLE || now.duration_since(s.last).as_secs_f32() < SAMPLE_PERIOD {
            return;
        }
        encoder.copy_buffer_to_buffer(&pool.counts, 0, &s.buf, 0, count_bytes);
        encoder.copy_buffer_to_buffer(&pool.bands, 0, &s.buf, count_bytes, BANDS_BYTES);
        s.last = now;
        s.state.store(SAMPLE_ARMED, Ordering::Release);
    }

    /// `AURORA_OIT_STATS`: after the first band's capture, its heads (the
    /// fragments it stored) beside the counts, when [`Oit::sample_layers`]
    /// copied them this frame.
    pub fn sample_heads(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(pool) = &self.pool else {
            return;
        };
        let Ok(sample) = self.sample.lock() else {
            return;
        };
        let Some(s) = sample.as_ref() else {
            return;
        };
        use std::sync::atomic::Ordering;
        if s.state.load(Ordering::Acquire) != SAMPLE_ARMED {
            return;
        }
        encoder.copy_buffer_to_buffer(&pool.heads, 0, &s.buf, s.count_bytes + BANDS_BYTES, pool.heads.size());
        s.state.store(SAMPLE_ENCODED, Ordering::Release);
    }

    /// Maps what was copied this frame, once its commands are submitted.
    pub fn after_submit(&self) {
        use std::sync::atomic::Ordering;
        let Ok(sample) = self.sample.lock() else {
            return;
        };
        let Some(s) = sample.as_ref() else {
            return;
        };
        if s.state.load(Ordering::Acquire) != SAMPLE_ENCODED {
            return;
        }
        s.state.store(SAMPLE_WAITING, Ordering::Release);
        let state = s.state.clone();
        s.buf.map_async(wgpu::MapMode::Read, .., move |r| {
            state.store(if r.is_ok() { SAMPLE_MAPPED } else { SAMPLE_FAILED }, Ordering::Release);
        });
    }

    /// The `oit layers:` line of the last read back, once the GPU delivered
    /// it (`AURORA_OIT_STATS`).
    pub fn layers_log(&self) -> Option<String> {
        use std::sync::atomic::Ordering;
        let Ok(sample) = self.sample.lock() else {
            return None;
        };
        let s = sample.as_ref()?;
        match s.state.load(Ordering::Acquire) {
            SAMPLE_MAPPED => {
                let line = s.buf.get_mapped_range(..).ok().map(|view| {
                    let words: &[u32] = bytemuck::cast_slice(&view);
                    let n = ((s.count_bytes / 4) as usize).min(words.len());
                    let b = (n + (BANDS_BYTES / 4) as usize).min(words.len());
                    layers_line(&words[..n], &words[n..b], &words[b..], s.width, s.height, s.slots, s.node_cap)
                });
                s.buf.unmap();
                s.state.store(SAMPLE_IDLE, Ordering::Release);
                line
            }
            SAMPLE_FAILED => {
                s.state.store(SAMPLE_IDLE, Ordering::Release);
                Some("oit layers: the read back failed".to_owned())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: u64 = u64::MAX;

    /// Everything the OIT holds for a plan: slots, nodes and their claims,
    /// heads, counts, composite, band depth mask, tile list, band plan
    /// (what `make_pool` allocates).
    fn total(p: Plan, w: u32, h: u32, samples: u32) -> u64 {
        u64::from(w) * u64::from(h) * u64::from(p.slots) * SLOT_BYTES
            + u64::from(p.node_cap) * (NODE_BYTES + CLAIM_BYTES)
            + fixed_bytes(w, h, samples)
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
    fn the_pool_takes_the_rest_of_the_budget() {
        // 1080p, 8 per pixel, MSAA 4x: 4 slots, the pool the rest of the 512 MB
        let p = plan(1920, 1080, 4, 8, None, BIG, 2048, 16384).unwrap();
        assert_eq!(p.slots, 4);
        let mb = total(p, 1920, 1080, 4) >> 20;
        assert!((510..=512).contains(&mb), "{mb} MB");
        // the default at 1600x900: about 19 million nodes
        let p = plan(1600, 900, 4, 8, None, BIG, 2048, 16384).unwrap();
        assert_eq!(p.slots, 4);
        assert!((18_000_000..21_000_000).contains(&p.node_cap), "{}", p.node_cap);
        assert!(total(p, 1600, 900, 4) <= MAX_POOL_BYTES);
    }

    #[test]
    fn an_override_sizes_the_pool_per_pixel() {
        // AURORA_OIT_NODES=8 at 1600x900: 4 slots and 4 nodes per pixel
        let p = plan(1600, 900, 4, 8, Some(8), BIG, 2048, 16384).unwrap();
        assert_eq!((p.slots, p.node_cap), (4, 1600 * 900 * 4));
        // 2: one slot and one node per pixel, many bands
        let p = plan(1600, 900, 4, 2, Some(2), BIG, 2048, 16384).unwrap();
        assert_eq!((p.slots, p.node_cap), (1, 1600 * 900));
    }

    #[test]
    fn the_fixed_buffers_count_in_the_budget() {
        // 24 bytes per pixel before the first slot (the heads, the counts,
        // the composite and its depth), the band depth mask per sample,
        // plus the tile list and the plan
        assert!(fixed_bytes(1600, 900, 1) > 1600 * 900 * 28 + 200 * 113 * 4);
        assert!(fixed_bytes(1600, 900, 1) < 1600 * 900 * 28 + 200 * 113 * 4 + (16 << 10));
        assert_eq!(fixed_bytes(1600, 900, 4) - fixed_bytes(1600, 900, 1), 1600 * 900 * 12);
        // however large the budget wanted, the total never passes the cap
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160), (7680, 4320)] {
            for samples in [1, 4, 8] {
                for per_pixel in [2, 8, 16, 32] {
                    for over in [None, Some(per_pixel)] {
                        if let Some(p) = plan(w, h, samples, per_pixel, over, BIG, 2048, 16384) {
                            assert!(
                                total(p, w, h, samples) <= MAX_POOL_BYTES,
                                "{w}x{h} at {per_pixel}, {samples}x: {} MB",
                                total(p, w, h, samples) >> 20
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_4k_screen_reduces_the_slots_and_stays_within_the_budget() {
        let p = plan(3840, 2160, 4, 8, None, BIG, 2048, 16384).unwrap();
        assert_eq!(p.slots, 1, "2 slots and the heads would pass half of what is left");
        assert!(total(p, 3840, 2160, 4) <= MAX_POOL_BYTES);
        assert!(total(p, 3840, 2160, 4) > (500 << 20), "the rest of the budget goes to the pool");
        assert!(u64::from(p.node_cap) * (NODE_BYTES + CLAIM_BYTES) >= MIN_POOL_BYTES);
        let p = plan(3840, 2160, 4, 32, None, BIG, 2048, 16384).unwrap();
        assert!(p.slots >= 1 && total(p, 3840, 2160, 4) <= MAX_POOL_BYTES);
        // 8K with MSAA 8x: the fixed buffers alone pass the budget
        assert_eq!(plan(7680, 4320, 8, 8, None, BIG, 2048, 16384), None);
    }

    #[test]
    fn the_slot_count_stays_in_its_bounds() {
        assert_eq!(plan(100, 100, 1, 2, None, BIG, 2048, 16384).unwrap().slots, 1);
        assert_eq!(plan(100, 100, 1, 0, None, BIG, 2048, 16384).unwrap().slots, 1);
        assert_eq!(plan(100, 100, 1, 32, None, BIG, 2048, 16384).unwrap().slots, MAX_SLOTS);
        assert_eq!(plan(100, 100, 1, 1000, None, BIG, 2048, 16384).unwrap().slots, MAX_SLOTS);
        // the device's layer limit
        assert_eq!(plan(100, 100, 1, 32, None, BIG, 3, 16384).unwrap().slots, 3);
    }

    #[test]
    fn the_device_limits_are_respected() {
        // a binding limit below the pool wins
        let p = plan(1920, 1080, 4, 8, None, 64 << 20, 2048, 16384).unwrap();
        assert_eq!(p.node_cap, (64 << 20) / (16 + 4));
        // a screen larger than the textures: no plan
        assert_eq!(plan(5000, 100, 1, 8, None, BIG, 2048, 4096), None);
        // no layer allowed
        assert_eq!(plan(100, 100, 1, 8, None, BIG, 0, 16384), None);
    }

    #[test]
    fn the_layers_line_reads_the_bands() {
        // 4 pixels: 0, 3, 30 and 2000 layers, 2 slots; two bands used, the
        // second passed the pool; band 0 stored 33 fragments in 2 pixels
        let counts = [0, 3, 30, 2000];
        let bands = [2, 1, 0, 2, 0, 8, 0, 1, 8, 16, 1, 2];
        let line = layers_line(&counts, &bands, &[5, 0, 0, 3, 1, 30, 2], 2, 2, 2, 100);
        assert!(line.contains("3 of 4 pixels covered (75.0 %), 2033 fragments"), "{line}");
        assert!(line.contains("over the 2 slots 3"), "{line}");
        assert!(line.contains("over 24 2"), "{line}");
        assert!(line.contains("over 1024 1"), "{line}");
        assert!(
            line.contains("nodes needed 2027 of 100 (2027 %): 2 band(s) of rows 0-8 8-16"),
            "{line}"
        );
        assert!(
            line.contains("a band passed the pool: fragments left out; band 0 stored 33 fragments in 2 pixels, 5 nodes"),
            "{line}"
        );
    }

    /// The order of oit.wgsl's sort (`before`): by depth, farthest first
    /// (reverse-Z: ascending), then by the tie-break; a total order, so the
    /// result never depends on the order the layers are listed in.
    fn sorted(layers: &[(u32, u32)]) -> Vec<(u32, u32)> {
        let before = |(k, t): (u32, u32), (k2, t2): (u32, u32)| k < k2 || (k == k2 && t < t2);
        let mut out: Vec<(u32, u32)> = Vec::new();
        for &l in layers {
            let at = out.iter().position(|&o| before(l, o)).unwrap_or(out.len());
            out.insert(at, l);
        }
        out
    }

    #[test]
    fn coplanar_layers_sort_by_their_tie_break_whatever_their_order() {
        // two panes at one depth (bits 0x3f00_0000) and a nearer strand
        let a = (0x3f00_0000, 0x1234);
        let b = (0x3f00_0000, 0x0abc);
        let c = (0x3f80_0000, 0x7777);
        let expected = vec![b, a, c];
        assert_eq!(sorted(&[a, b, c]), expected);
        assert_eq!(sorted(&[b, a, c]), expected);
        assert_eq!(sorted(&[c, a, b]), expected);
        assert_eq!(sorted(&[c, b, a]), expected);
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
        let gate = include_str!("shaders/oit_gate.wgsl");
        let module = naga::front::wgsl::parse_str(gate).unwrap_or_else(|e| panic!("{}", e.emit_to_string(gate)));
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(gate));
        }
    }

    /// The capture's four variants of object.wgsl (early depth test or not,
    /// the nodes allocated per subgroup or per fragment) are valid.
    #[test]
    fn the_capture_shader_is_valid_in_its_four_variants() {
        let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
        for (early, subgroups) in [(false, false), (true, false), (false, true), (true, true)] {
            let generated = object_source_with(&object, early, subgroups);
            // (a comment of the file names the attribute: the marker tells)
            assert_eq!(generated.contains("/*OIT_EARLY_DEPTH*/"), !early);
            // (the subgroup version is in the file, commented out)
            assert_eq!(generated.contains("/* <OIT_ALLOC_SUBGROUP>"), !subgroups);
            assert_eq!(generated.contains("// <OIT_ALLOC_SCALAR>"), !subgroups);
            assert_eq!(
                generated.matches("fn oit_alloc(").count(),
                1 + usize::from(!subgroups),
                "one live oit_alloc"
            );
            let src = format!("{}\n{}\n{}", crate::COMMON_SHADER, generated, include_str!("shaders/env.wgsl"));
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
            let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
            if let Err(e) = v.validate(&module) {
                panic!("early {early}, subgroups {subgroups}: {}", e.emit_to_string(&src));
            }
        }
    }
}
