//! Parcel media and media on a prim (shared media), as in Firestorm:
//!
//! * LLViewerParcelMedia / LLViewerParcelMediaAutoPlay: the parcel's media
//!   replaces its placeholder texture (`MediaID`) on every face using it;
//!   played from the top bar button, automatically after 5 s in the parcel
//!   when standing still and the texture is visible (ParcelMediaAutoPlayEnable),
//!   driven by ParcelMediaCommandMessage / ParcelMediaUpdate.
//! * LLVOVolume media data: faces flagged with media get their LLMediaEntry
//!   from the ObjectMedia capability when the object's "x-mv:" version
//!   changes; the media texture replaces the face's diffuse texture
//!   ("let the prim media win" over parcel media).
//! * LLViewerMedia::updateMedia: interest (pixels on screen), priorities and
//!   instance limits (PluginInstancesTotal / Normal / Low / CPULimit), one
//!   new plugin per second, volume with the distance roll-off.
//! * LLViewerMediaFocus: click a media face to focus it; mouse, wheel and
//!   keyboard then go to the page; Escape or a click elsewhere releases it.
//!
//! Plugins run out of process through `aurora-media` (SLPlugin + the
//! CEF / LibVLC plugins of an installed Firestorm, or our own `llplugin`).
//!
//! Derived from indra/newview (llviewermedia.cpp, llviewerparcelmedia.cpp,
//! llviewerparcelmediaautoplay.cpp, llviewermediafocus.cpp, llvovolume.cpp),
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project,
//! originally LGPL 2.1.

pub mod entry;
mod interact;
pub mod keys;
pub mod openid;
pub mod pick;
mod update;

use crate::scene::textures::TextureStreamer;
use crate::scene::{CullView, Scene};
use crate::world::World;
use aurora_llsd::{Llsd, Map};
use aurora_media::{BrowserSettings, MediaEvent, MediaPlugin, MediaStatus, Modifiers, MouseEvent, PluginPaths, Priority};
use aurora_render::{MipLevel, Renderer};
use entry::{MediaDataClient, MediaEntry, ObjectMediaData};
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// PARCEL_MEDIA_COMMAND_* (indra_constants.h).
pub mod command {
    pub const STOP: u32 = 0;
    pub const PAUSE: u32 = 1;
    pub const PLAY: u32 = 2;
    pub const LOOP: u32 = 3;
    pub const TIME: u32 = 6;
    pub const UNLOAD: u32 = 8;
}

/// "none/none": LLMIMETypes::getDefaultMimeType, never played.
const NONE_MIME: &str = "none/none";
/// LLViewerParcelMediaAutoPlay: seconds in the parcel, agent speed, pixels.
const AUTOPLAY_TIME: Duration = Duration::from_secs(5);
const AUTOPLAY_SPEED: f32 = 0.1;
const AUTOPLAY_SIZE: f32 = 24.0 * 24.0;
/// LLVIEWERMEDIA_CREATE_DELAY
const CREATE_DELAY: Duration = Duration::from_secs(1);
/// Settings without UI (settings.xml defaults).
const ROLLOFF_MIN: f32 = 40.0;
const ROLLOFF_MAX: f32 = 80.0;
const ROLLOFF_RATE: f32 = 0.02;
const INSTANCES_NORMAL: usize = 2;
const INSTANCES_LOW: usize = 4;
const INSTANCES_CPU_LIMIT: f64 = 0.9;
/// A freed media texture slot is released after the faces stopped using it.
const FREE_DELAY: Duration = Duration::from_secs(2);

/// Texture overrides read by the scene when it builds faces, and the parcel
/// media messages waiting for the media manager (kept in the world).
#[derive(Default)]
pub struct WorldMedia {
    /// Parcel media: placeholder texture → media texture.
    by_texture: HashMap<Uuid, Uuid>,
    /// Media on a prim: (object, face) → media texture.
    by_face: HashMap<(Uuid, u8), Uuid>,
    pub commands: Vec<ParcelMediaMsg>,
}

impl WorldMedia {
    /// The media texture shown on a face instead of `texture`, if any
    /// (prim media first, then parcel media).
    #[inline]
    pub fn face_texture(&self, object: &Uuid, face: u8, texture: &Uuid) -> Option<Uuid> {
        if self.by_face.is_empty() && self.by_texture.is_empty() {
            return None;
        }
        self.by_face.get(&(*object, face)).or_else(|| self.by_texture.get(texture)).copied()
    }
}

#[derive(Debug, Clone)]
pub enum ParcelMediaMsg {
    Command {
        flags: u32,
        command: u32,
        time: f32,
    },
    Update {
        url: String,
        media_id: Uuid,
        auto_scale: bool,
        mime: String,
        width: i32,
        height: i32,
        looping: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKey {
    Parcel,
    Prim { object: Uuid, face: u8 },
}

/// The GPU texture of a media.
struct MediaTexture {
    /// Media texture id (a local texture of the streamer).
    id: Uuid,
    slot: u32,
    size: (u32, u32),
    /// Holds a frame: faces may show it.
    ready: bool,
}

pub struct MediaImpl {
    pub key: MediaKey,
    /// URL asked for (mMediaURL).
    pub url: String,
    /// Type; empty until discovered.
    pub mime: String,
    probe: Option<crossbeam_channel::Receiver<String>>,
    plugin: Option<MediaPlugin>,
    pub width: i32,
    pub height: i32,
    pub auto_scale: bool,
    pub looping: bool,
    /// Started by the user, autoplay or a script; otherwise unloaded.
    pub wanted: bool,
    pub paused: bool,
    pub failed: Option<String>,
    texture: MediaTexture,
    pub interest: f32,
    pub distance: f32,
    pub priority: Priority,
    /// Own volume (media controls), 0..1.
    pub volume: f32,
    pub muted: bool,
    /// Prim media: entry this impl was made from.
    pub entry: Option<MediaEntry>,
    /// Parcel media: placeholder texture.
    pub media_id: Uuid,
    pub status: MediaStatus,
    /// Page location reported by the plugin.
    pub location: String,
    pub title: String,
    pub can_back: bool,
    pub can_forward: bool,
    pub current_time: f64,
    pub duration: f64,
    pub cursor: String,
    nav_pending: bool,
    mouse_down: bool,
    /// The user navigated (clicks, URL bar): report it to the server.
    user_nav: bool,
}

impl MediaImpl {
    fn new(key: MediaKey, url: String, mime: String) -> MediaImpl {
        MediaImpl {
            key,
            url,
            mime,
            probe: None,
            plugin: None,
            width: 0,
            height: 0,
            auto_scale: false,
            looping: false,
            wanted: false,
            paused: false,
            failed: None,
            texture: MediaTexture {
                id: Uuid::new_v4(),
                slot: 0,
                size: (0, 0),
                ready: false,
            },
            interest: 0.0,
            distance: 0.0,
            priority: Priority::Unloaded,
            volume: 1.0,
            muted: false,
            entry: None,
            media_id: Uuid::nil(),
            status: MediaStatus::None,
            location: String::new(),
            title: String::new(),
            can_back: false,
            can_forward: false,
            current_time: 0.0,
            duration: 0.0,
            cursor: String::new(),
            nav_pending: true,
            mouse_down: false,
            user_nav: false,
        }
    }

    /// (media size, texture size) of a loaded media (« Aligner » of the
    /// build floater's Media tab).
    pub fn sizes(&self) -> Option<((i32, i32), (i32, i32))> {
        let p = self.plugin.as_ref()?;
        Some((p.media_size(), p.texture_size()))
    }

    pub fn is_loaded(&self) -> bool {
        self.plugin.is_some()
    }

    /// Time-based media (video / audio through LibVLC): pause / play.
    pub fn is_time_media(&self) -> bool {
        self.plugin.as_ref().is_some_and(|p| p.plugin_name == "media_plugin_libvlc")
    }

    /// navigateTo: a new URL; the plugin is kept when its type still fits.
    pub fn navigate(&mut self, url: &str) {
        let url = url.trim();
        if url != self.url {
            self.location.clear();
        }
        self.url = url.to_string();
        self.failed = None;
        self.nav_pending = true;
        self.mime.clear();
        self.probe = None;
    }

    pub fn back(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_back();
            self.user_nav = true;
        }
    }

    pub fn forward(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_forward();
            self.user_nav = true;
        }
    }

    pub fn reload(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_reload(true);
        }
    }

    pub fn stop_loading(&mut self) {
        if let Some(p) = self.plugin.as_mut() {
            p.browse_stop();
        }
    }

    pub fn play(&mut self) {
        self.wanted = true;
        if self.paused {
            self.paused = false;
            if let Some(p) = self.plugin.as_mut() {
                p.start(1.0);
            }
        }
    }

    pub fn pause(&mut self) {
        self.paused = true;
        if let Some(p) = self.plugin.as_mut() {
            p.pause();
        }
    }

    pub fn seek(&mut self, t: f64) {
        if let Some(p) = self.plugin.as_mut() {
            p.seek(t);
        }
    }

    /// Home page of prim media.
    pub fn home(&mut self) {
        if let Some(home) = self.entry.as_ref().map(|e| e.home_url.trim().to_string()).filter(|h| !h.is_empty()) {
            self.navigate(&home);
            self.user_nav = true;
        }
    }

    /// The user typed a URL in the media controls.
    pub fn navigate_user(&mut self, url: &str) {
        let mut url = url.trim().to_string();
        if !url.contains("://") && !url.starts_with("data:") && !url.starts_with("about:") {
            url = format!("http://{url}");
        }
        self.navigate(&url);
        self.user_nav = true;
    }
}

/// Media settings (Firestorm defaults).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct MediaSettings {
    /// AudioStreamingMedia: media allowed at all.
    pub enabled: bool,
    /// PrimMediaMasterEnabled: media on a prim.
    pub prim_media: bool,
    /// MediaShowOnOthers: media on other avatars' attachments.
    pub show_on_others: bool,
    /// PluginInstancesTotal.
    pub max_instances: usize,
    /// PermAllowScriptedMedia: let scripts start the parcel media.
    pub allow_scripted: bool,
    /// MediaFirstClickInteract: the first click already goes to the page
    /// when the face allows it (first_click_interact).
    pub first_click_interact: bool,
    /// MediaAutoPlayHuds: media on HUDs plays by itself.
    pub autoplay_huds: bool,
    /// Folder with SLPlugin and llplugin\ (empty: automatic).
    pub plugin_dir: String,
}

impl Default for MediaSettings {
    fn default() -> Self {
        MediaSettings {
            enabled: true,
            prim_media: true,
            show_on_others: false,
            max_instances: 8,
            allow_scripted: false,
            first_click_interact: true,
            autoplay_huds: true,
            plugin_dir: String::new(),
        }
    }
}

/// Per-object media data and the versions seen / fetched.
#[derive(Default)]
struct ObjMedia {
    idx: usize,
    fetched_version: Option<u32>,
    data: Option<ObjectMediaData>,
    /// Still present at the last scan.
    alive: bool,
    hud: bool,
    on_other_avatar: bool,
    owner: Uuid,
    /// Agent of the last media change ("x-mv:<version>/<agent>").
    changed_by: Uuid,
}

/// Inputs of one update.
pub struct MediaFrame<'a> {
    pub view: &'a CullView,
    pub camera: Vec3,
    pub settings: &'a MediaSettings,
    /// ParcelMediaAutoPlayEnable (and isAutoPlayable for prim media).
    pub autoplay: bool,
    /// Master × media channel volume (0 when muted / disabled).
    pub channel_volume: f32,
    pub agent_speed: f32,
    pub window_focused: bool,
    pub net: Option<(&'a tokio::runtime::Handle, reqwest::Client)>,
    pub cache_dir: &'a std::path::Path,
    pub language: &'a str,
}

pub struct MediaManager {
    /// Picking keeps CLICK_ACTION_IGNORE selectable while tools are open.
    pub build_mode: bool,
    paths: Option<PluginPaths>,
    paths_for: Option<String>,
    pub impls: Vec<MediaImpl>,
    objects: HashMap<Uuid, ObjMedia>,
    client: MediaDataClient,
    last_create: Instant,
    pub focus: Option<MediaKey>,
    autoplay_since: Instant,
    autoplay_done: bool,
    /// MediaTentativeAutoPlay: cleared by "stop all", set again on teleport.
    pub tentative_autoplay: bool,
    last_scan: Instant,
    /// Objects showing the parcel placeholder texture (index), and their
    /// bounds this frame (interest of the parcel media).
    parcel_objects: Vec<usize>,
    parcel_bounds: Vec<(Vec3, f32)>,
    pending_free: Vec<(u32, Uuid, Instant)>,
    scratch: Vec<u8>,
    last_parcel_key: Option<(Option<u64>, i32)>,
    /// Offline demo: media entries without the capability.
    pub local_entries: HashMap<Uuid, ObjectMediaData>,
    nav_tx: crossbeam_channel::Sender<(Uuid, u8, Result<(), String>)>,
    nav_rx: crossbeam_channel::Receiver<(Uuid, u8, Result<(), String>)>,
    /// Messages for the chat / notifications (drained by the app).
    pub messages: Vec<String>,
    /// Offline demo: autoplay regardless of the settings.
    force_autoplay: bool,
    /// Periodic state summary in the log.
    last_diag: Instant,
    /// Picking geometry of media objects (shape key, faces).
    pick_cache: HashMap<Uuid, (u64, std::sync::Arc<Vec<Option<interact::CpuFace>>>)>,
    /// Last media pixel under the cursor (focused media).
    last_pixel: Option<(i32, i32)>,
    /// OpenID cookie of the web profiles (login token).
    pub openid: openid::OpenId,
}

impl Default for MediaManager {
    fn default() -> Self {
        let (nav_tx, nav_rx) = crossbeam_channel::unbounded();
        MediaManager {
            build_mode: false,
            paths: None,
            paths_for: None,
            impls: Vec::new(),
            objects: HashMap::new(),
            client: MediaDataClient::default(),
            last_create: Instant::now() - CREATE_DELAY,
            focus: None,
            autoplay_since: Instant::now(),
            autoplay_done: false,
            tentative_autoplay: true,
            last_scan: Instant::now() - Duration::from_secs(10),
            parcel_objects: Vec::new(),
            parcel_bounds: Vec::new(),
            pending_free: Vec::new(),
            scratch: Vec::new(),
            last_parcel_key: None,
            local_entries: HashMap::new(),
            nav_tx,
            nav_rx,
            messages: Vec::new(),
            force_autoplay: false,
            last_diag: Instant::now() - Duration::from_secs(15),
            pick_cache: HashMap::new(),
            last_pixel: None,
            openid: openid::OpenId::default(),
        }
    }
}

/// Settings of our CEF browsers (newSourceFromMediaType): cache, log,
/// language and the viewer's user agent.
pub fn browser_settings(cache_dir: &std::path::Path, language: &str) -> BrowserSettings {
    BrowserSettings {
        cache_path: format!("{}\\", cache_dir.join("cef").display()),
        username: "aurora".into(),
        cef_log_file: cache_dir.join("cef.log").display().to_string(),
        language: language.to_string(),
        user_agent: concat!("SecondLife (Aurora Viewer ", env!("CARGO_PKG_VERSION"), ")").to_string(),
        cookies_enabled: true,
        javascript_enabled: true,
        zoom_factor: 1.0,
    }
}

fn effective_parcel_url(pm: &aurora_net::ParcelMedia, url: &str) -> String {
    // LLViewerParcelMedia::update: shared HTML media uses its current URL
    if !pm.current_url.trim().is_empty() && pm.mime == "text/html" {
        pm.current_url.trim().to_string()
    } else {
        url.trim().to_string()
    }
}

fn sanitize(url: &str) -> String {
    // LLViewerMediaImpl logs without the query
    let u = url.split('?').next().unwrap_or(url);
    if u.len() > 120 {
        format!("{}…", &u[..u.char_indices().nth(120).map(|(i, _)| i).unwrap_or(u.len())])
    } else {
        u.to_string()
    }
}

impl MediaManager {
    /// SLPlugin / llplugin (searched once per configured folder).
    pub fn plugins(&mut self, settings: &MediaSettings) -> Option<&PluginPaths> {
        if self.paths_for.as_deref() != Some(settings.plugin_dir.as_str()) {
            let dir = settings.plugin_dir.trim();
            self.paths = PluginPaths::discover((!dir.is_empty()).then(|| std::path::Path::new(dir)));
            self.paths_for = Some(settings.plugin_dir.clone());
            match &self.paths {
                Some(p) => log::info!("media: plugins in {} (launcher {})", p.plugin_dir.display(), p.launcher.display()),
                None => log::warn!("media: no SLPlugin / llplugin folder found: web and video media disabled"),
            }
        }
        self.paths.as_ref()
    }

    pub fn plugins_found(&self) -> Option<&PluginPaths> {
        self.paths.as_ref()
    }

    pub fn find(&self, key: &MediaKey) -> Option<&MediaImpl> {
        self.impls.iter().find(|m| m.key == *key)
    }

    pub fn find_mut(&mut self, key: &MediaKey) -> Option<&mut MediaImpl> {
        self.impls.iter_mut().find(|m| m.key == *key)
    }

    pub fn focused(&self) -> Option<&MediaImpl> {
        self.focus.and_then(|k| self.find(&k))
    }

    pub fn focused_mut(&mut self) -> Option<&mut MediaImpl> {
        let k = self.focus?;
        self.find_mut(&k)
    }

    /// Parcel media started (button state).
    pub fn parcel_playing(&self) -> bool {
        self.find(&MediaKey::Parcel).is_some_and(|m| m.wanted && !m.paused)
    }

    pub fn parcel_status_playing(&self) -> bool {
        self.find(&MediaKey::Parcel).is_some_and(|m| m.status == MediaStatus::Playing)
    }

    /// LLToolPie::handle_click_action_play: pause, resume, or start parcel media.
    pub fn toggle_parcel(&mut self, world: &World, settings: &MediaSettings) {
        match self.find(&MediaKey::Parcel).map(|m| m.status) {
            Some(MediaStatus::Playing) => {
                if let Some(m) = self.find_mut(&MediaKey::Parcel) {
                    m.pause();
                }
            }
            Some(MediaStatus::Paused) => {
                if let Some(m) = self.find_mut(&MediaKey::Parcel) {
                    m.play();
                }
            }
            _ => self.play_parcel(world, settings),
        }
    }

    /// LLToolPie::handle_click_action_open_media: a face already displaying
    /// a media texture toggles parcel playback; otherwise open its web URL.
    pub fn click_open_media(&mut self, world: &World, idx: usize, face: i32, settings: &MediaSettings) -> Option<String> {
        let parcel = world.parcel.as_ref()?;
        let object = world.objects.get(idx)?;
        if face < 0 {
            return None;
        }
        let texture = object.te.as_ref()?.face(face as usize).texture;
        if self.impls.iter().any(|m| {
            (!m.media_id.is_nil() && m.media_id == texture)
                || m.texture.id == texture
                || m.key
                    == MediaKey::Prim {
                        object: object.full_id,
                        face: face as u8,
                    }
        }) {
            self.toggle_parcel(world, settings);
            None
        } else {
            let url = parcel.media_url.trim();
            // The platform opener handles web URLs, as with llLoadURL.
            (url.starts_with("http://") || url.starts_with("https://")).then(|| url.to_owned())
        }
    }

    /// Loaded plugins.
    pub fn loaded_count(&self) -> usize {
        self.impls.iter().filter(|m| m.is_loaded()).count()
    }

    /// Everything off (logout).
    pub fn clear(&mut self) {
        let keys: Vec<MediaKey> = self.impls.iter().map(|m| m.key).collect();
        for k in keys {
            self.remove(&k);
        }
        self.objects.clear();
        self.client.clear();
        self.focus = None;
        self.last_parcel_key = None;
    }

    /// Media data known for an object (fetched with ObjectMedia, or local
    /// in the demo): the build floater's Media tab reads it.
    pub fn object_media(&self, object: &Uuid) -> Option<&ObjectMediaData> {
        self.objects
            .get(object)
            .and_then(|o| o.data.as_ref())
            .or_else(|| self.local_entries.get(object))
    }

    /// Our own ObjectMedia UPDATE: show it at once (the simulator's new
    /// media version brings the definitive data); in the demo it is the data.
    pub fn set_object_media(&mut self, object: Uuid, data: ObjectMediaData, demo: bool) {
        if let Some(o) = self.objects.get_mut(&object) {
            o.data = Some(data.clone());
        }
        if demo {
            self.local_entries.insert(object, data);
        }
    }

    /// Offline demo: media on a face without the capability, autoplaying.
    pub fn demo_entry(&mut self, object: Uuid, face: u8, url: &str) {
        let entry = MediaEntry {
            current_url: url.to_string(),
            home_url: url.to_string(),
            auto_play: true,
            first_click_interact: true,
            width_pixels: 1024,
            height_pixels: 576,
            ..Default::default()
        };
        let mut faces = vec![None; face as usize + 1];
        faces[face as usize] = Some(entry);
        log::info!("media: demo media on {object} face {face}");
        self.local_entries.insert(object, ObjectMediaData { version: 1, faces });
        self.force_autoplay = true;
    }

    /// A teleport: MediaTentativeAutoPlay is set again.
    pub fn on_teleport(&mut self) {
        self.tentative_autoplay = true;
    }

    // -------------------------------------------------------------- parcel

    /// LLViewerParcelMedia::play.
    pub fn play_parcel(&mut self, world: &World, settings: &MediaSettings) {
        if !settings.enabled {
            return;
        }
        let Some(parcel) = world.parcel.as_ref() else {
            return;
        };
        let pm = &parcel.media;
        let url = effective_parcel_url(pm, &parcel.media_url);
        if url.is_empty() || pm.mime.eq_ignore_ascii_case(NONE_MIME) {
            return;
        }
        if let Some(m) = self.find_mut(&MediaKey::Parcel)
            && m.url == url
            && m.media_id == pm.media_id
        {
            m.play();
            return;
        }
        // another texture or URL: a new impl (the old one must not fight
        // over the texture)
        self.remove(&MediaKey::Parcel);
        // a type with a plugin is used as is (navigateInternal)
        let mime = if pm.mime.trim().is_empty() {
            String::new()
        } else {
            pm.mime.trim().to_ascii_lowercase()
        };
        let mut m = MediaImpl::new(MediaKey::Parcel, url, mime);
        m.width = pm.width;
        m.height = pm.height;
        m.auto_scale = pm.auto_scale;
        m.looping = pm.looping;
        m.media_id = pm.media_id;
        m.wanted = true;
        log::info!("media: parcel media {} ({})", sanitize(&m.url), m.mime);
        self.impls.push(m);
        self.autoplay_done = true;
    }

    pub fn stop_parcel(&mut self) {
        self.remove(&MediaKey::Parcel);
    }

    fn remove(&mut self, key: &MediaKey) {
        if let Some(i) = self.impls.iter().position(|m| m.key == *key) {
            let m = self.impls.remove(i);
            if m.texture.slot != 0 {
                self.pending_free.push((m.texture.slot, m.texture.id, Instant::now()));
            }
            if self.focus == Some(*key) {
                self.focus = None;
            }
            // dropping the plugin shuts it down
        }
    }

    /// ParcelMediaCommandMessage / ParcelMediaUpdate
    /// (processParcelMediaCommandMessage / processParcelMediaUpdate).
    fn on_parcel_messages(&mut self, world: &mut World, settings: &MediaSettings) {
        let msgs: Vec<ParcelMediaMsg> = std::mem::take(&mut world.media.commands);
        for msg in msgs {
            match msg {
                ParcelMediaMsg::Command { flags, command, time } => {
                    let transport =
                        (1 << command::STOP) | (1 << command::PAUSE) | (1 << command::PLAY) | (1 << command::LOOP) | (1 << command::UNLOAD);
                    if flags & transport != 0 {
                        match command {
                            command::STOP | command::UNLOAD => self.stop_parcel(),
                            command::PAUSE => {
                                if let Some(m) = self.find_mut(&MediaKey::Parcel) {
                                    m.pause();
                                }
                            }
                            command::PLAY | command::LOOP => {
                                let paused = self.find(&MediaKey::Parcel).is_some_and(|m| m.paused);
                                if paused {
                                    if let Some(m) = self.find_mut(&MediaKey::Parcel) {
                                        m.play();
                                    }
                                } else if settings.allow_scripted {
                                    self.play_parcel(world, settings);
                                } else {
                                    log::info!("media: scripted parcel media not allowed (Préférences › Son et voix)");
                                }
                            }
                            _ => {}
                        }
                    }
                    if flags & (1 << command::TIME) != 0 {
                        if self.find(&MediaKey::Parcel).is_none() && settings.allow_scripted {
                            self.play_parcel(world, settings);
                        }
                        if let Some(m) = self.find_mut(&MediaKey::Parcel) {
                            m.seek(time as f64);
                        }
                    }
                }
                ParcelMediaMsg::Update {
                    url,
                    media_id,
                    auto_scale,
                    mime,
                    width,
                    height,
                    looping,
                } => {
                    let Some(parcel) = world.parcel.as_mut() else {
                        continue;
                    };
                    let pm = &parcel.media;
                    let changed = parcel.media_url != url
                        || pm.media_id != media_id
                        || pm.auto_scale != auto_scale
                        || (!mime.is_empty() && pm.mime != mime)
                        || pm.width != width
                        || pm.height != height
                        || pm.looping != looping;
                    if !changed {
                        continue;
                    }
                    let p = std::sync::Arc::make_mut(parcel);
                    p.media_url = url;
                    p.media.media_id = media_id;
                    p.media.auto_scale = auto_scale;
                    if !mime.is_empty() {
                        p.media.mime = mime;
                    }
                    p.media.width = width;
                    p.media.height = height;
                    p.media.looping = looping;
                    // re-played only if a parcel media exists
                    if self.find(&MediaKey::Parcel).is_some() {
                        self.remove(&MediaKey::Parcel);
                        self.play_parcel(world, settings);
                    }
                }
            }
        }
    }

    /// LLViewerParcelMedia::update + LLViewerParcelMediaAutoPlay::tick.
    fn update_parcel(&mut self, world: &World, f: &MediaFrame) {
        let key = world.parcel.as_ref().map(|p| (world.main_region, p.local_id));
        let location_changed = key != self.last_parcel_key;
        if location_changed {
            self.last_parcel_key = key;
            self.autoplay_since = Instant::now();
            self.autoplay_done = false;
        }
        let Some(parcel) = world.parcel.as_ref() else {
            if self.find(&MediaKey::Parcel).is_some() {
                self.stop_parcel();
            }
            return;
        };
        let pm = &parcel.media;
        let url = effective_parcel_url(pm, &parcel.media_url);
        if let Some(m) = self.find(&MediaKey::Parcel) {
            let pm_mime = pm.mime.trim().to_ascii_lowercase();
            let changed = m.url != url || m.media_id != pm.media_id;
            if changed {
                // only replayed if the type is the same and the parcel too
                if !location_changed && !url.is_empty() && (m.mime.is_empty() || m.mime == pm_mime) {
                    self.play_parcel(world, f.settings);
                } else {
                    self.stop_parcel();
                }
            }
        }
        if !self.autoplay_done
            && self.autoplay_since.elapsed() > AUTOPLAY_TIME
            && !url.is_empty()
            && !pm.mime.eq_ignore_ascii_case(NONE_MIME)
            && self.find(&MediaKey::Parcel).is_none()
            && !pm.media_id.is_nil()
            && f.agent_speed < AUTOPLAY_SPEED
            && self.parcel_interest(f.view, world) > AUTOPLAY_SIZE
        {
            if f.autoplay && f.settings.enabled {
                self.play_parcel(world, f.settings);
            }
            self.autoplay_done = true;
        }
    }

    /// Pixels covered by objects showing the parcel placeholder texture.
    fn parcel_interest(&self, view: &CullView, world: &World) -> f32 {
        let _ = world;
        let mut best = 0.0f32;
        for &(c, r) in &self.parcel_bounds {
            if view.sphere_visible(c, r) {
                let px = view.pixel_size(c, r);
                best = best.max(px * px);
            }
        }
        best
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;
    #[test]
    fn play_pause_resume_and_open_media_use_parcel_state_and_clicked_texture() {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            w.apply(ev);
        }
        let p = std::sync::Arc::make_mut(w.parcel.as_mut().unwrap());
        p.media_url = "https://example.invalid/media".into();
        p.media.mime = "text/html".into();
        p.media.media_id = crate::demo::ACTION_MEDIA_TEX;
        let settings = MediaSettings::default();
        let mut m = MediaManager::default();
        m.toggle_parcel(&w, &settings);
        assert!(m.parcel_playing());
        m.find_mut(&MediaKey::Parcel).unwrap().status = MediaStatus::Playing;
        assert!(m.parcel_status_playing());
        m.toggle_parcel(&w, &settings);
        assert!(!m.parcel_playing());
        m.find_mut(&MediaKey::Parcel).unwrap().status = MediaStatus::Paused;
        m.toggle_parcel(&w, &settings);
        assert!(m.parcel_playing());
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        assert!(m.click_open_media(&w, idx, -1, &settings).is_none());
        assert_eq!(
            m.click_open_media(&w, idx, 0, &settings).as_deref(),
            Some("https://example.invalid/media")
        );
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(978)).unwrap();
        m.find_mut(&MediaKey::Parcel).unwrap().status = MediaStatus::Playing;
        assert!(m.click_open_media(&w, idx, 0, &settings).is_none());
        assert!(!m.parcel_playing());
        w.parcel = None;
        assert!(m.click_open_media(&w, idx, 0, &settings).is_none());
    }
}
