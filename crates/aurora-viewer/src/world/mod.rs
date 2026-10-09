//! Viewer-side world model fed by network events.

pub mod blocking;
pub mod body;
pub mod eep;
pub mod env;
pub mod groups;
pub mod inventory;
pub mod lookat;
pub mod lslbridge;
pub mod mutes;
pub mod names;
pub mod notifications;
pub mod objects;
pub mod outfit;
pub mod social;
pub mod status;
pub mod terrain;
pub mod tphistory;
pub mod worldmap;

use crate::agent::AgentState;
use crate::scene::avatar::AvatarLibrary;
use crate::scene::textures::TexSource;
use crate::ui_sound::{ImKind, ImMessage, UiSound};
use aurora_net::{
    AvatarAppearance, ChatMessage, ChatSourceType, ChatType, InstantMessage, LoginResponse, NetEvent, RegionHandle, RegionInfo, SunInfo,
};
use glam::Vec3;
use objects::{ObjKey, ObjectStore};
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
    pub dirty_chunks: HashSet<u32>,
    pub caps: Arc<HashMap<String, String>>,
    /// Voice server of the region (SimulatorFeatures): "webrtc", "vivox" or empty.
    pub voice_server: String,
    /// RenderMaterials limits of the region (SimulatorFeatures).
    pub materials_limits: crate::scene::legacy_mat::RegionLimits,
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
}

/// Profile data of an avatar.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub image_id: Uuid,
    pub about: String,
    pub born_on: String,
}

#[derive(Debug, Clone)]
pub struct ChatLine {
    pub time: SystemTime,
    pub from: String,
    pub text: String,
    pub kind: ChatKind,
    pub source: Uuid,
}

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
    fn from_signal(id: Uuid, sequence: i32, now: Instant, previous: Option<&Self>) -> Self {
        Self {
            id,
            sequence,
            sequence_start: previous.filter(|p| p.sequence == sequence).map_or(now, |p| p.sequence_start),
            continuous_start: previous.map_or(now, |p| p.continuous_start),
        }
    }
}

pub struct World {
    pub login: Option<Arc<LoginResponse>>,
    pub agent_id: Uuid,
    pub regions: HashMap<RegionHandle, Region>,
    pub main_region: Option<RegionHandle>,
    pub objects: ObjectStore,
    /// GLTF material overrides by object, (face, override) pairs. Kept apart
    /// from the objects like LL's region override cache: a message may come
    /// before its object, and the scene merges them when it builds the faces.
    pub gltf_overrides: HashMap<ObjKey, Arc<[(u8, aurora_assets::material::PbrOverride)]>>,
    pub chat: VecDeque<ChatLine>,
    pub chat_unread: usize,
    pub sun: Option<(SunInfo, Instant)>,
    pub agent: AgentState,
    pub coarse: HashMap<RegionHandle, Vec<(Uuid, Vec3)>>,
    appearances: HashMap<Uuid, (AvatarAppearance, u64)>,
    appearance_counter: u64,
    pub alerts: VecDeque<String>,
    pub movement_complete: bool,
    pub avatar_lib: Arc<AvatarLibrary>,
    /// Animated joint matrices (skeleton space, rest -> posed) of the avatars
    /// near the camera, so attachments follow their bone.
    pub avatar_poses: HashMap<Uuid, Vec<glam::Mat4>>,
    /// Height of each posed avatar's pelvis above its object position
    /// (shape: body size, hover).
    pub avatar_root_dz: HashMap<Uuid, f32>,
    /// Drawn body orientation per standing avatar (LLVOAvatar::updateOrientation).
    pub bodies: HashMap<Uuid, body::Body>,
    /// Where avatars look (ours and the ones others send).
    pub look_at: lookat::LookAts,
    pub teleporting: bool,
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
    /// Block list (LLMuteList).
    pub mutes: mutes::MuteList,
    /// Firestorm LSL bridge messages kept out of nearby chat.
    pub lsl_bridge: lslbridge::LslBridge,
    /// Our groups and the group / conference chat sessions.
    pub groups: groups::GroupChats,
    /// Online status (away, do not disturb...) and automatic responses.
    pub status: status::Status,
    pub inventory: inventory::Inventory,
    /// Playing animations per avatar / animesh.
    pub animations: HashMap<Uuid, Vec<PlayingAnimation>>,
    /// EEP day cycle of the main region (if the region provides one).
    pub day_cycle: Option<(RegionHandle, eep::DayCycle)>,
    /// Accumulated cloud scroll (LLEnvironment::mCloudScrollDelta).
    pub cloud_scroll: glam::Vec2,
    /// Name of the parcel the agent stands on.
    pub parcel_name: String,
    /// Full description of that parcel (About Land).
    pub parcel: Option<Arc<aurora_net::ParcelInfo>>,
    /// Media texture overrides and parcel media messages (media module).
    pub media: crate::media::WorldMedia,
    /// Avatar profiles (picture, about) and the ids still to request.
    pub profiles: HashMap<Uuid, Profile>,
    profile_pending: HashSet<Uuid>,
    profile_requested: HashSet<Uuid>,
    /// Notification center (bell).
    pub notifications: notifications::Notifications,
    /// Teleport history (navigation bar back / forward).
    pub tp_history: tphistory::TeleportHistory,
    /// World map regions, items, tracking and parcel overlays.
    pub map: worldmap::WorldMap,
    arrived_once: bool,
    /// Disk cache root (inventory cache).
    pub cache_dir: Option<std::path::PathBuf>,
}

const MAX_CHAT: usize = 500;

impl World {
    pub fn new(avatar_lib: Arc<AvatarLibrary>) -> World {
        World {
            ui_sounds: Vec::new(),
            im_messages: Vec::new(),
            balance_changes: Vec::new(),
            typing_started: Vec::new(),
            login: None,
            agent_id: Uuid::nil(),
            regions: HashMap::new(),
            main_region: None,
            objects: ObjectStore::default(),
            gltf_overrides: HashMap::new(),
            chat: VecDeque::new(),
            chat_unread: 0,
            sun: None,
            agent: AgentState::default(),
            coarse: HashMap::new(),
            appearances: HashMap::new(),
            avatar_poses: HashMap::new(),
            avatar_root_dz: HashMap::new(),
            bodies: HashMap::new(),
            look_at: lookat::LookAts::default(),
            appearance_counter: 1,
            alerts: VecDeque::new(),
            movement_complete: false,
            avatar_lib,
            teleporting: false,
            tp_progress: 0.0,
            tp_status: String::new(),
            tp_failed: false,
            outfit: Default::default(),
            diag_reported: HashSet::new(),
            events_this_frame: 0,
            balance: None,
            health: 100.0,
            social: social::Social::default(),
            mutes: mutes::MuteList::default(),
            lsl_bridge: lslbridge::LslBridge::default(),
            groups: groups::GroupChats::default(),
            status: status::Status::default(),
            inventory: inventory::Inventory::default(),
            animations: HashMap::new(),
            day_cycle: None,
            cloud_scroll: glam::Vec2::ZERO,
            parcel_name: String::new(),
            parcel: None,
            media: Default::default(),
            tp_history: tphistory::TeleportHistory::default(),
            map: worldmap::WorldMap::default(),
            notifications: notifications::Notifications::default(),
            profiles: HashMap::new(),
            profile_pending: HashSet::new(),
            profile_requested: HashSet::new(),
            arrived_once: false,
            cache_dir: None,
        }
    }

    pub fn reset(&mut self) {
        let lib = self.avatar_lib.clone();
        let chat = std::mem::take(&mut self.chat);
        let cache_dir = self.cache_dir.take();
        *self = World::new(lib);
        self.chat = chat;
        self.cache_dir = cache_dir;
    }

    pub fn main_origin(&self) -> Option<(u32, u32)> {
        self.main_region.map(aurora_net::handle_to_origin)
    }

    /// Offset of a region's origin relative to the render origin (main region).
    pub fn region_offset(&self, h: RegionHandle) -> Option<Vec3> {
        let (mx, my) = self.main_origin()?;
        let (x, y) = aurora_net::handle_to_origin(h);
        Some(Vec3::new(x as f32 - mx as f32, y as f32 - my as f32, 0.0))
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

    /// Persist the fetched inventory (next login skips unchanged folders).
    pub fn save_inventory_cache(&self) {
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

    /// LLViewerObject::interpolateLinearMotion terrain and visible-region
    /// constraints. The default Firestorm crossing fix is disabled, so known
    /// neighboring regions do not impose a separate prediction time limit.
    pub fn predict_agent(&mut self, time_dilation: f32, packet_age: f32) {
        if !self.agent.has_local_control() || !self.movement_complete {
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
        self.chat.push_back(line);
        self.chat_unread += 1;
        while self.chat.len() > MAX_CHAT {
            self.chat.pop_front();
        }
    }

    /// Ask for an avatar's profile (once).
    pub fn want_profile(&mut self, id: Uuid) {
        if !id.is_nil() && !self.profiles.contains_key(&id) && !self.profile_requested.contains(&id) {
            self.profile_pending.insert(id);
        }
    }

    /// Profiles to request now.
    pub fn take_profile_requests(&mut self) -> Vec<Uuid> {
        let v: Vec<Uuid> = self.profile_pending.drain().collect();
        self.profile_requested.extend(v.iter().copied());
        v
    }

    pub fn system_message(&mut self, text: impl Into<String>) {
        self.push_chat(ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: text.into(),
            kind: ChatKind::System,
            source: Uuid::nil(),
        });
    }

    fn ensure_region(&mut self, h: RegionHandle, size: (u32, u32)) -> &mut Region {
        self.regions.entry(h).or_insert_with(|| Region {
            handle: h,
            name: String::new(),
            info: None,
            heightmap: Heightmap::new(size.0.max(256), size.1.max(256)),
            dirty_chunks: HashSet::new(),
            caps: Arc::new(HashMap::new()),
            voice_server: String::new(),
            materials_limits: Default::default(),
        })
    }

    /// Visual parameter values (shape) of an avatar's last appearance.
    pub fn visual_params(&self, avatar: &Uuid) -> Option<&[u8]> {
        self.appearances.get(avatar).map(|(a, _)| a.visual_params.as_slice())
    }

    /// Hover height of an avatar's last appearance (m).
    pub fn appearance_hover(&self, avatar: &Uuid) -> f32 {
        self.appearances
            .get(avatar)
            .map(|(a, _)| a.hover_height)
            .filter(|h| h.is_finite())
            .unwrap_or(0.0)
            .clamp(-2.0, 2.0)
    }

    /// COF version of the last appearance received for an avatar.
    pub fn appearance_cof_version(&self, avatar: &Uuid) -> i32 {
        self.appearances.get(avatar).map(|(a, _)| a.cof_version).unwrap_or(-1)
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
            .collect()
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

    pub fn appearance_generation(&self, avatar: &Uuid) -> u64 {
        self.appearances.get(avatar).map(|(_, g)| *g).unwrap_or(0)
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
        self.events_this_frame += 1;
        match ev {
            NetEvent::LoggedIn(l) => {
                self.agent_id = l.agent_id;
                self.agent.position = Vec3::new(128.0, 128.0, 30.0);
                self.agent.set_look_at(l.look_at);
                self.social = social::Social::from_login(&l.raw);
                self.inventory = inventory::Inventory::from_login(&l.raw);
                if let Some(path) = self.inventory_cache_path() {
                    let n = self.inventory.load_cache(&path);
                    log::info!("inventory cache: {n} folders up to date");
                }
                let root = self.inventory.root;
                self.inventory.request(root);
                self.map.set_home_from_login(l.raw["home"].as_str());
                self.login = Some(l.clone());
                Some(NetEvent::LoggedIn(l))
            }
            NetEvent::MainRegionChanged { handle } => {
                let changed = self.main_region != Some(handle);
                self.main_region = Some(handle);
                self.ensure_region(handle, (256, 256));
                if changed {
                    // Region-local agent position is now relative to the new region.
                    self.movement_complete = false;
                }
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
                None
            }
            NetEvent::SimulatorFeatures {
                handle,
                voice_server_type,
                materials_rate,
                materials_max,
            } => {
                let r = self.ensure_region(handle, (256, 256));
                r.voice_server = voice_server_type;
                r.materials_limits = crate::scene::legacy_mat::RegionLimits::from_features(materials_rate, materials_max);
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
                if region_changed {
                    self.main_region = Some(handle);
                    self.ensure_region(handle, (256, 256));
                }
                self.agent.position = position;
                self.agent.set_look_at(look_at);
                self.agent.reset_motion();
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
                }
                if self.teleporting {
                    self.tp_progress = 0.6;
                    self.tp_status = "Chargement de la région…".into();
                }
                self.arrived_once = true;
                self.teleporting = false;
                Some(NetEvent::AgentMovementComplete { handle, position, look_at })
            }
            NetEvent::ObjectUpdates { handle, objects } => {
                let me = self.agent_id;
                for u in objects {
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
                    self.objects.upsert(handle, u);
                    if is_me {
                        self.agent
                            .on_server_update(pos, velocity, acceleration, parent != 0, Some(handle) == self.main_region);
                    }
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
                        if let Some(o) = self.objects.get_mut(idx) {
                            o.apply_terse(&t);
                            mine = o.full_id == me;
                            seated = o.parent_id != 0;
                        }
                        if mine {
                            self.agent
                                .on_server_update(t.position, t.velocity, t.acceleration, seated, Some(handle) == self.main_region);
                        }
                    }
                }
                None
            }
            NetEvent::ObjectsKilled { handle, local_ids } => {
                for id in local_ids {
                    let key = ObjKey {
                        region: handle,
                        local_id: id,
                    };
                    self.objects.remove(&key);
                    // a cache hit replays the cached ones, a full update comes with them
                    self.gltf_overrides.remove(&key);
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
            NetEvent::Chat(c) => {
                self.on_chat(c);
                None
            }
            NetEvent::MuteList(src) => {
                self.on_mute_list(src);
                None
            }
            ev @ (NetEvent::Groups(_)
            | NetEvent::GroupDropped(_)
            | NetEvent::SessionInvite(_)
            | NetEvent::SessionStarted { .. }
            | NetEvent::SessionAgents { .. }
            | NetEvent::SessionError { .. }
            | NetEvent::SessionClosed { .. }
            | NetEvent::ChatSessionReply { .. }) => self.on_group_event(ev),
            NetEvent::InstantMessage(im) => {
                self.on_im(im);
                None
            }
            NetEvent::Appearance(a) => {
                self.appearance_counter += 1;
                let id = a.avatar_id;
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
                self.objects.remove_region(handle);
                self.gltf_overrides.retain(|k, _| k.region != handle);
                self.regions.remove(&handle);
                self.coarse.remove(&handle);
                None
            }
            NetEvent::TeleportProgress { message } => {
                // LL teleport_strings.xml "progress" keys
                let text = match message.as_str() {
                    "sending_dest" | "sending_home" | "sending_landmark" => "Envoi vers la destination…",
                    "redirecting" | "relaying" => "Redirection vers un autre emplacement…",
                    "resolving" => "Recherche de la destination…",
                    "contacting" => "Contact avec la nouvelle région…",
                    "requesting" => "Demande de téléportation…",
                    "completing" | "completed_from" => "Fin de la téléportation…",
                    "arriving" => "Arrivée…",
                    "pending" => "En attente du serveur…",
                    _ => "Téléportation en cours…",
                };
                self.teleporting = true;
                self.tp_progress = (self.tp_progress + 0.06).clamp(0.15, 0.42);
                self.tp_status = text.into();
                None
            }
            NetEvent::TeleportFinished => {
                self.tp_progress = self.tp_progress.max(0.5);
                self.tp_status = "Connexion à la région…".into();
                None
            }
            NetEvent::TeleportLocal { flying } => {
                self.agent.flying = flying;
                None
            }
            NetEvent::TeleportStarted => {
                if !self.teleporting {
                    log::info!("TeleportStart (arrived {})", self.arrived_once);
                    // a teleport we did not ask for (lure, llTeleportAgent):
                    // process_teleport_start plays it too
                    self.ui_sounds.push(UiSound::TeleportOut);
                }
                self.teleporting = true;
                self.tp_failed = false;
                self.tp_progress = self.tp_progress.max(0.10);
                self.tp_status = "Téléportation acceptée…".into();
                self.system_message("Téléportation en cours…");
                None
            }
            NetEvent::TeleportFailed { reason } => {
                self.tp_history.failed();
                self.teleporting = false;
                self.tp_failed = true;
                self.system_message(format!("Échec de la téléportation : {reason}"));
                None
            }
            NetEvent::Alert { message } => {
                self.map.on_alert(&message);
                let restart = message.to_lowercase().contains("restart");
                self.ui_sounds.push(if restart { UiSound::Restart } else { UiSound::Alert });
                self.system_message(message.clone());
                self.notifications.push(
                    notifications::Kind::Alert,
                    "Alerte du simulateur",
                    message.clone(),
                    notifications::Data::None,
                );
                self.alerts.push_back(message);
                None
            }
            NetEvent::ScriptDialog {
                object_id, object_name, ..
            } if self.object_blocked(&object_id, &object_name) => None,
            NetEvent::ScriptQuestion { task_id, object_name, .. } if self.object_blocked(&task_id, &object_name) => None,
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
            } => {
                // process_script_question
                self.ui_sounds.push(UiSound::ScriptFloaterOpen);
                let lines = notifications::permission_lines(questions);
                let body = format!(
                    "L'objet « {object_name} » de {owner_name} demande l'autorisation de :\n• {}",
                    lines.join("\n• ")
                );
                self.notifications.push(
                    notifications::Kind::Permissions,
                    "Autorisations demandées",
                    body,
                    notifications::Data::Permissions {
                        task: task_id,
                        item: item_id,
                        questions,
                    },
                );
                None
            }
            NetEvent::LoadUrl {
                object_name, message, url, ..
            } => {
                let body = format!("{message}\n{url}");
                self.notifications.push(
                    notifications::Kind::Url,
                    format!("« {object_name} » propose un lien"),
                    body,
                    notifications::Data::Url(url),
                );
                None
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
            NetEvent::AvatarAnimations { avatar, anims } => {
                let now = Instant::now();
                if avatar == self.agent_id {
                    log::info!("own animations: {} playing", anims.len());
                    self.agent.ground_sit = anims.iter().any(|(id, _)| *id == body::ANIM_SIT_GROUND_CONSTRAINED);
                }
                let prev = self.animations.remove(&avatar).unwrap_or_default();
                // ANIM_AGENT_TYPE starting: the typing sound at the avatar
                const ANIM_AGENT_TYPE: Uuid = Uuid::from_u128(0xc541c47f_e0c0_058b_ad1a_d6ae3a4584d9);
                if anims.iter().any(|(id, _)| *id == ANIM_AGENT_TYPE) && !prev.iter().any(|p| p.id == ANIM_AGENT_TYPE) {
                    self.typing_started.push(avatar);
                }
                let list = anims
                    .into_iter()
                    .map(|(id, seq)| PlayingAnimation::from_signal(id, seq, now, prev.iter().find(|p| p.id == id)))
                    .collect();
                self.animations.insert(avatar, list);
                None
            }
            NetEvent::Environment { handle, environment } => {
                match eep::DayCycle::from_llsd(&environment) {
                    Some(d) => {
                        log::info!(
                            "EEP day cycle (parcel {}): {} sky keys, {} water keys, day {} s",
                            d.parcel_id,
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
                        self.day_cycle = Some((handle, d));
                    }
                    None => {
                        log::info!("environment has no sky track; using the default sky");
                        self.day_cycle = None;
                    }
                }
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
            NetEvent::RegionFlags { handle, flags } => {
                if let Some(r) = self.regions.get_mut(&handle)
                    && let Some(info) = r.info.as_mut()
                    && info.region_flags != flags
                {
                    Arc::make_mut(info).region_flags = flags;
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
                None
            }
            NetEvent::AvatarProfile {
                id,
                image_id,
                about,
                born_on,
            } => {
                self.profiles.insert(id, Profile { image_id, about, born_on });
                None
            }
            NetEvent::Balance(b) => {
                // the app compares with UISndMoneyChangeThreshold
                self.balance_changes.push((self.balance, b));
                self.balance = Some(b);
                None
            }
            NetEvent::InventoryContents(c) => {
                self.inventory.apply(c);
                None
            }
            NetEvent::InventoryFetchFailed { folders } => {
                self.inventory.failed(&folders);
                None
            }
            NetEvent::InventoryItems(items) => {
                self.inventory.add_items(items);
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
            NetEvent::MapBlocks { blocks, .. } => {
                self.map.apply_blocks(blocks);
                None
            }
            NetEvent::MapItems { item_type, items } => {
                self.map.apply_items(item_type, items);
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
        let kind = match c.source_type {
            ChatSourceType::System => ChatKind::System,
            ChatSourceType::Object => ChatKind::Object(c.chat_type),
            ChatSourceType::Agent => {
                if c.source_id == self.agent_id {
                    ChatKind::Own
                } else {
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
        });
    }

    fn on_im(&mut self, im: InstantMessage) {
        if self.im_blocked(&im) || self.on_session_im(&im) || self.on_status_im(&im) || self.on_im_offer(&im) {
            return;
        }
        // decided before the message opens its conversation
        let autoresponse = self.autoresponse_due(&im);
        // 0 = message from user, 19 = message from object; others are notices etc.
        if im.message.is_empty() {
            return;
        }
        if im.dialog == 0 && !im.from_agent_id.is_nil() {
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
            let s = self.social.session_mut(im.from_agent_id);
            s.lines.push(ChatLine {
                time: SystemTime::now(),
                from,
                text: im.message.clone(),
                kind: ChatKind::Im,
                source: im.from_agent_id,
            });
            s.unread += 1;
            if s.lines.len() > MAX_CHAT {
                s.lines.remove(0);
            }
            self.send_autoresponse(&im, autoresponse);
            return;
        }
        // dialog 19 = MessageFromObject
        let kind = if im.dialog == 19 { ChatKind::ObjectIm } else { ChatKind::Im };
        let from = if im.from_name.is_empty() {
            "Système".to_owned()
        } else {
            im.from_name
        };
        self.push_chat(ChatLine {
            time: SystemTime::now(),
            from,
            text: im.message,
            kind,
            source: im.from_agent_id,
        });
    }

    /// IM dialogs that are offers or notices go to the notification center.
    /// Returns true when handled.
    fn on_im_offer(&mut self, im: &InstantMessage) -> bool {
        use notifications::{Data, Kind, im as d};
        let from = if im.from_name.is_empty() {
            "Quelqu'un".to_owned()
        } else {
            im.from_name.clone()
        };
        let n = &mut self.notifications;
        match im.dialog {
            d::LURE_USER => {
                self.ui_sounds.push(UiSound::TeleportOffer);
                n.push(
                    Kind::Teleport,
                    format!("{from} vous propose une téléportation"),
                    im.message.clone(),
                    Data::Lure {
                        from: im.from_agent_id,
                        lure: im.session_id,
                    },
                );
            }
            d::FRIENDSHIP_OFFERED => {
                self.ui_sounds.push(UiSound::FriendshipOffer);
                n.push(
                    Kind::Friendship,
                    format!("{from} vous propose son amitié"),
                    im.message.clone(),
                    Data::Friend { tx: im.session_id },
                );
            }
            d::INVENTORY_OFFERED | d::TASK_INVENTORY_OFFERED => {
                self.ui_sounds.push(UiSound::InventoryOffer);
                let asset_type = im.binary_bucket.first().map(|b| *b as i8).unwrap_or(-1);
                let what = if im.message.is_empty() {
                    "un objet".to_owned()
                } else {
                    format!("« {} »", im.message)
                };
                n.push(
                    Kind::Inventory,
                    format!("{from} vous donne {what}"),
                    String::new(),
                    Data::Inventory {
                        from: im.from_agent_id,
                        tx: im.session_id,
                        asset_type,
                        task: im.dialog == d::TASK_INVENTORY_OFFERED,
                    },
                );
            }
            d::GROUP_INVITATION => {
                self.ui_sounds.push(UiSound::GroupInvitation);
                n.push(
                    Kind::Group,
                    "Invitation à rejoindre un groupe",
                    im.message.clone(),
                    Data::Group {
                        group: im.from_agent_id,
                        tx: im.session_id,
                    },
                );
            }
            d::GROUP_NOTICE => {
                self.ui_sounds.push(UiSound::GroupNotice);
                // "subject|message"
                let (subject, body) = im.message.split_once('|').unwrap_or(("Avis de groupe", im.message.as_str()));
                n.push(Kind::Group, format!("{from} : {subject}"), body.to_owned(), Data::None);
            }
            d::LURE_ACCEPTED => {
                n.push(
                    Kind::Info,
                    format!("{from} a accepté votre téléportation"),
                    String::new(),
                    Data::None,
                );
            }
            d::LURE_DECLINED => {
                n.push(
                    Kind::Info,
                    format!("{from} a refusé votre téléportation"),
                    im.message.clone(),
                    Data::None,
                );
            }
            d::FRIENDSHIP_ACCEPTED => {
                n.push(
                    Kind::Friendship,
                    format!("{from} a accepté votre amitié"),
                    String::new(),
                    Data::None,
                );
            }
            d::INVENTORY_ACCEPTED | d::INVENTORY_DECLINED => {
                let verb = if im.dialog == d::INVENTORY_ACCEPTED {
                    "accepté"
                } else {
                    "refusé"
                };
                n.push(Kind::Inventory, format!("{from} a {verb} votre objet"), String::new(), Data::None);
            }
            d::MESSAGEBOX | d::FROM_TASK_AS_ALERT => {
                n.push(Kind::Alert, from, im.message.clone(), Data::None);
            }
            d::GOTO_URL => {
                let url = String::from_utf8_lossy(&im.binary_bucket).trim_end_matches('\0').to_owned();
                n.push(Kind::Url, from, im.message.clone(), Data::Url(url));
            }
            _ => return false,
        }
        true
    }

    /// Answer a notification: messages to send and a URL to open.
    pub fn respond_notification(&mut self, id: u64, r: notifications::Response) -> (Vec<aurora_net::NetCommand>, Option<String>) {
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
        self.notifications.respond(
            id,
            r,
            |t| folders.iter().find(|(ft, _)| *ft == t).map(|(_, f)| *f).unwrap_or(root),
            calling,
        )
    }

    /// Name of an avatar by id (from its object name-values).
    pub fn avatar_name(&self, id: &Uuid) -> Option<String> {
        let idx = self.objects.index_of_uuid(id)?;
        self.objects.get(idx)?.display_name()
    }

    /// Friends' online / offline notices once their names are known
    /// (LLAvatarNameCache::get with a callback).
    pub fn flush_online_notices(&mut self) {
        for (n, online) in self.social.take_online_notices() {
            // FIRE-2731 (on_avatar_name_cache_notify)
            self.ui_sounds
                .push(if online { UiSound::FriendOnline } else { UiSound::FriendOffline });
            self.system_message(n);
        }
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
        let s = self.social.session_mut(to);
        s.lines.push(ChatLine {
            time: SystemTime::now(),
            from: me,
            text: text.to_owned(),
            kind: ChatKind::Own,
            source: Uuid::nil(),
        });
    }

    /// Turn the drawn bodies of the standing avatars toward where they face
    /// or travel (LLVOAvatar::updateRootPositionAndRotation). Returns our
    /// own avatar's turn (+1 left, -1 right, 0) for the control flags.
    pub fn update_bodies(&mut self, now: Instant, dt: f32, cam_at: Vec3, mouselook: bool) -> i8 {
        let mut own_turn = 0;
        let mut seen = std::collections::HashSet::new();
        for (_, o) in self.objects.iter() {
            if !o.is_avatar() || o.parent_id != 0 {
                continue;
            }
            let id = o.full_id;
            let is_self = id == self.agent_id && self.agent.has_local_control();
            let input = if is_self {
                let a = &self.agent;
                body::Input {
                    prim_dir: a.forward(),
                    velocity: a.velocity,
                    in_air: a.flying || a.velocity.z.abs() > 1.0,
                    mouselook_at: mouselook.then_some(cam_at),
                    flying: a.flying,
                }
            } else {
                body::Input {
                    prim_dir: o.predicted(now).1 * Vec3::X,
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

    pub fn animations_of(&self, owner: &Uuid) -> &[PlayingAnimation] {
        self.animations.get(owner).map(|v| v.as_slice()).unwrap_or(&[])
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
            avatar,
            anims: vec![(id, 1)],
        });
        // Deterministic earlier clock: no sleeps or dependence on timer resolution.
        let early = Instant::now() - std::time::Duration::from_secs(1);
        let previous = &mut world.animations.get_mut(&avatar).expect("signal")[0];
        previous.sequence_start = early;
        previous.continuous_start = early;
        world.apply(NetEvent::AvatarAnimations {
            avatar,
            anims: vec![(id, 2)],
        });
        let changed = world.animations_of(&avatar)[0];
        assert_eq!(changed.continuous_start, early);
        assert!(changed.sequence_start > early);
        world.apply(NetEvent::AvatarAnimations { avatar, anims: Vec::new() });
        world.apply(NetEvent::AvatarAnimations {
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

    #[test]
    fn own_ground_sit_animation_means_sitting() {
        let (mut world, _, _) = fixture();
        let me = world.agent_id;
        let sit = |avatar, anim| NetEvent::AvatarAnimations {
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
}
