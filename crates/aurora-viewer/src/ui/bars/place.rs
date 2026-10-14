//! The place island of the Aurora bars: back, forward, home, favourites,
//! then the place (rating, parcel, region and position, the parcel's
//! rights) and the landmark star. A click on the place opens its menu
//! (SLURL, destination, rights, About Land, history, home, Places, recent
//! places), replacing the location field Firestorm edits in place
//! (LLLocationInputCtrl, LLNavigationBar). The island stretches to the
//! account island, or takes the width of its content
//! (`Settings::place_bar_stretch`); what it shows follows the room it has
//! (`choose`).

use super::islands::{GAP, ICON, PAD, SLOT, demo_open, icon_slot, slot_face, slot_ink};
use super::{BarAction, StarAction, StatusInfo, paint_icon};
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::icons::Icons;
use crate::ui::menu;
use crate::ui::parcel_icons::{GLYPH, ParcelIcon, SLOT_HEIGHT, SLOT_WIDTH};
use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Vec2};

/// The Favorites button: the icon, then its caret.
const FAV_W: f32 = 6.0 + ICON + 6.0 + CARET + 6.0;
const CARET: f32 = 12.0;
/// Inside the place: room at its left and right ends, and between its parts.
const LOC_LEFT: f32 = 6.0;
const LOC_RIGHT: f32 = 8.0;
const LOC_GAP: f32 = 8.0;
/// The rating badge.
const BADGE: f32 = 15.0;
/// Between two rights; after the name of a right.
const RIGHT_GAP: f32 = 2.0;
const NAME_AFTER: f32 = 6.0;
const NAME_SIZE: f32 = 11.5;

/// How much the place island says, from the most to the least: every right
/// named, the forbidden ones named, the position, the parcel and the region
/// only. Each one drops what the previous one had when the room lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    AllNamed,
    ForbiddenNamed,
    Position,
    Bare,
}

/// The widths the choice needs, measured once per frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Widths {
    /// What every level shows: the buttons, the place's frame, the rating
    /// badge, the rights' icons.
    pub fixed: f32,
    pub parcel: f32,
    pub region: f32,
    /// The region with its position.
    pub region_position: f32,
    /// What the names add: the forbidden rights' alone, every right's.
    pub forbidden_names: f32,
    pub all_names: f32,
}

impl Widths {
    pub fn natural(&self, d: Detail) -> f32 {
        let region = if d == Detail::Bare { self.region } else { self.region_position };
        let names = match d {
            Detail::AllNamed => self.all_names,
            Detail::ForbiddenNamed => self.forbidden_names,
            _ => 0.0,
        };
        self.fixed + self.parcel + region + names
    }
}

/// The richest level that fits in `room`; with `names` off, the rights stay
/// icons whatever the room. The bare level when nothing fits (the parcel's
/// name is then cut).
pub fn choose(w: &Widths, room: f32, names: bool) -> Detail {
    let levels: &[Detail] = if names {
        &[Detail::AllNamed, Detail::ForbiddenNamed, Detail::Position, Detail::Bare]
    } else {
        &[Detail::Position, Detail::Bare]
    };
    levels.iter().copied().find(|d| w.natural(*d) <= room).unwrap_or(Detail::Bare)
}

/// Whether a right is named at `detail`.
fn named(it: &ParcelIcon, detail: Detail) -> bool {
    match detail {
        Detail::AllNamed => true,
        Detail::ForbiddenNamed => !it.ok,
        _ => false,
    }
}

fn name_font() -> FontId {
    FontId::proportional(NAME_SIZE)
}

/// The text after a right's icon: its name, the health when damage is on.
fn right_text(it: &ParcelIcon, detail: Detail) -> Option<String> {
    match (named(it, detail), &it.label) {
        (true, Some(l)) => Some(format!("{} {l}", it.name)),
        (true, None) => Some(it.name.to_owned()),
        (false, Some(l)) => Some(l.clone()),
        (false, None) => None,
    }
}

fn right_width(ui: &egui::Ui, it: &ParcelIcon, detail: Detail) -> f32 {
    SLOT_WIDTH
        + right_text(it, detail).map_or(0.0, |t| {
            RIGHT_GAP + ui.painter().layout_no_wrap(t, name_font(), Color32::PLACEHOLDER).size().x.ceil() + NAME_AFTER
        })
}

fn rights_width(ui: &egui::Ui, items: &[ParcelIcon], detail: Detail) -> f32 {
    let n = items.len() as f32;
    items.iter().map(|it| right_width(ui, it, detail)).sum::<f32>() + RIGHT_GAP * (n - 1.0).max(0.0)
}

fn parcel_font(ctx: &egui::Context) -> FontId {
    FontId::new(13.0, crate::ui::fonts::semibold(ctx))
}

/// The parcel's name (the region's when the parcel has none), the region,
/// the position.
fn texts(st: &StatusInfo) -> (String, String, String) {
    let position = format!("({:.0}, {:.0}, {:.0})", st.position.x, st.position.y, st.position.z);
    if st.parcel.is_empty() {
        (st.region.to_owned(), String::new(), position)
    } else {
        (st.parcel.to_owned(), st.region.to_owned(), position)
    }
}

/// The widths of the levels for this frame.
fn measure(ui: &egui::Ui, st: &StatusInfo) -> Widths {
    let (parcel, region, position) = texts(st);
    let width = |s: String, font: FontId| ui.painter().layout_no_wrap(s, font, Color32::PLACEHOLDER).size().x.ceil();
    // the position keeps the width of its widest digits: walking does not
    // make the island breathe
    let steady: String = position.chars().map(|c| if c.is_ascii_digit() { '0' } else { c }).collect();
    let region_w = width(region.clone(), FontId::proportional(12.0));
    let with_position = if region.is_empty() {
        width(steady, FontId::proportional(12.0))
    } else {
        width(format!("{region} {steady}"), FontId::proportional(12.0))
    };
    let buttons = PAD + SLOT * 3.0 + FAV_W + SLOT + GAP * 5.0 + PAD;
    let place = LOC_LEFT + BADGE + LOC_GAP + LOC_GAP + LOC_GAP * 2.0 + LOC_RIGHT;
    let icons = rights_width(ui, &st.parcel_icons, Detail::Bare);
    let rights_gap = if st.parcel_icons.is_empty() { 0.0 } else { LOC_GAP };
    Widths {
        fixed: buttons + place + icons + rights_gap,
        parcel: width(parcel, parcel_font(ui.ctx())),
        region: region_w,
        region_position: with_position,
        forbidden_names: rights_width(ui, &st.parcel_icons, Detail::ForbiddenNamed) - icons,
        all_names: rights_width(ui, &st.parcel_icons, Detail::AllNamed) - icons,
    }
}

/// The place island's rectangle in `span` (the room between the menu and
/// the account islands) and what it shows there.
pub fn layout(ui: &egui::Ui, st: &StatusInfo, span: Rect) -> (Rect, Detail) {
    let widths = measure(ui, st);
    let detail = choose(&widths, span.width(), st.islands.rights_text);
    let natural = widths.natural(detail);
    let width = if st.islands.stretch {
        span.width()
    } else {
        natural.min(span.width())
    };
    (Rect::from_min_size(span.min, Vec2::new(width.max(0.0), span.height())), detail)
}

/// The place island at `rect`. Returns the action of a click.
pub fn island(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    st: &StatusInfo,
    rect: Rect,
    detail: Detail,
) -> Option<BarAction> {
    let mut action = None;
    let top = rect.center().y - SLOT / 2.0;

    // a right click anywhere on the island: its two settings
    let back = ui.interact(rect, Id::new("bars_place_island"), Sense::click());
    place_options(&back, p, st, &mut action);
    // the demo cannot right-click: the same menu where the pointer would be
    if super::demo::get().open == Some(super::demo::Open::PlaceOptions) && ui.ctx().cumulative_frame_nr() >= 300 {
        let id = Id::new("bars_place_demo_options");
        let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
        let at = Pos2::new(rect.left() + rect.width() * 0.3, rect.center().y);
        menu::popup_at(ui.ctx(), id, at, &mut open, p, |ui| options_body(ui, p, st, &mut action));
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
    }

    let mut x = rect.left() + PAD;
    let mut next = |w: f32| {
        let r = Rect::from_min_size(Pos2::new(x, top), Vec2::new(w, SLOT));
        x += w + GAP;
        r
    };
    // back / forward through the teleport history, home
    for (left, dest) in [(true, st.tp_back.as_deref()), (false, st.tp_forward.as_deref())] {
        let id = Id::new(("bars_place_arrow", left));
        let icon = if left { "arrow-left" } else { "arrow-right" };
        let resp = icon_slot(ui, p, icons, next(SLOT), id, icon, dest.is_some());
        let tip = match dest {
            Some(d) if left => aurora_i18n::tr!("ui-bars-back", d = d.to_string()),
            Some(d) => aurora_i18n::tr!("ui-bars-next", d = d.to_string()),
            None if left => aurora_i18n::tr!("ui-bars-no-previous-destination").to_owned(),
            None => aurora_i18n::tr!("ui-bars-no-next-destination").to_owned(),
        };
        if resp.on_hover_text(tip).clicked() && dest.is_some() {
            action = Some(if left {
                BarAction::TeleportBack
            } else {
                BarAction::TeleportForward
            });
        }
    }
    if icon_slot(ui, p, icons, next(SLOT), Id::new("bars_place_home"), "home-filled", true)
        .on_hover_text(aurora_i18n::tr!("common-go-home"))
        .clicked()
    {
        action = Some(BarAction::TeleportHome);
    }
    if let Some(a) = favorites_button(ui, p, icons, next(FAV_W), st) {
        action = Some(a);
    }

    // the place, then the star at the right end
    let star = Rect::from_min_size(Pos2::new(rect.right() - PAD - SLOT, top), Vec2::splat(SLOT));
    let loc = Rect::from_min_max(Pos2::new(x, top), Pos2::new(star.left() - GAP, top + SLOT));
    if let Some(a) = place(ui, p, icons, panels, st, loc, detail) {
        action = Some(a);
    }
    if let Some(a) = star_slot(ui, p, icons, star, st) {
        action = Some(BarAction::LandmarkStar(a));
    }
    action
}

/// Right-click menu of the place island: its two settings, also in
/// Preferences › Interface.
fn place_options(resp: &egui::Response, p: &Palette, st: &StatusInfo, action: &mut Option<BarAction>) {
    menu::context_menu_of_button(resp, p, |ui| options_body(ui, p, st, action));
}

fn options_body(ui: &mut egui::Ui, p: &Palette, st: &StatusInfo, action: &mut Option<BarAction>) {
    let look = st.islands;
    if menu::check(
        ui,
        p,
        "arrows-horizontal",
        aurora_i18n::tr!("ui-options-place-bar-stretch"),
        look.stretch,
        true,
    ) {
        *action = Some(BarAction::PlaceBar {
            stretch: !look.stretch,
            rights_text: look.rights_text,
        });
    }
    if menu::check(
        ui,
        p,
        "text-size",
        aurora_i18n::tr!("ui-options-place-bar-rights-text"),
        look.rights_text,
        true,
    ) {
        *action = Some(BarAction::PlaceBar {
            stretch: look.stretch,
            rights_text: !look.rights_text,
        });
    }
}

/// The Favorites button: the icon and its caret, at every width. Its menu
/// adds this place to the favourites or opens Places › Favorites (the list
/// of the favourites themselves comes with the favourites menu).
fn favorites_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, rect: Rect, st: &StatusInfo) -> Option<BarAction> {
    let resp = ui.interact(rect, Id::new("bars_place_favorites"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = resp.hovered() || open;
    slot_face(ui, p, rect, hot, false);
    let ink = slot_ink(p, hot, true);
    let icon = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + ICON / 2.0, rect.center().y), Vec2::splat(ICON));
    paint_icon(ui, icons, "bookmarks-filled", icon, ICON, ink);
    let caret = Rect::from_center_size(Pos2::new(icon.right() + 6.0 + CARET / 2.0, rect.center().y), Vec2::splat(CARET));
    paint_icon(ui, icons, "chevron-down", caret, CARET, ink);
    demo_open(ui.ctx(), &resp, super::demo::Open::Favorites);
    let mut out = None;
    menu::dropdown_placed(
        &resp,
        p,
        egui::PopupCloseBehavior::CloseOnClick,
        Some((egui::RectAlign::BOTTOM_START, 10.0)),
        |ui| {
            let in_favorites = st.parcel_landmark.as_ref().is_some_and(|lm| lm.favorite);
            if menu::item_if(ui, p, "star-filled", aurora_i18n::tr!("landmark-star-add-favorites"), !in_favorites) {
                out = Some(BarAction::LandmarkStar(StarAction::AddFavorite));
            }
            if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-bars-open-favorites")) {
                out = Some(BarAction::OpenFavorites);
            }
        },
    );
    resp.on_hover_text(aurora_i18n::tr!("common-favorites"));
    out
}

/// The landmark star of the island (LLLocationInputCtrl's add landmark
/// button): the outline star opens the quick menu, the amber one edits the
/// landmark of this parcel.
fn star_slot(ui: &mut egui::Ui, p: &Palette, icons: &Icons, rect: Rect, st: &StatusInfo) -> Option<StarAction> {
    let resp = ui.interact(rect, Id::new("bars_place_star"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = resp.hovered() || open;
    slot_face(ui, p, rect, hot, false);
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if let Some(lm) = &st.parcel_landmark {
        paint_icon(ui, icons, "star-filled", rect, ICON, p.amber);
        let tip = if lm.favorite {
            aurora_i18n::tr!("landmark-star-in-favorites", name = lm.name.clone())
        } else {
            aurora_i18n::tr!("landmark-star-in-landmarks", name = lm.name.clone())
        };
        return resp.on_hover_text(tip).clicked().then_some(StarAction::Edit(lm.item));
    }
    paint_icon(ui, icons, "star", rect, ICON, slot_ink(p, hot, true));
    demo_open(ui.ctx(), &resp, super::demo::Open::Star);
    let mut out = None;
    menu::dropdown_placed(
        &resp,
        p,
        egui::PopupCloseBehavior::CloseOnClick,
        Some((egui::RectAlign::BOTTOM_END, 10.0)),
        |ui| {
            out = super::star_menu(ui, p);
        },
    );
    resp.on_hover_text(aurora_i18n::tr!("landmark-star-add"));
    out
}

/// The place: rating, parcel, region (and position), the rights. A click
/// on it opens its menu: no chevron says so (the human's request, the
/// mockup has one).
fn place(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    st: &StatusInfo,
    rect: Rect,
    detail: Detail,
) -> Option<BarAction> {
    let mut action = None;
    let resp = ui.interact(rect, Id::new("bars_place_location"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = resp.hovered() || open;
    slot_face(ui, p, rect, hot, false);
    let under = if hot { p.raised } else { p.bar };
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;

    // from the right: the rights
    let rights_w = rights_width(ui, &st.parcel_icons, detail);
    let rights_left = rect.right() - LOC_RIGHT - rights_w;

    // from the left: the badge, the parcel, the region
    let mut x = rect.left() + LOC_LEFT;
    let badge = Rect::from_center_size(Pos2::new(x + BADGE / 2.0, cy), Vec2::splat(BADGE));
    crate::ui::widgets::paint_maturity_badge(&painter, p, st.maturity, badge, crate::ui::widgets::inner_radius(ui.visuals()));
    x += BADGE + LOC_GAP;
    let (parcel, region, position) = texts(st);
    let region = match (detail, region.is_empty()) {
        (Detail::Bare, _) => region,
        (_, true) => position,
        (_, false) => format!("{region} {position}"),
    };
    let room = (rights_left - LOC_GAP * 2.0 - x).max(0.0);
    let region_g = painter.layout_no_wrap(region, FontId::proportional(12.0), p.muted);
    let parcel_room = (room - region_g.size().x - LOC_GAP).max(room.min(40.0));
    // a name longer than the room ends with "…"
    let mut job = egui::text::LayoutJob::simple_singleline(parcel, parcel_font(ui.ctx()), p.ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(parcel_room);
    let parcel_g = painter.layout_job(job);
    let parcel_w = parcel_g.size().x;
    crate::ui::widgets::paint_ink_left(&painter, parcel_g, Pos2::new(x, cy), p.ink);
    x += parcel_w + LOC_GAP;
    let region_room = (rights_left - LOC_GAP * 2.0 - x).max(0.0);
    if region_g.size().x <= region_room + 0.5 {
        crate::ui::widgets::paint_ink_left(&painter, region_g, Pos2::new(x, cy), p.muted);
    }

    // the rights: each one says what it is; a click opens About Land
    let mut rx = rights_left;
    for (i, it) in st.parcel_icons.iter().enumerate() {
        let w = right_width(ui, it, detail);
        let slot = Rect::from_min_size(Pos2::new(rx, cy - SLOT_HEIGHT / 2.0), Vec2::new(w, SLOT_HEIGHT));
        rx += w + RIGHT_GAP;
        let r = ui.interact(slot, Id::new(("bars_place_right", i)), Sense::click());
        if r.hovered() {
            ui.painter()
                .rect_filled(slot.expand(1.0), ui.visuals().window_corner_radius, p.raised);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let col = if it.ok { p.muted_dim } else { p.danger };
        let glyph = crate::ui::parcel_icons::glyph_rect(slot);
        paint_icon(ui, icons, it.icon, glyph, GLYPH, col);
        if it.strike {
            let bg = if r.hovered() { p.raised } else { under };
            crate::ui::parcel_icons::paint_strike(ui.painter(), glyph, bg, col);
        }
        if let Some(t) = right_text(it, detail) {
            let g = ui.painter().layout_no_wrap(t, name_font(), col);
            crate::ui::widgets::paint_ink_left(ui.painter(), g, Pos2::new(glyph.right() + RIGHT_GAP, cy), col);
        }
        if r.on_hover_text(aurora_i18n::tr!("ui-parcel-icons-click-about-land", value0 = it.tip.to_string()))
            .clicked()
        {
            panels.about_land = !panels.about_land;
        }
    }

    demo_open(ui.ctx(), &resp, super::demo::Open::Place);
    menu::dropdown_placed(
        &resp,
        p,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        Some((egui::RectAlign::BOTTOM_START, 10.0)),
        |ui| {
            if let Some(a) = place_menu(ui, p, icons, panels, st) {
                action = Some(a);
                ui.close();
            }
        },
    );
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-bars-place-menu-tip"));
    place_options(&resp, p, st, &mut action);
    action
}

/// The place's menu: its head (rating, parcel, region, position and
/// rating's name), the SLURL to copy, a destination to type, the rights in
/// words, then About Land, the teleport history, home, Places and the
/// places seen last.
fn place_menu(ui: &mut egui::Ui, p: &Palette, icons: &Icons, panels: &mut Panels, st: &StatusInfo) -> Option<BarAction> {
    const WIDTH: f32 = 400.0;
    let mut action = None;
    ui.set_width(WIDTH);
    let (parcel, region, _) = texts(st);
    // the head
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_space(6.0);
        crate::ui::widgets::maturity_badge(ui, p, st.maturity, BADGE);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                egui::RichText::new(&parcel)
                    .font(FontId::new(14.0, crate::ui::fonts::semibold(ui.ctx())))
                    .color(p.ink),
            );
            let mut sub = vec![];
            if !region.is_empty() {
                sub.push(region.clone());
            }
            sub.push(format!("{:.0}, {:.0}, {:.0}", st.position.x, st.position.y, st.position.z));
            if !st.maturity.is_empty() {
                sub.push(st.maturity.to_owned());
            }
            ui.label(egui::RichText::new(sub.join(" · ")).size(12.0).color(p.muted));
        });
        // the star: edits this parcel's landmark, or creates one
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(SLOT), Sense::click());
            ui.painter().rect_filled(
                rect,
                ui.visuals().window_corner_radius,
                if resp.hovered() {
                    p.raised.lerp_to_gamma(p.ink, 0.16)
                } else {
                    p.raised
                },
            );
            let lit = st.parcel_landmark.as_ref();
            let (icon, tint) = if lit.is_some() { ("star-filled", p.amber) } else { ("star", p.ink) };
            paint_icon(ui, icons, icon, rect, ICON, tint);
            let tip = match lit {
                Some(lm) if lm.favorite => aurora_i18n::tr!("landmark-star-in-favorites", name = lm.name.clone()),
                Some(lm) => aurora_i18n::tr!("landmark-star-in-landmarks", name = lm.name.clone()),
                None => aurora_i18n::tr!("landmark-star-add").to_owned(),
            };
            if resp.on_hover_text(tip).clicked() {
                action = Some(BarAction::LandmarkStar(match lit {
                    Some(lm) => StarAction::Edit(lm.item),
                    None => StarAction::MoreOptions,
                }));
            }
        });
    });
    ui.add_space(6.0);
    // the SLURL, to copy
    crate::ui::widgets::pills::field_frame(ui, p).show(ui, |ui| {
        ui.set_width(WIDTH - 18.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            crate::ui::widgets::glyph(ui, "link", p.muted, 14.0);
            let copy = aurora_i18n::tr!("ui-bars-copy");
            let button_w = 80.0;
            let text_w = ui.available_width() - button_w - 7.0;
            ui.add_sized(
                Vec2::new(text_w, 20.0),
                egui::Label::new(egui::RichText::new(&st.slurl).size(12.5).color(p.ink)).truncate(),
            );
            if crate::ui::widgets::action_button(
                ui,
                p,
                "copy-filled",
                copy,
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(button_w, 22.0),
            )
            .clicked()
            {
                ui.ctx().copy_text(st.slurl.clone());
            }
        });
    });
    ui.add_space(6.0);
    // a destination: SLURL, Region/x/y/z or a region's name, then Enter
    let dest_id = Id::new("bars_place_destination");
    let mut text = ui.ctx().data_mut(|d| d.get_temp::<String>(dest_id)).unwrap_or_default();
    let dest = &mut text;
    let field = ui
        .horizontal(|ui| {
            crate::ui::widgets::pills::icon_field(
                ui,
                p,
                "send-filled",
                dest,
                aurora_i18n::tr!("ui-bars-slurl-region-x-y-z-or-region-name"),
                WIDTH,
            )
        })
        .inner;
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let typed = std::mem::take(dest);
        if !typed.trim().is_empty() {
            action = Some(BarAction::TeleportTo(typed));
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(dest_id, text));
    // the rights, in words
    menu::heading(ui, p, aurora_i18n::tr!("ui-bars-parcel-rights"));
    for it in &st.parcel_icons {
        let color = if it.ok { p.ink } else { p.danger };
        let icon_color = if it.ok { p.muted } else { p.danger };
        let text = crate::ui::parcel_icons::headline(&it.tip);
        menu::info_colored(ui, it.icon, text, icon_color, color).on_hover_text(&it.tip);
    }
    menu::separator(ui, p);
    if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("common-about-land")) {
        panels.about_land = !panels.about_land;
    }
    if menu::item_keys(ui, p, "history", aurora_i18n::tr!("common-teleport-history"), "Alt+H") {
        action = Some(BarAction::TeleportHistory);
    }
    if menu::item(ui, p, "home-filled", aurora_i18n::tr!("common-go-home")) {
        action = Some(BarAction::TeleportHome);
    }
    if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("common-places")) {
        crate::ui::widgets::toggle_floater(ui.ctx(), "places", &mut panels.places);
    }
    if !st.recent.is_empty() {
        menu::heading(ui, p, aurora_i18n::tr!("ui-bars-recent-places"));
        for r in &st.recent {
            if menu::item_keys(ui, p, "arrow-back-up", &r.name, &r.region) {
                action = Some(BarAction::TeleportTo(r.slurl.clone()));
            }
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths() -> Widths {
        Widths {
            fixed: 400.0,
            parcel: 220.0,
            region: 70.0,
            region_position: 160.0,
            forbidden_names: 110.0,
            all_names: 460.0,
        }
    }

    /// The three widths of the mockup: 1280 shows the parcel and the region,
    /// 1536 adds the position and names the forbidden rights, 1920 names
    /// them all (with the names on).
    #[test]
    fn the_place_says_more_as_the_room_grows() {
        let w = widths();
        assert_eq!(choose(&w, 715.0, true), Detail::Bare);
        assert_eq!(choose(&w, 900.0, true), Detail::ForbiddenNamed);
        assert_eq!(choose(&w, 1300.0, true), Detail::AllNamed);
        assert_eq!(choose(&w, 800.0, true), Detail::Position);
    }

    /// Off by default: the rights stay icons, whatever the room.
    #[test]
    fn without_names_the_rights_stay_icons() {
        let w = widths();
        assert_eq!(choose(&w, 5000.0, false), Detail::Position);
        assert_eq!(choose(&w, 700.0, false), Detail::Bare);
        assert_eq!(choose(&w, 10.0, false), Detail::Bare);
    }

    #[test]
    fn a_level_is_never_wider_than_the_room_it_was_chosen_for() {
        let w = widths();
        for room in (600..1600).step_by(10) {
            let room = room as f32;
            let d = choose(&w, room, true);
            assert!(d == Detail::Bare || w.natural(d) <= room, "{room}: {d:?}");
        }
    }
}
