//! Recovery UI uses the viewer's theme, fonts, Tabler icons and Fluent
//! catalogues, but runs in a separate process with a CPU-only painter.

use crate::ui::{fonts, icons::Icons, widgets};
use egui::{RichText, Vec2};
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle},
    window::{Window, WindowId},
};

pub fn show(dir: &Path, record: super::Record) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    let context = egui::Context::default();
    record.theme.apply(&context, record.font_scale);
    // This window draws on events only: the few milliseconds of the system
    // font scan are waited for here instead of polled each frame.
    fonts::fallback::wait();
    fonts::apply(&context, &record.font);
    let icons = Icons::load(&context, None);
    let trace = super::trace(dir);
    let mut app = Recovery {
        dir: dir.to_owned(),
        record,
        context,
        icons,
        trace,
        painter: Default::default(),
        surface: None,
        window: None,
        input: None,
        error: None,
        last_frame: Instant::now(),
        next_repaint: Some(Instant::now()),
        frames: 0,
        copied: false,
        action_error: None,
        bug_report: Default::default(),
        reason: if dir.join("panic.txt").metadata().is_ok_and(|m| m.len() > 0) {
            aurora_i18n::tr!("crash-reason-rust")
        } else if dir.join("native.txt").metadata().is_ok_and(|m| m.len() > 0) {
            aurora_i18n::tr!("crash-reason-native")
        } else {
            aurora_i18n::tr!("crash-reason-unknown")
        },
    };
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        anyhow::bail!(error);
    }
    Ok(())
}

struct Recovery {
    dir: PathBuf,
    record: super::Record,
    context: egui::Context,
    icons: Icons,
    trace: String,
    painter: super::software::Painter,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
    window: Option<Arc<Window>>,
    input: Option<egui_winit::State>,
    error: Option<String>,
    last_frame: Instant,
    next_repaint: Option<Instant>,
    frames: u32,
    copied: bool,
    action_error: Option<String>,
    reason: &'static str,
    bug_report: crate::ui::bug_report::BugReportUi,
}

impl Recovery {
    fn content(&mut self, ui: &mut egui::Ui, event_loop: &ActiveEventLoop) {
        let p = self.record.theme.palette();
        if self.bug_report.requested {
            self.open_report();
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.panel).inner_margin(28.0))
            .show(ui, |ui| {
                // Do not leave the private stack readable behind the shared
                // report form's translucent viewer surface.
                if self.bug_report.open {
                    return;
                }
                ui.horizontal(|ui| {
                    if let Some(icon) = self.icons.get("alert-triangle-filled") {
                        ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(58.0)).tint(p.amber));
                    }
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("crash-heading")).size(25.0).strong().color(p.ink));
                        ui.add_space(5.0);
                        ui.label(RichText::new(aurora_i18n::tr!("crash-explanation")).color(p.muted));
                    });
                });
                ui.add_space(22.0);
                egui::Frame::new().fill(p.field).inner_margin(14.0).corner_radius(3).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("Aurora Viewer {}", self.record.version)).strong());
                        ui.label(
                            RichText::new(format!("· {} · {}", self.record.profile, self.record.language.to_uppercase())).color(p.muted),
                        );
                        ui.label(RichText::new(self.reason).color(p.amber));
                    });
                    ui.label(RichText::new(aurora_i18n::tr!("crash-local-only")).color(p.muted));
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("crash-details")).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::flat_button(ui, &p, aurora_i18n::tr!("crash-open-folder")).clicked()
                            && let Err(error) = open_folder(&self.dir)
                        {
                            self.action_error = Some(error.to_string());
                        }
                        if ui
                            .add_enabled(!self.trace.is_empty(), egui::Button::new(aurora_i18n::tr!("crash-copy-trace")))
                            .clicked()
                        {
                            self.context.copy_text(self.trace.clone());
                            self.copied = true;
                        }
                    });
                });
                ui.add_space(8.0);
                let height = (ui.available_height() - 106.0).max(140.0);
                egui::Frame::new().fill(p.field).inner_margin(12.0).corner_radius(3).show(ui, |ui| {
                    egui::ScrollArea::both()
                        .id_salt("crash-trace")
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height)
                        .show(ui, |ui| {
                            if self.trace.is_empty() {
                                ui.label(RichText::new(aurora_i18n::tr!("crash-no-trace")).color(p.muted));
                            } else {
                                ui.add(
                                    egui::Label::new(RichText::new(&self.trace).monospace().size(12.0))
                                        .selectable(true)
                                        .wrap_mode(egui::TextWrapMode::Extend),
                                );
                            }
                        });
                });
                ui.add_space(10.0);
                ui.label(
                    RichText::new(if self.copied {
                        aurora_i18n::tr!("crash-copied")
                    } else {
                        aurora_i18n::tr!("crash-private-trace")
                    })
                    .size(12.0)
                    .color(p.muted),
                );
                ui.add_space(14.0);
                if self.action_error.is_some() {
                    ui.colored_label(p.danger, aurora_i18n::tr!("crash-folder-error"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::flat_button_sized(ui, &p, aurora_i18n::tr!("crash-close"), Vec2::new(110.0, 34.0)).clicked() {
                        event_loop.exit();
                    }
                    if widgets::flat_button_sized(ui, &p, aurora_i18n::tr!("crash-send-report"), Vec2::new(232.0, 34.0)).clicked() {
                        self.open_report();
                    }
                });
            });
        self.bug_report.show(ui.ctx(), &p, &self.icons, false);
    }

    fn open_report(&mut self) {
        self.bug_report
            .begin_preserved(super::report::draft(&self.record, &self.dir), self.dir.clone(), self.record.demo);
        if let Some(window) = &self.window {
            let scale = window.scale_factor();
            let height = window
                .current_monitor()
                .map(|m| f64::from(m.size().height) / scale - 100.0)
                .unwrap_or(940.0)
                .clamp(500.0, 940.0);
            let _ = window.request_inner_size(winit::dpi::LogicalSize::new(840.0, height));
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        if let Some(fps) = crate::cli::get().fps_limit {
            let next = self.last_frame + Duration::from_secs_f64(1.0 / f64::from(fps));
            if self.frames > 0 && Instant::now() < next {
                self.next_repaint = Some(next);
                return Ok(());
            }
        }
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing recovery window"))?
            .clone();
        let size = window.inner_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        let raw = self
            .input
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("missing recovery input"))?
            .take_egui_input(&window);
        let context = self.context.clone();
        let output = context.run_ui(raw, |ui| self.content(ui, event_loop));
        self.input
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("missing recovery input"))?
            .handle_platform_output_with_event_loop(&window, event_loop, output.platform_output);
        let primitives = context.tessellate(output.shapes, output.pixels_per_point);
        let surface = self.surface.as_mut().ok_or_else(|| anyhow::anyhow!("missing recovery surface"))?;
        surface.resize(width, height).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut buffer = surface.buffer_mut().map_err(|e| anyhow::anyhow!("{e}"))?;
        self.painter.paint(
            &mut buffer,
            [size.width as usize, size.height as usize],
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
            self.record.theme.palette().panel,
        );
        self.frames += 1;
        if self.record.demo
            && self.frames == 3
            && let Some(path) = std::env::var_os("AURORA_CRASH_CAPTURE")
        {
            let rgba: Vec<_> = buffer
                .iter()
                .flat_map(|p| [(*p >> 16) as u8, (*p >> 8) as u8, *p as u8, 255])
                .collect();
            image::save_buffer(path, &rgba, size.width, size.height, image::ColorType::Rgba8)?;
            if std::env::var_os("AURORA_CRASH_CAPTURE_EXIT").is_some() {
                event_loop.exit();
            }
        }
        buffer.present().map_err(|e| anyhow::anyhow!("{e}"))?;
        self.last_frame = Instant::now();
        let delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|o| o.repaint_delay)
            .unwrap_or(Duration::MAX);
        self.next_repaint = if self.frames < 3 {
            Some(self.last_frame)
        } else {
            self.last_frame.checked_add(delay)
        };
        Ok(())
    }
}

fn open_folder(dir: &Path) -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(dir).spawn().map(|_| ())
}

impl ApplicationHandler for Recovery {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let setup = || -> anyhow::Result<_> {
            let title = match &crate::cli::get().title {
                Some(title) => format!("{} — {title}", aurora_i18n::tr!("crash-title")),
                None => aurora_i18n::tr!("crash-title").to_owned(),
            };
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title(title)
                        .with_inner_size(winit::dpi::LogicalSize::new(840.0, 610.0))
                        .with_min_inner_size(winit::dpi::LogicalSize::new(700.0, 500.0)),
                )?,
            );
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
                window.request_redraw();
                self.window = Some(window);
                self.surface = Some(surface);
                self.input = Some(input);
                if self.record.demo && std::env::var_os("AURORA_DEMO_BUG_REPORT").is_some() {
                    self.open_report();
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
            return;
        }
        if let (Some(input), Some(window)) = (&mut self.input, &self.window) {
            let response = input.on_window_event(window, &event);
            if response.repaint {
                self.next_repaint = Some(Instant::now());
            }
        }
        if matches!(event, WindowEvent::RedrawRequested)
            && let Err(error) = self.render(event_loop)
        {
            self.error = Some(error.to_string());
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let cap = crate::cli::get()
            .fps_limit
            .map(|fps| Duration::from_secs_f64(1.0 / f64::from(fps)))
            .unwrap_or_default();
        if let Some(next) = self.next_repaint {
            let next = next.max(self.last_frame + cap);
            if Instant::now() >= next {
                self.next_repaint = None;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(next));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}
