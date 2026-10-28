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

fn mesh_request() -> crate::inventory::mesh_upload::Request {
    crate::inventory::mesh_upload::Request {
        folder: Uuid::from_u128(1),
        texture_folder: Uuid::from_u128(5),
        name: "Mesh fixture".into(),
        description: "Test object".into(),
        query: std::sync::Arc::new(vec![1]),
        resources: std::sync::Arc::new(vec![1]),
        timeout_secs: 600,
    }
}

#[tokio::test]
async fn mesh_fee_errors_keep_http_and_validation_reasons_without_private_response_fields() {
    use crate::inventory::mesh_upload as mesh;
    for status in [200, 400] {
        let (base, task) = server(|base| vec![
            (200, llsd_map! {"mesh_upload_status"=>"valid"}),
            (status, llsd_map! {"state"=>"error", "uploader"=>format!("{base}/secret-uploader"), "error"=>llsd_map! {
                "identifier"=>"MAV_FOUND_DEGENERATE_TRIANGLES", "message"=>format!("Invalid physics {base}/secret-cap"),
                "errors"=>Llsd::Array(vec![llsd_map! {"message"=>"Degenerate triangles"},llsd_map! {"message"=>"Degenerate triangles"}])
            }}),
        ]).await;
        let error = mesh::quote(&reqwest::Client::new(), &base, &base, &mesh_request())
            .await
            .err()
            .expect("fee rejected");
        let Error::Refused(reason) = &error else {
            panic!("reason lost: {error}")
        };
        assert_eq!(reason.status, (status == 400).then_some(400));
        assert_eq!(reason.identifier, "MAV_FOUND_DEGENERATE_TRIANGLES");
        assert_eq!(reason.messages, ["Invalid physics …", "Degenerate triangles"]);
        assert!(!format!("{error:?}").contains(&base));
        assert!(!format!("{error:?}").contains("secret"));
        assert_eq!(task.await.expect("server").len(), 2);
    }
}

#[tokio::test]
async fn failed_mesh_transfer_preserves_the_reason_and_an_uncertain_outcome_without_retrying() {
    let (base, task) = server(|_| vec![(500, llsd_map! {"error"=>llsd_map! {"message"=>"Mesh processing unavailable"}})]).await;
    let error = crate::inventory::mesh_upload::upload(&reqwest::Client::new(), base.clone(), Uuid::nil(), 10, mesh_request())
        .await
        .expect_err("uncertain outcome");
    let Error::Uncertain(reason) = error else {
        panic!("outcome warning lost")
    };
    assert_eq!(reason.status, Some(500));
    assert_eq!(reason.messages, ["Mesh processing unavailable"]);
    assert_eq!(task.await.expect("server").len(), 1);
}

#[test]
fn server_rejection_fields_are_bounded_and_paths_and_control_characters_are_removed() {
    let value = llsd_map! {"error"=>llsd_map! {"identifier"=>"https://secret.invalid/token", "message"=>"Invalid G:\\private\\file.dae \u{1b}geometry",
    "errors"=>Llsd::Array((0..20).map(|n| llsd_map! {"message"=>format!("{n}:{}", "é".repeat(800))}).collect())}};
    let reason = Rejection::from_response(Some(400), &value);
    assert!(reason.identifier.is_empty());
    assert_eq!(reason.messages[0], "Invalid … geometry");
    assert_eq!(reason.messages.len(), 9);
    assert!(reason.messages.iter().all(|message| message.chars().count() <= 512));
    assert!(!format!("{reason:?}").contains("private"));
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
/// LLSnapshotLivePreview::saveTexture: a texture asset of inventory type
/// "snapshot" in the Photo Album, every right for the next owner, the
/// group / everyone masks of the uploads.
#[tokio::test]
async fn a_photo_uploads_as_a_snapshot_texture_with_its_permissions() {
    let asset = Uuid::from_u128(3);
    let (base, task) = server(|base| {
        vec![
            (
                200,
                llsd_map! {"state"=>"upload","uploader"=>format!("{base}/binary"),"upload_price"=>10},
            ),
            (
                200,
                llsd_map! { "state"=>"complete","new_inventory_item"=>Uuid::from_u128(2),"new_asset"=>asset },
            ),
            (200, item(0, 15, asset)),
        ]
    })
    .await;
    let photo = Request {
        kind: Kind::Snapshot,
        name: "Snapshot : Place, Region (128, 128, 25) - General".into(),
        description: "Taken by Resident at Place, Region (128, 128, 25) - General".into(),
        permissions: Permissions {
            next_owner: 0x7fff_ffff,
            group: 0,
            everyone: 0x0000_8000,
        },
        ..request()
    };
    let receipt = upload(&reqwest::Client::new(), &base, &format!("{base}/fetch"), Uuid::from_u128(4), photo)
        .await
        .expect("upload");
    assert_eq!(receipt.item.inv_type, 15);
    assert_eq!(receipt.cost, 10);
    let requests = task.await.expect("server");
    let body = aurora_llsd::from_xml(&requests[0].1).expect("LLSD");
    assert_eq!(body["asset_type"].as_str(), "texture");
    assert_eq!(body["inventory_type"].as_str(), "snapshot");
    assert_eq!(body["next_owner_mask"].as_i32(), 0x7fff_ffff);
    assert_eq!(body["everyone_mask"].as_i32(), 0x0000_8000);
    assert_eq!(body["expected_upload_cost"].as_i32(), 10);
}
#[tokio::test]
async fn large_texture_upload_keeps_confirmed_fee_when_preliminary_price_is_lower() {
    for (kind, inventory_type) in [(Kind::Texture, 0), (Kind::Snapshot, 15)] {
        for (preliminary, paid) in [(Some(10), Some(50)), (Some(0), Some(0)), (Some(50), None), (None, Some(50))] {
            let asset = Uuid::from_u128(3);
            let (base, task) = server(|base| {
                let mut offer = llsd_map! {"state"=>"upload", "uploader"=>format!("{base}/binary")};
                if let Some(price) = preliminary {
                    offer.insert("upload_price", price);
                }
                let mut ack = llsd_map! {"state"=>"complete", "new_inventory_item"=>Uuid::from_u128(2), "new_asset"=>asset};
                if let Some(price) = paid {
                    ack.insert("upload_price", price);
                }
                vec![(200, offer), (200, ack), (200, item(0, inventory_type, asset))]
            })
            .await;
            let upload_request = Request {
                kind,
                expected_cost: 50,
                ..request()
            };
            let expected_data = upload_request.data.clone();
            let receipt = upload(
                &reqwest::Client::new(),
                &base,
                &format!("{base}/fetch"),
                Uuid::from_u128(4),
                upload_request,
            )
            .await
            .expect("the preliminary base fee must not block a confirmed 2K upload");
            assert_eq!(receipt.cost, paid.unwrap_or(50));
            assert_eq!(receipt.item.inv_type, inventory_type);
            let requests = task.await.expect("server");
            assert_eq!(requests.len(), 3, "one negotiation, one transfer, one inventory verification");
            let body = aurora_llsd::from_xml(&requests[0].1).expect("negotiation");
            assert_eq!(body["expected_upload_cost"].as_i32(), 50, "keep the user's confirmed fee");
            assert_eq!(requests[1].1, expected_data);
        }
    }
}

#[tokio::test]
async fn negative_preliminary_upload_fee_prevents_binary_transfer() {
    let (base, task) = server(|base| {
        vec![(
            200,
            llsd_map! {"state"=>"upload", "uploader"=>format!("{base}/binary"), "upload_price"=>-1},
        )]
    })
    .await;
    assert_eq!(
        upload(&reqwest::Client::new(), &base, &base, Uuid::nil(), request())
            .await
            .expect_err("invalid price"),
        Error::Response
    );
    assert_eq!(task.await.expect("server").len(), 1);
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
        name: "Mesh fixture".into(),
        description: "Test object".into(),
        query: std::sync::Arc::new(aurora_llsd::to_xml(&llsd_map! {"asset_type"=>"mesh"})),
        resources: std::sync::Arc::new(aurora_llsd::to_xml(&resources)),
        timeout_secs: 600,
    };
    let (base,task) = server(|base|vec![(200,llsd_map! {"mesh_upload_status"=>"valid"}),
        (200,llsd_map! {"state"=>"upload","uploader"=>format!("{base}/mesh"),"upload_price"=>17,"data"=>llsd_map! {"resource_cost"=>2.0}}),
        (200,llsd_map! {"state"=>"complete","new_inventory_item"=>Uuid::from_u128(2),"new_asset"=>asset, "new_next_owner_mask"=>123, "new_group_mask"=>32, "new_everyone_mask"=>16, "inventory_flags"=>256})]).await;
    let client = reqwest::Client::new();
    let offer = mesh::quote(&client, &base, &format!("{base}/permission"), &upload)
        .await
        .expect("quote");
    assert_eq!(offer.cost, 17);
    assert_eq!(offer.data["resource_cost"].as_f64(), 2.0);
    let receipt = mesh::upload(&client, offer.uploader, Uuid::from_u128(4), offer.cost, upload)
        .await
        .expect("mesh");
    assert_eq!(receipt.item.asset_type, 6);
    assert_eq!(receipt.item.inv_type, 6);
    assert_eq!(receipt.item.name, "Mesh fixture");
    assert_eq!(receipt.item.desc, "Test object");
    assert_eq!(receipt.item.owner, Uuid::from_u128(4));
    assert_eq!(receipt.item.next_owner_mask, 123);
    assert_eq!(receipt.item.group_mask, 32);
    assert_eq!(receipt.item.everyone_mask, 16);
    assert_eq!(receipt.item.flags, 256);
    assert_eq!(receipt.cost, 17);
    let requests = task.await.expect("server");
    assert_eq!(requests.len(), 3, "a completed mesh upload requires no inventory re-read");
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
        name: "Mesh fixture".into(),
        description: "Test object".into(),
        query: std::sync::Arc::new(vec![1]),
        resources: std::sync::Arc::new(vec![1]),
        timeout_secs: 600,
    };
    assert_eq!(
        mesh::quote(&reqwest::Client::new(), &base, &base, &upload).await.err(),
        Some(Error::MeshPermission)
    );
    assert_eq!(task.await.expect("server").len(), 1);
}

#[tokio::test]
async fn completed_mesh_uses_legacy_creation_defaults_and_never_fetches_or_reuploads() {
    let asset = Uuid::from_u128(3);
    let owner = Uuid::from_u128(4);
    let (base, task) = server(|_| {
        vec![(
            200,
            llsd_map! {"state"=>"complete", "new_inventory_item"=>Uuid::from_u128(2), "new_asset"=>asset},
        )]
    })
    .await;
    let receipt = crate::inventory::mesh_upload::upload(&reqwest::Client::new(), base, owner, 11, mesh_request())
        .await
        .expect("confirmed creation");
    assert_eq!(receipt.item.asset_id, asset);
    assert_eq!(receipt.item.owner, owner);
    assert_eq!(receipt.item.creator, owner);
    assert_eq!(receipt.item.parent, Uuid::from_u128(1));
    assert_eq!(receipt.item.base_mask, 0x7fff_ffff);
    assert_eq!(receipt.item.owner_mask, 0x7fff_ffff);
    assert_eq!(receipt.item.next_owner_mask, Permissions::default().next_owner);
    assert_eq!((receipt.item.group_mask, receipt.item.everyone_mask), (0, 0));
    assert_eq!(receipt.cost, 11);
    assert_eq!(task.await.expect("server").len(), 1);
}

#[tokio::test]
async fn incomplete_mesh_acknowledgements_stay_uncertain_without_reuploading() {
    for ack in [
        llsd_map! {"state"=>"complete", "new_inventory_item"=>Uuid::from_u128(2)},
        llsd_map! {"state"=>"upload", "new_inventory_item"=>Uuid::from_u128(2), "new_asset"=>Uuid::from_u128(3)},
    ] {
        let (base, task) = server(|_| vec![(200, ack)]).await;
        let error = crate::inventory::mesh_upload::upload(&reqwest::Client::new(), base, Uuid::from_u128(4), 11, mesh_request())
            .await
            .expect_err("unconfirmed creation");
        assert!(matches!(error, Error::Uncertain(_)));
        assert_eq!(task.await.expect("server").len(), 1);
    }
}
