//! "Report Abuse" window, floater_report_abuse.xml in the Aurora theme: the
//! screenshot, who reports from where, the object picked in the world, the
//! category, the abuser, the place, the summary and the details. The rules
//! are in `crate::report_abuse`; the app takes the screenshot, the picks and
//! the sending (`app::report_abuse`).

use super::avatar_picker::AvatarPicker;
use super::widgets::{self, ActionKind, Floater, action_button, flat_button, primary_button_sized};
use crate::report_abuse::{self as rules, Form, Notice};
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Vec2};
use glam::Vec3;
use std::time::Instant;
use uuid::Uuid;

const WIDTH: f32 = 420.0;
const SHOT_HEIGHT: f32 = 150.0;
const LABEL: f32 = 12.0;

/// The screenshot, shown and ready to upload.
pub struct Screenshot {
    pub texture: egui::TextureHandle,
    /// JPEG 2000 texture (None: the encoding failed; the report goes without).
    pub j2c: Option<Vec<u8>>,
}

/// What the window asks the app for, this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ReportAction {
    /// Arm the object picker (the next click in the world designates).
    Pick,
    CancelPick,
    /// refresh_screenshot (FIRE-15218): a new screenshot without the window.
    RefreshScreenshot,
    /// The form passed validateReport and the copyright check.
    Send,
}

#[derive(Default)]
pub struct ReportAbuseUi {
    pub open: bool,
    raise: bool,
    pub form: Form,
    /// The details the window opened with (mDefaultSummary).
    pub default_details: String,
    pub categories: Vec<(i32, String)>,
    pub region: String,
    /// Region-local position of the report (setPosBox).
    pub position: Vec3,
    pub object_id: Uuid,
    pub abuser_id: Uuid,
    /// Waiting for the ObjectPropertiesFamily of this object.
    pub awaiting_object: Option<Uuid>,
    /// The object picker is armed.
    pub picking: bool,
    pub screenshot: Option<Screenshot>,
    /// When to take the screenshot (AbuseReportScreenshotDelay after
    /// opening or after "Refresh").
    pub capture_at: Option<Instant>,
    /// The screenshot is being taken: the window is not drawn this frame.
    pub capture_pending: bool,
    /// Being scaled and encoded in the background.
    pub shot_job: Option<crossbeam_channel::Receiver<Option<(egui::ColorImage, Option<Vec<u8>>)>>>,
    /// mCopyrightWarningSeen.
    pub copyright_seen: bool,
    /// The report left; waiting for its answer.
    pub sending: bool,
    /// An alert over the window (validation, copyright, failure).
    pub notice: Option<String>,
    /// HelpReportAbuseConfirm, shown after the window closed.
    pub thanks: Option<String>,
    picker: AvatarPicker,
}

impl ReportAbuseUi {
    /// postBuild / onOpen: a blank report from where we stand, the
    /// screenshot shortly after. `location` is our SLURL.
    pub fn begin(&mut self, region: String, position: Vec3, location: String) {
        let categories = std::mem::take(&mut self.categories);
        *self = ReportAbuseUi {
            open: true,
            raise: true,
            region,
            position,
            categories: if categories.is_empty() {
                rules::default_categories()
            } else {
                categories
            },
            capture_at: Some(Instant::now() + rules::SCREENSHOT_DELAY),
            thanks: self.thanks.take(),
            ..Default::default()
        };
        self.form.location = location;
        rules::clamp_bytes(&mut self.form.location, rules::LOCATION_MAX_BYTES);
    }

    /// setFromAvatarID: the avatar is both the object and the abuser.
    pub fn set_avatar(&mut self, id: Uuid, name: &str) {
        self.object_id = id;
        self.abuser_id = id;
        self.awaiting_object = None;
        self.form.object_name = name.to_owned();
        self.form.owner_name = name.to_owned();
        self.form.abuser_name = name.to_owned();
    }

    /// setPickedObjectProperties: the object's name, its owner as abuser.
    pub fn set_object_properties(&mut self, name: &str, owner: Uuid, owner_name: &str) {
        self.awaiting_object = None;
        self.form.object_name = name.to_owned();
        self.form.owner_name = owner_name.to_owned();
        self.form.abuser_name = owner_name.to_owned();
        self.abuser_id = owner;
    }

    /// The categories of the region, in its language: the choice stays on
    /// the same value when the new list has it.
    pub fn set_categories(&mut self, list: Vec<(i32, String)>) {
        if !list.iter().any(|(value, _)| *value == self.form.category) {
            self.form.category = 0;
            self.form.category_label.clear();
        } else if let Some((_, label)) = list.iter().find(|(value, _)| *value == self.form.category) {
            self.form.category_label.clone_from(label);
        }
        self.categories = list;
    }

    /// Names known later than the window opened (avatar name cache).
    pub fn refresh_names(&mut self, world: &mut World) {
        let unknown = |s: &str| s.is_empty() || s == "…";
        if !self.abuser_id.is_nil() && unknown(&self.form.abuser_name) {
            world.social.want_name(self.abuser_id);
            let name = world.social.name_of(&self.abuser_id);
            if !unknown(&name) {
                if self.object_id == self.abuser_id {
                    self.form.object_name.clone_from(&name);
                    self.form.owner_name.clone_from(&name);
                } else if unknown(&self.form.owner_name) {
                    self.form.owner_name = world.legacy_name(&self.abuser_id).unwrap_or_else(|| name.clone());
                }
                self.form.abuser_name = name;
            }
        }
    }

    /// `grid`: the grid's name in the notices ([CURRENT_GRID]).
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World, grid: &str) -> Vec<ReportAction> {
        let mut actions = Vec::new();
        self.show_notices(ctx, p);
        if !self.open {
            return actions;
        }
        // the screenshot leaves the window out (takeNewSnapshot hides it)
        if self.capture_pending {
            ctx.request_repaint();
            return actions;
        }
        if std::mem::take(&mut self.raise) {
            super::widgets::expand_floater(ctx, "report_abuse");
        }
        if let Some(chosen) = self.picker.show(ctx, p, world)
            && let Some(id) = chosen.first().copied()
        {
            // callbackAvatarID
            self.abuser_id = id;
            self.form.abuser_name = world.social.name_of(&id);
        }
        if self.picking && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            actions.push(ReportAction::CancelPick);
        }
        let screen = ctx.content_rect();
        let mut open = self.open;
        Floater::new(
            "report_abuse",
            aurora_i18n::tr!("report-abuse-title"),
            egui::pos2(screen.center().x - WIDTH * 0.5, screen.top() + 40.0),
            Vec2::new(WIDTH, 700.0),
        )
        .auto_height()
        .show(ctx, p, &mut open, |ui| {
            ui.set_width(WIDTH - 24.0);
            self.screenshot_row(ui, p, &mut actions);
            ui.add_space(6.0);
            self.place_rows(ui, p, world);
            ui.add_space(4.0);
            self.object_rows(ui, p, &mut actions);
            ui.add_space(6.0);
            self.form_rows(ui, p);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let send = ui
                    .add_enabled_ui(!self.sending && self.capture_at.is_none() && self.shot_job.is_none(), |ui| {
                        primary_button_sized(ui, p, aurora_i18n::tr!("report-abuse-send"), Vec2::new(150.0, 28.0))
                    })
                    .inner;
                if send.clicked() {
                    self.on_send(grid, &mut actions);
                }
                if action_button(
                    ui,
                    p,
                    "x",
                    aurora_i18n::tr!("common-cancel"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 28.0),
                )
                .clicked()
                {
                    // onClickCancel
                    self.open = false;
                }
                if self.sending {
                    ui.spinner();
                    ui.label(RichText::new(aurora_i18n::tr!("report-abuse-sending")).size(LABEL).color(p.muted));
                }
            });
        });
        if !open || !self.open {
            self.close(&mut actions);
        }
        actions
    }

    fn close(&mut self, actions: &mut Vec<ReportAction>) {
        if self.picking {
            actions.push(ReportAction::CancelPick);
        }
        self.open = false;
        self.picker.open = false;
        // onClickCancel / onClose reset the warning
        self.copyright_seen = false;
    }

    fn screenshot_row(&mut self, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<ReportAction>) {
        let size = Vec2::new(ui.available_width(), SHOT_HEIGHT);
        let label = if self.capture_at.is_some() || self.shot_job.is_some() {
            aurora_i18n::tr!("report-abuse-screenshot")
        } else {
            aurora_i18n::tr!("report-abuse-no-screenshot")
        };
        let picture = widgets::picture(ui, p, self.screenshot.as_ref().map(|s| &s.texture), size, "camera-filled", label);
        // refresh_screenshot, in the picture's corner
        let corner = egui::Rect::from_min_size(picture.rect.right_bottom() - Vec2::new(28.0, 28.0), Vec2::splat(20.0));
        let busy = self.capture_at.is_some() || self.shot_job.is_some() || self.capture_pending;
        let refresh = ui
            .scope_builder(egui::UiBuilder::new().max_rect(corner), |ui| {
                widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("report-abuse-refresh-screenshot"), !busy)
            })
            .inner;
        if refresh {
            actions.push(ReportAction::RefreshScreenshot);
        }
    }

    fn place_rows(&self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        egui::Grid::new("report_abuse_place")
            .num_columns(2)
            .spacing([10.0, 1.0])
            .show(ui, |ui| {
                let row = |ui: &mut egui::Ui, label: &str, value: String| {
                    ui.label(RichText::new(label).size(LABEL).strong().color(p.ink));
                    ui.label(RichText::new(value).size(LABEL).color(p.muted));
                    ui.end_row();
                };
                row(ui, aurora_i18n::tr!("report-abuse-reporter"), world.own_name());
                row(ui, aurora_i18n::tr!("report-abuse-region"), self.region.clone());
                row(ui, aurora_i18n::tr!("report-abuse-position"), rules::position_text(self.position));
            });
    }

    fn object_rows(&mut self, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<ReportAction>) {
        let hint = if self.picking {
            aurora_i18n::tr!("report-abuse-picking")
        } else {
            aurora_i18n::tr!("report-abuse-pick-hint")
        };
        ui.label(RichText::new(hint).size(LABEL).color(if self.picking { p.violet } else { p.ink }));
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if widgets::icon_toggle(ui, p, "color-picker", aurora_i18n::tr!("report-abuse-pick-tooltip"), self.picking) {
                actions.push(if self.picking {
                    ReportAction::CancelPick
                } else {
                    ReportAction::Pick
                });
            }
            ui.add_space(6.0);
            egui::Grid::new("report_abuse_object")
                .num_columns(2)
                .spacing([10.0, 1.0])
                .show(ui, |ui| {
                    let waiting = self.awaiting_object.is_some();
                    for (label, value) in [
                        (aurora_i18n::tr!("report-abuse-object"), &self.form.object_name),
                        (aurora_i18n::tr!("report-abuse-owner"), &self.form.owner_name),
                    ] {
                        ui.label(RichText::new(label).size(LABEL).strong().color(p.ink));
                        if waiting && value.is_empty() {
                            ui.spinner();
                        } else {
                            ui.add(egui::Label::new(RichText::new(value.as_str()).size(LABEL).color(p.muted)).truncate());
                        }
                        ui.end_row();
                    }
                });
        });
    }

    fn form_rows(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let heading = |ui: &mut egui::Ui, text: &str| {
            ui.add_space(2.0);
            ui.label(RichText::new(text).size(LABEL).strong().color(p.ink));
        };
        let width = ui.available_width();
        let selected = self
            .categories
            .iter()
            .find(|(value, _)| *value == self.form.category)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| aurora_i18n::tr!("report-abuse-select-category").to_owned());
        egui::ComboBox::from_id_salt("report_abuse_category")
            .selected_text(selected)
            .width(width)
            .height(320.0)
            .show_ui(ui, |ui| {
                for (value, label) in &self.categories {
                    if ui.selectable_label(self.form.category == *value, label).clicked() {
                        self.form.category = *value;
                        self.form.category_label.clone_from(label);
                    }
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("report-abuse-category-tooltip"));
        heading(ui, aurora_i18n::tr!("report-abuse-abuser-name"));
        ui.horizontal(|ui| {
            // abuser_name_edit is disabled: filled by the pick or "Choose"
            let mut name = self.form.abuser_name.clone();
            ui.add_enabled(false, egui::TextEdit::singleline(&mut name).desired_width(width - 120.0));
            if flat_button(ui, p, aurora_i18n::tr!("report-abuse-choose"))
                .on_hover_text(aurora_i18n::tr!("report-abuse-choose-tooltip"))
                .clicked()
            {
                self.picker.open("report_abuse_picker", false);
            }
        });
        heading(ui, aurora_i18n::tr!("report-abuse-location"));
        if ui
            .add(egui::TextEdit::singleline(&mut self.form.location).desired_width(width))
            .changed()
        {
            rules::clamp_bytes(&mut self.form.location, rules::LOCATION_MAX_BYTES);
        }
        heading(ui, aurora_i18n::tr!("report-abuse-summary"));
        if ui
            .add(egui::TextEdit::singleline(&mut self.form.summary).desired_width(width))
            .changed()
        {
            rules::clamp_bytes(&mut self.form.summary, rules::SUMMARY_MAX_BYTES);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("report-abuse-details"))
                    .size(LABEL)
                    .strong()
                    .color(p.ink),
            );
            ui.label(
                RichText::new(aurora_i18n::tr!("report-abuse-details-hint"))
                    .size(LABEL)
                    .color(p.muted),
            );
        });
        if ui
            .add(
                egui::TextEdit::multiline(&mut self.form.details)
                    .desired_width(width)
                    .desired_rows(4),
            )
            .changed()
        {
            rules::clamp_chars(&mut self.form.details, rules::DETAILS_MAX_CHARS);
        }
        ui.add_space(2.0);
        ui.label(RichText::new(aurora_i18n::tr!("report-abuse-incomplete")).size(11.0).color(p.muted));
    }

    /// A click on "Report Abuse" (demo scripts).
    pub fn press_send(&mut self, grid: &str) -> Vec<ReportAction> {
        let mut actions = Vec::new();
        self.on_send(grid, &mut actions);
        actions
    }

    /// onClickSend: the picker closes, then validateReport and the copyright
    /// check; the app gathers and sends what passes.
    fn on_send(&mut self, grid: &str, actions: &mut Vec<ReportAction>) {
        if self.picking {
            actions.push(ReportAction::CancelPick);
        }
        if let Some(notice) = rules::validate(&self.form, &self.default_details) {
            self.notice = Some(notice.text(grid));
            return;
        }
        if rules::copyright_notice(&self.form, &mut self.copyright_seen) {
            self.notice = Some(Notice::Copyright.text(grid));
            return;
        }
        actions.push(ReportAction::Send);
    }

    /// The alerts, over everything (alertmodal).
    fn show_notices(&mut self, ctx: &egui::Context, p: &Palette) {
        for (slot, id) in [(&mut self.notice, "report_abuse_notice"), (&mut self.thanks, "report_abuse_thanks")] {
            let Some(text) = slot.as_deref() else {
                continue;
            };
            let mut ok = false;
            let modal = widgets::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("report-abuse-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
                });
                ui.add_space(10.0);
                ok = primary_button_sized(ui, p, aurora_i18n::tr!("common-ok"), Vec2::new(90.0, 26.0)).clicked();
            });
            if ok || modal.should_close() {
                *slot = None;
            }
        }
    }
}
