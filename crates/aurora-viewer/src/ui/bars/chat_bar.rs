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
        toolbar::{Bar, ToolbarSettings, ToolbarUi, structure},
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
        // where the block says it was drawn, read in the frame
        let placed = std::cell::Cell::new(None);
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
                    placed.set(chrome::chat_place(ui.ctx()));
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
            // the block says where it stands, for the chat lines that follow it
            let place = placed.get().expect("the block's place");
            assert_eq!((place.bar, place.floating), (bar, false), "{bar:?}: the block's bar");
            assert_eq!(place.rect.left_top(), egui::pos2(30.0, 30.0), "{bar:?}: the block's rect");
            assert!(frame(bar, vec![], true, &mut chat).is_none());
            assert!(placed.get().is_none(), "{bar:?}: no block drawn while the interface is edited");
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
    /// keeps its height under it, with the Legacy chat style too (the row is
    /// under the field in Conversations alone). The block's height used to
    /// count the field alone, so the row took the field's place, caught its
    /// clicks, and what was typed went under the bar.
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
            assert!(
                cell.bottom() <= typed.top() + 0.5,
                "above {above}: the row is over the field: {cell:?}, {typed:?}"
            );
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

    /// The chat bar's field has the recent emojis' arrow before its emoji
    /// button: a click asks the app to turn "Show recent emojis". The right
    /// click of an emoji of the row is the emoji's alone: its "Favorite",
    /// without the chat block's button and block settings (the human's
    /// requests, 2026-11-27).
    #[test]
    fn the_chat_bar_has_the_recent_emojis_arrow_and_an_emojis_menu_is_its_own() {
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
        let mut time = 0.0;
        // one frame of the block on the bottom bar: whether a menu is open,
        // and whether it ended with the chat block's sections
        let mut frame = |chat: &mut crate::ui::chat::ChatUi, events: Vec<egui::Event>| {
            let mut menu = (false, false);
            time += 1.0 / 60.0;
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    time: Some(time),
                    events,
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
                    let slot = Rect::from_min_size(egui::pos2(30.0, 30.0), egui::vec2(width, chrome::height(ui.ctx(), "bar_chat")));
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
                    menu = (
                        egui::Popup::is_any_open(ui.ctx()),
                        crate::ui::toolbar::native_context::shown(ui.ctx(), "bar_chat"),
                    );
                },
            )
            .textures_delta
            .clear();
            menu
        };
        let click = |at: egui::Pos2, button: egui::PointerButton, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let rect = |id: egui::Id| ctx.read_response(id).map(|r| r.rect);

        chat.recent_emojis = crate::ui::emoji::recent::Recent::demo("👍😂");
        chat.recent_emojis.enabled = true;
        for _ in 0..3 {
            frame(&mut chat, vec![]);
        }
        let arrow = rect(dock::recent_arrow_id()).expect("the recent emojis' arrow");
        let smile = rect(egui::Id::unique("chat_island_emoji")).expect("the emoji button");
        let typed = rect(crate::ui::chat::bar_input_id()).expect("the chat field");
        assert!(
            arrow.right() <= smile.left() + 0.5,
            "the arrow is before the emoji: {arrow:?}, {smile:?}"
        );
        assert!(
            typed.right() <= arrow.left() + 0.5,
            "the text stops before the arrow: {typed:?}, {arrow:?}"
        );
        assert_eq!(arrow.center().y, smile.center().y, "on the emoji's line");

        // its click is asked of the app, once
        assert!(!chat.recent_emojis.take_toggle());
        frame(&mut chat, vec![egui::Event::PointerMoved(arrow.center())]);
        frame(&mut chat, vec![click(arrow.center(), egui::PointerButton::Primary, true)]);
        frame(&mut chat, vec![click(arrow.center(), egui::PointerButton::Primary, false)]);
        assert!(chat.recent_emojis.take_toggle(), "the arrow's click turns the switch");

        // the right click of an emoji of the row: a menu, the emoji's alone
        let cell = rect(egui::Id::unique(("recent_emoji", crate::ui::chat::bar_input_id(), 0usize))).expect("the first recent emoji");
        frame(&mut chat, vec![egui::Event::PointerMoved(cell.center())]);
        frame(&mut chat, vec![click(cell.center(), egui::PointerButton::Secondary, true)]);
        frame(&mut chat, vec![click(cell.center(), egui::PointerButton::Secondary, false)]);
        let (open, block_sections) = frame(&mut chat, vec![]);
        assert!(open, "the emoji's menu is open");
        assert!(!block_sections, "without the chat block's settings");
        assert!(!chat.recent_emojis.take_toggle());
    }

    /// The windows keep clear of the chat island raised over the dock's row
    /// all along the screen, but a side bar keeps its place: its buttons do
    /// not move when the recent emojis' row comes and goes, on the side of
    /// the chat block as on the other. Only a bar that would stand over the
    /// raised island (held to the bottom of its room) gives it room.
    #[test]
    fn a_side_bar_keeps_its_place_beside_the_raised_chat_island() {
        let row = crate::ui::emoji::recent::HEIGHT;
        let centred = side_bars_as_the_row_comes(crate::ui::toolbar::Align::Center);
        assert_eq!(
            centred.rooms[1].bottom(),
            centred.rooms[0].bottom() - row,
            "the windows leave the raised island its room"
        );
        assert_eq!(centred.fly[1], centred.fly[0], "the right bar keeps its place");
        assert_eq!(
            centred.sit[1], centred.sit[0],
            "the left bar, on the chat block's side, does not reach the island: it keeps its place too"
        );

        let held = side_bars_as_the_row_comes(crate::ui::toolbar::Align::End);
        assert_eq!(held.fly[1], held.fly[0], "the right bar keeps its place");
        assert_eq!(
            held.sit[1],
            held.sit[0].translate(egui::vec2(0.0, -row)),
            "the left bar held to the bottom would stand over the raised island: it gives it room"
        );
        assert!(held.sit[1].bottom() <= held.rooms[1].bottom(), "{held:?}");
    }

    /// Two islands raised in one frame (the chat island of its own and the
    /// dock's chat block): a side bar's room goes down by the higher one,
    /// what the windows' room lost, and the place to keep clear of holds
    /// both. The second said used to replace the first.
    #[test]
    fn two_raised_chat_islands_count_as_the_higher_one() {
        let ctx = egui::Context::default();
        let room = Rect::from_min_max(egui::pos2(0.0, 50.0), egui::pos2(1536.0, 760.0));
        assert_eq!(chrome::side_room(&ctx, room), (room, None), "nothing raised");
        chrome::raise_chat(&ctx, egui::Rangef::new(8.0, 388.0), 53.0);
        chrome::raise_chat(&ctx, egui::Rangef::new(400.0, 790.0), 26.0);
        chrome::raise_chat(&ctx, egui::Rangef::new(900.0, 950.0), 0.0);
        let (own, raised) = chrome::side_room(&ctx, room);
        assert_eq!(own, Rect::from_min_max(room.min, egui::pos2(1536.0, 813.0)));
        assert_eq!(
            raised,
            Some(Rect::from_min_max(
                egui::pos2(8.0 - islands::MARGIN, 760.0),
                egui::pos2(790.0 + islands::MARGIN, 813.0)
            ))
        );
    }

    /// The windows' room and the buttons of a left (Sit) and a right (Fly)
    /// bar, the recent emojis' row hidden then shown, with the chat block at
    /// the left of the dock and the left bar aligned by `left`.
    #[derive(Debug)]
    struct SideBars {
        rooms: [Rect; 2],
        sit: [Rect; 2],
        fly: [Rect; 2],
    }

    fn side_bars_as_the_row_comes(left: crate::ui::toolbar::Align) -> SideBars {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        let mut chat = crate::ui::chat::ChatUi::default();
        let gestures = crate::gestures::Manager::default();
        let mut toolbar = ToolbarUi::default();
        toolbar.islands = true;
        let skin = crate::ui::skin::Layout::default();
        let mut settings = ToolbarSettings {
            composed: true,
            left: vec![crate::ui::bars::top::button("sit", "armchair")],
            right: vec![crate::ui::bars::top::button("fly", "plane-tilt-filled")],
            ..Default::default()
        };
        settings.left_style.align = Some(left);
        settings.compose_chat(&skin);
        let status = islands::tests::status(true, true);
        let icons = Icons::default();
        let favorites = crate::ui::bars::favorites::View::default();
        let mut emoji = crate::ui::emoji::Emoji::load();
        let mut width = 380.0;
        // one frame of the dock and the side bars: the room left to the windows
        let mut frame = |chat: &mut crate::ui::chat::ChatUi| {
            let mut room = Rect::NOTHING;
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1536.0, 864.0))),
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
                        style: settings.style(Bar::Bottom).resolve(Bar::Bottom, true),
                        editing: false,
                        floating: false,
                    };
                    let mut mic = crate::ui::bars::MicButton::default();
                    let top = dock::bottom(
                        ui,
                        &p,
                        &icons,
                        &skin,
                        &mut toolbar,
                        &settings,
                        &mut panels,
                        chat,
                        &gestures,
                        Some(&mut emoji),
                        &row,
                        &mut width,
                        &mut mic,
                    )
                    .2;
                    let area = crate::ui::layout::bars_room(ui.available_rect_before_wrap(), top, false);
                    let side = Row {
                        bar: Bar::Left,
                        style: Default::default(),
                        ..row
                    };
                    room = crate::ui::bars::side::bars(
                        ui,
                        &p,
                        &icons,
                        &skin,
                        &mut toolbar,
                        &settings,
                        &mut panels,
                        &mut mic,
                        &side,
                        area,
                        true,
                        [true, true],
                    )
                    .0;
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
            room
        };
        let button = |command: &str| {
            ctx.read_response(crate::ui::bars::tool_button_id(command))
                .unwrap_or_else(|| panic!("the {command} button"))
                .rect
        };
        let mut settle = |chat: &mut crate::ui::chat::ChatUi| {
            for _ in 0..3 {
                frame(chat);
            }
            frame(chat)
        };

        let room = settle(&mut chat);
        let (sit, fly) = (button("sit"), button("fly"));
        let field = ctx.read_response(crate::ui::chat::bar_input_id()).expect("the chat field").rect;
        assert!(field.center().x < 768.0, "the chat block is at the left of the dock: {field:?}");

        chat.recent_emojis = crate::ui::emoji::recent::Recent::demo("👍😂");
        chat.recent_emojis.enabled = true;
        let raised = settle(&mut chat);
        let shown = (button("sit"), button("fly"));

        chat.recent_emojis.enabled = false;
        assert_eq!(settle(&mut chat), room);
        assert_eq!((button("sit"), button("fly")), (sit, fly), "the row gone, both are where they were");
        SideBars {
            rooms: [room, raised],
            sit: [sit, shown.0],
            fly: [fly, shown.1],
        }
    }

    /// In the top bar, the chat block's island alone hangs under the bar's
    /// row for the recent emojis' row and for a paragraph: the bar and its
    /// buttons keep their height, the field stays on the bar's line and the
    /// row comes under it. What hangs under the bar is still the interface,
    /// and takes its clicks.
    #[test]
    fn the_top_bars_chat_island_alone_hangs_for_the_recent_emojis_and_a_paragraph() {
        type Frame<'a> = &'a mut dyn FnMut(Vec<egui::Event>, &mut crate::ui::chat::ChatUi);
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        let mut chat = crate::ui::chat::ChatUi::default();
        let gestures = crate::gestures::Manager::default();
        let mut toolbar = ToolbarUi::default();
        toolbar.islands = true;
        let skin = crate::ui::skin::Layout::default();
        let settings = ToolbarSettings {
            composed: true,
            chat_composed: true,
            top: vec![
                crate::ui::bars::top::button("bar_chat", "message-filled"),
                structure::separator("chat-tools"),
                crate::ui::bars::top::button("fly", "plane-tilt-filled"),
            ],
            ..Default::default()
        };
        let status = islands::tests::status(true, true);
        let icons = Icons::default();
        let favorites = crate::ui::bars::favorites::View::default();
        let mut emoji = crate::ui::emoji::Emoji::load();
        let mut width = 380.0;
        let mut frame = |events, chat: &mut crate::ui::chat::ChatUi| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1536.0, 864.0))),
                    events,
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
                        bar: Bar::Top,
                        style: settings.style(Bar::Top).resolve(Bar::Top, true),
                        editing: false,
                        floating: false,
                    };
                    crate::ui::bars::top::show(
                        ui,
                        &p,
                        &icons,
                        &skin,
                        &mut toolbar,
                        &settings,
                        &mut panels,
                        &mut crate::ui::bars::MicButton::default(),
                        &row,
                        false,
                    );
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
        };
        let rect = |id: egui::Id, what: &str| ctx.read_response(id).unwrap_or_else(|| panic!("{what}")).rect;
        let field = || rect(crate::ui::chat::bar_input_id(), "the chat field");
        let top = || rect(egui::Id::unique("top_toolbar_background"), "the top bar");
        let button = || rect(crate::ui::bars::tool_button_id("fly"), "the button beside the chat");
        let arrow = || rect(dock::recent_arrow_id(), "the recent emojis' arrow");
        let cell = || {
            rect(
                egui::Id::unique(("recent_emoji", crate::ui::chat::bar_input_id(), 0usize)),
                "the first recent emoji",
            )
        };
        let settle = |frame: Frame, chat: &mut crate::ui::chat::ChatUi| {
            for _ in 0..4 {
                frame(vec![], chat);
            }
        };
        let click = |frame: Frame, chat: &mut crate::ui::chat::ChatUi, pos: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(vec![egui::Event::PointerMoved(pos), press(true)], chat);
            frame(vec![press(false)], chat);
            frame(vec![], chat);
        };

        settle(&mut frame, &mut chat);
        let (plain, bar, beside, chevron) = (field(), top(), button(), arrow());
        assert_eq!(bar.height(), islands::HEIGHT, "a line: {bar:?}");
        assert!(
            bar.expand(0.5).contains_rect(plain),
            "the field alone is in the bar: {plain:?} in {bar:?}"
        );

        chat.recent_emojis = crate::ui::emoji::recent::Recent::demo("👍😂");
        chat.recent_emojis.enabled = true;
        settle(&mut frame, &mut chat);
        assert_eq!(top(), bar, "the bar keeps its place and its height");
        assert_eq!(button(), beside, "so does the button beside the chat");
        assert_eq!(arrow(), chevron, "and the arrow, under the pointer that pressed it");
        let (typed, first) = (field(), cell());
        assert_eq!(typed, plain, "the field stays on the bar's line");
        assert!(
            first.top() >= typed.bottom() - 0.5,
            "the row is under the field: {first:?}, {typed:?}"
        );
        assert!(first.bottom() > bar.bottom(), "the island hangs under the bar: {first:?}, {bar:?}");
        // under the bar, the pointer is still on the interface (an island's
        // area; the world is the background's)
        let island = |pos| ctx.layer_id_at(pos).is_some_and(|layer| layer.order == egui::Order::Middle);
        let out = egui::pos2(first.center().x, (first.bottom() + bar.bottom()) / 2.0);
        assert!(island(out), "{out:?} is over the interface");
        assert!(
            !island(egui::pos2(beside.center().x, out.y)),
            "under the other islands, the world keeps its clicks"
        );
        chat.input.clear();
        click(&mut frame, &mut chat, first.center());
        assert!(!chat.input.is_empty(), "a click on a recent emoji writes it");

        // a paragraph hangs the island the same way, from the bar's line
        chat.recent_emojis.enabled = false;
        chat.input = "one\ntwo\nthree".into();
        settle(&mut frame, &mut chat);
        let typed = field();
        assert_eq!((top(), button()), (bar, beside), "the bar and its buttons keep their place");
        assert!(
            (typed.top() - plain.top()).abs() <= 1.0 && typed.bottom() > bar.bottom(),
            "the paragraph starts on the bar's line and hangs under it: {typed:?}, {plain:?}, {bar:?}"
        );

        chat.input.clear();
        settle(&mut frame, &mut chat);
        assert_eq!((field(), top(), button()), (plain, bar, beside), "a line again: all as before");
    }

    /// On the dock, the chat block's island alone rises for the recent
    /// emojis' row and for a paragraph, as the chat island did before the
    /// field became a block: the dock and its buttons keep their height.
    /// What stands out over the dock is still the interface, and takes its
    /// clicks.
    #[test]
    fn the_docks_chat_island_alone_rises_for_the_recent_emojis_and_a_paragraph() {
        type Frame<'a> = &'a mut dyn FnMut(Vec<egui::Event>, &mut crate::ui::chat::ChatUi) -> f32;
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        let mut chat = crate::ui::chat::ChatUi::default();
        let gestures = crate::gestures::Manager::default();
        let mut toolbar = ToolbarUi::default();
        toolbar.islands = true;
        let skin = crate::ui::skin::Layout::default();
        let mut settings = ToolbarSettings {
            composed: true,
            ..Default::default()
        };
        settings.compose_chat(&skin);
        let neighbour = settings
            .buttons(&skin)
            .iter()
            .map(|button| button.command.clone())
            .find(|command| !structure::is_block(command) && !structure::is_separator(command))
            .expect("a plain button beside the chat block");
        let status = islands::tests::status(true, true);
        let icons = Icons::default();
        let favorites = crate::ui::bars::favorites::View::default();
        let mut emoji = crate::ui::emoji::Emoji::load();
        let mut width = 380.0;
        // one frame of the dock: the top of what the bottom islands keep
        let mut frame = |events, chat: &mut crate::ui::chat::ChatUi| {
            let mut top = 0.0;
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1536.0, 864.0))),
                    events,
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
                        style: settings.style(Bar::Bottom).resolve(Bar::Bottom, true),
                        editing: false,
                        floating: false,
                    };
                    top = dock::bottom(
                        ui,
                        &p,
                        &icons,
                        &skin,
                        &mut toolbar,
                        &settings,
                        &mut panels,
                        chat,
                        &gestures,
                        Some(&mut emoji),
                        &row,
                        &mut width,
                        &mut crate::ui::bars::MicButton::default(),
                    )
                    .2;
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
            top
        };
        let rect = |id: egui::Id, what: &str| ctx.read_response(id).unwrap_or_else(|| panic!("{what}")).rect;
        let field = || rect(crate::ui::chat::bar_input_id(), "the chat field");
        let dock = || rect(egui::Id::unique("toolbar_background"), "the dock");
        let button = || rect(crate::ui::bars::tool_button_id(&neighbour), "the button beside the chat");
        let cell = || {
            rect(
                egui::Id::unique(("recent_emoji", crate::ui::chat::bar_input_id(), 0usize)),
                "the first recent emoji",
            )
        };
        let settle = |frame: Frame, chat: &mut crate::ui::chat::ChatUi| {
            for _ in 0..3 {
                frame(vec![], chat);
            }
            frame(vec![], chat)
        };
        let click = |frame: Frame, chat: &mut crate::ui::chat::ChatUi, pos: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(vec![egui::Event::PointerMoved(pos), press(true)], chat);
            frame(vec![press(false)], chat);
            frame(vec![], chat);
        };

        let top = settle(&mut frame, &mut chat);
        let (plain, bar, beside) = (field(), dock(), button());
        // the emoji button opens its picker, and closes it: the block
        // stands on a layer over the dock's, and the click that opens the
        // picker is not one away from the button
        let smile = || rect(egui::Id::unique("chat_island_emoji"), "the emoji button").center();
        for open in [true, false] {
            click(&mut frame, &mut chat, smile());
            assert_eq!(chat.bar_emoji_open, open, "the emoji button's click, in the dock");
        }
        assert!(
            bar.expand(0.5).contains_rect(plain),
            "the field alone is in the dock: {plain:?} in {bar:?}"
        );

        chat.recent_emojis = crate::ui::emoji::recent::Recent::demo("👍😂");
        chat.recent_emojis.enabled = true;
        let row = crate::ui::emoji::recent::HEIGHT;
        for above in [true, false] {
            chat.recent_emojis.above = above;
            let raised = settle(&mut frame, &mut chat);
            assert_eq!(dock(), bar, "above {above}: the dock keeps its place and its height");
            assert_eq!(button(), beside, "above {above}: so does the button beside the chat");
            assert_eq!(
                raised,
                top - row,
                "above {above}: the windows and the chat lines leave the row its room"
            );
            let (typed, first) = (field(), cell());
            assert_eq!(typed.height(), plain.height(), "above {above}: the field keeps its height");
            // (the Legacy chat style's row is under the field in
            // Conversations alone: over it in the bar, with every style)
            assert_eq!(typed, plain, "above {above}: the field stays where it is under the row");
            assert!(
                first.bottom() <= typed.top() + 0.5,
                "above {above}: the row is over the field: {first:?}, {typed:?}"
            );
            let over = first;
            assert!(
                over.top() < bar.top(),
                "above {above}: the island stands out over the dock: {over:?}, {bar:?}"
            );
            // over the dock, the pointer is still on the interface (an
            // island's area; the world is the background's)
            let island = |pos| ctx.layer_id_at(pos).is_some_and(|layer| layer.order == egui::Order::Middle);
            let out = egui::pos2(over.center().x, (over.top() + bar.top()) / 2.0);
            assert!(island(out), "above {above}: {out:?} is over the interface");
            assert!(
                !island(egui::pos2(beside.center().x, out.y)),
                "above {above}: over the other islands, the world keeps its clicks"
            );
            for open in [true, false] {
                click(&mut frame, &mut chat, smile());
                assert_eq!(chat.bar_emoji_open, open, "above {above}: the emoji button's click");
            }
            chat.input.clear();
            click(&mut frame, &mut chat, egui::pos2(first.center().x, out.y));
            assert!(!chat.input.is_empty(), "above {above}: a click on a recent emoji writes it");
        }

        // a paragraph raises the island the same way, and its field takes
        // the clicks over the dock
        chat.recent_emojis.enabled = false;
        chat.input = "one\ntwo\nthree".into();
        let raised = settle(&mut frame, &mut chat);
        let typed = field();
        chat.let_keyboard_go();
        frame(vec![], &mut chat);
        click(
            &mut frame,
            &mut chat,
            egui::pos2(typed.left() + 40.0, (typed.top() + bar.top()) / 2.0),
        );
        assert!(chat.has_focus, "a click in the raised field gives it the keyboard");
        let raised = raised.min(settle(&mut frame, &mut chat));
        let typed = field();
        assert!(typed.height() > plain.height(), "the field holds the paragraph: {typed:?}");
        assert_eq!((dock(), button()), (bar, beside), "the dock keeps its height under a paragraph");
        assert!(
            typed.top() < bar.top(),
            "the paragraph stands out over the dock: {typed:?}, {bar:?}"
        );
        assert!(
            raised < typed.top() && raised < top,
            "the windows and the chat lines stop over the paragraph: {raised}, {typed:?}"
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
