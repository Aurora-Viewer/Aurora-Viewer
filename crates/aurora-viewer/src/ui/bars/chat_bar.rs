//! The nearby-chat field is a native toolbar entry, including inside drawers.
//! This user-requested extension retains FSNearbyChat::showDefaultChatBar /
//! FSNearbyChatControl's input behaviour (indra/newview/fsnearbychathub.cpp and
//! fsnearbychatcontrol.cpp).

use super::{Row, dock};
use crate::{
    theme::Palette,
    ui::{Panels, icons::Icons},
};
use egui::Rect;

pub fn width(screen: Rect, saved: f32) -> f32 {
    let wanted = if saved > 0.0 { saved } else { dock::default_chat_width(screen) };
    wanted.clamp(200.0, (screen.width() - 32.0).max(200.0))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    chat: &mut crate::ui::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    emoji: Option<&mut crate::ui::emoji::Emoji>,
    row: &Row,
    rect: Rect,
    width: &mut f32,
    committed: &mut bool,
) -> Option<crate::ui::chat::OutgoingChat> {
    if !chat.bar_shown() {
        let button = Rect::from_center_size(rect.center(), egui::Vec2::splat(super::islands::SLOT));
        dock::conversations_button(ui, p, icons, panels, chat, row, button);
        chat.bar_hovered = ui.rect_contains_pointer(rect);
        return None;
    }
    let geo = dock::Geometry {
        chat: rect,
        dock: Rect::NOTHING,
        thumbs: Rect::NOTHING,
        button: super::islands::SLOT + 4.0,
        height: super::islands::HEIGHT,
        widths: Vec::new(),
    };
    let out = dock::chat_island(ui, p, icons, panels, chat, gestures, emoji, row, geo, true, width, committed);
    chat.bar_hovered = ui.rect_contains_pointer(rect);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        bars::{chrome, islands},
        toolbar::{Bar, ToolbarSettings, ToolbarUi},
    };

    #[test]
    fn moved_chat_keeps_its_input_focus_draft_and_send_path_on_every_edge() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        let mut chat = crate::ui::chat::ChatUi::default();
        chat.input = "a draft with é".into();
        let gestures = crate::gestures::Manager::default();
        let mut toolbar = ToolbarUi::default();
        let settings = ToolbarSettings {
            composed: true,
            chat_composed: true,
            ..Default::default()
        };
        let button = crate::ui::bars::top::button("bar_chat", "message-filled");
        let status = islands::tests::status(true, true);
        let icons = Icons::default();
        let favorites = crate::ui::bars::favorites::View::default();
        let mut width = 380.0;
        let mut frame = |bar: Bar, events, editing: bool, chat: &mut crate::ui::chat::ChatUi| {
            let mut sent = None;
            toolbar.open = editing;
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    toolbar.prepare(ui.ctx(), &settings);
                    chrome::prepare(ui, &p, &status, &settings, editing);
                    chrome::prepare_chat(ui.ctx(), chat, width);
                    let row = Row {
                        unread: 0,
                        latest_unread: None,
                        flash: 0.0,
                        flying: false,
                        seated: false,
                        mouselook: false,
                        ao_active: false,
                        locked: false,
                        favorites: &favorites,
                        bar,
                        style: Default::default(),
                        editing,
                        floating: false,
                    };
                    let rect = Rect::from_min_size(egui::pos2(30.0, 30.0), egui::vec2(width, chrome::height(ui.ctx(), "bar_chat")));
                    chrome::register_at(ui, &button, &row, rect);
                    chrome::show(
                        ui,
                        &p,
                        &icons,
                        &mut panels,
                        &status,
                        &mut Default::default(),
                        &mut Default::default(),
                        Default::default(),
                        &mut Default::default(),
                        Default::default(),
                        &mut toolbar,
                        &settings,
                    );
                    sent = chrome::show_chat(ui, &p, &icons, &mut panels, chat, &gestures, None, &row, &mut width).0;
                },
            )
            .textures_delta
            .clear();
            sent
        };
        for bar in Bar::ALL {
            chat.focus_request = true;
            for _ in 0..3 {
                assert!(frame(bar, vec![], false, &mut chat).is_none());
            }
            assert!(chat.has_focus, "{bar:?}: the existing input keeps keyboard focus");
            assert_eq!(chat.input, "a draft with é");
            assert!(ctx.read_response(crate::ui::chat::bar_input_id()).is_some());
            assert!(frame(bar, vec![], true, &mut chat).is_none());
            let source = ctx
                .read_response(crate::ui::bars::tool_button_id("bar_chat"))
                .expect("movable edit cell");
            assert!(source.sense.senses_drag());
            assert_eq!(chat.input, "a draft with é", "edit mode never consumes the draft");
        }
        chat.focus_request = true;
        frame(Bar::Right, vec![], false, &mut chat);
        let sent = frame(
            Bar::Right,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            false,
            &mut chat,
        )
        .expect("existing chat send path");
        assert_eq!(sent.message, "a draft with é");
        assert!(chat.input.is_empty());
        chat.bar_mode = crate::settings::ChatBar::WhenTyping;
        chat.let_keyboard_go();
        for _ in 0..3 {
            frame(Bar::Right, vec![], false, &mut chat);
        }
        assert!(
            ctx.read_response(crate::ui::chat::bar_input_id()).is_none(),
            "an inactive autohidden field leaves its slot"
        );
        assert!(
            ctx.read_response(egui::Id::unique("chat_island_open")).is_some(),
            "the conversations button remains available"
        );
        chat.focus_request = true;
        frame(Bar::Right, vec![], false, &mut chat);
        assert!(chat.has_focus && ctx.read_response(crate::ui::chat::bar_input_id()).is_some());
    }

    /// The recent emojis' row has its own room in the chat block: the field
    /// keeps its height under it (over it with the Legacy chat style). The
    /// block's height used to count the field alone, so the row took the
    /// field's place, caught its clicks, and what was typed went under the
    /// bar.
    #[test]
    fn the_recent_emojis_row_leaves_the_chat_field_its_room() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        let mut chat = crate::ui::chat::ChatUi::default();
        let gestures = crate::gestures::Manager::default();
        let mut toolbar = ToolbarUi::default();
        let settings = ToolbarSettings {
            composed: true,
            chat_composed: true,
            ..Default::default()
        };
        let button = crate::ui::bars::top::button("bar_chat", "message-filled");
        let status = islands::tests::status(true, true);
        let icons = Icons::default();
        let favorites = crate::ui::bars::favorites::View::default();
        let mut emoji = crate::ui::emoji::Emoji::load();
        let mut width = 380.0;
        // one frame of the block on the bottom bar: the slot it was given
        let mut frame = |chat: &mut crate::ui::chat::ChatUi| {
            let mut slot = Rect::NOTHING;
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    toolbar.prepare(ui.ctx(), &settings);
                    chrome::prepare(ui, &p, &status, &settings, false);
                    chrome::prepare_chat(ui.ctx(), chat, width);
                    let row = Row {
                        unread: 0,
                        latest_unread: None,
                        flash: 0.0,
                        flying: false,
                        seated: false,
                        mouselook: false,
                        ao_active: false,
                        locked: false,
                        favorites: &favorites,
                        bar: Bar::Bottom,
                        style: Default::default(),
                        editing: false,
                        floating: false,
                    };
                    slot = Rect::from_min_size(egui::pos2(30.0, 30.0), egui::vec2(width, chrome::height(ui.ctx(), "bar_chat")));
                    chrome::register_at(ui, &button, &row, slot);
                    chrome::show(
                        ui,
                        &p,
                        &icons,
                        &mut panels,
                        &status,
                        &mut Default::default(),
                        &mut Default::default(),
                        Default::default(),
                        &mut Default::default(),
                        Default::default(),
                        &mut toolbar,
                        &settings,
                    );
                    chrome::show_chat(ui, &p, &icons, &mut panels, chat, &gestures, Some(&mut emoji), &row, &mut width);
                },
            )
            .textures_delta
            .clear();
            slot
        };
        let field = |ctx: &egui::Context| ctx.read_response(crate::ui::chat::bar_input_id()).expect("the chat field").rect;
        for _ in 0..2 {
            frame(&mut chat);
        }
        let alone = frame(&mut chat);
        let plain = field(&ctx);
        assert!(
            alone.expand(0.5).contains_rect(plain),
            "the field alone is in its block: {plain:?} in {alone:?}"
        );

        chat.recent_emojis = crate::ui::emoji::recent::Recent::demo("👍😂");
        chat.recent_emojis.enabled = true;
        for above in [true, false] {
            chat.recent_emojis.above = above;
            for _ in 0..2 {
                frame(&mut chat);
            }
            let slot = frame(&mut chat);
            assert_eq!(
                slot.height(),
                alone.height() + crate::ui::emoji::recent::HEIGHT,
                "above {above}: the block grows by the row"
            );
            let typed = field(&ctx);
            assert_eq!(typed.height(), plain.height(), "above {above}: the field keeps its height");
            assert!(
                slot.expand(0.5).contains_rect(typed),
                "above {above}: the field stays in the bar: {typed:?} in {slot:?}"
            );
            let cell = ctx
                .read_response(egui::Id::unique(("recent_emoji", crate::ui::chat::bar_input_id(), 0usize)))
                .expect("the first recent emoji")
                .rect;
            assert!(
                slot.expand(0.5).contains_rect(cell),
                "above {above}: the row is in the bar: {cell:?} in {slot:?}"
            );
            if above {
                assert!(cell.bottom() <= typed.top() + 0.5, "the row is over the field: {cell:?}, {typed:?}");
            } else {
                assert!(
                    cell.top() >= typed.bottom() - 0.5,
                    "the row is under the field: {cell:?}, {typed:?}"
                );
            }
        }

        // autohidden and out of use, the block is its button alone again
        chat.bar_mode = crate::settings::ChatBar::WhenTyping;
        chat.let_keyboard_go();
        for _ in 0..3 {
            frame(&mut chat);
        }
        assert_eq!(
            frame(&mut chat).height(),
            alone.height(),
            "no room kept for a row that is not shown"
        );
    }

    #[test]
    fn first_chat_and_its_gap_remain_in_the_toolbar_display_and_drop_rects() {
        let ctx = egui::Context::default();
        let mut settings = ToolbarSettings::default();
        let layout = crate::ui::skin::Layout::default();
        settings.compose_chat(&layout);
        let buttons = settings.buttons(&layout);
        let display = crate::ui::toolbar::structure::Display::for_bar(&ctx, Bar::Bottom, buttons, true);
        assert_eq!(display.buttons(), buttons);
        let chat = Rect::from_min_size(egui::pos2(8.0, 720.0), egui::vec2(380.0, 42.0));
        let mut rects = vec![Rect::NOTHING; display.buttons().len()];
        rects[0] = chat;
        let full = display.full_rects(&rects);
        assert_eq!(full.len(), buttons.len());
        assert_eq!(full[0], chat);
    }
}
