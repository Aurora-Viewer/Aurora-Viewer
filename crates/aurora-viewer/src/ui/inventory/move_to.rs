//! Destination picker for the inventory's "Move to…" action.
//! Uses LLFolderBridge's move / protected-folder rules (Firestorm
//! indra/newview/llinventorybridge.cpp, originally LGPL 2.1). The searchable
//! picker is an Aurora addition; submission uses the existing move mutation.

use super::*;
use crate::ui::widgets::{Floater, above_footer, flat_button, search_field};
use rayon::prelude::*;
use std::time::{Duration, Instant};

struct Destination {
    id: Uuid,
    name: String,
    path: String,
    search: String,
}

pub(super) struct MoveTo {
    ids: Vec<Uuid>,
    pub(super) filter: String,
    selected: Option<Uuid>,
    focus: bool,
    generation: Option<(u64, &'static str, view::Sort)>,
    protected: HashSet<Uuid>,
    destinations: Vec<Destination>,
    visible: Vec<usize>,
    query: Option<String>,
    fetch_poll: Option<Instant>,
    loaded: bool,
    failed: bool,
    message: String,
}

impl MoveTo {
    pub(super) fn new(ids: Vec<Uuid>) -> Self {
        Self {
            ids,
            filter: String::new(),
            selected: None,
            focus: true,
            generation: None,
            protected: HashSet::new(),
            destinations: Vec::new(),
            visible: Vec::new(),
            query: None,
            fetch_poll: None,
            loaded: false,
            failed: false,
            message: String::new(),
        }
    }

    fn update(&mut self, ctx: &egui::Context, inv: &Inventory, protected: &HashSet<Uuid>) {
        let generation = (inv.generation, aurora_i18n::language(), view::picker_sort(ctx));
        if self.generation != Some(generation) || self.protected != *protected {
            self.destinations = destinations(inv, &self.ids, protected);
            let order = view::picker_order(ctx, inv);
            self.destinations.par_sort_unstable_by_key(|row| order.rank(row.id));
            self.failed = inv
                .folders
                .values()
                .any(|folder| !folder.library && folder.state == FetchState::Failed);
            self.generation = Some(generation);
            self.protected.clone_from(protected);
            self.query = None;
        }
        let query = self.filter.trim().to_lowercase();
        if self.query.as_ref() != Some(&query) {
            self.visible = self
                .destinations
                .par_iter()
                .enumerate()
                .filter_map(|(i, row)| row.search.contains(&query).then_some(i))
                .collect();
            // Never submit a destination which the current filter hides.
            if self
                .selected
                .is_some_and(|id| !self.visible.iter().any(|i| self.destinations[*i].id == id))
            {
                self.selected = None;
            }
            self.query = Some(query);
        }
    }
}

fn destinations(inv: &Inventory, ids: &[Uuid], protected: &HashSet<Uuid>) -> Vec<Destination> {
    if ids.is_empty() || ids.iter().any(|id| !rules::movable(inv, *id, protected)) {
        return Vec::new();
    }
    let roots = rules::roots(inv, ids);
    // Resolve system roots once. Calling destination() separately for every
    // folder would repeatedly search the entire inventory for those roots.
    let excluded: HashSet<_> = [14, 46, 48, 50, 53]
        .into_iter()
        .filter_map(|kind| rules::system(inv, kind))
        .chain(protected.iter().copied())
        .chain(roots.iter().filter(|id| inv.folders.contains_key(id)).copied())
        .collect();
    inv.folders
        .par_iter()
        .filter_map(|(&id, _)| {
            if roots.iter().all(|source| rules::parent(inv, *source) == Some(id)) {
                return None;
            }
            let mut parts = Vec::new();
            let mut current = id;
            let mut seen = HashSet::new();
            while seen.insert(current) {
                let folder = inv.folders.get(&current)?;
                if excluded.contains(&current) || folder.library {
                    return None;
                }
                parts.push(folder_name(inv, current));
                if current == inv.root {
                    let name = parts.first()?.clone();
                    parts.reverse();
                    return Some(Destination {
                        id,
                        search: name.to_lowercase(),
                        name,
                        path: parts.join(" / "),
                    });
                }
                current = folder.info.parent;
            }
            None
        })
        .collect()
}

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(mut picker) = st.move_to.take() else { return };
    if picker.generation.is_some_and(|generation| generation.0 != inv.generation) {
        picker.loaded = false;
    }
    if !picker.loaded && picker.fetch_poll.is_none_or(|last| last.elapsed() >= Duration::from_millis(500)) {
        picker.loaded = rules::request_tree(inv, inv.root);
        picker.fetch_poll = Some(Instant::now());
    }
    view::set_picker_sort(ctx, prefs.sort);
    picker.update(ctx, inv, &prefs.protected);
    let mut open = true;
    let mut done = false;
    let window_id = format!("inventory_move_to_{}", st.ui_id);
    Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-move-to"),
        ctx.content_rect().center() - egui::vec2(260.0, 220.0),
        Vec2::new(520.0, 440.0),
    )
    .show(ctx, p, &mut open, |ui| {
        let summary = if picker.ids.len() == 1 {
            aurora_i18n::tr!("inventory-move-selection-name", name = rules::name(inv, picker.ids[0]))
        } else {
            aurora_i18n::tr!("inventory-move-selection-count", count = picker.ids.len())
        };
        ui.add(egui::Label::new(summary).truncate())
            .on_hover_text(picker.ids.iter().map(|id| rules::name(inv, *id)).collect::<Vec<_>>().join("\n"));
        let width = ui.available_width();
        let filter = search_field(ui, &mut picker.filter, aurora_i18n::tr!("inventory-move-filter"), width);
        if picker.focus {
            filter.request_focus();
            picker.focus = false;
        }
        picker.update(ctx, inv, &prefs.protected);
        let keyboard = filter.has_focus() || filter.lost_focus();
        let enter = keyboard && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if keyboard && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }
        #[cfg(test)]
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_move_filter", st.ui_id)), filter.rect));

        // the destinations in exactly the room the count, the buttons and the
        // message leave
        let (mut list, foot) = above_footer(ui);
        let list_height = list.available_height();
        egui::Frame::new().fill(p.field).show(&mut list, |ui| {
            ui.set_min_height(list_height);
            if picker.visible.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("inventory-move-no-folders")).color(p.muted));
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt(("inventory_move_destinations", st.ui_id))
                .max_height(ui.available_height())
                .auto_shrink([false, false])
                .show_rows(ui, 40.0, picker.visible.len(), |ui, range| {
                    for i in range {
                        let row = &picker.destinations[picker.visible[i]];
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::click());
                        if picker.selected == Some(row.id) {
                            ui.painter()
                                .rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.35));
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
                        }
                        if let Some(icon) = icons.get("folder-filled") {
                            ui.painter().image(
                                icon.id(),
                                egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 5.0), Vec2::splat(16.0)),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                p.muted,
                            );
                        }
                        for (text, offset, size, color) in [
                            (&row.name, egui::vec2(28.0, 3.0), 13.0, p.ink),
                            (&row.path, egui::vec2(28.0, 22.0), 11.0, p.muted),
                        ] {
                            let galley = egui::WidgetText::from(RichText::new(text).size(size).color(color)).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                (rect.width() - 34.0).max(0.0),
                                egui::TextStyle::Body,
                            );
                            ui.painter().galley(rect.min + offset, galley, color);
                        }
                        #[cfg(test)]
                        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_move_destination", st.ui_id, row.id)), rect));
                        if response.on_hover_text(&row.path).clicked() {
                            picker.selected = Some(row.id);
                            picker.message.clear();
                            filter.request_focus();
                        }
                    }
                });
        });
        foot.show(ui, |ui| {
            ui.label(
                RichText::new(if picker.failed {
                    aurora_i18n::tr!("inventory-move-loading-failed").into()
                } else if picker.loaded {
                    aurora_i18n::tr!("inventory-move-folder-count", count = picker.visible.len())
                } else {
                    aurora_i18n::tr!("inventory-move-loading").into()
                })
                .size(11.0)
                .color(p.muted),
            );
            ui.horizontal(|ui| {
                let enabled = st.pending.is_none() && picker.selected.is_some();
                ui.add_enabled_ui(enabled, |ui| {
                    let button = flat_button(ui, p, aurora_i18n::tr!("inventory-move-confirm"));
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_move_confirm", st.ui_id)), button.rect));
                    if (button.clicked() || enter)
                        && enabled
                        && let Some(to) = picker.selected
                    {
                        // Revalidate against the current model and permissions;
                        // the selection is the snapshot taken when the menu opened.
                        match rules::move_selection(inv, &picker.ids, to, &prefs.protected, &facts.worn) {
                            Ok(change) if !change.operations.is_empty() => {
                                actions.push(InvAction::Edit(change));
                                done = true;
                            }
                            Ok(_) => picker.selected = None,
                            Err(reason) => picker.message = reason,
                        }
                    }
                });
                let cancel = flat_button(ui, p, aurora_i18n::tr!("common-cancel"));
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_move_cancel", st.ui_id)), cancel.rect));
                done |= cancel.clicked();
            });
            if !picker.message.is_empty() {
                ui.label(RichText::new(&picker.message).color(p.warn));
            }
        });
    });
    if open && !done {
        st.move_to = Some(picker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::{InvFolder, operations::Operation};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn fixture() -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            Folder {
                info: InvFolder {
                    id: inv.root,
                    type_default: 8,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        crate::world::inventory::demo::seed(&mut inv, id(2));
        inv.folders.insert(
            id(8300),
            Folder {
                info: InvFolder {
                    id: id(8300),
                    parent: inv.root,
                    name: "Archives".into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&inv.root).expect("root").children.push(id(8300));
        inv
    }

    fn facts() -> Facts {
        Facts {
            agent: id(2),
            worn: HashSet::new(),
            worn_labels: Default::default(),
            worn_at: Default::default(),
            points: Vec::new(),
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
            names: Default::default(),
            ..Default::default()
        }
    }

    #[test]
    fn destinations_follow_move_rules_for_folders_items_and_mixed_selections() {
        let inv = fixture();
        for selection in [vec![id(8000)], vec![id(8109), id(8110)], vec![id(8000), id(8100)], vec![inv.root]] {
            for protected in [HashSet::new(), HashSet::from([id(8001)]), HashSet::from([id(8000)])] {
                let rows = destinations(&inv, &selection, &protected);
                for &to in inv.folders.keys() {
                    let expected = rules::under(&inv, to, inv.root)
                        && rules::destination(&inv, to, &protected)
                        && rules::move_selection(&inv, &selection, to, &protected, &HashSet::new())
                            .is_ok_and(|change| !change.operations.is_empty());
                    assert_eq!(
                        rows.iter().any(|row| row.id == to),
                        expected,
                        "selection {selection:?}, destination {to}"
                    );
                }
            }
        }
    }

    #[test]
    fn filter_is_case_insensitive_and_duplicate_folder_names_keep_their_paths() {
        let mut inv = fixture();
        for (n, parent) in [(8200, inv.root), (8201, id(8001))] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent,
                        name: "Destination".into(),
                        type_default: -1,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        let mut picker = MoveTo::new(vec![id(8100)]);
        picker.filter = "  dEsTiNa  ".into();
        picker.update(&egui::Context::default(), &inv, &HashSet::new());
        assert_eq!(picker.visible.len(), 2);
        let rows: Vec<_> = picker.visible.iter().map(|i| &picker.destinations[*i]).collect();
        assert_eq!(rows[0].name, rows[1].name);
        assert_ne!(rows[0].path, rows[1].path);
        picker.selected = Some(rows[0].id);
        picker.filter = "absent".into();
        picker.update(&egui::Context::default(), &inv, &HashSet::new());
        assert!(picker.visible.is_empty());
        assert!(picker.selected.is_none());
    }

    #[test]
    fn open_destination_picker_tracks_folder_date_and_name_options() {
        let ctx = egui::Context::default();
        let mut inv = fixture();
        for (n, name, date) in [(8500, "Sort folder 10", 200), (8501, "sort folder 2", 100)] {
            inv.created_folders(vec![InvFolder {
                id: id(n),
                parent: inv.root,
                name: name.into(),
                type_default: -1,
                ..Default::default()
            }]);
            inv.add_items(vec![aurora_net::inventory::InvItem {
                id: id(n + 10),
                parent: id(n),
                name: "Nested item".into(),
                created_at: date,
                ..Default::default()
            }]);
        }
        let mut picker = MoveTo::new(vec![id(8100)]);
        picker.filter = "sort folder".into();
        for (by_date, folders_by_name) in [(false, true), (true, false), (true, true), (true, false)] {
            view::set_picker_sort(
                &ctx,
                view::Sort {
                    by_date,
                    folders_by_name,
                    ..Default::default()
                },
            );
            picker.update(&ctx, &inv, &HashSet::new());
            let names: Vec<_> = picker.visible.iter().map(|i| picker.destinations[*i].name.as_str()).collect();
            assert_eq!(
                names,
                if by_date && !folders_by_name {
                    ["Sort folder 10", "sort folder 2"]
                } else {
                    ["sort folder 2", "Sort folder 10"]
                }
            );
        }
    }

    #[test]
    fn changing_permissions_or_folder_membership_invalidates_the_destination() {
        let mut inv = fixture();
        let mut picker = MoveTo::new(vec![id(8100)]);
        picker.update(&egui::Context::default(), &inv, &HashSet::new());
        picker.selected = Some(id(8001));
        picker.update(&egui::Context::default(), &inv, &HashSet::from([id(8001)]));
        assert!(picker.selected.is_none());
        picker.update(&egui::Context::default(), &inv, &HashSet::new());
        picker.selected = Some(id(8001));
        let change = rules::move_selection(&inv, &picker.ids, id(8001), &HashSet::new(), &HashSet::new()).expect("move");
        rules::demo_mutate(&mut inv, id(2), change).expect("demo move");
        picker.update(&egui::Context::default(), &inv, &HashSet::new());
        assert!(picker.selected.is_none(), "the new parent is no longer a useful destination");
    }

    /// The window keeps its size with its tallest footer: no folder found
    /// and a message under the buttons.
    #[test]
    fn the_window_keeps_its_size() {
        let prefs = InventoryPreferences::default();
        crate::ui::widgets::steady_tests::assert_window_steady(
            "inventory_move_to_00000000-0000-0000-0000-00000000004d",
            &[Vec2::new(520.0, 440.0), Vec2::new(320.0, 300.0)],
            "The folder cannot hold these items: one of them is worn.",
            || {
                let mut st = InventoryUi {
                    ui_id: id(77),
                    ..Default::default()
                };
                st.begin_move_to(id(8000));
                let picker = st.move_to.as_mut().expect("picker");
                picker.filter = "no such folder".into();
                picker.message = "The folder cannot hold these items: one of them is worn.".into();
                (fixture(), st)
            },
            |(inv, st), ctx, p| {
                show(ctx, p, &Icons::default(), inv, st, &prefs, &facts(), &mut Vec::new());
            },
        );
    }

    fn frame(
        ctx: &egui::Context,
        inv: &mut Inventory,
        st: &mut InventoryUi,
        prefs: &InventoryPreferences,
        events: Vec<egui::Event>,
    ) -> Vec<InvAction> {
        let mut actions = Vec::new();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &Icons::default(),
                    inv,
                    st,
                    prefs,
                    &facts(),
                    &mut actions,
                );
            },
        );
        drop(super::super::headless_output(output));
        actions
    }

    fn rect(ctx: &egui::Context, key: impl std::hash::Hash + std::fmt::Debug) -> egui::Rect {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(key)))
            .expect("widget drawn")
    }

    fn click(
        ctx: &egui::Context,
        inv: &mut Inventory,
        st: &mut InventoryUi,
        prefs: &InventoryPreferences,
        pos: egui::Pos2,
    ) -> Vec<InvAction> {
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(frame(
                ctx,
                inv,
                st,
                prefs,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            ));
        }
        actions
    }

    #[test]
    fn selecting_and_confirming_moves_the_original_multi_selection_without_touching_the_clipboard() {
        let ctx = egui::Context::default();
        let mut inv = fixture();
        let mut st = InventoryUi {
            selection: HashSet::from([id(8109), id(8110)]),
            clipboard: vec![id(8103)],
            cut: true,
            ..Default::default()
        };
        let prefs = InventoryPreferences::default();
        st.begin_move_to(id(8109));
        st.selection = HashSet::from([id(8103)]);
        for _ in 0..3 {
            assert!(frame(&ctx, &mut inv, &mut st, &prefs, Vec::new()).is_empty());
        }
        let confirm = rect(&ctx, ("inventory_move_confirm", st.ui_id)).center();
        assert!(
            click(&ctx, &mut inv, &mut st, &prefs, confirm).is_empty(),
            "choose a destination first"
        );
        let filter = rect(&ctx, ("inventory_move_filter", st.ui_id)).center();
        click(&ctx, &mut inv, &mut st, &prefs, filter);
        let query = folder_name(&inv, id(8001)).to_uppercase();
        frame(&ctx, &mut inv, &mut st, &prefs, vec![egui::Event::Text(query)]);
        assert_eq!(st.move_to.as_ref().expect("picker").visible.len(), 1);
        let row = rect(&ctx, ("inventory_move_destination", st.ui_id, id(8001))).center();
        assert!(
            click(&ctx, &mut inv, &mut st, &prefs, row).is_empty(),
            "selection alone must not move anything"
        );
        let confirm = rect(&ctx, ("inventory_move_confirm", st.ui_id)).center();
        let actions = click(&ctx, &mut inv, &mut st, &prefs, confirm);
        let [InvAction::Edit(change)] = actions.as_slice() else {
            panic!("one move mutation")
        };
        assert_eq!(change.operations.len(), 2);
        assert!(change.operations.iter().all(|operation| matches!(operation,
            Operation::Patch { id: source, folder: false, body } if [id(8109), id(8110)].contains(source) && body["parent_id"].as_uuid() == id(8001)
        )));
        assert!(st.move_to.is_none());
        assert_eq!(st.clipboard, vec![id(8103)]);
        assert!(st.cut);
        assert!(!st.consume_clipboard);
    }

    #[test]
    fn cancel_leaves_the_folder_and_its_contents_in_place() {
        let ctx = egui::Context::default();
        let mut inv = fixture();
        let mut st = InventoryUi::default();
        let prefs = InventoryPreferences::default();
        st.begin_move_to(id(8000));
        for _ in 0..3 {
            frame(&ctx, &mut inv, &mut st, &prefs, Vec::new());
        }
        let cancel = rect(&ctx, ("inventory_move_cancel", st.ui_id)).center();
        assert!(click(&ctx, &mut inv, &mut st, &prefs, cancel).is_empty());
        assert!(st.move_to.is_none());
        assert_eq!(rules::parent(&inv, id(8000)), Some(inv.root));
        assert_eq!(rules::parent(&inv, id(8100)), Some(id(8000)));
    }

    #[test]
    fn context_menu_opens_the_picker_for_folders_items_and_multi_selection() {
        for (source, selection) in [
            (id(8000), HashSet::from([id(8000)])),
            (id(8100), HashSet::from([id(8100)])),
            (id(8109), HashSet::from([id(8109), id(8110)])),
        ] {
            let ctx = egui::Context::default();
            let inv = fixture();
            let p = crate::theme::Theme::default().palette();
            let mut st = InventoryUi {
                selection: selection.clone(),
                ..Default::default()
            };
            let mut open = true;
            let mut preferences = InventoryPreferences::default();
            preferences
                .context_menus
                .profiles
                .entry(super::super::context_catalog::scope(&inv, source))
                .or_default()
                .open
                .insert("organize".into(), true);
            let mut position = None;
            for pass in 0..5 {
                let events = if pass >= 3 {
                    vec![
                        egui::Event::PointerMoved(position.expect("menu entry")),
                        egui::Event::PointerButton {
                            pos: position.expect("menu entry"),
                            button: egui::PointerButton::Primary,
                            pressed: pass == 3,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    Vec::new()
                };
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1200.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        crate::ui::menu::popup_at(
                            ui.ctx(),
                            egui::Id::unique("move_menu_test"),
                            egui::pos2(10.0, 10.0),
                            &mut open,
                            &p,
                            |ui| {
                                super::super::context::show(ui, &p, &inv, source, &mut st, &mut preferences, &facts(), &mut Vec::new());
                            },
                        );
                    },
                );
                position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("inventory-move-to") => {
                            Some(text.pos + text.galley.size() * 0.5)
                        }
                        _ => None,
                    })
                    .or(position);
                drop(super::super::headless_output(output));
            }
            let actual: HashSet<_> = st.move_to.expect("picker opened by menu").ids.into_iter().collect();
            assert_eq!(actual, selection);
            assert!(!open, "the context menu closes");
        }
    }

    #[test]
    fn enter_confirms_the_chosen_folder_after_a_row_click() {
        let ctx = egui::Context::default();
        let mut inv = fixture();
        let mut st = InventoryUi::default();
        let prefs = InventoryPreferences::default();
        st.begin_move_to(id(8100));
        st.filter_move_to(&folder_name(&inv, id(8001)));
        for _ in 0..3 {
            frame(&ctx, &mut inv, &mut st, &prefs, Vec::new());
        }
        let row = rect(&ctx, ("inventory_move_destination", st.ui_id, id(8001))).center();
        click(&ctx, &mut inv, &mut st, &prefs, row);
        let actions = frame(
            &ctx,
            &mut inv,
            &mut st,
            &prefs,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(matches!(actions.as_slice(), [InvAction::Edit(_)]));
        assert!(st.move_to.is_none());
    }

    #[test]
    fn picker_from_a_folder_window_outlives_its_source_and_can_choose_other_folders() {
        let ctx = egui::Context::default();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = id(2);
        world.inventory = fixture();
        let mut st = InventoryUi::default();
        st.open_folder_window(id(8000));
        let window = &mut st.windows[0];
        window.open = false;
        window.state.begin_move_to(id(8100));
        for _ in 0..3 {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    assert!(
                        super::super::show(
                            ui.ctx(),
                            &crate::theme::Theme::default().palette(),
                            &Icons::default(),
                            &mut world,
                            &mut st,
                            &mut InventoryPreferences::default(),
                            &mut false,
                            false,
                            &Default::default(),
                            &Default::default(),
                            &Default::default(),
                            &Default::default(),
                            true,
                        )
                        .is_empty()
                    );
                },
            );
            drop(super::super::headless_output(output));
        }
        assert_eq!(st.windows.len(), 1);
        assert!(
            st.windows[0]
                .state
                .move_to
                .as_ref()
                .expect("picker survives")
                .destinations
                .iter()
                .any(|row| row.id == id(8300))
        );
    }

    #[test]
    fn a_folder_and_its_selected_descendant_are_submitted_once() {
        let ctx = egui::Context::default();
        let mut inv = fixture();
        let mut st = InventoryUi {
            selection: HashSet::from([id(8000), id(8100)]),
            ..Default::default()
        };
        let prefs = InventoryPreferences::default();
        st.begin_move_to(id(8000));
        st.filter_move_to(&folder_name(&inv, id(700)));
        for _ in 0..3 {
            frame(&ctx, &mut inv, &mut st, &prefs, Vec::new());
        }
        // Pick any eligible system destination from the actual list.
        let picker = st.move_to.as_mut().expect("picker");
        picker.filter.clear();
        picker.update(&egui::Context::default(), &inv, &prefs.protected);
        let to = picker.destinations.first().expect("destination").id;
        picker.selected = Some(to);
        frame(&ctx, &mut inv, &mut st, &prefs, Vec::new());
        let confirm = rect(&ctx, ("inventory_move_confirm", st.ui_id)).center();
        let actions = click(&ctx, &mut inv, &mut st, &prefs, confirm);
        let [InvAction::Edit(change)] = actions.as_slice() else {
            panic!("one move mutation")
        };
        assert!(
            matches!(change.operations.as_slice(), [Operation::Patch { id: source, folder: true, body }]
            if *source == id(8000) && body["parent_id"].as_uuid() == to)
        );
    }
}
