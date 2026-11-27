//! AURORA_DEMO_COMPACT_WRAP=1|compact|aurora|bubbles|legacy (#3137): long
//! nearby chat lines from senders with long names and tags, to see how the
//! Compact style wraps them: the name and its tags lead the first row and the
//! next rows come back under the name, using the whole width. The demo's
//! long-named resident (`DEMO_LONG_NAME`) shouts (the SHOUTS tag), the
//! resident of `DEMO_NOVA` (the Aurora team's mark with AURORA_DEMO_STAFF=1)
//! says a paragraph, an emote and a short line, the long name a line with a
//! long link and emoji, and an object a long line (its own compact row). The
//! names shown are those the demo's name service gives these ids.
//! The lines come at the arrival, one every 0.3 s from 2 s in world; the
//! Conversations window shows them in the style named (`1` is `compact`).

use crate::settings::ChatStyle;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetEvent};
use glam::Vec3;
use uuid::Uuid;

/// The Conversations window's style; None: not this scenario.
pub fn scene() -> Option<ChatStyle> {
    parse(&std::env::var("AURORA_DEMO_COMPACT_WRAP").ok()?)
}

fn parse(v: &str) -> Option<ChatStyle> {
    match v.trim() {
        "compact" | "1" => Some(ChatStyle::Compact),
        "aurora" => Some(ChatStyle::Aurora),
        "bubbles" => Some(ChatStyle::Bubbles),
        "legacy" => Some(ChatStyle::Legacy),
        _ => None,
    }
}

const NOVA: Uuid = crate::demo::DEMO_NOVA;
/// The demo's resident with a long display name (`demo::long_name`).
const LONG_NAME: Uuid = crate::demo::DEMO_LONG_NAME;
const OBJECT: Uuid = Uuid::from_u128(0xC0A7_3137_0000_0000_0000_0000_0000_0002);
/// A link longer than a narrow window's row, which wraps inside itself.
const LINK: &str = "https://example.com/aurora/notes/compact-layout/wrapping/a-path-long-enough-to-wrap";

/// The scenario's lines, in order: sender, name, text, source, volume.
fn lines() -> Vec<(Uuid, String, String, ChatSourceType, ChatType)> {
    use aurora_i18n::tr;
    let nova = tr!("demo-nova-example").to_owned();
    // (the line's own name, until the display name arrives)
    let long = "Maximilienne Wolfeschlegelsteinhausen".to_owned();
    vec![
        (
            LONG_NAME,
            long.clone(),
            tr!("demo-compact-wrap-shout").to_owned(),
            ChatSourceType::Agent,
            ChatType::Shout,
        ),
        (
            NOVA,
            nova.clone(),
            tr!("demo-compact-wrap-paragraph").to_owned(),
            ChatSourceType::Agent,
            ChatType::Normal,
        ),
        (
            NOVA,
            nova.clone(),
            format!("/me {}", tr!("demo-compact-wrap-emote")),
            ChatSourceType::Agent,
            ChatType::Normal,
        ),
        (
            LONG_NAME,
            long.clone(),
            format!("{} {LINK} 🎉✨", tr!("demo-compact-wrap-link")),
            ChatSourceType::Agent,
            ChatType::Normal,
        ),
        (
            NOVA,
            nova,
            tr!("demo-compact-wrap-short").to_owned(),
            ChatSourceType::Agent,
            ChatType::Normal,
        ),
        (
            OBJECT,
            tr!("demo-compact-wrap-object-name").to_owned(),
            tr!("demo-compact-wrap-object-text").to_owned(),
            ChatSourceType::Object,
            ChatType::Normal,
        ),
    ]
}

/// The `n`th line, said 3 m from `me` (region-local in the demo region).
fn line(n: usize, me: Vec3) -> Option<NetEvent> {
    let (source_id, from_name, message, source_type, chat_type) = lines().into_iter().nth(n)?;
    // kept within the region, on the side of us that has room
    let x = if me.x + 3.0 < 256.0 { me.x + 3.0 } else { me.x - 3.0 };
    Some(NetEvent::Chat(ChatMessage {
        from_name,
        source_id,
        owner_id: Uuid::nil(),
        source_type,
        chat_type,
        region: Some(crate::demo::HANDLE),
        position: Vec3::new(x, me.y, me.z),
        message,
    }))
}

/// The scenario's next line now due, once; asked every frame in world.
pub fn tick(me: Vec3) -> Option<NetEvent> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SAID: AtomicUsize = AtomicUsize::new(0);
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    scene()?;
    let secs = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64();
    let said = SAID.load(Ordering::Relaxed);
    if secs < 2.0 || said > ((secs - 2.0) / 0.3) as usize {
        return None;
    }
    let event = line(said, me)?;
    SAID.store(said + 1, Ordering::Relaxed);
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_styles_are_named() {
        assert_eq!(parse("1"), Some(ChatStyle::Compact));
        assert_eq!(parse("compact"), Some(ChatStyle::Compact));
        assert_eq!(parse("legacy"), Some(ChatStyle::Legacy));
        assert_eq!(parse("nope"), None);
    }

    #[test]
    fn every_line_is_said_once_then_none() {
        let me = Vec3::new(254.0, 128.0, 25.0);
        let n = lines().len();
        for i in 0..n {
            let Some(NetEvent::Chat(c)) = line(i, me) else {
                panic!("line {i}");
            };
            assert!((0.0..256.0).contains(&c.position.x), "{}", c.position);
            assert!(!c.message.is_empty() && !c.from_name.is_empty());
        }
        assert!(line(n, me).is_none());
    }

    #[test]
    fn the_long_lines_wrap_in_a_narrow_window_and_one_is_an_emote() {
        let lines = lines();
        // longer than a row of a 400-point-wide window at the chat's size
        assert!(lines.iter().filter(|l| l.2.chars().count() > 120).count() >= 2);
        assert!(lines.iter().any(|l| l.2.starts_with("/me ")));
        assert!(lines.iter().any(|l| l.4 == ChatType::Shout));
    }
}
