//! RlvUtil::sendChatReply (indra/newview/rlvcommon.cpp, LGPL 2.1): one
//! reliable shout on a positive script channel, including an empty answer.
use aurora_msg::{msgs::ChatFromViewer, str_field};
use uuid::Uuid;

pub(crate) fn packet(agent: Uuid, session: Uuid, channel: i32, message: &str) -> Option<ChatFromViewer> {
    if channel <= 0 || channel == i32::MAX {
        return None;
    }
    let mut end = message.len().min(1023);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    let mut packet = ChatFromViewer::default();
    packet.agent_data.agent_id = agent;
    packet.agent_data.session_id = session;
    packet.chat_data.channel = channel;
    packet.chat_data.type_ = 2; // CHAT_TYPE_SHOUT
    packet.chat_data.message = str_field(&message[..end]);
    Some(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_msg::{Msg, field_str};

    #[test]
    fn rlva_replies_encode_empty_answers_and_truncate_once_at_a_utf8_boundary() {
        let agent = Uuid::from_u128(1);
        let session = Uuid::from_u128(2);
        for text in [String::new(), "é".repeat(1000), "a".repeat(1024)] {
            let packet = packet(agent, session, 123, &text).unwrap();
            let mut writer = aurora_msg::Writer::default();
            packet.encode_body(&mut writer);
            let decoded = ChatFromViewer::decode(&writer.buf).unwrap();
            assert_eq!(decoded.agent_data.agent_id, agent);
            assert_eq!(decoded.agent_data.session_id, session);
            assert_eq!(decoded.chat_data.channel, 123);
            assert_eq!(decoded.chat_data.type_, 2);
            let reply = field_str(&decoded.chat_data.message);
            assert!(reply.len() <= 1023 && text.starts_with(&reply));
            assert_eq!(decoded.chat_data.message.len(), reply.len() + 1);
            assert_eq!(
                reply.len(),
                if text.is_empty() {
                    0
                } else if text.starts_with('é') {
                    1022
                } else {
                    1023
                }
            );
        }
        for channel in [0, -1, i32::MAX] {
            assert!(packet(agent, session, channel, "").is_none());
        }
    }
}
