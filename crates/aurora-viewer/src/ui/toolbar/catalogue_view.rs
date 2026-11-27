//! The editor's categorized catalogue. Aliases merge known actions, never
//! translated labels: two equally named menu entries can do different things.

use super::{ToolbarSettings, catalogue, commands, label, sort_key};
use crate::ui::skin::{Layout, ToolButton};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Category {
    Avatar,
    Communication,
    World,
    Creation,
    Interface,
    Advanced,
    Help,
    Custom,
}

impl Category {
    pub const ALL: [Self; 8] = [
        Self::Avatar,
        Self::Communication,
        Self::World,
        Self::Creation,
        Self::Interface,
        Self::Advanced,
        Self::Help,
        Self::Custom,
    ];

    pub fn label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Avatar => "toolbar-category-avatar",
            Self::Communication => "toolbar-category-communication",
            Self::World => "toolbar-category-world",
            Self::Creation => "toolbar-category-creation",
            Self::Interface => "toolbar-category-interface",
            Self::Advanced => "toolbar-category-advanced",
            Self::Help => "toolbar-category-help",
            Self::Custom => "toolbar-category-custom",
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Avatar => "user-filled",
            Self::Communication => "messages-filled",
            Self::World => "world-filled",
            Self::Creation => "box",
            Self::Interface => "layout-grid-filled",
            Self::Advanced => "adjustments-filled",
            Self::Help => "help-circle-filled",
            Self::Custom => "settings-filled",
        }
    }
}

pub(super) fn category(command: &str) -> Category {
    use Category::*;
    if super::drawers::is_drawer(command) {
        return Interface;
    }
    if super::presets::import_choice(command).is_some() {
        return Creation;
    }
    if let Some(path) = command.strip_prefix(commands::PREFIX) {
        return match path.split('/').next().unwrap_or_default() {
            "common-me" => Avatar,
            "ui-bars-communicate" => Communication,
            "common-world" => World,
            "common-build" | "common-contents" => Creation,
            "common-advanced" => Advanced,
            "common-help" => Help,
            _ => Custom,
        };
    }
    match command {
        "move"
        | "mouselook"
        | "fly"
        | "sit"
        | "ao"
        | "movement"
        | "movelock"
        | "move_lock"
        | "water_walk"
        | "resync_animations"
        | "stop_animations"
        | "appearance"
        | "profile"
        | "avatar"
        | "avatar_studio"
        | "picks"
        | "poser"
        | "pose_stand"
        | "wearable_favorites"
        | "avatar_align"
        | "face_nearest_avatar"
        | "ao_mini"
        | "stand_up"
        | "fly_override"
        | "quickjump"
        | "movelock_options"
        | "movement_enhancer"
        | "buy_currency"
        | "money_tracker"
        | "experiences"
        | "gestures" => Avatar,
        "conversations" | "speak" | "people" | "friends" | "contact_sets" | "block_list" | "radar" | "group_titles" | "voice"
        | "defaultchatbar" | "bar_chat" | "conversation_log" | "discord" => Communication,
        "home" | "location_bar" | "places" | "favorites" | "worldmap" | "minimap" | "aboutland" | "regioninfo" | "teleport_history"
        | "landmark_here" | "search" | "destinations" | "regiontracker" | "areasearch" | "webbrowser" | "constellations" => World,
        "inventory"
        | "texture_library"
        | "build"
        | "marketplacelistings"
        | "myenvironments"
        | "snapshot"
        | "snapshot_to_disk"
        | "photo_roll"
        | "flickr"
        | "primfeed"
        | "phototools"
        | "phototools_view"
        | "360capture"
        | "director"
        | "second_camera"
        | "inventory_dual"
        | "build_focus"
        | "build_grab"
        | "build_edit"
        | "build_create"
        | "build_land"
        | "edit_particles"
        | "ik_edit"
        | "import"
        | "import_advanced"
        | "import_settings"
        | "blender_live" => Creation,
        "aurora_menu"
        | "preferences"
        | "graphics_preferences"
        | "quickprefs"
        | "immersive"
        | "view"
        | "vr_view"
        | "name_tags"
        | "hide_interface"
        | "huds"
        | "gaming_mode"
        | "toolbar_separator"
        | "toolbar_divider"
        | "toolbar_floating"
        | "bar_navigation"
        | "bar_account"
        | "bar_system"
        | "bar_notifications"
        | "bar_conversations"
        | "bar_status"
        | "bar_balance"
        | "bar_clock"
        | "bar_ping"
        | "bar_fps"
        | "bar_network"
        | "bar_media"
        | "bar_music"
        | "bar_volume" => Interface,
        "performance" | "beacons" | "settings_debug" | "statistics" | "soundexplorer" | "assetblacklist" | "omnifilter" | "RFO" => Advanced,
        "report_bug" | "reporter" | "gridstatus" | "howto" => Help,
        _ => Custom,
    }
}

/// These pairs are the same feature in `main_menus` and `run_command`.
/// Their original IDs, handlers and settings remain intact outside the editor.
pub(super) fn alias(command: &str) -> Option<&'static str> {
    Some(match command {
        "menu:common-world/common-about-land" => "aboutland",
        "menu:common-world/region-title" => "regioninfo",
        "menu:common-world/common-places" => "places",
        "menu:common-world/common-mini-map" => "minimap",
        "menu:common-world/common-world-map" => "worldmap",
        "menu:common-world/search-title" => "search",
        "menu:common-world/toolbar-command-area-search" => "areasearch",
        "menu:common-world/toolbar-command-region-tracker" => "regiontracker",
        "menu:common-world/toolbar-command-destinations" => "destinations",
        "menu:common-build/avatar-studio-title" => "avatar_studio",
        "menu:common-me/toolbar-command-avatar-align" => "avatar_align",
        "menu:common-me/toolbar-command-face-nearest" => "face_nearest_avatar",
        "menu:common-world/toolbar-command-resync-animations" => "resync_animations",
        "menu:common-contents/common-inventory" => "inventory",
        "menu:common-contents/gesture-title" | "menu:ui-bars-communicate/gesture-title" => "gestures",
        "menu:ui-bars-communicate/common-conversations" => "conversations",
        "menu:common-me/ui-bars-appearance" => "appearance",
        "menu:common-me/ui-bars-profile" => "profile",
        "menu:common-me/ui-bars-preferences" => "preferences",
        "menu:common-me/common-go-home" => "home",
        "menu:common-me/fly" => "fly",
        "menu:common-me/movelock-title" => "movelock",
        "menu:common-me/marketplace-listings-menu" => "marketplacelistings",
        "menu:common-me/currency-menu" => "buy_currency",
        "menu:common-me/tracker" => "money_tracker",
        "menu:common-me/ui-experiences-title" => "experiences",
        "menu:common-me/common-stand-up" => "stand_up",
        "menu:common-me/fly_override" => "fly_override",
        "menu:common-me/movelock-options" => "movelock_options",
        "menu:common-me/quickjump" => "quickjump",
        "menu:common-me/movement-title" => "movement_enhancer",
        "menu:common-build/entry-box" => "build",
        "menu:common-build/entry-pencil-filled" => "build_edit",
        "menu:common-build/entry-circle-plus-filled" => "build_create",
        "menu:common-build/entry-mountain-filled" => "build_land",
        "menu:common-build/ik-edit-title" => "ik_edit",
        "menu:common-build/ui-bars-edit-particles" => "edit_particles",
        // an entry of earlier versions, kept for the bars that saved it
        "menu:common-build/import-menu" => "import",
        "menu:common-build/import-types/import-menu-textures" => "import_texture",
        "menu:common-build/import-types/import-menu-animations" => "import_animation",
        "menu:common-build/import-types/import-menu-sounds" => "import_sound",
        "menu:common-build/import-types/import-menu-materials" => "import_material",
        "menu:common-build/import-types/import-menu-meshes" => "import_mesh",
        "menu:common-build/import-types/toolbar-import-advanced" => "import_advanced",
        "menu:common-build/import-types/toolbar-import-local" => "import_local",
        "menu:common-build/import-types/import-menu-options" => "import_settings",
        "menu:common-build/blender-live-title" => "blender_live",
        "menu:common-build/poser-title" => "poser",
        "menu:common-help/bug-title" => "report_bug",
        "menu:common-help/report-abuse-title" => "reporter",
        "menu:common-help/grid_status" => "gridstatus",
        "menu:common-advanced/debug-settings" => "settings_debug",
        "menu:common-advanced/graphics" => "graphics_preferences",
        _ => return None,
    })
}

pub(crate) struct Tile {
    pub button: ToolButton,
    pub category: Category,
    pub group: bool,
}

/// Keep the first visit focused on everyday controls. Remaining native menu
/// entries and specialist tools are still available in Advanced and global search.
fn primary(command: &str) -> bool {
    matches!(
        alias(command).unwrap_or(command),
        "conversations"
            | "speak"
            | "move"
            | "view"
            | "people"
            | "friends"
            | "inventory"
            | "gestures"
            | "appearance"
            | "search"
            | "places"
            | "snapshot"
            | "worldmap"
            | "minimap"
            | "sit"
            | "ao"
            | "preferences"
            | "fly"
            | "home"
            | "favorites"
            | "aboutland"
            | "build"
            | "profile"
            | "teleport_history"
            | "myenvironments"
            | "radar"
            | "contact_sets"
            | "defaultchatbar"
            | "bar_chat"
            | "immersive"
            | "location_bar"
            | "aurora_menu"
            | "report_bug"
            | "performance"
            | "voice"
            | "group_titles"
            | "landmark_here"
            | "toolbar_divider"
            | "bar_navigation"
            | "bar_status"
            | "bar_balance"
            | "bar_clock"
            | "bar_ping"
            | "bar_fps"
            | "bar_network"
            | "bar_media"
            | "bar_music"
            | "bar_volume"
            | "bar_notifications"
            | "bar_conversations"
    )
}

impl Tile {
    pub fn in_view(&self, advanced: bool, filter: &str) -> bool {
        self.matches(filter) && (!filter.is_empty() || advanced == (self.group || !primary(&self.button.command)))
    }

    pub fn label(&self) -> String {
        if self.group {
            aurora_i18n::tr!("toolbar-menu-group", name = label(&self.button))
        } else {
            label(&self.button).to_owned()
        }
    }

    pub fn matches(&self, filter: &str) -> bool {
        filter.is_empty() || sort_key(&self.label()).contains(filter) || self.button.command.contains(filter)
    }
}

pub(crate) fn entries(layout: &Layout, settings: &ToolbarSettings, menu: &commands::Catalogue) -> Vec<Tile> {
    let mut tiles: Vec<_> = catalogue(layout)
        .into_iter()
        .filter(|button| {
            ![super::structure::DIVIDER, super::drawers::FLOATING, super::drawers::INTERFACES].contains(&button.command.as_str())
        })
        .map(|button| Tile {
            category: category(&button.command),
            button,
            group: false,
        })
        .collect();
    let indices: std::collections::HashMap<_, _> = tiles.iter().enumerate().map(|(i, tile)| (tile.button.command.clone(), i)).collect();
    for entry in &menu.entries {
        // "Customize interface" ends the Interfaces block, never alone
        if entry.info
            || entry.separator
            || !entry.button.command.starts_with(commands::PREFIX)
            || super::catalogue::retired(&entry.button.command)
        {
            continue;
        }
        if !entry.group
            && let Some(index) = alias(&entry.button.command).and_then(|id| indices.get(id))
        {
            // A saved menu copy remains the editor's drag source when it is
            // the placed variant. Opening this window never migrates settings.
            if !settings.placed(&tiles[*index].button.command, layout) && settings.placed(&entry.button.command, layout) {
                tiles[*index].button = entry.button.clone();
            }
            continue;
        }
        tiles.push(Tile {
            button: entry.button.clone(),
            category: category(&entry.button.command),
            group: entry.group,
        });
    }
    // the user's floating blocks: their buttons, placed or not
    tiles.extend(
        settings
            .drawers
            .iter()
            .filter_map(|(id, drawer)| drawer.floating.as_ref().map(|floating| (id, floating)))
            .map(|(id, floating)| Tile {
                button: floating.button(id),
                category: Category::Interface,
                group: false,
            }),
    );
    tiles.sort_by_cached_key(|tile| (tile.category, sort_key(&tile.label())));
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_and_advanced_partition_the_catalogue_but_search_reaches_both() {
        let tiles = entries(&Layout::default(), &ToolbarSettings::default(), &Default::default());
        let main: Vec<_> = tiles.iter().filter(|tile| tile.in_view(false, "")).collect();
        assert!(
            (20..=50).contains(&main.len()),
            "the first view should stay manageable: {}",
            main.len()
        );
        for tile in &tiles {
            assert_ne!(tile.in_view(false, ""), tile.in_view(true, ""));
        }
        assert!(main.iter().any(|tile| tile.button.command == "inventory"));
        let debug = tiles
            .iter()
            .find(|tile| tile.button.command == "settings_debug")
            .expect("advanced command");
        assert!(!debug.in_view(false, ""));
        assert!(debug.in_view(true, ""));
        assert!(debug.in_view(false, "settings_debug"));
        assert!(
            primary("menu:common-world/common-about-land"),
            "saved menu aliases keep the same level as their toolbar counterpart"
        );
    }

    #[test]
    fn every_builtin_has_an_explicit_category_and_aliases_use_stable_ids() {
        for command in super::super::COMMANDS {
            assert_ne!(category(command.id), Category::Custom, "{}", command.id);
        }
        assert_eq!(alias("menu:common-world/common-about-land"), Some("aboutland"));
        assert_eq!(alias("menu:common-me/avatar-health-menu/avatar-health-stop-animations"), None);
        assert_eq!(category("skin_custom_command"), Category::Custom);
    }
}
