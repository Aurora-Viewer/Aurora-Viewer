//! What a notice of the viewer in the nearby chat is about (#741): a
//! teleport, a resident, an item, the region, the music, a call… Its compact row
//! shows an icon of that kind instead of the same "i" for all of them.
//!
//! The topic is kept beside the lines, by each notice's time and text, so
//! that a chat line keeps its shape for everything else (transcripts,
//! conversations, the other notices); a notice whose topic was not noted
//! (one read back from a transcript, one of a conversation) is `Info`.

use super::ChatLine;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::time::SystemTime;

/// What a notice is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Topic {
    /// Anything else: the viewer's own information.
    #[default]
    Info,
    /// A resident: a friend online or offline, blocked, a bump.
    People,
    /// A teleport, its progress or its failure, home.
    Teleport,
    /// An item of the inventory or an object.
    Items,
    /// The region: its restart, its alerts.
    Region,
    /// The parcel's music.
    Media,
    /// A call: its start, its end, its errors.
    Call,
    /// Something refused or that failed.
    Warning,
}

/// Notices kept: more than the nearby chat holds on screen; an older one
/// shows as `Info`.
const KEEP: usize = 512;

#[derive(Debug, Default)]
pub struct Topics {
    recent: VecDeque<(SystemTime, u64, Topic)>,
}

fn text_key(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl Topics {
    /// `line` is about `topic` (nothing to keep for `Info`).
    pub fn note(&mut self, line: &ChatLine, topic: Topic) {
        if topic == Topic::Info {
            return;
        }
        if self.recent.len() == KEEP {
            self.recent.pop_front();
        }
        self.recent.push_back((line.time, text_key(&line.text), topic));
    }

    /// What `line` is about.
    pub fn of(&self, line: &ChatLine) -> Topic {
        let key = text_key(&line.text);
        self.recent
            .iter()
            .rev()
            .find(|(time, text, _)| *time == line.time && *text == key)
            .map_or(Topic::Info, |(_, _, topic)| *topic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{ChatKind, Origin};
    use std::time::Duration;

    fn notice(secs: u64, text: &str) -> ChatLine {
        ChatLine {
            time: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + secs),
            from: String::new(),
            text: text.into(),
            kind: ChatKind::System,
            source: uuid::Uuid::nil(),
            origin: Origin::Live,
        }
    }

    #[test]
    fn a_notice_keeps_its_topic_and_an_unknown_one_is_info() {
        let mut topics = Topics::default();
        let online = notice(0, "Nova Example is online.");
        let tp = notice(1, "Teleporting to Aurora Demo…");
        topics.note(&online, Topic::People);
        topics.note(&tp, Topic::Teleport);
        assert_eq!(topics.of(&online), Topic::People);
        assert_eq!(topics.of(&tp), Topic::Teleport);
        // the same text at another time, another text at the same time
        assert_eq!(topics.of(&notice(5, "Nova Example is online.")), Topic::Info);
        assert_eq!(topics.of(&notice(0, "Orion Example is online.")), Topic::Info);
        // only so many are kept
        for i in 0..KEEP as u64 {
            topics.note(&notice(10 + i, "busy"), Topic::Region);
        }
        assert_eq!(topics.of(&online), Topic::Info);
        assert_eq!(topics.recent.len(), KEEP);
    }
}
