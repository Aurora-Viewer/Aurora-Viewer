//! Port of AISAPI::CreateInventory / SlamFolder (indra/newview/llaisapi.cpp,
//! originally LGPL 2.1). Outfits contain links, never copies of their assets.

use crate::inventory::operations::UdpEdit;
use crate::inventory::{FolderContents, InvFolder};
use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;
pub mod categories;

/// LLFolderType::FT_OUTFIT.
const OUTFIT_FOLDER: i8 = 47;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutfitLink {
    pub target: Uuid,
    pub name: String,
    pub desc: String,
    pub folder: bool,
    pub inv_type: i32,
}

#[derive(Debug, Clone)]
pub struct OutfitMutation {
    pub folder: Uuid,
    /// A new FT_OUTFIT category, created under My Outfits.
    pub create: Option<InvFolder>,
    pub links: Vec<OutfitLink>,
    /// Save As also changes the base-outfit link in the COF.
    pub cof: Option<(Uuid, Vec<OutfitLink>)>,
}

pub fn links_body(links: &[OutfitLink]) -> Llsd {
    Llsd::Array(
        links
            .iter()
            .map(|l| {
                llsd_map! {
                    "linked_id" => l.target,
                    "name" => l.name.clone(),
                    "desc" => l.desc.clone(),
                    "type" => if l.folder { 25 } else { 24 },
                    "inv_type" => l.inv_type,
                }
            })
            .collect(),
    )
}

/// Errors expose only operation/status, never a capability URL or response body.
pub async fn mutate(
    http: &reqwest::Client,
    cap: &str,
    fetch_cap: &str,
    owner: Uuid,
    mut change: OutfitMutation,
) -> Result<Vec<FolderContents>, String> {
    let cap = cap.trim_end_matches('/');
    let mut refresh = vec![change.folder];
    if let Some(cat) = &mut change.create {
        let body = llsd_map! { "categories" => Llsd::Array(vec![llsd_map! {
            "name" => cat.name.clone(), "type_default" => 47, "parent_id" => cat.parent, "category_id" => Uuid::nil(),
        }]) };
        let url = format!("{cap}/category/{}?tid={}", cat.parent, Uuid::new_v4());
        let response = request(http, &url, &body, false).await?;
        let created = response["_created_categories"].at(0).as_uuid();
        if created.is_nil() {
            return Err(aurora_i18n::tr!("outfits-the-server-did-not-confirm-outfit-creation-refresh-inventory").into());
        }
        // The server assigns the category UUID (asAISCreateCatLLSD).
        let provisional = change.folder;
        change.folder = created;
        refresh = vec![cat.parent, created];
        if let Some((_, links)) = &mut change.cof {
            for l in links.iter_mut().filter(|l| l.folder && l.target == provisional) {
                l.target = created;
            }
        }
    }
    let url = format!("{cap}/category/{}/links?tid={}", change.folder, Uuid::new_v4());
    request(http, &url, &links_body(&change.links), true).await?;
    if let Some((cof, links)) = &change.cof {
        let url = format!("{cap}/category/{cof}/links?tid={}", Uuid::new_v4());
        request(http, &url, &links_body(links), true).await?;
        refresh.push(*cof);
    }
    let body = crate::inventory::fetch_request_body(&refresh, owner);
    let response = crate::caps::post_llsd(http, fetch_cap, &body)
        .await
        .map_err(|_| aurora_i18n::tr!("outfits-the-outfit-was-sent-but-could-not-be-read").to_owned())?;
    let mut contents = crate::inventory::parse_fetch_response(&response);
    contents.sort_by_key(|c| refresh.iter().position(|id| *id == c.folder_id).unwrap_or(usize::MAX));
    if refresh
        .iter()
        .any(|id| !contents.iter().any(|c| c.folder_id == *id && c.version >= 0))
    {
        return Err(aurora_i18n::tr!("outfits-the-outfit-is-incomplete-refresh-before-trying-again").into());
    }
    Ok(contents)
}

/// `mutate` on a grid without AIS, which has no call that replaces a folder's
/// links: as slam_inventory_folder falls back to (llviewerinventory.cpp),
/// remove_folder_contents, then link_inventory_array. Links only: the folder's
/// other content stays.
pub(crate) async fn mutate_udp<E, EFut>(
    http: &reqwest::Client,
    fetch_cap: &str,
    owner: Uuid,
    change: OutfitMutation,
    mut send_edit: E,
) -> Result<Vec<FolderContents>, String>
where
    E: FnMut(UdpEdit) -> EFut,
    EFut: std::future::Future<Output = Result<(), String>>,
{
    let fetch = |folders: Vec<Uuid>| async move {
        let body = crate::inventory::fetch_request_body(&folders, owner);
        crate::caps::post_llsd(http, fetch_cap, &body)
            .await
            .map(|doc| crate::inventory::parse_fetch_response(&doc))
            .map_err(|_| aurora_i18n::tr!("outfits-the-outfit-was-sent-but-could-not-be-read").to_owned())
    };
    let mut refresh = vec![change.folder];
    if let Some(cat) = &change.create {
        // the simulator keeps the id a viewer gives a new folder
        send_edit(UdpEdit::CreateFolder {
            id: change.folder,
            parent: cat.parent,
            name: cat.name.clone(),
            type_default: OUTFIT_FOLDER,
        })
        .await?;
        refresh = vec![cat.parent, change.folder];
    }
    let mut slams = vec![(change.folder, &change.links)];
    if let Some((cof, links)) = &change.cof {
        slams.push((*cof, links));
        refresh.push(*cof);
    }
    for (folder, links) in &slams {
        if change.create.is_none() || *folder != change.folder {
            for content in fetch(vec![*folder]).await? {
                for item in content.items.iter().filter(|item| is_link(item.asset_type)) {
                    send_edit(UdpEdit::RemoveItem(item.id)).await?;
                }
            }
        }
        for link in links.iter() {
            send_edit(UdpEdit::Link {
                parent: *folder,
                target: link.target,
                name: link.name.clone(),
                desc: link.desc.clone(),
                folder: link.folder,
                inv_type: link.inv_type as i8,
            })
            .await?;
        }
    }
    // no answer comes on the circuit: a read shows when the links landed
    const RETRIES: u32 = 5;
    for attempt in 0..=RETRIES {
        let mut contents = fetch(refresh.clone()).await?;
        let linked = slams.iter().all(|(folder, links)| {
            links.iter().all(|link| {
                contents
                    .iter()
                    .filter(|c| c.folder_id == *folder)
                    .flat_map(|c| &c.items)
                    .any(|item| item.asset_id == link.target && is_link(item.asset_type))
            })
        });
        let complete = refresh
            .iter()
            .all(|id| contents.iter().any(|c| c.folder_id == *id && c.version >= 0));
        if linked && complete {
            contents.sort_by_key(|c| refresh.iter().position(|id| *id == c.folder_id).unwrap_or(usize::MAX));
            return Ok(contents);
        }
        if attempt < RETRIES {
            tokio::time::sleep(std::time::Duration::from_millis(200 << attempt.min(3))).await;
        }
    }
    Err(aurora_i18n::tr!("outfits-the-outfit-is-incomplete-refresh-before-trying-again").into())
}

/// LLAssetType::AT_LINK and AT_LINK_FOLDER.
fn is_link(asset_type: i32) -> bool {
    matches!(asset_type, 24 | 25)
}

async fn request(http: &reqwest::Client, url: &str, body: &Llsd, put: bool) -> Result<Llsd, String> {
    let req = if put { http.put(url) } else { http.post(url) };
    send(req, body).await
}

/// AISAPI::UpdateItem and favorite_send (llinventoryfunctions.cpp).
/// The favorite belongs to the original item, and is stored by the server.
pub async fn set_favorite(
    http: &reqwest::Client,
    cap: &str,
    _fetch_cap: &str,
    owner: Uuid,
    item: Uuid,
    favorite: bool,
) -> Result<crate::inventory::InvItem, String> {
    let value = if favorite {
        llsd_map! { "toggled" => true }
    } else {
        Llsd::default()
    };
    let body = llsd_map! { "favorite" => value };
    let url = format!("{}/item/{item}", cap.trim_end_matches('/'));
    send(http.patch(url), &body).await?;
    crate::inventory::ais::fetch_items(http, cap, owner, &[item])
        .await
        .map_err(|_| aurora_i18n::tr!("outfits-could-not-read-the-pick-back-refresh-before-trying").to_owned())?
        .into_iter()
        .find(|it| it.id == item && it.favorite == favorite)
        .ok_or_else(|| aurora_i18n::tr!("outfits-the-server-did-not-confirm-the-pick-change").into())
}

pub(crate) async fn send(req: reqwest::RequestBuilder, body: &Llsd) -> Result<Llsd, String> {
    let response = req
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(body))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| aurora_i18n::tr!("outfits-the-inventory-server-is-not-responding-refresh-before-trying").to_owned())?;
    if !response.status().is_success() {
        return Err(aurora_i18n::tr!(
            "outfits-the-server-rejected-the-operation-http",
            value0 = format!("{}", response.status().as_u16())
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| aurora_i18n::tr!("outfits-interrupted-inventory-response").to_owned())?;
    aurora_llsd::from_xml(&bytes).map_err(|_| aurora_i18n::tr!("outfits-unreadable-inventory-response").into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) async fn server(replies: Vec<(u16, Llsd)>) -> (String, tokio::task::JoinHandle<Vec<(String, Llsd)>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local test server");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, reply) in replies {
                let (mut socket, _) = listener.accept().await.expect("request");
                let mut bytes = Vec::new();
                let (start, len) = loop {
                    let mut buffer = [0u8; 4096];
                    let n = socket.read(&mut buffer).await.expect("HTTP read");
                    assert!(n > 0, "request ended before its body");
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(pos) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..pos]);
                        let len = header
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= pos + 4 + len {
                            break (pos + 4, len);
                        }
                    }
                };
                let first = String::from_utf8_lossy(&bytes[..start])
                    .lines()
                    .next()
                    .expect("request line")
                    .to_owned();
                requests.push((
                    first,
                    if len == 0 {
                        Llsd::Undef
                    } else {
                        aurora_llsd::from_xml(&bytes[start..start + len]).expect("LLSD request")
                    },
                ));
                let body = aurora_llsd::to_xml(&reply);
                let header = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.expect("HTTP header");
                socket.write_all(&body).await.expect("HTTP body");
                crate::test_http::finish(socket);
            }
            requests
        });
        (url, task)
    }

    #[tokio::test]
    async fn save_as_uses_server_category_id_and_refetches_before_success() {
        let parent = Uuid::from_u128(48);
        let cof = Uuid::from_u128(46);
        let assigned = Uuid::from_u128(200);
        let provisional = Uuid::from_u128(100);
        let contents = llsd_map! {"folders"=>Llsd::Array(vec![
            llsd_map! {"folder_id"=>assigned,"version"=>2},llsd_map! {"folder_id"=>cof,"version"=>3},
            llsd_map! {"folder_id"=>parent,"version"=>4,"categories"=>Llsd::Array(vec![llsd_map! {"category_id"=>assigned,"parent_id"=>parent,"name"=>"Tenue","type_default"=>47}])},
        ])};
        let (url, task) = server(vec![
            (200, llsd_map! {"_created_categories"=>Llsd::Array(vec![assigned.into()])}),
            (200, Llsd::new_map()),
            (200, Llsd::new_map()),
            (200, contents),
        ])
        .await;
        let link = OutfitLink {
            target: Uuid::from_u128(1),
            name: "Veste".into(),
            desc: "@800".into(),
            folder: false,
            inv_type: 18,
        };
        let change = OutfitMutation {
            folder: provisional,
            create: Some(InvFolder {
                id: provisional,
                parent,
                name: "Tenue".into(),
                type_default: 47,
                version: 1,
                ..Default::default()
            }),
            links: vec![link.clone()],
            cof: Some((
                cof,
                vec![
                    link,
                    OutfitLink {
                        target: provisional,
                        name: "Tenue".into(),
                        desc: String::new(),
                        folder: true,
                        inv_type: 8,
                    },
                ],
            )),
        };
        let result = mutate(&reqwest::Client::new(), &url, &format!("{url}/fetch"), Uuid::from_u128(5), change)
            .await
            .expect("saved outfit");
        assert_eq!(result[0].folder_id, parent, "parent must be applied before its new child");
        let requests = task.await.expect("server");
        assert!(requests[0].0.starts_with(&format!("POST /category/{parent}?tid=")));
        assert!(requests[1].0.starts_with(&format!("PUT /category/{assigned}/links?tid=")));
        assert!(matches!(requests[1].1, Llsd::Array(_)), "AIS SlamFolder accepts a plain array");
        assert_eq!(requests[2].1.at(1)["linked_id"].as_uuid(), assigned);
        assert!(requests[3].0.starts_with("POST /fetch "));
    }

    #[tokio::test]
    async fn refusal_does_not_report_a_saved_outfit_or_expose_response_text() {
        let (url, task) = server(vec![(403, llsd_map! {"error"=>"private server information"})]).await;
        let change = OutfitMutation {
            folder: Uuid::from_u128(46),
            create: None,
            links: Vec::new(),
            cof: None,
        };
        let error = mutate(&reqwest::Client::new(), &url, &format!("{url}/fetch"), Uuid::nil(), change)
            .await
            .expect_err("refused");
        assert!(error.contains("403"));
        assert!(!error.contains("private"));
        assert!(!error.contains(&url));
        assert_eq!(task.await.expect("server").len(), 1);
    }

    #[test]
    fn ais_links_keep_targets_and_layer_order() {
        let links = vec![
            OutfitLink {
                target: Uuid::from_u128(42),
                name: "Tatouage".into(),
                desc: "@1201".into(),
                folder: false,
                inv_type: 18,
            },
            OutfitLink {
                target: Uuid::from_u128(43),
                name: "Tenue".into(),
                desc: String::new(),
                folder: true,
                inv_type: 8,
            },
        ];
        let body = links_body(&links);
        assert_eq!(body.at(0)["linked_id"].as_uuid(), links[0].target);
        assert_eq!(body.at(0)["desc"].as_str(), "@1201");
        assert_eq!(body.at(0)["type"].as_i32(), 24);
        assert_eq!(body.at(1)["type"].as_i32(), 25);
    }

    #[tokio::test]
    async fn favorites_patch_the_original_and_are_confirmed_by_refetch() {
        let id = Uuid::from_u128(42);
        for favorite in [true, false] {
            let value = if favorite {
                llsd_map! { "toggled" => true }
            } else {
                Llsd::Undef
            };
            let (url, task) = server(vec![
                (200, Llsd::new_map()),
                (
                    200,
                    llsd_map! { "item_id" => id, "parent_id" => Uuid::from_u128(1), "name" => "Object", "type" => 6, "inv_type" => 6, "favorite" => value.clone() },
                ),
            ])
            .await;
            let it = set_favorite(&reqwest::Client::new(), &url, &format!("{url}/fetch"), Uuid::nil(), id, favorite)
                .await
                .expect("favorite");
            assert_eq!(it.favorite, favorite);
            let requests = task.await.expect("server");
            assert!(requests[0].0.starts_with(&format!("PATCH /item/{id} ")));
            assert_eq!(requests[0].1["favorite"], value);
            assert!(requests[1].0.starts_with(&format!("GET /item/{id} ")));
        }
    }
}
