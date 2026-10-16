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
        } = self;
        let mut open = true;
        frames.run(events, |ui| {
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
                None,
                &mut open,
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
    let field = egui::Id::new(("conv_input", Some(BOB)));
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
    assert!(h.frames.ctx.memory(|m| m.has_focus(egui::Id::new(("conv_input", Some(JANE))))));
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

/// The kinds of the column's sections.
#[test]
fn sessions_fall_in_their_sections() {
    let h = Harness::new(ChatStyle::Aurora);
    assert_eq!(super::super::sidebar::kind(&h.world, &JANE), Kind::Private);
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
    let input = egui::Id::new(("conv_input", Some(JANE)));
    let box_before = h.rect(input);
    h.st.searches.entry(Some(JANE)).or_default().open_find();
    h.settle();
    let field = h.rect(super::super::find::field_id());
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
    lines.push_back(line("Me", Uuid::nil(), ChatKind::Own, "mine"));
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
    lines.push_back(line("Me", Uuid::nil(), ChatKind::Own, "from elsewhere"));
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
            .memory(|m| m.area_rect(egui::Id::new("conversations")))
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
                    .memory(|m| m.area_rect(egui::Id::new("conversations")))
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
