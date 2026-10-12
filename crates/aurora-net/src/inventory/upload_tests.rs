use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(replies: impl FnOnce(&str) -> Vec<(u16, Llsd)>) -> (String, tokio::task::JoinHandle<Vec<(String, Vec<u8>)>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local server");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let replies = replies(&base);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, value) in replies {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .expect("request deadline")
                .expect("request");
            let mut bytes = Vec::new();
            let (start, length) = loop {
                let mut chunk = [0u8; 4096];
                let count = socket.read(&mut chunk).await.expect("read");
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let length = header
                        .lines()
                        .filter_map(|l| l.split_once(':'))
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break (end + 4, length);
                    }
                }
                assert!(bytes.len() < 2 * 1024 * 1024);
            };
            requests.push((
                String::from_utf8_lossy(&bytes[..start]).into_owned(),
                bytes[start..start + length].to_vec(),
            ));
            let body = aurora_llsd::to_xml(&value);
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
    (base, task)
}
fn request() -> Request {
    Request {
        kind: Kind::Animation,
        folder: Uuid::from_u128(1),
        name: "Motion".into(),
        description: "Test".into(),
        permissions: Permissions::default(),
        expected_cost: 10,
        data: vec![1, 2, 3, 4],
    }
}
fn item(asset_type: i32, inventory_type: i32, asset: Uuid) -> Llsd {
    llsd_map! { "items"=>Llsd::Array(vec![llsd_map! { "item_id"=>Uuid::from_u128(2),"parent_id"=>Uuid::from_u128(1),
    "asset_id"=>asset,"type"=>asset_type,"inv_type"=>inventory_type,"permissions"=>llsd_map! { "owner_id"=>Uuid::from_u128(4),"next_owner_mask"=>123 } }]) }
}
#[tokio::test]
async fn paid_assets_negotiate_send_binary_and_use_authoritative_inventory() {
    let asset = Uuid::from_u128(3);
    let (base, task) = server(|base| {
        vec![
            (
                200,
                llsd_map! {"state"=>"upload","uploader"=>format!("{base}/binary"),"upload_price"=>10},
            ),
            (
                200,
                llsd_map! { "state"=>"complete","new_inventory_item"=>Uuid::from_u128(2),"new_asset"=>asset,"upload_price"=>10 },
            ),
            (200, item(20, 19, asset)),
        ]
    })
    .await;
    let receipt = upload(
        &reqwest::Client::new(),
        &base,
        &format!("{base}/fetch"),
        Uuid::from_u128(4),
        request(),
    )
    .await
    .expect("upload");
    assert_eq!(receipt.item.next_owner_mask, 123);
    assert_eq!(receipt.item.asset_id, asset);
    let requests = task.await.expect("server");
    let body = aurora_llsd::from_xml(&requests[0].1).expect("LLSD");
    assert_eq!(body["asset_type"].as_str(), "animatn");
    assert_eq!(body["inventory_type"].as_str(), "animation");
    assert!(requests[1].0.starts_with("POST /binary "));
    assert!(
        requests[1]
            .0
            .to_ascii_lowercase()
            .contains("content-type: application/octet-stream")
    );
    assert_eq!(requests[1].1, [1, 2, 3, 4]);
    assert_eq!(
        aurora_llsd::from_xml(&requests[2].1).expect("fetch")["items"].at(0)["item_id"].as_uuid(),
        Uuid::from_u128(2)
    );
}
#[tokio::test]
async fn changed_fee_prevents_binary_transfer_and_unknown_outcome_never_retries() {
    let (base, task) = server(|base| {
        vec![(
            200,
            llsd_map! {"state"=>"upload","uploader"=>format!("{base}/binary"),"upload_price"=>40},
        )]
    })
    .await;
    assert_eq!(
        upload(&reqwest::Client::new(), &base, &base, Uuid::nil(), request())
            .await
            .expect_err("price"),
        Error::PriceChanged(Some(40))
    );
    assert_eq!(task.await.expect("server").len(), 1);
    let (base, task) = server(|base| {
        vec![
            (200, llsd_map! {"state"=>"upload","uploader"=>format!("{base}/once")}),
            (500, llsd_map! {"error"=>"private server details"}),
        ]
    })
    .await;
    let error = upload(&reqwest::Client::new(), &base, &base, Uuid::nil(), request())
        .await
        .expect_err("unknown");
    assert_eq!(error, Error::Unknown);
    assert!(!error.to_string().contains(&base));
    assert_eq!(task.await.expect("server").len(), 2);
}
#[tokio::test]
async fn mesh_permission_quote_and_llsd_transfer_create_an_object() {
    use crate::inventory::mesh_upload as mesh;
    let asset = Uuid::from_u128(3);
    let resources = llsd_map! {"mesh_list"=>Llsd::Array(vec![Llsd::Binary(vec![7,8])]),"instance_list"=>Llsd::Array(Vec::new())};
    let upload = mesh::Request {
        folder: Uuid::from_u128(1),
        texture_folder: Uuid::from_u128(5),
        query: std::sync::Arc::new(aurora_llsd::to_xml(&llsd_map! {"asset_type"=>"mesh"})),
        resources: std::sync::Arc::new(aurora_llsd::to_xml(&resources)),
    };
    let (base,task) = server(|base|vec![(200,llsd_map! {"mesh_upload_status"=>"valid"}),
        (200,llsd_map! {"state"=>"upload","uploader"=>format!("{base}/mesh"),"upload_price"=>17,"data"=>llsd_map! {"resource_cost"=>2.0}}),
        (200,llsd_map! {"state"=>"complete","new_inventory_item"=>Uuid::from_u128(2),"new_asset"=>asset}), (200,item(6,6,asset))]).await;
    let client = reqwest::Client::new();
    let offer = mesh::quote(&client, &base, &format!("{base}/permission"), &upload)
        .await
        .expect("quote");
    assert_eq!(offer.cost, 17);
    assert_eq!(offer.data["resource_cost"].as_f64(), 2.0);
    let receipt = mesh::upload(
        &client,
        offer.uploader,
        &format!("{base}/fetch"),
        Uuid::from_u128(4),
        offer.cost,
        upload,
    )
    .await
    .expect("mesh");
    assert_eq!(receipt.item.asset_type, 6);
    assert_eq!(receipt.cost, 17);
    let requests = task.await.expect("server");
    assert!(requests[0].0.starts_with("GET /permission "));
    assert_eq!(aurora_llsd::from_xml(&requests[2].1).expect("resources"), resources);
}
#[tokio::test]
async fn mesh_refuses_unqualified_accounts_without_a_fee_query() {
    use crate::inventory::mesh_upload as mesh;
    let (base, task) = server(|_| vec![(200, llsd_map! {"mesh_upload_status"=>"invalid"})]).await;
    let upload = mesh::Request {
        folder: Uuid::from_u128(1),
        texture_folder: Uuid::from_u128(5),
        query: std::sync::Arc::new(vec![1]),
        resources: std::sync::Arc::new(vec![1]),
    };
    assert_eq!(
        mesh::quote(&reqwest::Client::new(), &base, &base, &upload).await.err(),
        Some(Error::Rejected)
    );
    assert_eq!(task.await.expect("server").len(), 1);
}
