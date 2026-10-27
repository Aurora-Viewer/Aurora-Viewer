//! "Is typing": the notices we send while writing to a resident, and what a
//! conversation says of the others. Port of FSFloaterIM::setTyping,
//! timedUpdate and processIMTyping (indra/newview/fsfloaterim.cpp,
//! originally LGPL 2.1): a start once one has typed for a second, again
//! every four seconds while it goes on, a stop when the text is sent or
//! emptied, when another conversation takes the keyboard, or after five
//! seconds without a key (LLAgent::TYPING_TIMEOUT_SECS). Only in a
//! conversation with one resident, as Firestorm (no traffic to a group).

use std::time::{Duration, Instant};
use uuid::Uuid;

/// A start leaves after this much typing (mTypingTimer).
const START_AFTER: Duration = Duration::from_secs(1);
/// Sent again while typing goes on (ME_TYPING_TIMEOUT).
const REPEAT: Duration = Duration::from_secs(4);
/// Typing stops after this long without a key (TYPING_TIMEOUT_SECS).
const IDLE: Duration = Duration::from_secs(5);

/// Our typing in a conversation with one resident.
#[derive(Debug, Default)]
pub struct MeTyping {
    /// The resident written to.
    to: Option<Uuid>,
    /// When the typing began, and its last key.
    since: Option<Instant>,
    last_key: Option<Instant>,
    /// The last start sent (none yet: nothing to stop).
    sent: Option<Instant>,
}

/// One frame of the field of a conversation with one resident.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// The resident, when the conversation shown is one with a resident and
    /// its field has the keyboard.
    pub to: Option<Uuid>,
    /// Its text changed this frame by a key.
    pub typed: bool,
    pub empty: bool,
    /// A message left this frame.
    pub sent: bool,
}

impl MeTyping {
    /// The notices due this frame: (resident, typing).
    pub fn update(&mut self, now: Instant, field: Field) -> Vec<(Uuid, bool)> {
        let mut out = Vec::new();
        if let Some(to) = self.to {
            let idle = self.last_key.is_none_or(|k| now.duration_since(k) >= IDLE);
            if field.to != Some(to) || field.sent || field.empty || idle {
                if self.sent.is_some() {
                    out.push((to, false));
                }
                *self = MeTyping::default();
            }
        }
        if let Some(to) = field.to
            && field.typed
            && !field.empty
            && !field.sent
        {
            if self.to != Some(to) {
                *self = MeTyping {
                    to: Some(to),
                    since: Some(now),
                    ..MeTyping::default()
                };
            }
            self.last_key = Some(now);
        }
        if let (Some(to), Some(since)) = (self.to, self.since) {
            let due = match self.sent {
                None => now.duration_since(since) >= START_AFTER,
                Some(t) => now.duration_since(t) >= REPEAT,
            };
            if due {
                out.push((to, true));
                self.sent = Some(now);
            }
        }
        out
    }
}

/// Our typing in the nearby chat, seen by those in range: the typing chat
/// notices and animation. Port of FSNearbyChat::handleChatBarKeystroke
/// (indra/newview/fsnearbychathub.cpp) and LLAgent::startTyping /
/// stopTyping / getRenderState (indra/newview/llagent.cpp, originally LGPL
/// 2.1): a key that leaves text to say starts it at once, and only once; it
/// stops when the line is sent, when the text is emptied or turns into a
/// command, when the field loses the keyboard, or after five seconds
/// without a key.
#[derive(Debug, Default)]
pub struct LocalTyping {
    /// The last key while typing (None: not typing).
    last_key: Option<Instant>,
}

/// One frame of the nearby chat's fields (the chat bar or island, the
/// Conversations' nearby tab), gathered while they are drawn.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalField {
    /// One of them has the keyboard.
    pub focused: bool,
    /// Its text changed by a key: whether what is left is said aloud.
    pub typed: Option<bool>,
    /// A line left.
    pub sent: bool,
}

impl LocalTyping {
    /// We are typing (AGENT_STATE_TYPING).
    pub fn typing(&self) -> bool {
        self.last_key.is_some()
    }

    /// The change due this frame: Some(true) to start, Some(false) to stop.
    pub fn update(&mut self, now: Instant, field: LocalField) -> Option<bool> {
        if field.focused && field.typed == Some(true) && !field.sent {
            let start = !self.typing();
            self.last_key = Some(now);
            return start.then_some(true);
        }
        let idle = self.last_key.is_some_and(|k| now.duration_since(k) > IDLE);
        if field.sent || !field.focused || field.typed == Some(false) || idle {
            return self.stop().then_some(false);
        }
        None
    }

    /// Keep typing as if a key came now (`AURORA_DEMO_TYPING`); true when
    /// it starts.
    pub fn hold(&mut self, now: Instant) -> bool {
        self.last_key.replace(now).is_none()
    }

    /// Stop typing (a teleport, LLAgent::stopTyping); true when we were.
    pub fn stop(&mut self) -> bool {
        self.last_key.take().is_some()
    }
}

/// A text of the nearby chat that shows us typing: something to say on
/// channel 0, not a `/` command or channel, nor an emote unless
/// `during_emote` ("/me " or "/me'", FSTypeDuringEmote). With
/// `colon_emote` (AllowMUpose), any line that starts with ":" counts as an
/// emote, smileys included, like the Legacy Viewer's keystroke rule
/// (FSNearbyChat::handleChatBarKeystroke).
pub fn says_aloud(text: &str, during_emote: bool, colon_emote: bool) -> bool {
    let text = text.trim_start();
    match text.strip_prefix('/') {
        None if colon_emote && text.starts_with(':') => during_emote,
        None => !text.is_empty(),
        Some(rest) => {
            during_emote
                && rest.len() > 2
                && rest.is_char_boundary(2)
                && rest[..2].eq_ignore_ascii_case("me")
                && matches!(rest.as_bytes()[2], b' ' | b'\'')
        }
    }
}

/// "Nova is typing…", "Nova and Tess are typing…", "Several people are
/// typing…" for the names of those who type.
pub fn text(names: &[String]) -> Option<String> {
    Some(match names {
        [] => return None,
        [one] => aurora_i18n::tr!("ui-chat-typing-one", name = one.as_str()).to_owned(),
        [a, b] => aurora_i18n::tr!("ui-chat-typing-two", a = a.as_str(), b = b.as_str()).to_owned(),
        _ => aurora_i18n::tr!("ui-chat-typing-many").to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn field(to: Option<Uuid>, typed: bool, empty: bool, sent: bool) -> Field {
        Field { to, typed, empty, sent }
    }

    /// A start after a second of typing, again every four seconds while it
    /// goes on, a stop when the message leaves; nothing for a key alone.
    #[test]
    fn typing_starts_repeats_and_stops_with_the_message() {
        let (t0, a) = (Instant::now(), Uuid::from_u128(1));
        let mut me = MeTyping::default();
        assert!(me.update(t0, field(Some(a), true, false, false)).is_empty());
        assert!(me.update(at(t0, 500), field(Some(a), true, false, false)).is_empty());
        assert_eq!(me.update(at(t0, 1000), field(Some(a), false, false, false)), vec![(a, true)]);
        assert!(me.update(at(t0, 3000), field(Some(a), true, false, false)).is_empty());
        assert_eq!(me.update(at(t0, 5000), field(Some(a), true, false, false)), vec![(a, true)]);
        assert_eq!(me.update(at(t0, 5500), field(Some(a), false, true, true)), vec![(a, false)]);
        assert!(me.update(at(t0, 6000), field(Some(a), false, true, false)).is_empty());
        // one key, then the text emptied before the second: never started
        let mut me = MeTyping::default();
        me.update(t0, field(Some(a), true, false, false));
        assert!(me.update(at(t0, 300), field(Some(a), true, true, false)).is_empty());
        assert!(me.update(at(t0, 2000), field(Some(a), false, true, false)).is_empty());
    }

    /// Five seconds without a key, or another conversation taking the
    /// keyboard, stop the typing that was announced.
    #[test]
    fn idle_or_another_conversation_stops_it() {
        let (t0, a, b) = (Instant::now(), Uuid::from_u128(1), Uuid::from_u128(2));
        let mut me = MeTyping::default();
        me.update(t0, field(Some(a), true, false, false));
        assert_eq!(me.update(at(t0, 1000), field(Some(a), false, false, false)), vec![(a, true)]);
        assert!(me.update(at(t0, 4900), field(Some(a), false, false, false)).is_empty());
        assert_eq!(me.update(at(t0, 5000), field(Some(a), false, false, false)), vec![(a, false)]);
        let mut me = MeTyping::default();
        me.update(t0, field(Some(a), true, false, false));
        me.update(at(t0, 1000), field(Some(a), false, false, false));
        let out = me.update(at(t0, 1200), field(Some(b), true, false, false));
        assert_eq!(out, vec![(a, false)]);
        assert_eq!(me.update(at(t0, 2300), field(Some(b), false, false, false)), vec![(b, true)]);
    }

    fn local(focused: bool, typed: Option<bool>, sent: bool) -> LocalField {
        LocalField { focused, typed, sent }
    }

    /// The nearby chat starts at the first key, once, keeps going while
    /// keys come and stops with the line sent; the next key starts again.
    #[test]
    fn nearby_typing_starts_once_and_stops_with_the_line() {
        let t0 = Instant::now();
        let mut me = LocalTyping::default();
        assert_eq!(me.update(t0, local(true, Some(true), false)), Some(true));
        assert!(me.typing());
        assert_eq!(me.update(at(t0, 3000), local(true, Some(true), false)), None);
        // eight seconds after the start, but three after the last key
        assert_eq!(me.update(at(t0, 6000), local(true, None, false)), None);
        assert_eq!(me.update(at(t0, 6500), local(true, None, true)), Some(false));
        assert!(!me.typing());
        assert_eq!(me.update(at(t0, 7000), local(true, None, false)), None);
        assert_eq!(me.update(at(t0, 7500), local(true, Some(true), false)), Some(true));
    }

    /// Five seconds without a key, the keyboard taken elsewhere, the text
    /// emptied or turned into a command, or a teleport stop it.
    #[test]
    fn nearby_typing_stops_when_idle_unfocused_or_emptied() {
        let t0 = Instant::now();
        let mut me = LocalTyping::default();
        me.update(t0, local(true, Some(true), false));
        assert_eq!(me.update(at(t0, 5000), local(true, None, false)), None);
        assert_eq!(me.update(at(t0, 5001), local(true, None, false)), Some(false));
        me.update(t0, local(true, Some(true), false));
        assert_eq!(me.update(at(t0, 100), local(false, None, false)), Some(false));
        me.update(t0, local(true, Some(true), false));
        assert_eq!(me.update(at(t0, 100), local(true, Some(false), false)), Some(false));
        // nothing to stop: nothing sent
        assert_eq!(me.update(at(t0, 200), local(true, Some(false), false)), None);
        assert_eq!(me.update(at(t0, 300), local(false, Some(true), false)), None);
        me.update(t0, local(true, Some(true), false));
        assert!(me.stop());
        assert!(!me.stop());
        // the demo's typing without a key starts once, then goes on
        assert!(me.hold(t0));
        assert!(!me.hold(at(t0, 6000)));
        assert!(me.typing());
    }

    /// What shows us typing: words on channel 0; "/me" only with the
    /// emote setting; never a channel or a command.
    #[test]
    fn says_aloud_follows_the_chat_bar_keystroke_rule() {
        assert!(says_aloud("hello", false, false));
        assert!(says_aloud("  hi", false, false));
        assert!(!says_aloud("", false, false));
        assert!(!says_aloud("   ", false, false));
        assert!(!says_aloud("/5 open", true, false));
        assert!(!says_aloud("/me waves", false, false));
        assert!(says_aloud("/me waves", true, false));
        assert!(says_aloud("/ME's here", true, false));
        assert!(!says_aloud("/me", true, false));
        assert!(!says_aloud("/mesh", true, false));
        assert!(!says_aloud("/é", true, false));
        // ":" as "/me": an emote, a smiley too; plain text without the option
        assert!(says_aloud(":waves", false, false));
        assert!(!says_aloud(":waves", false, true));
        assert!(!says_aloud(" :)", false, true));
        assert!(says_aloud(":waves", true, true));
        assert!(says_aloud("hello :)", false, true));
    }
}
