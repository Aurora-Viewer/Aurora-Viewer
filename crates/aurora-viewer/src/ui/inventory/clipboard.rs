//! LLFolderView::copy/cut/paste and LLInvFVBridge clipboard rules
//! (Firestorm indra/llui/llfolderview.cpp and newview/llinventorybridge.cpp,
//! originally LGPL 2.1). Menus and keyboard share the same inventory clipboard.

use super::*;

pub(super) fn focus_id(st: &InventoryUi) -> egui::Id {
    egui::Id::unique(("inventory_clipboard_focus", st.ui_id))
}

pub(super) fn request_focus(ui: &egui::Ui, st: &mut InventoryUi) {
    st.list_focus_frame = Some(ui.ctx().cumulative_frame_nr());
    ui.memory_mut(|m| {
        m.request_focus(focus_id(st));
        m.set_focus_lock_filter(
            focus_id(st),
            egui::EventFilter {
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
}

pub(super) fn list_has_focus(ui: &egui::Ui, st: &InventoryUi) -> bool {
    ui.is_enabled()
        && st.dialog.is_none()
        && ui.memory(|m| m.has_focus(focus_id(st)) && m.allows_interaction(ui.layer_id()))
        && !ui.ctx().text_edit_focused()
        && !egui::Popup::is_any_open(ui.ctx())
}

/// A stable focus target survives scrolling selected rows out of the viewport.
/// Register the background before the rows so their clicks take precedence.
pub(super) fn list_focus(ui: &mut egui::Ui, st: &mut InventoryUi, height: f32) {
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
    let response = ui.interact(rect, focus_id(st), egui::Sense::click());
    let leaving = ui.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_none_or(|pos| !rect.contains(pos))
            || i.key_pressed(egui::Key::Tab)
            || i.key_pressed(egui::Key::Escape)
    });
    if leaving {
        st.list_focus_frame = None;
    }
    // A discarded sizing pass can omit the list and trigger egui's focus
    // dead-man switch. Restore only its immediately preceding focus request,
    // with no other focused widget, click outside, Tab, Escape or modal.
    let restore = !leaving
        && ui.is_enabled()
        && st.dialog.is_none()
        && !egui::Popup::is_any_open(ui.ctx())
        && st.list_focus_frame.is_some_and(|frame| ui.ctx().cumulative_frame_nr() <= frame + 1)
        && ui.memory(|m| m.focused().is_none() && m.allows_interaction(ui.layer_id()));
    if response.clicked() || response.has_focus() || restore {
        request_focus(ui, st);
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
    if !links && crate::world::marketplace::root(inv).is_some_and(|root| rules::under(inv, to, root)) {
        actions.push(InvAction::MarketplacePaste {
            to,
            ids: st.clipboard.clone(),
            copy: !cut,
        });
        return;
    }
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
    if !list_has_focus(ui, st) {
        return;
    }
    if !actions.iter().any(|action| matches!(action, InvAction::Edit(_))) {
        super::delete::shortcut(ui, inv, st, prefs, facts);
    }
    if st.dialog.is_some() {
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
        std::env::var("AURORA_DEMO_INVENTORY").ok().filter(|mode| {
            mode.starts_with("clipboard")
                || mode.starts_with("search-selection")
                || mode.starts_with("keyboard")
                || mode.starts_with("aurora-keyboard")
        })
    })
    .as_deref()
}

pub(super) fn demo_row(response: &egui::Response, st: &InventoryUi, item: Uuid) {
    if demo_mode().is_some() || super::delete::demo_enabled() || crate::demo::texture_library::inventory_drag_enabled() {
        response
            .ctx
            .data_mut(|data| data.insert_temp(egui::Id::unique(("clipboard_demo_row", st.ui_id, item)), response.rect.center()));
    }
}

pub(super) fn demo_caret(response: &egui::Response, st: &InventoryUi, folder: Uuid) {
    if demo_mode().is_some() {
        response
            .ctx
            .data_mut(|data| data.insert_temp(egui::Id::unique(("clipboard_demo_caret", st.ui_id, folder)), response.rect.center()));
    }
}

pub(super) fn demo_search(response: &egui::Response, st: &InventoryUi) {
    if demo_mode().is_some_and(|mode| mode.starts_with("search-selection")) {
        response.ctx.data_mut(|data| {
            data.insert_temp(egui::Id::unique(("inventory_demo_search", st.ui_id)), response.rect);
        });
    }
}

/// Exercise actual rows with the same native events egui-winit emits for
/// Ctrl+C/X/V, without activating the OS window or reading OS clipboard text.
pub fn demo_events(ctx: &egui::Context, frame: u64, st: &mut InventoryUi) -> Vec<egui::Event> {
    let Some(mode) = demo_mode().filter(|mode| *mode != "clipboard") else {
        return Vec::new();
    };
    if mode.starts_with("keyboard") || mode.starts_with("aurora-keyboard") {
        if matches!(frame, 650 | 651)
            && let Some(pos) =
                ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", st.ui_id, Uuid::from_u128(8100)))))
        {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 650,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
        }
        if matches!(frame, 665 | 680 | 710) {
            log::info!(
                "demo inventory keyboard input: focused={:?}, list_focus={}, text={}, popup={}, window_focus={}",
                ctx.memory(|m| m.focused()),
                ctx.memory(|m| m.has_focus(focus_id(st))),
                ctx.text_edit_focused(),
                egui::Popup::is_any_open(ctx),
                ctx.input(|i| i.focused)
            );
            return vec![
                egui::Event::ModifiersChanged(egui::Modifiers::SHIFT),
                egui::Event::Key {
                    key: if frame == 710 { egui::Key::ArrowUp } else { egui::Key::ArrowDown },
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                },
            ];
        }
        if matches!(frame, 690 | 720) {
            log::info!(
                "demo inventory keyboard: frame={frame}, selected_count={}, active={:?}, visible={}",
                st.selection.len(),
                st.selected,
                st.selected.is_some_and(|id| st.visible.contains(&id))
            );
        }
        return Vec::new();
    }
    if mode.starts_with("search-selection") {
        let keyboard = mode == "search-selection-keyboard";
        let pos = match frame {
            650 | 651 => ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", st.ui_id, Uuid::from_u128(11490))))),
            720 | 721 => ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("inventory_demo_search", st.ui_id))))
                .map(|rect| {
                    if keyboard {
                        rect.left_center() + egui::vec2(15.0, 0.0)
                    } else {
                        rect.right_center() - egui::vec2(14.0, 0.0)
                    }
                }),
            _ => None,
        };
        if let Some(pos) = pos {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 650 || frame == 720,
                    modifiers: Default::default(),
                },
            ];
        }
        if keyboard && matches!(frame, 730 | 731) {
            return vec![egui::Event::Key {
                key: if frame == 730 { egui::Key::A } else { egui::Key::Backspace },
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: if frame == 730 {
                    egui::Modifiers::COMMAND
                } else {
                    egui::Modifiers::NONE
                },
            }];
        }
        if frame == 800 {
            log::info!(
                "demo inventory search selection: query_empty={}, selected={:?}, visible={}",
                st.search.is_empty(),
                st.selected,
                st.selected.is_some_and(|id| st.visible.contains(&id))
            );
        }
        return Vec::new();
    }
    let window = mode == "clipboard-window";
    if mode == "clipboard-progress" {
        super::progress::demo(frame, st);
        return Vec::new();
    }
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
    if !window && matches!(frame, 780 | 781) {
        let to = if folder { st.selected } else { Some(Uuid::from_u128(8001)) };
        if let Some(pos) =
            to.and_then(|to| ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_caret", st.ui_id, to)))))
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
        |ui_id, item| ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", ui_id, Uuid::from_u128(item)))));
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
        classic: bool,
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
                classic: true,
            };
            harness.st.show_original(&harness.world.inventory, id(8100));
            harness.st.expand_all = Some(true);
            for _ in 0..4 {
                harness.frame(Vec::new());
            }
            harness
        }

        fn frame(&mut self, mut events: Vec<Event>) -> Vec<InvAction> {
            if let Some(modifiers) = events.iter().find_map(|event| match event {
                Event::Key {
                    key: egui::Key::ArrowDown | egui::Key::ArrowUp,
                    modifiers,
                    ..
                } => Some(*modifiers),
                _ => None,
            }) {
                events.insert(0, Event::ModifiersChanged(modifiers));
            }
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
                        &Default::default(),
                        &Default::default(),
                        &Default::default(),
                        self.classic,
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
            let label = if self.world.inventory.folders.contains_key(&item) {
                folder_name(&self.world.inventory, item)
            } else {
                rules::name(&self.world.inventory, item)
            };
            let rect = self
                .ctx
                .memory(|m| m.area_rect(egui::Id::unique(window)))
                .expect("inventory window");
            let state = self
                .st
                .windows
                .iter()
                .find(|w| window == format!("inventory_{}", w.id))
                .map_or(&self.st, |w| &*w.state);
            let list = self.ctx.read_response(focus_id(state)).expect("inventory list").rect;
            let row = self
                .ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("inventory_drag_row", state.ui_id, item))))
                .expect("drawn row");
            self.output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| {
                    position(&shape.shape, &label)
                        .filter(|pos| rect.contains(*pos) && list.contains(*pos) && row.contains(*pos) && shape.clip_rect.contains(*pos))
                })
                .unwrap_or_else(|| panic!("missing visible row: {label}"))
        }

        fn click(&mut self, pos: Pos2, modifiers: egui::Modifiers) -> Vec<InvAction> {
            let mut actions = Vec::new();
            for pressed in [true, false] {
                actions.extend(self.frame(vec![
                    Event::ModifiersChanged(modifiers),
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ]));
            }
            actions
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

        fn text(&self, label: &str) -> egui::Rect {
            fn rect(shape: &Shape, label: &str) -> Option<egui::Rect> {
                match shape {
                    Shape::Text(t) if t.galley.text() == label => Some(t.galley.rect.translate(t.pos.to_vec2())),
                    Shape::Vec(shapes) => shapes.iter().find_map(|shape| rect(shape, label)),
                    _ => None,
                }
            }
            self.output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| rect(&shape.shape, label))
                .expect("visible label")
        }

        fn trash_button(&self, ui_id: Uuid) -> egui::Rect {
            self.ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("inventory_delete_button", ui_id))))
                .expect("trash button")
        }

        fn add_trash(&mut self) -> Uuid {
            let trash = id(8020);
            let mut folder = self.world.inventory.folders[&id(8001)].clone();
            folder.info.id = trash;
            folder.info.parent = id(1);
            folder.info.name = "Trash".into();
            folder.info.type_default = 14;
            folder.children.clear();
            folder.items.clear();
            self.world.inventory.folders.insert(trash, folder);
            self.world.inventory.folders.get_mut(&id(1)).expect("root").children.push(trash);
            self.world.inventory.generation += 1;
            trash
        }
    }

    fn delete_key(modifiers: egui::Modifiers) -> Event {
        Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn arrow(down: bool, shift: bool) -> Event {
        Event::Key {
            key: if down { egui::Key::ArrowDown } else { egui::Key::ArrowUp },
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE },
        }
    }

    #[test]
    fn inventory_keyboard_extends_shrinks_and_copies_in_every_layout() {
        for (classic, width) in [(true, 680.0), (false, 440.0), (false, 800.0), (false, 1180.0)] {
            let mut h = Harness::new();
            h.classic = classic;
            super::super::super::widgets::place_floater(&h.ctx, "inventory", egui::pos2(10.0, 40.0), egui::vec2(width, 950.0));
            h.st.show_original(&h.world.inventory, id(8100));
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            let order = if !classic && width >= 800.0 {
                &h.st.flat.order
            } else {
                &h.st.plain.order
            };
            let siblings: Vec<_> = order
                .iter()
                .copied()
                .filter(|row| h.world.inventory.items.contains_key(row))
                .collect();
            let [first, second, third, ..] = siblings[..] else {
                panic!("three item rows")
            };
            h.select(first, false);
            assert!(
                h.ctx.memory(|m| m.has_focus(focus_id(&h.st))),
                "the clicked row gives the list focus"
            );
            assert!(!h.ctx.text_edit_focused(), "the list is not a text field");
            assert!(!egui::Popup::is_any_open(&h.ctx), "no popup takes the arrows");
            let before = h.row(first);
            h.frame(vec![arrow(true, true), arrow(true, true)]);
            assert_eq!(h.st.selected, Some(third), "{classic}, {width}");
            assert_eq!(h.st.selection, HashSet::from([first, second, third]));
            assert_eq!(
                h.row(first),
                before,
                "visible rows stay in place when using the arrows ({classic}, {width})"
            );
            h.frame(vec![arrow(false, true)]);
            assert_eq!(h.st.selected, Some(second));
            assert_eq!(h.st.selection, HashSet::from([first, second]));
            h.frame(vec![Event::Copy]);
            assert_eq!(h.st.clipboard.iter().copied().collect::<HashSet<_>>(), h.st.selection);
            h.frame(vec![arrow(false, false)]);
            assert_eq!(h.st.selected, Some(first));
            assert_eq!(h.st.selection, HashSet::from([first]));
            h.select(third, true);
            h.frame(vec![arrow(false, true)]);
            assert_eq!(
                h.st.selection,
                HashSet::from([second, third]),
                "keyboard restarts at the last Ctrl-click"
            );
        }
    }

    #[test]
    fn inventory_keyboard_scrolls_filtered_virtual_rows_and_keeps_text_focus() {
        let mut h = Harness::new();
        let original = h.world.inventory.items[&id(8100)].clone();
        h.world.inventory.add_items(
            (0..90)
                .map(|n| aurora_net::inventory::InvItem {
                    id: id(20_000 + n),
                    name: format!("Keyboard {n:03}"),
                    ..original.clone()
                })
                .collect(),
        );
        h.st.search = "Keyboard".into();
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.select(id(20_000), false);
        h.frame((0..60).map(|_| arrow(true, true)).collect());
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        assert_eq!(h.st.selected, Some(id(20_060)));
        assert_eq!(h.st.selection.len(), 61);
        h.row(id(20_060)); // The active row is actually drawn inside the viewport.
        h.st.begin_rename(&h.world.inventory, id(20_060));
        h.frame(Vec::new());
        assert!(h.ctx.text_edit_focused());
        h.frame(vec![arrow(false, true)]);
        assert_eq!(h.st.selection, HashSet::from([id(20_060)]));
        h.st.rename = None;
        h.ctx.memory_mut(|m| m.request_focus(egui::Id::unique("other_field")));
        h.frame(vec![arrow(false, true)]);
        assert_eq!(h.st.selected, Some(id(20_060)));
        h.st.show_original(&h.world.inventory, id(20_000));
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.select(id(20_000), false);
        h.frame((0..60).map(|_| arrow(true, true)).collect());
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        assert_eq!(h.st.selected, Some(id(20_060)));
        assert_eq!(h.st.selection.len(), 61);
        h.row(id(20_060)); // The unfiltered tree shares the same minimal scroll.
    }

    #[test]
    fn inventory_world_drop_releases_keyboard_without_closing_or_restoring_list_focus() {
        for classic in [false, true] {
            let mut h = Harness::new();
            h.classic = classic;
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h.select(id(8100), false);
            assert!(h.ctx.egui_wants_keyboard_input());
            let selection = h.st.selection.clone();
            h.st.release_world_drag_focus(&h.ctx);
            for _ in 0..4 {
                h.frame(Vec::new());
                assert!(!h.ctx.egui_wants_keyboard_input(), "movement keys belong to the scene");
                assert_eq!(h.st.selection, selection);
            }
            assert!(h.ctx.memory(|m| m.area_rect(egui::Id::unique("inventory"))).is_some());
        }
    }

    #[test]
    fn inventory_keyboard_uses_only_the_focused_folder_window() {
        let mut h = Harness::new();
        h.st.open_folder_window(id(8000));
        let window = h.st.windows[0].id;
        super::super::super::widgets::place_floater(
            &h.ctx,
            &format!("inventory_{window}"),
            egui::pos2(20.0, 50.0),
            egui::vec2(600.0, 800.0),
        );
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        let order = &h.st.windows[0].state.plain.order;
        let siblings: Vec<_> = order
            .iter()
            .copied()
            .filter(|row| h.world.inventory.items.contains_key(row))
            .collect();
        let first = siblings[0];
        let second = siblings[1];
        let main = h.st.selection.clone();
        h.click(h.row_in(first, &format!("inventory_{window}")), Default::default());
        h.frame(vec![arrow(true, true)]);
        assert_eq!(h.st.windows[0].state.selection, HashSet::from([first, second]));
        assert_eq!(h.st.selection, main);
        h.st.windows[0].state.dialog = Some(EditDialog::Delete(vec![first], false));
        h.frame(vec![arrow(true, true)]);
        assert_eq!(h.st.windows[0].state.selection, HashSet::from([first, second]));
    }

    #[test]
    fn delete_key_confirms_and_moves_selected_folder_with_its_contents_and_no_copy_item() {
        let mut h = Harness::new();
        let trash = h.add_trash();
        let mut child = h.world.inventory.items[&id(8104)].clone();
        child.parent = id(8001);
        h.world.inventory.server_updated(Vec::new(), vec![child]);
        h.st.expand_all = Some(true);
        h.frame(Vec::new());
        h.select(id(8001), false);
        h.select(id(8104), true);
        h.select(id(8109), true);
        assert!(h.frame(vec![delete_key(Default::default())]).is_empty());
        assert!(matches!(&h.st.dialog, Some(EditDialog::Delete(ids, false)) if ids.len() == 3));
        h.frame(Vec::new());
        let actions = h.click(h.text(aurora_i18n::tr!("common-delete")).center(), Default::default());
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("confirmed deletion must submit one mutation")
        };
        assert!(change.move_only());
        assert_eq!(change.operations.len(), 2, "a selected descendant moves with its folder once");
        rules::demo_mutate(&mut h.world.inventory, h.world.agent_id, change.clone()).expect("delete");
        assert_eq!(h.world.inventory.folders[&id(8001)].info.parent, trash);
        assert_eq!(h.world.inventory.items[&id(8104)].parent, id(8001));
        assert_eq!(h.world.inventory.items[&id(8109)].parent, trash);
        assert_eq!(h.world.inventory.items[&id(8100)].parent, id(8000));
    }

    #[test]
    fn trash_button_is_right_of_loaded_count_and_opens_same_confirmation() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        let button = h.trash_button(h.st.ui_id);
        let count = h.text(&aurora_i18n::tr!(
            "ui-inventory-objects-loaded",
            value0 = format!("{}", h.world.inventory.item_count())
        ));
        assert!(button.left() > count.right());
        assert!(button.left() - count.right() < 18.0, "the count sits beside the trash button");
        assert!((button.center().y - count.center().y).abs() < 3.0);
        assert!(h.click(button.center(), Default::default()).is_empty());
        assert!(matches!(&h.st.dialog, Some(EditDialog::Delete(ids, false)) if *ids == vec![id(8100)]));
        h.frame(Vec::new());
        assert!(
            h.click(h.text(aurora_i18n::tr!("common-cancel")).center(), Default::default())
                .is_empty()
        );
        assert!(h.st.dialog.is_none());
        assert_eq!(h.world.inventory.items[&id(8100)].parent, id(8000));
    }

    #[test]
    fn delete_keeps_text_input_priority_and_blocks_modified_keys_pending_and_protected_selection() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.frame(vec![delete_key(egui::Modifiers::CTRL)]);
        assert!(h.st.dialog.is_none());
        for item in [id(1), id(8003)] {
            h.select(item, false);
            h.frame(vec![delete_key(Default::default())]);
            h.click(h.trash_button(h.st.ui_id).center(), Default::default());
            assert!(h.st.dialog.is_none(), "system and library folders are protected");
        }
        h.st.selection.clear();
        h.frame(Vec::new());
        h.click(h.trash_button(h.st.ui_id).center(), Default::default());
        assert!(h.st.dialog.is_none());
        h.select(id(8100), false);
        h.st.pending = Some(Uuid::new_v4());
        h.frame(vec![delete_key(Default::default())]);
        h.click(h.trash_button(h.st.ui_id).center(), Default::default());
        assert!(h.st.dialog.is_none());
        h.st.pending = None;
        h.prefs.protected.insert(id(8000));
        h.select(id(8100), false);
        h.frame(vec![delete_key(Default::default())]);
        h.click(h.trash_button(h.st.ui_id).center(), Default::default());
        assert!(h.st.dialog.is_none());
        h.prefs.protected.clear();
        let mut worn = Facts::from_world(&h.world, false);
        worn.worn.insert(id(8100));
        assert!(!super::super::delete::allowed(&h.world.inventory, &[id(8100)], &h.prefs, &worn));
        assert!(!super::super::delete::allowed(&h.world.inventory, &[id(8000)], &h.prefs, &worn));
        h.select(id(8100), false);
        h.st.begin_rename(&h.world.inventory, id(8100));
        h.frame(Vec::new());
        assert!(h.ctx.text_edit_focused());
        h.frame(vec![delete_key(Default::default())]);
        assert!(h.st.dialog.is_none());
        assert_eq!(
            h.st.rename.as_ref().expect("rename").name,
            "",
            "Delete edits the selected name text"
        );
    }

    #[test]
    fn folder_window_delete_uses_its_own_selection_and_trash_items_request_purge() {
        let mut h = Harness::new();
        let trash = h.add_trash();
        let mut item = h.world.inventory.items[&id(8100)].clone();
        item.parent = trash;
        h.world.inventory.server_updated(Vec::new(), vec![item]);
        h.st.open_folder_window(trash);
        let window = h.st.windows[0].id;
        super::super::super::widgets::place_floater(
            &h.ctx,
            &format!("inventory_{window}"),
            egui::pos2(30.0, 50.0),
            egui::vec2(420.0, 440.0),
        );
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h.click(h.row_in(id(8100), &format!("inventory_{window}")), Default::default());
        h.frame(vec![delete_key(Default::default())]);
        assert!(h.st.dialog.is_none());
        assert!(matches!(&h.st.windows[0].state.dialog, Some(EditDialog::Delete(ids, true)) if *ids == vec![id(8100)]));
        h.frame(Vec::new());
        let actions = h.click(h.text(aurora_i18n::tr!("ui-inventory-dialogs-purge")).center(), Default::default());
        assert!(
            matches!(&actions[..], [InvAction::Edit(change)] if change.removed == vec![id(8100)] && matches!(&change.operations[..], [aurora_net::inventory::operations::Operation::Delete { id: item, folder: false }] if *item == id(8100)))
        );
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
    fn real_shortcuts_can_copy_again_and_cut_a_no_copy_item_while_a_copy_is_arriving() {
        let mut h = Harness::new();
        h.select(id(8100), false);
        h.frame(vec![Event::Copy]);
        h.select(id(8001), false);
        let actions = h.paste();
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("first copy")
        };
        h.st.begin_mutation(id(99), change);
        h.st.watch_copy(
            id(99),
            &aurora_net::inventory::operations::Reply {
                contents: Vec::new(),
                folders: Vec::new(),
                moves_queued: false,
                refresh: change.refresh.clone(),
                copies: change.copies.clone(),
                removed: Vec::new(),
                created: Default::default(),
            },
        );
        assert!(h.st.busy(), "copy remains visibly in progress");
        assert!(
            matches!(&h.paste()[..], [InvAction::Edit(change)] if change.copies.len() == 1),
            "Ctrl+V remains available"
        );
        h.select(id(8109), false);
        h.frame(vec![Event::Cut]);
        h.select(id(8001), false);
        assert!(
            matches!(&h.paste()[..], [InvAction::Edit(change)] if change.move_only()),
            "Ctrl+X/V can move a no-copy item during another copy"
        );
        assert_eq!(
            h.world.inventory.items[&id(8100)].parent,
            id(8000),
            "copy never consumes the original"
        );
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
                egui::Id::unique(format!("inventory_{window}")).with("place_pos"),
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
            .memory(|m| m.area_rect(egui::Id::unique(format!("inventory_{}", dest.ui_id))))
            .expect("destination window");
        h.click(root.center(), Default::default());
        assert!(h.ctx.memory(|m| m.has_focus(focus_id(&h.st.windows[1].state))));
        let actions = h.paste();
        assert!(matches!(&actions[..], [InvAction::Edit(change)] if change.copies.len() == 1 && change.copies[0].parent == id(8001)));
        h.ctx.memory_mut(|m| m.request_focus(egui::Id::unique("unrelated_widget")));
        assert!(h.paste().is_empty());
    }
}
