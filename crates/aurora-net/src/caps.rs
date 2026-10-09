//! Capabilities: seed resolution and the `EventQueueGet` long-poll.

use aurora_llsd::{Llsd, from_xml, llsd_map, to_xml};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

pub const REQUESTED_CAPS: &[&str] = &[
    "AgentPreferences",
    "AgentProfile",
    "AvatarPickerSearch",
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
    // LLViewerRegionImpl::buildCapabilityNames (indra/newview/llviewerregion.cpp,
    // originally LGPL 2.1): advertise support so the simulator sends the
    // ObjectAnimation UDP messages. This capability needs no HTTP request.
    "ObjectAnimation",
    "ObjectMedia",
    "ObjectMediaNavigate",
    "ParcelVoiceInfoRequest",
    "ProvisionVoiceAccountRequest",
    "RenderMaterials",
    "RequestTaskInventory",
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn seed_post_advertises_object_animation_without_requiring_a_cap_url() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local seed server");
        let seed = format!("http://{}/seed", listener.local_addr().expect("local address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("seed client");
            let mut request = Vec::new();
            let body_start = loop {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.expect("read seed request");
                assert_ne!(n, 0, "incomplete HTTP headers");
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..body_start]).expect("HTTP headers");
            assert!(headers.starts_with("POST /seed HTTP/1.1\r\n"));
            let body_len: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().expect("body length"))
                })
                .expect("content-length");
            while request.len() < body_start + body_len {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.expect("read seed body");
                assert_ne!(n, 0, "incomplete LLSD request");
                request.extend_from_slice(&buf[..n]);
            }
            let requested = from_xml(&request[body_start..body_start + body_len]).expect("seed LLSD");
            let supports_animesh = requested.as_array().iter().any(|v| v.as_str() == "ObjectAnimation");
            // Support markers may not return a URL. Asset caps must still resolve.
            let body = to_xml(&llsd_map! { "ViewerAsset" => "http://127.0.0.1/asset" });
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.expect("write headers");
            socket.write_all(&body).await.expect("write LLSD");
            supports_animesh
        });
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("HTTP client");
        let resolved = resolve_seed(&http, &seed).await.expect("seed response");
        assert!(server.await.expect("seed server"), "simulator was not told to send ObjectAnimation");
        assert_eq!(resolved.get("ViewerAsset").map(String::as_str), Some("http://127.0.0.1/asset"));
        assert!(!resolved.contains_key("ObjectAnimation"));
    }
}
