//! egui user interface.

pub mod about;
pub mod animation_explorer;
pub mod ao;
pub mod appearance;
pub mod area_search;
pub mod audio;
pub mod aurora_bg;
pub mod avatar_picker;
pub mod avatar_render_settings;
pub mod backdrop;
pub mod bars;
pub mod bidi;
pub mod blacklist;
pub mod bumps;
pub mod sound_explorer;
// Bug report form (disabled, see main.rs):
// pub mod bug_report;
pub mod beacons;
pub mod buy_currency;
pub mod buy_land;
pub mod call_invite;
pub mod chat;
pub mod chat_bubbles;
pub mod click_away_focus;
pub mod close_key;
pub mod color_picker;
pub mod colors;
pub mod contacts;
pub mod context;
pub mod controls;
pub mod crosshair;
pub mod debug_overlay;
pub mod dev_toolbox;
pub mod display_name;
pub mod emoji;
pub mod env_day_editor;
pub mod env_editor;
pub mod environment;
pub mod exit;
pub mod experience_profile;
pub mod find_shortcut;
pub mod follow;
pub mod fonts;
pub mod friend_prompt;
pub mod gestures;
pub(crate) mod give_drop;
pub mod grid_status;
pub mod grids;
pub mod group_profile;
pub mod hover_height;
pub mod hover_tips;
pub mod hud;
pub mod icons;
pub mod inspect;
pub mod interface;
pub mod inventory;
pub mod land;
pub mod landmark_create;
pub mod layout;
pub mod link_health;
pub mod loading;
pub mod login;
pub mod lookat_targets;
pub mod looks_editor;
pub mod map_tiles;
pub mod marketplace;
pub mod marketplace_listings;
pub mod media;
pub mod media_filter;
pub mod menu;
pub mod minimap;
pub mod money_tracker;
pub mod monitor;
pub mod movelock;
pub mod movement;
pub mod nameplate;
pub mod news;
pub mod notification_center;
pub mod notifications;
pub(crate) mod numeric_drag;
pub mod object_actions;
pub mod options;
pub mod owned_land;
pub mod parcel_icons;
pub mod pay;
pub mod people;
pub mod perf;
pub mod physics;
pub mod place_card;
pub mod place_details;
pub mod places;
pub mod portals;
pub mod presence;
pub mod profile;
pub mod region;
pub mod region_restart;
pub mod remote_object;
pub mod report_abuse;
pub mod rlva;
pub mod script_queue;
pub mod scripts;
pub mod search;
pub mod shape;
pub mod sheet;
pub mod sim_console;
pub mod skin;
pub mod sky_beacons;
pub mod snap;
pub mod snapshot;
pub mod sound_cues;
pub(crate) mod spelling;
pub mod staff;
pub mod stream_title;
pub(crate) mod text_menu;
pub mod texture_library;
pub mod texture_picker;
#[cfg(test)]
mod theme_guard;
pub mod toolbar;
pub mod translate_download;
pub mod update;
pub mod viewfinder;
pub mod voice_tag;
pub mod wearable;
pub mod web_view;
pub mod widgets;

#[cfg(test)]
mod uri_restore_tests;
pub mod worldmap;

/// Which floating panels are open.
#[derive(Debug, Clone)]
pub struct Panels {
    pub chat: bool,
    pub perf: bool,
    /// The Dev Toolbox (Advanced › Dev Toolbox, Ctrl+Alt+D).
    pub dev_toolbox: bool,
    /// Advanced › Region console (SimConsoleAsync, estate managers).
    pub sim_console: bool,
    /// The photo window (Firestorm's snapshot floater).
    pub snapshot: bool,
    pub people: bool,
    pub minimap: bool,
    /// World map (Ctrl+M).
    pub world_map: bool,
    pub beacons: bool,
    pub owned_land: bool,
    pub settings: bool,
    /// Local time of day: 0 shared (region EEP), 1 sunrise, 2 noon, 3 sunset, 4 midnight.
    pub time_of_day: u8,
    /// The environment selector (Firestorm's quick preferences lists).
    pub environment: bool,
    /// "Personal lighting" (LLFloaterEnvironmentAdjust).
    pub personal_lighting: bool,
    pub inventory: bool,
    pub texture_library: bool,
    pub texture_library_presentation: texture_library::Presentation,
    pub gestures: bool,
    pub appearance: bool,
    pub ao: bool,
    pub people_tab: u8,
    /// The torn-off Contacts window (ContactsTornOff).
    pub contacts: bool,
    /// "About Land" (navigation bar info icon).
    pub about_land: bool,
    /// "Region / Estate" (World menu, Alt+R).
    pub region_estate: bool,
    /// "Places" (favorites, landmarks, teleport history, place profiles).
    pub places: bool,
    /// "Search" (Firestorm's legacy search floater, Ctrl+F).
    pub search: bool,
    pub area_search: bool,
    pub sound_explorer: bool,
    /// "Animation explorer" (World menu).
    pub animation_explorer: bool,
    pub avatar_render_settings: bool,
    /// "Asset blacklist" (World menu): the derendered objects and residents.
    pub asset_blacklist: bool,
    /// "Money tracker" (Me menu): the L$ received and paid.
    pub money_tracker: bool,
    /// "Bumps, pushes & hits" (Help menu): who bumped, pushed or hit us.
    pub bumps: bool,
    /// "Grid status" (Help menu, toolbar): the grid's incidents.
    pub grid_status: bool,
    /// "About Aurora Viewer" (Help menu): the version, the system
    /// information and the licences.
    pub about: bool,
    /// World › Stream title (kept in `audio.stream_title_window`), and its
    /// history.
    pub stream_title: bool,
    pub stream_title_history: bool,
    /// The Controls window on its Camera tab (Firestorm's camera window
    /// with Preferences › Aurora), and on its Movement tab (the movement
    /// window).
    pub camera_controls: bool,
    pub movement_controls: bool,
    /// Navigation bar location being edited (text) and "just opened" (select all).
    pub nav_edit: Option<String>,
    pub nav_edit_new: bool,
}

impl Panels {
    /// The panels of a new session: those the settings recorded at the last
    /// exit, or, without a settings file, the first-launch layout of
    /// `Settings::default` (the radar, the mini-map and Conversations).
    /// Performance opens only if Preferences ask for it (`show_perf`).
    pub fn from_settings(settings: &crate::settings::Settings) -> Panels {
        Panels {
            chat: settings.show_chat,
            perf: settings.show_perf,
            dev_toolbox: false,
            sim_console: false,
            snapshot: false,
            people: settings.show_people,
            minimap: settings.show_minimap,
            world_map: false,
            beacons: false,
            owned_land: false,
            settings: false,
            time_of_day: settings.time_of_day,
            environment: false,
            personal_lighting: false,
            inventory: settings.show_inventory,
            texture_library: false,
            texture_library_presentation: settings.texture_library.presentation,
            gestures: false,
            appearance: settings.show_appearance,
            ao: settings.show_ao,
            people_tab: settings.people_tab.min(1),
            contacts: false,
            about_land: false,
            region_estate: false,
            places: false,
            search: false,
            area_search: false,
            sound_explorer: false,
            animation_explorer: false,
            avatar_render_settings: false,
            asset_blacklist: false,
            money_tracker: false,
            bumps: false,
            grid_status: false,
            about: false,
            stream_title: settings.audio.stream_title_window,
            stream_title_history: false,
            camera_controls: false,
            movement_controls: false,
            nav_edit: None,
            nav_edit_new: false,
        }
    }
}

/// Conversations and the mini-map as last seen by `remember`: their
/// "at startup" settings follow a window only when it opens or closes.
#[derive(Clone, Copy, Debug)]
pub struct StartupWindows {
    chat: bool,
    minimap: bool,
}

impl StartupWindows {
    pub fn new(panels: &Panels) -> StartupWindows {
        StartupWindows {
            chat: panels.chat,
            minimap: panels.minimap,
        }
    }

    /// Keeps the window's state at exit, like the save_visibility of
    /// floater_im_container.xml and floater_map.xml, but only when the
    /// window opens or closes: in between, the switches of Preferences ›
    /// Layout › Startup hold (#1320), until the next open or close.
    pub fn remember(&mut self, panels: &Panels, settings: &mut crate::settings::Settings) {
        if panels.chat != self.chat {
            self.chat = panels.chat;
            settings.show_chat = panels.chat;
        }
        if panels.minimap != self.minimap {
            self.minimap = panels.minimap;
            settings.show_minimap = panels.minimap;
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
    fn first_launch_opens_the_radar_the_mini_map_and_conversations() {
        let missing_file = Settings::default();
        let empty_file: Settings = serde_json::from_str("{}").expect("blank settings");
        for blank in [missing_file, empty_file] {
            let panels = Panels::from_settings(&blank);
            assert_eq!(open(&panels), ["chat", "people", "minimap"]);
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

    /// The Startup switches of Preferences hold while the windows stay as
    /// they are, and a window opened or closed afterwards wins again (#1320).
    #[test]
    fn startup_switches_hold_until_the_window_opens_or_closes() {
        let mut settings = Settings::default();
        let mut panels = Panels::from_settings(&settings);
        let mut seen = super::StartupWindows::new(&panels);
        // both windows open, both switches turned off in Preferences
        settings.show_chat = false;
        settings.show_minimap = false;
        for _ in 0..3 {
            seen.remember(&panels, &mut settings);
        }
        assert!(!settings.show_chat && !settings.show_minimap);
        // turned back on
        settings.show_chat = true;
        seen.remember(&panels, &mut settings);
        assert!(settings.show_chat);
        // the mini-map closed, then reopened: its state is remembered again
        panels.minimap = false;
        seen.remember(&panels, &mut settings);
        assert!(!settings.show_minimap);
        settings.show_minimap = true;
        seen.remember(&panels, &mut settings);
        assert!(settings.show_minimap, "the switch holds while the window stays closed");
        panels.minimap = true;
        panels.chat = false;
        seen.remember(&panels, &mut settings);
        assert!(settings.show_minimap && !settings.show_chat);
    }
}
