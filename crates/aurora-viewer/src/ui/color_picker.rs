//! Aurora color picker, following LLFloaterColorPicker and LLToolPipette
//! (Firestorm, originally LGPL 2.1). RGB/LSL/hex, HSL, saved palette,
//! pipette and Apply/OK/Cancel use one shared, unquantized sRGB draft.

pub(crate) mod demo;
mod model;
#[cfg(test)]
mod tests;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use model::{Draft, bytes, hex, hsl_to_rgb, lsl, parse_hex};
pub use model::{Preferences, Rgb};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Preview(Rgb),
    Accept(Rgb),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Control {
    Grid,
    Lightness,
    Current,
    Slot(usize),
    Mode(usize),
    Rgb(usize),
    Hsl(usize),
    Hex,
    Apply,
    Pipette,
    CopyLsl,
    Accept,
    Cancel,
}

#[derive(Default)]
pub struct ColorPicker {
    draft: Option<Draft>,
    mixed: bool,
    pub pipette: bool,
    pub sampled: Option<Uuid>,
    sample_cursor: Option<(f32, f32)>,
    pending_preview: bool,
    notice: String,
    track_controls: bool,
    pub(crate) controls: HashMap<Control, Rect>,
}

impl ColorPicker {
    pub fn open(&mut self, rgb: Rgb, mixed: bool) {
        self.draft = Some(Draft::new(rgb));
        self.mixed = mixed;
        self.pipette = false;
        self.sampled = None;
        self.sample_cursor = None;
        self.pending_preview = false;
        self.notice.clear();
        self.track_controls = cfg!(test) || demo::mode().is_some();
    }

    pub fn is_open(&self) -> bool {
        self.draft.is_some()
    }

    pub fn close(&mut self) {
        self.draft = None;
        self.pipette = false;
        self.sampled = None;
        self.pending_preview = false;
    }

    pub fn needs_sample(&mut self, cursor: (f32, f32)) -> bool {
        if !self.pipette || self.sample_cursor == Some(cursor) {
            return false;
        }
        self.sample_cursor = Some(cursor);
        true
    }

    pub fn sample(&mut self, color: Option<(Uuid, Rgb)>) {
        if let Some((id, rgb)) = color {
            if let Some(draft) = self.draft.as_mut() {
                self.pending_preview |= draft.set_rgb(rgb) || self.mixed;
                self.sampled = Some(id);
                self.notice.clear();
            }
        } else {
            self.sampled = None;
            self.notice = aurora_i18n::tr!("color-picker-no-face").to_owned();
        }
    }

    fn record(&mut self, control: Control, rect: Rect) {
        if self.track_controls {
            self.controls.insert(control, rect);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, prefs: &mut Preferences) -> Option<Event> {
        let mut draft = self.draft.take()?;
        prefs.normalize();
        self.controls.clear();
        let apply_before = prefs.apply;
        let mut edited = std::mem::take(&mut self.pending_preview);
        let mut accepted = false;
        let mut canceled = false;
        let mut open = true;
        let screen = ctx.content_rect();
        let size = Vec2::new((screen.width() - 32.0).clamp(360.0, 500.0), 440.0);
        Floater::new(
            "build_color_picker",
            aurora_i18n::tr!("color-picker-title"),
            screen.center() - size * 0.5,
            size,
        )
        .fixed()
        .auto_height()
        .framed_header("palette")
        .help(aurora_i18n::tr!("color-picker-help"))
        .show(ctx, p, &mut open, |ui| {
            egui::Frame::new().inner_margin(12).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(162.0);
                        ui.set_min_height(245.0);
                        edited |= self.fields(ui, p, &mut draft);
                    });
                    ui.add_space(4.0);
                    edited |= self.canvas(ui, p, &mut draft);
                });
                ui.add_space(2.0);
                self.previews(ui, p, &draft);
                ui.add_space(2.0);
                edited |= self.palette(ui, p, prefs, &mut draft);
                ui.label(RichText::new(aurora_i18n::tr!("color-picker-save-hint")).size(11.0).color(p.muted));
                ui.horizontal(|ui| {
                    let label = RichText::new(aurora_i18n::tr!("color-picker-pipette")).size(12.0).color(p.ink);
                    let button = if let Some(icon) = super::icons::global("eyedropper") {
                        egui::Button::image_and_text(egui::Image::new((icon.id(), Vec2::splat(16.0))).tint(p.ink), label)
                    } else {
                        egui::Button::new(label)
                    };
                    let pipette = ui.add(button.selected(self.pipette).min_size(Vec2::new(92.0, 28.0)));
                    self.record(Control::Pipette, pipette.rect);
                    if pipette.on_hover_text(aurora_i18n::tr!("color-picker-pipette-hint")).clicked() {
                        self.pipette = !self.pipette;
                        self.sample_cursor = None;
                        self.sampled = None;
                        self.notice.clear();
                    }
                    let copy = widgets::flat_button(ui, p, aurora_i18n::tr!("color-picker-copy-lsl"));
                    self.record(Control::CopyLsl, copy.rect);
                    if copy.clicked() {
                        ctx.copy_text(lsl(draft.rgb));
                        self.notice = aurora_i18n::tr!("color-picker-copied").to_owned();
                    }
                });
                let status = if self.pipette && self.notice.is_empty() {
                    aurora_i18n::tr!("color-picker-pipette-hint")
                } else {
                    &self.notice
                };
                ui.add_sized(
                    [ui.available_width(), 18.0],
                    egui::Label::new(RichText::new(status).size(11.0).color(p.muted)),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    let apply = ui.checkbox(&mut prefs.apply, aurora_i18n::tr!("color-picker-apply"));
                    self.record(Control::Apply, apply.rect);
                    apply.on_hover_text(aurora_i18n::tr!("color-picker-apply-hint"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let cancel = widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(86.0, 30.0));
                        self.record(Control::Cancel, cancel.rect);
                        canceled = cancel.clicked();
                        ui.add_enabled_ui(parse_hex(&draft.hex).is_some(), |ui| {
                            let ok = widgets::primary_button_sized(ui, p, aurora_i18n::tr!("color-picker-ok"), Vec2::new(72.0, 30.0));
                            self.record(Control::Accept, ok.rect);
                            accepted = ok.clicked();
                        });
                    });
                });
            });
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.pipette {
                self.pipette = false;
                self.sampled = None;
            } else {
                canceled = true;
            }
        }
        let rgb = draft.rgb;
        if !open || canceled {
            self.close();
            Some(Event::Cancel)
        } else if accepted {
            self.close();
            Some(Event::Accept(rgb))
        } else {
            let modified = rgb != draft.original;
            self.draft = Some(draft);
            (prefs.apply && (edited || !apply_before && modified)).then_some(Event::Preview(rgb))
        }
    }

    fn fields(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Draft) -> bool {
        let mode_before = draft.mode;
        ui.horizontal(|ui| {
            for (mode, label) in [aurora_i18n::tr!("color-picker-rgb"), "LSL", aurora_i18n::tr!("color-picker-hex")]
                .into_iter()
                .enumerate()
            {
                let response = widgets::underlined_button(ui, p, label, draft.mode == mode);
                self.record(Control::Mode(mode), response.rect);
                if response.clicked() {
                    draft.mode = mode;
                }
            }
        });
        if draft.mode != mode_before {
            draft.hex = hex(draft.rgb);
        }
        let mut edited = false;
        if draft.mode == 2 {
            ui.label(RichText::new(aurora_i18n::tr!("color-picker-hex-label")).size(12.0).color(p.muted));
            let field = ui.add(
                egui::TextEdit::singleline(&mut draft.hex)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(136.0)
                    .char_limit(7),
            );
            self.record(Control::Hex, field.rect);
            if field.changed()
                && let Some(rgb) = parse_hex(&draft.hex)
            {
                // Keep the field's text/caret while the user types.
                let text = draft.hex.clone();
                edited |= draft.set_rgb(rgb);
                draft.hex = text;
            }
            if parse_hex(&draft.hex).is_none() {
                ui.label(RichText::new(aurora_i18n::tr!("color-picker-invalid-hex")).size(11.0).color(p.warn));
            }
            ui.add_space(39.0);
        } else {
            let lsl = draft.mode == 1;
            let mut rgb = if lsl { draft.rgb } else { draft.rgb.map(|v| v * 255.0) };
            let mut changed = false;
            for (i, label) in [
                aurora_i18n::tr!("color-picker-red"),
                aurora_i18n::tr!("color-picker-green"),
                aurora_i18n::tr!("color-picker-blue"),
            ]
            .into_iter()
            .enumerate()
            {
                ui.horizontal(|ui| {
                    ui.add_sized([78.0, 26.0], egui::Label::new(RichText::new(label).size(12.0).color(p.muted)));
                    let response = number(
                        ui,
                        &mut rgb[i],
                        if lsl { 1.0 } else { 255.0 },
                        if lsl { 0.01 } else { 1.0 },
                        if lsl { 3 } else { 0 },
                    );
                    self.record(Control::Rgb(i), response.rect);
                    changed |= response.changed();
                });
            }
            if changed {
                edited |= draft.set_rgb(if lsl { rgb } else { rgb.map(|v| v / 255.0) });
            }
        }
        ui.add_space(3.0);
        ui.separator();
        let mut hsl = [draft.hsl[0] * 360.0, draft.hsl[1] * 100.0, draft.hsl[2] * 100.0];
        let mut changed = false;
        for (i, label) in [
            aurora_i18n::tr!("color-picker-hue"),
            aurora_i18n::tr!("color-picker-saturation"),
            aurora_i18n::tr!("color-picker-lightness"),
        ]
        .into_iter()
        .enumerate()
        {
            ui.horizontal(|ui| {
                ui.add_sized([78.0, 26.0], egui::Label::new(RichText::new(label).size(12.0).color(p.muted)));
                let response = number(ui, &mut hsl[i], if i == 0 { 360.0 } else { 100.0 }, 1.0, 1);
                self.record(Control::Hsl(i), response.rect);
                changed |= response.changed();
            });
        }
        if changed {
            edited |= draft.set_hsl([hsl[0] / 360.0, hsl[1] / 100.0, hsl[2] / 100.0]);
        }
        edited
    }

    fn canvas(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Draft) -> bool {
        let height = 230.0;
        let width = (ui.available_width() - 26.0).max(100.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        self.record(Control::Grid, rect);
        let mut mesh = egui::Mesh::default();
        // HSL at L=0.5 is piecewise bilinear in H/S: fourteen vertices are
        // sufficient for the full plane, with no image generation or upload.
        for col in 0..=6 {
            let h = col as f32 / 6.0;
            for (y, s) in [(rect.top(), 1.0), (rect.bottom(), 0.0)] {
                mesh.colored_vertex(Pos2::new(rect.left() + h * width, y), color(hsl_to_rgb([h, s, 0.5])));
            }
        }
        for col in 0..6 {
            let i = col * 2;
            mesh.add_triangle(i, i + 2, i + 1);
            mesh.add_triangle(i + 1, i + 2, i + 3);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        let mut hsl = draft.hsl;
        let mut edited = false;
        if (response.dragged() || response.is_pointer_button_down_on() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            hsl[0] = ((pos.x - rect.left()) / width).clamp(0.0, 1.0);
            hsl[1] = (1.0 - (pos.y - rect.top()) / height).clamp(0.0, 1.0);
            edited |= draft.set_hsl(hsl) || self.mixed;
        }
        let marker = Pos2::new(rect.left() + draft.hsl[0] * width, rect.bottom() - draft.hsl[1] * height);
        ui.painter().circle_stroke(marker, 6.0, Stroke::new(3.0, p.field));
        ui.painter().circle_stroke(marker, 6.0, Stroke::new(1.0, p.ink));
        ui.painter()
            .rect_stroke(rect, 2.0, Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
        let (bar, response) = ui.allocate_exact_size(Vec2::new(18.0, height), Sense::click_and_drag());
        self.record(Control::Lightness, bar);
        let mut mesh = egui::Mesh::default();
        for (fraction, light) in [(0.0, 1.0), (0.5, 0.5), (1.0, 0.0)] {
            let c = color(hsl_to_rgb([draft.hsl[0], draft.hsl[1], light]));
            mesh.colored_vertex(Pos2::new(bar.left(), bar.top() + fraction * height), c);
            mesh.colored_vertex(Pos2::new(bar.right(), bar.top() + fraction * height), c);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(1, 3, 2);
        mesh.add_triangle(2, 3, 4);
        mesh.add_triangle(3, 5, 4);
        ui.painter().add(egui::Shape::mesh(mesh));
        if (response.dragged() || response.is_pointer_button_down_on() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let mut hsl = draft.hsl;
            hsl[2] = (1.0 - (pos.y - bar.top()) / height).clamp(0.0, 1.0);
            edited |= draft.set_hsl(hsl) || self.mixed;
        }
        let y = bar.bottom() - draft.hsl[2] * height;
        ui.painter().hline(bar.x_range(), y, Stroke::new(3.0, p.field));
        ui.painter().hline(bar.x_range(), y, Stroke::new(1.0, p.ink));
        ui.painter()
            .rect_stroke(bar, 2.0, Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
        edited
    }

    fn previews(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &Draft) {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 8.0) * 0.5;
            for (rgb, caption, current) in [
                (
                    draft.original,
                    if self.mixed {
                        aurora_i18n::tr!("color-picker-mixed")
                    } else {
                        aurora_i18n::tr!("color-picker-original")
                    },
                    false,
                ),
                (draft.rgb, aurora_i18n::tr!("color-picker-current"), true),
            ] {
                ui.vertical(|ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(width, 32.0), if current { Sense::drag() } else { Sense::hover() });
                    if !current && self.mixed {
                        // A mixed selection has no single original swatch.
                        ui.painter().rect_filled(rect, 3.0, p.field);
                        for col in 0..(width / 8.0).ceil() as usize {
                            for row in 0..4 {
                                if (col + row) % 2 == 0 {
                                    let tile =
                                        Rect::from_min_size(rect.min + egui::vec2(col as f32 * 8.0, row as f32 * 8.0), Vec2::splat(8.0));
                                    ui.painter().rect_filled(tile.intersect(rect.shrink(1.0)), 0.0, p.raised);
                                }
                            }
                        }
                    } else {
                        ui.painter().rect_filled(rect, 3.0, color(rgb));
                    }
                    ui.painter()
                        .rect_stroke(rect, 3.0, Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
                    if current {
                        self.record(Control::Current, rect);
                        response.dnd_set_drag_payload(draft.rgb);
                    } else if self.mixed {
                        response.on_hover_text(aurora_i18n::tr!("color-picker-original-mixed"));
                    }
                    ui.label(RichText::new(caption).size(11.0).color(p.muted));
                });
            }
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences, draft: &mut Draft) -> bool {
        let gap = 4.0;
        let width = (ui.available_width() - gap * 15.0) / 16.0;
        let mut edited = false;
        for row in 0..2 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for col in 0..16 {
                    let slot = row * 16 + col;
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 21.0), Sense::click());
                    self.record(Control::Slot(slot), rect);
                    ui.painter().rect_filled(rect, 2.0, color(prefs.palette[slot]));
                    let dropping = response.dnd_hover_payload::<Rgb>().is_some();
                    ui.painter().rect_stroke(
                        rect,
                        2.0,
                        Stroke::new(if dropping { 2.0 } else { 1.0 }, if dropping { p.violet } else { p.raised }),
                        egui::StrokeKind::Inside,
                    );
                    if let Some(rgb) = response.dnd_release_payload::<Rgb>() {
                        prefs.store(slot, *rgb);
                        self.notice = aurora_i18n::tr!("color-picker-saved").to_owned();
                    } else if response.clicked() {
                        draft.set_rgb(prefs.palette[slot]);
                        edited = true;
                    }
                    response.context_menu(|ui| {
                        if ui.button(aurora_i18n::tr!("color-picker-save-slot")).clicked() {
                            prefs.store(slot, draft.rgb);
                            self.notice = aurora_i18n::tr!("color-picker-saved").to_owned();
                            ui.close();
                        }
                    });
                    response.on_hover_text(format!("#{}", hex(prefs.palette[slot])));
                }
            });
        }
        edited
    }
}

fn number(ui: &mut egui::Ui, value: &mut f32, max: f32, speed: f32, decimals: usize) -> egui::Response {
    let modifiers = ui.input(|i| i.modifiers);
    let speed = speed
        * 0.25
        * if modifiers.alt {
            10.0
        } else if modifiers.ctrl {
            0.1
        } else if modifiers.shift {
            0.01
        } else {
            1.0
        };
    ui.add_sized(
        [76.0, 26.0],
        egui::DragValue::new(value)
            .range(0.0..=max)
            .speed(speed)
            .min_decimals(decimals)
            .max_decimals(decimals)
            .custom_parser(aurora_i18n::parse_number),
    )
}

pub fn color(rgb: Rgb) -> Color32 {
    let [r, g, b] = bytes(rgb);
    Color32::from_rgb(r, g, b)
}
