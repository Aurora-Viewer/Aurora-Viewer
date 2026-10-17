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
        h.press(egui::Id::new("conversation_find"));
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
/// and a conference's: call, add people, profile, members, find, and the
/// rows of its "…".
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
        h.press(control_id("head-user-plus"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::PickPeople(id) if *id == BOB)),
            "{style:?}: add"
        );
        h.press(control_id("head-user-filled"));
        assert!(
            has(&h.take(), |a| matches!(a, ConvAction::Profile(id) if *id == BOB)),
            "{style:?}: profile"
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
        // a role group: members, no adding (like Firestorm)
        h.open(Some(GROUP));
        assert!(
            h.shown(control_id("head-user-plus")).is_none(),
            "{style:?}: a role group adds nobody"
        );
        let open = h.st.members_open;
        h.press(control_id("head-users"));
        assert_ne!(h.st.members_open, open, "{style:?}: members");
        // a conference: adding, and its members' panel
        h.open(Some(MEETING));
        h.st.members_open = true;
        h.press(control_id("head-user-plus"));
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
            let id = egui::Id::new(("thread_tool", salt, jane.time, jane.source, jane.text.as_str()));
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
                assert_eq!(
                    h.painted("incoming message 19"),
                    !older,
                    "{style:?}: chat keeps updating with its menu open"
                );
                if older {
                    assert_eq!(
                        h.text_rect(&original),
                        position,
                        "{style:?}: keep the normal manual-scroll position"
                    );
                } else {
                    assert!(!h.painted(&original), "{style:?}: the menu survives its row scrolling out of sight");
                }
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

/// A short conversation keeps updating while its sender menu is open,
/// including new messages from that very same sender.
#[test]
fn incoming_messages_keep_short_chat_context_menus_open() {
    for style in ALL {
        let mut h = H::new(style);
        h.world.groups.sessions.entry(GROUP).or_default().group = true;
        h.open(Some(GROUP));
        h.right_click(h.text_rect("Jane").center());
        for i in 0..3 {
            h.world
                .conversation_line(GROUP, line("Jane", JANE, ChatKind::Im, &format!("short incoming {i}")));
            h.frame(Vec::new());
            assert!(h.painted(aurora_i18n::tr!("common-ignore")), "{style:?}: sender menu stays open");
            assert!(
                h.painted(&format!("short incoming {i}")),
                "{style:?}: the new message appears with its menu open"
            );
        }
        h.history_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!egui::Popup::is_any_open(&h.frames.ctx), "{style:?}: Escape closes the menu");
        assert!(h.painted("short incoming 2"), "{style:?}: new messages appear after closing");
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
                h.painted("busy incoming 2099"),
                "{style:?}: the live end updates with its menu open"
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
            let window = h.frames.ctx.memory(|m| m.area_rect(egui::Id::new("conversations")).unwrap());
            let grab = h
                .shown(egui::Id::new("conversations").with("title_double_click"))
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
            let moved = h.frames.ctx.memory(|m| m.area_rect(egui::Id::new("conversations")).unwrap());
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

/// Copy and Quote use the original message even when incoming traffic has
/// removed it from both the viewport and the bounded live transcript.
#[test]
fn the_message_menu_retains_copy_and_quote_after_eviction() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        for quote in [false, true] {
            let mut h = H::new(style);
            h.open(Some(JANE));
            h.right_click(h.text_rect("hello from Jane").center());
            for i in 0..2100 {
                h.world
                    .conversation_line(JANE, line("Bob", BOB, ChatKind::Im, &format!("later message {i}")));
            }
            h.settle();
            assert!(!h.painted("hello from Jane"), "{style:?}: the original message left the transcript");
            assert!(h.painted("later message 2099"), "{style:?}: the transcript keeps updating");
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
        // a resident: no volume, the paperclip, and the send button
        h.open(Some(JANE));
        assert!(h.shown(control_id("box-volume")).is_none(), "{style:?}: no volume in an IM");
        h.press(control_id("box-paperclip"));
        assert!(has(&h.take(), |a| matches!(a, ConvAction::OpenInventory)), "{style:?}: paperclip");
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
