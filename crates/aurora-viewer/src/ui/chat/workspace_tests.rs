//! Aurora's workspace through real egui frames: the column, the toolbar of
//! a line, the members' panel, the rail and the window that stays in place.

use super::super::sidebar::{Kind, Pinned, entry_id};
use super::super::{ChatUi, ConvAction, show, thread};
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

/// The toolbar of a message under the pointer: Quote puts "> text" in the
/// field, which takes the keyboard.
#[test]
fn a_line_is_quoted_from_its_toolbar() {
    let mut h = Harness::new(ChatStyle::Aurora);
    h.st.selected = Some(JANE);
    let texts = h.settle();
    // the line (the column's sub-line says it too, further left)
    let text = last(&texts, "hello from Jane");
    h.frame(vec![egui::Event::PointerMoved(text.center())]);
    h.frame(Vec::new());
    let jane = h.world.social.ims.iter().find(|s| s.other == JANE).expect("Jane's conversation");
    let quote = h.rect(thread::tool_id("quote", &jane.lines[0].clone()));
    h.click(quote.center());
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
/// alone, no "Groups" nor "Conferences" head, nor their "+" (the human's
/// choice of 2026-10-16).
#[test]
fn empty_sections_are_not_drawn() {
    let mut h = Harness::new(ChatStyle::Aurora);
    let texts = h.settle();
    let head = |key: &'static str| aurora_i18n::text(key).to_uppercase();
    assert!(texts.iter().any(|t| t.source == head("ui-chat-section-private")), "the IMs' head");
    for key in ["ui-chat-section-groups", "ui-chat-section-conferences"] {
        assert!(texts.iter().all(|t| t.source != head(key)), "{key} drawn while empty");
    }
    let add = |section: &str| super::super::sidebar::control_id(&format!("section-add-{section}"));
    assert!(h.frames.ctx.read_response(add("Private")).is_some());
    assert!(h.frames.ctx.read_response(add("Group")).is_none());
    assert!(h.frames.ctx.read_response(add("Conference")).is_none());
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
