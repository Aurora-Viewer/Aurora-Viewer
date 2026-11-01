//! Viewer-side world model fed by network events.

pub mod animation_log;
pub mod anti_spam;
pub mod appearance;
pub mod attachment_inventory;
pub mod attention;
pub mod blocking;
pub mod body;
pub mod bubble_chat;
pub mod calling_cards;

pub mod beacons;
pub mod bridge;
pub mod calls;
pub mod chat_log;
pub mod collisions;
pub mod contact_sets;
pub mod derender;
pub mod edit_echo;
pub mod eep;
pub mod eep_env;
pub mod eep_photo;
pub mod env;
pub mod env_apply;
pub mod env_day_edit;
pub mod env_edit;
pub mod env_select;
pub mod estate;
pub mod experiences;
pub mod favorites;
pub mod flying_at_exit;
mod friend_join;
pub mod group_data;
pub mod group_money;
pub mod groups;
pub mod hand_ik;
pub mod home;
pub mod hover_height;
pub mod hud_lock;
pub mod hud_scale;
pub mod inventory;
pub mod inventory_offers;
pub mod land;
pub mod land_buy;
pub mod land_moderation;
pub mod landmarks;
pub mod lookat;
pub mod lsl_syntax;
pub mod lslbridge;
pub mod marketplace;
pub mod maturity;
pub mod motion;
pub mod mutes;
pub mod names;
pub mod night_sky;
pub mod notice_topics;
pub mod notification_history;
pub mod notifications;
pub mod object_im_filter;
pub mod objects;
pub mod open_ims;
pub mod outfit;
pub mod outfit_folders;
pub mod parcel_visibility;
pub mod parked;
pub mod particle_filters;
pub mod payments;
pub mod place_details;
pub mod place_snapshots;
pub mod pointat;
pub mod portals;
pub mod poser_consent;
pub mod profiles;
pub mod radar;
pub mod received_items;
pub mod recent;
pub mod region_alerts;
pub mod region_messages;
pub mod region_restart;
pub mod rig_graph;
pub mod rig_targets;
pub mod rlva;
pub mod script_debug;
pub mod search;
pub mod sim_console;
pub mod social;
pub mod sound_parcels;
pub mod staff;
pub mod status;
pub mod teleport_invites;
pub mod temp_attachments;
pub mod terrain;
pub mod tphistory;
pub mod transcripts;
pub mod user_info;
pub mod voice_moderation;
pub mod windlight_presets;
pub mod worldmap;

#[cfg(test)]
mod automated_chat_tests;
#[cfg(test)]
mod flash_tests;
#[cfg(test)]
mod group_notice_tests;
#[cfg(test)]
mod toasts_tests;

use crate::agent::AgentState;
use crate::scene::avatar::AvatarLibrary;
use crate::scene::textures::TexSource;
use crate::ui_sound::{ImKind, ImMessage, UiSound};
use aurora_net::{
    AvatarAppearance, ChatMessage, ChatSourceType, ChatType, InstantMessage, LoginResponse, NetEvent, RegionHandle, RegionInfo, SunInfo,
};
use glam::Vec3;
use objects::{ObjKey, Object, ObjectStore};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use terrain::Heightmap;
use uuid::Uuid;

pub struct Region {
    pub handle: RegionHandle,
    pub name: String,
    pub info: Option<Arc<RegionInfo>>,
    pub heightmap: Heightmap,
    pub wind: aurora_net::terrain::WindGrid,
    pub dirty_chunks: HashSet<u32>,
    /// Terrain patches received so far: what the grass standing on it was
    /// placed for (scene/plants.rs).
    pub terrain_revision: u64,
    pub terrain_surface: terrain::SurfaceState,
    pub caps: Arc<HashMap<String, String>>,
    /// Voice server of the region (SimulatorFeatures): "webrtc", "vivox" or empty.
    pub voice_server: String,
    pub avatar_hover_height: bool,
    /// SimulatorFeatures' LSLSyntaxId once they came (nil: none named).
    pub lsl_syntax_id: Option<Uuid>,
    /// RenderMaterials limits of the region (SimulatorFeatures).
    pub materials_limits: crate::scene::legacy_mat::RegionLimits,
    /// Object capacity (SimStats ObjectCapacity, LLViewerRegion::getMaxTasks);
    /// 0 until known.
    pub max_tasks: u32,
    /// An OpenSim grid's status feed and page (SimulatorFeatures'
    /// OpenSimExtras GridStatusRSS, GridStatus); empty when not given.
    pub grid_status: (String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    Local(ChatType),
    /// Object chat with its type (normal, owner say, debug = script error, direct).
    Object(ChatType),
    System,
    Im,
    /// Instant message sent by an object (llInstantMessage).
    ObjectIm,
    /// Ours, with the volume of a nearby line (Normal elsewhere).
    Own(ChatType),
    /// The card of a teleport offer or request (`teleport_invites.rs`),
    /// by its invitation: never written to the transcript.
    Invite(u64),
    /// The card of a resident's inventory offer (`inventory_offers.rs`), by
    /// its offer: never written to the transcript either.
    InventoryOffer(u64),
    /// The card of a resident's direct payment to us (`payments.rs`), by
    /// its payment: its text is written to the transcript by itself.
    Payment(u64),
}

impl ChatKind {
    /// One of our lines that is not a nearby one (an IM, a group's), or
    /// one said at the normal volume.
    pub const OWN: ChatKind = ChatKind::Own(ChatType::Normal);

    /// One of our own lines, whatever its volume.
    pub fn is_own(self) -> bool {
        matches!(self, ChatKind::Own(_))
    }

    /// How loud a resident's or object's line was said, ours included (LLChat's
    /// mChatType): whispered and shouted lines read the same whoever said
    /// them (FSChatHistory::appendMessage).
    pub fn volume(self) -> Option<ChatType> {
        match self {
            ChatKind::Local(t) | ChatKind::Own(t) | ChatKind::Object(t) => Some(t),
            _ => None,
        }
    }

    /// A notice of the viewer rather than someone's message: it stands
    /// apart from the groups of messages.
    pub fn is_notice(self) -> bool {
        matches!(
            self,
            ChatKind::System | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_)
        )
    }

    /// A card (an invitation, an offer with their buttons, a payment).
    pub fn is_card(self) -> bool {
        matches!(self, ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_))
    }

    /// Not sent by a person: what an object or a script says (its owner-say
    /// and its llRegionSayTo to us included), its IMs, and the viewer's
    /// notices. The cards of residents' offers are people's (#741).
    pub fn is_automated(self) -> bool {
        matches!(self, ChatKind::Object(_) | ChatKind::ObjectIm | ChatKind::System)
    }
}

/// Where a line comes from: said during this session, or read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Origin {
    #[default]
    Live,
    /// The end of the conversation's transcript on disk, shown when it
    /// opens (Firestorm's CHAT_STYLE_HISTORY).
    Log,
    /// The chat server's recent messages of a group
    /// (CHAT_STYLE_SERVER_HISTORY).
    ServerLog,
}

#[derive(Debug, Clone)]
pub struct ChatLine {
    pub time: SystemTime,
    pub from: String,
    pub text: String,
    pub kind: ChatKind,
    pub source: Uuid,
    pub origin: Origin,
}

/// Drawn speed (m/s) under which our walk stops after a release, with
/// Aurora's movement (`World::update_own_locomotion`).
const GLIDE_WALKING: f32 = 0.5;

/// First known simulator signal, independent of visibility and asset loading.
/// Sequence changes and uninterrupted loops keep separate receipt clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayingAnimation {
    pub id: Uuid,
    pub sequence: i32,
    pub sequence_start: Instant,
    /// Preserved while the UUID remains signaled, reset after an observed stop.
    pub continuous_start: Instant,
}

impl PlayingAnimation {
    pub(crate) fn from_signal(id: Uuid, sequence: i32, now: Instant, previous: Option<&Self>) -> Self {
        Self {
            id,
            sequence,
            sequence_start: previous.filter(|p| p.sequence == sequence).map_or(now, |p| p.sequence_start),
            continuous_start: previous.map_or(now, |p| p.continuous_start),
        }
    }
}

/// The Local tab of the texture picker (LLFloaterTexturePicker::onBtnAdd,
/// onBtnRemove, onBtnUpload, indra/newview/lltexturectrl.cpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAssetRequest {
    /// Choose files to preview locally: images, or glTF materials.
    Add {
        materials: bool,
    },
    Remove(Uuid),
    /// Open the importer on this local file, to upload it.
    Upload(Uuid),
}

pub struct World {
    /// mScreen's Y scale (LLVOAvatarSelf::updateCharacter).
    pub hud_aspect: f32,
    /// Where the HUDs are laid out in the window (left, top, right, bottom
    /// pixels; `None`: the whole window). `hud_aspect` is its aspect.
    pub hud_area: Option<[f32; 4]>,
    /// HUD zoom is local to the current session (LLAgentCamera).
    pub hud_zoom: f32,
    pub hud_scales: hud_scale::HudScales,
    pub hud_locks: hud_lock::HudLocks,
    /// Names of our temporary attachments (temp_attachments.rs).
    pub temp_attachment_names: temp_attachments::TempAttachmentNames,
    pub hover_height: hover_height::HoverHeight,
    /// The favourites in the account's order (`favorites.rs`).
    pub favorites: favorites::Favorites,
    /// What left the notification list, kept per account (the center's
    /// History).
    pub notification_history: notification_history::History,
    pub login: Option<Arc<LoginResponse>>,
    pub agent_id: Uuid,
    pub regions: HashMap<RegionHandle, Region>,
    pub main_region: Option<RegionHandle>,
    pub objects: ObjectStore,
    /// Session-only local contacts from our own seated vehicle.
    pub hand_contacts: hand_ik::Contacts,
    pub ik_targets: rig_targets::Targets,
    pub(crate) scripted_camera: crate::camera::follow::Sources,
    /// When the clocks of the regions were last advanced (`advance_motion`).
    motion_tick: Instant,
    /// GLTF material overrides by object, (face, override) pairs. Kept apart
    /// from the objects like LL's region override cache: a message may come
    /// before its object, and the scene merges them when it builds the faces.
    pub gltf_overrides: HashMap<ObjKey, Arc<[(u8, aurora_assets::material::PbrOverride)]>>,
    pub chat: VecDeque<ChatLine>,
    /// What each avatar said lately, for the bubbles above its head.
    pub bubble_chat: bubble_chat::BubbleChat,
    /// Follows `chat` for the view.
    pub chat_clock: transcripts::LineClock,
    pub chat_unread: usize,
    /// A resident said something in nearby chat since the app last looked:
    /// it may flash the Conversations button (FSNotifyNearbyChatFlash).
    pub nearby_news: bool,
    /// What the nearby chat's entry counts as unread: `chat_unread`, or only
    /// the people's lines while `quiet_automated_chat` is on.
    pub chat_badge: usize,
    /// Lines not sent by a person neither count as unread nor alert
    /// (Preferences › Chat, #741, off), given every frame by the app.
    pub quiet_automated_chat: bool,
    /// What the notices of the nearby chat are about, for their icon.
    pub notice_topics: notice_topics::Topics,
    /// Conversation transcripts of the account logged in.
    pub chat_log: chat_log::ChatLog,
    /// History of the nearby chat (asked once per login).
    nearby_history: transcripts::HistoryState,
    pub script_debug: script_debug::Messages,
    pub sun: Option<(SunInfo, Instant)>,
    pub agent: AgentState,
    /// Session-only UI readback of the active avatar follow.
    pub following_avatar: Option<Uuid>,
    /// The controls our scripts took (llTakeControls): who gets the left
    /// button's clicks (app/script_clicks.rs).
    pub script_controls: aurora_net::script_controls::ScriptControls,
    pub coarse: HashMap<RegionHandle, Vec<(Uuid, Vec3)>>,
    /// The agent's region before its last change, until the arrival.
    region_left: Option<RegionHandle>,
    /// Objects of the regions out of reach, until their simulator closes
    /// them or they are in reach again (parked.rs).
    parked: HashMap<RegionHandle, parked::Parked>,
    appearances: HashMap<Uuid, (AvatarAppearance, u64)>,
    appearance_counter: u64,
    /// Bakes the scene swapped in, by avatar (`refresh_bakes`).
    bake_refreshes: HashMap<Uuid, u64>,
    shape_preview: Option<Vec<u8>>,
    preview_bakes: HashMap<usize, Uuid>,
    shape_preview_generation: u64,
    pub alerts: VecDeque<String>,
    /// The announced restart of the agent's region (region_restart.rs).
    pub region_restart: region_restart::Tracker,
    pub movement_complete: bool,
    /// AURORA_DEMO_OUTFIT_RESTORE: the demo region answers the outfit
    /// restore, which then starts without the inventory capability.
    pub demo_outfit_region: bool,
    pub avatar_lib: Arc<AvatarLibrary>,
    /// Animated joint matrices (skeleton space, rest -> posed) of the avatars
    /// near the camera, so attachments follow their bone.
    pub avatar_poses: HashMap<Uuid, Vec<glam::Mat4>>,
    pub avatar_models: HashMap<Uuid, glam::Mat4>,
    /// Root offset from body size, shape hover and mesh pelvis fixups.
    /// A ground animesh entry holds only its mesh pelvis fixup.
    /// The adjustable hover height stays live, independent of pose timing.
    pub avatar_shape_dz: HashMap<Uuid, f32>,
    /// Local water-wave heave of our drawn avatar, independent of physics/hover.
    pub water_float: crate::movement::water_float::Follower,
    /// Drawn body orientation per standing avatar (LLVOAvatar::updateOrientation).
    pub bodies: HashMap<Uuid, body::Body>,
    /// Where avatars look (ours and the ones others send).
    pub look_at: lookat::LookAts,
    pub point_at: pointat::PointAts,
    /// Object messages from elsewhere (Preferences › Privacy, issue #623).
    pub object_im_filter: object_im_filter::Settings,
    pub anti_spam: anti_spam::Registry,
    pub teleporting: bool,
    /// Only a teleport to another region needs a loading screen. Unknown
    /// destinations wait for TeleportFinish to identify the destination.
    pub tp_show_progress: bool,
    /// Teleport progress (0..0.6 until arrival) and its step, failure flag.
    pub tp_progress: f32,
    pub tp_status: String,
    pub tp_failed: bool,
    /// Outfit restore at login (Current Outfit folder).
    pub outfit: outfit::OutfitRestore,
    /// Objects already reported by a diagnostic (logged once).
    diag_reported: HashSet<Uuid>,
    pub events_this_frame: usize,
    /// The login answer's `message` (LLAgent::mMOTD), trimmed: Contents ›
    /// Message of the day writes it again to the nearby chat.
    pub motd: String,
    pub balance: Option<i32>,
    /// m² of land the account holds (MoneyBalanceReply SquareMetersCommitted,
    /// LLStatusBar::getSquareMetersCommitted).
    pub land_committed: i32,
    /// m² the account may hold (SquareMetersCredit): what is left is
    /// LLStatusBar::getSquareMetersLeft.
    pub land_credit: i32,
    pub upload_cost: Option<i32>,
    pub local_assets: HashMap<Uuid, (String, bool)>,
    /// The texture picker's Local tab buttons (drained by the app's importer).
    pub local_asset_requests: Vec<LocalAssetRequest>,
    /// ExtEnvironment answers of the frame (parcel local id, error), for the
    /// environment editors' "Apply To Parcel / Region" (drained by the app).
    pub environment_answers: Vec<(i32, Result<(), String>)>,
    /// Health in percent (HealthMessage; only changes on damage-enabled land).
    pub health: f32,
    /// A parcel owner or an estate manager froze us (ViewerFrozenMessage):
    /// shown on the navigation bar while it lasts.
    pub frozen: bool,
    /// What the simulator killed while the photo mode held the world still
    /// (`freeze_motion`): each object's key and id, removed on thaw.
    held_kills: Vec<(ObjKey, Uuid)>,
    /// Interface sounds to play (drained by the app).
    pub ui_sounds: Vec<UiSound>,
    /// Conversation messages received (their sound depends on the IM modes).
    pub im_messages: Vec<ImMessage>,
    /// Taskbar flashes and system notifications asked for (drained by the
    /// app, `app/attention.rs`).
    pub attention: Vec<attention::Attention>,
    /// L$ balance changes (previous, new), for the money sounds.
    pub balance_changes: Vec<(Option<i32>, i32)>,
    /// Avatars that just started typing (typing sound, LLVOAvatar::startMotion).
    pub typing_started: Vec<Uuid>,
    /// Avatars whose viewer said they type in the nearby chat
    /// (`NetEvent::NearbyTyping`, LLVOAvatar::mTyping), until their stop,
    /// their line, or their leaving.
    nearby_typing: HashSet<Uuid>,
    pub social: social::Social,
    /// Contact sets and aliases (LGGContactSets), per account.
    pub contact_sets: contact_sets::ContactSets,
    /// The residents we dealt with lately (People › Recent).
    pub recent: recent::RecentPeople,
    /// Contact sets generation whose aliases the name cache has.
    aliases_synced: u64,
    /// Block list (LLMuteList).
    pub mutes: mutes::MuteList,
    pub poser_consent: poser_consent::State,
    pub portals: portals::State,
    /// Firestorm LSL bridge messages kept out of nearby chat.
    pub lsl_bridge: lslbridge::LslBridge,
    pub rlva: rlva::Rlva,
    pub rlva_privacy: rlva::privacy::TextFilter,
    pub rlva_hover_privacy: rlva::privacy::TextFilter,
    rlva_location_hidden: bool,
    /// Derendered objects and residents, and the asset blacklist.
    pub derender: derender::Derender,
    /// Our groups and the group / conference chat sessions.
    pub groups: groups::GroupChats,
    /// Voice calls with residents, groups and conferences.
    pub calls: calls::Calls,
    /// Online status (away, do not disturb...) and automatic responses.
    pub status: status::Status,
    pub inventory: inventory::Inventory,
    pub active_gestures: std::collections::BTreeSet<Uuid>,
    /// The inventory cache being read by its thread since the login reply
    /// (`poll_inventory_cache`), and when the read started.
    inventory_cache: Option<(crossbeam_channel::Receiver<Option<inventory::CachedInventory>>, Instant)>,
    /// The whole inventory, fetched in the background from login on.
    inventory_background: inventory::background::BackgroundFetch,
    /// Inventory changes outside the inventory window (`take_inventory_edits`).
    inventory_edits: Vec<aurora_net::inventory::operations::Mutation>,
    pub attachment_inventory: attachment_inventory::AttachmentInventory,
    pub received_items: received_items::ReceivedItems,
    /// Playing animations per avatar / animesh.
    pub animations: HashMap<Uuid, Vec<PlayingAnimation>>,
    /// AO remapping applies only to our rendered pose, never movement signals.
    pub ao_pose: Option<Vec<PlayingAnimation>>,
    /// Aurora's movement: our walk, run, crouch and fly started and
    /// stopped with the keys over the simulator's signals (own_motion.rs),
    /// and the list drawn instead of `animations` while it differs.
    own_locomotion: crate::own_motion::Locomotion,
    own_drawn: Option<Vec<PlayingAnimation>>,
    /// When the network read the event being applied (`apply_received`).
    received: Option<Instant>,
    local_preview: Option<PlayingAnimation>,
    /// (motion, start, sample) for the importer's locally controlled timeline.
    pub animation_preview_clock: Option<(Uuid, Instant, Instant)>,
    preview_server_motion: Option<PlayingAnimation>,
    /// Our animation sources, kept until the simulator stops the motion.
    animation_sources: HashMap<Uuid, HashSet<Uuid>>,
    /// The motions sources started on our avatar (Animation explorer).
    pub animation_log: animation_log::AnimationLog,
    /// Motions to stop after leaving a seat or losing a source (drained by app).
    pub animation_stops: Vec<Uuid>,
    /// Skeletons other avatars asked us to reset, with their animations or
    /// not (ViewerEffect ResetSkeleton, drained by app).
    pub skeleton_resets: Vec<(Uuid, bool)>,
    /// EEP environments (region, parcel, local) and their crossfades.
    pub eep: eep_env::EnvSelector,
    /// Monotonic clock of the environment crossfades.
    env_clock: Instant,
    /// Accumulated cloud scroll (LLEnvironment::mCloudScrollDelta).
    pub cloud_scroll: glam::Vec2,
    /// Directions of the shown sun and moon (the sky frame's), for their
    /// beacons.
    pub sky_directions: (glam::Vec3, glam::Vec3),
    /// Name of the parcel the agent stands on.
    pub parcel_name: String,
    /// Full description of that parcel (navigation bar icons, media, voice).
    pub parcel: Option<Arc<aurora_net::ParcelInfo>>,
    pub parcel_visibility: parcel_visibility::Visibility,
    /// The photo's isolation (`snapshot::isolate`): read with the parcel
    /// filter by `parcel_object_visibility`.
    pub photo_isolation: crate::snapshot::isolate::Isolation,
    /// About Land: the selected parcel, covenant, group / experience names.
    pub land: land::Land,
    /// The Region / Estate window's region and estate.
    pub estate: estate::Estate,
    /// Media texture overrides and parcel media messages (media module).
    pub media: crate::media::WorldMedia,
    /// The account's media filter list and its questions (media and music).
    pub media_filter: crate::media::filter::MediaFilter,
    /// Avatar profiles and the ids still to request.
    pub profiles: profiles::Profiles,
    /// Notification center (bell).
    pub notifications: notifications::Notifications,
    /// Teleport offers and requests from other residents, and their answers.
    pub teleport_invites: teleport_invites::TeleportInvites,
    /// Residents' inventory offers, as cards in their conversations.
    pub inventory_offers: inventory_offers::InventoryOffers,
    /// Residents' direct payments, as cards in their conversations.
    pub payments: payments::Payments,
    /// The content rating chosen during the session and its sync with the
    /// server.
    pub maturity: maturity::MaturityPreference,
    /// World › Set Home to Here waiting for the simulator's answer.
    pub set_home: home::SetHome,
    /// Preferences › Privacy's account settings kept by the grid.
    pub user_info: user_info::UserInfoState,
    /// The region console of estate managers (SimConsoleAsync).
    pub sim_console: sim_console::SimConsole,
    /// The LSL syntax ids already asked for this session (lsl_syntax.rs).
    lsl_syntax_asked: HashSet<Uuid>,
    /// Experiences: details, questions waiting for them, permissions.
    pub experiences: experiences::Experiences,
    /// Teleport history (navigation bar back / forward).
    pub tp_history: tphistory::TeleportHistory,
    /// Dated history of the Places window (teleport_history.txt).
    pub tp_storage: tphistory::HistoryStorage,
    /// Arrival waiting for its parcel name before it is saved in
    /// `tp_storage` (LLViewerParcelMgr's teleport finished signal): global
    /// position and when it happened.
    tp_record: Option<(glam::DVec3, Instant)>,
    /// Between the change of region of a border crossing and its arrival.
    crossing: bool,
    /// The last arrival was a border crossing: the agent was not placed.
    pub arrived_by_crossing: bool,
    /// Inventory folders asked again after a failed fetch since the agent
    /// last changed region (`InventoryFetchFailed`).
    inventory_retried: HashSet<Uuid>,
    origin_shift: Vec3,
    /// Since when our avatar wears objects the region it is in has not sent
    /// (`drop_left_behind`).
    left_behind_since: Option<Instant>,
    /// Landmark assets and the handles of their regions.
    pub landmarks: landmarks::Landmarks,
    /// World map regions, items, tracking and parcel overlays.
    pub map: worldmap::WorldMap,
    /// Place details windows of place links (their parcel requests).
    pub place_details: place_details::PlaceDetails,
    /// Parcel snapshots of the favourites and the Places rows without a
    /// thumbnail.
    pub place_snapshots: place_snapshots::PlaceSnapshots,
    /// "Search": result lists and details of the directory search.
    pub search: search::Search,
    /// Members, roles, titles and notices of the groups whose profile is
    /// open (LLGroupMgr).
    pub group_data: group_data::GroupMgr,
    /// The parcels and accounts of the group whose "Land and L$" tab shows.
    pub group_money: group_money::GroupMoney,
    arrived_once: bool,
    /// The instant messages kept while offline were asked for (once per
    /// login).
    offline_im_requested: bool,
    /// Which group titles the name tags show (from the settings).
    pub group_titles: names::GroupTitles,
    /// Script count and simulator version alerts (region_alerts.rs).
    pub region_alerts: region_alerts::RegionAlerts,
    /// Bumps, pushes and hits (collisions.rs).
    pub collisions: collisions::Collisions,
    /// Always fly after a teleport (FSFlyAfterTeleport, from the settings).
    pub fly_after_teleport: bool,
    /// The account's list of open private conversations (open_ims.rs).
    open_ims_path: Option<std::path::PathBuf>,
    /// Conversations of the last session, read at login, that the app
    /// reopens when the setting asks for it.
    pub reopen_ims: Vec<Uuid>,
    /// Disk cache root (inventory cache).
    pub cache_dir: Option<std::path::PathBuf>,
}

/// Lines kept in memory for one conversation (the nearby chat is one). The
/// view only lays out the lines in sight, so the bound is about memory;
/// older lines are in the conversation's transcript.
const MAX_CHAT: usize = 2000;
/// An arrival without teleport this far from where the agent is was not a
/// walk into a neighbour: the agent is placed as for a teleport.
const CROSSING_MAX_GAP: f32 = 64.0;
/// The typing notices of a conversation with one resident (llinstantmessage.h).
pub const IM_TYPING_START: u8 = 41;
pub const IM_TYPING_STOP: u8 = 42;

/// "(Saved <date>) " before a message kept while we were offline, with the
/// moment it was sent (Second Life time, like the times of the chat).
fn saved_prefix(timestamp: u32) -> String {
    if timestamp == 0 {
        return String::new();
    }
    let t = chat_log::format::Slt::from_unix(i64::from(timestamp));
    let month = crate::ui::land::months()[(t.month.clamp(1, 12) - 1) as usize];
    aurora_i18n::tr!(
        "world-im-saved",
        day = t.day.to_string(),
        month = month.to_string(),
        year = t.year.to_string(),
        time = format!("{:02}:{:02}", t.hour, t.minute)
    ) + " "
}

impl World {
    pub fn water_visual_dz(&self, id: &Uuid) -> f32 {
        if *id == self.agent_id && !self.teleporting && self.movement_complete && !self.agent.is_sitting() {
            self.water_float.offset()
        } else {
            0.0
        }
    }
    /// LLFloaterRegionInfo::refreshFromRegion on `handle`: the panels ask
    /// (estate, access, covenant, experiences, environment), then, when the
    /// window opens, RequestRegionInfo (requestRegionInfo).
    pub fn refresh_region_window(&mut self, handle: RegionHandle, request_info: bool) {
        self.estate.open(handle);
        let covenant = self.land.covenant_request(handle);
        self.estate.push(covenant);
        if request_info {
            self.estate.request_region_info(handle);
        }
    }

    pub fn new(avatar_lib: Arc<AvatarLibrary>) -> World {
        World {
            hud_aspect: 1.0,
            hud_area: None,
            hud_zoom: 1.0,
            hud_scales: Default::default(),
            hud_locks: Default::default(),
            temp_attachment_names: Default::default(),
            hover_height: Default::default(),
            favorites: Default::default(),
            notification_history: Default::default(),
            ui_sounds: Vec::new(),
            im_messages: Vec::new(),
            attention: Vec::new(),
            balance_changes: Vec::new(),
            typing_started: Vec::new(),
            nearby_typing: HashSet::new(),
            login: None,
            agent_id: Uuid::nil(),
            regions: HashMap::new(),
            main_region: None,
            objects: ObjectStore::default(),
            scripted_camera: Default::default(),
            motion_tick: Instant::now(),
            gltf_overrides: HashMap::new(),
            chat: VecDeque::new(),
            bubble_chat: Default::default(),
            chat_clock: Default::default(),
            chat_unread: 0,
            nearby_news: false,
            chat_badge: 0,
            quiet_automated_chat: false,
            notice_topics: Default::default(),
            chat_log: Default::default(),
            nearby_history: Default::default(),
            script_debug: script_debug::Messages::default(),
            sun: None,
            agent: AgentState::default(),
            following_avatar: None,
            script_controls: Default::default(),
            coarse: HashMap::new(),
            region_left: None,
            parked: HashMap::new(),
            appearances: HashMap::new(),
            avatar_poses: HashMap::new(),
            avatar_models: HashMap::new(),
            avatar_shape_dz: HashMap::new(),
            water_float: Default::default(),
            bodies: HashMap::new(),
            look_at: lookat::LookAts::default(),
            point_at: pointat::PointAts::default(),
            object_im_filter: object_im_filter::Settings::default(),
            anti_spam: Default::default(),
            appearance_counter: 1,
            bake_refreshes: HashMap::new(),
            shape_preview: None,
            preview_bakes: HashMap::new(),
            shape_preview_generation: 0,
            alerts: VecDeque::new(),
            region_restart: Default::default(),
            movement_complete: false,
            demo_outfit_region: false,
            avatar_lib,
            teleporting: false,
            tp_show_progress: false,
            tp_progress: 0.0,
            tp_status: String::new(),
            tp_failed: false,
            outfit: Default::default(),
            diag_reported: HashSet::new(),
            events_this_frame: 0,
            motd: String::new(),
            balance: None,
            land_committed: 0,
            land_credit: 0,
            upload_cost: None,
            local_assets: HashMap::new(),
            local_asset_requests: Vec::new(),
            environment_answers: Vec::new(),
            health: 100.0,
            frozen: false,
            held_kills: Vec::new(),
            social: social::Social::default(),
            contact_sets: Default::default(),
            recent: Default::default(),
            aliases_synced: 0,
            mutes: mutes::MuteList::default(),
            poser_consent: poser_consent::State::default(),
            portals: portals::State::default(),
            lsl_bridge: lslbridge::LslBridge::default(),
            hand_contacts: hand_ik::Contacts::default(),
            ik_targets: rig_targets::Targets::default(),
            rlva: rlva::Rlva::default(),
            rlva_privacy: Default::default(),
            rlva_hover_privacy: Default::default(),
            rlva_location_hidden: false,
            derender: derender::Derender::default(),
            groups: groups::GroupChats::default(),
            calls: calls::Calls::default(),
            status: status::Status::default(),
            inventory: inventory::Inventory::default(),
            active_gestures: Default::default(),
            inventory_cache: None,
            inventory_background: Default::default(),
            inventory_edits: Vec::new(),
            attachment_inventory: Default::default(),
            received_items: Default::default(),
            animations: HashMap::new(),
            ao_pose: None,
            own_locomotion: Default::default(),
            own_drawn: None,
            received: None,
            local_preview: None,
            animation_preview_clock: None,
            preview_server_motion: None,
            animation_sources: HashMap::new(),
            animation_log: Default::default(),
            animation_stops: Vec::new(),
            skeleton_resets: Vec::new(),
            eep: eep_env::EnvSelector::default(),
            env_clock: Instant::now(),
            cloud_scroll: glam::Vec2::ZERO,
            sky_directions: (glam::Vec3::Z, glam::Vec3::NEG_Z),
            parcel_name: String::new(),
            parcel_visibility: Default::default(),
            photo_isolation: Default::default(),
            parcel: None,
            land: land::Land::default(),
            estate: estate::Estate::default(),
            media: Default::default(),
            media_filter: Default::default(),
            tp_history: tphistory::TeleportHistory::default(),
            tp_storage: Default::default(),
            tp_record: None,
            crossing: false,
            arrived_by_crossing: false,
            inventory_retried: HashSet::new(),
            origin_shift: Vec3::ZERO,
            left_behind_since: None,
            landmarks: Default::default(),
            map: worldmap::WorldMap::default(),
            place_details: Default::default(),
            place_snapshots: Default::default(),
            search: Default::default(),
            group_data: Default::default(),
            group_money: Default::default(),
            notifications: notifications::Notifications::default(),
            teleport_invites: teleport_invites::TeleportInvites::default(),
            inventory_offers: inventory_offers::InventoryOffers::default(),
            payments: payments::Payments::default(),
            maturity: maturity::MaturityPreference::default(),
            set_home: home::SetHome::default(),
            user_info: user_info::UserInfoState::default(),
            sim_console: sim_console::SimConsole::default(),
            lsl_syntax_asked: HashSet::new(),
            experiences: experiences::Experiences::default(),
            profiles: profiles::Profiles::default(),
            arrived_once: false,
            offline_im_requested: false,
            group_titles: Default::default(),
            region_alerts: Default::default(),
            collisions: Default::default(),
            fly_after_teleport: false,
            open_ims_path: None,
            reopen_ims: Vec::new(),
            cache_dir: None,
        }
    }

    /// Back to the login screen. Nothing said to the account that leaves
    /// stays for the next one: the conversations and the nearby chat go, and
    /// the transcripts close (only their writer and settings are kept).
    pub fn reset(&mut self) {
        self.save_open_ims();
        let lib = self.avatar_lib.clone();
        let cache_dir = self.cache_dir.take();
        let mut chat_log = std::mem::take(&mut self.chat_log);
        chat_log.logout();
        // a setting, not the session's: the arrival's lines follow it too
        let quiet = self.quiet_automated_chat;
        *self = World::new(lib);
        self.chat_log = chat_log;
        self.cache_dir = cache_dir;
        self.quiet_automated_chat = quiet;
    }

    /// saveOpenIMs, when the session ends (quit, logout or disconnection).
    pub fn save_open_ims(&self) {
        if let Some(path) = &self.open_ims_path {
            let others: Vec<Uuid> = self.social.ims.iter().map(|s| s.other).collect();
            open_ims::save(path, &others);
        }
    }

    /// LLTeleportHistory::onTeleportFinished → LLTeleportHistoryStorage:
    /// the arrival is saved once the parcel of the new place is known.
    fn record_arrival(&mut self) {
        let Some((global, _)) = self.tp_record.take() else {
            return;
        };
        if self.rlva.has("showloc") {
            return;
        }
        let region = self
            .main_region
            .and_then(|h| self.regions.get(&h))
            .map(|r| r.name.clone())
            .unwrap_or_default();
        // the arrows' list names the place by its parcel too
        self.tp_history.name_parcel(&self.parcel_name);
        let title = tphistory::location_title(&self.parcel_name, &region);
        if title.is_empty() {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        self.tp_storage.add(&title, global, now);
    }

    /// An arrival whose parcel did not come within a few seconds is saved
    /// with what is known (the region name).
    pub fn flush_arrival(&mut self, now: Instant) {
        if self.tp_record.is_some_and(|(_, at)| now.duration_since(at).as_secs_f32() > 5.0) {
            self.record_arrival();
        }
        self.drop_left_behind(now);
    }

    /// Remove every object of a region and what is stored by their key.
    /// LLViewerObjectList::killObject never destroys gAgentAvatarp: our
    /// avatar moves to the agent's region with what it wears, which the
    /// destination then sends under its own ids.
    fn remove_region_objects(&mut self, handle: RegionHandle) {
        let kept = match (self.objects.index_of_uuid(&self.agent_id), self.main_region) {
            (Some(own), Some(main)) if main != handle => {
                self.objects.carry(own, handle, main);
                self.objects.subtree(own)
            }
            _ => Vec::new(),
        };
        self.scripted_camera.clear_region(handle, |id| {
            self.objects
                .index_of_uuid(&id)
                .is_some_and(|i| kept.contains(&i) || self.objects.get(i).is_some_and(|o| o.key.region != handle))
        });
        self.objects.remove_region(handle, &kept);
        self.move_rekeyed();
        self.gltf_overrides.retain(|k, _| k.region != handle);
        self.derender.forget_region(handle);
        self.forget_gone_look_ats();
    }

    /// The look-ats of the avatars and toward the objects that just left
    /// the store (`lookat::LookAts::forget`), and the typing of those
    /// avatars (gone with their LLVOAvatar).
    pub(crate) fn forget_gone_look_ats(&mut self) {
        let objects = &self.objects;
        self.look_at.forget(|id| objects.index_of_uuid(id).is_some());
        self.point_at.forget(|id| objects.index_of_uuid(id).is_some());
        self.nearby_typing.retain(|id| objects.index_of_uuid(id).is_some());
    }

    /// The avatar types in the nearby chat: its viewer said so, or it plays
    /// the typing animation (what the demo's crowd shows). The Legacy
    /// Viewer reads only the former (LLVOAvatar::isTyping), which a viewer
    /// sends even with its typing animation turned off.
    pub fn is_typing_nearby(&self, avatar: &Uuid) -> bool {
        self.nearby_typing.contains(avatar) || self.animations_of(avatar).iter().any(|a| a.id == radar::ANIM_AGENT_TYPE)
    }

    /// What is stored by object key follows the objects that changed key
    /// without being removed (border crossing, renumbering).
    fn move_rekeyed(&mut self) {
        for (old, new) in self.objects.take_rekeyed() {
            if let Some(sides) = self.gltf_overrides.remove(&old) {
                self.gltf_overrides.insert(new, sides);
            }
        }
    }

    /// process_kill_object: our avatar is never killed, and neither is what
    /// it wears during a teleport or when another region than the agent's
    /// asks for it (the one left by a teleport or a border crossing):
    /// Firestorm's FSExperimentalLostAttachmentsFix, FIRE-12004, on by
    /// default. Otherwise the agent's region detaches what it says.
    fn kill_is_ignored(&self, key: &ObjKey) -> bool {
        let Some(mut idx) = self.objects.index_of(key) else {
            return false;
        };
        let Some(own) = self.objects.index_of_uuid(&self.agent_id) else {
            return false;
        };
        if idx == own {
            return true;
        }
        if Some(key.region) == self.main_region && !self.teleporting {
            return false;
        }
        for _ in 0..16 {
            match self.objects.get(idx).and_then(|o| self.objects.parent_of(o)) {
                Some(p) if p == own => return true,
                Some(p) => idx = p,
                None => return false,
            }
        }
        false
    }

    /// What our avatar wore that the region it arrived in did not send
    /// again (taken off on the way, or a kill ignored above): removed once
    /// that region has had the time to send it. Firestorm keeps such ghosts
    /// until its next attachment refresh.
    fn drop_left_behind(&mut self, now: Instant) {
        const LEFT_BEHIND_AFTER: f32 = 15.0;
        let Some(avatar) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            self.left_behind_since = None;
            return;
        };
        let (key, region) = (avatar.key, avatar.key.region);
        // Render-only previews have a private namespace; a simulator can
        // never confirm their arrival or require their removal.
        let stale = |w: &World, i: &usize| w.objects.get(*i).is_some_and(|o| o.key.region != region && !o.is_client_preview());
        if !self.objects.children_of(&key).iter().any(|i| stale(self, i)) {
            self.left_behind_since = None;
            return;
        }
        let since = *self.left_behind_since.get_or_insert(now);
        // (not counted while the avatar itself still waits for that region)
        if key.local_id == objects::CARRIED_LOCAL_ID {
            self.left_behind_since = Some(now);
            return;
        }
        if now.duration_since(since).as_secs_f32() < LEFT_BEHIND_AFTER {
            return;
        }
        let gone: Vec<usize> = self.objects.children_of(&key).iter().copied().filter(|i| stale(self, i)).collect();
        log::info!("{} worn objects not sent by the region arrived in: removed", gone.len());
        for i in gone {
            self.release_linkset_camera(i, true);
            self.objects.remove_idx(i);
        }
        self.forget_gone_look_ats();
        self.left_behind_since = None;
    }

    pub fn main_origin(&self) -> Option<(u32, u32)> {
        self.main_region.map(aurora_net::handle_to_origin)
    }

    /// The agent's region changes; returns whether it did. Positions are
    /// relative to it: as LLAgent::setRegion, an agent walking or flying
    /// into a neighbour keeps its place in the world, and the camera follows
    /// (`take_origin_shift`). A teleport places the agent on its arrival.
    fn set_main_region(&mut self, handle: RegionHandle) -> bool {
        let old = self.main_region.replace(handle);
        self.ensure_region(handle, (256, 256));
        if old == Some(handle) {
            return false;
        }
        self.region_left = old;
        self.objects.clear_material_previews();
        self.inventory_retried.clear();
        if let Some(old) = old {
            let (ox, oy) = aurora_net::handle_to_origin(old);
            let (nx, ny) = aurora_net::handle_to_origin(handle);
            self.scripted_camera
                .shift(Vec3::new((ox as f64 - nx as f64) as f32, (oy as f64 - ny as f64) as f32, 0.0));
        }
        if let Some(old) = old
            && self.arrived_once
            && !self.teleporting
        {
            let (ox, oy) = aurora_net::handle_to_origin(old);
            let (nx, ny) = aurora_net::handle_to_origin(handle);
            let shift = Vec3::new((ox as f64 - nx as f64) as f32, (oy as f64 - ny as f64) as f32, 0.0);
            self.agent.shift(shift);
            self.origin_shift += shift;
            self.crossing = true;
        }
        true
    }

    /// How far what is kept in main-region coordinates outside the world
    /// (the camera) must move since last asked, after a border crossing.
    pub fn take_origin_shift(&mut self) -> Vec3 {
        std::mem::take(&mut self.origin_shift)
    }

    /// Behaviour of LLAgent::teleportCore(is_local), indra/newview/llagent.cpp:
    /// track local teleports without showing progress. Intentional difference
    /// requested for Aurora: local teleports are also silent.
    pub fn begin_teleport(&mut self, show_progress: bool) {
        self.cancel_friend_join_flight();
        self.cancel_friend_join();
        self.map.cancel_avatar_teleport();
        self.teleporting = true;
        self.tp_show_progress = show_progress;
        self.tp_failed = false;
        self.tp_progress = 0.05;
        self.tp_status = aurora_i18n::tr!("world-mod-requesting-teleport").into();
        if show_progress {
            self.ui_sounds.push(UiSound::TeleportOut);
        }
        self.get_off_for_teleport();
        // teleportCore: gObjectList.resetDerenderList()
        self.clear_temporary_derenders();
    }

    /// Offset of a region's origin relative to the render origin (main region).
    pub fn region_offset(&self, h: RegionHandle) -> Option<Vec3> {
        if h == objects::PREVIEW_REGION {
            return Some(Vec3::ZERO);
        }
        let (mx, my) = self.main_origin()?;
        let (x, y) = aurora_net::handle_to_origin(h);
        Some(Vec3::new(x as f32 - mx as f32, y as f32 - my as f32, 0.0))
    }

    /// Our global position (meters), for picks.
    pub fn agent_global(&self) -> Option<glam::DVec3> {
        let (x, y) = self.main_origin()?;
        let p = self.agent.position;
        Some(glam::DVec3::new(x as f64 + p.x as f64, y as f64 + p.y as f64, p.z as f64))
    }

    /// The content the agent may and wants to see (LLAgentAccess), from
    /// the login reply, with the preference chosen during the session.
    pub fn agent_access(&self) -> aurora_net::search::AgentAccess {
        let mut access = self.login.as_ref().map_or_else(Default::default, |l| {
            aurora_net::search::AgentAccess::from_login(
                &l.raw["agent_access_max"].to_string_value(),
                &l.raw["agent_region_access"].to_string_value(),
            )
        });
        if let Some(preferred) = self.maturity.preferred {
            access.preferred = preferred.min(access.max);
        }
        access
    }

    /// Change the content rating the agent wants (the Preferences' choice,
    /// LLAgent::handleMaturity): sent to the server, which may keep another.
    pub fn set_maturity_preference(&mut self, maturity: u8) {
        let current = self.agent_access().preferred;
        if maturity == current && self.maturity.synced() {
            return;
        }
        log::info!("maturity: asking for {}", aurora_net::search::access_short_string(maturity));
        self.maturity.request(maturity, current);
    }

    /// The server's answer about the preference: on a failure the teleport
    /// waiting for it ends and MaturityChangeError says what the server
    /// kept (reportPreferredMaturityError).
    fn on_maturity_answer(&mut self, result: Option<u8>) {
        let held = self.maturity.holds_teleport();
        if let maturity::Outcome::Failed { preferred, actual } = self.maturity.answer(result) {
            log::warn!(
                "maturity: the server keeps {} instead of {}",
                aurora_net::search::access_short_string(actual),
                aurora_net::search::access_short_string(preferred)
            );
            if held {
                self.teleporting = false;
                self.tp_show_progress = false;
            }
            let name = |m: u8| crate::ui::bars::maturity_name(m).to_lowercase();
            self.notifications.push(
                notifications::Kind::Alert,
                aurora_i18n::tr!("world-maturity-change-error-title"),
                aurora_i18n::tr!("world-maturity-change-error", preferred = name(preferred), actual = name(actual)),
                notifications::Data::None,
            );
        }
    }

    /// Every classified category of the grid, (id, name) by id.
    pub fn classified_categories(&self) -> Vec<(u32, String)> {
        let mut v: Vec<(u32, String)> = self
            .login
            .as_ref()
            .map(|l| {
                l.raw["classified_categories"]
                    .as_array()
                    .iter()
                    .map(|c| (c["category_id"].as_i32().max(0) as u32, c["category_name"].to_string_value()))
                    .filter(|(id, name)| *id != 0 && !name.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Classified categories of the grid (login "classified_categories").
    pub fn classified_category(&self, id: u32) -> String {
        self.login
            .as_ref()
            .and_then(|l| {
                l.raw["classified_categories"]
                    .as_array()
                    .iter()
                    .find(|c| c["category_id"].as_i32() == id as i32)
                    .map(|c| c["category_name"].to_string_value())
            })
            .unwrap_or_default()
    }

    pub fn main(&self) -> Option<&Region> {
        self.main_region.and_then(|h| self.regions.get(&h))
    }

    /// Cached inventory can precede arrival and the login seed's services.
    /// Match Session::EditInventory's metadata-write/readback preconditions.
    pub fn inventory_writable(&self) -> bool {
        self.movement_complete
            && !self.teleporting
            && self.inventory.held.is_none()
            && self.main().is_some_and(|region| {
                ["InventoryAPIv3", "FetchInventoryDescendents2"] // gitleaks:allow -- public protocol capability names
                    .iter()
                    .all(|name| region.caps.get(*name).is_some_and(|url| !url.is_empty()))
            })
    }

    pub fn main_water_height(&self) -> f32 {
        self.main().and_then(|r| r.info.as_ref()).map(|i| i.water_height).unwrap_or(20.0)
    }

    fn inventory_cache_path(&self) -> Option<std::path::PathBuf> {
        let dir = self.cache_dir.as_ref()?;
        (!self.agent_id.is_nil()).then(|| dir.join("inventory").join(format!("{}.inv", self.agent_id)))
    }

    /// Apply the inventory cache once its thread has read it (every frame
    /// after the login reply), then let the waiting folder requests go.
    pub fn poll_inventory_cache(&mut self) {
        let Some((reader, since)) = &self.inventory_cache else {
            return;
        };
        let cache = match reader.try_recv() {
            Ok(cache) => cache,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => None,
        };
        let waited = since.elapsed().as_secs_f32() * 1000.0;
        let t = Instant::now();
        let folders = cache.map_or(0, |c| self.inventory.apply_cache(c));
        log::info!(
            "inventory cache: {folders} folders up to date (read in {waited:.0} ms off the main thread, applied in {:.1} ms)",
            t.elapsed().as_secs_f32() * 1000.0
        );
        self.inventory_cache = None;
        self.inventory.release_requests();
    }

    /// The background inventory fetch's reads for this frame. It starts once
    /// the cache is applied (the folders it restored need no fetch) and the
    /// agent's region has its capabilities.
    pub fn pump_inventory_background(&mut self) -> Vec<aurora_net::NetCommand> {
        if self.inventory_cache.is_some() || self.inventory.root.is_nil() || self.main().is_none_or(|r| r.caps.is_empty()) {
            return Vec::new();
        }
        self.inventory_background.start(&self.inventory);
        self.inventory.repair_lost_hierarchy();
        self.inventory_background.pump(&mut self.inventory, self.agent_id)
    }

    /// Persist the fetched inventory (next login skips unchanged folders).
    pub fn save_inventory_cache(&self) {
        // not applied yet: the file holds more than the inventory in memory
        if self.inventory_cache.is_some() {
            return;
        }
        if let Some(path) = self.inventory_cache_path() {
            self.inventory.save_cache(&path);
        }
    }

    pub fn viewer_asset_url(&self) -> Option<String> {
        if let Some(u) = self.main().and_then(|r| r.caps.get("ViewerAsset")) {
            return Some(u.clone());
        }
        self.regions.values().find_map(|r| r.caps.get("ViewerAsset").cloned())
    }

    /// The agent stands inside its region (process_enable_simulator: with
    /// the neighbours off, they still connect while crossing a border).
    pub fn agent_inside_main_region(&self) -> bool {
        let Some(r) = self.main_region.and_then(|h| self.regions.get(&h)) else {
            return true;
        };
        let p = self.agent.position;
        p.x >= 0.0 && p.y >= 0.0 && p.x < r.heightmap.size_x as f32 && p.y < r.heightmap.size_y as f32
    }

    /// Terrain height under a render-space position.
    pub fn ground_height(&self, p: Vec3) -> Option<f32> {
        for r in self.regions.values() {
            let off = self.region_offset(r.handle)?;
            let lx = p.x - off.x;
            let ly = p.y - off.y;
            if lx >= 0.0 && ly >= 0.0 && lx < r.heightmap.size_x as f32 && ly < r.heightmap.size_y as f32 {
                return Some(r.heightmap.sample(lx, ly));
            }
        }
        None
    }

    /// Lowest height the centre of an avatar can be drawn at, `local` being
    /// its position in its region: LLViewerObject::interpolateLinearMotion
    /// keeps an extrapolated avatar half its height above the land.
    pub fn avatar_floor(&self, o: &Object, local: Vec3) -> Option<f32> {
        let off = self.region_offset(o.key.region)?;
        Some(self.ground_height(off + local)? + 0.5 * o.scale.z)
    }

    /// LLAgent::propagate (indra/newview/llagent.cpp): holding descend near
    /// the floor ends flight below 4 m/s horizontally. Resolve terrain and
    /// the simulator's object contact plane as LLWorld::resolveStepHeightGlobal
    /// does (llworld.cpp), with LLVOAvatar's 0.25 m foot tolerance.
    pub fn auto_land(&mut self, input: &crate::agent::MoveInput) {
        if !self.agent.flying
            || !input.down
            || input.up
            || self.agent.is_sitting()
            || !self.movement_complete
            || self.teleporting
            || self.crossing
            || !self.agent.velocity.is_finite()
            || self.agent.velocity.truncate().length_squared() >= 16.0
        {
            return;
        }
        let Some(avatar) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return;
        };
        // Use physical reports, never the prediction's lead, visual hover or
        // animated feet: these must not end a flight ahead of its support.
        let position = self.agent.server_pos;
        let height = avatar.scale.z;
        if Some(avatar.key.region) != self.main_region || !position.is_finite() || !height.is_finite() || height <= 0.0 {
            return;
        }
        let terrain = self.ground_height(position).filter(|z| z.is_finite());
        let contact = avatar
            .foot_plane
            .filter(|p| p.is_finite() && p.z > 0.1)
            // The plane is n.p = w, in main-region coordinates. The 5 cm
            // allowance follows resolveStepHeightGlobal's Havok correction.
            .map(|p| (p.w - p.x * position.x - p.y * position.y - 0.05) / p.z)
            .filter(|z| z.is_finite());
        let floor = match (terrain, contact) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if floor.is_some_and(|floor| position.z - height * 0.5 - floor <= 0.25) {
            // Clearing FLY is sufficient; STOP would brake the remaining momentum.
            self.agent.flying = false;
        }
    }

    /// Let the clocks of the regions run up to `now` (motion.rs): each at
    /// the time dilation of its simulator, slowing to a stop when its
    /// circuit goes silent. A region without a simulator in `sims` (the
    /// offline demo, a region parked or just announced) runs at full rate.
    /// Corrections that have faded end here, and so do the waits of our
    /// transform edits for their answer (edit_echo.rs). Then the roots
    /// avatars sit on take a step of their damping, those `selected` in the
    /// build tools excepted (ObjectStore::damp_seats).
    pub fn advance_motion(&mut self, now: Instant, sims: &[aurora_net::stats::SimulatorMotion], selected: impl Fn(&ObjKey) -> bool) {
        let dt = now.saturating_duration_since(self.motion_tick).as_secs_f64();
        self.motion_tick = now;
        for &handle in self.regions.keys() {
            let rate = sims
                .iter()
                .find(|s| s.handle == Some(handle))
                .map_or(1.0, |s| motion::clock_rate(s.time_dilation, s.packet_age()));
            self.objects.advance_motion(handle, dt * f64::from(rate));
            self.objects.note_motion_tick(now, handle, rate);
        }
        self.objects.expire_corrections(now);
        self.objects.expire_sent_edits(now);
        self.objects.damp_seats(now, dt as f32, selected);
    }

    /// The objects the simulator killed (process_kill_object,
    /// indra/newview/llviewermessage.cpp, originally LGPL 2.1).
    fn kill_objects(&mut self, handle: RegionHandle, local_ids: &[u32]) {
        let land_watches = self.land.watches(handle);
        let mut land_changed = false;
        for &id in local_ids {
            let key = ObjKey {
                region: handle,
                local_id: id,
            };
            if self.kill_is_ignored(&key) {
                continue;
            }
            self.derender.forget_key(&key);
            land_changed |= land_watches
                && self
                    .objects
                    .index_of(&key)
                    .and_then(|i| self.objects.get(i))
                    .is_some_and(|o| !o.is_avatar());
            if let Some(i) = self.objects.index_of(&key) {
                self.unregister_attachment(i);
                // (who sits on it stays, with what it wears: the
                // simulator does not send them again)
                self.get_off_linkset(i);
                let subtree = self.objects.subtree(i);
                for &child in &subtree {
                    if let Some(source) = self.objects.get(child).map(|o| o.full_id) {
                        self.scripted_camera.clear(source);
                        self.stop_motion_from_source(source);
                    }
                }
                // (an avatar leaving, an object deleted or taken, a
                // worn item taken off)
                self.objects.killed.push(subtree);
            }
            self.objects.remove(&key);
            // a cache hit replays the cached ones, a full update comes with them
            self.gltf_overrides.remove(&key);
        }
        self.forget_gone_look_ats();
        if land_changed {
            self.land.objects_changed(handle, Instant::now());
        }
    }

    /// While the photo mode holds the world still (issue #1154), what the
    /// simulator kills stays drawn until the thaw, like the moves it
    /// reports (`ObjectStore::hold_poses`): a derezzed prop, a temporary
    /// object or an avatar leaving does not spoil the shot being lined up.
    /// What the user does goes at once: what our avatar wears (an
    /// attachment taken off) and the objects we own (deleted or taken).
    /// The Legacy Viewer's FreezeTime still removes them: a visible
    /// difference asked for by the testers. Returns the kills to apply now.
    fn hold_kills(&mut self, handle: RegionHandle, local_ids: Vec<u32>) -> Vec<u32> {
        if !self.objects.holds_poses() {
            return local_ids;
        }
        let own = self.objects.index_of_uuid(&self.agent_id);
        let mut now = Vec::new();
        for id in local_ids {
            let key = ObjKey {
                region: handle,
                local_id: id,
            };
            let held = self.objects.index_of(&key).and_then(|idx| {
                let o = self.objects.get(idx)?;
                // (UpdateFlags tells it in every update; the owner only
                // comes with a sound or particles)
                let ours = o.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
                    || o.owner_id == self.agent_id
                    || own.is_some_and(|own| self.hangs_from(idx, own));
                (!ours).then_some(o.full_id)
            });
            match held {
                Some(full_id) => self.held_kills.push((key, full_id)),
                None => now.push(id),
            }
        }
        now
    }

    /// `idx` is `ancestor` or hangs from it (a prim of what an avatar wears).
    fn hangs_from(&self, mut idx: usize, ancestor: usize) -> bool {
        for _ in 0..64 {
            if idx == ancestor {
                return true;
            }
            match self.objects.get(idx).and_then(|o| self.objects.parent_of(o)) {
                Some(p) => idx = p,
                None => return false,
            }
        }
        false
    }

    /// Freeze the world at `at` for a photo, or thaw it (issue #1154,
    /// LLFloater360Capture::freezeWorld, indra/newview/llfloater360capture.cpp,
    /// originally LGPL 2.1): the objects and the avatars that move keep their
    /// place, ours with them (the agent places it, not a report).
    pub fn freeze_motion(&mut self, at: Option<Instant>) {
        let Some(at) = at else {
            self.objects.release_poses();
            // what left meanwhile goes now, unless its key went to another
            // object since (a new rez, or the same avatar back at another key)
            let kills = std::mem::take(&mut self.held_kills);
            if !kills.is_empty() {
                log::info!("world thawed: {} objects killed meanwhile removed", kills.len());
            }
            for (key, id) in kills {
                if self
                    .objects
                    .index_of(&key)
                    .and_then(|i| self.objects.get(i))
                    .is_some_and(|o| o.full_id == id)
                {
                    self.kill_objects(key.region, &[key.local_id]);
                }
            }
            return;
        };
        let own = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|idx| self.objects.get(idx))
            .filter(|o| o.parent_id == 0 && self.agent.has_local_control())
            .and_then(|o| {
                Some((
                    o.key,
                    (self.agent.position - self.region_offset(o.key.region)?, self.agent.body_rotation()),
                ))
            });
        self.objects.hold_poses(at, own);
    }

    /// A region that blocks flying lands our avatar when it arrives there or
    /// when the region's flags change (process_agent_movement_complete and
    /// process_region_info, indra/newview/llviewermessage.cpp): a flight
    /// restored at login, or carried into a no-fly region, ends.
    fn land_if_fly_blocked(&mut self, handle: RegionHandle) {
        let blocked = Some(handle) == self.main_region
            && self
                .regions
                .get(&handle)
                .and_then(|r| r.info.as_ref())
                .is_some_and(|i| i.region_flags & aurora_net::region_flags::BLOCK_FLY != 0);
        if blocked && self.agent.flying {
            log::info!("region {handle:x} blocks flying: landing");
            self.agent.flying = false;
        }
    }

    /// LLViewerObject::interpolateLinearMotion terrain and visible-region
    /// constraints. The default Firestorm crossing fix is disabled, so known
    /// neighboring regions do not impose a separate prediction time limit.
    pub fn predict_agent(&mut self, time_dilation: f32, packet_age: f32) {
        // (a border crossing does not stop the agent while the new region
        // completes the move)
        if !self.agent.has_local_control() || !(self.movement_complete || self.crossing) {
            return;
        }
        let start = self.agent.predicted_position();
        self.agent.predict(time_dilation, packet_age);
        let mut end = self.agent.predicted_position();
        if end == start {
            return;
        }
        let half_height = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|idx| self.objects.get(idx))
            .map_or(0.84, |o| 0.5 * o.scale.z);
        if let Some(ground) = self.ground_height(end) {
            end.z = end.z.max(ground + half_height);
        }
        let clipped = self.clip_to_visible_regions(start, end);
        self.agent.constrain_prediction(clipped, clipped != end);
    }

    /// When our avatar's report counts from: with Aurora's movement, when
    /// the network read it (item 6 of the network audit); with Firestorm's,
    /// the frame that applies it, as on Firestorm (`mLastInterpUpdateSecs`).
    /// The other objects take the reading time in both (`ObjectStore::report_clock`).
    fn agent_report_time(&self) -> Instant {
        match self.received {
            Some(received) if self.agent.predicting() => received,
            _ => Instant::now(),
        }
    }

    /// Keep the prediction's lead of our avatar (own_motion.rs) where the
    /// simulator could take it: in the regions we know, and on the land,
    /// whose slope it follows when walking.
    pub fn constrain_agent_lead(&mut self) {
        let lead = self.agent.wanted_lead();
        if lead == Vec3::ZERO {
            self.agent.set_lead(Vec3::ZERO);
            return;
        }
        let from = self.agent.predicted_position();
        let mut to = self.clip_to_visible_regions(from, from + lead);
        let half_height = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|idx| self.objects.get(idx))
            .map_or(0.84, |o| 0.5 * o.scale.z);
        let (here, there) = (self.ground_height(from), self.ground_height(to));
        if let (Some(here), Some(there)) = (here, there) {
            if !self.agent.flying && from.z - (here + half_height) < 0.3 {
                // walking: up and down the slope, as the simulator will
                to.z += there - here;
            }
            to.z = to.z.max(there + half_height);
        }
        self.agent.set_lead(to - from);
    }

    /// Start or stop our walk, run, crouch and fly animations with the keys
    /// (`intent`), with Aurora's movement on; off, or seated, in a teleport
    /// or before the arrival, the simulator's signals as they are.
    /// `window`: how long a local start or stop waits for the simulator.
    pub fn update_own_locomotion(
        &mut self,
        intent: Option<crate::own_motion::Intent>,
        window: f32,
        now: Instant,
        ao_basis: Option<PlayingAnimation>,
    ) {
        let standing = self.agent.has_local_control() && !self.agent.ground_sit;
        if !self.agent.predicting() || !standing || self.teleporting || !self.movement_complete {
            self.own_locomotion.reset();
            self.own_drawn = None;
            return;
        }
        let base = self.animations.get(&self.agent_id).map(Vec::as_slice).unwrap_or_default();
        // The AO can stop a replaced stock clip on the simulator. Retain its
        // semantic basis for local key-release prediction without altering the
        // received animation list or overriding a newer simulator state.
        let basis;
        let base = if let Some(signal) = ao_basis.filter(|_| {
            !base
                .iter()
                .any(|a| crate::ao::State::from_motion(a.id).is_some_and(|s| s != crate::ao::State::Typing))
        }) {
            basis = base.iter().copied().chain(std::iter::once(signal)).collect::<Vec<_>>();
            basis.as_slice()
        } else {
            base
        };
        // the walk goes on while the drawn avatar glides to its stop: the
        // feet do not slide
        let gliding = self.agent.drawn_velocity().truncate().length() > GLIDE_WALKING;
        let wanted = crate::own_motion::locomotion_animation(intent).or(if gliding { self.own_locomotion.wanted() } else { None });
        let moving = crate::own_motion::Moving {
            flying: self.agent.flying,
            // (the simulator's last report: does it still move our avatar)
            simulator: self.agent.velocity.truncate().length() > GLIDE_WALKING,
            drawn: gliding,
        };
        self.own_drawn = self.own_locomotion.overlay(base, wanted, moving, window, now);
    }

    /// Native Movelock must not force Standing while either the simulator or
    /// the drawn avatar is still coasting. Share the ordinary walk-stop speed;
    /// airborne anchoring also waits for vertical flight motion to settle.
    pub fn own_motion_moving(&self) -> bool {
        let speed_squared = |v: Vec3| {
            if self.agent.flying {
                v.length_squared()
            } else {
                v.truncate().length_squared()
            }
        };
        speed_squared(self.agent.velocity).max(speed_squared(self.agent.settling_velocity())) > GLIDE_WALKING.powi(2)
    }

    /// Our avatar's signals as the AO and the pose take them: the
    /// simulator's, with the keys' walk or stop over them
    /// (`update_own_locomotion`).
    pub fn own_signals(&self) -> &[PlayingAnimation] {
        match &self.own_drawn {
            Some(drawn) => drawn,
            None => self.animations.get(&self.agent_id).map(Vec::as_slice).unwrap_or_default(),
        }
    }

    /// Port of LLWorld::clipToVisibleRegions (indra/newview/llworld.cpp,
    /// originally LGPL 2.1). Coordinates here are relative to our main region.
    fn clip_to_visible_regions(&self, start: Vec3, end: Vec3) -> Vec3 {
        let region_at = |p: Vec3| {
            self.regions.values().find_map(|r| {
                let origin = self.region_offset(r.handle)?;
                let size = Vec3::new(r.heightmap.size_x as f32, r.heightmap.size_y as f32, 0.0);
                let local = p - origin;
                (local.x >= 0.0 && local.y >= 0.0 && local.x < size.x && local.y < size.y).then_some((origin, size))
            })
        };
        if region_at(end).is_some() {
            return end;
        }
        let Some((origin, size)) = region_at(start) else {
            return start;
        };
        let delta = end - start;
        let abs_delta = delta.abs();
        let local = end - origin;
        let factor = if local.x < 0.0 {
            if local.y < local.x {
                -local.y / abs_delta.y
            } else {
                -local.x / abs_delta.x
            }
        } else if local.x > size.x {
            if local.y > local.x {
                (local.y - size.y) / abs_delta.y
            } else {
                (local.x - size.x) / abs_delta.x
            }
        } else if local.y < 0.0 {
            -local.y / abs_delta.y
        } else if local.y > size.y {
            (local.y - size.y) / abs_delta.y
        } else {
            1.0
        };
        let mut clipped = local - delta * factor;
        clipped.x = clipped.x.clamp(0.0, size.x - 1e-5);
        clipped.y = clipped.y.clamp(0.0, size.y - 1e-5);
        clipped.z = clipped.z.clamp(0.0, 4096.0 - 1e-5);
        origin + clipped
    }

    fn push_chat(&mut self, line: ChatLine) {
        if self.alerts(line.kind) {
            self.chat_badge += 1;
        }
        self.nearby_push(line);
        self.chat_unread += 1;
    }

    /// A line of this kind counts as unread and may alert (keyword sound,
    /// flash, system notification): always, or only a person's while the
    /// option asks for it (#741).
    pub(crate) fn alerts(&self, kind: ChatKind) -> bool {
        !(self.quiet_automated_chat && kind.is_automated())
    }

    /// The nearby chat is read.
    pub fn nearby_read(&mut self) -> usize {
        self.chat_badge = 0;
        std::mem::take(&mut self.chat_unread)
    }

    /// Count on the Conversations button of the bottom bar: the unread IMs,
    /// group chats and conferences, like Firestorm's chiclet counters. Nearby
    /// chat keeps its own counter in the conversation list only, or a busy
    /// region would never let the button rest.
    pub fn conversations_badge(&self) -> usize {
        self.social.total_unread()
    }

    /// Ask for an avatar's profile (once).
    pub fn want_profile(&mut self, id: Uuid) {
        self.profiles.want(id);
    }

    /// Profiles to request now.
    pub fn take_profile_requests(&mut self) -> Vec<Uuid> {
        self.profiles.take_requests()
    }

    /// Picks, classifieds and friend rights changed from a profile window:
    /// the local copy follows before the command is sent.
    pub fn profile_command(&mut self, cmd: &aurora_net::NetCommand) {
        use aurora_net::NetCommand;
        match cmd {
            NetCommand::PickUpdate(p) => self.profiles.pick_saved(self.agent_id, (**p).clone()),
            NetCommand::PickDelete(id) => self.profiles.pick_deleted(self.agent_id, *id),
            NetCommand::ClassifiedDelete(id) => self.profiles.classified_deleted(self.agent_id, *id),
            NetCommand::ClassifiedUpdate(c) => self.profiles.classified_saved(self.agent_id, (**c).clone()),
            NetCommand::GrantUserRights { friend, rights } => {
                if let Some(f) = self.social.friends.iter_mut().find(|f| f.id == *friend) {
                    f.rights_given = *rights;
                }
                self.status.rights_set_by_hand(*friend, *rights);
            }
            _ => {}
        }
    }

    /// Contact set aliases into the name cache after a change (setPseudonym
    /// erases and fetches the name again in Firestorm).
    pub fn sync_contact_aliases(&mut self) {
        if self.aliases_synced == self.contact_sets.generation {
            return;
        }
        self.aliases_synced = self.contact_sets.generation;
        let aliases = self.contact_sets.pseudonyms().iter().map(|(id, a)| (*id, a.clone())).collect();
        self.social.avatar_names.set_aliases(aliases);
    }

    /// Save profile fields (the profile window): shown at once, sent with
    /// the returned command.
    pub fn save_profile(&mut self, target: Uuid, data: aurora_llsd::Llsd) -> aurora_net::NetCommand {
        self.profiles.apply_local(target, &data);
        aurora_net::NetCommand::UpdateProfile { target, data }
    }

    /// Save a group's identity (the group window): our copy follows before
    /// the command is sent, as LLPanelGroupGeneral::apply changes
    /// LLGroupMgr's data before sendUpdateGroupInfo, which expects no reply.
    pub fn save_group_info(&mut self, group: Uuid, info: aurora_net::groups::GroupInfo) {
        if let Some(g) = self.search.groups.get_mut(&group) {
            g.charter.clone_from(&info.charter);
            g.show_in_list = info.show_in_list;
            g.insignia = info.insignia;
            g.membership_fee = info.membership_fee;
            g.open_enrollment = info.open_enrollment;
            g.allow_publish = info.allow_publish;
            g.mature_publish = info.mature_publish;
        }
        self.follow_group_insignia(group, info.insignia);
        self.group_data.update_info(group, info);
    }

    /// A group's insignia, known from its profile or saved by us: our
    /// membership (conversations, group lists) and the profiles' group
    /// lists show it, not the one of the login's AgentGroupDataUpdate (the
    /// Legacy Viewer's group icons read LLGroupMgr's latest data).
    fn follow_group_insignia(&mut self, group: Uuid, insignia: Uuid) {
        if let Some(g) = self.groups.groups.iter_mut().find(|g| g.id == group) {
            g.insignia = insignia;
        }
        self.profiles.group_insignia(group, insignia);
    }

    pub fn system_message(&mut self, text: impl Into<String>) {
        self.system_notice(text, notice_topics::Topic::Info);
    }

    /// A notice of the viewer in the nearby chat, about `topic` (the icon
    /// of its compact row, #741).
    pub fn system_notice(&mut self, text: impl Into<String>, topic: notice_topics::Topic) {
        let line = ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: text.into(),
            kind: ChatKind::System,
            source: Uuid::nil(),
            origin: Origin::Live,
        };
        self.notice_topics.note(&line, topic);
        self.push_chat(line);
    }

    /// A notice of the viewer in a conversation (a call, an automatic
    /// reply), about `topic`, never unread.
    pub fn conversation_notice(&mut self, session: Uuid, text: impl Into<String>, topic: notice_topics::Topic) {
        let line = ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: text.into(),
            kind: ChatKind::System,
            source: Uuid::nil(),
            origin: Origin::Live,
        };
        self.notice_topics.note(&line, topic);
        self.conversation_push(session, line, false);
    }

    fn ensure_region(&mut self, h: RegionHandle, size: (u32, u32)) -> &mut Region {
        self.regions.entry(h).or_insert_with(|| Region {
            handle: h,
            name: String::new(),
            info: None,
            heightmap: Heightmap::new(size.0.max(256), size.1.max(256)),
            wind: Default::default(),
            dirty_chunks: HashSet::new(),
            terrain_revision: 0,
            terrain_surface: Default::default(),
            caps: Arc::new(HashMap::new()),
            voice_server: String::new(),
            avatar_hover_height: false,
            lsl_syntax_id: None,
            materials_limits: Default::default(),
            max_tasks: 0,
            grid_status: Default::default(),
        })
    }

    /// Visual parameter values (shape) of an avatar's last appearance.
    pub fn visual_params(&self, avatar: &Uuid) -> Option<&[u8]> {
        if *avatar == self.agent_id
            && let Some(values) = &self.shape_preview
        {
            return Some(values);
        }
        self.appearances.get(avatar).map(|(a, _)| a.visual_params.as_slice())
    }

    /// A draft never replaces the authoritative appearance received meanwhile.
    pub fn preview_shape(&mut self, values: Option<Vec<u8>>) {
        if self.shape_preview != values {
            self.shape_preview = values;
            self.shape_preview_generation = self.shape_preview_generation.wrapping_add(1);
        }
    }

    pub fn server_visual_params(&self) -> &[u8] {
        self.appearances
            .get(&self.agent_id)
            .map_or(&[], |(a, _)| a.visual_params.as_slice())
    }

    pub fn server_appearance(&self) -> Option<&AvatarAppearance> {
        self.appearances.get(&self.agent_id).map(|(appearance, _)| appearance)
    }

    /// Hover height of an avatar's last appearance (m).
    pub fn appearance_hover(&self, avatar: &Uuid) -> f32 {
        if *avatar == self.agent_id {
            return if self.hover_height_available() {
                self.hover_height.effective()
            } else {
                0.0
            };
        }
        self.appearances
            .get(avatar)
            .map(|(a, _)| a.hover_height)
            .filter(|h| h.is_finite())
            .unwrap_or(0.0)
            .clamp(hover_height::MIN, hover_height::MAX)
    }

    pub fn hover_height_available(&self) -> bool {
        // LLFloaterHoverHeight::updateEditEnabled gates local editing on the
        // feature, independently of sendHoverHeight's capability lookup.
        self.main().is_some_and(|region| region.avatar_hover_height)
    }

    pub fn hover_height_target(&self) -> Option<RegionHandle> {
        self.main()
            .filter(|region| region.avatar_hover_height && region.caps.contains_key("AgentPreferences"))
            .map(|region| region.handle)
    }

    /// COF version of the last appearance received for an avatar.
    pub fn appearance_cof_version(&self, avatar: &Uuid) -> i32 {
        self.appearances.get(avatar).map(|(a, _)| a.cof_version).unwrap_or(-1)
    }

    /// Worn objects the simulator listed in the avatar's last appearance
    /// (LLVOAvatar::mSimAttachments); 0 when it listed none.
    pub fn announced_attachments(&self, avatar: &Uuid) -> usize {
        self.appearances.get(avatar).map(|(a, _)| a.attachments as usize).unwrap_or(0)
    }

    /// Inventory items attached to our avatar (AttachItemID name-values).
    pub fn worn_attachment_items(&self) -> HashSet<Uuid> {
        let Some(idx) = self.objects.index_of_uuid(&self.agent_id) else {
            return HashSet::new();
        };
        let Some(me) = self.objects.get(idx) else {
            return HashSet::new();
        };
        self.objects
            .children_of(&me.key)
            .iter()
            .filter_map(|i| self.objects.get(*i))
            .filter_map(|o| o.attachment_item_id())
            .filter(|id| !id.is_nil())
            .collect()
    }

    /// The root prim of the object our avatar wears from inventory item
    /// `item` (LLVOAvatar::getWornAttachment).
    pub fn worn_attachment(&self, item: &Uuid) -> Option<usize> {
        if item.is_nil() {
            return None;
        }
        let me = self.objects.get(self.objects.index_of_uuid(&self.agent_id)?)?;
        self.objects
            .children_of(&me.key)
            .iter()
            .copied()
            .find(|i| self.objects.get(*i).and_then(|o| o.attachment_item_id()) == Some(*item))
    }

    /// Worn items whose object handles touches (enable_attachment_touch:
    /// flagHandleTouch of the attachment's root).
    pub fn touchable_attachment_items(&self) -> HashSet<Uuid> {
        let Some(me) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return HashSet::new();
        };
        self.objects
            .children_of(&me.key)
            .iter()
            .filter_map(|i| self.objects.get(*i))
            .filter(|o| o.update_flags & crate::ui::context::flags::HANDLE_TOUCH != 0)
            .filter_map(|o| o.attachment_item_id())
            .filter(|id| !id.is_nil())
            .collect()
    }

    pub fn worn_attachment_points(&self) -> HashMap<Uuid, u8> {
        self.objects
            .index_of_uuid(&self.agent_id)
            .and_then(|idx| self.objects.get(idx))
            .map(|me| {
                self.objects
                    .children_of(&me.key)
                    .iter()
                    .filter_map(|idx| self.objects.get(*idx))
                    .filter_map(|o| {
                        o.attachment_item_id()
                            .filter(|id| !id.is_nil())
                            .map(|id| (id, o.attachment_point()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Run the outfit restore and the wears waiting for their items; its
    /// network commands are left in `self.outfit.commands`.
    pub fn update_outfit(&mut self, second_life: bool, now: Instant) {
        if self.outfit.is_idle() {
            return;
        }
        let ready = self.demo_outfit_region
            || self.movement_complete && self.main().is_some_and(|r| r.caps.contains_key("FetchInventoryDescendents2"));
        let first_login = self
            .login
            .as_ref()
            .is_some_and(|l| l.raw["login-flags"].at(0)["ever_logged_in"].as_str() == "N");
        let inputs = outfit::OutfitInputs {
            worn: self.outfit_attachment_items(),
            received_cof_version: self.appearance_cof_version(&self.agent_id),
            inventory: &mut self.inventory,
            agent: self.agent_id,
            ready,
            first_login,
            second_life,
        };
        self.outfit.update(inputs, now);
    }

    /// Tells the outfit restore which worn objects are taken off on
    /// purpose (a detach sent by the resident, a script or RLVa).
    pub fn note_outfit_command(&mut self, command: &aurora_net::NetCommand) {
        match command {
            aurora_net::NetCommand::DetachAttachments(items) => {
                self.outfit.on_detach_requested(items);
                self.attachment_inventory.detaching(items);
            }
            aurora_net::NetCommand::DetachObject { handle, local_id } => {
                if let Some(item) = self
                    .objects
                    .index_of(&objects::ObjKey {
                        region: *handle,
                        local_id: *local_id,
                    })
                    .and_then(|i| self.objects.get(i))
                    .and_then(|o| o.attachment_item_id())
                {
                    self.outfit.on_detach_requested(&[item]);
                    self.attachment_inventory.detaching(&[item]);
                }
            }
            _ => {}
        }
    }

    /// "Force appearance update (rebake)": the server side (see
    /// `OutfitRestore::rebake`); its commands are left in
    /// `self.outfit.commands`, the textures are the scene's.
    pub fn rebake(&mut self) {
        let step = self.main().is_some_and(|r| r.caps.contains_key("IncrementCOFVersion"));
        let received = self.appearance_cof_version(&self.agent_id);
        self.outfit.rebake(&self.inventory, step, received);
    }

    pub fn appearance_generation(&self, avatar: &Uuid) -> u64 {
        self.appearances.get(avatar).map(|(_, g)| *g).unwrap_or(0)
            + self.bake_refreshes.get(avatar).copied().unwrap_or(0)
            + if *avatar == self.agent_id {
                self.shape_preview_generation
            } else {
                0
            }
    }

    /// A bake the scene held back while it loaded shows now
    /// (scene/shown_bakes.rs): the avatar and what shows its bakes are
    /// built again, as when an appearance comes.
    pub fn refresh_bakes(&mut self, avatar: &Uuid) {
        *self.bake_refreshes.entry(*avatar).or_default() += 1;
        if let Some(idx) = self.objects.index_of_uuid(avatar) {
            if let Some(o) = self.objects.get(idx) {
                let kids: Vec<usize> = self.objects.children_of(&o.key).to_vec();
                for k in kids {
                    self.mark_tree_material_dirty(k);
                }
            }
            if let Some(o) = self.objects.get_mut(idx) {
                o.material_dirty = true;
            }
        }
    }

    /// Publish one complete local editor generation without changing the
    /// authoritative appearance received from the server.
    pub fn preview_bakes(&mut self, bakes: HashMap<usize, Uuid>) {
        if self.preview_bakes != bakes {
            self.preview_bakes = bakes;
            let agent = self.agent_id;
            self.refresh_bakes(&agent);
        }
    }

    /// Baked texture id for an avatar (by object index) and TE slot.
    pub fn bake_texture(&self, avatar_idx: usize, te_index: usize) -> Option<Uuid> {
        let o = self.objects.get(avatar_idx)?;
        if o.full_id == self.agent_id
            && let Some(id) = self.preview_bakes.get(&te_index)
        {
            return Some(*id);
        }
        self.server_bake_texture(avatar_idx, te_index)
    }

    pub fn server_bake_texture(&self, avatar_idx: usize, te_index: usize) -> Option<Uuid> {
        let o = self.objects.get(avatar_idx)?;
        if let Some((a, _)) = self.appearances.get(&o.full_id)
            && let Some(te) = &a.texture_entry
        {
            return te.faces.get(te_index).map(|f| f.texture);
        }
        o.te.as_ref().and_then(|te| te.faces.get(te_index).map(|f| f.texture))
    }

    pub fn bake_source(&self, avatar: Uuid, bake: &str, tex: Uuid) -> TexSource {
        match self.login.as_ref().map(|l| l.agent_appearance_service.as_str()) {
            Some(base) if !base.is_empty() => {
                let base = if base.ends_with('/') { base.to_owned() } else { format!("{base}/") };
                TexSource::Bake {
                    url: format!("{base}texture/{avatar}/{bake}/{tex}"),
                }
            }
            _ => TexSource::Asset,
        }
    }

    /// Apply one network event. Returns events the app layer must handle.
    pub fn apply(&mut self, ev: NetEvent) -> Option<NetEvent> {
        self.received = None;
        self.apply_event(ev)
    }

    /// `apply` of an event the network read at `received`: the motion of
    /// the objects and of our avatar it reports starts from that moment,
    /// not from the frame that got to it (item 6 of the network audit;
    /// Firestorm takes the frame time, `mLastInterpUpdateSecs`).
    pub fn apply_received(&mut self, ev: NetEvent, received: Instant) -> Option<NetEvent> {
        self.received = Some(received);
        let out = self.apply_event(ev);
        self.received = None;
        out
    }

    fn apply_event(&mut self, ev: NetEvent) -> Option<NetEvent> {
        self.events_this_frame += 1;
        let mut ev = self.hold_for_parked(ev)?;
        if self.anti_spam_event(&mut ev) {
            return None;
        }
        match ev {
            NetEvent::LoggedIn(l) => {
                self.agent_id = l.agent_id;
                self.anti_spam.reset();
                // a new session: no script holds a control yet
                self.script_controls = Default::default();
                // the account is known: its transcripts open
                self.chat_log.login(l.agent_id);
                // LLVOAvatar::getFullname, sent when an offline friendship
                // offer is accepted
                self.notifications.own_name = format!("{} {}", l.first_name, l.last_name);
                self.agent.position = Vec3::new(128.0, 128.0, 30.0);
                self.agent.set_look_at(l.look_at);
                self.social = social::Social::from_login(&l.raw);
                let account = crate::settings::account_dir(&l.agent_id);
                self.contact_sets = contact_sets::ContactSets::load(account.join(contact_sets::CONTACT_SETS_FILE));
                self.groups.load_favorites(account.join(groups::FAVORITE_GROUPS_FILE));
                self.group_data.load_read(account.join(group_data::NOTICES_READ_FILE));
                self.status.invisible = status::Invisible::load(account.join(status::INVISIBLE_FILE));
                let hidden = self.status.resume_invisible(&mut self.social, &mut self.groups.out);
                if hidden > 0 {
                    log::info!("invisible: hidden again from {hidden} friend(s) the login still showed us to");
                }
                let open = account.join(open_ims::FILE);
                self.reopen_ims = open_ims::load(&open);
                self.open_ims_path = Some(open);
                self.calls.load_volumes(account.join(calls::VOLUMES_FILE));
                self.tp_storage = tphistory::HistoryStorage::load(account.join(tphistory::TELEPORT_HISTORY_FILE));
                self.hud_scales = hud_scale::HudScales::load(account.join(hud_scale::FILE));
                self.hud_locks = hud_lock::HudLocks::load(account.join(hud_lock::FILE));
                self.derender = derender::Derender::load(account.join(derender::FILE));
                self.media_filter = crate::media::filter::MediaFilter::load(account.join(crate::media::filter::FILE));
                self.aliases_synced = u64::MAX;
                self.sync_contact_aliases();
                self.inventory = inventory::Inventory::from_login(&l.raw);
                self.inventory.owner = self.agent_id;
                self.inventory_background = Default::default();
                // read off the main thread; until `poll_inventory_cache`
                // applies it, folder requests wait (the root's included)
                self.inventory_cache = self.inventory_cache_path().map(|path| {
                    self.inventory.hold_requests();
                    (inventory::read_cache_in_background(path), Instant::now())
                });
                let root = self.inventory.root;
                self.inventory.request(root);
                self.map.set_home_from_login(l.raw["home"].as_str());
                self.login = Some(l.clone());
                Some(NetEvent::LoggedIn(l))
            }
            NetEvent::MainRegionChanged { handle } => {
                if self.set_main_region(handle) {
                    // no agent update before the new region completes the move
                    self.movement_complete = false;
                    // LLViewerParcelMgr::onTeleportFinished: the new region's
                    // parcel is awaited; the one left must not lend its rules
                    // (flight, build, the bar's icons) in the meantime. The
                    // session sends it after this event, never before.
                    if self.parcel.take().is_some() {
                        log::info!("agent parcel: cleared, region {handle:x} entered");
                    }
                    self.parcel_name.clear();
                }
                // back in a region left moments ago: its objects first, its
                // simulator only sends what changed since
                self.unpark_region(handle);
                None
            }
            NetEvent::RegionHandshake(info) => {
                self.tp_history.name_current(info.handle, &info.name);
                let r = self.ensure_region(info.handle, (info.size_x, info.size_y));
                r.name = info.name.clone();
                if r.info.as_ref().is_some_and(|old| {
                    old.terrain_start_height != info.terrain_start_height || old.terrain_height_range != info.terrain_height_range
                }) {
                    r.dirty_chunks.extend(0..r.heightmap.chunk_count());
                }
                r.terrain_surface.applied = None;
                r.info = Some(info);
                None
            }
            NetEvent::Capabilities { handle, caps } => {
                self.ensure_region(handle, (256, 256)).caps = caps;
                // the folders that failed without them (a crossing, a
                // teleport) are asked again on the agent's new region
                if Some(handle) == self.main_region {
                    self.inventory.retry_failed();
                    self.inventory_background.resume(&self.inventory);
                }
                None
            }
            NetEvent::SimulatorFeatures {
                handle,
                voice_server_type,
                avatar_hover_height,
                materials_rate,
                materials_max,
                lsl_syntax_id,
                grid_status_feed,
                grid_status_page,
            } => {
                let r = self.ensure_region(handle, (256, 256));
                r.lsl_syntax_id = Some(lsl_syntax_id);
                r.grid_status = (grid_status_feed, grid_status_page);
                r.voice_server = voice_server_type;
                r.avatar_hover_height = avatar_hover_height;
                r.materials_limits = crate::scene::legacy_mat::RegionLimits::from_features(materials_rate, materials_max);
                None
            }
            NetEvent::HoverHeightUpdated { region, request, success } => {
                self.hover_height.reply(request, region, success);
                None
            }
            NetEvent::MaturityPreference(result) => {
                self.on_maturity_answer(result);
                None
            }
            NetEvent::HomeLocation(reply) => {
                self.on_home_location(reply);
                None
            }
            NetEvent::HomeAlert(alert) => {
                self.on_home_alert(alert);
                None
            }
            NetEvent::UserInfo(result) => {
                self.user_info.answer(result);
                None
            }
            NetEvent::UserInfoUpdateFailed(error) => {
                self.on_user_info_update_failed(error);
                None
            }
            NetEvent::LslSyntax { id, keywords } => {
                self.on_lsl_syntax(id, keywords);
                None
            }
            NetEvent::SimConsoleReply(reply) => {
                self.sim_console.reply(reply);
                None
            }
            NetEvent::VoiceModerationFailed { everyone } => {
                // messageHttpPost's failure_msg is only logged; a notice
                // says the click did nothing
                let text = if everyone {
                    aurora_i18n::tr!("world-mod-voice-moderation-all-failed")
                } else {
                    aurora_i18n::tr!("world-mod-voice-moderation-failed")
                };
                self.notifications.push(
                    notifications::Kind::Alert,
                    aurora_i18n::tr!("world-mod-voice-moderation"),
                    text.to_owned(),
                    notifications::Data::None,
                );
                None
            }
            NetEvent::AgentMovementComplete { handle, position, look_at } => {
                let region_changed = Some(handle) != self.main_region;
                log::info!(
                    "arrival: region {handle:x}{} position {position:.1?} (teleporting {}, first {})",
                    if region_changed { " (new)" } else { "" },
                    self.teleporting,
                    !self.arrived_once
                );
                // process_agent_movement_complete: only a teleport (or the
                // login) places the agent, turns it and stops it. Walking or
                // flying into a neighbour, it goes on where it is; an agent
                // the simulator puts far from there was moved all the same.
                let crossed = !self.teleporting && self.arrived_once;
                if region_changed {
                    self.set_main_region(handle);
                }
                self.crossing = false;
                self.arrived_by_crossing = crossed && self.agent.position.distance(position) < CROSSING_MAX_GAP;
                if !self.arrived_by_crossing {
                    self.agent.position = position;
                    self.agent.set_look_at(look_at);
                    self.agent.reset_motion();
                }
                // The coarse locations were sent around where the agent was.
                // The region left keeps its last list until its simulator
                // closes it (about 50 s), and the Legacy Viewer keeps showing
                // those avatars as present (LLWorld::getAvatars): zoom and
                // teleport aimed regions away (issue #1000). After a teleport
                // to another region (MainRegionChanged comes first) only the
                // arrival's list stays; the simulators still connected send
                // theirs again.
                if self.region_left.take().is_some_and(|left| left != handle) && !self.arrived_by_crossing {
                    self.coarse.retain(|h, _| *h == handle);
                }
                self.movement_complete = true;
                if self.teleporting || !self.arrived_once {
                    let region = if self.rlva.has("showloc") {
                        aurora_i18n::tr!("rlva-hidden-region").to_owned()
                    } else {
                        self.regions.get(&handle).map(|r| r.name.clone()).unwrap_or_default()
                    };
                    let at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64);
                    self.tp_history.arrived(tphistory::TpEntry {
                        region,
                        handle,
                        position,
                        parcel: String::new(),
                        at,
                    });
                    // LLViewerParcelMgr::onTeleportFinished: a local teleport keeps
                    // the agent parcel (the simulator only sends ParcelProperties
                    // when the parcel changes)
                    if region_changed || !self.arrived_once {
                        self.parcel = None;
                        self.parcel_name.clear();
                    }
                    let (ox, oy) = aurora_net::handle_to_origin(handle);
                    let global = glam::DVec3::new(ox as f64 + position.x as f64, oy as f64 + position.y as f64, position.z as f64);
                    // saved when the new parcel arrives; a local teleport
                    // within the same parcel gets none: flush_arrival
                    self.tp_record = Some((global, Instant::now()));
                }
                if self.teleporting {
                    self.tp_progress = 0.6;
                    self.tp_status = aurora_i18n::tr!("common-loading-region").into();
                }
                self.arrived_once = true;
                self.teleporting = false;
                self.park_far_regions();
                self.land_if_fly_blocked(handle);
                self.friend_join_flight_arrived(handle);
                self.friend_join_arrived(handle);
                Some(NetEvent::AgentMovementComplete { handle, position, look_at })
            }
            NetEvent::ObjectUpdates { handle, objects } => {
                let me = self.agent_id;
                // "About Land" open on a parcel of this region: a
                // prim that was not there changes its counts
                let mut land_changed = false;
                let land_watches = self.land.watches(handle);
                for u in objects {
                    if let Some(dropped) = self.derender.filter(handle, &u) {
                        if dropped.newly {
                            self.hide_key(dropped.key, dropped.by);
                        }
                        continue;
                    }
                    land_changed |= land_watches
                        && u.pcode == aurora_prim::params::LL_PCODE_VOLUME
                        && self
                            .objects
                            .index_of(&ObjKey {
                                region: handle,
                                local_id: u.local_id,
                            })
                            .is_none();
                    // diagnostic: bakes-on-mesh textures on something that is
                    // not a mesh (shown as its base prim shape)
                    if u.pcode == aurora_prim::params::LL_PCODE_VOLUME
                        && !u.volume.is_mesh()
                        && u.texture_entry
                            .as_ref()
                            .is_some_and(|te| te.faces.iter().any(|f| crate::scene::avatar::bake_slot_for(&f.texture).is_some()))
                        && self.diag_reported.insert(u.full_id)
                    {
                        log::warn!(
                            "object {} (parent {}, attach state {}) uses bake textures but has no mesh params: sculpt {:?}, extra {:?}, path {} profile {}",
                            u.full_id,
                            u.parent_id,
                            u.state,
                            u.extra.sculpt,
                            u.extra,
                            u.volume.path.curve_type,
                            u.volume.profile.curve_type
                        );
                    }
                    let is_me = u.full_id == me;
                    let avatar_id = u.full_id;
                    let pos = u.position;
                    let velocity = u.velocity;
                    let acceleration = u.acceleration;
                    let parent = u.parent_id;
                    if is_me {
                        self.on_seat_parent_change(handle, parent);
                    }
                    if u.parent_id == 0
                        && Some(handle) == self.main_region
                        && !self.teleporting
                        && let Some(idx) = self.objects.index_of_uuid(&u.full_id)
                    {
                        self.unregister_attachment(idx);
                    }
                    self.on_camera_parent_change(handle, &u);
                    let foot_plane = u.foot_plane;
                    self.objects.upsert_at(handle, u, self.received);
                    self.note_friend_join_object(handle, avatar_id, foot_plane);
                    if is_me {
                        let at = self.agent_report_time();
                        self.agent
                            .on_server_update_at(pos, velocity, acceleration, parent != 0, Some(handle) == self.main_region, at);
                    }
                }
                self.move_rekeyed();
                self.retry_ik_declarations(Instant::now());
                if land_changed {
                    self.land.objects_changed(handle, Instant::now());
                }
                None
            }
            NetEvent::TerseUpdates { handle, updates } => {
                let me = self.agent_id;
                for t in updates {
                    let key = ObjKey {
                        region: handle,
                        local_id: t.local_id,
                    };
                    if let Some(idx) = self.objects.index_of(&key) {
                        let mut mine = false;
                        let mut seated = false;
                        self.objects.apply_terse_at(idx, &t, self.received);
                        if let Some(o) = self.objects.get(idx) {
                            mine = o.full_id == me;
                            seated = o.parent_id != 0;
                            let id = o.full_id;
                            self.note_friend_join_object(handle, id, t.foot_plane);
                        }
                        if mine {
                            let at = self.agent_report_time();
                            self.agent.on_server_update_at(
                                t.position,
                                t.velocity,
                                t.acceleration,
                                seated,
                                Some(handle) == self.main_region,
                                at,
                            );
                        }
                    }
                }
                None
            }
            NetEvent::SetFollowCam {
                handle,
                object,
                properties,
            } => {
                if let Some(origin) = self.region_offset(handle) {
                    self.scripted_camera.set(object, handle, &properties, origin);
                }
                None
            }
            NetEvent::ClearFollowCam { object } => {
                self.scripted_camera.clear(object);
                None
            }
            NetEvent::ObjectsKilled { handle, local_ids } => {
                let local_ids = self.hold_kills(handle, local_ids);
                self.kill_objects(handle, &local_ids);
                None
            }
            NetEvent::GltfOverrides { handle, local_id, sides } => {
                let key = ObjKey { region: handle, local_id };
                if sides.is_empty() {
                    self.gltf_overrides.remove(&key);
                } else {
                    let sides: Arc<[_]> = sides
                        .iter()
                        .map(|(face, od)| (*face, aurora_assets::material::PbrOverride::from_llsd(od)))
                        .collect();
                    self.gltf_overrides.insert(key, sides);
                }
                if let Some(o) = self.objects.index_of(&key).and_then(|i| self.objects.get_mut(i)) {
                    o.material_dirty = true;
                    o.render.needs_records = true;
                }
                None
            }
            NetEvent::Terrain { handle, patches } => {
                let r = self.ensure_region(handle, (256, 256));
                for p in patches {
                    let dirty = r.heightmap.apply_patch(p.x, p.y, p.size, &p.heights);
                    r.dirty_chunks.extend(dirty);
                }
                r.terrain_revision += 1;
                None
            }
            NetEvent::Wind { handle, grid } => {
                self.ensure_region(handle, (256, 256)).wind = *grid;
                None
            }
            NetEvent::Chat(c) => {
                self.on_chat(c);
                None
            }
            // process_chat_from_simulator: an avatar not built yet (the
            // login) is skipped; never our own, echoed back
            NetEvent::NearbyTyping { avatar, typing } => {
                if !typing {
                    self.nearby_typing.remove(&avatar);
                } else if avatar != self.agent_id && self.objects.index_of_uuid(&avatar).is_some() {
                    self.nearby_typing.insert(avatar);
                }
                None
            }
            NetEvent::MuteList(src) => {
                self.on_mute_list(src);
                None
            }
            NetEvent::FriendshipTerminated(id) => {
                if self.social.end_friendship(&id) {
                    log::info!("friendship ended by {id}");
                }
                None
            }
            NetEvent::CallingCardOffered { from, transaction, handle } => {
                self.on_calling_card_offer(from, transaction, handle);
                None
            }
            NetEvent::CallingCardAnswered { accepted } => {
                self.on_calling_card_answer(accepted);
                None
            }
            ev @ (NetEvent::Groups(_)
            | NetEvent::ActiveGroup { .. }
            | NetEvent::GroupDropped(_)
            | NetEvent::SessionInvite(_)
            | NetEvent::SessionStarted { .. }
            | NetEvent::SessionAgents { .. }
            | NetEvent::SessionError { .. }
            | NetEvent::SessionClosed { .. }
            | NetEvent::ChatSessionReply { .. }) => {
                match &ev {
                    NetEvent::SessionStarted {
                        temp_session,
                        session,
                        success,
                        ..
                    } => self.call_session_started(*temp_session, *session, *success),
                    NetEvent::SessionClosed { session, .. } => self.call_session_closed(*session),
                    _ => {}
                }
                self.on_group_event(ev)
            }
            ev @ (NetEvent::VoiceInvite(_) | NetEvent::CallReply { .. } | NetEvent::SessionVoiceChannel { .. }) => {
                self.on_call_event(ev);
                None
            }
            NetEvent::InstantMessage(im) => {
                self.on_im(im);
                None
            }
            NetEvent::OfflineMessagesFailed => {
                // requestOfflineMessagesCoro: "Falling back to legacy method."
                self.groups.out.push(aurora_net::NetCommand::RequestOfflineMessagesLegacy);
                None
            }
            NetEvent::OfflineGroupInviteAccepted(_) => {
                // response_group_invitation_coro: sendAgentDataUpdateRequest
                self.groups.out.push(aurora_net::NetCommand::RequestGroups);
                None
            }
            NetEvent::Appearance(a) => {
                let id = a.avatar_id;
                let own = id == self.agent_id;
                let received = if own {
                    self.appearance_cof_version(&id).max(self.outfit.stepped_cof_version())
                } else {
                    -1
                };
                match appearance_verdict(&a, own, received) {
                    AppearanceVerdict::Apply => {}
                    AppearanceVerdict::Stale => {
                        log::warn!(
                            "stale own appearance received: COF version {} after {received}, dropped",
                            a.cof_version
                        );
                        return None;
                    }
                    AppearanceVerdict::Empty { refresh } => {
                        // no reliable basis to know where the bakes are
                        if refresh {
                            log::info!("empty own appearance: forcing a refresh");
                            self.notifications.push(
                                notifications::Kind::Info,
                                aurora_i18n::tr!("common-appearance"),
                                aurora_i18n::tr!("world-appearance-cloud-fix"),
                                notifications::Data::None,
                            );
                            self.rebake();
                        } else {
                            log::debug!("appearance of {id} without parameters ignored");
                        }
                        return None;
                    }
                }
                self.appearance_counter += 1;
                if own {
                    // (with the outfit restore lines: when our bakes came)
                    log::info!(
                        "own appearance received: COF version {}, bakes {}",
                        a.cof_version,
                        if a.texture_entry.is_some() { "present" } else { "absent" }
                    );
                }
                self.appearances.insert(id, (a, self.appearance_counter));
                // Attachments using bakes-on-mesh must refresh too.
                if let Some(idx) = self.objects.index_of_uuid(&id) {
                    if let Some(o) = self.objects.get(idx) {
                        let key = o.key;
                        let kids: Vec<usize> = self.objects.children_of(&key).to_vec();
                        for k in kids {
                            self.mark_tree_material_dirty(k);
                        }
                    }
                    if let Some(o) = self.objects.get_mut(idx) {
                        o.material_dirty = true;
                    }
                }
                None
            }
            NetEvent::Sun(s) => {
                self.sun = Some((s, Instant::now()));
                None
            }
            NetEvent::CoarseLocations { handle, positions } => {
                self.note_friend_join_coarse(handle, &positions);
                // LLViewerRegion::processCoarseUpdate updates the followed
                // friend's position, including the coarse (4 m) altitude.
                if let Some(a) = &self.map.avatar
                    && let Some((_, pos)) = positions.iter().find(|(id, _)| *id == a.id)
                {
                    // Full object updates retain exact height above the
                    // coarse protocol's 1,020 m ceiling.
                    let pos = self
                        .objects
                        .index_of_uuid(&a.id)
                        .and_then(|i| self.objects.get(i))
                        .filter(|o| o.key.region == handle)
                        .map_or(*pos, |o| o.position);
                    let (ox, oy) = aurora_net::handle_to_origin(handle);
                    self.map
                        .agent_found(a.id, ox as f64 + pos.x as f64, oy as f64 + pos.y as f64, Some(pos.z));
                }
                self.coarse.insert(handle, positions);
                None
            }
            NetEvent::RegionRemoved { handle } => {
                self.parked.remove(&handle);
                self.remove_region_objects(handle);
                self.regions.remove(&handle);
                self.coarse.remove(&handle);
                None
            }
            NetEvent::TeleportProgress { message } => {
                if !self.teleporting {
                    self.tp_show_progress = false;
                }
                // LL teleport_strings.xml "progress" keys
                let text = match message.as_str() {
                    "sending_dest" | "sending_home" | "sending_landmark" => aurora_i18n::tr!("common-sending-to-destination"),
                    "redirecting" | "relaying" => aurora_i18n::tr!("world-mod-redirecting-to-another-location"),
                    "resolving" => aurora_i18n::tr!("world-mod-searching-for-destination"),
                    "contacting" => aurora_i18n::tr!("common-contacting-the-new-region"),
                    "requesting" => aurora_i18n::tr!("world-mod-requesting-teleport"),
                    "completing" | "completed_from" => aurora_i18n::tr!("world-mod-finishing-teleport"),
                    "arriving" => aurora_i18n::tr!("world-mod-arriving"),
                    "pending" => aurora_i18n::tr!("world-mod-waiting-for-server"),
                    _ => aurora_i18n::tr!("world-mod-teleporting"),
                };
                self.teleporting = true;
                self.tp_progress = (self.tp_progress + 0.06).clamp(0.15, 0.42);
                self.tp_status = text.into();
                None
            }
            NetEvent::TeleportFinished { handle, flying } => {
                self.teleporting = true;
                let remote = Some(handle) != self.main_region;
                // TeleportStart has no destination. Unlike Firestorm's
                // process_teleport_start, defer its sound until a region change
                // is confirmed, preserving silence for local script/lure TPs.
                // A known remote request already played it in begin_teleport.
                if remote && !self.tp_show_progress {
                    self.ui_sounds.push(UiSound::TeleportOut);
                }
                self.tp_show_progress = remote;
                self.tp_progress = self.tp_progress.max(0.5);
                self.tp_status = aurora_i18n::tr!("common-connecting-to-region").into();
                // Legacy Viewer process_teleport_finish: apply both states,
                // including landing. Destination restrictions still take effect
                // on arrival (land_if_fly_blocked).
                self.agent.flying = flying || self.fly_after_teleport;
                Some(NetEvent::TeleportFinished { handle, flying })
            }
            NetEvent::TeleportLocal { flying } => {
                if !self.teleporting {
                    self.cancel_friend_join_flight();
                    self.cancel_friend_join();
                    self.map.cancel_avatar_teleport();
                }
                // process_teleport_local, indra/newview/llviewermessage.cpp.
                // This also covers server-initiated teleports: the following
                // AgentMovementComplete must record the arrival in history.
                self.teleporting = true;
                self.tp_show_progress = false;
                // A friend hop preserves existing flight here, but defers
                // taking off (even if flagged by the server) until its
                // arrival position has been applied.
                self.agent.flying = self.friend_join_local_flight(flying).unwrap_or(flying || self.fly_after_teleport);
                None
            }
            NetEvent::TeleportStarted => {
                if !self.teleporting {
                    self.cancel_friend_join_flight();
                    self.cancel_friend_join();
                    self.map.cancel_avatar_teleport();
                    self.tp_show_progress = false;
                    log::info!("TeleportStart (arrived {})", self.arrived_once);
                }
                self.teleporting = true;
                self.tp_failed = false;
                self.tp_progress = self.tp_progress.max(0.10);
                self.tp_status = aurora_i18n::tr!("common-teleport-accepted").into();
                self.system_notice(
                    aurora_i18n::tr!("world-mod-teleporting"),
                    crate::world::notice_topics::Topic::Teleport,
                );
                None
            }
            NetEvent::TeleportFailed { reason } => {
                self.cancel_friend_join_flight();
                self.cancel_friend_join();
                self.tp_history.failed();
                self.teleporting = false;
                self.tp_failed = true;
                self.system_notice(
                    aurora_i18n::tr!("world-mod-teleport-failed", reason = reason.to_string()),
                    crate::world::notice_topics::Topic::Teleport,
                );
                None
            }
            NetEvent::ScriptControlChange(blocks) => {
                for (take, controls, pass_on) in blocks {
                    self.script_controls.apply(take, controls, pass_on);
                }
                None
            }
            NetEvent::Alert { message } => {
                self.map.on_alert(&message);
                let restart = message.to_lowercase().contains("restart");
                self.ui_sounds.push(if restart { UiSound::Restart } else { UiSound::Alert });
                self.system_message(message.clone());
                let id = self.notifications.push(
                    notifications::Kind::Alert,
                    aurora_i18n::tr!("world-mod-simulator-alert"),
                    message.clone(),
                    notifications::Data::None,
                );
                // SystemMessageTip (process_alert_core)
                let notice = self.notice_of(id, attention::Open::Notifications);
                self.ask_attention(attention::Cause::Notice, notice);
                self.alerts.push_back(message);
                None
            }
            NetEvent::ScriptDialog {
                object_id, object_name, ..
            } if self.object_blocked(&object_id, &object_name) => None,
            // RLVa @sendchat (LLScriptHandler::processNotification,
            // llnotificationscripthandler.cpp): a dialog or text box whose
            // answer would be said on public chat is not shown at all.
            NetEvent::ScriptDialog { channel: 0, .. } if self.rlva.has("sendchat") => {
                self.system_notice(aurora_i18n::tr!("rlva-blocked-script-dialog"), notice_topics::Topic::Warning);
                None
            }
            // process_script_question drops only the questions of a blocked
            // object (LLMuteList::isMuted(taskid)): not those of a blocked
            // owner, nor on any other condition (no click needed, object out
            // of view or unknown, another region, Do Not Disturb)
            NetEvent::ScriptQuestion { task_id, .. } if self.mutes.is_muted_id(&task_id) => None,
            NetEvent::LoadUrl {
                object_id, object_name, ..
            } if self.object_blocked(&object_id, &object_name) => None,
            NetEvent::ScriptDialog {
                sender,
                object_id,
                object_name,
                owner_name,
                message,
                channel,
                buttons,
            } => {
                let textbox = buttons.first().is_some_and(|b| b == "!!llTextBox!!");
                let data = if textbox {
                    notifications::Data::TextBox {
                        sender,
                        object: object_id,
                        channel,
                    }
                } else {
                    let buttons = if buttons.is_empty() { vec!["OK".to_owned()] } else { buttons };
                    notifications::Data::Dialog {
                        sender,
                        object: object_id,
                        object_name: object_name.clone(),
                        own_object: self
                            .objects
                            .index_of_uuid(&object_id)
                            .and_then(|idx| self.objects.get(idx))
                            .is_some_and(|o| o.owner_id == self.agent_id),
                        channel,
                        buttons,
                    }
                };
                let title = format!("{object_name} ({owner_name})");
                let id = self.notifications.push(notifications::Kind::Script, title, message, data);
                self.link_object_notification(id, object_id, &object_name, &owner_name);
                // LLScriptFloater ("script_floater") opening
                self.ui_sounds.push(UiSound::ScriptFloaterOpen);
                // FSFlashOnScriptDialog and the ScriptDialog notification
                // (LLScriptFloaterManager::onAddNotification)
                let notice = self.notice_of(id, attention::Open::Notifications);
                self.ask_attention(attention::Cause::ScriptDialog, notice);
                None
            }
            NetEvent::ScriptQuestion {
                task_id,
                item_id,
                object_name,
                owner_name,
                questions,
                experience,
                region,
            } => {
                let (questions, auto_accept) = self.rlva_script_permissions(task_id, questions);
                if auto_accept && experience.is_nil() {
                    self.rlva.out.push(aurora_net::NetCommand::ScriptAnswer {
                        task_id,
                        item_id,
                        questions,
                        region,
                    });
                    return None;
                }
                // process_script_question: an experience's question waits
                // for the experience's details (LLExperienceCache::get), a
                // caution permission keeps the ordinary dialog
                if !experience.is_nil() && !experiences::caution(questions) {
                    let question = experiences::ExperienceQuestion {
                        task: task_id,
                        item: item_id,
                        object_name,
                        owner_name,
                        questions,
                        experience,
                        region,
                    };
                    if let Some(ready) = self.experiences.question(question) {
                        self.push_experience_question(ready);
                    }
                } else {
                    self.ui_sounds.push(UiSound::ScriptFloaterOpen);
                    let lines = notifications::permission_lines(questions);
                    let body = aurora_i18n::tr!(
                        "extra-object-owned-by-requests-permission-to",
                        object_name = object_name.to_string(),
                        owner_name = owner_name.to_string(),
                        value0 = lines.join("\n• ").to_string()
                    );
                    let id = self.notifications.push(
                        notifications::Kind::Permissions,
                        aurora_i18n::tr!("world-mod-requested-permissions"),
                        body,
                        notifications::Data::Permissions {
                            task: task_id,
                            item: item_id,
                            questions,
                            region,
                            experience: None,
                        },
                    );
                    self.link_object_notification(id, task_id, &object_name, &owner_name);
                }
                None
            }
            NetEvent::ExperienceInfo(list) => {
                for question in self.experiences.arrived(list) {
                    self.push_experience_question(question);
                }
                None
            }
            NetEvent::ExperienceAttachment { experience, object_name } => {
                if self.rlva.enabled() {
                    self.rlva.controllers.attachment(experience, object_name, Instant::now());
                    self.rlva_controllers_tick(Instant::now());
                }
                None
            }
            NetEvent::ExperiencePermission { experience, result, .. } => {
                self.experiences.permission(experience, result);
                None
            }
            NetEvent::GroupExperiences { group, ids } => {
                self.experiences.group_arrived(group, ids);
                None
            }
            NetEvent::LoadUrl {
                object_name,
                object_id,
                message,
                url,
            } => {
                let body = format!("{message}\n{url}");
                let id = self.notifications.push(
                    notifications::Kind::Url,
                    aurora_i18n::tr!("extra-offers-a-link", object_name = object_name.to_string()),
                    body,
                    notifications::Data::Url(url),
                );
                self.notifications
                    .link_name(id, object_name, notifications::NameTarget::Object(object_id));
                None
            }
            NetEvent::ScriptTeleportRequest {
                object_name,
                region,
                position,
                flags,
            } => {
                log::info!("llMapDestination from {object_name:?}: {region} {position:.0?}, flags {flags:#x}");
                // the app opens the map (LLFloaterWorldMap::openFloater)
                self.map
                    .map_destination(&region, position, flags)
                    .then_some(NetEvent::ScriptTeleportRequest {
                        object_name,
                        region,
                        position,
                        flags,
                    })
            }
            NetEvent::MeanCollisions(list) => {
                self.on_mean_collisions(list);
                None
            }
            NetEvent::Names(list) => {
                for (id, first, last) in list {
                    self.social.avatar_names.on_legacy(id, &first, &last);
                    self.social.names.insert(id, social::format_name(&first, &last));
                }
                self.refresh_task_inventory_offers();
                None
            }
            NetEvent::DisplayNames { names, bad_ids, max_age } => {
                self.social.avatar_names.on_display_names(&names, &bad_ids, max_age);
                self.refresh_task_inventory_offers();
                None
            }
            NetEvent::DisplayNamesFailed(ids) => {
                self.social.avatar_names.on_failed(&ids);
                None
            }
            NetEvent::DisplayNameUpdate { name, old_display_name } => {
                let notice = self.social.avatar_names.on_update(&name, &old_display_name);
                self.refresh_task_inventory_offers();
                log::info!("display name update: {notice}");
                if self.social.avatar_names.notify_changes {
                    self.system_message(notice);
                }
                None
            }
            NetEvent::FriendsOnline { ids, online } => {
                self.social.set_online(&ids, online);
                self.flush_online_notices();
                None
            }
            NetEvent::AvatarAnimations {
                avatar,
                mut anims,
                mut sources,
            } => {
                let now = Instant::now();
                self.drop_blacklisted_animations(avatar, &mut anims, &mut sources);
                if avatar == self.agent_id {
                    log::info!("own animations: {} playing", anims.len());
                    self.agent.ground_sit = anims.iter().any(|(id, _)| *id == body::ANIM_SIT_GROUND_CONSTRAINED);
                    // Firestorm process_avatar_animation / processAnimationStateChanges:
                    // source blocks correspond by index, and previous source entries
                    // survive packets without sources while the animation is active.
                    self.animation_sources.retain(|_, motions| {
                        motions.retain(|id| anims.iter().any(|(active, _)| active == id));
                        !motions.is_empty()
                    });
                    for (source, (id, _)) in sources.into_iter().zip(&anims) {
                        if !source.is_nil()
                            && self.objects.index_of_uuid(&source).is_some()
                            && self.animation_sources.entry(source).or_default().insert(*id)
                        {
                            // RecentAnimationList::addAnimation, on a new pair only
                            self.animation_log.record(*id, source, source == self.agent_id, now);
                        }
                    }
                }
                let prev = self.animations.remove(&avatar).unwrap_or_default();
                // ANIM_AGENT_TYPE starting: the typing sound at the avatar
                const ANIM_AGENT_TYPE: Uuid = Uuid::from_u128(0xc541c47f_e0c0_058b_ad1a_d6ae3a4584d9);
                if anims.iter().any(|(id, _)| *id == ANIM_AGENT_TYPE) && !prev.iter().any(|p| p.id == ANIM_AGENT_TYPE) {
                    self.typing_started.push(avatar);
                }
                let mut list: Vec<_> = anims
                    .into_iter()
                    .map(|(id, seq)| PlayingAnimation::from_signal(id, seq, now, prev.iter().find(|p| p.id == id)))
                    .collect();
                if avatar == self.agent_id
                    && let Some(preview) = self.local_preview
                {
                    self.preview_server_motion = list.iter().find(|a| a.id == preview.id).copied();
                    list.retain(|a| a.id != preview.id);
                    list.push(preview);
                }
                self.animations.insert(avatar, list);
                self.note_friend_join_animations(avatar);
                None
            }
            NetEvent::Environment {
                handle,
                parcel_id: requested,
                environment,
            } => {
                let answer = eep::EnvAnswer::from_llsd(&environment);
                let parcel_id = answer.parcel_id;
                match answer.day.as_ref().filter(|d| d.is_valid()) {
                    Some(d) => {
                        log::info!(
                            "EEP day cycle (parcel {parcel_id}): {} sky keys, {} water keys, day {} s",
                            d.sky_tracks[0].len(),
                            d.water.len(),
                            d.length
                        );
                        if let Some(s) = d.sky_tracks[0].first().map(|k| &k.1) {
                            log::info!(
                                "sky: classic {} probe ambiance {} sunlight {:?} ambient {:?} gamma {} cloud shadow {} blue horizon {:?} haze {} / {}",
                                s.can_auto_adjust,
                                s.probe_ambiance,
                                s.sunlight,
                                s.ambient,
                                s.gamma,
                                s.cloud_shadow,
                                s.blue_horizon,
                                s.haze_horizon,
                                s.haze_density
                            );
                        }
                    }
                    None => log::info!(
                        "environment of parcel {parcel_id} (requested {requested}) has no valid day cycle ({})",
                        if answer.day.is_some() {
                            "empty water or ground sky track"
                        } else {
                            "none"
                        }
                    ),
                }
                let ctx = eep_env::AnswerContext {
                    main_region: self.main_region,
                    agent_parcel: self.parcel.as_ref().map(|p| p.local_id),
                    teleporting: self.teleporting,
                };
                let outcome = self.eep.on_answer(handle, requested, answer, ctx);
                log::info!("EEP answer for parcel {parcel_id}: {outcome:?}");
                None
            }
            NetEvent::WaterHeight { handle, height } => {
                if let Some(r) = self.regions.get_mut(&handle)
                    && let Some(info) = r.info.as_mut()
                {
                    Arc::make_mut(info).water_height = height;
                }
                None
            }
            NetEvent::ParcelMediaCommand { flags, command, time } => {
                self.media
                    .commands
                    .push(crate::media::ParcelMediaMsg::Command { flags, command, time });
                None
            }
            NetEvent::ParcelMediaUpdate {
                url,
                media_id,
                auto_scale,
                mime,
                width,
                height,
                looping,
                ..
            } => {
                self.media.commands.push(crate::media::ParcelMediaMsg::Update {
                    url,
                    media_id,
                    auto_scale,
                    mime,
                    width,
                    height,
                    looping,
                });
                None
            }
            NetEvent::RegionFlags { handle, flags, max_tasks } => {
                if let Some(r) = self.regions.get_mut(&handle) {
                    r.max_tasks = max_tasks;
                    if let Some(info) = r.info.as_mut()
                        && info.region_flags != flags
                    {
                        Arc::make_mut(info).region_flags = flags;
                    }
                }
                self.land_if_fly_blocked(handle);
                None
            }
            NetEvent::Land(ev) => {
                if let aurora_net::land::LandEvent::AttachmentScripts(result) = &ev {
                    match result {
                        Ok((_, attachments)) => {
                            self.temp_attachment_names.learn(attachments);
                            self.rlva_controller_resources(attachments)
                        }
                        Err(_) => self.rlva_controller_resources(&[]),
                    }
                }
                let ctx = land::SelectionContext {
                    agent_region: self.main_region,
                    region_id: self
                        .land
                        .sel
                        .as_ref()
                        .and_then(|s| self.regions.get(&s.handle))
                        .and_then(|r| r.info.as_ref())
                        .map_or(Uuid::nil(), |i| i.region_id),
                };
                if let aurora_net::land::LandEvent::Selected { info, .. } = &ev
                    && !info.is_group_owned
                {
                    self.social.want_name(info.owner_id);
                }
                // the environment editors wait for the answer of what they applied
                if let aurora_net::land::LandEvent::Environment { local_id, result } = &ev {
                    self.environment_answers
                        .push((*local_id, result.as_ref().map(|_| ()).map_err(Clone::clone)));
                    // and Region / Estate shows the region's
                    if *local_id == aurora_net::land::REGION_PARCEL_ID {
                        self.estate.environment_answer(result.as_ref());
                    }
                }
                self.land.apply(ev, &ctx);
                None
            }
            NetEvent::Estate(ev) => {
                match &ev {
                    aurora_net::estate::EstateEvent::TerrainTransforms {
                        handle,
                        transforms: Ok(transforms),
                    } => {
                        if let Some(region) = self.regions.get_mut(handle) {
                            region.terrain_surface.transforms = *transforms;
                        }
                    }
                    aurora_net::estate::EstateEvent::Stats { stats, .. } => self.on_sim_stats_alert(stats),
                    aurora_net::estate::EstateEvent::SimVersion { version, .. } => self.on_sim_version_alert(version),
                    _ => {}
                }
                if self.estate.apply(ev)
                    && let Some(handle) = self.estate.open
                {
                    self.refresh_region_window(handle, false);
                }
                None
            }
            NetEvent::Health(health) => {
                self.health = health;
                None
            }
            NetEvent::Frozen(frozen) => {
                if frozen != self.frozen {
                    log::info!("frozen: {frozen}");
                }
                self.frozen = frozen;
                None
            }
            NetEvent::FreezeAlert(alert) => {
                // the AvatarFrozen… notifications, worded in the viewer's language
                let message = match &alert {
                    aurora_net::freeze::Alert::Frozen { by, seconds: Some(s) } => {
                        aurora_i18n::tr!("world-mod-frozen-for", name = by.as_str(), seconds = *s)
                    }
                    aurora_net::freeze::Alert::Frozen { by, seconds: None } => {
                        aurora_i18n::tr!("world-mod-frozen", name = by.as_str())
                    }
                    aurora_net::freeze::Alert::Unfrozen { by } => aurora_i18n::tr!("world-mod-unfrozen", name = by.as_str()),
                };
                self.ui_sounds.push(UiSound::Alert);
                self.system_message(message.clone());
                let id = self.notifications.push(
                    notifications::Kind::Alert,
                    aurora_i18n::tr!("world-mod-simulator-alert"),
                    message.clone(),
                    notifications::Data::None,
                );
                let notice = self.notice_of(id, attention::Open::Notifications);
                self.ask_attention(attention::Cause::Notice, notice);
                self.alerts.push_back(message);
                None
            }
            NetEvent::AgentParcel(info) => {
                // one line per parcel entered, or when its flight rule changes
                let fly = info.flags & aurora_net::parcel_flags::ALLOW_FLY != 0;
                if self
                    .parcel
                    .as_ref()
                    .is_none_or(|p| p.local_id != info.local_id || (p.flags & aurora_net::parcel_flags::ALLOW_FLY != 0) != fly)
                {
                    log::info!(
                        "agent parcel: local id {} in region {:x}, flight {}",
                        info.local_id,
                        self.main_region.unwrap_or(0),
                        if fly { "allowed" } else { "forbidden" }
                    );
                }
                self.parcel_name = info.name.clone();
                if !info.is_group_owned {
                    self.social.want_name(info.owner_id);
                }
                self.parcel = Some(info);
                self.record_arrival();
                None
            }
            NetEvent::ParcelVoiceInfo {
                handle,
                local_id,
                allowed,
                use_estate,
            } => {
                if self.main_region == Some(handle)
                    && let Some(parcel) = self.parcel.as_mut().filter(|p| p.local_id == local_id)
                {
                    use aurora_net::parcel_flags as pf;
                    let mut flags = parcel.flags & !pf::ALLOW_VOICE_CHAT;
                    if allowed {
                        flags |= pf::ALLOW_VOICE_CHAT;
                    }
                    if let Some(estate) = use_estate {
                        flags &= !pf::USE_ESTATE_VOICE_CHAN;
                        if estate {
                            flags |= pf::USE_ESTATE_VOICE_CHAN;
                        }
                    }
                    if flags != parcel.flags {
                        Arc::make_mut(parcel).flags = flags;
                    }
                }
                None
            }
            NetEvent::RegionIdHandle { region_id, handle } => {
                self.landmarks.on_region_handle(region_id, handle);
                None
            }
            NetEvent::AvatarProfile(p) => {
                self.profiles.insert(*p);
                None
            }
            NetEvent::PickInfo(p) => {
                self.profiles.picks.insert(p.id, *p);
                None
            }
            NetEvent::AvatarClassifieds { target, list } => {
                // replies come in several messages: merge
                let l = self.profiles.classified_lists.entry(target).or_default();
                for (id, name) in list {
                    if !l.iter().any(|(c, _)| *c == id) {
                        l.push((id, name));
                    }
                }
                None
            }
            NetEvent::ClassifiedInfo(c) => {
                self.search
                    .name_row(search::Tab::Classifieds, search::Key::Id(c.id), &c.name, Vec::new());
                self.profiles.classifieds.insert(c.id, *c);
                None
            }
            NetEvent::ClassifiedStats { id, stats } => {
                self.profiles.classified_stats.insert(id, stats);
                None
            }
            NetEvent::ParcelInfo(p) => {
                self.place_details.parcel_known(p.id, p.global);
                self.profiles.parcels.insert(p.id, *p);
                None
            }
            NetEvent::DirReply(r) => {
                self.search.apply_reply(&r);
                if matches!(r.rows, aurora_net::search::DirRows::Groups(_)) {
                    self.search.refresh_group_counts(&self.group_data);
                }
                None
            }
            NetEvent::EventInfo(e) => {
                let date = search::Value::Dated {
                    text: e.date.clone(),
                    unix: e.date_utc,
                };
                self.search
                    .name_row(search::Tab::Events, search::Key::Event(e.id), &e.name, vec![date]);
                self.search.events.insert(e.id, *e);
                None
            }
            ev @ (NetEvent::GroupMembers(_)
            | NetEvent::GroupRoles(_)
            | NetEvent::GroupRoleMembers(_)
            | NetEvent::GroupTitles { .. }
            | NetEvent::GroupNotices { .. }
            | NetEvent::GroupBanList { .. }) => {
                self.group_data.on_event(&ev);
                if matches!(ev, NetEvent::GroupMembers(_)) {
                    self.search.refresh_group_counts(&self.group_data);
                }
                None
            }
            NetEvent::GroupLand(reply) => {
                // process_places_reply: only for a group of ours
                if reply.group.is_nil() || self.groups.is_member(&reply.group) {
                    self.group_money.on_land(&reply);
                } else {
                    log::warn!("PlacesReply for a group we are not in");
                }
                None
            }
            NetEvent::GroupAccount(reply) => {
                self.group_money.on_account(&reply);
                None
            }
            // LLGroupMgr::processJoinGroupReply / processLeaveGroupReply: the
            // group list again, the group's data forgotten, its profile
            // asked again
            // processEjectGroupMemberReply: a failure refreshes the group
            NetEvent::GroupEjected { group, success } => {
                if !success {
                    log::warn!("group {group}: ejection refused, the group is asked again");
                    let member = self.groups.is_member(&group);
                    self.group_data.refresh(group, member);
                }
                None
            }
            NetEvent::GroupJoined { group, success } | NetEvent::GroupLeft { group, success } => {
                if success {
                    self.groups.out.push(aurora_net::NetCommand::RequestGroups);
                    self.group_data.clear(&group);
                    self.group_money.clear(&group);
                    self.search.request_group(group);
                }
                None
            }
            // processCreateGroupReply: the list asked again, the new group
            // ours at once with every ability until it arrives (Firestorm's
            // "fake it"), its profile and our balance asked
            // (LLPanelGroupCreate::refreshCreatedGroup); the window follows
            NetEvent::GroupCreated { group, success, .. } => {
                let name = self.groups.creating.take().unwrap_or_default();
                if success && !group.is_nil() {
                    self.groups.out.push(aurora_net::NetCommand::RequestGroups);
                    self.groups.out.push(aurora_net::NetCommand::RequestBalance);
                    if !self.groups.is_member(&group) {
                        self.groups.groups.push(aurora_net::GroupMembership {
                            id: group,
                            name,
                            insignia: Uuid::nil(),
                            powers: u64::MAX,
                            accept_notices: true,
                            list_in_profile: true,
                            contribution: 0,
                        });
                    }
                    self.search.request_group(group);
                }
                Some(ev)
            }
            NetEvent::GroupProfile(g) => {
                self.follow_group_insignia(g.id, g.insignia);
                self.search.groups.insert(g.id, *g);
                self.search.refresh_group_counts(&self.group_data);
                self.refresh_task_inventory_offers();
                None
            }
            NetEvent::ProductInfo(types) => {
                self.search.apply_land_types(types);
                None
            }
            NetEvent::RemoteParcel { handle, position, result } => {
                self.map.apply_remote_parcel(handle, position, result.as_ref().ok().copied());
                self.place_snapshots
                    .apply_remote_parcel(handle, position, result, &self.profiles.parcels);
                self.place_details.apply_remote_parcel(handle, position, result);
                None
            }
            NetEvent::UserRights { agent, rights } => {
                // LLAvatarTracker::processChange
                for (related, r) in rights {
                    if agent == self.agent_id {
                        if let Some(f) = self.social.friends.iter_mut().find(|f| f.id == related) {
                            f.rights_given = r;
                        }
                    } else if let Some(f) = self.social.friends.iter_mut().find(|f| f.id == agent) {
                        // the online right is not part of the change
                        f.rights_has = r | (f.rights_has & 1);
                    }
                }
                None
            }
            NetEvent::ProfileSaveFailed { target, reason } => {
                self.system_message(aurora_i18n::tr!(
                    "world-mod-could-not-save-the-profile",
                    reason = reason.to_string()
                ));
                // show what the server really has
                self.profiles.refresh(target);
                None
            }
            NetEvent::Balance(b) => {
                // the app compares with UISndMoneyChangeThreshold
                self.balance_changes.push((self.balance, b));
                self.balance = Some(b);
                None
            }
            NetEvent::LandHoldings { credit, committed } => {
                self.land_committed = committed;
                self.land_credit = credit;
                None
            }
            NetEvent::InventoryContents(c) => {
                self.inventory.apply(c);
                None
            }
            NetEvent::InventoryTreeFetched { request, result } => {
                if let Ok(contents) = result {
                    self.inventory.apply(contents);
                }
                self.inventory_background.on_reply(&mut self.inventory, request);
                None
            }
            NetEvent::InventoryFetchFailed { folders } => {
                self.inventory.failed(&folders);
                // BGFolderHttpHandler::processFailure puts a failed folder
                // back in the queue. A fetch sent on the capabilities of the
                // region just left fails once the agent is in the next one:
                // asked again once per change of region, then on the new
                // region's capabilities (`retry_failed`)
                for id in folders {
                    if self.inventory_retried.insert(id) {
                        self.inventory.request(id);
                    }
                }
                None
            }
            NetEvent::InventoryItems(items) => {
                self.inventory.add_items(items);
                None
            }
            NetEvent::AttachmentInventoryItems(items) => {
                self.inventory.add_attachment_items(items);
                None
            }
            NetEvent::InventoryOrphansFetched { request, owner, result } => {
                if owner == self.agent_id && self.inventory_background.on_orphans_reply(request, result.is_ok()) {
                    match result {
                        Ok(contents) => {
                            self.inventory.server_updated(contents.folders, Vec::new());
                            self.inventory.repair_lost_folders();
                            let (attachments, orphans): (Vec<_>, Vec<_>) = contents.items.into_iter().partition(|item| {
                                self.worn_attachment(&item.id).and_then(|i| self.objects.get(i)).is_some_and(|o| {
                                    o.attachment_point() != 0 && !temp_attachments::is_temp_attachment(o.full_id, Some(item.id))
                                })
                            });
                            self.inventory.add_attachment_items(attachments);
                            self.inventory.add_orphan_items(orphans);
                        }
                        Err(reason) => log::warn!("lost inventory recovery failed: {reason}"),
                    }
                }
                None
            }
            NetEvent::AttachmentAssetSaved(item) => {
                if self.rlva.enabled() {
                    self.rlva.watchdog.saved(item, Instant::now());
                }
                None
            }
            NetEvent::InventoryUpdated { folders, items } => {
                self.reveal_added(&folders, &items);
                self.inventory.server_updated(folders, items);
                None
            }
            NetEvent::InventoryDelivery { transaction, folders } => {
                if self.rlva.enabled() {
                    self.rlva.offers.delivered(transaction, &folders, &self.inventory);
                }
                None
            }
            NetEvent::InventoryMoved { id, parent, folder } => {
                if folder {
                    if let Some(f) = self.inventory.folders.get(&id) {
                        let mut moved = f.info.clone();
                        moved.parent = parent;
                        self.inventory.server_updated(vec![moved], Vec::new());
                    }
                } else if let Some(item) = self.inventory.items.get(&id) {
                    let mut moved = item.clone();
                    moved.parent = parent;
                    self.inventory.server_updated(Vec::new(), vec![moved]);
                }
                None
            }
            NetEvent::InventoryItemsFailed { items } => {
                log::warn!("{} inventory items could not be fetched", items.len());
                self.inventory.items_failed(&items);
                None
            }
            NetEvent::InventoryItemsMissing { items } => {
                log::warn!("{} inventory items do not exist (broken links)", items.len());
                self.inventory.items_missing(items);
                None
            }
            NetEvent::AppearanceRequestResult {
                cof_version,
                success,
                expected,
            } => {
                self.outfit.on_bake_result(cof_version, success, expected);
                None
            }
            NetEvent::CofVersionIncremented(version) => {
                let received = self.appearance_cof_version(&self.agent_id);
                self.outfit.on_cof_stepped(version, &mut self.inventory, received);
                None
            }
            NetEvent::MapBlocks { blocks, .. } => {
                self.map.apply_blocks(blocks);
                None
            }
            NetEvent::MapItems { item_type, items } => {
                self.map.apply_items(item_type, items);
                None
            }
            NetEvent::AgentFound { prey, x, y } => {
                // FindAgent only gives global X/Y. A loaded target in the
                // same spot can supply Z; a distant reply must not reuse our
                // own altitude (for example, a skybox at 3,000 m).
                let altitude = self.avatar_map_position(prey).and_then(|pos| {
                    let (ox, oy) = self.main_origin()?;
                    let nearby = (ox as f64 + pos.x as f64 - x).hypot(oy as f64 + pos.y as f64 - y) < 8.0;
                    nearby.then_some(pos.z)
                });
                self.map.agent_found(prey, x, y, altitude);
                None
            }
            NetEvent::ParcelCollision {
                handle,
                kind,
                use_pass,
                bitmap,
            } => {
                self.map.apply_collision(handle, kind, use_pass, bitmap);
                None
            }
            NetEvent::ParcelOverlay { handle, sequence, data } => {
                self.parcel_visibility.invalidate(handle);
                let size = self
                    .regions
                    .get(&handle)
                    .map(|r| (r.heightmap.size_x, r.heightmap.size_y))
                    .unwrap_or((256, 256));
                self.map.apply_overlay(handle, size, sequence, &data);
                None
            }
            NetEvent::LookAt {
                effect,
                source,
                target,
                offset,
                kind,
                ..
            } => {
                // LLHUDEffectLookAt::unpackData: only from an avatar in the
                // world (an effect without its source dies at once)
                let known = self
                    .objects
                    .index_of_uuid(&source)
                    .and_then(|i| self.objects.get(i))
                    .is_some_and(|o| o.is_avatar());
                if source != self.agent_id && known {
                    self.look_at.receive(effect, source, target, offset, kind);
                }
                None
            }
            NetEvent::PointAt {
                effect,
                source,
                target,
                offset,
                kind,
                ..
            } => {
                let known = self
                    .objects
                    .index_of_uuid(&source)
                    .and_then(|i| self.objects.get(i))
                    .is_some_and(|o| o.is_avatar());
                if source != self.agent_id && known {
                    self.point_at.receive(effect, source, target, offset, kind);
                }
                None
            }
            NetEvent::ResetSkeleton {
                source,
                target,
                animations,
            } => {
                if let Some(reset) = self.skeleton_reset_from(source, target, animations) {
                    self.skeleton_resets.push(reset);
                }
                None
            }
            other => Some(other),
        }
    }

    fn mark_tree_material_dirty(&mut self, idx: usize) {
        let Some(o) = self.objects.get_mut(idx) else {
            return;
        };
        o.material_dirty = true;
        let key = o.key;
        let kids: Vec<usize> = self.objects.children_of(&key).to_vec();
        for k in kids {
            self.mark_tree_material_dirty(k);
        }
    }

    fn on_chat(&mut self, c: ChatMessage) {
        let controller_allowed = self.rlva_controller_chat_allowed(c.source_id);
        if c.source_type == aurora_net::ChatSourceType::Object
            && c.chat_type == aurora_net::ChatType::OwnerSay
            && c.message.starts_with('@')
            && controller_allowed
        {
            self.rlva_bind_controller(c.source_id);
            self.rlva_prepare_version(c.source_id, &c.message);
        }
        if controller_allowed && self.rlva.receive(&c) {
            self.eep.rlva_locked = self.rlva.has("setenv");
            let environment_owner = self.rlva.rules_for("setenv", Uuid::nil()).next().map(|(id, _)| id);
            if self.eep.rlva_locked
                && self.rlva.environment.is_none()
                && let Some(actor) = environment_owner
            {
                self.rlva.environment = Some((actor, self.eep.current().0));
            }
            self.inventory.rlva_preview_mask = self.rlva.preview_mask();
            self.refresh_rlva_privacy();
            self.rlva_watchdog_tick(Instant::now());
            return;
        }
        let mut c = c;
        if let std::borrow::Cow::Owned(text) = self.rlva_filter_text(&c.message) {
            c.message = text;
        }
        let own_attachment = self.rlva.enabled()
            && c.source_type == aurora_net::ChatSourceType::Object
            && c.owner_id == self.agent_id
            && self.rlva_own_attachment_root(c.source_id).is_some();
        if !own_attachment && !self.rlva.receive_chat(&mut c, self.agent_id) {
            return;
        }
        if c.message.is_empty() || self.lsl_bridge.hides(&c) || self.chat_blocked(&c) || self.object_chat_filtered(&c) {
            return;
        }
        if self.receive_hand_contacts(&c, Instant::now()) {
            return;
        }
        if self.script_debug.route(&c, self.agent_id, Instant::now()) || self.anti_spam_chat(&c) {
            return;
        }
        let kind = match c.source_type {
            ChatSourceType::System => ChatKind::System,
            ChatSourceType::Object => ChatKind::Object(c.chat_type),
            ChatSourceType::Agent => {
                if c.source_id == self.agent_id {
                    // said, whispered or shouted: shown like anyone's
                    // (FSChatHistory::appendMessage styles by mChatType)
                    ChatKind::Own(c.chat_type)
                } else {
                    // "We have a real utterance now": no more typing
                    self.nearby_typing.remove(&c.source_id);
                    // LLNearbyChatHandler::processChat: the speaker joins
                    // the recent people
                    self.met(c.source_id);
                    // LLAgent::heardChat (half the time, ll_rand(2))
                    let coin = Uuid::new_v4().as_bytes()[0] & 1 == 0;
                    self.look_at.heard_chat(c.source_id, coin);
                    // FIRE-36367 (process_chat_from_simulator): said, whispered
                    // or shouted by someone else (9 = CHAT_TYPE_DIRECT)
                    if !c.source_id.is_nil()
                        && matches!(
                            c.chat_type,
                            ChatType::Whisper | ChatType::Normal | ChatType::Shout | ChatType::Other(9)
                        )
                    {
                        self.ui_sounds.push(UiSound::NearbyChat);
                    }
                    ChatKind::Local(c.chat_type)
                }
            }
        };
        // IDEVO: avatars by their complete name ("Jane Doe (janedoe)")
        let from = match c.source_type {
            ChatSourceType::Agent => self.social.avatar_names.complete(&c.source_id).unwrap_or(c.from_name),
            _ => c.from_name,
        };
        // a resident's line that mentions the agent
        // (LLUrlRegistry::containsAgentMention)
        if matches!(kind, ChatKind::Local(_)) && crate::ui::chat::mentions(&c.message, &self.agent_id) {
            let notice = attention::Notice {
                title: from.clone(),
                body: c.message.clone(),
                open: attention::Open::NearbyChat,
            };
            self.ask_attention(attention::Cause::Mention, Some(notice));
        }
        // process_chat_from_simulator: every line but ours and the debug
        // channel's (FSKeywords::chatContainsKeyword)
        // (an object's line alerts no more while #741's option is on)
        if !matches!(kind, ChatKind::Own(_) | ChatKind::Object(ChatType::Debug)) && self.alerts(kind) {
            self.keyword_alert(c.source_id, &from, &c.message, true, attention::Open::NearbyChat);
        }
        // a resident's line, not ours nor an object's: the Conversations
        // button may flash (the blocked never got this far)
        self.nearby_news |= matches!(kind, ChatKind::Local(_));
        // the bubble above the speaker's head: residents only, ourselves
        // included, never while we are in do not disturb
        // (process_chat_from_simulator, LLVOAvatar::addChat)
        if matches!(kind, ChatKind::Local(_) | ChatKind::Own(_)) && !self.status.dnd {
            use bubble_chat::Voice;
            let voice = match c.chat_type {
                ChatType::Whisper => Some(Voice::Whisper),
                ChatType::Normal => Some(Voice::Normal),
                ChatType::Shout => Some(Voice::Shout),
                _ => None,
            };
            if let Some(voice) = voice {
                self.bubble_chat.add(
                    c.source_id,
                    bubble_chat::Utterance {
                        text: c.message.clone(),
                        from: from.clone(),
                        voice,
                        said: Instant::now(),
                    },
                );
            }
        }
        self.push_chat(ChatLine {
            time: SystemTime::now(),
            from,
            text: c.message,
            kind,
            source: c.source_id,
            origin: Origin::Live,
        });
    }

    fn on_im(&mut self, mut im: InstantMessage) {
        // a message to the whole region, before the rules of conversations
        // (region_messages.rs)
        if region_messages::is_region_message(&im) {
            if !self.anti_spam_im(&im) {
                self.on_region_message(&im);
            }
            return;
        }
        if self.im_blocked(&im) {
            return;
        }
        if self.anti_spam_im(&im) {
            return;
        }
        // IM queries obey the same friends-only policy, before recvim.
        if im.dialog == 0 && self.friends_only_drops(&im.from_agent_id, &im.from_name) {
            log::info!("message from a non-friend dropped (friends only)");
            return;
        }
        if self.rlva_im_query(&im) {
            return;
        }
        let rlva_filter = self.rlva_receive_im(&mut im);
        // IM_TYPING_START / STOP (llimprocessing.cpp: processIMTypingStart /
        // Stop): "is typing" in their conversation, never a line
        if matches!(im.dialog, IM_TYPING_START | IM_TYPING_STOP) {
            if !im.from_agent_id.is_nil() {
                self.social.set_typing(im.from_agent_id, im.dialog == IM_TYPING_START);
            }
            return;
        }
        if self.on_session_im(&im) || self.on_status_im(&im) || self.on_im_offer(&im) {
            return;
        }
        // decided before the message opens its conversation
        let autoresponse = self.autoresponse_due(&im);
        // 0 = message from user, 19 = message from object; others are notices etc.
        if im.message.is_empty() {
            return;
        }
        if im.dialog == 0 && !im.from_agent_id.is_nil() {
            // their message ends their "is typing" (LLIMModel::addMessage)
            self.social.set_typing(im.from_agent_id, false);
            // LLIMMgr::addMessage: the sender joins the recent people
            self.met(im.from_agent_id);
            // an ordinary message: their automatic response is behind them
            self.social.auto_responded.remove(&im.from_agent_id);
            if !im.from_name.is_empty() {
                self.social.names.entry(im.from_agent_id).or_insert_with(|| im.from_name.clone());
            }
            // UISndNewIncomingIMSession (by the IM mode)
            self.im_messages.push(ImMessage {
                kind: ImKind::Private,
                session: im.from_agent_id,
                new_session: !self.social.has_session(&im.from_agent_id),
            });
            let from = self
                .social
                .avatar_names
                .complete(&im.from_agent_id)
                .unwrap_or_else(|| im.from_name.clone());
            self.social.session_mut(im.from_agent_id);
            let open = attention::Open::Conversation(im.from_agent_id);
            self.keyword_alert(im.from_agent_id, &from, &im.message, false, open);
            // a message kept while we were offline says when it was sent
            // ("(Saved <date>)" before the text, llimprocessing.cpp)
            let text = if im.offline {
                saved_prefix(im.timestamp) + &im.message
            } else {
                im.message.clone()
            };
            let line = ChatLine {
                time: SystemTime::now(),
                from,
                text,
                kind: ChatKind::Im,
                source: im.from_agent_id,
                origin: Origin::Live,
            };
            self.conversation_push(im.from_agent_id, line, true);
            if rlva_filter != rlva::communication::Received::Masked {
                self.send_autoresponse(&im, autoresponse);
            }
            return;
        }
        // dialog 19 = MessageFromObject
        if self.object_im_filtered(&im) {
            return;
        }
        let kind = if im.dialog == 19 { ChatKind::ObjectIm } else { ChatKind::Im };
        let from = if im.from_name.is_empty() {
            aurora_i18n::tr!("common-system").to_owned()
        } else {
            im.from_name
        };
        // an object's message kept while we were offline (IM_FROM_TASK)
        let text = if im.offline && im.dialog == 19 {
            saved_prefix(im.timestamp) + &im.message
        } else {
            im.message
        };
        // an object's message by FSFlashOnObjectIM, told like an IM by its
        // name (GrowlManager::onNearbyChatMessage); a message without a
        // sender ("System") is a SystemMessage
        let cause = if im.dialog == 19 {
            attention::Cause::ObjectIm
        } else {
            attention::Cause::Message
        };
        // (neither while #741's option keeps objects quiet)
        if self.alerts(kind) {
            let notice = attention::Notice {
                title: from.clone(),
                body: text.clone(),
                open: attention::Open::NearbyChat,
            };
            self.ask_attention(cause, Some(notice));
            // an object's IM is checked as nearby chat (processNewMessage,
            // IM_FROM_TASK)
            if im.dialog == 19 {
                self.keyword_alert(im.from_agent_id, &from, &text, true, attention::Open::NearbyChat);
            }
        }
        self.push_chat(ChatLine {
            time: SystemTime::now(),
            from,
            text,
            kind,
            source: im.from_agent_id,
            origin: Origin::Live,
        });
    }

    /// IM dialogs that are offers or notices go to the notification center.
    /// Returns true when handled.
    fn on_im_offer(&mut self, im: &InstantMessage) -> bool {
        use notifications::{Data, Kind, im as d};
        let from = if im.from_name.is_empty() {
            aurora_i18n::tr!("world-mod-someone").to_owned()
        } else {
            im.from_name.clone()
        };
        let before = self.notifications.last_id();
        let n = &mut self.notifications;
        match im.dialog {
            // teleport offers and requests (teleport_invites.rs)
            d::LURE_USER => self.on_teleport_offer(im, &from),
            d::TELEPORT_REQUEST => self.on_teleport_request(im, &from),
            d::GODLIKE_LURE_USER => self.on_godlike_lure(im, &from),
            d::LURE_ACCEPTED => self.on_teleport_offer_answered(im, true),
            d::LURE_DECLINED => self.on_teleport_offer_answered(im, false),
            d::FRIENDSHIP_OFFERED => {
                self.ui_sounds.push(UiSound::FriendshipOffer);
                n.push(
                    Kind::Friendship,
                    aurora_i18n::tr!("world-mod-offers-you-friendship", from = from.to_string()),
                    im.message.clone(),
                    Data::Friend {
                        from: im.from_agent_id,
                        tx: im.session_id,
                        offline: im.offline,
                    },
                );
            }
            d::TASK_INVENTORY_OFFERED => {
                if self.on_task_inventory_offer(im, &from) {
                    return true;
                }
            }
            d::INVENTORY_OFFERED => {
                let asset_type = im.binary_bucket.first().map(|b| *b as i8).unwrap_or(-1);
                // A resident's offer is in our inventory before we answer:
                // prefetch it (inventory_offer_handler, llimprocessing.cpp),
                // or it stays unseen until the next login. A folder cannot be
                // fetched by id; it lands in the root, reloaded instead.
                let object = notifications::offered_object(&im.binary_bucket).map_or(Uuid::nil(), |(_, id)| id);
                if !object.is_nil() {
                    self.groups.out.push(if asset_type == notifications::AT_CATEGORY {
                        aurora_net::NetCommand::FetchInventory {
                            folders: vec![self.inventory.root],
                            owner: self.agent_id,
                            library: false,
                        }
                    } else {
                        aurora_net::NetCommand::FetchItems {
                            items: vec![object],
                            owner: self.agent_id,
                        }
                    });
                }
                let what = inventory_offers::item_label(&im.message);
                let entry = n.push(
                    Kind::Inventory,
                    aurora_i18n::tr!("world-mod-gives-you", from = from.to_string(), what = what.to_string()),
                    String::new(),
                    Data::Inventory {
                        from: im.from_agent_id,
                        tx: im.session_id,
                        asset_type,
                        object,
                        task: false,
                        task_offer: None,
                    },
                );
                n.link_name(entry, from.clone(), notifications::NameTarget::Avatar(im.from_agent_id));
                if self.receive_inventory_offer(im, entry) {
                    return true;
                }
                self.ui_sounds.push(UiSound::InventoryOffer);
                self.on_inventory_offer_card(im, &from, asset_type, entry);
            }
            // FSShowJoinedGroupInvitations (FIRE-20385): an invitation to a
            // group we are already in is dropped unless asked for
            d::GROUP_INVITATION if self.groups.is_member(&im.from_agent_id) && !self.groups.show_joined_invitations => {
                log::info!("group invitation to a group we are already in: not shown");
            }
            d::GROUP_INVITATION => {
                self.ui_sounds.push(UiSound::GroupInvitation);
                n.push(
                    Kind::Group,
                    aurora_i18n::tr!("world-mod-invitation-to-join-a-group"),
                    im.message.clone(),
                    Data::Group {
                        group: im.from_agent_id,
                        tx: im.session_id,
                        offline: im.offline,
                    },
                );
            }
            d::GROUP_NOTICE => {
                self.ui_sounds.push(UiSound::GroupNotice);
                self.push_group_notice(im, &from);
            }
            d::FRIENDSHIP_ACCEPTED => {
                self.social.form_friendship(im.from_agent_id);
                self.status
                    .hide_from_new_friend(&mut self.social, &mut self.groups.out, im.from_agent_id);
                n.push(
                    Kind::Friendship,
                    aurora_i18n::tr!("world-mod-accepted-your-friendship", from = from.to_string()),
                    String::new(),
                    Data::None,
                );
            }
            d::INVENTORY_ACCEPTED | d::INVENTORY_DECLINED => {
                let verb = if im.dialog == d::INVENTORY_ACCEPTED {
                    aurora_i18n::tr!("world-mod-accepted")
                } else {
                    aurora_i18n::tr!("world-mod-declined")
                };
                n.push(
                    Kind::Inventory,
                    aurora_i18n::tr!("world-mod-has-your-object", from = from.to_string(), verb = verb.to_string()),
                    String::new(),
                    Data::None,
                );
            }
            d::MESSAGEBOX | d::FROM_TASK_AS_ALERT => {
                n.push(Kind::Alert, from, im.message.clone(), Data::None);
            }
            // a notice opened in the group profile's Notices tab
            d::GROUP_NOTICE_REQUESTED => return self.group_data.on_notice_im(im),
            d::GOTO_URL => {
                let url = String::from_utf8_lossy(&im.binary_bucket).trim_end_matches('\0').to_owned();
                n.push(Kind::Url, from, im.message.clone(), Data::Url(url));
            }
            _ => return false,
        }
        // every offer and notice flashes (the end of processNewMessage);
        // the system hears those of the Legacy list (growl_notifications.xml:
        // TeleportOffered*, TeleportRequest, OfferFriendship, FriendshipAccepted,
        // UserGiveItem, ObjectGiveItem, GroupNotice, InventoryAccepted /
        // Declined, SystemMessageTip)
        let told = matches!(
            im.dialog,
            d::LURE_USER
                | d::GODLIKE_LURE_USER
                | d::TELEPORT_REQUEST
                | d::FRIENDSHIP_OFFERED
                | d::FRIENDSHIP_ACCEPTED
                | d::INVENTORY_OFFERED
                | d::TASK_INVENTORY_OFFERED
                | d::INVENTORY_ACCEPTED
                | d::INVENTORY_DECLINED
                | d::GROUP_NOTICE
                | d::MESSAGEBOX
        );
        let last = self.notifications.last_id();
        if last != before
            && !matches!(
                im.dialog,
                d::TASK_INVENTORY_OFFERED | d::GROUP_NOTICE | d::FROM_TASK_AS_ALERT | d::GOTO_URL | d::MESSAGEBOX
            )
        {
            let target = if im.from_group || im.dialog == d::GROUP_INVITATION {
                notifications::NameTarget::Group(im.from_agent_id)
            } else {
                notifications::NameTarget::Avatar(im.from_agent_id)
            };
            self.notifications.link_name(last, im.from_name.clone(), target);
        }
        let notice = if told && last != before {
            self.notice_of(last, attention::Open::Notifications)
        } else {
            None
        };
        self.ask_attention(attention::Cause::Message, notice);
        true
    }

    /// A group notice received (IM_GROUP_NOTICE, process_improved_im in
    /// indra/newview/llimprocessing.cpp and LLToastGroupNotifyPanel,
    /// originally LGPL 2.1): titled by its group and subject, its sender
    /// first in the text ("Sent by"), the group's insignia beside it and its
    /// attachment offered on the card. A notice whose bucket cannot be read
    /// still shows, titled by its sender as before.
    fn push_group_notice(&mut self, im: &InstantMessage, from: &str) {
        use notifications::{Data, Kind, NameTarget, NoticeAttachment};
        // "subject|message"
        let (subject, body) = im
            .message
            .split_once('|')
            .unwrap_or((aurora_i18n::tr!("common-group-notice"), im.message.as_str()));
        let bucket = aurora_net::groups::notice_bucket(&im.binary_bucket);
        if bucket.is_none() {
            log::warn!("group notice: malformed binary bucket");
        }
        let group = bucket.as_ref().map(|b| b.group).filter(|g| !g.is_nil());
        let group_name = group.and_then(|g| {
            self.groups
                .group(&g)
                .map(|m| m.name.clone())
                .or_else(|| self.search.groups.get(&g).map(|s| s.name.clone()))
                .filter(|name| !name.is_empty())
        });
        let title = format!("{} : {subject}", group_name.as_deref().unwrap_or(from));
        let sent_by = aurora_i18n::tr!("world-mod-group-notice-sent-by", sender = from.to_string());
        let text = if body.is_empty() { sent_by } else { format!("{sent_by}\n{body}") };
        let data = match (group, &bucket) {
            (Some(group), Some(b)) => Data::GroupNotice {
                group,
                attachment: b.has_inventory.then(|| NoticeAttachment {
                    from: im.from_agent_id,
                    tx: im.session_id,
                    name: b.item_name.clone(),
                    asset_type: b.asset_type,
                    saved: false,
                }),
            },
            _ => Data::None,
        };
        let id = self.notifications.push(Kind::Group, title, text, data);
        if let (Some(group), Some(name)) = (group, group_name) {
            self.notifications.link_name(id, name, NameTarget::Group(group));
        }
        // the sender, unless the notice came under the group's own id
        if Some(im.from_agent_id) != group {
            self.notifications
                .link_name(id, im.from_name.clone(), NameTarget::Avatar(im.from_agent_id));
        }
    }

    /// "Save to inventory" of a notice's attachment: into the system folder
    /// of its type, as an offer accepted (LLOfferInfo::mFolderID).
    pub fn save_notice_attachment(&mut self, group: Uuid) {
        let Some(asset_type) = self
            .group_data
            .get(&group)
            .and_then(|g| g.notice.as_ref())
            .and_then(|n| n.attachment.as_ref())
            .map(|a| a.1)
        else {
            return;
        };
        let inv = &self.inventory;
        let folder = inv
            .folders
            .values()
            .find(|f| !f.library && f.info.type_default == i32::from(asset_type))
            .map_or(inv.root, |f| f.info.id);
        self.group_data.save_attachment(group, folder);
    }

    fn link_object_notification(&mut self, id: u64, object: Uuid, name: &str, owner_name: &str) {
        use notifications::NameTarget;
        self.notifications.link_name(id, name, NameTarget::Object(object));
        if let Some(owner) = self
            .objects
            .index_of_uuid(&object)
            .and_then(|i| self.objects.get(i))
            .map(|o| o.owner_id)
        {
            let group = self.groups.groups.iter().any(|g| g.id == owner)
                || self.land.group_names.contains_key(&owner)
                || self.search.groups.contains_key(&owner);
            self.notifications.link_name(
                id,
                owner_name,
                if group {
                    NameTarget::Group(owner)
                } else {
                    NameTarget::Avatar(owner)
                },
            );
        }
    }

    /// The Calling Cards folder, sent with a friendship offer
    /// (LLAvatarActions::requestFriendship).
    pub fn calling_card_folder(&self) -> Uuid {
        let inv = &self.inventory;
        inv.folders
            .values()
            .find(|f| !f.library && f.info.type_default == 2)
            .map(|f| f.info.id)
            .unwrap_or(inv.root)
    }

    /// ScriptQuestionExperience (process_script_experience_details): the
    /// experience's question once its details are known, grid-wide or not.
    fn push_experience_question(&mut self, q: experiences::ExperienceQuestion) {
        let details = self.experiences.details.get(&q.experience);
        let name = details
            .map(|d| d.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| q.experience.to_string());
        let scope = if details.is_some_and(|d| d.grid_wide()) {
            aurora_i18n::tr!("world-experiences-grid-scope")
        } else {
            aurora_i18n::tr!("world-experiences-land-scope")
        };
        let lines = notifications::permission_lines(q.questions);
        // LLCacheName::cleanFullName: the owner without " Resident"
        let owner = q.owner_name.strip_suffix(" Resident").unwrap_or(&q.owner_name);
        let body = aurora_i18n::tr!(
            "world-experiences-question",
            object_name = q.object_name.clone(),
            owner_name = owner.to_owned(),
            scope = scope.to_owned(),
            experience = name.clone(),
            questions = lines.join("\n• ")
        );
        let id = self.notifications.push(
            notifications::Kind::Permissions,
            aurora_i18n::tr!("world-experiences-question-title", name = name.clone()),
            body,
            notifications::Data::Permissions {
                task: q.task,
                item: q.item,
                questions: q.questions,
                region: q.region,
                experience: Some(notifications::ExperiencePrompt {
                    id: q.experience,
                    name: name.clone(),
                    object_name: q.object_name.clone(),
                }),
            },
        );
        self.notifications
            .link_name(id, name, notifications::NameTarget::Experience(q.experience));
        self.link_object_notification(id, q.task, &q.object_name, owner);
        // UISndQuestionExperience is Firestorm's own; the script sound is Aurora's
        self.ui_sounds.push(UiSound::ScriptFloaterOpen);
    }

    /// Answer a notification: messages to send and a URL to open.
    pub fn respond_notification(&mut self, id: u64, r: notifications::Response) -> (Vec<aurora_net::NetCommand>, Option<String>) {
        let data = self.notifications.list.iter().find(|n| n.id == id).map(|n| n.data.clone());
        if let Some(notifications::Data::RlvaQuery { from, exceptions }) = data {
            self.rlva_answer_im_query(from, exceptions, &r);
            self.notifications.respond(id, r, |_| Uuid::nil(), Uuid::nil());
            return (Vec::new(), None);
        }
        // the experience's profile opens over its question, which stays
        if let notifications::Response::OpenExperience(experience) = r {
            self.experiences.open_profile(experience);
            return (Vec::new(), None);
        }
        // script_question_cb: "Block object" (Mute) and "Block experience"
        // refuse the question first, then block
        if let Some(notifications::Data::Permissions { task, experience, .. }) = &data
            && matches!(r, notifications::Response::Block | notifications::Response::BlockExperience)
        {
            let (out, _) = self
                .notifications
                .respond(id, notifications::Response::Decline, |_| Uuid::nil(), Uuid::nil());
            match (&r, experience) {
                (notifications::Response::Block, Some(prompt)) => {
                    if let Err(error) = self.block(*task, &prompt.object_name, mutes::MuteType::Object) {
                        self.system_message(error);
                    }
                    // script_question_mute: the object's other questions go too
                    let blocked = *task;
                    self.notifications
                        .list
                        .retain(|n| !matches!(&n.data, notifications::Data::Permissions { task, .. } if *task == blocked));
                }
                (notifications::Response::BlockExperience, Some(prompt)) => {
                    self.experiences
                        .change(prompt.id, aurora_net::experience::ExperiencePermissionChange::Block);
                }
                _ => {}
            }
            return (out, None);
        }
        // script_question_cb: accepting an experience's question allows it
        // (the "experience_permission" event the profile listens to)
        if let (
            notifications::Response::Accept,
            Some(notifications::Data::Permissions {
                experience: Some(prompt), ..
            }),
        ) = (&r, &data)
        {
            self.experiences
                .permissions
                .insert(prompt.id, aurora_net::experience::ExperiencePermission::Allowed);
        }
        if matches!(r, notifications::Response::Block) {
            if let Some(notifications::Data::Inventory {
                from,
                task: true,
                task_offer: Some(offer),
                ..
            }) = &data
            {
                return (self.mute_task_inventory_owner(id, *from, offer), None);
            }
            if let Some(notifications::Data::Dialog {
                object,
                object_name,
                own_object: false,
                ..
            }) = self.notifications.list.iter().find(|n| n.id == id).map(|n| n.data.clone())
                && let Err(error) = self.block(object, &object_name, mutes::MuteType::Object)
            {
                self.system_message(error);
            }
            return (Vec::new(), None);
        }
        // voice calls (LLIncomingCallDialog::processCallResponse): accept,
        // decline (closing too) or "Start IM"
        if let Some(notifications::Data::Call { invite }) = &data {
            let invite = (**invite).clone();
            self.notifications.respond(id, r.clone(), |_| Uuid::nil(), Uuid::nil());
            let answer = match r {
                notifications::Response::Accept => calls::InviteAnswer::Accept,
                notifications::Response::Button(0) => calls::InviteAnswer::Text,
                _ => calls::InviteAnswer::Decline,
            };
            self.invite_card_answered(invite, answer);
            return (std::mem::take(&mut self.groups.out), None);
        }
        // teleport offers and requests: answered through their invitation,
        // closing the notification declines (as Firestorm's toasts do)
        if let Some(notifications::Data::Lure { invite, .. } | notifications::Data::TeleportRequest { invite, .. }) =
            self.notifications.list.iter().find(|n| n.id == id).map(|n| n.data.clone())
        {
            let accept = matches!(r, notifications::Response::Accept);
            self.notifications.list.retain(|n| n.id != id);
            return (self.answer_teleport_invite(invite, accept), None);
        }
        // callingcard_offer_callback: a decline also sends our do not
        // disturb response
        if matches!(r, notifications::Response::Decline)
            && let Some(notifications::Data::CallingCard { from, .. }) = &data
        {
            self.calling_card_declined(*from);
        }
        // friendship_offer_callback: formFriendship on accept
        if matches!(r, notifications::Response::Accept)
            && let Some(notifications::Data::Friend { from, .. }) =
                self.notifications.list.iter().find(|n| n.id == id).map(|n| n.data.clone())
        {
            self.social.form_friendship(from);
            self.status.hide_from_new_friend(&mut self.social, &mut self.groups.out, from);
        }
        let inv = &self.inventory;
        let folder_of = |t: i32| {
            inv.folders
                .values()
                .find(|f| !f.library && f.info.type_default == t)
                .map(|f| f.info.id)
                .unwrap_or(inv.root)
        };
        // SL folder types match the asset types for the system folders
        let calling = folder_of(2);
        let folders: Vec<(i8, Uuid)> = [0i8, 1, 3, 5, 6, 7, 10, 13, 20, 21, 49, 56, 57]
            .iter()
            .map(|t| (*t, folder_of(*t as i32)))
            .collect();
        let root = inv.root;
        // refusing a resident's offer also discards what it already put in
        // our inventory; closing the notification keeps it
        let discard = match self.notifications.list.iter().find(|n| n.id == id).map(|n| &n.data) {
            Some(notifications::Data::Inventory { asset_type, object, .. })
                if !object.is_nil() && matches!(r, notifications::Response::Decline) =>
            {
                Some((*asset_type, *object))
            }
            _ => None,
        };
        // ShowInInventory: what a kept offer gives is shown in the inventory
        let kept = match self.notifications.list.iter().find(|n| n.id == id) {
            Some(notifications::Notification {
                data: notifications::Data::Inventory { object, task, .. },
                ..
            }) if matches!(r, notifications::Response::Accept) => Some((*object, *task)),
            // a notice's attachment: what the server adds next
            // (LLOpenTaskGroupOffer)
            Some(n) if matches!(r, notifications::Response::Accept) && n.offered_attachment().is_some() => Some((Uuid::nil(), true)),
            _ => None,
        };
        // a resident's offer: its card in the conversation takes the answer
        let offer_outcome = self
            .notifications
            .list
            .iter()
            .find(|n| n.id == id && matches!(n.data, notifications::Data::Inventory { task: false, .. }))
            .map(|n| notifications::outcome_of(n, &r));
        let mut out = self.notifications.respond(
            id,
            r,
            |t| folders.iter().find(|(ft, _)| *ft == t).map(|(_, f)| *f).unwrap_or(root),
            calling,
        );
        self.rlva_shared_offer_answer(id, &mut out.0);
        self.received_items.answer(id, &mut out.0);
        if let Some((asset_type, object)) = discard {
            self.discard_offer(asset_type, object);
        }
        if let Some(outcome) = offer_outcome {
            self.inventory_offer_answered(id, outcome);
        }
        if let Some((object, task)) = kept {
            self.reveal_kept_offer(object, task);
        }
        out
    }

    /// A refused resident's offer is already in our inventory: it goes to
    /// the Trash (LLDiscardAgentOffer, indra/newview/llviewermessage.cpp,
    /// originally LGPL 2.1). The change is queued for the app and does not
    /// need the item to be fetched yet.
    pub(super) fn discard_offer(&mut self, asset_type: i8, object: Uuid) {
        use aurora_net::inventory::operations::{Mutation, Operation};
        use inventory::actions;
        let inv = &self.inventory;
        // 14: LLFolderType::FT_TRASH
        let Some(trash) = actions::system(inv, 14) else {
            log::warn!("refused inventory offer {object} left in place: no Trash folder");
            return;
        };
        if actions::under(inv, object, trash) {
            return;
        }
        // until it is fetched, it is where the server files its type (the
        // root for a folder: both share their numbers with the asset types)
        let from = actions::parent(inv, object)
            .or_else(|| actions::system(inv, i32::from(asset_type)))
            .unwrap_or(inv.root);
        self.inventory_edits.push(Mutation {
            operations: vec![Operation::Patch {
                id: object,
                folder: asset_type == notifications::AT_CATEGORY,
                body: aurora_llsd::llsd_map! { "parent_id" => trash },
            }],
            refresh: vec![from, trash],
            ..Default::default()
        });
    }

    /// Inventory changes decided outside the inventory window, for the app
    /// to send.
    pub fn take_inventory_edits(&mut self) -> Vec<aurora_net::inventory::operations::Mutation> {
        std::mem::take(&mut self.inventory_edits)
    }

    /// Run before a confirmed detach unlinks/removes its root, while the
    /// avatar parent and AttachItemID are still available. Child prims,
    /// temporary attachments and other avatars cannot change our COF.
    fn unregister_attachment(&mut self, idx: usize) {
        let Some(object) = self.objects.get(idx).filter(|o| !o.is_avatar()) else {
            return;
        };
        let ours = self
            .objects
            .parent_of(object)
            .and_then(|p| self.objects.get(p))
            .is_some_and(|p| p.is_avatar() && p.full_id == self.agent_id);
        let item = object.attachment_item_id();
        if !ours {
            return;
        }
        // LLVOAvatarSelf::detachObject (indra/newview/llvoavatarself.cpp,
        // LGPL 2.1): temporary attachments have no inventory item, but their
        // root and linked scripts still own motions that must be stopped.
        self.stop_linkset_motions(idx);
        if let Some(item) = item {
            self.attachment_inventory.detached(item);
        }
        if let Some(item) = item
            && self.outfit.keeps_cof_link(item)
        {
            // the region replaced it during the login restore: worn again
            return;
        }
        if let Some(item) = item
            && let Some(change) = appearance::unlink_attachment(&mut self.inventory, item)
        {
            self.inventory_edits.push(change);
        }
    }

    /// Name of an avatar by id (from its object name-values).
    pub fn avatar_name(&self, id: &Uuid) -> Option<String> {
        if !id.is_nil() && !self.rlva_can_name(*id) {
            return Some(rlva::privacy::anonymous(*id).display_name.clone());
        }
        let idx = self.objects.index_of_uuid(id)?;
        self.objects.get(idx)?.display_name()
    }

    /// Friends' online / offline notices once their names are known
    /// (LLAvatarNameCache::get with a callback).
    pub fn flush_online_notices(&mut self) {
        for (id, n, online) in self.social.take_online_notices() {
            // FIRE-2731 (on_avatar_name_cache_notify)
            self.ui_sounds
                .push(if online { UiSound::FriendOnline } else { UiSound::FriendOffline });
            // the toast of LLNotificationTipHandler (ChatOnlineNotification),
            // never during login, in do not disturb nor for a blocked resident
            if self.notifications.toasts.presence && self.notifications.started && !self.status.dnd && !self.mutes.is_muted_id(&id) {
                let notification = self.notifications.push_presence(id, n.clone(), online);
                self.notifications
                    .link_name(notification, self.social.name_of(&id), notifications::NameTarget::Avatar(id));
            }
            // FriendOnlineOffline for the system (never during login)
            if self.notifications.started && !self.mutes.is_muted_id(&id) {
                let notice = attention::Notice {
                    title: n.clone(),
                    body: String::new(),
                    open: attention::Open::Conversation(id),
                };
                self.ask_attention(attention::Cause::Notice, Some(notice));
            }
            // the line of nearby chat (OnlineOfflinetoNearbyChat), only when
            // asked for: the toast already says it
            if self.notifications.toasts.presence_in_chat {
                // checked as nearby chat (LLAvatarTracker::processNotify),
                // unless notices are kept quiet (#741)
                if self.alerts(ChatKind::System) {
                    let from = self.social.name_of(&id);
                    self.keyword_alert(id, &from, &n, true, attention::Open::NearbyChat);
                }
                self.system_notice(n, notice_topics::Topic::People);
            }
        }
    }

    /// The line `m` just added to its conversation, when a resident wrote
    /// it (no system lines, no objects, nothing of ours or of the blocked).
    fn resident_line(&self, m: &ImMessage) -> Option<&ChatLine> {
        let line = self.social.ims.iter().find(|s| s.other == m.session).and_then(|s| s.lines.back())?;
        (!line.source.is_nil() && line.source != self.agent_id && line.kind == ChatKind::Im && !self.mutes.is_muted_id(&line.source))
            .then_some(line)
    }

    /// `m` makes the Conversations button flash while the window is closed
    /// (FSFloaterIMContainer::addFlashingSession, from notify_of_message in
    /// llimview.cpp): a resident's message, never in do not disturb. Group
    /// chats too, whatever their toast.
    pub fn message_flashes_conversations(&self, m: &ImMessage) -> bool {
        !self.status.dnd && self.resident_line(m).is_some()
    }

    /// A message received in a conversation that is not in sight
    /// (`in_sight`: its tab shown in the Conversations window): a toast, and
    /// when asked for an entry under the bell until the conversation is
    /// read, as notify_of_message decides (llimview.cpp) — never in do not
    /// disturb with the Legacy Viewer's toasts, group chats by their own
    /// preference.
    pub fn notify_message(&mut self, m: &ImMessage, in_sight: bool) {
        let prefs = self.notifications.toasts;
        let wanted = match m.kind {
            ImKind::Group => prefs.group_im,
            ImKind::Private | ImKind::Conference => prefs.im,
        };
        // Aurora's paused bell still keeps them in "To do", silently; a
        // toast alone has nothing to show then
        if !wanted || in_sight || (self.status.dnd && !(self.notifications.aurora && prefs.im_in_bell)) {
            return;
        }
        let Some(line) = self.resident_line(m) else {
            return;
        };
        let (from, text) = (line.source, line.text.clone());
        let group = m.kind != ImKind::Private;
        let (title, body) = if group {
            (self.session_title(&m.session), format!("{}: {text}", line.from))
        } else {
            (line.from.clone(), text)
        };
        let notification = self.notifications.push_message(m.session, from, title, body, prefs.im_in_bell);
        self.notifications
            .link_name(notification, self.social.name_of(&from), notifications::NameTarget::Avatar(from));
        if m.kind == ImKind::Group {
            self.notifications.link_name(
                notification,
                self.session_title(&m.session),
                notifications::NameTarget::Group(m.session),
            );
        }
    }

    /// A friend's or a message's card clicked: it is acknowledged (gone from
    /// the bell, the conversation read) and the conversation to show is
    /// returned — the friend's IM (LLAvatarActions::startIM) or the session
    /// of the messages (FSFloaterIM::show).
    pub fn open_from_notification(&mut self, id: u64) -> Option<Uuid> {
        let n = self.notifications.list.iter().find(|n| n.id == id)?;
        let session = match n.data {
            notifications::Data::Presence { who, online: true } => {
                self.social.session_mut(who);
                who
            }
            notifications::Data::Message { session, .. } => session,
            _ => return None,
        };
        self.notifications.list.retain(|n| n.id != id);
        if let Some(s) = self.social.ims.iter_mut().find(|s| s.other == session) {
            s.unread = 0;
        }
        self.acknowledge_read_messages();
        self.social.focus_im = Some(session);
        Some(session)
    }

    /// Each frame: the file of the history once read, and what left the
    /// notification list since.
    pub fn record_notification_history(&mut self) {
        self.notification_history.poll();
        self.close_orphan_inventory_offers();
        for e in self.notifications.take_departures() {
            self.notification_history.add(e);
        }
    }

    /// The message entries whose conversation has been read leave the bell.
    pub fn acknowledge_read_messages(&mut self) {
        let ims = &self.social.ims;
        self.notifications
            .acknowledge_messages(|s| ims.iter().find(|i| i.other == *s).is_none_or(|i| i.unread == 0));
    }

    /// Complete name of an avatar ("Jane Doe (janedoe)"), else its legacy
    /// name (name-values, UUIDNameReply).
    pub fn person_name(&self, id: &Uuid) -> Option<String> {
        self.social
            .avatar_names
            .complete(id)
            .or_else(|| self.avatar_name(id))
            .or_else(|| self.social.names.get(id).cloned())
    }

    pub fn region_name(&self) -> String {
        self.main().map(|r| r.name.clone()).unwrap_or_default()
    }

    /// The agent's region granted the capability `name`
    /// (LLViewerRegion::isCapabilityAvailable).
    pub fn main_has_cap(&self, name: &str) -> bool {
        self.main().is_some_and(|r| r.caps.contains_key(name))
    }

    /// The status feed and page the agent's region names for its OpenSim
    /// grid (LFSimFeatureHandler::gridStatusRSS, gridStatusURL).
    pub fn grid_status_extras(&self) -> Option<(&str, &str)> {
        self.main().map(|r| (r.grid_status.0.as_str(), r.grid_status.1.as_str()))
    }

    /// `id` talked with us now (LLRecentPeople::add), here.
    pub fn met(&mut self, id: Uuid) {
        let region = if self.rlva.has("showloc") {
            String::new()
        } else {
            self.region_name()
        };
        let group = self.groups.is_member(&id);
        self.recent.add(self.agent_id, id, group, &region, std::time::SystemTime::now());
    }

    /// Fraction of the main region's terrain received.
    pub fn terrain_progress(&self) -> f32 {
        self.main().map(|r| r.heightmap.received_fraction()).unwrap_or(0.0)
    }

    /// Our own avatar name (display name once known).
    pub fn own_name(&self) -> String {
        let names = &self.social.avatar_names;
        if let Some(n) = names.get(&self.agent_id) {
            return n.display(&names.options);
        }
        self.login
            .as_ref()
            .map(|l| social::format_name(&l.first_name, &l.last_name))
            .unwrap_or_default()
    }

    /// Record an IM we sent.
    pub fn own_im(&mut self, to: Uuid, text: &str) {
        let text = self.rlva_outgoing_im_text(to, false, text).into_owned();
        let me = self
            .login
            .as_ref()
            .map(|l| social::format_name(&l.first_name, &l.last_name))
            .unwrap_or_default();
        self.social.session_mut(to);
        let line = ChatLine {
            time: SystemTime::now(),
            from: me,
            text,
            kind: ChatKind::OWN,
            source: Uuid::nil(),
            origin: Origin::Live,
        };
        self.conversation_push(to, line, false);
    }

    /// Add a line said at its own time to a conversation, opening it (the
    /// demo's conversations over several days).
    pub fn conversation_line(&mut self, session: Uuid, line: ChatLine) {
        self.social.session_mut(session);
        self.conversation_push(session, line, false);
    }

    /// Our teleport offer (send_lures, llviewermessage.cpp): the invitation
    /// followed by the SLURL of where we stand, one StartLure for every
    /// invitee, and the TeleportOfferSent notice of each.
    pub fn offer_teleport(&mut self, to: Vec<Uuid>, message: &str) -> aurora_net::NetCommand {
        let message = format!("{message}\r\n{}", crate::slurl::make(&self.region_name(), self.agent.position));
        for id in &to {
            self.teleport_offer_sent(*id);
        }
        aurora_net::NetCommand::OfferTeleport { to, message }
    }

    /// A teleport offer went out (send_lures, llviewermessage.cpp): the
    /// TeleportOfferSent notice ("Teleport offer sent to <name>") goes to the
    /// conversation with that avatar without counting as unread, or only to
    /// its transcript when none is open. Never to the nearby chat nor a
    /// toast (log_to_im, not log_to_chat, show_toast false in
    /// notifications.xml).
    pub fn teleport_offer_sent(&mut self, to: Uuid) {
        let text = aurora_i18n::tr!(
            "common-teleport-offer-sent",
            name = format!("secondlife:///app/agent/{to}/completename")
        );
        self.note_with(to, text);
    }

    /// Turn the drawn bodies of the standing avatars toward where they face
    /// or travel (LLVOAvatar::updateRootPositionAndRotation). Returns our
    /// own avatar's turn (+1 left, -1 right, 0) for the control flags.
    pub fn update_bodies(&mut self, now: Instant, dt: f32, cam_at: Vec3, mouselook: bool, step_back: bool) -> i8 {
        let mut own_turn = 0;
        let mut seen = std::collections::HashSet::new();
        for (_, o) in self.objects.avatars() {
            if o.parent_id != 0 {
                continue;
            }
            let id = o.full_id;
            let is_self = id == self.agent_id && self.agent.has_local_control();
            // the walk the simulator signals (mSignaledAnimations), ours
            // with Aurora's movement
            let walking = step_back && {
                let signals = if id == self.agent_id {
                    self.own_signals()
                } else {
                    self.animations.get(&id).map_or(&[][..], |v| v.as_slice())
                };
                signals.iter().any(|a| a.id == crate::own_motion::ANIM_WALK)
            };
            let input = if is_self {
                let a = &self.agent;
                // (with Aurora's movement, the velocity it is drawn at)
                let velocity = a.drawn_velocity();
                body::Input {
                    prim_dir: a.forward(),
                    velocity,
                    in_air: a.flying || velocity.z.abs() > 1.0,
                    mouselook_at: mouselook.then_some(cam_at),
                    flying: a.flying,
                    step_back: walking,
                }
            } else {
                body::Input {
                    prim_dir: self.objects.predicted(o, now).1 * Vec3::X,
                    velocity: o.velocity,
                    in_air: o.velocity.z.abs() > 1.0,
                    mouselook_at: None,
                    flying: false,
                    step_back: walking,
                }
            };
            let no_rotate = self
                .animations
                .get(&id)
                .is_some_and(|l| l.iter().any(|a| body::NO_ROTATE_ANIMS.contains(&a.id)));
            seen.insert(id);
            let b = self.bodies.entry(id).or_insert_with(|| body::Body::new(input.prim_dir));
            if no_rotate {
                continue;
            }
            let turn = b.update(&input, dt);
            if is_self {
                own_turn = turn;
            }
        }
        self.bodies.retain(|id, _| seen.contains(id));
        own_turn
    }

    /// LLHUDEffectResetSkeleton::unpackData / update: a reset is followed
    /// only when an avatar we see asks it for itself. Our own effect, if the
    /// simulator ever sends it back, was already applied when we sent it
    /// (it would otherwise stop our animations a second time).
    fn skeleton_reset_from(&self, source: Uuid, target: Uuid, animations: bool) -> Option<(Uuid, bool)> {
        let is_avatar = |id: &Uuid| {
            self.objects
                .index_of_uuid(id)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|o| o.is_avatar())
        };
        (source != self.agent_id && source == target && is_avatar(&source)).then_some((target, animations))
    }

    pub fn animations_of(&self, owner: &Uuid) -> &[PlayingAnimation] {
        if *owner == self.agent_id {
            if let Some(pose) = &self.ao_pose {
                return pose;
            }
            return self.own_signals();
        }
        self.animations.get(owner).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// AOEngine::foreignAnimations: only the seat/linkset can cancel smart
    /// sits; attachments and unrelated scripted objects must not cancel them.
    pub fn seat_has_animations(&self) -> bool {
        let Some(avatar) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return false;
        };
        let Some(seat) = avatar.parent_key() else {
            return false;
        };
        self.animation_sources.keys().any(|source| {
            let Some(mut object) = self.objects.index_of_uuid(source).and_then(|i| self.objects.get(i)) else {
                return false;
            };
            let mut visited = HashSet::new();
            while visited.insert(object.key) {
                if object.is_avatar() {
                    return false;
                }
                if object.key == seat {
                    return true;
                }
                let Some(next) = self.objects.parent_of(object).and_then(|i| self.objects.get(i)) else {
                    return false;
                };
                object = next;
            }
            false
        })
    }

    /// LLPreviewAnim::play locally: keep the motion across server signals,
    /// and restore a server-owned motion with the same UUID when stopping.
    pub fn preview_animation(&mut self, id: Uuid, start: bool) {
        if start && let Some(previous) = self.local_preview {
            self.preview_animation(previous.id, false);
        }
        let motions = self.animations.entry(self.agent_id).or_default();
        if start {
            self.animation_preview_clock = None;
            self.preview_server_motion = motions.iter().find(|a| a.id == id).copied();
            let preview = PlayingAnimation::from_signal(id, 0, Instant::now(), None);
            self.local_preview = Some(preview);
            motions.retain(|a| a.id != id);
            motions.push(preview);
        } else if self.local_preview.is_some_and(|a| a.id == id) {
            self.animation_preview_clock = None;
            motions.retain(|a| a.id != id);
            if let Some(motion) = self.preview_server_motion.take() {
                motions.push(motion);
            }
            self.local_preview = None;
        }
    }

    /// LLFloaterBvhPreview::onSliderMove/onBtnPlayPause (LGPL 2.1):
    /// keep preview time independent of server motion signals.
    pub fn set_animation_preview_clock(&mut self, id: Uuid, start: Instant, sample: Instant) {
        self.animation_preview_clock = self.local_preview.filter(|p| p.id == id).map(|_| (id, start, sample));
    }

    /// LLVOAvatar::getOffObject and LLVOAvatarSelf::detachObject
    /// (indra/newview/llvoavatar.cpp / llvoavatarself.cpp, LGPL 2.1): release
    /// only the seat/attachment and its linked prims, keeping unrelated
    /// sources (including what the seated avatar wears) intact.
    fn release_linkset_camera(&mut self, root: usize, clear: bool) {
        let Some(object) = self.objects.get(root) else {
            return;
        };
        let children = self.objects.children_of(&object.key).iter().copied();
        for idx in std::iter::once(root).chain(children) {
            if let Some(object) = self.objects.get(idx).filter(|o| !o.is_avatar()) {
                if clear {
                    self.scripted_camera.clear(object.full_id);
                } else {
                    self.scripted_camera.deactivate(object.full_id);
                }
            }
        }
    }

    /// Run before upsert unlinks the old parent. Local release is mandatory
    /// even when no ClearFollowCamProperties arrives from the simulator.
    fn on_camera_parent_change(&mut self, region: RegionHandle, update: &aurora_net::objects::ObjectUpdate) {
        if !self.scripted_camera.has_active() || (update.pcode != aurora_prim::params::LL_PCODE_LEGACY_AVATAR && update.parent_id != 0) {
            return;
        }
        let Some(idx) = self.objects.index_of_uuid(&update.full_id) else {
            return;
        };
        let Some(object) = self.objects.get(idx) else {
            return;
        };
        let new_parent = (update.parent_id != 0).then_some(ObjKey {
            region,
            local_id: update.parent_id,
        });
        if object.parent_key() == new_parent {
            return;
        }
        let Some(parent) = self.objects.parent_of(object) else {
            return;
        };
        if object.is_avatar() {
            // The seat can be rekeyed during a crossing; parent_key follows
            // that rekey above, so a still-seated update releases nothing.
            self.release_linkset_camera(parent, false);
        } else if update.parent_id == 0
            && self
                .objects
                .get(parent)
                .is_some_and(|p| p.is_avatar() && p.full_id == self.agent_id)
        {
            self.release_linkset_camera(idx, false);
        }
    }

    /// LLViewerObject::markDead (indra/newview/llviewerobject.cpp, LGPL 2.1):
    /// the avatars sitting on a linkset that leaves the world (killed,
    /// derendered) do not go with it. They get off (LLVOAvatar::getOffObject)
    /// and stay where they are drawn, with what they wear. Returns true when
    /// one of them is ours.
    pub(super) fn get_off_linkset(&mut self, root: usize) -> bool {
        let now = Instant::now();
        let mut seated = Vec::new();
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            let Some(o) = self.objects.get(i) else {
                continue;
            };
            for &k in self.objects.children_of(&o.key) {
                if self.objects.get(k).is_some_and(|c| c.is_avatar()) {
                    seated.push(k);
                } else {
                    stack.push(k);
                }
            }
        }
        let mut ours = false;
        for i in seated {
            ours |= self.get_off(i, now);
        }
        ours
    }

    /// LLVOAvatar::getOffObject (indra/newview/llvoavatar.cpp, LGPL 2.1) of
    /// the avatar at `i`: unparented where it is drawn, our own seat's
    /// motions stopped while the seat is still known. True when it is ours.
    fn get_off(&mut self, i: usize, now: Instant) -> bool {
        let pose = crate::scene::Scene::object_transform_unshifted(self, i, now);
        let Some(o) = self.objects.get(i) else {
            return false;
        };
        let (ours, region) = (o.full_id == self.agent_id, o.key.region);
        if ours {
            self.on_seat_parent_change(region, 0);
        }
        let origin = self.region_offset(region);
        self.objects.detach(i);
        if let (Some((pos, rot, _)), Some(origin), Some(o)) = (pose, origin, self.objects.get_mut(i)) {
            o.position = pos - origin;
            o.rotation = rot;
        }
        ours
    }

    /// LLAgent::teleportCore (indra/newview/llagent.cpp, LGPL 2.1): "force
    /// stand up and stop a sitting animation (if any), see MAINT-3969".
    /// Every teleport we ask for gets our avatar off its seat here: the
    /// simulator unseats it, but its later updates may be terse ones, which
    /// carry no parent, and the old seat would hold the avatar seated after
    /// the arrival. The seat's scripted camera lets go
    /// (LLFollowCamMgr::setCameraActive) and the agent stands where it was
    /// drawn. A sit on the ground has no parent and stays to the simulator.
    fn get_off_for_teleport(&mut self) {
        let Some(i) = self.objects.index_of_uuid(&self.agent_id) else {
            return;
        };
        let Some(seat) = self
            .objects
            .get(i)
            .filter(|o| o.is_avatar() && o.parent_id != 0)
            .and_then(|o| self.objects.parent_of(o))
        else {
            return;
        };
        let mut root = seat;
        while let Some(parent) = self.objects.get(root).and_then(|o| self.objects.parent_of(o)) {
            root = parent;
        }
        self.release_linkset_camera(root, false);
        self.get_off(i, Instant::now());
        let pos = self
            .objects
            .get(i)
            .and_then(|o| Some(o.position + self.region_offset(o.key.region)?));
        if let Some(pos) = pos {
            self.agent.get_off(pos);
        }
        log::info!("teleport: our avatar gets off its seat (teleportCore)");
    }

    /// Port of LLVOAvatar::getOffObject / LLVOAvatarSelf::stopMotionFromSource
    /// (indra/newview/llvoavatar.cpp, llvoavatarself.cpp, LGPL 2.1). Run before
    /// replacing our object's parent, while the old seat/linkset is known.
    fn on_seat_parent_change(&mut self, region: RegionHandle, parent: u32) {
        let Some(avatar) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return;
        };
        // (the same seat under its new key when both crossed a border)
        let Some(seat_key) = avatar.parent_key().filter(|k| *k != ObjKey { region, local_id: parent }) else {
            return;
        };
        let seat = self.objects.index_of(&seat_key).into_iter();
        let children = self.objects.children_of(&seat_key).iter().copied();
        let sources: Vec<_> = seat
            .chain(children)
            .filter_map(|idx| self.objects.get(idx).map(|o| o.full_id))
            .collect();
        for source in sources {
            self.stop_motion_from_source(source);
        }
    }

    /// LLVOAvatarSelf::detachObject: only the linkset, never an avatar
    /// seated on it or that avatar's attachments.
    fn stop_linkset_motions(&mut self, root: usize) {
        let Some(object) = self.objects.get(root) else {
            return;
        };
        let children = self.objects.children_of(&object.key).iter().copied();
        let sources: Vec<_> = std::iter::once(root)
            .chain(children)
            .filter_map(|idx| self.objects.get(idx).filter(|o| !o.is_avatar()).map(|o| o.full_id))
            .collect();
        for source in sources {
            self.stop_motion_from_source(source);
        }
    }

    /// Whether `source` still plays `animation` on our avatar (the
    /// explorer's "Still playing", AnimationExplorer::updateList).
    pub fn animation_playing_from(&self, source: &Uuid, animation: &Uuid) -> bool {
        self.animation_sources
            .get(source)
            .is_some_and(|motions| motions.contains(animation))
    }

    /// process_avatar_animation's asset blacklist (FSAssetBlacklist,
    /// indra/newview/llviewermessage.cpp): a blacklisted motion is never
    /// played on an avatar, and each signal of it asks the simulator to
    /// stop it on ours, so the people around do not see it either (once
    /// per frame at most). The source blocks stay aligned
    /// with the motions they name. Animesh (ObjectAnimation) is left as is,
    /// like the Legacy Viewer's process_object_animation.
    fn drop_blacklisted_animations(&mut self, avatar: Uuid, anims: &mut Vec<(Uuid, i32)>, sources: &mut Vec<Uuid>) {
        if self.derender.is_empty() || !anims.iter().any(|(id, _)| self.derender.animation_blocked(id)) {
            return;
        }
        let animesh = self
            .objects
            .index_of_uuid(&avatar)
            .and_then(|i| self.objects.get(i))
            .is_some_and(|o| !o.is_avatar());
        if animesh {
            return;
        }
        let mut index = 0;
        let mut kept_sources = Vec::with_capacity(sources.len());
        anims.retain(|(id, _)| {
            let i = index;
            index += 1;
            if self.derender.animation_blocked(id) {
                if !self.animation_stops.contains(id) {
                    self.animation_stops.push(*id);
                }
                return false;
            }
            if let Some(source) = sources.get(i) {
                kept_sources.push(*source);
            }
            true
        });
        *sources = kept_sources;
    }

    /// LLVOAvatarSelf::stopMotionFromSource and LLViewerObject::markDead
    /// (indra/newview/llvoavatarself.cpp / llviewerobject.cpp, LGPL 2.1).
    /// Request the simulator's stop; its animation signal remains authoritative.
    fn stop_motion_from_source(&mut self, source: Uuid) {
        if let Some(motions) = self.animation_sources.remove(&source) {
            for id in motions {
                if !self.animation_stops.contains(&id) {
                    log::info!("animation source released: stop {id} from {source}");
                    self.animation_stops.push(id);
                }
            }
        }
    }
}

/// What becomes of an AvatarAppearance message.
#[derive(Debug, PartialEq, Eq)]
enum AppearanceVerdict {
    Apply,
    /// Our own, for a COF version we already have (or stepped past).
    Stale,
    /// One visual parameter or none; `refresh` for our own once one was
    /// received (we may show as a cloud).
    Empty {
        refresh: bool,
    },
}

/// The checks of LLVOAvatar::processAvatarAppearance
/// (indra/newview/llvoavatar.cpp, originally LGPL 2.1): our own appearance
/// never rolls back to a COF version already received (`received` also
/// counts the version the IncrementCOFVersion answer slammed), server bakes
/// only; a message with one parameter or none says nothing reliable and is
/// dropped for everyone, with a forced refresh for ours.
fn appearance_verdict(a: &AvatarAppearance, own: bool, received: i32) -> AppearanceVerdict {
    if own && a.appearance_version > 0 && received >= a.cof_version {
        return AppearanceVerdict::Stale;
    }
    if a.visual_params.len() <= 1 {
        return AppearanceVerdict::Empty {
            refresh: own && received != -1,
        };
    }
    AppearanceVerdict::Apply
}

#[cfg(test)]
mod parcel_voice_tests {
    use super::*;
    use aurora_net::parcel_flags as pf;

    #[test]
    fn parcel_voice_pushes_only_change_current_voice_flags_and_keep_local_media() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.main_region = Some(7);
        let original = Arc::new(aurora_net::ParcelInfo {
            local_id: 42,
            flags: pf::SOUND_LOCAL | pf::CREATE_OBJECTS | pf::ALLOW_VOICE_CHAT | pf::USE_ESTATE_VOICE_CHAN,
            music_url: "https://music.invalid/local".into(),
            media_url: "https://media.invalid/local".into(),
            bitmap: vec![0x42; 512],
            ..Default::default()
        });
        world.parcel = Some(original.clone());
        let event = |handle, local_id, allowed, use_estate| NetEvent::ParcelVoiceInfo {
            handle,
            local_id,
            allowed,
            use_estate,
        };
        for (handle, local_id) in [(8, 42), (7, 43)] {
            world.apply(event(handle, local_id, false, Some(false)));
            assert!(Arc::ptr_eq(world.parcel.as_ref().unwrap(), &original));
        }
        for (allowed, estate) in [(false, None), (true, Some(false)), (true, Some(true))] {
            let before = world.parcel.clone().unwrap();
            world.apply(event(7, 42, allowed, estate));
            let current = world.parcel.clone().unwrap();
            assert_eq!(current.flags & pf::ALLOW_VOICE_CHAT != 0, allowed);
            assert_eq!(
                current.flags & pf::USE_ESTATE_VOICE_CHAN != 0,
                estate.unwrap_or(before.flags & pf::USE_ESTATE_VOICE_CHAN != 0)
            );
            assert_eq!(
                current.flags & (pf::SOUND_LOCAL | pf::CREATE_OBJECTS),
                pf::SOUND_LOCAL | pf::CREATE_OBJECTS
            );
            assert_eq!(current.music_url, original.music_url);
            assert_eq!(current.media_url, original.media_url);
            assert_eq!(current.bitmap, original.bitmap);
            world.apply(event(7, 42, allowed, estate));
            assert!(Arc::ptr_eq(world.parcel.as_ref().unwrap(), &current));
        }
        world.parcel = None;
        world.apply(event(7, 42, true, Some(true)));
        assert!(world.parcel.is_none(), "voice alone cannot locate the agent parcel");
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::*;

    fn appearance(cof_version: i32, params: usize) -> AvatarAppearance {
        AvatarAppearance {
            avatar_id: Uuid::from_u128(1),
            texture_entry: None,
            visual_params: vec![128; params],
            cof_version,
            appearance_version: 1,
            hover_height: 0.0,
            attachments: 0,
        }
    }

    #[test]
    fn own_appearance_never_rolls_back() {
        assert_eq!(appearance_verdict(&appearance(7, 253), true, -1), AppearanceVerdict::Apply);
        assert_eq!(appearance_verdict(&appearance(7, 253), true, 6), AppearanceVerdict::Apply);
        assert_eq!(appearance_verdict(&appearance(7, 253), true, 7), AppearanceVerdict::Stale);
        assert_eq!(appearance_verdict(&appearance(5, 253), true, 7), AppearanceVerdict::Stale);
        // others are never compared, nor legacy (client) bakes
        assert_eq!(appearance_verdict(&appearance(5, 253), false, 7), AppearanceVerdict::Apply);
        let legacy = AvatarAppearance {
            appearance_version: 0,
            ..appearance(5, 253)
        };
        assert_eq!(appearance_verdict(&legacy, true, 7), AppearanceVerdict::Apply);
    }

    #[test]
    fn appearance_without_parameters_is_dropped() {
        for params in [0, 1] {
            assert_eq!(
                appearance_verdict(&appearance(3, params), false, -1),
                AppearanceVerdict::Empty { refresh: false }
            );
            // ours forces a refresh once a version was received
            assert_eq!(
                appearance_verdict(&appearance(3, params), true, -1),
                AppearanceVerdict::Empty { refresh: false }
            );
            assert_eq!(
                appearance_verdict(&appearance(3, params), true, 2),
                AppearanceVerdict::Empty { refresh: true }
            );
        }
        assert_eq!(appearance_verdict(&appearance(3, 2), false, -1), AppearanceVerdict::Apply);
    }

    #[test]
    fn world_keeps_the_newer_own_shape() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        let own = world.agent_id;
        let send = |world: &mut World, cof_version: i32, value: u8, params: usize| {
            world.apply(NetEvent::Appearance(AvatarAppearance {
                avatar_id: own,
                visual_params: vec![value; params],
                ..appearance(cof_version, params)
            }));
        };
        send(&mut world, 4, 10, 253);
        send(&mut world, 3, 20, 253);
        send(&mut world, 4, 30, 253);
        send(&mut world, 5, 40, 0);
        assert_eq!(world.visual_params(&own).map(|v| v[0]), Some(10));
        assert_eq!(world.appearance_cof_version(&own), 4);
        send(&mut world, 5, 50, 253);
        assert_eq!(world.visual_params(&own).map(|v| v[0]), Some(50));
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use aurora_net::objects::TerseUpdate;

    #[test]
    fn animation_signals_keep_continuous_time_until_an_observed_stop() {
        let id = Uuid::from_u128(17);
        let t0 = Instant::now();
        let t1 = t0 + std::time::Duration::from_secs(1);
        let first = PlayingAnimation::from_signal(id, 1, t0, None);
        assert_eq!(PlayingAnimation::from_signal(id, 1, t1, Some(&first)), first);
        let changed = PlayingAnimation::from_signal(id, 2, t1, Some(&first));
        assert_eq!(changed.sequence_start, t1);
        assert_eq!(changed.continuous_start, t0);
        let restart = PlayingAnimation::from_signal(id, 2, t1, None);
        assert_eq!(restart.sequence_start, t1);
        assert_eq!(restart.continuous_start, t1);
    }

    #[test]
    fn animation_network_updates_preserve_loop_clock_and_record_interframe_stop() {
        let (mut world, _, _) = fixture();
        let avatar = world.agent_id;
        let id = Uuid::from_u128(18);
        world.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar,
            anims: vec![(id, 1)],
        });
        // Deterministic earlier clock: no sleeps or dependence on timer resolution.
        let early = Instant::now() - std::time::Duration::from_secs(1);
        let previous = &mut world.animations.get_mut(&avatar).expect("signal")[0];
        previous.sequence_start = early;
        previous.continuous_start = early;
        world.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar,
            anims: vec![(id, 2)],
        });
        let changed = world.animations_of(&avatar)[0];
        assert_eq!(changed.continuous_start, early);
        assert!(changed.sequence_start > early);
        world.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar,
            anims: Vec::new(),
        });
        world.apply(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar,
            anims: vec![(id, 2)],
        });
        let restarted = world.animations_of(&avatar)[0];
        assert_eq!(restarted.sequence_start, restarted.continuous_start);
        assert!(restarted.continuous_start > early);
    }

    fn fixture() -> (World, aurora_net::objects::ObjectUpdate, RegionHandle) {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        let me = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
        world.agent_id = me;
        let (handle, update) = crate::demo::events()
            .into_iter()
            .find_map(|ev| {
                if let NetEvent::ObjectUpdates { handle, objects } = ev {
                    objects.into_iter().find(|o| o.full_id == me).map(|o| (handle, o))
                } else {
                    None
                }
            })
            .expect("demo contains our avatar");
        world.main_region = Some(handle);
        world.ensure_region(handle, (256, 256));
        (world, update, handle)
    }

    fn teleport_fixture() -> (World, RegionHandle) {
        let (mut world, _, handle) = fixture();
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: Vec3::new(40.0, 50.0, 30.0),
            look_at: Vec3::X,
        });
        world.parcel_name = aurora_i18n::tr!("world-mod-demo-parcel").into();
        (world, handle)
    }

    fn seated_fixture() -> (World, aurora_net::objects::ObjectUpdate, RegionHandle, Uuid, Uuid) {
        let (mut world, mut avatar, handle) = fixture();
        let mut seat = avatar.clone();
        seat.full_id = Uuid::from_u128(950);
        seat.local_id = 950;
        seat.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        let mut child = seat.clone();
        child.full_id = Uuid::from_u128(951);
        child.local_id = 951;
        child.parent_id = seat.local_id;
        let (root, linked) = (seat.full_id, child.full_id);
        let mut ao = seat.clone();
        ao.full_id = Uuid::from_u128(999);
        ao.local_id = 999;
        ao.parent_id = avatar.local_id;
        avatar.parent_id = seat.local_id;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![seat, child, ao, avatar.clone()],
        });
        (world, avatar, handle, root, linked)
    }

    fn camera_source(world: &mut World, handle: RegionHandle, object: Uuid, locked: bool) {
        world.apply(NetEvent::SetFollowCam {
            handle,
            object,
            properties: vec![(12, 1.0), (21, f32::from(u8::from(locked)))],
        });
    }

    #[test]
    fn the_frozen_world_keeps_what_others_derez_until_the_thaw() {
        let (mut world, me, handle) = fixture();
        let mut prop = me.clone();
        prop.full_id = Uuid::from_u128(970);
        prop.local_id = 970;
        prop.parent_id = 0;
        prop.owner_id = Uuid::from_u128(971);
        prop.update_flags = 0;
        prop.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        let mut reused = prop.clone();
        reused.full_id = Uuid::from_u128(972);
        reused.local_id = 972;
        let mut mine = prop.clone();
        mine.full_id = Uuid::from_u128(973);
        mine.local_id = 973;
        mine.update_flags = crate::ui::context::flags::OBJECT_YOU_OWNER;
        let mut worn = prop.clone();
        worn.full_id = Uuid::from_u128(974);
        worn.local_id = 974;
        worn.parent_id = me.local_id;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![me.clone(), prop.clone(), reused.clone(), mine.clone(), worn.clone()],
        });
        let present = |world: &World, id: u128| world.objects.index_of_uuid(&Uuid::from_u128(id)).is_some();
        world.freeze_motion(Some(Instant::now()));
        world.apply(NetEvent::ObjectsKilled {
            handle,
            local_ids: vec![970, 972, 973, 974],
        });
        // what the user does goes at once, the rest is held
        assert!(present(&world, 970) && present(&world, 972));
        assert!(!present(&world, 973) && !present(&world, 974));
        assert!(world.objects.killed.iter().flatten().count() >= 2);
        // a new object at a held key is not the one that left
        let mut newcomer = reused.clone();
        newcomer.full_id = Uuid::from_u128(975);
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![newcomer],
        });
        world.freeze_motion(None);
        assert!(!present(&world, 970));
        assert!(present(&world, 975));
        assert!(world.held_kills.is_empty());
    }

    #[test]
    fn a_killed_seat_leaves_who_sat_on_it_with_what_they_wear() {
        let (mut world, avatar, handle, root, linked) = seated_fixture();
        // another resident on the same seat, wearing something
        let mut other = avatar.clone();
        other.full_id = Uuid::from_u128(960);
        other.local_id = 960;
        let mut worn = other.clone();
        worn.full_id = Uuid::from_u128(961);
        worn.local_id = 961;
        worn.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        worn.parent_id = other.local_id;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![other.clone(), worn.clone()],
        });
        world.objects.killed.clear();
        // the seat dies before the simulator stands them up
        world.apply(NetEvent::ObjectsKilled {
            handle,
            local_ids: vec![950],
        });
        assert!(world.objects.index_of_uuid(&root).is_none());
        assert!(world.objects.index_of_uuid(&linked).is_none());
        for (sitter, attachment) in [(avatar.full_id, Uuid::from_u128(999)), (other.full_id, worn.full_id)] {
            let idx = world.objects.index_of_uuid(&sitter).expect("the avatar stays");
            let o = world.objects.get(idx).expect("avatar");
            assert_eq!(o.parent_id, 0, "it got off");
            let kids = world.objects.children_of(&o.key).to_vec();
            assert_eq!(kids, vec![world.objects.index_of_uuid(&attachment).expect("what it wears stays")]);
        }
        let gone: Vec<Uuid> = world
            .objects
            .killed
            .iter()
            .flatten()
            .filter_map(|&i| world.objects.get(i).map(|o| o.full_id))
            .collect();
        assert!(gone.is_empty(), "only the seat's prims leave the scene: {gone:?}");
    }

    #[test]
    fn standing_deactivates_seat_and_linked_cameras_without_a_clear_packet() {
        let (mut world, mut avatar, handle, root, linked) = seated_fixture();
        camera_source(&mut world, handle, root, true);
        camera_source(&mut world, handle, linked, true);
        avatar.parent_id = 0;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar.clone()],
        });
        assert!(!world.scripted_camera.has_active());
        // Stand releases both sources but keeps the parameters of each one.
        for source in [root, linked] {
            world.apply(NetEvent::SetFollowCam {
                handle,
                object: source,
                properties: vec![(0, 20.0)],
            });
            assert!(!world.scripted_camera.has_active());
            world.apply(NetEvent::SetFollowCam {
                handle,
                object: source,
                properties: vec![(12, 1.0)],
            });
            assert!(world.scripted_camera.active().expect("retained camera").position_locked);
            world.scripted_camera.deactivate(source);
        }
        // Repeated standing updates cannot revoke a subsequently active source.
        camera_source(&mut world, handle, root, true);
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert!(world.scripted_camera.has_active());
    }

    #[test]
    fn standing_preserves_an_active_attachment_camera_and_same_seat_updates() {
        let (mut world, mut avatar, handle, root, linked) = seated_fixture();
        let worn = Uuid::from_u128(999);
        camera_source(&mut world, handle, worn, false);
        camera_source(&mut world, handle, root, true);
        camera_source(&mut world, handle, linked, true);
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar.clone()],
        });
        assert!(world.scripted_camera.active().expect("still seated").position_locked);
        avatar.parent_id = 0;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert!(!world.scripted_camera.active().expect("attachment resumes").position_locked);
    }

    #[test]
    fn changing_seats_releases_the_old_linkset_and_preserves_the_new_source() {
        let (mut world, mut avatar, handle, root, linked) = seated_fixture();
        camera_source(&mut world, handle, root, true);
        camera_source(&mut world, handle, linked, true);
        let other = Uuid::from_u128(952);
        camera_source(&mut world, handle, other, false);
        avatar.parent_id = 952;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert!(!world.scripted_camera.active().expect("new seat camera").position_locked);
        world.apply(NetEvent::ClearFollowCam { object: other });
        assert!(!world.scripted_camera.has_active(), "neither old seat source can resume");
    }

    #[test]
    fn detaching_an_attachment_releases_its_root_and_linked_cameras() {
        let (mut world, avatar, handle, root, _) = seated_fixture();
        let mut worn = avatar.clone();
        worn.full_id = Uuid::from_u128(999);
        worn.local_id = 999;
        worn.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        worn.parent_id = avatar.local_id;
        let mut child = worn.clone();
        child.full_id = Uuid::from_u128(1000);
        child.local_id = 1000;
        child.parent_id = worn.local_id;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![worn.clone(), child.clone()],
        });
        camera_source(&mut world, handle, root, false);
        camera_source(&mut world, handle, worn.full_id, true);
        camera_source(&mut world, handle, child.full_id, true);
        worn.parent_id = 0;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![worn],
        });
        assert!(!world.scripted_camera.active().expect("seat resumes").position_locked);
        world.apply(NetEvent::ClearFollowCam { object: root });
        assert!(!world.scripted_camera.has_active());
        world.apply(NetEvent::SetFollowCam {
            handle,
            object: child.full_id,
            properties: vec![(12, 1.0)],
        });
        assert!(world.scripted_camera.active().expect("retained detached params").position_locked);
    }

    #[test]
    fn deleting_a_source_clears_its_linked_camera_params_and_preserves_an_unrelated_source() {
        let (mut world, _, handle, root, linked) = seated_fixture();
        let other = Uuid::from_u128(777);
        camera_source(&mut world, handle, other, false);
        camera_source(&mut world, handle, root, true);
        camera_source(&mut world, handle, linked, true);
        world.apply(NetEvent::ObjectsKilled {
            handle,
            local_ids: vec![950],
        });
        assert!(!world.scripted_camera.active().expect("unrelated camera survives").position_locked);
        world.apply(NetEvent::ClearFollowCam { object: other });
        assert!(!world.scripted_camera.has_active());
        for source in [root, linked] {
            world.apply(NetEvent::SetFollowCam {
                handle,
                object: source,
                properties: vec![(12, 1.0)],
            });
            assert!(!world.scripted_camera.active().expect("fresh default params").position_locked);
        }
    }

    #[test]
    fn crossing_with_a_rekeyed_seat_does_not_deactivate_its_camera() {
        let (mut world, mut avatar, handle, root, _) = seated_fixture();
        camera_source(&mut world, handle, root, true);
        let mut seat = avatar.clone();
        seat.full_id = root;
        seat.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        seat.local_id = 1950;
        seat.parent_id = 0;
        let there = handle + (256u64 << 32);
        avatar.local_id = 19000;
        avatar.parent_id = seat.local_id;
        world.apply(NetEvent::ObjectUpdates {
            handle: there,
            objects: vec![seat, avatar],
        });
        assert!(world.scripted_camera.active().expect("vehicle camera preserved").position_locked);
    }

    #[test]
    fn standing_requests_only_seat_and_linked_prim_animation_stops_once() {
        let (mut world, mut avatar, handle, root, linked) = seated_fixture();
        let (sit, hands, ao) = (Uuid::from_u128(71), Uuid::from_u128(72), Uuid::from_u128(73));
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(sit, 1), (hands, 1), (ao, 1), (Uuid::from_u128(74), 1)],
            sources: vec![root, linked, Uuid::from_u128(999)],
        });
        // A sequence change/sparse source list must not forget the seat.
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(sit, 2), (hands, 2), (ao, 2)],
            // The same motion can also be associated with a linked prim.
            sources: vec![linked],
        });
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar.clone()],
        });
        assert!(world.animation_stops.is_empty(), "still on the same seat");
        avatar.parent_id = 0;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar.clone()],
        });
        assert_eq!(world.animation_stops.len(), 2);
        assert!(world.animation_stops.contains(&sit) && world.animation_stops.contains(&hands));
        assert!(!world.animation_stops.contains(&ao));
        assert!(world.animation_sources.contains_key(&Uuid::from_u128(999)));
        world.animation_stops.clear();
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert!(world.animation_stops.is_empty(), "repeated standing packet");
    }

    #[test]
    fn animation_explorer_logs_new_sources_once_and_blacklisted_motions_never_play() {
        let (mut world, _, _, root, linked) = seated_fixture();
        let (sit, hands, blocked) = (Uuid::from_u128(71), Uuid::from_u128(72), Uuid::from_u128(73));
        let signal = |world: &mut World, anims: Vec<(Uuid, i32)>, sources: Vec<Uuid>| {
            world.apply(NetEvent::AvatarAnimations {
                avatar: world.agent_id,
                anims,
                sources,
            });
        };
        signal(&mut world, vec![(sit, 1), (hands, 1)], vec![root, linked]);
        signal(&mut world, vec![(sit, 2), (hands, 2)], vec![root, linked]);
        let logged: Vec<_> = world.animation_log.entries().map(|e| (e.animation, e.source)).collect();
        assert_eq!(logged, [(sit, root), (hands, linked)], "a repeated packet is not a new start");
        assert!(world.animation_playing_from(&root, &sit));
        world.derender.add(
            blocked,
            derender::Entry {
                name: String::new(),
                region: String::new(),
                kind: derender::Kind::Animation,
                date: 0,
                permanent: false,
            },
        );
        // The blocked motion comes first: the sources of the others stay theirs.
        signal(&mut world, vec![(blocked, 3), (sit, 3), (hands, 3)], vec![linked, root, linked]);
        assert!(
            world
                .animations
                .get(&world.agent_id)
                .is_some_and(|list| list.iter().all(|a| a.id != blocked))
        );
        assert_eq!(world.animation_stops, [blocked], "the simulator is asked to stop it");
        assert!(world.animation_playing_from(&root, &sit) && world.animation_playing_from(&linked, &hands));
        assert!(!world.animation_playing_from(&linked, &blocked));
        assert_eq!(world.animation_log.entries().count(), 2);
        signal(&mut world, vec![(hands, 4)], vec![linked]);
        assert!(!world.animation_playing_from(&root, &sit), "a stopped motion is no longer playing");
    }

    #[test]
    fn ao_smart_sit_uses_only_the_seat_and_linkset_sources() {
        let (mut world, mut avatar, handle, root, linked) = seated_fixture();
        for (source, expected) in [(Uuid::from_u128(999), false), (root, true), (linked, true)] {
            world.animation_sources.clear();
            world.apply(NetEvent::AvatarAnimations {
                avatar: world.agent_id,
                anims: vec![(Uuid::from_u128(77), 1)],
                sources: vec![source],
            });
            assert_eq!(world.seat_has_animations(), expected);
        }
        avatar.parent_id = 0;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert!(!world.seat_has_animations());
    }

    #[test]
    fn ao_pose_does_not_replace_network_signals_or_other_avatars() {
        let (mut world, _, _) = fixture();
        let other = Uuid::from_u128(2);
        let stock = crate::ao::State::GroundSit.motion();
        let now = Instant::now();
        world.ao_pose = Some(vec![PlayingAnimation::from_signal(Uuid::from_u128(950), 1, now, None)]);
        for avatar in [world.agent_id, other] {
            world.apply(NetEvent::AvatarAnimations {
                avatar,
                anims: vec![(stock, 1)],
                sources: Vec::new(),
            });
        }
        assert!(world.agent.ground_sit);
        assert_eq!(world.animations[&world.agent_id][0].id, stock);
        assert_eq!(world.animations_of(&world.agent_id)[0].id, Uuid::from_u128(950));
        assert_eq!(world.animations_of(&other)[0].id, stock);
    }

    #[test]
    fn stopped_motion_loses_its_source_before_restart_elsewhere() {
        let (mut world, mut avatar, handle, root, _) = seated_fixture();
        let id = Uuid::from_u128(71);
        for (anims, sources) in [
            (vec![(id, 1)], vec![root]),
            (Vec::new(), Vec::new()),
            (vec![(id, 2)], vec![Uuid::from_u128(999)]),
        ] {
            world.apply(NetEvent::AvatarAnimations {
                avatar: world.agent_id,
                anims,
                sources,
            });
        }
        avatar.parent_id = 0;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert!(world.animation_stops.is_empty());
        assert_eq!(world.animation_sources.len(), 1);
    }

    #[test]
    fn other_avatars_and_excess_source_blocks_do_not_stop_our_motions() {
        let (mut world, mut avatar, handle, root, _) = seated_fixture();
        let id = Uuid::from_u128(71);
        world.apply(NetEvent::AvatarAnimations {
            avatar: Uuid::from_u128(111),
            anims: vec![(id, 1)],
            sources: vec![root],
        });
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(id, 1)],
            sources: vec![Uuid::nil(), root],
        });
        avatar.parent_id = 0;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert!(world.animation_stops.is_empty());
    }

    #[test]
    fn switching_seats_stops_the_previous_seat_motion() {
        let (mut world, mut avatar, handle, root, _) = seated_fixture();
        let id = Uuid::from_u128(71);
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(id, 1)],
            sources: vec![root],
        });
        avatar.parent_id = 952;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        assert_eq!(world.animation_stops, vec![id]);
    }

    /// Seated on the fixture's seat with one of its motions playing and its
    /// scripted camera on; the teleport of `remote` kind starts, then the
    /// simulator's next update of us is a terse one (no parent).
    fn teleport_from_seat(remote: bool) -> (World, Uuid) {
        let (mut world, avatar, handle, root, _) = seated_fixture();
        let motion = Uuid::from_u128(73);
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(motion, 1)],
            sources: vec![root],
        });
        camera_source(&mut world, handle, root, true);
        assert!(world.agent.seated && world.scripted_camera.has_active());
        world.begin_teleport(remote);
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("avatar exists");
        let o = world.objects.get(idx).expect("avatar exists");
        assert_eq!(o.parent_id, 0, "off the seat before the request leaves (teleportCore)");
        assert!(!world.agent.seated && !world.agent.is_sitting());
        assert!(!world.scripted_camera.has_active(), "the seat's camera lets go");
        assert_eq!(world.animation_stops, vec![motion], "the seat's motion is stopped");
        world.apply(NetEvent::TerseUpdates {
            handle,
            updates: vec![TerseUpdate {
                local_id: avatar.local_id,
                state: 0,
                is_avatar: true,
                foot_plane: None,
                position: Vec3::new(40.0, 50.0, 30.0),
                velocity: Vec3::ZERO,
                acceleration: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
                texture_entry: None,
            }],
        });
        (world, root)
    }

    #[test]
    fn a_local_teleport_from_a_seat_arrives_standing() {
        let (mut world, _) = teleport_from_seat(false);
        let handle = world.main_region.expect("region");
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: Vec3::new(40.0, 50.0, 30.0),
            look_at: Vec3::X,
        });
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("avatar exists");
        let o = world.objects.get(idx).expect("avatar exists");
        assert_eq!((o.parent_id, o.position), (0, Vec3::new(40.0, 50.0, 30.0)));
        assert!(!world.agent.is_sitting());
        assert_eq!(world.agent.position, Vec3::new(40.0, 50.0, 30.0));
    }

    #[test]
    fn a_teleport_to_another_region_from_a_seat_leaves_it_too() {
        let (world, root) = teleport_from_seat(true);
        // the seat stays behind with nothing on it
        let seat = world.objects.index_of_uuid(&root).expect("seat stays");
        let key = world.objects.get(seat).expect("seat").key;
        assert!(
            world
                .objects
                .children_of(&key)
                .iter()
                .all(|&c| world.objects.get(c).is_some_and(|o| !o.is_avatar()))
        );
        assert!(!world.agent.is_sitting());
    }

    #[test]
    fn a_teleport_while_standing_changes_nothing_of_the_seat_state() {
        let (mut world, _) = teleport_fixture();
        let before = world.agent.position;
        world.begin_teleport(false);
        assert!(world.animation_stops.is_empty() && !world.agent.is_sitting());
        assert_eq!(world.agent.position, before);
    }

    #[test]
    fn demo_seat_motion_stops_after_the_simulator_receives_our_request() {
        let (mut world, _, _) = fixture();
        for frame in [250, 300, 700] {
            for event in crate::demo::sit_events(frame) {
                world.apply(event);
            }
        }
        assert_eq!(world.animation_stops, vec![crate::demo::SEAT_ANIM]);
        for anim in std::mem::take(&mut world.animation_stops) {
            for event in crate::demo::demo_reply(&aurora_net::NetCommand::AgentAnimation { anim, start: false }) {
                world.apply(event);
            }
        }
        assert!(!world.animations_of(&world.agent_id).iter().any(|a| a.id == crate::demo::SEAT_ANIM));
    }

    #[test]
    fn local_teleport_is_silent_without_progress_and_preserves_arrival_state() {
        let (mut world, handle) = teleport_fixture();
        let previous = world.agent.position;
        world.begin_teleport(false);
        assert!(world.teleporting && !world.tp_show_progress);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
        world.apply(NetEvent::TeleportStarted);
        world.apply(NetEvent::TeleportProgress {
            message: "requesting".into(),
        });
        assert!(world.teleporting && !world.tp_show_progress);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
        world.apply(NetEvent::TeleportLocal { flying: true });
        let position = Vec3::new(128.0, 128.0, 1010.0);
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position,
            look_at: Vec3::Y,
        });
        assert!(!world.teleporting && !world.tp_show_progress);
        assert_eq!(world.agent.position, position);
        assert!(world.agent.flying);
        assert_eq!(world.parcel_name, aurora_i18n::tr!("world-mod-demo-parcel"));
        assert_eq!(world.tp_history.previous().expect("previous arrival").position, previous);
        // Late progress packets must never bring back a local loading screen.
        world.apply(NetEvent::TeleportStarted);
        world.apply(NetEvent::TeleportProgress {
            message: "arriving".into(),
        });
        assert!(!world.tp_show_progress);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
    }

    #[test]
    fn auto_land_clears_only_fly_after_a_grounded_descending_report() {
        use aurora_net::control;
        let (mut world, mut avatar, handle) = arrived_fixture();
        let ground = world.ground_height(avatar.position).unwrap();
        avatar.position.z = ground + avatar.scale.z * 0.5 + 0.1;
        avatar.velocity = Vec3::new(2.0, 1.0, -4.0);
        let input = crate::agent::MoveInput {
            down: true,
            forward: true,
            ..Default::default()
        };
        for predicting in [false, true] {
            world.agent.set_predicting(predicting);
            world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![avatar.clone()],
            });
            world.agent.flying = true;
            let position = world.agent.server_pos;
            let velocity = world.agent.velocity;
            // Rendering and prediction cannot decide whether physics has landed.
            world.agent.position.z += 10.0;
            let before = world.agent.control_flags(&input, false);
            world.auto_land(&input);
            assert!(!world.agent.flying, "prediction {predicting}");
            assert_eq!(world.agent.control_flags(&input, false), before & !control::FLY);
            assert_eq!(before & control::STOP, 0);
            assert_ne!(before & control::UP_NEG, 0);
            assert_eq!(world.agent.server_pos, position);
            assert_eq!(world.agent.velocity, velocity);
        }
    }

    #[test]
    fn auto_land_resolves_platform_and_sloped_contact_from_terse_reports() {
        let (mut world, avatar, handle) = arrived_fixture();
        let ground = world.ground_height(avatar.position).unwrap();
        let input = crate::agent::MoveInput {
            down: true,
            ..Default::default()
        };
        for (floor, normal, lands) in [
            (ground + 100.0, Vec3::Z, true),
            (ground + 100.0, Vec3::new(0.0, 0.6, 0.8), true),
            (ground + 100.0, Vec3::ZERO, false),
            (ground + 100.0, Vec3::X, false),
            (ground + 100.0, -Vec3::Z, false),
            (ground + 100.0, Vec3::splat(f32::NAN), false),
            // An old platform far below us cannot end a later flight.
            (ground, Vec3::Z, false),
        ] {
            let position = avatar.position.with_z(ground + 100.0 + avatar.scale.z * 0.5 + 0.1);
            let point = position.with_z(floor);
            world.apply(NetEvent::TerseUpdates {
                handle,
                updates: vec![TerseUpdate {
                    local_id: avatar.local_id,
                    state: 0,
                    is_avatar: true,
                    foot_plane: Some(normal.extend(normal.dot(point))),
                    position,
                    velocity: Vec3::NEG_Z * 3.0,
                    acceleration: Vec3::ZERO,
                    rotation: glam::Quat::IDENTITY,
                    angular_velocity: Vec3::ZERO,
                    texture_entry: None,
                }],
            });
            world.agent.flying = true;
            world.auto_land(&input);
            assert_eq!(!world.agent.flying, lands, "normal {normal:?}, floor {floor}");
        }
        // A cleared contact plane still permits landing on terrain.
        let idx = world.objects.index_of_uuid(&world.agent_id).unwrap();
        world.objects.get_mut(idx).unwrap().foot_plane = Some(glam::Vec4::ZERO);
        world.agent.server_pos.z = ground + avatar.scale.z * 0.5;
        world.auto_land(&input);
        assert!(!world.agent.flying);
    }

    #[test]
    fn auto_land_requires_descend_ground_contact_and_slow_horizontal_motion() {
        let (mut world, mut avatar, handle) = arrived_fixture();
        let ground = world.ground_height(avatar.position).unwrap();
        for (clearance, velocity, down, up, seated, teleporting, crossing, arrived) in [
            (0.3, Vec3::ZERO, true, false, false, false, false, true),
            (0.0, Vec3::X * 4.0, true, false, false, false, false, true),
            (0.0, Vec3::new(3.0, 3.0, 0.0), true, false, false, false, false, true),
            (0.0, Vec3::splat(f32::NAN), true, false, false, false, false, true),
            (0.0, Vec3::ZERO, false, false, false, false, false, true),
            (0.0, Vec3::ZERO, true, true, false, false, false, true),
            (0.0, Vec3::ZERO, true, false, true, false, false, true),
            (0.0, Vec3::ZERO, true, false, false, true, false, true),
            (0.0, Vec3::ZERO, true, false, false, false, true, true),
            (0.0, Vec3::ZERO, true, false, false, false, false, false),
        ] {
            avatar.position.z = ground + avatar.scale.z * 0.5 + clearance;
            world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![avatar.clone()],
            });
            world.agent.flying = true;
            world.agent.velocity = velocity;
            world.agent.seated = seated;
            world.teleporting = teleporting;
            world.crossing = crossing;
            world.movement_complete = arrived;
            // A drawn avatar already at floor level cannot land an airborne report.
            world.agent.position.z = ground;
            world.auto_land(&crate::agent::MoveInput {
                down,
                up,
                ..Default::default()
            });
            assert!(
                world.agent.flying,
                "clearance {clearance}, velocity {velocity:?}, input {down}/{up}"
            );
        }
    }

    #[test]
    fn a_region_that_blocks_flying_lands_a_restored_flight() {
        let (mut world, handle) = teleport_fixture();
        let arrive = |world: &mut World| {
            world.apply(NetEvent::AgentMovementComplete {
                handle,
                position: Vec3::new(128.0, 128.0, 40.0),
                look_at: Vec3::X,
            });
        };
        // the region as its handshake describes it, flying allowed
        let mut info = crate::demo::events()
            .into_iter()
            .find_map(|e| match e {
                NetEvent::RegionHandshake(info) => Some((*info).clone()),
                _ => None,
            })
            .expect("demo handshake");
        info.handle = handle;
        world.apply(NetEvent::RegionHandshake(Arc::new(info.clone())));
        // the flight of the last session, restored at login
        world.agent.flying = true;
        arrive(&mut world);
        assert!(world.agent.flying, "flying allowed: the flight goes on");
        let flags = info.region_flags | aurora_net::region_flags::BLOCK_FLY;
        world.apply(NetEvent::RegionFlags {
            handle,
            flags,
            max_tasks: 15_000,
        });
        assert!(!world.agent.flying, "the region now blocks flying: landed");
        world.agent.flying = true;
        arrive(&mut world);
        assert!(!world.agent.flying, "an arrival in a no-fly region lands");
    }

    #[test]
    fn unsolicited_local_teleport_hides_previous_progress_and_records_history() {
        let (mut world, handle) = teleport_fixture();
        let previous = world.agent.position;
        // A preceding remote arrival may still have its screen fading out.
        world.tp_show_progress = true;
        world.apply(NetEvent::TeleportStarted);
        assert!(!world.tp_show_progress);
        world.apply(NetEvent::TeleportLocal { flying: false });
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: Vec3::new(60.0, 70.0, 30.0),
            look_at: Vec3::X,
        });
        assert!(!world.teleporting && !world.tp_show_progress);
        assert_eq!(world.tp_history.previous().expect("previous arrival").position, previous);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
    }

    #[test]
    fn unknown_destination_only_shows_progress_when_another_region_is_confirmed() {
        let (mut world, handle) = teleport_fixture();
        world.begin_teleport(false);
        world.apply(NetEvent::TeleportStarted);
        assert!(!world.tp_show_progress);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
        let destination = handle + (256_u64 << 32);
        world.apply(NetEvent::TeleportFinished {
            handle: destination,
            flying: false,
        });
        assert!(world.teleporting && world.tp_show_progress);
        world.apply(NetEvent::TeleportFinished {
            handle: destination,
            flying: false,
        });
        assert_eq!(world.ui_sounds.iter().filter(|s| **s == UiSound::TeleportOut).count(), 1);
        world.apply(NetEvent::AgentMovementComplete {
            handle: destination,
            position: Vec3::new(60.0, 70.0, 30.0),
            look_at: Vec3::X,
        });
        assert!(!world.teleporting && world.tp_show_progress);
        assert_eq!(world.main_region, Some(destination));
        assert!(world.parcel_name.is_empty());
    }

    #[test]
    fn known_remote_destination_shows_progress_and_failure_ends_the_teleport() {
        let (mut world, handle) = teleport_fixture();
        world.begin_teleport(true);
        world.apply(NetEvent::TeleportStarted);
        assert!(world.teleporting && world.tp_show_progress);
        world.apply(NetEvent::TeleportFinished {
            handle: handle + (256_u64 << 32),
            flying: false,
        });
        assert_eq!(world.ui_sounds.iter().filter(|s| **s == UiSound::TeleportOut).count(), 1);
        world.apply(NetEvent::TeleportFailed {
            reason: aurora_i18n::tr!("world-mod-destination-unavailable").into(),
        });
        assert!(!world.teleporting && world.tp_failed);
        // The screen remains eligible for its failure fade-out.
        assert!(world.tp_show_progress);
        world.ui_sounds.clear();
        world.begin_teleport(false);
        assert!(!world.tp_failed && !world.tp_show_progress);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
    }

    #[test]
    fn finish_in_the_current_region_does_not_show_progress() {
        let (mut world, handle) = teleport_fixture();
        world.begin_teleport(false);
        world.apply(NetEvent::TeleportFinished { handle, flying: false });
        assert!(world.teleporting && !world.tp_show_progress);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
    }

    #[test]
    fn remote_teleport_applies_both_flight_states_and_the_explicit_arrival_preference() {
        for previous in [false, true] {
            for flying in [false, true] {
                for preference in [false, true] {
                    let (mut world, handle) = teleport_fixture();
                    world.agent.flying = previous;
                    world.fly_after_teleport = preference;
                    world.begin_teleport(true);
                    world.apply(NetEvent::TeleportFinished { handle, flying });
                    world.apply(NetEvent::AgentMovementComplete {
                        handle,
                        position: Vec3::new(60.0, 70.0, 30.0),
                        look_at: Vec3::X,
                    });
                    assert_eq!(world.agent.flying, flying || preference);
                    let flags = world.agent.control_flags(&crate::agent::MoveInput::default(), false);
                    assert_eq!(flags & aurora_net::control::FLY != 0, flying || preference);
                }
            }
        }
    }

    #[test]
    fn full_and_terse_updates_keep_server_velocity_and_acceleration() {
        let (mut world, mut update, handle) = fixture();
        update.position = Vec3::new(40.0, 50.0, 1000.0);
        update.velocity = Vec3::new(3.2, 0.0, -1.0);
        update.acceleration = Vec3::new(0.0, 0.0, -9.8);
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![update.clone()],
        });
        assert_eq!(world.agent.velocity, update.velocity);
        assert_eq!(world.agent.predicted_position(), update.position);
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("avatar exists");
        assert_eq!(world.objects.get(idx).expect("avatar exists").acceleration, update.acceleration);
        let terse = TerseUpdate {
            local_id: update.local_id,
            state: 0,
            is_avatar: true,
            foot_plane: None,
            position: update.position + Vec3::X,
            velocity: Vec3::Y * 5.0,
            acceleration: Vec3::Z,
            rotation: glam::Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            texture_entry: None,
        };
        world.apply(NetEvent::TerseUpdates {
            handle,
            updates: vec![terse.clone()],
        });
        assert_eq!(world.agent.velocity, terse.velocity);
        assert_eq!(world.agent.predicted_position(), terse.position);
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: Vec3::splat(100.0),
            look_at: Vec3::X,
        });
        assert_eq!(world.agent.velocity, Vec3::ZERO);
        assert_eq!(world.agent.predicted_position(), Vec3::splat(100.0));
    }

    #[test]
    fn arrival_draws_in_main_region_coordinates_even_with_an_old_avatar_object() {
        let (mut world, update, handle) = fixture();
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![update],
        });
        let neighbor = handle + (256_u64 << 32);
        let arrival = Vec3::new(4.0, 100.0, 30.0);
        world.apply(NetEvent::AgentMovementComplete {
            handle: neighbor,
            position: arrival,
            look_at: Vec3::X,
        });
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("avatar exists");
        let (position, _, _) = crate::scene::Scene::object_transform(&world, idx, Instant::now(), 0).expect("avatar transform");
        assert_eq!(position, arrival);
    }

    #[test]
    fn prediction_clamps_to_terrain_plus_half_avatar_height() {
        let (mut world, mut update, handle) = fixture();
        update.scale.z = 2.0;
        update.position = Vec3::new(40.0, 50.0, 10.0);
        update.velocity = -Vec3::Z;
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![update],
        });
        world.movement_complete = true;
        world.regions.get_mut(&handle).expect("main region").heightmap.heights.fill(20.0);
        world.predict_agent(1.0, 0.0);
        assert_eq!(world.agent.predicted_position().z, 21.0);
    }

    #[test]
    fn prediction_stops_at_unknown_region_but_can_enter_known_neighbor() {
        let (mut world, _, handle) = fixture();
        let start = Vec3::new(255.0, 100.0, 30.0);
        let end = Vec3::new(258.0, 100.0, 30.0);
        let clipped = world.clip_to_visible_regions(start, end);
        assert!(clipped.x < 256.0 && clipped.x > 255.9);
        assert_eq!(clipped.y, end.y);
        assert_eq!(clipped.z, end.z);
        let neighbor = handle + (256_u64 << 32);
        world.ensure_region(neighbor, (256, 256));
        assert_eq!(world.clip_to_visible_regions(start, end), end);
    }

    /// LLHUDEffectResetSkeleton::update: only an avatar resetting itself.
    #[test]
    fn a_skeleton_reset_is_followed_only_for_an_avatar_resetting_itself() {
        let (mut world, _, _) = fixture();
        for ev in crate::demo::events() {
            if matches!(ev, NetEvent::ObjectUpdates { .. }) {
                world.apply(ev);
            }
        }
        let (me, loup, nova) = (world.agent_id, crate::demo::DEMO_LOUP, crate::demo::DEMO_NOVA);
        assert!(world.objects.index_of_uuid(&loup).is_some(), "the demo avatar is there");
        let reset = |source, target| NetEvent::ResetSkeleton {
            source,
            target,
            animations: true,
        };
        world.apply(reset(loup, loup));
        assert_eq!(world.skeleton_resets, vec![(loup, true)]);
        world.skeleton_resets.clear();
        // someone else's skeleton, our own echo, an unknown sender: ignored
        world.apply(reset(loup, nova));
        world.apply(reset(me, me));
        world.apply(reset(Uuid::from_u128(99), Uuid::from_u128(99)));
        assert!(world.skeleton_resets.is_empty());
    }

    #[test]
    fn own_ground_sit_animation_means_sitting() {
        let (mut world, _, _) = fixture();
        let me = world.agent_id;
        let sit = |avatar, anim| NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar,
            anims: vec![(anim, 1)],
        };
        world.apply(sit(crate::demo::DEMO_LOUP, body::ANIM_SIT_GROUND_CONSTRAINED));
        assert!(!world.agent.is_sitting());
        world.apply(sit(me, body::ANIM_SIT_GROUND_CONSTRAINED));
        assert!(world.agent.is_sitting() && !world.agent.seated);
        world.apply(sit(me, crate::demo::IDLE_ANIM));
        assert!(!world.agent.is_sitting());
    }

    /// IM_TYPING_START / STOP from a resident: "is typing" in their
    /// conversation, neither a line nor a conversation opened.
    #[test]
    fn typing_notices_mark_the_resident_typing_without_a_line() {
        let (mut world, _, _) = fixture();
        let from = Uuid::from_u128(77);
        let notice = |world: &World, dialog| {
            NetEvent::InstantMessage(aurora_net::InstantMessage {
                from_agent_id: from,
                from_name: "Typer Resident".into(),
                to_agent_id: world.agent_id,
                dialog,
                session_id: Uuid::from_u128(world.agent_id.as_u128() ^ from.as_u128()),
                message: "typing".into(),
                offline: false,
                from_group: false,
                binary_bucket: vec![0],
                timestamp: 0,
            })
        };
        world.apply(notice(&world, IM_TYPING_START));
        assert!(world.social.is_typing(&from));
        assert!(!world.social.has_session(&from), "no conversation opened");
        assert!(world.im_messages.is_empty(), "no message");
        world.apply(notice(&world, IM_TYPING_STOP));
        assert!(!world.social.is_typing(&from));
    }

    fn inventory_offer(world: &World, dialog: u8, bucket: Vec<u8>) -> NetEvent {
        NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: Uuid::from_u128(77),
            from_name: "Giver Resident".into(),
            to_agent_id: world.agent_id,
            dialog,
            session_id: Uuid::new_v4(),
            message: "Alpha".into(),
            offline: false,
            from_group: false,
            binary_bucket: bucket,
            timestamp: 0,
        })
    }

    fn agent_bucket(asset_type: u8, object: Uuid) -> Vec<u8> {
        let mut bucket = vec![asset_type];
        bucket.extend_from_slice(object.as_bytes());
        bucket
    }

    #[test]
    fn resident_offer_is_fetched_on_arrival_and_trashed_when_refused() {
        use aurora_net::NetCommand;
        use aurora_net::inventory::operations::Operation;
        use notifications::{Response, im};
        let (mut world, _, _) = fixture();
        let me = world.agent_id;
        inventory::demo::seed(&mut world.inventory, me);
        let trash = inventory::actions::system(&world.inventory, 14).expect("Trash");
        let item = Uuid::from_u128(0xFEED);

        world.apply(inventory_offer(&world, im::INVENTORY_OFFERED, agent_bucket(5, item)));
        assert!(
            world
                .take_social_commands()
                .iter()
                .any(|c| matches!(c, NetCommand::FetchItems { items, owner } if items[..] == [item] && *owner == me))
        );
        let id = world.notifications.list.last().expect("offer").id;
        let (cmds, _) = world.respond_notification(id, Response::Accept);
        assert!(matches!(
            &cmds[..],
            [NetCommand::SendImDialog {
                dialog: im::INVENTORY_ACCEPTED,
                ..
            }]
        ));
        assert!(world.take_inventory_edits().is_empty());

        // refused before the fetch answered: the move does not need the item
        world.apply(inventory_offer(&world, im::INVENTORY_OFFERED, agent_bucket(5, item)));
        let id = world.notifications.list.last().expect("offer").id;
        let (cmds, _) = world.respond_notification(id, Response::Decline);
        assert!(matches!(
            &cmds[..],
            [NetCommand::SendImDialog {
                dialog: im::INVENTORY_DECLINED,
                ..
            }]
        ));
        let edits = world.take_inventory_edits();
        let [edit] = &edits[..] else {
            panic!("one move to the Trash, got {}", edits.len());
        };
        assert!(matches!(&edit.operations[..],
            [Operation::Patch { id, folder: false, body }] if *id == item && body["parent_id"].as_uuid() == trash));
        assert!(edit.refresh.contains(&trash));
    }

    #[test]
    fn folder_offer_reloads_the_root_and_object_offers_wait_for_the_server() {
        use aurora_net::NetCommand;
        use notifications::{Response, im};
        let (mut world, _, _) = fixture();
        let me = world.agent_id;
        inventory::demo::seed(&mut world.inventory, me);
        let root = world.inventory.root;
        let folder = Uuid::from_u128(0xF01D);

        world.apply(inventory_offer(
            &world,
            im::INVENTORY_OFFERED,
            agent_bucket(notifications::AT_CATEGORY as u8, folder),
        ));
        assert!(
            world
                .take_social_commands()
                .iter()
                .any(|c| matches!(c, NetCommand::FetchInventory { folders, library: false, .. } if folders[..] == [root]))
        );
        // closing the notification keeps the folder: accepted, nothing moved
        let id = world.notifications.list.last().expect("offer").id;
        let (cmds, _) = world.respond_notification(id, Response::Dismiss);
        assert!(
            matches!(&cmds[..], [NetCommand::SendImDialog { dialog: im::INVENTORY_ACCEPTED, bucket, .. }] if bucket[..] == root.as_bytes()[..])
        );
        assert!(world.take_inventory_edits().is_empty());
        world.apply(inventory_offer(
            &world,
            im::INVENTORY_OFFERED,
            agent_bucket(notifications::AT_CATEGORY as u8, folder),
        ));
        let id = world.notifications.list.last().expect("offer").id;
        world.respond_notification(id, Response::Decline);
        assert!(matches!(&world.take_inventory_edits()[..],
            [edit] if matches!(&edit.operations[..], [aurora_net::inventory::operations::Operation::Patch { id, folder: true, .. }] if *id == folder)));

        // an object's offer only names the type: nothing exists before it is accepted
        world.take_social_commands();
        world.apply(inventory_offer(&world, im::TASK_INVENTORY_OFFERED, vec![6]));
        assert!(
            !world
                .take_social_commands()
                .iter()
                .any(|c| matches!(c, NetCommand::FetchItems { .. } | NetCommand::FetchInventory { .. }))
        );
        let id = world.notifications.list.last().expect("offer").id;
        assert!(world.notifications.list.last().is_some_and(|n| n.close_declines()));
        let (cmds, _) = world.respond_notification(id, Response::Dismiss);
        assert!(matches!(
            &cmds[..],
            [NetCommand::SendImDialog {
                dialog: im::TASK_INVENTORY_DECLINED,
                ..
            }]
        ));
        assert!(world.take_inventory_edits().is_empty());
    }

    /// An experience's question: from region 77, for teleport.
    fn experience_question(experience: Uuid, questions: i32) -> NetEvent {
        NetEvent::ScriptQuestion {
            task_id: Uuid::from_u128(0x7A5C),
            item_id: Uuid::from_u128(0x17E),
            object_name: "Club door".into(),
            owner_name: "Door Maker Resident".into(),
            questions,
            experience,
            region: 77,
        }
    }

    #[test]
    fn an_experience_question_waits_for_its_details_and_is_answered_to_its_region() {
        let (mut world, _, _) = fixture();
        let xp = Uuid::from_u128(0xE0E0);
        let count = world.notifications.list.len();
        world.apply(experience_question(xp, 1 << 12));
        // nothing shown before the details: LLExperienceCache::get first
        assert_eq!(world.notifications.list.len(), count);
        assert!(matches!(&world.experiences.take_commands()[..], [aurora_net::NetCommand::ExperienceInfo(ids)] if ids == &vec![xp]));
        world.apply(NetEvent::ExperienceInfo(vec![aurora_net::experience::ExperienceDetails {
            id: xp,
            name: "Club Teleport".into(),
            properties: aurora_net::experience::PROPERTY_GRID,
            ..Default::default()
        }]));
        let n = world.notifications.list.last().expect("question shown");
        assert!(n.body.contains("Club Teleport") && n.body.contains("Club door"));
        assert!(n.body.contains("Door Maker") && !n.body.contains("Resident"));
        assert!(n.body.contains(aurora_i18n::tr!("world-experiences-grid-scope")));
        assert!(matches!(&n.data, notifications::Data::Permissions { experience: Some(p), .. } if p.id == xp));
        let id = n.id;
        // the profile link keeps the question
        assert!(
            world
                .respond_notification(id, notifications::Response::OpenExperience(xp))
                .0
                .is_empty()
        );
        assert_eq!(world.experiences.profile, Some(xp));
        assert!(world.notifications.list.iter().any(|n| n.id == id));
        let (cmds, _) = world.respond_notification(id, notifications::Response::Accept);
        assert!(matches!(
            &cmds[..],
            [aurora_net::NetCommand::ScriptAnswer {
                questions: 4096,
                region: 77,
                ..
            }]
        ));
        assert_eq!(
            world.experiences.permissions.get(&xp),
            Some(&aurora_net::experience::ExperiencePermission::Allowed)
        );
    }

    #[test]
    fn blocking_an_experience_or_its_object_refuses_the_question() {
        let (mut world, _, _) = fixture();
        let xp = Uuid::from_u128(0xE1E1);
        world.apply(NetEvent::ExperienceInfo(vec![aurora_net::experience::ExperienceDetails::unknown(
            xp,
        )]));
        world.experiences.take_commands();
        world.apply(experience_question(xp, 1 << 12));
        let n = world.notifications.list.last().expect("question");
        // unknown details: still asked, land-scope, named by its id
        assert!(n.body.contains(&xp.to_string()) && n.body.contains(aurora_i18n::tr!("world-experiences-land-scope")));
        let (cmds, _) = world.respond_notification(n.id, notifications::Response::BlockExperience);
        assert!(matches!(
            &cmds[..],
            [aurora_net::NetCommand::ScriptAnswer {
                questions: 0,
                region: 77,
                ..
            }]
        ));
        assert!(world.experiences.take_commands().iter().any(|c| matches!(
            c,
            aurora_net::NetCommand::ExperiencePermission {
                experience,
                change: aurora_net::experience::ExperiencePermissionChange::Block
            } if *experience == xp
        )));
        // Block object: refused, the object blocked, its next question dropped
        world.apply(experience_question(xp, 1 << 12));
        let id = world.notifications.list.last().unwrap().id;
        let (cmds, _) = world.respond_notification(id, notifications::Response::Block);
        assert!(matches!(&cmds[..], [aurora_net::NetCommand::ScriptAnswer { questions: 0, .. }]));
        assert!(world.mutes.is_muted_id(&Uuid::from_u128(0x7A5C)));
        let count = world.notifications.list.len();
        world.apply(experience_question(xp, 1 << 12));
        assert_eq!(world.notifications.list.len(), count);
    }

    #[test]
    fn a_question_shows_without_a_click_from_an_unknown_object_of_another_region() {
        let (mut world, _, _) = fixture();
        let xp = Uuid::from_u128(0xE3E3);
        // nothing selected, the object never seen, a neighbouring region:
        // a collision, a sensor or the parcel's arrival script asks
        world.apply(NetEvent::ScriptQuestion {
            task_id: Uuid::from_u128(0x5EE),
            item_id: Uuid::from_u128(0x5EF),
            object_name: "Greeter".into(),
            owner_name: "Land Owner".into(),
            questions: 1 << 18,
            experience: xp,
            region: 0xABCD,
        });
        world.apply(NetEvent::ExperienceInfo(vec![aurora_net::experience::ExperienceDetails::unknown(
            xp,
        )]));
        let n = world.notifications.list.last().expect("shown");
        assert!(matches!(
            &n.data,
            notifications::Data::Permissions {
                region: 0xABCD,
                experience: Some(_),
                ..
            }
        ));
        let (cmds, _) = world.respond_notification(n.id, notifications::Response::Accept);
        assert!(matches!(&cmds[..], [aurora_net::NetCommand::ScriptAnswer { region: 0xABCD, .. }]));
    }

    #[test]
    fn a_caution_permission_keeps_the_ordinary_question_and_a_plain_one_has_no_experience() {
        let (mut world, _, _) = fixture();
        world.apply(experience_question(Uuid::from_u128(0xE2E2), 1 << 1 | 1 << 12));
        assert!(matches!(
            world.notifications.list.last().map(|n| &n.data),
            Some(notifications::Data::Permissions {
                experience: None,
                region: 77,
                ..
            })
        ));
        assert!(world.experiences.take_commands().is_empty());
        world.apply(experience_question(Uuid::nil(), 1 << 4));
        assert!(matches!(
            world.notifications.list.last().map(|n| &n.data),
            Some(notifications::Data::Permissions {
                experience: None,
                questions: 16,
                ..
            })
        ));
    }

    #[test]
    fn script_dialog_replies_keep_the_sender_without_resolving_the_hud_in_the_scene() {
        let (mut world, _, here) = fixture();
        let sender = ([127, 0, 0, 1], 13001).into();
        let object_id = Uuid::from_u128(0x4321);
        assert!(world.objects.index_of_uuid(&object_id).is_none());
        let labels = vec!["  Keep spaces  ".to_owned(), "✓ Sélection".to_owned(), " ".to_owned()];
        for channel in [i32::MIN, -42, 0, 42, i32::MAX] {
            for index in 0..labels.len() {
                world.apply(NetEvent::ScriptDialog {
                    sender,
                    object_id,
                    object_name: "HUD".into(),
                    owner_name: "Example Resident".into(),
                    message: "Choose".into(),
                    channel,
                    buttons: labels.clone(),
                });
                // The menu can outlive a region crossing. Object lookup,
                // parent/root transforms and HUD rendering never route it.
                world.main_region = Some(here + (256u64 << 32));
                let id = world.notifications.list.last().expect("dialog").id;
                let (commands, url) = world.respond_notification(id, notifications::Response::Button(index));
                assert!(url.is_none());
                assert!(matches!(commands.as_slice(), [aurora_net::NetCommand::ScriptDialogReply {
                    sender: Some(to), object_id: object, channel: ch, index: button, label,
                }] if *to == sender && *object == object_id && *ch == channel && *button == index as i32 && label == &labels[index]));
            }
        }
        world.apply(NetEvent::ScriptDialog {
            sender,
            object_id,
            object_name: "HUD".into(),
            owner_name: "Example Resident".into(),
            message: "Enter text".into(),
            channel: -42,
            buttons: vec!["!!llTextBox!!".into()],
        });
        let n = world.notifications.list.last_mut().expect("text box");
        n.text = "  ✓ Original text  ".into();
        let id = n.id;
        let (commands, _) = world.respond_notification(id, notifications::Response::SendText);
        assert!(matches!(commands.as_slice(), [aurora_net::NetCommand::ScriptDialogReply {
            sender: Some(to), object_id: object, channel: -42, index: 0, label,
        }] if *to == sender && *object == object_id && label == "  ✓ Original text  "));
    }

    #[test]
    fn script_menu_ignore_sends_no_reply_and_block_suppresses_future_menus() {
        let (mut world, _, _) = fixture();
        let object_id = Uuid::from_u128(0x4321);
        let menu = || NetEvent::ScriptDialog {
            sender: ([127, 0, 0, 1], 13000).into(),
            object_id,
            object_name: aurora_i18n::tr!("world-mod-test-menu").into(),
            owner_name: aurora_i18n::tr!("world-mod-test-seller").into(),
            message: aurora_i18n::tr!("world-mod-choose").into(),
            channel: -42,
            buttons: vec![aurora_i18n::tr!("common-yes").into(), aurora_i18n::tr!("common-no").into()],
        };
        world.apply(menu());
        let id = world.notifications.list.last().unwrap().id;
        assert!(world.respond_notification(id, notifications::Response::Dismiss).0.is_empty());
        assert!(world.notifications.list.iter().all(|n| n.id != id));
        assert!(!world.mutes.is_muted_id(&object_id));
        world.apply(menu());
        let id = world.notifications.list.last().unwrap().id;
        assert!(world.respond_notification(id, notifications::Response::Block).0.is_empty());
        assert!(world.mutes.is_muted_id(&object_id));
        assert!(world.notifications.list.iter().all(|n| n.id != id));
        let count = world.notifications.list.len();
        world.apply(menu());
        assert_eq!(world.notifications.list.len(), count);
    }

    /// Our avatar wearing a prim in the demo region, and a second region.
    fn worn_fixture() -> (
        World,
        aurora_net::objects::ObjectUpdate,
        aurora_net::objects::ObjectUpdate,
        RegionHandle,
        RegionHandle,
    ) {
        let (mut world, avatar, here) = fixture();
        let mut worn = avatar.clone();
        worn.local_id = avatar.local_id + 1;
        worn.full_id = Uuid::from_u128(0xBEEF);
        worn.parent_id = avatar.local_id;
        worn.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        worn.state = 0x20;
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![avatar.clone(), worn.clone()],
        });
        let there = here + (256u64 << 32);
        world.ensure_region(there, (256, 256));
        (world, avatar, worn, here, there)
    }

    /// Another viewer's typing notices make the avatar type, without its
    /// typing animation (PlayTypingAnim off), until its stop, its line or
    /// its leaving (process_chat_from_simulator); ours echoed back and an
    /// avatar not built yet are skipped.
    #[test]
    fn typing_notices_make_an_avatar_type_without_its_animation() {
        let (mut world, avatar, here) = fixture();
        let mut other = avatar.clone();
        other.local_id = 950;
        other.full_id = Uuid::from_u128(0xD00D);
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![avatar.clone(), other.clone()],
        });
        let notice = |avatar, typing| NetEvent::NearbyTyping { avatar, typing };
        let id = other.full_id;
        world.apply(notice(Uuid::from_u128(0xFEED), true));
        assert!(!world.is_typing_nearby(&Uuid::from_u128(0xFEED)), "not built yet");
        world.apply(notice(world.agent_id, true));
        assert!(!world.is_typing_nearby(&world.agent_id), "our own echo");
        world.apply(notice(id, true));
        assert!(world.is_typing_nearby(&id));
        world.apply(notice(id, false));
        assert!(!world.is_typing_nearby(&id));
        // the line said ends it
        world.apply(notice(id, true));
        world.apply(NetEvent::Chat(ChatMessage {
            from_name: "Other".into(),
            source_id: id,
            owner_id: id,
            source_type: ChatSourceType::Agent,
            chat_type: ChatType::Normal,
            position: Vec3::new(10.0, 10.0, 20.0),
            message: "hello".into(),
        }));
        assert!(!world.is_typing_nearby(&id));
        // gone with the avatar, not back with it
        world.apply(notice(id, true));
        world.apply(NetEvent::ObjectsKilled {
            handle: here,
            local_ids: vec![other.local_id],
        });
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![other],
        });
        assert!(!world.is_typing_nearby(&id));
    }

    #[test]
    fn our_avatar_and_what_it_wears_survive_a_teleport_to_another_region() {
        let (mut world, avatar, worn, here, there) = worn_fixture();
        let me = world.agent_id;
        let own = world.objects.index_of_uuid(&me).expect("avatar");
        let worn_idx = world.objects.index_of_uuid(&worn.full_id).expect("worn");
        world.apply(NetEvent::MainRegionChanged { handle: there });
        // the region left asks to remove both, then goes away
        world.apply(NetEvent::ObjectsKilled {
            handle: here,
            local_ids: vec![worn.local_id, avatar.local_id],
        });
        assert_eq!(world.objects.index_of_uuid(&me), Some(own));
        assert_eq!(world.objects.index_of_uuid(&worn.full_id), Some(worn_idx));
        world.apply(NetEvent::RegionRemoved { handle: here });
        assert_eq!(world.objects.index_of_uuid(&me), Some(own));
        assert_eq!(world.objects.index_of_uuid(&worn.full_id), Some(worn_idx));
        let key = world.objects.get(own).expect("avatar").key;
        assert_eq!((key.region, key.local_id), (there, objects::CARRIED_LOCAL_ID));
        // the destination sends them under its own ids: the same objects
        let mut avatar2 = avatar.clone();
        avatar2.local_id = 7001;
        let mut worn2 = worn.clone();
        worn2.local_id = 7002;
        worn2.parent_id = 7001;
        world.apply(NetEvent::ObjectUpdates {
            handle: there,
            objects: vec![avatar2, worn2],
        });
        assert_eq!(world.objects.index_of_uuid(&me), Some(own));
        assert_eq!(world.objects.index_of_uuid(&worn.full_id), Some(worn_idx));
        let w = world.objects.get(worn_idx).expect("worn");
        assert_eq!(w.key.region, there);
        assert_eq!(world.objects.parent_of(w), Some(own));
        // nothing is left behind to remove
        let t = Instant::now();
        world.flush_arrival(t);
        world.flush_arrival(t + std::time::Duration::from_secs(60));
        assert_eq!(world.objects.index_of_uuid(&worn.full_id), Some(worn_idx));
    }

    #[test]
    fn worn_object_the_destination_never_sends_is_removed() {
        let (mut world, avatar, worn, here, there) = worn_fixture();
        world.apply(NetEvent::MainRegionChanged { handle: there });
        world.apply(NetEvent::RegionRemoved { handle: here });
        let t = Instant::now();
        // not counted while our avatar itself has not come
        world.flush_arrival(t);
        world.flush_arrival(t + std::time::Duration::from_secs(30));
        assert!(world.objects.index_of_uuid(&worn.full_id).is_some());
        let mut avatar2 = avatar.clone();
        avatar2.local_id = 7001;
        world.apply(NetEvent::ObjectUpdates {
            handle: there,
            objects: vec![avatar2],
        });
        world.flush_arrival(t + std::time::Duration::from_secs(31));
        world.flush_arrival(t + std::time::Duration::from_secs(44));
        assert!(world.objects.index_of_uuid(&worn.full_id).is_some());
        world.flush_arrival(t + std::time::Duration::from_secs(46));
        assert!(world.objects.index_of_uuid(&worn.full_id).is_none());
        assert!(world.objects.index_of_uuid(&world.agent_id).is_some());
    }

    #[test]
    fn blender_live_preview_survives_arrival_cleanup_while_stale_grid_attachments_are_removed() {
        let (mut world, avatar, worn, here, there) = worn_fixture();
        let parent = world
            .objects
            .get(world.objects.index_of_uuid(&world.agent_id).expect("our avatar"))
            .expect("avatar")
            .key;
        let mut preview = prim(&worn, 42, parent.local_id);
        preview.state = 1u8.rotate_left(4);
        let id = preview.full_id;
        world.objects.insert_preview_attachment(parent, preview);
        let now = Instant::now();
        world.flush_arrival(now);
        world.flush_arrival(now + std::time::Duration::from_secs(30));
        assert!(
            world.objects.index_of_uuid(&id).is_some(),
            "a preview is not a missing simulator attachment"
        );
        world.apply(NetEvent::MainRegionChanged { handle: there });
        world.apply(NetEvent::RegionRemoved { handle: here });
        let mut destination = avatar;
        destination.local_id = 7001;
        world.apply(NetEvent::ObjectUpdates {
            handle: there,
            objects: vec![destination],
        });
        world.flush_arrival(now + std::time::Duration::from_secs(31));
        world.flush_arrival(now + std::time::Duration::from_secs(47));
        assert!(
            world.objects.index_of_uuid(&worn.full_id).is_none(),
            "normal stale attachment cleanup remains active"
        );
        world.flush_arrival(now + std::time::Duration::from_secs(120));
        let preview = world
            .objects
            .get(world.objects.index_of_uuid(&id).expect("preview retained"))
            .expect("preview");
        assert_eq!(preview.parent_region, there);
        assert!(preview.is_client_preview());
    }

    /// A region a teleport away from `here`.
    fn far_from(here: RegionHandle) -> RegionHandle {
        here + ((100 * 256u64) << 32)
    }

    /// A teleport to `handle`, from TeleportFinish to the arrival.
    fn teleport_to(world: &mut World, handle: RegionHandle) {
        world.apply(NetEvent::TeleportFinished { handle, flying: false });
        world.apply(NetEvent::MainRegionChanged { handle });
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: Vec3::new(128.0, 128.0, 30.0),
            look_at: Vec3::X,
        });
    }

    /// A prim of `template`'s kind, alone or linked under `parent`.
    fn prim(template: &aurora_net::objects::ObjectUpdate, local_id: u32, parent: u32) -> aurora_net::objects::ObjectUpdate {
        let mut p = template.clone();
        p.local_id = local_id;
        p.full_id = Uuid::from_u128(0xCAFE_0000 + u128::from(local_id));
        p.parent_id = parent;
        p.state = 0;
        p
    }

    #[test]
    fn objects_of_a_region_out_of_reach_are_parked_and_come_back_as_they_were() {
        use aurora_net::objects::ParticleUpdate;
        let (mut world, _, worn, here, _) = worn_fixture();
        let mut root = prim(&worn, 900, 0);
        root.particles = ParticleUpdate::Set(vec![1, 2, 3]);
        let kids = [prim(&worn, 901, 900), prim(&worn, 902, 900), prim(&worn, 903, 900)];
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: std::iter::once(root.clone()).chain(kids.iter().cloned()).collect(),
        });
        // the scene has seen them all
        let root_idx = world.objects.index_of_uuid(&root.full_id).expect("root");
        world.objects.get_mut_untracked(root_idx).expect("root").particle_update = None;
        world.objects.settle(root_idx);
        world.objects.take_touched(&mut Vec::new());
        let me = world.agent_id;
        let own = world.objects.index_of_uuid(&me).expect("avatar");
        let count = world.objects.len();

        let far = far_from(here);
        teleport_to(&mut world, far);
        // nothing of the region left is in the store; our avatar and what
        // it wears came along; the region is still known
        for u in std::iter::once(&root).chain(&kids) {
            assert!(world.objects.index_of_uuid(&u.full_id).is_none());
        }
        assert_eq!(world.objects.len(), count - 4);
        assert_eq!(world.objects.index_of_uuid(&me), Some(own));
        assert!(world.objects.index_of_uuid(&worn.full_id).is_some());
        assert_eq!(world.objects.get(own).expect("avatar").key.region, far);
        assert!(world.regions.contains_key(&here));
        assert!(world.parked.contains_key(&here));

        // what its simulator still sends waits: the root moves, a child goes
        let mut moved = root.clone();
        moved.particles = ParticleUpdate::Keep;
        moved.position = Vec3::new(10.0, 20.0, 30.0);
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![moved],
        });
        world.apply(NetEvent::ObjectsKilled {
            handle: here,
            local_ids: vec![902],
        });
        assert!(world.objects.index_of_uuid(&root.full_id).is_none());

        // back before the simulator closed the region: it sends nothing
        // again, the objects come from here
        world.objects.take_touched(&mut Vec::new());
        teleport_to(&mut world, here);
        assert!(!world.parked.contains_key(&here));
        let root_idx = world.objects.index_of_uuid(&root.full_id).expect("root is back");
        let r = world.objects.get(root_idx).expect("root");
        assert_eq!(r.key.local_id, 900);
        assert_eq!(r.position, Vec3::new(10.0, 20.0, 30.0));
        // new for the scene, with the particle system it was sent once
        assert!(world.objects.is_arrival(root_idx));
        assert!(world.objects.touched().contains(&root_idx));
        assert_eq!(r.particle_update, Some(ParticleUpdate::Set(vec![1, 2, 3])));
        assert!(r.shape_dirty && r.material_dirty && r.sound_update);
        // its children hang from it again, in their order, without the one removed
        let linked: Vec<Uuid> = world
            .objects
            .children_of(&r.key)
            .iter()
            .filter_map(|&k| world.objects.get(k).map(|o| o.full_id))
            .collect();
        assert_eq!(linked, [kids[0].full_id, kids[2].full_id]);
        assert!(world.objects.index_of_uuid(&kids[1].full_id).is_none());
        assert_eq!(world.objects.index_of_uuid(&me), Some(own));
        assert_eq!(world.objects.len(), count - 1);
        // the region just left is the one parked now
        assert!(world.parked.contains_key(&far));
    }

    #[test]
    fn a_neighbour_keeps_its_objects_when_the_border_is_crossed() {
        let (mut world, _, worn, here, there) = worn_fixture();
        let p = prim(&worn, 900, 0);
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![p.clone()],
        });
        world.apply(NetEvent::MainRegionChanged { handle: there });
        world.apply(NetEvent::AgentMovementComplete {
            handle: there,
            position: Vec3::new(1.0, 128.0, 30.0),
            look_at: Vec3::X,
        });
        assert!(world.objects.index_of_uuid(&p.full_id).is_some());
        assert!(world.parked.is_empty());
        // and a teleport next door, or two regions further, parks nothing
        teleport_to(&mut world, there + (2 * (256u64 << 32)));
        assert!(world.objects.index_of_uuid(&p.full_id).is_some());
        assert!(world.parked.is_empty());
    }

    #[test]
    fn a_parked_region_goes_for_good_when_its_simulator_closes_it() {
        let (mut world, _, worn, here, _) = worn_fixture();
        let p = prim(&worn, 900, 0);
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![p.clone()],
        });
        teleport_to(&mut world, far_from(here));
        assert!(world.parked.contains_key(&here));
        world.apply(NetEvent::RegionRemoved { handle: here });
        assert!(!world.parked.contains_key(&here));
        assert!(!world.regions.contains_key(&here));
        // a later visit starts from what the simulator sends
        teleport_to(&mut world, here);
        assert!(world.objects.index_of_uuid(&p.full_id).is_none());
        assert!(world.objects.index_of_uuid(&world.agent_id).is_some());
    }

    #[test]
    fn a_teleport_elsewhere_forgets_the_coarse_locations_of_the_regions_left() {
        let (mut world, _, _, here, there) = worn_fixture();
        let friend = Uuid::from_u128(0xF00D);
        let stranger = Uuid::from_u128(0xF00E);
        let seen = |world: &mut World, handle, id| {
            world.apply(NetEvent::CoarseLocations {
                handle,
                positions: vec![(id, Vec3::new(40.0, 50.0, 20.0))],
            });
        };
        world.arrived_once = true;
        seen(&mut world, here, friend);
        seen(&mut world, there, stranger);
        // walking into the neighbour: both simulators still send their lists
        let position = world.agent.position - Vec3::new(256.0, 0.0, 0.0);
        world.apply(NetEvent::AgentMovementComplete {
            handle: there,
            position,
            look_at: Vec3::X,
        });
        assert!(world.arrived_by_crossing);
        assert!(world.avatar_map_position(friend).is_some());
        assert!(world.avatar_map_position(stranger).is_some());

        // a teleport far away: the list of the arrival region stays, the
        // friend left behind is no longer around (issue #1000)
        let far = far_from(there);
        world.begin_teleport(false);
        seen(&mut world, far, stranger);
        teleport_to(&mut world, far);
        assert_eq!(world.coarse.keys().copied().collect::<Vec<_>>(), vec![far]);
        assert!(world.avatar_map_position(friend).is_none());
        assert!(!world.can_locate_avatar(friend));
        assert!(world.avatar_map_position(stranger).is_some());
        // the simulator left still sends its list while we are its child:
        // it waits with the region's objects
        assert!(world.parked.contains_key(&there));
        seen(&mut world, there, friend);
        assert!(world.avatar_map_position(friend).is_none());
        assert!(!world.can_locate_avatar(friend));
        // back there before it closed, its last list is the current one
        world.begin_teleport(false);
        teleport_to(&mut world, there);
        assert!(!world.parked.contains_key(&there));
        assert_eq!(world.coarse.keys().copied().collect::<Vec<_>>(), vec![there]);
        assert!(world.avatar_map_position(friend).is_some());
    }

    #[test]
    fn an_avatar_met_elsewhere_in_the_meantime_is_not_restored_over_itself() {
        let (mut world, avatar, worn, here, _) = worn_fixture();
        let mut other = avatar.clone();
        other.local_id = 950;
        other.full_id = Uuid::from_u128(0xD00D);
        let mut hat = prim(&worn, 951, 950);
        hat.state = worn.state;
        let stays = prim(&worn, 900, 0);
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![other.clone(), hat.clone(), stays.clone()],
        });
        let far = far_from(here);
        teleport_to(&mut world, far);
        // the other avatar followed us: the far region sends it
        let mut followed = other.clone();
        followed.local_id = 7100;
        world.apply(NetEvent::ObjectUpdates {
            handle: far,
            objects: vec![followed],
        });
        let live = world.objects.index_of_uuid(&other.full_id).expect("avatar in the far region");
        // back: the newer one stays where it is, its parked self and what
        // hung from it are dropped
        world.apply(NetEvent::TeleportFinished {
            handle: here,
            flying: false,
        });
        world.apply(NetEvent::MainRegionChanged { handle: here });
        assert_eq!(world.objects.index_of_uuid(&other.full_id), Some(live));
        assert_eq!(world.objects.get(live).expect("avatar").key.region, far);
        assert!(world.objects.index_of_uuid(&hat.full_id).is_none());
        assert!(world.objects.index_of_uuid(&stays.full_id).is_some());
        // once arrived, it is parked with the far region it is in
        world.apply(NetEvent::AgentMovementComplete {
            handle: here,
            position: Vec3::new(128.0, 128.0, 30.0),
            look_at: Vec3::X,
        });
        assert!(world.objects.index_of_uuid(&other.full_id).is_none());
        assert!(world.objects.index_of_uuid(&stays.full_id).is_some());
    }

    #[test]
    fn the_agent_region_still_detaches_what_we_wear() {
        let (mut world, avatar, worn, here, _) = worn_fixture();
        // our avatar is never removed, even by its own region
        world.apply(NetEvent::ObjectsKilled {
            handle: here,
            local_ids: vec![avatar.local_id, worn.local_id],
        });
        assert!(world.objects.index_of_uuid(&world.agent_id).is_some());
        assert!(world.objects.index_of_uuid(&worn.full_id).is_none());
    }

    fn scripted_detach_fixture(point: u8) -> (World, aurora_net::objects::ObjectUpdate, RegionHandle) {
        let (mut world, _, mut worn, here, _) = worn_fixture();
        appearance::seed_demo(&mut world.inventory, world.agent_id);
        worn.state = point.rotate_left(4);
        worn.name_values = format!("AttachItemID STRING RW SV {}", Uuid::from_u128(717));
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![worn.clone()],
        });
        (world, worn, here)
    }

    #[test]
    fn a_scripted_detach_is_not_undone_by_the_next_inventory_attach_or_detach() {
        use aurora_net::{NetCommand, inventory::operations::Operation};
        for point in [2, 35] {
            for killed in [true, false] {
                for attach in [true, false] {
                    let (mut world, mut worn, handle) = scripted_detach_fixture(point);
                    let item = Uuid::from_u128(717);
                    let cof = appearance::cof(&world.inventory).expect("COF");
                    let saved = Uuid::from_u128(704);
                    let saved_links = appearance::folder_links(&world.inventory, saved);
                    let before = appearance::folder_links(&world.inventory, cof);
                    let link_ids: HashSet<_> = world.inventory.folders[&cof]
                        .items
                        .iter()
                        .copied()
                        .filter(|id| world.inventory.items[id].asset_id == item)
                        .collect();
                    if killed {
                        world.apply(NetEvent::ObjectsKilled {
                            handle,
                            local_ids: vec![worn.local_id],
                        });
                    } else {
                        worn.parent_id = 0;
                        world.apply(NetEvent::ObjectUpdates {
                            handle,
                            objects: vec![worn.clone()],
                        });
                    }
                    assert!(!world.worn_attachment_items().contains(&item));
                    let mut expected = before;
                    expected.retain(|link| link.target != item);
                    assert_eq!(appearance::folder_links(&world.inventory, cof), expected);
                    assert_eq!(appearance::folder_links(&world.inventory, saved), saved_links);
                    assert!(world.inventory.items.contains_key(&item), "keep the inventory original");
                    let edits = world.take_inventory_edits();
                    assert_eq!(edits.len(), 1, "persist the detach without a user inventory action");
                    assert_eq!(edits[0].refresh, [cof]);
                    assert_eq!(edits[0].removed.iter().copied().collect::<HashSet<_>>(), link_ids);
                    assert!(
                        edits[0]
                            .operations
                            .iter()
                            .all(|op| matches!(op, Operation::Delete { id, folder: false } if link_ids.contains(id)))
                    );
                    // The unrelated action can occur before AIS answers the unlink.
                    let action = if attach {
                        appearance::Action::Add(Uuid::from_u128(723))
                    } else {
                        appearance::Action::Remove(Uuid::from_u128(718))
                    };
                    let (change, sync) =
                        appearance::plan(&world.inventory, action, &world.worn_attachment_points()).expect("inventory action");
                    assert!(sync);
                    appearance::demo_mutate(&mut world.inventory, world.agent_id, &change).expect("COF reply");
                    assert!(
                        !appearance::sync_commands(&world.inventory, world.agent_id, &world.worn_attachment_items())
                            .iter()
                            .any(|cmd| matches!(cmd, NetCommand::RezAttachments(items) if items.iter().any(|a| a.item_id == item)))
                    );
                    // Repeated packets must not enqueue another deletion.
                    world.apply(NetEvent::ObjectsKilled {
                        handle,
                        local_ids: vec![worn.local_id],
                    });
                    assert!(world.take_inventory_edits().is_empty());
                    let (change, _) = appearance::plan(&world.inventory, appearance::Action::Add(item), &world.worn_attachment_points())
                        .expect("explicitly wearing the detached original still works");
                    assert!(change.links.iter().any(|link| link.target == item));
                }
            }
        }
    }

    #[test]
    fn detaching_an_attachment_stops_its_root_and_linked_animations_once() {
        for point in [2, 35] {
            for temporary in [false, true] {
                for killed in [false, true] {
                    let (mut world, mut worn, handle) = scripted_detach_fixture(point);
                    if temporary {
                        worn.name_values = format!("AttachItemID STRING RW SV {}", Uuid::nil());
                    }
                    let child = prim(&worn, 950, worn.local_id);
                    let other = prim(&worn, 951, worn.parent_id);
                    world.apply(NetEvent::ObjectUpdates {
                        handle,
                        objects: vec![worn.clone(), child.clone(), other.clone()],
                    });
                    let (root_motion, child_motion, ao) = (Uuid::from_u128(71), Uuid::from_u128(72), Uuid::from_u128(73));
                    let anims = vec![(root_motion, 1), (child_motion, 1), (ao, 1)];
                    world.apply(NetEvent::AvatarAnimations {
                        avatar: world.agent_id,
                        anims: anims.clone(),
                        sources: vec![worn.full_id, child.full_id, other.full_id],
                    });
                    // A motion may have several sources; later sparse packets
                    // retain the earlier associations until it stops.
                    world.apply(NetEvent::AvatarAnimations {
                        avatar: world.agent_id,
                        anims,
                        sources: vec![child.full_id],
                    });
                    assert!(world.animation_stops.is_empty());
                    let cof = appearance::cof(&world.inventory).expect("COF");
                    let before = appearance::folder_links(&world.inventory, cof);
                    let detach = if killed {
                        NetEvent::ObjectsKilled {
                            handle,
                            local_ids: vec![worn.local_id, child.local_id],
                        }
                    } else {
                        worn.parent_id = 0;
                        NetEvent::ObjectUpdates {
                            handle,
                            objects: vec![worn.clone()],
                        }
                    };
                    world.apply(detach.clone());
                    assert_eq!(
                        world.animation_stops.iter().copied().collect::<HashSet<_>>(),
                        HashSet::from([root_motion, child_motion])
                    );
                    assert_eq!(world.animation_stops.len(), 2, "one stop per motion");
                    assert!(!world.animation_sources.contains_key(&worn.full_id));
                    assert!(!world.animation_sources.contains_key(&child.full_id));
                    assert_eq!(world.animation_sources.get(&other.full_id), Some(&HashSet::from([ao])));
                    if temporary {
                        assert_eq!(appearance::folder_links(&world.inventory, cof), before);
                        assert!(world.take_inventory_edits().is_empty());
                    }
                    world.animation_stops.clear();
                    world.apply(detach);
                    assert!(world.animation_stops.is_empty(), "a repeated detach has no sources left");
                }
            }
        }
    }

    #[test]
    fn killing_an_animation_source_stops_only_its_motions() {
        for attached in [false, true] {
            let (mut world, worn, handle) = scripted_detach_fixture(35);
            let mut source = prim(&worn, 950, if attached { worn.local_id } else { 0 });
            source.name_values.clear();
            world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![source.clone()],
            });
            let (motion, ao) = (Uuid::from_u128(71), Uuid::from_u128(72));
            world.apply(NetEvent::AvatarAnimations {
                avatar: world.agent_id,
                anims: vec![(motion, 1), (ao, 1)],
                sources: vec![source.full_id, worn.full_id],
            });
            // Updating a world root without changing its parent is no detach.
            world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![source.clone()],
            });
            assert!(world.animation_stops.is_empty());
            world.apply(NetEvent::ObjectsKilled {
                handle,
                local_ids: vec![source.local_id],
            });
            assert_eq!(world.animation_stops, vec![motion]);
            assert!(!world.animation_sources.contains_key(&source.full_id));
            assert_eq!(world.animation_sources.get(&worn.full_id), Some(&HashSet::from([ao])));
            assert!(world.take_inventory_edits().is_empty());
        }
    }

    #[test]
    fn teleport_and_border_crossing_kills_do_not_unlink_our_attachments() {
        for teleport in [true, false] {
            let (mut world, worn, here) = scripted_detach_fixture(35);
            let cof = appearance::cof(&world.inventory).expect("COF");
            let links = appearance::folder_links(&world.inventory, cof);
            let motion = Uuid::from_u128(71);
            world.apply(NetEvent::AvatarAnimations {
                avatar: world.agent_id,
                anims: vec![(motion, 1)],
                sources: vec![worn.full_id],
            });
            if teleport {
                world.teleporting = true;
            } else {
                world.apply(NetEvent::MainRegionChanged {
                    handle: here + (256u64 << 32),
                });
            }
            world.apply(NetEvent::ObjectsKilled {
                handle: here,
                local_ids: vec![worn.local_id],
            });
            assert!(world.worn_attachment_items().contains(&Uuid::from_u128(717)));
            assert_eq!(appearance::folder_links(&world.inventory, cof), links);
            assert!(world.take_inventory_edits().is_empty());
            assert!(
                world.animation_stops.is_empty(),
                "crossing kills do not detach the animation source"
            );
            assert_eq!(world.animation_sources.get(&worn.full_id), Some(&HashSet::from([motion])));
        }
    }

    #[test]
    fn child_other_avatar_and_temporary_attachment_detaches_do_not_unlink_our_outfit() {
        for kind in 0..3 {
            let (mut world, worn, handle) = scripted_detach_fixture(35);
            let mut other = prim(&worn, 950, worn.local_id);
            other.name_values.clone_from(&worn.name_values);
            if kind == 1 {
                let mut avatar = crate::demo::action_avatar(false);
                avatar.local_id = 951;
                avatar.full_id = Uuid::from_u128(951);
                other.parent_id = avatar.local_id;
                world.apply(NetEvent::ObjectUpdates {
                    handle,
                    objects: vec![avatar],
                });
            } else if kind == 2 {
                other.parent_id = world
                    .objects
                    .get(world.objects.index_of_uuid(&world.agent_id).expect("our avatar"))
                    .expect("avatar")
                    .key
                    .local_id;
                other.name_values = format!("AttachItemID STRING RW SV {}", Uuid::nil());
            }
            let cof = appearance::cof(&world.inventory).expect("COF");
            let links = appearance::folder_links(&world.inventory, cof);
            world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![other.clone()],
            });
            world.apply(NetEvent::ObjectsKilled {
                handle,
                local_ids: vec![other.local_id],
            });
            assert_eq!(appearance::folder_links(&world.inventory, cof), links);
            assert!(world.take_inventory_edits().is_empty());
        }
    }

    /// Another avatar in `handle`, made from ours.
    fn other_avatar(world: &mut World, avatar: &aurora_net::objects::ObjectUpdate, handle: RegionHandle, local_id: u32) -> Uuid {
        let mut other = avatar.clone();
        other.local_id = local_id;
        other.full_id = Uuid::from_u128(0xD00D_0000 + u128::from(local_id));
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![other.clone()],
        });
        other.full_id
    }

    /// `source` looks at `target` (its eyes when it is an avatar).
    fn look_at(world: &mut World, source: Uuid, target: Uuid) {
        world.apply(NetEvent::LookAt {
            effect: Uuid::from_u128(0xEFFE_C700 + source.as_u128()),
            source,
            target,
            offset: [0.0; 3],
            kind: lookat::CONVERSATION,
            duration: lookat::MAX_TIMEOUT,
        });
    }

    #[test]
    fn pointat_accepts_known_remote_avatars_and_cleans_up_on_kill() {
        let (mut world, avatar, _, here, _) = worn_fixture();
        let source = other_avatar(&mut world, &avatar, here, 950);
        let event = NetEvent::PointAt {
            effect: Uuid::from_u128(999),
            source,
            target: world.agent_id,
            offset: [0.0; 3],
            kind: pointat::SELECT,
            duration: f32::MAX / 4.0,
        };
        world.apply(event.clone());
        assert_eq!(world.point_at.remote.len(), 1);
        world.apply(NetEvent::ObjectsKilled {
            handle: here,
            local_ids: vec![950],
        });
        assert!(world.point_at.remote.is_empty());
        world.apply(event);
        assert!(world.point_at.remote.is_empty(), "an absent source cannot create a motion");
    }

    #[test]
    fn a_look_at_goes_with_its_avatar() {
        // issue #482: the mark of an avatar that logged off stayed on our head
        let (mut world, avatar, _, here, _) = worn_fixture();
        let me = world.agent_id;
        let other = other_avatar(&mut world, &avatar, here, 950);
        let stays = other_avatar(&mut world, &avatar, here, 951);
        look_at(&mut world, other, me);
        look_at(&mut world, stays, me);
        assert_eq!(world.look_at.remote.len(), 2);
        // the simulator kills it (logged off, gone out of view)
        world.apply(NetEvent::ObjectsKilled {
            handle: here,
            local_ids: vec![950],
        });
        assert!(!world.look_at.remote.contains_key(&other));
        assert_eq!(world.look_at.remote.get(&stays).map(|t| t.object), Some(Some(me)));
        // an effect from an avatar that is not in the world is dropped
        look_at(&mut world, other, me);
        assert!(!world.look_at.remote.contains_key(&other));
    }

    #[test]
    fn a_look_at_toward_an_object_that_goes_is_cleared() {
        let (mut world, avatar, worn, here, _) = worn_fixture();
        let other = other_avatar(&mut world, &avatar, here, 950);
        let p = prim(&worn, 900, 0);
        world.apply(NetEvent::ObjectUpdates {
            handle: here,
            objects: vec![p.clone()],
        });
        look_at(&mut world, other, p.full_id);
        world
            .look_at
            .own
            .set(lookat::FOCUS, Some(p.full_id), glam::DVec3::ZERO, Instant::now());
        world.apply(NetEvent::ObjectsKilled {
            handle: here,
            local_ids: vec![900],
        });
        // clearLookAtTarget: the avatar stays, its look-at is "none"
        assert_eq!(world.look_at.remote.get(&other), Some(&lookat::Target::NONE));
        assert_eq!(world.look_at.own.target, lookat::Target::NONE);
    }

    #[test]
    fn look_ats_go_with_the_region_left_or_closed() {
        let (mut world, avatar, _, here, there) = worn_fixture();
        let me = world.agent_id;
        let near = other_avatar(&mut world, &avatar, there, 960);
        let far_one = other_avatar(&mut world, &avatar, here, 950);
        look_at(&mut world, near, me);
        look_at(&mut world, far_one, me);
        // a neighbour's simulator closes its circuit
        world.apply(NetEvent::RegionRemoved { handle: there });
        assert!(!world.look_at.remote.contains_key(&near));
        assert!(world.look_at.remote.contains_key(&far_one));
        // a teleport far away: the region left is parked, out of view, and
        // the mark of an avatar there does not follow us
        teleport_to(&mut world, far_from(here));
        assert!(world.parked.contains_key(&here));
        assert!(world.look_at.remote.is_empty());
    }

    /// The region west of `here`.
    fn west_of(here: RegionHandle) -> RegionHandle {
        here - (256u64 << 32)
    }

    #[test]
    fn flying_into_a_neighbour_keeps_the_agent_where_it_is() {
        let (mut world, here) = teleport_fixture();
        let there = west_of(here);
        world.agent.position = Vec3::new(0.5, 50.0, 30.0);
        world.agent.velocity = Vec3::new(-15.0, 0.0, 0.0);
        let yaw = world.agent.yaw;
        world.apply(NetEvent::MainRegionChanged { handle: there });
        // the same place, in the coordinates of the region entered
        assert_eq!(world.agent.position, Vec3::new(256.5, 50.0, 30.0));
        assert_eq!(world.take_origin_shift(), Vec3::new(256.0, 0.0, 0.0));
        assert_eq!(world.take_origin_shift(), Vec3::ZERO);
        assert!(!world.movement_complete);
        // the region entered completes the move: nothing is placed again
        world.apply(NetEvent::AgentMovementComplete {
            handle: there,
            position: Vec3::new(254.0, 50.0, 30.0),
            look_at: Vec3::Y,
        });
        assert!(world.arrived_by_crossing && world.movement_complete);
        assert_eq!(world.agent.position, Vec3::new(256.5, 50.0, 30.0));
        assert_eq!(world.agent.velocity, Vec3::new(-15.0, 0.0, 0.0));
        assert_eq!(world.agent.yaw, yaw);
        assert_eq!(world.take_origin_shift(), Vec3::ZERO);
    }

    #[test]
    fn hover_editing_and_sending_recover_in_either_order_after_an_account_switch() {
        use aurora_net::NetCommand;
        let (mut world, here) = teleport_fixture();
        for account in [1, 2] {
            world.reset();
            world.agent_id = Uuid::from_u128(account);
            world.apply(NetEvent::MainRegionChanged { handle: here });
            assert!(!world.hover_height_available());
            assert_eq!(world.hover_height_target(), None);
            let caps = Arc::new(HashMap::from([("AgentPreferences".into(), "http://127.0.0.1/preferences".into())]));
            if account == 1 {
                world.apply(NetEvent::Capabilities {
                    handle: here,
                    caps: caps.clone(),
                });
                assert!(!world.hover_height_available());
                assert_eq!(world.hover_height_target(), None);
            }
            let features = |enabled| NetEvent::SimulatorFeatures {
                handle: here,
                voice_server_type: String::new(),
                avatar_hover_height: enabled,
                materials_rate: None,
                materials_max: None,
                lsl_syntax_id: Uuid::nil(),
                grid_status_feed: String::new(),
                grid_status_page: String::new(),
            };
            world.apply(features(true));
            assert!(world.hover_height_available());
            let value = if account == 1 { 0.2 } else { -0.1 };
            world.hover_height.preview(value);
            world.hover_height.commit();
            assert_eq!(world.appearance_hover(&world.agent_id), value);
            if account == 2 {
                // Local editing works before the new session's write cap.
                assert_eq!(world.hover_height_target(), None);
                assert!(world.hover_height.next_update(None).is_none());
                world.apply(NetEvent::Capabilities { handle: here, caps });
            }
            assert_eq!(world.hover_height_target(), Some(here));
            let target = world.hover_height_target();
            assert!(
                matches!(world.hover_height.next_update(target), Some(NetCommand::SetHoverHeight { value: sent, .. }) if sent == value)
            );
            world.apply(features(false));
            assert!(!world.hover_height_available());
            assert_eq!(world.hover_height_target(), None);
            assert_eq!(world.appearance_hover(&world.agent_id), 0.0);
        }
    }

    #[test]
    fn inventory_fetches_lost_in_a_crossing_are_asked_again() {
        use inventory::{FetchState, Folder};
        let (mut world, here) = teleport_fixture();
        let there = west_of(here);
        let cof = Uuid::from_u128(46);
        world.inventory.folders.insert(
            cof,
            Folder {
                info: aurora_net::inventory::InvFolder {
                    id: cof,
                    type_default: 46,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Unknown,
                library: false,
            },
        );
        world.inventory.request(cof);
        world.inventory.queue.clear();
        world.apply(NetEvent::MainRegionChanged { handle: there });
        // sent on the capabilities of the region left: asked again once
        world.apply(NetEvent::InventoryFetchFailed { folders: vec![cof] });
        assert_eq!(world.inventory.queue, vec![(cof, false)]);
        world.inventory.queue.clear();
        // the region entered has no capabilities yet: it waits for them
        world.apply(NetEvent::InventoryFetchFailed { folders: vec![cof] });
        assert!(world.inventory.queue.is_empty());
        assert_eq!(world.inventory.folders[&cof].state, FetchState::Failed);
        // a neighbour's capabilities change nothing
        let caps = std::sync::Arc::new(HashMap::from([("FetchInventoryDescendents2".to_owned(), "x".to_owned())]));
        world.apply(NetEvent::Capabilities {
            handle: here,
            caps: caps.clone(),
        });
        assert!(world.inventory.queue.is_empty());
        world.apply(NetEvent::Capabilities { handle: there, caps });
        assert_eq!(world.inventory.queue, vec![(cof, false)]);
        assert_eq!(world.inventory.folders[&cof].state, FetchState::Fetching);
    }

    #[test]
    fn crossing_announced_by_the_arrival_alone_shifts_the_agent_too() {
        let (mut world, here) = teleport_fixture();
        world.agent.position = Vec3::new(0.5, 50.0, 30.0);
        world.apply(NetEvent::AgentMovementComplete {
            handle: west_of(here),
            position: Vec3::new(255.0, 50.0, 30.0),
            look_at: Vec3::Y,
        });
        assert!(world.arrived_by_crossing);
        assert_eq!(world.agent.position, Vec3::new(256.5, 50.0, 30.0));
        assert_eq!(world.take_origin_shift(), Vec3::new(256.0, 0.0, 0.0));
    }

    #[test]
    fn teleport_to_a_neighbour_still_places_the_agent() {
        let (mut world, here) = teleport_fixture();
        let there = west_of(here);
        world.agent.position = Vec3::new(0.5, 50.0, 30.0);
        world.apply(NetEvent::TeleportStarted);
        world.apply(NetEvent::MainRegionChanged { handle: there });
        assert_eq!(world.take_origin_shift(), Vec3::ZERO);
        world.apply(NetEvent::AgentMovementComplete {
            handle: there,
            position: Vec3::new(200.0, 20.0, 40.0),
            look_at: Vec3::Y,
        });
        assert!(!world.arrived_by_crossing);
        assert_eq!(world.agent.position, Vec3::new(200.0, 20.0, 40.0));
        assert_eq!(world.agent.velocity, Vec3::ZERO);
    }

    #[test]
    fn arrival_far_from_the_agent_places_it_even_without_a_teleport() {
        let (mut world, here) = teleport_fixture();
        world.agent.position = Vec3::new(10.0, 50.0, 30.0);
        world.apply(NetEvent::AgentMovementComplete {
            handle: here,
            position: Vec3::new(200.0, 50.0, 30.0),
            look_at: Vec3::Y,
        });
        assert!(!world.arrived_by_crossing);
        assert_eq!(world.agent.position, Vec3::new(200.0, 50.0, 30.0));
    }

    fn arrived_fixture() -> (World, aurora_net::objects::ObjectUpdate, RegionHandle) {
        let (mut world, update, handle) = fixture();
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![update.clone()],
        });
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: update.position,
            look_at: Vec3::X,
        });
        (world, update, handle)
    }

    /// Item 6 of the network audit: with Aurora's movement our avatar's
    /// report moves from when the network read it, not from when a frame
    /// got to it; with Firestorm's, from the frame, as on Firestorm.
    #[test]
    fn our_avatars_report_moves_from_when_it_was_received() {
        let (mut world, update, handle) = arrived_fixture();
        world.agent.set_predicting(true);
        let received = Instant::now() - std::time::Duration::from_millis(100);
        let terse = TerseUpdate {
            local_id: update.local_id,
            state: 0,
            is_avatar: true,
            foot_plane: None,
            position: update.position,
            velocity: Vec3::X * 4.0,
            acceleration: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            texture_entry: None,
        };
        world.apply_received(
            NetEvent::TerseUpdates {
                handle,
                updates: vec![terse.clone()],
            },
            received,
        );
        world.predict_agent(1.0, 0.0);
        let moved = world.agent.predicted_position().x - update.position.x;
        assert!(moved >= 0.4 - 1e-3, "{moved}");
        // Firestorm's movement: the frame's time
        let (mut world, update, handle) = arrived_fixture();
        world.apply_received(
            NetEvent::TerseUpdates {
                handle,
                updates: vec![terse],
            },
            received,
        );
        world.predict_agent(1.0, 0.0);
        let moved = world.agent.predicted_position().x - update.position.x;
        assert!(moved < 0.1, "{moved}");
    }

    /// Aurora's movement starts our walk with the key, over the
    /// simulator's signals, which stay as they came; off, or seated, the
    /// simulator's list alone.
    #[test]
    fn with_aurora_movement_our_walk_follows_the_keys_but_never_seated() {
        use crate::own_motion::{ANIM_STAND, ANIM_WALK, Intent};
        let (mut world, _, _) = arrived_fixture();
        let me = world.agent_id;
        world.apply(NetEvent::AvatarAnimations {
            avatar: me,
            anims: vec![(ANIM_STAND, 1)],
            sources: Vec::new(),
        });
        let walk = Intent::from_flags(aurora_net::control::AT_POS, 0.0, false);
        let ids = |w: &World| w.animations_of(&me).iter().map(|a| a.id).collect::<Vec<_>>();
        let now = Instant::now();
        world.agent.set_predicting(true);
        world.update_own_locomotion(walk, 0.8, now, None);
        assert_eq!(ids(&world), [ANIM_WALK]);
        assert_eq!(world.animations[&me].iter().map(|a| a.id).collect::<Vec<_>>(), [ANIM_STAND]);
        world.agent.set_predicting(false);
        world.update_own_locomotion(walk, 0.8, now, None);
        assert_eq!(ids(&world), [ANIM_STAND]);
        world.agent.set_predicting(true);
        world.agent.seated = true;
        world.update_own_locomotion(walk, 0.8, now, None);
        assert_eq!(ids(&world), [ANIM_STAND]);
    }

    #[test]
    fn native_movelock_motion_includes_server_travel_and_visual_settling() {
        let (mut world, _, _) = arrived_fixture();
        world.agent.reset_motion();
        for speed in [3.2, 5.13, 15.0] {
            world.agent.velocity = Vec3::X * speed;
            assert!(world.own_motion_moving(), "walk/run/boost deceleration is still movement");
        }
        world.agent.velocity = Vec3::ZERO;
        world.agent.position -= Vec3::X * 0.2;
        assert!(world.own_motion_moving(), "a zero-velocity report does not finish visual smoothing");
        for _ in 0..60 {
            world.agent.update(&Default::default(), false, 1.0 / 60.0, 3.0);
        }
        assert!(!world.own_motion_moving());
        world.agent.velocity = Vec3::Z * 2.0;
        assert!(!world.own_motion_moving(), "ground vertical corrections are not walking");
        world.agent.flying = true;
        assert!(world.own_motion_moving(), "vertical flight must settle before airborne anchoring");
        world.agent.velocity = Vec3::ZERO;
        assert!(!world.own_motion_moving());

        world.agent.set_predicting(true);
        world.agent.position -= Vec3::X * 0.2;
        world.agent.update(&Default::default(), false, 1.0 / 60.0, 3.0);
        assert!(world.own_motion_moving(), "Aurora's drawn glide must settle too");
        for _ in 0..180 {
            world.agent.update(&Default::default(), false, 1.0 / 60.0, 3.0);
        }
        assert!(!world.own_motion_moving());
    }

    #[test]
    fn an_ao_suppressed_walk_still_predicts_key_release_without_replacing_new_server_states() {
        use crate::own_motion::{ANIM_STAND, ANIM_WALK, Intent};
        use std::time::Duration;
        let (mut world, _, _) = arrived_fixture();
        let me = world.agent_id;
        let now = Instant::now();
        world.agent.set_predicting(true);
        world.apply(NetEvent::AvatarAnimations {
            avatar: me,
            anims: vec![(ANIM_STAND, 1)],
            sources: Vec::new(),
        });
        let walk = Intent::from_flags(aurora_net::control::AT_POS, 0.0, false);
        world.update_own_locomotion(walk, 0.8, now, None);
        let basis = world
            .own_signals()
            .iter()
            .find(|a| a.id == ANIM_WALK)
            .copied()
            .expect("predicted walk");
        let custom = Uuid::from_u128(29001);
        world.apply(NetEvent::AvatarAnimations {
            avatar: me,
            anims: vec![(custom, 1)],
            sources: Vec::new(),
        });
        world.update_own_locomotion(None, 0.8, now + Duration::from_millis(100), Some(basis));
        assert!(world.own_signals().iter().any(|a| a.id == ANIM_STAND));
        assert!(!world.own_signals().iter().any(|a| a.id == ANIM_WALK));
        assert_eq!(world.animations[&me].iter().map(|a| a.id).collect::<Vec<_>>(), vec![custom]);
        let fall = crate::ao::State::Falling.motion();
        world.apply(NetEvent::AvatarAnimations {
            avatar: me,
            anims: vec![(fall, 1), (custom, 1)],
            sources: Vec::new(),
        });
        world.update_own_locomotion(None, 0.8, now + Duration::from_millis(120), Some(basis));
        assert!(world.own_signals().iter().any(|a| a.id == fall));
        assert!(!world.own_signals().iter().any(|a| a.id == ANIM_WALK));
    }

    /// Offline, any prim can be sat on and stood up from (`demo::sit_on_prim`):
    /// seated, our avatar is its child; standing, it is beside it, and the
    /// first frame standing starts from that report, with no lead.
    #[test]
    fn the_demo_seats_our_avatar_on_any_prim_and_stands_it_up() {
        let (mut world, _, handle) = arrived_fixture();
        let cube = crate::demo::events()
            .into_iter()
            .find_map(|ev| match ev {
                NetEvent::ObjectUpdates { objects, .. } => objects
                    .into_iter()
                    .find(|o| o.parent_id == 0 && o.pcode == aurora_prim::params::LL_PCODE_VOLUME),
                _ => None,
            })
            .expect("a prim of the demo");
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![cube.clone()],
        });
        world.agent.set_predicting(true);
        for ev in crate::demo::sit_on_prim(cube.local_id, cube.full_id, cube.scale) {
            world.apply(ev);
        }
        assert!(world.agent.seated);
        let me = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|i| world.objects.get(i))
            .expect("our avatar");
        assert_eq!(me.parent_id, cube.local_id);
        for ev in crate::demo::stand_from_prim(cube.position, cube.scale) {
            world.apply(ev);
        }
        assert!(!world.agent.seated);
        let stood = world.agent.position;
        assert!(
            stood.truncate().distance(cube.position.truncate()) < cube.scale.length() + 1.0,
            "{stood:?}"
        );
        assert_eq!(world.agent.wanted_lead(), Vec3::ZERO);
        assert_eq!(world.agent.predicted_position(), stood);
    }
}

#[cfg(test)]
mod group_insignia_tests {
    use super::*;

    fn membership(id: Uuid, insignia: Uuid) -> aurora_net::GroupMembership {
        aurora_net::GroupMembership {
            id,
            name: "Group".into(),
            insignia,
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    /// A new insignia saved from the group window shows at once in our
    /// group lists, conversations and profile, not after a relog.
    #[test]
    fn a_saved_insignia_is_shown_at_once() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        let (group, old, new) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        world.agent_id = Uuid::from_u128(9);
        world.groups.groups.push(membership(group, old));
        world.profiles.insert(aurora_net::AvatarProfile {
            id: world.agent_id,
            groups: vec![aurora_net::profile::ProfileGroup {
                id: group,
                name: "Group".into(),
                insignia: old,
            }],
            ..Default::default()
        });
        world.apply(NetEvent::GroupProfile(Box::new(aurora_net::search::GroupProfile {
            id: group,
            insignia: old,
            charter: "Old".into(),
            ..Default::default()
        })));
        world.save_group_info(
            group,
            aurora_net::groups::GroupInfo {
                charter: "New".into(),
                insignia: new,
                ..Default::default()
            },
        );
        assert_eq!(world.groups.group(&group).map(|g| g.insignia), Some(new));
        assert_eq!(
            world.search.groups.get(&group).map(|g| (g.insignia, g.charter.as_str())),
            Some((new, "New"))
        );
        assert_eq!(world.profiles.get(&world.agent_id).map(|p| p.groups[0].insignia), Some(new));
        assert!(
            world
                .group_data
                .out
                .iter()
                .any(|c| matches!(c, aurora_net::NetCommand::UpdateGroupInfo { group: g, info } if *g == group && info.insignia == new))
        );
    }

    /// Changed by another officer: the group's profile, read again, brings
    /// it to our membership.
    #[test]
    fn a_group_profile_reply_brings_the_new_insignia() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        let (group, old, new) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        world.groups.groups.push(membership(group, old));
        world.apply(NetEvent::GroupProfile(Box::new(aurora_net::search::GroupProfile {
            id: group,
            insignia: new,
            ..Default::default()
        })));
        assert_eq!(world.groups.group(&group).map(|g| g.insignia), Some(new));
        // removed: the group glyph
        world.apply(NetEvent::GroupProfile(Box::new(aurora_net::search::GroupProfile {
            id: group,
            insignia: Uuid::nil(),
            ..Default::default()
        })));
        assert_eq!(world.groups.group(&group).map(|g| g.insignia), Some(Uuid::nil()));
    }
}
