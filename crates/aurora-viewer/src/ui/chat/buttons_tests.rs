//! Every control of the Conversations window, pressed through real egui
//! frames in the four styles: what it asks of the app (`ConvAction`), what
//! it changes in the window, or the menu it opens.

use super::members::member_id;
use super::sidebar::{Pinned, control_id, entry_id, search_id};
use super::stage::person_id;
use super::{ChatUi, ConvAction, Start, classic_id, compose::Volume, show};
use crate::settings::ChatStyle;
use crate::ui::widgets::elide_tests::{Frames, Text};
use crate::world::calls::{CallMember, InviteAnswer, PendingInvite};
use crate::world::{ChatKind, ChatLine, Origin, World};
use std::collections::HashMap;
use uuid::Uuid;

const JANE: Uuid = Uuid::from_u128(0xA);
const BOB: Uuid = Uuid::from_u128(0xB);
const GROUP: Uuid = Uuid::from_u128(0x6E);
const MEETING: Uuid = Uuid::from_u128(0xC0);
const ALL: [ChatStyle; 4] = [ChatStyle::Aurora, ChatStyle::Compact, ChatStyle::Bubbles, ChatStyle::Legacy];

#[test]
fn teleport_invitation_paints_one_destination_in_every_style() {
    use crate::world::teleport_invites::{Answer, Invite, InviteKind, Place, Rating};
    for style in ALL {
        let mut h = H::new(style);
        let message = "Join me\r\nhttp://maps.secondlife.com/secondlife/Aurora%20Demo/135/88/24";
        h.world.teleport_invites.list.push(Invite {
            id: 1,
            from: JANE,
            kind: InviteKind::Offer {
                lure: Uuid::from_u128(1),
                place: Some(Place {
                    grid: (1000, 1000),
                    pos: glam::Vec3::new(133.0, 88.0, 25.0),
                    region: Some("Aurora Demo".into()),
                }),
                godlike: false,
            },
            message: message.into(),
            rating: Rating::Allowed,
            answer: Answer::Pending,
            notification: 0,
        });
        h.world.conversation_line(JANE, line("Jane", JANE, ChatKind::Invite(1), message));
        h.open(Some(JANE));
        h.settle();
        let occurrences: usize = h.texts.iter().map(|t| t.source.matches("Aurora Demo (").count()).sum();
        assert_eq!(occurrences, 1, "{style:?}: the destination is painted once");
        assert!(
            h.texts.iter().any(|t| t.source.contains("Join me")),
            "{style:?}: the sender's text stays"
        );
        assert_eq!(h.world.teleport_invites.get(1).unwrap().message, message);
    }
}

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

struct H {
    frames: Frames,
    world: World,
    st: ChatUi,
    icons: crate::ui::icons::Icons,
    emoji: crate::ui::emoji::Emoji,
    contacts: crate::ui::contacts::ContactsUi,
    contacts_settings: crate::ui::contacts::ContactsSettings,
    gestures: crate::gestures::Manager,
    actions: Vec<ConvAction>,
    texts: Vec<Text>,
}

impl H {
    /// Jane's and Bob's conversations, a role group's and a conference's,
    /// in a 960 × 580 window.
    fn new(style: ChatStyle) -> H {
        let frames = Frames::new();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.social.names.insert(JANE, "Jane Doe".into());
        world.social.names.insert(BOB, "Bob Doe".into());
        world.conversation_line(JANE, line("Jane", JANE, ChatKind::Im, "hello from Jane"));
        world.conversation_line(BOB, line("Bob", BOB, ChatKind::Im, "hello from Bob"));
        world.groups.groups.push(aurora_net::GroupMembership {
            id: GROUP,
            name: "Builders".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        world.conversation_line(GROUP, line("Jane", JANE, ChatKind::Im, "hi builders"));
        let mut meeting = crate::world::groups::SessionInfo::default();
        meeting.name = "Meeting".into();
        meeting.participants = [JANE, BOB].into_iter().collect();
        world.groups.sessions.insert(MEETING, meeting);
        world.conversation_line(MEETING, line("Bob", BOB, ChatKind::Im, "hi meeting"));
        crate::ui::widgets::place_floater(&frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 580.0));
        H {
            frames,
            world,
            st: ChatUi {
                style,
                voice_on: true,
                ..ChatUi::default()
            },
            icons,
            emoji: crate::ui::emoji::Emoji::load(),
            contacts: Default::default(),
            contacts_settings: Default::default(),
            gestures: Default::default(),
            actions: Vec::new(),
            texts: Vec::new(),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let p = crate::theme::Theme::default().palette();
        let pics = HashMap::new();
        let H {
            frames,
            world,
            st,
            icons,
            emoji,
            contacts,
            contacts_settings,
            gestures,
            actions,
            texts,
        } = self;
        let mut open = true;
        *texts = frames.run(events, |ui| {
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
                false,
                None,
                &mut open,
            ));
        });
    }

    /// A frame, and what it asks of the platform (a copy).
    fn frame_out(&mut self, events: Vec<egui::Event>) -> egui::PlatformOutput {
        let p = crate::theme::Theme::default().palette();
        let pics = HashMap::new();
        let H {
            frames,
            world,
            st,
            icons,
            emoji,
            contacts,
            contacts_settings,
            gestures,
            actions,
            texts,
        } = self;
        let mut open = true;
        let (t, out) = frames.run_with_output(events, |ui| {
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
                false,
                None,
                &mut open,
            ));
        });
        *texts = t;
        out
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    fn shown(&self, id: egui::Id) -> Option<egui::Rect> {
        self.frames.ctx.read_response(id).map(|r| r.rect)
    }

    fn at(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.settle();
    }

    /// Press the control `id`.
    fn press(&mut self, id: egui::Id) {
        let rect = self.shown(id).unwrap_or_else(|| panic!("{id:?} is not shown"));
        self.at(rect.center());
    }

    /// Press the last text `source` painted (a menu row, a word button).
    fn press_text(&mut self, source: &str) {
        let rect = self.text_rect(source);
        self.at(rect.center());
    }

    /// Where the last text `source` is painted.
    fn text_rect(&self, source: &str) -> egui::Rect {
        self.texts
            .iter()
            .rev()
            .find(|t| t.source == source)
            .unwrap_or_else(|| panic!("{source:?} is not painted"))
            .rect
    }

    /// A right click at `pos`.
    fn right_click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.settle();
    }

    fn painted(&self, source: &str) -> bool {
        self.texts.iter().any(|t| t.source == source)
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![egui::Event::Text(text.into())]);
        self.settle();
    }

    fn enter(&mut self, modifiers: egui::Modifiers) {
        self.frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.settle();
    }

    fn history_key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(history_events(key, modifiers));
        self.settle();
    }

    fn bar_frame(&mut self, events: Vec<egui::Event>) -> Option<super::OutgoingChat> {
        let mut sent = None;
        self.frames.run(events, |ui| {
            sent = super::bar(ui, &crate::theme::Theme::default().palette(), &mut self.st, &self.gestures, 400.0);
        });
        sent
    }

    fn take(&mut self) -> Vec<ConvAction> {
        std::mem::take(&mut self.actions)
    }

    /// Show a conversation, its field with the keyboard.
    fn open(&mut self, conversation: Option<Uuid>) {
        self.st.selected = conversation;
        self.st.conv_focus_request = true;
        self.settle();
        self.take();
    }

    /// A call in progress in `conversation`, Jane in it with us.
    fn call(&mut self, conversation: Uuid) {
        self.world.start_call(conversation);
        let me = self.world.agent_id;
        if let Some(call) = self.world.calls.active.as_mut() {
            for id in [me, JANE] {
                call.members.push(CallMember {
                    id,
                    level: 0.0,
                    speaking: id == JANE,
                    muted: false,
                });
            }
        }
        self.st.mic = Some(true);
        self.settle();
        self.take();
    }

    /// Someone calls us: Jane herself, or Jane in the role group.
    fn invite(&mut self, p2p: bool) {
        let session = if p2p { Uuid::from_u128(0x5E) } else { GROUP };
        self.world.calls.invites.push(PendingInvite {
            invite: aurora_net::VoiceInvite {
                session,
                session_name: "Builders".into(),
                from: JANE,
                from_name: "Jane".into(),
                p2p,
                voice: Default::default(),
            },
            notification: 0,
            since: std::time::Instant::now(),
        });
        self.settle();
    }
}

fn history_events(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![
        egui::Event::ModifiersChanged(modifiers),
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
    ]
}

#[test]
fn input_history_is_separate_for_local_private_group_and_conference_in_every_style() {
    for style in ALL {
        let mut h = H::new(style);
        let conversations = [None, Some(JANE), Some(BOB), Some(GROUP), Some(MEETING)];
        for (n, conversation) in conversations.into_iter().enumerate() {
            h.open(conversation);
            h.type_text(&format!("first {n}"));
            h.enter(egui::Modifiers::NONE);
            h.type_text(&format!("latest {n}"));
            let send = if style.workspace() {
                control_id("box-send")
            } else {
                classic_id("send")
            };
            h.press(send);
            assert!(h.st.draft_mut(conversation).is_empty(), "{style:?} {conversation:?}");
        }
        for (n, conversation) in conversations.into_iter().enumerate() {
            h.open(conversation);
            h.type_text(&format!("draft {n}"));
            h.history_key(egui::Key::ArrowUp, egui::Modifiers::CTRL);
            assert_eq!(h.st.draft_mut(conversation), &format!("latest {n}"), "{style:?}");
            h.history_key(egui::Key::ArrowUp, egui::Modifiers::CTRL);
            assert_eq!(h.st.draft_mut(conversation), &format!("first {n}"), "{style:?}");
            h.history_key(egui::Key::ArrowUp, egui::Modifiers::CTRL);
            assert_eq!(h.st.draft_mut(conversation), &format!("first {n}"), "{style:?}");
            h.history_key(egui::Key::ArrowDown, egui::Modifiers::CTRL);
            h.history_key(egui::Key::ArrowDown, egui::Modifiers::CTRL);
            assert_eq!(h.st.draft_mut(conversation), &format!("draft {n}"), "{style:?}");
        }
        // Closing one conversation discards only its recall, as its editor
        // is destroyed in the Legacy Viewer; opening it starts afresh.
        h.st.forget(JANE);
        h.open(Some(JANE));
        h.history_key(egui::Key::ArrowUp, egui::Modifiers::CTRL);
        assert!(h.st.draft_mut(Some(JANE)).is_empty());
        h.open(Some(GROUP));
        h.history_key(egui::Key::ArrowUp, egui::Modifiers::CTRL);
        assert_eq!(h.st.draft_mut(Some(GROUP)), "latest 3");
    }
}

#[test]
fn input_history_is_shared_both_ways_between_local_fields_in_every_style() {
    for style in ALL {
        let mut h = H::new(style);
        h.st.focus_request = true;
        for _ in 0..3 {
            h.bar_frame(Vec::new());
        }
        h.bar_frame(vec![egui::Event::Text("/42 café 🙂".into())]);
        let sent = h
            .bar_frame(history_events(egui::Key::Enter, egui::Modifiers::CTRL))
            .expect("bar sends");
        assert_eq!(sent.channel, 42);
        assert_eq!(sent.message, "café 🙂");
        assert_eq!(sent.chat_type, aurora_net::ChatType::Shout);
        h.open(None);
        h.history_key(egui::Key::ArrowUp, egui::Modifiers::CTRL);
        assert_eq!(h.st.draft_mut(None), "/42 café 🙂", "{style:?}");
        h.history_key(egui::Key::ArrowDown, egui::Modifiers::CTRL);
        h.type_text("from the window");
        h.enter(egui::Modifiers::NONE);
        h.st.focus_request = true;
        for _ in 0..3 {
            h.bar_frame(Vec::new());
        }
        h.bar_frame(vec![egui::Event::Text("bar draft".into())]);
        for modifiers in [
            egui::Modifiers::NONE,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Modifiers::ALT,
        ] {
            h.bar_frame(history_events(egui::Key::ArrowUp, modifiers));
            assert_eq!(h.st.input, "bar draft", "{style:?}");
        }
        h.bar_frame(history_events(egui::Key::ArrowUp, egui::Modifiers::CTRL));
        assert_eq!(h.st.input, "from the window", "{style:?}");
        let range = egui::TextEdit::load_state(&h.frames.ctx, super::bar_input_id())
            .and_then(|s| s.cursor.char_range())
            .expect("recalled cursor");
        assert!(range.is_empty());
        assert_eq!(range.primary.index.0, h.st.input.chars().count());
        h.bar_frame(history_events(egui::Key::ArrowUp, egui::Modifiers::CTRL));
        assert_eq!(h.st.input, "/42 café 🙂", "{style:?}");
        h.bar_frame(history_events(egui::Key::ArrowDown, egui::Modifiers::CTRL));
        h.bar_frame(history_events(egui::Key::ArrowDown, egui::Modifiers::CTRL));
        assert_eq!(h.st.input, "bar draft", "{style:?}");
    }
}

fn has(actions: &[ConvAction], f: impl Fn(&ConvAction) -> bool) -> bool {
    actions.iter().any(f)
}

/// Column (Aurora, Compact): the search filters and finds a resident; the
/// "new conversation" button beside it starts each kind of conversation;
/// the entries show theirs.
#[test]
fn the_column_searches_and_starts_conversations() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        h.settle();
        h.press(search_id());
        h.type_text("bob");
        assert!(h.shown(entry_id(Pinned::Conversation(BOB))).is_some(), "{style:?}");
        assert!(
            h.shown(entry_id(Pinned::Conversation(JANE))).is_none(),
            "{style:?}: Jane filtered out"
        );
        h.press(egui::Id::unique("conversation_find"));
        assert!(h.st.picker.open && h.st.start == Start::Im, "{style:?}: the resident search");
        h.st.picker.open = false;
        h.st.side_filter.clear();
        h.settle();
        h.press(control_id("new-conversation"));
        h.press_text(aurora_i18n::tr!("ui-chat-start-private"));
        assert!(h.st.picker.open && h.st.start == Start::Im, "{style:?}: a private message");
        h.st.picker.open = false;
        h.settle();
        h.press(control_id("new-conversation"));
        h.press_text(aurora_i18n::tr!("ui-chat-start-conference"));
        assert!(h.st.picker.open && h.st.start == Start::Conference, "{style:?}: a conference");
        h.st.picker.open = false;
        h.settle();
        h.press(control_id("new-conversation"));
        h.press_text(aurora_i18n::tr!("ui-chat-start-group"));
        h.press_text("Builders");
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::StartGroupChat(g) if *g == GROUP)),
            "{style:?}: a group's chat"
        );
        for section in ["Private", "Group", "Conference"] {
            assert!(
                h.shown(control_id(&format!("section-add-{section}"))).is_none(),
                "{style:?}: no + on the {section} section"
            );
        }
        h.press(entry_id(Pinned::Conversation(MEETING)));
        assert_eq!(h.st.selected, Some(MEETING), "{style:?}");
        h.press(entry_id(Pinned::Local));
        assert_eq!(h.st.selected, None, "{style:?}");
    }
}

/// Narrow, the rail's pictures show their conversations.
#[test]
fn the_rail_shows_conversations() {
    let mut h = H::new(ChatStyle::Aurora);
    crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(460.0, 580.0));
    h.settle();
    h.press(entry_id(Pinned::Conversation(BOB)));
    assert_eq!(h.st.selected, Some(BOB));
    h.press(entry_id(Pinned::Local));
    assert_eq!(h.st.selected, None);
}

/// The head (Aurora, Compact) of a resident's conversation, a role group's
/// and a conference's: call, add people, profile, add friend, teleport,
/// close, members, find, and the rows of its "…".
#[test]
fn the_head_acts() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        h.open(Some(BOB));
        h.press(control_id("head-phone-filled"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Call(id) if *id == BOB)),
            "{style:?}: call"
        );
        h.press(control_id("head-message-circle-plus"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::PickPeople(id) if *id == BOB)),
            "{style:?}: add"
        );
        h.press(control_id("head-user-filled"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Profile(id) if *id == BOB)),
            "{style:?}: profile"
        );
        h.press(control_id("head-send-filled"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::OfferTeleport(id) if *id == BOB)),
            "{style:?}: teleport"
        );
        h.press(control_id("head-x"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Close(id) if *id == BOB)),
            "{style:?}: close"
        );
        h.press(control_id("head-search-filled"));
        assert!(h.st.searches.get(&Some(BOB)).is_some_and(|s| s.find.is_some()), "{style:?}: find");
        h.press(control_id("head-search-filled"));
        assert!(
            h.st.searches.get(&Some(BOB)).is_some_and(|s| s.find.is_none()),
            "{style:?}: find closed"
        );
        for (row, wanted) in [
            (aurora_i18n::tr!("common-offer-teleport"), "teleport"),
            (aurora_i18n::tr!("ui-chat-give-from-inventory"), "give"),
            (aurora_i18n::tr!("ui-call-add-people"), "add"),
            (aurora_i18n::tr!("common-block"), "block"),
        ] {
            h.press(control_id("head-dots-filled"));
            h.press_text(row);
            let actions = h.take();
            let ok = match wanted {
                "teleport" => has(&actions, |a| matches!(a, ConvAction::OfferTeleport(id) if *id == BOB)),
                "give" => has(&actions, |a| matches!(a, ConvAction::OpenInventory)),
                "add" => has(&actions, |a| matches!(a, ConvAction::PickPeople(id) if *id == BOB)),
                _ => has(&actions, |a| matches!(a, ConvAction::ToggleBlock(id) if *id == BOB)),
            };
            assert!(ok, "{style:?}: … › {row}");
        }
        h.press(control_id("head-dots-filled"));
        h.press_text(aurora_i18n::tr!("common-become-friends"));
        assert!(
            crate::ui::context::take_requests(&h.frames.ctx)
                .iter()
                .any(|a| matches!(a, crate::ui::context::CtxAction::AddFriend(id) if *id == BOB)),
            "{style:?}: … › become friends"
        );
        // the friendship offer of the head, only to a stranger
        h.press(control_id("head-user-plus"));
        assert!(
            crate::ui::context::take_requests(&h.frames.ctx)
                .iter()
                .any(|a| matches!(a, crate::ui::context::CtxAction::AddFriend(id) if *id == BOB)),
            "{style:?}: add friend"
        );
        h.world.social.friends.push(crate::world::social::Friend {
            id: BOB,
            online: true,
            rights_given: 1,
            rights_has: 1,
        });
        h.settle();
        assert!(
            h.shown(control_id("head-user-plus")).is_none(),
            "{style:?}: a friend is not offered friendship"
        );
        h.world.social.friends.clear();
        // a role group: members, no adding (like Firestorm)
        h.open(Some(GROUP));
        assert!(
            h.shown(control_id("head-message-circle-plus")).is_none(),
            "{style:?}: a role group adds nobody"
        );
        let open = h.st.members_open;
        h.press(control_id("head-users"));
        assert_ne!(h.st.members_open, open, "{style:?}: members");
        // a conference: adding, and its members' panel
        h.open(Some(MEETING));
        h.st.members_open = true;
        h.press(control_id("head-message-circle-plus"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::PickPeople(id) if *id == MEETING)),
            "{style:?}"
        );
        h.press(member_id(BOB));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Profile(id) if *id == BOB)),
            "{style:?}: member"
        );
        h.press_text(aurora_i18n::tr!("ui-call-add"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::PickPeople(id) if *id == MEETING)),
            "{style:?}: panel's add"
        );
        // the nearby chat: the people around
        h.open(None);
        h.press(control_id("head-users"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::OpenPeople(0))),
            "{style:?}: nearby people"
        );
    }
}

/// A message under the pointer: no buttons, the arrow (no text cursor); its
/// right click copies it, quotes it in the field, and holds its author's
/// menu (the profile among them).
#[test]
fn the_message_menu_acts() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        h.open(Some(JANE));
        let text = h.texts.iter().rev().find(|t| t.source == "hello from Jane").expect("the line").rect;
        h.frame(vec![egui::Event::PointerMoved(text.center())]);
        let out = h.frame_out(Vec::new());
        assert_eq!(out.cursor_icon, egui::CursorIcon::Default, "{style:?}: the arrow over a message");
        let jane = h.world.social.ims.iter().find(|s| s.other == JANE).expect("Jane").lines[0].clone();
        for salt in ["copy", "quote", "profile", "more"] {
            let id = egui::Id::unique(("thread_tool", salt, jane.time, jane.source, jane.text.as_str()));
            assert!(h.shown(id).is_none(), "{style:?}: no {salt} button");
        }
        // copy
        h.right_click(text.center());
        h.world
            .conversation_line(JANE, line("Bob", BOB, ChatKind::Im, "arriving while copying"));
        h.settle();
        let row = h.text_rect(aurora_i18n::tr!("ui-chat-copy-message"));
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.frame(vec![egui::Event::PointerMoved(row.center())]);
        h.frame(vec![button(row.center(), true)]);
        let out = h.frame_out(vec![button(row.center(), false)]);
        assert!(
            out.commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t.ends_with("hello from Jane"))),
            "{style:?}: copy"
        );
        h.settle();
        // quote
        let text = h.text_rect("hello from Jane");
        h.right_click(text.center());
        h.world
            .conversation_line(JANE, line("Bob", BOB, ChatKind::Im, "arriving while quoting"));
        h.settle();
        h.press_text(aurora_i18n::tr!("ui-chat-quote-message"));
        assert_eq!(h.st.draft_mut(Some(JANE)), "> hello from Jane ", "{style:?}: quote");
        // the author's menu
        let text = h.text_rect("hello from Jane");
        h.right_click(text.center());
        assert!(h.painted(aurora_i18n::tr!("common-view-profile")), "{style:?}: the author's menu");
    }
}

/// A double click on the first or the last word of a message, then Ctrl+C,
/// copies that word alone: not the line's "[time] Name: " prefix, nor the
/// whole message.
#[test]
fn a_double_clicked_word_is_copied_alone() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for conversation in [None, Some(JANE)] {
            // the first and last words of a message, and a message of one word
            for (said, at, word) in [
                ("hello from Jane", 2.5 / 15.0, "hello"),
                ("hello from Jane", 13.0 / 15.0, "Jane"),
                ("hello", 0.5, "hello"),
            ] {
                let mut h = H::new(style);
                let message = line("Jane", JANE, ChatKind::Im, said);
                if let Some(id) = conversation {
                    h.world.social.session_mut(id).lines = [message].into();
                } else {
                    h.world.chat = [message].into();
                }
                h.open(conversation);
                let text = h.text_rect(said);
                let pos = egui::pos2(text.left() + text.width() * at, text.center().y);
                let button = |pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                h.frame(vec![egui::Event::PointerMoved(pos)]);
                for _ in 0..2 {
                    h.frame(vec![button(true)]);
                    h.frame(vec![button(false)]);
                }
                h.settle();
                let copied = h.frame_out(vec![egui::Event::Copy]).commands.into_iter().find_map(|c| match c {
                    egui::OutputCommand::CopyText(t) => Some(t),
                    _ => None,
                });
                assert_eq!(copied.as_deref(), Some(word), "{style:?}, {conversation:?}, {said:?}");
            }
        }
    }
}

#[test]
fn selected_messages_copy_from_the_menu_even_after_eviction() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for conversation in [None, Some(JANE)] {
            for reverse in [false, true] {
                let mut h = H::new(style);
                let messages = [
                    line("Jane", JANE, ChatKind::Im, "first selected message"),
                    line("Bob", BOB, ChatKind::Im, "second selected message"),
                ];
                let expected = messages
                    .iter()
                    .map(|l| format!("{}{}", super::copy_prefix(l, &l.from), l.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(id) = conversation {
                    h.world.social.session_mut(id).lines = messages.into();
                } else {
                    h.world.chat = messages.into();
                }
                h.open(conversation);
                let first = h.text_rect("first selected message");
                let last = h.text_rect("second selected message");
                let (a, b) = (
                    egui::pos2(first.left() + 1.0, first.center().y),
                    egui::pos2(last.right() + 2.0, last.center().y),
                );
                let (from, to) = if reverse { (b, a) } else { (a, b) };
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                h.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
                for k in 1..=4 {
                    h.frame(vec![egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
                }
                h.frame(vec![button(to, false)]);
                let copied = |output: egui::PlatformOutput| {
                    output.commands.into_iter().find_map(|c| match c {
                        egui::OutputCommand::CopyText(t) => Some(t),
                        _ => None,
                    })
                };
                assert_eq!(
                    copied(h.frame_out(vec![egui::Event::Copy])),
                    Some(expected.clone()),
                    "{style:?}, {conversation:?}"
                );
                h.right_click(last.center());
                // The menu keeps the selection that was actually clicked,
                // even if incoming traffic evicts both selected messages.
                let (lines, clock) = if let Some(id) = conversation {
                    let session = h.world.social.session_mut(id);
                    (&mut session.lines, &mut session.clock)
                } else {
                    (&mut h.world.chat, &mut h.world.chat_clock)
                };
                for _ in 0..2 {
                    lines.pop_front();
                    clock.popped_front();
                }
                lines.push_back(line("Bob", BOB, ChatKind::Im, "new arrival"));
                clock.pushed_back();
                h.settle();
                let pos = h.text_rect(aurora_i18n::tr!("ui-chat-copy-message")).center();
                h.frame(vec![egui::Event::PointerMoved(pos), button(pos, true)]);
                assert_eq!(
                    copied(h.frame_out(vec![button(pos, false)])),
                    Some(expected),
                    "{style:?}, {conversation:?}, reverse={reverse}"
                );
            }
        }
    }
}

#[test]
fn pasted_transcripts_keep_paragraphs_when_sent_in_every_style() {
    for style in ALL {
        for conversation in [None, Some(JANE)] {
            for text in ["Jane: first\nBob: second", "Jane: café 🙂\r\nBob: 猫\rthird"] {
                let mut h = H::new(style);
                h.open(conversation);
                let expected = text.replace("\r\n", "\n").replace('\r', "\n");
                let field = egui::Id::unique(("conv_input", conversation));
                let before = h.shown(field).expect("chat field");
                h.type_text("replace me");
                // egui creates an undo checkpoint after one stable second.
                for _ in 0..65 {
                    h.frame(Vec::new());
                }
                h.frame(history_events(egui::Key::A, egui::Modifiers::CTRL | egui::Modifiers::COMMAND));
                h.frame(vec![egui::Event::Paste(text.into())]);
                h.settle();
                assert_eq!(h.st.draft_mut(conversation), &expected, "{style:?}, {conversation:?}");
                let after = h.shown(field).expect("chat field");
                assert!(
                    after.height() > before.height() && after.height() < before.height() * 4.0,
                    "a pasted transcript expands the editor within its row limit: {before:?} -> {after:?}"
                );
                h.frame(history_events(egui::Key::Z, egui::Modifiers::CTRL | egui::Modifiers::COMMAND));
                assert_eq!(h.st.draft_mut(conversation), "replace me", "undo restores the replaced draft");
                h.frame(history_events(egui::Key::A, egui::Modifiers::CTRL | egui::Modifiers::COMMAND));
                h.frame(vec![egui::Event::Paste(text.into())]);
                h.frame(history_events(egui::Key::A, egui::Modifiers::CTRL | egui::Modifiers::COMMAND));
                let output = h.frame_out(vec![egui::Event::Copy]);
                assert!(
                    output
                        .commands
                        .iter()
                        .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if *t == expected)),
                    "copying the pasted draft retains its paragraphs"
                );
                h.enter(Default::default());
                let actions = h.take();
                assert!(
                    actions.iter().any(|action| match action {
                        ConvAction::Local(out) => conversation.is_none() && out.message == expected,
                        ConvAction::Im { to, text } => Some(*to) == conversation && *text == expected,
                        _ => false,
                    }),
                    "{style:?}, {conversation:?}: the outgoing message retains its paragraphs"
                );
            }
        }
    }
}

#[test]
fn pasted_transcripts_keep_paragraphs_in_the_nearby_chat_bar() {
    let mut h = H::new(ChatStyle::Aurora);
    h.st.focus_request = true;
    h.bar_frame(Vec::new());
    h.bar_frame(Vec::new());
    let text = "Jane: café\nBob: second line";
    h.bar_frame(vec![egui::Event::Paste(text.into())]);
    assert_eq!(h.st.input, text);
    let sent = h
        .bar_frame(history_events(egui::Key::Enter, egui::Modifiers::NONE))
        .expect("send the transcript");
    assert_eq!(sent.message, text);
}

/// The explicit paragraph chord wins over completion and sending, including
/// in the narrower detached panes. Shift+Enter whispers only in nearby chat;
/// in IMs and sessions it neither completes, inserts nor sends anything.
#[test]
fn paragraph_chord_edits_without_sending_in_every_conversation_style() {
    for style in ALL {
        for conversation in [None, Some(JANE), Some(GROUP), Some(MEETING)] {
            for detached in [false, true] {
                let mut h = H::new(style);
                h.open(conversation);
                if detached {
                    h.st.pop_out(conversation);
                    crate::ui::widgets::place_floater(
                        &h.frames.ctx,
                        &super::popout::floater_id(conversation),
                        egui::pos2(100.0, 100.0),
                        egui::vec2(500.0, 500.0),
                    );
                    h.st.conv_focus_request = true;
                    h.settle();
                }
                h.type_text("first :hea");
                if conversation.is_some() {
                    h.history_key(egui::Key::Enter, egui::Modifiers::SHIFT);
                    assert_eq!(h.st.draft_mut(conversation), "first :hea", "Shift+Enter leaves completion alone");
                    assert!(!h.take().iter().any(|a| matches!(a, ConvAction::Local(_) | ConvAction::Im { .. })));
                }
                h.history_key(egui::Key::Enter, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
                assert_eq!(h.st.draft_mut(conversation), "first :hea\n", "{style:?}, detached={detached}");
                assert!(!h.take().iter().any(|a| matches!(a, ConvAction::Local(_) | ConvAction::Im { .. })));
                h.type_text("second 🙂");
                h.history_key(egui::Key::Enter, egui::Modifiers::SHIFT);
                if conversation.is_some() {
                    assert_eq!(
                        h.st.draft_mut(conversation),
                        "first :hea\nsecond 🙂",
                        "Shift+Enter preserves the draft"
                    );
                    assert!(!h.take().iter().any(|a| matches!(a, ConvAction::Local(_) | ConvAction::Im { .. })));
                    let field = egui::Id::unique(("conv_input", conversation));
                    assert!(h.frames.ctx.memory(|m| m.has_focus(field)), "Shift+Enter preserves focus");
                    h.history_key(egui::Key::Enter, egui::Modifiers::NONE);
                }
                assert!(
                    h.take().iter().any(|a| match a {
                        ConvAction::Local(out) => out.message == "first :hea\nsecond 🙂" && out.chat_type == aurora_net::ChatType::Whisper,
                        ConvAction::Im { to, text } => Some(*to) == conversation && text == "first :hea\nsecond 🙂",
                        _ => false,
                    }),
                    "{style:?}, {conversation:?}, detached={detached}: the expected send shortcut"
                );
            }
        }
    }
}

#[test]
fn paragraph_chord_replaces_selection_and_undoes_without_sending() {
    for style in ALL {
        let mut h = H::new(style);
        h.open(None);
        h.type_text("replace me");
        for _ in 0..65 {
            h.frame(Vec::new());
        }
        h.history_key(egui::Key::A, egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
        h.history_key(egui::Key::Enter, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
        assert_eq!(h.st.draft_mut(None), "\n");
        assert!(!h.take().iter().any(|a| matches!(a, ConvAction::Local(_))));
        h.history_key(egui::Key::Z, egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
        assert_eq!(h.st.draft_mut(None), "replace me");
    }
}

#[test]
fn paragraph_editor_wraps_scrolls_and_collapses_in_every_style() {
    for style in ALL {
        let mut h = H::new(style);
        h.open(Some(JANE));
        let field = egui::Id::unique(("conv_input", Some(JANE)));
        let rect = |h: &H| h.frames.ctx.read_response(field).unwrap().interact_rect;
        let before = rect(&h);
        let text = "A wrapping sentence with café and an emoji 🙂. ".repeat(100);
        h.frame(vec![egui::Event::Paste(text.clone())]);
        h.settle();
        let expanded = rect(&h);
        assert!(expanded.height() > before.height() + 10.0, "{style:?}: wraps");
        assert!(expanded.height() <= 100.0, "{style:?}: bounded viewport {expanded:?}");
        for _ in 0..30 {
            h.frame(Vec::new());
        }
        assert_eq!(rect(&h), expanded, "{style:?}: neither field nor window keeps growing");
        assert_eq!(h.st.draft_mut(Some(JANE)), &text);
        h.enter(egui::Modifiers::NONE);
        assert!(
            h.take()
                .iter()
                .any(|a| matches!(a, ConvAction::Im { text: sent, .. } if sent == text.trim()))
        );
        assert!(
            (rect(&h).height() - before.height()).abs() < 0.5,
            "{style:?}: collapses after sending"
        );
    }
}

#[test]
fn nearby_bar_paragraph_chord_keeps_focus_and_volume_shortcuts() {
    let mut h = H::new(ChatStyle::Aurora);
    h.st.keep_focus = true;
    h.st.focus_request = true;
    h.bar_frame(Vec::new());
    h.bar_frame(Vec::new());
    h.bar_frame(vec![egui::Event::Paste("first\r\nsecond\rthird".into())]);
    assert!(
        h.bar_frame(history_events(egui::Key::Enter, egui::Modifiers::CTRL | egui::Modifiers::SHIFT))
            .is_none()
    );
    h.bar_frame(vec![egui::Event::Paste("\nlast 🙂".into())]);
    assert_eq!(h.st.input, "first\nsecond\nthird\n\nlast 🙂");
    assert!(h.frames.ctx.memory(|m| m.has_focus(super::bar_input_id())));
    let sent = h.bar_frame(history_events(egui::Key::Enter, egui::Modifiers::CTRL)).unwrap();
    assert_eq!(sent.message, "first\nsecond\nthird\n\nlast 🙂");
    assert_eq!(sent.chat_type, aurora_net::ChatType::Shout);
    h.bar_frame(Vec::new());
    h.bar_frame(vec![egui::Event::Text("whisper".into())]);
    let sent = h.bar_frame(history_events(egui::Key::Enter, egui::Modifiers::SHIFT)).unwrap();
    assert_eq!(sent.chat_type, aurora_net::ChatType::Whisper);
}

/// Incoming group messages must leave the original sender available to
/// block, both at the live end and while reading older messages (#557).
#[test]
fn incoming_messages_keep_chat_context_menus_open() {
    for style in ALL {
        for older in [false, true] {
            for sender in [false, true] {
                if !style.workspace() && !sender {
                    continue;
                }
                let mut h = H::new(style);
                h.world.groups.sessions.entry(GROUP).or_default().group = true;
                for i in 0..80 {
                    h.world
                        .conversation_line(GROUP, line("Jane", JANE, ChatKind::Im, &format!("earlier message {i}")));
                    h.world
                        .conversation_line(GROUP, line("Bob", BOB, ChatKind::Im, &format!("another message {i}")));
                }
                h.open(Some(GROUP));
                for _ in 0..30 {
                    h.frame(Vec::new());
                }
                if older {
                    let at = h.text_rect("another message 79").center();
                    h.frame(vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, 250.0),
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Unknown,
                            modifiers: Default::default(),
                        },
                    ]);
                    for _ in 0..30 {
                        h.frame(Vec::new());
                    }
                    assert!(!h.painted("another message 79"), "{style:?}: reading older messages");
                }
                let bodies: Vec<_> = h.texts.iter().filter(|t| t.source.starts_with("earlier message ")).collect();
                let body = bodies[bodies.len() / 2];
                let original = body.source.clone();
                let at = if sender {
                    h.texts
                        .iter()
                        .filter(|t| t.source == "Jane")
                        .min_by(|a, b| {
                            (a.rect.center().y - body.rect.center().y)
                                .abs()
                                .total_cmp(&(b.rect.center().y - body.rect.center().y).abs())
                        })
                        .unwrap_or_else(|| panic!("{style:?}: Jane's name is painted"))
                        .rect
                        .center()
                } else {
                    body.rect.center()
                };
                h.right_click(at);
                let block = aurora_i18n::tr!("common-ignore");
                assert!(h.painted(block), "{style:?}, older={older}, sender={sender}: menu opened");
                let position = h.text_rect(&original);
                for i in 0..20 {
                    h.world
                        .conversation_line(GROUP, line("Bob", BOB, ChatKind::Im, &format!("incoming message {i}")));
                    h.frame(Vec::new());
                    assert!(
                        h.painted(block),
                        "{style:?}, older={older}, sender={sender}: menu survives message {i}"
                    );
                }
                h.settle();
                assert!(
                    !h.painted("incoming message 19"),
                    "{style:?}: new messages stay below the paused viewport"
                );
                assert_eq!(
                    h.text_rect(&original),
                    position,
                    "{style:?}: the menu holds the original message in place"
                );
                h.press_text(block);
                assert!(
                    crate::ui::context::take_requests(&h.frames.ctx)
                        .iter()
                        .any(|a| matches!(a, crate::ui::context::CtxAction::ToggleBlock(id) if *id == JANE)),
                    "{style:?}: block the original sender, not the arriving messages' sender"
                );
                assert!(
                    !egui::Popup::is_any_open(&h.frames.ctx),
                    "{style:?}: choosing an action closes the menu"
                );
                let received = h.world.social.ims.iter().find(|s| s.other == GROUP).expect("group conversation");
                assert_eq!(received.lines.len(), 181, "{style:?}: every incoming message was received");
                assert_eq!(
                    h.st.views[&Some(GROUP)].lines().count(),
                    received.lines.len(),
                    "{style:?}: the live view contains every received message"
                );
                assert_eq!(
                    h.painted("incoming message 19"),
                    !older,
                    "{style:?}: retain live-end or manual-scroll behavior"
                );
            }
        }
    }
}

/// A short conversation keeps receiving messages while its menu is open.
/// Its bottom alignment must not pull the original message upward (#591).
#[test]
fn incoming_messages_keep_short_chat_context_menus_open() {
    for style in ALL {
        let mut h = H::new(style);
        h.world.groups.sessions.entry(GROUP).or_default().group = true;
        h.open(Some(GROUP));
        h.right_click(h.text_rect("Jane").center());
        let original = h.text_rect("hi builders");
        for i in 0..3 {
            h.world
                .conversation_line(GROUP, line("Jane", JANE, ChatKind::Im, &format!("short incoming {i}")));
            h.frame(Vec::new());
            assert!(h.painted(aurora_i18n::tr!("common-ignore")), "{style:?}: sender menu stays open");
            assert_eq!(
                h.text_rect("hi builders"),
                original,
                "{style:?}: keep the original message in place"
            );
            if !style.workspace() {
                assert!(
                    h.painted(&format!("short incoming {i}")),
                    "{style:?}: new messages fit below the original"
                );
            }
        }
        h.history_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!egui::Popup::is_any_open(&h.frames.ctx), "{style:?}: Escape closes the menu");
        assert!(h.painted("short incoming 2"), "{style:?}: new messages appear after closing");
    }
}

/// A fresh menu moves an existing New marker below the messages already
/// received, then leaves it before the first arrival throughout that menu.
#[test]
fn chat_context_menu_moves_the_new_marker_below_existing_messages() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        for i in 0..6 {
            h.world
                .conversation_line(GROUP, line("Bob", BOB, ChatKind::Im, &format!("before menu {i}")));
        }
        h.open(Some(GROUP));
        let session = h.world.social.ims.iter().find(|s| s.other == GROUP).unwrap();
        let boundary = session.clock.first + session.lines.len() as u64;
        h.st.new_lines.pause_at(Some(GROUP), boundary - 2);
        h.settle();
        let before = h.text_rect("before menu 5");
        h.right_click(h.text_rect("before menu 5").center());
        assert_eq!(
            h.text_rect("before menu 5"),
            before,
            "{style:?}: moving the older marker keeps the target in place"
        );
        assert_eq!(
            h.st.new_lines.at(Some(GROUP)),
            Some(boundary),
            "{style:?}: move the older boundary below the current messages"
        );
        let original = h.text_rect("before menu 5");
        for i in 0..3 {
            h.world
                .conversation_line(GROUP, line("Jane", JANE, ChatKind::Im, &format!("after menu {i}")));
            h.settle();
            assert_eq!(
                h.st.new_lines.at(Some(GROUP)),
                Some(boundary),
                "{style:?}: the first new arrival keeps the marker"
            );
            assert!(
                h.text_rect("before menu 5").min.distance(original.min) < 0.5,
                "{style:?}: the marker must not move the target by a pixel"
            );
        }
        h.history_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(h.painted("after menu 2"), "{style:?}: Escape resumes following");
        assert_eq!(h.st.new_lines.at(Some(GROUP)), Some(boundary));
    }
}

/// A link's own context menu holds and highlights its containing message,
/// while retaining its original native actions.
#[test]
fn an_inline_place_menu_holds_its_containing_message() {
    let url = "http://maps.secondlife.com/secondlife/Aurora%20Demo/128/130/24";
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        h.world
            .conversation_line(GROUP, line("Jane", JANE, ChatKind::Im, &format!("Look here: {url}")));
        h.open(Some(GROUP));
        // the link in the thread, not the column's preview of the line,
        // which reads its label too (#947)
        let link = h
            .texts
            .iter()
            .find(|t| t.source.contains("Aurora Demo (") && !t.source.contains("Look here"))
            .expect("place badge")
            .source
            .clone();
        h.right_click(h.text_rect(&link).center());
        let original = h.text_rect(&link);
        let copy = aurora_i18n::tr!("common-copy-slurl");
        assert!(h.painted(copy), "{style:?}: the place's own menu opens");
        assert!(h.st.views[&Some(GROUP)].is_paused(), "{style:?}: its message holds the viewport");
        for i in 0..20 {
            h.world
                .conversation_line(GROUP, line("Bob", BOB, ChatKind::Im, &format!("after the place {i}")));
        }
        h.settle();
        assert!(h.painted(copy), "{style:?}: the place menu survives arrivals");
        assert!(
            h.text_rect(&link).min.distance(original.min) < 0.5,
            "{style:?}: the linked message stays fixed"
        );
        assert!(
            h.st.views[&Some(GROUP)].below() > 0.0,
            "{style:?}: arrivals stay below the viewport"
        );
        let pos = h.text_rect(copy).center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.frame(vec![egui::Event::PointerMoved(pos)]);
        h.frame(vec![button(true)]);
        let out = h.frame_out(vec![button(false)]);
        assert!(
            out.commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == url)),
            "{style:?}: keep the link's Copy action"
        );
        h.settle();
        assert!(!egui::Popup::is_any_open(&h.frames.ctx));
        assert!(!h.st.views[&Some(GROUP)].is_paused());
        assert!(h.st.views[&Some(GROUP)].below() <= 1.0, "{style:?}: closing resumes following");
        h.world
            .conversation_line(GROUP, line("Jane", JANE, ChatKind::Im, &format!("A fresh place: {url}")));
        h.settle();
        h.right_click(h.text_rect(&link).center());
        assert!(h.painted(copy));
        for i in 0..2100 {
            h.world
                .conversation_line(GROUP, line("Bob", BOB, ChatKind::Im, &format!("evict the place {i}")));
        }
        h.settle();
        assert!(
            !egui::Popup::is_any_open(&h.frames.ctx),
            "{style:?}: eviction dismisses the inline menu"
        );
        assert!(
            !h.st.views[&Some(GROUP)].is_paused(),
            "{style:?}: no invisible menu keeps the viewport paused"
        );
    }
}

/// A busy conversation may evict even the clicked message from its
/// bounded live buffer; its menu must still act on the original sender.
#[test]
fn incoming_messages_keep_evicted_chat_context_menu_targets() {
    for style in ALL {
        for sender in [false, true] {
            if !style.workspace() && !sender {
                continue;
            }
            let mut h = H::new(style);
            h.world.groups.sessions.entry(GROUP).or_default().group = true;
            h.open(Some(GROUP));
            let original = h.text_rect("hi builders");
            let at = if sender { h.text_rect("Jane").center() } else { original.center() };
            h.right_click(at);
            for i in 0..2100 {
                h.world
                    .conversation_line(GROUP, line("Bob", BOB, ChatKind::Im, &format!("busy incoming {i}")));
            }
            let received = h.world.social.ims.iter().find(|s| s.other == GROUP).expect("group conversation");
            assert!(
                received.lines.iter().all(|line| line.text != "hi builders"),
                "the live buffer evicted the target"
            );
            h.settle();
            assert!(
                h.painted(aurora_i18n::tr!("common-ignore")),
                "{style:?}, sender={sender}: menu survives eviction"
            );
            assert!(
                !h.painted("hi builders"),
                "{style:?}: the transcript no longer draws the evicted row"
            );
            assert!(
                !h.painted("busy incoming 2099"),
                "{style:?}: eviction does not resume automatic following"
            );
            h.press_text(aurora_i18n::tr!("common-ignore"));
            assert!(
                crate::ui::context::take_requests(&h.frames.ctx)
                    .iter()
                    .any(|a| matches!(a, crate::ui::context::CtxAction::ToggleBlock(id) if *id == JANE)),
                "{style:?}: the retained menu still ignores Jane"
            );
            assert!(!egui::Popup::is_any_open(&h.frames.ctx), "{style:?}: the menu closes");
            assert!(h.painted("busy incoming 2099"), "{style:?}: resume at the live end");
        }
    }
}

/// A menu is detached from its message's layout, but closes as soon as
/// its conversation window is dragged, before the mouse is released.
#[test]
fn dragging_the_conversation_closes_its_context_menu() {
    for style in ALL {
        for sender in [false, true] {
            if !style.workspace() && !sender {
                continue;
            }
            let mut h = H::new(style);
            h.world.groups.sessions.entry(GROUP).or_default().group = true;
            h.open(Some(GROUP));
            let at = h.text_rect(if sender { "Jane" } else { "hi builders" }).center();
            h.right_click(at);
            assert!(h.painted(aurora_i18n::tr!("common-ignore")), "{style:?}: menu opened");
            let window = h.frames.ctx.memory(|m| m.area_rect(egui::Id::unique("conversations")).unwrap());
            let grab = h
                .shown(egui::Id::unique("conversations").with("title_double_click"))
                .expect("window title")
                .center();
            h.frame(vec![egui::Event::PointerMoved(grab)]);
            h.frame(vec![egui::Event::PointerButton {
                pos: grab,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]);
            for step in 1..=3 {
                h.frame(vec![egui::Event::PointerMoved(grab + egui::vec2(25.0, 15.0) * step as f32)]);
            }
            let moved = h.frames.ctx.memory(|m| m.area_rect(egui::Id::unique("conversations")).unwrap());
            assert!(
                moved.min.distance(window.min) > 10.0,
                "{style:?}: the real window moved: {window:?} -> {moved:?}, grab={grab:?}, dragged={:?}",
                h.frames.ctx.dragged_id()
            );
            assert!(
                !egui::Popup::is_any_open(&h.frames.ctx),
                "{style:?}: dragging dismisses the menu before release"
            );
            assert!(!h.painted(aurora_i18n::tr!("common-ignore")), "{style:?}: no menu is left behind");
        }
    }
}

/// A resize is a drag, so egui's outside-click dismissal does not run until
/// release. Every edge/corner must dismiss the transcript menu immediately (#589).
#[test]
fn resizing_the_conversation_closes_its_context_menu() {
    type Grab = fn(egui::Rect) -> egui::Pos2;
    let cases: [(&str, Grab, egui::Vec2); 8] = [
        ("left", |r| r.left_center(), egui::vec2(45.0, 0.0)),
        ("right", |r| r.right_center(), egui::vec2(-45.0, 0.0)),
        ("top", |r| r.center_top(), egui::vec2(0.0, 35.0)),
        ("bottom", |r| egui::pos2(r.right() - 40.0, r.bottom()), egui::vec2(0.0, -35.0)),
        ("top-left", |r| r.left_top(), egui::vec2(45.0, 35.0)),
        ("bottom-left", |r| r.left_bottom(), egui::vec2(45.0, -35.0)),
        ("top-right", |r| r.right_top(), egui::vec2(-45.0, 35.0)),
        ("bottom-right", |r| r.right_bottom(), egui::vec2(-45.0, -35.0)),
    ];
    for style in ALL {
        for sender in [false, true] {
            if !style.workspace() && !sender {
                continue;
            }
            for (edge, grab_of, delta) in cases {
                let mut h = H::new(style);
                h.world.groups.sessions.entry(GROUP).or_default().group = true;
                crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(150.0, 100.0), egui::vec2(960.0, 580.0));
                h.open(Some(GROUP));
                let at = h.text_rect(if sender { "Jane" } else { "hi builders" }).center();
                h.right_click(at);
                let menu_label = aurora_i18n::tr!("common-ignore");
                assert!(h.painted(menu_label), "{style:?}, sender={sender}, {edge}: menu opened");
                let window = || {
                    h.frames
                        .ctx
                        .memory(|m| m.area_rect(egui::Id::unique("conversations")).expect("window"))
                };
                let before = window();
                let grab = grab_of(before);
                h.frame(vec![egui::Event::PointerMoved(grab)]);
                assert!(h.painted(menu_label), "{style:?}, sender={sender}, {edge}: hovering keeps the menu");
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                h.frame(vec![button(grab, true)]);
                h.frame(vec![egui::Event::PointerMoved(grab + delta)]);
                let resized = h
                    .frames
                    .ctx
                    .memory(|m| m.area_rect(egui::Id::unique("conversations")).expect("window"));
                assert!(
                    (resized.size() - before.size()).length() > 10.0,
                    "{style:?}, sender={sender}, {edge}: the real window resized: {before:?} -> {resized:?}"
                );
                assert!(h.frames.ctx.input(|i| i.pointer.primary_down()), "the button is still held");
                assert!(
                    !egui::Popup::is_any_open(&h.frames.ctx),
                    "{style:?}, sender={sender}, {edge}: resizing dismisses the menu before release"
                );
                assert!(!h.painted(menu_label), "{style:?}, sender={sender}, {edge}: no menu is left behind");
                h.frame(vec![button(grab + delta, false)]);
                h.settle();
                let at = h.text_rect(if sender { "Jane" } else { "hi builders" }).center();
                h.right_click(at);
                assert!(h.painted(menu_label), "{style:?}, sender={sender}, {edge}: the menu can reopen");
            }
        }
    }
}

/// Copy and Quote use the original message even when incoming traffic has
/// removed it from both the viewport and the bounded live transcript.
#[test]
fn the_message_menu_retains_copy_and_quote_after_eviction() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for quote in [false, true] {
            let mut h = H::new(style);
            h.open(Some(JANE));
            h.right_click(h.text_rect("hello from Jane").center());
            assert!(
                egui::Popup::is_any_open(&h.frames.ctx),
                "{style:?}, quote={quote}: original menu opened"
            );
            assert!(
                h.st.views[&Some(JANE)].is_paused(),
                "{style:?}, quote={quote}: original viewport paused"
            );
            for i in 0..2100 {
                h.world
                    .conversation_line(JANE, line("Bob", BOB, ChatKind::Im, &format!("later message {i}")));
            }
            h.settle();
            assert!(!h.painted("hello from Jane"), "{style:?}: the original message left the transcript");
            assert!(
                h.st.views[&Some(JANE)].below() > 1000.0,
                "{style:?}, quote={quote}: the live end remains below the paused transcript (the sidebar still previews the latest message)"
            );
            if quote {
                h.press_text(aurora_i18n::tr!("ui-chat-quote-message"));
                assert_eq!(
                    h.st.draft_mut(Some(JANE)),
                    "> hello from Jane ",
                    "{style:?}: quote the original message"
                );
            } else {
                let pos = h.text_rect(aurora_i18n::tr!("ui-chat-copy-message")).center();
                let button = |pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                h.frame(vec![egui::Event::PointerMoved(pos)]);
                h.frame(vec![button(true)]);
                let out = h.frame_out(vec![button(false)]);
                assert!(
                    out.commands
                        .iter()
                        .any(|c| matches!(c, egui::OutputCommand::CopyText(text) if text.ends_with("hello from Jane"))),
                    "{style:?}: copy the original message"
                );
            }
            assert!(
                !egui::Popup::is_any_open(&h.frames.ctx),
                "{style:?}: the chosen action dismisses the menu"
            );
        }
    }
}

/// The field (Aurora, Compact): emoji, the volume's menu, inventory,
/// channel, send.
#[test]
fn the_field_acts() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        h.open(None);
        h.press(control_id("box-mood-smile-filled"));
        assert!(h.st.emoji_open, "{style:?}: emoji");
        h.press(control_id("box-mood-smile-filled"));
        assert!(!h.st.emoji_open, "{style:?}: emoji closed");
        // no gestures button: their window, their keys, their trigger
        assert!(h.shown(control_id("box-hand-finger")).is_none(), "{style:?}: no gestures");
        // the channel: hidden by default, "/5" before the text still sends
        // on 5
        h.open(None);
        h.type_text("/5 hidden");
        assert!(h.shown(control_id("box-channel")).is_none(), "{style:?}: no chip by default");
        h.enter(Default::default());
        assert!(
            has(
                &h.take(),
                |a| matches!(a, ConvAction::Local(c) if c.channel == 5 && c.message == "hidden")
            ),
            "{style:?}: /5 without the chip"
        );
        // shown by the preferences: "/5" opening the text, back to 0 from it
        h.st.show_channel = true;
        h.open(None);
        h.type_text("/5 secret");
        assert!(h.painted("/5"), "{style:?}: the chip shows the channel");
        h.press(control_id("box-channel"));
        h.press_text(aurora_i18n::tr!("ui-chat-channel-reset"));
        assert_eq!(h.st.draft_mut(None), "secret", "{style:?}: channel reset");
        // shout from the volume's menu, then say what was typed
        assert!(h.painted(aurora_i18n::tr!("ui-chat-volume-say")), "{style:?}: the volume named");
        h.press(control_id("box-volume"));
        h.press_text(aurora_i18n::tr!("ui-chat-volume-shout"));
        assert_eq!(h.st.volume, Volume::Shout, "{style:?}");
        assert!(
            h.painted(aurora_i18n::tr!("ui-chat-volume-shout")),
            "{style:?}: the button names it"
        );
        h.st.conv_focus_request = true;
        h.settle();
        h.enter(Default::default());
        assert!(
            has(
                &h.take(),
                |a| matches!(a, ConvAction::Local(c) if c.message == "secret" && c.chat_type == aurora_net::ChatType::Shout)
            ),
            "{style:?}: shouted"
        );
        h.press(control_id("box-volume"));
        h.press_text(aurora_i18n::tr!("ui-chat-volume-say"));
        assert_eq!(h.st.volume, Volume::Say, "{style:?}");
        // narrow, the volume stays (its icon), its menu still acts
        crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(460.0, 580.0));
        // (the column eases to the rail)
        for _ in 0..20 {
            h.settle();
        }
        assert!(h.shown(control_id("box-channel")).is_none(), "{style:?}: narrow, no channel chip");
        h.press(control_id("box-volume"));
        h.press_text(aurora_i18n::tr!("ui-chat-volume-whisper"));
        assert_eq!(h.st.volume, Volume::Whisper, "{style:?}: narrow");
        crate::ui::widgets::place_floater(&h.frames.ctx, "conversations", egui::pos2(20.0, 20.0), egui::vec2(960.0, 580.0));
        for _ in 0..20 {
            h.settle();
        }
        // a resident: no volume, no attach button (#627), and the send button
        h.open(Some(JANE));
        assert!(h.shown(control_id("box-volume")).is_none(), "{style:?}: no volume in an IM");
        assert!(
            h.shown(control_id("box-paperclip")).is_none(),
            "{style:?}: no attach button in an IM"
        );
        h.st.conv_focus_request = true;
        h.settle();
        h.type_text("hi Jane");
        h.press(control_id("box-send"));
        assert!(
            has(
                &h.take(),
                |a| matches!(a, ConvAction::Im { to, text } if *to == JANE && text == "hi Jane")
            ),
            "{style:?}: send"
        );
    }
}

/// Inventory dragged with an IM shown (#627): its messages fade behind
/// "Drop here to give to Jane" from the start of the drag, wherever the
/// pointer is; a drop on the conversation gives what is carried; a group's
/// conversation, which takes no inventory, shows no hint.
#[test]
fn an_inventory_drag_says_where_to_drop_it() {
    const ITEM: Uuid = Uuid::from_u128(0x17E);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        let hint = aurora_i18n::tr!("ui-chat-drop-to-give", name = h.world.session_title(&JANE));
        h.open(Some(JANE));
        assert!(!h.painted(&hint), "{style:?}: no hint without a drag");
        // the drag starts outside the window (from the inventory's)
        let away = egui::pos2(4.0, 4.0);
        h.frame(vec![egui::Event::PointerMoved(away), button(away, true)]);
        egui::DragAndDrop::set_payload(&h.frames.ctx, crate::ui::inventory::InvDrag(ITEM));
        for _ in 0..20 {
            h.frame(Vec::new());
        }
        assert!(h.painted(&hint), "{style:?}: the hint shows before the pointer reaches the IM");
        let over = h.text_rect(&hint).center();
        h.frame(vec![egui::Event::PointerMoved(over)]);
        h.frame(vec![button(over, false)]);
        assert!(
            has(
                &h.take(),
                |a| matches!(a, ConvAction::GiveItems { to, items } if *to == JANE && items[..] == [ITEM])
            ),
            "{style:?}: the drop gives"
        );
        for _ in 0..20 {
            h.frame(Vec::new());
        }
        assert!(!h.painted(&hint), "{style:?}: the hint goes with the drag");
        // a group's conversation: no hint, nothing given
        h.open(Some(GROUP));
        let group_hint = aurora_i18n::tr!("ui-chat-drop-to-give", name = h.world.session_title(&GROUP));
        h.frame(vec![egui::Event::PointerMoved(away), button(away, true)]);
        egui::DragAndDrop::set_payload(&h.frames.ctx, crate::ui::inventory::InvDrag(ITEM));
        for _ in 0..20 {
            h.frame(Vec::new());
        }
        assert!(!h.painted(&group_hint), "{style:?}: no hint in a group");
        h.frame(vec![button(away, false)]);
        assert!(
            !has(&h.take(), |a| matches!(a, ConvAction::GiveItems { .. })),
            "{style:?}: nothing given in a group"
        );
        // a resident dropped on the conference still joins it
        const NEW: Uuid = Uuid::from_u128(0xD0);
        h.open(Some(MEETING));
        h.frame(vec![egui::Event::PointerMoved(away), button(away, true)]);
        egui::DragAndDrop::set_payload(&h.frames.ctx, crate::ui::call_invite::PersonDrag(NEW));
        let over = h.text_rect("hi meeting").center();
        h.frame(vec![egui::Event::PointerMoved(over)]);
        h.frame(vec![button(over, false)]);
        assert!(
            has(
                &h.take(),
                |a| matches!(a, ConvAction::AddPeople(c, people) if *c == MEETING && people[..] == [NEW])
            ),
            "{style:?}: a resident joins"
        );
    }
}

/// Exercise the actual pane in every style, including its detached version:
/// the resident drop handler must preserve all three inventory cargo types.
#[test]
fn inventory_drops_give_to_the_resident_in_every_conversation_style() {
    const ITEM: Uuid = Uuid::from_u128(0x17E);
    const FOLDER: Uuid = Uuid::from_u128(0xF01D);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    for style in ALL {
        for detached in [false, true] {
            let mut h = H::new(style);
            h.open(Some(JANE));
            if detached {
                h.st.pop_out(Some(JANE));
                crate::ui::widgets::place_floater(
                    &h.frames.ctx,
                    &super::popout::floater_id(Some(JANE)),
                    egui::pos2(100.0, 100.0),
                    egui::vec2(500.0, 500.0),
                );
                h.settle();
            }
            let draft = "keep this unsent draft";
            h.st.drafts.insert(Some(JANE), draft.into());
            for ids in [vec![ITEM], vec![FOLDER], vec![ITEM, FOLDER]] {
                let over = h.text_rect("hello from Jane").center();
                let away = egui::pos2(4.0, 4.0);
                h.take();
                h.frame(vec![egui::Event::PointerMoved(away), button(away, true)]);
                if ids == [FOLDER] {
                    egui::DragAndDrop::set_payload(&h.frames.ctx, crate::ui::inventory::FolderDrag(FOLDER));
                } else {
                    crate::ui::inventory::drag::demo_carry(&h.frames.ctx, ids.clone());
                }
                h.frame(vec![egui::Event::PointerMoved(over)]);
                assert_eq!(
                    crate::ui::inventory::drag::carried_count(&h.frames.ctx),
                    Some(ids.len()),
                    "{style:?}, detached={detached}: hovering keeps the complete cargo"
                );
                h.frame(vec![button(over, false)]);
                h.settle();
                let actions = h.take();
                assert_eq!(
                    actions.iter().filter(|a| matches!(a, ConvAction::GiveItems { .. })).count(),
                    1,
                    "{style:?}, detached={detached}: exactly one confirmation"
                );
                assert!(
                    has(
                        &actions,
                        |a| matches!(a, ConvAction::GiveItems { to, items } if *to == JANE && *items == ids)
                    ),
                    "{style:?}, detached={detached}: the complete cargo goes to Jane"
                );
                assert!(!has(&actions, |a| matches!(a, ConvAction::AddPeople(..))));
                assert!(!egui::DragAndDrop::has_any_payload(&h.frames.ctx));
                assert_eq!(h.st.drafts[&Some(JANE)], draft);
            }
        }
    }
}

#[test]
fn inventory_drops_do_not_give_to_nearby_groups_or_conferences() {
    const ITEM: Uuid = Uuid::from_u128(0x17E);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    for style in ALL {
        let mut h = H::new(style);
        h.world
            .chat
            .push_back(line("Bob", BOB, ChatKind::Local(aurora_net::ChatType::Normal), "hello nearby"));
        for (conversation, text) in [(None, "hello nearby"), (Some(GROUP), "hi builders"), (Some(MEETING), "hi meeting")] {
            h.open(conversation);
            let over = h.text_rect(text).center();
            let away = egui::pos2(4.0, 4.0);
            h.frame(vec![egui::Event::PointerMoved(away), button(away, true)]);
            egui::DragAndDrop::set_payload(&h.frames.ctx, crate::ui::inventory::InvDrag(ITEM));
            h.frame(vec![egui::Event::PointerMoved(over)]);
            h.frame(vec![button(over, false)]);
            assert!(!has(&h.take(), |a| matches!(a, ConvAction::GiveItems { .. })));
        }
        // Person cargo still follows the invitation path in classic panes.
        const NEW: Uuid = Uuid::from_u128(0xD0);
        h.open(Some(MEETING));
        let over = h.text_rect("hi meeting").center();
        h.frame(vec![egui::Event::PointerMoved(over), button(over, true)]);
        egui::DragAndDrop::set_payload(&h.frames.ctx, crate::ui::call_invite::PersonDrag(NEW));
        h.frame(Vec::new());
        h.frame(vec![button(over, false)]);
        assert!(has(
            &h.take(),
            |a| matches!(a, ConvAction::AddPeople(c, people) if *c == MEETING && people[..] == [NEW])
        ));
    }
}

/// A call in progress (Aurora, Compact): its stage, its strip, the column's
/// panel while another conversation is shown, a participant's menu.
#[test]
fn the_call_controls_act() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for conversation in [BOB, MEETING] {
            let mut h = H::new(style);
            h.open(Some(conversation));
            h.call(conversation);
            let what = format!("{style:?} {conversation}");
            h.press(control_id("call-mic"));
            assert!(has(&h.take(), |a| matches!(a, ConvAction::ToggleMic)), "{what}: mic");
            h.press(control_id("call-output"));
            assert!(
                has(&h.take(), |a| matches!(a, ConvAction::ListOutputDevices)),
                "{what}: outputs listed"
            );
            h.press_text(aurora_i18n::tr!("ui-options-system-default"));
            assert!(
                has(&h.take(), |a| matches!(a, ConvAction::SetOutputDevice(d) if d.is_empty())),
                "{what}: output"
            );
            h.press(control_id("call-add"));
            assert!(
                has(&h.take(), |a| matches!(a, ConvAction::PickPeople(id) if *id == conversation)),
                "{what}: add"
            );
            h.press(person_id(JANE));
            h.press_text(aurora_i18n::tr!("ui-chat-call-mute-for-me"));
            assert!(
                has(&h.take(), |a| matches!(a, ConvAction::SetVolume(id, v) if *id == JANE && *v == 0.0)),
                "{what}"
            );
            h.press(person_id(JANE));
            h.press_text(aurora_i18n::tr!("common-profile"));
            assert!(
                has(&h.take(), |a| matches!(a, ConvAction::Profile(id) if *id == JANE)),
                "{what}: profile"
            );
            h.press(control_id("call-reduce"));
            assert!(h.st.stage_mini, "{what}: reduced");
            h.press(control_id("strip-mic"));
            assert!(has(&h.take(), |a| matches!(a, ConvAction::ToggleMic)), "{what}: strip mic");
            h.press(control_id("strip-expand"));
            assert!(!h.st.stage_mini, "{what}: expanded");
            h.press(control_id("call-hang-up"));
            assert!(has(&h.take(), |a| matches!(a, ConvAction::HangUp)), "{what}: hang up");
            h.st.stage_mini = true;
            h.settle();
            h.press(control_id("strip-hang-up"));
            assert!(has(&h.take(), |a| matches!(a, ConvAction::HangUp)), "{what}: strip hang up");
            // the head's green phone hangs up too
            h.press(control_id("head-phone-filled"));
            assert!(has(&h.take(), |a| matches!(a, ConvAction::HangUp)), "{what}: head hang up");
            // the column's panel, from the nearby chat
            h.open(None);
            h.press(control_id("panel-mic"));
            assert!(has(&h.take(), |a| matches!(a, ConvAction::ToggleMic)), "{what}: panel mic");
            h.press(control_id("panel-hang-up"));
            assert!(has(&h.take(), |a| matches!(a, ConvAction::HangUp)), "{what}: panel hang up");
            h.press(control_id("panel-back"));
            assert_eq!(h.st.selected, Some(conversation), "{what}: back to the call");
        }
    }
}

/// A role group's call adds nobody, from its stage either.
#[test]
fn a_role_group_call_adds_nobody() {
    let mut h = H::new(ChatStyle::Aurora);
    h.open(Some(GROUP));
    h.call(GROUP);
    assert!(h.shown(control_id("call-hang-up")).is_some());
    assert!(h.shown(control_id("call-add")).is_none());
}

/// Invitations (Aurora, Compact): a resident's call is answered or
/// declined, a group's joined or ignored; a group call going on without
/// us is joined.
#[test]
fn invitations_are_answered() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for (p2p, answer) in [
            (true, InviteAnswer::Accept),
            (true, InviteAnswer::Decline),
            (false, InviteAnswer::Accept),
            (false, InviteAnswer::Decline),
        ] {
            let mut h = H::new(style);
            h.open(Some(if p2p { JANE } else { GROUP }));
            h.invite(p2p);
            let what = format!("{style:?} p2p {p2p} {answer:?}");
            if !p2p {
                assert!(h.painted(aurora_i18n::tr!("ui-chat-call-ignore")), "{what}: Ignore");
            }
            h.press(control_id(if answer == InviteAnswer::Accept {
                "call-answer"
            } else {
                "call-decline"
            }));
            assert!(
                has(
                    &h.take(),
                    |a| matches!(a, ConvAction::AnswerCall { answer: got, .. } if *got == answer)
                ),
                "{what}"
            );
        }
        let mut h = H::new(style);
        h.world.calls.launched.insert(GROUP, (JANE, std::time::Instant::now()));
        h.open(Some(GROUP));
        h.press(control_id("call-join"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Call(id) if *id == GROUP)),
            "{style:?}: join"
        );
    }
}

/// A group chat's Snooze, in every style: the head's button beside the
/// close one and the entry's right-click menu; none for an IM or a
/// conference (FSFloaterIM::setCanSnooze).
#[test]
fn a_group_chat_snoozes_from_its_head_and_its_menu() {
    for style in ALL {
        let mut h = H::new(style);
        h.world.groups.sessions.entry(GROUP).or_default().group = true;
        let button = if style.workspace() {
            control_id("head-alarm-snooze-filled")
        } else {
            classic_id("alarm-snooze-filled")
        };
        h.open(Some(GROUP));
        h.press(button);
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Snooze(id) if *id == GROUP)),
            "{style:?}: the head's snooze"
        );
        for other in [BOB, MEETING] {
            h.open(Some(other));
            assert!(h.shown(button).is_none(), "{style:?}: no snooze for {other}");
        }
        // the group's entry (Bob's conversation shown)
        h.open(Some(BOB));
        let name = h.world.session_title(&GROUP);
        let at = h.text_rect(&name).center();
        h.right_click(at);
        h.press_text(aurora_i18n::tr!("ui-chat-snooze"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Snooze(id) if *id == GROUP)),
            "{style:?}: the entry's snooze"
        );
    }
}

/// The window of tabs (Bubbles, Lines): its tabs, its head's buttons, the
/// emoji, find (Ctrl+F) and send still act as before the redesign.
#[test]
fn the_window_of_tabs_still_acts() {
    for style in [ChatStyle::Bubbles, ChatStyle::Legacy] {
        let mut h = H::new(style);
        h.settle();
        // its tabs: Bob's, by its name
        let name = h.world.session_title(&BOB);
        h.press_text(&name);
        assert_eq!(h.st.selected, Some(BOB), "{style:?}: tab");
        h.press(classic_id("user-filled"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Profile(id) if *id == BOB)),
            "{style:?}: profile"
        );
        h.press(classic_id("send-filled"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::OfferTeleport(id) if *id == BOB)),
            "{style:?}: teleport"
        );
        h.press(classic_id("x"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Close(id) if *id == BOB)),
            "{style:?}: close"
        );
        h.press(classic_id("phone-filled"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Call(id) if *id == BOB)),
            "{style:?}: call"
        );
        h.press(classic_id("mood-smile-filled"));
        assert!(h.st.emoji_open, "{style:?}: emoji");
        h.press(classic_id("mood-smile-filled"));
        h.st.conv_focus_request = true;
        h.settle();
        h.frame(vec![egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        h.settle();
        assert!(h.st.searches.get(&Some(BOB)).is_some_and(|s| s.find.is_some()), "{style:?}: Ctrl+F");
        h.st.searches.clear();
        h.st.conv_focus_request = true;
        h.settle();
        h.type_text("yo Bob");
        h.press(classic_id("send"));
        assert!(
            has(
                &h.take(),
                |a| matches!(a, ConvAction::Im { to, text } if *to == BOB && text == "yo Bob")
            ),
            "{style:?}: send"
        );
        // a group: its participants' button
        let group = h.world.session_title(&MEETING);
        h.press_text(&group);
        let before = h.st.show_profile;
        h.press(classic_id("users-group"));
        assert_ne!(h.st.show_profile, before, "{style:?}: participants");
    }
}

/// Ctrl+F opens the find bar in every style.
#[test]
fn find_opens_in_every_style() {
    for style in ALL {
        let mut h = H::new(style);
        h.open(Some(JANE));
        h.frame(vec![egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        h.settle();
        assert!(h.st.searches.get(&Some(JANE)).is_some_and(|s| s.find.is_some()), "{style:?}");
    }
}
