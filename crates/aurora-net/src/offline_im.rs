//! Instant messages kept by the grid while the agent was offline, as the
//! `ReadOfflineMsgs` capability returns them: port of
//! LLIMProcessing::requestOfflineMessagesCoro (indra/newview/llimprocessing.cpp,
//! originally LGPL 2.1), which hands each entry to processNewMessage like an
//! ImprovedInstantMessage that just arrived.
//!
//! The answer is an array whose first element is the array of messages, or
//! a map with a "messages" array (both exist on the grid). An entry carries
//! no session id for friendship offers and group invitations: those are
//! answered through the AcceptFriendship / DeclineFriendship and
//! AcceptGroupInvite / DeclineGroupInvite capabilities, which is why
//! Firestorm only uses `ReadOfflineMsgs` when they are there.

use crate::types::InstantMessage;
use aurora_llsd::Llsd;
use uuid::Uuid;

/// IM_FROM_TASK.
const DIALOG_FROM_TASK: u8 = 19;

/// The capabilities Firestorm wants before it reads the offline messages
/// through `ReadOfflineMsgs` (else the legacy RetrieveInstantMessages).
pub const REQUIRED_CAPS: [&str; 3] = ["ReadOfflineMsgs", "AcceptFriendship", "AcceptGroupInvite"];

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum OfflineError {
    /// Nothing in the answer (Firestorm stops there, without a fallback).
    #[error("no contents")]
    Empty,
    #[error("invalid contents")]
    Invalid,
}

/// The messages of an answer of `ReadOfflineMsgs`, in the order given.
pub fn parse(contents: &Llsd) -> Result<Vec<InstantMessage>, OfflineError> {
    if contents.is_empty() {
        return Err(OfflineError::Empty);
    }
    let messages = if contents.is_array() {
        contents.at(0)
    } else if contents.has("messages") {
        contents.get("messages")
    } else {
        return Err(OfflineError::Invalid);
    };
    if !messages.is_array() {
        return Err(OfflineError::Invalid);
    }
    Ok(messages.as_array().iter().map(message).collect())
}

/// One entry as an instant message (the arguments of processNewMessage).
fn message(m: &Llsd) -> InstantMessage {
    // an integer, or "Y" / "N" from older simulators
    let from_group = match m.get("from_group") {
        Llsd::Integer(i) => *i != 0,
        other => other.as_str() == "Y",
    };
    let dialog = m.get("dialog").as_i32() as u8;
    let mut session_id = m.get("transaction-id").as_uuid();
    if session_id.is_nil() && dialog == DIALOG_FROM_TASK {
        session_id = m.get("asset_id").as_uuid();
    }
    InstantMessage {
        from_agent_id: m.get("from_agent_id").as_uuid(),
        from_name: m.get("from_agent_name").as_str().to_owned(),
        to_agent_id: m.get("to_agent_id").as_uuid(),
        dialog,
        session_id,
        message: m.get("message").as_str().to_owned(),
        // IM_OFFLINE unless the entry says otherwise
        offline: !m.has("offline") || m.get("offline").as_i32() != 0,
        from_group,
        binary_bucket: if m.has("binary_bucket") {
            m.get("binary_bucket").as_binary().to_vec()
        } else {
            vec![0]
        },
        timestamp: m.get("timestamp").as_i32() as u32,
    }
}

/// LLURI::escape with its default set: everything but the unreserved
/// characters becomes %XX.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// URL of the AcceptFriendship capability for an offer (accept_friendship_coro):
/// the query names who offered and our own full name, in quotes.
pub fn accept_friendship_url(cap: &str, from: &Uuid, agent_name: &str) -> String {
    format!("{cap}?from={from}&agent_name=\"{}\"", escape(agent_name))
}

/// URL of the DeclineFriendship capability (decline_friendship_coro), which
/// is called with DELETE.
pub fn decline_friendship_url(cap: &str, from: &Uuid) -> String {
    format!("{cap}?from={from}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn entry() -> Llsd {
        llsd_map! {
            "from_agent_id" => id(2),
            "from_agent_name" => "Jane Doe",
            "to_agent_id" => id(1),
            "dialog" => 0,
            "transaction-id" => id(0xABC),
            "timestamp" => 1_790_000_000,
            "message" => "are you there?",
            "from_group" => 0,
            "parent_estate_id" => 1,
            "region_id" => id(9),
            "local_x" => 12.0,
            "local_y" => 34.0,
            "local_z" => 56.0
        }
    }

    #[test]
    fn both_shapes_of_the_answer() {
        let list = Llsd::Array(vec![entry(), entry()]);
        // an array holding the array of messages
        let as_array = Llsd::Array(vec![list.clone()]);
        assert_eq!(parse(&as_array).unwrap().len(), 2);
        // a map with "messages"
        let as_map = llsd_map! { "messages" => list };
        let got = parse(&as_map).unwrap();
        assert_eq!(got.len(), 2);
        let m = &got[0];
        assert_eq!(m.from_agent_id, id(2));
        assert_eq!(m.from_name, "Jane Doe");
        assert_eq!(m.to_agent_id, id(1));
        assert_eq!(m.dialog, 0);
        assert_eq!(m.session_id, id(0xABC));
        assert_eq!(m.message, "are you there?");
        assert_eq!(m.timestamp, 1_790_000_000);
        // no "offline": IM_OFFLINE; no bucket: one zero byte
        assert!(m.offline);
        assert!(!m.from_group);
        assert_eq!(m.binary_bucket, [0]);
        // it travels through the LLSD XML of the capability unchanged
        let bytes = aurora_llsd::to_xml(&as_map);
        assert_eq!(parse(&aurora_llsd::from_xml(&bytes).unwrap()).unwrap(), got);
    }

    #[test]
    fn empty_and_invalid_answers() {
        assert_eq!(parse(&Llsd::Undef), Err(OfflineError::Empty));
        assert_eq!(parse(&Llsd::Array(Vec::new())), Err(OfflineError::Empty));
        assert_eq!(parse(&Llsd::new_map()), Err(OfflineError::Empty));
        assert_eq!(parse(&llsd_map! { "other" => 1 }), Err(OfflineError::Invalid));
        assert_eq!(parse(&llsd_map! { "messages" => "x" }), Err(OfflineError::Invalid));
        assert_eq!(parse(&Llsd::Array(vec![Llsd::from("x")])), Err(OfflineError::Invalid));
        // no message kept: an empty list, not an error
        assert_eq!(parse(&llsd_map! { "messages" => Llsd::Array(Vec::new()) }), Ok(Vec::new()));
    }

    #[test]
    fn fields_of_an_entry() {
        // a group notice with its bucket, "Y" from an older simulator
        let mut m = entry();
        m.insert("dialog", 32);
        m.insert("from_group", "Y");
        m.insert("binary_bucket", vec![1u8, 2, 3]);
        m.insert("offline", 0);
        let got = parse(&llsd_map! { "messages" => Llsd::Array(vec![m]) }).unwrap();
        assert_eq!(got[0].dialog, 32);
        assert!(got[0].from_group);
        assert!(!got[0].offline);
        assert_eq!(got[0].binary_bucket, [1, 2, 3]);
        // an object's message without a transaction: its asset id names it
        let mut m = entry();
        m.insert("dialog", 19);
        m.insert("transaction-id", Uuid::nil());
        m.insert("asset_id", id(0x0B1));
        let got = parse(&llsd_map! { "messages" => Llsd::Array(vec![m]) }).unwrap();
        assert_eq!(got[0].session_id, id(0x0B1));
        // a friendship offer has no session id: it is answered by capability
        let mut m = entry();
        m.insert("dialog", 38);
        m.insert("transaction-id", Uuid::nil());
        m.insert("asset_id", id(0x0B1));
        let got = parse(&llsd_map! { "messages" => Llsd::Array(vec![m]) }).unwrap();
        assert!(got[0].session_id.is_nil());
        // an entry that is not a map gives an empty message, not a failure
        let got = parse(&llsd_map! { "messages" => Llsd::Array(vec![Llsd::from(3)]) }).unwrap();
        assert!(got[0].from_agent_id.is_nil() && got[0].message.is_empty());
    }

    #[test]
    fn friendship_capability_urls() {
        let from = id(0x77);
        assert_eq!(
            accept_friendship_url("https://sim.example/cap/x", &from, "Jane Doe"),
            format!("https://sim.example/cap/x?from={from}&agent_name=\"Jane%20Doe\"")
        );
        assert_eq!(escape("Éa-b_c.d~e&f"), "%C3%89a-b_c.d~e%26f");
        assert_eq!(
            decline_friendship_url("https://sim.example/cap/y", &from),
            format!("https://sim.example/cap/y?from={from}")
        );
    }
}
