//! DLSS frame generation's images (prototype, Linux's sl-ngx only; dlss.rs
//! `Dlss::generate`): the setting `RenderSettings::dlss_fg`, which
//! `AURORA_DLSS_FG=1` or `0` overrides for a run.
//!
//! Each frame DLSS draws, the window's image is copied twice: before the
//! interface (`hudless`) and as shown (`back`). NGX makes from the first,
//! with the frame's depth and motion, the frame halfway between the last
//! frame and this one, without the interface (`out`); the shown frame
//! (`mixed`) is `out` with the real frame's pixels wherever the two copies
//! differ, which is where the interface drew (shaders/dlss_fg_mix.wgsl).
//! NGX can take the interface from that difference itself, but smeared the
//! dock and chat bar at some window sizes (1226x2055, not 1226x2050 or
//! 1836x995). One format for the images NGX reads and writes, as storage: the
//! window's (BGRA) is not storage without a device feature that changes
//! every shader (spirv_fix.rs), so they are RGBA, drawn by a blit.
//!
//! The frame is drawn aside (`real`, the window's format) and presented
//! at its end: the generated frame first, in the window image acquired at
//! the frame's start, then the real one, in a second (renderer.rs). Under
//! Fifo, which the renderer sets while frame generation runs, the two
//! presents pace the real frames at half the refresh rate; the viewer's
//! frame cap stands aside then ([`running`]).
//!
//! With sl-ngx's presenter (dlss.rs `Presenter`, Linux), the window's
//! surface is the presenter's instead: the frames are drawn into a ring of
//! [`RING`] slots, each a real frame and the generated one as shown (the
//! window's format), handed over at the frame's end and kept until the
//! presenter has read them. It presents both, half a frame apart, on a
//! queue of its own, with Reflex on its swapchain.
//!
//! `AURORA_DLSS_FG_DUMP=<folder>` writes the three images of three frames
//! in a row as PPM, from the `AURORA_DLSS_FG_DUMP_AT`-th generation (300
//! by default): the test of what generation makes, before it is shown.

use crate::pipeline_cache::CachedPipelines;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Frame generation ran on the last frame drawn for the window.
static RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_running(on: bool) {
    RUNNING.store(on, Ordering::Relaxed);
}

/// DLSS frame generation runs in the window: the screen paces the frames
/// (Fifo), so the viewer's frame cap would only break the pairs.
pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// What shows frame generation's frames, for Preferences ([`status`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum FgShown {
    /// not running on the last frames drawn for the window
    #[default]
    Off,
    /// Linux: sl-ngx's presenter, on a swapchain of its own
    Presenter,
    /// Linux: the viewer's own two presents a frame (the prototype), and
    /// why sl-ngx's presenter does not show them
    TwoPresents(Option<FgRefusal>),
    /// Windows: Streamline, at the present; with the status it gave when
    /// it turned frame generation off at this window size
    Streamline(Option<String>),
}

/// Why sl-ngx's presenter does not show frame generation's frames
/// (Linux): the viewer presents them the prototype's way instead.
#[derive(Clone, Debug, PartialEq)]
pub enum FgRefusal {
    /// the sl-ngx loaded predates the presenter (updated, it runs from
    /// the next start)
    OldSlNgx,
    /// the device was opened without the presenter's queue (no second
    /// queue in the graphics family)
    NoQueue,
    /// the second camera holds the window's surface (tried again once
    /// it lets go)
    SecondCamera,
    /// a frame handed over was not released within 1 s
    Stalled,
    /// anything else, as the log says it (a Vulkan result, a call)
    Failed(String),
}

/// Frame generation as it runs in the window, over the last second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FgStatus {
    pub shown: FgShown,
    /// Reflex in low-latency mode on the swapchain that shows the frames
    pub reflex: bool,
    /// frames shown per frame drawn (2: one generated for each); 0 not
    /// known yet
    pub per_drawn: f32,
    /// frames drawn and frames shown per second; `shown_fps` 0 not known
    /// yet. Linux counts what was presented (or, with present timing,
    /// what the screen showed); Windows multiplies the drawn by what
    /// Streamline says it presents per frame drawn.
    pub drawn_fps: f32,
    pub shown_fps: f32,
}

static STATUS: std::sync::Mutex<Option<FgStatus>> = std::sync::Mutex::new(None);

/// Frame generation as it runs in the window (Preferences says it under
/// the switch); [`FgShown::Off`] while it does not.
pub fn status() -> FgStatus {
    STATUS.lock().ok().and_then(|s| s.clone()).unwrap_or_default()
}

pub(crate) fn set_status(status: Option<FgStatus>) {
    if let Ok(mut s) = STATUS.lock() {
        *s = status;
    }
}

/// Frame generation's frames counted over about a second, for [`status`].
pub(crate) struct Meter {
    since: std::time::Instant,
    /// frames drawn since `since`, frames generated among them
    drawn: u64,
    /// the shown and the real frames' running totals at `since`, where
    /// they are counted
    totals: Option<(u64, u64)>,
}

/// What a [`Meter`] measured over its second.
pub(crate) struct Measured {
    pub drawn_fps: f32,
    /// frames shown and real frames shown per second, from the totals
    /// given at both ends of the second
    pub shown_fps: f32,
    pub per_drawn: f32,
}

impl Meter {
    pub(crate) fn new(now: std::time::Instant, totals: Option<(u64, u64)>) -> Self {
        Self {
            since: now,
            drawn: 0,
            totals,
        }
    }

    /// One frame drawn with frame generation on. Once a second has
    /// passed, true: the caller reads its totals and calls [`Meter::take`].
    pub(crate) fn frame(&mut self, now: std::time::Instant) -> bool {
        self.drawn += 1;
        now.duration_since(self.since) >= std::time::Duration::from_secs(1)
    }

    /// The second's numbers, from the shown and real frames' running
    /// totals now (None where they are not counted: shown 0, per drawn
    /// 0), and the next second started.
    pub(crate) fn take(&mut self, now: std::time::Instant, totals: Option<(u64, u64)>) -> Measured {
        let secs = now.duration_since(self.since).as_secs_f32().max(1e-3);
        let (shown, real) = match (self.totals, totals) {
            (Some((s0, r0)), Some((s1, r1))) => (s1.saturating_sub(s0), r1.saturating_sub(r0)),
            _ => (0, 0),
        };
        let measured = Measured {
            drawn_fps: self.drawn as f32 / secs,
            shown_fps: shown as f32 / secs,
            per_drawn: if real == 0 { 0.0 } else { shown as f32 / real as f32 },
        };
        *self = Self::new(now, totals);
        measured
    }
}

pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// frames dumped in a row
const DUMP_FRAMES: u64 = 3;

/// the slots handed to sl-ngx's presenter in turn (its header asks at
/// least three: one drawn, one waiting, one being read)
pub(crate) const RING: usize = 3;

/// One frame's images: the frame as drawn (`real`), and with the
/// presenter the generated frame as shown (`shown`, the window's format),
/// both read by it until `released`.
struct Slot {
    real: wgpu::Texture,
    real_view: wgpu::TextureView,
    shown: Option<wgpu::Texture>,
    /// the presenter's `released` value these images wait for, 0 if none
    released: u64,
}

pub(crate) struct FrameGen {
    size: (u32, u32),
    format: wgpu::TextureFormat,
    /// the frames as drawn, presented after the generated ones: one, or
    /// with the presenter a ring of [`RING`] (`slot` this frame's)
    slots: Vec<Slot>,
    slot: usize,
    pub back: wgpu::TextureView,
    pub hudless: wgpu::TextureView,
    pub out: wgpu::TextureView,
    /// the generated frame as shown: `out` under the real frame's interface
    mixed: wgpu::TextureView,
    mix_pipeline: wgpu::RenderPipeline,
    mix_group: wgpu::BindGroup,
    blit: wgpu::util::TextureBlitter,
    /// into a window image (its format)
    present_blit: wgpu::util::TextureBlitter,
    /// the index of the last frame generation ran on
    last: Option<u64>,
    /// generations run, and those that made a frame
    pub runs: u64,
    pub generated: u64,
    /// presents of generated and of real frames
    pub presented_generated: u64,
    pub presented_real: u64,
    /// frames the presenter did not take (paused, or still busy)
    pub not_taken: u64,
    dump: Option<Dump>,
}

struct Dump {
    folder: PathBuf,
    at: u64,
    /// this frame's readbacks: (name, buffer, padded row)
    pending: Vec<(String, wgpu::Buffer, u32)>,
}

impl FrameGen {
    /// The images at `size`, the window's `format`; `ring`: for sl-ngx's
    /// presenter ([`RING`] slots, each with its generated frame as shown).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: (u32, u32), format: wgpu::TextureFormat, ring: bool) -> Self {
        let make = |label, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: usage | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let dump = std::env::var_os("AURORA_DLSS_FG_DUMP").map(|folder| Dump {
            folder: PathBuf::from(folder),
            at: std::env::var("AURORA_DLSS_FG_DUMP_AT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            pending: Vec::new(),
        });
        // NGX writes the output behind wgpu's back: wgpu would zero it at its
        // first read as never written (a black first frame), so it is
        // written once here
        let out = make(
            "DLSS FG output",
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            out.texture().as_image_copy(),
            &vec![0; size.0 as usize * size.1 as usize * 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * 4),
                rows_per_image: Some(size.1),
            },
            out.texture().size(),
        );
        let window_image = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let slots = (0..if ring { RING } else { 1 })
            .map(|_| {
                let real = window_image("DLSS FG real frame");
                Slot {
                    real_view: real.create_view(&wgpu::TextureViewDescriptor::default()),
                    real,
                    shown: ring.then(|| window_image("DLSS FG generated frame for the presenter")),
                    released: 0,
                }
            })
            .collect();
        let back = make("DLSS FG shown", wgpu::TextureUsages::RENDER_ATTACHMENT);
        let hudless = make("DLSS FG hudless", wgpu::TextureUsages::RENDER_ATTACHMENT);
        let mixed = make("DLSS FG generated as shown", wgpu::TextureUsages::RENDER_ATTACHMENT);
        let (mix_pipeline, mix_group) = mix(device, [&back, &hudless, &out]);
        log::info!(
            "DLSS frame generation: images made at {}x{}{}",
            size.0,
            size.1,
            if ring { ", a ring of 3 for sl-ngx's presenter" } else { "" }
        );
        Self {
            size,
            format,
            slots,
            slot: 0,
            present_blit: wgpu::util::TextureBlitter::new(device, format),
            back,
            hudless,
            out,
            mixed,
            mix_pipeline,
            mix_group,
            blit: wgpu::util::TextureBlitter::new(device, FORMAT),
            last: None,
            runs: 0,
            generated: 0,
            presented_generated: 0,
            presented_real: 0,
            not_taken: 0,
            dump,
        }
    }

    pub fn fits(&self, size: (u32, u32), format: wgpu::TextureFormat, ring: bool) -> bool {
        self.size == size && self.format == format && (self.slots.len() > 1) == ring
    }

    /// This frame's real frame, drawn into.
    pub fn real(&self) -> &wgpu::Texture {
        &self.slots[self.slot].real
    }

    /// This frame's generated frame as shown, for the presenter.
    pub fn shown(&self) -> Option<&wgpu::Texture> {
        self.slots[self.slot].shown.as_ref()
    }

    /// The size of the images.
    #[cfg_attr(
        not(all(feature = "dlss", target_os = "linux")),
        allow(dead_code, reason = "sl-ngx's presenter only")
    )]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The next slot becomes this frame's: the presenter's `released`
    /// value to wait for before its images are written, 0 if none.
    #[cfg_attr(
        not(all(feature = "dlss", target_os = "linux")),
        allow(dead_code, reason = "sl-ngx's presenter only")
    )]
    pub fn advance(&mut self) -> u64 {
        self.slot = (self.slot + 1) % self.slots.len();
        std::mem::take(&mut self.slots[self.slot].released)
    }

    /// This frame's images were taken by the presenter, until `released`
    /// reaches `value`.
    #[cfg_attr(
        not(all(feature = "dlss", target_os = "linux")),
        allow(dead_code, reason = "sl-ngx's presenter only")
    )]
    pub fn handed(&mut self, value: u64) {
        self.slots[self.slot].released = value;
    }

    /// The images handed to the presenter left in the layout it reads
    /// them in (transfer source; it leaves them so): the real frame, and
    /// the generated one as shown when there is one. Last in the frame.
    pub fn for_presenter(&self, encoder: &mut wgpu::CommandEncoder, generated: bool) {
        let slot = &self.slots[self.slot];
        let shown = slot.shown.as_ref().filter(|_| generated);
        encoder.transition_resources(
            std::iter::empty(),
            std::iter::once(&slot.real).chain(shown).map(|texture| wgpu::TextureTransition {
                texture,
                selector: None,
                state: wgpu::TextureUses::COPY_SRC,
            }),
        );
    }

    /// The generated frame (`generated`), under the real frame's
    /// interface, or the real one into a window image. After generation's
    /// commands.
    pub fn present_into(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, window: &wgpu::Texture, generated: bool) {
        let target = window.create_view(&wgpu::TextureViewDescriptor::default());
        let source = if generated {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DLSS FG interface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.mixed,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.mix_pipeline);
            pass.set_bind_group(0, &self.mix_group, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            &self.mixed
        } else {
            &self.slots[self.slot].real_view
        };
        self.present_blit.copy(device, encoder, source, &target);
    }

    /// The window's image before the interface.
    pub fn copy_hudless(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView) {
        self.blit.copy(device, encoder, scene, &self.hudless);
    }

    /// The window's image as shown.
    pub fn copy_back(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, shown: &wgpu::TextureView) {
        self.blit.copy(device, encoder, shown, &self.back);
    }

    /// Generation ran on the frame before `index`: its history goes on.
    /// Marks `index` as the last.
    pub fn chained(&mut self, index: u64) -> bool {
        let chained = self.last.is_some_and(|last| last + 1 == index);
        self.last = Some(index);
        chained
    }

    /// A frame generation did not run on: the next starts afresh.
    pub fn broken(&mut self) {
        self.last = None;
    }

    /// The readbacks of a dumped frame, submitted after `present_into`'s
    /// commands: `generated` is the frame as shown, `between` NGX's.
    pub fn encode_dump(&mut self, device: &wgpu::Device) -> Option<wgpu::CommandBuffer> {
        let n = self.runs;
        let dump = self.dump.as_mut().filter(|d| n >= d.at && n < d.at + DUMP_FRAMES)?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("DLSS FG dump"),
        });
        let (w, h) = self.size;
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let names = [
            ("shown", &self.back),
            ("hudless", &self.hudless),
            ("between", &self.out),
            ("generated", &self.mixed),
        ];
        for (name, view) in names {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("DLSS FG dump"),
                size: crate::readback::readback_size(u64::from(row) * u64::from(h)),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                view.texture().as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(h),
                    },
                },
                view.texture().size(),
            );
            dump.pending.push((format!("fg-{n:04}-{name}.ppm"), buffer, row));
        }
        Some(encoder.finish())
    }

    /// Writes a dumped frame's images, once submitted. Waits for the GPU:
    /// a test's frames only.
    pub fn write_dump(&mut self, device: &wgpu::Device) {
        let Some(dump) = self.dump.as_mut().filter(|d| !d.pending.is_empty()) else {
            return;
        };
        let (w, h) = self.size;
        for (_, buffer, _) in &dump.pending {
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }
        if let Err(e) = device.poll(wgpu::PollType::wait_indefinitely()) {
            log::error!("DLSS FG dump: {e}");
            dump.pending.clear();
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&dump.folder) {
            log::error!("DLSS FG dump: {}: {e}", dump.folder.display());
        }
        for (name, buffer, row) in dump.pending.drain(..) {
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            {
                let data = match buffer.slice(..).get_mapped_range() {
                    Ok(data) => data,
                    Err(e) => {
                        log::error!("DLSS FG dump: {name}: {e}");
                        continue;
                    }
                };
                for y in 0..h as usize {
                    let line = &data[y * row as usize..][..w as usize * 4];
                    for px in line.as_chunks::<4>().0 {
                        ppm.extend_from_slice(&px[..3]);
                    }
                }
            }
            buffer.unmap();
            let path = dump.folder.join(&name);
            match std::fs::write(&path, ppm) {
                Ok(()) => log::info!("DLSS FG dump: {}", path.display()),
                Err(e) => log::error!("DLSS FG dump: {}: {e}", path.display()),
            }
        }
    }
}

/// The pass that lays the real frame's interface over the generated frame:
/// its pipeline, and its images bound (shown, hudless, generated), which
/// live as long as the `FrameGen`.
fn mix(device: &wgpu::Device, views: [&wgpu::TextureView; 3]) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
    let entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("DLSS FG interface"),
        entries: &[entry(0), entry(1), entry(2)],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("DLSS FG interface"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(views[1]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(views[2]),
            },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("DLSS FG interface"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dlss_fg_mix.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("DLSS FG interface"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("DLSS FG interface"),
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
                format: FORMAT,
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
    (pipeline, group)
}
