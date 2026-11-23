//! AURORA_DEMO_CONV_OPTIONS=aurora|compact|bubbles|legacy[:on]: the
//! conversation options of Preferences › Chat
//! (`ui/chat/conversation_options.rs`) without a grid, the Conversations
//! window closed and placed at a known size, in that chat style.
//!
//! - At login: Nova's conversation, its history from the transcript (two
//!   lines of yesterday, greyed), then our lines (one an emote), hers and a
//!   system line ("Teleport offer sent to …").
//! - The offline messages asked once logged in: two of Orion's and one for
//!   the Plaza Market group, kept while we were away.
//! - Frame 640: Aurora Builders' chat, where Tess Touch moderates; frame
//!   660: a conference; frame 700: Tess Touch writes privately; frame 760:
//!   the moderator and we write in Aurora Builders.
//!
//! `on`: every option on for the run, nothing saved: tabs ordered by kind,
//! names as "username (display name)", moderators' names bold and their text
//! in italics, "(You)", brackets, history faded to 0.4, and the window
//! opening on the offline messages. `group` or `nova` opens that
//! conversation at frame 780.

use super::{DEMO_FRIEND, DEMO_GROUP1, DEMO_GROUP3, DEMO_NOVA};
use crate::ui::chat::conversation_options::{FontStyle, Settings, TabNames, TabOrder};
use crate::world::{ChatKind, ChatLine, Origin, World};
use aurora_net::NetEvent;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// Orion Example (the demo's names follow the id's last byte).
pub const ORION: Uuid = Uuid::from_u128(0xC0_0000_0000_0000_0000_0000_0000_0202);
/// The conference of frame 660.
pub const CONFERENCE: Uuid = Uuid::from_u128(0xC0_0000_0000_0000_0000_0000_0000_0C0F);

/// The frame `group` or `nova` selects its conversation, after the lines of
/// frame 761.
pub const SHOW_AT: u64 = 780;

/// The scenario: its chat style, whether every option is on, and the
/// conversation selected at `SHOW_AT` (`group`: Aurora Builders, `nova`:
/// Nova's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scene {
    pub style: crate::settings::ChatStyle,
    pub on: bool,
    pub show: Option<Uuid>,
    pub paging: bool,
}

pub fn scene() -> Option<Scene> {
    parse(&std::env::var("AURORA_DEMO_CONV_OPTIONS").ok()?)
}

fn parse(v: &str) -> Option<Scene> {
    use crate::settings::ChatStyle;
    let mut flags = v.trim().split(':');
    let style = match flags.next()? {
        "compact" => ChatStyle::Compact,
        "bubbles" => ChatStyle::Bubbles,
        "legacy" => ChatStyle::Legacy,
        _ => ChatStyle::Aurora,
    };
    let mut scene = Scene {
        style,
        on: false,
        show: None,
        paging: false,
    };
    for flag in flags {
        match flag {
            "on" => scene.on = true,
            "group" => scene.show = Some(DEMO_GROUP1),
            "nova" => scene.show = Some(DEMO_FRIEND),
            "paging" => scene.paging = true,
            _ => {}
        }
    }
    Some(scene)
}

/// `on`: the options of the run.
pub fn force(s: &mut Settings) {
    if !scene().is_some_and(|s| s.on) {
        return;
    }
    *s = Settings {
        auto_order: true,
        order: TabOrder::GroupsConferencesIms,
        new_first: false,
        tab_names: TabNames::UsernameAndDisplay,
        highlight_moderators: true,
        moderator_name: FontStyle::Bold,
        moderator_text: FontStyle::Italic,
        show_you: true,
        system_brackets: true,
        history_fade: 0.4,
        open_on_offline: true,
    };
}

fn line(time: SystemTime, from: &str, source: Uuid, kind: ChatKind, origin: Origin, text: &str) -> ChatLine {
    ChatLine {
        time,
        from: from.to_owned(),
        text: text.to_owned(),
        kind,
        source,
        origin,
        blocked: false,
        beyond_range: false,
        speaker: Default::default(),
    }
}

/// Nova's conversation, written once the login is in.
pub fn lines(world: &mut World) {
    use aurora_i18n::tr;
    let now = SystemTime::now();
    let ago = |minutes: u64| now - Duration::from_secs(minutes * 60);
    let me = world.own_name();
    let nova = "Nova Example";
    let yesterday = 26 * 60;
    for l in [
        line(
            ago(yesterday),
            nova,
            DEMO_FRIEND,
            ChatKind::Im,
            Origin::Log,
            tr!("demo-bubbles-cant"),
        ),
        line(
            ago(yesterday - 2),
            &me,
            Uuid::nil(),
            ChatKind::OWN,
            Origin::Log,
            tr!("demo-bubbles-sorry"),
        ),
        line(ago(9), &me, Uuid::nil(), ChatKind::OWN, Origin::Live, tr!("demo-bubbles-free")),
        line(
            ago(8),
            nova,
            DEMO_FRIEND,
            ChatKind::Im,
            Origin::Live,
            tr!("demo-come-see-the-lagoon-at-sunset"),
        ),
        line(ago(7), &me, Uuid::nil(), ChatKind::OWN, Origin::Live, "/me waves"),
        line(
            ago(6),
            "",
            Uuid::nil(),
            ChatKind::System,
            Origin::Live,
            &tr!("common-teleport-offer-sent", name = nova),
        ),
        line(
            ago(5),
            nova,
            DEMO_FRIEND,
            ChatKind::Im,
            Origin::Live,
            tr!("demo-yes-beautiful-at-sunset"),
        ),
    ] {
        world.conversation_line(DEMO_FRIEND, l);
    }
}

fn im(agent: Uuid, from: Uuid, name: &str, dialog: u8, session: Uuid, text: &str, offline: bool) -> NetEvent {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
        .saturating_sub(if offline { 3 * 3600 } else { 0 });
    NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: from,
        from_name: name.into(),
        to_agent_id: agent,
        dialog,
        session_id: session,
        message: text.into(),
        offline,
        from_group: false,
        binary_bucket: if dialog == 17 { b"Aurora Builders\0".to_vec() } else { Vec::new() },
        timestamp,
    })
}

fn invite(session: Uuid, name: &str, text: &str, offline: bool) -> NetEvent {
    NetEvent::SessionInvite(aurora_net::SessionInvite {
        session,
        from: DEMO_NOVA,
        from_name: "Tess Touch".into(),
        message: text.into(),
        session_name: name.into(),
        offline,
        timestamp: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32)
            .saturating_sub(if offline { 3600 } else { 0 }),
    })
}

/// What the offline message request answers.
pub fn offline_messages(agent: Uuid) -> Vec<NetEvent> {
    use aurora_i18n::tr;
    vec![
        im(agent, ORION, "Orion Example", 0, Uuid::nil(), tr!("demo-chatlog-offline-1"), true),
        im(agent, ORION, "Orion Example", 0, Uuid::nil(), tr!("demo-chatlog-offline-3"), true),
        invite(DEMO_GROUP3, tr!("common-plaza-market"), tr!("demo-chatlog-offline-2"), true),
    ]
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use aurora_i18n::tr;
    match frame {
        640 => vec![invite(
            DEMO_GROUP1,
            "Aurora Builders",
            tr!("demo-has-anyone-seen-the-new-eep-sky"),
            false,
        )],
        660 => vec![invite(CONFERENCE, "Sunset Crew", tr!("demo-bubbles-join"), false)],
        700 => vec![im(
            agent,
            DEMO_NOVA,
            "Tess Touch",
            0,
            Uuid::nil(),
            tr!("demo-arriving-with-textures"),
            false,
        )],
        // Tess Touch moderates Aurora Builders (the demo's session reply)
        760 => vec![im(
            agent,
            DEMO_NOVA,
            "Tess Touch",
            17,
            DEMO_GROUP1,
            tr!("demo-yes-beautiful-at-sunset"),
            false,
        )],
        _ => Vec::new(),
    }
}

/// Our line in Aurora Builders, after the moderator's.
pub fn own_lines(world: &mut World, frame: u64) {
    if frame != 761 {
        return;
    }
    let me = world.own_name();
    world.conversation_line(
        DEMO_GROUP1,
        line(
            SystemTime::now(),
            &me,
            Uuid::nil(),
            ChatKind::OWN,
            Origin::Live,
            aurora_i18n::tr!("demo-bubbles-free"),
        ),
    );
}

/// Numbered, scrollable transcripts for testing Page Up and Page Down.
pub fn paging_lines(world: &mut World) {
    let start = SystemTime::now() - Duration::from_secs(3600);
    for n in 1..=40 {
        let time = start + Duration::from_secs(n * 30);
        let text = aurora_i18n::tr!("demo-scroll-earlier", n = n as i64);
        let message = line(time, "Tess Touch", DEMO_NOVA, ChatKind::Im, Origin::Live, &text);
        for session in [DEMO_FRIEND, DEMO_GROUP1, CONFERENCE] {
            world.conversation_line(session, message.clone());
        }
        let _ = world.apply(NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Tess Touch".into(),
            source_id: DEMO_NOVA,
            owner_id: Uuid::nil(),
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: aurora_net::ChatType::Normal,
            position: world.agent.position,
            message: text,
            region: Some(super::HANDLE),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ChatStyle;

    #[test]
    fn the_scenario_reads_its_style_and_flag() {
        assert_eq!(
            parse("legacy:on"),
            Some(Scene {
                style: ChatStyle::Legacy,
                on: true,
                show: None,
                paging: false,
            })
        );
        assert_eq!(
            parse("1"),
            Some(Scene {
                style: ChatStyle::Aurora,
                on: false,
                show: None,
                paging: false,
            })
        );
        assert_eq!(parse("legacy:group:on").map(|s| (s.on, s.show)), Some((true, Some(DEMO_GROUP1))));
        assert_eq!(parse("aurora:nova").and_then(|s| s.show), Some(DEMO_FRIEND));
        assert_eq!(parse("compact").map(|s| s.style), Some(ChatStyle::Compact));
        assert!(parse("compact:paging").is_some_and(|s| s.paging));
        assert_eq!(parse("bubbles:on").map(|s| (s.style, s.on)), Some((ChatStyle::Bubbles, true)));
    }

    #[test]
    fn offline_messages_are_kept_ones() {
        let events = offline_messages(Uuid::from_u128(1));
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|e| match e {
            NetEvent::InstantMessage(im) => im.offline,
            NetEvent::SessionInvite(inv) => inv.offline,
            _ => false,
        }));
    }
}
