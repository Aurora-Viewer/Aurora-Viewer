//! A visible destination for Tab and Shift+Tab, including painter-drawn
//! controls. The Legacy Viewer's LLButton::draw and LLLineEditor::draw
//! (indra/llui/llbutton.cpp, lllineeditor.cpp) outline the focused control.
//! Aurora shows this extra outline after keyboard navigation, until the
//! next pointer press; ordinary mouse focus and text carets keep their look.
//!
//! Observe input only. egui and the widgets still decide what Tab does,
//! which control takes the focus, and whether a text editor inserts a tab.

use std::collections::HashSet;

use egui::{Context, Event, Key, Painter, Stroke, StrokeKind, TouchPhase, Ui, ViewportId};

use crate::theme::Palette;

/// Every interface context, including login and native secondary windows,
/// receives this plugin through the theme. Reloading the skin updates the
/// colors without resetting how the user is navigating.
pub fn install(ctx: &Context, palette: &Palette) {
    ctx.add_plugin(KeyboardFocus {
        palette: *palette,
        keyboard: HashSet::new(),
    });
    if let Some(plugin) = ctx.plugin_opt::<KeyboardFocus>() {
        plugin.lock().palette = *palette;
    }
}

struct KeyboardFocus {
    palette: Palette,
    keyboard: HashSet<ViewportId>,
}

impl egui::Plugin for KeyboardFocus {
    fn debug_name(&self) -> &'static str {
        "keyboard_focus"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        let viewport = ctx.viewport_id();
        let navigated = ctx.input(|input| {
            let mut navigated = false;
            for event in &input.events {
                match event {
                    Event::Key {
                        key: Key::Tab,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.alt && !modifiers.ctrl && !modifiers.command => {
                        self.keyboard.insert(viewport);
                        navigated = true;
                    }
                    Event::PointerButton { pressed: true, .. }
                    | Event::Touch {
                        phase: TouchPhase::Start, ..
                    } => {
                        self.keyboard.remove(&viewport);
                    }
                    _ => {}
                }
            }
            navigated
        });
        if navigated {
            // egui applies Shift+Tab's destination at the next pass so that
            // the previous widget receives gained_focus then.
            ctx.request_repaint();
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        if !self.keyboard.contains(&ctx.viewport_id()) || !ctx.input(|input| input.focused) {
            return;
        }
        let Some(id) = ctx.memory(|memory| memory.focused()) else {
            return;
        };
        // Current pass only: read_response also finds last pass's widget,
        // which could belong to a window just closed. Neither inspecting
        // the widget nor drawing its outline may change keyboard focus.
        let Some(widget) = ctx.viewport(|viewport| viewport.this_pass.widgets.get(id).copied()) else {
            return;
        };
        if !widget.enabled
            || !widget.visible
            || !widget.interact_rect.is_positive()
            || !ctx.memory(|memory| memory.allows_interaction(widget.layer_id))
        {
            return;
        }
        // Keep both the original layer (windows/popups still cover it)
        // and its clip (a scrolled-out control cannot draw over its pane).
        // Two theme colors keep the mark distinct on an accent or image as
        // well as on a normal field, including the high-contrast palette.
        let painter = Painter::new(ctx.clone(), widget.layer_id, widget.interact_rect);
        let radius = ui.visuals().widgets.inactive.corner_radius;
        painter.rect_stroke(widget.rect, radius, Stroke::new(4.0, self.palette.bar), StrokeKind::Inside);
        painter.rect_stroke(widget.rect, radius, Stroke::new(2.0, self.palette.ink), StrokeKind::Inside);
    }
}

#[cfg(test)]
mod tests;
