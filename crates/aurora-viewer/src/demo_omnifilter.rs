//! AURORA_DEMO_OMNIFILTER=1: the Omnifilter (`crate::omnifilter`,
//! `world/omnifilter.rs`) on for this run with rules of its own, never
//! saved, and the Rules Editor open. From frame 700, nearby chat, an IM and
//! two script dialogs arrive: the shouter's line is hidden, the
//! advertising line is replaced, the "free L$" IM is dropped, the greeter's
//! menu is answered "Yes" after a second without being shown, the other
//! menu and the ordinary lines arrive. The log has one "matched" line per
//! rule that took a message, and the editor's log lists them.

use crate::omnifilter::{MatchType, Rule, RuleSet, Settings, Source};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage, NetEvent};
use glam::Vec3;
use uuid::Uuid;

/// The frame the editor opens.
pub const OPEN: u64 = 640;

pub fn on() -> bool {
    std::env::var_os("AURORA_DEMO_OMNIFILTER").is_some()
}

/// On, the demo's rule set, for this run.
pub fn settings() -> Option<Settings> {
    on().then_some(Settings {
        enabled: true,
        rule_set: 0,
    })
}

const SHOUTER: &str = "Shouty McShout";
const ADVERTISER: &str = "Ad Board";
const GREETER: &str = "Greeter Board";
const FOUNTAIN: &str = "Fountain";
const SPAMMER: &str = "Lucky Spammer";

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x0F11_7E40_0000_0000_0000_0000_0000_0000 | n)
}

fn rule(name: &str, f: impl FnOnce(&mut Rule)) -> Rule {
    let mut r = Rule::new(name);
    r.enabled = true;
    f(&mut r);
    r
}

/// The demo's rules (never saved: the engine has no file).
pub fn rules() -> Option<Vec<RuleSet>> {
    if !on() {
        return None;
    }
    let rules = vec![
        rule(aurora_i18n::tr!("demo-omnifilter-rule-shouter"), |r| {
            r.sender_name = SHOUTER.into();
            r.set_source(Source::NearbyChat, true);
        }),
        rule(aurora_i18n::tr!("demo-omnifilter-rule-ads"), |r| {
            r.content = aurora_i18n::tr!("demo-omnifilter-ad-word").into();
            r.set_source(Source::NearbyChat, true);
            r.set_source(Source::ObjectChat, true);
            r.chat_replace = aurora_i18n::tr!("demo-omnifilter-ad-hidden").into();
        }),
        rule(aurora_i18n::tr!("demo-omnifilter-rule-im"), |r| {
            r.content = r"free\s+L\$".into();
            r.content_match = MatchType::Regex;
            r.set_source(Source::InstantMessage, true);
        }),
        rule(aurora_i18n::tr!("demo-omnifilter-rule-greeter"), |r| {
            r.sender_name = GREETER.into();
            r.sender_match = MatchType::Exact;
            r.set_source(Source::ScriptDialog, true);
            r.button_reply = "Yes".into();
            r.reply_delay = 1.0;
        }),
        {
            let mut r = rule(aurora_i18n::tr!("demo-omnifilter-rule-off"), |_| {});
            r.enabled = false;
            r
        },
    ];
    Some(vec![RuleSet {
        name: crate::omnifilter::DEFAULT_SET.into(),
        rules,
    }])
}

fn chat(from: &str, n: u128, source: ChatSourceType, text: &str) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: from.into(),
        source_id: id(n),
        owner_id: id(n),
        source_type: source,
        chat_type: ChatType::Normal,
        region: None,
        position: Vec3::new(128.0, 128.0, 21.0),
        message: text.into(),
    })
}

fn dialog(object: &str, n: u128, message: &str, buttons: &[&str]) -> NetEvent {
    NetEvent::ScriptDialog {
        sender: ([127, 0, 0, 1], 13000).into(),
        object_id: id(n),
        object_name: object.into(),
        owner_name: aurora_i18n::tr!("demo-violet-wolf").into(),
        owner_id: super::DEMO_FRIEND,
        message: message.into(),
        channel: -4300 - n as i32,
        buttons: buttons.iter().map(|b| (*b).to_owned()).collect(),
    }
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    let ev = match frame {
        700 => chat(SHOUTER, 1, ChatSourceType::Agent, aurora_i18n::tr!("demo-omnifilter-shout")),
        730 => chat(
            ADVERTISER,
            2,
            ChatSourceType::Object,
            &aurora_i18n::tr!("demo-omnifilter-ad", word = aurora_i18n::tr!("demo-omnifilter-ad-word").to_owned()),
        ),
        760 => chat(
            aurora_i18n::tr!("demo-violet-wolf"),
            3,
            ChatSourceType::Agent,
            aurora_i18n::tr!("demo-omnifilter-hello"),
        ),
        790 => NetEvent::InstantMessage(InstantMessage {
            from_agent_id: id(4),
            from_name: SPAMMER.into(),
            to_agent_id: agent,
            dialog: 0,
            session_id: id(4),
            message: "Get free  L$ now!".into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }),
        820 => dialog(GREETER, 5, aurora_i18n::tr!("demo-omnifilter-greeter"), &["Yes", "No"]),
        850 => dialog(FOUNTAIN, 6, aurora_i18n::tr!("demo-choose-a-color-for-the-water"), &["1", "2", "3"]),
        _ => return Vec::new(),
    };
    vec![ev]
}
