//! Real Open-window pointer/keyboard regressions for issue #1410.
use super::*;
use crate::build::BuildTool;
use crate::interaction::{Contents, Target};
use aurora_net::build::perm;
use aurora_net::inventory::InvItem;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use uuid::Uuid;

struct Window {
    ctx: egui::Context,
    icons: super::super::icons::Icons,
    state: Interactions,
    world: World,
    build: BuildTool,
    actions: Vec<ContentsAction>,
    object: Uuid,
    ids: Vec<Uuid>,
    frame: u32,
}

impl Window {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let icons = super::super::icons::Icons::load(&ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let idx = world.objects.iter().find(|(_, o)| o.key.local_id == 118).unwrap().0;
        let agent = world.agent_id;
        let o = world.objects.get_mut(idx).unwrap();
        o.owner_id = agent;
        o.update_flags = crate::ui::context::flags::OBJECT_MODIFY | crate::ui::context::flags::OBJECT_YOU_OWNER;
        let object = o.full_id;
        let key = o.key;
        let ids: Vec<_> = (0..4).map(|n| Uuid::from_u128(99000 + n)).collect();
        let items = ids
            .iter()
            .enumerate()
            .map(|(n, id)| InvItem {
                id: *id,
                parent: object,
                owner: agent,
                name: format!("Entry {n:03}"),
                asset_type: 7,
                inv_type: 7,
                owner_mask: perm::COPY | perm::MODIFY | perm::TRANSFER,
                ..Default::default()
            })
            .collect();
        let mut state = Interactions::default();
        state.contents = Some(Contents {
            target: Target {
                action: Action::Open,
                clicked: key,
                clicked_id: object,
                key,
                object,
                root: object,
                menu: false,
            },
            result: Some(Ok(items)),
            opened: std::time::Instant::now(),
        });
        let mut window = Self {
            ctx,
            icons,
            state,
            world,
            build: BuildTool::default(),
            actions: Vec::new(),
            object,
            ids,
            frame: 0,
        };
        for _ in 0..5 {
            window.show(Vec::new());
        }
        window
    }

    fn show(&mut self, events: Vec<Event>) -> Vec<(String, Rect)> {
        self.frame += 1;
        let ctx = self.ctx.clone();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                time: Some(self.frame as f64 * 0.1),
                focused: true,
                events,
                ..Default::default()
            },
            |_| {
                self.actions.extend(contents_shortcuts(&ctx, &self.state, &self.world, &self.build));
                self.actions.extend(show_contents(
                    &ctx,
                    &crate::theme::Theme::default().palette(),
                    &self.icons,
                    &mut self.state,
                    &self.world,
                    &self.build,
                ));
            },
        );
        let mut texts = Vec::new();
        fn visit(shape: &egui::epaint::Shape, texts: &mut Vec<(String, Rect)>) {
            match shape {
                egui::epaint::Shape::Text(t) => texts.push((t.galley.text().into(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| visit(s, texts)),
                _ => {}
            }
        }
        for shape in &output.shapes {
            visit(&shape.shape, &mut texts);
        }
        output.textures_delta.clear();
        texts
    }

    fn position(&mut self, text: &str) -> Pos2 {
        let texts = self.show(Vec::new());
        let mut matches: Vec<_> = texts.iter().filter(|(s, _)| s.starts_with(text)).collect();
        if egui::Popup::is_any_open(&self.ctx) {
            matches.reverse();
        }
        matches.first().unwrap_or_else(|| panic!("missing {text}")).1.center()
    }

    fn click(&mut self, text: &str, modifiers: Modifiers, button: PointerButton) {
        let pos = self.position(text);
        self.show(vec![Event::PointerMoved(pos), Event::ModifiersChanged(modifiers)]);
        for pressed in [true, false] {
            self.show(vec![Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            }]);
        }
    }

    fn key(&mut self, key: Key, modifiers: Modifiers, repeat: bool) {
        self.show(vec![
            Event::ModifiersChanged(modifiers),
            Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat,
                modifiers,
            },
        ]);
        assert!(
            !self.ctx.input(|i| i
                .events
                .iter()
                .any(|e| matches!(e, Event::Key { key: k, pressed: true, .. } if *k == key))),
            "the contents list must consume its keys before object shortcuts"
        );
    }

    fn drag(&mut self, text: &str) {
        let pos = self.position(text);
        self.show(vec![
            Event::ModifiersChanged(Modifiers::NONE),
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        self.show(vec![Event::PointerMoved(pos + egui::vec2(20.0, 0.0))]);
    }

    fn selection(&self) -> std::collections::HashSet<Uuid> {
        self.ctx.data(|d| {
            d.get_temp::<super::super::inventory::task_selection::Selection>(contents_list::state_id())
                .unwrap()
                .items
        })
    }
}

#[test]
fn open_contents_ctrl_shift_and_right_click_keep_the_batch_in_list_order() {
    let mut w = Window::new();
    w.click("Entry 000", Modifiers::NONE, PointerButton::Primary);
    w.click("Entry 002", Modifiers::CTRL, PointerButton::Primary);
    assert_eq!(w.selection(), std::collections::HashSet::from([w.ids[0], w.ids[2]]));
    w.click("Entry 000", Modifiers::NONE, PointerButton::Secondary);
    w.show(Vec::new());
    assert_eq!(w.selection().len(), 2);
    w.click(
        aurora_i18n::tr!("build-ui-contents-copy-to-inventory"),
        Modifiers::NONE,
        PointerButton::Primary,
    );
    assert!(
        matches!(&w.actions[..], [ContentsAction::Take(object, items)] if *object == w.object && items.iter().map(|i| i.item_id).collect::<Vec<_>>() == [w.ids[0], w.ids[2]])
    );
    w.actions.clear();
    w.click("Entry 001", Modifiers::NONE, PointerButton::Primary);
    w.click("Entry 003", Modifiers::SHIFT, PointerButton::Primary);
    assert_eq!(w.selection(), std::collections::HashSet::from([w.ids[1], w.ids[2], w.ids[3]]));
    w.drag("Entry 002");
    let drag = egui::DragAndDrop::payload::<super::super::inventory::task_drop::TaskDrag>(&w.ctx).unwrap();
    assert_eq!(
        drag.requests.iter().map(|r| r.item.item_id).collect::<Vec<_>>(),
        [w.ids[1], w.ids[2], w.ids[3]]
    );
}

#[test]
fn open_contents_select_all_delete_is_scoped_and_readonly_and_repeats_are_safe() {
    for editable in [false, true] {
        let mut w = Window::new();
        if !editable {
            let idx = w.world.objects.index_of_uuid(&w.object).unwrap();
            w.world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
        }
        w.click("Entry 000", Modifiers::NONE, PointerButton::Primary);
        w.key(Key::A, Modifiers::CTRL, false);
        assert_eq!(w.selection().len(), 4);
        // A stale row cannot be deleted after the simulator removes it.
        w.state
            .contents
            .as_mut()
            .unwrap()
            .result
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .remove(1);
        w.key(Key::Delete, Modifiers::NONE, false);
        if editable {
            assert!(
                matches!(&w.actions[..], [ContentsAction::Remove(object, ids)] if *object == w.object && ids == &vec![w.ids[0], w.ids[2], w.ids[3]])
            );
        } else {
            assert!(w.actions.is_empty());
        }
        w.actions.clear();
        w.key(Key::Delete, Modifiers::NONE, true);
        assert!(w.actions.is_empty());
    }
}

#[test]
fn open_contents_drag_is_all_or_nothing_for_no_copy_worn_items_and_whole_copy_ignores_selection() {
    for worn in [false, true] {
        let mut w = Window::new();
        w.state.contents.as_mut().unwrap().result.as_mut().unwrap().as_mut().unwrap()[2].owner_mask &= !perm::COPY;
        if worn {
            let idx = w.world.objects.index_of_uuid(&w.object).unwrap();
            let o = w.world.objects.get_mut(idx).unwrap();
            o.parent_id = 1;
            o.state = 0x10;
        }
        w.click("Entry 000", Modifiers::NONE, PointerButton::Primary);
        w.click("Entry 002", Modifiers::CTRL, PointerButton::Primary);
        let request = copy_request(w.state.contents.as_ref().unwrap(), None, &w.world);
        if worn {
            assert!(request.is_err());
        } else {
            assert_eq!(request.unwrap().items.len(), 4, "Copy to inventory still unpacks the whole box");
        }
        w.drag("Entry 000");
        let drag = egui::DragAndDrop::payload::<super::super::inventory::task_drop::TaskDrag>(&w.ctx);
        if worn {
            assert!(drag.is_none(), "one refused no-copy item refuses the batch");
        } else {
            let drag = drag.unwrap();
            assert_eq!(drag.requests.len(), 2);
            assert!(matches!(drag.kind, crate::build::task_inventory::TransferKind::Move { .. }));
        }
    }
}

#[test]
fn open_contents_accepts_a_multi_item_drop_into_an_empty_object_and_refuses_a_bad_batch() {
    for rejected in [false, true] {
        let mut w = Window::new();
        w.state.contents.as_mut().unwrap().result = Some(Ok(Vec::new()));
        let items: Vec<_> = (0..2)
            .map(|n| InvItem {
                id: Uuid::from_u128(99500 + n),
                parent: w.world.inventory.root,
                owner: w.world.agent_id,
                asset_type: if rejected && n == 1 { 24 } else { 7 },
                inv_type: 7,
                owner_mask: perm::COPY | perm::MODIFY | perm::TRANSFER,
                ..Default::default()
            })
            .collect();
        let ids = items.iter().map(|i| i.id).collect::<Vec<_>>();
        w.world.inventory.add_items(items);
        let pos = w.position(aurora_i18n::tr!("ui-object-actions-this-object-is-empty"));
        super::super::inventory::drag::demo_carry(&w.ctx, ids.clone());
        w.show(vec![Event::PointerMoved(pos)]);
        w.show(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        if rejected {
            assert!(w.actions.is_empty());
        } else {
            assert!(matches!(&w.actions[..], [ContentsAction::Drop(object, dropped, false)] if *object == w.object && dropped == &ids));
        }
    }
}
