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
        | "picks"
        | "poser"
        | "pose_stand"
        | "wearable_favorites"
        | "avatar_align"
        | "face_nearest_avatar"
        | "gestures" => Avatar,
        "conversations" | "speak" | "people" | "friends" | "contact_sets" | "block_list" | "radar" | "group_titles" | "voice"
        | "defaultchatbar" | "conversation_log" | "discord" => Communication,
        "home" | "location_bar" | "places" | "favorites" | "worldmap" | "minimap" | "aboutland" | "regioninfo" | "teleport_history"
        | "landmark_here" | "search" | "destinations" | "regiontracker" | "areasearch" | "webbrowser" => World,
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
        | "360capture" => Creation,
        "aurora_menu" | "preferences" | "quickprefs" | "immersive" | "view" | "vr_view" | "toolbar_separator" | "bar_navigation"
        | "bar_account" | "bar_system" | "bar_notifications" | "bar_conversations" => Interface,
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
        "menu:common-build/entry-box" => "build",
        "menu:common-build/poser-title" => "poser",
        "menu:common-help/bug-title" => "report_bug",
        "menu:common-help/report-abuse-title" => "reporter",
        "menu:common-help/grid_status" => "gridstatus",
        "menu:common-advanced/debug-settings" => "settings_debug",
        _ => return None,
    })
}

pub(crate) struct Tile {
    pub button: ToolButton,
    pub category: Category,
    pub group: bool,
}

impl Tile {
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
        .map(|button| Tile {
            category: category(&button.command),
            button,
            group: false,
        })
        .collect();
    let indices: std::collections::HashMap<_, _> = tiles.iter().enumerate().map(|(i, tile)| (tile.button.command.clone(), i)).collect();
    for entry in &menu.entries {
        if entry.info || entry.separator || !entry.button.command.starts_with(commands::PREFIX) {
            continue;
        }
        if !entry.group
            && let Some(index) = alias(&entry.button.command).and_then(|id| indices.get(id))
        {
            // A saved menu copy remains the editor's drag source when it is
            // the placed variant. Opening this window never migrates settings.
            if settings.location(&tiles[*index].button.command, layout).is_none()
                && settings.location(&entry.button.command, layout).is_some()
            {
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
    tiles.sort_by_cached_key(|tile| (tile.category, sort_key(&tile.label())));
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

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
