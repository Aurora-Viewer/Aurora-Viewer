//! RlvUtil::sendBusyMessage/sendIMMessage (rlvcommon.cpp, LGPL 2.1).
use aurora_msg::{msgs::ImprovedInstantMessage, str_field};
use uuid::Uuid;

pub(crate) fn packet(
    agent: Uuid,
    session: Uuid,
    name: &str,
    to: Uuid,
    message: &str,
    busy: bool,
    offline: bool,
) -> Option<ImprovedInstantMessage> {
    if to.is_nil() {
        return None;
    }
    let mut end = message.len().min(1023);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    let mut packet = ImprovedInstantMessage::default();
    packet.agent_data.agent_id = agent;
    packet.agent_data.session_id = session;
    let body = &mut packet.message_block;
    body.to_agent_id = to;
    body.dialog = if busy { 20 } else { 0 };
    body.offline = u8::from(!busy && offline);
    body.id = if busy {
        Uuid::nil()
    } else {
        Uuid::from_u128(agent.as_u128() ^ to.as_u128())
    };
    body.from_agent_name = str_field(name);
    body.message = str_field(&message[..end]);
    body.binary_bucket = vec![0];
    Some(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_msg::{Msg, field_str};

    #[test]
    fn rlva_im_replies_encode_busy_and_normal_sessions_offline_flags_and_utf8_bounds() {
        let agent = Uuid::from_u128(1);
        let session = Uuid::from_u128(2);
        let to = Uuid::from_u128(3);
        for busy in [false, true] {
            for offline in [false, true] {
                let message = "é".repeat(1000);
                let packet = packet(agent, session, "Demo Resident", to, &message, busy, offline).unwrap();
                let mut writer = aurora_msg::Writer::default();
                packet.encode_body(&mut writer);
                let decoded = ImprovedInstantMessage::decode(&writer.buf).unwrap();
                assert_eq!(decoded.agent_data.agent_id, agent);
                assert_eq!(decoded.agent_data.session_id, session);
                assert_eq!(decoded.message_block.to_agent_id, to);
                assert_eq!(decoded.message_block.id, if busy { Uuid::nil() } else { Uuid::from_u128(2) });
                assert_eq!(decoded.message_block.dialog, if busy { 20 } else { 0 });
                assert_eq!(decoded.message_block.offline, u8::from(!busy && offline));
                assert_eq!(field_str(&decoded.message_block.message).len(), 1022);
                assert_eq!(decoded.message_block.binary_bucket, [0]);
            }
        }
        assert!(packet(agent, session, "", Uuid::nil(), "", false, false).is_none());
    }
}
