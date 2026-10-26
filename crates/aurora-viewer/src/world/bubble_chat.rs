//! What each avatar said lately, for the chat bubbles above its head
//! (Preferences › Chat, UseChatBubbles; #673). Port of LLVOAvatar::addChat
//! and of the line ageing of LLVOAvatar::idleUpdateNameTagText
//! (indra/newview/llvoavatar.cpp, originally LGPL 2.1). Kept whatever the
//! setting, so that turning it on shows what was just said.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use uuid::Uuid;

/// CHAT_FADE_TIME: a line goes from new to normal, then to old, every 8 s.
const FADE_SECONDS: f32 = 8.0;
/// BUBBLE_CHAT_TIME: three fades, then the line is no longer drawn.
pub const LINE_SECONDS: f32 = 3.0 * FADE_SECONDS;
/// MAX_BUBBLE_CHAT_UTTERANCES and MAX_BUBBLE_CHAT_LENGTH
/// (DB_CHAT_MSG_STR_LEN, in bytes as the Legacy Viewer's std::string).
const MAX_UTTERANCES: usize = 12;
const MAX_LENGTH: usize = 1023;

/// How it was said: a whisper is drawn in italics, a shout in bold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    Normal,
    Whisper,
    Shout,
}

#[derive(Debug, Clone)]
pub struct Utterance {
    /// The message as received, links and "/me" included: the bubble
    /// shows their labels (`ui::chat::resolved_text`).
    pub text: String,
    /// The speaker's name as the nearby chat has it, for an emote.
    pub from: String,
    pub voice: Voice,
    pub said: Instant,
}

impl Utterance {
    /// "/me waves" is "Name waves" (LLViewerChat::formatChatMsg).
    pub fn emote(&self) -> bool {
        self.text.starts_with("/me ") || self.text.starts_with("/me'")
    }
}

#[derive(Debug, Default)]
pub struct BubbleChat {
    said: HashMap<Uuid, VecDeque<Utterance>>,
}

fn age(line: &Utterance, now: Instant) -> f32 {
    now.saturating_duration_since(line.said).as_secs_f32()
}

impl BubbleChat {
    /// LLVOAvatar::addChat: the oldest lines go past twelve, or past 1023
    /// bytes of text together.
    pub fn add(&mut self, avatar: Uuid, line: Utterance) {
        let now = line.said;
        // speakers whose every line has aged out
        self.said
            .retain(|_, lines| lines.back().is_some_and(|l| age(l, now) < LINE_SECONDS));
        let lines = self.said.entry(avatar).or_default();
        lines.push_back(line);
        let mut length: usize = lines.iter().map(|l| l.text.len()).sum();
        while lines.len() > MAX_UTTERANCES || (length > MAX_LENGTH && lines.len() > 1) {
            if let Some(old) = lines.pop_front() {
                length -= old.text.len();
            }
        }
    }

    /// The lines of `avatar` still drawn at `now`, oldest first, each with
    /// its tone (`tone`).
    pub fn lines(&self, avatar: &Uuid, now: Instant) -> impl Iterator<Item = (&Utterance, f32)> {
        self.said
            .get(avatar)
            .into_iter()
            .flatten()
            .filter_map(move |l| tone(age(l, now)).map(|t| (l, t)))
    }

    pub fn has_lines(&self, avatar: &Uuid, now: Instant) -> bool {
        self.lines(avatar, now).next().is_some()
    }
}

/// How faded a line of `age` seconds is: 0 new, 1 normal, 2 old, blended
/// over the last tenth of each 8 s step; None once 24 s old.
pub fn tone(age: f32) -> Option<f32> {
    let a = age / FADE_SECONDS;
    if !(0.0..3.0).contains(&a) {
        // (a line from the future is new)
        return (a < 0.0).then_some(0.0);
    }
    let step = |from: f32| ((a - from) * 10.0).clamp(0.0, 1.0);
    Some(step(0.9) + step(1.9))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn said(text: &str, at: Instant) -> Utterance {
        Utterance {
            text: text.into(),
            from: "Nova Example".into(),
            voice: Voice::Normal,
            said: at,
        }
    }

    #[test]
    fn a_line_ages_new_normal_old_then_goes_after_24_seconds() {
        assert_eq!(tone(0.0), Some(0.0));
        assert_eq!(tone(7.0), Some(0.0));
        assert!(tone(7.6).is_some_and(|t| t > 0.0 && t < 1.0));
        assert_eq!(tone(8.0), Some(1.0));
        assert_eq!(tone(15.0), Some(1.0));
        assert!(tone(15.6).is_some_and(|t| t > 1.0 && t < 2.0));
        assert_eq!(tone(16.0), Some(2.0));
        assert_eq!(tone(23.9), Some(2.0));
        assert_eq!(tone(24.0), None);
    }

    #[test]
    fn twelve_lines_at_most_and_1023_bytes_together() {
        let t0 = Instant::now();
        let id = Uuid::from_u128(1);
        let mut chat = BubbleChat::default();
        for k in 0..15 {
            chat.add(id, said(&format!("line {k}"), t0));
        }
        let kept: Vec<_> = chat.lines(&id, t0).map(|(l, _)| l.text.clone()).collect();
        assert_eq!(kept.len(), 12);
        assert_eq!(kept[0], "line 3");
        // three long lines: the first goes, the newest always stays
        let long = "x".repeat(400);
        for _ in 0..3 {
            chat.add(id, said(&long, t0));
        }
        let lengths: usize = chat.lines(&id, t0).map(|(l, _)| l.text.len()).sum();
        assert!(lengths <= 1023, "{lengths}");
        chat.add(id, said(&"y".repeat(1500), t0));
        assert_eq!(chat.lines(&id, t0).count(), 1);
    }

    #[test]
    fn each_avatar_keeps_its_own_lines_and_silent_ones_are_forgotten() {
        let t0 = Instant::now();
        let (nova, tess) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut chat = BubbleChat::default();
        chat.add(nova, said("hello", t0));
        chat.add(tess, said("hi", t0 + Duration::from_secs(10)));
        let later = t0 + Duration::from_secs(25);
        assert!(!chat.has_lines(&nova, later));
        assert!(chat.has_lines(&tess, later));
        chat.add(tess, said("again", later));
        assert!(!chat.said.contains_key(&nova));
        assert!(said("/me waves", t0).emote() && !said("/meow", t0).emote());
    }
}
