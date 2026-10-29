//! Recovery UI uses the viewer's theme, fonts, Tabler icons and Fluent
//! catalogues, but runs in a separate process with a CPU-only painter.

use crate::settings::CrashReports;
use crate::ui::{
    fonts,
    icons::Icons,
    widgets::{self, ActionKind},
};
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

pub fn show(dir: &Path, record: super::Record, summary: super::summary::Summary) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    let context = egui::Context::default();
    record.theme.apply(&context, record.font_scale);
    // This window draws on events only: the few milliseconds of the system
    // font scan are waited for here instead of polled each frame.
    fonts::fallback::wait();
    fonts::apply(&context, &record.font);
    let icons = Icons::load(&context, None);
    let trace = super::trace(dir);
    let sending = Sending::new(dir, &record);
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
        summary,
        sending,
        // Bug report form (disabled, see main.rs):
        // bug_report: Default::default(),
        reason: if dir.join("thread.txt").exists() {
            aurora_i18n::tr!("crash-reason-thread")
        } else if dir.join("panic.txt").metadata().is_ok_and(|m| m.len() > 0) {
            aurora_i18n::tr!("crash-reason-rust")
        } else if dir.join("oom.txt").metadata().is_ok_and(|m| m.len() > 0) {
            aurora_i18n::tr!("crash-reason-oom")
        } else if dir.join("native.txt").metadata().is_ok_and(|m| m.len() > 0) {
            aurora_i18n::tr!("crash-reason-native")
        } else if dir.join("gpu-lost.txt").exists() {
            aurora_i18n::tr!("crash-reason-gpu-lost")
        } else if dir.join("gpu-stall.txt").metadata().is_ok_and(|m| m.len() > 0) {
            aurora_i18n::tr!("crash-reason-gpu")
        } else if dir.join("freeze.txt").metadata().is_ok_and(|m| m.len() > 0) {
            aurora_i18n::tr!("crash-reason-freeze")
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
    summary: super::summary::Summary,
    sending: Sending,
    // Bug report form (disabled, see main.rs):
    // bug_report: crate::ui::bug_report::BugReportUi,
}

/// The report's way to GitHub: the Preferences choice of the crashed
/// session, the preview the tester reads before publishing, and the issue
/// once created (`sent.txt` keeps it, so a reopened window never sends twice).
struct Sending {
    mode: CrashReports,
    github: super::github::Session,
    preview: bool,
    description: String,
    /// The tester's answer: their graphics drivers are up to date.
    drivers_current: bool,
    sent: Option<u64>,
    body_copied: bool,
}

impl Sending {
    fn new(dir: &Path, record: &super::Record) -> Self {
        let mut github = super::github::Session::new(record.demo);
        let mut mode = record.crash_reports;
        let mut preview = false;
        let mut sent = super::read_limited(&dir.join("sent.txt"), 32)
            .ok()
            .and_then(|n| n.trim().parse().ok());
        // `AURORA_DEMO_CRASH_SEND=ask|always|never[,connected][,preview][,authorize][,sent][,fail]`:
        // the demo's simulated GitHub (docs/TESTING.md).
        if record.demo
            && let Ok(options) = std::env::var("AURORA_DEMO_CRASH_SEND")
        {
            for option in options.split(',') {
                match option {
                    "ask" => mode = CrashReports::Ask,
                    "always" => mode = CrashReports::Always,
                    "never" => mode = CrashReports::Never,
                    "connected" => github.demo_authorize(),
                    "preview" => preview = true,
                    "authorize" => {
                        preview = true;
                        github.authorize();
                    }
                    "sent" => sent = Some(4242),
                    _ => {}
                }
            }
        }
        Self {
            mode,
            github,
            preview: (preview || opens_on_preview(mode, sent.is_some())) && mode != CrashReports::Never,
            description: String::new(),
            drivers_current: false,
            sent,
            body_copied: false,
        }
    }
}

/// Always opens straight on the report to publish; it is never published
/// without the tester's words (a crash without them can rarely be
/// reproduced), so nothing is sent by itself.
fn opens_on_preview(mode: CrashReports, sent: bool) -> bool {
    mode == CrashReports::Always && !sent
}

/// A report needs the tester's words: a stack alone rarely says what led
/// to it.
const MIN_DESCRIPTION: usize = 10;

fn described(description: &str) -> bool {
    description.trim().chars().count() >= MIN_DESCRIPTION
}

/// The tester's own words come first, then their answer about the
/// drivers; HTML comments are neutralized so they cannot imitate the
/// markers the triage reads.
fn issue_body(report: &str, description: &str, drivers_current: bool) -> String {
    format!(
        "### What happened\n\n{}\n\nGPU drivers up to date (tester): {}\n\n{report}",
        description.trim().replace("<!--", "&lt;!--"),
        if drivers_current { "yes" } else { "no or not sure" }
    )
}

impl Recovery {
    fn content(&mut self, ui: &mut egui::Ui, event_loop: &ActiveEventLoop) {
        let p = self.record.theme.palette();
        self.sending.github.poll();
        if let Some(number) = self.sending.github.created
            && self.sending.sent.is_none()
        {
            if let Err(error) = std::fs::write(self.dir.join("sent.txt"), number.to_string()) {
                self.action_error = Some(error.to_string());
            }
            self.sending.sent = Some(number);
            self.sending.preview = false;
        }
        // Bug report form (disabled, see main.rs):
        // if self.bug_report.requested {
        //     self.open_report();
        // }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.panel).inner_margin(28.0))
            .show(ui, |ui| {
                // Bug report form (disabled, see main.rs):
                // // Do not leave the private stack readable behind the shared
                // // report form's translucent viewer surface.
                // if self.bug_report.open {
                //     return;
                // }
                let radius = ui.visuals().window_corner_radius;
                ui.horizontal(|ui| {
                    if let Some(icon) = self.icons.get("alert-triangle-filled") {
                        ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(44.0)).tint(p.amber));
                    }
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("crash-heading")).size(20.0).strong().color(p.ink));
                        ui.add_space(3.0);
                        ui.label(RichText::new(aurora_i18n::tr!("crash-explanation")).color(p.muted));
                    });
                });
                ui.add_space(18.0);
                egui::Frame::new()
                    .fill(p.field)
                    .inner_margin(14.0)
                    .corner_radius(radius)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("Aurora Viewer {}", self.record.version)).strong());
                            ui.label(
                                RichText::new(format!("· {} · {}", self.record.profile, self.record.language.to_uppercase()))
                                    .color(p.muted),
                            );
                            ui.label(RichText::new(self.reason).color(p.amber));
                        });
                        self.status(ui, &p);
                    });
                ui.add_space(18.0);
                if self.sending.preview {
                    self.preview(ui, &p);
                    return;
                }
                ui.horizontal(|ui| {
                    if let Some(icon) = self.icons.get("code") {
                        ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(17.0)).tint(p.violet_light));
                    }
                    ui.label(RichText::new(aurora_i18n::tr!("crash-details")).size(13.0).strong().color(p.ink));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let small = Vec2::new(0.0, 26.0);
                        let open = aurora_i18n::tr!("crash-open-folder");
                        if widgets::action_button(ui, &p, "folder-open-filled", open, ActionKind::Flat, small).clicked()
                            && let Err(error) = open_folder(&self.dir)
                        {
                            self.action_error = Some(error.to_string());
                        }
                        let copy = aurora_i18n::tr!("crash-copy-trace");
                        if ui
                            .add_enabled_ui(!self.trace.is_empty(), |ui| {
                                widgets::action_button(ui, &p, "copy-filled", copy, ActionKind::Flat, small)
                            })
                            .inner
                            .clicked()
                        {
                            self.context.copy_text(self.trace.clone());
                            self.copied = true;
                        }
                    });
                });
                ui.add_space(8.0);
                let height = (ui.available_height() - 104.0).max(140.0);
                egui::Frame::new()
                    .fill(p.field)
                    .inner_margin(12.0)
                    .corner_radius(radius)
                    .show(ui, |ui| {
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
                    let big = Vec2::new(0.0, 32.0);
                    if widgets::action_button(ui, &p, "x", aurora_i18n::tr!("crash-close"), ActionKind::Flat, big).clicked() {
                        event_loop.exit();
                    }
                    // Never: the report stays on this PC, nothing to send.
                    let sendable = self.sending.mode != CrashReports::Never && self.sending.sent.is_none() && !self.sending.github.sending;
                    let send = aurora_i18n::tr!("crash-send-report");
                    if sendable && widgets::action_button(ui, &p, "send-filled", send, ActionKind::Primary, big).clicked() {
                        self.open_preview();
                    }
                });
            });
        // Bug report form (disabled, see main.rs):
        // self.bug_report.show(ui.ctx(), &p, &self.icons, false);
    }

    /// The second line of the session card: where the report stands.
    fn status(&self, ui: &mut egui::Ui, p: &crate::theme::Palette) {
        let github = &self.sending.github;
        if let Some(number) = self.sending.sent {
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("crash-sent", number = number.to_string())).color(p.ink));
                let view = aurora_i18n::tr!("crash-view-issue");
                if widgets::action_button(ui, p, "external-link-filled", view, ActionKind::Flat, Vec2::new(0.0, 24.0)).clicked() {
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab(format!("{}/{number}", super::github::ISSUES)));
                }
            });
        } else if github.sending {
            ui.label(RichText::new(aurora_i18n::tr!("crash-sending")).color(p.muted));
        } else {
            ui.label(RichText::new(aurora_i18n::tr!("crash-local-only")).color(p.muted));
        }
    }

    fn open_preview(&mut self) {
        self.sending.preview = true;
        self.sending.body_copied = false;
        // Room for the text the tester reads before publishing.
        if let Some(window) = &self.window {
            let scale = window.scale_factor();
            let height = window
                .current_monitor()
                .map(|m| f64::from(m.size().height) / scale - 100.0)
                .unwrap_or(760.0)
                .clamp(500.0, 760.0);
            let _ = window.request_inner_size(winit::dpi::LogicalSize::new(840.0, height));
        }
    }

    /// Exactly what the issue will contain, the tester's optional words,
    /// then GitHub: connect once, publish.
    fn preview(&mut self, ui: &mut egui::Ui, p: &crate::theme::Palette) {
        let radius = ui.visuals().window_corner_radius;
        let busy = self.sending.github.sending;
        ui.label(
            RichText::new(aurora_i18n::tr!("crash-preview-heading"))
                .size(13.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(3.0);
        ui.label(RichText::new(aurora_i18n::tr!("crash-preview-explanation")).color(p.muted));
        ui.add_space(10.0);
        ui.label(RichText::new(aurora_i18n::tr!("crash-description")).size(12.5).color(p.muted));
        ui.add_space(3.0);
        ui.add_enabled(
            !busy,
            egui::TextEdit::multiline(&mut self.sending.description)
                .desired_rows(3)
                .char_limit(2000)
                .desired_width(f32::INFINITY),
        );
        ui.add_space(6.0);
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                widgets::checkbox(ui, p, &mut self.sending.drivers_current, aurora_i18n::tr!("crash-drivers-current"));
                if let Some(gpu) = &self.summary.gpu {
                    ui.label(RichText::new(aurora_i18n::tr!("crash-drivers-found", gpu = gpu.clone())).color(p.muted));
                }
            });
        });
        ui.add_space(10.0);
        let described = described(&self.sending.description);
        let body = issue_body(&self.summary.body, &self.sending.description, self.sending.drivers_current);
        let height = (ui.available_height() - 104.0).max(120.0);
        egui::Frame::new()
            .fill(p.field)
            .inner_margin(12.0)
            .corner_radius(radius)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("crash-preview")
                    .auto_shrink([false, false])
                    .max_height(height)
                    .min_scrolled_height(height)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&body).monospace().size(12.0)).selectable(true));
                    });
            });
        ui.add_space(8.0);
        let github = &self.sending.github;
        let small = Vec2::new(0.0, 26.0);
        if let Some(code) = github.user_code.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("crash-github-code")).color(p.muted));
                ui.label(RichText::new(&code).monospace().size(18.0).strong().color(p.ink));
                if widgets::action_button(
                    ui,
                    p,
                    "copy-filled",
                    aurora_i18n::tr!("crash-github-copy-code"),
                    ActionKind::Flat,
                    small,
                )
                .clicked()
                {
                    ui.ctx().copy_text(code.clone());
                }
                let open = aurora_i18n::tr!("crash-github-open");
                if widgets::action_button(ui, p, "external-link-filled", open, ActionKind::Flat, small).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(super::github::VERIFY));
                }
            });
        } else if github.authorizing() {
            ui.label(RichText::new(aurora_i18n::tr!("crash-github-waiting")).color(p.muted));
        } else if busy {
            ui.label(RichText::new(aurora_i18n::tr!("crash-sending")).color(p.muted));
        } else if let Some(failure) = github.failure {
            ui.colored_label(p.danger, failure.message());
        } else if self.sending.body_copied {
            ui.label(RichText::new(aurora_i18n::tr!("crash-body-copied")).color(p.muted));
        } else if !described {
            ui.label(RichText::new(aurora_i18n::tr!("crash-description-required")).color(p.amber));
        } else {
            ui.label(RichText::new(aurora_i18n::tr!("crash-preview-private")).size(12.0).color(p.muted));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let connected = self.sending.github.connected();
            if connected
                && !busy
                && widgets::action_button(ui, p, "unlink", aurora_i18n::tr!("crash-github-forget"), ActionKind::Flat, small).clicked()
            {
                self.sending.github.forget();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let big = Vec2::new(0.0, 32.0);
                let back = aurora_i18n::tr!("crash-back");
                if ui
                    .add_enabled_ui(!busy, |ui| widgets::action_button(ui, p, "arrow-left", back, ActionKind::Flat, big))
                    .inner
                    .clicked()
                {
                    self.sending.github.cancel_authorization();
                    self.sending.preview = false;
                }
                if connected {
                    let publish = aurora_i18n::tr!("crash-publish");
                    if ui
                        .add_enabled_ui(!busy && described, |ui| {
                            widgets::action_button(ui, p, "send-filled", publish, ActionKind::Primary, big)
                        })
                        .inner
                        .clicked()
                    {
                        self.sending.github.send(self.summary.title.clone(), body.clone());
                    }
                } else if self.sending.github.authorizing() {
                    let cancel = aurora_i18n::tr!("crash-github-cancel");
                    if widgets::action_button(ui, p, "x", cancel, ActionKind::Flat, big).clicked() {
                        self.sending.github.cancel_authorization();
                    }
                } else {
                    let connect = aurora_i18n::tr!("crash-github-connect");
                    if widgets::action_button(ui, p, "brand-github-filled", connect, ActionKind::Primary, big).clicked() {
                        self.sending.github.authorize();
                    }
                }
                // The same text through the browser, for a tester who prefers
                // not to authorize the app.
                let browser = aurora_i18n::tr!("crash-open-browser");
                if !busy
                    && ui
                        .add_enabled_ui(described, |ui| {
                            widgets::action_button(ui, p, "external-link-filled", browser, ActionKind::Flat, big)
                        })
                        .inner
                        .clicked()
                {
                    let (url, with_body) = super::github::browser_url(&self.summary.title, &body);
                    if !with_body {
                        ui.ctx().copy_text(body.clone());
                    }
                    self.sending.body_copied = !with_body;
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            });
        });
    }

    // Bug report form (disabled, see main.rs):
    // fn open_report(&mut self) {
    //     self.bug_report
    //         .begin_preserved(super::report::draft(&self.record, &self.dir), self.dir.clone(), self.record.demo);
    //     if let Some(window) = &self.window {
    //         let scale = window.scale_factor();
    //         let height = window
    //             .current_monitor()
    //             .map(|m| f64::from(m.size().height) / scale - 100.0)
    //             .unwrap_or(940.0)
    //             .clamp(500.0, 940.0);
    //         let _ = window.request_inner_size(winit::dpi::LogicalSize::new(840.0, height));
    //     }
    // }

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
        } else if self.sending.github.busy() {
            // The GitHub worker answers on a channel, not with an event.
            Some(self.last_frame + delay.min(Duration::from_millis(200)))
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
                    crate::cli::with_desktop_id(Window::default_attributes())
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
                // Demo options that open on the preview.
                if self.sending.preview {
                    self.open_preview();
                }
                // Bug report form (disabled, see main.rs):
                // if self.record.demo && std::env::var_os("AURORA_DEMO_BUG_REPORT").is_some() {
                //     self.open_report();
                // }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_opens_on_the_report_to_publish_once() {
        assert!(opens_on_preview(CrashReports::Always, false));
        for (mode, sent) in [
            (CrashReports::Always, true),
            (CrashReports::Ask, false),
            (CrashReports::Never, false),
        ] {
            assert!(!opens_on_preview(mode, sent), "{mode:?} {sent}");
        }
    }

    #[test]
    fn the_testers_words_lead_the_issue_without_imitating_its_markers() {
        let report = "<!-- aurora-crash v1 -->\n| Field | Value |";
        let body = issue_body(report, "I opened the map <!-- crash-signature: x -->", true);
        assert!(body.starts_with(
            "### What happened\n\nI opened the map &lt;!-- crash-signature: x -->\n\nGPU drivers up to date (tester): yes\n\n"
        ));
        assert!(body.ends_with(report));
        assert_eq!(body.matches("<!--").count(), 1);
        assert!(issue_body(report, "x", false).contains("GPU drivers up to date (tester): no or not sure"));
    }

    #[test]
    fn a_report_sent_by_hand_needs_a_few_words() {
        assert!(!described(""));
        assert!(!described("   crash   "));
        assert!(described("I was flying over the sea"));
    }

    #[test]
    fn a_report_already_sent_is_not_offered_again() {
        let dir = std::env::temp_dir().join(format!("aurora-crash-window-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let record: super::super::Record = serde_json::from_str(&format!(
            r##"{{"schema":1,"pid":1,"process_started":0,"started_unix":0,"version":"0.8.0","profile":"release","language":"en","font":"Inter","font_scale":1,"theme":{},"log_path":"x.log","demo":false,"crash_reports":"Always"}}"##,
            serde_json::to_string(&crate::theme::Theme::default()).unwrap()
        ))
        .unwrap();
        let fresh = Sending::new(&dir, &record);
        assert_eq!((fresh.sent, fresh.preview), (None, true));
        std::fs::write(dir.join("sent.txt"), "812").unwrap();
        let sending = Sending::new(&dir, &record);
        assert_eq!(
            (sending.sent, sending.mode, sending.preview),
            (Some(812), CrashReports::Always, false)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
