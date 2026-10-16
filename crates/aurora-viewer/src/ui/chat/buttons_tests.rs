//! Every control of the Conversations window, pressed through real egui
//! frames in the four styles: what it asks of the app (`ConvAction`), what
//! it changes in the window, or the menu it opens.

use super::members::member_id;
use super::sidebar::{Pinned, control_id, entry_id, search_id};
use super::stage::person_id;
use super::{ChatUi, ConvAction, Start, classic_id, compose::Volume, show, thread};
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
        let rect = self
            .texts
            .iter()
            .rev()
            .find(|t| t.source == source)
            .unwrap_or_else(|| panic!("{source:?} is not painted"))
            .rect;
        self.at(rect.center());
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

/// The toolbar of a line under the pointer: copy, profile, its menu.
#[test]
fn the_line_toolbar_acts() {
    for style in [ChatStyle::Aurora, ChatStyle::Compact] {
        let mut h = H::new(style);
        h.open(Some(JANE));
        let text = h.texts.iter().rev().find(|t| t.source == "hello from Jane").expect("the line").rect;
        h.frame(vec![egui::Event::PointerMoved(text.center())]);
        h.frame(Vec::new());
        let jane = h.world.social.ims.iter().find(|s| s.other == JANE).expect("Jane").lines[0].clone();
        let copy = h.shown(thread::tool_id("copy", &jane)).expect("copy");
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.frame(vec![egui::Event::PointerMoved(copy.center())]);
        h.frame(vec![button(copy.center(), true)]);
        let out = h.frame_out(vec![button(copy.center(), false)]);
        assert!(
            out.commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t.ends_with("hello from Jane"))),
            "{style:?}: copy"
        );
        h.settle();
        h.frame(vec![egui::Event::PointerMoved(text.center())]);
        h.frame(Vec::new());
        h.press(thread::tool_id("profile", &jane));
        assert_eq!(
            crate::ui::profile::take_open_request(&h.frames.ctx),
            Some(JANE),
            "{style:?}: profile"
        );
        h.frame(vec![egui::Event::PointerMoved(text.center())]);
        h.frame(Vec::new());
        h.press(thread::tool_id("more", &jane));
        assert!(h.painted(aurora_i18n::tr!("common-view-profile")), "{style:?}: the author's menu");
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
        // the channel: "/5" opening the text, back to 0 from the chip
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
