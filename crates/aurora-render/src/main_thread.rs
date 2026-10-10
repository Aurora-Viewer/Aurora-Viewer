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
//! - **Render thread, [`Backend`].** How to draw: the surface, pipelines,
//!   render targets, per-frame uniforms, egui, the compute culling and its
//!   bins, occlusion, readbacks. It is the only user of the queue.
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
use crate::gpu_cull::{CullTables, GpuCull};
use crate::packet::{EguiFrame, FramePacket, FrameResult, SceneGpu, Spent, SurfaceState, capture_for};
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
    pub max_textures: u32,
    /// Dedicated video memory (MB) when it can be detected.
    pub vram_mb: Option<u64>,
    /// Integrated GPU (shares the system memory).
    pub integrated: bool,
}

/// First skin binding to send when `sent` of `len` are already on the GPU:
/// the new tail, or everything if the list was started over.
fn skin_binds_tail(sent: usize, len: usize) -> usize {
    if sent <= len { sent } else { 0 }
}

/// Packet buffers kept for the next frames (one in flight, one being built).
const SPARE_PACKETS: usize = 2;

/// Handles of the scene's buffers (reference counted: no GPU work).
fn scene_gpu(
    records: &RecordStore,
    cull: &CullTables,
    textures: &TextureTable,
    geometry: &GeometryArena,
    palettes: &wgpu::Buffer,
    skin_binds: &wgpu::Buffer,
    bind_generation: u64,
) -> SceneGpu {
    SceneGpu {
        records: records.buffer.clone(),
        palettes: palettes.clone(),
        skin_binds: skin_binds.clone(),
        records_key: (records.generation, bind_generation),
        faces: records.faces.buffer().clone(),
        face_count: records.faces.len(),
        objects: cull.objects.buffer().clone(),
        object_count: cull.objects.len(),
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
    pub device: wgpu::Device,
    pub info: GpuInfo,
    pub geometry: GeometryArena,
    /// Copies of the streamed data staged by background jobs (upload.rs),
    /// submitted first in the frame's submit.
    pub uploads: UploadQueue,
    pub records: RecordStore,
    pub textures: TextureTable,
    /// The scene's tables of the GPU-driven draw lists (gpu_cull.rs).
    pub cull: CullTables,
    /// Queue writes of the frame being built (writes.rs).
    writes: GpuWrites,
    palette_buffer: wgpu::Buffer,
    /// Palette bytes written since the last frame (AURORA_PROFILE).
    palettes_uploaded: u64,
    skin_bind_buffer: wgpu::Buffer,
    /// Skin bindings already in `skin_bind_buffer` (see `set_skin_binds`).
    skin_binds_sent: usize,
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
    /// Texture animation clock (tex_anim.rs), shared with the backend.
    anim_clock: crate::tex_anim::AnimClock,
    /// Inverse view-projection of the last frame built (cursor rays).
    last_inv_vp: Mat4,
    /// Pixel to read the depth of in the next frame, and the last answer.
    hover_request: Option<(u32, u32)>,
    hover_latest: Option<Vec3>,
    /// Frames handed over.
    frames: u64,
    /// The renderer's side of the last frame result, and whether it came
    /// since the last `render` returned.
    last_stats: RenderStats,
    fresh: bool,
    spare: Vec<Spent>,
    failed: bool,
    /// Set to request a copy of the next presented frame.
    pub capture_request: bool,
    /// Capture the 3D view without the interface (next frame) into `captured`.
    pub capture_scene: bool,
    /// Last captured frame (width, height, RGBA8).
    pub captured: Option<(u32, u32, Vec<u8>)>,
}

impl Renderer {
    /// Create the renderer on the Vulkan backend for the given window.
    pub fn new(target: impl Into<wgpu::SurfaceTarget<'static>>, width: u32, height: u32, vsync: bool) -> Result<Renderer, RenderError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN;
        desc.flags = if std::env::var_os("AURORA_GPU_VALIDATION").is_some() {
            wgpu::InstanceFlags::debugging()
        } else {
            wgpu::InstanceFlags::empty()
        };
        let instance = wgpu::Instance::new(desc);
        let surface = instance.create_surface(target).map_err(|e| RenderError::Surface(e.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let ainfo = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, driver {} {})",
            ainfo.name,
            ainfo.backend,
            ainfo.driver,
            ainfo.driver_info
        );
        let vram_mb = detect_vram_mb(&adapter);
        log::info!(
            "GPU memory: {}",
            vram_mb.map(|m| format!("{m} MB")).unwrap_or_else(|| "unknown".into())
        );

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
        if afeat.contains(wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY) {
            features |= wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY;
        }
        // GPU draw lists (gpu_cull.rs): draw counts written by the GPU
        if afeat.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT) {
            features |= wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;
        }
        let max_textures = alim
            .max_binding_array_elements_per_shader_stage
            .min(alim.max_sampled_textures_per_shader_stage.saturating_sub(16))
            .clamp(64, 16384);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("aurora"),
            required_features: features,
            required_limits: alim.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| {
            log::error!("wgpu error: {e}");
        }));
        let info = GpuInfo {
            name: ainfo.name.clone(),
            backend: format!("{:?}", ainfo.backend),
            driver: format!("{} {}", ainfo.driver, ainfo.driver_info),
            bindless,
            timestamps,
            max_textures,
            vram_mb,
            integrated: ainfo.device_type == wgpu::DeviceType::IntegratedGpu,
        };

        // ---- the scene's stores; what they write at creation is replayed
        // with the first frame
        let mut writes = GpuWrites::default();
        let textures = TextureTable::new(&device, &mut writes, max_textures, 8);
        let geometry = GeometryArena::new(&device, alim.max_buffer_size.min(1 << 31));
        let records = RecordStore::new(&device);
        let cull = CullTables::new(&device);
        let palette_buffer = Self::make_palette_buffer(&device, 64 * 160);
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
        let backend = Backend::new(BackendInit {
            device: device.clone(),
            queue,
            surface,
            adapter,
            width,
            height,
            vsync,
            timestamps,
            adapter_specific,
            gpu_cull,
            tex_layout: textures.layout.clone(),
            scene: scene_gpu(&records, &cull, &textures, &geometry, &palette_buffer, &skin_bind_buffer, 0),
            anim_clock,
        })?;
        let msaa_supported = backend.msaa_supported().to_vec();
        let can_capture = backend.can_capture();
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
            device,
            info,
            geometry,
            uploads,
            records,
            textures,
            cull,
            writes,
            palette_buffer,
            palettes_uploaded: 0,
            skin_bind_buffer,
            skin_binds_sent: 0,
            bind_generation: 0,
            gpu_cull,
            size: (width.max(1), height.max(1)),
            vsync,
            settings: RenderSettings::default(),
            msaa_supported,
            can_capture,
            anim_clock,
            last_inv_vp: Mat4::IDENTITY,
            hover_request: None,
            hover_latest: None,
            frames: 0,
            last_stats: RenderStats::default(),
            fresh: false,
            spare: Vec::new(),
            failed: false,
            capture_request: false,
            capture_scene: false,
            captured: None,
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
            &self.skin_bind_buffer,
            self.bind_generation,
        )
    }

    fn make_skin_bind_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skin binds"),
            size: (count.max(1) * std::mem::size_of::<SkinBind>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload the skin bindings (inverse bind matrix + palette joint per
    /// mesh joint) referenced by `DrawRecord::flags[2]` of skinned records.
    /// The list only grows (a rigged mesh appends its joints): only the
    /// entries not sent yet are written, unless the buffer had to grow. The
    /// whole list sent again for every new rigged mesh was megabytes a frame
    /// while a crowd loads.
    pub fn set_skin_binds(&mut self, binds: &[SkinBind]) {
        if binds.is_empty() {
            return;
        }
        let size = std::mem::size_of::<SkinBind>();
        let mut from = skin_binds_tail(self.skin_binds_sent, binds.len());
        if std::mem::size_of_val(binds) as u64 > self.skin_bind_buffer.size() {
            let mut n = (self.skin_bind_buffer.size() as usize / size).max(1);
            while n < binds.len() {
                n *= 2;
            }
            self.skin_bind_buffer = Self::make_skin_bind_buffer(&self.device, n);
            self.bind_generation += 1;
            from = 0;
        }
        if from < binds.len() {
            self.writes
                .write_buffer(&self.skin_bind_buffer, (from * size) as u64, bytemuck::cast_slice(&binds[from..]));
        }
        self.skin_binds_sent = binds.len();
    }

    fn make_palette_buffer(device: &wgpu::Device, matrices: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("joint palettes"),
            size: (matrices.max(1) * 64) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
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
            self.palette_buffer = Self::make_palette_buffer(&self.device, n);
            self.bind_generation += 1;
        }
        self.writes.write_buffer(&self.palette_buffer, 0, bytemuck::cast_slice(mats));
        self.palettes_uploaded += bytes;
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

    pub fn size(&self) -> (u32, u32) {
        self.size
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

    // ------------------------------------------------------------ resources

    pub fn upload_mesh(&mut self, vertices: &[Vertex], indices: &[u16]) -> Option<MeshAlloc> {
        self.geometry
            .alloc(&self.device, &mut self.writes, &mut self.uploads, vertices, None, indices)
    }

    pub fn upload_skinned_mesh(&mut self, vertices: &[Vertex], skin: &[SkinVertex], indices: &[u16]) -> Option<MeshAlloc> {
        self.geometry
            .alloc(&self.device, &mut self.writes, &mut self.uploads, vertices, Some(skin), indices)
    }

    /// Upload a mesh staged by a background job (`mesh.data.ready()` must
    /// hold): the main thread only records the copies.
    pub fn upload_mesh_staged(&mut self, mesh: &StagedMesh) -> Option<MeshAlloc> {
        self.geometry.alloc_staged(&self.device, &mut self.writes, &mut self.uploads, mesh)
    }

    pub fn free_mesh(&mut self, m: MeshAlloc) {
        self.geometry.free(m);
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
    fn apply(&mut self, r: FrameResult) {
        self.hover_latest = r.hover;
        self.cull.last = r.cull;
        if let Some(c) = r.captured {
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
    }

    /// Close the frame and have it drawn: with a render thread the packet
    /// is handed over (after the frame before it is presented) and this
    /// returns at once, with the statistics of that frame before; without,
    /// the frame is drawn here. A frame that carries a capture is waited
    /// for: `captured` holds it when this returns.
    pub fn render(&mut self, params: FrameParams, lists: &DrawLists, ui: Option<EguiFrame>) -> RenderStats {
        let t0 = Instant::now();
        // ---- the scene's mirrors join the journal of this frame
        self.records.flush(&self.device, &mut self.writes);
        self.cull.objects.flush(&self.device, &mut self.writes);
        self.textures.maintain(&self.device, &mut self.writes, &mut self.uploads, false);
        let uploads = self.uploads.take_frame();
        let mut stats = RenderStats {
            records_uploaded: self.records.uploaded,
            palettes_uploaded: std::mem::take(&mut self.palettes_uploaded),
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
            return stats;
        }
        // ---- the packet: this frame's journal leaves, an empty one (with
        // the memory of an earlier frame) takes its place
        let mut spent = self.spare.pop().unwrap_or_default();
        std::mem::swap(&mut self.writes, &mut spent.writes);
        spent.lists.copy_from(lists);
        let capture = capture_for(self.capture_scene, self.capture_request, self.can_capture);
        self.last_inv_vp = (params.proj * params.view).inverse();
        let index = self.frames;
        let packet = FramePacket {
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
            capture,
        };
        self.frames += 1;
        let built = t0.elapsed();

        // ---- to the backend
        let mut results: [Option<Box<FrameResult>>; 2] = [None, None];
        let mut waited = Duration::ZERO;
        let mut gone = false;
        match &mut self.host {
            Host::Inline(backend) => results[0] = Some(Box::new(backend.render(packet))),
            Host::Thread { owner, .. } => {
                match owner.submit(Job::Frame(Box::new(packet))) {
                    Ok(s) => {
                        waited = s.waited;
                        if let Some(Done::Frame(r)) = s.previous {
                            results[0] = Some(r);
                        }
                    }
                    Err(_) => gone = true,
                }
                if capture.is_some() && !gone {
                    // the capture is read right after this call
                    let t = Instant::now();
                    match owner.finish() {
                        Ok(Some(Done::Frame(r))) => {
                            if r.index != index {
                                log::warn!("capture: frame {} answered for frame {index}", r.index);
                            }
                            results[1] = Some(r);
                        }
                        Ok(_) => {}
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
        if std::mem::take(&mut self.fresh) {
            stats = self.last_stats;
        } else {
            // no result since the last call (first frame, frame after a
            // capture): the last readbacks stay, the timings are not
            // counted twice
            stats = RenderStats {
                gpu_ms: self.last_stats.gpu_ms,
                gpu_elements: self.last_stats.gpu_elements,
                occluded: self.last_stats.occluded,
                visible_objects: self.last_stats.visible_objects,
                gpu_cull: self.last_stats.gpu_cull,
                water_reflection: self.last_stats.water_reflection,
                mirror: self.last_stats.mirror,
                ..Default::default()
            };
        }
        stats.records_uploaded = main.records_uploaded;
        stats.palettes_uploaded = main.palettes_uploaded;
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
        stats
    }

    /// World position of the opaque surface under a pixel of the last frame
    /// (depth prepass read-back; None for the sky). Blocks on the GPU for one
    /// tiny copy, and for the frame in flight on the render thread: call it
    /// on clicks only.
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

/// Size of the largest device-local memory heap (the GPU's own VRAM on a
/// discrete card), read from Vulkan. None on other backends.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_binds_send_only_the_new_tail() {
        assert_eq!(skin_binds_tail(0, 134), 0);
        assert_eq!(skin_binds_tail(134, 244), 134);
        // nothing new: an empty tail
        assert_eq!(skin_binds_tail(244, 244), 244);
        // a shorter list is a new one
        assert_eq!(skin_binds_tail(244, 134), 0);
    }
}
