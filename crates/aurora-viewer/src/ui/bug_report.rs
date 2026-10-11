//! Public report form, shared by all entry points.

use super::widgets::{Floater, flat_button};
use crate::bug_report::github::{self, Github};
use crate::bug_report::{Field, Report, files};
use crate::theme::Palette;
use egui::{RichText, Vec2};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Default, PartialEq)]
enum Delivery {
    Local,
    Browser,
    #[default]
    Direct,
}

#[derive(Clone, Copy, PartialEq)]
enum ImageOrigin {
    Capture(bool),
    File,
}

struct ImageJob {
    rx: crossbeam_channel::Receiver<Result<Option<files::Attachment>, &'static str>>,
    origin: ImageOrigin,
    include: bool,
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
    image_job: Option<ImageJob>,
    image_origin: Option<ImageOrigin>,
    save_job: Option<crossbeam_channel::Receiver<std::io::Result<files::Saved>>>,
    pub capture_requested: Option<bool>,
    pub capture_pending: bool,
    saved: Option<files::Saved>,
    error: Option<&'static str>,
    demo: bool,
    delivery: Delivery,
    pending_delivery: Option<(Delivery, Report, bool)>,
    github: Github,
    notice: Option<&'static str>,
    draft_active: bool,
    draft_id: uuid::Uuid,
    browser_opened: bool,
    demo_text: Option<[String; 4]>,
    language: &'static str,
}

impl BugReportUi {
    pub fn begin(&mut self, fields: Vec<Field>, demo: bool) {
        // Keep the sent report visible for its issue link and picture attachment,
        // then start a fresh draft when the user opens the form again.
        if self.github.created.is_some() {
            self.new_report();
        }
        self.requested = false;
        self.open = true;
        // Closing the floater preserves the draft and its original context.
        if self.draft_active {
            return;
        }
        self.draft_active = true;
        self.draft_id = uuid::Uuid::new_v4();
        self.tab = 0;
        self.report.fields = fields;
        self.language = aurora_i18n::language();
        self.demo = demo;
        if demo {
            self.report.title = aurora_i18n::tr!("bug-demo-title").into();
            self.report.steps = aurora_i18n::tr!("bug-demo-steps").into();
            self.report.expected = aurora_i18n::tr!("bug-demo-expected").into();
            self.report.actual = aurora_i18n::tr!("bug-demo-actual").into();
            self.demo_text = Some([
                self.report.title.clone(),
                self.report.steps.clone(),
                self.report.expected.clone(),
                self.report.actual.clone(),
            ]);
            match std::env::var("AURORA_DEMO_BUG_REPORT").as_deref() {
                Ok("details") => {
                    self.tab = 2;
                    self.report.include_excerpt = true;
                    for field in &mut self.report.fields {
                        if field.heading == "Location" {
                            field.included = false;
                        }
                    }
                }
                Ok(mode @ ("image" | "image-ui" | "refresh")) => {
                    self.tab = 1;
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
        self.image_job = Some(ImageJob {
            rx,
            origin: ImageOrigin::Capture(self.include_ui),
            include: if matches!(self.image_origin, Some(ImageOrigin::Capture(_))) {
                self.include_image
            } else {
                true
            },
        });
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
        if let Some(job) = &self.image_job
            && let Ok(result) = job.rx.try_recv()
        {
            let (origin, include) = (job.origin, job.include);
            self.image_job = None;
            match result {
                Ok(Some(image)) => {
                    self.texture = Some(ctx.load_texture("bug-report-image", image.preview.clone(), Default::default()));
                    self.image = Some(image);
                    self.include_image = include;
                    self.image_origin = Some(origin);
                    self.error = None;
                    self.saved = None;
                }
                Ok(None) => {}
                Err(error) => {
                    self.error = Some(error);
                    if let Some(ImageOrigin::Capture(with_ui)) = self.image_origin {
                        self.include_ui = with_ui;
                    }
                }
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
                            // Opening a draft URL is not confirmation of publication.
                            self.browser_opened = true;
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
                Err(_) => self.error = Some("bug-save-error"),
            }
        }
    }

    fn refresh_language(&mut self, language: &'static str) {
        if self.language == language {
            return;
        }
        self.language = language;
        // Only untouched demo prose is translated. Never overwrite the user's draft.
        if let Some(previous) = &mut self.demo_text {
            for ((value, old), key) in [
                &mut self.report.title,
                &mut self.report.steps,
                &mut self.report.expected,
                &mut self.report.actual,
            ]
            .into_iter()
            .zip(previous.iter_mut())
            .zip(["bug-demo-title", "bug-demo-steps", "bug-demo-expected", "bug-demo-actual"])
            {
                let translated = aurora_i18n::text_in(language, key);
                if value == old {
                    *value = translated.into();
                }
                *old = translated.into();
            }
        }
        for field in &mut self.report.fields {
            if field.label == "bug-language" {
                field.value = language.into();
            }
        }
    }

    pub fn demo_step(&mut self, frame: u64) {
        if !self.demo || !matches!(frame, 700 | 900) {
            return;
        }
        match std::env::var("AURORA_DEMO_BUG_REPORT").as_deref() {
            Ok("refresh") => {
                self.include_ui = frame == 700;
                self.capture_option_changed(true);
            }
            Ok("language") => {
                aurora_i18n::select(if frame == 700 { "en" } else { "fr" }, None);
            }
            _ => {}
        }
    }

    fn image_busy(&self) -> bool {
        self.image_job.is_some() || self.capture_requested.is_some() || self.capture_pending
    }

    fn capture_option_changed(&mut self, in_world: bool) {
        // Imported pictures must never be replaced by the camera.
        if in_world && !self.image_busy() && matches!(self.image_origin, Some(ImageOrigin::Capture(_))) {
            self.capture_requested = Some(self.include_ui);
        }
    }

    fn new_report(&mut self) {
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

    fn confirm_browser_publication(&mut self) {
        if self.browser_opened {
            self.new_report();
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, icons: &super::icons::Icons, in_world: bool) {
        self.refresh_language(aurora_i18n::language());
        self.poll(ctx);
        if self.capture_pending {
            return;
        }
        let mut open = self.open;
        let screen = ctx.content_rect();
        let available = (screen.size() - Vec2::new(28.0, 36.0)).max(Vec2::splat(1.0));
        let size = Vec2::new(800.0, 860.0).min(available);
        let mut floater = Floater::new("bug_report", aurora_i18n::tr!("bug-title"), screen.center() - size * 0.5, size);
        floater.min_size = Vec2::new(700.0, 840.0).min(available);
        floater.show(ctx, p, &mut open, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            // Also let the whole page scroll in short windows: sending controls
            // and status messages must remain reachable when they wrap.
            let page_height = ui.available_height();
            egui::ScrollArea::vertical()
                .id_salt(("bug-page", self.draft_id))
                .max_height(page_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if let Some(icon) = icons.get("bug") {
                            ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(28.0)).tint(p.violet_light));
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(aurora_i18n::tr!("bug-heading")).size(20.0).color(p.ink));
                            ui.label(RichText::new(aurora_i18n::tr!("bug-public-warning")).color(p.amber));
                        });
                    });
                    ui.add_space(6.0);
                    ui.label(aurora_i18n::tr!("bug-review-help"));
                    if self.demo {
                        ui.label(RichText::new(aurora_i18n::tr!("bug-demo-warning")).color(p.teal));
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    let working = self.save_job.is_some() || self.github.sending;
                    let image_note = if self.include_image && self.image.is_some() { 40.0 } else { 0.0 };
                    let auth_controls = if self.delivery == Delivery::Direct && !self.demo {
                        65.0
                    } else {
                        0.0
                    };
                    let height = (page_height - 315.0 - image_note - auth_controls).clamp(160.0, 440.0);
                    ui.add_enabled_ui(!working && self.github.created.is_none(), |ui| {
                        self.form_panel(ui, p, in_world, height);
                    });
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(RichText::new(aurora_i18n::tr!("bug-send-heading")).strong().color(p.violet_light));
                    ui.add_enabled_ui(!working && self.github.created.is_none(), |ui| self.delivery_options(ui, p));
                    if self.include_image && self.image.is_some() {
                        ui.label(RichText::new(aurora_i18n::tr!("bug-manual-image")).color(p.amber));
                    }
                    ui.add_space(6.0);
                    if !self.report.ready() {
                        ui.label(RichText::new(aurora_i18n::tr!("bug-required-help")).color(p.muted));
                    }
                    let idle = !working && self.diagnostics.is_none() && !self.image_busy();
                    ui.horizontal_wrapped(|ui| {
                        let label = if self.delivery == Delivery::Direct {
                            aurora_i18n::tr!("bug-send-direct")
                        } else {
                            aurora_i18n::tr!("bug-open-github")
                        };
                        let delivery_ready = self.delivery == Delivery::Browser || self.github.connected() || self.demo;
                        if ui
                            .add_enabled(
                                idle && self.report.ready() && delivery_ready && self.github.created.is_none(),
                                egui::Button::new(RichText::new(label).color(p.ink))
                                    .fill(p.violet)
                                    .min_size(Vec2::new(180.0, 30.0)),
                            )
                            .clicked()
                        {
                            self.save(self.delivery);
                        }
                        ui.menu_button(aurora_i18n::tr!("bug-other-actions"), |ui| {
                            if ui.button(aurora_i18n::tr!("bug-copy")).clicked() {
                                ctx.copy_text(self.report.markdown());
                                ui.close();
                            }
                            if ui.add_enabled(idle, egui::Button::new(aurora_i18n::tr!("bug-save"))).clicked() {
                                self.save(Delivery::Local);
                                ui.close();
                            }
                            if let Some(saved) = &self.saved
                                && ui.button(aurora_i18n::tr!("bug-open-folder")).clicked()
                            {
                                let path = saved.public_dir.clone();
                                std::thread::spawn(move || {
                                    let _ = files::open_directory(&path);
                                });
                                ui.close();
                            }
                            if self.github.created.is_some() {
                                ui.separator();
                                if ui
                                    .add_enabled(idle, egui::Button::new(aurora_i18n::tr!("bug-new-report")))
                                    .clicked()
                                {
                                    self.new_report();
                                    ui.close();
                                }
                            }
                        });
                        if working || self.diagnostics.is_some() {
                            ui.spinner();
                        }
                    });
                    if let Some(key) = self.error {
                        ui.label(RichText::new(aurora_i18n::text(key)).color(p.danger));
                    }
                    if let Some(error) = self.github.failure {
                        ui.label(RichText::new(error.message()).color(p.danger));
                    }
                    if let Some(notice) = self.notice {
                        ui.label(RichText::new(aurora_i18n::text(notice)).color(p.teal));
                    }
                    if self.browser_opened && self.github.created.is_none() {
                        ui.label(RichText::new(aurora_i18n::tr!("bug-browser-confirm-help")).color(p.muted));
                        if ui
                            .add_enabled(idle, egui::Button::new(aurora_i18n::tr!("bug-browser-confirm")))
                            .clicked()
                        {
                            self.confirm_browser_publication();
                        }
                    }
                    if let Some(number) = self.github.created {
                        ui.hyperlink_to(
                            aurora_i18n::tr!("bug-created", number = number.to_string()),
                            format!("{}/{number}", github::ISSUES),
                        );
                    }
                    if let Some(saved) = &self.saved {
                        ui.label(RichText::new(aurora_i18n::tr!("bug-saved")).color(p.success));
                        if !saved.private_log_saved && !self.demo {
                            ui.label(RichText::new(aurora_i18n::tr!("bug-private-log-unavailable")).color(p.muted));
                        }
                    }
                });
        });
        self.open = open;
    }

    fn form_panel(&mut self, ui: &mut egui::Ui, p: &Palette, in_world: bool, height: f32) {
        ui.label(
            RichText::new(aurora_i18n::tr!("bug-compose-heading"))
                .strong()
                .color(p.violet_light),
        );
        // Keep navigation outside the form's scroll area.
        super::widgets::tabs(
            ui,
            p,
            &mut self.tab,
            &[
                (aurora_i18n::tr!("bug-description"), true),
                (aurora_i18n::tr!("bug-picture"), true),
                (aurora_i18n::tr!("bug-diagnostics"), true),
            ],
        );
        if self.tab == 0 {
            ui.add_space(8.0);
            self.form(ui, p, in_world, height - 50.0);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("bug-form", self.draft_id, self.tab))
            .max_height(height - 50.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(8.0);
                self.form(ui, p, in_world, height - 50.0);
            });
    }

    fn form(&mut self, ui: &mut egui::Ui, p: &Palette, in_world: bool, height: f32) {
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
                    // Long prose scrolls inside its own editor, so all three
                    // questions remain visible instead of growing the form.
                    let editor_height = (3.0 * ui.text_style_height(&egui::TextStyle::Body) + 8.0).max(64.0);
                    egui::ScrollArea::vertical()
                        .id_salt(("bug-description", self.draft_id, key))
                        .max_height(editor_height)
                        .min_scrolled_height(editor_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(value)
                                    .char_limit(6000)
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    ui.add_space(8.0);
                }
                ui.label(RichText::new(aurora_i18n::tr!("bug-no-secrets")).size(12.0).color(p.muted));
            }
            2 => {
                ui.label(RichText::new(aurora_i18n::tr!("bug-fields-help")).size(12.0).color(p.muted));
                for field in &mut self.report.fields {
                    ui.checkbox(&mut field.included, aurora_i18n::text(field.label));
                    ui.indent(field.heading, |ui| {
                        ui.add(egui::Label::new(RichText::new(field.display_value()).color(p.muted)).wrap());
                    });
                    ui.add_space(4.0);
                }
                ui.separator();
                ui.add_enabled_ui(!self.report.excerpt.is_empty(), |ui| {
                    ui.checkbox(&mut self.report.include_excerpt, aurora_i18n::tr!("bug-log"));
                });
                ui.label(RichText::new(aurora_i18n::tr!("bug-log-help")).size(12.0).color(p.muted));
                if !self.report.excerpt.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(crate::bug_report::privacy::display_excerpt(&self.report.excerpt)))
                            .wrap()
                            .selectable(true),
                    );
                }
            }
            _ => {
                ui.label(RichText::new(aurora_i18n::tr!("bug-picture-warning")).color(p.amber));
                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    columns[0].with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        let busy = self.image_busy();
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
                                    self.image_job = Some(ImageJob {
                                        rx,
                                        origin: ImageOrigin::File,
                                        include: true,
                                    });
                                    std::thread::spawn(move || {
                                        let _ = tx.send(files::choose_image());
                                    });
                                }
                            });
                            if ui.checkbox(&mut self.include_ui, aurora_i18n::tr!("bug-with-ui")).changed() {
                                self.capture_option_changed(in_world);
                            }
                        });
                        if busy {
                            ui.spinner();
                        }
                        if self.image.is_some() {
                            ui.add_enabled_ui(!busy, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.checkbox(&mut self.include_image, aurora_i18n::tr!("bug-include-image"));
                                    if flat_button(ui, p, aurora_i18n::tr!("bug-remove-image")).clicked() {
                                        self.image = None;
                                        self.texture = None;
                                        self.include_image = false;
                                        self.image_origin = None;
                                    }
                                })
                            });
                        }
                    });
                    columns[1].with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        if let (Some(texture), Some(image)) = (&self.texture, &self.image) {
                            ui.add(
                                egui::Image::new(texture)
                                    .max_width(ui.available_width())
                                    .max_height((height - 55.0).clamp(120.0, 320.0))
                                    .maintain_aspect_ratio(true),
                            );
                            ui.label(
                                RichText::new(format!("{} × {} · PNG", image.width, image.height))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        } else {
                            ui.label(RichText::new(aurora_i18n::tr!("bug-image-empty")).color(p.muted));
                        }
                    });
                });
            }
        }
    }

    fn delivery_options(&mut self, ui: &mut egui::Ui, p: &Palette) {
        // Only the current draft remembers the chosen delivery mode.
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.delivery, Delivery::Direct, aurora_i18n::tr!("bug-direct-mode"));
            ui.radio_value(&mut self.delivery, Delivery::Browser, aurora_i18n::tr!("bug-browser-mode"));
        });
        ui.label(
            RichText::new(aurora_i18n::text(if self.delivery == Delivery::Browser {
                "bug-browser-help"
            } else {
                "bug-direct-help"
            }))
            .color(p.muted),
        );
        if self.delivery != Delivery::Direct {
            return;
        }
        if self.demo {
            return;
        }
        if self.github.connected() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("bug-connected")).size(12.0).color(p.teal));
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
                    .size(12.0)
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
