//! Application: window, event loop, screens (login / loading / world).

use crate::agent::MoveInput;
use crate::camera::Camera;
use crate::frame_profile::{FrameProfile, Lap, Part, Parts, SceneCounts};
use crate::keybinds::{Action, Input, Mods};
use crate::scene::avatar::AvatarLibrary;
use crate::scene::{CullView, Scene};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::{self, Panels, bars::BarAction, chat::ChatUi, login::LoginAction, login::LoginForm, perf::PerfData, skin::Skin};
use crate::ui_sound::UiSound;
use crate::world::World;
use crate::world::env::Environment;
use aurora_net::{LoginRequest, NetClient, NetCommand, NetEvent, StartLocation, control};
use aurora_render::{DrawLists, EguiHandOver, FrameParams, RenderStats, Renderer};
use glam::{Vec3, Vec4};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

mod ao;
mod bug_report;
mod buy_currency;
mod buy_land;
mod chat_commands;
mod click_pick;
mod color_picker;
mod context_menu;
mod controls;
mod cursor_hold;
mod dof;
mod double_click;
mod env_apply;
mod env_day_editor;
mod env_editor;
mod gestures;
mod hover_height;
mod hud_drag;
mod imports;
mod inspect;
mod inventory;
mod inventory_drag;
mod inventory_thumbnail;
mod marketplace_listings;
mod object_actions;
mod pay;
mod plate_click;
mod region_restart;
mod report_abuse;
mod reset_skeleton;
mod scripts;
mod shape_editor;
mod snapshot;
mod task_notecards;
mod texture_refresh;
mod worn_demo;

enum Screen {
    Login,
    Loading { since: Instant },
    World,
}

struct Gfx {
    window: Arc<Window>,
    /// Revealed only once the renderer has presented its first image.
    shown: bool,
    maximize_on_show: bool,
    renderer: Renderer,
    egui_state: egui_winit::State,
}

/// Teleport screen state (also the login loading screen fading out).
struct TpOverlay {
    start: Instant,
    /// Arrived in the new region (then: region loading); end = fading out.
    arrived: Option<Instant>,
    end: Option<Instant>,
    dest: String,
    /// The login loading screen finishing (full bar, no teleport).
    loading_end: bool,
    /// Displayed progress (glides toward the steps, never backwards).
    shown: f32,
    /// What the screen said when its fade out began; it leaves saying that.
    leaving: Option<ui::loading::BannerText>,
}

impl TpOverlay {
    fn new(dest: String, loading_end: bool) -> TpOverlay {
        TpOverlay {
            start: Instant::now(),
            arrived: None,
            end: None,
            dest,
            loading_end,
            shown: 0.0,
            leaving: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseMode {
    None,
    /// Left button held on the own avatar: turn it with the mouse.
    Steer,
    /// Alt+left held: alt-camera drag (zoom; Ctrl orbit; Ctrl+Shift pan).
    FocusOrbit,
}

/// Aurora also uses a held right button to orbit, unlike LLToolPie::handleRightMouseDown
/// (indra/newview/lltoolpie.cpp), which opens the menu immediately. Keep that extension
/// from stealing ordinary clicks: jitter within 10 UI points never starts an orbit.
#[derive(Default)]
struct RightGesture {
    offset: (f32, f32),
    orbit: bool,
}

impl RightGesture {
    fn motion(&mut self, dx: f32, dy: f32, pixels_per_point: f32) -> bool {
        if !self.orbit {
            self.offset.0 += dx / pixels_per_point;
            self.offset.1 += dy / pixels_per_point;
            self.orbit = self.offset.0 * self.offset.0 + self.offset.1 * self.offset.1 >= 10.0 * 10.0;
        }
        // Once dragging, reversing direction must neither pause the orbit nor open a menu.
        self.orbit
    }
}

/// SL timings (llviewerinput.cpp): double-tap window / nudge phase, auto-fly.
const NUDGE_TIME: f32 = 0.25;
/// How fast the avatar turns toward the camera / walking direction while
/// steering with the left button (rad/s).
const STEER_TURN_RATE: f32 = 9.0;
const FLY_TIME: f32 = 0.5;

struct LogoData {
    ui: egui::ColorImage,
    icon: Option<(Vec<u8>, u32, u32)>,
}

pub struct App {
    settings: Settings,
    disk_cache: crate::cache::DiskCache,
    skin: Skin,
    palette: Palette,
    net: NetClient,
    gfx: Option<Gfx>,
    egui_ctx: egui::Context,
    logo: Option<egui::TextureHandle>,
    logo_rx: Option<crossbeam_channel::Receiver<LogoData>>,
    world: World,
    scene: Scene,
    probes: crate::scene::probes::ProbeManager,
    /// Throttle of the "sky frame" log line.
    sky_log: crate::world::env::SkyLog,
    camera: Camera,
    input: MoveInput,
    /// Keys and bindable mouse buttons held down.
    down: HashSet<Input>,
    shift: bool,
    ctrl: bool,
    screen: Screen,
    login_form: LoginForm,
    bug_report: ui::bug_report::BugReportUi,
    login_progress: Option<(String, f32)>,
    /// `AURORA_DEMO_LOGIN=add-mfa`: when the offline login server asks for
    /// the second factor, after the request has been pending a moment.
    demo_mfa_challenge: Option<std::time::Instant>,
    panels: Panels,
    chat_ui: ChatUi,
    toolbar_ui: ui::toolbar::ToolbarUi,
    /// The skin's toolbar as the dock of the islands has it by default.
    dock_layout: ui::bars::dock::DockLayout,
    /// Where the favourites lived when the toolbar last followed it (the
    /// dock's Favorites button comes and goes with `FavoritesAccess::Dock`).
    favorites_access_applied: Option<crate::settings::FavoritesAccess>,
    /// Thumbnails of the favourites to load.
    favorite_images: std::collections::HashSet<uuid::Uuid>,
    /// The History window (a conversation's whole transcript).
    history_ui: ui::chat::history::HistoryUi,
    perf: PerfData,
    /// AURORA_PROFILE: frame laps and the summary line in the log.
    frame_profile: FrameProfile,
    /// What the network did each second: Performance window, and the
    /// "net summary" line in the log with AURORA_PROFILE.
    net_profile: crate::net_profile::NetProfile,
    /// `AURORA_SYNC_SLICE=alt`: seconds of the frame profile so far.
    sync_alt: u64,
    /// AURORA_NAME_TAGS=alt: this second draws the classic tags.
    name_tags_alt_classic: bool,
    /// AURORA_DEMO_TAGS: the set of states its residents show.
    demo_tag_states: Option<bool>,
    /// AURORA_DEMO_PEOPLE: the states of its residents were sent.
    demo_people_states: bool,
    /// Interface parts of the frame's profile (`u_*`).
    ui_parts: Parts,
    /// The interface's shapes leave for the render thread, which
    /// tessellates them (aurora-render, ui_frame.rs).
    egui_hand_over: EguiHandOver,
    last_frame: Instant,
    /// Duration of the frame before the last one (s).
    prev_dt: f32,
    start: Instant,
    right_drag: bool,
    /// The system cursor hidden and locked by mouselook or a camera drag.
    cursor_hold: cursor_hold::CursorHold,
    numeric_drag: ui::numeric_drag::NumericDrag,
    alt: bool,
    left_down: bool,
    /// Clicks that wait for the depth under them, and what follows them.
    clicks: click_pick::ClickWait,
    /// Displacement while the right button is held (click vs orbit drag).
    right_gesture: RightGesture,
    /// Open right-click menu.
    context_menu: Option<ui::context::ContextMenu>,
    /// What a left-button drag does (steer the avatar, orbit a focus point).
    mouse_mode: MouseMode,
    /// Build tools (selection, manipulators, create, terraform).
    build: crate::build::BuildTool,
    hud_drag: Option<crate::build::hud_drag::HudDrag>,
    interactions: crate::interaction::Interactions,
    cursors: crate::cursors::Cursors,
    /// Demo mode: answers to the build tools' commands.
    build_demo: crate::build::demo_sim::DemoSim,
    /// Cursor position in physical pixels.
    cursor_pos: (f32, f32),
    /// Last walk-key press (for double-tap running) and the active temporary run.
    last_tap: Option<(u8, Instant)>,
    temp_run: Option<u8>,
    /// "Always run" (Ctrl+R).
    always_run: bool,
    /// The Controls window (`ui/controls.rs`), the keys its buttons and
    /// joysticks hold this frame, and its "Object view": a left click in the
    /// world focuses the camera, as with Alt (LLToolCamera).
    controls_ui: ui::controls::ControlsUi,
    controls_held: Vec<Action>,
    /// Its free camera joysticks this frame: move, look.
    controls_free: (egui::Vec2, egui::Vec2),
    object_view: bool,
    /// Double click on the ground (src/autopilot.rs): the presses of the
    /// left button on the world, whether the one being handled is the
    /// second of a double click, and the walk it may have started.
    double_click: crate::autopilot::DoubleClickDetector,
    double_press: bool,
    autopilot: Option<crate::autopilot::AutoPilot>,
    /// The control flags the walk asks for this frame.
    autopilot_flags: u32,
    /// Microphone switched on (toggle mode).
    mic_on: bool,
    /// AURORA_DEMO_CALL: the simulated voice server of the calls.
    call_demo: crate::demo::calls::CallDemo,
    /// The call whose conversation was last brought up.
    call_shown: Option<uuid::Uuid>,
    /// When the voice volumes changed by the call strip are saved.
    volumes_save_at: Option<Instant>,
    /// Residents added to a conversation: the picker and the confirmation.
    call_invite_ui: ui::call_invite::CallInviteUi,
    /// Microphone button of the bottom bar held (hold-to-talk mode).
    mic_button_held: bool,
    /// Voice: connected in this region, own speaking level (0..1).
    voice_connected: bool,
    voice_level: f32,
    /// When the forward/back and strafe keys started being held (nudge phase).
    fb_since: Option<Instant>,
    /// Turn key held since (yaw rate ramp).
    turn_since: Option<Instant>,
    /// Since when each camera key is held (CameraKeys rate ramp).
    cam_key_since: [Option<Instant>; 10],
    /// Look-at: the privacy settings (PrivateLookAtTarget,
    /// PrivateLocalLookAtTarget) our last ViewerEffect followed (none before
    /// the first frame), camera focus, cursor and body heading
    /// (LLAgentCamera::updateLookAt).
    look_at_privacy: Option<(bool, bool)>,
    /// What the simulator was told of the static eyes animation this
    /// session (FSStaticEyes); none before the arrival.
    static_eyes_sent: Option<bool>,
    /// The world point the cursor was over last frame, and the object it
    /// lies on (the hover pick of the cursor icon), for the look-at.
    look_hover: Option<(Option<usize>, Vec3)>,
    /// Fade-in of the look-at targets (`ui::lookat_targets`).
    look_at_fades: ui::lookat_targets::Fades,
    /// Offline demo: eye tracking on without touching the saved settings.
    demo_look_at: bool,
    /// AURORA_DEMO_LOOKAT=targets: the crosshairs shown, every avatar
    /// looking somewhere.
    demo_look_at_targets: bool,
    look_focus: Option<Vec3>,
    look_cursor: (f32, f32),
    look_root_at: Vec3,
    lr_since: Option<Instant>,
    /// When the jump key started being held (auto-fly after 0.5 s).
    up_since: Option<Instant>,
    /// Last vertical control flags sent (diagnostic log).
    last_vertical_flags: u32,
    last_render: RenderStats,
    empty_lists: DrawLists,
    quit_at: Option<Instant>,
    exit_prompt: ui::exit::ExitPrompt,
    loading_stage: String,
    move_complete_at: Option<Instant>,
    /// Teleport diagnostics still to log (seconds after the arrival).
    tp_checks: Vec<f32>,
    /// Debug: culling view kept while "Freeze culling" is on.
    frozen_cull: Option<CullView>,
    /// Interface sounds caused by the widgets (clicks, keys, windows).
    sound_cues: ui::sound_cues::SoundCues,
    /// Last frame: microphone on, context menu open, interactive script
    /// dialogs (their sounds).
    mic_was: bool,
    context_menu_was: bool,
    /// "Delete" waiting for a yes: (object, why it asks).
    delete_confirm: Option<(uuid::Uuid, String)>,
    /// Teleport asked by a place link, waiting for a yes (TeleportViaSLAPP),
    /// with the state of its "Do not show again" box.
    place_confirm: Option<(String, Vec3, bool)>,
    /// External web link waiting for the warning's answer, with the state of
    /// its "Do not warn me again" box and, for a dangerous link, the reason.
    url_confirm: Option<(String, bool, Option<&'static str>)>,
    script_dialogs_were: usize,
    /// Last tracker beacon sound (FIRE-16969).
    beacon_sound_at: Option<Instant>,
    /// Movement keys held since, and the server position then (stuck agent).
    stuck_watch: Option<(Instant, Vec3)>,
    stuck_logged: Option<Instant>,
    /// Attachment diagnostics written to the log (0, 1 or 2 times).
    attachments_logged: u8,
    loading_frac: f32,
    loading_target: f32,
    frame_count: u64,
    /// Refresh rate of the screen the window is on, followed while it runs.
    screen_rate: crate::screen_rate::ScreenRate,
    /// The window has the keyboard focus (background frame cap).
    focused: bool,
    /// Loading / teleport screen background (last view, blurred).
    backdrop: ui::backdrop::Backdrop,
    /// Capture the 3D view on the next frame; a capture is on its way.
    want_scene_capture: bool,
    scene_capture_pending: bool,
    scene_capture_place: Option<(crate::login_accounts::Account, String)>,
    /// Teleport screen: start, end (fade out), destination.
    tp_overlay: Option<TpOverlay>,
    was_teleporting: bool,
    /// Offline demo teleport: started at.
    demo_tp_start: Option<Instant>,
    demo_tp_triggered: bool,
    /// AURORA_DEMO_MMO: frame its script starts at (world shown).
    demo_mmo_start: Option<u64>,
    /// Where the offline demo teleport lands (the asked position).
    demo_tp_dest: Option<Vec3>,
    /// AURORA_DEMO_RADAR=tp: the radar starts in another region.
    demo_radar_tp: bool,
    /// Destination of the teleport being started (teleport screen).
    tp_dest: Option<String>,
    /// Window moved / resized: its normal geometry is recorded once settled.
    geom_changed: Option<Instant>,
    frame_start: Instant,
    people_ui: ui::people::PeopleUi,
    /// Avatars around us, swept once a second on `radar_clock`.
    radar: crate::world::radar::Radar,
    radar_clock: Instant,
    contacts_ui: ui::contacts::ContactsUi,
    /// Mini-map and world map state, map server tiles.
    minimap_ui: ui::minimap::MiniMap,
    display_name_ui: ui::display_name::DisplayNameUi,
    /// World › Landmark this Place (LLFloaterCreateLandmark).
    create_landmark_ui: ui::landmark_create::CreateLandmarkUi,
    /// Inventory generation whose Favorites / Landmarks folders were asked for.
    landmark_folders_checked: u64,
    hover_height_ui: ui::hover_height::HoverHeightUi,
    /// Report Abuse (LLFloaterReporter).
    report_abuse: ui::report_abuse::ReportAbuseUi,
    /// The depth of field's focus distance and its transition.
    dof_focus: crate::dof::Focus,
    /// Photos: the window, the frames drawn at their size, the saves.
    snapshots: snapshot::Snapshots,
    inspect_ui: ui::inspect::InspectUi,
    buy_currency_ui: ui::buy_currency::BuyCurrencyUi,
    /// The Pay window of a resident or a group (`ui/pay.rs`).
    pay_ui: ui::pay::PayUi,
    /// Buy Land (LLFloaterBuyLand), and the selection it follows.
    buy_land_ui: ui::buy_land::BuyLandUi,
    buy_land_source: buy_land::BuyLandSource,
    /// World › SL Marketplace (LLFloaterMarketplace).
    marketplace_ui: ui::marketplace::MarketplaceUi,
    /// Me › Marketplace listings (LLFloaterMarketplaceListings).
    listings_ui: ui::marketplace_listings::ListingsUi,
    /// The demo's SLM server (AURORA_DEMO_MARKETPLACE_LISTINGS).
    demo_slm: Option<crate::world::marketplace::demo::DemoSlm>,
    people_map_ui: ui::minimap::MiniMap,
    worldmap_ui: ui::worldmap::WorldMapUi,
    map_tiles: ui::map_tiles::MapTiles,
    options_ui: ui::options::OptionsUi,
    audio_ui: ui::audio::AudioUi,
    notif_ui: ui::notifications::NotifUi,
    /// The region restart alert (app/region_restart.rs).
    region_restart: region_restart::State,
    /// AURORA_DEMO_SLURLS: region lookups the demo server answers late.
    demo_lookups: Vec<(Instant, NetCommand)>,
    /// Parcel music stream on (radio toggle).
    music_playing: bool,
    /// Parcel media and media on a prim (web / video plugins).
    media: crate::media::MediaManager,
    media_ui: ui::media::MediaUi,
    /// Cursor asked by the focused media page (link hand, text beam).
    media_cursor: Option<egui::CursorIcon>,
    /// Audio output (None: no device / failed to start).
    audio_engine: Option<aurora_audio::AudioEngine>,
    /// Settings last applied to the engine, stream URL playing, parcel music URL seen.
    applied_audio: Option<crate::settings::AudioSettings>,
    stream_url: String,
    music_parcel_url: String,
    /// `/music` chat command: a stream played instead of the parcel's until
    /// the parcel stream changes.
    music_override: Option<String>,
    /// `key2name` chat commands waiting for a name.
    key_to_name: Vec<(uuid::Uuid, Instant)>,
    /// Voice chat (WebRTC) and the HTTP client it provisions with.
    voice: crate::voice::Voice,
    voice_http: reqwest::Client,
    /// Login screen news and latest version.
    login_info: ui::news::LoginInfo,
    update_demo: Option<ui::update::UpdateDemo>,
    updater: crate::updater::Updater,
    /// Aurora's media plugins pack: installed, downloading or missing.
    plugin_pack: crate::media::pack::PluginPack,
    update_acknowledged: bool,
    /// Smoothed speaking outlines of the avatar name tags.
    voice_tags: ui::voice_tag::VoiceTags,
    /// Name plates of the avatars (the default style of the name tags).
    plates: ui::nameplate::Plates,
    /// Avatar name tags on screen last frame, far first: a click on one
    /// counts as one on its avatar.
    name_tags: Vec<ui::hud::NameTag>,
    /// Clickable destination marker last painted under the HUDs and UI.
    beacon_hit: Option<ui::minimap::BeaconHit>,
    /// The avatar whose plate is under the pointer (app/plate_click.rs).
    plate_hover: Option<uuid::Uuid>,
    /// Block list version applied to object sounds, and the objects whose
    /// sounds it silenced.
    sound_blocks: (u64, std::collections::HashSet<uuid::Uuid>),
    /// Objects whose update carried their attached sound (reused).
    sound_changes: Vec<usize>,
    /// Audio devices listed for the preferences.
    devices_listed: bool,
    /// Color emoji (Noto Color Emoji) for chat and the picker.
    emoji: ui::emoji::Emoji,
    /// The system's fallback fonts were still being looked for when the
    /// fonts were installed: installed again once they are there.
    system_fonts_pending: bool,
    /// Profile pictures by avatar id (egui textures).
    avatar_pics: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    /// The same textures by picture, for the residents who share one.
    avatar_pic_images: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    /// Other interface images by asset id (profiles: 1st life picture,
    /// group insignias).
    ui_images: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    /// Avatar profile windows.
    profile_ui: ui::profile::ProfileUi,
    /// "Location details" windows (standalone place profiles).
    place_ui: ui::place_details::PlaceDetailsUi,
    /// "Places".
    places_ui: ui::places::PlacesUi,
    /// "Search".
    search_ui: ui::search::SearchUi,
    /// AURORA_DEMO_PLACE=repere|historique, opened once logged in.
    demo_place: Option<crate::world::place_details::Source>,
    /// AURORA_DEMO_STREAM: objects and textures arriving in waves.
    stream_demo: Option<crate::demo::stream::StreamDemo>,
    /// AURORA_DEMO_TEXREFRESH: "Refresh textures" on a stale cached texture.
    tex_refresh_demo: Option<crate::demo::tex_refresh::TexRefreshDemo>,
    /// AURORA_DEMO_EXPERIENCE: experiences asking (door, pad, greeter).
    experience_demo: Option<crate::demo::experience::ExperienceDemo>,
    /// AURORA_DEMO_GEOMETRY: meshes filling the geometry arena.
    geometry_demo: Option<crate::demo::geometry::GeometryDemo>,
    /// Detail levels of the objects around the camera, while the debug
    /// overlay shows them.
    lod_infos: Vec<crate::scene::LodInfo>,
    /// Parcel snapshots and group names the place profiles want.
    place_images: std::collections::HashSet<uuid::Uuid>,
    place_groups: std::collections::HashSet<uuid::Uuid>,
    /// "About Land".
    land_ui: ui::land::LandUi,
    region_ui: ui::region::RegionUi,
    group_profile_ui: ui::group_profile::GroupProfileUi,
    /// Environment selector and "Personal lighting".
    env_ui: ui::environment::EnvironmentUi,
    /// The selector's folder fetches started (the selector was opened).
    env_scan: bool,
    /// "Personal lighting" was open last frame (captures on opening).
    lighting_was_open: bool,
    /// Sky and water editors (LLFloaterFixedEnvironment).
    env_editor: ui::env_editor::EnvEditorUi,
    /// A Windlight preset being read for "Import" (bytes and preset name).
    env_import: Option<std::sync::mpsc::Receiver<Option<(Vec<u8>, String)>>>,
    /// Day cycle editor (LLFloaterEditExtDayCycle).
    day_editor: ui::env_day_editor::DayEditorUi,
    /// A sky, water or day item the day editor waits for, and what for.
    day_fetch: Option<(uuid::Uuid, env_day_editor::DayFetch)>,
    /// A Windlight day being read for the day editor's "Import".
    day_import: Option<std::sync::mpsc::Receiver<env_apply::DayImport>>,
    /// The ExtEnvironment update an editor sent and waits for.
    env_applying: Option<env_apply::Applying>,
    /// The sun and moon beacons when the first environment editor opened.
    env_beacons_saved: Option<[bool; 2]>,
    demo: bool,
    inventory_ui: ui::inventory::InventoryUi,
    gesture_state: gestures::State,
    imports: crate::imports::Imports,
    task_transfer: Option<ui::inventory::task_drop::PendingTransfer>,
    /// Inventory edits asked by the world (`World::take_inventory_edits`),
    /// awaiting their reply.
    world_edits: std::collections::HashSet<uuid::Uuid>,
    /// AURORA_DEMO_INVENTORY=offer: folder the offered item was last shown in.
    demo_offer_shown: Option<uuid::Uuid>,
    ao_ui: ui::ao::AoUi,
    ao_catalog: crate::ao::inventory::Catalog,
    ao_runtime: crate::ao::runtime::Runtime,
    ao_agent: uuid::Uuid,
    ao_missing: HashSet<uuid::Uuid>,
    ao_demo: Option<String>,
    ao_default_applied: bool,
    inventory_drag: inventory_drag::InventoryDrag,
    task_notecards: std::collections::HashMap<(uuid::Uuid, uuid::Uuid), task_notecards::TaskNotecard>,
    demo_documents: std::collections::HashMap<uuid::Uuid, Vec<u8>>,
    script_editors: ui::scripts::Editors,
    demo_script_sources: std::collections::HashMap<aurora_net::scripts::ScriptTarget, String>,
    demo_script_experiences: std::collections::HashMap<aurora_net::scripts::ScriptTarget, uuid::Uuid>,
    inventory_images: inventory_thumbnail::ImageJobs,
    appearance_ui: ui::appearance::AppearanceUi,
    shape_ui: crate::shape_editor::State,
    shape_camera: Option<Camera>,
    last_social_poll: Instant,
    /// Last keyboard / mouse button input (automatic away, AFKTimeout), and
    /// whether the current away status was set by that timer.
    last_input: Instant,
    auto_away: bool,
    /// AURORA_CAPTURE: file, frames to capture (several: one file each,
    /// `<name>-<frame>.png`), quit after the last one.
    capture: Option<(Vec<(u64, std::path::PathBuf)>, bool)>,
}

/// The bandwidth setting, or AURORA_BANDWIDTH_KBPS for this launch: a value
/// beyond the range of the Preferences, to measure what a simulator sends
/// when asked for more (the setting itself is not changed).
fn bandwidth_kbps(settings: &Settings) -> f32 {
    std::env::var("AURORA_BANDWIDTH_KBPS")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|kbps| kbps.is_finite())
        .unwrap_or(settings.bandwidth_kbps as f32)
}

/// Test overrides of the settings (used by automated captures): the
/// graphics level first, so that the single settings after it still change
/// it. Applied at start, and again after a new profile's level is chosen.
fn apply_test_overrides(settings: &mut Settings) {
    if let Some(level) = std::env::var("AURORA_GRAPHICS_PRESET")
        .ok()
        .and_then(|v| crate::graphics_presets::Preset::from_name(&v))
    {
        level.apply(settings);
    }
    if let Some(v) = std::env::var("AURORA_FPS_LIMIT")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&v| v > 0)
    {
        settings.fps_cap = true;
        settings.fps_limit = v.clamp(10, 500);
    }
    if let Some(v) = std::env::var("AURORA_AA").ok().and_then(|v| v.parse().ok()) {
        settings.antialiasing = v;
    }
    if let Some(v) = std::env::var("AURORA_MAX_COMPLEXITY").ok().and_then(|v| v.parse().ok()) {
        settings.max_complexity = v;
    }
    if let Some(v) = std::env::var("AURORA_MAX_AVATARS").ok().and_then(|v| v.parse().ok()) {
        settings.max_avatars = v;
    }
    if let Some(v) = std::env::var("AURORA_SHADOWS").ok().and_then(|v| v.parse().ok()) {
        settings.shadow_quality = v;
    }
    if let Ok(v) = std::env::var("AURORA_PROJECTOR_SHADOWS") {
        settings.projector_shadows = v != "0";
    }
    if let Ok(v) = std::env::var("AURORA_OCCLUSION") {
        settings.occlusion_culling = v != "0";
    }
}

impl App {
    pub fn new() -> anyhow::Result<App> {
        // found on a background thread while the rest starts
        ui::fonts::fallback::start();
        let mut settings = Settings::load();
        aurora_i18n::select(&settings.language, crate::cli::get().language.as_deref());
        if std::env::var_os("AURORA_DEMO").is_some() && std::env::var_os("AURORA_DEMO_KEYBOARD").is_some() {
            // Exercise a first launch without changing the user's bindings.
            settings.keybinds = Default::default();
        }
        apply_test_overrides(&mut settings);
        let net = NetClient::new()?;
        net.set_agent_language(aurora_i18n::language(), settings.language_is_public);
        let voice_http = net.caps_http();
        let login_info = ui::news::LoginInfo::start(net.runtime());
        let updater = crate::updater::Updater::new(net.runtime());
        let plugin_pack = crate::media::pack::PluginPack::new(crate::settings::media_plugins_dir());
        let mut media = crate::media::MediaManager::default();
        media.set_pack_dir(plugin_pack.dir());
        let out_device = (!settings.audio.output_device.is_empty()).then(|| settings.audio.output_device.clone());
        let lib = Arc::new(AvatarLibrary::load());
        let cache = crate::settings::cache_dir();
        let disk_cache = crate::cache::DiskCache::new(cache.clone(), settings.cache_size_mb);
        net.set_object_cache_dir(Some(cache.join("objects")));
        net.set_bandwidth(bandwidth_kbps(&settings));
        let cache_root = cache.clone();
        let egui_ctx = egui::Context::default();
        ui::colors::set(&settings.colors);
        let skin = Skin::load(&egui_ctx, "aurora", settings.font_scale);
        crate::crash::update(&skin.theme, &settings);
        let palette = skin.theme.palette();
        egui_ctx.set_zoom_factor(settings.ui_scale);
        // last view of the previous session for the loading screen
        let mut backdrop = ui::backdrop::Backdrop::default();
        if settings.loading_backdrop && std::env::var_os("AURORA_CAPTURE").is_none() {
            backdrop.load_saved();
        }
        // floater positions / sizes from the last session (not for
        // captures, unless AURORA_CAPTURE_LAYOUT asks for it: a capture of
        // what a saved layout gives in this window; it is never written)
        if std::env::var_os("AURORA_CAPTURE").is_none() || std::env::var_os("AURORA_CAPTURE_LAYOUT").is_some() {
            ui::layout::load(&egui_ctx);
        }
        // after the layout: restoring it drops the fonts installed before
        let system_fonts_pending = !ui::fonts::apply(&egui_ctx, &settings.font);

        // Decode the 4096px logo off the main thread.
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let bytes = include_bytes!("../assets/logo-loup-couleur-4096.png");
            match image::load_from_memory(bytes) {
                Ok(img) => {
                    let ui_img = img.resize(384, 384, image::imageops::FilterType::Triangle).to_rgba8();
                    let icon = img.resize(64, 64, image::imageops::FilterType::Triangle).to_rgba8();
                    let ci = egui::ColorImage::from_rgba_unmultiplied([ui_img.width() as usize, ui_img.height() as usize], ui_img.as_raw());
                    let (w, h) = icon.dimensions();
                    let _ = tx.send(LogoData {
                        ui: ci,
                        icon: Some((icon.into_raw(), w, h)),
                    });
                }
                Err(e) => log::warn!("logo decode failed: {e}"),
            }
        });

        let panels = Panels::from_settings(&settings);
        let mut scene = Scene::new(cache, lib.clone());
        scene.draw_distance = settings.draw_distance;
        scene.lod_factor = settings.lod_factor;
        let update_demo = ui::update::UpdateDemo::from_env(&settings);
        Ok(App {
            settings,
            disk_cache,
            skin,
            palette,
            net,
            gfx: None,
            egui_ctx,
            logo: None,
            logo_rx: Some(rx),
            world: {
                let mut w = World::new(lib);
                w.cache_dir = Some(cache_root);
                w
            },
            scene,
            probes: Default::default(),
            sky_log: Default::default(),
            camera: Camera::default(),
            input: MoveInput::default(),
            down: HashSet::new(),
            shift: false,
            ctrl: false,
            screen: Screen::Login,
            login_form: LoginForm::default(),
            bug_report: Default::default(),
            login_progress: None,
            demo_mfa_challenge: None,
            panels,
            chat_ui: ChatUi::default(),
            toolbar_ui: Default::default(),
            dock_layout: Default::default(),
            favorites_access_applied: None,
            favorite_images: Default::default(),
            history_ui: Default::default(),
            perf: PerfData::default(),
            frame_profile: FrameProfile::default(),
            net_profile: crate::net_profile::NetProfile::default(),
            sync_alt: 0,
            name_tags_alt_classic: false,
            demo_tag_states: None,
            demo_people_states: false,
            ui_parts: Parts::default(),
            egui_hand_over: EguiHandOver::default(),
            last_frame: Instant::now(),
            prev_dt: 0.0,
            start: Instant::now(),
            right_drag: false,
            cursor_hold: Default::default(),
            numeric_drag: Default::default(),
            alt: false,
            left_down: false,
            clicks: Default::default(),
            right_gesture: RightGesture::default(),
            context_menu: None,
            mouse_mode: MouseMode::None,
            build: Default::default(),
            hud_drag: None,
            interactions: Default::default(),
            cursors: crate::cursors::Cursors::with_palette(&palette),
            build_demo: Default::default(),
            cursor_pos: (0.0, 0.0),
            last_tap: None,
            temp_run: None,
            always_run: false,
            controls_ui: Default::default(),
            controls_held: Vec::new(),
            controls_free: (egui::Vec2::ZERO, egui::Vec2::ZERO),
            object_view: false,
            double_click: Default::default(),
            double_press: false,
            autopilot: None,
            autopilot_flags: 0,
            mic_on: false,
            call_demo: Default::default(),
            call_shown: None,
            volumes_save_at: None,
            call_invite_ui: Default::default(),
            mic_button_held: false,
            voice_connected: false,
            voice_level: 0.0,
            fb_since: None,
            turn_since: None,
            cam_key_since: [None; 10],
            look_at_privacy: None,
            static_eyes_sent: None,
            look_hover: None,
            look_at_fades: Default::default(),
            demo_look_at: false,
            demo_look_at_targets: false,
            look_focus: None,
            look_cursor: (0.0, 0.0),
            look_root_at: Vec3::X,
            lr_since: None,
            up_since: None,
            last_vertical_flags: 0,
            last_render: RenderStats::default(),
            empty_lists: DrawLists::default(),
            quit_at: None,
            exit_prompt: Default::default(),
            loading_stage: String::new(),
            move_complete_at: None,
            tp_checks: Vec::new(),
            frozen_cull: None,
            sound_cues: Default::default(),
            mic_was: false,
            context_menu_was: false,
            delete_confirm: None,
            place_confirm: None,
            url_confirm: None,
            script_dialogs_were: 0,
            beacon_sound_at: None,
            stuck_watch: None,
            stuck_logged: None,
            attachments_logged: 0,
            loading_frac: 0.0,
            loading_target: 0.0,
            frame_count: 0,
            screen_rate: Default::default(),
            focused: true,
            backdrop,
            want_scene_capture: false,
            scene_capture_pending: false,
            scene_capture_place: None,
            tp_overlay: None,
            was_teleporting: false,
            demo_tp_start: None,
            demo_tp_triggered: false,
            demo_mmo_start: None,
            demo_tp_dest: None,
            demo_radar_tp: false,
            tp_dest: None,
            geom_changed: None,
            frame_start: Instant::now(),
            people_ui: Default::default(),
            radar: Default::default(),
            radar_clock: Instant::now(),
            contacts_ui: Default::default(),
            minimap_ui: Default::default(),
            display_name_ui: Default::default(),
            create_landmark_ui: Default::default(),
            landmark_folders_checked: u64::MAX,
            hover_height_ui: Default::default(),
            report_abuse: Default::default(),
            dof_focus: Default::default(),
            snapshots: Default::default(),
            inspect_ui: Default::default(),
            buy_currency_ui: Default::default(),
            pay_ui: Default::default(),
            buy_land_ui: Default::default(),
            buy_land_source: Default::default(),
            marketplace_ui: Default::default(),
            listings_ui: Default::default(),
            demo_slm: None,
            people_map_ui: Default::default(),
            worldmap_ui: Default::default(),
            map_tiles: ui::map_tiles::MapTiles::new(),
            options_ui: Default::default(),
            audio_ui: Default::default(),
            notif_ui: Default::default(),
            region_restart: Default::default(),
            demo_lookups: Vec::new(),
            music_playing: false,
            audio_engine: match aurora_audio::AudioEngine::new_with_device(out_device) {
                Ok(e) => {
                    let o = e.output_info();
                    log::info!("audio: {} ({} Hz, {} canaux)", o.device_name, o.sample_rate, o.channels);
                    Some(e)
                }
                Err(e) => {
                    log::warn!("audio disabled: {e}");
                    None
                }
            },
            applied_audio: None,
            stream_url: String::new(),
            music_parcel_url: String::new(),
            music_override: None,
            key_to_name: Vec::new(),
            media,
            media_ui: Default::default(),
            media_cursor: None,
            voice: Default::default(),
            voice_http,
            update_demo,
            updater,
            plugin_pack,
            update_acknowledged: false,
            login_info,
            voice_tags: Default::default(),
            plates: Default::default(),
            name_tags: Vec::new(),
            beacon_hit: None,
            plate_hover: None,
            sound_blocks: Default::default(),
            sound_changes: Vec::new(),
            devices_listed: false,
            emoji: ui::emoji::Emoji::load(),
            system_fonts_pending,
            avatar_pics: Default::default(),
            avatar_pic_images: Default::default(),
            ui_images: Default::default(),
            profile_ui: Default::default(),
            place_ui: Default::default(),
            places_ui: Default::default(),
            search_ui: Default::default(),
            place_images: Default::default(),
            demo_place: None,
            stream_demo: None,
            tex_refresh_demo: None,
            experience_demo: None,
            geometry_demo: None,
            lod_infos: Vec::new(),
            place_groups: Default::default(),
            land_ui: Default::default(),
            region_ui: Default::default(),
            group_profile_ui: Default::default(),
            env_ui: Default::default(),
            env_scan: false,
            lighting_was_open: false,
            env_editor: Default::default(),
            env_import: None,
            day_editor: Default::default(),
            day_fetch: None,
            day_import: None,
            env_applying: None,
            env_beacons_saved: None,
            demo: false,
            inventory_ui: Default::default(),
            gesture_state: Default::default(),
            imports: Default::default(),
            task_transfer: None,
            world_edits: Default::default(),
            demo_offer_shown: None,
            ao_ui: Default::default(),
            ao_catalog: Default::default(),
            ao_runtime: Default::default(),
            ao_agent: uuid::Uuid::nil(),
            ao_missing: Default::default(),
            ao_demo: std::env::var("AURORA_DEMO_AO").ok(),
            ao_default_applied: false,
            inventory_drag: Default::default(),
            task_notecards: Default::default(),
            demo_documents: Default::default(),
            script_editors: Default::default(),
            demo_script_sources: Default::default(),
            demo_script_experiences: Default::default(),
            inventory_images: Default::default(),
            appearance_ui: Default::default(),
            shape_ui: Default::default(),
            shape_camera: None,
            last_social_poll: Instant::now(),
            last_input: Instant::now(),
            auto_away: false,
            capture: std::env::var_os("AURORA_CAPTURE").map(|p| {
                let frames = std::env::var("AURORA_CAPTURE_FRAMES").unwrap_or_default();
                (
                    capture_files(std::path::Path::new(&p), &frames),
                    std::env::var_os("AURORA_CAPTURE_EXIT").is_some(),
                )
            }),
        })
    }

    /// What the experience demo's simulator answers or does on its own.
    fn apply_experience_demo(&mut self, reply: crate::demo::experience::Reply) {
        match reply {
            crate::demo::experience::Reply::Events(events) => {
                for ev in events {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
            }
            crate::demo::experience::Reply::Say(text) => self.world.system_message(text),
            crate::demo::experience::Reply::Nothing => {}
        }
    }

    fn send(&mut self, cmd: NetCommand) {
        // LLAgent::startTeleportRequest: a teleport waits while the maturity
        // preference is not in sync with the server (TELEPORT_PENDING)
        if !self.world.maturity.synced()
            && matches!(
                cmd,
                NetCommand::TeleportTo { .. }
                    | NetCommand::TeleportLandmark(_)
                    | NetCommand::TeleportToRegion { .. }
                    | NetCommand::TeleportHome
                    | NetCommand::AcceptLure { .. }
            )
        {
            log::info!("teleport: waiting for the maturity preference");
            self.world.maturity.hold(cmd);
            return;
        }
        // AURORA_DEMO_EXPERIENCE: the simulator and the capabilities
        if self.demo
            && let Some(reply) = self.experience_demo.as_mut().and_then(|demo| demo.reply(&cmd))
        {
            self.apply_experience_demo(reply);
            return;
        }
        // AURORA_DEMO_MATURITY: the server agrees, keeps its value or fails
        if self.demo
            && let NetCommand::SetMaturityPreference(maturity) = cmd
        {
            let result = crate::demo::maturity_reply(maturity);
            log::info!("demo maturity: asked {maturity}, answer {result:?}");
            self.world.apply(NetEvent::MaturityPreference(result));
            return;
        }
        if self.demo && self.gesture_demo_command(&cmd) {
            return;
        }
        // AURORA_DEMO_SLURLS: a region lookup answered a little later
        if self.demo && crate::demo::slurls_scene() && matches!(cmd, NetCommand::MapNameRequest { .. }) {
            self.demo_lookups.push((Instant::now() + crate::demo::SLURLS_LOOKUP_DELAY, cmd));
            return;
        }
        if self.demo
            && let NetCommand::SetHoverHeight { region, request, value } = &cmd
        {
            let success = !std::env::var("AURORA_DEMO_HOVER_HEIGHT").is_ok_and(|mode| mode == "failure");
            log::info!("demo hover height: committed {value:.3}, success {success}");
            self.world.hover_height.reply(*request, *region, success);
            return;
        }
        if self.demo
            && let NetCommand::UploadAsset { request, upload } = cmd
        {
            self.demo_asset_upload(request, upload);
            return;
        }
        if self.demo {
            if let NetCommand::QuoteMesh { request, .. } = &cmd {
                self.on_app_event(NetEvent::MeshQuoted {
                    request: *request,
                    result: if self.imports.demo_mode == "bulk-mesh-denied" {
                        Err(aurora_net::inventory::upload::Error::MeshPermission)
                    } else {
                        Ok(aurora_net::inventory::mesh_upload::Quote {
                            token: uuid::Uuid::new_v4(),
                            cost: 11,
                            data: aurora_llsd::Llsd::new_map(),
                        })
                    },
                });
                return;
            }
            if let NetCommand::UploadMesh { request, .. } = &cmd {
                if let Some(upload) = self.imports.demo_mesh_request() {
                    self.demo_asset_upload(*request, upload);
                }
                return;
            }
            match &cmd {
                NetCommand::Build(aurora_net::build::BuildCmd::SetTextures { local_id, .. })
                    if ui::color_picker::demo::mode().is_some() =>
                {
                    log::info!("demo color ObjectImage: prim={local_id}");
                }
                NetCommand::TeleportTo { handle, position, look_at } => {
                    if Some(*handle) == self.world.main_region {
                        self.world.apply(NetEvent::TeleportLocal {
                            flying: self.world.agent.flying,
                        });
                        if let Some(ev) = self.world.apply(NetEvent::AgentMovementComplete {
                            handle: *handle,
                            position: *position,
                            look_at: *look_at,
                        }) {
                            self.on_app_event(ev);
                        }
                    } else {
                        self.demo_tp_start = Some(Instant::now());
                        self.demo_tp_dest = Some(*position);
                    }
                }
                NetCommand::ObjectGrab { local_id, surface, .. } => {
                    log::info!("demo touch_start: prim={local_id}, face={}, uv={:?}", surface.face, surface.uv);
                    self.world.system_message(aurora_i18n::tr!("app-demo-touch-start-simulated"));
                }
                NetCommand::ObjectGrabUpdate { surface, .. } => {
                    log::info!("demo touch / grab update: face={}, uv={:?}", surface.face, surface.uv);
                }
                NetCommand::ObjectRelease { local_id, surface, .. } => {
                    log::info!("demo touch_end: prim={local_id}, face={}", surface.face);
                    self.world.system_message(aurora_i18n::tr!("app-demo-touch-end-simulated"));
                }
                NetCommand::AvatarTexturesRequest(id) => log::info!("demo avatartexturesrequest: {id}"),
                NetCommand::IncrementCofVersion => {
                    // the server's answer: the folder's version, one step on
                    let version = self
                        .world
                        .inventory
                        .folders
                        .values()
                        .find(|f| f.info.type_default == crate::world::outfit::FT_CURRENT_OUTFIT && !f.library)
                        .map(|f| f.info.version + 1);
                    log::info!("demo IncrementCOFVersion: {version:?}");
                    let _ = self.world.apply(NetEvent::CofVersionIncremented(version));
                }
                _ => {}
            }
            let expense = match &cmd {
                NetCommand::BuyObject { price, .. } => Some((*price, "achat")),
                NetCommand::PayObject { amount, .. } | NetCommand::PayResident { amount, .. } => Some((*amount, "paiement")),
                _ => None,
            };
            if let Some((amount, kind)) = expense {
                log::info!("demo action: {kind} L$ {amount}");
                let balance = self.world.balance.unwrap_or(250).saturating_sub(amount);
                self.world.apply(NetEvent::Balance(balance));
                self.pay_ui.on_balance(balance);
                self.world.system_message(aurora_i18n::tr!(
                    "app-demo-of-l-simulated",
                    kind = kind.to_string(),
                    amount = format!("{}", amount)
                ));
            }
            if matches!(&cmd, NetCommand::OneShotControl(f) if f & control::STAND_UP != 0)
                && std::env::var_os("AURORA_DEMO_ACTIONS").is_some()
            {
                self.world.apply(NetEvent::ObjectUpdates {
                    handle: self.world.main_region.unwrap_or_default(),
                    objects: vec![crate::demo::action_avatar(false)],
                });
            }
            if let NetCommand::RequestObjects { handle, local_ids } = &cmd {
                let objects = local_ids
                    .iter()
                    .filter_map(|local_id| {
                        self.world
                            .objects
                            .index_of(&crate::world::objects::ObjKey {
                                region: *handle,
                                local_id: *local_id,
                            })
                            .and_then(|i| self.world.objects.get(i))
                            .map(crate::build::replace::demo_update)
                    })
                    .collect();
                self.world.apply(NetEvent::ObjectUpdates { handle: *handle, objects });
                return;
            }
            if let NetCommand::Build(b) = &cmd {
                for ev in self.build_demo.reply(&self.world, b) {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
                return;
            }
            if self.demo_script_command(&cmd)
                || self.demo_currency_command(&cmd)
                || self.demo_buy_land_command(&cmd)
                || self.demo_marketplace_command(&cmd)
            {
                return;
            }
            if matches!(cmd, NetCommand::RequestInventoryMerchant) {
                self.inventory_ui.merchant = true;
                return;
            }
            if let NetCommand::EditInventory { request, change } = &cmd {
                let result = crate::world::inventory::actions::demo_mutate(&mut self.world.inventory, self.world.agent_id, change.clone());
                self.on_app_event(NetEvent::InventoryEdited { request: *request, result });
                return;
            }
            if let NetCommand::UploadInventoryThumbnail {
                request,
                item,
                folder,
                parent,
                data,
            } = &cmd
            {
                let asset = uuid::Uuid::new_v4();
                if let Ok(decoded) = aurora_assets::decode_j2k(data, 0) {
                    let texture = self.egui_ctx.load_texture(
                        format!("demo_thumbnail_{asset}"),
                        egui::ColorImage::from_rgba_unmultiplied([decoded.width as usize, decoded.height as usize], &decoded.data),
                        egui::TextureOptions::LINEAR,
                    );
                    self.ui_images.insert(asset, texture);
                }
                let change = aurora_net::inventory::thumbnail::patch(*item, *folder, *parent, asset);
                let result = crate::world::inventory::actions::demo_mutate(&mut self.world.inventory, self.world.agent_id, change);
                self.on_app_event(NetEvent::InventoryEdited { request: *request, result });
                return;
            }
            if let NetCommand::CopyInventoryItems(copies) = &cmd {
                let items = copies
                    .iter()
                    .filter_map(|c| {
                        self.world.inventory.items.get(&c.item).cloned().map(|mut it| {
                            it.id = uuid::Uuid::new_v4();
                            it.parent = c.parent;
                            it
                        })
                    })
                    .collect();
                self.world.inventory.add_items(items);
                return;
            }
            if let NetCommand::FinishInventoryCopies { request, reply } = &cmd {
                let items = reply
                    .copies
                    .iter()
                    .filter_map(|c| {
                        self.world.inventory.items.get(&c.item).cloned().map(|mut it| {
                            it.id = uuid::Uuid::new_v4();
                            it.parent = c.parent;
                            it
                        })
                    })
                    .collect();
                self.world.inventory.add_items(items);
                let change = aurora_net::inventory::operations::Mutation {
                    refresh: reply.refresh.clone(),
                    ..Default::default()
                };
                let result = crate::world::inventory::actions::demo_mutate(&mut self.world.inventory, self.world.agent_id, change).map(
                    |mut completed| {
                        completed.created.clone_from(&reply.created);
                        completed
                    },
                );
                self.on_app_event(NetEvent::InventoryEdited { request: *request, result });
                return;
            }
            if let NetCommand::CreateInventoryWearable { parent, kind, name, .. } = &cmd {
                self.send(NetCommand::CreateInventoryItem {
                    parent: *parent,
                    kind: aurora_net::inventory::operations::NewItem::Wearable(*kind),
                    name: name.clone(),
                });
                return;
            }
            if let NetCommand::SaveInventoryContent { request, item, data, .. } = &cmd {
                if std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| v == "note-error") {
                    self.on_app_event(NetEvent::InventoryContentSaved {
                        request: *request,
                        item: *item,
                        result: Err(aurora_i18n::tr!("notecard-demo-save-error").into()),
                    });
                    return;
                }
                let result = self
                    .world
                    .inventory
                    .items
                    .get(item)
                    .cloned()
                    .map(|mut it| {
                        it.asset_id = uuid::Uuid::new_v4();
                        self.demo_documents.insert(it.id, data.clone());
                        it
                    })
                    .ok_or_else(|| aurora_i18n::tr!("common-item-not-found").to_owned());
                // a saved settings item's new asset, as the simulator would serve it
                if let Ok(it) = &result
                    && it.asset_type == 56
                    && let Some(map) = crate::world::eep::Settings::decode_asset(data)
                    && let Some(s) = crate::world::eep::Settings::from_llsd(&map)
                {
                    self.scene.settings.insert_local(it.asset_id, s, Some(map));
                }
                self.on_app_event(NetEvent::InventoryContentSaved {
                    request: *request,
                    item: *item,
                    result,
                });
                return;
            }
            if let NetCommand::CreateSettingsItem {
                request,
                parent,
                name,
                subtype,
                next_owner_mask,
                data,
            } = &cmd
            {
                let item = aurora_net::inventory::InvItem {
                    id: uuid::Uuid::new_v4(),
                    parent: *parent,
                    name: name.clone(),
                    asset_type: 56,
                    inv_type: 25,
                    asset_id: uuid::Uuid::new_v4(),
                    flags: u32::from(*subtype),
                    owner: self.world.agent_id,
                    creator: self.world.agent_id,
                    base_mask: 0x7fffffff,
                    owner_mask: 0x7fffffff,
                    everyone_mask: 0x8000,
                    next_owner_mask: next_owner_mask.unwrap_or(0x86000),
                    ..Default::default()
                };
                self.demo_documents.insert(item.id, data.clone());
                if let Some(map) = crate::world::eep::Settings::decode_asset(data)
                    && let Some(s) = crate::world::eep::Settings::from_llsd(&map)
                {
                    self.scene.settings.insert_local(item.asset_id, s, Some(map));
                }
                self.world.inventory.add_items(vec![item.clone()]);
                self.on_app_event(NetEvent::InventoryContentSaved {
                    request: *request,
                    item: item.id,
                    result: Ok(item),
                });
                return;
            }
            if let NetCommand::CopyNotecardInventory {
                editor,
                notecard,
                item,
                open,
                ..
            } = &cmd
            {
                let mut copied = item.as_ref().clone();
                copied.id = uuid::Uuid::new_v4();
                copied.parent =
                    crate::world::inventory::actions::system(&self.world.inventory, copied.asset_type).unwrap_or(self.world.inventory.root);
                copied.owner = self.world.agent_id;
                self.notecard_inventory_copied(*editor, *notecard, *open, Ok(copied));
                return;
            }
            if let NetCommand::PreviewTaskNotecard { object, item, .. } = &cmd {
                let data = self.demo_documents.get(&item.id).cloned().unwrap_or_else(|| {
                    crate::world::inventory::demo::note_asset(
                        std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| v == "note-task-embedded"),
                    )
                });
                self.task_notecard_preview(*object, item.id, item.asset_id, Ok(data));
                return;
            }
            if let NetCommand::SaveTaskNotecard {
                request,
                object,
                item,
                data,
                ..
            } = &cmd
            {
                let result = if std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| v == "note-task-error") {
                    Err(aurora_i18n::tr!("notecard-demo-save-error").into())
                } else {
                    self.demo_documents.insert(*item, data.clone());
                    Ok(uuid::Uuid::new_v4())
                };
                self.task_notecard_saved(*request, *object, *item, result);
                return;
            }
            if let NetCommand::CreateInventoryItem { parent, kind, name } = &cmd {
                let (asset, inventory, subtype) = kind.types();
                // a landmark's asset is written by the server from where the
                // agent stands (the demo region has no id: a version 1
                // landmark, with the global position)
                let asset_id = match (kind, self.world.agent_global()) {
                    (aurora_net::inventory::operations::NewItem::Landmark, Some(g)) => {
                        let id = uuid::Uuid::new_v4();
                        let text = format!("Landmark version 1\nposition {} {} {}\n", g.x, g.y, g.z);
                        self.world.landmarks.on_data(id, Some(text.as_bytes()));
                        id
                    }
                    _ => uuid::Uuid::nil(),
                };
                let item = aurora_net::inventory::InvItem {
                    asset_id,
                    id: uuid::Uuid::new_v4(),
                    parent: *parent,
                    name: name.clone(),
                    asset_type: asset as i32,
                    inv_type: inventory as i32,
                    flags: subtype as u32,
                    owner: self.world.agent_id,
                    creator: self.world.agent_id,
                    base_mask: 0x7fffffff,
                    owner_mask: 0x7fffffff,
                    next_owner_mask: 0x7fffffff,
                    ..Default::default()
                };
                self.on_app_event(NetEvent::InventoryCreated(item));
                return;
            }
            if let NetCommand::PreviewInventoryItem(it) = &cmd {
                let library = (it.asset_type == 56).then(|| crate::demo::env::asset_bytes(it.asset_id)).flatten();
                let bytes = self.demo_documents.get(&it.id).cloned().or(library).unwrap_or_else(|| match it.asset_type {
                    56 => b"<?llsd/notation?>\n{'type':'sky'}".to_vec(),
                    57 => aurora_llsd::to_binary(
                        &aurora_llsd::llsd_map! { "version" => "1.1", "type" => "GLTF 2.0", "data" => "{\"asset\":{\"version\":\"2.0\"},\"materials\":[{}]}" },
                    ),
                    7 => crate::world::inventory::demo::note_asset(
                        std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| matches!(v.as_str(), "note-embedded" | "note-image" | "note-landmark" | "note-copy" | "note-menu-right")),
                    ),
                    _ => aurora_i18n::tr!("app-demo-content", value0 = it.name.to_string()).into_bytes(),
                });
                self.inventory_preview(it.id, it.asset_id, Ok(bytes));
                return;
            }
            if let NetCommand::UpdateOutfit { request, change } = &cmd {
                let result = crate::world::appearance::demo_mutate(&mut self.world.inventory, self.world.agent_id, change);
                self.on_app_event(NetEvent::OutfitUpdated { request: *request, result });
                return;
            }
            if let NetCommand::SetInventoryFavorite { item, favorite } = &cmd {
                let result = self
                    .world
                    .inventory
                    .items
                    .get(item)
                    .cloned()
                    .map(|mut it| {
                        it.favorite = *favorite;
                        it
                    })
                    .ok_or_else(|| aurora_i18n::tr!("common-item-not-found").into());
                self.on_app_event(NetEvent::InventoryFavoriteUpdated { item: *item, result });
                return;
            }
            if let NetCommand::UpdateOutfitCategory { request, change } = &cmd {
                let result = crate::world::appearance::demo_category(&mut self.world.inventory, self.world.agent_id, change);
                self.on_app_event(NetEvent::OutfitUpdated { request: *request, result });
                return;
            }
            if let NetCommand::RezAttachments(items) = &cmd {
                self.world.apply(NetEvent::ObjectUpdates {
                    handle: self.world.main_region.unwrap_or_default(),
                    objects: items.iter().map(crate::demo::outfit_attachment).collect(),
                });
                return;
            }
            if let NetCommand::DetachAttachments(items) = &cmd {
                let local_ids = items
                    .iter()
                    .filter_map(|id| self.world.objects.index_of_uuid(id))
                    .filter_map(|idx| self.world.objects.get(idx))
                    .map(|o| o.key.local_id)
                    .collect();
                self.world.apply(NetEvent::ObjectsKilled {
                    handle: self.world.main_region.unwrap_or_default(),
                    local_ids,
                });
                return;
            }
            // the sample offer of the notification center is "already in the
            // inventory", like a resident's offer on the grid
            if let NetCommand::FetchItems { items, owner } = &cmd
                && items.contains(&crate::demo::DEMO_OFFER)
            {
                let parent = crate::world::inventory::actions::system(&self.world.inventory, 6).unwrap_or(self.world.inventory.root);
                self.world.inventory.add_items(vec![crate::demo::offered_item(parent, *owner)]);
                return;
            }
            // the demo's dataserver creates the group and takes its fee from
            // the balance, or refuses it (name taken, L$ or group limit)
            if let NetCommand::CreateGroup { name, info } = &cmd {
                let fee = ui::group_profile::create::fee(&self.world);
                let can_join = ui::group_profile::create::can_join_groups(&self.world);
                let balance = self.world.balance.unwrap_or(250);
                for ev in crate::demo::group::create(name, info, balance, fee, can_join) {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
                return;
            }
            // a new classified is paid when it is published, as the
            // simulator does on the grid
            if let Some(price) = crate::demo::classifieds::price_of_new(&cmd)
                && let Some(balance) = self.world.balance
            {
                self.world.apply(NetEvent::Balance(balance - price));
            }
            for mut ev in crate::demo::demo_reply(&cmd) {
                if matches!(cmd, NetCommand::ObjectRelease { .. }) {
                    crate::demo::hud::preserve_touch_transform(&self.world, &mut ev);
                }
                if let NetEvent::InventoryContents(contents) = &mut ev {
                    for c in contents {
                        // the synthetic listing of its folder keeps the offered item
                        if let Some(offer) = self.world.inventory.items.get(&crate::demo::DEMO_OFFER)
                            && offer.parent == c.folder_id
                            && c.items.iter().all(|it| it.id != offer.id)
                        {
                            c.items.push(offer.clone());
                        }
                        if let Some(folder) = self.world.inventory.folders.get(&c.folder_id)
                            && matches!(folder.info.type_default, 46..=48)
                        {
                            c.version = folder.info.version;
                            c.folders = folder
                                .children
                                .iter()
                                .filter_map(|id| self.world.inventory.folders.get(id).map(|f| f.info.clone()))
                                .collect();
                            c.items = folder
                                .items
                                .iter()
                                .filter_map(|id| self.world.inventory.items.get(id).cloned())
                                .collect();
                        }
                    }
                }
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
            }
            return;
        }
        self.net.send(cmd);
    }

    fn in_world(&self) -> bool {
        matches!(self.screen, Screen::World | Screen::Loading { .. })
    }

    /// The avatar name tags are the classic ones, not the name plates: the
    /// setting, unless AURORA_NAME_TAGS forces a style.
    fn classic_name_tags(&self) -> bool {
        match name_tags_env() {
            Some(NameTagsEnv::Classic) => true,
            Some(NameTagsEnv::Plate) => false,
            Some(NameTagsEnv::Alternate) => self.name_tags_alt_classic,
            None => self.settings.classic_name_tags,
        }
    }

    /// The conversation transcripts follow the preferences, and know before
    /// the login which grid it is made on (the demo is a grid of its own).
    fn sync_chat_log(&mut self) {
        let s = &self.settings;
        let log = &mut self.world.chat_log;
        log.cfg = crate::world::chat_log::Config {
            log_im: s.log_im,
            log_nearby: s.log_nearby,
            show_history: s.log_show_history,
            dated_names: s.log_dated_names,
        };
        // the demo keeps no transcripts unless its scenario asks for them:
        // a run must not change what the next one shows
        let off = self.demo && !matches!(crate::demo::chatlog::mode(), Some(m) if !matches!(m, crate::demo::chatlog::Mode::Long { .. }));
        log.set_grid(if off {
            String::new()
        } else {
            crate::world::chat_log::grid_name(&s.grid, &s.custom_login_uri, self.demo)
        });
    }

    // ------------------------------------------------------------------ login

    fn start_login(&mut self) {
        if self.demo {
            let mode = std::env::var("AURORA_DEMO_LOGIN").unwrap_or_default();
            // Which identity the offline server received: the account being
            // added, or a saved card (never the name itself).
            let saved = self
                .login_form
                .accounts
                .iter()
                .any(|account| account.key() == (self.settings.login_uri(), self.settings.username.trim().to_lowercase()));
            log::info!(
                "demo login: request for {} (adding {}, code {})",
                if saved { "a saved account" } else { "a new account" },
                self.login_form.adding,
                !self.login_form.mfa_token.trim().is_empty()
            );
            if matches!(mode.as_str(), "derez" | "derez1" | "connecting") {
                self.login_form.busy = true;
                self.login_progress = Some((aurora_i18n::tr!("session-connecting-to-login-server").into(), 0.35));
            } else if mode == "add-mfa" && self.login_form.mfa_token.trim().is_empty() {
                self.login_form.busy = true;
                self.login_progress = Some((aurora_i18n::tr!("session-connecting-to-login-server").into(), 0.35));
                self.demo_mfa_challenge = Some(std::time::Instant::now() + std::time::Duration::from_millis(1200));
            } else {
                self.login_form.transition = None;
                self.login_form.error = Some(aurora_i18n::tr!("app-demo-mode-no-grid-connection").into());
            }
            return;
        }
        let start = match self.settings.start_location.as_str() {
            "home" => StartLocation::Home,
            "region" if !self.settings.start_region.trim().is_empty() => StartLocation::Region {
                name: self.settings.start_region.trim().to_owned(),
                x: 128,
                y: 128,
                z: 30,
            },
            _ => StartLocation::Last,
        };
        let (mac, id0) = self.settings.hashed_ids();
        // typed password first, else the remembered one (already hashed)
        let password = match (&self.login_form.stored, self.login_form.password.is_empty()) {
            (Some(h), true) => h.clone(),
            _ => self.login_form.password.clone(),
        };
        let req = LoginRequest {
            login_uri: self.settings.login_uri(),
            username: self.settings.username.trim().to_owned(),
            password,
            start,
            agree_to_tos: self.login_form.agree_tos,
            read_critical: self.login_form.agree_tos,
            mfa_token: self.login_form.mfa_token.trim().to_owned(),
            mfa_hash: self
                .login_form
                .mfa_authorization
                .as_ref()
                .filter(|authorization| self.login_form.remember_mfa && authorization.valid_at(crate::credentials::now()))
                .map_or_else(String::new, |authorization| authorization.hash.clone()),
            mac,
            id0,
        };
        self.login_form.busy = true;
        self.login_form.error = None;
        self.login_progress = Some((aurora_i18n::tr!("app-connecting").into(), 0.05));
        self.settings.save();
        self.net.send(NetCommand::Login(Box::new(req)));
    }

    fn back_to_login(&mut self, error: Option<String>) {
        self.sync_local_picker_bindings();
        self.imports.flush_local_registry();
        for entry in &self.imports.entries {
            if let Some(crate::imports::Source::Mesh(draft)) = &entry.source {
                draft.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let imports = std::mem::take(&mut self.imports);
        if let Some(gfx) = &mut self.gfx {
            let mut slots = HashSet::new();
            for entry in &imports.entries {
                for asset in &entry.local_mesh_ids {
                    self.scene.meshes.remove_local(asset);
                }
                if entry.local_published {
                    self.scene.materials.remove_local(&entry.id);
                }
                for (asset, slot) in &entry.local_slots {
                    self.scene.textures.unregister_local(asset);
                    if slots.insert(*slot) {
                        gfx.renderer.free_texture(*slot);
                    }
                }
            }
        }
        self.scene.jobs.discard(imports);
        self.record_inventory_logout();
        self.inventory_ui = Default::default();
        self.gesture_state = Default::default();
        self.buy_currency_ui = Default::default();
        // the next session signs in with its own cookie (the browser stops)
        self.marketplace_ui = Default::default();
        // the listings belong to the account (LLMarketplaceData)
        self.listings_ui = Default::default();
        self.demo_slm = None;
        self.task_notecards.clear();
        self.script_editors.disconnected();
        self.demo_script_sources.clear();
        self.hud_drag = None;
        self.media.clear();
        self.media.openid.clear();
        // the search window's browser and its page, opened with the session
        // that ends here
        self.search_ui = Default::default();
        self.panels.search = false;
        self.save_local_environment();
        self.env_ui = Default::default();
        self.env_scan = false;
        self.panels.environment = false;
        self.panels.personal_lighting = false;
        self.env_editor.close();
        self.env_import = None;
        self.day_editor.close();
        self.day_fetch = None;
        self.day_import = None;
        self.env_applying = None;
        self.env_beacons_frame();
        self.world.eep.clear_edit();
        self.world.save_inventory_cache();
        self.world.save_mute_cache();
        if let Some(path) = self.name_cache_path() {
            self.world.social.avatar_names.save(&path);
        }
        self.scene.sounds.clear(self.audio_engine.as_ref());
        if let Some(g) = &mut self.gfx {
            self.scene.clear(&mut g.renderer);
        }
        self.world.reset();
        // the transcripts shown belong to the account that leaves
        self.history_ui = Default::default();
        self.chat_ui = ChatUi::default();
        self.ao_agent = uuid::Uuid::nil();
        self.appearance_ui = Default::default();
        self.shape_ui = Default::default();
        self.shape_camera = None;
        self.interactions = Default::default();
        self.screen = Screen::Login;
        self.login_form.busy = false;
        self.login_form.transition = None;
        self.login_form.pending_connect = None;
        self.login_form.focused = false;
        self.login_form.error = error;
        self.login_form.reconnect = false;
        self.login_progress = None;
        // the alert belonged to the session (the demo's scenario stays played)
        let demo = self.region_restart.take_demo();
        self.region_restart = region_restart::State::default();
        self.region_restart.put_demo(demo);
        self.move_complete_at = None;
        self.static_eyes_sent = None;
        self.set_mouselook_grab(false);
        self.camera = Camera::default();
    }

    fn on_app_event(&mut self, ev: NetEvent) {
        if let NetEvent::MoneyTransaction {
            success,
            kind,
            dest,
            ref description,
            ..
        } = ev
        {
            self.pay_ui.on_transaction(success, kind, dest, description);
            return;
        }
        if let NetEvent::Balance(balance) = ev {
            self.pay_ui.on_balance(balance);
        }
        if let NetEvent::GroupCreated { group, success, message } = &ev {
            self.group_profile_ui.on_created(&mut self.world, *group, *success, message);
            return;
        }
        if let NetEvent::ScriptTeleportRequest { .. } = ev {
            // llMapDestination asked for the map (BEACON_SHOW_MAP)
            self.panels.world_map = true;
            return;
        }
        if let NetEvent::UploadCost(cost) = ev {
            self.world.upload_cost = Some(cost);
            return;
        }
        if let NetEvent::MeshQuoted { request, result } = ev {
            self.imports.mesh_quoted(request, result);
            return;
        }
        if let NetEvent::AssetUploaded { request, result } = ev {
            if !self.demo {
                self.send(NetCommand::RequestBalance);
            }
            if self.photo_owns_upload(request) {
                self.photo_uploaded(result);
                return;
            }
            if let Some(item) = self.imports.uploaded(request, result) {
                let id = item.id;
                self.world.inventory.add_items(vec![item]);
                self.inventory_ui.show_original(&self.world.inventory, id);
            }
            return;
        }
        if let NetEvent::InventoryCreated(item) = ev {
            let gesture = item.asset_type == 21;
            let id = item.id;
            if self.create_landmark_ui.created(id, item.parent, item.asset_type) {
                // the Create Landmark window edits it: no rename in the inventory
                self.world.inventory.add_items(vec![item]);
                return;
            }
            self.world.inventory.add_items(vec![item]);
            self.inventory_ui.show_original(&self.world.inventory, id);
            self.inventory_ui.begin_rename(&self.world.inventory, id);
            if gesture && self.gesture_state.new_editor {
                self.gesture_state.new_editor = false;
                self.gesture_action(ui::gestures::Action::Open(id));
            }
            return;
        }
        if self.marketplace_listings_event(&ev) {
            return;
        }
        if let NetEvent::InventoryMerchant(result) = ev {
            match result {
                Ok(merchant) => self.inventory_ui.merchant = merchant,
                Err(reason) => self.inventory_ui.message = reason,
            }
            return;
        }
        if let NetEvent::InventoryContentSaved { request, item, result } = ev {
            if self.env_editor.saving == Some(request) {
                self.env_editor_saved(result);
                return;
            }
            if self.day_editor.saving == Some(request) {
                self.day_editor_saved(result);
                return;
            }
            if self.gesture_save_pending(request, item) {
                self.gesture_saved(request, item, result);
                return;
            }
            self.inventory_content_saved(request, item, result);
            return;
        }
        if let NetEvent::NotecardCopySubmitted { editor, notecard, result } = ev {
            self.notecard_copy_submitted(editor, notecard, result);
            return;
        }
        if let NetEvent::NotecardInventoryCopied {
            editor,
            notecard,
            open,
            result,
        } = ev
        {
            self.notecard_inventory_copied(editor, notecard, open, result);
            return;
        }
        if let NetEvent::InventoryOperationFailed(reason) = ev {
            self.gesture_state.new_editor = false;
            self.inventory_ui.message = reason;
            return;
        }
        if let NetEvent::InventoryProgress { request, progress } = ev {
            self.inventory_ui.update_mutation_progress(request, progress);
            return;
        }
        if let NetEvent::InventoryEdited { request, result } = ev {
            if self.gesture_metadata_result(request, &result) {
                return;
            }
            if self.build.replacement_inventory_result(request, &result) {
                return;
            }
            if self.create_landmark_ui.edits.remove(&request) {
                match result {
                    Ok(reply) => {
                        self.create_landmark_ui.folder_created(&reply.created);
                        self.world.inventory.remove(&reply.removed);
                        self.world.inventory.apply(reply.contents);
                        self.world.inventory.created_folders(reply.folders);
                    }
                    Err(reason) => log::warn!("landmark change not saved: {reason}"),
                }
                return;
            }
            if self.world_edits.remove(&request) {
                match result {
                    Ok(reply) => {
                        self.world.inventory.remove(&reply.removed);
                        self.world.inventory.apply(reply.contents);
                    }
                    Err(reason) => log::warn!("refused inventory offer not moved to the Trash: {reason}"),
                }
                return;
            }
            if self.ao_ui.pending.as_ref().is_some_and(|p| p.request == request) {
                self.ao_inventory_result(result);
                return;
            }
            self.inventory_result(request, result);
            return;
        }
        if let NetEvent::InventoryPreview { item, asset, result } = ev {
            if self.shape_ui.loaded(&mut self.world, item, asset, &result) {
                return;
            }
            if self.ao_ui.import == Some(item) {
                // A shared download can serve both an AO import and an editor.
                self.ao_import_result(item, result.clone());
            }
            self.inventory_preview(item, asset, result);
            return;
        }
        if let NetEvent::GestureLoaded { item, asset, result } = ev {
            self.gesture_loaded(item, asset, result);
            return;
        }
        if let NetEvent::WearableSaved { request, result } = ev {
            if self.shape_ui.draft.as_ref().is_none_or(|draft| draft.pending != Some(request)) {
                return;
            }
            let saved = result.as_ref().ok().map(|item| item.id);
            let copy = self.shape_ui.saved(&mut self.world, request, result);
            if let Some(item) = copy.or(saved) {
                self.apply_appearance_action(crate::world::appearance::Action::WearItem {
                    item,
                    replace: true,
                    point: 0,
                });
            }
            return;
        }
        if let NetEvent::TaskNotecardPreview {
            object,
            item,
            asset,
            result,
        } = ev
        {
            self.task_notecard_preview(object, item, asset, result);
            return;
        }
        if let NetEvent::TaskNotecardSaved {
            request,
            object,
            item,
            result,
        } = ev
        {
            self.task_notecard_saved(request, object, item, result);
            return;
        }
        if let NetEvent::ScriptLoaded { request, target, result } = ev {
            self.script_editors.loaded(request, target, result);
            return;
        }
        if let NetEvent::ScriptExperiences { request, target, result } = ev {
            self.script_editors.experiences_loaded(request, target, result);
            return;
        }
        if let NetEvent::ScriptSaved { request, target, result } = ev {
            self.script_saved(request, target, result);
            return;
        }
        if let NetEvent::InventoryThumbnailSource {
            request,
            asset,
            resize,
            result,
        } = ev
        {
            self.inventory_image_source(request, asset, resize, result);
            return;
        }
        if let NetEvent::InventoryFavoriteUpdated { item, result } = ev {
            if self.appearance_ui.favorite_pending.remove(&item) {
                match result {
                    Ok(it) => {
                        self.world.inventory.add_items(vec![it]);
                        self.appearance_ui.message.clear();
                    }
                    Err(reason) => self.appearance_ui.message = reason,
                }
            }
            return;
        }
        if let NetEvent::OutfitUpdated { request, result } = ev {
            if self.appearance_ui.pending.is_some_and(|(id, _)| id == request) {
                let sync = self.appearance_ui.pending.take().is_some_and(|(_, sync)| sync);
                match result {
                    Ok(contents) => {
                        self.world.inventory.apply(contents);
                        self.appearance_ui.message.clear();
                        self.appearance_ui.save_as = None;
                        if self.appearance_ui.saved_new {
                            self.appearance_ui.open(1, false);
                            self.appearance_ui.saved_new = false;
                        }
                        if sync {
                            let mut commands = crate::world::appearance::sync_commands(
                                &self.world.inventory,
                                self.world.agent_id,
                                &self.world.worn_attachment_items(),
                            );
                            for attachment in self.appearance_ui.attachments.drain(..) {
                                crate::world::appearance::apply_attachment_override(&mut commands, attachment);
                            }
                            for cmd in commands {
                                self.send(cmd);
                            }
                        }
                    }
                    Err(reason) => {
                        self.appearance_ui.attachments.clear();
                        self.inventory_ui.message.clone_from(&reason);
                        self.appearance_ui.message = reason;
                        self.appearance_ui.refresh(&mut self.world.inventory);
                    }
                }
            }
            return;
        }
        match ev {
            NetEvent::ObjectProperties(props) => {
                self.report_abuse_properties(&props);
                self.world.on_hud_properties(&props);
                self.interactions.on_properties(&props);
                self.build.on_properties(props);
            }
            NetEvent::PayPrice { object, default, buttons } => self.interactions.on_prices(object, default, buttons),
            NetEvent::AbuseCategories(list) => self.report_abuse.set_categories(list),
            NetEvent::AbuseReportSent(result) => self.report_abuse_sent(result),
            NetEvent::TaskInventory { object, serial, result } => {
                self.interactions.on_purchase_inventory(object, &result);
                let items = result
                    .as_ref()
                    .map(|v| v.iter().filter_map(|i| i.to_inv_item()).collect())
                    .map_err(|e| e.clone());
                self.interactions.on_contents(object, items);
                if let Ok(items) = result {
                    self.build.on_task_inventory(&self.world, object, serial, items);
                    self.flush_build();
                } else if let Err(reason) = result {
                    self.build.replacement_task_inventory_error(object, reason);
                }
            }
            NetEvent::CapReply { tag, result } => self.build.on_cap_reply(&mut self.world, tag, result),
            NetEvent::ScriptRunning {
                object_id,
                item_id,
                running,
                mono,
            } => {
                self.build.contents.running.insert((object_id, item_id), (running, mono));
                self.script_editors.running(
                    aurora_net::scripts::ScriptTarget {
                        object: object_id,
                        item: item_id,
                    },
                    running,
                    mono,
                );
            }
            NetEvent::PhysicsProperties(list) => {
                for (local_id, params) in list {
                    self.build.physics.insert(local_id, params);
                    self.build.physics_materials_known.insert(local_id);
                }
            }
            NetEvent::SelectedParcel { handle, sequence, parcel } => self.build.on_selected_parcel(handle, sequence, parcel),
            NetEvent::LoginProgress { message, fraction } => {
                self.loading_stage = message.clone();
                self.login_progress = Some((message, fraction));
            }
            NetEvent::LoginFailed {
                error,
                mfa_required,
                tos_required,
                bad_credentials,
            } => {
                self.login_form.busy = false;
                self.login_form.transition = None;
                self.login_form.focused = false;
                self.login_progress = None;
                // wrong name / password: do not try the remembered one again
                if bad_credentials && self.login_form.password.is_empty() && self.login_form.stored.take().is_some() {
                    crate::credentials::forget(&self.settings.login_uri(), &self.settings.username);
                    let key = (self.settings.login_uri(), self.settings.username.trim().to_lowercase());
                    if let Some(mut account) = self.login_form.accounts.iter().find(|account| account.key() == key).cloned() {
                        account.remember_password = false;
                        self.login_form.accounts = crate::login_accounts::save(account);
                    }
                }
                if mfa_required {
                    // A server challenge invalidates this account's authorization only.
                    self.login_form.mfa_authorization = None;
                    self.login_form.mfa_token.clear();
                    if !self.demo {
                        crate::credentials::forget_mfa(&self.settings.login_uri(), &self.settings.username);
                    }
                    self.login_form.show_mfa = true;
                    self.login_form.error = Some(aurora_i18n::tr!("app-enter-the-code-from-your-authenticator-app").into());
                } else if tos_required {
                    self.login_form.tos_message = Some(error);
                } else {
                    self.login_form.error = Some(error);
                }
            }
            NetEvent::LoggedIn(l) => {
                self.world.notification_history = if self.demo {
                    Default::default()
                } else {
                    crate::world::notification_history::History::load(
                        crate::settings::account_dir(&l.agent_id).join(crate::world::notification_history::FILE),
                    )
                };
                self.world.hover_height = if self.demo {
                    Default::default()
                } else {
                    crate::world::hover_height::HoverHeight::load(
                        crate::settings::account_dir(&l.agent_id).join(crate::world::hover_height::FILE),
                    )
                };
                self.world.favorites = if self.demo {
                    Default::default()
                } else {
                    crate::world::favorites::Favorites::load(crate::settings::account_dir(&l.agent_id).join(crate::world::favorites::FILE))
                };
                self.restore_gestures(&l.raw);
                self.net_profile.reset();
                self.login_form.transition = None;
                self.login_form.busy = false;
                self.login_form.adding = false;
                self.login_form.add_return = None;
                self.login_form.editing = false;
                // the demo's teleport history: several days, never saved
                if self.demo {
                    self.world.tp_storage = crate::world::tphistory::HistoryStorage::in_memory(crate::demo::place::history());
                }
                // web profiles know who we are (LLStartup: LLViewerMedia::openIDSetup)
                let (openid_url, openid_token) = (l.raw["openid_url"].as_str(), l.raw["openid_token"].as_str());
                if !openid_url.is_empty() && !openid_token.is_empty() {
                    self.media.openid.start(
                        self.net.runtime(),
                        self.net.caps_http(),
                        openid_url.to_owned(),
                        openid_token.to_owned(),
                    );
                }
                // interface sounds ready when needed (init_audio preloads)
                for id in self.settings.audio.ui.preload() {
                    self.scene.sounds.want(id);
                }
                // remembered password: the hash of the one typed (kept when
                // the remembered one was used); otherwise forgotten
                let (uri, user) = (self.settings.login_uri(), self.settings.username.clone());
                if self.settings.remember_password {
                    if !self.login_form.password.is_empty() {
                        crate::credentials::store(&uri, &user, &aurora_net::login::login_hash(&uri, &self.login_form.password));
                    }
                } else {
                    crate::credentials::forget(&uri, &user);
                }
                if !self.demo && self.settings.remember_username {
                    self.login_form.accounts = crate::login_accounts::save(crate::login_accounts::Account::from_settings(
                        &self.settings,
                        self.login_form.remember_mfa,
                        Some(l.agent_id),
                    ));
                }
                if !self.demo {
                    // An explicit MFA opt-in is independent of remembering the login name/password.
                    let previous = self
                        .login_form
                        .mfa_authorization
                        .as_ref()
                        .filter(|_| self.login_form.mfa_token.is_empty());
                    self.login_form.mfa_authorization =
                        crate::credentials::complete_mfa(&uri, &user, self.login_form.remember_mfa, l.mfa_hash.as_deref(), previous);
                }
                // the typed password is not kept in memory
                self.login_form.password.clear();
                self.login_form.stored = None;
                self.login_form.stored_for = None;
                self.login_form.password.clear();
                self.login_form.mfa_token.clear();
                self.login_form.agree_tos = false;
                self.settings.mfa_hash.clear();
                if let Some(path) = self.name_cache_path() {
                    self.world.social.avatar_names.load(&path);
                }
                self.screen = Screen::Loading { since: Instant::now() };
                self.world.notifications.started = false;
                self.loading_frac = 0.0;
                self.loading_target = 0.0;
                self.loading_stage = aurora_i18n::tr!("common-connecting-to-region").into();
                if !l.message.trim().is_empty() {
                    self.world.system_message(l.message.trim().to_owned());
                }
                self.world.system_message(aurora_i18n::tr!(
                    "app-welcome-2",
                    value0 = l.first_name.to_string(),
                    value1 = l.last_name.to_string()
                ));
                self.restore_local_environment();
            }
            NetEvent::TeleportFinished { .. } => {
                if self.world.tp_show_progress && matches!(self.screen, Screen::World) && !self.was_teleporting {
                    // Finish and arrival can be drained in the same frame.
                    // Start the screen here so a fast remote teleport still
                    // gets its region-loading phase and arrival fade.
                    self.show_teleport_progress();
                }
            }
            NetEvent::AgentMovementComplete { .. } => {
                self.move_complete_at = Some(Instant::now());
                self.tp_checks = vec![10.0, 3.0];
                // (a border crossing leaves the camera where it is)
                if !self.world.arrived_by_crossing {
                    self.camera.on_teleport_arrival(&self.settings.camera);
                }
                self.scene.textures.mark_arrival();
                self.loading_stage = aurora_i18n::tr!("app-loading-scene").into();
            }
            NetEvent::SitResponse {
                object,
                camera_eye,
                camera_at,
                force_mouselook,
            } => self.camera.on_sit_response(object, camera_eye, camera_at, force_mouselook),
            NetEvent::CameraConstraint(plane) => self.camera.set_collide_plane(plane, &self.settings.camera),
            NetEvent::RegionRestart(notice) => self.on_region_restart(notice),
            NetEvent::RegionRestartCancelled => self.on_region_restart_cancelled(),
            NetEvent::Disconnected { reason } => {
                // cut by the region's restart: say so, and offer to reconnect
                let restart = self.restart_disconnect();
                let reconnect = restart.is_some();
                self.back_to_login(Some(
                    restart.unwrap_or_else(|| aurora_i18n::tr!("app-disconnected", reason = reason.to_string())),
                ));
                self.login_form.reconnect = reconnect;
            }
            // ---- sounds (process_sound_trigger, setAttachedSound)
            NetEvent::SoundTrigger {
                sound,
                owner,
                object,
                parent,
                handle,
                position,
                gain,
            } => {
                if self.world.sound_blocked(&object, &owner) || (!parent.is_nil() && self.world.sound_blocked(&parent, &uuid::Uuid::nil()))
                {
                    return;
                }
                let a = &self.settings.audio;
                if !a.collision_sounds && crate::scene::sounds::is_collision(&sound) {
                    return;
                }
                // gestures: sounds an avatar triggers (not our own)
                if object == owner && owner != self.world.agent_id && !a.gesture_sounds {
                    return;
                }
                if let Some(off) = self.world.region_offset(handle) {
                    self.scene.sounds.trigger(sound, off + position, None, gain);
                }
            }
            NetEvent::AttachedSound {
                object,
                sound,
                owner,
                gain,
                flags,
            } => {
                self.flush_object_sounds();
                if self.world.sound_blocked(&object, &owner) {
                    return;
                }
                self.scene
                    .sounds
                    .set_attached(self.audio_engine.as_ref(), object, sound, gain, flags);
            }
            NetEvent::AttachedSoundGain { object, gain } => {
                self.flush_object_sounds();
                self.scene.sounds.set_gain(object, gain);
            }
            NetEvent::PreloadSounds(ids) => {
                for id in ids {
                    self.scene.sounds.want(id);
                }
            }
            NetEvent::LoggedOut => {
                self.back_to_login(None);
            }
            // both windows estimate L$: the one that asked takes the answer
            NetEvent::CurrencyQuote { amount, result } => {
                if !self.buy_land_ui.quote_reply(amount, &result) {
                    self.buy_currency_ui.quote_reply(amount, result);
                }
            }
            NetEvent::LandToolPreflight {
                billable_area,
                currency_buy,
                result,
            } => self.buy_land_ui.preflight_reply(billable_area, currency_buy, result),
            NetEvent::LandToolPrepared(result) => self.land_tool_prepared(result),
            NetEvent::CurrencyBought(result) => self.currency_bought(result),
            // LLSetDisplayNameReply + LLFloaterDisplayName::onCacheSetName
            NetEvent::SetDisplayNameReply { status, reason, content } => {
                log::info!("display name change: {status} {reason}");
                // our name was out of date (409 Conflict): fetch it again
                if status == 409 {
                    let me = self.world.agent_id;
                    self.world.social.avatar_names.refetch(&me);
                }
                let (kind, title, body) = match ui::display_name::reply_message(status, &content) {
                    Ok(m) => (
                        crate::world::notifications::Kind::Info,
                        aurora_i18n::tr!("app-display-name-changed"),
                        m,
                    ),
                    Err(m) => (crate::world::notifications::Kind::Alert, aurora_i18n::tr!("common-display-name"), m),
                };
                self.world
                    .notifications
                    .push(kind, title, body, crate::world::notifications::Data::None);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ input

    /// LLTracker::handleMouseDown: only the visible destination marker
    /// takes the press. UI, worn HUDs and camera modifiers keep priority.
    fn beacon_hover(&self) -> bool {
        let ppp = self.egui_ctx.pixels_per_point();
        let at = egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp);
        matches!(self.screen, Screen::World)
            && !self.camera.mouselook()
            && !self.alt
            && !self.ctrl
            && !self.shift
            && !self.color_pipette()
            && !self.exit_prompt.open
            && !self.cursor_hold.held()
            && self.mouse_mode == MouseMode::None
            && self
                .beacon_hit
                .as_ref()
                .zip(self.world.map.track.as_ref())
                .is_some_and(|(hit, track)| hit.contains(at, track))
            && (!self.settings.show_huds || self.scene.hud_pick(&self.world, self.cursor_pos, true).is_none())
    }

    /// The camera drag a left click would start under the pointer, like
    /// LLToolCamera::handleHover (lltoolfocus.cpp): Alt held over the world,
    /// or the build Focus tool. AltGr alone (right Alt, typed as Ctrl + Alt)
    /// is a character key, not the camera.
    fn camera_tool_hover(&self, ctx: &egui::Context) -> Option<crate::camera::ToolMode> {
        use crate::camera::ToolMode;
        let focus_tool = self.build.open && self.build.tool == crate::build::Tool::Focus;
        let alt_gr = self.down.contains(&Input::key(KeyCode::AltRight)) && !self.down.contains(&Input::key(KeyCode::AltLeft));
        if !matches!(self.screen, Screen::World)
            || self.mouse_mode != MouseMode::None
            || self.camera.mouselook()
            || !(focus_tool || (self.alt && !alt_gr))
            || ctx.is_pointer_over_egui()
            || ctx.egui_wants_pointer_input()
        {
            return None;
        }
        Some(if focus_tool && !self.ctrl && !self.shift {
            match self.build.focus_mode {
                crate::build::FocusMode::Zoom => ToolMode::Zoom,
                crate::build::FocusMode::Orbit => ToolMode::Orbit,
                crate::build::FocusMode::Pan => ToolMode::Pan,
            }
        } else {
            ToolMode::from_mods(self.ctrl, self.shift)
        })
    }

    fn toggle_mouselook(&mut self) {
        self.camera.toggle_mouselook(&mut self.world.agent, &self.settings.camera);
        let ml = self.camera.mouselook();
        self.set_mouselook_grab(ml);
    }

    /// Mouse motion: mouselook, camera drags (steering, Alt), right drag orbit.
    fn camera_mouse_delta(&mut self, dx: f32, dy: f32) {
        if self.clicks.waiting() {
            // behind the click that will say what this motion drags
            self.clicks.hold(click_pick::Held::Motion(dx, dy));
            return;
        }
        if self.hud_drag.is_some() {
            return;
        }
        let s = self.settings.mouse_sensitivity;
        if self.camera.mouselook() {
            self.camera.look(dx, dy, s, &mut self.world.agent);
        } else if self.mouse_mode != MouseMode::None {
            // LLToolCamera: steering is the zoom tool on our avatar
            let focus_tool = self.build.open && self.build.tool == crate::build::Tool::Focus && !self.ctrl && !self.shift;
            let mode = match self.mouse_mode {
                MouseMode::Steer => crate::camera::ToolMode::Zoom,
                _ if focus_tool => match self.build.focus_mode {
                    crate::build::FocusMode::Zoom => crate::camera::ToolMode::Zoom,
                    crate::build::FocusMode::Orbit => crate::camera::ToolMode::Orbit,
                    crate::build::FocusMode::Pan => crate::camera::ToolMode::Pan,
                },
                _ => crate::camera::ToolMode::from_mods(self.ctrl, self.shift),
            };
            let width = self.gfx.as_ref().map(|g| g.renderer.size().0 as f32).unwrap_or(1280.0);
            self.camera.drag(mode, dx, dy, width, &mut self.world.agent);
        } else if self.right_drag {
            if !self.right_gesture.motion(dx, dy, self.egui_ctx.pixels_per_point()) {
                return; // maybe a click: do not move the camera yet
            }
            match self.camera.free_mut() {
                // the free camera looks around instead of orbiting
                Some(free) => free.drag(dx, dy, s),
                None => self.camera.orbit_drag(dx, dy, s, &mut self.world.agent),
            }
        }
    }

    /// AURORA_DEMO_CAMERA: scripted camera input (camera/demo.rs).
    fn demo_camera_steps(&mut self, spec: &str) {
        use crate::camera::demo::Step;
        for step in crate::camera::demo::steps(spec, self.frame_count) {
            match step {
                Step::Cursor(x, y) => self.cursor_pos = (x, y),
                Step::CursorOnOwnTag => {
                    let ppp = self.egui_ctx.pixels_per_point();
                    match self.name_tags.iter().find(|t| t.id == self.world.agent_id) {
                        Some(t) => self.cursor_pos = (t.rect.center().x * ppp, t.rect.center().y * ppp),
                        None => log::warn!("demo camera: our name tag is not on screen"),
                    }
                }
                Step::Mods { ctrl, shift, alt } => {
                    self.ctrl = ctrl;
                    self.shift = shift;
                    self.alt = alt;
                }
                Step::Press => self.on_left_press(),
                Step::Release => self.on_left_release(),
                Step::RightButton(pressed) => self.mouse_input(MouseButton::Right, pressed, false),
                Step::Drag(dx, dy) => self.camera_mouse_delta(dx, dy),
                Step::Walk(on) => {
                    if on {
                        self.down.insert(Input::key(KeyCode::ArrowUp));
                    } else {
                        self.down.remove(&Input::key(KeyCode::ArrowUp));
                    }
                }
                Step::ToggleMouselook => self.toggle_mouselook(),
                Step::Wheel(clicks) => {
                    let was = self.camera.mouselook();
                    self.camera
                        .scroll(clicks, false, false, &mut self.world.agent, &mut self.settings.camera);
                    if was != self.camera.mouselook() {
                        self.set_mouselook_grab(!was);
                    }
                }
                Step::Fly => self.world.agent.flying = true,
                Step::FollowEvents => {
                    if let Some(handle) = self.world.main_region {
                        let events = if spec == "scripted-sit" {
                            crate::camera::demo::sit_follow_events(self.frame_count, handle, self.world.agent.position)
                        } else {
                            crate::camera::demo::follow_events(self.frame_count, handle, self.world.agent.position)
                        };
                        for ev in events {
                            self.world.apply(ev);
                        }
                    }
                }
                Step::SitEvents => {
                    for ev in crate::demo::sit_events(self.frame_count) {
                        if let Some(e) = self.world.apply(ev) {
                            self.on_app_event(e);
                        }
                    }
                }
                Step::Log => self.demo_note(click_pick::DemoNote::Camera(self.frame_count)),
            }
        }
    }

    /// One scripted step of AURORA_DEMO_ENV_SELECT, through the same calls
    /// as the environment window.
    fn demo_env_step(&mut self, step: crate::demo::env::Step) {
        use crate::demo::env::Step;
        use crate::world::env_select::{self, SettingsKind};
        match step {
            Step::Open { lighting } => {
                self.panels.environment = true;
                self.panels.personal_lighting = lighting;
            }
            Step::Pick(kind, asset) => self.world.eep.request_local(asset, kind),
            Step::Next(kind) => {
                let view = self.world.eep.local_view();
                let shown = match kind {
                    SettingsKind::Sky => view.sky,
                    SettingsKind::Water => view.water,
                    SettingsKind::Day => view.day,
                };
                let list = self.env_ui.catalog().list(kind);
                if let Some(asset) = env_select::step(list, env_select::shown_index(list, shown), true) {
                    self.world.eep.request_local(asset, kind);
                }
            }
            Step::Shared => {
                self.world.eep.clear_local();
                self.panels.time_of_day = 0;
            }
            Step::EditLighting => {
                self.world.eep.edit_personal(crate::demo::env::edit_lighting);
            }
            Step::OpenList(kind) => self.env_ui.open_list = Some(kind),
        }
    }

    /// Escape / "Reset camera" (handle_reset_view).
    fn reset_camera_view(&mut self) {
        let was = self.camera.mouselook();
        self.camera.handle_reset_view(&mut self.world.agent, &self.settings.camera);
        if was && !self.camera.mouselook() {
            self.set_mouselook_grab(false);
        }
    }

    fn mods(&self) -> Mods {
        Mods {
            ctrl: self.ctrl,
            shift: self.shift,
            alt: self.alt,
        }
    }

    /// Walk bindings taking part in double-tap running: 0 forward, 1 back,
    /// 2 left, 3 right (turn keys only when they strafe).
    fn run_group(&self, input: &Input) -> Option<u8> {
        // steering with the left button: left / right walk too (camera-relative)
        let strafe = self.mouse_mode == MouseMode::Steer || self.shift != self.settings.arrows_strafe;
        self.settings.keybinds.held_actions_of(input).into_iter().find_map(|a| match a {
            Action::Forward => Some(0),
            Action::Back => Some(1),
            Action::Left if strafe => Some(2),
            Action::Right if strafe => Some(3),
            Action::StrafeLeft => Some(2),
            Action::StrafeRight => Some(3),
            _ => None,
        })
    }

    /// The right button walks forward: held while steering with the left
    /// button on our avatar.
    fn mouse_walks(&self) -> bool {
        self.right_drag && self.mouse_mode == MouseMode::Steer
    }

    /// A walk input of `group` pressed or released: pressed twice quickly, it
    /// runs while held (agent_handle_doubletap_run). The right button of
    /// mouse steering counts as a forward key, so a double right click runs.
    fn walk_tap(&mut self, group: u8, pressed: bool) {
        if pressed && self.in_world() {
            if self.temp_run.is_none()
                && let Some((last, at)) = self.last_tap
                && last == group
                && at.elapsed().as_secs_f32() < NUDGE_TIME
            {
                self.temp_run = Some(group);
                if !self.always_run {
                    self.send(NetCommand::SetAlwaysRun(true));
                }
            }
            self.last_tap = Some((group, Instant::now()));
        } else if !pressed {
            self.check_temp_run();
        }
    }

    /// Stop the temporary run once nothing of its walk group is held any more
    /// (agent_check_temporary_run).
    fn check_temp_run(&mut self) {
        let Some(group) = self.temp_run else {
            return;
        };
        let still_held = self.down.iter().any(|k| self.run_group(k) == Some(group)) || (group == 0 && self.mouse_walks());
        if !still_held {
            self.temp_run = None;
            if !self.always_run {
                self.send(NetCommand::SetAlwaysRun(false));
            }
        }
    }

    /// Voice transmission wanted (microphone button / push-to-talk).
    fn talking(&self) -> bool {
        if self.settings.audio.mic_hold {
            self.mic_button_held || self.settings.keybinds.held(Action::PushToTalk, &self.down, self.mods())
        } else {
            self.mic_on
        }
    }

    /// A key or a bindable mouse button changed state.
    fn on_input(&mut self, input: Input, pressed: bool) {
        if pressed {
            self.down.insert(input.clone());
        } else {
            self.down.remove(&input);
        }
        if let Some(group) = self.run_group(&input) {
            self.walk_tap(group, pressed);
        }
        if !pressed || !self.in_world() {
            return;
        }
        // push-to-talk key in toggle mode: each press switches the microphone
        if !self.settings.audio.mic_hold && self.settings.keybinds.held_actions_of(&input).contains(&Action::PushToTalk) {
            self.mic_on = !self.mic_on;
        }
        let Some(action) = self.settings.keybinds.triggered(&input, self.mods()) else {
            return;
        };
        match action {
            Action::Fly => self.toggle_fly(),
            Action::AlwaysRun => self.set_always_run(!self.always_run),
            Action::Mouselook => self.toggle_mouselook(),
            Action::CameraControls => self.panels.camera_controls = !self.panels.camera_controls,
            Action::ResetCamera => {
                if !self.photo_escape() {
                    self.reset_camera_view();
                }
            }
            Action::Chat => self.chat_ui.focus_request = true,
            Action::ToggleMic => self.mic_on = !self.mic_on,
            // the shortcuts of the windows, like their toolbar buttons: a
            // minimized window expands (LLFloaterReg::toggleInstance)
            Action::Conversations => ui::widgets::toggle_floater(&self.egui_ctx, "conversations", &mut self.panels.chat),
            Action::People => {
                ui::widgets::toggle_floater(&self.egui_ctx, "people", &mut self.panels.people);
                self.panels.people_tab = 0;
            }
            Action::Inventory => ui::widgets::toggle_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory),
            Action::Gestures => ui::widgets::toggle_floater(&self.egui_ctx, "gestures", &mut self.panels.gestures),
            Action::Minimap => self.panels.minimap = !self.panels.minimap,
            Action::WorldMap => ui::widgets::toggle_floater(&self.egui_ctx, "worldmap", &mut self.panels.world_map),
            Action::TeleportHistory => self.toggle_teleport_history(),
            Action::RegionEstate => ui::widgets::toggle_floater(&self.egui_ctx, ui::region::WINDOW, &mut self.panels.region_estate),
            Action::Search => ui::widgets::toggle_floater(&self.egui_ctx, ui::search::WINDOW, &mut self.panels.search),
            Action::Performance => ui::widgets::toggle_floater(&self.egui_ctx, "perf", &mut self.panels.perf),
            Action::Preferences => ui::widgets::toggle_floater(&self.egui_ctx, "options", &mut self.panels.settings),
            Action::Import => self.imports.toggle_launcher(&self.egui_ctx),
            Action::ShowTransparency => {
                let d = &mut self.settings.debug;
                d.show_alpha = !d.show_alpha;
                self.world.system_message(if d.show_alpha {
                    aurora_i18n::tr!("app-show-transparency-enabled-ctrl-alt-t-to-disable")
                } else {
                    aurora_i18n::tr!("app-show-transparency-disabled")
                });
            }
            Action::Rebake => self.rebake(),
            Action::PhotoWindow => ui::widgets::toggle_floater(&self.egui_ctx, ui::snapshot::WINDOW, &mut self.panels.snapshot),
            Action::PhotoToDisk => self.photo_to_disk(),
            // held actions are read every frame
            _ => {}
        }
    }

    /// Is a world point on (or very near) our own avatar?
    /// Where a ray enters and leaves our avatar's box (the size the
    /// simulator gives the avatar object, a little wider for the arms), and
    /// where along it the ray passes within 0.2 m of the body's vertical axis
    /// (the click finds the avatar before its body and clothes are drawn).
    fn own_avatar_ray(&self, origin: Vec3, dir: Vec3) -> Option<(f32, f32, Option<f32>)> {
        let a = &self.world.agent;
        let size = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .map(|o| o.scale)
            .filter(|s| s.z > 0.5 && s.is_finite())
            .unwrap_or(Vec3::new(0.45, 0.6, 1.9));
        let half = Vec3::new((size.x * 0.5).max(0.3) + 0.15, (size.y * 0.5).max(0.3) + 0.15, size.z * 0.5 + 0.1);
        // box space: centered on the agent, turned with its body
        let inv = a.body_rotation().inverse();
        let o = inv * (origin - a.position);
        let d = inv * dir;
        let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
        for k in 0..3 {
            if d[k].abs() < 1e-8 {
                if o[k].abs() > half[k] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-half[k] - o[k]) / d[k], (half[k] - o[k]) / d[k]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        if t0 > t1 {
            return None;
        }
        // closest approach to the vertical axis, inside the box
        let dxy = d.truncate();
        let t = if dxy.length_squared() > 1e-8 {
            -o.truncate().dot(dxy) / dxy.length_squared()
        } else {
            t0
        };
        let t = t.clamp(t0, t1);
        let q = o + d * t;
        let axis = (q.truncate().length() < 0.2).then_some(t);
        Some((t0, t1, axis))
    }

    /// The avatar whose name tag is under the cursor, and the point on the
    /// cursor ray at its tag. LLPipeline::lineSegmentIntersectInWorld tests
    /// the avatar tags after the world: a tag picks its avatar unless
    /// something is in front of it, the nearest tag winning.
    fn pick_name_tag(&self, hit: Option<Vec3>, ray: Option<(Vec3, Vec3)>) -> Option<(usize, Vec3)> {
        let (o, d) = ray?;
        let ppp = self.egui_ctx.pixels_per_point().max(0.1);
        let cursor = egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp);
        let now = Instant::now();
        let world_t = hit.map(|p| (p - o).dot(d));
        // drawn far first: the last one under the cursor is on top
        self.name_tags.iter().rev().filter(|t| t.rect.contains(cursor)).find_map(|tag| {
            let idx = self.world.objects.index_of_uuid(&tag.id)?;
            let (pos, _, _) = crate::scene::Scene::object_transform(&self.world, idx, now, 0)?;
            // where the tag stands for the scene's depth: what hides it there
            // keeps the click, what shows it gives it
            let anchor = self.scene.name_tag_anchor(&tag.id, pos);
            let t = (ui::hud::tag_point(anchor, self.camera.position) - o).dot(d);
            match world_t {
                Some(h) if h < t - 0.05 => None,
                _ => Some((idx, o + d * t)),
            }
        })
    }

    /// Left press on the world, past the build tools (LLToolPie).
    fn on_left_press(&mut self) {
        self.click(click_pick::Click::World);
    }

    /// What `on_left_press` does, run by `click`: again from the start when
    /// the depth under the cursor has come back (app/click_pick.rs).
    fn world_press(&mut self) {
        self.left_down = true;
        if self.hud_drag_requested()
            && self.settings.show_huds
            && let Some(idx) = self.scene.hud_move_pick(&self.world, self.cursor_pos)
        {
            // Own the complete gesture: no media / script click or camera focus,
            // even when the visible frontmost HUD cannot be moved.
            self.media.unfocus();
            self.hud_drag = self
                .scene
                .lists
                .hud_view
                .and_then(|view| crate::build::hud_drag::HudDrag::start(&self.world, idx, view, self.cursor_pos));
            return;
        }
        if self.gfx.is_none() {
            return;
        }
        let (x, y) = self.cursor_pos;
        if let Some((idx, point, hud_ray)) = self.scene.hud_pick(&self.world, (x, y), true) {
            self.media.build_mode = self.build.open;
            if !self.alt && !self.ctrl && !self.shift && !self.build.open {
                let mods = aurora_media::Modifiers {
                    control: false,
                    alt: false,
                    shift: false,
                };
                let distance = (point - hud_ray.0).dot(hud_ray.1);
                if self
                    .media
                    .on_click(&self.world, &self.scene, hud_ray, Some(distance), &self.settings.media, mods, true)
                {
                    return;
                }
                if let Some(target) = crate::interaction::target(&self.world, idx, &self.interactions.props) {
                    self.activate_object_action(target, idx, point, Some(hud_ray));
                }
            }
            return;
        }
        let Some(picked) = self.click_pick() else {
            return;
        };
        // (kept until the depth is in hand: the press is run again then)
        let double = std::mem::take(&mut self.double_press);
        let (ray, depth) = (picked.ray, picked.depth);
        let hit = self
            .scene
            .action_point(&self.world, ray, depth, self.build.open, self.settings.draw_distance);
        // another avatar's plate with a click action takes the click
        if self.plate_press(hit, ray) {
            return;
        }
        // a rigged mesh or a worn mesh shown there: the GPU alone poses it
        let shown = self.designate(crate::scene::drawn::Gesture::Touch, &picked, hit);
        let hit = shown.map_or(hit, |d| Some(d.hit.point));
        // on our avatar: a surface inside its box under the cursor (nothing
        // in front of it); or, when the ray goes through the box without
        // touching anything (body not drawn yet), close to the body's axis
        let avatar_hit = ray.filter(|_| shown.is_none_or(|d| !d.blend)).and_then(|(o, d)| {
            let (t0, t1, axis) = self.own_avatar_ray(o, d)?;
            match hit.map(|p| (p - o).dot(d)) {
                Some(t) if t < t0 - 0.05 => None,
                Some(t) if t <= t1 + 0.05 => hit,
                _ => axis.map(|t| o + d * t),
            }
        });
        // or on our name tag
        let avatar_hit = avatar_hit.or_else(|| {
            let (idx, p) = self.pick_name_tag(hit, ray)?;
            (self.world.objects.get(idx)?.full_id == self.world.agent_id).then_some(p)
        });
        let on_avatar = avatar_hit.is_some();
        // a media face takes the click (LLToolPie::handleMediaClick)
        self.media.build_mode = self.build.open;
        // "Object view" of the Controls window: the click is Alt's
        let alt = self.alt || self.object_view;
        if !alt && !on_avatar {
            if let Some(ray) = ray {
                let depth_t = hit.map(|p| (p - ray.0).dot(ray.1));
                let mods = aurora_media::Modifiers {
                    control: self.ctrl,
                    alt: self.alt,
                    shift: self.shift,
                };
                if self
                    .media
                    .on_click(&self.world, &self.scene, ray, depth_t, &self.settings.media, mods, false)
                {
                    return;
                }
            }
        } else {
            self.media.unfocus();
        }
        let now = Instant::now();
        let own_idx = self.world.objects.index_of_uuid(&self.world.agent_id);
        if self.demo && std::env::var_os("AURORA_DEMO_ACTIONS").is_some() {
            log::info!(
                "demo action pick: hit={hit:?}, avatar={on_avatar}, picked={:?}",
                hit.zip(ray)
                    .and_then(|(p, r)| self.scene.interaction_at_ray(&self.world, p, r, true))
                    .and_then(|i| self.world.objects.get(i))
                    .map(|o| (o.key.local_id, o.click_action))
            );
        }
        // the click acts on the rigged mesh it lands on, as on any object:
        // also one we wear, like the scripted attachments Firestorm lets
        // us touch (its rigged ones are out of the left click's reach)
        if !alt
            && !self.ctrl
            && !self.shift
            && !self.build.open
            && !self.camera.mouselook()
            && let Some(d) = shown
            && let Some(target) = crate::interaction::target(&self.world, d.idx, &self.interactions.props)
        {
            let surface = self.scene.drawn_surface(&self.world, &d);
            self.activate_object_action_at(target, d.idx, d.hit.point, ray, surface);
            return;
        }
        if !on_avatar
            && shown.is_none()
            && !alt
            && !self.ctrl
            && !self.shift
            && !self.build.open
            && !self.camera.mouselook()
            && let Some(point) = hit
            && let Some(pick_ray) = ray
            && let Some(idx) = self.scene.interaction_at_ray(&self.world, point, pick_ray, true)
            && let Some(target) = crate::interaction::target(&self.world, idx, &self.interactions.props)
        {
            self.activate_object_action(target, idx, point, ray);
            return;
        }
        if alt {
            // Alt+click (LLToolCamera): lock the camera on the clicked point
            // (our avatar too, to look at a part of it closely); keep holding
            // to zoom, Ctrl to orbit, Ctrl+Shift to pan
            if let (Some(p), Some(ray)) = (avatar_hit.or(hit), ray) {
                let idx = if on_avatar {
                    own_idx
                } else {
                    self.scene.interaction_at(&self.world, p, now, self.build.open)
                };
                let object = idx.and_then(|i| crate::camera::pick_focus_object(&self.world, i, now));
                self.camera.alt_focus(p, object, ray, &mut self.world.agent, &self.settings.camera);
                self.camera.begin_drag(false);
                self.mouse_mode = MouseMode::FocusOrbit;
                self.set_mouselook_grab(true);
            }
        } else if let (Some(p), Some(ray)) = (avatar_hit, ray) {
            // click on our avatar (LLToolPie -> LLToolCamera): the camera goes
            // back to it unless ClickOnAvatarKeepsCamera; hold and drag to steer
            let object = own_idx.and_then(|i| crate::camera::pick_focus_object(&self.world, i, now));
            let s = &self.settings.camera;
            self.camera.alt_focus(p, object, ray, &mut self.world.agent, s);
            if !s.click_avatar_keeps_camera {
                self.camera.set_focus_on_avatar(true, true, true, &mut self.world.agent, s);
            }
            self.camera.begin_drag(true);
            self.mouse_mode = MouseMode::Steer;
            self.set_mouselook_grab(true);
        } else if crate::autopilot::takes_ground_double_click(crate::autopilot::Press {
            double,
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            build: self.build.open,
            on_avatar,
        }) && let Some(point) = hit
        {
            // a double click on the ground, or on an object without a click
            // action (which took the press above): walk_to / teleport_to
            self.double_click_ground(point);
        }
    }

    /// Right click in the world: menu for what is under the cursor.
    /// Keep the audio engine in step with the settings and the parcel music
    /// (LLViewerParcelMedia / LLViewerAudio: autoplay on entering a parcel).
    /// The look-at preferences, as the demo scenario wants them: everything
    /// shown and sent.
    fn look_at_settings(&self) -> crate::world::lookat::Settings {
        let mut s = self.settings.lookat;
        if self.demo_look_at {
            s.private = false;
            s.private_local = false;
        }
        if self.demo_look_at_targets {
            s.show_targets = true;
            s.hide_own = false;
            s.lines = true;
        }
        s
    }

    /// Our look-at target (LLAgentCamera::setLookAt): none at all with
    /// PrivateLocalLookAtTarget; with a range, a target away from us is
    /// brought back within it (FSLookAtTargetLimitDistance).
    fn set_look_at(&mut self, kind: u8, object: Option<uuid::Uuid>, offset: glam::DVec3) {
        use crate::world::lookat;
        let now = Instant::now();
        let me = self.world.agent_id;
        let prefs = self.look_at_settings();
        if prefs.private_local {
            self.world.look_at.own.set(lookat::NONE, Some(me), glam::DVec3::ZERO, now);
            return;
        }
        let (mut object, mut offset) = (object, offset);
        if lookat::clamped(kind) && prefs.limit_distance && object != Some(me) {
            let Some((mx, my)) = self.world.main_origin() else {
                return;
            };
            let origin = glam::DVec3::new(mx as f64, my as f64, 0.0);
            // the target as a global point
            if let Some(obj) = object {
                let Some(idx) = self.world.objects.index_of_uuid(&obj) else {
                    return;
                };
                let p = if self.world.objects.get(idx).is_some_and(|o| o.is_avatar()) {
                    self.scene.head_world(&self.world, &obj, now)
                } else {
                    crate::scene::Scene::object_transform(&self.world, idx, now, 0).map(|(p, r, _)| p + r * offset.as_vec3())
                };
                let Some(p) = p else {
                    return;
                };
                offset = origin + p.as_dvec3();
                object = None;
            }
            let head = self.scene.head_world(&self.world, &me, now).unwrap_or(self.world.agent.position);
            offset = lookat::within(origin + head.as_dvec3(), offset, prefs.max_distance as f64);
        }
        self.world.look_at.own.set(kind, object, offset, now);
    }

    /// Where our avatar looks this frame (LLAgentCamera::updateLookAt and
    /// setFocusGlobal), and the ViewerEffect sent when it changes.
    fn update_look_at(&mut self, width: f32, height: f32) {
        use crate::world::lookat;
        let now = Instant::now();
        let me = self.world.agent_id;
        let prefs = self.look_at_settings();
        let privacy = (prefs.private, prefs.private_local);
        if self.look_at_privacy.is_some_and(|was| was != privacy) {
            if prefs.private && prefs.private_local {
                // nothing is sent any more: the others stop showing our
                // last target
                self.send(NetCommand::LookAt {
                    effect: self.world.look_at.own.effect,
                    target: uuid::Uuid::nil(),
                    offset: [0.0; 3],
                    kind: lookat::NONE,
                    duration: 0.0,
                });
            } else {
                // what the others get changed: tell them now
                self.world.look_at.own.needs_send = true;
            }
        }
        self.look_at_privacy = Some(privacy);
        // the camera aims at a point (Ctrl+Alt+click, zoom): FOCUS, then CLEAR
        if self.camera.focus_point() != self.look_focus {
            self.look_focus = self.camera.focus_point();
            match (self.look_focus, self.world.main_origin()) {
                (Some(p), Some((mx, my))) => {
                    let g = glam::DVec3::new(mx as f64, my as f64, 0.0) + p.as_dvec3();
                    self.set_look_at(lookat::FOCUS, None, g);
                }
                _ => self.set_look_at(lookat::CLEAR, None, glam::DVec3::ZERO),
            }
        }
        // heard chat (AUTO_LISTEN)
        let requests: Vec<_> = std::mem::take(&mut self.world.look_at.requests);
        for (kind, object, offset) in requests {
            self.set_look_at(kind, object, offset);
        }
        // idle (mouse still, not turning): ahead of us; otherwise where the
        // cursor points (±45° across the window) or the mouselook view
        let body = self
            .world
            .bodies
            .get(&me)
            .map(|b| b.rotation)
            .unwrap_or(self.world.agent.body_rotation());
        let root_at = body * Vec3::X;
        let turning = root_at.dot(self.look_root_at) <= 0.95;
        self.look_root_at = root_at;
        let (cx, cy) = self.cursor_pos;
        let still = (cx - self.look_cursor.0).abs() < 0.5 && (cy - self.look_cursor.1).abs() < 0.5;
        self.look_cursor = (cx, cy);
        let (kind, object, offset) = if still && !turning {
            let v = self.world.agent.velocity;
            let ahead = if v.length_squared() > 4.0 {
                v
            } else {
                self.world.agent.forward() * 2.0
            };
            (lookat::IDLE, Some(me), (body.inverse() * ahead).as_dvec3())
        } else if self.camera.mouselook() {
            (lookat::MOUSELOOK, Some(me), self.camera.forward().as_dvec3())
        } else {
            let dir = lookat::cursor_direction(
                self.camera.forward(),
                cx / width.max(1.0) - 0.5,
                0.5 - cy / height.max(1.0),
                prefs.head_yaw,
                prefs.head_pitch,
            );
            lookat::cursor_look(prefs.aim, self.look_hover_target(now), me, dir)
        };
        self.set_look_at(kind, object, offset);
        let own = &mut self.world.look_at.own;
        own.update(now);
        if own.needs_send {
            let sent = lookat::outgoing(own.target, me, prefs.private, prefs.private_local);
            own.sent(now);
            if let Some(t) = sent {
                let cmd = NetCommand::LookAt {
                    effect: own.effect,
                    target: t.object.unwrap_or_default(),
                    offset: t.offset.to_array(),
                    kind: t.kind,
                    duration: own.duration,
                };
                self.send(cmd);
            }
        }
    }

    /// AURORA_DEMO_LOOKAT=targets: every other avatar sends a look-at, of a
    /// type and toward a target that change from one round to the next: a
    /// point, another avatar, the cursor, mouselook.
    fn demo_look_at_everyone(&mut self) {
        use crate::world::lookat;
        let Some((mx, my)) = self.world.main_origin() else {
            return;
        };
        let origin = glam::DVec3::new(mx as f64, my as f64, 0.0);
        let now = Instant::now();
        let me = self.world.agent_id;
        let avatars: Vec<(uuid::Uuid, Vec3, glam::Quat)> = self
            .world
            .objects
            .avatars()
            .filter(|(_, o)| o.full_id != me)
            .filter_map(|(idx, o)| {
                let (p, r, _) = crate::scene::Scene::object_transform(&self.world, idx, now, 0)?;
                Some((o.full_id, p, r))
            })
            .collect();
        let round = (self.frame_count / 120) as usize;
        let mut events = Vec::new();
        for (i, (id, p, r)) in avatars.iter().enumerate() {
            let turn = (i * 7 + round * 3) as f32 * 0.9;
            let ahead = *r * glam::Quat::from_rotation_z(turn.sin() * 0.8) * Vec3::X;
            let step = (i + round) % 5;
            let (kind, target, offset) = match step {
                // a point in front, on the ground or above it
                0 | 3 => {
                    let at = *p + ahead * (2.0 + (i % 4) as f32) + Vec3::Z * (turn.cos() * 0.8);
                    let kind = if step == 0 { lookat::FOCUS } else { lookat::SELECT };
                    (kind, uuid::Uuid::nil(), (origin + at.as_dvec3()).to_array())
                }
                // the next avatar's eyes
                1 => {
                    let (other, _, _) = avatars[(i + 1) % avatars.len()];
                    (lookat::CONVERSATION, other, [0.0; 3])
                }
                // where its cursor points: a direction from its head
                2 => (lookat::FREELOOK, *id, (ahead * 2.0).as_dvec3().to_array()),
                _ => (lookat::MOUSELOOK, *id, (ahead * 3.0).as_dvec3().to_array()),
            };
            events.push(NetEvent::LookAt {
                effect: uuid::Uuid::from_u128(0x1000_0000 + i as u128),
                source: *id,
                target,
                offset,
                kind,
                duration: lookat::MAX_TIMEOUT,
            });
        }
        for ev in events {
            if let Some(e) = self.world.apply(ev) {
                self.on_app_event(e);
            }
        }
    }

    /// The point under the cursor the head looks at (`lookat::cursor_look`):
    /// the hover pick of the last frame, on an object in its frame (an
    /// avatar's from its head, as calcTargetPosition aims), or a global
    /// point of the ground. None over the sky, a window, a HUD or
    /// ourselves and what we wear.
    fn look_hover_target(&self, now: Instant) -> Option<(Option<uuid::Uuid>, glam::DVec3)> {
        use crate::scene::Scene;
        let (idx, point) = self.look_hover?;
        let me = self.world.agent_id;
        let Some(idx) = idx else {
            let (mx, my) = self.world.main_origin()?;
            return Some((None, glam::DVec3::new(mx as f64, my as f64, 0.0) + point.as_dvec3()));
        };
        let o = self.world.objects.get(idx)?;
        if Scene::skeleton_owner(&self.world, idx).is_some_and(|(owner, _)| owner == me) {
            return None;
        }
        let (at, rot, hud) = Scene::object_transform(&self.world, idx, now, 0)?;
        if hud {
            return None;
        }
        let at = if o.is_avatar() {
            self.scene.head_world(&self.world, &o.full_id, now)?
        } else {
            at
        };
        Some((Some(o.full_id), crate::world::lookat::local_offset(point, at, rot)))
    }

    /// FSStaticEyes (update_static_eyes in llstartup.cpp, its setting's
    /// listener in llviewercontrol.cpp): the animation that holds the eyes
    /// still is started once arrived, then started or stopped as the setting
    /// changes.
    fn sync_static_eyes(&mut self) {
        if self.move_complete_at.is_none() {
            return;
        }
        let want = self.settings.lookat.static_eyes;
        match self.static_eyes_sent {
            Some(sent) if sent == want => {}
            None if !want => self.static_eyes_sent = Some(false),
            _ => {
                self.send(NetCommand::AgentAnimation {
                    anim: crate::world::lookat::STATIC_EYES,
                    start: want,
                });
                self.static_eyes_sent = Some(want);
            }
        }
    }

    /// World and interface sounds: object sound changes, the world's
    /// interface sounds, windows opening / closing, then placement for the
    /// listener (the camera).
    /// Sound changes carried by object updates, applied before a later
    /// sound message (the order LL handles them in).
    fn flush_object_sounds(&mut self) {
        let engine = self.audio_engine.as_ref();
        // the block list changed: silence / restore the sounds it covers
        let version = self.world.mutes.version;
        let recheck = self.sound_blocks.0 != version;
        self.sound_blocks.0 = version;
        let mutes = &self.world.mutes;
        let blocked =
            |o: &crate::world::objects::Object| mutes.is_muted_id(&o.full_id) || (!o.owner_id.is_nil() && mutes.sounds_muted(&o.owner_id));
        if recheck {
            // rare: every object with a sound, but for those whose update is
            // applied below
            for o in self.world.objects.slots.iter().flatten() {
                if o.sound_update || o.sound.id.is_nil() {
                    continue;
                }
                if blocked(o) {
                    if self.sound_blocks.1.insert(o.full_id) {
                        self.scene.sounds.apply_object_sound(engine, o.full_id, Default::default());
                    }
                } else if self.sound_blocks.1.remove(&o.full_id) {
                    self.scene.sounds.apply_object_sound(engine, o.full_id, o.sound);
                }
            }
        }
        // the objects that got a full update, listed by the store (no walk
        // over every object each frame and before each sound message), in
        // slot order as a walk would take them
        let mut changed = std::mem::take(&mut self.sound_changes);
        self.world.objects.take_sound_changes(&mut changed);
        changed.sort_unstable();
        for &idx in &changed {
            let Some(o) = self.world.objects.get_mut_untracked(idx) else {
                continue;
            };
            if !std::mem::take(&mut o.sound_update) {
                continue;
            }
            if blocked(o) {
                self.sound_blocks.1.insert(o.full_id);
            } else {
                self.scene.sounds.apply_object_sound(engine, o.full_id, o.sound);
            }
        }
        changed.clear();
        self.sound_changes = changed;
    }

    /// Plays an interface sound when the settings allow it (make_ui_sound /
    /// find_ui_sound: its play flag, its asset, the "Interface sounds"
    /// switch).
    fn play_ui_sound(&mut self, s: UiSound) {
        if !self.settings.audio.ui_sounds {
            return;
        }
        if let Some(id) = self.settings.audio.ui.resolve(s, false) {
            // RUST_LOG=info,ui_sound=debug (Firestorm UISndDebugSpamToggle)
            log::debug!(target: "ui_sound", "UISnd{} ({id})", s.entry().name);
            self.scene.sounds.play_ui(self.audio_engine.as_ref(), id);
        }
    }

    /// Aurora's notification center (Preferences › Aurora › Notifications),
    /// or Firestorm's toasts; AURORA_DEMO_NOTIFS=legacy shows the latter for
    /// the session without saving it.
    fn notifications_aurora(&self) -> bool {
        if self.demo && crate::demo::notifs::mode().is_some_and(|m| m.starts_with("legacy")) {
            return false;
        }
        self.settings.notifications_style == crate::settings::NotificationsStyle::Aurora
    }

    /// A click in the center besides an answer: the paused bell's banner, an
    /// entry of the history.
    fn on_notification_ask(&mut self, ctx: &egui::Context, ask: ui::notification_center::Ask) {
        use crate::world::notification_history::Follow;
        match ask {
            ui::notification_center::Ask::EndDnd => self.world.set_dnd(false),
            ui::notification_center::Ask::Follow(e) => match e.follow.clone() {
                Follow::None => {}
                Follow::Restore => {
                    self.world.notifications.restore(&e);
                }
                Follow::Conversation(session) => {
                    if !self.world.social.ims.iter().any(|s| s.other == session) {
                        self.world.social.session_mut(session);
                    }
                    self.world.social.focus_im = Some(session);
                    self.notif_ui.open = false;
                    ui::notifications::open_conversations(ctx, &mut self.panels.chat);
                }
                Follow::Place(link) => {
                    if let Some(l) = crate::slurl::parse(&link) {
                        self.notif_ui.open = false;
                        ui::context::request(ctx, ui::context::CtxAction::ShowPlace(l.region, l.pos));
                    }
                }
                Follow::Inventory(item) => {
                    self.inventory_ui.show_original(&self.world.inventory, item);
                    self.panels.inventory = true;
                    self.notif_ui.open = false;
                }
            },
        }
    }

    fn update_sounds(&mut self) {
        self.flush_object_sounds();
        let ui = &self.settings.audio.ui;
        let w = &mut self.world;
        // conversations by their IM mode (LLIMMgr::addMessage), never in
        // do not disturb; "in front" = shown in the focused window
        let dnd = w.status.dnd;
        w.notifications.toasts = crate::world::notifications::ToastPrefs {
            presence: self.settings.toast_friends_online,
            im: self.settings.toast_im,
            // AURORA_DEMO_TOASTS shows a group message too (off by default)
            group_im: self.settings.toast_group_im || (self.demo && crate::demo::toasts::mode().is_some()),
        };
        // a conversation in sight: its tab shown in the Conversations window
        // (FSFloaterIM visible), whether or not the viewer has the focus
        let shown = (self.panels.chat && !crate::ui::widgets::floater_minimized(&self.egui_ctx, "conversations"))
            .then_some(self.chat_ui.selected)
            .flatten();
        for m in std::mem::take(&mut w.im_messages) {
            let front = self.focused && self.panels.chat && self.chat_ui.selected == Some(m.session);
            if !dnd && let Some(s) = ui.im_sound(m, front) {
                w.ui_sounds.push(s);
            }
            w.notify_message(&m, shown == Some(m.session));
        }
        w.acknowledge_read_messages();
        for (old, new) in std::mem::take(&mut w.balance_changes) {
            if let Some(s) = ui.money_sound(old, new) {
                w.ui_sounds.push(s);
            }
        }
        // push to talk / microphone button (voice_follow_key, LLVoiceClient)
        if self.mic_on != self.mic_was {
            self.mic_was = self.mic_on;
            w.ui_sounds.push(UiSound::MicToggle);
        }
        // context menu closed (PieMenu::hide)
        let menu = self.context_menu.is_some();
        if self.context_menu_was && !menu {
            w.ui_sounds.push(UiSound::PieMenuHide);
        }
        self.context_menu_was = menu;
        // script dialog answered or dismissed (closeFloater of script_floater)
        let dialogs = w
            .notifications
            .list
            .iter()
            .filter(|n| n.kind == crate::world::notifications::Kind::Script && n.interactive())
            .count();
        if dialogs < self.script_dialogs_were {
            w.ui_sounds.push(UiSound::ScriptFloaterClose);
        }
        self.script_dialogs_were = dialogs;
        // tracker beacon: closer = more frequent (FIRE-16969)
        let beacon = w.map.track.as_ref().zip(w.main_origin()).map(|(t, (ox, oy))| {
            let base = Vec3::new((t.x - ox as f64) as f32, (t.y - oy as f64) as f32, t.z);
            base.distance(w.agent.position)
        });
        match beacon {
            Some(d) if ui.plays(UiSound::TrackerBeacon) && matches!(self.screen, Screen::World) => {
                let now = Instant::now();
                match self.beacon_sound_at {
                    None => self.beacon_sound_at = Some(now),
                    Some(t) if now.duration_since(t).as_secs_f32() > crate::ui_sound::beacon_interval(d) => {
                        self.beacon_sound_at = Some(now);
                        w.ui_sounds.push(UiSound::TrackerBeacon);
                    }
                    Some(_) => {}
                }
            }
            _ => self.beacon_sound_at = None,
        }
        for s in std::mem::take(&mut self.world.ui_sounds) {
            self.play_ui_sound(s);
        }
        // others starting to type: a sound at the avatar (LLVOAvatar, SFX)
        let typing: Vec<uuid::Uuid> = std::mem::take(&mut self.world.typing_started);
        let typing_sound = self
            .settings
            .audio
            .ui
            .resolve(UiSound::Typing, false)
            .filter(|_| self.settings.audio.ui_sounds);
        for av in typing {
            if let Some(id) = typing_sound
                && let Some(pos) = self
                    .world
                    .objects
                    .index_of_uuid(&av)
                    .and_then(|i| Scene::object_transform(&self.world, i, Instant::now(), 0))
            {
                self.scene.sounds.trigger(id, pos.0, Some(av), 1.0);
            }
        }
        let engine = self.audio_engine.as_ref();
        let sounds = &mut self.scene.sounds;
        let fwd = (self.camera.target - self.camera.position).normalize_or(Vec3::X);
        let listener = crate::scene::sounds::Listener {
            pos: self.camera.position,
            right: fwd.cross(Vec3::Z).normalize_or(Vec3::X),
            underwater: self.camera.position.z < self.world.main_water_height(),
        };
        sounds.update(engine, &self.world, &listener, Instant::now());
    }

    fn update_audio(&mut self) {
        let in_world = self.in_world();
        let url = if in_world {
            self.world
                .parcel
                .as_ref()
                .map(|pc| pc.music_url.trim().to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };
        // only web streams
        let url = if url.starts_with("http://") || url.starts_with("https://") {
            url
        } else {
            String::new()
        };
        if url != self.music_parcel_url {
            // new parcel stream: autoplay, or keep playing if the music was on;
            // it replaces a stream started by `/music`
            self.music_override = None;
            self.music_playing = !url.is_empty() && (self.settings.audio.music_autoplay || self.music_playing);
            self.music_parcel_url = url.clone();
        }
        let Some(engine) = &self.audio_engine else {
            return;
        };
        let url = match &self.music_override {
            Some(u) if u.starts_with("http://") || u.starts_with("https://") => u.clone(),
            _ => url,
        };
        let want = if self.music_playing { url } else { String::new() };
        if want != self.stream_url {
            if want.is_empty() {
                engine.stop_stream();
            } else {
                log::info!("parcel music: {want}");
                engine.play_stream(&want);
            }
            self.stream_url = want;
        }
        if self.applied_audio.as_ref() != Some(&self.settings.audio) {
            let a = &self.settings.audio;
            if self.applied_audio.as_ref().map(|p| p.output_device.as_str()) != Some(a.output_device.as_str())
                && self.applied_audio.is_some()
            {
                engine.set_output_device((!a.output_device.is_empty()).then(|| a.output_device.clone()));
            }
            // same order as AUDIO_CHANNELS
            for (i, ch) in aurora_audio::Channel::ALL.iter().enumerate() {
                engine.set_volume(*ch, a.volume[i]);
                engine.set_muted(*ch, a.muted[i]);
                engine.set_enabled(*ch, i == 0 || a.enabled[i]);
            }
            self.applied_audio = Some(a.clone());
        }
    }

    fn request_exit(&mut self) {
        self.exit_prompt.request();
        self.down.clear();
        self.right_drag = false;
        self.mic_button_held = false;
        self.drop_clicks();
        self.on_left_release();
        self.release_object_hold();
        if self.temp_run.take().is_some() && !self.always_run {
            self.send(NetCommand::SetAlwaysRun(false));
        }
        self.set_mouselook_grab(false);
        self.egui_ctx.request_repaint();
    }

    fn confirm_exit(&mut self, action: ui::exit::Action) {
        if action != ui::exit::Action::Cancel && !self.script_editors.prepare_exit() {
            self.exit_prompt.open = false;
            return;
        }
        if !self.exit_prompt.confirm(action, Instant::now()) {
            return;
        }
        if self.in_world() && !self.demo {
            self.record_inventory_logout();
            self.want_scene_capture = true;
            self.net.send(NetCommand::Logout);
        }
    }

    /// Log out, save the window, layout, inventory cache and settings, exit.
    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.sync_local_picker_bindings();
        self.imports.flush_local_registry();
        self.record_inventory_logout();
        // revertBeaconsState when the editors close with the viewer
        self.env_beacons_release();
        self.drop_clicks();
        if self.hud_drag.is_some() {
            self.on_left_release();
        }
        if self.in_world() {
            self.net.send(NetCommand::Logout);
            std::thread::sleep(Duration::from_millis(300));
        }
        if let Some(g) = self.gfx.as_ref().filter(|_| forced_window_size().is_none()) {
            self.settings.window_maximized = g.window.is_maximized();
            // when maximized, keep the restored size / position recorded below
            if !self.settings.window_maximized && !g.window.is_minimized().unwrap_or(false) {
                let s = g.window.inner_size();
                let sf = g.window.scale_factor();
                self.settings.window_width = (s.width as f64 / sf) as u32;
                self.settings.window_height = (s.height as f64 / sf) as u32;
                if let Ok(p) = g.window.outer_position() {
                    self.settings.window_x = Some(p.x);
                    self.settings.window_y = Some(p.y);
                }
            }
        }
        if self.capture.is_none() {
            ui::layout::save(&self.egui_ctx);
        }
        self.world.save_inventory_cache();
        if let Some(path) = self.name_cache_path() {
            self.world.social.avatar_names.save(&path);
        }
        self.settings.save();
        // the last lines of the conversations reach their files
        self.world.chat_log.flush(Duration::from_secs(2));
        event_loop.exit();
    }

    fn record_inventory_logout(&mut self) {
        if self.in_world() && !self.demo {
            self.settings.inventory.last_logout = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            self.settings.save();
        }
    }

    /// Remember the normal (not maximized / minimized) window size and
    /// position, 300 ms after the last move / resize: while maximizing,
    /// Windows reports the new geometry before the window reads as maximized.
    fn record_window_geometry(&mut self) {
        let Some(t) = self.geom_changed else {
            return;
        };
        if t.elapsed() < Duration::from_millis(300) || self.capture.is_some() || forced_window_size().is_some() {
            return;
        }
        self.geom_changed = None;
        let Some(g) = &self.gfx else {
            return;
        };
        if g.window.is_maximized() || g.window.is_minimized().unwrap_or(false) {
            return;
        }
        let s = g.window.inner_size();
        if s.width == 0 || s.height == 0 {
            return;
        }
        let sf = g.window.scale_factor();
        self.settings.window_width = (s.width as f64 / sf) as u32;
        self.settings.window_height = (s.height as f64 / sf) as u32;
        if let Ok(p) = g.window.outer_position() {
            self.settings.window_x = Some(p.x);
            self.settings.window_y = Some(p.y);
        }
    }

    /// Navigation bar: teleport to a typed / pasted place (SLURL, Region/x/y/z,
    /// region name). The current region is reached directly, others through
    /// a map lookup by name.
    fn teleport_to_text(&mut self, text: &str) {
        let Some(l) = crate::slurl::parse(text) else {
            self.world.system_message(aurora_i18n::tr!("app-destination-not-recognized"));
            return;
        };
        self.world.system_message(aurora_i18n::tr!(
            "app-teleporting-to",
            value0 = l.region.to_string(),
            value1 = format!("{:.0}", l.pos.x),
            value2 = format!("{:.0}", l.pos.y),
            value3 = format!("{:.0}", l.pos.z)
        ));
        self.teleport_to_location(l.region, l.pos);
    }

    /// The box "Do not show again" of a teleport confirmation was ticked and
    /// the teleport accepted: later ones start at once (Preferences ›
    /// Interface › Teleport brings the question back).
    fn stop_confirming_teleports(&mut self) {
        self.settings.confirm_teleports = false;
        self.settings.save();
    }

    /// Teleport to a position of a region given by name: the current region
    /// directly, others through a map lookup by name.
    fn teleport_to_location(&mut self, region: String, pos: Vec3) {
        let here = self.world.region_name();
        let local = region.eq_ignore_ascii_case(&here);
        self.begin_teleport(format!("{} ({:.0}, {:.0}, {:.0})", region, pos.x, pos.y, pos.z), !local);
        if local {
            if let Some(handle) = self.world.main_region {
                self.send(NetCommand::TeleportTo {
                    handle,
                    position: pos,
                    look_at: Vec3::X,
                });
            }
        } else if self.demo {
            // Only inter-region demo teleports simulate a connection delay.
            self.demo_tp_start = Some(Instant::now());
            self.demo_tp_dest = Some(pos);
        } else {
            self.send(NetCommand::TeleportToRegion {
                name: region,
                position: pos,
            });
        }
    }

    /// Radar of the People window: one sweep a second (also with the
    /// window closed, for its alerts), the profiles its columns show and
    /// the crossings it reports (FSRadar::updateRadarList, fsradar.cpp).
    fn update_radar(&mut self) {
        use crate::world::radar::Event;
        let now = self.radar_clock.elapsed().as_secs_f64();
        // not during a teleport: our position is still the one we had in
        // the region we leave, and the avatars left behind fade from the
        // arrival on
        if !self.in_world() || self.world.teleporting || !self.radar.due(now) {
            return;
        }
        let seen = crate::world::radar::sightings(&self.world);
        let gaps = crate::world::radar::region_gaps(&self.world);
        // AURORA_DEMO_RADAR=tp: for the first ten seconds (at 60 images a
        // second) we are still in the region the demo "teleports" from
        let my_region = if self.demo_radar_tp && self.frame_count < 600 {
            Some(crate::demo::RADAR_LEFT_REGION)
        } else {
            self.world.main_region
        };
        let mut events = self
            .radar
            .sweep(seen, self.world.agent.position, my_region, &gaps, self.settings.draw_distance, now);
        // AURORA_DEMO_PEOPLE: its residents arrived before the session
        if self.demo {
            for (id, ago) in crate::demo::people::arrivals() {
                self.radar.backdate(&id, now - ago);
            }
        }
        let s = self.settings.radar.clone();
        // Firestorm asks for every profile as its entry appears; here only
        // when something reads them (a column on screen, the age alert)
        let on_screen = self.panels.people && self.panels.people_tab == 0;
        if s.age_alert || (on_screen && s.wants_profiles()) {
            for id in self.radar.take_profile_wants() {
                self.world.want_profile(id);
            }
        }
        if s.age_alert {
            let today = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            let profiles = &self.world.profiles;
            events.extend(self.radar.age_alerts(i64::from(s.age_alert_days), |id| {
                profiles.get(id).and_then(|p| ui::people::age_days(p, today))
            }));
        }
        for event in events {
            let meters = |d: f32| aurora_i18n::number(f64::from(d), 2);
            let (wanted, text) = match event {
                Event::ChatEnter(_, d) => (s.chat_enter, aurora_i18n::tr!("radar-chat-enter", distance = meters(d))),
                Event::ChatLeave(_) => (s.chat_leave, aurora_i18n::tr!("radar-chat-leave").to_owned()),
                Event::DrawEnter(_, d) => (s.draw_enter, aurora_i18n::tr!("radar-draw-enter", distance = meters(d))),
                Event::DrawLeave(_) => (s.draw_leave, aurora_i18n::tr!("radar-draw-leave").to_owned()),
                Event::RegionEnter(_, Some(d)) => (s.region_enter, aurora_i18n::tr!("radar-region-enter", distance = meters(d))),
                Event::RegionEnter(_, None) => (s.region_enter, aurora_i18n::tr!("radar-region-enter-unknown").to_owned()),
                Event::RegionLeave(_) => (s.region_leave, aurora_i18n::tr!("radar-region-leave").to_owned()),
                Event::AgeAlert(_, days) => (s.age_alert, aurora_i18n::tr!("radar-age-alert", days = days)),
            };
            if !wanted {
                continue;
            }
            let id = event.avatar();
            if s.toasts {
                let name = ui::people::radar_name(&self.world, &id, s.name_format);
                self.world.notifications.push(
                    crate::world::notifications::Kind::Info,
                    name,
                    text,
                    crate::world::notifications::Data::None,
                );
            } else {
                // the name is a link to the resident, as in Firestorm's chat
                self.world.system_message(format!("secondlife:///app/agent/{id}/about {text}"));
            }
        }
    }

    /// Voice session for the current region / parcel, microphone, push-to-talk.
    fn update_voice(&mut self) {
        let tuning = self.panels.settings && self.options_ui.tab == ui::options::TAB_AUDIO;
        // device lists for the preferences (listed once per opening)
        if tuning && !self.devices_listed {
            self.options_ui.output_devices = aurora_audio::output_devices();
            self.options_ui.input_devices = aurora_audio::input_devices();
            self.devices_listed = true;
        } else if !self.panels.settings {
            self.devices_listed = false;
        }
        let main = self.world.main();
        // global coordinates: region origin (from its handle) + local position
        let origin = self
            .world
            .main_region
            .map(|h| glam::DVec3::new((h >> 32) as f64, (h & 0xffff_ffff) as f64, 0.0))
            .unwrap_or_default();
        let global = |p: Vec3| origin + p.as_dvec3();
        let cam_rot = glam::Quat::from_mat4(&self.camera.view().inverse());
        let grid = match self.settings.grid {
            crate::settings::GridChoice::SecondLifeBeta => "aditi",
            _ => "agni",
        };
        let transmit = self.talking();
        let inputs = crate::voice::VoiceInputs {
            enabled: self.settings.audio.voice_enabled,
            in_world: self.in_world(),
            offline: self.demo,
            tuning,
            engine: self.audio_engine.as_ref(),
            http: &self.voice_http,
            agent_id: self.world.agent_id,
            grid,
            region: self.world.main_region,
            caps: main.map(|r| &*r.caps),
            voice_server: main.map(|r| r.voice_server.as_str()).unwrap_or(""),
            parcel: self.world.parcel.as_deref(),
            input_device: &self.settings.audio.input_device,
            mic_gain: self.settings.audio.mic_gain,
            transmit,
            avatar: (global(self.world.agent.position), self.world.agent.body_rotation()),
            listener: (global(self.camera.position), cam_rot),
            call: self.world.calls.active.as_ref().and_then(|c| c.channel.as_ref()),
        };
        self.voice.update(inputs);
        // the call's channel: its state and participants (the demo has no
        // voice server and simulates them)
        let server_type = main.map(|r| r.voice_server.clone()).unwrap_or_default();
        self.world.calls.voice_enabled = self.settings.audio.voice_enabled;
        self.world.calls.server_type = server_type;
        self.world.calls.reject_group = self.settings.audio.reject_group_calls;
        self.world.calls.reject_adhoc = self.settings.audio.reject_adhoc_calls;
        self.world.calls.reject_p2p = self.settings.audio.reject_p2p_calls;
        self.world.expire_call_invites();
        if self.volumes_save_at.is_some_and(|t| Instant::now() >= t) {
            self.volumes_save_at = None;
            self.world.calls.save_volumes();
        }
        // each resident's voice volume (LLSpeakerVolumeStorage)
        self.voice.apply_volumes(&self.world.calls.volumes);
        let report = if self.demo {
            let in_world = self.in_world();
            for ev in self.call_demo.scenario(&mut self.world, in_world) {
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
            }
            self.call_demo.report(&self.world, transmit)
        } else {
            self.voice.call_report()
        };
        if let Some((status, members)) = report {
            self.world.call_voice_update(status, members);
        }
        // a new call shows its conversation (started, accepted, answered by text)
        let call = self.world.calls.active.as_ref().map(|c| c.session);
        if call.is_some() && call != self.call_shown {
            ui::notifications::open_conversations(&self.egui_ctx, &mut self.panels.chat);
        }
        self.call_shown = call;
        // the microphone a call wants when it connects or ends (toggle mode)
        if let Some(on) = self.world.calls.mic.take()
            && !self.settings.audio.mic_hold
        {
            self.mic_on = on;
        }
        let world = &self.world;
        self.voice.apply_blocks(|id| world.voice_blocked(id));
        self.voice_connected = self.voice.connected;
        self.voice_level = self.voice.own_level;
        self.options_ui.mic_level = self.voice.mic_meter;
    }

    /// End of a shortcut capture (None = cancelled).
    fn finish_capture(&mut self, input: Option<Input>) {
        let Some(ui::options::Capture::Binding(action, slot)) = self.options_ui.take_capture() else {
            return;
        };
        if let Some(input) = input {
            let b = crate::keybinds::Binding::captured(input, self.mods());
            self.settings.keybinds.set(action, slot, Some(b));
            if action == Action::HudDrag {
                self.down.clear();
            }
            self.settings.save();
        }
    }

    /// What the build floater asked for (build::ui::Request).
    fn on_build_request(&mut self, r: crate::build::ui::Request) {
        use crate::build::ui::Request;
        match r {
            Request::TransferTask(request) => self.apply_inventory_action(ui::inventory::InvAction::TakeTask(request)),
            Request::ZoomFraction(f) => self.camera.set_zoom_fraction(f),
            Request::AboutLand => self.panels.about_land = true,
            // LLPanelLandInfo::onClickBuy: the land tool's selection
            Request::BuyLand => self.open_buy_land(buy_land::BuyLandSource::LandTool, false),
            Request::Profile(id) => self.profile_ui.open(&mut self.world, id),
            Request::GroupProfile(id) => self.group_profile_ui.open(&mut self.world, id),
            Request::PasteVector(which) => {
                let text = self.gfx.as_mut().and_then(|g| g.egui_state.clipboard_text()).unwrap_or_default();
                self.build.paste_vector(&mut self.world, &self.settings.build, which, &text);
            }
            Request::Script { object, item } => self.open_script(aurora_net::scripts::ScriptTarget { object, item }),
        }
        self.flush_build();
    }

    /// Send what the build tools queued.
    fn flush_build(&mut self) {
        for c in std::mem::take(&mut self.build.out) {
            self.send(c);
        }
    }

    fn demo_action_steps(&mut self) {
        // Exercise the real depth pick after the renderer is back in self.gfx.
        if !self.demo {
            return;
        }
        self.demo_script_steps();
        // AURORA_DEMO_HOVER=1: the cursor swept over the scene, then still
        if let Some(g) = &self.gfx
            && let Some(cursor) = crate::demo::hover_sweep(self.frame_count, g.renderer.size())
        {
            self.cursor_pos = cursor;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_ACTIONS") else {
            return;
        };
        if mode.ends_with("-ready") && !mode.starts_with("blend-") && !mode.starts_with("mask-") {
            return;
        }
        let id = crate::demo::action_mode_id(&mode);
        let click_frame = if mode.starts_with("linked-") { 724 } else { 284 };
        if self.frame_count == 260
            && (id == 0 || id == 980 || id == 981 || mode.starts_with("blend-") || mode.starts_with("mask-"))
            && let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::action_id(971))
            && let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, Instant::now(), 0)
        {
            let direction = (self.camera.position - pos).normalize_or(-Vec3::X);
            let rotation = glam::Quat::from_rotation_arc(Vec3::X, direction);
            self.world.apply(if mode.starts_with("blend-") || mode.starts_with("mask-") {
                crate::demo::alpha_click_overlay(&mode, pos + direction * 1.0, rotation)
            } else {
                crate::demo::action_overlay(if id == 980 { 980 } else { 981 }, pos + direction * 1.0, rotation)
            });
        }
        if self.frame_count == 275 && (mode == "pause" || mode == "open-media-playing") {
            self.media.play_parcel(&self.world, &self.settings.media);
        }
        if (283..=284).contains(&self.frame_count)
            && (mode == "pause" || mode == "open-media-playing")
            && let Some(m) = self.media.find_mut(&crate::media::MediaKey::Parcel)
        {
            // Offline state fixture; real status is supplied by the media plugin.
            m.status = aurora_media::MediaStatus::Playing;
        }
        if id != 0
            // Hold the late hover sample on target as well: native focus
            // events may have moved the cursor since the initial projection.
            && ((click_frame - 4..=click_frame).contains(&self.frame_count)
                || (mode.ends_with("-ready") && (640..=650).contains(&self.frame_count)))
            && let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::action_id(id))
            && let Some(pos) = Scene::object_transform(&self.world, idx, Instant::now(), 0).map(|(pos, _, _)| match id {
                985 => pos + Vec3::new(-1.05, 0.0, 0.7),
                986 => pos + Vec3::Z * 0.6,
                _ => pos,
            })
            && let Some(cursor) = self.build.cam.project_px(pos)
        {
            self.cursor_pos = cursor;
        }
        if id != 0 && self.frame_count == click_frame && !mode.ends_with("-ready") {
            if mode.ends_with("-build") {
                self.build.open_build(crate::build::Tool::Edit);
                self.click(click_pick::Click::Build);
            } else if mode.ends_with("-menu") {
                self.open_context_menu();
            } else {
                self.on_left_press();
            }
            if id != 975 && id != 982 {
                self.on_left_release();
            }
            self.demo_note(click_pick::DemoNote::Action(mode.clone()));
        }
        if (id == 975 || id == 982) && self.frame_count == 300 {
            self.cursor_pos.0 += 30.0;
        }
        if (id == 975 || id == 982) && self.frame_count == 325 {
            self.on_left_release();
        }
        if mode.ends_with("-confirm")
            && self.frame_count == 320
            && let Some(cmd) = self.interactions.confirm(&self.world, (id == 972).then_some(10))
        {
            self.send(cmd);
        }
    }

    /// CTRL + wheel cycles the edit selection before HUD, media or camera zoom.
    fn on_build_scroll(&mut self, delta: f32) -> bool {
        if self.build.replacement_active() {
            return true;
        }
        if !self.ctrl || self.alt || !self.build.open || self.build.tool != crate::build::Tool::Edit || self.camera.mouselook() {
            self.build.reset_selection_wheel();
            return false;
        }
        // A held drag owns the selection until release. Still consume the
        // gesture so the camera or a HUD cannot move underneath its handles.
        if self.left_down || self.build.manip.dragging() || self.hud_drag.is_some() || self.mouse_mode != MouseMode::None {
            self.build.reset_selection_wheel();
            return true;
        }
        let steps = self
            .build
            .selection_wheel_steps(delta, self.cursor_pos, self.settings.build.edit_linked);
        if steps == 0 {
            return true;
        }
        let mut hits = self
            .settings
            .show_huds
            .then_some(self.scene.lists.hud_view)
            .flatten()
            .map(|view| {
                self.scene
                    .edit_ray_hits(&self.world, view.ray(self.cursor_pos.0, self.cursor_pos.1), true)
            })
            .unwrap_or_default();
        if hits.is_empty()
            && let Some(ray) = self
                .gfx
                .as_ref()
                .and_then(|g| g.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1))
        {
            hits = self.scene.edit_ray_hits(&self.world, ray, false);
        }
        self.build.cycle_under_cursor(&mut self.world, &self.settings.build, &hits, steps);
        self.flush_build();
        true
    }

    /// Left press while building: handles, selection, create, terraform.
    /// Run by `click`; false also when it stopped to ask for the depth under
    /// the cursor (`ClickWait::waiting`), to run again when it has come.
    fn build_mouse_down(&mut self) -> bool {
        // Report Abuse's object picker takes the click (LLToolObjPicker)
        if self.report_abuse.picking {
            self.left_down = true;
            self.report_abuse_pick();
            return true;
        }
        if self.color_pipette() {
            self.left_down = true;
            self.sample_build_color();
            return true;
        }
        if !self.build.open
            || self.alt
            || (self.hud_drag_requested() && self.settings.show_huds && self.scene.hud_move_pick(&self.world, self.cursor_pos).is_some())
        {
            return false;
        }
        if self.build.tool == crate::build::Tool::Focus {
            return self.focus_tool_down();
        }
        if self.gfx.is_none() {
            return false;
        }
        let (x, y) = self.cursor_pos;
        let hud = (self.build.tool == crate::build::Tool::Edit)
            .then(|| self.scene.hud_edit_pick(&self.world, (x, y)))
            .flatten();
        let (hit, target, ray) = if let Some((idx, point, ray)) = hud {
            (Some(point), Some(idx), ray)
        } else if self.build.tool == crate::build::Tool::Edit && self.build.edit_mode == crate::build::EditMode::Face {
            // LLToolFace picks the current cursor ray. A delayed GPU depth
            // sample can still point at the previous object during Shift clicks.
            let ray = self.build.cam.ray(x, y);
            let target = self.scene.edit_ray_hits(&self.world, ray, false).first().copied();
            (None, target, ray)
        } else {
            let Some(picked) = self.click_pick() else {
                return false;
            };
            let (hit, target) = if matches!(self.build.tool, crate::build::Tool::Edit | crate::build::Tool::Grab) {
                self.scene
                    .selection_pick(&self.world, picked.ray, picked.depth, true, self.settings.draw_distance)
            } else {
                (
                    picked.depth,
                    picked.depth.and_then(|p| self.scene.pick_at(&self.world, p, Instant::now())),
                )
            };
            // a rigged mesh is selected where it is shown (Firestorm: with
            // the move tool only, LLToolCompTranslate)
            let (hit, target) = match self.designate(crate::scene::drawn::Gesture::Build, &picked, hit) {
                Some(d) => (Some(d.hit.point), Some(d.idx)),
                None => (hit, target),
            };
            (hit, target, picked.ray.unwrap_or_else(|| self.build.cam.ray(x, y)))
        };
        let mods = crate::build::Mods {
            shift: self.shift,
            ctrl: self.ctrl,
            alt: self.alt,
        };
        // the surface under the cursor: face tool, grab surface info
        let surface = match (self.build.tool, target) {
            (crate::build::Tool::Edit | crate::build::Tool::Grab, Some(i)) => self.scene.touch_surface(&self.world, i, ray),
            _ => None,
        };
        let used = self.build.mouse_down(
            &mut self.world,
            &mut self.settings.build,
            hit,
            target,
            surface,
            self.cursor_pos,
            mods,
        );
        self.flush_build();
        if used {
            self.left_down = true;
        }
        used
    }

    /// Focus tool press (LLToolCamera): focus on the clicked point, then
    /// the drag zooms, orbits or pans as the tool's radio or Ctrl / Shift say.
    fn focus_tool_down(&mut self) -> bool {
        let Some(picked) = self.click_pick() else {
            return false;
        };
        let (x, y) = self.cursor_pos;
        let hit = picked.depth;
        let ray = self.build.cam.ray(x, y);
        let now = Instant::now();
        if let Some(p) = hit {
            let idx = self.scene.pick_at(&self.world, p, now);
            let object = idx.and_then(|i| crate::camera::pick_focus_object(&self.world, i, now));
            self.camera.alt_focus(p, object, ray, &mut self.world.agent, &self.settings.camera);
        }
        self.camera.begin_drag(false);
        self.mouse_mode = MouseMode::FocusOrbit;
        self.set_mouselook_grab(true);
        self.left_down = true;
        true
    }

    /// Right button: orbit drag, context menu on a click, and walking forward
    /// while steering with the left button (a double click runs).
    fn on_right_button(&mut self, pressed: bool, over_ui: bool) {
        if self.hud_drag.is_some() {
            return;
        }
        let was_drag = self.right_drag;
        self.right_drag = pressed && (!over_ui || self.mouse_mode == MouseMode::Steer) && self.in_world();
        if self.mouse_walks() {
            self.walk_tap(0, true);
        } else if !pressed {
            self.check_temp_run();
        }
        if pressed {
            self.right_gesture = RightGesture::default();
        } else if was_drag && !self.right_gesture.orbit && self.mouse_mode == MouseMode::None {
            // a click (no drag): context menu
            self.open_context_menu();
        }
    }

    fn on_left_release(&mut self) {
        if self.clicks.waiting() {
            // after the press it ends, which still waits for its depth
            self.clicks.hold(click_pick::Held::LeftRelease);
            return;
        }
        if self.left_down && self.color_pipette() {
            if !self.egui_ctx.is_pointer_over_egui() {
                self.sample_build_color();
            }
            self.release_color_pipette();
        }
        self.update_hud_drag(self.scene.lists.hud_view);
        if let Some(cmd) = self.hud_drag.take().and_then(|drag| drag.release(&self.world)) {
            if self.demo {
                log::info!("demo HUD drag saved: {cmd:?}");
            }
            self.send(NetCommand::Build(cmd));
        }
        self.release_object_hold();
        self.left_down = false;
        self.build.mouse_up(&mut self.world, &self.settings.build);
        self.flush_build();
        self.media.on_release(aurora_media::Modifiers {
            control: self.ctrl,
            alt: self.alt,
            shift: self.shift,
        });
        if self.mouse_mode != MouseMode::None {
            self.mouse_mode = MouseMode::None;
            self.camera.end_drag();
            if !self.camera.mouselook() {
                self.set_mouselook_grab(false);
            }
            // the right button no longer walks: a double right click run ends
            self.check_temp_run();
        }
    }

    /// Show progress only for a known destination outside the current region.
    /// Unlike Firestorm's fallback for an unresolved landmark, wait for the
    /// server's destination before showing a screen for an unknown destination,
    /// so even these local teleports remain direct.
    fn begin_teleport(&mut self, dest: String, show_progress: bool) {
        self.drop_clicks();
        self.stop_autopilot(true);
        if self.hud_drag.is_some() {
            self.on_left_release();
        }
        self.release_object_hold();
        self.tp_dest = Some(dest);
        self.media.on_teleport();
        self.world.begin_teleport(show_progress);
    }

    fn show_teleport_progress(&mut self) {
        if self.settings.loading_backdrop {
            self.want_scene_capture = true;
        }
        let dest = self.tp_dest.take().unwrap_or_default();
        self.tp_overlay = Some(TpOverlay::new(dest, false));
        self.was_teleporting = true;
    }

    fn teleport_history(&mut self, back: bool) {
        let dest = if back {
            self.world.tp_history.go_back()
        } else {
            self.world.tp_history.go_forward()
        };
        if let Some(e) = dest {
            self.world
                .system_message(aurora_i18n::tr!("app-teleporting-to-2", value0 = e.region.to_string()));
            self.begin_teleport(e.region.clone(), Some(e.handle) != self.world.main_region);
            self.send(NetCommand::TeleportTo {
                handle: e.handle,
                position: e.position,
                look_at: Vec3::X,
            });
        }
    }

    /// Toolbar sit button (FSSelfForceSit, llviewermenu.cpp): stands up when
    /// sitting on an object or on the ground, sits on the ground otherwise.
    fn toggle_ground_sit(&mut self) {
        let f = if self.world.agent.is_sitting() {
            control::STAND_UP
        } else {
            control::SIT_ON_GROUND
        };
        self.send(NetCommand::OneShotControl(f));
    }

    /// "Always run" (Ctrl+R, the Controls window's Walk and Run).
    fn set_always_run(&mut self, on: bool) {
        if on == self.always_run {
            return;
        }
        self.always_run = on;
        self.send(NetCommand::SetAlwaysRun(self.always_run || self.temp_run.is_some()));
        self.world.system_message(if self.always_run {
            aurora_i18n::tr!("app-always-run-enabled")
        } else {
            aurora_i18n::tr!("app-always-run-disabled")
        });
    }

    /// Preferences › Camera › "Reset all", and the Controls window's reset:
    /// every camera setting back to its default, the view behind the avatar.
    fn reset_camera_all(&mut self) {
        self.settings.camera = crate::camera::CameraSettings::default();
        self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
    }

    fn toggle_fly(&mut self) {
        self.world.agent.flying = !self.world.agent.flying;
        if !self.world.agent.flying {
            // landing: a short UP_NEG nudge is implicit when FLY is cleared
            self.net.send(NetCommand::OneShotControl(control::STOP));
        }
    }

    fn update_move_input(&mut self, typing: bool, dt: f32) {
        let (kb, m) = (&self.settings.keybinds, self.mods());
        let k = |a: Action| kb.held(a, &self.down, m);
        let mut i = MoveInput::default();
        if !typing {
            i.forward = k(Action::Forward);
            i.back = k(Action::Back);
            let (left, right) = (k(Action::Left), k(Action::Right));
            if self.shift != self.settings.arrows_strafe {
                i.strafe_left = left;
                i.strafe_right = right;
            } else {
                i.turn_left = left;
                i.turn_right = right;
            }
            i.strafe_left |= k(Action::StrafeLeft);
            i.strafe_right |= k(Action::StrafeRight);
            i.up = k(Action::Up);
            // Page Down / C: crouch on the ground (crouch-walk while moving), descend in flight
            i.down = k(Action::Down);
        }
        // the Controls window's buttons, held like their keys
        let held = |a: Action| self.controls_held.contains(&a);
        i.forward |= held(Action::Forward);
        i.back |= held(Action::Back);
        if self.shift != self.settings.arrows_strafe {
            i.strafe_left |= held(Action::Left);
            i.strafe_right |= held(Action::Right);
        } else {
            i.turn_left |= held(Action::Left);
            i.turn_right |= held(Action::Right);
        }
        i.strafe_left |= held(Action::StrafeLeft);
        i.strafe_right |= held(Action::StrafeRight);
        i.up |= held(Action::Up);
        i.down |= held(Action::Down);
        if self.camera.free().is_some() {
            // the photo's free camera takes the movement keys: the avatar
            // stays where it is (no walk, turn, jump or flight start)
            self.photo_free_move(&i, dt);
            self.input = MoveInput::default();
            self.fb_since = None;
            self.lr_since = None;
            self.up_since = None;
            self.turn_since = None;
            self.cam_key_since = [None; 10];
            return;
        }
        if self.mouse_mode == MouseMode::Steer {
            // left button held on the avatar (Firestorm): the avatar faces the
            // camera; the arrows are camera-relative and turn the avatar toward
            // where it walks (→ = turn right and walk), the camera stays put
            let x = i.forward as i32 - i.back as i32;
            let y = (i.turn_left || i.strafe_left) as i32 - (i.turn_right || i.strafe_right) as i32;
            // + right button = walk forward (MMO style)
            let x = if self.right_drag && x == 0 && y == 0 { 1 } else { x };
            let moving = x != 0 || y != 0;
            let heading = self.world.agent.yaw + self.camera.orbit_yaw;
            let target = if moving { heading + (y as f32).atan2(x as f32) } else { heading };
            let d = (target - self.world.agent.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let step = STEER_TURN_RATE * dt;
            let turn = d.clamp(-step, step);
            self.world.agent.yaw += turn;
            self.camera.orbit_yaw -= turn;
            i.forward = moving;
            i.back = false;
            i.turn_left = false;
            i.turn_right = false;
            i.strafe_left = false;
            i.strafe_right = false;
        }
        i.run = self.temp_run.is_some();
        // nudge phase: the first 0.25 s of a press makes small steps
        let now = Instant::now();
        let since = |s: &mut Option<Instant>, held: bool| -> bool {
            if !held {
                *s = None;
                return false;
            }
            s.get_or_insert(now).elapsed().as_secs_f32() < NUDGE_TIME
        };
        i.nudge_fb = since(&mut self.fb_since, i.forward || i.back) && self.mouse_mode != MouseMode::Steer;
        i.nudge_lr = since(&mut self.lr_since, i.strafe_left || i.strafe_right);
        // holding jump for 0.5 s on the ground starts flying (AutomaticFly)
        if i.up {
            self.world.agent.record_jump_input(now);
            let held = self.up_since.get_or_insert(now).elapsed().as_secs_f32();
            if held > FLY_TIME && !self.world.agent.flying && !self.world.agent.seated && self.in_world() {
                self.world.agent.flying = true;
            }
        } else {
            self.up_since = None;
        }
        i.turn_held = if i.turn_left || i.turn_right {
            self.turn_since.get_or_insert(now).elapsed().as_secs_f32()
        } else {
            self.turn_since = None;
            0.0
        };
        // moving brings the camera back to the avatar (LLAgent::moveAt & co ->
        // resetView, FSResetCameraOnMovement): after an orbit the agent turns
        // to where the camera looks, the camera stays put
        let moving = i.forward || i.back || i.turn_left || i.turn_right || i.strafe_left || i.strafe_right || i.up || i.down;
        if moving && self.mouse_mode != MouseMode::FocusOrbit {
            let steering = self.mouse_mode == MouseMode::Steer;
            self.camera
                .reset_view(&mut self.world.agent, &self.settings.camera, true, false, true, steering);
        }
        self.input = i;
        // camera keys (Alt + arrows...), ramping up like get_orbit_rate
        use crate::keybinds::Action as A;
        const CAMERA_KEYS: [A; 10] = [
            A::CamOrbitCw,
            A::CamOrbitCcw,
            A::CamOrbitOver,
            A::CamOrbitUnder,
            A::CamZoomIn,
            A::CamZoomOut,
            A::CamPanLeft,
            A::CamPanRight,
            A::CamPanUp,
            A::CamPanDown,
        ];
        let mut rate = [0.0f32; 10];
        for (n, a) in CAMERA_KEYS.into_iter().enumerate() {
            let key = !typing && self.settings.keybinds.held(a, &self.down, m);
            if self.in_world() && (key || self.controls_held.contains(&a)) {
                let held = self.cam_key_since[n].get_or_insert(now).elapsed().as_secs_f32();
                rate[n] = if held < NUDGE_TIME { 0.05 + held * 0.95 / NUDGE_TIME } else { 1.0 };
            } else {
                self.cam_key_since[n] = None;
            }
        }
        let keys = crate::camera::CameraKeys {
            orbit_right: rate[0],
            orbit_left: rate[1],
            orbit_up: rate[2],
            orbit_down: rate[3],
            zoom_in: rate[4],
            zoom_out: rate[5],
            pan_left: rate[6],
            pan_right: rate[7],
            pan_up: rate[8],
            pan_down: rate[9],
        };
        self.camera.apply_keys(keys, &mut self.world.agent, &self.settings.camera, dt);
    }

    // ------------------------------------------------------------------ frame

    /// LLAppViewer::idleNameCache: display names of the nearby avatars and
    /// of every name looked up, through the People API once the main
    /// region's capabilities are known (legacy names without it).
    fn poll_names(&mut self) {
        let s = &self.settings;
        let options = crate::world::names::NameOptions {
            use_display_names: s.use_display_names,
            show_usernames: s.show_usernames,
            legacy_format: s.legacy_username_format,
            trim_resident: s.trim_resident,
        };
        let caps = self
            .world
            .main()
            .filter(|r| !r.caps.is_empty())
            .map(|r| r.caps.contains_key("GetDisplayNames"));
        let names = &mut self.world.social.avatar_names;
        names.options = options;
        names.notify_changes = s.display_name_notices;
        // nearby avatars, in view or not (tags, radar, minimap)
        for list in self.world.coarse.values() {
            for (id, _) in list {
                names.want(id);
            }
        }
        let mut legacy: Vec<uuid::Uuid> = std::mem::take(&mut names.legacy_asks);
        if let Some(has_cap) = caps {
            names.legacy_protocol = !has_cap;
            let ids = names.take_asks();
            if has_cap && !ids.is_empty() {
                self.send(NetCommand::RequestDisplayNames(ids));
            } else {
                legacy.extend(ids);
            }
        }
        if !legacy.is_empty() {
            self.send(NetCommand::RequestNames(legacy));
        }
        self.world.social.avatar_names.idle();
        self.world.sync_contact_aliases();
        self.world.flush_online_notices();
    }

    /// Display name cache file (avatar_name_cache.xml; per grid; none in
    /// demo mode).
    /// LLEnvironment's local_environment_data.bin, in the account folder
    /// (never in the offline demo).
    fn local_env_path(&self) -> Option<std::path::PathBuf> {
        (!self.demo && !self.world.agent_id.is_nil())
            .then(|| crate::settings::account_dir(&self.world.agent_id).join(crate::world::eep_env::LOCAL_ENV_FILE))
    }

    /// Keep the local environment for the next session when it changed
    /// (LLEnvironment::saveToSettings; the file goes when there is nothing
    /// to keep or EnvironmentPersistAcrossLogin is off).
    fn save_local_environment(&mut self) {
        if !self.world.eep.take_local_dirty() {
            return;
        }
        let Some(path) = self.local_env_path() else {
            return;
        };
        match self.world.eep.saved_local().filter(|_| self.settings.env_persist) {
            Some(data) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(&path, aurora_llsd::to_binary(&data)) {
                    log::warn!("local environment not saved ({e}): {}", path.display());
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// The last session's local environment (LLEnvironment::loadFromSettings).
    fn restore_local_environment(&mut self) {
        if !self.settings.env_persist {
            return;
        }
        let Some(path) = self.local_env_path() else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        match aurora_llsd::from_binary(&bytes) {
            Ok((data, _)) if data.is_map() => {
                log::info!("local environment of the last session restored");
                self.world.eep.restore_local(&data);
            }
            _ => log::warn!("local environment unreadable: {}", path.display()),
        }
    }

    fn name_cache_path(&self) -> Option<std::path::PathBuf> {
        use crate::settings::GridChoice;
        if self.demo {
            return None;
        }
        let file = match self.settings.grid {
            GridChoice::SecondLife => "avatar_name_cache.xml".to_owned(),
            GridChoice::SecondLifeBeta => "avatar_name_cache.aditi.xml".to_owned(),
            GridChoice::Custom => {
                let grid: String = self
                    .settings
                    .custom_login_uri
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
                    .collect();
                format!("avatar_name_cache.{}.xml", grid.trim_matches('_'))
            }
        };
        Some(crate::settings::cache_dir().join(file))
    }

    /// An action of the Contacts window (docked or torn off).
    fn on_contacts_action(&mut self, c: ui::contacts::ContactsAction, a: &mut UiActions) {
        use ui::contacts::ContactsAction;
        match c {
            ContactsAction::Call(ids) => match ids.as_slice() {
                [one] => self.on_ctx_action(ui::context::CtxAction::Call(*one)),
                _ if self.settings.audio.voice_enabled => self.world.start_conference(ids),
                _ => {}
            },
            ContactsAction::Im(id) => {
                self.world.ui_sounds.push(UiSound::StartIm);
                self.world.social.session_mut(id);
                self.world.social.focus_im = Some(id);
                self.panels.chat = true;
            }
            ContactsAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
            ContactsAction::OfferTeleport(id) => a.offer_tp.push(id),
            ContactsAction::SendTeleportOffer { to, message } => {
                let cmd = self.world.offer_teleport(to, &message);
                self.send(cmd);
            }
            ContactsAction::GroupChat(id) => {
                self.world.ui_sounds.push(UiSound::StartIm);
                self.world.start_group_chat(id);
                self.world.social.focus_im = Some(id);
                self.panels.chat = true;
            }
            ContactsAction::Net(cmd) => {
                match &cmd {
                    // LLAvatarTracker::terminateBuddy forgets the friend at once
                    NetCommand::TerminateFriendship(id) => {
                        self.world.social.end_friendship(id);
                    }
                    NetCommand::GrantUserRights { .. } => self.world.profile_command(&cmd),
                    _ => {}
                }
                self.send(cmd);
            }
            ContactsAction::OfferFriendship { to, message } => {
                let folder = self.world.calling_card_folder();
                self.send(ui::contacts::friendship_offer(to, message, folder));
            }
            ContactsAction::ToggleTornOff => {
                let torn_off = !self.settings.contacts.torn_off;
                self.settings.contacts.torn_off = torn_off;
                // FSFloaterIMContainer::removeFloater / addFloater
                self.panels.contacts = torn_off;
                self.chat_ui.contacts = !torn_off;
                if !torn_off {
                    self.panels.chat = true;
                }
            }
        }
    }

    /// Profile pictures wanted by the UI: request profiles, stream the
    /// picture textures and turn the decoded copies into egui textures.
    fn update_avatar_pics(&mut self, renderer: &mut Renderer) {
        for id in self.chat_ui.wanted_names.drain() {
            self.world.social.want_name(id);
        }
        let mut wanted: Vec<uuid::Uuid> = self.chat_ui.wanted_pics.drain().collect();
        if !self.demo && !self.world.agent_id.is_nil() {
            wanted.push(self.world.agent_id);
        }
        wanted.extend(self.profile_ui.wanted_pics.drain());
        wanted.extend(self.worldmap_ui.wanted_pics.drain());
        wanted.extend(self.contacts_ui.wanted_pics.drain());
        wanted.extend(self.plates.wanted_pics.drain());
        wanted.extend(self.search_ui.wanted_pics.drain());
        wanted.extend(self.group_profile_ui.wanted_pics.drain());
        wanted.extend(self.people_ui.wanted_pics.drain());
        wanted.extend(self.notif_ui.wanted_pics.drain());
        wanted.extend(self.land_ui.wanted_pics.drain());
        wanted.extend(self.region_ui.wanted_pics.drain());
        for agent in wanted {
            if self.avatar_pics.contains_key(&agent) {
                continue;
            }
            self.world.want_profile(agent);
            let Some(image) = self.world.profiles.get(&agent).map(|p| p.sl_image).filter(|i| !i.is_nil()) else {
                continue;
            };
            // two residents with the same picture share its texture (its
            // pixels are delivered once)
            if let Some(t) = self.avatar_pic_images.get(&image) {
                self.avatar_pics.insert(agent, t.clone());
                continue;
            }
            self.scene.textures.want_ui_image(renderer, image);
            if let Some((w, h, rgba)) = self.scene.textures.take_ui_image(&image)
                && rgba.len() == (w * h * 4) as usize
            {
                let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                let t = self.egui_ctx.load_texture(format!("pic-{agent}"), ci, egui::TextureOptions::LINEAR);
                // the Aurora profile's banner: a few pixels the filter blurs
                let banner = ui::profile::banner_image(w as usize, h as usize, &rgba);
                let banner = self
                    .egui_ctx
                    .load_texture(format!("banner-{image}"), banner, egui::TextureOptions::LINEAR);
                self.profile_ui.banners.insert(image, banner);
                self.avatar_pic_images.insert(image, t.clone());
                self.avatar_pics.insert(agent, t);
                if agent == self.world.agent_id && !self.demo {
                    if let Some(texture) = self.avatar_pics.get(&agent) {
                        self.login_form.portraits.insert(agent, texture.clone());
                    }
                    let path = crate::settings::account_dir(&agent).join("login_portrait.png");
                    rayon::spawn(move || {
                        let Some(image) = image::RgbaImage::from_raw(w, h, rgba) else {
                            return;
                        };
                        let image = image::DynamicImage::ImageRgba8(image).resize(512, 512, image::imageops::FilterType::Triangle);
                        let mut bytes = std::io::Cursor::new(Vec::new());
                        if image.write_to(&mut bytes, image::ImageFormat::Png).is_ok() {
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = crate::atomic_file::write(&path, &bytes.into_inner());
                        }
                    });
                }
            }
        }
        let mut images: Vec<uuid::Uuid> = self.profile_ui.wanted_images.drain().collect();
        images.extend(self.place_images.drain());
        images.extend(self.land_ui.wanted_images.drain());
        images.extend(self.region_ui.wanted_images.drain());
        images.extend(self.buy_land_ui.wanted_images.drain());
        images.extend(self.contacts_ui.wanted_images.drain());
        images.extend(self.people_ui.wanted_images.drain());
        images.extend(self.build.ui.wanted_images.drain());
        images.extend(self.appearance_ui.wanted_images.drain());
        images.extend(self.inventory_ui.wanted_images.drain());
        images.extend(self.worldmap_ui.wanted_images.drain());
        images.extend(self.env_editor.wanted_images.drain());
        images.extend(self.day_editor.wanted_images.drain());
        images.extend(self.favorite_images.drain());
        for note in self.task_notecards.values_mut() {
            images.extend(note.state.wanted_images.drain());
        }
        for image in images {
            if self.ui_images.contains_key(&image) {
                continue;
            }
            self.scene.textures.want_ui_image(renderer, image);
            if let Some((w, h, rgba)) = self.scene.textures.take_ui_image(&image)
                && rgba.len() == (w * h * 4) as usize
            {
                let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                let t = self.egui_ctx.load_texture(format!("img-{image}"), ci, egui::TextureOptions::LINEAR);
                self.ui_images.insert(image, t);
            }
        }
    }

    /// Place profiles and "Places": arrivals saved in the history, landmark
    /// assets and region handles, folders and group names asked by the
    /// lists, region names resolved, parcel requests.
    fn update_places(&mut self) {
        let now = Instant::now();
        if self.world.movement_complete
            && let Some(source) = self.demo_place.take()
        {
            // a landmark's profile shows its item: fetch the landmarks first
            let missing = match &source {
                crate::world::place_details::Source::Landmark { item, .. } => !self.world.inventory.items.contains_key(item),
                _ => false,
            };
            if missing {
                let inv = &self.world.inventory;
                let root = ui::places::system_folder(inv, ui::places::FOLDER_LANDMARKS);
                let folders: Vec<uuid::Uuid> = root
                    .into_iter()
                    .chain(
                        root.and_then(|r| inv.folders.get(&r))
                            .map(|f| f.children.clone())
                            .unwrap_or_default(),
                    )
                    .collect();
                for f in folders {
                    self.world.inventory.request(f);
                }
                self.demo_place = Some(source);
            } else {
                self.show_place_profile(source);
            }
        }
        // AURORA_DEMO_LANDMARK: World › Landmark this place, or a landmark of
        // the plaza already made, once the agent stands on a parcel
        let folder_ready = |kind: i32| {
            let inv = &self.world.inventory;
            crate::world::appearance::system_folder(inv, kind)
                .and_then(|f| inv.folders.get(&f))
                .is_some_and(|f| f.state == crate::world::inventory::FetchState::Fetched)
        };
        let ready = match self.create_landmark_ui.demo_pending {
            Some(ui::landmark_create::DemoStart::Quick(kind)) => folder_ready(kind),
            _ => true,
        };
        if self.world.movement_complete
            && self.world.parcel.is_some()
            && ready
            && let Some(start) = self.create_landmark_ui.demo_pending.take()
        {
            match start {
                ui::landmark_create::DemoStart::Window => self.open_create_landmark(None),
                ui::landmark_create::DemoStart::Quick(kind) => {
                    if let Some(cmd) = self.create_landmark_ui.quick(&self.world, kind) {
                        self.send(cmd);
                    }
                }
            }
        }
        self.world.flush_arrival(now);
        // LLLandmark::setRegionHandle: the agent's region needs no request
        if let Some(h) = self.world.main_region
            && let Some(id) = self.world.regions.get(&h).and_then(|r| r.info.as_ref()).map(|i| i.region_id)
        {
            self.world.landmarks.set_local_region(id, h);
        }
        for asset in self.places_ui.want_landmarks.drain() {
            self.world.landmarks.resolve(asset);
        }
        for folder in self.places_ui.want_folders.drain().chain(self.worldmap_ui.want_folders.drain()) {
            self.world.inventory.request(folder);
        }
        for g in self.place_groups.drain() {
            self.world.land.group_name(&g, &self.world.groups.groups);
        }
        for r in std::mem::take(&mut self.scene.landmark_results) {
            self.world.landmarks.on_fetch(r.key, r.data.as_deref().ok());
        }
        if self.demo {
            for (_, asset) in self.world.landmarks.take_wanted() {
                let data = crate::demo::place::landmark_asset(asset);
                self.world.landmarks.on_data(asset, data.as_deref().map(str::as_bytes));
            }
        } else if let Some(base) = self.world.viewer_asset_url() {
            for (key, asset) in self.world.landmarks.take_wanted() {
                self.net.fetcher.request(aurora_net::FetchRequest {
                    key,
                    url: crate::scene::textures::asset_url(&base, "landmark_id", &asset),
                    range: None,
                    // a window waits on it
                    priority: 1e12,
                    accept: "*/*",
                });
            }
        }
        self.world.place_details.update(&self.world.map, &mut self.world.landmarks, now);
        let mut cmds = self.world.place_details.take_commands();
        cmds.extend(self.world.landmarks.take_commands());
        // "Search": a key typed in the people tab waits for its name
        let names = &self.world.social.avatar_names;
        if let Some(id) = self.world.search.resolve_key(|id| names.complete(id), now) {
            names.want(&id);
        }
        cmds.extend(self.world.search.take_commands());
        for c in cmds {
            self.send(c);
        }
    }

    /// Map server tiles, connected regions known to the world map, map
    /// requests and teleports queued by the maps, tracking arrival.
    fn update_maps(&mut self) {
        if !self.demo
            && let Some(l) = &self.world.login
        {
            // login "map-server-url", else MapServerURL
            let url = l.raw["map-server-url"].as_str();
            let url = if url.trim().is_empty() {
                "https://map.secondlife.com/"
            } else {
                url
            };
            self.map_tiles.configure(url, self.net.runtime(), self.net.caps_http());
        }
        self.map_tiles.update(&self.egui_ctx);
        if self.demo && std::env::var_os("AURORA_DEMO_BAN").is_some() {
            // offline check of the ban lines: kept shown as if just pushed back
            if let Some(e) = self.world.apply(crate::demo::ban_collision()) {
                self.on_app_event(e);
            }
            self.world.map.blocked_alert = Some(Instant::now());
        }
        for r in self.world.regions.values() {
            if let Some(i) = &r.info {
                let (ox, oy) = aurora_net::handle_to_origin(r.handle);
                self.world
                    .map
                    .sims
                    .entry((ox / 256, oy / 256))
                    .or_insert_with(|| crate::world::worldmap::SimInfo {
                        name: i.name.clone(),
                        access: i.sim_access,
                        flags: i.region_flags,
                        image: uuid::Uuid::nil(),
                        size_x: r.heightmap.size_x.max(256),
                        size_y: r.heightmap.size_y.max(256),
                    });
                self.world.map.ratings.learn(&i.name, i.sim_access);
            }
        }
        // the regions of the place links drawn last frame whose rating is
        // not known yet: asked once, a few a second (RegionRatings)
        for name in ui::widgets::take_wanted_regions(&self.egui_ctx) {
            self.world.map.ratings.want(&name);
        }
        let now = Instant::now();
        let (due, later) = std::mem::take(&mut self.demo_lookups).into_iter().partition(|(at, _)| *at <= now);
        self.demo_lookups = later;
        for (_, cmd) in due {
            for ev in crate::demo::demo_reply(&cmd) {
                self.world.apply(ev);
            }
        }
        if !self.demo_lookups.is_empty() {
            self.egui_ctx.request_repaint_after(crate::demo::SLURLS_LOOKUP_DELAY / 4);
        }
        let me = ui::minimap::to_global(&self.world, self.world.agent.position);
        if self.world.movement_complete && !self.world.teleporting {
            self.world.map.check_arrival(me.0, me.1, self.world.agent.position.z);
        }
        // About Land requests (selection and its refresh, lists, covenant...)
        self.world.land.tick(Instant::now());
        for c in self.world.land.take_commands() {
            self.send(c);
        }
        // Region / Estate requests, in the panels' order
        for c in self.world.estate.take_commands() {
            self.send(c);
        }
        self.update_places();
        // the maturity preference's requests, and the teleport that waited
        // for it
        for c in self.world.maturity.take_commands() {
            self.send(c);
        }
        // experiences' details and permissions (questions, profile)
        for c in self.world.experiences.take_commands() {
            self.send(c);
        }
        // where the teleport offers lead, and the gods' lures accepted at once
        self.world.refresh_teleport_places();
        for c in self.world.teleport_invites.take_commands() {
            if matches!(c, NetCommand::AcceptLure { .. }) {
                self.begin_teleport(String::new(), false);
            }
            self.send(c);
        }
        for c in self.world.map.take_commands() {
            if let NetCommand::TeleportTo { handle, .. } = &c {
                let dest = self.world.map.track.as_ref().map(|t| t.label.clone()).unwrap_or_default();
                self.begin_teleport(dest, Some(*handle) != self.world.main_region);
            }
            self.send(c);
        }
        self.minimap_ui.prune(&self.world);
        // parcel overlays of regions we left (their lines would be stale on return)
        let regions = &self.world.regions;
        self.world.map.overlays.retain(|h, _| regions.contains_key(h));
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.demo && self.frame_count == 650 && std::env::var_os("AURORA_DEMO_EXIT").is_some() {
            self.request_exit();
        }
        if self.demo
            && self.frame_count == 800
            && let Ok(mode) = std::env::var("AURORA_DEMO_EXIT")
        {
            match mode.as_str() {
                "cancel" => self.confirm_exit(ui::exit::Action::Cancel),
                "logout" => self.confirm_exit(ui::exit::Action::Logout),
                "quit" => self.confirm_exit(ui::exit::Action::Quit),
                _ => {}
            }
        }
        if self.updater.poll(&self.settings, &mut self.login_info, self.net.runtime()) {
            self.shutdown(event_loop);
            return;
        }
        // Aurora's media plugins: downloaded once, when none are found (the
        // web pages of the profiles need them even with media disabled)
        let plugins_wanted = self.media.plugins(&self.settings.media).is_none();
        if std::mem::take(&mut self.options_ui.media_pack_retry) {
            self.plugin_pack.retry(self.net.runtime());
        }
        if self.plugin_pack.poll(plugins_wanted, self.net.runtime()) {
            self.media.set_pack_dir(self.plugin_pack.dir());
        }
        self.options_ui.media_pack = self.plugin_pack.state();
        ui::web_view::set_plugins_status(&self.egui_ctx, self.plugin_pack.state());
        self.poll_inventory_images();
        // a click whose depth is back already (the render thread finished
        // the last frame while this thread waited for its next one)
        self.resolve_clicks();
        self.frame_profile.lap(Lap::Between);
        let now = Instant::now();
        if let Some(action) = self.exit_prompt.finish(!self.in_world() || self.demo, now) {
            match action {
                ui::exit::Action::Quit => {
                    self.shutdown(event_loop);
                    return;
                }
                ui::exit::Action::Logout => self.back_to_login(None),
                ui::exit::Action::Cancel => {}
            }
        }
        if !self.exit_prompt.open && self.camera.mouselook() && self.focused && !self.cursor_hold.held() {
            self.set_mouselook_grab(true);
        }
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        if let Some(t) = self.quit_at
            && now >= t
        {
            event_loop.exit();
            return;
        }

        // logo / icons
        if let Some(rx) = &self.logo_rx
            && let Ok(l) = rx.try_recv()
        {
            self.logo = Some(self.egui_ctx.load_texture("logo", l.ui, egui::TextureOptions::LINEAR));
            if let (Some(g), Some((rgba, w, h))) = (&self.gfx, l.icon)
                && let Ok(icon) = winit::window::Icon::from_rgba(rgba, w, h)
            {
                g.window.set_window_icon(Some(icon));
            }
            self.logo_rx = None;
        }
        if self.skin.maybe_reload(&self.egui_ctx, self.settings.font_scale) {
            crate::crash::update(&self.skin.theme, &self.settings);
            self.palette = self.skin.theme.palette();
            self.cursors.set_palette(&self.palette);
            self.egui_ctx.set_zoom_factor(self.settings.ui_scale);
        }

        self.frame_profile.lap(Lap::Start);
        // ---- network events (time-boxed)
        let t_ev = Instant::now();
        self.world.events_this_frame = 0;
        self.world.script_debug.owner_say = self.settings.script_editor.owner_say_debug;
        self.sync_chat_log();
        while let Ok((sent, ev)) = self.net.events.try_recv() {
            if let Some(app_ev) = self.world.apply(ev) {
                self.on_app_event(app_ev);
            }
            let now = Instant::now();
            self.net_profile.event_applied(sent, now);
            if now.duration_since(t_ev) > Duration::from_millis(8) {
                break;
            }
        }
        self.net_profile.events_left(self.net.events.len());
        self.net_profile.corrections(self.world.agent.take_corrections());
        // a border crossing changed what positions are relative to: the
        // camera moves with the agent, in the same frame
        let shift = self.world.take_origin_shift();
        if shift != Vec3::ZERO {
            self.camera.shift(shift);
        }
        self.world.poll_inventory_cache();
        self.world.poll_transcripts();
        self.update_hover_height();
        self.perf.events_ms = t_ev.elapsed().as_secs_f32() * 1000.0;
        for anim in std::mem::take(&mut self.world.animation_stops) {
            self.send(NetCommand::AgentAnimation { anim, start: false });
        }
        self.apply_skeleton_resets();
        self.frame_profile.lap(Lap::Events);
        self.poll_ao();
        self.sync_static_eyes();
        if self.in_world() && self.last_social_poll.elapsed() > Duration::from_millis(500) {
            self.last_social_poll = Instant::now();
            self.poll_names();
            for id in self.world.take_profile_requests() {
                self.send(NetCommand::RequestProfile(id));
            }
            // outfit restore at login (queues the COF fetch below)
            let second_life = !matches!(self.settings.grid, crate::settings::GridChoice::Custom);
            self.world.update_outfit(second_life, Instant::now());
            for cmd in std::mem::take(&mut self.world.outfit.commands) {
                self.send(cmd);
            }
            // block list, groups and group chat sessions
            self.world.groups.mute_all_groups = self.settings.mute_all_groups;
            self.world.groups.mute_when_notices_off = self.settings.mute_groups_without_notices;
            self.world.groups.report_blocks = self.settings.report_blocks;
            if self.world.status.cfg != self.settings.autoresponse {
                self.world.status.cfg = self.settings.autoresponse.clone();
            }
            // AFKTimeout: away after some idle time, back on the next input
            let idle_limit = self.settings.autoresponse.away_after_minutes as u64 * 60;
            if idle_limit > 0 && !self.world.status.away && self.last_input.elapsed().as_secs() >= idle_limit {
                self.world.set_away(true);
                self.auto_away = true;
            } else if self.auto_away && self.last_input.elapsed().as_secs() < 1 {
                self.world.set_away(false);
                self.auto_away = false;
            }
            self.world.tick_social();
            self.update_gestures();
            for cmd in self.world.take_social_commands() {
                self.send(cmd);
            }
            // refused offers: to the Trash, whatever the inventory window does
            self.update_marketplace_listings();
            for change in self.world.take_inventory_edits() {
                let request = uuid::Uuid::new_v4();
                self.world_edits.insert(request);
                self.send(NetCommand::EditInventory { request, change });
            }
            // the environment selector's lists: library Environments, then the
            // inventory, once the selector has been opened
            if self.env_scan && self.env_ui.scan_left != 0 {
                self.env_ui.scan_left = crate::world::env_select::scan_step(&mut self.world.inventory);
            }
            self.save_local_environment();
            for id in self.world.eep.take_failed() {
                // FailedToFindSettings
                let name = [
                    crate::world::env_select::SettingsKind::Sky,
                    crate::world::env_select::SettingsKind::Water,
                    crate::world::env_select::SettingsKind::Day,
                ]
                .iter()
                .find_map(|k| self.env_ui.catalog().name_of(*k, id).map(str::to_owned))
                .unwrap_or_else(|| id.to_string());
                self.world.system_message(aurora_i18n::tr!(
                    "app-could-not-load-settings-from-the-database",
                    name = name.to_string()
                ));
            }
            let queue: Vec<(uuid::Uuid, bool)> = std::mem::take(&mut self.world.inventory.queue);
            let (mine, lib): (Vec<_>, Vec<_>) = queue.into_iter().partition(|(_, l)| !l);
            if !mine.is_empty() {
                self.send(NetCommand::FetchInventory {
                    folders: mine.into_iter().map(|(f, _)| f).collect(),
                    owner: self.world.agent_id,
                    library: false,
                });
            }
            if !lib.is_empty() {
                self.send(NetCommand::FetchInventory {
                    folders: lib.into_iter().map(|(f, _)| f).collect(),
                    owner: self.world.inventory.lib_owner,
                    library: true,
                });
            }
        }

        self.frame_profile.lap(Lap::Social);
        // Coalesce OS pointer moves before taking the renderer for this frame.
        if self.left_down && self.color_pipette() && !self.egui_ctx.is_pointer_over_egui() {
            self.sample_build_color();
        }
        let Some(mut gfx) = self.gfx.take() else {
            return;
        };
        self.apply_local_imports(&mut gfx.renderer);
        let t_up = Instant::now();
        let (w, h) = gfx.renderer.size();
        // a photo drawn at its own size this frame: its proportions, the
        // part of it this frame draws, the height its details are chosen for
        self.photo_update(&gfx.renderer);
        let photo = self.photo_frame(&mut gfx.renderer);
        let window_aspect = w as f32 / h.max(1) as f32;
        let aspect = photo.as_ref().map_or(window_aspect, |f| f.aspect);
        let photo_tile = photo.as_ref().map_or(glam::Mat4::IDENTITY, |f| f.tile);
        let lod_height = photo.as_ref().map_or(h, |f| f.height);
        let frame_proj = |camera: &crate::camera::Camera| match &photo {
            Some(f) => photo_tile * camera.photo_proj(f.aspect, window_aspect),
            None => camera.proj(aspect),
        };

        // ---- agent / camera
        let typing = self.egui_ctx.egui_wants_keyboard_input();
        self.update_move_input(typing, dt);
        self.update_autopilot(dt);
        self.update_audio();
        let mut edit_camera_frame = None;
        if self.in_world() {
            self.update_sounds();
        }
        if self.in_world() {
            let ml = self.camera.mouselook();
            // the clocks the other avatars and the objects move along
            // (world/motion.rs); offline there is no simulator to slow them
            self.world.advance_motion(Instant::now(), &self.net.stats.simulators.lock());
            if self.demo {
                // no simulator offline: move the avatar locally
                let p = self.world.agent.position;
                let floor = crate::demo::floor_at(p.x, p.y);
                self.world.agent.simulate_locally(&self.input, ml, Some(floor), dt);
                if let Some(mm) = crate::demo::agent_jitter() {
                    let p = self.world.agent.predicted_position() + crate::demo::agent_jitter_offset(self.frame_count, mm);
                    self.world.agent.constrain_prediction(p, false);
                }
            } else {
                let motion = *self.net.stats.simulator_motion.lock();
                if motion.handle == self.world.main_region {
                    self.world.predict_agent(motion.time_dilation, motion.packet_age());
                }
            }
            let camera_distance = self.camera.position.distance(self.world.agent.position);
            self.world.agent.update(&self.input, ml, dt, camera_distance);
            // seated, the agent's position is the one its seat gives it: the
            // map, the radar, the location bar and the voice read it
            if self.world.agent.seated
                && let Some(p) = crate::scene::Scene::seated_agent_position(&self.world, Instant::now())
            {
                self.world.agent.follow_seat(p);
            }
            let frame = crate::camera::FrameInput {
                dt,
                now: Instant::now(),
                steering: self.mouse_mode == MouseMode::Steer,
                build_mode: self.build.open,
                edit_tracking: self
                    .camera
                    .focus_object_id()
                    .filter(|id| self.build.edits_object(&self.world, id))
                    .map(|_| self.settings.build.follow_edit_object),
                draw_distance: self.settings.draw_distance,
            };
            if frame.edit_tracking.is_some() {
                // Evaluate the focused edit selection after the local drag,
                // so a stale simulator echo cannot move the camera for a frame.
                edit_camera_frame = Some(frame);
            } else {
                self.camera.update(&mut self.world, &self.settings.camera, &frame);
            }
            let at = self.camera.forward();
            self.world.agent.body_turn = self.world.update_bodies(Instant::now(), dt, at, ml);
            self.update_look_at(w as f32, h as f32);
            let mut c = self.world.agent.controls(&self.input, &self.camera, self.settings.draw_distance);
            // the walk to a double-clicked point (src/autopilot.rs)
            c.control_flags |= self.autopilot_flags;
            if self.autopilot.is_some() {
                c.flags |= crate::autopilot::AU_FLAGS_CLIENT_AUTOPILOT;
            }
            let moving = control::AT_POS | control::AT_NEG | control::LEFT_POS | control::LEFT_NEG | control::UP_POS | control::UP_NEG;
            if self.world.status.away && c.control_flags & moving != 0 {
                self.world.set_away(false);
                self.auto_away = false;
            }
            if self.world.status.away {
                c.control_flags |= control::AWAY;
            }
            // diagnostic: vertical controls sent to the simulator
            let vertical = c.control_flags & (control::UP_POS | control::UP_NEG | control::FLY);
            if vertical != self.last_vertical_flags {
                log::info!(
                    "controls: up {} down {} fly {} (z {:.2})",
                    vertical & control::UP_POS != 0,
                    vertical & control::UP_NEG != 0,
                    vertical & control::FLY != 0,
                    self.world.agent.server_pos.z
                );
                self.last_vertical_flags = vertical;
            }
            self.net.set_controls(c);
        }
        self.update_voice();
        self.update_radar();
        self.update_region_restart();
        let mut view = self.camera.view();
        let mut proj = frame_proj(&self.camera);
        let mut vp = proj * view;
        let mut cull = CullView::new(vp, self.camera.position, lod_height as f32, self.camera.fov_y);
        // debug: frozen culling view (move around to see what it drops)
        if self.settings.debug.freeze_culling {
            cull = *self.frozen_cull.get_or_insert(cull);
        } else {
            self.frozen_cull = None;
        }

        if let Some(still) = &mut self.scene.still {
            still.frame(crate::scene::still_check::View {
                camera: self.camera.position,
                target: self.camera.target,
                fov_y: self.camera.fov_y,
                agent: self.world.agent.position,
            });
        }
        self.frame_profile.lap(Lap::AgentCamera);
        // ---- scene
        let in_world = self.in_world();
        if in_world {
            self.scene.draw_distance = self.settings.draw_distance;
            self.scene.lod_factor = self.settings.lod_factor;
            self.scene.max_avatars = self.settings.max_avatars as usize;
            // reflection lists: planar reflections and reflection probe captures
            self.scene.reflections = self.settings.water_reflections > 0 || self.settings.mirrors > 0 || in_world;
            self.scene.fov_y = self.camera.fov_y;
            self.scene.agent_idx = self.world.objects.index_of_uuid(&self.world.agent_id);
            self.scene.debug_alpha = match self.settings.debug {
                d if d.show_alpha => Some(d.show_alpha_rigged),
                _ => None,
            };
            // finished jobs only leave cheap work to the main thread (their
            // data is staged for the GPU by the jobs): a short budget is
            // enough, and keeps a burst of results from stretching a frame
            // The shorter of the last two frames: one long frame (objects
            // arriving by thousands, a stall) is not a slow frame rate, and
            // gave the next one three times the budget, making it long too.
            let frame_time = Duration::from_secs_f32(dt.min(self.prev_dt));
            self.prev_dt = dt;
            let results_budget = crate::scene::textures::frame_share(frame_time, Duration::from_millis(1));
            // the full syncs of arriving objects get the same share; more
            // behind a loading or teleport screen, whose frames show nothing
            let covered = matches!(self.screen, Screen::Loading { .. }) || self.tp_overlay.as_ref().is_some_and(|o| o.end.is_none());
            self.scene.sync_budget = if covered { Duration::from_millis(8) } else { results_budget };
            self.scene.process_results(&mut gfx.renderer, &self.net, results_budget);
            self.frame_profile.lap(Lap::Results);
            if self.demo
                && let Some(mode) = &self.ao_demo
            {
                crate::ao::demo::signal(&mut self.world, self.frame_count, mode, now);
            }
            self.update_ao(now);
            // poses first: attachments follow their bone in the same frame
            let completed = self.scene.update_poses(&mut self.world, self.camera.position, now);
            self.scene.update_auto_hover(
                &mut self.world,
                self.settings.auto_hover_height,
                self.settings.auto_hover_offset,
                !self.hover_height_ui.previewing_offset(),
                now,
            );
            for anim in completed {
                self.ao_runtime.finished(anim);
                // LLAgent::requestStopMotion: stop the finished non-looping
                // animation and advance simulator-controlled transitions.
                let flags = self.world.agent.animation_stop_flags(anim, self.input.up, now);
                if flags != 0 {
                    self.send(NetCommand::OneShotControl(flags));
                }
                self.send(NetCommand::AgentAnimation { anim, start: false });
                log::debug!("own animation timed stop: {anim}, finish_anim={}", flags != 0);
            }
            self.frame_profile.lap(Lap::Poses);
            let mut exceptions = self.settings.render_exceptions_map();
            // blocked residents: grey silhouettes (FIRE-11783)
            for id in self.world.mutes.blocked_ids() {
                exceptions.insert(id, 3);
            }
            let friends: std::collections::HashSet<uuid::Uuid> = self.world.social.friends.iter().map(|f| f.id).collect();
            self.scene.update_complexity(
                &self.world,
                &crate::scene::complexity::Rules {
                    max: self.settings.max_complexity,
                    max_area: self.settings.max_attachment_area,
                    mode: self.settings.complexity_mode,
                    exceptions: &exceptions,
                    friends: &friends,
                },
            );
            self.frame_profile.lap(Lap::Complexity);
            let hud_size = gfx.window.inner_size();
            self.world.hud_aspect = match self.snapshots.capturing {
                // the HUD laid out for the photo's proportions
                Some(spec) => spec.width as f32 / spec.height.max(1) as f32,
                None => hud_size.width as f32 / hud_size.height.max(1) as f32,
            };
            self.refresh_local_huds();
            let drag_view =
                self.scene.lists.hud_view.map(|view| {
                    aurora_render::HudView::new([hud_size.width, hud_size.height], self.world.hud_zoom, view.min_x, view.max_x)
                });
            self.update_hud_drag(drag_view);
            if let Some(view) = drag_view {
                self.keep_scaled_huds_accessible(view);
            }
            // Apply the local drag after simulator updates and before scene sync.
            // LLManipScale::dragFace/dragCorner (indra/newview/llmanipscale.cpp)
            // also update locally every hover, independently of sendUpdates' 10 Hz throttle. Doing
            // this in UI preparation let an older simulator echo reach the GPU
            // for one frame, before the manipulator restored the current size.
            self.build.cam = crate::build::geom::Cam::new(
                self.camera.position,
                self.camera.target,
                vp,
                self.camera.fov_y,
                w as f32,
                h as f32,
                self.egui_ctx.pixels_per_point(),
            );
            if self.build.hud_selected(&self.world) && !self.settings.show_huds {
                self.build.deselect_all(&self.world);
            } else if self.build.tool == crate::build::Tool::Edit
                && self.build.hud_selected(&self.world)
                && let Some(view) = drag_view
            {
                self.build.cam = crate::build::geom::Cam::for_hud(view, self.egui_ctx.pixels_per_point());
            }
            let pan = if self.build.open && self.build.tool == crate::build::Tool::Edit && self.camera.third_person() {
                self.build
                    .manip
                    .camera_pan(&self.settings.build, &self.build.cam, self.cursor_pos, dt)
            } else {
                Vec3::ZERO
            };
            let panned = self.camera.pan_edit(pan, &mut self.world.agent, &self.settings.camera);
            if panned {
                self.build.cam = crate::build::geom::Cam::new(
                    self.camera.position,
                    self.camera.target,
                    self.camera.proj(aspect) * self.camera.view(),
                    self.camera.fov_y,
                    w as f32,
                    h as f32,
                    self.egui_ctx.pixels_per_point(),
                );
            }
            let over_ui = self.egui_ctx.is_pointer_over_egui() || self.egui_ctx.egui_wants_pointer_input();
            let mods = crate::build::Mods {
                shift: self.shift,
                ctrl: self.ctrl,
                alt: self.alt,
            };
            self.build
                .update(&mut self.world, &self.settings.build, self.cursor_pos, over_ui, mods);
            self.scene.selection_wire = self.build.wire_selection(&self.world, &self.settings.build);
            // another avatar's attachment the open menu is about, outlined
            // like a child prim of a selection (it cannot be selected)
            for idx in self.menu_outline() {
                self.scene.selection_wire.entry(idx).or_insert(false);
            }
            for (idx, current) in self.inspect_outline() {
                self.scene.selection_wire.entry(idx).or_insert(current);
            }
            self.scene.selection_silhouettes = self.build.silhouette_selection(&self.world, &self.settings.build);
            let selected_faces = self.build.face_selection(&self.world, &self.settings.build);
            self.scene.set_face_selection(&mut self.world, selected_faces);
            if std::mem::take(&mut self.build.ui.reveal_faces) {
                self.scene.reveal_selected_faces();
            }
            self.flush_build();
            let camera_changed = edit_camera_frame.is_some() || panned;
            if let Some(frame) = edit_camera_frame {
                self.camera.update(&mut self.world, &self.settings.camera, &frame);
            }
            if camera_changed {
                view = self.camera.view();
                proj = frame_proj(&self.camera);
                vp = proj * view;
                cull = CullView::new(vp, self.camera.position, lod_height as f32, self.camera.fov_y);
                if self.settings.debug.freeze_culling {
                    cull = *self.frozen_cull.get_or_insert(cull);
                }
                self.scene.fov_y = self.camera.fov_y;
                self.build.cam = crate::build::geom::Cam::new(
                    self.camera.position,
                    self.camera.target,
                    vp,
                    self.camera.fov_y,
                    w as f32,
                    h as f32,
                    self.egui_ctx.pixels_per_point(),
                );
            }
            self.scene.sync(&mut gfx.renderer, &mut self.world, &cull);
            self.frame_profile.lap(Lap::Sync);
            {
                // web pages and videos on faces (LLViewerMedia::updateMedia)
                let a = &self.settings.audio;
                let channel = |i: usize| if a.muted[i] || !a.enabled[i] { 0.0 } else { a.volume[i] };
                let cache_dir = crate::settings::cache_dir();
                let frame = crate::media::MediaFrame {
                    view: &cull,
                    camera: self.camera.position,
                    settings: &self.settings.media,
                    autoplay: a.media_autoplay,
                    // demo captures stay silent unless AURORA_DEMO_SOUND is set
                    channel_volume: if self.demo && std::env::var_os("AURORA_DEMO_SOUND").is_none() {
                        0.0
                    } else {
                        channel(0) * channel(5)
                    },
                    agent_speed: self.world.agent.velocity.length(),
                    window_focused: self.focused,
                    net: (!self.demo).then(|| (self.net.runtime(), self.net.caps_http())),
                    cache_dir: &cache_dir,
                    language: aurora_i18n::language(),
                };
                self.media.update(&mut self.world, &mut self.scene, &mut gfx.renderer, &frame);
                self.options_ui.media_plugins = self
                    .media
                    .plugins_found()
                    .map(|p| p.plugin_dir.display().to_string())
                    .unwrap_or_default();
                self.options_ui.media_running = self.media.loaded_count();
                for msg in self.media.messages.drain(..) {
                    self.world.system_message(msg);
                }
                // mouse moves over the focused media
                self.media_cursor = None;
                if self.media.focus.is_some()
                    && !self.egui_ctx.is_pointer_over_egui()
                    && let Some(ray) = gfx.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1)
                {
                    let mods = aurora_media::Modifiers {
                        control: self.ctrl,
                        alt: self.alt,
                        shift: self.shift,
                    };
                    self.media.build_mode = self.build.open;
                    let displayed = gfx.renderer.hover_pick(self.cursor_pos.0, self.cursor_pos.1);
                    let point = if self.build.open {
                        self.scene
                            .interaction_point(&self.world, Some(ray), displayed, true, self.settings.draw_distance)
                    } else {
                        self.scene
                            .hover_pick(&self.world, Some(ray), displayed, self.settings.draw_distance)
                            .0
                    };
                    let depth = point.map(|p| (p - ray.0).dot(ray.1));
                    let (ray, depth, hud) = self
                        .scene
                        .hud_pick(&self.world, self.cursor_pos, true)
                        .map_or((ray, depth, false), |(_, p, r)| (r, Some((p - r.0).dot(r.1)), true));
                    if let Some(c) = self.media.on_hover(&self.world, &self.scene, ray, depth, mods, hud) {
                        self.media_cursor = Some(match c.as_str() {
                            "UI_CURSOR_HAND" | "hand" => egui::CursorIcon::PointingHand,
                            "UI_CURSOR_IBEAM" | "ibeam" => egui::CursorIcon::Text,
                            _ => egui::CursorIcon::Default,
                        });
                    }
                }
            }
            self.frame_profile.lap(Lap::Media);
            // avatars still loading: clouds (Firestorm) and progress bars
            self.scene.update_loading(&self.world, Instant::now());
            // the demo avatars have no real bakes: no clouds offline unless
            // AURORA_DEMO_CLOUD is set
            static DEMO_CLOUD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            let mode = if self.demo && !*DEMO_CLOUD.get_or_init(|| std::env::var_os("AURORA_DEMO_CLOUD").is_some()) {
                2
            } else {
                self.settings.loading_avatars
            };
            self.scene.hide_loading = mode == 0;
            self.scene.loading_bars = mode < 2;
            let clouds: Vec<uuid::Uuid> = if mode == 0 {
                self.scene.loading_avatars().keys().copied().collect()
            } else {
                Vec::new()
            };
            self.scene.particles.sync_clouds(&clouds);
            self.scene
                .banlines
                .update(&mut gfx.renderer, &self.world, self.settings.maps.ban_lines, self.camera.position);
            self.frame_profile.lap(Lap::Extras);
            self.scene.build_lists(&mut gfx.renderer, &cull, self.settings.shadows);
            self.scene
                .build_huds(&self.world, [hud_size.width, hud_size.height], self.settings.show_huds);
            self.frame_profile.lap(Lap::Lists);
            if self.demo && std::env::var_os("AURORA_DEMO_ANIMESH").is_some() && self.frame_count.is_multiple_of(120) {
                self.scene.log_demo_animesh(&self.world);
            }
            // AURORA_DEMO_ANIMESH=churn: the ground animesh takes a new mesh
            // asset; the one it leaves is freed with its skin bindings
            if self.demo
                && self.frame_count > 0
                && self.frame_count.is_multiple_of(crate::demo::ANIMESH_CHURN_FRAMES)
                && crate::demo::animesh_churn()
            {
                let generation = (self.frame_count / crate::demo::ANIMESH_CHURN_FRAMES) as u32;
                self.scene
                    .install_demo_animesh_mesh(&mut gfx.renderer, crate::demo::animesh_churn_mesh(generation));
                for ev in crate::demo::animesh_churn_events(generation) {
                    self.world.apply(ev);
                }
            }
            self.options_ui.vram = (gfx.renderer.info.vram_mb, gfx.renderer.info.integrated);
            // Preferences › Debug › Detail levels: what the overlay labels
            self.lod_infos = if self.settings.debug.lod_info {
                let range = ui::debug_overlay::RANGE;
                self.scene
                    .lod_infos(&self.world, &gfx.renderer.records, self.camera.position, Instant::now(), range)
            } else {
                Vec::new()
            };
            // diagnostic: our attachments 40 s and 120 s after arrival
            let since_arrival = self.move_complete_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            if (self.attachments_logged == 0 && since_arrival >= 40) || (self.attachments_logged == 1 && since_arrival >= 120) {
                self.attachments_logged += 1;
                self.scene.log_attachments(&self.world, self.world.agent_id, &gfx.renderer.textures);
                // and the avatars within 10 m
                let me = self.world.agent.position;
                let near: Vec<uuid::Uuid> = self
                    .world
                    .objects
                    .iter()
                    .filter(|(_, o)| o.is_avatar() && o.full_id != self.world.agent_id && o.position.distance(me) < 10.0)
                    .map(|(_, o)| o.full_id)
                    .collect();
                for id in near {
                    self.scene.log_attachments(&self.world, id, &gfx.renderer.textures);
                }
            }
            if std::mem::take(&mut self.options_ui.describe_nearby) {
                // scene space: the agent avatar's drawn center
                let me = self.scene.agent_idx.and_then(|i| self.scene.gpu.get(i)).map(|g| g.center);
                if let Some(me) = me {
                    self.scene.log_nearby(&self.world, me, 12.0);
                }
            }
            let texture_budget = self
                .settings
                .texture_budget(gfx.renderer.info.vram_mb, gfx.renderer.info.integrated);
            self.scene
                .stream(&mut gfx.renderer, &self.net, &self.world, texture_budget * 1024 * 1024, frame_time);
        }
        self.perf.update_ms = t_up.elapsed().as_secs_f32() * 1000.0;
        self.frame_profile.lap(Lap::Stream);
        // the parts of the `Params` lap (`p_*` in the profile)
        let mut t_part = Instant::now();

        // ---- loading -> world transition
        if let Screen::Loading { since } = self.screen {
            let moved = self.move_complete_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
            let terrain = self.world.terrain_progress();
            if self.world.movement_complete && since.elapsed() > Duration::from_millis(1500) && (terrain > 0.97 || moved > 6.0) {
                self.screen = Screen::World;
                self.world.notifications.started = true;
                self.camera.snap();
                // the loading screen fades out over the world
                let mut o = TpOverlay::new(String::new(), true);
                o.start = since;
                o.shown = 1.0;
                o.end = Some(Instant::now());
                self.tp_overlay = Some(o);
                self.was_teleporting = self.world.teleporting;
            }
        }
        // diagnostic after an arrival: what the simulator and our avatar say
        // (T-pose / stuck after a teleport)
        if let (Some(&at), Some(t)) = (self.tp_checks.last(), self.move_complete_at)
            && t.elapsed().as_secs_f32() >= at
        {
            self.tp_checks.pop();
            let a = &self.world.agent;
            let i = &self.input;
            let input = i.forward || i.back || i.strafe_left || i.strafe_right || i.up || i.down;
            log::info!(
                "tp check +{at:.0} s: agent {:.1?} (server {:.1?}, {:.1} s ago, vel {:.1?}), flying {}, seated {}, moving keys {input}; {}",
                a.position,
                a.server_pos,
                a.server_age(),
                a.velocity,
                a.flying,
                a.seated,
                self.scene.own_avatar_diag(&self.world, self.camera.position)
            );
        }
        // diagnostic: movement keys held for 2 s but the simulator does not move us
        {
            let i = &self.input;
            let moving = (i.forward || i.back || i.strafe_left || i.strafe_right || i.up || i.down)
                && matches!(self.screen, Screen::World)
                && !self.world.agent.seated;
            let server = self.world.agent.server_pos;
            match (moving, self.stuck_watch) {
                (false, _) => self.stuck_watch = None,
                (true, None) => self.stuck_watch = Some((Instant::now(), server)),
                (true, Some((t, p))) => {
                    let quiet = self.stuck_logged.is_none_or(|l| l.elapsed() > Duration::from_secs(10));
                    if t.elapsed() > Duration::from_secs(2) && server.distance(p) < 0.1 && quiet {
                        self.stuck_logged = Some(Instant::now());
                        let a = &self.world.agent;
                        log::warn!(
                            "agent stuck: movement keys held 2 s, simulator position {:.1?} unchanged ({:.1} s since its last report), flying {}; {}",
                            a.server_pos,
                            a.server_age(),
                            a.flying,
                            self.scene.own_avatar_diag(&self.world, self.camera.position)
                        );
                    }
                }
            }
        }
        // ---- teleport screen
        self.backdrop.poll(&self.egui_ctx);
        if let Some(t0) = self.demo_tp_start {
            // offline demo teleport: the real steps, landing where we were
            let e = t0.elapsed().as_secs_f32();
            let w = &mut self.world;
            if e >= 3.2 {
                self.demo_tp_start = None;
                w.tp_progress = 0.6;
                w.tp_status = aurora_i18n::tr!("common-loading-region").into();
                if let (Some(position), Some(handle)) = (self.demo_tp_dest.take(), w.main_region) {
                    let ev = NetEvent::AgentMovementComplete {
                        handle,
                        position,
                        look_at: Vec3::X,
                    };
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
                let w = &mut self.world;
                w.teleporting = false;
            } else if e >= 2.4 {
                w.tp_progress = w.tp_progress.max(0.5);
                w.tp_status = aurora_i18n::tr!("common-connecting-to-region").into();
            } else if e >= 0.9 {
                w.tp_progress = (0.15 + (e - 0.9) * 0.18).min(0.42);
                w.tp_status = if e < 1.6 {
                    aurora_i18n::tr!("common-sending-to-destination")
                } else {
                    aurora_i18n::tr!("common-contacting-the-new-region")
                }
                .into();
            } else if e >= 0.35 {
                w.tp_progress = w.tp_progress.max(0.10);
                w.tp_status = aurora_i18n::tr!("common-teleport-accepted").into();
            }
        }
        // TeleportLocal can also resolve an initially unknown destination.
        // Drop a previous teleport's fade immediately, keeping only login's.
        if !self.world.tp_show_progress {
            if self.tp_overlay.as_ref().is_some_and(|o| !o.loading_end) {
                self.tp_overlay = None;
                self.was_teleporting = false;
            }
            if !self.world.teleporting {
                self.tp_dest = None;
            }
        }
        let tp = self.world.teleporting && self.world.tp_show_progress && matches!(self.screen, Screen::World);
        if tp && !self.was_teleporting {
            // keep the view we are leaving for the teleport screen
            self.show_teleport_progress();
        } else if !tp
            && self.was_teleporting
            && let Some(o) = self.tp_overlay.as_mut()
        {
            if self.world.tp_failed {
                o.end.get_or_insert(Instant::now());
            } else {
                o.arrived.get_or_insert(Instant::now());
            }
        }
        self.was_teleporting = tp;
        if let Some(o) = self.tp_overlay.as_mut() {
            // target: the teleport steps, then the new region loading
            let target = match o.arrived {
                _ if o.loading_end => 1.0,
                None => self.world.tp_progress.max(0.05),
                Some(at) => {
                    let terrain = self.world.terrain_progress();
                    let st = self.scene.textures.stats;
                    let tex = st.loaded as f32 / st.total.max(1) as f32;
                    let ready = (terrain > 0.97 && at.elapsed() > Duration::from_millis(800)) || at.elapsed() > Duration::from_secs(10);
                    if ready {
                        1.0
                    } else {
                        0.6 + 0.4 * (0.75 * terrain + 0.25 * tex).clamp(0.0, 1.0) * 0.97
                    }
                }
            };
            // glide toward the target, never backwards
            let k = 1.0 - (-2.6 * dt).exp();
            o.shown = (o.shown + (target - o.shown) * k).max(o.shown).min(1.0);
            if target >= 1.0 && o.shown > 0.985 {
                o.end.get_or_insert(Instant::now());
            }
            // no answer from the simulator: give up the screen
            if o.arrived.is_none() && o.start.elapsed() > Duration::from_secs(90) {
                o.end.get_or_insert(Instant::now());
            }
            if o.end.is_some_and(|e| e.elapsed().as_secs_f32() > ui::loading::FADE_SECONDS) {
                self.tp_overlay = None;
            }
        }
        t_part = self.scene.parts.lap(Part::ParamsScreen, t_part);

        // ---- environment: local preset > parcel > region (or default day),
        // with crossfades; the simulator sun only for regions without EEP
        self.world.eep.set_manual_transition(self.settings.env_manual_transition);
        let (sky_frame, water_frame) = self.world.environment_frames(&mut self.panels.time_of_day);
        self.world.sky_directions = (sky_frame.sun_direction(), sky_frame.moon_direction());
        // cloud scroll (LLEnvironment::updateCloudScroll: rate / 100 per
        // second), held still by the photo's light (pauseCloudScroll)
        if !self.photo_clouds_paused() {
            self.world.cloud_scroll += sky_frame.cloud_scroll_rate * dt / 100.0;
        }
        self.world.cloud_scroll = glam::Vec2::new(
            self.world.cloud_scroll.x.rem_euclid(1024.0),
            self.world.cloud_scroll.y.rem_euclid(1024.0),
        );
        let mut env = Environment::from_eep(&sky_frame, &water_frame, self.world.cloud_scroll, self.settings.draw_distance);
        if in_world {
            // what the sky source is: region, day cycle, altitude track, preset
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                self.world.main_region.hash(&mut h);
                self.panels.time_of_day.hash(&mut h);
                // local preset / parcel / region day / default, with its id
                format!("{:?}", self.world.eep.shown_source()).hash(&mut h);
                h.finish()
            };
            if self.sky_log.due(Instant::now(), key) {
                log::info!("{}", env.describe(&sky_frame));
            }
        }
        let slots = self.scene.env_textures(&mut gfx.renderer, env.textures);
        env.sky.cloud_texture = slots[0];
        env.sky.sun_texture = slots[1];
        env.sky.moon_texture = slots[2];
        env.water.normal_texture = slots[3];
        env.sky.rainbow_texture = slots[4];
        env.sky.halo_texture = slots[5];
        t_part = self.scene.parts.lap(Part::ParamsEnv, t_part);
        let mirror = if in_world && self.settings.mirrors > 0 {
            self.scene.find_mirror(&self.world, &cull, Instant::now())
        } else {
            None
        };
        t_part = self.scene.parts.lap(Part::ParamsMirror, t_part);
        // particles
        if in_world {
            let max_particles = self.settings.max_particles as usize;
            let mut out = std::mem::take(&mut self.scene.lists.particles);
            let scene = &mut self.scene;
            scene.particles.update(
                &mut gfx.renderer,
                &mut scene.textures,
                &mut self.world,
                dt,
                self.camera.position,
                max_particles,
                self.settings.draw_distance,
                &mut out,
                &mut scene.parts,
            );
            self.scene.lists.particles = out;
            t_part = Instant::now();
        }
        let (lights, spot_shadows) = if in_world {
            let view = crate::scene::lights::LightView {
                eye: self.camera.position,
                forward: self.camera.forward(),
                dt,
                projector_shadows: self.settings.projector_shadows && self.settings.shadow_quality > 0,
            };
            self.scene.local_lights(&self.world, &mut gfx.renderer, &view)
        } else {
            (Vec::new(), [None; 2])
        };
        t_part = self.scene.parts.lap(Part::ParamsLights, t_part);
        let probes = if in_world {
            let slots = self.settings.probe_count.clamp(8, 64);
            if self.probes.slots != slots || self.probes.level != self.settings.reflection_probes {
                self.probes.slots = slots;
                self.probes.level = self.settings.reflection_probes.min(3);
                self.probes.reset();
            }
            self.probes
                .update(&self.scene, &self.world, self.camera.position, env.probe_ambiance, Instant::now())
        } else {
            aurora_render::ProbeFrame::default()
        };
        let fp = FrameParams {
            selection_color: egui::Rgba::from(self.palette.violet_light).to_array(),
            selection_pulse: self.scene.selection_pulse(Instant::now()),
            selection_scale: self.egui_ctx.pixels_per_point(),
            view,
            proj,
            camera_pos: self.camera.position,
            near: self.camera.near,
            far: self.settings.draw_distance,
            fov_y: self.camera.fov_y,
            time: self.start.elapsed().as_secs_f32(),
            sun_dir: env.sun_dir,
            sun_visible: env.sun_visible,
            sun_color: env.sun_color,
            sky_zenith: env.zenith,
            sky_horizon: env.horizon,
            ground_color: env.ground,
            fog_color: env.fog_color,
            fog_density: env.fog_density,
            exposure: self.photo_exposure(),
            sharpen: self.settings.sharpen,
            glow: self.settings.glow,
            water_height: self.world.main_water_height(),
            shadows: self.settings.shadow_quality > 0 && matches!(self.screen, Screen::World),
            shadow_distance: self.settings.shadow_params().1,
            lights,
            spot_shadows,
            clear_color: Vec4::new(env.fog_color.x, env.fog_color.y, env.fog_color.z, 1.0),
            sky: env.sky,
            water: env.water,
            mirror,
            probes,
            wireframe: self.settings.debug.wireframe,
            debug_glow: self.settings.debug.glow_view,
            blend_late_discard: self.frame_profile.blend_late_discard(),
            all_lights: self.frame_profile.all_lights(),
            probe_walk_list: self.frame_profile.probe_walk_list(),
            debug: self.settings.debug.render(),
            dof: {
                // the image's own vertical field of view (a photo's may be
                // wider than the window's)
                let lens_proj = match &photo {
                    Some(f) => self.camera.photo_proj(f.aspect, window_aspect),
                    None => self.camera.proj(aspect),
                };
                let fov = 2.0 * (1.0 / lens_proj.y_axis.y.max(1e-6)).atan();
                self.dof_frame(&mut gfx.renderer, dt, fov, lod_height, h)
            },
        };
        self.scene.parts.lap(Part::ParamsProbes, t_part);

        self.frame_profile.lap(Lap::Params);
        // ---- UI
        let t_ui = Instant::now();
        self.world.objects.refresh_labelled();
        if self.system_fonts_pending && ui::fonts::fallback::ready().is_some() {
            self.system_fonts_pending = !ui::fonts::apply(&self.egui_ctx, &self.settings.font);
        }
        let mut raw = gfx.egui_state.take_egui_input(&gfx.window);
        self.demo_script_input(&mut raw);
        if self.demo && self.capture.is_some() {
            raw.events.extend(self.login_form.motion.demo_events(self.frame_count));
            raw.events.extend(self.login_form.manager.demo_events(self.frame_count));
        }
        // Inline-edit captures simulate logical focus without activating the
        // native window or taking the user's keyboard (with_active(false)).
        if self.demo
            && self.capture.is_some()
            && (ui::color_picker::demo::mode().is_some()
                || std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| {
                    matches!(
                        v.as_str(),
                        "rename" | "new-script" | "new-note" | "new-folder" | "resize-left" | "resize-right"
                    ) || v.starts_with("note-")
                        || v.starts_with("clipboard-")
                }))
        {
            raw.focused = true;
            raw.events.retain(|e| !matches!(e, egui::Event::WindowFocused(false)));
            if let Some(viewport) = raw.viewports.get_mut(&raw.viewport_id) {
                viewport.focused = Some(true);
            }
        }
        if self.demo
            && self.capture.is_some()
            && let Ok(view) = std::env::var("AURORA_DEMO_INVENTORY")
            && matches!(view.as_str(), "resize-left" | "resize-right")
            && let Some(rect) = self.egui_ctx.memory(|memory| memory.area_rect(egui::Id::new("inventory")))
        {
            let left = view == "resize-left";
            let start = if left { rect.left_center() } else { rect.right_center() };
            let target = egui::pos2(if left { rect.right() + 120.0 } else { rect.left() - 120.0 }, start.y);
            match self.frame_count {
                300 => raw.events.push(egui::Event::PointerMoved(start)),
                301 | 400 => raw.events.push(egui::Event::PointerButton {
                    pos: if self.frame_count == 301 { start } else { target },
                    button: egui::PointerButton::Primary,
                    pressed: self.frame_count == 301,
                    modifiers: egui::Modifiers::NONE,
                }),
                302 | 360 => raw.events.push(egui::Event::PointerMoved(target)),
                _ => {}
            }
        }
        // AURORA_DEMO_POINTER="x,y[,l|r|s][;x,y…]": the pointer at these window
        // pixels from frame 300, one point every 60 frames (hover states,
        // sub-menus); ",l"/",r" left-/right-clicks the interface there; ",s"
        // leaves the pointer at rest there instead of moving it every frame
        // (egui only shows a tooltip under a pointer at rest)
        if self.demo
            && self.frame_count >= 300
            && let Ok(v) = std::env::var("AURORA_DEMO_POINTER")
        {
            let points: Vec<(f32, f32, Option<egui::PointerButton>, bool)> = v
                .split(';')
                .filter_map(|pt| {
                    let mut it = pt.split(',').map(str::trim);
                    let x = it.next()?.parse().ok()?;
                    let y = it.next()?.parse().ok()?;
                    let (button, still) = match it.next() {
                        Some("l") => (Some(egui::PointerButton::Primary), false),
                        Some("r") => (Some(egui::PointerButton::Secondary), false),
                        Some("s") => (None, true),
                        _ => (None, false),
                    };
                    Some((x, y, button, still))
                })
                .collect();
            let step = (self.frame_count - 300) / 60;
            let i = (step as usize).min(points.len().saturating_sub(1));
            if let Some(&(x, y, button, still)) = points.get(i) {
                let ppp = self.egui_ctx.pixels_per_point().max(0.1);
                let pos = egui::pos2(x / ppp, y / ppp);
                let click_frame = (self.frame_count - 300) % 60;
                // at rest: only brought (back) there when it is elsewhere
                if !still || self.egui_ctx.input(|i| i.pointer.latest_pos()) != Some(pos) {
                    raw.events.push(egui::Event::PointerMoved(pos));
                }
                if let Some(button) = button
                    && step as usize == i
                    && matches!(click_frame, 2 | 3)
                {
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: click_frame == 2,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
        let ctx = self.egui_ctx.clone();
        if self.demo {
            self.gesture_demo_input(&mut raw);
            self.double_click_menu_demo_input(&mut raw);
            raw.events.extend(ui::widgets::demo::events(&ctx, self.frame_count));
            raw.events
                .extend(ui::toolbar::demo_events(&ctx, self.frame_count, &mut self.toolbar_ui));
            raw.events.extend(ui::bars::favorites::demo_events(&ctx, self.frame_count));
            raw.events.extend(crate::demo::chatlog::input_events(self.frame_count));
            raw.events.extend(ui::widgets::demo::snap_events(&ctx, self.frame_count));
            raw.events.extend(ui::widgets::demo::resize_events(&ctx, self.frame_count));
            raw.events.extend(ui::widgets::demo::minimized_events(&ctx, self.frame_count));
            raw.events.extend(ui::color_picker::demo::events(
                &self.egui_ctx,
                &self.build.ui.color_picker,
                self.frame_count,
            ));
            for d in self.task_notecards.values_mut() {
                raw.events
                    .extend(ui::inventory::document_demo_events(&ctx, self.frame_count, &mut d.state));
            }
            raw.events
                .extend(ui::inventory::document_demo_events(&ctx, self.frame_count, &mut self.inventory_ui));
            raw.events
                .extend(ui::inventory::clipboard_demo_events(&ctx, self.frame_count, &mut self.inventory_ui));
            raw.events
                .extend(ui::inventory::delete_demo_events(&ctx, self.frame_count, &self.inventory_ui));
        }
        let projector = ui::hud::Projector {
            view_proj: vp,
            width: w as f32,
            height: h as f32,
            ppp: ctx.pixels_per_point(),
        };
        if self.demo && ui::widgets::demo::rez_mode().is_some() {
            let mode = ui::widgets::demo::rez_mode().unwrap_or("1");
            let point = if matches!(mode, "object" | "contents") {
                self.world
                    .objects
                    .index_of_uuid(&crate::demo::action_id(970))
                    .and_then(|idx| Scene::object_transform(&self.world, idx, Instant::now(), 0))
                    .map(|(point, _, _)| point)
            } else {
                let p = self.world.agent.position + Vec3::new(5.0, 2.0, 0.0);
                self.world.ground_height(p).map(|z| Vec3::new(p.x, p.y, z))
            };
            if let Some(pos) = point.and_then(|point| self.build.cam.project(point)) {
                if mode == "contents" && (680..=686).contains(&self.frame_count) {
                    raw.events.push(egui::Event::ModifiersChanged(egui::Modifiers {
                        ctrl: true,
                        ..Default::default()
                    }));
                }
                raw.events.extend(ui::widgets::demo::rez_events(&ctx, self.frame_count, pos));
            }
        }
        if self.build.tool == crate::build::Tool::Edit
            && self.build.hud_selected(&self.world)
            && self.settings.show_huds
            && let Some(view) = self.scene.lists.hud_view
        {
            self.build.cam = crate::build::geom::Cam::for_hud(view, ctx.pixels_per_point());
        }
        if self.in_world() && self.build.open {
            for idx in self.build.sel_prims(&self.world) {
                if let Some(id) = self.world.objects.get(idx).map(|o| o.full_id)
                    && !self.build.face_counts.contains_key(&id)
                    && let Some(n) = self.media.face_count(&self.world, &self.scene, idx)
                {
                    self.build.face_counts.insert(id, n);
                }
            }
            if self.build.face_counts.len() > 1024 {
                self.build.face_counts.clear();
            }
        }
        let mut actions = UiActions::default();
        let vsync = gfx.renderer.vsync();
        self.frame_profile.lap(Lap::UiPrep);
        // Serialize the report capture with other users of renderer.captured.
        // Start before drawing so the report window is absent from the picture.
        if self.bug_report.capture_requested.is_some()
            && !self.inventory_images.auto_capture_pending
            && self.inventory_images.capturing.is_none()
            && !self.scene_capture_pending
            && !gfx.renderer.capture_request
            && !gfx.renderer.capture_scene
            && let Some(with_ui) = self.bug_report.capture_requested.take()
        {
            self.bug_report.capture_pending = true;
            if with_ui {
                gfx.renderer.capture_request = true;
            } else {
                gfx.renderer.capture_scene = true;
            }
        }
        self.report_abuse_capture(&mut gfx.renderer);
        self.sound_cues.before(&ctx, &raw);
        self.numeric_drag.before(&mut raw, ctx.pixels_per_point());
        let mut inventory_drop = None;
        let t_pass = Instant::now();
        let mut windows_ms = 0.0;
        let mut full = ctx.run_ui(raw, |ui| {
            self.sound_cues.pass_start(ui.ctx());
            let t = Instant::now();
            actions = self.draw_ui(ui, &projector, &gfx.renderer);
            if inventory_drop.is_none() {
                inventory_drop = self.inventory_drag_input(ui.ctx(), &gfx.renderer);
            }
            // the glass blurs the world: there is none behind the login screen
            if self.in_world() {
                ui::menu::glaze(ui.ctx(), &self.palette);
            }
            windows_ms += t.elapsed().as_secs_f32() * 1000.0;
        });
        // interface sounds of the widgets; the sounds are SL assets, loaded
        // once logged in
        let in_world_view = matches!(self.screen, Screen::World);
        let cues = self.sound_cues.after(&ctx, &full.platform_output.events, in_world_view);
        if let Some(anchor) = self.numeric_drag.after(
            &ctx,
            &full.platform_output.events,
            self.focused && !self.cursor_hold.held() && !self.exit_prompt.open,
        ) {
            let position = winit::dpi::PhysicalPosition::new(anchor.x * ctx.pixels_per_point(), anchor.y * ctx.pixels_per_point());
            if (self.cursor_pos.0 - position.x).abs() > 0.5 || (self.cursor_pos.1 - position.y).abs() > 0.5 {
                if gfx.window.set_cursor_position(position).is_ok() {
                    self.cursor_pos = (position.x, position.y);
                } else {
                    self.numeric_drag.cancel();
                }
            }
        }
        let previews = ui::sound_cues::take_previews(&ctx);
        if self.in_world() {
            self.world.ui_sounds.extend(cues);
            // preview buttons of the sound preferences (force_sound)
            for id in previews {
                self.scene.sounds.play_ui(self.audio_engine.as_ref(), id);
            }
        }
        self.ui_parts
            .add(Part::UiPass, t_pass.elapsed().as_secs_f32() * 1000.0 - windows_ms);
        self.frame_profile.lap(Lap::Ui);
        // Native bitmap cursors, including the original Firestorm hotspot.
        // Set this every frame because egui's custom cursor output is sticky.
        full.platform_output.cursor_image = None;
        if let Some(drop) = inventory_drop {
            self.apply_inventory_drop(drop);
        }
        self.update_object_hold(&gfx.renderer);
        if let Some(url) = self.interactions.media_url.take() {
            if self.demo {
                log::info!("demo open media: external browser request simulated");
                self.world.system_message(aurora_i18n::tr!("app-demo-opening-media-url-simulated"));
            } else {
                full.platform_output
                    .commands
                    .push(egui::OutputCommand::OpenUrl(egui::OpenUrl::new_tab(url)));
            }
        }
        self.look_hover = None;
        if matches!(self.screen, Screen::World)
            && !self.build.open
            && !self.alt
            && !self.ctrl
            && !self.shift
            && self.mouse_mode == MouseMode::None
            && !self.camera.mouselook()
            && self.media_cursor.is_none()
            && !ctx.is_pointer_over_egui()
            && !ctx.egui_wants_pointer_input()
        {
            let hovered = self.scene.hover_hud(&self.world, self.cursor_pos).or_else(|| {
                // Preserve the world hover cache (scene/hover.rs).
                let (point, idx) = self.scene.hover_pick(
                    &self.world,
                    gfx.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1),
                    gfx.renderer.hover_pick(self.cursor_pos.0, self.cursor_pos.1),
                    self.settings.draw_distance,
                );
                // the same pick gives the look-at its point
                self.look_hover = point.map(|p| (idx, p));
                idx
            });
            if self.demo && self.frame_count == 650 && std::env::var_os("AURORA_DEMO_ACTIONS").is_some() {
                log::info!(
                    "demo cursor hover: prim={:?}, action={:?}",
                    hovered.and_then(|idx| self.world.objects.get(idx)).map(|o| o.key.local_id),
                    hovered.and_then(|idx| crate::interaction::cursor_action(&self.world, idx, &self.interactions.props)),
                );
            }
            if let Some(idx) = hovered
                && let Some(target) = crate::interaction::target(&self.world, idx, &std::collections::HashMap::new())
            {
                if let Some(cmd) = self.interactions.hover_request(target) {
                    self.send(cmd);
                }
                if let Some(action) = crate::interaction::cursor_action(&self.world, idx, &self.interactions.props) {
                    if self.demo && self.frame_count == 722 && std::env::var_os("AURORA_DEMO_ACTIONS").is_some() {
                        log::info!("demo action hover: {action:?}, prim={}", target.clicked.local_id);
                    }
                    if action == crate::interaction::Action::Touch {
                        full.platform_output.cursor_icon = egui::CursorIcon::PointingHand;
                    }
                    full.platform_output.cursor_image = self.cursors.image(action, self.media.parcel_status_playing());
                }
            }
        }
        if self.plate_cursor() {
            full.platform_output.cursor_icon = egui::CursorIcon::PointingHand;
            full.platform_output.cursor_image = self.cursors.image(crate::interaction::Action::Touch, false);
        }
        if !ctx.is_pointer_over_egui() && !ctx.egui_wants_pointer_input() && self.beacon_hover() {
            full.platform_output.cursor_icon = egui::CursorIcon::PointingHand;
            full.platform_output.cursor_image = self.cursors.image(crate::interaction::Action::Touch, false);
        }
        if let Some(h) = &self.interactions.held {
            if h.physical {
                full.platform_output.cursor_image = self.cursors.image(crate::interaction::Action::Grab, false);
            } else {
                full.platform_output.cursor_icon = egui::CursorIcon::PointingHand;
                full.platform_output.cursor_image = self.cursors.image(crate::interaction::Action::Touch, false);
            }
        }
        if let Some(mode) = self.camera_tool_hover(&ctx) {
            full.platform_output.cursor_icon = match mode {
                crate::camera::ToolMode::Zoom => egui::CursorIcon::ZoomIn,
                crate::camera::ToolMode::Orbit => egui::CursorIcon::AllScroll,
                crate::camera::ToolMode::Pan => egui::CursorIcon::Move,
            };
            full.platform_output.cursor_image = self.cursors.camera(mode);
        }
        if self.hud_drag.is_some()
            || (self.hud_drag_requested()
                && self.settings.show_huds
                && matches!(self.screen, Screen::World)
                && self.mouse_mode == MouseMode::None
                && !self.camera.mouselook()
                && !ctx.is_pointer_over_egui()
                && !ctx.egui_wants_pointer_input()
                && self
                    .scene
                    .hud_move_pick(&self.world, self.cursor_pos)
                    .is_some_and(|idx| crate::build::hud_drag::HudDrag::can_start(&self.world, idx)))
        {
            full.platform_output.cursor_icon = egui::CursorIcon::Grabbing;
            full.platform_output.cursor_image = self.cursors.hud_drag();
        }
        if let Some(cursor) = self.inventory_drag.cursor {
            full.platform_output.cursor_icon = cursor;
            full.platform_output.cursor_image = self.cursors.inventory_drag(cursor);
        }
        if (self.color_pipette() || self.report_abuse.picking) && !ctx.is_pointer_over_egui() {
            full.platform_output.cursor_icon = egui::CursorIcon::Crosshair;
            full.platform_output.cursor_image = self.cursors.pipette();
        }
        if self.cursor_hold.held() {
            // the camera holds the hidden cursor: a cursor change from egui
            // would show it again in the middle of the drag
            full.platform_output.cursor_icon = egui::CursorIcon::None;
            full.platform_output.cursor_image = None;
        }
        if matches!(self.screen, Screen::Login)
            && (self.login_form.motion.is_dragging() || self.login_form.manager.is_dragging())
            && full.platform_output.cursor_icon == egui::CursorIcon::Grabbing
        {
            // Windows substitutes arrows for the system Grabbing cursor.
            full.platform_output.cursor_image = self.cursors.hud_drag();
        }
        if self.demo
            && self.frame_count == 650
            && (std::env::var_os("AURORA_DEMO_ACTIONS").is_some() || std::env::var_os("AURORA_DEMO_PLATE_HOVER").is_some())
        {
            log::info!(
                "demo cursor output: icon={:?}, bitmap={}, plate={:?}",
                full.platform_output.cursor_icon,
                full.platform_output.cursor_image.is_some(),
                self.plate_hover,
            );
        }
        gfx.egui_state
            .handle_platform_output_with_event_loop(&gfx.window, event_loop, full.platform_output);
        self.perf.ui_ms = t_ui.elapsed().as_secs_f32() * 1000.0;
        self.frame_profile.lap(Lap::Hover);
        let egui_frame = self.egui_hand_over.frame(
            &ctx,
            std::mem::take(&mut full.shapes),
            std::mem::take(&mut full.textures_delta),
            full.pixels_per_point,
        );

        // apply UI actions
        if actions.font_changed {
            self.system_fonts_pending = !ui::fonts::apply(&self.egui_ctx, &self.settings.font);
        }
        if actions.settings_changed || std::mem::take(&mut self.snapshots.settings_changed) {
            aurora_i18n::select(&self.settings.language, crate::cli::get().language.as_deref());
            crate::crash::update(&self.skin.theme, &self.settings);
            self.net
                .set_agent_language(aurora_i18n::language(), self.settings.language_is_public);
            if self.settings.vsync != vsync {
                gfx.renderer.set_vsync(self.settings.vsync);
            }
            self.skin.theme.apply(&self.egui_ctx, self.settings.font_scale);
            ui::colors::set(&self.settings.colors);
            gfx.renderer.apply_settings(self.settings.render_settings());
            self.panels.time_of_day = self.settings.time_of_day;
            self.egui_ctx.set_zoom_factor(self.settings.ui_scale);
            self.net.send(NetCommand::SetDrawDistance(self.settings.draw_distance));
            self.disk_cache.set_limit_mb(self.settings.cache_size_mb);
            self.net.set_bandwidth(bandwidth_kbps(&self.settings));
        }
        self.handle_actions(actions, event_loop);
        for r in std::mem::take(&mut self.build.ui.requests) {
            self.on_build_request(r);
        }
        self.update_avatar_pics(&mut gfx.renderer);
        self.update_maps();
        self.poll_key_to_name();

        if let Some(request) = self.inventory_images.capture.take() {
            if self.scene_capture_pending
                || self.inventory_images.capturing.is_some()
                || self.inventory_images.auto_capture_pending
                || self.bug_report.capture_pending
                || self.report_abuse.capture_pending
            {
                self.inventory_images.capture = Some(request);
            } else {
                gfx.renderer.capture_scene = true;
                self.inventory_images.capturing = Some(request);
            }
        }
        if self.want_scene_capture
            && !self.scene_capture_pending
            && self.inventory_images.capturing.is_none()
            && !self.inventory_images.auto_capture_pending
            && !self.bug_report.capture_pending
            && !self.report_abuse.capture_pending
        {
            self.want_scene_capture = false;
            self.scene_capture_place = None;
            if !self.demo && matches!(self.screen, Screen::World) {
                let key = (self.settings.login_uri(), self.settings.username.trim().to_lowercase());
                self.scene_capture_place = self
                    .login_form
                    .accounts
                    .iter()
                    .find(|account| account.key() == key && account.agent == Some(self.world.agent_id))
                    .cloned()
                    .map(|account| (account, self.world.region_name()));
            }
            gfx.renderer.capture_scene = true;
            self.scene_capture_pending = true;
        }
        self.frame_profile.lap(Lap::Actions);
        // ---- render: the frame is closed and leaves as a packet for the
        // render thread (aurora-render, main_thread.rs), which draws it
        // while this thread goes on with the next frame. The statistics
        // returned are those of the frame before; a frame that carries a
        // capture is waited for, its pixels are read in the tail below.
        let lists = if self.in_world() { &self.scene.lists } else { &self.empty_lists };
        let stats = gfx.renderer.render(fp, lists, Some(egui_frame));
        self.photo_drawn(&mut gfx.renderer);
        self.last_render = stats;
        if stats.presented && !gfx.shown {
            gfx.shown = true;
            // On Windows, maximizing can show even a hidden window. Defer
            // it until its client area already contains a rendered image.
            if gfx.maximize_on_show {
                gfx.window.set_maximized(true);
            }
            gfx.window.set_visible(true);
            log::info!("startup window shown after the first presented frame");
        }
        if stats.presented && !self.update_acknowledged {
            self.update_acknowledged = true;
            aurora_updater::install::acknowledge_frame();
        }
        if gfx.renderer.failed() {
            crate::crash::failure("The render thread terminated unexpectedly.");
            // the render thread is gone (its panic is in the log): nothing
            // more can be shown, leave cleanly
            self.gfx = Some(gfx);
            self.shutdown(event_loop);
            return;
        }
        self.frame_profile.lap(Lap::Render);
        // frame limiter; vsync already paces at the rate of the screen the
        // window is on now (it may have moved, the screen may have changed
        // its mode)
        self.screen_rate.poll(&gfx.window, Instant::now());
        let monitor_hz = self.screen_rate.hz();
        let timed = self.capture.is_some() || self.frame_profile.enabled();
        let cap = frame_cap(
            &self.settings,
            self.focused,
            self.in_world(),
            monitor_hz,
            timed,
            crate::cli::get().fps_limit,
        );
        let paced_by_vsync = gfx.renderer.vsync() && cap >= monitor_hz - 0.5;
        let target = (cap >= 1.0 && !paced_by_vsync).then(|| Duration::from_secs_f32(1.0 / cap));
        // (while Windows holds the window, this thread is left to its loop:
        // the wait is done between the frames, on another thread)
        if !crate::window_hold::paces(target)
            && let Some(target) = target
        {
            let spent = self.frame_start.elapsed();
            if spent < target {
                let rest = target - spent;
                if rest > Duration::from_millis(2) {
                    std::thread::sleep(rest - Duration::from_millis(1));
                }
                while self.frame_start.elapsed() < target {
                    std::hint::spin_loop();
                }
            }
        }
        self.frame_profile.lap(Lap::Limiter);
        self.frame_start = Instant::now();
        self.frame_count += 1;
        if self.demo
            && self.frame_count == 1500
            && std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| matches!(v.as_str(), "image-photo" | "image-photo-save"))
        {
            let request = uuid::Uuid::new_v4();
            let item = uuid::Uuid::from_u128(8101);
            self.inventory_ui.thumbnail.pending = Some(request);
            self.inventory_image_action(request, item, ui::inventory::thumbnail::Input::Capture);
        }
        if self.demo && self.frame_count == 2100 && std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| v == "image-photo-save") {
            let mut actions = Vec::new();
            self.inventory_ui.thumbnail.save_photo(&mut actions);
            for action in actions {
                self.apply_inventory_action(action);
            }
        }
        if self.demo {
            let time = self.egui_ctx.input(|i| i.time);
            for event in crate::demo::people::state_events(time, &mut self.demo_people_states) {
                self.world.apply(event);
            }
            for event in crate::demo::tags::state_events(time, &mut self.demo_tag_states) {
                self.world.apply(event);
            }
        }
        if self.demo && std::env::var_os("AURORA_DEMO_ANIM_LOOP").is_some() {
            for event in crate::demo::loop_animation_events(self.frame_count) {
                self.world.apply(event);
            }
        }
        if self.demo
            && let Some(mode) = crate::demo::deform::mode()
        {
            for event in crate::demo::deform::events(self.frame_count, mode) {
                self.world.apply(event);
            }
            if let Some((avatar, animations)) = crate::demo::deform::reset_at(self.frame_count, mode) {
                self.on_ctx_action(ui::context::CtxAction::ResetSkeleton { avatar, animations });
            }
        }
        if self.demo
            && let Some(mode) = crate::demo::notifs::mode()
        {
            let agent = self.world.agent_id;
            if self.frame_count == crate::demo::notifs::DND_AT && mode == "dnd" {
                self.world.set_dnd(true);
            }
            for event in crate::demo::notifs::events(self.frame_count, agent) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
            if self.frame_count == crate::demo::notifs::ANSWER_AT && matches!(mode.as_str(), "history" | "legacy-history") {
                // a few cards answered or closed: they go to the history
                let ids: Vec<(u64, bool)> = self.world.notifications.listed().map(|n| (n.id, n.interactive())).collect();
                for (id, interactive) in ids.into_iter().take(4) {
                    // the teleport offer refused (its place stays), the rest kept or read
                    let lure = self
                        .world
                        .notifications
                        .list
                        .iter()
                        .any(|n| n.id == id && matches!(n.data, crate::world::notifications::Data::Lure { .. }));
                    let r = if lure {
                        crate::world::notifications::Response::Decline
                    } else if interactive {
                        crate::world::notifications::Response::Accept
                    } else {
                        crate::world::notifications::Response::Dismiss
                    };
                    let (cmds, _) = self.world.respond_notification(id, r);
                    for c in cmds.into_iter().filter(|c| !matches!(c, NetCommand::AcceptLure { .. })) {
                        self.send(c);
                    }
                }
            }
            if self.frame_count == crate::demo::notifs::LIST_AT && matches!(mode.as_str(), "list" | "history" | "dnd" | "legacy-history") {
                self.notif_ui.open = true;
                if mode.ends_with("history") {
                    self.notif_ui.center.tab = ui::notification_center::Tab::History;
                }
            }
        }
        if self.demo
            && let Some(mode) = crate::demo::toasts::mode()
        {
            let agent = self.world.agent_id;
            for event in crate::demo::toasts::events(self.frame_count, agent) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
            if self.frame_count == crate::demo::toasts::LIST_AT && mode == "1" {
                self.notif_ui.open = true;
            }
            if self.frame_count == crate::demo::toasts::CLICK_AT && (mode == "click" || mode == "minimized") {
                // the click on the resident's toast, as the card answers it
                let nova = self.world.notifications.list.iter().find_map(|n| match n.data {
                    crate::world::notifications::Data::Message { session, .. } if session == crate::demo::DEMO_NOVA => Some(n.id),
                    _ => None,
                });
                if let Some(id) = nova
                    && self.world.open_from_notification(id).is_some()
                {
                    ui::notifications::open_conversations(&self.egui_ctx, &mut self.panels.chat);
                }
            }
        }
        if self.demo {
            for event in crate::demo::eep::events(self.frame_count) {
                self.world.apply(event);
            }
            for event in crate::demo::sky::events(self.frame_count) {
                self.world.apply(event);
            }
            if let Some(step) = crate::demo::env::step(self.frame_count) {
                self.demo_env_step(step);
            }
            if let Some(scenario) = crate::demo::env::editor() {
                if scenario.starts_with("day") {
                    self.demo_day_editor(scenario, self.frame_count);
                } else if crate::demo::env::parcel_scenario() {
                    self.demo_env_apply(scenario, self.frame_count);
                } else if self.frame_count == 400 {
                    self.demo_env_editor(scenario);
                } else if scenario == "item-save" {
                    self.demo_env_editor_step(self.frame_count);
                }
            }
        }
        // last view for the loading screens (blurred in a thread, or now when quitting)
        if self.bug_report.capture_pending
            && let Some((w, h, pixels)) = gfx.renderer.captured.take()
        {
            self.bug_report.captured(w, h, pixels);
        }
        self.report_abuse_captured(&mut gfx.renderer);
        if self.inventory_images.capturing.is_some()
            && let Some((w, h, pixels)) = gfx.renderer.captured.take()
            && let Some(request) = self.inventory_images.capturing.take()
        {
            self.prepare_inventory_photo(request, w, h, pixels);
        }
        if self.scene_capture_pending
            && let Some((w, h, px)) = gfx.renderer.captured.take()
        {
            self.scene_capture_pending = false;
            let wait = self.quit_at.is_some() || self.exit_prompt.open;
            let pixels = std::sync::Arc::new(px);
            if let Some((account, region)) = self.scene_capture_place.take() {
                self.login_form.places.capture(account, region, w, h, pixels.clone(), wait);
            }
            if self.settings.loading_backdrop {
                self.backdrop.set_capture(w, h, pixels, wait);
            }
        }
        if let Some((files, exit)) = &mut self.capture {
            if !self.inventory_images.auto_capture_pending
                && files.iter().any(|(at, _)| *at <= self.frame_count)
                && self.inventory_images.capturing.is_none()
                && !self.scene_capture_pending
            {
                gfx.renderer.capture_request = true;
                self.inventory_images.auto_capture_pending = true;
            }
            // the picture of the earliest pending capture
            if let Some((w, h, px)) = gfx.renderer.captured.take()
                && !files.is_empty()
            {
                self.inventory_images.auto_capture_pending = false;
                let (at, file) = files.remove(0);
                match image::save_buffer(&file, &px, w, h, image::ExtendedColorType::Rgba8) {
                    Ok(()) => log::info!("frame {at} captured to {}", file.display()),
                    Err(e) => log::warn!("capture failed: {e}"),
                }
                if *exit && files.is_empty() {
                    self.quit_at = Some(Instant::now());
                }
            }
        }
        self.perf
            .frame(dt * 1000.0, self.net.stats.snapshot(), &self.last_render, &self.scene.stats);
        self.frame_profile.lap(Lap::Tail);
        self.end_profile_frame(w, h);
        self.gfx = Some(gfx);
        // the clicks whose depth came back with this frame's hand-over
        self.resolve_clicks();
        self.demo_action_steps();
        self.demo_click_steps();
        if self.demo && self.frame_count == 700 && std::env::var("AURORA_DEMO_MAP").as_deref() == Ok("beacon-click") {
            if let Some(hit) = &self.beacon_hit {
                let at = hit.rect.center() * self.egui_ctx.pixels_per_point();
                self.cursor_pos = (at.x, at.y);
                self.mouse_input(MouseButton::Left, true, false);
                self.mouse_input(MouseButton::Left, false, false);
            }
            log::info!(
                "demo beacon click: cleared={}, mouse_mode={:?}, pending_pick={}",
                self.world.map.track.is_none(),
                self.mouse_mode,
                self.clicks.waiting(),
            );
        }
        if self.demo {
            self.demo_double_click_steps();
        }
        self.demo_hud_drag_steps();
        // AURORA_DEMO_INVENTORY=offer|offer-reject: the sample offer's item
        // shown in the folder it reaches; the second rejects it at frame 700
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_INVENTORY")
            && mode.starts_with("offer")
        {
            if self.frame_count == 700 && mode == "offer-reject" {
                let offer = self
                    .world
                    .notifications
                    .list
                    .iter()
                    .find(|n| n.kind == crate::world::notifications::Kind::Inventory);
                if let Some(id) = offer.map(|n| n.id) {
                    for cmd in self
                        .world
                        .respond_notification(id, crate::world::notifications::Response::Decline)
                        .0
                    {
                        self.send(cmd);
                    }
                }
            }
            let folder = self.world.inventory.items.get(&crate::demo::DEMO_OFFER).map(|it| it.parent);
            if folder.is_some() && folder != self.demo_offer_shown {
                self.demo_offer_shown = folder;
                self.inventory_ui.show_original(&self.world.inventory, crate::demo::DEMO_OFFER);
            }
        }
        self.demo_worn_steps();
        if self.demo && std::env::var("AURORA_DEMO_HUDS").is_ok_and(|m| m == "touch") {
            if self.frame_count == 720
                && let Some(view) = self.scene.lists.hud_view
            {
                self.cursor_pos = (view.width * 0.5 - 0.18 * view.height, view.height * 0.5 - 0.028 * view.height);
                self.on_left_press();
            } else if self.frame_count == 800 {
                self.cursor_pos.0 += 12.0;
            } else if self.frame_count == 900 {
                self.on_left_release();
            }
        }
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_HUDS")
            && matches!(mode.as_str(), "menu" | "edit" | "edit-zoom")
        {
            let key = crate::world::objects::ObjKey {
                region: self.world.main_region.unwrap_or_default(),
                local_id: crate::demo::hud::CHILD,
            };
            if self.frame_count == 720
                && let Some(view) = self.scene.lists.hud_view
            {
                self.cursor_pos = (
                    view.width * 0.5 - 0.18 * view.height * view.zoom,
                    view.height * 0.5 - 0.028 * view.height * view.zoom,
                );
                self.open_context_menu();
            } else if mode != "menu" {
                if self.frame_count == 760 {
                    self.on_ctx_action(ui::context::CtxAction::EditAttachment(key));
                    self.context_menu = None;
                } else if self.frame_count == 800
                    && let Some(b) = self.build.bounds(&self.world, Instant::now())
                {
                    let handle = b.center + Vec3::Y * self.build.cam.meters_for_pixels(b.center, 45.0);
                    if let Some(cursor) = self.build.cam.project_px(handle) {
                        self.cursor_pos = cursor;
                        self.click(click_pick::Click::Build);
                    }
                } else if (801..=840).contains(&self.frame_count) {
                    self.cursor_pos.0 += 2.0;
                } else if self.frame_count == 850 {
                    self.on_left_release();
                    if let Some(idx) = self.world.objects.index_of(&crate::world::objects::ObjKey {
                        region: key.region,
                        local_id: crate::demo::hud::FIRST,
                    }) && let Some(o) = self.world.objects.get(idx)
                    {
                        log::info!(
                            "demo HUD edit: local position={:?}, attachment={}",
                            o.position,
                            o.attachment_point()
                        );
                    }
                }
            }
        }
        // captures: AURORA_DEMO_RCLICK="x,y" simulates a right click (physical
        // px) at frame 225; "tag" on the name tag of the nearest other avatar
        // at frame 600, once the tags are shown
        if self.demo
            && (self.frame_count == 225 || self.frame_count == 600)
            && let Ok(v) = std::env::var("AURORA_DEMO_RCLICK")
        {
            let ppp = self.egui_ctx.pixels_per_point();
            let c: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            let at = if v.trim() == "tag" {
                let tag = self.name_tags.iter().rev().find(|t| t.id != self.world.agent_id);
                if tag.is_none() && self.frame_count == 600 {
                    log::warn!("demo right click: no other avatar's name tag on screen");
                }
                tag.filter(|_| self.frame_count == 600)
                    .map(|t| (t.rect.center().x * ppp, t.rect.center().y * ppp))
            } else {
                (self.frame_count == 225 && c.len() == 2).then(|| (c[0], c[1]))
            };
            if let Some(p) = at {
                self.cursor_pos = p;
                self.open_context_menu();
            }
        }
        // AURORA_DEMO_STATUSMENU=invisible: the status menu, invisible (shown
        // only: no right is changed, nothing is saved), once logged in
        if self.demo && self.frame_count == 60 && std::env::var("AURORA_DEMO_STATUSMENU").is_ok_and(|v| v == "invisible") {
            self.world.status.invisible.on = true;
        }
        // AURORA_DEMO_CONTACTS: contact sets and aliases to show, once logged in
        if self.demo && self.frame_count == 60 && std::env::var_os("AURORA_DEMO_CONTACTS").is_some() {
            crate::demo::seed_contact_sets(&mut self.world);
        }
        // AURORA_DEMO_PEOPLE's tabs: the mock-up's sets, pins and recent people
        if self.demo && self.frame_count == 60 {
            crate::demo::people::seed(&mut self.world);
        }
        // AURORA_DEMO_NAVEDIT=1: click in the location field at frame 230
        if self.demo && self.frame_count == 230 && std::env::var_os("AURORA_DEMO_NAVEDIT").is_some() {
            self.panels.nav_edit = Some(crate::slurl::make(&self.world.region_name(), self.world.agent.position));
            self.panels.nav_edit_new = true;
        }
        // AURORA_DEMO_SIT=n: the toolbar sit button clicked n times, at
        // frames 240, 300, 360…
        if self.demo
            && self.frame_count >= 240
            && self.frame_count.is_multiple_of(60)
            && let Some(n) = std::env::var("AURORA_DEMO_SIT").ok().and_then(|v| v.trim().parse::<u64>().ok())
            && (self.frame_count - 240) / 60 < n
        {
            self.toggle_ground_sit();
            log::info!("demo sit button: sitting {}", self.world.agent.is_sitting());
        }
        if let (Some(demo), Some(g)) = (&mut self.geometry_demo, &mut self.gfx) {
            demo.tick(self.frame_count, &mut g.renderer);
        }
        if let (Some(demo), Some(g)) = (&mut self.stream_demo, &mut self.gfx) {
            let pending = self.scene.stats.geom_pending;
            demo.tick(
                self.frame_count,
                &mut self.world,
                &mut self.scene.textures,
                &mut g.renderer,
                &self.scene.jobs,
                pending,
            );
        }
        // AURORA_DEMO_TEXREFRESH: a panel with a stale cached texture
        if let Some(demo) = &mut self.tex_refresh_demo
            && let Some(object) = demo.tick(self.frame_count, &mut self.world, self.scene.textures.cache_dir())
        {
            self.refresh_object_textures(object);
        }
        // AURORA_DEMO_EXPERIENCE: the door or the pad, and what asks
        // without a click (walking into the pad, arriving)
        if let Some(reply) = self
            .experience_demo
            .as_mut()
            .and_then(|demo| demo.tick(self.frame_count, &mut self.world))
        {
            self.apply_experience_demo(reply);
        }
        // AURORA_DEMO_TP_AWAY=1: out of reach of the demo region, then back
        if self.demo
            && let Some(events) = crate::demo::away_teleport(self.frame_count, self.world.agent.position, self.world.agent.forward())
        {
            for ev in events {
                self.world.apply(ev);
            }
            log::info!(
                "demo teleport away: frame {}, {} objects in the store",
                self.frame_count,
                self.world.objects.len()
            );
        }
        // AURORA_DEMO_TEXTURES_CHURN=1: stress cubes removed at frame 300
        // (textures released, evicted after 2 s), back with new textures at 700
        if self.demo
            && crate::demo::texture_churn()
            && let Some(n) = crate::demo::texture_stress_count()
        {
            if self.frame_count == 300 {
                self.scene.textures.evict_after = Duration::from_secs(2);
                for i in (0..n).filter(|&i| crate::demo::stress_churned(i)) {
                    self.scene.textures.release(&crate::demo::stress_texture(i));
                }
                for ev in crate::demo::texture_churn_kill(n) {
                    self.world.apply(ev);
                }
            }
            if self.frame_count == 700 {
                if let Some(g) = &mut self.gfx {
                    let ids = (0..n).filter(|&i| crate::demo::stress_churned(i)).map(|i| n + i);
                    self.scene.textures.install_demo_stress(&mut g.renderer, ids);
                }
                for ev in crate::demo::texture_churn_respawn(n) {
                    self.world.apply(ev);
                }
            }
        }
        // Wait until login's loading fade is over, so the demo exercises a
        // teleport from the visible world even on a very fast machine.
        if self.demo
            && self.frame_count >= 240
            && !self.demo_tp_triggered
            && matches!(self.screen, Screen::World)
            && self.tp_overlay.is_none()
        {
            self.demo_tp_triggered = true;
            if let Ok(v) = std::env::var("AURORA_DEMO_TP") {
                let c: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                let (x, y, z) = if c.len() == 3 { (c[0], c[1], c[2]) } else { (128.0, 128.0, 30.0) };
                let region = if v == "remote" {
                    aurora_i18n::tr!("app-aurora-elsewhere")
                } else {
                    aurora_i18n::tr!("common-aurora-demo")
                };
                self.teleport_to_location(region.into(), Vec3::new(x, y, z));
                log::info!(
                    "demo teleport: frame {}, showing progress {}, position {:.1?}",
                    self.frame_count,
                    self.world.tp_show_progress,
                    self.world.agent.position
                );
            }
        }
        if self.demo && self.frame_count == 240 {
            // AURORA_DEMO_CHATCMD="calc 2+2;rolld 2 20": lines typed in the
            // chat bar, one after the other (chat bar commands)
            if let Ok(lines) = std::env::var("AURORA_DEMO_CHATCMD") {
                for line in lines.split(';').map(str::trim).filter(|l| !l.is_empty()) {
                    let (channel, message) = ui::chat::parse_channel(line);
                    self.send_local_chat(ui::chat::OutgoingChat {
                        message,
                        channel,
                        chat_type: aurora_net::ChatType::Normal,
                    });
                }
            }
            // AURORA_DEMO_DISPLAYNAME="name": display name change (simulated
            // reply, never sent in demo mode), then the window, now locked
            if let Ok(new) = std::env::var("AURORA_DEMO_DISPLAYNAME") {
                let me = self.world.agent_id;
                let old = self
                    .world
                    .social
                    .avatar_names
                    .get(&me)
                    .map(|n| n.display_name.clone())
                    .unwrap_or_default();
                self.send(NetCommand::SetDisplayName { old, new });
                self.display_name_ui.open();
            }
            self.demo_buy_currency();
            self.demo_buy_land();
            self.demo_pay();
        }
        // AURORA_DEMO_LOOKAT=1: our eye tracking on (cursor on the left of
        // the window, moving), Violet Wolf looks at a point north of him;
        // =own: the crosshairs shown, ours too, the real cursor; =own,x,y:
        // the cursor at (x, y), fractions of the window, moving by a pixel
        // (the head follows a cursor that moves); =targets: own
        // with every other avatar looking somewhere new every 2 s
        if self.demo
            && let Ok(value) = std::env::var("AURORA_DEMO_LOOKAT")
        {
            let mut parts = value.split(',');
            let mode = parts.next().unwrap_or_default().trim();
            let pinned: Vec<f32> = parts.filter_map(|v| v.trim().parse().ok()).collect();
            self.demo_look_at = true;
            self.demo_look_at_targets = matches!(mode, "own" | "targets");
            if mode == "targets" && self.frame_count >= 200 && self.frame_count.is_multiple_of(120) {
                self.demo_look_at_everyone();
            }
            // the cursor is pinned for captures; "own" and "targets" leave
            // the real one alone
            let pin = match (mode, pinned.as_slice()) {
                ("own", [x, y]) => Some((*x, *y, (self.frame_count % 2) as f32)),
                ("own" | "targets", _) => None,
                _ => Some((0.15, 0.3, (self.frame_count % 2) as f32)),
            };
            if let (Some((x, y, wiggle)), Some(g)) = (pin, self.gfx.as_ref()) {
                let (w, h) = g.renderer.size();
                self.cursor_pos = (w as f32 * x + wiggle, h as f32 * y);
            }
            // ",side": from frame 300 the camera beside our head, aimed at a
            // point in front of the face (which we then look at: a focus),
            // to see where the line leaves the head
            if value.ends_with(",side")
                && self.frame_count == 300
                && let Some(eyes) = self.scene.eyes_world(&self.world, &self.world.agent_id, Instant::now())
            {
                let rot = self.world.agent.body_rotation();
                let ahead = eyes + rot * Vec3::X * 0.5;
                self.camera
                    .set_view(ahead - rot * Vec3::Y * 1.2, ahead, &mut self.world.agent, &self.settings.camera);
            }
            if self.frame_count == 230
                && let Some((mx, my)) = self.world.main_origin()
            {
                let p = self.world.agent.position + Vec3::new(4.0, 6.0, 1.0);
                let ev = NetEvent::LookAt {
                    effect: uuid::Uuid::from_u128(0x100c),
                    source: crate::demo::DEMO_LOUP,
                    target: uuid::Uuid::nil(),
                    offset: [mx as f64 + p.x as f64, my as f64 + p.y as f64, p.z as f64],
                    kind: crate::world::lookat::FOCUS,
                    duration: crate::world::lookat::MAX_TIMEOUT,
                };
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
            }
            if self.frame_count.is_multiple_of(60) {
                let t = self.world.look_at.own.target;
                log::info!(
                    "demo lookat: own kind {} offset {:.2?}, remote {}",
                    t.kind,
                    t.offset,
                    self.world.look_at.remote.len()
                );
            }
        }
        // AURORA_DEMO_KEY="down" (up, left, right): hold that arrow from frame
        // 235, log the agent and drawn body headings every 30 frames
        if self.demo
            && self.frame_count >= 235
            && let Ok(v) = std::env::var("AURORA_DEMO_KEY")
        {
            let key = match v.as_str() {
                "up" => KeyCode::ArrowUp,
                "left" => KeyCode::ArrowLeft,
                "right" => KeyCode::ArrowRight,
                _ => KeyCode::ArrowDown,
            };
            self.down.insert(Input::key(key));
            if self.frame_count.is_multiple_of(30) {
                let body = self.world.bodies.get(&self.world.agent_id).map(|b| {
                    let f = b.rotation * Vec3::X;
                    f.y.atan2(f.x).to_degrees()
                });
                log::info!(
                    "demo key: agent yaw {:.0}°, body {:.0?}°, turn {}, vel {:.1?}",
                    self.world.agent.yaw.to_degrees(),
                    body,
                    self.world.agent.body_turn,
                    self.world.agent.velocity
                );
            }
        }
        // AURORA_DEMO_BUILD="mode,x,y[,part,dx,dy]": build tools script (build/demo.rs)
        if self.demo
            && let Ok(v) = std::env::var("AURORA_DEMO_BUILD")
        {
            for step in crate::build::demo::steps(&v, self.frame_count, &self.build, &self.world, &self.settings.build) {
                match step {
                    crate::build::demo::Step::Open(t, m, land) => {
                        self.build.open_build(t);
                        self.build.edit_mode = m;
                        crate::build::demo::apply_tab(&mut self.build);
                        self.settings.build.land_action = land;
                    }
                    crate::build::demo::Step::Cursor(x, y) => self.cursor_pos = (x, y),
                    crate::build::demo::Step::CreateAt(point) => {
                        self.build.mouse_down(
                            &mut self.world,
                            &mut self.settings.build,
                            Some(point),
                            None,
                            None,
                            self.cursor_pos,
                            crate::build::Mods::default(),
                        );
                        self.flush_build();
                    }
                    crate::build::demo::Step::Delete => {
                        log::info!("demo build created selection: {:?}", self.build.selection);
                        self.build.delete(&mut self.world);
                        log::info!("demo build created delete: {}", self.build.status);
                        self.flush_build();
                    }
                    crate::build::demo::Step::Contents => {
                        self.build.tab = 4;
                        self.panels.inventory = true;
                        self.inventory_ui.expand_all = Some(true);
                    }
                    crate::build::demo::Step::TexturePicker(materials) => {
                        let current = self
                            .build
                            .sel_te(&self.world)
                            .first()
                            .and_then(|&(idx, face)| self.world.objects.get(idx)?.te.as_ref().map(|te| te.face(face as usize).texture))
                            .unwrap_or_default();
                        if materials {
                            self.build
                                .ui
                                .picker
                                .open_materials(aurora_i18n::tr!("build-ui-texture-choose-material"), uuid::Uuid::nil());
                            self.build.ui.pick_target = Some(crate::build::ui::PickTarget::PbrMaterial);
                            self.build.ui.tex_tab = 0;
                        } else {
                            self.build.ui.picker.open_with_defaults(
                                aurora_i18n::tr!("build-ui-texture-choose-texture"),
                                current,
                                Some(aurora_prim::te::DEFAULT_TEXTURE),
                                false,
                            );
                            self.build.ui.pick_target = Some(crate::build::ui::PickTarget::Diffuse);
                        }
                        self.imports.open = false;
                        crate::build::demo::texture_inventory(&mut self.world);
                    }
                    crate::build::demo::Step::TakeTask(copy) => {
                        if let Some(idx) = self.build.contents_target(&self.world)
                            && let Some(o) = self.world.objects.get(idx)
                            && let Some(inv) = self.build.contents.objects.get(&o.full_id)
                            && let Some(item) = inv
                                .items
                                .iter()
                                .find(|i| !i.is_folder && (i.owner_mask & aurora_net::build::perm::COPY != 0) == copy)
                        {
                            self.apply_inventory_action(ui::inventory::InvAction::TakeTask(Box::new(
                                crate::build::task_inventory::TransferRequest {
                                    object: o.full_id,
                                    item: item.clone(),
                                    folder: self.world.inventory.root,
                                    approved: None,
                                    ignore_warning: false,
                                },
                            )));
                        }
                    }
                    crate::build::demo::Step::Select(idx) => {
                        self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                        self.flush_build();
                    }
                    crate::build::demo::Step::Press => self.click(click_pick::Click::Build),
                    crate::build::demo::Step::Release => self.build_release(),
                    crate::build::demo::Step::Face(idx, face, shift) => {
                        self.build.mouse_down(
                            &mut self.world,
                            &mut self.settings.build,
                            None,
                            Some(idx),
                            Some(aurora_net::TouchSurface {
                                face: face as i32,
                                ..Default::default()
                            }),
                            self.cursor_pos,
                            crate::build::Mods {
                                shift,
                                ..Default::default()
                            },
                        );
                        log::info!("demo build faces: {:?}", self.build.sel_te(&self.world));
                        self.flush_build();
                    }
                    crate::build::demo::Step::Focus(follow) => {
                        self.settings.build.follow_edit_object = follow;
                        if let Some(idx) = self.build.sel_prims(&self.world).first().copied()
                            && let Some(object) = crate::camera::pick_focus_object(&self.world, idx, Instant::now())
                        {
                            let point = object.1.position;
                            let ray = (self.camera.position, (point - self.camera.position).normalize_or_zero());
                            self.camera
                                .alt_focus(point, Some(object), ray, &mut self.world.agent, &self.settings.camera);
                        }
                    }
                    crate::build::demo::Step::CloseView => {
                        if let Some(idx) = self.build.sel_prims(&self.world).first().copied()
                            && let Some((point, _, _)) = crate::scene::Scene::object_transform(&self.world, idx, Instant::now(), 0)
                        {
                            self.camera.set_view(
                                point + Vec3::new(-4.0, -6.0, 3.0),
                                point,
                                &mut self.world.agent,
                                &self.settings.camera,
                            );
                            self.panels.inventory = false;
                        }
                    }
                    crate::build::demo::Step::Linked(on) => {
                        self.build.set_edit_linked(&mut self.world, &mut self.settings.build, on);
                        self.flush_build();
                    }
                    crate::build::demo::Step::ReplaceBegin(contents) => {
                        self.build.begin_replacement(&self.world, &self.settings.build);
                        self.build.replacement.options.contents = contents;
                        self.build.replacement.options.size = contents;
                    }
                    crate::build::demo::Step::ReplacePick(idx) => {
                        self.build.pick_replacement(&self.world, Some(idx));
                        self.flush_build();
                    }
                    crate::build::demo::Step::ReplaceApply => {
                        self.build.apply_replacement(&self.world);
                        self.flush_build();
                    }
                    crate::build::demo::Step::Wheel(delta) => {
                        let ctrl = self.ctrl;
                        self.ctrl = true;
                        let before = (self.camera.position, self.world.hud_zoom);
                        let handled = self.on_build_scroll(delta);
                        self.ctrl = ctrl;
                        log::info!(
                            "demo build cycle: delta {delta}, handled {handled}, selected {:?}, camera unchanged {}",
                            self.build.selection.iter().map(|k| k.local_id).collect::<Vec<_>>(),
                            before == (self.camera.position, self.world.hud_zoom)
                        );
                    }
                }
            }
        }
        // AURORA_DEMO_MEDIA_CLICK="x,y[,x2,y2]": clicks on a media face at
        // frames 700 (focus) and 760, a click at (x2, y2) at 820, then types
        // "aurora". The optional start frame allows slow plugins to load.
        if self.demo
            && let Ok(v) = std::env::var("AURORA_DEMO_MEDIA_CLICK")
        {
            let c: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            let start = std::env::var("AURORA_DEMO_MEDIA_CLICK_FRAME")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(700);
            let f = self.frame_count.checked_sub(start).map_or(0, |delta| delta.saturating_add(700));
            let at = match f {
                700 | 760 if c.len() >= 2 => Some((c[0], c[1])),
                820 if c.len() >= 4 => Some((c[2], c[3])),
                _ => None,
            };
            if let Some(pos) = at {
                self.cursor_pos = pos;
                self.on_left_press();
                self.on_left_release();
                self.demo_note(click_pick::DemoNote::Media(pos, self.frame_count));
            }
            if (850..862).contains(&f) {
                let k = (f - 850) as usize;
                let (code, ch) = [
                    (KeyCode::KeyA, "a"),
                    (KeyCode::KeyU, "u"),
                    (KeyCode::KeyR, "r"),
                    (KeyCode::KeyO, "o"),
                    (KeyCode::KeyR, "r"),
                    (KeyCode::KeyA, "a"),
                ][k / 2];
                let mods = aurora_media::Modifiers::default();
                self.media
                    .on_key(code, 0x1E, k.is_multiple_of(2), false, k.is_multiple_of(2).then_some(ch), mods);
            }
        }
        // AURORA_DEMO_CAMERA="alt,x,y" (pan, zoom, ml, wheel, fly): camera/demo.rs
        if self.demo
            && let Ok(v) = std::env::var("AURORA_DEMO_CAMERA")
        {
            self.demo_camera_steps(&v);
        }
        // AURORA_DEMO_MMO="x,y": left press on the avatar once the world shows
        // (frame 225 at the earliest), right button held 10 to 105 frames
        // later (MMO walking test), positions logged
        if self.demo
            && let Ok(v) = std::env::var("AURORA_DEMO_MMO")
        {
            if self.demo_mmo_start.is_none() && self.frame_count >= 225 && matches!(self.screen, Screen::World) && self.tp_overlay.is_none()
            {
                self.demo_mmo_start = Some(self.frame_count);
            }
            let c: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            let f = self.demo_mmo_start.map_or(u64::MAX, |s| self.frame_count - s);
            match (f, c.len()) {
                (0, n) if n >= 2 => {
                    self.cursor_pos = (c[0], c[1]);
                    self.on_left_press();
                    self.demo_note(click_pick::DemoNote::Mmo);
                }
                // third value 1: hold the right arrow instead of the right button
                (10, _) if c.get(2) == Some(&1.0) => {
                    self.down.insert(Input::key(KeyCode::ArrowRight));
                }
                // third value 2: double right click, held the second time (run)
                (11, _) if c.get(2) == Some(&2.0) => self.on_right_button(false, false),
                (12, _) if c.get(2) == Some(&2.0) => self.on_right_button(true, false),
                (10, _) => self.on_right_button(true, false),
                (105, _) => {
                    let running = self.temp_run.is_some();
                    if self.right_drag {
                        self.on_right_button(false, false);
                    }
                    self.down.remove(&Input::key(KeyCode::ArrowRight));
                    log::info!(
                        "demo mmo: after walking, mode {:?}, pos {:?}, yaw {:.2}, camera yaw {:.2}, running {running} (after release {})",
                        self.mouse_mode,
                        self.world.agent.position,
                        self.world.agent.yaw,
                        self.world.agent.yaw + self.camera.orbit_yaw,
                        self.temp_run.is_some()
                    );
                }
                _ => {}
            }
        }
    }

    /// AURORA_PROFILE: the frame's counters and the settings that change its
    /// cost, for the summary line.
    fn end_profile_frame(&mut self, width: u32, height: u32) {
        let s = &self.settings;
        let present = present_mode_name(self.last_render.present_mode);
        let screen_hz = self.screen_rate.hz();
        self.frame_profile.settings(|| {
            format!(
                "{width}x{height} vsync={} present={present} screen_hz={screen_hz:.0} fps_cap={} draw_distance={:.0} lod_factor={:.2} \
                 max_avatars={} shadows={} shadow_quality={} probes={} mirrors={} {:?}",
                s.vsync,
                if s.fps_cap { s.fps_limit } else { 0 },
                s.draw_distance,
                s.lod_factor,
                s.max_avatars,
                s.shadows,
                s.shadow_quality,
                s.reflection_probes,
                s.mirrors,
                s.render_settings(),
            )
        });
        let in_world = self.in_world();
        let lists = &self.scene.lists;
        let st = &self.scene.stats;
        let counts = SceneCounts {
            objects: st.objects,
            visible: st.visible_objects,
            synced: if in_world { st.synced } else { 0 },
            rebuilt: if in_world { st.rebuilt } else { 0 },
            posed: if in_world { st.posed } else { 0 },
            blend: lists.blend.len(),
            blend_glow: lists.blend_glow.iter().filter(|g| **g).count(),
            glow_alpha: lists.glow_alpha.len(),
            jobs: st.jobs,
            geom_pending: st.geom_pending,
            sync_backlog: if in_world { st.sync_backlog } else { 0 },
            anchored: if in_world { st.anchored } else { 0 },
        };
        let mut parts = std::mem::take(&mut self.scene.parts);
        parts.merge(&std::mem::take(&mut self.ui_parts));
        let logged = self.frame_profile.end_frame(&self.last_render, &counts, &parts);
        self.net_profile.end_frame(&self.net.stats);
        // AURORA_SYNC_SLICE=alt: the sync of each second by one way or the
        // other, named at the end of its line
        if logged || self.sync_alt == 0 {
            self.sync_alt += 1;
            if let Some(old) = crate::scene::sync_sets::alternate_slice(self.sync_alt) {
                self.frame_profile.set_mark(if old { " | xmode=old" } else { " | xmode=new" });
            }
            // AURORA_PROBE_WALK=alt: the whole probe list walked one second in three
            if aurora_render::probes::ProbeWalk::from_env() == aurora_render::probes::ProbeWalk::Alternate {
                let old = self.frame_profile.probe_walk_list();
                self.frame_profile.set_mark(if old { " | xmode=old" } else { " | xmode=new" });
            }
            // AURORA_NAME_TAGS=alt: the classic tags one second in three
            if name_tags_env() == Some(NameTagsEnv::Alternate) {
                let old = self.sync_alt.is_multiple_of(3);
                self.name_tags_alt_classic = old;
                self.frame_profile.set_mark(if old { " | xmode=old" } else { " | xmode=new" });
            }
        }
    }

    /// Local chat from the chat bar or the conversations window: a chat bar
    /// command (FSCmdLine) or a line sent on its channel.
    fn send_local_chat(&mut self, mut c: ui::chat::OutgoingChat) {
        if c.channel == 0 {
            let (revised, gesture) = self.gesture_state.manager.revise(&c.message);
            c.message = revised;
            if let Some(id) = gesture {
                self.gesture_play(id, None);
            }
        }
        if c.message.is_empty() || (c.channel == 0 && self.chat_command(&c.message)) {
            return;
        }
        // :shortcode: emoji
        self.send(NetCommand::Chat {
            message: ui::emoji::expand_shortcodes(&c.message),
            channel: c.channel,
            chat_type: c.chat_type,
        });
    }

    fn handle_actions(&mut self, a: UiActions, event_loop: &ActiveEventLoop) {
        if let Some(action) = a.exit {
            self.confirm_exit(action);
            return;
        }
        if a.update_quit {
            self.shutdown(event_loop);
            return;
        }
        match a.login {
            LoginAction::Login if !self.updater.blocks_login() && aurora_updater::test_environment().is_none() => self.start_login(),
            LoginAction::CheckUpdates => self.updater.check(self.net.runtime(), true),
            LoginAction::ReportBug => self.bug_report.requested = true,
            LoginAction::Quit => self.request_exit(),
            LoginAction::Preferences => ui::widgets::toggle_floater(&self.egui_ctx, "options", &mut self.panels.settings),
            LoginAction::None | LoginAction::Login => {}
        }
        if let Some(c) = a.chat {
            self.send_local_chat(c);
        }
        for (to, text) in a.ims {
            let text = ui::emoji::expand_shortcodes(&text);
            if self.world.is_group_session(&to) {
                self.world.send_session_im(to, &text);
                continue;
            }
            if self.world.mutes.is_muted_id(&to) {
                let name = self.world.legacy_name(&to).unwrap_or_else(|| self.world.social.name_of(&to));
                self.world.unblock(to, &name, 0);
                self.world.system_message(aurora_i18n::tr!(
                    "app-was-unblocked-because-you-sent-them-an-im",
                    name = name.to_string()
                ));
            }
            self.world.own_im(to, &text);
            self.send(NetCommand::SendIm { to, message: text });
        }
        // menu choices made anywhere in the interface (ui::context::request)
        for act in ui::context::take_requests(&self.egui_ctx) {
            self.on_ctx_action(act);
        }
        if let Some(act) = a.ctx_action {
            self.on_ctx_action(act);
        }
        if let Some(amount) = a.object_confirm
            && let Some(cmd) = self.interactions.confirm(&self.world, amount)
        {
            self.send(cmd);
        }
        if let Some((old, new)) = a.set_display_name {
            self.send(NetCommand::SetDisplayName { old, new });
        }
        for cmd in a.currency {
            self.send(cmd);
        }
        // the invitation to edit first (handle_lure), for all those chosen
        if !a.offer_tp.is_empty() {
            self.contacts_ui.ask_teleport_offer(a.offer_tp, &self.world.region_name());
        }
        if let Some(asset) = a.landmark {
            self.begin_teleport(aurora_i18n::tr!("common-landmark").into(), false);
            self.send(NetCommand::TeleportLandmark(asset));
        }
        if a.cancel_loading {
            self.net.send(NetCommand::Logout);
            self.back_to_login(Some(aurora_i18n::tr!("app-login-canceled").into()));
        }
        match a.audio.unwrap_or(ui::audio::AudioAction::None) {
            ui::audio::AudioAction::None => {}
            ui::audio::AudioAction::ToggleMusic => self.music_playing = !self.music_playing,
            ui::audio::AudioAction::ToggleMedia => {
                if self.media.parcel_playing() {
                    self.media.stop_parcel();
                } else {
                    self.media.play_parcel(&self.world, &self.settings.media);
                }
            }
            // applied to the audio engine each frame; saved with the settings on exit
            ui::audio::AudioAction::Changed => {}
            ui::audio::AudioAction::OpenPreferences => {
                self.panels.settings = true;
                self.options_ui.tab = ui::options::TAB_AUDIO;
            }
        }
        match a.bar {
            BarAction::CheckUpdates => self.updater.check(self.net.runtime(), true),
            BarAction::ReportBug => self.bug_report.requested = true,
            BarAction::ReportAbuse => self.open_report_abuse(crate::report_abuse::ReportTarget::Nothing),
            BarAction::Import => {
                self.imports.toggle_launcher(&self.egui_ctx);
                self.egui_ctx.request_repaint();
            }
            BarAction::ShowHuds(visible) => {
                if self.hud_drag.is_some() {
                    self.on_left_release();
                }
                self.release_object_hold();
                self.settings.show_huds = visible;
                self.settings.save();
                if !visible {
                    self.scene.lists.hud_view = None;
                    self.media.unfocus();
                }
            }
            BarAction::HudZoom(zoom) => self.world.hud_zoom = zoom.clamp(0.1, 1.0),
            BarAction::None => {}
            BarAction::Build(t) => {
                match t {
                    1 => self.build.open_build(crate::build::Tool::Edit),
                    2 => self.build.open_build(crate::build::Tool::Create),
                    3 => self.build.open_build(crate::build::Tool::Land),
                    _ if self.build.open => self.build.close(&mut self.world, &mut self.settings.build),
                    // LLToolMgr::enterBuildMode: the Create tool
                    _ => self.build.open_build(crate::build::Tool::Create),
                }
                self.flush_build();
            }
            BarAction::Logout => {
                self.want_scene_capture = true;
                self.net.send(NetCommand::Logout);
                self.world.system_message(aurora_i18n::tr!("app-logging-out"));
            }
            BarAction::Quit => {
                self.request_exit();
            }
            BarAction::TeleportHome => {
                self.begin_teleport(aurora_i18n::tr!("common-home").into(), false);
                self.send(NetCommand::TeleportHome);
            }
            BarAction::TeleportBack => self.teleport_history(true),
            BarAction::TeleportForward => self.teleport_history(false),
            BarAction::TeleportTo(text) => self.teleport_to_text(&text),
            BarAction::SetStatus(m) => {
                self.world.set_away(m.away);
                self.auto_away = false;
                self.world.set_dnd(m.dnd);
                self.world.set_invisible(m.invisible);
                let a = &mut self.settings.autoresponse;
                a.autorespond = m.autorespond;
                a.autorespond_nonfriends = m.autorespond_nonfriends;
                a.reject_teleports = m.reject_teleports;
                a.reject_group_invites = m.reject_group_invites;
                a.reject_friendship = m.reject_friendship;
                a.ignore_adhoc = m.ignore_adhoc;
                a.report_ignored_adhoc = m.report_ignored_adhoc;
                a.adhoc_from_friends = m.adhoc_from_friends;
                self.world.status.cfg = a.clone();
                self.settings.save();
            }
            BarAction::MyProfile => {
                let me = self.world.agent_id;
                self.profile_ui.open(&mut self.world, me);
            }
            BarAction::BuyCurrency => self.open_buy_currency(),
            BarAction::Marketplace => self.marketplace_ui.open(None),
            BarAction::MarketplaceListings => self.open_marketplace_listings(),
            BarAction::ChatPrefs => {
                self.panels.settings = true;
                self.options_ui.tab = ui::options::TAB_CHAT;
            }
            BarAction::VoicePrefs => {
                self.panels.settings = true;
                self.options_ui.tab = ui::options::TAB_AUDIO;
            }
            BarAction::ToggleFly => self.toggle_fly(),
            BarAction::SetDoubleClick(choice) => {
                // like the "Double-click teleport enabled / disabled" line
                // Firestorm writes to the nearby chat
                self.settings.double_click = choice;
                self.settings.save();
                self.world
                    .system_message(aurora_i18n::tr!("app-double-click-ground-set", action = choice.label().to_string()));
            }
            BarAction::ToggleMouselook => self.toggle_mouselook(),
            BarAction::SitGround => self.toggle_ground_sit(),
            BarAction::StandUp => self.send(NetCommand::OneShotControl(control::STAND_UP)),
            BarAction::ResetCamera => self.reset_camera_view(),
            BarAction::BanLines(v) => self.settings.maps.ban_lines = v,
            BarAction::SharedEnvironment => self.world.eep.clear_local(),
            BarAction::NewEnvironment(kind) => match kind {
                crate::world::env_select::SettingsKind::Day => self.open_new_day_editor(),
                kind => self.open_env_editor_snapshot(kind),
            },
            BarAction::TeleportHistory => self.toggle_teleport_history(),
            BarAction::CreateLandmark => self.open_create_landmark(None),
            BarAction::LandmarkStar(star) => self.landmark_star(star),
            BarAction::PlaceBar { stretch, rights_text } => {
                self.settings.place_bar_stretch = stretch;
                self.settings.place_bar_rights_text = rights_text;
                self.settings.save();
            }
            BarAction::ShowBugButton(shown) => {
                self.settings.show_bug_report = shown;
                self.settings.save();
            }
            BarAction::Favorite(f) => self.favorite_action(f),
        }
    }

    /// Places › Favorites, the first tab.
    fn open_places_favorites(&mut self) {
        self.places_ui.tab = 0;
        ui::widgets::open_floater(&self.egui_ctx, "places", &mut self.panels.places);
    }

    /// A click in the favourites of the islands.
    fn favorite_action(&mut self, action: ui::bars::favorites::FavAction) {
        use ui::bars::favorites::FavAction;
        match action {
            // LLFavoritesBarCtrl::onButtonClick: no confirmation
            FavAction::Teleport(asset) => {
                self.begin_teleport(aurora_i18n::tr!("common-landmark").into(), false);
                self.send(NetCommand::TeleportLandmark(asset));
            }
            FavAction::ShowOnMap(g) => {
                self.world.map.track_location(g.x, g.y, g.z as f32, false);
                self.panels.world_map = true;
            }
            FavAction::Edit(item) => self.landmark_star(ui::bars::StarAction::Edit(item)),
            FavAction::Move(item, rank) => self.world.favorites.rearrange(&self.world.inventory, item, rank),
            FavAction::AddHere => self.landmark_star(ui::bars::StarAction::AddFavorite),
            FavAction::OpenPlaces => self.open_places_favorites(),
            FavAction::Access(access) => {
                self.settings.favorites_access = access;
                self.settings.save();
            }
            FavAction::Fold(folded) => {
                self.settings.favorites_row_folded = folded;
                self.settings.save();
            }
        }
    }

    /// Where the favourites live: the setting, or what `AURORA_DEMO_BARS`
    /// asks for in demo (never saved).
    fn favorites_access(&self) -> crate::settings::FavoritesAccess {
        let demo = if self.demo { ui::bars::demo::get().access } else { None };
        demo.unwrap_or(self.settings.favorites_access)
    }

    /// The dock's Favorites button follows where the favourites live: it
    /// comes with `FavoritesAccess::Dock` (after Places, as in the mockup)
    /// and goes when they move elsewhere. A button added by hand stays as
    /// long as the choice does not change.
    fn follow_favorites_access(&mut self) {
        use crate::settings::FavoritesAccess;
        if self.demo && ui::bars::demo::get().access.is_some() {
            return;
        }
        let access = self.settings.favorites_access;
        let before = self.favorites_access_applied.replace(access);
        if before.is_none() || before == Some(access) {
            return;
        }
        let layout = self.dock_layout.of(&self.skin.layout).clone();
        let toolbar = self.settings.toolbar.get_or_insert_default();
        let mut buttons = toolbar.buttons(&layout).to_vec();
        let has = buttons.iter().any(|b| b.command == "favorites");
        if access == FavoritesAccess::Dock && !has {
            let at = buttons.iter().position(|b| b.command == "places").map_or(buttons.len(), |i| i + 1);
            buttons.insert(
                at,
                ui::skin::ToolButton {
                    icon: "bookmarks-filled".into(),
                    command: "favorites".into(),
                    tooltip: String::new(),
                },
            );
        } else if access != FavoritesAccess::Dock && has {
            buttons.retain(|b| b.command != "favorites");
        } else {
            return;
        }
        toolbar.buttons = Some(buttons);
        self.settings.save();
    }

    /// The favourites as the bars show them this frame. Their landmarks,
    /// regions and thumbnails are asked for only while they are on screen
    /// (the row always is), as LLFavoritesOrderStorage asks the landmarks
    /// for its SLURLs.
    fn favorites_view(&mut self, shown: bool, here: bool) -> ui::bars::favorites::View {
        use crate::settings::FavoritesAccess;
        use crate::world::inventory::FetchState;
        let access = self.favorites_access();
        // AURORA_DEMO_BARS=added: the last one comes from the star
        if self.demo
            && self.world.favorites.added.is_none()
            && let Some(id) = crate::demo::place::added_favorite()
            && self.world.inventory.items.contains_key(&id)
        {
            self.world.favorites.added = Some((id, std::time::Instant::now()));
        }
        let shown = shown || access == FavoritesAccess::Row;
        let folder = crate::world::appearance::system_folder(&self.world.inventory, crate::world::favorites::FOLDER_FAVORITES);
        let state = folder.and_then(|f| self.world.inventory.folders.get(&f)).map(|f| f.state);
        if shown
            && let Some(f) = folder
            && matches!(state, Some(FetchState::Unknown | FetchState::Failed))
        {
            self.world.inventory.request(f);
        }
        let ids = self.world.favorites.list(&self.world.inventory).to_vec();
        let mut list = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(item) = self.world.inventory.items.get(&id) else {
                continue;
            };
            let (asset, name, thumb) = (item.asset_id, item.name.clone(), item.thumbnail);
            let global = match self.world.landmarks.peek(asset) {
                crate::world::landmarks::Resolved::Ready { global, .. } => Some(global),
                crate::world::landmarks::Resolved::Pending if shown => {
                    self.world.landmarks.resolve(asset);
                    None
                }
                _ => None,
            };
            let sim = global.and_then(|g| self.world.map.sim_at_global(g.x, g.y).map(|(_, s)| (s.name.clone(), s.access)));
            if shown
                && sim.is_none()
                && let Some(g) = global
            {
                let (x, y) = ((g.x / 256.0) as i64, (g.y / 256.0) as i64);
                self.world.map.want_regions(x, y, x, y);
            }
            let thumbnail = (!thumb.is_nil()).then_some(thumb).and_then(|t| {
                let loaded = self.ui_images.get(&t).map(|h| h.id());
                if loaded.is_none() && shown {
                    self.favorite_images.insert(t);
                }
                loaded
            });
            let slurl = global.zip(sim.as_ref()).map(|(g, (region, _))| {
                let pos = glam::Vec3::new(g.x.rem_euclid(256.0) as f32, g.y.rem_euclid(256.0) as f32, g.z as f32);
                crate::slurl::make(region, pos)
            });
            list.push(ui::bars::favorites::Favorite {
                item: id,
                asset,
                name,
                region: sim.as_ref().map(|s| s.0.clone()).unwrap_or_default(),
                rating: sim.as_ref().map_or("", |s| ui::bars::maturity_name(s.1)),
                global,
                slurl,
                thumbnail,
            });
        }
        // marked as just added for a minute
        let added = self
            .world
            .favorites
            .added
            .filter(|(_, at)| at.elapsed() < std::time::Duration::from_secs(60))
            .map(|(id, _)| id);
        // this parcel is a favourite: the parcel's landmark is one, or one of
        // the favourites lies in it
        let parcel_cells = self.world.parcel.as_ref().zip(self.world.main_origin());
        let here = here
            || list.iter().any(|f| {
                f.global
                    .zip(parcel_cells)
                    .is_some_and(|(g, (parcel, origin))| crate::world::landmarks::in_parcel(&parcel.bitmap, origin, g))
            });
        ui::bars::favorites::View {
            list,
            added,
            here,
            access,
            folded: if self.demo {
                ui::bars::demo::get().folded.unwrap_or(self.settings.favorites_row_folded)
            } else {
                self.settings.favorites_row_folded
            },
            loading: state != Some(FetchState::Fetched),
        }
    }

    /// Firestorm's bars or the islands, and how the islands are set: the
    /// settings, or what `AURORA_DEMO_BARS` asks for in demo (never saved).
    fn bars_look(&self) -> (bool, ui::bars::IslandsLook) {
        let s = &self.settings;
        let demo = if self.demo { ui::bars::demo::get() } else { Default::default() };
        let look = ui::bars::IslandsLook {
            stretch: demo.stretch.unwrap_or(s.place_bar_stretch),
            rights_text: demo.rights_text.unwrap_or(s.place_bar_rights_text),
        };
        (demo.classic.unwrap_or(s.classic_bars), look)
    }

    /// The last places of the teleport history for the place menu, newest
    /// first, without the one we stand in (its last entry): two, as in the
    /// mockup.
    fn recent_places(&self) -> Vec<ui::bars::RecentPlace> {
        self.world
            .tp_storage
            .items
            .iter()
            .rev()
            .skip(1)
            .take(2)
            .map(|item| {
                let (name, region) = match item.title.rsplit_once(", ") {
                    Some((name, region)) => (name.to_owned(), region.to_owned()),
                    None => (item.title.clone(), String::new()),
                };
                ui::bars::RecentPlace {
                    name,
                    region,
                    slurl: item.slurl.clone(),
                }
            })
            .collect()
    }

    /// The grid of the session for the web search: only the main grid's
    /// session is known to search.secondlife.com.
    fn search_grid(&self) -> crate::world::search::SearchGrid {
        use crate::world::search::SearchGrid;
        match self.settings.grid {
            crate::settings::GridChoice::SecondLife => SearchGrid::Main,
            crate::settings::GridChoice::SecondLifeBeta => SearchGrid::Beta,
            crate::settings::GridChoice::Custom => SearchGrid::Other,
        }
    }

    /// The search page of the web tab (FSPanelSearchWeb::loadURL), None on
    /// a grid without one. On the main grid it carries the session id of the
    /// agent, as Firestorm's does: it is only given to the embedded browser,
    /// never logged, shown or saved; on the beta grid it is anonymous.
    /// AURORA_DEMO_SEARCH_WEB=<page>: a page of the tester's choice in demo,
    /// which loads nothing by itself and has no session.
    fn search_home(&self) -> Option<crate::world::search::WebHome> {
        if self.demo {
            let url = std::env::var("AURORA_DEMO_SEARCH_WEB").ok().filter(|u| !u.trim().is_empty())?;
            return Some(crate::world::search::WebHome { url, identified: false });
        }
        crate::world::search::web_home(
            self.search_grid(),
            self.world.agent_access(),
            aurora_i18n::language(),
            self.search_session(),
        )
    }

    /// LLFloaterReg::showInstance("add_landmark"): World › Landmark this
    /// Place, the Places "add" entries and About Land's Landmark button;
    /// `dest` is the folder chosen in Places.
    pub(super) fn open_create_landmark(&mut self, dest: Option<uuid::Uuid>) {
        if let Some(cmd) = self.create_landmark_ui.open(&self.world, dest) {
            self.send(cmd);
        }
    }

    /// hasParcelLandmark: the landmark lying on the agent's parcel (the lit
    /// star; the "add landmark" entries are greyed out meanwhile).
    fn parcel_landmark(&mut self) -> Option<ui::landmark_create::ParcelLandmark> {
        // the Favorites and Landmarks folders are fetched for it, as
        // Firestorm has them after its background inventory fetch
        if self.world.inventory.generation != self.landmark_folders_checked {
            for id in crate::world::landmarks::unfetched_folders(&self.world.inventory) {
                self.world.inventory.request(id);
            }
            self.landmark_folders_checked = self.world.inventory.generation;
        }
        let origin = self.world.main_origin();
        self.create_landmark_ui
            .parcel_landmark(&self.world.inventory, &mut self.world.landmarks, self.world.parcel.as_ref(), origin)
    }

    /// The navigation bar's star.
    fn landmark_star(&mut self, action: ui::bars::StarAction) {
        use ui::bars::StarAction;
        use ui::landmark_create::{FOLDER_FAVORITES, FOLDER_LANDMARKS};
        let cmd = match action {
            StarAction::AddFavorite => {
                // its row is marked as just added once it arrives
                self.world.favorites.expect_added();
                self.create_landmark_ui.quick(&self.world, FOLDER_FAVORITES)
            }
            StarAction::AddLandmark => self.create_landmark_ui.quick(&self.world, FOLDER_LANDMARKS),
            StarAction::MoreOptions => self.create_landmark_ui.open(&self.world, None),
            StarAction::Edit(item) => {
                self.create_landmark_ui.open_edit(&self.world.inventory, item);
                None
            }
        };
        if let Some(cmd) = cmd {
            self.send(cmd);
        }
    }

    /// The Marketplace page of the grid (World › SL Marketplace), None on a
    /// grid without one. AURORA_DEMO_MARKETPLACE=<page>: a page of the
    /// tester's choice in demo, which also stands for the Marketplace links.
    fn marketplace_home(&self) -> Option<String> {
        if self.demo {
            return std::env::var("AURORA_DEMO_MARKETPLACE").ok().filter(|u| !u.trim().is_empty());
        }
        ui::marketplace::home(&self.settings.grid).map(str::to_owned)
    }

    /// The session id the web tab's search page is opened with; none in
    /// demo.
    fn search_session(&self) -> Option<uuid::Uuid> {
        if self.demo {
            return None;
        }
        self.world.login.as_ref().map(|l| l.session_id).filter(|id| !id.is_nil())
    }

    /// What the search window asked for.
    fn apply_search_actions(&mut self, ctx: &egui::Context, actions: Vec<ui::search::SearchAction>) {
        use ui::search::SearchAction;
        let mut save = false;
        for a in actions {
            match a {
                SearchAction::Find(request) => {
                    let access = self.world.agent_access();
                    self.world.search.find(request, access, Instant::now());
                }
                SearchAction::Page(tab, forward) => self.world.search.turn_page(tab, forward),
                SearchAction::Select(tab, key) => self.world.search.select(tab, key),
                SearchAction::Sort(tab, column) => {
                    if let Some(list) = self.world.search.list_mut(tab) {
                        list.sort_by(column);
                    }
                }
                SearchAction::RefreshGroup(id) => self.send(NetCommand::GroupProfileRequest(id)),
                SearchAction::ShowOnMap(g) => {
                    self.world.map.track_location(g.x, g.y, g.z as f32, false);
                    self.panels.world_map = true;
                }
                SearchAction::Teleport(g) => {
                    self.world.map.track_location(g.x, g.y, g.z as f32, true);
                    // FSLegacySearchActionOnTeleport
                    match self.settings.search.teleport_action {
                        1 => self.panels.search = false,
                        2 => ui::widgets::minimize_floater(ctx, ui::search::WINDOW),
                        _ => {}
                    }
                }
                SearchAction::SettingsChanged => save = true,
            }
        }
        if save {
            self.settings.save();
        }
    }

    /// LLPanelGroupBulkBan::submit's reasons: who was not banned, and why.
    fn tell_bulk_ban(&mut self, done: &crate::world::group_data::BulkBan) {
        let names =
            |ids: &[uuid::Uuid], world: &crate::world::World| ids.iter().map(|id| world.social.name_of(id)).collect::<Vec<_>>().join(", ");
        if !done.already.is_empty() {
            let text = aurora_i18n::tr!("ui-group-admin-already-banned", names = names(&done.already, &self.world));
            self.world.system_message(text);
        }
        if done.myself {
            self.world.system_message(aurora_i18n::tr!("ui-group-admin-cannot-ban-yourself"));
        }
        if !done.over_limit.is_empty() {
            let text = aurora_i18n::tr!("ui-group-admin-ban-limit", names = names(&done.over_limit, &self.world));
            self.world.system_message(text);
        }
    }

    /// sendEjectNotifications: "You ejected X from the group Y", for each.
    fn notify_ejected(&mut self, group: uuid::Uuid, members: &[uuid::Uuid]) {
        let group_name = self.world.search.groups.get(&group).map(|g| g.name.clone()).unwrap_or_default();
        let me = self.world.agent_id;
        for m in members.iter().filter(|m| **m != me) {
            let name = self.world.social.name_of(m);
            self.world
                .system_message(aurora_i18n::tr!("ui-group-admin-ejected", name = name, group = group_name.clone()));
        }
    }

    /// What the group profile window asked (LLGroupActions).
    fn apply_group_actions(&mut self, ctx: &egui::Context, actions: Vec<ui::group_profile::GroupAction>) {
        use ui::group_profile::GroupAction;
        let mut search = Vec::new();
        for a in actions {
            match a {
                GroupAction::Search(a) => search.push(a),
                GroupAction::Load { group, member } => {
                    self.world.group_data.open(group, member);
                    // LLPanelGroupExperiences::setGroupID → activate: the
                    // tab's count is known before it is opened
                    if member {
                        self.world.experiences.request_group(group);
                    }
                }
                GroupAction::Refresh { group, member } => {
                    self.send(NetCommand::GroupProfileRequest(group));
                    self.world.group_data.refresh(group, member);
                    self.world.group_money.refresh(group);
                    if member {
                        self.world.experiences.request_group(group);
                    }
                }
                GroupAction::Join(group) => self.send(NetCommand::JoinGroup(group)),
                GroupAction::Eject { group, members } | GroupAction::Ban { group, members } if members.is_empty() => {
                    log::debug!("group {group}: nobody to eject");
                }
                GroupAction::Eject { group, members } => {
                    self.notify_ejected(group, &members);
                    self.world.group_data.eject(group, members, self.world.agent_id);
                }
                GroupAction::Ban { group, members } => {
                    self.notify_ejected(group, &members);
                    self.world.group_data.ban(group, members, self.world.agent_id);
                }
                GroupAction::ChangeRoles { group, changes } => self.world.group_data.change_roles(group, changes),
                GroupAction::UpdateRoles { group, updates } => self.world.group_data.update_roles(group, updates),
                GroupAction::UpdateInfo { group, info } => {
                    self.world.group_data.update_info(group, info);
                    // the profile again, to show what the server kept
                    self.send(NetCommand::GroupProfileRequest(group));
                }
                GroupAction::Create { name, info } => {
                    self.world.groups.creating = Some(name.clone());
                    self.send(NetCommand::CreateGroup { name, info });
                }
                GroupAction::RequestBans(group) => self.world.group_data.request_bans(group),
                GroupAction::Unban { group, ids } => self.world.group_data.unban(group, ids),
                GroupAction::BanResidents { group, ids } => {
                    let me = self.world.agent_id;
                    let done = self.world.group_data.ban_residents(group, ids, me);
                    self.tell_bulk_ban(&done);
                }
                GroupAction::RefreshNotices(group) => self.world.group_data.request_notices(group),
                GroupAction::OpenNotice { group, notice } => {
                    log::info!("group notice asked: {notice}");
                    self.world.group_data.open_notice(group, notice);
                }
                GroupAction::SaveAttachment(group) => self.world.save_notice_attachment(group),
                GroupAction::SendNotice {
                    group,
                    subject,
                    message,
                    item,
                } => {
                    // send_group_notice: an instant message to the group
                    let (text, bucket) = aurora_net::groups::notice_to_send(&subject, &message, item);
                    self.send(NetCommand::SendImDialog {
                        to: group,
                        dialog: crate::world::notifications::im::GROUP_NOTICE,
                        id: uuid::Uuid::nil(),
                        message: text,
                        bucket,
                    });
                    let me = self.world.notifications.own_name.clone();
                    self.world.group_data.add_sent_notice(group, subject, me, item.is_some());
                }
                GroupAction::Invite { group, invites } => {
                    log::info!("group invite: {} resident(s) invited", invites.len());
                    self.send(NetCommand::InviteToGroup { group, invites });
                }
                // LLGroupActions::startCall
                GroupAction::Call(group) => self.on_ctx_action(ui::context::CtxAction::Call(group)),
                GroupAction::Activate(group) => self.send(NetCommand::ActivateGroup(group)),
                GroupAction::SetTitle { group, role } => self.world.group_data.set_title(group, role),
                GroupAction::SetFlags {
                    group,
                    accept_notices,
                    list_in_profile,
                } => self.world.groups.set_flags(group, accept_notices, list_in_profile),
                GroupAction::ShowInPeople(name) => {
                    self.panels.people = true;
                    self.panels.people_tab = 2;
                    self.people_ui.filter = name;
                }
                GroupAction::LoadLand { group, ledger } => {
                    // the land types name the parcels' "Type" column
                    self.world.search.want_land_types();
                    self.world.group_money.open(group, ledger);
                }
                GroupAction::LandPage { group, ledger } => self.world.group_money.request(group, ledger),
                GroupAction::LandStep { group, ledger, earlier } => self.world.group_money.step(group, ledger, earlier),
                GroupAction::SetContribution { group, contribution } => {
                    if !self.world.groups.set_contribution(group, contribution) {
                        self.world.system_message(aurora_i18n::tr!("ui-group-land-contribution-error"));
                    }
                }
                // onMapButton: "use the agent's z-height"
                GroupAction::ShowOnMap { x, y } => {
                    let z = self.world.agent_global().map_or(0.0, |g| g.z as f32);
                    self.world.map.track_location(x, y, z, false);
                    self.panels.world_map = true;
                }
                GroupAction::AboutLand { x, y } => {
                    ui::land::LandUi::select_at_global(&mut self.world, x, y);
                    self.panels.about_land = true;
                }
                GroupAction::LoadExperiences(group) => self.world.experiences.request_group(group),
                GroupAction::ExperienceProfile(experience) => self.world.experiences.open_profile(experience),
            }
        }
        self.apply_search_actions(ctx, search);
    }

    /// toggleTeleportHistory (llviewermenu.cpp) without the standalone
    /// history floater: hide Places when shown, else open it on the history.
    fn toggle_teleport_history(&mut self) {
        if self.panels.places {
            self.panels.places = false;
        } else {
            self.world.place_details.close_panel();
            self.places_ui.open_tab(ui::places::TAB_HISTORY);
            self.panels.places = true;
        }
    }

    /// Remembered password of the user shown on the login screen: read
    /// when the grid or the name changes, forgotten as soon as the option is
    /// switched off.
    fn sync_remembered_password(&mut self) {
        let key = (self.settings.login_uri(), self.settings.username.trim().to_lowercase());
        if !self.login_form.accounts_loaded && !self.demo {
            self.login_form.accounts = crate::login_accounts::load();
            if !crate::settings::config_dir().join("login_accounts.json").exists()
                && self.settings.remember_username
                && !key.1.is_empty()
                && !self.login_form.accounts.iter().any(|a| a.key() == key)
            {
                self.login_form.accounts =
                    crate::login_accounts::save(crate::login_accounts::Account::from_settings(&self.settings, false, None));
            }
            self.login_form.accounts_loaded = true;
            self.login_form.places.load(&self.login_form.accounts);
            if !self.login_form.editing
                && let Some(account) = self.login_form.accounts.iter().find(|account| account.key() == key)
            {
                account.apply(&mut self.settings);
            }
            let (tx, rx) = crossbeam_channel::unbounded();
            let portraits: Vec<_> = self
                .login_form
                .accounts
                .iter()
                .filter_map(|a| a.agent)
                .filter(|agent| !self.login_form.portraits.contains_key(agent))
                .collect();
            self.login_form.portrait_rx = Some(rx);
            rayon::spawn(move || {
                for agent in portraits {
                    let path = crate::settings::account_dir(&agent).join("login_portrait.png");
                    let Ok(bytes) = std::fs::read(path) else {
                        continue;
                    };
                    if bytes.len() > 4 * 1024 * 1024 {
                        continue;
                    }
                    let Ok(mut reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() else {
                        continue;
                    };
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(512);
                    limits.max_image_height = Some(512);
                    limits.max_alloc = Some(4 * 1024 * 1024);
                    reader.limits(limits);
                    let Ok(image) = reader.decode() else {
                        continue;
                    };
                    let rgba = image.to_rgba8();
                    let image = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                    let _ = tx.send((agent, image));
                }
            });
        }
        if let Some(rx) = &self.login_form.portrait_rx {
            for (agent, image) in rx.try_iter() {
                self.login_form.portraits.insert(
                    agent,
                    self.egui_ctx
                        .load_texture(format!("login-portrait-{agent}"), image, egui::TextureOptions::LINEAR),
                );
            }
        }
        self.sync_remembered_mfa(&key);
        // Derez snapshots retain texture IDs until the transition completes.
        if self.login_form.transition.is_none() {
            self.login_form.places.poll(&self.egui_ctx, &mut self.login_form.accounts);
        }
        if self.demo {
            if self.login_form.accounts_loaded {
                self.login_form.stored = self.settings.remember_password.then(|| "demo-login-marker".into());
                self.login_form.stored_for = Some(key);
                return;
            }
            // The login demo uses a synthetic marker and never accesses the OS store.
            if !self.settings.remember_password || self.login_form.stored_for.as_ref() != Some(&key) {
                self.login_form.stored = None;
            }
            return;
        }
        let was_checked = self.login_form.remember_checked;
        let changed = self.login_form.stored_for.as_ref() != Some(&key);
        if changed {
            if self.login_form.stored_for.is_some() {
                self.login_form.password.clear();
                self.login_form.mfa_token.clear();
                self.settings.mfa_hash.clear();
            }
            self.login_form.stored = None;
            if !self.login_form.editing
                && let Some(account) = self.login_form.accounts.iter().find(|a| a.key() == key)
            {
                self.settings.remember_password = account.remember_password;
            }
            self.login_form.stored_for = Some(key.clone());
        }
        self.login_form.remember_checked = self.settings.remember_password;
        if !self.settings.remember_password {
            if self.login_form.stored.take().is_some() {
                crate::credentials::forget(&key.0, &key.1);
                if let Some(mut account) = self.login_form.accounts.iter().find(|a| a.key() == key).cloned() {
                    account.remember_password = false;
                    self.login_form.accounts = crate::login_accounts::save(account);
                }
            }
            return;
        }
        if changed || !was_checked {
            self.login_form.stored = crate::credentials::load(&key.0, &key.1);
            self.login_form.stored_for = Some(key);
        }
    }

    fn sync_remembered_mfa(&mut self, key: &(String, String)) {
        if self.demo || self.login_form.mfa_for.as_ref() == Some(key) {
            return;
        }
        self.login_form.mfa_authorization = crate::credentials::load_mfa(&key.0, &key.1);
        // Migrate the old global value only for the user/grid it was saved with.
        if !self.settings.mfa_hash.is_empty() && !key.1.is_empty() {
            if self.login_form.mfa_authorization.is_none() {
                let authorization = crate::credentials::MfaAuthorization::renewed(
                    std::mem::take(&mut self.settings.mfa_hash),
                    None,
                    crate::credentials::now(),
                );
                crate::credentials::store_mfa(&key.0, &key.1, &authorization);
                self.login_form.mfa_authorization = Some(authorization);
            }
            self.settings.mfa_hash.clear();
            self.settings.save();
        }
        self.login_form.mfa_token.clear();
        self.login_form.remember_mfa = self.login_form.mfa_authorization.is_some()
            || self
                .login_form
                .accounts
                .iter()
                .find(|account| account.key() == *key)
                .is_some_and(|account| account.remember_mfa);
        self.login_form.mfa_for = Some(key.clone());
    }

    /// The preferences window, in the world and over the login screen. Every
    /// setting is the viewer's own (settings.rs), so all of them can change
    /// before login; only the transcripts folder belongs to an account, and
    /// its group says so until one is logged in (`chat_log_dir` stays empty).
    fn draw_options(&mut self, ctx: &egui::Context, p: &Palette, a: &mut UiActions) {
        let mut open = self.panels.settings;
        self.options_ui.cache_usage = self.disk_cache.usage();
        self.options_ui.cache_clearing = self.disk_cache.is_clearing();
        self.options_ui.chat_log_dir = self.world.chat_log.dir().map(std::path::Path::to_path_buf);
        if let Some(done) = self.world.chat_log.take_erased() {
            self.options_ui.chat_log_status = match done {
                Ok(count) => aurora_i18n::tr!("ui-options-chat-logs-erased", count = count),
                Err(e) => {
                    log::warn!("conversation transcripts: erasing failed ({e})");
                    aurora_i18n::tr!("ui-options-chat-logs-erase-failed").to_owned()
                }
            };
        }
        self.options_ui.maturity = self.world.login.is_some().then(|| self.world.agent_access());
        let res = ui::options::show(ctx, p, &self.skin.icons, &mut self.settings, &mut self.options_ui, &mut open);
        if let Some(maturity) = res.maturity {
            self.world.set_maturity_preference(maturity);
        }
        if !open {
            self.options_ui.capture = None;
        }
        if res.clear_cache {
            self.disk_cache.clear();
            self.net.send(NetCommand::ClearObjectCache);
        }
        if res.open_chat_logs
            && let Some(dir) = self.world.chat_log.dir()
        {
            // the folder may not exist before the first line
            let _ = std::fs::create_dir_all(dir);
            if let Err(e) = crate::bug_report::files::open_directory(dir) {
                log::warn!("cannot open the transcripts folder: {e}");
            }
        }
        if res.erase_chat_logs {
            self.options_ui.chat_log_status.clear();
            self.world.chat_log.erase();
        }
        a.settings_changed = res.changed;
        a.font_changed = res.font_changed;
        if res.refresh_devices {
            self.devices_listed = false;
        }
        if res.reset_camera {
            self.reset_camera_all();
        }
        if res.reset {
            // the panels must not write the old time of day back
            self.panels.time_of_day = self.settings.time_of_day;
            self.settings.save();
            self.world.system_message(aurora_i18n::tr!("app-all-settings-have-been-reset"));
            ui::layout::forget(ctx);
        }
        if let Some(tab) = res.reset_section {
            if tab == 0 {
                self.panels.time_of_day = self.settings.time_of_day;
            }
            self.settings.save();
            self.world
                .system_message(aurora_i18n::tr!("app-section-settings-reset", section = ui::options::tab_name(tab)));
        }
        if self.panels.settings && !open {
            self.settings.save();
        }
        self.panels.settings = open;
    }

    fn draw_ui(&mut self, ui: &mut egui::Ui, projector: &ui::hud::Projector, renderer: &Renderer) -> UiActions {
        let mut a = UiActions::default();
        let p = self.palette;
        let ctx = ui.ctx().clone();
        let beacon_hovered = !ctx.is_pointer_over_egui() && !ctx.egui_wants_pointer_input() && self.beacon_hover();
        self.beacon_hit = None;
        let mut t = Instant::now();
        if self.exit_prompt.open {
            ui.disable();
            ctx.memory_mut(|m| m.set_modal_layer(egui::Modal::default_area(egui::Id::new("viewer_exit")).layer()));
        }
        a.update_quit = !self.exit_prompt.open
            && ui::update::show_real(
                &ctx,
                &p,
                &self.skin.icons,
                &mut self.updater,
                &mut self.settings,
                self.net.runtime(),
            );
        match self.screen {
            Screen::Login => {
                if let Some(demo) = &mut self.update_demo {
                    a.login = demo.show(ui, &p, &self.skin.icons, self.logo.as_ref());
                    a.exit = self.exit_prompt.show(&ctx, &p, false);
                    return a;
                }
                // `AURORA_DEMO_LOGIN_SUBMIT=<frame>`: press the open form's
                // Log in at that frame, as its button does (the editor
                // closes, an Add stays an Add while the request runs).
                if self.demo
                    && std::env::var("AURORA_DEMO_LOGIN_SUBMIT")
                        .ok()
                        .and_then(|frame| frame.parse::<u64>().ok())
                        .is_some_and(|frame| frame == self.frame_count)
                {
                    self.login_form.editing = false;
                    self.start_login();
                }
                if self.demo_mfa_challenge.is_some_and(|at| std::time::Instant::now() >= at) {
                    self.demo_mfa_challenge = None;
                    self.on_app_event(NetEvent::LoginFailed {
                        error: String::new(),
                        mfa_required: true,
                        tos_required: false,
                        bad_credentials: false,
                    });
                } else if self.demo_mfa_challenge.is_some() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                self.sync_remembered_password();
                let progress = self.login_progress.as_ref().map(|(m, f)| (m.as_str(), *f));
                self.login_info.poll();
                self.login_form.block_reason = self
                    .updater
                    .login_block_reason()
                    .or_else(|| aurora_updater::test_environment().map(|_| aurora_i18n::tr!("update-login-test")));
                a.login = ui::login::show(
                    ui,
                    &p,
                    &self.skin.icons,
                    self.logo.as_ref(),
                    &mut self.settings,
                    &mut self.login_form,
                    &self.login_info,
                    progress,
                );
                self.sync_remembered_password();
                // the cogwheel of the login screen (settings before login);
                // the screen's cards are areas of the same order, and a click
                // on one would raise it over the window
                self.draw_options(&ctx, &p, &mut a);
                if self.panels.settings {
                    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new("options")));
                }
                // Keep disconnected script drafts accessible for copying.
                for action in self
                    .script_editors
                    .show(&ctx, &p, &mut self.settings.script_editor, &mut self.settings.build.color_picker)
                {
                    self.apply_script_action(action);
                }
            }
            Screen::Loading { since } => {
                let stats = self.scene.textures.stats;
                let region = self.world.region_name();
                let terrain = self.world.terrain_progress();
                let base = if self.world.movement_complete { 0.55 } else { 0.35 };
                let raw = base + terrain * 0.35 + (stats.loaded as f32 / stats.total.max(1) as f32) * 0.1;
                // never move backwards
                // Target never moves backwards; the displayed value eases toward
                // it every frame (plus a slow creep) so the bar glides smoothly.
                self.loading_target = self.loading_target.max(raw.min(1.0));
                let dt = ctx.input(|i| i.stable_dt).min(0.1);
                let creep = (self.loading_target + 0.08).min(0.98);
                let goal = if self.loading_frac < self.loading_target {
                    self.loading_target
                } else {
                    creep
                };
                let rate = if self.loading_frac < self.loading_target { 2.5 } else { 0.08 };
                self.loading_frac += (goal - self.loading_frac) * (1.0 - (-rate * dt).exp());
                let frac = self.loading_frac;
                let info = ui::loading::LoadingInfo {
                    stage: &self.loading_stage,
                    fraction: frac,
                    region: &region,
                    objects: self.world.objects.len(),
                    textures: (stats.loaded, stats.total),
                    terrain,
                    elapsed: since.elapsed().as_secs_f32(),
                };
                let facts = ui::loading::PlaceFacts::gather(&mut self.world, &mut self.map_tiles, info.region, None, true);
                a.cancel_loading = ui::loading::show(
                    ui,
                    &p,
                    self.logo.as_ref(),
                    self.settings.loading_backdrop.then_some(&self.backdrop),
                    &info,
                    &facts.place(),
                );
            }
            Screen::World => {
                // the full-frame photo view: the world and its own strip only
                if self.photo_full_frame() {
                    self.photo_ui(&ctx, &p, renderer);
                    return a;
                }
                // a held photo (freeze frame) instead of the world, under the windows
                let held = self.photo_held(&ctx, &p);
                // what belongs to the world first: the HUD attachments, the bars and
                // every window cover the name tags, the floating texts and the beacon
                let voice_levels = if self.demo {
                    static VOICE_MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
                    let mut levels = crate::demo::voice_levels(
                        ctx.input(|i| i.time),
                        self.world.agent_id,
                        self.settings.audio.voice_enabled,
                        self.talking(),
                        self.voice.mic_level,
                        VOICE_MODE.get_or_init(|| std::env::var("AURORA_DEMO_VOICE").ok()).as_deref(),
                    );
                    crate::demo::tags::voice(&mut levels, ctx.input(|i| i.time));
                    crate::demo::people::voice(&mut levels, ctx.input(|i| i.time));
                    levels
                } else {
                    self.voice.levels(self.world.agent_id)
                };
                // the look-at crosshairs under the name tags (none over a held
                // photo, nor the walk mark)
                if !held {
                    ui::lookat_targets::draw(
                        &ctx,
                        &p,
                        &self.world,
                        &self.scene,
                        projector,
                        self.camera.position,
                        &self.look_at_settings(),
                        &mut self.look_at_fades,
                    );
                    self.draw_walk_mark(&ctx, &p, projector);
                }
                let classic_tags = self.classic_name_tags();
                // AURORA_DEMO_FRIEND_PLATES=1: the setting for this run, without saving it
                self.plates.friends_only_full =
                    self.settings.plates_friends_only_full || (self.demo && std::env::var_os("AURORA_DEMO_FRIEND_PLATES").is_some());
                self.update_plate_hover(&ctx);
                self.name_tags = if held {
                    Vec::new()
                } else {
                    ui::hud::draw(
                        &ctx,
                        &p,
                        &self.skin.icons,
                        &self.world,
                        &self.scene,
                        projector,
                        self.camera.position,
                        !self.camera.mouselook(),
                        self.settings.name_tag_distance,
                        &voice_levels,
                        &mut self.voice_tags,
                        {
                            let s = &self.settings;
                            // debug: on every tag
                            let all = s.debug.complexity_tags;
                            (all || s.tag_complexity).then_some(ui::hud::TagComplexity {
                                values: &self.scene.avatar_complexity,
                                areas: &self.scene.avatar_area,
                                too_complex: &self.scene.too_complex,
                                max: s.max_complexity,
                                max_area: s.max_attachment_area,
                                only_too_complex: !all && s.tag_complexity_too_complex_only,
                                own: all || s.tag_complexity_own,
                            })
                        },
                        self.scene.loading_bars.then(|| self.scene.loading_avatars()),
                        (!classic_tags).then_some(ui::hud::PlateFrame {
                            plates: &mut self.plates,
                            pictures: &self.avatar_pics,
                            too_complex: &self.scene.too_complex,
                            demo_too_complex: crate::demo::tags::too_complex().filter(|_| self.demo),
                        }),
                    )
                };
                if !self.camera.mouselook() && !held {
                    self.beacon_hit = ui::minimap::draw_beacon(&ctx, &p, &self.world, projector, beacon_hovered);
                }
                ui::sky_beacons::draw(
                    &ctx,
                    &p,
                    &self.world,
                    projector,
                    [self.settings.sun_beacon, self.settings.moon_beacon],
                );
                aurora_render::end_world_labels(&ctx);
                ui::hud::draw_attachments(&ctx, &self.world, &self.scene);
                t = self.ui_parts.lap(Part::UiTags, t);
                let parcel_landmark = self.parcel_landmark();
                self.follow_favorites_access();
                let favorites_shown = ui::bars::favorites::take_shown(&ctx);
                let here = parcel_landmark.as_ref().is_some_and(|lm| lm.favorite);
                let favorites = self.favorites_view(favorites_shown, here);
                let (classic_bars, islands) = self.bars_look();
                let own_name = self.world.own_name();
                // AURORA_DEMO_BARS: the long parcel name of the mockup (the
                // islands only: Firestorm's bars stay as they were)
                let parcel_name = if self.demo && ui::bars::demo::get().on && !classic_bars {
                    aurora_i18n::tr!("demo-bars-parcel")
                } else {
                    self.world.parcel_name.as_str()
                };
                let icons = &self.skin.icons;
                let layout = &self.skin.layout;
                let maturity = self
                    .world
                    .main()
                    .and_then(|r| r.info.as_ref())
                    .map(|i| ui::bars::maturity_name(i.sim_access))
                    .unwrap_or("");
                let status = ui::bars::StatusInfo {
                    show_bug_report: if self.demo {
                        ui::bars::demo::get().bug.unwrap_or(self.settings.show_bug_report)
                    } else {
                        self.settings.show_bug_report
                    },
                    show_huds: self.settings.show_huds,
                    hud_zoom: self.world.hud_zoom,
                    ban_lines: self.settings.maps.ban_lines,
                    region: &self.world.region_name(),
                    parcel: parcel_name,
                    maturity,
                    position: self.world.agent.position,
                    fps: self.perf.fps,
                    flying: self.world.agent.flying,
                    seated: self.world.agent.is_sitting(),
                    ping_ms: self.net.stats.ping_ms.load(std::sync::atomic::Ordering::Relaxed),
                    balance: self.world.balance,
                    voice: (self.voice.light, self.voice.status.clone()),
                    slurl: crate::slurl::make(&self.world.region_name(), self.world.agent.position),
                    tp_back: self.world.tp_history.previous().map(|e| e.region.clone()),
                    tp_forward: self.world.tp_history.next().map(|e| e.region.clone()),
                    status: self.world.status.modes(),
                    local_env: self.world.eep.has_local(),
                    double_click: self.double_click_action(),
                    parcel_landmark,
                    marketplace: self.marketplace_home().is_some()
                        || (self.demo && crate::world::marketplace::demo::Mode::from_env().is_some()),
                    parcel_icons: match (self.world.main().and_then(|r| r.info.as_ref()), self.world.parcel.as_deref()) {
                        (Some(region), Some(parcel)) => ui::parcel_icons::parcel_icons(&ui::parcel_icons::ParcelState {
                            region_flags: region.region_flags,
                            parcel,
                            agent_id: self.world.agent_id,
                            group: self.world.groups.group(&parcel.group_id),
                            health: self.world.health,
                        }),
                        _ => Vec::new(),
                    },
                    classic_bars,
                    islands,
                    own_name: &own_name,
                    own_pic: self.avatar_pics.get(&self.world.agent_id),
                    recent: if classic_bars { Vec::new() } else { self.recent_places() },
                    favorites: &favorites,
                };
                let media = ui::audio::MediaState {
                    music_available: self.world.parcel.as_ref().is_some_and(|pc| !pc.music_url.is_empty()),
                    music_playing: self.music_playing,
                    media_available: self.settings.media.enabled
                        && self.world.parcel.as_ref().is_some_and(|pc| !pc.media_url.trim().is_empty()),
                    media_playing: self.media.parcel_playing(),
                };
                let aurora = self.notifications_aurora();
                let bell = ui::bars::BellState {
                    unread: if aurora {
                        ui::notification_center::bell_count(&self.world.notifications, &self.notif_ui.center)
                    } else {
                        self.world.notifications.unread()
                    },
                    aurora,
                    dnd: self.world.status.dnd,
                };
                let (bar, audio_action) = ui::bars::top_bars(
                    ui,
                    &p,
                    icons,
                    layout,
                    &mut self.panels,
                    &status,
                    &mut self.settings.audio,
                    &mut self.audio_ui,
                    media,
                    &mut self.notif_ui,
                    bell,
                );
                a.bar = bar;
                a.audio = Some(audio_action);
                // AURORA_DEMO_BARS: the mockup's voice (connected; talking
                // with `voice`) and chat island width, for this run only and
                // with the islands (Firestorm's bars stay as they were)
                let demo_bars = if self.demo && !classic_bars {
                    ui::bars::demo::get()
                } else {
                    Default::default()
                };
                let mut demo_chat_width = demo_bars.chat_width;
                let mut mic = ui::bars::MicButton {
                    on: self.mic_on,
                    hold: self.settings.audio.mic_hold,
                    talking: self.talking() || demo_bars.voice,
                    level: if demo_bars.voice { 0.62 } else { self.voice_level },
                    connected: self.voice_connected || demo_bars.on,
                    key: self
                        .settings
                        .keybinds
                        .get(crate::keybinds::Action::PushToTalk)
                        .into_iter()
                        .flatten()
                        .next(),
                    input_device: self.settings.audio.input_device.clone(),
                    output_device: self.settings.audio.output_device.clone(),
                    input_devices: &self.options_ui.input_devices,
                    output_devices: &self.options_ui.output_devices,
                    voice_dot: (!classic_bars).then(|| {
                        if demo_bars.on {
                            (crate::voice::VoiceLight::Connected, String::new())
                        } else {
                            (self.voice.light, self.voice.status.clone())
                        }
                    }),
                    ..Default::default()
                };
                self.toolbar_ui.favorites_access = self.favorites_access();
                let toolbar_migrated = self.settings.migrate_toolbar(self.world.agent_id);
                let mut demo_toolbar = None;
                if self.demo && ui::bars::demo::get().access == Some(crate::settings::FavoritesAccess::Dock) {
                    let layout = self.dock_layout.of(layout).clone();
                    let mut t = self.settings.toolbar.clone().unwrap_or_default();
                    let mut buttons = t.buttons(&layout).to_vec();
                    if !buttons.iter().any(|b| b.command == "favorites") {
                        let at = buttons.iter().position(|b| b.command == "places").map_or(buttons.len(), |i| i + 1);
                        buttons.insert(
                            at,
                            ui::skin::ToolButton {
                                icon: "bookmarks-filled".into(),
                                command: "favorites".into(),
                                tooltip: String::new(),
                            },
                        );
                    }
                    t.buttons = Some(buttons);
                    demo_toolbar = Some(t);
                }
                let toolbar_settings = match demo_toolbar.as_mut() {
                    Some(t) => t,
                    None => self.settings.toolbar.get_or_insert_default(),
                };
                // the dock leaves the Conversations button out of its default
                let bar_layout = if classic_bars { layout } else { self.dock_layout.of(layout) };
                let (chat, bar2, bottom_top, chat_width_set) = ui::bars::bottom_bar(
                    ui,
                    &p,
                    icons,
                    bar_layout,
                    &mut self.toolbar_ui,
                    toolbar_settings,
                    &mut self.panels,
                    &mut self.chat_ui,
                    &self.gesture_state.manager,
                    if demo_bars.read { 0 } else { self.world.conversations_badge() },
                    self.world.social.latest_unread(),
                    self.world.agent.flying,
                    self.world.agent.is_sitting(),
                    self.camera.mouselook(),
                    match demo_chat_width.as_mut() {
                        Some(width) => width,
                        None if classic_bars => &mut self.settings.chat_bar_width,
                        None => &mut self.settings.chat_island_width,
                    },
                    &mut mic,
                    self.settings.ao.accounts.get(&self.world.agent_id).is_some_and(|a| a.enabled) && !self.ao_runtime.paused,
                    !classic_bars,
                    Some(&mut self.emoji),
                    &favorites,
                );
                // what the bars leave: where the windows are laid out, moved
                // and snapped this frame (above the dock with the islands)
                let left = ui.available_rect_before_wrap();
                let area = if classic_bars {
                    left
                } else {
                    let mut area = ui::bars::islands::work_area(left, bottom_top);
                    if let Some(row) = ui::bars::islands::favorites_row_bottom(ui.ctx()) {
                        area.min.y = area.min.y.max(row + ui::bars::islands::MARGIN);
                    }
                    area
                };
                ui::layout::set_work_area(ui.ctx(), area);
                ui::layout::frame(ui.ctx(), &p, self.settings.snap_windows);
                self.toolbar_ui.show(ui.ctx(), &p, bar_layout, toolbar_settings);
                if self.toolbar_ui.commit(bar_layout, toolbar_settings) || chat_width_set || toolbar_migrated {
                    self.settings.save();
                }
                // the Favorites button's right click: where the favourites live
                if let Some(access) = self.toolbar_ui.access_choice.take() {
                    self.settings.favorites_access = access;
                    self.settings.save();
                }
                self.mic_on = mic.on;
                self.mic_button_held = mic.held;
                if mic.mode_changed || mic.devices_changed {
                    // the audio engine and the voice follow the settings
                    self.settings.audio.mic_hold = mic.hold;
                    self.settings.audio.input_device = std::mem::take(&mut mic.input_device);
                    self.settings.audio.output_device = std::mem::take(&mut mic.output_device);
                    self.settings.save();
                }
                let (open_prefs, list_devices) = (mic.open_prefs, mic.list_devices);
                if list_devices {
                    self.options_ui.output_devices = aurora_audio::output_devices();
                    self.options_ui.input_devices = aurora_audio::input_devices();
                }
                if open_prefs {
                    self.panels.settings = true;
                    self.options_ui.tab = ui::options::TAB_AUDIO;
                }
                a.chat = chat;
                if !matches!(bar2, BarAction::None) {
                    a.bar = bar2;
                }
                t = self.ui_parts.lap(Part::UiBars, t);
                // overlays
                self.draw_hud_scale(&ctx);
                {
                    let mods = crate::build::Mods {
                        shift: self.shift,
                        ctrl: self.ctrl,
                        alt: self.alt,
                    };
                    crate::build::ui::shortcuts(&ctx, &mut self.build, &mut self.world, &mut self.settings.build);
                    self.build
                        .draw_overlay(&ctx, &self.palette, &self.world, &self.scene, &self.settings.build, mods);
                    let mut env = crate::build::ui::Env {
                        images: &self.ui_images,
                        media: &mut self.media,
                        scene: &mut self.scene,
                        zoom_fraction: self.camera.zoom_fraction(),
                        demo: self.demo,
                    };
                    if crate::build::ui::show(&ctx, &p, &mut self.build, &mut self.world, &mut self.settings.build, mods, &mut env) {
                        self.settings.save();
                    }
                }
                ui::debug_overlay::draw(
                    &ctx,
                    &self.world,
                    &self.scene,
                    projector,
                    self.camera.position,
                    &self.settings.debug,
                    self.settings.debug.probes.then(|| self.probes.debug_list()),
                    &self.lod_infos,
                );
                if self.camera.mouselook() {
                    let c = ctx.content_rect().center();
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("xhair")));
                    painter.circle_stroke(c, 4.0, egui::Stroke::new(1.5, p.ink));
                    if self.settings.camera.show_mouselook_instructions {
                        // LLViewerWindow::drawMouselookInstructions: the way
                        // out, half transparent above the bottom bar (the
                        // hidden cursor cannot reach its button)
                        let key = self.settings.keybinds.get(crate::keybinds::Action::ResetCamera)[0]
                            .as_ref()
                            .map_or_else(|| aurora_i18n::tr!("keybinds-esc").to_string(), |b| b.label());
                        let text = aurora_i18n::tr!("app-mouselook-leave", key = key);
                        let at = egui::pos2(c.x, bottom_top - 16.0);
                        let font = egui::FontId::proportional(18.0);
                        let shadow = at + egui::vec2(1.0, 1.0);
                        painter.text(shadow, egui::Align2::CENTER_BOTTOM, &text, font.clone(), p.navy.gamma_multiply(0.6));
                        painter.text(at, egui::Align2::CENTER_BOTTOM, &text, font, p.ink.gamma_multiply(0.6));
                    }
                }
                if self.skin.layout.chat_toasts && !(self.panels.chat && ui::chat::local_chat_shown(&ctx, &self.chat_ui)) {
                    ui::chat::toasts(
                        &ctx,
                        &p,
                        &self.world,
                        &self.avatar_pics,
                        &mut self.chat_ui.wanted_names,
                        &mut self.chat_ui.wanted_pics,
                        bottom_top,
                        self.settings.chat_toast_seconds,
                        self.settings.chat_timestamps,
                        !classic_bars,
                    );
                }
                t = self.ui_parts.lap(Part::UiOverlays, t);
                let map_cam = {
                    let r = ctx.content_rect();
                    let aspect = (r.width() / r.height().max(1.0)).max(0.1);
                    ui::minimap::MapCamera {
                        pos: self.camera.position,
                        dir: self.camera.target - self.camera.position,
                        hfov: 2.0 * ((self.camera.fov_y * 0.5).tan() * aspect).atan(),
                        far: self.settings.draw_distance,
                    }
                };
                let mut mini = Vec::new();
                let mut open = self.panels.chat;
                let mut contact_actions = Vec::new();
                self.chat_ui.style = self.settings.chat_style;
                self.chat_ui.voice_on = self.settings.audio.voice_enabled;
                self.chat_ui.mic = (!self.settings.audio.mic_hold).then_some(self.mic_on);
                if self.world.calls.active.is_some() {
                    self.chat_ui.volumes.clone_from(&self.world.calls.volumes);
                }
                for c in ui::chat::show(
                    &ctx,
                    &p,
                    icons,
                    &mut self.emoji,
                    &self.avatar_pics,
                    &self.ui_images,
                    &mut self.world,
                    &self.gesture_state.manager,
                    &mut self.chat_ui,
                    &mut self.contacts_ui,
                    &mut self.settings.contacts,
                    &mut open,
                ) {
                    match c {
                        ui::chat::ConvAction::Local(out) => a.chat = Some(out),
                        ui::chat::ConvAction::Im { to, text } => a.ims.push((to, text)),
                        ui::chat::ConvAction::OfferTeleport(id) => a.offer_tp.push(id),
                        ui::chat::ConvAction::ToggleBlock(id) => a.ctx_action = Some(ui::context::CtxAction::ToggleBlock(id)),
                        ui::chat::ConvAction::Close(id) => {
                            self.world.conversation_closed(&id);
                            self.world.leave_session(id);
                            self.chat_ui.forget(id);
                            // closed from the list, another conversation may be shown
                            if self.chat_ui.selected == Some(id) {
                                self.chat_ui.selected = None;
                                self.chat_ui.show_profile = false;
                            }
                        }
                        ui::chat::ConvAction::Call(id) => a.ctx_action = Some(ui::context::CtxAction::Call(id)),
                        ui::chat::ConvAction::HangUp => self.world.end_call(),
                        ui::chat::ConvAction::ToggleMic => self.mic_on = !self.mic_on,
                        ui::chat::ConvAction::AnswerCall { session, answer } => self.world.answer_call_invite(session, answer),
                        ui::chat::ConvAction::PickPeople(conversation) => self.call_invite_ui.pick_for(conversation),
                        ui::chat::ConvAction::AddPeople(conversation, ids) => self.call_invite_ui.add(&mut self.world, conversation, ids),
                        ui::chat::ConvAction::SetVolume(id, v) => {
                            self.world.calls.set_volume(id, v);
                            // saved once the slider rests
                            self.volumes_save_at = Some(Instant::now() + std::time::Duration::from_millis(500));
                        }
                        ui::chat::ConvAction::BlockGroupChat(id) => {
                            self.world.conversation_closed(&id);
                            self.world.set_group_chat_blocked(id, true);
                            self.chat_ui.forget(id);
                            self.chat_ui.selected = None;
                        }
                        ui::chat::ConvAction::OpenPeople(tab) => {
                            self.panels.people = true;
                            self.panels.people_tab = tab;
                        }
                        ui::chat::ConvAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
                        ui::chat::ConvAction::Contacts(c) => contact_actions.push(c),
                        ui::chat::ConvAction::OpenContacts => self.panels.contacts = true,
                    }
                }
                self.panels.chat = open;
                if let Some(ui::chat::history::HistoryAction::OpenFolder(dir)) = self.history_ui.show(
                    &ctx,
                    &p,
                    icons,
                    &mut self.emoji,
                    &self.avatar_pics,
                    &self.world,
                    &mut self.chat_ui.wanted_names,
                ) && let Err(e) = crate::bug_report::files::open_directory(&dir)
                {
                    log::warn!("cannot open the transcripts folder: {e}");
                }
                if self.settings.contacts.torn_off {
                    let mut open = self.panels.contacts;
                    let pics = ui::contacts::Pics {
                        avatars: &self.avatar_pics,
                        images: &self.ui_images,
                    };
                    contact_actions.extend(ui::contacts::window(
                        &ctx,
                        &p,
                        &mut self.world,
                        &mut self.contacts_ui,
                        &mut self.settings.contacts,
                        &pics,
                        &mut open,
                    ));
                    self.panels.contacts = open;
                }
                // their confirmations, also asked from the other menus
                contact_actions.extend(ui::contacts::dialog_windows(
                    &ctx,
                    &p,
                    &mut self.world,
                    &mut self.contacts_ui,
                    &mut self.settings.contacts,
                ));
                // residents added to a conversation: a conversation with one
                // resident that became a conference closes (FSFloaterIM::
                // addP2PSessionParticipants)
                if let Some(closed) = self.call_invite_ui.show(&ctx, &p, &mut self.world) {
                    self.world.leave_session(closed);
                    self.chat_ui.forget(closed);
                }
                for c in contact_actions {
                    self.on_contacts_action(c, &mut a);
                }
                t = self.ui_parts.lap(Part::UiChat, t);
                let mut people_map = |ui: &mut egui::Ui, size: egui::Vec2| {
                    let acts = self.people_map_ui.draw(
                        ui,
                        &p,
                        &self.world,
                        &mut self.map_tiles,
                        &map_cam,
                        &mut self.settings.maps,
                        size,
                        true,
                    );
                    mini.extend(acts);
                };
                let people_before = (self.settings.radar.clone(), self.settings.friends_list.clone());
                let mut radar = ui::people::RadarData {
                    radar: &self.radar,
                    settings: &mut self.settings.radar,
                    voice: &voice_levels,
                    draw_distance: self.settings.draw_distance,
                    now: self.radar_clock.elapsed().as_secs_f64(),
                    pics: &self.avatar_pics,
                };
                let people = ui::people::people_window(
                    &ctx,
                    &p,
                    &self.world,
                    &mut self.panels,
                    &mut self.people_ui,
                    &mut radar,
                    &mut self.settings.friends_list,
                    ui::people::ContactsParts {
                        ui: &mut self.contacts_ui,
                        settings: &mut self.settings.contacts,
                        images: &self.ui_images,
                    },
                    &mut people_map,
                    !self.settings.classic_people && !(self.demo && crate::demo::people::legacy()),
                );
                if people_before != (self.settings.radar.clone(), self.settings.friends_list.clone()) {
                    self.settings.save();
                }
                for pa in people {
                    match pa {
                        ui::people::PeopleAction::OpenIm(id) => {
                            // LLAvatarActions::startIM: they join the recent people
                            self.world.met(id);
                            self.world.ui_sounds.push(UiSound::StartIm);
                            self.world.social.session_mut(id);
                            self.world.social.focus_im = Some(id);
                            self.panels.chat = true;
                        }
                        ui::people::PeopleAction::OfferTeleport(id) => a.offer_tp.push(id),
                        ui::people::PeopleAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
                        ui::people::PeopleAction::TooFarToZoom => {
                            self.world.system_message(aurora_i18n::tr!("radar-too-far-to-zoom"));
                        }
                        ui::people::PeopleAction::GroupChat(id) => {
                            self.world.ui_sounds.push(UiSound::StartIm);
                            self.world.start_group_chat(id);
                            self.panels.chat = true;
                        }
                        ui::people::PeopleAction::GroupNotices(id, on) => self.world.groups.set_accept_notices(id, on),
                        ui::people::PeopleAction::ModifyRights { friend, rights, grant } => {
                            self.contacts_ui.ask_modify_rights(friend, rights, grant)
                        }
                        ui::people::PeopleAction::PickFriend => self.contacts_ui.pick_new_friend(),
                        ui::people::PeopleAction::Unblock(id, name) => self.world.unblock(id, &name, 0),
                        ui::people::PeopleAction::BlockByName(name) => {
                            if let Err(e) = self.world.block(uuid::Uuid::nil(), &name, crate::world::mutes::MuteType::ByName) {
                                self.world.system_message(e);
                            }
                        }
                        ui::people::PeopleAction::BlockFlag { id, name, kind, flag, on } => {
                            if on {
                                if let Err(e) = self.world.block_flags(id, &name, kind, flag) {
                                    self.world.system_message(e);
                                }
                            } else {
                                self.world.unblock(id, &name, flag);
                            }
                        }
                        ui::people::PeopleAction::SearchGroups => {
                            self.panels.search = true;
                            self.search_ui.open_tab(crate::world::search::Tab::Groups);
                        }
                        ui::people::PeopleAction::GroupData(group) => self.world.group_data.open(group, true),
                        ui::people::PeopleAction::GroupTitle { group, role } => self.world.group_data.set_title(group, role),
                        ui::people::PeopleAction::GroupFlags { group, notices, profile } => {
                            self.world.groups.set_flags(group, notices, profile)
                        }
                        ui::people::PeopleAction::InviteToGroup(group) => self.group_profile_ui.open_with_invite(&mut self.world, group),
                    }
                }
                t = self.ui_parts.lap(Part::UiPeople, t);
                self.poll_inventory_animation_stats();
                self.show_imports(&ctx, &p);
                let inventory_preferences = self.settings.inventory.clone();
                let mut open = self.panels.inventory;
                if !self.inventory_ui.merchant_requested {
                    self.inventory_ui.merchant_requested = true;
                    self.send(NetCommand::RequestInventoryMerchant);
                }
                for ia in ui::inventory::show(
                    &ctx,
                    &p,
                    &self.skin.icons,
                    &mut self.world,
                    &mut self.inventory_ui,
                    &mut self.settings.inventory,
                    &mut open,
                    self.appearance_ui.pending.is_some(),
                    &self.ui_images,
                ) {
                    match ia {
                        ui::inventory::InvAction::TeleportLandmark(asset) => a.landmark = Some(asset),
                        other => self.apply_inventory_action(other),
                    }
                }
                self.panels.inventory = open;
                self.show_gestures(&ctx, &p);
                if self.settings.inventory != inventory_preferences {
                    self.settings.save();
                }
                if self.panels.appearance {
                    let worn = self.world.worn_attachment_items();
                    for command in self.appearance_ui.prepare(&mut self.world.inventory, self.world.agent_id, &worn) {
                        self.send(command);
                    }
                }
                let complexity = self.scene.avatar_complexity.get(&self.world.agent_id).copied().unwrap_or_default();
                let mut open = self.panels.appearance;
                for action in ui::appearance::show(
                    &ctx,
                    &p,
                    &mut self.world,
                    &mut self.appearance_ui,
                    &mut open,
                    complexity,
                    &self.ui_images,
                ) {
                    self.apply_appearance_action(action);
                }
                self.panels.appearance = open;
                self.update_demo_shape_mesh();
                self.shape_ui.update(&mut self.world, &self.scene, &ctx);
                let shape_preferences = (self.shape_ui.hints, self.shape_ui.feet);
                let shape_group = self.shape_ui.group;
                let (shape_actions, visible) = ui::shape::show(&ctx, &p, &mut self.world, &mut self.shape_ui);
                self.demo_shape_drag();
                if shape_preferences != (self.shape_ui.hints, self.shape_ui.feet) {
                    self.settings.shape_show_hints = self.shape_ui.hints;
                    self.settings.shape_height_feet = self.shape_ui.feet;
                    self.settings.save();
                }
                self.shape_ui.request_hints(&self.world, &self.scene.jobs, &ctx, &visible);
                if self.shape_ui.open && self.shape_camera.is_some() && (self.shape_ui.group != shape_group || self.shape_ui.camera_pending)
                {
                    self.focus_shape_category();
                }
                for action in shape_actions {
                    self.shape_action(action);
                }
                if !self.shape_ui.open
                    && let Some(camera) = self.shape_camera.take()
                {
                    self.camera = camera;
                }
                let mut open = self.panels.ao;
                let mut account = self.settings.ao.accounts.get(&self.world.agent_id).cloned().unwrap_or_default();
                let actions = ui::ao::show(
                    &ctx,
                    &p,
                    &mut self.world.inventory,
                    &self.ao_catalog,
                    &mut self.ao_runtime,
                    &mut account,
                    &mut self.ao_ui,
                    &mut open,
                );
                self.panels.ao = open;
                if actions.iter().any(|a| matches!(a, ui::ao::Action::Preferences)) {
                    self.settings.ao.accounts.insert(self.world.agent_id, account);
                }
                for action in actions {
                    self.apply_ao_action(action);
                }
                t = self.ui_parts.lap(Part::UiInventory, t);
                let facts = self.context_facts();
                let had_menu = self.context_menu.is_some();
                // Capture-only style override; leave the saved preference intact.
                let use_pie = if self.demo {
                    match std::env::var("AURORA_DEMO_PIE").as_deref() {
                        Ok("1") => true,
                        Ok("0") => false,
                        _ => self.settings.use_pie_menu,
                    }
                } else {
                    self.settings.use_pie_menu
                };
                ui::context::show(&ctx, &p, &mut self.context_menu, &facts, use_pie);
                if had_menu && self.context_menu.is_none() {
                    // LLViewerMenuHolderGL::hideMenus drops the menu's selection
                    self.build.release_menu_selection(&self.world);
                    self.flush_build();
                }
                if let Some((id, text)) = self.delete_confirm.clone()
                    && let Some(yes) = ui::context::delete_confirm(&ctx, &p, &text)
                {
                    self.delete_confirm = None;
                    if yes {
                        ui::context::request(&ctx, ui::context::CtxAction::DeleteConfirmed(id));
                    }
                }
                if let Some((region, _, dont_ask)) = self.place_confirm.as_mut()
                    && let Some(yes) = ui::context::teleport_confirm(
                        &ctx,
                        &p,
                        "place_teleport_confirm",
                        &ui::context::place_teleport_question(region),
                        dont_ask,
                    )
                    && let Some((region, pos, dont_ask)) = self.place_confirm.take()
                    && yes
                {
                    if dont_ask {
                        self.stop_confirming_teleports();
                    }
                    ui::context::request(&ctx, ui::context::CtxAction::TeleportToPlaceConfirmed(region, pos));
                }
                if let Some((url, dont_warn, danger)) = self.url_confirm.as_mut()
                    && let Some(yes) = ui::context::external_link_confirm(&ctx, &p, url, dont_warn, *danger)
                {
                    let (url, dont_warn) = (url.clone(), *dont_warn);
                    self.url_confirm = None;
                    if yes {
                        if dont_warn {
                            self.settings.warn_external_links = false;
                            self.settings.save();
                        }
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                }
                ui::media::show(
                    &ctx,
                    &p,
                    &self.skin.icons,
                    &mut self.media,
                    &mut self.media_ui,
                    // under the top bar or the islands
                    ui::layout::work_area(&ctx).top() - ctx.content_rect().top(),
                );
                if let Some(c) = self.media_cursor {
                    ctx.set_cursor_icon(c);
                }
                self.show_pay(&ctx, &p, &mut a);
                for document in ui::object_actions::show_contents(&ctx, &p, &self.skin.icons, &mut self.interactions, &self.world) {
                    match document {
                        ui::object_actions::ContentsOpen::Script(object, item) => {
                            self.open_script(aurora_net::scripts::ScriptTarget { object, item })
                        }
                        ui::object_actions::ContentsOpen::Notecard(object, item) => self.open_task_notecard(object, item),
                    }
                }
                self.show_task_notecards(&ctx, &p);
                self.show_scripts(&ctx, &p);
                if let Some(action) = ui::inventory::task_drop::confirm(&ctx, &p, &mut self.task_transfer) {
                    self.apply_inventory_action(action);
                }
                // a teleport invitation whose card is in sight needs no toast
                // (LLOfferHandler: a toast only when the IM floater is closed)
                if self.panels.chat
                    && let Some(open) = self.chat_ui.selected
                {
                    let seen: Vec<u64> = self
                        .world
                        .teleport_invites
                        .pending_notifications()
                        .filter(|(_, from)| *from == open)
                        .map(|(n, _)| n)
                        .collect();
                    for n in self.world.notifications.list.iter_mut().filter(|n| seen.contains(&n.id)) {
                        n.read = true;
                    }
                }
                // nothing over the view in mouselook or the photo's full frame;
                // Firestorm's toasts wait in the bell in do not disturb too
                let aurora = self.notifications_aurora();
                let dnd = self.world.status.dnd;
                let toasts_hidden = self.camera.mouselook() || self.photo_full_frame() || (dnd && !aurora);
                self.world.notifications.aurora = aurora;
                let restart = self.restart_bell_info(&p);
                let mut toast_env = ui::notifications::ToastEnv {
                    pics: &self.avatar_pics,
                    emoji: &mut self.emoji,
                    hidden: toasts_hidden,
                    message_seconds: f64::from(self.settings.im_toast_seconds),
                    restart,
                };
                let (answers, asks) = if aurora {
                    ui::notification_center::show(
                        &ctx,
                        &p,
                        &self.skin.icons,
                        &self.world.map.ratings,
                        &mut self.world.notifications,
                        &mut self.world.notification_history,
                        &mut self.notif_ui,
                        &mut toast_env,
                        dnd,
                    )
                } else {
                    ui::notifications::show(
                        &ctx,
                        &p,
                        &self.skin.icons,
                        &self.world.map.ratings,
                        &mut self.world.notifications,
                        &mut self.world.notification_history,
                        &mut self.notif_ui,
                        &mut toast_env,
                    )
                };
                // the region restart over the toasts, under the open list
                self.show_region_restart(&ctx, &p);
                self.order_region_restart(&ctx, aurora);
                for ask in asks {
                    self.on_notification_ask(&ctx, ask);
                }
                for (id, r) in answers {
                    if self.region_restart_bell_answer(id, &r) {
                        continue;
                    }
                    if matches!(r, crate::world::notifications::Response::Open) {
                        let presence = self
                            .world
                            .notifications
                            .list
                            .iter()
                            .any(|n| n.id == id && matches!(n.data, crate::world::notifications::Data::Presence { .. }));
                        if self.world.open_from_notification(id).is_some() {
                            if presence {
                                self.world.ui_sounds.push(UiSound::StartIm);
                            }
                            ui::notifications::open_conversations(&ctx, &mut self.panels.chat);
                        }
                        continue;
                    }
                    let (cmds, url) = self.world.respond_notification(id, r);
                    if cmds.iter().any(|c| matches!(c, NetCommand::AcceptLure { .. })) {
                        self.begin_teleport(String::new(), false);
                    }
                    for c in cmds {
                        self.send(c);
                    }
                    // only web links (llLoadURL / goto URL), opened after the user's click
                    if let Some(u) = url.filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
                        ctx.open_url(egui::OpenUrl::new_tab(u));
                    }
                }
                self.world.record_notification_history();
                t = self.ui_parts.lap(Part::UiMenus, t);
                let mut open = self.panels.about_land;
                // the mini-map's image of the selection's region, for the header
                let land_map = open
                    .then(|| self.world.land.sel.as_ref().map(|s| s.handle).or(self.world.main_region))
                    .flatten()
                    .and_then(|h| self.minimap_ui.region_image(&ctx, &self.world, &mut self.map_tiles, h));
                // AURORA_DEMO_LAND=…:classic shows Firestorm's tabs without saving it
                let classic_land =
                    self.settings.classic_about_land || (self.demo && crate::demo::land::scenario().is_some_and(|s| s.classic));
                if self.land_ui.show(
                    &ctx,
                    &p,
                    &mut self.world,
                    &self.ui_images,
                    &self.avatar_pics,
                    land_map.as_ref(),
                    &mut self.settings.audio.saved_streams,
                    self.demo,
                    classic_land,
                    &mut open,
                ) {
                    self.settings.save();
                }
                self.panels.about_land = open;
                // Region / Estate: asked by About Land or the restart notice
                if ui::region::take_open_request(&ctx) {
                    ui::widgets::open_floater(&ctx, ui::region::WINDOW, &mut self.panels.region_estate);
                }
                let mut open = self.panels.region_estate;
                let region_map = open
                    .then_some(self.world.main_region)
                    .flatten()
                    .and_then(|h| self.minimap_ui.region_image(&ctx, &self.world, &mut self.map_tiles, h));
                if self.region_ui.show(
                    &ctx,
                    &p,
                    &mut self.world,
                    &self.avatar_pics,
                    &self.ui_images,
                    region_map.as_ref(),
                    &mut self.settings,
                    &mut open,
                ) {
                    self.settings.save();
                }
                self.panels.region_estate = open;
                match self.region_ui.action.take() {
                    Some(ui::region::RegionAction::AboutLand) => self.panels.about_land = true,
                    Some(ui::region::RegionAction::WorldMap) => ui::widgets::open_floater(&ctx, "worldmap", &mut self.panels.world_map),
                    None => {}
                }
                if std::mem::take(&mut self.land_ui.custom_day) {
                    self.open_parcel_day_editor();
                }
                a.set_display_name = self.display_name_ui.show(&ctx, &p, &self.world);
                for cmd in self.create_landmark_ui.show(&ctx, &p, &self.world.inventory) {
                    self.send(cmd);
                }
                // the experience profile (the permission question's link)
                if let Some((experience, change)) = ui::experience_profile::show(&ctx, &p, &mut self.world) {
                    self.world.experiences.change(experience, change);
                }
                self.photo_ui(&ctx, &p, renderer);
                self.controls_window(&ctx, &p);
                self.report_abuse_ui(&ctx, &p);
                let hover_enabled = self.world.hover_height_available();
                let automatic_base = self.scene.automatic_hover_base(&self.world);
                if self.hover_height_ui.show(
                    &ctx,
                    &p,
                    &mut self.world.hover_height,
                    hover_enabled,
                    ui::hover_height::HoverHeightOptions {
                        camera: &mut self.settings.camera.hover_affects_camera,
                        automatic: &mut self.settings.auto_hover_height,
                        offset: &mut self.settings.auto_hover_offset,
                        automatic_base,
                        neutral_pose_test: &mut self.scene.neutral_pose_test,
                    },
                ) {
                    self.settings.save();
                }
                if self.inspect_ui.open {
                    let (rows, totals) = self.inspect_rows();
                    let mut columns = self.settings.inspect_columns;
                    if let Some(id) = self.inspect_ui.show(&ctx, &p, &rows, &totals, &mut columns) {
                        self.profile_ui.open(&mut self.world, id);
                    }
                    if columns != self.settings.inspect_columns {
                        self.settings.inspect_columns = columns;
                        self.settings.save();
                    }
                    if !self.inspect_ui.open {
                        self.inspect_closed();
                    }
                }
                a.currency = self.buy_currency_ui.show(&ctx, &p, &self.world);
                a.currency.extend(self.show_buy_land(&ctx, &p));
                // links and names clicked anywhere (LLAgentHandler)
                if let Some(id) = ui::profile::take_open_request(&ctx) {
                    self.profile_ui.open(&mut self.world, id);
                }
                // the "Feed" tab: a browser on the grid's web profiles
                let paths = self.media.plugins(&self.settings.media).cloned();
                let browser = crate::media::browser_settings(&crate::settings::cache_dir(), aurora_i18n::language());
                let cookie = self.media.openid.cookie();
                let profile_base = match self.settings.grid {
                    // AURORA_DEMO_FEED=<page>: the feed browser on a local page
                    _ if self.demo => std::env::var("AURORA_DEMO_FEED").ok(),
                    ref grid => crate::snapshot::profile::grid_web_profile(
                        grid,
                        &self
                            .world
                            .login
                            .as_ref()
                            .map(|l| l.raw["web_profile_url"].to_string_value())
                            .unwrap_or_default(),
                    ),
                };
                // World › SL Marketplace: the same plugins, browser settings
                // and OpenID cookie (LLFloaterMarketplace::postBuild)
                let marketplace_home = self.marketplace_home();
                self.marketplace_ui.show(
                    &ctx,
                    &p,
                    &ui::marketplace::MarketWeb {
                        paths: paths.as_ref(),
                        browser: &browser,
                        cookie: cookie.as_ref(),
                        home: marketplace_home.as_deref(),
                    },
                );
                self.show_marketplace_listings(&ctx, &p);
                let web = ui::profile::WebContext {
                    paths: paths.as_ref(),
                    browser: &browser,
                    cookie: cookie.as_ref(),
                    profile_base: profile_base.as_deref(),
                };
                let acts = self.profile_ui.show(
                    &ctx,
                    &p,
                    &mut self.emoji,
                    &self.avatar_pics,
                    &self.ui_images,
                    &mut self.world,
                    &mut self.chat_ui.wanted_names,
                    &web,
                    self.settings.classic_profiles,
                );
                for pa in acts {
                    match pa {
                        ui::profile::ProfileAction::Im(id) => a.ctx_action = Some(ui::context::CtxAction::Im(id)),
                        ui::profile::ProfileAction::OfferTeleport(id) => a.offer_tp.push(id),
                        ui::profile::ProfileAction::ToggleBlock(id) => a.ctx_action = Some(ui::context::CtxAction::ToggleBlock(id)),
                        ui::profile::ProfileAction::DisplayName => self.display_name_ui.open(),
                        ui::profile::ProfileAction::ModifyRights { friend, rights, grant } => {
                            self.contacts_ui.ask_modify_rights(friend, rights, grant)
                        }
                        ui::profile::ProfileAction::GroupChat(id) => {
                            self.world.start_group_chat(id);
                            self.panels.chat = true;
                        }
                        ui::profile::ProfileAction::Save { target, data } => {
                            let cmd = self.world.save_profile(target, data);
                            self.send(cmd);
                        }
                        ui::profile::ProfileAction::Net(cmd) => {
                            self.world.profile_command(&cmd);
                            self.send(cmd);
                        }
                        ui::profile::ProfileAction::ShowOnMap(g) => {
                            self.world.map.track_location(g.x, g.y, g.z as f32, false);
                            self.panels.world_map = true;
                        }
                        ui::profile::ProfileAction::Teleport(g) => self.world.map.track_location(g.x, g.y, g.z as f32, true),
                    }
                }
                // place profiles: standalone windows and "Places"
                if self.panels.places {
                    self.places_ui.landmark_here = self.parcel_landmark().is_some();
                }
                let mut c = ui::place_details::Ctx {
                    p: &p,
                    world: &self.world,
                    images: &self.ui_images,
                    emoji: &mut self.emoji,
                    want_names: &mut self.chat_ui.wanted_names,
                    wanted_images: &mut self.place_images,
                    want_groups: &mut self.place_groups,
                };
                let window_acts = self.place_ui.show(&ctx, &mut c);
                let mut open = self.panels.places;
                let places_acts = self.places_ui.show(
                    &ctx,
                    &mut c,
                    &mut open,
                    self.settings.landmarks_by_date,
                    self.settings.confirm_teleports,
                );
                self.panels.places = open;
                for pa in window_acts {
                    match pa {
                        // FSFloaterPlaceDetails::onTeleportButtonClicked: no
                        // confirmation for a place (teleportViaLocation +
                        // trackLocation), TeleportFromLandmark for a landmark
                        ui::place_details::PlaceAction::Teleport(g) => self.world.map.track_location(g.x, g.y, g.z as f32, true),
                        ui::place_details::PlaceAction::TeleportLandmark { asset, name } => self.places_ui.confirm_landmark(asset, name),
                        ui::place_details::PlaceAction::ShowOnMap(g) => {
                            self.world.map.track_location(g.x, g.y, g.z as f32, false);
                            self.panels.world_map = true;
                        }
                        ui::place_details::PlaceAction::Close(serial) => self.world.place_details.close_window(serial),
                    }
                }
                for pa in places_acts {
                    use ui::places::PlacesAction;
                    match pa {
                        PlacesAction::Teleport(g) => self.world.map.track_location(g.x, g.y, g.z as f32, true),
                        PlacesAction::TeleportLandmark(asset) => a.landmark = Some(asset),
                        PlacesAction::CreateLandmark(dest) => self.open_create_landmark(dest),
                        PlacesAction::ShowOnMap(g) => {
                            self.world.map.track_location(g.x, g.y, g.z as f32, false);
                            self.panels.world_map = true;
                        }
                        PlacesAction::OpenProfile(source) => {
                            self.world
                                .place_details
                                .open_panel(source, &self.world.map, &mut self.world.landmarks, Instant::now())
                        }
                        PlacesAction::CloseProfile => self.world.place_details.close_panel(),
                        PlacesAction::RemoveHistory(i) => self.world.tp_storage.remove(i),
                        PlacesAction::ClearHistory => {
                            // order matters: the back / forward list first
                            self.world.tp_history.purge();
                            self.world.tp_storage.clear();
                        }
                        PlacesAction::SortByDate(on) => {
                            self.settings.landmarks_by_date = on;
                            self.settings.save();
                        }
                        PlacesAction::StopConfirmingTeleports => self.stop_confirming_teleports(),
                    }
                }
                // "Search"
                let search_home = self.panels.search.then(|| self.search_home()).flatten();
                let search_session = self.search_session();
                // the session's cookie only goes with a page opened with the
                // session (main grid; never in demo)
                let search_identified = search_home.as_ref().is_some_and(|h| h.identified);
                let search_anonymous = !self.demo && self.search_grid() == crate::world::search::SearchGrid::Beta;
                let mut c = ui::place_details::Ctx {
                    p: &p,
                    world: &self.world,
                    images: &self.ui_images,
                    emoji: &mut self.emoji,
                    want_names: &mut self.chat_ui.wanted_names,
                    wanted_images: &mut self.place_images,
                    want_groups: &mut self.place_groups,
                };
                let search_web = ui::search::WebEnv {
                    paths: paths.as_ref(),
                    browser: &browser,
                    cookie: cookie.as_ref().filter(|_| search_identified),
                    home: search_home.map(|h| h.url),
                    session: search_session,
                    anonymous: search_anonymous,
                };
                let mut open = self.panels.search;
                let search_acts = self
                    .search_ui
                    .show(&ctx, &mut c, &self.avatar_pics, &mut self.settings.search, &search_web, &mut open);
                // the profile of a group, whose name was clicked anywhere
                // (LLGroupActions::show)
                let group_env = ui::group_profile::Env {
                    avatar_pics: &self.avatar_pics,
                    classic: self.settings.classic_profiles || (self.demo && crate::demo::group::create_classic()),
                    now: ctx.input(|i| i.time),
                    group_calls: self.settings.audio.voice_enabled,
                };
                let group_acts = self.group_profile_ui.show(&ctx, &mut c, &group_env);
                self.panels.search = open;
                self.apply_search_actions(&ctx, search_acts);
                self.apply_group_actions(&ctx, group_acts);
                let wanted = self.group_profile_ui.dialogs(&ctx, &p, &mut self.world, &self.ui_images);
                self.place_images.extend(wanted);
                if let Some(id) = ui::group_profile::take_open_request(&ctx) {
                    self.group_profile_ui.open(&mut self.world, id);
                }
                if ui::group_profile::take_create_request(&ctx) {
                    self.group_profile_ui.create.open(&self.world);
                }
                t = self.ui_parts.lap(Part::UiPlaces, t);
                let mut open = self.panels.minimap;
                mini.extend(self.minimap_ui.window(
                    &ctx,
                    &p,
                    &self.world,
                    &mut self.map_tiles,
                    &map_cam,
                    &mut self.settings.maps,
                    &mut open,
                ));
                self.panels.minimap = open;
                for ma in mini {
                    match ma {
                        ui::minimap::MiniMapAction::Teleport { x, y, z } => self.world.map.track_location(x, y, z, true),
                        ui::minimap::MiniMapAction::Track { x, y, z } => self.world.map.track_location(x, y, z, false),
                        ui::minimap::MiniMapAction::StopTracking => self.world.map.clear_track(),
                        ui::minimap::MiniMapAction::OpenWorldMap => self.panels.world_map = true,
                        ui::minimap::MiniMapAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
                        ui::minimap::MiniMapAction::AboutLand(gx, gy) => {
                            ui::land::LandUi::select_at_global(&mut self.world, gx, gy);
                            self.panels.about_land = true;
                        }
                        ui::minimap::MiniMapAction::Colors(k) => self.settings.colors = k,
                        ui::minimap::MiniMapAction::Close => self.panels.minimap = false,
                    }
                }
                let mut open = self.panels.world_map;
                let pictures = ui::worldmap::MapPictures {
                    images: &self.ui_images,
                    avatars: &self.avatar_pics,
                };
                let wm = self.worldmap_ui.show(
                    &ctx,
                    &p,
                    &mut self.world,
                    &mut self.map_tiles,
                    &map_cam,
                    &mut self.settings.maps,
                    self.settings.world_map_style,
                    &pictures,
                    &mut open,
                );
                self.panels.world_map = open;
                for wa in wm {
                    match wa {
                        ui::worldmap::WorldMapAction::TeleportHome => {
                            a.bar = BarAction::TeleportHome;
                            self.panels.world_map = false;
                        }
                        ui::worldmap::WorldMapAction::TeleportLandmark(asset) => a.landmark = Some(asset),
                        ui::worldmap::WorldMapAction::OpenPlace(source) => self.show_place_profile(source),
                    }
                }
                t = self.ui_parts.lap(Part::UiMaps, t);
                let mut open = self.panels.perf;
                let net = self.net.stats.snapshot();
                let view = ui::perf::PerfView {
                    data: &self.perf,
                    render: &self.last_render,
                    gpu: &renderer.info,
                    net: &net,
                    link: (&self.net_profile.last, &self.net_profile.totals),
                    scene: &self.scene.stats,
                    tex: &self.scene.textures.stats,
                    meshes_fetching: self.scene.meshes.fetching,
                    regions: self.world.regions.len(),
                    draw_distance: self.settings.draw_distance,
                    probes: self.probes.counts(),
                    sounds: self.scene.sounds.voices(),
                    complexity: (
                        self.scene.avatar_complexity.get(&self.world.agent_id).copied(),
                        self.scene.too_complex.len(),
                    ),
                };
                ui::perf::show(&ctx, &p, &view, &mut open);
                self.panels.perf = open;
                t = self.ui_parts.lap(Part::UiPerf, t);
                // environment selector (Firestorm's quick preferences lists)
                if self.panels.environment {
                    if !self.env_scan {
                        self.env_scan = true;
                        self.env_ui.scan_left = usize::MAX;
                    }
                    self.env_ui.refresh(&self.world.inventory);
                }
                let mut open = self.panels.environment;
                let mut lighting = self.panels.personal_lighting;
                let env_actions = self.env_ui.show_selector(
                    &ctx,
                    &p,
                    self.world.eep.local_view(),
                    self.world.eep.local_loading(),
                    self.panels.time_of_day,
                    &mut open,
                    &mut lighting,
                );
                self.panels.environment = open;
                // captureCurrentEnvironment: on opening, and again when another
                // local change left no fixed sky to edit (onEnvironmentUpdated)
                if lighting && (!self.lighting_was_open || self.world.eep.personal_sky().is_none()) {
                    self.world.eep.capture_personal();
                }
                self.env_ui.beacons = [self.settings.sun_beacon, self.settings.moon_beacon];
                let reset = self.env_ui.show_lighting(&ctx, &p, &mut self.world.eep, &mut lighting);
                if lighting {
                    self.set_env_beacons(self.env_ui.beacons);
                }
                self.lighting_was_open = lighting;
                self.panels.personal_lighting = lighting;
                for action in env_actions.into_iter().chain(reset) {
                    match action {
                        ui::environment::EnvAction::Pick(kind, asset) => self.world.eep.request_local(asset, kind),
                        ui::environment::EnvAction::Shared => {
                            self.world.eep.clear_local();
                            self.panels.time_of_day = 0;
                        }
                        ui::environment::EnvAction::Preset(n) => self.panels.time_of_day = n,
                    }
                }
                self.draw_env_editor(&ctx, &p);
                self.draw_day_editor(&ctx, &p);
                self.draw_options(&ctx, &p, &mut a);
                self.settings.show_chat = self.panels.chat;
                self.settings.show_perf = self.panels.perf;
                self.settings.show_minimap = self.panels.minimap;
                self.settings.show_people = self.panels.people;
                self.settings.people_tab = self.panels.people_tab;
                self.settings.show_inventory = self.panels.inventory;
                self.settings.show_appearance = self.panels.appearance;
                self.settings.show_ao = self.panels.ao;
                // (not the photo's time of day, which is never saved)
                if self.panels.time_of_day != self.settings.time_of_day && !self.photo_time_held() {
                    self.settings.time_of_day = self.panels.time_of_day;
                }
                if let Some(msg) = self.world.alerts.pop_front() {
                    log::info!("alert: {msg}");
                }
                t = self.ui_parts.lap(Part::UiOptions, t);
                // teleport screen (and the login loading screen fading out)
                if let Some(o) = self.tp_overlay.as_mut() {
                    // quick fade in, fade out at the end
                    let fade_in = if o.loading_end {
                        1.0
                    } else {
                        o.start.elapsed().as_secs_f32() / 0.25
                    };
                    let fade_out = o.end.map_or(1.0, |e| 1.0 - e.elapsed().as_secs_f32() / ui::loading::FADE_SECONDS);
                    let alpha = fade_in.min(fade_out).clamp(0.0, 1.0);
                    let title = if o.loading_end {
                        aurora_i18n::tr!("common-loading-world")
                    } else {
                        aurora_i18n::tr!("common-teleport-2")
                    };
                    // a screen fading out says what it said when the fade began
                    let text = o.leaving.take().unwrap_or_else(|| {
                        // the banner names the region; its arrival point is one of its facts
                        let (dest, point) = ui::loading::split_destination(&o.dest);
                        let arrived = o.loading_end || o.arrived.is_some() || dest.is_empty();
                        let name = if dest.is_empty() {
                            self.world.region_name()
                        } else {
                            dest.to_owned()
                        };
                        let stage = if o.loading_end {
                            // the line the loading screen ended on
                            let stats = self.scene.textures.stats;
                            ui::loading::stage_line(
                                &self.loading_stage,
                                &name,
                                self.world.terrain_progress(),
                                self.world.objects.len(),
                                (stats.loaded, stats.total),
                            )
                        } else if o.shown >= 0.985 {
                            aurora_i18n::tr!("app-welcome").to_owned()
                        } else {
                            self.world.tp_status.clone()
                        };
                        let facts = ui::loading::PlaceFacts::gather(&mut self.world, &mut self.map_tiles, &name, point, arrived);
                        ui::loading::BannerText { name, stage, facts }
                    });
                    ui::loading::teleport_overlay(
                        &ctx,
                        &p,
                        self.logo.as_ref(),
                        self.settings.loading_backdrop.then_some(&self.backdrop),
                        title,
                        &text.name,
                        &text.stage,
                        Some(o.shown),
                        alpha,
                        &text.facts.place(),
                    );
                    if o.end.is_some() {
                        o.leaving = Some(text);
                    }
                }
            }
        }
        if self.demo && self.frame_count == 550 && std::env::var_os("AURORA_DEMO_BUG_REPORT").is_some() {
            self.bug_report.requested = true;
        }
        if self.bug_report.requested {
            self.open_bug_report(renderer);
        }
        if self.demo {
            self.bug_report.demo_step(self.frame_count);
        }
        self.bug_report
            .show(&ctx, &p, &self.skin.icons, matches!(self.screen, Screen::World));
        a.exit = self.exit_prompt.show(&ctx, &p, self.in_world());
        self.ui_parts.lap(Part::UiRest, t);
        a
    }
}

/// AURORA_WINDOW_SIZE="w,h": the window size of this run (logical pixels),
/// to measure by window size. It is not remembered: the next run opens at
/// the size of the last session again.
fn forced_window_size() -> Option<(u32, u32)> {
    static SIZE: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        let v = std::env::var("AURORA_WINDOW_SIZE").ok()?;
        let (w, h) = v.split_once(',')?;
        Some((w.trim().parse::<u32>().ok()?.max(800), h.trim().parse::<u32>().ok()?.max(560)))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameTagsEnv {
    Classic,
    Plate,
    Alternate,
}

/// AURORA_NAME_TAGS=classic|plate|alt: the style of the avatar name tags of
/// this run, whatever the setting (which is left alone). `alt` switches
/// with each `perf summary` line, to measure both styles in one run.
fn name_tags_env() -> Option<NameTagsEnv> {
    static STYLE: std::sync::OnceLock<Option<NameTagsEnv>> = std::sync::OnceLock::new();
    *STYLE.get_or_init(|| match std::env::var("AURORA_NAME_TAGS").ok()?.trim() {
        "classic" => Some(NameTagsEnv::Classic),
        "plate" => Some(NameTagsEnv::Plate),
        "alt" => Some(NameTagsEnv::Alternate),
        other => {
            log::warn!("AURORA_NAME_TAGS: unknown style {other:?} (classic, plate or alt)");
            None
        }
    })
}

#[derive(Default)]
struct UiActions {
    exit: Option<ui::exit::Action>,
    update_quit: bool,
    login: LoginAction,
    chat: Option<ui::chat::OutgoingChat>,
    bar: BarAction,
    settings_changed: bool,
    font_changed: bool,
    cancel_loading: bool,
    ims: Vec<(uuid::Uuid, String)>,
    offer_tp: Vec<uuid::Uuid>,
    landmark: Option<uuid::Uuid>,
    audio: Option<ui::audio::AudioAction>,
    ctx_action: Option<ui::context::CtxAction>,
    object_confirm: Option<Option<i32>>,
    /// Display name change to send (old, new).
    set_display_name: Option<(String, String)>,
    /// Estimate and purchase requests of the Buy L$ window.
    currency: Vec<NetCommand>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        // captures keep a fixed, predictable window
        let capture = self.capture.is_some();
        let (mut w, mut h) = (self.settings.window_width.max(800), self.settings.window_height.max(600));
        // (a forced size is placed like a capture's window: not where the
        // last session was, not maximized)
        let forced = forced_window_size();
        if let Some(size) = forced {
            (w, h) = size;
        }
        let capture = capture || forced.is_some();
        let mut pos = None;
        let mut monitor = event_loop.primary_monitor();
        // last position, only if it is still on a connected screen
        if let (Some(x), Some(y), false) = (self.settings.window_x, self.settings.window_y, capture) {
            let on = event_loop.available_monitors().find(|m| {
                let (p, s) = (m.position(), m.size());
                // the title bar (top-left corner area) must be reachable
                x + 40 >= p.x && x + 40 < p.x + s.width as i32 && y + 10 >= p.y && y + 10 < p.y + s.height as i32
            });
            if on.is_some() {
                pos = Some((x, y));
                monitor = on;
            }
        }
        // a "normal" size covering the whole screen is a maximized window
        // recorded as normal (older versions): restore to 80 % of the screen
        if let (Some(m), false) = (&monitor, capture) {
            let sf = m.scale_factor();
            let (mw, mh) = (m.size().width as f64 / sf, m.size().height as f64 / sf);
            if w as f64 >= mw * 0.95 && h as f64 >= mh * 0.9 {
                w = (mw * 0.8) as u32;
                h = (mh * 0.8) as u32;
                let p = m.position();
                pos = Some((
                    p.x + ((m.size().width as f64 - w as f64 * sf) / 2.0) as i32,
                    p.y + ((m.size().height as f64 - h as f64 * sf) / 2.0) as i32,
                ));
            }
        }
        let mut attrs = Window::default_attributes()
            .with_title(crate::cli::window_title())
            // Vulkan initialization and the first frame take time: Windows
            // must not expose the unpainted client area while they run.
            .with_visible(false)
            // Automated captures must not steal the user's keyboard focus.
            .with_active(!capture)
            .with_inner_size(winit::dpi::LogicalSize::new(w, h))
            .with_min_inner_size(winit::dpi::LogicalSize::new(800, 560));
        if let Some((x, y)) = pos {
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("cannot create window: {e}");
                event_loop.exit();
                return;
            }
        };
        crate::window_hold::install(&window);
        let size = window.inner_size();
        if std::env::var_os("AURORA_NOVSYNC").is_some() {
            self.settings.vsync = false;
        }
        let renderer = match Renderer::new(window.clone(), size.width, size.height, self.settings.vsync) {
            Ok(r) => r,
            Err(e) => {
                log::error!("renderer initialisation failed: {e}");
                eprintln!(
                    "{}",
                    aurora_i18n::tr!("app-aurora-viewer-unable-to-initialize-vulkan", e = format!("{}", e))
                );
                event_loop.exit();
                return;
            }
        };
        log::info!(
            "renderer ready: {} via {} ({} texture slots in {} pages at most, GPU timers: {})",
            renderer.info.name,
            renderer.info.backend,
            renderer.textures.capacity(),
            renderer.info.max_textures,
            renderer.info.timestamps
        );
        let mut renderer = renderer;
        if std::mem::take(&mut self.settings.new_profile) {
            // a first launch: the graphics level from the card, like
            // Firestorm's recommended settings; the test overrides still win
            let system_mb = {
                use sysinfo::{MemoryRefreshKind, RefreshKind, System};
                let system = System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()));
                Some(system.total_memory() / 1024 / 1024).filter(|&mb| mb > 0)
            };
            let level = crate::graphics_presets::Preset::for_machine(renderer.info.integrated, renderer.info.vram_mb, system_mb);
            log::info!(
                "new profile: graphics level {level:?} (integrated {}, video memory {:?} MB, system memory {:?} MB)",
                renderer.info.integrated,
                renderer.info.vram_mb,
                system_mb
            );
            level.apply(&mut self.settings);
            apply_test_overrides(&mut self.settings);
            self.settings.save();
        }
        renderer.apply_settings(self.settings.render_settings());
        self.screen_rate.poll(&window, Instant::now());
        let max_tex = renderer.device.limits().max_texture_dimension_2d as usize;
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_tex),
        );
        self.gfx = Some(Gfx {
            window,
            shown: false,
            maximize_on_show: self.settings.window_maximized && !capture,
            renderer,
            egui_state,
        });
        if std::env::var_os("AURORA_DEMO").is_some() {
            self.demo = true;
            if self.update_demo.is_some() {
                // The update preview stays offline on a synthetic login screen.
                return;
            }
            // before the login scenarios: with AURORA_DEMO_LOGIN the
            // preferences open over the login screen (its cogwheel)
            if std::env::var_os("AURORA_DEMO_OPTIONS").is_some() {
                self.panels.settings = true;
                self.panels.perf = false;
                let value = std::env::var("AURORA_DEMO_OPTIONS").unwrap_or_default();
                let (tab, filter) = value.split_once(':').unwrap_or((&value, ""));
                self.options_ui.tab = tab.parse().unwrap_or(0);
                if self.options_ui.tab == 8 {
                    self.options_ui.shortcut_filter = filter.to_owned();
                } else if filter == "reset" {
                    // the "Reset this section" confirmation, for captures
                    self.options_ui.ask_reset_section();
                } else {
                    self.options_ui.scroll_to_end = filter == "end";
                }
            }
            if let Ok(mode) = std::env::var("AURORA_DEMO_LOGIN") {
                if matches!(
                    mode.as_str(),
                    "cards"
                        | "cards1"
                        | "cards2"
                        | "cards8"
                        | "derez1"
                        | "add-mfa"
                        | "drag"
                        | "drag2"
                        | "drag-card"
                        | "drag-card2"
                        | "hover"
                        | "wheel-gap"
                        | "arrows"
                        | "static-arrows"
                        | "derez"
                        | "mfa"
                        | "manage"
                        | "manage8"
                        | "manage-drag"
                        | "manage-confirm"
                        | "first"
                        | "first-mfa"
                        | "add"
                        | "password"
                        | "connecting"
                        | "first-connecting"
                        | "add-connecting"
                ) {
                    self.login_form.accounts = (0..match mode.as_str() {
                        "cards2" | "drag2" | "drag-card2" | "mfa" | "manage" | "manage-confirm" | "add" | "password" | "connecting"
                        | "add-connecting" => 2,
                        "first" | "first-mfa" | "first-connecting" => 0,
                        "cards1" | "derez1" | "add-mfa" => 1,
                        "cards8" | "drag" | "drag-card" | "manage8" | "manage-drag" | "wheel-gap" | "arrows" => 8,
                        _ => 3,
                    })
                        .map(|i| crate::login_accounts::Account {
                            username: format!("Example {}", i + 1),
                            uri: self.settings.login_uri(),
                            remember_password: !mode.starts_with("manage") || i % 3 != 2,
                            remember_mfa: mode.starts_with("manage") && i % 2 == 0,
                            start_location: if i % 2 == 0 { "last" } else { "home" }.into(),
                            start_region: String::new(),
                            agent: None,
                            last_place: None,
                        })
                        .collect();
                    self.login_form.accounts_loaded = true;
                    self.login_form
                        .places
                        .preview(&self.egui_ctx, &self.palette, &mut self.login_form.accounts);
                    self.settings.username = self
                        .login_form
                        .accounts
                        .first()
                        .map_or_else(String::new, |account| account.username.clone());
                    self.settings.remember_password = !self.login_form.accounts.is_empty();
                    if matches!(mode.as_str(), "first" | "first-mfa" | "add" | "add-mfa" | "password") {
                        self.settings.remember_password = false;
                        self.login_form.editing = true;
                        self.login_form.adding = matches!(mode.as_str(), "add" | "add-mfa");
                        if self.login_form.adding {
                            // what the Add button records to come back to
                            self.login_form.add_return = Some((self.settings.login_uri(), self.settings.username.trim().to_lowercase()));
                            self.settings.username.clear();
                        }
                        if mode == "add-mfa" {
                            // a second identity, ready to submit (no real credential)
                            self.settings.username = "Example new account".into();
                            self.login_form.password = "demo-placeholder".into();
                        }
                        if mode == "first-mfa" {
                            self.settings.username = "Example new account".into();
                            self.login_form.show_mfa = true;
                        }
                        if mode == "password" {
                            self.login_form.accounts[0].remember_password = false;
                        }
                    }
                    if mode.ends_with("-connecting") {
                        self.settings.username = "Example new account".into();
                        self.settings.remember_password = false;
                        self.login_form.busy = true;
                        self.login_form.adding = mode == "add-connecting";
                        self.login_form.editing = true;
                        self.login_progress = Some((aurora_i18n::tr!("session-connecting-to-login-server").into(), 0.35));
                    }
                    if matches!(mode.as_str(), "derez" | "derez1" | "connecting") {
                        self.login_form.pending_connect = Some(0);
                    } else if mode == "mfa" {
                        self.login_form.editing = true;
                        self.login_form.show_mfa = true;
                    } else if mode.starts_with("manage") {
                        self.login_form.managing = true;
                        if mode == "manage-confirm" {
                            self.login_form.manager.preview_remove(&self.login_form.accounts[0]);
                        }
                    }
                    return;
                }
                self.settings.username = "Demo Resident".into();
                self.settings.remember_username = true;
                self.settings.remember_password = mode == "remembered";
                self.login_form.stored = self.settings.remember_password.then(|| "demo-login-marker".into());
                self.login_form.stored_for = Some((self.settings.login_uri(), self.settings.username.to_lowercase()));
                // Stay on the login screen without a grid or a credential store.
                return;
            }
            log::info!("demo mode: injecting synthetic region");
            let rig = self.scene.avatar_lib.rig.clone();
            if crate::demo::tags::mode() == Some(crate::demo::tags::Mode::Height) {
                self.scene.anims.insert(
                    crate::demo::tags::HEIGHT_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::tags::height_animation(&rig), &rig)),
                );
            }
            self.scene.anims.insert(
                crate::demo::SEAT_ANIM,
                Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::seat_animation(), &rig)),
            );
            if crate::demo::deform::mode().is_some() {
                self.scene.anims.insert(
                    crate::demo::deform::DEFORM_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::deform::animation(&rig), &rig)),
                );
            }
            let demo_anim = if std::env::var_os("AURORA_DEMO_ANIM_LOOP").is_some() {
                crate::demo::loop_animation()
            } else {
                crate::demo::idle_animation()
            };
            self.scene.anims.insert(
                crate::demo::IDLE_ANIM,
                Arc::new(crate::scene::anim::BoundAnim::bind(demo_anim, &rig)),
            );
            // AURORA_DEMO_ENV_SELECT: the library's settings assets, known locally
            if crate::demo::env::enabled() {
                for (id, settings, map) in crate::demo::env::settings_assets() {
                    self.scene.settings.insert_local(id, settings, Some(map));
                }
            }
            // AURORA_DEMO_BARS's favourites: the mockup's gradients as thumbnails
            for (id, colors) in crate::demo::place::favorite_thumbnails() {
                let image = ui::bars::favorites::demo_thumbnail(colors);
                let t = self
                    .egui_ctx
                    .load_texture(format!("demo-favorite-{id}"), image, egui::TextureOptions::LINEAR);
                self.ui_images.insert(id, t);
            }
            // AURORA_DEMO_PERF=compact|full|network: the performance window in
            // that view (network: full, with its Network section open)
            if let Ok(v) = std::env::var("AURORA_DEMO_PERF") {
                self.panels.perf = true;
                let network = v.trim() == "network";
                ui::perf::set_full(&self.egui_ctx, network || v.trim() == "full");
                if network {
                    ui::perf::open_network(&self.egui_ctx);
                }
            }
            if let Some(t) = std::env::var("AURORA_DEMO_TOD").ok().and_then(|v| v.parse::<u8>().ok()) {
                self.panels.time_of_day = t.min(4);
            }
            // AURORA_DEMO_DEBUG="bounds,culling,..." turns debug overlays on (captures)
            if let Ok(v) = std::env::var("AURORA_DEMO_DEBUG") {
                let d = &mut self.settings.debug;
                for k in v.split(',').map(str::trim) {
                    match k {
                        "complexity" => d.complexity_tags = true,
                        "alpha" => d.show_alpha = true,
                        "alpha_rigged" => d.show_alpha_rigged = true,
                        "wire" => d.wireframe = true,
                        "bounds" => d.bounds = true,
                        "culling" => d.culling = true,
                        "freeze" => d.freeze_culling = true,
                        "lights" => d.lights = true,
                        "glow" => d.glow_objects = true,
                        "glow_view" => d.glow_view = true,
                        "probes" => d.probes = true,
                        "skeletons" => d.skeletons = true,
                        "lod" => d.lod_info = true,
                        "shadows" => d.shadow_view = true,
                        "ao" => d.ao_view = true,
                        "check_lights" => d.check_light_sets = true,
                        "check_probes" => d.check_probe_sets = true,
                        _ => {}
                    }
                }
            }
            // AURORA_DEMO_CAM="yaw,pitch,distance" (radians, meters) for captures
            if let Ok(v) = std::env::var("AURORA_DEMO_CAM") {
                let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if p.len() >= 2 {
                    self.camera.demo_orbit(p[0], p[1], p.get(2).copied(), &self.settings.camera);
                }
            }
            // AURORA_DEMO_TAGS: residents whose name plates show every
            // state, at the three distance levels
            if crate::demo::tags::mode().is_some() {
                for ev in crate::demo::tags::events() {
                    self.world.apply(ev);
                }
                // (nothing over the rows of plates)
                self.panels.people = false;
            }
            // AURORA_DEMO_RADAR=1|tp|friends|groups|profile: residents around
            // for the radar of People (tp: with the ones of a region a
            // teleport just left), or its friends / groups tab (profil: with
            // the profile of a group)
            if let Ok(v) = std::env::var("AURORA_DEMO_RADAR") {
                self.demo_radar_tp = v == "tp";
                for ev in crate::demo::radar_events(self.demo_radar_tp) {
                    self.world.apply(ev);
                }
                self.panels.people = true;
                self.panels.people_tab = match v.as_str() {
                    "friends" | "amis" => 1,
                    "groups" | "groupes" | "profile" | "profil" => 2,
                    _ => 0,
                };
                if matches!(v.as_str(), "profile" | "profil") {
                    self.group_profile_ui.open(&mut self.world, crate::demo::DEMO_GROUP1);
                }
            }
            // AURORA_DEMO_PEOPLE=rows|crowd|sheet|legacy: Near you with the
            // residents of the People mock-up, alone on screen
            if crate::demo::people::mode().is_some() {
                for ev in crate::demo::people::events() {
                    self.world.apply(ev);
                }
                // the density is the list's own toggle; the look is forced
                // for the run only (`demo::people::legacy`), not saved
                self.settings.radar.compact = crate::demo::people::compact();
                // the friends tab opens sorted as the mock-up's (online first)
                self.settings.friends_list.sort = ui::people::FriendsSettings::default().sort;
                if let Some(id) = crate::demo::people::sheet() {
                    self.people_ui.radar_selected = vec![id];
                    self.people_ui.open_sheet(id);
                }
                if crate::demo::people::mode() == Some(crate::demo::people::Mode::Groups) {
                    self.people_ui.open_group_sheet(crate::demo::DEMO_GROUP1);
                    self.world.group_data.open(crate::demo::DEMO_GROUP1, true);
                }
                // the mock-up's window
                if let Some((w, h)) = crate::demo::people::window_size() {
                    let pos = ui::people::default_pos(ui::layout::work_area(&self.egui_ctx));
                    ui::widgets::place_floater(&self.egui_ctx, ui::people::WINDOW, pos, egui::vec2(w, h));
                }
                if let Some(id) = crate::demo::people::hovered() {
                    self.people_ui.radar_selected = vec![id];
                    self.people_ui.demo_hover = Some(id);
                }
                // the contact sets tab: the family, Nova chosen
                if crate::demo::people::mode() == Some(crate::demo::people::Mode::Sets) {
                    self.contacts_ui.set_choice = Some(aurora_i18n::tr!("demo-family").to_owned());
                    self.people_ui.select(vec![crate::demo::people::NOVA]);
                }
                self.panels.people = true;
                self.panels.people_tab = crate::demo::people::tab();
                self.panels.inventory = false;
                self.panels.chat = false;
                self.panels.minimap = false;
                self.panels.perf = false;
            }
            if std::env::var_os("AURORA_DEMO_UI").is_some() {
                self.panels.people = true;
                self.panels.people_tab = std::env::var("AURORA_DEMO_UI").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
                self.panels.inventory = true;
                self.panels.chat = true;
                self.panels.perf = false;
                self.panels.minimap = false;
            }
            // AURORA_DEMO_PLACE=1|lagoon|nordheim|grove|flaw|unknown: the
            // place details of a place link, as if clicked
            if let Some((region, pos)) = crate::demo::place::scenario() {
                ui::context::request(&self.egui_ctx, ui::context::CtxAction::ShowPlaceInfo(region.to_owned(), pos));
                self.panels.perf = false;
            }
            // AURORA_DEMO_PLACE=repere|historique: a landmark's / history
            // entry's profile (asked once the inventory is there: demo_pending)
            if let Some(source) = crate::demo::place::profile_scenario() {
                self.demo_place = Some(source);
                self.panels.perf = false;
            }
            if let Some(start) = ui::landmark_create::DemoStart::from_env() {
                self.create_landmark_ui.demo_pending = Some(start);
                self.panels.perf = false;
            }
            // AURORA_DEMO_PLACES=favorites|landmarks|history: "Places" on a tab
            if let Some(tab) = std::env::var("AURORA_DEMO_PLACES").ok().and_then(|v| ui::places::tab_from_name(&v)) {
                self.places_ui.open_tab(tab);
                self.panels.places = true;
                self.panels.perf = false;
            }
            // AURORA_DEMO_MARKETPLACE=<page>: World › SL Marketplace on that
            // page, open at start
            if self.marketplace_home().is_some() {
                self.marketplace_ui.open(None);
                self.panels.perf = false;
            }
            // AURORA_DEMO_MARKETPLACE_LISTINGS=merchant|not-merchant|error|audit:
            // Me › Marketplace listings open on the demo SLM server
            if crate::world::marketplace::demo::Mode::from_env().is_some() {
                self.demo_marketplace_listings();
                self.panels.perf = false;
            }
            // AURORA_DEMO_TP_CONFIRM=link|landmark: a teleport confirmation on
            // screen, asked whatever the saved setting says (not saved)
            if let Some(confirm) = crate::demo::place::teleport_confirm_scenario() {
                use crate::demo::place::TeleportConfirm;
                self.settings.confirm_teleports = true;
                match confirm {
                    TeleportConfirm::Link(region, pos) => self.place_confirm = Some((region.to_owned(), pos, false)),
                    TeleportConfirm::Landmark(asset, name) => self.places_ui.confirm_landmark(asset, name.to_owned()),
                }
                self.panels.perf = false;
            }
            // AURORA_DEMO_SEARCH=<tab>[:text]: "Search" on a tab, its
            // search done ("page" in the text: a full page, "xyz": nothing;
            // for land the text is the price limit, 0 by default)
            if let Ok(v) = std::env::var("AURORA_DEMO_SEARCH") {
                let (name, text) = v.split_once(':').unwrap_or((v.as_str(), "aurora"));
                // land without a text keeps the default price field ("0")
                let typed = !name.trim().eq_ignore_ascii_case("land") || v.contains(':');
                if let Some(tab) = crate::world::search::Tab::from_name(name) {
                    self.search_ui.open_tab(tab);
                    if typed {
                        self.search_ui.set_text(tab, text);
                    }
                    self.panels.search = true;
                    self.panels.perf = false;
                    if let Some(request) = self.search_ui.request(tab, &self.settings.search) {
                        let access = self.world.agent_access();
                        self.world.search.find(request, access, Instant::now());
                    }
                }
            }
            // AURORA_DEMO_PLACE_WINDOW=1: the standalone windows instead
            if std::env::var_os("AURORA_DEMO_PLACE_WINDOW").is_some() {
                self.settings.standalone_place_details = true;
            }
            // AURORA_DEMO_PROFILE=wolf|nova|friend|self[:tab|edit|compact]:
            // a profile window
            if let Ok(v) = std::env::var("AURORA_DEMO_PROFILE") {
                let (who, option) = v.split_once(':').unwrap_or((v.as_str(), "0"));
                let id = match who {
                    "self" => crate::demo::DEMO_AGENT,
                    "nova" => crate::demo::DEMO_NOVA,
                    "friend" => crate::demo::DEMO_FRIEND,
                    _ => crate::demo::DEMO_LOUP,
                };
                self.profile_ui.open(&mut self.world, id);
                self.profile_ui.demo(&self.egui_ctx, id, option);
                self.panels.perf = false;
            }
            // AURORA_DEMO_REGION=overview[:owner|:manager|:resident]: Region /
            // Estate on that page
            if let Some(scenario) = crate::demo::region::scenario() {
                self.region_ui.demo(scenario.page, scenario.preview, scenario.manage);
                if scenario.compact {
                    let pos = egui::pos2(560.0, 80.0);
                    ui::widgets::place_floater(&self.egui_ctx, ui::region::WINDOW, pos, egui::vec2(480.0, 720.0));
                }
                self.panels.region_estate = true;
                self.panels.perf = false;
            }
            // AURORA_DEMO_LAND=<tab>[:role][:classic]: About Land on that tab
            if let Some(scenario) = crate::demo::land::scenario() {
                self.land_ui.set_tab(&scenario.tab);
                self.panels.about_land = true;
                self.panels.perf = false;
            }
            // AURORA_DEMO_GROUP_PROFILE=1|member|officer|owner|guest: the
            // profile of the demo parcel's group, as after a click on its
            // name (a guest's: a group we are not in)
            if std::env::var_os("AURORA_DEMO_GROUP_PROFILE").is_some() {
                self.group_profile_ui.open(&mut self.world, crate::demo::group::shown_group());
                if let Some(member) = crate::demo::group::members_tab() {
                    self.group_profile_ui.show_member(member);
                }
                if let Some((group, read, compose)) = crate::demo::group::notices_tab() {
                    self.world.group_data.set_read(group, read);
                    self.group_profile_ui.show_notices(compose);
                }
                if crate::demo::group::admin_tab() {
                    self.group_profile_ui.show_admin(crate::demo::group::identity_section());
                }
                if crate::demo::group::experiences_tab() {
                    self.group_profile_ui.show_experiences();
                }
                if let Some(ledger) = crate::demo::group::land_tab() {
                    // the land we hold, for the most we may give
                    self.world.apply(NetEvent::LandHoldings {
                        credit: crate::demo::group::LAND_CREDIT,
                        committed: crate::demo::group::LAND_COMMITTED,
                    });
                    self.group_profile_ui.show_land(ledger);
                }
                if let Some(invitees) = crate::demo::group::invitees() {
                    self.group_profile_ui.invite.open(crate::demo::group::shown_group(), invitees);
                }
                self.panels.perf = false;
            }
            // AURORA_DEMO_GROUP_CREATE=form|cost|refused|limit|created: "Create
            // a group", as from People › Groups, once the group list is in
            if let Some(scenario) = crate::demo::group::create_scenario() {
                use crate::demo::group::CreateScenario;
                use ui::group_profile::create::{DemoStart, DemoThen, Draft};
                let draft = Draft {
                    name: crate::demo::group::create_name(scenario).into(),
                    info: aurora_net::groups::GroupInfo {
                        charter: aurora_i18n::tr!("demo-group-create-charter").into(),
                        show_in_list: true,
                        open_enrollment: true,
                        ..Default::default()
                    },
                    rating: Some(false),
                };
                let (draft, then) = match scenario {
                    CreateScenario::Form | CreateScenario::Limit => (None, DemoThen::Wait),
                    CreateScenario::Cost => (Some(draft), DemoThen::Press),
                    CreateScenario::Refused | CreateScenario::Created => (Some(draft), DemoThen::Send),
                };
                self.group_profile_ui.create.demo(DemoStart { draft, then });
                self.panels.perf = false;
            }
            // AURORA_DEMO_CONTACTS=friends|groups|sets|detached|add: the
            // Contacts tab of Conversations (or its torn-off window, or the
            // resident picker of "Add...")
            if let Ok(v) = std::env::var("AURORA_DEMO_CONTACTS") {
                self.panels.chat = true;
                self.panels.perf = false;
                self.chat_ui.contacts = true;
                self.settings.contacts.tab = match v.as_str() {
                    "groups" | "groupes" => 1,
                    "sets" | "cercles" => 2,
                    _ => 0,
                };
                let detached = matches!(v.as_str(), "detached" | "detache");
                self.settings.contacts.torn_off = detached;
                self.panels.contacts = detached;
                if matches!(v.as_str(), "add" | "ajout") {
                    self.contacts_ui.pick_new_friend();
                }
            }
            // AURORA_DEMO_CONV=1: the demo group chat with its participants;
            // =names[:narrow|wide]: conversations and nearby avatars whose
            // names do not fit, in windows of a known size
            if let Some((conversations, people)) = crate::demo::long_names_scenario() {
                self.panels.chat = true;
                self.panels.people = true;
                self.panels.people_tab = 0;
                self.panels.perf = false;
                self.chat_ui.selected = Some(crate::demo::DEMO_LONG_NAME);
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(8.0, 480.0), conversations);
                ui::widgets::place_floater(&self.egui_ctx, "people", egui::pos2(8.0, 40.0), people);
            } else if crate::demo::many_conversations_scenario() {
                // =many: more conversations than the list shows, the last
                // one opened as from a notification (the list scrolls to it)
                self.panels.chat = true;
                self.panels.perf = false;
                self.chat_ui
                    .open(crate::demo::many_conversation(crate::demo::MANY_CONVERSATIONS - 1));
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 420.0), egui::vec2(600.0, 320.0));
            } else if let Some(view) = crate::demo::bubbles_scenario() {
                // =bubbles[:busy|:group|:local|:legacy]: the window as a
                // messenger, or its legacy lines (its lines are added once
                // the login is in)
                self.panels.chat = true;
                self.panels.perf = false;
                self.chat_ui.selected = crate::demo::bubbles_conversation(view);
                if view == crate::demo::Bubbles::Legacy {
                    self.chat_ui.demo_style = Some(crate::settings::ChatStyle::Legacy);
                }
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(460.0, 230.0), egui::vec2(700.0, 560.0));
            } else if crate::demo::teleport_offer_scene().is_some() {
                self.panels.chat = true;
                self.panels.perf = false;
                self.chat_ui.open(crate::demo::DEMO_NOVA);
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 420.0), egui::vec2(600.0, 320.0));
            } else if std::env::var_os("AURORA_DEMO_CONV").is_some() {
                self.panels.chat = true;
                self.chat_ui.selected = Some(crate::demo::DEMO_GROUP1);
                self.chat_ui.show_profile = true;
            }
            // captures: AURORA_DEMO_MAP=1 opens the mini-map and the world map
            // (with a tracked spot east of the plaza)
            if let Ok(scenario) = std::env::var("AURORA_DEMO_MAP") {
                let mode = scenario.split(',').next().unwrap_or_default().trim();
                self.panels.minimap = true;
                self.panels.world_map = mode != "mini" && !mode.starts_with("beacon");
                self.panels.perf = false;
                let (x, y, z) = match mode {
                    "beacon" | "beacon-click" => (160.0, 128.0, 30.0),
                    "beacon-offscreen" => (80.0, 128.0, 30.0),
                    _ => (200.0, 150.0, 25.0),
                };
                if mode != "coordinates" {
                    self.world.map.track_location(256000.0 + x, 256000.0 + y, z, false);
                }
                // words separated by commas: search:<text> (the search with its
                // suggestions), drawer:<landmarks|history|friends>, landmarks
                // (212 of them, the drawer open), friend (Nova followed),
                // legacy (the side panel layout, the setting untouched)
                for word in scenario.split(',').map(str::trim) {
                    if let Some(text) = word.strip_prefix("search:") {
                        self.worldmap_ui.demo_search(&mut self.world, text);
                    } else if let Some(tab) = word.strip_prefix("drawer:") {
                        self.worldmap_ui.demo_drawer(tab);
                    } else if word == "landmarks" {
                        self.worldmap_ui.demo_drawer("landmarks");
                    } else if word == "friend" {
                        self.world.map.track_avatar(crate::demo::DEMO_FRIEND, "Nova ♪");
                    } else if word == "legacy" {
                        self.worldmap_ui.demo_style = Some(crate::settings::WorldMapStyle::Legacy);
                    }
                }
            }
            // captures: AURORA_DEMO_SIT keeps the toolbar sit button visible
            if std::env::var_os("AURORA_DEMO_SIT").is_some() {
                self.panels.perf = false;
            }
            // captures: AURORA_DEMO_NOTIF=1 opens the notification list
            if std::env::var_os("AURORA_DEMO_NOTIF").is_some() {
                self.notif_ui.open = true;
            }
            // AURORA_DEMO_NOTIFS: nothing else open over the corners
            if crate::demo::notifs::mode().is_some() {
                self.panels.perf = false;
                self.panels.chat = false;
                self.panels.inventory = false;
            }
            // AURORA_DEMO_TOASTS: the toasts arrive with the Conversations
            // window closed, or minimized for `minimized`
            if let Some(mode) = crate::demo::toasts::mode() {
                self.panels.perf = false;
                self.panels.chat = mode == "minimized";
                if mode == "minimized" {
                    ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(460.0, 230.0), egui::vec2(700.0, 560.0));
                    ui::widgets::minimize_floater(&self.egui_ctx, "conversations");
                }
            }
            // AURORA_DEMO_MATURITY: the content rating changed in the
            // Preferences, answered by the demo server
            if let Some(mode) = crate::demo::maturity_scene() {
                self.panels.settings = true;
                self.panels.perf = false;
                self.options_ui.tab = 1;
                self.notif_ui.open = mode != "agree";
            }
            // AURORA_DEMO_LURE: teleports received, with the notification list
            // (and the conversation the answers go to)
            if let Some(scene) = crate::demo::lure_scene() {
                use crate::demo::LureScene;
                self.notif_ui.open = true;
                self.panels.perf = false;
                if scene == LureScene::Reject {
                    self.world.status.cfg.reject_teleports = true;
                }
                if scene == LureScene::Answers || scene.shows_card() {
                    self.panels.chat = true;
                    self.chat_ui.open(crate::demo::DEMO_NOVA);
                    ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 420.0), egui::vec2(600.0, 320.0));
                }
                // the card alone: the list stays closed where no answer is shown
                if matches!(
                    scene,
                    LureScene::OfferCard | LureScene::RequestCard | LureScene::Legacy | LureScene::Exceeded
                ) {
                    self.notif_ui.open = false;
                }
                if scene == LureScene::Legacy {
                    self.chat_ui.demo_style = Some(crate::settings::ChatStyle::Legacy);
                }
            }
            // AURORA_DEMO_TALK=1: microphone on (voice dot of our avatar)
            if std::env::var_os("AURORA_DEMO_TALK").is_some() {
                self.mic_on = true;
            }
            for (id, m) in crate::demo::legacy_materials() {
                self.scene.legacy_mats.insert(id, m);
            }
            for (id, m) in crate::demo::pbr_materials() {
                self.scene.materials.insert(id, m);
            }
            self.sync_chat_log();
            for ev in crate::demo::events() {
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
            }
            // AURORA_DEMO_SLURLS: place links of every rating, and one unknown
            if crate::demo::slurls_scene() {
                for ev in crate::demo::slurls_events() {
                    self.world.apply(ev);
                }
                let text = crate::demo::slurls_text();
                let agent = self.world.agent_id;
                self.world.apply(NetEvent::InstantMessage(aurora_net::InstantMessage {
                    from_agent_id: crate::demo::DEMO_NOVA,
                    from_name: "Tess Touch".into(),
                    to_agent_id: agent,
                    dialog: 0,
                    session_id: uuid::Uuid::nil(),
                    message: text.clone(),
                    offline: false,
                    from_group: false,
                    binary_bucket: Vec::new(),
                    timestamp: 0,
                }));
                // and said by an object, in the chat console
                self.world.apply(NetEvent::Chat(aurora_net::ChatMessage {
                    from_name: aurora_i18n::tr!("demo-places-sign").into(),
                    source_id: uuid::Uuid::from_u128(0x5106),
                    owner_id: uuid::Uuid::nil(),
                    source_type: aurora_net::ChatSourceType::Object,
                    chat_type: aurora_net::ChatType::Normal,
                    position: glam::Vec3::ZERO,
                    message: text.clone(),
                }));
                self.world.notifications.push(
                    crate::world::notifications::Kind::Info,
                    "Tess Touch",
                    text,
                    crate::world::notifications::Data::None,
                );
                self.notif_ui.open = true;
                self.panels.perf = false;
                self.panels.places = false;
                self.panels.people = false;
                self.panels.chat = true;
                self.chat_ui.open(crate::demo::DEMO_NOVA);
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 420.0), egui::vec2(600.0, 320.0));
                // =offer: our teleport offer to her, with our region's badge
                if std::env::var("AURORA_DEMO_SLURLS").as_deref() == Ok("offer") {
                    let region = self
                        .world
                        .main()
                        .and_then(|r| r.info.as_ref())
                        .map(|i| i.name.clone())
                        .unwrap_or_default();
                    self.contacts_ui.ask_teleport_offer(vec![crate::demo::DEMO_NOVA], &region);
                }
            }
            // AURORA_DEMO_MATURITY: "General" asked once the login is known
            if crate::demo::maturity_scene().is_some() {
                self.world.set_maturity_preference(aurora_net::search::SIM_ACCESS_PG);
            }
            // AURORA_DEMO_LURE=card-answered / notification-answered: the
            // offer answered by its card's button, or by its entry's
            if let Some(scene) = crate::demo::lure_scene()
                && let Some(v) = self.world.teleport_invites.list.first().map(|v| (v.id, v.notification))
            {
                use crate::demo::LureScene;
                match scene {
                    LureScene::CardAnswered | LureScene::ExceededContinue => {
                        self.on_ctx_action(ui::context::CtxAction::AnswerTeleport { invite: v.0, accept: true })
                    }
                    LureScene::NotificationAnswered => {
                        let (cmds, _) = self.world.respond_notification(v.1, crate::world::notifications::Response::Decline);
                        for c in cmds {
                            self.send(c);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(view) = crate::demo::bubbles_scenario() {
                crate::demo::bubbles_lines(&mut self.world, view);
            }
            // AURORA_DEMO_CONV=tp[:sent|reject]: the avatar menu's teleport
            // offer (its invitation, or sent as invited to the resident whose
            // conversation is open and to one without any), or a request
            // while we reject teleports
            {
                use crate::demo::{DEMO_NOVA, DEMO_TESS, TeleportOfferScene};
                use ui::context::CtxAction;
                match crate::demo::teleport_offer_scene() {
                    Some(TeleportOfferScene::Invitation) => self.on_ctx_action(CtxAction::OfferTeleport(DEMO_NOVA)),
                    Some(TeleportOfferScene::Sent) => {
                        let invitation = aurora_i18n::tr!("common-join-me-in", region = self.world.region_name());
                        let cmd = self.world.offer_teleport(vec![DEMO_NOVA, DEMO_TESS], &invitation);
                        self.send(cmd);
                    }
                    Some(TeleportOfferScene::Reject) => self.contacts_ui.ask_teleport_request(DEMO_NOVA, true),
                    None => {}
                }
            }
            if std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("faces")) {
                let invisible = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("faces-invisible,"));
                let complex = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("faces-complex,"));
                let events = if complex {
                    crate::demo::complex_face_selection_events()
                } else {
                    crate::demo::face_selection_events(invisible)
                };
                if complex && let Some(gfx) = &mut self.gfx {
                    self.scene.install_demo_selection_mesh(&mut gfx.renderer);
                }
                for event in events {
                    self.world.apply(event);
                }
            }
            // AURORA_DEMO_CHATLOG: the conversation transcripts, written in
            // the demo account's own folder (demo_chatlog.rs)
            if let Some(mode) = crate::demo::chatlog::mode() {
                use crate::demo::chatlog::{self, Mode};
                self.panels.chat = true;
                self.panels.perf = false;
                self.panels.people = false;
                self.panels.minimap = false;
                // windows of a known size
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(8.0, 44.0), egui::vec2(660.0, 560.0));
                ui::widgets::place_floater(&self.egui_ctx, "chat_history", egui::pos2(8.0, 44.0), egui::vec2(620.0, 600.0));
                let me = self.world.agent_id;
                if !matches!(mode, Mode::Long { .. })
                    && let Some(dir) = self.world.chat_log.dir().map(std::path::Path::to_path_buf)
                    && let Err(e) = chatlog::seed(&dir, mode, &self.world.own_name())
                {
                    log::warn!("demo transcripts not written: {e}");
                }
                match mode {
                    Mode::Long { lines, .. } => {
                        for i in 0..lines {
                            let (own, text) = chatlog::long_line(i);
                            if own {
                                self.world.own_im(chatlog::PEER, &text);
                            } else {
                                self.world.apply(chatlog::message(me, &text));
                            }
                        }
                        // not two thousand notification sounds
                        self.world.im_messages.clear();
                    }
                    Mode::Offline => {}
                    Mode::Im | Mode::Find | Mode::History { .. } => {
                        self.world.apply(chatlog::new_message(me));
                    }
                }
                if let Mode::History { .. } = mode {
                    self.panels.chat = false;
                    let (title, file) = self.world.transcript_of(Some(chatlog::PEER));
                    self.history_ui.show_file(title, file);
                }
                self.chat_ui.selected = Some(chatlog::PEER);
            }
            if std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("cycle,") || v.starts_with("cycle-linked,")) {
                for ev in crate::demo::selection_cycle_events() {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_SKY_OPTICS: from behind the avatar, up at the eastern sky
            if let Some(dir) = crate::demo::sky::view_direction()
                && std::env::var_os("AURORA_DEMO_CAM").is_none()
            {
                let eye = self.world.agent.position + Vec3::new(-3.0, 0.0, 2.5);
                self.camera
                    .set_view(eye, eye + dir * 30.0, &mut self.world.agent, &self.settings.camera);
            }
            if std::env::var_os("AURORA_DEMO_REPLACE_PRIM").is_some() && std::env::var_os("AURORA_DEMO_CAM").is_none() {
                let focus = Vec3::new(135.5, 125.0, crate::demo::floor_at(135.0, 125.0) + 1.3);
                self.camera.set_view(
                    focus + Vec3::new(0.0, -6.0, 4.0),
                    focus,
                    &mut self.world.agent,
                    &self.settings.camera,
                );
            }
            if std::env::var_os("AURORA_DEMO_FLEXIBLE").is_some() && std::env::var_os("AURORA_DEMO_CAM").is_none() {
                let edit_focus = std::env::var("AURORA_DEMO_BUILD")
                    .ok()
                    .and_then(|spec| crate::build::demo::target_index(&spec, &self.world))
                    .and_then(|i| crate::scene::Scene::object_transform(&self.world, i, Instant::now(), 0))
                    .map(|(pos, _, _)| pos);
                let focus = edit_focus.unwrap_or_else(|| Vec3::new(141.0, 132.0, crate::demo::floor_at(141.0, 132.0) + 3.0));
                let offset = if edit_focus.is_some() {
                    Vec3::new(0.0, -7.0, 3.0)
                } else {
                    Vec3::new(0.0, -14.0, 6.0)
                };
                self.camera
                    .set_view(focus + offset, focus, &mut self.world.agent, &self.settings.camera);
            }
            if let Some(mode) = &self.ao_demo {
                crate::ao::demo::assets(&mut self.scene);
                let selected = crate::ao::demo::seed(&mut self.world.inventory, self.world.agent_id);
                self.settings.ao.accounts.insert(
                    self.world.agent_id,
                    crate::ao::Account {
                        enabled: mode != "off",
                        selected,
                    },
                );
                self.panels.ao = true;
                self.panels.perf = false;
                self.panels.inventory = false;
            }
            if let Ok(view) = std::env::var("AURORA_DEMO_APPEARANCE") {
                crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
                for cmd in
                    crate::world::appearance::sync_commands(&self.world.inventory, self.world.agent_id, &self.world.worn_attachment_items())
                {
                    self.send(cmd);
                }
                self.panels.appearance = true;
                self.panels.perf = false;
                self.appearance_ui.open(
                    match view.as_str() {
                        "outfits" => 1,
                        "worn" | "save" => 2,
                        _ => 0,
                    },
                    view == "edit",
                );
                if view == "save" {
                    self.appearance_ui.begin_save_as(&self.world.inventory);
                }
                if view == "duplicates" {
                    crate::world::appearance::seed_duplicate_response(&mut self.world.inventory, self.world.agent_id);
                }
                if matches!(view.as_str(), "menu" | "duplicates") {
                    self.appearance_ui.open_outfit(uuid::Uuid::from_u128(704));
                }
            }
            self.demo_shape_editor();
            if let Some(mode) = ui::widgets::demo::rez_mode() {
                if mode != "1" && std::env::var_os("AURORA_DEMO_CAM").is_none() {
                    self.camera.demo_orbit(0.0, 0.6, Some(9.0), &self.settings.camera);
                }
                crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                self.panels.inventory = true;
                self.inventory_ui.show_original(
                    &self.world.inventory,
                    uuid::Uuid::from_u128(if mode == "nocopy" { 8109 } else { 8100 }),
                );
                self.inventory_ui.expand_all = Some(true);
                if matches!(mode, "object" | "contents") {
                    for ev in crate::demo::action_events() {
                        self.world.apply(ev);
                    }
                    if let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::action_id(970)) {
                        let p = self.world.agent.position + Vec3::new(5.0, 2.0, 0.0);
                        let z = self.world.ground_height(p).unwrap_or(p.z);
                        if let Some(o) = self.world.objects.get_mut(idx) {
                            o.position = Vec3::new(p.x, p.y, z + 1.0);
                            o.scale = Vec3::ONE;
                            o.owner_id = self.world.agent_id;
                            o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY | crate::ui::context::flags::OBJECT_YOU_OWNER;
                        }
                    }
                }
            }
            if let Ok(view) = std::env::var("AURORA_DEMO_INVENTORY") {
                crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                let configured_view = matches!(
                    view.as_str(),
                    "sort"
                        | "filters"
                        | "preferences"
                        | "recent"
                        | "worn"
                        | "filtered"
                        | "large"
                        | "long"
                        | "long-filtered"
                        | "resize-left"
                        | "resize-right"
                        | "received"
                        | "received-filtered"
                        | "received-missing"
                );
                if configured_view {
                    crate::world::inventory::demo::seed_view(&mut self.world.inventory, self.world.agent_id, view == "large");
                    self.settings.inventory = ui::inventory::InventoryPreferences::default();
                    self.inventory_ui.show_original(&self.world.inventory, self.world.inventory.root);
                    self.inventory_ui.filters_open = view == "filters";
                    self.inventory_ui.preferences_open = view == "preferences";
                    if view == "sort" {
                        self.inventory_ui.show_original(&self.world.inventory, uuid::Uuid::from_u128(8000));
                        self.inventory_ui.open_folder_window(uuid::Uuid::from_u128(8000));
                    }
                    if view == "recent" {
                        self.inventory_ui.tab = 2;
                    }
                    if view == "worn" {
                        self.inventory_ui.tab = 3;
                    }
                    if matches!(view.as_str(), "received" | "received-filtered" | "received-missing") {
                        self.inventory_ui.tab = 1;
                        if view != "received-missing" {
                            crate::world::inventory::demo::seed_received(&mut self.world.inventory, self.world.agent_id);
                            self.inventory_ui.expand_all = Some(true);
                        }
                        if view == "received-filtered" {
                            self.inventory_ui.search = aurora_i18n::tr!("world-inventory-demo-instructions").into();
                        }
                    }
                    if view == "filtered" {
                        self.settings.inventory.filter_defaults.types &= !(1 << 6);
                        self.inventory_ui = Default::default();
                        self.inventory_ui.filters_open = true;
                    }
                    if view == "large" {
                        self.inventory_ui.search = aurora_i18n::tr!("app-item").into();
                    }
                    if matches!(view.as_str(), "long" | "long-filtered" | "resize-left" | "resize-right") {
                        crate::world::inventory::demo::seed_long_names(&mut self.world.inventory);
                        if view == "long-filtered" {
                            self.inventory_ui.search = "JEANS_SUBSTANCE".into();
                        } else {
                            self.inventory_ui.show_original(&self.world.inventory, uuid::Uuid::from_u128(8100));
                        }
                        self.inventory_ui.expand_all = Some(true);
                    }
                }
                self.panels.inventory = true;
                self.panels.perf = false;
                self.panels.appearance = false;
                self.panels.minimap = false;
                // "offer" modes keep the sample inventory offer to answer
                if !view.starts_with("offer") {
                    self.world.notifications = Default::default();
                }
                if let Some(id) = crate::world::inventory::demo::target(&view) {
                    self.inventory_ui.show_original(&self.world.inventory, id);
                    if view.starts_with("clipboard") {
                        if view == "clipboard-window" {
                            self.inventory_ui.open_folder_window(uuid::Uuid::from_u128(8000));
                            self.inventory_ui.open_folder_window(uuid::Uuid::from_u128(8001));
                        } else {
                            crate::world::inventory::demo::seed_clipboard_tree(&mut self.world.inventory);
                            if view == "clipboard-cut-folder"
                                && let Some(item) = self.world.inventory.items.get_mut(&uuid::Uuid::from_u128(8223))
                            {
                                item.owner_mask &= !crate::world::inventory::actions::COPY;
                            }
                        }
                    } else if view.starts_with("delete-") {
                        crate::world::inventory::demo::seed_clipboard_tree(&mut self.world.inventory);
                        self.inventory_ui.expand_all = Some(true);
                        self.inventory_ui.show_original(&self.world.inventory, uuid::Uuid::from_u128(8001));
                    } else if view == "new-folder" {
                        if let Ok(change) = crate::world::inventory::actions::create_folder(id, aurora_i18n::tr!("common-new-folder")) {
                            self.apply_inventory_action(ui::inventory::InvAction::Edit(change));
                        }
                    } else if matches!(view.as_str(), "new-script" | "new-note") {
                        self.apply_inventory_action(ui::inventory::InvAction::Create {
                            parent: id,
                            kind: if view == "new-script" {
                                aurora_net::inventory::operations::NewItem::Script
                            } else {
                                aurora_net::inventory::operations::NewItem::Note
                            },
                            name: if view == "new-script" {
                                aurora_i18n::tr!("common-new-script")
                            } else {
                                aurora_i18n::tr!("common-new-notecard")
                            }
                            .into(),
                        });
                    } else if matches!(view.as_str(), "folder-window" | "folder-window-search") {
                        self.inventory_ui.open_folder_window(id);
                        if view == "folder-window-search"
                            && let Some(window) = self.inventory_ui.windows.last_mut()
                        {
                            window.state.search = aurora_i18n::tr!("app-demo").into();
                        }
                    } else if view.starts_with("move-") && view != "move-menu" {
                        if view == "move-multi" {
                            self.inventory_ui.selection.extend([8109, 8110].map(uuid::Uuid::from_u128));
                        }
                        if view == "move-window" {
                            self.inventory_ui.open_folder_window(uuid::Uuid::from_u128(8000));
                            if let Some(window) = self.inventory_ui.windows.last_mut() {
                                window.state.begin_move_to(id);
                            }
                        } else {
                            self.inventory_ui.begin_move_to(id);
                            if view == "move-filter" {
                                self.inventory_ui.filter_move_to(aurora_i18n::tr!("world-inventory-demo-subfolder"));
                            }
                        }
                    } else if view == "rename" {
                        self.inventory_ui.begin_rename(&self.world.inventory, id);
                    } else if matches!(view.as_str(), "multi-add" | "multi-detach" | "delete") {
                        let items: Vec<_> = [8100, 8109, 8110].into_iter().map(uuid::Uuid::from_u128).collect();
                        if view == "multi-detach" {
                            self.apply_appearance_action(crate::world::appearance::Action::WearItems {
                                items: items.clone(),
                                replace: false,
                                point: 35,
                            });
                        }
                        self.inventory_ui.selection.extend(items.iter().copied());
                        if view == "delete" {
                            self.inventory_ui.dialog = Some(ui::inventory::EditDialog::Delete(items, false));
                        } else {
                            self.inventory_ui.demo_menu = Some(id);
                        }
                    } else if matches!(view.as_str(), "properties" | "animation-properties") {
                        self.inventory_ui.dialog = Some(ui::inventory::EditDialog::Properties(id));
                    } else if view.starts_with("note-task") {
                        self.demo_task_notecard(&view);
                    } else if view.starts_with("note-") {
                        if view == "note-readonly"
                            && let Some(it) = self.world.inventory.items.get_mut(&id)
                        {
                            it.owner_mask &= !crate::world::inventory::actions::MODIFY;
                        }
                        if view == "note-new"
                            && let Some(it) = self.world.inventory.items.get_mut(&id)
                        {
                            it.asset_id = uuid::Uuid::nil();
                        }
                        self.inventory_ui.preview = Some(id);
                        self.apply_inventory_action(ui::inventory::InvAction::Preview(id));
                    } else if view == "animation-open" {
                        self.inventory_ui.preview = Some(id);
                    } else if view.starts_with("image") {
                        self.inventory_ui.thumbnail.open(id);
                        self.inventory_ui.thumbnail.picker_open = view == "image-picker";
                        self.inventory_ui.thumbnail.photo_open = matches!(view.as_str(), "image-photo" | "image-photo-save");
                    } else {
                        self.inventory_ui.demo_menu = Some(id);
                    }
                } else if !configured_view {
                    self.inventory_ui.show_original(&self.world.inventory, self.world.inventory.root);
                }
                for cmd in
                    crate::world::appearance::sync_commands(&self.world.inventory, self.world.agent_id, &self.world.worn_attachment_items())
                {
                    self.send(cmd);
                }
            }
            if std::env::var_os("AURORA_DEMO_ACTIONS").is_some() {
                for ev in crate::demo::action_events() {
                    self.world.apply(ev);
                }
                self.settings.audio.media_autoplay = false;
                if let Some(parcel) = self.world.parcel.as_mut() {
                    let p = Arc::make_mut(parcel);
                    p.media_url = "https://example.invalid/aurora-media".into();
                    p.media.current_url = crate::demo::demo_media_page().into();
                    p.media.mime = "text/html".into();
                    p.media.media_id = crate::demo::ACTION_MEDIA_TEX;
                    p.media.auto_scale = true;
                }
            }
            if std::env::var_os("AURORA_DEMO_ANIMESH").is_some() {
                if let Some(g) = &mut self.gfx {
                    self.scene.install_demo_animesh(&mut g.renderer);
                }
                for ev in crate::demo::animesh_events() {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_CLASSIC: classic avatars, shaped and baked
            if std::env::var_os("AURORA_DEMO_CLASSIC").is_some() {
                for ev in crate::demo::classic_events(&self.world.avatar_lib.shape_params) {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_CLOUD=self: our avatar with an invisible head bake,
            // wearing a mesh that cannot be downloaded (issue #121)
            if std::env::var("AURORA_DEMO_CLOUD").is_ok_and(|v| v == "self") {
                self.scene.meshes.insert_demo_unavailable(crate::demo::CLOUD_MESH);
                for ev in crate::demo::cloud_self_events() {
                    self.world.apply(ev);
                }
            }
            self.stream_demo = crate::demo::stream::StreamDemo::from_env();
            self.tex_refresh_demo = crate::demo::tex_refresh::TexRefreshDemo::from_env();
            self.experience_demo = crate::demo::experience::ExperienceDemo::from_env();
            self.geometry_demo = crate::demo::geometry::GeometryDemo::from_env();
            if let Some(n) = crate::demo::texture_stress_count() {
                if let Some(g) = &mut self.gfx {
                    self.scene.textures.install_demo_stress(&mut g.renderer, 0..n);
                }
                for ev in crate::demo::texture_stress_events(n) {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_CROWD=<n>: avatars wearing attachments that follow
            // their animated bones (scene sync stress test)
            if let Some(n) = crate::demo::crowd_count() {
                let rigged = std::env::var_os("AURORA_DEMO_ANIMESH").is_some();
                // (<n>,<worn>: dancers dressed in rigged meshes)
                if let Some((_, triangles)) = crate::demo::crowd_worn().filter(|_| rigged) {
                    if let Some(g) = &mut self.gfx {
                        self.scene
                            .install_demo_rigged_mesh(&mut g.renderer, crate::demo::CROWD_DENSE_MESH, Some(triangles));
                    }
                    let rig = self.scene.avatar_lib.rig.clone();
                    self.scene.anims.insert(
                        crate::demo::CROWD_DANCE_ANIM,
                        Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::crowd_dance_animation(), &rig)),
                    );
                }
                for ev in crate::demo::crowd_events(n, rigged) {
                    self.world.apply(ev);
                }
                // (…,busy: flexible and spinning attachments, a large HUD,
                // spinning objects on the ground)
                if crate::demo::crowd_busy() {
                    for ev in crate::demo::crowd_busy_events(n) {
                        self.world.apply(ev);
                    }
                }
            }
            // AURORA_DEMO_LIGHTS=<n>: small local lights among the crowd
            if let Some(n) = crate::demo::lights_count() {
                for ev in crate::demo::lights_events(n) {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_HAIR=<layers>[,<segments>[,<empty %>[,<period>]]]: layered
            // blended shells on our avatar and on the crowd (blended pass load)
            if let Some(hair) = crate::demo::hair_demo() {
                if let Some(g) = &mut self.gfx {
                    self.scene.install_demo_hair(&mut g.renderer, hair);
                }
                for ev in crate::demo::hair_events(hair, crate::demo::crowd_count().unwrap_or(0)) {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_GLYPHS=1|2: residents named and talking in the
            // characters of the grid's display names (system fallback fonts)
            if let Some(reversed) = crate::demo::glyphs::scenario() {
                for ev in crate::demo::glyphs::events(reversed) {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
                self.panels.chat = true;
                self.panels.people = false;
                self.panels.inventory = false;
                self.panels.minimap = false;
                self.panels.perf = false;
            }
            // AURORA_DEMO_EMOJI_PICKER=1|<text>: the conversation window open
            // on its emoji picker (colour emoji font); a text is put in the
            // input field, as if picked
            if let Ok(text) = std::env::var("AURORA_DEMO_EMOJI_PICKER") {
                self.panels.chat = true;
                self.chat_ui.emoji_open = true;
                if text.trim() != "1" {
                    self.chat_ui.conv_input = text;
                }
            }
            // AURORA_DEMO_SOUND=1: a looped chime on the fountain and the
            // interface sounds (audible: off by default for the captures)
            if std::env::var_os("AURORA_DEMO_SOUND").is_some() {
                let (id, clip) = crate::demo::chime();
                self.scene.sounds.insert(id, clip);
                for id in self.settings.audio.ui.preload() {
                    self.scene.sounds.insert(id, crate::demo::ui_sound(id));
                }
                for ev in crate::demo::sound_events(id) {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
            }
            // AURORA_DEMO_MEDIA=<url> (1: a test page): a screen with media
            // on a prim east of the start position, playing at once
            if let Some(v) = std::env::var_os("AURORA_DEMO_MEDIA") {
                let v = v.to_string_lossy().into_owned();
                let url = if v == "1" || v.is_empty() {
                    crate::demo::demo_media_page().to_string()
                } else {
                    v
                };
                let (ev, object, face) = crate::demo::media_screen();
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
                self.media.demo_entry(object, face, &url);
            }
            // AURORA_DEMO_WORN: an object worn on our avatar's back
            if std::env::var_os("AURORA_DEMO_WORN").is_some() {
                for ev in crate::demo::worn::events() {
                    self.world.apply(ev);
                }
                crate::demo::worn::seed_inventory(&mut self.world);
            }
            // HUD fixtures exercise screen-space rendering and the same
            // linked-object touch / inventory paths as real attachments.
            if let Ok(mode) = std::env::var("AURORA_DEMO_HUDS") {
                let faces = matches!(mode.as_str(), "faces" | "faces-turned");
                if faces {
                    for (id, material) in crate::demo::hud::face_materials() {
                        self.scene.materials.insert(id, material);
                    }
                }
                let events = if faces {
                    crate::demo::hud::face_events(mode == "faces-turned")
                } else {
                    crate::demo::hud::events()
                };
                for ev in events {
                    self.world.apply(ev);
                }
                self.settings.show_huds = mode != "hidden";
                self.panels.inventory = false;
                self.panels.perf = false;
                self.panels.chat = false;
                self.panels.appearance = false;
                self.panels.settings = false;
                self.world.notifications.list.clear();
                crate::demo::hud::seed_inventory(&mut self.world);
                self.world.hud_scales = crate::world::hud_scale::HudScales::load(
                    crate::settings::account_dir(&self.world.agent_id).join(crate::world::hud_scale::FILE),
                );
                if matches!(
                    mode.as_str(),
                    "scale-reattach" | "scale-rename" | "scale-copy" | "scale-temp" | "scale-temp-item" | "scale-temp-instance"
                ) {
                    // Deterministic persistence regressions, independent of
                    // previously saved demo profiles. Disk saves have unit tests.
                    self.world.hud_scales = crate::world::hud_scale::HudScales::default();
                    let id = crate::demo::hud::object(31, false).full_id;
                    if mode == "scale-reattach"
                        && let Some(item) = self.world.inventory.items.get_mut(&id)
                    {
                        item.creator = uuid::Uuid::nil();
                    }
                    if matches!(mode.as_str(), "scale-temp" | "scale-temp-item" | "scale-temp-instance") {
                        self.world.inventory.items.remove(&id);
                        self.world.inventory.items.remove(&uuid::Uuid::from_u128(8399));
                        if let Some(root) = self.world.objects.index_of_uuid(&id)
                            && let Some(o) = self.world.objects.get_mut(root)
                        {
                            o.name_values = if mode == "scale-temp-item" {
                                format!("AttachItemID STRING RW SV {}", uuid::Uuid::from_u128(999_001))
                            } else if mode == "scale-temp-instance" {
                                format!("AttachItemID STRING RW SV {id}")
                            } else {
                                String::new()
                            };
                        }
                    }
                }
                if matches!(mode.as_str(), "zoom" | "edit-zoom" | "drag-zoom") {
                    self.world.hud_zoom = 0.5;
                }
                if mode.starts_with("drag") || mode.starts_with("scale") || mode == "edit-scroll" {
                    let code = match mode.as_str() {
                        "drag-secondary" | "scale-secondary" => KeyCode::AltLeft,
                        "drag-key" | "scale-key" => KeyCode::KeyB,
                        _ => KeyCode::ControlLeft,
                    };
                    self.settings.keybinds.set(
                        Action::HudDrag,
                        0,
                        Some(crate::keybinds::Binding::captured(Input::key(code), Mods::default())),
                    );
                    let secondary = matches!(mode.as_str(), "drag-secondary" | "scale-secondary")
                        .then(|| crate::keybinds::Binding::captured(Input::key(KeyCode::ControlLeft), Mods::default()));
                    self.settings.keybinds.set(Action::HudDrag, 1, secondary);
                }
                if mode == "drag-ignore" {
                    let idx = self
                        .world
                        .objects
                        .iter()
                        .find(|(_, o)| o.key.local_id == crate::demo::hud::CHILD)
                        .map(|(idx, _)| idx);
                    if let Some(o) = idx.and_then(|idx| self.world.objects.get_mut(idx)) {
                        o.click_action = crate::interaction::code::IGNORE;
                    }
                }
                if mode == "media" {
                    let mut hud = crate::demo::hud::object(35, false);
                    let face = crate::demo::media_screen().2;
                    hud.scale = Vec3::new(0.012, 0.7, 0.394);
                    if let Some(te) = hud.texture_entry.as_mut() {
                        let tf = &mut Arc::make_mut(te).faces[face as usize];
                        tf.texture = aurora_prim::te::BLANK_TEXTURE;
                        tf.color = [1.0; 4];
                        tf.scale_t = 576.0 / 1024.0;
                        tf.offset_t = -(1.0 - tf.scale_t) * 0.5;
                    }
                    self.media.demo_entry(hud.full_id, face, crate::demo::demo_media_page());
                    self.world.apply(NetEvent::ObjectUpdates {
                        handle: self.world.main_region.unwrap_or_default(),
                        objects: vec![hud],
                    });
                }
            }
            // AURORA_DEMO_PARCEL_MEDIA=<url> (1: the test page): parcel media
            // on the first test panel's texture, started at once.
            if let Some(v) = std::env::var_os("AURORA_DEMO_PARCEL_MEDIA") {
                let v = v.to_string_lossy().into_owned();
                let url = if v == "1" || v.is_empty() {
                    crate::demo::demo_media_page().to_string()
                } else {
                    v
                };
                if let Some(parcel) = self.world.parcel.as_mut() {
                    let pc = Arc::make_mut(parcel);
                    pc.media_url = url;
                    pc.media.mime = "text/html".into();
                    pc.media.media_id = crate::demo::parcel_media_texture();
                    pc.media.auto_scale = true;
                }
                self.media.play_parcel(&self.world, &self.settings.media);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(g) = self.gfx.as_mut() else {
            return;
        };
        let resp = g.egui_state.on_window_event(&g.window, &event);
        match &event {
            WindowEvent::CloseRequested => {
                self.request_exit();
            }
            WindowEvent::Resized(s) => {
                g.renderer.resize(s.width, s.height);
                self.geom_changed = Some(Instant::now());
            }
            WindowEvent::Moved(_) => self.geom_changed = Some(Instant::now()),
            WindowEvent::RedrawRequested => {
                self.frame(event_loop);
            }
            WindowEvent::ModifiersChanged(m) => {
                self.shift = m.state().shift_key();
                self.ctrl = m.state().control_key();
                self.alt = m.state().alt_key();
                if !self.ctrl || self.alt {
                    self.build.reset_selection_wheel();
                }
            }
            WindowEvent::Focused(true) => {
                self.focused = true;
                if !self.demo
                    && matches!(self.screen, Screen::Login)
                    && !self.login_form.editing
                    && !self.login_form.busy
                    && self.login_form.transition.is_none()
                {
                    self.login_form.accounts_loaded = false;
                    self.login_form.mfa_for = None;
                    self.login_form.stored = None;
                    self.login_form.stored_for = None;
                }
            }
            WindowEvent::Focused(false) => {
                self.gesture_state.manager.release(None);
                self.numeric_drag.cancel();
                self.build.reset_selection_wheel();
                self.focused = false;
                self.down.clear();
                self.alt = false;
                self.ctrl = false;
                self.shift = false;
                self.right_drag = false;
                self.mic_button_held = false;
                self.drop_clicks();
                self.on_left_release();
                // LLViewerWindow::handleFocusLost: the cursor shown and free
                // again, also in mouselook (taken back with the focus)
                self.release_cursor(true);
                if self.temp_run.take().is_some() && !self.always_run {
                    self.send(NetCommand::SetAlwaysRun(false));
                }
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                self.last_input = Instant::now();
                if self.exit_prompt.open {
                    return;
                }
                if let PhysicalKey::Code(code) = ke.physical_key {
                    let pressed = ke.state == ElementState::Pressed;
                    // The color dialog owns Escape before camera shortcuts.
                    // egui already received the event above.
                    let eyedropper = ui::widgets::color::pipette::armed(&self.egui_ctx);
                    if pressed
                        && code == KeyCode::Escape
                        && (self.build.ui.color_picker.is_open() || self.script_editors.color_open() || eyedropper)
                    {
                        return;
                    }
                    // learn the keyboard layout for the shortcut labels
                    if pressed
                        && !self.shift
                        && !self.ctrl
                        && !self.alt
                        && let winit::keyboard::Key::Character(s) = &ke.logical_key
                        && !format!("{code:?}").starts_with("Digit")
                        && !format!("{code:?}").starts_with(aurora_i18n::tr!("common-numpad"))
                    {
                        crate::keybinds::learn(code, s);
                    }
                    // shortcut being captured in the preferences
                    if self.options_ui.capture.is_some() {
                        if let Some(input) = self.options_ui.capture_key(code, pressed, ke.repeat) {
                            self.finish_capture(input);
                        }
                        return;
                    }
                    // A gesture key must still be held when the chat or a web
                    // page has keyboard focus; it only owns a visible HUD drag.
                    let input = Input::key(code);
                    if self.settings.keybinds.uses_input(Action::HudDrag, &input) {
                        if pressed {
                            self.down.insert(input);
                        } else {
                            self.down.remove(&input);
                        }
                    }
                    // Releases always go through so keys never get stuck.
                    let typing = self.egui_ctx.egui_wants_keyboard_input();
                    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
                    let gesture_key = crate::gestures::keys::from_key(code, &ke.key_without_modifiers());
                    if !pressed && let Some(key) = gesture_key {
                        self.gesture_state.manager.release(Some(key));
                    }
                    if pressed
                        && !ke.repeat
                        && !typing
                        && self.in_world()
                        && !self.alt
                        && self.profile_ui.focused_web().is_none()
                        && self.search_ui.focused_web().is_none()
                        && let Some(key) = gesture_key
                        && let Some(id) = self
                            .gesture_state
                            .manager
                            .key_trigger(key, u32::from(self.ctrl) | if self.shift { 4 } else { 0 })
                    {
                        self.gesture_play(id, Some(key));
                        return;
                    }
                    // a profile's web page that was clicked gets the keyboard
                    let web = match self.profile_ui.focused_web() {
                        Some(p) => Some(p),
                        None => self.search_ui.focused_web(),
                    };
                    if !typing && let Some(p) = web {
                        use winit::platform::scancode::PhysicalKeyExtScancode;
                        let scancode = ke.physical_key.to_scancode().unwrap_or(0);
                        let mods = aurora_media::Modifiers {
                            control: self.ctrl,
                            alt: self.alt,
                            shift: self.shift,
                        };
                        if crate::media::keys::forward(p, code, scancode, pressed, ke.repeat, ke.text.as_deref(), mods) || pressed {
                            return;
                        }
                    }
                    // a focused media page gets the keyboard (LLViewerMediaFocus)
                    if self.media.focus.is_some() && !typing && (!resp.consumed || !pressed) {
                        use winit::platform::scancode::PhysicalKeyExtScancode;
                        let scancode = ke.physical_key.to_scancode().unwrap_or(0);
                        let mods = aurora_media::Modifiers {
                            control: self.ctrl,
                            alt: self.alt,
                            shift: self.shift,
                        };
                        if self.media.on_key(code, scancode, pressed, ke.repeat, ke.text.as_deref(), mods) && pressed {
                            return;
                        }
                    }
                    // Ctrl / Alt shortcuts of the viewer still work while a text
                    // field has the focus (chat bar...), as Firestorm's menu
                    // accelerators go before the focused field
                    // (LLViewerWindow::handleKey); the field keeps its own
                    // editing shortcuts (copy, paste, undo, words...)
                    let shortcut = pressed
                        && typing
                        && (self.ctrl || self.alt)
                        && !crate::keybinds::is_text_edit_key(code)
                        && self.settings.keybinds.triggered(&Input::key(code), self.mods()).is_some();
                    // Ctrl+F belongs to the window with its own find when the
                    // keyboard is in it (script editor, conversation), or the
                    // pointer over a conversation with the keyboard nowhere:
                    // the viewer's shortcut on these keys ("Search")
                    // only runs otherwise. The rule: ui::find_shortcut::owner.
                    // The key is the one egui reads, which the window takes.
                    let find_key = matches!(&ke.logical_key, winit::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("f"));
                    let find_taken = pressed && self.ctrl && find_key && ui::find_shortcut::window_takes_it(&self.egui_ctx);
                    if (!pressed || (!resp.consumed && !typing) || shortcut) && !(pressed && ke.repeat) && !find_taken {
                        self.on_input(Input::key(code), pressed);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.last_input = Instant::now();
                if self.exit_prompt.open {
                    return;
                }
                let pressed = *state == ElementState::Pressed;
                let over_ui = self.egui_ctx.is_pointer_over_egui() || self.egui_ctx.egui_wants_pointer_input();
                // shortcut being captured: middle / side buttons bind, left / right cancel
                if self.options_ui.capture.is_some() {
                    if pressed {
                        self.finish_capture(Input::mouse(*button));
                    }
                    return;
                }
                // (in its turn behind a click that waits for its depth)
                self.mouse_input(*button, pressed, over_ui);
            }
            WindowEvent::MouseWheel { delta, .. } if !self.exit_prompt.open && !self.egui_ctx.is_pointer_over_egui() && self.in_world() => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
                if self.on_build_scroll(steps) || self.on_hud_scroll(steps) {
                    return;
                }
                let mods = aurora_media::Modifiers {
                    control: self.ctrl,
                    alt: self.alt,
                    shift: self.shift,
                };
                let ray = if self.media.focus.is_some() {
                    self.gfx
                        .as_ref()
                        .and_then(|g| g.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1))
                } else {
                    None
                };
                let on_media = match ray {
                    Some(ray) => {
                        let (ray, depth, hud) = self
                            .scene
                            .hud_pick(&self.world, self.cursor_pos, true)
                            .map_or((ray, None, false), |(_, p, r)| (r, Some((p - r.0).dot(r.1)), true));
                        self.media.on_scroll(&self.world, &self.scene, ray, depth, steps, mods, hud)
                    }
                    None => false,
                };
                // (the free camera has no zoom: the wheel would change the
                // avatar's view behind it, or enter mouselook)
                if !on_media && self.camera.free().is_none() {
                    // SL wheel clicks: positive zooms out
                    let was = self.camera.mouselook();
                    self.camera
                        .scroll(-steps, self.ctrl, self.shift, &mut self.world.agent, &mut self.settings.camera);
                    if was != self.camera.mouselook() {
                        self.set_mouselook_grab(!was);
                    }
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.numeric_drag.active() {
                if let Some(g) = &mut self.gfx {
                    g.egui_state.on_mouse_motion(delta);
                }
                return;
            }
            if !self.in_world() || self.exit_prompt.open {
                return;
            }
            let dy = if self.settings.invert_mouse { -delta.1 } else { delta.1 };
            self.camera_mouse_delta(delta.0 as f32, dy as f32);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.record_window_geometry();
        if self.gfx.as_ref().is_some_and(|g| !g.shown) {
            // Hidden windows may receive no RedrawRequested event. Keep
            // producing frames until a real presentation makes it visible,
            // including when the render thread returns the previous frame.
            self.frame(event_loop);
        } else if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }
}

/// Frames per second the frame limiter holds the loop to (0 = none): the
/// user cap, the lower background cap while the window is not focused, and
/// the screen rate outside the world. The login screen draws an empty scene:
/// left free it runs at thousands of frames per second, which floods the
/// desktop compositor and makes the whole machine stutter on some setups.
/// `timed` runs (AURORA_CAPTURE, AURORA_PROFILE) count or measure frames,
/// often in a window that never had the focus: no background cap there.
fn frame_cap(s: &Settings, focused: bool, in_world: bool, monitor_hz: f32, timed: bool, fps_limit: Option<u32>) -> f32 {
    let mut cap = if s.fps_cap { s.fps_limit as f32 } else { f32::INFINITY };
    if let Some(limit) = fps_limit {
        cap = cap.min(limit as f32);
    }
    if !focused && s.background_fps_cap && !timed {
        cap = cap.min(s.background_fps_limit as f32);
    }
    if !in_world {
        cap = cap.min(monitor_hz);
    }
    if cap.is_finite() { cap } else { 0.0 }
}

/// Name of the swapchain's presentation mode for the profile and the bug
/// reports; `unknown` until the renderer has answered a frame.
fn present_mode_name(mode: Option<aurora_render::wgpu::PresentMode>) -> &'static str {
    mode.map_or("unknown", aurora_render::present::name)
}

/// AURORA_CAPTURE files: one per frame of AURORA_CAPTURE_FRAMES (comma
/// separated, default 240), in frame order; with several frames each file is
/// named `<name>-<frame>.<ext>`.
fn capture_files(path: &std::path::Path, frames: &str) -> Vec<(u64, std::path::PathBuf)> {
    let mut at: Vec<u64> = frames.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    if at.is_empty() {
        at.push(240);
    }
    at.sort_unstable();
    at.dedup();
    if at.len() == 1 {
        return vec![(at[0], path.to_path_buf())];
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "png".into());
    at.into_iter()
        .map(|f| (f, path.with_file_name(format!("{stem}-{f}.{ext}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{RightGesture, capture_files, frame_cap};
    use crate::settings::Settings;
    use std::path::{Path, PathBuf};

    #[test]
    fn right_click_tolerates_small_straight_and_diagonal_motion() {
        for (dx, dy) in [(7.0, 0.0), (0.0, -7.0), (6.0, 6.0)] {
            let mut gesture = RightGesture::default();
            assert!(!gesture.motion(dx, dy, 1.0));
            assert!(!gesture.orbit);
        }
    }

    #[test]
    fn right_click_jitter_does_not_accumulate_travel() {
        let mut gesture = RightGesture::default();
        for _ in 0..100 {
            assert!(!gesture.motion(2.0, -1.0, 1.0));
            assert!(!gesture.motion(-2.0, 1.0, 1.0));
        }
        assert!(!gesture.orbit);
    }

    #[test]
    fn right_orbit_starts_even_with_slow_motion_and_stays_active_on_return() {
        let mut gesture = RightGesture::default();
        for _ in 0..9 {
            assert!(!gesture.motion(1.0, 0.0, 1.0));
        }
        assert!(gesture.motion(1.0, 0.0, 1.0));
        assert!(gesture.motion(-10.0, 0.0, 1.0));
        assert!(gesture.motion(0.25, -0.25, 1.0));
        assert!(gesture.orbit);
    }

    #[test]
    fn right_orbit_threshold_scales_with_the_interface() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut gesture = RightGesture::default();
            assert!(!gesture.motion(6.0 * scale, 6.0 * scale, scale));
            assert!(gesture.motion(0.0, 2.0 * scale, scale));
        }
    }

    #[test]
    fn frame_cap_rules() {
        let free = Settings {
            fps_cap: false,
            background_fps_cap: false,
            ..Settings::default()
        };
        // in world: free unless the user asks for a cap
        assert_eq!(frame_cap(&free, true, true, 144.0, false, None), 0.0);
        assert_eq!(frame_cap(&free, false, true, 144.0, false, None), 0.0);
        // outside the world (login): never above the screen rate
        assert_eq!(frame_cap(&free, true, false, 144.0, false, None), 144.0);
        let capped = Settings {
            fps_cap: true,
            fps_limit: 90,
            background_fps_cap: true,
            background_fps_limit: 15,
            ..Settings::default()
        };
        assert_eq!(frame_cap(&capped, true, true, 144.0, false, None), 90.0);
        assert_eq!(frame_cap(&capped, true, false, 144.0, false, None), 90.0);
        assert_eq!(frame_cap(&capped, true, false, 60.0, false, None), 60.0);
        // not focused: the lower of the two caps
        assert_eq!(frame_cap(&capped, false, true, 144.0, false, None), 15.0);
        assert_eq!(frame_cap(&capped, false, false, 144.0, false, None), 15.0);
        // captures and profiles keep their pace without the focus
        assert_eq!(frame_cap(&capped, false, true, 144.0, true, None), 90.0);
        assert_eq!(frame_cap(&free, false, false, 144.0, true, None), 144.0);
        assert_eq!(frame_cap(&free, true, true, 144.0, true, None), 0.0);
        assert_eq!(frame_cap(&free, true, true, 144.0, false, Some(500)), 500.0);
        // A process cap survives preferences, capture mode and loss of focus.
        for focused in [false, true] {
            for in_world in [false, true] {
                for timed in [false, true] {
                    assert_eq!(frame_cap(&free, focused, in_world, 144.0, timed, Some(60)), 60.0);
                    assert!(frame_cap(&capped, focused, in_world, 144.0, timed, Some(60)) <= 60.0);
                }
            }
        }
        assert_eq!(frame_cap(&capped, true, true, 144.0, false, Some(30)), 30.0);
    }

    #[test]
    fn capture_files_by_frame() {
        assert_eq!(capture_files(Path::new("c/a.png"), ""), vec![(240, PathBuf::from("c/a.png"))]);
        assert_eq!(capture_files(Path::new("c/a.png"), "620"), vec![(620, PathBuf::from("c/a.png"))]);
        assert_eq!(
            capture_files(Path::new("c/a.png"), "2560, 2500"),
            vec![(2500, PathBuf::from("c/a-2500.png")), (2560, PathBuf::from("c/a-2560.png"))]
        );
    }
}
