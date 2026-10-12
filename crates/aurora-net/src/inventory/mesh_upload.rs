//! LLMeshUploadThread fee query and upload (Firestorm llmeshrepository.cpp).
//! The private one-use uploader stays in the session; UI receives only a local
//! token. Prepared LLSD is serialized by a worker, not by the frame thread.

use super::{
    upload::{Error, Receipt, Rejection, mesh_reply},
    *,
};
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone)]
pub struct Request {
    pub folder: Uuid,
    pub texture_folder: Uuid,
    pub query: Arc<Vec<u8>>,
    pub resources: Arc<Vec<u8>>,
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        if self.folder.is_nil()
            || self.texture_folder.is_nil()
            || self.query.is_empty()
            || self.resources.is_empty()
            || self.query.len() > 256 * 1024 * 1024
            || self.resources.len() > 256 * 1024 * 1024
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Quote {
    pub token: Uuid,
    pub cost: i32,
    pub data: Llsd,
}
pub(crate) struct Offer {
    pub uploader: String,
    pub cost: i32,
    pub data: Llsd,
}
fn metrics(value: &Llsd) -> Llsd {
    let number = |v: &Llsd| match v {
        Llsd::Integer(n) if *n >= 0 => Some(Llsd::Integer(*n)),
        Llsd::Real(n) if n.is_finite() && *n >= 0.0 => Some(Llsd::Real(*n)),
        _ => None,
    };
    let mut clean = Llsd::new_map();
    for key in [
        "resource_cost",
        "model_streaming_cost",
        "simulation_cost",
        "physics_cost",
        "streaming_cost",
    ] {
        if let Some(n) = number(&value[key]) {
            clean.insert(key, n);
        }
    }
    for (map, keys) in [
        (
            "upload_price_breakdown",
            &["mesh_streaming", "mesh_physics", "mesh_instance", "texture", "model"][..],
        ),
        ("model_physics_cost", &["hull", "mesh", "decomposition"][..]),
        ("streaming_params", &["high_lod", "medium_lod", "low_lod", "lowest_lod"][..]),
    ] {
        let mut numbers = Llsd::new_map();
        for key in keys {
            if let Some(n) = number(&value[map][*key]) {
                numbers.insert(*key, n);
            }
        }
        clean.insert(map, numbers);
    }
    clean
}

pub(crate) async fn quote(http: &reqwest::Client, cap: &str, permission: &str, request: &Request) -> Result<Offer, Error> {
    request.validate()?;
    let access = http
        .get(permission)
        .header("Accept", "application/llsd+xml")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    let access = mesh_reply(access).await?;
    if !matches!(access["mesh_upload_status"].as_str(), "" | "valid") {
        return Err(Error::MeshPermission);
    }
    let response = http
        .post(cap)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(request.query.as_ref().clone())
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    let response = mesh_reply(response).await?;
    if response["state"].as_str() != "upload" || response["uploader"].as_str().is_empty() {
        return Err(Error::Refused(Rejection::from_response(None, &response)));
    }
    let cost = match &response["upload_price"] {
        Llsd::Integer(n) if *n >= 0 => *n,
        _ => return Err(Error::Response),
    };
    Ok(Offer {
        uploader: response["uploader"].as_str().to_owned(),
        cost,
        data: metrics(&response["data"]),
    })
}

pub(crate) async fn upload(
    http: &reqwest::Client,
    uploader: String,
    fetch: &str,
    owner: Uuid,
    cost: i32,
    request: Request,
) -> Result<Receipt, Error> {
    request.validate()?;
    let response = http
        .post(uploader)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(request.resources.as_ref().clone())
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| Error::Unknown)?;
    let response = mesh_reply(response).await.map_err(|error| match error {
        Error::Refused(reason) => Error::Uncertain(reason),
        _ => Error::Unknown,
    })?;
    let id = response["new_inventory_item"].as_uuid();
    let asset = response["new_asset"].as_uuid();
    if response["state"].as_str() != "complete" || id.is_nil() || asset.is_nil() {
        return Err(Error::Uncertain(Rejection::from_response(None, &response)));
    }
    let reread = super::upload::reread(http, fetch, owner, id).await?;
    let item = parse_items_response(&reread)
        .into_iter()
        .find(|item| {
            item.id == id
                && item.asset_id == asset
                && item.asset_type == 6
                && item.inv_type == 6
                && item.parent == request.folder
                && item.owner == owner
        })
        .ok_or(Error::Verification)?;
    let paid = match &response["upload_price"] {
        Llsd::Integer(n) if *n >= 0 => *n,
        _ => cost,
    };
    Ok(Receipt { item, cost: paid })
}
