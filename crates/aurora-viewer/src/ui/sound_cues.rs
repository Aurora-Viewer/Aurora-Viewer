//! Interface sounds caused by the widgets themselves, found from egui's
//! interaction state instead of at every call site, after Firestorm's llui:
//!
//! * LLButton::handleMouseUp: UISndClickRelease when a button released on
//!   itself (sound_flags defaults to MOUSE_UP, llview.cpp), which covers
//!   checkboxes, tabs, combo boxes and menu items (LLMenuItemGL), and when a
//!   button is committed from the keyboard (LLButton::onCommit).
//!   egui: a click-only widget (button, checkbox, selectable, combo...) got
//!   clicked. Text fields, sliders, maps and labels with selectable text
//!   also sense drags and stay silent, like LLLineEditor / LLTextBase.
//! * LLSlider::handleMouseDown / Up: UISndClick / UISndClickRelease around
//!   a slider drag.
//! * LLLineEditor / LLTextEditor: LLUI::reportBadKeystroke when a key does
//!   nothing in a text field (erase at the start, arrow past an end, typing
//!   past the length limit or in a read-only field).
//! * LLFloater::openFloater / closeFloater: UISndWindowOpen / Close when a
//!   floater appears / disappears (floaters register each frame they are
//!   shown with [`floater_shown`]); un-minimizing plays UISndWindowClose
//!   (LLFloater::setMinimized).
//!
//! Other UI code asks for a sound with [`request`].

use crate::ui_sound::UiSound;
use egui::output::OutputEvent;
use egui::text_selection::CCursorRange;
use egui::{Context, Event, Id, Key, WidgetType};
use std::collections::HashSet;

fn requests_id() -> Id {
    Id::new("aurora_ui_sound_requests")
}

fn floaters_id() -> Id {
    Id::new("aurora_ui_sound_floaters")
}

/// Plays `s` after this frame (gated by the sound settings).
pub fn request(ctx: &Context, s: UiSound) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<UiSound>>(requests_id()).push(s));
}

fn previews_id() -> Id {
    Id::new("aurora_ui_sound_previews")
}

/// Plays the asset `id` whatever the settings (preview button, force_sound).
pub fn preview(ctx: &Context, id: uuid::Uuid) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<uuid::Uuid>>(previews_id()).push(id));
}

pub fn take_previews(ctx: &Context) -> Vec<uuid::Uuid> {
    ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<Vec<uuid::Uuid>>(previews_id())))
}

/// A floater is shown this frame (its open / close sounds).
pub fn floater_shown(ctx: &Context, id: Id) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<HashSet<Id>>(floaters_id()).insert(id);
    });
}

/// Widgets that play the click sound when clicked from the keyboard.
fn keyboard_clickable(t: WidgetType) -> bool {
    matches!(
        t,
        WidgetType::Button
            | WidgetType::Checkbox
            | WidgetType::RadioButton
            | WidgetType::SelectableLabel
            | WidgetType::ComboBox
            | WidgetType::CollapsingHeader
            | WidgetType::ColorButton
    )
}

/// Keys that do something in a text field (others, like Enter or Tab,
/// belong to the window).
fn edit_key(k: Key) -> bool {
    matches!(k, Key::Backspace | Key::Delete | Key::ArrowLeft | Key::ArrowRight)
}

#[derive(Default)]
pub struct SoundCues {
    /// Widgets clicked by the pointer this frame (all passes).
    clicked: Vec<Id>,
    /// Widget dragged at the last pass.
    dragged: Option<Id>,
    /// Slider being dragged (its press sound played).
    slider: Option<Id>,
    /// Text field focused before the frame, its cursor, and whether a key
    /// was pressed in it.
    text: Option<(Id, Option<CCursorRange>)>,
    text_keys: bool,
    floaters: Option<HashSet<Id>>,
}

impl SoundCues {
    /// Before running egui on `input`.
    pub fn before(&mut self, ctx: &Context, input: &egui::RawInput) {
        self.clicked.clear();
        self.text = ctx.memory(|m| m.focused()).and_then(|id| {
            let state = egui::text_edit::TextEditState::load(ctx, id)?;
            Some((id, state.cursor.char_range()))
        });
        self.text_keys = self.text.is_some()
            && input.events.iter().any(|e| match e {
                Event::Key { key, pressed: true, .. } => edit_key(*key),
                Event::Text(t) => !t.is_empty(),
                _ => false,
            });
    }

    /// At the start of each egui pass (the interaction of this frame; a
    /// discarded first pass keeps its click).
    pub fn pass_start(&mut self, ctx: &Context) {
        let (clicked, dragged) = ctx.interaction_snapshot(|s| (s.clicked, s.dragged));
        self.dragged = dragged;
        let Some(id) = clicked else {
            return;
        };
        let click_only = ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .get(id)
                .is_some_and(|w| w.enabled && w.sense.senses_click() && !w.sense.senses_drag())
        });
        if click_only && !self.clicked.contains(&id) {
            self.clicked.push(id);
        }
    }

    /// After the frame: the sounds it caused. `in_world`: the world is shown,
    /// so floaters count (not at login, nor when they come back at arrival).
    pub fn after(&mut self, ctx: &Context, events: &[OutputEvent], in_world: bool) -> Vec<UiSound> {
        let mut out: Vec<UiSound> = ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<Vec<UiSound>>(requests_id())));

        let keyboard_click = self.clicked.is_empty()
            && events
                .iter()
                .any(|e| matches!(e, OutputEvent::Clicked(i) if keyboard_clickable(i.typ)));
        if !self.clicked.is_empty() || keyboard_click {
            out.push(UiSound::ClickRelease);
        }

        // sliders: press when a drag starts changing one, release at the end
        let slider_moved = events
            .iter()
            .any(|e| matches!(e, OutputEvent::ValueChanged(i) if i.typ == WidgetType::Slider));
        if let Some(s) = self.slider
            && self.dragged != Some(s)
        {
            self.slider = None;
            out.push(UiSound::ClickRelease);
        }
        if slider_moved && self.slider.is_none() && self.dragged.is_some() {
            self.slider = self.dragged;
            out.push(UiSound::Click);
        }

        // a key that did nothing in the focused text field
        if self.text_keys
            && let Some((id, before)) = self.text
            && ctx.memory(|m| m.focused()) == Some(id)
        {
            // egui reports a change for any editing key, even Backspace in
            // an empty field; prev_text_value is only set when the text changed
            let edited = events.iter().any(|e| {
                matches!(e, OutputEvent::ValueChanged(i)
                    if i.typ == WidgetType::TextEdit && i.prev_text_value.is_some())
            });
            let after = egui::text_edit::TextEditState::load(ctx, id).and_then(|s| s.cursor.char_range());
            if !edited && after == before {
                out.push(UiSound::BadKeystroke);
            }
        }

        // floaters that appeared / disappeared
        let shown = ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<HashSet<Id>>(floaters_id())));
        if in_world {
            if let Some(prev) = &self.floaters {
                if shown.difference(prev).next().is_some() {
                    out.push(UiSound::WindowOpen);
                }
                if prev.difference(&shown).next().is_some() {
                    out.push(UiSound::WindowClose);
                }
            }
            self.floaters = Some(shown);
        } else {
            self.floaters = None;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a window with a focused single-line field holding
    /// `text`, receiving `events`; returns the sounds.
    fn frame(ctx: &Context, cues: &mut SoundCues, text: &mut String, events: Vec<Event>) -> Vec<UiSound> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
            events,
            ..Default::default()
        };
        cues.before(ctx, &raw);
        let mut out = ctx.run_ui(raw, |ui| {
            cues.pass_start(ui.ctx());
            let r = ui.add(egui::TextEdit::singleline(text).id(Id::new("field")));
            r.request_focus();
        });
        // no renderer here (the font atlas would trip a debug assertion)
        out.textures_delta.clear();
        cues.after(ctx, &out.platform_output.events, true)
    }

    fn key(k: Key) -> Event {
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn bad_keystroke_only_when_nothing_changes() {
        let ctx = Context::default();
        let mut cues = SoundCues::default();
        let mut text = String::new();
        // focus the field first
        for _ in 0..3 {
            frame(&ctx, &mut cues, &mut text, vec![]);
        }
        assert!(frame(&ctx, &mut cues, &mut text, vec![key(Key::Backspace)]).contains(&UiSound::BadKeystroke));
        assert!(!frame(&ctx, &mut cues, &mut text, vec![Event::Text("ab".into())]).contains(&UiSound::BadKeystroke));
        assert_eq!(text, "ab");
        assert!(!frame(&ctx, &mut cues, &mut text, vec![key(Key::Backspace)]).contains(&UiSound::BadKeystroke));
        assert_eq!(text, "a");
        // the cursor is at the end: Right does nothing, Left moves
        assert!(frame(&ctx, &mut cues, &mut text, vec![key(Key::ArrowRight)]).contains(&UiSound::BadKeystroke));
        assert!(!frame(&ctx, &mut cues, &mut text, vec![key(Key::ArrowLeft)]).contains(&UiSound::BadKeystroke));
        assert!(frame(&ctx, &mut cues, &mut text, vec![key(Key::Backspace)]).contains(&UiSound::BadKeystroke));
    }
}
