//! AURORA_DEMO_CHAT_RANGE=aurora|compact|bubbles|legacy|lines: residents
//! and objects talk from near and from beyond the say range, so that the
//! far lines show dimmed (Preferences › Chat › Brightness beyond chat
//! range, LLViewerChat::getChatColor). A resident 6 m away says a line, one
//! 60 m away shouts, an object 45 m away shouts and one 4 m away speaks:
//! one line every 3 s from 3 s in world, around where we stand, five rounds.
//! The Conversations window shows the nearby chat in the style named, or
//! `lines` the lines above the chat bar (the window closed).

use crate::settings::ChatStyle;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetEvent};
use glam::Vec3;
use uuid::Uuid;

/// The Conversations window's style; `Some(None)`: the lines above the chat
/// bar; None: not this scenario.
pub fn scene() -> Option<Option<ChatStyle>> {
    parse(&std::env::var("AURORA_DEMO_CHAT_RANGE").ok()?)
}

fn parse(v: &str) -> Option<Option<ChatStyle>> {
    match v.trim() {
        "aurora" | "1" => Some(Some(ChatStyle::Aurora)),
        "compact" => Some(Some(ChatStyle::Compact)),
        "bubbles" => Some(Some(ChatStyle::Bubbles)),
        "legacy" => Some(Some(ChatStyle::Legacy)),
        "lines" => Some(None),
        _ => None,
    }
}

/// The demo's Nova, whose name stays hers (the name cache knows her).
const NEAR_RESIDENT: Uuid = crate::demo::DEMO_NOVA;
const FAR_RESIDENT: Uuid = Uuid::from_u128(0xC4A7_4A9E_0000_0000_0000_0000_0000_0002);
const FAR_OBJECT: Uuid = Uuid::from_u128(0xC4A7_4A9E_0000_0000_0000_0000_0000_0003);
const NEAR_OBJECT: Uuid = Uuid::from_u128(0xC4A7_4A9E_0000_0000_0000_0000_0000_0004);
/// Lines in a round, and rounds.
const ROUND: usize = 4;
const ROUNDS: usize = 5;

/// The `n`th line of the scenario, said around `me` (region-local in the
/// demo region).
fn line(n: usize, me: Vec3) -> NetEvent {
    use aurora_i18n::tr;
    let (from, name, text, source_type, chat_type, offset) = match n % ROUND {
        0 => (
            NEAR_RESIDENT,
            tr!("demo-nova-example"),
            tr!("demo-chat-range-near"),
            ChatSourceType::Agent,
            ChatType::Normal,
            Vec3::new(6.0, 0.0, 0.0),
        ),
        1 => (
            FAR_RESIDENT,
            tr!("demo-orion-example"),
            tr!("demo-chat-range-far"),
            ChatSourceType::Agent,
            ChatType::Shout,
            Vec3::new(-42.0, 42.0, 5.0),
        ),
        2 => (
            FAR_OBJECT,
            tr!("demo-chat-range-stage"),
            tr!("demo-chat-range-object-far"),
            ChatSourceType::Object,
            ChatType::Shout,
            Vec3::new(30.0, -33.0, 2.0),
        ),
        _ => (
            NEAR_OBJECT,
            tr!("demo-chat-range-kiosk"),
            tr!("demo-chat-range-object-near"),
            ChatSourceType::Object,
            ChatType::Normal,
            Vec3::new(0.0, 4.0, 0.0),
        ),
    };
    // kept within the region, the side of us that has room
    let at = |m: f32, d: f32| if (0.0..256.0).contains(&(m + d)) { m + d } else { m - d };
    NetEvent::Chat(ChatMessage {
        from_name: name.into(),
        source_id: from,
        owner_id: Uuid::nil(),
        source_type,
        chat_type,
        region: Some(crate::demo::HANDLE),
        position: Vec3::new(at(me.x, offset.x), at(me.y, offset.y), me.z + offset.z),
        message: text.into(),
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
    if secs < 3.0 || said >= ROUND * ROUNDS || said > ((secs - 3.0) / 3.0) as usize {
        return None;
    }
    SAID.store(said + 1, Ordering::Relaxed);
    Some(line(said, me))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::radar::beyond_say_range;

    #[test]
    fn the_far_speakers_are_beyond_the_say_range_and_the_near_ones_within() {
        // in the middle of the region and at its corners
        for me in [
            Vec3::new(132.0, 126.0, 25.0),
            Vec3::new(2.0, 2.0, 25.0),
            Vec3::new(254.0, 254.0, 25.0),
        ] {
            let far: Vec<bool> = (0..ROUND)
                .map(|n| {
                    let NetEvent::Chat(c) = line(n, me) else { unreachable!() };
                    assert!((0.0..256.0).contains(&c.position.x) && (0.0..256.0).contains(&c.position.y));
                    beyond_say_range(c.position, Vec3::ZERO, me, 20.0)
                })
                .collect();
            assert_eq!(far, vec![false, true, true, false], "{me}");
        }
    }

    #[test]
    fn the_styles_are_named() {
        assert_eq!(parse("legacy"), Some(Some(ChatStyle::Legacy)));
        assert_eq!(parse("lines"), Some(None));
        assert_eq!(parse("nope"), None);
    }
}
