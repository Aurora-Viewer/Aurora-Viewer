//! Our own bakes, on a grid that does not bake avatars (OpenSim): the
//! viewer composes them, uploads them and sends the appearance itself, as
//! the Legacy Viewer does when a region has no central baking
//! (LLAgent::sendAgentSetAppearance, llagent.cpp; LLVOAvatarSelf and
//! LLViewerTexLayerSetBuffer::doUpload, llvoavatarself.cpp and
//! llviewertexlayer.cpp; LLWearableData::computeBakedTextureHash,
//! llwearabledata.cpp; all originally LGPL 2.1). OpenSim takes them in
//! UploadBakedTextureModule.cs and AvatarFactoryModule.cs.

use aurora_llsd::{Llsd, llsd_map};
use glam::Vec3;
use uuid::{Uuid, uuid};

/// One bake: its texture-entry face, the id mixed into its cache key, and
/// the wearable types (`Wearable::kind`) it is made of, in hashing order
/// (LLAvatarAppearanceDefines, llavatarappearancedefines.cpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bake {
    pub te: u8,
    pub hash_id: Uuid,
    pub wearables: &'static [u8],
}

// Wearable types (LLWearableType::EType).
const SHAPE: u8 = 0;
const SKIN: u8 = 1;
const HAIR: u8 = 2;
const EYES: u8 = 3;
const SHIRT: u8 = 4;
const PANTS: u8 = 5;
const SHOES: u8 = 6;
const SOCKS: u8 = 7;
const JACKET: u8 = 8;
const GLOVES: u8 = 9;
const UNDERSHIRT: u8 = 10;
const UNDERPANTS: u8 = 11;
const SKIRT: u8 = 12;
const ALPHA: u8 = 13;
const TATTOO: u8 = 14;
pub const UNIVERSAL: u8 = 16;

/// Head, upper, lower, eyes, skirt and hair; then the five of bakes on mesh,
/// which a region without them leaves out.
pub const BAKES: [Bake; 11] = [
    Bake {
        te: 8,
        hash_id: uuid!("a4b9dc38-e13b-4df9-b284-751efb0566ff"),
        wearables: &[SHAPE, SKIN, HAIR, TATTOO, ALPHA, UNIVERSAL],
    },
    Bake {
        te: 9,
        hash_id: uuid!("5943ff64-d26c-4a90-a8c0-d61f56bd98d4"),
        wearables: &[SHAPE, SKIN, SHIRT, JACKET, GLOVES, UNDERSHIRT, TATTOO, ALPHA, UNIVERSAL],
    },
    Bake {
        te: 10,
        hash_id: uuid!("2944ee70-90a7-425d-a5fb-d749c782ed7d"),
        wearables: &[SHAPE, SKIN, PANTS, SHOES, SOCKS, JACKET, UNDERPANTS, TATTOO, ALPHA, UNIVERSAL],
    },
    Bake {
        te: 11,
        hash_id: uuid!("27b1bc0f-979f-4b13-95fe-b981c2ba9788"),
        wearables: &[EYES, UNIVERSAL, ALPHA],
    },
    Bake {
        te: 19,
        hash_id: uuid!("03e7e8cb-1368-483b-b6f3-74850838ba63"),
        wearables: &[SKIRT, UNIVERSAL],
    },
    Bake {
        te: 20,
        hash_id: uuid!("a60e85a9-74e8-48d8-8a2d-8129f28d9b61"),
        wearables: &[HAIR, UNIVERSAL, ALPHA],
    },
    Bake {
        te: 40,
        hash_id: uuid!("9f39febf-22d7-0087-79d1-e9e8c6c9ed19"),
        wearables: &[UNIVERSAL],
    },
    Bake {
        te: 41,
        hash_id: uuid!("054a7a58-8ed5-6386-0add-3b636fb28b78"),
        wearables: &[UNIVERSAL],
    },
    Bake {
        te: 42,
        hash_id: uuid!("790c11be-b25c-c17e-b4d2-6a4ad786b752"),
        wearables: &[UNIVERSAL],
    },
    Bake {
        te: 43,
        hash_id: uuid!("d78c478f-48c7-5928-5864-8d99fb1f521e"),
        wearables: &[UNIVERSAL],
    },
    Bake {
        te: 44,
        hash_id: uuid!("6a95dd53-edd9-aac8-f6d3-27ed99f3c3eb"),
        wearables: &[UNIVERSAL],
    },
];

/// Faces of an avatar's texture entry (TEX_NUM_INDICES).
pub const AVATAR_FACES: usize = 45;

/// The cache key of a bake for what is worn (computeBakedTextureHash): the
/// MD5 of each worn wearable's asset id, its types in the bake's order and
/// their layers in order, then the bake's own id. `None` when nothing it is
/// made of is worn: no key is sent for it.
pub fn cache_id(bake: &Bake, worn: impl Fn(u8) -> Vec<Uuid>) -> Option<Uuid> {
    let mut hash = md5::Context::new();
    let mut any = false;
    for kind in bake.wearables {
        for asset in worn(*kind) {
            hash.consume(asset.as_bytes());
            any = true;
        }
    }
    if !any {
        return None;
    }
    hash.consume(bake.hash_id.as_bytes());
    Some(Uuid::from_bytes(hash.finalize().0))
}

/// What `AgentSetAppearance` carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    /// The avatar's bounding size (LLAvatarAppearance::computeBodySize).
    pub size: Vec3,
    /// (cache key, texture-entry face) of each bake sent.
    pub cache: Vec<(Uuid, u8)>,
    /// The packed texture entry; empty while the bakes are not all there,
    /// which updates the visual parameters alone.
    pub texture_entry: Vec<u8>,
    /// Visual parameters of groups 0 and 3, by id, as bytes.
    pub params: Vec<u8>,
}

/// The message, `serial` being the count of those sent this session.
pub fn agent_set_appearance(agent: Uuid, session: Uuid, serial: u32, a: &Appearance) -> aurora_msg::msgs::AgentSetAppearance {
    use aurora_msg::msgs::agent_set_appearance as m;
    let mut msg = aurora_msg::msgs::AgentSetAppearance::default();
    msg.agent_data.agent_id = agent;
    msg.agent_data.session_id = session;
    msg.agent_data.serial_num = serial;
    msg.agent_data.size = a.size;
    msg.wearable_data = a
        .cache
        .iter()
        .map(|(id, te)| m::WearableData {
            cache_id: *id,
            texture_index: *te,
        })
        .collect();
    msg.object_data.texture_entry = a.texture_entry.clone();
    msg.visual_param = a.params.iter().map(|v| m::VisualParam { param_value: *v }).collect();
    msg
}

/// Ask the region for the bakes it keeps (LLAgentWearables::queryWearableCache):
/// each bake's cache key and face. The answer, AgentCachedTextureResponse,
/// names the texture of each key it knows and a nil id for the others.
pub fn agent_cached_texture(agent: Uuid, session: Uuid, serial: i32, keys: &[(Uuid, u8)]) -> aurora_msg::msgs::AgentCachedTexture {
    use aurora_msg::msgs::agent_cached_texture as m;
    let mut msg = aurora_msg::msgs::AgentCachedTexture::default();
    msg.agent_data.agent_id = agent;
    msg.agent_data.session_id = session;
    msg.agent_data.serial_num = serial;
    msg.wearable_data = keys
        .iter()
        .map(|(id, te)| m::WearableData {
            id: *id,
            texture_index: *te,
        })
        .collect();
    msg
}

/// Largest bake OpenSim takes (UploadBakedTextureModule.cs).
const MAX_UPLOAD: usize = 6_000_000;

/// Upload one bake (LLViewerAssetUpload, the `UploadBakedTexture`
/// capability): ask for an uploader, then send it the JPEG 2000 bytes. The
/// answer names the new texture.
pub async fn upload(http: &reqwest::Client, cap: &str, j2c: Vec<u8>) -> Result<Uuid, String> {
    if j2c.is_empty() || j2c.len() > MAX_UPLOAD {
        return Err(aurora_i18n::tr!("local-bakes-the-bake-is-too-large").into());
    }
    let failed = || aurora_i18n::tr!("local-bakes-the-bake-could-not-be-uploaded").to_string();
    let reply = crate::caps::post_llsd(http, cap, &llsd_map! { "item_id" => Uuid::nil() })
        .await
        .map_err(|_| failed())?;
    let uploader = reply["uploader"].as_str().to_owned();
    if uploader.is_empty() {
        return Err(failed());
    }
    let response = http
        .post(&uploader)
        .header("Content-Type", "image/x-j2c")
        .body(j2c)
        .send()
        .await
        .map_err(|_| failed())?;
    if !response.status().is_success() {
        return Err(failed());
    }
    let bytes = response.bytes().await.map_err(|_| failed())?;
    let done: Llsd = aurora_llsd::from_xml(&bytes).map_err(|_| failed())?;
    let id = done["new_asset"].as_uuid();
    if done["state"].as_str() != "complete" || id.is_nil() {
        return Err(failed());
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_key_is_the_legacy_viewers() {
        // MD5 of the shape's and skin's asset ids, then the head bake's id
        // (hashlib, computed outside the viewer)
        let worn = |kind| match kind {
            SHAPE => vec![Uuid::from_u128(1)],
            SKIN => vec![Uuid::from_u128(2)],
            _ => Vec::new(),
        };
        assert_eq!(cache_id(&BAKES[0], worn), Some(uuid!("1d53c4a5-ced6-e222-5000-ea4bbdb28553")));
    }

    #[test]
    fn a_bake_with_nothing_of_it_worn_has_no_key() {
        assert_eq!(cache_id(&BAKES[4], |_| Vec::new()), None);
        // the skirt bake is only the skirt and the universal
        assert_eq!(
            cache_id(&BAKES[4], |kind| if kind == SHAPE { vec![Uuid::from_u128(1)] } else { Vec::new() }),
            None
        );
    }

    #[test]
    fn the_bakes_are_opensims_faces() {
        let faces: Vec<u8> = BAKES.iter().map(|b| b.te).collect();
        assert_eq!(faces, [8, 9, 10, 11, 19, 20, 40, 41, 42, 43, 44]);
    }

    #[test]
    fn the_message_carries_the_appearance_in_order() {
        let a = Appearance {
            size: Vec3::new(0.45, 0.6, 1.9),
            cache: vec![(Uuid::from_u128(7), 8)],
            texture_entry: vec![1, 2, 3],
            params: vec![10, 20, 30],
        };
        let msg = agent_set_appearance(Uuid::from_u128(1), Uuid::from_u128(2), 3, &a);
        assert_eq!((msg.agent_data.serial_num, msg.agent_data.size), (3, a.size));
        assert_eq!(
            (msg.wearable_data[0].cache_id, msg.wearable_data[0].texture_index),
            (Uuid::from_u128(7), 8)
        );
        assert_eq!(msg.object_data.texture_entry, vec![1, 2, 3]);
        assert_eq!(msg.visual_param.iter().map(|p| p.param_value).collect::<Vec<_>>(), [10, 20, 30]);
    }

    #[test]
    fn the_cache_query_carries_each_key_with_its_face() {
        let msg = agent_cached_texture(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            5,
            &[(Uuid::from_u128(7), 8), (Uuid::from_u128(9), 20)],
        );
        assert_eq!(msg.agent_data.serial_num, 5);
        assert_eq!(
            msg.wearable_data.iter().map(|w| (w.id, w.texture_index)).collect::<Vec<_>>(),
            [(Uuid::from_u128(7), 8), (Uuid::from_u128(9), 20)]
        );
    }

    #[tokio::test]
    async fn a_bake_goes_up_through_its_uploader() {
        let asset = Uuid::from_u128(42);
        let (uploader, sent) = crate::outfits::tests::server(vec![(
            200,
            llsd_map! { "new_asset" => asset, "new_inventory_item" => Uuid::nil(), "state" => "complete" },
        )])
        .await;
        let (cap, asked) = crate::outfits::tests::server(vec![(
            200,
            llsd_map! { "uploader" => format!("{uploader}/up"), "state" => "upload" },
        )])
        .await;
        let id = upload(&reqwest::Client::new(), &cap, vec![1, 2, 3]).await;
        assert_eq!(id, Ok(asset));
        let asked = asked.await.expect("cap server");
        assert_eq!(asked[0].1["item_id"].as_uuid(), Uuid::nil());
        let sent = sent.await.expect("uploader server");
        assert!(sent[0].0.starts_with("POST /up"), "{:?}", sent[0].0);
        assert_eq!(sent[0].1, Llsd::Binary(vec![1, 2, 3]), "the JPEG 2000 bytes as they are");
    }

    #[tokio::test]
    async fn an_upload_the_server_does_not_complete_fails() {
        let (uploader, _sent) = crate::outfits::tests::server(vec![(200, llsd_map! { "state" => "error" })]).await;
        let (cap, _asked) = crate::outfits::tests::server(vec![(200, llsd_map! { "uploader" => uploader })]).await;
        assert!(upload(&reqwest::Client::new(), &cap, vec![1]).await.is_err());
        assert!(
            upload(&reqwest::Client::new(), &cap, Vec::new()).await.is_err(),
            "an empty bake is refused"
        );
    }
}
