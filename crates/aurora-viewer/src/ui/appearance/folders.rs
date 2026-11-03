//! Groups of My Outfits in Appearance (#1002): their rows in the list,
//! their sections in the gallery, their menu, "Move to folder" and the New,
//! Rename and Delete dialogs. An Aurora addition: the Legacy Viewer lists
//! the outfits of these folders flat (LLOutfitListBase::refreshList) and
//! leaves the folders to the inventory; the rules are those of
//! `world/outfit_folders.rs`.

use super::{Action, AppearanceUi};
use crate::{
    theme::Palette,
    ui::{icons, menu, widgets},
    world::{World, inventory::Inventory, outfit_folders as folders},
};
use egui::{RichText, Vec2};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct State {
    dialog: Option<Dialog>,
    /// The name field takes the focus, its text selected, when it opens.
    focus: bool,
}

enum Dialog {
    /// A new group in `parent`, which takes `then_move` at once.
    New {
        parent: Uuid,
        name: String,
        then_move: Vec<Uuid>,
    },
    Rename {
        id: Uuid,
        name: String,
    },
    Delete(Uuid),
}

impl AppearanceUi {
    /// Opens or closes a group, in the list and the gallery alike.
    pub(super) fn toggle_group(&mut self, id: Uuid) {
        if !self.closed_groups.remove(&id) {
            self.closed_groups.insert(id);
        }
        self.closed_version = self.closed_version.wrapping_add(1);
    }

    /// Opens the groups that hold `id`, so that it can be shown.
    pub(super) fn open_groups_of(&mut self, inv: &Inventory, id: Uuid) {
        let mut at = inv.folders.get(&id).map(|f| f.info.parent);
        while let Some(group) = at.filter(|g| folders::is_group(inv, *g)) {
            if self.closed_groups.remove(&group) {
                self.closed_version = self.closed_version.wrapping_add(1);
            }
            at = inv.folders.get(&group).map(|f| f.info.parent);
        }
    }

    pub(super) fn begin_new_folder(&mut self, parent: Uuid, then_move: Vec<Uuid>) {
        self.folders.dialog = Some(Dialog::New {
            parent,
            name: aurora_i18n::tr!("common-new-folder").into(),
            then_move,
        });
        self.folders.focus = true;
    }
}

/// The header row of a group (or, with no `open`, of the gallery's
/// section of outfits outside any group): a caret, the folder, its name
/// and, at the end, how many outfits it holds. It takes the inventory's
/// row height, `depth` indents below the groups above it; a click
/// opens or closes it.
pub(super) fn header(ui: &mut egui::Ui, p: &Palette, title: &str, count: usize, open: Option<bool>, depth: usize) -> egui::Response {
    const ICON: f32 = 14.0;
    const CARET: f32 = 12.0;
    let unit = ui.spacing().indent;
    let height = crate::ui::inventory::AURORA_ROW_HEIGHT;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::click());
    response.widget_info(|| {
        if open.is_some() {
            egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, ui.is_enabled(), title)
        } else {
            egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), title)
        }
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let row = egui::Rect::from_min_max(egui::pos2(rect.left() + depth as f32 * unit, rect.top()), rect.max);
    let hot = open.is_some() && (response.hovered() || response.has_focus());
    let painter = ui.painter();
    if hot {
        painter.rect_filled(row, ui.visuals().window_corner_radius, p.raised);
    }
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    let mut x = row.left();
    if let Some(open) = open {
        let caret = if open { "caret-down-filled" } else { "caret-right-filled" };
        if let Some(t) = icons::global(caret) {
            let at = egui::Rect::from_center_size(egui::pos2(x + unit / 2.0, row.center().y), Vec2::splat(CARET));
            painter.image(t.id(), at, uv, if hot { p.ink } else { p.muted });
        }
        x += unit;
    }
    let icon = if open.is_some() { "folder-filled" } else { "hanger-2-filled" };
    if let Some(t) = icons::global(icon) {
        let at = egui::Rect::from_center_size(egui::pos2(x + ICON / 2.0, row.center().y), Vec2::splat(ICON));
        painter.image(t.id(), at, uv, p.violet_light);
    }
    x += ICON + 6.0;
    let count = painter.layout_no_wrap(
        if count == 0 {
            aurora_i18n::tr!("ui-appearance-folders-empty").to_string()
        } else {
            aurora_i18n::tr!("outfit-count", count = count)
        },
        egui::FontId::proportional(11.0),
        p.muted,
    );
    let count_left = row.right() - 6.0 - count.size().x;
    painter.galley(egui::pos2(count_left, row.center().y - count.size().y / 2.0), count, p.muted);
    let name = egui::WidgetText::from(RichText::new(title).strong().color(p.ink)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (count_left - 8.0 - x).max(1.0),
        egui::TextStyle::Button,
    );
    let origin = widgets::row_text_origin(ui.painter(), &name, x, row.center().y);
    ui.painter().galley(origin, name, p.ink);
    response
}

/// The menu of a group's header: a folder inside it, rename, move, delete.
pub(super) fn group_menu(
    response: &egui::Response,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    id: Uuid,
    actions: &mut Vec<Action>,
) {
    menu::context_menu(response, p, |ui| {
        let inv = &world.inventory;
        let Some(folder) = inv.folders.get(&id) else { return };
        let too_deep = folders::depth(inv, id).is_none_or(|d| d >= folders::MAX_DEPTH);
        if menu::item_if(ui, p, "folder-filled", aurora_i18n::tr!("ui-appearance-folders-new"), !too_deep) {
            st.begin_new_folder(id, Vec::new());
        }
        if menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("ui-appearance-folders-rename")) {
            st.folders.dialog = Some(Dialog::Rename {
                id,
                name: folder.info.name.clone(),
            });
            st.folders.focus = true;
        }
        move_menu(ui, p, inv, st, &[id], actions);
        menu::separator(ui, p);
        // getCanRemoveOutfit keeps the worn outfit, and so the group holding it
        let trash = folders::trash(inv, id).is_ok();
        let delete = ui.add_enabled_ui(trash, |ui| {
            menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("ui-appearance-folders-delete"), trash)
        });
        if folders::holds_current(inv, id) {
            delete
                .response
                .on_disabled_hover_text(aurora_i18n::tr!("ui-appearance-folders-holds-current"));
        }
        if delete.inner {
            st.folders.dialog = Some(Dialog::Delete(id));
        }
    });
}

/// "New folder" of the gear menu and of the gallery's section of outfits
/// outside any group: a group in My Outfits.
pub(super) fn new_folder_item(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut AppearanceUi) {
    if let Some(my) = folders::my_outfits(inv)
        && menu::item(ui, p, "folder-filled", aurora_i18n::tr!("ui-appearance-folders-new"))
    {
        st.begin_new_folder(my, Vec::new());
    }
}

/// "Move to folder": My Outfits and every group, the one holding all of
/// `ids` checked, those that cannot take them greyed; then a new folder
/// that takes them at once.
pub(super) fn move_menu(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut AppearanceUi, ids: &[Uuid], actions: &mut Vec<Action>) {
    let movable = !ids.is_empty() && ids.iter().all(|id| folders::in_my_outfits(inv, *id) || folders::is_group(inv, *id));
    menu::submenu(
        ui,
        p,
        "folder-filled",
        aurora_i18n::tr!("ui-appearance-folders-move"),
        movable,
        |ui| {
            let parents: Vec<_> = ids.iter().filter_map(|id| inv.folders.get(id).map(|f| f.info.parent)).collect();
            // Keep the popup within the viewport, leaving its New folder action
            // outside the scrolling list even for wardrobes with many groups.
            egui::ScrollArea::vertical()
                .id_salt("outfit_move_targets")
                .max_height((ui.ctx().content_rect().height() * 0.6).min(440.0))
                .show(ui, |ui| {
                    for (target, depth) in folders::targets(inv) {
                        let name = if depth == 0 {
                            aurora_i18n::tr!("common-my-outfits").to_string()
                        } else {
                            inv.folders.get(&target).map(|f| f.info.name.clone()).unwrap_or_default()
                        };
                        let label = format!("{}{name}", "   ".repeat(depth));
                        let here = parents.iter().all(|p| *p == target);
                        let allowed = ids.iter().all(|id| folders::can_move(inv, *id, target));
                        if menu::check(ui, p, "folder-filled", &label, here, allowed && !here)
                            && let Ok(change) = folders::move_into(inv, ids, target)
                        {
                            actions.push(Action::Folders(change));
                        }
                    }
                });
            menu::separator(ui, p);
            if let Some(my) = folders::my_outfits(inv)
                && menu::item(ui, p, "folder-filled", aurora_i18n::tr!("ui-appearance-folders-new"))
            {
                st.begin_new_folder(my, ids.to_vec());
            }
        },
    );
}

/// The New, Rename and Delete dialogs of the groups.
pub(super) fn dialogs(ctx: &egui::Context, p: &Palette, world: &World, st: &mut AppearanceUi, actions: &mut Vec<Action>) {
    let inv = &world.inventory;
    let Some(dialog) = st.folders.dialog.as_mut() else {
        return;
    };
    let mut done = false;
    let response = widgets::Modal::new(egui::Id::unique("outfit_folder_dialog")).show(ctx, |ui| {
        ui.set_width(350.0);
        let (title, name) = match dialog {
            Dialog::New { name, .. } => (aurora_i18n::tr!("ui-appearance-folders-new"), Some(name)),
            Dialog::Rename { name, .. } => (aurora_i18n::tr!("ui-appearance-folders-rename"), Some(name)),
            Dialog::Delete(_) => (aurora_i18n::tr!("ui-appearance-folders-delete"), None),
        };
        ui.label(RichText::new(title.trim_end_matches('…')).size(17.0).color(p.ink));
        ui.add_space(8.0);
        let mut valid = true;
        if let Some(name) = name {
            let r = ui.add(egui::TextEdit::singleline(name).char_limit(63).desired_width(f32::INFINITY));
            if st.folders.focus {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ctx, r.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                    state.store(ctx, r.id);
                }
                st.folders.focus = false;
            }
            valid = crate::world::inventory::actions::clean_name(name).is_ok();
        } else if let Dialog::Delete(id) = dialog {
            let name = inv.folders.get(id).map(|f| f.info.name.clone()).unwrap_or_default();
            let count = folders::tree(inv, "", false, folders::Order::List)
                .iter()
                .find(|e| e.id == *id)
                .map_or(0, |e| e.outfits);
            ui.label(aurora_i18n::tr!("ui-appearance-folders-delete-confirm", name = name, count = count));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            use widgets::{ActionKind, action_button, action_button_if};
            let size = Vec2::new(0.0, 26.0);
            let (icon, label, kind) = if matches!(dialog, Dialog::Delete(_)) {
                ("trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger)
            } else {
                ("check", aurora_i18n::tr!("common-ok"), ActionKind::Primary)
            };
            let enter = widgets::enter_pressed(ui);
            if action_button_if(ui, p, icon, label, kind, size, valid).clicked() || (valid && enter) {
                let change = match dialog {
                    Dialog::New { parent, name, then_move } => {
                        if then_move.is_empty() {
                            folders::create(inv, *parent, name)
                        } else {
                            folders::create_and_move(inv, *parent, name, then_move)
                        }
                    }
                    Dialog::Rename { id, name } => folders::rename(inv, *id, name),
                    Dialog::Delete(id) => folders::trash(inv, *id),
                };
                match change {
                    Ok(change) => actions.push(Action::Folders(change)),
                    Err(reason) => st.message = reason,
                }
                done = true;
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                done = true;
            }
        });
    });
    if done || response.should_close() {
        st.folders.dialog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::{Frames, painted};

    #[test]
    fn long_move_menu_scrolls_to_last_folder_and_keeps_new_folder_visible() {
        for zoom in [1.0, 2.0] {
            let mut inv = Inventory::default();
            crate::world::appearance::seed_demo(&mut inv, Uuid::from_u128(1));
            folders::seed_demo(&mut inv, Uuid::from_u128(1));
            folders::seed_demo_many(&mut inv);
            let mut frames = Frames::new();
            frames.ctx.set_zoom_factor(zoom);
            let p = crate::theme::Theme::default().palette();
            let mut st = AppearanceUi::default();
            let mut actions = Vec::new();
            let mut open = true;
            {
                let mut draw = |ui: &mut egui::Ui| {
                    let screen = ui.ctx().content_rect();
                    menu::popup_at(
                        ui.ctx(),
                        egui::Id::unique("outfit_move_test"),
                        egui::pos2(80.0, screen.bottom() - 60.0),
                        &mut open,
                        &p,
                        |ui| move_menu(ui, &p, &inv, &mut st, &[Uuid::from_u128(744)], &mut actions),
                    );
                };
                let mut texts = Vec::new();
                for _ in 0..4 {
                    texts = frames.run(Vec::new(), &mut draw);
                }
                let heading = painted(&texts, aurora_i18n::tr!("ui-appearance-folders-move")).rect.center();
                texts = frames.hover(heading, &mut draw);
                let screen = frames.ctx.content_rect();
                let new_folder = painted(&texts, aurora_i18n::tr!("ui-appearance-folders-new")).rect;
                assert!(screen.contains_rect(new_folder), "New folder stays on screen at zoom {zoom}");
                assert!(!texts.iter().any(|t| t.source.trim() == "Style 319"));
                let top = painted(&texts, aurora_i18n::tr!("common-my-outfits")).rect.center();
                let pointer = top + Vec2::new(30.0, 70.0);
                frames.run(
                    vec![
                        egui::Event::PointerMoved(pointer),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: Vec2::new(0.0, -30_000.0),
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Unknown,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    &mut draw,
                );
                for _ in 0..60 {
                    texts = frames.run(Vec::new(), &mut draw);
                }
                let last = texts
                    .iter()
                    .find(|t| t.source.trim() == "Style 319")
                    .expect("last target after scrolling");
                assert!(screen.contains_rect(last.rect));
                assert_eq!(painted(&texts, aurora_i18n::tr!("ui-appearance-folders-new")).rect, new_folder);
                let pos = last.rect.center();
                for pressed in [true, false] {
                    frames.run(
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        &mut draw,
                    );
                }
            }
            let [Action::Folders(change)] = actions.as_slice() else {
                panic!("one folder move")
            };
            assert!(change.move_only());
            assert!(matches!(
                change.operations.as_slice(),
                [aurora_net::inventory::operations::Operation::Patch { id, folder: true, body }]
                    if *id == Uuid::from_u128(744) && body["parent_id"].as_uuid() == Uuid::from_u128(1_485_319)
            ));
        }
    }
}
