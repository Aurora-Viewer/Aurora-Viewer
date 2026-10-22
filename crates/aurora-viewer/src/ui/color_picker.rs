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

const WINDOW_ID: &str = "build_color_picker";
// LLDragHandle::sSnapMargin / SnapMargin (Firestorm settings.xml).
const SNAP_MARGIN: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Preview(Rgb),
    Accept(Rgb),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Control {
    Origin,
    Title,
    ParentTitle,
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
    window_id: String,
    draft: Option<Draft>,
    mixed: bool,
    pub pipette: bool,
    pub sampled: Option<Uuid>,
    sample_cursor: Option<(f32, f32)>,
    pending_preview: bool,
    notice: String,
    track_controls: bool,
    parent: Option<egui::Id>,
    parent_rect: Option<Rect>,
    screen_rect: Option<Rect>,
    opening: bool,
    origin: Option<Rect>,
    dock: Option<DockSide>,
    dragging: bool,
    pub(crate) controls: HashMap<Control, Rect>,
}

impl ColorPicker {
    pub fn named(window_id: String) -> Self {
        Self {
            window_id,
            ..Self::default()
        }
    }

    fn window_id(&self) -> &str {
        if self.window_id.is_empty() { WINDOW_ID } else { &self.window_id }
    }
    pub fn open(&mut self, rgb: Rgb, mixed: bool) {
        self.draft = Some(Draft::new(rgb));
        self.mixed = mixed;
        self.pipette = false;
        self.sampled = None;
        self.sample_cursor = None;
        self.pending_preview = false;
        self.notice.clear();
        self.parent = None;
        self.parent_rect = None;
        self.screen_rect = None;
        self.opening = true;
        self.origin = None;
        self.dock = None;
        self.dragging = false;
        self.track_controls = cfg!(test) || demo::mode().is_some();
    }

    /// Open beside the invoking floater and follow its subsequent movements.
    pub fn open_from(&mut self, rgb: Rgb, mixed: bool, parent: egui::Id) {
        self.open(rgb, mixed);
        self.parent = Some(parent);
        self.dock = Some(DockSide::Right);
    }

    /// The invoking swatch supplies its visible rectangle on every layout pass.
    pub fn set_origin(&mut self, rect: Rect) {
        self.origin = Some(rect);
    }

    pub fn is_open(&self) -> bool {
        self.draft.is_some()
    }

    pub fn close(&mut self) {
        self.draft = None;
        self.pipette = false;
        self.sampled = None;
        self.pending_preview = false;
        self.parent = None;
        self.parent_rect = None;
        self.screen_rect = None;
        self.origin = None;
        self.dock = None;
        self.dragging = false;
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

    pub(crate) fn track_controls(&mut self) {
        self.track_controls = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, prefs: &mut Preferences) -> Option<Event> {
        let window_id = self.window_id().to_owned();
        let window_id = window_id.as_str();
        let mut draft = self.draft.take()?;
        prefs.normalize();
        self.controls.clear();
        let apply_before = prefs.apply;
        let mut edited = std::mem::take(&mut self.pending_preview);
        let mut accepted = false;
        let mut canceled = false;
        let mut open = true;
        let screen = ctx.content_rect();
        let size = Vec2::new((screen.width() - 32.0).clamp(360.0, 460.0), 420.0);
        let parent = self.parent.and_then(|id| ctx.memory(|m| m.area_rect(id)));
        let moving = ctx.is_being_dragged(egui::Id::unique(window_id).with("move"));
        if moving {
            // LLFloater::translate only moves dependents still snapped to it.
            // Keep the color owner for the context cone, even while detached.
            self.dock = None;
        }
        let following = self.dock.is_some() && (self.opening || parent != self.parent_rect || self.screen_rect != Some(screen));
        if let Some(parent) = parent.filter(|_| following) {
            let height = ctx
                .memory(|m| m.area_rect(egui::Id::unique(window_id)))
                .map_or(size.y, |rect| rect.height());
            let measured = Vec2::new(size.x, height);
            let (pos, side) = adjacent_position(screen, parent, measured, self.dock.unwrap_or(DockSide::Right));
            self.dock = Some(side);
            if self.opening {
                widgets::place_floater(ctx, window_id, pos, size);
            } else {
                widgets::move_floater(ctx, window_id, pos);
            }
        }
        Floater::new(
            window_id,
            aurora_i18n::tr!("color-picker-title"),
            screen.center() - size * 0.5,
            size,
        )
        .fixed()
        .auto_height()
        .framed_header("palette-filled")
        .help(aurora_i18n::tr!("color-picker-help"))
        .show(ctx, p, &mut open, |ui| {
            egui::Frame::new().inner_margin(10).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(160.0);
                        ui.set_min_height(220.0);
                        edited |= self.fields(ui, p, &mut draft);
                    });
                    ui.add_space(2.0);
                    edited |= self.canvas(ui, p, &mut draft);
                });
                ui.add_space(2.0);
                self.previews(ui, p, &draft);
                ui.add_space(2.0);
                edited |= self.palette(ui, p, prefs, &mut draft);
                ui.label(RichText::new(aurora_i18n::tr!("color-picker-save-hint")).size(11.0).color(p.muted));
                ui.horizontal(|ui| {
                    let label = RichText::new(aurora_i18n::tr!("color-picker-pipette")).size(12.0).color(p.ink);
                    let button = if let Some(icon) = super::icons::global("color-picker") {
                        egui::Button::image_and_text(egui::Image::new((icon.id(), Vec2::splat(16.0))).tint(p.ink), label)
                    } else {
                        egui::Button::new(label)
                    };
                    let pipette = ui.add(button.selected(self.pipette).min_size(Vec2::new(88.0, 26.0)));
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
                    [ui.available_width(), 16.0],
                    egui::Label::new(RichText::new(status).size(11.0).color(p.muted)),
                );
                widgets::divider(ui, p);
                ui.horizontal(|ui| {
                    let apply = widgets::checkbox(ui, p, &mut prefs.apply, aurora_i18n::tr!("color-picker-apply"));
                    self.record(Control::Apply, apply.rect);
                    apply.on_hover_text(aurora_i18n::tr!("color-picker-apply-hint"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let cancel = widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(80.0, 28.0));
                        self.record(Control::Cancel, cancel.rect);
                        canceled = cancel.clicked();
                        ui.add_enabled_ui(parse_hex(&draft.hex).is_some(), |ui| {
                            let ok = widgets::primary_button_sized(ui, p, aurora_i18n::tr!("color-picker-ok"), Vec2::new(68.0, 28.0));
                            self.record(Control::Accept, ok.rect);
                            accepted = ok.clicked();
                        });
                    });
                });
            });
        });
        if let Some(parent) = parent.filter(|_| following)
            && let Some(rect) = ctx.memory(|m| m.area_rect(egui::Id::unique(window_id)))
        {
            // Auto-height is known after the first layout. Correct the screen
            // constraint in another pass so opening near the bottom stays visible.
            let (pos, side) = adjacent_position(screen, parent, rect.size(), self.dock.unwrap_or(DockSide::Right));
            self.dock = Some(side);
            if rect.min.distance(pos) > 0.5 {
                widgets::move_floater(ctx, window_id, pos);
                ctx.request_discard("position color picker beside its parent");
            }
        }
        if self.dragging
            && !moving
            && let Some(parent) = parent
            && let Some(rect) = ctx.memory(|m| m.area_rect(egui::Id::unique(window_id)))
            && let Some(side) = nearby_snap(screen, parent, rect)
        {
            self.dock = Some(side);
            let (pos, _) = adjacent_position(screen, parent, rect.size(), side);
            widgets::move_floater(ctx, window_id, pos);
            ctx.request_discard("snap color picker to its parent");
        }
        self.dragging = moving;
        self.opening = false;
        self.parent_rect = parent;
        self.screen_rect = Some(screen);
        self.draw_owner_link(ctx, p);
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

    /// LLFloater::drawConeToOwner / LLFloaterColorPicker::draw (Firestorm,
    /// originally LGPL 2.1): fade a context cone while moving the dialog.
    fn draw_owner_link(&mut self, ctx: &egui::Context, p: &Palette) {
        let id = egui::Id::unique(self.window_id());
        if let Some(rect) = ctx.memory(|m| m.area_rect(id)) {
            self.record(Control::Title, Rect::from_min_size(rect.min, Vec2::new(rect.width() - 100.0, 40.0)));
        }
        if let Some(rect) = self.parent_rect {
            self.record(
                Control::ParentTitle,
                Rect::from_min_size(rect.min, Vec2::new(rect.width() - 100.0, 40.0)),
            );
        }
        let moving = ctx.is_being_dragged(id.with("move"));
        let opacity = ctx.animate_bool_with_time(id.with("owner_link"), moving, 0.08);
        if let Some(origin) = self.origin.take() {
            self.record(Control::Origin, origin);
            if opacity <= 0.001 {
                return;
            }
            let Some(window) = ctx.memory(|m| m.area_rect(id)) else {
                return;
            };
            let mesh = owner_link(origin, window, p.bar.gamma_multiply(0.4 * opacity));
            ctx.layer_painter(egui::LayerId::background()).add(egui::Shape::mesh(mesh));
            if let Some(parent) = self.parent {
                ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, parent)).rect_stroke(
                    origin.expand(2.0),
                    ctx.global_style().visuals.window_corner_radius,
                    Stroke::new(1.5, p.violet.gamma_multiply(opacity)),
                    egui::StrokeKind::Outside,
                );
            }
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
            let field = ui
                .horizontal(|ui| {
                    ui.add_sized([14.0, 23.0], egui::Label::new(RichText::new("#").monospace().color(p.muted)));
                    ui.add_sized(
                        [120.0, 23.0],
                        egui::TextEdit::singleline(&mut draft.hex)
                            .font(egui::TextStyle::Monospace)
                            .align(egui::Align2::LEFT_CENTER)
                            .desired_width(120.0)
                            .char_limit(7),
                    )
                })
                .inner;
            self.record(Control::Hex, field.rect);
            if field.changed() {
                // The prefix is drawn separately, including for pasted #RRGGBB.
                if draft.hex.starts_with('#') {
                    draft.hex.remove(0);
                }
                if let Some(rgb) = parse_hex(&draft.hex) {
                    // Keep the field's text/caret while the user types.
                    let text = draft.hex.clone();
                    edited |= draft.set_rgb(rgb);
                    draft.hex = text;
                }
            }
            if parse_hex(&draft.hex).is_none() {
                ui.label(RichText::new(aurora_i18n::tr!("color-picker-invalid-hex")).size(11.0).color(p.warn));
            }
            ui.add_space(32.0);
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
                    ui.add_sized([78.0, 23.0], egui::Label::new(RichText::new(label).size(12.0).color(p.muted)));
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
        widgets::divider(ui, p);
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
                ui.add_sized([78.0, 23.0], egui::Label::new(RichText::new(label).size(12.0).color(p.muted)));
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
        let height = 210.0;
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
        ui.painter().rect_stroke(
            rect,
            ui.visuals().window_corner_radius,
            Stroke::new(1.0, p.raised),
            egui::StrokeKind::Inside,
        );
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
        ui.painter().rect_stroke(
            bar,
            ui.visuals().window_corner_radius,
            Stroke::new(1.0, p.raised),
            egui::StrokeKind::Inside,
        );
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
                        ui.allocate_exact_size(Vec2::new(width, 26.0), if current { Sense::drag() } else { Sense::hover() });
                    if !current && self.mixed {
                        // A mixed selection has no single original swatch.
                        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
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
                        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, color(rgb));
                    }
                    ui.painter().rect_stroke(
                        rect,
                        ui.visuals().window_corner_radius,
                        Stroke::new(1.0, p.raised),
                        egui::StrokeKind::Inside,
                    );
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
        let gap = 3.0;
        let width = (ui.available_width() - gap * 15.0) / 16.0;
        let mut edited = false;
        for row in 0..2 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for col in 0..16 {
                    let slot = row * 16 + col;
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 19.0), Sense::click());
                    self.record(Control::Slot(slot), rect);
                    ui.painter()
                        .rect_filled(rect, ui.visuals().window_corner_radius, color(prefs.palette[slot]));
                    let dropping = response.dnd_hover_payload::<Rgb>().is_some();
                    ui.painter().rect_stroke(
                        rect,
                        ui.visuals().window_corner_radius,
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
                    super::menu::context_menu(&response, p, |ui| {
                        if super::menu::item(ui, p, "device-floppy-filled", aurora_i18n::tr!("color-picker-save-slot")) {
                            prefs.store(slot, draft.rgb);
                            self.notice = aurora_i18n::tr!("color-picker-saved").to_owned();
                        }
                    });
                    response.on_hover_text(format!("#{}", hex(prefs.palette[slot])));
                }
            });
        }
        edited
    }
}

/// A context cone shades the gap between the visible swatch and dialog edges.
fn owner_link(origin: Rect, window: Rect, shade: Color32) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    if origin.intersects(window) {
        return mesh;
    }
    let (from, to) = if window.left() >= origin.right() {
        (
            [origin.right_top(), origin.right_bottom()],
            [window.left_top(), window.left_bottom()],
        )
    } else if window.right() <= origin.left() {
        (
            [origin.left_top(), origin.left_bottom()],
            [window.right_top(), window.right_bottom()],
        )
    } else if window.top() >= origin.bottom() {
        (
            [origin.left_bottom(), origin.right_bottom()],
            [window.left_top(), window.right_top()],
        )
    } else {
        (
            [origin.left_top(), origin.right_top()],
            [window.left_bottom(), window.right_bottom()],
        )
    };
    for pos in from {
        mesh.colored_vertex(pos, Color32::TRANSPARENT);
    }
    for pos in to {
        mesh.colored_vertex(pos, shade);
    }
    mesh.add_triangle(0, 2, 1);
    mesh.add_triangle(1, 2, 3);
    mesh
}

/// Keep the chosen edge while it fits; otherwise switch sides and clamp to
/// the screen when neither edge or the parent's top can fit the whole dialog.
fn adjacent_position(screen: Rect, parent: Rect, size: Vec2, preferred: DockSide) -> (Pos2, DockSide) {
    let left = parent.left() - size.x;
    let right = parent.right();
    let fits = |x: f32| x >= screen.left() && x + size.x <= screen.right();
    let side = match preferred {
        DockSide::Right if !fits(right) => DockSide::Left,
        DockSide::Left if !fits(left) && fits(right) => DockSide::Right,
        _ => preferred,
    };
    let x = if side == DockSide::Right { right } else { left };
    (
        Pos2::new(
            x.clamp(screen.left(), (screen.right() - size.x).max(screen.left())),
            parent.top().clamp(screen.top(), (screen.bottom() - size.y).max(screen.top())),
        ),
        side,
    )
}

/// Only a released dialog near a real adjacent position becomes dependent
/// again; screen clamping alone must not create an overlapping snap target.
fn nearby_snap(screen: Rect, parent: Rect, window: Rect) -> Option<DockSide> {
    [DockSide::Right, DockSide::Left].into_iter().find(|&side| {
        let x = if side == DockSide::Right {
            parent.right()
        } else {
            parent.left() - window.width()
        };
        let y = parent
            .top()
            .clamp(screen.top(), (screen.bottom() - window.height()).max(screen.top()));
        x >= screen.left()
            && x + window.width() <= screen.right()
            && (window.left() - x).abs() <= SNAP_MARGIN
            && (window.top() - y).abs() <= SNAP_MARGIN
    })
}

fn number(ui: &mut egui::Ui, value: &mut f32, max: f32, speed: f32, decimals: usize) -> egui::Response {
    let modifiers = ui.input(|i| i.modifiers);
    let increment = speed;
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
    let (mut response, step) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let response = ui.add_sized(
                [63.0, 23.0],
                egui::DragValue::new(&mut *value)
                    .range(0.0..=max)
                    .speed(speed)
                    .min_decimals(decimals)
                    .max_decimals(decimals)
                    .custom_parser(aurora_i18n::parse_number),
            );
            (response, widgets::step_buttons(ui, 23.0))
        })
        .inner;
    if step != 0 {
        *value = (*value + increment * step as f32).clamp(0.0, max);
        response.mark_changed();
    }
    response
}

pub fn color(rgb: Rgb) -> Color32 {
    let [r, g, b] = bytes(rgb);
    Color32::from_rgb(r, g, b)
}
