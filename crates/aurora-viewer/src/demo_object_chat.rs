//! AURORA_DEMO_OBJECT_CHAT=aurora|compact|bubbles|legacy|lines[:on]: the
//! nearby chat at the arrival, residents talking among objects (#475): a
//! greeter, a tip jar, a door's emote, a vendor's line said to us with a
//! link, our HUD's owner-say and its script error (#1263; the tip jar's is
//! not ours: not shown). The Conversations window shows the nearby
//! chat in the style named, or `lines` the cards above the chat bar (the
//! window closed). `on` turns the compact object rows on for the run,
//! nothing saved; `im` shows the friend's conversation instead, with a
//! call's and an automatic reply's lines, and `imlines` turns their
//! sub-option on. `inspect` opens the first object's inspector for captures.
//! `unnamed` adds an object with an empty name whose line imitates a
//! resident's ("Nova Example: …"), and `spoof` the same with the mark of
//! unnamed objects on for the run (FSMarkObjects), nothing saved.

use crate::settings::ChatStyle;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetEvent};
use glam::Vec3;
use uuid::Uuid;

/// The scenario of this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scene {
    /// The Conversations window's style; None: the lines above the chat bar.
    pub style: Option<ChatStyle>,
    /// The compact object rows, for the run.
    pub on: bool,
    /// The friend's conversation shown instead of the nearby chat.
    pub im: bool,
    /// The conversations' system lines compacted too, for the run.
    pub im_lines: bool,
    /// Open the greeter's inspector for a visual check.
    pub inspect: bool,
    /// An unnamed object imitating a resident's line.
    pub unnamed: bool,
    /// The same, unnamed objects marked for the run.
    pub spoof: bool,
}

pub fn scene() -> Option<Scene> {
    parse(&std::env::var("AURORA_DEMO_OBJECT_CHAT").ok()?)
}

fn parse(v: &str) -> Option<Scene> {
    let mut flags = v.trim().split(':');
    let style = match flags.next()? {
        "aurora" | "1" => Some(ChatStyle::Aurora),
        "compact" => Some(ChatStyle::Compact),
        "bubbles" => Some(ChatStyle::Bubbles),
        "legacy" => Some(ChatStyle::Legacy),
        "lines" => None,
        _ => return None,
    };
    let flags: Vec<&str> = flags.collect();
    Some(Scene {
        style,
        on: flags.contains(&"on"),
        im: flags.contains(&"im"),
        im_lines: flags.contains(&"imlines"),
        inspect: flags.contains(&"inspect"),
        unnamed: flags.contains(&"unnamed") || flags.contains(&"spoof"),
        spoof: flags.contains(&"spoof"),
    })
}

/// The compact rows are asked for by the scenario.
pub fn on() -> bool {
    scene().is_some_and(|s| s.on)
}

fn object(n: u128) -> Uuid {
    Uuid::from_u128(0x0B1E_C7C4_0000_0000_0000_0000_0000_0100 | n)
}

/// What is said at the arrival, in order.
pub fn events(agent: Uuid) -> Vec<NetEvent> {
    use aurora_i18n::tr;
    let say = |from: Uuid, owner: Uuid, name: &str, text: &str, source_type, chat_type| {
        NetEvent::Chat(ChatMessage {
            from_name: name.into(),
            source_id: from,
            owner_id: owner,
            source_type,
            chat_type,
            region: Some(crate::demo::HANDLE),
            position: Vec3::new(128.0, 128.0, 25.0),
            message: text.into(),
        })
    };
    let (agent_said, object_said) = (ChatSourceType::Agent, ChatSourceType::Object);
    let normal = ChatType::Normal;
    let stranger = Uuid::from_u128(0x0B1E_C7C4_0000_0000_0000_0000_0000_0001);
    // an object named with spaces, whose line reads like a resident's
    let spoofer = scene().is_some_and(|s| s.unnamed).then(|| {
        let imitation = format!("{}: {}", tr!("demo-nova-example"), tr!("demo-object-chat-reply"));
        say(object(6), stranger, "  ", &imitation, object_said, normal)
    });
    let mut events = vec![
        NetEvent::DisplayNames {
            names: vec![aurora_net::AvatarNameData {
                id: stranger,
                username: "demo.owner".into(),
                display_name: tr!("demo-object-chat-owner").into(),
                legacy_first: "Demo".into(),
                legacy_last: "Owner".into(),
                ..Default::default()
            }],
            bad_ids: Vec::new(),
            max_age: None,
        },
        say(
            super::DEMO_NOVA,
            Uuid::nil(),
            tr!("demo-nova-example"),
            tr!("demo-bubbles-local-1"),
            agent_said,
            normal,
        ),
        say(
            object(1),
            stranger,
            tr!("demo-object-chat-greeter"),
            tr!("demo-object-chat-welcome"),
            object_said,
            normal,
        ),
        say(
            super::DEMO_FRIEND,
            Uuid::nil(),
            tr!("demo-orion-example"),
            tr!("demo-object-chat-reply"),
            agent_said,
            normal,
        ),
        say(
            object(2),
            stranger,
            tr!("demo-object-chat-tip-jar"),
            tr!("demo-object-chat-tip"),
            object_said,
            normal,
        ),
        say(
            object(3),
            agent,
            tr!("common-demo-hud"),
            tr!("demo-object-chat-ao"),
            object_said,
            ChatType::OwnerSay,
        ),
        // a script error of our HUD, in red under its warning sign (#1263),
        // and the tip jar's, someone else's, never shown
        say(
            object(3),
            agent,
            tr!("common-demo-hud"),
            tr!("demo-object-chat-script-error"),
            object_said,
            ChatType::Debug,
        ),
        say(
            object(2),
            stranger,
            tr!("demo-object-chat-tip-jar"),
            tr!("demo-object-chat-script-error"),
            object_said,
            ChatType::Debug,
        ),
        say(
            object(4),
            stranger,
            tr!("demo-object-chat-door"),
            tr!("demo-object-chat-door-text"),
            object_said,
            normal,
        ),
        say(
            object(5),
            stranger,
            tr!("demo-object-chat-vendor"),
            tr!("demo-object-chat-direct"),
            object_said,
            ChatType::Other(9),
        ),
        say(
            super::DEMO_NOVA,
            Uuid::nil(),
            tr!("demo-nova-example"),
            tr!("demo-bubbles-local-2"),
            agent_said,
            normal,
        ),
    ];
    events.extend(spoofer);
    events
}

/// The notices of the scenario, said once the world is in: a friend
/// online, then a teleport.
pub fn notices(world: &mut crate::world::World) {
    use crate::world::notice_topics::Topic;
    let online = aurora_i18n::tr!("world-social-is-online", n = aurora_i18n::tr!("demo-nova-example").to_string());
    world.system_notice(online, Topic::People);
    world.system_notice(aurora_i18n::tr!("world-mod-teleporting"), Topic::Teleport);
    // the friend's conversation: a call ended, an automatic reply
    let friend = super::DEMO_FRIEND;
    let ended = aurora_i18n::tr!(
        "world-calls-ended-by-other",
        name = aurora_i18n::tr!("demo-orion-example").to_string()
    );
    world.conversation_notice(friend, ended, Topic::Call);
    let reply = aurora_i18n::tr!(
        "world-status-auto-reply-sent",
        text = aurora_i18n::tr!("demo-object-chat-reply").to_string()
    );
    world.conversation_notice(friend, reply, Topic::Info);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_names_a_style_or_the_lines_and_turns_the_rows_on() {
        let scene = |style, on| {
            Some(Scene {
                style,
                on,
                im: false,
                im_lines: false,
                inspect: false,
                unnamed: false,
                spoof: false,
            })
        };
        assert_eq!(parse("aurora"), scene(Some(ChatStyle::Aurora), false));
        assert_eq!(parse("1"), scene(Some(ChatStyle::Aurora), false));
        assert_eq!(parse("compact:on"), scene(Some(ChatStyle::Compact), true));
        assert_eq!(parse("bubbles:on"), scene(Some(ChatStyle::Bubbles), true));
        assert_eq!(parse("legacy"), scene(Some(ChatStyle::Legacy), false));
        assert_eq!(parse("lines:on"), scene(None, true));
        assert_eq!(parse("nothing"), None);
        let im = parse("aurora:on:im:imlines").expect("a scene");
        assert!(im.on && im.im && im.im_lines);
        assert!(parse("compact:on:inspect").unwrap().inspect);
        let spoof = parse("aurora:spoof").expect("a scene");
        assert!(spoof.unnamed && spoof.spoof);
        let unnamed = parse("aurora:unnamed").expect("a scene");
        assert!(unnamed.unnamed && !unnamed.spoof);
    }
}
