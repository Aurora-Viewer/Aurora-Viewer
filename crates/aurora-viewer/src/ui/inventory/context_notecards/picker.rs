//! LLFloaterTexturePicker's searchable, non-empty folder presentation
//! (indra/newview/lltexturectrl.cpp), restricted to native notecards.
//! Uses the inventory's cached sort, ancestor filtering and virtual rows.
use super::{Inventory, available};
use crate::{
    theme::Palette,
    ui::{
        inventory::{Facts, InventoryPreferences, tree, view},
        widgets,
    },
    world::inventory::actions,
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Picker {
    view: view::ViewCache,
    tree: view::TreeState,
    known: HashSet<Uuid>,
    generation: Option<u64>,
    initialized: bool,
    poll: Option<Instant>,
    loaded: bool,
    focus: bool,
    reveal: Option<Uuid>,
}

impl Picker {
    pub(super) fn reveal(&mut self, item: Uuid) {
        self.reveal = Some(item);
    }
    fn prepare(&mut self, ctx: &egui::Context, inv: &mut Inventory, query: &str) {
        if self.generation != Some(inv.generation) {
            self.loaded = false;
            self.tree
                .collapse_folders(inv.folders.keys().filter(|id| !self.known.contains(id)).copied(), 0);
            self.known.extend(inv.folders.keys().copied());
            self.generation = Some(inv.generation);
        }
        if !self.loaded && self.poll.is_none_or(|last| last.elapsed() >= Duration::from_millis(500)) {
            let personal = actions::request_tree(inv, inv.root);
            let library = actions::request_tree(inv, inv.lib_root);
            self.loaded = personal && library;
            self.poll = Some(Instant::now());
        }
        if !self.initialized {
            self.tree.expand_folder(inv.root, 0);
            self.initialized = true;
        }
        if let Some(item) = self.reveal.take() {
            self.tree.reveal(inv, item, 0);
        }
        let prefs = InventoryPreferences {
            sort: view::picker_sort(ctx),
            ..Default::default()
        };
        let filters = view::Filters {
            types: 1 << 7,
            links: view::Links::Exclude,
            ..Default::default()
        };
        self.view
            .search(inv, inv.root, query, &prefs, &filters, 0, None, &Facts::default(), Uuid::nil(), 0);
        self.tree.prepare(&self.view, inv.root, usize::from(!query.trim().is_empty()), true);
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        inv: &mut Inventory,
        query: &mut String,
        selected: &mut Option<Uuid>,
        enabled: bool,
    ) -> (bool, Option<Uuid>) {
        // Inventory menus use the foreground layer; resource floaters use
        // the normal window layer. Retire the owner popup before opening.
        if !self.focus {
            egui::Popup::close_all(ctx);
        }
        let mut open = true;
        let mut cancel = false;
        let mut chosen = None;
        widgets::Floater::new(
            "inventory_menu_notecard_picker",
            aurora_i18n::tr!("inventory-menu-load-notecard"),
            ctx.content_rect().center() - egui::vec2(220.0, 240.0),
            egui::vec2(440.0, 480.0),
        )
        .show(ctx, p, &mut open, |ui| {
            let filter = widgets::search_field(ui, query, aurora_i18n::tr!("build-layout-filter-notecards"), ui.available_width());
            if !self.focus {
                filter.request_focus();
                self.focus = true;
            }
            #[cfg(test)]
            ctx.data_mut(|d| d.insert_temp(egui::Id::unique("menu_note_search"), filter.rect));
            self.prepare(ctx, inv, query);
            // A new query or inventory update must never submit an invisible,
            // deleted or newly restricted previous selection.
            if selected.is_some_and(|id| available(inv, id).is_none() || !self.view.results.contains(&id)) {
                *selected = None;
            }
            let tab = usize::from(!query.trim().is_empty());
            let (mut list, footer) = widgets::above_footer(ui);
            let mut toggle = None;
            tree::frame(&mut list, p, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if self.tree.rows.is_empty() {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("build-layout-no-notecards")).color(p.muted));
                }
                egui::ScrollArea::vertical()
                    .id_salt("menu_note_tree")
                    .auto_shrink([false, false])
                    .max_height(ui.available_height())
                    .show_rows(ui, 22.0, self.tree.rows.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for index in range {
                            let row = self.tree.rows[index];
                            let can_select = available(inv, row.id).is_some();
                            let active = !row.folder && *selected == Some(row.id);
                            let name = if row.id == inv.root {
                                aurora_i18n::tr!("common-my-inventory").to_owned()
                            } else if row.id == inv.lib_root {
                                aurora_i18n::tr!("common-library").to_owned()
                            } else {
                                actions::name(inv, row.id)
                            };
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 22.0),
                                if row.folder || can_select {
                                    egui::Sense::click()
                                } else {
                                    egui::Sense::hover()
                                },
                            );
                            if active || response.hovered() {
                                ui.painter()
                                    .rect_filled(rect, ui.visuals().window_corner_radius, if active { p.violet } else { p.raised });
                            }
                            let mut child = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt(row.id)
                                    .max_rect(egui::Rect::from_min_max(
                                        rect.min + egui::vec2(4.0 + row.depth.min(16) as f32 * 12.0, 0.0),
                                        rect.max,
                                    ))
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            child.spacing_mut().item_spacing.x = 2.0;
                            if row.folder {
                                if tree::caret(&mut child, p, row.open).clicked() {
                                    toggle = Some(row.id);
                                }
                            } else {
                                child.add_space(12.0);
                            }
                            widgets::glyph(
                                &mut child,
                                if row.folder {
                                    if row.open { "folder-open-filled" } else { "folder-filled" }
                                } else {
                                    "file-text-filled"
                                },
                                if active { p.on_accent } else { p.violet_light },
                                14.0,
                            );
                            widgets::paint_elided(
                                &child,
                                &name,
                                child.cursor().min + egui::vec2(4.0, 11.0),
                                child.available_width() - 4.0,
                                egui::FontId::proportional(12.0),
                                if active {
                                    p.on_accent
                                } else if row.folder || can_select {
                                    p.ink
                                } else {
                                    p.muted_dim
                                },
                            );
                            #[cfg(test)]
                            ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("menu_note_row", row.id)), rect));
                            if response.on_hover_text(&name).clicked() {
                                if row.folder {
                                    toggle = Some(row.id);
                                } else if can_select {
                                    *selected = Some(row.id);
                                }
                            }
                        }
                    });
            });
            if let Some(id) = toggle {
                self.tree.toggle(id, tab);
            }
            footer.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let can_load = enabled && selected.is_some_and(|id| available(inv, id).is_some());
                    let reload = ui
                        .add_enabled_ui(can_load, |ui| widgets::flat_button(ui, p, aurora_i18n::tr!("build-layout-reload")))
                        .inner;
                    #[cfg(test)]
                    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("menu_note_reload"), reload.rect));
                    if reload.clicked() {
                        chosen = *selected;
                    }
                    cancel |= widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked();
                });
            });
        });
        (cancel || chosen.is_some() || !open, chosen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::inventory::{
        InvAction, InventoryUi,
        context_notecards::{self, Mode},
    };

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    struct Harness {
        ctx: egui::Context,
        inv: Inventory,
        state: InventoryUi,
        prefs: InventoryPreferences,
        commands: Vec<InvAction>,
    }
    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut inv = Inventory {
                root: id(1),
                ..Default::default()
            };
            inv.folders.insert(
                inv.root,
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id: inv.root,
                        type_default: 8,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: crate::world::inventory::FetchState::Fetched,
                    library: false,
                },
            );
            crate::world::inventory::demo::seed(&mut inv, id(2));
            let mut nested = inv.items[&id(8103)].clone();
            nested.id = id(8301);
            nested.parent = id(8001);
            nested.name = "Nested preset".into();
            inv.add_items(vec![nested]);
            let mut state = InventoryUi::default();
            state.menu_editor.notecard = Some(Mode::Load);
            let mut h = Self {
                ctx,
                inv,
                state,
                prefs: InventoryPreferences::default(),
                commands: Vec::new(),
            };
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h
        }
        fn frame(&mut self, events: Vec<egui::Event>) {
            let p = crate::theme::Theme::default().palette();
            let _ = crate::ui::login::test_output(self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    context_notecards::show(ui.ctx(), &p, &mut self.inv, &mut self.state, &self.prefs, &mut self.commands);
                },
            ));
        }
        fn click(&mut self, key: impl std::hash::Hash + std::fmt::Debug) {
            let pos = self
                .ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(key)))
                .expect("visible picker control")
                .center();
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            self.frame(Vec::new());
        }
    }

    #[test]
    fn opening_the_notecard_picker_dismisses_its_owner_but_allows_later_popups() {
        let mut h = Harness::new();
        let owner = egui::Id::unique("notecard_picker_owner");
        h.state.demo_menu = Some(id(8100));
        h.state.menu_editor.notecard = Some(Mode::Load);
        egui::Popup::open_id(&h.ctx, owner);
        assert!(egui::Popup::is_id_open(&h.ctx, owner));
        h.frame(Vec::new());
        assert!(!egui::Popup::is_id_open(&h.ctx, owner));
        assert!(h.state.demo_menu.is_none());
        let other = egui::Id::unique("notecard_picker_text_menu");
        egui::Popup::open_id(&h.ctx, other);
        h.frame(Vec::new());
        assert!(
            egui::Popup::is_id_open(&h.ctx, other),
            "the picker must not dismiss later field menus"
        );
    }

    #[test]
    fn native_notecard_picker_browses_nested_folders_and_loads_only_after_confirmation() {
        let mut h = Harness::new();
        assert!(!h.state.menu_notecards.picker.tree.rows.iter().any(|r| r.id == id(8301)));
        h.click(("menu_note_row", id(8000)));
        assert!(h.state.menu_notecards.selected.is_none(), "folders cannot be submitted");
        h.click(("menu_note_row", id(8001)));
        h.click(("menu_note_row", id(8301)));
        assert_eq!(h.state.menu_notecards.selected, Some(id(8301)));
        assert!(h.commands.is_empty(), "selecting must not fetch an asset");
        h.click("menu_note_reload");
        assert!(matches!(h.commands.as_slice(), [InvAction::LoadMenuNotecard(item)] if *item == id(8301)));
        assert!(h.state.menu_notecards.open.is_none());
    }

    #[test]
    fn native_notecard_picker_search_retains_ancestors_and_rechecks_visibility_and_copy_permission() {
        let mut h = Harness::new();
        h.click("menu_note_search");
        h.frame(vec![egui::Event::Text("nested".into())]);
        h.frame(Vec::new());
        let rows = &h.state.menu_notecards.picker.tree.rows;
        assert_eq!(
            rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [h.inv.root, id(8000), id(8001), id(8301)]
        );
        assert_eq!(rows.last().unwrap().depth, 3);
        assert!(!rows.iter().any(|r| r.id == id(8104)), "textures are excluded");
        h.click(("menu_note_row", id(8301)));
        h.inv.items.get_mut(&id(8301)).unwrap().owner_mask &= !actions::COPY;
        h.inv.generation += 1;
        h.frame(Vec::new());
        assert!(h.state.menu_notecards.selected.is_none());
        h.click("menu_note_reload");
        assert!(h.commands.is_empty() && h.state.menu_notecards.open.is_some());
        h.state.menu_notecards.filter.clear();
        h.frame(Vec::new());
        assert!(
            !h.state.menu_notecards.picker.tree.rows.iter().any(|r| r.id == id(8301)),
            "search preserves collapsed browsing branches"
        );
    }
}
