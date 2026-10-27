//! LLMeshUploadThread fee query and upload (Legacy Viewer llmeshrepository.cpp).
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
    pub name: String,
    pub description: String,
    pub query: Arc<Vec<u8>>,
    pub resources: Arc<Vec<u8>>,
    /// MeshUploadTimeOut, frozen with the mesh and quote; never sent on wire.
    pub timeout_secs: u32,
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        if self.folder.is_nil()
            || self.texture_folder.is_nil()
            || self.name.trim().is_empty()
            || self.name.len() > 63
            || self.description.len() > 127
            || self.name.contains('\0')
            || self.description.contains('\0')
            || self.query.is_empty()
            || self.resources.is_empty()
            || self.timeout_secs == 0
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
        .timeout(Duration::from_secs(u64::from(request.timeout_secs)))
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

pub(crate) async fn upload(http: &reqwest::Client, uploader: String, owner: Uuid, cost: i32, request: Request) -> Result<Receipt, Error> {
    request.validate()?;
    let response = http
        .post(uploader)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(request.resources.as_ref().clone())
        .timeout(Duration::from_secs(u64::from(request.timeout_secs)))
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
    // on_new_single_inventory_upload_complete (llmeshrepository.cpp): the
    // completed upload is authoritative. FetchInventory2 may still be stale
    // or unavailable; it must not turn an already paid upload into a failure.
    // Creation permissions come from the server, never from the requested masks.
    let granted = response.has("new_next_owner_mask");
    let item = InvItem {
        id,
        parent: request.folder,
        name: request.name,
        desc: request.description,
        asset_id: asset,
        asset_type: 6,
        inv_type: 6,
        owner,
        creator: owner,
        base_mask: 0x7fff_ffff,
        owner_mask: 0x7fff_ffff,
        next_owner_mask: if granted {
            response["new_next_owner_mask"].as_u32()
        } else {
            super::upload::Permissions::default().next_owner
        },
        group_mask: if granted { response["new_group_mask"].as_u32() } else { 0 },
        everyone_mask: if granted { response["new_everyone_mask"].as_u32() } else { 0 },
        flags: response["inventory_flags"].as_u32(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_secs() as i64)
            .unwrap_or_default(),
        ..Default::default()
    };
    let paid = match &response["upload_price"] {
        Llsd::Integer(n) if *n >= 0 => *n,
        _ => cost,
    };
    Ok(Receipt { item, cost: paid })
}
