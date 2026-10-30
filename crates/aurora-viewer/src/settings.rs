//! Persistent user settings (JSON in the platform config directory).
//! Passwords are never stored.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod area_search;
pub mod backup;
mod hover_height;
pub mod stream_list;

/// How the Conversations window looks (Preferences › Aurora): Aurora's
/// workspace (the default: a column of conversations, messages grouped by
/// author with their picture, `ui/chat/aurora.rs`), its compact variant (one
/// row per message, no pictures), a messenger's bubbles (`ui/chat/bubbles.rs`)
/// or the lines of Firestorm's conversations (the legacy style).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChatStyle {
    #[default]
    Aurora,
    Compact,
    Bubbles,
    Legacy,
}

impl ChatStyle {
    /// Aurora's workspace (column, head, members, call stage): the two
    /// first styles. The others keep the window of tabs.
    pub fn workspace(self) -> bool {
        matches!(self, ChatStyle::Aurora | ChatStyle::Compact)
    }
}

/// Where the thumbnails of the open conversations stand (Preferences ›
/// Chat, `ui/chat/thumbnails.rs`): the Legacy Viewer's IM chiclets, shown
/// unless `FSDisableIMChiclets`. Beside the chat bar by default (the human's
/// choice of 2026-10-26; the Legacy Viewer keeps them at the top right).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChatThumbnails {
    Off,
    #[default]
    ChatBar,
    TopRight,
}

/// Where the islands show the favourites (Preferences › Interface,
/// "Favourites access"): a button of the place island and its menu (the
/// default), a row of pills under the islands, or a button of the dock and
/// its grid (`ui/bars/favorites.rs`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FavoritesAccess {
    #[default]
    Menu,
    Row,
    Dock,
}

/// What the system island shows (its right click, "Show in the island"),
/// as Firestorm's status bar menu shows or hides its parts
/// (menu_hide_navbar.xml); all shown by default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SystemIsland {
    pub clock: bool,
    pub ping: bool,
    pub fps: bool,
    /// Media, music and volume.
    pub media: bool,
    /// The time the bars' clock shows; a left click on it switches to the
    /// other (#453, `ui/bars/clock.rs`).
    #[serde(deserialize_with = "clock_zone_or_slt")]
    pub clock_zone: ClockZone,
    /// The clock's seconds (the Legacy Viewer's FSStatusBarTimeFormat
    /// "Seconds" formats); on, as Aurora's clock always showed them.
    pub clock_seconds: bool,
    /// 12 or 24 hours, by default as the system's regional format.
    pub clock_format: ClockFormat,
}

/// A saved clock time: UTC, a choice of the clock's first version, and
/// anything unknown fall back to SLT rather than failing the whole file.
fn clock_zone_or_slt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ClockZone, D::Error> {
    Ok(match String::deserialize(d)?.as_str() {
        "Local" => ClockZone::Local,
        _ => ClockZone::Slt,
    })
}

impl Default for SystemIsland {
    fn default() -> Self {
        SystemIsland {
            clock: true,
            ping: true,
            fps: true,
            media: true,
            clock_zone: ClockZone::Slt,
            clock_seconds: true,
            clock_format: ClockFormat::Auto,
        }
    }
}

/// The time of the bars' clock: Second Life time (US Pacific) or the
/// computer's own.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClockZone {
    #[default]
    Slt,
    Local,
}

/// The hours of the bars' clock: as the system's regional format (Windows'
/// short time, LC_TIME elsewhere), 12 hours with AM / PM, or 24 hours.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClockFormat {
    #[default]
    Auto,
    H12,
    H24,
}

/// How notifications arrive (Preferences › Aurora): unrolled from the bell
/// at the right end of the navigation bar and folded back into it (the
/// default, `ui/notification_center.rs`), or as Firestorm's toasts stacked
/// apart from the bell (`ui/notifications.rs`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum NotificationsStyle {
    #[default]
    Aurora,
    Legacy,
}

/// Where the friends' online / offline toasts of Aurora's notification
/// center stand (Preferences › Chat › Toasts), in the room the bars leave
/// (`ui/notification_center/presence.rs`). Top right by default, under the
/// bell, waiting while its cards or its list are out; Firestorm's toasts
/// keep them in their own stack.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PresencePosition {
    TopLeft,
    TopCenter,
    #[default]
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl PresencePosition {
    pub const ALL: [PresencePosition; 6] = [
        PresencePosition::TopLeft,
        PresencePosition::TopCenter,
        PresencePosition::TopRight,
        PresencePosition::BottomLeft,
        PresencePosition::BottomCenter,
        PresencePosition::BottomRight,
    ];

    /// The toasts start from the top of the room and stack downwards, or
    /// from its bottom and stack upwards.
    pub fn top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopCenter | Self::TopRight)
    }

    /// The name of the place in the preferences.
    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => aurora_i18n::tr!("ui-options-position-top-left"),
            Self::TopCenter => aurora_i18n::tr!("ui-options-position-top-center"),
            Self::TopRight => aurora_i18n::tr!("ui-options-position-top-right"),
            Self::BottomLeft => aurora_i18n::tr!("ui-options-position-bottom-left"),
            Self::BottomCenter => aurora_i18n::tr!("ui-options-position-bottom-center"),
            Self::BottomRight => aurora_i18n::tr!("ui-options-position-bottom-right"),
        }
    }

    /// The word of `AURORA_DEMO_PRESENCE` (`top-left` … `bottom-right`).
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "top-left" => Self::TopLeft,
            "top-center" => Self::TopCenter,
            "top-right" => Self::TopRight,
            "bottom-left" => Self::BottomLeft,
            "bottom-center" => Self::BottomCenter,
            "bottom-right" => Self::BottomRight,
            _ => return None,
        })
    }
}

/// How the world map lays out its controls: over a map filling the window
/// (the default), or in Firestorm's side panel next to the map (the legacy
/// style, Preferences › Aurora).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorldMapStyle {
    #[default]
    Aurora,
    Legacy,
}

/// The photo mode: Aurora's viewfinder (the world full screen under a
/// camera's overlay, `ui::viewfinder`), Aurora's panel anchored on the
/// right, or the Legacy Viewer's floater (floater_snapshot.xml: a floating
/// window, the controls on the left, the preview on the right).
/// Preferences › Aurora.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PhotoWindowStyle {
    #[default]
    Aurora,
    Panel,
    Legacy,
}

/// The order of the landmarks in Aurora's "Places" (`ui/places/aurora.rs`):
/// last visit first (the default), by name, or newest landmark first.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlacesSort {
    #[default]
    Visit,
    Name,
    Created,
}

/// What a plain, Shift or Ctrl left click on another avatar's name plate
/// does (Preferences › Avatars). Each gesture chooses an avatar-menu action
/// independently; Nothing, the default, leaves the click to the world.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlateClick {
    #[default]
    Nothing,
    Profile,
    InstantMessage,
    OfferTeleport,
    Menu,
}

impl PlateClick {
    pub const ALL: [PlateClick; 5] = [
        PlateClick::Nothing,
        PlateClick::Profile,
        PlateClick::InstantMessage,
        PlateClick::OfferTeleport,
        PlateClick::Menu,
    ];
}

/// What the crash window does with a crash report (Preferences › Privacy).
/// It becomes a public GitHub issue in the tester's name, hence Ask by
/// default where the Legacy Viewer always sends to its own server
/// (llcrashlogger.cpp, CrashSubmitBehavior).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CrashReports {
    #[default]
    Ask,
    /// Published as soon as the window opens, once GitHub is authorized.
    Always,
    /// No sending from the window; the report stays on the PC.
    Never,
}

/// The look of the world's hover tips (Preferences › Interface): a bubble
/// next to the pointer, a card with the details and the actions (the
/// default), or a fixed readout in a corner.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum HoverTipForm {
    Bubble,
    #[default]
    Card,
    Readout,
}

impl HoverTipForm {
    pub const ALL: [HoverTipForm; 3] = [HoverTipForm::Bubble, HoverTipForm::Card, HoverTipForm::Readout];
}

/// The world's hover tips: what is under the pointer, object, avatar or
/// land (the Legacy Viewer's "Hover Tips" of panel_preferences_UI.xml).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HoverTips {
    /// ShowHoverTips (View › Show Tips, Ctrl+Shift+T). Off by default, the
    /// human's choice (2026-10-21); the Legacy Viewer's is on.
    pub enabled: bool,
    pub form: HoverTipForm,
    /// ShowAllObjectHoverTip: every object, not only those that can be
    /// touched, paid, bought or have media (off, as in the Legacy Viewer).
    pub all_objects: bool,
    /// ShowLandHoverTip: the parcel under the pointer (off, as in the
    /// Legacy Viewer).
    pub land: bool,
    /// FSShowGroupTitleInTooltip: the avatars' group title above their name.
    pub group_titles: bool,
    /// Seconds the pointer rests before a tip shows (ObjectInspectorTooltipDelay
    /// and AvatarInspectorTooltipDelay, one setting here).
    pub delay: f32,
}

/// The longest hover tip delay of the preferences' slider (the Legacy
/// Viewer's delay sliders go to 6 s).
pub const MAX_HOVER_TIP_DELAY: f32 = 6.0;

impl Default for HoverTips {
    fn default() -> Self {
        HoverTips {
            enabled: false,
            form: HoverTipForm::Card,
            all_objects: false,
            land: false,
            group_titles: true,
            delay: 0.35,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GridChoice {
    SecondLife,
    SecondLifeBeta,
    Custom,
}

/// Debug overlays (the Dev Toolbox's toggles), not saved: they reset at each launch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DebugView {
    /// Complexity under the name tags, green to red (Firestorm FSTagShowARW).
    pub complexity_tags: bool,
    /// "Show transparency" (Ctrl+Alt+T,LLDrawPoolAlpha::sShowDebugAlpha).
    pub show_alpha: bool,
    /// ... rigged faces too (Firestorm sShowDebugAlphaRigged).
    pub show_alpha_rigged: bool,
    pub wireframe: bool,
    /// Object bounding boxes.
    pub bounds: bool,
    /// Bounding boxes colored by what the culling decided.
    pub culling: bool,
    /// Keep the culling view where it is (move around to see what it drops).
    pub freeze_culling: bool,
    pub lights: bool,
    /// Objects with glowing faces.
    pub glow_objects: bool,
    /// Glow amount view (red = glow x10).
    pub glow_view: bool,
    /// The shadow cascade each pixel reads, instead of the image.
    pub shadow_view: bool,
    /// The ambient occlusion alone, instead of the image.
    pub ao_view: bool,
    /// Magenta where the sort of the local lights by screen tile misses one.
    pub check_light_sets: bool,
    /// Magenta where the sort of the reflection probes misses one.
    pub check_probe_sets: bool,
    pub probes: bool,
    pub skeletons: bool,
    /// Detail level of the objects around the pointer, with the distance
    /// and the radius it comes from (Firestorm: Develop › Render Metadata ›
    /// LOD Info).
    pub lod_info: bool,
    /// Dev Toolbox R3: every face tinted by the pass it is drawn in.
    pub pass_view: bool,
}

impl DebugView {
    /// One view at a time replaces the image: the one just turned on
    /// (`before`: the state the interface started from) turns the others
    /// off.
    pub fn keep_one_view(&mut self, before: &DebugView) {
        let turned_on = [
            self.glow_view && !before.glow_view,
            self.shadow_view && !before.shadow_view,
            self.ao_view && !before.ao_view,
        ];
        if let Some(kept) = turned_on.iter().position(|&on| on) {
            self.glow_view = kept == 0;
            self.shadow_view = kept == 1;
            self.ao_view = kept == 2;
        }
    }

    /// What the renderer is asked for.
    pub fn render(&self) -> aurora_render::DebugRender {
        aurora_render::DebugRender {
            shadows: self.shadow_view,
            ambient_occlusion: self.ao_view,
            check_light_sets: self.check_light_sets,
            check_probe_sets: self.check_probe_sets,
        }
    }

    /// An overlay drawn over the 3D view is on.
    pub fn any_overlay(&self) -> bool {
        self.bounds || self.culling || self.lights || self.glow_objects || self.probes || self.skeletons || self.lod_info
    }
}

/// The former look-at range slider's maximum, which meant no limit.
const FORMER_LOOK_AT_UNLIMITED: f32 = 64.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub update_mode: crate::updater::Mode,
    /// Stable releases only, or also the betas built every night
    /// (Preferences › Language and updates).
    pub update_channel: aurora_updater::feed::Channel,
    pub ignored_update: String,
    pub area_search: area_search::AreaSearch,
    pub crash_reports: CrashReports,
    /// Firestorm Language: default follows the OS.
    pub language: String,
    /// Firestorm LanguageIsPublic, exposed through llGetAgentLanguage.
    pub language_is_public: bool,
    /// Alpha feedback shortcut; Help and login remain available.
    pub show_bug_report: bool,
    /// ShowHUDAttachments, enabled by default in Firestorm.
    pub show_huds: bool,
    /// sunbeacon / moonbeacon: a line from the avatar towards the sun or
    /// the moon ("Show Beacon" of the Sun & Moon tab).
    pub sun_beacon: bool,
    pub moon_beacon: bool,
    pub beacons: crate::world::beacons::Beacons,
    pub world_overlays: crate::world::beacons::Overlays,
    /// Debug overlays (not saved).
    #[serde(skip)]
    pub debug: DebugView,
    /// The Dev Toolbox's arrangement (Customize); None: the default one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_toolbox: Option<crate::dev_toolbox::tools::Layout>,
    /// Local construction preferences and layout, shared by every account.
    pub build: crate::build::BuildSettings,
    /// Camera (Firestorm "Move & View › View").
    pub camera: crate::camera::CameraSettings,
    /// The crosshair in mouselook (ShowCrosshairs; #1166).
    pub crosshair: crate::ui::crosshair::Settings,
    /// LLViewerJoystick: device, axis mapping and separate avatar/flycam tuning.
    pub joystick: crate::joystick::Settings,
    /// Ground foot geometry after wearing/detaching a body attachment.
    pub auto_hover_height: bool,
    /// Fixed adjustment added to each automatic grounding estimate (metres).
    pub auto_hover_offset: f32,
    /// None keeps the former shared hover options until the first login.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hover_height_accounts: Option<std::collections::HashMap<uuid::Uuid, hover_height::Preferences>>,
    /// Runtime hover options belong to this account after login.
    #[serde(skip)]
    pub(crate) hover_height_account: Option<uuid::Uuid>,
    /// Contacts window (Friends / Groups / Contact sets).
    pub contacts: crate::ui::contacts::ContactsSettings,
    /// Radar of the People window.
    pub radar: crate::ui::people::RadarSettings,
    /// Friends tab of the People window.
    pub friends_list: crate::ui::people::FriendsSettings,
    pub inventory: crate::ui::inventory::InventoryPreferences,
    pub texture_library: crate::ui::texture_library::Preferences,
    pub gestures: crate::ui::gestures::Settings,
    pub imports: crate::imports::Preferences,
    pub ao: crate::ao::Settings,
    /// Requested position lock and options, independent for each avatar.
    pub bridge: std::collections::HashMap<uuid::Uuid, crate::world::bridge::Preferences>,
    pub movements: std::collections::HashMap<uuid::Uuid, crate::movement::Preferences>,
    /// Photo window (Firestorm's snapshot floater settings).
    pub snapshot: crate::snapshot::Settings,
    /// Depth of field (Preferences › Graphics).
    pub dof: crate::dof::Settings,
    pub shape_show_hints: bool,
    pub shape_height_feet: bool,
    /// The shape editor stands our avatar up and plays the customise
    /// animation, which turns it to face the camera (on, as the Legacy
    /// Viewer always does; Preferences › Avatars › Animations). Off, our
    /// avatar keeps its pose and the camera frames it from the front.
    pub customize_animation: bool,
    pub script_editor: crate::ui::scripts::settings::ScriptSettings,
    pub username: String,
    pub remember_username: bool,
    /// Remember the password (its login hash, in the system credential
    /// store; Firestorm "Remember password"). Off by default.
    pub remember_password: bool,
    pub grid: GridChoice,
    pub custom_login_uri: String,
    /// Saved grid metadata, independent of the account catalogue.
    pub grids: Vec<crate::grids::Grid>,
    pub start_location: String,
    pub start_region: String,
    pub draw_distance: f32,
    /// The GPU to draw with (Preferences › Graphics; #894): empty for the
    /// automatic choice (discrete first), or the adapter's name; read at
    /// startup (`aurora_render::adapter_choice`), `AURORA_GPU` overrides it.
    pub gpu_adapter: String,
    /// The graphics driver whose "out of date" warning the user dismissed
    /// on the login screen (`gpu_driver::Notice::key`): a newer driver that
    /// is still too old warns again.
    pub gpu_driver_dismissed: String,
    /// Let other programs' overlay layers (Steam overlay, RivaTuner, Epic…)
    /// into the viewer (Preferences › Graphics); off: kept out at startup
    /// (`aurora_render::vulkan_layers`), read by `allow_overlays_at_start`.
    pub allow_overlays: bool,
    pub vsync: bool,
    pub shadows: bool,
    pub force_midday: bool,
    /// Local time of day: 0 shared (region EEP), 1 sunrise, 2 noon, 3 sunset, 4 midnight.
    pub time_of_day: u8,
    /// Crossfade (s) of manual environment changes: selector, presets
    /// (Firestorm FSEnvironmentManualTransitionTime, 0 = instant).
    pub env_manual_transition: f32,
    /// Keep the local environment from one session to the next (Firestorm
    /// EnvironmentPersistAcrossLogin, on).
    pub env_persist: bool,
    pub exposure: f32,
    /// Contrast adaptive sharpening (Firestorm RenderCASSharpness, 0.4).
    pub sharpen: f32,
    /// Glow of faces with a glow value (Firestorm RenderGlow, on).
    pub glow: bool,
    /// Aurora's bloom around the bright parts of the image (the Legacy
    /// Viewer has none): off by default.
    pub bloom: bool,
    /// The share of the light over the threshold it spreads (0.05 to 1).
    pub bloom_strength: f32,
    /// GPU occlusion culling: objects hidden behind others are not drawn
    /// (Firestorm UseOcclusion, on by default). Renamed from `gpu_occlusion`
    /// (off while it cost more than it saved) so that saved settings take
    /// the new default.
    pub occlusion_culling: bool,
    pub lod_factor: f32,
    /// Texture memory sized from the detected video memory (else `texture_budget_mb`).
    pub texture_budget_auto: bool,
    pub texture_budget_mb: u32,
    pub ui_scale: f32,
    /// Firestorm UsePieMenu. Keep Aurora's existing list menu by default.
    pub use_pie_menu: bool,
    /// Independent arrangements for land, objects, avatars, attachments and HUDs.
    pub world_context_menus: crate::ui::context::Preferences,
    pub particle_filters: crate::world::particle_filters::ParticleFilters,
    /// Windows snap to each other and to the edges while they are moved or
    /// resized (Firestorm SnapEnabled, on).
    pub snap_windows: bool,
    /// Windows stop at the left, right and bottom edges of the screen (on):
    /// off, a window may cross that edge, its title bar kept within reach
    /// (`ui::layout::Edges`). It never crosses the top edge.
    pub windows_stop_left: bool,
    pub windows_stop_right: bool,
    pub windows_stop_bottom: bool,
    /// Performance opens with the session (Preferences › General, off):
    /// the user's choice only, not the window's state at exit. Off, the
    /// frame rate of the bars opens it.
    pub show_perf: bool,
    /// Conversations and the mini-map open with the session: the window's
    /// last open or close, or the switch of Preferences › Layout › Startup
    /// turned since (`ui::StartupWindows`).
    pub show_chat: bool,
    pub show_minimap: bool,
    /// FSUseStandalonePlaceDetailsFloater: landmarks, place links and history
    /// entries open in their own window instead of the Places window.
    pub standalone_place_details: bool,
    /// FSInspectColumnConfig: columns of the Inspect window (ui/inspect.rs).
    pub inspect_columns: u32,
    /// LandmarksSortedByDate: the Landmarks tab of Places sorted by date.
    pub landmarks_by_date: bool,
    /// Ask before a teleport to a landmark, a history entry or a place link
    /// (TeleportFromLandmark, TeleportToHistoryEntry, TeleportViaSLAPP; the
    /// box "Do not show again" of the confirmation turns it off). Firestorm
    /// keeps one "ignore" per notification: one setting here for the three.
    pub confirm_teleports: bool,
    /// SettingsMakeNoTrans is asked (its "ignore" box turns it off; shown
    /// by default like the notification).
    pub confirm_no_transfer_settings: bool,
    /// ChangeObjectBonusFactor is asked before Region / Estate sends a new
    /// object bonus (its "ignore" box turns it off; shown by default).
    pub confirm_object_bonus: bool,
    /// The object menu asks before "Return" (ReturnToOwner), "Drop"
    /// (AttachmentDrop) and the script actions on a selection
    /// (ConfirmScriptModify); their "ignore" boxes turn these off.
    pub confirm_return_to_owner: bool,
    pub confirm_attachment_drop: bool,
    pub confirm_script_modify: bool,
    /// RegionCheckTextureHeights: Region / Estate asks before sending
    /// elevation ranges whose low end is above the high one ("Don't ask"
    /// turns it off; on by default).
    pub region_check_texture_heights: bool,
    /// Ask before a payment above `payment_confirm_threshold` L$
    /// (FSConfirmPayments, on by default).
    pub confirm_payments: bool,
    /// FSPaymentConfirmationThreshold: 200 L$ by default.
    pub payment_confirm_threshold: i32,
    /// The money tracker records the payments even while it is closed
    /// (FSAlwaysTrackPayments, off by default).
    pub always_track_payments: bool,
    /// Preferences › Chat › Conversations: a friend's direct payment opens
    /// their conversation on its card (#1198); a payment from anyone else
    /// shows there only when it is open. On by default.
    pub payment_opens_conversation: bool,
    /// Preferences › Chat › Conversations: our own direct payment to a
    /// resident opens their conversation on its card (#1198); otherwise it
    /// shows there only when it is open. Off by default.
    pub payment_sent_opens_conversation: bool,
    /// Mini-map and world map options (Firestorm MiniMap* / Map* settings).
    pub maps: MapSettings,
    /// "Search": last tab, rating boxes and land limits.
    pub search: crate::world::search::SearchSettings,
    /// Other panels open at exit (restored at startup) and the People tab.
    pub show_people: bool,
    pub people_tab: u8,
    pub show_inventory: bool,
    pub show_appearance: bool,
    /// Crop outfit gallery previews to fill their frames (off: fit the whole image).
    pub appearance_fill_previews: bool,
    pub show_ao: bool,
    /// Loading / teleport screens show the last view, blurred.
    pub loading_backdrop: bool,
    pub mouse_sensitivity: f32,
    pub window_width: u32,
    pub window_height: u32,
    /// Last window position (outer, physical pixels) and maximized state
    /// (Firestorm WindowX / WindowY / WindowMaximized). A first launch opens
    /// maximized (`window_starts_maximized`).
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub window_maximized: bool,
    /// Legacy single-account authorization, read once for migration only.
    #[serde(skip_serializing)]
    pub mfa_hash: String,
    pub font: String,
    pub font_scale: f32,
    pub chat_timestamps: bool,
    /// The channel selector of the nearby chat's field (FSShowChatChannel,
    /// off by default; Preferences > Chat). "/n" before the text works
    /// without it.
    pub show_chat_channel: bool,
    /// Our own objects' script errors in the nearby chat, with a red
    /// warning sign, instead of opening the Script debug window, which
    /// still keeps them (ShowScriptErrorsLocation 0; on, the human's choice
    /// for #1263, the window in the Legacy Viewer; Preferences > Chat).
    pub script_errors_in_chat: bool,
    /// They are also said on `script_errors_channel` (off; #1263, not in
    /// the Legacy Viewer).
    pub script_errors_to_channel: bool,
    pub script_errors_channel: i32,
    /// What objects say in the nearby chat on one small row like the
    /// viewer's notices (#475, off by default; Preferences > Chat). Not in
    /// the Legacy Viewer.
    pub compact_automated_chat: bool,
    /// With it, the system lines of the conversations (calls, automatic
    /// replies) too (off, the human's choice of 2026-10-24).
    pub compact_im_system_lines: bool,
    /// The local chat bar keeps the keyboard after a message is sent; Enter
    /// on the empty bar gives it back to the world (the Legacy Viewer's
    /// CloseChatOnReturn off). On by default (human choice of 2026-10-16);
    /// off, the bar lets go after sending.
    pub chat_keep_focus: bool,
    /// The nearby chat fields complete a gesture's trigger after `/`, with
    /// the list of the matching ones (FSChatbarGestureAutoCompleteEnable, on).
    pub chat_gesture_autocomplete: bool,
    /// The nearby chat fields complete the name of a resident around after
    /// three letters (FSChatbarNamePrediction; on, the human's choice of
    /// 2026-10-26, where the Legacy Viewer leaves it off).
    pub chat_name_prediction: bool,
    /// ":" at the start of a line is "/me", in the nearby chat and the
    /// conversations (AllowMUpose, off; #1202): ":waves" is "Name waves".
    pub colon_emote: bool,
    /// `@` in a chat field lists the residents of the conversation to mention
    /// (FSUseChatMentionAutoComplete, on).
    pub chat_mention_autocomplete: bool,
    /// The local chat bar (the chat island) shows only while it is used:
    /// Enter or a letter key brings it (#1009). The Legacy Viewer's
    /// `AutohideChatBar`, off by default; with the islands the default dock
    /// then keeps the Conversations button.
    pub autohide_chat_bar: bool,
    /// Show Contacts in Conversations (Preferences > Chat, on by default as
    /// the Legacy Viewer's always-present entry; it can be hidden there).
    pub show_contacts_tab: bool,
    /// The conversations asked for (a name's menu, a toast, the bell, a
    /// call) open in their own window instead of the Conversations window,
    /// the nearby chat excepted (#717; the Legacy Viewer's separate IM
    /// windows, FSChatWindow 0; off, its tabbed window by default).
    pub conversations_own_windows: bool,
    /// Tell the resident of a conversation that we are typing
    /// (FSSendTypingState, on by default; Preferences > Chat).
    pub send_typing_state: bool,
    /// Our avatar plays the typing animation while we type in the nearby
    /// chat (PlayTypingAnim, on; Preferences > Chat); the typing notices go
    /// out either way.
    pub play_typing_anim: bool,
    /// An emote ("/me") shows us typing too (FSTypeDuringEmote, off).
    pub type_during_emote: bool,
    /// The name tags show who types: the plates' dots, "(Typing)" on the
    /// classic tags (FSShowTypingStateInNameTag). On, unlike the Legacy
    /// Viewer's off: the plates have shown it since their redesign.
    pub typing_in_name_tags: bool,
    /// A "> " before "… is typing" in the tab of a conversation with one
    /// resident (FSTypingChevronPrefix, off).
    pub typing_chevron: bool,
    /// Nearby chat in bubbles above the residents' heads, under their name
    /// (UseChatBubbles, off; Preferences > Chat; #673).
    pub chat_bubbles: bool,
    /// With the bubbles, the residents' lines leave the chat above the chat
    /// bar; the nearby chat keeps them (FSBubblesHideConsoleAndToasts, on).
    pub chat_bubbles_hide_console: bool,
    /// Three dots in the bubble of a resident who types
    /// (UseTypingBubbles, off).
    pub typing_bubbles: bool,
    /// Opacity of the bubbles' background (ChatBubbleOpacity, 0.5).
    pub chat_bubble_opacity: f32,
    /// The entry of a conversation that receives a message while another
    /// is shown flashes in violet, then stays tinted until read
    /// (FSFloaterIMContainer::startFlashingTab, FlashCount / FlashPeriod;
    /// Preferences > Chat, on).
    pub flash_unread_conversations: bool,
    /// The Conversations button flashes when an IM, a group chat or a
    /// conference message arrives while the window is closed, then stays
    /// lit until it is opened; minimized, its bar flashes instead
    /// (FSNotifyIMFlash, on).
    pub flash_conversations_button: bool,
    /// A resident's line of nearby chat flashes it too, while the window is
    /// closed (FSNotifyNearbyChatFlash, on); the button's right-click menu
    /// turns it off, IMs and group chats keep flashing.
    pub flash_conversations_nearby: bool,
    /// The thumbnails of the open conversations, with their unread count
    /// (the Legacy Viewer's IM chiclets, #1134): where, or none.
    pub chat_thumbnails: ChatThumbnails,
    /// A thumbnail flashes when its conversation receives a message, then
    /// keeps a mark until it is read (on).
    pub chat_thumbnails_flash: bool,
    /// Flash the taskbar button when a message, an offer or a mention
    /// arrives while the window is not in front (FSFlashOnMessage; on here,
    /// off in the Legacy Viewer: the point of #680 is not to miss them).
    pub flash_on_message: bool,
    /// Also for an object's message (FSFlashOnObjectIM, on).
    pub flash_on_object_im: bool,
    /// Flash for a script dialog (FSFlashOnScriptDialog, off).
    pub flash_on_script_dialog: bool,
    /// Windows notifications for messages, mentions, offers and alerts
    /// while the window is not in front (FSEnableGrowl, off).
    pub system_notifications: bool,
    /// The same notifications while the window is in front (FSGrowlWhenActive, off).
    pub system_notifications_when_active: bool,
    pub chat_toast_seconds: f32,
    /// Toast when a friend comes online or goes offline (ChatOnlineNotification, on).
    pub toast_friends_online: bool,
    /// The same notices as a line of nearby chat (OnlineOfflinetoNearbyChat;
    /// off here, on in the Legacy Viewer's own settings).
    pub friends_online_in_chat: bool,
    /// Where those toasts stand with Aurora's notification center.
    pub presence_position: PresencePosition,
    /// A friendship offer shows as a card in the middle of the screen
    /// instead of a toast (`ui::friend_prompt`, #465; on, an Aurora
    /// addition the Legacy Viewer does not have: its toast stays one switch
    /// away).
    pub friend_request_prompt: bool,
    /// A message to the whole region shows as a card in the middle of the
    /// screen instead of a toast (`ui::region_message_prompt`, #1404; on,
    /// an Aurora addition: off gives back the Legacy Viewer's GodMessage
    /// toast). Its line of nearby chat stays either way.
    pub region_message_prompt: bool,
    /// Toast of an IM or conference message out of sight (EnableIMChatPopups, on).
    pub toast_im: bool,
    /// Toast of a group chat message (EnableGroupChatPopups, off).
    pub toast_group_im: bool,
    /// The message toasts also stay under the bell until their conversation
    /// is read. Off: the toast only, as the Legacy Viewer's notification
    /// well never lists IMs (#752).
    pub ims_in_notification_center: bool,
    /// Seconds a message toast stays (NotificationToastLifeTime, 30).
    pub im_toast_seconds: f32,
    /// Write instant messages, group chats and conferences to text files in
    /// the account's folder (Firestorm KeepConversationLogTranscripts = 2).
    pub log_im: bool,
    /// Write the nearby chat too (Firestorm LogNearbyChat, off).
    pub log_nearby: bool,
    /// Reopen the private conversations of the last session at login (the
    /// Legacy Viewer's FSRestoreOpenIMs, off).
    pub reopen_ims: bool,
    /// Show the end of a conversation's file when it opens (LogShowHistory).
    pub log_show_history: bool,
    /// Date in the names of these files (LogFileNamewithDate, off).
    pub log_dated_names: bool,
    /// Ask before opening a web link outside the trusted domains (the box
    /// "Do not warn me again" of the warning turns it off).
    pub warn_external_links: bool,
    /// Sites added to, or removed from, the built-in trusted domains
    /// (Preferences › Protections; link_trust.rs).
    pub trusted_sites: crate::link_trust::TrustedSites,
    /// FSMuteAllGroups: refuse every group chat.
    pub mute_all_groups: bool,
    /// FSMuteGroupWhenNoticesDisabled: refuse group chat of groups whose
    /// notices are turned off.
    pub mute_groups_without_notices: bool,
    /// GroupSnoozeTime: how long the snooze of a group chat lasts, in
    /// minutes (15; 0 only closes it until its next message). The Legacy
    /// Viewer counts seconds: a snooze is minutes to hours long (#945).
    pub group_snooze_minutes: u32,
    /// The former duration in seconds: read once, carried into
    /// `group_snooze_minutes`.
    #[serde(rename = "group_snooze_seconds", skip_serializing)]
    pub(crate) former_group_snooze_seconds: Option<u32>,
    /// FSEnablePerGroupSnoozeDuration: ask the duration at each snooze
    /// (off).
    pub group_snooze_ask: bool,
    /// FSReportBlockToNearbyChat: say in nearby chat when the block list changes.
    pub report_blocks: bool,
    /// Show the invitations to a group we are already in (the Legacy
    /// Viewer's FSShowJoinedGroupInvitations, off: they are dropped).
    pub show_joined_group_invitations: bool,
    /// Tell in nearby chat when the region's script count changes by at
    /// least the threshold (FSReportTotalScriptCountChanges, off; 100).
    pub script_count_alert: bool,
    pub script_count_threshold: u32,
    /// A notice when the region runs another simulator version than the
    /// last one (FSShowServerVersionChangeNotice). Off, unlike the Legacy
    /// Viewer's on: it came at nearly every teleport and region crossing
    /// and told the user nothing they could act on (human choice,
    /// 2026-10-30).
    pub sim_version_alert: bool,
    /// Object messages from elsewhere dropped (Preferences › Privacy, #623).
    pub object_im_filter: crate::world::object_im_filter::Settings,
    pub anti_spam: crate::world::anti_spam::Settings,
    /// Global RLVa opt-in. Session restrictions are never persisted.
    pub rlva_enabled: bool,
    pub rlva: crate::world::rlva::preferences::Preferences,
    pub pose_requests: crate::world::poser_consent::Preferences,
    pub poser: crate::ik_edit::poser::Config,
    /// Automatic responses and the kept online status modes.
    pub autoresponse: crate::world::status::AutoResponse,
    /// The chat bar as a command line (FSCmdLine*, Preferences › Chat ›
    /// Commands).
    pub chat_commands: crate::cmdline::ChatCommandSettings,
    /// Keyword alerts in chat and IMs (FSKeyword*, Preferences › Chat).
    pub keywords: crate::keywords::KeywordSettings,
    /// Spell checking of the text fields (SpellCheck, SpellCheckDictionary;
    /// Preferences › Chat › Spelling).
    pub spelling: crate::spelling::SpellSettings,
    /// AutoReplace: the keywords of the replacement lists replaced as they
    /// are typed (off, as in the Legacy Viewer).
    pub auto_replace: bool,
    pub name_tag_distance: f32,
    pub show_own_name_tag: bool,
    pub show_friend_name_tags: bool,
    pub show_other_name_tags: bool,
    /// LLVOAvatar::idleUpdateNameTag: fade after a continuous visible interval.
    pub temporary_name_tags: bool,
    pub name_tag_show_seconds: f32,
    /// Avatar name tags drawn as before the name plates (`ui::nameplate`):
    /// lines of text on a flat tag, in the tag colors of the preferences.
    pub classic_name_tags: bool,
    /// Complete name plates for friends only: the others stop at the
    /// compact pill (off by default).
    pub plates_friends_only_full: bool,
    /// A friend's name plate takes a light tint of the friends' color of
    /// Preferences › Colors (on by default): friends stand out in a crowd.
    pub plates_friend_tint: bool,
    /// A name plate under the pointer shows full while it stays there (on
    /// by default).
    pub plates_hover_expand: bool,
    /// Preferences › Just for fun (off by default, #1112): a plate wobbles
    /// like a jelly each time the pointer reaches it, and overshoots when it
    /// grows (`ui/nameplate/fidget.rs`).
    pub plates_jiggle: bool,
    /// Preferences › Just for fun (off by default, #1114): the clicks on
    /// each plate are counted, and a combo shows a counter
    /// (`ui/nameplate/combo.rs`).
    pub plates_click_combo: bool,
    pub plate_click: PlateClick,
    pub plate_shift_click: PlateClick,
    pub plate_ctrl_click: PlateClick,
    /// Profiles in Firestorm's tabbed window (Second Life, Feed, Favorites…)
    /// instead of the header and cards of `ui/profile/aurora.rs`; group
    /// profiles in the classic window too (`ui/group_profile.rs`).
    pub classic_profiles: bool,
    /// A friend's rights on the Info card of the Aurora profile (on by
    /// default); off, they stay in the Contacts window.
    pub profile_rights: bool,
    /// Firestorm's Pay window (floater_pay.xml) instead of `ui/pay.rs`.
    pub classic_pay_window: bool,
    /// The Legacy Viewer's money tracker (floater_fs_money_tracker.xml)
    /// instead of the Aurora look of `ui/money_tracker/aurora.rs`.
    pub legacy_money_tracker: bool,
    /// The full-width top and bottom bars of Firestorm instead of the glass
    /// islands of `ui/bars/islands.rs`.
    pub classic_bars: bool,
    /// The place island stretches to the islands on its right (on by
    /// default); off, it is as wide as its content.
    pub place_bar_stretch: bool,
    /// The parcel rights of the place island are named next to their icons
    /// when there is room (off by default: icons alone, named in their
    /// tooltips).
    pub place_bar_rights_text: bool,
    /// Where the islands show the favourites.
    pub favorites_access: FavoritesAccess,
    /// The row of favourites (`FavoritesAccess::Row`) folded to its tab.
    pub favorites_row_folded: bool,
    /// What the system island shows.
    pub system_island: SystemIsland,
    /// Firestorm's ConfirmHideUI ("Confirm before hiding UI"): with
    /// Firestorm's bars, hiding the interface asks first.
    pub confirm_hide_interface: bool,
    /// The avatars' name tags in immersive mode (#859): off by default, its
    /// button beside the one that leaves it.
    pub immersive_name_tags: bool,
    /// The HUDs in immersive mode (#1190): shown by default, as Show HUDs
    /// leaves them, its button beside the name tags'.
    pub immersive_huds: bool,
    /// Firestorm's traffic indicator (ShowNetStats, off by default): a
    /// small curve of the network in the system island.
    pub show_net_stats: bool,
    /// The L$ balance in the top bar (the Legacy Viewer's
    /// FSShowCurrencyBalanceInStatusbar, on by default; #1340): hidden, it
    /// stays out of sight on a stream.
    pub show_balance: bool,
    /// People › Near you as Firestorm's radar table (`ui/people/radar.rs`)
    /// instead of the rows grouped by range and the sheet of
    /// `ui/people/nearby.rs`.
    pub classic_people: bool,
    /// Firestorm's yellow region restart box (floater_region_restarting.xml)
    /// instead of the alert card of `ui/region_restart.rs`.
    pub classic_restart_alert: bool,
    /// The Legacy Viewer's one-line nearby chat lines above the chat bar
    /// instead of the cards grouped by sender, with their picture
    /// (`ui/chat/console.rs`).
    pub classic_chat_lines: bool,
    /// The Legacy Viewer's grey objects while they load instead of the rez
    /// effect (`scene/rez.rs`): a glowing grid, then a line revealing them.
    /// The effect by default since it covers every loading path (no face
    /// stays grey: `rez_still_grey` in object.wgsl).
    pub classic_rez_effect: bool,
    /// The world's hover tips (Preferences › Interface).
    pub hover_tips: HoverTips,
    /// The Legacy Viewer's plain text hover tips with their "i" button
    /// instead of Aurora's bubble, card or readout (`ui/hover_tips.rs`).
    pub classic_hover_tips: bool,
    /// Aurora's movement of our avatar (Preferences › Aurora › Movement,
    /// `own_motion.rs`): it moves as soon as a key is pressed and corrections
    /// fade slowly. Off, the default until it has been tried on the grid:
    /// Firestorm's, the avatar moves when the simulator's update comes back.
    pub predict_own_motion: bool,
    /// Preferences › Aurora › Animations: follow the first known signal's
    /// elapsed time, including late loading and edit release. Off by default:
    /// the Legacy Viewer starts at asset activation and suspends during edits.
    pub synchronize_animations: bool,
    /// Experimental filing of new deliveries by the sender's legacy name.
    pub sort_received_items: bool,
    pub received_items_since: i64,
    /// Preferences › Inventory › Received items: a notification for each
    /// Marketplace delivery (Aurora only, the Legacy Viewer just marks the
    /// inbox new; #752). On by default: nothing else tells about a gift.
    pub notify_deliveries: bool,
    /// Preferences › Inventory › Received items: keeping an offer opens the
    /// inventory on what it gave (ShowInInventory, off by default as in the
    /// Legacy Viewer).
    pub show_accepted_in_inventory: bool,
    /// Replace native motion foot placement/recovery with Aurora's terrain
    /// solver. Off by default: the Legacy Viewer behaviour remains automatic.
    pub ground_avatar_feet: bool,
    /// Also adapt walking, running and crouch-walking feet. Off by default.
    pub ground_feet_while_moving: bool,
    /// Maximum vertical terrain correction in metres, relative to animation.
    pub ground_feet_max_height: f32,
    /// Other avatars receiving Aurora leg correction, nearest in the camera
    /// frustum. Our avatar is outside this budget; zero means self only.
    pub ground_feet_max_avatars: u32,
    /// The region restart alert (Preferences › Interface › Region restart).
    pub region_restart: crate::world::region_restart::Settings,
    /// Bumps, pushes and hits told in nearby chat or to scripts
    /// (Preferences › Interface › Bumps, pushes & hits).
    pub collisions: crate::world::collisions::Settings,
    /// "Reduce animations": the interface's pulses, shakes and spinners
    /// stand still (the region restart alert, the rez effect, the bounce of
    /// a hovered name plate), and egui's transitions end at once
    /// (`theme::apply_motion`).
    pub reduce_motion: bool,
    /// Preferences › Accessibility › Colour blindness: the interface's
    /// colours adapted to a colour vision deficiency (`crate::colorblind`),
    /// off by default.
    pub color_blindness: crate::colorblind::Deficiency,
    /// How strongly (0–1, 1 by default: the full correction).
    pub color_blindness_strength: f32,
    /// Preferences › Accessibility › Vision: the high contrast palette
    /// (`crate::high_contrast`, #906) instead of the skin's colours, off by
    /// default.
    pub high_contrast: bool,
    /// Preferences › Appearance › Skin: the accent, background and text
    /// colours of the interface (`crate::interface_colors`), each the
    /// skin's by default.
    pub interface_colors: crate::interface_colors::InterfaceColors,
    /// Firestorm's camera and movement windows (floater_camera.xml,
    /// floater_moveview.xml) instead of `ui/controls.rs`.
    pub classic_controls: bool,
    /// Camera presets, saved from the Controls window (Firestorm's camera
    /// presets) or the viewfinder (`camera::presets`); kept apart from
    /// `camera`, which "Reset" sets back.
    pub camera_shots: Vec<crate::camera::CameraShot>,
    /// The lines of the Conversations window (`ui/chat/bubbles.rs` or the
    /// legacy lines).
    pub chat_style: ChatStyle,
    /// Pictures and videos under the links of the Conversations window
    /// (Preferences › Chat; `link_preview.rs`).
    pub link_previews: crate::link_preview::Settings,
    /// The GIF button of the Conversations field and its picker
    /// (Preferences › Chat; `gif_search.rs`).
    pub gifs: crate::gif_search::Settings,
    /// The notification center's style (Preferences › Aurora).
    pub notifications_style: NotificationsStyle,
    /// The world map's layout (`ui/worldmap/overlay.rs` or the side panel).
    pub world_map_style: WorldMapStyle,
    /// The photo window's layout (Preferences › Aurora).
    pub photo_window: PhotoWindowStyle,
    /// The user's graphics values while the photo window's "Photo quality"
    /// raises them (`snapshot::quality`): never saved, they are what is
    /// written in their place.
    #[serde(skip)]
    pub photo_held: Option<crate::snapshot::quality::Held>,
    /// About Land as Firestorm's nine tabs (`ui/land`) instead of the
    /// overview and the settings grouped by intent (`ui/land/aurora`).
    pub classic_about_land: bool,
    /// Search as the Legacy Viewer's seven tabs (`ui/search.rs`) instead of
    /// one field, type chips and the details beside the results
    /// (`ui/search/aurora.rs`).
    pub classic_search: bool,
    /// Region / Estate as Firestorm's eight tabs (not ported yet: the
    /// Aurora window opens meanwhile) instead of Aurora's (`ui/region`).
    pub classic_region_estate: bool,
    /// The inventory as the Legacy Viewer's single tree with its tabs and
    /// footer instead of Aurora's window that widens from a compact tree to
    /// an explorer (`ui/inventory/aurora`).
    pub classic_inventory: bool,
    /// "Places" as the Legacy Viewer's three tabs (`ui/places.rs`) instead
    /// of Aurora's list and preview (`ui/places/aurora.rs`).
    pub classic_places: bool,
    /// The order of Aurora's "Places" lists.
    pub places_sort: PlacesSort,
    pub invert_mouse: bool,
    /// Aurora extension: allow the left/right mouse chord on land and objects
    /// without a click action, in addition to our avatar. Off by default.
    pub world_mouse_walk: bool,
    pub arrows_strafe: bool,
    /// The arrow keys (and Page Up / Down, Home) move the avatar while the
    /// chat bar has the keyboard, even with text in it; an empty bar always
    /// lets them through. The Legacy Viewer's `ArrowKeysAlwaysMove`, off by
    /// default (app_settings/settings.xml): with text in the bar the arrows
    /// move its cursor.
    pub arrow_keys_always_move: bool,
    /// Letter keys move the avatar (WASD) instead of starting the nearby
    /// chat. Firestorm's `LetterKeysAffectsMovementNotFocusChatBar`, on by
    /// default.
    pub letter_keys_move: bool,
    /// Avatars that walk backwards are drawn stepping back instead of
    /// turning around to face where they go (only on this screen). The
    /// Legacy Viewer's `FSDisableTurningAroundWhenWalkingBackwards`, off by
    /// default.
    pub walk_backwards_without_turning: bool,
    /// Hold Up to start flying (agent_jump, indra/newview/llviewerinput.cpp).
    /// Aurora splits AutomaticFly into independent takeoff/landing choices;
    /// both default to on.
    pub automatic_fly: bool,
    /// Hold Down on physical support to end flight (LLAgent::propagate,
    /// indra/newview/llagent.cpp), independently of automatic takeoff.
    pub automatic_land: bool,
    /// Always fly after a teleport (the Legacy Viewer's FSFlyAfterTeleport,
    /// off); a region that blocks flying still lands the avatar.
    pub fly_after_teleport: bool,
    /// The crouch key stays down until pressed again (FSCrouchToggle, off).
    pub crouch_toggle: bool,
    /// Quickjump: jump and land without waiting for the pre-jump and landing
    /// animations (FSIgnoreFinishAnimation, off; Me › Quickjump too).
    pub quickjump: bool,
    /// A left click on an object whose click action is Sit does not sit
    /// (FSBlockClickSit, off); the menu's Sit still does.
    pub block_click_sit: bool,
    /// What a double click on the ground does (Preferences › Controls, the
    /// wolf menu › Me): nothing by default, as in Firestorm.
    pub double_click: crate::autopilot::DoubleClick,
    pub follow: crate::autopilot::FollowSettings,
    /// Look-at: what we send of where we look, the crosshairs of where the
    /// avatars look, how our head follows the cursor (Preferences ›
    /// Privacy and Controls).
    pub lookat: crate::world::lookat::Settings,
    /// The former "Look-at tracking" switch and its range, read once to
    /// carry the user's choice into `lookat`.
    #[serde(rename = "look_at", skip_serializing)]
    pub(crate) former_look_at: Option<bool>,
    #[serde(rename = "look_at_range", skip_serializing)]
    pub(crate) former_look_at_range: Option<f32>,
    /// 0 off, 1 low, 2 medium, 3 high, 4 ultra
    pub shadow_quality: u8,
    /// The two projectors that weigh most on screen cast shadows
    /// (Firestorm RenderShadowDetail 2, "Sun/Moon + Projectors", its default).
    pub projector_shadows: bool,
    /// Screen-space contact shadows of the sun or the moon, beyond the
    /// Legacy Viewer (contact_shadows.rs): what the shadow map is too coarse
    /// to show, where things touch. Off by default; only with the shadows.
    pub contact_shadows: bool,
    /// 0 none, 1 FXAA, 2 MSAA 2x, 3 MSAA 4x, 4 MSAA 8x, 5 TAA, 6 SMAA (the
    /// default, RenderFSAAType 2 of Firestorm's High to Ultra levels in
    /// featuretable.txt)
    pub antialiasing: u8,
    /// Geometric specular anti-aliasing (common.wgsl `specular_aa_roughness`):
    /// highlights widened where the shading normal turns faster than a
    /// pixel, so they stop shimmering as the camera moves. An Aurora
    /// improvement, on by default.
    pub specular_aa: bool,
    /// 0 Khronos PBR Neutral, 1 ACES (RenderTonemapType: ACES by default,
    /// like the Legacy Viewer)
    pub tonemapper: u8,
    /// Share of the tone mapped color over the exposed linear one, on skies
    /// that are not classic (RenderTonemapMix, 0.7 like the Legacy Viewer).
    pub tonemap_mix: f32,
    /// "HDR and Emissive" (RenderDisableVintageMode, on by default, off in
    /// the Legacy Viewer's Low and LowMid levels): off, no exposure, tone
    /// mapping nor sharpening, no emissive on opaque PBR faces and 8-bit
    /// reflection probes (`RenderSettings::hdr`).
    pub hdr_emissive: bool,
    /// Optional frame rate cap (on by default at 120 images per second).
    pub fps_cap: bool,
    pub fps_limit: u32,
    /// Named graphics quality snapshots (#1218), independent of frame pacing.
    pub graphics_saved: std::collections::BTreeMap<String, crate::graphics_presets::saved::Saved>,
    /// The explicitly selected profile, retained while its quality is edited.
    pub graphics_saved_active: Option<String>,
    /// Lower cap while the window is not focused (on by default, as
    /// Firestorm yields BackgroundYieldTime every frame in the background).
    pub background_fps_cap: bool,
    pub background_fps_limit: u32,
    /// Maximum live particles (0 = particles off).
    pub max_particles: u32,
    /// Avatars drawn (the nearest ones).
    pub max_avatars: u32,
    /// Ambient occlusion: 0 off, 1 low, 2 high (the levels of the Legacy
    /// method).
    pub ssao: u8,
    /// How the ambient occlusion is computed: 0 Legacy (the Legacy Viewer's
    /// screen-space occlusion), 1 GTAO.
    pub ao_method: u8,
    /// GTAO quality: 0 low, 1 medium, 2 high, 3 ultra.
    pub gtao_quality: u8,
    /// Local visual water, independent of simulator physics.
    pub water_effects: WaterEffectsSettings,
    /// With GTAO: screen-space indirect light (a visibility bitmask and one
    /// bounce of light from the surfaces around). Aurora's own, off by
    /// default.
    pub screen_indirect_light: bool,
    /// Its strength, 0 to 4 (1 = one physical bounce).
    pub screen_indirect_intensity: f32,
    /// Water reflections: 0 off, 1 low, 2 medium, 3 high, 4 ultra.
    pub water_reflections: u8,
    /// Mirrors: 0 off, 1 low, 2 medium, 3 high, 4 ultra.
    pub mirrors: u8,
    /// Reflection probes (RenderReflectionProbeLevel): 0 none (sky only),
    /// 1 manual, 2 manual + terrain, 3 full scene.
    pub reflection_probes: u8,
    /// Reflection probe cube slots (RenderReflectionProbeCount; 8..=64).
    pub probe_count: u32,
    /// RenderAvatarMaxComplexity: avatars above it show as grey silhouettes
    /// (0 = no limit; Firestorm's Ultra preset is 350 000).
    pub max_complexity: u32,
    /// RenderAutoMuteSurfaceAreaLimit: attachments' surface area (m²) above
    /// which an avatar also shows as a silhouette (0 = no limit; ignored
    /// without a complexity limit, as in LL).
    pub max_attachment_area: f32,
    /// Avatars still loading: 0 particle cloud + progress bar under the name
    /// (Firestorm), 1 progress bar only (drawn as they load), 2 nothing.
    pub loading_avatars: u8,
    /// RenderAvatarComplexityMode: 0 limit everyone, 1 always show friends
    /// fully, 2 show only friends.
    pub complexity_mode: u8,
    /// AvatarPhysics: the bounce of the avatars that wear a physics layer
    /// (scene/body_physics.rs).
    pub avatar_physics: bool,
    /// RenderAvatarPhysicsLODFactor, 0 to 1: at 1 every avatar moves; below,
    /// avatars small on screen keep their shape; 0 stops the physics.
    pub avatar_physics_detail: f32,
    /// Per-avatar exceptions (LLRenderMuteList): avatar id -> 1 never render
    /// fully (silhouette), 2 always render fully.
    pub render_exceptions: Vec<(String, u8)>,
    /// Complexity in the name tags (FSTagShowARW, on), only for avatars over
    /// the limit (FSTagShowTooComplexOnlyARW, on), ours too (FSTagShowOwnARW, off).
    pub tag_complexity: bool,
    pub tag_complexity_too_complex_only: bool,
    pub tag_complexity_own: bool,
    /// Display names (UseDisplayNames, on), the username with them
    /// (NameTagShowUsernames, on) as "Firstname Lastname"
    /// (FSNameTagShowLegacyUsernames, on), "Resident" hidden
    /// (FSTrimLegacyNames, on), name changes told in the chat
    /// (FSShowDisplayNameUpdateNotification, on).
    pub use_display_names: bool,
    pub show_usernames: bool,
    /// Which group titles the name tags show (GroupTitlesTagMode, all).
    pub group_titles: crate::world::names::GroupTitles,
    /// Read the former separate switch once; the dropdown now owns this choice (#803).
    #[serde(rename = "hide_own_group_title", skip_serializing)]
    pub(crate) former_hide_own_group_title: bool,
    pub legacy_username_format: bool,
    pub trim_resident: bool,
    pub display_name_notices: bool,
    /// Screen-space reflections.
    pub ssr: bool,
    /// Anisotropic filtering: 1 (off), 2, 4, 8, 16.
    pub anisotropy: u16,
    /// Disk cache size (MB) for textures, meshes, animations, objects and inventory.
    pub cache_size_mb: u32,
    /// The folder the cache goes in, "" for the default one (the Legacy
    /// Viewer's CacheLocation): read at startup (cache_location.rs).
    pub cache_location: String,
    /// The folder holding the conversation transcripts, one folder per
    /// account inside, "" for the default one (InstantMessageLogPath).
    pub chat_log_location: String,
    /// The most the simulators may send, all channels together, in kb/s
    /// (Firestorm's ThrottleBandwidthKBPS; the default and the top of the
    /// range are Aurora's, `aurora_net::throttle::DEFAULT_KBPS`).
    pub bandwidth_kbps: u32,
    /// Connect only to the region we are in (the Legacy Viewer's
    /// FSDisableNeighbourRegionConnections, off): the neighbours are not
    /// shown, and crossings on foot or in vehicles are less reliable.
    pub no_neighbour_regions: bool,
    pub parcel_visibility: crate::world::parcel_visibility::Settings,
    /// Width of the local chat input in the bottom bar (px, 0 = skin default).
    pub chat_bar_width: f32,
    /// Width of the chat island of the bottom islands (px, 0 = the default
    /// of the screen's width, `ui::bars::dock::default_chat_width`).
    pub chat_island_width: f32,
    /// Shared toolbar, like the window layout and graphics preferences.
    /// None delays migration until the first account enters the world.
    #[serde(rename = "toolbar_common", default)]
    pub toolbar: Option<crate::ui::toolbar::ToolbarSettings>,
    /// Keep the earlier per-account layouts through saves made before login.
    #[serde(rename = "toolbar", default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub(crate) legacy_toolbars: std::collections::HashMap<uuid::Uuid, crate::ui::toolbar::ToolbarSettings>,
    /// Volumes and toggles (Firestorm volume panel).
    pub audio: AudioSettings,
    /// Parcel media and media on a prim.
    pub media: crate::media::MediaSettings,
    /// Name tag, minimap and chat colors (Firestorm color preferences).
    pub colors: ColorSettings,
    /// Rebindable keyboard / mouse controls.
    #[serde(default = "legacy_keybindings")]
    pub keybinds: crate::keybinds::KeyBindings,
    /// Read the former single-key setting once; new files use HudDrag's two slots.
    #[serde(rename = "hud_drag_key", skip_serializing)]
    pub(crate) legacy_hud_drag_key: Option<crate::keybinds::HoldKey>,
    /// Bumped when defaults change in a way that must reach existing settings
    /// (missing in older files: 0).
    #[serde(default)]
    pub settings_version: u32,
    /// The file as a newer Aurora wrote it, to write it back without losing
    /// what this build does not know (`tolerant::merge_over_newer`).
    #[serde(skip)]
    pub(crate) newer_file: Option<std::sync::Arc<NewerFile>>,
    /// No settings file existed: the graphics level is chosen for the
    /// graphics card once it is known (`graphics_presets::for_machine`).
    #[serde(skip)]
    pub new_profile: bool,
}

/// The default strength of GTAO's indirect light: at 1 (one physical
/// bounce) it hardly shows in the demo's coloured corner, at 2 a glow starts
/// around the foot of objects, at 4 it is grainy (captures of 2026-10-17).
pub(crate) const SCREEN_INDIRECT_INTENSITY: f32 = 1.5;

pub(crate) const DEFAULT_GROUND_FEET_HEIGHT: f32 = 0.4;
pub(crate) const DEFAULT_GROUND_FEET_AVATARS: u32 = 5;
pub(crate) const MAX_GROUND_FEET_AVATARS: u32 = 100;
pub(crate) const MAX_GROUND_FEET_HEIGHT: f32 = 1.0;

pub(crate) fn foot_height_limit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, MAX_GROUND_FEET_HEIGHT)
    } else {
        DEFAULT_GROUND_FEET_HEIGHT
    }
}

/// Limits of Preferences › Appearance › Text size, a multiplier of every
/// interface font. The Legacy Viewer stops at +2 points (FSFontSizeAdjustment,
/// panel_preferences_UI.xml, about 1.2×); Aurora goes up to twice the size for
/// visually impaired users (#1227), on top of the interface scale.
pub(crate) const FONT_SCALE_MIN: f32 = 0.8;
pub(crate) const FONT_SCALE_MAX: f32 = 2.0;

/// The default strength of the bloom (half of the light over its threshold).
pub(crate) const BLOOM_STRENGTH: f32 = 0.5;

/// RenderTonemapMix's default (settings.xml).
pub(crate) const TONEMAP_MIX: f32 = 0.7;

/// The longest group chat snooze, as the Legacy Viewer's (GroupSnoozeTime,
/// an S32 of seconds).
pub(crate) const GROUP_SNOOZE_MAX_MINUTES: u32 = i32::MAX as u32 / 60;

const SETTINGS_VERSION: u32 = 29;

/// `settings.json` from a newer build: the document, what this build loaded
/// from it, and the top-level keys to leave as they were unless changed here
/// (those this build could not read, and the version).
#[derive(Debug)]
pub(crate) struct NewerFile {
    theirs: serde_json::Value,
    loaded: serde_json::Value,
    kept: Vec<String>,
}

/// RGBA color (sRGB, straight alpha).
pub type Rgba = [u8; 4];

const fn hex(v: u32) -> Rgba {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]
}

/// Firestorm color preferences (colors.xml names in comments); defaults
/// follow the Aurora skin palette.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ColorSettings {
    // name tags
    /// Display names colored by who the avatar is to us (me, friend, Linden,
    /// blocked), on the plates and the classic tags; off, every name is in
    /// the theme's ink. On by default since 2026-10-28 (the human's choice:
    /// friends stand out in a crowd), as the Legacy Viewer always colors them
    /// (LLVOAvatar::getNameTagColor).
    pub relation_names: bool,
    pub tag_me: Rgba,
    pub tag_friend: Rgba,
    pub tag_muted: Rgba,
    pub tag_linden: Rgba,
    /// Display name equals the username (NameTagMatch).
    pub tag_match: Rgba,
    /// Custom display name (NameTagMismatch).
    pub tag_mismatch: Rgba,
    // minimap
    pub map_me: Rgba,
    pub map_other: Rgba,
    pub map_friend: Rgba,
    pub map_linden: Rgba,
    pub map_muted: Rgba,
    pub map_ranges: bool,
    pub map_whisper_on: bool,
    pub map_say_on: bool,
    pub map_shout_on: bool,
    pub map_whisper: Rgba,
    pub map_say: Rgba,
    pub map_shout: Rgba,
    // chat messages
    pub chat_mine: Rgba,
    pub chat_others: Rgba,
    pub chat_objects: Rgba,
    pub chat_friends: Rgba,
    pub chat_linden: Rgba,
    pub chat_muted: Rgba,
    pub chat_system: Rgba,
    /// Script errors (DEBUG_CHANNEL).
    pub chat_errors: Rgba,
    pub chat_object_im: Rgba,
    /// llOwnerSay.
    pub chat_owner: Rgba,
    pub chat_urls: Rgba,
    /// secondlife:/// links.
    pub chat_slurl: Rgba,
    /// llRegionSayTo.
    pub chat_direct: Rgba,
    // sender names
    pub sender_avatar: Rgba,
    pub sender_object: Rgba,
    // chat mentions
    pub mention_text: Rgba,
    pub mention_residents: Rgba,
    pub mention_me: Rgba,
}

impl Default for ColorSettings {
    fn default() -> Self {
        ColorSettings {
            relation_names: true,
            tag_me: hex(0xFFFFFF),
            tag_friend: hex(0x818CF8),
            tag_muted: hex(0x6F7BA0),
            tag_linden: hex(0xF472B6),
            tag_match: hex(0xE8EDFB),
            tag_mismatch: hex(0xE8EDFB),
            map_me: hex(0x8B5CF6),
            map_other: hex(0x4ADE80),
            map_friend: hex(0x818CF8),
            map_linden: hex(0xF472B6),
            map_muted: hex(0x6F7BA0),
            map_ranges: true,
            map_whisper_on: true,
            map_say_on: true,
            map_shout_on: true,
            map_whisper: hex(0x6CC24A),
            map_say: hex(0x6CC24A),
            map_shout: hex(0xE0661A),
            chat_mine: hex(0x9AA6C6),
            chat_others: hex(0xE8EDFB),
            chat_objects: hex(0x4ADE80),
            chat_friends: hex(0x818CF8),
            chat_linden: hex(0xF472B6),
            chat_muted: hex(0x6F7BA0),
            chat_system: hex(0x9AA6C6),
            chat_errors: hex(0xF87171),
            chat_object_im: hex(0xA78BFA),
            chat_owner: hex(0xFCD34D),
            chat_urls: hex(0x5EEAD4),
            // every clickable link of a text (web, place, avatar, group)
            // shares the brand teal
            chat_slurl: hex(0x5EEAD4),
            chat_direct: hex(0xFB923C),
            sender_avatar: hex(0xC4B5FD),
            sender_object: hex(0x4ADE80),
            mention_text: hex(0xFCD34D),
            mention_residents: hex(0x8B5CF6),
            mention_me: hex(0x4F46E5),
        }
    }
}

/// Audio channels in Firestorm's order: Main, Interface, Ambient, Sounds,
/// Music, Multimedia, Voice.
pub fn audio_channels() -> [&'static str; 7] {
    [
        aurora_i18n::tr!("settings-main"),
        aurora_i18n::tr!("common-interface"),
        aurora_i18n::tr!("common-ambient"),
        aurora_i18n::tr!("common-sounds"),
        aurora_i18n::tr!("common-music"),
        aurora_i18n::tr!("settings-multimedia"),
        aurora_i18n::tr!("common-voice"),
    ]
}

/// ShowStreamMetadata: 0 off, 1 toast, 2 nearby chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StreamTitleShow {
    Off,
    Toast,
    #[default]
    Chat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioSettings {
    pub explorer: crate::ui::sound_explorer::Filters,
    /// VoiceEarLocation, VoiceAutomaticGainControl, VoiceEchoCancellation and
    /// VoiceNoiseSuppressionLevel (panel_preferences_sound.xml).
    pub voice_ear: VoiceEar,
    pub voice_automatic_gain: bool,
    pub voice_echo_cancellation: bool,
    pub voice_noise_suppression: u8,
    pub lip_sync: bool,
    pub voice_visualizer: bool,
    pub voice_visualizer_self: bool,
    pub voice_visualizer_dot: bool,
    pub voice_visualizers_in_calls: bool,
    pub conversation_voice_indicator: bool,
    /// Volume 0..1 per channel (`AUDIO_CHANNELS`).
    pub volume: [f32; 7],
    /// The volume sliders follow a loudness curve instead of the gain
    /// (issue #463, off; `widgets::volume_slider`). The stored volumes stay
    /// linear gains either way.
    pub perceptual_volume: bool,
    pub muted: [bool; 7],
    /// Channel enabled (checkbox; the master has none).
    pub enabled: [bool; 7],
    /// Play the parcel music automatically when entering a parcel
    /// (FSParcelMusicAutoPlay, off by default: the radio toggle starts it).
    pub music_autoplay: bool,
    /// Play parcel media automatically.
    pub media_autoplay: bool,
    /// Where the parcel music's title is told when it changes (the Legacy
    /// Viewer's ShowStreamMetadata: nearby chat).
    pub stream_title: StreamTitleShow,
    /// Also say it on a script channel (StreamMetadataAnnounceToChat, off;
    /// StreamMetadataAnnounceChannel, 362394).
    pub stream_title_announce: bool,
    pub stream_title_channel: i32,
    /// World › Stream title shown, kept between sessions (the Legacy
    /// Viewer's fs_streamtitle saves its visibility; off).
    pub stream_title_window: bool,
    /// Aurora: the stream title as its text alone, laid over the world and
    /// moved anywhere, instead of the window (off).
    pub stream_title_text_only: bool,
    /// Titles kept in its history (FSStreamtitleHistoryLength, 10).
    pub stream_title_history: u32,
    /// Cut every sound while the window is minimised (MuteWhenMinimized, off).
    pub mute_when_minimized: bool,
    /// Output device name (empty = system default).
    pub output_device: String,
    /// Microphone name (empty = system default).
    pub input_device: String,
    /// Connect to voice chat.
    pub voice_enabled: bool,
    /// Microphone button of the bottom bar: false = toggle (click on / click
    /// off), true = hold to talk (Firestorm PushToTalkToggle inverted).
    pub mic_hold: bool,
    /// Microphone gain (0..2).
    pub mic_gain: f32,
    /// Collision sounds (EnableCollisionSounds, on).
    pub collision_sounds: bool,
    /// Sounds of other avatars' gestures (EnableGestureSounds, on).
    pub gesture_sounds: bool,
    /// Interface sounds (IM, money, teleport, windows, menus, typing...;
    /// Firestorm's PlayModeUISnd* defaults).
    pub ui_sounds: bool,
    /// Each interface sound: played or not, asset (UISnd* / PlayModeUISnd*).
    pub ui: crate::ui_sound::UiSoundSettings,
    /// Saved music stream URLs (FSStreamList "audio", FIRE-593), offered by
    /// the "Sound" tab of About Land.
    pub saved_streams: Vec<String>,
    /// Decline every incoming group / conference / resident voice call
    /// (VoiceCallsRejectGroup / AdHoc / P2P, off).
    pub reject_group_calls: bool,
    pub reject_adhoc_calls: bool,
    pub reject_p2p_calls: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings {
            explorer: Default::default(),
            voice_ear: VoiceEar::Camera,
            voice_automatic_gain: true,
            voice_echo_cancellation: true,
            voice_noise_suppression: 4,
            lip_sync: true,
            voice_visualizer: true,
            voice_visualizer_self: true,
            voice_visualizer_dot: true,
            voice_visualizers_in_calls: true,
            conversation_voice_indicator: true,
            volume: [0.8, 0.5, 0.5, 0.5, 0.4, 0.5, 0.7],
            perceptual_volume: false,
            muted: [false; 7],
            enabled: [true; 7],
            music_autoplay: false,
            media_autoplay: false,
            stream_title: StreamTitleShow::Chat,
            stream_title_announce: false,
            stream_title_channel: 362394,
            stream_title_window: false,
            stream_title_text_only: false,
            stream_title_history: 10,
            mute_when_minimized: false,
            output_device: String::new(),
            input_device: String::new(),
            voice_enabled: true,
            mic_hold: false,
            mic_gain: 1.0,
            collision_sounds: true,
            gesture_sounds: true,
            ui_sounds: true,
            ui: Default::default(),
            saved_streams: Vec::new(),
            reject_group_calls: false,
            reject_adhoc_calls: false,
            reject_p2p_calls: false,
        }
    }
}

/// VoiceEarLocation (panel_preferences_sound.xml); SL exposes two choices.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum VoiceEar {
    #[default]
    Camera,
    Avatar,
}

impl AudioSettings {
    pub fn world_voice_visualizer(&self, own: bool, in_call: bool, mouselook: bool) -> bool {
        self.voice_visualizer && (!in_call || self.voice_visualizers_in_calls) && (!own || (self.voice_visualizer_self && !mouselook))
    }
    pub fn mic_processing(&self) -> aurora_audio::MicProcessing {
        aurora_audio::MicProcessing {
            automatic_gain: self.voice_automatic_gain,
            echo_cancellation: self.voice_echo_cancellation,
            noise_suppression: self.voice_noise_suppression.min(4),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            update_mode: crate::updater::Mode::Ask,
            update_channel: aurora_updater::feed::Channel::Stable,
            ignored_update: String::new(),
            area_search: Default::default(),
            crash_reports: CrashReports::Ask,
            language: "default".into(),
            language_is_public: true,
            show_bug_report: true,
            show_huds: true,
            sun_beacon: false,
            moon_beacon: false,
            beacons: Default::default(),
            world_overlays: Default::default(),
            debug: DebugView::default(),
            dev_toolbox: None,
            build: Default::default(),
            camera: Default::default(),
            crosshair: Default::default(),
            joystick: Default::default(),
            auto_hover_height: false,
            auto_hover_offset: 0.0,
            hover_height_accounts: None,
            hover_height_account: None,
            contacts: Default::default(),
            radar: Default::default(),
            friends_list: Default::default(),
            inventory: Default::default(),
            texture_library: Default::default(),
            gestures: Default::default(),
            shape_show_hints: true,
            shape_height_feet: false,
            customize_animation: true,
            imports: Default::default(),
            script_editor: Default::default(),
            username: String::new(),
            remember_username: true,
            remember_password: false,
            grid: GridChoice::SecondLife,
            custom_login_uri: "http://127.0.0.1:9000/".into(),
            grids: Vec::new(),
            start_location: "last".into(),
            start_region: String::new(),
            draw_distance: 96.0,
            gpu_adapter: String::new(),
            gpu_driver_dismissed: String::new(),
            allow_overlays: false,
            vsync: false,
            shadows: true,
            force_midday: false,
            time_of_day: 0,
            env_manual_transition: 0.0,
            env_persist: true,
            exposure: 1.0,
            sharpen: 0.4,
            glow: true,
            bloom: false,
            bloom_strength: BLOOM_STRENGTH,
            occlusion_culling: true,
            lod_factor: 2.0,
            texture_budget_auto: true,
            texture_budget_mb: 1536,
            ui_scale: 1.0,
            use_pie_menu: false,
            world_context_menus: Default::default(),
            particle_filters: Default::default(),
            snap_windows: true,
            windows_stop_left: true,
            windows_stop_right: true,
            windows_stop_bottom: true,
            // First launch (no settings file): the radar (People on its
            // first tab, top left), the mini-map (top right) and
            // Conversations (bottom right) are open, as asked by the human;
            // the other windows wait for their button. A saved file keeps
            // what was open at exit, Performance aside (`show_perf`).
            show_perf: false,
            show_chat: true,
            show_minimap: true,
            standalone_place_details: false,
            inspect_columns: crate::ui::inspect::ALL_COLUMNS,
            landmarks_by_date: true,
            confirm_teleports: true,
            confirm_no_transfer_settings: true,
            confirm_object_bonus: true,
            confirm_return_to_owner: true,
            confirm_attachment_drop: true,
            confirm_script_modify: true,
            region_check_texture_heights: true,
            confirm_payments: true,
            payment_confirm_threshold: 200,
            always_track_payments: false,
            payment_opens_conversation: true,
            payment_sent_opens_conversation: false,
            maps: MapSettings::default(),
            search: Default::default(),
            show_people: true,
            people_tab: 0,
            show_inventory: false,
            show_appearance: false,
            appearance_fill_previews: false,
            show_ao: false,
            ao: Default::default(),
            bridge: Default::default(),
            movements: Default::default(),
            snapshot: Default::default(),
            dof: Default::default(),
            loading_backdrop: true,
            mouse_sensitivity: 1.0,
            window_width: 1600,
            window_height: 900,
            window_x: None,
            window_y: None,
            window_maximized: false,
            mfa_hash: String::new(),
            font: crate::ui::fonts::DEFAULT.into(),
            font_scale: 1.0,
            chat_timestamps: true,
            show_chat_channel: false,
            script_errors_in_chat: true,
            script_errors_to_channel: false,
            script_errors_channel: 1,
            compact_automated_chat: false,
            compact_im_system_lines: false,
            chat_keep_focus: true,
            chat_gesture_autocomplete: true,
            chat_name_prediction: true,
            colon_emote: false,
            chat_mention_autocomplete: true,
            autohide_chat_bar: false,
            show_contacts_tab: true,
            conversations_own_windows: false,
            send_typing_state: true,
            play_typing_anim: true,
            type_during_emote: false,
            typing_in_name_tags: true,
            typing_chevron: false,
            chat_bubbles: false,
            chat_bubbles_hide_console: true,
            typing_bubbles: false,
            chat_bubble_opacity: 0.5,
            flash_unread_conversations: true,
            flash_conversations_button: true,
            flash_conversations_nearby: true,
            chat_thumbnails: ChatThumbnails::ChatBar,
            chat_thumbnails_flash: true,
            flash_on_message: true,
            flash_on_object_im: true,
            flash_on_script_dialog: false,
            system_notifications: false,
            system_notifications_when_active: false,
            chat_toast_seconds: 20.0,
            toast_friends_online: true,
            friends_online_in_chat: false,
            presence_position: PresencePosition::TopRight,
            friend_request_prompt: true,
            region_message_prompt: true,
            toast_im: true,
            toast_group_im: false,
            ims_in_notification_center: false,
            im_toast_seconds: 30.0,
            log_im: true,
            log_nearby: false,
            reopen_ims: false,
            log_show_history: true,
            log_dated_names: false,
            warn_external_links: true,
            trusted_sites: Default::default(),
            mute_all_groups: false,
            mute_groups_without_notices: false,
            group_snooze_minutes: 15,
            former_group_snooze_seconds: None,
            group_snooze_ask: false,
            report_blocks: false,
            show_joined_group_invitations: false,
            script_count_alert: false,
            script_count_threshold: 100,
            sim_version_alert: false,
            object_im_filter: Default::default(),
            anti_spam: Default::default(),
            rlva_enabled: false,
            rlva: Default::default(),
            pose_requests: Default::default(),
            poser: Default::default(),
            autoresponse: Default::default(),
            chat_commands: Default::default(),
            keywords: Default::default(),
            spelling: Default::default(),
            auto_replace: false,
            name_tag_distance: 64.0,
            show_own_name_tag: true,
            show_friend_name_tags: true,
            show_other_name_tags: true,
            temporary_name_tags: false,
            name_tag_show_seconds: 10.0,
            classic_name_tags: false,
            classic_profiles: false,
            profile_rights: true,
            classic_pay_window: false,
            legacy_money_tracker: false,
            classic_bars: false,
            place_bar_stretch: true,
            place_bar_rights_text: false,
            favorites_access: FavoritesAccess::Menu,
            favorites_row_folded: false,
            system_island: SystemIsland::default(),
            confirm_hide_interface: true,
            immersive_name_tags: false,
            immersive_huds: true,
            show_net_stats: false,
            show_balance: true,
            classic_people: false,
            classic_restart_alert: false,
            classic_chat_lines: false,
            classic_rez_effect: false,
            hover_tips: HoverTips::default(),
            classic_hover_tips: false,
            predict_own_motion: false,
            synchronize_animations: false,
            sort_received_items: false,
            received_items_since: 0,
            notify_deliveries: true,
            show_accepted_in_inventory: false,
            ground_avatar_feet: false,
            ground_feet_while_moving: false,
            ground_feet_max_height: DEFAULT_GROUND_FEET_HEIGHT,
            ground_feet_max_avatars: DEFAULT_GROUND_FEET_AVATARS,
            region_restart: Default::default(),
            collisions: Default::default(),
            reduce_motion: false,
            color_blindness: crate::colorblind::Deficiency::Off,
            color_blindness_strength: 1.0,
            high_contrast: false,
            interface_colors: Default::default(),
            classic_controls: false,
            camera_shots: Vec::new(),
            plates_friends_only_full: false,
            plates_friend_tint: true,
            plates_hover_expand: true,
            plates_jiggle: false,
            plates_click_combo: false,
            plate_click: PlateClick::Nothing,
            plate_shift_click: PlateClick::Nothing,
            plate_ctrl_click: PlateClick::Nothing,
            chat_style: ChatStyle::Aurora,
            link_previews: Default::default(),
            gifs: Default::default(),
            notifications_style: NotificationsStyle::Aurora,
            world_map_style: WorldMapStyle::Aurora,
            photo_window: PhotoWindowStyle::Aurora,
            photo_held: None,
            classic_about_land: false,
            classic_search: false,
            classic_region_estate: false,
            classic_inventory: false,
            classic_places: false,
            places_sort: PlacesSort::Visit,
            invert_mouse: false,
            arrows_strafe: false,
            world_mouse_walk: false,
            arrow_keys_always_move: false,
            letter_keys_move: true,
            walk_backwards_without_turning: false,
            automatic_fly: true,
            automatic_land: true,
            fly_after_teleport: false,
            crouch_toggle: false,
            quickjump: false,
            block_click_sit: false,
            double_click: crate::autopilot::DoubleClick::Nothing,
            follow: Default::default(),
            lookat: Default::default(),
            former_look_at: None,
            former_look_at_range: None,
            shadow_quality: 3,
            projector_shadows: true,
            contact_shadows: false,
            antialiasing: 6,
            specular_aa: true,
            tonemapper: 1,
            tonemap_mix: TONEMAP_MIX,
            hdr_emissive: true,
            fps_cap: true,
            fps_limit: 120,
            graphics_saved: Default::default(),
            graphics_saved_active: None,
            background_fps_cap: true,
            background_fps_limit: 15,
            max_particles: 4096,
            max_avatars: 16,
            ssao: 1,
            ao_method: 0,
            gtao_quality: 2,
            water_effects: WaterEffectsSettings::default(),
            screen_indirect_light: false,
            screen_indirect_intensity: SCREEN_INDIRECT_INTENSITY,
            water_reflections: 1,
            reflection_probes: 3,
            probe_count: 32,
            max_complexity: 350_000,
            max_attachment_area: 1000.0,
            loading_avatars: 0,
            complexity_mode: 0,
            avatar_physics: true,
            avatar_physics_detail: 1.0,
            render_exceptions: Vec::new(),
            tag_complexity: true,
            tag_complexity_too_complex_only: true,
            tag_complexity_own: false,
            use_display_names: true,
            show_usernames: true,
            group_titles: Default::default(),
            former_hide_own_group_title: false,
            legacy_username_format: true,
            trim_resident: true,
            display_name_notices: true,
            mirrors: 1,
            ssr: false,
            anisotropy: 8,
            cache_size_mb: 20480,
            cache_location: String::new(),
            chat_log_location: String::new(),
            bandwidth_kbps: aurora_net::throttle::DEFAULT_KBPS as u32,
            no_neighbour_regions: false,
            parcel_visibility: Default::default(),
            chat_bar_width: 0.0,
            chat_island_width: 0.0,
            toolbar: Default::default(),
            legacy_toolbars: Default::default(),
            audio: AudioSettings::default(),
            media: Default::default(),
            colors: ColorSettings::default(),
            keybinds: Default::default(),
            legacy_hud_drag_key: None,
            settings_version: SETTINGS_VERSION,
            newer_file: None,
            new_profile: false,
        }
    }
}

/// A file without a shortcut table predates HudDrag. Leave it missing until
/// migration can copy the saved legacy choice instead of an invented default.
fn legacy_keybindings() -> crate::keybinds::KeyBindings {
    let mut kb = crate::keybinds::KeyBindings::default();
    kb.map.retain(|(a, _)| *a != crate::keybinds::Action::HudDrag);
    kb
}

/// Automatic texture memory (MB) from the detected video memory: 70 % of
/// it, at least 1.5 GB left to the rest, never under 768 MB. Integrated
/// GPUs share the system memory: 2 GB; unknown: 1.5 GB.
pub fn auto_texture_budget(vram_mb: Option<u64>, integrated: bool) -> u64 {
    match vram_mb {
        Some(v) if !integrated && v >= 1024 => (v * 7 / 10).min(v.saturating_sub(1536)).max(768),
        _ if integrated => 2048,
        _ => 1536,
    }
}

/// The offline demo keeps its own settings and cache (a `demo` folder inside
/// each), so test runs never change the user's real ones.
/// `allow_overlays` of a settings file's bytes; false for anything else.
fn allow_overlays_in(file: Option<&[u8]>) -> bool {
    file.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .and_then(|value| value.get("allow_overlays").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn for_mode(dir: PathBuf) -> PathBuf {
    if std::env::var_os("AURORA_DEMO").is_some() {
        dir.join("demo")
    } else {
        dir
    }
}

/// Unit tests never read or write the user's folders: theirs are in the
/// build folder of the worktree.
fn unit_test_root() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/unit-test-user-data"))
    }
    #[cfg(not(test))]
    {
        None
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(root) = unit_test_root() {
        return root.join("config");
    }
    if let Some(test) = aurora_updater::test_environment() {
        return test.root.join("profile");
    }
    for_mode(
        directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    )
}

/// Files of one account (LL_PATH_PER_SL_ACCOUNT): contact sets, pinned
/// groups.
pub fn account_dir(agent: &uuid::Uuid) -> PathBuf {
    accounts_dir().join(agent.to_string())
}

/// The folder that holds one folder per account id. An account id says
/// nothing of the grid: what must not cross from one grid to another (the
/// conversation transcripts) adds the grid under it.
pub fn accounts_dir() -> PathBuf {
    config_dir().join("accounts")
}

pub fn cache_dir() -> PathBuf {
    if let Some(root) = unit_test_root() {
        return root.join("cache");
    }
    if let Some(test) = aurora_updater::test_environment() {
        return test.root.join("cache");
    }
    // the folder chosen in the preferences, read at startup (cache_location.rs)
    if let Some(chosen) = crate::cache_location::chosen() {
        return for_mode(chosen.to_path_buf());
    }
    for_mode(default_cache_root())
}

pub(crate) fn default_cache_root() -> PathBuf {
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("cache"))
}

/// The cache folders the viewer uses when none is chosen (before the demo's
/// subfolder): the ones a change of folder may remove.
pub fn default_cache_dirs() -> Vec<PathBuf> {
    vec![default_cache_root()]
}

/// Where Aurora's media plugins pack is installed: next to the logs, outside
/// the cache (it survives a cache clear and the viewer's updates) and shared
/// by the offline demo and the real sessions, several hundred MB being
/// downloaded once.
pub fn media_plugins_dir() -> PathBuf {
    if let Some(root) = unit_test_root() {
        return root.join("media-plugins");
    }
    if let Some(test) = aurora_updater::test_environment() {
        return test.root.join("media-plugins");
    }
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
        .map(|d| d.data_local_dir().join("media-plugins"))
        .unwrap_or_else(|| PathBuf::from("media-plugins"))
}

/// What a settings file says of the viewer window: (its maximized state,
/// whether it gives a size).
fn saved_window_state(file: &serde_json::Value) -> (Option<bool>, bool) {
    let maximized = file.get("window_maximized").and_then(serde_json::Value::as_bool);
    let size = file.get("window_width").is_some() || file.get("window_height").is_some();
    (maximized, size)
}

/// Whether the viewer window opens maximized. The state saved at the last
/// exit when there is one: the user who restored the window keeps its size,
/// the one who left it maximized finds it maximized. Without it: maximized
/// on a first launch (no settings file, or one that says nothing of the
/// window), but not when the file gives a size and no state, a size chosen
/// by a script or a test. Never when the size is forced for the run
/// (`forced`: a capture, which needs its exact window).
pub(crate) fn window_starts_maximized(saved_maximized: Option<bool>, saved_size: bool, forced: bool) -> bool {
    !forced && saved_maximized.unwrap_or(!saved_size)
}

impl Settings {
    /// Rendering exception of an avatar: 0 none, 1 never render fully,
    /// 2 always render fully.
    pub fn render_exception(&self, id: &uuid::Uuid) -> u8 {
        let s = id.to_string();
        self.render_exceptions.iter().find(|(x, _)| *x == s).map_or(0, |(_, m)| *m)
    }

    pub fn set_render_exception(&mut self, id: uuid::Uuid, mode: u8) {
        let s = id.to_string();
        self.render_exceptions.retain(|(x, _)| *x != s);
        if (1..=2).contains(&mode) {
            self.render_exceptions.push((s, mode));
        }
    }

    pub fn render_exceptions_map(&self) -> std::collections::HashMap<uuid::Uuid, u8> {
        self.render_exceptions
            .iter()
            .filter_map(|(id, m)| Some((uuid::Uuid::parse_str(id).ok()?, *m)))
            .collect()
    }

    pub(crate) fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Settings {
        Self::load_from(&Self::path())
    }

    /// Whether the overlays are allowed, before anything else of the viewer
    /// starts (the Vulkan layers are chosen on `main`'s first lines, before
    /// any thread): this one field of the file, no log, no thread; false
    /// when the file or the field is missing or unreadable.
    pub fn allow_overlays_at_start() -> bool {
        allow_overlays_in(std::fs::read(Self::path()).ok().as_deref())
    }

    fn load_from(path: &Path) -> Settings {
        // what the file says of the window: nothing on a first launch
        let mut saved_window = (None, false);
        let mut newer = None;
        let file = std::fs::read(path).ok();
        let new_profile = file.is_none();
        let mut s = file
            .and_then(|b| {
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&b) else {
                    // rewritten with the defaults at the next save: keep it
                    log::warn!("settings.json is not readable JSON, defaults used");
                    crate::tolerant::backup(path);
                    return None;
                };
                saved_window = saved_window_state(&value);
                // Select before serde fills missing fields: default auto-replies
                // must use the saved locale too, without replacing custom text.
                aurora_i18n::select(
                    value["language"].as_str().unwrap_or("default"),
                    crate::cli::get().language.as_deref(),
                );
                let (s, from_newer) = Self::read_document(value, path)?;
                newer = from_newer;
                Some(s)
            })
            .unwrap_or_default();
        let forced = std::env::var_os("AURORA_CAPTURE").is_some();
        s.window_maximized = window_starts_maximized(saved_window.0, saved_window.1, forced);
        s.new_profile = new_profile;
        s.finished(newer)
    }

    /// Reads the parsed file, leaving out only what this build does not
    /// understand; for a newer build's file, also what to keep of it on save.
    fn read_document(value: serde_json::Value, path: &Path) -> Option<(Settings, Option<(serde_json::Value, Vec<String>)>)> {
        let version = value["settings_version"].as_u64().unwrap_or(0);
        let Some(read) = crate::tolerant::from_value::<Settings>(value.clone(), "settings.json") else {
            crate::tolerant::backup(path);
            return None;
        };
        let mut newer = None;
        if version > SETTINGS_VERSION as u64 {
            // No migration runs backwards: `sanitized` only migrates older
            // versions, and the newer number is written back.
            log::info!(
                "settings.json comes from a newer Aurora (version {version}, this build {SETTINGS_VERSION}): \
                 read without migration, its unknown values kept on save"
            );
            let mut kept: Vec<String> = read.dropped.iter().map(|p| crate::tolerant::top_key(p).to_owned()).collect();
            kept.push("settings_version".into());
            kept.sort();
            kept.dedup();
            newer = Some((value, kept));
        }
        if read.repaired() || newer.is_some() {
            crate::tolerant::backup(path);
        }
        Some((read.value, newer))
    }

    /// Migrated and bounded, remembering a newer build's file to save over.
    fn finished(self, newer: Option<(serde_json::Value, Vec<String>)>) -> Settings {
        let mut s = self.sanitized();
        if let Some((theirs, kept)) = newer
            && let Ok(loaded) = serde_json::to_value(&s)
        {
            s.newer_file = Some(std::sync::Arc::new(NewerFile { theirs, loaded, kept }));
        }
        s
    }

    pub(crate) fn sanitized(mut self) -> Settings {
        if self
            .graphics_saved_active
            .as_ref()
            .is_some_and(|name| !self.graphics_saved.contains_key(name))
        {
            self.graphics_saved_active = None;
        }
        for profile in self.world_context_menus.profiles.values_mut() {
            profile.locked = true;
        }
        self.grids.retain_mut(|grid| grid.validate().is_ok());
        if std::mem::take(&mut self.former_hide_own_group_title) {
            self.group_titles = crate::world::names::GroupTitles::AllExceptMine;
        }
        // the shutter's switch of the interface sounds became the photos'
        // own ("Quiet snapshots"): a shutter turned off there stays off
        if self.audio.ui.play.remove("Snapshot") == Some(false) {
            self.snapshot.sound = false;
        }
        if self.settings_version < 2 {
            // new defaults: 96 m draw distance, LOD factor 2
            self.draw_distance = 96.0;
            self.lod_factor = 2.0;
        }
        if self.settings_version < 3 && self.font == "Inter" {
            // Noto Sans became the default (consistent with the Noto emoji)
            self.font = "Noto Sans".into();
        }
        if self.force_midday {
            // former "always noon" switch
            self.time_of_day = 2;
            self.force_midday = false;
        }
        if let Some(key) = self.legacy_hud_drag_key.take()
            && !self.keybinds.map.iter().any(|(a, _)| *a == crate::keybinds::Action::HudDrag)
        {
            self.keybinds.set(crate::keybinds::Action::HudDrag, 0, Some(key.into_binding()));
        }
        self.keybinds = std::mem::take(&mut self.keybinds).sanitized();
        if self.settings_version < 10
            && self.keybinds.get(crate::keybinds::Action::HudDrag) == [Some(crate::keybinds::HoldKey::Alt.into_binding()), None]
        {
            // Replace the former HUD default without changing custom or cleared slots (#266).
            self.keybinds.set(
                crate::keybinds::Action::HudDrag,
                0,
                Some(crate::keybinds::HoldKey::Ctrl.into_binding()),
            );
        }
        if self.former_look_at.is_some() || self.former_look_at_range.is_some() {
            self.lookat = crate::world::lookat::Settings::from_former(
                self.former_look_at.take(),
                self.former_look_at_range.take(),
                FORMER_LOOK_AT_UNLIMITED,
            );
        }
        self.lookat = self.lookat.sanitized();
        if let Some(seconds) = self.former_group_snooze_seconds.take() {
            // rounded up: a short snooze stays a snooze, not "until the next message"
            self.group_snooze_minutes = seconds.div_ceil(60);
        }
        self.group_snooze_minutes = self.group_snooze_minutes.min(GROUP_SNOOZE_MAX_MINUTES);
        self.ground_feet_max_height = foot_height_limit(self.ground_feet_max_height);
        self.ground_feet_max_avatars = self.ground_feet_max_avatars.min(MAX_GROUND_FEET_AVATARS);
        self.camera = std::mem::take(&mut self.camera).sanitized();
        crate::camera::presets::sanitize(&mut self.camera_shots);
        if self.settings_version < 5 {
            // new defaults: no vsync, no frame cap (the old "0 = screen rate,
            // 1 = unlimited" setting becomes a switch + value)
            self.vsync = false;
            self.fps_cap = self.fps_limit >= 2;
            if !self.fps_cap {
                self.fps_limit = 120;
            }
        }
        self.fps_limit = self.fps_limit.clamp(10, 500);
        self.background_fps_limit = self.background_fps_limit.clamp(1, 120);
        if self.settings_version < 6 {
            // new defaults: texture memory from the detected VRAM, 20 GB disk cache
            self.texture_budget_auto = true;
            if self.cache_size_mb == 4096 {
                self.cache_size_mb = 20480;
            }
        }
        if self.settings_version < 7 {
            // new default: parcel music waits for the radio toggle (Firestorm)
            self.audio.music_autoplay = false;
        }
        if self.settings_version < 8 && self.colors.chat_slurl == hex(0x6F7BA0) {
            // new default: SL links (places, avatars, groups) in the link teal
            self.colors.chat_slurl = ColorSettings::default().chat_slurl;
        }
        if self.settings_version < 9 {
            // new default: the background cap is on (a window left behind no
            // longer keeps the GPU and the desktop busy)
            self.background_fps_cap = true;
        }
        if self.settings_version < 11 {
            // the names were colored by relation on the classic tags only:
            // whoever used them keeps their colors, the plates turn white
            self.colors.relation_names = self.classic_name_tags;
        }
        if self.settings_version < 4 {
            // walking: arrow keys only (no more WASD / ZQSD by default)
            self.keybinds.drop_letter_walk();
        }
        if self.settings_version < 12 {
            // letter shortcuts follow the layout (M is the M key on AZERTY)
            self.keybinds.follow_layout_letters();
        }
        if self.settings_version < 13 && self.chat_style == ChatStyle::Bubbles {
            // the bubbles were the default every file wrote: Aurora's
            // workspace takes their place; the Firestorm lines, a choice, stay
            self.chat_style = ChatStyle::Aurora;
        }
        if self.settings_version < 14 {
            // new defaults: unrestricted camera, no camera collisions (every
            // file wrote the old "off" defaults)
            self.camera.disable_constraints = true;
            self.camera.ignore_sim_constraints = true;
        }
        if self.settings_version < 15 {
            // new default: Contacts shown in Conversations (every file wrote
            // the old "hidden" default)
            self.show_contacts_tab = true;
        }
        if self.settings_version < 16 && self.font == crate::ui::fonts::NOTO {
            // Inter is the default interface font again: every file wrote
            // the former default, Noto Sans (version 3 had moved Inter to it)
            self.font = crate::ui::fonts::DEFAULT.into();
        }
        if self.settings_version < 17 {
            // new default: the chat bar keeps the keyboard after sending
            // (every file wrote the old "off")
            self.chat_keep_focus = true;
        }
        if self.settings_version < 18 {
            // Deluxe now takes GTAO: a profile still exactly on the former
            // Deluxe follows it, any other keeps its occlusion
            crate::graphics_presets::upgrade_deluxe(&mut self);
        }
        if self.settings_version < 19 {
            // Deluxe now also takes the bloom, the contact shadows and the
            // indirect light: the same rule, a profile still exactly on the
            // Deluxe of version 18 follows it
            crate::graphics_presets::upgrade_deluxe_effects(&mut self);
        }
        if self.settings_version < 20 {
            // the bloom and the indirect light are experimental and left
            // Deluxe, and High takes GTAO at Medium quality: a profile still
            // exactly on the Deluxe or the High of version 19 follows it,
            // any other keeps its values
            crate::graphics_presets::upgrade_deluxe_experimental(&mut self);
            crate::graphics_presets::upgrade_high_gtao(&mut self);
        }
        if self.settings_version < 21 {
            // the levels of 2026-10-17 (GTAO from Medium up, contact shadows
            // in High and Deluxe, indirect light and 192 m in Deluxe): a
            // profile still exactly on the Medium, High or Deluxe of version
            // 20 follows it, any other keeps its values
            crate::graphics_presets::upgrade_levels_v21(&mut self);
        }
        if self.settings_version < 22 {
            // "Arrow keys always move me" was on by default, unlike the
            // Legacy Viewer: with text in the chat bar the arrows turned the
            // avatar instead of moving the cursor. Every earlier file holds
            // that default, so it goes off once
            self.arrow_keys_always_move = false;
        }
        if self.settings_version < 23 {
            // new default: the rez effect instead of grey objects (every
            // earlier file wrote the old grey default)
            self.classic_rez_effect = false;
        }
        if self.settings_version < 24 {
            // the levels now set the avatar physics detail: a profile still
            // exactly on Potato or Low takes theirs, any other keeps 1
            crate::graphics_presets::upgrade_physics_detail(&mut self);
        }
        if self.settings_version < 25 {
            // ACES is the default tone mapper, as RenderTonemapType in the
            // Legacy Viewer: every earlier file wrote the former default,
            // Khronos PBR Neutral, which looked the same while skies were
            // not tone mapped (no automatic exposure nor mix either)
            self.tonemapper = 1;
        }
        if self.settings_version < 26 {
            // `show_perf` held Performance's state at exit (open on a first
            // launch): now a choice, off, the window waits for the frame
            // rate's click
            self.show_perf = false;
        }
        if self.settings_version < 27 {
            // names colored by relation are the default again: every earlier
            // file wrote the former default, off
            self.colors.relation_names = true;
        }
        if self.settings_version < 28 {
            // the levels now set HDR and Emissive: a profile still exactly
            // on Potato or Low turns it off, any other keeps it on
            crate::graphics_presets::upgrade_hdr_emissive(&mut self);
        }
        if self.settings_version < 29 {
            // new default: no simulator version notice (every earlier file
            // wrote the former default, on)
            self.sim_version_alert = false;
        }
        if !self.audio.mic_gain.is_finite() {
            self.audio.mic_gain = 1.0;
        }
        self.audio.mic_gain = self.audio.mic_gain.clamp(0.0, 2.0);
        self.time_of_day = self.time_of_day.min(crate::world::env::TIME_OF_DAY_PRESETS);
        self.env_manual_transition = if self.env_manual_transition.is_finite() {
            self.env_manual_transition.clamp(0.0, 60.0)
        } else {
            0.0
        };
        self.settings_version = SETTINGS_VERSION;
        self.max_particles = self.max_particles.min(8192);
        self.max_avatars = self.max_avatars.clamp(1, 100);
        self.complexity_mode = self.complexity_mode.min(2);
        self.avatar_physics_detail = if self.avatar_physics_detail.is_finite() {
            self.avatar_physics_detail.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.loading_avatars = self.loading_avatars.min(2);
        self.max_attachment_area = if self.max_attachment_area.is_finite() {
            self.max_attachment_area.clamp(0.0, 100_000.0)
        } else {
            1000.0
        };
        self.render_exceptions
            .retain(|(id, m)| uuid::Uuid::parse_str(id).is_ok() && (1..=2).contains(m));
        self.ssao = self.ssao.min(2);
        self.ao_method = self.ao_method.min(1);
        self.gtao_quality = self.gtao_quality.min(3);
        self.water_reflections = self.water_reflections.min(4);
        self.mirrors = self.mirrors.min(4);
        self.anisotropy = match self.anisotropy {
            0 | 1 => 1,
            2 | 3 => 2,
            4..=7 => 4,
            8..=15 => 8,
            _ => 16,
        };
        self.cache_size_mb = self.cache_size_mb.clamp(512, 65536);
        self.bandwidth_kbps = self.bandwidth_kbps.clamp(50, aurora_net::throttle::DEFAULT_KBPS as u32);
        if !self.chat_bar_width.is_finite() || self.chat_bar_width < 0.0 {
            self.chat_bar_width = 0.0;
        }
        if !self.chat_island_width.is_finite() || self.chat_island_width < 0.0 {
            self.chat_island_width = 0.0;
        }
        self.draw_distance = self.draw_distance.clamp(32.0, 512.0);
        self.chat_commands.sanitize();
        if let Some(toolbar) = &mut self.toolbar {
            toolbar.sanitize();
        }
        for toolbar in self.legacy_toolbars.values_mut() {
            toolbar.sanitize();
        }
        self.snapshot.sanitize();
        self.dof.sanitize();
        self.exposure = self.exposure.clamp(0.25, 4.0);
        self.screen_indirect_intensity = if self.screen_indirect_intensity.is_finite() {
            self.screen_indirect_intensity.clamp(0.0, 4.0)
        } else {
            SCREEN_INDIRECT_INTENSITY
        };
        self.sharpen = if self.sharpen.is_finite() {
            self.sharpen.clamp(0.0, 1.0)
        } else {
            0.4
        };
        self.bloom_strength = if self.bloom_strength.is_finite() {
            self.bloom_strength.clamp(0.05, 1.0)
        } else {
            BLOOM_STRENGTH
        };
        self.lod_factor = self.lod_factor.clamp(0.5, 4.0);
        self.ui_scale = self.ui_scale.clamp(0.75, 2.0);
        self.mouse_sensitivity = self.mouse_sensitivity.clamp(0.1, 5.0);
        self.texture_budget_mb = self.texture_budget_mb.clamp(256, 32768);
        self.font_scale = self.font_scale.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX);
        self.hover_tips.delay = if self.hover_tips.delay.is_finite() {
            self.hover_tips.delay.clamp(0.0, MAX_HOVER_TIP_DELAY)
        } else {
            HoverTips::default().delay
        };
        self.shadow_quality = self.shadow_quality.min(4);
        self.antialiasing = self.antialiasing.min(6);
        self.reflection_probes = self.reflection_probes.min(3);
        self.probe_count = self.probe_count.clamp(8, 64);
        self.tonemapper = self.tonemapper.min(1);
        self.tonemap_mix = if self.tonemap_mix.is_finite() {
            self.tonemap_mix.clamp(0.0, 1.0)
        } else {
            TONEMAP_MIX
        };
        self.chat_toast_seconds = self.chat_toast_seconds.clamp(0.0, 60.0);
        self.chat_bubble_opacity = if self.chat_bubble_opacity.is_finite() {
            self.chat_bubble_opacity.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.im_toast_seconds = self.im_toast_seconds.clamp(3.0, 60.0);
        self.name_tag_distance = self.name_tag_distance.clamp(0.0, 128.0);
        self.name_tag_show_seconds = if self.name_tag_show_seconds.is_finite() {
            self.name_tag_show_seconds.clamp(1.0, 60.0)
        } else {
            10.0
        };
        self
    }

    /// Deliberate difference from Firestorm's per-account toolbars.xml, at the
    /// human's request: every account uses the same toolbar. Resolve the old
    /// layout only once, after login, to retain the account being tested.
    pub fn migrate_toolbar(&mut self, account: uuid::Uuid) -> bool {
        let mut changed = false;
        if self.toolbar.is_none() {
            self.toolbar = Some(self.legacy_toolbars.remove(&account).unwrap_or_default());
            changed = true;
        }
        if !self.legacy_toolbars.is_empty() {
            self.legacy_toolbars.clear();
            changed = true;
        }
        changed
    }

    /// Back to the defaults ("Reset everything"), keeping the account and
    /// grid used to log in, the start location and the window size.
    pub fn reset_keeping_account(&mut self) {
        let old = std::mem::take(self);
        *self = Settings {
            username: old.username,
            remember_username: old.remember_username,
            grid: old.grid,
            custom_login_uri: old.custom_login_uri,
            grids: old.grids,
            start_location: old.start_location,
            start_region: old.start_region,
            window_width: old.window_width,
            window_height: old.window_height,
            window_x: old.window_x,
            window_y: old.window_y,
            window_maximized: old.window_maximized,
            // a newer build's own values are not this build's to reset
            newer_file: old.newer_file,
            // Named profiles are user content, like separate preset files.
            graphics_saved: old.graphics_saved,
            // Reset this avatar's hover options while keeping other accounts.
            hover_height_accounts: old.hover_height_accounts,
            hover_height_account: old.hover_height_account,
            ..Settings::default()
        };
    }

    pub fn save(&self) {
        let dir = config_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("cannot create config dir: {e}");
            return;
        }
        self.save_to(&Self::path());
    }

    /// What is written to the file: the user's own graphics values while
    /// the photo quality raises them, no user name unless remembered.
    pub(crate) fn to_saved(&self) -> Settings {
        let mut copy = self.clone();
        copy.remember_hover_account();
        if let Some(mut held) = copy.photo_held.take() {
            // a setting changed since the last frame is the user's too
            held.sync(&copy);
            held.restore(&mut copy);
        }
        if !copy.remember_username {
            copy.username.clear();
        }
        copy
    }

    fn save_to(&self, path: &Path) {
        let copy = self.to_saved();
        // Over a newer build's file, write ours over theirs: what this build
        // does not know, and what it could not read and left unchanged, stay.
        let bytes = match &copy.newer_file {
            Some(newer) => serde_json::to_value(&copy)
                .map(|ours| crate::tolerant::merge_over_newer(&newer.theirs, ours, &newer.loaded, &newer.kept))
                .and_then(|merged| serde_json::to_vec_pretty(&merged)),
            None => serde_json::to_vec_pretty(&copy),
        };
        match bytes {
            Ok(b) => {
                if let Err(e) = crate::atomic_file::write(path, &b) {
                    log::warn!("cannot save settings: {e}");
                }
            }
            Err(e) => log::warn!("cannot serialize settings: {e}"),
        }
    }

    pub fn login_uri(&self) -> String {
        match self.grid {
            GridChoice::SecondLife => aurora_net::login::SL_MAIN_GRID.into(),
            GridChoice::SecondLifeBeta => aurora_net::login::SL_BETA_GRID.into(),
            GridChoice::Custom => self.custom_login_uri.trim().to_owned(),
        }
    }

    /// Texture memory budget (MB): the manual value, or in automatic mode a
    /// share of the detected video memory (as Firestorm's unlimited VRAM
    /// mode: leave room for render targets, geometry and other programs).
    pub fn texture_budget(&self, vram_mb: Option<u64>, integrated: bool) -> u64 {
        if !self.texture_budget_auto {
            return self.texture_budget_mb as u64;
        }
        auto_texture_budget(vram_mb, integrated)
    }

    /// (resolution, distance) for the shadow quality step.
    pub fn shadow_params(&self) -> (u32, f32) {
        match self.shadow_quality {
            0 => (0, 0.0),
            1 => (1024, 48.0),
            2 => (2048, 96.0),
            3 => (2048, 192.0),
            _ => (4096, 256.0),
        }
    }

    pub fn render_settings(&self) -> aurora_render::RenderSettings {
        use aurora_render::{AntiAliasing, AoMethod, Tonemapper};
        let mut water_effects = self.water_effects.render();
        water_effects.particle_budget = self.max_particles;
        aurora_render::RenderSettings {
            water_effects,
            aa: match self.antialiasing {
                0 => AntiAliasing::None,
                1 => AntiAliasing::Fxaa,
                2 => AntiAliasing::Msaa(2),
                3 => AntiAliasing::Msaa(4),
                4 => AntiAliasing::Msaa(8),
                5 => AntiAliasing::Taa,
                _ => AntiAliasing::Smaa,
            },
            tonemapper: if self.tonemapper == 1 {
                Tonemapper::Aces
            } else {
                Tonemapper::KhronosNeutral
            },
            hdr: self.hdr_emissive,
            shadow_resolution: self.shadow_params().0,
            ssao: self.ssao,
            ao_method: if self.ao_method == 1 {
                AoMethod::Gtao {
                    quality: self.gtao_quality,
                }
            } else {
                AoMethod::Legacy
            },
            screen_indirect: self.ao_method == 1 && self.screen_indirect_light,
            screen_indirect_intensity: self.screen_indirect_intensity,
            contact_shadows: self.contact_shadows,
            specular_aa: self.specular_aa,
            water_reflection_scale: Self::reflection_scale(self.water_reflections),
            mirror_scale: Self::reflection_scale(self.mirrors),
            probe_slots: self.probe_count.clamp(8, 64),
            ssr: self.ssr,
            anisotropy: self.anisotropy,
            occlusion: self.occlusion_culling,
        }
    }

    /// Fraction of the screen resolution for a reflection quality step.
    pub fn reflection_scale(step: u8) -> f32 {
        match step {
            0 => 0.0,
            1 => 0.25,
            2 => 0.5,
            3 => 0.75,
            _ => 1.0,
        }
    }
}

/// Mini-map (LLNetMap) and world map (LLFloaterWorldMap) options, with the
/// Firestorm defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MapSettings {
    /// MiniMapScale: pixels per region, 32..4096.
    pub mini_scale: f32,
    /// MiniMapRotate: camera direction at the top (else north).
    pub mini_rotate: bool,
    /// MiniMapObjects.
    pub mini_objects: bool,
    /// FSNetMapPhysical / FSNetMapScripted / FSNetMapTempOnRez.
    pub mini_physical: bool,
    pub mini_scripted: bool,
    pub mini_temp_on_rez: bool,
    /// MiniMapShowPropertyLines / MiniMapForSaleParcels.
    pub mini_property_lines: bool,
    pub mini_for_sale: bool,
    /// MiniMapCollisionParcels: parcels we may not enter, filled in red.
    pub mini_collision: bool,
    /// MiniMapAutoCenter.
    pub mini_auto_center: bool,
    /// FSNetMapDoubleClickAction: 0 nothing, 1 world map, 2 teleport.
    pub mini_double_click: u8,
    /// MapScale: world map pixels per region.
    pub world_scale: f32,
    /// MapShowPeople / MapShowInfohubs / MapShowTelehubs / MapShowLandForSale.
    pub world_people: bool,
    pub world_infohubs: bool,
    pub world_telehubs: bool,
    pub world_land_for_sale: bool,
    /// MapShowEvents / ShowMatureEvents / ShowAdultEvents.
    pub world_events: bool,
    pub world_mature_events: bool,
    pub world_adult_events: bool,
    /// The inventory's landmarks as pins (the full-frame map's layer; not
    /// in Firestorm).
    pub world_landmarks: bool,
    /// MapShowGridCoords.
    pub world_grid_coords: bool,
    /// FSWorldMapDoubleclickTeleport.
    pub world_double_click_tp: bool,
    /// ShowBanLines: 0 hidden, 1 on collision, 2 on proximity.
    pub ban_lines: u8,
}

impl Default for MapSettings {
    fn default() -> Self {
        MapSettings {
            mini_scale: 128.0,
            // forced to north-up at the first login (llnetmap.cpp)
            mini_rotate: false,
            mini_objects: true,
            mini_physical: false,
            mini_scripted: false,
            mini_temp_on_rez: false,
            mini_property_lines: true,
            mini_for_sale: false,
            mini_collision: true,
            mini_auto_center: true,
            mini_double_click: 2,
            world_scale: 128.0,
            world_people: true,
            world_infohubs: true,
            world_telehubs: true,
            world_land_for_sale: true,
            world_events: true,
            world_mature_events: true,
            world_adult_events: false,
            world_landmarks: true,
            world_grid_coords: false,
            world_double_click_tp: true,
            ban_lines: 1,
        }
    }
}

/// Per-effect choices survive quality changes and the master switch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct WaterEffectsSettings {
    pub enabled: bool,
    pub quality: u8,
    pub waves: bool,
    pub absorption: bool,
    pub fresnel: bool,
    pub wet_ground: bool,
    pub shore_foam: bool,
    pub contact_foam: bool,
    pub crest_foam: bool,
    pub interactions: bool,
    pub caustics: bool,
    pub splashes: bool,
    pub bubbles: bool,
}
impl Default for WaterEffectsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            quality: 1,
            waves: true,
            absorption: true,
            fresnel: true,
            wet_ground: true,
            shore_foam: true,
            contact_foam: true,
            crest_foam: true,
            interactions: true,
            caustics: true,
            splashes: true,
            bubbles: true,
        }
    }
}
impl WaterEffectsSettings {
    pub fn render(&self) -> aurora_render::WaterEffects {
        aurora_render::WaterEffects {
            enabled: self.enabled,
            quality: self.quality.min(3),
            particle_budget: 4096,
            waves: self.waves,
            absorption: self.absorption,
            fresnel: self.fresnel,
            wet_ground: self.wet_ground,
            shore_foam: self.shore_foam,
            contact_foam: self.contact_foam,
            crest_foam: self.crest_foam,
            interactions: self.interactions,
            caustics: self.caustics,
            splashes: self.splashes,
            bubbles: self.bubbles,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn automatic_flight_preferences_default_on_and_keep_independent_saved_choices() {
        use super::Settings;
        for json in ["{}", r#"{"automatic_fly":false}"#, r#"{"automatic_land":false}"#] {
            let loaded: Settings = serde_json::from_str(json).expect("synthetic older preferences");
            assert_eq!(loaded.automatic_fly, !json.contains("automatic_fly"));
            assert_eq!(loaded.automatic_land, !json.contains("automatic_land"));
        }
        for automatic_fly in [false, true] {
            for automatic_land in [false, true] {
                let original = Settings {
                    automatic_fly,
                    automatic_land,
                    ..Settings::default()
                };
                let bytes = serde_json::to_vec(&original).expect("serialize automatic flight choices");
                let loaded: Settings = serde_json::from_slice(&bytes).expect("restore automatic flight choices");
                assert_eq!((loaded.automatic_fly, loaded.automatic_land), (automatic_fly, automatic_land));
            }
        }
    }

    /// The early read of `allow_overlays` agrees with the full settings and
    /// keeps the overlays out on any doubt.
    #[test]
    fn overlays_are_allowed_only_when_the_file_says_so() {
        assert!(super::allow_overlays_in(Some(br#"{"draw_distance":96,"allow_overlays":true}"#)));
        for file in [
            Some(&br#"{"allow_overlays":false}"#[..]),
            Some(br#"{"allow_overlays":"yes"}"#),
            Some(br#"{"draw_distance":96}"#),
            Some(b"not json"),
            None,
        ] {
            assert!(!super::allow_overlays_in(file), "{file:?}");
        }
        assert!(!super::Settings::default().allow_overlays);
        let saved = serde_json::to_vec(&super::Settings {
            allow_overlays: true,
            ..Default::default()
        })
        .expect("settings json");
        assert!(super::allow_overlays_in(Some(&saved)));
    }

    /// UTC, a clock time of the first version (#453), and an unknown name
    /// come back as SLT, the rest of the island kept.
    #[test]
    fn a_saved_utc_clock_falls_back_to_slt() {
        let read = |json: &str| serde_json::from_str::<super::SystemIsland>(json).expect("system island");
        let utc = read(r#"{"clock_zone":"Utc","clock_seconds":false,"ping":false}"#);
        assert_eq!(utc.clock_zone, super::ClockZone::Slt);
        assert!(!utc.clock_seconds && !utc.ping);
        assert_eq!(read(r#"{"clock_zone":"Mars"}"#).clock_zone, super::ClockZone::Slt);
        assert_eq!(read(r#"{"clock_zone":"Local"}"#).clock_zone, super::ClockZone::Local);
        assert_eq!(read("{}").clock_format, super::ClockFormat::Auto);
        let local = super::SystemIsland {
            clock_zone: super::ClockZone::Local,
            ..Default::default()
        };
        assert_eq!(read(&serde_json::to_string(&local).expect("json")), local);
    }

    #[test]
    fn group_title_choices_migrate_the_former_switch_and_survive_reload() {
        use crate::world::names::GroupTitles;

        let old: super::Settings = serde_json::from_str("{}").expect("old settings");
        assert_eq!(old.sanitized().group_titles, GroupTitles::All);
        for mode in GroupTitles::ALL {
            for hide in [false, true] {
                let old: super::Settings = serde_json::from_value(serde_json::json!({
                    "group_titles": mode,
                    "hide_own_group_title": hide,
                }))
                .expect("former title preferences");
                let migrated = old.sanitized();
                let expected = if hide { GroupTitles::AllExceptMine } else { mode };
                assert_eq!(migrated.group_titles, expected);
                let saved = serde_json::to_value(&migrated).expect("save title choice");
                assert!(saved.get("hide_own_group_title").is_none(), "only the combined choice is saved");
                let reloaded: super::Settings = serde_json::from_value(saved).expect("reload title choice");
                assert_eq!(reloaded.sanitized().group_titles, expected);
            }
        }
    }

    #[test]
    fn a_shutter_turned_off_in_the_interface_sounds_stays_quiet() {
        let fresh: super::Settings = serde_json::from_str("{}").expect("fresh settings");
        assert!(
            fresh.sanitized().snapshot.sound,
            "the shutter sounds by default, like the Legacy Viewer"
        );
        for (play, sound) in [(false, false), (true, true)] {
            let old: super::Settings = serde_json::from_value(serde_json::json!({
                "audio": { "ui": { "play": { "Snapshot": play, "Click": false } } },
            }))
            .expect("former shutter switch");
            let migrated = old.sanitized();
            assert_eq!(migrated.snapshot.sound, sound);
            assert!(!migrated.audio.ui.play.contains_key("Snapshot"), "one switch is left");
            assert_eq!(migrated.audio.ui.play.get("Click"), Some(&false), "the other sounds are kept");
        }
        let quiet: super::Settings =
            serde_json::from_value(serde_json::json!({ "snapshot": { "sound": false } })).expect("quiet snapshots");
        assert!(!quiet.sanitized().snapshot.sound);
    }

    #[test]
    fn plate_click_preferences_preserve_old_settings_and_independent_choices() {
        let old: super::Settings = serde_json::from_str(r#"{"plate_click":"Profile"}"#).expect("old plate setting");
        assert_eq!(old.plate_click, super::PlateClick::Profile);
        assert_eq!(old.plate_shift_click, super::PlateClick::Nothing);
        assert_eq!(old.plate_ctrl_click, super::PlateClick::Nothing);
        for action in super::PlateClick::ALL {
            let original = super::Settings {
                plate_click: super::PlateClick::Profile,
                plate_shift_click: action,
                plate_ctrl_click: super::PlateClick::Menu,
                ..Default::default()
            };
            let bytes = serde_json::to_vec(&original).expect("synthetic plate choices");
            let saved: super::Settings = serde_json::from_slice(&bytes).expect("read plate choices");
            let saved = saved.sanitized();
            assert_eq!(saved.plate_click, original.plate_click);
            assert_eq!(saved.plate_shift_click, original.plate_shift_click);
            assert_eq!(saved.plate_ctrl_click, original.plate_ctrl_click);
        }
    }

    #[test]
    fn construction_layout_is_saved_locally_and_shared_between_accounts() {
        use crate::build::layout::{Item, Panel};
        let path = scratch_settings_file();
        let first = uuid::Uuid::from_u128(101);
        let second = uuid::Uuid::from_u128(102);
        let mut settings = Settings::default();
        settings.select_hover_account(first);
        settings.build.layout.place(Item::Actions, Panel::Settings, Some(Item::Snap), false);
        settings.build.layout.place(Item::Shape, Panel::Hidden, None, false);
        settings.build.transform_expanded = false;
        settings.build.grid_resolution = 1.25;
        let expected = settings.build.clone();
        settings.save_to(&path);
        let mut other = Settings::load_from(&path);
        other.select_hover_account(second);
        assert_eq!(other.build, expected);
        other.build.layout.place(Item::Shape, Panel::Navigation, Some(Item::General), true);
        other.build.snap = false;
        let expected = other.build.clone();
        other.save_to(&path);
        let mut reopened = Settings::load_from(&path);
        reopened.select_hover_account(first);
        assert_eq!(reopened.build, expected);
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(
            written["build"]["layout"].is_object(),
            "Construction uses the global preference object"
        );
    }
    #[test]
    fn water_choices_round_trip_and_quality_does_not_reset_effects() {
        let mut settings = super::WaterEffectsSettings {
            enabled: true,
            quality: 255,
            bubbles: false,
            caustics: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let loaded: super::WaterEffectsSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, loaded);
        assert_eq!(loaded.render().quality, 3);
        let flags = settings.render().flags();
        settings.quality = 0;
        assert_eq!(settings.render().flags(), flags);
        settings.enabled = false;
        assert_eq!(settings.render().flags(), 0);
        settings.enabled = true;
        assert_eq!(settings.render().flags(), flags);
        let old: super::WaterEffectsSettings = serde_json::from_str("{\"enabled\":false}").unwrap();
        assert!(!old.enabled);
        assert!(old.bubbles);
    }
    use super::*;

    #[test]
    fn foot_avatar_budget_migrates_defaults_round_trips_and_sanitizes() {
        let previous: Settings = serde_json::from_str(r#"{"ground_avatar_feet":true,"ground_feet_max_height":0.4}"#).unwrap();
        assert_eq!(previous.ground_feet_max_avatars, 5, "existing preferences get the new default");
        assert!(previous.ground_avatar_feet);
        for (maximum, expected) in [(0, 0), (5, 5), (27, 27), (100, 100), (u32::MAX, 100)] {
            let settings = Settings {
                ground_feet_max_avatars: maximum,
                ..Settings::default()
            };
            let saved = serde_json::to_string(&settings).unwrap();
            let restored: Settings = serde_json::from_str(&saved).unwrap();
            assert_eq!(restored.sanitized().ground_feet_max_avatars, expected);
        }
    }

    #[test]
    fn foot_height_settings_sanitize_limits_and_nonfinite_values() {
        for (value, expected) in [
            (-1.0, 0.0),
            (0.4, 0.4),
            (1.0, 1.0),
            (4.0, 1.0),
            (f32::NAN, 0.4),
            (f32::INFINITY, 0.4),
        ] {
            let settings = Settings {
                ground_feet_max_height: value,
                ..Settings::default()
            }
            .sanitized();
            assert_eq!(settings.ground_feet_max_height, expected);
        }
    }

    #[test]
    fn name_tag_preferences_preserve_old_defaults_and_saved_choices() {
        let old: Settings = serde_json::from_str("{}").expect("synthetic old settings");
        assert!(old.show_own_name_tag && old.show_friend_name_tags && old.show_other_name_tags);
        assert!(!old.temporary_name_tags);
        assert_eq!(old.name_tag_show_seconds, 10.0);
        let original = Settings {
            show_own_name_tag: false,
            show_friend_name_tags: false,
            show_other_name_tags: false,
            temporary_name_tags: true,
            name_tag_show_seconds: 27.0,
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&original).expect("serialize synthetic settings");
        let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
        let saved = saved.sanitized();
        assert!(!saved.show_own_name_tag && !saved.show_friend_name_tags && !saved.show_other_name_tags);
        assert!(saved.temporary_name_tags);
        assert_eq!(saved.name_tag_show_seconds, 27.0);
        for (value, expected) in [(-1.0, 1.0), (100.0, 60.0), (f32::NAN, 10.0), (f32::INFINITY, 10.0)] {
            let bounded = Settings {
                name_tag_show_seconds: value,
                ..Default::default()
            }
            .sanitized();
            assert_eq!(bounded.name_tag_show_seconds, expected);
        }
    }

    /// The views that replace the image: the last one turned on stays.
    #[test]
    fn one_debug_view_replaces_the_image_at_a_time() {
        let mut d = DebugView {
            glow_view: true,
            lod_info: true,
            ..Default::default()
        };
        let before = d;
        // nothing turned on: nothing changes
        d.keep_one_view(&before);
        assert_eq!(d, before);
        d.ao_view = true;
        d.keep_one_view(&before);
        assert!(d.ao_view && !d.glow_view && !d.shadow_view);
        let before = d;
        d.shadow_view = true;
        d.keep_one_view(&before);
        assert!(d.shadow_view && !d.ao_view && !d.glow_view);
        // turned off: none is left, and the rest is untouched
        let before = d;
        d.shadow_view = false;
        d.check_probe_sets = true;
        d.keep_one_view(&before);
        assert!(!d.shadow_view && !d.ao_view && !d.glow_view && d.lod_info && d.check_probe_sets);
        let asked = d.render();
        assert!(asked.check_probe_sets && !asked.check_light_sets && !asked.shadows && !asked.ambient_occlusion);
    }
    use crate::keybinds::{Action, Input};
    use winit::keyboard::KeyCode;

    #[test]
    fn notecard_view_defaults_to_classic_and_last_choice_survives_restart() {
        use crate::ui::inventory::NotecardView;

        let path = scratch_settings_file();
        for old in ["{}", r#"{"inventory":{"search_library":false}}"#] {
            std::fs::write(&path, old).expect("synthetic old settings");
            assert_eq!(Settings::load_from(&path).inventory.notecard_view, NotecardView::Classic);
        }
        let mut settings = Settings::load_from(&path);
        for view in [NotecardView::Markdown, NotecardView::Classic, NotecardView::Edit] {
            settings.inventory.notecard_view = view;
            settings.save_to(&path);
            settings = Settings::load_from(&path);
            assert_eq!(settings.inventory.notecard_view, view);
            assert!(!settings.inventory.search_library, "other saved preferences survive");
        }
    }

    #[test]
    fn teleport_confirmation_is_on_for_old_settings_and_survives_reload() {
        let old: Settings = serde_json::from_str("{}").expect("synthetic old settings");
        assert!(old.confirm_teleports);
        let original = Settings {
            confirm_teleports: false,
            ..Settings::default()
        };
        let saved = serde_json::to_vec(&original).expect("serialize synthetic settings");
        let loaded: Settings = serde_json::from_slice(&saved).expect("reload synthetic settings");
        assert!(!loaded.sanitized().confirm_teleports);
    }

    #[test]
    fn chat_channel_selector_is_hidden_by_default_and_survives_reload() {
        assert!(!Settings::default().show_chat_channel);
        let old: Settings = serde_json::from_str("{}").expect("synthetic old settings");
        assert!(!old.show_chat_channel);
        for show_chat_channel in [false, true] {
            let original = Settings {
                show_chat_channel,
                ..Settings::default()
            };
            let saved = serde_json::to_vec(&original).expect("serialize synthetic channel preference");
            let loaded: Settings = serde_json::from_slice(&saved).expect("reload synthetic channel preference");
            assert_eq!(loaded.sanitized().show_chat_channel, show_chat_channel);
        }
    }

    /// The chat bar keeps the keyboard by default, earlier files get it once
    /// (they all wrote the old "off"), a later choice stays.
    #[test]
    fn a_file_on_the_former_deluxe_moves_to_gtao_once() {
        use crate::graphics_presets::{Preset, deluxe_v18};
        let mut old = Settings {
            settings_version: 17,
            ..Settings::default()
        };
        // the Deluxe of then: Legacy occlusion, none of the later effects
        deluxe_v18().write(&mut old);
        old.ao_method = 0;
        let now = old.sanitized();
        // straight to the Deluxe of now
        assert_eq!(Preset::matching(&now), Some(Preset::Deluxe));
        assert_eq!((now.ao_method, now.gtao_quality), (1, 3));
        // chosen after the change: kept
        let mut chosen = Settings::default();
        Preset::Deluxe.apply(&mut chosen);
        chosen.ao_method = 0;
        assert_eq!(chosen.sanitized().ao_method, 0);
    }

    #[test]
    fn a_file_on_the_deluxe_of_version_18_takes_the_new_effects_once() {
        use crate::graphics_presets::{Preset, deluxe_v18};
        let mut old = Settings {
            settings_version: 18,
            ..Settings::default()
        };
        deluxe_v18().write(&mut old);
        let now = old.sanitized();
        // through version 19 to the Deluxe of now: the contact shadows and
        // the indirect light, the experimental bloom off
        assert!(now.contact_shadows && !now.bloom && now.screen_indirect_light);
        assert_eq!(Preset::matching(&now), Some(Preset::Deluxe));
        // turned off after the change: kept off
        let mut chosen = Settings::default();
        Preset::Deluxe.apply(&mut chosen);
        chosen.contact_shadows = false;
        assert!(!chosen.sanitized().contact_shadows);
    }

    #[test]
    fn a_file_on_the_deluxe_of_version_19_turns_the_experimental_effects_off_once() {
        use crate::graphics_presets::{Preset, deluxe_v19};
        let mut old = Settings {
            settings_version: 19,
            ..Settings::default()
        };
        deluxe_v19().write(&mut old);
        let now = old.sanitized();
        // to the Deluxe of now: the bloom off, the indirect light on again
        assert!(now.contact_shadows && !now.bloom && now.screen_indirect_light);
        assert_eq!(Preset::matching(&now), Some(Preset::Deluxe));
        // turned on again after the change: kept on
        let mut chosen = Settings::default();
        Preset::Deluxe.apply(&mut chosen);
        chosen.bloom = true;
        chosen.screen_indirect_light = true;
        let kept = chosen.sanitized();
        assert!(kept.bloom && kept.screen_indirect_light);
    }

    #[test]
    fn a_file_on_the_high_of_version_19_moves_to_gtao_medium_once() {
        use crate::graphics_presets::{Preset, high_v19};
        let mut old = Settings {
            settings_version: 19,
            ..Settings::default()
        };
        high_v19().write(&mut old);
        let now = old.sanitized();
        // through version 20 to the High of now
        assert_eq!((now.ao_method, now.gtao_quality), (1, 2));
        assert!(now.contact_shadows);
        assert_eq!(Preset::matching(&now), Some(Preset::High));
        // the Legacy method chosen after the change: kept
        let mut chosen = Settings::default();
        Preset::High.apply(&mut chosen);
        chosen.ao_method = 0;
        assert_eq!(chosen.sanitized().ao_method, 0);
    }

    #[test]
    fn files_on_the_levels_of_version_20_follow_the_levels_of_now_once() {
        use crate::graphics_presets::{Preset, deluxe_v20, high_v20, medium_v20};
        for (former, preset) in [
            (medium_v20(), Preset::Medium),
            (high_v20(), Preset::High),
            (deluxe_v20(), Preset::Deluxe),
        ] {
            let mut old = Settings {
                settings_version: 20,
                ..Settings::default()
            };
            former.write(&mut old);
            assert_eq!(Preset::matching(&old), None, "{preset:?}");
            let now = old.sanitized();
            assert_eq!(Preset::matching(&now), Some(preset));
            // customized on that level: kept
            let mut custom = Settings {
                settings_version: 20,
                ..Settings::default()
            };
            former.write(&mut custom);
            custom.max_particles = 1024;
            let before = (custom.ssao, custom.gtao_quality, custom.draw_distance);
            let kept = custom.sanitized();
            assert_eq!((kept.ssao, kept.gtao_quality, kept.draw_distance), before);
        }
    }

    #[test]
    fn chat_bar_keeps_focus_by_default_and_once_for_earlier_files() {
        assert!(Settings::default().sanitized().chat_keep_focus);
        let old = Settings {
            settings_version: 16,
            chat_keep_focus: false,
            ..Settings::default()
        };
        assert!(old.sanitized().chat_keep_focus);
        let chosen = Settings {
            chat_keep_focus: false,
            ..Settings::default()
        };
        assert!(!chosen.sanitized().chat_keep_focus);
    }

    /// ArrowKeysAlwaysMove is off in the Legacy Viewer: the arrows move the
    /// chat bar's cursor. Earlier files all held Aurora's former "on".
    #[test]
    fn arrow_keys_edit_the_chat_bar_by_default_and_once_for_earlier_files() {
        assert!(!Settings::default().sanitized().arrow_keys_always_move);
        let old = Settings {
            settings_version: 21,
            arrow_keys_always_move: true,
            ..Settings::default()
        };
        assert!(!old.sanitized().arrow_keys_always_move);
        let chosen = Settings {
            arrow_keys_always_move: true,
            ..Settings::default()
        };
        assert!(chosen.sanitized().arrow_keys_always_move);
    }

    /// ACES and a mix of 0.7 are the defaults, like RenderTonemapType and
    /// RenderTonemapMix; earlier files all held Khronos PBR Neutral, a later
    /// choice of it stays.
    #[test]
    fn aces_is_the_default_tone_mapper_and_once_for_earlier_files() {
        let s = Settings::default().sanitized();
        assert_eq!((s.tonemapper, s.tonemap_mix), (1, 0.7));
        let old = Settings {
            settings_version: 24,
            tonemapper: 0,
            ..Settings::default()
        };
        assert_eq!(old.sanitized().tonemapper, 1);
        let chosen = Settings {
            tonemapper: 0,
            tonemap_mix: f32::NAN,
            ..Settings::default()
        };
        let chosen = chosen.sanitized();
        assert_eq!((chosen.tonemapper, chosen.tonemap_mix), (0, 0.7));
    }

    /// Performance does not open with the session: off by default, off
    /// once for earlier files (they held the window's state at exit), on
    /// when Preferences ask for it.
    #[test]
    fn performance_waits_for_the_frame_rate_click() {
        assert!(!Settings::default().sanitized().show_perf);
        let old = Settings {
            settings_version: 25,
            show_perf: true,
            ..Settings::default()
        };
        assert!(!old.sanitized().show_perf);
        let chosen = Settings {
            show_perf: true,
            ..Settings::default()
        };
        assert!(chosen.sanitized().show_perf);
    }

    /// The rez effect is the default; earlier files all held the former
    /// grey default, a later choice of grey stays.
    #[test]
    fn rez_effect_is_the_default_and_once_for_earlier_files() {
        assert!(!Settings::default().sanitized().classic_rez_effect);
        let old = Settings {
            settings_version: 22,
            classic_rez_effect: true,
            ..Settings::default()
        };
        assert!(!old.sanitized().classic_rez_effect);
        let chosen = Settings {
            classic_rez_effect: true,
            ..Settings::default()
        };
        assert!(chosen.sanitized().classic_rez_effect);
    }

    #[test]
    fn contacts_tab_is_shown_by_default_and_survives_reload() {
        assert!(Settings::default().show_contacts_tab);
        // the earlier files all wrote "hidden": shown once, then kept
        let old: Settings =
            serde_json::from_str(r#"{"settings_version": 14, "show_contacts_tab": false}"#).expect("synthetic old settings");
        assert!(old.sanitized().show_contacts_tab);
        for show_contacts_tab in [false, true] {
            let original = Settings {
                show_contacts_tab,
                ..Settings::default()
            };
            let saved = serde_json::to_vec(&original).expect("serialize synthetic contacts preference");
            let loaded: Settings = serde_json::from_slice(&saved).expect("reload synthetic contacts preference");
            assert_eq!(loaded.sanitized().show_contacts_tab, show_contacts_tab);
        }
    }

    #[test]
    fn context_menu_style_defaults_to_list_and_survives_reload() {
        let old: Settings = serde_json::from_str("{}").expect("synthetic old settings");
        assert!(!old.use_pie_menu);
        for use_pie_menu in [false, true] {
            let original = Settings {
                use_pie_menu,
                ..Settings::default()
            };
            let saved = serde_json::to_vec(&original).expect("serialize synthetic menu settings");
            let loaded: Settings = serde_json::from_slice(&saved).expect("reload synthetic menu settings");
            assert_eq!(loaded.sanitized().use_pie_menu, use_pie_menu);
        }
    }

    #[test]
    fn old_settings_follow_the_os_and_saved_language_and_privacy_survive() {
        let old: Settings = serde_json::from_str("{}").expect("synthetic old settings");
        assert_eq!(old.language, "default");
        assert!(old.language_is_public);
        for language in ["default", "en", "fr", "pt-BR"] {
            for public in [false, true] {
                let original = Settings {
                    language: language.into(),
                    language_is_public: public,
                    ..Settings::default()
                };
                let bytes = serde_json::to_vec(&original).expect("serialize synthetic language settings");
                let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic language settings");
                let saved = saved.sanitized();
                assert_eq!(saved.language, language);
                assert_eq!(saved.language_is_public, public);
            }
        }
    }

    #[test]
    fn fps_defaults_and_saved_preferences() {
        let defaults = Settings::default().sanitized();
        assert!(defaults.fps_cap);
        assert_eq!(defaults.fps_limit, 120);
        let missing: Settings = serde_json::from_str("{}").expect("synthetic settings without FPS preferences");
        let missing = missing.sanitized();
        assert!(missing.fps_cap);
        assert_eq!(missing.fps_limit, 120);
        for enabled in [false, true] {
            for limit in [30, 60, 144, 500] {
                let original = Settings {
                    fps_cap: enabled,
                    fps_limit: limit,
                    background_fps_limit: 120,
                    ..Settings::default()
                };
                let bytes = serde_json::to_vec(&original).expect("serialize synthetic settings");
                let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
                let saved = saved.sanitized();
                assert_eq!(saved.fps_cap, enabled);
                assert_eq!(saved.fps_limit, limit);
                assert_eq!(saved.background_fps_limit, 120);
            }
        }
    }

    #[test]
    fn legacy_hud_key_migrates_to_ctrl_or_keeps_the_custom_choice() {
        let old: Settings = serde_json::from_str("{}").expect("old synthetic settings");
        assert_eq!(
            old.sanitized().keybinds.get(Action::HudDrag),
            Settings::default().keybinds.get(Action::HudDrag)
        );
        for key in [
            crate::keybinds::HoldKey::Alt,
            crate::keybinds::HoldKey::Ctrl,
            crate::keybinds::HoldKey::Shift,
            crate::keybinds::HoldKey::Key("KeyB".into()),
        ] {
            for table_present in [false, true] {
                let mut json = serde_json::json!({ "hud_drag_key": key });
                if table_present {
                    json["keybinds"] = serde_json::to_value(legacy_keybindings()).expect("old shortcut table");
                }
                let loaded: Settings = serde_json::from_value(json).expect("synthetic old settings");
                let loaded = loaded.sanitized();
                let migrated = if key == crate::keybinds::HoldKey::Alt {
                    crate::keybinds::HoldKey::Ctrl
                } else {
                    key.clone()
                };
                assert_eq!(loaded.keybinds.get(Action::HudDrag), [Some(migrated.into_binding()), None]);
                let json = serde_json::to_value(&loaded).expect("migrated settings");
                assert!(json.get("hud_drag_key").is_none());
                let reloaded: Settings = serde_json::from_value(json).expect("reload migrated settings");
                assert_eq!(reloaded.sanitized().keybinds, loaded.keybinds);
            }
        }
    }

    #[test]
    fn old_default_hud_binding_changes_once_and_custom_slots_are_preserved() {
        use crate::keybinds::HoldKey;
        let alt = Some(HoldKey::Alt.into_binding());
        let ctrl = Some(HoldKey::Ctrl.into_binding());
        let key = Some(HoldKey::Key("KeyB".into()).into_binding());
        for slots in [
            [alt.clone(), None],
            [None, None],
            [key.clone(), ctrl.clone()],
            [alt.clone(), key.clone()],
            [ctrl.clone(), alt.clone()],
        ] {
            let expected = if slots == [alt.clone(), None] {
                [ctrl.clone(), None]
            } else {
                slots.clone()
            };
            for version in [9, SETTINGS_VERSION] {
                let mut settings = Settings {
                    settings_version: version,
                    ..Settings::default()
                };
                for (slot, binding) in slots.iter().enumerate() {
                    settings.keybinds.set(Action::HudDrag, slot, binding.clone());
                }
                let json = serde_json::to_vec(&settings).expect("synthetic saved settings");
                let loaded: Settings = serde_json::from_slice(&json).expect("read synthetic settings");
                let loaded = loaded.sanitized();
                assert_eq!(
                    loaded.keybinds.get(Action::HudDrag),
                    if version < 10 { expected.clone() } else { slots.clone() }
                );
                assert_eq!(loaded.settings_version, SETTINGS_VERSION);
                assert_eq!(loaded.clone().sanitized().keybinds, loaded.keybinds);
            }
        }
    }

    #[test]
    fn hud_primary_secondary_and_cleared_slots_survive_stale_legacy_settings() {
        use crate::keybinds::{Binding, Mods};
        for slots in [
            [
                Some(Binding::captured(Input::key(KeyCode::KeyB), Mods::default())),
                Some(Binding::captured(Input::key(KeyCode::ControlRight), Mods::default())),
            ],
            [None, None],
        ] {
            let mut settings = Settings::default();
            for (slot, binding) in slots.iter().enumerate() {
                settings.keybinds.set(Action::HudDrag, slot, binding.clone());
            }
            let mut json = serde_json::to_value(&settings).expect("new settings");
            json["hud_drag_key"] = serde_json::json!("Alt");
            let loaded: Settings = serde_json::from_value(json).expect("settings with a stale key");
            assert_eq!(loaded.sanitized().keybinds.get(Action::HudDrag), slots);
        }
    }

    /// A first launch opens the viewer window maximized; afterwards the
    /// window is what the user left.
    #[test]
    fn the_first_launch_window_is_maximized() {
        let state = |json: &str| {
            let file: serde_json::Value = serde_json::from_str(json).expect("synthetic settings");
            let (maximized, size) = saved_window_state(&file);
            (
                window_starts_maximized(maximized, size, false),
                window_starts_maximized(maximized, size, true),
            )
        };
        // no settings file (nothing read), a blank one, one without a word of the window
        assert!(window_starts_maximized(None, false, false));
        assert_eq!(state("{}"), (true, false));
        assert_eq!(state(r#"{"language":"fr","show_chat":true}"#), (true, false));
        // the state saved at exit
        assert_eq!(
            state(r#"{"window_width":1600,"window_height":900,"window_maximized":false}"#),
            (false, false)
        );
        assert_eq!(
            state(r#"{"window_width":1600,"window_height":900,"window_maximized":true}"#),
            (true, false)
        );
        // a size alone: forced by a script or a test
        assert_eq!(state(r#"{"window_width":1280,"window_height":720}"#), (false, false));
        // what a session writes always says both: a restored window stays restored
        let mut left = Settings::default();
        assert!(!left.window_maximized, "the default is the saved state of a restored window");
        left.window_width = 1280;
        let json = serde_json::to_string(&left).expect("serialize synthetic settings");
        assert_eq!(state(&json), (false, false));
        left.window_maximized = true;
        let json = serde_json::to_string(&left).expect("serialize synthetic settings");
        assert_eq!(state(&json), (true, false));
    }

    #[test]
    fn a_new_profile_starts_with_smaa_and_a_saved_choice_is_kept() {
        use aurora_render::AntiAliasing;
        let fresh: Settings = serde_json::from_str("{}").expect("synthetic empty settings");
        assert_eq!(fresh.sanitized().render_settings().aa, AntiAliasing::Smaa);
        let saved: Settings = serde_json::from_str(r#"{"antialiasing":3}"#).expect("synthetic saved settings");
        assert_eq!(saved.sanitized().render_settings().aa, AntiAliasing::Msaa(4));
    }

    #[test]
    fn first_launch_movement_defaults_survive_saving_and_loading() {
        let fresh = Settings::default().sanitized();
        let bytes = serde_json::to_vec(&fresh).expect("serialize synthetic settings");
        let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
        assert_eq!(saved.sanitized().keybinds, fresh.keybinds);
        for action in [Action::Forward, Action::Back, Action::Left, Action::Right] {
            assert!(fresh.keybinds.get(action)[1].is_some());
        }
    }

    #[test]
    fn existing_settings_do_not_gain_or_replace_movement_bindings() {
        let mut existing = Settings::default();
        for action in [Action::Forward, Action::Back, Action::Left, Action::Right] {
            existing.keybinds.set(action, 1, None);
        }
        existing.keybinds.set(
            Action::Forward,
            1,
            Some(crate::keybinds::Binding {
                input: Input::key(KeyCode::KeyT),
                ctrl: false,
                shift: false,
                alt: false,
            }),
        );
        let bytes = serde_json::to_vec(&existing).expect("serialize synthetic settings");
        let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
        assert_eq!(saved.sanitized().keybinds, existing.keybinds);
    }

    /// The move of the letter shortcuts to the layout's letters runs once
    /// (`KeyBindings::follow_layout_letters`, tested per layout there): a
    /// key chosen afterwards, even the former QWERTY place, is kept.
    #[test]
    fn letter_shortcuts_follow_the_layout_once_and_keep_choices() {
        let plain = |code| {
            Some(crate::keybinds::Binding {
                input: Input::key(code),
                ctrl: false,
                shift: false,
                alt: false,
            })
        };
        for version in [11, SETTINGS_VERSION] {
            let mut file = Settings {
                settings_version: version,
                ..Settings::default()
            };
            file.keybinds.set(Action::Mouselook, 0, plain(KeyCode::KeyK));
            file.keybinds.set(Action::Mouselook, 1, None);
            let loaded = file.sanitized();
            assert_eq!(loaded.keybinds.get(Action::Mouselook), [plain(KeyCode::KeyK), None]);
            assert_eq!(loaded.settings_version, SETTINGS_VERSION);
        }
        let mut chosen = Settings::default();
        chosen.keybinds.set(Action::Mouselook, 0, plain(KeyCode::KeyM));
        assert_eq!(chosen.sanitized().keybinds.get(Action::Mouselook)[0], plain(KeyCode::KeyM));
    }

    /// The bubbles every earlier file wrote (they were the default) become
    /// Aurora's workspace once; Firestorm's lines, a choice, stay; from now
    /// on the bubbles chosen again are kept.
    #[test]
    fn saved_bubbles_move_to_the_aurora_workspace_once() {
        assert_eq!(Settings::default().chat_style, ChatStyle::Aurora);
        for (before, after) in [(ChatStyle::Bubbles, ChatStyle::Aurora), (ChatStyle::Legacy, ChatStyle::Legacy)] {
            let old = Settings {
                settings_version: 12,
                chat_style: before,
                ..Settings::default()
            };
            assert_eq!(old.sanitized().chat_style, after, "{before:?}");
        }
        for style in [ChatStyle::Bubbles, ChatStyle::Compact, ChatStyle::Legacy] {
            let chosen = Settings {
                chat_style: style,
                ..Settings::default()
            };
            assert_eq!(chosen.sanitized().chat_style, style);
        }
    }

    /// The unrestricted camera and the camera collisions turned off reach the
    /// earlier files once (they all wrote the old "off"); turned off again
    /// afterwards, they stay off.
    #[test]
    fn camera_constraints_are_off_by_default_and_once_for_earlier_files() {
        let c = Settings::default().sanitized().camera;
        assert!(c.disable_constraints && c.ignore_sim_constraints);
        let mut old = Settings {
            settings_version: 13,
            ..Settings::default()
        };
        old.camera.disable_constraints = false;
        old.camera.ignore_sim_constraints = false;
        let c = old.sanitized().camera;
        assert!(c.disable_constraints && c.ignore_sim_constraints);
        let mut chosen = Settings::default();
        chosen.camera.disable_constraints = false;
        chosen.camera.ignore_sim_constraints = false;
        let c = chosen.sanitized().camera;
        assert!(!c.disable_constraints && !c.ignore_sim_constraints);
    }

    /// Inter reaches the earlier files once (they all wrote Noto Sans, and
    /// those before version 3 Inter); chosen again afterwards, Noto Sans
    /// stays, and so does any other font.
    #[test]
    fn inter_is_the_default_font_and_once_for_earlier_files() {
        assert_eq!(Settings::default().sanitized().font, "Inter");
        for (version, font, now) in [
            (15, "Noto Sans", "Inter"),
            (2, "Inter", "Inter"),
            (15, "Roboto", "Roboto"),
            (
                15,
                "system:C:\\Windows\\Fonts\\segoeui.ttf",
                "system:C:\\Windows\\Fonts\\segoeui.ttf",
            ),
            (16, "Noto Sans", "Noto Sans"),
        ] {
            let old = Settings {
                settings_version: version,
                font: font.into(),
                ..Settings::default()
            };
            assert_eq!(old.sanitized().font, now, "version {version}, {font}");
        }
    }

    /// A file written before both defaults changed (version 14) gets both:
    /// the Contacts tab of version 15 and the Inter font of version 16.
    #[test]
    fn a_version_14_file_gets_the_contacts_tab_and_inter() {
        let old = Settings {
            settings_version: 14,
            font: "Noto Sans".into(),
            show_contacts_tab: false,
            ..Settings::default()
        };
        let now = old.sanitized();
        assert!(now.show_contacts_tab);
        assert_eq!(now.font, "Inter");
        assert_eq!(now.settings_version, SETTINGS_VERSION);
    }

    #[test]
    fn parcel_music_waits_for_the_radio_toggle() {
        assert!(!Settings::default().sanitized().audio.music_autoplay);
        let mut old = Settings {
            settings_version: 6,
            ..Settings::default()
        };
        old.audio.music_autoplay = true;
        assert!(!old.sanitized().audio.music_autoplay);
        // switched back on after the migration: kept
        let mut on = Settings::default();
        on.audio.music_autoplay = true;
        assert!(on.sanitized().audio.music_autoplay);
    }

    #[test]
    fn names_by_relation_are_colored_by_default_and_for_earlier_files() {
        assert!(Settings::default().sanitized().colors.relation_names);
        // a file from before the switch has no such field
        for classic in [false, true] {
            let mut file = serde_json::to_value(Settings {
                settings_version: 10,
                classic_name_tags: classic,
                ..Settings::default()
            })
            .expect("synthetic saved settings");
            file["colors"].as_object_mut().expect("colors").remove("relation_names");
            let old: Settings = serde_json::from_value(file).expect("read synthetic settings");
            assert!(old.sanitized().colors.relation_names);
        }
        // a file that wrote the former default, off
        let mut off = Settings {
            settings_version: 26,
            ..Settings::default()
        };
        off.colors.relation_names = false;
        assert!(off.sanitized().colors.relation_names);
        // switched off after the migration: kept
        let mut off = Settings::default();
        off.colors.relation_names = false;
        assert!(!off.sanitized().colors.relation_names);
    }

    #[test]
    fn the_simulator_version_notice_is_off_by_default_and_for_earlier_files() {
        assert!(!Settings::default().sanitized().sim_version_alert);
        // a file that wrote the former default, on
        let old = Settings {
            settings_version: 28,
            sim_version_alert: true,
            ..Settings::default()
        };
        assert!(!old.sanitized().sim_version_alert);
        // switched on after the migration: kept
        let on = Settings {
            sim_version_alert: true,
            ..Settings::default()
        };
        assert!(on.sanitized().sim_version_alert);
    }

    #[test]
    fn background_cap_is_on_by_default() {
        assert!(Settings::default().sanitized().background_fps_cap);
        let old = Settings {
            settings_version: 8,
            background_fps_cap: false,
            ..Settings::default()
        };
        assert!(old.sanitized().background_fps_cap);
        // switched off after the migration: kept
        let off = Settings {
            background_fps_cap: false,
            ..Settings::default()
        };
        assert!(!off.sanitized().background_fps_cap);
    }

    #[test]
    fn the_former_look_at_switch_carries_into_the_privacy_settings() {
        // an older file: the switch off (its default) and a 10 m range
        let file = serde_json::json!({ "look_at": false, "look_at_range": 10.0, "settings_version": 9 });
        let s = serde_json::from_value::<Settings>(file).expect("read old settings").sanitized();
        assert!(s.lookat.private && s.lookat.private_local);
        assert!(s.lookat.limit_distance && s.lookat.max_distance == 10.0);
        // saved again: the former keys are gone, the choice stays
        let again = serde_json::to_value(&s).expect("serialize");
        assert!(again.get("look_at").is_none() && again.get("look_at_range").is_none());
        let s = serde_json::from_value::<Settings>(again).expect("read new settings").sanitized();
        assert!(s.lookat.private && s.lookat.private_local);
        // on, without a limit
        let file = serde_json::json!({ "look_at": true, "look_at_range": 64.0 });
        let s = serde_json::from_value::<Settings>(file).expect("read old settings").sanitized();
        assert_eq!(s.lookat, crate::world::lookat::Settings::default());
    }

    /// The snooze was saved in seconds (#945): it comes back in whole
    /// minutes, rounded up so that a short one does not become 0.
    #[test]
    fn the_former_snooze_seconds_carry_into_minutes() {
        let read = |file: serde_json::Value| serde_json::from_value::<Settings>(file).expect("read settings").sanitized();
        for (seconds, minutes) in [(900, 15), (0, 0), (20, 1), (61, 2), (3600, 60)] {
            let s = read(serde_json::json!({ "group_snooze_seconds": seconds }));
            assert_eq!(s.group_snooze_minutes, minutes, "{seconds} s");
        }
        // saved again: only the minutes
        let again = serde_json::to_value(read(serde_json::json!({ "group_snooze_seconds": 120 }))).expect("serialize");
        assert!(again.get("group_snooze_seconds").is_none());
        assert_eq!(read(again).group_snooze_minutes, 2);
        assert_eq!(Settings::default().group_snooze_minutes, 15);
        let huge = read(serde_json::json!({ "group_snooze_minutes": u32::MAX }));
        assert_eq!(huge.group_snooze_minutes, GROUP_SNOOZE_MAX_MINUTES);
    }

    fn scratch_settings_file() -> PathBuf {
        let dir = unit_test_root()
            .expect("unit test root")
            .join("forward-compat")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).expect("scratch folder");
        dir.join("settings.json")
    }

    fn read_back(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path).expect("saved settings")).expect("saved JSON")
    }

    /// The file an older build sees after a newer one: a shortcut, a choice
    /// and an option it does not know, under a higher version.
    fn file_from_a_newer_build() -> serde_json::Value {
        let mut file = serde_json::to_value(Settings {
            draw_distance: 200.0,
            username: "Someone Resident".into(),
            remember_username: true,
            ..Settings::default()
        })
        .expect("synthetic settings");
        file["settings_version"] = serde_json::json!(SETTINGS_VERSION + 5);
        file["future_option"] = serde_json::json!({ "x": 1 });
        file["chat_style"] = serde_json::json!("Cards");
        file["keybinds"]["map"]
            .as_array_mut()
            .expect("shortcut table")
            .push(serde_json::json!(["FutureAction", [null, null]]));
        file
    }

    #[test]
    fn a_newer_builds_file_keeps_everything_else_and_loses_nothing_on_save() {
        let path = scratch_settings_file();
        let file = file_from_a_newer_build();
        std::fs::write(&path, serde_json::to_vec(&file).expect("serialize")).expect("write");
        let (s, newer) = Settings::read_document(file, &path).expect("readable settings");
        let s = s.finished(newer);
        // only what is not understood falls back to its default
        assert_eq!(s.draw_distance, 200.0);
        assert_eq!(s.username, "Someone Resident");
        assert_eq!(s.chat_style, ChatStyle::Aurora);
        assert_eq!(s.keybinds, Settings::default().keybinds);
        assert!(crate::tolerant::backup_path(&path).exists(), "the newer file is kept aside");
        // saved unchanged: the newer build finds its own values again
        s.save_to(&path);
        let written = read_back(&path);
        assert_eq!(written["future_option"], serde_json::json!({ "x": 1 }));
        assert_eq!(written["chat_style"], "Cards");
        assert_eq!(written["settings_version"], SETTINGS_VERSION + 5);
        assert!(
            written["keybinds"]["map"]
                .as_array()
                .expect("map")
                .iter()
                .any(|e| e[0] == "FutureAction"),
            "the newer shortcut is still saved"
        );
        assert_eq!(written["draw_distance"], 200.0);
        // what is changed here is written, and the unknown option still stays
        let mut changed = s.clone();
        changed.draw_distance = 128.0;
        changed.chat_style = ChatStyle::Legacy;
        changed.save_to(&path);
        let written = read_back(&path);
        assert_eq!(written["draw_distance"], 128.0);
        assert_eq!(written["chat_style"], "Legacy");
        assert_eq!(written["future_option"], serde_json::json!({ "x": 1 }));
        // read again here: the change stays, the newer values are still kept
        let (again, newer) = Settings::read_document(written, &path).expect("readable settings");
        assert_eq!(again.chat_style, ChatStyle::Legacy);
        assert_eq!(again.draw_distance, 128.0);
        assert!(newer.is_some());
    }

    #[test]
    fn an_unreadable_value_in_a_current_file_costs_only_that_value() {
        let path = scratch_settings_file();
        let mut file = serde_json::to_value(Settings {
            draw_distance: 180.0,
            ..Settings::default()
        })
        .expect("synthetic settings");
        file["chat_style"] = serde_json::json!(42);
        file["former_key"] = serde_json::json!(true);
        std::fs::write(&path, serde_json::to_vec(&file).expect("serialize")).expect("write");
        let (s, newer) = Settings::read_document(file, &path).expect("readable settings");
        assert!(newer.is_none(), "same version: nothing to keep on save");
        let s = s.finished(newer);
        assert_eq!(s.draw_distance, 180.0);
        assert_eq!(s.chat_style, ChatStyle::Aurora);
        assert!(crate::tolerant::backup_path(&path).exists(), "the damaged file is kept aside");
        s.save_to(&path);
        let written = read_back(&path);
        // a key no build writes any more is not carried forward
        assert!(written.get("former_key").is_none());
        assert_eq!(written["settings_version"], SETTINGS_VERSION);
    }
}
