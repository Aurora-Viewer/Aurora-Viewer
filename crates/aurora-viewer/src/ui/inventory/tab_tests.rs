//! Exercise real tab clicks and existing per-item save/close paths.
use super::*;
use std::{collections::HashMap, sync::Arc};

struct Fixture {
    ctx: egui::Context,
    world: World,
    state: InventoryUi,
    /// The dual layout's right pane, when shown (with the inventory window
    /// open: the pane is skipped while it is closed).
    dual: Option<InventoryUi>,
    open: bool,
    prefs: InventoryPreferences,
    labels: Vec<(String, egui::Rect)>,
}
impl Fixture {
    fn new(kind: i32) -> Self {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(2);
        crate::world::inventory::demo::seed(&mut world.inventory, world.agent_id);
        world.inventory.add_items(
            [9101, 9102]
                .into_iter()
                .enumerate()
                .map(|(index, n)| aurora_net::inventory::InvItem {
                    id: Uuid::from_u128(n),
                    parent: world.inventory.root,
                    name: format!("Item {index}"),
                    asset_type: kind,
                    owner: world.agent_id,
                    owner_mask: 0x7fff_ffff,
                    base_mask: 0x7fff_ffff,
                    ..Default::default()
                })
                .collect(),
        );
        Self {
            ctx: Default::default(),
            world,
            state: Default::default(),
            dual: None,
            open: false,
            prefs: Default::default(),
            labels: Vec::new(),
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        let theme = crate::theme::Theme::default();
        theme.apply(&self.ctx, 1.0);
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                super::show(
                    ui.ctx(),
                    &theme.palette(),
                    &Icons::default(),
                    &mut self.world,
                    &mut self.state,
                    self.dual.as_mut(),
                    &mut self.prefs,
                    &mut self.open,
                    false,
                    &HashMap::new(),
                    &HashMap::new(),
                    &HashMap::new(),
                    &Default::default(),
                    false,
                );
            },
        );
        fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(text) if !text.galley.elided => labels.push((
                    text.galley.text().to_owned(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                )),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, labels);
                    }
                }
                _ => {}
            }
        }
        self.labels.clear();
        for shape in &output.shapes {
            collect(&shape.shape, &mut self.labels);
        }
        output.textures_delta.clear();
    }
    fn settle(&mut self) {
        for _ in 0..6 {
            self.frame(Vec::new());
        }
    }
    fn button(&mut self, pos: egui::Pos2, pressed: bool) {
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            },
        ]);
    }
    fn click(&mut self, pos: egui::Pos2) {
        self.button(pos, true);
        self.button(pos, false);
        self.settle();
    }
    fn host_rect(&self) -> egui::Rect {
        let id = self
            .state
            .windows
            .iter()
            .find_map(|w| w.state.tab_host.as_ref())
            .unwrap()
            .window_id();
        self.ctx.memory(|memory| memory.area_rect(egui::Id::unique(id))).unwrap()
    }
    fn tab(&mut self, label: &str) {
        self.settle();
        let host = self.host_rect();
        let pos = self
            .labels
            .iter()
            .find(|(name, rect)| name == label && rect.center().y > host.top() + 40.0 && rect.center().y < host.top() + 95.0)
            .unwrap_or_else(|| panic!("tab {label}: {:?}", self.labels))
            .1
            .center();
        self.click(pos);
    }
    fn label(&mut self, label: &str) {
        self.settle();
        let pos = self
            .labels
            .iter()
            .rev()
            .find(|(name, _)| name == label)
            .unwrap_or_else(|| panic!("label {label}: {:?}", self.labels))
            .1
            .center();
        self.click(pos);
    }
    fn close_host(&mut self) {
        let rect = self.host_rect();
        self.click(egui::pos2(rect.right() - 18.0, rect.top() + 19.0));
    }
}

#[test]
fn preview_tabs_properties_preserve_each_items_fields_and_reopen_the_existing_host() {
    let mut f = Fixture::new(6);
    let ids = [9101, 9102].map(Uuid::from_u128);
    f.state.open_properties_many(&ids);
    f.settle();
    assert_eq!(f.state.windows.len(), 2);
    let first_id = f.state.windows[0].id;
    f.state.windows[0].state.property_edit.as_mut().unwrap().1.desc = "First draft".into();
    f.tab("Item 1");
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
    assert_ne!(f.state.windows[1].state.property_edit.as_ref().unwrap().1.desc, "First draft");
    f.tab("Item 0");
    assert_eq!(f.state.windows[0].state.property_edit.as_ref().unwrap().1.desc, "First draft");
    assert!(f.state.focus_properties(&f.ctx, ids[1]));
    f.settle();
    assert_eq!(f.state.windows.len(), 2);
    assert_eq!(f.state.windows[0].id, first_id);
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
    f.close_host();
    assert!(f.state.windows.is_empty());
}

#[test]
fn properties_asked_in_a_folder_window_reach_the_open_windows_instead_of_a_copy() {
    let mut f = Fixture::new(6);
    let ids = [9101, 9102].map(Uuid::from_u128);
    f.state.open_properties_many(&ids);
    f.settle();
    let before: Vec<_> = f.state.windows.iter().map(|w| w.id).collect();
    // asked again from a folder window, one item then both
    f.state.answer_properties(&f.ctx, document::PropertiesAsk::One(ids[1]));
    f.settle();
    assert_eq!(f.state.windows.iter().map(|w| w.id).collect::<Vec<_>>(), before);
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
    f.state.answer_properties(&f.ctx, document::PropertiesAsk::Many(ids.to_vec()));
    f.settle();
    assert_eq!(f.state.windows.iter().map(|w| w.id).collect::<Vec<_>>(), before);
    // an item not shown yet gets a window of its own
    f.close_host();
    f.state.answer_properties(&f.ctx, document::PropertiesAsk::One(ids[0]));
    f.settle();
    assert_eq!(f.state.windows.len(), 1);
    assert!(matches!(f.state.windows[0].state.dialog, Some(EditDialog::Properties(id)) if id == ids[0]));
}

#[test]
fn preview_tabs_notecards_keep_background_saves_and_cancelled_closure_independent() {
    let mut f = Fixture::new(7);
    let ids = [9101, 9102].map(Uuid::from_u128);
    for id in ids {
        f.state.open_document(&f.ctx, id, 7, None);
        f.state
            .windows
            .last_mut()
            .unwrap()
            .state
            .load_document(7, true, &Ok(b"Original".to_vec()));
    }
    f.state.group_documents(&ids, None);
    f.settle();
    for (index, window) in f.state.windows.iter_mut().enumerate() {
        window.state.preview_text = Some(Ok(format!("Draft {index}")));
        window.state.document_changed();
    }
    f.tab("Item 1 *");
    let request = Uuid::from_u128(9200);
    assert!(
        f.state.windows[0]
            .state
            .start_content_save(request, 7, "Draft 0".into(), false)
            .is_ok()
    );
    assert!(f.state.windows[0].state.finish_content_save(request, None));
    f.settle();
    assert!(!f.state.windows[0].state.preview_dirty);
    assert!(f.state.windows[1].state.preview_dirty);
    f.close_host();
    assert!(matches!(f.state.windows[1].state.dialog, Some(EditDialog::Discard(id)) if id == ids[1]));
    f.label(aurora_i18n::tr!("common-cancel"));
    assert_eq!(f.state.windows.len(), 2);
    assert_eq!(f.state.windows[1].state.preview_text, Some(Ok("Draft 1".into())));
    f.tab("Item 0");
    assert!(
        f.state.windows[1]
            .state
            .start_content_save(request, 7, "Draft 1".into(), false)
            .is_ok()
    );
    assert!(f.state.windows[1].state.finish_content_save(request, Some("Save failed")));
    f.settle();
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
    assert!(f.state.windows[1].state.preview_dirty);
    f.close_host();
    f.label(aurora_i18n::tr!("ui-inventory-dialogs-discard-changes"));
    assert!(f.state.windows.is_empty());
}

#[test]
fn preview_tabs_media_can_switch_and_close_without_replacing_their_item_states() {
    for kind in [1, 20] {
        let mut f = Fixture::new(kind);
        let ids = [9101, 9102].map(Uuid::from_u128);
        for id in ids {
            f.state.open_document(&f.ctx, id, kind, None);
        }
        f.state.group_documents(&ids, None);
        f.settle();
        f.tab("Item 1");
        assert_eq!(f.state.windows.len(), 2);
        assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
        assert_eq!(f.state.windows[0].state.preview, Some(ids[0]));
        if kind == 20 {
            let before = f.host_rect();
            let corner = before.right_bottom() - Vec2::splat(2.0);
            let target = corner + Vec2::new(90.0, 40.0);
            f.frame(vec![egui::Event::PointerMoved(corner)]);
            f.button(corner, true);
            for _ in 0..4 {
                f.frame(vec![egui::Event::PointerMoved(target)]);
            }
            f.button(target, false);
            f.settle();
            assert!(f.host_rect().width() > before.width() + 40.0, "animation host remains resizable");
        }
        f.close_host();
        assert!(f.state.windows.is_empty(), "media kind {kind}");
    }
}

#[test]
fn properties_asked_in_the_right_pane_of_the_dual_inventory_open_their_windows() {
    let mut f = Fixture::new(6);
    f.dual = Some(InventoryUi::dual_pane());
    f.open = true;
    let ids = [9101, 9102].map(Uuid::from_u128);
    // the right pane's context menu, as for a folder window
    let ask = |f: &mut Fixture, command: context_catalog::Command| {
        let dual = f.dual.as_mut().unwrap();
        let mut command = Some(command);
        let mut output = f.ctx.run_ui(Default::default(), |ui| {
            context_catalog::execute(
                command.take().expect("one pass"),
                ui,
                &f.world.inventory,
                dual,
                &mut f.prefs,
                &Facts::default(),
                &mut Vec::new(),
            );
        });
        output.textures_delta.clear();
        f.settle();
    };
    ask(&mut f, context_catalog::Command::Properties(ids[0]));
    assert!(f.dual.as_ref().unwrap().properties_asked.is_empty(), "the main inventory answers");
    assert_eq!(f.state.windows.len(), 1);
    assert!(matches!(f.state.windows[0].state.dialog, Some(EditDialog::Properties(id)) if id == ids[0]));
    let first = f.state.windows[0].id;
    ask(&mut f, context_catalog::Command::PropertiesMany(ids.to_vec()));
    assert_eq!(f.state.windows.len(), 2);
    assert_eq!(f.state.windows[0].id, first, "the window already shown is kept");
    assert!(f.state.windows.iter().all(|w| w.state.tab_host.is_some()), "one tab group");
    ask(&mut f, context_catalog::Command::Properties(ids[1]));
    assert_eq!(f.state.windows.len(), 2);
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
}

#[test]
fn documents_opened_from_the_right_pane_share_one_tab_group() {
    let mut f = Fixture::new(7);
    let mut dual = InventoryUi::dual_pane();
    let ids = [9101, 9102].map(Uuid::from_u128);
    // one notecard already open from the main pane, the other opened by a
    // bulk Open in the right pane (`App`, `InvAction::PreviewMany`)
    let loaded = |state: &mut InventoryUi| {
        let window = state.windows.last_mut().unwrap();
        window.state.load_document(7, true, &Ok(b"Original".to_vec()));
    };
    f.state.open_document(&f.ctx, ids[0], 7, None);
    loaded(&mut f.state);
    f.settle();
    assert_eq!(f.state.windows.len(), 1);
    dual.open_document(&f.ctx, ids[1], 7, None);
    loaded(&mut dual);
    f.state.group_documents(&ids, Some(&dual));
    f.dual = Some(dual);
    f.open = true;
    f.settle();
    assert_eq!(f.state.windows.len(), 2);
    let hosts: Vec<_> = f
        .state
        .windows
        .iter()
        .map(|w| w.state.tab_host.as_ref().map(|host| host.id))
        .collect();
    assert!(hosts[0].is_some() && hosts[0] == hosts[1], "{hosts:?}");
}

#[test]
fn a_save_question_stays_while_a_group_closes_and_another_tab_is_clicked() {
    let mut f = Fixture::new(7);
    let ids = [9101, 9102].map(Uuid::from_u128);
    for id in ids {
        f.state.open_document(&f.ctx, id, 7, None);
        f.state
            .windows
            .last_mut()
            .unwrap()
            .state
            .load_document(7, true, &Ok(b"Original".to_vec()));
    }
    f.state.group_documents(&ids, None);
    f.settle();
    for (index, window) in f.state.windows.iter_mut().enumerate() {
        window.state.preview_text = Some(Ok(format!("Draft {index}")));
        window.state.document_changed();
    }
    f.settle();
    f.close_host();
    assert!(matches!(f.state.windows[0].state.dialog, Some(EditDialog::Discard(id)) if id == ids[0]));
    let question = aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes").to_string();
    // a click on the other tab while the question waits changes nothing
    f.tab("Item 1 *");
    assert!(f.state.windows[0].state.tab_host.as_ref().unwrap().active());
    assert!(matches!(f.state.windows[0].state.dialog, Some(EditDialog::Discard(id)) if id == ids[0]));
    assert!(f.state.windows[1].state.dialog.is_none());
    assert!(f.labels.iter().any(|(text, _)| *text == question), "the question is still drawn");
    let dialog = format!("inventory_dialog_{}", f.state.windows[0].id);
    let host = f.state.windows[0].state.tab_host.as_ref().unwrap().window_id();
    assert!(super::super::widgets::floater_above(&f.ctx, &dialog, &host), "above its group");
    // cancelled, the tabs work again
    f.label(aurora_i18n::tr!("common-cancel"));
    assert!(f.state.windows[0].state.dialog.is_none(), "cancelled");
    f.tab("Item 1 *");
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
    // the close goes on to the second notecard once the first is answered
    f.close_host();
    f.label(aurora_i18n::tr!("ui-inventory-dialogs-discard-changes"));
    assert_eq!(f.state.windows.len(), 1);
    assert!(matches!(f.state.windows[0].state.dialog, Some(EditDialog::Discard(id)) if id == ids[0]));
    f.label(aurora_i18n::tr!("ui-inventory-dialogs-discard-changes"));
    assert!(f.state.windows.is_empty());
}

/// Two loaded notecards in one group, both with unsaved drafts.
fn dirty_notecards() -> (Fixture, [Uuid; 2]) {
    let mut f = Fixture::new(7);
    let ids = [9101, 9102].map(Uuid::from_u128);
    for id in ids {
        f.state.open_document(&f.ctx, id, 7, None);
        f.state
            .windows
            .last_mut()
            .unwrap()
            .state
            .load_document(7, true, &Ok(b"Original".to_vec()));
    }
    f.state.group_documents(&ids, None);
    f.settle();
    for (index, window) in f.state.windows.iter_mut().enumerate() {
        window.state.preview_text = Some(Ok(format!("Draft {index}")));
        window.state.document_changed();
    }
    f.settle();
    (f, ids)
}

#[test]
fn opening_or_failing_another_tab_while_a_question_waits_keeps_the_question_in_front() {
    let (mut f, ids) = dirty_notecards();
    f.close_host();
    assert!(matches!(f.state.windows[0].state.dialog, Some(EditDialog::Discard(id)) if id == ids[0]));
    let question = aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes").to_string();
    // a double-click on the second notecard in the inventory
    f.state.open_document(&f.ctx, ids[1], 7, None);
    f.settle();
    assert!(f.state.windows[0].state.tab_host.as_ref().unwrap().active());
    assert!(f.labels.iter().any(|(text, _)| *text == question), "the question is still drawn");
    // the second notecard's background save fails meanwhile
    let request = Uuid::from_u128(9200);
    assert!(
        f.state.windows[1]
            .state
            .start_content_save(request, 7, "Draft 1".into(), false)
            .is_ok()
    );
    assert!(f.state.windows[1].state.finish_content_save(request, Some("Save failed")));
    f.settle();
    assert!(f.state.windows[0].state.tab_host.as_ref().unwrap().active());
    assert!(matches!(f.state.windows[0].state.dialog, Some(EditDialog::Discard(id)) if id == ids[0]));
    assert!(f.labels.iter().any(|(text, _)| *text == question), "the question is still drawn");
    // answered: the notecard asked for comes forward
    f.label(aurora_i18n::tr!("common-cancel"));
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
    assert_eq!(f.state.windows.len(), 2);
}

#[test]
fn a_notecard_attachment_stays_open_when_its_tab_goes_to_the_back() {
    let (mut f, _) = dirty_notecards();
    f.state.windows[0].state.attachment_preview = Some(aurora_net::inventory::InvItem {
        id: Uuid::from_u128(9300),
        name: "Attached picture".into(),
        asset_type: 0,
        ..Default::default()
    });
    f.settle();
    assert!(f.labels.iter().any(|(text, _)| text == "Attached picture"));
    f.tab("Item 1 *");
    assert!(f.state.windows[1].state.tab_host.as_ref().unwrap().active());
    assert!(
        f.labels.iter().any(|(text, _)| text == "Attached picture"),
        "the attachment's window stays: {:?}",
        f.labels
    );
    assert!(f.state.windows[0].state.attachment_preview.is_some());
}
