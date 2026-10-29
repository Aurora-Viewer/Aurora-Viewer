//! Named graphics controls shared with the FPS hover list.

use super::{OptionsResult, OptionsUi};
use crate::graphics_presets::saved::{self, Saved, valid_name};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::{RichText, Vec2};

#[derive(Default)]
pub(super) struct Editor {
    name: String,
    focus_name: bool,
    pending: Option<Pending>,
}

enum Pending {
    Save(Box<Saved>),
    Replace(String, Box<Saved>),
    Delete(String),
}

impl Editor {
    fn request_save_as(&mut self, s: &Settings) {
        self.name.clear();
        self.focus_name = true;
        self.pending = Some(Pending::Save(Box::new(Saved::capture(s))));
    }
}

/// Keep the bin on the same right edge for short, long and translated names.
/// Its hitbox is separate from the profile selection, so deletion never loads it.
fn profile_row(ui: &mut egui::Ui, p: &Palette, name: &str, active: bool) -> egui::InnerResponse<(bool, bool)> {
    ui.push_id(("graphics_profile", name), |ui| {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 20.0 - ui.spacing().item_spacing.x).max(1.0);
            let load = ui
                .add_sized(
                    Vec2::new(width, 20.0),
                    egui::Button::selectable(active, ()).left_text(name).truncate(),
                )
                .on_hover_text(name)
                .clicked();
            let delete = widgets::icon_button(ui, p, "trash-filled", tr!("common-delete"), true);
            (load, delete)
        })
    })
    .inner
}

pub(super) fn controls(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut changed = false;
    let editor = &mut st.saved_graphics;
    let current = saved::choices(s);
    let selected = saved::selected(s).map(str::to_owned);
    let label = match selected.as_deref() {
        Some(name) if s.graphics_saved[name] != Saved::capture(s) => tr!("graphics-saved-modified", name = name.to_owned()),
        Some(name) => name.to_owned(),
        None => tr!("ui-options-preset-custom").to_owned(),
    };
    ui.horizontal(|ui| {
        let actions_width: f32 = [tr!("common-save"), tr!("graphics-saved-save-as")]
            .into_iter()
            .map(|label| {
                ui.painter()
                    .layout_no_wrap(
                        label.to_owned(),
                        egui::TextStyle::Button.resolve(ui.style()),
                        ui.visuals().text_color(),
                    )
                    .size()
                    .x
                    + ui.spacing().button_padding.x * 2.0
                    + ui.spacing().item_spacing.x
            })
            .sum();
        let list_width = (ui.available_width() - actions_width).clamp(64.0, 260.0);
        let mut demo_list = false;
        // Capture the production popup and name dialog without native pointer
        // automation. These two states are confined to offline demo runs.
        if std::env::var_os("AURORA_DEMO").is_some() && ui.ctx().cumulative_frame_nr() >= 300 {
            let mode = std::env::var("AURORA_DEMO_OPTIONS").unwrap_or_default();
            let id = egui::Id::unique("graphics_profile_demo_opened");
            if mode == "graphics:profiles-list" {
                demo_list = true;
            } else if mode == "graphics:save-profile" && !ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
                editor.request_save_as(s);
                ui.ctx().data_mut(|d| d.insert_temp(id, true));
            }
        }
        let list = egui::ComboBox::from_id_salt("saved_graphics")
            .width(list_width)
            .truncate()
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (name, active) in &current {
                    let (load, delete) = profile_row(ui, p, name, *active).inner;
                    if load {
                        changed |= saved::select(s, name);
                        ui.close();
                    }
                    if delete {
                        editor.pending = Some(Pending::Delete(name.clone()));
                        ui.close();
                    }
                }
                if current.is_empty() {
                    ui.label(tr!("graphics-saved-empty"));
                }
            });
        if demo_list {
            // Use the response's real popup ID, including ComboBox's salt.
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&list.response));
        }
        if ui.add_enabled(selected.is_some(), egui::Button::new(tr!("common-save"))).clicked() {
            changed |= saved::save_selected(s);
        }
        if ui.button(tr!("graphics-saved-save-as")).clicked() {
            editor.request_save_as(s);
        }
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
        Pending::Save(_) => ("", tr!("graphics-saved-save"), tr!("common-save")),
        Pending::Replace(name, _) => (name.as_str(), tr!("graphics-saved-replace"), tr!("common-save")),
        Pending::Delete(name) => (name.as_str(), tr!("graphics-saved-delete"), tr!("common-delete")),
    };
    let saving = matches!(pending, Pending::Save(_));
    let deleting = matches!(pending, Pending::Delete(_));
    let mut confirmed = false;
    let mut cancelled = false;
    let modal = widgets::Modal::new(egui::Id::unique("graphics_saved_confirm")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new(title).strong());
        if saving {
            let input = ui.add(
                egui::TextEdit::singleline(&mut editor.name)
                    .id(egui::Id::unique("graphics_saved_name"))
                    .hint_text(tr!("graphics-saved-name"))
                    .char_limit(64)
                    .desired_width(ui.available_width()),
            );
            if editor.focus_name {
                input.request_focus();
                editor.focus_name = false;
            }
            if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                confirmed = valid_name(&editor.name);
                if !confirmed {
                    input.request_focus();
                }
            }
        } else {
            ui.label(name);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let clicked = if deleting {
                widgets::action_button(ui, p, "trash-filled", action, widgets::ActionKind::Danger, Vec2::new(100.0, 26.0)).clicked()
            } else {
                ui.add_enabled_ui(!saving || valid_name(&editor.name), |ui| {
                    widgets::primary_button_sized(ui, p, action, Vec2::new(100.0, 26.0)).clicked()
                })
                .inner
            };
            confirmed |= clicked;
            cancelled = ui.button(tr!("common-cancel")).clicked();
        });
    });
    if confirmed {
        match editor.pending.take() {
            Some(Pending::Save(values)) => {
                let name = editor.name.trim().to_owned();
                if s.graphics_saved.contains_key(&name) {
                    editor.pending = Some(Pending::Replace(name, values));
                    return;
                }
                s.graphics_saved.insert(name.clone(), *values);
                s.graphics_saved_active = Some(name);
            }
            Some(Pending::Replace(name, values)) => {
                s.graphics_saved.insert(name.clone(), *values);
                s.graphics_saved_active = Some(name);
            }
            Some(Pending::Delete(name)) => {
                s.graphics_saved.remove(&name);
                if s.graphics_saved_active.as_ref() == Some(&name) {
                    s.graphics_saved_active = None;
                }
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
    fn graphics_saved_save_updates_the_selection_while_save_as_requests_a_name_and_confirms_duplicates() {
        let mut frames = Frames::new();
        let mut s = Settings::default();
        let mut st = OptionsUi::default();
        frame(&mut frames, &mut s, &mut st, vec![]);
        let input = egui::Id::unique("graphics_saved_name");
        assert!(frames.ctx.read_response(input).is_none(), "no permanent name field");
        button(&mut frames, &mut s, &mut st, tr!("graphics-saved-save-as"));
        assert!(matches!(st.saved_graphics.pending, Some(Pending::Save(_))));
        assert!(frames.ctx.memory(|m| m.has_focus(input)), "the dialog is ready for typing");
        assert!(s.graphics_saved.is_empty());
        button(&mut frames, &mut s, &mut st, tr!("common-cancel"));
        assert!(s.graphics_saved.is_empty(), "cancelling the name prompt saves nothing");
        button(&mut frames, &mut s, &mut st, tr!("graphics-saved-save-as"));
        frame(&mut frames, &mut s, &mut st, vec![egui::Event::Text("Night".into())]);
        frame(
            &mut frames,
            &mut s,
            &mut st,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        frame(
            &mut frames,
            &mut s,
            &mut st,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(s.graphics_saved.len(), 1);
        assert_eq!(saved::selected(&s), Some("Night"));
        s.draw_distance = 192.0;
        let texts = frame(&mut frames, &mut s, &mut st, vec![]);
        assert!(
            texts.iter().any(|t| t.source == tr!("graphics-saved-modified", name = "Night")),
            "the selected name is marked modified"
        );
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        assert!(
            st.saved_graphics.pending.is_none(),
            "Save updates the selected profile without a name prompt"
        );
        assert_eq!(s.graphics_saved["Night"].quality.draw_distance, 192.0);
        let original = s.graphics_saved["Night"].clone();
        s.draw_distance = 224.0;
        button(&mut frames, &mut s, &mut st, tr!("graphics-saved-save-as"));
        assert!(st.saved_graphics.name.is_empty(), "Save as asks for a new name");
        frame(&mut frames, &mut s, &mut st, vec![egui::Event::Text("Night".into())]);
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        assert!(matches!(st.saved_graphics.pending, Some(Pending::Replace(_, _))));
        assert_eq!(s.graphics_saved["Night"], original);
        button(&mut frames, &mut s, &mut st, tr!("common-cancel"));
        assert_eq!(s.graphics_saved["Night"], original);
        button(&mut frames, &mut s, &mut st, tr!("graphics-saved-save-as"));
        frame(&mut frames, &mut s, &mut st, vec![egui::Event::Text("Night".into())]);
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        button(&mut frames, &mut s, &mut st, tr!("common-save"));
        assert_eq!(s.graphics_saved["Night"].quality.draw_distance, 224.0);
    }

    #[test]
    fn graphics_saved_bins_share_the_right_edge_and_do_not_select_their_rows() {
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let names = ["Day".to_owned(), "W".repeat(64)];
        let mut rects = [egui::Rect::NOTHING; 2];
        let mut actions = [(false, false); 2];
        let mut body = |ui: &mut egui::Ui| {
            ui.set_width(320.0);
            for (i, name) in names.iter().enumerate() {
                let row = profile_row(ui, &p, name, false);
                rects[i] = row.response.rect;
                actions[i] = row.inner;
            }
        };
        frames.run(vec![], &mut body);
        frames.run(vec![], &mut body);
        assert_eq!(rects[0].right(), rects[1].right(), "long names cannot push the bin out");
        let pos = egui::pos2(rects[1].right() - 10.0, rects[1].center().y);
        let mut body = |ui: &mut egui::Ui| {
            ui.set_width(320.0);
            for (i, name) in names.iter().enumerate() {
                actions[i] = profile_row(ui, &p, name, false).inner;
            }
        };
        frames.run(vec![egui::Event::PointerMoved(pos)], &mut body);
        frames.run(vec![click(pos, true)], &mut body);
        frames.run(vec![click(pos, false)], &mut body);
        assert_eq!(actions, [(false, false), (false, true)], "the bin click never selects a profile");
    }

    fn bin(frames: &mut Frames, s: &mut Settings, st: &mut OptionsUi, name: &str) {
        let texts = frame(frames, s, st, vec![]);
        let popup = frames
            .ctx
            .memory(|m| {
                m.areas()
                    .visible_layer_ids()
                    .into_iter()
                    .find_map(|layer| (layer.order == egui::Order::Foreground).then(|| m.area_rect(layer.id)).flatten())
            })
            .expect("open profile list");
        let y = texts
            .iter()
            .find(|t| t.source == name && popup.contains(t.rect.center()))
            .expect("profile in the dropdown, not its selected label")
            .rect
            .center()
            .y;
        let pos = egui::pos2(popup.right() - 14.0, y);
        frame(frames, s, st, vec![egui::Event::PointerMoved(pos)]);
        frame(frames, s, st, vec![click(pos, true)]);
        frame(frames, s, st, vec![click(pos, false)]);
    }

    #[test]
    fn graphics_saved_dropdown_loads_rows_and_confirms_the_clicked_profiles_deletion() {
        let mut frames = Frames::new();
        let mut s = Settings::default();
        let day = Saved::capture(&s);
        let mut night = day.clone();
        night.quality.draw_distance = 224.0;
        s.graphics_saved.insert("Day".into(), day);
        s.graphics_saved.insert("Night".into(), night.clone());
        let mut st = OptionsUi::default();
        button(&mut frames, &mut s, &mut st, tr!("ui-options-preset-custom"));
        button(&mut frames, &mut s, &mut st, "Night");
        assert_eq!(Saved::capture(&s), night, "selecting a row still loads it");
        button(&mut frames, &mut s, &mut st, "Night");
        bin(&mut frames, &mut s, &mut st, "Day");
        assert!(matches!(&st.saved_graphics.pending, Some(Pending::Delete(name)) if name == "Day"));
        assert_eq!(Saved::capture(&s), night, "deleting another row never loads it");
        assert_eq!(s.graphics_saved.len(), 2);
        button(&mut frames, &mut s, &mut st, tr!("common-cancel"));
        assert_eq!(s.graphics_saved.len(), 2);
        button(&mut frames, &mut s, &mut st, "Night");
        bin(&mut frames, &mut s, &mut st, "Day");
        button(&mut frames, &mut s, &mut st, tr!("common-delete"));
        assert!(!s.graphics_saved.contains_key("Day"));
        assert!(s.graphics_saved.contains_key("Night"));
        assert_eq!(Saved::capture(&s), night);
        assert_eq!(saved::selected(&s), Some("Night"), "deleting another profile keeps the selection");
        button(&mut frames, &mut s, &mut st, "Night");
        bin(&mut frames, &mut s, &mut st, "Night");
        button(&mut frames, &mut s, &mut st, tr!("common-delete"));
        assert!(saved::selected(&s).is_none(), "deleting the selected profile clears its identity");
    }
}
