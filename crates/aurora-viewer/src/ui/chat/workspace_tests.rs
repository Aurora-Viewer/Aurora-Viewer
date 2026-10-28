//! Aurora's workspace through real egui frames: the column, the menu of
//! a line, the members' panel, the rail and the window that stays in place.

use super::super::sidebar::{Kind, Pinned, entry_id};
use super::super::{ChatUi, ConvAction, show};
use crate::settings::ChatStyle;
use crate::ui::widgets::elide_tests::{Frames, painted};
use crate::world::{ChatKind, ChatLine, Origin, World};
use std::collections::HashMap;
use uuid::Uuid;

const JANE: Uuid = Uuid::from_u128(0xA);
const BOB: Uuid = Uuid::from_u128(0xB);

fn line(from: &str, source: Uuid, kind: ChatKind, text: &str) -> ChatLine {
    ChatLine {
        time: std::time::SystemTime::now(),
        from: from.into(),
        text: text.into(),
        kind,
        source,
        origin: Origin::Live,
    }
}

/// Where the last text `source` is painted (a line of the thread, after
/// the column's sub-line saying the same).
fn last(texts: &[crate::ui::widgets::elide_tests::Text], source: &str) -> egui::Rect {
    texts.iter().rev().find(|t| t.source == source).expect("the text is painted").rect
}

/// The window, its world and what it asked, frame after frame.
struct Harness {
    frames: Frames,
    world: World,
    st: ChatUi,
    icons: crate::ui::icons::Icons,
    emoji: crate::ui::emoji::Emoji,
    contacts: crate::ui::contacts::ContactsUi,
    contacts_settings: crate::ui::contacts::ContactsSettings,
    gestures: crate::gestures::Manager,
    actions: Vec<ConvAction>,
    /// Preferences › Chat › Show the Contacts tab.
    show_contacts: bool,
    friends_pane: bool,
    people: crate::ui::people::PeopleUi,
    friends: crate::ui::people::FriendsSettings,
    /// The window is open (its cross or Ctrl+W closes it).
    open: bool,
}

impl Harness {
    fn new(style: ChatStyle) -> Harness {
        let frames = Frames::new();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "hello from Jane"));
        world.conversation_line(BOB, line("Bob", BOB, ChatKind::Im, "hello from Bob"));
        Harness {
            frames,
            world,
            st: ChatUi {
                style,
                ..ChatUi::default()
            },
            icons,
            emoji: crate::ui::emoji::Emoji::load(),
            contacts: Default::default(),
            contacts_settings: Default::default(),
            gestures: Default::default(),
            actions: Vec::new(),
            show_contacts: false,
            friends_pane: false,
            people: Default::default(),
            friends: Default::default(),
            open: true,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<crate::ui::widgets::elide_tests::Text> {
        let p = crate::theme::Theme::default().palette();
        let pics = HashMap::new();
        let Harness {
            frames,
            world,
            st,
            icons,
            emoji,
            contacts,
            contacts_settings,
            gestures,
            actions,
            show_contacts,
            friends_pane,
            people,
            friends,
            open,
        } = self;
        let radar = crate::world::radar::Radar::default();
        let mut radar_settings = crate::ui::people::RadarSettings::default();
        let voice = HashMap::new();
        let data = crate::ui::people::RadarData {
            radar: &radar,
            settings: &mut radar_settings,
            voice: &voice,
            draw_distance: 64.0,
            now: 0.0,
            pics: &pics,
        };
        frames.run(events, |ui| {
            // as the app does before any window
            crate::ui::close_key::begin(ui.ctx());
            actions.extend(show(
                ui.ctx(),
                &p,
                icons,
                emoji,
                &pics,
                &pics,
                world,
                gestures,
                st,
                contacts,
                contacts_settings,
                *show_contacts,
                (*friends_pane).then_some(crate::ui::people::FriendsPane {
                    st: people,
                    data: &data,
                    settings: friends,
                    images: &pics,
                }),
                open,
            ));
        })
    }

    fn settle(&mut self) -> Vec<crate::ui::widgets::elide_tests::Text> {
        self.frame(Vec::new());
        self.frame(Vec::new());
        self.frame(Vec::new())
    }

    fn rect(&self, id: egui::Id) -> egui::Rect {
        self.frames.ctx.read_response(id).expect("the widget was drawn").rect
    }

    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
    }

    fn hold(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        for _ in 0..36 {
            self.frame(Vec::new());
        }
        self.frame(vec![button(false)]);
    }

    /// A right click at `at`, and the frames that show its menu.
    fn right_click(&mut self, at: egui::Pos2) -> Vec<crate::ui::widgets::elide_tests::Text> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.settle()
    }
}

/// The column lists the conversations; a click shows one and gives its
/// field the keyboard, the × of an entry under the pointer closes it.
#[test]
fn the_column_shows_and_closes_conversations() {
    let mut h = Harness::new(ChatStyle::Aurora);
    let texts = h.settle();
    assert!(
        painted(&texts, "hello from Bob").rect.width() > 0.0,
        "the last message under the name"
    );
    let bob = h.rect(entry_id(Pinned::Conversation(BOB)));
    h.click(bob.center());
    h.settle();
    assert_eq!(h.st.selected, Some(BOB));
    let field = egui::Id::unique(("conv_input", Some(BOB)));
    assert!(h.frames.ctx.memory(|m| m.has_focus(field)));
    // back to the nearby chat, then Jane's entry closed from its ×
    let local = h.rect(entry_id(Pinned::Local));
    h.click(local.center());
    assert_eq!(h.st.selected, None);
    let jane = h.rect(entry_id(Pinned::Conversation(JANE)));
    h.frame(vec![egui::Event::PointerMoved(jane.center())]);
    h.frame(Vec::new());
    let close = h.rect(entry_id(Pinned::Conversation(JANE)).with("close"));
    h.click(close.center());
    assert!(
        h.actions.iter().any(|a| matches!(a, ConvAction::Close(id) if *id == JANE)),
        "the × closes the conversation"
    );
    assert_eq!(h.st.selected, None, "without showing it");
}

/// The right-click menu of a message: Quote puts "> text" in the field,
/// which takes the keyboard.
#[test]
fn a_line_is_quoted_from_its_menu() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.st.selected = Some(JANE);
    let texts = h.settle();
    // the line (the column's sub-line says it too, further left)
    let text = last(&texts, "hello from Jane");
    let button = |pressed| egui::Event::PointerButton {
        pos: text.center(),
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    h.frame(vec![egui::Event::PointerMoved(text.center())]);
    h.frame(vec![button(true)]);
    h.frame(vec![button(false)]);
    let texts = h.settle();
    h.click(last(&texts, aurora_i18n::tr!("ui-chat-quote-message")).center());
    h.settle();
    assert_eq!(h.st.draft_mut(Some(JANE)), "> hello from Jane ");
    assert!(h.frames.ctx.memory(|m| m.has_focus(egui::Id::unique(("conv_input", Some(JANE))))));
}

/// A drag on the head (not the title bar) leaves the window in place.
#[test]
fn the_window_does_not_move_from_its_workspace() {
    let mut h = Harness::new(ChatStyle::Compact);
    h.st.selected = Some(JANE);
    let texts = h.settle();
    let before = last(&texts, "hello from Jane");
    let bob = h.rect(entry_id(Pinned::Conversation(BOB)));
    // the column's background under its entries, then the head's middle
    for from in [
        egui::pos2(bob.center().x, bob.bottom() + 120.0),
        egui::pos2(before.center().x + 120.0, before.top() - 200.0),
    ] {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
        for k in 1..=4 {
            h.frame(vec![egui::Event::PointerMoved(from + egui::vec2(30.0, 20.0) * k as f32)]);
        }
        h.frame(vec![button(from + egui::vec2(120.0, 80.0), false)]);
    }
    let texts = h.settle();
    assert_eq!(last(&texts, "hello from Jane"), before, "the window stayed in place");
}

/// Narrow, the column is a rail of pictures: no search field, no names.
#[test]
fn a_narrow_window_has_a_rail() {
    let mut h = Harness::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(460.0, 420.0));
    let texts = h.settle();
    let entry = h.rect(entry_id(Pinned::Conversation(BOB)));
    assert!(entry.width() <= super::super::sidebar::RAIL, "{entry:?}");
    assert!(texts.iter().all(|t| t.source != "hello from Bob"), "no sub-line in the rail");
}

/// A key pressed and released (a key egui still holds down comes back as
/// a repeat).
fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .to_vec()
}

/// Alt + the arrows go round the conversations with the keyboard in the
/// window, the field of the one shown taking it; Ctrl+W closes the one
/// shown and shows the next, then on the nearby chat closes the window
/// (#651), in both styles of the list.
#[test]
fn the_keyboard_goes_round_the_conversations_and_closes_them() {
    for style in [ChatStyle::Legacy, ChatStyle::Aurora] {
        let mut h = Harness::new(style);
        h.st.conv_focus_request = true;
        h.settle();
        let order = if style.workspace() {
            super::super::sidebar::tab_order(&h.world, &h.st)
        } else {
            super::super::keys::legacy_order(&h.world, &h.st)
        };
        assert_eq!(order.len(), 3, "{style:?}: the nearby chat, Jane and Bob");
        let (first, last) = (order[1], order[2]);
        let field = |tab| egui::Id::unique(("conv_input", tab));
        h.frame(key(egui::Key::ArrowDown, egui::Modifiers::ALT));
        h.settle();
        assert_eq!(h.st.selected, first, "{style:?}");
        assert!(
            h.frames.ctx.memory(|m| m.has_focus(field(first))),
            "{style:?}: its field has the keyboard"
        );
        h.frame(key(egui::Key::ArrowUp, egui::Modifiers::ALT));
        h.settle();
        assert_eq!(h.st.selected, None, "{style:?}");
        // round the list
        h.frame(key(egui::Key::ArrowLeft, egui::Modifiers::ALT));
        h.settle();
        assert_eq!(h.st.selected, last, "{style:?}");
        // the keyboard elsewhere: the camera's keys
        h.frames.ctx.memory_mut(|m| m.surrender_focus(field(last)));
        h.frame(key(egui::Key::ArrowRight, egui::Modifiers::ALT));
        h.settle();
        assert_eq!(h.st.selected, last, "{style:?}");
        // Ctrl+W: the conversation shown, the one above it shown next (it was the last)
        h.frame(key(egui::Key::W, egui::Modifiers::COMMAND));
        h.settle();
        let closed = last.expect("a conversation");
        assert!(
            h.actions.iter().any(|a| matches!(a, ConvAction::Close(id) if *id == closed)),
            "{style:?}"
        );
        assert_eq!(h.st.selected, first, "{style:?}");
        assert!(h.open, "{style:?}");
        // on the nearby chat, the window
        h.st.selected = None;
        h.settle();
        h.actions.clear();
        h.frame(key(egui::Key::W, egui::Modifiers::COMMAND));
        assert!(!h.open, "{style:?}");
        assert!(!h.actions.iter().any(|a| matches!(a, ConvAction::Close(_))), "{style:?}");
    }
}

/// The kinds of the column's sections.
#[test]
fn sessions_fall_in_their_sections() {
    let h = Harness::new(ChatStyle::Aurora);
    assert_eq!(super::super::sidebar::kind(&h.world, &JANE), Kind::Private);
}

/// A contact set shown apart takes its members' private conversations into
/// a section of its own, named after it, above the private messages
/// (#1104): the keyboard follows it, and a drag does not leave it.
#[test]
fn a_set_shown_apart_has_its_own_section() {
    use super::super::sidebar::{Section, section, tab_order};
    let mut h = Harness::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 650.0));
    h.world.contact_sets.add_set("Besties");
    h.world.contact_sets.add_to_set(&[BOB], "Besties", |_| true);
    let texts = h.settle();
    assert_eq!(tab_order(&h.world, &h.st), [None, Some(JANE), Some(BOB)], "off: with the others");
    assert!(texts.iter().all(|t| t.source != "BESTIES"));
    h.world.contact_sets.set_separate_chats("Besties", true);
    let texts = h.settle();
    assert_eq!(section(&h.world, &BOB), Section::Set(0));
    assert_eq!(tab_order(&h.world, &h.st), [None, Some(BOB), Some(JANE)]);
    let head = painted(&texts, "BESTIES").rect;
    let private = painted(&texts, &aurora_i18n::tr!("ui-chat-section-private").to_uppercase()).rect;
    let (bob, jane) = (h.conversation_rect(BOB), h.conversation_rect(JANE));
    assert!(head.center().y < bob.center().y, "{head:?} {bob:?}");
    assert!(bob.center().y < private.center().y && private.center().y < jane.center().y);
    let original = h.st.reorder.sessions(&h.world);
    h.drag_conversation(bob.center(), jane.center(), Harness::drop_at(jane.center()));
    assert_eq!(h.st.reorder.sessions(&h.world), original, "no move out of the set's section");
}

impl Harness {
    fn conversation_rect(&mut self, id: Uuid) -> egui::Rect {
        let texts = self.settle();
        if self.st.style.workspace() {
            self.rect(entry_id(Pinned::Conversation(id)))
        } else {
            painted(&texts, &self.world.session_title(&id)).rect
        }
    }

    fn drag_conversation(&mut self, from: egui::Pos2, to: egui::Pos2, end: Vec<egui::Event>) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.frame(vec![egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        for t in [0.3, 0.6, 1.0] {
            self.frame(vec![egui::Event::PointerMoved(from.lerp(to, t))]);
        }
        self.frame(end);
    }

    fn drop_at(at: egui::Pos2) -> Vec<egui::Event> {
        vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]
    }
}

#[test]
fn conversation_drag_reorders_every_style_without_switching_or_changing_sessions() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact, ChatStyle::Bubbles, ChatStyle::Legacy] {
        let mut h = Harness::new(style);
        // The older tabs have automatic widget ids: locate their painted
        // names, which must be distinct rather than two unresolved "…".
        h.world.social.names.insert(JANE, "Jane".into());
        h.world.social.names.insert(BOB, "Bob".into());
        h.st.selected = Some(BOB);
        *h.st.draft_mut(Some(BOB)) = "unfinished".into();
        let jane = h.conversation_rect(JANE).center();
        let bob = h.conversation_rect(BOB).center() + egui::vec2(0.0, 5.0);
        h.drag_conversation(jane, bob, Harness::drop_at(bob));
        let bob = h.conversation_rect(BOB);
        let jane = h.conversation_rect(JANE);
        assert!(bob.center().y < jane.center().y, "{style:?}: Bob is now first");
        assert_eq!(h.st.selected, Some(BOB));
        assert_eq!(h.st.draft_mut(Some(BOB)), "unfinished");
        assert_eq!(h.world.social.ims.iter().map(|s| s.other).collect::<Vec<_>>(), [JANE, BOB]);
        assert!(h.actions.is_empty(), "ordering sends no network action");
        // Switching presentation keeps the same order.
        h.st.style = if style.workspace() { ChatStyle::Legacy } else { ChatStyle::Aurora };
        let bob = h.conversation_rect(BOB);
        let jane = h.conversation_rect(JANE);
        assert!(
            bob.center().y < jane.center().y,
            "{style:?} to {:?}: Bob {bob:?}, Jane {jane:?}, order {:?}",
            h.st.style,
            h.st.reorder.sessions(&h.world)
        );
    }
}

#[test]
fn conversation_drag_stays_in_its_category_and_keeps_pinned_entries() {
    const GROUP: Uuid = Uuid::from_u128(0xC0);
    const GROUP2: Uuid = Uuid::from_u128(0xC1);
    const CONFERENCE: Uuid = Uuid::from_u128(0xC2);
    let mut h = Harness::new(ChatStyle::Aurora);
    h.show_contacts = true;
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 650.0));
    for (id, group) in [(GROUP, true), (GROUP2, true), (CONFERENCE, false)] {
        let mut info = crate::world::groups::SessionInfo::default();
        info.group = group;
        info.name = id.to_string();
        h.world.groups.sessions.insert(id, info);
        h.world.conversation_line(id, line("Jane", JANE, ChatKind::Im, "test"));
    }
    h.settle();
    let original = h.st.reorder.sessions(&h.world);
    for (source, target) in [(JANE, GROUP), (GROUP, JANE), (GROUP, CONFERENCE)] {
        let from = h.conversation_rect(source).center();
        let to = h.conversation_rect(target).center();
        h.drag_conversation(from, to, Harness::drop_at(to));
        assert_eq!(h.st.reorder.sessions(&h.world), original, "no cross-category move");
    }
    for pinned in [Pinned::Contacts, Pinned::Local] {
        h.settle();
        let from = h.conversation_rect(JANE).center();
        let to = h.rect(entry_id(pinned)).center();
        h.drag_conversation(from, to, Harness::drop_at(to));
        assert_eq!(h.st.reorder.sessions(&h.world), original);
        let to = h.conversation_rect(BOB).center();
        h.drag_conversation(h.rect(entry_id(pinned)).center(), to, Harness::drop_at(to));
        assert_eq!(h.st.reorder.sessions(&h.world), original, "pinned entries cannot start a move");
    }
    let from = h.conversation_rect(GROUP2).center();
    let to = h.conversation_rect(GROUP).center() - egui::vec2(0.0, 5.0);
    h.drag_conversation(from, to, Harness::drop_at(to));
    assert!(h.conversation_rect(GROUP2).center().y < h.conversation_rect(GROUP).center().y);
}

#[test]
fn conversation_drag_pops_out_outside_cancels_with_escape_and_works_in_the_rail() {
    let mut h = Harness::new(ChatStyle::Compact);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(460.0, 500.0));
    let original = [JANE, BOB];
    let ids = |h: &Harness| h.st.reorder.sessions(&h.world).into_iter().map(|(id, _)| id).collect::<Vec<_>>();
    // dropped outside the window: in its own window there, the order kept
    let from = h.conversation_rect(JANE).center();
    let outside = egui::pos2(1000.0, 700.0);
    h.drag_conversation(from, outside, Harness::drop_at(outside));
    h.settle();
    assert_eq!(ids(&h), original);
    assert!(h.st.popped.contains(Some(JANE)), "the rail's entry popped out");
    h.st.popped.remove(Some(JANE));
    h.settle();
    let from = h.conversation_rect(JANE).center();
    let to = h.conversation_rect(BOB).center() + egui::vec2(0.0, 5.0);
    h.drag_conversation(
        from,
        to,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    h.frame(Harness::drop_at(to));
    assert_eq!(ids(&h), original);
    let from = h.conversation_rect(JANE).center();
    let to = h.conversation_rect(BOB).center() + egui::vec2(0.0, 5.0);
    h.drag_conversation(from, to, Harness::drop_at(to));
    assert_eq!(ids(&h), [BOB, JANE]);
}

#[test]
fn conversation_order_survives_filtering_messages_and_new_or_closed_sessions() {
    let mut h = Harness::new(ChatStyle::Aurora);
    let from = h.conversation_rect(JANE).center();
    let to = h.conversation_rect(BOB).center() + egui::vec2(0.0, 5.0);
    h.drag_conversation(from, to, Harness::drop_at(to));
    h.st.side_filter = "no matching conversation".into();
    h.settle();
    h.world.conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "another message"));
    const NEW: Uuid = Uuid::from_u128(0xD);
    h.world.conversation_line(NEW, line("New", NEW, ChatKind::Im, "new session"));
    h.st.side_filter.clear();
    h.settle();
    assert_eq!(
        h.st.reorder.sessions(&h.world).into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
        [BOB, JANE, NEW]
    );
    h.world.social.ims.retain(|s| s.other != BOB);
    h.st.forget(BOB);
    h.settle();
    h.world.conversation_line(BOB, line("Bob", BOB, ChatKind::Im, "reopened"));
    h.settle();
    assert_eq!(
        h.st.reorder.sessions(&h.world).into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
        [JANE, NEW, BOB]
    );
}

#[test]
fn conversation_drag_tolerates_small_click_movements_and_a_disappearing_source() {
    let mut h = Harness::new(ChatStyle::Aurora);
    let from = h.conversation_rect(JANE).center();
    let to = from + egui::vec2(2.0, 1.0);
    h.drag_conversation(from, to, Harness::drop_at(to));
    assert_eq!(h.st.selected, Some(JANE), "a small movement is still a click");
    assert_eq!(
        h.st.reorder.sessions(&h.world).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [JANE, BOB]
    );
    let from = h.conversation_rect(JANE).center();
    let to = h.conversation_rect(BOB).center() + egui::vec2(0.0, 5.0);
    h.drag_conversation(from, to, Vec::new());
    h.world.social.ims.retain(|s| s.other != JANE);
    h.frame(Vec::new());
    h.frame(Harness::drop_at(to));
    assert_eq!(h.st.reorder.sessions(&h.world).iter().map(|(id, _)| *id).collect::<Vec<_>>(), [BOB]);
}

#[test]
fn conversation_drag_scrolls_to_conversations_below_the_viewport() {
    let mut h = Harness::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(700.0, 400.0));
    for n in 0..20 {
        let id = Uuid::from_u128(0x100 + n);
        h.world.conversation_line(id, line("Other", id, ChatKind::Im, "test"));
    }
    let from = h.conversation_rect(JANE).center();
    let before = h.rect(entry_id(Pinned::Conversation(BOB))).top();
    let last_visible = |h: &Harness| {
        h.world
            .social
            .ims
            .iter()
            .filter_map(|s| {
                let r = h.frames.ctx.read_response(entry_id(Pinned::Conversation(s.other)))?.interact_rect;
                (r.height() > 3.0).then_some(r)
            })
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom()))
            .expect("a visible entry")
    };
    let last = last_visible(&h);
    let edge = egui::pos2(last.center().x, last.bottom() - 2.0);
    h.drag_conversation(from, edge, Vec::new());
    for _ in 0..90 {
        h.frame(Vec::new());
    }
    assert!(
        h.rect(entry_id(Pinned::Conversation(BOB))).top() < before - 100.0,
        "the list scrolled while held"
    );
    let target = last_visible(&h).center();
    h.frame(vec![egui::Event::PointerMoved(target)]);
    h.frame(Harness::drop_at(target));
    let order = h.st.reorder.sessions(&h.world);
    assert!(
        order.iter().position(|(id, _)| *id == JANE).unwrap() > 3,
        "the first conversation moved beyond the initial viewport"
    );
}

/// Dropped outside the Conversations window, an entry's conversation opens
/// in its own window where the entry was let go, its top left corner where
/// the ghost's was (#910), in the workspace and the tabs.
#[test]
fn an_entry_dropped_outside_opens_its_window_where_it_was_let_go() {
    for style in [ChatStyle::Aurora, ChatStyle::Legacy] {
        let mut h = Harness::new(style);
        h.world.social.names.insert(JANE, "Jane".into());
        h.world.social.names.insert(BOB, "Bob".into());
        crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(500.0, 40.0), egui::vec2(700.0, 500.0));
        h.settle();
        let jane = h.conversation_rect(JANE);
        let from = jane.center();
        let to = egui::pos2(160.0, 300.0);
        h.drag_conversation(from, to, Vec::new());
        let ghost = h
            .frames
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("conversation_drag_ghost")))
            .expect("the ghost is drawn while held");
        assert!(ghost.contains(to), "{style:?}: under the pointer");
        // where the entry was taken
        if style.workspace() {
            assert!((ghost.min - (to - (from - jane.min))).length() < 1.0, "{style:?}: {ghost:?}");
        }
        h.frame(Harness::drop_at(to));
        h.settle();
        assert!(h.st.popped.contains(Some(JANE)), "{style:?}");
        assert_eq!(
            h.st.reorder.sessions(&h.world).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [JANE, BOB],
            "{style:?}: the order is kept"
        );
        let window = h
            .frames
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(super::super::popout::floater_id(Some(JANE)).as_str())))
            .expect("its window is drawn");
        assert!(
            (window.min - ghost.min).length() < 2.0,
            "{style:?}: the window at {:?}, the ghost was at {:?}",
            window.min,
            ghost.min
        );
        assert!(h.actions.is_empty(), "{style:?}: no network action");
    }
}

/// A drop in the window but off the list (the messages) changes nothing.
#[test]
fn a_drop_in_the_window_off_the_list_does_nothing() {
    let mut h = Harness::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(300.0, 40.0), egui::vec2(800.0, 560.0));
    h.settle();
    let from = h.conversation_rect(JANE).center();
    let messages = egui::pos2(800.0, 300.0);
    h.drag_conversation(from, messages, Harness::drop_at(messages));
    h.settle();
    assert!(!h.st.popped.contains(Some(JANE)));
    assert_eq!(
        h.st.reorder.sessions(&h.world).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [JANE, BOB]
    );
}

/// While an entry is held over its new place, the entries move apart
/// there; Escape closes the gap and moves nothing.
#[test]
fn the_entries_move_apart_where_the_dragged_one_lands() {
    let mut h = Harness::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(300.0, 40.0), egui::vec2(800.0, 560.0));
    h.settle();
    let jane = h.conversation_rect(JANE);
    let bob = h.conversation_rect(BOB);
    // Bob taken up above Jane, held there
    h.drag_conversation(bob.center(), egui::pos2(jane.center().x, jane.top() + 6.0), Vec::new());
    for _ in 0..20 {
        h.frame(Vec::new());
    }
    let moved = h.rect(entry_id(Pinned::Conversation(JANE)));
    assert!(
        (moved.top() - bob.top()).abs() < 1.0,
        "Jane moved down by a whole entry, to Bob's place: {moved:?} from {jane:?}"
    );
    h.frame(key(egui::Key::Escape, egui::Modifiers::NONE));
    for _ in 0..20 {
        h.frame(Vec::new());
    }
    assert_eq!(h.rect(entry_id(Pinned::Conversation(JANE))).top(), jane.top(), "the gap closed");
    h.frame(Harness::drop_at(jane.center()));
    h.settle();
    assert_eq!(
        h.st.reorder.sessions(&h.world).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [JANE, BOB]
    );
}

/// The nearby chat drags out of the window into its own (#910); the chat
/// island's button then shows and hides that window alone.
#[test]
fn the_nearby_chat_drags_out_and_its_button_shows_and_hides_it() {
    for style in [ChatStyle::Aurora, ChatStyle::Legacy] {
        let mut h = Harness::new(style);
        crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(500.0, 40.0), egui::vec2(700.0, 500.0));
        h.settle();
        let local = if style.workspace() {
            h.rect(entry_id(Pinned::Local))
        } else {
            let texts = h.settle();
            painted(&texts, aurora_i18n::tr!("common-nearby-chat")).rect
        };
        let to = egui::pos2(160.0, 300.0);
        h.drag_conversation(local.center(), to, Harness::drop_at(to));
        h.settle();
        assert!(h.st.nearby_apart(), "{style:?}");
        assert!(h.st.nearby_window_shown(&h.frames.ctx), "{style:?}");
        let ctx = h.frames.ctx.clone();
        let in_sight = |h: &Harness| super::super::conversations_in_sight(&ctx, &h.st, false).contains(&None);
        assert!(in_sight(&h), "{style:?}");
        // put away: the window is not drawn, the nearby chat out of sight
        h.st.toggle_nearby_window(&ctx);
        h.settle();
        assert!(!h.st.nearby_window_shown(&ctx) && h.st.nearby_apart(), "{style:?}");
        assert!(!in_sight(&h), "{style:?}");
        let window = egui::Id::unique(super::super::popout::floater_id(None).as_str());
        let drawn = |h: &Harness| {
            h.frames
                .ctx
                .memory(|m| m.areas().visible_last_frame(&egui::LayerId::new(egui::Order::Middle, window)))
        };
        assert!(!drawn(&h), "{style:?}");
        // shown back
        h.st.toggle_nearby_window(&ctx);
        h.settle();
        assert!(h.st.nearby_window_shown(&ctx) && drawn(&h), "{style:?}");
        // the nearby chat cannot move among the conversations
        assert_eq!(
            h.st.reorder.sessions(&h.world).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [JANE, BOB],
            "{style:?}"
        );
    }
}

#[test]
fn conversation_reorder_demo_drags_out_of_the_window() {
    for (demo, popped) in [
        (super::super::ReorderDemo::Out, Some(JANE)),
        (super::super::ReorderDemo::Nearby, None),
    ] {
        let mut h = Harness::new(ChatStyle::Aurora);
        h.st.demo_reorder = Some(demo);
        crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(500.0, 60.0), egui::vec2(700.0, 560.0));
        h.settle();
        for frame in 700..=760 {
            let events = super::super::reorder::demo_events(&h.frames.ctx, frame);
            h.frame(events);
        }
        h.settle();
        assert!(h.st.popped.contains(popped), "{demo:?}");
    }
}

#[test]
fn conversation_reorder_demo_uses_the_current_window_position() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.st.demo_reorder = Some(super::super::ReorderDemo::Drop);
    h.settle();
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(100.0, 60.0), egui::vec2(800.0, 560.0));
    h.settle();
    for frame in 700..=760 {
        let events = super::super::reorder::demo_events(&h.frames.ctx, frame);
        h.frame(events);
    }
    h.settle();
    assert!(h.rect(entry_id(Pinned::Conversation(BOB))).top() < h.rect(entry_id(Pinned::Conversation(JANE))).top());
    assert_eq!(
        h.st.selected, None,
        "the synthetic drag does not select text or change conversations"
    );
}

/// An entry says who types before its last message.
#[test]
fn an_entry_says_who_types() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.world.social.set_typing(BOB, true);
    let texts = h.settle();
    let typing = aurora_i18n::tr!("ui-chat-sub-typing");
    assert!(texts.iter().any(|t| t.source == typing), "Bob's entry says he types");
    assert!(texts.iter().all(|t| t.source != "hello from Bob"));
}

/// In the window of tabs, the tab of a resident typing to us says so, and
/// says their name again once they stop.
#[test]
fn a_tab_says_who_types() {
    for style in [ChatStyle::Bubbles, ChatStyle::Legacy] {
        let mut h = Harness::new(style);
        let name = h.world.session_title(&BOB);
        let typing = super::super::typing::text(&[h.world.social.display_name_of(&BOB)]).expect("one name");
        h.world.social.set_typing(BOB, true);
        let texts = h.settle();
        assert!(texts.iter().any(|t| t.source == typing), "{style:?}: Bob's tab says he types");
        h.world.social.set_typing(BOB, false);
        let texts = h.settle();
        assert!(texts.iter().all(|t| t.source != typing), "{style:?}");
        assert!(texts.iter().any(|t| t.source == name), "{style:?}: Bob's tab names him again");
    }
}

/// The head of a resident's conversation offers them a teleport, and its ×
/// at the right edge closes the conversation; in the window of tabs too.
/// The nearby chat has no ×.
#[test]
fn the_head_offers_a_teleport_and_closes_the_conversation() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact, ChatStyle::Bubbles, ChatStyle::Legacy] {
        let mut h = Harness::new(style);
        crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(100.0, 60.0), egui::vec2(800.0, 560.0));
        h.settle();
        let id = |icon: &str| {
            if style.workspace() {
                super::super::sidebar::control_id(&format!("head-{icon}"))
            } else {
                super::super::classic_id(icon)
            }
        };
        assert!(h.frames.ctx.read_response(id("x")).is_none(), "{style:?}: no × for the nearby chat");
        h.st.open(BOB);
        h.settle();
        let tp = h.rect(id("send-filled"));
        h.click(tp.center());
        assert!(
            h.actions.iter().any(|a| matches!(a, ConvAction::OfferTeleport(id) if *id == BOB)),
            "{style:?}: a teleport offered to Bob"
        );
        let close = h.rect(id("x"));
        let window = h
            .frames
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("conversations")))
            .expect("window");
        assert!(close.right() > window.right() - 60.0, "{style:?}: the × at the right edge");
        h.click(close.center());
        assert!(
            h.actions.iter().any(|a| matches!(a, ConvAction::Close(id) if *id == BOB)),
            "{style:?}: the × closes Bob's conversation"
        );
    }
}

/// The window keeps its size while conversations open and close and long
/// messages come in, in every style, at the interface scales of the
/// options (the Resize of egui widened it by what its content measured, a
/// pixel or two at a time, and never gave it back).
#[test]
fn the_window_keeps_its_size_through_tabs_and_long_messages() {
    const NEW: Uuid = Uuid::from_u128(0xD);
    let styles = [ChatStyle::Aurora, ChatStyle::Compact, ChatStyle::Bubbles, ChatStyle::Legacy];
    let cases = styles.into_iter().flat_map(|s| {
        [1.0f32, 1.25, 1.5].into_iter().flat_map(move |z| {
            [egui::vec2(700.0, 440.0), egui::vec2(523.3, 301.7)]
                .into_iter()
                .map(move |size| (s, z, size))
        })
    });
    for (style, zoom, size) in cases {
        let mut h = Harness::new(style);
        crate::theme::Theme::default().apply(&h.frames.ctx, 1.0);
        crate::ui::fonts::apply(&h.frames.ctx, crate::ui::fonts::DEFAULT);
        h.frames.ctx.set_zoom_factor(zoom);
        crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(100.0, 60.0), size);
        h.settle();
        let case = format!("{style:?} at {zoom}, {size:?}");
        let win = |h: &Harness| {
            h.frames
                .ctx
                .memory(|m| m.area_rect(egui::Id::unique("conversations")))
                .expect("window")
        };
        let r0 = win(&h);
        assert!((r0.width() - size.x).abs() < 1.0, "{case}: placed at {r0:?}");
        let mut log = Vec::new();
        let mut step = |h: &mut Harness, what: &str| {
            for k in 0..6 {
                h.frame(Vec::new());
                let r = win(h);
                if r != r0 {
                    log.push(format!("{what}, frame {k}: {r:?}"));
                }
            }
        };
        h.st.open(JANE);
        step(&mut h, "select Jane");
        h.world.conversation_line(NEW, line("Newcomer", NEW, ChatKind::Im, "hi"));
        h.st.open(NEW);
        step(&mut h, "open new");
        h.world
            .conversation_line(NEW, line("Newcomer", NEW, ChatKind::Im, &"long words ".repeat(60)));
        step(&mut h, "long line");
        h.world
            .conversation_line(NEW, line("Newcomer", NEW, ChatKind::Im, &"x".repeat(400)));
        step(&mut h, "long word");
        h.world.conversation_closed(&NEW);
        h.world.leave_session(NEW);
        h.st.forget(NEW);
        h.st.selected = None;
        step(&mut h, "close new");
        h.world.conversation_closed(&JANE);
        h.world.leave_session(JANE);
        h.st.forget(JANE);
        step(&mut h, "close Jane");
        assert!(log.is_empty(), "{case}: placed at {r0:?}, then\n{}", log.join("\n"));
    }
}

/// A conference's members' panel: its moderators, then its participants,
/// opened and closed from the head.
#[test]
fn the_members_panel_lists_a_conference() {
    const MEETING: Uuid = Uuid::from_u128(0xC0);
    let mut h = Harness::new(ChatStyle::Aurora);
    let mut info = crate::world::groups::SessionInfo::default();
    info.name = "Meeting".into();
    info.participants = [JANE, BOB].into_iter().collect();
    info.moderators = [JANE].into_iter().collect();
    h.world.groups.sessions.insert(MEETING, info);
    h.world.conversation_line(MEETING, line("Bob", BOB, ChatKind::Im, "hi all"));
    h.st.selected = Some(MEETING);
    h.st.members_open = true;
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 560.0));
    let texts = h.settle();
    let title = |name: &str| format!("{} — ", name.to_uppercase());
    let moderators = title(aurora_i18n::tr!("ui-chat-members-moderators"));
    let participants = title(aurora_i18n::tr!("ui-chat-members-participants"));
    let m = texts.iter().find(|t| t.source.starts_with(&moderators)).expect("the moderators");
    let p = texts
        .iter()
        .find(|t| t.source.starts_with(&participants))
        .expect("the participants");
    assert!(m.rect.top() < p.rect.top());
    assert_eq!(super::super::sidebar::kind(&h.world, &MEETING), Kind::Conference);
    h.st.members_open = false;
    let texts = h.settle();
    assert!(texts.iter().all(|t| !t.source.starts_with(&moderators)), "closed");
}

/// The style chosen in Preferences › Aurora switches the window at once,
/// over a demo scenario's style too (the human's report of 2026-10-15:
/// the choice did nothing while `AURORA_DEMO_CONV=aurora` held its own).
#[test]
fn the_style_of_the_preferences_switches_the_window_live() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.st.demo_style = Some(ChatStyle::Aurora);
    let workspace_shown = |h: &Harness| h.frames.ctx.read_response(entry_id(Pinned::Local)).is_some();
    h.st.set_style(ChatStyle::Aurora);
    h.settle();
    assert!(workspace_shown(&h));
    for style in [
        ChatStyle::Bubbles,
        ChatStyle::Legacy,
        ChatStyle::Compact,
        ChatStyle::Aurora,
        ChatStyle::Legacy,
    ] {
        h.st.set_style(style);
        h.settle();
        assert_eq!(workspace_shown(&h), style.workspace(), "{style:?}");
    }
}

/// Switching to Lines must retain live media and the same consent cards,
/// in the Conversations window and in an independent conversation (#976).
#[test]
fn link_previews_survive_style_switches_and_detached_conversations() {
    for popped in [false, true] {
        let mut h = Harness::new(ChatStyle::Aurora);
        h.st.selected = Some(JANE);
        h.st.previews.demo = true;
        h.st.previews.settings.size = crate::link_preview::Size::Small;
        h.world
            .conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "https://i.imgur.com/demo.gif"));
        if popped {
            h.st.popped.pop_out(Some(JANE), false, false, false);
            h.open = false;
        }
        for style in [
            ChatStyle::Aurora,
            ChatStyle::Compact,
            ChatStyle::Bubbles,
            ChatStyle::Legacy,
            ChatStyle::Aurora,
        ] {
            h.st.set_style(style);
            let texts = h.settle();
            assert!(
                texts.iter().any(|t| t.source == "GIF"),
                "{style:?}, popped={popped}: the animated picture is drawn"
            );
            assert!(
                texts
                    .iter()
                    .all(|t| t.source != "https://i.imgur.com/demo.gif" || t.rect.right() <= painted(&texts, "GIF").rect.left()),
                "{style:?}, popped={popped}: the ready GIF replaces its raw link in the thread (#933); the sidebar keeps its message summary"
            );
        }
        h.st.set_style(ChatStyle::Legacy);
        h.world
            .conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "https://example.com/picture.png"));
        let texts = h.settle();
        assert!(
            texts.iter().any(|t| t.source == aurora_i18n::tr!("ui-chat-preview-not-loaded")),
            "popped={popped}: an unknown image site still waits for consent"
        );
    }
}

/// Enabling cards in Lines must preserve the opt-out and never fetch links
/// restored from transcripts.
#[test]
fn legacy_link_previews_respect_privacy_and_transcript_history() {
    for (origin, enabled) in [(Origin::Live, false), (Origin::Log, true), (Origin::ServerLog, true)] {
        let mut h = Harness::new(ChatStyle::Legacy);
        h.st.selected = Some(JANE);
        h.st.previews.demo = true;
        h.st.previews.settings.ims = enabled;
        let mut message = line("Jane", JANE, ChatKind::Im, "https://example.com/picture.png");
        message.origin = origin;
        h.world.conversation_line(JANE, message);
        let texts = h.settle();
        assert!(texts.iter().any(|t| t.source == "https://example.com/picture.png"));
        assert!(!texts.iter().any(|t| t.source == aurora_i18n::tr!("ui-chat-preview-not-loaded")));
    }
}

/// A conference lists who is not in it: those it started with who never
/// joined and those who left; a role group's chat lists only who is in it.
#[test]
fn a_conference_lists_its_offline_members() {
    const MEETING: Uuid = Uuid::from_u128(0xC0);
    const LEFT: Uuid = Uuid::from_u128(0xC1);
    const NEVER: Uuid = Uuid::from_u128(0xC2);
    let mut h = Harness::new(ChatStyle::Aurora);
    let mut info = crate::world::groups::SessionInfo::default();
    info.name = "Meeting".into();
    info.participants = [JANE].into_iter().collect();
    info.left = [LEFT].into_iter().collect();
    h.world.groups.sessions.insert(MEETING, info);
    h.world.calls.conferences.insert(MEETING, vec![JANE, NEVER]);
    h.world.conversation_line(MEETING, line("Jane", JANE, ChatKind::Im, "hi all"));
    h.st.selected = Some(MEETING);
    h.st.members_open = true;
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 560.0));
    let texts = h.settle();
    let offline = format!("{} — 2", aurora_i18n::tr!("ui-chat-members-offline").to_uppercase());
    assert!(
        texts.iter().any(|t| t.source == offline),
        "the one who left and the one who never came"
    );
    let note = aurora_i18n::tr!("ui-chat-members-present-only");
    assert!(texts.iter().all(|t| t.source != note), "a conference is not a role group");
}

/// The column draws only the sections that hold a conversation: with IMs
/// alone, no "Groups" nor "Conferences" head (the human's choice of
/// 2026-10-16); conversations start from the one button beside the search
/// field.
#[test]
fn empty_sections_are_not_drawn() {
    let mut h = Harness::new(ChatStyle::Aurora);
    let texts = h.settle();
    let head = |key: &'static str| aurora_i18n::text(key).to_uppercase();
    assert!(texts.iter().any(|t| t.source == head("ui-chat-section-private")), "the IMs' head");
    for key in ["ui-chat-section-groups", "ui-chat-section-conferences"] {
        assert!(texts.iter().all(|t| t.source != head(key)), "{key} drawn while empty");
    }
    let new = super::super::sidebar::control_id("new-conversation");
    assert!(h.frames.ctx.read_response(new).is_some(), "the new conversation button");
}

/// A threshold crossed back needs a margin past it: a window resized
/// around 600 points (or around the room of the members' panel) switches
/// its layout once each way (the human's report of 2026-10-16).
#[test]
fn layout_thresholds_have_a_margin() {
    use super::{RAIL_BELOW, members_room_now, rail_now};
    assert!(rail_now(None, RAIL_BELOW - 1.0));
    assert!(!rail_now(None, RAIL_BELOW + 1.0));
    assert!(rail_now(Some(true), RAIL_BELOW + 20.0), "a rail stays one past 600");
    assert!(!rail_now(Some(true), RAIL_BELOW + 41.0));
    assert!(!rail_now(Some(false), RAIL_BELOW + 1.0));
    let room = super::members::WIDTH + super::MESSAGES_MIN;
    assert!(members_room_now(None, room));
    assert!(!members_room_now(None, room - 1.0));
    assert!(members_room_now(Some(true), room - 20.0), "the panel stays a little under its room");
    assert!(!members_room_now(Some(true), room - 41.0));
}

/// The window narrowed past 600 points: the column eases to the rail over
/// a few frames instead of jumping in one.
#[test]
fn the_column_eases_to_the_rail() {
    let mut h = Harness::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(700.0, 560.0));
    h.settle();
    let wide = h.rect(entry_id(Pinned::Local)).width();
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(560.0, 560.0));
    let mut widths = Vec::new();
    for _ in 0..30 {
        h.frame(Vec::new());
        widths.push(h.rect(entry_id(Pinned::Local)).width());
    }
    let narrow = *widths.last().expect("frames");
    let between = widths.iter().filter(|w| **w < wide - 1.0 && **w > narrow + 1.0).count();
    assert!(narrow < wide && between >= 3, "{wide} → {widths:?}");
}

/// Finding in a conversation opens the field in the head's row: no strip
/// pushes the messages (the box keeps its place), and Escape gives the head
/// back (the human's request of 2026-10-16).
#[test]
fn the_find_field_opens_in_the_head_row() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.st.selected = Some(JANE);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 560.0));
    h.settle();
    let input = egui::Id::unique(("conv_input", Some(JANE)));
    let box_before = h.rect(input);
    h.st.searches.entry(Some(JANE)).or_default().open_find();
    h.settle();
    let field = h.rect(super::super::find::field_id(Some(JANE)));
    assert!(
        field.bottom() < box_before.top() - 200.0,
        "the field is up in the head, not over the box: {field:?}"
    );
    assert!(
        field.top() < 20.0 + 32.0 + super::super::head::HEIGHT,
        "in the head's row: {field:?}"
    );
    assert_eq!(h.rect(input), box_before, "the messages and the box did not move");
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    h.frame(vec![escape]);
    h.settle();
    assert!(h.st.searches.get(&Some(JANE)).is_some_and(|s| s.find.is_none()), "Escape closed it");
}

/// Thirty friends, all online, named "Friend 00" to "Friend 29".
fn add_friends(world: &mut World) {
    for i in 0..30u128 {
        let id = Uuid::from_u128(0xF000 + i);
        world.social.friends.push(crate::world::social::Friend {
            id,
            online: true,
            rights_given: 1,
            rights_has: 1,
        });
        world.social.names.insert(id, format!("Friend {i:02}"));
    }
}

fn contacts_navigation(style: ChatStyle, width: f32) -> Harness {
    let mut h = Harness::new(style);
    add_friends(&mut h.world);
    h.show_contacts = true;
    h.friends_pane = true;
    h.st.contacts = true;
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(width, 560.0));
    h
}

#[test]
fn contacts_workspace_search_stays_available_with_old_settings() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = contacts_navigation(style, 700.0);
        h.contacts_settings.show_search = false;
        h.people.filter = "Friend 03".into();
        let texts = h.settle();
        assert!(texts.iter().any(|t| t.source == "Friend 03"));
        assert!(!texts.iter().any(|t| t.source == "Friend 05"));
        assert!(h.actions.is_empty(), "filtering never starts an action");
    }
}

/// Hover actions deliberately cover names; the avatar remains a row click target.
fn contact_avatar(texts: &[crate::ui::widgets::elide_tests::Text], name: &str) -> egui::Pos2 {
    painted(texts, name).rect.left_center() - egui::vec2(25.0, 0.0)
}

/// A single click must leave the row available for the second click even
/// when the friend's sheet will replace the list in a reduced layout.
#[test]
fn contacts_navigation_double_click_opens_im_at_every_width() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for width in [960.0, 700.0, 460.0] {
            for waiting_frames in [0, 10] {
                let mut h = contacts_navigation(style, width);
                let texts = h.settle();
                let at = contact_avatar(&texts, "Friend 00");
                h.click(at);
                assert!(h.actions.is_empty(), "a single click does not start an IM");
                h.settle();
                for _ in 0..waiting_frames {
                    h.frame(Vec::new());
                }
                h.click(at);
                let ims = h
                    .actions
                    .iter()
                    .filter(
                        |a| matches!(a, ConvAction::People(crate::ui::people::PeopleAction::OpenIm(id)) if *id == Uuid::from_u128(0xF000)),
                    )
                    .count();
                assert_eq!(ims, 1, "{style:?} at {width}: double-click starts exactly one IM");
                if width != 960.0 {
                    for _ in 0..30 {
                        h.frame(Vec::new());
                    }
                    assert_eq!(h.people.sheet, None, "a double click cannot show a sheet after the IM");
                }
            }
        }
    }
}

/// Contacts returns from a sheet to the list, with the same state as its
/// close button, both in the column and in the rail of pictures.
#[test]
fn contacts_navigation_entry_returns_from_sheet_to_list() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for width in [960.0, 700.0, 460.0] {
            let mut h = contacts_navigation(style, width);
            let texts = h.settle();
            h.people.filter = "Friend 0".into();
            h.hold(contact_avatar(&texts, "Friend 00"));
            for _ in 0..30 {
                h.frame(Vec::new());
            }
            assert_eq!(h.people.sheet, Some(Uuid::from_u128(0xF000)), "a long press opens the sheet");
            assert!(h.actions.is_empty(), "releasing the hold cannot also start an IM");
            let selected = h.people.sel.clone();
            let texts = h.settle();
            assert_eq!(
                texts.iter().any(|t| t.source == "Friend 01"),
                width == 960.0,
                "{width}: reduced layouts show only the sheet"
            );
            h.click(h.rect(entry_id(Pinned::Contacts)).center());
            let texts = h.settle();
            assert_eq!(
                h.people.sheet,
                (width == 960.0).then_some(Uuid::from_u128(0xF000)),
                "{style:?} at {width}: Contacts returns to the list, retaining the permanent large-layout sheet"
            );
            assert!(texts.iter().any(|t| t.source == "Friend 01"), "the list is back");
            assert_eq!(h.people.filter, "Friend 0", "the filter is preserved");
            assert_eq!(h.people.sel, selected, "the selection is preserved");
            assert!(h.st.contacts);
            let at = contact_avatar(&texts, "Friend 00");
            h.click(at);
            h.click(at);
            assert!(
                h.actions.iter().any(
                    |a| matches!(a, ConvAction::People(crate::ui::people::PeopleAction::OpenIm(id)) if *id == Uuid::from_u128(0xF000))
                ),
                "{style:?} at {width}: double-click still works immediately after returning to Contacts"
            );
        }
    }
}

#[test]
fn contacts_navigation_single_click_opens_sheet_only_at_large_width() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for width in [960.0, 860.0, 700.0, 460.0, 394.0] {
            let mut h = contacts_navigation(style, width);
            let texts = h.settle();
            h.click(contact_avatar(&texts, "Friend 00"));
            for _ in 0..45 {
                h.frame(Vec::new());
            }
            assert_eq!(
                h.people.sheet,
                (width >= 860.0).then_some(Uuid::from_u128(0xF000)),
                "{style:?} at {width}: a row click opens the sheet only when the list stays beside it"
            );
            assert_eq!(h.people.sel, vec![Uuid::from_u128(0xF000)], "the contact is selected");
            assert!(h.actions.is_empty());
            let texts = h.settle();
            assert!(texts.iter().any(|t| t.source == "Friend 01"), "the list stays visible");
            if width >= 860.0 {
                h.click(contact_avatar(&texts, "Friend 01"));
                assert_eq!(
                    h.people.sheet,
                    Some(Uuid::from_u128(0xF001)),
                    "clicking another row updates the open sheet"
                );
                assert!(h.actions.is_empty(), "switching sheets does not start an IM");
            }
        }
    }
}

/// The explicit contact-details action stays immediately before More,
/// including reduced layouts where permissions would crowd it out.
#[test]
fn contacts_navigation_details_button_opens_sheet() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for width in [960.0, 860.0, 700.0, 460.0, 394.0] {
            for online in [false, true] {
                for permissions in [false, true] {
                    let mut h = contacts_navigation(style, width);
                    let id = Uuid::from_u128(0xF000);
                    for friend in &mut h.world.social.friends {
                        friend.online = online;
                    }
                    h.contacts_settings.column_permissions = permissions;
                    let texts = h.settle();
                    let name = contact_avatar(&texts, "Friend 00");
                    h.frame(vec![egui::Event::PointerMoved(name)]);
                    h.settle();
                    let hovered = h.frames.ctx.interaction_snapshot(|i| i.hovered.clone());
                    let row = hovered
                        .iter()
                        .filter_map(|id| h.frames.ctx.read_response(*id))
                        .filter(|r| r.sense.senses_drag() && r.rect.contains(name))
                        .min_by(|a, b| a.rect.width().total_cmp(&b.rect.width()))
                        .expect("the hovered friend row");
                    let mut buttons: Vec<_> = h.frames.ctx.viewport(|v| {
                        v.this_pass
                            .widgets
                            .get_layer(row.layer_id)
                            .filter(|w| {
                                w.sense.senses_click()
                                    && w.rect.height() == 24.0
                                    && w.rect.width() <= 26.0
                                    && row.rect.contains_rect(w.rect)
                            })
                            .map(|w| w.rect)
                            .collect()
                    });
                    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
                    assert_eq!(buttons.len(), if online { 5 } else { 4 }, "all quick actions remain visible");
                    assert_eq!(row.rect.height(), 42.0, "contact rows keep their height at every window width");
                    assert!(buttons.iter().all(|button| button.center().y == row.rect.center().y));
                    let details = buttons[buttons.len() - 2].center();
                    h.click(details);
                    assert_eq!(
                        h.people.sheet,
                        Some(id),
                        "{style:?} at {width}, online={online}, permissions={permissions}: the button before More opens the inline sheet"
                    );
                    assert_eq!(h.people.sel, vec![id]);
                    assert!(h.actions.is_empty(), "details does not open a profile, IM or teleport offer");
                    h.settle();
                    h.click(h.rect(entry_id(Pinned::Contacts)).center());
                    let texts = h.settle();
                    assert_eq!(h.people.sheet, (width >= 860.0).then_some(id));
                    assert!(texts.iter().any(|t| t.source == "Friend 01"), "Contacts returns to the list");
                }
            }
        }
    }
}

#[test]
fn contacts_navigation_large_details_preserve_selection_and_show_empty_state() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = contacts_navigation(style, 960.0);
        h.settle();
        assert_eq!(
            h.people.sheet,
            Some(Uuid::from_u128(0xF000)),
            "the first visible friend is selected automatically"
        );
        assert_eq!(h.people.sel, vec![Uuid::from_u128(0xF000)]);
        h.people.close_sheet();
        h.people.sel = vec![Uuid::from_u128(0xF005)];
        h.settle();
        assert_eq!(
            h.people.sheet,
            Some(Uuid::from_u128(0xF005)),
            "an existing selection wins over the first row"
        );
        h.people.filter = "Friend 12".into();
        h.settle();
        assert_eq!(
            h.people.sheet,
            Some(Uuid::from_u128(0xF00C)),
            "the first matching friend replaces a filtered-out selection"
        );
        h.people.filter = "no matching contact".into();
        let texts = h.settle();
        assert_eq!(h.people.sheet, None);
        assert!(h.people.sel.is_empty());
        assert!(texts.iter().any(|t| t.source == aurora_i18n::tr!("people-contact-details-empty")));
        h.people.filter.clear();
        h.settle();
        assert_eq!(
            h.people.sheet,
            Some(Uuid::from_u128(0xF000)),
            "clearing the filter restores the detail panel"
        );
        h.world.social.friends.clear();
        let texts = h.settle();
        assert!(
            texts.iter().any(|t| t.source == aurora_i18n::tr!("people-contact-details-empty")),
            "an empty friend list keeps a placeholder"
        );
        assert!(h.actions.is_empty(), "automatic selection never starts a conversation");
    }
}

#[test]
fn contacts_navigation_resizing_keeps_the_list_and_selection() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = contacts_navigation(style, 960.0);
        let selected = Uuid::from_u128(0xF005);
        h.people.sel = vec![selected];
        h.settle();
        for width in [
            960.0, 880.0, 860.0, 850.0, 840.0, 820.0, 700.0, 640.0, 620.0, 600.0, 580.0, 460.0, 580.0, 600.0, 620.0, 640.0, 700.0, 820.0,
            840.0, 860.0, 880.0, 960.0,
        ] {
            crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(width, 560.0));
            for frame in 0..18 {
                let texts = h.frame(Vec::new());
                assert!(
                    texts.iter().any(|t| t.source == "Friend 00"),
                    "{style:?} at {width}, frame {frame}: the list stays visible through each threshold"
                );
                assert_eq!(h.people.sel, vec![selected], "resizing preserves the chosen friend");
            }
            if width >= 880.0 {
                assert_eq!(h.people.sheet, Some(selected), "the large layout always shows the selected friend");
            } else if width <= 820.0 {
                assert_eq!(h.people.sheet, None, "the reduced layout returns to the list");
            }
        }
    }
}

#[test]
fn contacts_navigation_narrow_sort_menu_keeps_all_choices() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = contacts_navigation(style, 400.0);
        let texts = h.settle();
        h.click(painted(&texts, aurora_i18n::tr!("people-sort-online-first")).rect.center());
        let texts = h.settle();
        let old = h.friends.sort;
        h.click(painted(&texts, aurora_i18n::tr!("common-username")).rect.center());
        assert_ne!(h.friends.sort, old, "the collapsed sort menu still changes the sort");
    }
}

/// The contact list shown in the workspace's Contacts tab fills the pane:
/// inside a row of the layout it took one line's height and showed three
/// friends of thirty (the human's report of 2026-10-16).
#[test]
fn the_contact_list_fills_the_contacts_pane() {
    let mut h = Harness::new(ChatStyle::Aurora);
    add_friends(&mut h.world);
    h.show_contacts = true;
    h.st.contacts = true;
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 560.0));
    let texts = h.settle();
    let rows: Vec<&crate::ui::widgets::elide_tests::Text> = texts.iter().filter(|t| t.source.starts_with("Friend ")).collect();
    assert!(rows.len() >= 15, "{} friends shown", rows.len());
    let lowest = rows.iter().map(|t| t.rect.bottom()).fold(f32::MIN, f32::max);
    assert!(lowest > 20.0 + 560.0 - 120.0, "the list ends at {lowest}");
}

/// In Aurora's look of People the Contacts tab is People's friends tab:
/// its rows, and the column's entry counting the friends online.
#[test]
fn the_contacts_tab_shows_peoples_friends() {
    let mut h = Harness::new(ChatStyle::Aurora);
    add_friends(&mut h.world);
    h.st.contacts = true;
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 560.0));
    let p = crate::theme::Theme::default().palette();
    let radar = crate::world::radar::Radar::default();
    let mut radar_settings = crate::ui::people::RadarSettings::default();
    let (voice, pics) = (HashMap::new(), HashMap::new());
    let data = crate::ui::people::RadarData {
        radar: &radar,
        settings: &mut radar_settings,
        voice: &voice,
        draw_distance: 64.0,
        now: 0.0,
        pics: &pics,
    };
    let mut people = crate::ui::people::PeopleUi::default();
    let mut friends = crate::ui::people::FriendsSettings::default();
    let mut texts = Vec::new();
    for _ in 0..3 {
        let mut open = true;
        let Harness {
            frames,
            world,
            st,
            icons,
            emoji,
            contacts,
            contacts_settings,
            gestures,
            actions,
            ..
        } = &mut h;
        texts = frames.run(Vec::new(), |ui| {
            actions.extend(show(
                ui.ctx(),
                &p,
                icons,
                emoji,
                &pics,
                &pics,
                world,
                gestures,
                st,
                contacts,
                contacts_settings,
                true,
                Some(crate::ui::people::FriendsPane {
                    st: &mut people,
                    data: &data,
                    settings: &mut friends,
                    images: &pics,
                }),
                &mut open,
            ));
        });
    }
    assert!(texts.iter().any(|t| t.source == "Friend 00"), "the friends' rows");
    let sub = aurora_i18n::tr!("ui-chat-contacts-friends-sub", online = 30);
    assert!(texts.iter().any(|t| t.source == sub), "the column says {sub:?}");
    // the contact list's own tabs are not there
    let groups = aurora_i18n::tr!("ui-contacts-contact-sets");
    assert!(texts.iter().all(|t| t.source != groups), "no contact list tabs");
}

/// Who types in the nearby chat shows above the field, their name in the
/// ink (a layout that once took the fonts' lock twice and froze the
/// viewer at its first frame: this frame must end).
#[test]
fn the_field_says_who_types_nearby() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.st.nearby_typing = vec![BOB];
    h.st.nearby_count = 1;
    let texts = h.settle();
    let name = h.world.social.display_name_of(&BOB);
    assert!(
        texts.iter().any(|t| t.source.contains(&name) && t.source != name),
        "the typing line names {name}"
    );
}

/// The "New" line: away from a conversation, two lines arrive; back, it is
/// above them; what we write then, and lines arriving under our eyes, do
/// not move it; away and back with nothing new, it is gone. The window
/// without the focus counts as away; each conversation keeps its own.
#[test]
fn the_new_line_marks_what_arrived_while_away() {
    use std::collections::VecDeque;
    let mut lines: VecDeque<ChatLine> = (0..3).map(|k| line("Jane", JANE, ChatKind::Im, &format!("old {k}"))).collect();
    let mut nl = super::NewLines::default();
    let (jane, bob) = (Some(JANE), Some(BOB));
    // first sight, nothing unread: no line
    nl.frame(jane, 1, true, 10, &lines, 0);
    assert_eq!(nl.at(jane), None);
    // under our eyes: no line either
    lines.push_back(line("Jane", JANE, ChatKind::Im, "while looking"));
    nl.frame(jane, 2, true, 10, &lines, 1);
    assert_eq!(nl.at(jane), None, "a line arriving in sight");
    // Bob's conversation shown (frames 3-5), two lines arrive meanwhile
    nl.frame(bob, 3, true, 0, &VecDeque::new(), 0);
    lines.push_back(line("Jane", JANE, ChatKind::Im, "away 1"));
    lines.push_back(line("Jane", JANE, ChatKind::Im, "away 2"));
    nl.frame(jane, 6, true, 10, &lines, 2);
    assert_eq!(nl.at(jane), Some(14), "above the first line that arrived away");
    // we answer, Jane answers, under our eyes: it stays
    lines.push_back(line("Me", Uuid::nil(), ChatKind::OWN, "mine"));
    nl.frame(jane, 7, true, 10, &lines, 0);
    lines.push_back(line("Jane", JANE, ChatKind::Im, "reply"));
    nl.frame(jane, 8, true, 10, &lines, 1);
    assert_eq!(nl.at(jane), Some(14), "what we write never moves it");
    // the window closed (no frame 9-19) then reopened, nothing new: gone
    nl.frame(jane, 20, true, 10, &lines, 0);
    assert_eq!(nl.at(jane), None, "back with nothing new");
    // the viewer loses the focus, a line arrives, the focus comes back
    nl.frame(jane, 21, false, 10, &lines, 0);
    lines.push_back(line("Jane", JANE, ChatKind::Im, "while unfocused"));
    nl.frame(jane, 22, false, 10, &lines, 1);
    nl.frame(jane, 23, true, 10, &lines, 0);
    assert_eq!(nl.at(jane), Some(18), "the focus away counts as away");
    // only our own lines arrived while away (written from elsewhere): none
    nl.frame(bob, 24, true, 0, &VecDeque::new(), 0);
    lines.push_back(line("Me", Uuid::nil(), ChatKind::OWN, "from elsewhere"));
    nl.frame(jane, 25, true, 10, &lines, 0);
    assert_eq!(nl.at(jane), None, "no line above our own");
    assert_eq!(nl.at(bob), None, "each conversation its own");
}

/// The first sight of a conversation since the login: the lines unread
/// then are under the "New" line.
#[test]
fn the_new_line_on_first_sight_is_above_the_unread_lines() {
    use std::collections::VecDeque;
    let lines: VecDeque<ChatLine> = (0..5).map(|k| line("Jane", JANE, ChatKind::Im, &format!("l{k}"))).collect();
    let mut nl = super::NewLines::default();
    nl.frame(None, 1, true, 100, &lines, 2);
    assert_eq!(nl.at(None), Some(103));
}

/// Shown while the viewer has not the focus (just logged in, another
/// application in front): the unread lines keep their "New" line, and the
/// focus coming back does not take it away.
#[test]
fn the_new_line_waits_for_the_focus() {
    use std::collections::VecDeque;
    let lines: VecDeque<ChatLine> = (0..5).map(|k| line("Jane", JANE, ChatKind::Im, &format!("l{k}"))).collect();
    let mut nl = super::NewLines::default();
    nl.frame(None, 1, false, 0, &lines, 2);
    assert_eq!(nl.at(None), Some(3));
    nl.frame(None, 2, false, 0, &lines, 0);
    nl.frame(None, 3, true, 0, &lines, 0);
    assert_eq!(nl.at(None), Some(3), "still unseen when the focus comes");
    nl.frame(None, 4, true, 0, &lines, 0);
    assert_eq!(nl.at(None), Some(3));
}

#[test]
fn the_menu_boundary_ignores_our_replies_and_survives_focus_changes() {
    use std::collections::VecDeque;
    let mut lines = VecDeque::from([line("Jane", JANE, ChatKind::Im, "already seen")]);
    let mut nl = super::NewLines::default();
    nl.frame(Some(JANE), 1, true, 100, &lines, 0);
    nl.pause_at(Some(JANE), 101);
    lines.push_back(line("Me", Uuid::nil(), ChatKind::OWN, "our reply"));
    nl.frame(Some(JANE), 2, false, 100, &lines, 0);
    assert_eq!(nl.at(Some(JANE)), Some(102));
    lines.push_back(line("Jane", JANE, ChatKind::Im, "arrived during the menu"));
    nl.frame(Some(JANE), 3, false, 100, &lines, 1);
    assert_eq!(nl.at(Some(JANE)), Some(102));
    nl.frame(Some(JANE), 4, true, 100, &lines, 0);
    assert_eq!(
        nl.at(Some(JANE)),
        Some(102),
        "closing/refocusing preserves the first unread arrival"
    );
}

/// The window's right edge dragged quickly in past the rail's threshold and
/// back out: the column takes its width at once on every frame (never
/// between the two, which kept the content wider than the window), and the
/// window follows the pointer all the way (the human's report of
/// 2026-10-16: it "shrinks a bit" and catches up).
#[test]
fn a_dragged_edge_is_followed_at_once() {
    let mut h = Harness::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(700.0, 560.0));
    h.settle();
    let wide = h.rect(entry_id(Pinned::Local)).width();
    let window = |h: &Harness| {
        h.frames
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("conversations")))
            .expect("the window")
    };
    let start = window(&h);
    let grab = egui::pos2(start.right() - 1.0, start.center().y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    h.frame(vec![egui::Event::PointerMoved(grab)]);
    h.frame(vec![button(grab, true)]);
    let mut widths = Vec::new();
    let mut gaps = Vec::new();
    for dx in [
        -40.0, -80.0, -120.0, -160.0, -200.0, -160.0, -120.0, -80.0, -40.0, 0.0, -100.0, -200.0,
    ] {
        let at = grab + egui::vec2(dx, 0.0);
        h.frame(vec![egui::Event::PointerMoved(at)]);
        widths.push(h.rect(entry_id(Pinned::Local)).width());
        gaps.push(window(&h).right() - at.x);
    }
    h.frame(vec![button(grab + egui::vec2(-200.0, 0.0), false)]);
    let narrow = widths.iter().copied().fold(f32::MAX, f32::min);
    assert!(narrow < wide - 50.0, "the rail was reached: {widths:?}");
    assert!(
        widths.iter().all(|w| (w - wide).abs() < 1.0 || (w - narrow).abs() < 1.0),
        "no column between its widths during the drag: {widths:?}"
    );
    assert!(
        gaps.iter().all(|g| (g - gaps[0]).abs() < 1.5),
        "the window follows the pointer: {gaps:?}"
    );
}

/// Each edge and a corner of the window dragged quickly in and out, past
/// its minimum: the edges that are not dragged never move, on any frame
/// (the human's report of 2026-10-16: the opposite edge jumped for a frame
/// then came back).
#[test]
fn the_fixed_edges_stay_while_one_is_dragged_fast() {
    // (grab point from the window's rect, direction of the drag, which
    // sides move: left, right, top, bottom)
    type Grab = fn(egui::Rect) -> egui::Pos2;
    let cases: [(&str, Grab, egui::Vec2, [bool; 4]); 5] = [
        (
            "right",
            |r| egui::pos2(r.right() - 1.0, r.center().y),
            egui::vec2(-1.0, 0.0),
            [false, true, false, false],
        ),
        (
            "left",
            |r| egui::pos2(r.left() + 1.0, r.center().y),
            egui::vec2(1.0, 0.0),
            [true, false, false, false],
        ),
        (
            "bottom",
            |r| egui::pos2(r.center().x, r.bottom() - 1.0),
            egui::vec2(0.0, -1.0),
            [false, false, false, true],
        ),
        (
            "top",
            |r| egui::pos2(r.center().x, r.top() + 1.0),
            egui::vec2(0.0, 1.0),
            [false, false, true, false],
        ),
        (
            "corner",
            |r| r.right_bottom() - egui::vec2(6.0, 6.0),
            egui::vec2(-1.0, -1.0),
            [false, true, false, true],
        ),
    ];
    for (name, grab_of, dir, moves) in cases {
        for style in [ChatStyle::Aurora, ChatStyle::Legacy] {
            let mut h = Harness::new(style);
            h.st.selected = Some(JANE);
            crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(200.0, 150.0), egui::vec2(800.0, 560.0));
            h.settle();
            let window = |h: &Harness| {
                h.frames
                    .ctx
                    .memory(|m| m.area_rect(egui::Id::unique("conversations")))
                    .expect("the window")
            };
            let start = window(&h);
            let grab = grab_of(start);
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            h.frame(vec![egui::Event::PointerMoved(grab)]);
            h.frame(vec![button(grab, true)]);
            let mut rects = Vec::new();
            // quick flicks: 150 points a frame, in past the minimum and out
            for d in [150.0, 300.0, 450.0, 600.0, 450.0, 150.0, -100.0, 200.0, 500.0, 0.0, 600.0, 0.0] {
                h.frame(vec![egui::Event::PointerMoved(grab + dir * d)]);
                rects.push(window(&h));
            }
            h.frame(vec![button(grab, false)]);
            let sides = |r: egui::Rect| [r.left(), r.right(), r.top(), r.bottom()];
            let fixed = sides(start);
            for (k, r) in rects.iter().enumerate() {
                for (s, (&now, &was)) in sides(*r).iter().zip(fixed.iter()).enumerate() {
                    assert!(
                        moves[s] || (now - was).abs() < 0.5,
                        "{name} {style:?}, frame {k}: side {s} moved from {was} to {now} ({rects:?})"
                    );
                }
            }
        }
    }
}

/// The window's height dragged, slowly and quickly, by its bottom edge
/// then by its top edge, in a short conversation and in one longer than the
/// window: the messages keep their place against the field (bottom edge:
/// they slide with it) and on the screen (top edge), on every frame; the
/// Legacy style's short conversation, standing at the top, against the
/// window's top (the human's report of 2026-10-16: the lines "hop" while
/// the window moves).
#[test]
fn the_messages_stay_put_while_the_height_is_dragged() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact, ChatStyle::Legacy] {
        for long in [false, true] {
            let mut h = Harness::new(style);
            if long {
                for k in 0..40 {
                    h.world
                        .conversation_line(JANE, line("Jane", JANE, ChatKind::Im, &format!("line {k}")));
                }
            }
            let last = if long { "line 39" } else { "hello from Jane" };
            h.st.selected = Some(JANE);
            crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(200.0, 150.0), egui::vec2(800.0, 560.0));
            h.settle();
            h.settle();
            let window = |h: &Harness| {
                h.frames
                    .ctx
                    .memory(|m| m.area_rect(egui::Id::unique("conversations")))
                    .expect("the window")
            };
            for (edge, steps) in [
                ("bottom slow", [-7.3_f32; 12].to_vec()),
                ("bottom fast", vec![-80.0, -80.0, -80.0, 120.0, 120.0, -60.5, 60.5]),
                ("top slow", [7.3_f32; 12].to_vec()),
                ("top fast", vec![80.0, 80.0, 80.0, -120.0, -120.0, 60.5, -60.5]),
            ] {
                let start = window(&h);
                let top = edge.starts_with("top");
                let grab = if top {
                    egui::pos2(start.center().x, start.top() + 1.0)
                } else {
                    egui::pos2(start.center().x, start.bottom() - 1.0)
                };
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                h.frame(vec![egui::Event::PointerMoved(grab)]);
                h.frame(vec![button(grab, true)]);
                let mut at = grab;
                let mut seen = Vec::new();
                for dy in steps {
                    at.y += dy;
                    let texts = h.frame(vec![egui::Event::PointerMoved(at)]);
                    let row = last_text(&texts, last);
                    let w = window(&h);
                    // against the window's bottom (the field), or on the screen;
                    // a short conversation of the Legacy style stands at the
                    // top: against the window's top
                    seen.push(if !long && style == ChatStyle::Legacy {
                        row.top() - w.top()
                    } else if top {
                        row.top()
                    } else {
                        row.top() - w.bottom()
                    });
                }
                h.frame(vec![button(at, false)]);
                h.settle();
                assert!(
                    seen.iter().all(|y| (y - seen[0]).abs() < 0.01),
                    "{style:?}, {} conversation, {edge}: {seen:?}",
                    if long { "long" } else { "short" }
                );
            }
        }
    }
}

/// Where the last text `source` is painted, if it is.
fn last_text(texts: &[crate::ui::widgets::elide_tests::Text], source: &str) -> egui::Rect {
    texts
        .iter()
        .rev()
        .find(|t| t.source == source)
        .map_or(egui::Rect::NOTHING, |t| t.rect)
}

const GROUP: Uuid = Uuid::from_u128(0x6);

/// Whether a text `label` is painted.
fn shown(texts: &[crate::ui::widgets::elide_tests::Text], label: &str) -> bool {
    texts.iter().any(|t| t.source == label)
}

/// The right-click menu of a private conversation in the column: Close and
/// its call, the resident's own menu (the one of their name) and the rows
/// the conversation item adds; its Close closes the conversation.
#[test]
fn a_private_conversation_opens_its_menu_in_the_column() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.settle();
    let jane = h.rect(entry_id(Pinned::Conversation(JANE)));
    let texts = h.right_click(jane.center());
    let close = aurora_i18n::tr!("ui-chat-close-conversation");
    for label in [
        close,
        aurora_i18n::tr!("common-voice-call"),
        aurora_i18n::tr!("common-view-profile"),
        aurora_i18n::tr!("common-copy-name"),
        aurora_i18n::tr!("common-pay"),
    ] {
        assert!(shown(&texts, label), "{label:?} in the menu");
    }
    assert!(!shown(&texts, aurora_i18n::tr!("people-group-profile")), "no group rows");
    h.click(last(&texts, close).center());
    assert!(h.actions.iter().any(|a| matches!(a, ConvAction::Close(id) if *id == JANE)));
}

/// A group's conversation: Close, its call, the transcript and the group's
/// actions (profile, activate unless it is the active group, stop its
/// chat, leave), none of a resident's.
#[test]
fn a_group_conversation_opens_the_group_actions() {
    use crate::ui::context::{CtxAction, take_requests};
    let mut h = Harness::new(ChatStyle::Aurora);
    h.world.groups.groups.push(aurora_net::GroupMembership {
        id: GROUP,
        name: "Builders".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    h.world.conversation_line(GROUP, line("Bob", BOB, ChatKind::Im, "hello builders"));
    h.world.calls.voice_enabled = true;
    h.settle();
    let group = h.rect(entry_id(Pinned::Conversation(GROUP)));
    let texts = h.right_click(group.center());
    let activate = aurora_i18n::tr!("ui-chat-menu-activate-group");
    let call = aurora_i18n::tr!("common-voice-call");
    assert!(
        shown(&texts, aurora_i18n::tr!("ui-chat-close-conversation"))
            || shown(&texts, aurora_i18n::tr!("ui-chat-close-conversation-leave"))
    );
    for label in [
        call,
        aurora_i18n::tr!("common-view-conversation-history"),
        aurora_i18n::tr!("people-group-profile"),
        activate,
        aurora_i18n::tr!("common-stop-receiving-this-group-s-chat"),
        aurora_i18n::tr!("ui-contacts-leave-group"),
    ] {
        assert!(shown(&texts, label), "{label:?} in the menu");
    }
    assert!(!shown(&texts, aurora_i18n::tr!("common-view-profile")), "no resident rows");
    take_requests(&h.frames.ctx);
    h.click(last(&texts, activate).center());
    let asked = take_requests(&h.frames.ctx);
    assert!(
        asked.iter().any(|a| matches!(a, CtxAction::ActivateGroup(id) if *id == GROUP)),
        "{asked:?}"
    );
    // the active group already: greyed
    h.world.groups.active = GROUP;
    h.settle();
    let texts = h.right_click(group.center());
    h.click(last(&texts, activate).center());
    assert!(take_requests(&h.frames.ctx).is_empty(), "nothing asked from a greyed row");
    let texts = h.right_click(group.center());
    h.click(last(&texts, call).center());
    assert!(h.actions.iter().any(|a| matches!(a, ConvAction::Call(id) if *id == GROUP)));
}

/// The nearby chat's menu holds its transcript only: it cannot be closed.
#[test]
fn the_nearby_chat_menu_holds_its_transcript() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.settle();
    let local = h.rect(entry_id(Pinned::Local));
    let texts = h.right_click(local.center());
    assert!(shown(&texts, aurora_i18n::tr!("common-view-conversation-history")));
    assert!(!shown(&texts, aurora_i18n::tr!("ui-chat-close-conversation")));
    assert!(!shown(&texts, aurora_i18n::tr!("common-voice-call")));
}

/// The tabs of the Bubbles and Lines styles open the same menu.
#[test]
fn the_tabs_of_the_other_styles_open_the_menu() {
    for style in [ChatStyle::Bubbles, ChatStyle::Legacy] {
        let mut h = Harness::new(style);
        let texts = h.settle();
        let name = h.world.session_title(&JANE);
        // the tab: the leftmost text with the conversation's name
        let tab = texts
            .iter()
            .filter(|t| t.source == name)
            .min_by(|a, b| a.rect.left().total_cmp(&b.rect.left()))
            .expect("the tab is drawn")
            .rect;
        let texts = h.right_click(tab.center());
        let close = aurora_i18n::tr!("ui-chat-close-conversation");
        assert!(shown(&texts, close), "{style:?}");
        assert!(shown(&texts, aurora_i18n::tr!("common-view-profile")), "{style:?}");
        h.click(last(&texts, close).center());
        assert!(
            h.actions.iter().any(|a| matches!(a, ConvAction::Close(id) if *id == JANE)),
            "{style:?}"
        );
    }
}

/// Away from the end of a conversation, the button back to the latest
/// messages shows and counts what came in; its click brings the end back,
/// where the next line shows at once (#558), in both windows.
#[test]
fn the_latest_button_brings_the_end_back() {
    let jump = aurora_i18n::tr!("ui-chat-latest");
    let two = aurora_i18n::tr!("ui-chat-latest-new", count = 2usize);
    let count = |texts: &[crate::ui::widgets::elide_tests::Text], s: &str| texts.iter().filter(|t| t.source == s).count();
    let has = |texts: &[crate::ui::widgets::elide_tests::Text], s: &str| count(texts, s) > 0;
    for style in [ChatStyle::Aurora, ChatStyle::Legacy] {
        let mut h = Harness::new(style);
        for k in 0..60 {
            h.world
                .conversation_line(JANE, line("Jane", JANE, ChatKind::Im, &format!("old {k}")));
        }
        h.st.selected = Some(JANE);
        // never, not even for the frame the conversation is first shown
        for _ in 0..8 {
            assert!(!has(&h.frame(Vec::new()), jump), "{style:?}: at the end, no button");
        }
        // to the top (the demo's way)
        h.st.demo_scroll = Some(JANE);
        h.settle();
        h.settle();
        assert!(h.st.demo_scrolled, "{style:?}");
        let texts = (0..12).map(|_| h.frame(Vec::new())).last().unwrap_or_default();
        assert!(has(&texts, jump), "{style:?}: far from the end");
        // two lines come in, out of sight
        h.world.conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "new 1"));
        h.world.conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "new 2"));
        // (the column's entry says the last one too)
        let texts = h.settle();
        let away = count(&texts, "new 2");
        let at = painted(&texts, &two).rect.center();
        h.click(at);
        let texts = (0..12).map(|_| h.frame(Vec::new())).last().unwrap_or_default();
        assert_eq!(count(&texts, "new 2"), away + 1, "{style:?}: the end in sight");
        assert!(!has(&texts, &two) && !has(&texts, jump), "{style:?}: the button gone");
        // the lines follow the end again
        h.world.conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "after"));
        assert_eq!(count(&h.settle(), "after"), away + 1, "{style:?}");
    }
}

/// The column's entries say the last line with its links by their labels,
/// as the thread shows them, never the raw `secondlife:///app` address
/// (#947): the nearby chat's entry and a conversation's.
#[test]
fn the_column_previews_show_link_labels() {
    let (lune, group) = (Uuid::from_u128(0x71), Uuid::from_u128(0x72));
    let mut h = Harness::new(ChatStyle::Aurora);
    h.world.social.names.insert(lune, "Lune Étoilée".into());
    h.world.land.group_names.insert(group, "Cercle des Brumes".into());
    let said = format!(
        "join secondlife:///app/group/{group}/about with secondlife:///app/agent/{lune}/about at secondlife:///app/teleport/Pinede/1/2/3"
    );
    h.world
        .chat
        .push_back(line("Nova Demo", BOB, ChatKind::Local(aurora_net::ChatType::Normal), &said));
    h.world.conversation_line(
        JANE,
        line("Jane", JANE, ChatKind::Im, &format!("ask secondlife:///app/agent/{lune}/about")),
    );
    let texts = h.settle();
    // (a teleport link reads "Teleport to Pinede (1,2,3)", in the language)
    assert!(
        texts
            .iter()
            .any(|t| t.source.contains(" · Nova : join Cercle des Brumes with Lune Étoilée at ") && t.source.ends_with(" Pinede (1,2,3)")),
        "the nearby chat's entry: {:?}",
        texts.iter().map(|t| &t.source).collect::<Vec<_>>()
    );
    assert!(texts.iter().any(|t| t.source.ends_with("ask Lune Étoilée")), "Jane's entry");
    assert!(texts.iter().all(|t| !t.source.contains("secondlife:")), "no raw address anywhere");
}
