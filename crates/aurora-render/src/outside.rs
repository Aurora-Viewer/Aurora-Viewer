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
//!
//! What the interface paints into a target of its own shows outside as it
//! does inside: the previews (preview.rs, whose cache the windows share)
//! and the second camera's picture. What it paints through the viewer's
//! frame (the glass, the labels in the world) has nothing to read here and
//! is left out ([`drawable`]).

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
                            // (the surface before its window, as an entry
                            // drops them)
                            drop(surface);
                            drop(keep);
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
    /// `second_view`: the second camera's picture, when it has one;
    /// `look_thumbs`: the photo filters' catalogue pictures.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lost: &crate::device_lost::DeviceLost,
        egui: &mut egui_wgpu::Renderer,
        previews: &mut crate::preview::Cache,
        second_view: Option<egui::TextureId>,
        look_thumbs: Option<&crate::look_thumbs::LookThumbs>,
        index: u64,
    ) -> Drawn {
        let mut drawn = Drawn::default();
        let started = Instant::now();
        // (the previews of a window that closed go with it)
        previews.keep_outside(|key| self.entries.contains_key(&key), egui);
        for (key, entry) in &mut self.entries {
            let now = Instant::now();
            if entry.pending.is_none() || entry.rest_until.is_some_and(|until| now < until) {
                continue;
            }
            entry.rest_until = None;
            if entry.draw(*key, device, queue, lost, egui, previews, second_view, look_thumbs, index) {
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
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        key: u64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lost: &crate::device_lost::DeviceLost,
        egui: &mut egui_wgpu::Renderer,
        previews: &mut crate::preview::Cache,
        second_view: Option<egui::TextureId>,
        look_thumbs: Option<&crate::look_thumbs::LookThumbs>,
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
        let sd = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: ui.pixels_per_point,
        };
        let view = target.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("window outside"),
        });
        drawable(
            &mut ui.primitives,
            ui.pixels_per_point,
            key,
            device,
            queue,
            &mut encoder,
            egui,
            previews,
            second_view,
            look_thumbs,
        );
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

/// What a window outside draws of its pass: the previews, the second
/// camera's picture and the photo filters' pictures become meshes of their
/// targets (drawn into `encoder` first when they changed), as in the
/// viewer's frame; what is painted
/// through the viewer's frame (the glass, the labels in the world, the
/// markers of its passes) has nothing to read here and goes. egui-wgpu
/// gets meshes only.
#[allow(clippy::too_many_arguments)]
fn drawable(
    primitives: &mut Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    key: u64,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    egui: &mut egui_wgpu::Renderer,
    previews: &mut crate::preview::Cache,
    second_view: Option<egui::TextureId>,
    look_thumbs: Option<&crate::look_thumbs::LookThumbs>,
) {
    crate::second_view::replace(primitives, second_view);
    previews.render(
        crate::preview::Shower::Outside(key),
        device,
        queue,
        encoder,
        egui,
        primitives,
        pixels_per_point,
    );
    crate::look_thumbs::resolve(look_thumbs, primitives);
    primitives.retain(|p| matches!(p.primitive, Primitive::Mesh(_)));
}

/// Whether the shapes of two passes of a window outside draw the same
/// picture. egui compares two paint callbacks by their address, and each
/// pass makes new ones: a window that shows a preview would be drawn again
/// at every frame. Two callbacks of one place are the same when they show
/// the same preview; the second camera's picture never is (it changes with
/// every frame of the viewer); and those a window outside does not draw
/// (`drawable`) are the same when they are of one kind.
pub fn same_picture(a: &[egui::epaint::ClippedShape], b: &[egui::epaint::ClippedShape]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| a.clip_rect == b.clip_rect && same_shape(&a.shape, &b.shape))
}

fn same_shape(a: &egui::Shape, b: &egui::Shape) -> bool {
    match (a, b) {
        (egui::Shape::Vec(a), egui::Shape::Vec(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_shape(a, b)),
        (egui::Shape::Callback(a), egui::Shape::Callback(b)) => a.rect == b.rect && same_callback(a, b),
        _ => a == b,
    }
}

fn same_callback(a: &egui::PaintCallback, b: &egui::PaintCallback) -> bool {
    if crate::second_view::is(a) || crate::second_view::is(b) {
        return false;
    }
    // (the catalogue's pictures are drawn again on a new still every few
    // seconds, one a frame: a picture that moves, as the second camera's)
    if crate::look_thumbs::is(a) || crate::look_thumbs::is(b) {
        return false;
    }
    crate::preview_mesh::same(a, b)
        .or_else(|| crate::preview::same(a, b))
        .or_else(|| crate::panorama::same(a, b))
        .or_else(|| crate::picture_look::same(a, b))
        .unwrap_or_else(|| {
            let kind = |c: &egui::PaintCallback| std::any::Any::type_id(&*c.callback);
            kind(a) == kind(b)
        })
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

    /// One pass of a window that shows what `add` paints.
    fn pass(ctx: &egui::Context, add: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, add);
        // (no renderer takes the font atlas here)
        out.textures_delta.clear();
        out
    }

    fn triangle() -> std::sync::Arc<egui::Mesh> {
        let mut mesh = egui::Mesh::default();
        for point in [egui::pos2(12.0, 12.0), egui::pos2(70.0, 20.0), egui::pos2(20.0, 70.0)] {
            mesh.colored_vertex(point, egui::Color32::from_rgb(200, 125, 70));
        }
        mesh.add_triangle(0, 1, 2);
        std::sync::Arc::new(mesh)
    }

    const TILE: egui::Rect = egui::Rect::from_min_max(egui::pos2(8.0, 8.0), egui::pos2(72.0, 72.0));

    /// A callback of the viewer's frame, which a window outside leaves out.
    struct Marker;

    fn marker(ui: &egui::Ui) {
        ui.painter().add(egui::Shape::Callback(egui::PaintCallback {
            rect: TILE,
            callback: std::sync::Arc::new(Marker),
        }));
    }

    #[test]
    fn a_preview_shown_again_is_the_same_picture() {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("tile");
        let mesh = triangle();
        let tile = |mesh: &std::sync::Arc<egui::Mesh>| {
            let mesh = mesh.clone();
            pass(&ctx, move |ui| {
                marker(ui);
                crate::preview_mesh::paint(ui, id, TILE, mesh.clone());
            })
            .shapes
        };
        let (first, again) = (tile(&mesh), tile(&mesh));
        // egui compares the callbacks by their address: new ones each pass
        assert!(first != again);
        assert!(same_picture(&first, &again));
        // another mesh, another place, one shape less: another picture
        assert!(!same_picture(&first, &tile(&triangle())));
        let mut moved = again.clone();
        moved[0].clip_rect = moved[0].clip_rect.translate(egui::vec2(1.0, 0.0));
        assert!(!same_picture(&first, &moved));
        assert!(!same_picture(&first, &again[1..]));
        // inside a group of shapes too
        let group = |shapes: &[egui::epaint::ClippedShape]| {
            vec![egui::epaint::ClippedShape {
                shape: egui::Shape::Vec(shapes.iter().map(|s| s.shape.clone()).collect()),
                ..shapes[0].clone()
            }]
        };
        assert!(same_picture(&group(&first), &group(&again)));
        assert!(!same_picture(&group(&first), &group(&tile(&triangle()))));
    }

    #[test]
    fn the_filters_pictures_are_never_the_same_picture() {
        let ctx = egui::Context::default();
        let tiles = || {
            pass(&ctx, |ui| {
                crate::look_thumbs::paint(ui.painter(), TILE, 0, egui::Color32::WHITE, 2.0);
            })
            .shapes
        };
        assert!(!same_picture(&tiles(), &tiles()));
    }

    #[test]
    fn the_second_camera_is_never_the_same_picture() {
        let ctx = egui::Context::default();
        let live = || pass(&ctx, |ui| crate::second_view::paint(ui, TILE)).shapes;
        assert!(!same_picture(&live(), &live()));
        // and a preview is not the marker that stood there
        let preview = pass(&ctx, |ui| {
            crate::preview_mesh::paint(ui, egui::Id::unique("tile"), TILE, triangle())
        })
        .shapes;
        let marked = pass(&ctx, |ui| marker(ui)).shapes;
        assert!(!same_picture(&preview, &marked));
        assert!(same_picture(&marked, &pass(&ctx, |ui| marker(ui)).shapes));
    }

    /// The Build tools' shapes outside (#925): a preview is drawn in a
    /// window outside, kept while that window shows it whatever the viewer's
    /// own frames show, and let go with the window.
    #[test]
    fn a_preview_outside_is_drawn_and_kept_while_its_window_shows_it() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("test GPU");
        let mut egui = egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, Default::default());
        let mut previews = crate::preview::Cache::default();
        let ctx = egui::Context::default();
        let mesh = triangle();
        let shown = |previews: &mut crate::preview::Cache, egui: &mut egui_wgpu::Renderer| {
            let mesh = mesh.clone();
            let out = pass(&ctx, move |ui| {
                marker(ui);
                crate::preview_mesh::paint(ui, egui::Id::unique("tile"), TILE, mesh.clone());
            });
            let mut primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
            assert!(primitives.iter().any(|p| matches!(p.primitive, Primitive::Callback(_))));
            let mut encoder = device.create_command_encoder(&Default::default());
            drawable(
                &mut primitives,
                out.pixels_per_point,
                7,
                &device,
                &queue,
                &mut encoder,
                egui,
                previews,
                None,
                None,
            );
            queue.submit([encoder.finish()]);
            // meshes only, and the preview is one of a texture of its own
            let textures: Vec<egui::TextureId> = primitives
                .iter()
                .map(|p| match &p.primitive {
                    Primitive::Mesh(mesh) => mesh.texture_id,
                    Primitive::Callback(_) => panic!("a callback is left for egui-wgpu"),
                })
                .filter(|texture| matches!(texture, egui::TextureId::User(_)))
                .collect();
            assert_eq!(textures.len(), 1, "the preview's mesh");
            textures[0]
        };
        let first = shown(&mut previews, &mut egui);
        // frames of the viewer that show no preview: the window's stays
        for _ in 0..3 {
            let mut encoder = device.create_command_encoder(&Default::default());
            previews.render(
                crate::preview::Shower::Viewer,
                &device,
                &queue,
                &mut encoder,
                &mut egui,
                &mut [],
                1.0,
            );
            queue.submit([encoder.finish()]);
        }
        assert_eq!(shown(&mut previews, &mut egui), first, "kept, not made again");
        // another window outside closes: nothing changes for this one
        previews.keep_outside(|key| key == 7, &mut egui);
        assert_eq!(shown(&mut previews, &mut egui), first);
        // its own window closes: the preview goes, and a new one is made
        previews.keep_outside(|_| false, &mut egui);
        assert_ne!(shown(&mut previews, &mut egui), first, "let go with its window");
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
