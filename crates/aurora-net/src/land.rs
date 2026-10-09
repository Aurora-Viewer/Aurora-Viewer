//! Parcel ("About Land") protocol: the selected parcel, its properties
//! update, access / ban / experience lists, dwell, object owners, object
//! return, release / deed / pass, the estate covenant and the parcel
//! environment.
//!
//! Port of LLViewerParcelMgr (newview/llviewerparcelmgr.cpp), LLParcel
//! (llinventory/llparcel.cpp), LLFloaterLand's message senders
//! (newview/llfloaterland.cpp), process_covenant_reply
//! (newview/llfloaterregioninfo.cpp), LLTransferSourceParamsEstate
//! (llmessage/lltransfermanager.cpp), LLNotecard::importStream
//! (llinventory/llnotecard.cpp), LLEnvironment::coroUpdateEnvironment /
//! coroResetEnvironment (newview/llenvironment.cpp), originally LGPL 2.1.

use crate::types::{ParcelInfo, RegionHandle};
use aurora_llsd::Llsd;
use glam::Vec3;
use std::sync::Arc;
use uuid::Uuid;

/// ParcelPropertiesRequest sequence id of the selection (SELECTED_PARCEL_SEQ_ID).
pub const SELECTED_PARCEL_SEQ_ID: i32 = -10000;
/// Parcel request results (llparcelflags.h PARCEL_RESULT_*).
pub const PARCEL_RESULT_NO_DATA: i32 = -1;
pub const PARCEL_RESULT_SUCCESS: i32 = 0;
pub const PARCEL_RESULT_MULTIPLE: i32 = 1;

/// Access list kinds (llparcelflags.h AL_*).
pub const AL_ACCESS: u32 = 1 << 0;
pub const AL_BAN: u32 = 1 << 1;
pub const AL_ALLOW_EXPERIENCE: u32 = 1 << 3;
pub const AL_BLOCK_EXPERIENCE: u32 = 1 << 4;
/// Most entries of one access / ban list (PARCEL_MAX_ACCESS_LIST).
pub const PARCEL_MAX_ACCESS_LIST: usize = 300;
/// Most experiences per list (PARCEL_MAX_EXPERIENCE_LIST).
pub const PARCEL_MAX_EXPERIENCE_LIST: usize = 24;
/// Entries per ParcelAccessListUpdate (PARCEL_MAX_ENTRIES_PER_PACKET).
pub const PARCEL_MAX_ENTRIES_PER_PACKET: usize = 48;

/// Object return / selection kinds (llparcelflags.h RT_*).
pub const RT_OWNER: u32 = 1 << 1;
pub const RT_GROUP: u32 = 1 << 2;
pub const RT_OTHER: u32 = 1 << 3;
pub const RT_LIST: u32 = 1 << 4;

/// Parcel ownership status (LLParcel::EOwnershipStatus).
pub const OS_LEASED: u8 = 0;
pub const OS_LEASE_PENDING: u8 = 1;

/// ParcelExtendedFlags bit: media on prims outside the parcel stays hidden.
pub const PEF_OBSCURE_MOAP: u32 = 1;

/// One entry of an access, ban or experience list (LLAccessEntry):
/// `time` = Unix time the entry expires, 0 = never.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessEntry {
    pub id: Uuid,
    pub time: i32,
    pub flags: u32,
}

/// One row of ParcelObjectOwnersReply.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectOwner {
    pub id: Uuid,
    pub is_group: bool,
    pub count: i32,
    /// Deprecated by the server (always false); the viewer works it out.
    pub online: bool,
    /// DataExtended TimeStamp: Unix time of the most recent object.
    pub most_recent: u32,
}

/// The parcel fields a properties update sends (LLParcel::packMessage).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParcelUpdate {
    pub local_id: i32,
    pub flags: u32,
    pub sale_price: i32,
    pub name: String,
    pub desc: String,
    pub music_url: String,
    pub media_url: String,
    pub media_desc: String,
    pub media_type: String,
    pub media_width: i32,
    pub media_height: i32,
    pub media_auto_scale: bool,
    pub media_loop: bool,
    pub media_current_url: String,
    pub media_id: Uuid,
    pub media_allow_navigate: bool,
    pub media_prevent_camera_zoom: bool,
    pub media_url_timeout: f32,
    pub group_id: Uuid,
    pub pass_price: i32,
    pub pass_hours: f32,
    pub category: u8,
    pub auth_buyer: Uuid,
    pub snapshot_id: Uuid,
    pub user_location: Vec3,
    pub user_look_at: Vec3,
    pub landing_type: u8,
    pub see_avs: bool,
    pub group_av_sounds: bool,
    pub any_av_sounds: bool,
    pub obscure_moap: bool,
}

impl ParcelUpdate {
    /// The update that keeps everything of `p` as it is.
    pub fn from_parcel(p: &ParcelInfo) -> ParcelUpdate {
        ParcelUpdate {
            local_id: p.local_id,
            flags: p.flags,
            sale_price: p.sale_price,
            name: p.name.clone(),
            desc: p.desc.clone(),
            music_url: p.music_url.clone(),
            media_url: p.media_url.clone(),
            media_desc: p.media.desc.clone(),
            media_type: p.media.mime.clone(),
            media_width: p.media.width,
            media_height: p.media.height,
            media_auto_scale: p.media.auto_scale,
            media_loop: p.media.looping,
            media_current_url: p.media.current_url.clone(),
            media_id: p.media.media_id,
            media_allow_navigate: p.media.allow_navigate,
            media_prevent_camera_zoom: p.media.prevent_camera_zoom,
            media_url_timeout: p.media.url_timeout,
            group_id: p.group_id,
            pass_price: p.pass_price,
            pass_hours: p.pass_hours,
            category: p.category as u8,
            auth_buyer: p.auth_buyer,
            snapshot_id: p.snapshot_id,
            user_location: p.user_location,
            user_look_at: p.user_look_at,
            landing_type: p.landing_type as u8,
            see_avs: p.see_avatars,
            group_av_sounds: p.group_av_sounds,
            any_av_sounds: p.any_av_sounds,
            obscure_moap: p.media.obscure_moap,
        }
    }

    /// Body of the ParcelPropertiesUpdate capability (LLParcel::packMessage
    /// with message flags 0x01: the simulator answers with the selection).
    pub fn to_llsd(&self) -> Llsd {
        let mut m = Llsd::new_map();
        m.insert("flags", Llsd::Binary(1u32.to_be_bytes().to_vec()));
        m.insert("local_id", self.local_id);
        m.insert("parcel_flags", Llsd::Binary(self.flags.to_be_bytes().to_vec()));
        m.insert("sale_price", self.sale_price);
        m.insert("name", self.name.as_str());
        m.insert("description", self.desc.as_str());
        m.insert("music_url", self.music_url.as_str());
        m.insert("media_url", self.media_url.as_str());
        m.insert("media_desc", self.media_desc.as_str());
        m.insert("media_type", self.media_type.as_str());
        m.insert("media_width", self.media_width);
        m.insert("media_height", self.media_height);
        m.insert("auto_scale", self.media_auto_scale);
        m.insert("media_loop", self.media_loop);
        m.insert("media_current_url", self.media_current_url.as_str());
        // OBSOLETE, still sent by the viewer
        m.insert("obscure_media", false);
        m.insert("obscure_music", false);
        m.insert("media_id", self.media_id);
        m.insert("media_allow_navigate", self.media_allow_navigate);
        m.insert("media_prevent_camera_zoom", self.media_prevent_camera_zoom);
        m.insert("media_url_timeout", self.media_url_timeout as f64);
        m.insert("group_id", self.group_id);
        m.insert("pass_price", self.pass_price);
        m.insert("pass_hours", self.pass_hours as f64);
        m.insert("category", self.category as i32);
        m.insert("auth_buyer_id", self.auth_buyer);
        m.insert("snapshot_id", self.snapshot_id);
        m.insert("user_location", vec3_llsd(self.user_location));
        m.insert("user_look_at", vec3_llsd(self.user_look_at));
        m.insert("landing_type", self.landing_type as i32);
        m.insert("see_avs", self.see_avs);
        m.insert("group_av_sounds", self.group_av_sounds);
        m.insert("any_av_sounds", self.any_av_sounds);
        m.insert("obscure_moap", self.obscure_moap);
        m
    }
}

fn vec3_llsd(v: Vec3) -> Llsd {
    Llsd::Array(vec![Llsd::Real(v.x as f64), Llsd::Real(v.y as f64), Llsd::Real(v.z as f64)])
}

/// Requests of the About Land floater; `handle` is the region of the
/// selection (LLViewerParcelMgr::getSelectionRegion).
#[derive(Debug, Clone)]
pub enum LandCommand {
    /// Select the parcel(s) in a rectangle of region coordinates
    /// (LLViewerParcelMgr::selectLand: ParcelPropertiesRequest with
    /// SELECTED_PARCEL_SEQ_ID); `snap` selects the whole parcel hit.
    Select {
        handle: RegionHandle,
        west: f32,
        south: f32,
        east: f32,
        north: f32,
        snap: bool,
    },
    /// sendParcelPropertiesUpdate: capability, else ParcelPropertiesUpdate.
    Update { handle: RegionHandle, update: Box<ParcelUpdate> },
    /// sendParcelAccessListRequest (`flags` = AL_* bits).
    AccessListRequest { handle: RegionHandle, local_id: i32, flags: u32 },
    /// sendParcelAccessListUpdate: the whole list of one kind.
    AccessListUpdate {
        handle: RegionHandle,
        local_id: i32,
        flags: u32,
        entries: Vec<AccessEntry>,
    },
    /// sendParcelDwellRequest.
    DwellRequest { handle: RegionHandle, local_id: i32 },
    /// LLPanelLandObjects::onClickRefresh: ParcelObjectOwnersRequest.
    ObjectOwnersRequest { handle: RegionHandle, local_id: i32 },
    /// send_return_objects_message (`owners` with RT_LIST).
    ReturnObjects {
        handle: RegionHandle,
        local_id: i32,
        return_type: u32,
        owners: Vec<Uuid>,
    },
    /// send_other_clean_time_message: auto-return delay in minutes.
    SetOtherCleanTime { handle: RegionHandle, local_id: i32, minutes: i32 },
    /// sendParcelRelease (Abandon Land).
    Release { handle: RegionHandle, local_id: i32 },
    /// reclaimParcel (region owner).
    Reclaim { handle: RegionHandle, local_id: i32 },
    /// sendParcelDeed.
    DeedToGroup { handle: RegionHandle, local_id: i32, group: Uuid },
    /// buyPass.
    BuyPass { handle: RegionHandle, local_id: i32 },
    /// EstateCovenantRequest (LLPanelLandCovenant::refresh).
    CovenantRequest { handle: RegionHandle },
    /// RemoteParcelRequest capability: the parcel UUID of a point.
    ParcelIdRequest {
        handle: RegionHandle,
        local_id: i32,
        region_id: Uuid,
        position: Vec3,
    },
    /// LLEnvironment::requestParcel: GET ExtEnvironment?parcelid=N (the
    /// answer is only shown, not applied to the sky).
    EnvironmentRequest { local_id: i32 },
    /// updateParcel with the day length / offset (seconds; 0 = unchanged).
    EnvironmentUpdate { local_id: i32, day_length: i32, day_offset: i32 },
    /// resetParcel: back to the region environment.
    EnvironmentReset { local_id: i32 },
    /// Names of experiences (GetExperienceInfo capability).
    ExperienceInfo(Vec<Uuid>),
    /// Group names (UUIDGroupNameRequest, LLCacheName::getGroupName).
    GroupNames(Vec<Uuid>),
    /// Resident search of the avatar picker (LLFloaterAvatarPicker::find):
    /// AvatarPickerSearch capability, else AvatarPickerRequest.
    AvatarSearch { query: String },
    /// LLFloaterURLEntry::getMediaTypeCoro: HEAD of a parcel media URL.
    MediaType { url: String },
}

/// Answers for the About Land floater.
#[derive(Debug, Clone)]
pub enum LandEvent {
    /// ParcelProperties of the selection (SELECTED_PARCEL_SEQ_ID).
    Selected {
        handle: RegionHandle,
        info: Arc<ParcelInfo>,
        /// PARCEL_RESULT_*.
        result: i32,
    },
    /// ParcelAccessListReply of the selected parcel (one AL_* kind).
    AccessList {
        local_id: i32,
        flags: u32,
        entries: Vec<AccessEntry>,
    },
    /// ParcelDwellReply.
    Dwell { local_id: i32, dwell: f32 },
    /// ParcelObjectOwnersReply.
    ObjectOwners(Vec<ObjectOwner>),
    /// EstateCovenantReply.
    Covenant {
        handle: RegionHandle,
        covenant_id: Uuid,
        timestamp: u32,
        estate_name: String,
        estate_owner: Uuid,
    },
    /// The covenant notecard text (None: it could not be loaded).
    CovenantText { covenant_id: Uuid, text: Option<String> },
    /// RemoteParcelRequest answer (None: the parcel id could not be resolved).
    ParcelId { local_id: i32, id: Option<Uuid> },
    /// ExtEnvironment of a parcel (`environment` map), or why it failed.
    Environment { local_id: i32, result: Result<Llsd, String> },
    /// GetExperienceInfo: (experience, name).
    ExperienceInfo(Vec<(Uuid, String)>),
    /// UUIDGroupNameReply: (group, name).
    GroupNames(Vec<(Uuid, String)>),
    /// Avatar picker results for `query`, None when the search failed.
    AvatarSearch { query: String, results: Option<Vec<FoundAvatar>> },
    /// MIME type of a media URL ("none/none" when it could not be found).
    MediaType { url: String, mime: String },
}

/// One resident found by the avatar picker.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundAvatar {
    pub id: Uuid,
    pub display_name: String,
    pub username: String,
}

/// Escape a picker query like LLURI::escape after the viewer replaced
/// the dots of usernames with spaces.
pub fn avatar_search_query(text: &str) -> String {
    text.replace('.', " ")
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Parse the ParcelProperties event queue message (LLSD form of the UDP
/// message, LLViewerParcelMgr::processParcelProperties + LLParcel::unpackMessage).
pub fn parse_parcel_properties(b: &Llsd) -> ParcelInfo {
    let pd = &b["ParcelData"][0];
    let vec3 = |v: &Llsd| -> Vec3 {
        let r = Vec3::new(v.at(0).as_f32(), v.at(1).as_f32(), v.at(2).as_f32());
        if r.is_finite() { r } else { Vec3::ZERO }
    };
    let has_md = b.has("MediaData");
    let md = &b["MediaData"][0];
    let ls = &b["MediaLinkSharing"][0];
    // SeeAVs / AnyAVSounds / GroupAVSounds all come from newer servers;
    // missing, they default to true (have_new_parcel_limit_data false)
    let new_limits = pd.has("SeeAVs") && pd.has("AnyAVSounds") && pd.has("GroupAVSounds");
    let limit = |k: &str| !new_limits || pd[k].as_bool();
    ParcelInfo {
        local_id: pd["LocalID"].as_i32(),
        sequence_id: pd["SequenceID"].as_i32(),
        request_result: pd["RequestResult"].as_i32(),
        name: pd["Name"].to_string_value(),
        desc: pd["Desc"].to_string_value(),
        owner_id: pd["OwnerID"].as_uuid(),
        group_id: pd["GroupID"].as_uuid(),
        is_group_owned: pd["IsGroupOwned"].as_bool(),
        auction_id: u32_of(&pd["AuctionID"]),
        status: pd["Status"].as_i32() as u8,
        area: pd["Area"].as_i32(),
        claim_date: pd["ClaimDate"].as_i32(),
        claim_price: pd["ClaimPrice"].as_i32(),
        rent_price: pd["RentPrice"].as_i32(),
        aabb_min: vec3(&pd["AABBMin"]),
        aabb_max: vec3(&pd["AABBMax"]),
        max_prims: pd["MaxPrims"].as_i32(),
        total_prims: pd["TotalPrims"].as_i32(),
        owner_prims: pd["OwnerPrims"].as_i32(),
        group_prims: pd["GroupPrims"].as_i32(),
        other_prims: pd["OtherPrims"].as_i32(),
        selected_prims: pd["SelectedPrims"].as_i32(),
        prim_bonus: if pd.has("ParcelPrimBonus") {
            pd["ParcelPrimBonus"].as_f32()
        } else {
            1.0
        },
        sim_max_prims: pd["SimWideMaxPrims"].as_i32(),
        sim_total_prims: pd["SimWideTotalPrims"].as_i32(),
        other_clean_time: pd["OtherCleanTime"].as_i32(),
        flags: u32_of(&pd["ParcelFlags"]),
        sale_price: pd["SalePrice"].as_i32(),
        auth_buyer: pd["AuthBuyerID"].as_uuid(),
        category: pd["Category"].as_i32(),
        pass_price: pd["PassPrice"].as_i32(),
        pass_hours: pd["PassHours"].as_f32(),
        music_url: pd["MusicURL"].to_string_value(),
        media_url: pd["MediaURL"].to_string_value(),
        snapshot_id: pd["SnapshotID"].as_uuid(),
        user_location: vec3(&pd["UserLocation"]),
        user_look_at: vec3(&pd["UserLookAt"]),
        landing_type: pd["LandingType"].as_i32(),
        see_avatars: limit("SeeAVs"),
        any_av_sounds: limit("AnyAVSounds"),
        group_av_sounds: limit("GroupAVSounds"),
        have_new_parcel_limit_data: new_limits,
        region_push_override: pd["RegionPushOverride"].as_bool(),
        region_deny_anonymous: pd["RegionDenyAnonymous"].as_bool(),
        region_deny_age_unverified: b["AgeVerificationBlock"][0]["RegionDenyAgeUnverified"].as_bool(),
        // missing block: older servers always allowed it
        region_allow_access_override: !b.has("RegionAllowAccessBlock")
            || b["RegionAllowAccessBlock"][0]["RegionAllowAccessOverride"].as_bool(),
        region_allow_env_override: b["ParcelEnvironmentBlock"][0]["RegionAllowEnvironmentOverride"].as_bool(),
        env_version: b["ParcelEnvironmentBlock"][0]["ParcelEnvironmentVersion"].as_i32(),
        media: crate::types::ParcelMedia {
            media_id: pd["MediaID"].as_uuid(),
            auto_scale: pd["MediaAutoScale"].as_bool(),
            // no MediaData block: legacy QuickTime type, looping
            mime: if has_md {
                md["MediaType"].to_string_value()
            } else {
                "video/vnd.secondlife.qt.legacy".into()
            },
            desc: md["MediaDesc"].to_string_value(),
            width: md["MediaWidth"].as_i32(),
            height: md["MediaHeight"].as_i32(),
            looping: !has_md || md["MediaLoop"].as_bool(),
            current_url: ls["MediaCurrentURL"].to_string_value(),
            allow_navigate: ls["MediaAllowNavigate"].as_bool(),
            prevent_camera_zoom: ls["MediaPreventCameraZoom"].as_bool(),
            url_timeout: ls["MediaURLTimeout"].as_f32(),
            obscure_moap: u32_of(&b["ParcelExtendedFlags"][0]["Flags"]) & PEF_OBSCURE_MOAP != 0,
        },
    }
}

/// U32 fields travel as 4-byte big-endian binaries in LLSD.
pub fn u32_of(v: &Llsd) -> u32 {
    match v {
        Llsd::Binary(b) if b.len() == 4 => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        other => other.as_i32() as u32,
    }
}

/// ParcelObjectOwnersReply from the event queue (the UDP message is deprecated).
pub fn parse_object_owners(b: &Llsd) -> Vec<ObjectOwner> {
    let ext = &b["DataExtended"];
    b["Data"]
        .as_array()
        .iter()
        .enumerate()
        .filter(|(_, d)| !d["OwnerID"].as_uuid().is_nil())
        .map(|(i, d)| ObjectOwner {
            id: d["OwnerID"].as_uuid(),
            is_group: d["IsGroupOwned"].as_bool(),
            count: d["Count"].as_i32(),
            online: d["OnlineStatus"].as_bool(),
            most_recent: u32_of(&ext.at(i)["TimeStamp"]),
        })
        .collect()
}

/// Split an access list into ParcelAccessListUpdate messages
/// (sendParcelAccessListUpdate): `Sections` is computed like the viewer
/// (integer division, then ceil), and an empty list still sends one
/// message with a null entry so the simulator clears it.
pub fn access_list_chunks(entries: &[AccessEntry]) -> (i32, Vec<Vec<AccessEntry>>) {
    let sections = (entries.len() / PARCEL_MAX_ENTRIES_PER_PACKET) as i32;
    if entries.is_empty() {
        let null = AccessEntry {
            id: Uuid::nil(),
            time: 0,
            flags: 0,
        };
        return (sections, vec![vec![null]]);
    }
    (sections, entries.chunks(PARCEL_MAX_ENTRIES_PER_PACKET).map(<[_]>::to_vec).collect())
}

/// Transfer source parameters of an estate asset (LLTransferSourceParamsEstate
/// packed by LLDataPackerBinaryBuffer): agent, session, EstateAssetType
/// (ET_Covenant = 0) little-endian.
pub fn covenant_transfer_params(agent: Uuid, session: Uuid) -> Vec<u8> {
    let mut v = Vec::with_capacity(36);
    v.extend_from_slice(agent.as_bytes());
    v.extend_from_slice(session.as_bytes());
    v.extend_from_slice(&0i32.to_le_bytes());
    v
}

/// An asset arriving by TransferPacket (LLTransferTarget, receive side):
/// packets may come out of order.
#[derive(Debug, Default)]
pub struct TransferAssembly {
    /// Expected size (TransferInfo), 0 until known.
    pub size: usize,
    packets: std::collections::BTreeMap<i32, Vec<u8>>,
    done: bool,
}

impl TransferAssembly {
    pub fn set_size(&mut self, size: i32) {
        self.size = size.max(0) as usize;
    }

    /// Add one packet; `last` when its status is LLTS_DONE.
    pub fn add(&mut self, packet: i32, data: Vec<u8>, last: bool) {
        self.packets.insert(packet, data);
        self.done |= last;
    }

    fn received(&self) -> usize {
        self.packets.values().map(Vec::len).sum()
    }

    /// The whole asset once every packet is in.
    pub fn complete(&self) -> Option<Vec<u8>> {
        let contiguous = self.packets.keys().copied().eq(0..self.packets.len() as i32);
        let full = (self.size > 0 && self.received() >= self.size) || self.done;
        (contiguous && full && !self.packets.is_empty()).then(|| self.packets.values().flatten().copied().collect())
    }
}

/// Text of a notecard asset (LLNotecard::importStream: "Linden text version
/// N { LLEmbeddedItems ... Text length L <text> }"); embedded inventory
/// characters (private use area) are dropped.
pub fn notecard_text(data: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(data);
    if !s.starts_with("Linden text version") {
        return None;
    }
    let key = "Text length ";
    let at = s.find(key)? + key.len();
    let rest = &s[at..];
    let nl = rest.find('\n')?;
    let len: usize = rest[..nl].trim().parse().ok()?;
    let body = &rest.as_bytes()[nl + 1..];
    let text = String::from_utf8_lossy(&body[..len.min(body.len())]).into_owned();
    Some(text.chars().filter(|c| !('\u{F0000}'..='\u{FFFFD}').contains(c)).collect())
}

/// Body of an ExtEnvironment PUT changing only the day length / offset
/// (coroUpdateEnvironment for a parcel; `flags` 0).
pub fn environment_update_body(day_length: i32, day_offset: i32) -> Llsd {
    let mut env = Llsd::new_map();
    if day_length > 0 {
        env.insert("day_length", day_length);
    }
    if day_offset > 0 {
        env.insert("day_offset", day_offset);
    }
    env.insert("flags", 0);
    let mut body = Llsd::new_map();
    body.insert("environment", env);
    body
}

/// Why a RemoteParcelRequest gave no parcel (LLPanelPlaceInfo::setErrorStatus
/// and the cases around it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteParcelError {
    /// The region has no RemoteParcelRequest capability
    /// (LLPanelPlaceInfo::displayParcelInfo: "server_update_text").
    NoCapability,
    /// HTTP error status (404 and 499 have their own texts).
    Status(u16),
    /// The request could not be sent or its answer read.
    Failed,
    /// The answer has no parcel at that point (null `parcel_id`).
    NoParcel,
}

/// Body of a RemoteParcelRequest POST
/// (LLRemoteParcelInfoProcessor::regionParcelInfoCoro): the point in region
/// coordinates, the region id when known and the handle of the 256 m slot
/// holding the point (`ll_sd_from_U64`: 8 bytes, big-endian).
pub fn remote_parcel_body(location: Vec3, region_id: Uuid, handle: Option<RegionHandle>) -> Llsd {
    let mut body = Llsd::new_map();
    body.insert("location", vec3_llsd(location));
    if !region_id.is_nil() {
        body.insert("region_id", region_id);
    }
    if let Some(h) = handle {
        body.insert("region_handle", Llsd::Binary(h.to_be_bytes().to_vec()));
    }
    body
}

/// The parcel of a RemoteParcelRequest answer (`parcel_id`).
pub fn parse_remote_parcel_reply(v: &Llsd) -> Result<Uuid, RemoteParcelError> {
    let id = v["parcel_id"].as_uuid();
    if id.is_nil() { Err(RemoteParcelError::NoParcel) } else { Ok(id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> Llsd {
        props_from(PROPS)
    }

    fn props_from(xml: &str) -> Llsd {
        aurora_llsd::from_xml(xml.as_bytes()).expect("test llsd")
    }

    const PROPS: &str = r#"<llsd><map>
            <key>ParcelData</key><array><map>
                <key>LocalID</key><integer>7</integer>
                <key>SequenceID</key><integer>-10000</integer>
                <key>RequestResult</key><integer>0</integer>
                <key>Name</key><string>Tordangle Harbor</string>
                <key>OwnerID</key><uuid>11111111-1111-1111-1111-111111111111</uuid>
                <key>IsGroupOwned</key><boolean>1</boolean>
                <key>AuctionID</key><binary encoding="base64">AAAAAw==</binary>
                <key>Status</key><integer>0</integer>
                <key>Area</key><integer>12288</integer>
                <key>ParcelFlags</key><binary encoding="base64">IAAAAQ==</binary>
                <key>ParcelPrimBonus</key><real>1.5</real>
                <key>PassHours</key><real>1</real>
                <key>UserLocation</key><array><real>10</real><real>20</real><real>30</real></array>
                <key>AABBMin</key><array><real>0</real><real>64</real><real>0</real></array>
                <key>AABBMax</key><array><real>128</real><real>160</real><real>0</real></array>
                <key>SeeAVs</key><boolean>0</boolean>
                <key>AnyAVSounds</key><boolean>1</boolean>
                <key>GroupAVSounds</key><boolean>1</boolean>
            </map></array>
            <key>ParcelExtendedFlags</key><array><map><key>Flags</key><binary encoding="base64">AAAAAQ==</binary></map></array>
        </map></llsd>"#;

    #[test]
    fn parses_selected_parcel() {
        let p = parse_parcel_properties(&props());
        assert_eq!(p.local_id, 7);
        assert_eq!(p.sequence_id, SELECTED_PARCEL_SEQ_ID);
        assert_eq!(p.auction_id, 3);
        assert_eq!(p.flags, 0x2000_0001);
        assert_eq!(p.prim_bonus, 1.5);
        assert_eq!(p.user_location, Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(p.aabb_max, Vec3::new(128.0, 160.0, 0.0));
        assert!(p.have_new_parcel_limit_data);
        assert!(!p.see_avatars);
        assert!(p.media.obscure_moap);
        // MediaData missing: the legacy defaults
        assert!(p.media.looping);
        assert_eq!(p.media.mime, "video/vnd.secondlife.qt.legacy");
        // RegionAllowAccessBlock missing: allowed
        assert!(p.region_allow_access_override);
    }

    #[test]
    fn old_servers_default_limits_to_true() {
        let b = props_from(&PROPS.replace("<key>GroupAVSounds</key><boolean>1</boolean>", ""));
        let p = parse_parcel_properties(&b);
        assert!(!p.have_new_parcel_limit_data);
        assert!(p.see_avatars && p.any_av_sounds && p.group_av_sounds);
    }

    #[test]
    fn update_round_trips_the_parcel() {
        let p = parse_parcel_properties(&props());
        let u = ParcelUpdate::from_parcel(&p);
        let sd = u.to_llsd();
        assert_eq!(sd["local_id"].as_i32(), 7);
        assert_eq!(u32_of(&sd["parcel_flags"]), 0x2000_0001);
        assert_eq!(u32_of(&sd["flags"]), 1);
        assert_eq!(sd["name"].as_str(), "Tordangle Harbor");
        assert_eq!(sd["user_location"].at(2).as_f32(), 30.0);
        assert!(!sd["see_avs"].as_bool());
        assert!(sd["obscure_moap"].as_bool());
    }

    #[test]
    fn access_lists_split_like_the_viewer() {
        let e = |n: u128| AccessEntry {
            id: Uuid::from_u128(n + 1),
            time: 0,
            flags: 0,
        };
        let (sections, chunks) = access_list_chunks(&[]);
        assert_eq!(sections, 0);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0][0].id.is_nil());
        let list: Vec<_> = (0..100).map(e).collect();
        let (sections, chunks) = access_list_chunks(&list);
        assert_eq!(sections, 2);
        assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), [48, 48, 4]);
    }

    #[test]
    fn object_owners_with_timestamps() {
        let xml = r#"<llsd><map>
            <key>Data</key><array>
                <map><key>OwnerID</key><uuid>22222222-2222-2222-2222-222222222222</uuid><key>IsGroupOwned</key><boolean>0</boolean><key>Count</key><integer>12</integer><key>OnlineStatus</key><boolean>0</boolean></map>
                <map><key>OwnerID</key><uuid>00000000-0000-0000-0000-000000000000</uuid><key>Count</key><integer>1</integer></map>
            </array>
            <key>DataExtended</key><array>
                <map><key>TimeStamp</key><binary encoding="base64">ZQAAAA==</binary></map>
                <map><key>TimeStamp</key><binary encoding="base64">AAAAAA==</binary></map>
            </array>
        </map></llsd>"#;
        let b = aurora_llsd::from_xml(xml.as_bytes()).expect("test llsd");
        let o = parse_object_owners(&b);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].count, 12);
        assert_eq!(o[0].most_recent, 0x6500_0000);
    }

    #[test]
    fn transfer_assembles_out_of_order() {
        let mut t = TransferAssembly::default();
        t.set_size(6);
        t.add(1, b"def".to_vec(), true);
        assert_eq!(t.complete(), None);
        t.add(0, b"abc".to_vec(), false);
        assert_eq!(t.complete().as_deref(), Some(&b"abcdef"[..]));
    }

    #[test]
    fn covenant_params_layout() {
        let a = Uuid::from_u128(1);
        let s = Uuid::from_u128(2);
        let p = covenant_transfer_params(a, s);
        assert_eq!(p.len(), 36);
        assert_eq!(&p[..16], a.as_bytes());
        assert_eq!(&p[32..], &[0, 0, 0, 0]);
    }

    #[test]
    fn reads_notecard_text() {
        let card = "Linden text version 2\n{\nLLEmbeddedItems version 1\n{\ncount 0\n}\nText length 23\nBy purchasing\u{F0000} land\n}\n";
        assert_eq!(notecard_text(card.as_bytes()).as_deref(), Some("By purchasing land\n"));
        assert_eq!(notecard_text(b"not a notecard"), None);
    }

    #[test]
    fn picker_query_is_escaped() {
        assert_eq!(avatar_search_query("tess.touch"), "tess%20touch");
        assert_eq!(avatar_search_query("Zoë"), "Zo%C3%AB");
    }

    #[test]
    fn environment_update_omits_unset_values() {
        let b = environment_update_body(14400, 0);
        assert_eq!(b["environment"]["day_length"].as_i32(), 14400);
        assert!(!b["environment"].has("day_offset"));
    }

    #[test]
    fn remote_parcel_body_layout() {
        let handle = crate::types::origin_to_handle(256_000, 256_256);
        let b = remote_parcel_body(Vec3::new(140.0, 120.0, 25.0), Uuid::nil(), Some(handle));
        assert_eq!(b["location"].as_vec3(), [140.0, 120.0, 25.0]);
        assert!(!b.has("region_id"));
        match &b["region_handle"] {
            Llsd::Binary(v) => assert_eq!(v, &[0, 3, 232, 0, 0, 3, 233, 0]),
            other => panic!("region_handle {other:?}"),
        }
        let r = Uuid::from_u128(7);
        assert_eq!(remote_parcel_body(Vec3::ZERO, r, None)["region_id"].as_uuid(), r);
    }

    #[test]
    fn remote_parcel_reply() {
        let id = Uuid::from_u128(0x9A2C);
        let mut v = Llsd::new_map();
        v.insert("parcel_id", id);
        assert_eq!(parse_remote_parcel_reply(&v), Ok(id));
        v.insert("parcel_id", Uuid::nil());
        assert_eq!(parse_remote_parcel_reply(&v), Err(RemoteParcelError::NoParcel));
        assert_eq!(parse_remote_parcel_reply(&Llsd::Undef), Err(RemoteParcelError::NoParcel));
    }
}
