//! Real capability handshake, binary upload and read-back against a local mock.
use super::operations::save_content;
use aurora_assets::gesture::{Gesture, Step};
use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

async fn server(stale: bool, reject: bool, material: bool) -> (String, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local server");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let replies = vec![
        (200, llsd_map! { "state" => "upload", "uploader" => format!("{url}/upload") }),
        (
            if reject { 503 } else { 200 },
            llsd_map! { "state" => "complete", "new_asset" => Uuid::from_u128(40) },
        ),
        (
            200,
            llsd_map! { "items" => Llsd::Array(vec![llsd_map! { "item_id" => Uuid::from_u128(3), "parent_id" => Uuid::from_u128(1), "type" => if material {57} else {21}, "inv_type" => if material {26} else {20}, "asset_id" => Uuid::from_u128(if stale { 4 } else { 40 }), "name" => "Content", "permissions" => llsd_map! { "owner_id" => Uuid::from_u128(2) } }]) },
        ),
    ];
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, reply) in replies.into_iter().take(if reject { 2 } else { 3 }) {
            let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                .await
                .expect("request timeout")
                .expect("request");
            let mut bytes = Vec::new();
            let (start, len) = loop {
                let mut buffer = [0u8; 4096];
                let n = socket.read(&mut buffer).await.expect("read");
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let len = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        })
                        .expect("length");
                    if bytes.len() >= end + 4 + len {
                        break (end + 4, len);
                    }
                }
            };
            requests.push(bytes[start..start + len].to_vec());
            let body = aurora_llsd::to_xml(&reply);
            let headers = format!(
                "HTTP/1.1 {status} Test\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.expect("headers");
            socket.write_all(&body).await.expect("body");
            crate::test_http::finish(socket);
        }
        requests
    });
    (url, task)
}

#[tokio::test]
async fn gesture_upload_preserves_bytes_and_requires_matching_inventory_asset() {
    let data = Gesture {
        steps: vec![Step::Chat {
            text: "/28574 T4".into(),
            flags: 0,
        }],
        ..Default::default()
    }
    .encode()
    .expect("encode");
    for (stale, reject) in [(false, false), (true, false), (false, true)] {
        let (url, task) = server(stale, reject, false).await;
        let result = save_content(
            &reqwest::Client::new(),
            &url,
            &format!("{url}/fetch"),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            21,
            data.clone(),
        )
        .await;
        assert_eq!(result.is_ok(), !stale && !reject);
        let requests = task.await.expect("server");
        let negotiation = aurora_llsd::from_xml(&requests[0]).expect("LLSD");
        assert_eq!(negotiation["item_id"].as_uuid(), Uuid::from_u128(3));
        assert_eq!(requests[1], data);
        if !stale && !reject {
            assert_eq!(result.expect("saved").asset_id, Uuid::from_u128(40));
        }
    }
}

#[tokio::test]
async fn material_upload_preserves_asset_bytes_and_requires_matching_inventory_asset() {
    let material = aurora_assets::PbrMaterial {
        roughness_factor: 0.35,
        normal_texture: Some(Uuid::from_u128(50)),
        ..Default::default()
    };
    let data = material.encode_asset().expect("material asset");
    for (stale, reject) in [(false, false), (true, false), (false, true)] {
        let (url, task) = server(stale, reject, true).await;
        let result = save_content(
            &reqwest::Client::new(),
            &url,
            &format!("{url}/fetch"),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            57,
            data.clone(),
        )
        .await;
        assert_eq!(result.is_ok(), !stale && !reject, "{result:?}");
        let requests = task.await.expect("server");
        assert_eq!(requests[1], data);
        assert_eq!(aurora_assets::parse_material_asset(&requests[1]).expect("read upload"), material);
        let negotiation = aurora_llsd::from_xml(&requests[0]).expect("LLSD");
        assert_eq!(negotiation["item_id"].as_uuid(), Uuid::from_u128(3));
        assert!(!negotiation.has("target"));
    }
    assert!(
        save_content(
            &reqwest::Client::new(),
            "invalid",
            "invalid",
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            57,
            b"invalid".to_vec()
        )
        .await
        .is_err()
    );
}
