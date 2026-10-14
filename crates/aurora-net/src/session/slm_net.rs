//! Sending of the SLM API requests (crate::slm) on the region's
//! `DirectDelivery` capability, like LLMarketplaceData's coroutines: JSON
//! bodies with `Accept` and `Content-Type: application/json`, the answer read
//! as JSON whatever its status.

use crate::slm::{self, Method, SlmReply, SlmRequest};
use std::time::Duration;

/// The request on `cap` and its reading. No capability fails like the empty
/// URL of getSLMConnectURL does.
pub(super) async fn slm_request(http: &reqwest::Client, cap: Option<&str>, request: &SlmRequest) -> SlmReply {
    let Some(cap) = cap else {
        return SlmReply::Failed(slm::error(0, "No marketplace capability", &serde_json::Value::Null));
    };
    let url = request.url(cap);
    let builder = match request.method() {
        Method::Get => http.get(&url),
        Method::Post => http.post(&url),
        Method::Put => http.put(&url),
        Method::Delete => http.delete(&url),
    }
    .header("Accept", "application/json")
    .header("Content-Type", "application/json")
    .timeout(Duration::from_secs(30));
    let builder = match request.body() {
        Some(body) => builder.body(body.to_string()),
        None => builder,
    };
    match builder.send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let reason = resp.status().canonical_reason().unwrap_or("").to_owned();
            let text = resp.text().await.unwrap_or_default();
            // an empty or non-JSON body reads as nothing (LLSD undefined)
            let body = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            let reply = slm::reply(request, code, &reason, &body);
            if let SlmReply::Failed(_) = &reply {
                // log_SLM_warning: the request and the status, never the URL
                // (it carries the capability)
                log::warn!("SLM API: {} failed with status {code} {reason}", request.name());
            }
            reply
        }
        Err(e) => {
            let reason = e.without_url().to_string();
            log::warn!("SLM API: {} failed: {reason}", request.name());
            SlmReply::Failed(slm::error(0, &reason, &serde_json::Value::Null))
        }
    }
}
