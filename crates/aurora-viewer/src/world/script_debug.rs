//! LLFloaterIMNearbyChatHandler::processChat / LLFloaterScriptDebug::addScriptLine
//! (Firestorm, LGPL 2.1). Muting and bridge filtering happen before this routing.
//!
//! Every script error of our own objects is kept for the Script debug
//! window. With "Show script errors in the nearby chat" (ShowScriptErrors-
//! Location 0, on by default in Aurora, #1263) it also goes on to the
//! nearby chat and the window does not open by itself; otherwise the window
//! opens on it, as with the Legacy Viewer's default. "Also send them on a
//! channel" (off) queues each one for that chat channel, a few per second.

use super::ChatLine;
use aurora_net::{ChatMessage, ChatSourceType, ChatType};
use std::{
    collections::VecDeque,
    time::{Instant, SystemTime},
};
use uuid::Uuid;

/// Script errors that can be sent on the channel in a burst...
const CHANNEL_BURST: f32 = 5.0;
/// ...then one per second: a looping error must not flood the channel.
const CHANNEL_PER_SECOND: f32 = 1.0;

#[derive(Default)]
pub struct Messages {
    pub lines: VecDeque<ChatLine>,
    /// Bumped by a line that opens the window.
    pub opens: u64,
    pub owner_say: bool,
    /// Script errors also go to the nearby chat (Preferences › Chat).
    pub in_chat: bool,
    /// The channel the script errors are also said on, when chosen.
    pub channel: Option<i32>,
    /// Lines waiting to be said on `channel`, taken by the app.
    pub outgoing: Vec<(i32, String)>,
    /// What may still be said on the channel (`CHANNEL_BURST` at most),
    /// counted at `refilled`.
    tokens: f32,
    refilled: Option<Instant>,
}

impl Messages {
    /// Returns true when the message belongs exclusively to script debug.
    pub fn route(&mut self, message: &ChatMessage, agent: Uuid, now: Instant) -> bool {
        if message.source_type != ChatSourceType::Object
            || !(message.chat_type == ChatType::Debug || (self.owner_say && message.chat_type == ChatType::OwnerSay))
        {
            return false;
        }
        // EXT-7762: errors from another resident's objects are not ours.
        if message.owner_id != agent || agent.is_nil() {
            return true;
        }
        if self.lines.len() == 500 {
            self.lines.pop_front();
        }
        self.lines.push_back(ChatLine {
            time: SystemTime::now(),
            from: message.from_name.clone(),
            text: message.message.clone(),
            kind: super::ChatKind::Object(message.chat_type),
            source: message.source_id,
            origin: super::Origin::Live,
        });
        let error = message.chat_type == ChatType::Debug;
        if error && let Some(channel) = self.channel {
            self.queue_for_channel(channel, &message.from_name, &message.message, now);
        }
        // an owner-say line sent here shows nowhere else: it opens the window
        let in_chat = error && self.in_chat;
        if !in_chat {
            self.opens = self.opens.wrapping_add(1);
        }
        !in_chat
    }

    /// Queues "Object: error" for the channel while the rate allows it.
    fn queue_for_channel(&mut self, channel: i32, from: &str, text: &str, now: Instant) {
        let elapsed = self
            .refilled
            .map_or(CHANNEL_BURST, |t| now.duration_since(t).as_secs_f32() * CHANNEL_PER_SECOND);
        self.tokens = (self.tokens + elapsed).min(CHANNEL_BURST);
        self.refilled = Some(now);
        if self.tokens < 1.0 {
            log::debug!("script error not sent on channel {channel}: too many at once");
            return;
        }
        self.tokens -= 1.0;
        self.outgoing.push((channel, format!("{from}: {text}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn message(owner: Uuid, chat_type: ChatType) -> ChatMessage {
        ChatMessage {
            from_name: "Object".into(),
            source_id: Uuid::from_u128(2),
            owner_id: owner,
            source_type: ChatSourceType::Object,
            chat_type,
            position: glam::Vec3::ZERO,
            message: "hello".into(),
        }
    }

    #[test]
    fn debug_accepts_only_owned_objects_and_owner_say_routing_is_optional_and_bounded() {
        let agent = Uuid::from_u128(1);
        let now = Instant::now();
        let mut message = message(agent, ChatType::OwnerSay);
        let mut messages = Messages::default();
        assert!(!messages.route(&message, agent, now));
        messages.owner_say = true;
        assert!(messages.route(&message, agent, now));
        message.owner_id = Uuid::from_u128(3);
        assert!(messages.route(&message, agent, now));
        assert_eq!(messages.lines.len(), 1);
        message.owner_id = agent;
        messages.owner_say = false;
        message.chat_type = ChatType::Debug;
        for _ in 0..510 {
            assert!(messages.route(&message, agent, now));
        }
        assert_eq!(messages.lines.len(), 500);
        message.source_type = ChatSourceType::Agent;
        assert!(!messages.route(&message, agent, now));
    }

    /// In the nearby chat, our own script errors go on to the chat and stay
    /// in the window, which they no longer open; another resident's errors
    /// still show nowhere, and an owner-say sent to the window still opens it.
    #[test]
    fn errors_in_chat_go_on_to_the_chat_and_keep_the_window_closed() {
        let agent = Uuid::from_u128(1);
        let now = Instant::now();
        let mut messages = Messages {
            in_chat: true,
            owner_say: true,
            ..Default::default()
        };
        assert!(
            !messages.route(&message(agent, ChatType::Debug), agent, now),
            "on to the nearby chat"
        );
        assert_eq!((messages.lines.len(), messages.opens), (1, 0));
        assert!(
            messages.route(&message(Uuid::from_u128(3), ChatType::Debug), agent, now),
            "not ours: dropped"
        );
        assert_eq!(messages.lines.len(), 1);
        assert!(messages.route(&message(agent, ChatType::OwnerSay), agent, now));
        assert_eq!((messages.lines.len(), messages.opens), (2, 1));
        messages.in_chat = false;
        assert!(messages.route(&message(agent, ChatType::Debug), agent, now));
        assert_eq!((messages.lines.len(), messages.opens), (3, 2));
    }

    /// Only our own script errors are said on the channel, with their
    /// object's name, five at once then one a second.
    #[test]
    fn errors_go_on_the_channel_at_a_bounded_rate() {
        let agent = Uuid::from_u128(1);
        let start = Instant::now();
        let mut messages = Messages {
            in_chat: true,
            owner_say: true,
            ..Default::default()
        };
        messages.route(&message(agent, ChatType::Debug), agent, start);
        assert!(messages.outgoing.is_empty(), "no channel chosen");
        messages.channel = Some(-7);
        messages.route(&message(agent, ChatType::OwnerSay), agent, start);
        messages.route(&message(Uuid::from_u128(3), ChatType::Debug), agent, start);
        assert!(messages.outgoing.is_empty(), "owner-say and others' errors stay");
        for _ in 0..8 {
            messages.route(&message(agent, ChatType::Debug), agent, start);
        }
        assert_eq!(messages.outgoing.len(), 5);
        assert_eq!(messages.outgoing[0], (-7, "Object: hello".to_owned()));
        messages.route(&message(agent, ChatType::Debug), agent, start + Duration::from_millis(500));
        assert_eq!(messages.outgoing.len(), 5);
        messages.route(&message(agent, ChatType::Debug), agent, start + Duration::from_millis(1100));
        assert_eq!(messages.outgoing.len(), 6);
    }
}
