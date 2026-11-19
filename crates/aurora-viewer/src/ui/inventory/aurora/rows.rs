//! Aurora's inventory rows: 28 px high, the list's width, a long name cut
//! short instead of widening the list. The type icon, the name, its pills
//! (worn, link) and favourite star, the rights at the end of the name, and
//! from the medium width the columns (type, rights, date) at the right. A
//! hovered wearable offers "Add" or "Take off" in place, and the wide
//! window a "more actions" button. What a row does (selection, drag and
//! drop, tooltip, menus) is `item_response` / `folder_response`, shared
//! with the Legacy Viewer's window.

use super::super::permissions::{Permissions, Rights};
use super::super::view::{COLUMNS, ColumnId};
use super::super::*;
use super::columns::{ColumnFit, Columns, GAP, PAD};
use super::{Frame, Layout, rights};
use crate::ui::{sheet, widgets};
use egui::{Color32, Pos2, Rangef, Rect, Sense, Vec2};

/// The height of a row, without the spacing between rows.
pub const HEIGHT: f32 = 24.0;
/// Where a placeholder's text starts ("Empty" under an open folder): past
/// the caret and the icon.
pub const LEAD: f32 = 34.0;
const ICON: f32 = 16.0;
const CARET: f32 = 14.0;
const NAME: f32 = 13.0;
const SMALL: f32 = 12.0;

/// The compact tree retains its inline rights; expanded views use a column.
fn rights_in_name(frame: Frame) -> bool {
    frame.layout == Layout::Compact && frame.rights != Rights::Hidden
}

/// The rect of the row about to be drawn, and the same across the whole
/// list (its highlight starts at the list's edge, before the indentation).
fn row_rects(ui: &egui::Ui) -> (Rect, Rect) {
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width().max(0.0), HEIGHT));
    let left = ui.clip_rect().left().min(rect.left());
    (rect, Rect::from_x_y_ranges(left..=rect.right(), rect.y_range()))
}

/// In the explorer's list a row is dragged by its icon and name only, as
/// in Windows Explorer's Details view: a click anywhere on it selects it,
/// and a drag from its other columns draws the rubber band
/// (`marquee`). Elsewhere (the tree) the whole row drags, as before.
fn row_sense(frame: Frame, draggable: bool) -> Sense {
    if frame.explorer || !draggable {
        Sense::click()
    } else {
        Sense::click_and_drag()
    }
}

/// The explorer's drag handle of a row: from its left edge to the end of
/// its name (`row_sense`). Laid out before the row itself: egui gives a
/// click to the row on top and a drag to the handle under it, where a
/// drag-only widget on top would hide the row's clicks (egui's hit_test).
#[allow(clippy::too_many_arguments)]
fn name_handle(
    ui: &egui::Ui,
    frame: Frame,
    st: &mut InventoryUi,
    id: Uuid,
    rect: Rect,
    name_left: f32,
    name: &str,
    name_end: f32,
    folder: bool,
    draggable: bool,
) {
    if !frame.explorer || !draggable {
        return;
    }
    let width = ui
        .painter()
        .layout_no_wrap(name.to_owned(), egui::FontId::proportional(NAME), Color32::PLACEHOLDER)
        .size()
        .x;
    let end = (name_left + width + 4.0).min(name_end);
    let handle = Rect::from_x_y_ranges(rect.left()..=end.clamp(rect.left(), rect.right()), rect.y_range());
    // click as well as drag: egui starts a drag-only widget's drag at the
    // press, which would take the click from the row above it
    let response = ui.interact(handle, ui.make_persistent_id(("inv_name_drag", id)), Sense::click_and_drag());
    drag::source(ui, &response, id, folder, st);
}

/// The keyboard's place when it is not the selection (Ctrl+↑/↓).
fn focus_ring(ui: &egui::Ui, p: &Palette, full: Rect, st: &InventoryUi, id: Uuid) {
    if st.focus == Some(id) {
        ui.painter().rect_stroke(
            full.shrink(0.5),
            ui.visuals().window_corner_radius,
            egui::Stroke::new(1.0, p.violet),
            egui::StrokeKind::Inside,
        );
    }
}

fn background(ui: &egui::Ui, p: &Palette, full: Rect, selected: bool) {
    let fill = if selected {
        p.violet.gamma_multiply(0.28)
    } else if ui.rect_contains_pointer(full) && ui.ctx().dragged_id().is_none() {
        p.raised
    } else {
        return;
    };
    ui.painter().rect_filled(full, ui.visuals().window_corner_radius, fill);
}

fn paint_icon(ui: &egui::Ui, icons: &Icons, name: &str, center: Pos2, tint: Color32) {
    if let Some(texture) = icons.get(name) {
        ui.painter().image(
            texture.id(),
            Rect::from_center_size(center, Vec2::splat(super::super::category::row_icon_size(name, ICON))),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// A text cut short at `width`, on one line.
fn cut(ui: &egui::Ui, text: &str, size: f32, color: Color32, width: f32, italics: bool) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            italics,
            ..egui::TextFormat::simple(egui::FontId::proportional(size), color)
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

/// The tint of a pill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tone {
    /// Worn, or an active gesture: the "active" colour.
    Worn,
    /// A plain fact (a link).
    Plain,
}

fn pill_width(ui: &egui::Ui, text: &str) -> f32 {
    widgets::row_tag_width(ui, text)
}

fn paint_pill(ui: &egui::Ui, p: &Palette, text: &str, tone: Tone, left: f32, center_y: f32) -> f32 {
    widgets::paint_row_tag(ui, p, text, tone == Tone::Worn, left, center_y)
}

/// The pills of an item: worn (where, at the wide width) or an active
/// gesture, and a link.
fn pills(inv: &Inventory, it: &aurora_net::inventory::InvItem, facts: &Facts, layout: Layout) -> Vec<(String, Tone)> {
    let mut pills = Vec::new();
    let original = rules::original(inv, it.id).unwrap_or(it.id);
    if facts.active_gestures.contains(&original) {
        pills.push((aurora_i18n::tr!("gesture-active").to_owned(), Tone::Worn));
    } else if facts.worn.contains(&original) {
        let worn = aurora_i18n::tr!("common-worn");
        let text = match facts.worn_at.get(&original) {
            Some(point) if layout == Layout::Wide => format!("{worn} · {point}"),
            _ => worn.to_owned(),
        };
        pills.push((text, Tone::Worn));
    }
    if matches!(it.asset_type, 24 | 25) {
        pills.push((aurora_i18n::tr!("inventory-pill-link").to_owned(), Tone::Plain));
    }
    pills
}

/// The short date of a row: "14 oct.", with the year when it is not this
/// one.
pub fn short_date(seconds: i64, now: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    let (year, month, day) = aurora_net::profile::civil_from_days(seconds.div_euclid(86400));
    let (this_year, _, _) = aurora_net::profile::civil_from_days(now.div_euclid(86400));
    crate::ui::land::day_month((year != this_year).then_some(year), month as usize, i64::from(day))
}

/// The name of an item's type, as the type column shows it: "Clothing ·
/// Skirt", "Object", "Link · Notecard"…
pub fn kind_name(inv: &Inventory, id: Uuid) -> String {
    if let Some(folder) = inv.folders.get(&id) {
        let count = folder.items.len() + folder.children.len();
        return if folder.state == FetchState::Fetched {
            aurora_i18n::tr!("inventory-kind-folder-count", count = count)
        } else {
            aurora_i18n::tr!("inventory-kind-folder").to_owned()
        };
    }
    let Some(it) = inv.items.get(&id) else {
        return String::new();
    };
    let linked = matches!(it.asset_type, 24 | 25);
    if it.asset_type == 25 {
        return aurora_i18n::tr!("inventory-kind-folder-link").to_owned();
    }
    let original = rules::original(inv, id).and_then(|id| inv.items.get(&id));
    let kind = match original {
        Some(original) => item_kind(original),
        None => aurora_i18n::tr!("inventory-kind-broken-link").to_owned(),
    };
    if linked && original.is_some() {
        aurora_i18n::tr!("inventory-kind-link-to", kind = kind)
    } else {
        kind
    }
}

fn item_kind(it: &aurora_net::inventory::InvItem) -> String {
    let wearable = || {
        crate::world::appearance::wearables()
            .get((it.flags & 0xff) as usize)
            .copied()
            .unwrap_or_default()
    };
    let simple = match (it.asset_type, it.inv_type) {
        (5, _) => return aurora_i18n::tr!("inventory-kind-clothing-of", kind = wearable()),
        (13, _) => return aurora_i18n::tr!("inventory-kind-body-part-of", kind = wearable()),
        (_, 15) => aurora_i18n::tr!("inventory-kind-snapshot"),
        (0, _) => aurora_i18n::tr!("common-texture"),
        (1, _) => aurora_i18n::tr!("common-sound"),
        (2, _) => aurora_i18n::tr!("inventory-kind-calling-card"),
        (3, _) => aurora_i18n::tr!("common-landmark"),
        (6, _) => aurora_i18n::tr!("common-object"),
        (7, _) => aurora_i18n::tr!("inventory-kind-notecard"),
        (10, _) => aurora_i18n::tr!("inventory-kind-script"),
        (20, _) => aurora_i18n::tr!("inventory-kind-animation"),
        (21, _) => aurora_i18n::tr!("inventory-kind-gesture"),
        (49, _) => aurora_i18n::tr!("inventory-kind-mesh"),
        (56, _) => aurora_i18n::tr!("inventory-kind-settings"),
        (57, _) => aurora_i18n::tr!("inventory-kind-material"),
        _ => aurora_i18n::tr!("inventory-kind-other"),
    };
    simple.to_owned()
}

/// What a hovered row offers in place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Quick {
    /// Add without replacing, independently of the double-click preferences.
    Add,
    /// Wear a non-stackable wearable, or take off a worn item.
    Wear(bool),
    /// Its "more actions" menu (the wide width).
    More,
}

const QUICK_H: f32 = 22.0;

fn quick_width(ui: &egui::Ui, quick: Quick) -> f32 {
    match quick {
        Quick::Add | Quick::Wear(_) => {
            let label = quick_label(quick);
            ui.painter()
                .layout_no_wrap(label.to_owned(), egui::FontId::proportional(SMALL), Color32::PLACEHOLDER)
                .size()
                .x
                + 16.0
        }
        Quick::More => QUICK_H,
    }
}

fn quick_label(quick: Quick) -> &'static str {
    match quick {
        Quick::Add => aurora_i18n::tr!("common-add"),
        Quick::Wear(true) => aurora_i18n::tr!("inventory-take-off"),
        Quick::Wear(false) => aurora_i18n::tr!("common-wear"),
        Quick::More => aurora_i18n::tr!("inventory-more-actions"),
    }
}

/// The buttons a hovered item shows: add or take off a wearable, and
/// "more actions" at the wide width. Body parts and physics only replace;
/// a worn body part cannot be taken off.
pub(super) fn quick_buttons(inv: &Inventory, id: Uuid, facts: &Facts, layout: Layout) -> Vec<Quick> {
    let mut quick = Vec::new();
    if let Some(original) = rules::original(inv, id).and_then(|id| inv.items.get(&id))
        && matches!(original.asset_type, 5 | 6 | 13)
        && !rules::in_type(inv, id, 14)
    {
        let worn = facts.worn.contains(&original.id);
        if !(worn && original.asset_type == 13) {
            let add = !worn && (original.asset_type == 6 || (original.asset_type == 5 && original.flags & 0xff != 15));
            quick.push(if add { Quick::Add } else { Quick::Wear(worn) });
        }
    }
    if layout == Layout::Wide {
        quick.push(Quick::More);
    }
    quick
}

/// Draw the quick buttons ending at `right`, centred on `center_y`.
#[allow(clippy::too_many_arguments)]
pub(super) fn show_quick(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    id: Uuid,
    quick: &[Quick],
    right: f32,
    center_y: f32,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let mut x = right;
    for (n, q) in quick.iter().enumerate().rev() {
        let w = quick_width(ui, *q);
        let rect = Rect::from_min_size(Pos2::new(x - w, center_y - QUICK_H / 2.0), Vec2::new(w, QUICK_H));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_quick", id, n)), rect));
        x = rect.left() - 4.0;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("inventory_quick", id, n)));
        match q {
            Quick::Add | Quick::Wear(_) => {
                let clicked = child
                    .add_enabled_ui(
                        st.pending.is_none() && !facts.appearance_busy && facts.rlva_allows(inv, id, *q == Quick::Add),
                        |ui| widgets::action_button(ui, p, "", quick_label(*q), widgets::ActionKind::Flat, rect.size()).clicked(),
                    )
                    .inner;
                if clicked {
                    st.select(ui, id, false);
                    if *q == Quick::Add {
                        // Explicit Add follows LLObjectBridge::performAction("wear_add")
                        // (indra/newview/llinventorybridge.cpp), never the double-click defaults.
                        if let Some(item) = rules::original(inv, id) {
                            actions.push(if rules::library(inv, item) {
                                InvAction::WearCopy { item, replace: false }
                            } else {
                                InvAction::Appearance(crate::world::appearance::Action::WearItem {
                                    item,
                                    replace: false,
                                    point: 0,
                                })
                            });
                        }
                    } else {
                        context::double_click(inv, id, st, prefs, facts, actions);
                    }
                }
            }
            Quick::More => {
                let more = aurora_i18n::tr!("inventory-more-actions");
                if widgets::action_button(&mut child, p, "dots-filled", "", widgets::ActionKind::Flat, rect.size())
                    .on_hover_text(more)
                    .clicked()
                {
                    st.select(ui, id, true);
                    st.row_menu = Some((id, rect.left_bottom()));
                    st.background_menu = false;
                }
            }
        }
    }
}

/// The name of the creator of `it` (its original's for a link) as the
/// creator column shows it, from `Facts::creator_names`; `None` while it is
/// not known.
pub(in super::super) fn creator_key(inv: &Inventory, it: &aurora_net::inventory::InvItem) -> (Uuid, Uuid) {
    let original = rules::original(inv, it.id).and_then(|id| inv.items.get(&id)).unwrap_or(it);
    let owner = if original.group_owned { Uuid::nil() } else { original.owner };
    (original.creator, owner)
}

/// What the worn column says of an item: worn (where, at the wide width),
/// or an active gesture.
fn worn_text(inv: &Inventory, id: Uuid, facts: &Facts, layout: Layout) -> Option<String> {
    let original = rules::original(inv, id).unwrap_or(id);
    if facts.active_gestures.contains(&original) {
        Some(aurora_i18n::tr!("gesture-active").to_owned())
    } else if facts.worn.contains(&original) {
        let worn = aurora_i18n::tr!("common-worn");
        Some(match facts.worn_at.get(&original) {
            Some(point) if layout == Layout::Wide => format!("{worn} · {point}"),
            _ => worn.to_owned(),
        })
    } else {
        None
    }
}

/// Paint the columns of the medium and wide widths for `id`, and note in
/// `fit` how wide each one's text is.
#[allow(clippy::too_many_arguments)]
fn paint_columns(
    ui: &egui::Ui,
    p: &Palette,
    inv: &Inventory,
    view: &super::super::view::ViewCache,
    facts: &Facts,
    fit: &mut ColumnFit,
    id: Uuid,
    cols: Columns,
    frame: Frame,
    permissions: Option<Permissions>,
    center_y: f32,
    selected: bool,
) {
    let ink = if selected { p.violet_pale } else { p.muted };
    let item = inv.items.get(&id);
    let folder = inv.folders.get(&id);
    let text = |fit: &mut ColumnFit, column: ColumnId, range: Rangef, text: &str| {
        if text.is_empty() {
            return;
        }
        let natural = ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::proportional(SMALL), ink);
        fit.note(column, natural.size().x);
        let galley = cut(ui, text, SMALL, ink, range.span(), false);
        widgets::paint_row_text(ui.painter(), galley, range.min, center_y, ink);
    };
    for (column, range) in cols.iter() {
        match column {
            ColumnId::Kind => {
                // the items' column says a folder's count when shown
                let kind = match folder {
                    Some(_) if cols.get(ColumnId::Count).is_some() => aurora_i18n::tr!("inventory-kind-folder").to_owned(),
                    _ => kind_name(inv, id),
                };
                text(fit, column, range, &kind);
            }
            ColumnId::Rights => {
                if let Some(permissions) = permissions {
                    fit.note(column, rights::width(frame.rights));
                    rights::paint(ui, p, permissions, frame.rights, range.min, center_y, selected);
                }
            }
            ColumnId::Date => {
                let seconds = item.map_or_else(|| view.folder_date(id), |it| it.created_at);
                text(fit, column, range, &short_date(seconds, super::super::view::now()));
            }
            ColumnId::Creator => {
                if let Some(it) = item.filter(|it| it.asset_type != 25) {
                    let key = creator_key(inv, it);
                    if !key.0.is_nil() {
                        let name = facts
                            .creator_names
                            .get(&key)
                            .cloned()
                            .flatten()
                            .unwrap_or_else(|| aurora_i18n::tr!("common-loading-2").to_owned());
                        text(fit, column, range, &name);
                    }
                }
            }
            ColumnId::Description => {
                if let Some(it) = item {
                    text(fit, column, range, it.desc.lines().next().unwrap_or_default());
                }
            }
            ColumnId::Worn => {
                if let Some(worn) = worn_text(inv, id, facts, frame.layout) {
                    text(fit, column, range, &worn);
                }
            }
            ColumnId::Count => {
                if let Some(f) = folder.filter(|f| f.state == FetchState::Fetched) {
                    text(fit, column, range, &aurora_i18n::integer((f.items.len() + f.children.len()) as i64));
                }
            }
        }
    }
}

/// An item's row.
#[allow(clippy::too_many_arguments)]
pub(in super::super) fn item(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    id: Uuid,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let (Some(it), Some(frame)) = (inv.items.get(&id), st.aurora) else {
        return;
    };
    let opacity = ui.opacity();
    ui.multiply_opacity(clipboard::opacity(inv, st, id));
    st.visible.push(id);
    let (rect, full) = row_rects(ui);
    let selected = st.selection.contains(&id);
    let cols = Columns::new(frame, full);
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_row_columns", id)), cols));
    let icon_center = Pos2::new(rect.left() + 4.0 + ICON / 2.0, rect.center().y);
    let name_left = icon_center.x + ICON / 2.0 + 8.0;
    let (icon_name, tint) = inventory_item_icon(inv, it, p);
    // Renaming: the field in place of the name.
    let name_rect = Rect::from_x_y_ranges(name_left..=cols.name_end, rect.y_range().shrink(2.0));
    let mut field = ui.new_child(egui::UiBuilder::new().max_rect(name_rect).id_salt(("inventory_rename_row", id)));
    field.set_clip_rect(Rect::from_x_y_ranges(name_rect.x_range(), rect.y_range()).intersect(ui.clip_rect()));
    if rename_row(&mut field, p, inv, id, st, actions) {
        background(ui, p, full, selected);
        paint_icon(ui, icons, icon_name, icon_center, tint);
        ui.advance_cursor_after_rect(rect);
        ui.set_opacity(opacity);
        return;
    }
    name_handle(ui, frame, st, id, rect, name_left, &it.name, cols.name_end, false, true);
    let r = ui.interact(rect, ui.make_persistent_id(("inv_item", id)), row_sense(frame, true));
    ui.advance_cursor_after_rect(rect);
    #[cfg(test)]
    tests::record(ui, id, rect);
    if frame.explorer {
        super::marquee::record(st, id, rect);
    }
    if st.selected == Some(id) {
        st.selected_rect = Some(rect);
    }
    background(ui, p, full, selected);
    focus_ring(ui, p, full, st, id);
    paint_icon(ui, icons, icon_name, icon_center, tint);
    let permissions = Permissions::inventory(inv, id);
    let hovered = ui.rect_contains_pointer(full) && ui.ctx().dragged_id().is_none();
    let quick = if hovered {
        quick_buttons(inv, id, facts, frame.layout)
    } else {
        Vec::new()
    };
    let quick_w: f32 = quick.iter().map(|q| quick_width(ui, *q) + 4.0).sum();
    let trailing = if rights_in_name(frame) && permissions.is_some() {
        rights::width(frame.rights)
    } else {
        0.0
    };
    let pills = pills(inv, it, facts, frame.layout);
    let pills_w: f32 = pills.iter().map(|(text, _)| pill_width(ui, text) + 6.0).sum();
    let favorite = rules::is_favorite(inv, id);
    let star_w = if favorite { 20.0 } else { 0.0 };
    let room = cols.name_end - name_left - quick_w - trailing - pills_w - star_w - if trailing > 0.0 { 8.0 } else { 0.0 };
    // LLItemBridge::getLabelStyle / LLGestureBridge::getLabelStyle
    // (indra/newview/llinventorybridge.cpp) supply the worn/active state.
    // By human choice, names share the tag's semi-bold face and text colour.
    let active = pills.iter().any(|(_, tone)| *tone == Tone::Worn);
    let ink = if active { widgets::row_tag_ink(p, true) } else { p.ink };
    // an Aurora Weather code at the end of a sky's or day's name: the tag
    let coded = super::super::weather_coded_name(it);
    let shown = coded.unwrap_or(&it.name);
    let name = if selected || active {
        sheet::semibold(ui, shown, NAME, ink)
    } else {
        egui::text::LayoutJob::single_section(shown.to_owned(), egui::TextFormat::simple(egui::FontId::proportional(NAME), ink))
    };
    let mut name = name;
    if coded.is_some() {
        super::super::weather_tag(p, &mut name, egui::FontId::proportional(NAME));
    }
    name.wrap = egui::text::TextWrapping::truncate_at_width(room.max(24.0));
    if matches!(it.asset_type, 24 | 25) {
        for section in &mut name.sections {
            section.format.italics = true;
        }
    }
    let galley = ui.painter().layout_job(name);
    let mut x = name_left + galley.size().x;
    let clip = ui.clip_rect();
    ui.set_clip_rect(clip.intersect(Rect::from_min_max(full.min, Pos2::new(cols.name_end, full.bottom()))));
    widgets::paint_row_text(ui.painter(), galley, name_left, rect.center().y, ink);
    for (text, tone) in &pills {
        x += 6.0;
        x += paint_pill(ui, p, text, *tone, x, rect.center().y);
    }
    if favorite {
        widgets::paint_glyph(
            ui.painter(),
            "star-filled",
            Pos2::new(x + 6.0 + 7.0, rect.center().y),
            14.0,
            p.amber,
        );
    }
    if trailing > 0.0
        && let Some(permissions) = permissions
    {
        rights::paint(
            ui,
            p,
            permissions,
            frame.rights,
            cols.name_end - trailing,
            rect.center().y,
            selected,
        );
    }
    ui.set_clip_rect(clip);
    paint_columns(
        ui,
        p,
        inv,
        &st.view,
        facts,
        &mut st.column_fit,
        id,
        cols,
        frame,
        permissions,
        rect.center().y,
        selected,
    );
    ui.set_opacity(opacity);
    item_response(ui, p, inv, it, r, permissions, st, prefs, facts, actions);
    if !quick.is_empty() {
        let right = cols.name_end - trailing - if trailing > 0.0 { 8.0 } else { 0.0 };
        ui.set_clip_rect(clip.intersect(Rect::from_x_y_ranges(name_rect.x_range(), rect.y_range())));
        show_quick(ui, p, inv, id, &quick, right, rect.center().y, st, prefs, facts, actions);
        ui.set_clip_rect(clip);
    }
}

/// A folder's row: with its caret in a tree, without it in the explorer,
/// where a double-click goes into the folder.
#[allow(clippy::too_many_arguments)]
pub(in super::super) fn folder(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    id: Uuid,
    open: &mut bool,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let (Some(folder), Some(frame)) = (inv.folders.get(&id), st.aurora) else {
        return;
    };
    let opacity = ui.opacity();
    ui.multiply_opacity(clipboard::opacity(inv, st, id));
    st.visible.push(id);
    let (rect, full) = row_rects(ui);
    let selected = st.selection.contains(&id);
    let cols = Columns::new(frame, full);
    let caret_center = Pos2::new(rect.left() + 2.0 + CARET / 2.0, rect.center().y);
    let icon_center = if frame.explorer {
        Pos2::new(rect.left() + 4.0 + ICON / 2.0, rect.center().y)
    } else {
        Pos2::new(caret_center.x + CARET / 2.0 + 4.0 + ICON / 2.0, rect.center().y)
    };
    let name_left = icon_center.x + ICON / 2.0 + 8.0;
    let tint = folder_colors::tint(prefs, id).unwrap_or(if selected { p.violet_pale } else { p.muted });
    let icon = folder_icon(folder.info.type_default, *open && !frame.explorer);
    let name_rect = Rect::from_x_y_ranges(name_left..=cols.name_end, rect.y_range().shrink(2.0));
    let mut field = ui.new_child(egui::UiBuilder::new().max_rect(name_rect).id_salt(("inventory_rename_row", id)));
    field.set_clip_rect(Rect::from_x_y_ranges(name_rect.x_range(), rect.y_range()).intersect(ui.clip_rect()));
    if rename_row(&mut field, p, inv, id, st, actions) {
        background(ui, p, full, selected);
        paint_icon(ui, icons, icon, icon_center, tint);
        ui.advance_cursor_after_rect(rect);
        ui.set_opacity(opacity);
        return;
    }
    let draggable = folder_draggable(inv, id, prefs);
    let sense = row_sense(frame, draggable);
    name_handle(
        ui,
        frame,
        st,
        id,
        rect,
        name_left,
        &folder_name(inv, id),
        cols.name_end,
        true,
        draggable,
    );
    let r = ui.interact(rect, ui.make_persistent_id(("inv_folder", id)), sense);
    ui.advance_cursor_after_rect(rect);
    #[cfg(test)]
    tests::record(ui, id, rect);
    if frame.explorer {
        super::marquee::record(st, id, rect);
    }
    if st.selected == Some(id) {
        st.selected_rect = Some(rect);
    }
    background(ui, p, full, selected);
    focus_ring(ui, p, full, st, id);
    paint_icon(ui, icons, icon, icon_center, tint);
    let caret = (!frame.explorer).then(|| {
        let caret_rect = Rect::from_center_size(caret_center, Vec2::splat(CARET + 4.0));
        let caret = ui.interact(caret_rect, ui.make_persistent_id(("inv_caret", id)), Sense::click());
        let color = if caret.hovered() { p.ink } else { p.muted };
        widgets::paint_glyph(ui.painter(), widgets::disclosure_caret(*open), caret_center, CARET, color);
        caret
    });
    if let Some(caret) = &caret {
        clipboard::demo_caret(caret, st, id);
        if caret.clicked() {
            *open = !*open;
        }
    }
    let name = folder_name(inv, id);
    let label = match folder.state {
        FetchState::Fetching => aurora_i18n::tr!("status-name-loading", name = name),
        FetchState::Failed => aurora_i18n::tr!("ui-inventory-failed-2", name = name),
        _ => name,
    };
    // A tree counts what a folder holds at its end; the columns say it.
    let count = (cols.get(ColumnId::Kind).is_none() && cols.get(ColumnId::Count).is_none() && folder.state == FetchState::Fetched)
        .then(|| aurora_i18n::integer((folder.items.len() + folder.children.len()) as i64));
    let count_w = count.as_ref().map_or(0.0, |count| {
        ui.painter()
            .layout_no_wrap(count.clone(), egui::FontId::proportional(11.5), p.muted)
            .size()
            .x
            + 8.0
    });
    let star_w = if folder.info.favorite { 20.0 } else { 0.0 };
    // the " (Protected) " suffix of LLFolderViewItem::drawLabel
    // (indra/llui/llfolderviewitem.cpp), as the rows' pill
    let locked = prefs.protected.contains(&id).then(|| aurora_i18n::tr!("inventory-pill-protected"));
    let locked_w = locked.map_or(0.0, |text| pill_width(ui, text) + 6.0);
    let job = if selected {
        sheet::semibold(ui, &label, NAME, p.ink)
    } else {
        egui::text::LayoutJob::single_section(label, egui::TextFormat::simple(egui::FontId::proportional(NAME), p.ink))
    };
    let mut job = job;
    job.wrap = egui::text::TextWrapping::truncate_at_width((cols.name_end - name_left - count_w - star_w - locked_w).max(24.0));
    let galley = ui.painter().layout_job(job);
    let mut name_end = name_left + galley.size().x;
    let clip = ui.clip_rect();
    ui.set_clip_rect(clip.intersect(Rect::from_min_max(full.min, Pos2::new(cols.name_end, full.bottom()))));
    widgets::paint_row_text(ui.painter(), galley, name_left, rect.center().y, p.ink);
    if let Some(text) = locked {
        name_end += 6.0;
        name_end += paint_pill(ui, p, text, Tone::Plain, name_end, rect.center().y);
    }
    if folder.info.favorite {
        widgets::paint_glyph(
            ui.painter(),
            "star-filled",
            Pos2::new(name_end + 6.0 + 7.0, rect.center().y),
            14.0,
            p.amber,
        );
    }
    if let Some(count) = count {
        let galley = ui.painter().layout_no_wrap(
            count,
            egui::FontId::proportional(11.5),
            if selected { p.violet_pale } else { p.muted },
        );
        let left = cols.name_end - galley.size().x;
        widgets::paint_row_text(
            ui.painter(),
            galley,
            left,
            rect.center().y,
            if selected { p.violet_pale } else { p.muted },
        );
    }
    ui.set_clip_rect(clip);
    paint_columns(
        ui,
        p,
        inv,
        &st.view,
        facts,
        &mut st.column_fit,
        id,
        cols,
        frame,
        None,
        rect.center().y,
        selected,
    );
    let lead = caret.as_ref().map(|caret| caret.rect);
    let was_open = *open;
    ui.set_opacity(opacity);
    let r = folder_response(ui, p, inv, id, r, lead, open, st, prefs, facts, actions);
    if frame.explorer {
        // The explorer goes into a folder (a double-click, or cargo held
        // over it) instead of opening it in place.
        if r.double_clicked() || (*open && !was_open) {
            st.browse = id;
            st.selection.clear();
            st.selected = None;
        }
        *open = false;
    } else if r.double_clicked() {
        *open = !*open;
    }
}

/// A group's header in the explorer's list (`groups.rs`): its caret, its
/// name and how many entries it holds, a line to the row's end. True when
/// it is clicked (it opens or closes).
pub fn group_header(ui: &mut egui::Ui, p: &Palette, name: &str, count: usize, open: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ui.min_size().y.max(24.0)), Sense::click());
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_group", name.to_owned())), rect));
    let ink = if response.hovered() { p.ink } else { p.muted };
    let caret = Pos2::new(rect.left() + PAD + CARET / 2.0, rect.center().y);
    widgets::paint_glyph(ui.painter(), widgets::disclosure_caret(open), caret, CARET, ink);
    let label = sheet::semibold(ui, name, SMALL, ink);
    let galley = ui.painter().layout_job(label);
    let left = caret.x + CARET / 2.0 + 6.0;
    let mut right = left + galley.size().x;
    widgets::paint_row_text(ui.painter(), galley, left, rect.center().y, ink);
    let count = ui
        .painter()
        .layout_no_wrap(aurora_i18n::integer(count as i64), egui::FontId::proportional(SMALL), p.muted_dim);
    widgets::paint_row_text(ui.painter(), count.clone(), right + 8.0, rect.center().y, p.muted_dim);
    right += 8.0 + count.size().x + 10.0;
    if right < rect.right() - PAD {
        ui.painter()
            .hline(right..=rect.right() - PAD, rect.center().y, widgets::divider_stroke(p));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The titles of the details' columns above a list whose scroll area is
/// `list`. A click sorts by a column, a second reverses it (Windows
/// Explorer, Dolphin; Aurora difference: the Legacy Viewer sorts by name or
/// date only, from its menu); the arrow shows the order followed. A title
/// dragged sideways moves its column; a right-click opens the columns' menu.
#[allow(clippy::too_many_arguments)]
pub fn header(
    ui: &mut egui::Ui,
    p: &Palette,
    frame: Frame,
    list: egui::Id,
    prefs: &mut InventoryPreferences,
    tab: usize,
    fit: &mut ColumnFit,
) {
    use super::super::view::SortHeader;
    fit.start();
    let width = widgets::header_width(ui, list);
    let (rect, background) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    let area = Rect::from_min_size(rect.min, Vec2::new(width, rect.height()));
    // The titles go under the dividers' handles, which sit in the gaps.
    let mut shown = frame;
    shown.columns = prefs.explorer;
    let before = Columns::new(shown, area);
    let name = (None, SortHeader::Name, Rangef::new(area.left() + PAD, before.name_end));
    let titles: Vec<_> = std::iter::once(name)
        .chain(before.iter().map(|(id, range)| (Some(id), id.header(), range)))
        .collect();
    let (sorted, descending) = prefs.sort_for(tab).header();
    let mut clicked = None;
    let mut menu = background.secondary_clicked().then(|| background.interact_pointer_pos()).flatten();
    let mut moved = None;
    for (column, header, range) in &titles {
        let hit = Rect::from_x_y_ranges(*range, rect.y_range());
        let sense = if column.is_some() {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };
        let response = ui
            .interact(hit, ui.scope_id().with(("inventory_column_title", *header)), sense)
            .on_hover_text(aurora_i18n::tr!("inventory-sort-by-column"));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_column_title", *header)), hit));
        if response.clicked() {
            clicked = Some(*header);
        }
        if response.secondary_clicked() {
            menu = response.interact_pointer_pos();
        }
        let text = column.map_or(aurora_i18n::tr!("common-name"), |id| super::columns::title(id, shown));
        let active = *header == sorted;
        let ink = if active || response.hovered() || response.dragged() {
            p.ink
        } else {
            p.muted
        };
        let arrow = if active { 14.0 } else { 0.0 };
        if let Some(id) = column {
            let natural = ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.5), ink);
            fit.note(*id, natural.size().x + arrow);
        }
        let galley = cut(ui, text, 11.5, ink, (range.span() - arrow).max(0.0), false);
        let text_right = range.min + galley.size().x;
        widgets::paint_row_text(ui.painter(), galley, range.min, rect.center().y, ink);
        // The date's own order is the newest first: its arrow points down.
        let down = descending != (*header == SortHeader::Date);
        if active && let Some(texture) = crate::ui::icons::global(if down { "caret-down-filled" } else { "caret-up-filled" }) {
            ui.painter().image(
                texture.id(),
                Rect::from_center_size(egui::pos2(text_right + 8.0, rect.center().y), Vec2::splat(10.0)),
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                ink,
            );
        }
        // A title carried sideways: the line shows where its column goes.
        if let Some(id) = column
            && (response.dragged() || response.drag_stopped())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let others: Vec<(ColumnId, Rangef)> = before.iter().filter(|(other, _)| other != id).collect();
            let place = others.iter().filter(|(_, r)| r.center() < pointer.x).count();
            let x = match others.get(place) {
                Some((_, r)) => r.min - GAP * 0.5,
                None => others.last().map_or(before.name_end + GAP * 0.5, |(_, r)| r.max + GAP * 0.5),
            };
            if response.dragged() {
                ui.painter().rect_filled(hit, ui.visuals().widgets.inactive.corner_radius, p.raised);
                ui.painter().vline(
                    x,
                    rect.y_range().shrink(2.0),
                    widgets::border_stroke(p, egui::Stroke::new(2.0, p.violet_light)),
                );
            } else {
                // before the column it lands on, or after the last one shown
                let target = others.get(place).map(|(before, _)| *before);
                moved = Some((*id, target));
            }
        }
    }
    if let Some(header) = clicked {
        prefs.sort_for_mut(tab).click(header);
    }
    if let Some((id, target)) = moved {
        let columns = &mut prefs.explorer.columns;
        let index = match target {
            Some(target) => columns
                .0
                .iter()
                .filter(|s| s.id != id)
                .position(|s| s.id == target)
                .unwrap_or(COLUMNS - 1),
            None => {
                let last = before.iter().filter(|(other, _)| *other != id).last().map(|(last, _)| last);
                last.and_then(|last| columns.0.iter().filter(|s| s.id != id).position(|s| s.id == last))
                    .map_or(0, |i| i + 1)
            }
        };
        columns.move_to(id, index);
    }
    let _cols = super::columns::resize(ui, p, frame, area, &mut prefs.explorer, fit);
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique("inventory_header_columns"), _cols));
    // the columns' menu, kept where the right-click opened it
    let popup = list.with("inventory_header_menu");
    if let Some(pos) = menu {
        ui.ctx().data_mut(|data| data.insert_temp(popup, pos));
        egui::Popup::open_id(ui.ctx(), popup);
    }
    if let Some(pos) = ui.ctx().data(|data| data.get_temp::<Pos2>(popup)) {
        let measured = *fit;
        crate::ui::menu::context_menu_at(ui, popup, pos, p, |ui| {
            super::columns::options(ui, p, &mut prefs.explorer, &mut prefs.rights, &measured, frame.layout);
        });
        if !egui::Popup::is_id_open(ui.ctx(), popup) {
            ui.ctx().data_mut(|data| data.remove::<Pos2>(popup));
        }
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, widgets::divider_stroke(p));
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Where a row was drawn, for the tests that click it.
    pub(in super::super) fn record(ui: &egui::Ui, id: Uuid, rect: Rect) {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(("aurora_row", id)), rect));
    }

    pub(in super::super) fn drawn(ctx: &egui::Context, id: Uuid) -> Option<Rect> {
        ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique(("aurora_row", id))))
    }

    fn frame(layout: Layout, rights: Rights) -> Frame {
        Frame {
            layout,
            rights,
            columns: Default::default(),
            explorer: false,
        }
    }

    #[test]
    fn the_columns_appear_with_the_width_and_never_overlap() {
        let compact = Columns::new(
            frame(Layout::Compact, Rights::Mine),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, HEIGHT)),
        );
        assert_eq!(compact.get(ColumnId::Kind), None);
        assert_eq!(compact.get(ColumnId::Date), None);
        assert_eq!(compact.name_end, 400.0 - PAD);
        let medium = Columns::new(
            frame(Layout::Medium, Rights::Mine),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, HEIGHT)),
        );
        assert!(
            medium.get(ColumnId::Rights).is_some(),
            "expanded views have a resizable permissions column"
        );
        let (kind, date) = (medium.get(ColumnId::Kind).unwrap(), medium.get(ColumnId::Date).unwrap());
        assert!(medium.name_end < kind.min && kind.max < date.min && date.max <= 600.0);
        for rights in [Rights::Mine, Rights::Next, Rights::Both] {
            let wide = Columns::new(
                frame(Layout::Wide, rights),
                Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, HEIGHT)),
            );
            let (kind, range, date) = (
                wide.get(ColumnId::Kind).unwrap(),
                wide.get(ColumnId::Rights).unwrap(),
                wide.get(ColumnId::Date).unwrap(),
            );
            assert!(wide.name_end < kind.min && kind.max < range.min && range.max < date.min);
            assert!(range.span() >= rights::width(rights));
        }
        assert!(
            Columns::new(
                frame(Layout::Wide, Rights::Hidden),
                Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, HEIGHT))
            )
            .get(ColumnId::Rights)
            .is_none()
        );
    }

    #[test]
    fn rights_end_the_name_only_in_the_compact_view() {
        assert!(rights_in_name(frame(Layout::Compact, Rights::Mine)));
        assert!(!rights_in_name(frame(Layout::Medium, Rights::Both)));
        assert!(!rights_in_name(frame(Layout::Medium, Rights::Hidden)));
        assert!(!rights_in_name(frame(Layout::Wide, Rights::Mine)));
    }

    #[test]
    fn a_date_of_this_year_has_no_year() {
        // 2026-10-14 and 2025-09-03, read on 2026-10-19
        let now = 1_792_368_000;
        // the day and month in the language's order
        let date = crate::ui::land::day_month;
        assert_eq!(short_date(1_791_936_000, now), date(None, 10, 14));
        assert_eq!(short_date(1_756_857_600, now), date(Some(2025), 9, 3));
        assert_eq!(short_date(0, now), "");
    }

    #[test]
    fn a_worn_body_part_offers_nothing_to_take_off() {
        use aurora_net::inventory::InvItem;
        let mut inv = Inventory::default();
        for (n, asset_type) in [(1_u128, 13), (2, 5), (3, 7)] {
            inv.items.insert(
                Uuid::from_u128(n),
                InvItem {
                    id: Uuid::from_u128(n),
                    asset_type,
                    ..Default::default()
                },
            );
        }
        let mut facts = Facts::default();
        facts.worn.insert(Uuid::from_u128(1));
        facts.worn.insert(Uuid::from_u128(2));
        assert!(quick_buttons(&inv, Uuid::from_u128(1), &facts, Layout::Compact).is_empty());
        assert_eq!(
            quick_buttons(&inv, Uuid::from_u128(2), &facts, Layout::Compact),
            [Quick::Wear(true)]
        );
        assert_eq!(quick_buttons(&inv, Uuid::from_u128(3), &facts, Layout::Wide), [Quick::More]);
        facts.worn.clear();
        assert_eq!(
            quick_buttons(&inv, Uuid::from_u128(1), &facts, Layout::Wide),
            [Quick::Wear(false), Quick::More]
        );
        assert_eq!(quick_buttons(&inv, Uuid::from_u128(2), &facts, Layout::Compact), [Quick::Add]);
        assert_eq!(quick_label(Quick::Add), aurora_i18n::tr!("common-add"));
        inv.items.get_mut(&Uuid::from_u128(2)).unwrap().flags = 15;
        assert_eq!(
            quick_buttons(&inv, Uuid::from_u128(2), &facts, Layout::Compact),
            [Quick::Wear(false)]
        );
    }
}
