//! LLFolderView::copy/cut/paste and LLInvFVBridge clipboard rules
//! (Firestorm indra/llui/llfolderview.cpp and newview/llinventorybridge.cpp,
//! originally LGPL 2.1). Menus and keyboard share the same inventory clipboard.

use super::*;

pub(super) fn focus_id(st: &InventoryUi) -> egui::Id {
    egui::Id::new(("inventory_clipboard_focus", st.ui_id))
}

/// A stable focus target survives scrolling selected rows out of the viewport.
/// Register the background before the rows so their clicks take precedence.
pub(super) fn list_focus(ui: &mut egui::Ui, st: &InventoryUi, height: f32) {
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
    let response = ui.interact(rect, focus_id(st), egui::Sense::click());
    if response.clicked() {
        response.request_focus();
    }
}

pub(super) fn copy_allowed(inv: &Inventory, ids: &[Uuid]) -> bool {
    !ids.is_empty()
        && ids.iter().all(|id| {
            inv.folders.contains_key(id)
                || inv
                    .items
                    .get(id)
                    .is_some_and(|it| it.owner_mask & rules::COPY != 0 || !matches!(it.asset_type, 2 | 49))
        })
}

pub(super) fn store(ctx: &egui::Context, st: &mut InventoryUi, ids: Vec<Uuid>, cut: bool) {
    if let Some(id) = ids.first() {
        // egui-winit translates Ctrl+V into Paste only for nonempty OS text.
        // Keep the UUID marker used by the context menu; never import UUIDs
        // supplied by external text into the inventory clipboard.
        ctx.copy_text(id.to_string());
    }
    st.clipboard = ids;
    st.cut = cut;
}

pub(super) fn paste(
    inv: &Inventory,
    to: Uuid,
    links: bool,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let cut = st.cut && !links;
    match rules::paste(inv, &st.clipboard, to, cut, links, &prefs.protected, &facts.worn) {
        Ok(change) => {
            st.consume_clipboard = cut;
            st.message.clear();
            actions.push(InvAction::Edit(change));
        }
        Err(reason) => {
            st.consume_clipboard = false;
            st.message = reason;
        }
    }
}

pub(super) fn shortcuts(
    ui: &mut egui::Ui,
    inv: &Inventory,
    root: Uuid,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    if !ui.is_enabled()
        || !ui.memory(|m| m.has_focus(focus_id(st)) && m.allows_interaction(ui.layer_id()))
        || ui.ctx().text_edit_focused()
        || egui::Popup::is_any_open(ui.ctx())
    {
        return;
    }
    // egui-winit consumes the physical C/X/V presses and emits these native
    // clipboard events instead. Testing key_pressed(C) would miss real input.
    let mut events = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if matches!(event, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)) {
                events.push(event.clone());
                false
            } else {
                true
            }
        });
    });
    for event in events {
        if st.pending.is_some() || actions.iter().any(|action| matches!(action, InvAction::Edit(_))) {
            continue;
        }
        match event {
            egui::Event::Copy | egui::Event::Cut => {
                let cut = matches!(event, egui::Event::Cut);
                let mut ids: Vec<_> = st.selection.iter().copied().collect();
                ids.sort();
                let allowed = if cut {
                    !ids.is_empty() && ids.iter().all(|id| rules::movable(inv, *id, &prefs.protected))
                } else {
                    copy_allowed(inv, &ids)
                };
                if allowed {
                    store(ui.ctx(), st, ids, cut);
                }
            }
            egui::Event::Paste(text) => {
                // Another application or text field can replace OS clipboard
                // text. Its contents must not paste an old inventory selection.
                if st.clipboard.first().is_none_or(|id| text.trim() != id.to_string()) {
                    continue;
                }
                let to = st.selected.filter(|id| st.selection.contains(id)).map_or(root, |id| {
                    if inv.folders.contains_key(&id) {
                        id
                    } else {
                        rules::parent(inv, id).unwrap_or(root)
                    }
                });
                if rules::paste_allowed(inv, &st.clipboard, to, st.cut, false, &prefs.protected, &facts.worn) {
                    paste(inv, to, false, st, prefs, facts, actions);
                }
            }
            _ => {}
        }
    }
}

fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_INVENTORY")
            .ok()
            .filter(|mode| mode.starts_with("clipboard"))
    })
    .as_deref()
}

pub(super) fn demo_row(response: &egui::Response, st: &InventoryUi, item: Uuid) {
    if demo_mode().is_some() {
        response
            .ctx
            .data_mut(|data| data.insert_temp(egui::Id::new(("clipboard_demo_row", st.ui_id, item)), response.rect.center()));
    }
}

pub(super) fn demo_caret(response: &egui::Response, st: &InventoryUi, folder: Uuid) {
    if demo_mode().is_some() {
        response
            .ctx
            .data_mut(|data| data.insert_temp(egui::Id::new(("clipboard_demo_caret", st.ui_id, folder)), response.rect.center()));
    }
}

/// Exercise actual rows with the same native events egui-winit emits for
/// Ctrl+C/X/V, without activating the OS window or reading OS clipboard text.
pub fn demo_events(ctx: &egui::Context, frame: u64, st: &mut InventoryUi) -> Vec<egui::Event> {
    let Some(mode) = demo_mode().filter(|mode| *mode != "clipboard") else {
        return Vec::new();
    };
    let window = mode == "clipboard-window";
    if frame == 640 {
        super::super::widgets::place_floater(ctx, "inventory", egui::pos2(20.0, 56.0), egui::vec2(680.0, 850.0));
        for (index, w) in st.windows.iter().enumerate() {
            super::super::widgets::place_floater(
                ctx,
                &format!("inventory_{}", w.id),
                egui::pos2(730.0, 56.0 + index as f32 * 460.0),
                egui::vec2(420.0, 440.0),
            );
        }
    }
    let source_ui = if window {
        st.windows.first().map(|w| w.id).unwrap_or(st.ui_id)
    } else {
        st.ui_id
    };
    let cut = matches!(mode, "clipboard-cut" | "clipboard-cut-folder");
    let folder = matches!(mode, "clipboard-folder" | "clipboard-cut-folder");
    if folder && frame == 785 {
        st.expand_all = Some(true);
        st.reveal = st.selected;
    }
    if folder && matches!(frame, 750 | 751) {
        return vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: frame == 750,
            repeat: false,
            modifiers: Default::default(),
        }];
    }
    if !window && matches!(frame, 780 | 781) {
        let to = if folder { st.selected } else { Some(Uuid::from_u128(8001)) };
        if let Some(pos) =
            to.and_then(|to| ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::new(("clipboard_demo_caret", st.ui_id, to)))))
        {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 780,
                    modifiers: Default::default(),
                },
            ];
        }
    }
    let row =
        |ui_id, item| ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::new(("clipboard_demo_row", ui_id, Uuid::from_u128(item)))));
    let (pos, ctrl) = match frame {
        650 | 651 => (
            row(
                source_ui,
                if folder {
                    8001
                } else if cut {
                    8109
                } else {
                    8100
                },
            ),
            false,
        ),
        665 | 666 if !window && !folder => (row(source_ui, if cut { 8110 } else { 8104 }), true),
        700 | 701 if window => (
            st.windows
                .get(1)
                .and_then(|w| ctx.read_response(focus_id(&w.state)))
                .map(|r| r.rect.center()),
            false,
        ),
        700 | 701 => (row(st.ui_id, if folder { 8300 } else { 8001 }), false),
        _ => (None, false),
    };
    if let Some(pos) = pos {
        let modifiers = egui::Modifiers {
            ctrl,
            command: ctrl,
            ..Default::default()
        };
        return vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(frame, 650 | 665 | 700),
                modifiers,
            },
        ];
    }
    if frame == 680 {
        return vec![if cut { egui::Event::Cut } else { egui::Event::Copy }];
    }
    if (frame == 730 || frame == 760 && mode == "clipboard-copy")
        && let Some(id) = st.clipboard.first()
    {
        return vec![egui::Event::Paste(id.to_string())];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, epaint::Shape};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    struct Harness {
        ctx: egui::Context,
        world: World,
        st: InventoryUi,
        prefs: InventoryPreferences,
        output: egui::FullOutput,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            world.agent_id = id(2);
            world.inventory.root = id(1);
            world.inventory.folders.insert(
                id(1),
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id: id(1),
                        type_default: 8,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
            crate::world::inventory::demo::seed(&mut world.inventory, world.agent_id);
            // Keep the clipboard fixture compact; unrelated outfit links would
            // otherwise fill the viewport before the rows under test.
            let folders = [id(1), id(8000), id(8001), id(8002), id(8003)];
            world.inventory.folders.retain(|id, _| folders.contains(id));
            world.inventory.items.retain(|id, _| (8100..=8113).contains(&id.as_u128()));
            for folder in world.inventory.folders.values_mut() {
                folder.children.retain(|id| folders.contains(id));
                folder.items.retain(|id| (8100..=8113).contains(&id.as_u128()));
            }
            super::super::super::widgets::place_floater(&ctx, "inventory", egui::pos2(900.0, 40.0), egui::vec2(680.0, 950.0));
            let mut harness = Self {
                ctx,
                world,
                st: InventoryUi {
                    expand_all: Some(true),
                    ..Default::default()
                },
                prefs: InventoryPreferences::default(),
                output: Default::default(),
                time: 0.0,
            };
            harness.st.show_original(&harness.world.inventory, id(8100));
            harness.st.expand_all = Some(true);
            for _ in 0..4 {
                harness.frame(Vec::new());
            }
            harness
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<InvAction> {
            self.time += 0.3;
            let mut actions = Vec::new();
            self.output = headless_output(self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                    events,
                    time: Some(self.time),
                    ..Default::default()
                },
                |ui| {
                    actions = show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &mut self.world,
                        &mut self.st,
                        &mut self.prefs,
                        &mut true,
                        false,
                        &Default::default(),
                    );
                },
            ));
            actions
        }

        fn row(&self, item: Uuid) -> Pos2 {
            self.row_in(item, "inventory")
        }

        fn row_in(&self, item: Uuid, window: &str) -> Pos2 {
            fn position(shape: &Shape, label: &str) -> Option<Pos2> {
                match shape {
                    Shape::Text(t) if t.galley.text().starts_with(label) => Some(t.pos + egui::vec2(4.0, 4.0)),
                    Shape::Vec(shapes) => shapes.iter().find_map(|shape| position(shape, label)),
                    _ => None,
                }
            }
            let label = rules::name(&self.world.inventory, item);
            let rect = self.ctx.memory(|m| m.area_rect(egui::Id::new(window))).expect("inventory window");
            self.output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| position(&shape.shape, &label).filter(|pos| rect.contains(*pos) && shape.clip_rect.contains(*pos)))
                .unwrap_or_else(|| panic!("missing visible row: {label}"))
        }

        fn click(&mut self, pos: Pos2, modifiers: egui::Modifiers) {
            for pressed in [true, false] {
                self.frame(vec![
                    Event::ModifiersChanged(modifiers),
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ]);
            }
        }

        fn select(&mut self, item: Uuid, ctrl: bool) {
            self.click(
                self.row(item),
                egui::Modifiers {
                    ctrl,
                    command: ctrl,
                    ..Default::default()
                },
            );
        }

        fn paste(&mut self) -> Vec<InvAction> {
            self.frame(vec![Event::Paste(self.st.clipboard[0].to_string())])
        }
    }

    #[test]
    fn native_copy_events_copy_every_selected_item_and_allow_repeated_paste() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.select(id(8104), true);
        assert_eq!(h.st.selection.len(), 2);
        assert!(h.frame(vec![Event::Copy]).is_empty());
        assert_eq!(h.st.clipboard, vec![id(8100), id(8104)]);
        assert!(!h.st.cut);
        assert!(
            h.output
                .platform_output
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if *text == id(8100).to_string()))
        );
        h.select(id(8001), false);
        for _ in 0..2 {
            let actions = h.paste();
            assert!(
                matches!(&actions[..], [InvAction::Edit(change)] if change.copies.len() == 2 && change.copies.iter().all(|copy| copy.parent == id(8001)))
            );
            assert!(!h.st.consume_clipboard);
        }
        assert_eq!(h.world.inventory.items[&id(8100)].parent, id(8000));
    }

    #[test]
    fn native_cut_moves_no_copy_and_no_transfer_items_without_removing_them_early() {
        let mut h = Harness::new();
        h.select(id(8109), false);
        h.select(id(8110), true);
        h.frame(vec![Event::Cut]);
        assert!(h.st.cut);
        assert_eq!(h.world.inventory.items[&id(8109)].parent, id(8000));
        h.select(id(8001), false);
        let actions = h.paste();
        assert!(h.st.consume_clipboard);
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("move mutation")
        };
        assert!(change.copies.is_empty());
        assert_eq!(change.operations.len(), 2);
        for op in &change.operations {
            assert!(
                matches!(op, aurora_net::inventory::operations::Operation::Patch { id: item, folder: false, body } if [id(8109), id(8110)].contains(item) && body["parent_id"].as_uuid() == id(8001))
            );
        }
    }

    #[test]
    fn folder_copy_includes_descendants_once_and_refuses_cycles() {
        let mut h = Harness::new();
        let mut item = h.world.inventory.items[&id(8104)].clone();
        item.id = id(8200);
        item.parent = id(8001);
        item.name = "Nested texture".into();
        h.world.inventory.add_items(vec![item]);
        h.frame(Vec::new());
        h.select(id(8001), false);
        h.select(id(8200), true);
        h.frame(vec![Event::Copy]);
        let actions = h.paste();
        assert!(actions.is_empty(), "cannot copy a folder into itself");
        h.select(id(8100), false); // Paste into this item's parent.
        let actions = h.paste();
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("folder copy")
        };
        assert_eq!(change.operations.len(), 1);
        assert_eq!(change.copies.len(), 1);
        assert_eq!(change.copies[0].item, id(8200));
        assert_eq!(h.world.inventory.items[&id(8200)].parent, id(8001));
    }

    #[test]
    fn keyboard_obeys_permissions_pending_mutations_and_external_clipboard_changes() {
        let mut h = Harness::new();
        h.select(id(8109), false);
        h.frame(vec![Event::Copy]); // Still available for Paste as link.
        h.select(id(8001), false);
        assert!(h.paste().is_empty(), "no-copy item cannot be duplicated");
        h.select(id(8100), false);
        h.prefs.protected.insert(id(8000));
        h.frame(vec![Event::Cut]);
        assert!(!h.st.cut);
        assert_eq!(h.st.clipboard, vec![id(8109)]);
        h.prefs.protected.clear();
        h.st.pending = Some(id(99));
        h.frame(vec![Event::Cut]);
        assert_eq!(h.st.clipboard, vec![id(8109)]);
        assert!(!h.st.cut);
        h.st.pending = None;
        h.frame(vec![Event::Copy]);
        h.select(id(8001), false);
        assert!(h.frame(vec![Event::Paste("external text".into())]).is_empty());
        assert!(h.frame(vec![Event::Paste(id(8104).to_string())]).is_empty());
        assert_eq!(h.st.clipboard, vec![id(8100)]);
    }

    #[test]
    fn text_fields_keep_copy_cut_and_paste_and_return_control_to_the_list_on_click() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.frame(vec![Event::Copy]);
        h.st.begin_rename(&h.world.inventory, id(8100));
        h.frame(Vec::new());
        assert!(h.ctx.text_edit_focused());
        h.frame(vec![Event::Copy]);
        assert_eq!(h.st.clipboard, vec![id(8100)]);
        assert!(
            h.output
                .platform_output
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if *text == rules::name(&h.world.inventory, id(8100))))
        );
        h.frame(vec![Event::Cut]);
        assert!(!h.st.cut);
        h.frame(vec![Event::Paste("New name".into())]);
        assert_eq!(h.st.rename.as_ref().expect("rename").name, "New name");
        h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        h.frame(Vec::new());
        h.select(id(8100), false);
        h.frame(vec![Event::Cut]);
        assert!(h.st.cut);
    }

    #[test]
    fn folder_windows_share_clipboard_and_only_the_focused_list_handles_paste() {
        let mut h = Harness::new();
        h.st.open_folder_window(id(8000));
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        let window = h.st.windows[0].id;
        h.ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new(format!("inventory_{window}")).with("place_pos"),
                egui::pos2(30.0, 50.0),
            )
        });
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.click(h.row_in(id(8100), &format!("inventory_{window}")), Default::default());
        assert_eq!(h.st.windows[0].state.selected, Some(id(8100)));
        h.frame(vec![Event::Copy]);
        assert_eq!(h.st.clipboard, vec![id(8100)]);
        h.st.open_folder_window(id(8001));
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        let dest = &h.st.windows[1].state;
        let root = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::new(format!("inventory_{}", dest.ui_id))))
            .expect("destination window");
        h.click(root.center(), Default::default());
        assert!(h.ctx.memory(|m| m.has_focus(focus_id(&h.st.windows[1].state))));
        let actions = h.paste();
        assert!(matches!(&actions[..], [InvAction::Edit(change)] if change.copies.len() == 1 && change.copies[0].parent == id(8001)));
        h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("unrelated_widget")));
        assert!(h.paste().is_empty());
    }
}
