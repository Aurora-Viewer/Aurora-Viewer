//! Participant discovery from Chat Modernization. Port of requestList and
//! validateConversationList / parseTimeUuid in indra/newview/
//! llchatservicehistory.cpp and llchatservicehistorycore.cpp (LGPL 2.1).
//! Only the conversation list is read; this does not change IM delivery.

use aurora_llsd::Llsd;
use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const CAP: &str = "PersistentChatConversationsRequest";
const MAX_BYTES: usize = 16 * 1024 * 1024;
const UUID_EPOCH: u64 = 122_192_928_000_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    pub id: Uuid,
    pub when: SystemTime,
}

fn text<'a>(value: &'a Llsd, key: &str) -> Option<&'a str> {
    match value.get(key) {
        Llsd::String(s) => Some(s),
        _ => None,
    }
}

fn canonical(text: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(text).ok()?;
    (id.to_string() == text).then_some(id)
}

fn timestamp(token: &str) -> Option<SystemTime> {
    let id = canonical(token)?;
    let b = id.as_bytes();
    if b[6] >> 4 != 1 || b[8] & 0xc0 != 0x80 {
        return None;
    }
    let low = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64;
    let mid = u16::from_be_bytes([b[4], b[5]]) as u64;
    let high = u16::from_be_bytes([b[6] & 0x0f, b[7]]) as u64;
    let ticks = ((high << 48) | (mid << 32) | low).checked_sub(UUID_EPOCH)?;
    UNIX_EPOCH.checked_add(Duration::new(ticks / 10_000_000, ((ticks % 10_000_000) * 100) as u32))
}

/// Validate the entire response before publishing any participant. Known group
/// and ad-hoc rows are ignored; system traffic's nil participant is excluded.
pub fn parse(value: &Llsd, agent: Uuid) -> Option<Vec<Participant>> {
    let Llsd::Array(rows) = value else { return None };
    if rows.len() > 10_000 {
        return None;
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for row in rows {
        let kind = text(row, "conversation_type")?;
        let conversation = text(row, "conversation_id")?;
        if conversation.is_empty() || conversation.len() > 512 || conversation.contains('\0') {
            return None;
        }
        match kind {
            "group" | "adhoc" => continue,
            "direct" => {}
            _ => return None,
        }
        let id = canonical(text(row, "other_participant_id")?)?;
        let when = timestamp(text(row, "last_msg_id")?)?;
        let expected = if agent < id {
            format!("{agent}_{id}")
        } else {
            format!("{id}_{agent}")
        };
        if id == agent || conversation != expected || !seen.insert(id) {
            return None;
        }
        if !id.is_nil() {
            result.push(Participant { id, when });
        }
    }
    Some(result)
}

pub(crate) async fn fetch(http: &reqwest::Client, url: &str, agent: Uuid) -> Result<Vec<Participant>, &'static str> {
    // No retries: one discovery per login, including 429 responses. A failed
    // lookup never erases locally saved participants or alters IM transport.
    let mut response = http
        .get(url)
        .header("Accept", "application/llsd+xml")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "transport failure")?;
    if !response.status().is_success() {
        return Err("HTTP failure");
    }
    if response.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
        return Err("response too large");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "body failure")? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("response too large");
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = aurora_llsd::from_xml(&bytes).map_err(|_| "invalid LLSD")?;
    parse(&body, agent).ok_or("invalid conversation list")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn row(agent: Uuid, id: Uuid) -> Llsd {
        let conversation = if agent < id {
            format!("{agent}_{id}")
        } else {
            format!("{id}_{agent}")
        };
        llsd_map! {
            "conversation_type" => "direct", "conversation_id" => conversation,
            "other_participant_id" => id.to_string(),
            "last_msg_id" => "13814000-1dd2-11b2-8000-000000000001"
        }
    }

    #[test]
    fn persistent_chat_validates_identity_time_and_excludes_non_residents() {
        let me = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let data = Llsd::Array(vec![
            row(me, other),
            row(me, Uuid::nil()),
            llsd_map! {
                "conversation_type" => "group", "conversation_id" => "group"
            },
        ]);
        assert_eq!(
            parse(&data, me),
            Some(vec![Participant {
                id: other,
                when: UNIX_EPOCH
            }])
        );
        assert!(parse(&data, Uuid::from_u128(3)).is_none());
        assert!(parse(&Llsd::Array(vec![row(me, other), row(me, other)]), me).is_none());
        assert!(parse(&Llsd::Array(vec![row(me, me)]), me).is_none());
        assert!(parse(&Llsd::Undef, me).is_none());
        assert!(timestamp(&other.to_string()).is_none());
    }

    #[tokio::test]
    async fn persistent_chat_fetches_only_the_list_and_rejects_http_errors() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let me = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = aurora_llsd::to_xml(&Llsd::Array(vec![row(me, other)]));
        let server = tokio::spawn(async move {
            for status in ["200 OK", "429 Too Many Requests"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..n]).starts_with("GET /participants HTTP/1.1"));
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
                crate::test_http::finish(socket);
            }
        });
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{address}/participants");
        assert_eq!(fetch(&http, &url, me).await.unwrap()[0].id, other);
        assert!(fetch(&http, &url, me).await.is_err());
        server.await.unwrap();
    }
}
