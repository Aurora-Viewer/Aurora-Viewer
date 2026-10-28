//! The "Replace links" window. Port of LLFloaterLinkReplace
//! (indra/newview/llfloaterlinkreplace.cpp, originally LGPL 2.1): the new
//! target is dropped from an inventory window on a field, as on
//! LLInventoryLinkReplaceDropTarget, a link standing for its original. The
//! filtered list of the items of the same kind is an Aurora addition, kept
//! from the first version of the window. It is built once per inventory
//! change, in parallel, and only the rows in sight are laid out: filtering,
//! sorting and drawing every item at every frame held a resident with a few
//! hundred thousand items under 2 frames per second while it was open (#1297).

use super::*;
use crate::ui::widgets::{Floater, above_footer, flat_button};
use aurora_net::inventory::InvItem;
use aurora_net::inventory::operations::Operation;
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const ROW: f32 = 40.0;
/// While the whole inventory loads, its generation changes with every
/// folder received: the list follows at this pace, not at every frame.
const LOADING_REBUILD: Duration = Duration::from_millis(1_000);

struct Candidate {
    id: Uuid,
    name: String,
    path: String,
    search: String,
    asset_type: i32,
    inv_type: i32,
    flags: u32,
}

pub struct ReplaceLinks {
    /// The item the links point to.
    pub(super) old: Uuid,
    pub(super) selected: Option<Uuid>,
    pub(super) filter: String,
    focus: bool,
    generation: Option<(u64, &'static str, view::Sort)>,
    protected: HashSet<Uuid>,
    built: Option<Instant>,
    candidates: Vec<Candidate>,
    visible: Vec<usize>,
    query: Option<String>,
    /// The links the window would replace (LLFloaterLinkReplace's
    /// "ItemsFound").
    links: usize,
    refused: bool,
}

impl ReplaceLinks {
    pub(super) fn new(old: Uuid) -> Self {
        Self {
            old,
            selected: None,
            filter: String::new(),
            focus: true,
            generation: None,
            protected: HashSet::new(),
            built: None,
            candidates: Vec::new(),
            visible: Vec::new(),
            query: None,
            links: 0,
            refused: false,
        }
    }

    fn update(&mut self, ctx: &egui::Context, inv: &Inventory, protected: &HashSet<Uuid>, loading: bool) {
        let generation = (inv.generation, aurora_i18n::language(), view::picker_sort(ctx));
        let stale = self.generation != Some(generation) || self.protected != *protected;
        if stale && (!loading || self.built.is_none_or(|built| built.elapsed() >= LOADING_REBUILD)) {
            self.candidates = candidates(inv, self.old);
            let order = view::picker_order(ctx, inv);
            self.candidates.par_sort_unstable_by_key(|row| order.rank(row.id));
            self.links = links(inv, self.old, protected).len();
            self.generation = Some(generation);
            self.protected.clone_from(protected);
            self.built = Some(Instant::now());
            self.query = None;
            // a target deleted, moved to the Trash or changed meanwhile
            if self.selected.is_some_and(|id| target(inv, self.old, id) != Some(id)) {
                self.selected = None;
            }
        }
        let query = self.filter.trim().to_lowercase();
        if self.query.as_ref() != Some(&query) {
            self.visible = self
                .candidates
                .par_iter()
                .enumerate()
                .filter_map(|(i, row)| row.search.contains(&query).then_some(i))
                .collect();
            self.query = Some(query);
        }
    }
}

/// The asset type, and for clothing and body parts the wearable type, that
/// a new target must share with the old one.
fn same_kind(old: &InvItem, it: &InvItem) -> bool {
    it.asset_type == old.asset_type && (!matches!(old.asset_type, 5 | 13) || it.flags & 0xff == old.flags & 0xff)
}

/// The item that may replace the links to `old` when `id` is chosen or
/// dropped: `id` itself, or its original for a link (getLinkedUUID). Never
/// `old`, a broken link, an item in the Library or in the Trash.
pub(super) fn target(inv: &Inventory, old: Uuid, id: Uuid) -> Option<Uuid> {
    let id = rules::original(inv, id)?;
    let (old_item, it) = (inv.items.get(&old)?, inv.items.get(&id)?);
    (id != old && !matches!(it.asset_type, 24 | 25) && same_kind(old_item, it) && !rules::library(inv, id) && !rules::in_type(inv, id, 14))
        .then_some(id)
}

/// The path of `folder` from the root, or None when it lies in the Trash or
/// in the Library. A folder whose parents are unknown keeps the part known.
fn folder_path(inv: &Inventory, folder: Uuid, trash: Option<Uuid>) -> Option<String> {
    let mut parts = Vec::new();
    let mut current = folder;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let Some(f) = inv.folders.get(&current) else { break };
        if f.library || Some(current) == trash {
            return None;
        }
        parts.push(folder_name(inv, current));
        if current == inv.root {
            break;
        }
        current = f.info.parent;
    }
    parts.reverse();
    Some(parts.join(" / "))
}

/// Every item `target` accepts for `old`. The Trash and the Library are
/// resolved once per folder, not once per item: rules::in_type looks the
/// Trash up among all the folders at each call.
fn candidates(inv: &Inventory, old: Uuid) -> Vec<Candidate> {
    let Some(old_item) = inv.items.get(&old) else {
        return Vec::new();
    };
    let trash = rules::system(inv, 14);
    let paths: HashMap<Uuid, Option<String>> = inv.folders.par_iter().map(|(&id, _)| (id, folder_path(inv, id, trash))).collect();
    inv.items
        .par_iter()
        .filter(|(_, it)| it.id != old && !matches!(it.asset_type, 24 | 25) && same_kind(old_item, it))
        .filter_map(|(_, it)| {
            // an item outside any known folder stays, as for `target`
            let path = paths.get(&it.parent).cloned().unwrap_or_else(|| Some(String::new()))?;
            Some(Candidate {
                id: it.id,
                search: it.name.to_lowercase(),
                name: it.name.clone(),
                path,
                asset_type: it.asset_type,
                inv_type: it.inv_type,
                flags: it.flags,
            })
        })
        .collect()
}

/// The links to `old` the window replaces: outside the Trash, and those of
/// the outfits even where the inventory is otherwise protected.
fn links<'a>(inv: &'a Inventory, old: Uuid, protected: &HashSet<Uuid>) -> Vec<&'a InvItem> {
    inv.items
        .values()
        .filter(|it| {
            it.asset_type == 24
                && it.asset_id == old
                && !rules::in_type(inv, it.id, 14)
                && (rules::mutable(inv, it.id, protected) || rules::in_type(inv, it.id, 46))
        })
        .collect()
}

/// A new link to `new` beside each link to `old`, then the old link deleted
/// (LLFloaterLinkReplace::processBatch and linkCreatedCallback). In an
/// outfit, the link keeps the clothing order in its description, and the
/// other kinds take none, so that the outfit is not dirty when worn.
fn plan(inv: &Inventory, old: Uuid, new: Uuid, protected: &HashSet<Uuid>) -> Mutation {
    let mut change = Mutation::default();
    let Some(replacement) = inv.items.get(&new) else {
        return change;
    };
    for link in links(inv, old, protected) {
        let outfit = rules::in_type(inv, link.id, 47) || rules::in_type(inv, link.id, 46);
        let desc = if outfit {
            if replacement.asset_type == 5 {
                link.desc.clone()
            } else {
                String::new()
            }
        } else {
            replacement.desc.clone()
        };
        change.operations.push(Operation::Links {
            parent: link.parent,
            links: vec![aurora_net::outfits::OutfitLink {
                target: new,
                name: replacement.name.clone(),
                desc,
                folder: false,
                inv_type: replacement.inv_type,
            }],
        });
        change.operations.push(Operation::Delete {
            id: link.id,
            folder: false,
        });
        change.removed.push(link.id);
        change.refresh.push(link.parent);
    }
    change
}

/// An item's icon, name and path in `rect`.
#[allow(clippy::too_many_arguments)]
fn paint_item(ui: &egui::Ui, p: &Palette, icons: &Icons, rect: egui::Rect, icon: &str, tint: egui::Color32, name: &str, path: &str) {
    if let Some(icon) = icons.get(icon) {
        ui.painter().image(
            icon.id(),
            egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 5.0), Vec2::splat(16.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    for (text, offset, size, color) in [
        (name, egui::vec2(28.0, 3.0), 13.0, p.ink),
        (path, egui::vec2(28.0, 22.0), 11.0, p.muted),
    ] {
        let galley = egui::WidgetText::from(RichText::new(text).size(size).color(color)).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (rect.width() - 34.0).max(0.0),
            egui::TextStyle::Body,
        );
        ui.painter().galley(rect.min + offset, galley, color);
    }
}

/// The window; false once closed, done or cancelled.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    picker: &mut ReplaceLinks,
    actions: &mut Vec<InvAction>,
) -> bool {
    view::set_picker_sort(ctx, prefs.sort);
    picker.update(ctx, inv, &prefs.protected, st.fetch_all);
    let mut open = true;
    let mut done = false;
    let window_id = format!("inventory_replace_links_{}", st.ui_id);
    Floater::new(
        &window_id,
        aurora_i18n::tr!("common-replace-links"),
        ctx.content_rect().center() - egui::vec2(240.0, 230.0),
        Vec2::new(480.0, 460.0),
    )
    .show(ctx, p, &mut open, |ui| {
        let old = rules::name(inv, picker.old).to_string();
        ui.add(egui::Label::new(aurora_i18n::tr!("ui-inventory-dialogs-replace-links-to-with", value0 = old)).truncate());

        // the drop field of the Legacy Viewer, which shows the chosen target
        let (zone, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), egui::Sense::hover());
        let hovered = response
            .dnd_hover_payload::<InvDrag>()
            .map(|drag| target(inv, picker.old, drag.0).is_some());
        let stroke = match hovered {
            Some(true) => p.violet_light,
            Some(false) => p.danger,
            None => p.raised,
        };
        ui.painter().rect_filled(zone, ui.visuals().window_corner_radius, p.field);
        ui.painter().rect_stroke(
            zone,
            ui.visuals().window_corner_radius,
            egui::Stroke::new(1.5, stroke),
            egui::StrokeKind::Inside,
        );
        if let Some(drag) = response.dnd_release_payload::<InvDrag>() {
            match target(inv, picker.old, drag.0) {
                Some(id) => {
                    picker.selected = Some(id);
                    picker.refused = false;
                }
                None => picker.refused = true,
            }
        }
        #[cfg(test)]
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_replace_links_drop", st.ui_id)), zone));
        match picker.selected.and_then(|id| inv.items.get(&id)) {
            Some(it) => {
                let path = folder_path(inv, it.parent, None).unwrap_or_default();
                let tint = item_icon_color(it.asset_type, it.inv_type, p);
                paint_item(
                    ui,
                    p,
                    icons,
                    zone,
                    item_icon(it.asset_type, it.inv_type, it.flags),
                    tint,
                    &it.name,
                    &path,
                );
            }
            None => {
                let galley = egui::WidgetText::from(
                    RichText::new(aurora_i18n::tr!("ui-inventory-dialogs-replace-links-drop"))
                        .size(12.0)
                        .color(p.muted),
                )
                .into_galley(ui, Some(egui::TextWrapMode::Truncate), zone.width() - 16.0, egui::TextStyle::Body);
                ui.painter().galley(
                    egui::pos2(zone.min.x + 8.0, zone.center().y - galley.size().y * 0.5),
                    galley,
                    p.muted,
                );
            }
        }

        let width = ui.available_width();
        let clear_shown = !picker.filter.is_empty();
        let (filter, cleared) = super::tree::search_field(
            ui,
            p,
            &mut picker.filter,
            aurora_i18n::tr!("ui-inventory-dialogs-replace-links-filter"),
            width,
            clear_shown,
            aurora_i18n::tr!("common-clear"),
        );
        if cleared {
            picker.filter.clear();
        }
        if picker.focus {
            filter.request_focus();
            picker.focus = false;
        }
        picker.update(ctx, inv, &prefs.protected, st.fetch_all);
        #[cfg(test)]
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_replace_links_filter", st.ui_id)), filter.rect));

        let (mut list, foot) = above_footer(ui);
        let list_height = list.available_height();
        egui::Frame::new().fill(p.field).show(&mut list, |ui| {
            ui.set_min_height(list_height);
            if picker.visible.is_empty() && !st.fetch_all {
                ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-dialogs-replace-links-none")).color(p.muted));
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt(("inventory_replace_links_items", st.ui_id))
                .max_height(ui.available_height())
                .auto_shrink([false, false])
                .show_rows(ui, ROW, picker.visible.len(), |ui, range| {
                    for i in range {
                        let row = &picker.candidates[picker.visible[i]];
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), egui::Sense::click());
                        if picker.selected == Some(row.id) {
                            ui.painter()
                                .rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.35));
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
                        }
                        let tint = item_icon_color(row.asset_type, row.inv_type, p);
                        let icon = item_icon(row.asset_type, row.inv_type, row.flags);
                        paint_item(ui, p, icons, rect, icon, tint, &row.name, &row.path);
                        #[cfg(test)]
                        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_replace_links_row", st.ui_id, row.id)), rect));
                        if response.on_hover_text(&row.path).clicked() {
                            picker.selected = Some(row.id);
                            picker.refused = false;
                        }
                    }
                });
        });
        foot.show(ui, |ui| {
            let status = if st.fetch_all {
                aurora_i18n::tr!("ui-inventory-dialogs-loading-all-folders-before-replacing").to_owned()
            } else {
                format!(
                    "{} · {}",
                    aurora_i18n::tr!(
                        "inventory-items-count",
                        count = picker.visible.len(),
                        number = aurora_i18n::integer(picker.visible.len() as i64)
                    ),
                    aurora_i18n::tr!(
                        "ui-inventory-dialogs-replace-links-found",
                        count = picker.links,
                        number = aurora_i18n::integer(picker.links as i64)
                    )
                )
            };
            ui.label(RichText::new(status).size(11.0).color(p.muted));
            ui.horizontal(|ui| {
                let enabled = picker.selected.is_some() && st.pending.is_none() && !st.fetch_all && !facts.appearance_busy;
                ui.add_enabled_ui(enabled, |ui| {
                    let button = flat_button(ui, p, aurora_i18n::tr!("ui-inventory-dialogs-replace"));
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_replace_links_confirm", st.ui_id)), button.rect));
                    if button.clicked()
                        && enabled
                        && let Some(new) = picker.selected
                    {
                        let change = plan(inv, picker.old, new, &prefs.protected);
                        if change.operations.is_empty() {
                            st.message = aurora_i18n::tr!("ui-inventory-dialogs-no-editable-links-in-the-loaded-folders").into();
                        } else {
                            actions.push(InvAction::Edit(change));
                        }
                        done = true;
                    }
                });
                done |= flat_button(ui, p, aurora_i18n::tr!("extra-close-cancel")).clicked();
            });
            if picker.refused {
                ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-dialogs-replace-links-refused")).color(p.warn));
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }
    });
    open && !done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// The big demo inventory, with its root, a Trash and a Library.
    fn fixture(items: usize) -> Inventory {
        use crate::world::inventory::Folder;
        use aurora_net::inventory::InvFolder;
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(3),
            ..Default::default()
        };
        for (folder, parent, kind, library) in [
            (id(1), Uuid::nil(), 8, false),
            (id(2), id(1), 14, false),
            (id(3), Uuid::nil(), 8, true),
        ] {
            inv.folders.insert(
                folder,
                Folder {
                    info: InvFolder {
                        id: folder,
                        parent,
                        type_default: kind,
                        name: format!("Folder {}", folder.as_u128()),
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library,
                },
            );
        }
        crate::world::inventory::demo::seed_big(&mut inv, id(9), items);
        // one object of the same kind in the Trash and one in the Library
        for (item, parent) in [(id(40), id(2)), (id(41), id(3))] {
            let mut it = inv.items[&crate::world::inventory::demo::BIG_LINKED].clone();
            it.id = item;
            it.parent = parent;
            inv.add_items(vec![it]);
        }
        inv
    }

    #[test]
    fn the_list_holds_exactly_the_items_a_drop_accepts() {
        let inv = fixture(2_000);
        let old = crate::world::inventory::demo::BIG_LINKED;
        let listed: HashSet<_> = candidates(&inv, old).into_iter().map(|row| row.id).collect();
        let accepted: HashSet<_> = inv
            .items
            .keys()
            .filter(|it| target(&inv, old, **it) == Some(**it))
            .copied()
            .collect();
        assert_eq!(listed, accepted);
        assert!(!listed.is_empty());
        assert!(!listed.contains(&old), "never the item the links point to");
        assert!(!listed.contains(&id(40)) && !listed.contains(&id(41)), "no Trash, no Library");
        assert!(listed.iter().all(|it| inv.items[it].asset_type == 6));
    }

    #[test]
    fn a_dropped_link_stands_for_its_original() {
        let inv = fixture(2_000);
        let old = crate::world::inventory::demo::BIG_LINKED;
        // item 99 links to item 50, another object
        let link = id(21_000_099);
        assert_eq!(inv.items[&link].asset_type, 24);
        assert_eq!(target(&inv, old, link), Some(id(21_000_050)));
        // a link to `old` itself, or an item of another kind, is refused
        assert_eq!(target(&inv, old, id(21_000_049)), None);
        assert_eq!(target(&inv, old, id(21_000_020)), None);
    }

    #[test]
    fn the_list_is_built_once_per_inventory_change_and_filtered_without_rebuilding() {
        let ctx = egui::Context::default();
        let mut inv = fixture(2_000);
        let mut picker = ReplaceLinks::new(crate::world::inventory::demo::BIG_LINKED);
        picker.update(&ctx, &inv, &HashSet::new(), false);
        let built = picker.built.expect("built");
        let all = picker.visible.len();
        assert_eq!(all, picker.candidates.len());
        assert!(picker.links > 0);
        picker.filter = "  JACKET 0".into();
        picker.update(&ctx, &inv, &HashSet::new(), false);
        assert_eq!(picker.built, Some(built), "a filter does not rebuild the list");
        assert!(!picker.visible.is_empty() && picker.visible.len() < all);
        assert!(
            picker
                .visible
                .iter()
                .all(|i| picker.candidates[*i].name.to_lowercase().contains("jacket 0"))
        );
        inv.generation += 1;
        picker.update(&ctx, &inv, &HashSet::new(), true);
        assert_eq!(picker.built, Some(built), "while loading, at most once a second");
        picker.update(&ctx, &inv, &HashSet::new(), false);
        assert_ne!(picker.built, Some(built), "a change rebuilds the list once loaded");
    }

    #[test]
    fn a_chosen_target_moved_to_the_trash_is_dropped() {
        let ctx = egui::Context::default();
        let mut inv = fixture(2_000);
        let mut picker = ReplaceLinks::new(crate::world::inventory::demo::BIG_LINKED);
        picker.update(&ctx, &inv, &HashSet::new(), false);
        let chosen = picker.candidates[0].id;
        picker.selected = Some(chosen);
        inv.items.get_mut(&chosen).expect("item").parent = id(2);
        inv.generation += 1;
        picker.update(&ctx, &inv, &HashSet::new(), false);
        assert_eq!(picker.selected, None);
        assert!(!picker.candidates.iter().any(|row| row.id == chosen));
    }

    #[test]
    fn replacing_links_each_link_to_the_old_item_once() {
        let inv = fixture(2_000);
        let old = crate::world::inventory::demo::BIG_LINKED;
        let new = id(21_000_001);
        let change = plan(&inv, old, new, &HashSet::new());
        let expected: Vec<_> = inv
            .items
            .values()
            .filter(|it| it.asset_type == 24 && it.asset_id == old)
            .map(|it| it.id)
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(change.removed.len(), expected.len());
        assert!(expected.iter().all(|link| change.removed.contains(link)));
        assert!(change.operations.iter().any(|operation| matches!(operation,
            Operation::Links { links, .. } if links.len() == 1 && links[0].target == new)));
        assert!(plan(&inv, old, id(999), &HashSet::new()).operations.is_empty(), "a vanished target");
    }

    /// The cost #1297 measured: a frame of the window over a big inventory.
    /// Its first frame builds the list; the next ones must not walk it again.
    #[test]
    fn a_frame_over_a_big_inventory_lays_out_only_the_rows_in_sight() {
        let ctx = egui::Context::default();
        let inv = fixture(60_000);
        let mut st = InventoryUi::default();
        st.begin_replace_links(crate::world::inventory::demo::BIG_LINKED);
        st.fetch_all = false;
        let prefs = InventoryPreferences::default();
        let facts = Facts::default();
        let frame = |st: &mut InventoryUi| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    super::super::dialogs::show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &inv,
                        st,
                        &prefs,
                        &facts,
                        &Default::default(),
                        &mut Vec::new(),
                    );
                },
            );
            let texts = output
                .shapes
                .iter()
                .filter(|shape| matches!(shape.shape, egui::epaint::Shape::Text(_)))
                .count();
            drop(super::super::headless_output(output));
            texts
        };
        for _ in 0..3 {
            frame(&mut st);
        }
        let Some(EditDialog::ReplaceLinks(picker)) = &st.dialog else {
            panic!("the window stays open")
        };
        assert!(picker.candidates.len() > 20_000);
        let built = picker.built;
        let texts = frame(&mut st);
        assert!(texts < 200, "{texts} texts drawn for the rows in sight");
        let Some(EditDialog::ReplaceLinks(picker)) = &st.dialog else {
            panic!("the window stays open")
        };
        assert_eq!(picker.built, built, "no rebuild without a change");
    }

    #[test]
    fn dropping_an_item_on_the_field_chooses_it_and_replace_submits_it() {
        let ctx = egui::Context::default();
        let inv = fixture(2_000);
        let mut st = InventoryUi::default();
        st.begin_replace_links(crate::world::inventory::demo::BIG_LINKED);
        st.fetch_all = false;
        let prefs = InventoryPreferences::default();
        let facts = Facts::default();
        let frame = |st: &mut InventoryUi, events: Vec<egui::Event>, drag: Option<Uuid>| {
            let mut actions = Vec::new();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if let Some(drag) = drag {
                        egui::DragAndDrop::set_payload(ui.ctx(), InvDrag(drag));
                    }
                    super::super::dialogs::show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &inv,
                        st,
                        &prefs,
                        &facts,
                        &Default::default(),
                        &mut actions,
                    );
                },
            );
            drop(super::super::headless_output(output));
            actions
        };
        let ui_id = st.ui_id;
        let rect = |key: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique((key, ui_id))))
                .expect("widget drawn")
        };
        for _ in 0..3 {
            frame(&mut st, Vec::new(), None);
        }
        let zone = rect("inventory_replace_links_drop").center();
        let release = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        // a texture is refused, a link to another object stands for it
        frame(&mut st, vec![egui::Event::PointerMoved(zone)], Some(id(21_000_032)));
        frame(&mut st, release(zone), Some(id(21_000_032)));
        let Some(EditDialog::ReplaceLinks(picker)) = &st.dialog else {
            panic!("open")
        };
        assert!(picker.refused && picker.selected.is_none());
        frame(&mut st, vec![egui::Event::PointerMoved(zone)], Some(id(21_000_099)));
        frame(&mut st, release(zone), Some(id(21_000_099)));
        let Some(EditDialog::ReplaceLinks(picker)) = &st.dialog else {
            panic!("open")
        };
        assert_eq!(picker.selected, Some(id(21_000_050)));
        assert!(!picker.refused);
        frame(&mut st, Vec::new(), None);
        let confirm = rect("inventory_replace_links_confirm").center();
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(frame(
                &mut st,
                vec![
                    egui::Event::PointerMoved(confirm),
                    egui::Event::PointerButton {
                        pos: confirm,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                None,
            ));
        }
        let [InvAction::Edit(change)] = actions.as_slice() else {
            panic!("one replacement")
        };
        assert!(change.operations.iter().any(|operation| matches!(operation,
            Operation::Links { links, .. } if links[0].target == id(21_000_050))));
        assert!(st.dialog.is_none(), "the window closes once submitted");
    }
}
