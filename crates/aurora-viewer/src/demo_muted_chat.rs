//! AURORA_DEMO_MUTED_CHAT=aurora|compact|bubbles|legacy|lines[:hidden|collapsed|greyed]:
//! the demo's blocked resident (Violet Wolf, on the demo block list) talks
//! in nearby chat among Nova's lines, three lines in a row then one more,
//! and the blocked spam box says a line in between (FSShowMutedChatHistory,
//! fsfloaternearbychat.cpp). One line every 2 s from 3 s in world. The
//! Conversations window shows the nearby chat in the style named, or
//! `lines` the lines above the chat bar (the window closed), where no
//! blocked line ever comes. The mode named sets Preferences › Chat ›
//! Blocked residents' chat for the run, nothing saved; without one the
//! preference applies. Expected in nearby chat, for the modes:
//! hidden: Nova's three lines only; collapsed: Nova, "3 messages hidden",
//! Nova, "1 message hidden", Nova; greyed: all of Violet's lines, greyed.
//! The spam box's line is never shown.

use crate::settings::{BlockedChat, ChatStyle};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetEvent};
use glam::Vec3;
use uuid::Uuid;

/// The Conversations window's style (None: the lines above the chat bar),
/// and the mode for the run (None: the preference's).
pub fn scene() -> Option<(Option<ChatStyle>, Option<BlockedChat>)> {
    parse(&std::env::var("AURORA_DEMO_MUTED_CHAT").ok()?)
}

fn parse(v: &str) -> Option<(Option<ChatStyle>, Option<BlockedChat>)> {
    let mut words = v.trim().split(':');
    let style = match words.next()? {
        "aurora" | "1" => Some(ChatStyle::Aurora),
        "compact" => Some(ChatStyle::Compact),
        "bubbles" => Some(ChatStyle::Bubbles),
        "legacy" => Some(ChatStyle::Legacy),
        "lines" => None,
        _ => return None,
    };
    let mode = match words.next() {
        Some("hidden") => Some(BlockedChat::Hidden),
        Some("collapsed") => Some(BlockedChat::Collapsed),
        Some("greyed") => Some(BlockedChat::Greyed),
        _ => None,
    };
    Some((style, mode))
}

/// The spam box of the demo block list (an object, blocked by id).
const SPAM_BOX: Uuid = Uuid::from_u128(0xB10C_0000_0000_0000_0000_0000_0000_0001);

/// Who says what, in order.
const LINES: [Who; 8] = [
    Who::Nova,
    Who::Blocked,
    Who::Blocked,
    Who::Blocked,
    Who::SpamBox,
    Who::Nova,
    Who::Blocked,
    Who::Nova,
];

/// The `n`th line's text.
fn text(n: usize) -> String {
    use aurora_i18n::tr;
    match n {
        0 => tr!("demo-muted-chat-near-1"),
        1 => tr!("demo-muted-chat-blocked-1"),
        2 => tr!("demo-muted-chat-blocked-2"),
        3 => tr!("demo-muted-chat-blocked-3"),
        4 => tr!("demo-muted-chat-object"),
        5 => tr!("demo-muted-chat-near-2"),
        6 => tr!("demo-muted-chat-blocked-4"),
        _ => tr!("demo-muted-chat-near-3"),
    }
    .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Nova,
    Blocked,
    SpamBox,
}

/// The `n`th line, said a few metres from `me`.
fn line(n: usize, me: Vec3) -> NetEvent {
    use aurora_i18n::tr;
    let (source_id, from_name, source_type, offset) = match LINES[n] {
        Who::Nova => (
            crate::demo::DEMO_NOVA,
            tr!("demo-nova-example"),
            ChatSourceType::Agent,
            Vec3::new(4.0, 0.0, 0.0),
        ),
        Who::Blocked => (
            crate::demo::DEMO_LOUP,
            tr!("demo-violet-wolf"),
            ChatSourceType::Agent,
            Vec3::new(0.0, 5.0, 0.0),
        ),
        Who::SpamBox => (SPAM_BOX, tr!("demo-spammer"), ChatSourceType::Object, Vec3::new(-3.0, 0.0, 0.0)),
    };
    // kept within the region, the side of us that has room
    let at = |m: f32, d: f32| if (0.0..256.0).contains(&(m + d)) { m + d } else { m - d };
    NetEvent::Chat(ChatMessage {
        from_name: from_name.into(),
        source_id,
        owner_id: Uuid::nil(),
        source_type,
        chat_type: ChatType::Normal,
        region: Some(crate::demo::HANDLE),
        position: Vec3::new(at(me.x, offset.x), at(me.y, offset.y), me.z + offset.z),
        message: text(n),
    })
}

/// The scenario's next line now due, once; asked every frame in world.
pub fn tick(me: Vec3) -> Option<NetEvent> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SAID: AtomicUsize = AtomicUsize::new(0);
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    scene()?;
    let secs = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64();
    let said = SAID.load(Ordering::Relaxed);
    if secs < 3.0 || said >= LINES.len() || said > ((secs - 3.0) / 2.0) as usize {
        return None;
    }
    SAID.store(said + 1, Ordering::Relaxed);
    Some(line(said, me))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_style_and_the_mode_are_named() {
        assert_eq!(
            parse("legacy:collapsed"),
            Some((Some(ChatStyle::Legacy), Some(BlockedChat::Collapsed)))
        );
        assert_eq!(parse("lines"), Some((None, None)));
        assert_eq!(parse("aurora:greyed"), Some((Some(ChatStyle::Aurora), Some(BlockedChat::Greyed))));
        assert_eq!(parse("nope"), None);
    }

    #[test]
    fn every_line_has_its_text() {
        for n in 0..LINES.len() {
            let NetEvent::Chat(c) = line(n, Vec3::new(128.0, 128.0, 25.0)) else {
                unreachable!()
            };
            assert!(!c.message.is_empty() && !c.message.starts_with("demo-"), "{n}: {}", c.message);
        }
    }
}
