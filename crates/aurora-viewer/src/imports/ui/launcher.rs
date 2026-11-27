//! Compact import entry point. Typed pickers follow llviewermenufile.cpp;
//! the movable icon strip and shared upload queue are Aurora additions.

use super::*;

pub(crate) const WINDOW: &str = crate::imports::LAUNCHER_WINDOW;

/// The quick import bar is a floating block of the toolbars
/// (`ui::toolbar::floating`, without the grip it had: its glass moves it);
/// Esc still hides it while no import window is open.
pub(super) fn show(ctx: &egui::Context, imports: &mut Imports) {
    if imports.launcher_open
        && !widgets::interface_hidden(ctx)
        && !imports.any_window_open()
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        imports.launcher_open = false;
    }
}

pub(super) fn keep_accessible(ctx: &egui::Context, imports: &Imports) {
    if imports.launcher_open
        && ctx.memory(|memory| {
            memory.area_rect(egui::Id::unique(WINDOW)).is_some_and(|bar| {
                std::iter::once(Choice::Bulk).chain(windows::TYPES).any(|choice| {
                    imports.window_open(choice)
                        && memory
                            .area_rect(egui::Id::unique(choice.window_id()))
                            .is_some_and(|window| bar.intersects(window))
                })
            })
        })
    {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(WINDOW)));
    }
}
