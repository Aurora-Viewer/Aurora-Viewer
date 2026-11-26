//! Floating blocks (Aurora's own, human request, 2026-11-25): a group of
//! toolbar buttons of its own on the screen, moved by its glass like the
//! quick import bar it generalises (`drawers::IMPORT_BAR`). Its buttons are
//! a drawer's (`Drawer::floating`, the same rules and edits); its place is a
//! window's (`ui_layout.ron`). Editing the bars shows every block, hidden or
//! not, and takes the buttons dropped on it.

use super::drawers::{self, IMPORT_BAR};
use super::{Bar, Drag, Edit, ToolbarUi, structure};
use crate::theme::Palette;
use crate::ui::{Panels, bars, icons::Icons, menu, widgets};
use egui::{Id, Vec2};

/// A button's icon, as in the dock (`bars/dock.rs`).
const ICON: f32 = bars::dock::BUTTON_ICON;

fn drag_id() -> Id {
    Id::unique("toolbar_floating_drag")
}

/// A floating block's button is dragged: the block moves with it.
pub(super) fn dragged(ctx: &egui::Context, command: &str) {
    ctx.data_mut(|data| data.insert_temp(drag_id(), command.to_owned()));
}

/// The window that keeps the block `id`'s place; the import bar keeps the
/// one its strip had.
pub(crate) fn window_id(id: &str) -> String {
    if id == IMPORT_BAR {
        crate::imports::LAUNCHER_WINDOW.to_owned()
    } else {
        format!("toolbar_floating_{}", id.trim_start_matches(drawers::PREFIX))
    }
}

/// The right click of a floating block, on its glass or its button (its
/// button and its shortcut show and hide it: no Show/Hide here, human
/// request, 2026-11-25; its settings window keeps Delete, the menu does not,
/// 2026-11-26).
pub(super) fn context(ui: &mut egui::Ui, p: &Palette, toolbar: &mut ToolbarUi, id: &str, locked: bool) {
    let settings = super::appearance::settings(ui.ctx());
    super::section(ui, p, aurora_i18n::tr!("toolbar-block-settings"));
    super::blocks::kind_menu(ui, p, toolbar, &settings, id, None);
    toolbar.block_shortcut_item(ui, p, &settings, id);
    if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-block-edit"), !locked) {
        toolbar.edit_drawer(id);
    }
}

/// Every floating block shown, or all of them while the bars are edited;
/// `import_shown` is the quick import bar's display.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    toolbar: &mut ToolbarUi,
    panels: &mut Panels,
    mic: &mut bars::MicButton,
    row: &bars::Row<'_>,
    import_shown: &mut bool,
) -> bars::BarAction {
    let settings = ToolbarUi::layout_configuration(ctx).unwrap_or_else(|| super::appearance::settings(ctx));
    let editing = ToolbarUi::editing(ctx);
    let hidden = widgets::interface_hidden(ctx);
    let mut action = bars::BarAction::None;
    for (id, drawer) in &settings.drawers {
        let Some(floating) = &drawer.floating else { continue };
        let import = id == IMPORT_BAR;
        let shown = if import { *import_shown } else { floating.shown };
        drawers::set_open(ctx, id, shown && !hidden);
        if hidden || !(shown || editing) {
            continue;
        }
        let window = window_id(id);
        // a block that closes hides once the click that asked for it has
        // left no popup, as a drawer folds (`drawers::close_after_click`)
        let kept = editing || toolbar.drawer_editor.as_deref() == Some(id.as_str());
        let close = drawers::area_id(id).with("close");
        if ctx.data(|data| data.get_temp::<bool>(close)).unwrap_or(false) && (kept || !egui::Popup::is_any_open(ctx)) {
            ctx.data_mut(|data| data.remove::<bool>(close));
            if floating.closes && !kept {
                if import {
                    *import_shown = false;
                } else {
                    toolbar.pending = Some(Edit::FloatingShown(id.clone(), false));
                }
                continue;
            }
        }
        if let Some((placed, pos)) = toolbar.floating_placed.clone()
            && placed == *id
        {
            toolbar.floating_placed = None;
            widgets::move_floater(ctx, &window, pos);
        }
        let vertical = floating.vertical;
        let bar = if vertical { Bar::Left } else { Bar::Top };
        let display = structure::Display::for_drawer(ctx, bar, &drawer.buttons, editing);
        // a bar block's buttons, as tall as the dock's (human request,
        // 2026-11-25), like an unfolded drawer's
        let square = bars::dock::button_size(ctx.content_rect());
        let buttons = display.buttons();
        let spacing = ctx.global_style().spacing.item_spacing;
        let widths: Vec<_> = buttons
            .iter()
            .map(|button| bars::natural_width(ctx, button, drawer.mode, square, ICON, square + 8.0))
            .collect();
        let heights: Vec<_> = buttons
            .iter()
            .map(|button| {
                if !vertical {
                    square
                } else if structure::is_divider(&button.command) {
                    structure::DIVIDER_SIZE
                } else if structure::is_separator(&button.command) {
                    structure::style(ctx, &button.command).size
                } else {
                    square
                }
            })
            .collect();
        // the column of a vertical block is as wide as its widest button
        let column = buttons
            .iter()
            .zip(&widths)
            .filter(|(button, _)| !structure::is_separator(&button.command) && !structure::is_divider(&button.command))
            .map(|(_, width)| *width)
            .fold(square, f32::max);
        let looks: Vec<_> = buttons
            .iter()
            .zip(widths.iter().zip(&heights))
            .map(|(button, (width, height))| {
                let width = if vertical && !structure::is_separator(&button.command) && !structure::is_divider(&button.command) {
                    column
                } else {
                    *width
                };
                (
                    button.command.clone(),
                    bars::ButtonLook {
                        size: Vec2::new(width, *height),
                        icon: ICON,
                        dock: true,
                        mode: drawer.mode,
                        left: vertical,
                        opens: if vertical {
                            egui::RectAlign::RIGHT
                        } else {
                            egui::RectAlign::BOTTOM_START
                        },
                        editing,
                    },
                )
            })
            .collect();
        let work = crate::ui::layout::work_area(ctx);
        let (point, start) = if import {
            let room = crate::ui::layout::move_area(ctx);
            (
                egui::Align2::CENTER_TOP,
                egui::pos2(room.center().x, work.top() + crate::ui::layout::GAP),
            )
        } else {
            (egui::Align2::CENTER_CENTER, work.center())
        };
        let glass = ctx.memory(|memory| memory.area_rect(Id::unique(&window)));
        widgets::toolstrip(ctx, &window, point, start, |ui| {
            // its right click, under its buttons: theirs stay theirs, and a
            // drag still moves the block
            if let Some(rect) = glass {
                let background = ui.interact(rect, Id::unique(("floating_glass", id)), egui::Sense::click());
                menu::context_menu_of_button(&background, p, |ui| {
                    context(ui, p, toolbar, id, settings.locked);
                });
            }
            // the glass is a bar block's, drawn by the row around its
            // buttons (`toolbar_row`, `Row::floating`)
            egui::Frame::new()
                .inner_margin(egui::Margin::same(bars::islands::PAD as i8))
                .show(ui, |ui| {
                    let mut rects = Vec::new();
                    // a row's buttons are as tall as one another: centred, the
                    // row took the screen's height and the block could not
                    // reach its top or its bottom (human report, 2026-11-25)
                    let layout = if vertical {
                        egui::Layout::top_down(egui::Align::Min)
                    } else {
                        egui::Layout::left_to_right(egui::Align::Min)
                    };
                    ui.with_layout(layout, |ui| {
                        ui.spacing_mut().item_spacing = if vertical {
                            Vec2::new(spacing.x, bars::islands::GAP)
                        } else {
                            Vec2::new(bars::islands::GAP, spacing.y)
                        };
                        if buttons.is_empty() {
                            widgets::island(ui, p, |ui| {
                                ui.add_sized(
                                    Vec2::new(160.0, square),
                                    egui::Label::new(egui::RichText::new(aurora_i18n::tr!("toolbar-floating-empty")).color(p.muted)),
                                );
                            });
                        }
                        let nested = bars::Row {
                            bar,
                            style: super::Style {
                                mode: drawer.mode,
                                align: super::Align::Start,
                                sizing: super::Sizing::Auto,
                            },
                            editing,
                            floating: true,
                            ..*row
                        };
                        rects = bars::toolbar_row(
                            ui,
                            p,
                            icons,
                            buttons,
                            |b| {
                                looks
                                    .iter()
                                    .find(|(command, _)| command == &b.command)
                                    .map(|(_, look)| *look)
                                    .unwrap_or_else(|| bars::ButtonLook::bar(square))
                            },
                            toolbar,
                            panels,
                            mic,
                            &nested,
                            &mut action,
                        );
                    });
                    let zone = ui.min_rect();
                    let target = ui.interact(zone, Id::unique(("floating_drop", id)), egui::Sense::hover());
                    super::drop_preview::protect(ctx, zone, ui.layer_id());
                    if !settings.locked
                        && target.dnd_hover_payload::<Drag>().is_some()
                        && let Some(drag) = target.dnd_release_payload::<Drag>()
                    {
                        ctx.data_mut(|data| data.insert_temp(Id::unique("toolbar_drop_handled"), true));
                        let rank = ctx.pointer_interact_pos().map_or(drawer.buttons.len(), |pos| {
                            super::insertion(&display.full_rects(&rects), pos, vertical)
                        });
                        if let Some(edit) = drawers::insertion_edit(id, rank, &drag) {
                            toolbar.pending = Some(edit);
                        }
                    }
                    if floating.closes && !kept {
                        let toggle = ctx
                            .read_response(bars::tool_button_id(id))
                            .map_or(egui::Rect::NOTHING, |response| response.rect);
                        drawers::close_on_click_outside(ctx, id, toggle, ui.layer_id());
                        drawers::close_after_click(ctx, id, buttons, &rects, zone, ui.layer_id(), editing);
                    }
                });
        });
        // a drag on one of its buttons moves the whole block
        if let Some(command) = ctx.data(|data| data.get_temp::<String>(drag_id()))
            && drawer.buttons.iter().any(|button| button.command == command)
        {
            ctx.data_mut(|data| data.remove_temp::<String>(drag_id()));
            let delta = ctx.input(|input| input.pointer.delta());
            if delta != Vec2::ZERO
                && let Some(rect) = ctx.memory(|memory| memory.area_rect(Id::unique(&window)))
            {
                widgets::move_floater(ctx, &window, rect.min + delta);
            }
        }
    }
    action
}
