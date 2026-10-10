//! Application: window, event loop, screens (login / loading / world).

use crate::agent::MoveInput;
use crate::camera::Camera;
use crate::frame_profile::{FrameProfile, Lap, SceneCounts};
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
use aurora_render::{DrawLists, EguiFrame, FrameParams, PointLight, RenderStats, Renderer};
use glam::{Vec3, Vec4};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

mod chat_commands;
mod context_menu;
mod hud_drag;
mod inventory;
mod inventory_thumbnail;
mod object_actions;

enum Screen {
    Login,
    Loading { since: Instant },
    World,
}

struct Gfx {
    window: Arc<Window>,
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
    login_progress: Option<(String, f32)>,
    panels: Panels,
    chat_ui: ChatUi,
    perf: PerfData,
    /// AURORA_PROFILE: frame laps and the summary line in the log.
    frame_profile: FrameProfile,
    last_frame: Instant,
    start: Instant,
    right_drag: bool,
    cursor_grabbed: bool,
    alt: bool,
    left_down: bool,
    /// Mouse travel while the right button is held (click vs orbit drag).
    right_moved: f32,
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
    /// "Toujours courir" (Ctrl+R).
    always_run: bool,
    /// Microphone switched on (toggle mode).
    mic_on: bool,
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
    /// Look-at: setting last frame, camera focus, cursor and body heading
    /// (LLAgentCamera::updateLookAt).
    look_at_was_on: bool,
    /// Offline demo: eye tracking on without touching the saved settings.
    demo_look_at: bool,
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
    loading_stage: String,
    move_complete_at: Option<Instant>,
    /// Teleport diagnostics still to log (seconds after the arrival).
    tp_checks: Vec<f32>,
    /// Debug: culling view kept while « Figer le culling » is on.
    frozen_cull: Option<CullView>,
    /// Interface sounds caused by the widgets (clicks, keys, windows).
    sound_cues: ui::sound_cues::SoundCues,
    /// Last frame: microphone on, context menu open, interactive script
    /// dialogs (their sounds).
    mic_was: bool,
    context_menu_was: bool,
    /// « Supprimer » waiting for a yes: (object, why it asks).
    delete_confirm: Option<(uuid::Uuid, String)>,
    /// Teleport asked by a place link, waiting for a yes (TeleportViaSLAPP).
    place_confirm: Option<(String, Vec3)>,
    /// External web link waiting for the warning's answer, with the state of
    /// its « Ne plus me prévenir » box and, for a dangerous link, the reason.
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
    monitor_hz: f32,
    /// The window has the keyboard focus (background frame cap).
    focused: bool,
    /// Loading / teleport screen background (last view, blurred).
    backdrop: ui::backdrop::Backdrop,
    /// Capture the 3D view on the next frame; a capture is on its way.
    want_scene_capture: bool,
    scene_capture_pending: bool,
    /// Closing the window: waiting for the last view capture (since).
    closing: Option<Instant>,
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
    /// Destination of the teleport being started (teleport screen).
    tp_dest: Option<String>,
    /// Window moved / resized: its normal geometry is recorded once settled.
    geom_changed: Option<Instant>,
    frame_start: Instant,
    people_ui: ui::people::PeopleUi,
    contacts_ui: ui::contacts::ContactsUi,
    /// Mini-map and world map state, map server tiles.
    minimap_ui: ui::minimap::MiniMap,
    display_name_ui: ui::display_name::DisplayNameUi,
    people_map_ui: ui::minimap::MiniMap,
    worldmap_ui: ui::worldmap::WorldMapUi,
    map_tiles: ui::map_tiles::MapTiles,
    options_ui: ui::options::OptionsUi,
    audio_ui: ui::audio::AudioUi,
    notif_ui: ui::notifications::NotifUi,
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
    /// Voice dots above the avatars (wave animation state).
    voice_dots: ui::voice_dot::VoiceDots,
    /// Avatar name tags on screen last frame, far first: a click on one
    /// counts as one on its avatar.
    name_tags: Vec<ui::hud::NameTag>,
    /// Block list version applied to object sounds, and the objects whose
    /// sounds it silenced.
    sound_blocks: (u64, std::collections::HashSet<uuid::Uuid>),
    /// Audio devices listed for the preferences.
    devices_listed: bool,
    /// Color emoji (Noto 3D) for chat and the picker.
    emoji: ui::emoji::Emoji,
    /// Profile pictures by avatar id (egui textures).
    avatar_pics: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    /// Other interface images by asset id (profiles: 1st life picture,
    /// group insignias).
    ui_images: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    /// Avatar profile windows.
    profile_ui: ui::profile::ProfileUi,
    /// "Détails de l'emplacement" windows (standalone place profiles).
    place_ui: ui::place_details::PlaceDetailsUi,
    /// « Lieux ».
    places_ui: ui::places::PlacesUi,
    /// AURORA_DEMO_PLACE=repere|historique, opened once logged in.
    demo_place: Option<crate::world::place_details::Source>,
    /// AURORA_DEMO_STREAM: objects and textures arriving in waves.
    stream_demo: Option<crate::demo::stream::StreamDemo>,
    /// Parcel snapshots and group names the place profiles want.
    place_images: std::collections::HashSet<uuid::Uuid>,
    place_groups: std::collections::HashSet<uuid::Uuid>,
    /// "À propos du terrain".
    land_ui: ui::land::LandUi,
    /// Environment selector and « Éclairage personnel ».
    env_ui: ui::environment::EnvironmentUi,
    /// The selector's folder fetches started (the selector was opened).
    env_scan: bool,
    /// « Éclairage personnel » was open last frame (captures on opening).
    lighting_was_open: bool,
    demo: bool,
    inventory_ui: ui::inventory::InventoryUi,
    inventory_images: inventory_thumbnail::ImageJobs,
    appearance_ui: ui::appearance::AppearanceUi,
    last_social_poll: Instant,
    /// Last keyboard / mouse button input (automatic away, AFKTimeout), and
    /// whether the current away status was set by that timer.
    last_input: Instant,
    auto_away: bool,
    /// AURORA_CAPTURE: file, frames to capture (several: one file each,
    /// `<name>-<frame>.png`), quit after the last one.
    capture: Option<(Vec<(u64, std::path::PathBuf)>, bool)>,
}

impl App {
    pub fn new() -> anyhow::Result<App> {
        let mut settings = Settings::load();
        if std::env::var_os("AURORA_DEMO").is_some() && std::env::var_os("AURORA_DEMO_KEYBOARD").is_some() {
            // Exercise a first launch without changing the user's bindings.
            settings.keybinds = Default::default();
        }
        // test overrides (used by automated captures)
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
        if let Ok(v) = std::env::var("AURORA_OCCLUSION") {
            settings.occlusion_culling = v != "0";
        }
        let net = NetClient::new()?;
        let voice_http = net.caps_http();
        let login_info = ui::news::LoginInfo::start(net.runtime());
        let out_device = (!settings.audio.output_device.is_empty()).then(|| settings.audio.output_device.clone());
        let lib = Arc::new(AvatarLibrary::load());
        let cache = crate::settings::cache_dir();
        let disk_cache = crate::cache::DiskCache::new(cache.clone(), settings.cache_size_mb);
        net.set_object_cache_dir(Some(cache.join("objects")));
        let cache_root = cache.clone();
        let egui_ctx = egui::Context::default();
        ui::fonts::apply(&egui_ctx, &settings.font);
        ui::colors::set(&settings.colors);
        let skin = Skin::load(&egui_ctx, "aurora", settings.font_scale);
        let palette = skin.theme.palette();
        egui_ctx.set_zoom_factor(settings.ui_scale);
        // last view of the previous session for the loading screen
        let mut backdrop = ui::backdrop::Backdrop::default();
        if settings.loading_backdrop && std::env::var_os("AURORA_CAPTURE").is_none() {
            backdrop.load_saved();
        }
        // floater positions / sizes from the last session (not for captures)
        if std::env::var_os("AURORA_CAPTURE").is_none() {
            ui::layout::load(&egui_ctx);
        }

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

        let panels = Panels {
            chat: settings.show_chat,
            perf: settings.show_perf,
            people: settings.show_people,
            minimap: settings.show_minimap,
            world_map: false,
            settings: false,
            time_of_day: settings.time_of_day,
            environment: false,
            personal_lighting: false,
            inventory: settings.show_inventory,
            appearance: settings.show_appearance,
            people_tab: settings.people_tab.min(1),
            contacts: false,
            about_land: false,
            places: false,
            nav_edit: None,
            nav_edit_new: false,
        };
        let mut scene = Scene::new(cache, lib.clone());
        scene.draw_distance = settings.draw_distance;
        scene.lod_factor = settings.lod_factor;
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
            login_progress: None,
            panels,
            chat_ui: ChatUi::default(),
            perf: PerfData::default(),
            frame_profile: FrameProfile::default(),
            last_frame: Instant::now(),
            start: Instant::now(),
            right_drag: false,
            cursor_grabbed: false,
            alt: false,
            left_down: false,
            right_moved: 0.0,
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
            mic_on: false,
            mic_button_held: false,
            voice_connected: false,
            voice_level: 0.0,
            fb_since: None,
            turn_since: None,
            cam_key_since: [None; 10],
            look_at_was_on: false,
            demo_look_at: false,
            look_focus: None,
            look_cursor: (0.0, 0.0),
            look_root_at: Vec3::X,
            lr_since: None,
            up_since: None,
            last_vertical_flags: 0,
            last_render: RenderStats::default(),
            empty_lists: DrawLists::default(),
            quit_at: None,
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
            monitor_hz: 60.0,
            focused: true,
            backdrop,
            want_scene_capture: false,
            scene_capture_pending: false,
            closing: None,
            tp_overlay: None,
            was_teleporting: false,
            demo_tp_start: None,
            demo_tp_triggered: false,
            demo_mmo_start: None,
            demo_tp_dest: None,
            tp_dest: None,
            geom_changed: None,
            frame_start: Instant::now(),
            people_ui: Default::default(),
            contacts_ui: Default::default(),
            minimap_ui: Default::default(),
            display_name_ui: Default::default(),
            people_map_ui: Default::default(),
            worldmap_ui: Default::default(),
            map_tiles: ui::map_tiles::MapTiles::new(),
            options_ui: Default::default(),
            audio_ui: Default::default(),
            notif_ui: Default::default(),
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
            media: Default::default(),
            media_ui: Default::default(),
            media_cursor: None,
            voice: Default::default(),
            voice_http,
            login_info,
            voice_dots: Default::default(),
            name_tags: Vec::new(),
            sound_blocks: Default::default(),
            devices_listed: false,
            emoji: ui::emoji::Emoji::load(),
            avatar_pics: Default::default(),
            ui_images: Default::default(),
            profile_ui: Default::default(),
            place_ui: Default::default(),
            places_ui: Default::default(),
            place_images: Default::default(),
            demo_place: None,
            stream_demo: None,
            place_groups: Default::default(),
            land_ui: Default::default(),
            env_ui: Default::default(),
            env_scan: false,
            lighting_was_open: false,
            demo: false,
            inventory_ui: Default::default(),
            inventory_images: Default::default(),
            appearance_ui: Default::default(),
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

    fn send(&mut self, cmd: NetCommand) {
        if self.demo {
            match &cmd {
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
                    self.world.system_message("Démo : touch_start simulé.");
                }
                NetCommand::ObjectGrabUpdate { surface, .. } => {
                    log::info!("demo touch / grab update: face={}, uv={:?}", surface.face, surface.uv);
                }
                NetCommand::ObjectRelease { local_id, surface, .. } => {
                    log::info!("demo touch_end: prim={local_id}, face={}", surface.face);
                    self.world.system_message("Démo : touch_end simulé.");
                }
                _ => {}
            }
            let expense = match &cmd {
                NetCommand::BuyObject { price, .. } => Some((*price, "achat")),
                NetCommand::PayObject { amount, .. } => Some((*amount, "paiement")),
                _ => None,
            };
            if let Some((amount, kind)) = expense {
                log::info!("demo action: {kind} L$ {amount}");
                let balance = self.world.balance.unwrap_or(250).saturating_sub(amount);
                self.world.apply(NetEvent::Balance(balance));
                self.world.system_message(format!("Démo : {kind} de L$ {amount} simulé."));
            }
            if matches!(&cmd, NetCommand::OneShotControl(f) if f & control::STAND_UP != 0)
                && std::env::var_os("AURORA_DEMO_ACTIONS").is_some()
            {
                self.world.apply(NetEvent::ObjectUpdates {
                    handle: self.world.main_region.unwrap_or_default(),
                    objects: vec![crate::demo::action_avatar(false)],
                });
            }
            if let NetCommand::Build(b) = &cmd {
                for ev in self.build_demo.reply(&self.world, b) {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
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
            if let NetCommand::CreateInventoryWearable { parent, kind, name, .. } = &cmd {
                self.send(NetCommand::CreateInventoryItem {
                    parent: *parent,
                    kind: aurora_net::inventory::operations::NewItem::Wearable(*kind),
                    name: name.clone(),
                });
                return;
            }
            if let NetCommand::SaveInventoryContent { item, .. } = &cmd {
                let result = self
                    .world
                    .inventory
                    .items
                    .get(item)
                    .cloned()
                    .ok_or_else(|| "Élément introuvable.".to_owned());
                self.on_app_event(NetEvent::InventoryContentSaved { item: *item, result });
                return;
            }
            if let NetCommand::CreateInventoryItem { parent, kind, name } = &cmd {
                let (asset, inventory, subtype) = kind.types();
                let item = aurora_net::inventory::InvItem {
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
                let bytes = match it.asset_type {
                    56 => b"<?llsd/notation?>\n{'type':'sky'}".to_vec(),
                    57 => aurora_llsd::to_binary(
                        &aurora_llsd::llsd_map! { "version" => "1.1", "type" => "GLTF 2.0", "data" => "{\"asset\":{\"version\":\"2.0\"},\"materials\":[{}]}" },
                    ),
                    _ => format!("Contenu de démonstration : {}", it.name).into_bytes(),
                };
                self.inventory_preview(it.id, Ok(bytes));
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
                    .ok_or_else(|| "Élément introuvable.".into());
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
            for mut ev in crate::demo::demo_reply(&cmd) {
                if matches!(cmd, NetCommand::ObjectRelease { .. }) {
                    crate::demo::hud::preserve_placement(&self.world, &mut ev);
                }
                if let NetEvent::InventoryContents(contents) = &mut ev {
                    for c in contents {
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

    // ------------------------------------------------------------------ login

    fn start_login(&mut self) {
        if self.demo {
            self.login_form.error = Some("Mode démo : aucune connexion à une grille.".into());
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
            mfa_hash: self.settings.mfa_hash.clone(),
            mac,
            id0,
        };
        self.login_form.busy = true;
        self.login_form.error = None;
        self.login_progress = Some(("Connexion…".into(), 0.05));
        self.settings.save();
        self.net.send(NetCommand::Login(Box::new(req)));
    }

    fn back_to_login(&mut self, error: Option<String>) {
        self.hud_drag = None;
        self.media.clear();
        self.media.openid.clear();
        self.save_local_environment();
        self.env_ui = Default::default();
        self.env_scan = false;
        self.panels.environment = false;
        self.panels.personal_lighting = false;
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
        self.appearance_ui = Default::default();
        self.interactions = Default::default();
        self.screen = Screen::Login;
        self.login_form.busy = false;
        self.login_form.focused = false;
        self.login_form.error = error;
        self.login_progress = None;
        self.move_complete_at = None;
        self.set_mouselook_grab(false);
        self.camera = Camera::default();
    }

    fn on_app_event(&mut self, ev: NetEvent) {
        if let NetEvent::InventoryCreated(item) = ev {
            let id = item.id;
            self.world.inventory.add_items(vec![item]);
            self.inventory_ui.show_original(&self.world.inventory, id);
            self.inventory_ui.begin_rename(&self.world.inventory, id);
            return;
        }
        if let NetEvent::InventoryMerchant(result) = ev {
            match result {
                Ok(merchant) => self.inventory_ui.merchant = merchant,
                Err(reason) => self.inventory_ui.message = reason,
            }
            return;
        }
        if let NetEvent::InventoryContentSaved { item, result } = ev {
            self.inventory_ui.save_pending = false;
            let saved = result.is_ok();
            match result {
                Ok(it) => {
                    self.world.inventory.add_items(vec![it]);
                    if self.inventory_ui.preview == Some(item) {
                        self.inventory_ui.preview_dirty = false;
                    }
                    self.inventory_ui.message = "Document enregistré.".into();
                }
                Err(reason) => self.inventory_ui.message = reason,
            }
            for w in &mut self.inventory_ui.windows {
                if w.state.preview == Some(item) {
                    w.state.save_pending = false;
                    if saved {
                        w.state.preview_dirty = false;
                    }
                    w.state.message.clone_from(&self.inventory_ui.message);
                }
            }
            return;
        }
        if let NetEvent::InventoryOperationFailed(reason) = ev {
            self.inventory_ui.message = reason;
            return;
        }
        if let NetEvent::InventoryEdited { request, result } = ev {
            self.inventory_result(request, result);
            return;
        }
        if let NetEvent::InventoryPreview { item, result } = ev {
            self.inventory_preview(item, result);
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
                self.interactions.on_properties(&props);
                self.build.on_properties(props);
            }
            NetEvent::PayPrice { object, default, buttons } => self.interactions.on_prices(object, default, buttons),
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
            }
            NetEvent::PhysicsProperties(list) => {
                for (local_id, params) in list {
                    self.build.physics.insert(local_id, params);
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
                self.login_form.focused = false;
                self.login_progress = None;
                // wrong name / password: do not try the remembered one again
                if bad_credentials && self.login_form.password.is_empty() && self.login_form.stored.take().is_some() {
                    crate::credentials::forget(&self.settings.login_uri(), &self.settings.username);
                }
                if mfa_required {
                    self.login_form.show_mfa = true;
                    self.login_form.error = Some("Saisissez le code de votre application d'authentification.".into());
                } else if tos_required {
                    self.login_form.tos_message = Some(error);
                } else {
                    self.login_form.error = Some(error);
                }
            }
            NetEvent::LoggedIn(l) => {
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
                // the typed password is not kept in memory
                self.login_form.password.clear();
                self.login_form.stored = None;
                self.login_form.stored_for = None;
                self.login_form.password.clear();
                self.login_form.mfa_token.clear();
                self.login_form.agree_tos = false;
                if let Some(h) = &l.mfa_hash {
                    self.settings.mfa_hash = h.clone();
                    self.settings.save();
                }
                if let Some(path) = self.name_cache_path() {
                    self.world.social.avatar_names.load(&path);
                }
                self.screen = Screen::Loading { since: Instant::now() };
                self.loading_frac = 0.0;
                self.loading_target = 0.0;
                self.loading_stage = "Connexion à la région…".into();
                if !l.message.trim().is_empty() {
                    self.world.system_message(l.message.trim().to_owned());
                }
                self.world.system_message(format!("Bienvenue, {} {}.", l.first_name, l.last_name));
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
                self.camera.on_teleport_arrival(&self.settings.camera);
                self.loading_stage = "Chargement de la scène…".into();
            }
            NetEvent::SitResponse {
                object,
                camera_eye,
                camera_at,
                force_mouselook,
            } => self.camera.on_sit_response(object, camera_eye, camera_at, force_mouselook),
            NetEvent::CameraConstraint(plane) => self.camera.set_collide_plane(plane, &self.settings.camera),
            NetEvent::Disconnected { reason } => {
                self.back_to_login(Some(format!("Déconnecté : {reason}")));
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
            // LLSetDisplayNameReply + LLFloaterDisplayName::onCacheSetName
            NetEvent::SetDisplayNameReply { status, reason, content } => {
                log::info!("display name change: {status} {reason}");
                // our name was out of date (409 Conflict): fetch it again
                if status == 409 {
                    let me = self.world.agent_id;
                    self.world.social.avatar_names.refetch(&me);
                }
                let (kind, title, body) = match ui::display_name::reply_message(status, &content) {
                    Ok(m) => (crate::world::notifications::Kind::Info, "Nom d'affichage changé", m),
                    Err(m) => (crate::world::notifications::Kind::Alert, "Nom d'affichage", m),
                };
                self.world
                    .notifications
                    .push(kind, title, body, crate::world::notifications::Data::None);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ input

    fn set_mouselook_grab(&mut self, on: bool) {
        let Some(g) = &self.gfx else {
            return;
        };
        if on == self.cursor_grabbed {
            return;
        }
        if on {
            let ok = g
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            g.window.set_cursor_visible(false);
            self.cursor_grabbed = ok;
        } else {
            let _ = g.window.set_cursor_grab(CursorGrabMode::None);
            g.window.set_cursor_visible(true);
            self.cursor_grabbed = false;
        }
    }

    fn toggle_mouselook(&mut self) {
        self.camera.toggle_mouselook(&mut self.world.agent, &self.settings.camera);
        let ml = self.camera.mouselook();
        self.set_mouselook_grab(ml);
    }

    /// Mouse motion: mouselook, camera drags (steering, Alt), right drag orbit.
    fn camera_mouse_delta(&mut self, dx: f32, dy: f32) {
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
            self.right_moved += dx.abs() + dy.abs();
            if self.right_moved < 4.0 {
                return; // maybe a click: do not move the camera yet
            }
            self.camera.orbit_drag(dx, dy, s, &mut self.world.agent);
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
                Step::SitEvents => {
                    for ev in crate::demo::sit_events(self.frame_count) {
                        if let Some(e) = self.world.apply(ev) {
                            self.on_app_event(e);
                        }
                    }
                }
                Step::Log => log::info!(
                    "demo camera f{}: {} (mouse {:?}, agent yaw {:.2})",
                    self.frame_count,
                    self.camera.describe(),
                    self.mouse_mode,
                    self.world.agent.yaw
                ),
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

    /// Escape / « Réinitialiser la caméra » (handle_reset_view).
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
            Action::AlwaysRun => {
                self.always_run = !self.always_run;
                self.send(NetCommand::SetAlwaysRun(self.always_run || self.temp_run.is_some()));
                self.world.system_message(if self.always_run {
                    "Toujours courir : activé"
                } else {
                    "Toujours courir : désactivé"
                });
            }
            Action::Mouselook => self.toggle_mouselook(),
            Action::ResetCamera => self.reset_camera_view(),
            Action::Chat => self.chat_ui.focus_request = true,
            Action::ToggleMic => self.mic_on = !self.mic_on,
            Action::Conversations => self.panels.chat = !self.panels.chat,
            Action::People => {
                self.panels.people = !self.panels.people;
                self.panels.people_tab = 0;
            }
            Action::Inventory => self.panels.inventory = !self.panels.inventory,
            Action::Minimap => self.panels.minimap = !self.panels.minimap,
            Action::WorldMap => self.panels.world_map = !self.panels.world_map,
            Action::TeleportHistory => self.toggle_teleport_history(),
            Action::Performance => self.panels.perf = !self.panels.perf,
            Action::Preferences => self.panels.settings = !self.panels.settings,
            Action::ShowTransparency => {
                let d = &mut self.settings.debug;
                d.show_alpha = !d.show_alpha;
                self.world.system_message(if d.show_alpha {
                    "Afficher la transparence : activé (Ctrl+Alt+T pour couper)"
                } else {
                    "Afficher la transparence : désactivé"
                });
            }
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
            let t = (pos + Vec3::Z * ui::hud::TAG_HEIGHT - o).dot(d);
            match world_t {
                Some(h) if h < t - 0.05 => None,
                _ => Some((idx, o + d * t)),
            }
        })
    }

    fn on_left_press(&mut self) {
        self.left_down = true;
        if self.alt
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
        let Some(g) = self.gfx.as_mut() else {
            return;
        };
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
        let ray = g.renderer.cursor_ray(x, y);
        let hit = self.scene.action_point(
            &self.world,
            ray,
            g.renderer.pick_world(x, y),
            self.build.open,
            self.settings.draw_distance,
        );
        // on our avatar: a surface inside its box under the cursor (nothing
        // in front of it); or, when the ray goes through the box without
        // touching anything (body not drawn yet), close to the body's axis
        let avatar_hit = ray.and_then(|(o, d)| {
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
        if !self.alt && !on_avatar {
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
        if !on_avatar
            && !self.alt
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
        if self.alt {
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
        }
    }

    /// Right click in the world: menu for what is under the cursor.
    /// Keep the audio engine in step with the settings and the parcel music
    /// (LLViewerParcelMedia / LLViewerAudio: autoplay on entering a parcel).
    /// Our look-at target (LLAgentCamera::setLookAt): off = nothing shown
    /// nor sent (PrivateLocalLookAtTarget + PrivateLookAtTarget); with a
    /// range, a target away from us is brought back within it
    /// (FSLookAtTargetLimitDistance).
    fn set_look_at(&mut self, kind: u8, object: Option<uuid::Uuid>, offset: glam::DVec3) {
        use crate::world::lookat;
        let now = Instant::now();
        let me = self.world.agent_id;
        if !(self.settings.look_at || self.demo_look_at) {
            self.world.look_at.own.set(lookat::NONE, Some(me), glam::DVec3::ZERO, now);
            return;
        }
        let (mut object, mut offset) = (object, offset);
        let range = self.settings.look_at_range;
        if lookat::clamped(kind) && range < crate::settings::LOOK_AT_UNLIMITED && object != Some(me) {
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
            let head = origin + head.as_dvec3();
            let d = offset - head;
            let range = range as f64;
            if d.length() > range {
                offset = head + d * (range / d.length());
            }
        }
        self.world.look_at.own.set(kind, object, offset, now);
    }

    /// Where our avatar looks this frame (LLAgentCamera::updateLookAt and
    /// setFocusGlobal), and the ViewerEffect sent when it changes.
    fn update_look_at(&mut self, width: f32, height: f32) {
        use crate::world::lookat;
        let now = Instant::now();
        let me = self.world.agent_id;
        let on = self.settings.look_at || self.demo_look_at;
        if self.look_at_was_on && !on {
            // switched off: the others stop showing our last target
            self.send(NetCommand::LookAt {
                effect: self.world.look_at.own.effect,
                target: uuid::Uuid::nil(),
                offset: [0.0; 3],
                kind: lookat::NONE,
                duration: 0.0,
            });
        }
        self.look_at_was_on = on;
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
        let (kind, offset) = if still && !turning {
            let v = self.world.agent.velocity;
            let ahead = if v.length_squared() > 4.0 {
                v
            } else {
                self.world.agent.forward() * 2.0
            };
            (lookat::IDLE, body.inverse() * ahead)
        } else if self.camera.mouselook() {
            (lookat::MOUSELOOK, self.camera.forward())
        } else {
            // YawFromMousePosition / PitchFromMousePosition = 90°
            let fwd = self.camera.forward();
            let xc = cx / width.max(1.0) - 0.5;
            let yc = 0.5 - cy / height.max(1.0);
            let left = Vec3::Z.cross(fwd).normalize_or(Vec3::Y);
            let dir = glam::Quat::from_axis_angle(Vec3::Z, -xc * std::f32::consts::FRAC_PI_2)
                * glam::Quat::from_axis_angle(left, -yc * std::f32::consts::FRAC_PI_2)
                * fwd;
            (lookat::FREELOOK, dir)
        };
        self.set_look_at(kind, Some(me), offset.as_dvec3());
        let own = &mut self.world.look_at.own;
        own.update(now);
        if on && own.needs_send {
            let t = own.target;
            let cmd = NetCommand::LookAt {
                effect: own.effect,
                target: t.object.unwrap_or_default(),
                offset: t.offset.to_array(),
                kind: t.kind,
                duration: own.duration,
            };
            own.sent(now);
            self.send(cmd);
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
        for o in self.world.objects.slots.iter_mut().flatten() {
            if o.sound_update {
                o.sound_update = false;
                if blocked(o) {
                    self.sound_blocks.1.insert(o.full_id);
                } else {
                    self.scene.sounds.apply_object_sound(engine, o.full_id, o.sound);
                }
            } else if recheck && !o.sound.id.is_nil() {
                if blocked(o) {
                    if self.sound_blocks.1.insert(o.full_id) {
                        self.scene.sounds.apply_object_sound(engine, o.full_id, Default::default());
                    }
                } else if self.sound_blocks.1.remove(&o.full_id) {
                    self.scene.sounds.apply_object_sound(engine, o.full_id, o.sound);
                }
            }
        }
    }

    /// Plays an interface sound when the settings allow it (make_ui_sound /
    /// find_ui_sound: its play flag, its asset, the "Sons de l'interface"
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

    fn update_sounds(&mut self) {
        self.flush_object_sounds();
        let ui = &self.settings.audio.ui;
        let w = &mut self.world;
        // conversations by their IM mode (LLIMMgr::addMessage), never in
        // do not disturb; "in front" = shown in the focused window
        let dnd = w.status.dnd;
        for m in std::mem::take(&mut w.im_messages) {
            let front = self.focused && self.panels.chat && self.chat_ui.selected == Some(m.session);
            if !dnd && let Some(s) = ui.im_sound(m, front) {
                w.ui_sounds.push(s);
            }
        }
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

    /// Log out, save the window, layout, inventory cache and settings, exit.
    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        if self.hud_drag.is_some() {
            self.on_left_release();
        }
        if self.in_world() {
            self.net.send(NetCommand::Logout);
            std::thread::sleep(Duration::from_millis(300));
        }
        if let Some(g) = &self.gfx {
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
        event_loop.exit();
    }

    /// Remember the normal (not maximized / minimized) window size and
    /// position, 300 ms after the last move / resize: while maximizing,
    /// Windows reports the new geometry before the window reads as maximized.
    fn record_window_geometry(&mut self) {
        let Some(t) = self.geom_changed else {
            return;
        };
        if t.elapsed() < Duration::from_millis(300) || self.capture.is_some() {
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
            self.world.system_message("Destination non reconnue.");
            return;
        };
        self.world.system_message(format!(
            "Téléportation vers {} ({:.0}, {:.0}, {:.0})…",
            l.region, l.pos.x, l.pos.y, l.pos.z
        ));
        self.teleport_to_location(l.region, l.pos);
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
        };
        self.voice.update(inputs);
        let world = &self.world;
        self.voice.apply_blocks(|id| world.voice_blocked(id));
        self.voice_connected = self.voice.connected;
        self.voice_level = self.voice.own_level;
        self.options_ui.mic_level = self.voice.mic_meter;
    }

    /// End of a shortcut capture (None = cancelled).
    fn finish_capture(&mut self, input: Option<Input>) {
        let Some((action, slot)) = self.options_ui.capture.take() else {
            return;
        };
        if let Some(input) = input {
            let b = crate::keybinds::Binding {
                input,
                ctrl: self.ctrl,
                shift: self.shift,
                alt: self.alt,
            };
            self.settings.keybinds.set(action, slot, Some(b));
            self.settings.save();
        }
    }

    /// What the build floater asked for (build::ui::Request).
    fn on_build_request(&mut self, r: crate::build::ui::Request) {
        use crate::build::ui::Request;
        match r {
            Request::ZoomFraction(f) => self.camera.set_zoom_fraction(f),
            Request::AboutLand => self.panels.about_land = true,
            Request::BuyLand => self
                .world
                .system_message("L'achat de terrain n'est pas encore disponible dans Aurora."),
            Request::Profile(id) => self.profile_ui.open(&mut self.world, id),
            Request::GroupProfile(_) => self
                .world
                .system_message("Le profil des groupes n'est pas encore disponible dans Aurora."),
            Request::PasteVector(which) => {
                let text = self.gfx.as_mut().and_then(|g| g.egui_state.clipboard_text()).unwrap_or_default();
                self.build.paste_vector(&mut self.world, &self.settings.build, which, &text);
            }
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
        // AURORA_DEMO_HOVER=1: the cursor swept over the scene, then still
        if let Some(g) = &self.gfx
            && let Some(cursor) = crate::demo::hover_sweep(self.frame_count, g.renderer.size())
        {
            self.cursor_pos = cursor;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_ACTIONS") else {
            return;
        };
        let id = crate::demo::action_mode_id(&mode);
        let click_frame = if mode.starts_with("linked-") { 724 } else { 284 };
        if self.frame_count == 260
            && (id == 0 || id == 980 || id == 981)
            && let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::action_id(971))
            && let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, Instant::now(), 0)
        {
            let direction = (self.camera.position - pos).normalize_or(-Vec3::X);
            let rotation = glam::Quat::from_rotation_arc(Vec3::X, direction);
            self.world.apply(crate::demo::action_overlay(
                if id == 980 { 980 } else { 981 },
                pos + direction * 1.0,
                rotation,
            ));
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
            && (click_frame - 4..=click_frame).contains(&self.frame_count)
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
        if id != 0 && self.frame_count == click_frame {
            if mode.ends_with("-build") {
                self.build.open_build(crate::build::Tool::Edit);
                self.build_mouse_down();
            } else {
                self.on_left_press();
            }
            if id != 975 && id != 982 {
                self.on_left_release();
            }
            log::info!(
                "demo action click: {mode}; dialog={}, seated={}, contents={}, parcel_playing={}, parcel_paused={}, selected={:?}, focus={:?}, media_url={}",
                self.interactions.dialog.is_some(),
                self.world.agent.is_sitting(),
                self.interactions
                    .contents
                    .as_ref()
                    .and_then(|c| c.result.as_ref())
                    .and_then(|r| r.as_ref().ok())
                    .map_or(0, Vec::len),
                self.media.parcel_playing(),
                self.media.find(&crate::media::MediaKey::Parcel).is_some_and(|m| m.paused),
                self.build.selection,
                self.camera.focus_point(),
                self.interactions.media_url.is_some(),
            );
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

    /// Left press while building: handles, selection, create, terraform.
    fn build_mouse_down(&mut self) -> bool {
        if !self.build.open || self.alt {
            return false;
        }
        if self.build.tool == crate::build::Tool::Focus {
            return self.focus_tool_down();
        }
        let Some(g) = self.gfx.as_mut() else {
            return false;
        };
        let (x, y) = self.cursor_pos;
        let hit = g.renderer.pick_world(x, y);
        let target = hit.and_then(|p| self.scene.pick_at(&self.world, p, Instant::now()));
        let mods = crate::build::Mods {
            shift: self.shift,
            ctrl: self.ctrl,
            alt: self.alt,
        };
        // the surface under the cursor: face tool, grab surface info
        let surface = match (self.build.tool, target) {
            (crate::build::Tool::Edit | crate::build::Tool::Grab, Some(i)) => {
                self.scene.touch_surface(&self.world, i, self.build.cam.ray(x, y))
            }
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
        let Some(g) = self.gfx.as_mut() else {
            return false;
        };
        let (x, y) = self.cursor_pos;
        let hit = g.renderer.pick_world(x, y);
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
            self.right_moved = 0.0;
        } else if was_drag && self.right_moved < 4.0 && self.mouse_mode == MouseMode::None {
            // a click (no drag): context menu
            self.open_context_menu();
        }
    }

    fn on_left_release(&mut self) {
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
            self.world.system_message(format!("Téléportation vers {}…", e.region));
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
            if !typing && self.in_world() && self.settings.keybinds.held(a, &self.down, m) {
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

    fn lights(&self) -> Vec<PointLight> {
        let eye = self.camera.position;
        let mut lights: Vec<(f32, PointLight)> = Vec::new();
        for &idx in &self.scene.light_objects {
            let Some(o) = self.world.objects.get(idx) else {
                continue;
            };
            let Some(l) = o.extra.light else {
                continue;
            };
            let Some(g) = self.scene.gpu.get(idx) else {
                continue;
            };
            if g.hud {
                continue;
            }
            let d = g.center.distance(eye);
            if d > 96.0 + l.radius * 1.5 {
                continue;
            }
            // the packed light color is already linear (LLLightParams::unpack ->
            // setLinearColor; pipeline.cpp sends getLightLinearColor() as is),
            // times 3.25 for the PBR punctual intensity (multiPointLightF)
            let color = Vec3::new(l.color[0], l.color[1], l.color[2]) * l.color[3] * 3.25;
            lights.push((
                d,
                PointLight {
                    position: g.center,
                    // deferred SL lights reach 1.5x their radius with half the
                    // falloff (pipeline.cpp: getLightRadius() * 1.5,
                    // getLightFalloff(DEFERRED_LIGHT_FALLOFF = 0.5))
                    radius: (l.radius * 1.5).clamp(0.1, 30.0),
                    color,
                    falloff: l.falloff * 0.5,
                },
            ));
        }
        lights.sort_by(|a, b| a.0.total_cmp(&b.0));
        lights.into_iter().take(64).map(|(_, l)| l).collect()
    }

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
            ContactsAction::Im(id) => {
                self.world.ui_sounds.push(UiSound::StartIm);
                self.world.social.session_mut(id);
                self.world.social.focus_im = Some(id);
                self.panels.chat = true;
            }
            ContactsAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
            ContactsAction::OfferTeleport(id) => a.offer_tp.push(id),
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
        wanted.extend(self.profile_ui.wanted_pics.drain());
        wanted.extend(self.contacts_ui.wanted_pics.drain());
        for agent in wanted {
            if self.avatar_pics.contains_key(&agent) {
                continue;
            }
            self.world.want_profile(agent);
            let Some(image) = self.world.profiles.get(&agent).map(|p| p.sl_image).filter(|i| !i.is_nil()) else {
                continue;
            };
            self.scene.textures.want_ui_image(renderer, image);
            if let Some((w, h, rgba)) = self.scene.textures.take_ui_image(&image)
                && rgba.len() == (w * h * 4) as usize
            {
                let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                let t = self.egui_ctx.load_texture(format!("pic-{agent}"), ci, egui::TextureOptions::LINEAR);
                self.avatar_pics.insert(agent, t);
            }
        }
        let mut images: Vec<uuid::Uuid> = self.profile_ui.wanted_images.drain().collect();
        images.extend(self.place_images.drain());
        images.extend(self.land_ui.wanted_images.drain());
        images.extend(self.contacts_ui.wanted_images.drain());
        images.extend(self.build.ui.wanted_images.drain());
        images.extend(self.appearance_ui.wanted_images.drain());
        images.extend(self.inventory_ui.wanted_images.drain());
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

    /// Place profiles and « Lieux »: arrivals saved in the history, landmark
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
        for folder in self.places_ui.want_folders.drain() {
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
            }
        }
        let me = ui::minimap::to_global(&self.world, self.world.agent.position);
        if self.world.movement_complete && !self.world.teleporting {
            self.world.map.check_arrival(me.0, me.1, self.world.agent.position.z);
        }
        // About Land requests (selection, lists, covenant...)
        for c in self.world.land.take_commands() {
            self.send(c);
        }
        self.update_places();
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
        self.poll_inventory_images();
        self.frame_profile.lap(Lap::Between);
        let now = Instant::now();
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
            self.palette = self.skin.theme.palette();
            self.cursors.set_palette(&self.palette);
            self.egui_ctx.set_zoom_factor(self.settings.ui_scale);
        }

        self.frame_profile.lap(Lap::Start);
        // ---- network events (time-boxed)
        let t_ev = Instant::now();
        self.world.events_this_frame = 0;
        while let Ok(ev) = self.net.events.try_recv() {
            if let Some(app_ev) = self.world.apply(ev) {
                self.on_app_event(app_ev);
            }
            if t_ev.elapsed() > Duration::from_millis(8) {
                break;
            }
        }
        self.perf.events_ms = t_ev.elapsed().as_secs_f32() * 1000.0;
        for anim in std::mem::take(&mut self.world.animation_stops) {
            self.send(NetCommand::AgentAnimation { anim, start: false });
        }
        self.frame_profile.lap(Lap::Events);
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
            for cmd in self.world.take_social_commands() {
                self.send(cmd);
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
                self.world.system_message(format!(
                    "Impossible de charger les paramètres de {name} à partir de la base de données."
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
        let Some(mut gfx) = self.gfx.take() else {
            return;
        };
        let t_up = Instant::now();
        let (w, h) = gfx.renderer.size();
        let aspect = w as f32 / h.max(1) as f32;

        // ---- agent / camera
        let typing = self.egui_ctx.egui_wants_keyboard_input();
        self.update_move_input(typing, dt);
        self.update_audio();
        if self.in_world() {
            self.update_sounds();
        }
        if self.in_world() {
            let ml = self.camera.mouselook();
            if self.demo {
                // no simulator offline: move the avatar locally
                let p = self.world.agent.position;
                let floor = crate::demo::floor_at(p.x, p.y);
                self.world.agent.simulate_locally(&self.input, ml, Some(floor), dt);
            } else {
                let motion = *self.net.stats.simulator_motion.lock();
                if motion.handle == self.world.main_region {
                    self.world.predict_agent(motion.time_dilation, motion.packet_age());
                }
            }
            let camera_distance = self.camera.position.distance(self.world.agent.position);
            self.world.agent.update(&self.input, ml, dt, camera_distance);
            let frame = crate::camera::FrameInput {
                dt,
                now: Instant::now(),
                steering: self.mouse_mode == MouseMode::Steer,
                build_mode: self.build.open,
                draw_distance: self.settings.draw_distance,
            };
            self.camera.update(&mut self.world, &self.settings.camera, &frame);
            let at = self.camera.forward();
            self.world.agent.body_turn = self.world.update_bodies(Instant::now(), dt, at, ml);
            self.update_look_at(w as f32, h as f32);
            let mut c = self.world.agent.controls(&self.input, &self.camera, self.settings.draw_distance);
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
            *self.net.controls.lock() = c;
        }
        self.update_voice();
        let view = self.camera.view();
        let proj = self.camera.proj(aspect);
        let vp = proj * view;
        let mut cull = CullView::new(vp, self.camera.position, h as f32, self.camera.fov_y);
        // debug: frozen culling view (move around to see what it drops)
        if self.settings.debug.freeze_culling {
            cull = *self.frozen_cull.get_or_insert(cull);
        } else {
            self.frozen_cull = None;
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
            let frame_time = Duration::from_secs_f32(dt);
            let results_budget = crate::scene::textures::frame_share(frame_time, Duration::from_millis(1));
            // the full syncs of arriving objects get the same share; more
            // behind a loading or teleport screen, whose frames show nothing
            let covered = matches!(self.screen, Screen::Loading { .. }) || self.tp_overlay.as_ref().is_some_and(|o| o.end.is_none());
            self.scene.sync_budget = if covered { Duration::from_millis(8) } else { results_budget };
            self.scene.process_results(&mut gfx.renderer, &self.net, results_budget);
            self.frame_profile.lap(Lap::Results);
            // poses first: attachments follow their bone in the same frame
            let completed = self
                .scene
                .update_poses(&mut gfx.renderer, &mut self.world, self.camera.position, now);
            for anim in completed {
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
            self.world.hud_aspect = hud_size.width as f32 / hud_size.height.max(1) as f32;
            let drag_view =
                self.scene.lists.hud_view.map(|view| {
                    aurora_render::HudView::new([hud_size.width, hud_size.height], self.world.hud_zoom, view.min_x, view.max_x)
                });
            self.update_hud_drag(drag_view);
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
                    language: "fr",
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
                    let point = self.scene.interaction_point(
                        &self.world,
                        Some(ray),
                        gfx.renderer.hover_pick(self.cursor_pos.0, self.cursor_pos.1),
                        self.build.open,
                        self.settings.draw_distance,
                    );
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
            self.options_ui.vram = (gfx.renderer.info.vram_mb, gfx.renderer.info.integrated);
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

        // ---- loading -> world transition
        if let Screen::Loading { since } = self.screen {
            let moved = self.move_complete_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
            let terrain = self.world.terrain_progress();
            if self.world.movement_complete && since.elapsed() > Duration::from_millis(1500) && (terrain > 0.97 || moved > 6.0) {
                self.screen = Screen::World;
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
                w.tp_status = "Chargement de la région…".into();
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
                w.tp_status = "Connexion à la région…".into();
            } else if e >= 0.9 {
                w.tp_progress = (0.15 + (e - 0.9) * 0.18).min(0.42);
                w.tp_status = if e < 1.6 {
                    "Envoi vers la destination…"
                } else {
                    "Contact avec la nouvelle région…"
                }
                .into();
            } else if e >= 0.35 {
                w.tp_progress = w.tp_progress.max(0.10);
                w.tp_status = "Téléportation acceptée…".into();
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
            let k = 1.0 - (-7.0 * dt).exp();
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

        // ---- environment: local preset > parcel > region (or default day),
        // with crossfades; the simulator sun only for regions without EEP
        self.world.eep.set_manual_transition(self.settings.env_manual_transition);
        let (sky_frame, water_frame) = self.world.environment_frames(&mut self.panels.time_of_day);
        // cloud scroll (LLEnvironment::updateCloudScroll: rate / 100 per second)
        self.world.cloud_scroll += sky_frame.cloud_scroll_rate * dt / 100.0;
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
        let mirror = if in_world && self.settings.mirrors > 0 {
            self.scene.find_mirror(&self.world, &cull, Instant::now())
        } else {
            None
        };
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
            );
            self.scene.lists.particles = out;
        }
        let lights = if in_world { self.lights() } else { Vec::new() };
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
            exposure: self.settings.exposure,
            sharpen: self.settings.sharpen,
            glow: self.settings.glow,
            water_height: self.world.main_water_height(),
            shadows: self.settings.shadow_quality > 0 && matches!(self.screen, Screen::World),
            shadow_distance: self.settings.shadow_params().1,
            lights,
            clear_color: Vec4::new(env.fog_color.x, env.fog_color.y, env.fog_color.z, 1.0),
            sky: env.sky,
            water: env.water,
            mirror,
            probes,
            wireframe: self.settings.debug.wireframe,
            debug_glow: self.settings.debug.glow_view,
        };

        self.frame_profile.lap(Lap::Params);
        // ---- UI
        let t_ui = Instant::now();
        let mut raw = gfx.egui_state.take_egui_input(&gfx.window);
        // Inline-edit captures simulate logical focus without activating the
        // native window or taking the user's keyboard (with_active(false)).
        if self.demo
            && self.capture.is_some()
            && std::env::var("AURORA_DEMO_INVENTORY")
                .is_ok_and(|v| matches!(v.as_str(), "rename" | "new-script" | "new-note" | "new-folder"))
        {
            raw.focused = true;
            raw.events.retain(|e| !matches!(e, egui::Event::WindowFocused(false)));
            if let Some(viewport) = raw.viewports.get_mut(&raw.viewport_id) {
                viewport.focused = Some(true);
            }
        }
        // AURORA_DEMO_POINTER="x,y[,r][;x,y…]": the pointer at these window
        // pixels from frame 300, one point every 60 frames (hover states,
        // sub-menus); ",r" right-clicks the interface there
        if self.demo
            && self.frame_count >= 300
            && let Ok(v) = std::env::var("AURORA_DEMO_POINTER")
        {
            let points: Vec<(f32, f32, bool)> = v
                .split(';')
                .filter_map(|pt| {
                    let mut it = pt.split(',').map(str::trim);
                    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next() == Some("r")))
                })
                .collect();
            let step = (self.frame_count - 300) / 60;
            let i = (step as usize).min(points.len().saturating_sub(1));
            if let Some(&(x, y, right)) = points.get(i) {
                let ppp = self.egui_ctx.pixels_per_point().max(0.1);
                let pos = egui::pos2(x / ppp, y / ppp);
                raw.events.push(egui::Event::PointerMoved(pos));
                let click_frame = (self.frame_count - 300) % 60;
                if right && step as usize == i && matches!(click_frame, 2 | 3) {
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed: click_frame == 2,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
        let ctx = self.egui_ctx.clone();
        let projector = ui::hud::Projector {
            view_proj: vp,
            width: w as f32,
            height: h as f32,
            ppp: ctx.pixels_per_point(),
        };
        // build tools: camera of this frame, hover / drags / land brush
        self.build.cam = crate::build::geom::Cam::new(
            self.camera.position,
            self.camera.target,
            vp,
            self.camera.fov_y,
            w as f32,
            h as f32,
            ctx.pixels_per_point(),
        );
        if self.in_world() {
            let over_ui = ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input();
            let mods = crate::build::Mods {
                shift: self.shift,
                ctrl: self.ctrl,
                alt: self.alt,
            };
            if self.build.open {
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
            self.build
                .update(&mut self.world, &self.settings.build, self.cursor_pos, over_ui, mods);
            self.scene.selection_wire = self.build.wire_selection(&self.world, &self.settings.build);
            self.flush_build();
        }
        let mut actions = UiActions::default();
        let vsync = gfx.renderer.vsync();
        self.frame_profile.lap(Lap::UiPrep);
        self.sound_cues.before(&ctx, &raw);
        let mut full = ctx.run_ui(raw, |ui| {
            self.sound_cues.pass_start(ui.ctx());
            actions = self.draw_ui(ui, &projector, &gfx.renderer);
        });
        // interface sounds of the widgets; the sounds are SL assets, loaded
        // once logged in
        let in_world_view = matches!(self.screen, Screen::World);
        let cues = self.sound_cues.after(&ctx, &full.platform_output.events, in_world_view);
        let previews = ui::sound_cues::take_previews(&ctx);
        if self.in_world() {
            self.world.ui_sounds.extend(cues);
            // preview buttons of the sound preferences (force_sound)
            for id in previews {
                self.scene.sounds.play_ui(self.audio_engine.as_ref(), id);
            }
        }
        self.frame_profile.lap(Lap::Ui);
        // Native bitmap cursors, including the original Firestorm hotspot.
        // Set this every frame because egui's custom cursor output is sticky.
        full.platform_output.cursor_image = None;
        self.update_object_hold(&gfx.renderer);
        if let Some(url) = self.interactions.media_url.take() {
            if self.demo {
                log::info!("demo open media: external browser request simulated");
                self.world.system_message("Démo : ouverture de l'URL du média simulée.");
            } else {
                full.platform_output
                    .commands
                    .push(egui::OutputCommand::OpenUrl(egui::OpenUrl::new_tab(url)));
            }
        }
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
            && let Some(idx) = self
                .scene
                .hud_pick(&self.world, self.cursor_pos, false)
                .map(|(idx, _, _)| idx)
                .or_else(|| {
                    // Preserve the world hover cache (scene/hover.rs).
                    self.scene.hover_object(
                        &self.world,
                        gfx.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1),
                        gfx.renderer.hover_pick(self.cursor_pos.0, self.cursor_pos.1),
                        self.settings.draw_distance,
                    )
                })
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
        if let Some(h) = &self.interactions.held {
            if h.physical {
                full.platform_output.cursor_image = self.cursors.image(crate::interaction::Action::Grab, false);
            } else {
                full.platform_output.cursor_icon = egui::CursorIcon::PointingHand;
                full.platform_output.cursor_image = None;
            }
        }
        if self.hud_drag.is_some()
            || (self.alt
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
        gfx.egui_state
            .handle_platform_output_with_event_loop(&gfx.window, event_loop, full.platform_output);
        self.frame_profile.lap(Lap::Hover);
        let ppp = full.pixels_per_point;
        let prims = ctx.tessellate(full.shapes, ppp);
        self.perf.ui_ms = t_ui.elapsed().as_secs_f32() * 1000.0;
        self.frame_profile.lap(Lap::Tessellate);

        // apply UI actions
        if actions.font_changed {
            ui::fonts::apply(&self.egui_ctx, &self.settings.font);
        }
        if actions.settings_changed {
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
        }
        self.handle_actions(actions, event_loop);
        for r in std::mem::take(&mut self.build.ui.requests) {
            self.on_build_request(r);
        }
        self.update_avatar_pics(&mut gfx.renderer);
        self.update_maps();
        self.poll_key_to_name();

        if let Some(request) = self.inventory_images.capture.take() {
            if self.scene_capture_pending || self.inventory_images.capturing.is_some() || self.inventory_images.auto_capture_pending {
                self.inventory_images.capture = Some(request);
            } else {
                gfx.renderer.capture_scene = true;
                self.inventory_images.capturing = Some(request);
            }
        }
        if self.want_scene_capture && self.inventory_images.capturing.is_none() && !self.inventory_images.auto_capture_pending {
            self.want_scene_capture = false;
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
        let stats = gfx.renderer.render(
            fp,
            lists,
            Some(EguiFrame {
                primitives: prims,
                textures_delta: std::mem::take(&mut full.textures_delta),
                pixels_per_point: ppp,
            }),
        );
        self.last_render = stats;
        if gfx.renderer.failed() {
            // the render thread is gone (its panic is in the log): nothing
            // more can be shown, leave cleanly
            self.gfx = Some(gfx);
            self.shutdown(event_loop);
            return;
        }
        self.frame_profile.lap(Lap::Render);
        // frame limiter; vsync already paces at the screen rate
        let timed = self.capture.is_some() || self.frame_profile.enabled();
        let cap = frame_cap(
            &self.settings,
            self.focused,
            self.in_world(),
            self.monitor_hz,
            timed,
            crate::cli::get().fps_limit,
        );
        let paced_by_vsync = gfx.renderer.vsync() && cap >= self.monitor_hz - 0.5;
        if cap >= 1.0 && !paced_by_vsync {
            let target = Duration::from_secs_f32(1.0 / cap);
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
        if self.demo && std::env::var_os("AURORA_DEMO_ANIM_LOOP").is_some() {
            for event in crate::demo::loop_animation_events(self.frame_count) {
                self.world.apply(event);
            }
        }
        if self.demo {
            for event in crate::demo::eep::events(self.frame_count) {
                self.world.apply(event);
            }
            if let Some(step) = crate::demo::env::step(self.frame_count) {
                self.demo_env_step(step);
            }
        }
        // last view for the loading screens (blurred in a thread, or now when quitting)
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
            let wait = self.closing.is_some() || self.quit_at.is_some();
            self.backdrop.set_capture(w, h, px, wait);
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
        self.demo_action_steps();
        self.demo_hud_drag_steps();
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
        // closing the window: leave once the last view is kept (or after 1 s)
        if let Some(t) = self.closing
            && ((!self.scene_capture_pending && !self.want_scene_capture) || t.elapsed() > Duration::from_secs(1))
        {
            self.closing = None;
            self.shutdown(event_loop);
            return;
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
        // AURORA_DEMO_CONTACTS: contact sets and aliases to show, once logged in
        if self.demo && self.frame_count == 60 && std::env::var_os("AURORA_DEMO_CONTACTS").is_some() {
            crate::demo::seed_contact_sets(&mut self.world);
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
                let region = if v == "remote" { "Aurora Ailleurs" } else { "Aurora Démo" };
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
        }
        // AURORA_DEMO_LOOKAT=1: our eye tracking on (cursor on the left of
        // the window, moving), Loup Violet looks at a point north of him
        if self.demo && std::env::var_os("AURORA_DEMO_LOOKAT").is_some() {
            self.demo_look_at = true;
            if let Some(g) = self.gfx.as_ref() {
                let (w, h) = g.renderer.size();
                self.cursor_pos = (w as f32 * 0.15 + (self.frame_count % 2) as f32, h as f32 * 0.3);
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
                    crate::build::demo::Step::Press => {
                        self.build_mouse_down();
                    }
                    crate::build::demo::Step::Release => {
                        self.build.mouse_up(&mut self.world, &self.settings.build);
                        self.flush_build();
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
                log::info!(
                    "demo media click at {pos:?} (frame {}): focus {:?}",
                    self.frame_count,
                    self.media.focus
                );
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
                    log::info!("demo mmo: mode {:?}, pos {:?}", self.mouse_mode, self.world.agent.position);
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
        self.frame_profile.settings(|| {
            format!(
                "{width}x{height} vsync={} fps_cap={} draw_distance={:.0} lod_factor={:.2} max_avatars={} shadows={} \
                 shadow_quality={} probes={} mirrors={} {:?}",
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
        };
        let parts = std::mem::take(&mut self.scene.parts);
        self.frame_profile.end_frame(&self.last_render, &counts, &parts);
    }

    /// Local chat from the chat bar or the conversations window: a chat bar
    /// command (FSCmdLine) or a line sent on its channel.
    fn send_local_chat(&mut self, c: ui::chat::OutgoingChat) {
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
        match a.login {
            LoginAction::Login => self.start_login(),
            LoginAction::Quit => event_loop.exit(),
            LoginAction::None => {}
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
                self.world
                    .system_message(format!("{name} a été débloqué(e) parce que vous lui avez envoyé un IM."));
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
        for to in a.offer_tp {
            self.net.send(NetCommand::OfferTeleport {
                to,
                message: "Rejoins-moi !".into(),
            });
            self.world.system_message("Offre de téléportation envoyée.");
        }
        if let Some(asset) = a.landmark {
            self.begin_teleport("Repère".into(), false);
            self.net.send(NetCommand::TeleportLandmark(asset));
        }
        if a.cancel_loading {
            self.net.send(NetCommand::Logout);
            self.back_to_login(Some("Connexion annulée.".into()));
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
                if self.settings.loading_backdrop {
                    self.want_scene_capture = true;
                }
                self.net.send(NetCommand::Logout);
                self.world.system_message("Déconnexion…");
            }
            BarAction::Quit => {
                if self.in_world() {
                    if self.settings.loading_backdrop {
                        self.want_scene_capture = true;
                    }
                    self.net.send(NetCommand::Logout);
                }
                self.settings.save();
                self.quit_at = Some(Instant::now() + Duration::from_millis(400));
            }
            BarAction::TeleportHome => {
                self.begin_teleport("Domicile".into(), false);
                self.net.send(NetCommand::TeleportHome);
            }
            BarAction::TeleportBack => self.teleport_history(true),
            BarAction::TeleportForward => self.teleport_history(false),
            BarAction::TeleportTo(text) => self.teleport_to_text(&text),
            BarAction::SetStatus(m) => {
                self.world.set_away(m.away);
                self.auto_away = false;
                self.world.set_dnd(m.dnd);
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
            BarAction::ChatPrefs => {
                self.panels.settings = true;
                self.options_ui.tab = ui::options::TAB_CHAT;
            }
            BarAction::VoicePrefs => {
                self.panels.settings = true;
                self.options_ui.tab = ui::options::TAB_AUDIO;
            }
            BarAction::ToggleFly => self.toggle_fly(),
            BarAction::ToggleMouselook => self.toggle_mouselook(),
            BarAction::SitGround => self.toggle_ground_sit(),
            BarAction::StandUp => self.send(NetCommand::OneShotControl(control::STAND_UP)),
            BarAction::ResetCamera => self.reset_camera_view(),
            BarAction::BanLines(v) => self.settings.maps.ban_lines = v,
            BarAction::SharedEnvironment => self.world.eep.clear_local(),
            BarAction::TeleportHistory => self.toggle_teleport_history(),
        }
    }

    /// toggleTeleportHistory (llviewermenu.cpp) without the standalone
    /// history floater: hide Lieux when shown, else open it on the history.
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
        if self.demo {
            // The login demo uses a synthetic marker and never accesses the OS store.
            if !self.settings.remember_password || self.login_form.stored_for.as_ref() != Some(&key) {
                self.login_form.stored = None;
            }
            return;
        }
        if !self.settings.remember_password {
            if self.login_form.stored.take().is_some() {
                crate::credentials::forget(&key.0, &key.1);
            }
            self.login_form.stored_for = None;
            return;
        }
        if self.login_form.stored_for.as_ref() != Some(&key) {
            self.login_form.stored = crate::credentials::load(&key.0, &key.1);
            self.login_form.stored_for = Some(key);
        }
    }

    fn draw_ui(&mut self, ui: &mut egui::Ui, projector: &ui::hud::Projector, renderer: &Renderer) -> UiActions {
        let mut a = UiActions::default();
        let p = self.palette;
        let ctx = ui.ctx().clone();
        match self.screen {
            Screen::Login => {
                self.sync_remembered_password();
                let progress = self.login_progress.as_ref().map(|(m, f)| (m.as_str(), *f));
                self.login_info.poll();
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
                a.cancel_loading = ui::loading::show(
                    ui,
                    &p,
                    self.logo.as_ref(),
                    self.settings.loading_backdrop.then_some(&self.backdrop),
                    &info,
                );
            }
            Screen::World => {
                let icons = &self.skin.icons;
                let layout = &self.skin.layout;
                let maturity = self
                    .world
                    .main()
                    .and_then(|r| r.info.as_ref())
                    .map(|i| ui::bars::maturity_name(i.sim_access))
                    .unwrap_or("");
                let status = ui::bars::StatusInfo {
                    show_huds: self.settings.show_huds,
                    hud_zoom: self.world.hud_zoom,
                    ban_lines: self.settings.maps.ban_lines,
                    region: &self.world.region_name(),
                    parcel: &self.world.parcel_name,
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
                };
                let media = ui::audio::MediaState {
                    music_available: self.world.parcel.as_ref().is_some_and(|pc| !pc.music_url.is_empty()),
                    music_playing: self.music_playing,
                    media_available: self.settings.media.enabled
                        && self.world.parcel.as_ref().is_some_and(|pc| !pc.media_url.trim().is_empty()),
                    media_playing: self.media.parcel_playing(),
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
                    self.world.notifications.unread(),
                );
                a.bar = bar;
                a.audio = Some(audio_action);
                let mut mic = ui::bars::MicButton {
                    on: self.mic_on,
                    hold: self.settings.audio.mic_hold,
                    talking: self.talking(),
                    level: self.voice_level,
                    connected: self.voice_connected,
                    ..Default::default()
                };
                let (chat, bar2, bottom_top, chat_width_set) = ui::bars::bottom_bar(
                    ui,
                    &p,
                    icons,
                    layout,
                    &mut self.panels,
                    &mut self.chat_ui,
                    self.world.chat_unread + self.world.social.total_unread(),
                    self.world.agent.flying,
                    self.world.agent.is_sitting(),
                    self.camera.mouselook(),
                    &mut self.settings.chat_bar_width,
                    &mut mic,
                );
                if chat_width_set {
                    self.settings.save();
                }
                self.mic_on = mic.on;
                self.mic_button_held = mic.held;
                if mic.mode_changed {
                    self.settings.audio.mic_hold = mic.hold;
                    self.settings.save();
                }
                if mic.open_prefs {
                    self.panels.settings = true;
                    self.options_ui.tab = ui::options::TAB_AUDIO;
                }
                a.chat = chat;
                if !matches!(bar2, BarAction::None) {
                    a.bar = bar2;
                }
                // overlays
                let voice_levels = if self.demo {
                    crate::demo::voice_levels(ctx.input(|i| i.time), self.world.agent_id, self.talking(), self.voice.mic_level)
                } else {
                    self.voice.levels()
                };
                self.name_tags = ui::hud::draw(
                    &ctx,
                    &p,
                    &self.world,
                    projector,
                    self.camera.position,
                    !self.camera.mouselook(),
                    self.settings.name_tag_distance,
                    &voice_levels,
                    &mut self.voice_dots,
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
                );
                if !self.camera.mouselook() {
                    ui::minimap::draw_beacon(&ctx, &self.world, projector);
                }
                ui::hud::draw_attachments(&ctx, &self.world, &self.scene);
                {
                    let mods = crate::build::Mods {
                        shift: self.shift,
                        ctrl: self.ctrl,
                        alt: self.alt,
                    };
                    crate::build::ui::shortcuts(&ctx, &mut self.build, &mut self.world, &mut self.settings.build);
                    self.build.draw_overlay(&ctx, &self.world, &self.settings.build, mods);
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
                );
                if self.camera.mouselook() {
                    let c = ctx.content_rect().center();
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("xhair")));
                    painter.circle_stroke(c, 4.0, egui::Stroke::new(1.5, p.ink));
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
                    );
                }
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
                for c in ui::chat::show(
                    &ctx,
                    &p,
                    icons,
                    &mut self.emoji,
                    &self.avatar_pics,
                    &self.ui_images,
                    &mut self.world,
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
                        ui::chat::ConvAction::LeaveSession(id) => {
                            self.world.leave_session(id);
                            self.chat_ui.selected = None;
                        }
                        ui::chat::ConvAction::BlockGroupChat(id) => {
                            self.world.set_group_chat_blocked(id, true);
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
                contact_actions.extend(ui::contacts::dialog_windows(&ctx, &p, &mut self.world, &mut self.contacts_ui));
                for c in contact_actions {
                    self.on_contacts_action(c, &mut a);
                }
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
                let people = ui::people::people_window(&ctx, &p, &self.world, &mut self.panels, &mut self.people_ui, &mut people_map);
                for pa in people {
                    match pa {
                        ui::people::PeopleAction::OpenIm(id) => {
                            self.world.ui_sounds.push(UiSound::StartIm);
                            self.world.social.session_mut(id);
                            self.world.social.focus_im = Some(id);
                            self.panels.chat = true;
                        }
                        ui::people::PeopleAction::OfferTeleport(id) => a.offer_tp.push(id),
                        ui::people::PeopleAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
                        ui::people::PeopleAction::GroupChat(id) => {
                            self.world.ui_sounds.push(UiSound::StartIm);
                            self.world.start_group_chat(id);
                            self.panels.chat = true;
                        }
                        ui::people::PeopleAction::GroupChatBlocked(id, on) => self.world.set_group_chat_blocked(id, on),
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
                    }
                }
                self.poll_inventory_animation_stats();
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
                let facts = self.context_facts();
                let had_menu = self.context_menu.is_some();
                ui::context::show(&ctx, &p, &mut self.context_menu, &facts);
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
                if let Some((region, pos)) = self.place_confirm.clone()
                    && let Some(yes) = ui::context::place_teleport_confirm(&ctx, &p, &region)
                {
                    self.place_confirm = None;
                    if yes {
                        ui::context::request(&ctx, ui::context::CtxAction::TeleportToPlaceConfirmed(region, pos));
                    }
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
                    self.skin.layout.nav_bar_height,
                );
                if let Some(c) = self.media_cursor {
                    ctx.set_cursor_icon(c);
                }
                a.object_confirm = ui::object_actions::show(&ctx, &p, &mut self.interactions, &self.world);
                ui::object_actions::show_contents(&ctx, &p, &self.skin.icons, &mut self.interactions, &self.world);
                for (id, r) in ui::notifications::show(&ctx, &p, &self.skin.icons, &mut self.world.notifications, &mut self.notif_ui) {
                    let (cmds, url) = self.world.respond_notification(id, r);
                    if cmds.iter().any(|c| matches!(c, NetCommand::AcceptLure { .. })) {
                        self.begin_teleport(String::new(), false);
                    }
                    for c in cmds {
                        self.net.send(c);
                    }
                    // only web links (llLoadURL / goto URL), opened after the user's click
                    if let Some(u) = url.filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
                        ctx.open_url(egui::OpenUrl::new_tab(u));
                    }
                }
                let mut open = self.panels.about_land;
                if self.land_ui.show(
                    &ctx,
                    &p,
                    &mut self.world,
                    &self.ui_images,
                    &mut self.settings.audio.saved_streams,
                    self.demo,
                    &mut open,
                ) {
                    self.settings.save();
                }
                self.panels.about_land = open;
                a.set_display_name = self.display_name_ui.show(&ctx, &p, &self.world);
                // links and names clicked anywhere (LLAgentHandler)
                if let Some(id) = ui::profile::take_open_request(&ctx) {
                    self.profile_ui.open(&mut self.world, id);
                }
                // the "Flux" tab: a browser on the grid's web profiles
                let paths = self.media.plugins(&self.settings.media).cloned();
                let browser = crate::media::browser_settings(&crate::settings::cache_dir(), "fr");
                let cookie = self.media.openid.cookie();
                let profile_base = match self.settings.grid {
                    // AURORA_DEMO_FEED=<page>: the feed browser on a local page
                    _ if self.demo => std::env::var("AURORA_DEMO_FEED").ok(),
                    crate::settings::GridChoice::SecondLife => Some("https://my.secondlife.com/".to_owned()),
                    crate::settings::GridChoice::SecondLifeBeta => Some("https://my.secondlife-beta.com/".to_owned()),
                    crate::settings::GridChoice::Custom => self
                        .world
                        .login
                        .as_ref()
                        .map(|l| l.raw["web_profile_url"].to_string_value())
                        .filter(|u| !u.trim().is_empty()),
                };
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
                    &self.world,
                    &mut self.chat_ui.wanted_names,
                    &web,
                );
                for pa in acts {
                    match pa {
                        ui::profile::ProfileAction::Im(id) => a.ctx_action = Some(ui::context::CtxAction::Im(id)),
                        ui::profile::ProfileAction::OfferTeleport(id) => a.offer_tp.push(id),
                        ui::profile::ProfileAction::ToggleBlock(id) => a.ctx_action = Some(ui::context::CtxAction::ToggleBlock(id)),
                        ui::profile::ProfileAction::DisplayName => self.display_name_ui.open(),
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
                // place profiles: standalone windows and « Lieux »
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
                let places_acts = self.places_ui.show(&ctx, &mut c, &mut open, self.settings.landmarks_by_date);
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
                    }
                }
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
                        ui::minimap::MiniMapAction::StopTracking => self.world.map.track = None,
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
                let wm = self.worldmap_ui.show(
                    &ctx,
                    &p,
                    &mut self.world,
                    &mut self.map_tiles,
                    &map_cam,
                    &mut self.settings.maps,
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
                    }
                }
                let mut open = self.panels.perf;
                let net = self.net.stats.snapshot();
                let view = ui::perf::PerfView {
                    data: &self.perf,
                    render: &self.last_render,
                    gpu: &renderer.info,
                    net: &net,
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
                let reset = self.env_ui.show_lighting(&ctx, &p, &mut self.world.eep, &mut lighting);
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
                let mut open = self.panels.settings;
                self.options_ui.cache_usage = self.disk_cache.usage();
                self.options_ui.cache_clearing = self.disk_cache.is_clearing();
                let res = ui::options::show(&ctx, &p, &self.skin.icons, &mut self.settings, &mut self.options_ui, &mut open);
                if !open {
                    self.options_ui.capture = None;
                }
                if res.clear_cache {
                    self.disk_cache.clear();
                    self.net.send(NetCommand::ClearObjectCache);
                }
                a.settings_changed = res.changed;
                a.font_changed = res.font_changed;
                if res.refresh_devices {
                    self.devices_listed = false;
                }
                if res.reset {
                    // the panels must not write the old time of day back
                    self.panels.time_of_day = self.settings.time_of_day;
                    self.settings.save();
                    self.world.system_message("Tous les réglages ont été réinitialisés.");
                    ui::layout::forget(&ctx);
                }
                if self.panels.settings && !open {
                    self.settings.save();
                }
                self.panels.settings = open;
                self.settings.show_chat = self.panels.chat;
                self.settings.show_perf = self.panels.perf;
                self.settings.show_minimap = self.panels.minimap;
                self.settings.show_people = self.panels.people;
                self.settings.people_tab = self.panels.people_tab;
                self.settings.show_inventory = self.panels.inventory;
                self.settings.show_appearance = self.panels.appearance;
                if self.panels.time_of_day != self.settings.time_of_day {
                    self.settings.time_of_day = self.panels.time_of_day;
                }
                if let Some(msg) = self.world.alerts.pop_front() {
                    log::info!("alert: {msg}");
                }
                // teleport screen (and the login loading screen fading out)
                if let Some(o) = &self.tp_overlay {
                    // quick fade in, fade out at the end
                    let fade_in = if o.loading_end {
                        1.0
                    } else {
                        o.start.elapsed().as_secs_f32() / 0.25
                    };
                    let fade_out = o.end.map_or(1.0, |e| 1.0 - e.elapsed().as_secs_f32() / ui::loading::FADE_SECONDS);
                    let alpha = fade_in.min(fade_out).clamp(0.0, 1.0);
                    let (title, sub, detail) = if o.loading_end {
                        ("Chargement du monde", self.world.region_name(), String::new())
                    } else {
                        let sub = if o.dest.is_empty() {
                            self.world.region_name()
                        } else {
                            o.dest.clone()
                        };
                        let detail = if o.shown >= 0.985 {
                            "Bienvenue !".to_owned()
                        } else {
                            self.world.tp_status.clone()
                        };
                        ("Téléportation", sub, detail)
                    };
                    ui::loading::teleport_overlay(
                        &ctx,
                        &p,
                        self.logo.as_ref(),
                        self.settings.loading_backdrop.then_some(&self.backdrop),
                        title,
                        &sub,
                        &detail,
                        Some(o.shown),
                        alpha,
                    );
                }
            }
        }
        a
    }
}

#[derive(Default)]
struct UiActions {
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
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        // captures keep a fixed, predictable window
        let capture = self.capture.is_some();
        let (mut w, mut h) = (self.settings.window_width.max(800), self.settings.window_height.max(600));
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
        // maximized after creation: the creation attribute is not reliable on Windows
        if self.settings.window_maximized && !capture {
            window.set_maximized(true);
        }
        let size = window.inner_size();
        if std::env::var_os("AURORA_NOVSYNC").is_some() {
            self.settings.vsync = false;
        }
        let renderer = match Renderer::new(window.clone(), size.width, size.height, self.settings.vsync) {
            Ok(r) => r,
            Err(e) => {
                log::error!("renderer initialisation failed: {e}");
                eprintln!("Aurora Viewer: impossible d'initialiser Vulkan : {e}");
                event_loop.exit();
                return;
            }
        };
        log::info!(
            "renderer ready: {} via {} ({} texture slots, GPU timers: {})",
            renderer.info.name,
            renderer.info.backend,
            renderer.info.max_textures,
            renderer.info.timestamps
        );
        let mut renderer = renderer;
        renderer.apply_settings(self.settings.render_settings());
        self.monitor_hz = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| mhz as f32 / 1000.0)
            .unwrap_or(60.0);
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
            renderer,
            egui_state,
        });
        if std::env::var_os("AURORA_DEMO").is_some() {
            self.demo = true;
            if let Ok(mode) = std::env::var("AURORA_DEMO_LOGIN") {
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
            self.scene.anims.insert(
                crate::demo::SEAT_ANIM,
                Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::seat_animation(), &rig)),
            );
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
                for (id, settings) in crate::demo::env::settings_assets() {
                    self.scene.settings.insert_local(id, settings);
                }
            }
            if std::env::var_os("AURORA_DEMO_OPTIONS").is_some() {
                self.panels.settings = true;
                self.panels.perf = false;
                self.options_ui.tab = std::env::var("AURORA_DEMO_OPTIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            // AURORA_DEMO_PERF=compact|full: the performance window in that view
            if let Ok(v) = std::env::var("AURORA_DEMO_PERF") {
                self.panels.perf = true;
                ui::perf::set_full(&self.egui_ctx, v.trim() == "full");
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
            if std::env::var_os("AURORA_DEMO_UI").is_some() {
                self.panels.people = true;
                self.panels.people_tab = std::env::var("AURORA_DEMO_UI").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
                self.panels.inventory = true;
                self.panels.chat = true;
                self.panels.perf = false;
                self.panels.minimap = false;
            }
            // AURORA_DEMO_PLACE=1|lagune|nordheim|pinede|faille|inconnue: the
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
            // AURORA_DEMO_PLACES=favoris|reperes|historique: « Lieux » on a tab
            if let Some(tab) = std::env::var("AURORA_DEMO_PLACES").ok().and_then(|v| ui::places::tab_from_name(&v)) {
                self.places_ui.open_tab(tab);
                self.panels.places = true;
                self.panels.perf = false;
            }
            // AURORA_DEMO_PLACE_WINDOW=1: the standalone windows instead
            if std::env::var_os("AURORA_DEMO_PLACE_WINDOW").is_some() {
                self.settings.standalone_place_details = true;
            }
            // AURORA_DEMO_PROFILE=loup|nova|friend|self[:tab]: a profile window
            if let Ok(v) = std::env::var("AURORA_DEMO_PROFILE") {
                let (who, tab) = v.split_once(':').unwrap_or((v.as_str(), "0"));
                let id = match who {
                    "self" => crate::demo::DEMO_AGENT,
                    "nova" => crate::demo::DEMO_NOVA,
                    "friend" => crate::demo::DEMO_FRIEND,
                    _ => crate::demo::DEMO_LOUP,
                };
                self.profile_ui.open(&mut self.world, id);
                self.profile_ui.set_tab(id, tab.parse().unwrap_or(0));
                self.panels.perf = false;
            }
            // AURORA_DEMO_LAND=<onglet>[:owner]: About Land on that tab
            if let Some((tab, _)) = crate::demo::land::scenario() {
                self.land_ui.set_tab(&tab);
                self.panels.about_land = true;
                self.panels.perf = false;
            }
            // AURORA_DEMO_CONTACTS=amis|groupes|cercles|detache|ajout: the
            // Contacts tab of Conversations (or its torn-off window, or the
            // resident picker of « Ajouter... »)
            if let Ok(v) = std::env::var("AURORA_DEMO_CONTACTS") {
                self.panels.chat = true;
                self.panels.perf = false;
                self.chat_ui.contacts = true;
                self.settings.contacts.tab = match v.as_str() {
                    "groupes" => 1,
                    "cercles" => 2,
                    _ => 0,
                };
                self.settings.contacts.torn_off = v == "detache";
                self.panels.contacts = v == "detache";
                if v == "ajout" {
                    self.contacts_ui.pick_new_friend();
                }
            }
            // AURORA_DEMO_CONV=1: the demo group chat with its participants
            if std::env::var_os("AURORA_DEMO_CONV").is_some() {
                self.panels.chat = true;
                self.chat_ui.selected = Some(crate::demo::DEMO_GROUP1);
                self.chat_ui.show_profile = true;
            }
            // captures: AURORA_DEMO_MAP=1 opens the mini-map and the world map
            // (with a tracked spot east of the plaza)
            if std::env::var_os("AURORA_DEMO_MAP").is_some() {
                self.panels.minimap = true;
                self.panels.world_map = std::env::var("AURORA_DEMO_MAP").as_deref() != Ok("mini");
                self.panels.perf = false;
                self.world.map.track_location(256000.0 + 200.0, 256000.0 + 150.0, 25.0, false);
            }
            // captures: AURORA_DEMO_SIT keeps the toolbar sit button visible
            if std::env::var_os("AURORA_DEMO_SIT").is_some() {
                self.panels.perf = false;
            }
            // captures: AURORA_DEMO_NOTIF=1 opens the notification list
            if std::env::var_os("AURORA_DEMO_NOTIF").is_some() {
                self.notif_ui.open = true;
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
            for ev in crate::demo::events() {
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
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
            if let Ok(view) = std::env::var("AURORA_DEMO_INVENTORY") {
                crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                self.panels.inventory = true;
                self.panels.perf = false;
                self.panels.appearance = false;
                self.panels.minimap = false;
                self.world.notifications = Default::default();
                if let Some(id) = crate::world::inventory::demo::target(&view) {
                    self.inventory_ui.show_original(&self.world.inventory, id);
                    if view == "new-folder" {
                        if let Ok(change) = crate::world::inventory::actions::create_folder(id, "Nouveau dossier") {
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
                            name: if view == "new-script" { "Nouveau script" } else { "Nouvelle note" }.into(),
                        });
                    } else if matches!(view.as_str(), "folder-window" | "folder-window-search") {
                        self.inventory_ui.open_folder_window(id);
                        if view == "folder-window-search"
                            && let Some(window) = self.inventory_ui.windows.last_mut()
                        {
                            window.state.search = "démonstration".into();
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
                    } else if view == "animation-open" {
                        self.inventory_ui.preview = Some(id);
                    } else if view.starts_with("image") {
                        self.inventory_ui.thumbnail.open(id);
                        self.inventory_ui.thumbnail.picker_open = view == "image-picker";
                        self.inventory_ui.thumbnail.photo_open = matches!(view.as_str(), "image-photo" | "image-photo-save");
                    } else {
                        self.inventory_ui.demo_menu = Some(id);
                    }
                } else {
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
                    p.media.current_url = crate::demo::DEMO_MEDIA_PAGE.into();
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
            self.stream_demo = crate::demo::stream::StreamDemo::from_env();
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
                for ev in crate::demo::crowd_events(n, rigged) {
                    self.world.apply(ev);
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
                    crate::demo::DEMO_MEDIA_PAGE.to_string()
                } else {
                    v
                };
                let (ev, object, face) = crate::demo::media_screen();
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
                self.media.demo_entry(object, face, &url);
            }
            // HUD fixtures exercise screen-space rendering and the same
            // linked-object touch / inventory paths as real attachments.
            if let Ok(mode) = std::env::var("AURORA_DEMO_HUDS") {
                for ev in crate::demo::hud::events() {
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
                if mode == "zoom" || mode == "drag-zoom" {
                    self.world.hud_zoom = 0.5;
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
                    self.media.demo_entry(hud.full_id, face, crate::demo::DEMO_MEDIA_PAGE);
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
                    crate::demo::DEMO_MEDIA_PAGE.to_string()
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
                // in world: keep the last view (loading screens) before leaving
                if self.in_world() && self.settings.loading_backdrop && self.capture.is_none() && self.closing.is_none() {
                    self.closing = Some(Instant::now());
                    self.want_scene_capture = true;
                    return;
                }
                self.shutdown(event_loop);
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
            }
            WindowEvent::Focused(true) => self.focused = true,
            WindowEvent::Focused(false) => {
                self.focused = false;
                self.down.clear();
                self.right_drag = false;
                self.mic_button_held = false;
                self.on_left_release();
                if self.temp_run.take().is_some() && !self.always_run {
                    self.send(NetCommand::SetAlwaysRun(false));
                }
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                self.last_input = Instant::now();
                if let PhysicalKey::Code(code) = ke.physical_key {
                    let pressed = ke.state == ElementState::Pressed;
                    // learn the keyboard layout for the shortcut labels
                    if pressed
                        && !self.shift
                        && !self.ctrl
                        && !self.alt
                        && let winit::keyboard::Key::Character(s) = &ke.logical_key
                        && !format!("{code:?}").starts_with("Digit")
                        && !format!("{code:?}").starts_with("Numpad")
                    {
                        crate::keybinds::learn(code, s);
                    }
                    // shortcut being captured in the preferences
                    if self.options_ui.capture.is_some() {
                        if pressed && !ke.repeat && !Input::is_modifier(code) {
                            let input = (code != KeyCode::Escape).then(|| Input::key(code));
                            self.finish_capture(input);
                        }
                        return;
                    }
                    // Releases always go through so keys never get stuck.
                    let typing = self.egui_ctx.egui_wants_keyboard_input();
                    // a profile's web page that was clicked gets the keyboard
                    if !typing && let Some(p) = self.profile_ui.focused_web() {
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
                    if (!pressed || (!resp.consumed && !typing) || shortcut) && !(pressed && ke.repeat) {
                        self.on_input(Input::key(code), pressed);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.last_input = Instant::now();
                let pressed = *state == ElementState::Pressed;
                let over_ui = self.egui_ctx.is_pointer_over_egui() || self.egui_ctx.egui_wants_pointer_input();
                // shortcut being captured: middle / side buttons bind, left / right cancel
                if self.options_ui.capture.is_some() {
                    if pressed {
                        self.finish_capture(Input::mouse(*button));
                    }
                    return;
                }
                match button {
                    MouseButton::Right => self.on_right_button(pressed, over_ui),
                    MouseButton::Left if pressed => {
                        if !over_ui && self.in_world() && !self.camera.mouselook() && !self.build_mouse_down() {
                            self.on_left_press();
                        }
                    }
                    MouseButton::Left => self.on_left_release(),
                    other => {
                        if let Some(input) = Input::mouse(*other) {
                            // bindable buttons (push-to-talk on the wheel click...)
                            if !pressed || !over_ui {
                                self.on_input(input, pressed);
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } if !self.egui_ctx.is_pointer_over_egui() && self.in_world() => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
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
                if !on_media {
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
            if !self.in_world() {
                return;
            }
            let dy = if self.settings.invert_mouse { -delta.1 } else { delta.1 };
            self.camera_mouse_delta(delta.0 as f32, dy as f32);
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        self.record_window_geometry();
        if let Some(g) = &self.gfx {
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
    use super::{capture_files, frame_cap};
    use crate::settings::Settings;
    use std::path::{Path, PathBuf};

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
