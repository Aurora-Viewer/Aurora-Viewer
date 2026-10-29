//! Inventory panel hosted by Marketplace listings. LLPanelMarketplaceListings
//! uses the ordinary inventory bridges (indra/newview/llinventorybridge.cpp).
//! Keep its selection and rename separate, with the inventory windows owning
//! its dialogs, thumbnails and documents and sharing the inventory clipboard.
use super::*;

pub(crate) const PANEL: Uuid = Uuid::from_u128(0x89b3_4fc2_8f71_41d6_b3c7_7e99_62b1_1164);

pub(crate) struct Panel<'a> {
    pub state: &'a mut InventoryUi,
    pub prefs: &'a mut InventoryPreferences,
    pub facts: &'a Facts,
    pub actions: Vec<InvAction>,
}

pub(super) fn restrict(c: &mut context_catalog::Catalog, inv: &Inventory, id: Uuid, st: &InventoryUi) {
    let Some(root) = crate::world::marketplace::root(inv).filter(|root| rules::under(inv, id, *root)) else {
        return;
    };
    if st.ui_id == PANEL {
        c.entries.retain(|entry| {
            entry.id.starts_with("listing")
                || entry.id.starts_with("separator:")
                || matches!(
                    entry.id.as_str(),
                    "lock"
                        | "new"
                        | "new-folder"
                        | "properties"
                        | "rename"
                        | "image"
                        | "copy"
                        | "cut"
                        | "paste"
                        | "favorite"
                        | "favorite-add"
                        | "favorite-remove"
                        | "organize"
                        | "move"
                        | "default-folder"
                        | "group-selection"
                        | "find-links"
                        | "paste-link"
                        | "replace-links"
                        | "protect-folder"
                        | "reload"
                        | "show-window"
                        | "delete"
                        | "copy-name"
                        | "folder-uuid"
                        | "show-main"
                )
        });
    }
    let to = inv.folders.contains_key(&id).then_some(id).or_else(|| rules::parent(inv, id));
    for entry in &mut c.entries {
        if entry.id == "new-folder" {
            entry.enabled &= to.is_some_and(|to| crate::world::marketplace::validate::can_create_folder(inv, root, to));
        }
        if entry.id == "paste-link" {
            entry.enabled = false;
        }
        if entry.id.starts_with("new-") && entry.id != "new-folder" {
            entry.enabled = false;
        }
    }
}

impl InventoryUi {
    pub(crate) fn select_created_listing(&mut self, inv: &Inventory, id: Uuid) -> bool {
        if !crate::world::marketplace::root(inv).is_some_and(|root| rules::under(inv, id, root)) {
            return false;
        }
        let Some(window) = self.windows.iter_mut().find(|w| w.id == PANEL) else {
            return false;
        };
        window.state.begin_rename(inv, id);
        true
    }

    pub(crate) fn take_listings_panel(&mut self) -> InventoryWindow {
        let mut window = self
            .windows
            .iter()
            .position(|w| w.id == PANEL)
            .map(|i| self.windows.remove(i))
            .unwrap_or_else(|| InventoryWindow {
                id: PANEL,
                root: Uuid::nil(),
                additional: false,
                state: Box::new(InventoryUi {
                    ui_id: PANEL,
                    ..Default::default()
                }),
                open: false,
            });
        window.state.clipboard.clone_from(&self.clipboard);
        window.state.cut = self.cut;
        window.state.pending = self.pending;
        window.state.merchant = self.merchant;
        window
    }

    pub(crate) fn put_listings_panel(&mut self, mut window: InventoryWindow) {
        self.clipboard.clone_from(&window.state.clipboard);
        self.cut = window.state.cut;
        self.consume_clipboard |= std::mem::take(&mut window.state.consume_clipboard);
        self.windows.push(window);
    }
}

impl Panel<'_> {
    pub fn begin(&mut self) {
        self.state.plain.order.clone_from(&self.state.visible);
        self.state.visible.clear();
    }

    pub fn reveal(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.state.reveal_path.iter().copied()
    }

    pub fn rename(&mut self, ui: &mut egui::Ui, p: &Palette, inv: &Inventory, id: Uuid) -> bool {
        self.state.visible.push(id);
        rename_row(ui, p, inv, id, self.state, self.prefs, &mut self.actions)
    }

    pub fn interact(&mut self, ui: &egui::Ui, inv: &Inventory, id: Uuid, response: &egui::Response) {
        if response.clicked() || response.secondary_clicked() {
            // The same selection rules, with the order of this hosted tree.
            self.state.select(ui, id, response.secondary_clicked());
        }
        if response.double_clicked() && inv.items.contains_key(&id) {
            context::open(inv, id, self.state, &mut self.actions);
        }
    }

    pub fn menu(&mut self, ui: &mut egui::Ui, p: &Palette, inv: &Inventory, id: Uuid) {
        context::show(ui, p, inv, id, self.state, self.prefs, self.facts, &mut self.actions);
    }

    pub fn finish(&mut self, ctx: &egui::Context, inv: &Inventory) {
        finish_hidden_rename(inv, self.state, self.prefs, &mut self.actions);
        self.state.menu_editor.finish_inventory_frame(ctx, &mut self.prefs.context_menus);
    }

    pub fn shortcuts(&mut self, ui: &mut egui::Ui, inv: &Inventory, root: Uuid) {
        clipboard::shortcuts(ui, inv, root, self.state, self.prefs, self.facts, &mut self.actions);
    }

    pub fn focus(&mut self, ui: &mut egui::Ui, height: f32) {
        clipboard::list_focus(ui, self.state, height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::marketplace::demo;

    fn inventory() -> Inventory {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.created_folders(vec![aurora_net::inventory::InvFolder {
            id: inv.root,
            type_default: 8,
            ..Default::default()
        }]);
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        inv.remove(&[Uuid::from_u128(8002)]);
        demo::seed(&mut inv, Uuid::from_u128(2));
        inv
    }

    #[test]
    fn listing_version_and_item_menus_expose_the_legacy_inventory_actions() {
        let inv = inventory();
        let state = InventoryUi {
            ui_id: PANEL,
            ..Default::default()
        };
        let prefs = InventoryPreferences::default();
        let facts = Facts::default();
        for id in [demo::HOODIE, demo::HOODIE_V2, Uuid::from_u128(8_400_102), demo::POSTER_STOCK] {
            let catalog = context_catalog::build(&inv, id, &state, &prefs, &facts);
            for key in [
                "rename",
                "image",
                "copy",
                "cut",
                "paste",
                "paste-link",
                "find-links",
                "delete",
                "group-selection",
                "favorite",
            ] {
                assert!(catalog.entries.iter().any(|entry| entry.id == key), "{id}: {key}");
            }
            assert!(
                !catalog
                    .entries
                    .iter()
                    .find(|entry| entry.id == "paste-link")
                    .expect("paste links")
                    .enabled
            );
            assert!(
                !catalog
                    .entries
                    .iter()
                    .any(|entry| ["share", "marketplace-copy", "marketplace-move"].contains(&entry.id.as_str()))
            );
            if inv.folders.contains_key(&id) {
                for key in ["protect-folder", "reload", "show-window"] {
                    assert!(catalog.entries.iter().any(|entry| entry.id == key && entry.enabled), "{id}: {key}");
                }
                assert_eq!(
                    catalog
                        .entries
                        .iter()
                        .find(|entry| entry.id == "new-folder")
                        .expect("new folder")
                        .enabled,
                    id != demo::POSTER_STOCK
                );
                assert!(!catalog.entries.iter().any(|entry| entry.id == "new-script"));
            } else {
                for key in ["properties", "replace-links", "default-folder"] {
                    assert!(catalog.entries.iter().any(|entry| entry.id == key && entry.enabled), "{id}: {key}");
                }
            }
        }
    }

    fn execute(key: &str, id: Uuid, inv: &Inventory, state: &mut InventoryUi, prefs: &mut InventoryPreferences) -> Vec<InvAction> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let mut catalog = context_catalog::build(inv, id, state, prefs, &Facts::default());
            let entry = catalog.entries.iter().find(|entry| entry.id == key).expect("menu entry");
            assert!(entry.enabled, "{key}");
            let menu_layout::Kind::Action(index) = entry.kind else {
                panic!("action entry")
            };
            let command = catalog.commands[index].take().expect("native command");
            context_catalog::execute(command, ui, inv, state, prefs, &Facts::default(), &mut actions);
        });
        output.textures_delta.clear();
        actions
    }

    #[test]
    fn hosted_commands_reach_the_inventory_dialogs_clipboard_and_mutations() {
        let inv = inventory();
        let item = Uuid::from_u128(8_400_102);
        let mut state = InventoryUi {
            ui_id: PANEL,
            selected: Some(item),
            selection: HashSet::from([item]),
            ..Default::default()
        };
        let mut prefs = InventoryPreferences::default();
        execute("properties", item, &inv, &mut state, &mut prefs);
        assert!(matches!(state.dialog, Some(EditDialog::Properties(id)) if id == item));
        state.dialog = None;
        execute("rename", item, &inv, &mut state, &mut prefs);
        assert_eq!(state.rename.as_ref().map(|rename| rename.id), Some(item));
        execute("copy", item, &inv, &mut state, &mut prefs);
        assert_eq!(state.clipboard, vec![item]);
        assert!(!state.cut);
        execute("image", item, &inv, &mut state, &mut prefs);
        assert_eq!(state.thumbnail.item, Some(item));
        assert!(matches!(execute("find-links", item, &inv, &mut state, &mut prefs).as_slice(), [InvAction::FindLinks(id)] if *id == item));
        assert!(matches!(
            execute("favorite", item, &inv, &mut state, &mut prefs).as_slice(),
            [InvAction::Edit(_)]
        ));
        execute("delete", item, &inv, &mut state, &mut prefs);
        assert!(matches!(&state.dialog, Some(EditDialog::Delete(ids, false)) if ids == &[item]));
        state.dialog = None;
        execute("protect-folder", demo::HOODIE, &inv, &mut state, &mut prefs);
        assert!(prefs.protected.contains(&demo::HOODIE));
        let catalog = context_catalog::build(&inv, item, &state, &prefs, &Facts::default());
        for key in ["rename", "cut", "delete"] {
            assert!(
                !catalog
                    .entries
                    .iter()
                    .find(|entry| entry.id == key)
                    .expect("protected command")
                    .enabled
            );
        }
        execute("protect-folder", demo::HOODIE, &inv, &mut state, &mut prefs);
        execute("show-window", demo::HOODIE, &inv, &mut state, &mut prefs);
        assert!(state.windows.iter().any(|window| window.root == demo::HOODIE && window.open));
        assert!(
            matches!(execute("reload", demo::HOODIE, &inv, &mut state, &mut prefs).as_slice(), [InvAction::Preview(id)] if *id == demo::HOODIE)
        );
    }

    #[test]
    fn the_panel_keeps_its_selection_and_shares_the_main_inventory_clipboard() {
        let mut main = InventoryUi {
            clipboard: vec![demo::HOODIE],
            cut: true,
            ..Default::default()
        };
        let mut window = main.take_listings_panel();
        assert_eq!(window.state.clipboard, main.clipboard);
        assert!(window.state.cut);
        window.state.selected = Some(demo::HOODIE_V2);
        window.state.clipboard = vec![demo::HOODIE_V2];
        window.state.cut = false;
        main.put_listings_panel(window);
        assert_eq!(main.clipboard, vec![demo::HOODIE_V2]);
        assert!(!main.cut);
        assert!(main.selected.is_none());
        let window = main.take_listings_panel();
        assert_eq!(window.state.selected, Some(demo::HOODIE_V2));
    }

    #[test]
    fn replace_links_finishes_loading_without_an_open_inventory_window() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.inventory = inventory();
        let mut main = InventoryUi::default();
        let mut window = main.take_listings_panel();
        window.state.begin_replace_links(Uuid::from_u128(8_400_102));
        main.put_listings_panel(window);
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                ..Default::default()
            },
            |ui| {
                super::super::show(
                    ui.ctx(),
                    &theme.palette(),
                    &Icons::default(),
                    &mut world,
                    &mut main,
                    &mut InventoryPreferences::default(),
                    &mut false,
                    false,
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    false,
                );
            },
        );
        output.textures_delta.clear();
        let panel = main.windows.iter().find(|w| w.id == PANEL).expect("hosted panel");
        assert!(!panel.state.fetch_all, "Replace links must leave its loading state");
        assert!(matches!(panel.state.dialog, Some(EditDialog::ReplaceLinks(..))));
    }

    #[test]
    fn a_native_right_click_menu_survives_the_next_inventory_frame() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.inventory = inventory();
        let mut main = InventoryUi::default();
        let mut prefs = InventoryPreferences::default();
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let item = Uuid::from_u128(8_400_102);
        let mut position = egui::Pos2::ZERO;
        fn has_text(shape: &egui::epaint::Shape, text: &str) -> bool {
            match shape {
                egui::epaint::Shape::Text(shape) => shape.galley.text() == text,
                egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|shape| has_text(shape, text)),
                _ => false,
            }
        }
        for frame in 0..5 {
            let events = if (1..=2).contains(&frame) {
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Secondary,
                        pressed: frame == 1,
                        modifiers: Default::default(),
                    },
                ]
            } else {
                Vec::new()
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    super::super::show(
                        ui.ctx(),
                        &p,
                        &Icons::default(),
                        &mut world,
                        &mut main,
                        &mut prefs,
                        &mut false,
                        false,
                        &Default::default(),
                        &Default::default(),
                        &Default::default(),
                        &Default::default(),
                        false,
                    );
                    let mut window = main.take_listings_panel();
                    let mut panel = Panel {
                        state: &mut window.state,
                        prefs: &mut prefs,
                        facts: &Facts::default(),
                        actions: Vec::new(),
                    };
                    let response = ui.add(egui::Label::new("Marketplace item").sense(egui::Sense::click()));
                    position = response.rect.center();
                    crate::ui::menu::context_menu(&response, &p, |ui| panel.menu(ui, &p, &world.inventory, item));
                    panel.finish(ui.ctx(), &world.inventory);
                    main.put_listings_panel(window);
                },
            );
            if frame >= 3 {
                assert!(
                    output
                        .shapes
                        .iter()
                        .any(|shape| has_text(&shape.shape, aurora_i18n::tr!("common-rename"))),
                    "menu vanished on frame {frame}"
                );
            }
            output.textures_delta.clear();
        }
    }
}
