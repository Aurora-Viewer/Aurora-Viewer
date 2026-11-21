//! Windows of the interface outside the viewer's window: a window shown
//! in a native window of its own (another screen), drawn by the viewer's
//! renderer.
//!
//! One device, one queue and one egui renderer draw every window: the
//! textures the interface loaded (the fonts, the pictures) are the main
//! window's, and nothing is made twice. The main thread opens a surface on
//! the native window (`Renderer::open_outside`) and hands the window's
//! shapes over when what it shows changed (`Renderer::draw_outside`); the
//! render thread owns the surface from then on. It draws the windows
//! outside once the viewer's own frame is submitted and presented, each in
//! a submission of its own: egui-wgpu has one set of buffers, written again
//! for each window.
//!
//! A window outside never makes the 3D view wait for a screen. Its
//! swapchain does not synchronize (`Mailbox`, else `Immediate`), it is
//! drawn only when a frame was handed for it, and a surface that held the
//! render thread all the same (a hidden window under `Fifo`, where the
//! compositor stops answering) is left alone for a while ([`REST`]).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use egui::epaint::Primitive;

use crate::render_thread::KeepAlive;
use crate::ui_frame::EguiFrame;

/// A draw of a window outside this long held the render thread: the
/// viewer's frames waited behind it.
const HELD: Duration = Duration::from_millis(50);
/// How long a surface that held the render thread is left alone.
const REST: Duration = Duration::from_secs(1);

/// The interface of one window outside, as its pass left it.
pub struct OutsideFrame {
    /// The window's drawable size, in pixels.
    pub width: u32,
    pub height: u32,
    /// What the window shows where the interface draws nothing.
    pub clear: egui::Color32,
    /// The texture changes of the pass go with the viewer's own frame
    /// (`Renderer::draw_outside` moves them there): the textures are shared.
    pub ui: EguiFrame,
}

/// What became of a window outside on the render thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutsideEvent {
    /// Its surface is destroyed: the native window can go.
    Closed(u64),
    /// Its surface cannot be drawn by the viewer's device; it is destroyed.
    Failed(u64, String),
}

pub(crate) enum Change {
    Open {
        key: u64,
        // dropped before `keep` (field order): the window outlives its surface
        surface: wgpu::Surface<'static>,
        keep: KeepAlive,
        width: u32,
        height: u32,
        /// Synchronized with its screen (`Fifo`): only to measure what the
        /// rest protects from.
        sync: bool,
    },
    Close(u64),
}

/// What a frame packet carries for the windows outside.
#[derive(Default)]
pub(crate) struct Packet {
    pub changes: Vec<Change>,
    /// At most one frame per window: the last handed.
    pub frames: Vec<(u64, OutsideFrame)>,
}

impl Packet {
    /// `frame` replaces the one waiting for the same window.
    pub fn hand(&mut self, key: u64, frame: OutsideFrame) {
        self.frames.retain(|(k, _)| *k != key);
        self.frames.push((key, frame));
    }
}

struct Entry {
    // dropped before `keep` (field order): the window outlives its surface
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Handed and not drawn yet.
    pending: Option<OutsideFrame>,
    /// Left alone until then ([`REST`]).
    rest_until: Option<Instant>,
    #[allow(dead_code)] // only held, to drop after the surface
    keep: KeepAlive,
}

/// The render thread's side: the surfaces of the windows outside.
#[derive(Default)]
pub(crate) struct Surfaces {
    entries: BTreeMap<u64, Entry>,
    events: Vec<OutsideEvent>,
}

/// What `draw` did, for the statistics.
#[derive(Default, Clone, Copy)]
pub(crate) struct Drawn {
    pub windows: u32,
    pub ms: f32,
}

/// The configuration of a window outside: the viewer's output `format`
/// (egui-wgpu's pipeline is made for it), never synchronized unless asked.
fn configuration(
    caps: &wgpu::SurfaceCapabilities,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    sync: bool,
) -> Result<wgpu::SurfaceConfiguration, String> {
    if !caps.formats.contains(&format) {
        return Err(format!("its surface has no {format:?} (it offers {:?})", caps.formats));
    }
    let present_mode = if sync {
        wgpu::PresentMode::Fifo
    } else {
        crate::present::choose(&caps.present_modes, false, None)
    };
    Ok(wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: width.max(1),
        height: height.max(1),
        present_mode,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: vec![],
        color_space: Default::default(),
    })
}

/// `color` as the clear value of a target of `format`: egui writes
/// display-encoded values to a target that is not sRGB, and wgpu encodes
/// what goes to one that is.
fn clear_value(color: egui::Color32, format: wgpu::TextureFormat) -> wgpu::Color {
    let [r, g, b, _] = if format.is_srgb() {
        egui::Rgba::from(color).to_array()
    } else {
        color.to_array().map(|c| f32::from(c) / 255.0)
    };
    wgpu::Color {
        r: f64::from(r),
        g: f64::from(g),
        b: f64::from(b),
        a: 1.0,
    }
}

/// When a surface whose draw took `took` may be drawn again.
fn rest_after(took: Duration, now: Instant) -> Option<Instant> {
    (took >= HELD).then(|| now + REST)
}

impl Surfaces {
    /// The openings and closings a packet carries, then its frames.
    pub fn take(&mut self, device: &wgpu::Device, adapter: &wgpu::Adapter, format: wgpu::TextureFormat, packet: Packet) {
        for change in packet.changes {
            match change {
                Change::Open {
                    key,
                    surface,
                    keep,
                    width,
                    height,
                    sync,
                } => {
                    let made = if adapter.is_surface_supported(&surface) {
                        configuration(&surface.get_capabilities(adapter), format, width, height, sync)
                    } else {
                        Err("the viewer's graphics card does not draw to it".into())
                    };
                    match made {
                        Ok(config) => {
                            crate::present::configure(&surface, device, &config);
                            log::info!(
                                "window outside {key:x}: {} x {}, presentation {:?}",
                                config.width,
                                config.height,
                                config.present_mode
                            );
                            self.entries.insert(
                                key,
                                Entry {
                                    surface,
                                    config,
                                    pending: None,
                                    rest_until: None,
                                    keep,
                                },
                            );
                        }
                        Err(why) => {
                            log::warn!("window outside {key:x}: {why}");
                            self.events.push(OutsideEvent::Failed(key, why));
                        }
                    }
                }
                Change::Close(key) => {
                    // (the entry's surface, then its window's keep-alive)
                    self.entries.remove(&key);
                    self.events.push(OutsideEvent::Closed(key));
                }
            }
        }
        for (key, frame) in packet.frames {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.pending = Some(frame);
            }
        }
    }

    pub fn take_events(&mut self) -> Vec<OutsideEvent> {
        std::mem::take(&mut self.events)
    }

    /// Draws and presents every window a frame waits for. The viewer's own
    /// frame is submitted: egui's buffers are free to be written again.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lost: &crate::device_lost::DeviceLost,
        egui: &mut egui_wgpu::Renderer,
        index: u64,
    ) -> Drawn {
        let mut drawn = Drawn::default();
        let started = Instant::now();
        for (key, entry) in &mut self.entries {
            let now = Instant::now();
            if entry.pending.is_none() || entry.rest_until.is_some_and(|until| now < until) {
                continue;
            }
            entry.rest_until = None;
            if entry.draw(device, queue, lost, egui, index) {
                drawn.windows += 1;
            }
            let took = now.elapsed();
            entry.rest_until = rest_after(took, Instant::now());
            if entry.rest_until.is_some() {
                log::warn!(
                    "window outside {key:x}: its draw held the render thread {:.0} ms, left alone for {REST:?}",
                    took.as_secs_f32() * 1000.0
                );
            }
        }
        if drawn.windows > 0 {
            drawn.ms = started.elapsed().as_secs_f32() * 1000.0;
        }
        drawn
    }

    /// After a device loss: the surfaces are left to the process, as the
    /// viewer's own is (`Backend`'s drop, #1650). Their windows are let go
    /// of: the viewer closes them and opens new ones for the renderer of
    /// the recovery, as it replaces its own window.
    pub fn leak(&mut self) {
        for (_, entry) in std::mem::take(&mut self.entries) {
            let Entry {
                surface, pending, keep, ..
            } = entry;
            std::mem::forget(surface);
            drop(pending);
            drop(keep);
        }
    }
}

impl Entry {
    /// Draws the frame waiting, if the surface gives an image; the frame
    /// stays for the next try otherwise.
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lost: &crate::device_lost::DeviceLost,
        egui: &mut egui_wgpu::Renderer,
        index: u64,
    ) -> bool {
        let Some((width, height)) = self.pending.as_ref().map(|f| (f.width.max(1), f.height.max(1))) else {
            return false;
        };
        if (self.config.width, self.config.height) != (width, height) {
            self.config.width = width;
            self.config.height = height;
            crate::present::configure(&self.surface, device, &self.config);
        }
        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                crate::present::configure(&self.surface, device, &self.config);
                return false;
            }
            _ => return false,
        };
        let Some(frame) = self.pending.take() else {
            return false;
        };
        let clear = clear_value(frame.clear, self.config.format);
        let mut ui = frame.ui.tessellate();
        // what the interface paints through the viewer's frame (the glass,
        // the previews, the labels) has nothing to read here
        ui.primitives.retain(|p| matches!(p.primitive, Primitive::Mesh(_)));
        let sd = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: ui.pixels_per_point,
        };
        let view = target.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("window outside"),
        });
        let mut cmds = egui.update_buffers(device, queue, &mut encoder, &ui.primitives, &sd);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("window outside"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            // (without meshes egui-wgpu keeps an earlier frame's slices, #1235)
            if crate::ui_frame::meshes_uploaded(&ui.primitives) {
                egui.render(&mut pass, &ui.primitives, &sd);
            }
        }
        cmds.push(encoder.finish());
        crate::device_lost::submit_tracked(lost, queue, index, cmds);
        queue.present(target);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(formats: &[wgpu::TextureFormat], modes: &[wgpu::PresentMode]) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats: formats.to_vec(),
            present_modes: modes.to_vec(),
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
            ..Default::default()
        }
    }

    #[test]
    fn a_window_outside_never_waits_for_its_screen_when_it_can() {
        use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let mode =
            |modes: &[wgpu::PresentMode], sync| configuration(&caps(&[format], modes), format, 640, 480, sync).map(|c| c.present_mode);
        assert_eq!(mode(&[Fifo, FifoRelaxed, Mailbox, Immediate], false), Ok(Mailbox));
        assert_eq!(mode(&[Fifo, Immediate], false), Ok(Immediate));
        // a surface with nothing else: the rest protects the viewer's frames
        assert_eq!(mode(&[Fifo], false), Ok(Fifo));
        assert_eq!(mode(&[Fifo, Mailbox], true), Ok(Fifo));
    }

    #[test]
    fn a_window_outside_takes_the_viewer_s_format_or_nothing() {
        let ours = wgpu::TextureFormat::Bgra8Unorm;
        let other = wgpu::TextureFormat::Rgba16Float;
        let modes = [wgpu::PresentMode::Fifo];
        let made = configuration(&caps(&[other, ours], &modes), ours, 0, 0, false).expect("the format is offered");
        assert_eq!(made.format, ours);
        // a window not laid out yet: never an empty swapchain
        assert_eq!((made.width, made.height), (1, 1));
        let refused = configuration(&caps(&[other], &modes), ours, 640, 480, false);
        assert!(refused.is_err_and(|why| why.contains("Bgra8Unorm")));
    }

    #[test]
    fn the_clear_value_is_what_egui_would_write() {
        let grey = egui::Color32::from_rgb(128, 128, 128);
        let plain = clear_value(grey, wgpu::TextureFormat::Bgra8Unorm);
        assert!((plain.r - 128.0 / 255.0).abs() < 1e-6);
        // an sRGB target encodes: the value is given linear
        let encoded = clear_value(grey, wgpu::TextureFormat::Bgra8UnormSrgb);
        assert!((encoded.r - 0.2158).abs() < 1e-3, "{}", encoded.r);
        assert_eq!(plain.a, 1.0);
    }

    #[test]
    fn a_surface_that_held_the_render_thread_rests() {
        let now = Instant::now();
        assert_eq!(rest_after(Duration::from_millis(2), now), None);
        assert_eq!(rest_after(HELD, now), Some(now + REST));
        assert_eq!(rest_after(Duration::from_secs(1), now), Some(now + REST));
    }

    #[test]
    fn the_last_frame_handed_replaces_the_one_waiting() {
        let ctx = egui::Context::default();
        let mut hand_over = crate::EguiHandOver::default();
        let mut frame = |width| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
            OutsideFrame {
                width,
                height: 10,
                clear: egui::Color32::BLACK,
                ui: hand_over.frame(&ctx, out.shapes, out.textures_delta, out.pixels_per_point),
            }
        };
        let mut packet = Packet::default();
        packet.hand(7, frame(100));
        packet.hand(9, frame(300));
        packet.hand(7, frame(200));
        let sizes: Vec<(u64, u32)> = packet.frames.iter().map(|(key, f)| (*key, f.width)).collect();
        assert_eq!(sizes, [(9, 300), (7, 200)]);
    }
}
