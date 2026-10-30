//! Contents tab (LLPanelContents + LLPanelObjectInventory,
//! indra/newview/llpanelcontents.cpp / llpanelobjectinventory.cpp,
//! originally LGPL 2.1): the object's inventory with its buttons, filter,
//! item menu, and inventory items dropped on it.

use super::contents_selection;
use crate::build::BuildTool;
use crate::build::contents::AT_LSL_TEXT;
use crate::build::contents_drop;
use crate::theme::Palette;
use crate::ui::inventory::item_icon;
use crate::ui::inventory::permissions::Permissions;
use crate::ui::{menu, widgets};
use crate::world::World;
use aurora_net::build::{TaskItem, perm};
use egui::{RichText, Vec2};

/// Localized item name (LLTaskInvFVBridge::getDisplayName). Permissions are
/// displayed with the same compact suffix as the user's inventory.
fn display_name(i: &TaskItem) -> String {
    if i.asset_type == AT_LSL_TEXT && i.name.starts_with("New Script") {
        i.name.replacen("New Script", aurora_i18n::tr!("common-new-script"), 1)
    } else {
        i.name.clone()
    }
}

/// Keep Firestorm's restriction terms searchable after moving them to badges.
fn searchable_name(i: &TaskItem) -> String {
    let mut name = display_name(i);
    for (bit, suffix) in [
        (perm::COPY, aurora_i18n::tr!("build-ui-contents-no-copy")),
        (perm::MODIFY, aurora_i18n::tr!("build-ui-contents-no-modify")),
        (perm::TRANSFER, aurora_i18n::tr!("build-ui-contents-no-transfer")),
    ] {
        if i.owner_mask & bit == 0 {
            name.push_str(suffix);
        }
    }
    name
}

/// "Contents (N items)".
fn root_label(n: usize) -> String {
    aurora_i18n::tr!("contents-count", count = n)
}

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    let Some(idx) = tool.contents_target(world) else { return };
    let Some(object_id) = world.objects.get(idx).map(|o| o.full_id) else {
        return;
    };
    tool.request_contents(world, idx, false);
    let editable = tool.contents_editable(world, idx);
    let single = tool.roots(world).len() == 1 || tool.sel_prims(world).len() == 1;

    if super::legacy::active(ui) {
        ui.columns(3, |columns| {
            if columns[0]
                .add_enabled(editable && single, egui::Button::new(aurora_i18n::tr!("common-new-script")))
                .clicked()
            {
                tool.new_script(world, idx);
            }
            if columns[1].button(aurora_i18n::tr!("common-rights")).clicked() {
                tool.ui.bulk_perms_open = !tool.ui.bulk_perms_open;
            }
            if columns[2].button(aurora_i18n::tr!("build-ui-contents-refresh")).clicked() {
                tool.request_contents(world, idx, true);
            }
            if columns[2]
                .add_enabled(
                    editable && single,
                    egui::Button::new(RichText::new(aurora_i18n::tr!("build-ui-contents-reset-scripts")).size(11.0)),
                )
                .clicked()
            {
                tool.reset_scripts(world);
            }
        });
    } else {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    editable && single,
                    egui::Button::new(RichText::new(aurora_i18n::tr!("common-new-script")).size(12.0)),
                )
                .clicked()
            {
                tool.new_script(world, idx);
            }
            if ui
                .button(RichText::new(aurora_i18n::tr!("common-rights")).size(12.0))
                .on_hover_text(aurora_i18n::tr!("build-ui-contents-change-content-permissions"))
                .clicked()
            {
                tool.ui.bulk_perms_open = !tool.ui.bulk_perms_open;
            }
            ui.vertical(|ui| {
                if ui.small_button(aurora_i18n::tr!("build-ui-contents-refresh")).clicked() {
                    tool.request_contents(world, idx, true);
                }
                if ui
                    .add_enabled(
                        editable && single,
                        egui::Button::new(RichText::new(aurora_i18n::tr!("build-ui-contents-reset-scripts")).size(11.0)).small(),
                    )
                    .clicked()
                {
                    tool.reset_scripts(world);
                }
            });
        });
    }
    ui.add(
        egui::TextEdit::singleline(&mut tool.ui.contents_filter)
            .hint_text(aurora_i18n::tr!("build-ui-contents-enter-filter-text"))
            .desired_width(ui.available_width()),
    );
    crate::ui::inventory::permissions::next_owner_toggle(ui, p, &mut tool.ui.contents_show_next_owner);
    let inv = tool.contents.objects.get(&object_id).cloned().unwrap_or_default();
    let items: Vec<TaskItem> = inv.items.iter().filter(|i| !i.is_folder).cloned().collect();
    let filter = tool.ui.contents_filter.trim().to_uppercase();
    let mut list: Vec<&TaskItem> = items
        .iter()
        .filter(|i| filter.is_empty() || searchable_name(i).to_uppercase().contains(&filter))
        .collect();
    list.sort_by_key(|i| i.name.to_lowercase());
    tool.ui
        .contents_selection
        .sync(object_id, list.iter().map(|it| it.item_id).collect(), ui.layer_id());
    // Fill the bounded section viewport, including after a user resize.
    // The list also accepts inventory drops; only its frame margins sit outside it.
    let height = (ui.clip_rect().bottom() - ui.cursor().top() - 8.0).max(40.0);
    let frame = egui::Frame::new()
        .fill(p.field.gamma_multiply(0.72))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(4);
    let zone = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::inventory_spacing(ui);
        let background = ui.interact(
            egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height)),
            contents_selection::focus_id(object_id),
            egui::Sense::click(),
        );
        if background.clicked() {
            background.request_focus();
            tool.ui.contents_selection.items.clear();
        }
        egui::ScrollArea::both()
            .id_salt("task_contents_list")
            .max_width(ui.available_width())
            .max_height(height)
            .min_scrolled_height((height - ui.spacing().scroll.allocated_width()).max(40.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let open_icon = if inv.loading && items.is_empty() {
                    "folder-filled"
                } else {
                    "folder-open-filled"
                };
                ui.horizontal(|ui| {
                    if let Some(t) = crate::ui::icons::global(open_icon) {
                        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(p.violet_light));
                    }
                    let title = if inv.loading && inv.items.is_empty() {
                        aurora_i18n::tr!("build-ui-contents-loading-contents").to_owned()
                    } else {
                        root_label(items.len())
                    };
                    ui.label(RichText::new(title).size(13.0).color(p.ink))
                        .on_hover_text(aurora_i18n::tr!("common-object-contents"));
                });
                ui.indent("task_contents_items", |ui| {
                    for item in list {
                        item_row(ui, p, tool, world, idx, object_id, item, editable);
                    }
                });
            });
    });
    if ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !zone.response.rect.contains(pos)))
        && !egui::Popup::is_any_open(ui.ctx())
    {
        ui.memory_mut(|m| m.surrender_focus(contents_selection::focus_id(object_id)));
    }
    contents_selection::finish_if_hidden_or_blurred(ui.ctx(), tool, world);
    if demo_enabled() {
        ui.ctx().data_mut(|d| d.insert_temp(demo_zone_id(), zone.response.rect));
    }
    // egui's generic drop zone replaces the frame fill with a button color.
    // Keep the inventory surface and show a border while cargo is over it.
    if egui::DragAndDrop::has_any_payload(ui.ctx()) && zone.response.contains_pointer() {
        let carried = crate::ui::inventory::drag::carried_ids(ui.ctx());
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        // Items, folders and selections alike (LLTaskCategoryBridge::dragOrDrop),
        // all of them accepted or none.
        let check = carried
            .as_ref()
            .map(|ids| tool.ui.contents_drop.check(world, ids, idx, ctrl, contents_drop::Target::Panel));
        let allowed = check.is_some_and(|c| c.is_ok());
        ui.painter().rect_stroke(
            zone.response.rect,
            ui.visuals().window_corner_radius,
            egui::Stroke::new(1.0, if allowed { p.violet_light } else { p.muted_dim }),
            egui::StrokeKind::Inside,
        );
        if carried.is_some() {
            crate::ui::inventory::drag::feedback(
                ui.ctx(),
                if allowed {
                    egui::CursorIcon::Copy
                } else {
                    egui::CursorIcon::NotAllowed
                },
            );
        }
        if let Some(ids) = carried
            && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
        {
            egui::DragAndDrop::clear_payload(ui.ctx());
            match contents_drop::plan(world, &ids, idx, ctrl, contents_drop::Target::Panel) {
                Ok(drops) => tool.drop_inventory(world, idx, drops),
                Err(reason) => tool.status = contents_drop::reason_text(reason).into(),
            }
        }
    }
}

/// `AURORA_DEMO_CONTENTS_DROP=panel…` carries inventory onto the list.
fn demo_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_DEMO").as_deref() == Ok("1") && std::env::var_os("AURORA_DEMO_CONTENTS_DROP").is_some())
}

fn demo_zone_id() -> egui::Id {
    egui::Id::unique("contents_drop_demo_zone")
}

/// Where the Contents list was drawn, for the demo pointer.
pub(crate) fn demo_zone(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp(demo_zone_id()))
}

#[allow(clippy::too_many_arguments)]
fn item_row(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    idx: usize,
    object_id: uuid::Uuid,
    item: &TaskItem,
    editable: bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        widgets::inventory_spacing(ui);
        if let Some(t) = crate::ui::icons::global(item_icon(item.asset_type as i32, item.inv_type as i32, item.flags)) {
            let color = crate::ui::inventory::item_icon_color(item.asset_type as i32, item.inv_type as i32, p);
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(color));
        } else {
            ui.add_space(16.0);
        }
        // renaming in place
        if contents_selection::rename_row(ui, tool, world, object_id, item.item_id) {
            return;
        }
        let running = tool.contents.running.get(&(object_id, item.item_id)).copied();
        let mut name = display_name(item);
        if item.asset_type == AT_LSL_TEXT && running.is_some_and(|r| !r.0) {
            name.push_str(aurora_i18n::tr!("build-ui-contents-stopped"));
        }
        let permissions = Permissions::new(item.owner_mask, item.next_owner_mask).with_next_owner(tool.ui.contents_show_next_owner);
        let selected = tool.ui.contents_selection.object == Some(object_id) && tool.ui.contents_selection.items.contains(&item.item_id);
        let r = ui
            .push_id(("task_item", object_id, item.item_id), |ui| permissions.row(ui, p, &name, selected))
            .inner
            .on_hover_ui(|ui| {
                ui.label(&name);
                if !item.description.is_empty() {
                    ui.label(&item.description);
                }
                widgets::divider(ui, p);
                ui.label(permissions.tooltip());
            });
        if r.clicked() || r.secondary_clicked() {
            tool.ui
                .contents_selection
                .select(ui, object_id, item.item_id, r.secondary_clicked());
        }
        if r.drag_started() && !selected {
            tool.ui.contents_selection.select(ui, object_id, item.item_id, false);
        }
        let item_modify = item.owner_mask & perm::MODIFY != 0;
        // A selected row carries the whole selection (LLFolderView::startDrag).
        let dragged = if r.hovered() || r.drag_started() || r.dragged() {
            contents_selection::acted_on(tool, object_id, item)
        } else {
            vec![item.clone()]
        };
        crate::ui::inventory::task_drop::source(
            ui,
            &r,
            world,
            object_id,
            &dragged,
            crate::build::task_inventory::owned(world, object_id, tool.props.get(&object_id)),
        );
        let script = item.asset_type == AT_LSL_TEXT;
        menu::context_menu(&r, p, |ui| {
            let taken = contents_selection::acted_on(tool, object_id, item);
            let transfers: Result<Vec<_>, _> = taken.iter().map(|it| tool.task_transfer_kind(world, object_id, it)).collect();
            let can_take = transfers.is_ok();
            let can_open = script || item.asset_type == 7;
            let no_open = if can_open {
                ""
            } else {
                aurora_i18n::tr!("build-ui-contents-opening-items-inside-an-object-is-not-available-yet")
            };
            if menu::item_tip(ui, p, "folder-open-filled", aurora_i18n::tr!("common-open"), can_open, no_open) {
                if script {
                    tool.ui.requests.push(super::Request::Script {
                        object: object_id,
                        item: item.item_id,
                    });
                } else {
                    tool.ui.open_notecard = Some((object_id, item.item_id));
                }
            }
            if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("common-properties")) {
                tool.open_task_item_properties(object_id, item);
            }
            if menu::item_if(
                ui,
                p,
                "pencil-filled",
                aurora_i18n::tr!("common-rename"),
                editable && item_modify && tool.ui.contents_selection.items.len() == 1,
            ) {
                contents_selection::begin_rename(tool, world, object_id, item.item_id);
            }
            if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), editable) {
                contents_selection::delete_selected(tool, world);
            }
            // a no-copy item leaves the object: the row says so
            let moves = transfers.as_ref().is_ok_and(|kinds| {
                kinds
                    .iter()
                    .any(|kind| matches!(kind, crate::build::task_inventory::TransferKind::Move { .. }))
            });
            let take = if moves {
                aurora_i18n::tr!("task-transfer-move-menu")
            } else {
                aurora_i18n::tr!("build-ui-contents-copy-to-inventory")
            };
            let take_tip = aurora_i18n::tr!("build-ui-contents-like-dragging-the-item-to-inventory");
            if menu::item_tip(ui, p, "briefcase-filled", take, can_take, take_tip) {
                for it in &taken {
                    tool.copy_item_to_inventory(world, idx, it, uuid::Uuid::nil());
                }
            }
            if item.asset_type == AT_LSL_TEXT {
                menu::separator(ui, p);
                let on = running.map(|r| r.0).unwrap_or(true);
                let (icon, label) = if on {
                    ("player-stop-filled", aurora_i18n::tr!("common-stop"))
                } else {
                    ("player-play-filled", aurora_i18n::tr!("build-ui-contents-run"))
                };
                if menu::item_if(ui, p, icon, label, editable) {
                    tool.set_script_running(world, idx, item.item_id, !on);
                }
            }
        });
        if r.double_clicked() {
            if script {
                tool.ui.requests.push(super::Request::Script {
                    object: object_id,
                    item: item.item_id,
                });
            } else if item.asset_type == 7 {
                tool.ui.open_notecard = Some((object_id, item.item_id));
            } else if editable && item_modify {
                contents_selection::begin_rename(tool, world, object_id, item.item_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_opens_scripts_and_notecards_even_read_only_and_never_starts_renaming() {
        for (asset_type, editable) in [(7, true), (7, false), (AT_LSL_TEXT, true), (AT_LSL_TEXT, false)] {
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut tool = BuildTool::default();
            let object = uuid::Uuid::from_u128(1);
            let item = TaskItem {
                item_id: uuid::Uuid::from_u128(2),
                name: "Note".into(),
                asset_type,
                owner_mask: perm::COPY | perm::MODIFY | perm::TRANSFER,
                ..Default::default()
            };
            let mut time = 0.0;
            let mut frame = |events: Vec<egui::Event>, tool: &mut BuildTool| {
                time += 0.04;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 400.0))),
                        time: Some(time),
                        focused: true,
                        events,
                        ..Default::default()
                    },
                    |ui| item_row(ui, &p, tool, &mut world, 0, object, &item, editable),
                );
                // This headless test inspects shapes without uploading textures.
                output.textures_delta.clear();
                output
            };
            let output = frame(Vec::new(), &mut tool);
            let pos = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text().starts_with("Note ") => Some(t.pos + t.galley.size() * 0.5),
                    _ => None,
                })
                .expect("note row");
            for _ in 0..2 {
                for pressed in [true, false] {
                    frame(
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        &mut tool,
                    );
                }
            }
            if asset_type == AT_LSL_TEXT {
                assert!(tool.ui.open_notecard.is_none());
                assert_eq!(tool.ui.requests.len(), 1);
                assert!(matches!(
                    tool.ui.requests.first(),
                    Some(super::super::Request::Script { object: opened, item: opened_item })
                    if *opened == object && *opened_item == item.item_id
                ));
            } else {
                assert_eq!(tool.ui.open_notecard, Some((object, item.item_id)));
                assert!(tool.ui.requests.is_empty());
            }
            assert!(tool.ui.rename.is_none());
            assert_eq!(tool.ui.contents_selection.object, Some(object));
            assert_eq!(tool.ui.contents_selection.items, std::collections::HashSet::from([item.item_id]));
        }
    }

    #[test]
    fn labels() {
        assert_eq!(root_label(0), aurora_i18n::tr!("contents-count", count = 0));
        assert_eq!(root_label(3), aurora_i18n::tr!("contents-count", count = 3));
        let i = TaskItem {
            name: "New Script".into(),
            asset_type: AT_LSL_TEXT,
            owner_mask: perm::COPY | perm::MODIFY,
            ..Default::default()
        };
        assert_eq!(display_name(&i), aurora_i18n::tr!("common-new-script"));
        assert!(searchable_name(&i).contains(aurora_i18n::tr!("build-ui-contents-no-transfer")));
    }
}
