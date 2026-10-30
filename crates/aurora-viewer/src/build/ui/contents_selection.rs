//! Focus, multiple selection and in-place rename for task inventory.
//! LLPanelObjectInventory::handleKeyHere and LLFolderView selection/renamer
//! (Firestorm llpanelobjectinventory.cpp / llfolderview.cpp, LGPL 2.1).

use crate::build::{BuildTool, Tool};
use crate::world::World;
use aurora_net::build::{TaskItem, perm};
use egui::{Id, Key};
use std::collections::HashMap;
use uuid::Uuid;

pub use crate::ui::inventory::task_selection::Selection;
pub(super) use crate::ui::inventory::task_selection::focus_id;

pub struct Rename {
    pub object: Uuid,
    pub item: Uuid,
    pub text: String,
    focus: bool,
}

fn rename_id(edit: &Rename) -> Id {
    Id::unique(("task_contents_rename", edit.object, edit.item))
}

pub(super) fn begin_rename(tool: &mut BuildTool, world: &World, object: Uuid, item: Uuid) {
    let Some(idx) = world.objects.index_of_uuid(&object) else { return };
    if !tool.contents_editable(world, idx) || tool.ui.contents_selection.items.len() != 1 {
        return;
    }
    if tool.ui.contents_selection.object != Some(object) || !tool.ui.contents_selection.items.contains(&item) {
        return;
    }
    let Some(item) = tool.contents.objects.get(&object).and_then(|inv| {
        inv.items
            .iter()
            .find(|it| it.item_id == item && !it.is_folder && it.owner_mask & perm::MODIFY != 0)
    }) else {
        return;
    };
    tool.ui.rename = Some(Rename {
        object,
        item: item.item_id,
        text: item.name.clone(),
        focus: true,
    });
}

impl BuildTool {
    pub(crate) fn begin_contents_rename(&mut self, world: &World, object: Uuid, item: Uuid) {
        begin_rename(self, world, object, item);
    }
}

pub(super) fn finish_rename(tool: &mut BuildTool, world: &World) {
    let Some(edit) = tool.ui.rename.take() else { return };
    let Some(idx) = world.objects.index_of_uuid(&edit.object) else {
        return;
    };
    if !tool.contents_editable(world, idx) {
        return;
    }
    let Some(item) = tool.contents.objects.get(&edit.object).and_then(|inv| {
        inv.items
            .iter()
            .find(|it| it.item_id == edit.item && !it.is_folder && it.owner_mask & perm::MODIFY != 0)
    }) else {
        return;
    };
    let Ok(name) = crate::world::inventory::actions::clean_name(&edit.text) else {
        return;
    };
    if name != item.name {
        tool.rename_item(world, idx, edit.item, &name);
    }
}

/// Runs after the panel too: clicking a later row/widget can remove focus
/// after the renamer's own response has already been evaluated this pass.
pub(super) fn finish_if_hidden_or_blurred(ctx: &egui::Context, tool: &mut BuildTool, world: &World) {
    let Some(edit) = tool.ui.rename.as_ref() else { return };
    // egui can release a text editor's focus at the start of an Escape frame,
    // before the build shortcut dispatcher or the row runs. Cancel first.
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        let object = edit.object;
        tool.ui.rename = None;
        ctx.memory_mut(|m| m.request_focus(focus_id(object)));
        return;
    }
    let visible = tool.open
        && (tool.tool == Tool::Edit || (tool.ui.legacy_window && tool.tool != Tool::Land))
        && tool.tab == 4
        && tool
            .contents_target(world)
            .and_then(|idx| world.objects.get(idx))
            .is_some_and(|o| o.full_id == edit.object);
    if !visible || (!edit.focus && !ctx.memory(|m| m.has_focus(rename_id(edit)))) {
        finish_rename(tool, world);
    }
}

pub(super) fn rename_row(ui: &mut egui::Ui, tool: &mut BuildTool, world: &World, object: Uuid, item: Uuid) -> bool {
    let Some(edit) = tool.ui.rename.as_mut().filter(|edit| edit.object == object && edit.item == item) else {
        return false;
    };
    let id = rename_id(edit);
    // TextEdit consumes Escape/Enter while changing focus. Keep the user's
    // intent before adding it, so Escape cannot become a blur commit.
    let cancel = ui.input(|i| i.key_pressed(Key::Escape));
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let r = ui.add(
        egui::TextEdit::singleline(&mut edit.text)
            .id(id)
            .desired_width(160.0)
            .char_limit(63),
    );
    if cancel {
        tool.ui.rename = None;
        ui.memory_mut(|m| m.request_focus(focus_id(object)));
    } else if edit.focus && !enter {
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(edit.text.chars().count()),
            )));
            state.store(ui.ctx(), id);
        }
        edit.focus = false;
    } else if enter || !r.has_focus() {
        finish_rename(tool, world);
        if enter {
            ui.memory_mut(|m| m.request_focus(focus_id(object)));
        }
    }
    true
}

/// The items a drag or "Copy to inventory" of `item` acts on: the whole
/// selection, in list order, when `item` is part of it; `item` alone otherwise.
pub(super) fn acted_on(tool: &BuildTool, object: Uuid, item: &TaskItem) -> Vec<TaskItem> {
    let selection = &tool.ui.contents_selection;
    if selection.object != Some(object) || selection.items.len() < 2 || !selection.items.contains(&item.item_id) {
        return vec![item.clone()];
    }
    let Some(inv) = tool.contents.objects.get(&object) else {
        return vec![item.clone()];
    };
    let by_id: HashMap<Uuid, &TaskItem> = inv.items.iter().map(|it| (it.item_id, it)).collect();
    selection
        .order
        .iter()
        .filter(|id| selection.items.contains(id))
        .filter_map(|id| by_id.get(id).map(|it| (*it).clone()))
        .collect()
}

pub(super) fn delete_selected(tool: &mut BuildTool, world: &World) {
    let selection = &tool.ui.contents_selection;
    let Some(object) = selection.object else { return };
    let Some(idx) = world.objects.index_of_uuid(&object) else { return };
    if !tool.contents_editable(world, idx) {
        return;
    }
    let ids: Vec<_> = selection.order.iter().filter(|id| selection.items.contains(id)).copied().collect();
    let valid = tool
        .contents
        .objects
        .get(&object)
        .is_some_and(|inv| ids.iter().all(|id| inv.items.iter().any(|it| it.item_id == *id && !it.is_folder)));
    if !valid {
        return;
    }
    for id in ids {
        tool.remove_item(world, idx, id);
    }
    tool.ui.contents_selection.items.clear();
}

/// The list owns Delete even when empty/read-only. Dispatch before global
/// object shortcuts, and consume repeated presses so none reaches ObjectDelete.
pub(super) fn shortcuts(ctx: &egui::Context, tool: &mut BuildTool, world: &World) -> bool {
    finish_if_hidden_or_blurred(ctx, tool, world);
    if !tool.open
        || (tool.tool != Tool::Edit && !(tool.ui.legacy_window && tool.tool != Tool::Land))
        || tool.tab != 4
        || ctx.text_edit_focused()
    {
        return false;
    }
    let selection = &tool.ui.contents_selection;
    let Some(object) = selection.object else { return false };
    if !ctx.memory(|m| m.has_focus(focus_id(object))) {
        return false;
    }
    let allowed = !egui::Popup::is_any_open(ctx)
        && selection.layer.is_some_and(|layer| ctx.memory(|m| m.allows_interaction(layer)))
        && tool
            .contents_target(world)
            .and_then(|idx| world.objects.get(idx))
            .is_some_and(|o| o.full_id == object);
    if allowed
        && ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(event,
        egui::Event::Key { key: Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::B, pressed: true, modifiers, .. }
            if (modifiers.command || modifiers.ctrl) && !modifiers.shift && !modifiers.alt)
            })
        })
    {
        ctx.memory_mut(|m| m.surrender_focus(focus_id(object)));
        return false;
    }
    let mut keys = Vec::new();
    let mut handled = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
                && ((modifiers.is_none() && matches!(key, Key::Delete | Key::F2))
                    || (*key == Key::A && (modifiers.command || modifiers.ctrl) && !modifiers.shift && !modifiers.alt))
            {
                handled = true;
                if !repeat {
                    keys.push(*key);
                }
                false
            } else {
                true
            }
        })
    });
    if allowed {
        for key in keys {
            match key {
                Key::Delete => delete_selected(tool, world),
                Key::F2 => {
                    if let Some(item) = tool.ui.contents_selection.items.iter().next().copied()
                        && tool.ui.contents_selection.items.len() == 1
                    {
                        begin_rename(tool, world, object, item);
                    }
                }
                Key::A => tool
                    .ui
                    .contents_selection
                    .items
                    .extend(tool.ui.contents_selection.order.iter().copied()),
                _ => {}
            }
        }
    }
    handled
}
