//! The outfit's auto portrait window and the square photo mode's banner
//! and helpers (Fider #447; the rules are `snapshot::portrait`, the app's
//! side `app::outfit_portrait`). The window: the live preview, square, with
//! its thirds and corners, the choices (framing, pose, angle, backdrop,
//! light), then Take, the photo to confirm, Use or Retake. An outfit not
//! worn is offered to be worn first; once the photo is over, the previous
//! outfit is offered back. Nothing is worn or taken off without a click.

use super::super::{icons, widgets};
use crate::snapshot::portrait::{Angle, Backdrop, Framing, Lighting, PortraitSettings, Pose};
use crate::theme::Palette;
use crate::ui::fonts;
use egui::{Align2, Rect, RichText, Sense, Vec2};

/// The auto portrait's window.
pub const WINDOW: &str = "outfit_portrait";
/// The previous outfit's question, once the photo is over.
pub const DONE_WINDOW: &str = "outfit_portrait_done";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Shoot,
    Use,
    Retake,
    /// Cancel, the window's X or the photo mode's: the photo part ends.
    Cancel,
    /// Wear the outfit for the photo.
    Wear,
    /// Put the outfit worn before back on.
    PutBack,
    /// Keep the outfit worn for the photo.
    Keep,
    /// The same outfit in the square photo mode.
    PhotoMode,
    /// The photo mode's helpers.
    Reframe,
    Pose,
    Studio,
}

/// What the window shows.
pub struct View<'a> {
    pub name: &'a str,
    pub preview: Option<&'a egui::TextureHandle>,
    /// The photo awaiting Use.
    pub shot: Option<&'a egui::TextureHandle>,
    /// The outfit's picture now.
    pub current: Option<&'a egui::TextureHandle>,
    pub worn: bool,
    /// Worn from here: the name of the outfit worn before (None inside:
    /// not known).
    pub previous: Option<Option<&'a str>>,
    /// The current outfit has unsaved changes.
    pub dirty: bool,
    pub shooting: bool,
    /// The photo part is over: the previous outfit's question.
    pub over: bool,
    pub saved: bool,
    /// The photo mode's helpers that are on.
    pub pose: bool,
    pub studio: bool,
}

const FRAMINGS: [Framing; 3] = [Framing::Body, Framing::Bust, Framing::Face];
const POSES: [Pose; 3] = [Pose::A, Pose::ArmsDown, Pose::Current];
const ANGLES: [Angle; 3] = [Angle::Front, Angle::ThreeQuarter, Angle::Profile];
const BACKDROPS: [Backdrop; 4] = [Backdrop::Dark, Backdrop::Grey, Backdrop::Light, Backdrop::Scene];
const LIGHTINGS: [Lighting; 2] = [Lighting::Studio, Lighting::Region];

/// The auto portrait's window, or once its photo is over, the previous
/// outfit's question; `open` is cleared by its X.
pub fn window(ctx: &egui::Context, p: &Palette, settings: &mut PortraitSettings, view: &View, open: &mut bool) -> Vec<Action> {
    let mut actions = Vec::new();
    let title = aurora_i18n::tr!("ui-appearance-portrait-title", outfit = view.name).to_string();
    if view.over {
        widgets::Floater::new(DONE_WINDOW, title, egui::pos2(160.0, 140.0), egui::vec2(420.0, 160.0))
            .auto_height()
            .show(ctx, p, open, |ui| done(ui, p, view, &mut actions));
        return actions;
    }
    widgets::Floater::new(WINDOW, title, egui::pos2(120.0, 80.0), egui::vec2(780.0, 600.0))
        .steady_size()
        .show(ctx, p, open, |ui| {
            let (mut above, footer) = widgets::above_footer(ui);
            body(&mut above, p, settings, view, &mut actions);
            footer.show(ui, |ui| footer_ui(ui, p, view, &mut actions));
        });
    actions
}

/// The preview and its notes on the left, the choices on the right.
fn body(ui: &mut egui::Ui, p: &Palette, settings: &mut PortraitSettings, view: &View, actions: &mut Vec<Action>) {
    let room = ui.available_rect_before_wrap();
    // the notes under the square: the local note, the current picture and
    // the offer to wear the outfit, each when shown
    let mut notes = 140.0;
    if view.current.is_some() {
        notes += 100.0;
    }
    if !view.worn {
        notes += if view.previous.is_some() { 40.0 } else { 130.0 };
    }
    let side = (room.width() * 0.54).min(room.height() - notes).max(120.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.vertical(|ui| {
            ui.set_width(side);
            ui.spacing_mut().item_spacing.y = 10.0;
            let (square, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
            preview(ui, p, square, view);
            not_worn(ui, p, view, actions);
            note(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-appearance-portrait-local"));
            if let Some(t) = view.current {
                current(ui, p, t);
            }
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let shot = view.shot.is_some() || view.shooting;
            ui.add_enabled_ui(!shot, |ui| choices_ui(ui, p, settings));
        });
    });
}

pub(super) fn choices_ui(ui: &mut egui::Ui, p: &Palette, s: &mut PortraitSettings) {
    choice(
        ui,
        p,
        aurora_i18n::tr!("ui-appearance-portrait-framing"),
        &mut s.framing,
        &FRAMINGS,
        &[
            aurora_i18n::tr!("ui-appearance-portrait-body"),
            aurora_i18n::tr!("ui-appearance-portrait-bust"),
            aurora_i18n::tr!("ui-appearance-portrait-face"),
        ],
    );
    choice(
        ui,
        p,
        aurora_i18n::tr!("ui-appearance-portrait-pose"),
        &mut s.pose,
        &POSES,
        &[
            aurora_i18n::tr!("ui-appearance-portrait-pose-a"),
            aurora_i18n::tr!("ui-appearance-portrait-arms-down"),
            aurora_i18n::tr!("ui-appearance-portrait-pose-current"),
        ],
    );
    choice(
        ui,
        p,
        aurora_i18n::tr!("ui-appearance-portrait-angle"),
        &mut s.angle,
        &ANGLES,
        &[
            aurora_i18n::tr!("ui-appearance-portrait-front"),
            aurora_i18n::tr!("ui-appearance-portrait-three-quarter"),
            aurora_i18n::tr!("ui-appearance-portrait-profile"),
        ],
    );
    choice(
        ui,
        p,
        aurora_i18n::tr!("ui-appearance-portrait-backdrop"),
        &mut s.backdrop,
        &BACKDROPS,
        &[
            aurora_i18n::tr!("ui-appearance-portrait-dark"),
            aurora_i18n::tr!("ui-appearance-portrait-grey"),
            aurora_i18n::tr!("ui-appearance-portrait-light"),
            aurora_i18n::tr!("ui-appearance-portrait-scene"),
        ],
    );
    choice(
        ui,
        p,
        aurora_i18n::tr!("ui-appearance-portrait-lighting"),
        &mut s.lighting,
        &LIGHTINGS,
        &[
            aurora_i18n::tr!("ui-appearance-portrait-studio"),
            aurora_i18n::tr!("ui-appearance-portrait-region"),
        ],
    );
    ui.add_space(6.0);
    // (the backdrop alone already shows our avatar alone)
    ui.add_enabled_ui(s.backdrop == Backdrop::Scene, |ui| {
        widgets::checkbox(ui, p, &mut s.hide_others, aurora_i18n::tr!("ui-appearance-portrait-hide-others"));
    });
    widgets::checkbox(ui, p, &mut s.keep, aurora_i18n::tr!("ui-appearance-portrait-keep"));
}

/// One choice: its title, then its options side by side, or one under the
/// other when they do not fit (a narrow window, a large text).
fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, p: &Palette, title: &str, value: &mut T, all: &[T], labels: &[&str]) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(12.0).family(fonts::semibold(ui.ctx())).color(p.muted));
    let mut selected = all.iter().position(|v| v == value).unwrap_or(0);
    let options: Vec<(&str, bool)> = labels.iter().map(|l| (*l, false)).collect();
    let changed = if widgets::segmented_size(ui, &options).x <= ui.available_width() {
        widgets::segmented(ui, p, &mut selected, &options)
    } else {
        let mut changed = false;
        for (i, label) in labels.iter().enumerate() {
            if widgets::radio(ui, p, selected == i, *label).clicked() && selected != i {
                selected = i;
                changed = true;
            }
        }
        changed
    };
    if changed {
        *value = all[selected];
    }
}

/// The preview, or the photo to confirm, with the thirds, the corners and
/// the "framed" pill.
fn preview(ui: &egui::Ui, p: &Palette, square: Rect, view: &View) {
    let painter = ui.painter_at(square);
    let radius = ui.visuals().window_corner_radius;
    painter.rect_filled(square, radius, p.bar);
    let image = view.shot.or(view.preview);
    if let Some(t) = image {
        painter.image(
            t.id(),
            square,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            widgets::PICTURE_TINT,
        );
    } else {
        painter.text(
            square.center(),
            Align2::CENTER_CENTER,
            aurora_i18n::tr!("ui-appearance-portrait-preparing"),
            egui::FontId::proportional(13.0),
            p.muted,
        );
    }
    let line = egui::Stroke::new(1.0, p.ink.gamma_multiply(0.16));
    for i in 1..3 {
        let f = i as f32 / 3.0;
        let x = square.left() + square.width() * f;
        let y = square.top() + square.height() * f;
        painter.line_segment([egui::pos2(x, square.top()), egui::pos2(x, square.bottom())], line);
        painter.line_segment([egui::pos2(square.left(), y), egui::pos2(square.right(), y)], line);
    }
    let corner = egui::Stroke::new(2.0, p.ink);
    let (inset, arm) = (12.0, 22.0);
    let r = square.shrink(inset);
    for (at, dx, dy) in [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.left_bottom(), 1.0, -1.0),
        (r.right_bottom(), -1.0, -1.0),
    ] {
        painter.line_segment([at, at + Vec2::new(arm * dx, 0.0)], corner);
        painter.line_segment([at, at + Vec2::new(0.0, arm * dy)], corner);
    }
    // the pill: framed automatically, or the photo to confirm
    let (dot, text) = if view.shot.is_some() {
        (p.violet_light, aurora_i18n::tr!("ui-appearance-portrait-check"))
    } else {
        (p.success, aurora_i18n::tr!("ui-appearance-portrait-framed"))
    };
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), p.ink);
    let pill = Rect::from_center_size(
        egui::pos2(square.center().x, square.top() + 24.0),
        Vec2::new(galley.size().x + 32.0, 24.0),
    );
    painter.rect_filled(pill, radius, p.navy.gamma_multiply(0.7));
    painter.circle_filled(egui::pos2(pill.left() + 13.0, pill.center().y), 3.5, dot);
    painter.galley(
        egui::pos2(pill.left() + 22.0, pill.center().y - galley.size().y / 2.0),
        galley,
        p.ink,
    );
}

/// An icon and a muted, wrapped line.
pub(super) fn note(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if let Some(t) = icons::global(icon) {
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(p.muted));
        }
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
    });
}

/// The outfit not worn: the offer to wear it, and what putting the
/// current one back would lose.
fn not_worn(ui: &mut egui::Ui, p: &Palette, view: &View, actions: &mut Vec<Action>) {
    if view.worn {
        return;
    }
    if view.previous.is_some() {
        note(ui, p, "hanger-2-filled", aurora_i18n::tr!("ui-appearance-portrait-wearing"));
        return;
    }
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            note(ui, p, "alert-triangle-filled", aurora_i18n::tr!("ui-appearance-portrait-not-worn"));
            if view.dirty {
                note(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-appearance-portrait-unsaved"));
            }
            let label = aurora_i18n::tr!("ui-appearance-portrait-wear");
            let width = widgets::action_button_fit(ui, "hanger-2-filled", label);
            if widgets::action_button(
                ui,
                p,
                "hanger-2-filled",
                label,
                widgets::ActionKind::Primary,
                Vec2::new(width, 30.0),
            )
            .clicked()
            {
                actions.push(Action::Wear);
            }
        });
}

/// The outfit's picture now, which the photo replaces.
fn current(ui: &mut egui::Ui, p: &Palette, t: &egui::TextureHandle) {
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(
                    egui::Image::new(t)
                        .fit_to_exact_size(Vec2::splat(54.0))
                        .corner_radius(ui.visuals().window_corner_radius),
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-appearance-portrait-current"))
                            .size(12.0)
                            .family(fonts::semibold(ui.ctx()))
                            .color(p.ink),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(aurora_i18n::tr!("ui-appearance-portrait-current-replaced"))
                                .size(12.0)
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                });
            });
        });
}

/// Cancel and Take (Retake and Use once the photo is shown), and the way
/// to the photo mode.
fn footer_ui(ui: &mut egui::Ui, p: &Palette, view: &View, actions: &mut Vec<Action>) {
    let height = 36.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if view.shot.is_some() {
            let retake = aurora_i18n::tr!("ui-appearance-portrait-retake");
            let width = widgets::action_button_fit(ui, "refresh", retake);
            if widgets::action_button(ui, p, "refresh", retake, widgets::ActionKind::Flat, Vec2::new(width, height)).clicked() {
                actions.push(Action::Retake);
            }
            let size = Vec2::new(ui.available_width(), height);
            let use_it = aurora_i18n::tr!("ui-appearance-portrait-use");
            if widgets::action_button(ui, p, "check", use_it, widgets::ActionKind::Primary, size).clicked() {
                actions.push(Action::Use);
            }
            return;
        }
        let cancel = aurora_i18n::tr!("common-cancel");
        let width = widgets::action_button_fit(ui, "x", cancel);
        if widgets::action_button(ui, p, "x", cancel, widgets::ActionKind::Flat, Vec2::new(width, height)).clicked() {
            actions.push(Action::Cancel);
        }
        let size = Vec2::new(ui.available_width(), height);
        if view.shooting {
            ui.add_sized(size, |ui: &mut egui::Ui| {
                widgets::busy_button(ui, p, aurora_i18n::tr!("ui-appearance-portrait-taking"))
            });
        } else {
            let take = aurora_i18n::tr!("ui-appearance-portrait-take");
            // (a photo of another outfit than the one it is for is no use)
            let ready = view.worn;
            if widgets::action_button_if(ui, p, "camera-filled", take, widgets::ActionKind::Primary, size, ready).clicked() && ready {
                actions.push(Action::Shoot);
            }
        }
    });
    ui.add_space(4.0);
    ui.vertical_centered(|ui| {
        if ui
            .add(
                egui::Button::new(
                    RichText::new(aurora_i18n::tr!("ui-appearance-portrait-adjust"))
                        .size(12.0)
                        .color(p.violet_light),
                )
                .frame(false),
            )
            .clicked()
        {
            actions.push(Action::PhotoMode);
        }
    });
}

/// Once the photo is over: saved or not, and the previous outfit back.
fn done(ui: &mut egui::Ui, p: &Palette, view: &View, actions: &mut Vec<Action>) {
    ui.spacing_mut().item_spacing.y = 10.0;
    let (icon, text) = if view.saved {
        ("square-check-filled", aurora_i18n::tr!("ui-appearance-portrait-saved"))
    } else {
        ("info-circle-filled", aurora_i18n::tr!("ui-appearance-portrait-not-saved"))
    };
    note(ui, p, icon, text);
    let previous = view.previous.flatten();
    if previous.is_none() {
        note(
            ui,
            p,
            "info-circle-filled",
            aurora_i18n::tr!("ui-appearance-portrait-previous-unknown"),
        );
    }
    let keep = aurora_i18n::tr!("ui-appearance-portrait-keep-this");
    let mut buttons = vec![widgets::DialogButton {
        icon: "hanger-2-filled",
        label: keep,
        kind: if previous.is_some() {
            widgets::ActionKind::Flat
        } else {
            widgets::ActionKind::Primary
        },
        enabled: true,
    }];
    let put_back = previous.map(|name| aurora_i18n::tr!("ui-appearance-portrait-put-back", outfit = name).to_string());
    if let Some(label) = &put_back {
        buttons.push(widgets::DialogButton {
            icon: "arrow-back-up",
            label,
            kind: widgets::ActionKind::Primary,
            enabled: true,
        });
    }
    match widgets::dialog_buttons(ui, p, &buttons, 32.0) {
        Some(0) => actions.push(Action::Keep),
        Some(_) => actions.push(Action::PutBack),
        None => {}
    }
}

/// The square photo mode over the viewfinder: the banner with the
/// outfit's name and its X, the helpers on the left, how it goes on the
/// right, and once the photo is taken, the photo with Use and Retake.
pub fn photo_mode(ctx: &egui::Context, p: &Palette, view: &View) -> Vec<Action> {
    let mut actions = Vec::new();
    let screen = ctx.content_rect();
    let glass = p.navy.gamma_multiply(0.82);
    egui::Area::new(egui::Id::unique("outfit_photo_banner"))
        .order(egui::Order::Foreground)
        .fade_in(false)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(screen.center_top() + Vec2::new(0.0, 60.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(glass)
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin {
                    left: 14,
                    right: 8,
                    top: 6,
                    bottom: 6,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if let Some(t) = icons::global("hanger-2-filled") {
                            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(p.violet_light));
                        }
                        let mut job = egui::text::LayoutJob::default();
                        let font = egui::FontId::proportional(13.0);
                        job.append(
                            &format!("{} ", aurora_i18n::tr!("ui-appearance-portrait-vf-banner")),
                            0.0,
                            egui::TextFormat::simple(font.clone(), p.ink),
                        );
                        job.append(view.name, 0.0, fonts::semibold_format(&fonts::semibold(ui.ctx()), 13.0, p.ink));
                        job.append(
                            &format!("  · {}", aurora_i18n::tr!("ui-appearance-portrait-vf-square")),
                            0.0,
                            egui::TextFormat::simple(font, p.muted),
                        );
                        ui.label(job);
                        let exit = aurora_i18n::tr!("ui-appearance-portrait-vf-exit");
                        if widgets::icon_button(ui, p, "x", exit, true) {
                            actions.push(Action::Cancel);
                        }
                    });
                });
        });
    // the helpers, low on the left: the viewfinder's key list holds the
    // top left corner and its focus modes the bottom
    egui::Area::new(egui::Id::unique("outfit_photo_helpers"))
        .order(egui::Order::Foreground)
        .fade_in(false)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(screen.left_bottom() + Vec2::new(24.0, -80.0))
        .show(ctx, |ui| {
            ui.set_width(230.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-appearance-portrait-vf-helpers").to_uppercase())
                    .size(11.0)
                    .family(fonts::semibold(ui.ctx()))
                    .color(p.muted),
            );
            for (icon, label, on, action) in [
                (
                    "focus-centered",
                    aurora_i18n::tr!("ui-appearance-portrait-vf-reframe"),
                    false,
                    Action::Reframe,
                ),
                (
                    "man-filled",
                    aurora_i18n::tr!("ui-appearance-portrait-vf-pose"),
                    view.pose,
                    Action::Pose,
                ),
                (
                    "user-scan",
                    aurora_i18n::tr!("ui-appearance-portrait-vf-studio"),
                    view.studio,
                    Action::Studio,
                ),
            ] {
                if helper(ui, p, icon, label, on).clicked() {
                    actions.push(action);
                }
            }
            if !view.worn {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(glass)
                    .corner_radius(ui.visuals().window_corner_radius)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let text = if view.previous.is_some() {
                            aurora_i18n::tr!("ui-appearance-portrait-wearing")
                        } else {
                            aurora_i18n::tr!("ui-appearance-portrait-not-worn")
                        };
                        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.ink)).wrap());
                        if view.previous.is_none() {
                            if view.dirty {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(aurora_i18n::tr!("ui-appearance-portrait-unsaved"))
                                            .size(12.0)
                                            .color(p.muted),
                                    )
                                    .wrap(),
                                );
                            }
                            let label = aurora_i18n::tr!("ui-appearance-portrait-wear");
                            let size = Vec2::new(ui.available_width(), 30.0);
                            if widgets::action_button(ui, p, "hanger-2-filled", label, widgets::ActionKind::Primary, size).clicked() {
                                actions.push(Action::Wear);
                            }
                        }
                    });
            }
        });
    // how it goes, low on the right, clear of the viewfinder's button
    // rail and its depth of field readout
    egui::Area::new(egui::Id::unique("outfit_photo_how"))
        .order(egui::Order::Foreground)
        .fade_in(false)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(screen.right_bottom() + Vec2::new(-100.0, -120.0))
        .show(ctx, |ui| {
            ui.set_width(230.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-appearance-portrait-vf-how").to_uppercase())
                    .size(11.0)
                    .family(fonts::semibold(ui.ctx()))
                    .color(p.muted),
            );
            egui::Frame::new()
                .fill(glass)
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(aurora_i18n::tr!("ui-appearance-portrait-vf-how-text"))
                                .size(12.0)
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                });
        });
    // the photo taken, to confirm
    if let Some(t) = view.shot.filter(|_| !view.shooting) {
        egui::Area::new(egui::Id::unique("outfit_photo_shot"))
            .order(egui::Order::Foreground)
            .fade_in(false)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(screen.center())
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(egui::Stroke::new(1.0, p.raised))
                    .corner_radius(ui.visuals().window_corner_radius)
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        let side = (screen.height() * 0.45).clamp(160.0, 320.0);
                        ui.set_width(side);
                        ui.spacing_mut().item_spacing.y = 10.0;
                        ui.add(
                            egui::Image::new(t)
                                .fit_to_exact_size(Vec2::splat(side))
                                .corner_radius(ui.visuals().window_corner_radius),
                        );
                        let height = 34.0;
                        let use_it = aurora_i18n::tr!("ui-appearance-portrait-vf-use");
                        if widgets::action_button(ui, p, "check", use_it, widgets::ActionKind::Primary, Vec2::new(side, height)).clicked() {
                            actions.push(Action::Use);
                        }
                        let retake = aurora_i18n::tr!("ui-appearance-portrait-retake");
                        if widgets::action_button(ui, p, "refresh", retake, widgets::ActionKind::Flat, Vec2::new(side, height)).clicked() {
                            actions.push(Action::Retake);
                        }
                    });
            });
    }
    actions
}

/// A helper of the photo mode: an icon and its label on glass, outlined
/// in the accent while on.
fn helper(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: bool) -> egui::Response {
    let size = Vec2::new(ui.available_width(), 36.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, label));
    let face = if response.hovered() {
        p.raised
    } else {
        p.navy.gamma_multiply(0.82)
    };
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(rect, radius, face);
    if on {
        ui.painter()
            .rect_stroke(rect, radius, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
    }
    if let Some(t) = icons::global(icon) {
        let at = Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(16.0));
        ui.painter()
            .image(t.id(), at, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), p.ink);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        p.ink,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View<'static> {
        View {
            name: "Velvet evening",
            preview: None,
            shot: None,
            current: None,
            worn: false,
            previous: None,
            dirty: true,
            shooting: false,
            over: false,
            saved: false,
            pose: false,
            studio: false,
        }
    }

    #[test]
    fn the_window_keeps_its_size() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[egui::vec2(780.0, 600.0), egui::vec2(560.0, 480.0)],
            aurora_i18n::tr!("ui-appearance-portrait-adjust"),
            PortraitSettings::default,
            |settings, ctx, p| {
                window(ctx, p, settings, &view(), &mut true);
            },
        );
    }
}
