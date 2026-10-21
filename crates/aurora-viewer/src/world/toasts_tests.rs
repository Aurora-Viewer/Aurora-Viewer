//! Toasts of friends online / offline and of messages out of sight, and
//! their entries under the bell (`World::flush_online_notices`,
//! `World::notify_message`, `World::open_from_notification`).

use super::notifications::{Data, Kind, ToastPrefs};
use super::*;

const ME: Uuid = Uuid::from_u128(1);
const JANE: Uuid = Uuid::from_u128(2);
const TOM: Uuid = Uuid::from_u128(3);
const GROUP: Uuid = Uuid::from_u128(0x6200);

fn world() -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = ME;
    w.social.names.insert(JANE, "Jane Doe".into());
    w.social.names.insert(TOM, "Tom Smith".into());
    w.notifications.toasts = ToastPrefs::default();
    // the world is shown, past the login
    w.notifications.started = true;
    w
}

/// A message from `from` in `session`, as `on_im` / a group chat leaves it:
/// the line in the conversation, unread, and the message for the app.
fn receive(w: &mut World, session: Uuid, from: Uuid, text: &str, kind: ImKind, in_sight: bool) {
    let name = w.social.name_of(&from);
    w.session_lines(session);
    w.conversation_push(
        session,
        ChatLine {
            time: SystemTime::now(),
            from: name,
            text: text.into(),
            kind: ChatKind::Im,
            source: from,
            origin: Origin::Live,
        },
        true,
    );
    let m = ImMessage {
        kind,
        session,
        new_session: false,
    };
    w.notify_message(&m, in_sight);
}

fn messages(w: &World) -> Vec<(Uuid, usize, String)> {
    w.notifications
        .list
        .iter()
        .filter_map(|n| match n.data {
            Data::Message { session, count, .. } => Some((session, count, n.body.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn nearby_chat_does_not_raise_the_conversations_button_count() {
    let mut w = world();
    w.push_chat(ChatLine {
        time: SystemTime::now(),
        from: "Tom Smith".into(),
        text: "hello everyone".into(),
        kind: ChatKind::Local(aurora_net::ChatType::Normal),
        source: TOM,
        origin: Origin::Live,
    });
    // the conversation list still counts it on its nearby chat row
    assert_eq!(w.chat_unread, 1);
    assert_eq!(w.conversations_badge(), 0);
    receive(&mut w, JANE, JANE, "hi", ImKind::Private, false);
    receive(&mut w, GROUP, TOM, "meeting", ImKind::Group, false);
    assert_eq!(w.conversations_badge(), 2);
}

/// The minimized Conversations floater's outline (#293) follows the pill of
/// its button: off with unread nearby chat only, on for an IM.
#[test]
fn nearby_chat_alone_lights_neither_the_outline_nor_the_pill() {
    let mut w = world();
    w.push_chat(ChatLine {
        time: SystemTime::now(),
        from: "Tom Smith".into(),
        text: "hello everyone".into(),
        kind: ChatKind::Local(aurora_net::ChatType::Normal),
        source: TOM,
        origin: Origin::Live,
    });
    assert!(w.chat_unread > 0);
    assert_eq!(w.conversations_badge(), 0);
    assert!(!crate::ui::chat::conversations_waiting(&w));
    receive(&mut w, JANE, JANE, "hi", ImKind::Private, false);
    assert!(crate::ui::chat::conversations_waiting(&w));
}

#[test]
fn a_friend_online_is_a_toast_that_never_enters_the_bell() {
    let mut w = world();
    w.social.form_friendship(JANE);
    w.social.set_online(&[JANE], true);
    w.flush_online_notices();
    // the toast is there, waiting for the interface to show it
    let n = w
        .notifications
        .list
        .iter()
        .find(|n| n.kind == Kind::Presence)
        .expect("a presence toast");
    assert!(n.transient && n.opens());
    // but nothing is listed nor counted under the bell
    assert_eq!(w.notifications.unread(), 0);
    assert_eq!(w.notifications.listed().count(), 0);
    w.notifications.mark_all_read();
    assert_eq!(w.notifications.listed().count(), 0);
    // and no line of nearby chat by default (OnlineOfflinetoNearbyChat off)
    assert!(!w.chat.iter().any(|l| l.kind == ChatKind::System && l.text.contains("Jane Doe")));
    // going offline: a toast that opens nothing
    w.social.set_online(&[JANE], false);
    w.flush_online_notices();
    let off = w.notifications.list.iter().rev().find(|n| n.kind == Kind::Presence).unwrap();
    assert!(matches!(off.data, Data::Presence { online: false, .. }) && !off.opens());
    assert_eq!(w.notifications.unread(), 0);
}

#[test]
fn friends_announced_during_login_get_no_toast_but_keep_their_line() {
    let mut w = world();
    w.notifications.toasts.presence_in_chat = true;
    w.notifications.started = false;
    w.social.form_friendship(JANE);
    w.social.form_friendship(TOM);
    // the friends already online, announced while the loading screen shows
    w.social.set_online(&[JANE], true);
    w.flush_online_notices();
    assert!(!w.notifications.list.iter().any(|n| n.kind == Kind::Presence));
    assert!(w.chat.iter().any(|l| l.kind == ChatKind::System && l.text.contains("Jane Doe")));
    // once the world is shown, a friend logging in gets their toast
    w.notifications.started = true;
    w.social.set_online(&[TOM], true);
    w.flush_online_notices();
    let toasts: Vec<_> = w.notifications.list.iter().filter(|n| n.kind == Kind::Presence).collect();
    assert_eq!(toasts.len(), 1);
    assert!(matches!(toasts[0].data, Data::Presence { who: TOM, online: true }));
}

#[test]
fn the_nearby_chat_line_of_a_friend_follows_its_own_preference() {
    // off by default, the toast on by default: they do not depend on each other
    let d = crate::settings::Settings::default();
    assert!(d.toast_friends_online && !d.friends_online_in_chat);
    assert!(ToastPrefs::default().presence && !ToastPrefs::default().presence_in_chat);
    for (toast, in_chat) in [(true, false), (true, true), (false, true), (false, false)] {
        let mut w = world();
        w.notifications.toasts.presence = toast;
        w.notifications.toasts.presence_in_chat = in_chat;
        w.social.form_friendship(JANE);
        w.social.set_online(&[JANE], true);
        w.flush_online_notices();
        let line = w.chat.iter().any(|l| l.kind == ChatKind::System && l.text.contains("Jane Doe"));
        let toast_shown = w.notifications.list.iter().any(|n| n.kind == Kind::Presence);
        assert_eq!((toast_shown, line), (toast, in_chat), "toast {toast}, in chat {in_chat}");
    }
}

#[test]
fn friend_toasts_follow_the_preference_dnd_and_blocks() {
    for (pref, dnd, blocked) in [(false, false, false), (true, true, false), (true, false, true)] {
        let mut w = world();
        w.notifications.toasts.presence = pref;
        w.status.dnd = dnd;
        w.social.form_friendship(JANE);
        if blocked {
            let _ = w.block(JANE, "Jane Doe", mutes::MuteType::Agent);
        }
        w.social.set_online(&[JANE], true);
        w.flush_online_notices();
        assert!(
            !w.notifications.list.iter().any(|n| n.kind == Kind::Presence),
            "pref {pref}, dnd {dnd}, blocked {blocked}"
        );
    }
}

#[test]
fn messages_of_one_conversation_fill_one_entry_until_read() {
    let mut w = world();
    w.notifications.toasts.im_in_bell = true;
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, false);
    receive(&mut w, JANE, JANE, "are you there?", ImKind::Private, false);
    receive(&mut w, TOM, TOM, "hi", ImKind::Private, false);
    let m = messages(&w);
    assert_eq!(m.len(), 2, "{m:?}");
    // the newest message of Jane, and how many came since it was read
    assert!(m.contains(&(JANE, 2, "are you there?".into())), "{m:?}");
    assert!(m.contains(&(TOM, 1, "hi".into())), "{m:?}");
    // listed and counted under the bell, unlike a friend's notice
    assert_eq!(w.notifications.unread(), 2);
    // Jane's conversation read in the Conversations window
    w.social.ims.iter_mut().find(|s| s.other == JANE).unwrap().unread = 0;
    w.acknowledge_read_messages();
    assert_eq!(messages(&w), vec![(TOM, 1, "hi".into())]);
}

/// By default a message is a toast only, as the Legacy Viewer's
/// notification well never lists IMs (#752): one card per conversation
/// still, neither listed nor counted, never in the history once read.
#[test]
fn by_default_a_message_toasts_without_staying_under_the_bell() {
    let mut w = world();
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, false);
    receive(&mut w, JANE, JANE, "are you there?", ImKind::Private, false);
    assert_eq!(messages(&w), vec![(JANE, 2, "are you there?".into())]);
    assert!(w.notifications.list.iter().all(|n| n.transient));
    assert_eq!((w.notifications.listed().count(), w.notifications.unread()), (0, 0));
    // kept under the bell, then read: the history does not keep it either
    let mut w = world();
    w.notifications.toasts.im_in_bell = true;
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, false);
    assert!(w.notifications.take_departures().is_empty());
    w.social.ims.iter_mut().find(|s| s.other == JANE).unwrap().unread = 0;
    w.acknowledge_read_messages();
    assert!(messages(&w).is_empty());
    assert!(w.notifications.take_departures().is_empty());
}

#[test]
fn no_message_entry_in_sight_in_dnd_from_a_blocked_resident_or_when_turned_off() {
    // the conversation shown in the window
    let mut w = world();
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, true);
    assert!(messages(&w).is_empty());
    // do not disturb (notify_of_message)
    let mut w = world();
    w.status.dnd = true;
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, false);
    assert!(messages(&w).is_empty());
    // Aurora's paused bell keeps it in "To do", silently, when messages
    // stay under it; a toast alone has nothing to show
    let mut w = world();
    w.notifications.aurora = true;
    w.notifications.toasts.im_in_bell = true;
    w.status.dnd = true;
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, false);
    assert_eq!(messages(&w).len(), 1);
    w.notifications.toasts.im_in_bell = false;
    receive(&mut w, TOM, TOM, "hi", ImKind::Private, false);
    assert_eq!(messages(&w).len(), 1);
    // a blocked resident in a group chat
    let mut w = world();
    w.notifications.toasts.group_im = true;
    let _ = w.block(TOM, "Tom Smith", mutes::MuteType::Agent);
    receive(&mut w, GROUP, TOM, "spam", ImKind::Group, false);
    assert!(messages(&w).is_empty());
    // IM toasts turned off
    let mut w = world();
    w.notifications.toasts.im = false;
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, false);
    assert!(messages(&w).is_empty());
}

#[test]
fn group_chats_toast_only_when_their_preference_is_on() {
    // off by default (EnableGroupChatPopups)
    let mut w = world();
    receive(&mut w, GROUP, TOM, "meeting at 9", ImKind::Group, false);
    assert!(messages(&w).is_empty());
    let mut w = world();
    w.notifications.toasts.group_im = true;
    receive(&mut w, GROUP, TOM, "meeting at 9", ImKind::Group, false);
    // the card names the sender in its text
    assert_eq!(messages(&w), vec![(GROUP, 1, "Tom Smith: meeting at 9".into())]);
    // conferences follow the IM preference
    let mut w = world();
    w.notifications.toasts.im = false;
    w.notifications.toasts.group_im = true;
    receive(&mut w, TOM, TOM, "conference", ImKind::Conference, false);
    assert!(messages(&w).is_empty());
}

#[test]
fn clicking_a_message_card_reads_its_conversation_and_focuses_its_tab() {
    let mut w = world();
    w.notifications.toasts.group_im = true;
    receive(&mut w, JANE, JANE, "hello", ImKind::Private, false);
    receive(&mut w, GROUP, TOM, "meeting at 9", ImKind::Group, false);
    for session in [GROUP, JANE] {
        let id = w
            .notifications
            .list
            .iter()
            .find(|n| matches!(n.data, Data::Message { session: s, .. } if s == session))
            .unwrap()
            .id;
        assert_eq!(w.open_from_notification(id), Some(session));
        // acknowledged: gone from the bell, the conversation read, its tab asked
        assert!(!w.notifications.list.iter().any(|n| n.id == id));
        assert_eq!(w.social.ims.iter().find(|s| s.other == session).unwrap().unread, 0);
        assert_eq!(w.social.focus_im.take(), Some(session));
    }
    assert_eq!(w.notifications.unread(), 0);
}

#[test]
fn clicking_a_friend_online_starts_an_im_and_offline_opens_nothing() {
    let mut w = world();
    let on = w.notifications.push_presence(JANE, "Jane Doe is online.", true);
    let off = w.notifications.push_presence(TOM, "Tom Smith is offline.", false);
    assert_eq!(w.open_from_notification(on), Some(JANE));
    assert!(w.social.has_session(&JANE));
    assert_eq!(w.social.focus_im.take(), Some(JANE));
    assert_eq!(w.open_from_notification(off), None);
    assert!(w.social.focus_im.is_none());
}
