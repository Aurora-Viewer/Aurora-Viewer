//! Viewer-side world model fed by network events.

pub mod appearance;
pub mod blocking;
pub mod body;
pub mod calls;
pub mod chat_log;
pub mod contact_sets;
pub mod eep;
pub mod eep_env;
pub mod env;
pub mod env_apply;
pub mod env_day_edit;
pub mod env_edit;
pub mod env_select;
pub mod estate;
pub mod experiences;
pub mod favorites;
pub mod flying_at_exit;
pub mod group_data;
pub mod group_money;
pub mod groups;
pub mod hover_height;
pub mod hud_scale;
pub mod inventory;
pub mod land;
pub mod land_buy;
pub mod landmarks;
pub mod lookat;
pub mod lslbridge;
pub mod marketplace;
pub mod maturity;
pub mod motion;
pub mod mutes;
pub mod names;
pub mod notification_history;
pub mod notifications;
pub mod objects;
pub mod outfit;
pub mod parked;
pub mod place_details;
pub mod profiles;
pub mod radar;
pub mod recent;
pub mod region_restart;
pub mod script_debug;
pub mod search;
pub mod social;
pub mod status;
pub mod teleport_invites;
pub mod terrain;
pub mod tphistory;
pub mod transcripts;
pub mod windlight_presets;
pub mod worldmap;

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
    pub caps: Arc<HashMap<String, String>>,
    /// Voice server of the region (SimulatorFeatures): "webrtc", "vivox" or empty.
    pub voice_server: String,
    pub avatar_hover_height: bool,
    /// RenderMaterials limits of the region (SimulatorFeatures).
    pub materials_limits: crate::scene::legacy_mat::RegionLimits,
    /// Object capacity (SimStats ObjectCapacity, LLViewerRegion::getMaxTasks);
    /// 0 until known.
    pub max_tasks: u32,
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
    Own,
    /// The card of a teleport offer or request (`teleport_invites.rs`),
    /// by its invitation: never written to the transcript.
    Invite(u64),
}

impl ChatKind {
    /// A notice of the viewer rather than someone's message: it stands
    /// apart from the groups of messages.
    pub fn is_notice(self) -> bool {
        matches!(self, ChatKind::System | ChatKind::Invite(_))
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

/// One simulator signal, with independent sequence and continuous clocks.
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

pub struct World {
    /// mScreen's Y scale (LLVOAvatarSelf::updateCharacter).
    pub hud_aspect: f32,
    /// HUD zoom is local to the current session (LLAgentCamera).
    pub hud_zoom: f32,
    pub hud_scales: hud_scale::HudScales,
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
    pub(crate) scripted_camera: crate::camera::follow::Sources,
    /// When the clocks of the regions were last advanced (`advance_motion`).
    motion_tick: Instant,
    /// GLTF material overrides by object, (face, override) pairs. Kept apart
    /// from the objects like LL's region override cache: a message may come
    /// before its object, and the scene merges them when it builds the faces.
    pub gltf_overrides: HashMap<ObjKey, Arc<[(u8, aurora_assets::material::PbrOverride)]>>,
    pub chat: VecDeque<ChatLine>,
    /// Follows `chat` for the view.
    pub chat_clock: transcripts::LineClock,
    pub chat_unread: usize,
    /// Conversation transcripts of the account logged in.
    pub chat_log: chat_log::ChatLog,
    /// History of the nearby chat (asked once per login).
    nearby_history: transcripts::HistoryState,
    pub script_debug: script_debug::Messages,
    pub sun: Option<(SunInfo, Instant)>,
    pub agent: AgentState,
    pub coarse: HashMap<RegionHandle, Vec<(Uuid, Vec3)>>,
    /// Objects of the regions out of reach, until their simulator closes
    /// them or they are in reach again (parked.rs).
    parked: HashMap<RegionHandle, parked::Parked>,
    appearances: HashMap<Uuid, (AvatarAppearance, u64)>,
    appearance_counter: u64,
    shape_preview: Option<Vec<u8>>,
    shape_preview_generation: u64,
    pub alerts: VecDeque<String>,
    /// The announced restart of the agent's region (region_restart.rs).
    pub region_restart: region_restart::Tracker,
    pub movement_complete: bool,
    pub avatar_lib: Arc<AvatarLibrary>,
    /// Animated joint matrices (skeleton space, rest -> posed) of the avatars
    /// near the camera, so attachments follow their bone.
    pub avatar_poses: HashMap<Uuid, Vec<glam::Mat4>>,
    /// Pelvis offset from body size, shape hover and mesh overrides only.
    /// The adjustable hover height stays live, independent of pose timing.
    pub avatar_shape_dz: HashMap<Uuid, f32>,
    /// Drawn body orientation per standing avatar (LLVOAvatar::updateOrientation).
    pub bodies: HashMap<Uuid, body::Body>,
    /// Where avatars look (ours and the ones others send).
    pub look_at: lookat::LookAts,
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
    pub balance: Option<i32>,
    /// m² of land the account holds (MoneyBalanceReply SquareMetersCommitted,
    /// LLStatusBar::getSquareMetersCommitted).
    pub land_committed: i32,
    /// m² the account may hold (SquareMetersCredit): what is left is
    /// LLStatusBar::getSquareMetersLeft.
    pub land_credit: i32,
    pub upload_cost: Option<i32>,
    pub local_assets: HashMap<Uuid, (String, bool)>,
    /// ExtEnvironment answers of the frame (parcel local id, error), for the
    /// environment editors' "Apply To Parcel / Region" (drained by the app).
    pub environment_answers: Vec<(i32, Result<(), String>)>,
    /// Health in percent (HealthMessage; only changes on damage-enabled land).
    pub health: f32,
    /// Interface sounds to play (drained by the app).
    pub ui_sounds: Vec<UiSound>,
    /// Conversation messages received (their sound depends on the IM modes).
    pub im_messages: Vec<ImMessage>,
    /// L$ balance changes (previous, new), for the money sounds.
    pub balance_changes: Vec<(Option<i32>, i32)>,
    /// Avatars that just started typing (typing sound, LLVOAvatar::startMotion).
    pub typing_started: Vec<Uuid>,
    pub social: social::Social,
    /// Contact sets and aliases (LGGContactSets), per account.
    pub contact_sets: contact_sets::ContactSets,
    /// The residents we dealt with lately (People › Recent).
    pub recent: recent::RecentPeople,
    /// Contact sets generation whose aliases the name cache has.
    aliases_synced: u64,
    /// Block list (LLMuteList).
    pub mutes: mutes::MuteList,
    /// Firestorm LSL bridge messages kept out of nearby chat.
    pub lsl_bridge: lslbridge::LslBridge,
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
    /// Refused offers to move to the Trash (`take_inventory_edits`).
    inventory_edits: Vec<aurora_net::inventory::operations::Mutation>,
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
    /// Motions to stop on the simulator after leaving a seat (drained by app).
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
    /// About Land: the selected parcel, covenant, group / experience names.
    pub land: land::Land,
    /// The Region / Estate window's region and estate.
    pub estate: estate::Estate,
    /// Media texture overrides and parcel media messages (media module).
    pub media: crate::media::WorldMedia,
    /// Avatar profiles and the ids still to request.
    pub profiles: profiles::Profiles,
    /// Notification center (bell).
    pub notifications: notifications::Notifications,
    /// Teleport offers and requests from other residents, and their answers.
    pub teleport_invites: teleport_invites::TeleportInvites,
    /// The content rating chosen during the session and its sync with the
    /// server.
    pub maturity: maturity::MaturityPreference,
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
            hud_zoom: 1.0,
            hud_scales: Default::default(),
            hover_height: Default::default(),
            favorites: Default::default(),
            notification_history: Default::default(),
            ui_sounds: Vec::new(),
            im_messages: Vec::new(),
            balance_changes: Vec::new(),
            typing_started: Vec::new(),
            login: None,
            agent_id: Uuid::nil(),
            regions: HashMap::new(),
            main_region: None,
            objects: ObjectStore::default(),
            scripted_camera: Default::default(),
            motion_tick: Instant::now(),
            gltf_overrides: HashMap::new(),
            chat: VecDeque::new(),
            chat_clock: Default::default(),
            chat_unread: 0,
            chat_log: Default::default(),
            nearby_history: Default::default(),
            script_debug: script_debug::Messages::default(),
            sun: None,
            agent: AgentState::default(),
            coarse: HashMap::new(),
            parked: HashMap::new(),
            appearances: HashMap::new(),
            avatar_poses: HashMap::new(),
            avatar_shape_dz: HashMap::new(),
            bodies: HashMap::new(),
            look_at: lookat::LookAts::default(),
            appearance_counter: 1,
            shape_preview: None,
            shape_preview_generation: 0,
            alerts: VecDeque::new(),
            region_restart: Default::default(),
            movement_complete: false,
            avatar_lib,
            teleporting: false,
            tp_show_progress: false,
            tp_progress: 0.0,
            tp_status: String::new(),
            tp_failed: false,
            outfit: Default::default(),
            diag_reported: HashSet::new(),
            events_this_frame: 0,
            balance: None,
            land_committed: 0,
            land_credit: 0,
            upload_cost: None,
            local_assets: HashMap::new(),
            environment_answers: Vec::new(),
            health: 100.0,
            social: social::Social::default(),
            contact_sets: Default::default(),
            recent: Default::default(),
            aliases_synced: 0,
            mutes: mutes::MuteList::default(),
            lsl_bridge: lslbridge::LslBridge::default(),
            groups: groups::GroupChats::default(),
            calls: calls::Calls::default(),
            status: status::Status::default(),
            inventory: inventory::Inventory::default(),
            active_gestures: Default::default(),
            inventory_cache: None,
            inventory_edits: Vec::new(),
            animations: HashMap::new(),
            ao_pose: None,
            own_locomotion: Default::default(),
            own_drawn: None,
            received: None,
            local_preview: None,
            animation_preview_clock: None,
            preview_server_motion: None,
            animation_sources: HashMap::new(),
            animation_stops: Vec::new(),
            skeleton_resets: Vec::new(),
            eep: eep_env::EnvSelector::default(),
            env_clock: Instant::now(),
            cloud_scroll: glam::Vec2::ZERO,
            sky_directions: (glam::Vec3::Z, glam::Vec3::NEG_Z),
            parcel_name: String::new(),
            parcel: None,
            land: land::Land::default(),
            estate: estate::Estate::default(),
            media: Default::default(),
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
            search: Default::default(),
            group_data: Default::default(),
            group_money: Default::default(),
            notifications: notifications::Notifications::default(),
            teleport_invites: teleport_invites::TeleportInvites::default(),
            maturity: maturity::MaturityPreference::default(),
            experiences: experiences::Experiences::default(),
            profiles: profiles::Profiles::default(),
            arrived_once: false,
            offline_im_requested: false,
            cache_dir: None,
        }
    }

    /// Back to the login screen. Nothing said to the account that leaves
    /// stays for the next one: the conversations and the nearby chat go, and
    /// the transcripts close (only their writer and settings are kept).
    pub fn reset(&mut self) {
        let lib = self.avatar_lib.clone();
        let cache_dir = self.cache_dir.take();
        let mut chat_log = std::mem::take(&mut self.chat_log);
        chat_log.logout();
        *self = World::new(lib);
        self.chat_log = chat_log;
        self.cache_dir = cache_dir;
    }

    /// LLTeleportHistory::onTeleportFinished → LLTeleportHistoryStorage:
    /// the arrival is saved once the parcel of the new place is known.
    fn record_arrival(&mut self) {
        let Some((global, _)) = self.tp_record.take() else {
            return;
        };
        let region = self
            .main_region
            .and_then(|h| self.regions.get(&h))
            .map(|r| r.name.clone())
            .unwrap_or_default();
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
        let stale = |w: &World, i: &usize| w.objects.get(*i).is_some_and(|o| o.key.region != region);
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
        self.teleporting = true;
        self.tp_show_progress = show_progress;
        self.tp_failed = false;
        self.tp_progress = 0.05;
        self.tp_status = aurora_i18n::tr!("world-mod-requesting-teleport").into();
        if show_progress {
            self.ui_sounds.push(UiSound::TeleportOut);
        }
    }

    /// Offset of a region's origin relative to the render origin (main region).
    pub fn region_offset(&self, h: RegionHandle) -> Option<Vec3> {
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

    /// Let the clocks of the regions run up to `now` (motion.rs): each at
    /// the time dilation of its simulator, slowing to a stop when its
    /// circuit goes silent. A region without a simulator in `sims` (the
    /// offline demo, a region parked or just announced) runs at full rate.
    /// Corrections that have faded end here.
    pub fn advance_motion(&mut self, now: Instant, sims: &[aurora_net::stats::SimulatorMotion]) {
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
    pub fn update_own_locomotion(&mut self, intent: Option<crate::own_motion::Intent>, window: f32, now: Instant) {
        let standing = self.agent.has_local_control() && !self.agent.ground_sit;
        if !self.agent.predicting() || !standing || self.teleporting || !self.movement_complete {
            self.own_locomotion.reset();
            self.own_drawn = None;
            return;
        }
        let base = self.animations.get(&self.agent_id).map(Vec::as_slice).unwrap_or_default();
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
        self.nearby_push(line);
        self.chat_unread += 1;
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

    pub fn system_message(&mut self, text: impl Into<String>) {
        self.push_chat(ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: text.into(),
            kind: ChatKind::System,
            source: Uuid::nil(),
            origin: Origin::Live,
        });
    }

    fn ensure_region(&mut self, h: RegionHandle, size: (u32, u32)) -> &mut Region {
        self.regions.entry(h).or_insert_with(|| Region {
            handle: h,
            name: String::new(),
            info: None,
            heightmap: Heightmap::new(size.0.max(256), size.1.max(256)),
            wind: Default::default(),
            dirty_chunks: HashSet::new(),
            caps: Arc::new(HashMap::new()),
            voice_server: String::new(),
            avatar_hover_height: false,
            materials_limits: Default::default(),
            max_tasks: 0,
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
                self.hover_height.value()
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
        self.main()
            .is_some_and(|region| region.avatar_hover_height && region.caps.contains_key("AgentPreferences"))
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

    /// Run the outfit restore; its network commands are left in
    /// `self.outfit.commands`.
    pub fn update_outfit(&mut self, second_life: bool, now: Instant) {
        if self.outfit.is_done() {
            return;
        }
        let ready = self.movement_complete && self.main().is_some_and(|r| r.caps.contains_key("FetchInventoryDescendents2"));
        let first_login = self
            .login
            .as_ref()
            .is_some_and(|l| l.raw["login-flags"].at(0)["ever_logged_in"].as_str() == "N");
        let inputs = outfit::OutfitInputs {
            worn: self.worn_attachment_items(),
            received_cof_version: self.appearance_cof_version(&self.agent_id),
            inventory: &mut self.inventory,
            agent: self.agent_id,
            ready,
            first_login,
            second_life,
        };
        self.outfit.update(inputs, now);
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
            + if *avatar == self.agent_id {
                self.shape_preview_generation
            } else {
                0
            }
    }

    /// Baked texture id for an avatar (by object index) and TE slot.
    pub fn bake_texture(&self, avatar_idx: usize, te_index: usize) -> Option<Uuid> {
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
        let ev = self.hold_for_parked(ev)?;
        match ev {
            NetEvent::LoggedIn(l) => {
                self.agent_id = l.agent_id;
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
                self.status.invisible = status::Invisible::load(account.join(status::INVISIBLE_FILE), &self.social.friends);
                self.calls.load_volumes(account.join(calls::VOLUMES_FILE));
                self.tp_storage = tphistory::HistoryStorage::load(account.join(tphistory::TELEPORT_HISTORY_FILE));
                self.hud_scales = hud_scale::HudScales::load(account.join(hud_scale::FILE));
                self.aliases_synced = u64::MAX;
                self.sync_contact_aliases();
                self.inventory = inventory::Inventory::from_login(&l.raw);
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
                r.info = Some(info);
                None
            }
            NetEvent::Capabilities { handle, caps } => {
                self.ensure_region(handle, (256, 256)).caps = caps;
                // the folders that failed without them (a crossing, a
                // teleport) are asked again on the agent's new region
                if Some(handle) == self.main_region {
                    self.inventory.retry_failed();
                }
                None
            }
            NetEvent::SimulatorFeatures {
                handle,
                voice_server_type,
                avatar_hover_height,
                materials_rate,
                materials_max,
            } => {
                let r = self.ensure_region(handle, (256, 256));
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
                self.movement_complete = true;
                if self.teleporting || !self.arrived_once {
                    let region = self.regions.get(&handle).map(|r| r.name.clone()).unwrap_or_default();
                    self.tp_history.arrived(tphistory::TpEntry { region, handle, position });
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
                Some(NetEvent::AgentMovementComplete { handle, position, look_at })
            }
            NetEvent::ObjectUpdates { handle, objects } => {
                let me = self.agent_id;
                // "About Land" open on a parcel of this region: a
                // prim that was not there changes its counts
                let mut land_changed = false;
                let land_watches = self.land.watches(handle);
                for u in objects {
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
                    let pos = u.position;
                    let velocity = u.velocity;
                    let acceleration = u.acceleration;
                    let parent = u.parent_id;
                    if is_me {
                        self.on_seat_parent_change(handle, parent);
                    }
                    self.on_camera_parent_change(handle, &u);
                    self.objects.upsert_at(handle, u, self.received);
                    if is_me {
                        let at = self.agent_report_time();
                        self.agent
                            .on_server_update_at(pos, velocity, acceleration, parent != 0, Some(handle) == self.main_region, at);
                    }
                }
                self.move_rekeyed();
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
                let land_watches = self.land.watches(handle);
                let mut land_changed = false;
                for id in local_ids {
                    let key = ObjKey {
                        region: handle,
                        local_id: id,
                    };
                    if self.kill_is_ignored(&key) {
                        continue;
                    }
                    land_changed |= land_watches
                        && self
                            .objects
                            .index_of(&key)
                            .and_then(|i| self.objects.get(i))
                            .is_some_and(|o| !o.is_avatar());
                    if let Some(i) = self.objects.index_of(&key) {
                        for child in self.objects.subtree(i) {
                            if let Some(o) = self.objects.get(child) {
                                self.scripted_camera.clear(o.full_id);
                            }
                        }
                    }
                    self.objects.remove(&key);
                    // a cache hit replays the cached ones, a full update comes with them
                    self.gltf_overrides.remove(&key);
                }
                if land_changed {
                    self.land.objects_changed(handle, Instant::now());
                }
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
                self.appearance_counter += 1;
                let id = a.avatar_id;
                if id == self.agent_id {
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
            NetEvent::TeleportFinished { handle } => {
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
                Some(NetEvent::TeleportFinished { handle })
            }
            NetEvent::TeleportLocal { flying } => {
                // process_teleport_local, indra/newview/llviewermessage.cpp.
                // This also covers server-initiated teleports: the following
                // AgentMovementComplete must record the arrival in history.
                self.teleporting = true;
                self.tp_show_progress = false;
                self.agent.flying = flying;
                None
            }
            NetEvent::TeleportStarted => {
                if !self.teleporting {
                    self.tp_show_progress = false;
                    log::info!("TeleportStart (arrived {})", self.arrived_once);
                }
                self.teleporting = true;
                self.tp_failed = false;
                self.tp_progress = self.tp_progress.max(0.10);
                self.tp_status = aurora_i18n::tr!("common-teleport-accepted").into();
                self.system_message(aurora_i18n::tr!("world-mod-teleporting"));
                None
            }
            NetEvent::TeleportFailed { reason } => {
                self.tp_history.failed();
                self.teleporting = false;
                self.tp_failed = true;
                self.system_message(aurora_i18n::tr!("world-mod-teleport-failed", reason = reason.to_string()));
                None
            }
            NetEvent::Alert { message } => {
                self.map.on_alert(&message);
                let restart = message.to_lowercase().contains("restart");
                self.ui_sounds.push(if restart { UiSound::Restart } else { UiSound::Alert });
                self.system_message(message.clone());
                self.notifications.push(
                    notifications::Kind::Alert,
                    aurora_i18n::tr!("world-mod-simulator-alert"),
                    message.clone(),
                    notifications::Data::None,
                );
                self.alerts.push_back(message);
                None
            }
            NetEvent::ScriptDialog {
                object_id, object_name, ..
            } if self.object_blocked(&object_id, &object_name) => None,
            // process_script_question drops only the questions of a blocked
            // object (LLMuteList::isMuted(taskid)): not those of a blocked
            // owner, nor on any other condition (no click needed, object out
            // of view or unknown, another region, Do Not Disturb)
            NetEvent::ScriptQuestion { task_id, .. } if self.mutes.is_muted_id(&task_id) => None,
            NetEvent::LoadUrl {
                object_id, object_name, ..
            } if self.object_blocked(&object_id, &object_name) => None,
            NetEvent::ScriptDialog {
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
                        object: object_id,
                        channel,
                    }
                } else {
                    let buttons = if buttons.is_empty() { vec!["OK".to_owned()] } else { buttons };
                    notifications::Data::Dialog {
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
                self.notifications.push(notifications::Kind::Script, title, message, data);
                // LLScriptFloater ("script_floater") opening
                self.ui_sounds.push(UiSound::ScriptFloaterOpen);
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
                    self.notifications.push(
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
                }
                None
            }
            NetEvent::ExperienceInfo(list) => {
                for question in self.experiences.arrived(list) {
                    self.push_experience_question(question);
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
                object_name, message, url, ..
            } => {
                let body = format!("{message}\n{url}");
                self.notifications.push(
                    notifications::Kind::Url,
                    aurora_i18n::tr!("extra-offers-a-link", object_name = object_name.to_string()),
                    body,
                    notifications::Data::Url(url),
                );
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
            NetEvent::Names(list) => {
                for (id, first, last) in list {
                    self.social.avatar_names.on_legacy(id, &first, &last);
                    self.social.names.insert(id, social::format_name(&first, &last));
                }
                None
            }
            NetEvent::DisplayNames { names, bad_ids, max_age } => {
                self.social.avatar_names.on_display_names(&names, &bad_ids, max_age);
                None
            }
            NetEvent::DisplayNamesFailed(ids) => {
                self.social.avatar_names.on_failed(&ids);
                None
            }
            NetEvent::DisplayNameUpdate { name, old_display_name } => {
                let notice = self.social.avatar_names.on_update(&name, &old_display_name);
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
            NetEvent::AvatarAnimations { avatar, anims, sources } => {
                let now = Instant::now();
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
                        if !source.is_nil() && self.objects.index_of_uuid(&source).is_some() {
                            self.animation_sources.entry(source).or_default().insert(*id);
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
                }
                self.land.apply(ev, &ctx);
                None
            }
            NetEvent::Estate(ev) => {
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
            NetEvent::AgentParcel(info) => {
                self.parcel_name = info.name.clone();
                if !info.is_group_owned {
                    self.social.want_name(info.owner_id);
                }
                self.parcel = Some(info);
                self.record_arrival();
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
                self.profiles.classifieds.insert(c.id, *c);
                None
            }
            NetEvent::ClassifiedStats { id, stats } => {
                self.profiles.classified_stats.insert(id, stats);
                None
            }
            NetEvent::ParcelInfo(p) => {
                self.profiles.parcels.insert(p.id, *p);
                None
            }
            NetEvent::DirReply(r) => {
                self.search.apply_reply(&r);
                None
            }
            NetEvent::EventInfo(e) => {
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
                None
            }
            NetEvent::GroupLand(reply) => {
                // process_places_reply: only for a group of ours
                if self.groups.is_member(&reply.group) {
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
                self.search.groups.insert(g.id, *g);
                None
            }
            NetEvent::ProductInfo(types) => {
                self.search.apply_land_types(types);
                None
            }
            NetEvent::RemoteParcel { handle, position, result } => {
                self.map.apply_remote_parcel(handle, position, result.as_ref().ok().copied());
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
            NetEvent::InventoryUpdated { folders, items } => {
                self.inventory.server_updated(folders, items);
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
                self.map.agent_found(prey, x, y, self.agent.position.z);
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
                if source != self.agent_id {
                    self.look_at.receive(effect, source, target, offset, kind);
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
        if c.message.is_empty() || self.lsl_bridge.hides(&c) || self.chat_blocked(&c) {
            return;
        }
        if self.script_debug.route(&c, self.agent_id) {
            return;
        }
        let kind = match c.source_type {
            ChatSourceType::System => ChatKind::System,
            ChatSourceType::Object => ChatKind::Object(c.chat_type),
            ChatSourceType::Agent => {
                if c.source_id == self.agent_id {
                    ChatKind::Own
                } else {
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
        self.push_chat(ChatLine {
            time: SystemTime::now(),
            from,
            text: c.message,
            kind,
            source: c.source_id,
            origin: Origin::Live,
        });
    }

    fn on_im(&mut self, im: InstantMessage) {
        if self.im_blocked(&im) {
            return;
        }
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
        // VoiceCallsFriendsOnly: a non-friend's message is neither shown nor
        // answered
        if im.dialog == 0 && self.friends_only_drops(&im.from_agent_id, &im.from_name) {
            log::info!("message from a non-friend dropped (friends only)");
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
            self.send_autoresponse(&im, autoresponse);
            return;
        }
        // dialog 19 = MessageFromObject
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
            d::INVENTORY_OFFERED | d::TASK_INVENTORY_OFFERED => {
                self.ui_sounds.push(UiSound::InventoryOffer);
                let task = im.dialog == d::TASK_INVENTORY_OFFERED;
                let asset_type = im.binary_bucket.first().map(|b| *b as i8).unwrap_or(-1);
                // A resident's offer is in our inventory before we answer:
                // prefetch it (inventory_offer_handler, llimprocessing.cpp),
                // or it stays unseen until the next login. A folder cannot be
                // fetched by id; it lands in the root, reloaded instead.
                let object = (!task)
                    .then(|| notifications::offered_object(&im.binary_bucket))
                    .flatten()
                    .map_or(Uuid::nil(), |(_, id)| id);
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
                let what = if im.message.is_empty() {
                    aurora_i18n::tr!("world-mod-an-object").to_owned()
                } else {
                    format!("« {} »", im.message)
                };
                n.push(
                    Kind::Inventory,
                    aurora_i18n::tr!("world-mod-gives-you", from = from.to_string(), what = what.to_string()),
                    String::new(),
                    Data::Inventory {
                        from: im.from_agent_id,
                        tx: im.session_id,
                        asset_type,
                        object,
                        task,
                    },
                );
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
                // "subject|message"
                let (subject, body) = im
                    .message
                    .split_once('|')
                    .unwrap_or((aurora_i18n::tr!("common-group-notice"), im.message.as_str()));
                n.push(Kind::Group, format!("{from} : {subject}"), body.to_owned(), Data::None);
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
        true
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
        self.notifications.push(
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
                    name,
                    object_name: q.object_name,
                }),
            },
        );
        // UISndQuestionExperience is Firestorm's own; the script sound is Aurora's
        self.ui_sounds.push(UiSound::ScriptFloaterOpen);
    }

    /// Answer a notification: messages to send and a URL to open.
    pub fn respond_notification(&mut self, id: u64, r: notifications::Response) -> (Vec<aurora_net::NetCommand>, Option<String>) {
        let data = self.notifications.list.iter().find(|n| n.id == id).map(|n| n.data.clone());
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
        let out = self.notifications.respond(
            id,
            r,
            |t| folders.iter().find(|(ft, _)| *ft == t).map(|(_, f)| *f).unwrap_or(root),
            calling,
        );
        if let Some((asset_type, object)) = discard {
            self.discard_offer(asset_type, object);
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

    /// Name of an avatar by id (from its object name-values).
    pub fn avatar_name(&self, id: &Uuid) -> Option<String> {
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
                self.notifications.push_presence(id, n.clone(), online);
            }
            // the line of nearby chat stays: Firestorm's own skin sends the
            // notices there (OnlineOfflinetoNearbyChat)
            self.system_message(n);
        }
    }

    /// A message received in a conversation that is not in sight
    /// (`in_sight`: its tab shown in the Conversations window): a toast and
    /// an entry under the bell until the conversation is read, as
    /// notify_of_message decides (llimview.cpp) — never in do not disturb
    /// with Firestorm's toasts, group chats by their own preference.
    pub fn notify_message(&mut self, m: &ImMessage, in_sight: bool) {
        let prefs = self.notifications.toasts;
        let wanted = match m.kind {
            ImKind::Group => prefs.group_im,
            ImKind::Private | ImKind::Conference => prefs.im,
        };
        // Aurora's paused bell still keeps them in "To do", silently
        if !wanted || in_sight || (self.status.dnd && !self.notifications.aurora) {
            return;
        }
        let Some(line) = self.social.ims.iter().find(|s| s.other == m.session).and_then(|s| s.lines.back()) else {
            return;
        };
        // only what a resident wrote (no system lines, no objects)
        if line.source.is_nil() || line.kind != ChatKind::Im || self.mutes.is_muted_id(&line.source) {
            return;
        }
        let (from, text) = (line.source, line.text.clone());
        let group = m.kind != ImKind::Private;
        let (title, body) = if group {
            (self.session_title(&m.session), format!("{}: {text}", line.from))
        } else {
            (line.from.clone(), text)
        };
        self.notifications.push_message(m.session, from, title, body);
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

    /// `id` talked with us now (LLRecentPeople::add), here.
    pub fn met(&mut self, id: Uuid) {
        let region = self.region_name();
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
        let me = self
            .login
            .as_ref()
            .map(|l| social::format_name(&l.first_name, &l.last_name))
            .unwrap_or_default();
        self.social.session_mut(to);
        let line = ChatLine {
            time: SystemTime::now(),
            from: me,
            text: text.to_owned(),
            kind: ChatKind::Own,
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
    pub fn update_bodies(&mut self, now: Instant, dt: f32, cam_at: Vec3, mouselook: bool) -> i8 {
        let mut own_turn = 0;
        let mut seen = std::collections::HashSet::new();
        for (_, o) in self.objects.avatars() {
            if o.parent_id != 0 {
                continue;
            }
            let id = o.full_id;
            let is_self = id == self.agent_id && self.agent.has_local_control();
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
                }
            } else {
                body::Input {
                    prim_dir: self.objects.predicted(o, now).1 * Vec3::X,
                    velocity: o.velocity,
                    in_air: o.velocity.z.abs() > 1.0,
                    mouselook_at: None,
                    flying: false,
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
        for idx in seat.chain(children) {
            let Some(source) = self.objects.get(idx).map(|o| o.full_id) else {
                continue;
            };
            if let Some(motions) = self.animation_sources.remove(&source) {
                for id in motions {
                    if !self.animation_stops.contains(&id) {
                        log::info!("leaving seat: stop animation {id} from {source}");
                        self.animation_stops.push(id);
                    }
                }
            }
        }
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
        world.apply(NetEvent::TeleportFinished { handle: destination });
        assert!(world.teleporting && world.tp_show_progress);
        world.apply(NetEvent::TeleportFinished { handle: destination });
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
        world.apply(NetEvent::TeleportFinished { handle });
        assert!(world.teleporting && !world.tp_show_progress);
        assert!(!world.ui_sounds.contains(&UiSound::TeleportOut));
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
    fn script_menu_ignore_sends_no_reply_and_block_suppresses_future_menus() {
        let (mut world, _, _) = fixture();
        let object_id = Uuid::from_u128(0x4321);
        let menu = || NetEvent::ScriptDialog {
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

    /// A region a teleport away from `here`.
    fn far_from(here: RegionHandle) -> RegionHandle {
        here + ((100 * 256u64) << 32)
    }

    /// A teleport to `handle`, from TeleportFinish to the arrival.
    fn teleport_to(world: &mut World, handle: RegionHandle) {
        world.apply(NetEvent::TeleportFinished { handle });
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
        world.apply(NetEvent::TeleportFinished { handle: here });
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
        world.update_own_locomotion(walk, 0.8, now);
        assert_eq!(ids(&world), [ANIM_WALK]);
        assert_eq!(world.animations[&me].iter().map(|a| a.id).collect::<Vec<_>>(), [ANIM_STAND]);
        world.agent.set_predicting(false);
        world.update_own_locomotion(walk, 0.8, now);
        assert_eq!(ids(&world), [ANIM_STAND]);
        world.agent.set_predicting(true);
        world.agent.seated = true;
        world.update_own_locomotion(walk, 0.8, now);
        assert_eq!(ids(&world), [ANIM_STAND]);
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
