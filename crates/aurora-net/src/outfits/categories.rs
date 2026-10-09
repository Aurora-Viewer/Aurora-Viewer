//! Outfit category metadata, following AISAPI::UpdateCategory, favorite_send,
//! LLFloaterChangeItemThumbnail::setThumbnailId and LLInventoryModel::removeCategory
//! (Firestorm indra/newview, originally LGPL 2.1). Deletion moves to Trash.

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
    fn body(&self) -> Llsd {
        match self {
            Self::Rename(name) => llsd_map! { "name" => name.clone() },
            Self::Favorite(favorite) => llsd_map! { "favorite" => if *favorite {
                llsd_map! { "toggled" => true }
            } else { Llsd::default() } },
            // The thumbnail service requires a string, not an LLSD UUID.
            Self::Thumbnail(id) => llsd_map! { "thumbnail" => if !id.is_nil() {
                llsd_map! { "asset_id" => id.to_string() }
            } else { Llsd::default() } },
            Self::Trash(parent) => llsd_map! { "parent_id" => *parent },
        }
    }

    pub fn apply(&self, folder: &mut InvFolder) {
        match self {
            Self::Rename(name) => folder.name.clone_from(name),
            Self::Favorite(favorite) => folder.favorite = *favorite,
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

pub async fn mutate(
    http: &reqwest::Client,
    cap: &str,
    fetch_cap: &str,
    owner: Uuid,
    change: Mutation,
) -> Result<Vec<FolderContents>, String> {
    let url = format!("{}/category/{}", cap.trim_end_matches('/'), change.folder);
    super::send(http.patch(url), &change.update.body()).await?;
    let mut parents = vec![change.parent];
    if let Update::Trash(parent) = change.update
        && parent != change.parent
    {
        parents.push(parent);
    }
    let response = crate::caps::post_llsd(http, fetch_cap, &crate::inventory::fetch_request_body(&parents, owner))
        .await
        .map_err(|_| "La relecture de la tenue a échoué. Actualisez avant de réessayer.".to_owned())?;
    let mut contents = crate::inventory::parse_fetch_response(&response);
    if parents
        .iter()
        .any(|id| !contents.iter().any(|c| c.folder_id == *id && c.version >= 0))
        || !contents
            .iter()
            .flat_map(|c| &c.folders)
            .any(|f| f.id == change.folder && change.update.confirmed(f))
    {
        return Err("Le serveur n’a pas confirmé la modification de la tenue. Actualisez avant de réessayer.".into());
    }
    contents.sort_by_key(|c| parents.iter().position(|id| *id == c.folder_id).unwrap_or(usize::MAX));
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::super::tests::server;
    use super::*;

    fn reply(folder: Uuid, parent: Uuid, old_parent: Uuid, update: &Update) -> Llsd {
        let mut category = llsd_map! { "category_id" => folder, "parent_id" => parent, "type_default" => 47, "name" => "Tenue" };
        for (key, value) in update.body().as_map().into_iter().flatten() {
            category.insert(key.clone(), value.clone());
        }
        let mut folders = vec![llsd_map! { "folder_id" => parent, "version" => 3, "categories" => Llsd::Array(vec![category]) }];
        if old_parent != parent {
            folders.push(llsd_map! { "folder_id" => old_parent, "version" => 4, "categories" => Llsd::Array(vec![]) });
        }
        llsd_map! { "folders" => Llsd::Array(folders) }
    }

    #[tokio::test]
    async fn category_updates_use_ais_patch_and_confirm_parent_metadata() {
        let folder = Uuid::from_u128(47);
        let parent = Uuid::from_u128(48);
        let trash = Uuid::from_u128(14);
        for update in [
            Update::Rename("Veste d’hiver".into()),
            Update::Favorite(true),
            Update::Favorite(false),
            Update::Thumbnail(Uuid::from_u128(9)),
            Update::Thumbnail(Uuid::nil()),
            Update::Trash(trash),
        ] {
            let destination = if let Update::Trash(parent) = update { parent } else { parent };
            let (url, task) = server(vec![(200, Llsd::new_map()), (200, reply(folder, destination, parent, &update))]).await;
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
            )
            .await
            .expect("confirmed category update");
            assert_eq!(result[0].folder_id, parent);
            let requests = task.await.expect("server");
            assert_eq!(requests[0].0, format!("PATCH /category/{folder} HTTP/1.1"));
            assert_eq!(requests[0].1, update.body());
            assert!(requests[1].0.starts_with("POST /fetch "));
            assert_eq!(requests[1].1["folders"].as_array().len(), if destination == parent { 1 } else { 2 });
        }
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
        )
        .await
        .expect_err("refused");
        assert!(error.contains("403"));
        assert!(!error.contains("private") && !error.contains(&url));
        assert_eq!(task.await.expect("server").len(), 1);
    }
}
