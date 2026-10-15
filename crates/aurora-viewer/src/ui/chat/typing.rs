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
}
