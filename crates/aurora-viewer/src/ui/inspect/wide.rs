//! The wide Inspect: a board of tiles that answer each other (the objects in
//! 3D, their list, the complexity among the people here or the land impact on
//! the parcel, who made them, where they are worn or who owns them, figures,
//! prims). Any tile grows, the others staying beside it. Aurora's own
//! presentation of the compact window's data.

use super::model::{self, Area, Item, Tier, Whole};
use super::{Action, Column, Frame, InspectUi, Row, Subject, details, kilobytes, megabytes, people};
use crate::theme::Palette;
use crate::ui::board::{self, Form, Lit};
use crate::ui::object_stage::Showing;
use crate::ui::widgets;
use egui::{Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// The tiles of the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tile {
    Stage,
    Items,
    Weight,
    Makers,
    Places,
    Figures,
    Prims,
}

const TILES: [board::Tile<Tile>; 7] = {
    const fn tile(id: Tile, columns: u8, rows: u8) -> board::Tile<Tile> {
        board::Tile { id, columns, rows }
    }
    [
        tile(Tile::Stage, 2, 5),
        tile(Tile::Items, 2, 5),
        tile(Tile::Weight, 4, 1),
        tile(Tile::Makers, 1, 2),
        tile(Tile::Places, 1, 2),
        tile(Tile::Figures, 1, 2),
        tile(Tile::Prims, 1, 2),
    ]
};

/// What the other tiles are narrowed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// A creator, by their id and the name shown.
    Maker(Option<Uuid>, String),
    Area(Area),
    /// An owner, by the name shown.
    Owner(String),
}

impl Pick {
    fn keeps(&self, rows: &[Row], item: &Item) -> bool {
        let row = &rows[item.first];
        match self {
            Pick::Maker(id, name) => row.creator == *id && row.creator_name == *name,
            Pick::Area(area) => item.area == *area,
            Pick::Owner(name) => row.owner_name == *name,
        }
    }
}

/// The wide window's own state.
#[derive(Default)]
pub struct Wide {
    pub grown: Option<Tile>,
    pub pick: Option<Pick>,
    /// Items left out of the complexity (view only).
    pub aside: Vec<Uuid>,
    /// The item the pointer is on in a list.
    pointed: Option<Uuid>,
    sorts: HashMap<&'static str, Option<(usize, bool)>>,
}

impl Wide {
    pub fn restart(&mut self) {
        self.pick = None;
        self.aside.clear();
        self.grown = None;
        self.pointed = None;
    }
}

/// What every tile reads.
struct View<'a> {
    rows: &'a [Row],
    /// All the items.
    items: &'a [Item],
    /// Those the search and the picked maker, area or owner keep.
    shown: Vec<&'a Item>,
    /// Those the search alone keeps.
    found: Vec<&'a Item>,
    tiers: HashMap<Uuid, Tier>,
    whole: Whole,
    open: Option<&'a Item>,
}

impl View<'_> {
    fn row(&self, item: &Item) -> &Row {
        &self.rows[item.first]
    }
}

/// Height of a row of a list on the board.
const ROW: f32 = 24.0;
const HEADER: f32 = 38.0;
const SEARCH: f32 = 32.0;

pub(super) fn show(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &mut Frame<'_>,
    items: &[Item],
    actions: &mut Vec<Action>,
) {
    let rows = frame.rows;
    let found = model::shown(rows, items, &state.query, frame.view.sort);
    if let Some(pick) = &state.wide.pick
        && !items.iter().any(|i| pick.keeps(rows, i))
    {
        state.wide.pick = None;
    }
    let shown: Vec<&Item> = found
        .iter()
        .copied()
        .filter(|i| state.wide.pick.as_ref().is_none_or(|pick| pick.keeps(rows, i)))
        .collect();
    state.wide.aside.retain(|id| items.iter().any(|i| i.root == *id));
    let view = View {
        rows,
        items,
        shown,
        found,
        tiers: items.iter().map(|i| i.root).zip(model::tiers(items)).collect(),
        whole: Whole::of(items),
        open: state.opened.and_then(|id| items.iter().find(|i| i.root == id)),
    };
    header(state, ui, p, frame, &view);
    ui.add_space(8.0);
    let (mut body, foot) = widgets::above_footer(ui);
    board_tiles(state, &mut body, p, frame, &view, actions);
    foot.show(ui, |ui| footer(state, ui, p, frame, &view, actions));
}

/// Who or what is inspected and what the board is narrowed to; the search on
/// the right, as wide as the tiles under it.
fn header(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, view: &View<'_>) {
    let scale = crate::ui::fonts::scale(ui.ctx());
    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER * scale.max(1.0)), Sense::hover());
    let half = (line.width() - board::GAP) / 2.0;
    let search = Rect::from_min_size(
        egui::pos2(line.right() - half, line.center().y - SEARCH * scale.max(1.0) / 2.0),
        Vec2::new(half, SEARCH * scale.max(1.0)),
    );
    board::search(ui, p, search, &mut state.query, aurora_i18n::tr!("inspect-search"));
    let mut left = line.with_max_x(search.left() - 10.0);
    let pic = Rect::from_center_size(egui::pos2(left.left() + 17.0, line.center().y), Vec2::splat(34.0));
    let radius = ui.visuals().window_corner_radius;
    let (title, second) = match state.subject {
        Subject::Avatar(_) => {
            state.wanted_pics.extend(frame.subject_profile);
            let tex = frame.subject_profile.and_then(|id| frame.pics.get(&id));
            widgets::avatar_disc(ui, p, pic, tex, frame.subject_name, false);
            (frame.subject_name.to_owned(), frame.subject_username.unwrap_or_default().to_owned())
        }
        Subject::Objects => {
            ui.painter().rect_filled(pic, radius, p.raised);
            crate::ui::sheet::icon_at(ui, "cube", pic.center(), 18.0, p.violet_light);
            (
                aurora_i18n::tr!("inspect-objects-title", count = view.items.len()),
                aurora_i18n::tr!("inspect-prims-count", count = view.rows.len()),
            )
        }
    };
    left.min.x = pic.right() + 9.0;
    let chips = chips(state, frame, view);
    // the name keeps what the chips leave, a hundred points at least
    let font = egui::FontId::new(15.0, crate::ui::fonts::semibold(ui.ctx()));
    let name_w = ui.painter().layout_no_wrap(title.clone(), font.clone(), p.ink).size().x;
    let second_w = ui
        .painter()
        .layout_no_wrap(second.clone(), egui::FontId::proportional(12.0), p.ink)
        .size()
        .x;
    let wanted = name_w.max(second_w).min(left.width());
    let name_w = if chips.is_empty() {
        wanted
    } else {
        wanted.min((left.width() * 0.5).max(100.0))
    };
    let off = if second.is_empty() { 0.0 } else { 9.0 * scale };
    widgets::paint_elided(
        ui,
        &title,
        egui::pos2(left.left(), line.center().y - off),
        name_w + 1.0,
        font,
        p.ink,
    );
    widgets::paint_elided(
        ui,
        &second,
        egui::pos2(left.left(), line.center().y + off),
        name_w + 1.0,
        egui::FontId::proportional(12.0),
        p.muted,
    );
    let mut at = left.left() + name_w + 12.0;
    for chip in chips {
        let room = left.right() - at;
        if room < 70.0 {
            break;
        }
        let (width, pressed) = chip_at(ui, p, egui::pos2(at, line.center().y), room, &chip);
        at += width + 6.0;
        if pressed {
            match chip.what {
                Drop::Item => state.opened = None,
                Drop::Pick => state.wide.pick = None,
                Drop::Aside => state.wide.aside.clear(),
            }
        }
    }
}

/// What a chip takes away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Drop {
    Item,
    Pick,
    Aside,
}

struct Chip {
    text: String,
    icon: &'static str,
    what: Drop,
}

fn chips(state: &InspectUi, _frame: &Frame<'_>, view: &View<'_>) -> Vec<Chip> {
    let mut out = Vec::new();
    if let Some(item) = view.open {
        out.push(Chip {
            text: view.row(item).name.trim().to_owned(),
            icon: "cube",
            what: Drop::Item,
        });
    }
    if let Some(pick) = &state.wide.pick {
        let (text, icon) = match pick {
            Pick::Maker(_, name) | Pick::Owner(name) => (name.clone(), "user-filled"),
            Pick::Area(area) => (area.title().to_owned(), "map-pin-filled"),
        };
        out.push(Chip {
            text,
            icon,
            what: Drop::Pick,
        });
    }
    if !state.wide.aside.is_empty() {
        out.push(Chip {
            text: aurora_i18n::tr!("inspect-aside-count", count = state.wide.aside.len()),
            icon: "circle-minus",
            what: Drop::Aside,
        });
    }
    out
}

/// A chip with a cross: its left middle at `at`. Returns its width, and
/// whether it was pressed.
fn chip_at(ui: &mut egui::Ui, p: &Palette, at: egui::Pos2, room: f32, chip: &Chip) -> (f32, bool) {
    let font = egui::FontId::proportional(12.5);
    let text_w = ui.painter().layout_no_wrap(chip.text.clone(), font.clone(), p.ink).size().x;
    let width = (text_w + 58.0).min(room).min(190.0);
    let rect = Rect::from_min_size(egui::pos2(at.x, at.y - 15.0), Vec2::new(width, 30.0));
    let resp = ui.interact(rect, ui.scope_id().with(("inspect_chip", chip.what as u8)), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = if resp.hovered() { 0.4 } else { 0.24 };
    ui.painter().rect(
        rect,
        radius,
        p.violet.gamma_multiply(face),
        Stroke::new(1.0, p.violet_light),
        egui::StrokeKind::Inside,
    );
    crate::ui::sheet::icon_at(ui, chip.icon, egui::pos2(rect.left() + 17.0, at.y), 14.0, p.ink);
    widgets::paint_elided(ui, &chip.text, egui::pos2(rect.left() + 30.0, at.y), width - 54.0, font, p.ink);
    crate::ui::sheet::icon_at(ui, "x", egui::pos2(rect.right() - 13.0, at.y), 12.0, p.muted);
    let tip = aurora_i18n::tr!("board-remove", name = chip.text.as_str());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
    let pressed = resp.on_hover_text(tip).clicked();
    (width, pressed)
}

fn board_tiles(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, frame: &mut Frame<'_>, view: &View<'_>, actions: &mut Vec<Action>) {
    if state.wide.grown.is_some() && board::escaped(ui) {
        state.wide.grown = None;
    }
    let scale = crate::ui::fonts::scale(ui.ctx());
    let area = ui.available_rect_before_wrap();
    let (_, height) = board::place(&TILES, state.wide.grown, area, scale);
    let pointed = state.wide.pointed.take();
    let mut draw = |ui: &mut egui::Ui, area: Rect| {
        let (placed, height) = board::place(&TILES, state.wide.grown, area, scale);
        for tile in placed {
            let id = egui::Id::unique(("inspect_tile", tile.id));
            let rect = board::slide(ui, id, tile.rect);
            let toggled = one_tile(state, ui, p, frame, view, actions, id, rect, tile.id, tile.form, pointed);
            if toggled {
                state.wide.grown = (state.wide.grown != Some(tile.id)).then_some(tile.id);
            }
        }
        height
    };
    if height > area.height() + 1.0 {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let area = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height.max(area.height())));
            let height = draw(ui, area.with_max_y(area.top() + 1.0));
            ui.allocate_space(Vec2::new(area.width(), height));
        });
    } else {
        draw(ui, area);
        ui.advance_cursor_after_rect(area);
    }
}

#[allow(clippy::too_many_arguments)]
fn one_tile(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &mut Frame<'_>,
    view: &View<'_>,
    actions: &mut Vec<Action>,
    id: egui::Id,
    rect: Rect,
    tile: Tile,
    form: Form,
    pointed: Option<Uuid>,
) -> bool {
    match tile {
        Tile::Stage => stage(state, ui, p, frame, view, actions, id, rect, form, pointed),
        Tile::Weight => match state.subject {
            Subject::Avatar(avatar) => weight(state, ui, p, frame, view, id, rect, form, avatar),
            Subject::Objects => land(ui, p, frame, view, id, rect, form),
        },
        Tile::Items => items_tile(state, ui, p, frame, view, id, rect, form),
        Tile::Makers => makers(state, ui, p, frame, view, actions, id, rect, form),
        Tile::Places => places(state, ui, p, frame, view, actions, id, rect, form),
        Tile::Figures => figures(state, ui, p, frame, view, id, rect, form),
        Tile::Prims => prims(state, ui, p, frame, view, actions, id, rect, form),
    }
}

// ---- the 3D view ----

#[allow(clippy::too_many_arguments)]
fn stage(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    view: &View<'_>,
    actions: &mut Vec<Action>,
    id: egui::Id,
    rect: Rect,
    form: Form,
    pointed: Option<Uuid>,
) -> bool {
    let radius = ui.visuals().window_corner_radius;
    let stroke = widgets::border_stroke(p, Stroke::NONE);
    ui.painter().rect(
        rect,
        radius,
        crate::ui::object_stage::ground_color(p),
        stroke,
        egui::StrokeKind::Inside,
    );
    let outfit = match state.subject {
        Subject::Avatar(_) => aurora_i18n::tr!("inspect-stage-outfit"),
        Subject::Objects => aurora_i18n::tr!("inspect-stage-objects"),
    };
    let title = view.open.map_or(outfit, |item| view.row(item).name.trim());
    let showing = Showing {
        alone: view.open.map(|i| i.root),
        outlined: state.selected,
        pointed: pointed.or_else(|| state.demo_hover.then(|| view.items.get(3).map(|i| i.root)).flatten()),
    };
    let absent = state.stage.stage.model().is_err();
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let ui = &mut child;
    let lines = match (form, view.open) {
        (Form::Small, _) => 0.0,
        (_, None) => 1.0,
        (_, Some(_)) => 3.0,
    };
    let top_room = (10.0 + lines * 17.0) * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    if absent && form != Form::Small {
        missing(state, ui, p, frame, view, rect);
    }
    if let Some(hit) = state.stage.show(ui, p, rect, top_room, showing, title, &|hit| {
        let id = if view.open.is_some() { hit.prim } else { hit.root };
        view.rows.iter().find(|r| r.id == id).map(|r| r.name.clone())
    }) && form != Form::Small
    {
        if state.opened == Some(hit.root) {
            state.selected = Some(hit.prim);
        } else {
            state.open_item(Some(hit.root));
        }
    }
    if form == Form::Small {
        let font = egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
        widgets::paint_elided(ui, title, rect.left_top() + Vec2::new(10.0, 18.0), rect.width() - 44.0, font, p.ink);
        let right = egui::pos2(rect.right() - 8.0, rect.top() + 18.0);
        return board::grow_button(ui, p, id, right, form, outfit);
    }
    // the title over the view, and its buttons on the right
    let top = rect.top() + 20.0;
    let mut right = rect.right() - 8.0;
    let toggled = board::grow_button(ui, p, id, egui::pos2(right, top), form, outfit);
    right -= 30.0;
    if let Some(item) = view.open {
        let row = view.row(item);
        let mut button = |icon: &str, tip: &str, enabled: bool| {
            let pressed = enabled && corner_button(ui, p, id.with(icon), egui::pos2(right - 12.0, top), icon, tip);
            if enabled {
                right -= 28.0;
            }
            pressed
        };
        if button("user-filled", aurora_i18n::tr!("inspect-profile-tip"), row.creator.is_some())
            && let Some(who) = row.creator
        {
            actions.push(Action::Profile {
                who,
                named_here_alone: row.creator_named_here_alone,
            });
        }
        if button("chevron-left", aurora_i18n::tr!("inspect-stage-back"), view.items.len() > 1) {
            state.open_item(None);
        }
        if button("eye-filled", aurora_i18n::tr!("inspect-look-at-tip"), true) {
            actions.push(Action::LookAt(item.root));
        }
    }
    let left = rect.left() + 12.0;
    let font = egui::FontId::new(16.0, crate::ui::fonts::semibold(ui.ctx()));
    widgets::paint_elided(ui, title, egui::pos2(left, top - 1.0), (right - left - 8.0).max(30.0), font, p.ink);
    if let Some(item) = view.open {
        let row = view.row(item);
        let line = 17.0 * crate::ui::fonts::scale(ui.ctx());
        let place = match state.subject {
            Subject::Avatar(_) => point_label(frame, item.point),
            Subject::Objects => aurora_i18n::tr!("inspect-owned-by-name", name = row.owner_name.as_str()),
        };
        let made = aurora_i18n::tr!("inspect-place-made-by", place = place, name = row.creator_name.as_str());
        let small = egui::FontId::proportional(12.0);
        let width = rect.width() - 24.0;
        widgets::paint_elided(ui, &made, egui::pos2(left, top + line + 2.0), width, small.clone(), p.ink);
        let note = row.description.trim();
        widgets::paint_elided(ui, note, egui::pos2(left, top + 2.0 * line + 2.0), width, small, p.muted);
    }
    toggled
}

/// What stands in the place of the view when there is none: the figures of
/// what is inspected.
fn missing(state: &InspectUi, ui: &mut egui::Ui, p: &Palette, _frame: &Frame<'_>, view: &View<'_>, rect: Rect) {
    if state.stage.stage.model().err() == Some(crate::object_stage::Absent::Loading) {
        return;
    }
    let (triangles, vertices, memory, faces) = match view.open {
        Some(i) => (u64::from(i.triangles), u64::from(i.vertices), i.vram, u64::from(i.faces)),
        None => (view.whole.triangles, view.whole.vertices, view.whole.vram, view.whole.faces),
    };
    let n = |v: u64| aurora_i18n::integer(v as i64);
    let cells = [
        (n(triangles), aurora_i18n::tr!("inspect-col-triangles")),
        (n(vertices), aurora_i18n::tr!("inspect-col-vertices")),
        (megabytes(memory), aurora_i18n::tr!("inspect-tile-vram")),
        (n(faces), aurora_i18n::tr!("inspect-col-faces")),
    ];
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + 24.0, rect.bottom() - 110.0),
        egui::pos2(rect.right() - 24.0, rect.bottom() - 14.0),
    );
    if inner.top() < rect.center().y + 30.0 {
        return;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt("inspect_missing").max_rect(inner));
    figure_grid(&mut child, p, &cells.map(|(v, l)| (v, l.to_owned())), 2);
}

fn corner_button(ui: &mut egui::Ui, p: &Palette, id: egui::Id, center: egui::Pos2, icon: &str, tip: &str) -> bool {
    let rect = Rect::from_center_size(center, Vec2::splat(24.0));
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let ink = if resp.hovered() { p.ink } else { p.muted };
    crate::ui::sheet::icon_at(ui, icon, rect.center(), 15.0, ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip).clicked()
}

/// The attachment point's name (avatar_lad.xml).
fn point_label(frame: &Frame<'_>, point: u8) -> String {
    super::list::point_label(frame, point)
}

// ---- the items ----

/// How an item is lit: the accent while it is what is outlined in the world,
/// grey once one of its prims is.
fn lit(state: &InspectUi, item: &Item) -> Lit {
    if state.opened != Some(item.root) {
        // pointed at in the 3D view
        let over = state.opened.is_none() && state.stage.over().is_some_and(|hit| hit.root == item.root);
        if over { Lit::Pointed } else { Lit::Off }
    } else if state.selected == Some(item.root) {
        Lit::Outlined
    } else {
        Lit::On
    }
}

fn weight_of(tier: Tier) -> u8 {
    match tier {
        Tier::Light => 0,
        Tier::Medium => 1,
        Tier::Heavy => 2,
    }
}

/// The figure an item is told by: its complexity, or its land impact.
fn value_badge(ui: &egui::Ui, p: &Palette, right: f32, y: f32, item: &Item, row: &Row, tier: Tier, objects: bool) -> f32 {
    if objects {
        let (text, weight) = match row.land_impact {
            Some(v) => (aurora_i18n::integer(v.round() as i64), land_weight(v)),
            None => ("…".to_owned(), 0),
        };
        return board::badge(ui, p, right, y, text, weight);
    }
    board::badge(ui, p, right, y, model::short(item.complexity), weight_of(tier))
}

/// The badge of a land impact: grey, white from 10, orange from 40 (Area
/// Search's).
pub(crate) fn land_weight(v: f32) -> u8 {
    if v >= 40.0 { 2 } else { u8::from(v >= 10.0) }
}

/// The picture a row of an item starts with: its creator's, or its owner's
/// for objects of the world (a group's is square).
fn row_face(state: &mut InspectUi, ui: &egui::Ui, p: &Palette, frame: &Frame<'_>, at: egui::Pos2, row: &Row) {
    let r = Rect::from_center_size(at, Vec2::splat(18.0));
    let objects = state.subject == Subject::Objects;
    if objects && row.owner_group.is_some() {
        group_face(ui, p, r);
        return;
    }
    let (who, name) = if objects {
        (row.owner, &row.owner_name)
    } else {
        (row.creator, &row.creator_name)
    };
    state.wanted_pics.extend(who);
    widgets::avatar_disc(ui, p, r, who.and_then(|id| frame.pics.get(&id)), name, false);
}

pub(crate) fn group_face(ui: &egui::Ui, p: &Palette, r: Rect) {
    ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, p.raised);
    crate::ui::sheet::icon_at(ui, "users", r.center(), r.width() * 0.66, p.muted);
}

/// One sentence for a screen reader.
fn spoken(state: &InspectUi, frame: &Frame<'_>, row: &Row, item: &Item) -> String {
    let complexity = aurora_i18n::integer(i64::from(item.complexity));
    if state.avatar() {
        aurora_i18n::tr!(
            "inspect-spoken-worn",
            name = row.name.trim(),
            point = point_label(frame, item.point),
            complexity = complexity,
            creator = row.creator_name.as_str()
        )
    } else {
        aurora_i18n::tr!(
            "inspect-spoken-object",
            name = row.name.trim(),
            owner = row.owner_name.as_str(),
            complexity = complexity,
            creator = row.creator_name.as_str()
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn items_tile(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    view: &View<'_>,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> bool {
    let objects = state.subject == Subject::Objects;
    let title = if objects {
        aurora_i18n::tr!("inspect-tab-objects")
    } else {
        aurora_i18n::tr!("inspect-tile-items")
    };
    let (shown, all) = (view.shown.len(), view.items.len());
    let note = if shown != all {
        aurora_i18n::tr!("board-some-of", shown = shown, all = all)
    } else if objects {
        aurora_i18n::tr!("inspect-selected-count", count = all)
    } else {
        aurora_i18n::tr!("inspect-worn-count", count = all)
    };
    let head = board::Head {
        title,
        note: if form == Form::Small { "" } else { &note },
        spoken: &format!("{title}, {note}"),
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => {
            let of = aurora_i18n::tr!("board-of-all", all = all);
            board::figure(ui, p, &aurora_i18n::integer(shown as i64), &of);
        }
        Form::Usual => item_list(state, ui, p, frame, view),
        Form::Grown => item_table(state, ui, p, frame, view),
    })
    .0
}

fn nothing(state: &InspectUi, ui: &mut egui::Ui, p: &Palette, view: &View<'_>) {
    let text = if view.items.is_empty() || state.query.trim().is_empty() {
        aurora_i18n::tr!("inspect-empty").to_owned()
    } else {
        aurora_i18n::tr!("inspect-no-match", query = state.query.trim())
    };
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(12.0).color(p.muted));
}

fn item_list(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, view: &View<'_>) {
    if view.shown.is_empty() {
        nothing(state, ui, p, view);
        return;
    }
    let objects = state.subject == Subject::Objects;
    let nav = board::Nav::begin(ui, "inspect_items", view.shown.len(), 10);
    let height = ROW * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    board::virtual_rows(ui, "inspect_items", view.shown.len(), height, None, &nav, |ui, i, line, _| {
        let item = view.shown[i];
        let row = view.row(item);
        let lit = lit(state, item);
        let resp = board::row_at(ui, p, &nav, i, line, lit);
        let y = line.center().y;
        row_face(state, ui, p, frame, egui::pos2(line.left() + 13.0, y), row);
        let mut right = value_badge(ui, p, line.right() - 6.0, y, item, row, view.tiers[&item.root], objects) - 8.0;
        let aside = state.wide.aside.contains(&item.root);
        let note = if aside {
            aurora_i18n::tr!("inspect-is-aside").to_owned()
        } else if objects {
            String::new()
        } else {
            point_label(frame, item.point)
        };
        if !note.is_empty() && line.width() > 230.0 {
            right = text_before(ui, right, y, &note, 92.0, 11.5, if aside { p.warn } else { p.muted }) - 8.0;
        }
        if lit.chosen() {
            right = board::check(ui, p, right, y);
        }
        let left = line.left() + 28.0;
        let font = name_font(ui, lit.chosen(), 12.5);
        let ink = if aside { p.muted } else { p.ink };
        widgets::paint_elided(ui, row.name.trim(), egui::pos2(left, y), (right - left).max(20.0), font, ink);
        let label = spoken(state, frame, row, item);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, lit.chosen(), &label));
        respond(state, resp, item);
    });
}

/// A click opens an item (again: none), the pointer lights it in 3D.
fn respond(state: &mut InspectUi, resp: egui::Response, item: &Item) {
    if resp.hovered() {
        state.wide.pointed = Some(item.root);
    }
    if resp.clicked() {
        let open = state.opened == Some(item.root);
        state.open_item((!open).then_some(item.root));
    }
}

fn name_font(ui: &egui::Ui, strong: bool, size: f32) -> egui::FontId {
    if strong {
        egui::FontId::new(size, crate::ui::fonts::semibold(ui.ctx()))
    } else {
        egui::FontId::proportional(size)
    }
}

/// A short text that ends at `right`, cut to `width`; returns its left edge.
fn text_before(ui: &egui::Ui, right: f32, y: f32, text: &str, width: f32, size: f32, color: egui::Color32) -> f32 {
    let font = egui::FontId::proportional(size);
    let w = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x
        .min(width);
    widgets::paint_elided(ui, text, egui::pos2(right - w, y), w + 1.0, font, color);
    right - w
}

fn scroll<R>(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .inner
}

/// The grown list: every item with where it is worn (who owns it), its
/// creator and its figures, sorted on any column.
fn item_table(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, view: &View<'_>) {
    let objects = state.subject == Subject::Objects;
    let cols = [
        board::Col::text(aurora_i18n::tr!("inspect-col-item"), 1.4),
        board::Col::text(
            if objects {
                aurora_i18n::tr!("inspect-owned-by")
            } else {
                aurora_i18n::tr!("inspect-worn-on")
            },
            0.9,
        ),
        board::Col::text(aurora_i18n::tr!("inspect-made-by"), 1.0),
        board::Col::figure(
            if objects {
                aurora_i18n::tr!("area-search-land-impact")
            } else {
                aurora_i18n::tr!("inspect-totals-complexity")
            },
            96.0,
        ),
        board::Col::figure(aurora_i18n::tr!("inspect-col-triangles"), 82.0),
        board::Col::figure(aurora_i18n::tr!("inspect-tile-vram"), 78.0),
    ];
    let mut sort = state.wide.sorts.get("items").copied().flatten();
    board::header(ui, p, "inspect_item_heads", &cols, &mut sort);
    state.wide.sorts.insert("items", sort);
    if view.shown.is_empty() {
        nothing(state, ui, p, view);
        return;
    }
    let mut list = view.shown.clone();
    if let Some((c, down)) = sort {
        let text = crate::natural_order::cmp;
        list.sort_by(|a, b| {
            let (ra, rb) = (view.row(a), view.row(b));
            let order = match c {
                0 => text(&ra.name, &rb.name),
                1 if objects => text(&ra.owner_name, &rb.owner_name),
                1 => text(&point_label(frame, a.point), &point_label(frame, b.point)),
                2 => text(&ra.creator_name, &rb.creator_name),
                3 if objects => ra.land_impact.unwrap_or(-1.0).total_cmp(&rb.land_impact.unwrap_or(-1.0)),
                3 => a.complexity.cmp(&b.complexity),
                4 => a.triangles.cmp(&b.triangles),
                _ => a.vram.cmp(&b.vram),
            };
            if down { order.reverse() } else { order }
        });
    }
    let nav = board::Nav::begin(ui, "inspect_item_table", list.len(), 12);
    board::virtual_rows(ui, "inspect_item_table", list.len(), 27.0, None, &nav, |ui, i, line, _| {
        let item = list[i];
        let row = view.row(item);
        let lit = lit(state, item);
        let resp = board::row_at(ui, p, &nav, i, line, lit);
        let cells = board::col_rects(&cols, line);
        let y = line.center().y;
        let name = if !lit.chosen() {
            cells[0]
        } else {
            cells[0].with_max_x(cells[0].right() - 18.0)
        };
        board::cell(ui, name, row.name.trim(), p.ink);
        if lit.chosen() {
            board::check(ui, p, cells[0].right() - 2.0, y);
        }
        if objects {
            let face = Rect::from_center_size(egui::pos2(cells[1].left() + 17.0, y), Vec2::splat(18.0));
            row_face(state, ui, p, frame, face.center(), row);
            board::cell(ui, cells[1].with_min_x(face.right()), &row.owner_name, p.ink);
        } else {
            board::cell(ui, cells[1], &point_label(frame, item.point), p.muted);
        }
        creator_cell(state, ui, p, frame, cells[2], row);
        value_badge(ui, p, cells[3].right() - 8.0, y, item, row, view.tiers[&item.root], objects);
        board::figure_cell(ui, cells[4], &aurora_i18n::integer(i64::from(item.triangles)), p.ink);
        board::figure_cell(ui, cells[5], &megabytes(item.vram), p.ink);
        let label = spoken(state, frame, row, item);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, lit.chosen(), &label));
        respond(state, resp, item);
    });
}

fn creator_cell(state: &mut InspectUi, ui: &egui::Ui, p: &Palette, frame: &Frame<'_>, cell: Rect, row: &Row) {
    let face = Rect::from_center_size(egui::pos2(cell.left() + 17.0, cell.center().y), Vec2::splat(18.0));
    state.wanted_pics.extend(row.creator);
    let tex = row.creator.and_then(|id| frame.pics.get(&id));
    widgets::avatar_disc(ui, p, face, tex, &row.creator_name, false);
    board::cell(ui, cell.with_min_x(face.right()), &row.creator_name, p.ink);
}

// ---- complexity among the people here, or land impact ----

#[allow(clippy::too_many_arguments)]
fn weight(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    view: &View<'_>,
    id: egui::Id,
    rect: Rect,
    form: Form,
    avatar: Uuid,
) -> bool {
    let left_out: u32 = view
        .items
        .iter()
        .filter(|i| state.wide.aside.contains(&i.root))
        .map(|i| i.complexity)
        .sum();
    let total = frame.totals.complexity.saturating_sub(left_out);
    let over = frame.max_complexity > 0 && total > frame.max_complexity;
    let open = view.open.filter(|i| !state.wide.aside.contains(&i.root));
    let subject = people::Subject {
        id: avatar,
        pic: frame.subject_profile,
        name: frame.subject_name,
        total,
        without: open.map(|i| total.saturating_sub(i.complexity)),
        over,
    };
    let title = aurora_i18n::tr!("inspect-totals-complexity");
    let value = aurora_i18n::integer(i64::from(total));
    let ink = if over { p.warn } else { p.ink };
    let summary = complexity_spoken(frame, total, avatar);
    match form {
        Form::Small => {
            let head = board::Head {
                title,
                note: "",
                spoken: &summary,
            };
            board::tile(ui, p, id, rect, form, Some(head), |ui| board::figure(ui, p, &value, "")).0
        }
        Form::Usual => {
            board::surface(ui, p, rect, form);
            let info = ui.interact(rect, id, Sense::hover());
            info.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &summary));
            let inner = rect.shrink2(Vec2::new(10.0, 8.0));
            figure_block(state, ui, p, frame, inner, &value, ink, over, view);
            let toggled = board::grow_button(ui, p, id, egui::pos2(inner.right(), inner.top() + 10.0), form, title);
            let chart = Rect::from_min_max(
                egui::pos2((inner.left() + 130.0).min(inner.center().x), inner.top()),
                egui::pos2(inner.right() - 30.0, inner.bottom() + 2.0),
            );
            people::paint(ui, p, frame, &mut state.wanted_pics, chart, &subject, false);
            toggled
        }
        Form::Grown => {
            let head = board::Head {
                title,
                note: "",
                spoken: &summary,
            };
            board::tile(ui, p, id, rect, form, Some(head), |ui| {
                ui.horizontal(|ui| {
                    let bold = crate::ui::fonts::semibold(ui.ctx());
                    ui.label(RichText::new(&value).size(24.0).family(bold).color(ink));
                    aside_button(state, ui, p, view);
                });
                let (chart, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 96.0), Sense::hover());
                people::paint(ui, p, frame, &mut state.wanted_pics, chart, &subject, true);
                ui.add_space(6.0);
                let mut sort = state.wide.sorts.get("people").copied().unwrap_or(Some((1, true)));
                people::table(ui, p, frame, &mut state.wanted_pics, &subject, &mut sort);
                state.wide.sorts.insert("people", sort);
            })
            .0
        }
    }
}

/// The strip in one sentence: the total, the rank among the people here, the
/// limit.
fn complexity_spoken(frame: &Frame<'_>, total: u32, avatar: Uuid) -> String {
    let others = frame.people.iter().filter(|o| o.id != avatar);
    let heavier = others.clone().filter(|o| o.complexity > total).count();
    aurora_i18n::tr!(
        "inspect-complexity-spoken",
        value = aurora_i18n::integer(i64::from(total)),
        rank = heavier + 1,
        count = others.count() + 1
    )
}

/// The total at the left of the strip, with the button that sets the open
/// item aside under it.
#[allow(clippy::too_many_arguments)]
fn figure_block(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    inner: Rect,
    value: &str,
    ink: egui::Color32,
    over: bool,
    view: &View<'_>,
) {
    let block = inner.with_max_x(inner.left() + 122.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("inspect_total")
            .max_rect(block)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 1.0;
    child.horizontal(|ui| {
        let bold = crate::ui::fonts::semibold(ui.ctx());
        ui.label(RichText::new(value).size(22.0).family(bold).color(ink));
        if over {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            crate::ui::sheet::icon_at(ui, "alert-triangle-filled", r.center(), 16.0, p.warn);
            let limit = aurora_i18n::integer(i64::from(frame.max_complexity));
            resp.on_hover_text(format!(
                "{}\n{}",
                aurora_i18n::tr!("inspect-over-limit"),
                aurora_i18n::tr!("inspect-limit-tip", limit = limit)
            ));
        }
    });
    if view.open.is_some() {
        aside_button(state, &mut child, p, view);
    } else {
        let word = aurora_i18n::tr!("inspect-totals-complexity");
        child.label(RichText::new(word).size(12.0).color(p.muted));
    }
}

fn aside_button(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, view: &View<'_>) {
    let Some(item) = view.open else { return };
    let aside = state.wide.aside.iter().position(|id| *id == item.root);
    let (label, tip) = match aside {
        Some(_) => (aurora_i18n::tr!("inspect-put-back"), aurora_i18n::tr!("inspect-put-back-tip")),
        None => (aurora_i18n::tr!("inspect-set-aside"), aurora_i18n::tr!("inspect-set-aside-tip")),
    };
    let size = Vec2::new(0.0, 24.0);
    if widgets::action_button(ui, p, "", label, widgets::ActionKind::Flat, size)
        .on_hover_text(tip)
        .clicked()
    {
        match aside {
            Some(at) => {
                state.wide.aside.remove(at);
            }
            None => state.wide.aside.push(item.root),
        }
    }
}

/// The land impact of the objects on their parcel.
fn land(ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, view: &View<'_>, id: egui::Id, rect: Rect, form: Form) -> bool {
    let title = aurora_i18n::tr!("area-search-land-impact");
    let known: Vec<f32> = view.items.iter().filter_map(|i| view.row(i).land_impact).collect();
    let total: f32 = known.iter().sum();
    let value = if known.len() < view.items.len() && known.is_empty() {
        "…".to_owned()
    } else {
        aurora_i18n::integer(total.round() as i64)
    };
    let head = board::Head {
        title,
        note: "",
        spoken: &format!("{title}, {value}"),
    };
    if form == Form::Small {
        return board::tile(ui, p, id, rect, form, Some(head), |ui| board::figure(ui, p, &value, "")).0;
    }
    if form == Form::Grown {
        return board::tile(ui, p, id, rect, form, Some(head), |ui| {
            let bold = crate::ui::fonts::semibold(ui.ctx());
            ui.label(RichText::new(&value).size(24.0).family(bold).color(p.ink));
            ui.add_space(6.0);
            let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::hover());
            land_bar(ui, p, frame, bar, total);
            ui.add_space(8.0);
            let cols = [
                board::Col::text(aurora_i18n::tr!("inspect-col-item"), 1.0),
                board::Col::text(aurora_i18n::tr!("inspect-owned-by"), 0.9),
                board::Col::figure(title, 120.0),
                board::Col::figure(aurora_i18n::tr!("inspect-tile-prims"), 80.0),
            ];
            let mut none = None;
            board::header(ui, p, "inspect_land_heads", &cols, &mut none);
            let mut list: Vec<&Item> = view.items.iter().collect();
            list.sort_by(|a, b| {
                let v = |i: &Item| view.row(i).land_impact.unwrap_or(-1.0);
                v(b).total_cmp(&v(a))
            });
            let nav = board::Nav::begin(ui, "inspect_land", list.len(), 10);
            board::virtual_rows(ui, "inspect_land", list.len(), 27.0, None, &nav, |ui, i, line, _| {
                let item = list[i];
                let row = view.row(item);
                board::row_at(ui, p, &nav, i, line, Lit::Off);
                let cells = board::col_rects(&cols, line);
                board::cell(ui, cells[0], row.name.trim(), p.ink);
                board::cell(ui, cells[1], &row.owner_name, p.muted);
                let v = row.land_impact;
                let text = v.map_or("…".to_owned(), |v| aurora_i18n::integer(v.round() as i64));
                board::badge(ui, p, cells[2].right() - 8.0, line.center().y, text, land_weight(v.unwrap_or(0.0)));
                board::figure_cell(ui, cells[3], &aurora_i18n::integer(item.prims.len() as i64), p.ink);
            });
        })
        .0;
    }
    board::surface(ui, p, rect, form);
    let inner = rect.shrink2(Vec2::new(10.0, 8.0));
    let toggled = board::grow_button(ui, p, id, egui::pos2(inner.right(), inner.top() + 10.0), form, title);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("inspect_land_total")
            .max_rect(inner.with_max_x(inner.left() + 122.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 1.0;
    let bold = crate::ui::fonts::semibold(child.ctx());
    child.label(RichText::new(&value).size(22.0).family(bold).color(p.ink));
    child.label(RichText::new(title).size(12.0).color(p.muted));
    let bar = Rect::from_min_max(
        egui::pos2(inner.left() + 130.0, inner.top() + 4.0),
        egui::pos2(inner.right() - 30.0, inner.bottom()),
    );
    land_bar(ui, p, frame, bar, total);
    toggled
}

/// Others, these and what is free, when the objects are on our parcel.
fn land_bar(ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, rect: Rect, these: f32) {
    let Some((used, max)) = frame.parcel.filter(|(_, max)| *max > 0) else {
        let text = aurora_i18n::tr!("inspect-land-unknown");
        widgets::paint_elided(
            ui,
            text,
            rect.left_center(),
            rect.width(),
            egui::FontId::proportional(12.0),
            p.muted,
        );
        return;
    };
    let (used, max) = (used as f32, max as f32);
    let others = (used - these).max(0.0);
    let free = (max - used).max(0.0);
    let total = (others + these + free).max(1.0);
    let text_h = 16.0 * crate::ui::fonts::scale(ui.ctx());
    let bar = rect.with_max_y((rect.bottom() - text_h - 3.0).max(rect.top() + 8.0));
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let gap = 2.0;
    let width = |v: f32| (bar.width() - 2.0 * gap) * v / total;
    // never too thin to be seen
    let mine = width(these).max(6.0).min(bar.width() * 0.9);
    let x1 = bar.left() + width(others).min(bar.width() - mine - gap);
    let others_r = bar.with_max_x(x1);
    let these_r = Rect::from_min_max(egui::pos2(x1 + gap, bar.top()), egui::pos2(x1 + gap + mine, bar.bottom()));
    let free_r = bar.with_min_x((these_r.right() + gap).min(bar.right()));
    ui.painter().rect_filled(others_r, radius, p.muted.gamma_multiply(0.7));
    ui.painter().rect_filled(these_r, radius, p.ink);
    ui.painter()
        .rect_stroke(free_r, radius, Stroke::new(1.0, p.muted), egui::StrokeKind::Inside);
    let y = rect.bottom() - text_h / 2.0;
    let n = |v: f32| aurora_i18n::integer(v.round() as i64);
    let others_t = aurora_i18n::tr!("inspect-land-others", n = n(others));
    let these_t = aurora_i18n::tr!("inspect-land-these", n = n(these));
    let free_t = aurora_i18n::tr!("inspect-land-free", n = n(free));
    let font = egui::FontId::proportional(11.5);
    let w = |t: &str| ui.painter().layout_no_wrap(t.to_owned(), font.clone(), p.ink).size().x;
    let (wo, wt, wf) = (w(&others_t), w(&these_t), w(&free_t));
    widgets::paint_elided(ui, &others_t, egui::pos2(bar.left(), y), wo + 1.0, font.clone(), p.muted);
    let mid = (these_r.center().x - wt / 2.0).clamp(bar.left() + wo + 8.0, (bar.right() - wf - wt - 8.0).max(bar.left() + wo + 8.0));
    widgets::paint_elided(ui, &these_t, egui::pos2(mid, y), wt + 1.0, font.clone(), p.ink);
    widgets::paint_elided(ui, &free_t, egui::pos2(bar.right() - wf, y), wf + 1.0, font, p.muted);
    ui.interact(rect, ui.scope_id().with("inspect_land_bar"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{others_t}, {these_t}, {free_t}")));
}

// ---- who made them ----

/// The creators of `items`: their id and shown name, whether only this window
/// names them, and their items, most first.
struct Maker<'a> {
    id: Option<Uuid>,
    name: String,
    here_alone: bool,
    items: Vec<&'a Item>,
}

fn makers_of<'a>(rows: &[Row], items: &[&'a Item]) -> Vec<Maker<'a>> {
    let mut out: Vec<Maker<'a>> = Vec::new();
    for item in items {
        let row = &rows[item.first];
        match out.iter_mut().find(|m| m.id == row.creator && m.name == row.creator_name) {
            Some(m) => m.items.push(item),
            None => out.push(Maker {
                id: row.creator,
                name: row.creator_name.clone(),
                here_alone: row.creator_named_here_alone,
                items: vec![item],
            }),
        }
    }
    out.sort_by(|a, b| {
        b.items
            .len()
            .cmp(&a.items.len())
            .then_with(|| crate::natural_order::cmp(&a.name, &b.name))
    });
    out
}

#[allow(clippy::too_many_arguments)]
fn makers(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    view: &View<'_>,
    actions: &mut Vec<Action>,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> bool {
    let title = aurora_i18n::tr!("inspect-made-by");
    // narrowed to a maker, the others stay to be picked instead
    let base = match &state.wide.pick {
        Some(Pick::Maker(..)) => &view.found,
        _ => &view.shown,
    };
    let list = makers_of(view.rows, base);
    let note = aurora_i18n::integer(list.len() as i64);
    let head = board::Head {
        title,
        note: if form == Form::Usual && view.open.is_some() { "" } else { &note },
        spoken: &format!("{title}, {note}"),
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => board::figure(ui, p, &note, aurora_i18n::tr!("inspect-creators-word")),
        Form::Usual => {
            if let Some(item) = view.open {
                let row = view.row(item);
                let made = Pick::Maker(row.creator, row.creator_name.clone());
                let count = view.items.iter().filter(|i| made.keeps(view.rows, i)).count();
                let line = aurora_i18n::tr!("inspect-made-of", count = count, all = view.items.len());
                if person(state, ui, p, frame, row.creator, None, &row.creator_name, &line)
                    && let Some(who) = row.creator
                {
                    actions.push(Action::Profile {
                        who,
                        named_here_alone: row.creator_named_here_alone,
                    });
                }
                return;
            }
            let (count, more) = fitting(ui, list.len());
            let nav = board::Nav::begin(ui, "inspect_makers", count, 3);
            for (i, m) in list.iter().take(count).enumerate() {
                let pick = Pick::Maker(m.id, m.name.clone());
                let on = state.wide.pick.as_ref() == Some(&pick);
                let (line, resp) = board::row(ui, p, &nav, i, ROW, if on { Lit::On } else { Lit::Off });
                let face = Rect::from_center_size(egui::pos2(line.left() + 11.0, line.center().y), Vec2::splat(18.0));
                state.wanted_pics.extend(m.id);
                widgets::avatar_disc(ui, p, face, m.id.and_then(|id| frame.pics.get(&id)), &m.name, false);
                let n = aurora_i18n::integer(m.items.len() as i64);
                let right = board::text_ending(ui, line.right() - 5.0, line.center().y, &n, 11.5, p.muted);
                let left = face.right() + 6.0;
                let at = egui::pos2(left, line.center().y);
                widgets::paint_elided(ui, &m.name, at, right - left - 6.0, name_font(ui, on, 12.5), p.ink);
                let label = format!("{}, {n}", m.name);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
                if resp.clicked() {
                    state.wide.pick = (!on).then_some(pick);
                }
            }
            and_more(ui, p, more);
        }
        Form::Grown => {
            let cols = [
                board::Col::text(aurora_i18n::tr!("inspect-col-item"), 1.3),
                board::Col::text(aurora_i18n::tr!("inspect-worn-on"), 1.0),
                board::Col::figure(aurora_i18n::tr!("inspect-totals-complexity"), 100.0),
                board::Col::figure(aurora_i18n::tr!("inspect-tile-vram"), 84.0),
            ];
            let mut none = None;
            board::header(ui, p, "inspect_maker_heads", &cols, &mut none);
            let count = list.iter().map(|m| m.items.len()).sum();
            let nav = board::Nav::begin(ui, "inspect_maker_table", count, 10);
            let mut at = 0;
            scroll(ui, "inspect_maker_table", |ui| {
                for m in &list {
                    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
                    let y = line.bottom() - 13.0;
                    let face = Rect::from_center_size(egui::pos2(line.left() + 17.0, y), Vec2::splat(18.0));
                    state.wanted_pics.extend(m.id);
                    widgets::avatar_disc(ui, p, face, m.id.and_then(|id| frame.pics.get(&id)), &m.name, false);
                    let note = group_note(&m.items);
                    let mut right = line.right() - 8.0;
                    if let Some(who) = m.id {
                        if corner_button(
                            ui,
                            p,
                            id.with(("maker", who)),
                            egui::pos2(right - 12.0, y),
                            "external-link-filled",
                            aurora_i18n::tr!("inspect-profile-tip"),
                        ) {
                            actions.push(Action::Profile {
                                who,
                                named_here_alone: m.here_alone,
                            });
                        }
                        right -= 30.0;
                    }
                    let edge = board::text_ending(ui, right, y, &note, 11.5, p.muted);
                    let left = face.right() + 7.0;
                    widgets::paint_elided(
                        ui,
                        &m.name,
                        egui::pos2(left, y),
                        edge - left - 8.0,
                        name_font(ui, true, 12.5),
                        p.ink,
                    );
                    for item in &m.items {
                        let row = view.row(item);
                        let lit = lit(state, item);
                        let (line, resp) = board::row(ui, p, &nav, at, 27.0, lit);
                        at += 1;
                        let cells = board::col_rects(&cols, line);
                        board::cell(ui, cells[0], row.name.trim(), p.ink);
                        let place = if state.avatar() {
                            point_label(frame, item.point)
                        } else {
                            row.owner_name.clone()
                        };
                        board::cell(ui, cells[1], &place, p.muted);
                        let value = model::short(item.complexity);
                        board::badge(
                            ui,
                            p,
                            cells[2].right() - 8.0,
                            line.center().y,
                            value,
                            weight_of(view.tiers[&item.root]),
                        );
                        board::figure_cell(ui, cells[3], &megabytes(item.vram), p.ink);
                        let label = spoken(state, frame, row, item);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, lit.chosen(), &label));
                        respond(state, resp, item);
                    }
                }
            });
        }
    })
    .0
}

/// "3 items · 5,500 complexity".
fn group_note(items: &[&Item]) -> String {
    let complexity: u32 = items.iter().map(|i| i.complexity).sum();
    [
        aurora_i18n::tr!("inspect-summary-items", count = items.len()),
        aurora_i18n::tr!("inspect-section-complexity", value = aurora_i18n::integer(i64::from(complexity))),
    ]
    .join(" · ")
}

/// How many rows of a short list fit, with the line for the others.
fn fitting(ui: &egui::Ui, count: usize) -> (usize, usize) {
    let room = ui.available_height();
    if count as f32 * ROW <= room + 1.0 {
        return (count, 0);
    }
    let fit = (((room - 17.0) / ROW).floor().max(1.0) as usize).min(count);
    (fit, count - fit)
}

fn and_more(ui: &mut egui::Ui, p: &Palette, more: usize) {
    if more > 0 {
        ui.add_space(2.0);
        let text = aurora_i18n::tr!("board-more", count = more);
        ui.label(RichText::new(text).size(11.5).color(p.muted));
    }
}

/// Someone, or a group, with the button that opens their profile at the foot
/// of the tile. True when it was pressed.
#[allow(clippy::too_many_arguments)]
fn person(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    who: Option<Uuid>,
    group: Option<Uuid>,
    name: &str,
    line: &str,
) -> bool {
    let (top, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::hover());
    let face = Rect::from_center_size(egui::pos2(top.left() + 16.0, top.center().y), Vec2::splat(32.0));
    if group.is_some() {
        group_face(ui, p, face);
    } else {
        state.wanted_pics.extend(who);
        widgets::avatar_disc(ui, p, face, who.and_then(|id| frame.pics.get(&id)), name, false);
    }
    two_lines(ui, p, top, face.right() + 8.0, name, line);
    foot_button(
        ui,
        p,
        if group.is_some() {
            aurora_i18n::tr!("common-group")
        } else {
            aurora_i18n::tr!("common-profile")
        },
        who.is_some() || group.is_some(),
    )
}

fn two_lines(ui: &egui::Ui, p: &Palette, rect: Rect, left: f32, first: &str, second: &str) {
    let width = (rect.right() - left).max(20.0);
    let y = rect.center().y;
    let off = 8.0 * crate::ui::fonts::scale(ui.ctx());
    widgets::paint_elided(ui, first, egui::pos2(left, y - off), width, name_font(ui, true, 13.0), p.ink);
    widgets::paint_elided(
        ui,
        second,
        egui::pos2(left, y + off),
        width,
        egui::FontId::proportional(11.5),
        p.muted,
    );
}

fn foot_button(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool) -> bool {
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        widgets::action_button_if(ui, p, "", label, widgets::ActionKind::Flat, Vec2::new(0.0, 24.0), enabled).clicked()
    })
    .inner
}

// ---- where they are worn, or who owns them ----

/// A row of the Worn on or Owned by tile.
struct Place<'a> {
    pick: Pick,
    title: String,
    who: Option<Uuid>,
    group: Option<Uuid>,
    items: Vec<&'a Item>,
}

fn places_of<'a>(rows: &[Row], items: &[&'a Item], avatar: bool) -> Vec<Place<'a>> {
    if avatar {
        return Area::ORDER
            .into_iter()
            .filter_map(|area| {
                let items: Vec<&Item> = items.iter().copied().filter(|i| i.area == area).collect();
                (!items.is_empty()).then(|| Place {
                    pick: Pick::Area(area),
                    title: area.title().to_owned(),
                    who: None,
                    group: None,
                    items,
                })
            })
            .collect();
    }
    let mut out: Vec<Place<'a>> = Vec::new();
    for item in items {
        let row = &rows[item.first];
        match out.iter_mut().find(|o| o.title == row.owner_name) {
            Some(o) => o.items.push(item),
            None => out.push(Place {
                pick: Pick::Owner(row.owner_name.clone()),
                title: row.owner_name.clone(),
                who: row.owner,
                group: row.owner_group,
                items: vec![item],
            }),
        }
    }
    let land = |o: &Place<'_>| -> f32 { o.items.iter().filter_map(|i| rows[i.first].land_impact).sum() };
    out.sort_by(|a, b| land(b).total_cmp(&land(a)).then(b.items.len().cmp(&a.items.len())));
    out
}

#[allow(clippy::too_many_arguments)]
fn places(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    view: &View<'_>,
    actions: &mut Vec<Action>,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> bool {
    let avatar = state.avatar();
    let title = if avatar {
        aurora_i18n::tr!("inspect-worn-on")
    } else {
        aurora_i18n::tr!("inspect-owned-by")
    };
    let base = match &state.wide.pick {
        Some(Pick::Area(_) | Pick::Owner(_)) => &view.found,
        _ => &view.shown,
    };
    let list = places_of(view.rows, base, avatar);
    let n = aurora_i18n::integer(list.len() as i64);
    let head = board::Head {
        title,
        note: "",
        spoken: &format!("{title}, {n}"),
    };
    // what the badge of a place says
    let badge = |items: &[&Item]| -> String {
        if avatar {
            model::short(items.iter().map(|i| i.complexity).sum())
        } else {
            let sum: f32 = items.iter().filter_map(|i| view.row(i).land_impact).sum();
            aurora_i18n::integer(sum.round() as i64)
        }
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => {
            let word = if avatar {
                aurora_i18n::tr!("inspect-areas-word")
            } else {
                aurora_i18n::tr!("inspect-owners-word")
            };
            board::figure(ui, p, &n, word);
        }
        Form::Usual => {
            if let Some(item) = view.open {
                let row = view.row(item);
                if avatar {
                    let (top, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::hover());
                    let pin = egui::pos2(top.left() + 16.0, top.center().y);
                    ui.painter().circle_filled(pin, 16.0, p.raised);
                    crate::ui::sheet::icon_at(ui, "map-pin-filled", pin, 16.0, p.ink);
                    two_lines(ui, p, top, top.left() + 40.0, &point_label(frame, item.point), item.area.title());
                    if foot_button(ui, p, aurora_i18n::tr!("inspect-look-at"), true) {
                        actions.push(Action::LookAt(item.root));
                    }
                } else if person(
                    state,
                    ui,
                    p,
                    frame,
                    row.owner,
                    row.owner_group,
                    &row.owner_name,
                    &aurora_i18n::tr!("inspect-prims-count", count = item.prims.len()),
                ) {
                    open_owner(row, actions);
                }
                return;
            }
            let (count, more) = fitting(ui, list.len());
            let nav = board::Nav::begin(ui, "inspect_places", count, 3);
            for (i, place) in list.iter().take(count).enumerate() {
                let on = state.wide.pick.as_ref() == Some(&place.pick);
                let (line, resp) = board::row(ui, p, &nav, i, ROW, if on { Lit::On } else { Lit::Off });
                let y = line.center().y;
                let mut left = line.left() + 4.0;
                if !avatar {
                    let face = Rect::from_center_size(egui::pos2(line.left() + 11.0, y), Vec2::splat(18.0));
                    if place.group.is_some() {
                        group_face(ui, p, face);
                    } else {
                        state.wanted_pics.extend(place.who);
                        let tex = place.who.and_then(|id| frame.pics.get(&id));
                        widgets::avatar_disc(ui, p, face, tex, &place.title, false);
                    }
                    left = face.right() + 6.0;
                }
                let right = board::badge(ui, p, line.right() - 4.0, y, badge(&place.items), 1) - 8.0;
                let n = aurora_i18n::integer(place.items.len() as i64);
                let right = board::text_ending(ui, right, y, &n, 11.5, p.muted) - 6.0;
                let font = name_font(ui, on, 12.5);
                widgets::paint_elided(ui, &place.title, egui::pos2(left, y), (right - left).max(20.0), font, p.ink);
                let label = format!("{}, {n}", place.title);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
                if resp.clicked() {
                    state.wide.pick = (!on).then(|| place.pick.clone());
                }
            }
            and_more(ui, p, more);
        }
        Form::Grown => {
            let cols = [
                board::Col::text(aurora_i18n::tr!("inspect-col-item"), 1.4),
                board::Col::text(
                    if avatar {
                        aurora_i18n::tr!("inspect-point")
                    } else {
                        aurora_i18n::tr!("inspect-col-description")
                    },
                    1.0,
                ),
                board::Col::text(aurora_i18n::tr!("inspect-made-by"), 1.0),
                board::Col::figure(
                    if avatar {
                        aurora_i18n::tr!("inspect-totals-complexity")
                    } else {
                        aurora_i18n::tr!("area-search-land-impact")
                    },
                    100.0,
                ),
            ];
            let mut none = None;
            board::header(ui, p, "inspect_place_heads", &cols, &mut none);
            let count = list.iter().map(|m| m.items.len()).sum();
            let nav = board::Nav::begin(ui, "inspect_place_table", count, 10);
            let mut at = 0;
            scroll(ui, "inspect_place_table", |ui| {
                for place in &list {
                    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
                    let y = line.bottom() - 13.0;
                    let edge = board::text_ending(ui, line.right() - 8.0, y, &group_note(&place.items), 11.5, p.muted);
                    let left = line.left() + 8.0;
                    widgets::paint_elided(
                        ui,
                        &place.title,
                        egui::pos2(left, y),
                        edge - left - 8.0,
                        name_font(ui, true, 12.5),
                        p.ink,
                    );
                    for item in &place.items {
                        let row = view.row(item);
                        let lit = lit(state, item);
                        let (line, resp) = board::row(ui, p, &nav, at, 27.0, lit);
                        at += 1;
                        let cells = board::col_rects(&cols, line);
                        board::cell(ui, cells[0], row.name.trim(), p.ink);
                        let second = if avatar {
                            point_label(frame, item.point)
                        } else {
                            row.description.trim().to_owned()
                        };
                        board::cell(ui, cells[1], &second, p.muted);
                        creator_cell(state, ui, p, frame, cells[2], row);
                        value_badge(
                            ui,
                            p,
                            cells[3].right() - 8.0,
                            line.center().y,
                            item,
                            row,
                            view.tiers[&item.root],
                            !avatar,
                        );
                        let label = spoken(state, frame, row, item);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, lit.chosen(), &label));
                        respond(state, resp, item);
                    }
                }
            });
        }
    })
    .0
}

/// The owner's profile, or their group's.
fn open_owner(row: &Row, actions: &mut Vec<Action>) {
    if let Some(group) = row.owner_group {
        actions.push(Action::Group(group));
    } else if let Some(who) = row.owner {
        actions.push(Action::Profile {
            who,
            named_here_alone: false,
        });
    }
}

// ---- figures ----

/// Figures in columns: the value over its word.
fn figure_grid(ui: &mut egui::Ui, p: &Palette, cells: &[(String, String)], columns: usize) {
    let rows = cells.len().div_ceil(columns);
    let area = ui.available_rect_before_wrap();
    let size = Vec2::new(area.width() / columns as f32, area.height() / rows.max(1) as f32);
    let scale = crate::ui::fonts::scale(ui.ctx());
    for (i, (value, word)) in cells.iter().enumerate() {
        let min = area.min + Vec2::new((i % columns) as f32 * size.x, (i / columns) as f32 * size.y);
        let y = min.y + size.y / 2.0;
        let bold = name_font(ui, true, 13.5);
        widgets::paint_elided(ui, value, egui::pos2(min.x, y - 8.0 * scale), size.x - 4.0, bold, p.ink);
        let small = egui::FontId::proportional(11.0);
        widgets::paint_elided(ui, word, egui::pos2(min.x, y + 7.0 * scale), size.x - 4.0, small, p.muted);
        let r = Rect::from_min_size(min, size);
        ui.interact(r, ui.scope_id().with(("figure", i)), Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{word}, {value}")));
    }
}

#[allow(clippy::too_many_arguments)]
fn figures(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    view: &View<'_>,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> bool {
    let title = aurora_i18n::tr!("inspect-tile-figures");
    let head = board::Head {
        title,
        note: "",
        spoken: title,
    };
    let narrowed = view.shown.len() != view.items.len();
    let mut sum = view.whole;
    if narrowed {
        let kept: Vec<Item> = view.shown.iter().map(|i| (*i).clone()).collect();
        sum = Whole::of(&kept);
    } else {
        // (a texture two prims share counts once, as the Legacy totals)
        sum.texture_memory = frame.totals.texture_memory;
        sum.vram = frame.totals.vram;
    }
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => {
            let word = aurora_i18n::tr!("inspect-triangles-word");
            board::figure(ui, p, &model::short(sum.triangles.min(u64::from(u32::MAX)) as u32), word);
        }
        Form::Usual => {
            let short = |v: u64| model::short(v.min(u64::from(u32::MAX)) as u32);
            let word = |word: &str, part: u64, whole: u64| {
                if view.open.is_some() && view.items.len() > 1 {
                    let percent = aurora_i18n::tr!("inspect-percent", percent = model::share(part, whole));
                    format!("{word} · {percent}")
                } else {
                    word.to_owned()
                }
            };
            let w = &view.whole;
            let (t, v, g, m, f, n) = match view.open {
                Some(i) => (
                    u64::from(i.triangles),
                    u64::from(i.vertices),
                    i.vram,
                    i.texture_memory,
                    u64::from(i.faces),
                    i.prims.len(),
                ),
                None => (
                    sum.triangles,
                    sum.vertices,
                    sum.vram,
                    sum.texture_memory,
                    sum.faces,
                    view.shown.iter().map(|i| i.prims.len()).sum(),
                ),
            };
            let cells = [
                (short(t), word(aurora_i18n::tr!("inspect-col-triangles"), t, w.triangles)),
                (short(v), word(aurora_i18n::tr!("inspect-col-vertices"), v, w.vertices)),
                (megabytes(g), word(aurora_i18n::tr!("inspect-tile-vram"), g, w.vram)),
                (megabytes(m), word(aurora_i18n::tr!("inspect-totals-textures"), m, w.texture_memory)),
                (short(f), word(aurora_i18n::tr!("inspect-col-faces"), f, w.faces)),
                (aurora_i18n::integer(n as i64), aurora_i18n::tr!("inspect-tile-prims").to_owned()),
            ];
            figure_grid(ui, p, &cells, 2);
        }
        Form::Grown => {
            let t = frame.totals;
            let n = |v: u32| aurora_i18n::integer(i64::from(v));
            let kb = |v: u64| aurora_i18n::tr!("inspect-kilobytes", n = kilobytes(v));
            // "Total stats" of floater_inspect.xml, all ten
            let tiles = [
                (n(t.objects), aurora_i18n::tr!("inspect-tab-objects"), None),
                (n(t.prims), aurora_i18n::tr!("inspect-tile-prims"), None),
                (n(t.faces), aurora_i18n::tr!("inspect-col-faces"), None),
                (n(t.visible_faces), aurora_i18n::tr!("inspect-totals-visible"), None),
                (n(t.vertices), aurora_i18n::tr!("inspect-col-vertices"), None),
                (n(t.triangles), aurora_i18n::tr!("inspect-col-triangles"), None),
                (n(t.complexity), aurora_i18n::tr!("inspect-totals-complexity"), None),
                (n(t.textures), aurora_i18n::tr!("inspect-totals-textures"), None),
                (kb(t.texture_memory), aurora_i18n::tr!("inspect-tile-tmem"), None),
                (kb(t.vram), aurora_i18n::tr!("inspect-tile-vram"), None),
            ];
            super::list::tiles(ui, p, &tiles, if ui.available_width() > 480.0 { 5 } else { 3 });
            let cols = [
                board::Col::text(aurora_i18n::tr!("inspect-col-item"), 1.0),
                board::Col::figure(aurora_i18n::tr!("inspect-col-triangles"), 84.0),
                board::Col::figure(aurora_i18n::tr!("inspect-col-vertices"), 80.0),
                board::Col::figure(aurora_i18n::tr!("inspect-col-faces"), 64.0),
                board::Col::figure(aurora_i18n::tr!("inspect-tile-vram"), 80.0),
                board::Col::figure(aurora_i18n::tr!("inspect-totals-textures"), 84.0),
            ];
            let mut sort = state.wide.sorts.get("figures").copied().unwrap_or(Some((1, true)));
            board::header(ui, p, "inspect_figure_heads", &cols, &mut sort);
            state.wide.sorts.insert("figures", sort);
            let mut list = view.shown.clone();
            if let Some((c, down)) = sort {
                list.sort_by(|a, b| {
                    let order = match c {
                        0 => crate::natural_order::cmp(&view.row(a).name, &view.row(b).name),
                        1 => a.triangles.cmp(&b.triangles),
                        2 => a.vertices.cmp(&b.vertices),
                        3 => a.faces.cmp(&b.faces),
                        4 => a.vram.cmp(&b.vram),
                        _ => a.texture_memory.cmp(&b.texture_memory),
                    };
                    if down { order.reverse() } else { order }
                });
            }
            let nav = board::Nav::begin(ui, "inspect_figure_table", list.len(), 10);
            board::virtual_rows(ui, "inspect_figure_table", list.len(), 27.0, None, &nav, |ui, i, line, _| {
                let item = list[i];
                let row = view.row(item);
                let lit = lit(state, item);
                let resp = board::row_at(ui, p, &nav, i, line, lit);
                let cells = board::col_rects(&cols, line);
                board::cell(ui, cells[0], row.name.trim(), p.ink);
                board::figure_cell(ui, cells[1], &n(item.triangles), p.ink);
                board::figure_cell(ui, cells[2], &n(item.vertices), p.ink);
                board::figure_cell(ui, cells[3], &n(item.faces), p.ink);
                board::figure_cell(ui, cells[4], &megabytes(item.vram), p.ink);
                board::figure_cell(ui, cells[5], &megabytes(item.texture_memory), p.ink);
                let label = spoken(state, frame, row, item);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, lit.chosen(), &label));
                respond(state, resp, item);
            });
        }
    })
    .0
}

// ---- prims ----

#[allow(clippy::too_many_arguments)]
fn prims(
    state: &mut InspectUi,
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &mut Frame<'_>,
    view: &View<'_>,
    actions: &mut Vec<Action>,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> bool {
    let title = aurora_i18n::tr!("inspect-tile-prims");
    let count: usize = match view.open {
        Some(item) => item.prims.len(),
        None => view.shown.iter().map(|i| i.prims.len()).sum(),
    };
    let n = aurora_i18n::integer(count as i64);
    let note = if form == Form::Grown && state.sort.is_none() {
        aurora_i18n::tr!("inspect-linkset-order", count = n.as_str())
    } else {
        n.clone()
    };
    let head = board::Head {
        title,
        note: if form == Form::Small { "" } else { &note },
        spoken: &format!("{title}, {note}"),
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => {
            let word = match view.open {
                Some(item) => view.row(item).name.trim().to_owned(),
                None => aurora_i18n::tr!("inspect-summary-items", count = view.shown.len()),
            };
            board::figure(ui, p, &n, &word);
        }
        Form::Usual => match view.open {
            Some(item) => open_prims(state, ui, p, view, item),
            None => {
                // the items with the most prims
                let mut list = view.shown.clone();
                list.sort_by_key(|i| std::cmp::Reverse(i.prims.len()));
                let (count, more) = fitting(ui, list.len());
                let nav = board::Nav::begin(ui, "inspect_prim_items", count, 3);
                for (i, item) in list.iter().take(count).enumerate() {
                    let (line, resp) = board::row(ui, p, &nav, i, ROW, Lit::Off);
                    let y = line.center().y;
                    let n = aurora_i18n::integer(item.prims.len() as i64);
                    let right = board::text_ending(ui, line.right() - 5.0, y, &n, 11.5, p.muted);
                    let name = view.row(item).name.trim();
                    let left = line.left() + 4.0;
                    let font = egui::FontId::proportional(12.5);
                    widgets::paint_elided(ui, name, egui::pos2(left, y), right - left - 6.0, font, p.ink);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{name}, {n}")));
                    respond(state, resp, item);
                }
                and_more(ui, p, more);
            }
        },
        Form::Grown => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                for column in Column::ALL.into_iter().skip(1) {
                    if board::pill(ui, p, column.title(), super::shown(*frame.columns, column)) {
                        *frame.columns ^= column.bit();
                    }
                }
                let default = if state.avatar() {
                    super::AVATAR_COLUMNS
                } else {
                    super::ALL_COLUMNS
                };
                if *frame.columns != default
                    && widgets::action_button(
                        ui,
                        p,
                        "",
                        aurora_i18n::tr!("inspect-reset-columns"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 26.0),
                    )
                    .clicked()
                {
                    *frame.columns = default;
                }
            });
            ui.add_space(4.0);
            let kept: Vec<&Row> = match view.open {
                Some(item) => item.prims.iter().map(|i| &view.rows[*i]).collect(),
                None => view.shown.iter().flat_map(|i| i.prims.iter().map(|r| &view.rows[*r])).collect(),
            };
            details::table(state, ui, p, kept, *frame.columns, actions);
        }
    })
    .0
}

/// The prims of the open item: one is outlined in the world.
fn open_prims(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, view: &View<'_>, item: &Item) {
    let nav = board::Nav::begin(ui, "inspect_open_prims", item.prims.len(), 4);
    let height = ROW * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    egui::ScrollArea::vertical()
        .id_salt("inspect_open_prims")
        .auto_shrink([false, false])
        .show_rows(ui, height, item.prims.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let prim = &view.rows[item.prims[i]];
                let on = state.selected == Some(prim.id);
                let (line, resp) = board::row(ui, p, &nav, i, height, if on { Lit::Outlined } else { Lit::Off });
                let y = line.center().y;
                let n = model::short(prim.triangles);
                let mut right = board::text_ending(ui, line.right() - 5.0, y, &n, 11.5, p.muted) - 4.0;
                if on {
                    right = board::check(ui, p, right, y);
                }
                let left = line.left() + if prim.root { 4.0 } else { 12.0 };
                let name = prim.name.trim();
                widgets::paint_elided(ui, name, egui::pos2(left, y), right - left, name_font(ui, on, 12.5), p.ink);
                let label = format!("{name}, {n} {}", aurora_i18n::tr!("inspect-col-triangles"));
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
                if resp.clicked() {
                    state.selected = Some(prim.id);
                }
            }
        });
}

// ---- the foot ----

fn footer(state: &InspectUi, ui: &mut egui::Ui, p: &Palette, _frame: &Frame<'_>, view: &View<'_>, actions: &mut Vec<Action>) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        let row = state.selected.and_then(|id| view.rows.iter().find(|r| r.id == id));
        let row = row.filter(|_| view.open.is_some() || state.wide.grown == Some(Tile::Prims));
        let text = match row {
            Some(row) => aurora_i18n::tr!("inspect-outlined", name = row.name.trim()),
            None => {
                let makers = makers_of(view.rows, &view.shown).len();
                let items = if state.avatar() {
                    aurora_i18n::tr!("inspect-summary-items", count = view.shown.len())
                } else {
                    aurora_i18n::tr!("inspect-objects-title", count = view.shown.len())
                };
                aurora_i18n::tr!("inspect-by-creators", items = items, count = makers)
            }
        };
        let owner = aurora_i18n::tr!("inspect-owner-button");
        let group = aurora_i18n::tr!("inspect-group-button");
        let creator = aurora_i18n::tr!("inspect-creator-button");
        let owner = if row.is_some_and(|r| r.owner_group.is_some()) {
            group
        } else {
            owner
        };
        widgets::status_dot(ui, p.violet);
        let buttons =
            widgets::action_button_fit(ui, "", owner) + widgets::action_button_fit(ui, "", creator) + 3.0 * ui.spacing().item_spacing.x;
        let room = (ui.available_width() - buttons).max(40.0);
        ui.allocate_ui_with_layout(Vec2::new(room, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(room);
            ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.muted)).truncate());
        });
        // LLFloaterInspect::onClickOwnerProfile / onClickCreatorProfile: of
        // the outlined prim (a group's owner opens the group, where the
        // Legacy button is greyed)
        let flat = widgets::ActionKind::Flat;
        let has_owner = row.is_some_and(|r| r.owner.is_some() || r.owner_group.is_some());
        if widgets::action_button_if(ui, p, "", owner, flat, Vec2::new(0.0, 26.0), has_owner)
            .on_hover_text(aurora_i18n::tr!("inspect-owner-profile-tip"))
            .clicked()
            && let Some(row) = row
        {
            open_owner(row, actions);
        }
        let has_creator = row.is_some_and(|r| r.creator.is_some());
        if widgets::action_button_if(ui, p, "", creator, flat, Vec2::new(0.0, 26.0), has_creator)
            .on_hover_text(aurora_i18n::tr!("inspect-creator-profile-tip"))
            .clicked()
            && let Some(row) = row
            && let Some(who) = row.creator
        {
            actions.push(Action::Profile {
                who,
                named_here_alone: row.creator_named_here_alone,
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: u128, point: u8, maker: &str, owner: &str) -> Row {
        Row {
            id: Uuid::from_u128(n),
            root: true,
            name: format!("Item {n}"),
            point,
            creator_name: maker.into(),
            owner_name: owner.into(),
            land_impact: Some(n as f32),
            complexity: 100,
            ..Row::default()
        }
    }

    #[test]
    fn a_pick_narrows_to_a_maker_an_area_or_an_owner() {
        let rows = [row(1, 2, "Ivy", "Teo"), row(2, 7, "Ivy", "Noor"), row(3, 2, "Wren", "Noor")];
        let items = model::items(&rows);
        let kept = |pick: &Pick| -> Vec<usize> { items.iter().filter(|i| pick.keeps(&rows, i)).map(|i| i.first).collect() };
        assert_eq!(kept(&Pick::Maker(None, "Ivy".into())), [0, 1]);
        assert_eq!(kept(&Pick::Area(Area::Head)), [0, 2]);
        assert_eq!(kept(&Pick::Owner("Noor".into())), [1, 2]);
    }

    #[test]
    fn makers_come_most_items_first_and_owners_by_land_impact() {
        let rows = [
            row(1, 0, "Ivy", "Teo"),
            row(2, 0, "Wren", "Teo"),
            row(3, 0, "Wren", "Noor"),
            row(9, 0, "Ivy", "Noor"),
        ];
        let items = model::items(&rows);
        let all: Vec<&Item> = items.iter().collect();
        let names: Vec<String> = makers_of(&rows, &all).into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["Ivy", "Wren"], "as many each: by name");
        let owners: Vec<String> = places_of(&rows, &all, false).into_iter().map(|o| o.title).collect();
        assert_eq!(owners, ["Noor", "Teo"]);
    }

    #[test]
    fn areas_go_head_to_toe_and_leave_out_the_empty_ones() {
        let rows = [row(1, 7, "Ivy", ""), row(2, 2, "Ivy", "")];
        let items = model::items(&rows);
        let all: Vec<&Item> = items.iter().collect();
        let picks: Vec<Pick> = places_of(&rows, &all, true).into_iter().map(|o| o.pick).collect();
        assert_eq!(picks, [Pick::Area(Area::Head), Pick::Area(Area::Legs)]);
    }
}
