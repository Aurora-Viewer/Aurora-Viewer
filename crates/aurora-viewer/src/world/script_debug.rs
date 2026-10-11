//! LLFloaterIMNearbyChatHandler::processChat / LLFloaterScriptDebug::addScriptLine
//! (Firestorm, LGPL 2.1). Muting and bridge filtering happen before this routing.

use super::ChatLine;
use aurora_net::{ChatMessage, ChatSourceType, ChatType};
use std::{collections::VecDeque, time::SystemTime};
use uuid::Uuid;

#[derive(Default)]
pub struct Messages {
    pub lines: VecDeque<ChatLine>,
    pub revision: u64,
    pub owner_say: bool,
}

impl Messages {
    /// Returns true when the message belongs exclusively to script debug.
    pub fn route(&mut self, message: &ChatMessage, agent: Uuid) -> bool {
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
        });
        self.revision = self.revision.wrapping_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_accepts_only_owned_objects_and_owner_say_routing_is_optional_and_bounded() {
        let agent = Uuid::from_u128(1);
        let mut message = ChatMessage {
            from_name: "Object".into(),
            source_id: Uuid::from_u128(2),
            owner_id: agent,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "hello".into(),
        };
        let mut messages = Messages::default();
        assert!(!messages.route(&message, agent));
        messages.owner_say = true;
        assert!(messages.route(&message, agent));
        message.owner_id = Uuid::from_u128(3);
        assert!(messages.route(&message, agent));
        assert_eq!(messages.lines.len(), 1);
        message.owner_id = agent;
        messages.owner_say = false;
        message.chat_type = ChatType::Debug;
        for _ in 0..510 {
            assert!(messages.route(&message, agent));
        }
        assert_eq!(messages.lines.len(), 500);
        message.source_type = ChatSourceType::Agent;
        assert!(!messages.route(&message, agent));
    }
}
