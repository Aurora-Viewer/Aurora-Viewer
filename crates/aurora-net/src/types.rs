//! Events emitted by the network layer and commands it accepts.

use crate::login::{LoginRequest, LoginResponse};
use crate::objects::{ObjectUpdate, TerseUpdate};
use crate::terrain::TerrainPatch;
use aurora_prim::TextureEntry;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Region identifier: global origin in meters packed as `(x << 32) | y`.
pub type RegionHandle = u64;

pub fn handle_to_origin(h: RegionHandle) -> (u32, u32) {
    ((h >> 32) as u32, (h & 0xFFFF_FFFF) as u32)
}

pub fn origin_to_handle(x: u32, y: u32) -> RegionHandle {
    ((x as u64) << 32) | y as u64
}

#[derive(Debug, Clone)]
pub struct RegionInfo {
    pub handle: RegionHandle,
    pub name: String,
    pub region_id: Uuid,
    pub water_height: f32,
    pub sim_access: u8,
    pub region_flags: u32,
    pub terrain_base: [Uuid; 4],
    pub terrain_detail: [Uuid; 4],
    pub terrain_start_height: [f32; 4],
    pub terrain_height_range: [f32; 4],
    pub size_x: u32,
    pub size_y: u32,
    pub is_main: bool,
    /// RegionHandshake SimOwner / IsEstateManager (LLViewerRegion::getOwner,
    /// canManageEstate).
    pub owner: Uuid,
    pub is_estate_manager: bool,
    /// RegionInfo3 ProductName ("Mainland / Full Region"...).
    pub product_name: String,
}

/// Parcel description from ParcelProperties (About Land).
#[derive(Debug, Clone, Default)]
pub struct ParcelInfo {
    pub local_id: i32,
    /// SequenceID of the message (0+ agent parcel, -10000 selection...).
    pub sequence_id: i32,
    /// PARCEL_RESULT_* (land::PARCEL_RESULT_MULTIPLE: several parcels).
    pub request_result: i32,
    pub name: String,
    pub desc: String,
    pub owner_id: Uuid,
    pub group_id: Uuid,
    pub is_group_owned: bool,
    pub auction_id: u32,
    /// LLParcel::EOwnershipStatus (land::OS_LEASED...).
    pub status: u8,
    pub area: i32,
    pub claim_price: i32,
    pub rent_price: i32,
    /// Bounding box in region coordinates (z unused).
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
    /// Parcel prim capacity (with bonus) and usage.
    pub max_prims: i32,
    pub total_prims: i32,
    /// Region object bonus factor (ParcelPrimBonus).
    pub prim_bonus: f32,
    pub owner_prims: i32,
    pub group_prims: i32,
    pub other_prims: i32,
    pub selected_prims: i32,
    pub sim_max_prims: i32,
    pub sim_total_prims: i32,
    /// Auto-return of other residents' objects, minutes (0 = off).
    pub other_clean_time: i32,
    /// `PF_*` flags (llparcelflags.h).
    pub flags: u32,
    pub sale_price: i32,
    pub auth_buyer: Uuid,
    pub category: i32,
    pub claim_date: i32,
    pub pass_price: i32,
    pub pass_hours: f32,
    pub music_url: String,
    pub media_url: String,
    pub snapshot_id: Uuid,
    /// Landing point (zero = none) and its look-at direction.
    pub user_location: Vec3,
    pub user_look_at: Vec3,
    /// LLParcel::ELandingType: 0 blocked, 1 landing point, 2 anywhere.
    pub landing_type: i32,
    pub see_avatars: bool,
    pub any_av_sounds: bool,
    pub group_av_sounds: bool,
    /// The server sent SeeAVs / AnyAVSounds / GroupAVSounds.
    pub have_new_parcel_limit_data: bool,
    /// Estate overrides (RegionPushOverride, RegionDenyAnonymous,
    /// RegionDenyAgeUnverified, RegionAllowAccessOverride).
    pub region_push_override: bool,
    pub region_deny_anonymous: bool,
    pub region_deny_age_unverified: bool,
    pub region_allow_access_override: bool,
    pub region_allow_env_override: bool,
    pub env_version: i32,
    /// Parcel media (MediaData / MediaLinkSharing blocks).
    pub media: ParcelMedia,
}

/// Parcel media settings (LLParcel::unpackMessage).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParcelMedia {
    /// Placeholder texture replaced by the media on every face using it.
    pub media_id: Uuid,
    pub auto_scale: bool,
    /// MIME type; `video/vnd.secondlife.qt.legacy` when the block is missing.
    pub mime: String,
    pub desc: String,
    pub width: i32,
    pub height: i32,
    pub looping: bool,
    /// MediaLinkSharing: current URL of shared HTML media.
    pub current_url: String,
    pub allow_navigate: bool,
    pub prevent_camera_zoom: bool,
    pub url_timeout: f32,
    /// ParcelExtendedFlags: hide media on a prim outside this parcel.
    pub obscure_moap: bool,
}

/// One region of the world map (MapBlockReply Data + Size).
#[derive(Debug, Clone)]
pub struct MapBlock {
    /// Grid position in region units (global meters / 256).
    pub x: u16,
    pub y: u16,
    /// Empty for a non-existent slot (access 255).
    pub name: String,
    /// SIM_ACCESS_*: 13 PG, 21 mature, 42 adult, 254 down, 255 non-existent.
    pub access: u8,
    pub region_flags: u32,
    pub water_height: u8,
    pub agents: u8,
    pub map_image_id: Uuid,
    pub size_x: u16,
    pub size_y: u16,
}

/// One world map item (MapItemReply Data).
#[derive(Debug, Clone)]
pub struct MapItem {
    /// Global position in meters.
    pub x: u32,
    pub y: u32,
    pub id: Uuid,
    pub extra: i32,
    pub extra2: i32,
    pub name: String,
}

/// MapItemRequest item types (llmapitemtypes / LLWorldMap).
pub mod map_item {
    pub const TELEHUB: u32 = 1;
    pub const PG_EVENT: u32 = 2;
    pub const MATURE_EVENT: u32 = 3;
    pub const AGENT_LOCATIONS: u32 = 6;
    pub const LAND_FOR_SALE: u32 = 7;
    pub const CLASSIFIED: u32 = 8;
    pub const ADULT_EVENT: u32 = 9;
    pub const LAND_FOR_SALE_ADULT: u32 = 10;
}

pub mod parcel_flags {
    pub const ALLOW_FLY: u32 = 1 << 0;
    pub const ALLOW_OTHER_SCRIPTS: u32 = 1 << 1;
    pub const FOR_SALE: u32 = 1 << 2;
    pub const ALLOW_LANDMARK: u32 = 1 << 3;
    pub const ALLOW_TERRAFORM: u32 = 1 << 4;
    pub const ALLOW_DAMAGE: u32 = 1 << 5;
    pub const CREATE_OBJECTS: u32 = 1 << 6;
    pub const USE_ACCESS_GROUP: u32 = 1 << 8;
    pub const USE_ACCESS_LIST: u32 = 1 << 9;
    pub const USE_BAN_LIST: u32 = 1 << 10;
    pub const USE_PASS_LIST: u32 = 1 << 11;
    pub const SHOW_DIRECTORY: u32 = 1 << 12;
    pub const ALLOW_DEED_TO_GROUP: u32 = 1 << 13;
    pub const CONTRIBUTE_WITH_DEED: u32 = 1 << 14;
    pub const SOUND_LOCAL: u32 = 1 << 15;
    pub const SELL_PARCEL_OBJECTS: u32 = 1 << 16;
    pub const ALLOW_PUBLISH: u32 = 1 << 17;
    pub const MATURE_PUBLISH: u32 = 1 << 18;
    pub const RESTRICT_PUSHOBJECT: u32 = 1 << 21;
    pub const DENY_ANONYMOUS: u32 = 1 << 22;
    pub const ALLOW_GROUP_SCRIPTS: u32 = 1 << 25;
    pub const CREATE_GROUP_OBJECTS: u32 = 1 << 26;
    pub const ALLOW_ALL_OBJECT_ENTRY: u32 = 1 << 27;
    pub const ALLOW_GROUP_OBJECT_ENTRY: u32 = 1 << 28;
    pub const ALLOW_VOICE_CHAT: u32 = 1 << 29;
    /// Voice uses the estate channel (else the parcel has its own).
    pub const USE_ESTATE_VOICE_CHAN: u32 = 1 << 30;
    pub const DENY_AGEUNVERIFIED: u32 = 1 << 31;
}

/// `REGION_FLAGS_*` (llregionflags.h) of RegionHandshake / SimStats.
pub mod region_flags {
    pub const ALLOW_DAMAGE: u32 = 1 << 0;
    pub const BLOCK_LAND_RESELL: u32 = 1 << 7;
    pub const SKIP_SCRIPTS: u32 = 1 << 13;
    pub const BLOCK_FLY: u32 = 1 << 19;
    pub const ESTATE_SKIP_SCRIPTS: u32 = 1 << 21;
    pub const RESTRICT_PUSHOBJECT: u32 = 1 << 22;
    pub const ALLOW_PARCEL_CHANGES: u32 = 1 << 26;
    pub const ALLOW_VOICE: u32 = 1 << 28;
    pub const BLOCK_PARCEL_SEARCH: u32 = 1 << 29;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatType {
    Whisper,
    Normal,
    Shout,
    OwnerSay,
    Debug,
    Region,
    Other(u8),
}

impl ChatType {
    pub fn from_u8(v: u8) -> ChatType {
        match v {
            0 => ChatType::Whisper,
            1 => ChatType::Normal,
            2 => ChatType::Shout,
            6 => ChatType::Debug,
            7 => ChatType::Region,
            8 => ChatType::OwnerSay,
            o => ChatType::Other(o),
        }
    }
    pub fn to_u8(self) -> u8 {
        match self {
            ChatType::Whisper => 0,
            ChatType::Normal => 1,
            ChatType::Shout => 2,
            ChatType::Debug => 6,
            ChatType::Region => 7,
            ChatType::OwnerSay => 8,
            ChatType::Other(o) => o,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatSourceType {
    System,
    Agent,
    Object,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub from_name: String,
    pub source_id: Uuid,
    pub owner_id: Uuid,
    pub source_type: ChatSourceType,
    pub chat_type: ChatType,
    pub position: Vec3,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct InstantMessage {
    pub from_agent_id: Uuid,
    pub from_name: String,
    pub to_agent_id: Uuid,
    pub dialog: u8,
    pub session_id: Uuid,
    pub message: String,
    pub offline: bool,
    pub from_group: bool,
    /// Extra data (inventory offers: asset type + item id; group invites: fee...).
    pub binary_bucket: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AvatarAppearance {
    pub avatar_id: Uuid,
    pub texture_entry: Option<Arc<TextureEntry>>,
    pub visual_params: Vec<u8>,
    pub cof_version: i32,
    pub hover_height: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct SunInfo {
    pub sun_direction: Vec3,
    pub sun_phase: f32,
    pub sec_per_day: u32,
    pub usec_since_start: u64,
}

#[derive(Debug, Clone)]
pub enum NetEvent {
    /// The mute list (LLMuteList): downloaded file, or keep the cached copy.
    MuteList(crate::social::MuteListSource),
    /// Groups the agent belongs to (AgentGroupDataUpdate; added or updated).
    Groups(Vec<crate::social::GroupMembership>),
    /// The agent left / was removed from a group (AgentDropGroup).
    GroupDropped(Uuid),
    /// Our active group and its title (AgentDataUpdate; nil = none).
    ActiveGroup {
        id: Uuid,
        name: String,
        title: String,
    },
    /// A friend ended the friendship (TerminateFriendship from the simulator).
    FriendshipTerminated(Uuid),
    /// First message of a group / conference chat session (ChatterBoxInvitation).
    SessionInvite(crate::social::SessionInvite),
    /// Reply to a session we started (ChatterBoxSessionStartReply).
    SessionStarted {
        temp_session: Uuid,
        session: Uuid,
        success: bool,
        error: String,
        agents: Vec<crate::social::SessionAgent>,
    },
    /// Participants joined / left a session (ChatterBoxSessionAgentListUpdates).
    SessionAgents {
        session: Uuid,
        agents: Vec<crate::social::SessionAgent>,
    },
    /// A session event failed (ChatterBoxSessionEventReply, e.g. muted by a moderator).
    SessionError {
        session: Uuid,
        event: String,
        error: String,
    },
    /// The server closed a session (ForceCloseChatterBoxSession).
    SessionClosed {
        session: Uuid,
        reason: String,
    },
    /// Answer of the ChatSessionRequest capability (None = failed).
    ChatSessionReply {
        method: crate::social::SessionMethod,
        session: Uuid,
        result: Option<aurora_llsd::Llsd>,
    },
    LoginProgress {
        message: String,
        fraction: f32,
    },
    /// `bad_credentials`: refused for the user name / password (reason "key").
    LoginFailed {
        error: String,
        mfa_required: bool,
        tos_required: bool,
        bad_credentials: bool,
    },
    LoggedIn(Arc<LoginResponse>),
    /// A simulator circuit was established (main or neighbor).
    RegionHandshake(Arc<RegionInfo>),
    /// The agent's main region changed (login, teleport, crossing).
    MainRegionChanged {
        handle: RegionHandle,
    },
    AgentMovementComplete {
        handle: RegionHandle,
        position: Vec3,
        look_at: Vec3,
    },
    /// SoundTrigger: a sound played once at a place (llTriggerSound,
    /// collisions, gestures).
    SoundTrigger {
        sound: Uuid,
        owner: Uuid,
        object: Uuid,
        parent: Uuid,
        handle: RegionHandle,
        /// Region-local position.
        position: Vec3,
        gain: f32,
    },
    /// AttachedSound: llPlaySound / llLoopSound on an object.
    AttachedSound {
        object: Uuid,
        sound: Uuid,
        owner: Uuid,
        gain: f32,
        flags: u8,
    },
    /// AttachedSoundGainChange (llAdjustSoundVolume).
    AttachedSoundGain {
        object: Uuid,
        gain: f32,
    },
    /// PreloadSound: sounds an object is about to play.
    PreloadSounds(Vec<Uuid>),
    /// An avatar's look-at target (ViewerEffect, LLHUDEffectLookAt): the
    /// target object and the offset from it, or a global position when the
    /// object is nil.
    LookAt {
        effect: Uuid,
        source: Uuid,
        target: Uuid,
        offset: [f64; 3],
        kind: u8,
        duration: f32,
    },
    /// TeleportLocal (a teleport inside the current region), sent just before
    /// its AgentMovementComplete: whether the simulator says we are flying.
    TeleportLocal {
        flying: bool,
    },
    Capabilities {
        handle: RegionHandle,
        caps: Arc<HashMap<String, String>>,
    },
    /// SimulatorFeatures of a region (voice server type: "webrtc" / "vivox",
    /// RenderMaterials limits read by LLViewerRegion::resetMaterialsCapThrottle
    /// and getMaxMaterialsPerTransaction; `None` = not given by the region).
    SimulatorFeatures {
        handle: RegionHandle,
        voice_server_type: String,
        /// "RenderMaterialsCapability": requests per second.
        materials_rate: Option<f32>,
        /// "MaxMaterialsPerTransaction": material ids per request.
        materials_max: Option<u32>,
    },
    ObjectUpdates {
        handle: RegionHandle,
        objects: Vec<ObjectUpdate>,
    },
    TerseUpdates {
        handle: RegionHandle,
        updates: Vec<TerseUpdate>,
    },
    ObjectsKilled {
        handle: RegionHandle,
        local_ids: Vec<u32>,
    },
    /// GLTF material overrides of one object (GenericStreamingMessage
    /// 0x4175, LLGLTFMaterialList::applyOverrideMessage): every message
    /// replaces all the object's overrides; `sides` = (face, `od` entry),
    /// empty clears them. Also replayed from the object cache on a hit.
    GltfOverrides {
        handle: RegionHandle,
        local_id: u32,
        sides: Vec<(u8, aurora_llsd::Llsd)>,
    },
    Terrain {
        handle: RegionHandle,
        patches: Vec<TerrainPatch>,
    },
    Chat(ChatMessage),
    InstantMessage(InstantMessage),
    Appearance(AvatarAppearance),
    Sun(SunInfo),
    CoarseLocations {
        handle: RegionHandle,
        positions: Vec<(Uuid, Vec3)>,
    },
    RegionRemoved {
        handle: RegionHandle,
    },
    TeleportStarted,
    /// TeleportProgress from the simulator (LL message key, e.g. "sending_dest").
    TeleportProgress {
        message: String,
    },
    /// TeleportFinish: the destination region is known, connecting to it.
    TeleportFinished {
        handle: RegionHandle,
    },
    TeleportFailed {
        reason: String,
    },
    Alert {
        message: String,
    },
    Disconnected {
        reason: String,
    },
    LoggedOut,
    /// Agent names resolved (id, first, last).
    Names(Vec<(Uuid, String, String)>),
    /// GetDisplayNames answer (People API): resolved names, ids it could not
    /// resolve, and the Cache-Control max-age of the reply (seconds).
    DisplayNames {
        names: Vec<AvatarNameData>,
        bad_ids: Vec<Uuid>,
        max_age: Option<u64>,
    },
    /// GetDisplayNames request failed (HTTP error): fall back to legacy names.
    DisplayNamesFailed(Vec<Uuid>),
    /// DisplayNameUpdate (event queue): an avatar changed its display name.
    DisplayNameUpdate {
        name: AvatarNameData,
        old_display_name: String,
    },
    /// Answer to our display name change (SetDisplayNameReply on the event
    /// queue, or the POST failing): HTTP-like status, reason, and `content`
    /// (display_name, error_tag, error_description).
    SetDisplayNameReply {
        status: i32,
        reason: String,
        content: aurora_llsd::Llsd,
    },
    FriendsOnline {
        ids: Vec<Uuid>,
        online: bool,
    },
    Balance(i32),
    /// `result["environment"]` from the ExtEnvironment capability (region
    /// or parcel; `environment["parcel_id"]` tells which).
    Environment {
        handle: RegionHandle,
        environment: aurora_llsd::Llsd,
    },
    /// llDialog / llTextBox menu from a script.
    ScriptDialog {
        object_id: Uuid,
        object_name: String,
        owner_name: String,
        message: String,
        channel: i32,
        buttons: Vec<String>,
    },
    /// A script asks for permissions (llRequestPermissions).
    ScriptQuestion {
        task_id: Uuid,
        item_id: Uuid,
        object_name: String,
        owner_name: String,
        questions: i32,
    },
    /// AvatarSitResponse: the seat's camera (llSetCameraEyeOffset /
    /// llSetCameraAtOffset, seat-relative) and llForceMouselook.
    SitResponse {
        object: Uuid,
        camera_eye: Vec3,
        camera_at: Vec3,
        force_mouselook: bool,
    },
    /// CameraConstraint: plane (normal, distance) between our head and
    /// the camera, region coordinates.
    CameraConstraint(glam::Vec4),
    /// llLoadURL.
    LoadUrl {
        object_name: String,
        object_id: Uuid,
        message: String,
        url: String,
    },
    /// Avatar profile data (AgentProfile / AvatarPropertiesReply).
    AvatarProfile(Box<crate::profile::AvatarProfile>),
    /// PickInfoReply.
    PickInfo(Box<crate::PickInfo>),
    /// AvatarClassifiedReply: an avatar's classifieds (id, name).
    AvatarClassifieds {
        target: Uuid,
        list: Vec<(Uuid, String)>,
    },
    /// ClassifiedInfoReply.
    ClassifiedInfo(Box<crate::ClassifiedInfo>),
    /// ParcelInfoReply (LLRemoteParcelInfoProcessor).
    ParcelInfo(Box<crate::ParcelSummary>),
    /// ChangeUserRights: `agent` changed the rights it grants to each
    /// (related avatar, rights); `agent` = us when we changed ours.
    UserRights {
        agent: Uuid,
        rights: Vec<(Uuid, i32)>,
    },
    /// Saving a profile field failed (AgentProfile PUT).
    ProfileSaveFailed {
        target: Uuid,
        reason: String,
    },
    /// A region's water height changed (RegionInfo).
    WaterHeight {
        handle: RegionHandle,
        height: f32,
    },
    /// A region's `REGION_FLAGS_*` and object capacity (SimStats, about
    /// once a second; LLViewerRegion::setRegionFlags / setMaxTasks).
    RegionFlags {
        handle: RegionHandle,
        flags: u32,
        max_tasks: u32,
    },
    /// Agent health in percent (HealthMessage, damage-enabled land).
    Health(f32),
    /// The parcel the agent stands on (main region).
    AgentParcel(Arc<ParcelInfo>),
    /// About Land: selected parcel, its lists, covenant... (land.rs).
    Land(crate::land::LandEvent),
    /// ParcelMediaCommandMessage: llParcelMediaCommandList (flags of the
    /// PARCEL_MEDIA_COMMAND_* bits, command, time).
    ParcelMediaCommand {
        flags: u32,
        command: u32,
        time: f32,
    },
    /// ParcelMediaUpdate: new media of the agent parcel (sent to one agent).
    ParcelMediaUpdate {
        url: String,
        media_id: Uuid,
        auto_scale: bool,
        mime: String,
        desc: String,
        width: i32,
        height: i32,
        looping: bool,
    },
    /// MapBlockReply: regions of the world map (`null_sims` when the reply
    /// answered a "return non-existent" request).
    MapBlocks {
        blocks: Vec<MapBlock>,
        null_sims: bool,
    },
    /// ParcelOverlay: quarter `sequence` (0..3) of a region's 4 m parcel
    /// grid, one byte per cell (ownership & 0x07, 0x40 west line, 0x80 south line).
    ParcelOverlay {
        handle: RegionHandle,
        sequence: i32,
        data: Vec<u8>,
    },
    /// ParcelProperties with a collision sequence id: the agent nears a
    /// parcel it may not enter (LLViewerParcelMgr::processParcelProperties).
    /// `kind`: 1 banned, 2 not in the group, 3 not on the access list;
    /// `bitmap`: one bit per 4 m cell of the region (x + y * cells).
    ParcelCollision {
        handle: RegionHandle,
        kind: u8,
        use_pass: bool,
        bitmap: Vec<u8>,
    },
    /// MapItemReply: telehubs, agent counts, land for sale, events...
    MapItems {
        item_type: u32,
        items: Vec<MapItem>,
    },
    /// Animations currently playing on an avatar (id, sequence).
    AvatarAnimations {
        avatar: Uuid,
        anims: Vec<(Uuid, i32)>,
    },
    InventoryContents(Vec<crate::inventory::FolderContents>),
    InventoryFetchFailed {
        folders: Vec<Uuid>,
    },
    /// Items fetched by id (FetchInventory2).
    InventoryItems(Vec<crate::inventory::InvItem>),
    InventoryItemsFailed {
        items: Vec<Uuid>,
    },
    /// Answer to a server appearance (bake) request: success, or the COF
    /// version the server expected.
    AppearanceRequestResult {
        cof_version: i32,
        success: bool,
        expected: Option<i32>,
    },
    /// ObjectProperties / ObjectPropertiesFamily (build tools: selection).
    ObjectProperties(Vec<crate::build::ObjectProps>),
    /// llSetPayPrice values for the clicked object.
    PayPrice {
        object: Uuid,
        default: i32,
        buttons: Vec<i32>,
    },
    /// Reply to BuildCmd::Cap (`Err` holds the HTTP / LLSD error).
    CapReply {
        tag: u64,
        result: Result<aurora_llsd::Llsd, String>,
    },
    /// An object's inventory: capability RequestTaskInventory, or
    /// RequestTaskInventory → ReplyTaskInventory → Xfer of the listing file
    /// (LLViewerObject::loadTaskInvLLSD / loadTaskInvFile). `serial`: the
    /// simulator's inventory serial when given. Items include the
    /// "Contents" root folder (`is_folder`).
    TaskInventory {
        object: Uuid,
        serial: Option<i16>,
        result: Result<Vec<crate::build::TaskItem>, String>,
    },
    /// ScriptRunningReply (and the event queue's Mono flag when given).
    ScriptRunning {
        object_id: Uuid,
        item_id: Uuid,
        running: bool,
        mono: Option<bool>,
    },
    /// ObjectPhysicsProperties (event queue): physics shape and material
    /// values of selected prims.
    PhysicsProperties(Vec<(u32, crate::build::PhysicsParams)>),
    /// ParcelProperties answering a ParcelPropertiesRequest of the build
    /// tools (sequence ≥ 1, LLViewerParcelMgr's selection), any region.
    SelectedParcel {
        handle: RegionHandle,
        sequence: i32,
        parcel: std::sync::Arc<ParcelInfo>,
    },
}

/// One avatar record of the People API (GetDisplayNames "agents" entries,
/// DisplayNameUpdate "agent"), as read by LLAvatarName::fromLLSD.
#[derive(Debug, Clone, Default)]
pub struct AvatarNameData {
    pub id: Uuid,
    /// Account name ("jane.doe", or "janedoe" for Resident names).
    pub username: String,
    pub display_name: String,
    pub legacy_first: String,
    pub legacy_last: String,
    /// No display name chosen: the display name is built from the username.
    pub is_default: bool,
    /// Unix time before which the display name cannot be changed again.
    pub next_update: f64,
}

impl AvatarNameData {
    pub fn from_llsd(v: &aurora_llsd::Llsd) -> AvatarNameData {
        let mut d = AvatarNameData {
            id: v["id"].as_uuid(),
            username: v["username"].to_string_value(),
            display_name: v["display_name"].to_string_value(),
            legacy_first: v["legacy_first_name"].to_string_value(),
            legacy_last: v["legacy_last_name"].to_string_value(),
            is_default: v["is_display_name_default"].as_bool(),
            next_update: v["display_name_next_update"].as_f64(),
        };
        // older People API replies name it "sl_id"
        if d.username.is_empty() {
            d.username = v["sl_id"].to_string_value();
        }
        // some avatars have no explicit display name: force a legible one
        if d.display_name.is_empty() {
            d.display_name = d.username.clone();
            d.is_default = true;
        }
        d
    }
}

/// One object to attach from the inventory (point 0 = where it was last worn).
#[derive(Debug, Clone)]
pub struct AttachRequest {
    pub item_id: Uuid,
    pub owner_id: Uuid,
    pub point: u8,
    /// Keep what is already on the point (ATTACHMENT_ADD).
    pub add: bool,
    pub flags: u32,
    pub group_mask: u32,
    pub everyone_mask: u32,
    pub next_owner_mask: u32,
    pub name: String,
    pub desc: String,
}

/// LLToolGrab SurfaceInfo: llDetectedTouch* values in region coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchSurface {
    pub uv: Vec3,
    pub st: Vec3,
    pub face: i32,
    pub position: Vec3,
    pub normal: Vec3,
    pub binormal: Vec3,
}
impl Default for TouchSurface {
    fn default() -> Self {
        Self {
            uv: Vec3::new(-1.0, -1.0, 0.0),
            st: Vec3::new(-1.0, -1.0, 0.0),
            face: -1,
            position: Vec3::ZERO,
            normal: Vec3::ZERO,
            binormal: Vec3::ZERO,
        }
    }
}
#[derive(Debug, Clone)]
pub enum NetCommand {
    /// Ask for the mute list (MuteListRequest; CRC of our cached copy, 0 if none).
    RequestMuteList {
        crc: u32,
    },
    /// Add / change a mute list entry (UpdateMuteListEntry; kind = LLMute::EType).
    UpdateMute {
        id: Uuid,
        name: String,
        kind: i32,
        flags: u32,
    },
    /// Remove a mute list entry (RemoveMuteListEntry).
    RemoveMute {
        id: Uuid,
        name: String,
    },
    /// Ask for our groups (AgentDataUpdateRequest).
    RequestGroups,
    /// Make a group active, nil for none (ActivateGroup).
    ActivateGroup(Uuid),
    /// Leave a group (LeaveGroupRequest; AgentDropGroup follows).
    LeaveGroup(Uuid),
    /// End a friendship (TerminateFriendship).
    TerminateFriendship(Uuid),
    /// Start / stop an animation on our avatar (AgentAnimation: away, busy...).
    AgentAnimation {
        anim: Uuid,
        start: bool,
    },
    /// Accept / decline a session invitation or fetch its history (ChatSessionRequest).
    ChatSession {
        method: crate::social::SessionMethod,
        session: Uuid,
    },
    Login(Box<LoginRequest>),
    Logout,
    Chat {
        message: String,
        channel: i32,
        chat_type: ChatType,
    },
    RequestObjects {
        handle: RegionHandle,
        local_ids: Vec<u32>,
    },
    TeleportHome,
    TeleportTo {
        handle: RegionHandle,
        position: Vec3,
        look_at: Vec3,
    },
    /// Change the draw distance used in AgentUpdate (`Far`).
    SetDrawDistance(f32),
    /// Walk / run toggle (SetAlwaysRun), used for double-tap running.
    SetAlwaysRun(bool),
    /// Our look-at target (ViewerEffect, LLHUDEffectLookAt::packData).
    LookAt {
        effect: Uuid,
        target: Uuid,
        offset: [f64; 3],
        kind: u8,
        duration: f32,
    },
    /// Raw IM with a given dialog (offer answers: accepted / declined...).
    SendImDialog {
        to: Uuid,
        dialog: u8,
        id: Uuid,
        message: String,
        bucket: Vec<u8>,
    },
    /// Accept a teleport offer (TeleportLureRequest, via lure).
    AcceptLure {
        lure_id: Uuid,
    },
    AcceptFriendship {
        transaction: Uuid,
        folder: Uuid,
    },
    DeclineFriendship {
        transaction: Uuid,
    },
    ScriptDialogReply {
        object_id: Uuid,
        channel: i32,
        index: i32,
        label: String,
    },
    /// Grant script permissions (ScriptAnswerYes; questions = granted bits).
    ScriptAnswer {
        task_id: Uuid,
        item_id: Uuid,
        questions: i32,
    },
    /// Touch an object (ObjectGrab + ObjectDeGrab), like a left click in SL.
    Touch {
        local_id: u32,
    },
    ObjectGrab {
        handle: RegionHandle,
        local_id: u32,
        offset: Vec3,
        surface: TouchSurface,
    },
    ObjectGrabUpdate {
        handle: RegionHandle,
        object: Uuid,
        offset: Vec3,
        position: Vec3,
        elapsed_ms: u32,
        surface: TouchSurface,
    },
    ObjectRelease {
        handle: RegionHandle,
        local_id: u32,
        surface: TouchSurface,
    },
    RequestTaskInventory {
        handle: RegionHandle,
        local_id: u32,
        object: Uuid,
    },
    /// Sit on an object (AgentRequestSit; `offset` in the object frame).
    RequestSit {
        handle: RegionHandle,
        target: Uuid,
        offset: Vec3,
    },
    RequestObjectProperties {
        handle: RegionHandle,
        object: Uuid,
    },
    RequestPayPrice {
        handle: RegionHandle,
        object: Uuid,
    },
    /// Sent only after the user confirms the displayed price (ObjectBuy).
    BuyObject {
        handle: RegionHandle,
        local_id: u32,
        folder: Uuid,
        sale_type: u8,
        price: i32,
    },
    /// Payment to the clicked prim, not directly to its owner.
    PayObject {
        handle: RegionHandle,
        object: Uuid,
        amount: i32,
        description: String,
    },
    /// Profile of an avatar (picture, about text): AgentProfile cap or AvatarPropertiesRequest.
    RequestProfile(Uuid),
    /// Details of a pick (generic "pickinforequest": creator, pick).
    PickInfoRequest {
        creator: Uuid,
        pick: Uuid,
    },
    /// Create or save one of our picks (PickInfoUpdate; the simulator sets
    /// the parcel).
    PickUpdate(Box<crate::PickInfo>),
    PickDelete(Uuid),
    /// An avatar's classifieds (generic "avatarclassifiedsrequest").
    ClassifiedsRequest(Uuid),
    ClassifiedInfoRequest(Uuid),
    ClassifiedDelete(Uuid),
    /// ParcelInfoRequest (name and region of a pick's parcel).
    ParcelInfoRequest(Uuid),
    /// About Land requests (land.rs).
    Land(crate::land::LandCommand),
    /// GrantUserRights: rights we give a friend (1 online, 2 map, 4 modify).
    GrantUserRights {
        friend: Uuid,
        rights: i32,
    },
    /// Save profile fields (saveAgentUserInfoCoro): PUT <AgentProfile>/<target>
    /// with a map of keys (`sl_about_text`, `fl_about_text`, `allow_publish`,
    /// `hide_age`: target = us; `notes`: target = the avatar noted).
    UpdateProfile {
        target: Uuid,
        data: aurora_llsd::Llsd,
    },
    /// Stand up, sit on ground, etc. are expressed as one-shot control flags.
    OneShotControl(u32),
    RequestNames(Vec<Uuid>),
    /// Display names through the GetDisplayNames capability of the main
    /// region (LLAvatarNameCache::requestNamesViaCapability).
    RequestDisplayNames(Vec<Uuid>),
    /// Change our display name (SetDisplayName capability; the People API
    /// wants the old and new values; "" resets to the username).
    SetDisplayName {
        old: String,
        new: String,
    },
    SendIm {
        to: Uuid,
        message: String,
    },
    OfferTeleport {
        to: Uuid,
        message: String,
    },
    TeleportLandmark(Uuid),
    /// Teleport to a region by name (map lookup), position in the region.
    TeleportToRegion {
        name: String,
        position: Vec3,
    },
    /// World map regions in a block of grid coordinates (region units,
    /// inclusive); `null_sims` asks for empty slots too (MAP_SIM_RETURN_NULL_SIMS).
    MapBlockRequest {
        min_x: u16,
        min_y: u16,
        max_x: u16,
        max_y: u16,
        null_sims: bool,
    },
    /// World map region search by name (MapNameRequest, LAYER_FLAG).
    MapNameRequest {
        name: String,
    },
    /// World map items of one type (MapItemRequest; handle 0 = whole grid).
    MapItemRequest {
        item_type: u32,
        handle: RegionHandle,
    },
    FetchInventory {
        folders: Vec<Uuid>,
        owner: Uuid,
        library: bool,
    },
    /// Fetch items by id (FetchInventory2).
    FetchItems {
        items: Vec<Uuid>,
        owner: Uuid,
    },
    /// Wear objects from the inventory (RezMultipleAttachmentsFromInv).
    RezAttachments(Vec<AttachRequest>),
    /// Ask the server to rebuild our appearance from the Current Outfit
    /// folder (UpdateAvatarAppearance capability).
    RequestServerAppearance {
        cof_version: i32,
    },
    /// Legacy wearables message the SL servers expect after login
    /// (LLAgentWearables::sendDummyAgentWearablesUpdate).
    DummyWearablesUpdate,
    /// Forget the in-memory object caches (the files are deleted by the viewer).
    ClearObjectCache,
    /// Build tools: selection, moves, new prims, links, terraforming.
    Build(crate::build::BuildCmd),
}

pub mod control {
    pub const AT_POS: u32 = 0x0000_0001;
    pub const AT_NEG: u32 = 0x0000_0002;
    pub const LEFT_POS: u32 = 0x0000_0004;
    pub const LEFT_NEG: u32 = 0x0000_0008;
    pub const UP_POS: u32 = 0x0000_0010;
    pub const UP_NEG: u32 = 0x0000_0020;
    pub const PITCH_POS: u32 = 0x0000_0040;
    pub const PITCH_NEG: u32 = 0x0000_0080;
    pub const YAW_POS: u32 = 0x0000_0100;
    pub const YAW_NEG: u32 = 0x0000_0200;
    pub const FAST_AT: u32 = 0x0000_0400;
    pub const FAST_LEFT: u32 = 0x0000_0800;
    pub const FAST_UP: u32 = 0x0000_1000;
    pub const FLY: u32 = 0x0000_2000;
    pub const STOP: u32 = 0x0000_4000;
    pub const FINISH_ANIM: u32 = 0x0000_8000;
    pub const STAND_UP: u32 = 0x0001_0000;
    pub const SIT_ON_GROUND: u32 = 0x0002_0000;
    pub const MOUSELOOK: u32 = 0x0004_0000;
    pub const NUDGE_AT_POS: u32 = 0x0008_0000;
    pub const NUDGE_AT_NEG: u32 = 0x0010_0000;
    pub const NUDGE_LEFT_POS: u32 = 0x0020_0000;
    pub const NUDGE_LEFT_NEG: u32 = 0x0040_0000;
    pub const NUDGE_UP_POS: u32 = 0x0080_0000;
    pub const NUDGE_UP_NEG: u32 = 0x0100_0000;
    pub const TURN_LEFT: u32 = 0x0200_0000;
    pub const TURN_RIGHT: u32 = 0x0400_0000;
    pub const AWAY: u32 = 0x0800_0000;
}

/// Agent state the viewer writes and the network layer sends at ~10 Hz.
#[derive(Debug, Clone, Copy)]
pub struct AgentControls {
    pub control_flags: u32,
    pub body_rotation: Quat,
    pub head_rotation: Quat,
    pub camera_center: Vec3,
    pub camera_at: Vec3,
    pub camera_left: Vec3,
    pub camera_up: Vec3,
    pub far: f32,
    pub state: u8,
    pub flags: u8,
}

impl Default for AgentControls {
    fn default() -> Self {
        Self {
            control_flags: 0,
            body_rotation: Quat::IDENTITY,
            head_rotation: Quat::IDENTITY,
            camera_center: Vec3::ZERO,
            camera_at: Vec3::X,
            camera_left: Vec3::Y,
            camera_up: Vec3::Z,
            far: 128.0,
            state: 0,
            flags: 0,
        }
    }
}

impl AgentControls {
    /// Whether the change since `prev` warrants an immediate update
    /// (mirrors the thresholds of `LLAgent::sendAgentUpdate`).
    pub fn significantly_differs(&self, prev: &AgentControls) -> bool {
        self.control_flags != prev.control_flags
            || self.state != prev.state
            || self.flags != prev.flags
            || self.body_rotation.dot(prev.body_rotation).abs() < 0.9999
            || self.head_rotation.dot(prev.head_rotation).abs() < 0.9999
            || self.camera_center.distance(prev.camera_center) > 0.1
            || self.camera_at.dot(prev.camera_at) < 0.999
            || (self.far - prev.far).abs() > 1.0
    }
}
