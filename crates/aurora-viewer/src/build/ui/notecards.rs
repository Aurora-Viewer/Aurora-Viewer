//! Inventory-backed construction profiles. Downloads are validated in full
//! before applying; saves succeed only on the matching upload acknowledgement.

use super::{Request, common, compact};
use crate::build::{BuildSettings, BuildTool, layout::Profile};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::{World, inventory::actions};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Export,
    Load,
}

#[derive(Default)]
pub struct State {
    pub open: Option<Mode>,
    pub name: String,
    pub selected: Option<Uuid>,
    pub filter: String,
    pub pending_save: Option<Uuid>,
    pub pending_load: Option<Uuid>,
    pub message: String,
}

pub fn saved(tool: &mut BuildTool, request: Uuid, result: &Result<aurora_net::inventory::InvItem, String>) -> bool {
    if tool.ui.notecards.pending_save != Some(request) {
        return false;
    }
    tool.ui.notecards.pending_save = None;
    tool.ui.notecards.message = match result {
        Ok(_) => aurora_i18n::tr!("build-layout-saved").into(),
        Err(reason) => reason.clone(),
    };
    true
}

pub fn loaded(tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, item: Uuid, result: &Result<Vec<u8>, String>) -> bool {
    if tool.ui.notecards.pending_load != Some(item) {
        return false;
    }
    tool.ui.notecards.pending_load = None;
    let result = result
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|data| aurora_net::inventory::notecard::Notecard::decode(data))
        .and_then(|note| Profile::read(&note.text));
    tool.ui.notecards.message = match result {
        Ok(profile) => {
            profile.apply(tool, world, s);
            aurora_i18n::tr!("build-layout-loaded").into()
        }
        Err(reason) => reason,
    };
    true
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &BuildSettings) {
    let Some(mode) = tool.ui.notecards.open else { return };
    let saving = mode == Mode::Export;
    let mut close = false;
    let modal = widgets::Modal::new(egui::Id::unique("build-layout-notecard")).show(ctx, |ui| {
        common::typography(ui);
        ui.set_width(330.0);
        ui.horizontal(|ui| {
            common::section(
                ui,
                p,
                if saving {
                    aurora_i18n::tr!("build-layout-export-notecard")
                } else {
                    aurora_i18n::tr!("build-layout-load-notecard")
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close |= compact::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true).clicked();
            });
        });
        if saving {
            common::label(ui, p, aurora_i18n::tr!("common-name-2"));
            ui.add(
                egui::TextEdit::singleline(&mut tool.ui.notecards.name)
                    .desired_width(ui.available_width())
                    .char_limit(63),
            );
        } else {
            ui.add(
                egui::TextEdit::singleline(&mut tool.ui.notecards.filter)
                    .desired_width(ui.available_width())
                    .hint_text(aurora_i18n::tr!("build-layout-filter-notecards")),
            );
            let filter = tool.ui.notecards.filter.to_lowercase();
            let mut items: Vec<_> = world
                .inventory
                .items
                .values()
                .filter(|item| {
                    item.asset_type == 7
                        && item.owner_mask & actions::COPY != 0
                        && !world.inventory.in_folder_type(item.id, 14)
                        && item.name.to_lowercase().contains(&filter)
                })
                .map(|item| (item.id, item.name.clone()))
                .collect();
            items.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
            egui::ScrollArea::vertical()
                .id_salt("layout-notecard-list")
                .max_height(260.0)
                .show(ui, |ui| {
                    if items.is_empty() {
                        common::small(ui, p, aurora_i18n::tr!("build-layout-no-notecards"));
                    }
                    for (id, name) in items {
                        if ui.selectable_label(tool.ui.notecards.selected == Some(id), name).clicked() {
                            tool.ui.notecards.selected = Some(id);
                        }
                    }
                });
        }
        if !tool.ui.notecards.message.is_empty() {
            common::small(ui, p, &tool.ui.notecards.message);
        }
        ui.horizontal(|ui| {
            close |= ui.button(aurora_i18n::tr!("common-cancel")).clicked();
            let busy = tool.ui.notecards.pending_save.is_some() || tool.ui.notecards.pending_load.is_some();
            let enabled = !busy
                && (saving || world.inventory.can_preview(7))
                && if saving {
                    !tool.ui.notecards.name.trim().is_empty()
                } else {
                    tool.ui.notecards.selected.is_some_and(|id| {
                        world.inventory.items.get(&id).is_some_and(|item| {
                            item.asset_type == 7 && item.owner_mask & actions::COPY != 0 && !world.inventory.in_folder_type(id, 14)
                        })
                    })
                };
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(if saving {
                        aurora_i18n::tr!("build-layout-export")
                    } else {
                        aurora_i18n::tr!("build-layout-reload")
                    }),
                )
                .clicked()
            {
                if saving {
                    match serde_json::to_string_pretty(&Profile::capture(tool, s)) {
                        Ok(text) => {
                            tool.ui.requests.push(Request::ExportLayoutNotecard {
                                name: tool.ui.notecards.name.trim().into(),
                                text,
                            });
                            close = true;
                        }
                        Err(_) => tool.ui.notecards.message = aurora_i18n::tr!("build-layout-invalid-notecard").into(),
                    }
                } else if let Some(id) = tool.ui.notecards.selected {
                    tool.ui.requests.push(Request::LoadLayoutNotecard(id));
                    close = true;
                }
            }
        });
    });
    if close || modal.should_close() {
        tool.ui.notecards.open = None;
    }
}
