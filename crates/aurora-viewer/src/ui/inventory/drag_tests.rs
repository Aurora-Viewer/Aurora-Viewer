//! Real row selection/drag/release, including nested and cross-window targets.
use super::*;
use aurora_net::inventory::operations::Operation;

struct Fixture {
    ctx: egui::Context,
    inv: Inventory,
    state: InventoryUi,
    other: InventoryUi,
    prefs: InventoryPreferences,
    facts: Facts,
    time: f64,
    actions: Vec<InvAction>,
    two_windows: bool,
    scrolling: Option<bool>,
}

impl Fixture {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.created_folders(
            [
                (1, 0, "Inventory", 8),
                (10, 1, "A Sources", -1),
                (20, 1, "B Destination", -1),
                (21, 20, "Nested", -1),
            ]
            .map(|(n, parent, name, kind)| aurora_net::inventory::InvFolder {
                id: id(n),
                parent: id(parent),
                name: name.into(),
                type_default: kind,
                ..Default::default()
            })
            .to_vec(),
        );
        for folder in inv.folders.values_mut() {
            folder.state = FetchState::Fetched;
        }
        inv.add_items(
            (100..103)
                .map(|n| aurora_net::inventory::InvItem {
                    id: id(n),
                    parent: id(10),
                    asset_type: if n == 102 { 57 } else { 0 },
                    name: format!("Item {n}"),
                    owner: id(2),
                    owner_mask: rules::COPY | rules::MODIFY | rules::TRANSFER,
                    ..Default::default()
                })
                .collect(),
        );
        let mut state = InventoryUi {
            ui_id: id(800),
            ..Default::default()
        };
        state.reveal_path.insert(id(10));
        state.reveal = Some(id(100));
        let mut f = Self {
            ctx,
            inv,
            state,
            other: InventoryUi {
                ui_id: id(801),
                ..Default::default()
            },
            prefs: InventoryPreferences::default(),
            facts: Facts {
                agent: id(2),
                worn: Default::default(),
                active_gestures: Default::default(),
                worn_labels: Default::default(),
                worn_at: Default::default(),
                points: Vec::new(),
                appearance_busy: false,
                touchable: Default::default(),
                names: Default::default(),
                ..Default::default()
            },
            time: 0.0,
            actions: Vec::new(),
            two_windows: false,
            scrolling: None,
        };
        f.settle();
        f
    }
    fn frame(&mut self, mut events: Vec<egui::Event>, ctrl: bool) -> egui::CursorIcon {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let p = crate::theme::Theme::default().palette();
        events.insert(0, egui::Event::ModifiersChanged(modifiers(ctrl)));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                ui.horizontal_top(|ui| {
                    draw_tree(
                        ui,
                        &p,
                        &mut self.inv,
                        &mut self.state,
                        &mut self.prefs,
                        &self.facts,
                        &mut self.actions,
                        id(1),
                        self.scrolling,
                    );
                    if self.two_windows {
                        draw_tree(
                            ui,
                            &p,
                            &mut self.inv,
                            &mut self.other,
                            &mut self.prefs,
                            &self.facts,
                            &mut self.actions,
                            id(20),
                            self.scrolling,
                        );
                    }
                });
                // Other UI is drawn after the list: resolve feedback last.
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                if let Some(cursor) = drag::cursor(ui.ctx()) {
                    ui.ctx().set_cursor_icon(cursor);
                }
                drag::badge(ui.ctx(), &p);
            },
        );
        output.textures_delta.clear();
        output.platform_output.cursor_icon
    }
    fn settle(&mut self) {
        for _ in 0..6 {
            self.frame(Vec::new(), false);
        }
    }
    fn row_in(&self, n: u128, editor: Uuid) -> egui::Pos2 {
        let rect = self
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("inventory_drag_row", editor, id(n)))))
            .expect("visible row");
        rect.min + Vec2::new(25.0, rect.height() * 0.5)
    }
    fn row(&self, n: u128) -> egui::Pos2 {
        self.row_in(n, self.state.ui_id)
    }
    fn button(&mut self, pos: egui::Pos2, pressed: bool, ctrl: bool) {
        self.frame(
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: modifiers(ctrl),
                },
            ],
            ctrl,
        );
    }
    fn click(&mut self, n: u128, ctrl: bool) {
        let pos = self.row(n);
        self.button(pos, true, ctrl);
        self.button(pos, false, ctrl);
    }
    fn start(&mut self, n: u128) {
        let pos = self.row(n);
        self.button(pos, true, false);
        self.frame(vec![egui::Event::PointerMoved(pos + Vec2::new(50.0, 0.0))], false);
    }
    fn hover(&mut self, pos: egui::Pos2) -> egui::CursorIcon {
        self.frame(vec![egui::Event::PointerMoved(pos)], false)
    }
    fn moved(&self) -> Vec<(Uuid, Uuid)> {
        self.actions
            .iter()
            .flat_map(|a| match a {
                InvAction::Edit(change) => change
                    .operations
                    .iter()
                    .map(|op| match op {
                        Operation::Patch { id, body, .. } => (*id, body["parent_id"].as_uuid()),
                        _ => panic!("move must retain the original item"),
                    })
                    .collect::<Vec<_>>(),
                _ => panic!("unexpected side effect"),
            })
            .collect()
    }
}

fn draw_tree(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    root: Uuid,
    scrolling: Option<bool>,
) {
    ui.push_id(st.ui_id, |ui| {
        ui.vertical(|ui| {
            ui.set_width(480.0);
            let zone = egui::Frame::new().show(ui, |ui| {
                ui.set_min_size(Vec2::new(480.0, 550.0));
                st.visible.clear();
                if scrolling == Some(true) {
                    st.view
                        .search(inv, root, "", prefs, &st.filters, 0, None, facts, facts.agent, view::now());
                    prepare_result_tree(st, inv, root);
                    search_scroll(ui, p, &Icons::default(), inv, st, prefs, facts, actions, 170.0);
                } else {
                    let height = if scrolling.is_some() { 170.0 } else { 550.0 };
                    rows::scroll(ui, p, &Icons::default(), inv, st, prefs, facts, actions, height, root, true);
                }
            });
            drag::target(ui, p, &zone.response, inv, root, st, prefs, facts, actions);
        });
    });
}
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn modifiers(ctrl: bool) -> egui::Modifiers {
    egui::Modifiers {
        ctrl,
        command: ctrl,
        ..Default::default()
    }
}

#[path = "drag_navigation_tests.rs"]
mod navigation_tests;

#[test]
fn inventory_drag_ctrl_selection_moves_both_textures_once_without_changing_the_model_before_ack() {
    let mut f = Fixture::new();
    f.click(100, false);
    f.click(101, true);
    assert_eq!(f.state.selection, HashSet::from([id(100), id(101)]));
    f.start(100);
    assert!(
        !egui::DragAndDrop::has_payload_of_type::<InvDrag>(&f.ctx),
        "a batch must not be accepted by single-item destinations"
    );
    assert_eq!(f.state.selection.len(), 2);
    let to = f.row(20);
    assert_eq!(f.hover(to), egui::CursorIcon::Move);
    assert!(f.actions.is_empty(), "hover must not send a mutation");
    f.button(to, false, false);
    assert_eq!(f.moved(), vec![(id(100), id(20)), (id(101), id(20))]);
    assert_eq!(f.inv.items[&id(100)].parent, id(10));
    assert_eq!(f.inv.items[&id(101)].parent, id(10));
    f.settle();
    assert_eq!(f.actions.len(), 1, "one batch, no repeated release");
}

#[test]
fn inventory_drag_rejects_the_whole_batch_when_a_source_or_destination_is_protected_or_busy() {
    for rejection in [0, 2, 3] {
        let mut f = Fixture::new();
        f.click(100, false);
        f.click(101, true);
        f.start(100);
        let to = f.row(20);
        assert_eq!(f.hover(to), egui::CursorIcon::Move);
        match rejection {
            0 => {
                f.prefs.protected.insert(id(101));
            }
            2 => {
                f.state.pending = Some(id(999));
            }
            _ => {
                f.inv.items.remove(&id(101));
                f.inv.generation += 1;
            }
        }
        assert_eq!(f.hover(to), egui::CursorIcon::NotAllowed);
        f.button(to, false, false);
        assert!(f.actions.is_empty(), "a rejected row must not fall through to the enclosing root");
        assert!(!f.state.message.is_empty());
    }
}

#[test]
fn inventory_drag_can_move_items_into_a_protected_folder() {
    let mut f = Fixture::new();
    f.click(100, false);
    f.start(100);
    let to = f.row(20);
    f.prefs.protected.insert(id(20));
    assert_eq!(f.hover(to), egui::CursorIcon::Move);
    f.button(to, false, false);
    assert_eq!(f.actions.len(), 1);
}

#[test]
fn inventory_drag_single_and_unselected_rows_keep_single_item_world_payloads() {
    for selected in [false, true] {
        let mut f = Fixture::new();
        if selected {
            f.click(100, false);
            f.click(101, true);
        }
        f.start(102);
        assert_eq!(egui::DragAndDrop::payload::<InvDrag>(&f.ctx).expect("single cargo").0, id(102));
        assert_eq!(f.state.selection, HashSet::from([id(102)]));
        let to = f.row(20);
        f.hover(to);
        f.button(to, false, false);
        assert_eq!(f.moved(), vec![(id(102), id(20))]);
    }
}

#[test]
fn inventory_drag_moves_no_copy_items_and_links_without_copying_or_resolving_them() {
    let mut f = Fixture::new();
    f.inv.items.get_mut(&id(100)).unwrap().owner_mask = 0;
    let link = f.inv.items.get_mut(&id(101)).unwrap();
    link.asset_type = 24;
    link.asset_id = id(102);
    f.click(100, false);
    f.click(101, true);
    f.start(100);
    let to = f.row(20);
    f.hover(to);
    f.button(to, false, false);
    assert_eq!(f.moved(), vec![(id(100), id(20)), (id(101), id(20))]);
    assert_eq!(f.inv.items[&id(102)].parent, id(10));
}

#[test]
fn inventory_drag_moves_saved_outfits_into_my_outfits_groups() {
    let mut f = Fixture::new();
    f.inv.created_folders(
        [
            (700, 1, "My Outfits", 48),
            (740, 700, "Wardrobe", -1),
            (744, 700, "Saved outfit", 47),
        ]
        .map(|(n, parent, name, kind)| aurora_net::inventory::InvFolder {
            id: id(n),
            parent: id(parent),
            name: name.into(),
            type_default: kind,
            ..Default::default()
        })
        .to_vec(),
    );
    for folder in f.inv.folders.values_mut() {
        folder.state = FetchState::Fetched;
    }
    f.state.reveal_path.insert(id(700));
    f.settle();
    f.start(744);
    assert_eq!(
        egui::DragAndDrop::payload::<FolderDrag>(&f.ctx).expect("outfit folder cargo").0,
        id(744)
    );
    let to = f.row(740);
    assert_ne!(f.hover(to), egui::CursorIcon::NotAllowed);
    f.button(to, false, false);
    assert_eq!(f.moved(), vec![(id(744), id(740))]);
}

#[test]
fn inventory_drag_normalizes_selected_folders_and_children_and_rejects_descendant_targets() {
    let mut f = Fixture::new();
    f.click(10, false);
    f.click(100, true);
    f.start(100);
    let to = f.row(20);
    f.hover(to);
    f.button(to, false, false);
    assert_eq!(f.moved(), vec![(id(10), id(20))]);
    let mut f = Fixture::new();
    f.state.reveal_path.insert(id(20));
    f.settle();
    f.start(20);
    assert_eq!(egui::DragAndDrop::payload::<FolderDrag>(&f.ctx).expect("folder cargo").0, id(20));
    let to = f.row(21);
    assert_eq!(f.hover(to), egui::CursorIcon::NotAllowed);
    f.button(to, false, false);
    assert!(f.actions.is_empty());
}

#[test]
fn inventory_drag_moves_a_selection_between_two_inventory_windows() {
    for background in [false, true] {
        let mut f = Fixture::new();
        f.two_windows = true;
        f.settle();
        f.click(100, false);
        f.click(101, true);
        f.start(100);
        let mut to = f.row_in(20, f.other.ui_id);
        if background {
            to += Vec2::new(80.0, 300.0);
        }
        assert_eq!(f.hover(to), egui::CursorIcon::Move);
        f.button(to, false, false);
        assert_eq!(f.moved(), vec![(id(100), id(20)), (id(101), id(20))]);
    }
}

#[test]
fn inventory_drag_shift_selection_moves_the_complete_range() {
    let mut f = Fixture::new();
    f.click(100, false);
    let pos = f.row(102);
    let modifiers = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    for pressed in [true, false] {
        f.frame(
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                },
            ],
            false,
        );
    }
    assert_eq!(f.state.selection, HashSet::from([id(100), id(101), id(102)]));
    f.start(101);
    let to = f.row(20);
    f.hover(to);
    f.button(to, false, false);
    assert_eq!(f.moved(), vec![(id(100), id(20)), (id(101), id(20)), (id(102), id(20))]);
}

#[test]
fn inventory_drag_hover_opens_the_destination_and_escape_cancels_without_moving() {
    let mut f = Fixture::new();
    let nested = egui::Id::unique(("inventory_drag_row", f.state.ui_id, id(21)));
    assert!(f.ctx.data(|d| d.get_temp::<egui::Rect>(nested)).is_none());
    f.click(100, false);
    f.click(101, true);
    f.start(100);
    let to = f.row(20);
    f.hover(to);
    for _ in 0..25 {
        f.frame(Vec::new(), false);
    }
    assert!(f.ctx.data(|d| d.get_temp::<egui::Rect>(nested)).is_some());
    f.frame(
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        false,
    );
    f.button(to, false, false);
    assert!(f.actions.is_empty());
    assert!(!egui::DragAndDrop::has_any_payload(&f.ctx));
}

#[test]
fn inventory_drag_same_parent_is_a_no_op_and_does_not_move_to_the_enclosing_root() {
    let mut f = Fixture::new();
    f.click(100, false);
    f.click(101, true);
    f.start(100);
    let to = f.row(10);
    assert_eq!(f.hover(to), egui::CursorIcon::NotAllowed);
    f.button(to, false, false);
    assert!(f.actions.is_empty());
}

/// Two real clicks on an object row put it on; once worn, the same
/// double-click takes it off (LLObjectBridge::openItem).
#[test]
fn inventory_double_click_attaches_an_object_then_detaches_it_once_worn() {
    use crate::world::appearance::Action as Wear;
    let mut f = Fixture::new();
    if let Some(it) = f.inv.items.get_mut(&id(101)) {
        it.asset_type = 6;
    }
    f.settle();
    let double_click = |f: &mut Fixture| {
        f.click(101, false);
        f.click(101, false);
        let actions = std::mem::take(&mut f.actions);
        // a second past the double-click delay: the next pair is no triple click
        for _ in 0..4 {
            f.settle();
        }
        actions
    };
    let attach = double_click(&mut f);
    assert!(
        matches!(&attach[..], [InvAction::Appearance(Wear::WearItem { item, replace: true, point: 0 })] if *item == id(101)),
        "not worn: attach"
    );
    f.facts.worn.insert(id(101));
    let detach = double_click(&mut f);
    assert!(
        matches!(&detach[..], [InvAction::Appearance(Wear::Remove(item))] if *item == id(101)),
        "worn: detach, got {} actions",
        detach.len()
    );
    // a single click only selects
    f.click(101, false);
    assert!(f.actions.is_empty());
}
