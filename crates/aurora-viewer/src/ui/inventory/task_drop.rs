//! Shared drag source for both object inventories, destination folders and
//! MoveInventoryFromObject / MoveInventoryFromScriptedObject confirmations.
use super::*;
use crate::build::task_inventory::{self, TransferKind, TransferRequest};
use aurora_net::build::TaskItem;

/// The dragged rows, one request each in list order
/// (LLFolderViewModelInventory::startDrag → beginMultiDrag).
#[derive(Clone)]
pub struct TaskDrag {
    pub requests: Vec<TransferRequest>,
    /// Move as soon as one item leaves the object (the cursor).
    pub kind: TransferKind,
}

/// No-copy items of one object, into one folder, waiting for the warning.
/// warn_move_inventory attaches the items dropped while its notification is
/// open to that notification: one answer covers the batch.
pub struct PendingTransfer {
    pub requests: Vec<TransferRequest>,
    pub scripted: bool,
    pub ignore_warning: bool,
}

impl PendingTransfer {
    pub fn new(request: TransferRequest, scripted: bool) -> Self {
        Self {
            requests: vec![request],
            scripted,
            ignore_warning: false,
        }
    }

    /// Takes `request` into this batch when it has the same object and
    /// destination; hands it back otherwise.
    pub fn join(&mut self, request: TransferRequest) -> Option<TransferRequest> {
        let Some(first) = self.requests.first() else {
            return Some(request);
        };
        if first.object != request.object || first.folder != request.folder {
            return Some(request);
        }
        if !self.requests.iter().any(|r| r.item.item_id == request.item.item_id) {
            self.requests.push(request);
        }
        None
    }
}

/// Read after all windows, before egui clears released cargo. Rows precede
/// enclosing surfaces, so the first hovered target owns the feedback this pass.
/// Firestorm: LLToolDragAndDrop::acceptanceToCursor / handleHover.
pub fn cursor(ctx: &egui::Context) -> Option<egui::CursorIcon> {
    if !egui::DragAndDrop::has_payload_of_type::<TaskDrag>(ctx) || !ctx.input(|i| i.pointer.primary_down()) {
        return None;
    }
    let pass = ctx.cumulative_pass_nr();
    Some(ctx.data_mut(|data| {
        data.get_temp::<(u64, egui::CursorIcon)>(egui::Id::unique("task_inventory_drag_cursor"))
            .filter(|(at, _)| *at == pass)
            .map_or(egui::CursorIcon::NotAllowed, |(_, cursor)| cursor)
    }))
}

fn target_cursor(ctx: &egui::Context, cursor: egui::CursorIcon) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let id = egui::Id::unique("task_inventory_drag_cursor");
        if data.get_temp::<(u64, egui::CursorIcon)>(id).is_none_or(|(at, _)| at != pass) {
            data.insert_temp(id, (pass, cursor));
            true
        } else {
            false
        }
    })
}

/// `items`: the row and, when it is part of a multiple selection, every
/// selected row. One item refused refuses the drag (LLFolderViewModelInventory::
/// startDrag: `can_drag` of every item).
pub fn source(ui: &mut egui::Ui, r: &egui::Response, world: &World, object: Uuid, items: &[TaskItem], owned: bool) {
    let access: Result<Vec<TransferKind>, _> = items.iter().map(|item| task_inventory::kind(world, object, item, owned)).collect();
    if let Err(error) = &access {
        r.clone().on_hover_text(*error);
    }
    if r.drag_started() {
        if let Ok(kinds) = access.as_ref()
            && !kinds.is_empty()
        {
            let kind = kinds
                .iter()
                .copied()
                .find(|kind| *kind != TransferKind::Copy)
                .unwrap_or(TransferKind::Copy);
            let requests = items
                .iter()
                .map(|item| TransferRequest {
                    object,
                    item: item.clone(),
                    folder: Uuid::nil(),
                    approved: None,
                    ignore_warning: false,
                })
                .collect();
            r.dnd_set_drag_payload(TaskDrag { requests, kind });
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        }
    } else if r.dragged() && access.is_err() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
    }
}

pub fn target(
    ui: &mut egui::Ui,
    p: &Palette,
    r: &egui::Response,
    inv: &Inventory,
    folder: Uuid,
    prefs: &InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    let Some(cargo) = r.dnd_hover_payload::<TaskDrag>() else { return };
    // Every item accepted or none (LLToolDragAndDrop::dragOrDrop).
    let allowed = cargo
        .requests
        .iter()
        .all(|request| task_inventory::destination(inv, folder, &request.item, &prefs.protected));
    if !target_cursor(
        ui.ctx(),
        if allowed {
            if cargo.kind == TransferKind::Copy {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::Move
            }
        } else {
            egui::CursorIcon::NotAllowed
        },
    ) {
        return;
    }
    if allowed {
        super::tree::drop_outline(ui, p, r.rect, true);
        if let Some(cargo) = r.dnd_release_payload::<TaskDrag>() {
            for request in &cargo.requests {
                let mut request = request.clone();
                request.folder = folder;
                actions.push(InvAction::TakeTask(Box::new(request)));
            }
        }
    } else {
        // Do not let an enclosing inventory surface accept a rejected row.
        r.dnd_release_payload::<TaskDrag>();
    }
}

/// The first waiting batch, one dialog at a time; its answer gives one
/// action per item (none when cancelled).
pub fn confirm(ctx: &egui::Context, p: &Palette, pending: &mut std::collections::VecDeque<PendingTransfer>) -> Vec<InvAction> {
    let Some(transfer) = pending.front_mut() else {
        return Vec::new();
    };
    let mut answer = None;
    super::super::widgets::Modal::new(egui::Id::unique("task_inventory_transfer")).show(ctx, |ui| {
        ui.set_max_width(380.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("task-transfer-title"))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(8.0);
        if let [request] = &transfer.requests[..] {
            ui.label(RichText::new(&request.item.name).strong().color(p.ink));
            ui.label(aurora_i18n::tr!("task-transfer-warning"));
        } else {
            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                for request in &transfer.requests {
                    ui.label(RichText::new(&request.item.name).strong().color(p.ink));
                }
            });
            ui.label(aurora_i18n::tr!("task-transfer-warning-many", count = transfer.requests.len()));
        }
        if transfer.scripted {
            ui.label(RichText::new(aurora_i18n::tr!("task-transfer-scripted-warning")).color(p.warn));
        }
        ui.add_space(8.0);
        let ignore = aurora_i18n::tr!("task-transfer-ignore");
        super::super::widgets::switch_labeled(ui, p, &mut transfer.ignore_warning, ignore);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            use super::super::widgets::{ActionKind, action_button};
            let size = Vec2::new(0.0, 26.0);
            if action_button(ui, p, "check", aurora_i18n::tr!("task-transfer-confirm"), ActionKind::Primary, size).clicked() {
                answer = Some(true);
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                answer = Some(false);
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
    });
    let Some(answer) = answer else {
        return Vec::new();
    };
    let Some(transfer) = pending.pop_front() else {
        return Vec::new();
    };
    if !answer {
        return Vec::new();
    }
    transfer
        .requests
        .into_iter()
        .map(|mut request| {
            request.approved = Some(transfer.scripted);
            request.ignore_warning = transfer.ignore_warning;
            InvAction::TakeTask(Box::new(request))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (World, TransferRequest) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let object = world.objects.iter().find(|(_, o)| o.key.local_id == 118).unwrap().1.full_id;
        let request = TransferRequest {
            object,
            item: TaskItem {
                item_id: Uuid::from_u128(99001),
                parent_id: object,
                owner_id: world.agent_id,
                owner_mask: aurora_net::build::perm::COPY,
                name: "Source".into(),
                ..Default::default()
            },
            folder: world.inventory.root,
            approved: None,
            ignore_warning: false,
        };
        (world, request)
    }

    fn input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
            time: Some(time),
            focused: true,
            events,
            ..Default::default()
        }
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn real_drag_sets_task_cargo_and_drops_once_into_the_chosen_folder_or_rejects_it() {
        for (copyable, rejected) in [(true, false), (true, true), (false, false), (false, true)] {
            let (world, mut request) = fixture();
            if !copyable {
                request.item.owner_mask = 0;
            }
            // a multiple selection: the first row decides the cursor (it moves or not)
            let second = TaskItem {
                item_id: Uuid::from_u128(99002),
                name: "Second".into(),
                owner_mask: aurora_net::build::perm::COPY,
                ..request.item.clone()
            };
            let items = [request.item.clone(), second];
            let folder = world
                .inventory
                .folders
                .values()
                .find(|f| f.info.type_default == 0 && !f.library)
                .unwrap()
                .info
                .id;
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let mut prefs = InventoryPreferences::default();
            if rejected {
                prefs.protected.insert(folder);
            }
            let mut time = 0.0;
            let mut actions = Vec::new();
            let mut frame = |events| {
                time += 0.04;
                let mut output = ctx.run_ui(input(time, events), |ui| {
                    let zone = egui::Frame::new().show(ui, |ui| {
                        ui.set_min_size(Vec2::new(700.0, 400.0));
                        let r = ui.put(
                            egui::Rect::from_min_size(egui::pos2(20.0, 20.0), Vec2::new(180.0, 26.0)),
                            egui::Button::new("Source").sense(egui::Sense::click_and_drag()),
                        );
                        source(ui, &r, &world, request.object, &items, true);
                        let r = ui.put(
                            egui::Rect::from_min_size(egui::pos2(20.0, 180.0), Vec2::new(180.0, 26.0)),
                            egui::Button::new("Folder"),
                        );
                        target(ui, &p, &r, &world.inventory, folder, &prefs, &mut actions);
                    });
                    target(ui, &p, &zone.response, &world.inventory, world.inventory.root, &prefs, &mut actions);
                    // A later window/widget must not override the drag cursor.
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                    if let Some(cursor) = cursor(ui.ctx()) {
                        ui.ctx().set_cursor_icon(cursor);
                    }
                });
                output.textures_delta.clear();
                output.platform_output.cursor_icon
            };
            let start = egui::pos2(40.0, 32.0);
            let end = egui::pos2(40.0, 192.0);
            frame(Vec::new());
            frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
            frame(vec![egui::Event::PointerMoved(egui::pos2(100.0, 32.0))]);
            assert!(egui::DragAndDrop::has_payload_of_type::<TaskDrag>(&ctx));
            assert!(!egui::DragAndDrop::has_payload_of_type::<super::super::InvDrag>(&ctx));
            let accepted_cursor = if copyable { egui::CursorIcon::Copy } else { egui::CursorIcon::Move };
            let expected = if rejected { egui::CursorIcon::NotAllowed } else { accepted_cursor };
            assert_eq!(frame(vec![egui::Event::PointerMoved(end)]), expected);
            assert_eq!(frame(Vec::new()), expected, "feedback persists while held still");
            assert_eq!(
                frame(vec![egui::Event::PointerMoved(egui::pos2(760.0, 460.0))]),
                egui::CursorIcon::NotAllowed
            );
            assert_eq!(
                frame(vec![egui::Event::PointerMoved(egui::pos2(320.0, 300.0))]),
                accepted_cursor,
                "empty inventory surface"
            );
            assert_eq!(frame(vec![egui::Event::PointerMoved(end)]), expected);
            assert_eq!(
                frame(vec![button(end, false)]),
                egui::CursorIcon::Text,
                "release restores the underlying cursor"
            );
            assert!(cursor(&ctx).is_none());
            if rejected {
                assert!(actions.is_empty());
            } else {
                let taken: Vec<_> = actions
                    .iter()
                    .map(|action| match action {
                        InvAction::TakeTask(req) if req.folder == folder && req.approved.is_none() => req.item.item_id,
                        _ => panic!("one TakeTask into the folder per dragged item"),
                    })
                    .collect();
                assert_eq!(taken, [Uuid::from_u128(99001), Uuid::from_u128(99002)]);
            }
        }
    }

    #[test]
    fn escape_clears_drag_feedback_before_the_next_pass() {
        let (_, request) = fixture();
        let ctx = egui::Context::default();
        let start = egui::pos2(40.0, 32.0);
        let mut output = ctx.run_ui(input(0.0, vec![egui::Event::PointerMoved(start), button(start, true)]), |ui| {
            egui::DragAndDrop::set_payload(
                ui.ctx(),
                TaskDrag {
                    requests: vec![request.clone()],
                    kind: TransferKind::Copy,
                },
            );
            target_cursor(ui.ctx(), egui::CursorIcon::Copy);
            assert_eq!(cursor(ui.ctx()), Some(egui::CursorIcon::Copy));
        });
        output.textures_delta.clear();
        let mut output = ctx.run_ui(input(0.04, Vec::new()), |ui| {
            assert_eq!(
                cursor(ui.ctx()),
                Some(egui::CursorIcon::NotAllowed),
                "previous target cannot leak into a new pass"
            );
        });
        output.textures_delta.clear();
        let mut output = ctx.run_ui(
            input(
                0.08,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ),
            |ui| {
                assert!(cursor(ui.ctx()).is_none());
            },
        );
        output.textures_delta.clear();
        assert!(!egui::DragAndDrop::has_payload_of_type::<TaskDrag>(&ctx));
    }

    #[test]
    fn warning_cancel_sends_nothing_and_move_carries_explicit_approval_for_the_batch() {
        for accept in [false, true] {
            let (_, request) = fixture();
            let mut second = request.clone();
            second.item.item_id = Uuid::from_u128(99002);
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let mut batch = PendingTransfer::new(request, true);
            assert!(batch.join(second).is_none());
            let mut pending = std::collections::VecDeque::from([batch]);
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(input(0.0, Vec::new()), |_| {
                actions.extend(confirm(&ctx, &p, &mut pending));
            });
            output.textures_delta.clear();
            // The modal area obtains its size and fades in after its first frame.
            let mut output = ctx.run_ui(input(0.25, Vec::new()), |_| {
                actions.extend(confirm(&ctx, &p, &mut pending));
            });
            let pos = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text() == aurora_i18n::tr!("task-transfer-confirm") => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("move button");
            output.textures_delta.clear();
            assert!(actions.is_empty() && pending.len() == 1);
            if accept {
                for (time, pressed) in [(0.29, true), (0.33, false)] {
                    let mut output = ctx.run_ui(input(time, vec![egui::Event::PointerMoved(pos), button(pos, pressed)]), |_| {
                        actions.extend(confirm(&ctx, &p, &mut pending));
                    });
                    output.textures_delta.clear();
                }
                assert_eq!(actions.len(), 2, "one answer for every item of the drop");
                assert!(
                    actions
                        .iter()
                        .all(|action| matches!(action, InvAction::TakeTask(req) if req.approved == Some(true)))
                );
            } else {
                let mut output = ctx.run_ui(
                    input(
                        0.29,
                        vec![egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                    ),
                    |_| {
                        actions.extend(confirm(&ctx, &p, &mut pending));
                    },
                );
                output.textures_delta.clear();
                assert!(actions.is_empty());
            }
            assert!(pending.is_empty());
        }
    }

    #[test]
    fn a_batch_joins_items_of_its_object_and_folder_only() {
        let (world, request) = fixture();
        let mut batch = PendingTransfer::new(request.clone(), false);
        assert!(batch.join(request.clone()).is_none(), "the same item twice");
        let mut other_item = request.clone();
        other_item.item.item_id = Uuid::from_u128(99002);
        assert!(batch.join(other_item).is_none());
        let mut other_folder = request.clone();
        other_folder.item.item_id = Uuid::from_u128(99003);
        other_folder.folder = world.inventory.folders.keys().copied().find(|id| *id != request.folder).unwrap();
        assert!(batch.join(other_folder).is_some());
        let mut other_object = request.clone();
        other_object.object = Uuid::from_u128(5);
        assert!(batch.join(other_object).is_some());
        assert_eq!(batch.requests.len(), 2);
    }
}
