//! LLFloaterChangeItemThumbnail: free dedicated thumbnail upload, followed
//! by an AIS patch and authoritative parent re-read (Firestorm, LGPL 2.1).

use super::operations::{Mutation, Operation, metadata_thumbnail};
use aurora_llsd::llsd_map;
use uuid::Uuid;

pub async fn upload(http: &reqwest::Client, cap: &str, item: Uuid, folder: bool, data: Vec<u8>) -> Result<Uuid, String> {
    if item.is_nil() || data.is_empty() || data.len() > 2 * 1024 * 1024 {
        return Err("L’image est vide ou trop volumineuse.".into());
    }
    let body = if folder {
        llsd_map! { "category_id" => item }
    } else {
        llsd_map! { "item_id" => item }
    };
    for attempt in 0..3 {
        let response = http
            .post(cap)
            .header("Content-Type", "application/llsd+xml")
            .header("Accept", "application/llsd+xml")
            .body(aurora_llsd::to_xml(&body))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| "Le service de vignettes ne répond pas.".to_owned())?;
        let response = read_reply(response).await?;
        let url = response["uploader"].as_str();
        if url.is_empty() {
            return Err("Le serveur n’a pas fourni de destination pour l’image.".into());
        }
        // Capability URLs are never included in diagnostics.
        let response = http
            .post(url)
            .header("Content-Type", "application/jp2")
            .timeout(std::time::Duration::from_secs(30))
            .body(data.clone())
            .send()
            .await;
        if let Ok(response) = response
            && response.status().is_success()
        {
            let reply = read_reply(response).await?;
            let asset = reply["new_asset"].as_uuid();
            if reply["state"].as_str() == "complete" && !asset.is_nil() {
                return Ok(asset);
            }
            return Err("Le serveur n’a pas confirmé le chargement de l’image.".into());
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    Err("L’image n’a pas pu être chargée après trois tentatives.".into())
}

async fn read_reply(mut response: reqwest::Response) -> Result<aurora_llsd::Llsd, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Le service de vignettes a refusé l’opération (HTTP {}).",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Réponse interrompue.".to_owned())? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err("Réponse de chargement trop volumineuse.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    aurora_llsd::from_xml(&bytes).map_err(|_| "Réponse de chargement invalide.".into())
}

pub fn patch(item: Uuid, folder: bool, parent: Uuid, asset: Uuid) -> Mutation {
    Mutation {
        operations: vec![Operation::Patch {
            id: item,
            folder,
            body: metadata_thumbnail(asset),
        }],
        refresh: vec![parent],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::Llsd;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn thumbnails_use_a_new_uploader_after_a_failed_binary_post() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("test server");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let upload_url = base.clone();
        let id = Uuid::from_u128(1);
        let asset = Uuid::from_u128(2);
        let task = tokio::spawn(async move {
            let replies = [
                (200, llsd_map! { "uploader" => format!("{upload_url}/first") }),
                (503, llsd_map! { "error" => "private details" }),
                (200, llsd_map! { "uploader" => format!("{upload_url}/second") }),
                (200, llsd_map! { "state" => "complete", "new_asset" => asset }),
            ];
            let mut requests = Vec::new();
            for (status, reply) in replies {
                let (mut socket, _) = listener.accept().await.expect("request");
                let mut bytes = Vec::new();
                let (start, length) = loop {
                    let mut chunk = [0u8; 4096];
                    let n = socket.read(&mut chunk).await.expect("read");
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        let length: usize = header
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, v)| v.trim().parse().ok())
                            })
                            .expect("length");
                        if bytes.len() >= end + 4 + length {
                            break (end + 4, length);
                        }
                    }
                };
                requests.push((
                    String::from_utf8_lossy(&bytes[..start]).into_owned(),
                    bytes[start..start + length].to_vec(),
                ));
                let body = aurora_llsd::to_xml(&reply);
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .expect("headers");
                socket.write_all(&body).await.expect("body");
            }
            requests
        });
        assert_eq!(
            upload(&reqwest::Client::new(), &base, id, true, vec![255, 79, 0, 1])
                .await
                .expect("upload"),
            asset
        );
        let requests = task.await.expect("server");
        for n in [0, 2] {
            let body = aurora_llsd::from_xml(&requests[n].1).expect("LLSD");
            assert_eq!(body["category_id"].as_uuid(), id);
            assert!(!body.has("item_id"));
        }
        for (n, path) in [(1, "first"), (3, "second")] {
            assert!(requests[n].0.starts_with(&format!("POST /{path} ")));
            assert!(requests[n].0.to_ascii_lowercase().contains("content-type: application/jp2"));
            assert_eq!(requests[n].1, [255, 79, 0, 1]);
        }
    }
    #[tokio::test]
    async fn an_upload_without_confirmation_never_supplies_an_asset() {
        let (url, task) = crate::outfits::tests::server(vec![(200, Llsd::new_map())]).await;
        let error = upload(&reqwest::Client::new(), &url, Uuid::from_u128(1), false, vec![1])
            .await
            .expect_err("no uploader");
        assert!(!error.contains(&url));
        assert_eq!(task.await.expect("server").len(), 1);
    }
    #[test]
    fn upload_patch_targets_the_actual_item_or_category() {
        let id = Uuid::from_u128(1);
        let parent = Uuid::from_u128(2);
        let asset = Uuid::from_u128(3);
        for folder in [false, true] {
            let change = patch(id, folder, parent, asset);
            assert_eq!(change.refresh, vec![parent]);
            assert!(
                matches!(&change.operations[0], Operation::Patch { id: actual, folder: f, body } if *actual == id && *f == folder && body["thumbnail"]["asset_id"].as_str() == asset.to_string())
            );
        }
    }
}
