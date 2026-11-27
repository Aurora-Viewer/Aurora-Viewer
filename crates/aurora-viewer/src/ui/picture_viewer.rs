//! The Picture viewer window (World › Picture viewer; Aurora's own): a
//! picture of the computer shown, cropped, turned, resized, its light and
//! colour set, its looks and LUTs (the photo window's), seen as a card in 3D
//! with a relief, and saved to a new file. The pictures and their work are in
//! `app/picture_viewer.rs`; this file draws and says what was asked
//! (`Action`).

use super::looks_editor::{LooksUi, Options, Stack};
use super::widgets;
use crate::picture::adjust::Adjust;
use crate::picture::{Crop, Edit};
use crate::theme::Palette;
use egui::{Pos2, Rect, RichText, Sense, Vec2};
use std::collections::HashMap;

pub const WINDOW: &str = "picture_viewer";

/// The side panel's width.
const SIDE: f32 = 290.0;

/// The crop's ratios (width:height), "Free" first.
const RATIOS: &[(f32, f32)] = &[(0.0, 0.0), (1.0, 1.0), (4.0, 3.0), (3.2, 2.0), (16.0, 9.0), (3.0, 4.0), (9.0, 16.0)];

/// What the window asks of `app::picture_viewer`.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Open,
    Save,
    Undo,
    Redo,
    /// Back to the picture as it was opened.
    Reset,
    Edit(Edit),
    /// The light and colour moved; `done`: the slider was let go (one step of
    /// the history).
    Adjust {
        adjust: Adjust,
        done: bool,
    },
    /// The relief moved; `done`: let go (the card is made again).
    Relief {
        relief: f32,
        done: bool,
    },
    /// The looks changed.
    Looks,
    /// A looks editor's own request (import a LUT…).
    LooksEditor(super::snapshot::Action),
    /// The overwrite question answered.
    Overwrite(bool),
}

/// The 2D view: zoomed and moved over the fitted picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View2d {
    /// 1 is fitted to the canvas.
    pub zoom: f32,
    pub offset: Vec2,
}

impl Default for View2d {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: Vec2::ZERO,
        }
    }
}

impl View2d {
    fn zoom_at(&mut self, factor: f32, anchor: Vec2) {
        let next = (self.zoom * factor).clamp(0.05, 32.0);
        self.offset = anchor + (self.offset - anchor) * (next / self.zoom);
        self.zoom = next;
    }

    /// Where a `size` picture is drawn in `canvas`.
    pub fn rect(&self, size: Vec2, canvas: Rect) -> Rect {
        let fit = (canvas.width() / size.x.max(1.0)).min(canvas.height() / size.y.max(1.0));
        Rect::from_center_size(canvas.center() + self.offset, size * fit * self.zoom)
    }

    /// The zoom that shows each pixel of a `size` picture as one point.
    fn actual(size: Vec2, canvas: Rect) -> Self {
        let fit = (canvas.width() / size.x.max(1.0)).min(canvas.height() / size.y.max(1.0));
        Self {
            zoom: (1.0 / fit.max(1e-6)).clamp(0.05, 32.0),
            offset: Vec2::ZERO,
        }
    }
}

/// The window's own state for the session (what it draws, not the pictures).
#[derive(Default)]
pub struct PictureViewerUi {
    pub view: View2d,
    /// 1:1 asked: applied once the canvas is known.
    actual: bool,
    pub mode3d: bool,
    pub view3d: Option<aurora_render::preview::View>,
    /// The crop being chosen, in fractions of the picture shown.
    pub crop: Option<Crop>,
    crop_from: Option<Pos2>,
    pub ratio: usize,
    /// The resize, in percent of the current size.
    pub resize: f32,
    /// The light and colour shown by the sliders (sent as they move).
    pub adjust: Adjust,
    pub relief: f32,
    pub looks: LooksUi,
    /// The overwrite question is shown.
    pub confirm_overwrite: Option<String>,
}

/// What the window shows, given by the app each frame.
pub struct Shown<'a> {
    /// The picture's name, or None before one is opened.
    pub name: Option<&'a str>,
    /// The picture on screen (the working copy, edited).
    pub texture: Option<&'a egui::TextureHandle>,
    /// Its size on screen (the working copy's).
    pub size: (u32, u32),
    /// The size the picture is saved at.
    pub full: (u32, u32),
    /// The 3D card, once made.
    pub model: Option<std::sync::Arc<aurora_render::preview::Model>>,
    /// The looks, drawn on the GPU over the 2D picture (`picture_look`).
    pub graded: Option<aurora_render::picture_look::Graded>,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Opening, working or saving: the progress (0..1) and its words.
    pub busy: Option<(f32, String)>,
    /// The last message: words, and whether it is an error.
    pub status: Option<(&'a str, bool)>,
    pub luts: &'a [String],
    pub thumbs: &'a HashMap<String, usize>,
}

pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    open: &mut bool,
    ui_state: &mut PictureViewerUi,
    shown: Shown,
    mut stack: Stack,
    out: &mut Vec<Action>,
) {
    widgets::Floater::new(
        WINDOW,
        aurora_i18n::tr!("picture-viewer-title"),
        egui::pos2(220.0, 90.0),
        Vec2::new(1000.0, 660.0),
    )
    .help(aurora_i18n::tr!("picture-viewer-help"))
    .show(ctx, p, open, |ui| {
        let bounds = ui.available_rect_before_wrap();
        let has = shown.texture.is_some();
        let toolbar = Rect::from_min_size(bounds.min, Vec2::new(bounds.width(), 34.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(toolbar), |ui| {
            ui.horizontal(|ui| toolbar_row(ui, p, ui_state, &shown, has, out));
        });
        let body_top = toolbar.bottom() + 6.0;
        let side = Rect::from_min_max(Pos2::new(bounds.right() - SIDE, body_top), bounds.max);
        let canvas = Rect::from_min_max(Pos2::new(bounds.left(), body_top), Pos2::new(side.left() - 10.0, bounds.bottom()));
        if has {
            ui.scope_builder(egui::UiBuilder::new().max_rect(side), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("picture_viewer_side")
                    .auto_shrink([false, false])
                    .show(ui, |ui| side_panel(ui, p, ui_state, &shown, &mut stack, out));
            });
        }
        let canvas = if has { canvas } else { Rect::from_min_max(canvas.min, bounds.max) };
        ui.scope_builder(egui::UiBuilder::new().max_rect(canvas), |ui| {
            if !has {
                empty(ui, p, canvas, &shown);
            } else if ui_state.mode3d {
                card(ui, p, ui_state, &shown, canvas);
            } else {
                picture(ui, p, ui_state, &shown, canvas);
            }
        });
        ui.allocate_rect(bounds, Sense::hover());
    });
    if let Some(name) = ui_state.confirm_overwrite.clone() {
        overwrite(ctx, p, &name, ui_state, out);
    }
}

fn toolbar_row(ui: &mut egui::Ui, p: &Palette, s: &mut PictureViewerUi, shown: &Shown, has: bool, out: &mut Vec<Action>) {
    let busy = shown.busy.is_some();
    if widgets::icon_button(ui, p, "folder-open-filled", aurora_i18n::tr!("picture-viewer-open"), !busy) {
        out.push(Action::Open);
    }
    if widgets::icon_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("picture-viewer-save"), has && !busy) {
        out.push(Action::Save);
    }
    if widgets::icon_button(ui, p, "arrow-back-up", aurora_i18n::tr!("picture-viewer-undo"), shown.can_undo) {
        out.push(Action::Undo);
    }
    if widgets::icon_button(ui, p, "arrow-forward-up", aurora_i18n::tr!("picture-viewer-redo"), shown.can_redo) {
        out.push(Action::Redo);
    }
    if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("picture-viewer-reset"), shown.can_undo) {
        out.push(Action::Reset);
    }
    ui.add_space(10.0);
    if widgets::icon_button(ui, p, "arrows-maximize", aurora_i18n::tr!("picture-viewer-fit"), has && !s.mode3d) {
        s.view = View2d::default();
    }
    if widgets::icon_button(ui, p, "zoom-in-filled", aurora_i18n::tr!("picture-viewer-actual"), has && !s.mode3d) {
        s.actual = true;
    }
    if widgets::icon_toggle(ui, p, "cube-3d-sphere", aurora_i18n::tr!("picture-viewer-3d"), s.mode3d) && has {
        s.mode3d = !s.mode3d;
        s.crop = None;
    }
    if let Some(name) = shown.name {
        ui.add_space(12.0);
        ui.label(RichText::new(name).color(p.ink).strong());
        let (w, h) = shown.full;
        ui.label(RichText::new(aurora_i18n::tr!("picture-viewer-size", width = w, height = h)).color(p.muted));
    }
    if let Some((fraction, words)) = &shown.busy {
        ui.add_space(12.0);
        widgets::progress(ui, p, "picture_viewer_progress", *fraction, 160.0, words, None);
    } else if let Some((words, error)) = shown.status {
        ui.add_space(12.0);
        let color = if error { p.danger } else { p.muted };
        ui.add(egui::Label::new(RichText::new(words).color(color)).truncate());
    }
}

fn empty(ui: &mut egui::Ui, p: &Palette, canvas: Rect, shown: &Shown) {
    let words = match shown.busy {
        Some(_) => aurora_i18n::tr!("picture-viewer-loading"),
        None => aurora_i18n::tr!("picture-viewer-empty"),
    };
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout(words.to_owned(), font, p.muted, (canvas.width() - 40.0).max(1.0));
    let at = canvas.center() - galley.size() * 0.5;
    ui.painter().galley(at, galley, p.muted);
    if let Some((error, true)) = shown.status {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let galley = ui
            .painter()
            .layout(error.to_owned(), font, p.danger, (canvas.width() - 40.0).max(1.0));
        ui.painter()
            .galley(Pos2::new(canvas.center().x - galley.size().x * 0.5, at.y + 30.0), galley, p.danger);
    }
}

/// The picture in 2D: zoomed with the wheel, moved by dragging, fitted
/// again on a double click; a crop being chosen is dragged on it.
fn picture(ui: &mut egui::Ui, p: &Palette, s: &mut PictureViewerUi, shown: &Shown, canvas: Rect) {
    let Some(texture) = shown.texture else { return };
    let size = Vec2::new(shown.size.0 as f32, shown.size.1 as f32);
    if std::mem::take(&mut s.actual) {
        s.view = View2d::actual(
            Vec2::new(shown.full.0 as f32, shown.full.1 as f32) * size.x / shown.full.0.max(1) as f32,
            canvas,
        );
        // 1:1 is of the full picture, which the screen copy may be smaller than
        s.view.zoom = (s.view.zoom * shown.full.0 as f32 / shown.size.0.max(1) as f32).clamp(0.05, 32.0);
    }
    let response = ui.interact(canvas, ui.scope_id().with("picture_canvas"), Sense::click_and_drag());
    if response.hovered() {
        let (wheel, pinch) = ui.input_mut(|i| {
            let wheel = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta = Vec2::ZERO;
            (wheel, i.zoom_delta())
        });
        let factor = if pinch != 1.0 { pinch } else { (wheel * 0.005).exp() };
        if factor != 1.0 {
            let anchor = response.hover_pos().unwrap_or(canvas.center()) - canvas.center();
            s.view.zoom_at(factor, anchor);
        }
    }
    let image = s.view.rect(size, canvas);
    if let Some(crop) = s.crop.as_mut() {
        crop_drag(
            &response,
            image,
            crop,
            &mut s.crop_from,
            RATIOS[s.ratio.min(RATIOS.len() - 1)],
            shown.size,
        );
    } else if response.dragged_by(egui::PointerButton::Primary) {
        s.view.offset += response.drag_delta();
    }
    if response.double_clicked() {
        s.view = View2d::default();
    }
    let cursor = if s.crop.is_some() {
        egui::CursorIcon::Crosshair
    } else if response.dragged() {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::Grab
    };
    response.on_hover_cursor(cursor);
    let painter = ui.painter_at(canvas);
    checkerboard(&painter, p, image.intersect(canvas));
    match &shown.graded {
        Some(graded) => aurora_render::picture_look::paint(&painter, image, graded.clone()),
        None => {
            painter.image(
                texture.id(),
                image,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                widgets::PICTURE_TINT,
            );
        }
    }
    if let Some(crop) = s.crop {
        let c = crop.clamped();
        let keep = Rect::from_min_size(
            image.min + Vec2::new(c.x * image.width(), c.y * image.height()),
            Vec2::new(c.w * image.width(), c.h * image.height()),
        );
        // the parts cut away are darkened around the part kept
        let shade = p.navy.gamma_multiply(0.62);
        for r in [
            Rect::from_min_max(image.min, Pos2::new(image.right(), keep.top())),
            Rect::from_min_max(Pos2::new(image.left(), keep.bottom()), image.max),
            Rect::from_min_max(Pos2::new(image.left(), keep.top()), Pos2::new(keep.left(), keep.bottom())),
            Rect::from_min_max(Pos2::new(keep.right(), keep.top()), Pos2::new(image.right(), keep.bottom())),
        ] {
            painter.rect_filled(r, 0.0, shade);
        }
        painter.rect_stroke(keep, 0.0, egui::Stroke::new(1.5, p.violet_light), egui::StrokeKind::Middle);
    }
}

/// A crop dragged out on the picture drawn at `image`: from where the press
/// started to the pointer, held to `ratio` (width, height; 0: free) of the
/// picture's pixels (`size`).
fn crop_drag(response: &egui::Response, image: Rect, crop: &mut Crop, from: &mut Option<Pos2>, ratio: (f32, f32), size: (u32, u32)) {
    let frac = |at: Pos2| {
        Pos2::new(
            ((at.x - image.left()) / image.width().max(1.0)).clamp(0.0, 1.0),
            ((at.y - image.top()) / image.height().max(1.0)).clamp(0.0, 1.0),
        )
    };
    if response.drag_started_by(egui::PointerButton::Primary) {
        *from = response.interact_pointer_pos().map(frac);
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let (Some(start), Some(at)) = (*from, response.interact_pointer_pos().map(frac))
    {
        let mut w = (at.x - start.x).abs();
        let mut h = (at.y - start.y).abs();
        if ratio.0 > 0.0 {
            // the ratio is of pixels; the fractions of a picture that is not
            // square are not
            let aspect = ratio.0 / ratio.1 * size.1.max(1) as f32 / size.0.max(1) as f32;
            if w / aspect > h {
                h = w / aspect;
            } else {
                w = h * aspect;
            }
        }
        let x = if at.x < start.x { start.x - w } else { start.x };
        let y = if at.y < start.y { start.y - h } else { start.y };
        *crop = Crop { x, y, w, h }.clamped();
    }
    if response.drag_stopped() {
        *from = None;
    }
}

/// A light/dark check under the picture, where it is transparent.
fn checkerboard(painter: &egui::Painter, p: &Palette, rect: Rect) {
    if !rect.is_positive() {
        return;
    }
    painter.rect_filled(rect, 0.0, p.panel);
    let cell = 12.0;
    let dark = p.field;
    let mut y = rect.top();
    let mut row = 0;
    while y < rect.bottom() {
        let mut x = rect.left() + if row % 2 == 0 { 0.0 } else { cell };
        while x < rect.right() {
            let r = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(cell)).intersect(rect);
            painter.rect_filled(r, 0.0, dark);
            x += cell * 2.0;
        }
        y += cell;
        row += 1;
    }
}

/// The picture as a card in 3D: dragged to turn, the wheel to come closer,
/// a double click to face it again.
fn card(ui: &mut egui::Ui, p: &Palette, s: &mut PictureViewerUi, shown: &Shown, canvas: Rect) {
    let background = p.panel.to_normalized_gamma_f32();
    let view = s.view3d.get_or_insert_with(|| crate::picture::relief::front_view(background));
    view.background = background;
    let Some(model) = shown.model.clone() else {
        ui.painter().text(
            canvas.center(),
            egui::Align2::CENTER_CENTER,
            aurora_i18n::tr!("picture-viewer-making-card"),
            egui::TextStyle::Body.resolve(ui.style()),
            p.muted,
        );
        return;
    };
    // the looks are drawn (unseen) for the card's picture, read back
    if let Some(graded) = &shown.graded {
        aurora_render::picture_look::paint(&ui.painter().with_clip_rect(Rect::NOTHING), canvas, graded.clone());
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(canvas));
    let response = aurora_render::preview::paint(&mut child, ui.scope_id().with("picture_card"), model, view, canvas.size());
    if response.double_clicked() {
        // the preview's own reset turns to its default side: face the card
        *view = crate::picture::relief::front_view(background);
    }
}

fn side_panel(ui: &mut egui::Ui, p: &Palette, s: &mut PictureViewerUi, shown: &Shown, stack: &mut Stack, out: &mut Vec<Action>) {
    ui.set_width(SIDE - 14.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    let busy = shown.busy.is_some();
    widgets::collapsing(
        ui,
        p,
        "picture_viewer_shape",
        aurora_i18n::tr!("picture-viewer-shape"),
        true,
        |ui| {
            ui.horizontal_wrapped(|ui| {
                let tip = aurora_i18n::tr!("picture-viewer-crop");
                if widgets::icon_toggle(ui, p, "crop", tip, s.crop.is_some()) && !s.mode3d {
                    s.crop = if s.crop.is_some() { None } else { Some(Crop::FULL) };
                }
                for (icon, tip, edit) in [
                    ("rotate", aurora_i18n::tr!("picture-viewer-rotate-left"), Edit::RotateLeft),
                    (
                        "rotate-clockwise",
                        aurora_i18n::tr!("picture-viewer-rotate-right"),
                        Edit::RotateRight,
                    ),
                    ("flip-horizontal", aurora_i18n::tr!("picture-viewer-flip-h"), Edit::FlipHorizontal),
                    ("flip-vertical", aurora_i18n::tr!("picture-viewer-flip-v"), Edit::FlipVertical),
                ] {
                    if widgets::icon_button(ui, p, icon, tip, !busy) {
                        out.push(Action::Edit(edit));
                    }
                }
            });
            if let Some(crop) = s.crop {
                ui.label(
                    RichText::new(aurora_i18n::tr!("picture-viewer-crop-hint"))
                        .size(11.5)
                        .color(p.muted),
                );
                let labels: Vec<String> = RATIOS
                    .iter()
                    .map(|&(w, h)| {
                        if w == 0.0 {
                            aurora_i18n::tr!("picture-viewer-ratio-free").to_string()
                        } else {
                            format!("{w}:{h}")
                        }
                    })
                    .collect();
                ui.horizontal_wrapped(|ui| {
                    for (i, label) in labels.iter().enumerate() {
                        if widgets::radio_value(ui, p, &mut s.ratio, i, label.as_str()).changed() {
                            s.crop = Some(Crop::FULL);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let size = Vec2::new(120.0, 28.0);
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), widgets::ActionKind::Flat, size).clicked() {
                        s.crop = None;
                    }
                    let full = crop.clamped() == Crop::FULL;
                    if widgets::action_button_if(
                        ui,
                        p,
                        "crop",
                        aurora_i18n::tr!("picture-viewer-crop-apply"),
                        widgets::ActionKind::Primary,
                        size,
                        !full && !busy,
                    )
                    .clicked()
                    {
                        out.push(Action::Edit(Edit::Crop(crop.clamped())));
                        s.crop = None;
                    }
                });
            }
            ui.add_space(4.0);
            if s.resize <= 0.0 {
                s.resize = 100.0;
            }
            let (w, h) = shown.full;
            let new = crate::picture::geometry::resized(w, h, s.resize / 100.0, s.resize / 100.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("picture-viewer-resize")).color(p.ink));
                ui.add(egui::DragValue::new(&mut s.resize).range(1.0..=400.0).suffix(" %").speed(0.5));
                ui.label(RichText::new(aurora_i18n::tr!("picture-viewer-size", width = new.0, height = new.1)).color(p.muted));
            });
            let changed = new != (w, h);
            if widgets::action_button_if(
                ui,
                p,
                "dimensions",
                aurora_i18n::tr!("picture-viewer-resize-apply"),
                widgets::ActionKind::Flat,
                Vec2::new(SIDE - 24.0, 28.0),
                changed && !busy,
            )
            .clicked()
            {
                let k = s.resize / 100.0;
                out.push(Action::Edit(Edit::Resize { sx: k, sy: k }));
                s.resize = 100.0;
            }
        },
    );
    widgets::collapsing(
        ui,
        p,
        "picture_viewer_light",
        aurora_i18n::tr!("picture-viewer-light"),
        true,
        |ui| {
            let a = &mut s.adjust;
            let mut moved = false;
            let mut done = false;
            for (label, value, min) in [
                (aurora_i18n::tr!("picture-viewer-brightness"), &mut a.brightness, -1.0),
                (aurora_i18n::tr!("picture-viewer-contrast"), &mut a.contrast, -1.0),
                (aurora_i18n::tr!("picture-viewer-saturation"), &mut a.saturation, -1.0),
                (aurora_i18n::tr!("picture-viewer-warmth"), &mut a.warmth, -1.0),
                (aurora_i18n::tr!("picture-viewer-sharpen"), &mut a.sharpen, 0.0),
            ] {
                let (m, d) = slider_row(ui, p, label, value, min);
                moved |= m;
                done |= d;
            }
            if moved || done {
                out.push(Action::Adjust { adjust: *a, done });
            }
        },
    );
    widgets::collapsing(
        ui,
        p,
        "picture_viewer_looks",
        aurora_i18n::tr!("picture-viewer-looks"),
        false,
        |ui| {
            let before = (stack.looks.clone(), stack.look_settings.clone());
            let thumb = |id: &str| shown.thumbs.get(id).copied();
            let opts = Options {
                list_height: 220.0,
                height: None,
                compare: false,
                luts: shown.luts,
                pano: false,
                thumb: &thumb,
            };
            let mut actions = Vec::new();
            let reborrowed = Stack {
                looks: &mut *stack.looks,
                look_settings: &mut *stack.look_settings,
                favorites: &mut *stack.favorites,
            };
            s.looks.show(ui, p, reborrowed, opts, &mut actions);
            out.extend(actions.into_iter().map(Action::LooksEditor));
            if (&*stack.looks, &*stack.look_settings) != (&before.0, &before.1) {
                out.push(Action::Looks);
            }
        },
    );
    if s.mode3d {
        widgets::collapsing(
            ui,
            p,
            "picture_viewer_relief",
            aurora_i18n::tr!("picture-viewer-relief"),
            true,
            |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("picture-viewer-relief-hint"))
                        .size(11.5)
                        .color(p.muted),
                );
                let mut percent = s.relief * 100.0;
                let (moved, done) = slider_row(ui, p, aurora_i18n::tr!("picture-viewer-relief"), &mut percent, 0.0);
                s.relief = percent / 100.0;
                if moved || done {
                    out.push(Action::Relief { relief: s.relief, done });
                }
            },
        );
    }
}

/// A setting from `min` to 1 (shown in percent): its name, a slider, its
/// value and a reset. Whether it moved, and whether it was let go (or reset).
fn slider_row(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut f32, min: f32) -> (bool, bool) {
    let mut moved = false;
    let mut done = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            Vec2::new(78.0, 20.0),
            egui::Label::new(RichText::new(label).color(p.ink)).truncate(),
        );
        let mut percent = *value * 100.0;
        // the row fits the side panel: name, slider, value, reset
        ui.spacing_mut().slider_width = 96.0;
        let r = ui.add(
            egui::Slider::new(&mut percent, (min * 100.0)..=100.0)
                .show_value(false)
                // a setting whose zero is the middle shows no fill from the left
                .trailing_fill(min >= 0.0),
        );
        let field = ui.add(
            egui::DragValue::new(&mut percent)
                .range((min * 100.0)..=100.0)
                .fixed_decimals(0)
                .suffix(" %"),
        );
        if r.changed() || field.changed() {
            *value = percent / 100.0;
            moved = true;
        }
        done |= r.drag_stopped() || (field.changed() && !field.dragged()) || field.drag_stopped();
        // a click on the track moves the value without a drag to let go
        done |= r.changed() && !r.dragged();
        if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("common-reset"), *value != 0.0) {
            *value = 0.0;
            moved = true;
            done = true;
        }
    });
    (moved, done)
}

fn overwrite(ctx: &egui::Context, p: &Palette, name: &str, s: &mut PictureViewerUi, out: &mut Vec<Action>) {
    let mut answer = None;
    let width = 380.0;
    widgets::Modal::new(egui::Id::unique("picture_viewer_overwrite")).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("picture-viewer-overwrite-title"))
                .size(17.0)
                .strong()
                .color(p.ink),
        );
        ui.label(RichText::new(aurora_i18n::tr!("picture-viewer-overwrite", name = name.to_string())).color(p.muted));
        ui.horizontal(|ui| {
            let size = Vec2::new((width - 8.0) / 2.0, 32.0);
            if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), widgets::ActionKind::Flat, size).clicked() {
                answer = Some(false);
            }
            if widgets::action_button(
                ui,
                p,
                "device-floppy-filled",
                aurora_i18n::tr!("picture-viewer-overwrite-yes"),
                widgets::ActionKind::Danger,
                size,
            )
            .clicked()
            {
                answer = Some(true);
            }
        });
    });
    if let Some(yes) = answer {
        s.confirm_overwrite = None;
        out.push(Action::Overwrite(yes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2d_view_fits_zooms_at_the_pointer_and_shows_actual_size() {
        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let fitted = View2d::default().rect(Vec2::new(800.0, 200.0), canvas);
        assert_eq!(fitted.size(), Vec2::new(400.0, 100.0), "the whole picture, its shape kept");
        let mut view = View2d::default();
        // zooming at a point keeps that point where it is
        let anchor = Vec2::new(100.0, 0.0);
        let before = view.rect(Vec2::new(800.0, 200.0), canvas);
        let under = (canvas.center() + anchor - before.min) / before.size();
        view.zoom_at(2.0, anchor);
        let after = view.rect(Vec2::new(800.0, 200.0), canvas);
        let still = (canvas.center() + anchor - after.min) / after.size();
        assert!((under - still).length() < 1e-4);
        let one = View2d::actual(Vec2::new(800.0, 200.0), canvas);
        assert!((one.rect(Vec2::new(800.0, 200.0), canvas).width() - 800.0).abs() < 1e-3);
    }
}
