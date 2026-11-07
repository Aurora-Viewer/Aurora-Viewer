//! Demo of the conversation log (toolbar `conversation_log`), without a
//! grid nor a file: `AURORA_DEMO_CONVERSATION_LOG=<mode>` fills the log at
//! frame 590 with entries of the demo's residents and groups (a friend, an
//! entry with offline messages unread, two groups, a conference, entries
//! of days and weeks ago) and opens the window at frame 600. Nothing is
//! written: the demo has no transcript folder.
//!
//! - `1` or `open`: the window by date, friends first (the defaults);
//! - `name`: sorted by name;
//! - `filter`: "no" typed in the filter, its matches highlighted;
//! - `off`: logging turned off, the old entries still listed;
//! - `empty`: logging on and nothing from before: the demo's open group
//!   chat is logged as it opens;
//! - `live`: an empty log; a resident's offline message at frame 600 opens
//!   a conversation, which the log lists at once with its unread mark.

use crate::world::World;
use crate::world::chat_log::conversations::{Conversation, Kind, private_session};
use aurora_net::{InstantMessage, NetEvent};
use std::sync::OnceLock;
use uuid::Uuid;

/// The frame the log is filled, and the one the window opens.
pub const FILL_FRAME: u64 = 590;
pub const OPEN_FRAME: u64 = 600;
/// `filter`: what is typed.
pub const FILTER: &str = "no";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Open,
    Name,
    Filter,
    Off,
    Empty,
    Live,
}

pub fn mode() -> Option<Mode> {
    static MODE: OnceLock<Option<Mode>> = OnceLock::new();
    *MODE.get_or_init(|| parse(&std::env::var("AURORA_DEMO_CONVERSATION_LOG").ok()?))
}

fn parse(value: &str) -> Option<Mode> {
    match value.trim() {
        "1" | "open" => Some(Mode::Open),
        "name" => Some(Mode::Name),
        "filter" => Some(Mode::Filter),
        "off" => Some(Mode::Off),
        "empty" => Some(Mode::Empty),
        "live" => Some(Mode::Live),
        _ => None,
    }
}

/// The entries: the demo's residents and groups, with their names as the
/// world knows them.
pub fn entries(world: &World, now: i64) -> Vec<Conversation> {
    if mode() == Some(Mode::Live) {
        return Vec::new();
    }
    let me = world.agent_id;
    let (minute, hour, day) = (60, 3600, 86_400);
    let resident = |id: Uuid, fallback: &str, ago: i64, offline: bool| {
        // the demo's names, told apart (its name cache gives some
        // residents the same display name)
        let name = fallback.to_owned();
        Conversation {
            time: now - ago,
            kind: Kind::Private,
            history_file: name.clone(),
            name,
            session: private_session(me, id),
            participant: id,
            offline,
        }
    };
    let group = |id: Uuid, fallback: &str, ago: i64| {
        let name = world.groups.group(&id).map_or_else(|| fallback.to_owned(), |g| g.name.clone());
        Conversation {
            time: now - ago,
            kind: Kind::Group,
            history_file: format!("{name} (group)"),
            name,
            session: id,
            participant: id,
            offline: false,
        }
    };
    let conference = Conversation {
        time: now - 8 * day,
        kind: Kind::Conference,
        name: "Ad-hoc Conference".into(),
        history_file: "Ad-hoc Conference".into(),
        session: Uuid::from_u128(0xC0FE_0000_0000_0000_0000_0000_0000_0001),
        participant: crate::demo::DEMO_TESS,
        offline: false,
    };
    vec![
        group(crate::demo::DEMO_GROUP2, "Demo group", 60 * day),
        resident(crate::demo::DEMO_TESS, "Tess Example", 30 * day, false),
        conference,
        resident(crate::demo::DEMO_LOUP, "Loup Example", 3 * day, false),
        group(crate::demo::DEMO_GROUP1, "Demo group", day + 4 * hour),
        resident(crate::demo::DEMO_NOVA, "Nova Example", 2 * hour, true),
        resident(crate::demo::DEMO_FRIEND, "Friend Example", 5 * minute, false),
    ]
}

/// `live`: a resident's message kept while we were logged out.
pub fn new_message(agent: Uuid) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: crate::demo::DEMO_NOVA,
        from_name: "Nova Example".into(),
        to_agent_id: agent,
        dialog: 0,
        session_id: crate::demo::DEMO_NOVA,
        message: aurora_i18n::tr!("demo-chatlog-offline-1").into(),
        offline: true,
        from_group: false,
        binary_bucket: Vec::new(),
        // sent an hour ago
        timestamp: crate::world::chat_log::conversations::now().saturating_sub(3600).max(0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        assert_eq!(parse("1"), Some(Mode::Open));
        assert_eq!(parse(" filter "), Some(Mode::Filter));
        assert_eq!(parse("live"), Some(Mode::Live));
        assert_eq!(parse("0"), None);
        assert_eq!(parse(""), None);
    }
}
