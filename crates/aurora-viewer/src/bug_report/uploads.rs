//! Cellar-backed attachments. The public issue reserves exact file digests;
//! GitHub credentials are sent only to GitHub, never to Aurora's file API.

use super::github::{Failure, json};
use sha2::{Digest, Sha256};

pub const BASE: &str = "https://api.auroraviewer.com";
const MAX_BYTES: usize = 16 * 1024 * 1024;

pub struct Pending {
    pub number: u64,
    pub image: Option<Vec<u8>>,
    pub logs: Option<Vec<u8>>,
}

fn declaration(kind: &str, bytes: &[u8]) -> String {
    let digest: String = Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    format!("<!-- aurora-{kind}-v1 sha256={digest} -->")
}

pub fn declare(mut body: String, image: Option<&[u8]>, logs: Option<&[u8]>) -> Result<String, Failure> {
    for (kind, bytes) in [("screenshot", image), ("logs", logs)] {
        if let Some(bytes) = bytes {
            if bytes.is_empty() || bytes.len() > MAX_BYTES {
                return Err(Failure::Attachments);
            }
            body.push_str("\n\n");
            body.push_str(&declaration(kind, bytes));
        }
    }
    Ok(body)
}

pub async fn preflight(client: &reqwest::Client, base: &str, image: bool, logs: bool) -> Result<(), Failure> {
    let response = client
        .get(format!("{base}/health"))
        .send()
        .await
        .map_err(|_| Failure::Attachments)?;
    if !response.status().is_success() {
        return Err(Failure::Attachments);
    }
    let value = json(response).await.map_err(|_| Failure::Attachments)?;
    if (image && value["uploads_enabled"] != true) || (logs && value["private_logs_enabled"] != true) {
        return Err(Failure::Attachments);
    }
    Ok(())
}

pub async fn complete(client: &reqwest::Client, github: &str, base: &str, token: &str, pending: &Pending) -> Result<(), Failure> {
    let mut links = Vec::new();
    for (kind, route, extension, mime, bytes) in [
        ("screenshot", "images", "png", "image/png", &pending.image),
        ("logs", "logs", "zip", "application/zip", &pending.logs),
    ] {
        if let Some(bytes) = bytes {
            let url = format!("{base}/v1/{route}/issues/{}.{extension}", pending.number);
            // Exact bytes, no source path, filename, authorization or private
            // diagnostics in the image request. PUT is safe to repeat.
            let response = client
                .put(&url)
                .header("Content-Type", mime)
                .body(bytes.clone())
                .send()
                .await
                .map_err(|_| Failure::Attachments)?;
            if !response.status().is_success() {
                return Err(Failure::Attachments);
            }
            links.push((kind, bytes, url));
        }
    }
    let issue_url = format!("{github}/{}", pending.number);
    let response = client
        .get(&issue_url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .send()
        .await
        .map_err(|_| Failure::Attachments)?;
    if !response.status().is_success() {
        return Err(Failure::Attachments);
    }
    let value = json(response).await.map_err(|_| Failure::Attachments)?;
    let mut body = value["body"].as_str().ok_or(Failure::Attachments)?.to_owned();
    let mut changed = false;
    // Read the current body on each retry; preserve edits made on GitHub.
    for (kind, bytes, url) in links {
        if !body.contains(&declaration(kind, bytes)) {
            return Err(Failure::Attachments);
        }
        if !body.contains(&url) {
            body.push_str(&if kind == "screenshot" {
                format!("\n\n## Screenshot\n\n![Screenshot]({url})")
            } else {
                format!("\n\n## Private logs\n\n[Download logs (developer GitHub access required)]({url})")
            });
            changed = true;
        }
    }
    if changed {
        let response = client
            .patch(issue_url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10")
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&serde_json::json!({"body":body})).map_err(|_| Failure::Attachments)?)
            .send()
            .await
            .map_err(|_| Failure::Attachments)?;
        if !response.status().is_success() {
            return Err(Failure::Attachments);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    fn fixture(replies: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let response_base = base.clone();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let body = body.replace("__BASE__", &response_base);
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0; 1024];
                    let n = stream.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap_or("0")
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                requests.push(request);
            }
            requests
        });
        (base, server)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[test]
    fn attachments_do_not_send_github_credentials_and_retry_preserves_issue_edits() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let pending = Pending {
            number: 42,
            image: Some(b"SYNTHETIC_PNG".to_vec()),
            logs: Some(b"SYNTHETIC_PRIVATE_ZIP".to_vec()),
        };
        let original = declare("Edit made on GitHub".into(), pending.image.as_deref(), pending.logs.as_deref()).unwrap();
        let (base, server) = fixture(vec![
            (200, "{}".into()), (200, "{}".into()),
            (200, serde_json::json!({"number":42, "body":original}).to_string()), (200, "{}".into()),
            (200, "{}".into()), (200, "{}".into()),
            (200, serde_json::json!({"body":format!("{original}\n![Screenshot](__BASE__/v1/images/issues/42.png)\n[Private logs](__BASE__/v1/logs/issues/42.zip)")}).to_string()),
        ]);
        rt.block_on(complete(
            &client(),
            &format!("{base}/issues"),
            &base,
            "test-only-github-token",
            &pending,
        ))
        .unwrap();
        rt.block_on(complete(
            &client(),
            &format!("{base}/issues"),
            &base,
            "test-only-github-token",
            &pending,
        ))
        .unwrap();
        let requests = server.join().unwrap();
        for (index, bytes) in requests.iter().take(4).enumerate() {
            let request = String::from_utf8_lossy(bytes);
            if index < 2 {
                assert!(request.starts_with("PUT /v1/"));
                assert!(!request.to_ascii_lowercase().contains("authorization"));
                assert!(!request.contains("test-only-github-token"));
            } else {
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer test-only-github-token")
                );
            }
        }
        let patch = String::from_utf8_lossy(&requests[3]);
        let (_, body) = patch.split_once("\r\n\r\n").unwrap();
        let sent: serde_json::Value = serde_json::from_str(body).unwrap();
        let linked = sent["body"].as_str().unwrap();
        assert!(linked.starts_with("Edit made on GitHub"));
        assert!(linked.contains(&format!("![Screenshot]({base}/v1/images/issues/42.png)")));
        assert!(linked.contains(&format!("{base}/v1/logs/issues/42.zip")));
        assert!(!linked.contains("SYNTHETIC_PRIVATE_ZIP"));
        assert!(!linked.contains("test-only-github-token"));
        // Lost PATCH replies are recoverable: links already present produce no
        // second PATCH and never any POST that could create a duplicate issue.
        assert_eq!(requests.len(), 7);
        assert!(String::from_utf8_lossy(&requests[6]).starts_with("GET /issues/42 "));
    }

    #[test]
    fn disabled_service_is_rejected_before_issue_creation_and_failed_upload_never_patches() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (base, server) = fixture(vec![(200, r#"{"uploads_enabled":true,"private_logs_enabled":false}"#.into())]);
        assert_eq!(rt.block_on(preflight(&client(), &base, true, true)), Err(Failure::Attachments));
        assert!(String::from_utf8_lossy(&server.join().unwrap()[0]).starts_with("GET /health "));
        let pending = Pending {
            number: 42,
            image: Some(vec![1, 2, 3]),
            logs: None,
        };
        let (base, server) = fixture(vec![(503, "{}".into())]);
        assert_eq!(
            rt.block_on(complete(&client(), &format!("{base}/issues"), &base, "test-only", &pending)),
            Err(Failure::Attachments)
        );
        assert_eq!(server.join().unwrap().len(), 1);
        assert_eq!(declare("No attachments".into(), None, None).unwrap(), "No attachments");
        assert!(declare("x".into(), Some(&[]), None).is_err());
    }
}
