//! The Aurora style's tool windows: Camera, Move, Create and Land, each
//! with its panel (tools.rs) in a window of its own (build/tool_windows.rs
//! has the rules). The window of the tool in use says so in its title; a
//! press in a window makes its tool the one in use.

use super::{Env, INSPECTOR_ID, common, compact};
use crate::build::{BuildSettings, BuildTool, Mods, Tool};
use crate::theme::Palette;
use crate::ui::widgets::{self, Floater};
use crate::world::World;
use egui::Vec2;

pub(super) fn floater_id(which: Tool) -> &'static str {
    match which {
        Tool::Focus => "build_tool_camera",
        Tool::Grab => "build_tool_move",
        Tool::Create => "build_tool_create",
        Tool::Land => "build_tool_land",
        Tool::Edit => INSPECTOR_ID,
    }
}

/// The size a tool window first opens at: its panel without a scroll bar
/// (the Create window's height follows its content).
fn default_size(which: Tool) -> Vec2 {
    match which {
        Tool::Focus | Tool::Grab => Vec2::new(280.0, 190.0),
        Tool::Create => Vec2::new(300.0, 440.0),
        _ => Vec2::new(330.0, 560.0),
    }
}

/// Whether a press started in window `id` this frame, on top of what is
/// under the pointer there.
pub(super) fn pressed_in(ctx: &egui::Context, id: &str) -> bool {
    let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None }) else {
        return false;
    };
    ctx.layer_id_at(pos).is_some_and(|layer| layer.id == egui::Id::unique(id))
}

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
) {
    let screen = ctx.content_rect();
    // first opened beside the main window, one below the other
    let beside = ctx
        .memory(|m| m.area_rect(egui::Id::unique(INSPECTOR_ID)))
        .map_or(screen.left() + 370.0, |rect| rect.right() + 8.0);
    for which in [Tool::Focus, Tool::Grab, Tool::Create, Tool::Land] {
        if !tool.tool_window_open(which) {
            continue;
        }
        let id = floater_id(which);
        let (_, title) = compact::window_heading(which);
        let slot = match which {
            Tool::Focus => 0.0,
            Tool::Grab => 1.0,
            Tool::Create => 2.0,
            _ => 3.0,
        };
        let mut floater = Floater::new(id, title, egui::pos2(beside, 40.0 + 36.0 * slot), default_size(which)).silent();
        floater.min_size = Vec2::new(240.0, 140.0);
        // The Create window as tall as what it shows (human request,
        // 2026-11-24): no scroll bar, its height following its shapes and
        // its options, shown or hidden, and only its width resized.
        let fits = which == Tool::Create;
        floater = if fits { floater.width_only() } else { floater.steady_size() };
        if tool.tool == which {
            floater = floater.title_state(p.violet, aurora_i18n::tr!("build-tool-in-use"));
        }
        let mut open = true;
        floater.show(ctx, p, &mut open, |ui| {
            common::typography(ui);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            if fits {
                compact::tool_panel(ui, p, tool, world, s, mods, env, which);
                return;
            }
            // laid out at the width it keeps once the scroll bar shows: a
            // panel that grows taller as it narrows would make it come and
            // go (#1092)
            let width = widgets::scrolled_width(ui).max(1.0);
            egui::ScrollArea::vertical()
                .id_salt(("build_tool_window", id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    compact::tool_panel(ui, p, tool, world, s, mods, env, which);
                });
        });
        if !open {
            tool.close_tool_window(which, world, s);
        } else if pressed_in(ctx, id) {
            tool.activate(which);
        }
    }
}
