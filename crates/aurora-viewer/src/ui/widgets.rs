//! Firestorm-style building blocks: floaters with a slim title bar
//! (title, ?, —, ✕), flat tabs, column headers and status dots.

use crate::theme::Palette;
use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Vec2};

pub struct Floater<'a> {
    pub id: &'a str,
    pub title: String,
    pub default_pos: egui::Pos2,
    pub default_size: Vec2,
    pub min_size: Vec2,
    pub resizable: bool,
    pub help: Option<&'a str>,
    /// Extra title bar button: (Phosphor icon, tooltip, set when clicked).
    pub action: Option<(&'a str, &'a str, &'a mut bool)>,
    /// No open / close sounds (sound_flags="0", e.g. floater_tools).
    pub silent: bool,
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
            .stroke(Stroke::new(1.0, p.raised))
            .corner_radius(CornerRadius::same(2))
            .inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 3,
                bottom: 6,
            });
        let mut close = false;
        let mut toggle_min = false;
        let mut out = None;
        let mut w = egui::Window::new(&self.title)
            .id(egui::Id::new(self.id))
            .title_bar(false)
            .fade_in(false)
            .fade_out(false)
            .frame(frame)
            .default_pos(self.default_pos)
            .resizable(self.resizable && !minimized);
        if self.resizable && !minimized {
            w = w.default_size(self.default_size).min_size(self.min_size);
        } else if !minimized {
            w = w.default_width(self.default_size.x);
        }
        w.show(ctx, |ui| {
            // title bar
            ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                let title = RichText::new(&self.title).size(13.0).color(p.ink);
                // a fixed floater is as wide as its title; a resizable one
                // cuts a long title short before the buttons (LLFloater
                // titles end with an ellipsis)
                let truncate = self.resizable && !minimized;
                if !truncate {
                    ui.label(title.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if title_button(ui, p, "x").on_hover_text("Fermer").clicked() {
                        close = true;
                    }
                    if title_button(ui, p, "minus").on_hover_text("Réduire").clicked() {
                        toggle_min = true;
                    }
                    if let Some((icon, tip, clicked)) = self.action.as_mut()
                        && title_button(ui, p, icon).on_hover_text(*tip).clicked()
                    {
                        **clicked = true;
                    }
                    if let Some(h) = self.help {
                        title_button(ui, p, "question").on_hover_text(h);
                    }
                    if truncate {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(title).truncate());
                        });
                    }
                });
            });
            if !minimized {
                ui.add_space(2.0);
                out = Some(body(ui));
            }
        });
        if close {
            *open = false;
        }
        if toggle_min {
            ctx.data_mut(|d| d.insert_persisted(min_id, !minimized));
            // LLFloater::setMinimized(false) plays UISndWindowClose
            if minimized && !self.silent {
                super::sound_cues::request(ctx, crate::ui_sound::UiSound::WindowClose);
            }
        }
        out
    }
}

/// Title bar button with a Phosphor icon (drawn by hand if it is missing).
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
        "question" => {
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
        ui.spacing_mut().item_spacing.x = 1.0;
        let n = labels.len().max(1) as f32;
        let font = egui::FontId::proportional(12.0);
        let text_w: Vec<f32> = labels
            .iter()
            .map(|(l, _)| ui.fonts_mut(|f| f.layout_no_wrap((*l).to_owned(), font.clone(), Color32::WHITE).size().x) + 18.0)
            .collect();
        let total: f32 = text_w.iter().sum::<f32>() + n;
        let extra = ((ui.available_width() - total) / n).max(0.0);
        for (i, (label, enabled)) in labels.iter().enumerate() {
            let w = text_w[i] + extra.min(40.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), if *enabled { Sense::click() } else { Sense::hover() });
            let sel = *selected == i;
            let fill = if sel {
                p.raised
            } else if resp.hovered() && *enabled {
                p.field.gamma_multiply(1.4)
            } else {
                p.field
            };
            ui.painter().rect_filled(rect, CornerRadius::same(1), fill);
            if sel {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(rect.left_bottom() - Vec2::new(0.0, 2.0), Vec2::new(rect.width(), 2.0)),
                    0.0,
                    p.violet,
                );
            }
            let col = if !*enabled {
                p.muted_dim
            } else if sel {
                p.ink
            } else {
                p.muted
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                *label,
                egui::FontId::proportional(12.0),
                col,
            );
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

/// Small flat button with a Phosphor icon (copy / paste / flip...).
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
    ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(p.ink))
            .fill(p.raised)
            .corner_radius(CornerRadius::same(2))
            .min_size(Vec2::new(0.0, 20.0)),
    )
}

/// Equal-sized dialog buttons; long script labels keep their full hover text.
pub fn flat_button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, size: Vec2) -> egui::Response {
    ui.add_sized(
        size,
        egui::Button::new(RichText::new(label).size(12.0).color(p.ink))
            .fill(p.raised)
            .corner_radius(CornerRadius::same(2))
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
        "Général" => ("G", p.violet, Color32::WHITE, "Région Générale (tout public)"),
        "Modéré" => ("M", Color32::WHITE, p.navy, "Région Modérée"),
        "Adulte" => ("A", Color32::from_rgb(0xD9, 0x2D, 0x2D), Color32::WHITE, "Région Adulte"),
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
/// Phosphor `icon` over a caption) while there is none.
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

/// A menu opened by a Phosphor icon button (menu_button with an image).
pub fn icon_menu(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str, content: impl FnOnce(&mut egui::Ui)) {
    let Some(t) = super::icons::global(name) else {
        ui.menu_button(tip, content);
        return;
    };
    let img = egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(14.0))).tint(p.muted);
    let (resp, _) = egui::containers::menu::MenuButton::from_button(egui::Button::image(img).frame(false)).ui(ui, content);
    resp.on_hover_text(tip);
}
