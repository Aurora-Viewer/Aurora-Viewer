//! Shared drag source for both object inventories, destination folders and
//! MoveInventoryFromObject / MoveInventoryFromScriptedObject confirmations.
use super::*;
use crate::build::task_inventory::{self, TransferKind, TransferRequest};
use aurora_net::build::TaskItem;

#[derive(Clone)]
pub struct TaskDrag {
    pub request: TransferRequest,
    pub kind: TransferKind,
}

pub struct PendingTransfer {
    pub request: TransferRequest,
    pub scripted: bool,
}

pub fn source(ui: &mut egui::Ui, r: &egui::Response, world: &World, object: Uuid, item: &TaskItem, owned: bool) {
    let access = task_inventory::kind(world, object, item, owned);
    if let Err(error) = access {
        r.clone().on_hover_text(error);
    }
    if r.drag_started() {
        if let Ok(kind) = access {
            r.dnd_set_drag_payload(TaskDrag {
                request: TransferRequest {
                    object,
                    item: item.clone(),
                    folder: Uuid::nil(),
                    approved: None,
                    ignore_warning: false,
                },
                kind,
            });
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
    let allowed = task_inventory::destination(inv, folder, &cargo.request.item, &prefs.protected);
    ui.ctx().set_cursor_icon(if allowed {
        if cargo.kind == TransferKind::Copy {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::Move
        }
    } else {
        egui::CursorIcon::NotAllowed
    });
    if allowed {
        let radius = ui.visuals().window_corner_radius;
        ui.painter()
            .rect_stroke(r.rect, radius, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
        if let Some(cargo) = r.dnd_release_payload::<TaskDrag>() {
            let mut request = cargo.request.clone();
            request.folder = folder;
            actions.push(InvAction::TakeTask(Box::new(request)));
        }
    } else {
        // Do not let an enclosing inventory surface accept a rejected row.
        r.dnd_release_payload::<TaskDrag>();
    }
}

pub fn confirm(ctx: &egui::Context, p: &Palette, pending: &mut Option<PendingTransfer>) -> Option<InvAction> {
    let transfer = pending.as_mut()?;
    let mut answer = None;
    super::super::widgets::Modal::new(egui::Id::new("task_inventory_transfer")).show(ctx, |ui| {
        ui.set_max_width(380.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("task-transfer-title"))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(&transfer.request.item.name).strong().color(p.ink));
        ui.label(aurora_i18n::tr!("task-transfer-warning"));
        if transfer.scripted {
            ui.label(RichText::new(aurora_i18n::tr!("task-transfer-scripted-warning")).color(p.warn));
        }
        ui.add_space(8.0);
        let ignore = aurora_i18n::tr!("task-transfer-ignore");
        super::super::widgets::switch_labeled(ui, p, &mut transfer.request.ignore_warning, ignore);
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
    let answer = answer?;
    let mut transfer = pending.take()?;
    if !answer {
        return None;
    }
    transfer.request.approved = Some(transfer.scripted);
    Some(InvAction::TakeTask(Box::new(transfer.request)))
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
        for rejected in [false, true] {
            let (world, request) = fixture();
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
                        source(ui, &r, &world, request.object, &request.item, true);
                        let r = ui.put(
                            egui::Rect::from_min_size(egui::pos2(20.0, 180.0), Vec2::new(180.0, 26.0)),
                            egui::Button::new("Folder"),
                        );
                        target(ui, &p, &r, &world.inventory, folder, &prefs, &mut actions);
                    });
                    target(ui, &p, &zone.response, &world.inventory, world.inventory.root, &prefs, &mut actions);
                });
                output.textures_delta.clear();
            };
            let start = egui::pos2(40.0, 32.0);
            let end = egui::pos2(40.0, 192.0);
            frame(Vec::new());
            frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
            frame(vec![egui::Event::PointerMoved(egui::pos2(100.0, 32.0))]);
            assert!(egui::DragAndDrop::has_payload_of_type::<TaskDrag>(&ctx));
            assert!(!egui::DragAndDrop::has_payload_of_type::<super::super::InvDrag>(&ctx));
            frame(vec![egui::Event::PointerMoved(end)]);
            frame(vec![button(end, false)]);
            if rejected {
                assert!(actions.is_empty());
            } else {
                assert!(matches!(&actions[..], [InvAction::TakeTask(req)] if req.folder == folder && req.approved.is_none()));
            }
        }
    }

    #[test]
    fn warning_cancel_sends_nothing_and_move_carries_explicit_approval() {
        for accept in [false, true] {
            let (_, request) = fixture();
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let mut pending = Some(PendingTransfer { request, scripted: true });
            let mut action = None;
            let mut output = ctx.run_ui(input(0.0, Vec::new()), |_| {
                action = confirm(&ctx, &p, &mut pending);
            });
            output.textures_delta.clear();
            // The modal area obtains its size and fades in after its first frame.
            let mut output = ctx.run_ui(input(0.25, Vec::new()), |_| {
                if let Some(a) = confirm(&ctx, &p, &mut pending) {
                    action = Some(a);
                }
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
            assert!(action.is_none() && pending.is_some());
            if accept {
                for (time, pressed) in [(0.29, true), (0.33, false)] {
                    let mut output = ctx.run_ui(input(time, vec![egui::Event::PointerMoved(pos), button(pos, pressed)]), |_| {
                        if let Some(a) = confirm(&ctx, &p, &mut pending) {
                            action = Some(a);
                        }
                    });
                    output.textures_delta.clear();
                }
                assert!(matches!(action, Some(InvAction::TakeTask(req)) if req.approved == Some(true)));
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
                        action = confirm(&ctx, &p, &mut pending);
                    },
                );
                output.textures_delta.clear();
                assert!(action.is_none());
            }
            assert!(pending.is_none());
        }
    }
}
