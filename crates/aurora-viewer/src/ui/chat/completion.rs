//! Inline gesture completion from LLFloaterIMNearbyChat::onChatBoxKeystroke
//! (Firestorm indra/newview/llfloaterimnearbychat.cpp, originally LGPL 2.1).

use crate::gestures::Manager;
use egui::text::{CCursor, CCursorRange};

pub(super) fn complete(ui: &egui::Ui, output: &mut egui::text_edit::TextEditOutput, text: &mut String, gestures: &Manager) {
    let entered_text = ui.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::Text(s) | egui::Event::Paste(s) if !s.is_empty()))
    });
    let submitting = output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if !output.response.changed() || (!output.response.has_focus() && !submitting) || !entered_text {
        return;
    }
    let typed = text.chars().count();
    if !output.cursor_range.is_some_and(|r| r.is_empty() && r.primary.index.0 == typed) {
        return;
    }
    let prefix = text.trim_start();
    if !prefix.starts_with('/') || prefix.chars().count() < 2 {
        return;
    }
    let Some(completed) = gestures.complete(prefix) else {
        return;
    };
    let suffix = &completed[prefix.len()..];
    if suffix.is_empty() {
        return;
    }
    text.push_str(suffix);
    let range = CCursorRange::two(CCursor::new(typed), CCursor::new(text.chars().count()));
    output.cursor_range = Some(range);
    output.state.cursor.set_char_range(Some(range));
    output.state.clone().store(ui.ctx(), output.response.id);
    ui.ctx().request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chat::{ChatUi, OutgoingChat, bar};
    use aurora_assets::gesture::Gesture;
    use uuid::Uuid;

    struct Fixture {
        ctx: egui::Context,
        chat: ChatUi,
        gestures: Manager,
    }
    impl Fixture {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            ctx.memory_mut(|m| m.request_focus(egui::Id::unique("nearby_chat_input")));
            let id = Uuid::from_u128(1);
            let mut gestures = Manager::default();
            gestures.entries.insert(
                id,
                crate::gestures::Entry {
                    asset: Uuid::nil(),
                    definition: Gesture {
                        trigger: "/eject".into(),
                        ..Default::default()
                    },
                },
            );
            gestures.active.insert(id);
            let mut fixture = Self {
                ctx,
                chat: ChatUi::default(),
                gestures,
            };
            for _ in 0..3 {
                fixture.frame(Vec::new());
            }
            fixture
        }
        fn frame(&mut self, mut events: Vec<egui::Event>) -> Option<OutgoingChat> {
            let mut outgoing = None;
            let modifiers = events
                .iter()
                .find_map(|e| match e {
                    egui::Event::Key { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.horizontal(|ui| {
                        outgoing = bar(ui, &crate::theme::Theme::default().palette(), &mut self.chat, &self.gestures, 400.0);
                    });
                },
            );
            output.textures_delta.clear();
            outgoing
        }
        fn text(&mut self, text: &str) {
            self.frame(vec![egui::Event::Text(text.into())]);
        }
        fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> Option<OutgoingChat> {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }])
        }
        fn selection(&self) -> std::ops::Range<usize> {
            let range = egui::text_edit::TextEditState::load(&self.ctx, egui::Id::unique("nearby_chat_input"))
                .and_then(|s| s.cursor.char_range())
                .expect("chat cursor")
                .as_sorted_char_range();
            range.start.0..range.end.0
        }
    }

    #[test]
    fn typing_selects_the_suffix_and_enter_sends_the_completed_trigger() {
        for modifiers in [egui::Modifiers::NONE, egui::Modifiers::CTRL, egui::Modifiers::SHIFT] {
            let mut f = Fixture::new();
            f.text("/ej");
            assert_eq!(f.chat.input, "/eject");
            assert_eq!(f.selection(), 3..6);
            let sent = f.key(egui::Key::Enter, modifiers).expect("completed gesture sent");
            assert_eq!(sent.message, "/eject");
            assert_eq!(sent.channel, 0);
            assert_eq!(
                sent.chat_type,
                if modifiers.ctrl {
                    aurora_net::ChatType::Shout
                } else if modifiers.shift {
                    aurora_net::ChatType::Whisper
                } else {
                    aurora_net::ChatType::Normal
                }
            );
            assert!(f.chat.input.is_empty());
            let (_, id) = f.gestures.revise(&sent.message);
            assert_eq!(id, Some(Uuid::from_u128(1)));
        }
    }

    #[test]
    fn continued_typing_replaces_the_suggestion_and_backspace_dismisses_it() {
        let mut f = Fixture::new();
        f.text("/e");
        assert_eq!(f.selection(), 2..6);
        f.text("j");
        assert_eq!(f.chat.input, "/eject");
        assert_eq!(f.selection(), 3..6);
        f.key(egui::Key::Backspace, egui::Modifiers::NONE);
        assert_eq!(f.chat.input, "/ej");
        f.frame(Vec::new());
        assert_eq!(f.chat.input, "/ej");
        f.text("x");
        assert_eq!(f.chat.input, "/ejx");
    }

    #[test]
    fn channel_prefix_and_edits_in_the_middle_are_not_completed() {
        let mut f = Fixture::new();
        f.text("/28574 message");
        let sent = f.key(egui::Key::Enter, egui::Modifiers::NONE).expect("channel chat sent");
        assert_eq!((sent.channel, sent.message.as_str()), (28574, "message"));
        let mut f = Fixture::new();
        f.text("/ej");
        f.key(egui::Key::ArrowLeft, egui::Modifiers::NONE);
        f.text("e");
        assert_eq!(f.chat.input, "/ejeect");
    }

    #[test]
    fn completion_handles_unicode_and_is_disabled_with_gestures() {
        let mut f = Fixture::new();
        f.gestures.entries.get_mut(&Uuid::from_u128(1)).expect("gesture").definition.trigger = "/éject".into();
        f.text("/Éj");
        assert_eq!(f.chat.input, "/Éject");
        assert_eq!(f.selection(), 3..6);
        let mut f = Fixture::new();
        f.gestures.enabled = false;
        f.text("/ej");
        assert_eq!(f.chat.input, "/ej");
    }

    #[test]
    fn completion_also_runs_when_text_and_enter_arrive_in_one_frame() {
        let mut f = Fixture::new();
        let sent = f
            .frame(vec![
                egui::Event::Text("/ej".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ])
            .expect("completed gesture sent in one frame");
        assert_eq!(sent.message, "/eject");
    }

    #[test]
    fn color_emoji_input_keeps_inline_completion_and_selection() {
        let mut f = Fixture::new();
        f.gestures.entries.get_mut(&Uuid::from_u128(1)).expect("gesture").definition.trigger = "/😀 wave".into();
        let id = egui::Id::unique("color_gesture_input");
        let mut emoji = crate::ui::emoji::Emoji::load();
        let mut text = "/😀".to_owned();
        f.ctx.memory_mut(|m| m.request_focus(id));
        for frame in 0..4 {
            let events = if frame == 3 {
                vec![egui::Event::Text(" w".into())]
            } else {
                Vec::new()
            };
            let mut output = f.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut edit = emoji.text_field(ui, &mut text, |edit| edit.id(id).desired_width(400.0));
                    complete(ui, &mut edit, &mut text, &f.gestures);
                },
            );
            let painted_emoji = output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default()));
            output.textures_delta.clear();
            assert!(painted_emoji, "the input must still paint the color emoji image");
        }
        assert_eq!(text, "/😀 wave");
        let range = egui::text_edit::TextEditState::load(&f.ctx, id)
            .and_then(|s| s.cursor.char_range())
            .expect("completion selection")
            .as_sorted_char_range();
        assert_eq!(range.start.0..range.end.0, 4..7);
    }
}
