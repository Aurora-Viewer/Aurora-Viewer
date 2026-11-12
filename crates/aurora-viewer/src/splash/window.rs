//! The splash process: a small borderless window drawn on the CPU (egui
//! through the crash window's software painter, `crash/software.rs`, into a
//! `softbuffer` surface), so that it needs nothing of the GPU the viewer is
//! still preparing. It reads the viewer's lines on its standard input
//! (`progress::Message`) and closes on `done` or when the viewer is gone.

use super::progress::{Bar, Message, Step};
use crate::theme::Palette;
use crate::ui::widgets;
use egui::{RichText, Vec2};
use std::io::{BufRead, Write};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy, OwnedDisplayHandle},
    window::{Window, WindowId},
};

/// The window's size, in points.
const SIZE: Vec2 = Vec2::new(420.0, 236.0);
const LOGO: f32 = 56.0;
const BAR: f32 = 300.0;

/// What the reader thread hands the window.
#[derive(Debug)]
enum Input {
    Message(Message),
    /// The viewer closed our input: it is gone, or done with us.
    Closed,
}

pub fn run() -> anyhow::Result<()> {
    let event_loop = EventLoop::<Input>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let demo = std::env::var_os("AURORA_DEMO").is_some();
    // `AURORA_DEMO_SPLASH=<step>`: that step shown whatever the viewer says
    let forced = demo
        .then(|| std::env::var("AURORA_DEMO_SPLASH").ok())
        .flatten()
        .and_then(|s| Step::parse(&s));
    // `AURORA_DEMO_SPLASH_HOLD=<seconds>`: open at least that long
    let hold = demo
        .then(|| std::env::var("AURORA_DEMO_SPLASH_HOLD").ok())
        .flatten()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| Duration::from_secs_f32(s.min(600.0)));
    spawn_reader(proxy)?;
    let context = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&context, 1.0);
    // the system's fonts for the scripts the embedded ones lack (Japanese):
    // a few milliseconds
    crate::ui::fonts::fallback::wait();
    crate::ui::fonts::apply(&context, "");
    let mut app = Splash {
        context,
        palette: theme.palette(),
        painter: Default::default(),
        surface: None,
        window: None,
        input: None,
        background: None,
        last_pixels: Vec::new(),
        logo: None,
        step: forced.unwrap_or(Step::Starting),
        forced: forced.is_some(),
        version: aurora_version::version().to_owned(),
        bar: Bar::default(),
        opened: Instant::now(),
        hold,
        closing: false,
        shown: false,
        next_repaint: Some(Instant::now()),
        error: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        anyhow::bail!(error);
    }
    Ok(())
}

/// Reads the viewer's lines; its end (the viewer gone) closes the splash.
fn spawn_reader(proxy: EventLoopProxy<Input>) -> std::io::Result<()> {
    std::thread::Builder::new().name("aurora-splash-input".into()).spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if let Some(message) = Message::parse(&line)
                && proxy.send_event(Input::Message(message)).is_err()
            {
                return;
            }
        }
        let _ = proxy.send_event(Input::Closed);
    })?;
    Ok(())
}

struct Splash {
    context: egui::Context,
    palette: Palette,
    painter: crate::crash::software::Painter,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
    window: Option<Arc<Window>>,
    input: Option<egui_winit::State>,
    /// The card under the content, painted once for a size.
    background: Option<([usize; 2], Vec<u32>)>,
    /// The last frame, for `AURORA_SPLASH_CAPTURE`.
    last_pixels: Vec<u32>,
    logo: Option<egui::TextureHandle>,
    step: Step,
    forced: bool,
    /// Under the title: this executable's, or the one an update installs.
    version: String,
    bar: Bar,
    opened: Instant,
    hold: Option<Duration>,
    closing: bool,
    shown: bool,
    next_repaint: Option<Instant>,
    error: Option<String>,
}

impl Splash {
    /// Closes now, or once `AURORA_DEMO_SPLASH_HOLD` has passed (a held
    /// splash shows first, for its capture).
    fn close(&mut self, event_loop: &ActiveEventLoop) {
        self.closing = true;
        if self.hold.is_none() {
            self.capture();
            event_loop.exit();
        }
    }

    fn capture(&self) {
        let Some(path) = std::env::var_os("AURORA_SPLASH_CAPTURE") else {
            return;
        };
        let (Some(window), false) = (&self.window, self.last_pixels.is_empty()) else {
            return;
        };
        let size = window.inner_size();
        let rgba: Vec<u8> = self
            .last_pixels
            .iter()
            .flat_map(|p| [(*p >> 16) as u8, (*p >> 8) as u8, *p as u8, 255])
            .collect();
        if let Err(e) = image::save_buffer(path, &rgba, size.width, size.height, image::ColorType::Rgba8) {
            eprintln!("startup splash: capture not written: {e}");
        }
    }

    fn logo(&mut self) -> Option<egui::TextureHandle> {
        if self.logo.is_none() {
            let pixels = (LOGO * self.context.pixels_per_point() * 2.0).round() as u32;
            match image::load_from_memory(crate::app::LOGO_PNG) {
                Ok(img) => {
                    let img = img.resize(pixels, pixels, image::imageops::FilterType::Triangle).to_rgba8();
                    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                    self.logo = Some(self.context.load_texture("splash-logo", ci, egui::TextureOptions::LINEAR));
                }
                Err(e) => eprintln!("startup splash: logo not decoded: {e}"),
            }
        }
        self.logo.clone()
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let fraction = self.bar.advance(self.step);
        let logo = self.logo();
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            ui.add_space(26.0);
            ui.vertical_centered(|ui| {
                if let Some(logo) = &logo {
                    ui.add(egui::Image::new(logo).fit_to_exact_size(Vec2::splat(LOGO)));
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Aurora Viewer").size(19.0).strong().color(p.ink));
                ui.label(RichText::new(&self.version).size(11.0).color(p.muted_dim));
                ui.add_space(18.0);
                ui.allocate_ui(Vec2::new(BAR, 30.0), |ui| {
                    let text = format!("{{}} · {}", self.step.status());
                    widgets::progress(ui, &p, "splash", fraction, BAR, &text, None);
                });
            });
        });
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let window = self.window.clone().ok_or_else(|| anyhow::anyhow!("no splash window"))?;
        let size = window.inner_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        let raw = self
            .input
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no splash input"))?
            .take_egui_input(&window);
        let context = self.context.clone();
        let output = context.run_ui(raw, |ui| self.content(ui));
        let ppp = output.pixels_per_point;
        let primitives = context.tessellate(output.shapes, ppp);
        let dims = [size.width as usize, size.height as usize];
        self.painter.update_textures(&output.textures_delta);
        if self.background.as_ref().is_none_or(|(d, _)| *d != dims) {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(dims[0] as f32, dims[1] as f32) / ppp);
            let clipped = context.tessellate(background(&self.palette, &context, rect), ppp);
            let mut pixels = vec![0; dims[0] * dims[1]];
            self.painter.fill_and_draw(&mut pixels, dims, &clipped, ppp, self.palette.navy);
            self.background = Some((dims, pixels));
        }
        let surface = self.surface.as_mut().ok_or_else(|| anyhow::anyhow!("no splash surface"))?;
        surface.resize(width, height).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut buffer = surface.buffer_mut().map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some((_, background)) = &self.background
            && background.len() == buffer.len()
        {
            buffer.copy_from_slice(background);
        }
        self.painter.draw(&mut buffer, dims, &primitives, ppp);
        self.painter.free_textures(&output.textures_delta);
        if std::env::var_os("AURORA_SPLASH_CAPTURE").is_some() {
            self.last_pixels.clear();
            self.last_pixels.extend_from_slice(&buffer);
        }
        buffer.present().map_err(|e| anyhow::anyhow!("{e}"))?;
        if !self.shown {
            // shown once its first image is in it: no white flash
            self.shown = true;
            window.set_visible(true);
            // the viewer's own window takes the focus back when it shows
            #[cfg(windows)]
            // SAFETY: no pointer; a refusal only leaves the focus where it is.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                    windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY,
                );
            }
            // the viewer logs when the splash was on screen; nobody reads it
            // when started by hand
            let mut out = std::io::stdout();
            let _ = writeln!(out, "shown").and_then(|()| out.flush());
        }
        let delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|o| o.repaint_delay)
            .unwrap_or(Duration::MAX);
        if delay < Duration::from_secs(1) {
            self.next_repaint = Some(Instant::now() + delay.max(Duration::from_millis(16)));
        }
        let _ = event_loop;
        Ok(())
    }
}

/// The login screen's accent card over the whole window
/// (`ui/login.rs`, `glass_card`): the aurora's sky out of focus with two of
/// its lights under the panel tint, the brand line along the top, the
/// border of the overlays. Painted once: the sky stands still here.
fn background(p: &Palette, ctx: &egui::Context, rect: egui::Rect) -> Vec<egui::epaint::ClippedShape> {
    use crate::ui::aurora_bg;
    let lights = vec![
        aurora_bg::radial_glow_shape(
            rect.left_top() + Vec2::new(rect.width() * 0.18, rect.height() * 0.12),
            rect.width() * 0.95,
            p.violet,
            0.24,
        ),
        aurora_bg::radial_glow_shape(
            rect.right_bottom() - Vec2::new(rect.width() * 0.12, rect.height() * 0.14),
            rect.width() * 0.8,
            p.teal,
            0.13,
        ),
    ];
    // the moment of the sky the login screen opens on
    let mut shapes = aurora_bg::frosted_pane(p, rect, 12.0, rect, lights);
    shapes.push(crate::ui::login::strip(
        egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), 2.0)),
        &[(0.0, p.violet), (1.0, p.teal)],
    ));
    let stroke = widgets::overlay_frame(ctx, p).stroke;
    let corners = ctx.global_style().visuals.window_corner_radius;
    shapes.push(egui::Shape::rect_stroke(rect, corners, stroke, egui::StrokeKind::Inside));
    shapes
        .into_iter()
        .map(|shape| egui::epaint::ClippedShape {
            clip_rect: rect,
            shape,
            transform_after_tessellation: egui::emath::TSTransform::IDENTITY,
        })
        .collect()
}

/// Centred on the screen the viewer will most likely open on.
fn centred(event_loop: &ActiveEventLoop) -> Option<winit::dpi::PhysicalPosition<i32>> {
    let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next())?;
    let (pos, size, scale) = (monitor.position(), monitor.size(), monitor.scale_factor());
    let (w, h) = ((SIZE.x as f64 * scale) as i32, (SIZE.y as f64 * scale) as i32);
    Some(winit::dpi::PhysicalPosition::new(
        pos.x + (size.width as i32 - w) / 2,
        pos.y + (size.height as i32 - h) / 2,
    ))
}

impl ApplicationHandler<Input> for Splash {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let setup = || -> anyhow::Result<_> {
            let mut attrs = crate::cli::with_desktop_id(Window::default_attributes())
                .with_title(crate::cli::window_title())
                .with_decorations(false)
                .with_resizable(false)
                .with_visible(false)
                .with_inner_size(winit::dpi::LogicalSize::new(SIZE.x, SIZE.y));
            if let Some(at) = centred(event_loop) {
                attrs = attrs.with_position(at);
            }
            #[cfg(windows)]
            {
                use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
                // almost square corners and the shadow of a window
                attrs = attrs
                    .with_corner_preference(CornerPreference::RoundSmall)
                    .with_undecorated_shadow(true);
            }
            let window = Arc::new(event_loop.create_window(attrs)?);
            let display = softbuffer::Context::new(event_loop.owned_display_handle()).map_err(|e| anyhow::anyhow!("{e}"))?;
            let surface = softbuffer::Surface::new(&display, window.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;
            let input = egui_winit::State::new(
                self.context.clone(),
                egui::ViewportId::ROOT,
                &window,
                Some(window.scale_factor() as f32),
                None,
                None,
            );
            Ok((window, surface, input))
        };
        match setup() {
            Ok((window, surface, input)) => {
                self.next_repaint = Some(Instant::now());
                self.window = Some(window);
                self.surface = Some(surface);
                self.input = Some(input);
            }
            Err(error) => {
                self.error = Some(error.to_string());
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, input: Input) {
        match input {
            Input::Message(Message::Step(step)) => {
                if !self.forced && step != self.step {
                    self.step = step;
                    self.next_repaint = Some(Instant::now());
                }
            }
            Input::Message(Message::Version(version)) => {
                if version != self.version {
                    self.version = version;
                    self.next_repaint = Some(Instant::now());
                }
            }
            Input::Message(Message::Done) | Input::Closed => self.close(event_loop),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            // nothing to cancel: the viewer goes on starting behind it
            WindowEvent::CloseRequested => {}
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render(event_loop) {
                    self.error = Some(error.to_string());
                    event_loop.exit();
                }
            }
            event => {
                if let (Some(input), Some(window)) = (&mut self.input, &self.window) {
                    let _ = input.on_window_event(window, &event);
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // asked to close and held long enough (`AURORA_DEMO_SPLASH_HOLD`)
        let leave = self.closing.then(|| self.opened + self.hold.unwrap_or_default());
        if leave.is_some_and(|t| Instant::now() >= t) && self.shown {
            self.capture();
            event_loop.exit();
            return;
        }
        // drawn from here, not on a redraw request: Windows sends none to
        // a window still hidden, and it shows once drawn
        if self.next_repaint.is_some_and(|t| Instant::now() >= t) {
            self.next_repaint = None;
            if let Err(error) = self.render(event_loop) {
                self.error = Some(error.to_string());
                event_loop.exit();
                return;
            }
        }
        let wake = match (self.next_repaint, leave) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
