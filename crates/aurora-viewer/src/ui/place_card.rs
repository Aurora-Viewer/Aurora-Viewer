//! The place card shared by the windows that list places (Aurora's Search,
//! Aurora's Places, whose favourites are dragged into the bar's order, and
//! Aurora's Destinations): a row with the parcel's snapshot, its name
//! and a second line (region and position, a date…), the G / M / A badge at
//! the right end, and room before it for what the window adds (traffic, a
//! sale pill, a favourite star). The snapshot covers its frame, its middle
//! kept, like the pictures of the place profiles.

use crate::theme::Palette;
use crate::ui::widgets;
use egui::{FontId, Pos2, Rect, Sense, Vec2};

/// The height of a row and the size of its picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowSize {
    pub height: f32,
    pub thumb: Vec2,
}

impl RowSize {
    /// The rich rows of the search results.
    pub const SEARCH: RowSize = RowSize {
        height: 52.0,
        thumb: Vec2::new(64.0, 40.0),
    };
    /// The list of Aurora's Places, widened.
    pub const PLACES_WIDE: RowSize = RowSize {
        height: 54.0,
        thumb: Vec2::new(72.0, 42.0),
    };
    /// The list of Aurora's Places, narrow (as it opens).
    pub const PLACES_NARROW: RowSize = RowSize {
        height: 48.0,
        thumb: Vec2::new(52.0, 32.0),
    };
    /// The guide's places in Aurora's Destinations, as it opens: a larger
    /// photo and three lines.
    pub const DESTINATIONS: RowSize = RowSize {
        height: 72.0,
        thumb: Vec2::new(96.0, 60.0),
    };
}

/// What the picture of a row shows.
pub enum Thumb<'a> {
    /// A snapshot, or the mountain of the place profiles while there is none.
    Picture(Option<&'a egui::TextureHandle>),
    /// Something else painted in the frame (an event's date, a group's
    /// initials): the frame's face is already drawn.
    Paint(&'a dyn Fn(&egui::Painter, Rect)),
}

/// One row: its picture, its two lines, its rating and whether it is the
/// selected one.
pub struct Row<'a> {
    pub thumb: Thumb<'a>,
    pub title: &'a str,
    pub subtitle: &'a str,
    /// The rating's name ("General"…), None when it is not known.
    pub rating: Option<&'a str>,
    pub selected: bool,
}

/// What a row of Aurora's Destinations adds to a plain one: a count laid
/// over its picture (the people there), the rating before its second line
/// and a third line (a date, a visit), each with its icon.
#[derive(Default)]
pub struct Extras<'a> {
    pub over_picture: Option<(&'a str, &'a str)>,
    /// The rating's name, as `Row::rating`, before the second line.
    pub badge: Option<&'a str>,
    pub third: Option<(&'a str, &'a str, egui::Color32)>,
}

const PAD: f32 = 8.0;
const GAP: f32 = 10.0;
/// Room the drag grip takes before the picture.
const GRIP: f32 = 16.0;

/// A clickable row as wide as the space left. `trailing` lays out from the
/// right edge, before the badge (the first widget it adds is the rightmost);
/// the two lines take what it leaves.
pub fn row(ui: &mut egui::Ui, p: &Palette, size: RowSize, row: &Row, trailing: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    paint_row(ui, p, size, row, false, &Extras::default(), trailing)
}

/// A `row` with what Aurora's Destinations adds (`Extras`).
pub fn row_with(
    ui: &mut egui::Ui,
    p: &Palette,
    size: RowSize,
    row: &Row,
    extras: &Extras,
    trailing: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    paint_row(ui, p, size, row, false, extras, trailing)
}

/// A `row` that can also be dragged (the favourites of Places, put in the
/// bar's order): a grip before its picture, and a response that reports the
/// drag.
pub fn draggable_row(ui: &mut egui::Ui, p: &Palette, size: RowSize, row: &Row, trailing: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    paint_row(ui, p, size, row, true, &Extras::default(), trailing)
}

fn paint_row(
    ui: &mut egui::Ui,
    p: &Palette,
    size: RowSize,
    row: &Row,
    grip: bool,
    extras: &Extras,
    trailing: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let sense = if grip { Sense::click_and_drag() } else { Sense::click() };
    let title = FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()));
    // three lines share the row the two lines share otherwise; a larger
    // text spreads them and the row grows by the difference (#1227)
    let gap = if extras.third.is_some() {
        widgets::two_lines_gap(ui, &title, &FontId::proportional(12.0), 17.0)
    } else {
        0.0
    };
    let height = size.height.max(2.0 * gap + 34.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), sense);
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let radius = ui.visuals().window_corner_radius;
    if row.selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
    }
    let mut left = rect.left() + PAD;
    if grip {
        if let Some(t) = crate::ui::icons::global("grip-vertical") {
            let at = Rect::from_center_size(Pos2::new(rect.left() + 13.0, rect.center().y), Vec2::splat(14.0));
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(t.id(), at, uv, p.muted_dim);
        }
        left += GRIP;
    }
    let thumb = Rect::from_min_size(Pos2::new(left, rect.center().y - size.thumb.y / 2.0), size.thumb);
    match &row.thumb {
        Thumb::Picture(tex) => picture(ui, p, *tex, thumb, (size.thumb.y * 0.45).min(20.0)),
        Thumb::Paint(paint) => {
            ui.painter().rect_filled(thumb, radius, p.field);
            paint(ui.painter(), thumb);
        }
    }
    if let Some((icon, text)) = extras.over_picture {
        picture_pill(ui, p, Pos2::new(thumb.left() + 4.0, thumb.bottom() - 4.0), icon, text, 10.0);
    }
    // the right end: the badge, then what the window adds
    let mut right = rect.right() - PAD;
    if let Some(name) = row.rating {
        let badge = Rect::from_center_size(Pos2::new(right - 8.0, rect.center().y), Vec2::splat(16.0));
        if widgets::paint_maturity_badge(ui.painter(), p, name, badge, widgets::inner_radius(ui.visuals())) {
            right = badge.left() - GAP;
        }
    }
    let text_left = thumb.right() + GAP;
    let lane = Rect::from_min_max(Pos2::new(text_left, rect.top()), Pos2::new(right, rect.bottom()));
    let mut end = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(lane)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    end.spacing_mut().item_spacing.x = 8.0;
    trailing(&mut end);
    let used = end.min_rect();
    let text_right = if used.width() > 0.0 { used.left() - GAP } else { right };
    let width = (text_right - text_left).max(20.0);
    let (y1, y2) = if extras.third.is_some() { (-gap, 0.0) } else { (-8.0, 9.0) };
    let cut = widgets::paint_elided(ui, row.title, Pos2::new(text_left, rect.center().y + y1), width, title, p.ink);
    let mut sub_left = text_left;
    if let Some(name) = extras.badge {
        let badge = Rect::from_center_size(Pos2::new(text_left + 7.0, rect.center().y + y2), Vec2::splat(14.0));
        if widgets::paint_maturity_badge(ui.painter(), p, name, badge, widgets::inner_radius(ui.visuals())) {
            sub_left = badge.right() + 6.0;
        }
    }
    widgets::paint_elided(
        ui,
        row.subtitle,
        Pos2::new(sub_left, rect.center().y + y2),
        (text_right - sub_left).max(20.0),
        FontId::proportional(12.0),
        p.muted,
    );
    if let Some((icon, text, color)) = extras.third {
        let cy = rect.center().y + gap;
        let mut x = text_left;
        if let Some(t) = crate::ui::icons::global(icon) {
            let at = Rect::from_center_size(Pos2::new(x + 6.0, cy), Vec2::splat(12.0));
            ui.painter()
                .image(t.id(), at, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), color);
            x += 17.0;
        }
        widgets::paint_elided(
            ui,
            text,
            Pos2::new(x, cy),
            (text_right - x).max(20.0),
            FontId::proportional(11.5),
            color,
        );
    }
    if cut { resp.on_hover_text(row.title) } else { resp }
}

/// A count or a tag laid over a picture, from its lower left corner `at`:
/// the icon and the text on a dark pill, legible on any photo.
pub fn picture_pill(ui: &egui::Ui, p: &Palette, at: Pos2, icon: &str, text: &str, size: f32) -> Rect {
    let font = FontId::new(size, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.ink);
    let h = (size * 1.8).round();
    let icon_w = if icon.is_empty() { 0.0 } else { size + 3.0 };
    let pill = Rect::from_min_size(
        Pos2::new(at.x, at.y - h),
        Vec2::new(galley.size().x + icon_w + 2.0 * (size * 0.6).round(), h),
    );
    if !ui.is_rect_visible(pill) {
        return pill;
    }
    ui.painter()
        .rect_filled(pill, ui.visuals().window_corner_radius, p.navy.gamma_multiply(0.75));
    let mut x = pill.left() + (size * 0.6).round();
    if let Some(t) = crate::ui::icons::global(icon) {
        let r = Rect::from_center_size(Pos2::new(x + size / 2.0, pill.center().y), Vec2::splat(size));
        ui.painter()
            .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), p.ink);
        x += icon_w;
    }
    widgets::paint_row_text(ui.painter(), galley, x, pill.center().y, p.ink);
    pill
}

/// The width of a `picture_pill`, for one laid from a right edge.
pub fn picture_pill_width(ui: &egui::Ui, icon: &str, text: &str, size: f32) -> f32 {
    let font = FontId::new(size, crate::ui::fonts::semibold(ui.ctx()));
    let w = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x;
    w + if icon.is_empty() { 0.0 } else { size + 3.0 } + 2.0 * (size * 0.6).round()
}

/// A snapshot covering `rect`, or the mountain on the field's face.
pub fn picture(ui: &egui::Ui, p: &Palette, tex: Option<&egui::TextureHandle>, rect: Rect, icon_size: f32) {
    let radius = ui.visuals().window_corner_radius;
    match tex {
        Some(t) => {
            let [w, h] = t.size();
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
                .uv(cover_uv(w as f32, h as f32, rect.size()))
                .tint(widgets::PICTURE_TINT)
                .corner_radius(radius)
                .paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, radius, p.field);
            if let Some(t) = crate::ui::icons::global("mountain-filled") {
                let at = Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.painter().image(t.id(), at, uv, p.muted_dim);
            }
        }
    }
}

/// The part of a `w` x `h` picture that covers a frame of `frame`, its
/// middle kept: the longer side is cut.
pub fn cover_uv(w: f32, h: f32, frame: Vec2) -> Rect {
    let aspect = w.max(1.0) / h.max(1.0);
    let target = frame.x / frame.y.max(1.0);
    if aspect > target {
        let k = target / aspect;
        Rect::from_min_max(Pos2::new((1.0 - k) * 0.5, 0.0), Pos2::new((1.0 + k) * 0.5, 1.0))
    } else {
        let k = aspect / target;
        Rect::from_min_max(Pos2::new(0.0, (1.0 - k) * 0.5), Pos2::new(1.0, (1.0 + k) * 0.5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_covers_its_frame_from_the_middle() {
        // a 4:3 snapshot in the 64 x 40 frame of a row: top and bottom cut
        let uv = cover_uv(256.0, 192.0, Vec2::new(64.0, 40.0));
        assert!((uv.min.x, uv.max.x) == (0.0, 1.0));
        assert!((uv.min.y - 1.0 / 12.0).abs() < 1e-4 && (uv.max.y - 11.0 / 12.0).abs() < 1e-4);
        // a square one in a wide frame keeps its middle band
        let uv = cover_uv(128.0, 128.0, Vec2::new(200.0, 100.0));
        assert!((uv.min.y - 0.25).abs() < 1e-4 && (uv.max.y - 0.75).abs() < 1e-4);
        // a tall one in a square frame: the sides stay, the height is cut
        let uv = cover_uv(100.0, 200.0, Vec2::splat(50.0));
        assert!((uv.min.y - 0.25).abs() < 1e-4 && uv.min.x == 0.0);
        // the same shape is shown whole
        let uv = cover_uv(320.0, 200.0, Vec2::new(64.0, 40.0));
        assert!(uv.min.x.abs() < 1e-4 && uv.min.y.abs() < 1e-4 && (uv.max.x - 1.0).abs() < 1e-4 && (uv.max.y - 1.0).abs() < 1e-4);
    }
}
