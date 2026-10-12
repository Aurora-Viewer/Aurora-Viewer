//! egui user interface.

pub mod ao;
pub mod appearance;
pub mod audio;
pub mod aurora_bg;
pub mod avatar_picker;
pub mod backdrop;
pub mod bars;
pub mod bug_report;
pub mod chat;
pub mod color_picker;
pub mod colors;
pub mod contacts;
pub mod context;
pub mod debug_overlay;
pub mod display_name;
pub mod emoji;
pub mod environment;
pub mod exit;
pub mod fonts;
pub mod hud;
pub mod icons;
pub mod inventory;
pub mod land;
pub mod layout;
pub mod loading;
pub mod login;
pub mod map_tiles;
pub mod media;
pub mod menu;
pub mod minimap;
pub mod news;
pub mod notifications;
pub mod object_actions;
pub mod options;
pub mod parcel_icons;
pub mod people;
pub mod perf;
pub mod place_details;
pub mod places;
pub mod profile;
pub mod scripts;
pub mod skin;
pub mod sound_cues;
pub mod texture_picker;
pub mod update;
pub mod voice_tag;
pub mod web_view;
pub mod widgets;
pub mod worldmap;

/// Which floating panels are open.
#[derive(Debug, Clone)]
pub struct Panels {
    pub chat: bool,
    pub perf: bool,
    pub people: bool,
    pub minimap: bool,
    /// Carte du monde (Ctrl+M).
    pub world_map: bool,
    pub settings: bool,
    /// Local time of day: 0 shared (region EEP), 1 sunrise, 2 noon, 3 sunset, 4 midnight.
    pub time_of_day: u8,
    /// The environment selector (Firestorm's quick preferences lists).
    pub environment: bool,
    /// « Éclairage personnel » (LLFloaterEnvironmentAdjust).
    pub personal_lighting: bool,
    pub inventory: bool,
    pub appearance: bool,
    pub ao: bool,
    pub people_tab: u8,
    /// The torn-off Contacts window (ContactsTornOff).
    pub contacts: bool,
    /// "À propos du terrain" (navigation bar info icon).
    pub about_land: bool,
    /// « Lieux » (favorites, landmarks, teleport history, place profiles).
    pub places: bool,
    /// Navigation bar location being edited (text) and "just opened" (select all).
    pub nav_edit: Option<String>,
    pub nav_edit_new: bool,
}

impl Panels {
    /// The panels of a new session: those the settings recorded at the last
    /// exit, or, without a settings file, the first-launch layout of
    /// `Settings::default` (the radar and the mini-map).
    pub fn from_settings(settings: &crate::settings::Settings) -> Panels {
        Panels {
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
            ao: settings.show_ao,
            people_tab: settings.people_tab.min(1),
            contacts: false,
            about_land: false,
            places: false,
            nav_edit: None,
            nav_edit_new: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Panels;
    use crate::settings::Settings;

    /// Every window a session can start with, by name.
    fn open(panels: &Panels) -> Vec<&'static str> {
        [
            ("chat", panels.chat),
            ("perf", panels.perf),
            ("people", panels.people),
            ("minimap", panels.minimap),
            ("world_map", panels.world_map),
            ("settings", panels.settings),
            ("environment", panels.environment),
            ("personal_lighting", panels.personal_lighting),
            ("inventory", panels.inventory),
            ("appearance", panels.appearance),
            ("ao", panels.ao),
            ("contacts", panels.contacts),
            ("about_land", panels.about_land),
            ("places", panels.places),
        ]
        .into_iter()
        .filter_map(|(name, open)| open.then_some(name))
        .collect()
    }

    #[test]
    fn first_launch_opens_the_radar_the_mini_map_and_performance() {
        let missing_file = Settings::default();
        let empty_file: Settings = serde_json::from_str("{}").expect("blank settings");
        for blank in [missing_file, empty_file] {
            let panels = Panels::from_settings(&blank);
            assert_eq!(open(&panels), ["perf", "people", "minimap"]);
            assert_eq!(panels.people_tab, 0, "the radar is the first tab of People");
            assert!(
                !blank.radar.show_minimap,
                "the mini-map window is open: the radar does not repeat it"
            );
        }
    }

    /// The radar in the top left corner, the mini-map in the top right one,
    /// on the same line and apart, down to a 1280 x 720 window.
    #[test]
    fn the_first_launch_windows_take_opposite_corners() {
        for (width, height) in [(1280.0, 720.0), (1600.0, 900.0)] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            let radar = egui::Rect::from_min_size(super::people::DEFAULT_POS, super::people::DEFAULT_SIZE);
            let map = super::minimap::default_rect(screen);
            assert!(screen.contains_rect(radar) && screen.contains_rect(map), "{width} x {height}");
            assert!(!radar.intersects(map), "{width} x {height}");
            assert_eq!(radar.top(), map.top());
            assert!(radar.left() < 20.0 && screen.right() - map.right() < 20.0);
            // Performance stands under the mini-map, on its right edge
            let perf = super::perf::default_pos(screen, super::perf::COMPACT_W);
            assert!(perf.y > map.bottom() && perf.y - map.bottom() < 20.0);
            assert_eq!(perf.x + super::perf::COMPACT_W + super::perf::SIDES, map.right());
            assert!(perf.x > radar.right(), "{width} x {height}");
        }
    }

    #[test]
    fn a_saved_session_reopens_its_own_windows() {
        let saved: Settings = serde_json::from_str(
            r#"{"show_chat":true,"show_perf":true,"show_people":false,"show_minimap":false,
                "show_inventory":true,"people_tab":1,"radar":{"show_minimap":true}}"#,
        )
        .expect("saved settings");
        let panels = Panels::from_settings(&saved);
        assert_eq!(open(&panels), ["chat", "perf", "inventory"]);
        assert_eq!(panels.people_tab, 1);
        assert!(saved.radar.show_minimap);
    }
}
