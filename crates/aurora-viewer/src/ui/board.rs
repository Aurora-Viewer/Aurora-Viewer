//! The board of tiles of the wide Inspect and Area Search: tiles packed on
//! columns, one of which can grow while the others stay beside it, small and
//! alive. The layout is pure; the drawing follows the theme.

use super::widgets;
use crate::theme::Palette;
use egui::{Rect, RichText, Sense, Stroke, Vec2};

/// Between two tiles.
pub const GAP: f32 = 8.0;
/// Width of the small tiles beside a grown one.
const SIDE: f32 = 185.0;
/// Narrowest column, at the text's usual size.
const COLUMN: f32 = 125.0;
/// Smallest height of a row, and the tallest it is stretched to.
const ROW: f32 = 64.0;
/// Inside a tile.
const PAD: Vec2 = Vec2::new(10.0, 8.0);
/// The line of a tile's title.
pub const TITLE: f32 = 24.0;

/// A tile to place: how many columns and rows it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile<T> {
    pub id: T,
    pub columns: u8,
    pub rows: u8,
}

/// Where a tile is: column, row, columns, rows.
type Slot = [usize; 4];

/// How a tile is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Beside a grown tile: a figure.
    Small,
    /// On the board.
    Usual,
    /// Opened: everything.
    Grown,
}

/// A tile's place this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed<T> {
    pub id: T,
    pub rect: Rect,
    pub form: Form,
}

/// How many columns a board of that width has: four, two with a large text
/// or a narrow window, one at the least.
pub fn columns(width: f32, text_scale: f32) -> usize {
    let room = width / (COLUMN * text_scale.max(0.5));
    if room >= 4.0 {
        4
    } else if room >= 2.0 {
        2
    } else {
        1
    }
}

/// The tiles in order, each in the first free place of `columns` columns.
fn pack<T: Copy>(tiles: &[Tile<T>], columns: usize) -> (Vec<Slot>, usize) {
    let mut used: Vec<Vec<bool>> = Vec::new();
    let mut slots = Vec::with_capacity(tiles.len());
    for tile in tiles {
        // on fewer columns the large tiles are lower: the next ones stay in
        // sight
        let most = if columns < 4 { 3 } else { usize::MAX };
        let w = usize::from(tile.columns).clamp(1, columns);
        let h = usize::from(tile.rows).clamp(1, most);
        let mut row = 0;
        let (x, y) = 'found: loop {
            for x in 0..=columns - w {
                let free = (row..row + h).all(|y| used.get(y).is_none_or(|line| line[x..x + w].iter().all(|u| !u)));
                if free {
                    break 'found (x, row);
                }
            }
            row += 1;
        };
        while used.len() < y + h {
            used.push(vec![false; columns]);
        }
        for line in &mut used[y..y + h] {
            line[x..x + w].fill(true);
        }
        slots.push([x, y, w, h]);
    }
    (slots, used.len())
}

/// The tiles over `area`: packed, or `grown` large and the others in a column
/// beside it. Returns them and the height they take (past `area`'s, the board
/// scrolls).
pub fn place<T: Copy + PartialEq>(tiles: &[Tile<T>], grown: Option<T>, area: Rect, text_scale: f32) -> (Vec<Placed<T>>, f32) {
    let columns = columns(area.width(), text_scale);
    if let Some(open) = grown.filter(|g| tiles.iter().any(|t| t.id == *g)) {
        return (place_grown(tiles, open, area, columns), area.height());
    }
    let (slots, rows) = pack(tiles, columns);
    let width = (area.width() - GAP * (columns - 1) as f32) / columns as f32;
    let least = ROW * text_scale.max(1.0);
    let height = ((area.height() - GAP * rows.saturating_sub(1) as f32) / rows.max(1) as f32).max(least);
    let placed = tiles
        .iter()
        .zip(&slots)
        .map(|(tile, [x, y, w, h])| {
            let min = area.min + Vec2::new(*x as f32 * (width + GAP), *y as f32 * (height + GAP));
            let size = Vec2::new(*w as f32 * (width + GAP) - GAP, *h as f32 * (height + GAP) - GAP);
            Placed {
                id: tile.id,
                rect: Rect::from_min_size(min, size),
                form: Form::Usual,
            }
        })
        .collect();
    (placed, rows as f32 * (height + GAP) - GAP)
}

fn place_grown<T: Copy + PartialEq>(tiles: &[Tile<T>], open: T, area: Rect, columns: usize) -> Vec<Placed<T>> {
    let others = tiles.iter().filter(|t| t.id != open).count();
    // no room beside it: alone
    let beside = columns >= 4 && others > 0;
    let large = if beside { area.with_max_x(area.right() - SIDE - GAP) } else { area };
    let height = (area.height() - GAP * others.saturating_sub(1) as f32) / others.max(1) as f32;
    let mut y = area.top();
    tiles
        .iter()
        .filter_map(|tile| {
            if tile.id == open {
                return Some(Placed {
                    id: tile.id,
                    rect: large,
                    form: Form::Grown,
                });
            }
            if !beside {
                return None;
            }
            let rect = Rect::from_min_size(egui::pos2(area.right() - SIDE, y), Vec2::new(SIDE, height));
            y += height + GAP;
            Some(Placed {
                id: tile.id,
                rect,
                form: Form::Small,
            })
        })
        .collect()
}

/// A tile slides to its place (at once with Reduce motion, which takes egui's
/// animation time to 0).
pub fn slide(ui: &egui::Ui, id: egui::Id, to: Rect) -> Rect {
    let time = if ui.style().animation_time > 0.0 { 0.18 } else { 0.0 };
    let ctx = ui.ctx();
    let at = |n: u8, v: f32| ctx.animate_value_with_time(id.with(n), v, time);
    // from the board's own corner: a window that moves drags nothing behind
    let o = ui.max_rect().min;
    let x = at(0, to.left() - o.x);
    let y = at(1, to.top() - o.y);
    Rect::from_min_size(o + Vec2::new(x, y), Vec2::new(at(2, to.width()), at(3, to.height())))
}

/// A size that slides to its new value.
pub fn slide_value(ui: &egui::Ui, id: egui::Id, to: f32) -> f32 {
    let time = if ui.style().animation_time > 0.0 { 0.18 } else { 0.0 };
    ui.ctx().animate_value_with_time(id, to, time)
}

/// What a tile's title says.
pub struct Head<'a> {
    pub title: &'a str,
    /// Quieter, after it.
    pub note: &'a str,
    /// What a screen reader says of the whole tile.
    pub spoken: &'a str,
}

/// Draw a tile's surface, its title and its button to grow or close, then its
/// inside. Returns whether the button was pressed.
pub fn tile<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    id: egui::Id,
    rect: Rect,
    form: Form,
    head: Option<Head<'_>>,
    inside: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    tile_on(ui, p, id, rect, form, head, true, inside)
}

/// `tile`, on a surface already there when not `painted`.
#[allow(clippy::too_many_arguments)]
pub fn tile_on<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    id: egui::Id,
    rect: Rect,
    form: Form,
    head: Option<Head<'_>>,
    painted: bool,
    inside: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    if painted {
        surface(ui, p, rect, form);
    }
    let inner = rect.shrink2(PAD);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing.y = 0.0;
    let toggled = head.is_some_and(|head| title(&mut child, p, id, form, &head));
    let out = inside(&mut child);
    (toggled, out)
}

pub fn surface(ui: &egui::Ui, p: &Palette, rect: Rect, form: Form) {
    let radius = ui.visuals().window_corner_radius;
    let fill = if form == Form::Grown {
        p.bar
    } else {
        p.raised.gamma_multiply(0.45)
    };
    let usual = if form == Form::Grown {
        Stroke::new(1.0, p.muted.gamma_multiply(0.45))
    } else {
        Stroke::NONE
    };
    let stroke = widgets::border_stroke(p, usual);
    ui.painter().rect(rect, radius, fill, stroke, egui::StrokeKind::Inside);
}

/// The title line; true when its button was pressed.
fn title(ui: &mut egui::Ui, p: &Palette, id: egui::Id, form: Form, head: &Head<'_>) -> bool {
    let (line, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TITLE), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, head.spoken));
    let pressed = grow_button(ui, p, id, line.right_center(), form, head.title);
    let size = if form == Form::Grown { 16.0 } else { 12.5 };
    let font = egui::FontId::new(size, super::fonts::semibold(ui.ctx()));
    let room = (line.width() - TITLE - 4.0).max(20.0);
    let width = ui.painter().layout_no_wrap(head.title.to_owned(), font.clone(), p.ink).size().x;
    widgets::paint_elided(ui, head.title, line.left_center(), room, font, p.ink);
    if !head.note.is_empty() && width + 24.0 < room {
        let at = line.left_center() + Vec2::new(width + 7.0, 0.0);
        widgets::paint_elided(ui, head.note, at, room - width - 7.0, egui::FontId::proportional(11.5), p.muted);
    }
    pressed
}

/// The button that grows a tile, or closes the grown one; its right edge at
/// `right`.
pub fn grow_button(ui: &mut egui::Ui, p: &Palette, id: egui::Id, right: egui::Pos2, form: Form, title: &str) -> bool {
    let rect = Rect::from_center_size(right - Vec2::new(TITLE / 2.0, 0.0), Vec2::splat(TITLE));
    let resp = ui.interact(rect, id.with("grow"), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let (icon, tip) = if form == Form::Grown {
        ("x", aurora_i18n::tr!("board-close", title = title))
    } else {
        ("arrows-maximize", aurora_i18n::tr!("board-grow", title = title))
    };
    let ink = if resp.hovered() { p.ink } else { p.muted };
    super::sheet::icon_at(ui, icon, rect.center(), 14.0, ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
    resp.on_hover_text(tip).clicked()
}

/// A large figure with a word beside it: the inside of a small tile.
pub fn figure(ui: &mut egui::Ui, p: &Palette, value: &str, word: &str) {
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                RichText::new(value)
                    .size(20.0)
                    .family(super::fonts::semibold(ui.ctx()))
                    .color(p.ink),
            );
            ui.add(egui::Label::new(RichText::new(word).size(11.5).color(p.muted)).truncate());
        });
    });
}

/// How a row of a tile's list is lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lit {
    #[default]
    Off,
    /// Chosen: a filter, or the item of the prim outlined in the world.
    On,
    /// Outlined in the world: the accent.
    Outlined,
    /// Not chosen, but pointed at elsewhere in the window.
    Pointed,
}

impl Lit {
    pub fn chosen(self) -> bool {
        matches!(self, Lit::On | Lit::Outlined)
    }
}

/// The keyboard in a list: one stop for Tab, then the arrows, Home and End
/// from row to row.
pub struct Nav {
    id: egui::Id,
    at: usize,
    moved: bool,
}

impl Nav {
    /// `step`: how many rows Page Up and Page Down move.
    pub fn begin(ui: &egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug, count: usize, step: usize) -> Nav {
        let id = egui::Id::unique(("board_nav", ui.layer_id(), egui::Id::unique(salt)));
        let last = count.saturating_sub(1);
        let mut at = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0).min(last);
        let focused = id.with(at);
        let mut moved = false;
        if ui.memory(|m| m.has_focus(focused)) {
            let key = |k| ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
            let to = if key(egui::Key::ArrowDown) {
                at + 1
            } else if key(egui::Key::ArrowUp) {
                at.saturating_sub(1)
            } else if key(egui::Key::PageDown) {
                at + step.max(1)
            } else if key(egui::Key::PageUp) {
                at.saturating_sub(step.max(1))
            } else if key(egui::Key::Home) {
                0
            } else if key(egui::Key::End) {
                last
            } else {
                at
            };
            moved = to.min(last) != at;
            at = to.min(last);
            ui.memory_mut(|m| {
                m.request_focus(id.with(at));
                // the arrows are the list's, not egui's between widgets
                m.set_focus_lock_filter(
                    id.with(at),
                    egui::EventFilter {
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
        }
        ui.data_mut(|d| d.insert_temp(id, at));
        Nav { id, at, moved }
    }

    /// The row the keyboard is on, or comes to.
    pub fn at(&self) -> usize {
        self.at
    }

    /// The keyboard is on the list.
    pub fn focused(&self, ui: &egui::Ui) -> bool {
        ui.memory(|m| m.has_focus(self.id.with(self.at)))
    }

    /// The row the keyboard just came to: a virtual list scrolls there.
    pub fn arrived(&self) -> Option<usize> {
        self.moved.then_some(self.at)
    }

    /// Row `index` over `rect`.
    pub fn row(&self, ui: &egui::Ui, rect: Rect, index: usize) -> egui::Response {
        let sense = if index == self.at { Sense::click() } else { Sense::CLICK };
        let resp = ui.interact(rect, self.id.with(index), sense);
        if resp.clicked() || resp.secondary_clicked() {
            ui.data_mut(|d| d.insert_temp(self.id, index));
        }
        if self.moved && index == self.at {
            ui.scroll_to_rect_animation(rect, None, egui::style::ScrollAnimation::none());
        }
        resp
    }
}

/// A long list of rows of one height; `card` is room under one of them.
/// `draw` gets the row's index, its rectangle and the room under it. Only
/// what shows is drawn; the keyboard's row is brought into view.
pub fn virtual_rows(
    ui: &mut egui::Ui,
    salt: &str,
    count: usize,
    height: f32,
    card: Option<(usize, f32)>,
    nav: &Nav,
    mut draw: impl FnMut(&mut egui::Ui, usize, Rect, Option<Rect>),
) {
    let top = |i: usize| i as f32 * height + card.filter(|(at, _)| i > *at).map_or(0.0, |(_, h)| h);
    let total = top(count);
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            ui.set_width(ui.available_width());
            let area = ui.max_rect();
            let rect = |i: usize| Rect::from_min_size(egui::pos2(area.left(), area.top() + top(i)), Vec2::new(area.width(), height));
            if let Some(i) = nav.arrived() {
                ui.scroll_to_rect_animation(rect(i), None, egui::style::ScrollAnimation::none());
            }
            let (from, to) = (viewport.top(), viewport.bottom());
            let before = card.map_or(0.0, |(_, h)| h);
            let first = (((from - before) / height).floor().max(0.0) as usize).min(count);
            let last = ((to / height).ceil().max(0.0) as usize).min(count);
            // the keyboard's row stays there for the keyboard
            let stop = (nav.at < first || nav.at >= last) && nav.at < count;
            for i in (first..last).chain(stop.then_some(nav.at)) {
                let row = rect(i);
                let under = card
                    .filter(|(at, _)| *at == i)
                    .map(|(_, h)| Rect::from_min_size(row.left_bottom(), Vec2::new(row.width(), h)));
                let bottom = under.map_or(row.bottom(), |r| r.bottom());
                if i != nav.at && (bottom < area.top() + from || row.top() > area.top() + to) {
                    continue;
                }
                draw(ui, i, row, under);
            }
        });
}

/// A row of `virtual_rows`.
pub fn row_at(ui: &egui::Ui, p: &Palette, nav: &Nav, index: usize, rect: Rect, lit: Lit) -> egui::Response {
    let resp = nav.row(ui, rect, index);
    paint_lit(ui, p, rect, lit, resp.hovered());
    resp
}

/// One row of a list: its face and what it senses. The caller paints over it.
pub fn row(ui: &mut egui::Ui, p: &Palette, nav: &Nav, index: usize, height: f32, lit: Lit) -> (Rect, egui::Response) {
    let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), height));
    let resp = nav.row(ui, rect, index);
    if ui.is_rect_visible(rect) {
        paint_lit(ui, p, rect, lit, resp.hovered());
    }
    (rect, resp)
}

/// Where a chosen row's check goes, so that its colour is not alone: its
/// right edge at `right`; returns its left edge.
pub fn check(ui: &egui::Ui, p: &Palette, right: f32, y: f32) -> f32 {
    super::sheet::icon_at(ui, "check", egui::pos2(right - 7.0, y), 13.0, p.ink);
    right - 18.0
}

/// A column of a table: its width, or 0 for a share of what is left
/// (`weight`).
#[derive(Debug, Clone, Copy)]
pub struct Col<'a> {
    pub title: &'a str,
    pub width: f32,
    pub weight: f32,
    pub numeric: bool,
}

impl<'a> Col<'a> {
    pub fn text(title: &'a str, weight: f32) -> Self {
        Col {
            title,
            width: 0.0,
            weight,
            numeric: false,
        }
    }

    pub fn figure(title: &'a str, width: f32) -> Self {
        Col {
            title,
            width,
            weight: 0.0,
            numeric: true,
        }
    }
}

/// The columns over a line.
pub fn col_rects(cols: &[Col<'_>], line: Rect) -> Vec<Rect> {
    let fixed: f32 = cols.iter().map(|c| c.width).sum();
    let weights: f32 = cols.iter().filter(|c| c.width == 0.0).map(|c| c.weight).sum();
    let unit = ((line.width() - fixed) / weights.max(0.01)).max(0.0);
    let mut x = line.left();
    cols.iter()
        .map(|c| {
            let w = if c.width > 0.0 { c.width } else { unit * c.weight };
            let r = Rect::from_min_max(egui::pos2(x, line.top()), egui::pos2(x + w, line.bottom()));
            x += w;
            r
        })
        .collect()
}

/// The headings of a table; a click sorts, a second one turns it over, a
/// third goes back to the order of the list.
pub fn header(ui: &mut egui::Ui, p: &Palette, salt: &str, cols: &[Col<'_>], sort: &mut Option<(usize, bool)>) {
    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    for (i, (col, cell)) in cols.iter().zip(col_rects(cols, line)).enumerate() {
        let resp = ui.interact(cell, ui.scope_id().with((salt, i)), Sense::click());
        let sorted = sort.filter(|(c, _)| *c == i);
        let ink = if sorted.is_some() || resp.hovered() { p.ink } else { p.muted };
        let mark = if sorted.is_some() { 14.0 } else { 0.0 };
        if col.numeric {
            text_ending(ui, cell.right() - 8.0 - mark, cell.center().y, col.title, 11.5, ink);
        } else {
            let font = egui::FontId::proportional(11.5);
            let at = cell.left_center() + Vec2::new(8.0, 0.0);
            widgets::paint_elided(ui, col.title, at, cell.width() - 12.0 - mark, font, ink);
        }
        if let Some((_, down)) = sorted {
            let icon = if down { "caret-down-filled" } else { "caret-up-filled" };
            super::sheet::icon_at(ui, icon, egui::pos2(cell.right() - 10.0, cell.center().y), 8.0, p.violet_light);
        }
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, sorted.is_some(), col.title));
        if resp.clicked() {
            *sort = match sorted {
                // figures start with the largest
                None => Some((i, col.numeric)),
                Some((_, down)) if down == col.numeric => Some((i, !down)),
                Some(_) => None,
            };
        }
    }
    ui.painter().hline(line.x_range(), line.bottom(), widgets::divider_stroke(p));
}

/// A cell of text.
pub fn cell(ui: &egui::Ui, rect: Rect, text: &str, color: egui::Color32) {
    let at = rect.left_center() + Vec2::new(8.0, 0.0);
    widgets::paint_elided(ui, text, at, rect.width() - 12.0, egui::FontId::proportional(12.5), color);
}

/// A cell of figures.
pub fn figure_cell(ui: &egui::Ui, rect: Rect, text: &str, color: egui::Color32) {
    text_ending(ui, rect.right() - 8.0, rect.center().y, text, 12.5, color);
}

/// The search of a wide window over `rect`, with its shortcut written in it
/// while it is empty.
pub fn search(ui: &mut egui::Ui, p: &Palette, rect: Rect, text: &mut String, hint: &str) {
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt("board_search").max_rect(rect));
    let mut margin = widgets::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
    margin.left += 10;
    let empty = text.is_empty();
    let field = widgets::search_text_edit_with_margin(&mut child, p, text, margin, |edit| {
        edit.hint_text(hint)
            .align(egui::Align2::LEFT_CENTER)
            .desired_width(rect.width() - 10.0)
            .min_size(rect.size())
    });
    if empty && !field.has_focus() {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let body = ui.painter().layout_no_wrap(hint.to_owned(), font, p.muted).size().x;
        let caps = widgets::keycap::find_caps(ui, p);
        let width = widgets::keycap::row_width(&caps);
        // (nothing over the hint with a large text)
        if body + width + 44.0 < rect.width() {
            let at = egui::pos2(rect.right() - 10.0 - width, rect.center().y);
            widgets::keycap::paint_row(ui, p, &caps, at, p.field);
        }
    }
    if super::find_shortcut::take(ui, &[field.id]) {
        field.request_focus();
    }
}

/// Escape for a window whose grown tile closes: the keyboard in no field, or
/// in this window, which is on top.
pub fn escaped(ui: &egui::Ui) -> bool {
    let typing = ui.ctx().egui_wants_keyboard_input();
    let over = ui.ctx().pointer_latest_pos().and_then(|at| ui.ctx().layer_id_at(at)) == Some(ui.layer_id());
    !typing && over && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

pub fn paint_lit(ui: &egui::Ui, p: &Palette, rect: Rect, lit: Lit, hovered: bool) {
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let (fill, stroke) = match lit {
        Lit::Off if hovered => (p.raised.gamma_multiply(0.6), Stroke::NONE),
        Lit::Pointed => (p.raised.gamma_multiply(0.6), Stroke::NONE),
        Lit::Off => return,
        Lit::On => (p.raised, Stroke::new(1.0, p.muted.gamma_multiply(0.6))),
        Lit::Outlined => (p.violet.gamma_multiply(0.22), Stroke::new(1.0, p.violet_light)),
    };
    ui.painter().rect(rect, radius, fill, stroke, egui::StrokeKind::Inside);
}

/// A value's badge, its right edge at `right`; returns its left edge.
/// `weight`: 0 light, 1 medium, 2 heavy.
pub fn badge(ui: &egui::Ui, p: &Palette, right: f32, y: f32, text: String, weight: u8) -> f32 {
    let (face, ink) = match weight {
        0 => (p.raised.gamma_multiply(0.7), p.muted),
        1 => (p.raised, p.ink),
        _ => (p.warn.gamma_multiply(0.16), p.warn),
    };
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(11.0), ink);
    let width = (galley.size().x + 12.0).max(32.0);
    let r = Rect::from_min_max(egui::pos2(right - width, y - 9.0), egui::pos2(right, y + 9.0));
    ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, face);
    let at = egui::pos2(r.center().x - galley.size().x / 2.0, y);
    widgets::paint_ink_left(ui.painter(), galley, at, ink);
    r.left()
}

/// A line of text ending at `right`; returns its left edge.
pub fn text_ending(ui: &egui::Ui, right: f32, y: f32, text: &str, size: f32, color: egui::Color32) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), color);
    let left = right - galley.size().x;
    widgets::paint_ink_left(ui.painter(), galley, egui::pos2(left, y), color);
    left
}

/// A pill that can be on (a filter, a column shown): the accent's wash and
/// outline, with a check so that the colour is not alone.
pub fn pill(ui: &mut egui::Ui, p: &Palette, text: &str, on: bool) -> bool {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.ink);
    let mark = if on { 16.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 18.0 + mark, 26.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let (fill, stroke) = match (on, resp.hovered()) {
        (true, _) => (p.violet.gamma_multiply(0.28), Stroke::new(1.0, p.violet_light)),
        (false, true) => (p.raised, Stroke::NONE),
        (false, false) => (p.raised.gamma_multiply(0.55), Stroke::NONE),
    };
    let stroke = widgets::border_stroke(p, stroke);
    ui.painter().rect(rect, radius, fill, stroke, egui::StrokeKind::Inside);
    let ink = if on { p.ink } else { p.muted };
    if on {
        super::sheet::icon_at(ui, "check", egui::pos2(rect.left() + 14.0, rect.center().y), 12.0, ink);
    }
    let at = egui::pos2(rect.left() + 9.0 + mark, rect.center().y);
    widgets::paint_ink_left(ui.painter(), galley, at, ink);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, text));
    resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: [Tile<u8>; 7] = [
        Tile {
            id: 0,
            columns: 2,
            rows: 5,
        },
        Tile {
            id: 1,
            columns: 2,
            rows: 5,
        },
        Tile {
            id: 2,
            columns: 4,
            rows: 1,
        },
        Tile {
            id: 3,
            columns: 1,
            rows: 2,
        },
        Tile {
            id: 4,
            columns: 1,
            rows: 2,
        },
        Tile {
            id: 5,
            columns: 1,
            rows: 2,
        },
        Tile {
            id: 6,
            columns: 1,
            rows: 2,
        },
    ];

    fn area(w: f32, h: f32) -> Rect {
        Rect::from_min_size(egui::pos2(10.0, 20.0), Vec2::new(w, h))
    }

    fn apart(placed: &[Placed<u8>]) {
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                assert!(!a.rect.shrink(0.5).intersects(b.rect.shrink(0.5)), "{a:?} over {b:?}");
            }
        }
    }

    #[test]
    fn the_board_fills_its_area_without_a_tile_over_another() {
        let area = area(764.0, 616.0);
        let (placed, height) = place(&BOARD, None, area, 1.0);
        assert_eq!(placed.len(), 7);
        assert!((height - 616.0).abs() < 0.5);
        apart(&placed);
        assert!(placed.iter().all(|t| area.expand(0.5).contains_rect(t.rect)));
        // the two large ones side by side, the strip under them
        assert_eq!(placed[0].rect.top(), placed[1].rect.top());
        assert!(placed[2].rect.width() > 760.0 && placed[2].rect.top() > placed[0].rect.bottom());
    }

    #[test]
    fn a_grown_tile_keeps_the_others_beside_it() {
        let area = area(764.0, 616.0);
        let (placed, _) = place(&BOARD, Some(5), area, 1.0);
        apart(&placed);
        let grown: Vec<_> = placed.iter().filter(|t| t.form == Form::Grown).collect();
        assert_eq!((grown.len(), grown[0].id), (1, 5));
        assert_eq!(placed.iter().filter(|t| t.form == Form::Small).count(), 6);
        // a tile that is not on the board grows nothing
        assert!(place(&BOARD, Some(9), area, 1.0).0.iter().all(|t| t.form == Form::Usual));
    }

    #[test]
    fn a_large_text_takes_two_columns_then_one_and_scrolls() {
        assert_eq!(columns(764.0, 1.0), 4);
        assert_eq!(columns(764.0, 2.0), 2);
        assert_eq!(columns(764.0, 1.5), 4);
        assert_eq!(columns(240.0, 1.0), 1);
        let (placed, height) = place(&BOARD, None, area(764.0, 616.0), 2.0);
        apart(&placed);
        assert!(height > 616.0, "taller than its area: it scrolls");
        // nothing beside a grown tile: it stands alone
        let (placed, _) = place(&BOARD, Some(1), area(764.0, 616.0), 2.0);
        assert_eq!(placed.len(), 1);
    }
}
