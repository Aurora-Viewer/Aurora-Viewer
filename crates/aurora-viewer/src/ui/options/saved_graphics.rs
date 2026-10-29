//! Named graphics controls shared with the FPS hover list.

use super::{OptionsResult, OptionsUi};
use crate::graphics_presets::saved::{Saved, valid_name};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::{RichText, Vec2};

#[derive(Default)]
pub(super) struct Editor {
    initialized: bool,
    name: String,
    pending: Option<Pending>,
}

enum Pending {
    Replace(String, Box<Saved>),
    Delete(String),
}

pub(super) fn controls(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut changed = false;
    let editor = &mut st.saved_graphics;
    let current = crate::graphics_presets::saved::choices(s);
    let selected = current.iter().find(|(_, same)| *same).map(|(name, _)| name.as_str());
    if !editor.initialized {
        editor.initialized = true;
        if editor.name.is_empty() {
            editor.name = selected.unwrap_or_default().to_owned();
        }
    }
    ui.vertical(|ui| {
        egui::ComboBox::from_id_salt("saved_graphics")
            .width(260.0)
            .truncate()
            .selected_text(selected.unwrap_or(tr!("ui-options-preset-custom")))
            .show_ui(ui, |ui| {
                for (name, active) in &current {
                    if ui.selectable_label(*active, name).clicked() {
                        if let Some(saved) = s.graphics_saved.get(name).cloned() {
                            saved.apply(s);
                            editor.name = name.clone();
                            changed = true;
                        }
                        ui.close();
                    }
                }
                if current.is_empty() {
                    ui.label(tr!("graphics-saved-empty"));
                }
            });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut editor.name)
                    .hint_text(tr!("graphics-saved-name"))
                    .char_limit(64)
                    .desired_width(180.0),
            );
            let name = editor.name.trim().to_owned();
            ui.add_enabled_ui(valid_name(&name), |ui| {
                if widgets::action_button(
                    ui,
                    p,
                    "device-floppy-filled",
                    tr!("common-save"),
                    widgets::ActionKind::Flat,
                    Vec2::ZERO,
                )
                .clicked()
                {
                    let values = Saved::capture(s);
                    if s.graphics_saved.contains_key(&name) {
                        editor.pending = Some(Pending::Replace(name.clone(), Box::new(values)));
                    } else {
                        s.graphics_saved.insert(name.clone(), values);
                        changed = true;
                    }
                }
            });
            ui.add_enabled_ui(s.graphics_saved.contains_key(&name), |ui| {
                if widgets::action_button(ui, p, "trash-filled", tr!("common-delete"), widgets::ActionKind::Flat, Vec2::ZERO).clicked() {
                    editor.pending = Some(Pending::Delete(name));
                }
            });
        });
    });
    changed
}

pub(super) fn confirm(ctx: &egui::Context, p: &Palette, s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult, open: bool) {
    let editor = &mut st.saved_graphics;
    if !open {
        editor.pending = None;
    }
    let Some(pending) = &editor.pending else {
        return;
    };
    let (name, title, action) = match pending {
        Pending::Replace(name, _) => (name, tr!("graphics-saved-replace"), tr!("common-save")),
        Pending::Delete(name) => (name, tr!("graphics-saved-delete"), tr!("common-delete")),
    };
    let deleting = matches!(pending, Pending::Delete(_));
    let mut confirmed = false;
    let mut cancelled = false;
    let modal = widgets::Modal::new(egui::Id::unique("graphics_saved_confirm")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new(title).strong());
        ui.label(name);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            confirmed = if deleting {
                widgets::action_button(ui, p, "trash-filled", action, widgets::ActionKind::Danger, Vec2::new(100.0, 26.0)).clicked()
            } else {
                widgets::primary_button_sized(ui, p, action, Vec2::new(100.0, 26.0)).clicked()
            };
            cancelled = ui.button(tr!("common-cancel")).clicked();
        });
    });
    if confirmed {
        match editor.pending.take() {
            Some(Pending::Replace(name, values)) => {
                s.graphics_saved.insert(name, *values);
            }
            Some(Pending::Delete(name)) => {
                s.graphics_saved.remove(&name);
                editor.name.clear();
            }
            None => return,
        }
        r.changed = true;
    } else if cancelled || modal.should_close() {
        editor.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::{Frames, Text};

    fn click(pos: egui::Pos2, down: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: down,
            modifiers: Default::default(),
        }
    }

    fn frame(frames: &mut Frames, s: &mut Settings, st: &mut OptionsUi, events: Vec<egui::Event>) -> Vec<Text> {
        let p = crate::theme::Theme::default().palette();
        frames.run(events, |ui| {
            controls(ui, &p, s, st);
            confirm(ui.ctx(), &p, s, st, &mut OptionsResult::default(), true);
        })
    }

    fn button(frames: &mut Frames, s: &mut Settings, st: &mut OptionsUi, label: &str) {
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frame(frames, s, st, vec![]);
        }
        // The confirmation comes after the editor and may repeat Save/Delete.
        let pos = texts.iter().rfind(|t| t.source == label).expect("button is painted").rect.center();
        frame(frames, s, st, vec![egui::Event::PointerMoved(pos)]);
        frame(frames, s, st, vec![click(pos, true)]);
        frame(frames, s, st, vec![click(pos, false)]);
    }

    #[test]
    fn graphics_saved_editor_starts_with_the_active_profile_without_refilling_a_cleared_draft() {
        let mut frames = Frames::new();
        let mut s = Settings::default();
        s.graphics_saved.insert("Night".into(), Saved::capture(&s));
        let mut st = OptionsUi::default();
        frame(&mut frames, &mut s, &mut st, vec![]);
        assert_eq!(st.saved_graphics.name, "Night");
        st.saved_graphics.name.clear();
        frame(&mut frames, &mut s, &mut st, vec![]);
        assert!(
            st.saved_graphics.name.is_empty(),
            "the user can clear the name to save another profile"
        );
    }

    #[test]
    fn graphics_saved_editor_confirms_replacement_and_deletion() {
        let mut frames = Frames::new();
        let mut s = Settings::default();
        let mut st = OptionsUi::default();
        st.saved_graphics.name = "Night".into();
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        assert_eq!(s.graphics_saved.len(), 1);
        let original = s.graphics_saved["Night"].clone();
        s.draw_distance = 224.0;
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        assert!(st.saved_graphics.pending.is_some());
        assert_eq!(s.graphics_saved["Night"], original);
        button(&mut frames, &mut s, &mut st, tr!("common-cancel"));
        assert_eq!(s.graphics_saved["Night"], original);
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        assert_eq!(s.graphics_saved["Night"].quality.draw_distance, 224.0);
        button(&mut frames, &mut s, &mut st, tr!("common-delete"));
        assert_eq!(s.graphics_saved.len(), 1);
        button(&mut frames, &mut s, &mut st, tr!("common-cancel"));
        assert_eq!(s.graphics_saved.len(), 1);
        button(&mut frames, &mut s, &mut st, tr!("common-delete"));
        button(&mut frames, &mut s, &mut st, tr!("common-delete"));
        assert!(s.graphics_saved.is_empty());
    }
}
