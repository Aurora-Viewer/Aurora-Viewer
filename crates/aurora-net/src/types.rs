//! Events emitted by the network layer and commands it accepts.

use crate::login::{LoginRequest, LoginResponse};
use crate::objects::{ObjectUpdate, TerseUpdate};
use crate::terrain::TerrainPatch;
use aurora_prim::TextureEntry;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Options of llMapDestination's ScriptTeleportRequest (indra_constants.h).
pub mod beacon {
    pub const SHOW_MAP: u32 = 0x1;
    pub const FOCUS_MAP: u32 = 0x2;
}

/// Region identifier: global origin in meters packed as `(x << 32) | y`.
pub type RegionHandle = u64;

/// Keep the source of each PreloadSound block through the viewer's filters.
#[derive(Debug, Clone, Copy)]
pub struct SoundPreload {
    pub sound: Uuid,
    pub object: Uuid,
    pub owner: Uuid,
}

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
    /// RegionHandshake BillableFactor: land here is charged as its area
    /// times this (LLViewerRegion::getBillableFactor).
    pub billable_factor: f32,
}

/// Parcel description from ParcelProperties (About Land).
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// The parcel's cells: one bit per 4 m cell of the region, x + y * cells
    /// (Bitmap; LLViewerParcelMgr keeps the agent parcel's as
    /// mAgentParcelOverlay for inAgentParcel).
    pub bitmap: Vec<u8>,
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
    /// Parcel grid units of the requested rectangle owned by the agent, by
    /// others, and public (SelfCount / OtherCount / PublicCount:
    /// LLParcelSelection's mSelected*Count).
    pub self_count: i32,
    pub other_count: i32,
    pub public_count: i32,
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
    pub const BLOCK_TERRAFORM: u32 = 1 << 6;
    pub const BLOCK_LAND_RESELL: u32 = 1 << 7;
    pub const SKIP_SCRIPTS: u32 = 1 << 13;
    pub const BLOCK_FLY: u32 = 1 << 19;
    pub const ESTATE_SKIP_SCRIPTS: u32 = 1 << 21;
    pub const RESTRICT_PUSHOBJECT: u32 = 1 << 22;
    pub const ALLOW_PARCEL_CHANGES: u32 = 1 << 26;
    pub const BLOCK_FLYOVER: u32 = 1 << 27;
    pub const ALLOW_VOICE: u32 = 1 << 28;
    pub const BLOCK_PARCEL_SEARCH: u32 = 1 << 29;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatType {
    Whisper,
    Normal,
    Shout,
    /// CHAT_TYPE_START / STOP: an empty line telling those in range that we
    /// type in the nearby chat, or stopped (LLAgent::startTyping).
    StartTyping,
    StopTyping,
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
            4 => ChatType::StartTyping,
            5 => ChatType::StopTyping,
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
            ChatType::StartTyping => 4,
            ChatType::StopTyping => 5,
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

#[derive(Debug, Clone, PartialEq)]
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
    /// When it was sent (Unix time); 0 when the simulator gave none.
    pub timestamp: u32,
}

#[derive(Debug, Clone)]
pub struct AvatarAppearance {
    pub avatar_id: Uuid,
    pub texture_entry: Option<Arc<TextureEntry>>,
    pub visual_params: Vec<u8>,
    pub cof_version: i32,
    /// AppearanceData.AppearanceVersion: 1 for server bakes, 0 without
    /// the block.
    pub appearance_version: u8,
    pub hover_height: f32,
    /// Worn objects the simulator lists (AttachmentBlock entries with an
    /// id, each once: LLVOAvatar::mSimAttachments).
    pub attachments: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct SunInfo {
    pub sun_direction: Vec3,
    pub sun_phase: f32,
    pub sec_per_day: u32,
    pub usec_since_start: u64,
}

/// One block of MeanCollisionAlert: who bumped, pushed or hit us.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeanCollision {
    pub perp: Uuid,
    /// Unix time of the collision, from the simulator.
    pub time: u32,
    pub magnitude: f32,
    /// EMeanCollisionType (mean_collision_data.h): 1 bump, 2 llPushObject,
    /// 3 selected object, 4 scripted object, 5 physical object.
    pub kind: u8,
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
    /// A resident offers their calling card (process_offer_callingcard):
    /// who, the transaction, and the region whose simulator gets the answer.
    CallingCardOffered {
        from: Uuid,
        transaction: Uuid,
        handle: RegionHandle,
    },
    /// Our calling card was accepted or declined (AcceptCallingCard /
    /// DeclineCallingCard from the simulator).
    CallingCardAnswered {
        accepted: bool,
    },
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
    /// The ReadOfflineMsgs capability failed: Firestorm then asks by
    /// message (`NetCommand::RequestOfflineMessagesLegacy`).
    OfflineMessagesFailed,
    /// An offline group invitation was accepted by capability: the group
    /// list is to ask again.
    OfflineGroupInviteAccepted(Uuid),
    /// Answer of the ChatSessionRequest capability (None = failed;
    /// `not_found`: the server answered 404, the session is gone).
    ChatSessionReply {
        method: crate::social::SessionMethod,
        session: Uuid,
        result: Option<aurora_llsd::Llsd>,
        not_found: bool,
    },
    /// Someone calls us (ChatterBoxInvitation with "voice").
    VoiceInvite(crate::social::VoiceInvite),
    /// Answer of a voice call request: the body, or the HTTP status of the
    /// failure (0 = no capability / no reply).
    CallReply {
        method: crate::social::CallMethod,
        session: Uuid,
        result: Result<aurora_llsd::Llsd, u16>,
    },
    /// New voice channel of a session (ChatterBoxSessionUpdate with
    /// "voice_channel_info"): a call in progress moves to it.
    SessionVoiceChannel {
        session: Uuid,
        info: aurora_llsd::Llsd,
    },
    LoginProgress {
        message: String,
        fraction: f32,
    },
    /// `bad_credentials`: refused for the user name / password (reason "key").
    /// `previous_session`: the grid is still logging out a previous session
    /// of this account (reason "presence", LoginError::previous_session).
    LoginFailed {
        error: String,
        mfa_required: bool,
        tos_required: bool,
        bad_credentials: bool,
        previous_session: bool,
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
    PreloadSounds(Vec<SoundPreload>),
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
    /// Selection/grab target driving LLEditingMotion (LLHUDEffectPointAt).
    PointAt {
        effect: Uuid,
        source: Uuid,
        target: Uuid,
        offset: [f64; 3],
        kind: u8,
        duration: f32,
    },
    /// An avatar asks the others to reset its skeleton (ViewerEffect,
    /// LLHUDEffectResetSkeleton::unpackData): `source` is the sender, `target`
    /// the object to reset (the sender itself when the packet names none).
    ResetSkeleton {
        source: Uuid,
        target: Uuid,
        animations: bool,
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
        avatar_hover_height: bool,
        /// "RenderMaterialsCapability": requests per second.
        materials_rate: Option<f32>,
        /// "MaxMaterialsPerTransaction": material ids per request.
        materials_max: Option<u32>,
        /// "LSLSyntaxId": the LSL keywords in use (lsl_syntax.rs); nil
        /// when the region names none.
        lsl_syntax_id: Uuid,
        /// "OpenSimExtras" "GridStatusRSS" and "GridStatus": an OpenSim
        /// grid's status feed and page (Help › Grid status); empty when
        /// not given, always on Second Life.
        grid_status_feed: String,
        grid_status_page: String,
    },
    HoverHeightUpdated {
        region: RegionHandle,
        request: u64,
        success: bool,
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
    Wind {
        handle: RegionHandle,
        grid: Box<crate::terrain::WindGrid>,
    },
    Chat(ChatMessage),
    /// An avatar in earshot starts or stops typing in the nearby chat: the
    /// empty START_TYPING / STOP_TYPING lines its viewer sends
    /// (process_chat_from_simulator → LLVOAvatar::startTyping / stopTyping),
    /// whether or not it plays the typing animation.
    NearbyTyping {
        avatar: Uuid,
        typing: bool,
    },
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
        flying: bool,
    },
    /// A named destination lookup completed. The viewer rechecks the pending
    /// action before issuing TeleportTo; resolving a name never teleports.
    TeleportRegionResolved {
        request: Uuid,
        result: Result<RegionHandle, String>,
    },
    /// The answer to `SetMaturityPreference`: the preference the server
    /// kept, or None when the request failed.
    MaturityPreference(Option<u8>),
    /// The HomeLocation capability's answer to `SetHomeLocation`.
    HomeLocation(crate::home::Reply),
    /// A simulator alert about the home location (home.rs), shown in the
    /// viewer's words.
    HomeAlert(crate::home::Alert),
    /// The account's directory settings (UserInfo capability or
    /// UserInfoReply), or why they could not be read.
    UserInfo(Result<crate::user_info::UserInfo, crate::user_info::Error>),
    /// `UpdateUserInfo` did not go through.
    UserInfoUpdateFailed(crate::user_info::Error),
    /// Help › Server release notes: the page to open, or why there is none
    /// (`NetCommand::ServerReleaseNotes`, release_notes.rs).
    ServerReleaseNotes(Result<String, String>),
    Pathfinding {
        request: Uuid,
        region: RegionHandle,
        kind: crate::pathfinding::Kind,
        result: Result<Vec<crate::pathfinding::Row>, crate::pathfinding::Error>,
    },
    /// The region console's output (`SimConsoleResponse`), or Err when the
    /// command did not reach the simulator (sim_console.rs).
    SimConsoleReply(Result<String, String>),
    /// The LSL keywords of a syntax id (`NetCommand::FetchLslSyntax`).
    LslSyntax {
        id: Uuid,
        keywords: Result<Vec<crate::lsl_syntax::Keyword>, String>,
    },
    /// A nearby voice moderation request failed (voice_moderation.rs);
    /// `everyone` for mute / unmute all.
    VoiceModerationFailed {
        everyone: bool,
    },
    TeleportFailed {
        reason: String,
    },
    Alert {
        message: String,
    },
    /// The agent's region will restart (AlertInfo RegionRestartMinutes /
    /// RegionRestartSeconds, or OpenSim's `/RESTART_X_…`; region_restart.rs).
    RegionRestart(crate::region_restart::Notice),
    /// "Region restart cancelled." (region_restart.rs).
    RegionRestartCancelled,
    /// ScriptControlChange: controls scripts take or release, each block
    /// (take, controls, pass on to the avatar), for
    /// `script_controls::ScriptControls::apply`.
    ScriptControlChange(Vec<(bool, u32, bool)>),
    Disconnected {
        reason: String,
        /// The link was lost (silence, event queue, region gone) rather
        /// than ended by the grid (kicked): offering to reconnect makes sense.
        lost: bool,
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
    /// The outcome of a transaction, from a MoneyBalanceReply whose
    /// TransactionInfo names one (TransactionType 0 is a plain balance
    /// update). Sent before the new balance.
    MoneyTransaction {
        /// MoneyData TransactionID: the simulator may send a reply twice.
        id: Uuid,
        success: bool,
        kind: i32,
        source: Uuid,
        source_group: bool,
        dest: Uuid,
        dest_group: bool,
        amount: i32,
        /// TransactionInfo ItemDescription: the item sold, or the message
        /// of a gift ("Payment" when none).
        item: String,
        description: String,
    },
    /// MoneyBalanceReply's land: m² the account may hold (tier) and m² it
    /// holds (LLStatusBar::setLandCredit / setLandCommitted).
    LandHoldings {
        credit: i32,
        committed: i32,
    },
    /// Answer of the land helper to `LandToolPreflight`, for the billable
    /// area and the L$ asked (so a stale answer can be told apart).
    LandToolPreflight {
        billable_area: i32,
        currency_buy: i32,
        result: Result<crate::land_tool::LandToolInfo, crate::currency::CurrencyError>,
    },
    /// Outcome of `LandToolBuy`: the website side of a land purchase is done
    /// (membership, land use, L$); the ParcelBuy is sent next.
    LandToolPrepared(Result<(), crate::currency::CurrencyError>),
    /// Answer of the currency helper to `CurrencyQuote`; `amount` is the one
    /// asked, so an estimate for a figure typed over can be dropped.
    CurrencyQuote {
        amount: i32,
        result: Result<crate::currency::CurrencyQuote, crate::currency::CurrencyError>,
    },
    /// Outcome of `BuyCurrency`. The new balance follows later, pushed by
    /// the simulator once the purchase is processed.
    CurrencyBought(Result<(), crate::currency::CurrencyError>),
    /// `result["environment"]` from the ExtEnvironment capability (region
    /// or parcel; `environment["parcel_id"]` tells which). `handle` and
    /// `parcel_id` are what was requested (-1: the region), so the viewer
    /// can drop answers for a region or parcel the agent has left.
    Environment {
        handle: RegionHandle,
        parcel_id: i32,
        environment: aurora_llsd::Llsd,
    },
    /// llDialog / llTextBox menu from a script.
    ScriptDialog {
        /// Exact simulator endpoint retained by process_script_dialog.
        sender: std::net::SocketAddr,
        object_id: Uuid,
        object_name: String,
        owner_name: String,
        message: String,
        channel: i32,
        buttons: Vec<String>,
    },
    /// A script asks for permissions (llRequestPermissions), possibly on
    /// behalf of an experience (llRequestExperiencePermissions; nil if not).
    /// `region`: the simulator that asked, which gets the answer
    /// (process_script_question keeps the sender).
    ScriptQuestion {
        task_id: Uuid,
        item_id: Uuid,
        object_name: String,
        owner_name: String,
        questions: i32,
        experience: Uuid,
        region: RegionHandle,
    },
    /// GetExperienceInfo: the details of the experiences asked for (unknown
    /// ones when they could not be read).
    ExperienceInfo(Vec<crate::experience::ExperienceDetails>),
    /// ExperienceEvent's permission-4 event, with the experience in its invoice.
    ExperienceAttachment {
        experience: Uuid,
        object_name: String,
    },
    /// GroupExperiences: the experiences a group owns (empty on a failure).
    GroupExperiences {
        group: Uuid,
        ids: Vec<Uuid>,
    },
    /// ExperiencePreferences: the agent's permission for an experience after
    /// reading or changing it.
    ExperiencePermission {
        experience: Uuid,
        change: crate::experience::ExperiencePermissionChange,
        result: Result<crate::experience::ExperiencePermission, String>,
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
    /// llSetCameraParams: wire property IDs (llfollowcamparams.h) and values.
    /// Position / focus components are relative to the sending region.
    SetFollowCam {
        handle: RegionHandle,
        object: Uuid,
        properties: Vec<(i32, f32)>,
    },
    /// llClearCameraParams: remove this source, exposing the previous one.
    ClearFollowCam {
        object: Uuid,
    },
    /// llLoadURL.
    LoadUrl {
        object_name: String,
        object_id: Uuid,
        message: String,
        url: String,
    },
    /// llMapDestination (ScriptTeleportRequest): a place to show on the
    /// world map. Despite its name it never teleports by itself.
    ScriptTeleportRequest {
        object_name: String,
        region: String,
        position: Vec3,
        /// `beacon::SHOW_MAP` / `beacon::FOCUS_MAP`.
        flags: u32,
    },
    /// MeanCollisionAlert: residents who bumped us, pushed us or hit us
    /// with an object (process_mean_collision_alert_message).
    MeanCollisions(Vec<MeanCollision>),
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
    /// The answer of SearchStatRequest for one of our classifieds.
    ClassifiedStats {
        id: Uuid,
        stats: crate::profile::ClassifiedStats,
    },
    /// ParcelInfoReply (LLRemoteParcelInfoProcessor).
    ParcelInfo(Box<crate::ParcelSummary>),
    /// One reply packet of a directory search (Dir*Reply).
    DirReply(Box<crate::search::DirReply>),
    /// EventInfoReply (LLEventInfo).
    EventInfo(Box<crate::search::EventInfo>),
    /// GroupProfileReply (LLGroupMgr::processGroupPropertiesReply).
    GroupProfile(Box<crate::search::GroupProfile>),
    /// Members of a group: one GroupMembersReply packet, or the whole list
    /// of the GroupMemberData capability.
    GroupMembers(Box<crate::groups::MembersPage>),
    /// Roles of a group (GroupRoleDataReply, one packet).
    GroupRoles(Box<crate::groups::RolesPage>),
    /// Role / member pairs of a group (GroupRoleMembersReply, one packet).
    GroupRoleMembers(Box<crate::groups::RoleMembersPage>),
    /// The titles we can wear in a group (GroupTitlesReply).
    GroupTitles {
        group: Uuid,
        request: Uuid,
        titles: Vec<crate::groups::GroupTitle>,
    },
    /// The notice list of a group (GroupNoticesListReply, one packet).
    GroupNotices {
        group: Uuid,
        notices: Vec<crate::groups::NoticeSummary>,
    },
    /// JoinGroupReply / LeaveGroupReply.
    GroupJoined {
        group: Uuid,
        success: bool,
    },
    GroupLeft {
        group: Uuid,
        success: bool,
    },
    /// CreateGroupReply: the new group, or the server's reason for refusing
    /// it (LLGroupMgr::processCreateGroupReply).
    GroupCreated {
        group: Uuid,
        success: bool,
        message: String,
    },
    /// EjectGroupMemberReply.
    GroupEjected {
        group: Uuid,
        success: bool,
    },
    /// The ban list of a group (GroupAPIv1): residents and ban dates.
    GroupBanList {
        group: Uuid,
        bans: Vec<(Uuid, f64)>,
    },
    /// The group's parcels and land totals (PlacesReply), one packet.
    GroupLand(Box<crate::group_money::GroupLandReply>),
    /// A page of the group's accounts (GroupAccount…Reply), one packet.
    GroupAccount(Box<crate::group_money::AccountReply>),
    /// Land types of the grid, (ProductSKU, description) (ProductInfoRequest).
    ProductInfo(Vec<(String, String)>),
    /// RegionIDAndHandleReply (LLLandmark::processRegionIDAndHandle).
    RegionIdHandle {
        region_id: Uuid,
        handle: RegionHandle,
    },
    /// Answer to NetCommand::RemoteParcelRequest (same handle and position).
    RemoteParcel {
        handle: RegionHandle,
        position: Vec3,
        result: Result<Uuid, crate::land::RemoteParcelError>,
    },
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
    /// A parcel owner or an estate manager froze the agent, or released it
    /// (ViewerFrozenMessage).
    Frozen(bool),
    /// Who froze or released the agent (AvatarFrozen… alerts, freeze.rs).
    FreezeAlert(crate::freeze::Alert),
    /// The parcel the agent stands on (main region).
    AgentParcel(Arc<ParcelInfo>),
    /// A pushed spatial voice change; other local parcel state stays intact.
    ParcelVoiceInfo {
        handle: RegionHandle,
        local_id: i32,
        allowed: bool,
        use_estate: Option<bool>,
    },
    /// About Land: selected parcel, its lists, covenant... (land.rs).
    Land(crate::land::LandEvent),
    /// Region / Estate window (estate.rs).
    Estate(crate::estate::EstateEvent),
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
    /// FindAgent answered: where a friend who lets us see them on the map
    /// is (global metres, no height; LLAvatarTracker::processAgentFound).
    AgentFound {
        prey: Uuid,
        x: f64,
        y: f64,
    },
    /// Animations currently playing on an avatar (id, sequence).
    AvatarAnimations {
        avatar: Uuid,
        anims: Vec<(Uuid, i32)>,
        /// AnimationSourceList, aligned with the first entries of `anims`.
        /// Empty for ObjectAnimation (animesh), which has no source blocks.
        sources: Vec<Uuid>,
    },
    InventoryContents(Vec<crate::inventory::FolderContents>),
    /// Unsolicited server changes, distinct from snapshots fetched by the client.
    InventoryUpdated {
        folders: Vec<crate::inventory::InvFolder>,
        items: Vec<crate::inventory::InvItem>,
    },
    /// Delivery identity follows InventoryUpdated, so observers see its headers.
    InventoryDelivery {
        transaction: Uuid,
        folders: Vec<Uuid>,
    },
    InventoryMoved {
        id: Uuid,
        parent: Uuid,
        folder: bool,
    },
    InventoryProgress {
        request: Uuid,
        progress: crate::inventory::operations::Progress,
    },
    InventoryEdited {
        request: Uuid,
        result: Result<crate::inventory::operations::Reply, String>,
    },
    AssetUploaded {
        request: Uuid,
        result: Result<crate::inventory::upload::Receipt, crate::inventory::upload::Error>,
    },
    UploadCost(i32),
    MeshQuoted {
        request: Uuid,
        result: Result<crate::inventory::mesh_upload::Quote, crate::inventory::upload::Error>,
    },
    WearableSaved {
        request: Uuid,
        result: Result<crate::inventory::InvItem, String>,
    },
    InventoryPreview {
        item: Uuid,
        asset: Uuid,
        result: Result<Vec<u8>, String>,
    },
    GestureLoaded {
        item: Uuid,
        asset: Uuid,
        result: Result<aurora_assets::gesture::Gesture, String>,
    },
    TaskNotecardPreview {
        object: Uuid,
        item: Uuid,
        asset: Uuid,
        result: Result<Vec<u8>, String>,
    },
    TaskNotecardSaved {
        request: Uuid,
        object: Uuid,
        item: Uuid,
        result: Result<Uuid, String>,
    },
    ScriptLoaded {
        request: Uuid,
        target: crate::scripts::ScriptTarget,
        result: Result<String, String>,
    },
    ScriptExperiences {
        request: Uuid,
        target: crate::scripts::ScriptTarget,
        result: crate::scripts::experiences::ScriptExperiences,
    },
    ScriptSaved {
        request: Uuid,
        target: crate::scripts::ScriptTarget,
        result: Result<crate::scripts::ScriptSaved, String>,
    },
    BridgeReply {
        request: Uuid,
        result: Result<crate::bridge::Ack, String>,
    },
    BridgeQueryReply {
        request: Uuid,
        result: Result<crate::bridge::query::Reply, String>,
    },
    InventoryThumbnailSource {
        request: Uuid,
        asset: Uuid,
        resize: bool,
        result: Result<Vec<u8>, String>,
    },
    InventoryOperationFailed(String),
    NotecardCopySubmitted {
        editor: Uuid,
        notecard: Uuid,
        result: Result<(), String>,
    },
    NotecardInventoryCopied {
        editor: Uuid,
        notecard: Uuid,
        open: bool,
        result: Result<crate::inventory::InvItem, String>,
    },
    /// The new item of a `NetCommand::CopyInventoryItem`, as the server
    /// created it, or why it never arrived.
    InventoryItemCopied {
        request: Uuid,
        result: Result<crate::inventory::InvItem, String>,
    },
    InventoryMerchant(Result<bool, String>),
    /// The merchant status of the "Marketplace listings" window
    /// (getMerchantStatusCoro).
    MerchantStatus(crate::slm::MerchantStatus),
    /// The answer of an SLM API request.
    Slm {
        request: crate::slm::SlmRequest,
        reply: crate::slm::SlmReply,
    },
    InventoryContentSaved {
        request: Uuid,
        item: Uuid,
        result: Result<crate::inventory::InvItem, String>,
    },
    /// SaveAssetIntoInventory confirms that a detached object's asset is saved.
    AttachmentAssetSaved(Uuid),
    OutfitUpdated {
        request: Uuid,
        result: Result<Vec<crate::inventory::FolderContents>, String>,
    },
    InventoryFavoriteUpdated {
        item: Uuid,
        result: Result<crate::inventory::InvItem, String>,
    },
    /// Reply to `NetCommand::FetchInventoryTree`: the folder snapshots it
    /// brought, parents first, or why the read failed.
    InventoryTreeFetched {
        request: u64,
        result: Result<Vec<crate::inventory::FolderContents>, String>,
    },
    InventoryFetchFailed {
        folders: Vec<Uuid>,
    },
    /// Items fetched by id (FetchInventory2).
    InventoryItems(Vec<crate::inventory::InvItem>),
    /// A locally requested creation, confirmed by its UDP callback.
    InventoryCreated(crate::inventory::InvItem),
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
    /// Answer to `IncrementCofVersion`: the folder's new version, None when
    /// the region has no such capability or every attempt failed.
    CofVersionIncremented(Option<i32>),
    /// ObjectProperties / ObjectPropertiesFamily (build tools: selection).
    ObjectProperties(Vec<crate::build::ObjectProps>),
    /// The region's abuse categories: (value, localized label).
    AbuseCategories(Vec<(i32, String)>),
    /// Outcome of `SendAbuseReport` (`Err`: the text to show).
    AbuseReportSent(Result<(), String>),
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
    /// ParcelProperties of the parcel under the pointer
    /// (HOVERED_PARCEL_SEQ_ID: the world's hover tips), any region.
    HoverParcel {
        handle: RegionHandle,
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
    /// Send the current language and visibility through its capability.
    UpdateAgentLanguage,
    /// LLVOAvatarSelf::sendHoverHeight: final value, scoped to its region.
    SetHoverHeight {
        region: RegionHandle,
        request: u64,
        value: f32,
    },
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
    /// Ask for the instant messages kept while we were offline, once after
    /// login (LLIMProcessing::requestOfflineMessages: the ReadOfflineMsgs
    /// capability, else RetrieveInstantMessages).
    RequestOfflineMessages,
    /// The same by the RetrieveInstantMessages message only (after
    /// `NetEvent::OfflineMessagesFailed`).
    RequestOfflineMessagesLegacy,
    /// Answer a friendship offer received while offline: by the
    /// AcceptFriendship / DeclineFriendship capability, else by message when
    /// it came with a transaction id.
    AnswerOfflineFriendship {
        from: Uuid,
        transaction: Uuid,
        /// Calling cards folder (the message's FolderData).
        folder: Uuid,
        accept: bool,
        /// Our legacy full name, sent with the acceptance.
        agent_name: String,
    },
    /// Answer a group invitation received while offline without a session
    /// id (AcceptGroupInvite / DeclineGroupInvite capability).
    AnswerOfflineGroupInvite {
        group: Uuid,
        accept: bool,
    },
    /// Make a group active, nil for none (ActivateGroup).
    ActivateGroup(Uuid),
    /// Leave a group (LeaveGroupRequest; AgentDropGroup follows).
    LeaveGroup(Uuid),
    /// Receive the notices of a group or not, and list it in our profile or
    /// not (SetGroupAcceptNotices, LLAgent::setUserGroupFlags): both flags
    /// travel together.
    SetGroupFlags {
        group: Uuid,
        accept_notices: bool,
        list_in_profile: bool,
    },
    /// End a friendship (TerminateFriendship).
    TerminateFriendship(Uuid),
    /// Start / stop an animation on our avatar (AgentAnimation: away, busy...).
    AgentAnimation {
        anim: Uuid,
        start: bool,
    },
    /// Start / stop several animations in one AgentAnimation
    /// (LLAgent::sendAnimationRequests); nothing is sent without any.
    AgentAnimations {
        anims: Vec<Uuid>,
        start: bool,
    },
    /// One AO transition, with old stops and new starts in the same message.
    AgentAnimationChanges {
        changes: Vec<(Uuid, bool)>,
    },
    /// Clear the region's animation state overrides of our avatar
    /// (LLAgent::sendAnimationStateReset: a nil animation, stopped).
    AnimationStateReset,
    /// Revoke script permissions in the scripts of an object, or of the whole
    /// region when `target` is its id (LLAgent::sendRevokePermissions).
    RevokePermissions {
        target: Uuid,
        permissions: u32,
    },
    /// Accept / decline a session invitation or fetch its history (ChatSessionRequest).
    ChatSession {
        method: crate::social::SessionMethod,
        session: Uuid,
    },
    /// Voice call request on ChatSessionRequest (credentials, P2P start /
    /// decline); `server_type` is the region's VoiceServerType.
    Call {
        method: crate::social::CallMethod,
        session: Uuid,
        server_type: String,
    },
    Login(Box<LoginRequest>),
    Logout,
    Chat {
        message: String,
        channel: i32,
        chat_type: ChatType,
    },
    /// One bounded script-protocol answer, including an empty answer.
    /// Unlike user chat this must never be split or redirected.
    ChatReply {
        message: String,
        channel: i32,
    },
    /// A protocol IM response, after query/consent checks rather than user IM policy.
    ImReply {
        to: Uuid,
        message: String,
        busy: bool,
        offline: bool,
    },
    RequestObjects {
        handle: RegionHandle,
        local_ids: Vec<u32>,
    },
    TeleportHome,
    /// The content rating the agent wants (UpdateAgentInformation,
    /// LLAgent::sendMaturityPreferenceToServer); answered by
    /// `NetEvent::MaturityPreference`.
    SetMaturityPreference(u8),
    /// World › Set Home to Here (LLAgent::setStartPosition): the agent's
    /// feet in its region and where it faces, through the HomeLocation
    /// capability (answered by `NetEvent::HomeLocation`) or
    /// SetStartLocationRequest without it.
    SetHomeLocation {
        position: Vec3,
        look_at: Vec3,
    },
    /// The account's directory settings (LLAgent::sendAgentUserInfoRequest),
    /// answered by `NetEvent::UserInfo`.
    RequestUserInfo,
    /// LLAgent::sendAgentUpdateUserInfo: "hidden" or "default", and the
    /// e-mail flag off Second Life only; a failure answers
    /// `NetEvent::UserInfoUpdateFailed`.
    UpdateUserInfo {
        directory_visibility: String,
        im_via_email: Option<bool>,
    },
    /// Fetch the page of the region's ServerReleaseNotes capability,
    /// answered by `NetEvent::ServerReleaseNotes`.
    ServerReleaseNotes,
    FetchPathfinding {
        request: Uuid,
        region: RegionHandle,
        kind: crate::pathfinding::Kind,
    },
    /// One command line for the region console (SimConsoleAsync); the
    /// output comes as `NetEvent::SimConsoleReply`.
    SimConsole(String),
    /// The LSL keywords of the syntax `id`, from the cache folder or the
    /// region's LSLSyntax capability; answered by `NetEvent::LslSyntax`.
    FetchLslSyntax {
        id: Uuid,
        cache_dir: std::path::PathBuf,
    },
    /// Mute or unmute in the region's spatial voice
    /// (SpatialVoiceModerationRequest).
    ModerateVoice {
        target: crate::voice_moderation::Target,
        mute: bool,
    },
    TeleportTo {
        handle: RegionHandle,
        position: Vec3,
        look_at: Vec3,
    },
    /// Change the draw distance used in AgentUpdate (`Far`).
    SetDrawDistance(f32),
    /// No connection to the neighbouring regions while true
    /// (FSDisableNeighbourRegionConnections): the viewer sends false while
    /// the agent is outside its region, so that a crossing still connects.
    RefuseNeighbourRegions(bool),
    /// The camera's vertical viewing angle (radians) changed: AgentFOV now,
    /// and again at each arrival (LLViewerCamera::setView).
    SetFov(f32),
    /// Controls that changed enough to be sent now (`NetClient::set_controls`).
    /// Every change of the control flags comes through here, in order: a
    /// key tapped between two ticks of the session is not lost.
    AgentControls(AgentControls),
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
    /// Our selection/grab target (LLHUDEffectPointAt::packData).
    PointAt {
        effect: Uuid,
        target: Uuid,
        offset: [f64; 3],
        kind: u8,
        duration: f32,
    },
    /// Ask the other viewers to reset our skeleton (ViewerEffect,
    /// LLHUDEffectResetSkeleton::packData), and our animations with it.
    ResetSkeleton {
        effect: Uuid,
        target: Uuid,
        animations: bool,
    },
    /// Raw IM with a given dialog (offer answers: accepted / declined...).
    SendImDialog {
        to: Uuid,
        dialog: u8,
        id: Uuid,
        message: String,
        bucket: Vec<u8>,
    },
    /// Accept a teleport offer (TeleportLureRequest, LLAgent::doTeleportViaLure):
    /// a god's lure cannot be cancelled.
    AcceptLure {
        lure_id: Uuid,
        godlike: bool,
    },
    AcceptFriendship {
        transaction: Uuid,
        folder: Uuid,
    },
    DeclineFriendship {
        transaction: Uuid,
    },
    /// Give our calling card to an avatar, through the simulator of the
    /// region they are in (LLAvatarGiveCard: OfferCallingCard).
    OfferCallingCard {
        handle: RegionHandle,
        dest: Uuid,
        transaction: Uuid,
    },
    /// Answer a calling card offer, to the simulator that brought it
    /// (callingcard_offer_callback): accepted into `folder` (the Calling
    /// Cards folder), or declined (`None`).
    AnswerCallingCard {
        handle: RegionHandle,
        transaction: Uuid,
        folder: Option<Uuid>,
    },
    ScriptDialogReply {
        /// Reply to the menu's sender (callback_script_dialog). None is
        /// reserved for viewer-originated avatar chat, e.g. bridge discovery.
        sender: Option<std::net::SocketAddr>,
        object_id: Uuid,
        channel: i32,
        index: i32,
        label: String,
    },
    /// Grant script permissions (ScriptAnswerYes; questions = granted bits),
    /// sent to the simulator that asked (`region`).
    ScriptAnswer {
        task_id: Uuid,
        item_id: Uuid,
        questions: i32,
        region: RegionHandle,
    },
    /// GetExperienceInfo for these experiences.
    ExperienceInfo(Vec<Uuid>),
    /// The experiences a group owns (GroupExperiences capability), answered
    /// by NetEvent::GroupExperiences.
    GroupExperiences(Uuid),
    /// ExperiencePreferences: read, allow, block or forget an experience.
    ExperiencePermission {
        experience: Uuid,
        change: crate::experience::ExperiencePermissionChange,
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
    /// RequestObjectPropertiesFamily with COMPLAINT_REPORT_REQUEST, for the
    /// object an abuse report is about (LLFloaterReporter::getObjectInfo).
    RequestReportObjectProperties {
        handle: RegionHandle,
        object: Uuid,
    },
    /// The abuse categories in this language (AbuseCategories); answered by
    /// `AbuseCategories`, nothing when the region has no such capability.
    RequestAbuseCategories {
        language: String,
    },
    /// Send an abuse report (sendReportViaCaps, else UserReport after the
    /// screenshot's upload); answered by `AbuseReportSent`.
    SendAbuseReport(Box<crate::abuse_report::AbuseReport>),
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
    /// L$ given to a resident or a group (give_money with TRANS_GIFT):
    /// `message` is the optional payment message, empty for none.
    PayResident {
        to: Uuid,
        group: bool,
        amount: i32,
        message: String,
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
    /// Create or save one of our classifieds (ClassifiedInfoUpdate,
    /// LLPanelProfileClassified::sendUpdate): a new one is paid
    /// `price` L$ by the simulator.
    ClassifiedUpdate(Box<crate::ClassifiedInfo>),
    /// The clicks on one of our classifieds (SearchStatRequest capability).
    ClassifiedStatsRequest(Uuid),
    /// A click to report (SearchStatTracking capability).
    ClassifiedClick {
        kind: crate::profile::ClassifiedClick,
        classified: Box<crate::ClassifiedInfo>,
    },
    /// ParcelInfoRequest (name and region of a pick's parcel, place details).
    ParcelInfoRequest(Uuid),
    /// A directory search (search.rs): `id` names the query in its replies,
    /// `start` is the first row of the page asked.
    DirQuery {
        id: Uuid,
        start: i32,
        query: crate::search::DirQuery,
    },
    /// EventInfoRequest: the details of an event found.
    EventInfoRequest(u32),
    /// GroupProfileRequest: the details of a group found.
    GroupProfileRequest(Uuid),
    /// The members of a group (LLGroupMgr::sendCapGroupMembersRequest: the
    /// GroupMemberData capability, GroupMembersRequest without it).
    GroupMembersRequest {
        group: Uuid,
        request: Uuid,
    },
    /// GroupRoleDataRequest / GroupRoleMembersRequest / GroupTitlesRequest.
    GroupRoleDataRequest {
        group: Uuid,
        request: Uuid,
    },
    GroupRoleMembersRequest {
        group: Uuid,
        request: Uuid,
    },
    GroupTitlesRequest {
        group: Uuid,
        request: Uuid,
    },
    /// Wear the title of one of our roles (GroupTitleUpdate).
    GroupTitleUpdate {
        group: Uuid,
        role: Uuid,
    },
    /// GroupNoticesListRequest.
    GroupNoticesListRequest(Uuid),
    /// EjectGroupMemberRequest (LLGroupMgr::sendGroupMemberEjects).
    EjectGroupMembers {
        group: Uuid,
        members: Vec<Uuid>,
    },
    /// GroupRoleChanges (sendGroupRoleMemberChanges).
    GroupRoleChanges {
        group: Uuid,
        changes: Vec<crate::groups::RoleChange>,
    },
    /// The GroupAPIv1 capability: a POST of a ban action (create, delete),
    /// or the ban list asked (`action` None, a GET).
    GroupBans {
        group: Uuid,
        action: Option<i32>,
        ids: Vec<Uuid>,
        /// BAN_UPDATE: the list asked again after the action.
        refresh: bool,
    },
    /// GroupRoleUpdate (LLGroupMgrGroupData::sendRoleChanges).
    GroupRoleUpdate {
        group: Uuid,
        updates: Vec<crate::groups::RoleUpdate>,
    },
    /// UpdateGroupInfo (LLGroupMgr::sendUpdateGroupInfo).
    UpdateGroupInfo {
        group: Uuid,
        info: crate::groups::GroupInfo,
    },
    /// CreateGroupRequest (LLGroupMgr::sendCreateGroupRequest), answered by
    /// NetEvent::GroupCreated. The simulator charges the creation fee.
    CreateGroup {
        name: String,
        info: crate::groups::GroupInfo,
    },
    /// GroupNoticeRequest: one notice, answered by an IM_GROUP_NOTICE_REQUESTED
    /// instant message.
    GroupNoticeRequest(Uuid),
    /// The parcels the group owns (PlacesQuery, DFQ_GROUP_OWNED), answered
    /// by NetEvent::GroupLand. A nil group requests the agent's personal
    /// holdings with DFQ_AGENT_OWNED (LLFloaterLandHoldings).
    GroupLandQuery {
        group: Uuid,
        transaction: Uuid,
    },
    /// SetGroupContribution: our land given to the group, in m².
    SetGroupContribution {
        group: Uuid,
        contribution: i32,
    },
    /// A page of the group's accounts (GroupAccountSummaryRequest,
    /// GroupAccountDetailsRequest, GroupAccountTransactionsRequest),
    /// answered by NetEvent::GroupAccount.
    GroupAccount(crate::group_money::AccountRequest),
    /// Join a group (JoinGroupRequest).
    JoinGroup(Uuid),
    /// Invite residents into a group, each into a role (InviteGroupRequest,
    /// (invitee, role) pairs).
    InviteToGroup {
        group: Uuid,
        invites: Vec<(Uuid, Uuid)>,
    },
    /// The land types of the grid (ProductInfoRequest capability).
    ProductInfoRequest,
    /// The parcel at a point of any region, by the RemoteParcelRequest
    /// capability of the agent's region (LLPanelPlaceInfo::displayParcelInfo
    /// for a place link): `handle` of the 256 m slot holding the point,
    /// `position` in region coordinates, `region_id` when known (a
    /// landmark's). Answered by NetEvent::RemoteParcel.
    RemoteParcelRequest {
        handle: RegionHandle,
        position: Vec3,
        region_id: Uuid,
    },
    /// RegionHandleRequest: the handle of a region known by its id
    /// (LLLandmark::requestRegionHandle). Answered by NetEvent::RegionIdHandle.
    RegionHandleRequest(Uuid),
    /// About Land requests (land.rs).
    Land(crate::land::LandCommand),
    /// Region / Estate window (estate.rs).
    Estate(crate::estate::EstateCommand),
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
    /// StartLure: one message for every invitee (send_lures).
    OfferTeleport {
        to: Vec<Uuid>,
        message: String,
    },
    TeleportLandmark(Uuid),
    /// Cancel the restart of the agent's region (estate managers):
    /// EstateOwnerMessage `restart` with -1, as Region / Estate › Debug ›
    /// "Cancel restart" (LLPanelRegionDebugInfo::onClickCancelRestart).
    CancelRegionRestart,
    /// Teleport to a region by name (map lookup), position in the region.
    TeleportToRegion {
        request: Uuid,
        name: String,
        position: Vec3,
        look_at: Vec3,
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
    /// The avatar the map follows, nil to stop (TrackAgent,
    /// LLAvatarTracker::track / untrack).
    TrackAgent(Uuid),
    /// Ask where a friend is (FindAgent, LLAvatarTracker::findAgent);
    /// answered with `NetEvent::AgentFound` when they let us map them.
    FindAgent(Uuid),
    /// The background inventory fetch (LLInventoryModelBackgroundFetch):
    /// `parent` with its whole subtree, or with `children` only those of its
    /// folders, `depth` levels down (AIS). Without AIS, the folders alone.
    FetchInventoryTree {
        request: u64,
        parent: Uuid,
        children: Vec<Uuid>,
        depth: i32,
        owner: Uuid,
        library: bool,
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
    EditInventory {
        request: Uuid,
        change: crate::inventory::operations::Mutation,
    },
    CopyInventoryItems(Vec<crate::inventory::operations::CopyItem>),
    /// One item copy whose new item is reported by `InventoryItemCopied`
    /// (copy_inventory_item with an LLInventoryCallback). A nil `parent`
    /// lets the server choose the default folder of the item's type.
    CopyInventoryItem {
        request: Uuid,
        copy: crate::inventory::operations::CopyItem,
    },
    /// Track a mutation's deferred item copies while its folders remain visible.
    FinishInventoryCopies {
        request: Uuid,
        reply: crate::inventory::operations::Reply,
    },
    CreateInventoryItem {
        parent: Uuid,
        kind: crate::inventory::operations::NewItem,
        name: String,
        permissions: Option<crate::inventory::upload::Permissions>,
    },
    PreviewInventoryItem(crate::inventory::InvItem),
    PreviewTaskNotecard {
        handle: RegionHandle,
        object: Uuid,
        item: Box<crate::inventory::InvItem>,
    },
    SaveTaskNotecard {
        request: Uuid,
        handle: RegionHandle,
        object: Uuid,
        item: Uuid,
        data: Vec<u8>,
    },
    LoadScript {
        request: Uuid,
        target: crate::scripts::ScriptTarget,
        handle: RegionHandle,
        owner: Uuid,
        asset: Uuid,
    },
    LoadScriptExperiences {
        request: Uuid,
        target: crate::scripts::ScriptTarget,
        handle: RegionHandle,
    },
    SaveScript {
        request: Uuid,
        target: crate::scripts::ScriptTarget,
        handle: RegionHandle,
        options: crate::scripts::ScriptOptions,
        source: String,
    },
    Bridge {
        request: Uuid,
        command: crate::bridge::Command,
    },
    BridgeQuery {
        request: Uuid,
        endpoint: crate::bridge::Endpoint,
        query: crate::bridge::query::Query,
    },
    FetchInventoryThumbnail {
        request: Uuid,
        asset: Uuid,
        resize: bool,
    },
    UploadInventoryThumbnail {
        request: Uuid,
        item: Uuid,
        folder: bool,
        parent: Uuid,
        data: Vec<u8>,
    },
    UploadAsset {
        request: Uuid,
        upload: crate::inventory::upload::Request,
    },
    QuoteMesh {
        request: Uuid,
        upload: crate::inventory::mesh_upload::Request,
    },
    UploadMesh {
        request: Uuid,
        token: Uuid,
    },
    /// Release a fee offer after its draft changes or is removed.
    DiscardMeshQuote(Uuid),
    RequestBalance,
    /// Estimate for buying `amount` L$ (XML-RPC getCurrencyQuote on the
    /// grid's helper page, `currency::helper_uri`).
    CurrencyQuote {
        helper: String,
        amount: i32,
    },
    /// Buy the L$ of an estimate (XML-RPC buyCurrency); `password` is empty
    /// unless the helper asked for a "password" confirmation.
    BuyCurrency {
        helper: String,
        quote: crate::currency::CurrencyQuote,
        password: crate::currency::Password,
    },
    /// What buying land needs on the website side (XML-RPC
    /// preflightBuyLandPrep on `<helper>landtool.php`).
    LandToolPreflight {
        helper: String,
        billable_area: i32,
        currency_buy: i32,
    },
    /// Do that website side (XML-RPC buyLandPrep), before the ParcelBuy.
    LandToolBuy {
        helper: String,
        request: crate::land_tool::LandToolBuy,
    },
    RequestInventoryMerchant,
    /// GET /merchant for the "Marketplace listings" window
    /// (LLMarketplaceData::initializeSLM).
    RequestMerchantStatus,
    /// One SLM API request (LLMarketplaceData's coroutines).
    Slm(crate::slm::SlmRequest),
    SaveWearable {
        request: Uuid,
        item: Box<crate::inventory::InvItem>,
        data: Vec<u8>,
        copy_name: Option<String>,
    },
    CreateInventoryWearable {
        parent: Uuid,
        kind: u8,
        name: String,
        data: Vec<u8>,
        permissions: Option<crate::inventory::upload::Permissions>,
    },
    RestoreInventoryObject(crate::inventory::InvItem),
    SaveInventoryContent {
        request: Uuid,
        item: Uuid,
        asset_type: i32,
        data: Vec<u8>,
    },
    /// Create a named notecard, then upload its text after the matching
    /// inventory callback. Uses UpdateNotecardAgentInventory, without a fee.
    CreateNotecardItem {
        request: Uuid,
        parent: Uuid,
        name: String,
        data: Vec<u8>,
        permissions: Option<crate::inventory::upload::Permissions>,
    },
    /// A new environment settings item holding `data` (LLSettingsVOBase::
    /// createInventoryItem): CreateInventoryItem, then the asset through
    /// UpdateSettingsAgentInventory. Answered by `InventoryContentSaved`.
    CreateSettingsItem {
        request: Uuid,
        parent: Uuid,
        name: String,
        /// LLSettingsType: 0 sky, 1 water, 2 day cycle.
        subtype: u8,
        /// The edited item's next owner permissions (Save As); None: the
        /// defaults for new settings.
        next_owner_mask: Option<u32>,
        data: Vec<u8>,
    },
    LoadGesture {
        item: Uuid,
        asset: Uuid,
    },
    SetGestureActive {
        item: Uuid,
        asset: Uuid,
        active: bool,
    },
    /// SoundTrigger at our position in the main region (send_sound_trigger):
    /// gesture sounds and a sound item double-clicked in the inventory.
    GestureSound {
        asset: Uuid,
        position: Vec3,
    },
    CopyNotecardInventory {
        editor: Uuid,
        open: bool,
        handle: Option<RegionHandle>,
        object: Uuid,
        notecard: Uuid,
        item: Box<crate::inventory::InvItem>,
    },
    /// AIS link changes, acknowledged and re-fetched before updating appearance.
    UpdateOutfit {
        request: Uuid,
        change: crate::outfits::OutfitMutation,
    },
    UpdateOutfitCategory {
        request: Uuid,
        change: crate::outfits::categories::Mutation,
    },
    DetachAttachments(Vec<Uuid>),
    /// Detach an attachment without an inventory item, in its simulator.
    DetachObject {
        handle: RegionHandle,
        local_id: u32,
    },
    SetInventoryFavorite {
        item: Uuid,
        favorite: bool,
    },
    /// Ask the server to rebuild our appearance from the Current Outfit
    /// folder (UpdateAvatarAppearance capability).
    RequestServerAppearance {
        cof_version: i32,
    },
    /// Rebake: ask the server to step our Current Outfit folder's version
    /// (IncrementCOFVersion capability, LLAppearanceMgr::
    /// syncCofVersionAndRefresh), answered by `CofVersionIncremented`.
    IncrementCofVersion,
    /// Have the simulator send an avatar's appearance again (generic
    /// "avatartexturesrequest", LLAvatarPropertiesProcessor::
    /// sendAvatarTexturesRequest).
    AvatarTexturesRequest(Uuid),
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
    /// Left button outside mouselook, and in mouselook when no script took
    /// its mouselook bit (LLViewerInput::scanMouse, LLToolGrabBase).
    pub const LBUTTON_DOWN: u32 = 0x1000_0000;
    pub const LBUTTON_UP: u32 = 0x2000_0000;
    /// Left button in mouselook, taken by a script (LLToolCompGun).
    pub const ML_LBUTTON_DOWN: u32 = 0x4000_0000;
    pub const ML_LBUTTON_UP: u32 = 0x8000_0000;
}

/// The typing bit of AgentUpdate's State (AGENT_STATE_TYPING, llagent.cpp).
pub const AGENT_STATE_TYPING: u8 = 0x04;

/// Agent state the viewer writes each frame and the network layer sends by
/// the rule of `agent_update.rs`.
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
    /// The render state (LLAgent::getRenderState): `AGENT_STATE_TYPING`
    /// while we type in the nearby chat.
    pub state: u8,
    pub flags: u8,
    /// Alt or Ctrl held: the left-button bits go out as releases
    /// (script_controls::mask_left_button).
    pub alt_or_ctrl: bool,
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
            alt_or_ctrl: false,
        }
    }
}
