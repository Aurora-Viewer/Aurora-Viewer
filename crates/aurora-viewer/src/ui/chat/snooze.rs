//! The duration asked before a group chat's snooze, when Preferences ›
//! Chat say so (FSEnablePerGroupSnoozeDuration): the Legacy Viewer's
//! SnoozeDuration notification (FSFloaterIM::confirmSnooze and
//! snoozeDurationCallback), a whole number proposed from the setting
//! (GroupSnoozeTime). In minutes rather than its seconds, like the setting
//! (#945). Instead of its SnoozeDurationInvalidInput alert after the fact,
//! OK waits for a valid number.

use super::ChatUi;
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::World;
use egui::RichText;
use uuid::Uuid;

/// The group chat to snooze and the duration being typed.
pub struct Prompt {
    group: Uuid,
    text: String,
    /// The field takes the keyboard, its text selected, once.
    focus: bool,
}

impl Prompt {
    pub fn new(group: Uuid, minutes: u32) -> Self {
        Self {
            group,
            text: minutes.to_string(),
            focus: true,
        }
    }
}

/// A duration as the Legacy Viewer reads it (a whole number, at least 0),
/// spaces around it allowed, in minutes up to the setting's longest.
fn minutes(text: &str) -> Option<u32> {
    text.trim().parse().ok().filter(|m| *m <= crate::settings::GROUP_SNOOZE_MAX_MINUTES)
}

/// The dialog while a snooze waits for its duration: the group chat and
/// the minutes once confirmed.
pub fn dialog(ctx: &egui::Context, p: &Palette, world: &World, st: &mut ChatUi) -> Option<(Uuid, u32)> {
    let prompt = st.snooze.as_mut()?;
    let mut done = None;
    let mut close = false;
    let title = aurora_i18n::tr!("ui-chat-snooze-title", group = world.session_title(&prompt.group));
    let response = widgets::Modal::new(egui::Id::unique("group_chat_snooze")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new(title).size(17.0).color(p.ink));
        ui.add_space(8.0);
        ui.label(aurora_i18n::tr!("ui-chat-snooze-prompt"));
        ui.add_space(4.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut prompt.text)
                .char_limit(10)
                .desired_width(f32::INFINITY),
        );
        if prompt.focus {
            r.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ctx, r.id) {
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(prompt.text.chars().count()),
                )));
                state.store(ctx, r.id);
            }
            prompt.focus = false;
        }
        let valid = minutes(&prompt.text);
        if valid.is_none() {
            ui.add_space(4.0);
            ui.label(RichText::new(aurora_i18n::tr!("ui-chat-snooze-invalid")).size(12.0).color(p.muted));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            use widgets::{ActionKind, action_button, action_button_if};
            let size = egui::Vec2::new(0.0, 26.0);
            if action_button_if(
                ui,
                p,
                "check",
                aurora_i18n::tr!("common-ok"),
                ActionKind::Primary,
                size,
                valid.is_some(),
            )
            .clicked()
                || (valid.is_some() && widgets::enter_pressed(ui))
            {
                done = valid.map(|s| (prompt.group, s));
                close = true;
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                close = true;
            }
        });
    });
    if close || response.should_close() {
        st.snooze = None;
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FSFloaterIM::snoozeDurationCallback: a whole number of at least 0,
    /// here of minutes.
    #[test]
    fn the_duration_is_a_whole_number_of_minutes() {
        assert_eq!(minutes("15"), Some(15));
        assert_eq!(minutes(" 0 "), Some(0));
        assert_eq!(minutes("1440"), Some(1440));
        for bad in ["", "-5", "1.5", "ten", "15m", "99999999"] {
            assert_eq!(minutes(bad), None, "{bad:?}");
        }
    }

    /// The dialog keeps its size, its buttons inside it.
    #[test]
    fn the_dialog_keeps_its_size() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            "group_chat_snooze",
            &[egui::vec2(360.0, 200.0)],
            aurora_i18n::tr!("common-cancel"),
            || {
                let mut st = ChatUi::default();
                st.ask_snooze(Uuid::from_u128(1), 15);
                (st, World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load())))
            },
            |(st, world), ctx, p| {
                dialog(ctx, p, world, st);
            },
        );
    }
}
