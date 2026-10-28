//! Outfit category metadata, following AISAPI::UpdateCategory, favorite_send,
//! LLFloaterChangeItemThumbnail::setThumbnailId and LLInventoryModel::removeCategory
//! (Firestorm indra/newview, originally LGPL 2.1). Deletion moves to Trash
//! with MoveInventoryFolder, like LLViewerInventoryCategory::updateParentOnServer:
//! AIS refuses a `parent_id` in a category PATCH (HTTP 400).

use crate::inventory::operations::InventoryMove;
use crate::inventory::{FolderContents, InvFolder};
use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Rename(String),
    Favorite(bool),
    Thumbnail(Uuid),
    Trash(Uuid),
}

#[derive(Debug, Clone)]
pub struct Mutation {
    pub folder: Uuid,
    pub parent: Uuid,
    pub update: Update,
}

impl Update {
    /// The AIS PATCH body; `None` for Trash, which is a UDP move.
    fn body(&self) -> Option<Llsd> {
        Some(match self {
            Self::Rename(name) => llsd_map! { "name" => name.clone() },
            Self::Favorite(favorite) => llsd_map! { "favorite" => if *favorite {
                llsd_map! { "toggled" => true }
            } else { Llsd::default() } },
            // The thumbnail service requires a string, not an LLSD UUID.
            Self::Thumbnail(id) => llsd_map! { "thumbnail" => if !id.is_nil() {
                llsd_map! { "asset_id" => id.to_string() }
            } else { Llsd::default() } },
            Self::Trash(_) => return None,
        })
    }

    pub fn apply(&self, folder: &mut InvFolder) {
        match self {
            Self::Rename(name) => folder.name.clone_from(name),
            Self::Favorite(favorite) => {
                folder.favorite = *favorite;
                folder.favorite_known = true;
            }
            Self::Thumbnail(id) => folder.thumbnail = *id,
            Self::Trash(parent) => folder.parent = *parent,
        }
    }

    fn confirmed(&self, folder: &InvFolder) -> bool {
        match self {
            Self::Rename(name) => folder.name == *name,
            Self::Favorite(favorite) => folder.favorite == *favorite,
            Self::Thumbnail(id) => folder.thumbnail == *id,
            Self::Trash(parent) => folder.parent == *parent,
        }
    }
}

pub(crate) async fn mutate<F, Fut>(
    http: &reqwest::Client,
    cap: &str,
    fetch_cap: &str,
    owner: Uuid,
    change: Mutation,
    send_move: F,
) -> Result<Vec<FolderContents>, String>
where
    F: FnOnce(InventoryMove) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut parents = vec![change.parent];
    if let Update::Trash(parent) = change.update {
        if change.folder.is_nil() || parent.is_nil() {
            return Err(aurora_i18n::tr!("inventory-move-invalid-target").into());
        }
        send_move(InventoryMove {
            id: change.folder,
            parent,
            folder: true,
        })
        .await?;
        if parent != change.parent {
            parents.push(parent);
        }
    } else if let Some(body) = change.update.body() {
        let url = format!("{}/category/{}", cap.trim_end_matches('/'), change.folder);
        super::send(http.patch(url), &body).await?;
    }
    let read_error = || aurora_i18n::tr!("outfits-categories-could-not-read-the-outfit-back-refresh-before-trying").to_owned();
    // A reliable UDP move can reach the fetch service after the first read,
    // as in inventory::operations: retry reads, never the move.
    let retries = if matches!(change.update, Update::Trash(_)) { 5 } else { 0 };
    for attempt in 0..=retries {
        let mut contents = if matches!(change.update, Update::Favorite(_)) {
            crate::inventory::ais::fetch_folders(http, cap, owner, &parents)
                .await
                .map_err(|_| read_error())?
        } else {
            let response = crate::caps::post_llsd(http, fetch_cap, &crate::inventory::fetch_request_body(&parents, owner))
                .await
                .map_err(|_| read_error())?;
            crate::inventory::parse_fetch_response(&response)
        };
        let complete = parents
            .iter()
            .all(|id| contents.iter().any(|c| c.folder_id == *id && c.version >= 0));
        if complete
            && contents
                .iter()
                .flat_map(|c| &c.folders)
                .any(|f| f.id == change.folder && change.update.confirmed(f))
        {
            contents.sort_by_key(|c| parents.iter().position(|id| *id == c.folder_id).unwrap_or(usize::MAX));
            return Ok(contents);
        }
        if attempt < retries {
            tokio::time::sleep(std::time::Duration::from_millis(200 << attempt.min(3))).await;
        }
    }
    Err(aurora_i18n::tr!("outfits-categories-the-server-did-not-confirm-the-outfit-change-refresh").into())
}

#[cfg(test)]
mod tests {
    use super::super::tests::server;
    use super::*;

    fn reply(folder: Uuid, parent: Uuid, old_parent: Uuid, update: &Update) -> Llsd {
        let mut category = llsd_map! { "category_id" => folder, "parent_id" => parent, "type_default" => 47, "name" => "Tenue" };
        for (key, value) in update.body().as_ref().and_then(Llsd::as_map).into_iter().flatten() {
            category.insert(key.clone(), value.clone());
        }
        if matches!(update, Update::Favorite(_)) {
            let mut categories = Llsd::new_map();
            categories.insert(folder.to_string(), category);
            return llsd_map! { "category_id" => parent, "version" => 3,
            "_embedded" => llsd_map! { "categories" => categories, "items" => Llsd::new_map(), "links" => Llsd::new_map() } };
        }
        let mut folders = vec![llsd_map! { "folder_id" => parent, "version" => 3, "categories" => Llsd::Array(vec![category]) }];
        if old_parent != parent {
            folders.push(llsd_map! { "folder_id" => old_parent, "version" => 4, "categories" => Llsd::Array(vec![]) });
        }
        llsd_map! { "folders" => Llsd::Array(folders) }
    }

    async fn no_move(movement: InventoryMove) -> Result<(), String> {
        panic!("unexpected inventory move {movement:?}")
    }

    #[tokio::test]
    async fn category_updates_use_ais_patch_and_confirm_parent_metadata() {
        let folder = Uuid::from_u128(47);
        let parent = Uuid::from_u128(48);
        for update in [
            Update::Rename("Veste d’hiver".into()),
            Update::Favorite(true),
            Update::Favorite(false),
            Update::Thumbnail(Uuid::from_u128(9)),
            Update::Thumbnail(Uuid::nil()),
        ] {
            let (url, task) = server(vec![(200, Llsd::new_map()), (200, reply(folder, parent, parent, &update))]).await;
            let result = mutate(
                &reqwest::Client::new(),
                &url,
                &format!("{url}/fetch"),
                Uuid::nil(),
                Mutation {
                    folder,
                    parent,
                    update: update.clone(),
                },
                no_move,
            )
            .await
            .expect("confirmed category update");
            assert_eq!(result[0].folder_id, parent);
            let requests = task.await.expect("server");
            assert_eq!(requests[0].0, format!("PATCH /category/{folder} HTTP/1.1"));
            assert_eq!(Some(requests[0].1.clone()), update.body());
            if matches!(update, Update::Favorite(_)) {
                assert!(requests[1].0.starts_with(&format!("GET /category/{parent}/children?depth=0 ")));
            } else {
                assert!(requests[1].0.starts_with("POST /fetch "));
                assert_eq!(requests[1].1["folders"].as_array().len(), 1);
            }
        }
    }

    /// #1293: AIS answers 400 to a `parent_id` PATCH, so deleting an outfit
    /// sends MoveInventoryFolder and reads both folders back until the move
    /// shows, without a second move.
    #[tokio::test]
    async fn trash_moves_the_folder_over_udp_and_waits_for_the_read_back() {
        let folder = Uuid::from_u128(47);
        let parent = Uuid::from_u128(48);
        let trash = Uuid::from_u128(14);
        let update = Update::Trash(trash);
        // The first read still lists the outfit under its old parent.
        let category = llsd_map! { "category_id" => folder, "parent_id" => parent, "type_default" => 47, "name" => "Tenue" };
        let stale = llsd_map! { "folders" => Llsd::Array(vec![
            llsd_map! { "folder_id" => parent, "version" => 3, "categories" => Llsd::Array(vec![category]) },
            llsd_map! { "folder_id" => trash, "version" => 2, "categories" => Llsd::Array(vec![]) },
        ]) };
        let (url, task) = server(vec![(200, stale), (200, reply(folder, trash, parent, &update))]).await;
        let moves = std::sync::Mutex::new(Vec::new());
        let result = mutate(
            &reqwest::Client::new(),
            &url,
            &format!("{url}/fetch"),
            Uuid::nil(),
            Mutation { folder, parent, update },
            |movement| {
                moves.lock().expect("moves").push(movement);
                async { Ok(()) }
            },
        )
        .await
        .expect("confirmed trash");
        assert_eq!(
            moves.into_inner().expect("moves"),
            [InventoryMove {
                id: folder,
                parent: trash,
                folder: true
            }]
        );
        assert_eq!(result[0].folder_id, parent);
        assert_eq!(result[1].folder_id, trash);
        let requests = task.await.expect("server");
        assert!(requests.iter().all(|(line, _)| line.starts_with("POST /fetch ")), "no AIS PATCH");
        assert!(requests.iter().all(|(_, body)| body["folders"].as_array().len() == 2));
    }

    #[tokio::test]
    async fn refused_trash_move_stops_before_any_request() {
        let (url, task) = server(Vec::new()).await;
        let error = mutate(
            &reqwest::Client::new(),
            &url,
            &format!("{url}/fetch"),
            Uuid::nil(),
            Mutation {
                folder: Uuid::from_u128(47),
                parent: Uuid::from_u128(48),
                update: Update::Trash(Uuid::from_u128(14)),
            },
            |_| async { Err("offline".to_owned()) },
        )
        .await
        .expect_err("refused");
        assert_eq!(error, "offline");
        assert!(task.await.expect("server").is_empty());
    }

    #[tokio::test]
    async fn unconfirmed_category_metadata_is_not_reported_as_success() {
        let folder = Uuid::from_u128(47);
        let parent = Uuid::from_u128(48);
        let (url, task) = server(vec![
            (200, Llsd::new_map()),
            (200, reply(folder, parent, parent, &Update::Favorite(false))),
        ])
        .await;
        let result = mutate(
            &reqwest::Client::new(),
            &url,
            &format!("{url}/fetch"),
            Uuid::nil(),
            Mutation {
                folder,
                parent,
                update: Update::Favorite(true),
            },
            no_move,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(task.await.expect("server").len(), 2);
    }

    #[tokio::test]
    async fn refused_category_update_stops_before_refresh_and_hides_private_data() {
        let (url, task) = server(vec![(403, llsd_map! { "error" => "private server information" })]).await;
        let error = mutate(
            &reqwest::Client::new(),
            &url,
            &format!("{url}/fetch"),
            Uuid::nil(),
            Mutation {
                folder: Uuid::from_u128(47),
                parent: Uuid::from_u128(48),
                update: Update::Rename("Nom".into()),
            },
            no_move,
        )
        .await
        .expect_err("refused");
        assert!(error.contains("403"));
        assert!(!error.contains("private") && !error.contains(&url));
        assert_eq!(task.await.expect("server").len(), 1);
    }
}
