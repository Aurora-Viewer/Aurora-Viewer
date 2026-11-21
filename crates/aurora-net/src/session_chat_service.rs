//! Session side of the grid's direct IM history (LLChatServiceHistory,
//! indra/newview/llchatservicehistory.cpp, LL viewer 26.5): the transport
//! of one request. What to ask, when and how often is the viewer's
//! (`world::chat_service`); this side only sends and reports, so that the
//! pacing, retries and validation stay in one testable place.

use super::Session;
use crate::probe::{self, HttpKind};
use crate::types::{NetCommand, NetEvent};
use aurora_llsd::Llsd;
use std::time::Duration;

/// HttpOptions::setTimeout(30) of LLChatServiceHistory::request.
const TIMEOUT: Duration = Duration::from_secs(30);

/// LLCore's status for a body that does not parse as the LLSD its
/// Content-Type announces ("Failed to deserialize",
/// LLCoreHttpUtil::HttpCoroHandler::onCompleted, llcorehttputil.cpp).
const STATUS_BAD_LLSD: u16 = 499;

impl Session<'_> {
    /// Commands of this module; false when `c` is not one of them.
    pub(super) fn on_chat_service_command(&mut self, c: &NetCommand) -> bool {
        let NetCommand::ChatServiceRequest { id, url, body } = c else {
            return false;
        };
        let (id, url, body) = (*id, url.clone(), body.clone());
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let (status, body) = request(&http, &url, body.as_ref()).await;
            let _ = events.send(NetEvent::ChatServiceResponse { id, status, body });
        });
        true
    }
}

/// LLChatServiceHistory::request: no transport retry (setRetries(0)), the
/// adapter's LLSD headers on the GET as on the POST
/// (HttpCoroutineAdapter::checkDefaultHeaders), the body kept on a success
/// only. The URL is never logged.
async fn request(http: &reqwest::Client, url: &str, body: Option<&Llsd>) -> (u16, Option<Llsd>) {
    let req = match body {
        Some(body) => http.post(url).body(aurora_llsd::to_xml(body)),
        None => http.get(url),
    };
    let req = req
        .header("Accept", "application/llsd+xml")
        .header("Content-Type", "application/llsd+xml")
        .timeout(TIMEOUT);
    let timed = probe::HttpRequest::start(HttpKind::Cap);
    let finish = |timed: Option<probe::HttpRequest>, status: u16, bytes: usize, ok: bool| {
        if let Some(t) = timed {
            t.finish(status, bytes, ok);
        }
    };
    let resp = match req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            log::info!("chat service history: request failed ({})", e.without_url());
            finish(timed, 0, 0, false);
            return (0, None);
        }
    };
    let status = resp.status().as_u16();
    let llsd_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().to_ascii_lowercase().starts_with("application/llsd+xml"));
    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            log::info!("chat service history: reading the answer failed ({})", e.without_url());
            finish(timed, 0, 0, false);
            return (0, None);
        }
    };
    let (status, body) = interpret(status, llsd_type, &bytes);
    finish(timed, status, bytes.len(), (200..300).contains(&status));
    (status, body)
}

/// The status and body LLCoreHttpUtil hands to the coroutine
/// (HttpCoroHandler::onCompleted, llcorehttputil.cpp:470-520): an empty body
/// is undefined; a body that does not parse turns the status into 499 when
/// it was announced as LLSD, else becomes an empty map; anything but a
/// success keeps no body (LLChatServiceHistory::request, CS:974).
fn interpret(status: u16, llsd_type: bool, bytes: &[u8]) -> (u16, Option<Llsd>) {
    let parsed = if bytes.is_empty() {
        Llsd::Undef
    } else {
        match aurora_llsd::from_xml(bytes) {
            Ok(v) => v,
            Err(_) if llsd_type => return (STATUS_BAD_LLSD, None),
            Err(_) => Llsd::Map(Default::default()),
        }
    };
    if (200..300).contains(&status) {
        (status, Some(parsed))
    } else {
        (status, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_success_is_undefined() {
        assert_eq!(interpret(200, true, b""), (200, Some(Llsd::Undef)));
    }

    #[test]
    fn a_bad_llsd_body_announced_as_llsd_is_499() {
        assert_eq!(interpret(200, true, b"<llsd><map>"), (STATUS_BAD_LLSD, None));
    }

    #[test]
    fn a_bad_body_of_another_type_is_an_empty_map() {
        assert_eq!(interpret(200, false, b"not xml"), (200, Some(Llsd::Map(Default::default()))));
    }

    #[test]
    fn an_error_status_keeps_no_body() {
        assert_eq!(interpret(429, true, b"<llsd><map></map></llsd>"), (429, None));
    }

    #[test]
    fn an_array_root_is_given_as_it_is() {
        let (status, body) = interpret(200, true, b"<llsd><array><string>a</string></array></llsd>");
        assert_eq!(status, 200);
        assert!(matches!(body, Some(Llsd::Array(ref a)) if a.len() == 1));
    }
}
