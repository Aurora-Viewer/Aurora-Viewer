use super::*;
use crate::omnifilter::{DEFAULT_SET, Engine, MatchType, Rule, RuleSet};
use crate::scene::avatar::AvatarLibrary;
use aurora_net::NetEvent;
use std::sync::Arc;

const SENDER: ([u8; 4], u16) = ([127, 0, 0, 1], 13000);

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn world(rules: Vec<Rule>) -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = id(9);
    w.mutes.reset(w.agent_id);
    w.omnifilter = Engine::with_sets(vec![RuleSet {
        name: DEFAULT_SET.into(),
        rules,
    }]);
    w.omnifilter.enabled = true;
    w
}

fn rule(f: impl FnOnce(&mut Rule)) -> Rule {
    let mut r = Rule::new("rule");
    r.enabled = true;
    f(&mut r);
    r
}

fn chat(from: &str, source: ChatSourceType, text: &str) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: from.into(),
        source_id: id(1),
        owner_id: id(1),
        source_type: source,
        chat_type: ChatType::Normal,
        region: None,
        position: glam::Vec3::new(128.0, 128.0, 20.0),
        message: text.into(),
    })
}

fn im(dialog: u8, from: &str, text: &str) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: id(2),
        from_name: from.into(),
        to_agent_id: id(9),
        dialog,
        session_id: id(2),
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    })
}

fn dialog(buttons: &[&str]) -> NetEvent {
    NetEvent::ScriptDialog {
        sender: SENDER.into(),
        object_id: id(5),
        object_name: "Greeter".into(),
        owner_name: "Jane Resident".into(),
        owner_id: id(6),
        message: "Welcome!".into(),
        channel: -77,
        buttons: buttons.iter().map(|b| (*b).to_owned()).collect(),
    }
}

fn texts(w: &World) -> Vec<String> {
    w.chat.iter().map(|l| l.text.clone()).collect()
}

fn sent(w: &mut World) -> Vec<NetCommand> {
    w.groups.take_commands()
}

#[test]
fn nearby_chat_is_hidden_replaced_or_passes() {
    let mut w = world(vec![
        rule(|r| {
            r.sender_name = "Shouter".into();
            r.set_source(Source::NearbyChat, true);
        }),
        rule(|r| {
            r.content = "sale".into();
            r.chat_replace = "ad".into();
        }),
    ]);
    w.apply(chat("Shouter", ChatSourceType::Agent, "LOUD"));
    w.apply(chat("Shop", ChatSourceType::Object, "Big SALE today"));
    w.apply(chat("Friend", ChatSourceType::Agent, "hello"));
    // the shouter's rule is for residents' chat only
    w.apply(chat("Shouter", ChatSourceType::Object, "an object named so"));
    assert_eq!(
        texts(&w),
        [
            aurora_i18n::tr!("omnifilter-replacement", replacement = "ad".to_owned()).to_string(),
            "hello".to_owned(),
            "an object named so".to_owned()
        ]
    );
    let logged: Vec<_> = w.omnifilter.log.iter().map(|(_, n)| n.clone()).collect();
    assert_eq!(logged.len(), 2);
}

#[test]
fn the_filter_off_lets_everything_through() {
    let mut w = world(vec![rule(|_| {})]);
    w.omnifilter.enabled = false;
    w.apply(chat("Anyone", ChatSourceType::Agent, "hi"));
    w.apply(dialog(&["Yes"]));
    assert_eq!(texts(&w), ["hi"]);
    assert_eq!(w.notifications.list.len(), 1);
}

#[test]
fn an_instant_message_is_dropped_before_its_conversation_opens() {
    let mut w = world(vec![rule(|r| {
        r.content = r"free\s+L\$".into();
        r.content_match = MatchType::Regex;
        r.set_source(Source::InstantMessage, true);
    })]);
    w.apply(im(0, "Spammer", "free  L$ here"));
    assert!(!w.social.has_session(&id(2)));
    w.apply(im(0, "Spammer", "a plain hello"));
    assert!(w.social.has_session(&id(2)));
}

#[test]
fn an_object_im_is_replaced() {
    let mut w = world(vec![rule(|r| {
        r.set_source(Source::ObjectInstantMessage, true);
        r.chat_replace = "quiet".into();
    })]);
    w.apply(im(19, "Box", "buy me"));
    assert_eq!(
        texts(&w),
        [aurora_i18n::tr!("omnifilter-replacement", replacement = "quiet".to_owned()).to_string()]
    );
}

#[test]
fn a_group_chat_invitation_is_dropped() {
    let mut w = world(vec![rule(|r| r.set_source(Source::GroupChat, true))]);
    w.apply(NetEvent::SessionInvite(aurora_net::SessionInvite {
        session: id(40),
        from: id(2),
        from_name: "Someone".into(),
        message: "hi all".into(),
        session_name: "Club".into(),
        offline: false,
        timestamp: 0,
    }));
    assert!(!w.groups.sessions.contains_key(&id(40)));
    assert_eq!(w.omnifilter.log.len(), 1);
}

#[test]
fn a_matched_dialog_is_not_shown_and_clicks_the_rules_button() {
    let mut w = world(vec![rule(|r| {
        r.sender_name = "Greeter".into();
        r.content = "button_name=Yes".into();
        r.set_source(Source::ScriptDialog, true);
        r.button_reply = "Yes".into();
    })]);
    w.apply(dialog(&["No", "Yes"]));
    assert!(w.notifications.list.is_empty());
    let cmds = sent(&mut w);
    assert!(
        matches!(&cmds[..], [NetCommand::ScriptDialogReply { sender: Some(s), object_id, channel: -77, index: 1, label }]
            if *s == std::net::SocketAddr::from(SENDER) && *object_id == id(5) && label == "Yes"),
        "{cmds:?}"
    );
    // without that button the rule does not match (its content)
    w.apply(dialog(&["Maybe"]));
    assert_eq!(w.notifications.list.len(), 1);
}

#[test]
fn a_button_the_dialog_does_not_have_answers_nothing() {
    // "Ignore" and Client_Side_Ignore answer nothing (index -1)
    for reply in ["Ignore", "Client_Side_Ignore", ""] {
        let mut w = world(vec![rule(|r| r.button_reply = reply.into())]);
        w.apply(dialog(&["Yes"]));
        assert!(w.notifications.list.is_empty(), "{reply:?}: taken");
        assert!(sent(&mut w).is_empty(), "{reply:?}: nothing sent");
    }
}

#[test]
fn client_side_mute_blocks_the_object() {
    let mut w = world(vec![rule(|r| r.button_reply = "Client_Side_Mute".into())]);
    w.apply(dialog(&["Yes"]));
    assert!(w.mutes.is_muted_id(&id(5)));
    assert!(sent(&mut w).iter().all(|c| !matches!(c, NetCommand::ScriptDialogReply { .. })));
}

#[test]
fn a_text_box_always_sends_the_rules_reply() {
    for text in ["my answer", ""] {
        let mut w = world(vec![rule(|r| {
            r.textbox_reply = text.into();
            r.button_reply = "ignored for a text box".into();
        })]);
        w.apply(dialog(&["!!llTextBox!!"]));
        let cmds = sent(&mut w);
        assert!(
            matches!(&cmds[..], [NetCommand::ScriptDialogReply { sender: Some(s), object_id, channel: -77, index: 0, label }]
                if *s == std::net::SocketAddr::from(SENDER) && *object_id == id(5) && label == text),
            "{cmds:?}"
        );
    }
}

#[test]
fn an_automatic_text_box_reply_is_cleaned_and_cut_like_a_typed_one() {
    // CRs become line feeds, and the cut never splits a character
    let long = format!("a\r\nb\r{}", "é".repeat(200));
    let mut w = world(vec![rule(|r| r.textbox_reply = long.clone())]);
    w.apply(dialog(&["!!llTextBox!!"]));
    let cmds = sent(&mut w);
    let [NetCommand::ScriptDialogReply { label, .. }] = &cmds[..] else {
        panic!("{cmds:?}");
    };
    assert!(label.starts_with("a\nb\n"));
    assert!(!label.contains('\r'));
    assert!(label.len() <= crate::world::notifications::TEXT_BOX_MAX_BYTES);
    assert!(label.ends_with('é'), "cut on a character boundary");
}

#[test]
fn the_answer_waits_for_the_rules_delay() {
    let mut w = world(vec![rule(|r| {
        r.button_reply = "Yes".into();
        r.reply_delay = 1.5;
    })]);
    w.apply(dialog(&["Yes"]));
    assert!(sent(&mut w).is_empty());
    let now = Instant::now();
    w.omnifilter_flush(now + Duration::from_millis(500));
    assert!(sent(&mut w).is_empty());
    w.omnifilter_flush(now + Duration::from_secs(2));
    assert_eq!(sent(&mut w).len(), 1);
}

#[test]
fn a_load_url_opens_only_with_go_to_page() {
    let url = |w: &mut World| {
        w.apply(NetEvent::LoadUrl {
            object_name: "Shop".into(),
            object_id: id(7),
            owner_id: id(6),
            owner_is_group: false,
            message: "Our store".into(),
            url: "https://example.invalid/".into(),
        })
    };
    let mut w = world(vec![rule(|r| {
        r.set_source(Source::UrlRequest, true);
        r.content = "example.invalid".into();
        r.button_reply = "Gotopage".into();
    })]);
    url(&mut w);
    assert!(w.notifications.list.is_empty());
    assert_eq!(w.take_omnifilter_urls(), ["https://example.invalid/"]);
    let mut w = world(vec![rule(|r| r.set_source(Source::UrlRequest, true))]);
    url(&mut w);
    assert!(w.notifications.list.is_empty(), "taken, answered Cancel");
    assert!(w.take_omnifilter_urls().is_empty());
}

#[test]
fn the_dialogs_content_is_its_notification_text_with_its_buttons() {
    assert_eq!(
        dialog_content("Jane", "Box", "Pick", &["A".into(), "!!llTextBox!!".into()]),
        "Jane's '<nolink>Box</nolink>'\nPick\nbutton_name=Client_Side_Mute\nbutton_name=Client_Side_Ignore\nbutton_name=A\nbutton_name=!!llTextBox!!"
    );
    assert_eq!(
        load_url_content("https://a.invalid/", "Hi", "Box", id(3), true),
        format!(
            "Load web page https://a.invalid/ ?\n\nHi\n\nFrom object: <nolink>Box</nolink>, owner: secondlife:///app/group/{}/about",
            id(3)
        )
    );
}

#[test]
fn typing_starts_and_stops_are_not_filtered() {
    let mut w = world(vec![rule(|_| {})]);
    let mut c = ChatMessage {
        from_name: "X".into(),
        source_id: id(1),
        owner_id: id(1),
        source_type: ChatSourceType::Agent,
        chat_type: ChatType::StartTyping,
        region: None,
        position: glam::Vec3::ZERO,
        message: String::new(),
    };
    assert!(w.omnifilter_chat(&mut c));
    assert!(w.omnifilter.log.is_empty());
    c.chat_type = ChatType::Whisper;
    assert!(!w.omnifilter_chat(&mut c));
}
