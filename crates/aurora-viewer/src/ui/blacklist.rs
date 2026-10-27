//! "Asset blacklist": the derendered objects and residents, temporary and
//! permanent, and the way back for them. Port of FSFloaterAssetBlacklist
//! (indra/newview/fsfloaterassetblacklist.cpp, floater_fs_asset_blacklist.xml,
//! menu_fs_asset_blacklist.xml) for the entries Aurora acts on: a filter,
//! the list with its name, region, type, date and permanent columns,
//! sortable, several rows selected with Ctrl / Shift, "Remove selected"
//! (also in the rows' right-click menu) and "Clear temporary". Sound Explorer
//! adds sound assets and owner categories to this same list. Textures and
//! animations remain preserved from imported files without being applied.

use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::derender::{Derender, Entry, Kind, RowId};
use egui::{Rect, RichText, Sense, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

/// The window's id (Floater.Toggle "fs_asset_blacklist").
pub const WINDOW: &str = "asset_blacklist";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Name,
    Uuid,
    Region,
    Type,
    Date,
    Permanent,
}

impl Column {
    const ALL: [Column; 6] = [
        Column::Name,
        Column::Uuid,
        Column::Region,
        Column::Type,
        Column::Date,
        Column::Permanent,
    ];

    fn title(self) -> &'static str {
        match self {
            Column::Name => aurora_i18n::tr!("common-name"),
            Column::Uuid => aurora_i18n::tr!("blacklist-col-uuid"),
            Column::Region => aurora_i18n::tr!("blacklist-col-region"),
            Column::Type => aurora_i18n::tr!("common-type"),
            Column::Date => aurora_i18n::tr!("blacklist-col-date"),
            Column::Permanent => aurora_i18n::tr!("blacklist-col-permanent"),
        }
    }

    /// Share of the row's width (the floater's column widths).
    fn weight(self) -> f32 {
        match self {
            Column::Name => 2.2,
            Column::Uuid => 0.0,
            Column::Region => 1.1,
            Column::Type => 0.0,
            Column::Date => 1.2,
            Column::Permanent => 0.8,
        }
    }
}

/// What the window asks the app for.
#[derive(Debug, Clone, PartialEq)]
pub enum BlacklistAction {
    OpenIgnored,
    /// FSAssetBlacklist::removeItemsFromBlacklist.
    Remove(Vec<RowId>),
    /// LLViewerObjectList::resetDerenderList(true).
    ClearTemporary,
}

#[derive(Default)]
pub struct BlacklistUi {
    filter: String,
    selected: HashSet<RowId>,
    /// The row Shift extends a selection from.
    anchor: Option<RowId>,
    /// Column sorted on and whether descending; None: newest first.
    sort: Option<(Column, bool)>,
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Object => aurora_i18n::tr!("common-object"),
        Kind::Resident => aurora_i18n::tr!("common-resident"),
        Kind::Sound(0) => aurora_i18n::tr!("sound-explorer-asset"),
        Kind::Sound(1) => aurora_i18n::tr!("sound-explorer-worn"),
        Kind::Sound(2) => aurora_i18n::tr!("sound-explorer-rezzed"),
        Kind::Sound(4) => aurora_i18n::tr!("sound-explorer-gestures"),
        Kind::Sound(_) => aurora_i18n::tr!("sound-explorer-owner-sounds"),
        Kind::Animation => aurora_i18n::tr!("inventory-kind-animation"),
    }
}

fn name_of(e: &Entry) -> &str {
    if e.name.is_empty() {
        aurora_i18n::tr!("blacklist-unknown-object")
    } else {
        &e.name
    }
}

fn region_of(e: &Entry) -> &str {
    match (e.kind, e.region.is_empty()) {
        (Kind::Object, true) => aurora_i18n::tr!("blacklist-unknown-region"),
        _ => &e.region,
    }
}

/// Date and time in SLT, like the other dates of the interface.
fn date_text(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let (y, m, d, _, h, mi, _) = crate::ui::land::slt_parts(secs);
    aurora_i18n::tr!(
        "inspect-date",
        year = format!("{y:04}"),
        month = format!("{m:02}"),
        day = format!("{d:02}"),
        hour = format!("{h:02}"),
        minute = format!("{mi:02}")
    )
}

fn row_kind(e: &Entry, id: RowId) -> Kind {
    match e.kind {
        Kind::Sound(_) if id.flag != 0 => Kind::Sound(id.flag),
        kind => kind,
    }
}

fn cell_text(e: &Entry, column: Column, id: RowId) -> String {
    match column {
        Column::Name => name_of(e).to_owned(),
        Column::Uuid => id.id.to_string(),
        Column::Region => region_of(e).to_owned(),
        Column::Type => kind_label(row_kind(e, id)).to_owned(),
        Column::Date => date_text(e.date),
        // asset_permanent: a check mark, drawn as an icon
        Column::Permanent => String::new(),
    }
}

/// The Legacy Viewer's name filter, extended to UUIDs so sound assets can
/// be found independently of the resident name saved with them.
fn rows<'a>(list: impl Iterator<Item = (&'a Uuid, &'a Entry)>, filter: &str, sort: Option<(Column, bool)>) -> Vec<(RowId, &'a Entry)> {
    let filter = filter.trim().to_lowercase();
    let mut rows: Vec<(RowId, &Entry)> = list
        .filter(|(id, e)| filter.is_empty() || name_of(e).to_lowercase().contains(&filter) || id.to_string().contains(&filter))
        .flat_map(|(id, e)| {
            let flags = match e.kind {
                Kind::Sound(flags) => flags,
                _ => 0,
            };
            [0, 1, 2, 4]
                .into_iter()
                .filter(move |flag| if flags == 0 { *flag == 0 } else { *flag != 0 && flags & flag != 0 })
                .map(move |flag| (RowId { id: *id, flag }, e))
        })
        .collect();
    let text = |a: &str, b: &str| a.to_lowercase().cmp(&b.to_lowercase());
    rows.sort_by(|(ia, a), (ib, b)| {
        let order = match sort {
            None => b.date.cmp(&a.date),
            Some((column, descending)) => {
                let o = match column {
                    Column::Name => text(name_of(a), name_of(b)),
                    Column::Uuid => ia.id.cmp(&ib.id),
                    Column::Region => text(region_of(a), region_of(b)),
                    Column::Type => text(kind_label(row_kind(a, *ia)), kind_label(row_kind(b, *ib))),
                    Column::Date => a.date.cmp(&b.date),
                    Column::Permanent => a.permanent.cmp(&b.permanent),
                };
                if descending { o.reverse() } else { o }
            }
        };
        // a stable order for equal rows
        order.then_with(|| ia.cmp(ib))
    });
    rows
}

fn cells(row: Rect) -> Vec<(Column, Rect)> {
    let total: f32 = Column::ALL.iter().map(|c| c.weight()).sum();
    // Keep all 36 characters legible even in a narrow, previously saved window.
    let uuid_width = 288.0f32.min(row.width());
    let type_width = 172.0f32.min(row.width() - uuid_width);
    let unit = (row.width() - uuid_width - type_width) / total;
    let mut x = row.left();
    Column::ALL
        .into_iter()
        .map(|c| {
            let w = match c {
                Column::Uuid => uuid_width,
                Column::Type => type_width,
                _ => unit * c.weight(),
            };
            let cell = Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(x + w, row.bottom()));
            x += w;
            (c, cell)
        })
        .collect()
}

impl BlacklistUi {
    fn header(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let width = widgets::header_width(ui, ui.scope_id().with("blacklist_rows"));
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
        for (column, cell) in cells(rect.with_max_x(rect.left() + width)) {
            let resp = ui.interact(cell, ui.scope_id().with(("blacklist_header", column as u8)), Sense::click());
            let sorted = self.sort.filter(|(c, _)| *c == column);
            let color = if sorted.is_some() || resp.hovered() { p.ink } else { p.muted };
            let room = cell.width() - if sorted.is_some() { 22.0 } else { 12.0 };
            let left = egui::pos2(cell.left() + 8.0, cell.center().y);
            widgets::paint_elided(ui, column.title(), left, room, egui::FontId::proportional(12.0), color);
            if let Some((_, descending)) = sorted
                && let Some(texture) = crate::ui::icons::global(if descending { "caret-down-filled" } else { "caret-up-filled" })
            {
                let at = Rect::from_center_size(egui::pos2(cell.right() - 8.0, cell.center().y), Vec2::splat(8.0));
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(texture.id(), at, uv, p.violet_light);
            }
            if resp.clicked() {
                self.sort = match sorted {
                    Some((_, false)) => Some((column, true)),
                    Some((_, true)) => None,
                    None => Some((column, false)),
                };
            }
        }
    }

    /// A click on a row: alone, Ctrl to add or take it out, Shift for the
    /// rows from the last one clicked.
    fn click(&mut self, id: RowId, order: &[RowId], modifiers: egui::Modifiers) {
        if modifiers.shift
            && let Some(anchor) = self.anchor
            && let (Some(a), Some(b)) = (order.iter().position(|i| *i == anchor), order.iter().position(|i| *i == id))
        {
            if !modifiers.command {
                self.selected.clear();
            }
            self.selected.extend(&order[a.min(b)..=a.max(b)]);
            return;
        }
        if modifiers.command {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
        } else {
            self.selected.clear();
            self.selected.insert(id);
        }
        self.anchor = Some(id);
    }

    /// Draw the window while `open`.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, list: &Derender, open: &mut bool) -> Option<BlacklistAction> {
        if !*open {
            return None;
        }
        let mut action = None;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("blacklist-title"),
            egui::pos2(260.0, 140.0),
            Vec2::new(1050.0, 320.0),
        );
        floater.min_size = Vec2::new(680.0, 240.0);
        floater.show(ctx, p, open, |ui| {
            const BUTTONS: f32 = 26.0;
            let w = ui.available_width();
            ui.allocate_ui_with_layout(Vec2::new(w, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                widgets::search_box(ui, p, &mut self.filter, aurora_i18n::tr!("blacklist-filter"), w);
            });
            ui.add_space(6.0);
            let shown = rows(list.entries(), &self.filter, self.sort);
            let order: Vec<RowId> = shown.iter().map(|(id, _)| *id).collect();
            self.selected.retain(|id| list.has_row(*id));
            let footer = BUTTONS + 6.0 + ui.spacing().item_spacing.y;
            let height = (ui.available_height() - footer).max(0.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_height(height);
                    self.header(ui, p);
                    let out = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        if shown.is_empty() {
                            ui.add_space(8.0);
                            ui.label(RichText::new(aurora_i18n::tr!("blacklist-empty")).size(12.0).color(p.muted));
                        }
                        let radius = ui.visuals().window_corner_radius;
                        for (id, entry) in &shown {
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let selected = self.selected.contains(id);
                            if selected {
                                ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                            }
                            for (column, cell) in cells(rect) {
                                if column == Column::Permanent {
                                    if entry.permanent
                                        && let Some(texture) = crate::ui::icons::global("check")
                                    {
                                        let at = Rect::from_center_size(egui::pos2(cell.left() + 16.0, cell.center().y), Vec2::splat(14.0));
                                        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                                        ui.painter().image(texture.id(), at, uv, p.ink);
                                    }
                                    continue;
                                }
                                let text = cell_text(entry, column, *id);
                                let left = egui::pos2(cell.left() + 8.0, cell.center().y);
                                let color = if column == Column::Name { p.ink } else { p.muted };
                                let font = if column == Column::Uuid {
                                    egui::FontId::monospace(12.0)
                                } else {
                                    egui::FontId::proportional(12.5)
                                };
                                widgets::paint_elided(ui, &text, left, cell.width() - 12.0, font, color);
                            }
                            resp.clone()
                                .on_hover_text(format!("{}\n{}\n{}", name_of(entry), cell_text(entry, Column::Type, *id), id.id));
                            if resp.clicked() {
                                let modifiers = ui.input(|i| i.modifiers);
                                self.click(*id, &order, modifiers);
                            }
                            // menu_fs_asset_blacklist.xml: "Remove" on the
                            // selection, the row right-clicked joining it
                            if resp.secondary_clicked() && !selected {
                                self.selected.clear();
                                self.selected.insert(*id);
                                self.anchor = Some(*id);
                            }
                            super::menu::context_menu(&resp, p, |ui| {
                                if super::menu::item(ui, p, "copy-filled", aurora_i18n::tr!("build-ui-general-copy-uuid")) {
                                    let mut seen = HashSet::new();
                                    let ids: Vec<_> = order
                                        .iter()
                                        .filter(|id| self.selected.contains(id) && seen.insert(id.id))
                                        .map(|id| id.id.to_string())
                                        .collect();
                                    ui.ctx().copy_text(ids.join("\n"));
                                    ui.close();
                                }
                                if super::menu::item(ui, p, "trash-filled", aurora_i18n::tr!("common-remove")) {
                                    action = Some(BlacklistAction::Remove(self.selected.iter().copied().collect()));
                                    ui.close();
                                }
                            });
                        }
                    });
                    widgets::remember_scroll_bar(ui, ui.scope_id().with("blacklist_rows"), &out);
                },
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.set_height(BUTTONS);
                let size = Vec2::new(0.0, BUTTONS);
                let remove = widgets::action_button_if(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("blacklist-remove-selected"),
                    ActionKind::Flat,
                    size,
                    !self.selected.is_empty(),
                );
                if remove.clicked() {
                    action = Some(BlacklistAction::Remove(self.selected.iter().copied().collect()));
                }
                let temporary = list.entries().any(|(_, e)| !e.permanent);
                let clear = widgets::action_button_if(
                    ui,
                    p,
                    "eye-filled",
                    aurora_i18n::tr!("blacklist-clear-temporary"),
                    ActionKind::Flat,
                    size,
                    temporary,
                );
                if clear.on_hover_text(aurora_i18n::tr!("blacklist-clear-temporary-tip")).clicked() {
                    action = Some(BlacklistAction::ClearTemporary);
                }
                if widgets::action_button(ui, p, "ban", aurora_i18n::tr!("blacklist-ignored"), ActionKind::Flat, size)
                    .on_hover_text(aurora_i18n::tr!("blacklist-ignored-tip"))
                    .clicked()
                {
                    action = Some(BlacklistAction::OpenIgnored);
                }
            });
        });
        if let Some(BlacklistAction::Remove(ids)) = &action {
            for id in ids {
                self.selected.remove(id);
            }
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn entry(name: &str, date: i64, permanent: bool) -> Entry {
        Entry {
            name: name.into(),
            region: String::new(),
            kind: Kind::Object,
            date,
            permanent,
        }
    }

    #[test]
    fn rows_are_filtered_on_the_name_and_newest_first_until_sorted() {
        let list: HashMap<Uuid, Entry> = [
            (Uuid::from_u128(1), entry("Lantern", 10, true)),
            (Uuid::from_u128(2), entry("barrel", 30, false)),
            (Uuid::from_u128(3), entry("Big lantern", 20, false)),
        ]
        .into_iter()
        .collect();
        let names = |filter: &str, sort| {
            rows(list.iter(), filter, sort)
                .into_iter()
                .map(|(_, e)| e.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("", None), ["barrel", "Big lantern", "Lantern"]);
        assert_eq!(names("LANTERN", None), ["Big lantern", "Lantern"]);
        assert_eq!(names("", Some((Column::Name, false))), ["barrel", "Big lantern", "Lantern"]);
        assert_eq!(names("", Some((Column::Permanent, true))), ["Lantern", "barrel", "Big lantern"]);
        // an object without a name, and without a region
        let unnamed = entry("", 0, false);
        let id = RowId::whole(Uuid::nil());
        assert_eq!(cell_text(&unnamed, Column::Name, id), aurora_i18n::tr!("blacklist-unknown-object"));
        assert_eq!(
            cell_text(&unnamed, Column::Region, id),
            aurora_i18n::tr!("blacklist-unknown-region")
        );
        assert_eq!(cell_text(&unnamed, Column::Date, id), "");
    }

    #[test]
    fn blacklist_uuid_search_and_sort_use_the_asset_key_independently_of_its_saved_name() {
        let a = Uuid::parse_str("fedcba98-7654-4321-9abc-0123456789ab").unwrap();
        let b = Uuid::parse_str("01234567-89ab-4cde-8fed-cba987654321").unwrap();
        let sound = || Entry {
            kind: Kind::Sound(0),
            ..entry("Same demo owner", 10, true)
        };
        let list = HashMap::from([(a, sound()), (b, sound())]);
        let selected = rows(list.iter(), "FEDCBA98-7654", None);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.id, a);
        assert_eq!(cell_text(selected[0].1, Column::Uuid, selected[0].0), a.to_string());
        let sorted = rows(list.iter(), "demo owner", Some((Column::Uuid, false)));
        assert_eq!(sorted.iter().map(|(id, _)| id.id).collect::<Vec<_>>(), [b, a]);
    }

    #[test]
    fn the_window_keeps_its_size() {
        let mut list = Derender::default();
        for i in 0..40 {
            list.add(Uuid::from_u128(i), entry(&format!("Object {i}"), i as i64, false));
        }
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(1050.0, 320.0), Vec2::new(680.0, 240.0)],
            aurora_i18n::tr!("blacklist-clear-temporary"),
            BlacklistUi::default,
            |ui, ctx, p| {
                ui.show(ctx, p, &list, &mut true);
            },
        );
    }

    #[test]
    fn ctrl_and_shift_choose_several_rows() {
        let order: Vec<RowId> = (1..=4).map(Uuid::from_u128).map(RowId::whole).collect();
        let mut ui = BlacklistUi::default();
        let none = egui::Modifiers::default();
        let ctrl = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        ui.click(order[0], &order, none);
        ui.click(order[2], &order, shift);
        assert_eq!(ui.selected, order[..3].iter().copied().collect::<HashSet<_>>());
        ui.click(order[1], &order, ctrl);
        assert_eq!(ui.selected, [order[0], order[2]].into_iter().collect::<HashSet<_>>());
        ui.click(order[3], &order, none);
        assert_eq!(ui.selected, [order[3]].into_iter().collect::<HashSet<_>>());
    }

    #[test]
    fn blacklist_sound_categories_have_independent_rows_selection_and_labels() {
        let owner = Uuid::from_u128(45);
        let mut e = entry("Demo resident", 10, true);
        e.kind = Kind::Sound(7);
        let list = HashMap::from([(owner, e)]);
        let shown = rows(list.iter(), "DEMO", None);
        let order: Vec<_> = shown.iter().map(|(id, _)| *id).collect();
        assert_eq!(order, [1, 2, 4].map(|flag| RowId { id: owner, flag }));
        let labels: Vec<_> = shown.iter().map(|(id, e)| cell_text(e, Column::Type, *id)).collect();
        assert_eq!(
            labels,
            [
                aurora_i18n::tr!("sound-explorer-worn"),
                aurora_i18n::tr!("sound-explorer-rezzed"),
                aurora_i18n::tr!("sound-explorer-gestures")
            ]
        );
        let mut ui = BlacklistUi::default();
        ui.click(order[0], &order, Default::default());
        ui.click(
            order[2],
            &order,
            egui::Modifiers {
                command: true,
                ..Default::default()
            },
        );
        assert_eq!(ui.selected, HashSet::from([order[0], order[2]]));
        assert!(
            !ui.selected.contains(&order[1]),
            "the shared resident UUID is not the selection identity"
        );
    }
}

#[cfg(test)]
#[path = "blacklist_sound_tests.rs"]
mod sound_tests;
