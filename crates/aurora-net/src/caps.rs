//! Capabilities: seed resolution and the `EventQueueGet` long-poll.

use aurora_llsd::{Llsd, from_xml, llsd_map, to_xml};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

pub const REQUESTED_CAPS: &[&str] = &[
    "AgentPreferences",
    "AgentProfile",
    "AvatarRenderInfo",
    "ChatSessionRequest",
    "EnvironmentSettings",
    "EventQueueGet",
    "ExtEnvironment",
    "FetchInventory2",
    "FetchInventoryDescendents2",
    "FetchLib2",
    "FetchLibDescendents2",
    "GetDisplayNames",
    "GetMesh",
    "GetMesh2",
    "GetTexture",
    "ModifyMaterialParams",
    "ObjectMedia",
    "ObjectMediaNavigate",
    "ParcelVoiceInfoRequest",
    "ProvisionVoiceAccountRequest",
    "RenderMaterials",
    "SetDisplayName",
    "SimulatorFeatures",
    "UpdateAvatarAppearance",
    "ViewerAsset",
    "ViewerStats",
    "VoiceSignalingRequest",
];

#[derive(Debug, thiserror::Error)]
pub enum CapsError {
    #[error("http: {0}")]
    Http(String),
    #[error("status {0}")]
    Status(u16),
    #[error("llsd: {0}")]
    Llsd(#[from] aurora_llsd::LlsdError),
}

pub async fn post_llsd(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Llsd, CapsError> {
    let resp = http
        .post(url)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(to_xml(body))
        .send()
        .await
        .map_err(|e| CapsError::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(CapsError::Status(status.as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| CapsError::Http(e.to_string()))?;
    if bytes.is_empty() {
        return Ok(Llsd::Undef);
    }
    Ok(from_xml(&bytes)?)
}

/// POST the list of wanted capabilities to the seed and return the map.
pub async fn resolve_seed(http: &reqwest::Client, seed: &str) -> Result<HashMap<String, String>, CapsError> {
    let req = Llsd::Array(REQUESTED_CAPS.iter().map(|s| Llsd::from(*s)).collect());
    let mut last_err = None;
    for attempt in 0..4 {
        match post_llsd(http, seed, &req).await {
            Ok(v) => {
                let mut caps = HashMap::new();
                if let Some(m) = v.as_map() {
                    for (k, val) in m {
                        let s = val.to_string_value();
                        if !s.is_empty() {
                            caps.insert(k.clone(), s);
                        }
                    }
                }
                return Ok(caps);
            }
            Err(e) => {
                log::warn!("seed capability request failed (attempt {attempt}): {e}");
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (attempt + 1))).await;
            }
        }
    }
    Err(last_err.unwrap_or(CapsError::Status(0)))
}

#[derive(Debug, Clone)]
pub struct EqEvent {
    pub sim: std::net::SocketAddr,
    pub message: String,
    pub body: Llsd,
}

/// Long-poll `EventQueueGet` until the receiver is dropped or the cap 404s.
pub async fn event_queue_loop(http: reqwest::Client, url: String, sim: std::net::SocketAddr, tx: mpsc::UnboundedSender<EqEvent>) {
    let mut ack: Llsd = Llsd::Undef;
    let mut failures = 0u32;
    loop {
        if tx.is_closed() {
            return;
        }
        let body = llsd_map! { "ack" => ack.clone(), "done" => false };
        let fut = http
            .post(&url)
            .header("Content-Type", "application/llsd+xml")
            .header("Accept", "application/llsd+xml")
            .timeout(Duration::from_secs(75))
            .body(to_xml(&body))
            .send();
        match fut.await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match status {
                    200 => {
                        failures = 0;
                        let Ok(bytes) = resp.bytes().await else {
                            continue;
                        };
                        let Ok(v) = from_xml(&bytes) else {
                            log::warn!("event queue: malformed LLSD");
                            continue;
                        };
                        ack = v["id"].clone();
                        for ev in v["events"].as_array() {
                            let e = EqEvent {
                                sim,
                                message: ev["message"].as_str().to_owned(),
                                body: ev["body"].clone(),
                            };
                            if tx.send(e).is_err() {
                                return;
                            }
                        }
                    }
                    // Timeouts on the server side: just poll again.
                    499 | 500 | 502 | 503 | 504 => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    404 | 410 => {
                        log::info!("event queue for {sim} closed ({status})");
                        return;
                    }
                    _ => {
                        failures += 1;
                        log::warn!("event queue status {status}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
            Err(e) => {
                if e.is_timeout() {
                    continue;
                }
                failures += 1;
                log::warn!("event queue error: {e}");
                tokio::time::sleep(Duration::from_millis((500 * failures.min(10)) as u64)).await;
            }
        }
        if failures > 20 {
            log::warn!("event queue for {sim} giving up");
            return;
        }
    }
}
