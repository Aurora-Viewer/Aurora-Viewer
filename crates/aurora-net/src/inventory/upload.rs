//! LLResourceUploadInfo::generatePostBody and upload coroutine
//! (Firestorm indra/newview/llviewerassetupload.cpp, originally LGPL 2.1).
//! Only bounded error fields enter diagnostics, with URLs and paths removed.

use aurora_llsd::{Llsd, llsd_map};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Texture,
    Sound,
    Animation,
    Material,
    Mesh,
    /// A photo of the world: a texture asset in the inventory type
    /// IT_SNAPSHOT (LLSnapshotLivePreview::saveTexture).
    Snapshot,
}
impl Kind {
    pub fn asset_type(self) -> i32 {
        match self {
            Self::Texture | Self::Snapshot => 0,
            Self::Sound => 1,
            Self::Animation => 20,
            Self::Material => 57,
            Self::Mesh => 6,
        }
    }
    pub fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::Texture => ("texture", "texture"),
            Self::Sound => ("sound", "sound"),
            Self::Animation => ("animatn", "animation"),
            Self::Material => ("material", "material"),
            Self::Mesh => ("mesh", "object"),
            Self::Snapshot => ("texture", "snapshot"),
        }
    }
    /// The upload folder chosen per type; a photo goes to the Photo Album
    /// instead (FT_SNAPSHOT_CATEGORY), its slot is the textures' one.
    pub fn folder_slot(self) -> usize {
        match self {
            Self::Texture | Self::Snapshot => 0,
            Self::Sound => 1,
            Self::Animation => 2,
            Self::Material => 4,
            Self::Mesh => 3,
        }
    }
}

/// Unknown costs stay unknown: opening the importer before login must never
/// promise a free upload. 2K pricing depends on area, not the longest side.
#[derive(Debug, Clone, Default)]
pub struct Prices {
    pub texture: Option<i32>,
    pub large_texture: Option<i32>,
    pub sound: Option<i32>,
    pub animation: Option<i32>,
}
impl Prices {
    pub fn with_fallback(mut self, cost: Option<i32>) -> Self {
        let cost = cost.filter(|n| *n >= 0);
        self.texture = self.texture.or(cost);
        self.large_texture = self.large_texture.or(cost);
        self.sound = self.sound.or(cost);
        self.animation = self.animation.or(cost);
        self
    }
    pub fn from_login(raw: &Llsd) -> Self {
        let benefits = &raw["account_level_benefits"];
        let price = |key| match &benefits[key] {
            Llsd::Integer(n) if *n >= 0 => Some(*n),
            _ => None,
        };
        let texture = price("texture_upload_cost");
        Self {
            texture,
            large_texture: benefits["large_texture_upload_cost"]
                .as_array()
                .iter()
                .filter_map(|v| match v {
                    Llsd::Integer(n) if *n >= 0 => Some(*n),
                    _ => None,
                })
                .min()
                .or(texture),
            sound: price("sound_upload_cost"),
            animation: price("animation_upload_cost"),
        }
    }
    pub fn cost(&self, kind: Kind, dimensions: (u32, u32)) -> Option<i32> {
        match kind {
            Kind::Texture | Kind::Snapshot if u64::from(dimensions.0) * u64::from(dimensions.1) > 1024 * 1024 => self.large_texture,
            Kind::Texture | Kind::Snapshot => self.texture,
            Kind::Sound => self.sound,
            Kind::Animation => self.animation,
            Kind::Material => Some(0),
            Kind::Mesh => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub next_owner: u32,
    pub group: u32,
    pub everyone: u32,
}
impl Default for Permissions {
    fn default() -> Self {
        Self {
            next_owner: crate::build::perm::MOVE | crate::build::perm::TRANSFER,
            group: 0,
            everyone: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub kind: Kind,
    pub folder: Uuid,
    pub name: String,
    pub description: String,
    pub permissions: Permissions,
    pub expected_cost: i32,
    pub data: Vec<u8>,
}
impl Request {
    pub fn body(&self) -> Result<Llsd, Error> {
        if self.folder.is_nil()
            || self.expected_cost < 0
            || self.name.trim().is_empty()
            || self.name.len() > 63
            || self.description.len() > 127
            || self.name.contains('\0')
            || self.description.contains('\0')
            || self.data.is_empty()
            || self.data.len() > 64 * 1024 * 1024
        {
            return Err(Error::Invalid);
        }
        let (asset, inventory) = self.kind.names();
        Ok(llsd_map! {
            "folder_id" => self.folder, "asset_type" => asset, "inventory_type" => inventory,
            "name" => self.name.as_str(), "description" => self.description.as_str(),
            "next_owner_mask" => self.permissions.next_owner as i32,
            "group_mask" => self.permissions.group as i32,
            "everyone_mask" => self.permissions.everyone as i32,
            "expected_upload_cost" => self.expected_cost,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Receipt {
    pub item: super::InvItem,
    pub cost: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub status: Option<u16>,
    pub identifier: String,
    pub messages: Vec<String>,
}

impl Rejection {
    // LLMeshUploadThread::log_upload_error reads error.message, identifier
    // and the per-model errors array. Never propagate the full LLSD response.
    pub(super) fn from_response(status: Option<u16>, value: &Llsd) -> Self {
        let clean = |text: &str| {
            text.split_whitespace()
                .map(|word| {
                    let token = word.trim_start_matches(['\'', '"', '(', '[']);
                    if token.contains(":/") || token.contains(":\\") || token.starts_with("\\\\") || token.starts_with('/') {
                        "…"
                    } else {
                        word
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect::<String>()
        };
        let error = &value["error"];
        let mut messages = Vec::new();
        for message in [error["message"].as_str(), error.as_str(), value["message"].as_str()] {
            let message = clean(message);
            if !message.is_empty() && !messages.contains(&message) {
                messages.push(message);
            }
        }
        if let Llsd::Array(errors) = &error["errors"] {
            for detail in errors.iter().take(8) {
                let message = clean(detail["message"].as_str());
                if !message.is_empty() && !messages.contains(&message) {
                    messages.push(message);
                }
            }
        }
        let identifier = error["identifier"].as_str();
        let identifier = if identifier.len() <= 96 && identifier.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
            identifier.to_owned()
        } else {
            String::new()
        };
        Self {
            status,
            identifier,
            messages,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid upload request")]
    Invalid,
    #[error("upload service unavailable")]
    Unavailable,
    #[error("upload refused")]
    Rejected,
    #[error("mesh upload permission denied")]
    MeshPermission,
    #[error("upload refused: {0:?}")]
    Refused(Rejection),
    #[error("upload response invalid")]
    Response,
    #[error("upload outcome unknown; check inventory before retrying")]
    Unknown,
    #[error("upload outcome unknown: {0:?}; check inventory before retrying")]
    Uncertain(Rejection),
    #[error("uploaded, but inventory could not be verified; reload inventory")]
    Verification,
    #[error("price changed; request a new confirmation")]
    PriceChanged(Option<i32>),
}

pub(super) async fn reply(mut response: reqwest::Response) -> Result<Llsd, Error> {
    if !response.status().is_success() {
        return Err(Error::Rejected);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unknown)? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(Error::Response);
        }
        bytes.extend_from_slice(&chunk);
    }
    aurora_llsd::from_xml(&bytes).map_err(|_| Error::Response)
}

pub(super) async fn mesh_reply(mut response: reqwest::Response) -> Result<Llsd, Error> {
    let status = (!response.status().is_success()).then_some(response.status().as_u16());
    let fallback = || Error::Refused(Rejection::from_response(status, &Llsd::Undef));
    let mut bytes = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| if status.is_some() { fallback() } else { Error::Unknown })?;
        let Some(chunk) = chunk else { break };
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(if status.is_some() { fallback() } else { Error::Response });
        }
        bytes.extend_from_slice(&chunk);
    }
    let parsed = aurora_llsd::from_xml(&bytes);
    if status.is_some() {
        return Err(Error::Refused(Rejection::from_response(status, &parsed.unwrap_or(Llsd::Undef))));
    }
    parsed.map_err(|_| Error::Response)
}

pub(super) async fn reread(http: &reqwest::Client, fetch: &str, owner: Uuid, id: Uuid) -> Result<Llsd, Error> {
    let response = http
        .post(fetch)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(&super::fetch_items_body(&[id], owner)))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| Error::Verification)?;
    reply(response).await.map_err(|_| Error::Verification)
}

pub async fn upload(http: &reqwest::Client, cap: &str, fetch: &str, owner: Uuid, request: Request) -> Result<Receipt, Error> {
    if matches!(request.kind, Kind::Material | Kind::Mesh) {
        return Err(Error::Invalid);
    }
    let body = request.body()?;
    let negotiation = http
        .post(cap)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(&body))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    let offer = reply(negotiation).await?;
    if offer["state"].as_str() != "upload" || offer["uploader"].as_str().is_empty() {
        return Err(Error::Rejected);
    }
    // LLViewerAssetUpload::AssetInventoryUploadCoproc (indra/newview/llviewerassetupload.cpp)
    // reads the paid price after the binary upload. The preliminary response can
    // still carry the base texture fee before the server knows the image size.
    // Keep the confirmed expected_upload_cost; only an increase needs reconfirmation.
    if let Llsd::Integer(n) = &offer["upload_price"]
        && (*n < 0 || *n > request.expected_cost)
    {
        return Err(if *n >= 0 { Error::PriceChanged(Some(*n)) } else { Error::Response });
    }
    let response = http
        .post(offer["uploader"].as_str())
        .header("Content-Type", "application/octet-stream")
        .body(request.data)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| Error::Unknown)?;
    let ack = reply(response).await.map_err(|_| Error::Unknown)?;
    let item_id = ack["new_inventory_item"].as_uuid();
    let asset = ack["new_asset"].as_uuid();
    if ack["state"].as_str() != "complete" || item_id.is_nil() || asset.is_nil() {
        return Err(Error::Unknown);
    }
    // The re-read preserves the permissions actually granted by the server.
    let reread = reread(http, fetch, owner, item_id).await?;
    let item = super::parse_items_response(&reread)
        .into_iter()
        .find(|item| {
            item.id == item_id
                && item.asset_id == asset
                && item.asset_type == request.kind.asset_type()
                && item.parent == request.folder
                && item.owner == owner
        })
        .ok_or(Error::Verification)?;
    let cost = match &ack["upload_price"] {
        Llsd::Integer(n) if *n >= 0 => *n,
        _ => request.expected_cost,
    };
    Ok(Receipt { item, cost })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_prices_keep_unknown_values_and_use_area_threshold() {
        let raw = llsd_map! { "account_level_benefits" => llsd_map! {
            "texture_upload_cost" => 10, "sound_upload_cost" => 0, "animation_upload_cost" => -1,
            "large_texture_upload_cost" => Llsd::Array(vec![50.into(), 40.into()])
        }};
        let prices = Prices::from_login(&raw);
        assert_eq!(prices.cost(Kind::Texture, (2048, 512)), Some(10));
        assert_eq!(prices.cost(Kind::Texture, (2048, 1024)), Some(40));
        assert_eq!(prices.cost(Kind::Sound, (0, 0)), Some(0));
        assert_eq!(prices.cost(Kind::Animation, (0, 0)), None);
        assert_eq!(Prices::default().cost(Kind::Texture, (1024, 1024)), None);
        // a photo costs what a texture of its size does
        assert_eq!(prices.cost(Kind::Snapshot, (1024, 1024)), Some(10));
        assert_eq!(prices.cost(Kind::Snapshot, (2048, 1024)), Some(40));
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod http_tests;
