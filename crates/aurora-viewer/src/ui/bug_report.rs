//! Explicit review of the public report, shared by all entry points.

use super::widgets::{Floater, flat_button};
use crate::bug_report::github::{self, Github};
use crate::bug_report::{Field, Report, files};
use crate::theme::Palette;
use egui::{RichText, Vec2};

#[derive(Clone, Copy, Default, PartialEq)]
enum Delivery {
    #[default]
    Local,
    Browser,
    Direct,
}

#[derive(Default)]
pub struct BugReportUi {
    pub open: bool,
    pub requested: bool,
    pub report: Report,
    tab: usize,
    include_ui: bool,
    include_image: bool,
    image: Option<files::Attachment>,
    texture: Option<egui::TextureHandle>,
    diagnostics: Option<crossbeam_channel::Receiver<files::Diagnostics>>,
    image_job: Option<crossbeam_channel::Receiver<Result<Option<files::Attachment>, String>>>,
    save_job: Option<crossbeam_channel::Receiver<std::io::Result<files::Saved>>>,
    pub capture_requested: Option<bool>,
    pub capture_pending: bool,
    saved: Option<files::Saved>,
    error: Option<String>,
    demo: bool,
    delivery: Delivery,
    pending_delivery: Option<(Delivery, Report, bool)>,
    github: Github,
    notice: Option<&'static str>,
}

impl BugReportUi {
    pub fn begin(&mut self, fields: Vec<Field>, demo: bool) {
        self.requested = false;
        self.open = true;
        // Closing the floater preserves the draft and its original context.
        if !self.report.fields.is_empty() {
            return;
        }
        self.report.fields = fields;
        self.demo = demo;
        if demo {
            self.report.title = aurora_i18n::tr!("bug-demo-title").into();
            self.report.steps = aurora_i18n::tr!("bug-demo-steps").into();
            self.report.expected = aurora_i18n::tr!("bug-demo-expected").into();
            self.report.actual = aurora_i18n::tr!("bug-demo-actual").into();
            match std::env::var("AURORA_DEMO_BUG_REPORT").as_deref() {
                Ok("details") => {
                    self.tab = 1;
                    self.report.include_excerpt = true;
                    for field in &mut self.report.fields {
                        if field.heading == "Location" {
                            field.included = false;
                        }
                    }
                }
                Ok(mode @ ("image" | "image-ui")) => {
                    self.tab = 2;
                    self.include_ui = mode == "image-ui";
                    self.capture_requested = Some(self.include_ui);
                }
                Ok("direct") => self.delivery = Delivery::Direct,
                _ => {}
            }
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.diagnostics = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(files::diagnostics(demo));
        });
    }

    pub fn captured(&mut self, width: u32, height: u32, pixels: Vec<u8>) {
        self.capture_pending = false;
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.image_job = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(files::capture(width, height, pixels).map(Some));
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.github.poll();
        if let Some(rx) = &self.diagnostics
            && let Ok(diagnostics) = rx.try_recv()
        {
            self.report.fields.extend(diagnostics.fields);
            self.report.excerpt = diagnostics.excerpt;
            self.diagnostics = None;
        }
        if let Some(rx) = &self.image_job
            && let Ok(result) = rx.try_recv()
        {
            self.image_job = None;
            match result {
                Ok(Some(image)) => {
                    self.texture = Some(ctx.load_texture("bug-report-image", image.preview.clone(), Default::default()));
                    self.image = Some(image);
                    self.include_image = true;
                    self.saved = None;
                }
                Ok(None) => {}
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(rx) = &self.save_job
            && let Ok(result) = rx.try_recv()
        {
            self.save_job = None;
            match result {
                Ok(saved) => {
                    if let Some((delivery, report, with_image)) = self.pending_delivery.take() {
                        if self.demo && delivery != Delivery::Local {
                            self.notice = Some("bug-demo-saved");
                        } else if delivery == Delivery::Browser {
                            let body = report.markdown();
                            let draft = github::browser_draft(&report.title, &body);
                            if draft.paste_body {
                                ctx.copy_text(body);
                            }
                            self.notice = Some(if draft.paste_body {
                                "bug-browser-paste"
                            } else {
                                "bug-browser-opened"
                            });
                            ctx.open_url(egui::OpenUrl::new_tab(draft.url));
                            if with_image {
                                let path = saved.public_dir.clone();
                                std::thread::spawn(move || {
                                    let _ = files::open_directory(&path);
                                });
                            }
                        } else if delivery == Delivery::Direct {
                            self.github.submit(report.title.clone(), report.markdown());
                            self.notice = None;
                        }
                    }
                    self.saved = Some(saved);
                }
                Err(_) => self.error = Some(aurora_i18n::tr!("bug-save-error").into()),
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, icons: &super::icons::Icons, in_world: bool) {
        self.poll(ctx);
        if self.capture_pending {
            return;
        }
        let mut open = self.open;
        let screen = ctx.content_rect();
        let size = Vec2::new(880.0f32.min(screen.width() - 28.0), 660.0f32.min(screen.height() - 36.0));
        let mut floater = Floater::new("bug_report", aurora_i18n::tr!("bug-title"), screen.center() - size * 0.5, size);
        floater.min_size = Vec2::new(390.0, 340.0);
        floater.show(ctx, p, &mut open, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if let Some(icon) = icons.get("bug") {
                    ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(28.0)).tint(p.violet_light));
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("bug-heading")).size(20.0).strong().color(p.ink));
                    ui.label(RichText::new(aurora_i18n::tr!("bug-public-warning")).color(p.amber));
                });
            });
            ui.add_space(8.0);
            ui.label(RichText::new(aurora_i18n::tr!("bug-review-help")).small().color(p.muted));
            if self.demo {
                ui.label(RichText::new(aurora_i18n::tr!("bug-demo-warning")).small().color(p.teal));
            }
            ui.separator();
            let working = self.save_job.is_some() || self.github.sending;
            let height = (ui.available_height() - 155.0).max(130.0);
            ui.add_enabled_ui(!working, |ui| {
                if ui.available_width() >= 730.0 {
                    ui.columns(2, |columns| {
                        egui::ScrollArea::vertical()
                            .id_salt("bug-form")
                            .max_height(height)
                            .auto_shrink([false, false])
                            .show(&mut columns[0], |ui| {
                                self.form(ui, p, in_world);
                            });
                        Self::preview(&self.report, &mut columns[1], p, height);
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("bug-narrow")
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.form(ui, p, in_world);
                            ui.separator();
                            Self::preview(&self.report, ui, p, 200.0);
                        });
                }
            });
            ui.separator();
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).color(p.danger));
            }
            if let Some(error) = self.github.failure {
                ui.label(RichText::new(error.message()).color(p.danger));
            }
            if let Some(notice) = self.notice {
                ui.label(RichText::new(aurora_i18n::text(notice)).small().color(p.teal));
            }
            if let Some(number) = self.github.created {
                ui.hyperlink_to(
                    aurora_i18n::tr!("bug-created", number = number.to_string()),
                    format!("{}/{}", github::ISSUES, number),
                );
            }
            if let Some(saved) = &self.saved {
                ui.label(RichText::new(aurora_i18n::tr!("bug-saved")).color(p.success));
                if !saved.private_log_saved && !self.demo {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("bug-private-log-unavailable"))
                            .small()
                            .color(p.muted),
                    );
                }
            }
            ui.add_enabled_ui(!working, |ui| self.delivery_options(ui, p));
            ui.horizontal_wrapped(|ui| {
                if flat_button(ui, p, aurora_i18n::tr!("bug-copy")).clicked() {
                    ctx.copy_text(self.report.markdown());
                }
                let ready = !working
                    && self.diagnostics.is_none()
                    && self.image_job.is_none()
                    && self.capture_requested.is_none()
                    && self.report.ready();
                if ui.add_enabled(ready, egui::Button::new(aurora_i18n::tr!("bug-save"))).clicked() {
                    self.save(Delivery::Local);
                }
                let delivery_ready = self.delivery == Delivery::Browser || self.github.connected() || self.demo;
                let label = if self.delivery == Delivery::Direct {
                    aurora_i18n::tr!("bug-send-direct")
                } else {
                    aurora_i18n::tr!("bug-open-github")
                };
                if ui
                    .add_enabled(
                        ready && delivery_ready && self.github.created.is_none(),
                        egui::Button::new(RichText::new(label).color(p.ink)).fill(p.violet),
                    )
                    .clicked()
                {
                    self.save(self.delivery);
                }
                if let Some(saved) = &self.saved
                    && flat_button(ui, p, aurora_i18n::tr!("bug-open-folder")).clicked()
                {
                    let path = saved.public_dir.clone();
                    std::thread::spawn(move || {
                        let _ = files::open_directory(&path);
                    });
                }
                if working || self.diagnostics.is_some() {
                    ui.spinner();
                }
                if !working && flat_button(ui, p, aurora_i18n::tr!("bug-new-report")).clicked() {
                    let mut github = std::mem::take(&mut self.github);
                    github.created = None;
                    github.failure = None;
                    *self = Self {
                        github,
                        requested: true,
                        open: true,
                        ..Default::default()
                    };
                }
            });
        });
        if !open {
            self.github.cancel_authorization();
        }
        self.open = open;
    }

    fn form(&mut self, ui: &mut egui::Ui, p: &Palette, in_world: bool) {
        super::widgets::tabs(
            ui,
            p,
            &mut self.tab,
            &[
                (aurora_i18n::tr!("bug-description"), true),
                (aurora_i18n::tr!("bug-diagnostics"), true),
                (aurora_i18n::tr!("bug-picture"), true),
            ],
        );
        ui.add_space(10.0);
        match self.tab {
            0 => {
                ui.label(aurora_i18n::tr!("bug-summary"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.report.title)
                        .char_limit(180)
                        .desired_width(f32::INFINITY)
                        .hint_text(aurora_i18n::tr!("bug-summary-hint")),
                );
                ui.add_space(8.0);
                for (key, value) in [
                    ("bug-steps", &mut self.report.steps),
                    ("bug-expected", &mut self.report.expected),
                    ("bug-actual", &mut self.report.actual),
                ] {
                    ui.label(aurora_i18n::text(key));
                    ui.add(
                        egui::TextEdit::multiline(value)
                            .char_limit(6000)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(8.0);
                }
                ui.label(RichText::new(aurora_i18n::tr!("bug-no-secrets")).small().color(p.muted));
            }
            1 => {
                ui.label(RichText::new(aurora_i18n::tr!("bug-fields-help")).small().color(p.muted));
                for field in &mut self.report.fields {
                    ui.checkbox(&mut field.included, aurora_i18n::text(field.label));
                    ui.indent(field.heading, |ui| {
                        ui.add(egui::Label::new(RichText::new(&field.value).small().color(p.muted)).wrap());
                    });
                    ui.add_space(4.0);
                }
                ui.separator();
                ui.add_enabled_ui(!self.report.excerpt.is_empty(), |ui| {
                    ui.checkbox(&mut self.report.include_excerpt, aurora_i18n::tr!("bug-log"));
                });
                ui.label(RichText::new(aurora_i18n::tr!("bug-log-help")).small().color(p.muted));
                if !self.report.excerpt.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(&self.report.excerpt).monospace().small())
                            .wrap()
                            .selectable(true),
                    );
                }
            }
            _ => {
                ui.label(RichText::new(aurora_i18n::tr!("bug-picture-warning")).color(p.amber));
                ui.add_space(8.0);
                let busy = self.image_job.is_some() || self.capture_requested.is_some() || self.capture_pending;
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(in_world, egui::Button::new(aurora_i18n::tr!("bug-take-picture")))
                            .on_disabled_hover_text(aurora_i18n::tr!("bug-picture-login"))
                            .clicked()
                        {
                            self.capture_requested = Some(self.include_ui);
                        }
                        if flat_button(ui, p, aurora_i18n::tr!("bug-attach-image")).clicked() {
                            let (tx, rx) = crossbeam_channel::bounded(1);
                            self.image_job = Some(rx);
                            std::thread::spawn(move || {
                                let _ = tx.send(files::choose_image());
                            });
                        }
                    });
                    ui.checkbox(&mut self.include_ui, aurora_i18n::tr!("bug-with-ui"));
                });
                if busy {
                    ui.spinner();
                }
                if let (Some(texture), Some(image)) = (&self.texture, &self.image) {
                    ui.add_space(8.0);
                    ui.add(
                        egui::Image::new(texture)
                            .max_width(ui.available_width())
                            .max_height(230.0)
                            .maintain_aspect_ratio(true),
                    );
                    ui.label(
                        RichText::new(format!("{} × {} · PNG", image.width, image.height))
                            .small()
                            .color(p.muted),
                    );
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.include_image, aurora_i18n::tr!("bug-include-image"));
                        if flat_button(ui, p, aurora_i18n::tr!("bug-remove-image")).clicked() {
                            self.image = None;
                            self.texture = None;
                            self.include_image = false;
                        }
                    });
                }
            }
        }
    }

    fn preview(report: &Report, ui: &mut egui::Ui, p: &Palette, height: f32) {
        ui.label(RichText::new(aurora_i18n::tr!("bug-preview")).strong().color(p.violet_light));
        ui.label(RichText::new(&report.title).strong().color(p.ink));
        egui::ScrollArea::vertical()
            .id_salt("bug-preview-text")
            .max_height(height - 50.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(report.markdown()).monospace().size(11.5).color(p.ink))
                        .wrap()
                        .selectable(true),
                );
            });
    }

    fn delivery_options(&mut self, ui: &mut egui::Ui, p: &Palette) {
        // No preference/authorization is persisted; browser is the default.
        if self.delivery == Delivery::Local {
            self.delivery = Delivery::Browser;
        }
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.delivery, Delivery::Browser, aurora_i18n::tr!("bug-browser-mode"));
            ui.selectable_value(&mut self.delivery, Delivery::Direct, aurora_i18n::tr!("bug-direct-mode"));
        });
        if self.include_image && self.image.is_some() {
            ui.label(RichText::new(aurora_i18n::tr!("bug-manual-image")).small().color(p.amber));
        }
        if self.delivery != Delivery::Direct {
            return;
        }
        if self.demo {
            return;
        }
        if self.github.connected() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("bug-connected")).small().color(p.teal));
                if flat_button(ui, p, aurora_i18n::tr!("bug-disconnect")).clicked() {
                    self.github.disconnect();
                }
            });
        } else if let Some(code) = self.github.user_code.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.label(aurora_i18n::tr!("bug-device-instruction"));
                ui.monospace(&code);
                if flat_button(ui, p, aurora_i18n::tr!("bug-copy-code")).clicked() {
                    ui.ctx().copy_text(code);
                }
                ui.hyperlink_to(aurora_i18n::tr!("bug-authorize-browser"), github::VERIFY);
                if flat_button(ui, p, aurora_i18n::tr!("bug-cancel-auth")).clicked() {
                    self.github.cancel_authorization();
                }
            });
        } else if self.github.authorizing() {
            ui.spinner();
            if flat_button(ui, p, aurora_i18n::tr!("bug-cancel-auth")).clicked() {
                self.github.cancel_authorization();
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!github::CLIENT_ID.is_empty(), egui::Button::new(aurora_i18n::tr!("bug-connect")))
                    .clicked()
                {
                    self.github.authorize();
                }
                ui.label(
                    RichText::new(aurora_i18n::text(if github::CLIENT_ID.is_empty() {
                        "bug-github-config"
                    } else {
                        "bug-account-needed"
                    }))
                    .small()
                    .color(p.muted),
                );
            });
        }
    }

    fn save(&mut self, delivery: Delivery) {
        self.error = None;
        let report = self.report.clone();
        let image = self.image.clone();
        let include_image = self.include_image;
        self.pending_delivery = Some((delivery, report.clone(), include_image && image.is_some()));
        self.notice = None;
        let demo = self.demo;
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.save_job = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(files::save(&report, image.as_ref(), include_image, demo));
        });
    }
}
