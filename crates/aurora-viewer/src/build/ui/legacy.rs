//! Legacy Viewer construction layout: LLFloaterTools::updatePopup/refresh,
//! LLPanelObject and skins/default/xui/en/floater_tools.xml. The shared native
//! panels retain permissions, drafts, undo and simulator commands.

use super::{Env, common, compact, contents, features, general, layout, object, texture, tools};
use crate::build::layout::Item;
use crate::build::{BuildSettings, BuildTool, EditMode, Mods, Tool};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::World;
use egui::{RichText, Vec2};

pub(super) const DEFAULT_SIZE: Vec2 = egui::vec2(400.0, 820.0);
pub(super) const MIN_SIZE: Vec2 = egui::vec2(380.0, 480.0);

/// Section identities are shared with Aurora, so switching looks retains the
/// selected panel. The former separate Shape section belongs to Object here.
fn tab_index(tab: usize) -> usize {
    match tab {
        1 | 5 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        _ => 0,
    }
}

fn tool_strip(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool) {
    ui.horizontal(|ui| {
        let width = ((ui.available_width() - ui.spacing().item_spacing.x * 4.0) / 5.0).max(1.0);
        for (mode, icon, label) in [
            (Tool::Focus, "zoom-filled", aurora_i18n::tr!("build-compact-camera")),
            (Tool::Grab, "hand-grab", aurora_i18n::tr!("build-compact-grab")),
            (Tool::Edit, "pointer-filled", aurora_i18n::tr!("common-edit")),
            (Tool::Create, "wand", aurora_i18n::tr!("build-compact-create")),
            (Tool::Land, "mountain-filled", aurora_i18n::tr!("build-compact-land")),
        ] {
            let kind = if tool.tool == mode {
                widgets::ActionKind::Primary
            } else {
                widgets::ActionKind::Flat
            };
            let response = widgets::action_button(ui, p, icon, "", kind, Vec2::new(width, 30.0)).on_hover_text(label);
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, tool.tool == mode, label));
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::unique(("legacy-build-tool", mode as u8)), response.rect));
            if response.clicked() {
                tool.set_tool(mode);
            }
        }
    });
}

fn edit_options(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    ui.columns(2, |columns| {
        for column in columns.iter_mut() {
            column.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        }
        for (mode, label) in [
            (EditMode::Move, aurora_i18n::tr!("build-ui-tools-move")),
            (EditMode::Rotate, aurora_i18n::tr!("common-rotate")),
            (EditMode::Stretch, aurora_i18n::tr!("build-compact-stretch")),
            (EditMode::Face, aurora_i18n::tr!("build-compact-select-face")),
            (EditMode::Align, aurora_i18n::tr!("build-compact-align")),
        ] {
            if widgets::radio(
                &mut columns[0],
                p,
                tool.effective_mode(mods) == mode,
                RichText::new(label).size(common::BODY),
            )
            .clicked()
            {
                tool.set_edit_mode(world, s, mode);
            }
        }
        for item in [
            Item::ScaleBoth,
            Item::StretchTextures,
            Item::Snap,
            Item::RootAxis,
            Item::Highlight,
            Item::Probes,
        ] {
            layout::control(&mut columns[1], p, tool, world, s, mods, item);
        }
    });
    ui.horizontal(|ui| {
        layout::control(ui, p, tool, world, s, mods, Item::Linked);
    });
    ui.horizontal(|ui| {
        common::label(ui, p, aurora_i18n::tr!("build-legacy-ruler"));
        let label = match s.grid_mode {
            1 => aurora_i18n::tr!("build-compact-local"),
            2 => aurora_i18n::tr!("build-compact-reference"),
            _ => aurora_i18n::tr!("build-compact-world"),
        };
        egui::ComboBox::from_id_salt("legacy-grid-mode")
            .width(105.0)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (mode, label) in [
                    (0, aurora_i18n::tr!("build-compact-world")),
                    (1, aurora_i18n::tr!("build-compact-local")),
                    (2, aurora_i18n::tr!("build-compact-reference")),
                ] {
                    if ui.selectable_value(&mut s.grid_mode, mode, label).changed() && mode == 2 && tool.grid_objects.is_empty() {
                        tool.use_selection_for_grid(world, s);
                    }
                }
            });
        compact::action_row(ui, p, tool, world, s);
    });
}

pub(super) fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
) {
    tool_strip(ui, p, tool);
    common::small(ui, p, tools::status_text(tool, s, mods));
    ui.add_space(4.0);
    // Bound oversized tool options on short screens while retaining the tabs
    // and a usable property viewport. The tool strip itself never scrolls.
    let width = widgets::scrolled_width(ui).max(1.0);
    let options_height = (ui.available_height() * 0.55).clamp(96.0, 330.0);
    egui::ScrollArea::vertical()
        .id_salt(("legacy-build-options", tool.tool as u8))
        .max_height(options_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(width);
            match tool.tool {
                Tool::Focus => tools::focus_panel(ui, p, tool, mods, env),
                Tool::Grab => tools::grab_panel(ui, p, tool, mods),
                Tool::Edit => edit_options(ui, p, tool, world, s, mods),
                Tool::Create => tools::create_panel(ui, p, s),
                Tool::Land => tools::land_panel(ui, p, tool, world, s),
            }
        });
    if !s.expanded {
        return;
    }
    widgets::divider(ui, p);
    if tool.tool == Tool::Land {
        egui::ScrollArea::vertical().id_salt("legacy-build-parcel").show(ui, |ui| {
            tools::parcel_panel(ui, p, tool, world, s, env.property_lines);
        });
        return;
    }
    compact::selection(ui, p, tool, world, s);
    let mut tab = tab_index(tool.tab);
    if widgets::tabs(
        ui,
        p,
        &mut tab,
        &[
            (aurora_i18n::tr!("build-compact-general"), true),
            (aurora_i18n::tr!("build-legacy-object"), true),
            (aurora_i18n::tr!("build-compact-features"), true),
            (aurora_i18n::tr!("build-legacy-texture"), true),
            (aurora_i18n::tr!("build-compact-contents"), true),
        ],
    ) {
        tool.tab = [6, 5, 2, 3, 4][tab];
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt(("legacy-build-body", tab))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(width);
            ui.push_id(("legacy-build-panel", tab), |ui| match tab {
                0 => general::show(ui, p, tool, world, false),
                1 => {
                    object::show_flags(ui, p, tool, world);
                    widgets::divider(ui, p);
                    object::show_transform(ui, p, tool, world, s, mods);
                    widgets::divider(ui, p);
                    object::show_shape(ui, p, tool, world, env);
                }
                2 => features::show(ui, p, tool, world, s, env),
                3 => texture::show(ui, p, tool, world, s, env),
                4 => contents::show(ui, p, tool, world),
                _ => unreachable!(),
            });
            if !tool.status.is_empty() {
                common::small(ui, p, &tool.status);
            }
        });
}
