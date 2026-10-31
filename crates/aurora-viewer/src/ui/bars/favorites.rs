//! The favourites of the islands, in the three ways the human can choose
//! (Preferences › Interface › "Favourites access"): the place island's
//! button and its menu (F1, the default), a row of pills under the islands
//! (F2) or the dock's Favorites button and its grid (F3). The three draw the
//! same list (`world/favorites.rs`: the Favorites folder in its order) and
//! act the same: a click teleports through the landmark, a drag changes the
//! order, a right click offers Firestorm's favourites menu
//! (menu_favorites.xml, LLFavoritesBarCtrl::doToSelected). Firestorm shows
//! its favourites as one bar of buttons under the navigation bar
//! (LLFavoritesBarCtrl); the menu and the grid are Aurora's.

use super::islands::{GAP, ICON, PAD, SLOT, area, slot_face, slot_ink};
use super::{BarAction, paint_icon};
use crate::settings::FavoritesAccess;
use crate::theme::Palette;
use crate::ui::icons::Icons;
use crate::ui::menu;
use crate::ui::places::drag::{self, PlaceDrag as Dragged};
use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Vec2};
use glam::DVec3;
use uuid::Uuid;

/// One favourite as the bars show it.
#[derive(Debug, Clone, PartialEq)]
pub struct Favorite {
    pub item: Uuid,
    pub asset: Uuid,
    pub name: String,
    /// The region, once the landmark and the map know it.
    pub region: String,
    /// "PG", "Mature", "Adult" or "" (unknown), as `bars::maturity_name`.
    pub rating: &'static str,
    pub global: Option<DVec3>,
    pub slurl: Option<String>,
    /// The item's thumbnail, else its parcel's snapshot, once loaded.
    pub thumbnail: Option<egui::TextureId>,
}

/// What the bars know of the favourites this frame.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub list: Vec<Favorite>,
    /// The favourite added from the bars a moment ago.
    pub added: Option<Uuid>,
    /// The parcel we stand on is already a favourite.
    pub here: bool,
    pub access: FavoritesAccess,
    /// The row (F2) folded to its tab.
    pub folded: bool,
    /// The Favorites folder is not read yet.
    pub loading: bool,
}

/// What a click in the favourites asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum FavAction {
    /// LLFavoritesBarCtrl::onButtonClick: teleport through the landmark.
    Teleport(Uuid),
    ShowOnMap(DVec3),
    /// "View / Edit Landmark".
    Edit(Uuid),
    /// The favourite dropped before the one at this rank.
    Move(Uuid, usize),
    /// "Add this place to favourites" (the star's own action).
    AddHere,
    /// Places › Favorites.
    OpenPlaces,
    Access(FavoritesAccess),
    Fold(bool),
}

impl From<FavAction> for BarAction {
    fn from(a: FavAction) -> Self {
        BarAction::Favorite(a)
    }
}

fn drag_source(response: &egui::Response, fav: &Favorite) {
    drag::source(response, || Dragged {
        target: drag::Target::Landmark(fav.item),
        favorite: Some(fav.item),
        name: fav.name.clone(),
        thumbnail: fav.thumbnail,
    });
}

fn shown_id() -> Id {
    Id::unique("bars_favorites_shown")
}

/// The favourites were on screen this frame: the app resolves their
/// regions and loads their thumbnails only then.
fn mark_shown(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(shown_id(), true));
}

/// Whether the favourites were on screen last frame (read once).
pub fn take_shown(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(shown_id())).unwrap_or(false)
}

/// The favourites whose name or region holds `filter`.
fn filtered<'a>(view: &'a View, filter: &str) -> Vec<(usize, &'a Favorite)> {
    let f = filter.trim().to_lowercase();
    view.list
        .iter()
        .enumerate()
        .filter(|(_, fav)| f.is_empty() || fav.name.to_lowercase().contains(&f) || fav.region.to_lowercase().contains(&f))
        .collect()
}

/// A favourite's picture: its thumbnail, or the landmark's pin on the
/// raised face while it has none.
fn thumb(ui: &egui::Ui, p: &Palette, icons: &Icons, rect: Rect, fav: &Favorite, radius: egui::CornerRadius) {
    let painter = ui.painter();
    match fav.thumbnail {
        Some(t) => {
            let uv = cover(rect.aspect_ratio(), 1.0);
            egui::Image::new((t, rect.size())).uv(uv).corner_radius(radius).paint_at(ui, rect);
        }
        None => {
            painter.rect_filled(rect, radius, p.raised);
            let size = (rect.height() * 0.5).min(18.0);
            paint_icon(ui, icons, "map-pin-filled", rect, size, p.muted_dim);
        }
    }
    // the thin light edge of the mockup's thumbnails
    painter.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, p.ink.gamma_multiply(0.10)),
        egui::StrokeKind::Inside,
    );
}

/// The part of a square-ish picture that fills a frame of `aspect`
/// (width / height), centred, without stretching.
fn cover(aspect: f32, image_aspect: f32) -> Rect {
    if aspect > image_aspect {
        let h = image_aspect / aspect;
        Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0))
    } else {
        let w = aspect / image_aspect;
        Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0))
    }
}

fn badge(ui: &egui::Ui, p: &Palette, rect: Rect, rating: &str) {
    if !rating.is_empty() {
        crate::ui::widgets::paint_maturity_badge(ui.painter(), p, rating, rect, crate::ui::widgets::inner_radius(ui.visuals()));
    }
}

fn semibold(ui: &egui::Ui, size: f32) -> FontId {
    FontId::new(size, crate::ui::fonts::semibold(ui.ctx()))
}

/// A text cut with "…" at `width`.
fn cut(ui: &egui::Ui, text: &str, font: FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// Firestorm's favourites menu (menu_favorites.xml) on a favourite.
fn fav_menu(resp: &egui::Response, p: &Palette, fav: &Favorite, out: &mut Option<FavAction>) {
    menu::context_menu(resp, p, |ui| {
        if menu::item(ui, p, "location-filled", aurora_i18n::tr!("common-teleport-3")) {
            *out = Some(FavAction::Teleport(fav.asset));
        }
        if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map"), fav.global.is_some())
            && let Some(g) = fav.global
        {
            *out = Some(FavAction::ShowOnMap(g));
        }
        if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("favorites-view-edit")) {
            *out = Some(FavAction::Edit(fav.item));
        }
        menu::todo(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-places-move-to-landmarks"));
        if menu::item_if(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl"), fav.slurl.is_some())
            && let Some(s) = &fav.slurl
        {
            ui.ctx().copy_text(s.clone());
        }
        menu::separator(ui, p);
        menu::todo(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename"));
        menu::todo(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"));
    });
}

/// The choice of where the favourites live, as a sub-menu (also the dock's
/// Favorites button's, `toolbar.rs`): the new place when one is picked.
pub(crate) fn access_choice(ui: &mut egui::Ui, p: &Palette, access: FavoritesAccess) -> Option<FavoritesAccess> {
    let mut out = None;
    menu::submenu(
        ui,
        p,
        "bookmarks-filled",
        aurora_i18n::tr!("ui-options-favorites-access"),
        true,
        |ui| {
            for (value, icon, label) in [
                (FavoritesAccess::Menu, "bookmarks-filled", aurora_i18n::tr!("favorites-access-menu")),
                (
                    FavoritesAccess::Row,
                    "layout-navbar-expand-filled",
                    aurora_i18n::tr!("favorites-access-row"),
                ),
                (
                    FavoritesAccess::Dock,
                    "layout-bottombar-filled",
                    aurora_i18n::tr!("favorites-access-dock"),
                ),
            ] {
                if menu::check(ui, p, icon, label, access == value, true) && access != value {
                    out = Some(value);
                }
            }
        },
    );
    out
}

fn access_menu(ui: &mut egui::Ui, p: &Palette, access: FavoritesAccess, out: &mut Option<FavAction>) {
    if let Some(a) = access_choice(ui, p, access) {
        *out = Some(FavAction::Access(a));
    }
}

/// The header of the menu and of the grid: the icon, "Favorites", their
/// count, and what a click does.
fn header(ui: &mut egui::Ui, p: &Palette, icons: &Icons, count: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let cy = rect.top() + 4.0 + 9.0;
    let icon = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + 8.0, cy), Vec2::splat(16.0));
    paint_icon(ui, icons, "bookmarks-filled", icon, 16.0, p.muted);
    let title = ui
        .painter()
        .layout_no_wrap(aurora_i18n::tr!("common-favorites").to_owned(), semibold(ui, 13.5), p.ink);
    let tw = title.size().x;
    crate::ui::widgets::paint_ink_left(ui.painter(), title, Pos2::new(icon.right() + 8.0, cy), p.ink);
    let n = ui
        .painter()
        .layout_no_wrap(count.to_string(), FontId::proportional(12.0), p.muted_dim);
    crate::ui::widgets::paint_ink_left(ui.painter(), n, Pos2::new(icon.right() + 8.0 + tw + 8.0, cy), p.muted_dim);
    let hint = ui.painter().layout_no_wrap(
        aurora_i18n::tr!("favorites-click-teleports").to_owned(),
        FontId::proportional(11.0),
        p.muted_dim,
    );
    let hw = hint.size().x;
    crate::ui::widgets::paint_ink_left(ui.painter(), hint, Pos2::new(rect.right() - 6.0 - hw, cy), p.muted_dim);
}

/// The empty state: no favourite yet, how to add one, and the button that
/// adds this place.
fn empty(ui: &mut egui::Ui, p: &Palette, icons: &Icons, here: bool, out: &mut Option<FavAction>) {
    ui.vertical_centered(|ui| {
        ui.add_space(14.0);
        let (big, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
        ui.painter()
            .rect_filled(big, ui.visuals().window_corner_radius, p.violet_light.gamma_multiply(0.12));
        paint_icon(ui, icons, "bookmarks-filled", big, 22.0, p.violet_light);
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("favorites-empty-title"))
                .font(semibold(ui, 13.5))
                .color(p.ink),
        );
        ui.add_space(8.0);
        // the mockup's 310 px column
        ui.scope(|ui| {
            ui.set_max_width(310.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(aurora_i18n::tr!("favorites-empty-text"))
                        .size(12.0)
                        .color(p.muted),
                )
                .wrap(),
            );
        });
        ui.add_space(8.0);
        let label = aurora_i18n::tr!("favorites-add-here");
        let size = Vec2::new(0.0, 24.0);
        let enabled = !here;
        let resp = ui.add_enabled_ui(enabled, |ui| {
            crate::ui::widgets::action_button(ui, p, "star-filled", label, crate::ui::widgets::ActionKind::Primary, size)
        });
        if resp.inner.clicked() {
            *out = Some(FavAction::AddHere);
            ui.close();
        }
        ui.add_space(10.0);
    });
}

/// Row height and the gap between two rows of the menu (the mockup's
/// `.fr`, `.favlist`).
const ROW_H: f32 = 40.0;
const ROW_GAP: f32 = 1.0;
const MENU_ROWS: usize = 8;
const THUMB: Vec2 = Vec2::new(40.0, 28.0);

/// The rank a favourite dragged over `rects` (in order) would take, from
/// the pointer's coordinate along the list (`y`, or `x` for the row).
pub fn drop_rank(rects: &[Rect], at: f32, vertical: bool) -> usize {
    let mid = |r: &Rect| if vertical { r.center().y } else { r.center().x };
    rects.iter().position(|r| at < mid(r)).unwrap_or(rects.len())
}

/// The drop marker before the rank `rank` of `rects`.
fn paint_drop(ui: &egui::Ui, p: &Palette, rects: &[Rect], rank: usize, vertical: bool) {
    let Some(first) = rects.first() else { return };
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("bars_favorites_drop")));
    if vertical {
        let y = rects
            .get(rank)
            .map_or_else(|| rects[rects.len() - 1].bottom() + 1.0, |r| r.top() - 1.0);
        painter.line_segment(
            [Pos2::new(first.left() + 4.0, y), Pos2::new(first.right() - 4.0, y)],
            egui::Stroke::new(2.0, p.violet_light),
        );
    } else {
        let x = rects
            .get(rank)
            .map_or_else(|| rects[rects.len() - 1].right() + 1.0, |r| r.left() - 1.0);
        let r = rects[rank.min(rects.len() - 1)];
        painter.line_segment(
            [Pos2::new(x, r.center().y - 11.0), Pos2::new(x, r.center().y + 11.0)],
            egui::Stroke::new(2.0, p.violet_light),
        );
    }
}

/// While a favourite is dragged over `zone`: the marker, and its new rank
/// once dropped.
fn drop_on(ui: &egui::Ui, p: &Palette, zone: Rect, rects: &[(usize, Rect)], vertical: bool) -> Option<FavAction> {
    let dragged = egui::DragAndDrop::payload::<Dragged>(ui.ctx())?.favorite?;
    let pointer = ui.ctx().pointer_interact_pos()?;
    if !ui.rect_contains_pointer(zone) {
        return None;
    }
    let only: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
    let local = drop_rank(&only, if vertical { pointer.y } else { pointer.x }, vertical);
    paint_drop(ui, p, &only, local, vertical);
    // the rank in the whole list (a filter may hide some)
    let rank = rects.get(local).map_or_else(|| rects.last().map_or(0, |(i, _)| i + 1), |(i, _)| *i);
    if ui.input(|i| i.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        return Some(FavAction::Move(dragged, rank));
    }
    None
}

/// One row of the menu: the thumbnail, the name and the region, the
/// rating; under the pointer, the grip to drag it, Map and Edit. The one
/// added a moment ago is marked.
#[allow(clippy::too_many_arguments)]
fn menu_row(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    fav: &Favorite,
    added: bool,
    dragging: bool,
    out: &mut Option<FavAction>,
) -> Rect {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click_and_drag());
    let hot = !dragging && ui.rect_contains_pointer(rect);
    let radius = ui.visuals().window_corner_radius;
    if hot {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.35));
    } else if added {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.16));
        let edge = Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height()));
        ui.painter().rect_filled(edge, egui::CornerRadius::ZERO, p.violet_light);
    }
    if egui::DragAndDrop::payload::<Dragged>(ui.ctx()).is_some_and(|d| d.target.item() == Some(fav.item)) {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let cy = rect.center().y;
    let mut x = rect.left() + 6.0;
    if hot {
        let grip = Rect::from_center_size(Pos2::new(x - 4.0 + 7.0, cy), Vec2::splat(14.0));
        paint_icon(ui, icons, "grip-vertical", grip, 14.0, p.muted);
        x = grip.right() + 10.0;
    }
    let t = Rect::from_min_size(Pos2::new(x, cy - THUMB.y / 2.0), THUMB);
    thumb(ui, p, icons, t, fav, radius);
    x = t.right() + 10.0;
    let right_end = rect.right() - 8.0;
    let badge_r = Rect::from_min_size(Pos2::new(right_end - 15.0, cy - 7.5), Vec2::splat(15.0));
    badge(ui, p, badge_r, fav.rating);
    let mut text_right = badge_r.left() - 10.0;
    if hot {
        // Map and Edit, 24 px slots before the rating
        let edit = Rect::from_min_size(Pos2::new(text_right - 24.0, cy - 12.0), Vec2::splat(24.0));
        let map = edit.translate(Vec2::new(-26.0, 0.0));
        text_right = map.left() - 6.0;
        for (slot, icon, tip, act) in [
            (
                map,
                "map",
                aurora_i18n::tr!("common-show-on-map"),
                fav.global.map(FavAction::ShowOnMap),
            ),
            (
                edit,
                "pencil-filled",
                aurora_i18n::tr!("favorites-view-edit"),
                Some(FavAction::Edit(fav.item)),
            ),
        ] {
            let r = ui.interact(slot, Id::unique(("bars_favorites_row_act", fav.item, icon)), Sense::click());
            slot_face(ui, p, slot, r.hovered(), false);
            paint_icon(ui, icons, icon, slot, 15.0, if act.is_some() { p.ink } else { p.muted_dim });
            if r.on_hover_text(tip).clicked()
                && let Some(a) = act
            {
                *out = Some(a);
                ui.close();
            }
        }
    }
    let width = text_right - x;
    let name = cut(ui, &fav.name, semibold(ui, 12.5), p.ink, width);
    let name_h = name.size().y;
    ui.painter().galley(Pos2::new(x, cy - name_h + 1.0), name, p.ink);
    let mut sub = egui::text::LayoutJob::default();
    sub.append(&fav.region, 0.0, egui::TextFormat::simple(FontId::proportional(11.5), p.muted));
    if added {
        if !fav.region.is_empty() {
            sub.append(" · ", 0.0, egui::TextFormat::simple(FontId::proportional(11.5), p.muted));
        }
        sub.append(
            aurora_i18n::tr!("favorites-just-added"),
            0.0,
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 10.5, p.violet_light),
        );
    }
    sub.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    let sub = ui.painter().layout_job(sub);
    ui.painter().galley(Pos2::new(x, cy + 1.0), sub, p.muted);

    if resp.drag_started() {
        drag_source(&resp, fav);
    }
    fav_menu(&resp, p, fav, out);
    let tip = if fav.region.is_empty() {
        fav.name.clone()
    } else {
        format!("{} · {}", fav.name, fav.region)
    };
    if resp.on_hover_text(tip).clicked() {
        *out = Some(FavAction::Teleport(fav.asset));
        ui.close();
    }
    rect
}

/// The favourites menu (F1, and the row's "more"): header, filter, the
/// list (eight rows, then it scrolls), "Add this place", Places and the
/// choice of where the favourites live.
pub fn menu_body(ui: &mut egui::Ui, p: &Palette, icons: &Icons, view: &View, out: &mut Option<FavAction>) {
    mark_shown(ui.ctx());
    if view.list.is_empty() && !view.loading {
        ui.set_width(380.0 - 12.0);
        empty(ui, p, icons, view.here, out);
    } else {
        ui.set_width(390.0 - 12.0);
        header(ui, p, icons, view.list.len());
        let filter_id = Id::unique("bars_favorites_filter");
        let mut filter = ui.ctx().data(|d| d.get_temp::<String>(filter_id)).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            let w = ui.available_width() - 2.0;
            crate::ui::widgets::pills::search_box(ui, p, &mut filter, aurora_i18n::tr!("favorites-filter"), w);
        });
        ui.add_space(6.0);
        let rows = filtered(view, &filter);
        let height = (rows.len().clamp(1, MENU_ROWS)) as f32 * (ROW_H + ROW_GAP);
        let dragging = egui::DragAndDrop::has_payload_of_type::<Dragged>(ui.ctx());
        let mut rects = Vec::with_capacity(rows.len());
        let shown = egui::ScrollArea::vertical()
            .id_salt("bars_favorites_list")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = ROW_GAP;
                if rows.is_empty() {
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("favorites-no-match"))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
                for (i, fav) in &rows {
                    let r = menu_row(ui, p, icons, fav, view.added == Some(fav.item), dragging, out);
                    rects.push((*i, r));
                }
                demo_target(ui.ctx(), "row", &rects);
            });
        // the drop is only meaningful on the whole list
        if filter.trim().is_empty()
            && let Some(a) = drop_on(ui, p, shown.inner_rect, &rects, true)
        {
            *out = Some(a);
        }
        ui.ctx().data_mut(|d| d.insert_temp(filter_id, filter));
    }
    menu::separator(ui, p);
    if !view.list.is_empty() || view.loading {
        let add = aurora_i18n::tr!("favorites-add-here");
        if view.here {
            ui.add_enabled_ui(false, |ui| {
                menu::item_detail(ui, p, "star-filled", add, aurora_i18n::tr!("favorites-already-there"))
            });
        } else if menu::item(ui, p, "star-filled", add) {
            *out = Some(FavAction::AddHere);
        }
    }
    if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-bars-open-favorites")) {
        *out = Some(FavAction::OpenPlaces);
    }
    access_menu(ui, p, view.access, out);
}

/// The Favorites button of the place island (F1): the icon alone, at every
/// width, and its menu.
pub fn place_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, rect: Rect, view: &View) -> Option<BarAction> {
    let resp = ui.interact(rect, Id::unique("bars_place_favorites"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = resp.hovered() || open;
    slot_face(ui, p, rect, hot, false);
    let ink = slot_ink(p, hot, true);
    let icon = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + ICON / 2.0, rect.center().y), Vec2::splat(ICON));
    paint_icon(ui, icons, "bookmarks-filled", icon, ICON, ink);
    let caret = Rect::from_center_size(Pos2::new(icon.right() + 6.0 + CARET / 2.0, rect.center().y), Vec2::splat(CARET));
    paint_icon(ui, icons, "chevron-down", caret, CARET, ink);
    super::islands::demo_open(ui.ctx(), &resp, super::demo::Open::Favorites);
    let mut out = None;
    menu::dropdown_placed(
        &resp,
        p,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        Some((egui::RectAlign::BOTTOM_START, 10.0)),
        |ui| menu_body(ui, p, icons, view, &mut out),
    );
    resp.on_hover_text(aurora_i18n::tr!("common-favorites"));
    out.map(BarAction::from)
}

/// The width of the place island's Favorites button: the icon, then its
/// caret.
pub const PLACE_BUTTON_W: f32 = 6.0 + ICON + 6.0 + CARET + 6.0;
const CARET: f32 = 12.0;

/// Height of the row of favourites (F2) and its gap under the islands.
pub const ROW_HEIGHT: f32 = 34.0;
const PILL_H: f32 = 26.0;
const PILL_THUMB: f32 = 16.0;

fn pill_width(ui: &egui::Ui, fav: &Favorite) -> f32 {
    let w = ui
        .painter()
        .layout_no_wrap(fav.name.clone(), FontId::proportional(12.0), Color32::PLACEHOLDER)
        .size()
        .x;
    5.0 + PILL_THUMB + 6.0 + w.ceil() + 9.0
}

/// The row of favourites under the place island (F2): one pill per
/// favourite while they fit, "n more" (the menu with the rest), and the
/// button that folds it; folded, a tab that unfolds it. Returns its
/// rectangle.
pub fn row(ctx: &egui::Context, p: &Palette, icons: &Icons, view: &View, under: Rect) -> (Rect, Option<BarAction>) {
    let mut out = None;
    let top = under.bottom() + super::islands::MARGIN;
    if view.folded {
        // a tab under the place island's left end
        let rect = Rect::from_min_size(Pos2::new(under.left(), top), Vec2::new(PAD * 2.0 + SLOT + 14.0, ROW_HEIGHT));
        area(ctx, p, "favorites_tab", rect, |ui| {
            let slot = rect.shrink2(Vec2::new(PAD, (ROW_HEIGHT - PILL_H) / 2.0));
            let resp = ui.interact(slot, Id::unique("bars_favorites_unfold"), Sense::click());
            slot_face(ui, p, slot, resp.hovered(), false);
            let ink = slot_ink(p, resp.hovered(), true);
            let icon = Rect::from_center_size(Pos2::new(slot.left() + 6.0 + ICON / 2.0, slot.center().y), Vec2::splat(ICON));
            paint_icon(ui, icons, "bookmarks-filled", icon, ICON, ink);
            let caret = Rect::from_center_size(Pos2::new(icon.right() + 4.0 + 6.0, slot.center().y), Vec2::splat(CARET));
            paint_icon(ui, icons, "chevron-down", caret, CARET, ink);
            row_menu(&resp, p, view, &mut out);
            if resp.on_hover_text(aurora_i18n::tr!("favorites-row-unfold")).clicked() {
                out = Some(FavAction::Fold(false));
            }
        });
        return (rect, out.map(BarAction::from));
    }
    let rect = Rect::from_min_max(Pos2::new(under.left() - 1.0, top), Pos2::new(under.right() + 1.0, top + ROW_HEIGHT));
    area(ctx, p, "favorites_row", rect, |ui| {
        mark_shown(ui.ctx());
        let back = ui.interact(rect, Id::unique("bars_favorites_row_back"), Sense::click());
        row_menu(&back, p, view, &mut out);
        let cy = rect.center().y;
        let fold = Rect::from_min_size(Pos2::new(rect.right() - PAD - SLOT, cy - PILL_H / 2.0), Vec2::new(SLOT, PILL_H));
        let r = ui.interact(fold, Id::unique("bars_favorites_fold"), Sense::click());
        slot_face(ui, p, fold, r.hovered(), false);
        paint_icon(
            ui,
            icons,
            "layout-navbar-collapse-filled",
            fold,
            ICON,
            slot_ink(p, r.hovered(), true),
        );
        if r.on_hover_text(aurora_i18n::tr!("favorites-row-fold")).clicked() {
            out = Some(FavAction::Fold(true));
        }
        let mut x = rect.left() + PAD;
        if view.list.is_empty() {
            let hint = ui.painter().layout_no_wrap(
                aurora_i18n::tr!("favorites-row-empty").to_owned(),
                FontId::proportional(12.0),
                p.muted_dim,
            );
            let icon = Rect::from_center_size(Pos2::new(x + 6.0 + 7.5, cy), Vec2::splat(15.0));
            paint_icon(ui, icons, "bookmarks-filled", icon, 15.0, p.muted_dim);
            crate::ui::widgets::paint_ink_left(ui.painter(), hint, Pos2::new(icon.right() + 6.0, cy), p.muted_dim);
            return;
        }
        // as many pills as fit before the "more" slot and the fold button
        let measure = ui.painter().clone();
        let more_w = |n: usize| {
            let t = aurora_i18n::tr!("favorites-more", n = n as i64);
            6.0 + ICON
                + 6.0
                + measure
                    .layout_no_wrap(t.to_string(), FontId::proportional(12.0), Color32::PLACEHOLDER)
                    .size()
                    .x
                + 6.0
        };
        let end = fold.left() - GAP;
        let widths: Vec<f32> = view.list.iter().map(|f| pill_width(ui, f)).collect();
        let total: f32 = widths.iter().sum::<f32>() + GAP * widths.len().saturating_sub(1) as f32;
        let fits = if x + total <= end {
            view.list.len()
        } else {
            let mut n = 0;
            let mut used = 0.0;
            while n < widths.len() {
                let next = used + widths[n] + if n > 0 { GAP } else { 0.0 };
                if x + next + GAP + more_w(view.list.len() - n - 1) > end {
                    break;
                }
                used = next;
                n += 1;
            }
            n
        };
        let dragging = egui::DragAndDrop::has_payload_of_type::<Dragged>(ui.ctx());
        let mut rects = Vec::with_capacity(fits);
        for (i, fav) in view.list.iter().take(fits).enumerate() {
            let pill = Rect::from_min_size(Pos2::new(x, cy - PILL_H / 2.0), Vec2::new(widths[i], PILL_H));
            x = pill.right() + GAP;
            rects.push((i, pill));
            let resp = ui.interact(pill, Id::unique(("bars_favorites_pill", fav.item)), Sense::click_and_drag());
            let moving = egui::DragAndDrop::payload::<Dragged>(ui.ctx()).is_some_and(|d| d.target.item() == Some(fav.item));
            let hot = !dragging && resp.hovered();
            slot_face(ui, p, pill, hot, false);
            let alpha = if moving { 0.35 } else { 1.0 };
            let t = Rect::from_min_size(Pos2::new(pill.left() + 5.0, cy - PILL_THUMB / 2.0), Vec2::splat(PILL_THUMB));
            let mut faded = ui.new_child(egui::UiBuilder::new().max_rect(pill));
            faded.multiply_opacity(alpha);
            thumb(&faded, p, icons, t, fav, crate::ui::widgets::inner_radius(ui.visuals()));
            let name = faded.painter().layout_no_wrap(fav.name.clone(), FontId::proportional(12.0), p.ink);
            crate::ui::widgets::paint_ink_left(faded.painter(), name, Pos2::new(t.right() + 6.0, cy), p.ink);
            if resp.drag_started() {
                drag_source(&resp, fav);
            }
            fav_menu(&resp, p, fav, &mut out);
            let rating = match fav.rating {
                "PG" => aurora_i18n::tr!("common-general"),
                "Mature" => aurora_i18n::tr!("common-moderate"),
                "Adult" => aurora_i18n::tr!("common-adult"),
                _ => "",
            };
            let facts: Vec<&str> = [fav.region.as_str(), rating].into_iter().filter(|s| !s.is_empty()).collect();
            let resp = resp.on_hover_ui(|ui| {
                ui.label(egui::RichText::new(&fav.name).font(semibold(ui, 12.0)).color(p.ink));
                if !facts.is_empty() {
                    ui.label(egui::RichText::new(facts.join(" · ")).size(12.0).color(p.ink));
                }
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("favorites-pill-hints"))
                        .size(11.5)
                        .color(p.muted),
                );
            });
            if resp.clicked() {
                out = Some(FavAction::Teleport(fav.asset));
            }
        }
        if fits < view.list.len() {
            let n = view.list.len() - fits;
            let more = Rect::from_min_size(Pos2::new(x, cy - PILL_H / 2.0), Vec2::new(more_w(n), PILL_H));
            let resp = ui.interact(more, Id::unique("bars_favorites_more"), Sense::click());
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
            slot_face(ui, p, more, resp.hovered() || open, false);
            let ink = slot_ink(p, resp.hovered() || open, true);
            let icon = Rect::from_center_size(Pos2::new(more.left() + 6.0 + ICON / 2.0, cy), Vec2::splat(ICON));
            paint_icon(ui, icons, "chevrons-right", icon, ICON, ink);
            let t = ui.painter().layout_no_wrap(
                aurora_i18n::tr!("favorites-more", n = n as i64).to_string(),
                FontId::proportional(12.0),
                ink,
            );
            crate::ui::widgets::paint_ink_left(ui.painter(), t, Pos2::new(icon.right() + 6.0, cy), ink);
            menu::dropdown_placed(
                &resp,
                p,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                Some((egui::RectAlign::BOTTOM_START, 10.0)),
                |ui| menu_body(ui, p, icons, view, &mut out),
            );
        }
        let zone = Rect::from_min_max(rect.min, Pos2::new(end, rect.bottom()));
        if let Some(a) = drop_on(ui, p, zone, &rects, false) {
            out = Some(a);
        }
    });
    (rect, out.map(BarAction::from))
}

/// The right-click menu of the row and of its tab.
fn row_menu(resp: &egui::Response, p: &Palette, view: &View, out: &mut Option<FavAction>) {
    menu::context_menu(resp, p, |ui| {
        let (icon, label, fold) = if view.folded {
            ("layout-navbar-expand-filled", aurora_i18n::tr!("favorites-row-unfold"), false)
        } else {
            ("layout-navbar-collapse-filled", aurora_i18n::tr!("favorites-row-fold"), true)
        };
        if menu::item(ui, p, icon, label) {
            *out = Some(FavAction::Fold(fold));
        }
        if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-bars-open-favorites")) {
            *out = Some(FavAction::OpenPlaces);
        }
        access_menu(ui, p, view.access, out);
    });
}

/// The grid of the dock's Favorites button (F3): four tiles a line, the
/// thumbnail with its rating, the name and the region; opened upwards.
pub fn grid_body(ui: &mut egui::Ui, p: &Palette, icons: &Icons, view: &View, out: &mut Option<FavAction>) {
    mark_shown(ui.ctx());
    const TILE: Vec2 = Vec2::new(124.0, 104.0);
    const COLS: usize = 4;
    if view.list.is_empty() && !view.loading {
        ui.set_width(380.0 - 12.0);
        empty(ui, p, icons, view.here, out);
    } else {
        ui.set_width(530.0 - 12.0);
        header(ui, p, icons, view.list.len());
        let lines = view.list.len().div_ceil(COLS).max(1);
        let height = lines.min(4) as f32 * (TILE.y + 4.0);
        let dragging = egui::DragAndDrop::has_payload_of_type::<Dragged>(ui.ctx());
        let mut rects = Vec::with_capacity(view.list.len());
        let shown = egui::ScrollArea::vertical()
            .id_salt("bars_favorites_grid")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let left = ui.cursor().left();
                let top = ui.cursor().top();
                let (area_rect, _) = ui.allocate_exact_size(
                    Vec2::new(COLS as f32 * (TILE.x + 4.0) - 4.0, lines as f32 * (TILE.y + 4.0) - 4.0),
                    Sense::hover(),
                );
                let _ = area_rect;
                for (i, fav) in view.list.iter().enumerate() {
                    let (col, line) = (i % COLS, i / COLS);
                    let tile = Rect::from_min_size(
                        Pos2::new(left + col as f32 * (TILE.x + 4.0), top + line as f32 * (TILE.y + 4.0)),
                        TILE,
                    );
                    rects.push((i, tile));
                    let resp = ui.interact(tile, Id::unique(("bars_favorites_tile", fav.item)), Sense::click_and_drag());
                    let hot = !dragging && resp.hovered();
                    let radius = ui.visuals().window_corner_radius;
                    if hot {
                        ui.painter().rect_filled(tile, radius, p.raised);
                    }
                    let picture = Rect::from_min_size(tile.min + Vec2::splat(4.0), Vec2::new(TILE.x - 8.0, 62.0));
                    thumb(ui, p, icons, picture, fav, radius);
                    badge(
                        ui,
                        p,
                        Rect::from_min_size(Pos2::new(picture.right() - 4.0 - 15.0, picture.top() + 4.0), Vec2::splat(15.0)),
                        fav.rating,
                    );
                    let name = cut(ui, &fav.name, semibold(ui, 12.0), p.ink, picture.width());
                    let nh = name.size().y;
                    ui.painter().galley(Pos2::new(picture.left(), picture.bottom() + 5.0), name, p.ink);
                    let region = cut(ui, &fav.region, FontId::proportional(11.0), p.muted, picture.width());
                    ui.painter()
                        .galley(Pos2::new(picture.left(), picture.bottom() + 5.0 + nh), region, p.muted);
                    if resp.drag_started() {
                        drag_source(&resp, fav);
                    }
                    fav_menu(&resp, p, fav, out);
                    if resp.on_hover_text(&fav.name).clicked() {
                        *out = Some(FavAction::Teleport(fav.asset));
                        ui.close();
                    }
                }
            });
        demo_target(ui.ctx(), "tile", &rects);
        if let Some(a) = drop_on(ui, p, shown.inner_rect, &rects, false) {
            *out = Some(a);
        }
    }
    menu::separator(ui, p);
    if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-bars-open-favorites")) {
        *out = Some(FavAction::OpenPlaces);
    }
    access_menu(ui, p, view.access, out);
}

/// A demo thumbnail: the mockup's 160° gradient through three colours
/// (`AURORA_DEMO_BARS`'s favourites).
pub fn demo_thumbnail(colors: [[u8; 3]; 3]) -> egui::ColorImage {
    const W: usize = 80;
    const H: usize = 56;
    // CSS's 160deg: towards the bottom, a little to the right
    let (dx, dy) = (20f32.to_radians().sin(), 20f32.to_radians().cos());
    let span = W as f32 * dx.abs() + H as f32 * dy;
    let mix = |a: [u8; 3], b: [u8; 3], t: f32| {
        let c = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
        Color32::from_rgb(c(0), c(1), c(2))
    };
    let mut pixels = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let t = ((x as f32 * dx + y as f32 * dy) / span).clamp(0.0, 1.0);
            pixels.push(if t < 0.55 {
                mix(colors[0], colors[1], t / 0.55)
            } else {
                mix(colors[1], colors[2], (t - 0.55) / 0.45)
            });
        }
    }
    egui::ColorImage::new([W, H], pixels)
}

/// The rows and tiles the demo points at: the mockup's hovered ones (the
/// second row of the menu, the sixth tile of the grid).
fn demo_target(ctx: &egui::Context, kind: &'static str, rects: &[(usize, Rect)]) {
    if !super::demo::get().on {
        return;
    }
    let k = if kind == "row" { 1 } else { 5 };
    if let Some((_, r)) = rects.iter().find(|(i, _)| *i == k) {
        ctx.data_mut(|d| d.insert_temp(Id::unique(("bars_favorites_demo_target", kind)), *r));
    }
}

/// `AURORA_DEMO_BARS=rowdrag`: the second pill of the row picked up at
/// frame 650 and held after the fourth (the mockup's reorder state).
pub fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let demo = super::demo::get();
    // the mockup's hovered row of the menu or tile of the grid
    if demo.open == Some(super::demo::Open::Favorites)
        && demo.favorites == Some(super::demo::Favorites::Twelve)
        && (700..=800).contains(&frame)
    {
        let kind = if demo.access == Some(FavoritesAccess::Dock) {
            "tile"
        } else {
            "row"
        };
        return ctx
            .data(|d| d.get_temp::<Rect>(Id::unique(("bars_favorites_demo_target", kind))))
            .map(|r| vec![egui::Event::PointerMoved(r.center())])
            .unwrap_or_default();
    }
    if !demo.row_drag || !(650..=700).contains(&frame) {
        return Vec::new();
    }
    let pill = |k: usize| {
        ctx.read_response(Id::unique(("bars_favorites_pill", crate::demo::place::bar_favorite_item(k))))
            .map(|r| r.rect)
    };
    let (Some(from), Some(to)) = (pill(1), pill(4)) else {
        return Vec::new();
    };
    let start = from.center();
    let target = Pos2::new(to.left() + 4.0, to.center().y);
    match frame {
        650 => vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
        651 => vec![egui::Event::PointerMoved(start + Vec2::new(12.0, 0.0))],
        _ => vec![egui::Event::PointerMoved(target)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_lands_before_the_first_item_past_the_pointer() {
        let rects: Vec<Rect> = (0..3)
            .map(|i| Rect::from_min_size(Pos2::new(0.0, i as f32 * 41.0), Vec2::new(100.0, 40.0)))
            .collect();
        assert_eq!(drop_rank(&rects, 5.0, true), 0);
        assert_eq!(drop_rank(&rects, 25.0, true), 1);
        assert_eq!(drop_rank(&rects, 70.0, true), 2);
        assert_eq!(drop_rank(&rects, 200.0, true), 3);
    }

    #[test]
    fn a_thumbnail_covers_its_frame_without_stretching() {
        let wide = cover(40.0 / 28.0, 1.0);
        assert!((wide.width() - 1.0).abs() < 1e-6 && (wide.height() - 0.7).abs() < 1e-6);
        let tall = cover(0.5, 1.0);
        assert!((tall.height() - 1.0).abs() < 1e-6 && (tall.width() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_filter_matches_names_and_regions() {
        let fav = |name: &str, region: &str| Favorite {
            item: Uuid::new_v4(),
            asset: Uuid::nil(),
            name: name.into(),
            region: region.into(),
            rating: "",
            global: None,
            slurl: None,
            thumbnail: None,
        };
        let view = View {
            list: vec![fav("Plage des Lucioles", "Aurora Démo"), fav("Club Néon", "Pulsar")],
            ..View::default()
        };
        assert_eq!(filtered(&view, "").len(), 2);
        assert_eq!(filtered(&view, "pulsar")[0].0, 1);
        assert_eq!(filtered(&view, " PLAGE ")[0].0, 0);
        assert!(filtered(&view, "zzz").is_empty());
    }
}
