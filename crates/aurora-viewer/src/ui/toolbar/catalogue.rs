//! The commands of the "Toolbar buttons" window: Firestorm's toybox
//! (indra/newview/app_settings/commands.xml, the 67 commands with
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
    ($( $id:literal, $icon:literal, $key:literal, $ready:literal; )*) => {
        &[$(Command { id: $id, icon: $icon, key: $key, ready: $ready },)*]
    };
}

/// Every command, in no particular order (the window sorts them by their
/// label in the interface's language, like LLFloaterToybox::postBuild).
pub const COMMANDS: &[Command] = commands![
    // Aurora's toolbar before PR 6
    "conversations", "Command_Chat_Icon", "common-conversations", true;
    "speak", "Command_Speak_Icon", "ui-skin-speak", true;
    "move", "Command_Move_Icon", "controls-movement-title", true;
    "view", "video-filled", "controls-camera-title", true;
    "mouselook", "Command_View_Icon", "common-mouselook-m", true;
    "people", "Command_People_Icon", "common-people", true;
    "friends", "Command_People_Icon", "common-friends", true;
    "inventory", "Command_Inventory_Icon", "common-inventory", true;
    "texture_library", "stack-2-filled", "texture-library-title", true;
    "gestures", "Command_Gestures_Icon", "gesture-title", true;
    "appearance", "Command_Appearance_Icon", "common-appearance", true;
    "search", "Command_Search_Icon", "search-title", true;
    "places", "Command_Places_Icon", "common-places", true;
    "snapshot", "Command_Snapshot_Icon", "photo-title", true;
    "worldmap", "Command_Map_Icon", "common-world-map", true;
    "minimap", "Command_MiniMap_Icon", "common-mini-map", true;
    "performance", "Command_Performance_Icon", "common-performance", true;
    "sit", "groundsit", "extra-sit-on-ground-stand-up", true;
    "ao", "AO", "ao-title", true;
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
    "radar", "radar", "toolbar-command-radar", true;
    "block_list", "ban", "toolbar-command-block-list", true;
    "contact_sets", "users-group", "toolbar-command-contact-sets", true;
    "stop_animations", "player-stop-filled", "ui-context-stop-animations", true;
    // Aurora's own: the interface hidden but the chat (#859)
    "immersive", "maximize", "ui-bars-immersive", true;
    // Firestorm's, not in Aurora yet
    "avatar", "user-circle", "toolbar-command-avatar", false;
    "destinations", "map-2", "toolbar-command-destinations", false;
    "howto", "Command_HowTo_Icon", "toolbar-command-howto", false;
    "picks", "heart-filled", "toolbar-command-picks", false;
    "poser", "wand", "toolbar-command-poser", true;
    "flickr", "brand-flickr", "toolbar-command-flickr", false;
    "primfeed", "rss", "toolbar-command-primfeed", false;
    "RFO", "user-check", "toolbar-command-friends-only", false;
    "gridstatus", "building-broadcast-tower-filled", "toolbar-command-grid-status", true;
    "voice", "Command_Voice_Icon", "toolbar-command-voice", false;
    "quickprefs", "adjustments-filled", "toolbar-command-quickprefs", false;
    "webbrowser", "world-filled", "toolbar-command-web-browser", false;
    "defaultchatbar", "message-filled", "toolbar-command-chat-bar", false;
    "areasearch", "zoom-scan-filled", "toolbar-command-area-search", true;
    "settings_debug", "terminal-2", "toolbar-command-debug-settings", false;
    "statistics", "statistics", "toolbar-command-statistics", false;
    "soundexplorer", "music", "toolbar-command-sound-explorer", true;
    "assetblacklist", "shield-off", "toolbar-command-asset-blacklist", true;
    "phototools", "camera-cog", "toolbar-command-phototools", false;
    "phototools_view", "video-filled", "toolbar-command-phototools-camera", false;
    "pose_stand", "man-filled", "toolbar-command-pose-stand", false;
    "snapshot_to_disk", "device-floppy-filled", "toolbar-command-snapshot-to-disk", false;
    "conversation_log", "file-text-filled", "toolbar-command-conversation-log", false;
    "movelock", "lock-filled", "movelock-title", true;
    "water_walk", "walk", "movement-water-walk", true;
    "resync_animations", "refresh", "toolbar-command-resync-animations", true;
    "regiontracker", "location-filled", "toolbar-command-region-tracker", false;
    "group_titles", "tags-filled", "toolbar-command-group-titles", false;
    "wearable_favorites", "hanger-2-filled", "toolbar-command-wearable-favorites", false;
    "beacons", "bulb-filled", "toolbar-command-beacons", false;
    "discord", "brand-discord-filled", "toolbar-command-discord", false;
    "360capture", "panorama-horizontal-filled", "toolbar-command-360-snapshot", false;
    "omnifilter", "filter-filled", "toolbar-command-omnifilter", false;
    "avatar_align", "user-scan", "toolbar-command-avatar-align", false;
    "face_nearest_avatar", "user-search", "toolbar-command-face-nearest", false;
];

impl Command {
    pub fn label(&self) -> &'static str {
        aurora_i18n::text(self.key)
    }

    pub fn label_in(&self, language: &str) -> &'static str {
        aurora_i18n::text_in(language, self.key)
    }
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
