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

mod about;
mod animation_explorer;
mod ao;
mod app_links;
mod area_search;
mod attention;
mod audio_mute;
mod avatar_render_settings;
mod bridge;
mod movelock;
mod particle_editor;
mod portals;
mod poser;
mod sound_explorer;
// Bug report form (disabled, see main.rs):
// mod bug_report;
mod alt_graph;
mod avatar_health;
mod blender_live;
mod blender_shape;
mod buy_currency;
mod buy_land;
mod camera_presets;
mod chat_commands;
mod click_pick;
mod color_picker;
mod contents_copy;
mod context_menu;
mod controls;
mod crash_state;
mod cursor_hold;
mod derender;
mod describe;
mod dev_toolbox;
mod folders;
use folders::chat_log_root;
mod contents_drop_demo;
mod dof;
mod double_click;
mod driver_check;
mod env_apply;
mod env_day_editor;
mod env_editor;
mod external_links;
mod follow;
mod gestures;
mod give_demo;
mod gpu_stall;
mod graphics_start;
mod grid_status;
mod hover_height;
mod hover_tips;
mod hud_drag;
mod imports;
mod inspect;
mod inventory;
mod inventory_drag;
mod inventory_hover;
mod inventory_thumbnail;
mod joystick;
mod land_moderation;
mod link_health;
mod marketplace_listings;
mod money_tracker;
mod monitor;
mod movement;
mod object_actions;
mod object_menu;
mod object_menu_demo;
mod own_motion;
mod pay;
mod photo_isolate;
mod physics_editor;
mod plate_click;
mod pointing;
mod region_restart;
mod region_window;
mod relogin;
mod report_abuse;
mod reset_skeleton;
mod rlva;
mod safe_graphics;
mod script_clicks;
mod scripts;
mod settings_backup;
mod shape_editor;
mod snapshot;
mod social_menu_demo;
mod stream_list;
mod suggest_demo;
mod task_notecards;
mod teleport_lookup;
mod teleport_screen;
mod texture_library;
mod texture_refresh;
mod viewer_stats;
mod viewfinder;
mod wearable_editor;
mod world_click_demo;
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
    /// When the region may show and how the bar finishes.
    mode: teleport_screen::Mode,
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
            mode: if loading_end {
                teleport_screen::Mode::Old
            } else {
                teleport_screen::Mode::for_next_teleport()
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseMode {
    None,
    /// Left button held on our avatar, or the optional world chord: steer it.
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
    /// The button has walked, even if the left button has since been released.
    walked: bool,
}

impl RightGesture {
    fn opens_menu(&self) -> bool {
        !self.orbit && !self.walked
    }

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
const FLY_TIME: f32 = 0.5;
/// Radians: a change of the viewing angle below this is not sent in AgentFOV
/// (an eased FOV zoom creeps toward its target for ever).
const FOV_SEND_EPSILON: f32 = 1e-3;

/// The wolf logo (window icon, login and loading screens, About, system
/// notifications), read from the branding folder: one file for the viewer,
/// the README and the exe icon.
pub(crate) const LOGO_PNG: &[u8] = include_bytes!("../../../assets/branding/aurora-logo.png");

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
    bridge_runtime: crate::world::bridge::Bridge,
    native_anchor: crate::movement::movelock::Controller,
    anchor_stands: crate::ao::anchor::StandCheck,
    /// Keep the verified standing set through an asynchronous stand-up reply.
    anchor_pose: Option<(u64, crate::ao::Set)>,
    bridge_services: crate::world::bridge::services::Services,
    bridge_demo: crate::world::bridge::demo::Demo,
    movelock_ui: ui::movelock::MoveLockUi,
    movement_ui: ui::movement::MovementUi,
    movement_runtime: crate::movement::Runtime,
    movement_probe: (Option<Instant>, bool, Option<Vec3>),
    pass_wall_at: Option<Instant>,
    scene: Scene,
    probes: crate::scene::probes::ProbeManager,
    /// Throttle of the "sky frame" log line.
    sky_log: crate::world::env::SkyLog,
    camera: Camera,
    input: MoveInput,
    joystick: joystick::State,
    /// Keys and bindable mouse buttons held down.
    down: HashSet<Input>,
    shift: bool,
    ctrl: bool,
    /// Command held, on macOS only (aurora_media::Modifiers::command).
    command: bool,
    screen: Screen,
    login_form: LoginForm,
    // Bug report form (disabled, see main.rs):
    // bug_report: ui::bug_report::BugReportUi,
    login_progress: Option<(String, f32)>,
    /// `AURORA_DEMO_LOGIN=add-mfa`: when the offline login server asks for
    /// the second factor, after the request has been pending a moment.
    demo_mfa_challenge: Option<std::time::Instant>,
    /// The wait for a previous session still on the grid (#956).
    relogin: relogin::Relogin,
    /// The viewer's part of Linden Lab's viewer statistics (app/viewer_stats.rs).
    viewer_stats: viewer_stats::Tracker,
    panels: Panels,
    /// Conversations and the mini-map as last frame left them (their
    /// startup settings follow an open or a close only).
    startup_windows: ui::StartupWindows,
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
    /// The Performance window's full view (ui/monitor) and what its
    /// snapshots need (app/monitor.rs).
    monitor: ui::monitor::Monitor,
    monitor_feed: monitor::MonitorFeed,
    /// The interface hidden (Alt+Shift+U) and the bars in mouselook.
    interface: ui::interface::Interface,
    /// AURORA_PROFILE: frame laps and the summary line in the log.
    frame_profile: FrameProfile,
    /// What the network did each second: Performance window, and the
    /// "net summary" line in the log with AURORA_PROFILE.
    net_profile: crate::net_profile::NetProfile,
    /// AURORA_PROFILE: the lines of every 10 s (messages, HTTP, assets,
    /// avatars, memory; deep_profile.rs).
    deep_profile: crate::deep_profile::DeepProfile,
    /// `AURORA_SYNC_SLICE=alt`: seconds of the frame profile so far.
    sync_alt: u64,
    /// AURORA_NAME_TAGS=alt: this second draws the classic tags.
    name_tags_alt_classic: bool,
    /// AURORA_DEMO_TAGS: the set of states its residents show.
    demo_tag_states: Option<bool>,
    /// AURORA_DEMO_CHAT_BUBBLES: when its residents' chat was last sent.
    demo_bubble_chat: f64,
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
    /// The camera's vertical angle last sent in AgentFOV.
    sent_fov: f32,
    /// The system cursor hidden and locked by mouselook or a camera drag.
    cursor_hold: cursor_hold::CursorHold,
    /// Alt held in mouselook: the cursor given back, the view still.
    alt_cursor: cursor_hold::AltCursor,
    numeric_drag: ui::numeric_drag::NumericDrag,
    /// Right-click menu of the text fields (Cut, Copy, Paste…).
    text_menu: ui::text_menu::TextMenu,
    alt: bool,
    /// AltGr held: Ctrl + Alt for shortcuts and the mouse (app/alt_graph.rs).
    alt_graph: bool,
    left_down: bool,
    /// An unhandled world press may start steering when the right button joins.
    world_mouse_walk_pending: bool,
    /// Clicks that wait for the depth under them, and what follows them.
    clicks: click_pick::ClickWait,
    script_clicks: script_clicks::ScriptClicks,
    /// Displacement while the right button is held (click vs orbit drag).
    right_gesture: RightGesture,
    /// Open right-click menu.
    context_menu: Option<ui::context::ContextMenu>,
    context_customization: ui::context::Customization,
    avatar_textures: ui::context::AvatarTexturesUi,
    object_profile: ui::context::ObjectProfileUi,
    pathfinding_ui: ui::context::PathfindingUi,
    /// What a left-button drag does (steer the avatar, orbit a focus point).
    mouse_mode: MouseMode,
    /// Build tools (selection, manipulators, create, terraform).
    build: crate::build::BuildTool,
    ik_editor: crate::ik_edit::Editor,
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
    follow_ui: ui::follow::FollowUi,
    portal_ui: ui::portals::PortalUi,
    /// Build › Edit particles (ParticleEditor).
    particle_editor: crate::build::ui::particles::ParticleEditor,
    particle_inject: particle_editor::Inject,
    portal_drop: Option<portals::landmark::Pending>,
    portal_journey: Option<portals::Journey>,
    /// A peer contains a full World; keep it off the Windows startup stack.
    portal_demo_peer: Option<Box<portals::demo::Peer>>,
    follow_recovery: Option<follow::Recovery>,
    /// The control flags the walk asks for this frame.
    autopilot_flags: u32,
    autopilot_stop_pending: bool,
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
    /// What the simulator was told of the static eyes animation since the
    /// last arrival (FSStaticEyes).
    static_eyes: crate::world::lookat::StaticEyes,
    /// The world point the cursor was over last frame, and the object it
    /// lies on (the hover pick of the cursor icon), for the look-at.
    look_hover: Option<(Option<usize>, Vec3)>,
    /// The world's hover tips (`app/hover_tips.rs`).
    hover_tips: hover_tips::HoverTipsState,
    /// Fade-in of the look-at targets (`ui::lookat_targets`).
    look_at_fades: ui::lookat_targets::Fades,
    /// Offline demo: eye tracking on without touching the saved settings.
    demo_look_at: bool,
    /// AURORA_DEMO_LOOKAT=ahead: "no look-at target, even for me" turned
    /// on (in memory) from frame 500.
    demo_look_ahead: bool,
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
    /// saves the window layout shortly after it changes (ui/layout.rs)
    layout_autosave: ui::layout::Autosave,
    loading_stage: String,
    move_complete_at: Option<Instant>,
    /// Teleport diagnostics still to log (seconds after the arrival).
    tp_checks: Vec<f32>,
    /// Debug: culling view kept while "Freeze culling" is on.
    frozen_cull: Option<CullView>,
    /// The camera's recent speed, for the textures' importance to it.
    texture_motion: crate::scene::texture_importance::CameraMotion,
    /// Interface sounds caused by the widgets (clicks, keys, windows).
    sound_cues: ui::sound_cues::SoundCues,
    /// Last frame: microphone on, context menu open, interactive script
    /// dialogs (their sounds).
    mic_was: bool,
    context_menu_was: bool,
    /// "Delete" waiting for a yes: (object, why it asks).
    /// The objects to delete once confirmed: one from a world menu, the
    /// selected rows of the Area Search menu.
    delete_confirm: Option<(Vec<uuid::Uuid>, String)>,
    /// "Freeze" / "Eject" waiting for an answer (app/land_moderation.rs).
    moderation_ask: Option<land_moderation::Ask>,
    /// The object menus' questions and script queue (app/object_menu.rs).
    object_menu: object_menu::ObjectMenu,
    /// Teleport asked by a place link, waiting for a yes (TeleportViaSLAPP),
    /// with the state of its "Do not show again" box.
    place_confirm: Option<(String, Vec3, bool)>,
    /// External web link waiting for the warning's answer, with the state of
    /// its "Do not warn me again" box and, for a dangerous link, the reason.
    url_confirm: Option<(String, bool, Option<&'static str>)>,
    /// The media filter's question on screen.
    media_filter_prompt: ui::media_filter::PromptUi,
    script_dialogs_were: usize,
    /// Last tracker beacon sound (FIRE-16969).
    beacon_sound_at: Option<Instant>,
    wind_sound: crate::scene::wind_sound::WindSound,
    /// Movement keys held since, and the server position then (stuck agent).
    stuck_watch: Option<(Instant, Vec3)>,
    stuck_logged: Option<Instant>,
    /// Attachment diagnostics written to the log (0, 1 or 2 times).
    attachments_logged: u8,
    /// A "Debug to log" press waiting for the names it asked
    /// (app/describe.rs): when it is written, what, and where.
    describe: Option<describe::Pending>,
    /// The Dev Toolbox: the window's state, and the app's (app/dev_toolbox.rs).
    dev_toolbox: ui::dev_toolbox::DevToolboxUi,
    toolbox: dev_toolbox::Toolbox,
    /// The region console's command line (Advanced › Region console).
    sim_console_input: ui::sim_console::ConsoleInput,
    loading_frac: f32,
    loading_target: f32,
    frame_count: u64,
    demo_rlva_frame: Option<u64>,
    rlva_login: crate::world::rlva::login::State,
    named_teleport: teleport_lookup::NamedTeleport,
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
    /// Only the friend-join demo changes regions and delays target data.
    demo_tp_region: Option<aurora_net::RegionHandle>,
    demo_friend_join_target: Option<(Instant, Vec3)>,
    /// Delayed flight signal, then the landing parcel for join-fly-late.
    demo_friend_join_flight: Option<(Instant, bool)>,
    /// AURORA_DEMO_RADAR=tp: the radar starts in another region.
    demo_radar_tp: bool,
    /// AURORA_DEMO_RADAR=walk: a resident comes and goes.
    demo_radar_walk: bool,
    /// Destination of the teleport being started (teleport screen).
    tp_dest: Option<String>,
    /// Window moved / resized: its normal geometry is recorded once settled.
    geom_changed: Option<Instant>,
    frame_start: Instant,
    /// A Windows Alt code being typed in a text field (alt_codes.rs), and
    /// the characters it gave, for egui's next frame.
    alt_codes: crate::alt_codes::AltCodes,
    alt_text: Vec<String>,
    people_ui: ui::people::PeopleUi,
    /// People's friends tab as Conversations › Contacts shows it: its own
    /// filter, selection and sheet.
    contacts_people_ui: ui::people::PeopleUi,
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
    area_search_ui: ui::area_search::AreaSearchUi,
    sound_explorer_ui: ui::sound_explorer::SoundExplorer,
    animation_explorer_ui: ui::animation_explorer::AnimationExplorer,
    animation_explorer: animation_explorer::ExplorerState,
    avatar_render_settings_ui: ui::avatar_render_settings::AvatarRenderSettings,
    area_search_feed: area_search::Feed,
    /// "Asset blacklist" (World menu).
    blacklist_ui: ui::blacklist::BlacklistUi,
    /// "Bumps, pushes & hits" (Help menu).
    bumps_ui: ui::bumps::BumpsUi,
    /// Help › Grid status: the grid's feed (`app/grid_status.rs`), whether
    /// its button stands on a toolbar, and whether the window was open
    /// last frame.
    grid_status: crate::grid_status::GridStatus,
    grid_status_listed: bool,
    grid_status_was_open: bool,
    about: about::About,
    /// The parcel radio's title and the ones before (World › Stream title).
    stream_titles: crate::stream_title::StreamTitles,
    stream_title_ui: ui::stream_title::StreamTitleUi,
    buy_currency_ui: ui::buy_currency::BuyCurrencyUi,
    /// The Pay window of a resident or a group (`ui/pay.rs`).
    pay_ui: ui::pay::PayUi,
    /// "Money tracker" (Me menu): the L$ received and paid.
    money_tracker_ui: ui::money_tracker::MoneyTrackerUi,
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
    grids_ui: ui::grids::Manager,
    audio_ui: ui::audio::AudioUi,
    notif_ui: ui::notifications::NotifUi,
    /// A friendship offer in the middle of the screen (#465).
    friend_prompt: ui::friend_prompt::FriendPromptUi,
    region_message_prompt: ui::region_message_prompt::RegionMessagePromptUi,
    /// The region restart alert (app/region_restart.rs).
    region_restart: region_restart::State,
    /// Taskbar flashes and system notifications (app/attention.rs).
    attention: attention::State,
    /// After the link with the region was lost: the grid, the user and the
    /// place "Reconnect" goes back to (ui/link_health.rs).
    reconnect_place: Option<(String, String, aurora_net::login::StartLocation)>,
    /// SLURLs from outside and the command line's login (app/external_links.rs).
    external: external_links::ExternalLinks,
    /// AURORA_DEMO_LINK: a silence of the region to show, or a lost link.
    demo_link_health: Option<link_health::LinkHealthDemo>,
    /// AURORA_DEMO_SLURLS: region lookups the demo server answers late.
    demo_lookups: Vec<(Instant, NetCommand)>,
    /// The object of an `objectim` link clicked (LLInspectRemoteObject).
    remote_object: Option<crate::slapp::RemoteObject>,
    /// Parcels asked for the labels of `parcel` links (once each).
    app_link_parcels: HashSet<uuid::Uuid>,
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
    /// The parcel music's title last told (ShowStreamMetadata).
    stream_title_said: Option<String>,
    /// Every sound is cut because the window is minimised (MuteWhenMinimized).
    minimized_mute: audio_mute::MinimizedMute,
    /// When the crash report's state was last noted (app/crash_state.rs).
    crash_state_at: Option<Instant>,
    /// The graphics card stopped finishing frames (app/gpu_stall.rs).
    gpu_stall: gpu_stall::GpuStall,
    /// The Down key's latch (FSCrouchToggle).
    crouch_toggle: crate::movement::crouch_toggle::CrouchToggle,
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
    name_tag_visibility: ui::hud::NameTagVisibility,
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
    /// The picture each of them shows: a profile that names another one
    /// (saved by us, read again) replaces it.
    avatar_pic_sources: std::collections::HashMap<uuid::Uuid, uuid::Uuid>,
    /// The same textures by picture, for the residents who share one.
    avatar_pic_images: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    /// Our login portrait (texture and saved file): agent, picture shown.
    login_portrait_source: Option<(uuid::Uuid, uuid::Uuid)>,
    /// Interface images (pictures included) first shown from the coarse
    /// levels of a progressive download, by texture: width held, since
    /// when. A finer copy replaces their pixels in place.
    ui_coarse: std::collections::HashMap<uuid::Uuid, (u32, Instant)>,
    /// Other interface images by asset id (profiles: 1st life picture,
    /// group insignias).
    ui_images: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    inventory_hover_images: inventory_hover::Images,
    ui_image_masks: std::collections::HashMap<uuid::Uuid, crate::world::rlva::effects::AlphaMask>,
    rlva_overlay_slots: std::collections::HashMap<uuid::Uuid, u32>,
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
    /// AURORA_DEMO_REZ_EFFECT: objects and an avatar appearing with the rez effect.
    rez_demo: Option<crate::demo::rez::RezDemo>,
    arrival_demo: Option<crate::demo::arrival::ArrivalDemo>,
    outfit_restore_demo: Option<crate::demo::outfit_restore::OutfitRestoreDemo>,
    /// AURORA_DEMO_TEXREFRESH: "Refresh textures" on a stale cached texture.
    tex_refresh_demo: Option<crate::demo::tex_refresh::TexRefreshDemo>,
    /// AURORA_DEMO_SCULPT_MISSING: a sculpt whose map is missing.
    sculpt_missing_demo: Option<crate::demo::sculpt::MissingMapDemo>,
    /// AURORA_DEMO_TEXTURE_MISSING: boxes whose texture is missing.
    texture_missing_demo: Option<crate::demo::missing_texture::MissingTextureDemo>,
    /// AURORA_DEMO_EXPERIENCE: experiences asking (door, pad, greeter).
    experience_demo: Option<crate::demo::experience::ExperienceDemo>,
    /// AURORA_DEMO_TP_AWAY: the demo region's coarse locations while away.
    away_coarse: Vec<(uuid::Uuid, Vec3)>,
    /// AURORA_DEMO_GEOMETRY: meshes filling the geometry arena.
    geometry_demo: Option<crate::demo::geometry::GeometryDemo>,
    /// Geometry each region needed at its last visit: the arena grows to
    /// it behind the loading screen.
    geometry_needs: crate::scene::geometry_need::GeometryNeeds,
    /// AURORA_DEMO_TURN: fast camera turns.
    turn_demo: Option<crate::demo::turn::TurnDemo>,
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
    texture_library: ui::texture_library::LibraryUi,
    library_drag: texture_library::DragState,
    library_demo_seeded: bool,
    gesture_state: gestures::State,
    imports: crate::imports::Imports,
    blender_live: crate::blender_live::Live,
    task_transfer: std::collections::VecDeque<ui::inventory::task_drop::PendingTransfer>,
    /// "Copy to inventory" of objects' contents (app/contents_copy.rs).
    contents_copies: contents_copy::ContentsCopies,
    /// Inventory edits asked by the world (`World::take_inventory_edits`),
    /// awaiting their reply.
    world_edits: std::collections::HashSet<uuid::Uuid>,
    /// AURORA_DEMO_INVENTORY=offer: folder the offered item was last shown in.
    demo_offer_shown: Option<uuid::Uuid>,
    ao_ui: ui::ao::AoUi,
    ao_catalog: crate::ao::inventory::Catalog,
    ao_bootstrap: crate::ao::bootstrap::Bootstrap,
    ao_runtime: crate::ao::runtime::Runtime,
    ao_agent: uuid::Uuid,
    ao_missing: HashSet<uuid::Uuid>,
    ao_demo: Option<String>,
    ao_default_applied: bool,
    /// AURORA_DEMO_LINK: the demo's simulated link for our avatar
    /// (demo_link.rs), created at the first frame in the world.
    demo_link_cfg: Option<crate::demo::link::Config>,
    demo_link: Option<crate::demo::link::DemoLink>,
    /// When the prediction's calibration was last logged (app/own_motion.rs).
    own_motion_logged: Option<Instant>,
    /// The demo's seat our avatar sits on (its place and size), to stand up
    /// beside it (app/own_motion.rs).
    demo_seat: Option<(Vec3, Vec3)>,
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
    physics_ui: crate::physics_editor::State,
    wearable_ui: crate::wearable_editor::State,
    /// Me › Avatar health and the customise animation.
    avatar_health: avatar_health::AvatarHealth,
    last_social_poll: Instant,
    /// Last keyboard / mouse button input (automatic away, AFKTimeout), and
    /// whether the current away status was set by that timer.
    last_input: Instant,
    auto_away: bool,
    /// What the network was last told about the neighbouring regions.
    neighbours_refused: bool,
    /// The cache and transcript folders being chosen (app/folders.rs).
    folders: folders::Folders,
    /// The settings backup being written or read (app/settings_backup.rs).
    settings_backup: settings_backup::Job,
    /// The stream list being exported or imported (app/stream_list.rs).
    stream_list: stream_list::Job,
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

/// The interface font, or AURORA_UI_FONT for this launch (a built-in name
/// or `system:<path>`): every font compared in one run of captures,
/// Preferences left alone.
fn ui_font(settings: &Settings) -> String {
    std::env::var("AURORA_UI_FONT")
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| settings.font.clone())
}

/// The login screen keeps one shortcut of the viewer: the preferences
/// (Ctrl+P unless rebound), as the Legacy Viewer's login menu does
/// (menu_login.xml, "Preferences..." on control|P).
fn login_shortcut(screen: &Screen, action: Option<Action>) -> bool {
    matches!(screen, Screen::Login) && action == Some(Action::Preferences)
}

/// A Ctrl / Alt shortcut of the viewer still works while a text field has
/// the focus, as the Legacy Viewer's menu accelerators go before the focused
/// field (LLViewerWindow::handleKey); the field keeps its own editing
/// shortcuts (copy, paste, undo, words...).
fn shortcut_while_typing(typing: bool, ctrl: bool, alt: bool, code: KeyCode, bound: bool) -> bool {
    typing && (ctrl || alt) && !crate::keybinds::is_text_edit_key(code) && bound
}

/// The interface scale, or AURORA_UI_SCALE for this launch (brought into
/// the range of the Preferences, 0.75–2).
fn ui_scale(settings: &Settings) -> f32 {
    std::env::var("AURORA_UI_SCALE")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|s| s.is_finite())
        .map_or(settings.ui_scale, |s| s.clamp(0.75, 2.0))
}

/// The text size, or AURORA_TEXT_SIZE for this launch (brought into the
/// range of the Preferences, never saved).
fn text_scale(settings: &Settings) -> f32 {
    std::env::var("AURORA_TEXT_SIZE")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|s| s.is_finite())
        .map_or(settings.font_scale, |s| {
            s.clamp(crate::settings::FONT_SCALE_MIN, crate::settings::FONT_SCALE_MAX)
        })
}

/// The colour vision the interface adapts to: the preferences', or in demo
/// mode `AURORA_DEMO_COLORBLIND` (a capture's, never saved).
fn wanted_vision(settings: &Settings) -> crate::colorblind::Vision {
    if std::env::var_os("AURORA_DEMO").is_some()
        && let Some(v) = crate::colorblind::demo_override()
    {
        return v;
    }
    crate::colorblind::Vision::new(settings.color_blindness, settings.color_blindness_strength)
}

/// Whether the interface takes the high contrast palette: the
/// preferences', or in demo mode `AURORA_DEMO_CONTRAST` (a capture's, never
/// saved).
fn wanted_high_contrast(settings: &Settings) -> bool {
    if std::env::var_os("AURORA_DEMO").is_some()
        && let Some(on) = crate::high_contrast::demo_override()
    {
        return on;
    }
    settings.high_contrast
}

/// The interface colours: the preferences', or in demo mode
/// `AURORA_DEMO_COLORS` (a capture's, never saved).
fn wanted_interface_colors(settings: &Settings) -> crate::interface_colors::InterfaceColors {
    if std::env::var_os("AURORA_DEMO").is_some()
        && let Some(colors) = crate::interface_colors::demo_override()
    {
        return colors;
    }
    settings.interface_colors.clone()
}

/// Test overrides of the settings (used by automated captures): the
/// graphics level first, so that the single settings after it still change
/// it. Applied at start, and again after a new profile's level is chosen.
fn apply_test_overrides(settings: &mut Settings) {
    if std::env::var_os("AURORA_DEMO").is_some() && ui::bars::demo::get().graphics_profiles {
        crate::graphics_presets::saved::demo(settings);
    }
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
    if let Some(v) = std::env::var("AURORA_RENDER_SCALE").ok().and_then(|v| v.parse::<u32>().ok()) {
        settings.render_scale_percent = v.clamp(50, 100);
    }
    if let Some(v) = std::env::var("AURORA_TEXTURE_BUDGET_MB").ok().and_then(|v| v.parse::<u32>().ok()) {
        settings.texture_budget_auto = false;
        settings.texture_budget_mb = v.clamp(256, 32768);
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
    if let Ok(v) = std::env::var("AURORA_HDR") {
        settings.hdr_emissive = v != "0";
    }
    if let Ok(v) = std::env::var("AURORA_OCCLUSION") {
        settings.occlusion_culling = v != "0";
    }
}

impl App {
    // Growing editor state must not accumulate large return-value temporaries
    // on the Windows main stack while the avatar library is being loaded.
    pub fn new() -> anyhow::Result<Box<App>> {
        // found on a background thread while the rest starts
        ui::fonts::fallback::start();
        // the regional 12 / 24 hours and time zone of the bars' clock, read once
        ui::bars::clock::read_system_clock();
        let mut settings = Settings::load();
        if std::env::var("AURORA_DEMO").as_deref() == Ok("1")
            && let Ok(mode) = std::env::var("AURORA_DEMO_MINIMIZED_MUTE")
        {
            settings.audio.mute_when_minimized = mode != "off";
        }
        aurora_i18n::select(&settings.language, crate::cli::get().language.as_deref());
        if std::env::var_os("AURORA_DEMO").is_some() && std::env::var_os("AURORA_DEMO_KEYBOARD").is_some() {
            // Exercise a first launch without changing the user's bindings.
            settings.keybinds = Default::default();
        }
        apply_test_overrides(&mut settings);
        external_links::apply_grid(&mut settings);
        if std::env::var("AURORA_DEMO").as_deref() == Ok("1")
            && std::env::var_os("AURORA_DEMO_TEXTURE_LIBRARY").is_some()
            && std::env::var_os("AURORA_DEMO_AO").is_none()
        {
            settings.show_ao = false;
        }
        // (an adapter known to drop under many downloads keeps them
        // limited: #1545, net_adapters.rs)
        let net = NetClient::new(crate::net_adapters::startup_mode(settings.download_connections))?;
        net.set_agent_language(aurora_i18n::language(), settings.language_is_public);
        let voice_http = net.caps_http();
        let login_info = ui::news::LoginInfo::start(net.runtime());
        let updater = crate::updater::Updater::new(net.runtime(), settings.update_channel);
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
        let geometry_needs = crate::scene::geometry_need::GeometryNeeds::load(&cache);
        let egui_ctx = egui::Context::default();
        // Restore window state before the theme and fonts: loading egui's
        // memory drops pending font definitions and the transient text scale.
        if std::env::var_os("AURORA_CAPTURE").is_none() || std::env::var_os("AURORA_CAPTURE_LAYOUT").is_some() {
            ui::layout::load(&egui_ctx);
        }
        ui::colors::set(&settings.colors);
        crate::keywords::set(&settings.keywords);
        crate::spelling::configure(&egui_ctx, &settings.spelling);
        crate::autoreplace::set_enabled(settings.auto_replace);
        crate::autoreplace::load();
        crate::link_trust::configure(&settings.trusted_sites);
        crate::link_preview::configure_sites(&settings.link_previews.picture_sites, crate::demo::picture_sites());
        // before the skin's palette is built and applied
        crate::colorblind::set(wanted_vision(&settings));
        crate::high_contrast::set(wanted_high_contrast(&settings));
        crate::interface_colors::set(wanted_interface_colors(&settings));
        let skin = Skin::load(&egui_ctx, "aurora", text_scale(&settings));
        crate::crash::update(&skin.theme, &settings);
        let palette = skin.theme.palette();
        egui_ctx.set_zoom_factor(ui_scale(&settings));
        // last view of the previous session for the loading screen
        let mut backdrop = ui::backdrop::Backdrop::default();
        if settings.loading_backdrop && std::env::var_os("AURORA_CAPTURE").is_none() {
            backdrop.load_saved();
        }
        let system_fonts_pending = !ui::fonts::apply(&egui_ctx, &ui_font(&settings));

        // Decode the logo off the main thread.
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || match image::load_from_memory(LOGO_PNG) {
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
        });

        let panels = Panels::from_settings(&settings);
        let chat_ui = ChatUi::from_settings(&settings);
        let mut scene = Scene::new(cache, lib.clone());
        scene.jobs.execute(|| {
            crate::movement::strokes::pack();
        });
        scene.draw_distance = settings.draw_distance;
        scene.lod_factor = settings.lod_factor;
        let update_demo = ui::update::UpdateDemo::from_env(&settings);
        Ok(Box::new(App {
            area_search_ui: ui::area_search::AreaSearchUi::from_saved(&settings.area_search),
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
            bridge_runtime: Default::default(),
            native_anchor: Default::default(),
            anchor_stands: Default::default(),
            anchor_pose: None,
            bridge_services: Default::default(),
            bridge_demo: Default::default(),
            movelock_ui: Default::default(),
            movement_ui: Default::default(),
            movement_runtime: Default::default(),
            movement_probe: (None, false, None),
            pass_wall_at: None,
            scene,
            probes: Default::default(),
            sky_log: Default::default(),
            camera: Camera::default(),
            input: MoveInput::default(),
            joystick: Default::default(),
            down: HashSet::new(),
            shift: false,
            ctrl: false,
            command: false,
            screen: Screen::Login,
            login_form: LoginForm::default(),
            // Bug report form (disabled, see main.rs):
            // bug_report: Default::default(),
            login_progress: None,
            demo_mfa_challenge: None,
            relogin: Default::default(),
            viewer_stats: Default::default(),
            startup_windows: ui::StartupWindows::new(&panels),
            panels,
            chat_ui,
            toolbar_ui: Default::default(),
            dock_layout: Default::default(),
            favorites_access_applied: None,
            favorite_images: Default::default(),
            history_ui: Default::default(),
            perf: PerfData::default(),
            monitor: ui::monitor::Monitor::default(),
            monitor_feed: monitor::MonitorFeed::default(),
            interface: Default::default(),
            frame_profile: FrameProfile::default(),
            net_profile: crate::net_profile::NetProfile::default(),
            deep_profile: crate::deep_profile::DeepProfile::default(),
            sync_alt: 0,
            name_tags_alt_classic: false,
            demo_tag_states: None,
            demo_bubble_chat: 0.0,
            demo_people_states: false,
            ui_parts: Parts::default(),
            egui_hand_over: EguiHandOver::default(),
            last_frame: Instant::now(),
            prev_dt: 0.0,
            start: Instant::now(),
            right_drag: false,
            sent_fov: crate::camera::DEFAULT_FOV,
            cursor_hold: Default::default(),
            alt_cursor: Default::default(),
            numeric_drag: Default::default(),
            text_menu: Default::default(),
            alt: false,
            alt_graph: false,
            left_down: false,
            world_mouse_walk_pending: false,
            clicks: Default::default(),
            script_clicks: Default::default(),
            right_gesture: RightGesture::default(),
            context_menu: None,
            context_customization: Default::default(),
            avatar_textures: Default::default(),
            object_profile: Default::default(),
            pathfinding_ui: Default::default(),
            mouse_mode: MouseMode::None,
            build: Default::default(),
            ik_editor: Default::default(),
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
            autopilot_stop_pending: false,
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
            static_eyes: Default::default(),
            look_hover: None,
            hover_tips: Default::default(),
            look_at_fades: Default::default(),
            demo_look_at: false,
            demo_look_ahead: false,
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
            layout_autosave: Default::default(),
            loading_stage: String::new(),
            move_complete_at: None,
            tp_checks: Vec::new(),
            frozen_cull: None,
            texture_motion: Default::default(),
            sound_cues: Default::default(),
            mic_was: false,
            context_menu_was: false,
            delete_confirm: None,
            moderation_ask: None,
            object_menu: Default::default(),
            place_confirm: None,
            url_confirm: None,
            media_filter_prompt: Default::default(),
            script_dialogs_were: 0,
            beacon_sound_at: None,
            wind_sound: Default::default(),
            stuck_watch: None,
            stuck_logged: None,
            attachments_logged: 0,
            describe: None,
            dev_toolbox: Default::default(),
            toolbox: Default::default(),
            sim_console_input: Default::default(),
            loading_frac: 0.0,
            loading_target: 0.0,
            frame_count: 0,
            demo_rlva_frame: None,
            rlva_login: if std::env::var("AURORA_DEMO").as_deref() == Ok("1") {
                Default::default()
            } else {
                crate::world::rlva::login::State::load(crate::settings::config_dir().join("rlva-login.json"))
            },
            named_teleport: Default::default(),
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
            demo_tp_region: None,
            demo_friend_join_target: None,
            demo_friend_join_flight: None,
            demo_radar_tp: false,
            demo_radar_walk: false,
            tp_dest: None,
            geom_changed: None,
            frame_start: Instant::now(),
            alt_codes: Default::default(),
            alt_text: Vec::new(),
            people_ui: Default::default(),
            contacts_people_ui: Default::default(),
            radar: Default::default(),
            radar_clock: Instant::now(),
            contacts_ui: Default::default(),
            minimap_ui: Default::default(),
            display_name_ui: Default::default(),
            create_landmark_ui: Default::default(),
            landmark_folders_checked: u64::MAX,
            hover_height_ui: Default::default(),
            follow_ui: Default::default(),
            portal_ui: Default::default(),
            particle_editor: Default::default(),
            particle_inject: Default::default(),
            portal_drop: None,
            portal_journey: None,
            portal_demo_peer: None,
            follow_recovery: None,
            report_abuse: Default::default(),
            dof_focus: Default::default(),
            snapshots: Default::default(),
            inspect_ui: Default::default(),
            sound_explorer_ui: Default::default(),
            animation_explorer_ui: Default::default(),
            animation_explorer: Default::default(),
            avatar_render_settings_ui: Default::default(),
            area_search_feed: Default::default(),
            blacklist_ui: Default::default(),
            bumps_ui: Default::default(),
            grid_status: Default::default(),
            grid_status_listed: false,
            grid_status_was_open: false,
            about: Default::default(),
            stream_titles: Default::default(),
            stream_title_ui: Default::default(),
            buy_currency_ui: Default::default(),
            pay_ui: Default::default(),
            money_tracker_ui: Default::default(),
            buy_land_ui: Default::default(),
            buy_land_source: Default::default(),
            marketplace_ui: Default::default(),
            listings_ui: Default::default(),
            demo_slm: None,
            people_map_ui: Default::default(),
            worldmap_ui: Default::default(),
            map_tiles: ui::map_tiles::MapTiles::new(),
            options_ui: Default::default(),
            grids_ui: Default::default(),
            audio_ui: Default::default(),
            notif_ui: Default::default(),
            friend_prompt: Default::default(),
            region_message_prompt: Default::default(),
            region_restart: Default::default(),
            attention: Default::default(),
            reconnect_place: None,
            external: external_links::ExternalLinks::start(),
            demo_link_health: link_health::LinkHealthDemo::from_env(),
            demo_lookups: Vec::new(),
            remote_object: None,
            app_link_parcels: HashSet::new(),
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
            stream_title_said: None,
            minimized_mute: audio_mute::MinimizedMute::default(),
            crash_state_at: None,
            gpu_stall: Default::default(),
            crouch_toggle: Default::default(),
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
            name_tag_visibility: Default::default(),
            beacon_hit: None,
            plate_hover: None,
            sound_blocks: Default::default(),
            sound_changes: Vec::new(),
            devices_listed: false,
            emoji: ui::emoji::Emoji::load(),
            system_fonts_pending,
            avatar_pics: Default::default(),
            avatar_pic_sources: Default::default(),
            avatar_pic_images: Default::default(),
            login_portrait_source: None,
            ui_coarse: Default::default(),
            ui_images: Default::default(),
            inventory_hover_images: Default::default(),
            ui_image_masks: Default::default(),
            rlva_overlay_slots: Default::default(),
            profile_ui: Default::default(),
            place_ui: Default::default(),
            places_ui: Default::default(),
            search_ui: Default::default(),
            place_images: Default::default(),
            demo_place: None,
            stream_demo: None,
            rez_demo: None,
            arrival_demo: None,
            outfit_restore_demo: None,
            tex_refresh_demo: None,
            sculpt_missing_demo: None,
            texture_missing_demo: None,
            experience_demo: None,
            away_coarse: Vec::new(),
            geometry_demo: None,
            geometry_needs,
            turn_demo: None,
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
            texture_library: Default::default(),
            library_demo_seeded: false,
            library_drag: Default::default(),
            gesture_state: Default::default(),
            imports: Default::default(),
            blender_live: Default::default(),
            task_transfer: Default::default(),
            contents_copies: Default::default(),
            world_edits: Default::default(),
            demo_offer_shown: None,
            ao_ui: Default::default(),
            ao_catalog: Default::default(),
            ao_bootstrap: Default::default(),
            ao_runtime: Default::default(),
            ao_agent: uuid::Uuid::nil(),
            ao_missing: Default::default(),
            ao_demo: std::env::var("AURORA_DEMO_AO").ok(),
            ao_default_applied: false,
            demo_link_cfg: crate::demo::link::Config::from_env(),
            demo_link: None,
            own_motion_logged: None,
            demo_seat: None,
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
            physics_ui: Default::default(),
            wearable_ui: Default::default(),
            avatar_health: Default::default(),
            last_social_poll: Instant::now(),
            last_input: Instant::now(),
            auto_away: false,
            neighbours_refused: false,
            folders: Default::default(),
            settings_backup: Default::default(),
            stream_list: Default::default(),
            capture: std::env::var_os("AURORA_CAPTURE").map(|p| {
                let frames = std::env::var("AURORA_CAPTURE_FRAMES").unwrap_or_default();
                (
                    capture_files(std::path::Path::new(&p), &frames),
                    std::env::var_os("AURORA_CAPTURE_EXIT").is_some(),
                )
            }),
        }))
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
        self.send_from(cmd, uuid::Uuid::nil());
    }

    /// What a COF sync may take off: the worn objects but the Aurora
    /// bridge, which the Legacy Viewer puts back into every replaced outfit
    /// (FSLSLBridge in LLAppearanceMgr::updateCOF, llappearancemgr.cpp).
    pub(crate) fn outfit_sync_worn(&self) -> std::collections::HashSet<uuid::Uuid> {
        let mut worn = self.world.worn_attachment_items();
        worn.remove(&self.bridge_runtime.item());
        worn
    }

    fn send_from(&mut self, mut cmd: NetCommand, actor: uuid::Uuid) {
        self.settings.inventory.creation_permissions.prepare(&mut cmd);
        if !self.object_profile.retain_selection(&self.world, &mut cmd) {
            return;
        }
        if self.blender_live.local_command(&cmd, &self.world) {
            return;
        }
        self.world.rlva_configure(self.settings.rlva_enabled);
        self.world.rlva.preferences.clone_from(&self.settings.rlva);
        if !self.settings.rlva_enabled {
            self.named_teleport.cancel_scripts();
            self.world.maturity.cancel_script_teleports();
            if !actor.is_nil() {
                return;
            }
        }
        self.world.rlva_prepare_outfit(&mut cmd, actor);
        if !actor.is_nil() && !self.world.rlva_normalize_teleport(&mut cmd) {
            return;
        }
        if let NetCommand::OfferTeleport { to, .. } = &mut cmd {
            self.world.rlva_filter_teleport_offer(to);
        }
        let im_placeholder = actor.is_nil() && self.world.rlva_prepare_outgoing_im(&mut cmd);
        if !im_placeholder && !self.world.rlva_allows_command(&cmd, actor) {
            self.world
                .system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
            return;
        }
        if let NetCommand::GroupTitleUpdate { group, role } = &cmd {
            self.world.group_data.title_sent(*group, *role);
        }
        if let NetCommand::Build(aurora_net::build::BuildCmd::Transform { handle, updates }) = &cmd {
            self.world.objects.note_sent_transforms(*handle, updates, Instant::now());
        }
        self.world.rlva_note_attachment_command(&cmd, actor, Instant::now());
        self.world.note_outfit_command(&cmd);
        self.world.rlva.note_sit_command(&cmd, actor);
        if self.world.rlva.enabled()
            && let NetCommand::RezAttachments(items) = &mut cmd
        {
            for item in items {
                item.add = true;
            }
        }
        let cmd = if let NetCommand::Chat {
            message,
            channel,
            chat_type,
        } = cmd
        {
            let mut outgoing = self.world.rlva.outgoing_chat(&message, channel, chat_type).into_iter();
            let Some(first) = outgoing.next() else {
                return;
            };
            for command in outgoing {
                self.net.send(command);
            }
            first
        } else {
            cmd
        };
        if let Some(flags) = self.native_anchor.observe(&cmd, self.world.agent.seated, Instant::now()) {
            let release = self.native_anchor.command(flags);
            self.send(release);
        }
        if self.bridge_runtime.observe_command(&cmd, Instant::now()) {
            let prefs = self.settings.bridge.entry(self.world.agent_id).or_default();
            if prefs.mode == crate::world::bridge::LockMode::Bridge {
                prefs.enabled = false;
                if !self.demo {
                    self.settings.save();
                }
            }
        }
        if self.demo && self.bridge_demo_command(&cmd) {
            return;
        }
        // LLAgent::startTeleportRequest: a teleport waits while the maturity
        // preference is not in sync with the server (TELEPORT_PENDING)
        self.named_teleport.observe(&cmd, actor);
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
            self.world.maturity.hold(cmd, actor);
            return;
        }
        // AURORA_DEMO_EXPERIENCE: the simulator and the capabilities
        if self.demo
            && let Some(reply) = self.experience_demo.as_mut().and_then(|demo| demo.reply(&cmd))
        {
            self.apply_experience_demo(reply);
            return;
        }
        // AURORA_DEMO_HOME / AURORA_DEMO_USERINFO: the region's answers
        if self.demo
            && let NetCommand::SetHomeLocation { position, .. } = cmd
        {
            for ev in crate::demo::home::reply(position) {
                self.world.apply(ev);
            }
            return;
        }
        if self.demo
            && let Some(events) = crate::demo::home::user_info_reply(&cmd)
        {
            for ev in events {
                self.world.apply(ev);
            }
            return;
        }
        // AURORA_DEMO_SIM_CONSOLE / AURORA_DEMO_VOICE_MODERATION: the
        // region's answers
        if self.demo
            && let NetCommand::SimConsole(command) = &cmd
        {
            self.world.apply(crate::demo::sim_extras::console_reply(command));
            return;
        }
        if self.demo
            && let NetCommand::ModerateVoice { target, mute } = cmd
        {
            let peers: Vec<uuid::Uuid> = self.world.calls.voice_peers.iter().copied().collect();
            crate::demo::sim_extras::moderate(target, mute, &peers);
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
            if let Some(demo) = &mut self.outfit_restore_demo
                && demo.handle(&cmd, &mut self.world, Instant::now())
            {
                return;
            }
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
                        self.demo_unseat_on_teleport();
                        self.world.apply(NetEvent::TeleportLocal {
                            flying: self.world.agent.flying
                                || (crate::demo::friend_join_flight_scenario()
                                    && !crate::demo::friend_join_late_flight_scenario()
                                    && !crate::demo::friend_join_custom_flight_scenario()),
                        });
                        if let Some(ev) = self.world.apply(NetEvent::AgentMovementComplete {
                            handle: *handle,
                            position: *position,
                            look_at: *look_at,
                        }) {
                            self.on_app_event(ev);
                        }
                    } else {
                        aurora_net::transition::begin(
                            aurora_net::transition::Kind::Teleport,
                            || "demo teleport started (offline: no simulator)".to_string(),
                            &self.net.stats,
                        );
                        self.demo_tp_start = Some(Instant::now());
                        self.demo_tp_dest = Some(*position);
                        self.demo_tp_region = crate::demo::friend_join_scenario().then_some(*handle);
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
            // the simulator's MoneyBalanceReply names a gift (the money
            // tracker, the card in the payee's conversation)
            if let NetCommand::PayResident {
                to,
                group,
                amount,
                message,
            } = &cmd
            {
                let item = if message.is_empty() { "Payment" } else { message.as_str() };
                let reply = NetEvent::MoneyTransaction {
                    id: uuid::Uuid::new_v4(),
                    success: true,
                    kind: 5001,
                    source: self.world.agent_id,
                    source_group: false,
                    dest: *to,
                    dest_group: *group,
                    amount: *amount,
                    item: item.to_owned(),
                    description: String::new(),
                };
                self.record_money(&reply);
                self.world.payment(&reply);
            }
            // a sit on any prim, and the stand up from it (app/own_motion.rs);
            // the stand up's animations still come from demo::respond
            let seated_on_demo_prim = self.demo_seat.is_some();
            if self.demo_sit_command(&cmd) && matches!(cmd, NetCommand::RequestSit { .. }) {
                return;
            }
            if matches!(&cmd, NetCommand::OneShotControl(f) if f & control::STAND_UP != 0)
                && !seated_on_demo_prim
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
            if crate::world::received_items::demo_accept(&mut self.world, &cmd) {
                return;
            }
            if let Some(ev) = crate::demo::task_offer_delivery(&cmd, self.world.agent_id) {
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
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
            // the server's copy: the default folder of its type for a nil parent
            if let NetCommand::CopyInventoryItem { request, copy } = &cmd {
                let inv = &self.world.inventory;
                let result = inv
                    .items
                    .get(&copy.item)
                    .cloned()
                    .map(|mut it| {
                        it.id = uuid::Uuid::new_v4();
                        it.owner = self.world.agent_id;
                        if !copy.name.is_empty() {
                            it.name.clone_from(&copy.name);
                        }
                        it.parent = if copy.parent.is_nil() {
                            crate::world::inventory::actions::system(inv, it.asset_type).unwrap_or(inv.root)
                        } else {
                            copy.parent
                        };
                        it
                    })
                    .ok_or_else(|| aurora_i18n::tr!("inventory-copy-unconfirmed").into());
                if let Ok(it) = &result {
                    self.world.inventory.add_items(vec![it.clone()]);
                }
                self.on_app_event(NetEvent::InventoryItemCopied { request: *request, result });
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
            if let NetCommand::CreateInventoryWearable {
                parent,
                kind,
                name,
                permissions,
                ..
            } = &cmd
            {
                self.send(NetCommand::CreateInventoryItem {
                    parent: *parent,
                    kind: aurora_net::inventory::operations::NewItem::Wearable(*kind),
                    name: name.clone(),
                    permissions: *permissions,
                });
                return;
            }
            if let NetCommand::CreateNotecardItem {
                request,
                parent,
                name,
                data,
                permissions,
            } = &cmd
            {
                let item = aurora_net::inventory::InvItem {
                    id: uuid::Uuid::new_v4(),
                    parent: *parent,
                    name: name.clone(),
                    asset_type: 7,
                    inv_type: 7,
                    asset_id: uuid::Uuid::new_v4(),
                    owner: self.world.agent_id,
                    creator: self.world.agent_id,
                    base_mask: 0x7fff_ffff,
                    owner_mask: 0x7fff_ffff,
                    next_owner_mask: permissions.unwrap_or_default().next_owner,
                    group_mask: permissions.unwrap_or_default().group,
                    everyone_mask: permissions.unwrap_or_default().everyone,
                    ..Default::default()
                };
                self.demo_documents.insert(item.id, data.clone());
                self.world.inventory.add_items(vec![item.clone()]);
                self.on_app_event(NetEvent::InventoryContentSaved {
                    request: *request,
                    item: item.id,
                    result: Ok(item),
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
            if let NetCommand::CreateInventoryItem {
                parent,
                kind,
                name,
                permissions,
            } = &cmd
            {
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
                    next_owner_mask: permissions.map_or(0x7fff_ffff, |p| p.next_owner),
                    group_mask: permissions.map_or(0, |p| p.group),
                    everyone_mask: permissions.map_or(0, |p| p.everyone),
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
            // the demo backpack comes back with its prims; other outfit
            // objects are invisible placeholders
            if let NetCommand::RezAttachments(items) = &cmd {
                self.world.apply(NetEvent::ObjectUpdates {
                    handle: self.world.main_region.unwrap_or_default(),
                    objects: items
                        .iter()
                        .flat_map(|item| crate::demo::worn::rez(item).unwrap_or_else(|| vec![crate::demo::outfit_attachment(item)]))
                        .collect(),
                });
                return;
            }
            // DetachAttachmentIntoInv names the item: the simulator kills the
            // object worn from it, whatever its object id
            if let NetCommand::DetachAttachments(items) = &cmd {
                let local_ids = items
                    .iter()
                    .filter_map(|id| self.world.worn_attachment(id))
                    .filter_map(|idx| self.world.objects.get(idx))
                    .map(|o| o.key.local_id)
                    .collect();
                self.world.apply(NetEvent::ObjectsKilled {
                    handle: self.world.main_region.unwrap_or_default(),
                    local_ids,
                });
                return;
            }
            // ObjectDetach of a temporary attachment: there is no item to
            // keep, the simulator kills the object
            if let NetCommand::DetachObject { handle, local_id } = &cmd {
                self.world.apply(NetEvent::ObjectsKilled {
                    handle: *handle,
                    local_ids: vec![*local_id],
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
                crate::ao::demo::preserve_custom_motions(&self.world, &cmd, &mut ev);
                if matches!(cmd, NetCommand::ObjectRelease { .. }) {
                    crate::demo::hud::preserve_touch_transform(&self.world, &mut ev);
                }
                if let NetEvent::InventoryContents(contents) | NetEvent::InventoryTreeFetched { result: Ok(contents), .. } = &mut ev {
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

    /// Nearby chat in bubbles above the heads (#673); AURORA_DEMO_CHAT_BUBBLES
    /// turns them on for the run, without saving it.
    fn chat_bubbles(&self) -> bool {
        self.settings.chat_bubbles || (self.demo && crate::demo::tags::chat_bubbles().is_some())
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

    /// Where the script errors go follows the preferences (#1263), before
    /// each frame's events and the demo's arrival lines.
    fn sync_script_debug(&mut self) {
        let debug = &mut self.world.script_debug;
        debug.owner_say = self.settings.script_editor.owner_say_debug;
        debug.in_chat = self.settings.script_errors_in_chat;
        // (never DEBUG_CHANNEL itself, the errors' own channel)
        debug.channel = (self.settings.script_errors_to_channel && self.settings.script_errors_channel != i32::MAX)
            .then_some(self.settings.script_errors_channel);
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
        // the chosen folder (InstantMessageLogPath); once logged in it only
        // changes with its files (`relocate`)
        if log.identity().is_none() {
            log.set_folder(chat_log_root(&s.chat_log_location));
        }
    }

    // ------------------------------------------------------------------ login

    fn start_login(&mut self) {
        // a previous session of this account may still be on the grid
        if self.hold_login() {
            return;
        }
        if self.demo && self.demo_refusal() {
            return;
        }
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
            if let Some((_, _, place)) = self.reconnect_place.take() {
                log::info!("demo login: reconnect to {}", place.to_login_string());
            }
            if let Some(l) = &self.login_form.start_slurl {
                log::info!("demo login: start at {}", crate::external_url::place_label(l));
            }
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
        // "Reconnect" after a lost link: the same place, for the same account
        let reconnect = self
            .reconnect_place
            .take()
            .filter(|(uri, user, _)| *uri == self.settings.login_uri() && *user == self.settings.username.trim().to_lowercase())
            .map(|(_, _, place)| place);
        // then a SLURL given from outside (LLStartUp::getStartSLURL)
        let slurl = if let Some(l) = self.login_form.start_slurl.as_ref() {
            let name = match crate::grids::resolve(&self.settings, &l.target()) {
                Ok(name) => name,
                Err(error) => {
                    self.login_form.error = Some(error.text().to_owned());
                    return;
                }
            };
            Some(StartLocation::Region {
                name,
                x: l.pos.x.round() as u32,
                y: l.pos.y.round() as u32,
                z: l.pos.z.round() as u32,
            })
        } else {
            None
        };
        let start = reconnect.or(slurl).unwrap_or_else(|| match self.settings.start_location.as_str() {
            "home" => StartLocation::Home,
            "region" if !self.settings.start_region.trim().is_empty() => StartLocation::Region {
                name: self.settings.start_region.trim().to_owned(),
                x: 128,
                y: 128,
                z: 30,
            },
            _ => StartLocation::Last,
        });
        let catalog_key = (self.settings.login_uri(), self.settings.username.trim().to_lowercase());
        let login_agent = self
            .login_form
            .accounts
            .iter()
            .find(|account| account.key() == catalog_key)
            .and_then(|account| account.agent);
        let start = if self.rlva_login.requires_last(self.settings.rlva_enabled, login_agent) {
            StartLocation::Last
        } else {
            start
        };
        let crate::machine_id::LoginIds { mac, id0 } = crate::machine_id::login_ids();
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
            last_exec: self.last_exec(),
        };
        self.login_form.busy = true;
        self.login_form.error = None;
        self.login_progress = Some((aurora_i18n::tr!("app-connecting").into(), 0.05));
        self.settings.save();
        self.net.send(NetCommand::Login(Box::new(req)));
    }

    fn back_to_login(&mut self, error: Option<String>) {
        // Capture the layout before replacing account-specific UI state.
        self.chat_ui.remember_layout(&mut self.settings);
        self.settings.show_camera_controls = self.panels.camera_controls;
        self.settings.show_movement_controls = self.panels.movement_controls;
        self.settings.save();
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
        if !self.demo {
            self.settings.release_hover_account();
        }
        self.hover_height_ui = Default::default();
        self.follow_ui = Default::default();
        self.portal_ui = Default::default();
        self.particle_editor = Default::default();
        self.particle_inject = Default::default();
        self.portal_drop = None;
        self.portal_journey = None;
        self.portal_demo_peer = None;
        self.follow_recovery = None;
        self.inventory_ui = Default::default();
        self.contents_copies = Default::default();
        self.texture_library.reset();
        self.library_drag = Default::default();
        self.library_demo_seeded = false;
        self.gesture_state = Default::default();
        self.buy_currency_ui = Default::default();
        self.money_tracker_ui = Default::default();
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
        self.scene.sounds.history = Default::default();
        self.sound_explorer_ui = Default::default();
        self.world.animation_log.clear();
        self.animation_explorer_ui = Default::default();
        self.animation_explorer = Default::default();
        self.avatar_render_settings_ui = Default::default();
        self.blender_live.stop();
        self.clear_blender_preview();
        self.blender_live = Default::default();
        if let Some(g) = &mut self.gfx {
            self.scene.clear(&mut g.renderer);
        }
        self.world.reset();
        // the next session's network starts with the neighbours allowed
        self.neighbours_refused = false;
        // the transcripts shown belong to the account that leaves
        self.history_ui = Default::default();
        self.chat_ui = ChatUi::from_settings(&self.settings);
        self.ao_agent = uuid::Uuid::nil();
        self.appearance_ui = Default::default();
        self.shape_ui = Default::default();
        self.shape_camera = None;
        self.physics_ui = Default::default();
        self.wearable_ui = Default::default();
        self.reset_avatar_health();
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
        self.static_eyes.on_arrival();
        self.set_mouselook_grab(false);
        self.camera = Camera::default();
    }

    fn on_app_event(&mut self, ev: NetEvent) {
        if matches!(&ev, NetEvent::AgentMovementComplete { .. })
            && let Some(recovery) = &mut self.follow_recovery
        {
            recovery.arrived = true;
        }
        if self.world.rlva_offer_event(&ev) || self.world.rlva_rename_event(&ev, Instant::now()) {
            if let NetEvent::InventoryEdited { request, .. } = ev {
                self.world_edits.remove(&request);
            }
            return;
        }
        if let NetEvent::TeleportRegionResolved { request, result } = ev {
            if !self.settings.rlva_enabled {
                self.named_teleport.cancel_scripts();
            }
            if let Some((actor, result)) = self.named_teleport.resolve(request, result) {
                match result {
                    Ok(command) => self.send_deferred_teleport(command, actor),
                    Err(reason) => {
                        self.world.apply(NetEvent::TeleportFailed { reason });
                    }
                }
            }
            return;
        }
        if self.world.received_items_event(&ev) {
            return;
        }
        if let NetEvent::BridgeQueryReply { request, result } = ev {
            let _ = self.sync_bridge_queries();
            if self.bridge_services.owns(request) || self.world.portals.owns(request) {
                let error = result.as_ref().err().map(String::as_str);
                self.bridge_runtime.query_reply(&self.world, error, Instant::now());
            }
            self.world.portals.reply(request, &result, Instant::now());
            if let Some(message) = self.bridge_services.reply(request, result, Instant::now()) {
                self.world.system_message(message);
            }
            return;
        }
        if self.bridge_runtime.event(&mut self.world, &ev, Instant::now()) {
            return;
        }
        if let NetEvent::MoneyTransaction {
            success,
            kind,
            dest,
            ref description,
            ..
        } = ev
        {
            self.record_money(&ev);
            self.world.payment(&ev);
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
        if let NetEvent::Pathfinding {
            request,
            region,
            kind,
            result,
        } = ev
        {
            self.pathfinding_ui.reply(request, region, kind, result);
            return;
        }
        if let NetEvent::ServerReleaseNotes(page) = ev {
            // LLFloaterAbout::handleServerReleaseNotes, then LLWeb::loadURL
            match page {
                Ok(url) => self.egui_ctx.open_url(egui::OpenUrl::new_tab(url)),
                Err(_) => {
                    self.world.notifications.push(
                        crate::world::notifications::Kind::Alert,
                        aurora_i18n::tr!("help-server-release-notes"),
                        aurora_i18n::tr!("help-server-release-notes-failed").to_owned(),
                        crate::world::notifications::Data::None,
                    );
                }
            }
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
            let material_map = self.imports.uploading_entry(request);
            if let Some(item) = self.imports.uploaded(request, result) {
                if let Some(entry) = material_map {
                    self.texture_library.map_uploaded(entry, &item);
                }
                let id = item.id;
                self.world.inventory.add_items(vec![item]);
                self.inventory_ui.show_original(&self.world.inventory, id);
            }
            return;
        }
        if let NetEvent::InventoryCreated(item) = ev {
            if self.physics_created(&item) {
                return;
            }
            let Some(item) = self.particle_script_created(item) else {
                return;
            };
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
            if self.ik_editor.project_saving(request) {
                if let Ok(saved) = &result {
                    self.world.inventory.add_items(vec![saved.clone()]);
                }
                self.ik_editor.project_saved(&result);
                return;
            }
            if ui::inventory::context_notecards::saved(&mut self.inventory_ui, request, &result) {
                if let Ok(item) = result {
                    self.world.inventory.add_items(vec![item]);
                }
                return;
            }
            if self.context_customization.notecards.saved(request, &result) {
                if let Ok(item) = result {
                    self.world.inventory.add_items(vec![item]);
                }
                return;
            }
            if crate::build::ui::layout_notecard_saved(&mut self.build, request, &result) {
                if let Ok(item) = result {
                    self.world.inventory.add_items(vec![item]);
                }
                return;
            }
            if self.texture_library.material_save_pending(request) {
                self.resource_material_saved(request, result);
                return;
            }
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
        if let NetEvent::InventoryItemCopied { request, result } = ev {
            if self.texture_library.material_save_pending(request) {
                self.resource_material_copied(request, result);
                return;
            }
            self.wear_copied(request, result);
            return;
        }
        if let NetEvent::InventoryOperationFailed(reason) = ev {
            self.gesture_state.new_editor = false;
            if self.physics_ui.creation_failed() {
                self.appearance_ui.message.clone_from(&reason);
            }
            self.inventory_ui.message = reason;
            return;
        }
        if let NetEvent::InventoryProgress { request, progress } = ev {
            self.inventory_ui.update_mutation_progress(request, progress);
            return;
        }
        if let NetEvent::InventoryEdited { request, result } = ev {
            if self.world.attachment_inventory.owns(request) {
                self.world.attachment_inventory_result(request, result);
                if self.appearance_ui.pending.is_some_and(|(id, _)| id == request) {
                    self.appearance_ui.pending = None;
                }
                return;
            }
            if self.imports.folders.owns(request) {
                self.imports.import_folder_result(request, result, &mut self.world.inventory);
                return;
            }
            if self.texture_library_result(request, &result) {
                return;
            }
            if self.particle_inject.owns(request) {
                self.particle_folder_created(request, result);
                return;
            }
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
            if self.contents_copies.owns(request) {
                self.contents_copy_result(request, result);
                return;
            }
            if self.world_edits.remove(&request) {
                match result {
                    Ok(reply) => {
                        self.world.inventory.remove(&reply.removed);
                        self.world.inventory.apply(reply.contents);
                        self.world.inventory.created_folders(reply.folders);
                    }
                    Err(reason) => log::warn!("background inventory change not saved: {reason}"),
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
            if self.ik_editor.project_preview(&self.world, item, &result) {
                return;
            }
            if ui::inventory::context_notecards::loaded(&mut self.inventory_ui, &mut self.settings.inventory, item, &result) {
                self.settings.save();
            }
            if let Some(menus) = self.context_customization.notecards.loaded(item, &result) {
                self.context_customization
                    .close(&self.egui_ctx, &mut self.settings.world_context_menus);
                if self.context_menu.take().is_some() {
                    self.build.release_menu_selection(&self.world);
                    self.flush_build();
                }
                self.settings.world_context_menus = menus;
                self.settings.save();
            }
            if crate::build::ui::layout_notecard_loaded(&mut self.build, &mut self.world, &mut self.settings.build, item, &result) {
                self.settings.save();
            }
            let blender_shape = self.blender_shape_loaded(item, asset, &result);
            if self.wearable_ui.loaded(&mut self.world, item, asset, &result) {
                return;
            }
            if self.physics_ui.loaded(&mut self.world, item, asset, &result) {
                return;
            }
            if self.shape_ui.loaded(&mut self.world, item, asset, &result) {
                return;
            }
            if blender_shape {
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
            if self.wearable_saved(request, &result) {
                return;
            }
            if self.physics_saved(request, &result) {
                return;
            }
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
            if self.particle_inject.owns(request) {
                self.particle_script_uploaded(request, result);
                return;
            }
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
                let actor = std::mem::take(&mut self.appearance_ui.pending_actor);
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
                            let crate::world::appearance::CofSync { mut commands, unknown } =
                                crate::world::appearance::sync_plan(&self.world.inventory, self.world.agent_id, &self.outfit_sync_worn());
                            for attachment in self.appearance_ui.attachments.drain(..) {
                                crate::world::appearance::apply_attachment_override(&mut commands, attachment);
                            }
                            for cmd in commands {
                                self.send_from(cmd, actor);
                            }
                            self.world.outfit.wear_when_known(unknown, self.world.agent_id, Instant::now());
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
                let area_reads: HashSet<_> = props
                    .iter()
                    .filter(|p| p.full && self.area_search_feed.expects(&p.object_id))
                    .map(|p| p.object_id)
                    .collect();
                if self.panels.area_search {
                    self.area_search_feed.on_properties(&props);
                } else {
                    self.area_search_feed.on_closed_properties(&props);
                }
                self.world.ik_targets.on_properties(&props);
                self.report_abuse_properties(&props);
                self.world.on_hud_properties(&props);
                self.scene.particles.on_properties(&props);
                self.interactions.on_properties(&props);
                self.object_profile.on_properties(&props);
                let props = self.area_search_build_properties(props, &area_reads);
                self.build.on_properties(props);
            }
            NetEvent::PayPrice { object, default, buttons } => self.interactions.on_prices(object, default, buttons),
            NetEvent::AbuseCategories(list) => self.report_abuse.set_categories(list),
            NetEvent::AbuseReportSent(result) => self.report_abuse_sent(result),
            NetEvent::TaskInventory { object, serial, result } => {
                if self.demo
                    && let Ok(items) = &result
                {
                    self.build_demo.adopt_contents(object, items);
                }
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
            NetEvent::CapReply { tag, result } => {
                self.area_search_feed.on_cost_reply(tag, &result);
                self.build.on_cap_reply(&mut self.world, tag, result);
            }
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
            NetEvent::SelectedParcel { handle, sequence, parcel } => {
                if !self.world.parcel_visibility_reply(handle, sequence, parcel.clone()) {
                    self.build.on_selected_parcel(handle, sequence, parcel);
                }
            }
            NetEvent::HoverParcel { handle, parcel } => self.on_hover_parcel(handle, parcel),
            NetEvent::LoginProgress { message, fraction } => {
                self.loading_stage = message.clone();
                self.login_progress = Some((message, fraction));
            }
            NetEvent::LoginFailed {
                error,
                mfa_required,
                tos_required,
                bad_credentials,
                previous_session,
            } => {
                // the grid still logs out a previous session: try again in a
                // minute, the countdown on the login screen
                if previous_session && self.retry_after_refusal(&error) {
                    return;
                }
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
                self.session_started(l.session_id);
                self.named_teleport = Default::default();
                if !self.demo {
                    let root = self.world.inventory.root;
                    self.inventory_ui.load_received(
                        crate::settings::account_dir(&l.agent_id).join(format!("received-{root}.json")),
                        root,
                    );
                    self.settings.select_hover_account(l.agent_id);
                    self.settings.save();
                }
                self.bridge_runtime = Default::default();
                self.native_anchor = Default::default();
                self.anchor_stands = Default::default();
                self.anchor_pose = None;
                self.movement_runtime = Default::default();
                self.scene.movement_preview = None;
                self.movement_probe = (None, false, None);
                self.pass_wall_at = None;
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
                // the saved smart searches of the inventory; the demo keeps
                // them in memory
                if !self.demo {
                    crate::ui::inventory::smart_store::load(
                        crate::settings::account_dir(&l.agent_id).join(crate::ui::inventory::smart_store::FILE),
                    );
                }
                // LLAgent::init: the FLY control is set before the first
                // AgentUpdate when the last session of this account ended
                // flying (the arrival lands it where flying is blocked)
                if !self.demo {
                    self.world.agent.flying = crate::world::flying_at_exit::load(
                        &crate::settings::account_dir(&l.agent_id).join(crate::world::flying_at_exit::FILE),
                    );
                } else if crate::scene::wind_sound::demo_spec().is_some_and(|(_, fly)| fly) {
                    self.world.agent.flying = true;
                }
                self.restore_gestures(&l.raw);
                self.net_profile.reset();
                self.login_form.transition = None;
                self.login_form.busy = false;
                self.login_form.adding = false;
                self.login_form.add_return = None;
                self.login_form.editing = false;
                // a start SLURL serves one login
                self.login_form.start_slurl = None;
                // the demo's teleport history: several days, never saved
                if self.demo {
                    self.world.tp_storage = crate::world::tphistory::HistoryStorage::in_memory(crate::demo::place::history());
                    // earlier places in the session, for the arrows' list
                    if crate::ui::bars::demo::get().open == Some(crate::ui::bars::demo::Open::TpList) {
                        self.world.tp_history = crate::world::tphistory::TeleportHistory::seeded(crate::demo::place::session());
                    }
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
                if let Some(id) = attention::keyword_sound(&self.settings, &self.settings.keywords) {
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
                l.message.trim().clone_into(&mut self.world.motd);
                if !self.world.motd.is_empty() {
                    self.world.system_message(self.world.motd.clone());
                }
                self.world.system_message(aurora_i18n::tr!(
                    "app-welcome-2",
                    value0 = l.first_name.to_string(),
                    value1 = l.last_name.to_string()
                ));
                self.restore_local_environment();
            }
            NetEvent::TeleportFinished { .. } => {
                self.stop_local_typing();
                if self.world.tp_show_progress && matches!(self.screen, Screen::World) && !self.was_teleporting {
                    // Finish and arrival can be drained in the same frame.
                    // Start the screen here so a fast remote teleport still
                    // gets its region-loading phase and arrival fade.
                    self.show_teleport_progress();
                }
            }
            NetEvent::AgentMovementComplete { .. } => {
                if let Some(journey) = &mut self.portal_journey {
                    journey.arrived = true;
                }
                aurora_net::transition::step(|| {
                    format!(
                        "viewer: arrival applied ({}), seated {}, {} worn objects",
                        if self.world.arrived_by_crossing { "crossing" } else { "teleport" },
                        self.world.agent.seated,
                        self.scene.own_attachment_count()
                    )
                });
                self.stop_local_typing();
                self.move_complete_at = Some(Instant::now());
                // the simulator we land in plays none of our animations yet
                self.static_eyes.on_arrival();
                self.tp_checks = vec![10.0, 3.0];
                // (a border crossing leaves the camera where it is)
                if !self.world.arrived_by_crossing {
                    self.camera.on_teleport_arrival(&self.settings.camera);
                }
                self.scene.textures.mark_arrival();
                self.loading_stage = aurora_i18n::tr!("app-loading-scene").into();
                self.start_friend_join_flight();
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
            NetEvent::Disconnected { reason, lost } => self.on_disconnected(&reason, lost),
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
                if self.world.sound_blocked(&object, &owner)
                    || self.world.sound_blacklisted(sound, object, owner)
                    || (!parent.is_nil() && self.world.sound_blocked(&parent, &uuid::Uuid::nil()))
                {
                    return;
                }
                // a radar script of ours asks for every avatar around: not a
                // sound to play (process_sound_trigger, legacy radar scripts)
                if owner == self.world.agent_id && sound == crate::world::radar::SCRIPT_ALERT_SYNC_SOUND {
                    self.radar.ask_script_sync();
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
                    // process_sound_trigger: reject parcel-isolated sounds
                    // before asking for their assets.
                    if !self.world.can_hear_sound(off + position) {
                        return;
                    }
                    self.scene
                        .sounds
                        .trigger_owned(sound, off + position, None, gain, crate::scene::sounds::Origin { object, owner });
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
                if !sound.is_nil() && (self.world.sound_blocked(&object, &owner) || self.world.sound_blacklisted(sound, object, owner)) {
                    return;
                }
                self.scene
                    .sounds
                    .set_attached(self.audio_engine.as_ref(), object, sound, gain, flags);
                self.scene.sounds.set_owner(object, owner);
            }
            NetEvent::AttachedSoundGain { object, gain } => {
                self.flush_object_sounds();
                self.scene.sounds.set_gain(object, gain);
            }
            NetEvent::PreloadSounds(sounds) => {
                for sound in sounds {
                    if !self.world.sound_blacklisted(sound.sound, sound.object, sound.owner) {
                        self.scene.sounds.want(sound.sound);
                    }
                }
            }
            NetEvent::LoggedOut => {
                self.session_ended();
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
        let (alt, ctrl) = self.mouse_alt_ctrl();
        matches!(self.screen, Screen::World)
            && !self.camera.mouselook()
            && !alt
            && !ctrl
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
            && (!self.huds_shown() || self.scene.hud_pick(&self.world, self.cursor_pos, false).is_none())
    }

    /// The camera drag a left click would start under the pointer, like
    /// LLToolCamera::handleHover (lltoolfocus.cpp): Alt held over the world
    /// (AltGr as Ctrl + Alt, app/alt_graph.rs), or the build Focus tool.
    fn camera_tool_hover(&self, ctx: &egui::Context) -> Option<crate::camera::ToolMode> {
        use crate::camera::ToolMode;
        let focus_tool = self.build.open && self.build.tool == crate::build::Tool::Focus;
        // (AltGr typing a character in a field leaves the cursor as it is)
        let (alt, ctrl) = if ctx.egui_wants_keyboard_input() {
            (self.alt, self.ctrl)
        } else {
            self.mouse_alt_ctrl()
        };
        if !matches!(self.screen, Screen::World)
            || self.mouse_mode != MouseMode::None
            || self.camera.mouselook()
            || !(focus_tool || alt)
            || ctx.is_pointer_over_egui()
            || ctx.egui_wants_pointer_input()
        {
            return None;
        }
        Some(if focus_tool && !ctrl && !self.shift {
            match self.build.focus_mode {
                crate::build::FocusMode::Zoom => ToolMode::Zoom,
                crate::build::FocusMode::Orbit => ToolMode::Orbit,
                crate::build::FocusMode::Pan => ToolMode::Pan,
            }
        } else {
            ToolMode::from_mods(ctrl, self.shift)
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
            // Alt held: the grab tool has the free cursor, the view stays
            if !self.alt_cursor.free() {
                self.camera.look(dx, dy, s, &mut self.world.agent);
            }
        } else if self.mouse_mode != MouseMode::None {
            // LLToolCamera: steering is the zoom tool on our avatar; the
            // modifiers held now choose the drag (AltGr: Ctrl + Alt)
            let (_, ctrl) = self.mouse_alt_ctrl();
            let focus_tool = self.build.open && self.build.tool == crate::build::Tool::Focus && !ctrl && !self.shift;
            let mode = match self.mouse_mode {
                MouseMode::Steer => crate::camera::ToolMode::Zoom,
                _ if focus_tool => match self.build.focus_mode {
                    crate::build::FocusMode::Zoom => crate::camera::ToolMode::Zoom,
                    crate::build::FocusMode::Orbit => crate::camera::ToolMode::Orbit,
                    crate::build::FocusMode::Pan => crate::camera::ToolMode::Pan,
                },
                _ => crate::camera::ToolMode::from_mods(ctrl, self.shift),
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
        if spec.ends_with("-body-on") || spec.ends_with("-body-off") {
            self.settings.camera.show_avatar_in_mouselook = spec.ends_with("-body-on");
            self.settings.camera.mouselook_follow_eyes = spec.contains("-eyes-on");
            if self.frame_count == 500 {
                for event in crate::demo::first_person_visibility_events() {
                    self.world.apply(event);
                }
            }
        }
        if spec.contains("-eyes-") && self.frame_count == 540 {
            let id = uuid::Uuid::from_u128(0x1015_eeee);
            self.scene.anims.insert(
                id,
                Arc::new(crate::scene::anim::BoundAnim::bind(
                    crate::demo::first_person_eye_animation(),
                    &self.world.avatar_lib.rig,
                )),
            );
            let mut anims = vec![(id, 1)];
            if self.world.agent.ground_sit {
                anims.push((crate::world::body::ANIM_SIT_GROUND_CONSTRAINED, 1));
            }
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: self.world.agent_id,
                anims,
                sources: Vec::new(),
            });
        }
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
                Step::AltGraph(held) => self.alt_graph = held,
                Step::Press => self.with_mouse_mods(Self::on_left_press),
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
                Step::Crouch(on) => {
                    self.settings.crouch_toggle = true;
                    if on {
                        self.down.insert(Input::key(KeyCode::PageDown));
                    } else {
                        self.down.remove(&Input::key(KeyCode::PageDown));
                    }
                }
                Step::ToggleMouselook => self.toggle_mouselook(),
                Step::GroundSit => {
                    if !self.world.agent.is_sitting() {
                        self.scene.anims.insert(
                            crate::world::body::ANIM_SIT_GROUND_CONSTRAINED,
                            Arc::new(crate::scene::anim::BoundAnim::bind(
                                crate::demo::first_person_ground_animation(),
                                &self.world.avatar_lib.rig,
                            )),
                        );
                    }
                    self.toggle_ground_sit();
                }
                Step::Wheel(clicks) => self.camera_wheel(clicks),
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
                    for ev in crate::camera::demo::sit_events(spec, self.frame_count) {
                        if let Some(e) = self.world.apply(ev) {
                            self.on_app_event(e);
                        }
                    }
                }
                Step::ZoomResident => {
                    let me = self.world.agent.position;
                    let nearest = self
                        .world
                        .objects
                        .iter()
                        .filter(|(_, o)| o.is_avatar() && o.parent_id == 0 && o.full_id != self.world.agent_id)
                        .min_by(|a, b| a.1.position.distance(me).total_cmp(&b.1.position.distance(me)))
                        .map(|(_, o)| o.full_id);
                    match nearest {
                        Some(id) => self.on_ctx_action(ui::context::CtxAction::ZoomAvatar(id)),
                        None => log::warn!("demo camera: no standing resident to zoom on"),
                    }
                }
                Step::MoveResident(x, y) => {
                    if let Some(idx) = self.camera.focus_object_id().and_then(|id| self.world.objects.index_of_uuid(&id))
                        && let Some(o) = self.world.objects.get_mut(idx)
                    {
                        o.position += glam::Vec3::new(x, y, 0.0);
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
        self.stop_autopilot(true);
        self.joystick.active = false;
        self.camera.clear_flycam();
        let was = self.camera.mouselook();
        self.camera.handle_reset_view(&mut self.world.agent, &self.settings.camera);
        if was && !self.camera.mouselook() {
            self.set_mouselook_grab(false);
        }
    }

    fn mods(&self) -> Mods {
        self.shortcut_mods(Mods {
            ctrl: self.ctrl,
            shift: self.shift,
            alt: self.alt,
        })
    }

    /// Walk bindings taking part in double-tap running: 0 forward, 1 back,
    /// 2 left, 3 right (turn keys only when they strafe).
    fn run_group(&self, input: &Input) -> Option<u8> {
        // steering with the left button: left / right slide
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

    /// The right button walks forward while the left button steers our avatar.
    fn mouse_walks(&self) -> bool {
        self.right_drag && self.mouse_mode == MouseMode::Steer
    }

    /// LLToolCamera's avatar steering, also used by the optional world chord.
    fn begin_mouse_steer(&mut self) {
        self.camera.begin_drag(true);
        self.mouse_mode = MouseMode::Steer;
        if self.right_drag {
            self.right_gesture.walked = true;
        }
        self.set_mouselook_grab(true);
    }

    /// Called only after HUD, media and object click actions have had priority.
    /// Unlike LLToolPie::startCameraSteering (indra/newview/lltoolpie.cpp), this
    /// Aurora option requires both buttons; a lone world press keeps its use.
    fn arm_world_mouse_walk(&mut self) {
        self.world_mouse_walk_pending = self.settings.world_mouse_walk
            && !self.alt
            && !self.object_view
            && !self.ctrl
            && !self.shift
            && !self.build.open
            && !self.camera.mouselook();
        self.start_world_mouse_walk();
    }

    fn start_world_mouse_walk(&mut self) {
        if self.world_mouse_walk_pending && self.left_down && self.right_drag && self.mouse_mode == MouseMode::None {
            let s = &self.settings.camera;
            if !s.click_avatar_keeps_camera {
                self.camera.set_focus_on_avatar(true, true, true, &mut self.world.agent, s);
            }
            self.begin_mouse_steer();
        }
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
            if !self.always_run && !self.joystick.running {
                self.send(NetCommand::SetAlwaysRun(false));
            }
        }
    }

    /// Master × media channel volume, 0 when either is muted or disabled
    /// (LLViewerMedia::getVolume): the media on faces and the videos of the
    /// conversations' cards. Demo captures stay silent unless
    /// AURORA_DEMO_SOUND is set.
    fn media_volume(&self) -> f32 {
        if self.demo && std::env::var_os("AURORA_DEMO_SOUND").is_none() {
            return 0.0;
        }
        audio_mute::media_volume(&self.settings.audio, self.minimized_mute.active)
    }

    /// Why we may not speak where we stand (LLAgent::isActionAllowed("speak"),
    /// indra/newview/llagent.cpp): the region or the parcel forbids nearby
    /// voice. A call has its own channel and is never denied.
    fn speak_denied(&self) -> Option<String> {
        if self.world.calls.active.is_some() {
            return None;
        }
        let region_flags = self.world.main().and_then(|r| r.info.as_ref()).map(|i| i.region_flags);
        crate::voice::spatial_voice_denied(region_flags, self.world.parcel.as_deref())
    }

    /// Switch the microphone (toggle mode), unless speaking is denied here
    /// (toggle_voice, indra/newview/llviewerinput.cpp).
    fn toggle_mic(&mut self) {
        if self.speak_denied().is_none() {
            self.mic_on = !self.mic_on;
        }
    }

    /// Voice transmission wanted (microphone button / push-to-talk).
    fn talking(&self) -> bool {
        if self.minimized_mute.active || self.speak_denied().is_some() {
            false
        } else if self.settings.audio.mic_hold {
            self.mic_button_held || self.settings.keybinds.held(Action::PushToTalk, &self.down, self.mods())
        } else {
            self.mic_on
        }
    }

    /// A key or a bindable mouse button changed state.
    fn on_input(&mut self, input: Input, pressed: bool) {
        let mods = self.mods();
        let action = self.settings.keybinds.update_held_input(&mut self.down, &input, pressed, mods);
        if let Some(group) = self.run_group(&input) {
            self.walk_tap(group, pressed && action.is_none());
        }
        if pressed && login_shortcut(&self.screen, action) {
            ui::widgets::toggle_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings);
            return;
        }
        if !pressed || !self.in_world() {
            return;
        }
        // push-to-talk key in toggle mode: each press switches the microphone
        if !self.settings.audio.mic_hold && self.settings.keybinds.held_actions_of(&input).contains(&Action::PushToTalk) {
            self.toggle_mic();
        }
        if self
            .settings
            .movements
            .get(&self.world.agent_id)
            .is_some_and(|prefs| prefs.pass_wall)
            && self.settings.keybinds.pass_wall_triggered(&input, self.mods())
        {
            self.pass_wall();
            return;
        }
        let Some(action) = action else {
            return;
        };
        match action {
            Action::Fly => self.toggle_fly(),
            Action::Flycam => self.toggle_joystick_flycam(),
            Action::AlwaysRun => self.set_always_run(!self.always_run),
            Action::MoveLock => self.toggle_move_lock(),
            Action::MovementEnhancer => self.open_movement(),
            Action::PassWall => self.pass_wall(),
            Action::SitGround => self.sit_stand_shortcut(),
            Action::Mouselook => self.toggle_mouselook(),
            Action::CameraControls => self.panels.camera_controls = !self.panels.camera_controls,
            Action::LensZoomIn | Action::LensZoomOut | Action::LensZoomDefault => {
                if self.world.rlva.camera_owner().is_none() {
                    let aspect = self.view_aspect();
                    let factor = match action {
                        Action::LensZoomIn => Some(1.0 / 1.2),
                        Action::LensZoomOut => Some(1.2),
                        _ => None,
                    };
                    self.settings.camera.fov =
                        crate::camera::zoom_lens(self.settings.camera.fov, factor, aspect, self.world.rlva.camera_fov_limits(aspect));
                    if !self.demo {
                        self.settings.save();
                    }
                }
            }
            Action::ResetCamera => {
                if !self.photo_escape() {
                    self.reset_camera_view();
                }
            }
            Action::Chat => self.chat_ui.focus_request = true,
            Action::ToggleMic => self.toggle_mic(),
            // the shortcuts of the windows, like their toolbar buttons: a
            // minimized window expands (LLFloaterReg::toggleInstance)
            Action::Conversations => ui::widgets::toggle_floater(&self.egui_ctx, "conversations", &mut self.panels.chat),
            Action::People => {
                ui::widgets::toggle_floater(&self.egui_ctx, "people", &mut self.panels.people);
                self.panels.people_tab = 0;
            }
            Action::Inventory => ui::widgets::toggle_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory),
            Action::TextureLibrary => ui::widgets::toggle_floater(
                &self.egui_ctx,
                self.settings.texture_library.presentation.window(),
                &mut self.panels.texture_library,
            ),
            Action::IkEdit => {
                self.ik_editor.open_ik();
                ui::widgets::open_floater(&self.egui_ctx, "ik-edit", &mut self.ik_editor.open);
            }
            Action::Gestures => ui::widgets::toggle_floater(&self.egui_ctx, "gestures", &mut self.panels.gestures),
            Action::Minimap => self.panels.minimap = !self.panels.minimap,
            Action::WorldMap => ui::widgets::toggle_floater(&self.egui_ctx, "worldmap", &mut self.panels.world_map),
            Action::TeleportHistory => self.toggle_teleport_history(),
            Action::RegionEstate => ui::widgets::toggle_floater(&self.egui_ctx, ui::region::WINDOW, &mut self.panels.region_estate),
            Action::Search => ui::widgets::toggle_floater(&self.egui_ctx, ui::search::WINDOW, &mut self.panels.search),
            Action::Performance => ui::widgets::toggle_floater(&self.egui_ctx, "perf", &mut self.panels.perf),
            Action::DevToolbox => ui::widgets::toggle_floater(&self.egui_ctx, ui::dev_toolbox::WINDOW, &mut self.panels.dev_toolbox),
            Action::Preferences => ui::widgets::toggle_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings),
            Action::ToggleInterface => self.toggle_interface(),
            Action::HoverTips => self.toggle_hover_tips(),
            Action::ShowHuds => self.set_show_huds(!self.settings.show_huds),
            Action::Import => self.imports.toggle_window(&self.egui_ctx),
            Action::Beacons => ui::widgets::toggle_floater(&self.egui_ctx, ui::beacons::WINDOW, &mut self.panels.beacons),
            Action::PropertyLines => {
                self.settings.world_overlays.property_lines = !self.settings.world_overlays.property_lines;
                self.settings.save();
            }
            Action::LandOwners => {
                self.settings.world_overlays.land_owners = !self.settings.world_overlays.land_owners;
                self.settings.save();
            }
            Action::HideParticles => {
                self.settings.world_overlays.hide_particles = !self.settings.world_overlays.hide_particles;
                self.settings.save();
            }
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
            Action::StopAnimations => self.stop_animations(false),
            Action::ResyncAnimations => self.scene.resync_animations(Instant::now()),
            Action::RefreshAttachments => self.refresh_attachments(),
            Action::PhotoWindow => ui::widgets::toggle_floater(&self.egui_ctx, ui::snapshot::WINDOW, &mut self.panels.snapshot),
            Action::PhotoToDisk => self.photo_to_disk(),
            Action::EnvSunrise | Action::EnvMidday | Action::EnvSunset | Action::EnvMidnight | Action::EnvShared => {
                self.env_shortcut(action)
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
        let (id, point) = plate_click::visible_tag(&self.name_tag_candidates(ray), hit, ray)?;
        Some((self.world.objects.index_of_uuid(&id)?, point))
    }

    /// Left press on the world, past the build tools (LLToolPie).
    fn on_left_press(&mut self) {
        self.click(click_pick::Click::World);
    }

    /// What `on_left_press` does, run by `click`: again from the start when
    /// the depth under the cursor has come back (app/click_pick.rs).
    fn world_press(&mut self) {
        self.left_down = true;
        self.world_mouse_walk_pending = false;
        if self.hud_drag_requested()
            && self.huds_shown()
            && let Some(idx) = self.scene.hud_move_pick(&self.world, self.cursor_pos)
            && !(self.alt && self.world.hud_move_locked(idx))
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
        if let Some((idx, point, hud_ray)) = self.scene.hud_pick(&self.world, (x, y), false)
            && !(self.alt && self.world.hud_move_locked(idx))
        {
            self.media.build_mode = self.build.open;
            if !self.alt && !self.ctrl && !self.shift && !self.build.open {
                let mods = aurora_media::Modifiers {
                    control: false,
                    alt: false,
                    shift: false,
                    command: false,
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
        // the click combo counts the press once, not again with the depth
        if !self.clicks.replaying() && self.plate_combo_press() {
            return;
        }
        let Some(picked) = self.click_pick() else {
            return;
        };
        // (kept until the depth is in hand: the press is run again then)
        let double = std::mem::take(&mut self.double_press);
        let (ray, depth) = (picked.ray, picked.depth);
        // another avatar's plate with a click action takes the click
        if self.plate_press(picked.tag) {
            return;
        }
        let alt = self.alt || self.object_view;
        // a rigged mesh with nothing to do with the click lets it through
        let depth = if alt || self.build.open {
            depth
        } else {
            self.scene
                .touch_depth(&self.world, &picked.drawn, ray, depth, self.settings.draw_distance)
        };
        let hit = if alt {
            picked.visible_point()
        } else {
            self.scene
                .action_point(&self.world, ray, depth, self.build.open, self.settings.draw_distance)
        };
        // a rigged mesh or a worn mesh shown there: the GPU alone poses it
        let shown = (!alt)
            .then(|| self.designate(crate::scene::drawn::Gesture::Touch, &picked, hit))
            .flatten();
        let hit = shown.map_or(hit, |d| Some(d.hit.point));
        // on our avatar: a surface inside its box under the cursor (nothing
        // in front of it); or, when the ray goes through the box without
        // touching anything (body not drawn yet), close to the body's axis
        let avatar_hit = ray.filter(|_| shown.is_none_or(|d| !d.blend)).and_then(|(o, d)| {
            let (t0, t1, axis) = self.own_avatar_ray(o, d)?;
            match hit.map(|p| (p - o).dot(d)) {
                Some(t) if t < t0 - 0.05 => None,
                Some(t) if t <= t1 + 0.05 => hit,
                _ if alt => None,
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
        if !alt && !on_avatar {
            if let Some(ray) = ray {
                let depth_t = hit.map(|p| (p - ray.0).dot(ray.1));
                let mods = aurora_media::Modifiers {
                    control: self.ctrl,
                    alt: self.alt,
                    shift: self.shift,
                    command: self.command,
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
        // us touch (its rigged ones are out of the left click's reach);
        // one without an action is not designated, the click went on behind
        if !alt
            && !self.ctrl
            && !self.shift
            && !self.build.open
            && !self.camera.mouselook()
            && let Some(d) = shown
            && let Some(target) = crate::interaction::target(&self.world, d.idx, &self.interactions.props)
        {
            let surface = self.scene.drawn_surface(&self.world, &d);
            if target.action == crate::interaction::Action::Disabled {
                self.arm_world_mouse_walk();
            }
            self.activate_object_action_at(target, d.idx, d.hit.point, ray, surface);
            return;
        }
        // a prim we wear takes the click before our avatar does
        // (LLToolPie::handleLeftClickPick: touchable first, then the avatar)
        if shown.is_none()
            && !alt
            && !self.ctrl
            && !self.shift
            && !self.build.open
            && !self.camera.mouselook()
            && let Some(point) = hit
            && let Some(pick_ray) = ray
            && let Some(idx) = self.scene.interaction_at_ray(&self.world, point, pick_ray, true)
            && (!on_avatar || self.own_attachment(idx))
            && let Some(target) = crate::interaction::target(&self.world, idx, &self.interactions.props)
        {
            if target.action == crate::interaction::Action::Disabled {
                self.arm_world_mouse_walk();
            }
            self.activate_object_action(target, idx, point, ray);
            return;
        }
        if alt {
            // Alt+click (LLToolCamera): lock the camera on the clicked point
            // (our avatar too, to look at a part of it closely); keep holding
            // to zoom, Ctrl to orbit, Ctrl+Shift to pan
            if let (Some(p), Some(ray)) = (hit, ray) {
                let idx = match picked.drawn.blend.or(picked.drawn.opaque) {
                    // Terrain has no object: do not bind the camera to an
                    // invisible prim whose box happens to enclose its point.
                    Some(d) => self.scene.record_object(d.record).map(|(idx, _)| idx),
                    None if on_avatar => own_idx,
                    None => self.scene.interaction_at(&self.world, p, now, self.build.open),
                };
                let object = idx.and_then(|i| crate::camera::pick_focus_object(&self.world, i, now));
                self.camera.alt_focus(p, object, ray, &mut self.world.agent, &self.settings.camera);
                self.camera.begin_drag(false);
                self.mouse_mode = MouseMode::FocusOrbit;
                self.hold_focus_cursor();
            }
        } else if let (Some(p), Some(ray)) = (avatar_hit, ray) {
            // ClickOnAvatarKeepsCamera (LLToolPie::handleLeftClickPick).
            // Unlike LLToolCamera::pickCallback (lltoolfocus.cpp), keep the
            // focus too when enabled: a plain press must not reframe (#1243).
            let s = &self.settings.camera;
            if !s.click_avatar_keeps_camera {
                let object = own_idx.and_then(|i| crate::camera::pick_focus_object(&self.world, i, now));
                self.camera.alt_focus(p, object, ray, &mut self.world.agent, s);
                self.camera.set_focus_on_avatar(true, true, true, &mut self.world.agent, s);
            }
            self.begin_mouse_steer();
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
            if picked.tag.is_none() {
                self.arm_world_mouse_walk();
            }
        } else if !self.build.open {
            if hit.is_some() && picked.tag.is_none() {
                self.arm_world_mouse_walk();
            }
            // a press nothing took (scanMouse): scripts that took the button hear it
            self.script_click_down((control::LBUTTON_DOWN, control::LBUTTON_UP));
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
            s.private_local = self.demo_look_ahead;
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
        // no target even for us (PrivateLocalLookAtTarget) turned on: the
        // last one is cleared at once (LOOKAT_TARGET_CLEAR, which a focus
        // that never times out would otherwise outlive), and head and eyes
        // look ahead (lookat::gaze)
        if prefs.private_local && self.look_at_privacy.is_some_and(|(_, was)| !was) {
            self.world.look_at.own.set(lookat::CLEAR, None, glam::DVec3::ZERO, now);
        }
        self.world.look_at.own_ahead = prefs.private_local;
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

    /// AURORA_DEMO_LOOKAT=leave (issue #482): at frame 230 Violet Wolf looks
    /// at us (his mark on our head), at frame 800 he logs off (the
    /// simulator kills his avatar): his mark must go with him.
    fn demo_look_at_leave(&mut self) {
        use crate::world::lookat;
        let wolf = crate::demo::DEMO_LOUP;
        let ev = match self.frame_count {
            230 => NetEvent::LookAt {
                effect: uuid::Uuid::from_u128(0x100c),
                source: wolf,
                target: self.world.agent_id,
                offset: [0.0; 3],
                kind: lookat::CONVERSATION,
                duration: lookat::MAX_TIMEOUT,
            },
            800 => {
                let Some(key) = self
                    .world
                    .objects
                    .index_of_uuid(&wolf)
                    .and_then(|i| self.world.objects.get(i))
                    .map(|o| o.key)
                else {
                    return;
                };
                NetEvent::ObjectsKilled {
                    handle: key.region,
                    local_ids: vec![key.local_id],
                }
            }
            _ => return,
        };
        if let Some(e) = self.world.apply(ev) {
            self.on_app_event(e);
        }
        log::info!(
            "demo lookat leave: frame {}, remote look-ats {}",
            self.frame_count,
            self.world.look_at.remote.len()
        );
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
    /// still is started after every arrival, then started or stopped as the
    /// setting changes (`StaticEyes`).
    fn sync_static_eyes(&mut self) {
        if self.move_complete_at.is_none() {
            return;
        }
        if let Some(start) = self.static_eyes.request(self.settings.lookat.static_eyes) {
            log::info!("static eyes: {} asked of the simulator", if start { "start" } else { "stop" });
            self.send(NetCommand::AgentAnimation {
                anim: crate::world::lookat::STATIC_EYES,
                start,
            });
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
        let version = self
            .world
            .mutes
            .version
            .wrapping_add(self.world.anti_spam.version)
            .wrapping_add(self.world.derender.generation);
        let recheck = self.sound_blocks.0 != version;
        self.sound_blocks.0 = version;
        let blocked = |world: &World, o: &crate::world::objects::Object| {
            world.sound_blocked(&o.full_id, &o.owner_id) || world.sound_blacklisted(o.sound.id, o.full_id, o.owner_id)
        };
        if recheck {
            // rare: every object with a sound, but for those whose update is
            // applied below
            for o in self.world.objects.slots.iter().flatten() {
                // AttachedSound packets can have started a loop after a
                // nil object snapshot. A new block must stop that loop too.
                if o.sound_update {
                    continue;
                }
                if blocked(&self.world, o) {
                    if self.sound_blocks.1.insert(o.full_id) {
                        self.scene.sounds.apply_object_sound(
                            engine,
                            o.full_id,
                            aurora_net::objects::AttachedSound {
                                flags: 1 << 5,
                                ..Default::default()
                            },
                        );
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
            let Some(o) = self.world.objects.get(idx) else {
                continue;
            };
            if blocked(&self.world, o) {
                self.sound_blocks.1.insert(o.full_id);
                self.scene.sounds.apply_object_sound(
                    engine,
                    o.full_id,
                    aurora_net::objects::AttachedSound {
                        flags: 1 << 5,
                        ..Default::default()
                    },
                );
            } else {
                self.sound_blocks.1.remove(&o.full_id);
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
                    if !self.world.rlva_start_ims(&[session]) {
                        return;
                    }
                    if !self.world.social.ims.iter().any(|s| s.other == session) {
                        self.world.social.session_mut(session);
                    }
                    self.world.social.focus_im = Some(session);
                    self.chat_ui.conv_focus_request = true;
                    self.notif_ui.open = false;
                    ui::notifications::open_conversations(ctx, &mut self.panels.chat);
                }
                Follow::Place(link) => {
                    if let Some(l) = crate::slurl::parse(&link) {
                        self.notif_ui.open = false;
                        ui::context::request(ctx, ui::context::CtxAction::ShowPlace(l.target(), l.pos));
                    }
                }
                Follow::Inventory(item) => {
                    self.inventory_ui.show_notified(&self.world.inventory, item);
                    self.panels.inventory = true;
                    self.notif_ui.open = false;
                }
            },
        }
    }

    /// A Marketplace delivery's "Show in inventory" (#752): the card is
    /// read, the delivery selected in the inventory.
    fn show_delivery(&mut self, id: u64, object: uuid::Uuid) {
        let _ = self.world.respond_notification(id, crate::world::notifications::Response::Open);
        self.inventory_ui.show_notified(&self.world.inventory, object);
        self.panels.inventory = true;
        self.notif_ui.open = false;
    }

    fn update_sounds(&mut self) {
        self.flush_object_sounds();
        let ui = &self.settings.audio.ui;
        let w = &mut self.world;
        // conversations by their IM mode (LLIMMgr::addMessage), never in
        // do not disturb; "in front" = shown in the focused window
        let dnd = w.status.dnd;
        // only people's lines count and alert (#741; AURORA_DEMO_OBJECT_CHAT=…:on
        // for the run)
        w.quiet_automated_chat = self.settings.compact_automated_chat || (self.demo && crate::demo::object_chat::on());
        w.notifications.toasts = crate::world::notifications::ToastPrefs {
            presence: self.settings.toast_friends_online,
            presence_in_chat: self.settings.friends_online_in_chat,
            im: self.settings.toast_im,
            // AURORA_DEMO_TOASTS shows a group message too (off by default)
            group_im: self.settings.toast_group_im || (self.demo && crate::demo::toasts::mode().is_some()),
            // and keeps the messages under the bell for its modes that list
            // or click them
            im_in_bell: self.settings.ims_in_notification_center
                || (self.demo && crate::demo::toasts::mode().is_some_and(|m| matches!(m.as_str(), "bell" | "click" | "minimized"))),
        };
        // a conversation in sight: its tab shown in the Conversations window
        // or its own window (FSFloaterIM visible), whether or not the viewer
        // has the focus
        let minimized = crate::ui::widgets::floater_minimized(&self.egui_ctx, "conversations");
        let in_sight = crate::ui::chat::conversations_in_sight(&self.egui_ctx, &self.chat_ui, self.panels.chat);
        // the Conversations button goes out once the window is shown
        // (FSFloaterIMContainer::setVisible), or with its preference off
        let flash = &mut self.chat_ui.button_flash;
        flash.set_on(self.settings.flash_conversations_button);
        flash.set_still(self.settings.reduce_motion || (self.demo && crate::demo::conv_button::still()));
        if self.panels.chat {
            flash.shown();
        }
        // nearby chat in the Conversations window, or in its own (#910)
        let nearby_apart = self.chat_ui.nearby_apart();
        let nearby_out_of_sight =
            crate::ui::chat::nearby_out_of_sight(nearby_apart, self.panels.chat, self.chat_ui.nearby_window_shown(&self.egui_ctx));
        if !nearby_out_of_sight {
            self.chat_ui.button_flash.nearby_seen();
        }
        let now = self.egui_ctx.input(|i| i.time);
        for m in std::mem::take(&mut w.im_messages) {
            let front =
                self.focused && ((self.panels.chat && self.chat_ui.selected == Some(m.session)) || in_sight.contains(&Some(m.session)));
            if !dnd && let Some(s) = ui.im_sound(m, front) {
                w.ui_sounds.push(s);
            }
            // closed: the button flashes; minimized: the window's bar
            if self.settings.flash_conversations_button && (!self.panels.chat || minimized) && w.message_flashes_conversations(&m) {
                self.chat_ui.button_flash.message(now, !self.panels.chat);
            }
            w.notify_message(&m, in_sight.contains(&Some(m.session)));
            w.message_attention(&m);
        }
        // a resident in nearby chat, out of sight (FSNotifyNearbyChatFlash)
        if std::mem::take(&mut w.nearby_news)
            && crate::ui::chat::nearby_flashes(
                self.settings.flash_conversations_button,
                // `flash_nearby`, field by field: the world is borrowed
                self.settings.flash_conversations_nearby && !(self.demo && crate::demo::conv_button::nearby_off()),
                nearby_out_of_sight,
                dnd,
            )
        {
            self.chat_ui.button_flash.nearby(now);
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

    /// StreamTitleDisplay::checkMetadata (streamtitledisplay.cpp): the title
    /// in nearby chat or in a notice, and on a script channel when asked.
    fn tell_stream_title(&mut self, title: &str) {
        let a = &self.settings.audio;
        match a.stream_title {
            crate::settings::StreamTitleShow::Off => {}
            crate::settings::StreamTitleShow::Chat => self.world.system_notice(
                aurora_i18n::tr!("app-stream-title-now-playing", title = title.to_owned()),
                crate::world::notice_topics::Topic::Media,
            ),
            crate::settings::StreamTitleShow::Toast => {
                self.world.notifications.push(
                    crate::world::notifications::Kind::Info,
                    aurora_i18n::tr!("app-stream-title-toast"),
                    title.to_owned(),
                    crate::world::notifications::Data::None,
                );
            }
        }
        // FSCommon::send_message_to_script_channel: a whisper
        if a.stream_title_announce && a.stream_title_channel != 0 {
            let channel = a.stream_title_channel;
            self.send(NetCommand::Chat {
                message: title.to_owned(),
                channel,
                chat_type: aurora_net::ChatType::Whisper,
            });
        }
    }

    /// audio_update_wind: the wind at our avatar, heard while flying above
    /// the water.
    fn update_wind_sound(&mut self, dt: f32) {
        let agent = &self.world.agent;
        let region_wind = self
            .world
            .main()
            .map_or(Vec3::ZERO, |r| r.wind.velocity(agent.position, r.heightmap.size_x as f32));
        // demo runs stay silent unless AURORA_DEMO_SOUND or AURORA_DEMO_WIND
        let demo_silent = self.demo && std::env::var_os("AURORA_DEMO_SOUND").is_none() && crate::scene::wind_sound::demo_spec().is_none();
        let input = crate::scene::wind_sound::WindInput {
            region_wind,
            velocity: agent.velocity,
            rotation: agent.body_rotation(),
            audible: matches!(self.screen, Screen::World)
                && !demo_silent
                && agent.flying
                && !agent.seated
                && agent.position.z >= self.world.main_water_height(),
        };
        self.wind_sound.update(self.audio_engine.as_ref(), &input, dt);
    }

    /// The media filter's question while one waits (the alerts of
    /// filterMediaUrl / filterAudioUrl), its answer kept as asked. The first
    /// one explains the filter and offers to turn it off (EnableMediaFilter).
    fn media_filter_question(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        use crate::media::filter::Remember;
        use ui::media_filter::Answer;
        if !self.settings.media.filter {
            return;
        }
        let first_use = !self.settings.media.filter_explained;
        let Some(q) = self.world.media_filter.question() else {
            return;
        };
        let Some(answer) = ui::media_filter::prompt(ctx, p, q, first_use, &mut self.media_filter_prompt) else {
            return;
        };
        let url = q.url.clone();
        let filter = &mut self.world.media_filter;
        match answer {
            Answer::Allow(remember) => filter.answer(&url, true, remember),
            Answer::Deny(remember) => filter.answer(&url, false, remember),
            Answer::TurnOff => {
                log::info!("media filter: turned off from its first question");
                self.settings.media.filter = false;
                filter.answer(&url, true, Remember::Once);
            }
        }
        if first_use || !self.settings.media.filter {
            self.settings.media.filter_explained = true;
            // the demo leaves the shared demo profile alone
            if !self.demo {
                self.settings.save();
            }
        }
    }

    /// The demo's media filter stays in memory: the demo account's file
    /// would carry one run's answers into the next.
    /// AURORA_DEMO_MEDIA_FILTER: `ask` (or `1`) starts a parcel media on a
    /// site the list does not know, `first` the same with the first use's
    /// explanation, `music` the parcel music on such a site, `list` puts a
    /// few sites in the list (Preferences › Sound and voice › Media).
    fn demo_media_filter(&mut self) {
        use crate::media::filter::{Action, Edit, MediaFilter};
        self.world.media_filter = MediaFilter::default();
        let Ok(mode) = std::env::var("AURORA_DEMO_MEDIA_FILTER") else {
            return;
        };
        // the scenario shows the filter whatever the demo profile says
        self.settings.media.filter = true;
        self.settings.media.filter_explained = mode != "first";
        match mode.as_str() {
            "ask" | "1" | "first" => {
                if let Some(parcel) = self.world.parcel.as_mut() {
                    let pc = Arc::make_mut(parcel);
                    pc.media_url = "https://media.example.com/welcome/index.html".into();
                    pc.media.mime = "text/html".into();
                    pc.media.media_id = crate::demo::parcel_media_texture();
                    pc.media.auto_scale = true;
                }
                self.media.play_parcel(&self.world, &self.settings.media);
            }
            "music" => {
                if let Some(parcel) = self.world.parcel.as_mut() {
                    Arc::make_mut(parcel).music_url = "http://radio.example.org:8000/live".into();
                }
                self.music_playing = true;
            }
            "list" => {
                for (site, action) in [
                    ("video.example.com", Action::Allow),
                    ("tracker.example.net", Action::Deny),
                    ("https://radio.example.org/live.mp3", Action::Allow),
                ] {
                    self.world.media_filter.apply(Edit::Set(site.into(), action));
                }
            }
            // media on an item we wear (on our back, or a HUD), on a site the
            // list does not know: no question unless `-ask` turns
            // filter_trust_own_worn off; `-blocked` puts the site in the list
            // as blocked
            "worn" | "worn-ask" | "worn-blocked" | "hud" | "hud-ask" | "hud-blocked" => {
                let url = "https://media.example.com/earring/index.html";
                self.settings.media.filter_trust_own_worn = !mode.ends_with("-ask");
                if mode.ends_with("-blocked") {
                    self.world.media_filter.apply(Edit::Set("media.example.com".into(), Action::Deny));
                }
                if mode.starts_with("hud") {
                    self.demo_hud_media(url);
                } else {
                    if std::env::var_os("AURORA_DEMO_WORN").is_none() {
                        for ev in crate::demo::worn::events() {
                            self.world.apply(ev);
                        }
                    }
                    self.media.demo_entry(crate::demo::worn::root().full_id, 0, url);
                }
            }
            _ => log::warn!("AURORA_DEMO_MEDIA_FILTER: unknown mode {mode}"),
        }
    }

    /// AURORA_DEMO_HUDS=media (and the media filter's `hud` modes): a worn
    /// HUD screen with media on `url`, playing at once.
    fn demo_hud_media(&mut self, url: &str) {
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
        self.media.demo_entry(hud.full_id, face, url);
        self.world.apply(NetEvent::ObjectUpdates {
            handle: self.world.main_region.unwrap_or_default(),
            objects: vec![hud],
        });
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
        let mut want = if self.music_playing { url } else { String::new() };
        // the media filter (filterAudioUrl): nothing reaches the station
        // before the resident answered; a refusal turns the radio off
        if !want.is_empty() && self.settings.media.filter {
            use crate::media::filter::{Source, Verdict};
            match self.world.media_filter.check(&want, Source::Music) {
                Verdict::Allow => {}
                Verdict::Ask => want.clear(),
                Verdict::Deny => {
                    self.music_playing = false;
                    self.music_override = None;
                    want.clear();
                }
            }
        }
        if want != self.stream_url {
            if want.is_empty() {
                engine.stop_stream();
            } else {
                log::info!("parcel music: {want}");
                engine.play_stream(&want);
            }
            self.stream_url = want;
        }
        // StreamTitleDisplay: the station's title, told when it changes
        let title = match engine.stream_status() {
            aurora_audio::StreamStatus::Playing { title } if !self.stream_url.is_empty() => title,
            _ => None,
        };
        // the stream title window's (FSStreamTitleManager): the title now,
        // empty once the stream stops; the demo's titles take its place
        if !(self.demo && crate::demo::stream_title::scenario().is_some()) {
            let now = if self.stream_url.is_empty() {
                ""
            } else {
                title.as_deref().unwrap_or("")
            };
            self.stream_titles.update(now, self.settings.audio.stream_title_history as usize);
        }
        if self.stream_url.is_empty() {
            self.stream_title_said = None;
        } else if let Some(title) = title.filter(|t| self.stream_title_said.as_ref() != Some(t)) {
            self.stream_title_said = Some(title.clone());
            self.tell_stream_title(&title);
        }
        let Some(engine) = &self.audio_engine else {
            return;
        };
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
                engine.set_muted(*ch, a.muted[i] || (i == 0 && self.minimized_mute.active));
                engine.set_enabled(*ch, i == 0 || a.enabled[i]);
            }
            self.applied_audio = Some(a.clone());
        }
        self.sync_minimized_mute();
    }

    /// MuteWhenMinimized (audio_update_volume): every sound stops while the
    /// window is minimised, losing the focus alone does not. Called each
    /// frame and when the window's size changes, frames being rare while it
    /// is minimised.
    fn sync_minimized_mute(&mut self) {
        let minimized = self.gfx.as_ref().and_then(|g| g.window.is_minimized());
        if !self.minimized_mute.update(self.settings.audio.mute_when_minimized, minimized) {
            return;
        }
        let mute = self.minimized_mute.active;
        if let Some(engine) = &self.audio_engine {
            engine.set_muted(aurora_audio::Channel::Master, mute || self.settings.audio.muted[0]);
        }
        let volume = self.media_volume();
        self.media.set_channel_volume(volume);
        self.chat_ui.previews.set_volume(volume);
        self.voice.set_transmit(self.talking());
        log::debug!("audio: minimized mute {mute}");
    }

    /// The geometry arena's use noted for the agent's region, and the arena
    /// grown to what the region needs while a loading or teleport screen
    /// (`covered`) hides what that costs (scene/geometry_need.rs).
    fn grow_geometry_ahead(&mut self, renderer: &mut Renderer, covered: bool) {
        let Some(region) = self.world.main_region else {
            return;
        };
        let geometry = &renderer.geometry;
        let (v, i) = geometry.used();
        let used = (v.min(u32::MAX as u64) as u32, i.min(u32::MAX as u64) as u32);
        let capacity = geometry.capacity();
        self.geometry_needs.note(region, used);
        if let Some((vertices, indices)) = self.geometry_needs.wanted(region, used, capacity, covered)
            && renderer.grow_geometry_ahead(vertices, indices)
        {
            log::info!(
                "geometry arena grown ahead behind the loading screen: {} → {} vertices, {} → {} indices",
                capacity.0,
                renderer.geometry.capacity().0,
                capacity.1,
                renderer.geometry.capacity().1
            );
        }
        if self.geometry_needs.unsaved() {
            self.geometry_needs.save(false);
        }
    }

    fn request_exit(&mut self) {
        if self
            .gfx
            .as_ref()
            .is_some_and(|g| g.renderer.stalled_for().is_some() || g.renderer.device_lost())
        {
            // nothing can be drawn while the graphics card holds a frame or
            // once it is lost, the exit prompt neither: the window's close
            // button closes
            log::warn!("exit asked while the graphics card does not answer: closing without the prompt");
            self.gpu_stall.close = true;
            return;
        }
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
        if action != ui::exit::Action::Cancel
            && (!self.close_wearable_editor() || !self.script_editors.prepare_exit() || !self.texture_library.prepare_exit(&self.egui_ctx))
        {
            self.exit_prompt.open = false;
            return;
        }
        if !self.exit_prompt.confirm(action, Instant::now()) {
            return;
        }
        if self.in_world() {
            self.world.save_open_ims();
        }
        if self.in_world() && !self.demo {
            self.record_inventory_logout();
            self.want_scene_capture = true;
            self.release_bridge();
            self.net.send(NetCommand::Logout);
        }
    }

    /// Log out, save the window, layout, inventory cache and settings, exit.
    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        // closed while the graphics card still holds a frame, from its
        // notice or not: reported, and the next start offers the safe
        // graphics (app/gpu_stall.rs)
        if let Some(stalled) = self.gfx.as_ref().and_then(|g| g.renderer.stalled_for()) {
            // the whole stall and where the render thread is now
            crate::crash::gpu_stalled(stalled.as_secs());
            crate::crash::gpu_closed();
        }
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
            self.release_bridge();
            self.net.send(NetCommand::Logout);
            std::thread::sleep(Duration::from_millis(300));
        }
        // the last viewer statistics report, posted before the LogoutRequest
        // (here or by the exit prompt's logout), may still be on its way: the
        // process must not end under it (the Legacy Viewer's finishes while
        // it waits for the LogoutReply). No wait when none is.
        if !self.net.wait_viewer_stats(Duration::from_secs(2)) {
            log::debug!("the last viewer statistics report was still being posted at exit");
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
            // LLAppViewer::disconnectViewer: the next login flies if this
            // session ends flying (world/flying_at_exit.rs)
            crate::world::flying_at_exit::save(
                &crate::settings::account_dir(&self.world.agent_id).join(crate::world::flying_at_exit::FILE),
                self.world.agent.flying,
            );
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
            self.world.system_notice(
                aurora_i18n::tr!("app-destination-not-recognized"),
                crate::world::notice_topics::Topic::Teleport,
            );
            return;
        };
        self.world.system_notice(
            aurora_i18n::tr!(
                "app-teleporting-to",
                value0 = l.region.to_string(),
                value1 = format!("{:.0}", l.pos.x),
                value2 = format!("{:.0}", l.pos.y),
                value3 = format!("{:.0}", l.pos.z)
            ),
            crate::world::notice_topics::Topic::Teleport,
        );
        self.teleport_to_location(l.target(), l.pos);
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
    fn grid_region(&mut self, region: &str) -> Option<String> {
        match crate::grids::resolve(&self.settings, region) {
            Ok(region) => Some(region),
            Err(error) => {
                self.world.system_message(error.text());
                None
            }
        }
    }

    fn teleport_to_location(&mut self, region: String, pos: Vec3) {
        self.teleport_to_location_looking(region, pos, Vec3::X);
    }

    fn teleport_to_location_looking(&mut self, region: String, pos: Vec3, look_at: Vec3) {
        let Some(region) = self.grid_region(&region) else {
            return;
        };
        let here = self.world.region_name();
        let local = region.eq_ignore_ascii_case(&here);
        let command = if local {
            let Some(handle) = self.world.main_region else {
                return;
            };
            NetCommand::TeleportTo {
                handle,
                position: pos,
                look_at,
            }
        } else {
            NetCommand::TeleportToRegion {
                request: uuid::Uuid::new_v4(),
                name: region.clone(),
                position: pos,
                look_at,
            }
        };
        if !self.user_teleport_allowed(&command) {
            return;
        }
        self.begin_teleport(format!("{} ({:.0}, {:.0}, {:.0})", region, pos.x, pos.y, pos.z), !local);
        if local {
            self.send(command);
        } else if self.demo {
            // Only inter-region demo teleports simulate a connection delay.
            aurora_net::transition::begin(
                aurora_net::transition::Kind::Teleport,
                || "demo teleport started (offline: no simulator)".to_string(),
                &self.net.stats,
            );
            self.demo_tp_start = Some(Instant::now());
            self.demo_tp_dest = Some(pos);
        } else {
            self.send(command);
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
        if self.demo_radar_walk {
            let walk = crate::demo::radar_walk(self.world.agent.position, now);
            self.world.apply(walk);
        }
        let mut seen = if self.world.rlva.has("shownearby") {
            Vec::new()
        } else {
            crate::world::radar::sightings(&self.world)
        };
        let endpoint = self.sync_bridge_queries();
        if let Some(command) =
            self.bridge_services
                .radar(&mut seen, self.settings.radar.enhance_by_bridge, endpoint.as_ref(), Instant::now())
        {
            self.send(command);
        }
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
        // the alerts to scripts: ScriptDialogReply from ourselves, a chat on
        // a negative channel that reaches our attachments
        let script_alerts = self.radar.script_alerts(s.enter_scripts, s.leave_scripts);
        if self.world.rlva.has("shownearby") {
            events.clear();
        } else {
            for label in script_alerts {
                self.send(NetCommand::ScriptDialogReply {
                    sender: None,
                    object_id: self.world.agent_id,
                    channel: s.script_channel,
                    index: 1,
                    label,
                });
            }
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
            self.play_ui_sound(event.sound());
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
                // the name is a link to the resident, as in the Legacy Viewer's chat;
                // checked for keywords as nearby chat (FSRadar::radarAlertMsg)
                let name = ui::people::radar_name(&self.world, &id, s.name_format);
                self.world
                    .keyword_alert(id, &name, &text, true, crate::world::attention::Open::NearbyChat);
                self.world.system_message(format!("secondlife:///app/agent/{id}/about {text}"));
            }
        }
    }

    /// Voice session for the current region / parcel, microphone, push-to-talk.
    fn update_voice(&mut self) {
        let tuning = self.panels.settings && self.options_ui.page == ui::options::Page::Sound;
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
        let cam_rot = crate::voice::listener_rotation(self.camera.view());
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
            region_flags: main.and_then(|r| r.info.as_ref()).map(|i| i.region_flags),
            caps: main.map(|r| &*r.caps),
            voice_server: main.map(|r| r.voice_server.as_str()).unwrap_or(""),
            parcel: self.world.parcel.as_deref(),
            input_device: &self.settings.audio.input_device,
            mic_gain: self.settings.audio.mic_gain,
            processing: self.settings.audio.mic_processing(),
            ear: self.settings.audio.voice_ear,
            transmit,
            avatar: (global(self.world.agent.position), self.world.agent.body_rotation()),
            listener: (global(self.camera.position), cam_rot),
            call: self.world.calls.active.as_ref().and_then(|c| c.channel.as_ref()),
        };
        self.voice.update(inputs);
        // the call's channel: its state and participants (the demo has no
        // voice server and simulates them)
        let server_type = main.map(|r| r.voice_server.clone()).unwrap_or_default();
        // nearby voice moderation: who a moderator muted, and us (the demo
        // simulates it, AURORA_DEMO_VOICE_MODERATION)
        let moderated = if self.demo {
            let peers = self.world.calls.voice_peers.iter().copied();
            crate::demo::sim_extras::nearby_moderated(self.frame_count, self.world.agent_id, peers)
        } else {
            self.voice.nearby_moderation()
        };
        self.world.set_nearby_moderated(moderated);
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
        // the land forbids voice: the microphone goes off, without the
        // toggle's sound (LLWebRTCVoiceClient::leaveChannel(true))
        if self.mic_on && self.speak_denied().is_some() {
            self.mic_on = false;
            self.mic_was = false;
        }
        let world = &self.world;
        self.voice.apply_blocks(|id| world.voice_blocked(id));
        self.voice_connected = self.voice.connected;
        self.voice_level = self.voice.own_level;
        self.options_ui.mic_level = self.voice.mic_meter;
    }

    /// End of a shortcut capture (None = cancelled).
    fn finish_capture(&mut self, input: Option<Input>) {
        let mods = self.mods();
        let Some(ui::options::Capture::Binding(action, slot)) = self.options_ui.take_capture() else {
            return;
        };
        if let Some(input) = input {
            let b = crate::keybinds::Binding::captured(input, mods);
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
            Request::ExportLayoutNotecard { name, text } => {
                let profile = crate::build::layout::Profile::read(&text);
                let data = aurora_net::inventory::notecard::Notecard::decode(text.as_bytes()).and_then(|note| note.encode(&text));
                if let Err(reason) = profile.map(|_| ()).and(data.as_ref().map(|_| ()).map_err(Clone::clone)) {
                    self.build.ui.notecards.message = reason;
                } else if let Ok(data) = data {
                    let parent = self
                        .world
                        .inventory
                        .folders
                        .values()
                        .find(|folder| !folder.library && folder.info.type_default == 7)
                        .map_or(self.world.inventory.root, |folder| folder.info.id);
                    if !parent.is_nil() && self.build.ui.notecards.pending_save.is_none() {
                        let request = uuid::Uuid::new_v4();
                        self.build.ui.notecards.pending_save = Some(request);
                        self.build.ui.notecards.message = aurora_i18n::tr!("notecard-saving").into();
                        self.send(NetCommand::CreateNotecardItem {
                            request,
                            parent,
                            name,
                            data,
                            permissions: None,
                        });
                    } else {
                        self.build.ui.notecards.message = aurora_i18n::tr!("session-saving-is-unavailable-in-this-region").into();
                    }
                }
            }
            Request::LoadLayoutNotecard(id) => {
                if self.world.inventory.can_preview(7)
                    && self.build.ui.notecards.pending_load.is_none()
                    && let Some(item) = self
                        .world
                        .inventory
                        .items
                        .get(&id)
                        .filter(|item| {
                            item.asset_type == 7
                                && item.owner_mask & crate::world::inventory::actions::COPY != 0
                                && !self.world.inventory.in_folder_type(id, 14)
                        })
                        .cloned()
                {
                    self.build.ui.notecards.pending_load = Some(id);
                    self.build.ui.notecards.message = aurora_i18n::tr!("build-layout-loading").into();
                    self.send(NetCommand::PreviewInventoryItem(item));
                }
            }
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
            Request::CopyKeys => self
                .egui_ctx
                .copy_text(self.build.copy_keys(&self.world, &self.settings.build, self.shift)),
            Request::Script { object, item } => self.open_script(aurora_net::scripts::ScriptTarget { object, item }),
        }
        self.flush_build();
    }

    /// Send what the build tools queued.
    fn flush_build(&mut self) {
        for c in std::mem::take(&mut self.build.out) {
            if let NetCommand::Build(
                aurora_net::build::BuildCmd::UpdateTaskInventory { item, .. } | aurora_net::build::BuildCmd::RezScript { item, .. },
            ) = &c
            {
                self.record_library_use(item.item_id);
            }
            self.send(c);
        }
    }

    fn demo_action_steps(&mut self) {
        // Exercise the real depth pick after the renderer is back in self.gfx.
        if !self.demo {
            return;
        }
        // AURORA_DEMO_BUILD_ECHO: transform reports a round trip late
        for ev in self.build_demo.arrived(Instant::now()) {
            if let Some(e) = self.world.apply(ev) {
                self.on_app_event(e);
            }
        }
        self.demo_script_steps();
        self.world_click_demo_steps();
        self.object_menu_demo_steps();
        if self.frame_count == 400
            && let Ok(mode) = std::env::var("AURORA_DEMO_BEACONS")
        {
            if let Some(handle) = self.world.main_region {
                let local_ids = self
                    .world
                    .objects
                    .iter()
                    .filter(|(_, o)| o.key.region == handle && !o.is_avatar())
                    .map(|(_, o)| o.key.local_id)
                    .collect();
                self.world.apply(NetEvent::ObjectsKilled { handle, local_ids });
            }
            self.world.apply(crate::demo::beacons::events());
            if mode == "parcels" {
                for event in crate::demo::beacons::overlay_events() {
                    self.world.apply(event);
                }
            }
            let (eye, target) = crate::demo::beacons::view(&mode);
            self.camera.set_view(eye, target, &mut self.world.agent, &self.settings.camera);
        }
        // AURORA_DEMO_MATERIAL_ARRIVALS: a material nobody uses arrives
        if let Some((id, m)) = crate::demo::material_arrival_due() {
            self.scene.legacy_mats.insert(id, m);
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
        let probe = mode.starts_with("probe-");
        if mode.ends_with("-ready") && !mode.starts_with("blend-") && !mode.starts_with("mask-") && !probe {
            return;
        }
        if let Some(copy) = mode.strip_prefix("open-") {
            self.demo_contents_copy(copy);
        }
        let id = crate::demo::action_mode_id(&mode);
        let click_frame = if mode.starts_with("linked-") { 724 } else { 284 };
        if self.frame_count == 260
            && (id == 0 || id == 980 || id == 981 || mode.starts_with("blend-") || mode.starts_with("mask-") || probe)
            && let Some(idx) = self
                .world
                .objects
                .index_of_uuid(&crate::demo::action_id(if probe { 975 } else { 971 }))
            && let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, Instant::now(), 0)
        {
            let direction = (self.camera.position - pos).normalize_or(-Vec3::X);
            let rotation = glam::Quat::from_rotation_arc(Vec3::X, direction);
            self.world.apply(if probe {
                crate::demo::click_probe(&mode, pos)
            } else if mode.starts_with("blend-") || mode.starts_with("mask-") {
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
            if mode.starts_with("pay-menu") || mode.starts_with("pay-linked-menu") {
                self.on_ctx_action(ui::context::CtxAction::Pay(crate::demo::action_id(id)));
                log::info!("demo menu payment: prim={id}, dialog={}", self.interactions.dialog.is_some());
            } else if mode.ends_with("-build") {
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
        if probe && self.frame_count == 740 {
            log::info!("demo probe click: mode={mode}, held={}", self.interactions.held.is_some());
        }
        if mode.ends_with("-confirm")
            && self.frame_count == 320
            && let Some(cmd) = self
                .interactions
                .confirm(&self.world, (id == 972 || mode.starts_with("pay-linked-menu")).then_some(10))
        {
            if let NetCommand::PayObject { object, .. } = &cmd {
                log::info!(
                    "demo payment confirmed: clicked={id}, object={object}, dialog={}",
                    self.interactions.dialog.is_some()
                );
            }
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
            .huds_shown()
            .then_some(self.scene.lists.hud_view)
            .flatten()
            .map(|view| {
                self.scene.edit_ray_hits(
                    &self.world,
                    view.ray(self.cursor_pos.0, self.cursor_pos.1),
                    true,
                    self.settings.build.select_probes,
                    self.settings.build.select_limit(),
                )
            })
            .unwrap_or_default();
        if hits.is_empty()
            && let Some(ray) = self
                .gfx
                .as_ref()
                .and_then(|g| g.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1))
        {
            hits = self.scene.edit_ray_hits(
                &self.world,
                ray,
                false,
                self.settings.build.select_probes,
                self.settings.build.select_limit(),
            );
        }
        self.build.cycle_under_cursor(&mut self.world, &self.settings.build, &hits, steps);
        self.flush_build();
        true
    }

    /// Left press while building: handles, selection, create, terraform.
    /// Run by `click`; false also when it stopped to ask for the depth under
    /// the cursor (`ClickWait::waiting`), to run again when it has come.
    fn build_mouse_down(&mut self) -> bool {
        if self.ik_editor.open && !self.alt {
            if self.ik_editor.picking {
                let Some(picked) = self.click_pick() else {
                    return true;
                };
                let (_, target) = self.scene.selection_pick(
                    &self.world,
                    picked.ray,
                    picked.depth,
                    true,
                    self.settings.build.select_probes,
                    self.settings.draw_distance,
                );
                if let Some(target) = target {
                    self.ik_editor.picked_prim(&self.world, &self.build, target);
                }
                return true;
            }
            if !self.build.open {
                // A worn HUD takes the press, unless a poser handle drawn over
                // it does: the Legacy Viewer's poser window leaves HUDs to the
                // pie tool, and only its visual manipulators take scene clicks
                // (FSFloaterPoser::enableVisualManipulators, FSToolCompPose).
                // World presses stay the editor's.
                return !(self.ik_editor.posing()
                    && !self.ik_editor.poser_owns_pointer()
                    && self.huds_shown()
                    && self.scene.hud_pick(&self.world, self.cursor_pos, false).is_some());
            }
        }
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
        if self.particle_editor.picking {
            self.left_down = true;
            self.particle_target_pick();
            return true;
        }
        if !self.build.open
            || self.alt
            || (self.hud_drag_requested() && self.huds_shown() && self.scene.hud_move_pick(&self.world, self.cursor_pos).is_some())
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
        } else if self.build.tool == crate::build::Tool::Create && !self.settings.build.copy_selection && !(self.shift || self.ctrl) {
            let ray = self.build.cam.ray(x, y);
            let mut candidates = std::mem::take(&mut self.scene.creation_preview.candidates);
            let surface = self
                .scene
                .creation_surface(&self.world, ray, self.settings.draw_distance, &mut candidates);
            self.scene.creation_preview.candidates = candidates;
            (surface.map(|s| s.0), surface.and_then(|s| s.1), ray)
        } else if self.build.tool == crate::build::Tool::Edit && self.build.edit_mode == crate::build::EditMode::Face {
            // LLToolFace picks the current cursor ray. A delayed GPU depth
            // sample can still point at the previous object during Shift clicks.
            let ray = self.build.cam.ray(x, y);
            let target = self
                .scene
                .edit_ray_hits(
                    &self.world,
                    ray,
                    false,
                    self.settings.build.select_probes,
                    self.settings.build.select_limit(),
                )
                .first()
                .copied();
            (None, target, ray)
        } else {
            let Some(picked) = self.click_pick() else {
                return false;
            };
            let (hit, target) = if matches!(self.build.tool, crate::build::Tool::Edit | crate::build::Tool::Grab) {
                self.scene.selection_pick(
                    &self.world,
                    picked.ray,
                    picked.depth,
                    true,
                    self.settings.build.select_probes,
                    self.settings.draw_distance,
                )
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
        let hit = picked.visible_point();
        let ray = picked.ray.unwrap_or_else(|| self.build.cam.ray(x, y));
        let now = Instant::now();
        if let Some(p) = hit {
            let idx = match picked.drawn.blend.or(picked.drawn.opaque) {
                Some(d) => self.scene.record_object(d.record).map(|(idx, _)| idx),
                None => self.scene.pick_at(&self.world, p, now),
            };
            let object = idx.and_then(|i| crate::camera::pick_focus_object(&self.world, i, now));
            self.camera.alt_focus(p, object, ray, &mut self.world.agent, &self.settings.camera);
        }
        self.camera.begin_drag(false);
        self.mouse_mode = MouseMode::FocusOrbit;
        // LLToolCamera::handleMouseUp: a click on the sky focused nothing,
        // the cursor comes back where it was pressed
        if hit.is_some() {
            self.hold_focus_cursor();
        } else {
            self.set_mouselook_grab(true);
        }
        self.left_down = true;
        true
    }

    /// Right button: orbit drag, context menu on a click, and walking forward
    /// while steering with the left button (a double click runs).
    fn on_right_button(&mut self, pressed: bool, over_ui: bool) {
        if self.hud_drag.is_some() {
            return;
        }
        if self.mouselook_right_button(pressed, over_ui) {
            return;
        }
        let was_drag = self.right_drag;
        self.right_drag = pressed && (!over_ui || self.mouse_mode == MouseMode::Steer) && self.in_world();
        if pressed {
            self.right_gesture = RightGesture::default();
            self.start_world_mouse_walk();
        }
        if self.mouse_walks() {
            self.right_gesture.walked = true;
            self.walk_tap(0, true);
        } else if !pressed {
            self.check_temp_run();
        }
        if !pressed && was_drag && self.right_gesture.opens_menu() && self.mouse_mode == MouseMode::None {
            // a click (no drag): context menu
            self.open_context_menu();
        }
    }

    /// The right button in mouselook (LLToolCompGun::handleRightMouseDown /
    /// Up): held, it zooms; Alt + right click goes on to the context menu
    /// only with FSEnableRightclickMenuInMouselook. True when it is handled.
    fn mouselook_right_button(&mut self, pressed: bool, over_ui: bool) -> bool {
        if pressed {
            if !self.camera.mouselook() || over_ui || !self.in_world() {
                return false;
            }
            if !self.alt {
                let aspect = self.view_aspect();
                self.camera.press_mouselook_zoom(&self.settings.camera, aspect);
                return true;
            }
            return !self.settings.camera.mouselook_alt_right_menu;
        }
        let handled = self.end_mouselook_zoom() || (self.camera.mouselook() && !self.settings.camera.mouselook_alt_right_menu);
        if handled {
            // a press from before mouselook ends here too, with no menu
            self.right_drag = false;
        }
        handled
    }

    /// The held mouselook zoom ends: its angle is remembered (saved with the
    /// settings when it changed). True when it was held.
    fn end_mouselook_zoom(&mut self) -> bool {
        let before = self.settings.camera.mouselook_zoom_fov;
        if !self.camera.release_mouselook_zoom(&mut self.settings.camera) {
            return false;
        }
        if self.settings.camera.mouselook_zoom_fov != before && !self.demo {
            self.settings.save();
        }
        true
    }

    /// Wheel clicks on the world (SL sign: positive zooms out): the held
    /// mouselook zoom's angle, or the camera's own use of the wheel.
    fn camera_wheel(&mut self, clicks: f32) {
        let aspect = self.view_aspect();
        if self.camera.scroll_mouselook_zoom(clicks, aspect) {
            return;
        }
        let was = self.camera.mouselook();
        self.camera
            .scroll(clicks, self.ctrl, self.shift, &mut self.world.agent, &mut self.settings.camera);
        if was != self.camera.mouselook() {
            self.set_mouselook_grab(!was);
        }
    }

    /// Width over height of the world view (the camera's limits on its
    /// viewing angle depend on it).
    fn view_aspect(&self) -> f32 {
        self.gfx.as_ref().map_or(16.0 / 9.0, |g| {
            let (w, h) = g.renderer.size();
            w as f32 / h.max(1) as f32
        })
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
        self.script_click_up();
        self.left_down = false;
        self.world_mouse_walk_pending = false;
        self.build.mouse_up(&mut self.world, &self.settings.build);
        self.flush_build();
        self.media.on_release(aurora_media::Modifiers {
            control: self.ctrl,
            alt: self.alt,
            shift: self.shift,
            command: self.command,
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

    /// RlvActions::canTeleport*: reject before UI and demo side effects.
    fn user_teleport_allowed(&mut self, command: &NetCommand) -> bool {
        self.world.rlva_configure(self.settings.rlva_enabled);
        self.world.rlva.preferences.clone_from(&self.settings.rlva);
        if self.world.rlva_allows_command(command, uuid::Uuid::nil()) {
            true
        } else {
            self.world
                .system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
            false
        }
    }

    /// Preflight before UI/simulation side effects, including offline demos.
    fn request_user_teleport(&mut self, command: NetCommand, dest: String, show_progress: bool) -> bool {
        if self.user_teleport_allowed(&command) {
            self.begin_teleport(dest, show_progress);
            self.send(command);
            true
        } else {
            false
        }
    }

    fn request_friend_join_teleport(&mut self, command: NetCommand, avatar: uuid::Uuid, fly_on_arrival: bool, fresh_physics: bool) -> bool {
        let NetCommand::TeleportTo { handle, .. } = &command else {
            return false;
        };
        let handle = *handle;
        let remote = Some(handle) != self.world.main_region;
        let dest = self.world.map.track.as_ref().map(|t| t.label.clone()).unwrap_or_default();
        if self.user_teleport_allowed(&command) {
            self.begin_teleport(dest, remote);
            if remote {
                self.world
                    .defer_friend_join_flight_at(avatar, fly_on_arrival, fresh_physics, handle);
            } else {
                self.world.defer_friend_join_flight(avatar, fly_on_arrival, fresh_physics);
            }
            self.send(command);
            true
        } else {
            false
        }
    }

    fn start_friend_join_flight(&mut self) {
        if self.world.take_friend_join_flight() {
            if !self.world.agent.flying {
                self.toggle_fly();
            }
            let mut controls = *self.net.controls.lock();
            controls.control_flags |= control::FLY;
            self.net.set_controls(controls);
            log::info!(
                "friend join: flight enabled after local arrival at {:.1?}",
                self.world.agent.position
            );
        }
    }

    /// A named lookup or maturity response can arrive after a new lock.
    /// Finish the matching user's progress/history through the normal failure
    /// path when that delayed action is refused; scripts retain their actor.
    fn send_deferred_teleport(&mut self, command: NetCommand, actor: uuid::Uuid) {
        if actor.is_nil() && !self.user_teleport_allowed(&command) {
            self.world.apply(NetEvent::TeleportFailed {
                reason: aurora_i18n::tr!("rlva-blocked").into(),
            });
        } else {
            self.send_from(command, actor);
        }
    }

    fn begin_teleport(&mut self, dest: String, show_progress: bool) {
        self.portal_journey = None;
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

    /// Teleport to `dest`, a place of the navigation history the arrows or
    /// their list chose (LLTeleportHistory::goToItem).
    fn teleport_history(&mut self, dest: Option<crate::world::tphistory::TpEntry>) {
        if let Some(e) = dest {
            self.world.system_notice(
                aurora_i18n::tr!("app-teleporting-to-2", value0 = e.region.to_string()),
                crate::world::notice_topics::Topic::Teleport,
            );
            if !self.request_user_teleport(
                NetCommand::TeleportTo {
                    handle: e.handle,
                    position: e.position,
                    look_at: Vec3::X,
                },
                e.region.clone(),
                Some(e.handle) != self.world.main_region,
            ) {
                self.world.tp_history.failed();
            }
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

    /// Alt+Shift+S: the menu's "Stand Up" while sitting (enable_standup_self),
    /// else its "Sit Down", which stays disabled while flying
    /// (enable_sitdown_self, indra/newview/llviewermenu.cpp).
    fn sit_stand_shortcut(&mut self) {
        if self.world.agent.is_sitting() || !self.world.agent.flying {
            self.toggle_ground_sit();
        }
    }

    /// "Always run" (Ctrl+R, the Controls window's Walk and Run).
    fn set_always_run(&mut self, on: bool) {
        if on && self.world.rlva.has("alwaysrun") {
            return;
        }
        if on == self.always_run {
            return;
        }
        self.always_run = on;
        self.send(NetCommand::SetAlwaysRun(
            self.always_run || self.temp_run.is_some() || self.joystick.running,
        ));
        self.world.system_message(if self.always_run {
            aurora_i18n::tr!("app-always-run-enabled")
        } else {
            aurora_i18n::tr!("app-always-run-disabled")
        });
    }

    /// Preferences › Camera › "Reset all", and the Controls window's reset:
    /// every camera setting back to its default, the view behind the avatar.
    fn reset_camera_all(&mut self) {
        let before = self.settings.camera.clone();
        self.settings.camera = crate::camera::CameraSettings::default();
        self.world.rlva.preserve_camera_settings(&mut self.settings.camera, &before);
        self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
    }

    /// LLAgent::toggleFlying → setFlying (indra/newview/llagent.cpp): only
    /// the FLY flag changes. No STOP with it: that flag brakes the avatar,
    /// and the fall must keep the flight's momentum, as in Firestorm.
    fn toggle_fly(&mut self) {
        if !self.world.agent.flying && self.world.rlva.has("fly") {
            return;
        }
        self.world.cancel_friend_join_flight();
        self.stop_autopilot(true);
        self.release_native_movelock();
        self.movement_runtime.cancel_water_exit();
        self.world.agent.flying = !self.world.agent.flying;
    }

    /// A conversation the user opened (a contact, a name's menu, People)
    /// comes up with its field taking the keyboard, ready to type in, as
    /// Firestorm's floater shown with the focus (LLFloaterReg::showInstance,
    /// LLFloater::setFocus → focusFirstItem). Openings that come from calls
    /// set `focus_im` alone and leave the keyboard where it is.
    pub(super) fn open_conversation_to_type(&mut self, id: uuid::Uuid) {
        if !self.world.rlva_start_ims(&[id]) {
            return;
        }
        // Create only after checking startim: pre-creating the session would
        // incorrectly qualify every target for the open-session exemption.
        if !self.world.is_group_session(&id) && !self.world.groups.is_member(&id) {
            self.world.social.session_mut(id);
            self.world.met(id);
            self.world.ui_sounds.push(UiSound::StartIm);
        }
        self.world.social.focus_im = Some(id);
        self.chat_ui.conv_focus_request = true;
        self.show_conversations();
    }

    /// The Conversations window for a conversation the user asked for (Send
    /// IM, Group chat, Conference, Call from a menu, a profile, People or
    /// Contacts): opened if closed, expanded if minimized and brought to the
    /// front (#704), as LLFloater::openFloater does to the container hosting
    /// the conversation (setMinimized(false), setVisibleAndFrontmost).
    pub(super) fn show_conversations(&mut self) {
        ui::notifications::open_conversations(&self.egui_ctx, &mut self.panels.chat);
    }

    /// The Snooze of a group chat (FSFloaterIM::onSnooze, confirmSnooze):
    /// asks its duration first when the settings say so
    /// (FSEnablePerGroupSnoozeDuration), else snoozes it for the setting's
    /// (GroupSnoozeTime; `AURORA_DEMO_SNOOZE` in demo).
    /// (Fields rather than `self`: it runs while the frame's UI borrows it.)
    fn ask_or_snooze(
        settings: &crate::settings::Settings,
        demo: bool,
        world: &mut crate::world::World,
        chat_ui: &mut ui::chat::ChatUi,
        group: uuid::Uuid,
    ) {
        let seconds = demo
            .then(crate::demo::snooze_seconds)
            .flatten()
            .unwrap_or(settings.group_snooze_minutes.saturating_mul(60));
        if settings.group_snooze_ask || (demo && crate::demo::snooze_ask()) {
            // the dialog counts whole minutes, like the setting: a demo's
            // seconds are rounded up
            chat_ui.ask_snooze(group, seconds.div_ceil(60));
        } else {
            Self::snooze_group_chat(world, chat_ui, group, seconds);
        }
    }

    /// Snooze a group chat for `seconds` (FSFloaterIM::snooze): its
    /// conversation closes without leaving the session, and its call ends as
    /// when it is closed.
    fn snooze_group_chat(world: &mut crate::world::World, chat_ui: &mut ui::chat::ChatUi, group: uuid::Uuid, seconds: u32) {
        if !world.snooze_group_chat(group, seconds) {
            return;
        }
        world.conversation_closed(&group);
        chat_ui.forget(group);
        if chat_ui.selected == Some(group) {
            chat_ui.selected = None;
            chat_ui.show_profile = false;
        }
    }

    /// A letter key started the nearby chat: in the Conversations window
    /// when it shows the nearby chat, in the chat bar otherwise, with the
    /// character typed in (LLViewerWindow::handleKey; Firestorm's
    /// FSLetterKeysFocusNearbyChatBar, on by default).
    fn start_chat_with(&mut self, text: &str) {
        if self.panels.chat && ui::chat::local_chat_shown(&self.egui_ctx, &self.chat_ui) {
            self.chat_ui.draft_mut(None).push_str(text);
            self.chat_ui.conv_focus_request = true;
        } else {
            self.chat_ui.input.push_str(text);
            self.chat_ui.focus_request = true;
        }
    }

    /// The chat bar has the keyboard and hands its movement keys to the
    /// avatar: empty, or "Arrow keys always move me while in chat".
    fn chat_bar_moves_avatar(&self) -> bool {
        self.in_world() && self.chat_ui.has_focus && (self.chat_ui.input.is_empty() || self.settings.arrow_keys_always_move)
    }

    /// This key, pressed now, goes to the avatar rather than to the chat
    /// bar's text (crate::keybinds::chat_bar_moves).
    fn chat_bar_passes_key(&self, code: KeyCode) -> bool {
        self.in_world()
            && self.chat_ui.has_focus
            && crate::keybinds::chat_bar_moves(code, self.ctrl, self.chat_ui.input.is_empty(), self.settings.arrow_keys_always_move)
    }

    /// A Windows Alt code typed in a text field (alt_codes.rs): true for one
    /// of its digits, which neither the field nor the viewer's shortcuts
    /// see. Left Alt's release hands the character to egui's next frame.
    /// Outside a text field, Alt and the numpad keep their shortcuts.
    fn alt_code_key(&mut self, event: &WindowEvent) -> bool {
        let WindowEvent::KeyboardInput { event: ke, .. } = event else {
            return false;
        };
        let PhysicalKey::Code(code) = ke.physical_key else {
            return false;
        };
        let pressed = ke.state == ElementState::Pressed;
        if code == KeyCode::AltLeft {
            if pressed {
                if !ke.repeat {
                    self.alt_codes.alt_pressed();
                }
            } else if let Some(text) = self.alt_codes.alt_released(ke.text.as_deref())
                && self.egui_ctx.egui_wants_keyboard_input()
            {
                self.alt_text.push(text);
            }
            return false;
        }
        if !pressed || !self.alt_codes.active() || !self.egui_ctx.egui_wants_keyboard_input() {
            return false;
        }
        self.alt_codes.key_pressed(code, ke.repeat)
    }

    fn update_move_input(&mut self, typing: bool, dt: f32) {
        self.poll_joystick(dt);
        if self.joystick_flycam_step(dt) {
            return;
        }
        let web_or_popup = self.media.focus.is_some()
            || self.profile_ui.focused_web().is_some()
            || self.search_ui.focused_web().is_some()
            || egui::Popup::is_any_open(&self.egui_ctx);
        let typing = typing || web_or_popup;
        let (kb, m) = (&self.settings.keybinds, self.mods());
        // typing in the chat bar, only the keys it hands to the avatar move
        // it (crate::keybinds::chat_bar_moves)
        let chat_moves = typing && !web_or_popup && self.chat_bar_moves_avatar();
        let passed: HashSet<Input>;
        let down = if chat_moves {
            passed = self
                .down
                .iter()
                .filter(|i| crate::keybinds::chat_bar_held_moves(i, self.ctrl))
                .cloned()
                .collect();
            &passed
        } else {
            &self.down
        };
        let k = |a: Action| kb.held(a, down, m);
        let mut i = MoveInput::default();
        if !typing || chat_moves {
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
            let passing = self
                .settings
                .movements
                .get(&self.world.agent_id)
                .is_some_and(|prefs| prefs.pass_wall)
                && self
                    .down
                    .iter()
                    .any(|input| self.settings.keybinds.pass_wall_triggered(input, self.mods()));
            i.up = k(Action::Up) && !passing;
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
        let joystick_up = self.camera.free().is_none()
            && self.settings.joystick.avatar_enabled
            && (self.joystick.motion.axes[2] > 0.0 || self.joystick.motion.jump || self.joystick.motion.rise);
        i.down = self.crouch_toggle.apply(
            self.settings.crouch_toggle,
            self.world.agent.flying,
            self.world.agent.is_sitting(),
            i.down,
            i.up || joystick_up,
        );
        self.joystick_avatar(&mut i);
        if self.camera.free().is_some() {
            // the photo's free camera takes the movement keys: the avatar
            // stays where it is (no walk, turn, jump or flight start).
            // The viewfinder pad must be applied here: this path returns
            // before `viewfinder_pad`, so Look / Glide / Forward were ignored.
            self.apply_viewfinder_free_pad();
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
            // Mouse steering (LLToolCamera::mMouseSteering): the mouse turns
            // the avatar itself (cameraOrbitAround), forward and back keep
            // their meaning and the turn keys slide instead (agent_turn_left /
            // agent_turn_right, indra/newview/llviewerinput.cpp). No turn
            // toward the keys: backing up must not spin the avatar (#1259).
            i.strafe_left |= i.turn_left;
            i.strafe_right |= i.turn_right;
            i.turn_left = false;
            i.turn_right = false;
            // + right button = walk forward (LLToolCamera::handleRightMouseDown)
            i.forward |= self.right_drag;
        }
        i.run = self.temp_run.is_some() || self.joystick.running;
        if self.world.rlva.has("temprun") {
            i.run = false;
        }
        if self.world.rlva.has("jump") && !self.world.agent.flying {
            i.up = false;
        }
        // nudge phase: the first 0.25 s of a press makes small steps
        let now = Instant::now();
        let since = |s: &mut Option<Instant>, held: bool| -> bool {
            if !held {
                *s = None;
                return false;
            }
            s.get_or_insert(now).elapsed().as_secs_f32() < NUDGE_TIME
        };
        i.nudge_fb = since(&mut self.fb_since, i.forward || i.back) && !self.mouse_walks();
        i.nudge_lr = since(&mut self.lr_since, i.strafe_left || i.strafe_right);
        // Native joystick translation uses moveAt/moveLeft, without the
        // keyboard's initial nudge (LLViewerJoystick::agentPush/Slide).
        i.nudge_fb &= self.joystick.motion.axes[0] == 0.0;
        i.nudge_lr &= self.joystick.motion.axes[1] == 0.0;
        if i.up {
            self.world.agent.record_jump_input(now);
        }
        if automatic_fly_held(&mut self.up_since, i.up, self.settings.automatic_fly, now)
            && self.movement_runtime.native_fly_allowed()
            && !self.world.rlva.has("fly")
            && !self.world.agent.flying
            && !self.world.agent.seated
            && self.in_world()
        {
            self.world.agent.flying = true;
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
        let moving = i.resets_camera(self.crouch_toggle.latched());
        if moving && self.mouse_mode != MouseMode::FocusOrbit {
            // A stick held to look up/down keeps its aim while walking,
            // like held mouse steering, instead of fighting the pitch fade.
            let steering = self.mouse_mode == MouseMode::Steer || i.joystick_pitch != 0.0;
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
        // the photo viewfinder's camera pad, held like the keys
        if let Some(pad) = self.viewfinder_pad(dt) {
            for (r, p) in rate.iter_mut().zip(pad) {
                *r = r.max(p);
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
        self.camera.joystick_pitch(i.joystick_pitch, dt);
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
                self.open_conversation_to_type(id);
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
                self.open_conversation_to_type(id);
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
        self.ask_app_link_names();
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
        wanted.extend(self.contacts_people_ui.wanted_pics.drain());
        wanted.extend(self.notif_ui.wanted_pics.drain());
        wanted.extend(self.land_ui.wanted_pics.drain());
        wanted.extend(self.region_ui.wanted_pics.drain());
        wanted.extend(self.money_tracker_ui.wanted_pics.drain());
        wanted.extend(self.pay_ui.wanted_pics.drain());
        wanted.extend(self.inventory_ui.wanted_pics.drain());
        self.refine_ui_copies();
        for agent in wanted {
            self.world.want_profile(agent);
            let Some(image) = self.world.profiles.get(&agent).map(|p| p.sl_image) else {
                continue;
            };
            // the picture its profile names now (saved by us, or the profile
            // read again: LLAvatarIconIDCache follows each reply)
            if self.avatar_pic_sources.get(&agent) == Some(&image) {
                continue;
            }
            if image.is_nil() {
                // removed: the default picture
                self.avatar_pics.remove(&agent);
                self.avatar_pic_sources.remove(&agent);
                continue;
            }
            // two residents with the same picture share its texture (its
            // pixels are delivered once)
            if let Some(t) = self.avatar_pic_images.get(&image) {
                self.avatar_pics.insert(agent, t.clone());
                self.avatar_pic_sources.insert(agent, image);
                continue;
            }
            // (a changed picture shows the old one until the new is decoded)
            self.scene.textures.want_ui_image(renderer, image, &self.scene.jobs);
            if let Some((w, h, rgba)) = self.scene.textures.take_ui_image(&image)
                && rgba.len() == (w * h * 4) as usize
            {
                let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                let t = self.egui_ctx.load_texture(format!("pic-{agent}"), ci, egui::TextureOptions::LINEAR);
                self.avatar_pic_images.insert(image, t.clone());
                self.avatar_pics.insert(agent, t);
                self.avatar_pic_sources.insert(agent, image);
                self.picture_pixels(image, w, h, rgba);
            }
        }
        self.follow_login_portrait(renderer);
        let mut images: Vec<uuid::Uuid> = self.profile_ui.wanted_images.drain().collect();
        images.extend(self.avatar_textures.wanted_images.drain());
        images.extend(self.place_images.drain());
        images.extend(self.land_ui.wanted_images.drain());
        images.extend(self.region_ui.wanted_images.drain());
        images.extend(self.buy_land_ui.wanted_images.drain());
        images.extend(self.contacts_ui.wanted_images.drain());
        images.extend(self.chat_ui.wanted_images.drain());
        images.extend(self.notif_ui.wanted_images.drain());
        images.extend(self.pay_ui.wanted_images.drain());
        images.extend(self.people_ui.wanted_images.drain());
        images.extend(self.contacts_people_ui.wanted_images.drain());
        images.extend(self.build.ui.wanted_images.drain());
        images.extend(self.appearance_ui.wanted_images.drain());
        images.extend(self.inventory_ui.wanted_images.drain());
        images.extend(self.texture_library.wanted_images.drain());
        images.extend(self.worldmap_ui.wanted_images.drain());
        images.extend(self.env_editor.wanted_images.drain());
        images.extend(self.particle_editor.wanted_images.drain());
        images.extend(self.particle_editor.picker.wanted_images.drain());
        images.extend(self.wearable_ui.wanted_images.drain());
        images.extend(self.day_editor.wanted_images.drain());
        images.extend(self.favorite_images.drain());
        let overlays = self.world.rlva_overlay_frames(Instant::now());
        self.rlva_overlay_slots.retain(|id, _| {
            let keep = overlays.iter().any(|frame| frame.texture == *id);
            if !keep {
                self.scene.textures.release(id);
            }
            keep
        });
        for frame in &overlays {
            self.rlva_overlay_slots.entry(frame.texture).or_insert_with(|| {
                self.scene
                    .textures
                    .acquire(renderer, frame.texture, crate::scene::textures::TexSource::Asset)
            });
        }
        images.extend(
            self.world
                .rlva_overlay_frames(Instant::now())
                .into_iter()
                .map(|frame| frame.texture),
        );
        for note in self.task_notecards.values_mut() {
            images.extend(note.state.wanted_images.drain());
        }
        for image in images {
            if self.ui_images.contains_key(&image) {
                continue;
            }
            self.scene.textures.want_ui_image(renderer, image, &self.scene.jobs);
            if let Some((w, h, rgba)) = self.scene.textures.take_ui_image(&image)
                && rgba.len() == (w * h * 4) as usize
            {
                let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                if let Some(mask) = crate::world::rlva::effects::AlphaMask::from_rgba(w, h, &rgba) {
                    self.ui_image_masks.insert(image, mask);
                }
                let t = self.egui_ctx.load_texture(format!("img-{image}"), ci, egui::TextureOptions::LINEAR);
                self.ui_images.insert(image, t);
                self.note_ui_copy(image, w);
            }
        }
        self.update_inventory_hover_images(renderer);
    }

    /// What a profile picture's pixels give besides its texture: the Aurora
    /// profile's banner, our own login portrait, and whether a finer copy
    /// is still to come.
    fn picture_pixels(&mut self, image: uuid::Uuid, w: u32, h: u32, rgba: Vec<u8>) {
        // the Aurora profile's banner: a few pixels the filter blurs
        let banner = ui::profile::banner_image(w as usize, h as usize, &rgba);
        let banner = self
            .egui_ctx
            .load_texture(format!("banner-{image}"), banner, egui::TextureOptions::LINEAR);
        self.profile_ui.banners.insert(image, banner);
        self.note_ui_copy(image, w);
        self.save_login_portrait(image, w, h, rgba);
    }

    /// Our login portrait follows our profile picture, changed here or
    /// elsewhere: the texture the login screen shows after a logout, and
    /// the file it reads at the next start.
    fn follow_login_portrait(&mut self, renderer: &mut Renderer) {
        let agent = self.world.agent_id;
        if self.demo || agent.is_nil() || self.world.profiles.awaiting(&agent) {
            return;
        }
        let Some(image) = self.world.profiles.get(&agent).map(|p| p.sl_image) else {
            return;
        };
        if image.is_nil() {
            // no picture any more: the login screen's default one
            if self.login_portrait_source != Some((agent, image)) {
                self.login_portrait_source = Some((agent, image));
                self.login_form.portraits.remove(&agent);
                let path = crate::settings::account_dir(&agent).join("login_portrait.png");
                rayon::spawn(move || {
                    if let Err(e) = std::fs::remove_file(&path)
                        && e.kind() != std::io::ErrorKind::NotFound
                    {
                        log::warn!("login portrait: {e}");
                    }
                });
            }
            return;
        }
        // the texture of our pictures, once decoded (we always want ours)
        let Some(texture) = self.avatar_pic_images.get(&image) else {
            return;
        };
        if self.login_form.portraits.get(&agent).map(|t| t.id()) != Some(texture.id()) {
            self.login_form.portraits.insert(agent, texture.clone());
        }
        // a picture shown before (ours earlier, someone else's) has no
        // pixels left for the file: decoded again
        if self.login_portrait_source != Some((agent, image)) {
            self.scene.textures.want_ui_image(renderer, image, &self.scene.jobs);
            if let Some((w, h, rgba)) = self.scene.textures.take_ui_image(&image)
                && rgba.len() == (w * h * 4) as usize
            {
                self.save_login_portrait(image, w, h, rgba);
            }
        }
    }

    /// The file of our login portrait, from our profile picture's pixels.
    fn save_login_portrait(&mut self, image: uuid::Uuid, w: u32, h: u32, rgba: Vec<u8>) {
        let agent = self.world.agent_id;
        if self.demo || !self.world.profiles.get(&agent).is_some_and(|p| p.sl_image == image) {
            return;
        }
        self.login_portrait_source = Some((agent, image));
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

    /// An interface image taken `width` px wide: followed while coarser than
    /// its texture's UI size (the first levels of a progressive download).
    fn note_ui_copy(&mut self, image: uuid::Uuid, width: u32) {
        if self.scene.textures.ui_image_width(&image).is_some_and(|full| width < full) {
            let since = self.ui_coarse.get(&image).map_or_else(Instant::now, |c| c.1);
            self.ui_coarse.insert(image, (width, since));
        } else {
            self.ui_coarse.remove(&image);
        }
    }

    /// Interface images first shown from the coarse levels of a progressive
    /// download stayed blurred: taken once, their texture was no longer
    /// asked for. It keeps loading toward its UI size, and each finer copy
    /// replaces the pixels of the egui texture in place (every clone of the
    /// handle follows: name plates, lists, profiles).
    fn refine_ui_copies(&mut self) {
        // (a texture that never gets finer is given up after that)
        const FOLLOWED: Duration = Duration::from_secs(300);
        // (an image dropped meanwhile, a texture refreshed: its next copy is
        // the one its loader takes)
        let (pics, images) = (&self.avatar_pic_images, &self.ui_images);
        self.ui_coarse.retain(|id, _| pics.contains_key(id) || images.contains_key(id));
        let coarse: Vec<(uuid::Uuid, u32)> = self.ui_coarse.iter().map(|(id, (w, _))| (*id, *w)).collect();
        for (image, have) in coarse {
            match self.scene.textures.take_ui_image(&image) {
                Some((w, h, rgba)) if w > have && rgba.len() == (w * h * 4) as usize => {
                    let ci = || egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                    if let Some(t) = self.ui_images.get_mut(&image) {
                        t.set(ci(), egui::TextureOptions::LINEAR);
                        if let Some(mask) = crate::world::rlva::effects::AlphaMask::from_rgba(w, h, &rgba) {
                            self.ui_image_masks.insert(image, mask);
                        }
                    }
                    if let Some(t) = self.avatar_pic_images.get_mut(&image) {
                        t.set(ci(), egui::TextureOptions::LINEAR);
                        self.picture_pixels(image, w, h, rgba);
                    } else {
                        self.note_ui_copy(image, w);
                    }
                }
                _ => self.scene.textures.refine_ui_image(image, have, &self.scene.jobs),
            }
        }
        let now = Instant::now();
        let textures = &self.scene.textures;
        self.ui_coarse
            .retain(|id, (_, since)| now.duration_since(*since) < FOLLOWED && !textures.is_unavailable(id));
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
        for (x, y) in std::mem::take(&mut self.places_ui.want_cells) {
            self.world.map.want_regions(x, y, x, y);
        }
        for (spot, global, region_id) in self.places_ui.want_snapshots.drain(..) {
            self.world.place_snapshots.want(spot, global, region_id, now);
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
                    priority: crate::scene::fetch_priority::urgent::WINDOW,
                    lane: aurora_net::Lane::Urgent,
                    accept: "*/*",
                });
            }
        }
        self.world.place_details.update(&self.world.map, &mut self.world.landmarks, now);
        for result in self.world.landmarks.take_actions(&mut self.world.map, now) {
            use crate::world::landmarks::ActionResult;
            match result {
                ActionResult::Slurl(url) => self.egui_ctx.copy_text(url),
                ActionResult::Map(global) => {
                    self.world.map.track_location(global.x, global.y, global.z as f32, false);
                    ui::widgets::open_floater(&self.egui_ctx, "worldmap", &mut self.panels.world_map);
                }
                ActionResult::Unavailable => self.inventory_ui.message = aurora_i18n::tr!("inventory-landmark-location-unknown").into(),
            }
        }
        let mut cmds = self.world.place_details.take_commands();
        cmds.extend(self.world.place_snapshots.take_commands());
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
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_PARCEL_VISIBILITY")
        {
            crate::demo::parcel_visibility::update_agent_parcel(&mut self.world, &mode, self.frame_count);
        }
        for command in self
            .world
            .update_parcel_visibility(&self.settings.parcel_visibility, Instant::now())
        {
            self.send(command);
        }
        if self.demo && std::env::var("AURORA_DEMO_PARCEL_VISIBILITY").is_ok_and(|mode| mode == "crossings") {
            crate::demo::parcel_visibility::check_crossings(&self.world, self.frame_count);
        }
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
        if !self.settings.rlva_enabled {
            self.world.maturity.cancel_script_teleports();
        }
        for (actor, c) in self.world.maturity.take_commands() {
            if matches!(
                c,
                NetCommand::TeleportTo { .. }
                    | NetCommand::TeleportToRegion { .. }
                    | NetCommand::TeleportHome
                    | NetCommand::TeleportLandmark(_)
                    | NetCommand::AcceptLure { .. }
            ) {
                self.send_deferred_teleport(c, actor);
            } else {
                self.send_from(c, actor);
            }
        }
        // Preferences › Privacy's account settings
        for c in self.world.user_info.take_commands() {
            self.send(c);
        }
        // the region console's command lines
        for c in self.world.sim_console.take_commands() {
            self.send(c);
        }
        // the script editor's keywords follow the region's LSL syntax
        if let Some(c) = self.world.follow_lsl_syntax() {
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
                self.request_user_teleport(c, String::new(), false);
            } else {
                self.send(c);
            }
        }
        for c in self.world.take_map_commands() {
            if let NetCommand::TeleportTo { handle, .. } = &c {
                let join = self.world.prepare_friend_join(&c);
                let flight_target = self.world.friend_teleport_avatar(&c);
                let dest = self.world.map.track.as_ref().map(|t| t.label.clone()).unwrap_or_default();
                let show = Some(*handle) != self.world.main_region;
                let accepted = if join.is_none()
                    && let Some(avatar) = flight_target
                {
                    self.request_friend_join_teleport(c, avatar, self.world.agent.flying, false)
                } else {
                    self.request_user_teleport(c, dest, show)
                };
                if accepted && let Some(join) = join {
                    self.world.arm_friend_join(join);
                }
            } else {
                self.send(c);
            }
        }
        let i = &self.input;
        let moved = i.forward
            || i.back
            || i.turn_left
            || i.turn_right
            || i.strafe_left
            || i.strafe_right
            || i.up
            || i.down
            || i.joystick_yaw.abs() > 0.001
            || i.joystick_pitch.abs() > 0.001
            || self.mouse_mode == MouseMode::Steer;
        if let Some(join) = self.world.finish_friend_join(Instant::now(), moved) {
            self.request_friend_join_teleport(join.command, join.avatar, join.fly_on_arrival, join.fresh_physics);
        }
        if moved {
            self.world.cancel_friend_join_flight();
        }
        self.start_friend_join_flight();
        self.minimap_ui.prune(&self.world);
        // parcel overlays of regions we left (their lines would be stale on return)
        let regions = &self.world.regions;
        self.world.map.overlays.retain(|h, _| regions.contains_key(h));
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        // the input this frame reads came with the events just handled: its
        // time to the screen starts here (Performance › latency, pacing.rs)
        if let Some(g) = &mut self.gfx {
            g.renderer.mark_input(Instant::now());
        }
        self.sync_minimized_mute();
        aurora_render::frame_priority::apply();
        self.world.anti_spam.configure(self.settings.anti_spam);
        self.update_rlva();
        if self.demo && self.frame_count == 700 && std::env::var_os("AURORA_DEMO_ANTI_SPAM").is_some() {
            for event in crate::demo::anti_spam::events(self.world.agent_id) {
                if let Some(event) = self.world.apply(event) {
                    self.on_app_event(event);
                }
            }
            log::info!(
                "anti-spam demo: {} blocked source/category pairs",
                self.world.anti_spam.blocks().len()
            );
        }
        if self.demo {
            self.demo_uri_restore();
        }
        if self.demo && self.frame_count == 650 && std::env::var_os("AURORA_DEMO_EXIT").is_some() {
            self.request_exit();
        }
        // AURORA_DEMO_HOME=<outcome>:set: World › Set Home to Here, once settled
        if self.demo
            && self.frame_count == 700
            && crate::demo::home::scene().1
            && let Some(cmd) = self.world.set_home_here()
        {
            self.send(cmd);
        }
        // AURORA_DEMO_SIM_CONSOLE=open: the console and two commands
        if self.demo && self.frame_count == 600 && crate::demo::sim_extras::console_open() {
            self.panels.sim_console = true;
            self.world.sim_console.send("help");
            self.world.sim_console.send("stats");
        }
        // AURORA_DEMO_FREEZE: frozen by Nova, released later
        if self.demo {
            for ev in crate::demo::sim_extras::freeze_events(self.frame_count) {
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
            }
        }
        // AURORA_DEMO_EMOJI_CODES: the keyboard given to the input again once
        // the demo's start has settled (it takes it back at the start)
        if self.demo
            && self.frame_count == 700
            && (std::env::var_os("AURORA_DEMO_EMOJI_CODES").is_some() || std::env::var_os("AURORA_DEMO_CHAT_INPUT").is_some())
        {
            self.chat_ui.conv_focus_request = true;
        }
        // AURORA_DEMO_SPELLING=menu|replace: the chat bar takes the keyboard,
        // its caret at the end
        if self.demo
            && self.frame_count == 700
            && matches!(
                crate::ui::text_menu::spelling_demo(),
                Some(crate::ui::text_menu::SpellingDemo::Menu | crate::ui::text_menu::SpellingDemo::Replace)
            )
        {
            self.chat_ui.focus_request = true;
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
        // the media filter's list in the preferences: the account's while
        // logged in, and the changes made there
        for edit in std::mem::take(&mut self.options_ui.media_filter_edits) {
            self.world.media_filter.apply(edit);
        }
        let rules = self.in_world().then(|| self.world.media_filter.rules());
        if self.options_ui.media_filter.as_deref() != rules {
            self.options_ui.media_filter = rules.map(<[_]>::to_vec);
        }
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
        if self.in_world() {
            self.session_heartbeat(now);
        }
        self.mouselook_alt_cursor();
        if !self.exit_prompt.open && self.camera.mouselook() && self.focused && !self.cursor_hold.held() && !self.alt_cursor.free() {
            self.set_mouselook_grab(true);
        }
        // a photo drawn in several frames: from its second part on, the
        // world keeps the instant of the first, time does not step and the
        // network's events wait (snapshot::Hold, rawSnapshot's single call)
        let hold = self.photo_hold(now);
        let still = hold.is_some_and(|h| h.later);
        let world_now = move || hold.map_or_else(Instant::now, |h| h.at);
        // (the frame's own instant, for what reads it outside a photo)
        let world_frame = hold.map_or(now, |h| h.at);
        let start = self.start;
        let world_clock = move || hold.map_or_else(|| start.elapsed().as_secs_f32(), |h| h.clock);
        let dt = if still {
            0.0
        } else {
            now.duration_since(self.last_frame).as_secs_f32().min(0.1)
        };
        // the photo mode's frozen world: the camera and the interface keep
        // the frame's time, what moves in the world gets none
        let frozen = self.photo_world_frozen();
        let world_dt = if frozen { 0.0 } else { dt };
        // the rez effects and the loading of what the camera finds go on
        let effects_now = move || if frozen { Instant::now() } else { world_now() };
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
        if self.skin.maybe_reload(&self.egui_ctx, text_scale(&self.settings)) {
            crate::crash::update(&self.skin.theme, &self.settings);
            self.palette = self.skin.theme.palette();
            self.cursors.set_palette(&self.palette);
            self.egui_ctx.set_zoom_factor(ui_scale(&self.settings));
        }

        self.frame_profile.lap(Lap::Start);
        // ---- network events (time-boxed)
        let t_ev = Instant::now();
        self.world.events_this_frame = 0;
        self.sync_script_debug();
        self.world.received_items.enabled = self.settings.sort_received_items;
        self.world.inventory_offers.auto_accept = self.settings.auto_accept_inventory;
        self.world.inventory_offers.show_accepted = self.settings.show_accepted_in_inventory;
        self.world.payments.open_for_friends = self.settings.payment_opens_conversation;
        self.world.payments.open_when_paying = self.settings.payment_sent_opens_conversation;
        self.world.received_items.enabled_since = self.settings.received_items_since;
        if self.demo
            && self.frame_count == 700
            && std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| mode.ends_with("received-filing"))
        {
            self.inventory_ui.expand_all = Some(true);
        }
        self.sync_chat_log();
        while !still && let Ok((sent, ev)) = self.net.events.try_recv() {
            // (`sent`: when the session read the datagram, the time its
            // motion starts from)
            if let Some(app_ev) = self.bridge_runtime.receive(&mut self.world, ev, Instant::now(), sent) {
                self.on_app_event(app_ev);
            }
            let now = Instant::now();
            self.net_profile.event_applied(sent, now);
            if now.duration_since(t_ev) > Duration::from_millis(8) {
                break;
            }
        }
        self.net_profile.events_left(self.net.events.len());
        let corrections = self.world.agent.take_corrections();
        if corrections.count > 0 {
            aurora_net::transition::step(|| format!("position of our avatar corrected by {:.2} m", corrections.largest));
        }
        self.net_profile.corrections(corrections);
        self.net_profile.lead(self.world.agent.prediction.take_lead_max());
        // a border crossing changed what positions are relative to: the
        // camera moves with the agent, in the same frame
        let shift = self.world.take_origin_shift();
        if shift != Vec3::ZERO {
            self.camera.shift(shift);
            if let Some(ap) = &mut self.autopilot {
                ap.shift(shift);
            }
        }
        self.world.poll_inventory_cache();
        let receipts = std::mem::take(&mut self.world.received_items.receipts);
        let deliveries = self.inventory_ui.update_received(&mut self.world.inventory, receipts);
        // the demo's seeded inboxes are not news unless its scenario asks
        if !deliveries.is_empty() && self.settings.notify_deliveries && (!self.demo || crate::demo::delivery::mode().is_some()) {
            self.world.notify_deliveries(&deliveries);
        }
        self.world.poll_transcripts();
        self.update_hover_height();
        self.update_avatar_health();
        if self.demo {
            crate::demo::tangents::tick(&mut self.world, &mut self.scene, self.frame_count);
            crate::demo::rigged_prop::tick(&mut self.world, &mut self.scene, self.frame_count);
            if let Some(e) = crate::demo::conference::tick().and_then(|ev| self.world.apply(ev)) {
                self.on_app_event(e);
            }
        }
        self.perf.events_ms = t_ev.elapsed().as_secs_f32() * 1000.0;
        for anim in std::mem::take(&mut self.world.animation_stops) {
            self.send(NetCommand::AgentAnimation { anim, start: false });
        }
        // our script errors said on their channel (#1263), through RLVa's
        // @sendchannel like any other line
        for (channel, message) in std::mem::take(&mut self.world.script_debug.outgoing) {
            self.send(NetCommand::Chat {
                message,
                channel,
                chat_type: aurora_net::ChatType::Normal,
            });
        }
        self.apply_skeleton_resets();
        self.frame_profile.lap(Lap::Events);
        self.poll_ao();
        self.sync_static_eyes();
        if self.in_world() {
            self.step_gestures();
        }
        if self.in_world() && self.last_social_poll.elapsed() > Duration::from_millis(500) {
            self.last_social_poll = Instant::now();
            self.poll_names();
            for id in self.world.take_profile_requests() {
                self.send(NetCommand::RequestProfile(id));
            }
            self.send_derender_requests();
            // Parentless server records are filed before their COF links:
            // reliable MoveInventoryItem, matching AISUpdate::doUpdate.
            if let Some(change) = self.world.inventory.take_parent_repairs() {
                let request = uuid::Uuid::new_v4();
                self.world_edits.insert(request);
                self.send(NetCommand::EditInventory { request, change });
            }
            // outfit restore at login (queues the COF fetch below)
            let second_life = !matches!(self.settings.grid, crate::settings::GridChoice::Custom);
            self.world.update_outfit(second_life, Instant::now());
            let allow_links = self.world.outfit.is_done() && self.appearance_ui.pending.is_none() && self.inventory_ui.pending.is_none();
            for command in self.world.update_attachment_inventory(Instant::now(), allow_links) {
                if let NetCommand::EditInventory { request, .. } = &command {
                    // Serialize COF writes with the same guard as interactive
                    // outfit changes, including inventory link replacement.
                    self.appearance_ui.pending = Some((*request, false));
                }
                self.send(command);
            }
            for cmd in std::mem::take(&mut self.world.outfit.commands) {
                self.send(cmd);
            }
            // block list, groups and group chat sessions
            self.world.groups.mute_all_groups = self.settings.mute_all_groups;
            self.world.groups.mute_when_notices_off = self.settings.mute_groups_without_notices;
            self.world.groups.report_blocks = self.settings.report_blocks;
            self.world.groups.show_joined_invitations = self.settings.show_joined_group_invitations;
            self.world.group_titles = self.settings.group_titles;
            self.world.fly_after_teleport = self.settings.fly_after_teleport && !(self.demo && crate::demo::friend_join_flight_scenario());
            self.world.region_alerts.cfg = crate::world::region_alerts::Config {
                script_count: self.settings.script_count_alert,
                script_threshold: self.settings.script_count_threshold,
                // AURORA_DEMO_SIM_VERSION=on: on for the run, not saved
                version: self.settings.sim_version_alert || (self.demo && crate::demo::sim_version::forced()),
            };
            // AURORA_DEMO_COLLISIONS=chat: the nearby chat lines for this run only
            self.world.collisions.cfg = if self.demo {
                crate::demo::collisions::settings(self.settings.collisions)
            } else {
                self.settings.collisions
            };
            // FSDisableNeighbourRegionConnections, lifted while crossing
            let refuse = self.settings.no_neighbour_regions && self.world.agent_inside_main_region();
            if self.neighbours_refused != refuse {
                self.neighbours_refused = refuse;
                self.net.send(NetCommand::RefuseNeighbourRegions(refuse));
            }
            // FSRestoreOpenIMs: the last session's private conversations,
            // once, after login
            if !self.world.reopen_ims.is_empty() {
                let others = std::mem::take(&mut self.world.reopen_ims);
                if self.settings.reopen_ims {
                    for other in others {
                        self.world.social.session_mut(other);
                    }
                }
            }
            // AURORA_DEMO_OBJECT_IM: a filter for this run only
            self.world.object_im_filter = crate::demo::object_im::settings()
                .filter(|_| self.demo)
                .unwrap_or(self.settings.object_im_filter);
            if self.world.status.cfg != self.settings.autoresponse {
                self.world.status.cfg = self.settings.autoresponse.clone();
            }
            // AFKTimeout: away after some idle time, back on the next input
            let idle_limit = if self.world.rlva.has("allowidle") {
                30 * 60
            } else {
                self.settings.autoresponse.away_after_minutes as u64 * 60
            };
            if idle_limit > 0 && !self.world.status.away && self.last_input.elapsed().as_secs() >= idle_limit {
                self.world.set_away(true);
                self.auto_away = true;
            } else if self.auto_away && self.last_input.elapsed().as_secs() < 1 {
                self.world.set_away(false);
                self.auto_away = false;
            }
            // QuitAfterSecondsOfAFK: an ordinary quit (unsaved scripts still
            // ask); asked once per time away
            if self.world.away_too_long(Instant::now()) {
                log::info!("quitting: away for longer than the setting");
                self.world.status.away_since = None;
                self.exit_prompt.request();
                self.confirm_exit(ui::exit::Action::Quit);
            }
            self.world.tick_social();
            self.update_gestures();
            for cmd in self.world.take_social_commands() {
                self.send(cmd);
            }
            // Background inventory changes, whatever the inventory window does.
            self.update_marketplace_listings();
            for command in self.world.file_received_items(Instant::now()) {
                self.send(command);
            }
            // ShowInInventory: a kept offer's item, once it is there
            // (openInventoryPanelAndSetSelection), never past @showinv
            if let Some(id) = self.world.take_inventory_reveal(Instant::now())
                && !self.world.rlva.has("showinv")
            {
                self.inventory_ui.show_original(&self.world.inventory, id);
                ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory);
            }
            self.contents_copy_tick(Instant::now());
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
            for cmd in self.world.pump_inventory_background() {
                self.send(cmd);
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
        // AURORA_DEMO_LINK's lost link, before the frame takes the window
        self.demo_link_lost();
        let Some(mut gfx) = self.gfx.take() else {
            return;
        };
        self.apply_local_imports(&mut gfx.renderer);
        self.wearable_ui.preview.apply(&mut self.world, &mut self.scene, &mut gfx.renderer);
        self.update_blender_live(&mut gfx.renderer);
        let t_up = Instant::now();
        let (w, h) = gfx.renderer.size();
        // a photo drawn at its own size this frame: its proportions, the
        // part of it this frame draws, the height its details are chosen for
        self.photo_update(&gfx.renderer);
        self.viewfinder_frame(&mut gfx.renderer);
        self.photo_isolate_frame(&mut gfx.renderer);
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
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_GROUNDING")
            && let Some(input) = crate::demo::grounding::scripted_input(&mode, self.frame_count)
        {
            self.input = input;
        }
        self.update_autopilot(dt);
        if self.in_world() && self.settings.automatic_land {
            self.world.auto_land(&self.input);
        }
        self.update_native_movelock();
        self.update_movement();
        self.update_audio();
        self.update_wind_sound(dt);
        let mut late_camera_frame = None;
        let mut late_controls = None;
        if self.in_world() {
            self.update_sounds();
            self.update_attention();
        }
        if self.in_world() {
            let ml = self.camera.mouselook();
            // the clocks the other avatars and the objects move along
            // (world/motion.rs); offline there is no simulator to slow them
            let build = &self.build;
            self.world
                .advance_motion(world_now(), &self.net.stats.simulators.lock(), |key| build.is_selected(key));
            if self.demo && self.demo_link_cfg.is_some() {
                // offline through a simulated link, as on a grid
                self.drive_demo_link(ml, dt);
            } else if self.demo {
                // no simulator offline: move the avatar locally
                let p = self.world.agent.position;
                let floor = crate::demo::floor_at(p.x, p.y);
                let floor = std::env::var("AURORA_DEMO_GROUNDING")
                    .ok()
                    .filter(|_| self.frame_count >= 600)
                    .map_or(floor, |mode| {
                        crate::demo::grounding::simulation_floor(&self.world, &self.scene, &mode)
                    });
                self.world
                    .agent
                    .simulate_locally(&self.input, ml, Some(floor), dt, self.settings.automatic_land);
                if let Some(mm) = crate::demo::agent_jitter() {
                    let p = self.world.agent.predicted_position() + crate::demo::agent_jitter_offset(self.frame_count, mm);
                    self.world.agent.constrain_prediction(p, false);
                }
            } else if !still {
                let motion = *self.net.stats.simulator_motion.lock();
                if motion.handle == self.world.main_region {
                    self.world.predict_agent(motion.time_dilation, motion.packet_age());
                }
            }
            self.update_own_prediction(ml, dt);
            let camera_distance = self.camera.position.distance(self.world.agent.position);
            self.world.agent.update(&self.input, ml, dt, camera_distance);
            if self.demo
                && let Ok(mode) = std::env::var("AURORA_DEMO_GROUNDING")
            {
                crate::demo::grounding::update_motion(&mut self.world, &self.scene, self.frame_count, &mode, &self.input);
            }
            // seated, the agent's position is the one its seat gives it: the
            // map, the radar, the location bar and the voice read it
            if self.world.agent.seated
                && let Some(p) = crate::scene::Scene::seated_agent_position(&self.world, world_now())
            {
                self.world.agent.follow_seat(p);
            }
            // mouselook left with the zoom still held (Escape, M, a seat):
            // the normal angle comes back, the zoomed one is remembered
            if !self.camera.mouselook() && self.camera.mouselook_zoom_held() {
                self.end_mouselook_zoom();
            }
            let frame = crate::camera::FrameInput {
                dt,
                aspect: self.camera_aspect(),
                now: world_now(),
                steering: self.mouse_mode == MouseMode::Steer,
                build_mode: self.build.open,
                edit_tracking: self
                    .camera
                    .focus_object_id()
                    .filter(|id| self.build.edits_object(&self.world, id))
                    .map(|_| self.settings.build.follow_edit_object),
                eye_offset: None,
                draw_distance: self.settings.draw_distance,
            };
            if frame.edit_tracking.is_some() || self.camera.needs_animated_eyes(&self.world.agent, &self.settings.camera) {
                // Evaluate the focused edit selection after the local drag,
                // and first-person eyes after this frame's animation.
                late_camera_frame = Some(frame);
            } else {
                self.camera.update(&mut self.world, &self.settings.camera, &frame);
            }
            let at = self.camera.forward();
            let step_back = self.settings.walk_backwards_without_turning;
            // (Body::update turns a little even without time: held apart)
            if !frozen {
                self.world.agent.body_turn = self.world.update_bodies(world_now(), dt, at, ml, step_back);
            }
            self.trace_own_motion();
            self.update_pointing();
            self.update_look_at(w as f32, h as f32);
            let mut c = self.world.agent.controls(&self.input, &self.camera, self.settings.draw_distance);
            // the walk to a double-clicked point (src/autopilot.rs)
            c.control_flags = self.autopilot_controls(c.control_flags);
            c.control_flags = crate::movement::controls(
                c.control_flags,
                self.movement_runtime.state,
                self.bridge_runtime.captures_motion(Instant::now()),
            );
            if self.autopilot.is_some() {
                c.flags |= crate::autopilot::AU_FLAGS_CLIENT_AUTOPILOT;
            }
            // RenderHideGroupTitle (send_agent_update)
            if self.settings.group_titles == crate::world::names::GroupTitles::AllExceptMine {
                c.flags |= crate::autopilot::AU_FLAGS_HIDETITLE;
            }
            let moving = control::AT_POS | control::AT_NEG | control::LEFT_POS | control::LEFT_NEG | control::UP_POS | control::UP_NEG;
            if self.world.status.away && c.control_flags & moving != 0 {
                self.world.set_away(false);
                self.auto_away = false;
            }
            if self.world.status.away {
                c.control_flags |= control::AWAY;
            }
            c.control_flags = crate::agent::AgentState::quickjump_flags(c.control_flags, self.settings.quickjump);
            // send_agent_update: Alt or Ctrl turns script clicks into releases
            c.alt_or_ctrl = self.alt || self.ctrl;
            if self.chat_ui.local_typing() && self.world.rlva.can_send_typing_start() {
                c.state |= aurora_net::AGENT_STATE_TYPING;
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
            if self.world.rlva.has("unsit") {
                c.control_flags &= !control::STAND_UP;
            }
            if self.world.rlva.has("sit") || (self.world.agent.is_sitting() && self.world.rlva.has("unsit")) {
                c.control_flags &= !control::SIT_ON_GROUND;
            }
            if !self.world.agent.flying && self.world.rlva.has("jump") {
                c.control_flags &= !control::UP_POS;
            }
            if late_camera_frame.is_some() && self.camera.needs_animated_eyes(&self.world.agent, &self.settings.camera) {
                late_controls = Some(c);
            } else {
                self.net.set_controls(c);
            }
        }
        self.update_voice();
        self.update_radar();
        self.update_bridge();
        self.update_region_restart();
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_HAND_IK").or_else(|_| {
                std::env::var("AURORA_DEMO_IK_EDIT").and_then(|m| {
                    if m.starts_with("poser") {
                        Err(std::env::VarError::NotPresent)
                    } else {
                        Ok("1".into())
                    }
                })
            })
        {
            crate::demo::hand_ik::tick(&mut self.world, &mut self.scene, self.frame_count, &mode);
            if self.frame_count == 350 {
                self.camera.demo_orbit(0.6, 0.15, Some(3.5), &self.settings.camera);
                self.panels.chat = false;
                self.panels.people = false;
                self.panels.perf = false;
            }
        }
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_IK_EDIT")
        {
            if mode == "shortcut" {
                if matches!(self.frame_count, 420 | 520) {
                    // Exercise the configurable command table, including a
                    // second press that must keep the existing editor open.
                    let ctrl = self.ctrl;
                    self.ctrl = true;
                    self.on_input(Input::key(KeyCode::KeyK), true);
                    self.on_input(Input::key(KeyCode::KeyK), false);
                    self.ctrl = ctrl;
                    log::info!("demo IK shortcut: frame={}, open={}", self.frame_count, self.ik_editor.open);
                }
            } else {
                self.ik_editor.demo_tick(&self.world, self.frame_count, &mode);
            }
            if mode.starts_with("build-") && self.frame_count == 450 {
                // Exercise the actual menu transitions in both opening orders.
                self.handle_actions(
                    UiActions {
                        bar: BarAction::Build(1),
                        ..Default::default()
                    },
                    event_loop,
                );
                self.settings.build.edit_linked = true;
                let root = crate::demo::action_id(crate::demo::hand_ik::ROOT);
                if let Some(index) = self.world.objects.index_of_uuid(&root)
                    && let Some(object) = self.world.objects.get_mut(index)
                {
                    object.update_flags |= ui::context::flags::OBJECT_MOVE | ui::context::flags::OBJECT_MODIFY;
                }
                for (local, name) in [(crate::demo::hand_ik::LEFT, "Left"), (crate::demo::hand_ik::RIGHT, "Right")] {
                    let id = crate::demo::action_id(local);
                    self.build.props.insert(
                        id,
                        aurora_net::build::ObjectProps {
                            object_id: id,
                            name: name.into(),
                            ..Default::default()
                        },
                    );
                }
                if let Some(index) = self
                    .world
                    .objects
                    .index_of_uuid(&crate::demo::action_id(crate::demo::hand_ik::LEFT))
                {
                    self.build.click_select(&mut self.world, &self.settings.build, Some(index), false);
                    if mode == "build-moved" {
                        let p = crate::scene::Scene::object_transform(&self.world, index, Instant::now(), 0).map(|t| t.0);
                        self.build.set_transform(
                            &mut self.world,
                            &self.settings.build,
                            p.map(|p| p + Vec3::Z * 0.08),
                            None,
                            None,
                            true,
                        );
                    }
                }
                if mode == "build-before" {
                    for _ in 0..2 {
                        self.handle_actions(
                            UiActions {
                                bar: BarAction::IkEdit,
                                ..Default::default()
                            },
                            event_loop,
                        );
                    }
                }
                if mode == "build-pick"
                    && let Some(index) = self
                        .world
                        .objects
                        .index_of_uuid(&crate::demo::action_id(crate::demo::hand_ik::RIGHT))
                {
                    self.ik_editor.picking = true;
                    self.ik_editor.picked_prim(&self.world, &self.build, index);
                }
                crate::ui::widgets::place_floater(
                    &self.egui_ctx,
                    "build_tools_inspector_c3",
                    egui::pos2(self.egui_ctx.content_rect().right() - 370.0, 80.0),
                    egui::vec2(344.0, 684.0),
                );
            }
            if mode.starts_with("build-") && self.frame_count == 600 {
                log::info!(
                    "demo concurrent IK/build: ik={}, build={}, selected={}",
                    self.ik_editor.open,
                    self.build.open,
                    self.build.selection.len()
                );
            }
        }
        self.world.hand_contacts.relay_required = !self.demo;
        if self.in_world()
            && let Some(key) = self
                .world
                .update_ik_targets(self.ik_editor.link_target_definition(), &self.build.props, Instant::now())
        {
            for command in self.build.prim_properties_query(&self.world, key) {
                if self.demo {
                    for event in crate::demo::hand_ik::editor_reply(&command, self.frame_count, "") {
                        match event {
                            NetEvent::ObjectProperties(props) => {
                                self.world.ik_targets.on_properties(&props);
                                self.build.on_properties(props);
                            }
                            event => {
                                self.world.apply(event);
                            }
                        }
                    }
                } else {
                    self.send(command);
                }
            }
        }
        if self.in_world()
            && let Some(command) = self.world.update_hand_contacts(Instant::now())
        {
            if self.demo {
                let mode = std::env::var("AURORA_DEMO_IK_EDIT").unwrap_or_default();
                for event in crate::demo::hand_ik::editor_reply(&command, self.frame_count, &mode) {
                    self.world.apply(event);
                }
            } else {
                self.send(command);
            }
        }
        if self.demo
            && matches!(self.screen, Screen::World)
            && crate::demo::water::update(
                &mut self.camera,
                &mut self.settings,
                &mut self.world,
                &mut self.panels,
                self.frame_count,
                self.start.elapsed().as_secs_f32(),
            )
        {
            gfx.renderer.apply_settings(self.settings.render_settings());
        }
        let mut view = self.camera.view();
        let mut proj = frame_proj(&self.camera);
        let mut vp = proj * view;
        let camera_fast = self.texture_motion.update(self.camera.position, self.camera.forward(), dt);
        let mut cull = CullView::new(vp, self.camera.position, lod_height as f32, self.camera.fov_y).with_texture_camera(
            self.camera.forward(),
            aspect,
            camera_fast,
        );
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
            self.scene.synchronize_animations = self.settings.synchronize_animations;
            self.scene.ground_avatar_feet = self.settings.ground_avatar_feet;
            self.scene.ground_feet_while_moving = self.settings.ground_feet_while_moving;
            self.scene.ground_feet_max_height = self.settings.ground_feet_max_height;
            self.scene.set_foot_scope_limit(self.settings.ground_feet_max_avatars);
            self.scene.body_physics = crate::scene::body_physics::Config {
                enabled: self.settings.avatar_physics,
                detail: self.settings.avatar_physics_detail,
                pixel_angle: lod_height as f32 / self.camera.fov_y.max(1e-3),
            };
            self.scene.lod_factor = self.settings.lod_factor;
            self.scene.max_avatars = self.settings.max_avatars as usize;
            if self.demo {
                self.scene.max_avatars = crate::demo::impostor_flip(self.scene.max_avatars);
            }
            // reflection lists: planar reflections and reflection probe captures
            self.scene.reflections = self.settings.water_reflections > 0 || self.settings.mirrors > 0 || in_world;
            self.scene.fov_y = self.camera.fov_y;
            self.scene.agent_idx = self.world.objects.index_of_uuid(&self.world.agent_id);
            if self.demo
                && let Some(alpha) = crate::demo::animesh_alpha_step(self.frame_count)
            {
                self.settings.debug.show_alpha = alpha.is_some();
                self.settings.debug.show_alpha_rigged = alpha.unwrap_or(false);
            }
            let debug_alpha = match self.settings.debug {
                d if d.show_alpha
                    && !self.world.rlva.has("viewtransparent")
                    && !self.world.rlva.has("edit")
                    && !self.world.rlva.has("showloc") =>
                {
                    Some(d.show_alpha_rigged)
                }
                _ => None,
            };
            self.scene.set_debug_alpha(&mut self.world, debug_alpha);
            self.scene.set_pass_view(&mut self.world, self.settings.debug.pass_view);
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
            self.grow_geometry_ahead(&mut gfx.renderer, covered);
            self.scene.process_results(&mut gfx.renderer, &self.net, results_budget);
            self.frame_profile.lap(Lap::Results);
            if self.demo
                && let Some(mode) = &self.ao_demo
            {
                crate::ao::demo::signal(&mut self.world, self.frame_count, mode, now);
            }
            self.update_ao(now);
            if self.demo && crate::demo::animation_edit_demo() {
                match self.frame_count {
                    420 | 750 => {
                        let local = if self.frame_count == 420 { crate::demo::OWN_RIGGED } else { 9200 };
                        let idx = self.world.objects.iter().find_map(|(i, o)| (o.key.local_id == local).then_some(i));
                        self.build.open_build(crate::build::Tool::Edit);
                        self.build.click_select(&mut self.world, &self.settings.build, idx, false);
                        log::info!("demo animation edit: frame {}, selected local {local}", self.frame_count);
                    }
                    630 | 930 => {
                        self.build.close(&mut self.world, &mut self.settings.build);
                        log::info!("demo animation edit: frame {}, released selection", self.frame_count);
                    }
                    _ => {}
                }
            }
            // what carries an edited attachment stands still (pauseAssociatedAvatars)
            self.scene.paused_skeletons = crate::build::anim_pause::paused_skeletons(&self.world, &self.build.selection);
            if self.world.teleporting {
                self.ik_editor.stop_editing();
            }
            self.ik_editor.tick_project(&self.scene.jobs, &self.egui_ctx, &self.world);
            self.ik_editor.tick_animations(&mut self.scene.anims, &mut self.world.inventory);
            self.ik_editor
                .tick_clip(&self.scene.jobs, &self.egui_ctx, self.world.avatar_lib.rig.clone());
            self.scene.ik_edit = self.ik_editor.request();
            if self.demo
                && let Ok(mode) = std::env::var("AURORA_DEMO_POSER")
            {
                self.poser_demo(&mode);
            }
            self.tick_poser_consent();
            self.tick_portals();
            // Persist a sensitivity drag once it rests, like the other sliders.
            if !self.egui_ctx.input(|i| i.pointer.any_down()) && self.ik_editor.sync_poser_settings(&mut self.settings.poser) {
                self.settings.save();
            }
            self.scene.poser = self.ik_editor.pose_layers(&self.world);
            self.ik_editor.tick_pose_storage(&self.scene.jobs, &self.egui_ctx, &self.world);
            if !self.scene.poser.is_empty() {
                self.scene.ik_edit = Default::default();
            }
            if self.demo && crate::demo::animation_edit_demo() && matches!(self.frame_count, 420 | 630 | 750 | 930) {
                log::info!("demo animation edit: {} frozen skeletons", self.scene.paused_skeletons.len());
            }
            // poses first: attachments follow their bone in the same frame
            self.scene.update_foot_scope(&self.world, cull, world_frame);
            let completed = self.scene.update_poses(&mut self.world, self.camera.position, world_frame);
            self.scene.set_auto_hover_second_life(matches!(
                self.settings.grid,
                crate::settings::GridChoice::SecondLife | crate::settings::GridChoice::SecondLifeBeta
            ));
            self.scene.update_auto_hover(
                &mut self.world,
                self.settings.auto_hover_height,
                self.settings.auto_hover_offset,
                !self.hover_height_ui.previewing_offset(),
                world_frame,
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
            if self.build.hud_selected(&self.world) && !self.huds_shown() {
                self.build.deselect_all(&self.world);
            }
            self.build.sync_hud_zoom(&mut self.world);
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
            if self.build.tool == crate::build::Tool::Edit
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
            let over_ui = self.ui_takes_pointer();
            let mods = crate::build::Mods {
                shift: self.shift,
                ctrl: self.ctrl,
                alt: self.alt,
            };
            self.build
                .update(&mut self.world, &self.settings.build, self.cursor_pos, over_ui, mods);
            let mut selection_wire = self.build.wire_selection(&self.world, &self.settings.build);
            // another avatar's attachment the open menu is about, outlined
            // like a child prim of a selection (it cannot be selected)
            for idx in self.menu_outline() {
                selection_wire.entry(idx).or_insert(false);
            }
            for (idx, current) in self.inspect_outline() {
                selection_wire.entry(idx).or_insert(current);
            }
            self.scene.set_wire_selection(&mut self.world, selection_wire);
            self.scene.selection_silhouettes = self.build.silhouette_selection(&self.world, &self.settings.build);
            // Use face coverage while highlighting local appearances. Actual
            // build selection, wire outlines and edits stay untouched; its
            // ordinary face overlay returns when the mode is switched off.
            let selected_faces = if self.imports.local_status.revealing() {
                self.imports.appearance_status(&self.world.objects).faces
            } else {
                self.build.face_selection(&self.world, &self.settings.build)
            };
            self.scene.set_face_selection(&mut self.world, selected_faces);
            if std::mem::take(&mut self.build.ui.reveal_faces) {
                self.scene.reveal_selected_faces();
            }
            self.flush_build();
            let camera_changed = late_camera_frame.is_some() || panned;
            if let Some(mut frame) = late_camera_frame {
                frame.eye_offset = self.scene.eyes_offset(&self.world, &self.world.agent_id);
                if self.camera.needs_animated_eyes(&self.world.agent, &self.settings.camera) {
                    frame.now = now;
                }
                self.camera.update(&mut self.world, &self.settings.camera, &frame);
                if let Some(controls) = late_controls {
                    // AgentUpdate must describe the displayed eye and aim,
                    // keeping the movement, script and RLVa flags prepared above.
                    let current = self.world.agent.controls(&self.input, &self.camera, self.settings.draw_distance);
                    self.net.set_controls(aurora_net::AgentControls {
                        control_flags: (controls.control_flags & !control::MOUSELOOK) | (current.control_flags & control::MOUSELOOK),
                        state: controls.state,
                        flags: controls.flags,
                        alt_or_ctrl: controls.alt_or_ctrl,
                        ..current
                    });
                }
            }
            // every change of the viewing angle goes to the simulator, as
            // LLViewerCamera::setView (the end of a zoom's easing excepted)
            if (self.camera.fov_y - self.sent_fov).abs() > FOV_SEND_EPSILON {
                self.sent_fov = self.camera.fov_y;
                self.net.send(NetCommand::SetFov(self.sent_fov));
            }
            if camera_changed {
                view = self.camera.view();
                proj = frame_proj(&self.camera);
                vp = proj * view;
                cull = CullView::new(vp, self.camera.position, lod_height as f32, self.camera.fov_y).with_texture_camera(
                    self.camera.forward(),
                    aspect,
                    camera_fast,
                );
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
            self.scene.update_beacons(&mut self.world, &self.settings.beacons, &cull);
            self.scene.held_at = hold.map(|h| h.at);
            self.scene.sync(&mut gfx.renderer, &mut self.world, &cull);
            self.frame_profile.lap(Lap::Sync);
            {
                // web pages and videos on faces (LLViewerMedia::updateMedia)
                let a = &self.settings.audio;
                let cache_dir = crate::settings::cache_dir();
                let frame = crate::media::MediaFrame {
                    view: &cull,
                    camera: self.camera.position,
                    settings: &self.settings.media,
                    autoplay: a.media_autoplay,
                    channel_volume: self.media_volume(),
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
                for msg in std::mem::take(&mut self.world.media_filter.messages) {
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
                        command: self.command,
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
                        .hud_pick(&self.world, self.cursor_pos, false)
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
            self.scene.update_loading(&self.world, effects_now());
            // the rez effect of what appears (Preferences › Aurora;
            // AURORA_REZ_EFFECT=0|1 overrides it): its grid stands for the
            // clouds of the avatars it draws
            static REZ_ENV: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
            let rez_env = *REZ_ENV.get_or_init(|| std::env::var("AURORA_REZ_EFFECT").ok().map(|v| v.trim() != "0"));
            self.scene.rez.enabled = rez_env.unwrap_or(!self.settings.classic_rez_effect && !self.settings.reduce_motion);
            self.scene.rez.log = self.demo;
            let covered = matches!(self.screen, Screen::Loading { .. }) || self.tp_overlay.as_ref().is_some_and(|o| o.end.is_none());
            self.scene.update_rez(&mut gfx.renderer, &self.world, &cull, covered, effects_now());
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
                let objects = &self.world.objects;
                let rez = &self.scene.rez;
                self.scene
                    .loading_avatars()
                    .keys()
                    .copied()
                    .filter(|id| objects.index_of_uuid(id).is_none_or(|i| !rez.shows_avatar(i)))
                    .collect()
            } else {
                Vec::new()
            };
            self.scene.particles.sync_clouds(&clouds);
            self.scene.drop_rez_clouds(&self.world);
            self.scene
                .banlines
                .update(&mut gfx.renderer, &self.world, self.settings.maps.ban_lines, self.camera.position);
            self.scene.portals.update(
                &mut gfx.renderer,
                &self.world,
                self.camera.position,
                &self.palette,
                world_clock(),
                !self.settings.reduce_motion,
            );
            self.frame_profile.lap(Lap::Extras);
            let overlays = self.settings.world_overlays;
            self.scene
                .update_land_overlays(&mut gfx.renderer, &self.world, overlays, &self.palette);
            self.scene.set_first_person(
                self.camera
                    .first_person_view()
                    .then_some(self.settings.camera.show_avatar_in_mouselook),
            );
            self.scene.update_portal_clipping(&mut gfx.renderer, &self.world);
            self.scene.build_lists(&mut gfx.renderer, &cull, self.settings.shadows);
            let preview = if (self.focused || self.demo && self.capture.is_some())
                && !covered
                && !self.camera.mouselook()
                && !self.exit_prompt.open
                && self.cursor_pos.0 >= 0.0
                && self.cursor_pos.1 >= 0.0
                && self.cursor_pos.0 < w as f32
                && self.cursor_pos.1 < h as f32
                && self.build.creation_preview_enabled(&self.settings.build, over_ui, mods)
            {
                let mut candidates = std::mem::take(&mut self.scene.creation_preview.candidates);
                let ray = self.build.cam.ray(self.cursor_pos.0, self.cursor_pos.1);
                let surface = self
                    .scene
                    .creation_surface(&self.world, ray, self.settings.draw_distance, &mut candidates);
                self.scene.creation_preview.candidates = candidates;
                surface.and_then(|(point, _)| crate::scene::creation_preview::Placement::new(&self.world, &self.settings.build, point))
            } else {
                None
            };
            self.scene.creation_preview.update(&mut gfx.renderer, preview);
            self.scene.creation_preview.append(&mut self.scene.lists);
            let physics = crate::scene::physics_shapes::selection(&self.build, &self.world, &self.settings.build);
            self.scene
                .physics_shapes
                .update(&mut gfx.renderer, &self.world, &mut self.scene.meshes, &physics);
            self.scene.physics_shapes.append(&mut self.scene.lists);
            let huds_shown = self.huds_shown();
            self.scene.build_huds(&self.world, [hud_size.width, hud_size.height], huds_shown);
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
                self.scene
                    .log_attachments(&self.world, self.world.agent_id, &gfx.renderer, &|_| None, None);
                self.scene.log_attachments_near(&self.world, 10.0, &gfx.renderer);
            }
            self.finish_describe(&gfx.renderer);
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
                let region = self.demo_tp_region.take().or(w.main_region);
                if let (Some(position), Some(handle)) = (self.demo_tp_dest.take(), region) {
                    if crate::demo::friend_join_scenario() {
                        self.demo_friend_join_target = Some((
                            Instant::now() + Duration::from_millis(700),
                            Vec3::new(position.x, position.y, 2400.0),
                        ));
                    }
                    self.demo_unseat_on_teleport();
                    aurora_net::transition::step(|| "AgentMovementComplete (demo)".to_string());
                    aurora_net::transition::arrived(Duration::from_secs(20));
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
                    if teleport_screen::ready(o.mode, at.elapsed(), terrain) {
                        1.0
                    } else {
                        0.6 + 0.4 * (0.75 * terrain + 0.25 * tex).clamp(0.0, 1.0) * 0.97
                    }
                }
            };
            o.shown = teleport_screen::glide(o.mode, o.shown, target, dt);
            if teleport_screen::done(o.mode, o.shown, target) && o.end.is_none() {
                o.end = Some(Instant::now());
                aurora_net::transition::end(
                    Some(aurora_net::transition::Kind::Teleport),
                    match o.mode {
                        teleport_screen::Mode::Quick => "arrived (teleport screen hidden)",
                        teleport_screen::Mode::Old => "arrived (teleport screen hidden, old screen)",
                    },
                    &self.net.stats,
                );
            }
            // no answer from the simulator: give up the screen
            if o.arrived.is_none() && o.start.elapsed() > Duration::from_secs(90) && o.end.is_none() {
                o.end = Some(Instant::now());
                aurora_net::transition::end(
                    Some(aurora_net::transition::Kind::Teleport),
                    "failed (no arrival 90 s after the teleport screen was shown)",
                    &self.net.stats,
                );
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
            self.world.cloud_scroll += sky_frame.cloud_scroll_rate * world_dt / 100.0;
        }
        self.world.cloud_scroll = glam::Vec2::new(
            self.world.cloud_scroll.x.rem_euclid(1024.0),
            self.world.cloud_scroll.y.rem_euclid(1024.0),
        );
        let mut env = Environment::from_eep(&sky_frame, &water_frame, self.world.cloud_scroll, self.settings.draw_distance);
        if let Some(sun) = self.demo.then(crate::demo::sun_override).flatten() {
            env.sun_dir = sun;
        }
        self.options_ui.sky_tone_mapped = !env.sky.no_post;
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
        if in_world {
            self.scene.update_water_effects(
                &self.world,
                self.camera.position,
                world_clock(),
                self.settings.water_effects.render(),
            );
        }
        // particles
        if in_world {
            let max_particles = self.settings.max_particles as usize;
            let mut out = std::mem::take(&mut self.scene.lists.particles);
            let scene = &mut self.scene;
            scene.particles.set_preview(self.particle_editor.preview());
            scene.particles.update(
                &mut gfx.renderer,
                &mut scene.textures,
                &mut self.world,
                dt,
                frozen,
                self.camera.position,
                max_particles,
                self.settings.draw_distance,
                &self.settings.particle_filters,
                &mut out,
                &mut scene.parts,
            );
            self.scene.lists.particles = out;
            if self.settings.world_overlays.hide_particles {
                self.scene.lists.particles.clear();
            }
            t_part = Instant::now();
        }
        let (lights, spot_shadows) = if in_world {
            let view = crate::scene::lights::LightView {
                eye: self.camera.position,
                forward: self.camera.forward(),
                // the frame's own view, even when the culling is frozen
                frustum: CullView::new(vp, self.camera.position, lod_height as f32, self.camera.fov_y),
                range: self.settings.draw_distance,
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
            if self.probes.hdr != self.settings.hdr_emissive {
                self.probes.hdr = self.settings.hdr_emissive;
                self.probes.refresh();
            }
            self.probes
                .update(&self.scene, &self.world, self.camera.position, env.probe_ambiance, world_now())
        } else {
            aurora_render::ProbeFrame::default()
        };
        let mut fp = FrameParams {
            rlva_resolution_divisor: self.world.rlva.resolution_divisor.unwrap_or(1),
            rlva_overlays: self
                .world
                .rlva_overlay_frames(Instant::now())
                .into_iter()
                .filter_map(|frame| {
                    self.rlva_overlay_slots
                        .get(&frame.texture)
                        .map(|slot| aurora_render::OverlayEffect {
                            color: frame.color,
                            texture: [*slot, 0, 0, 0],
                        })
                })
                .collect(),
            rlva_spheres: self.world.rlva_sphere_frames(self.camera.position, Instant::now()),
            selection_color: egui::Rgba::from(if self.imports.local_status.revealing() {
                self.palette.warn
            } else {
                self.palette.violet_light
            })
            .to_array(),
            rez_gradient: {
                // AURORA_REZ_STYLE=1|2|3: the effect's look (grid, scanlines,
                // wireframe, the one chosen), to compare them
                static STYLE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
                let style = *STYLE.get_or_init(|| {
                    std::env::var("AURORA_REZ_STYLE")
                        .ok()
                        .and_then(|v| v.trim().parse::<u8>().ok())
                        .filter(|s| (1..=3).contains(s))
                        .unwrap_or(3) as f32
                });
                let mut g = self.palette.aurora_gradient().map(|c| egui::Rgba::from(c).to_array());
                g[0][3] = style;
                // (faces still grey are drawn as the grid only with it on)
                g[1][3] = if self.scene.rez.enabled { 1.0 } else { 0.0 };
                g
            },
            selection_pulse: self
                .imports
                .local_status
                .pulse(Instant::now(), self.settings.reduce_motion)
                .unwrap_or_else(|| self.scene.selection_pulse(Instant::now())),
            selection_scale: self.egui_ctx.pixels_per_point(),
            view,
            proj,
            photo_tile,
            camera_pos: self.camera.position,
            near: self.camera.near,
            far: self.settings.draw_distance,
            fov_y: self.camera.fov_y,
            time: if self.demo {
                crate::demo::water::inspection_time(world_clock())
            } else {
                world_clock()
            },
            // (objects still rez in while the world is frozen: the camera
            // finds new ones)
            anim_at: hold.filter(|_| !frozen).map(|h| h.at),
            tex_anim_at: hold.map(|h| h.at),
            sun_dir: env.sun_dir,
            sun_visible: env.sun_visible,
            sun_color: env.sun_color,
            sky_zenith: env.zenith,
            sky_horizon: env.horizon,
            ground_color: env.ground,
            fog_color: env.fog_color,
            fog_density: env.fog_density,
            exposure: self.photo_exposure(),
            exposure_lock: self.viewfinder_on() && self.snapshots.vf.ae_lock,
            tonemap_mix: self.settings.tonemap_mix,
            sharpen: self.settings.sharpen,
            glow: self.settings.glow,
            bloom: if self.settings.bloom { self.settings.bloom_strength } else { 0.0 },
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
            wireframe: self.settings.debug.wireframe && !self.world.rlva.has("viewwireframe") && !self.world.rlva_has_locked_hud(),
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
            // the photo's isolation, "show only" on an empty background
            cutout: self.world.photo_isolation.empty_background(),
            look: None,
        };
        // a photo's look, or the live view's in the photo mode
        let focus = fp.dof.as_ref().map(|d| d.focus);
        self.snapshots.look_focus = focus;
        fp.look = match &photo {
            Some(f) => f.look.clone(),
            None => self.live_look((w, h), focus),
        };
        self.scene.parts.lap(Part::ParamsProbes, t_part);

        self.frame_profile.lap(Lap::Params);
        // ---- UI
        let t_ui = Instant::now();
        self.world.objects.refresh_labelled();
        if self.system_fonts_pending && ui::fonts::fallback::ready().is_some() {
            self.system_fonts_pending = !ui::fonts::apply(&self.egui_ctx, &ui_font(&self.settings));
        }
        let mut raw = gfx.egui_state.take_egui_input(&gfx.window);
        raw.events.extend(self.alt_text.drain(..).map(egui::Event::Text));
        self.demo_script_input(&mut raw);
        if self.demo && std::env::var("AURORA_DEMO_OPTIONS").is_ok_and(|mode| mode == "1:scale-keys") {
            raw.focused = true;
            raw.events.retain(|event| !matches!(event, egui::Event::WindowFocused(false)));
            if self.frame_count == 600 {
                self.settings.ui_scale = 1.0;
                self.egui_ctx.set_zoom_factor(1.0);
            }
            let key = match self.frame_count {
                620 => Some(egui::Key::Plus),
                622 => Some(egui::Key::Equals),
                900 | 902 | 904 | 906 => Some(egui::Key::Minus),
                _ => None,
            };
            if let Some(key) = key {
                raw.events.extend([true, false].map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }));
            }
            if matches!(self.frame_count, 700 | 800 | 960 | 1040) {
                log::info!(
                    "demo keyboard UI scale: frame={} saved={:.2} displayed={:.2}",
                    self.frame_count,
                    self.settings.ui_scale,
                    self.egui_ctx.zoom_factor()
                );
            }
        }
        if self.demo && self.capture.is_some() {
            raw.events.extend(self.login_form.motion.demo_events(self.frame_count));
            raw.events.extend(self.login_form.manager.demo_events(self.frame_count));
        }
        // Inline-edit captures simulate logical focus without activating the
        // native window or taking the user's keyboard (with_active(false)).
        if self.demo
            && self.capture.is_some()
            && (ui::color_picker::demo::mode().is_some()
                || self.chat_ui.demo_reorder.is_some()
                || std::env::var_os("AURORA_DEMO_EMOJI_CODES").is_some()
                || std::env::var_os("AURORA_DEMO_CHAT_INPUT").is_some()
                || std::env::var("AURORA_DEMO_OBJECT_MENU").is_ok_and(|mode| mode.starts_with("customize"))
                || std::env::var("AURORA_DEMO_MARKETPLACE_LISTINGS").is_ok_and(|mode| mode.starts_with("menu-"))
                || std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| {
                    matches!(
                        v.as_str(),
                        "rename"
                            | "landmark"
                            | "aurora-landmark"
                            | "new-script"
                            | "new-note"
                            | "new-folder"
                            | "resize-left"
                            | "resize-right"
                            | "keyboard"
                            | "keyboard-fold"
                            | "aurora-keyboard"
                            | "aurora-keyboard-fold"
                            | "aurora-keyboard-wide"
                    ) || v.starts_with("note-")
                        || v.starts_with("clipboard-")
                        || v.starts_with("aurora-menu-")
                        || v.starts_with("bulk-")
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
            && let Some(rect) = self.egui_ctx.memory(|memory| memory.area_rect(egui::Id::unique("inventory")))
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
        // AURORA_DEMO_POINTER="x,y[,l|r|s|h][;x,y…]": the pointer at these window
        // pixels from frame 300, one point every 60 frames (hover states,
        // sub-menus); ",l"/",r" left-/right-clicks the interface there; ",s"
        // leaves the pointer at rest there instead of moving it every frame
        // (egui only shows a tooltip under a pointer at rest)
        if self.demo
            && self.frame_count >= 300
            && let Ok(v) = std::env::var("AURORA_DEMO_POINTER")
        {
            let points: Vec<(f32, f32, Option<egui::PointerButton>, bool, u64)> = v
                .split(';')
                .filter_map(|pt| {
                    let mut it = pt.split(',').map(str::trim);
                    let x = it.next()?.parse().ok()?;
                    let y = it.next()?.parse().ok()?;
                    let (button, still, release) = match it.next() {
                        Some("l") => (Some(egui::PointerButton::Primary), false, 3),
                        Some("r") => (Some(egui::PointerButton::Secondary), false, 3),
                        Some("h") => (Some(egui::PointerButton::Primary), true, 59),
                        Some("s") => (None, true, 3),
                        _ => (None, false, 3),
                    };
                    Some((x, y, button, still, release))
                })
                .collect();
            let step = (self.frame_count - 300) / 60;
            let i = (step as usize).min(points.len().saturating_sub(1));
            if let Some(&(x, y, button, still, release)) = points.get(i) {
                let ppp = self.egui_ctx.pixels_per_point().max(0.1);
                let pos = egui::pos2(x / ppp, y / ppp);
                let click_frame = (self.frame_count - 300) % 60;
                // at rest: only brought (back) there when it is elsewhere
                if !still || self.egui_ctx.input(|i| i.pointer.latest_pos()) != Some(pos) {
                    raw.events.push(egui::Event::PointerMoved(pos));
                }
                if let Some(button) = button
                    && step as usize == i
                    && (click_frame == 2 || click_frame == release)
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
            self.local_appearance_demo_input(&mut raw);
            self.texture_library_demo_input(&mut raw);
            self.portal_drop_demo_input(&mut raw);
            self.give_demo_input(&mut raw, vp, (w, h));
            self.contents_drop_demo_input(&mut raw, vp, (w, h));
            if self.frame_count == 420
                && std::env::var("AURORA_DEMO_ACTIONS").as_deref() == Ok("open-multi")
                && let Some(c) = &self.interactions.contents
            {
                ui::object_actions::contents_demo_select_all(&self.egui_ctx, c.target.object, &mut raw);
            }
            self.wearable_drop_demo_input(&mut raw);
            ui::inventory::drag::demo::input(&ctx, self.frame_count, &self.world.inventory, &self.inventory_ui, &mut raw);
            self.gesture_demo_input(&mut raw);
            self.suggest_demo_input(&mut raw);
            self.double_click_menu_demo_input(&mut raw);
            raw.events.extend(ui::menu::search_demo_events(self.frame_count));
            raw.events.extend(ui::widgets::demo::events(&ctx, self.frame_count));
            raw.events.extend(ui::widgets::demo::worn_events(&ctx, self.frame_count));
            raw.events
                .extend(ui::toolbar::demo_events(&ctx, self.frame_count, &mut self.toolbar_ui));
            raw.events.extend(ui::bars::favorites::demo_events(&ctx, self.frame_count));
            raw.events.extend(ui::chat::demo_events(&ctx, self.frame_count));
            raw.events.extend(crate::demo::conv_button::input_events(&ctx, self.frame_count));
            raw.events.extend(crate::demo::chatlog::input_events(self.frame_count));
            raw.events.extend(ui::appearance::demo_events(self.frame_count));
            raw.events.extend(ui::appearance::layer_demo_events(&ctx, self.frame_count));
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
            raw.events.extend(ui::inventory::hover_demo_events(
                &ctx,
                self.frame_count,
                &self.world.inventory,
                &mut self.inventory_ui,
            ));
            raw.events
                .extend(ui::inventory::delete_demo_events(&ctx, self.frame_count, &self.inventory_ui));
            ui::inventory::aurora_demo(&ctx, self.frame_count);
            ui::inventory::bulk_permissions_demo(self.frame_count, &mut self.inventory_ui);
            if std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| matches!(mode.as_str(), "show-original" | "aurora-show-original")) {
                match self.frame_count {
                    650 => ui::widgets::minimize_floater(&ctx, "inventory"),
                    750 => self.on_ctx_action(ui::context::CtxAction::AttachmentInventory(uuid::Uuid::from_u128(8100))),
                    _ => {}
                }
            }
        }
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
            && self.huds_shown()
            && let Some(view) = self.scene.lists.hud_view
        {
            self.build.cam = crate::build::geom::Cam::for_hud(view, ctx.pixels_per_point());
        }
        self.build.sync_mesh_inspector(&mut self.world, &mut self.scene);
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
        // Bug report form (disabled, see main.rs):
        // // Serialize the report capture with other users of renderer.captured.
        // // Start before drawing so the report window is absent from the picture.
        // if self.bug_report.capture_requested.is_some()
        //     && !self.inventory_images.auto_capture_pending
        //     && self.inventory_images.capturing.is_none()
        //     && !self.scene_capture_pending
        //     && !gfx.renderer.capture_request
        //     && !gfx.renderer.capture_scene
        //     && let Some(with_ui) = self.bug_report.capture_requested.take()
        // {
        //     self.bug_report.capture_pending = true;
        //     if with_ui {
        //         gfx.renderer.capture_request = true;
        //     } else {
        //         gfx.renderer.capture_scene = true;
        //     }
        // }
        self.report_abuse_capture(&mut gfx.renderer);
        self.sound_cues.before(&ctx, &raw);
        self.options_ui
            .prepare_scale_input(&raw, ctx.pixels_per_point(), self.numeric_drag.active());
        self.numeric_drag.before(&mut raw, ctx.pixels_per_point());
        let egui_state = &mut gfx.egui_state;
        self.text_menu.before(
            &ctx,
            &mut raw,
            || egui_state.clipboard_text(),
            self.demo.then_some(self.frame_count),
        );
        let mut inventory_drop = Vec::new();
        let mut library_drop = None;
        let t_pass = Instant::now();
        let mut windows_ms = 0.0;
        let mut full = ctx.run_ui(raw, |ui| {
            self.sound_cues.pass_start(ui.ctx());
            // before any field reads its keys: Ctrl+W is the frontmost window's
            ui::close_key::begin(ui.ctx());
            let t = Instant::now();
            let projector = ui::hud::Projector::for_ui(ui.ctx(), vp, [w as f32, h as f32]);
            actions = self.draw_ui(ui, &projector, &gfx.renderer);
            actions.ui_scale_changed |= ui::options::scale_keyboard(ui.ctx(), &mut self.settings.ui_scale);
            if inventory_drop.is_empty() {
                inventory_drop = self.inventory_drag_input(ui.ctx(), &gfx.renderer);
                if library_drop.is_none() {
                    library_drop = self.texture_library_drag_input(ui.ctx(), &gfx.renderer);
                }
            }
            // after every window: their text fields have been drawn
            self.text_menu.after(ui.ctx(), &self.palette);
            // the glass blurs the world: there is none behind the login screen
            if self.in_world() {
                ui::menu::glaze(ui.ctx(), &self.palette);
            }
            windows_ms += t.elapsed().as_secs_f32() * 1000.0;
        });
        // a capture leaves the user's layout as it is (as at exit)
        if self.capture.is_none() {
            self.layout_autosave.after_pass(&ctx, Instant::now());
        }
        if self.ik_editor.take_binding_paste_request() {
            let text = egui_state.clipboard_text().unwrap_or_default();
            self.ik_editor.paste_binding(&self.world, &text);
        }
        ui::chat::previews::clipboard_update(&ctx, &mut full.platform_output);
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
        for drop in inventory_drop {
            self.apply_inventory_drop(drop);
        }
        if let Some(drop) = library_drop {
            self.apply_texture_library_drop(drop);
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
        self.hover_tips.avatar = None;
        let tips_on = self.hover_tips_on();
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
                let ray = gfx.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1);
                let (point, idx) = self.scene.hover_pick(
                    &self.world,
                    ray,
                    gfx.renderer.hover_pick(self.cursor_pos.0, self.cursor_pos.1),
                    self.settings.draw_distance,
                );
                // the same pick gives the look-at its point
                self.look_hover = point.map(|p| (idx, p));
                // the hover tips name the avatars the search leaves out,
                // in front of what it found
                if tips_on && let Some(ray) = ray {
                    let far = point.map_or(self.settings.draw_distance, |p| (p - ray.0).length() + 0.05);
                    self.hover_tips.avatar = self.scene.avatar_on_ray(&self.world, ray, far);
                }
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
        self.track_toolbox_hover(&gfx.renderer);
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
                && self.huds_shown()
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
        if (self.color_pipette() || self.report_abuse.picking || self.particle_editor.picking) && !ctx.is_pointer_over_egui() {
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
            self.system_fonts_pending = !ui::fonts::apply(&self.egui_ctx, &ui_font(&self.settings));
        }
        if actions.ui_scale_changed {
            // Zoom changes no render/network setting or crash record field.
            // In particular, never fsync the crash snapshot during a drag.
            self.egui_ctx.set_zoom_factor(ui_scale(&self.settings));
        }
        crate::theme::apply_motion(&self.egui_ctx, self.settings.reduce_motion);
        if actions.settings_changed || std::mem::take(&mut self.snapshots.settings_changed) {
            aurora_i18n::select(&self.settings.language, crate::cli::get().language.as_deref());
            crate::crash::update(&self.skin.theme, &self.settings);
            self.net
                .set_agent_language(aurora_i18n::language(), self.settings.language_is_public);
            if self.settings.vsync != vsync {
                gfx.renderer.set_vsync(self.settings.vsync);
            }
            // a new colour blindness, contrast or interface colour: the
            // palette again, at once
            let vision = wanted_vision(&self.settings);
            let high_contrast = wanted_high_contrast(&self.settings);
            let interface_colors = wanted_interface_colors(&self.settings);
            if vision != crate::colorblind::current()
                || high_contrast != crate::high_contrast::current()
                || interface_colors != crate::interface_colors::current()
            {
                crate::colorblind::set(vision);
                crate::high_contrast::set(high_contrast);
                crate::interface_colors::set(interface_colors);
                self.palette = self.skin.theme.palette();
                self.cursors.set_palette(&self.palette);
            }
            self.skin.theme.apply(&self.egui_ctx, text_scale(&self.settings));
            ui::colors::set(&self.settings.colors);
            crate::keywords::set(&self.keyword_settings());
            crate::spelling::configure(&self.egui_ctx, &self.settings.spelling);
            crate::autoreplace::set_enabled(self.settings.auto_replace);
            crate::link_trust::configure(&self.settings.trusted_sites);
            crate::link_preview::configure_sites(&self.settings.link_previews.picture_sites, crate::demo::picture_sites());
            gfx.renderer.apply_settings(self.settings.render_settings());
            self.panels.time_of_day = self.settings.time_of_day;
            self.egui_ctx.set_zoom_factor(ui_scale(&self.settings));
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
                // Bug report form (disabled, see main.rs):
                // || self.bug_report.capture_pending
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
            // Bug report form (disabled, see main.rs):
            // && !self.bug_report.capture_pending
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
            // it until its client area already contains a rendered image;
            // shown once, with the focus (window_show.rs)
            crate::window_show::show(&gfx.window, gfx.maximize_on_show);
            log::info!("startup window shown after the first presented frame");
            crate::crash::watch_window(&gfx.window);
            crate::crash::demo_freeze();
            crate::crash::demo_thread_death();
        }
        if stats.presented && !self.update_acknowledged {
            self.update_acknowledged = true;
            aurora_updater::install::acknowledge_frame();
        }
        // (a render thread that panicked on the device's loss: its notice
        // first, app/gpu_stall.rs)
        if gfx.renderer.failed() && !gfx.renderer.device_lost() {
            crate::crash::failure("The render thread terminated unexpectedly.");
            // the render thread is gone (its panic is in the log): nothing
            // more can be shown, leave cleanly
            self.gfx = Some(gfx);
            self.shutdown(event_loop);
            return;
        }
        // a background thread the session cannot go on without panicked
        // (crash/threads.rs): leave cleanly, the crash window says which
        if let Some(thread) = crate::crash::thread_died() {
            log::error!("the {thread} thread stopped after a panic: ending the session");
            crate::crash::failure(&format!("The {thread} thread stopped after a panic."));
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
        if !crate::window_hold::paces(target) {
            // frame pacing (aurora-render pacing.rs): when the GPU sets the
            // pace, the next frame reads its input once the GPU has finished
            // the frame before this one, instead of waiting behind it with
            // stale input; then the frame limiter
            gfx.renderer.wait_for_gpu();
            if let Some(target) = target {
                wait_until(self.frame_start + target);
            }
        }
        self.frame_profile.lap(Lap::Limiter);
        // the graph's frame time: from one end of the limiter to the next,
        // the pace the frames really leave at. `dt` starts after the input
        // events: a click or a mouse move handled before one frame and not
        // the next made that one look shorter than the cap (spikes above
        // the frame rate limit in the Performance window's graph)
        let paced = self.frame_start.elapsed();
        self.frame_start = Instant::now();
        self.frame_count += 1;
        self.note_viewer_stats(paced, &gfx.renderer.info);
        if self.demo
            && self.frame_count == 1500
            && std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|v| matches!(v.as_str(), "image-photo" | "image-photo-save"))
        {
            let request = uuid::Uuid::new_v4();
            let item = uuid::Uuid::from_u128(8101);
            self.inventory_ui.thumbnail.pending = Some(request);
            self.inventory_image_action(request, item, ui::inventory::thumbnail::Input::Capture);
        }
        if self.demo
            && let Some(big) = crate::world::inventory::demo::big()
        {
            self.demo_big_inventory(big);
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
            for event in crate::demo::tags::chat_events(time, &mut self.demo_bubble_chat) {
                self.world.apply(event);
            }
        }
        if self.demo && std::env::var_os("AURORA_DEMO_ANIM_LOOP").is_some() {
            // acos(dot) amplifies f32 normalization error near identical poses.
            let phase_error = |a: &glam::Quat, b: &glam::Quat| {
                let relative = a.conjugate() * *b;
                2.0 * Vec3::new(relative.x, relative.y, relative.z).length().atan2(relative.w.abs())
            };
            if (crate::demo::animation_sync_demo() || crate::demo::animation_resync_demo()) && self.frame_count == 300 {
                let rig = self.scene.avatar_lib.rig.clone();
                self.scene.anims.insert(
                    crate::demo::SYNC_DELAYED_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::loop_animation(), &rig)),
                );
                log::info!("demo animation sync: delayed asset ready at frame 300");
            }
            if crate::demo::animation_resync_demo() {
                if self.frame_count == 750 {
                    let ctrl = self.ctrl;
                    self.ctrl = true;
                    self.on_input(Input::key(KeyCode::KeyS), true);
                    self.on_input(Input::key(KeyCode::KeyS), false);
                    self.ctrl = ctrl;
                    log::info!("demo animation resync: Ctrl+S at frame 750");
                }
                if matches!(self.frame_count, 720 | 780 | 900)
                    && let Some(&joint) = self.scene.avatar_lib.rig.names.get("mElbowLeft")
                {
                    let (leader, _) = self.scene.preview_pose(&crate::demo::DEMO_AGENT);
                    let (follower, _) = self
                        .scene
                        .preview_pose(&uuid::Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002));
                    if let Some((a, b)) = leader.get(joint).zip(follower.get(joint)) {
                        log::info!(
                            "demo animation resync: frame {}, elbow phase error {:.6} rad",
                            self.frame_count,
                            phase_error(a, b)
                        );
                    }
                }
            }
            if crate::demo::animation_sync_demo()
                && (self.frame_count == 360 || (crate::demo::animation_edit_demo() && self.frame_count == 660))
                && let Some(&joint) = self.scene.avatar_lib.rig.names.get("mElbowLeft")
            {
                let (leader, _) = self.scene.preview_pose(&crate::demo::DEMO_AGENT);
                let (follower, _) = self
                    .scene
                    .preview_pose(&uuid::Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002));
                if let Some((a, b)) = leader.get(joint).zip(follower.get(joint)) {
                    log::info!(
                        "demo animation sync: frame {}, elbow phase error {:.6} rad",
                        self.frame_count,
                        phase_error(a, b)
                    );
                }
            }
            if crate::demo::animation_edit_demo()
                && self.frame_count == 960
                && let Some(&joint) = self.scene.avatar_lib.rig.names.get("mHead")
            {
                let owners = [9200, 9201].map(|local| {
                    self.world
                        .objects
                        .iter()
                        .find_map(|(_, o)| (o.key.local_id == local).then_some(o.full_id))
                });
                if let [Some(edited), Some(reference)] = owners {
                    let (a, _) = self.scene.preview_pose(&edited);
                    let (b, _) = self.scene.preview_pose(&reference);
                    if let Some((a, b)) = a.get(joint).zip(b.get(joint)) {
                        log::info!("demo animation sync: animesh resume phase error {:.6} rad", phase_error(a, b));
                    }
                }
            }
            for event in crate::demo::loop_animation_events(self.frame_count) {
                self.world.apply(event);
            }
        }
        if self.demo && crate::demo::physics::mode().is_some() {
            for event in crate::demo::physics::animation_events(self.frame_count) {
                self.world.apply(event);
            }
            // the volumes' offsets, to read the bounce and its rest in the log
            if (230..=1200).contains(&self.frame_count) && self.frame_count.is_multiple_of(6) {
                let rig = &self.scene.avatar_lib.rig;
                let name = |j: usize| {
                    crate::demo::physics::BALLS
                        .iter()
                        .find(|b| rig.names.get(b.0) == Some(&j))
                        .map_or("?", |b| b.0)
                };
                for (who, id) in [("own", crate::demo::DEMO_AGENT), ("other", crate::demo::DEMO_LOUP)] {
                    let offsets: Vec<String> = self
                        .scene
                        .body_physics_offsets(&id)
                        .into_iter()
                        .map(|(j, o)| format!("{}=({:.4},{:.4},{:.4})", name(j), o.x, o.y, o.z))
                        .collect();
                    log::info!("demo physics: frame {} {who} {}", self.frame_count, offsets.join(" "));
                }
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
            && let Some(mode) = crate::demo::emote::mode()
        {
            for event in crate::demo::emote::events(self.frame_count, mode) {
                self.world.apply(event);
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
            if self.frame_count == 850 && mode == "object-history" {
                if let Some(id) = self.world.notifications.list.first().map(|n| n.id) {
                    let (commands, _) = self.world.respond_notification(id, crate::world::notifications::Response::Decline);
                    for command in commands {
                        self.send(command);
                    }
                }
                self.notif_ui.open = true;
                self.notif_ui.center.tab = ui::notification_center::Tab::History;
            }
            // the notice's attachment saved from its card
            if self.frame_count == crate::demo::notifs::SAVE_AT
                && mode == "group-notice-saved"
                && let Some(id) = self
                    .world
                    .notifications
                    .list
                    .iter()
                    .find(|n| n.offered_attachment().is_some())
                    .map(|n| n.id)
            {
                let (commands, _) = self.world.respond_notification(id, crate::world::notifications::Response::Accept);
                for command in commands {
                    self.send(command);
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
            && let Some(mode) = crate::demo::friend_prompt::mode()
        {
            let agent = self.world.agent_id;
            for event in crate::demo::friend_prompt::events(self.frame_count, agent) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
            if mode == "later" && self.frame_count == crate::demo::friend_prompt::LATER_AT {
                self.friend_prompt.put_off();
            }
        }
        if self.demo && crate::demo::region_message::enabled() {
            for event in crate::demo::region_message::events(self.frame_count) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
        }
        if self.demo
            && crate::demo::sim_version::mode().is_some()
            && let Some(handle) = self.world.main_region
        {
            for event in crate::demo::sim_version::events(self.frame_count, handle) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
        }
        if self.demo && crate::demo::object_im::settings().is_some() {
            let agent = self.world.agent_id;
            for event in crate::demo::object_im::events(self.frame_count, agent) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
        }
        if self.demo && crate::demo::freeze_world::on() {
            for event in crate::demo::freeze_world::events(self.frame_count) {
                self.world.apply(event);
            }
        }
        if self.demo && crate::demo::seat_kill::on() {
            for event in crate::demo::seat_kill::events(self.frame_count) {
                self.world.apply(event);
            }
        }
        if self.demo
            && let Some((tab, end)) = crate::demo::about::tab()
        {
            if self.frame_count == crate::demo::about::OPEN {
                self.about.ui.open(&self.egui_ctx, tab, &mut self.panels.about);
            }
            // until the computer is read and the cards have their height
            self.about.ui.scroll_to_end = end && self.frame_count.saturating_sub(crate::demo::about::OPEN) < 60;
        }
        if self.demo
            && let Some(scenario) = crate::demo::stream_title::scenario()
        {
            let title = crate::demo::stream_title::title(self.frame_count, scenario);
            self.stream_titles.update(title, self.settings.audio.stream_title_history as usize);
            if self.frame_count == crate::demo::stream_title::OPEN {
                self.settings.audio.stream_title_text_only = scenario.text_only;
                ui::widgets::open_floater(&self.egui_ctx, ui::stream_title::WINDOW, &mut self.panels.stream_title);
                if scenario.history {
                    ui::widgets::open_floater(&self.egui_ctx, ui::stream_title::HISTORY, &mut self.panels.stream_title_history);
                }
            }
        }
        if self.demo && crate::demo::collisions::mode().is_some() {
            for event in crate::demo::collisions::events(self.frame_count) {
                self.world.apply(event);
            }
            if self.frame_count == crate::demo::collisions::OPEN {
                ui::widgets::open_floater(&self.egui_ctx, ui::bumps::WINDOW, &mut self.panels.bumps);
            }
        }
        if self.demo && crate::demo::presence::position().is_some() {
            let agent = self.world.agent_id;
            for event in crate::demo::presence::events(self.frame_count, agent) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
            for (open, close) in crate::demo::presence::LIST {
                if self.frame_count == open || self.frame_count == close {
                    self.notif_ui.open = self.frame_count == open;
                }
            }
        }
        if self.demo
            && let Some(mode) = crate::demo::attention::mode()
        {
            if self.frame_count == crate::demo::attention::MINIMIZE_AT
                && mode != "front"
                && let Some(g) = &self.gfx
            {
                g.window.set_minimized(true);
            }
            if self.frame_count == crate::demo::attention::RESTORE_AT
                && mode == "capture"
                && let Some(g) = &self.gfx
            {
                g.window.set_minimized(false);
                g.window.focus_window();
                self.notif_ui.open = true;
            }
            let agent = self.world.agent_id;
            for event in crate::demo::attention::events(self.frame_count, agent) {
                log::info!(
                    "demo attention: arrival at frame {}, behind {}",
                    self.frame_count,
                    self.window_behind()
                );
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
        }
        if self.demo && crate::demo::keywords::active() {
            let agent = self.world.agent_id;
            for event in crate::demo::keywords::events(self.frame_count, agent) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
        }
        if self.demo && crate::demo::conv_button::mode().is_some() {
            let agent = self.world.agent_id;
            for event in crate::demo::conv_button::events(self.frame_count, agent) {
                if let Some(e) = self.world.apply(event) {
                    self.on_app_event(e);
                }
            }
        }
        if self.demo
            && let Some(mode) = crate::demo::delivery::mode()
        {
            use crate::demo::delivery::{ARRIVE_AT, READ_AT, SEED_AT};
            let agent = self.world.agent_id;
            if self.frame_count == SEED_AT {
                crate::world::inventory::demo::seed_received(&mut self.world.inventory, agent);
                // the inbox already held its purchase: the badge's baseline, not news
                let _ = self.inventory_ui.update_received(&mut self.world.inventory, Vec::new());
            }
            if self.frame_count == ARRIVE_AT
                && let Some(event) = crate::demo::delivery::arrival(&self.world.inventory, agent)
                && let Some(e) = self.world.apply(event)
            {
                self.on_app_event(e);
            }
            let card = (self.frame_count == READ_AT)
                .then(|| {
                    self.world.notifications.list.iter().find_map(|n| match n.data {
                        crate::world::notifications::Data::Delivery { object } => Some((n.id, object)),
                        _ => None,
                    })
                })
                .flatten();
            match (mode.as_str(), card) {
                ("open", Some((id, object))) => self.show_delivery(id, object),
                ("history", card) if self.frame_count == READ_AT => {
                    if let Some((id, _)) = card {
                        let _ = self.world.respond_notification(id, crate::world::notifications::Response::Dismiss);
                    }
                    self.notif_ui.open = true;
                    self.notif_ui.center.tab = ui::notification_center::Tab::History;
                }
                _ => {}
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
            if self.frame_count == crate::demo::toasts::LIST_AT && (mode == "1" || mode == "bell") {
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
        // Bug report form (disabled, see main.rs):
        // if self.bug_report.capture_pending
        //     && let Some((w, h, pixels)) = gfx.renderer.captured.take()
        // {
        //     self.bug_report.captured(w, h, pixels);
        // }
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
        let mut net = self.net.stats.snapshot();
        if self.demo {
            ui::perf::demo_traffic(&mut net);
        }
        self.perf
            .frame(paced.as_secs_f32().min(0.1) * 1000.0, net, &self.last_render, &self.scene.stats);
        self.frame_profile.lap(Lap::Tail);
        self.end_profile_frame(w, h);
        self.end_deep_profile(&gfx.renderer);
        self.monitor_frame(&gfx.renderer);
        self.crash_state(&gfx.renderer, &gfx.window);
        self.gpu_stall_frame(&gfx.renderer);
        self.gfx = Some(gfx);
        if std::mem::take(&mut self.gpu_stall.close) {
            self.close_while_gpu_stalled(event_loop);
            return;
        }
        self.release_postponed_cursor();
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
            self.demo_avatar_travel_steps();
        }
        self.demo_hud_drag_steps();
        // AURORA_DEMO_INVENTORY=offer|offer-reject: the sample offer's item
        // shown in the folder it reaches; the second rejects it at frame 700
        // (offer-card…: answered from its card or its notification, the
        // conversation in sight; …-show: kept, the inventory opening on it)
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_INVENTORY")
            && mode.starts_with("offer")
        {
            let card = crate::demo::offer_card_scene();
            if self.frame_count == 700
                && card.is_some_and(|s| s.keep)
                && let Some(offer) = self.world.inventory_offers.list.first().map(|v| v.id)
            {
                self.on_ctx_action(ui::context::CtxAction::AnswerInventoryOffer { offer, keep: true });
            }
            // offer-show|offer-object-show: kept from the bell's entry
            let bell = match mode.as_str() {
                "offer-reject" => Some(crate::world::notifications::Response::Decline),
                "offer-show" | "offer-object-show" => Some(crate::world::notifications::Response::Accept),
                _ => card
                    .is_some_and(|s| s.reject)
                    .then_some(crate::world::notifications::Response::Decline),
            };
            if self.frame_count == 700
                && let Some(r) = bell
            {
                let offer = self
                    .world
                    .notifications
                    .list
                    .iter()
                    .find(|n| n.kind == crate::world::notifications::Kind::Inventory);
                if let Some(id) = offer.map(|n| n.id) {
                    for cmd in self.world.respond_notification(id, r).0 {
                        self.send(cmd);
                    }
                }
            }
            let folder = self.world.inventory.items.get(&crate::demo::DEMO_OFFER).map(|it| it.parent);
            if card.is_none() && !crate::demo::show_accepted_scene() && folder.is_some() && folder != self.demo_offer_shown {
                self.demo_offer_shown = folder;
                self.inventory_ui.show_original(&self.world.inventory, crate::demo::DEMO_OFFER);
            }
        }
        self.demo_worn_steps();
        self.demo_derender_steps();
        self.demo_social_menu_steps();
        self.demo_land_moderation();
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
        // AURORA_DEMO_VOICE_MENU=1|louder|muted: the world menu of the nearest
        // name tag of a resident in our voice channel at frame 600, its voice
        // row first; `louder` sets their volume to 160 % before, `muted`
        // blocks their voice (as the row's button does)
        if self.demo
            && self.frame_count == 600
            && let Ok(mode) = std::env::var("AURORA_DEMO_VOICE_MENU")
        {
            // (not the demo's blocked wolf: his voice is muted with the rest)
            let w = &self.world;
            let tag = self
                .name_tags
                .iter()
                .rev()
                .find(|t| t.id != w.agent_id && w.calls.hears(&t.id) && !w.voice_blocked(&t.id))
                .map(|t| (t.id, t.rect.center()));
            match tag {
                Some((id, at)) => {
                    match mode.trim() {
                        "louder" => self.world.calls.set_volume(id, 0.8),
                        "muted" => {
                            let name = self.world.legacy_name(&id).unwrap_or_else(|| self.world.social.name_of(&id));
                            let voice = crate::world::mutes::flag::VOICE_CHAT;
                            if let Err(e) = self.world.block_flags(id, &name, crate::world::mutes::MuteType::Agent, voice) {
                                log::warn!("demo voice menu: {e}");
                            }
                        }
                        _ => {}
                    }
                    let ppp = self.egui_ctx.pixels_per_point();
                    self.cursor_pos = (at.x * ppp, at.y * ppp);
                    self.open_context_menu();
                }
                None => log::warn!("demo voice menu: no name tag of a resident in voice on screen"),
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
            if std::env::var("AURORA_DEMO_CONTACTS").as_deref() == Ok("empty") {
                self.world.social.friends.clear();
            }
        }
        // AURORA_DEMO_PEOPLE's tabs: the mock-up's sets, pins and recent people
        if self.demo && self.frame_count == 60 {
            crate::demo::people::seed(&mut self.world);
            crate::demo::friend_location(&mut self.world);
        }
        if self.demo && matches!(self.frame_count, 700 | 720) && crate::demo::friend_join_scenario() {
            if self.frame_count == 700 {
                let contact = std::env::var("AURORA_DEMO_FRIEND_LOCATION").as_deref() != Ok("join-map");
                self.world.locate_avatar(crate::demo::DEMO_FRIEND, contact);
                self.panels.world_map = true;
            } else if std::env::var("AURORA_DEMO_FRIEND_LOCATION").as_deref() == Ok("join-map")
                && let Some(t) = &self.world.map.track
                && !t.pending
                && !t.invalid
            {
                self.world.map.push(crate::world::worldmap::teleport_command(t.x, t.y, t.z));
            }
        }
        if self.demo
            && self.demo_friend_join_target.as_ref().is_some_and(|(at, _)| Instant::now() >= *at)
            && let Some((_, position)) = self.demo_friend_join_target.take()
            && let Some(handle) = self.world.main_region
        {
            if crate::demo::friend_join_flight_scenario()
                && let Some(info) = self.world.regions.values().find_map(|r| r.info.as_ref()).cloned()
            {
                let mut info = (*info).clone();
                info.handle = handle;
                info.is_main = true;
                let (x, y) = aurora_net::handle_to_origin(handle);
                if let Some(sim) = self.world.map.sims.get(&(x / 256, y / 256)) {
                    info.name.clone_from(&sim.name);
                }
                info.region_flags &= !aurora_net::region_flags::BLOCK_FLY;
                if std::env::var("AURORA_DEMO_FRIEND_LOCATION").as_deref() == Ok("join-fly-blocked") {
                    info.region_flags |= aurora_net::region_flags::BLOCK_FLY;
                }
                self.world.apply(NetEvent::RegionHandshake(Arc::new(info)));
                let late = crate::demo::friend_join_late_flight_scenario();
                let parcel_position = if late { Vec3::new(100.0, 100.0, 0.0) } else { position };
                self.world.apply(NetEvent::AgentParcel(Arc::new(crate::demo::friend_join_parcel(
                    parcel_position,
                    !late,
                ))));
            }
            let mut target = crate::demo::action_avatar(false);
            target.full_id = crate::demo::DEMO_FRIEND;
            target.position = position;
            if crate::demo::friend_join_custom_flight_scenario() {
                target.foot_plane = Some(glam::Vec4::new(0.0, 0.0, 1.0, 20.0));
                target.velocity = Vec3::ZERO;
                target.acceleration = Vec3::ZERO;
            }
            self.world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![target],
            });
            let state = if crate::demo::friend_join_custom_flight_scenario() {
                crate::ao::State::Typing
            } else if crate::demo::friend_join_flight_scenario() && !crate::demo::friend_join_late_flight_scenario() {
                crate::ao::State::Hovering
            } else {
                crate::ao::State::Standing
            };
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: crate::demo::DEMO_FRIEND,
                anims: vec![(state.motion(), 1)],
                sources: vec![],
            });
            log::info!("demo friend join: fresh target position at {:.0} m", position.z);
            if crate::demo::friend_join_late_flight_scenario() {
                self.demo_friend_join_flight = Some((Instant::now() + Duration::from_millis(400), false));
            }
        }
        if self.demo
            && self.demo_friend_join_flight.as_ref().is_some_and(|(at, _)| Instant::now() >= *at)
            && let Some((_, parcel)) = self.demo_friend_join_flight.take()
        {
            if parcel {
                self.world.apply(NetEvent::AgentParcel(Arc::new(crate::demo::friend_join_parcel(
                    Vec3::new(60.0, 200.0, 2400.0),
                    true,
                ))));
                log::info!("demo friend join: delayed landing parcel allows flight");
            } else {
                self.world.apply(NetEvent::AvatarAnimations {
                    avatar: crate::demo::DEMO_FRIEND,
                    anims: vec![(crate::ao::State::Hovering.motion(), 1)],
                    sources: vec![],
                });
                log::info!("demo friend join: delayed hover signal");
                self.demo_friend_join_flight = Some((Instant::now() + Duration::from_millis(400), true));
            }
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
        self.demo_sit_teleport();
        if let (Some(demo), Some(g)) = (&mut self.geometry_demo, &mut self.gfx) {
            demo.tick(self.frame_count, &mut g.renderer);
        }
        if let Some(demo) = &mut self.turn_demo {
            self.world.agent.yaw += demo.tick(self.frame_count);
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
        if let Some(demo) = &mut self.arrival_demo {
            demo.tick(self.frame_count, &mut self.world, &self.scene);
        }
        if let Some(demo) = &mut self.outfit_restore_demo {
            demo.tick(self.frame_count, &mut self.world, Instant::now());
            if demo.restore_over(&self.world) {
                self.panels.appearance = true;
                self.appearance_ui.open(2, false);
            }
        }
        if let (Some(demo), Some(g)) = (&mut self.rez_demo, &mut self.gfx) {
            demo.tick(self.frame_count, &mut self.world, &mut self.scene, &mut g.renderer);
        }
        // AURORA_DEMO_TEXREFRESH: a panel with a stale cached texture
        if let Some(demo) = &mut self.tex_refresh_demo
            && let Some(object) = demo.tick(self.frame_count, &mut self.world, self.scene.textures.cache_dir())
        {
            self.refresh_object_textures(object);
        }
        if let Some(demo) = &mut self.sculpt_missing_demo {
            demo.tick(self.frame_count, &mut self.world, &mut self.scene.textures);
        }
        if let Some(demo) = &mut self.texture_missing_demo {
            demo.tick(self.frame_count, &mut self.world, &mut self.scene.textures);
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
            && let Some(events) = crate::demo::away_teleport(self.frame_count, &self.world, &mut self.away_coarse)
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
            self.demo_buy_pass();
            self.demo_pay();
            self.demo_money_tracker();
            self.demo_payment();
        }
        // AURORA_DEMO_LOOKAT=1: our eye tracking on (cursor on the left of
        // the window, moving), Violet Wolf looks at a point north of him;
        // =own: the crosshairs shown, ours too, the real cursor; =own,x,y:
        // the cursor at (x, y), fractions of the window, moving by a pixel
        // (the head follows a cursor that moves); =targets: own
        // with every other avatar looking somewhere new every 2 s; =leave
        // (or leave,x,y): own, Violet Wolf looks at us, then logs off
        if self.demo
            && let Ok(value) = std::env::var("AURORA_DEMO_LOOKAT")
        {
            let mut parts = value.split(',');
            let mode = parts.next().unwrap_or_default().trim();
            let pinned: Vec<f32> = parts.filter_map(|v| v.trim().parse().ok()).collect();
            self.demo_look_at = true;
            self.demo_look_at_targets = matches!(mode, "own" | "targets" | "leave");
            if mode == "targets" && self.frame_count >= 200 && self.frame_count.is_multiple_of(120) {
                self.demo_look_at_everyone();
            }
            if mode == "leave" {
                self.demo_look_at_leave();
            }
            // the cursor is pinned for captures; "own", "targets" and
            // "leave" leave the real one alone (where the human aims)
            let pin = match (mode, pinned.as_slice()) {
                ("own" | "leave", [x, y]) => Some((*x, *y, (self.frame_count % 2) as f32)),
                ("own" | "targets" | "leave", _) => None,
                _ => Some((0.15, 0.3, (self.frame_count % 2) as f32)),
            };
            if let (Some((x, y, wiggle)), Some(g)) = (pin, self.gfx.as_ref()) {
                let (w, h) = g.renderer.size();
                self.cursor_pos = (w as f32 * x + wiggle, h as f32 * y);
            }
            // ",side": from frame 300 the camera beside our head, aimed at a
            // point in front of the face (which we then look at: a focus),
            // to see where the line leaves the head
            // "ahead": from frame 300 the camera in front of our face, our
            // eyes on a point far to our left (a focus, which never times
            // out), then from frame 500 "no look-at target, even for me":
            // head and eyes come back straight ahead
            if mode == "ahead" {
                self.demo_look_ahead = self.frame_count >= 500;
                if (300..500).contains(&self.frame_count)
                    && let (Some(eyes), Some((mx, my))) = (
                        self.scene.eyes_world(&self.world, &self.world.agent_id, Instant::now()),
                        self.world.main_origin(),
                    )
                {
                    let rot = self.world.agent.body_rotation();
                    if self.frame_count == 300 {
                        self.camera
                            .set_view(eyes + rot * Vec3::X * 0.7, eyes, &mut self.world.agent, &self.settings.camera);
                    }
                    let p = eyes + rot * Vec3::new(1.0, 2.5, 0.0);
                    let g = glam::DVec3::new(mx as f64 + p.x as f64, my as f64 + p.y as f64, p.z as f64);
                    self.world.look_at.own.set(crate::world::lookat::FOCUS, None, g, Instant::now());
                }
            }
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
                && mode != "leave"
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
        self.demo_altgr_shortcuts();
        // AURORA_DEMO_KEY="down" (up, left, right): hold that arrow from frame
        // 235, log the agent and drawn body headings every 30 frames;
        // "up:pulse": 2 s held, 2 s released, over and over (by the clock,
        // the same at any frame rate)
        if self.demo
            && self.frame_count >= 235
            && let Ok(v) = std::env::var("AURORA_DEMO_KEY")
        {
            let (name, pulse) = v
                .split_once(':')
                .map_or((v.as_str(), false), |(name, mode)| (name, mode == "pulse"));
            let key = match name {
                "up" => KeyCode::ArrowUp,
                "left" => KeyCode::ArrowLeft,
                "right" => KeyCode::ArrowRight,
                _ => KeyCode::ArrowDown,
            };
            static PULSE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let since = PULSE.get_or_init(Instant::now).elapsed().as_secs();
            if pulse && !(since / 2).is_multiple_of(2) {
                self.down.remove(&Input::key(key));
            } else {
                self.down.insert(Input::key(key));
            }
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
        // AURORA_DEMO_ENV_KEYS="u" (y, n, x, j): Ctrl+Shift + that letter at
        // frame 300, through the shortcut table, as World › Environment's
        // shortcuts (#1264)
        if self.demo
            && self.frame_count == 300
            && let Ok(v) = std::env::var("AURORA_DEMO_ENV_KEYS")
        {
            let code = match v.as_str() {
                "u" => KeyCode::KeyU,
                "n" => KeyCode::KeyN,
                "x" => KeyCode::KeyX,
                "j" => KeyCode::KeyJ,
                _ => KeyCode::KeyY,
            };
            let mods = Mods {
                ctrl: true,
                shift: true,
                alt: false,
            };
            let action = self.settings.keybinds.triggered(&Input::key(code), mods);
            if let Some(action) = action {
                self.env_shortcut(action);
            }
            log::info!(
                "demo env keys: Ctrl+Shift+{v} -> {action:?}, time of day {}",
                self.panels.time_of_day
            );
        }
        if self.demo {
            self.particle_editor_demo();
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
                        crate::build::demo::apply_handle_size(&mut self.settings.build);
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
                    crate::build::demo::Step::CreationPreview(shape) => {
                        self.settings.build.create_shape = shape;
                        self.settings.build.new_prim_size = [2.0; 3];
                        self.settings.build.copy_selection = false;
                        let point = self.world.agent.position + Vec3::new(5.0, 3.0, 0.0);
                        let point = Vec3::new(point.x, point.y, crate::demo::floor_at(point.x, point.y));
                        self.camera.set_view(
                            point + Vec3::new(-4.0, -6.0, 3.0),
                            point + Vec3::Z,
                            &mut self.world.agent,
                            &self.settings.camera,
                        );
                        self.panels.inventory = false;
                        self.panels.chat = false;
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
                    crate::build::demo::Step::ContentsProperties => {
                        if let Some(idx) = self.build.contents_target(&self.world)
                            && let Some(object) = self.world.objects.get(idx).map(|o| o.full_id)
                            && let Some(item) = self
                                .build
                                .contents
                                .objects
                                .get(&object)
                                .and_then(|inv| inv.items.iter().find(|it| !it.is_folder))
                                .cloned()
                        {
                            self.build.open_task_item_properties(object, &item);
                        }
                    }
                    crate::build::demo::Step::ContentsSelection(rename) => {
                        if let Some(idx) = self.build.contents_target(&self.world)
                            && let Some(object) = self.world.objects.get(idx).map(|o| o.full_id)
                            && let Some(inv) = self.build.contents.objects.get(&object)
                        {
                            let mut items: Vec<_> = inv.items.iter().filter(|it| !it.is_folder).collect();
                            items.sort_by_key(|it| it.name.to_lowercase());
                            let ids: Vec<_> = items.iter().take(if rename { 1 } else { 2 }).map(|it| it.item_id).collect();
                            self.build.ui.contents_selection.items = ids.iter().copied().collect();
                            if rename && let Some(id) = ids.first() {
                                self.build.begin_contents_rename(&self.world, object, *id);
                            }
                        }
                    }
                    crate::build::demo::Step::TexturePicker(materials, local) => {
                        let current = self
                            .build
                            .sel_te(&self.world)
                            .first()
                            .and_then(|&(idx, face)| self.world.objects.get(idx)?.te.as_ref().map(|te| te.face(face as usize).texture))
                            .unwrap_or_default();
                        if materials {
                            self.build.ui.picker.open_materials(
                                aurora_i18n::tr!("build-ui-texture-choose-material"),
                                crate::demo::pbr_materials().first().map(|(id, _)| *id).unwrap_or_default(),
                            );
                            self.build.ui.pick_target = Some(crate::build::ui::PickTarget::PbrMaterial);
                            self.build.ui.tex_tab = 1;
                        } else {
                            self.build.ui.picker.open_with_defaults(
                                aurora_i18n::tr!("build-ui-texture-choose-texture"),
                                current,
                                Some(aurora_prim::te::DEFAULT_TEXTURE),
                                false,
                            );
                            self.build.ui.pick_target = Some(crate::build::ui::PickTarget::Diffuse);
                        }
                        if local {
                            self.build.ui.picker.show_local_tab();
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
                    crate::build::demo::Step::TakeAll => {
                        if let Some(idx) = self.build.contents_target(&self.world)
                            && let Some(object) = self.world.objects.get(idx).map(|o| o.full_id)
                            && let Some(inv) = self.build.contents.objects.get(&object)
                        {
                            let mut items: Vec<_> = inv.items.iter().filter(|it| !it.is_folder).cloned().collect();
                            items.sort_by_key(|it| it.name.to_lowercase());
                            self.build.ui.contents_selection.items = items.iter().map(|it| it.item_id).collect();
                            // what a drop of the selection on the folder sends (task_drop::target)
                            for item in items {
                                self.apply_inventory_action(ui::inventory::InvAction::TakeTask(Box::new(
                                    crate::build::task_inventory::TransferRequest {
                                        object,
                                        item,
                                        folder: self.world.inventory.root,
                                        approved: None,
                                        ignore_warning: false,
                                    },
                                )));
                            }
                        }
                    }
                    crate::build::demo::Step::CloseEditor => self.build.close(&mut self.world, &mut self.settings.build),
                    crate::build::demo::Step::Select(idx) => {
                        self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                        self.flush_build();
                    }
                    crate::build::demo::Step::SharedRights(idx, granted) => {
                        crate::build::demo::shared_rights(&mut self.world, idx, granted);
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
                            self.panels.chat = false;
                        }
                    }
                    crate::build::demo::Step::Linked(on) => {
                        self.build.set_edit_linked(&mut self.world, &mut self.settings.build, on);
                        self.flush_build();
                    }
                    crate::build::demo::Step::MeshLod(level) => {
                        self.panels.chat = false;
                        self.build.set_inspected_mesh_lod(level);
                    }
                    crate::build::demo::Step::Physics(show, kind) => {
                        self.settings.build.show_physics_shape = show;
                        self.build.set_physics(&self.world, |p| p.shape_type = kind);
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
                self.media.on_key(
                    &self.world,
                    code,
                    0x1E,
                    k.is_multiple_of(2),
                    false,
                    k.is_multiple_of(2).then_some(ch),
                    mods,
                );
            }
        }
        // AURORA_DEMO_CAMERA="alt,x,y" (pan, zoom, ml, wheel, fly): camera/demo.rs
        if self.demo
            && let Ok(v) = std::env::var("AURORA_DEMO_CAMERA")
        {
            self.demo_camera_steps(&v);
        }
        // AURORA_DEMO_MMO="x,y[,flags]": left press once the world shows
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
            let flag = |name| v.split(',').any(|s| s.trim() == name);
            let f = self.demo_mmo_start.map_or(u64::MAX, |s| self.frame_count - s);
            match (f, c.len()) {
                (0, n) if n >= 2 => {
                    if flag("world") || flag("world-off") {
                        self.settings.world_mouse_walk = flag("world");
                    }
                    self.cursor_pos = (c[0], c[1]);
                    if flag("right-before-left") {
                        self.mouse_input(MouseButton::Right, true, false);
                    }
                    self.mouse_input(MouseButton::Left, true, false);
                    if flag("quick") {
                        // Queue the chord and both releases behind the deferred pick.
                        self.mouse_input(MouseButton::Right, true, false);
                        self.mouse_input(MouseButton::Left, false, false);
                        self.mouse_input(MouseButton::Right, false, false);
                    }
                    self.demo_note(click_pick::DemoNote::Mmo);
                }
                (10, _) if flag("quick") || flag("right-before-left") => {}
                // third value 1: hold the right arrow instead of the right button
                (10, _) if c.get(2) == Some(&1.0) => {
                    self.demo_release_keyboard();
                    self.down.insert(Input::key(KeyCode::ArrowRight));
                }
                // third value 3: hold the down arrow (backing up must not turn, #1259)
                (10, _) if c.get(2) == Some(&3.0) => {
                    self.demo_release_keyboard();
                    self.down.insert(Input::key(KeyCode::ArrowDown));
                    log::info!(
                        "demo mmo: backing up from pos {:?}, yaw {:.2}",
                        self.world.agent.position,
                        self.world.agent.yaw
                    );
                }
                // third value 2: double right click, held the second time (run)
                (11, _) if c.get(2) == Some(&2.0) => self.mouse_input(MouseButton::Right, false, false),
                (12, _) if c.get(2) == Some(&2.0) => self.mouse_input(MouseButton::Right, true, false),
                (10, _) => self.mouse_input(MouseButton::Right, true, false),
                (20, _) => self.demo_note(click_pick::DemoNote::Mmo),
                (100, _) if flag("left-first") => self.mouse_input(MouseButton::Left, false, false),
                (105, _) => {
                    let running = self.temp_run.is_some();
                    if self.right_drag {
                        self.mouse_input(MouseButton::Right, false, false);
                    }
                    self.down.remove(&Input::key(KeyCode::ArrowRight));
                    self.down.remove(&Input::key(KeyCode::ArrowDown));
                    log::info!(
                        "demo mmo: after walking, mode {:?}, pos {:?}, yaw {:.2}, camera yaw {:.2}, running {running} (after release {})",
                        self.mouse_mode,
                        self.world.agent.position,
                        self.world.agent.yaw,
                        self.world.agent.yaw + self.camera.orbit_yaw,
                        self.temp_run.is_some()
                    );
                }
                (110, _) => self.mouse_input(MouseButton::Left, false, false),
                (115, _) => self.demo_note(click_pick::DemoNote::Mmo),
                (120, _) if flag("menu-after") => self.mouse_input(MouseButton::Right, true, false),
                (121, _) if flag("menu-after") => self.mouse_input(MouseButton::Right, false, false),
                (130, _) if flag("menu-after") => self.demo_note(click_pick::DemoNote::Mmo),
                _ => {}
            }
        }
    }

    /// Demo keys reach the avatar: the text field the demo windows opened
    /// with (the conversation's chat input) lets the keyboard go.
    fn demo_release_keyboard(&self) {
        self.egui_ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
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
            // AURORA_MATERIAL_REBUILD=alt: every material user rebuilt at a
            // material change one second in three
            if let Some(old) = crate::scene::mat_changes::alternate(self.sync_alt) {
                self.frame_profile.set_mark(if old { " | xmode=old" } else { " | xmode=new" });
            }
            // AURORA_IMPOSTOR_POSES=alt: impostors posed at every frame in the
            // same seconds
            if let Some(old) = crate::scene::impostors::alternate_poses(self.sync_alt) {
                self.frame_profile.set_mark(if old { " | xmode=old" } else { " | xmode=new" });
            }
            // AURORA_PROBE_WALK=alt: the whole probe list walked one second in three
            if aurora_render::probes::ProbeWalk::from_env() == aurora_render::probes::ProbeWalk::Alternate {
                let old = self.frame_profile.probe_walk_list();
                self.frame_profile.set_mark(if old { " | xmode=old" } else { " | xmode=new" });
            }
            // AURORA_FRAME_PRIORITY=alt: the frame's threads at normal
            // priority one second in three
            if aurora_render::frame_priority::PriorityMode::from_env() == aurora_render::frame_priority::PriorityMode::Alternate {
                let old = self.sync_alt.is_multiple_of(3);
                aurora_render::frame_priority::set_raised(!old);
                self.frame_profile.set_mark(if old { " | xmode=old" } else { " | xmode=new" });
            }
            // AURORA_RIGGED_BLEND=alt: the rigged blended faces drawn as
            // `runs` (#966) one second in three
            if crate::scene::blend_order::mode() == crate::scene::blend_order::Mode::Alternate {
                let old = self.sync_alt.is_multiple_of(3);
                crate::scene::blend_order::set_old_now(old);
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

    /// AURORA_PROFILE: the frame's share of the lines written every 10 s
    /// (deep_profile.rs), and what only their end reads.
    fn end_deep_profile(&mut self, renderer: &aurora_render::Renderer) {
        self.monitor_feed.upload_bytes = self.scene.textures.stats.uploaded_bytes_frame;
        // (the probes are read once a second for the resource monitor too)
        if !self.deep_profile.tick() {
            return;
        }
        let in_world = self.in_world();
        let texture_upload_bytes = std::mem::take(&mut self.scene.textures.stats.uploaded_bytes_frame);
        let st = &self.scene.stats;
        let tex = &self.scene.textures.stats;
        let frame = crate::deep_profile::FrameSample {
            texture_upload_bytes,
            decoding: tex.decoding,
            pending_upload: tex.pending_upload,
            jobs: st.jobs,
            geom_pending: st.geom_pending,
            meshes_fetching: self.scene.meshes.fetching,
            avatars: if in_world { st.avatars } else { Default::default() },
            posed: if in_world { st.posed } else { 0 },
            skeletons_rebuilt: if in_world { st.skeletons_rebuilt } else { 0 },
        };
        let (scene, render, net) = (&self.scene, &self.last_render, &self.net);
        self.deep_profile.end_frame(frame, || {
            let tex = &scene.textures.stats;
            let geometry_used = render.vertex_used
                * (std::mem::size_of::<aurora_render::Vertex>() + std::mem::size_of::<aurora_render::SkinVertex>()) as u64
                + render.index_used * 2;
            crate::deep_profile::PeriodSample {
                avatars: scene.avatar_sample(),
                gpu: crate::deep_profile::GpuMemory {
                    video: renderer.video_memory.read(&renderer.device),
                    textures: render.textures,
                    texture_bytes: render.texture_bytes,
                    pages: render.texture_pages,
                    page_bytes: render.texture_page_bytes,
                    geometry_bytes: render.geometry_bytes,
                    geometry_used,
                    texture_target: tex.target_bytes,
                    pressure: tex.pressure,
                },
                process: crate::sys_probe::process_memory(),
                texture_cache: scene.textures.cache_counts(),
                geometry_uploaded: renderer.geometry_uploaded,
                http_queued: net.stats.http_queued.load(std::sync::atomic::Ordering::Relaxed),
            }
        });
    }

    /// Local chat from the chat bar or the conversations window: a chat bar
    /// command (FSCmdLine) or a line sent on its channel.
    fn send_local_chat(&mut self, mut c: ui::chat::OutgoingChat) {
        if c.channel == 0 {
            // the MU* pose first, then the gesture (FSNearbyChat::sendChat)
            if self.settings.colon_emote {
                ui::chat::colon_emote(&mut c.message);
            }
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

    /// Our typing in the nearby chat, once per frame after its fields are
    /// drawn (`ChatUi::local_typing_update`).
    fn update_local_typing(&mut self) {
        self.chat_ui.type_during_emote = self.settings.type_during_emote;
        self.chat_ui.colon_emote = self.settings.colon_emote;
        if !self.world.rlva.can_send_typing_start() {
            self.stop_local_typing();
            return;
        }
        // AURORA_DEMO_TYPING: we type from our arrival on, no key needed
        if self.demo && crate::demo::typing_scenario() && matches!(self.screen, Screen::World) {
            if self.chat_ui.hold_local_typing(Instant::now()) {
                self.send_local_typing(true);
            }
            return;
        }
        if let Some(typing) = self.chat_ui.local_typing_update(Instant::now()) {
            self.send_local_typing(typing);
        }
    }

    /// LLAgent::startTyping / stopTyping: the typing animation (started
    /// only with PlayTypingAnim, always stopped), then the empty typing
    /// notice on channel 0 for those in range. AGENT_STATE_TYPING follows
    /// in the agent updates (`ChatUi::local_typing`).
    fn send_local_typing(&mut self, typing: bool) {
        if typing && !self.world.rlva.can_send_typing_start() {
            return;
        }
        log::info!("nearby chat: typing {}", if typing { "started" } else { "stopped" });
        if !typing || self.settings.play_typing_anim {
            self.send(NetCommand::AgentAnimation {
                anim: crate::world::radar::ANIM_AGENT_TYPE,
                start: typing,
            });
        }
        self.send(NetCommand::Chat {
            message: String::new(),
            channel: 0,
            chat_type: if typing {
                aurora_net::ChatType::StartTyping
            } else {
                aurora_net::ChatType::StopTyping
            },
        });
    }

    /// A teleport or an arrival ends our typing in the nearby chat (FIRE-7273
    /// in process_teleport_finish / process_agent_movement_complete).
    fn stop_local_typing(&mut self) {
        if self.chat_ui.stop_local_typing() {
            self.send_local_typing(false);
        }
    }

    /// "Report a bug" (login screen, top bar, Help menu): GitHub's issue
    /// template chooser in the browser, in place of the built-in form
    /// (disabled, see main.rs).
    fn open_issue_chooser(&self) {
        log::info!("bug report: opening the GitHub issue chooser");
        self.egui_ctx.open_url(egui::OpenUrl::new_tab(crate::links::NEW_ISSUE));
    }

    /// The bug button's right click (login screen, top bar): the version a
    /// report asks for, to the clipboard with no notice.
    fn copy_version(&self) {
        log::info!("bug report: version copied to the clipboard");
        self.egui_ctx.copy_text(aurora_version::version().to_owned());
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
            LoginAction::ReportBug => self.open_issue_chooser(),
            LoginAction::CopyVersion => self.copy_version(),
            LoginAction::SafeGraphics(choice) => self.safe_graphics_choice(choice),
            LoginAction::DismissDriverNotice => {
                if let Some(notice) = self.login_form.driver_notice.take() {
                    log::info!("graphics driver warning dismissed for {}", notice.key);
                    self.settings.gpu_driver_dismissed = notice.key;
                    self.settings.save();
                }
            }
            LoginAction::Quit => self.request_exit(),
            LoginAction::Preferences => ui::widgets::toggle_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings),
            LoginAction::Accessibility => {
                ui::widgets::open_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings);
                self.options_ui.open(ui::options::Page::Accessibility, 0);
            }
            LoginAction::None | LoginAction::Login => {}
        }
        // the line first, then the typing stop it brings (sendChat, then
        // stopTyping)
        if let Some(c) = a.chat {
            self.send_local_chat(c);
        }
        self.update_local_typing();
        for cmd in a.typing {
            self.send(cmd);
        }
        for id in a.gestures {
            self.gesture_play(id, None);
        }
        for (to, mut text) in a.ims {
            // ":" as "/me" in the conversations too (FSFloaterIM::sendMsg)
            if self.settings.colon_emote {
                ui::chat::colon_emote(&mut text);
            }
            let text = ui::emoji::expand_shortcodes(&text);
            if self.world.is_group_session(&to) {
                self.world.send_session_im(to, &text);
                continue;
            }
            if self.world.mutes.is_muted_id(&to) {
                let name = self.world.legacy_name(&to).unwrap_or_else(|| self.world.social.name_of(&to));
                self.world.unblock(to, &name, 0);
                self.world.system_notice(
                    aurora_i18n::tr!("app-was-unblocked-because-you-sent-them-an-im", name = name.to_string()),
                    crate::world::notice_topics::Topic::People,
                );
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
            let ids = a
                .offer_tp
                .into_iter()
                .filter(|id| self.world.rlva_can_offer_teleport(*id))
                .collect();
            self.contacts_ui.ask_teleport_offer(ids, &self.world.rlva_region_label());
        }
        if let Some(asset) = a.landmark {
            self.record_library_asset(asset, 3);
            self.request_user_teleport(
                NetCommand::TeleportLandmark(asset),
                aurora_i18n::tr!("common-landmark").into(),
                false,
            );
        }
        if a.cancel_loading {
            self.release_bridge();
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
                self.options_ui.open(ui::options::Page::Sound, 0);
            }
        }
        self.poll_stream_list();
        match a.bar {
            BarAction::ImportStreamList => self.import_stream_list(),
            BarAction::ExportStreamList => self.export_stream_list(),
            BarAction::CheckUpdates => self.updater.check(self.net.runtime(), true),
            BarAction::ReportBug => self.open_issue_chooser(),
            BarAction::CopyVersion => self.copy_version(),
            BarAction::ReportAbuse => self.open_report_abuse(crate::report_abuse::ReportTarget::Nothing),
            BarAction::ServerReleaseNotes => self.send(NetCommand::ServerReleaseNotes),
            // FSCommon::report_to_nearby_chat(gAgent.mMOTD)
            BarAction::MessageOfTheDay => self.world.system_message(self.world.motd.clone()),
            BarAction::Import => {
                self.imports.toggle_window(&self.egui_ctx);
                self.egui_ctx.request_repaint();
            }
            BarAction::BlenderLive => {
                ui::widgets::open_floater(&self.egui_ctx, "blender_live", &mut self.blender_live.open);
            }
            BarAction::EditParticles => self.open_particle_editor(),
            BarAction::ShowHuds(visible) => self.set_show_huds(visible),
            BarAction::HudZoom(zoom) => self.world.hud_zoom = zoom.clamp(0.1, 1.0),
            BarAction::None => {}
            BarAction::IkEdit => {
                self.ik_editor.toggle();
            }
            BarAction::Poser => {
                self.ik_editor.open_poser();
                ui::widgets::open_floater(&self.egui_ctx, "poser", &mut self.ik_editor.open);
            }
            BarAction::Portals => {
                ui::widgets::open_floater(&self.egui_ctx, "portals", &mut self.portal_ui.open);
            }
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
                self.ik_editor.stop_editing();
                self.want_scene_capture = true;
                self.release_bridge();
                self.net.send(NetCommand::Logout);
                self.world.system_message(aurora_i18n::tr!("app-logging-out"));
            }
            BarAction::Quit => {
                self.request_exit();
            }
            BarAction::TeleportHome => {
                self.request_user_teleport(NetCommand::TeleportHome, aurora_i18n::tr!("common-home").into(), false);
            }
            BarAction::SetHome => {
                if let Some(cmd) = self.world.set_home_here() {
                    self.send(cmd);
                }
            }
            BarAction::TeleportBack => {
                let dest = self.world.tp_history.go_back();
                self.teleport_history(dest);
            }
            BarAction::TeleportForward => {
                let dest = self.world.tp_history.go_forward();
                self.teleport_history(dest);
            }
            BarAction::TeleportHistoryItem(i) => {
                let dest = self.world.tp_history.go_to(i);
                self.teleport_history(dest);
            }
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
                self.options_ui.open(ui::options::Page::Chat, 0);
            }
            BarAction::VoicePrefs => {
                self.panels.settings = true;
                self.options_ui.open_audio(ui::options::AudioSection::Voice);
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
            BarAction::WorldOverlays(overlays) => {
                self.settings.world_overlays = overlays;
                self.settings.save();
            }
            BarAction::ShowTransparency => self.settings.debug.show_alpha = !self.settings.debug.show_alpha,
            BarAction::OwnedLand => {
                ui::widgets::open_floater(&self.egui_ctx, ui::owned_land::WINDOW, &mut self.panels.owned_land);
                self.world.group_money.request_owned_land();
            }
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
            BarAction::ToggleInterface => self.toggle_interface(),
            BarAction::HideInterface => self.interface.set_hidden(&self.egui_ctx, true),
            BarAction::ToggleImmersive => self.toggle_immersive(),
            BarAction::ImmersiveHuds(visible) => self.set_immersive_huds(visible),
            BarAction::HoverTips(menu) => {
                let tips = &mut self.settings.hover_tips;
                tips.enabled = menu.enabled;
                tips.land = menu.land;
                tips.all_objects = menu.all_objects;
                self.settings.save();
            }
            BarAction::SystemIsland(shown) => {
                self.settings.system_island = shown;
                self.settings.save();
            }
            BarAction::ShowBalance(on) => {
                self.settings.show_balance = on;
                self.settings.save();
            }
            BarAction::NetStats(on) => {
                self.settings.show_net_stats = on;
                self.settings.save();
            }
            BarAction::GraphicsPreset(level) => {
                // LLPanelPresetsPulldown: the level at once, as Preferences
                // › Graphics sets it
                level.apply(&mut self.settings);
                self.settings.save();
                self.snapshots.settings_changed = true;
            }
            BarAction::SavedGraphicsPreset(name) => {
                if crate::graphics_presets::saved::select(&mut self.settings, &name) {
                    self.settings.save();
                    self.snapshots.settings_changed = true;
                }
            }
            BarAction::FpsCap(enabled) => {
                self.settings.fps_cap = enabled;
                self.settings.save();
            }
            BarAction::CameraPreset(preset) => {
                if self.world.rlva.camera_preset_locked(uuid::Uuid::nil()) {
                    return;
                }
                // as the camera controls' presets (LLFloaterCamera::switchToPreset)
                self.settings.camera.apply_preset(preset);
                self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
                self.settings.save();
            }
            BarAction::GraphicsPreferences => {
                ui::widgets::open_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings);
                self.options_ui.open(ui::options::Page::Graphics, 0);
            }
            BarAction::CameraControls => self.panels.camera_controls = true,
            BarAction::ToggleMoveLock => self.toggle_move_lock(),
            BarAction::ToggleQuickjump => {
                self.settings.quickjump = !self.settings.quickjump;
                self.settings.save();
                log::info!("quickjump: {}", self.settings.quickjump);
            }
            BarAction::ToggleQuietSnapshots => {
                self.settings.snapshot.sound = !self.settings.snapshot.sound;
                self.settings.save();
                log::info!("quiet snapshots: {}", !self.settings.snapshot.sound);
            }
            BarAction::ToggleWaterWalk => self.toggle_water_walk(),
            BarAction::ToggleSwimming => self.toggle_swimming(),
            BarAction::MoveLockOptions => self.movelock_ui.open = true,
            BarAction::MovementEnhancer => self.open_movement(),
            BarAction::NetworkGraph => {
                ui::widgets::open_floater(&self.egui_ctx, "perf", &mut self.panels.perf);
                ui::perf::show_network(&self.egui_ctx);
                self.egui_ctx
                    .move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("perf")));
            }
            BarAction::CustomizeBars => self.toolbar_ui.open = true,
            BarAction::StopAnimations => self.stop_current_animations(),
            BarAction::ResyncAnimations => self.scene.resync_animations(Instant::now()),
        }
    }

    /// Alt+Shift+U, Show User Interface (LLViewToggleUI): nothing in
    /// mouselook; shown, hidden at once with the islands, after Firestorm's
    /// ConfirmHideUI with its bars (unless unticked); hidden, shown at once.
    fn toggle_interface(&mut self) {
        if self.camera.mouselook() {
            return;
        }
        if self.interface.hidden {
            self.interface.set_hidden(&self.egui_ctx, false);
        } else if self.bars_look().0 && self.settings.confirm_hide_interface {
            self.interface.confirm = true;
            self.interface.confirm_again = true;
        } else {
            self.interface.set_hidden(&self.egui_ctx, true);
        }
    }

    /// World › HUDs › Show HUDs and its Alt+Shift+H shortcut
    /// (LLViewShowHUDAttachments, indra/newview/llviewermenu.cpp): a HUD
    /// locked by RLVa keeps them shown; hiding them ends a HUD drag or hold
    /// and the focus of HUD media.
    fn set_show_huds(&mut self, visible: bool) {
        if !visible && self.world.rlva_has_locked_hud() {
            self.world
                .system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
            return;
        }
        if visible {
            if self.hud_drag.is_some() {
                self.on_left_release();
            }
            self.release_object_hold();
        } else {
            self.leave_huds();
        }
        self.settings.show_huds = visible;
        self.settings.save();
    }

    /// The HUDs about to go: a HUD drag or hold ends, and the focus of HUD
    /// media.
    fn leave_huds(&mut self) {
        if self.hud_drag.is_some() {
            self.on_left_release();
        }
        self.release_object_hold();
        self.scene.lists.hud_view = None;
        self.media.unfocus();
    }

    /// The immersive mode's HUDs button (#1190): as Show HUDs, a HUD locked
    /// by RLVa keeps them shown; the choice is saved.
    fn set_immersive_huds(&mut self, visible: bool) {
        if !visible && self.world.rlva_has_locked_hud() {
            self.world
                .system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
            return;
        }
        if !visible {
            self.leave_huds();
        }
        self.settings.immersive_huds = visible;
        self.settings.save();
    }

    /// Immersive mode (#859): nothing in mouselook, as Show User Interface;
    /// no question first (asked for by its own button); on, its button or
    /// the shortcut of Show User Interface leaves it. Its HUDs hidden
    /// (#1190), entering it is leaving them.
    fn toggle_immersive(&mut self) {
        if self.camera.mouselook() {
            return;
        }
        if self.interface.immersive {
            self.interface.set_hidden(&self.egui_ctx, false);
        } else {
            if !self.settings.immersive_huds && !self.world.rlva_has_locked_hud() {
                self.leave_huds();
            }
            self.interface.set_immersive(&self.egui_ctx);
        }
    }

    /// World › Environment's shortcuts (`World.EnvSettings`), like its menu
    /// entries; greyed by RLVa's @setenv (menu_viewer.xml: RLV.EnableIfNot).
    fn env_shortcut(&mut self, action: Action) {
        let Some(time) = action.time_of_day() else {
            return;
        };
        if self.world.eep.rlva_locked {
            return;
        }
        if time == 0 {
            self.world.eep.clear_local();
        }
        self.panels.time_of_day = time;
    }

    /// The keys of World › Environment's shortcuts, as its menu writes them
    /// (`ui::bars::StatusInfo::env_keys`).
    fn env_keys(&self) -> [String; 6] {
        let keys = |action| {
            self.settings.keybinds.get(action)[0]
                .as_ref()
                .map(|b| b.label())
                .unwrap_or_default()
        };
        [
            keys(Action::EnvShared),
            keys(Action::EnvSunrise),
            keys(Action::EnvMidday),
            keys(Action::EnvSunset),
            keys(Action::EnvMidnight),
            String::new(),
        ]
    }

    /// The keys of Show User Interface, as the menus write them.
    fn interface_keys(&self) -> String {
        self.settings.keybinds.get(crate::keybinds::Action::ToggleInterface)[0]
            .as_ref()
            .map(|b| b.label())
            .unwrap_or_default()
    }

    /// Me › Avatar health: our avatar and the keys of its shortcuts.
    fn health_menu(&self) -> ui::bars::HealthMenu {
        let keys = |action| {
            self.settings.keybinds.get(action)[0]
                .as_ref()
                .map(|b| b.label())
                .unwrap_or_default()
        };
        ui::bars::HealthMenu {
            me: self.world.agent_id,
            stop_keys: keys(Action::StopAnimations),
            rebake_keys: keys(Action::Rebake),
            refresh_keys: keys(Action::RefreshAttachments),
            resync_keys: keys(Action::ResyncAnimations),
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
                self.request_user_teleport(
                    NetCommand::TeleportLandmark(asset),
                    aurora_i18n::tr!("common-landmark").into(),
                    false,
                );
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
    /// Nearby chat flashes the Conversations button (Preferences › Chat, or
    /// that button's right-click menu); `AURORA_DEMO_CONV_BUTTON=mute` leaves
    /// it out for the run.
    fn flash_nearby(&self) -> bool {
        self.settings.flash_conversations_nearby && !(self.demo && crate::demo::conv_button::nearby_off())
    }

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
        let layout = self.dock_layout.of(&self.skin.layout, self.settings.autohide_chat_bar).clone();
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
        let now = std::time::Instant::now();
        for id in ids {
            let Some(item) = self.world.inventory.items.get(&id) else {
                continue;
            };
            let (asset, name, thumb) = (item.asset_id, item.name.clone(), item.thumbnail);
            let (global, region_id) = match self.world.landmarks.peek(asset) {
                crate::world::landmarks::Resolved::Ready { global, region_id, .. } => (Some(global), region_id),
                crate::world::landmarks::Resolved::Pending if shown => {
                    self.world.landmarks.resolve(asset);
                    (None, uuid::Uuid::nil())
                }
                _ => (None, uuid::Uuid::nil()),
            };
            let sim = global.and_then(|g| self.world.map.sim_at_global(g.x, g.y).map(|(_, s)| (s.name.clone(), s.access)));
            if shown
                && sim.is_none()
                && let Some(g) = global
            {
                let (x, y) = ((g.x / 256.0) as i64, (g.y / 256.0) as i64);
                self.world.map.want_regions(x, y, x, y);
            }
            // the item's thumbnail, else the snapshot of the landmark's parcel
            let picture = if thumb.is_nil() {
                let spot = crate::world::place_snapshots::Spot::Landmark(asset);
                if shown && let Some(g) = global {
                    self.world.place_snapshots.want(spot, g, region_id, now);
                }
                self.world.place_snapshots.snapshot(spot, &self.world.profiles.parcels)
            } else {
                Some(thumb)
            };
            let thumbnail = picture.and_then(|t| {
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

    /// Help › Knowledge base: the help page of the grid info of an OpenSim
    /// grid, none on Second Life (update_grid_help, llviewermenu.cpp).
    /// AURORA_DEMO_GRID_HELP=<page>: a page of the tester's choice in demo,
    /// named after its host.
    fn grid_help(&self) -> Option<ui::bars::GridHelp> {
        if self.demo {
            let url = std::env::var("AURORA_DEMO_GRID_HELP").ok().filter(|u| !u.trim().is_empty())?;
            let grid = crate::grids::authority(&url)?;
            return Some(ui::bars::GridHelp { grid, url });
        }
        let grid = crate::grids::active(&self.settings)?;
        (!grid.help.trim().is_empty()).then(|| ui::bars::GridHelp {
            grid: grid.name.clone(),
            url: grid.help.trim().to_owned(),
        })
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
        if let Some(grid) = crate::grids::active(&self.settings) {
            return grid
                .search_url(
                    self.search_ui.web_query(self.settings.classic_search),
                    aurora_i18n::language(),
                    self.world.agent_access().preferred,
                )
                .map(|url| crate::world::search::WebHome { url, identified: false });
        }
        crate::world::search::web_home(
            self.search_grid(),
            self.search_ui.web_query(self.settings.classic_search),
            self.world.agent_access(),
            aurora_i18n::language(),
            self.search_session(),
        )
    }

    /// LLFloaterReg::showInstance("add_landmark"): World › Landmark this
    /// Place, the Places "add" entries and About Land's Landmark button;
    /// `dest` is the folder chosen in Places.
    pub(super) fn open_create_landmark(&mut self, dest: Option<uuid::Uuid>) {
        if self.world.rlva.has("showloc") {
            return;
        }
        if let Some(cmd) = self.create_landmark_ui.open(&self.world, dest) {
            self.send(cmd);
        }
    }

    /// hasParcelLandmark: the landmark lying on the agent's parcel (the lit
    /// star). Existing landmarks never prevent creating another one.
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
                SearchAction::JoinGroup(id) => self.group_profile_ui.join(&self.world, id),
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
                SearchAction::WantParcel(id) => self.send(NetCommand::ParcelInfoRequest(id)),
                SearchAction::WantClassified(id) => self.send(NetCommand::ClassifiedInfoRequest(id)),
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
                    self.world.save_group_info(group, info);
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
                // showForGroup: sendAgentDataUpdateRequest, clearGroupData,
                // then updateLists: the properties first, roles and members
                GroupAction::ReloadForInvite(group) => {
                    self.send(NetCommand::RequestGroups);
                    self.world.search.request_group(group);
                    self.world.group_data.reload_for_invite(group);
                }
                // LLGroupActions::startCall
                GroupAction::Call(group) => self.on_ctx_action(ui::context::CtxAction::Call(group)),
                GroupAction::Activate(group) => self.send(NetCommand::ActivateGroup(group)),
                GroupAction::SetTitle { group, role } => {
                    if !self.world.rlva.has("setgroup") {
                        self.world.group_data.set_title(group, role);
                    }
                }
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
        self.options_ui.chat_log_custom = !self.settings.chat_log_location.is_empty();
        self.poll_folder_pick();
        self.poll_settings_backup();
        if let Some(done) = self.world.chat_log.take_relocated() {
            self.options_ui.chat_log_status = match done {
                Ok(count) => aurora_i18n::tr!("ui-options-chat-logs-moved", count = count),
                Err(e) => {
                    log::warn!("conversation transcripts: moving failed ({e})");
                    self.settings.chat_log_location = self.folders.chat_log_previous.take().unwrap_or_default();
                    self.folders.settings_changed = true;
                    aurora_i18n::tr!("ui-options-chat-logs-move-failed").to_owned()
                }
            };
        }
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
        // the account settings the grid keeps, asked at each opening
        // (AURORA_DEMO_USERINFO=opensim: another grid's)
        let second_life =
            !matches!(self.settings.grid, crate::settings::GridChoice::Custom) && !(self.demo && crate::demo::home::opensim());
        self.world.follow_preferences(self.panels.settings, second_life);
        self.options_ui.account = self.world.login.is_some().then(|| ui::options::privacy::Account {
            info: self.world.user_info.info.clone(),
            show_in_search: self.world.show_in_search(),
            second_life,
        });
        let time_of_day = self.settings.time_of_day;
        if self.options_ui.anti_spam_version != self.world.anti_spam.version {
            self.options_ui.anti_spam_version = self.world.anti_spam.version;
            self.options_ui.anti_spam_blocks = self
                .world
                .anti_spam
                .blocks()
                .into_iter()
                .map(|(queue, source)| {
                    let name = self
                        .build
                        .props
                        .get(&source)
                        .or_else(|| self.interactions.props.get(&source))
                        .map(|properties| properties.name.clone())
                        .filter(|s| !s.is_empty())
                        .or_else(|| self.world.social.avatar_names.complete(&source))
                        .or_else(|| self.world.social.names.get(&source).cloned())
                        .unwrap_or_else(|| source.to_string());
                    (queue, source, name)
                })
                .collect();
        }
        self.options_ui.rlva_restrictions = self.world.rlva.restrictions();
        self.options_ui.current_parcel = self.world.current_parcel_entry();
        self.options_ui.selected_parcel = self.world.selected_parcel_entry();
        self.options_ui.rlva_camera_preset_locked = self.world.rlva.camera_preset_locked(uuid::Uuid::nil());
        self.options_ui.rlva_camera_fov_locked = self.world.rlva.camera_owner().is_some();
        self.options_ui.joystick = self.joystick.snapshot.clone();
        self.options_ui.flycam_active = self.joystick.active;
        let before_camera = self.settings.camera.clone();
        let mut res = ui::options::show(ctx, p, &self.skin.icons, &mut self.settings, &mut self.options_ui, &mut open);
        self.world.rlva.preserve_camera_settings(&mut self.settings.camera, &before_camera);
        if res.toggle_flycam {
            self.toggle_joystick_flycam();
        }
        // the panels hold the time of day shown and write it back to the
        // settings later in the frame: a choice made here (or by a reset)
        // goes to them first, or it is undone at once (#742)
        if self.settings.time_of_day != time_of_day {
            self.panels.time_of_day = self.settings.time_of_day;
        }
        // Repeatable live-scale profiling without taking the native pointer.
        if self.demo
            && matches!(self.frame_count, 740 | 980)
            && std::env::var("AURORA_DEMO_OPTIONS").is_ok_and(|mode| mode == "1:scale-keys")
        {
            self.settings.snap_windows = !self.settings.snap_windows;
            res.changed = true;
            log::info!("demo keyboard UI scale: unrelated preference changed");
        }
        if self.demo && self.frame_count >= 600 && std::env::var("AURORA_DEMO_OPTIONS").is_ok_and(|mode| mode == "1:scale") {
            let phase = (self.frame_count - 600) % 600;
            let scale = 1.0 + phase.min(600 - phase) as f32 / 600.0;
            if self.settings.ui_scale != scale {
                self.settings.ui_scale = scale;
                res.ui_scale_changed = true;
            }
            if phase.is_multiple_of(300) {
                log::info!("demo interface scale: frame={} scale={scale:.3}", self.frame_count);
            }
        }
        if self.demo && self.frame_count >= 600 && std::env::var("AURORA_DEMO_OPTIONS").is_ok_and(|mode| mode == "2:text-size") {
            let phase = ((self.frame_count - 600) / 180) % 5;
            let scale = [0.8, 1.0, 1.4, 2.0, 1.0][phase as usize];
            if self.settings.font_scale != scale {
                self.settings.font_scale = scale;
                res.changed = true;
                log::info!("demo text size: frame={} scale={scale:.1}", self.frame_count);
            }
        }
        if res.reset_anti_spam {
            self.world.anti_spam.reset();
        }
        if let Some((queue, source)) = res.unblock_anti_spam {
            self.world.anti_spam.unblock(queue, source);
        }
        self.world.anti_spam.configure(self.settings.anti_spam);
        self.update_rlva();
        if let Some(change) = res.account
            && let Some(cmd) = self.world.change_user_info(change)
        {
            self.send(cmd);
        }
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
            if let Err(e) = crate::logging::open_directory(dir) {
                log::warn!("cannot open the transcripts folder: {e}");
            }
        }
        if res.erase_chat_logs {
            self.options_ui.chat_log_status.clear();
            self.world.chat_log.erase();
        }
        if let Some((folder, choice)) = res.folder {
            self.choose_folder(folder, choice);
        }
        if res.back_up {
            self.back_up_settings(ctx);
        }
        if res.pick_backup {
            self.pick_settings_backup();
        }
        if let Some(restored) = res.restored.take() {
            // a new egui memory drops the fonts waiting for the next pass
            res.font_changed |= self.restore_settings(ctx, restored);
        }
        a.settings_changed = res.changed || std::mem::take(&mut self.folders.settings_changed);
        a.ui_scale_changed = res.ui_scale_changed;
        a.font_changed = res.font_changed;
        if res.refresh_devices {
            self.devices_listed = false;
        }
        if std::mem::take(&mut self.options_ui.restart_voice) {
            self.voice.restart();
        }
        if res.reset_camera {
            self.reset_camera_all();
        }
        if res.reset {
            self.settings.save();
            self.world.system_message(aurora_i18n::tr!("app-all-settings-have-been-reset"));
            ui::layout::forget(ctx);
        }
        if let Some(page) = res.reset_section {
            self.settings.save();
            self.world
                .system_message(aurora_i18n::tr!("app-section-settings-reset", section = page.name()));
        }
        if res.open_dev_toolbox {
            self.panels.dev_toolbox = true;
        }
        if self.panels.settings && !open {
            self.settings.save();
        }
        self.panels.settings = open;
    }

    fn draw_ui(&mut self, ui: &mut egui::Ui, projector: &ui::hud::Projector, renderer: &Renderer) -> UiActions {
        self.world.refresh_rlva_privacy();
        self.notif_ui.privacy = self.world.rlva_privacy.clone();
        let mut a = UiActions::default();
        let p = self.palette;
        let ctx = ui.ctx().clone();
        ui::inventory::view::set_picker_sort(&ctx, self.settings.inventory.sort);
        ui::widgets::set_perceptual_volume(&ctx, self.settings.audio.perceptual_volume);
        let beacon_hovered = !ctx.is_pointer_over_egui() && !ctx.egui_wants_pointer_input() && self.beacon_hover();
        self.beacon_hit = None;
        let mut t = Instant::now();
        if self.exit_prompt.open {
            ui.disable();
            ctx.memory_mut(|m| m.set_modal_layer(egui::Modal::default_area(egui::Id::unique("viewer_exit")).layer()));
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
        if !matches!(self.screen, Screen::World) && self.interface.hidden {
            self.interface.set_hidden(&ctx, false);
        }
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
                        previous_session: false,
                    });
                } else if self.demo_mfa_challenge.is_some() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                self.sync_remembered_password();
                self.login_info.poll();
                self.login_form.block_reason = self
                    .updater
                    .login_block_reason()
                    .or_else(|| aurora_updater::test_environment().map(|_| aurora_i18n::tr!("update-login-test")));
                self.command_line_login();
                self.poll_login_wait(&ctx);
                let progress = self.login_progress.as_ref().map(|(m, f)| (m.as_str(), *f));
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
                    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(ui::options::WINDOW)));
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
                    self.photo_ui(&ctx, &p, projector, renderer);
                    // the poser stays over it (#1389): pose, then shoot
                    if self.ik_editor.open && self.ik_editor.posing() {
                        // no bars here: its room is the whole window, as
                        // large as it is now (the bars' room of the last
                        // frame with them would keep an older window size)
                        let screen = ctx.content_rect();
                        ui::layout::set_work_area(&ctx, screen);
                        ui::layout::set_move_area(&ctx, screen);
                        self.ik_editor.over_viewfinder = true;
                        self.ik_editor.show(&ctx, &p, &self.world, projector);
                        self.ik_editor.over_viewfinder = false;
                    }
                    // the exit's question still shows over the photo mode
                    a.exit = self.exit_prompt.show(&ctx, &p, self.in_world());
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
                // who gets the voice row of the avatar menus
                let me = self.world.agent_id;
                self.world.calls.voice_peers.clear();
                self.world.calls.voice_peers.extend(voice_levels.keys().filter(|id| **id != me));
                let audio = &self.settings.audio;
                self.scene.live_morphs.set_levels(
                    &voice_levels,
                    audio.lip_sync && (audio.voice_visualizers_in_calls || self.world.calls.active.is_none()),
                    Instant::now(),
                );
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
                // AURORA_VOICE_LEVEL_LOG=1 (temporary): our own plate's voice level in the log
                self.plates.probe = ui::nameplate::level_log().then_some(self.world.agent_id);
                self.update_plate_hover(&ctx);
                let chat_bubbles = self.chat_bubbles();
                self.name_tags = if held {
                    self.name_tag_visibility.clear();
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
                        !self.interface.immersive || self.settings.immersive_name_tags,
                        &self.settings,
                        &mut self.name_tag_visibility,
                        &voice_levels,
                        &self.settings.audio,
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
                            own_typing: self.chat_ui.local_typing(),
                        }),
                        chat_bubbles.then(|| ui::chat_bubbles::Frame {
                            chat: &self.world.bubble_chat,
                            typing: self.settings.typing_bubbles || (self.demo && crate::demo::tags::chat_bubbles() == Some(true)),
                            own_typing: self.chat_ui.local_typing(),
                            opacity: self.settings.chat_bubble_opacity,
                            now: Instant::now(),
                        }),
                    )
                };
                if !self.camera.mouselook() && !held {
                    self.beacon_hit = ui::minimap::draw_beacon(&ctx, &p, &self.world, projector, beacon_hovered);
                }
                // the hover tips of what the pointer rested on (none in
                // mouselook nor over a held photo)
                if let Some(action) = self.draw_hover_tips(&ctx, &p, !self.camera.mouselook() && !held) {
                    a.ctx_action = Some(action);
                }
                ui::sky_beacons::draw(
                    &ctx,
                    &p,
                    &self.world,
                    projector,
                    [self.settings.sun_beacon, self.settings.moon_beacon],
                );
                ui::beacons::draw(&ctx, &p, projector, &self.scene.beacon_markers, &self.settings.beacons);
                ui::portals::labels(&ctx, &p, &self.world, projector);
                aurora_render::end_world_labels(&ctx);
                ui::hud::draw_attachments(&ctx, &self.world, &self.scene);
                t = self.ui_parts.lap(Part::UiTags, t);
                let parcel_landmark = self.parcel_landmark();
                self.follow_favorites_access();
                let favorites_shown = ui::bars::favorites::take_shown(&ctx);
                let here = parcel_landmark.as_ref().is_some_and(|lm| lm.favorite);
                let favorites = self.favorites_view(favorites_shown, here);
                let (classic_bars, islands) = self.bars_look();
                self.poser_consent_ui(&ctx, &p);
                let own_name = self.world.own_name();
                // AURORA_DEMO_BARS: the long parcel name of the mockup (the
                // islands only: Firestorm's bars stay as they were)
                // AURORA_DEMO_PARCEL_NAME=<text>: any parcel name ("\n" is a
                // line break), to check how odd names sit in the place island
                let demo_parcel = if self.demo {
                    std::env::var("AURORA_DEMO_PARCEL_NAME").ok().map(|v| v.replace("\\n", "\n"))
                } else {
                    None
                };
                let parcel_name = if let Some(name) = demo_parcel.as_deref() {
                    name
                } else if self.demo && ui::bars::demo::get().on && !classic_bars {
                    aurora_i18n::tr!("demo-bars-parcel")
                } else {
                    self.world.parcel_name.as_str()
                };
                let mut contents_actions = Vec::new();
                let icons = &self.skin.icons;
                let layout = &self.skin.layout;
                let maturity = self
                    .world
                    .main()
                    .and_then(|r| r.info.as_ref())
                    .map(|i| ui::bars::maturity_name(i.sim_access))
                    .unwrap_or("");
                // the conversations' thumbnails (#1134): the bars reserve
                // their room, `thumbnails::show` draws them below
                let thumbnails = if self.demo { ui::chat::thumbnails::demo_place() } else { None };
                ui::chat::thumbnails::prepare(
                    &mut self.chat_ui,
                    &self.world,
                    thumbnails.unwrap_or(self.settings.chat_thumbnails),
                    self.settings.chat_thumbnails_flash,
                    self.settings.reduce_motion,
                );
                let status = ui::bars::StatusInfo {
                    location_hidden: self.world.rlva.has("showloc"),
                    show_bug_report: if self.demo {
                        ui::bars::demo::get().bug.unwrap_or(self.settings.show_bug_report)
                    } else {
                        self.settings.show_bug_report
                    },
                    show_huds: self.settings.show_huds,
                    hud_zoom: self.world.hud_zoom,
                    ban_lines: self.settings.maps.ban_lines,
                    world_overlays: self.settings.world_overlays,
                    show_transparency: self.settings.debug.show_alpha,
                    conversation_thumbnails: self.chat_ui.thumbnails.top_count(),
                    region: if self.world.rlva.has("showloc") {
                        aurora_i18n::tr!("rlva-hidden-region")
                    } else {
                        &self.world.region_name()
                    },
                    parcel: if self.world.rlva.has("showloc") {
                        aurora_i18n::tr!("rlva-hidden-parcel")
                    } else {
                        parcel_name
                    },
                    maturity,
                    position: self.world.agent.position,
                    fps: self.perf.fps,
                    flying: self.world.agent.flying,
                    seated: self.world.agent.is_sitting(),
                    ping_ms: self.net.stats.ping_ms.load(std::sync::atomic::Ordering::Relaxed),
                    balance: self.world.balance,
                    show_balance: if self.demo {
                        ui::bars::demo::get().show_balance.unwrap_or(self.settings.show_balance)
                    } else {
                        self.settings.show_balance
                    },
                    voice: (self.voice.light, self.voice.status.clone()),
                    slurl: if self.world.rlva.has("showloc") {
                        String::new()
                    } else {
                        crate::slurl::make(&self.world.region_name(), self.world.agent.position)
                    },
                    tp_back: self.world.tp_history.previous().map(|e| e.region.clone()),
                    tp_forward: self.world.tp_history.next().map(|e| e.region.clone()),
                    tp_session: self.world.tp_history.session(),
                    status: self.world.status.modes(),
                    local_env: self.world.eep.has_local(),
                    env_keys: self.env_keys(),
                    double_click: self.double_click_action(),
                    quickjump: self.settings.quickjump,
                    quiet_snapshots: !self.settings.snapshot.sound,
                    parcel_landmark,
                    can_set_home: self.world.can_set_home(),
                    release_notes: self.world.main_has_cap("ServerReleaseNotes"),
                    region_console: self.world.main_has_cap("SimConsoleAsync"),
                    motd: !self.world.motd.is_empty(),
                    grid_help: self.grid_help(),
                    marketplace: self.marketplace_home().is_some()
                        || (self.demo && crate::world::marketplace::demo::Mode::from_env().is_some()),
                    parcel_icons: match (self.world.main().and_then(|r| r.info.as_ref()), self.world.parcel.as_deref()) {
                        (Some(region), Some(parcel)) => ui::parcel_icons::parcel_icons(&ui::parcel_icons::ParcelState {
                            region_flags: region.region_flags,
                            parcel,
                            agent_id: self.world.agent_id,
                            group: self.world.groups.group(&parcel.group_id),
                            health: self.world.health,
                            frozen: self.world.frozen,
                        }),
                        _ => Vec::new(),
                    },
                    classic_bars,
                    islands,
                    own_name: &own_name,
                    own_pic: self.avatar_pics.get(&self.world.agent_id),
                    recent: if classic_bars { Vec::new() } else { self.recent_places() },
                    favorites: &favorites,
                    system_island: if self.demo {
                        let (demo, s) = (ui::bars::demo::get(), self.settings.system_island);
                        crate::settings::SystemIsland {
                            clock_zone: demo.clock_zone.unwrap_or(s.clock_zone),
                            clock_seconds: demo.clock_seconds.unwrap_or(s.clock_seconds),
                            clock_format: demo.clock_format.unwrap_or(s.clock_format),
                            ..s
                        }
                    } else {
                        self.settings.system_island
                    },
                    net_stats: if self.demo {
                        ui::bars::demo::get().net_stats.unwrap_or(self.settings.show_net_stats)
                    } else {
                        self.settings.show_net_stats
                    },
                    perf: &self.perf,
                    net_loss: crate::net_profile::loss_percent(&self.net_profile.last),
                    graphics_preset: crate::graphics_presets::Preset::matching(&self.settings),
                    graphics_saved: crate::graphics_presets::saved::choices(&self.settings),
                    fps_cap: self.settings.fps_cap,
                    fps_limit: self.settings.fps_limit,
                    camera_preset: self.settings.camera.preset,
                    interface_hidden: self.interface.hidden,
                    interface_keys: self.interface_keys(),
                    ik_keys: self.settings.keybinds.get(Action::IkEdit)[0]
                        .as_ref()
                        .map(|b| b.label())
                        .unwrap_or_default(),
                    edit_particles: self.particle_editor_target().is_some(),
                    health: self.health_menu(),
                    hover_tips: ui::bars::HoverTipsMenu {
                        enabled: self.settings.hover_tips.enabled,
                        land: self.settings.hover_tips.land,
                        all_objects: self.settings.hover_tips.all_objects,
                    },
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
                // the bars this frame: hidden with the interface (an edge
                // brings the islands back) and in mouselook, like Firestorm's
                let screen_rect = ctx.content_rect();
                let demo_hidden = if self.demo { ui::bars::demo::get().hidden } else { None };
                let chrome = self.interface.bars(&ui::interface::Frame {
                    classic: classic_bars,
                    mouselook: self.camera.mouselook(),
                    show_in_mouselook: self.settings.camera.show_interface_in_mouselook,
                    chat_active: self.chat_ui.has_focus || self.chat_ui.focus_request || !self.chat_ui.input.trim().is_empty(),
                    // AURORA_DEMO_BARS=peek: the pointer at the top edge
                    pointer: if demo_hidden == Some(true) {
                        Some(egui::pos2(screen_rect.center().x, screen_rect.top()))
                    } else {
                        ctx.input(|i| i.pointer.hover_pos())
                    },
                    screen: screen_rect,
                    top_rows: ui::bars::islands::favorites_row_bottom(&ctx)
                        .unwrap_or(screen_rect.top() + ui::bars::islands::MARGIN + ui::bars::islands::HEIGHT),
                    bottom_top: self.interface.last_room.map_or(screen_rect.bottom() - 56.0, |(_, bottom)| bottom),
                    menu_open: egui::Popup::is_any_open(&ctx),
                });
                if chrome.top {
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
                }
                // AURORA_DEMO_BARS: the mockup's voice (connected; talking
                // with `voice`) and chat island width, for this run only and
                // with the islands (Firestorm's bars stay as they were)
                let demo_bars = if self.demo && !classic_bars {
                    ui::bars::demo::get()
                } else {
                    Default::default()
                };
                let mut demo_chat_width = demo_bars.chat_width;
                // the chat bar shown only while it is used (#1009); also
                // AURORA_DEMO_BARS=autohide, for this run, with either bars,
                // and `chatstart`: Enter pressed at frame 300 on a draft
                let demo_autohide = if self.demo { ui::bars::demo::get() } else { Default::default() };
                let autohide_chat_bar = self.settings.autohide_chat_bar || demo_autohide.autohide;
                self.chat_ui.autohide = autohide_chat_bar;
                if demo_autohide.chat_start && ctx.cumulative_frame_nr() == 300 {
                    self.chat_ui.input = "Hello!".into();
                    self.chat_ui.focus_request = true;
                }
                let mut mic = ui::bars::MicButton {
                    on: self.mic_on,
                    denied: self.speak_denied(),
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
                    ear: self.settings.audio.voice_ear,
                    noise_suppression: self.settings.audio.voice_noise_suppression,
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
                self.toolbar_ui.flash_nearby = self.flash_nearby();
                self.chat_ui.flash_nearby = self.flash_nearby();
                let toolbar_migrated = self.settings.migrate_toolbar(self.world.agent_id);
                let mut demo_toolbar = None;
                // AURORA_DEMO_TOOLBAR=custom|side|labels|right: the mockup's
                // side bar, for this run only
                if self.demo && std::env::var_os("AURORA_DEMO_TOOLBAR").is_some() {
                    let layout = if classic_bars {
                        layout
                    } else {
                        self.dock_layout.of(layout, autohide_chat_bar)
                    };
                    let mut t = self.settings.toolbar.clone().unwrap_or_default();
                    ui::toolbar::demo_settings(&mut t, layout);
                    demo_toolbar = Some(t);
                }
                if self.demo && ui::bars::demo::get().access == Some(crate::settings::FavoritesAccess::Dock) {
                    let layout = self.dock_layout.of(layout, autohide_chat_bar).clone();
                    let mut t = demo_toolbar
                        .take()
                        .unwrap_or_else(|| self.settings.toolbar.clone().unwrap_or_default());
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
                // AURORA_DEMO_GRID_STATUS: the Grid status button at the end
                // of the bottom bar, for this run only
                if self.demo && crate::demo::grid_status::mode().is_some() {
                    let layout = if classic_bars {
                        layout.clone()
                    } else {
                        self.dock_layout.of(layout, autohide_chat_bar).clone()
                    };
                    let mut t = demo_toolbar
                        .take()
                        .unwrap_or_else(|| self.settings.toolbar.clone().unwrap_or_default());
                    let mut buttons = t.buttons(&layout).to_vec();
                    if let Some(c) = ui::toolbar::COMMANDS.iter().find(|c| c.id == "gridstatus")
                        && !buttons.iter().any(|b| b.command == c.id)
                    {
                        buttons.push(ui::skin::ToolButton {
                            icon: c.icon.into(),
                            command: c.id.into(),
                            tooltip: String::new(),
                        });
                    }
                    t.buttons = Some(buttons);
                    demo_toolbar = Some(t);
                }
                let toolbar_settings = match demo_toolbar.as_mut() {
                    Some(t) => t,
                    None => self.settings.toolbar.get_or_insert_default(),
                };
                // the dock leaves the Conversations button out of its default
                let bar_layout = if classic_bars {
                    layout
                } else {
                    self.dock_layout.of(layout, autohide_chat_bar)
                };
                // the feed is polled while its button stands on a toolbar
                // (LLFloaterGridStatus::checkGridStatusRSS, hasCommand)
                self.grid_status_listed = toolbar_settings.location("gridstatus", bar_layout).is_some();
                self.toolbar_ui.islands = !classic_bars;
                self.chat_ui.keep_focus = self.settings.chat_keep_focus;
                // the chat fields' suggestions (#980); the residents around
                // only while a nearby field lists or predicts them
                let suggest = &mut self.chat_ui.suggest;
                suggest.options = ui::chat::suggest::Options {
                    gestures: self.settings.chat_gesture_autocomplete,
                    // the demo's prediction leaves the saved settings alone
                    names: self.settings.chat_name_prediction || (self.demo && suggest_demo::mode() == "names"),
                    mentions: self.settings.chat_mention_autocomplete,
                };
                let now = ui.input(|i| i.time);
                if suggest.wants_nearby(now) {
                    suggest.set_nearby(ui::chat::suggest::nearby(&self.world), now);
                }
                let ao_active = self.settings.ao.accounts.get(&self.world.agent_id).is_some_and(|a| a.enabled) && !self.ao_runtime.paused;
                let (chat, bar2, bottom_top, chat_width_set) = if !chrome.bottom {
                    // hidden: the chat lines go down to the screen's bottom
                    (None, BarAction::None, screen_rect.bottom() - ui::bars::islands::MARGIN, false)
                } else {
                    ui::bars::bottom_bar(
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
                        ao_active,
                        !classic_bars,
                        Some(&mut self.emoji),
                        &favorites,
                    )
                };
                // what the bars leave: where the windows are laid out and
                // snapped this frame (above the dock with the islands)
                let mut area = ui::layout::bars_room(ui.available_rect_before_wrap(), bottom_top, classic_bars);
                // the favourites' row under the islands keeps the windows under it
                if !classic_bars && let Some(row) = ui::bars::islands::favorites_row_bottom(ui.ctx()) {
                    area.min.y = area.min.y.max(row + ui::bars::islands::MARGIN);
                }
                // the left and right toolbars, and the windows beside them
                let sides = self.interface.sides(
                    &ui::interface::Frame {
                        classic: classic_bars,
                        mouselook: self.camera.mouselook(),
                        show_in_mouselook: self.settings.camera.show_interface_in_mouselook,
                        chat_active: false,
                        // AURORA_DEMO_BARS=peekleft: the pointer at the left edge
                        pointer: if self.demo && ui::bars::demo::get().peek_left {
                            Some(egui::pos2(screen_rect.left(), screen_rect.center().y))
                        } else {
                            ctx.input(|i| i.pointer.hover_pos())
                        },
                        screen: screen_rect,
                        top_rows: 0.0,
                        bottom_top,
                        menu_open: egui::Popup::is_any_open(&ctx),
                    },
                    ui::bars::side::edges(&ctx),
                );
                let side_row = ui::bars::Row {
                    unread: if demo_bars.read { 0 } else { self.world.conversations_badge() },
                    latest_unread: self.world.social.latest_unread(),
                    flash: self.chat_ui.button_flash.button(ctx.input(|i| i.time)).0,
                    flying: self.world.agent.flying,
                    seated: self.world.agent.is_sitting(),
                    mouselook: self.camera.mouselook(),
                    ao_active,
                    locked: toolbar_settings.locked,
                    favorites: &favorites,
                    bar: ui::toolbar::Bar::Left,
                    style: Default::default(),
                    editing: self.toolbar_ui.open && !toolbar_settings.locked,
                };
                let (room, side_action) = ui::bars::side::bars(
                    ui,
                    &p,
                    icons,
                    bar_layout,
                    &mut self.toolbar_ui,
                    toolbar_settings,
                    &mut self.panels,
                    &mut mic,
                    &side_row,
                    area,
                    !classic_bars,
                    sides,
                );
                area = room;
                // without the bars, the windows keep the room they left
                if chrome.top && chrome.bottom {
                    self.interface.last_room = Some((area, bottom_top));
                } else if let Some((room, _)) = self.interface.last_room {
                    area = room;
                }
                ui::layout::set_work_area(ui.ctx(), area);
                ui::chat::thumbnails::show(
                    ui,
                    &p,
                    &mut self.world,
                    &mut self.chat_ui,
                    &mut self.panels,
                    &self.avatar_pics,
                    &self.ui_images,
                    area,
                    chrome.top,
                );
                // the islands leave room between them, above and below: the
                // windows move over the whole screen; the Legacy Viewer's
                // bars are full width, the windows stay between them
                ui::layout::set_move_area(ui.ctx(), if classic_bars { area } else { ui.ctx().content_rect() });
                let mut edges = ui::layout::Edges {
                    left: self.settings.windows_stop_left,
                    right: self.settings.windows_stop_right,
                    bottom: self.settings.windows_stop_bottom,
                };
                // AURORA_DEMO_WINDOW_EDGES=<l|r|b…>: those edges let open for
                // the run, nothing saved
                if self.demo
                    && let Ok(open) = std::env::var("AURORA_DEMO_WINDOW_EDGES")
                {
                    edges.left &= !open.contains('l');
                    edges.right &= !open.contains('r');
                    edges.bottom &= !open.contains('b');
                }
                ui::layout::set_window_edges(ui.ctx(), edges);
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
                // the Conversations button's right click (the chat island's
                // opens the window itself)
                for choice in [self.toolbar_ui.conversations_menu.take(), self.chat_ui.button_menu.take()]
                    .into_iter()
                    .flatten()
                {
                    match choice {
                        ui::chat::ButtonMenu::Open => {
                            ui::widgets::open_floater(ui.ctx(), "conversations", &mut self.panels.chat);
                        }
                        ui::chat::ButtonMenu::NearbyFlash(on) => {
                            self.settings.flash_conversations_nearby = on;
                            self.settings.save();
                        }
                    }
                }
                self.mic_on = mic.on;
                self.mic_button_held = mic.held;
                if mic.mode_changed || mic.devices_changed || mic.processing_changed {
                    // the audio engine and the voice follow the settings
                    self.settings.audio.mic_hold = mic.hold;
                    self.settings.audio.voice_ear = mic.ear;
                    self.settings.audio.voice_noise_suppression = mic.noise_suppression;
                    self.settings.audio.input_device = std::mem::take(&mut mic.input_device);
                    self.settings.audio.output_device = std::mem::take(&mut mic.output_device);
                    self.settings.save();
                }
                let (open_prefs, list_devices, open_processing) = (mic.open_prefs, mic.list_devices, mic.open_processing);
                if list_devices {
                    self.options_ui.output_devices = aurora_audio::output_devices();
                    self.options_ui.input_devices = aurora_audio::input_devices();
                }
                if open_prefs {
                    self.panels.settings = true;
                    self.options_ui.open_audio(if open_processing {
                        ui::options::AudioSection::Processing
                    } else {
                        ui::options::AudioSection::Voice
                    });
                }
                a.chat = chat;
                if !matches!(side_action, BarAction::None) {
                    a.bar = side_action;
                }
                if !matches!(bar2, BarAction::None) {
                    a.bar = bar2;
                }
                t = self.ui_parts.lap(Part::UiBars, t);
                // overlays
                self.draw_hud_view_bounds(&ctx);
                self.draw_hud_scale(&ctx);
                self.ik_editor.sync_build(
                    &self.world,
                    &self.build,
                    &self.settings.build,
                    self.cursor_pos,
                    crate::build::Mods {
                        shift: self.shift,
                        ctrl: self.ctrl,
                        alt: self.alt,
                    },
                );
                self.ik_editor.show(&ctx, &p, &self.world, projector);
                if !self.ik_editor.posing() {
                    self.ik_editor.request_object_data(&mut self.world, Instant::now());
                    self.build.out.extend(self.ik_editor.take_prim_requests());
                    self.build.out.extend(self.ik_editor.take_project_commands(self.world.agent_id));
                    if self
                        .ik_editor
                        .apply_prim_edit(&mut self.world, &mut self.build, &mut self.settings.build)
                    {
                        self.settings.save();
                    }
                }
                for prepared in self.ik_editor.take_clip_imports() {
                    self.imports.authored_animation(
                        &self.world,
                        &self.settings.inventory,
                        prepared.name,
                        prepared.bytes,
                        prepared.animation,
                    );
                }
                {
                    let mods = crate::build::Mods {
                        shift: self.shift,
                        ctrl: self.ctrl,
                        alt: self.alt,
                    };
                    // Focused web/media pages already received the native
                    // keys; their history must not also change world objects.
                    if self.media.focus.is_none() && self.profile_ui.focused_web().is_none() && self.search_ui.focused_web().is_none() {
                        contents_actions.extend(ui::object_actions::contents_shortcuts(
                            &ctx,
                            &self.interactions,
                            &self.world,
                            &self.build,
                        ));
                        crate::build::ui::shortcuts(&ctx, &mut self.build, &mut self.world, &mut self.settings.build);
                    }
                    self.build
                        .draw_overlay(&ctx, &self.palette, &self.world, &self.scene, &self.settings.build, mods);
                    let mut env = crate::build::ui::Env {
                        images: &self.ui_images,
                        media: &mut self.media,
                        scene: &mut self.scene,
                        zoom_fraction: self.camera.zoom_fraction(),
                        property_lines: &mut self.settings.world_overlays.property_lines,
                        demo: self.demo,
                    };
                    self.settings.build.show_parcel_owners = self.settings.world_overlays.land_owners;
                    if crate::build::ui::show(&ctx, &p, &mut self.build, &mut self.world, &mut self.settings.build, mods, &mut env) {
                        self.settings.save();
                    }
                    self.settings.world_overlays.land_owners = self.settings.build.show_parcel_owners;
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
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("xhair")));
                    // LLToolGun::draw: in the middle of the world view; the
                    // grab tool that Alt brings draws none
                    if !self.alt_cursor.free() {
                        let demo = ui::crosshair::demo(&self.settings.crosshair);
                        let crosshair = demo.as_ref().unwrap_or(&self.settings.crosshair);
                        let picture = ui::crosshair::picture(&ctx, crosshair);
                        ui::crosshair::draw(&painter, c, crosshair, picture.as_ref());
                    }
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
                    // FSBubblesHideConsoleAndToasts: the residents' lines are
                    // in their bubbles
                    let residents = !(self.chat_bubbles() && self.settings.chat_bubbles_hide_console);
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
                        // AURORA_DEMO_CHAT_LINES=legacy: the one-line look
                        // for the session, nothing saved
                        !(self.settings.classic_chat_lines || crate::demo::chat_lines_scenario() == Some(true)),
                        self.chat_ui.compact_automated,
                        residents,
                    );
                }
                // the hidden interface: how to bring it back (the islands), or
                // Firestorm's question before hiding it (its bars)
                let now = ctx.input(|i| i.time);
                if !classic_bars
                    && !chrome.top
                    && let Some(left) = self.interface.hint_left(now)
                {
                    let binding = self.settings.keybinds.get(crate::keybinds::Action::ToggleInterface)[0].clone();
                    ui::interface::hint(&ctx, &p, binding.as_ref());
                    ctx.request_repaint_after_secs(left as f32);
                }
                if self.interface.confirm {
                    let keys = self.interface_keys();
                    if let Some(hide) = ui::interface::confirm(&ctx, &p, &keys, &mut self.interface.confirm_again) {
                        self.interface.confirm = false;
                        if hide {
                            self.settings.confirm_hide_interface = self.interface.confirm_again;
                            self.settings.save();
                            a.bar = BarAction::HideInterface;
                        }
                    }
                }
                if self.interface.immersive && !self.camera.mouselook() {
                    let huds = self.settings.show_huds.then_some(self.settings.immersive_huds);
                    match ui::interface::immersive_buttons(&ctx, &p, self.settings.immersive_name_tags, huds) {
                        Some(ui::interface::ImmersiveAction::Leave) => a.bar = BarAction::ToggleImmersive,
                        Some(ui::interface::ImmersiveAction::NameTags(on)) => {
                            self.settings.immersive_name_tags = on;
                            self.settings.save();
                        }
                        Some(ui::interface::ImmersiveAction::Huds(on)) => a.bar = BarAction::ImmersiveHuds(on),
                        None => {}
                    }
                }
                // AURORA_DEMO_BARS=hidden / peek: hidden at frame 600 (asked
                // first with Firestorm's bars); `immersive`: Conversations
                // opened at frame 300, immersive mode at 600 (`immersivetags`:
                // the name tags shown, `immersivehuds`: the HUDs hidden, not
                // saved)
                if demo_hidden.is_some() && ctx.cumulative_frame_nr() == 600 && !self.interface.hidden {
                    a.bar = BarAction::ToggleInterface;
                }
                if self.demo && ui::bars::demo::get().immersive {
                    match ctx.cumulative_frame_nr() {
                        300 => ui::widgets::open_floater(&ctx, "conversations", &mut self.panels.chat),
                        600 if !self.interface.hidden => {
                            self.settings.immersive_name_tags = ui::bars::demo::get().immersive_tags;
                            self.settings.immersive_huds = !ui::bars::demo::get().immersive_no_huds;
                            a.bar = BarAction::ToggleImmersive;
                        }
                        _ => {}
                    }
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
                self.chat_ui.set_style(self.settings.chat_style);
                self.chat_ui.voice_on = self.settings.audio.voice_enabled;
                self.chat_ui.conversation_voice_indicator = self.settings.audio.conversation_voice_indicator;
                self.chat_ui.mic = (!self.settings.audio.mic_hold).then_some(self.mic_on);
                if self.world.calls.active.is_some() {
                    self.chat_ui.volumes.clone_from(&self.world.calls.volumes);
                    self.chat_ui.output_devices.clone_from(&self.options_ui.output_devices);
                    self.chat_ui.output_device.clone_from(&self.settings.audio.output_device);
                }
                self.chat_ui.timestamps = self.settings.chat_timestamps;
                // AURORA_DEMO_OBJECT_CHAT=…:on: for the run, nothing saved
                self.chat_ui.compact_automated = self.settings.compact_automated_chat || (self.demo && crate::demo::object_chat::on());
                self.chat_ui.compact_im_notices =
                    self.settings.compact_im_system_lines || (self.demo && crate::demo::object_chat::scene().is_some_and(|s| s.im_lines));
                self.chat_ui.show_channel = self.settings.show_chat_channel || self.chat_ui.demo_channel;
                self.chat_ui.send_typing = self.settings.send_typing_state;
                self.chat_ui.typing_chevron = self.settings.typing_chevron;
                self.chat_ui.flash_unread = self.settings.flash_unread_conversations;
                self.chat_ui.own_windows = self.settings.conversations_own_windows
                    || (self.demo && crate::demo::popout_scenario() == Some(crate::demo::Popout::Option));
                // AURORA_DEMO_CONV_POPOUT: at frame 640, the friend's IM and
                // the group popped out, or the group opened with the option
                // on, their windows at the left of the Conversations window
                if self.demo
                    && self.frame_count == 640
                    && let Some(mode) = crate::demo::popout_scenario()
                {
                    let size = egui::vec2(394.0, 332.0);
                    let group = Some(crate::demo::DEMO_GROUP1);
                    match mode {
                        crate::demo::Popout::Hand => {
                            self.chat_ui
                                .demo_pop_out(&ctx, Some(crate::demo::DEMO_FRIEND), egui::pos2(40.0, 60.0), size);
                            self.chat_ui.demo_pop_out(&ctx, group, egui::pos2(40.0, 420.0), size);
                        }
                        crate::demo::Popout::Option => {
                            self.world.social.focus_im = group;
                            ui::chat::demo_place_window(&ctx, group, egui::pos2(40.0, 60.0), size);
                        }
                        crate::demo::Popout::Nearby | crate::demo::Popout::NearbyOut => {
                            self.world.social.ims.clear();
                            self.world.social.focus_im = None;
                            self.chat_ui.selected = None;
                            self.chat_ui.contacts = false;
                            if mode == crate::demo::Popout::NearbyOut {
                                self.chat_ui.demo_pop_out(&ctx, None, egui::pos2(40.0, 60.0), size);
                            }
                        }
                    }
                }
                // the cards under links, their videos played by the media
                // plugins' browser
                self.chat_ui.previews.settings.clone_from(&self.settings.link_previews);
                self.chat_ui.previews.demo = self.demo && crate::demo::previews_offline();
                // the GIF picker of the fields (#932): made here in the demo's
                // GIF scenarios, the real settings and keys otherwise
                self.chat_ui.gifs.settings.clone_from(&self.settings.gifs);
                self.chat_ui.gifs.demo = self.demo && crate::demo::gifs_scenario().is_some_and(crate::demo::Gifs::made_here);
                let volume = self.media_volume();
                self.chat_ui.previews.web = self.chat_ui.previews.playing().then(|| ui::chat::previews::WebEnv {
                    paths: self.media.plugins(&self.settings.media).cloned(),
                    browser: crate::media::browser_settings(&crate::settings::cache_dir(), aurora_i18n::language()),
                    volume,
                });
                // who hears the nearby chat, and who of them types (the
                // radar's avatars in chat range, `World::is_typing_nearby`)
                self.chat_ui.nearby_count = 0;
                self.chat_ui.nearby_typing.clear();
                for e in self.radar.entries() {
                    if !e.left_behind && e.range.is_some_and(|d| d <= crate::world::radar::CHAT_RANGE) {
                        self.chat_ui.nearby_count += 1;
                        if e.typing {
                            self.chat_ui.nearby_typing.push(e.id);
                        }
                    }
                }
                // AURORA_DEMO_CONV=aurora / bubbles: someone types in the
                // conversation shown (the friend's tab says so in the window
                // of tabs)
                match crate::demo::bubbles_scenario() {
                    Some(crate::demo::Bubbles::Local) if crate::demo::aurora_scenario().is_some() => {
                        self.chat_ui.nearby_count = self.chat_ui.nearby_count.max(3);
                        self.chat_ui.nearby_typing = vec![crate::demo::DEMO_NOVA];
                    }
                    Some(crate::demo::Bubbles::Friend | crate::demo::Bubbles::Legacy) => {
                        self.world.social.set_typing(crate::demo::DEMO_FRIEND, true)
                    }
                    _ => {}
                }
                // AURORA_DEMO_FLASH: Tess writes now and then
                if self.demo
                    && let Some(ev) = crate::demo::flash_message(self.world.agent_id, self.radar_clock.elapsed().as_secs_f64())
                {
                    self.world.apply(ev);
                }
                // AURORA_DEMO_SNOOZE: Tess writes in the group chat, which
                // `auto` snoozes by itself
                if self.demo && self.in_world() {
                    if let Some(ev) = crate::demo::snooze_message(self.world.agent_id) {
                        self.world.apply(ev);
                    }
                    if crate::demo::snooze_now() {
                        Self::ask_or_snooze(
                            &self.settings,
                            self.demo,
                            &mut self.world,
                            &mut self.chat_ui,
                            crate::demo::DEMO_GROUP1,
                        );
                    }
                }
                // AURORA_DEMO_CONV_SCROLL=new: the friend writes while we read
                // higher up
                if self.demo {
                    crate::demo::conv_scroll_new(&mut self.world, self.chat_ui.demo_scrolled);
                }
                // AURORA_DEMO_CONV=aurora: the group's call rings without us
                if let Some(scene) = crate::demo::aurora_scenario() {
                    // call-away: the call opened its conversation; once it
                    // is connected the nearby chat comes back
                    static AWAY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    if scene.away
                        && self
                            .world
                            .calls
                            .active
                            .as_ref()
                            .is_some_and(|c| c.state == crate::world::calls::CallState::Connected)
                        && !AWAY.swap(true, std::sync::atomic::Ordering::Relaxed)
                    {
                        self.chat_ui.selected = None;
                    }
                    if scene.call == Some(crate::demo::AuroraCall::Ongoing)
                        && self.world.calls.invite_for(&crate::demo::DEMO_GROUP1).is_some()
                    {
                        self.world
                            .answer_call_invite(crate::demo::DEMO_GROUP1, crate::world::calls::InviteAnswer::Decline);
                    }
                }
                // Conversations › Contacts in Aurora's look of People: its
                // friends tab
                let people_aurora = !self.settings.classic_people && !(self.demo && crate::demo::people::legacy());
                let friends_before = self.settings.friends_list.clone();
                let contacts_radar = ui::people::RadarData {
                    radar: &self.radar,
                    settings: &mut self.settings.radar,
                    voice: &voice_levels,
                    draw_distance: self.settings.draw_distance,
                    now: self.radar_clock.elapsed().as_secs_f64(),
                    pics: &self.avatar_pics,
                };
                let friends = people_aurora.then_some(ui::people::FriendsPane {
                    st: &mut self.contacts_people_ui,
                    data: &contacts_radar,
                    settings: &mut self.settings.friends_list,
                    images: &self.ui_images,
                });
                let mut contacts_people = Vec::new();
                let mut snoozes = Vec::new();
                let conversation_actions = ui::chat::show(
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
                    self.settings.show_contacts_tab,
                    friends,
                    &mut open,
                );
                // the torn-off Contacts window, in the look of the tab it
                // left (#961): the friends pane where Conversations shows it
                // (Aurora's People in the workspace), the contact list
                // otherwise
                if self.settings.contacts.torn_off {
                    let mut contacts_open = self.panels.contacts;
                    let pics = ui::contacts::Pics {
                        avatars: &self.avatar_pics,
                        images: &self.ui_images,
                    };
                    let workspace = self.chat_ui.demo_style.unwrap_or(self.chat_ui.style).workspace();
                    let friends = (people_aurora && workspace).then_some(ui::people::FriendsPane {
                        st: &mut self.contacts_people_ui,
                        data: &contacts_radar,
                        settings: &mut self.settings.friends_list,
                        images: &self.ui_images,
                    });
                    let (contacts, people) = ui::contacts::window(
                        &ctx,
                        &p,
                        &mut self.world,
                        &mut self.contacts_ui,
                        &mut self.settings.contacts,
                        &pics,
                        friends,
                        &mut contacts_open,
                    );
                    contact_actions.extend(contacts);
                    contacts_people.extend(people);
                    self.panels.contacts = contacts_open;
                }
                for c in conversation_actions {
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
                        ui::chat::ConvAction::ToggleMic => {
                            // toggle_mic, written out: the skin's icons stay borrowed
                            if self.speak_denied().is_none() {
                                self.mic_on = !self.mic_on;
                            }
                        }
                        ui::chat::ConvAction::AnswerCall { session, answer } => self.world.answer_call_invite(session, answer),
                        ui::chat::ConvAction::PickPeople(conversation) => self.call_invite_ui.pick_for(conversation),
                        ui::chat::ConvAction::AddPeople(conversation, ids) => self.call_invite_ui.add(&mut self.world, conversation, ids),
                        ui::chat::ConvAction::SetVolume(id, v) => {
                            self.world.calls.set_volume(id, v);
                            // saved once the slider rests
                            self.volumes_save_at = Some(Instant::now() + std::time::Duration::from_millis(500));
                        }
                        ui::chat::ConvAction::Snooze(id) => snoozes.push(id),
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
                        ui::chat::ConvAction::People(pa) => contacts_people.push(pa),
                        ui::chat::ConvAction::OpenIm(id) => self.world.start_text_conference(vec![id]),
                        ui::chat::ConvAction::StartGroupChat(group) => self.world.start_group_chat(group),
                        // LLIMModel::sendTypingState: to the resident, in
                        // the session of the two of us (agent XOR other)
                        ui::chat::ConvAction::Typing { to, typing } => a.typing.push(NetCommand::SendImDialog {
                            to,
                            dialog: if typing {
                                crate::world::IM_TYPING_START
                            } else {
                                crate::world::IM_TYPING_STOP
                            },
                            id: uuid::Uuid::from_u128(self.world.agent_id.as_u128() ^ to.as_u128()),
                            message: "typing".to_owned(),
                            bucket: Vec::new(),
                        }),
                        // the inventory's Share dialog confirms, then gives
                        // (LLGiveInventory::doGiveInventoryItem)
                        ui::chat::ConvAction::GiveItems { to, items } => {
                            self.inventory_ui.dialog = Some(ui::inventory::EditDialog::Share(items, to));
                        }
                        // shown and in front even when already open
                        // (LLAvatarActions::share → showInstance), else the
                        // menu entry seems to do nothing (#627)
                        ui::chat::ConvAction::OpenInventory => {
                            ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory);
                            self.egui_ctx
                                .move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("inventory")));
                        }
                        ui::chat::ConvAction::SetOutputDevice(device) => {
                            self.settings.audio.output_device = device;
                            self.settings.save();
                        }
                        ui::chat::ConvAction::ListOutputDevices => {
                            self.options_ui.output_devices = aurora_audio::output_devices();
                        }
                        // done by the window before it returns
                        ui::chat::ConvAction::PopOut(_) | ui::chat::ConvAction::DockBack(_) | ui::chat::ConvAction::PopOutAt(..) => {}
                    }
                }
                self.panels.chat = open;
                for group in snoozes {
                    Self::ask_or_snooze(&self.settings, self.demo, &mut self.world, &mut self.chat_ui, group);
                }
                if let Some((group, minutes)) = ui::chat::snooze::dialog(&ctx, &p, &self.world, &mut self.chat_ui) {
                    Self::snooze_group_chat(&mut self.world, &mut self.chat_ui, group, minutes.saturating_mul(60));
                }
                // a link's menu hid the previews of its site
                if let Some(site) = self.chat_ui.previews.hide_site.take()
                    && !self.settings.link_previews.hidden_sites.contains(&site)
                {
                    self.settings.link_previews.hidden_sites.push(site);
                    self.settings.save();
                }
                // the player's own volume slider moved: the next video
                // starts there (#848); saved with the settings, like the
                // volumes of Sound and voice
                if let Some(volume) = crate::link_preview::take_video_volume() {
                    log::info!("link previews: video volume kept at {:.0} %", volume * 100.0);
                    self.settings.link_previews.video_volume = volume;
                }
                // a player's cog: the video settings
                if std::mem::take(&mut self.chat_ui.previews.open_settings) {
                    self.panels.settings = true;
                    self.options_ui.open_videos();
                }
                // the GIF picker's favourites, its last tab or its random id
                if std::mem::take(&mut self.chat_ui.gifs.changed) {
                    self.settings.gifs.clone_from(&self.chat_ui.gifs.settings);
                    self.settings.save();
                }
                if std::mem::take(&mut self.chat_ui.gifs.open_settings) {
                    self.chat_ui.gif_open = false;
                    self.panels.settings = true;
                    self.options_ui.open_gifs();
                } // a YouTube video playing in its window, or a card's that
                // stopped being drawn
                self.chat_ui.previews.player_window(&ctx, &p);
                if self.settings.friends_list != friends_before {
                    self.settings.save();
                }
                if let Some(ui::chat::history::HistoryAction::OpenFolder(dir)) = self.history_ui.show(
                    &ctx,
                    &p,
                    icons,
                    &mut self.emoji,
                    &self.avatar_pics,
                    &self.world,
                    &mut self.chat_ui.wanted_names,
                ) && let Err(e) = crate::logging::open_directory(&dir)
                {
                    log::warn!("cannot open the transcripts folder: {e}");
                }
                // their confirmations, also asked from the other menus
                contact_actions.extend(ui::contacts::dialog_windows(&ctx, &p, &mut self.world, &mut self.contacts_ui));
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
                // the radar's menu shows and sets the seven alert sounds of
                // the preferences at once
                self.settings.radar.alert_sounds = self.settings.audio.ui.radar_sounds();
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
                        images: &self.ui_images,
                    },
                    &mut people_map,
                    !self.settings.classic_people && !(self.demo && crate::demo::people::legacy()),
                );
                if self.settings.radar.alert_sounds != people_before.0.alert_sounds {
                    self.settings.audio.ui.set_radar_sounds(self.settings.radar.alert_sounds);
                }
                if people_before != (self.settings.radar.clone(), self.settings.friends_list.clone()) {
                    self.settings.save();
                }
                // (and those of Conversations › Contacts, the same tab)
                for pa in people.into_iter().chain(contacts_people) {
                    match pa {
                        ui::people::PeopleAction::OpenIm(id) => {
                            self.open_conversation_to_type(id);
                        }
                        ui::people::PeopleAction::OfferTeleport(id) => a.offer_tp.push(id),
                        ui::people::PeopleAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
                        ui::people::PeopleAction::TooFarToZoom => {
                            self.world.system_message(aurora_i18n::tr!("radar-too-far-to-zoom"));
                        }
                        ui::people::PeopleAction::GroupChat(id) => {
                            self.world.ui_sounds.push(UiSound::StartIm);
                            self.world.start_group_chat(id);
                            self.show_conversations();
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
                        ui::people::PeopleAction::GroupTitle { group, role } => {
                            if !self.world.rlva.has("setgroup") {
                                self.world.group_data.set_title(group, role);
                            }
                        }
                        ui::people::PeopleAction::GroupFlags { group, notices, profile } => {
                            self.world.groups.set_flags(group, notices, profile)
                        }
                        ui::people::PeopleAction::InviteToGroup(group) => self.group_profile_ui.open_with_invite(&mut self.world, group),
                    }
                }
                t = self.ui_parts.lap(Part::UiPeople, t);
                self.poll_inventory_animation_stats();
                self.show_imports(&ctx, &p);
                self.show_blender_live(&ctx, &p);
                let inventory_preferences = self.settings.inventory.clone();
                let import_preferences = self.settings.imports.clone();
                let mut open = self.panels.inventory;
                if !self.inventory_ui.merchant_requested {
                    self.inventory_ui.merchant_requested = true;
                    self.send(NetCommand::RequestInventoryMerchant);
                }
                self.prepare_inventory_marketplace_commands();
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
                    &self.inventory_hover_images.ready,
                    &self.avatar_pics,
                    &self.settings.script_editor,
                    // the inventory scenarios were written for the Legacy window,
                    // but for Aurora's own
                    if self.demo && ui::inventory::aurora_demo_active() {
                        false
                    } else {
                        self.settings.classic_inventory || (self.demo && ui::inventory::legacy_demo())
                    },
                ) {
                    match ia {
                        ui::inventory::InvAction::TeleportLandmark(asset) => a.landmark = Some(asset),
                        other => self.apply_inventory_action(other),
                    }
                }
                self.panels.inventory = open;
                ui::inventory::default_permissions::show(
                    &ctx,
                    &p,
                    &mut self.inventory_ui.default_permissions_open,
                    &mut self.settings.inventory.creation_permissions,
                    &mut self.settings.imports,
                );
                ui::inventory::view::set_picker_sort(&ctx, self.settings.inventory.sort);
                self.show_gestures(&ctx, &p);
                self.show_texture_library(&ctx, &p);
                self.inventory_ui.preferences_dirty |= self.settings.inventory != inventory_preferences;
                self.inventory_ui.preferences_dirty |= self.settings.imports != import_preferences;
                if self.inventory_ui.preferences_dirty && !ctx.input(|input| input.pointer.any_down()) {
                    self.inventory_ui.preferences_dirty = false;
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
                self.appearance_ui.fill_previews = self.settings.appearance_fill_previews;
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
                if self.settings.appearance_fill_previews != self.appearance_ui.fill_previews {
                    self.settings.appearance_fill_previews = self.appearance_ui.fill_previews;
                    self.settings.save();
                }
                self.panels.appearance = open;
                self.update_demo_shape_mesh();
                self.shape_ui.update(&mut self.world, &self.scene, &ctx);
                let shape_preferences = (self.shape_ui.hints, self.shape_ui.feet);
                let shape_group = self.shape_ui.group;
                let (shape_actions, visible) =
                    ui::shape::show(&ctx, &p, &mut self.world, &mut self.shape_ui, self.settings.customize_animation);
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
                    && !self.wearable_ui.open
                    && let Some(camera) = self.shape_camera.take()
                {
                    self.camera = camera;
                }
                self.physics_window(&ctx, &p);
                self.wearable_window(&ctx, &p);
                self.ao_window(&ctx, &p);
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
                if ui::context::show(
                    &ctx,
                    &p,
                    &mut self.context_menu,
                    &facts,
                    use_pie,
                    &mut self.settings.world_context_menus,
                    &mut self.context_customization,
                ) {
                    self.settings.save();
                }
                if had_menu && self.context_menu.is_none() {
                    // LLViewerMenuHolderGL::hideMenus drops the menu's selection
                    self.build.release_menu_selection(&self.world);
                    self.flush_build();
                }
                if let Some(command) =
                    self.context_customization
                        .notecards
                        .show(&ctx, &p, &mut self.world.inventory, &self.settings.world_context_menus)
                {
                    self.send(command);
                }
                self.object_menu_ui(&ctx, &p);
                self.avatar_textures.show(&ctx, &p, &self.world, &self.ui_images);
                self.object_profile_window(&ctx, &p);

                if let Some(command) = self.pathfinding_ui.show(&ctx, &p, &mut self.world) {
                    if self.demo {
                        self.pathfinding_ui.demo(&self.world, command);
                    } else {
                        self.send(command);
                    }
                }
                self.land_moderation_ui(&ctx, &p);
                if let Some((ids, text)) = self.delete_confirm.clone()
                    && let Some(yes) = ui::context::delete_confirm(&ctx, &p, &text)
                {
                    self.delete_confirm = None;
                    match ids.as_slice() {
                        [id] if yes => ui::context::request(&ctx, ui::context::CtxAction::DeleteConfirmed(*id)),
                        _ if yes => self.delete_objects(&ids),
                        _ => {}
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
                self.media_filter_question(&ctx, &p);
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
                self.show_money_tracker(&ctx, &p, &mut a);
                for action in contents_actions {
                    self.apply_contents_action(action);
                }
                for action in
                    ui::object_actions::show_contents(&ctx, &p, &self.skin.icons, &mut self.interactions, &self.world, &self.build)
                {
                    self.apply_contents_action(action);
                }
                self.show_task_notecards(&ctx, &p);
                self.show_scripts(&ctx, &p);
                for action in ui::inventory::task_drop::confirm(&ctx, &p, &mut self.task_transfer) {
                    self.apply_inventory_action(action);
                }
                if let Some(confirm) = ui::object_actions::confirm_copy(&ctx, &p, &mut self.contents_copies.confirm) {
                    self.contents_copy_confirmed(confirm);
                }
                // a teleport invitation or an inventory offer whose card is in
                // sight needs no toast (LLOfferHandler: a toast only when the
                // IM floater is closed; or its own window, `popout.rs`)
                let mut open: Vec<uuid::Uuid> = self.chat_ui.selected.filter(|_| self.panels.chat).into_iter().collect();
                open.extend(ui::chat::conversations_in_sight(&ctx, &self.chat_ui, false).into_iter().flatten());
                if !open.is_empty() {
                    let seen: Vec<u64> = self
                        .world
                        .teleport_invites
                        .pending_notifications()
                        .chain(self.world.inventory_offers.pending_notifications())
                        .filter(|(_, from)| open.contains(from))
                        .map(|(n, _)| n)
                        .collect();
                    for n in self.world.notifications.list.iter_mut().filter(|n| seen.contains(&n.id)) {
                        n.read = true;
                    }
                }
                // nothing over the view in mouselook or the photo's full frame;
                // in do not disturb only the script menus come out, in both
                // styles (#491)
                let aurora = self.notifications_aurora();
                let dnd = self.world.status.dnd;
                let toasts_hidden = self.camera.mouselook() || self.photo_full_frame();
                self.world.notifications.aurora = aurora;
                // #465: a friendship offer in the middle of the screen, before
                // the toasts it replaces (AURORA_DEMO_FRIEND_PROMPT: on for the run)
                let prompt_on = self.settings.friend_request_prompt || (self.demo && crate::demo::friend_prompt::mode().is_some());
                let prompt_allowed = !toasts_hidden && !dnd && self.world.notifications.started;
                // #1404: a message to the whole region there too, first: the
                // friendship offers wait behind it
                // (AURORA_DEMO_REGION_MESSAGE: on for the run)
                let region_on = self.settings.region_message_prompt || (self.demo && crate::demo::region_message::enabled());
                let mut region_action = None;
                let region_shown = self
                    .region_message_prompt
                    .next(&mut self.world.notifications, region_on, prompt_allowed);
                if let Some((id, from)) = region_shown {
                    let name = Some(self.world.social.display_name_of(&from))
                        .filter(|n| n != "…")
                        .or_else(|| self.world.avatar_name(&from));
                    if name.is_none() {
                        self.world.social.want_name(from);
                    }
                    let env = ui::region_message_prompt::PromptEnv {
                        name,
                        motion: !self.settings.reduce_motion,
                    };
                    region_action = self
                        .region_message_prompt
                        .show(&ctx, &p, &self.skin.icons, &self.world.notifications, id, &env);
                }
                let mut prompt_action = None;
                let friend_allowed = prompt_allowed && region_shown.is_none();
                if let Some((id, from)) = self.friend_prompt.next(&mut self.world.notifications, prompt_on, friend_allowed) {
                    self.notif_ui.wanted_pics.insert(from);
                    let name = Some(self.world.social.display_name_of(&from))
                        .filter(|n| n != "…")
                        .or_else(|| self.world.avatar_name(&from));
                    if name.is_none() {
                        self.world.social.want_name(from);
                    }
                    let env = ui::friend_prompt::PromptEnv {
                        pics: &self.avatar_pics,
                        name,
                        motion: !self.settings.reduce_motion,
                    };
                    prompt_action = self
                        .friend_prompt
                        .show(&ctx, &p, &self.skin.icons, &self.world.notifications, id, &env);
                }
                let restart = self.restart_bell_info(&p);
                // AURORA_DEMO_PRESENCE: a place for this run only
                let presence_at = crate::demo::presence::position()
                    .filter(|_| self.demo)
                    .unwrap_or(self.settings.presence_position);
                let classic_bars = self.bars_look().0;
                let mut toast_env = ui::notifications::ToastEnv {
                    pics: &self.avatar_pics,
                    images: &self.ui_images,
                    emoji: &mut self.emoji,
                    hidden: toasts_hidden,
                    message_seconds: f64::from(self.settings.im_toast_seconds),
                    restart,
                    presence_at,
                    classic_bars,
                };
                // the cards read the world for their links' names (#898): the
                // lists leave it while they are drawn, then come back
                let mut list = std::mem::take(&mut self.world.notifications);
                let mut hist = std::mem::take(&mut self.world.notification_history);
                let (mut answers, asks) = if aurora {
                    ui::notification_center::show(
                        &ctx,
                        &p,
                        &self.skin.icons,
                        &self.world,
                        &mut list,
                        &mut hist,
                        &mut self.notif_ui,
                        &mut toast_env,
                        dnd,
                    )
                } else {
                    ui::notifications::show(
                        &ctx,
                        &p,
                        &self.skin.icons,
                        &self.world,
                        &mut list,
                        &mut hist,
                        &mut self.notif_ui,
                        &mut toast_env,
                        dnd,
                    )
                };
                self.world.notifications = list;
                self.world.notification_history = hist;
                // the region restart over the toasts, under the open list
                self.show_region_restart(&ctx, &p);
                self.show_link_health(&ctx, &p);
                self.order_region_restart(&ctx, aurora);
                for ask in asks {
                    self.on_notification_ask(&ctx, ask);
                }
                match prompt_action {
                    Some(ui::friend_prompt::PromptAction::Answer(id, r)) => answers.push((id, r)),
                    Some(ui::friend_prompt::PromptAction::Profile(id)) => self.profile_ui.open(&mut self.world, id),
                    None => {}
                }
                match region_action {
                    Some(ui::region_message_prompt::PromptAction::Answer(id, r)) => answers.push((id, r)),
                    Some(ui::region_message_prompt::PromptAction::Profile(id)) => self.profile_ui.open(&mut self.world, id),
                    None => {}
                }
                for (id, r) in answers {
                    if self.region_restart_bell_answer(id, &r) {
                        continue;
                    }
                    if matches!(r, crate::world::notifications::Response::Open)
                        && let Some(object) = self.world.notifications.list.iter().find_map(|n| match n.data {
                            crate::world::notifications::Data::Delivery { object } if n.id == id => Some(object),
                            _ => None,
                        })
                    {
                        self.show_delivery(id, object);
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
                    for c in cmds {
                        if matches!(c, NetCommand::AcceptLure { .. }) {
                            self.request_user_teleport(c, String::new(), false);
                        } else {
                            self.send(c);
                        }
                    }
                    // only web links (llLoadURL / goto URL), opened after the user's click
                    if let Some(u) = url.filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
                        ctx.open_url(egui::OpenUrl::new_tab(u));
                    }
                }
                self.world.record_notification_history();
                t = self.ui_parts.lap(Part::UiMenus, t);
                let mut open = self.panels.about_land && self.world.rlva_can_view_parcel();
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
                    let id = if self.settings.classic_region_estate {
                        ui::region::CLASSIC_WINDOW
                    } else {
                        ui::region::WINDOW
                    };
                    ui::widgets::open_floater(&ctx, id, &mut self.panels.region_estate);
                }
                let mut open = self.panels.region_estate && self.world.rlva_can_view_region();
                if open {
                    self.feed_region_window(&ctx);
                }
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
                    Some(ui::region::RegionAction::CustomizeEnvironment) => self.open_region_day_editor(),
                    Some(ui::region::RegionAction::Deselect) => self.build.deselect_all(&self.world),
                    None => {}
                }
                // AURORA_DEMO_REGION: notecards for the covenant, once the
                // inventory is there
                if self.demo && open && !self.world.inventory.root.is_nil() && crate::demo::region::scenario().is_some() {
                    crate::demo::region::seed_notecards(&mut self.world.inventory, self.world.agent_id);
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
                // an `objectim` link clicked (LLInspectRemoteObject)
                ui::remote_object::show(&ctx, &p, &self.world, &mut self.remote_object);
                self.photo_ui(&ctx, &p, projector, renderer);
                self.controls_window(&ctx, &p);
                self.movelock_window(&ctx, &p);
                self.follow_window(&ctx, &p);
                self.portals_window(&ctx, &p);
                self.report_abuse_ui(&ctx, &p);
                self.particle_editor_window(&ctx, &p);
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
                self.blacklist_window(&ctx, &p);
                self.area_search_window(&ctx, &p);
                self.sound_explorer_window(&ctx, &p);
                self.animation_explorer_window(&ctx, &p);
                self.avatar_render_settings_window(&ctx, &p);
                self.bumps_ui.show(&ctx, &p, &self.world, &mut self.panels.bumps);
                self.grid_status_window(&ctx, &p);
                let shown = self.stream_title_ui.show(
                    &ctx,
                    &p,
                    &mut self.stream_titles,
                    &mut self.settings.audio,
                    &mut self.panels.stream_title,
                    &mut self.panels.stream_title_history,
                );
                if shown.settings {
                    self.settings.save();
                }
                ui::beacons::show(&ctx, &p, &mut self.panels.beacons, &mut self.settings);
                if ui::owned_land::show(&ctx, &p, &mut self.world, &mut self.panels.owned_land) {
                    ui::widgets::open_floater(&ctx, "worldmap", &mut self.panels.world_map);
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
                self.profile_ui.prepare_photos(&ctx, &mut self.scene, &self.world);
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
                    self.settings.profile_rights,
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
                            self.show_conversations();
                        }
                        // the conversations' give confirmation (#991)
                        ui::profile::ProfileAction::Give { to, items } => {
                            self.inventory_ui.dialog = Some(ui::inventory::EditDialog::Share(items, to));
                        }
                        // shown and in front, as the conversation's "Give an
                        // inventory item…"; RLVa @share refuses it first
                        // (LLAvatarActions::share)
                        ui::profile::ProfileAction::Share(id) => {
                            if self.world.rlva.allows("share", &id.to_string()) {
                                ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory);
                                self.egui_ctx
                                    .move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("inventory")));
                            } else {
                                self.world
                                    .system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
                            }
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
                let favorites_order = if self.panels.places && !self.settings.classic_places {
                    self.world.favorites.list(&self.world.inventory).to_vec()
                } else {
                    Vec::new()
                };
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
                // Aurora's look orders the favourites as the bar does
                let places_env = (!self.settings.classic_places).then(|| ui::places::aurora::Env {
                    sort: self.settings.places_sort,
                    favorites: &favorites_order,
                });
                let places_acts = self.places_ui.show(
                    &ctx,
                    &mut c,
                    &mut open,
                    self.settings.landmarks_by_date,
                    self.settings.confirm_teleports,
                    places_env.as_ref(),
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
                        PlacesAction::Edit(change) => self.apply_inventory_action(ui::inventory::InvAction::Edit(change)),
                        PlacesAction::ExpectFavorite => self.world.favorites.expect_added(),
                        PlacesAction::MoveFavorite(item, rank) => self.world.favorites.rearrange(&self.world.inventory, item, rank),
                        PlacesAction::OpenWindow(source) => {
                            let w = &mut self.world;
                            let serial = w.place_details.open_window(source, &w.map, &mut w.landmarks, Instant::now());
                            self.place_ui.raise(serial);
                        }
                        PlacesAction::Sort(by) => {
                            self.settings.places_sort = by;
                            self.settings.save();
                        }
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
                let search_acts = self.search_ui.show(
                    &ctx,
                    &mut c,
                    &self.avatar_pics,
                    &mut self.settings.search,
                    &search_web,
                    self.settings.classic_search,
                    &mut open,
                );
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
                let view = ui::perf::PerfView {
                    data: &self.perf,
                    gpu: &renderer.info,
                };
                let monitor_actions = ui::perf::show(&ctx, &p, &view, &mut self.monitor, &mut open);
                self.panels.perf = open;
                self.monitor_actions(monitor_actions);
                self.dev_toolbox_window(&ctx, &p, renderer);
                self.about_window(&ctx, &p, renderer);
                let region = self.world.region_name();
                let available = self.world.main_has_cap("SimConsoleAsync");
                let rlva_actions = self.options_ui.rlva_tools.show(&ctx, &p, &self.world);
                if rlva_actions.closed {
                    self.world.rlva.close_console();
                }
                if rlva_actions.clear_log {
                    self.world.rlva.diagnostics.entries.clear();
                }
                if let Some(input) = rlva_actions.input {
                    self.world.rlva_configure(self.settings.rlva_enabled);
                    self.world.rlva.preferences.clone_from(&self.settings.rlva);
                    self.world.rlva_console_input(&input);
                }
                ui::sim_console::show(
                    &ctx,
                    &p,
                    &mut self.panels.sim_console,
                    &region,
                    available,
                    &mut self.world.sim_console,
                    &mut self.sim_console_input,
                );
                t = self.ui_parts.lap(Part::UiPerf, t);
                // environment selector (Firestorm's quick preferences lists)
                if self.panels.environment {
                    if !self.env_scan {
                        self.env_scan = true;
                        self.env_ui.scan_left = usize::MAX;
                    }
                    self.env_ui.refresh(&ctx, &self.world.inventory);
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
                self.startup_windows.remember(&self.panels, &mut self.settings);
                self.chat_ui.remember_layout(&mut self.settings);
                self.settings.show_camera_controls = self.panels.camera_controls;
                self.settings.show_movement_controls = self.panels.movement_controls;
                self.settings.audio.stream_title_window = self.panels.stream_title;
                self.settings.show_people = self.panels.people;
                self.settings.people_tab = self.panels.people_tab;
                self.settings.show_inventory = self.panels.inventory;
                self.settings.show_appearance = self.panels.appearance;
                self.settings.show_ao = self.panels.ao;
                // (the photo's time of day is its own layer, never saved)
                if self.panels.time_of_day != self.settings.time_of_day {
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
        // Bug report form (disabled, see main.rs):
        // if self.demo && self.frame_count == 550 && std::env::var_os("AURORA_DEMO_BUG_REPORT").is_some() {
        //     self.bug_report.requested = true;
        // }
        // if self.bug_report.requested {
        //     self.open_bug_report(renderer);
        // }
        // if self.demo {
        //     self.bug_report.demo_step(self.frame_count);
        // }
        // self.bug_report
        //     .show(&ctx, &p, &self.skin.icons, matches!(self.screen, Screen::World));
        if self.demo && self.frame_count == 550 && std::env::var_os("AURORA_DEMO_GRIDS").is_some() {
            self.grids_ui.demo(&mut self.settings);
        }
        self.grids_ui.show(
            &ctx,
            &p,
            &mut self.settings,
            matches!(self.screen, Screen::Login) && !self.login_form.busy || self.demo,
            self.net.runtime(),
        );
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
    ui_scale_changed: bool,
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
    /// The typing notices of the conversations.
    typing: Vec<NetCommand>,
    /// Gestures played from the conversations' field.
    gestures: Vec<uuid::Uuid>,
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
        let mut attrs = crate::cli::with_desktop_id(Window::default_attributes())
            .with_title(crate::cli::window_title())
            // Metal needs a visible layer to present its first drawable.
            // Other platforms keep the unpainted client area hidden.
            .with_visible(cfg!(target_os = "macos"))
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
                graphics_start::report(&e.to_string(), capture);
                event_loop.exit();
                return;
            }
        };
        crate::window_hold::install(&window);
        // Cmd+Q asks like the close button (mac_quit.rs)
        #[cfg(target_os = "macos")]
        crate::mac_quit::route_quit_to_close();
        let size = window.inner_size();
        if std::env::var_os("AURORA_NOVSYNC").is_some() {
            self.settings.vsync = false;
        }
        // the GPU of Preferences › Graphics; AURORA_GPU=<number|name|auto>
        // overrides it for this launch (adapter_choice.rs)
        let gpu = std::env::var("AURORA_GPU").unwrap_or_else(|_| self.settings.gpu_adapter.clone());
        // AURORA_GPU_START_FAIL=1: as if no card could draw, to see the
        // message the user gets then (graphics_start.rs)
        let created = if std::env::var_os("AURORA_GPU_START_FAIL").is_some() {
            Err(aurora_render::RenderError::NoAdapter("AURORA_GPU_START_FAIL".into()))
        } else {
            Renderer::new(window.clone(), size.width, size.height, self.settings.vsync, &gpu)
        };
        let renderer = match created {
            Ok(r) => r,
            Err(e) => {
                log::error!("renderer initialisation failed: {e}");
                let message = if cfg!(target_os = "macos") {
                    aurora_i18n::tr!("app-aurora-viewer-unable-to-initialize-metal", e = format!("{}", e))
                } else {
                    aurora_i18n::tr!("app-aurora-viewer-unable-to-initialize-vulkan", e = format!("{}", e))
                };
                eprintln!("{message}");
                // the window is still hidden: without this the viewer just
                // vanished (#894)
                graphics_start::report(&e.to_string(), capture);
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
        // what Preferences › Graphics shows of the GPUs, from the renderer's
        // own list: here, not with the world's frames, so the login screen
        // has it too (#894: only "Automatic" was offered before login)
        self.options_ui.vram = (renderer.info.vram_mb, renderer.info.integrated);
        self.options_ui.gpus = renderer.info.adapters.clone();
        self.options_ui.gpu_in_use = renderer.info.name.clone();
        self.options_ui.gpu_notice = renderer.info.notice;
        self.login_form.driver_notice = self.driver_notice(&renderer.info);
        self.safe_graphics_at_start();
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
            if let Ok(modes) = std::env::var("AURORA_DEMO_VOICE_PREFS") {
                for mode in modes.split(',') {
                    let a = &mut self.settings.audio;
                    match mode.trim() {
                        "avatar" => a.voice_ear = crate::settings::VoiceEar::Avatar,
                        "raw" => {
                            a.voice_automatic_gain = false;
                            a.voice_echo_cancellation = false;
                            a.voice_noise_suppression = 0;
                        }
                        "no-lips" => a.lip_sync = false,
                        "no-indicators" => a.voice_visualizer = false,
                        "no-self" => a.voice_visualizer_self = false,
                        "no-dot" => a.voice_visualizer_dot = false,
                        "no-call-visuals" => a.voice_visualizers_in_calls = false,
                        "no-conversation" => a.conversation_voice_indicator = false,
                        _ => {}
                    }
                }
            }
            if self.update_demo.is_some() {
                // The update preview stays offline on a synthetic login screen.
                return;
            }
            // AURORA_DEMO_TOOLBOX=1, or "target:tool" (target 0 nearest, 1
            // selection, 2 under the cursor, 3 my avatar; tool: the one "Send a diagnostic"
            // shows, e.g. "0:J4"), or "edit" (Customize): the Dev Toolbox
            if let Ok(v) = std::env::var("AURORA_DEMO_TOOLBOX") {
                self.panels.dev_toolbox = true;
                self.panels.perf = false;
                if v == "edit" {
                    self.dev_toolbox.edit = true;
                } else if let Some((target, tool)) = v.split_once(':') {
                    if let Some(t) = target.parse::<usize>().ok().and_then(|i| crate::dev_toolbox::Target::ALL.get(i)) {
                        self.dev_toolbox.target = *t;
                    }
                    if let Some(t) = crate::dev_toolbox::tools::find(tool) {
                        self.dev_toolbox.selected = t.id;
                        self.toolbox.demo_press = Some(t.id);
                    }
                }
            }
            // before the login scenarios: with AURORA_DEMO_LOGIN the
            // preferences open over the login screen (its cogwheel)
            if let Ok(mode) = std::env::var("AURORA_DEMO_ANTI_SPAM") {
                let (id, clip) = crate::demo::chime();
                self.scene.sounds.insert(id, clip);
                self.settings.anti_spam = crate::world::anti_spam::Settings {
                    enabled: mode != "off",
                    global: mode == "global",
                    own_objects: mode == "text",
                    amount: if mode == "text" { 2 } else { 10 },
                    newlines: if mode == "text" { 5 } else { 70 },
                    ..Default::default()
                };
                self.world.anti_spam.configure(self.settings.anti_spam);
                self.panels.settings = true;
                self.options_ui.open(ui::options::Page::Protections, 0);
            }
            if std::env::var_os("AURORA_DEMO_OPTIONS").is_some() {
                self.panels.settings = true;
                self.panels.perf = false;
                let value = std::env::var("AURORA_DEMO_OPTIONS").unwrap_or_default();
                // the former Debug page: the Dev Toolbox
                if value.split(':').next() == Some("9") {
                    self.panels.settings = false;
                    self.panels.dev_toolbox = true;
                }
                if value.split(':').nth(1) == Some("animations-sync") {
                    self.settings.synchronize_animations = true;
                }
                if value == "avatars/1:group-titles" {
                    self.settings.group_titles = crate::world::names::GroupTitles::AllExceptMine;
                }
                if value == "inventory:recent-period" {
                    self.settings.inventory.recent_hours = 7 * 24;
                }
                // the confirmation of a restore, for a backup of the demo's
                // own profile
                if value.split(':').nth(1) == Some("restore") {
                    let mut backup = crate::settings::backup::Backup::new(&self.settings, ui::layout::text(&self.egui_ctx));
                    backup.read_files(&crate::settings::config_dir());
                    let logged_in = self.world.login.as_ref().map(|l| l.agent_id);
                    self.options_ui.restore = Some(ui::options::backup::Pending::new(backup, logged_in));
                }
                ui::options::demo(&mut self.options_ui, &value);
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
            self.scene.anims.insert(
                crate::world::radar::ANIM_AGENT_TYPE,
                Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::typing_animation(), &rig)),
            );
            if crate::demo::water::mode().is_some() {
                self.scene.anims.insert(
                    crate::demo::water::CONTACT_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::water::animation(), &rig)),
                );
            }
            if let Some(mode) = crate::demo::physics::mode() {
                self.scene.anims.insert(
                    crate::demo::physics::HOP_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::physics::animation(mode), &rig)),
                );
                if mode == crate::demo::physics::Mode::Off {
                    self.settings.avatar_physics = false;
                }
            }
            if crate::demo::deform::mode().is_some() {
                self.scene.anims.insert(
                    crate::demo::deform::DEFORM_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::deform::animation(&rig), &rig)),
                );
            }
            if let Some(mode) = crate::demo::emote::mode() {
                self.scene.anims.insert(
                    crate::demo::emote::EMOTE_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::emote::animation(mode), &rig)),
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
            if crate::demo::animation_sync_demo() {
                self.settings.synchronize_animations = true;
            }
            if crate::demo::animation_resync_demo() {
                self.settings.synchronize_animations = false;
            }
            if crate::demo::animation_sync_demo() || crate::demo::animation_resync_demo() {
                self.scene.anims.insert(
                    crate::demo::SYNC_LEADER_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::loop_animation(), &rig)),
                );
            }
            // AURORA_DEMO_LINK's stand-in simulator signals the stock walk,
            // run, fly and stand: drawn with the demo's own motions
            if self.demo_link_cfg.is_some() {
                for (id, anim) in crate::demo::link::animations() {
                    self.scene
                        .anims
                        .insert(id, Arc::new(crate::scene::anim::BoundAnim::bind(anim, &rig)));
                }
            }
            // the library skies of the time of day presets: nothing to fetch
            // them from offline, so the built-in sky with each preset's sun
            for preset in 1..=crate::world::env::TIME_OF_DAY_PRESETS {
                if let Some(id) = crate::world::env::time_of_day_sky(preset) {
                    let sky = crate::world::eep::SkyFrame::with_sun(crate::world::env::time_of_day_sun(preset));
                    self.scene.settings.insert_local(id, crate::world::eep::Settings::Sky(sky), None);
                }
            }
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
            // AURORA_DEMO_PERF=compact|full|network|monitor-<section>: the
            // performance window in that view (full: the resource monitor's
            // overview; network or monitor-<section>: that section of it)
            // a minute of synthetic traffic for the network's graphs
            self.perf.demo_history();
            if let Ok(v) = std::env::var("AURORA_DEMO_PERF") {
                self.panels.perf = true;
                let v = v.trim();
                if v == "network" {
                    ui::perf::show_network(&self.egui_ctx);
                } else if v == "monitor-scene-objects" {
                    ui::perf::set_full(&self.egui_ctx, true);
                    self.monitor.show_scene_objects();
                } else if let Some(tab) = v.strip_prefix("monitor-").and_then(ui::monitor::Tab::from_key) {
                    ui::perf::set_full(&self.egui_ctx, true);
                    ui::monitor::demo_tab(&self.egui_ctx, tab);
                } else {
                    ui::perf::set_full(&self.egui_ctx, v != "compact");
                }
            }
            if let Some(t) = std::env::var("AURORA_DEMO_TOD").ok().and_then(|v| v.parse::<u8>().ok()) {
                self.panels.time_of_day = t.min(crate::world::env::TIME_OF_DAY_PRESETS);
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
                        "pass" => d.pass_view = true,
                        _ => {}
                    }
                }
            }
            if std::env::var("AURORA_DEMO_PARTICLES").is_ok_and(|mode| matches!(mode.as_str(), "region-wind" | "region-calm")) {
                // Frame both plumes independently of the saved camera preset.
                self.panels.inventory = false;
                self.panels.ao = false;
                let ground = self.world.ground_height(glam::Vec3::new(136.0, 127.0, 0.0)).unwrap_or(20.0);
                self.camera.set_view(
                    glam::Vec3::new(125.0, 124.0, ground + 4.0),
                    glam::Vec3::new(137.0, 124.0, ground + 4.0),
                    &mut self.world.agent,
                    &self.settings.camera,
                );
                self.camera.snap();
            }
            if let Ok(mode) = std::env::var("AURORA_DEMO_BEACONS") {
                self.world.apply(crate::demo::beacons::events());
                let (id, clip) = crate::demo::chime();
                self.scene.sounds.insert(id, clip);
                self.panels.people = false;
                self.panels.minimap = false;
                self.panels.chat = false;
                self.panels.inventory = false;
                self.panels.ao = false;
                self.panels.beacons = mode != "parcels" && mode != "owned";
                self.settings.beacons = crate::world::beacons::Beacons {
                    lines: !matches!(mode.as_str(), "off" | "highlights" | "parcels" | "owned"),
                    highlights: !matches!(mode.as_str(), "off" | "lines" | "parcels" | "owned"),
                    scripted: true,
                    physical: true,
                    sound: true,
                    particles: true,
                    media: true,
                    ..Default::default()
                };
                self.settings.world_overlays.property_lines = mode == "parcels";
                self.settings.world_overlays.land_owners = mode == "parcels";
                self.settings.world_overlays.hide_particles = mode == "hidden-particles";
                if mode == "parcels" {
                    for event in crate::demo::beacons::overlay_events() {
                        self.world.apply(event);
                    }
                }
                if mode == "owned" {
                    self.panels.owned_land = true;
                    self.world.group_money.request_owned_land();
                }
            }
            // AURORA_DEMO_CAM="yaw,pitch,distance" (radians, meters) for captures
            if let Ok(v) = std::env::var("AURORA_DEMO_CAM") {
                let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if p.len() >= 2 {
                    self.camera.demo_orbit(p[0], p[1], p.get(2).copied(), &self.settings.camera);
                }
            }
            if crate::demo::tags::apply_name_tag_preferences(&mut self.settings) {
                self.panels.ao = false;
            }
            // AURORA_DEMO_TAGS: residents whose name plates show every
            // state, at the three distance levels
            if crate::demo::tags::mode().is_some() {
                crate::demo::tags::apply_render_preferences(&mut self.settings);
                for ev in crate::demo::tags::events() {
                    self.world.apply(ev);
                }
                // (nothing over the rows of plates)
                self.panels.people = false;
            }
            // AURORA_DEMO_RADAR=1|tp|walk|friends|groups|profile: residents
            // around for the radar of People (tp: with the ones of a region a
            // teleport just left; walk: with one who comes and goes), or its
            // friends / groups tab (profil: with the profile of a group)
            if let Ok(v) = std::env::var("AURORA_DEMO_RADAR") {
                self.demo_radar_tp = v == "tp";
                self.demo_radar_walk = v == "walk";
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
                if let Ok(filter) = std::env::var("AURORA_DEMO_PEOPLE_FILTER") {
                    self.people_ui.filter = filter;
                }
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
                    if crate::demo::people::visuals() {
                        self.contacts_ui.configure_set(aurora_i18n::tr!("demo-family"));
                        self.settings.classic_name_tags = std::env::var("AURORA_DEMO_PEOPLE").is_ok_and(|v| v == "visuals-legacy");
                    }
                }
                self.panels.people = true;
                self.panels.people_tab = crate::demo::people::tab();
                self.panels.inventory = false;
                self.panels.chat = false;
                self.panels.minimap = false;
                self.panels.perf = false;
            }
            // AURORA_DEMO_CHAT_LINES: the lines above the chat bar, the
            // Conversations window closed (it hides them while it shows the
            // local chat)
            if crate::demo::chat_lines_scenario().is_some() {
                self.panels.chat = false;
                self.panels.people = false;
                self.panels.inventory = false;
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
            // (after the tab, ":wide": Aurora's window widened to its rail and
            // preview; ":open": its first place selected; both: ":wide,open")
            let demo_places = std::env::var("AURORA_DEMO_PLACES").unwrap_or_default();
            let (demo_tab, demo_how) = demo_places.split_once(':').unwrap_or((demo_places.as_str(), ""));
            if let Some(tab) = ui::places::tab_from_name(demo_tab) {
                self.places_ui.open_tab(tab);
                self.panels.places = true;
                self.panels.perf = false;
                if demo_how.split(',').any(|w| w == "wide") {
                    ui::widgets::size_floater(&self.egui_ctx, "places", egui::vec2(1040.0, 640.0));
                }
                self.places_ui.aurora.demo_open = demo_how.split(',').any(|w| w == "open");
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
            // AURORA_DEMO_SEARCH=<tab>[:text]: "Search" on a tab (or on
            // Aurora's "all" chip), its search done ("page" in the text: a
            // full page, "xyz": nothing; for land in the Legacy look the
            // text is the price limit, 0 by default)
            if let Ok(v) = std::env::var("AURORA_DEMO_SEARCH") {
                let (name, text) = v.split_once(':').unwrap_or((v.as_str(), "aurora"));
                // land without a text keeps the default price field ("0")
                let typed = !name.trim().eq_ignore_ascii_case("land") || v.contains(':');
                let tab = crate::world::search::Tab::from_name(name);
                let all = name.trim().eq_ignore_ascii_case("all") && !self.settings.classic_search;
                if tab.is_some() || all {
                    match tab {
                        Some(tab) => self.search_ui.open_tab(tab),
                        None => self.search_ui.open_all(),
                    }
                    if typed {
                        self.search_ui.set_text(tab.unwrap_or(crate::world::search::Tab::People), text);
                    }
                    self.panels.search = true;
                    self.panels.perf = false;
                    let requests = if self.settings.classic_search {
                        tab.and_then(|t| self.search_ui.request(t, &self.settings.search))
                            .into_iter()
                            .collect()
                    } else {
                        self.search_ui.aurora_requests(&self.settings.search)
                    };
                    let access = self.world.agent_access();
                    for request in requests {
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
                self.region_ui.demo_people(scenario.people.clone());
                if let Some(d) = scenario.intervention.clone() {
                    self.region_ui.demo_intervention(d);
                }
                if let Some(report) = scenario.top {
                    self.region_ui.demo_top(report);
                }
                if let Some(d) = scenario.estate_wide.clone() {
                    self.region_ui.demo_estate_wide(d);
                }
                if scenario.classic {
                    let people = scenario.page == ui::region::Page::People;
                    self.region_ui
                        .demo_classic(ui::region::ClassicTab::of_section(scenario.manage.section, people));
                }
                if scenario.compact {
                    let pos = egui::pos2(560.0, 80.0);
                    ui::widgets::place_floater(&self.egui_ctx, ui::region::WINDOW, pos, egui::vec2(480.0, 720.0));
                } else if scenario.page == ui::region::Page::People || scenario.manage.advanced {
                    // tall enough for the four lists, or a section's Advanced fold
                    let pos = egui::pos2(340.0, 50.0);
                    ui::widgets::place_floater(&self.egui_ctx, ui::region::WINDOW, pos, egui::vec2(1000.0, 840.0));
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
            // AURORA_DEMO_CONTACTS=friends|groups|sets|detached|add|hidden: the
            // Contacts tab of Conversations (or its torn-off window, or the
            // resident picker of "Add...")
            if let Ok(v) = std::env::var("AURORA_DEMO_CONTACTS") {
                self.panels.chat = true;
                self.panels.perf = false;
                self.chat_ui.contacts = true;
                self.settings.show_contacts_tab = v != "hidden";
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
                // AURORA_DEMO_PREVIEW_PLAY=card|window: the first video plays
                self.chat_ui.previews.demo_play = match std::env::var("AURORA_DEMO_PREVIEW_PLAY").as_deref() {
                    Ok("card") => Some(crate::link_preview::Player::InCard),
                    Ok("window") => Some(crate::link_preview::Player::Window),
                    _ => None,
                };
                self.chat_ui.demo_style = Some(if view == crate::demo::Bubbles::Legacy {
                    crate::settings::ChatStyle::Legacy
                } else {
                    crate::settings::ChatStyle::Bubbles
                });
                // =aurora[:…]: the workspace, at the scene's width
                let mut size = egui::vec2(700.0, 560.0);
                if let Some(scene) = crate::demo::aurora_scenario() {
                    self.chat_ui.demo_style = Some(if scene.compact {
                        crate::settings::ChatStyle::Compact
                    } else {
                        crate::settings::ChatStyle::Aurora
                    });
                    self.chat_ui.selected = crate::demo::aurora_conversation(scene);
                    self.chat_ui.members_open = view == crate::demo::Bubbles::Group;
                    self.chat_ui.stage_mini = scene.call == Some(crate::demo::AuroraCall::Mini);
                    self.chat_ui.demo_new_menu = scene.new_menu;
                    self.chat_ui.demo_volume_menu = scene.volume_menu;
                    self.chat_ui.demo_channel = scene.channel;
                    // the window alone, to compare with the mockups
                    self.panels.people = false;
                    self.panels.inventory = false;
                    // the mockups' window: 580 px high, title bar included
                    size = egui::vec2(scene.width, 580.0);
                }
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(460.0, 230.0), size);
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
            // AURORA_DEMO_GIFS=open|klipy|sent|menu|nokey|net: the GIF picker of
            // the friend's conversation (#932), with AURORA_DEMO_CONV or alone
            if let Some(mode) = crate::demo::gifs_scenario() {
                if std::env::var_os("AURORA_DEMO_CONV").is_none() {
                    self.panels.chat = true;
                    self.panels.perf = false;
                    self.chat_ui.selected = Some(crate::demo::DEMO_FRIEND);
                    ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(460.0, 230.0), egui::vec2(700.0, 560.0));
                }
                self.chat_ui.gif_open = !matches!(mode, crate::demo::Gifs::NoKey | crate::demo::Gifs::Menu);
                self.chat_ui.gifs.demo_scene(mode);
            }
            // AURORA_DEMO_OBJECT_CHAT=<style>|lines[:on]: the nearby chat in
            // the Conversations window of that style, or the lines above the
            // chat bar with the window closed
            if let Some(scene) = crate::demo::object_chat::scene() {
                self.panels.people = false;
                self.panels.inventory = false;
                self.panels.perf = false;
                self.panels.chat = scene.style.is_some();
                if let Some(style) = scene.style {
                    self.chat_ui.selected = scene.im.then_some(crate::demo::DEMO_FRIEND);
                    self.chat_ui.demo_style = Some(style);
                    ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(460.0, 230.0), egui::vec2(760.0, 560.0));
                }
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
            // AURORA_DEMO_NOTIFS, AURORA_DEMO_PRESENCE: nothing else open
            // over the corners (and the nearby chat lines over the chat bar)
            if crate::demo::notifs::mode().is_some() || crate::demo::presence::position().is_some() {
                self.panels.perf = false;
                self.panels.chat = false;
                self.panels.inventory = false;
            }
            if crate::demo::presence::position().is_some() {
                self.panels.people = false;
            }
            // AURORA_DEMO_CONV_BUTTON: the Conversations window closed
            if crate::demo::conv_button::mode().is_some() {
                self.panels.perf = false;
                self.panels.chat = false;
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
                self.options_ui.open(ui::options::Page::Privacy, 0);
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
            // AURORA_DEMO_INVENTORY=offer-card…: the giver's conversation in
            // sight, the card alone
            if let Some(scene) = crate::demo::offer_card_scene() {
                self.panels.perf = false;
                self.panels.chat = true;
                self.chat_ui.open(crate::demo::DEMO_GIVER);
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 420.0), egui::vec2(600.0, 320.0));
                if scene.legacy {
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
            self.sync_script_debug();
            self.settings.auto_accept_inventory |= crate::demo::auto_accept_inventory_scene();
            self.settings.show_accepted_in_inventory |= crate::demo::show_accepted_scene();
            self.world.inventory_offers.auto_accept = self.settings.auto_accept_inventory;
            self.world.inventory_offers.show_accepted = self.settings.show_accepted_in_inventory;
            for ev in crate::demo::events() {
                if let Some(e) = self.world.apply(ev) {
                    self.on_app_event(e);
                }
            }
            if crate::demo::auto_accept_inventory_scene() {
                log::info!(
                    "demo inventory auto-accept: pending={}, cards={}, show={}",
                    self.world
                        .notifications
                        .list
                        .iter()
                        .filter(|n| n.kind == crate::world::notifications::Kind::Inventory)
                        .count(),
                    self.world.inventory_offers.list.len(),
                    self.settings.show_accepted_in_inventory,
                );
            }
            if let Ok(mode) = std::env::var("AURORA_DEMO_PARCEL_VISIBILITY") {
                crate::demo::parcel_visibility::install(&mut self.world, &mut self.settings, &mode);
            }
            // AURORA_DEMO_APPLINKS: a link of each `secondlife:///app/...`
            // command, in a conversation and in the bubbles
            if crate::demo::applinks_scene() {
                let item = self
                    .world
                    .inventory
                    .items
                    .values()
                    .filter(|i| !i.name.is_empty())
                    .min_by(|a, b| a.name.cmp(&b.name))
                    .map(|i| (i.id, i.name.clone()));
                let text = crate::demo::applinks_text(item);
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
                self.world.apply(NetEvent::Chat(aurora_net::ChatMessage {
                    from_name: aurora_i18n::tr!("demo-places-sign").into(),
                    source_id: uuid::Uuid::from_u128(0x5106),
                    owner_id: uuid::Uuid::nil(),
                    source_type: aurora_net::ChatSourceType::Object,
                    chat_type: aurora_net::ChatType::Normal,
                    position: glam::Vec3::ZERO,
                    message: text.clone(),
                }));
                self.panels.perf = false;
                self.panels.places = false;
                self.panels.people = false;
                self.panels.chat = true;
                // `preview`: Tess's conversation stays out of sight, so its
                // message gets a card and both column entries show it (#947)
                if !crate::demo::applinks_preview_scene() {
                    self.chat_ui.open(crate::demo::DEMO_NOVA);
                }
                ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 380.0), egui::vec2(640.0, 380.0));
                if crate::demo::applinks_actions_scene() {
                    self.panels.inventory = false;
                    ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(440.0, 90.0), egui::vec2(800.0, 440.0));
                }
                if let Some(action) = crate::demo::applinks_action(&text) {
                    self.prepare_uri_restore_demo(&action);
                    self.on_ctx_action(action);
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
            // AURORA_DEMO_CONV_SCROLL: the friend's earlier talk first
            if crate::demo::conv_scroll_scenario().is_some() {
                crate::demo::conv_scroll_lines(&mut self.world);
                self.chat_ui.demo_scroll = Some(crate::demo::DEMO_FRIEND);
            }
            if let Some(view) = crate::demo::bubbles_scenario() {
                crate::demo::bubbles_lines(&mut self.world, view);
            }
            // AURORA_DEMO_AREA_SEARCH: a native object list and filter fixtures.
            if let Ok(mode) = std::env::var("AURORA_DEMO_AREA_SEARCH") {
                self.demo_area_search(&mode);
            }
            if let Ok(mode) = std::env::var("AURORA_DEMO_AVATAR_RENDER_SETTINGS") {
                self.demo_avatar_render_settings(&mode);
            }
            if let Ok(mode) = std::env::var("AURORA_DEMO_SOUND_EXPLORER") {
                self.demo_sound_explorer(&mode);
            }
            if let Ok(mode) = std::env::var("AURORA_DEMO_ANIMATION_EXPLORER") {
                self.demo_animation_explorer(&mode);
            }
            // AURORA_DEMO_OBJECT_CHAT: two notices among the lines, each with
            // its topic's icon when compact
            if crate::demo::object_chat::scene().is_some() {
                crate::demo::object_chat::notices(&mut self.world);
            }
            if let Some(scene) = crate::demo::aurora_scenario() {
                crate::demo::aurora_lines(&mut self.world, scene);
                // the scene's unread lines as if they came while away
                self.chat_ui.demo_lines_unseen();
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
            if std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| {
                v.starts_with("faces")
                    || v.starts_with("shared")
                    || v.starts_with("mesh-inspector")
                    || v.starts_with("invisible-")
                    || v.starts_with("physics")
            }) {
                let invisible_manipulation = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("invisible-"));
                let invisible = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("faces-invisible,"));
                let inspector = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("mesh-inspector"));
                let physics = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("physics"));
                let complex = inspector
                    || physics
                    || invisible_manipulation
                    || std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("faces-complex,"));
                let mut events = if complex {
                    crate::demo::complex_face_selection_events()
                } else {
                    crate::demo::face_selection_events(invisible)
                };
                if invisible_manipulation {
                    for event in &mut events {
                        if let aurora_net::NetEvent::ObjectUpdates { objects, .. } = event {
                            for object in objects {
                                if let Some(te) = object.texture_entry.as_mut() {
                                    for face in &mut std::sync::Arc::make_mut(te).faces {
                                        face.color[3] = 0.0;
                                    }
                                }
                            }
                        }
                    }
                }
                if complex && let Some(gfx) = &mut self.gfx {
                    if physics {
                        let analysed = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("physics-analysed"));
                        crate::scene::physics_shapes::install_demo(&mut self.scene.meshes, analysed);
                    } else if inspector {
                        let empty = std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("mesh-inspector-cycle-empty"));
                        self.scene.install_demo_mesh_inspector(&mut gfx.renderer, empty);
                    } else {
                        self.scene.install_demo_selection_mesh(&mut gfx.renderer);
                    }
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
                if chatlog::find_flag("legacy") {
                    self.chat_ui.demo_style = Some(crate::settings::ChatStyle::Legacy);
                }
                if chatlog::find_flag("narrow") {
                    ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(8.0, 44.0), egui::vec2(480.0, 560.0));
                }
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
                let selected = if matches!(mode.as_str(), "folder-names" | "folder-names-wait") {
                    crate::ao::demo::seed_folder_names(&mut self.world.inventory, self.world.agent_id)
                } else if mode == "firestorm-wait" {
                    crate::ao::demo::seed_firestorm_wait(&mut self.world.inventory, self.world.agent_id)
                } else if mode.starts_with("migrate-alchemy") {
                    crate::ao::demo::seed_alchemy_migration(&mut self.world.inventory, self.world.agent_id)
                } else if mode.starts_with("migrate") {
                    crate::ao::demo::seed_migration(&mut self.world.inventory, self.world.agent_id)
                } else if mode == "firestorm" {
                    crate::ao::demo::seed_firestorm(&mut self.world.inventory, self.world.agent_id)
                } else if mode.starts_with("interface") {
                    crate::ao::demo::seed_interface(&mut self.world.inventory, self.world.agent_id, mode)
                } else if matches!(mode.as_str(), "swim-missing" | "quick-missing") {
                    crate::ao::demo::seed_swimming_incomplete(&mut self.world.inventory, self.world.agent_id)
                } else if matches!(mode.as_str(), "empty" | "quick-empty") {
                    uuid::Uuid::nil()
                } else if mode.starts_with("design-") || matches!(mode.as_str(), "movement" | "movement-rebound" | "swim" | "quick") {
                    crate::ao::demo::seed_movement_design(&mut self.world.inventory, self.world.agent_id)
                } else {
                    crate::ao::demo::seed(&mut self.world.inventory, self.world.agent_id)
                };
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
                if view.starts_with("folders") {
                    crate::world::outfit_folders::seed_demo(&mut self.world.inventory, self.world.agent_id);
                }
                if view.starts_with("layers") {
                    crate::world::appearance::seed_demo_layers(&mut self.world.inventory, self.world.agent_id);
                }
                if view.starts_with("wearables") {
                    crate::world::appearance::seed_demo_wearables(&mut self.world.inventory, self.world.agent_id);
                }
                if view == "broken" {
                    crate::world::appearance::seed_demo_broken(&mut self.world.inventory, self.world.agent_id);
                }
                if matches!(view.as_str(), "gallery-fit" | "gallery-fill") {
                    ui::appearance::demo_previews(&self.egui_ctx, &mut self.world.inventory, &mut self.ui_images);
                    self.settings.appearance_fill_previews = view == "gallery-fill";
                }
                let many = crate::world::appearance::demo_outfit_count();
                if many > 0 {
                    crate::world::appearance::seed_demo_many(&mut self.world.inventory, self.world.agent_id, many);
                }
                for cmd in
                    crate::world::appearance::sync_commands(&self.world.inventory, self.world.agent_id, &self.world.worn_attachment_items())
                {
                    self.send(cmd);
                }
                self.panels.appearance = !crate::world::appearance::demo_outfit_mode("closed");
                self.panels.perf = false;
                self.appearance_ui.open(
                    match view.as_str() {
                        "outfits" | "multi-outfits" | "folders-outfits" | "wearables-outfits" | "broken" => 1,
                        "worn" | "save" | "wearables" => 2,
                        _ => 0,
                    },
                    matches!(view.as_str(), "edit" | "wearables-edit") || view.starts_with("layers"),
                );
                if view == "save" {
                    self.appearance_ui.begin_save_as(&self.world.inventory);
                }
                if matches!(view.as_str(), "multi-gallery" | "multi-outfits") {
                    self.appearance_ui.demo_multiselection();
                }
                if view == "duplicates" {
                    crate::world::appearance::seed_duplicate_response(&mut self.world.inventory, self.world.agent_id);
                }
                if matches!(view.as_str(), "menu" | "duplicates" | "broken") {
                    self.appearance_ui.open_outfit(uuid::Uuid::from_u128(704));
                }
                if view == "wearables-outfits" {
                    self.appearance_ui.open_outfit(uuid::Uuid::from_u128(705));
                }
                if crate::world::appearance::demo_outfit_mode("locate") {
                    self.appearance_ui.locate_current(&self.world.inventory);
                }
            }
            self.demo_shape_editor();
            self.demo_wearable_editor();
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
                if matches!(view.as_str(), "landmark" | "aurora-landmark") {
                    let mut landmark = self.world.inventory.items[&uuid::Uuid::from_u128(8103)].clone();
                    landmark.id = uuid::Uuid::from_u128(8120);
                    landmark.name = aurora_i18n::tr!("common-landmark").into();
                    landmark.asset_type = 3;
                    landmark.inv_type = 3;
                    landmark.asset_id = crate::demo::place::landmark_ids(1).1;
                    self.world.inventory.add_items(vec![landmark]);
                }
                if matches!(view.as_str(), "outfit-menu" | "outfit-menu-loading") {
                    crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
                    if view == "outfit-menu-loading" {
                        self.world.inventory.items.remove(&uuid::Uuid::from_u128(723));
                    }
                }
                let configured_view = matches!(
                    view.as_str(),
                    "default-permissions"
                        | "aurora-default-permissions"
                        | "permissions"
                        | "lost"
                        | "bulk-permissions"
                        | "bulk-multi"
                        | "bulk-folder"
                        | "sort"
                        | "filters"
                        | "preferences"
                        | "recent"
                        | "worn"
                        | "filtered"
                        | "large"
                        | "search-selection"
                        | "search-selection-keyboard"
                        | "long"
                        | "long-filtered"
                        | "resize-left"
                        | "resize-right"
                        | "received"
                        | "received-big"
                        | "received-big-open"
                        | "received-filtered"
                        | "received-missing"
                        | "received-sort"
                        | "received-sort-name"
                        | "aurora-received-sort"
                        | "aurora-received-sort-name"
                        | "received-filing"
                        | "aurora-received-filing"
                        | "aurora-received-unread"
                        | "aurora-received-unread-compact"
                        | "aurora"
                        | "aurora-medium"
                        | "aurora-landmark"
                        | "aurora-wide"
                        | "aurora-columns"
                        | "aurora-columns-hidden"
                        | "aurora-worn"
                        | "aurora-worn-medium"
                        | "aurora-worn-wide"
                        | "aurora-sidebar-medium"
                        | "aurora-sidebar-wide"
                        | "aurora-rights"
                        | "aurora-no-details"
                        | "aurora-medium-no-details"
                        | "aurora-wide-no-details"
                        | "favorites"
                        | "favorites-multi"
                        | "aurora-favorites"
                        | "aurora-favorites-multi"
                        | "aurora-folder-window"
                        | "aurora-folder-window-wide"
                        | "additional"
                        | "aurora-additional"
                        | "show-original"
                        | "aurora-show-original"
                        | "smart"
                        | "aurora-smart"
                        | "aurora-smart-wide"
                );
                if configured_view {
                    crate::world::inventory::demo::seed_view(
                        &mut self.world.inventory,
                        self.world.agent_id,
                        view == "large" || view.starts_with("search-selection"),
                    );
                    self.settings.inventory = ui::inventory::InventoryPreferences::default();
                    self.inventory_ui.show_original(&self.world.inventory, self.world.inventory.root);
                    if view == "lost" {
                        crate::world::inventory::demo::seed_lost(&mut self.world.inventory, self.world.agent_id);
                        self.panels.inventory = true;
                        self.inventory_ui
                            .show_original(&self.world.inventory, crate::world::inventory::demo::LOST_FOLDER);
                        self.inventory_ui.expand_all = Some(true);
                    }
                    self.inventory_ui.filters_open = view == "filters";
                    self.inventory_ui.preferences_open = view == "preferences";
                    self.inventory_ui.default_permissions_open = view.ends_with("default-permissions");
                    if view.starts_with("bulk-") {
                        let ids = if view == "bulk-folder" {
                            vec![uuid::Uuid::from_u128(8000)]
                        } else {
                            [8100, 8102, 8103].into_iter().map(uuid::Uuid::from_u128).collect()
                        };
                        self.inventory_ui.show_original(&self.world.inventory, ids[0]);
                        self.inventory_ui.selection.extend(ids.iter().copied());
                        if view == "bulk-permissions" {
                            self.inventory_ui.open_bulk_permissions(ids);
                        }
                    }
                    if view == "permissions" {
                        self.inventory_ui.show_next_owner = true;
                        self.inventory_ui.expand_all = Some(true);
                    }
                    if view == "sort" {
                        self.inventory_ui.show_original(&self.world.inventory, uuid::Uuid::from_u128(8000));
                        self.inventory_ui.open_folder_window(uuid::Uuid::from_u128(8000));
                    }
                    if view.ends_with("additional") {
                        // the main window on a folder, the additional one
                        // opened by its button beside it
                        self.inventory_ui.show_original(&self.world.inventory, uuid::Uuid::from_u128(8100));
                        self.inventory_ui.toggle_additional_window(&self.egui_ctx);
                    }
                    if view.starts_with("aurora") {
                        if view.starts_with("aurora-sidebar-") {
                            crate::world::inventory::demo::seed_sidebar(&mut self.world.inventory);
                        }
                        // Aurora's window at each width, on the demo object
                        // of the right-click tests, a favourite beside it.
                        self.inventory_ui.show_original(&self.world.inventory, uuid::Uuid::from_u128(8100));
                        if view.starts_with("aurora-worn") {
                            self.inventory_ui.show_original(&self.world.inventory, uuid::Uuid::from_u128(715));
                            self.inventory_ui.selected = None;
                            self.inventory_ui.selection.clear();
                        }
                        if let Some(shirt) = self.world.inventory.items.get_mut(&uuid::Uuid::from_u128(8111)) {
                            shirt.favorite = true;
                        }
                        if view == "aurora-rights" {
                            self.settings.inventory.rights = ui::inventory::permissions::Rights::Both;
                        }
                        if view.starts_with("aurora-columns") {
                            let columns = &mut self.settings.inventory.explorer;
                            columns.sidebar_width = 280;
                            columns.kind_width = 176;
                            columns.rights_width = 80;
                            columns.date_width = 110;
                            columns.show_kind = view != "aurora-columns-hidden";
                            columns.show_date = view != "aurora-columns-hidden";
                        }
                        if view.ends_with("-no-details") {
                            self.settings.inventory.show_details = false;
                        }
                        if view.starts_with("aurora-folder-window") {
                            // the right-click tests in a window of their own
                            self.inventory_ui.open_folder_window(uuid::Uuid::from_u128(8000));
                            if let Some(window) = self.inventory_ui.windows.last_mut() {
                                window.id = ui::inventory::AURORA_DEMO_FOLDER_WINDOW;
                            }
                        }
                    }
                    if view.contains("favorites") {
                        let ids: Vec<_> = [8100, 8109, 8110].into_iter().map(uuid::Uuid::from_u128).collect();
                        if view.ends_with("multi") {
                            self.inventory_ui.show_original(&self.world.inventory, ids[0]);
                            self.inventory_ui.selection.extend(ids.iter().copied());
                            self.inventory_ui.demo_menu = Some(ids[0]);
                        } else {
                            // Run the same batch planner used by the menu.
                            if let Ok(change) = crate::world::inventory::actions::set_favorites(&self.world.inventory, &ids, true) {
                                let _ =
                                    crate::world::inventory::actions::demo_mutate(&mut self.world.inventory, self.world.agent_id, change);
                            }
                            self.inventory_ui.tab = 4;
                            self.settings.inventory.show_favorites = true;
                            self.inventory_ui.expand_all = Some(true);
                        }
                    }
                    if view == "recent" {
                        self.inventory_ui.tab = 2;
                    }
                    if view.ends_with("received-sort") || view.ends_with("received-sort-name") {
                        crate::world::inventory::demo::seed_received_sort(&mut self.world.inventory, self.world.agent_id);
                        self.inventory_ui.tab = 1;
                        self.inventory_ui.preferences_open = true;
                        self.inventory_ui.selected = None;
                        self.inventory_ui.selection.clear();
                        self.settings.inventory.received_sort.folders_by_name = view.ends_with("-name");
                    }
                    if view.ends_with("received-filing") || view.starts_with("aurora-received-unread") {
                        self.settings.sort_received_items = true;
                        self.settings.received_items_since = 0;
                        crate::world::received_items::demo(&mut self.world);
                        self.inventory_ui.tab = usize::from(view.ends_with("received-filing"));
                    }
                    if view == "worn" {
                        self.inventory_ui.tab = 3;
                    }
                    if matches!(view.as_str(), "received-big" | "received-big-open") {
                        self.inventory_ui.tab = 1;
                        crate::world::inventory::demo::seed_received_big(&mut self.world.inventory, self.world.agent_id, 150, 20);
                        if view == "received-big-open" {
                            self.inventory_ui.expand_all = Some(true);
                        }
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
                    if matches!(view.as_str(), "smart" | "aurora-smart" | "aurora-smart-wide") {
                        // advanced search: objects or scripts (an Any
                        // group) but not the no-copy object (a Not rule),
                        // with saved searches in the sidebar
                        use ui::inventory::smart::{Field, Group, Node, Op, Rule, SmartQuery};
                        let kinds = Rule {
                            bits: (1 << 6) | (1 << 10),
                            ..Rule::of(Field::Type)
                        };
                        let no_copy = aurora_i18n::tr!("world-inventory-demo-no-copy-object").to_lowercase();
                        let query = SmartQuery {
                            root: Group {
                                any: false,
                                not: false,
                                nodes: vec![
                                    Node::Group(Group {
                                        any: true,
                                        not: false,
                                        nodes: vec![Node::Rule(kinds), Node::Rule(Rule::text(Field::Name, Op::EndsInNumber, ""))],
                                    }),
                                    Node::Rule(Rule {
                                        not: true,
                                        ..Rule::text(Field::Name, Op::Contains, &no_copy)
                                    }),
                                ],
                            },
                        };
                        let worn = SmartQuery {
                            root: Group {
                                nodes: vec![Node::Rule(Rule::of(Field::Worn))],
                                ..Group::default()
                            },
                        };
                        let id = ui::inventory::smart_store::add(aurora_i18n::tr!("inventory-smart-untitled"), query.clone());
                        ui::inventory::smart_store::add(aurora_i18n::tr!("inventory-smart-worn"), worn);
                        self.inventory_ui.open_smart(Some(id), query);
                        // the classic layout edits the rules in the Filters floater
                        self.inventory_ui.filters_open = view == "smart";
                    }
                    if view.starts_with("search-selection") {
                        self.inventory_ui.search = aurora_i18n::tr!("world-inventory-demo-item", n = "1490");
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
                if let Some(big) = crate::world::inventory::demo::big() {
                    let open = crate::world::inventory::demo::seed_big(&mut self.world.inventory, self.world.agent_id, big.items);
                    self.settings.inventory = ui::inventory::InventoryPreferences::default();
                    self.inventory_ui.open_folders(open);
                    self.inventory_ui.tab = if big.recent { 2 } else { 0 };
                    self.inventory_ui.search.clone_from(&big.search);
                    if big.replace {
                        self.inventory_ui.begin_replace_links(crate::world::inventory::demo::BIG_LINKED);
                    }
                }
                // the "…-show" offers: the window opens on the kept item
                self.panels.inventory = !crate::demo::show_accepted_scene();
                self.settings.show_accepted_in_inventory |= crate::demo::show_accepted_scene();
                self.panels.perf = false;
                self.panels.appearance = false;
                self.panels.minimap = false;
                // "offer" modes keep the sample inventory offer to answer
                if !view.starts_with("offer") {
                    self.world.notifications = Default::default();
                }
                if view == "aurora-menu-textures-export"
                    && let Some(mut snapshot) = self.world.inventory.items.get(&uuid::Uuid::from_u128(8104)).cloned()
                {
                    snapshot.id = uuid::Uuid::from_u128(8180);
                    snapshot.asset_type = 15;
                    snapshot.inv_type = 15;
                    snapshot.name = format!("{} 2", snapshot.name);
                    self.world.inventory.add_items(vec![snapshot]);
                }
                if let Some(id) = crate::world::inventory::demo::target(&view, &self.world.inventory) {
                    self.inventory_ui.show_original(&self.world.inventory, id);
                    if let Some(item) = self.world.inventory.items.get_mut(&id) {
                        if view == "uuid-restricted" {
                            item.owner_mask &= !crate::world::inventory::actions::TRANSFER;
                        } else if view == "uuid-missing" {
                            item.asset_id = uuid::Uuid::nil();
                        }
                    }
                    if view == "uuid-multi" {
                        self.inventory_ui.selection.insert(uuid::Uuid::from_u128(8105));
                    }
                    if view == "aurora-menu-textures-export" {
                        self.inventory_ui.selection.insert(uuid::Uuid::from_u128(8180));
                    }
                    if view.starts_with("aurora-menu-") {
                        self.inventory_ui
                            .configure_menu_demo(&self.world.inventory, &mut self.settings.inventory, id, &view);
                    }
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
                    } else if matches!(
                        view.as_str(),
                        "properties" | "animation-properties" | "properties-no-mod" | "properties-no-creator"
                    ) {
                        // a no-modify object made by someone else, with its creator
                        // or, as the simulator sends a multi-creator object, without
                        if let Some(it) = self.world.inventory.items.get_mut(&id)
                            && view.starts_with("properties-no-")
                        {
                            it.owner_mask &= !crate::world::inventory::actions::MODIFY;
                            it.base_mask &= !crate::world::inventory::actions::MODIFY;
                            it.creator = if view == "properties-no-creator" {
                                uuid::Uuid::nil()
                            } else {
                                uuid::Uuid::from_u128(100)
                            };
                        }
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
                        if view.starts_with("image-layout") {
                            self.demo_inventory_file_choices(view.ends_with("wide"));
                        }
                    } else if !matches!(
                        view.as_str(),
                        "keyboard" | "keyboard-fold" | "aurora-keyboard" | "aurora-keyboard-fold" | "aurora-keyboard-wide"
                    ) {
                        self.inventory_ui.demo_menu = Some(id);
                    }
                } else if !configured_view && crate::world::inventory::demo::big().is_none() {
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
            if std::env::var_os("AURORA_DEMO_ANIMESH").is_some() || std::env::var("AURORA_DEMO_POSER").is_ok_and(|m| m == "animesh") {
                if let Some(g) = &mut self.gfx {
                    self.scene.install_demo_animesh(&mut g.renderer);
                }
                for ev in crate::demo::animesh_events() {
                    self.world.apply(ev);
                }
                if crate::demo::animation_edit_demo()
                    && let Some(wearer) = self
                        .world
                        .objects
                        .index_of_uuid(&self.world.agent_id)
                        .and_then(|i| self.world.objects.get(i))
                        .map(|o| o.key.local_id)
                {
                    for ev in crate::demo::own_rigged_events(wearer) {
                        self.world.apply(ev);
                    }
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
                for ev in crate::demo::cloud_self_events(&self.world.avatar_lib.shape_params) {
                    self.world.apply(ev);
                }
            }
            self.stream_demo = crate::demo::stream::StreamDemo::from_env();
            self.rez_demo = crate::demo::rez::RezDemo::from_env();
            self.arrival_demo = crate::demo::arrival::ArrivalDemo::from_env();
            self.outfit_restore_demo = crate::demo::outfit_restore::OutfitRestoreDemo::from_env();
            if let Some(demo) = &mut self.outfit_restore_demo {
                demo.setup(&mut self.world);
                self.panels.perf = false;
            }
            self.tex_refresh_demo = crate::demo::tex_refresh::TexRefreshDemo::from_env();
            self.sculpt_missing_demo = crate::demo::sculpt::MissingMapDemo::from_env();
            self.texture_missing_demo = crate::demo::missing_texture::MissingTextureDemo::from_env();
            self.experience_demo = crate::demo::experience::ExperienceDemo::from_env();
            self.geometry_demo = crate::demo::geometry::GeometryDemo::from_env();
            self.turn_demo = crate::demo::turn::TurnDemo::from_env();
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
            // AURORA_DEMO_PHYSICS: a physics layer and balls on the body of
            // our avatar and of the one in front of it
            if let Some(mode) = crate::demo::physics::mode() {
                if let Some(g) = &mut self.gfx {
                    self.scene.install_demo_physics_body(&mut g.renderer);
                }
                for ev in crate::demo::physics::events(&self.scene.avatar_lib.shape_params, mode) {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_PHYSICS_EDIT: after the layer above, which a new
            // layer replaces
            self.demo_physics_editor();
            // AURORA_DEMO_FEET=opaque|blend: a shoe on each foot of our avatar
            // and of the crowd (what the contact shadows see touch the ground)
            if let Some(blend) = crate::demo::feet_demo() {
                for ev in crate::demo::feet_events(blend, crate::demo::crowd_count().unwrap_or(0)) {
                    self.world.apply(ev);
                }
            }
            // AURORA_DEMO_GLYPHS=1|2|rtl: residents named and talking in the
            // characters of the grid's display names (system fallback fonts)
            if let Some(mode) = crate::demo::glyphs::scenario() {
                for ev in crate::demo::glyphs::events(mode) {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
                // closed with AURORA_DEMO_CHAT_LINES, whose lines show above
                // the chat bar with the Conversations window closed
                self.panels.chat = crate::demo::chat_lines_scenario().is_none();
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
                    *self.chat_ui.draft_mut(self.chat_ui.selected) = text;
                }
            }
            // AURORA_DEMO_EMOJI_CODES=<text>: the conversation window open, its
            // input holding <text> with the keyboard and the caret at the
            // end: a `:code` of two letters or more there opens the emoji
            // suggestions (`ui/emoji/codes.rs`)
            if let Ok(text) = std::env::var("AURORA_DEMO_EMOJI_CODES") {
                self.panels.chat = true;
                *self.chat_ui.draft_mut(self.chat_ui.selected) = text;
                self.chat_ui.conv_focus_request = true;
            }
            // Paragraph drafts in both editors, without a grid or clipboard.
            if let Ok(mode) = std::env::var("AURORA_DEMO_CHAT_INPUT") {
                self.panels.chat = true;
                let text = if mode == "long" {
                    "A long paragraph with café and emoji 🙂 that wraps within the chat field.\n".repeat(12)
                } else {
                    "First paragraph with café 🙂\n\nSecond paragraph\nCtrl+Shift+Enter adds a new line".into()
                };
                self.chat_ui.input.clone_from(&text);
                *self.chat_ui.draft_mut(self.chat_ui.selected) = text;
                self.chat_ui.conv_focus_request = true;
            }
            // AURORA_DEMO_SPELLING=1|menu|replace: misspelled words in the
            // chat bar and in the conversation input (`spelling.rs`); `menu`:
            // the text menu opened on the bar's last word with its
            // suggestions; `replace`: auto-replace on for the run (not
            // saved), "brb " then "ty " typed in the bar (`ui/text_menu.rs`)
            if let Some(mode) = crate::ui::text_menu::spelling_demo() {
                self.panels.chat = true;
                if mode == crate::ui::text_menu::SpellingDemo::Replace {
                    crate::autoreplace::set_enabled(true);
                } else {
                    self.chat_ui.input = "Helo, I will teleport to the sandbox with my frends".into();
                    *self.chat_ui.draft_mut(self.chat_ui.selected) = "Teh avatar is waiting at the landng point".into();
                }
            }
            // AURORA_DEMO_CONV_SELECT=all|part: the conversation window open,
            // its lines selected (Ctrl+A, or across three lines)
            if let Ok(mode) = std::env::var("AURORA_DEMO_CONV_SELECT") {
                self.panels.chat = true;
                self.chat_ui.demo_select = Some(mode.trim().to_owned());
            }
            // AURORA_DEMO_CONV_MENU=local|im|group|conference: the right-click
            // menu of a conversation of the list open (with AURORA_DEMO_CONV)
            if let Ok(kind) = std::env::var("AURORA_DEMO_CONV_MENU") {
                self.panels.chat = true;
                self.chat_ui.demo_menu = Some(kind.trim().to_owned());
            }
            // AURORA_DEMO_CONV_REORDER=1|hold|out|nearby: an entry of the
            // list dragged (with AURORA_DEMO_CONV)
            if let Ok(mode) = std::env::var("AURORA_DEMO_CONV_REORDER") {
                self.panels.chat = true;
                self.chat_ui.demo_reorder = Some(ui::chat::ReorderDemo::parse(&mode));
            }
            // AURORA_DEMO_KEYWORDS: the alerts on for the run, with the
            // Conversations window open on the nearby chat
            if crate::demo::keywords::active() {
                crate::keywords::set(&self.keyword_settings());
                self.panels.chat = true;
            }
            // AURORA_DEMO_SOUND=1: a looped chime in the world; =hud:
            // on a linked HUD prim. Interface sounds are enabled too.
            if std::env::var_os("AURORA_DEMO_SOUND").is_some() {
                let (id, clip) = crate::demo::chime();
                self.scene.sounds.insert(id, clip);
                for id in self.settings.audio.ui.preload() {
                    self.scene.sounds.insert(id, crate::demo::ui_sound(id));
                }
                let mode = std::env::var("AURORA_DEMO_SOUND").unwrap_or_default();
                let events = match mode.as_str() {
                    "hud" => crate::demo::hud::sound_events(id),
                    "parcel-open" | "parcel-local" | "parcel-source" => {
                        self.world.agent.get_off(Vec3::new(132.0, 132.0, self.world.agent.position.z));
                        crate::demo::sound_parcels::events(id, &mode)
                    }
                    _ => crate::demo::sound_events(id),
                };
                for ev in events {
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
                // `inventory`, `library`, `sound`: the rows to double-click in sight
                let mode = std::env::var("AURORA_DEMO_WORN").unwrap_or_default();
                if mode == "library" {
                    crate::demo::worn::seed_library(&mut self.world);
                }
                if mode == "sound" {
                    let (id, clip) = crate::demo::chime();
                    self.scene.sounds.insert(id, clip);
                    crate::demo::worn::seed_sound(&mut self.world, id);
                }
                if let Some(&(_, first)) = crate::demo::worn::double_clicks(&mode).first() {
                    self.panels.inventory = true;
                    self.inventory_ui.show_original(&self.world.inventory, first);
                }
            }
            // HUD fixtures exercise screen-space rendering and the same
            // linked-object touch / inventory paths as real attachments.
            if let Ok(mode) = std::env::var("AURORA_DEMO_HUDS") {
                let faces = matches!(mode.as_str(), "faces" | "faces-turned" | "faces-alpha");
                if mode == "blend-opacity" {
                    self.scene
                        .legacy_mats
                        .insert(crate::demo::hud::MAT_OPACITY_BLEND, Default::default());
                    for (id, material) in crate::demo::hud::opacity_materials() {
                        self.scene.materials.insert(id, material);
                    }
                }
                if faces {
                    for (id, mut material) in crate::demo::hud::face_materials() {
                        if mode == "faces-alpha" {
                            material.base_color_texture = Some(crate::demo::TEX_HOLES);
                        }
                        self.scene.materials.insert(id, material);
                    }
                }
                let events = if mode == "blend-opacity" {
                    crate::demo::hud::opacity_events()
                } else if faces {
                    crate::demo::hud::face_events(mode == "faces-turned")
                } else {
                    crate::demo::hud::events()
                };
                for ev in events {
                    self.world.apply(ev);
                }
                if mode == "faces-alpha" {
                    for local_id in 8270..8279 {
                        let key = crate::world::objects::ObjKey {
                            region: self.world.main_region.unwrap_or_default(),
                            local_id,
                        };
                        if let Some(idx) = self.world.objects.index_of(&key)
                            && let Some(object) = self.world.objects.get_mut(idx)
                            && let Some(te) = &mut object.te
                        {
                            Arc::make_mut(te).faces[4].texture = crate::demo::TEX_HOLES;
                        }
                    }
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
                if mode == "blend-opacity" {
                    self.world.hud_scales = crate::world::hud_scale::HudScales::default();
                    self.world.hud_zoom = 1.0;
                }
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
                    self.demo_hud_media(crate::demo::demo_media_page());
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
            self.demo_media_filter();
        }
    }

    /// The window, its renderer and egui-winit's state go while the event
    /// loop still lives: `run_app` drops the loop, and with it the Wayland
    /// connection, before `main` drops the app. On Wayland the clipboard of
    /// egui-winit (smithay-clipboard) keeps a worker thread on that
    /// connection; dropped after it, the worker destroyed its objects on a
    /// closed connection and every exit ended in a segmentation fault (before
    /// the browsers' private profiles were cleaned up).
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.gfx = None;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // the digits of a Windows Alt code typed in a text field are kept
        // from it (alt_codes.rs); the character comes at Alt's release
        if self.alt_code_key(&event) {
            return;
        }
        // a movement key the chat bar hands to the avatar never reaches its
        // text (LLViewerWindow::handleKey returns before the focused field)
        let chat_pass = matches!(
            &event,
            WindowEvent::KeyboardInput { event: ke, .. }
                if ke.state == ElementState::Pressed
                    && matches!(ke.physical_key, PhysicalKey::Code(code) if self.chat_bar_passes_key(code))
        );
        // Keep a lens shortcut out of egui: Ctrl+0 otherwise also resets
        // the interface scale at the end of the frame.
        let key_mods = match &event {
            WindowEvent::KeyboardInput { event: ke, .. } => self.key_mods(ke),
            _ => self.mods(),
        };
        let lens_shortcut = matches!(
            &event,
            WindowEvent::KeyboardInput { event: ke, .. }
                if ke.state == ElementState::Pressed
                    && matches!(ke.physical_key, PhysicalKey::Code(code)
                        if matches!(self.settings.keybinds.triggered(&Input::key(code), key_mods),
                            Some(Action::LensZoomIn | Action::LensZoomOut | Action::LensZoomDefault))
                            && (!(self.egui_ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(&self.egui_ctx))
                                || shortcut_while_typing(true, key_mods.ctrl, key_mods.alt, code, true)))
        ) && self.in_world()
            && self.options_ui.capture.is_none()
            && !self.exit_prompt.open
            && self.profile_ui.focused_web().is_none()
            && self.search_ui.focused_web().is_none()
            && self.media.focus.is_none();
        // the hidden cursor the camera holds is the tool's (cursor_hold.rs):
        // it never hovers, clicks or scrolls the windows (#1343)
        let held_pointer = self.cursor_hold.held() && cursor_hold::held_from_ui(&event);
        // AltGr's shortcut key must not also reach egui as a plain key:
        // for example G would snap a selection while Ctrl+Alt+G runs.
        // Releases still clear any key pressed before AltGr was held.
        let altgr_shortcut = matches!(&event, WindowEvent::KeyboardInput { event: ke, .. }
            if self.altgr_native_key(ke));
        let Some(g) = self.gfx.as_mut() else {
            return;
        };
        let resp = if chat_pass || lens_shortcut || held_pointer || altgr_shortcut {
            egui_winit::EventResponse {
                consumed: false,
                repaint: true,
            }
        } else {
            g.egui_state.on_window_event(&g.window, &event)
        };
        match &event {
            WindowEvent::CloseRequested => {
                self.request_exit();
            }
            WindowEvent::Resized(s) => {
                ui::menu::window_geometry_changed(&self.egui_ctx);
                g.renderer.resize(s.width, s.height);
                self.geom_changed = Some(Instant::now());
                self.sync_minimized_mute();
            }
            WindowEvent::Moved(_) => {
                ui::menu::window_geometry_changed(&self.egui_ctx);
                self.geom_changed = Some(Instant::now());
            }
            WindowEvent::RedrawRequested => {
                self.frame(event_loop);
            }
            WindowEvent::ModifiersChanged(m) => {
                self.shift = m.state().shift_key();
                self.ctrl = m.state().control_key();
                self.alt = m.state().alt_key();
                self.command = cfg!(target_os = "macos") && m.state().super_key();
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
                self.alt_codes.reset();
                self.chat_ui.cancel_gesture_key();
                self.gesture_state.manager.release(None);
                self.numeric_drag.cancel();
                self.build.reset_selection_wheel();
                self.focused = false;
                self.down.clear();
                // (the release of a button held when the focus goes never comes)
                self.alt_cursor.left_button(false);
                self.alt = false;
                self.alt_graph = false;
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
                self.track_alt_graph(ke);
                // Clear held controls before a dialog, capture or web/media
                // page can return early after receiving the native release.
                if ke.state == ElementState::Released
                    && let PhysicalKey::Code(code) = ke.physical_key
                {
                    self.on_input(Input::key(code), false);
                }
                if self.exit_prompt.open {
                    return;
                }
                if let PhysicalKey::Code(code) = ke.physical_key {
                    let pressed = ke.state == ElementState::Pressed;
                    let mods = self.key_mods(ke);
                    let altgr_text = self.altgr_text_input(ke);
                    // The color dialog and an emoji picker own Escape before
                    // camera shortcuts. egui already received the event above.
                    let eyedropper = ui::widgets::color::pipette::armed(&self.egui_ctx);
                    if pressed
                        && code == KeyCode::Escape
                        && (self.build.ui.color_picker.is_open()
                            || self.script_editors.color_open()
                            || eyedropper
                            || self.chat_ui.previews.enlarged()
                            || self.chat_ui.previews.image_window_open()
                            || ui::emoji::picker_open(&self.egui_ctx))
                    {
                        return;
                    }
                    // learn the keyboard layout for the shortcut labels
                    if pressed
                        && !self.shift
                        && !self.ctrl
                        && !self.alt
                        && !self.alt_graph
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
                    // egui received Ctrl+W above; reserve its physical key
                    // before gestures or movement can keep it held.
                    if pressed && ui::close_key::take_native_key(&ke.logical_key, code, mods, &mut self.down) {
                        self.check_temp_run();
                        return;
                    }
                    // the photo viewfinder's keys (the shutter, its menu…)
                    if !self.egui_ctx.egui_wants_keyboard_input() && self.viewfinder_key(code, &ke.logical_key, pressed, ke.repeat) {
                        self.down.remove(&Input::key(code));
                        return;
                    }
                    // The command table is shared with egui's dispatcher:
                    // an editing accelerator must never become a held move.
                    let input = Input::key(code);
                    let build_keyboard = self.in_world()
                        && !altgr_text
                        && !self.egui_ctx.egui_wants_keyboard_input()
                        && !egui::Popup::is_any_open(&self.egui_ctx);
                    let build_shortcut = pressed
                        .then(|| {
                            crate::build::ui::Shortcut::take_native_key(
                                &ke.logical_key,
                                code,
                                mods,
                                self.build.open,
                                build_keyboard,
                                &mut self.down,
                            )
                        })
                        .flatten();
                    let ui_shortcut = build_shortcut.is_some();
                    let command_key = pressed && !altgr_text && self.settings.keybinds.triggered(&input, mods).is_some();
                    if ui_shortcut || command_key {
                        self.down.remove(&input);
                        self.check_temp_run();
                    }
                    // A gesture key must still be held when the chat or a web
                    // page has keyboard focus; it only owns a visible HUD drag.
                    if pressed && !ke.repeat && !altgr_text && !ui_shortcut && self.settings.keybinds.uses_input(Action::HudDrag, &input) {
                        let _ = self.settings.keybinds.update_held_input(&mut self.down, &input, pressed, mods);
                    }
                    // Releases always go through so keys never get stuck.
                    let typing = self.egui_ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(&self.egui_ctx);
                    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
                    let gesture_key = crate::gestures::keys::from_key(code, &ke.key_without_modifiers());
                    if !pressed && let Some(key) = gesture_key {
                        self.gesture_state.manager.release(Some(key));
                    }
                    // Undo / Redo keep their keys while an editor with a
                    // history shows: the build tools, or the poser
                    let history_open = self.build.open || (self.ik_editor.open && self.ik_editor.posing());
                    if pressed
                        && !lens_shortcut
                        && !build_shortcut.is_some_and(|s| s.takes_gesture_key(history_open))
                        && (!ke.repeat || gesture_key.is_some_and(crate::gestures::Manager::repeat_triggers))
                        && !typing
                        && self.in_world()
                        && !mods.alt
                        && self.profile_ui.focused_web().is_none()
                        && self.search_ui.focused_web().is_none()
                        && self.media.focus.is_none()
                        && let Some(key) = gesture_key
                        && let Some(id) = self
                            .gesture_state
                            .manager
                            .key_trigger(key, u32::from(mods.ctrl) | if mods.shift { 4 } else { 0 })
                    {
                        self.gesture_play(id, Some(key));
                        return;
                    }
                    // a profile's web page that was clicked gets the keyboard
                    let web = match self.profile_ui.focused_web() {
                        Some(p) => Some(p),
                        None => self.search_ui.focused_web(),
                    };
                    let web_focused = web.is_some();
                    if !typing && let Some(p) = web {
                        use winit::platform::scancode::PhysicalKeyExtScancode;
                        let scancode = ke.physical_key.to_scancode().unwrap_or(0);
                        let mods = aurora_media::Modifiers {
                            control: self.ctrl,
                            alt: self.alt,
                            shift: self.shift,
                            command: self.command,
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
                            command: self.command,
                        };
                        if self
                            .media
                            .on_key(&self.world, code, scancode, pressed, ke.repeat, ke.text.as_deref(), mods)
                            && pressed
                        {
                            return;
                        }
                    }
                    // `start_gesture` (indra/newview/llviewerinput.cpp) takes
                    // the default chat bar on release, after text/web owners.
                    // Reserve the press too: letter-to-chat must not insert
                    // another slash or focus the Conversations field first.
                    if self.in_world()
                        && !web_focused
                        && self.media.focus.is_none()
                        && self.chat_ui.gesture_key(&self.egui_ctx, &ke.logical_key, code, pressed, mods)
                    {
                        return;
                    }
                    // "Letter keys move me" off: a printable key (Shift allowed),
                    // with the keyboard nowhere, starts the nearby chat with its
                    // character instead of moving or firing a shortcut; the
                    // gestures went first (LLViewerWindow::handleKey,
                    // LetterKeysAffectsMovementNotFocusChatBar)
                    if pressed
                        && !ke.repeat
                        && !typing
                        && !self.settings.letter_keys_move
                        && !ui_shortcut
                        && self.in_world()
                        && !mods.ctrl
                        && !mods.alt
                        && self.media.focus.is_none()
                        && let Some(text) = ke.text.as_deref()
                        && !text.is_empty()
                        && !text.chars().any(char::is_control)
                    {
                        self.start_chat_with(text);
                        return;
                    }
                    // Ctrl / Alt shortcuts of the viewer still work while a text
                    // field has the focus (chat bar, login fields...)
                    let shortcut = pressed
                        && shortcut_while_typing(
                            typing,
                            mods.ctrl,
                            mods.alt,
                            code,
                            self.settings.keybinds.triggered(&input, mods).is_some(),
                        );
                    // Ctrl+F belongs to the window with its own find when the
                    // script editor is active, the keyboard is in a conversation, or the
                    // pointer over a conversation with the keyboard nowhere:
                    // the viewer's shortcut on these keys ("Search")
                    // only runs otherwise. The rule: ui::find_shortcut::owner.
                    // The key is the one egui reads, which the window takes.
                    let find_key = matches!(&ke.logical_key, winit::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("f"));
                    let find_taken = pressed && self.ctrl && find_key && ui::find_shortcut::window_takes_it(&self.egui_ctx);
                    let script_command = pressed
                        && self.ctrl
                        && matches!(&ke.logical_key, winit::keyboard::Key::Character(c)
                            if c.eq_ignore_ascii_case("k") || (c.eq_ignore_ascii_case("s") && !self.shift && !self.alt))
                        && ui::scripts::editor_focused(&self.egui_ctx);
                    let document_save = pressed
                        && self.ctrl
                        && !self.shift
                        && !self.alt
                        && matches!(&ke.logical_key, winit::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("s"))
                        && ui::inventory::document_save_focused(&self.egui_ctx);
                    if pressed
                        && ((!resp.consumed && !typing) || shortcut || chat_pass)
                        && !ke.repeat
                        && !find_taken
                        && !script_command
                        && !document_save
                        && !ui_shortcut
                    {
                        self.on_input(Input::key(code), pressed);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x as f32, position.y as f32);
                self.recenter_mouselook_cursor();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.last_input = Instant::now();
                if *button == MouseButton::Left {
                    self.alt_cursor.left_button(*state == ElementState::Pressed);
                }
                if self.exit_prompt.open {
                    return;
                }
                let pressed = *state == ElementState::Pressed;
                let over_ui = self.ui_takes_pointer();
                // shortcut being captured: middle / side buttons bind, left / right cancel
                if self.options_ui.capture.is_some() {
                    if pressed {
                        self.finish_capture(Input::mouse(*button));
                    }
                    return;
                }
                // a press in the world takes the keyboard from the fields, also
                // when it turns into a drag (LLTool::handleAnyMouseClick:
                // setKeyboardFocus(NULL)); egui lets it go on a click only
                if pressed && !over_ui && self.in_world() {
                    self.egui_ctx.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
                }
                // (in its turn behind a click that waits for its depth)
                self.mouse_input(*button, pressed, over_ui);
            }
            WindowEvent::MouseWheel { delta, .. } if !self.exit_prompt.open && !self.ui_takes_pointer() && self.in_world() => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
                if self.on_build_scroll(steps) || self.on_hud_scroll(steps) || self.viewfinder_wheel(steps) {
                    return;
                }
                let mods = aurora_media::Modifiers {
                    control: self.ctrl,
                    alt: self.alt,
                    shift: self.shift,
                    command: self.command,
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
                            .hud_pick(&self.world, self.cursor_pos, false)
                            .map_or((ray, None, false), |(_, p, r)| (r, Some((p - r.0).dot(r.1)), true));
                        self.media.on_scroll(&self.world, &self.scene, ray, depth, steps, mods, hud)
                    }
                    None => false,
                };
                // (the free camera has no zoom: the wheel would change the
                // avatar's view behind it, or enter mouselook)
                if !on_media && self.camera.free().is_none() {
                    // SL wheel clicks: positive zooms out
                    self.camera_wheel(-steps);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            // a remote desktop's or a tablet's position: its motion (raw_mouse.rs)
            let Some(delta) = crate::raw_mouse::motion(delta) else {
                return;
            };
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
        self.sync_minimized_mute();
        self.record_window_geometry();
        self.attention_window();
        self.external_links();
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

/// The frame limiter's and the frame pacing's wait: a sleep to a
/// millisecond before `until` (the scheduler's grain), then a spin.
fn wait_until(until: Instant) {
    let now = Instant::now();
    if until <= now {
        return;
    }
    let rest = until - now;
    if rest > Duration::from_millis(2) {
        std::thread::sleep(rest - Duration::from_millis(1));
    }
    while Instant::now() < until {
        std::hint::spin_loop();
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

/// agent_jump (indra/newview/llviewerinput.cpp): AutomaticFly. Turning the
/// option off clears the hold, so enabling it cannot reuse an old press.
fn automatic_fly_held(since: &mut Option<Instant>, up: bool, enabled: bool, now: Instant) -> bool {
    if !up || !enabled {
        *since = None;
        return false;
    }
    now.duration_since(*since.get_or_insert(now)).as_secs_f32() > FLY_TIME
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
    fn automatic_flight_hold_restarts_after_release_or_disabling_the_preference() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut since = None;
        let mut held = |ms, up, enabled| super::automatic_fly_held(&mut since, up, enabled, start + Duration::from_millis(ms));
        assert!(!held(0, true, true));
        assert!(!held(500, true, true));
        assert!(held(501, true, true));
        assert!(!held(600, false, true));
        assert!(!held(1000, true, true));
        assert!(held(1501, true, true));
        assert!(!held(1600, true, false));
        assert!(!held(3000, true, false));
        assert!(!held(4000, true, true), "enabling starts a fresh hold");
        assert!(held(4501, true, true));
    }

    /// Ctrl+P opens the preferences from the login screen, a login field
    /// focused or not, as in the world; it types no "p" in the field.
    #[test]
    fn ctrl_p_opens_the_preferences_from_the_login_screen() {
        use super::{Screen, login_shortcut, shortcut_while_typing};
        use crate::keybinds::{Action, Input, Mods};
        use winit::keyboard::KeyCode;
        let bindings = Settings::default().keybinds;
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let action = bindings.triggered(&Input::key(KeyCode::KeyP), ctrl);
        assert_eq!(action, Some(Action::Preferences), "Ctrl+P is the preferences' shortcut");
        assert_eq!(bindings.triggered(&Input::key(KeyCode::KeyP), Mods::default()), None);
        // a field focused: the key still reaches the viewer's shortcuts
        assert!(shortcut_while_typing(true, true, false, KeyCode::KeyP, action.is_some()));
        // the fields keep their own editing shortcuts (Ctrl+C, Ctrl+V…)
        assert!(!shortcut_while_typing(true, true, false, KeyCode::KeyV, true));
        // a plain P while typing is the field's
        assert!(!shortcut_while_typing(true, false, false, KeyCode::KeyP, false));
        assert!(login_shortcut(&Screen::Login, action));
        // the other shortcuts wait for the world
        assert!(!login_shortcut(&Screen::Login, Some(Action::Fly)));
        assert!(!login_shortcut(&Screen::World, action), "in the world, the usual path opens them");
        assert!(!login_shortcut(
            &Screen::Loading {
                since: std::time::Instant::now()
            },
            action
        ));
    }

    #[test]
    fn right_click_tolerates_small_straight_and_diagonal_motion() {
        for (dx, dy) in [(7.0, 0.0), (0.0, -7.0), (6.0, 6.0)] {
            let mut gesture = RightGesture::default();
            assert!(!gesture.motion(dx, dy, 1.0));
            assert!(!gesture.orbit);
        }
    }

    #[test]
    fn mouse_walk_consumes_the_right_click_until_the_next_press() {
        let mut gesture = RightGesture::default();
        assert!(gesture.opens_menu(), "an ordinary right click keeps its menu");
        gesture.walked = true;
        assert!(!gesture.motion(0.0, 0.0, 1.0), "walking need not move the pointer");
        assert!(!gesture.opens_menu(), "the right release stays consumed after the left release");
        gesture = RightGesture::default();
        assert!(gesture.opens_menu(), "the next independent right click still works");
        assert!(gesture.motion(20.0, 0.0, 1.0));
        assert!(!gesture.opens_menu(), "a camera orbit still consumes its release");
    }

    #[test]
    fn world_mouse_walk_is_opt_in_and_survives_a_settings_round_trip() {
        let old: Settings = serde_json::from_str("{}").expect("old synthetic preferences");
        assert!(!old.world_mouse_walk);
        let enabled = Settings {
            world_mouse_walk: true,
            ..old
        };
        let saved = serde_json::to_string(&enabled).expect("serialize synthetic preferences");
        let restored: Settings = serde_json::from_str(&saved).expect("restore synthetic preferences");
        assert!(restored.world_mouse_walk);
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
