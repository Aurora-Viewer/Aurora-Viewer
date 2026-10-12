//! Firestorm-style building blocks: floaters with a slim title bar
//! (title, ?, —, ✕), flat tabs, column headers and status dots.

use crate::theme::Palette;
use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Vec2};

#[cfg(test)]
mod collapse_tests;
pub mod demo;
#[cfg(test)]
pub(crate) mod elide_tests;
mod resize;
mod workspace;
pub use workspace::{Modal, glyph, navigation, section_header, stat, surface};
pub(crate) use workspace::{inventory_item_label, inventory_spacing};

/// Keep status suffixes at normal weight and slightly darker than the name.
pub(crate) fn name_with_suffix(name: &str, suffix: &str, font_id: egui::FontId, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(name, 0.0, egui::TextFormat::simple(font_id.clone(), color));
    job.append(suffix, 0.0, egui::TextFormat::simple(font_id, color.gamma_multiply(0.85)));
    job
}

/// A one-line label at most `max_width` wide: a longer text ends with « … »,
/// as the texts Firestorm declares with use_ellipses="true" do
/// (LLFontGL::render), and shows whole on hover (egui's tooltip of an elided
/// label; Firestorm adds it to its truncated tab labels,
/// LLTabContainer::addLabelToolTip). It takes the room of its text only, so
/// what follows in a row stays next to it. For names of residents, groups
/// and conversations, whose length is not ours.
///
/// The galley comes from egui's layout cache: an unchanged text costs a
/// lookup per frame, not a layout.
pub(crate) fn elided_label(ui: &egui::Ui, text: impl Into<egui::WidgetText>, max_width: f32) -> egui::Label {
    egui::Label::new(elided_galley(ui, text, max_width))
}

/// The same for a row painted by hand: `text` starts at `left_center` and is
/// cut at `width`. Returns true when it was cut, for the caller to show the
/// whole text on hover (LLTabContainer::addLabelToolTip, FIRE-36603: only a
/// truncated label gets the tooltip).
pub(crate) fn paint_elided(ui: &egui::Ui, text: &str, left_center: egui::Pos2, width: f32, font: egui::FontId, color: Color32) -> bool {
    if text.is_empty() {
        return false;
    }
    // less room than the ellipsis itself: nothing readable to paint
    if width < font.size {
        return true;
    }
    let galley = elided_galley(ui, RichText::new(text).font(font).color(color), width);
    let pos = egui::pos2(left_center.x, left_center.y - galley.size().y * 0.5);
    let cut = galley.elided;
    ui.painter().galley(pos, galley, color);
    cut
}

fn elided_galley(ui: &egui::Ui, text: impl Into<egui::WidgetText>, max_width: f32) -> std::sync::Arc<egui::Galley> {
    text.into()
        .into_galley(ui, Some(egui::TextWrapMode::Truncate), max_width.max(0.0), egui::TextStyle::Body)
}

/// LLFontGL BOLD_OFFSET: egui's strong text changes color, not weight.
pub(crate) fn bold_text_offset(ui: &egui::Ui) -> Vec2 {
    let ppp = ui.ctx().pixels_per_point();
    Vec2::new(ppp.round().max(1.0) / ppp, 0.0)
}

pub struct Floater<'a> {
    pub id: &'a str,
    pub title: String,
    pub default_pos: egui::Pos2,
    pub default_size: Vec2,
    pub min_size: Vec2,
    pub resizable: bool,
    pub help: Option<&'a str>,
    /// Extra title bar button: (Tabler icon, tooltip, set when clicked).
    pub action: Option<(&'a str, &'a str, &'a mut bool)>,
    /// No open / close sounds (sound_flags="0", e.g. floater_tools).
    pub silent: bool,
    /// An inset dark title bar and edge-to-edge body for inspector layouts.
    pub framed_header: Option<&'a str>,
    /// Keep an inspector's width stable while its height follows the content.
    pub auto_height: bool,
    /// Put a toolbar directly beneath the title, without a separator layout row.
    pub compact_header: bool,
}

impl<'a> Floater<'a> {
    pub fn new(id: &'a str, title: impl Into<String>, pos: egui::Pos2, size: Vec2) -> Floater<'a> {
        Floater {
            id,
            title: title.into(),
            default_pos: pos,
            default_size: size,
            min_size: Vec2::new(220.0, 120.0),
            resizable: true,
            help: None,
            action: None,
            silent: false,
            framed_header: None,
            auto_height: false,
            compact_header: false,
        }
    }

    pub fn fixed(mut self) -> Self {
        self.resizable = false;
        self
    }

    pub fn help(mut self, text: &'a str) -> Self {
        self.help = Some(text);
        self
    }

    /// Add a title bar button left of « Réduire »; `clicked` is set when pressed.
    pub fn action(mut self, icon: &'a str, tip: &'a str, clicked: &'a mut bool) -> Self {
        self.action = Some((icon, tip, clicked));
        self
    }

    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    pub fn framed_header(mut self, icon: &'a str) -> Self {
        self.framed_header = Some(icon);
        self
    }

    pub fn auto_height(mut self) -> Self {
        self.auto_height = true;
        self.resizable = false;
        self
    }

    pub fn compact_header(mut self) -> Self {
        self.compact_header = true;
        self
    }

    /// Show the floater; `open` is cleared by the close button.
    pub fn show<R>(mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
        if !*open {
            return None;
        }
        let min_id = egui::Id::new((self.id, "minimized"));
        // persisted with the UI layout (ui_layout.ron)
        let minimized = ctx.data_mut(|d| d.get_persisted::<bool>(min_id)).unwrap_or(false);
        if !self.silent {
            super::sound_cues::floater_shown(ctx, egui::Id::new(self.id));
        }
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(
                p.panel.r(),
                p.panel.g(),
                p.panel.b(),
                p.panel_alpha,
            ))
            .stroke(Stroke::new(1.0, p.muted_dim.gamma_multiply(0.6)))
            .corner_radius(ctx.global_style().visuals.window_corner_radius)
            .shadow(ctx.global_style().visuals.window_shadow)
            .inner_margin(if self.framed_header.is_some() {
                egui::Margin::ZERO
            } else {
                egui::Margin {
                    left: 10,
                    right: 10,
                    top: if minimized { 4 } else { 5 },
                    bottom: if minimized { 4 } else { 10 },
                }
            });
        let mut close = false;
        let mut toggle_min = false;
        let mut out = None;
        let window_id = egui::Id::new(self.id);
        let expanded_id = window_id.with("expanded_size");
        let restore_id = window_id.with("restore_size");
        let expanded = ctx.data_mut(|d| d.get_persisted::<Vec2>(expanded_id)).unwrap_or(self.default_size);
        let restore = ctx.data_mut(|d| d.remove_temp::<bool>(restore_id)).unwrap_or(false);
        let resizable = self.resizable && !minimized;
        let resize_drag = resizable.then(|| resize::Drag::prepare(ctx, window_id, self.min_size)).flatten();
        // Our handles clamp both geometry and position at the minimum, while
        // Window retains its normal movement and persisted layout.
        let mut w = egui::Window::new(&self.title)
            .id(window_id)
            .title_bar(false)
            .fade_in(false)
            .fade_out(false)
            .frame(frame)
            .default_pos(self.default_pos)
            .resizable(false);
        if self.auto_height && !minimized {
            w = w.auto_sized().min_width(self.default_size.x).max_width(self.default_size.x);
        } else if self.resizable && !minimized {
            w = w.default_size(self.default_size).min_size(self.min_size);
        } else if !minimized {
            w = w.default_width(self.default_size.x);
        }
        // LLFloater::setMinimized keeps the expanded geometry. egui's Resize
        // otherwise retains the old height when the body disappears, and then
        // forgets it when clamped to the header. Restore that size once.
        // Aurora keeps the dragged bar's current position when restoring.
        if minimized {
            w = w.fixed_size(Vec2::new(expanded.x, if self.framed_header.is_some() { 40.0 } else { 38.0 }));
        } else if restore && !self.auto_height {
            w = w.fixed_size(expanded);
        }
        if let Some(pos) = ctx.data_mut(|d| d.remove_temp::<egui::Pos2>(window_id.with(PLACE_POS))) {
            w = w.current_pos(pos);
        }
        if let Some(drag) = resize_drag {
            let rect = drag.rect();
            w = w.fixed_size(rect.size()).current_pos(rect.min);
        }
        let mut glass_slot = None;
        let response = w.show(ctx, |ui| {
            glass_slot = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
            // Our custom resize handles replace egui's native resizability.
            // Reserve the requested content area as a resizable Window would:
            // height-filling scroll areas otherwise shave a few pixels off
            // each restoration even though Resize remembered the right size.
            if resizable || (restore && !self.auto_height) {
                ui.set_min_size(ui.available_size());
            }
            // title bar
            let mut buttons_left = f32::INFINITY;
            let framed_header = self.framed_header;
            if framed_header.is_some() {
                ui.spacing_mut().item_spacing.y = 0.0;
            }
            let mut header_content = |ui: &mut egui::Ui| {
                if framed_header.is_some() {
                    ui.spacing_mut().interact_size.y = 20.0;
                }
                ui.horizontal(|ui| {
                    ui.set_min_height(if framed_header.is_some() { 20.0 } else { 28.0 });
                    let title = RichText::new(&self.title).size(14.0).strong().color(p.ink);
                    // a fixed floater is as wide as its title; a resizable one
                    // cuts a long title short before the buttons (LLFloater
                    // titles end with an ellipsis)
                    let truncate = self.resizable || minimized || self.auto_height;
                    if !truncate {
                        glyph(ui, workspace::window_icon(self.id), p.violet_light, 18.0);
                        ui.label(title.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if title_button(ui, p, "x").on_hover_text(aurora_i18n::tr!("common-close")).clicked() {
                            close = true;
                        }
                        if title_button(ui, p, "minus")
                            .on_hover_text(if minimized {
                                aurora_i18n::tr!("common-expand")
                            } else {
                                aurora_i18n::tr!("common-collapse")
                            })
                            .clicked()
                        {
                            toggle_min = true;
                        }
                        if let Some((icon, tip, clicked)) = self.action.as_mut()
                            && title_button(ui, p, icon).on_hover_text(*tip).clicked()
                        {
                            **clicked = true;
                        }
                        if let Some(h) = self.help {
                            title_button(ui, p, "question-mark").on_hover_text(h);
                        }
                        buttons_left = ui.cursor().right();
                        if truncate {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                {
                                    let icon = framed_header.unwrap_or_else(|| workspace::window_icon(self.id));
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(17.0), Sense::hover());
                                    if let Some(t) = super::icons::global(icon) {
                                        ui.painter().image(
                                            t.id(),
                                            rect,
                                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                            p.violet_light,
                                        );
                                    }
                                }
                                ui.add(egui::Label::new(title).truncate());
                            });
                        }
                    });
                })
            };
            let header = if framed_header.is_some() {
                egui::Frame::new()
                    .fill(p.bar)
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 10,
                        bottom: 10,
                    })
                    .show(ui, &mut header_content)
                    .response
            } else {
                header_content(ui).response
            };
            let mut title_rect = header.rect;
            if framed_header.is_some() {
                ui.painter()
                    .hline(header.rect.x_range(), header.rect.bottom(), Stroke::new(1.0, p.raised));
            }
            title_rect.max.x = buttons_left;
            // Keep the title buttons outside the double-click target; Window
            // still owns dragging the bar, including when it is minimized.
            if ui
                .interact(title_rect, window_id.with("title_double_click"), Sense::click())
                .double_clicked()
            {
                toggle_min = true;
            }
            if !minimized {
                if framed_header.is_none() {
                    if self.compact_header {
                        ui.painter()
                            .hline(header.rect.x_range(), header.rect.bottom() + 2.0, Stroke::new(1.0, p.raised));
                    } else {
                        ui.separator();
                        ui.add_space(5.0);
                    }
                }
                out = Some(body(ui));
            }
        });
        if let Some(response) = response {
            if let Some((painter, slot)) = glass_slot {
                let radius = f32::from(ctx.global_style().visuals.window_corner_radius.nw);
                workspace::finish_glass(
                    &painter.with_clip_rect(response.response.rect),
                    slot,
                    response.response.rect,
                    p,
                    radius,
                );
            }
            if !minimized {
                ctx.data_mut(|d| d.insert_persisted(expanded_id, response.response.rect.size()));
            }
            if let Some(drag) = resize_drag {
                drag.finish(ctx, window_id, response.response.rect);
            }
            if resizable {
                resize::handles(ctx, &response.response);
            }
        }
        if close {
            *open = false;
        }
        if toggle_min {
            ctx.data_mut(|d| d.insert_persisted(min_id, !minimized));
            if minimized {
                ctx.data_mut(|d| d.insert_temp(restore_id, true));
            }
            // LLFloater::setMinimized(false) plays UISndWindowClose
            if minimized && !self.silent {
                super::sound_cues::request(ctx, crate::ui_sound::UiSound::WindowClose);
            }
        }
        out
    }
}

const PLACE_POS: &str = "place_pos";

/// Move a dependent floater without changing its size or minimized state.
pub fn move_floater(ctx: &egui::Context, id: &str, pos: egui::Pos2) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(id).with(PLACE_POS), pos));
}

/// Give the floater `id` this position and size on its next frame, expanded,
/// as if the user had moved and resized it: a demo scenario then shows the
/// same thing whatever layout the previous run left in ui_layout.ron.
pub fn place_floater(ctx: &egui::Context, id: &str, pos: egui::Pos2, size: Vec2) {
    let window_id = egui::Id::new(id);
    ctx.data_mut(|d| {
        d.insert_persisted(egui::Id::new((id, "minimized")), false);
        d.insert_persisted(window_id.with("expanded_size"), size);
        d.insert_temp(window_id.with("restore_size"), true);
        d.insert_temp(window_id.with(PLACE_POS), pos);
    });
}

/// Title bar button with a Tabler icon (drawn by hand if it is missing).
fn title_button(ui: &mut egui::Ui, p: &Palette, icon: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), Sense::click());
    let col = if resp.hovered() { p.ink } else { p.muted };
    let c = rect.center();
    let stroke = Stroke::new(1.4, col);
    let painter = ui.painter();
    if let Some(t) = super::icons::global(icon) {
        let r = egui::Rect::from_center_size(c, Vec2::splat(14.0));
        painter.image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), col);
        return resp;
    }
    match icon {
        "x" => {
            painter.line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], stroke);
            painter.line_segment([c + Vec2::new(4.0, -4.0), c + Vec2::new(-4.0, 4.0)], stroke);
        }
        "minus" => {
            painter.line_segment([c + Vec2::new(-5.0, 1.0), c + Vec2::new(5.0, 1.0)], stroke);
        }
        "question-mark" => {
            painter.circle_stroke(c, 6.0, Stroke::new(1.2, col));
            painter.text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(10.0), col);
        }
        _ => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, Vec2::splat(10.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
    resp
}

/// Flat on/off switch.
pub fn switch(ui: &mut egui::Ui, p: &Palette, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(34.0, 18.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let track = if *on { p.violet } else { p.raised };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(9), track);
    let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
    painter.circle_filled(egui::pos2(x, rect.center().y), 6.5, if *on { Color32::WHITE } else { p.muted });
    resp
}

/// Flat tab strip. Returns true when the selection changed.
pub fn tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let n = labels.len().max(1) as f32;
        let font = egui::FontId::proportional(12.0);
        let text_w: Vec<f32> = labels
            .iter()
            .map(|(l, _)| ui.fonts_mut(|f| f.layout_no_wrap((*l).to_owned(), font.clone(), Color32::WHITE).size().x) + 18.0)
            .collect();
        let gaps = (n - 1.0) * ui.spacing().item_spacing.x;
        let total: f32 = text_w.iter().sum::<f32>() + gaps;
        let compact = ((ui.available_width() - gaps) / (total - gaps)).clamp(0.0, 1.0);
        let extra = ((ui.available_width() - total) / n).max(0.0);
        for (i, (label, enabled)) in labels.iter().enumerate() {
            let w = text_w[i] * compact + extra.min(40.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), if *enabled { Sense::click() } else { Sense::hover() });
            let sel = *selected == i;
            if sel || (resp.hovered() && *enabled) {
                let fill = if sel {
                    p.violet.gamma_multiply(0.18)
                } else {
                    p.raised.gamma_multiply(0.25)
                };
                ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
            }
            underline(ui, p, &resp, sel, *enabled);
            let col = if !*enabled {
                p.muted_dim
            } else if sel {
                p.ink
            } else {
                p.muted
            };
            let galley = egui::WidgetText::from(*label).into_galley(
                ui,
                Some(egui::TextWrapMode::Truncate),
                (rect.width() - 8.0).max(0.0),
                font.clone(),
            );
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, col);
            let resp = resp.on_hover_text(*label);
            if resp.clicked() && !sel {
                *selected = i;
                changed = true;
            }
        }
    });
    changed
}

/// Column header row for list views.
pub fn header_row(ui: &mut egui::Ui, p: &Palette, cols: &[(&str, f32)]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.field);
    let mut x = rect.left() + 6.0;
    for (label, w) in cols {
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::proportional(12.0),
            p.muted,
        );
        x += if *w > 0.0 { *w } else { rect.width() * 0.5 };
    }
}

pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

pub fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width))
}

/// Small flat button with a Tabler icon (copy / paste / flip...).
pub fn icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(20.0, 20.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if resp.hovered() && enabled { p.raised } else { p.field };
    ui.painter().rect_filled(rect, 2.0, fill);
    let col = if enabled { p.ink } else { p.muted_dim };
    match super::icons::global(icon) {
        Some(t) => {
            ui.painter().image(
                t.id(),
                egui::Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                col,
            );
        }
        None => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(10.0),
                col,
            );
        }
    }
    resp.on_hover_text(tip).clicked() && enabled
}

/// Small flat text button.
pub fn flat_button(ui: &mut egui::Ui, p: &Palette, label: &str) -> egui::Response {
    text_button(ui, p, label, None)
}

/// A light toolbar action using the same gray/active underline as the tabs.
pub(crate) fn underlined_button(ui: &mut egui::Ui, p: &Palette, label: &str, selected: bool) -> egui::Response {
    let response = ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(p.ink))
            .frame(false)
            .min_size(Vec2::new(0.0, ui.spacing().interact_size.y)),
    );
    underline(ui, p, &response, selected || response.is_pointer_button_down_on(), ui.is_enabled());
    response
}

fn underline(ui: &egui::Ui, p: &Palette, response: &egui::Response, selected: bool, enabled: bool) {
    let color = if selected {
        p.violet
    } else if response.hovered() && enabled {
        p.muted
    } else {
        p.muted_dim.gamma_multiply(0.7)
    };
    let height = if selected { 2.0 } else { 1.0 };
    let rect = response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            rect.left_bottom() - Vec2::new(-6.0, height),
            Vec2::new((rect.width() - 12.0).max(1.0), height),
        ),
        0.0,
        color,
    );
}

/// Equal-sized dialog buttons; long script labels keep their full hover text.
pub fn flat_button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, size: Vec2) -> egui::Response {
    text_button(ui, p, label, Some(size)).on_hover_text(label)
}

fn text_button(ui: &mut egui::Ui, p: &Palette, label: &str, size: Option<Vec2>) -> egui::Response {
    // Keep the button in the parent's layout so wrapped toolbars can break
    // before it. A child UI would wrap internally and enlarge narrow windows.
    let style = ui.style().clone();
    {
        // A darker face stays identifiable even when panel and raised match.
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = p.field;
        w.inactive.bg_fill = p.field;
        w.inactive.bg_stroke = Stroke::new(1.0, p.muted_dim.gamma_multiply(0.65));
        w.hovered.weak_bg_fill = p.raised;
        w.hovered.bg_fill = p.raised;
        w.hovered.bg_stroke = Stroke::new(1.0, p.violet_light.gamma_multiply(0.65));
        w.active.weak_bg_fill = p.violet.gamma_multiply(0.28);
        w.active.bg_fill = p.violet.gamma_multiply(0.28);
        w.active.bg_stroke = Stroke::new(1.0, p.violet);
    }
    let button = egui::Button::new(RichText::new(label).size(12.0).color(p.ink));
    let response = match size {
        Some(size) => ui.add_sized(size, button.truncate()),
        None => ui.add(button.min_size(Vec2::new(0.0, 20.0))),
    };
    ui.set_style(style);
    response
}

/// Primary action using the Aurora accent and the flat button geometry.
pub fn primary_button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, size: Vec2) -> egui::Response {
    ui.add_sized(
        size,
        egui::Button::new(RichText::new(label).size(12.0).strong().color(p.ink))
            .fill(p.violet)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .truncate(),
    )
    .on_hover_text(label)
}

/// Green online / grey offline.
pub fn online_color(p: &Palette, online: bool) -> Color32 {
    if online { p.success } else { p.muted_dim }
}

/// Region maturity badge (G / M / A), flat vector square in the Aurora style.
pub fn maturity_badge(ui: &mut egui::Ui, p: &Palette, maturity: &str, size: f32) -> Option<egui::Response> {
    // Fixed-meaning colors (kept from the official viewer): G violet, M white, A red.
    let (letter, bg, fg, tip) = match maturity {
        value if value == aurora_i18n::text("common-general") => (
            "G",
            p.violet,
            Color32::WHITE,
            aurora_i18n::tr!("ui-widgets-general-region-all-ages"),
        ),
        value if value == aurora_i18n::text("common-moderate") => {
            ("M", Color32::WHITE, p.navy, aurora_i18n::tr!("ui-widgets-moderate-region"))
        }
        value if value == aurora_i18n::text("common-adult") => (
            "A",
            Color32::from_rgb(0xD9, 0x2D, 0x2D),
            Color32::WHITE,
            aurora_i18n::tr!("ui-widgets-adult-region"),
        ),
        _ => return None,
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), bg);
    // centered on the letter itself, not on the text row
    let galley = ui
        .painter()
        .layout_no_wrap(letter.to_owned(), egui::FontId::proportional(size * 0.72), fg);
    paint_ink_centered(ui.painter(), galley, rect.center(), fg);
    Some(resp.on_hover_text(tip))
}

/// Paint a galley so that its visible ink (not egui's text row, which
/// includes ascender / descender room) is centered on `center`: digits in
/// round badges then sit exactly in the middle.
pub fn paint_ink_centered(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, center: egui::Pos2, color: Color32) {
    let mut ink = egui::Rect::NOTHING;
    for row in &galley.rows {
        for g in &row.row.glyphs {
            if g.uv_rect.size.y <= 0.0 {
                continue;
            }
            let min = row.pos + g.pos.to_vec2() + g.uv_rect.offset;
            ink = ink.union(egui::Rect::from_min_size(min, g.uv_rect.size));
        }
    }
    let ink_center = if ink.is_positive() { ink.center() } else { galley.rect.center() };
    // whole pixels keep the glyphs crisp
    let pos = (center - ink_center.to_vec2()).round();
    painter.galley(pos, galley, color);
}

/// A picture fitted in `size` keeping its aspect, or a placeholder (a
/// Tabler `icon` over a caption) while there is none.
pub fn picture(
    ui: &mut egui::Ui,
    p: &Palette,
    tex: Option<&egui::TextureHandle>,
    size: Vec2,
    icon: &str,
    placeholder: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect_filled(rect, 2.0, p.field);
    match tex {
        Some(t) => {
            let [w, h] = t.size();
            let aspect = w.max(1) as f32 / h.max(1) as f32;
            let fit = if aspect >= size.x / size.y {
                Vec2::new(size.x, size.x / aspect)
            } else {
                Vec2::new(size.y * aspect, size.y)
            };
            ui.painter().image(
                t.id(),
                egui::Rect::from_center_size(rect.center(), fit),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            if let Some(t) = super::icons::global(icon) {
                let ir = egui::Rect::from_center_size(rect.center(), Vec2::splat(size.min_elem() * 0.45));
                ui.painter().image(
                    t.id(),
                    ir,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    p.muted_dim,
                );
            }
            ui.painter().text(
                rect.center_bottom() - Vec2::new(0.0, 12.0),
                egui::Align2::CENTER_CENTER,
                placeholder,
                egui::FontId::proportional(10.5),
                p.muted_dim,
            );
        }
    }
    ui.painter()
        .rect_stroke(rect, 2.0, egui::Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
    resp
}

/// A menu opened by a Tabler icon button (menu_button with an image).
pub fn icon_menu(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str, content: impl FnOnce(&mut egui::Ui)) {
    let Some(t) = super::icons::global(name) else {
        ui.menu_button(tip, content);
        return;
    };
    let img = egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(14.0))).tint(p.muted);
    let (resp, _) = egui::containers::menu::MenuButton::from_button(egui::Button::image(img).frame(false)).ui(ui, content);
    resp.on_hover_text(tip);
}
