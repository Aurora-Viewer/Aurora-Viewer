//! egui user interface.

pub mod audio;
pub mod aurora_bg;
pub mod avatar_picker;
pub mod backdrop;
pub mod bars;
pub mod chat;
pub mod colors;
pub mod contacts;
pub mod context;
pub mod debug_overlay;
pub mod display_name;
pub mod emoji;
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
pub mod profile;
pub mod skin;
pub mod sound_cues;
pub mod texture_picker;
pub mod voice_dot;
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
    pub inventory: bool,
    pub people_tab: u8,
    /// The torn-off Contacts window (ContactsTornOff).
    pub contacts: bool,
    /// "À propos du terrain" (navigation bar info icon).
    pub about_land: bool,
    /// Navigation bar location being edited (text) and "just opened" (select all).
    pub nav_edit: Option<String>,
    pub nav_edit_new: bool,
}
