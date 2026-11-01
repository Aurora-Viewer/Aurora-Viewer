//! The renderer as the scene sees it: the main thread's half.
//!
//! Who owns what:
//!
//! - **Main thread, here.** What to draw: the scene's stores and their CPU
//!   side — draw records and their face table ([`RecordStore`]), the object
//!   table of the GPU culling ([`CullTables`]), the geometry arena's
//!   allocators, the texture pages, slots and locations ([`TextureTable`]),
//!   joint palettes and skin bindings, the staging memory and the copies
//!   streamed from it ([`UploadQueue`]), the settings and the window size.
//!   The main thread creates GPU resources (creation has no ordering and
//!   the device is shared), but it never writes to them and never submits:
//!   every write goes to the journal of the frame being built (writes.rs).
//! - **Render thread, `Backend` (renderer.rs).** How to draw: the surface,
//!   pipelines, render targets, per-frame uniforms, egui, the compute
//!   culling and its bins, occlusion, readbacks. It is the only user of the
//!   queue.
//!
//! [`Renderer::render`] closes the frame on the main thread: the mirrors'
//! dirty ranges join the journal, and the journal, the streamed copies, the
//! handles of the scene's buffers as they are now, the camera, the CPU
//! draw lists and the interface leave as one frame packet (packet.rs). The
//! render thread replays the journal, then draws; meanwhile the main thread
//! is already on the next frame, whose writes go to another journal. So a
//! frame is always drawn with the data of one frame, whole.
//!
//! What comes back with the next hand-over (render_thread.rs): the frame's
//! statistics and the readbacks, each tied to the frame that asked —
//! the depth under the cursor (kept with the camera it was drawn with), the
//! culling counts, GPU timers. Two things wait for the render thread
//! instead: a frame that carries a capture (the same frame is captured as
//! before, and the caller reads `captured` right after), and the depth
//! under a click ([`Renderer::pick_world`]), which runs after the frame in
//! flight.

use crate::arena::{GeometryArena, MeshAlloc, RecordStore};
use crate::device_lost::DeviceLost;
use crate::gpu_cull::{CullTables, GpuCull};
use crate::packet::{FramePacket, FrameResult, SceneGpu, Spent, SurfaceState, capture_for};
use crate::render_thread::{Done, Host, Job, thread_wanted};
use crate::renderer::{Backend, BackendInit, RenderError};
use crate::textures::{MipLevel, TextureTable};
use crate::types::*;
use crate::upload::{StagedMesh, StagedTexture, StagingPool, UploadQueue};
use crate::writes::GpuWrites;
use glam::{Mat4, Vec3, Vec4};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub driver: String,
    pub bindless: bool,
    pub timestamps: bool,
    /// Elements a shader can bind in the array of texture pages: the pages
    /// the texture table can hold (its textures: `TextureTable::capacity`).
    pub max_textures: u32,
    /// Dedicated video memory (MB) when it can be detected.
    pub vram_mb: Option<u64>,
    /// Integrated GPU (shares the system memory).
    pub integrated: bool,
    /// The PCI vendor id and the largest 2D texture side the adapter takes
    /// (Linden Lab's viewer statistics).
    pub vendor_id: u32,
    pub max_texture_size: u32,
    /// The names of the adapters that can present to the window, in the
    /// log's order: the choice of Preferences › Graphics.
    pub adapters: Vec<String>,
    /// Why the dedicated GPU may not be the one drawing (shown under that
    /// choice too).
    pub notice: Option<crate::adapter_choice::Notice>,
}

/// The part of a range of skin bindings to send that is in a list of `len`.
fn skin_binds_range(dirty: std::ops::Range<usize>, len: usize) -> std::ops::Range<usize> {
    dirty.start.min(len)..dirty.end.min(len)
}

/// Packet buffers kept for the next frames (one in flight, one being built).
const SPARE_PACKETS: usize = 2;

/// A frame the GPU has kept this long is a stall, not a slow frame (the
/// slowest real frames, a first frame with every pipeline to make, take
/// well under a second on the render thread).
pub const STALL_AFTER: Duration = Duration::from_secs(2);
/// While stalled, how long each frame waits for the GPU before the main
/// thread goes on: the window stays alive, the loop does not spin.
const STALL_POLL: Duration = Duration::from_millis(50);
/// How long a frame carrying a capture is waited for.
const CAPTURE_WAIT: Duration = Duration::from_secs(10);
/// The longest the frame pacing waits for the GPU (pacing.rs): a frame of
/// 10 frames per second, well past any frame drawn in a busy region.
pub(crate) const GPU_WAIT: Duration = Duration::from_millis(100);

/// Interface texture changes kept while the GPU is stalled. Cleared when
/// dropped with the renderer: egui's debug builds refuse to drop a delta
/// not applied, and nobody draws any more then.
#[derive(Default)]
struct HeldTextures(egui::TexturesDelta);

impl Drop for HeldTextures {
    fn drop(&mut self) {
        self.0.clear();
    }
}

/// Handles of the scene's buffers (reference counted: no GPU work).
fn scene_gpu(
    records: &RecordStore,
    cull: &CullTables,
    textures: &TextureTable,
    geometry: &GeometryArena,
    palettes: &wgpu::Buffer,
    prev_palettes: &wgpu::Buffer,
    skin_binds: &wgpu::Buffer,
    bind_generation: u64,
) -> SceneGpu {
    SceneGpu {
        records: records.buffer.clone(),
        palettes: palettes.clone(),
        skin_binds: skin_binds.clone(),
        prev_models: records.prev_models.clone(),
        prev_palettes: prev_palettes.clone(),
        records_key: (records.generation, bind_generation),
        faces: records.faces.buffer().clone(),
        face_count: records.faces.len(),
        objects: cull.objects.buffer().clone(),
        object_count: cull.objects.len(),
        joint_boxes: cull.boxes.buffer().clone(),
        record_slots: records.slot_count(),
        textures: textures.bind_group.clone(),
        vertices: geometry.vertex_buffer.clone(),
        skin: geometry.skin_buffer.clone(),
        indices: geometry.index_buffer.clone(),
    }
}

pub struct Renderer {
    /// Where the backend runs. First field: dropped first, so the render
    /// thread has left before the scene's stores go.
    host: Host,
    vr: Arc<crate::vr_control::Control>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    _xr_graphics: Option<crate::xr_device::Graphics>,
    pub device: wgpu::Device,
    pub info: GpuInfo,
    pub geometry: GeometryArena,
    /// Copies of the streamed data staged by background jobs (upload.rs),
    /// submitted first in the frame's submit.
    pub uploads: UploadQueue,
    pub records: RecordStore,
    pub textures: TextureTable,
    /// Video memory budget and use of the moment, as the driver reports
    /// them (vram.rs).
    pub video_memory: crate::vram::VideoMemoryProbe,
    /// The skin bind or palette buffer could not grow: when to try again.
    table_backoff: crate::gpu_alloc::Backoff,
    /// The scene's tables of the GPU-driven draw lists (gpu_cull.rs).
    pub cull: CullTables,
    /// Queue writes of the frame being built (writes.rs).
    writes: GpuWrites,
    palette_buffer: wgpu::Buffer,
    /// The palettes as the last frame drew them (the motion vectors): the
    /// renderer copies `palette_buffer` into it at the end of each frame
    /// (`motion_history.rs`). The same size, unless video memory was full
    /// when the palettes grew.
    prev_palette_buffer: wgpu::Buffer,
    /// The next frame starts the motion history again (`cut_motion_history`).
    motion_cut: bool,
    /// Palette bytes written since the last frame (AURORA_PROFILE).
    palettes_uploaded: u64,
    /// Bytes of vertices deformed in place this frame (profile).
    vertices_uploaded: u64,
    /// Bytes of geometry written into the arena since the start (vertices,
    /// skin weights, indices): what the profile reports per second.
    pub geometry_uploaded: u64,
    skin_bind_buffer: wgpu::Buffer,
    /// Bumped when the palette or skin binding buffer is reallocated (the
    /// render thread rebuilds the records bind group).
    bind_generation: u64,
    /// The device can draw the GPU lists (and AURORA_CPU_CULL is not set).
    gpu_cull: bool,
    size: (u32, u32),
    vsync: bool,
    settings: RenderSettings,
    msaa_supported: Vec<u32>,
    /// The surface can be read back (captures).
    can_capture: bool,
    output_format: wgpu::TextureFormat,
    /// Texture animation clock (tex_anim.rs), shared with the backend.
    anim_clock: crate::tex_anim::AnimClock,
    /// Inverse view-projection of the last frame built (cursor rays).
    last_inv_vp: Mat4,
    last_view: Mat4,
    /// Pixel to read the depth of in the next frame, and the last answer.
    hover_request: Option<(u32, u32)>,
    hover_latest: Option<Vec3>,
    /// Clicks whose depth leaves with the next frame, the answers that
    /// came back, and the ticket of the next click.
    pick_requests: Vec<crate::pick::ClickRequest>,
    pick_answers: Vec<crate::pick::ClickAnswer>,
    pick_ticket: u64,
    /// The same for the faces drawn under a pixel (`request_drawn`).
    drawn_requests: Vec<crate::drawn_pick::DrawnRequest>,
    drawn_answers: Vec<crate::drawn_pick::DrawnAnswer>,
    /// Frames handed over.
    frames: u64,
    /// The renderer's side of the last frame result, and whether it came
    /// since the last `render` returned.
    last_stats: RenderStats,
    fresh: bool,
    /// Frame pacing (pacing.rs, on unless AURORA_FRAME_PACING=0): what the
    /// GPU has finished, as the backend tells it; the last wait.
    gpu_frames: Arc<crate::pacing::GpuFrames<wgpu::SubmissionIndex>>,
    gpu_waited: Duration,
    /// When the frame being built began reading its input.
    input_at: Option<Instant>,
    /// The same for the pacing's gate (`input_at` leaves with the packet),
    /// and the gate: whether the GPU alone sets the pace (pacing.rs).
    frame_began: Option<Instant>,
    pace_gate: crate::pacing::Gate,
    spare: Vec<Spent>,
    failed: bool,
    /// The render thread has held a frame past [`STALL_AFTER`]: frames are
    /// not built until it is free again (see `render`).
    stalled: bool,
    /// The interface's texture changes of the frames not built meanwhile:
    /// they leave with the next one (a lost font atlas update would leave
    /// glyphs missing for the rest of the session).
    held_textures: HeldTextures,
    /// Set to request a copy of the next presented frame.
    pub capture_request: bool,
    /// Capture the 3D view without the interface (next frame) into `captured`.
    pub capture_scene: bool,
    /// Last captured frame (width, height, RGBA8).
    pub captured: Option<(u32, u32, Vec<u8>)>,
    /// A photo for the next frame: it is drawn offscreen at its own size
    /// (see `frame_size`), waited for, and its image put in `snapshot`.
    pub snapshot_request: Option<crate::SnapshotSpec>,
    /// The answer to the last photo: its image (width, height, RGBA8), or
    /// None when it could not be drawn.
    pub snapshot: Option<Option<(u32, u32, Vec<u8>)>>,
    /// The photo asked left with a frame the GPU had not finished within
    /// `CAPTURE_WAIT`: its answer comes with a later frame, which does not
    /// draw it again (a second answer would be taken for the next part).
    snapshot_sent: bool,
    /// That photo was given up (`drop_snapshot`): its answer is not kept.
    snapshot_dropped: bool,
    /// Count the luminance histogram of the next frame shown at that
    /// exposure (asked each frame the photo viewfinder shows it).
    pub histogram_request: Option<f32>,
    /// The last histogram counted, from black to white.
    pub histogram: Option<Box<[u32; crate::HISTOGRAM_BINS]>>,
    /// Set by the device-lost callback (device_lost.rs).
    lost: DeviceLost,
    /// AURORA_DEMO_GPU_LOST: the frame the device is taken as lost at.
    demo_lost: Option<u64>,
}

impl Renderer {
    /// Create the renderer on the native graphics backend for the given window, on
    /// the GPU `gpu` asks for (`adapter_choice::Wanted`: empty for the
    /// automatic choice).
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
        vsync: bool,
        gpu: &str,
    ) -> Result<Renderer, RenderError> {
        let desc = || {
            let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
            desc.backends = crate::BACKENDS;
            desc.flags = if std::env::var_os("AURORA_GPU_VALIDATION").is_some() {
                wgpu::InstanceFlags::debugging()
            } else {
                wgpu::InstanceFlags::empty()
            };
            desc
        };
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let xr_graphics = match crate::xr_device::Graphics::new(desc().flags) {
            Ok(graphics) => Some(graphics),
            Err(e) => {
                log::info!("OpenXR initialization unavailable: {e}");
                if std::env::var_os("AURORA_DEMO_XR_DEVICE").is_some() || std::env::var_os("AURORA_DEMO_XR_WORLD").is_some() {
                    return Err(RenderError::Device(e.to_string()));
                }
                None
            }
        };
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let mut instance = xr_graphics
            .as_ref()
            .map_or_else(|| wgpu::Instance::new(desc()), |g| g.instance.clone());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let mut instance = wgpu::Instance::new(desc());
        // ---- the GPU: every adapter listed, then tried in our order
        // (adapter_choice.rs), the next one if one fails to open
        let mut adapters = pollster::block_on(instance.enumerate_adapters(crate::BACKENDS));
        // Dozen, kept out at start (vulkan_drivers.rs), may be the only
        // Vulkan driver of this PC: let it back in rather than draw nothing
        if adapters.is_empty() && crate::vulkan_drivers::allow_again() {
            log::warn!("GPU: no Vulkan adapter without Dozen (Vulkan on Direct3D 12): trying again with it");
            instance = wgpu::Instance::new(desc());
            adapters = pollster::block_on(instance.enumerate_adapters(crate::BACKENDS));
        }
        let surface = instance.create_surface(target).map_err(|e| RenderError::Surface(e.to_string()))?;
        let candidates: Vec<crate::adapter_choice::Candidate> = adapters
            .iter()
            .map(|a| {
                let info = a.get_info();
                crate::adapter_choice::Candidate {
                    name: info.name,
                    kind: crate::adapter_choice::Kind::from_wgpu(info.device_type),
                    vram_mb: detect_vram_mb(a),
                    surface: a.is_surface_supported(&surface),
                }
            })
            .collect();
        for (i, c) in candidates.iter().enumerate() {
            log::info!(
                "GPU adapter {i}: {} ({}, {}, {})",
                c.name,
                c.kind.label(),
                c.vram_mb.map(|m| format!("{m} MB")).unwrap_or_else(|| "memory unknown".into()),
                if c.surface { "can present" } else { "cannot present to the window" }
            );
        }
        let notice = if cfg!(target_os = "macos") {
            None
        } else {
            crate::adapter_choice::hint(&candidates)
        };
        if let Some(n) = notice {
            log::warn!("GPU: {}", n.log_line());
        }
        let wanted = crate::adapter_choice::Wanted::parse(gpu);
        let plan = crate::adapter_choice::plan(&candidates, &wanted);
        if plan.reason == crate::adapter_choice::Reason::WantedMissing {
            log::warn!(
                "GPU: \"{}\" asked for, not found among the adapters that can present: automatic choice",
                gpu.trim()
            );
        }
        let mut opened = None;
        let mut failure = RenderError::NoAdapter("no native GPU adapter can present to the window".into());
        for (n, &i) in plan.order.iter().enumerate() {
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            let opened_device = open_device(&adapters[i], xr_graphics.as_ref());
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            let opened_device = open_device(&adapters[i]);
            match opened_device {
                Ok(o) => {
                    let why = match (n, plan.reason) {
                        (0, crate::adapter_choice::Reason::Wanted) => "the one asked for (AURORA_GPU or Preferences › Graphics)",
                        (0, _) => "automatic: discrete before integrated, then the most video memory",
                        _ => "the ones before it failed to open",
                    };
                    log::info!("GPU adapter {i} chosen: {why}");
                    opened = Some((i, o));
                    break;
                }
                Err(e) => {
                    log::warn!("GPU adapter {i} ({}) could not be opened: {e}", candidates[i].name);
                    failure = e;
                }
            }
        }
        let (index, o) = opened.ok_or(failure)?;
        let adapter = adapters.swap_remove(index);
        let vram_mb = candidates[index].vram_mb;
        let ainfo = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, driver {} {})",
            ainfo.name,
            ainfo.backend,
            ainfo.driver,
            ainfo.driver_info
        );
        log::info!(
            "GPU memory: {}",
            vram_mb.map(|m| format!("{m} MB")).unwrap_or_else(|| "unknown".into())
        );
        let OpenedDevice {
            device,
            queue,
            alim,
            bindless,
            timestamps,
            adapter_specific,
            max_textures,
        } = o;
        let adapters: Vec<String> = candidates.into_iter().filter(|c| c.surface).map(|c| c.name).collect();
        // The same error every frame (a write into a buffer that could not
        // be made) wrote millions of lines a minute: once, then a count.
        let throttle = std::sync::Mutex::new(crate::gpu_alloc::LogThrottle::new(std::time::Duration::from_secs(10)));
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let line = throttle
                .lock()
                .ok()
                .and_then(|mut t| t.note(&format!("wgpu error: {e}"), std::time::Instant::now()));
            if let Some(line) = line {
                log::error!("{line}");
            }
        }));
        // A driver reset (TDR), a removed card or another program taking the
        // GPU: said in the log, where the crash report finds it, and nothing
        // waits on the device any more (device_lost.rs). `Destroyed` is the
        // device's own end at shutdown.
        let lost = DeviceLost::default();
        let on_lost = lost.clone();
        device.set_device_lost_callback(move |reason, message| match reason {
            wgpu::DeviceLostReason::Destroyed => log::info!("GPU device released: {message}"),
            _ => {
                on_lost.set();
                log::error!("GPU device lost ({reason:?}): {message}");
            }
        });
        let info = GpuInfo {
            name: ainfo.name.clone(),
            backend: format!("{:?}", ainfo.backend),
            driver: format!("{} {}", ainfo.driver, ainfo.driver_info),
            bindless,
            timestamps,
            max_textures,
            vram_mb,
            integrated: ainfo.device_type == wgpu::DeviceType::IntegratedGpu,
            vendor_id: ainfo.vendor,
            max_texture_size: alim.max_texture_dimension_2d,
            adapters,
            notice,
        };

        // ---- the scene's stores; what they write at creation is replayed
        // with the first frame
        let mut writes = GpuWrites::default();
        // (as many textures as the table has slots; `max_textures` bounds
        // the pages they are grouped in)
        let slots = crate::textures::slot_capacity(std::env::var("AURORA_TEXTURE_SLOTS").ok().as_deref());
        let textures = TextureTable::new(&device, &mut writes, slots, max_textures, 8);
        let geometry = GeometryArena::new(&device, alim.max_buffer_size.min(1 << 31));
        let records = RecordStore::new(&device);
        let cull = CullTables::new(&device);
        let palette_buffer = Self::make_palette_buffer(&device, 64 * 160);
        let prev_palette_buffer = device.create_buffer(&Self::prev_palette_desc(64 * 160));
        let skin_bind_buffer = Self::make_skin_bind_buffer(&device, 256);
        // staging memory of the streamed uploads, created once here
        let uploads = UploadQueue::new(&device, info.integrated || info.vram_mb.is_some_and(|mb| mb < 4096));
        let gpu_cull = GpuCull::supported(&device);
        log::info!(
            "draw lists: {}",
            if gpu_cull {
                "culled on the GPU (multi_draw_indexed_indirect_count)"
            } else {
                "built on the CPU (no MULTI_DRAW_INDIRECT_COUNT, or AURORA_CPU_CULL)"
            }
        );
        let anim_clock = crate::tex_anim::AnimClock::new(Instant::now());
        let video_memory = crate::vram::VideoMemoryProbe::new(adapter.clone());
        let pacing = std::env::var("AURORA_FRAME_PACING").ok();
        let gpu_frames = Arc::new(crate::pacing::GpuFrames::new(crate::pacing::wanted(pacing.as_deref())));
        let pace_gate = crate::pacing::Gate::new(crate::pacing::always(pacing.as_deref()));
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let vr = Arc::new(crate::vr_control::Control::new(xr_graphics.is_some()));
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let vr = Arc::new(crate::vr_control::Control::new(false));
        let backend = Backend::new(BackendInit {
            vr: vr.clone(),
            gpu_frames: gpu_frames.clone(),
            device: device.clone(),
            queue,
            surface: Arc::new(surface),
            external: false,
            output_format: None,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            xr: xr_graphics.as_ref().map(|g| g.binding()),
            adapter,
            width,
            height,
            vsync,
            timestamps,
            adapter_specific,
            gpu_cull,
            tex_layout: textures.layout.clone(),
            scene: scene_gpu(
                &records,
                &cull,
                &textures,
                &geometry,
                &palette_buffer,
                &prev_palette_buffer,
                &skin_bind_buffer,
                0,
            ),
            anim_clock,
            lost: lost.clone(),
        })?;
        let msaa_supported = backend.msaa_supported().to_vec();
        let can_capture = backend.can_capture();
        let output_format = backend.output_format();
        let host = Host::new(backend, thread_wanted(std::env::var("AURORA_RENDER_THREAD").ok().as_deref()));
        log::info!(
            "renderer: {}",
            if host.threaded() {
                "frames drawn by the render thread"
            } else {
                "frames drawn on the main thread (AURORA_RENDER_THREAD=0)"
            }
        );
        Ok(Renderer {
            host,
            vr,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            _xr_graphics: xr_graphics,
            device,
            info,
            geometry,
            uploads,
            records,
            textures,
            video_memory,
            table_backoff: Default::default(),
            cull,
            writes,
            palette_buffer,
            prev_palette_buffer,
            motion_cut: false,
            palettes_uploaded: 0,
            vertices_uploaded: 0,
            geometry_uploaded: 0,
            skin_bind_buffer,
            bind_generation: 0,
            gpu_cull,
            size: (width.max(1), height.max(1)),
            vsync,
            settings: RenderSettings::default(),
            msaa_supported,
            can_capture,
            output_format,
            anim_clock,
            last_inv_vp: Mat4::IDENTITY,
            last_view: Mat4::IDENTITY,
            hover_request: None,
            hover_latest: None,
            pick_requests: Vec::new(),
            pick_answers: Vec::new(),
            pick_ticket: 0,
            drawn_requests: Vec::new(),
            drawn_answers: Vec::new(),
            frames: 0,
            last_stats: RenderStats::default(),
            fresh: false,
            gpu_frames,
            gpu_waited: Duration::ZERO,
            input_at: None,
            frame_began: None,
            pace_gate,
            spare: Vec::new(),
            failed: false,
            stalled: false,
            held_textures: HeldTextures::default(),
            capture_request: false,
            capture_scene: false,
            captured: None,
            snapshot_request: None,
            snapshot: None,
            snapshot_sent: false,
            snapshot_dropped: false,
            histogram_request: None,
            histogram: None,
            lost,
            demo_lost: crate::device_lost::demo_frame(
                std::env::var_os("AURORA_DEMO").is_some(),
                std::env::var("AURORA_DEMO_GPU_LOST").ok().as_deref(),
            ),
        })
    }

    /// The scene's buffers as they are now, for a frame packet.
    fn scene_gpu(&self) -> SceneGpu {
        scene_gpu(
            &self.records,
            &self.cull,
            &self.textures,
            &self.geometry,
            &self.palette_buffer,
            &self.prev_palette_buffer,
            &self.skin_bind_buffer,
            self.bind_generation,
        )
    }

    fn make_skin_bind_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
        device.create_buffer(&Self::skin_bind_desc(count))
    }

    fn skin_bind_desc(count: usize) -> wgpu::BufferDescriptor<'static> {
        wgpu::BufferDescriptor {
            label: Some("skin binds"),
            size: (count.max(1) * std::mem::size_of::<SkinBind>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }
    }

    /// A larger table buffer (skin binds, palettes), or None when video
    /// memory is full: the caller keeps the one it has and writes what fits
    /// in it, and the growth is tried again a little later.
    fn grow_table(&mut self, desc: &wgpu::BufferDescriptor) -> Option<wgpu::Buffer> {
        let now = std::time::Instant::now();
        if !self.table_backoff.ready(now) {
            return None;
        }
        match crate::gpu_alloc::try_buffer(&self.device, desc) {
            Ok(b) => {
                self.table_backoff.succeeded();
                Some(b)
            }
            Err(e) => {
                if !self.table_backoff.waiting() {
                    log::warn!(
                        "{} not grown to {} MB ({e}): video memory is full, what does not fit is not drawn until it grows",
                        desc.label.unwrap_or("buffer"),
                        desc.size >> 20
                    );
                }
                self.table_backoff.failed(now);
                None
            }
        }
    }

    /// Upload the skin bindings (inverse bind matrix + palette joint per
    /// mesh joint) referenced by `DrawRecord::flags[2]` of skinned records.
    /// Only the `dirty` entries of the list are written (a rigged mesh sets
    /// the range of its joints, at the end or in the place of a mesh gone),
    /// unless the buffer had to grow. The whole list sent again for every
    /// new rigged mesh was megabytes a frame while a crowd loads.
    pub fn set_skin_binds(&mut self, binds: &[SkinBind], dirty: std::ops::Range<usize>) {
        let size = std::mem::size_of::<SkinBind>();
        let mut dirty = skin_binds_range(dirty, binds.len());
        if std::mem::size_of_val(binds) as u64 > self.skin_bind_buffer.size() {
            let mut n = (self.skin_bind_buffer.size() as usize / size).max(1);
            while n < binds.len() {
                n *= 2;
            }
            if let Some(b) = self.grow_table(&Self::skin_bind_desc(n)) {
                self.skin_bind_buffer = b;
                self.bind_generation += 1;
                dirty = 0..binds.len();
            }
        }
        // (what the buffer has no room for waits for its growth)
        let fits = self.skin_bind_buffer.size() as usize / size;
        let dirty = dirty.start.min(fits)..dirty.end.min(fits);
        if !dirty.is_empty() {
            self.writes.write_buffer(
                &self.skin_bind_buffer,
                (dirty.start * size) as u64,
                bytemuck::cast_slice(&binds[dirty]),
            );
        }
    }

    fn make_palette_buffer(device: &wgpu::Device, matrices: usize) -> wgpu::Buffer {
        device.create_buffer(&Self::palette_desc(matrices))
    }

    fn palette_desc(matrices: usize) -> wgpu::BufferDescriptor<'static> {
        wgpu::BufferDescriptor {
            label: Some("joint palettes"),
            size: (matrices.max(1) * 64) as u64,
            // COPY_SRC: the previous palettes are copied from it
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }
    }

    fn prev_palette_desc(matrices: usize) -> wgpu::BufferDescriptor<'static> {
        wgpu::BufferDescriptor {
            label: Some("previous joint palettes"),
            size: (matrices.max(1) * 64) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }
    }

    /// Palette slots given to new skeletons, after this frame's palette
    /// uploads: each is written whole, as its pose and as its previous
    /// pose, or it would seem to come from the last owner's (or from
    /// nothing).
    pub fn seed_palette_slots(&mut self, mats: &[[[f32; 4]; 4]], slots: &[usize], per: usize) {
        let fits = mats.len().min(self.palette_buffer.size() as usize / 64);
        let prev_fits = fits.min(self.prev_palette_buffer.size() as usize / 64);
        for &slot in slots {
            let a = slot * per;
            for (buffer, end) in [(&self.palette_buffer, fits), (&self.prev_palette_buffer, prev_fits)] {
                let b = ((slot + 1) * per).min(end);
                if a < b {
                    self.writes.write_buffer(buffer, (a * 64) as u64, bytemuck::cast_slice(&mats[a..b]));
                }
            }
        }
    }

    /// The next frame has no motion: the scene moved as a whole (a new main
    /// region shifts every position), so last frame's matrices are not
    /// where this frame's things came from. The renderer takes the current
    /// ones as the previous ones before drawing it.
    pub fn cut_motion_history(&mut self) {
        self.motion_cut = true;
    }

    /// Upload joint palettes (column-major matrices). Indices referenced by
    /// `DrawRecord::flags[1]` of skinned records.
    pub fn set_palettes(&mut self, mats: &[[[f32; 4]; 4]]) {
        if mats.is_empty() {
            return;
        }
        let bytes = (mats.len() * 64) as u64;
        if bytes > self.palette_buffer.size() {
            let mut n = (self.palette_buffer.size() / 64) as usize;
            while n * 64 < bytes as usize {
                n *= 2;
            }
            if let Some(b) = self.grow_table(&Self::palette_desc(n)) {
                self.palette_buffer = b;
                self.bind_generation += 1;
                // (the whole set is written below: the previous palettes
                // start from it, no skeleton moves in this frame)
                match crate::gpu_alloc::try_buffer(&self.device, &Self::prev_palette_desc(n)) {
                    Ok(prev) => self.prev_palette_buffer = prev,
                    Err(e) => log::warn!("previous joint palettes not grown ({e}): the skeletons past them get no motion"),
                }
                let prev = &mats[..mats.len().min(self.prev_palette_buffer.size() as usize / 64)];
                self.writes.write_buffer(&self.prev_palette_buffer, 0, bytemuck::cast_slice(prev));
            }
        }
        let mats = &mats[..mats.len().min(self.palette_buffer.size() as usize / 64)];
        self.writes.write_buffer(&self.palette_buffer, 0, bytemuck::cast_slice(mats));
        self.palettes_uploaded += (mats.len() * 64) as u64;
    }

    /// Upload only some palettes (`per` matrices each, by slot index): the
    /// skeletons posed this frame. Everything when the buffer must grow.
    pub fn set_palette_slots(&mut self, mats: &[[[f32; 4]; 4]], slots: &[usize], per: usize) {
        if (mats.len() * 64) as u64 > self.palette_buffer.size() {
            self.set_palettes(mats);
            return;
        }
        let mut sorted = slots.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut i = 0;
        while i < sorted.len() {
            // neighbouring slots in one write
            let first = sorted[i];
            let mut last = first;
            while i + 1 < sorted.len() && sorted[i + 1] == last + 1 {
                i += 1;
                last = sorted[i];
            }
            i += 1;
            let (a, b) = (first * per, ((last + 1) * per).min(mats.len()));
            if a < b {
                self.writes
                    .write_buffer(&self.palette_buffer, (a * 64) as u64, bytemuck::cast_slice(&mats[a..b]));
                self.palettes_uploaded += ((b - a) * 64) as u64;
            }
        }
    }

    /// Upload one range of the palette matrices (the anchors of the objects
    /// an avatar wears, when its joints were not posed again). Everything
    /// when the buffer must grow.
    pub fn set_palette_range(&mut self, mats: &[[[f32; 4]; 4]], range: std::ops::Range<usize>) {
        if (mats.len() * 64) as u64 > self.palette_buffer.size() {
            self.set_palettes(mats);
            return;
        }
        let (a, b) = (range.start.min(mats.len()), range.end.min(mats.len()));
        if a < b {
            self.writes
                .write_buffer(&self.palette_buffer, (a * 64) as u64, bytemuck::cast_slice(&mats[a..b]));
            self.palettes_uploaded += ((b - a) * 64) as u64;
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The size the next frame is drawn at: the photo's when one is asked
    /// (its projection, levels of detail and HUD are laid out for it), the
    /// window's otherwise.
    pub fn frame_size(&self) -> (u32, u32) {
        self.snapshot_request.map_or(self.size, |s| (s.width, s.height))
    }

    /// The largest side a photo can be drawn at in one pass.
    pub fn max_texture_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// The window was resized: the swapchain and the render targets follow
    /// with the next frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = (width, height);
    }

    /// The draw lists can be culled on the GPU (`DrawLists::gpu`).
    pub fn gpu_culling(&self) -> bool {
        self.gpu_cull
    }

    /// Supported MSAA sample counts (besides 1).
    pub fn msaa_supported(&self) -> &[u32] {
        &self.msaa_supported
    }

    /// Frames are drawn by the render thread (false: AURORA_RENDER_THREAD=0).
    pub fn threaded(&self) -> bool {
        self.host.threaded()
    }

    /// The render thread is gone (it panicked): nothing can be drawn any
    /// more, the viewer should close.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// The GPU device is lost for good (driver reset, card removed): nothing
    /// is drawn any more, the viewer should close.
    pub fn device_lost(&self) -> bool {
        self.lost.get()
    }

    /// How long the GPU has kept the frame in the render thread's hands,
    /// once that is past [`STALL_AFTER`]: no frame is drawn meanwhile, the
    /// window keeps answering. None while frames come.
    pub fn stalled_for(&self) -> Option<Duration> {
        let Host::Thread { owner, .. } = &self.host else {
            return None;
        };
        self.stalled.then(|| owner.held_for()).flatten()
    }

    /// Before a frame is built: wait for the render thread to be free, at
    /// most [`STALL_AFTER`] (then [`STALL_POLL`] while it stays stuck).
    /// False when it is still held: the stores keep their writes for the
    /// frame that will leave once it is free, nothing is lost or replayed
    /// twice. Never waits on the GPU without a bound: a tester's window
    /// stayed white and frozen after login, its main thread parked here.
    fn render_thread_free(&mut self) -> bool {
        let Host::Thread { owner, .. } = &self.host else {
            return true;
        };
        let wait = if self.stalled { STALL_POLL } else { STALL_AFTER };
        match owner.wait_until_free(Instant::now() + wait) {
            Ok(true) => {
                if std::mem::take(&mut self.stalled) {
                    log::warn!("the graphics card finished its frame again: drawing resumes");
                }
                true
            }
            Ok(false) => {
                if !std::mem::replace(&mut self.stalled, true) {
                    log::warn!(
                        "the graphics card has not finished a frame for {:?}: frames wait, the window keeps answering",
                        owner.held_for().unwrap_or(STALL_AFTER)
                    );
                }
                false
            }
            Err(_) => {
                self.fail();
                false
            }
        }
    }

    /// Give up the photo asked: not drawn, or its answer not kept when its
    /// frame is still on the GPU.
    pub fn drop_snapshot(&mut self) {
        self.snapshot_request = None;
        self.snapshot_dropped = self.snapshot_sent;
    }

    /// Apply quality settings: pipelines and targets are rebuilt by the
    /// backend with the next frame, only when needed.
    pub fn apply_settings(&mut self, s: RenderSettings) {
        if s.anisotropy != self.settings.anisotropy {
            self.textures.set_anisotropy(&self.device, s.anisotropy);
        }
        self.settings = s;
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        self.vsync = vsync;
    }

    pub fn vsync(&self) -> bool {
        self.vsync
    }

    /// Presentation mode the swapchain is configured with, as the backend
    /// reported it with the last frame it finished (None before the first).
    pub fn present_mode(&self) -> Option<wgpu::PresentMode> {
        self.last_stats.present_mode
    }

    /// The last frame the backend finished was drawn with the settings last
    /// applied: false while the pipelines for new MSAA samples or SSR compile
    /// on their worker (#1746), and until the backend has seen them.
    pub fn settings_applied(&self) -> bool {
        self.last_stats.settings == Some(self.settings)
    }

    // ------------------------------------------------------------ resources

    pub fn upload_mesh(&mut self, vertices: &[Vertex], indices: &[u16]) -> Option<MeshAlloc> {
        self.geometry_uploaded += (std::mem::size_of_val(vertices) + std::mem::size_of_val(indices)) as u64;
        self.geometry
            .alloc(&self.device, &mut self.writes, &mut self.uploads, vertices, None, indices)
    }

    pub fn upload_skinned_mesh(&mut self, vertices: &[Vertex], skin: &[SkinVertex], indices: &[u16]) -> Option<MeshAlloc> {
        self.geometry_uploaded += (std::mem::size_of_val(vertices) + std::mem::size_of_val(skin) + std::mem::size_of_val(indices)) as u64;
        self.geometry
            .alloc(&self.device, &mut self.writes, &mut self.uploads, vertices, Some(skin), indices)
    }

    /// Upload a mesh staged by a background job (`mesh.data.ready()` must
    /// hold): the main thread only records the copies.
    pub fn upload_mesh_staged(&mut self, mesh: &StagedMesh) -> Option<MeshAlloc> {
        let skin = if mesh.skinned { std::mem::size_of::<SkinVertex>() } else { 0 };
        self.geometry_uploaded +=
            (mesh.vertex_count as usize * (std::mem::size_of::<Vertex>() + skin) + mesh.index_count as usize * 2) as u64;
        self.geometry.alloc_staged(&self.device, &mut self.writes, &mut self.uploads, mesh)
    }

    pub fn free_mesh(&mut self, m: MeshAlloc) {
        self.geometry.free(m);
    }

    /// Grow the geometry arena now to hold that many vertices and indices
    /// (`GeometryArena::grow_ahead`): while a loading screen hides what
    /// the growth costs the GPU, and within what the video memory budget
    /// leaves free (another program or viewer may hold the rest).
    pub fn grow_geometry_ahead(&mut self, vertices: u32, indices: u32) -> bool {
        let allowance = self
            .video_memory
            .read(&self.device)
            .map(|m| crate::gpu_alloc::ahead_allowance(m.budget, m.usage));
        self.geometry
            .grow_ahead(&self.device, &mut self.writes, &mut self.uploads, vertices, indices, allowance)
    }

    /// Deform an existing allocation without touching indices or draw records.
    /// The write is queued with this frame; the render thread never waits.
    pub fn update_mesh_vertices(&mut self, mesh: MeshAlloc, vertices: &[Vertex]) -> bool {
        if vertices.len() != mesh.vertex_count as usize {
            return false;
        }
        self.writes.write_buffer(
            &self.geometry.vertex_buffer,
            mesh.vertex_offset as u64 * std::mem::size_of::<Vertex>() as u64,
            bytemuck::cast_slice(vertices),
        );
        self.vertices_uploaded += std::mem::size_of_val(vertices) as u64;
        true
    }

    pub fn create_texture(&mut self, mips: &[MipLevel]) -> Option<u32> {
        self.textures.create(&self.device, &mut self.writes, mips)
    }

    pub fn replace_texture(&mut self, slot: u32, mips: &[MipLevel]) -> bool {
        self.textures.replace(&self.device, &mut self.writes, slot, mips)
    }

    /// Staging memory shared with the background jobs (upload.rs).
    pub fn staging_pool(&self) -> Arc<StagingPool> {
        self.uploads.pool().clone()
    }

    /// Once a frame before the streamed copies: what the jobs staged so far
    /// becomes copyable as soon as they are done writing.
    pub fn prepare_uploads(&self) {
        self.uploads.prepare();
    }

    /// Replace a slot's texels with a mip chain staged by a background job
    /// (`staged.data.ready()` must hold): the main thread only records the
    /// copies. False when refused (size), as `replace_texture`.
    pub fn replace_texture_staged(&mut self, slot: u32, staged: &StagedTexture) -> bool {
        self.textures.replace_staged(&self.device, &mut self.uploads, slot, staged)
    }

    /// Overwrite part of a texture's level 0 in place (media frames).
    pub fn update_texture_region(&mut self, slot: u32, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) -> bool {
        self.textures.write_region(&mut self.writes, slot, x, y, w, h, rgba)
    }

    pub fn free_texture(&mut self, slot: u32) {
        self.textures.free(slot);
    }

    // ---------------------------------------------------------------- frame

    /// Clock of the texture animations: the scene puts their time origins on
    /// it, the shaders read the current time from the frame uniforms.
    pub fn anim_clock(&self) -> crate::tex_anim::AnimClock {
        self.anim_clock
    }

    /// Take in what a frame gave back.
    fn apply(&mut self, mut r: FrameResult) {
        self.hover_latest = r.hover;
        if let Some(h) = r.histogram.take() {
            self.histogram = Some(h);
        }
        self.pick_answers.append(&mut r.picks);
        self.drawn_answers.append(&mut r.drawn);
        // (the answer of a click given up while it was in flight is never
        // taken: the next ones push it out)
        const KEPT: usize = 16;
        if self.pick_answers.len() > KEPT {
            self.pick_answers.drain(..self.pick_answers.len() - KEPT);
        }
        if self.drawn_answers.len() > KEPT {
            self.drawn_answers.drain(..self.drawn_answers.len() - KEPT);
        }
        self.cull.last = r.cull;
        if r.snapshot && std::mem::take(&mut self.snapshot_dropped) {
            // the answer of a photo given up: a photo asked since is drawn
            // with the next frame
            self.snapshot_sent = false;
        } else if r.snapshot {
            self.snapshot_request = None;
            self.snapshot_sent = false;
            self.snapshot = Some(r.captured);
        } else if let Some(c) = r.captured {
            self.captured = Some(c);
            self.capture_request = false;
            self.capture_scene = false;
        }
        self.last_stats = r.stats;
        self.fresh = true;
        if self.spare.len() < SPARE_PACKETS {
            self.spare.push(r.spent);
        }
    }

    fn fail(&mut self) {
        if !std::mem::replace(&mut self.failed, true) {
            log::error!("the render thread is gone: nothing can be drawn any more");
        }
        self.fail_picks();
    }

    /// Nobody will read the depth under the clicks still asked: they are
    /// answered like a click on nothing.
    fn fail_picks(&mut self) {
        let asked = self.pick_requests.drain(..).map(|(ticket, _, _)| (ticket, None));
        self.pick_answers.extend(asked);
        let asked = self.drawn_requests.drain(..).map(|(ticket, _, _, _)| (ticket, Default::default()));
        self.drawn_answers.extend(asked);
    }

    /// Close the frame and have it drawn: with a render thread the packet
    /// is handed over (after the frame before it is presented) and this
    /// returns at once, with the statistics of that frame before; without,
    /// the frame is drawn here. A frame that carries a capture is waited
    /// for: `captured` holds it when this returns.
    pub fn vr_state(&self) -> crate::VrState {
        self.vr.state()
    }
    pub fn vr_engaged(&self) -> bool {
        self.vr.state().engaged()
    }
    pub fn set_vr_enabled(&self, on: bool) -> bool {
        self.vr.request(on)
    }

    pub fn render(&mut self, params: FrameParams, lists: &DrawLists, ui: Option<crate::EguiFrame>) -> RenderStats {
        self.render_output(params, lists, ui, None)
    }

    /// Draw a frame into a caller-owned texture instead of presenting the window.
    /// The target must belong to this renderer's device and match its output size
    /// and format. The texture stays alive until the render worker has submitted
    /// the frame; this call alone is not a GPU completion or OpenXR release fence.
    /// No egui shapes are drawn. World draw lists, including HUDs, are supplied
    /// by the caller. Each independent view needs its own rendering history;
    /// alternating eyes through this renderer would mix temporal state.
    pub fn render_to_texture(&mut self, params: FrameParams, lists: &DrawLists, texture: wgpu::Texture) -> RenderStats {
        self.render_output(params, lists, None, Some(texture))
    }

    /// Offline integration check of the complete world pass into an owned
    /// texture. Used only on a requested demo capture, not every frame.
    pub fn render_world_texture_capture(&mut self, params: FrameParams, lists: &DrawLists) -> RenderStats {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("world output integration capture"),
            size: wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mut world = DrawLists::default();
        world.copy_from(lists);
        world.strip_viewer_overlays();
        self.render_to_texture(params, &world, texture)
    }

    fn render_output(
        &mut self,
        params: FrameParams,
        lists: &DrawLists,
        ui: Option<crate::EguiFrame>,
        output: Option<wgpu::Texture>,
    ) -> RenderStats {
        let t0 = Instant::now();
        let mut ui = ui;
        if self.demo_lost == Some(self.frames) && !self.lost.get() {
            self.lost.set();
            log::error!("GPU device lost (Unknown): AURORA_DEMO_GPU_LOST at frame {}", self.frames);
        }
        if self.lost.get() {
            // nothing reaches a lost device: no frame is built, the photo
            // asked is answered as failed, the clicks as on nothing
            self.writes.clear();
            self.fail_picks();
            if self.snapshot_request.take().is_some() {
                self.snapshot = Some(None);
            }
            self.snapshot_sent = false;
            self.snapshot_dropped = false;
            return RenderStats {
                render_thread: self.host.threaded(),
                ..Default::default()
            };
        }
        if !self.failed && !self.render_thread_free() {
            // the GPU still has the frame before: this one is not built,
            // the caller goes on (input, network, the stall notice)
            if let Some(ui) = ui.as_mut() {
                self.held_textures.0.append(std::mem::take(&mut ui.textures_delta));
            }
            return RenderStats {
                render_thread: true,
                wait_render_ms: t0.elapsed().as_secs_f32() * 1000.0,
                ..Default::default()
            };
        }
        // the back-pressure: the frame before still in the render thread's
        // hands (the hand-over below finds it free)
        let free_wait = t0.elapsed();
        if let Some(ui) = ui.as_mut()
            && !self.held_textures.0.is_empty()
        {
            let mut held = std::mem::take(&mut self.held_textures.0);
            held.append(std::mem::take(&mut ui.textures_delta));
            ui.textures_delta = held;
        }
        // ---- the scene's mirrors join the journal of this frame
        self.records.flush(&self.device, &mut self.writes);
        self.cull.objects.flush(&self.device, &mut self.writes);
        self.cull.boxes.flush(&self.device, &mut self.writes);
        self.textures.maintain(&self.device, &mut self.writes, &mut self.uploads, false);
        let uploads = self.uploads.take_frame();
        let mut stats = RenderStats {
            records_uploaded: self.records.uploaded,
            palettes_uploaded: std::mem::take(&mut self.palettes_uploaded),
            cull_uploaded: self.records.faces.uploaded + self.cull.objects.uploaded + self.cull.boxes.uploaded,
            vertices_uploaded: std::mem::take(&mut self.vertices_uploaded),
            journal_bytes: self.writes.bytes() as u64,
            textures: self.textures.live(),
            texture_bytes: self.textures.bytes(),
            texture_pages: self.textures.pages(),
            texture_page_bytes: self.textures.page_bytes(),
            geometry_bytes: self.geometry.bytes(),
            records: self.records.live(),
            render_thread: self.host.threaded(),
            ..Default::default()
        };
        (stats.vertex_used, stats.index_used) = self.geometry.used();
        if self.failed {
            // nobody replays any more
            self.writes.clear();
            self.fail_picks();
            return stats;
        }
        // ---- the packet: this frame's journal leaves, an empty one (with
        // the memory of an earlier frame) takes its place
        let mut spent = self.spare.pop().unwrap_or_default();
        std::mem::swap(&mut self.writes, &mut spent.writes);
        spent.lists.copy_from(lists);
        // a photo first: the other captures wait for the next frame (and
        // for the answer of a photo still on the GPU)
        let capture = match self.snapshot_request {
            Some(_) if self.snapshot_sent => None,
            Some(s) => Some(crate::packet::Capture::Snapshot(s)),
            None => capture_for(self.capture_scene, self.capture_request, self.can_capture),
        };
        self.last_inv_vp = (params.proj * params.view).inverse();
        self.last_view = params.view;
        let index = self.frames;
        let packet = FramePacket {
            output,
            index,
            writes: spent.writes,
            uploads,
            scene: self.scene_gpu(),
            params,
            lists: spent.lists,
            ui,
            surface: SurfaceState {
                width: self.size.0,
                height: self.size.1,
                vsync: self.vsync,
            },
            settings: self.settings,
            hover: self.hover_request.take(),
            picks: std::mem::take(&mut self.pick_requests),
            drawn: std::mem::take(&mut self.drawn_requests),
            capture,
            histogram: self.histogram_request.take(),
            motion_cut: std::mem::take(&mut self.motion_cut),
            input_at: self.input_at.take(),
        };
        self.frames += 1;
        let built = t0.elapsed();

        // ---- to the backend
        let mut results: [Option<Box<FrameResult>>; 2] = [None, None];
        let mut waited = free_wait;
        let mut gone = false;
        match &mut self.host {
            Host::Inline(backend) => results[0] = Some(Box::new(backend.render(packet))),
            Host::Thread { owner, .. } => {
                match owner.submit(Job::Frame(Box::new(packet))) {
                    Ok(s) => {
                        // (added to the wait before the packet: replaced,
                        // the back-pressure counted as the packet's work)
                        waited += s.waited;
                        if let Some(Done::Frame(r)) = s.previous {
                            results[0] = Some(r);
                        }
                    }
                    Err(_) => gone = true,
                }
                if capture.is_some() && !gone {
                    // the capture is read right after this call; a GPU that
                    // stops answering in it leaves it for a later frame
                    let t = Instant::now();
                    match owner.finish_until(t + CAPTURE_WAIT) {
                        Ok(Some(Done::Frame(r))) => {
                            if r.index != index {
                                log::warn!("capture: frame {} answered for frame {index}", r.index);
                            }
                            results[1] = Some(r);
                        }
                        Ok(_) => {
                            if matches!(capture, Some(crate::packet::Capture::Snapshot(_))) {
                                self.snapshot_sent = true;
                            }
                        }
                        Err(_) => gone = true,
                    }
                    waited += t.elapsed();
                }
            }
        }
        for r in results.into_iter().flatten() {
            self.apply(*r);
        }
        if gone {
            self.fail();
        }

        // ---- statistics: the stores' side is this frame's, the backend's
        // side that of the last frame it finished (counted once)
        let main = stats;
        let fresh = std::mem::take(&mut self.fresh);
        if fresh {
            stats = self.last_stats;
        } else {
            // no result since the last call (first frame, frame after a
            // capture): the last readbacks stay, the timings are not
            // counted twice
            stats = RenderStats {
                gpu_ms: self.last_stats.gpu_ms,
                gpu_pixels: self.last_stats.gpu_pixels,
                gpu_elements: self.last_stats.gpu_elements,
                shadow: ShadowStats {
                    gpu_ms: self.last_stats.shadow.gpu_ms,
                    ..Default::default()
                },
                gpu_depth: self.last_stats.gpu_depth,
                gpu_ssao: self.last_stats.gpu_ssao,
                gpu_contact: self.last_stats.gpu_contact,
                gpu_bloom: self.last_stats.gpu_bloom,
                gpu_effects: self.last_stats.gpu_effects,
                occluded: self.last_stats.occluded,
                visible_objects: self.last_stats.visible_objects,
                gpu_cull: self.last_stats.gpu_cull,
                present_mode: self.last_stats.present_mode,
                water_reflection: self.last_stats.water_reflection,
                mirror: self.last_stats.mirror,
                ..Default::default()
            };
        }
        stats.records_uploaded = main.records_uploaded;
        stats.palettes_uploaded = main.palettes_uploaded;
        stats.cull_uploaded = main.cull_uploaded;
        stats.vertices_uploaded = main.vertices_uploaded;
        stats.journal_bytes = main.journal_bytes;
        stats.textures = main.textures;
        stats.texture_bytes = main.texture_bytes;
        stats.texture_pages = main.texture_pages;
        stats.texture_page_bytes = main.texture_page_bytes;
        stats.geometry_bytes = main.geometry_bytes;
        stats.vertex_used = main.vertex_used;
        stats.index_used = main.index_used;
        stats.records = main.records;
        stats.render_thread = main.render_thread;
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        stats.wait_render_ms = ms(waited);
        if stats.render_thread {
            // what the main thread spent: the packet and the hand-over
            stats.packet_ms = ms(t0.elapsed().saturating_sub(waited));
            stats.cpu_encode_ms = stats.packet_ms;
        } else {
            stats.packet_ms = ms(built);
            stats.cpu_encode_ms += stats.packet_ms;
        }
        stats.pace_ms = ms(self.gpu_waited);
        // the pacing's gate (pacing.rs): this frame's main-thread time, from
        // its input to the hand-over, without the back-pressure; the render
        // thread's and the GPU's of the last frame finished, the swapchain
        // wait left out (the GPU's pace, not work)
        if fresh && let Some(began) = self.frame_began.take() {
            let main = ms(began.elapsed().saturating_sub(waited));
            let render = stats.thread_ms - stats.cpu_phases[0];
            let on = self.pace_gate.update(main, render, stats.gpu_ms);
            self.gpu_frames.set_active(on);
        }
        stats
    }

    /// The frame being built begins reading its input now (the start of
    /// the viewer's frame): its time to the screen is measured from here.
    pub fn mark_input(&mut self, at: Instant) {
        self.input_at = Some(at);
        self.frame_began = Some(at);
    }

    /// Once a frame is handed over, before the next one reads its input:
    /// wait until the GPU has finished the frame before it (pacing.rs), at
    /// most [`GPU_WAIT`]. The time it took (nothing when the GPU is not the
    /// limit, or with AURORA_FRAME_PACING=0).
    pub fn wait_for_gpu(&mut self) -> Duration {
        self.gpu_waited = Duration::ZERO;
        if !self.gpu_frames.active() || self.failed || self.stalled || self.lost.get() {
            return Duration::ZERO;
        }
        // the frame handed over last, and the one before it
        let Some(before) = self.frames.checked_sub(2) else {
            return Duration::ZERO;
        };
        let t = Instant::now();
        match &self.host {
            // the render thread waits on the device and sends the news; a
            // timeout is a GPU that fell behind: the frames go on, the stall
            // notice is the render thread's (`stalled_for`)
            Host::Thread { .. } => {
                self.gpu_frames.wait_finished(before, t + GPU_WAIT);
            }
            // alone on the device: the wait is this thread's
            Host::Inline(backend) => backend.finish_frames_before(before + 1, GPU_WAIT),
        }
        self.gpu_waited = t.elapsed();
        self.gpu_waited
    }

    /// Ask for the world position of the opaque surface under a pixel of
    /// the last frame built (depth prepass read-back; None for the sky),
    /// without waiting for it: the request leaves with the next frame, the
    /// render thread copies the texel before it draws that frame, and
    /// [`Self::take_pick`] gives the answer one to three frames later. The
    /// ticket returned names the request.
    pub fn request_pick(&mut self, x: f32, y: f32) -> u64 {
        let ticket = self.pick_ticket;
        self.pick_ticket += 1;
        let (w, h) = self.size;
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 || self.failed {
            self.pick_answers.push((ticket, None));
        } else {
            self.pick_requests.push((ticket, x as u32, y as u32));
        }
        ticket
    }

    /// The answer to a [`Self::request_pick`], once: None while it has not
    /// come back. When the render thread has already finished the frame
    /// handed over, its result is taken in now instead of with the next
    /// hand-over: with a frame cap or vertical sync the render thread is
    /// done long before the next frame starts, and the click is answered
    /// one frame after it was made instead of two.
    pub fn take_pick(&mut self, ticket: u64) -> Option<Option<Vec3>> {
        if !self.pick_answers.iter().any(|(t, _)| *t == ticket) {
            self.collect();
        }
        let i = self.pick_answers.iter().position(|(t, _)| *t == ticket)?;
        Some(self.pick_answers.swap_remove(i).1)
    }

    /// Take in the result of the frame handed over if the render thread is
    /// done with it, without waiting for it.
    fn collect(&mut self) {
        let Host::Thread { owner, .. } = &mut self.host else {
            return;
        };
        match owner.take_ready() {
            Ok(Some(Done::Frame(r))) => self.apply(*r),
            Ok(_) => {}
            Err(_) => self.fail(),
        }
    }

    /// Give up a [`Self::request_pick`] nobody waits for any more.
    pub fn forget_pick(&mut self, ticket: u64) {
        self.pick_requests.retain(|(t, _, _)| *t != ticket);
        self.pick_answers.retain(|(t, _)| *t != ticket);
    }

    /// Ask what the next frame draws under a pixel: the draw record and
    /// the surface point of the nearest opaque or masked face and of the
    /// nearest blended face in front of it, posed and placed as shown
    /// (a rigged mesh is where the GPU skins it). Nothing waits: the frame
    /// draws its faces once more through a camera narrowed to that pixel
    /// (drawn_pick.rs), and [`Self::take_drawn`] gives the answer a few
    /// frames later. Costs that pass, in that frame only.
    pub fn request_drawn(&mut self, x: f32, y: f32, mode: crate::DrawnMode) -> u64 {
        let ticket = self.pick_ticket;
        self.pick_ticket += 1;
        let (w, h) = self.size;
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 || self.failed {
            self.drawn_answers.push((ticket, Default::default()));
        } else {
            self.drawn_requests.push((ticket, x as u32, y as u32, mode));
        }
        ticket
    }

    /// The answer to a [`Self::request_drawn`], once: None while it has
    /// not come back.
    pub fn take_drawn(&mut self, ticket: u64) -> Option<crate::Drawn> {
        if !self.drawn_answers.iter().any(|(t, _)| *t == ticket) {
            self.collect();
        }
        let i = self.drawn_answers.iter().position(|(t, _)| *t == ticket)?;
        Some(self.drawn_answers.swap_remove(i).1)
    }

    /// Give up a [`Self::request_drawn`] nobody waits for any more.
    pub fn forget_drawn(&mut self, ticket: u64) {
        self.drawn_requests.retain(|(t, _, _, _)| *t != ticket);
        self.drawn_answers.retain(|(t, _)| *t != ticket);
    }

    /// Like [`Self::request_pick`], waiting for the answer: for the frame in
    /// flight on the render thread, then for everything the GPU has in its
    /// queue (16 to 20 ms in a region that keeps it busy). What a click did
    /// before; kept to compare the two (AURORA_CLICK_PICK).
    pub fn pick_world(&mut self, x: f32, y: f32) -> Option<Vec3> {
        let (w, h) = self.size;
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 || self.failed {
            return None;
        }
        let mut frame = None;
        let mut gone = false;
        let hit = match &mut self.host {
            Host::Inline(backend) => backend.pick_world(x, y),
            Host::Thread { owner, .. } => {
                match owner.submit(Job::Pick { x, y }) {
                    Ok(s) => {
                        if let Some(Done::Frame(r)) = s.previous {
                            frame = Some(r);
                        }
                    }
                    Err(_) => gone = true,
                }
                match owner.finish() {
                    Ok(Some(Done::Pick(hit))) => hit,
                    Ok(_) => None,
                    Err(_) => {
                        gone = true;
                        None
                    }
                }
            }
        };
        if let Some(r) = frame {
            self.apply(*r);
        }
        if gone {
            self.fail();
        }
        hit
    }

    /// Like `pick_world` for the hover cursor, without waiting for the GPU:
    /// asks for this pixel in the next frame and returns the last answer
    /// (a few frames old).
    pub fn hover_pick(&mut self, x: f32, y: f32) -> Option<Vec3> {
        let (w, h) = self.size;
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
            return None;
        }
        self.hover_request = Some((x as u32, y as u32));
        self.hover_latest
    }

    /// World ray under a pixel of the last frame: origin on the near plane,
    /// unit direction.
    pub fn cursor_ray(&self, x: f32, y: f32) -> Option<(Vec3, Vec3)> {
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let ndc = |z: f32| Vec4::new(x / w * 2.0 - 1.0, 1.0 - y / h * 2.0, z, 1.0);
        let un = |v: Vec4| {
            let p = self.last_inv_vp * v;
            (p.w.abs() > 1e-9).then(|| p.truncate() / p.w).filter(|v| v.is_finite())
        };
        // reverse-Z: 1 is the near plane, small values far away
        let near = un(ndc(1.0))?;
        let far = un(ndc(0.001))?;
        let dir = (far - near).try_normalize()?;
        Some((near, dir))
    }

    /// Camera/axes of the submitted frame, matching particle_vs billboards.
    /// Invert only for a particle click, not on every rendered frame.
    pub fn particle_view(&self) -> (Vec3, Vec3, Vec3) {
        let inv = self.last_view.inverse();
        (
            inv.w_axis.truncate(),
            inv.transform_vector3(Vec3::X).normalize_or(Vec3::X),
            inv.transform_vector3(Vec3::Y).normalize_or(Vec3::Z),
        )
    }

    /// Project a world position to screen pixels (None if behind camera).
    pub fn project(&self, f: &FrameParams, p: Vec3) -> Option<(f32, f32, f32)> {
        let c = f.proj * f.view * p.extend(1.0);
        if c.w <= 0.01 {
            return None;
        }
        let ndc = c.truncate() / c.w;
        let x = (ndc.x * 0.5 + 0.5) * self.size.0 as f32;
        let y = (0.5 - ndc.y * 0.5) * self.size.1 as f32;
        Some((x, y, c.w))
    }

    pub fn clear_color_default() -> Vec4 {
        Vec4::new(0.03, 0.04, 0.12, 1.0)
    }
}

/// What `Renderer::new` needs of an opened adapter.
struct OpenedDevice {
    device: wgpu::Device,
    queue: wgpu::Queue,
    alim: wgpu::Limits,
    bindless: bool,
    timestamps: bool,
    adapter_specific: bool,
    max_textures: u32,
}

/// Open `adapter` with the features the renderer needs and the optional
/// ones it has; an error when it lacks a needed one or the device fails.
fn open_device(
    adapter: &wgpu::Adapter,
    #[cfg(any(target_os = "linux", target_os = "windows"))] xr: Option<&crate::xr_device::Graphics>,
) -> Result<OpenedDevice, RenderError> {
    let afeat = adapter.features();
    let alim = adapter.limits();
    let bindless_feats =
        wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
    let bindless = afeat.contains(bindless_feats) && alim.max_binding_array_elements_per_shader_stage >= 64;
    if !bindless {
        return Err(RenderError::Device(
            "the GPU driver lacks descriptor indexing (texture binding arrays)".into(),
        ));
    }
    let mut features = bindless_feats;
    if afeat.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE) {
        features |= wgpu::Features::INDIRECT_FIRST_INSTANCE;
    } else {
        return Err(RenderError::Device("INDIRECT_FIRST_INSTANCE unsupported".into()));
    }
    // The fragment stage of the world passes reads five storage buffers
    // (the frame group's 9, 16 and 24, the night sky's stars in stars.rs;
    // the records; the texture pages), one more than wgpu's downlevel
    // minimum. Drivers with descriptor indexing give far more: say so here
    // rather than fail when the pipelines are made.
    const FRAGMENT_STORAGE_BUFFERS: u32 = 5;
    if alim.max_storage_buffers_per_shader_stage < FRAGMENT_STORAGE_BUFFERS {
        return Err(RenderError::Device(format!(
            "the GPU driver allows {} storage buffers per shader stage, {FRAGMENT_STORAGE_BUFFERS} are needed",
            alim.max_storage_buffers_per_shader_stage
        )));
    }
    let timestamps = afeat.contains(wgpu::Features::TIMESTAMP_QUERY);
    if timestamps {
        features |= wgpu::Features::TIMESTAMP_QUERY;
        // GPU time by kind of element (performance panel)
        for f in [
            wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
            wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
        ] {
            if afeat.contains(f) {
                features |= f;
            }
        }
    }
    // needed for MSAA 8x on HDR targets (adapter-specific sample counts)
    let adapter_specific = afeat.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    if adapter_specific {
        features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    }
    // debug wireframe
    if afeat.contains(wgpu::Features::POLYGON_MODE_LINE) {
        features |= wgpu::Features::POLYGON_MODE_LINE;
    }
    // Keep the actual water triangles across the near plane. The portable
    // shader path restores their real depth when hardware clamp is absent.
    let portable_water =
        std::env::var_os("AURORA_DEMO").is_some() && std::env::var("AURORA_DEMO_WATER").is_ok_and(|mode| mode.contains("portable-depth"));
    if afeat.contains(wgpu::Features::DEPTH_CLIP_CONTROL) && !portable_water {
        features |= wgpu::Features::DEPTH_CLIP_CONTROL;
    }
    if afeat.contains(wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY) {
        features |= wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY;
    }
    // BC7 texture pages (textures.rs): a quarter of the video memory of
    // RGBA8 for the streamed textures; without it they stay RGBA8
    if afeat.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
        features |= wgpu::Features::TEXTURE_COMPRESSION_BC;
    }
    // GPU draw lists (gpu_cull.rs): draw counts written by the GPU
    if afeat.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT) {
        features |= wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;
    }
    let max_textures = texture_page_limit(&alim);
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("aurora"),
        required_features: features,
        required_limits: alim.clone(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    };
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let (device, queue) = if let Some(xr) = xr {
        let graphics = xr.open(adapter, &descriptor).map_err(|e| RenderError::Device(e.to_string()))?;
        log::info!("OpenXR-compatible viewer graphics device: {}", adapter.get_info().name);
        graphics
    } else {
        pollster::block_on(adapter.request_device(&descriptor)).map_err(|e| RenderError::Device(e.to_string()))?
    };
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).map_err(|e| RenderError::Device(e.to_string()))?;
    Ok(OpenedDevice {
        device,
        queue,
        alim,
        bindless,
        timestamps,
        adapter_specific,
        max_textures,
    })
}

/// Size of the largest device-local memory heap (the GPU's own VRAM on a
/// discrete card), read from Vulkan.
#[cfg(not(target_os = "macos"))]
fn detect_vram_mb(adapter: &wgpu::Adapter) -> Option<u64> {
    use ash::vk;
    // SAFETY: the hal adapter is only borrowed for this call; the physical
    // device query does not change any state.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let instance = hal.shared_instance().raw_instance();
    let props = unsafe { instance.get_physical_device_memory_properties(hal.raw_physical_device()) };
    props
        .memory_heaps_as_slice()
        .iter()
        .filter(|h| h.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
        .map(|h| h.size / (1024 * 1024))
        .max()
}

/// Memory the GPU can use well, read from Metal: the recommended working
/// set of the device (a share of the unified memory on Apple Silicon), the
/// figure the Legacy Viewer reads as `kCGLRPVideoMemoryMegabytes`
/// (indra/llwindow/llopenglview-objc.mm).
#[cfg(target_os = "macos")]
fn detect_vram_mb(adapter: &wgpu::Adapter) -> Option<u64> {
    use objc2_metal::MTLDevice;
    // SAFETY: the hal adapter is only borrowed for this call; reading a
    // device property changes no state.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }?;
    let bytes = hal.raw_device().recommendedMaxWorkingSetSize();
    (bytes > 0).then_some(bytes / (1024 * 1024))
}

// Binding-array entries have their own limit; they do not consume the
// ordinary sampled-texture slots used by frame attachments.
fn texture_page_limit(limits: &wgpu::Limits) -> u32 {
    limits.max_binding_array_elements_per_shader_stage.clamp(64, 16384)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindless_pages_are_not_limited_by_ordinary_texture_slots() {
        let mut limits = wgpu::Limits {
            max_sampled_textures_per_shader_stage: 96,
            max_binding_array_elements_per_shader_stage: 1_000_000,
            ..Default::default()
        };
        assert_eq!(texture_page_limit(&limits), 16384);
        limits.max_binding_array_elements_per_shader_stage = 512;
        assert_eq!(texture_page_limit(&limits), 512);
    }

    #[test]
    fn skin_binds_send_only_the_range_asked_for() {
        // a new mesh at the end of the list, or in the place of one gone
        assert_eq!(skin_binds_range(134..244, 244), 134..244);
        assert_eq!(skin_binds_range(20..54, 244), 20..54);
        // never past the list
        assert_eq!(skin_binds_range(200..300, 244), 200..244);
        assert!(skin_binds_range(300..320, 244).is_empty());
    }
}
