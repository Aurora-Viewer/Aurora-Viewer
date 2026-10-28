//! LLFloaterScriptDebug (Firestorm, LGPL 2.1): combined output and per-object
//! filtering. The world bounds messages, the UI opens when new output arrives
//! (not for a script error shown in the nearby chat, #1263).

use crate::{
    theme::Palette,
    ui::widgets::{Floater, flat_button},
    world::{ChatKind, script_debug::Messages},
};
use egui::{RichText, Vec2};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Debug {
    pub open: bool,
    opens: u64,
    selected: Option<Uuid>,
}
impl Debug {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, messages: &mut Messages) {
        if self.opens != messages.opens {
            self.open = true;
            self.opens = messages.opens;
        }
        if !self.open {
            return;
        }
        if self.selected.is_some_and(|id| !messages.lines.iter().any(|line| line.source == id)) {
            self.selected = None;
        }
        Floater::new(
            "script_debug",
            aurora_i18n::tr!("script-debug-title"),
            ctx.content_rect().center() - Vec2::new(310.0, 210.0),
            Vec2::new(620.0, 420.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            ui.horizontal(|ui| {
                let name = self
                    .selected
                    .and_then(|id| messages.lines.iter().find(|line| line.source == id))
                    .map(|line| line.from.as_str());
                egui::ComboBox::from_id_salt("script_debug_object")
                    .selected_text(name.unwrap_or(aurora_i18n::tr!("script-debug-all")))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.selected, None, aurora_i18n::tr!("script-debug-all"));
                        let mut seen = std::collections::HashSet::new();
                        for line in &messages.lines {
                            if seen.insert(line.source) {
                                ui.selectable_value(&mut self.selected, Some(line.source), &line.from)
                                    .on_hover_text(line.source.to_string());
                            }
                        }
                    });
                if flat_button(ui, p, aurora_i18n::tr!("script-debug-clear")).clicked() {
                    messages.lines.retain(|line| self.selected.is_some_and(|id| id != line.source));
                }
            });
            egui::ScrollArea::both().stick_to_bottom(true).show(ui, |ui| {
                for line in messages
                    .lines
                    .iter()
                    .filter(|line| self.selected.is_none_or(|id| id == line.source))
                {
                    let text = format!("{}: {}", line.from, line.text);
                    ui.add(
                        egui::Label::new(RichText::new(text).monospace().color(
                            if matches!(line.kind, ChatKind::Object(aurora_net::ChatType::Debug)) {
                                p.danger
                            } else {
                                p.ink
                            },
                        ))
                        .selectable(true),
                    );
                }
            });
        });
    }
}
