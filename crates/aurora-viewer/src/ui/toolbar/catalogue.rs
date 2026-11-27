//! The commands of the "Toolbar buttons" window: Firestorm's toybox
//! (indra/newview/app_settings/commands.xml, the commands with
//! `available_in_toybox`, originally LGPL 2.1) and Aurora's own (friends,
//! home, favourites, immersive mode). Aurora keeps its command names where it had one
//! (`conversations` for Firestorm's `chat`, `worldmap` for `map`…), so that
//! saved toolbars stay valid; the others take Firestorm's. A command Aurora
//! cannot run yet stays in the list, dimmed, with "Not available in Aurora
//! yet" (the human's decision of 2026-10-15).

/// One command of the catalogue.
pub struct Command {
    pub id: &'static str,
    /// Tabler icon, or one of the skin's aliases.
    pub icon: &'static str,
    /// The i18n key of its label.
    pub key: &'static str,
    /// Aurora runs it (`bars::run_command`).
    pub ready: bool,
}

macro_rules! commands {
    ($( $id:literal, $icon:literal, $key:literal, $ready:expr; )*) => {
        &[$(Command { id: $id, icon: $icon, key: $key, ready: $ready },)*]
    };
}

/// Every command, in no particular order (the window sorts them by their
/// label in the interface's language, like LLFloaterToybox::postBuild).
pub const COMMANDS: &[Command] = commands![
    "toolbar_separator", "arrows-horizontal", "toolbar-spacer", true;
    "toolbar_divider", "separator-vertical", "toolbar-separator", true;
    // a new floating block (human request, 2026-11-25; ui/toolbar/floating.rs)
    "toolbar_floating", "app-window-filled", "toolbar-floating-new", true;
    // the presets in a drawer (human request, 2026-11-26; drawers::INTERFACES)
    "drawer:interfaces", "layout-grid-filled", "toolbar-interfaces", true;
    "bar_navigation", "map-pin-filled", "toolbar-block-navigation", true;
    "bar_account", "user-filled", "toolbar-block-account", true;
    "bar_system", "gauge-filled", "toolbar-block-system", true;
    "bar_status", "user-filled", "toolbar-block-status", true;
    "bar_balance", "coin-filled", "toolbar-block-balance", true;
    "bar_clock", "clock-filled", "toolbar-block-clock", true;
    "bar_ping", "antenna-bars-5", "toolbar-block-ping", true;
    "bar_fps", "gauge-filled", "toolbar-block-fps", true;
    "bar_network", "arrows-down-up", "toolbar-block-network", true;
    "bar_media", "brand-youtube-filled", "toolbar-block-media", true;
    "bar_music", "radio", "toolbar-block-music", true;
    "bar_volume", "volume", "toolbar-block-volume", true;
    "bar_notifications", "bell-filled", "toolbar-block-notifications", true;
    "bar_conversations", "messages-filled", "toolbar-block-conversations", true;
    "bar_chat", "message-filled", "toolbar-block-chat", true;
    "report_bug", "bug-filled", "bug-title", true;
    "import_texture", "photo-filled", "toolbar-import-textures", true;
    "import_sound", "file-music-filled", "toolbar-import-sounds", true;
    "import_animation", "run", "toolbar-import-animations", true;
    "import_material", "droplet-filled", "toolbar-import-materials", true;
    "import_mesh", "cube-3d-sphere", "toolbar-import-meshes", true;
    "import_local", "eye-filled", "toolbar-import-local", true;
    // the quick import bar's Folder and Settings as buttons (human
    // request, 2026-11-25), with the bar's icons (imports/ui/launcher.rs)
    "import_advanced", "adjustments-filled", "toolbar-import-advanced", true;
    "import_settings", "settings-filled", "toolbar-import-settings", true;
    "location_bar", "map-pin-filled", "toolbar-location-bar", true;
    "aurora_menu", "wolf", "ui-bars-aurora-menu", true;
    // Aurora's toolbar before PR 6
    "conversations", "Command_Chat_Icon", "common-conversations", true;
    "speak", "Command_Speak_Icon", "ui-skin-speak", true;
    "move", "Command_Move_Icon", "controls-movement-title", true;
    "view", "video-filled", "controls-camera-title", true;
    "vr_view", "device-vision-pro-filled", "vr-view", cfg!(any(target_os = "linux", target_os = "windows", target_os = "macos"));
    "mouselook", "Command_View_Icon", "common-mouselook-m", true;
    "people", "Command_People_Icon", "common-people", true;
    "friends", "Command_People_Icon", "common-friends", true;
    "inventory", "Command_Inventory_Icon", "common-inventory", true;
    "inventory_dual", "columns-2-filled", "inventory-dual-panel", true;
    "texture_library", "stack-2-filled", "texture-library-title", true;
    "gestures", "Command_Gestures_Icon", "gesture-title", true;
    "appearance", "Command_Appearance_Icon", "common-appearance", true;
    "avatar_studio", "man-filled", "avatar-studio-title", true;
    "search", "Command_Search_Icon", "search-title", true;
    "places", "Command_Places_Icon", "common-places", true;
    "snapshot", "Command_Snapshot_Icon", "photo-title", true;
    "worldmap", "Command_Map_Icon", "common-world-map", true;
    "minimap", "Command_MiniMap_Icon", "common-mini-map", true;
    "performance", "Command_Performance_Icon", "common-performance", true;
    "sit", "groundsit", "extra-sit-on-ground-stand-up", true;
    "ao", "AO", "ao-title", true;
    "ao_mini", "app-window-filled", "toolbar-command-ao-mini", true;
    "movement", "swimming", "movement-swimming", true;
    "preferences", "Command_Preferences_Icon", "common-preferences", true;
    "fly", "plane-tilt-filled", "extra-fly-land-f", true;
    "home", "home-filled", "common-go-home", true;
    "favorites", "bookmarks-filled", "common-favorites", true;
    // Firestorm's, on what Aurora already does
    "aboutland", "info-circle-filled", "common-about-land", true;
    "build", "Command_Build_Icon", "common-build", true;
    "profile", "user-filled", "toolbar-command-profile", true;
    "reporter", "flag-filled", "report-abuse-title", true;
    "regioninfo", "building-castle", "region-title", true;
    "teleport_history", "history", "common-teleport-history", true;
    "landmark_here", "star-filled", "toolbar-command-landmark-here", true;
    "marketplacelistings", "tag-filled", "marketplace-listings-menu", true;
    "myenvironments", "Command_Environments_Icon", "toolbar-command-environments", true;
    "constellations", "sparkles-filled", "ui-bars-constellations", true;
    "radar", "radar", "toolbar-command-radar", true;
    "block_list", "ban", "toolbar-command-block-list", true;
    "contact_sets", "users-group", "toolbar-command-contact-sets", true;
    "stop_animations", "player-stop-filled", "ui-context-stop-animations", true;
    "snapshot_to_disk", "device-floppy-filled", "toolbar-command-snapshot-to-disk", true;
    "beacons", "bulb-filled", "toolbar-command-beacons", true;
    // the Legacy Viewer's Compass and its Face nearest avatar (FSAvatarAlignBase)
    "avatar_align", "user-scan", "toolbar-command-avatar-align", true;
    "face_nearest_avatar", "user-search", "toolbar-command-face-nearest", true;
    // Aurora's own: the interface hidden but the chat (#859)
    "immersive", "maximize", "ui-bars-immersive", true;
    "name_tags", "label-filled", "ui-bars-immersive-hide-tags", true;
    // Aurora's own: Show User Interface and Show HUDs as buttons (the Legacy
    // Viewer has them in its menus only)
    "hide_interface", "eye-off", "ui-bars-hide-interface", true;
    "huds", "device-desktop-filled", "ui-bars-immersive-hide-huds", true;
    // Aurora's own: the photo reel's quick view, the session's photos
    "photo_roll", "carousel-horizontal-filled", "toolbar-command-photo-roll", true;
    // Build and Me menu commands that can be added directly to a toolbar.
    "ik_edit", "bone-filled", "ik-edit-title", true;
    // the build window's five tools, "Build - " and its tool's name, with
    // the window's icons (build/ui/layout.rs)
    "build_focus", "zoom-filled", "toolbar-command-build-tool-focus", true;
    "build_grab", "hand-grab", "toolbar-command-build-tool-grab", true;
    "build_edit", "pointer-filled", "toolbar-command-build-tool-edit", true;
    "build_create", "wand", "toolbar-command-build-tool-create", true;
    "build_land", "mountain-filled", "toolbar-command-build-tool-land", true;
    "edit_particles", "sparkles-filled", "ui-bars-edit-particles", true;
    // the quick import bar (human request, 2026-11-25: "Import bar")
    "import", "file-upload-filled", "import-menu", true;
    "blender_live", "cube-3d-sphere", "blender-live-title", true;
    "buy_currency", "coin-filled", "currency-menu", true;
    "money_tracker", "receipt-filled", "money-tracker-title", true;
    "experiences", "flask-filled", "ui-experiences-title", true;
    "stand_up", "man-filled", "common-stand-up", true;
    "fly_override", "plane-departure-filled", "ui-bars-fly-override", true;
    "movelock_options", "lock-cog", "movelock-options", true;
    "quickjump", "arrow-big-up-lines-filled", "ui-bars-quickjump", true;
    "movement_enhancer", "steering-wheel-filled", "movement-title", true;
    // Aurora's own: the Director, a camera track to film with
    "director", "movie", "toolbar-command-director", true;
    // Aurora's own: our avatar seen by a camera of its own
    "second_camera", "device-cctv-filled", "second-camera-title", true;
    // Aurora's own: the viewer steps back for a game (app/gaming_mode.rs)
    "gaming_mode", "device-gamepad-2-filled", "toolbar-command-gaming-mode", true;
    // Firestorm's, not in Aurora yet
    "avatar", "user-circle", "toolbar-command-avatar", true;
    "destinations", "map-2", "toolbar-command-destinations", true;
    "picks", "heart-filled", "toolbar-command-picks", true;
    "poser", "wand", "toolbar-command-poser", true;
    "flickr", "brand-flickr", "toolbar-command-flickr", false;
    "primfeed", "rss", "toolbar-command-primfeed", false;
    "RFO", "user-check", "toolbar-command-friends-only", true;
    "gridstatus", "building-broadcast-tower-filled", "toolbar-command-grid-status", true;
    "voice", "Command_Voice_Icon", "toolbar-command-voice", true;
    "quickprefs", "adjustments-filled", "toolbar-command-quickprefs", true;
    "graphics_preferences", "settings-filled", "ui-bars-graphics-preferences", true;
    "webbrowser", "world-filled", "toolbar-command-web-browser", true;
    "defaultchatbar", "message-filled", "toolbar-command-chat-bar", true;
    "areasearch", "zoom-scan-filled", "toolbar-command-area-search", true;
    "settings_debug", "terminal-2", "toolbar-command-debug-settings", true;
    "statistics", "statistics", "toolbar-command-statistics", true;
    "soundexplorer", "music", "toolbar-command-sound-explorer", true;
    "assetblacklist", "shield-off", "toolbar-command-asset-blacklist", true;
    "phototools", "camera-cog", "toolbar-command-phototools", true;
    "phototools_view", "video-filled", "toolbar-command-phototools-camera", false;
    "pose_stand", "man-filled", "toolbar-command-pose-stand", true;
    "conversation_log", "file-text-filled", "toolbar-command-conversation-log", true;
    "movelock", "lock-filled", "movelock-title", true;
    "water_walk", "walk", "movement-water-walk", true;
    "resync_animations", "refresh", "toolbar-command-resync-animations", true;
    "regiontracker", "location-filled", "toolbar-command-region-tracker", true;
    "group_titles", "tags-filled", "toolbar-command-group-titles", true;
    "wearable_favorites", "hanger-2-filled", "toolbar-command-wearable-favorites", true;
    "discord", "brand-discord-filled", "toolbar-command-discord", false;
    "360capture", "panorama-horizontal-filled", "toolbar-command-360-snapshot", true;
    "omnifilter", "filter-filled", "toolbar-command-omnifilter", true;
];

impl Command {
    pub fn label(&self) -> &'static str {
        aurora_i18n::text(self.key)
    }

    pub fn label_in(&self, language: &str) -> &'static str {
        aurora_i18n::text_in(language, self.key)
    }
}

/// Commands the Legacy Viewer dropped: only stale entries of its skins'
/// toolbars.xml name them, and it skips them. A saved layout (older Aurora,
/// imported) that still has one drops it, with no empty button.
/// "Customize interface" alone, and its main menu entry's copy, are no
/// button any more: the Interfaces block ends with it (human request,
/// 2026-11-26; `drawers::migrate_interfaces` turns one on a bar into it).
const RETIRED: &[&str] = &["howto", "edit_interface", "menu:toolbar-interface-edit"];

/// Whether `id` names a command that no longer exists.
pub fn retired(id: &str) -> bool {
    RETIRED.contains(&id)
}

/// The catalogue's entry of a command.
pub fn find(id: &str) -> Option<&'static Command> {
    // Preserve saved Firestorm-command layouts without listing two lock buttons.
    let id = if id == "move_lock" { "movelock" } else { id };
    COMMANDS.iter().find(|c| c.id == id)
}

/// Whether Aurora runs `id`: every command a skin brings is taken as one.
pub fn ready(id: &str) -> bool {
    find(id).is_none_or(|c| c.ready)
}
