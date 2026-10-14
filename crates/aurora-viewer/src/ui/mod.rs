//! egui user interface.

pub mod ao;
pub mod appearance;
pub mod audio;
pub mod aurora_bg;
pub mod avatar_picker;
pub mod backdrop;
pub mod bars;
pub mod bug_report;
pub mod buy_currency;
pub mod buy_land;
pub mod call_invite;
pub mod chat;
pub mod color_picker;
pub mod colors;
pub mod contacts;
pub mod context;
pub mod debug_overlay;
pub mod display_name;
pub mod emoji;
pub mod env_day_editor;
pub mod env_editor;
pub mod environment;
pub mod exit;
pub mod experience_profile;
pub mod find_shortcut;
pub mod fonts;
pub mod gestures;
pub mod group_profile;
pub mod hover_height;
pub mod hud;
pub mod icons;
pub mod inspect;
pub mod inventory;
pub mod land;
pub mod landmark_create;
pub mod layout;
pub mod loading;
pub mod login;
pub mod lookat_targets;
pub mod map_tiles;
pub mod marketplace;
pub mod marketplace_listings;
pub mod media;
pub mod menu;
pub mod minimap;
pub mod nameplate;
pub mod news;
pub mod notification_center;
pub mod notifications;
pub(crate) mod numeric_drag;
pub mod object_actions;
pub mod options;
pub mod parcel_icons;
pub mod pay;
pub mod people;
pub mod perf;
pub mod place_details;
pub mod places;
pub mod presence;
pub mod profile;
pub mod report_abuse;
pub mod scripts;
pub mod search;
pub mod shape;
pub mod skin;
pub mod snap;
pub mod snapshot;
pub mod sound_cues;
pub mod texture_picker;
#[cfg(test)]
mod theme_guard;
pub mod toolbar;
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
    /// The photo window (Firestorm's snapshot floater).
    pub snapshot: bool,
    pub people: bool,
    pub minimap: bool,
    /// World map (Ctrl+M).
    pub world_map: bool,
    pub settings: bool,
    /// Local time of day: 0 shared (region EEP), 1 sunrise, 2 noon, 3 sunset, 4 midnight.
    pub time_of_day: u8,
    /// The environment selector (Firestorm's quick preferences lists).
    pub environment: bool,
    /// "Personal lighting" (LLFloaterEnvironmentAdjust).
    pub personal_lighting: bool,
    pub inventory: bool,
    pub gestures: bool,
    pub appearance: bool,
    pub ao: bool,
    pub people_tab: u8,
    /// The torn-off Contacts window (ContactsTornOff).
    pub contacts: bool,
    /// "About Land" (navigation bar info icon).
    pub about_land: bool,
    /// "Places" (favorites, landmarks, teleport history, place profiles).
    pub places: bool,
    /// "Search" (Firestorm's legacy search floater, Ctrl+F).
    pub search: bool,
    /// Navigation bar location being edited (text) and "just opened" (select all).
    pub nav_edit: Option<String>,
    pub nav_edit_new: bool,
}

impl Panels {
    /// The panels of a new session: those the settings recorded at the last
    /// exit, or, without a settings file, the first-launch layout of
    /// `Settings::default` (the radar, the mini-map, Performance and
    /// Conversations).
    pub fn from_settings(settings: &crate::settings::Settings) -> Panels {
        Panels {
            chat: settings.show_chat,
            perf: settings.show_perf,
            snapshot: false,
            people: settings.show_people,
            minimap: settings.show_minimap,
            world_map: false,
            settings: false,
            time_of_day: settings.time_of_day,
            environment: false,
            personal_lighting: false,
            inventory: settings.show_inventory,
            gestures: false,
            appearance: settings.show_appearance,
            ao: settings.show_ao,
            people_tab: settings.people_tab.min(1),
            contacts: false,
            about_land: false,
            places: false,
            search: false,
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
            ("search", panels.search),
        ]
        .into_iter()
        .filter_map(|(name, open)| open.then_some(name))
        .collect()
    }

    #[test]
    fn first_launch_opens_the_radar_the_mini_map_performance_and_conversations() {
        let missing_file = Settings::default();
        let empty_file: Settings = serde_json::from_str("{}").expect("blank settings");
        for blank in [missing_file, empty_file] {
            let panels = Panels::from_settings(&blank);
            assert_eq!(open(&panels), ["chat", "perf", "people", "minimap"]);
            assert_eq!(panels.people_tab, 0, "the radar is the first tab of People");
            assert!(
                !blank.radar.show_minimap,
                "the mini-map window is open: the radar does not repeat it"
            );
        }
    }

    /// The radar in the top left corner, the mini-map in the top right one
    /// with Performance under it, Conversations in the bottom right one:
    /// inside the work area and apart, each at the same gap from the bars,
    /// the edges and its neighbor, down to a 1280 x 720 window.
    #[test]
    fn the_first_launch_windows_take_their_corners() {
        use super::layout::GAP;
        for (width, height) in [(1280.0, 720.0), (1600.0, 900.0), (1920.0, 1017.0)] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            let work = super::layout::default_work_area(screen);
            assert!(work.top() > screen.top() && work.bottom() < screen.bottom());
            let radar = egui::Rect::from_min_size(super::people::default_pos(work), super::people::DEFAULT_SIZE);
            let map = super::minimap::default_rect(work);
            let perf = egui::Rect::from_min_size(
                super::perf::default_pos(work, super::perf::COMPACT_W),
                egui::vec2(super::perf::COMPACT_W + super::perf::SIDES, super::perf::COMPACT_H),
            );
            let chat = super::chat::default_rect(work);
            let windows = [("radar", radar), ("map", map), ("perf", perf), ("chat", chat)];
            for (i, (name, rect)) in windows.iter().enumerate() {
                assert!(
                    work.shrink(GAP - 0.01).contains_rect(*rect),
                    "{name} at {width} x {height}: {rect:?}"
                );
                for (other, other_rect) in &windows[i + 1..] {
                    // apart by the gap at least: expanded by half of it, they still miss each other
                    assert!(
                        !rect.expand(GAP / 2.0 - 0.01).intersects(other_rect.expand(GAP / 2.0 - 0.01)),
                        "{name} and {other} at {width} x {height}: {rect:?} {other_rect:?}"
                    );
                }
            }
            // the corners
            assert_eq!(radar.left_top(), work.left_top() + egui::Vec2::splat(GAP));
            assert_eq!(map.right_top(), work.right_top() + egui::vec2(-GAP, GAP));
            assert_eq!(chat.right_bottom(), work.right_bottom() - egui::Vec2::splat(GAP));
            // Performance stands under the mini-map, on its right edge
            assert_eq!(perf.top(), map.bottom() + GAP);
            assert_eq!(perf.right(), map.right());
            assert_eq!(chat.right(), map.right());
        }
        // "Search", closed at first: opened, it stands in the middle,
        // inside the work area, and hides none of these windows entirely
        for (width, height) in [(1280.0, 720.0), (1600.0, 900.0), (1920.0, 1017.0)] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            let work = super::layout::default_work_area(screen);
            let search = super::search::default_rect(work);
            assert_eq!(search.size(), egui::vec2(900.0, 600.0), "{width} x {height}");
            assert!(work.shrink(GAP - 0.01).contains_rect(search), "{width} x {height}: {search:?}");
            assert!((search.center() - work.center()).length() <= 1.0, "{width} x {height}: {search:?}");
            let perf_pos = super::perf::default_pos(work, super::perf::COMPACT_W);
            let perf = egui::Rect::from_min_size(
                perf_pos,
                egui::vec2(super::perf::COMPACT_W + super::perf::SIDES, super::perf::COMPACT_H),
            );
            let radar = egui::Rect::from_min_size(super::people::default_pos(work), super::people::DEFAULT_SIZE);
            for (name, rect) in [
                ("radar", radar),
                ("map", super::minimap::default_rect(work)),
                ("perf", perf),
                ("chat", super::chat::default_rect(work)),
            ] {
                // a strip wide enough to read and to click stays beside it
                let left = (search.left() - rect.left()).max(0.0);
                let right = (rect.right() - search.right()).max(0.0);
                assert!(left.max(right) >= 160.0, "{name} at {width} x {height}: {rect:?} under {search:?}");
            }
        }
        // Conversations has its whole height where there is room for it
        let large = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1017.0));
        assert_eq!(
            super::chat::default_rect(super::layout::default_work_area(large)).size(),
            egui::vec2(700.0, 440.0)
        );
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
