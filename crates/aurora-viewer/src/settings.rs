//! Persistent user settings (JSON in the platform config directory).
//! Passwords are never stored.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
}

impl Default for SystemIsland {
    fn default() -> Self {
        SystemIsland {
            clock: true,
            ping: true,
            fps: true,
            media: true,
        }
    }
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

/// The photo window: Aurora's panel anchored on the right, or Firestorm's
/// floater (floater_snapshot.xml: a floating window, the controls on the
/// left, the preview on the right). Preferences › Aurora.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PhotoWindowStyle {
    #[default]
    Aurora,
    Legacy,
}

/// What a left click on another avatar's name plate does (Preferences ›
/// Interface; the human's request of 2026-10-14). Each one is an entry of
/// the avatar's menu; nothing, the default, leaves the click to the world.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GridChoice {
    SecondLife,
    SecondLifeBeta,
    Custom,
}

/// Debug overlays (Preferences › Debug), not saved: they reset at each launch.
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
    pub ignored_update: String,
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
    /// Debug overlays (not saved).
    #[serde(skip)]
    pub debug: DebugView,
    /// Build tools (grid, snap, create, land brush).
    pub build: crate::build::BuildSettings,
    /// Camera (Firestorm "Move & View › View").
    pub camera: crate::camera::CameraSettings,
    /// Ground foot geometry after wearing/detaching a body attachment.
    pub auto_hover_height: bool,
    /// Fixed adjustment added to each automatic grounding estimate (metres).
    pub auto_hover_offset: f32,
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
    /// Photo window (Firestorm's snapshot floater settings).
    pub snapshot: crate::snapshot::Settings,
    /// Depth of field (Preferences › Graphics).
    pub dof: crate::dof::Settings,
    pub shape_show_hints: bool,
    pub shape_height_feet: bool,
    pub script_editor: crate::ui::scripts::settings::ScriptSettings,
    pub username: String,
    pub remember_username: bool,
    /// Remember the password (its login hash, in the system credential
    /// store; Firestorm "Remember password"). Off by default.
    pub remember_password: bool,
    pub grid: GridChoice,
    pub custom_login_uri: String,
    pub start_location: String,
    pub start_region: String,
    pub draw_distance: f32,
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
    /// Windows snap to each other and to the edges while they are moved or
    /// resized (Firestorm SnapEnabled, on).
    pub snap_windows: bool,
    pub show_perf: bool,
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
    /// RegionCheckTextureHeights: Region / Estate asks before sending
    /// elevation ranges whose low end is above the high one ("Don't ask"
    /// turns it off; on by default).
    pub region_check_texture_heights: bool,
    /// Ask before a payment above `payment_confirm_threshold` L$
    /// (FSConfirmPayments, on by default).
    pub confirm_payments: bool,
    /// FSPaymentConfirmationThreshold: 200 L$ by default.
    pub payment_confirm_threshold: i32,
    /// Mini-map and world map options (Firestorm MiniMap* / Map* settings).
    pub maps: MapSettings,
    /// "Search": last tab, rating boxes and land limits.
    pub search: crate::world::search::SearchSettings,
    /// Other panels open at exit (restored at startup) and the People tab.
    pub show_people: bool,
    pub people_tab: u8,
    pub show_inventory: bool,
    pub show_appearance: bool,
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
    /// Random stable machine identifier (hashed before being sent).
    pub machine_id: String,
    /// Legacy single-account authorization, read once for migration only.
    #[serde(skip_serializing)]
    pub mfa_hash: String,
    pub font: String,
    pub font_scale: f32,
    pub chat_timestamps: bool,
    /// Show Contacts in Conversations (Preferences > Chat, off by default).
    /// Unlike Firestorm's always-present entry, this is optional at the human's request.
    pub show_contacts_tab: bool,
    /// Tell the resident of a conversation that we are typing
    /// (FSSendTypingState, on by default; Preferences > Chat).
    pub send_typing_state: bool,
    pub chat_toast_seconds: f32,
    /// Toast when a friend comes online or goes offline (ChatOnlineNotification, on).
    pub toast_friends_online: bool,
    /// Where those toasts stand with Aurora's notification center.
    pub presence_position: PresencePosition,
    /// Toast of an IM or conference message out of sight (EnableIMChatPopups, on).
    pub toast_im: bool,
    /// Toast of a group chat message (EnableGroupChatPopups, off).
    pub toast_group_im: bool,
    /// Seconds a message toast stays (NotificationToastLifeTime, 30).
    pub im_toast_seconds: f32,
    /// Write instant messages, group chats and conferences to text files in
    /// the account's folder (Firestorm KeepConversationLogTranscripts = 2).
    pub log_im: bool,
    /// Write the nearby chat too (Firestorm LogNearbyChat, off).
    pub log_nearby: bool,
    /// Show the end of a conversation's file when it opens (LogShowHistory).
    pub log_show_history: bool,
    /// Date in the names of these files (LogFileNamewithDate, off).
    pub log_dated_names: bool,
    /// Ask before opening a web link outside the trusted domains (the box
    /// "Do not warn me again" of the warning turns it off).
    pub warn_external_links: bool,
    /// FSMuteAllGroups: refuse every group chat.
    pub mute_all_groups: bool,
    /// FSMuteGroupWhenNoticesDisabled: refuse group chat of groups whose
    /// notices are turned off.
    pub mute_groups_without_notices: bool,
    /// FSReportBlockToNearbyChat: say in nearby chat when the block list changes.
    pub report_blocks: bool,
    /// Automatic responses and the kept online status modes.
    pub autoresponse: crate::world::status::AutoResponse,
    /// The chat bar as a command line (FSCmdLine*, Preferences › Chat ›
    /// Commands).
    pub chat_commands: crate::cmdline::ChatCommandSettings,
    pub name_tag_distance: f32,
    /// Avatar name tags drawn as before the name plates (`ui::nameplate`):
    /// lines of text on a flat tag, in the tag colors of the preferences.
    pub classic_name_tags: bool,
    /// Complete name plates for friends only: the others stop at the
    /// compact pill (off by default).
    pub plates_friends_only_full: bool,
    /// A name plate under the pointer shows full while it stays there (on
    /// by default).
    pub plates_hover_expand: bool,
    pub plate_click: PlateClick,
    /// Profiles in Firestorm's tabbed window (Second Life, Feed, Favorites…)
    /// instead of the header and cards of `ui/profile/aurora.rs`; group
    /// profiles in the classic window too (`ui/group_profile.rs`).
    pub classic_profiles: bool,
    /// Firestorm's Pay window (floater_pay.xml) instead of `ui/pay.rs`.
    pub classic_pay_window: bool,
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
    /// Firestorm's traffic indicator (ShowNetStats, off by default): a
    /// small curve of the network in the system island.
    pub show_net_stats: bool,
    /// People › Near you as Firestorm's radar table (`ui/people/radar.rs`)
    /// instead of the rows grouped by range and the sheet of
    /// `ui/people/nearby.rs`.
    pub classic_people: bool,
    /// Firestorm's yellow region restart box (floater_region_restarting.xml)
    /// instead of the alert card of `ui/region_restart.rs`.
    pub classic_restart_alert: bool,
    /// The region restart alert (Preferences › Interface › Region restart).
    pub region_restart: crate::world::region_restart::Settings,
    /// "Reduce animations": the interface's pulses, shakes and spinners
    /// stand still (the region restart alert, for now).
    pub reduce_motion: bool,
    /// Preferences › Accessibility › Colour blindness: the interface's
    /// colours adapted to a colour vision deficiency (`crate::colorblind`),
    /// off by default.
    pub color_blindness: crate::colorblind::Deficiency,
    /// How strongly (0–1, 1 by default: the full correction).
    pub color_blindness_strength: f32,
    /// Firestorm's camera and movement windows (floater_camera.xml,
    /// floater_moveview.xml) instead of `ui/controls.rs`.
    pub classic_controls: bool,
    /// Camera positions saved from the Controls window (Firestorm's camera
    /// presets); kept apart from `camera`, which "Reset" sets back.
    pub camera_shots: Vec<crate::camera::CameraShot>,
    /// The lines of the Conversations window (`ui/chat/bubbles.rs` or the
    /// legacy lines).
    pub chat_style: ChatStyle,
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
    /// Region / Estate as Firestorm's eight tabs (not ported yet: the
    /// Aurora window opens meanwhile) instead of Aurora's (`ui/region`).
    pub classic_region_estate: bool,
    pub invert_mouse: bool,
    pub arrows_strafe: bool,
    /// The arrow keys (and Page Up / Down, Home) move the avatar while the
    /// chat bar has the keyboard, even with text in it; an empty bar always
    /// lets them through. Firestorm's `ArrowKeysAlwaysMove`, on by default.
    pub arrow_keys_always_move: bool,
    /// Letter keys move the avatar (WASD) instead of starting the nearby
    /// chat. Firestorm's `LetterKeysAffectsMovementNotFocusChatBar`, on by
    /// default.
    pub letter_keys_move: bool,
    /// What a double click on the ground does (Preferences › Controls, the
    /// wolf menu › Me): nothing by default, as in Firestorm.
    pub double_click: crate::autopilot::DoubleClick,
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
    /// 0 none, 1 FXAA, 2 MSAA 2x, 3 MSAA 4x, 4 MSAA 8x, 5 TAA, 6 SMAA (the
    /// default, RenderFSAAType 2 of Firestorm's High to Ultra levels in
    /// featuretable.txt)
    pub antialiasing: u8,
    /// 0 Khronos PBR Neutral, 1 ACES
    pub tonemapper: u8,
    /// Optional frame rate cap (on by default at 120 images per second).
    pub fps_cap: bool,
    pub fps_limit: u32,
    /// Lower cap while the window is not focused (on by default, as
    /// Firestorm yields BackgroundYieldTime every frame in the background).
    pub background_fps_cap: bool,
    pub background_fps_limit: u32,
    /// Maximum live particles (0 = particles off).
    pub max_particles: u32,
    /// Avatars drawn (the nearest ones).
    pub max_avatars: u32,
    /// Ambient occlusion: 0 off, 1 low, 2 high.
    pub ssao: u8,
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
    pub legacy_username_format: bool,
    pub trim_resident: bool,
    pub display_name_notices: bool,
    /// Screen-space reflections.
    pub ssr: bool,
    /// Anisotropic filtering: 1 (off), 2, 4, 8, 16.
    pub anisotropy: u16,
    /// Disk cache size (MB) for textures, meshes, animations, objects and inventory.
    pub cache_size_mb: u32,
    /// The most the simulators may send, all channels together, in kb/s
    /// (Firestorm's ThrottleBandwidthKBPS; the default and the top of the
    /// range are Aurora's, `aurora_net::throttle::DEFAULT_KBPS`).
    pub bandwidth_kbps: u32,
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

const SETTINGS_VERSION: u32 = 13;

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
    /// the theme's ink (the human's choice, 2026-10-14). Firestorm always
    /// colors them (LLVOAvatar::getNameTagColor).
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
            relation_names: false,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioSettings {
    /// Volume 0..1 per channel (`AUDIO_CHANNELS`).
    pub volume: [f32; 7],
    pub muted: [bool; 7],
    /// Channel enabled (checkbox; the master has none).
    pub enabled: [bool; 7],
    /// Play the parcel music automatically when entering a parcel
    /// (FSParcelMusicAutoPlay, off by default: the radio toggle starts it).
    pub music_autoplay: bool,
    /// Play parcel media automatically.
    pub media_autoplay: bool,
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
            volume: [0.8, 0.5, 0.5, 0.5, 0.4, 0.5, 0.7],
            muted: [false; 7],
            enabled: [true; 7],
            music_autoplay: false,
            media_autoplay: false,
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

impl Default for Settings {
    fn default() -> Self {
        Settings {
            update_mode: crate::updater::Mode::Ask,
            ignored_update: String::new(),
            language: "default".into(),
            language_is_public: true,
            show_bug_report: true,
            show_huds: true,
            sun_beacon: false,
            moon_beacon: false,
            debug: DebugView::default(),
            build: Default::default(),
            camera: Default::default(),
            auto_hover_height: true,
            auto_hover_offset: 0.0,
            contacts: Default::default(),
            radar: Default::default(),
            friends_list: Default::default(),
            inventory: Default::default(),
            texture_library: Default::default(),
            gestures: Default::default(),
            shape_show_hints: true,
            shape_height_feet: false,
            imports: Default::default(),
            script_editor: Default::default(),
            username: String::new(),
            remember_username: true,
            remember_password: false,
            grid: GridChoice::SecondLife,
            custom_login_uri: "http://127.0.0.1:9000/".into(),
            start_location: "last".into(),
            start_region: String::new(),
            draw_distance: 96.0,
            vsync: false,
            shadows: true,
            force_midday: false,
            time_of_day: 0,
            env_manual_transition: 0.0,
            env_persist: true,
            exposure: 1.0,
            sharpen: 0.4,
            glow: true,
            occlusion_culling: true,
            lod_factor: 2.0,
            texture_budget_auto: true,
            texture_budget_mb: 1536,
            ui_scale: 1.0,
            use_pie_menu: false,
            snap_windows: true,
            // First launch (no settings file): the radar (People on its
            // first tab, top left), the mini-map (top right), Performance
            // under it and Conversations (bottom right) are open, as asked
            // by the human; the other windows wait for their button. A
            // saved file keeps what was open at exit.
            show_perf: true,
            show_chat: true,
            show_minimap: true,
            standalone_place_details: false,
            inspect_columns: crate::ui::inspect::ALL_COLUMNS,
            landmarks_by_date: true,
            confirm_teleports: true,
            confirm_no_transfer_settings: true,
            confirm_object_bonus: true,
            region_check_texture_heights: true,
            confirm_payments: true,
            payment_confirm_threshold: 200,
            maps: MapSettings::default(),
            search: Default::default(),
            show_people: true,
            people_tab: 0,
            show_inventory: false,
            show_appearance: false,
            show_ao: false,
            ao: Default::default(),
            snapshot: Default::default(),
            dof: Default::default(),
            loading_backdrop: true,
            mouse_sensitivity: 1.0,
            window_width: 1600,
            window_height: 900,
            window_x: None,
            window_y: None,
            window_maximized: false,
            machine_id: uuid::Uuid::new_v4().to_string(),
            mfa_hash: String::new(),
            font: "Noto Sans".into(),
            font_scale: 1.0,
            chat_timestamps: true,
            show_contacts_tab: false,
            send_typing_state: true,
            chat_toast_seconds: 20.0,
            toast_friends_online: true,
            presence_position: PresencePosition::TopRight,
            toast_im: true,
            toast_group_im: false,
            im_toast_seconds: 30.0,
            log_im: true,
            log_nearby: false,
            log_show_history: true,
            log_dated_names: false,
            warn_external_links: true,
            mute_all_groups: false,
            mute_groups_without_notices: false,
            report_blocks: false,
            autoresponse: Default::default(),
            chat_commands: Default::default(),
            name_tag_distance: 64.0,
            classic_name_tags: false,
            classic_profiles: false,
            classic_pay_window: false,
            classic_bars: false,
            place_bar_stretch: true,
            place_bar_rights_text: false,
            favorites_access: FavoritesAccess::Menu,
            favorites_row_folded: false,
            system_island: SystemIsland::default(),
            confirm_hide_interface: true,
            show_net_stats: false,
            classic_people: false,
            classic_restart_alert: false,
            region_restart: Default::default(),
            reduce_motion: false,
            color_blindness: crate::colorblind::Deficiency::Off,
            color_blindness_strength: 1.0,
            classic_controls: false,
            camera_shots: Vec::new(),
            plates_friends_only_full: false,
            plates_hover_expand: true,
            plate_click: PlateClick::Nothing,
            chat_style: ChatStyle::Aurora,
            notifications_style: NotificationsStyle::Aurora,
            world_map_style: WorldMapStyle::Aurora,
            photo_window: PhotoWindowStyle::Aurora,
            photo_held: None,
            classic_about_land: false,
            classic_region_estate: false,
            invert_mouse: false,
            arrows_strafe: false,
            arrow_keys_always_move: true,
            letter_keys_move: true,
            double_click: crate::autopilot::DoubleClick::Nothing,
            lookat: Default::default(),
            former_look_at: None,
            former_look_at_range: None,
            shadow_quality: 3,
            projector_shadows: true,
            antialiasing: 6,
            tonemapper: 0,
            fps_cap: true,
            fps_limit: 120,
            background_fps_cap: true,
            background_fps_limit: 15,
            max_particles: 4096,
            max_avatars: 16,
            ssao: 1,
            water_reflections: 1,
            reflection_probes: 3,
            probe_count: 32,
            max_complexity: 350_000,
            max_attachment_area: 1000.0,
            loading_avatars: 0,
            complexity_mode: 0,
            render_exceptions: Vec::new(),
            tag_complexity: true,
            tag_complexity_too_complex_only: true,
            tag_complexity_own: false,
            use_display_names: true,
            show_usernames: true,
            legacy_username_format: true,
            trim_resident: true,
            display_name_notices: true,
            mirrors: 1,
            ssr: false,
            anisotropy: 8,
            cache_size_mb: 20480,
            bandwidth_kbps: aurora_net::throttle::DEFAULT_KBPS as u32,
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
    for_mode(
        directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("cache")),
    )
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

    fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Settings {
        Self::load_from(&Self::path())
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

    fn sanitized(mut self) -> Settings {
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
        self.camera = std::mem::take(&mut self.camera).sanitized();
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
        if !self.audio.mic_gain.is_finite() {
            self.audio.mic_gain = 1.0;
        }
        self.audio.mic_gain = self.audio.mic_gain.clamp(0.0, 2.0);
        self.time_of_day = self.time_of_day.min(4);
        self.env_manual_transition = if self.env_manual_transition.is_finite() {
            self.env_manual_transition.clamp(0.0, 60.0)
        } else {
            0.0
        };
        self.settings_version = SETTINGS_VERSION;
        self.max_particles = self.max_particles.min(8192);
        self.max_avatars = self.max_avatars.clamp(1, 100);
        self.complexity_mode = self.complexity_mode.min(2);
        self.loading_avatars = self.loading_avatars.min(2);
        self.max_attachment_area = if self.max_attachment_area.is_finite() {
            self.max_attachment_area.clamp(0.0, 100_000.0)
        } else {
            1000.0
        };
        self.render_exceptions
            .retain(|(id, m)| uuid::Uuid::parse_str(id).is_ok() && (1..=2).contains(m));
        self.ssao = self.ssao.min(2);
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
        self.sharpen = if self.sharpen.is_finite() {
            self.sharpen.clamp(0.0, 1.0)
        } else {
            0.4
        };
        self.lod_factor = self.lod_factor.clamp(0.5, 4.0);
        self.ui_scale = self.ui_scale.clamp(0.75, 2.0);
        self.mouse_sensitivity = self.mouse_sensitivity.clamp(0.1, 5.0);
        self.texture_budget_mb = self.texture_budget_mb.clamp(256, 32768);
        self.font_scale = self.font_scale.clamp(0.8, 1.4);
        self.shadow_quality = self.shadow_quality.min(4);
        self.antialiasing = self.antialiasing.min(6);
        self.reflection_probes = self.reflection_probes.min(3);
        self.probe_count = self.probe_count.clamp(8, 64);
        self.tonemapper = self.tonemapper.min(1);
        self.chat_toast_seconds = self.chat_toast_seconds.clamp(0.0, 60.0);
        self.im_toast_seconds = self.im_toast_seconds.clamp(3.0, 60.0);
        self.name_tag_distance = self.name_tag_distance.clamp(0.0, 128.0);
        if uuid::Uuid::parse_str(&self.machine_id).is_err() {
            self.machine_id = uuid::Uuid::new_v4().to_string();
        }
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
            start_location: old.start_location,
            start_region: old.start_region,
            window_width: old.window_width,
            window_height: old.window_height,
            window_x: old.window_x,
            window_y: old.window_y,
            window_maximized: old.window_maximized,
            // a newer build's own values are not this build's to reset
            newer_file: old.newer_file,
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
        use aurora_render::{AntiAliasing, Tonemapper};
        aurora_render::RenderSettings {
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
            shadow_resolution: self.shadow_params().0,
            ssao: self.ssao,
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

    /// Hashed identifiers sent at login (`mac`, `id0`).
    pub fn hashed_ids(&self) -> (String, String) {
        let mac = format!("{:x}", md5::compute(format!("mac:{}", self.machine_id)));
        let id0 = format!("{:x}", md5::compute(format!("id0:{}", self.machine_id)));
        (mac, id0)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn contacts_tab_is_hidden_by_default_and_survives_reload() {
        assert!(!Settings::default().show_contacts_tab);
        let old: Settings = serde_json::from_str("{}").expect("synthetic old settings");
        assert!(!old.show_contacts_tab);
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
    fn names_by_relation_stay_colored_for_classic_tag_users_only() {
        assert!(!Settings::default().sanitized().colors.relation_names);
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
            assert_eq!(old.sanitized().colors.relation_names, classic);
        }
        // changed after the migration: kept
        let mut on = Settings::default();
        on.colors.relation_names = true;
        assert!(on.sanitized().colors.relation_names);
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
