//! Firestorm-like chrome: menu/status bar and navigation bar at the top,
//! chat input + feature buttons at the bottom. Flat, skinnable via
//! `skin::Layout` and the theme palette. Aurora's top bar, glass islands
//! over the world, is in `islands.rs` and `place.rs`; these full-width bars
//! are its Firestorm look (Preferences › Aurora).

pub(crate) mod clock;
pub(crate) mod demo;
pub mod dock;
pub mod favorites;
mod graphics;
pub mod islands;
mod place;
pub mod side;
pub(crate) mod sizing;
mod tp_list;

use super::Panels;
use super::icons::Icons;
use super::menu;
use super::skin::Layout;
use crate::theme::Palette;
use egui::{Color32, RichText, Vec2};

#[derive(Default)]
pub enum BarAction {
    IkEdit,
    Poser,
    /// Build › Edit particles (Object.EditParticles).
    EditParticles,
    Portals,
    CheckUpdates,
    ReportBug,
    /// The bug button's right click: the viewer's version to the clipboard,
    /// silently, for a report or a chat.
    CopyVersion,
    /// Help › Report Abuse (LLFloaterReporter::showFromMenu).
    ReportAbuse,
    /// Help › Server release notes (LLFloaterAbout's release notes link).
    ServerReleaseNotes,
    /// Contents › Message of the day: the login message again in the
    /// nearby chat (Advanced.ToggleHUDInfo "motd").
    MessageOfTheDay,
    /// Same import chooser toggle as the configurable Import shortcut.
    Import,
    /// Advanced › Media streams backup (Streamlist.xml_import / xml_export).
    ImportStreamList,
    ExportStreamList,
    BlenderLive,
    ShowHuds(bool),
    HudZoom(f32),
    #[default]
    None,
    /// ShowBanLines: 0 hidden, 1 on collision, 2 on proximity.
    BanLines(u8),
    /// Build floater: 0 toggle, 1 edit, 2 create, 3 land.
    Build(u8),
    Logout,
    Quit,
    TeleportHome,
    /// World › Set Home to Here, or the home buttons' right click
    /// (World.SetHomeLocation).
    SetHome,
    ToggleFly,
    ToggleMoveLock,
    /// Me › Quickjump (Self.toggleIgnorePreJump, FSIgnoreFinishAnimation).
    ToggleQuickjump,
    /// Advanced › Quiet snapshots (PlayModeUISndSnapshot).
    ToggleQuietSnapshots,
    ToggleWaterWalk,
    ToggleSwimming,
    MoveLockOptions,
    MovementEnhancer,
    ToggleMouselook,
    SitGround,
    StandUp,
    ResetCamera,
    /// Teleport history (navigation bar arrows).
    TeleportBack,
    TeleportForward,
    /// A place of the arrows' list, by its index in the session's history.
    TeleportHistoryItem(usize),
    /// Voice light clicked: open "Sound and voice".
    VoicePrefs,
    /// Location typed or pasted in the navigation bar (SLURL, Region/x/y/z...).
    TeleportTo(String),
    /// Communicate > Online status changed.
    SetStatus(crate::world::status::StatusModes),
    /// Open the chat preferences (automatic responses).
    ChatPrefs,
    /// Me > Profile.
    MyProfile,
    /// World › Environment › Use shared environments
    /// (setSharedEnvironment: drops every local choice).
    SharedEnvironment,
    /// World › Environment › New sky / water / day cycle (World.EnvPreset
    /// new_sky, new_water, new_day_cycle).
    NewEnvironment(crate::world::env_select::SettingsKind),
    /// World › Teleport history (ToggleTeleportHistory, Alt+H).
    TeleportHistory,
    /// The balance, or Me › Buy L$ (LLStatusBar::onClickBuyCurrency).
    BuyCurrency,
    /// The balance shown or hidden (its right click, the account island's).
    ShowBalance(bool),
    /// Me › Double-click on the ground (Firestorm's "DoubleClick Teleport"
    /// of its movement menu, Advanced.SetDoubleClickAction).
    SetDoubleClick(crate::autopilot::DoubleClick),
    /// World › SL Marketplace (FSLoadSLMarketplace).
    Marketplace,
    /// Me › Marketplace listings (menu_viewer.xml "MarketplaceListings").
    MarketplaceListings,
    /// World › Landmark this Place (World.CreateLandmark).
    CreateLandmark,
    LandmarkStar(StarAction),
    /// The settings of the place island, from its right-click menu.
    PlaceBar {
        stretch: bool,
        rights_text: bool,
    },
    /// The bug report button shown or hidden (the menu island's right click).
    ShowBugButton(bool),
    /// A click in the favourites (`favorites.rs`).
    Favorite(favorites::FavAction),
    /// The system island's right click: what it shows, the traffic
    /// indicator, a graphics or camera preset, the windows it opens.
    SystemIsland(crate::settings::SystemIsland),
    NetStats(bool),
    GraphicsPreset(crate::graphics_presets::Preset),
    SavedGraphicsPreset(String),
    FpsCap(bool),
    CameraPreset(crate::camera::CameraPreset),
    GraphicsPreferences,
    CameraControls,
    /// Performance on its Network section (`perf::show_network`).
    NetworkGraph,
    /// The toolbar's "Toolbar buttons" window.
    CustomizeBars,
    /// The toolbar's Stop Animations (Firestorm's stop_animations command).
    StopAnimations,
    ResyncAnimations,
    /// Show User Interface (Alt+Shift+U), and hiding it once confirmed.
    ToggleInterface,
    HideInterface,
    /// Immersive mode (#859): the interface hidden but the chat.
    ToggleImmersive,
    /// The HUDs shown or hidden in immersive mode (#1190).
    ImmersiveHuds(bool),
    /// World › World tooltips: the three checks, as they are now.
    HoverTips(HoverTipsMenu),
    WorldOverlays(crate::world::beacons::Overlays),
    ShowTransparency,
    OwnedLand,
}

/// How the islands of Aurora's top bar are set (Preferences › Interface).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct IslandsLook {
    /// The place island reaches the account island.
    pub stretch: bool,
    /// The parcel rights are named when there is room.
    pub rights_text: bool,
}

/// A place of the teleport history, in the place menu.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentPlace {
    pub name: String,
    pub region: String,
    pub slurl: String,
}

pub struct StatusInfo<'a> {
    pub location_hidden: bool,
    pub show_bug_report: bool,
    pub show_huds: bool,
    pub hud_zoom: f32,
    pub region: &'a str,
    /// Parcel the agent stands on (may be empty).
    pub parcel: &'a str,
    /// Destinations of the back / forward arrows (None = disabled).
    pub tp_back: Option<String>,
    pub tp_forward: Option<String>,
    /// Every place of the session, for the arrows' list.
    pub tp_session: crate::world::tphistory::Session<'a>,
    pub maturity: &'a str,
    pub position: glam::Vec3,
    pub fps: f32,
    pub flying: bool,
    pub seated: bool,
    pub ping_ms: u32,
    pub balance: Option<i32>,
    /// The balance shown (`show_balance`, #1340).
    pub show_balance: bool,
    /// Voice connection light and its explanation.
    pub voice: (crate::voice::VoiceLight, String),
    /// SLURL of the current position (shown when the location is edited).
    pub slurl: String,
    /// Online status modes (away, do not disturb, autoresponse...).
    pub status: crate::world::status::StatusModes,
    /// ShowBanLines mode (World › Ban lines).
    pub ban_lines: u8,
    /// Abilities of the current parcel (right end of the location field).
    pub parcel_icons: Vec<super::parcel_icons::ParcelIcon>,
    /// A local environment is shown (selector, preset, personal lighting).
    pub local_env: bool,
    /// The keys of World › Environment's shortcuts, by `Panels::time_of_day`
    /// (0 Use shared environment; the legacy midday has none).
    pub env_keys: [String; 6],
    /// What a double click on the ground does (Me › Double-click on the ground).
    pub double_click: crate::autopilot::DoubleClick,
    /// Quickjump is on (Me › Quickjump).
    pub quickjump: bool,
    /// Photos are taken without the shutter sound nor the snapshot
    /// animation (Advanced › Quiet snapshots).
    pub quiet_snapshots: bool,
    /// The grid has a Marketplace (World › SL Marketplace is shown only on
    /// the Second Life grids, GridCheck "secondlife").
    pub marketplace: bool,
    /// The landmark lying on the agent's parcel (hasParcelLandmark): the
    /// star is lit and World › Landmark this place greyed out.
    pub parcel_landmark: Option<super::landmark_create::ParcelLandmark>,
    /// The agent's region allows setting home there
    /// (World.EnableSetHomeLocation).
    pub can_set_home: bool,
    /// The agent's region has a ServerReleaseNotes page (Help › Server
    /// release notes) and a SimConsoleAsync console (Advanced › Region
    /// console, estate managers).
    pub release_notes: bool,
    pub region_console: bool,
    /// The grid sent a message at login (Contents › Message of the day).
    pub motd: bool,
    /// The help page of the grid info (OpenSim), with the grid's name:
    /// Help › Knowledge base shows only with one.
    pub grid_help: Option<GridHelp>,
    /// Firestorm's full-width bars (Preferences › Aurora) instead of the
    /// islands.
    pub classic_bars: bool,
    pub islands: IslandsLook,
    /// Our name and profile picture (the account island).
    pub own_name: &'a str,
    pub own_pic: Option<&'a egui::TextureHandle>,
    /// The last places of the teleport history, newest first, without the
    /// one we are in (the place menu).
    pub recent: Vec<RecentPlace>,
    /// The favourites and where they live.
    pub favorites: &'a favorites::View,
    /// What the system island shows, the traffic indicator, the
    /// network's samples and the share of packets lost last second.
    pub system_island: crate::settings::SystemIsland,
    pub net_stats: bool,
    pub perf: &'a super::perf::PerfData,
    pub net_loss: f32,
    /// The graphics and camera presets in use (None: custom).
    pub graphics_preset: Option<crate::graphics_presets::Preset>,
    pub graphics_saved: Vec<(String, bool)>,
    pub fps_cap: bool,
    pub fps_limit: u32,
    pub camera_preset: crate::camera::CameraPreset,
    /// The interface hidden (an edge brought the islands back), and the
    /// keys of its shortcut.
    pub interface_hidden: bool,
    pub interface_keys: String,
    pub ik_keys: String,
    /// Build › Edit particles applies: an object we own is selected
    /// (Object.EnableEditParticles).
    pub edit_particles: bool,
    pub health: HealthMenu,
    pub hover_tips: HoverTipsMenu,
    pub world_overlays: crate::world::beacons::Overlays,
    pub show_transparency: bool,
    /// The conversations' thumbnails at the top right (#1134): how many,
    /// 0 for none (`thumbnails::reserve_top`).
    pub conversation_thumbnails: usize,
}

/// Help › Knowledge base, the Legacy Viewer's "[CURRENT_GRID] Help": the
/// grid's name (at the right end of the row) and its help page.
#[derive(Debug, Clone, PartialEq)]
pub struct GridHelp {
    pub grid: String,
    pub url: String,
}

/// World › World tooltips: the Legacy Viewer's Show Tips, Show Land
/// Tooltips and Show Tips On All Objects.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HoverTipsMenu {
    pub enabled: bool,
    pub land: bool,
    pub all_objects: bool,
}

/// What Me › Avatar health needs: our avatar (its skeleton resets) and the
/// keys of its shortcuts, as the menus write them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HealthMenu {
    pub me: uuid::Uuid,
    pub stop_keys: String,
    pub rebake_keys: String,
    pub refresh_keys: String,
    pub resync_keys: String,
}

/// The navigation bar's star (LLLocationInputCtrl's add landmark button).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StarAction {
    AddFavorite,
    AddLandmark,
    /// The Create Landmark window.
    MoreOptions,
    /// The lit star: edit the parcel's landmark.
    Edit(uuid::Uuid),
}

pub fn maturity_name(sim_access: u8) -> &'static str {
    match sim_access {
        13 => aurora_i18n::tr!("common-general"),
        21 => aurora_i18n::tr!("common-moderate"),
        42 => aurora_i18n::tr!("common-adult"),
        _ => "",
    }
}

/// Frame of the top and bottom bars: the `bar` color at the panels'
/// opacity, over the windows' blur (`widgets::finish_bar_glass`).
fn bar_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), p.panel_alpha))
        .inner_margin(egui::Margin::symmetric(6, 0))
}

/// Slot and icon size shared by the buttons of the top bar, so they sit at
/// a regular pitch whatever they draw.
const TOP_SLOT: Vec2 = Vec2::new(22.0, 20.0);
const TOP_ICON: f32 = 16.0;

fn icon_button_response(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, tip: &str, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(TOP_SLOT, egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if resp.hovered() { p.ink } else { p.muted };
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp.on_hover_text(tip)
}

/// The right click of a home button: World › Set Home to Here, offered by
/// the same rule; true when chosen.
pub(super) fn home_menu(resp: &egui::Response, p: &Palette, can_set_home: bool) -> bool {
    let mut set = false;
    menu::context_menu(resp, p, |ui| {
        set = menu::item_if(ui, p, "home-check", aurora_i18n::tr!("world-set-home-here"), can_set_home);
    });
    set
}

/// The balance's right click (the account island's in the islands): the
/// Legacy Viewer's "Show Currency Balance" of its status bar menu
/// (menu_hide_navbar.xml, FSShowCurrencyBalanceInStatusbar), then Buy L$.
pub(super) fn balance_menu(ui: &mut egui::Ui, p: &Palette, shown: bool, action: &mut BarAction) {
    if menu::check(ui, p, "coin-filled", aurora_i18n::tr!("ui-bars-show-balance"), shown, true) {
        *action = BarAction::ShowBalance(!shown);
    }
    if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("currency-menu")) {
        *action = BarAction::BuyCurrency;
    }
}

/// Local time of day choices (index = `Panels::time_of_day`; shown in the
/// order of `world::env::TIME_OF_DAY_ORDER`).
pub fn time_of_day() -> [&'static str; 6] {
    [
        aurora_i18n::tr!("ui-bars-shared-environments-region"),
        aurora_i18n::tr!("ui-bars-sunrise"),
        aurora_i18n::tr!("common-midday"),
        aurora_i18n::tr!("ui-bars-sunset"),
        aurora_i18n::tr!("common-midnight"),
        aurora_i18n::tr!("common-midday-legacy"),
    ]
}

/// Icons of the status entries, shared by the "Online status" menu and the
/// menu of the status light.
const AUTO_REPLY: &str = "message-chatbot-filled";
const NO_TELEPORT: &str = "plane-departure-filled";
const NO_INVITE: &str = "users-plus";
const NO_FRIENDSHIP: &str = "user-plus";
const AD_HOC: &str = "messages-filled";
/// "Invisible": the eye of the right to see someone online, barred.
const INVISIBLE: &str = "eye-off";

/// The shape of our own status dot, as the others' (`presence::Shape`):
/// hollow invisible, a bar in do not disturb, a crescent away.
fn status_shape(m: &crate::world::status::StatusModes) -> super::presence::Shape {
    use super::presence::Shape;
    if m.invisible {
        Shape::Ring
    } else if m.dnd {
        Shape::Bar
    } else if m.away {
        Shape::Crescent
    } else {
        Shape::Disc
    }
}

/// The colour of our own status dot, as the others' (`presence::color`):
/// away is the yellow crescent everywhere (#418).
fn status_color(p: &Palette, m: &crate::world::status::StatusModes) -> Color32 {
    if m.invisible {
        p.muted
    } else if m.dnd {
        p.danger
    } else if m.away || m.autorespond || m.autorespond_nonfriends {
        // the shape tells away (a crescent) from an automatic response (a disc)
        p.amber
    } else {
        p.success
    }
}

fn status_name(m: &crate::world::status::StatusModes) -> &'static str {
    if m.invisible {
        aurora_i18n::tr!("ui-bars-invisible")
    } else if m.dnd {
        aurora_i18n::tr!("common-do-not-disturb")
    } else if m.away {
        aurora_i18n::tr!("common-away")
    } else if m.autorespond {
        aurora_i18n::tr!("ui-bars-auto-reply")
    } else if m.autorespond_nonfriends {
        aurora_i18n::tr!("ui-bars-auto-reply-to-non-friends")
    } else {
        aurora_i18n::tr!("ui-bars-available")
    }
}

/// Check row of a menu that flips `on`; true when it was clicked (the menu
/// closes, as it did on a stock checkbox).
/// "Hide the interface" with its keys, "Show the interface" while it is
/// hidden (an edge brought the islands back); true when clicked.
pub(super) fn interface_item(ui: &mut egui::Ui, p: &Palette, st: &StatusInfo) -> bool {
    let (icon, label) = if st.interface_hidden {
        ("eye-filled", aurora_i18n::tr!("ui-bars-show-interface"))
    } else {
        ("eye-off", aurora_i18n::tr!("ui-bars-hide-interface"))
    };
    menu::item_keys(ui, p, icon, label, &st.interface_keys)
}

/// The menus' Immersive mode, after Show User Interface (#859).
pub(super) fn immersive_item(ui: &mut egui::Ui, p: &Palette) -> bool {
    menu::item(ui, p, "maximize", aurora_i18n::tr!("ui-bars-immersive"))
}

fn flip(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: &mut bool, enabled: bool) -> bool {
    let clicked = menu::check(ui, p, icon, label, *on, enabled);
    if clicked {
        *on = !*on;
    }
    clicked
}

/// Check row of a menu for the floater `id`, ticked while it is open: it
/// closes the window shown, expands it minimized, opens it closed
/// (LLFloaterReg::toggleInstance, as the toolbar button).
fn flip_window(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, id: &str, open: &mut bool) {
    if menu::check(ui, p, icon, label, *open, true) {
        super::widgets::toggle_floater(ui.ctx(), id, open);
    }
}

/// Entries of Communicate > Online status (Firestorm's menu).
fn status_menu(ui: &mut egui::Ui, p: &Palette, status: crate::world::status::StatusModes, action: &mut BarAction) {
    let mut m = status;
    let mut c = false;
    c |= flip(ui, p, "hourglass-filled", aurora_i18n::tr!("common-away"), &mut m.away, true);
    c |= flip(ui, p, "bell-off", aurora_i18n::tr!("common-do-not-disturb"), &mut m.dnd, true);
    c |= flip(ui, p, INVISIBLE, aurora_i18n::tr!("ui-bars-invisible"), &mut m.invisible, true);
    c |= flip(ui, p, AUTO_REPLY, aurora_i18n::tr!("ui-bars-auto-reply"), &mut m.autorespond, true);
    let non_friends = aurora_i18n::tr!("ui-bars-auto-reply-to-non-friends");
    c |= flip(ui, p, AUTO_REPLY, non_friends, &mut m.autorespond_nonfriends, true);
    menu::separator(ui, p);
    let teleports = aurora_i18n::tr!("ui-bars-reject-teleport-offers-and-requests");
    c |= flip(ui, p, NO_TELEPORT, teleports, &mut m.reject_teleports, true);
    let invites = aurora_i18n::tr!("ui-bars-reject-all-group-invitations");
    c |= flip(ui, p, NO_INVITE, invites, &mut m.reject_group_invites, true);
    let friendship = aurora_i18n::tr!("ui-bars-reject-all-friendship-requests");
    c |= flip(ui, p, NO_FRIENDSHIP, friendship, &mut m.reject_friendship, true);
    menu::submenu(ui, p, AD_HOC, aurora_i18n::tr!("ui-bars-ad-hoc-conferences"), true, |ui| {
        let ignore = aurora_i18n::tr!("ui-bars-ignore-all-and-exit-automatically");
        c |= flip(ui, p, "message-off", ignore, &mut m.ignore_adhoc, true);
        let enabled = m.ignore_adhoc;
        let report = aurora_i18n::tr!("ui-bars-report-ignored-residents-in-nearby-chat");
        c |= flip(ui, p, "message-filled", report, &mut m.report_ignored_adhoc, enabled);
        let friends = aurora_i18n::tr!("ui-bars-do-not-ignore-my-friends");
        c |= flip(ui, p, "users", friends, &mut m.adhoc_from_friends, enabled);
    });
    menu::separator(ui, p);
    if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("ui-bars-auto-reply-messages")) {
        *action = BarAction::ChatPrefs;
        return;
    }
    if c {
        *action = BarAction::SetStatus(m);
    }
}

/// Status dot next to the bell: its color tells the status, a click opens
/// the status choices.
fn status_dot(ui: &mut egui::Ui, p: &Palette, status: crate::world::status::StatusModes, action: &mut BarAction) {
    let (rect, resp) = ui.allocate_exact_size(TOP_SLOT, egui::Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if resp.hovered() || open {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    let col = status_color(p, &status);
    // its shape says it without the colour (hollow when invisible: nobody
    // sees the light), cut in the slot's face
    let face = if resp.hovered() || open { p.raised } else { p.bar };
    super::presence::paint_mark(ui.painter(), rect.center(), 5.0, status_shape(&status), col, face);
    // refusals on: a thin ring around the dot
    if status.reject_teleports || status.reject_group_invites || status.reject_friendship || status.ignore_adhoc {
        ui.painter().circle_stroke(rect.center(), 7.5, egui::Stroke::new(1.0, p.muted));
    }
    let resp = resp.on_hover_text(status_tip(&status));
    // demo captures: AURORA_DEMO_STATUSMENU=1 opens it (=invisible: with
    // that status, set by the viewer)
    static DEMO_OPEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *DEMO_OPEN.get_or_init(|| std::env::var_os("AURORA_DEMO_STATUSMENU").is_some()) && ui.ctx().cumulative_frame_nr() == 300 {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
    }
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        status_menu_body(ui, p, status, action);
    });
}

/// The tooltip of the status: its name, what invisible means, the refusals.
fn status_tip(status: &crate::world::status::StatusModes) -> String {
    let mut tip = aurora_i18n::tr!("ui-bars-status", value0 = status_name(status).to_string());
    if status.invisible {
        tip.push_str(&format!("\n{}", aurora_i18n::tr!("ui-bars-invisible-tip")));
    }
    for (on, what) in [
        (status.reject_teleports, aurora_i18n::tr!("ui-bars-teleports-rejected")),
        (status.reject_group_invites, aurora_i18n::tr!("ui-bars-group-invitations-rejected")),
        (status.reject_friendship, aurora_i18n::tr!("ui-bars-friendship-requests-rejected")),
        (status.ignore_adhoc, aurora_i18n::tr!("ui-bars-ad-hoc-conferences-ignored")),
    ] {
        if on {
            tip.push_str(&format!("\n• {what}"));
        }
    }
    tip
}

/// The menu of the status light (and of the account island's picture):
/// availability, automatic responses, refusals.
fn status_menu_body(ui: &mut egui::Ui, p: &Palette, status: crate::world::status::StatusModes, action: &mut BarAction) {
    {
        ui.set_min_width(250.0);
        // availability: one of four, like a presence menu. "Invisible"
        // is Firestorm's button of the friends list that hides our online
        // status from every friend (GlobalOnlineStatusToggle)
        let mut m = status;
        let current = if m.invisible {
            3
        } else if m.dnd {
            2
        } else if m.away {
            1
        } else {
            0
        };
        for (i, label) in [
            aurora_i18n::tr!("ui-bars-available"),
            aurora_i18n::tr!("common-away"),
            aurora_i18n::tr!("common-do-not-disturb"),
            aurora_i18n::tr!("ui-bars-invisible"),
        ]
        .into_iter()
        .enumerate()
        {
            // Preview each choice through the same shape and colour as the account dot.
            let choice = crate::world::status::StatusModes {
                away: i == 1,
                dnd: i == 2,
                invisible: i == 3,
                ..Default::default()
            };
            if menu::status_toggle(ui, p, status_shape(&choice), status_color(p, &choice), label, current == i) && current != i {
                m.away = i == 1;
                m.dnd = i == 2;
                m.invisible = i == 3;
                *action = BarAction::SetStatus(m);
            }
        }
        menu::separator(ui, p);
        let mut sub = BarAction::None;
        // automatic responses and refusals: the same entries as the menu
        let mut rest = status;
        rest.away = m.away;
        rest.dnd = m.dnd;
        rest.invisible = m.invisible;
        let mut c = false;
        c |= menu::toggle(ui, p, AUTO_REPLY, aurora_i18n::tr!("ui-bars-auto-reply"), &mut rest.autorespond);
        let non_friends = aurora_i18n::tr!("ui-bars-auto-reply-to-non-friends");
        c |= menu::toggle(ui, p, AUTO_REPLY, non_friends, &mut rest.autorespond_nonfriends);
        menu::separator(ui, p);
        let teleports = aurora_i18n::tr!("ui-bars-reject-teleports");
        c |= menu::toggle(ui, p, NO_TELEPORT, teleports, &mut rest.reject_teleports);
        let invites = aurora_i18n::tr!("ui-bars-reject-group-invitations");
        c |= menu::toggle(ui, p, NO_INVITE, invites, &mut rest.reject_group_invites);
        let friendship = aurora_i18n::tr!("ui-bars-reject-friendship-requests");
        c |= menu::toggle(ui, p, NO_FRIENDSHIP, friendship, &mut rest.reject_friendship);
        let ad_hoc = aurora_i18n::tr!("ui-bars-ignore-ad-hoc-conferences");
        c |= menu::toggle(ui, p, AD_HOC, ad_hoc, &mut rest.ignore_adhoc);
        if c {
            sub = BarAction::SetStatus(rest);
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("ui-bars-auto-reply-messages")) {
            sub = BarAction::ChatPrefs;
        }
        if !matches!(sub, BarAction::None) {
            *action = sub;
        }
    }
}

/// Viewer menus: the submenus of the wolf menu button of the navigation bar
/// (without "Advanced" unless `advanced`). An entry carries the icon its
/// function has in the bottom toolbar and in the world menus.
fn main_menus(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, st: &StatusInfo, action: &mut BarAction, advanced: bool) {
    menu::submenu(ui, p, "user-filled", aurora_i18n::tr!("common-me"), true, |ui| {
        flip_window(
            ui,
            p,
            "shirt-filled",
            aurora_i18n::tr!("ui-bars-appearance"),
            "appearance",
            &mut panels.appearance,
        );
        if menu::item(ui, p, "user-filled", aurora_i18n::tr!("ui-bars-profile")) {
            *action = BarAction::MyProfile;
        }
        // menu_viewer.xml: Marketplace listings, before Buy L$
        if st.marketplace && menu::item(ui, p, "tag-filled", aurora_i18n::tr!("marketplace-listings-menu")) {
            *action = BarAction::MarketplaceListings;
        }
        if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("currency-menu")) {
            *action = BarAction::BuyCurrency;
        }
        // menu_viewer.xml: Avatar › Money Tracker
        let tracker = aurora_i18n::tr!("money-tracker-title");
        if menu::check(ui, p, "receipt-filled", tracker, panels.money_tracker, true) {
            super::money_tracker::toggle(ui.ctx(), &mut panels.money_tracker);
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "home-filled", aurora_i18n::tr!("common-go-home")) {
            *action = BarAction::TeleportHome;
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "armchair", aurora_i18n::tr!("ui-bars-sit-on-ground")) {
            *action = BarAction::SitGround;
        }
        if menu::item(ui, p, "man-filled", aurora_i18n::tr!("common-stand-up")) {
            *action = BarAction::StandUp;
        }
        let fly = if st.flying {
            aurora_i18n::tr!("ui-bars-stop-flying")
        } else {
            aurora_i18n::tr!("common-fly")
        };
        if menu::item(ui, p, "plane-tilt-filled", fly) {
            *action = BarAction::ToggleFly;
        }
        // Firestorm keeps its double-click teleport beside its movement
        let locked = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(egui::Id::unique("movelock_requested")))
            .unwrap_or(false);
        if menu::check(ui, p, "lock-filled", aurora_i18n::tr!("movelock-title"), locked, true) {
            *action = BarAction::ToggleMoveLock;
        }
        if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("movelock-options")) {
            *action = BarAction::MoveLockOptions;
        }
        // menu_viewer.xml: Avatar › Movement › Quickjump, after Movelock
        let quickjump = aurora_i18n::tr!("ui-bars-quickjump");
        if menu::check(ui, p, "arrow-big-up-lines-filled", quickjump, st.quickjump, true) {
            *action = BarAction::ToggleQuickjump;
        }
        if menu::item(ui, p, "swimming", aurora_i18n::tr!("movement-title")) {
            *action = BarAction::MovementEnhancer;
        }
        // Firestorm keeps its double-click teleport beside its movement
        // entries (Avatar › Movement): the four choices of the preference
        menu::submenu(
            ui,
            p,
            "hand-click",
            aurora_i18n::tr!("ui-options-double-click-ground"),
            true,
            |ui| {
                for choice in crate::autopilot::DoubleClick::ALL {
                    if menu::check(ui, p, choice.icon(), choice.label(), st.double_click == choice, true) {
                        *action = BarAction::SetDoubleClick(choice);
                    }
                }
            },
        );
        health_menu(ui, p, &st.health);
        menu::separator(ui, p);
        if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("ui-bars-preferences")) {
            super::widgets::open_floater(ui.ctx(), "options", &mut panels.settings);
        }
        // menu_viewer.xml: Show User Interface (alt|shift|U), after
        // Preferences, Toolbar Buttons and Show HUD Attachments
        if interface_item(ui, p, st) {
            *action = BarAction::ToggleInterface;
        }
        if immersive_item(ui, p) {
            *action = BarAction::ToggleImmersive;
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "logout", aurora_i18n::tr!("ui-bars-log-out")) {
            *action = BarAction::Logout;
        }
        if menu::item(ui, p, "power", aurora_i18n::tr!("ui-bars-quit-aurora-viewer")) {
            *action = BarAction::Quit;
        }
    });
    menu::submenu(ui, p, "messages-filled", aurora_i18n::tr!("ui-bars-communicate"), true, |ui| {
        flip_window(
            ui,
            p,
            "messages-filled",
            aurora_i18n::tr!("common-conversations"),
            "conversations",
            &mut panels.chat,
        );
        let gestures = crate::keybinds::chord_label("KeyG", true, false, false);
        if menu::item_keys(ui, p, "hand-move", aurora_i18n::tr!("gesture-title"), &gestures) {
            super::widgets::toggle_floater(ui.ctx(), "gestures", &mut panels.gestures);
        }
        for (tab, icon, label) in [
            (1, "users", aurora_i18n::tr!("common-friends")),
            (0, "radar-filled", aurora_i18n::tr!("common-nearby-people")),
            (2, "users-group", aurora_i18n::tr!("common-groups")),
            (4, "ban", aurora_i18n::tr!("ui-bars-block-list")),
        ] {
            if menu::item(ui, p, icon, label) {
                super::widgets::open_floater(ui.ctx(), "people", &mut panels.people);
                panels.people_tab = tab;
            }
        }
        menu::separator(ui, p);
        // Firestorm "Online Status" menu
        menu::submenu(ui, p, "circle-filled", aurora_i18n::tr!("ui-bars-online-status"), true, |ui| {
            status_menu(ui, p, st.status, action)
        });
    });
    menu::submenu(ui, p, "world-filled", aurora_i18n::tr!("common-world"), true, |ui| {
        if menu::item_keys(
            ui,
            p,
            "refresh",
            aurora_i18n::tr!("toolbar-command-resync-animations"),
            &st.health.resync_keys,
        ) {
            *action = BarAction::ResyncAnimations;
        }
        menu::separator(ui, p);
        menu::submenu(ui, p, "device-desktop-filled", "HUDs", true, |ui| {
            let mut visible = st.show_huds;
            if flip(ui, p, "eye-filled", aurora_i18n::tr!("ui-bars-show-huds"), &mut visible, true) {
                *action = BarAction::ShowHuds(visible);
            }
            for (zoom, icon, label) in [
                (1.0, "restore", aurora_i18n::tr!("ui-bars-normal-size")),
                (
                    (st.hud_zoom / 1.2).max(0.1),
                    "zoom-out-filled",
                    aurora_i18n::tr!("ui-bars-zoom-out"),
                ),
                ((st.hud_zoom * 1.2).min(1.0), "zoom-in-filled", aurora_i18n::tr!("ui-bars-zoom-in")),
            ] {
                if menu::item(ui, p, icon, label) {
                    *action = BarAction::HudZoom(zoom);
                }
            }
        });
        // menu_viewer.xml: Teleport history (Alt+H), Places
        if menu::item(ui, p, "door-enter", aurora_i18n::tr!("portal-title")) {
            *action = BarAction::Portals;
        }
        let history = crate::keybinds::chord_label("KeyH", false, true, false);
        if menu::item_keys(ui, p, "history", aurora_i18n::tr!("common-teleport-history"), &history) {
            *action = BarAction::TeleportHistory;
        }
        flip_window(
            ui,
            p,
            "map-pin-filled",
            aurora_i18n::tr!("common-places"),
            "places",
            &mut panels.places,
        );
        // menu_viewer.xml: Parcel Details (About Land), then Region Details
        // (Region / Estate, Alt+R)
        // (About Land's floater has an id per layout: the flag alone)
        if menu::check(
            ui,
            p,
            "info-circle-filled",
            aurora_i18n::tr!("common-about-land"),
            panels.about_land,
            true,
        ) {
            panels.about_land = !panels.about_land;
        }
        let region = crate::keybinds::chord_label("KeyR", false, true, false);
        if menu::item_keys(ui, p, "building-castle", aurora_i18n::tr!("region-title"), &region) {
            super::widgets::toggle_floater(ui.ctx(), super::region::WINDOW, &mut panels.region_estate);
        }
        // menu_viewer.xml: Set Home to Here, after Region Details
        if menu::item_if(ui, p, "home-check", aurora_i18n::tr!("world-set-home-here"), st.can_set_home) {
            *action = BarAction::SetHome;
        }
        // menu_viewer.xml: Search (Ctrl+F)
        let search = crate::keybinds::chord_label("KeyF", true, false, false);
        if menu::item_keys(ui, p, "search-filled", aurora_i18n::tr!("search-title"), &search) {
            super::widgets::toggle_floater(ui.ctx(), super::search::WINDOW, &mut panels.search);
        }
        flip_window(
            ui,
            p,
            "zoom-scan-filled",
            aurora_i18n::tr!("toolbar-command-area-search"),
            super::area_search::WINDOW,
            &mut panels.area_search,
        );
        // menu_viewer.xml: SL Marketplace, after Search
        if st.marketplace && menu::item(ui, p, "building-store", aurora_i18n::tr!("marketplace-menu")) {
            *action = BarAction::Marketplace;
        }
        flip(
            ui,
            p,
            "compass-filled",
            aurora_i18n::tr!("common-mini-map"),
            &mut panels.minimap,
            true,
        );
        flip_window(
            ui,
            p,
            "map",
            aurora_i18n::tr!("common-world-map"),
            "worldmap",
            &mut panels.world_map,
        );
        // menu_viewer.xml: Stream Title (its window keeps the history of
        // the titles), then Landmark this Place, after the maps
        flip_window(
            ui,
            p,
            "playlist-filled",
            aurora_i18n::tr!("ui-bars-stream-title"),
            super::stream_title::WINDOW,
            &mut panels.stream_title,
        );
        if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("landmark-create-here")) {
            *action = BarAction::CreateLandmark;
        }
        if menu::item(ui, p, "map", aurora_i18n::tr!("owned-land-title")) {
            *action = BarAction::OwnedLand;
        }
        menu::submenu(ui, p, "eye-filled", aurora_i18n::tr!("world-show-more"), true, |ui| {
            flip_window(
                ui,
                p,
                "radar-filled",
                aurora_i18n::tr!("beacons-title"),
                super::beacons::WINDOW,
                &mut panels.beacons,
            );
            let mut overlays = st.world_overlays;
            let before = overlays;
            flip(
                ui,
                p,
                "border-all",
                aurora_i18n::tr!("world-property-lines"),
                &mut overlays.property_lines,
                true,
            );
            flip(ui, p, "map", aurora_i18n::tr!("world-land-owners"), &mut overlays.land_owners, true);
            if overlays != before {
                *action = BarAction::WorldOverlays(overlays);
            }
            if menu::check(
                ui,
                p,
                "layers-subtract",
                aurora_i18n::tr!("common-show-transparency"),
                st.show_transparency,
                true,
            ) {
                *action = BarAction::ShowTransparency;
            }
            if flip(
                ui,
                p,
                "sparkles-filled",
                aurora_i18n::tr!("world-hide-particles"),
                &mut overlays.hide_particles,
                true,
            ) {
                *action = BarAction::WorldOverlays(overlays);
            }
        });
        menu::submenu(ui, p, "barrier-block-filled", aurora_i18n::tr!("ui-bars-ban-lines"), true, |ui| {
            for (v, icon, label) in [
                (0u8, "eye-off", aurora_i18n::tr!("ui-bars-hidden")),
                (1, "alert-triangle-filled", aurora_i18n::tr!("ui-bars-on-collision")),
                (2, "radar-filled", aurora_i18n::tr!("common-nearby")),
            ] {
                if menu::check(ui, p, icon, label, st.ban_lines == v, true) {
                    *action = BarAction::BanLines(v);
                }
            }
        });
        // menu_viewer.xml: Show Info › Show Tips (Ctrl+Shift+T), Show Land
        // Tooltips, Show Tips On All Objects
        menu::submenu(ui, p, "pointer-filled", aurora_i18n::tr!("ui-options-hover-tips"), true, |ui| {
            let mut tips = st.hover_tips;
            let mut changed = menu::check(
                ui,
                p,
                "pointer-filled",
                aurora_i18n::tr!("ui-bars-hover-tips-show"),
                tips.enabled,
                true,
            );
            if changed {
                tips.enabled = !tips.enabled;
            }
            if menu::check(
                ui,
                p,
                "map-pin-filled",
                aurora_i18n::tr!("ui-options-hover-tips-land"),
                tips.land,
                tips.enabled,
            ) {
                tips.land = !tips.land;
                changed = true;
            }
            if menu::check(
                ui,
                p,
                "cube",
                aurora_i18n::tr!("ui-bars-hover-tips-all-objects"),
                tips.all_objects,
                tips.enabled,
            ) {
                tips.all_objects = !tips.all_objects;
                changed = true;
            }
            if changed {
                *action = BarAction::HoverTips(tips);
            }
        });
        menu::submenu(ui, p, "sunset-2-filled", aurora_i18n::tr!("common-environment"), true, |ui| {
            // back to the region / parcel EEP cycle, in real time
            let shared = !st.local_env;
            if menu::check_keys(
                ui,
                p,
                "world-filled",
                aurora_i18n::tr!("ui-bars-use-shared-environments"),
                shared,
                true,
                &st.env_keys[0],
            ) {
                panels.time_of_day = 0;
                *action = BarAction::SharedEnvironment;
            }
            menu::separator(ui, p);
            let icons = [
                "",
                "sunrise-filled",
                "sun-high-filled",
                "sunset-2-filled",
                "moon-filled",
                "sun-high-filled",
            ];
            let labels = time_of_day();
            for i in crate::world::env::TIME_OF_DAY_ORDER {
                let n = i as usize;
                if menu::check_keys(ui, p, icons[n], labels[n], panels.time_of_day == i, true, &st.env_keys[n]) {
                    panels.time_of_day = i;
                }
            }
            menu::separator(ui, p);
            // Firestorm's quick preferences lists, and World > Environment >
            // Personal Lighting...
            if menu::item(ui, p, "sun-filled", aurora_i18n::tr!("ui-bars-environment-picker")) {
                panels.environment = true;
            }
            if menu::item(ui, p, "adjustments-filled", aurora_i18n::tr!("common-personal-lighting")) {
                panels.personal_lighting = true;
            }
            // Firestorm's Phototools "New preset" buttons (World.EnvPreset)
            menu::separator(ui, p);
            use crate::world::env_select::SettingsKind;
            for (kind, icon, label) in [
                (SettingsKind::Sky, "sun-filled", aurora_i18n::tr!("ui-bars-new-sky")),
                (SettingsKind::Water, "droplet-filled", aurora_i18n::tr!("ui-bars-new-water")),
                (SettingsKind::Day, "sunset-2-filled", aurora_i18n::tr!("ui-bars-new-day-cycle")),
            ] {
                if menu::item(ui, p, icon, label) {
                    *action = BarAction::NewEnvironment(kind);
                }
            }
        });
        // menu_viewer.xml: World › Asset Blacklist, after the explorers
        flip_window(
            ui,
            p,
            "music",
            aurora_i18n::tr!("toolbar-command-sound-explorer"),
            super::sound_explorer::WINDOW,
            &mut panels.sound_explorer,
        );
        flip_window(
            ui,
            p,
            "run",
            aurora_i18n::tr!("ui-bars-animation-explorer"),
            super::animation_explorer::WINDOW,
            &mut panels.animation_explorer,
        );
        flip_window(
            ui,
            p,
            "eye-off",
            aurora_i18n::tr!("blacklist-title"),
            super::blacklist::WINDOW,
            &mut panels.asset_blacklist,
        );
        flip_window(
            ui,
            p,
            "eye-filled",
            aurora_i18n::tr!("avatar-render-title"),
            super::avatar_render_settings::WINDOW,
            &mut panels.avatar_render_settings,
        );
        menu::separator(ui, p);
        // Firestorm's View › Camera Controls and Movement Controls
        let camera = crate::keybinds::chord_label("KeyC", true, false, true);
        if menu::item_keys(ui, p, "video-filled", aurora_i18n::tr!("controls-camera-title"), &camera) {
            panels.camera_controls = true;
        }
        if menu::item(ui, p, "walk", aurora_i18n::tr!("controls-movement-title")) {
            panels.movement_controls = true;
        }
        if menu::item(ui, p, "eye-filled", aurora_i18n::tr!("common-mouselook-m")) {
            *action = BarAction::ToggleMouselook;
        }
        if menu::item(ui, p, "camera-filled", aurora_i18n::tr!("ui-bars-reset-camera-esc")) {
            *action = BarAction::ResetCamera;
        }
        let photo = crate::keybinds::chord_label("KeyS", true, false, true);
        if menu::item_keys(ui, p, "photo-filled", aurora_i18n::tr!("photo-title"), &photo) {
            super::widgets::toggle_floater(ui.ctx(), super::snapshot::WINDOW, &mut panels.snapshot);
        }
    });
    menu::submenu(ui, p, "box", aurora_i18n::tr!("common-build"), true, |ui| {
        build_menu(ui, p, action, &st.ik_keys, st.edit_particles);
    });
    menu::submenu(ui, p, "briefcase-filled", aurora_i18n::tr!("common-contents"), true, |ui| {
        contents_menu(ui, p, panels, action, st.marketplace, st.motd);
    });
    if advanced {
        menu::submenu(ui, p, "adjustments-filled", aurora_i18n::tr!("common-advanced"), true, |ui| {
            let perf = aurora_i18n::tr!("ui-bars-performance-ctrl-shift-1");
            flip_window(ui, p, "gauge-filled", perf, "perf", &mut panels.perf);
            let graphics = aurora_i18n::tr!("ui-bars-graphics-preferences");
            flip_window(ui, p, "settings-filled", graphics, "options", &mut panels.settings);
            let toolbox = aurora_i18n::tr!("dev-toolbox-menu");
            flip_window(
                ui,
                p,
                "code-circle-filled",
                toolbox,
                crate::ui::dev_toolbox::WINDOW,
                &mut panels.dev_toolbox,
            );
            // Develop › Consoles › Region Debug Console, for those the
            // region gives the console to
            if st.region_console {
                let console = aurora_i18n::tr!("sim-console-menu");
                flip_window(
                    ui,
                    p,
                    "terminal-2",
                    console,
                    crate::ui::sim_console::WINDOW,
                    &mut panels.sim_console,
                );
            }
            // Advanced › Quiet Snapshots (PlayModeUISndSnapshot)
            let quiet = aurora_i18n::tr!("photo-quiet-snapshots");
            if menu::check(ui, p, "camera-off", quiet, st.quiet_snapshots, true) {
                *action = BarAction::ToggleQuietSnapshots;
            }
            // Advanced › Media Streams Backup: the parcel stream list to
            // and from an XML file (Streamlist.xml_import / xml_export)
            menu::submenu(
                ui,
                p,
                "database-filled",
                aurora_i18n::tr!("ui-bars-media-streams-backup"),
                true,
                |ui| {
                    if menu::item(ui, p, "database-import", aurora_i18n::tr!("ui-bars-import-stream-list")) {
                        *action = BarAction::ImportStreamList;
                    }
                    if menu::item(ui, p, "database-export", aurora_i18n::tr!("ui-bars-export-stream-list")) {
                        *action = BarAction::ExportStreamList;
                    }
                },
            );
        });
    }
    menu::submenu(ui, p, "help-circle-filled", aurora_i18n::tr!("common-help"), true, |ui| {
        // menu_viewer.xml: the viewer's wiki and the grid's help first, then
        // Check Grid status before the reports. The wiki is the project's
        // own, without the Legacy Viewer's F1 (the favourites' key here).
        // The grid's help shows only when the grid info gives a help page
        // (update_grid_help, llviewermenu.cpp).
        use super::context::{CtxAction, request};
        if menu::item(ui, p, "book-filled", aurora_i18n::tr!("ui-bars-wiki")) {
            request(ui.ctx(), CtxAction::OpenUrl(crate::links::WIKI.into()));
        }
        if let Some(help) = &st.grid_help
            && menu::item_detail(ui, p, "article-filled", aurora_i18n::tr!("ui-bars-knowledge-base"), &help.grid)
        {
            request(ui.ctx(), CtxAction::OpenUrl(help.url.clone()));
        }
        menu::separator(ui, p);
        let grid_status = aurora_i18n::tr!("toolbar-command-grid-status");
        let tower = "building-broadcast-tower-filled";
        flip_window(ui, p, tower, grid_status, super::grid_status::WINDOW, &mut panels.grid_status);
        if crate::updater::checks_enabled() && menu::item(ui, p, "download-filled", aurora_i18n::tr!("update-check")) {
            *action = BarAction::CheckUpdates;
        }
        if menu::item(ui, p, "bug-filled", aurora_i18n::tr!("bug-title")) {
            *action = BarAction::ReportBug;
        }
        if menu::item(ui, p, "flag-filled", aurora_i18n::tr!("report-abuse-title")) {
            *action = BarAction::ReportAbuse;
        }
        // menu_viewer.xml: Help › Bumps, Pushes & Hits, after the reports
        let bumps = aurora_i18n::tr!("bumps-title");
        flip_window(ui, p, "bounce-right-filled", bumps, super::bumps::WINDOW, &mut panels.bumps);
        if st.release_notes && menu::item(ui, p, "file-text-filled", aurora_i18n::tr!("help-server-release-notes")) {
            *action = BarAction::ServerReleaseNotes;
        }
        menu::separator(ui, p);
        // Enable Sysinfo Button (SysinfoButtonInIM), then About, whose
        // window carries the system information
        menu::todo(ui, p, "device-desktop-filled", aurora_i18n::tr!("ui-bars-sysinfo-button"));
        menu::separator(ui, p);
        let about = aurora_i18n::tr!("ui-bars-about");
        flip_window(ui, p, "info-circle-filled", about, super::about::WINDOW, &mut panels.about);
        let version = aurora_i18n::tr!("common-aurora-viewer", value0 = aurora_version::version().to_string());
        menu::info(ui, p, "info-circle-filled", &version);
        menu::separator(ui, p);
        menu::info(ui, p, "walk", aurora_i18n::tr!("ui-bars-walk-arrow-keys-double-tap-to-run"));
        menu::info(
            ui,
            p,
            "plane-tilt-filled",
            aurora_i18n::tr!("extra-jump-ascend-page-crouch-page-fly-home"),
        );
        menu::info(ui, p, "eye-filled", aurora_i18n::tr!("ui-bars-mouselook-m-chat-enter"));
        menu::info(ui, p, "mouse-filled", aurora_i18n::tr!("ui-bars-right-click-drag-orbit-wheel-zoom"));
        menu::info(
            ui,
            p,
            "keyboard-filled",
            aurora_i18n::tr!("ui-bars-editable-shortcuts-preferences-shortcuts"),
        );
    });
}

/// Me › Avatar health (menu_viewer.xml "avhealth"): the usual fixes of a
/// broken avatar, in the Legacy Viewer's order. Its default avatar resets
/// ("Character Test") and complexity information are left out.
fn health_menu(ui: &mut egui::Ui, p: &Palette, h: &HealthMenu) {
    use super::context::{CtxAction, request};
    menu::submenu(ui, p, "medical-cross-filled", aurora_i18n::tr!("avatar-health-menu"), true, |ui| {
        if menu::item_keys(
            ui,
            p,
            "player-stop-filled",
            aurora_i18n::tr!("avatar-health-stop-animations"),
            &h.stop_keys,
        ) {
            request(ui.ctx(), CtxAction::StopAnimationsKeepPermissions);
        }
        if menu::item(
            ui,
            p,
            "player-stop-filled",
            aurora_i18n::tr!("avatar-health-stop-animations-revoke"),
        ) {
            request(ui.ctx(), CtxAction::StopAnimations);
        }
        if menu::item(ui, p, "stretching", aurora_i18n::tr!("avatar-health-undeform")) {
            request(ui.ctx(), CtxAction::Undeform);
        }
        if menu::item(ui, p, "bone-filled", aurora_i18n::tr!("ui-context-reset-skeleton")) {
            request(
                ui.ctx(),
                CtxAction::ResetSkeleton {
                    avatar: h.me,
                    animations: false,
                },
            );
        }
        if menu::item(ui, p, "run", aurora_i18n::tr!("ui-context-reset-skeleton-and-animations")) {
            request(
                ui.ctx(),
                CtxAction::ResetSkeleton {
                    avatar: h.me,
                    animations: true,
                },
            );
        }
        if menu::item_keys(ui, p, "reload", aurora_i18n::tr!("ui-context-rebake"), &h.rebake_keys) {
            request(ui.ctx(), CtxAction::Rebake);
        }
        if menu::item_keys(
            ui,
            p,
            "refresh",
            aurora_i18n::tr!("avatar-health-refresh-attachments"),
            &h.refresh_keys,
        ) {
            request(ui.ctx(), CtxAction::RefreshAttachments);
        }
    });
}

fn build_menu(ui: &mut egui::Ui, p: &Palette, action: &mut BarAction, ik_keys: &str, edit_particles: bool) {
    if menu::item(ui, p, "wand", aurora_i18n::tr!("poser-title")) {
        *action = BarAction::Poser;
    }
    if menu::item_keys(ui, p, "bone-filled", aurora_i18n::tr!("ik-edit-title"), ik_keys) {
        *action = BarAction::IkEdit;
    }
    for (mode, icon, label) in [
        (0, "box", aurora_i18n::tr!("ui-bars-build-ctrl-b")),
        (1, "pencil-filled", aurora_i18n::tr!("ui-bars-edit-objects-ctrl-3")),
        (2, "circle-plus-filled", aurora_i18n::tr!("ui-bars-create-prims-ctrl-4")),
        (3, "mountain-filled", aurora_i18n::tr!("ui-bars-edit-land-ctrl-5")),
    ] {
        if menu::item(ui, p, icon, label) {
            *action = BarAction::Build(mode);
        }
    }
    // menu_viewer.xml: Build › Object › Edit Particles, greyed out until an
    // object we own is selected (LLEnableEditParticleSource)
    if menu::item_tip(
        ui,
        p,
        "sparkles-filled",
        aurora_i18n::tr!("ui-bars-edit-particles"),
        edit_particles,
        if edit_particles {
            ""
        } else {
            aurora_i18n::tr!("particle-editor-menu-tip")
        },
    ) {
        *action = BarAction::EditParticles;
    }
    menu::separator(ui, p);
    if menu::item(ui, p, "file-upload-filled", aurora_i18n::tr!("import-menu")) {
        *action = BarAction::Import;
    }
    if menu::item(ui, p, "cube-3d-sphere", aurora_i18n::tr!("blender-live-title")) {
        *action = BarAction::BlenderLive;
    }
}

/// LindenXchange and Script Library of menu_viewer.xml (PromptShowURL).
const LINDEX_MARKET: &str = "https://secondlife.com/my/lindex/market.php";
const SCRIPT_LIBRARY: &str = "https://wiki.secondlife.com/wiki/LSL_Library";

/// Inventory and gestures, then the entries of the Legacy Viewer's Content
/// menu that Aurora's World menu does not hold already (Search and SL
/// Marketplace are there). L$ Market Data shows on the Second Life grids
/// only (GridCheck "secondlife"), as the Marketplace.
///
/// The pages go through the links' usual path (`CtxAction::OpenUrl`): the
/// Legacy Viewer's WebLaunchExternalTarget question is Aurora's link check,
/// which lets the well-known sites through and asks for the others.
/// Message of the day is greyed out when the grid sent none, instead of
/// writing an empty line.
fn contents_menu(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, action: &mut BarAction, marketplace: bool, motd: bool) {
    use super::context::{CtxAction, request};
    flip_window(
        ui,
        p,
        "briefcase-filled",
        aurora_i18n::tr!("common-inventory"),
        "inventory",
        &mut panels.inventory,
    );
    let gestures = crate::keybinds::chord_label("KeyG", true, false, false);
    if menu::item_keys(ui, p, "hand-move", aurora_i18n::tr!("gesture-title"), &gestures) {
        super::widgets::toggle_floater(ui.ctx(), "gestures", &mut panels.gestures);
    }
    menu::separator(ui, p);
    if marketplace && menu::item(ui, p, "chart-candle-filled", aurora_i18n::tr!("ui-bars-market-data")) {
        request(ui.ctx(), CtxAction::OpenUrl(LINDEX_MARKET.into()));
    }
    if menu::item(ui, p, "code", aurora_i18n::tr!("ui-bars-script-library")) {
        request(ui.ctx(), CtxAction::OpenUrl(SCRIPT_LIBRARY.into()));
    }
    menu::separator(ui, p);
    if menu::item_if(ui, p, "message-filled", aurora_i18n::tr!("ui-bars-message-of-the-day"), motd) {
        *action = BarAction::MessageOfTheDay;
    }
}

/// Draw a (white mask) icon tinted, centered in `rect`.
fn paint_icon(ui: &egui::Ui, icons: &Icons, name: &str, rect: egui::Rect, size: f32, tint: Color32) {
    if let Some(t) = icons.get(name) {
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// Back / forward arrow of the navigation bar; `dest` is the history entry
/// it leads to (None = disabled).
/// A back / forward arrow: whether it was clicked, and the place chosen
/// in its list of the session's places.
fn nav_arrow(ui: &mut egui::Ui, p: &Palette, icons: &Icons, left: bool, st: &StatusInfo) -> (bool, Option<BarAction>) {
    let dest = if left { st.tp_back.as_deref() } else { st.tp_forward.as_deref() };
    let (rect, resp) = ui.allocate_exact_size(TOP_SLOT, egui::Sense::click());
    let enabled = dest.is_some();
    let hot = resp.hovered() || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if enabled && hot {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    let col = if !enabled {
        p.muted_dim.gamma_multiply(0.6)
    } else if hot {
        p.ink
    } else {
        p.muted
    };
    paint_icon(ui, icons, if left { "arrow-left" } else { "arrow-right" }, rect, TOP_ICON, col);
    let resp = resp.on_hover_text(arrow_tip(left, dest, st));
    if left {
        islands::demo_open(ui.ctx(), &resp, demo::Open::TpList);
    }
    let (clicked, chosen) = tp_list::arrow(ui, p, &resp, st, None);
    (clicked && enabled, chosen)
}

/// The tooltip of a back / forward arrow: where it leads, and how to list
/// the session's places when there are some.
fn arrow_tip(left: bool, dest: Option<&str>, st: &StatusInfo) -> String {
    let tip = match dest {
        Some(d) if left => aurora_i18n::tr!("ui-bars-back", d = d.to_string()),
        Some(d) => aurora_i18n::tr!("ui-bars-next", d = d.to_string()),
        None if left => aurora_i18n::tr!("ui-bars-no-previous-destination").to_owned(),
        None => aurora_i18n::tr!("ui-bars-no-next-destination").to_owned(),
    };
    if dest.is_some() && tp_list::has_list(st) {
        format!("{tip}\n{}", aurora_i18n::tr!("ui-bars-tp-list-tip"))
    } else {
        tip
    }
}

/// Circled "i" (About Land) at the start of the location field.
fn info_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), egui::Sense::click());
    let col = if resp.hovered() { p.ink } else { p.muted };
    paint_icon(ui, icons, "info-circle-filled", rect, 16.0, col);
    resp.on_hover_text(aurora_i18n::tr!("common-about-land")).clicked()
}

/// Width kept for the landmark star at the right end of the location field.
const STAR_W: f32 = 22.0;

/// LLLocationInputCtrl's add landmark button: an outline star opening the
/// quick menu (Favorites, Landmarks, the Create Landmark window), lit in
/// amber when a landmark of the Favorites or Landmarks folders lies on the
/// agent's parcel; a click then edits it.
fn landmark_star(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    found: Option<&super::landmark_create::ParcelLandmark>,
) -> Option<StarAction> {
    use super::parcel_icons::{GLYPH, SLOT_HEIGHT, SLOT_WIDTH, glyph_rect};
    // the slot and the glyph of the parcel icons on its left: same size,
    // same centering, same hover face
    let (slot, resp) = ui.allocate_exact_size(Vec2::new(SLOT_WIDTH, SLOT_HEIGHT), egui::Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(slot.expand(1.0), ui.visuals().window_corner_radius, p.raised);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let glyph = glyph_rect(slot);
    if let Some(lm) = found {
        paint_icon(ui, icons, "star-filled", glyph, GLYPH, p.amber);
        let tip = if lm.favorite {
            aurora_i18n::tr!("landmark-star-in-favorites", name = lm.name.clone())
        } else {
            aurora_i18n::tr!("landmark-star-in-landmarks", name = lm.name.clone())
        };
        return resp.on_hover_text(tip).clicked().then_some(StarAction::Edit(lm.item));
    }
    paint_icon(ui, icons, "star", glyph, GLYPH, if resp.hovered() { p.ink } else { p.muted });
    let mut out = None;
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        out = star_menu(ui, p);
    });
    resp.on_hover_text(aurora_i18n::tr!("landmark-star-add"));
    out
}

/// The outline star's quick menu: Favorites, Landmarks, the Create Landmark
/// window.
fn star_menu(ui: &mut egui::Ui, p: &Palette) -> Option<StarAction> {
    let mut out = None;
    if menu::item(ui, p, "star-filled", aurora_i18n::tr!("landmark-star-add-favorites")) {
        out = Some(StarAction::AddFavorite);
    }
    if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("landmark-star-add-landmarks")) {
        out = Some(StarAction::AddLandmark);
    }
    if menu::item(ui, p, "dots-filled", aurora_i18n::tr!("landmark-star-more")) {
        out = Some(StarAction::MoreOptions);
    }
    out
}

/// Height of the chips of the top bar (voice light, flying, sitting).
const CHIP_HEIGHT: f32 = 18.0;

/// A state of our avatar (flying, sitting): the accent tint of an active
/// toolbar button, its label centered on its letters.
fn state_chip(ui: &mut egui::Ui, p: &Palette, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.5), p.ink);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 14.0, CHIP_HEIGHT), egui::Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.40));
    super::widgets::paint_ink_centered(ui.painter(), galley, rect.center(), p.ink);
}

/// The colour and the name of the voice connection's state.
fn voice_state(p: &Palette, light: crate::voice::VoiceLight) -> (Color32, &'static str) {
    use crate::voice::VoiceLight as L;
    match light {
        L::Connected => (p.success, aurora_i18n::tr!("common-voice-connected")),
        L::Connecting => (p.amber, aurora_i18n::tr!("common-connecting-to-voice")),
        L::Down => (p.danger, aurora_i18n::tr!("ui-bars-voice-not-connected")),
        L::Off => (p.muted_dim, aurora_i18n::tr!("common-voice-disabled")),
    }
}

/// The voice light as a dot on the microphone's upper right, ringed with
/// the bar's colour, pulsing while connecting (the mockup's `.vdot`).
fn voice_dot(ui: &egui::Ui, p: &Palette, icon: egui::Rect, light: crate::voice::VoiceLight) {
    let (col, _) = voice_state(p, light);
    let center = icon.right_top() + Vec2::new(0.5, 1.5);
    let col = if light == crate::voice::VoiceLight::Connecting {
        let t = ui.input(|i| i.time) as f32;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        col.gamma_multiply(0.55 + 0.45 * (t * 5.0).sin().abs())
    } else {
        col
    };
    ui.painter().circle_filled(center, 3.5, p.bar);
    ui.painter().circle_filled(center, 2.25, col);
}

/// Voice connection light: green connected, orange connecting, red not
/// connected, grey turned off. Returns true when clicked (voice settings).
fn voice_light(ui: &mut egui::Ui, p: &Palette, icons: &Icons, light: crate::voice::VoiceLight, why: &str) -> bool {
    use crate::voice::VoiceLight as L;
    let (col, state) = voice_state(p, light);
    // as wide as its content, with the same margin on both sides (the label
    // changes with the language)
    const MARGIN: f32 = 6.0;
    const LABEL_AT: f32 = MARGIN + 25.0;
    let font = egui::FontId::proportional(10.5);
    let label_width = ui
        .painter()
        .layout_no_wrap(aurora_i18n::tr!("common-voice").to_owned(), font.clone(), p.muted)
        .size()
        .x;
    let width = (LABEL_AT + label_width + MARGIN).ceil();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, CHIP_HEIGHT), egui::Sense::click());
    let painter = ui.painter();
    let radius = ui.visuals().window_corner_radius;
    let face = if resp.hovered() {
        p.raised.lerp_to_gamma(p.ink, 0.16)
    } else {
        p.raised
    };
    painter.rect_filled(rect, radius, face);
    let dot = egui::pos2(rect.left() + MARGIN + 4.0, rect.center().y);
    if light == L::Connecting {
        // pulse while connecting
        let t = ui.input(|i| i.time) as f32;
        let a = 0.55 + 0.45 * (t * 5.0).sin().abs();
        painter.circle_filled(dot, 4.0, col.gamma_multiply(a));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    } else {
        painter.circle_filled(dot, 4.0, col);
    }
    if light == L::Connected {
        painter.circle_stroke(dot, 6.0, egui::Stroke::new(1.0, col.gamma_multiply(0.35)));
    }
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + MARGIN + 17.0, rect.center().y), Vec2::splat(12.0));
    paint_icon(
        ui,
        icons,
        "microphone-filled",
        icon_rect,
        12.0,
        if resp.hovered() { p.ink } else { p.muted },
    );
    let color = if resp.hovered() { p.ink } else { p.muted };
    let label = ui
        .painter()
        .layout_no_wrap(aurora_i18n::tr!("common-voice").to_owned(), font, color);
    super::widgets::paint_ink_left(ui.painter(), label, egui::pos2(rect.left() + LABEL_AT, rect.center().y), color);
    let tip = if why.is_empty() || why == state {
        state.to_owned()
    } else {
        format!("{state}\n{why}")
    };
    resp.on_hover_text(aurora_i18n::tr!("ui-bars-click-sound-and-voice-settings", tip = tip.to_string()))
        .clicked()
}

/// Location of the navigation bar (LLLocationInputCtrl): shows "Parcel,
/// Region (x, y, z)"; a click turns it into the SLURL of the position,
/// selected and ready to copy; typing or pasting a place then Enter
/// teleports there; Escape or a click elsewhere restores the location.
fn location_field(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, loc: &str, slurl: &str, w: f32) -> Option<BarAction> {
    let id = egui::Id::unique("nav_location_edit");
    let Some(text) = panels.nav_edit.as_mut() else {
        // exactly the text width (truncated when too long): the maturity badge
        // stays right after the location
        let text_w = ui
            .painter()
            .layout_no_wrap(loc.to_owned(), egui::FontId::proportional(12.0), p.ink)
            .size()
            .x;
        let lw = text_w.min(w).ceil();
        let resp = ui
            .allocate_ui_with_layout(Vec2::new(lw, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_max_width(lw);
                ui.add(egui::Label::new(small(loc, p.ink)).truncate().sense(egui::Sense::click()))
            })
            .inner
            .on_hover_text(aurora_i18n::tr!(
                "ui-bars-click-to-copy-the-slurl-or-enter-a-destination",
                loc = loc.to_string()
            ));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        if resp.clicked() {
            panels.nav_edit = Some(slurl.to_owned());
            panels.nav_edit_new = true;
        }
        return None;
    };
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE.inner_margin(crate::ui::widgets::field_text_margin(ui.ctx(), &egui::FontId::proportional(12.0))))
            .font(egui::FontId::proportional(12.0))
            .text_color(p.ink)
            .hint_text(aurora_i18n::tr!("ui-bars-slurl-region-x-y-z-or-region-name"))
            .desired_width(w),
    );
    if panels.nav_edit_new {
        // focus first; once focused, select everything (ready to copy, or to
        // be replaced by typing / pasting)
        resp.request_focus();
        if resp.has_focus() {
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            let n = text.chars().count();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(n),
            )));
            state.store(ui.ctx(), id);
            panels.nav_edit_new = false;
        }
        ui.ctx().request_repaint();
        return None;
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if enter {
        let typed = std::mem::take(text);
        panels.nav_edit = None;
        if !typed.trim().is_empty() {
            return Some(BarAction::TeleportTo(typed));
        }
    } else if escape || !resp.has_focus() {
        // Escape or a click elsewhere: back to the location as it was
        panels.nav_edit = None;
        ui.memory_mut(|m| m.surrender_focus(id));
    }
    None
}

fn small(text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).size(12.0).color(color)
}

/// A readout of the navigation bar as wide as it was lately
/// (`islands::held_width`), its text centred: the frame rate, the latency
/// or the clock changing does not move what is left of it (#1246).
fn steady_label(ui: &mut egui::Ui, id: &str, text: String, color: Color32, sense: egui::Sense) -> egui::Response {
    let w = islands::held_width(ui, egui::Id::unique(("bars_legacy_readout", id)), &text, 12.0);
    let h = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(12.0)));
    ui.add_sized(Vec2::new(w, h), egui::Label::new(small(text, color)).sense(sense))
}

/// The bell of the navigation bar: its count, its style (Preferences ›
/// Aurora › Notifications) and do not disturb (the paused bell, which a
/// right click turns on and off, #491).
#[derive(Debug, Clone, Copy, Default)]
pub struct BellState {
    pub unread: usize,
    pub aurora: bool,
    pub dnd: bool,
}

/// Top chrome: a single navigation bar (Aurora menu, notifications,
/// teleport history, location) with the status on its right (balance,
/// time, ping, media and volumes, frame rate). Aurora's notification center
/// puts its bell at the right end (`bell.aurora`), Firestorm's next to the
/// wolf.
#[allow(clippy::too_many_arguments)]
pub fn top_bars(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    layout: &Layout,
    panels: &mut Panels,
    st: &StatusInfo,
    audio: &mut crate::settings::AudioSettings,
    audio_ui: &mut super::audio::AudioUi,
    media: super::audio::MediaState,
    notif: &mut super::notifications::NotifUi,
    bell: BellState,
) -> (BarAction, super::audio::AudioAction) {
    if !st.classic_bars {
        return islands::top(ui, p, icons, panels, st, audio, audio_ui, media, notif, bell);
    }
    let mut action = BarAction::None;
    let mut audio_action = super::audio::AudioAction::None;
    let mut glass = None;
    let bar = egui::Panel::top("nav_bar")
        .frame(bar_frame(p))
        .exact_size(layout.nav_bar_height)
        .show(ui, |ui| {
            glass = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
            // the bar's background spans the whole window width
            ui.set_min_width(ui.available_width());
            ui.horizontal_centered(|ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.style_mut().visuals.button_frame = false;
                // Aurora menu (white wolf): Me, Communicate, World, Build, Contents, Advanced, Help
                if let Some(t) = icons.get("wolf") {
                    let img = egui::Image::new(t).fit_to_exact_size(Vec2::new(18.0, 18.0)).tint(p.ink);
                    menu::button(ui, p, egui::Button::image(img), |ui| {
                        menu::searchable(ui, p, |ui| main_menus(ui, p, panels, st, &mut action, true));
                    })
                    .on_hover_text(aurora_i18n::tr!("ui-bars-aurora-menu"));
                }
                // Firestorm's notification well: the bell next to the wolf
                if !bell.aurora {
                    let resp = super::notifications::bell(ui, p, icons, bell.unread, notif.open, bell.dnd, false);
                    notif.anchor = Some(resp.rect.left_bottom());
                    notif.bell = Some(resp.rect);
                    if resp.clicked() {
                        notif.open = !notif.open;
                    }
                    if resp.secondary_clicked() {
                        action = BarAction::SetStatus(super::notifications::bell_dnd_toggled(st.status));
                    }
                }
                // online status (away, do not disturb, automatic responses)
                if st.show_bug_report {
                    let resp = icon_button_response(ui, p, icons, "bug-filled", aurora_i18n::tr!("bug-title"), TOP_ICON);
                    if resp.clicked() {
                        action = BarAction::ReportBug;
                    } else if resp.secondary_clicked() {
                        action = BarAction::CopyVersion;
                    }
                }
                status_dot(ui, p, st.status, &mut action);
                // back / forward through the teleport history, home (Firestorm navigation bar)
                for left in [true, false] {
                    let (clicked, chosen) = nav_arrow(ui, p, icons, left, st);
                    if clicked {
                        action = if left {
                            BarAction::TeleportBack
                        } else {
                            BarAction::TeleportForward
                        };
                    }
                    if let Some(a) = chosen {
                        action = a;
                    }
                }
                let home = icon_button_response(ui, p, icons, "Home_Off", aurora_i18n::tr!("common-go-home"), TOP_ICON);
                if home.clicked() {
                    action = BarAction::TeleportHome;
                }
                if home_menu(&home, p, st.can_set_home) {
                    action = BarAction::SetHome;
                }
                ui.add_space(6.0);

                // status on the right (laid out from the right edge), the location
                // field takes what is left
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.add_space(2.0);
                    // Aurora's center: the bell at the right end, after the
                    // frame rate, its cards unroll under it
                    if bell.aurora {
                        // room for the count at the window's edge
                        ui.add_space(4.0);
                        let resp = super::notification_center::bell(ui, p, icons, &notif.center, bell.unread, notif.open, bell.dnd, false);
                        notif.anchor = Some(resp.rect.left_bottom());
                        notif.bell = Some(resp.rect);
                        if resp.clicked() {
                            notif.open = !notif.open;
                        }
                        if resp.secondary_clicked() {
                            action = BarAction::SetStatus(super::notifications::bell_dnd_toggled(st.status));
                        }
                    }
                    let fps_col = if st.fps >= 50.0 {
                        p.teal
                    } else if st.fps >= 25.0 {
                        p.amber
                    } else {
                        p.danger
                    };
                    // a click opens (or closes) the performance window
                    let fps = steady_label(ui, "fps", islands::fps_text(st.fps), fps_col, egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    graphics::hover(&fps, p, st, &mut action);
                    if fps.clicked() {
                        panels.perf = !panels.perf;
                    }
                    audio_action = super::audio::top_controls(ui, p, icons, audio, audio_ui, media, super::audio::BAR_SLOT);
                    if voice_light(ui, p, icons, st.voice.0, &st.voice.1) {
                        action = BarAction::VoicePrefs;
                    }
                    steady_label(ui, "ping", islands::ping_text(st.ping_ms), p.muted, egui::Sense::hover())
                        .on_hover_text(aurora_i18n::tr!("ui-bars-region-latency"));
                    // a left click switches SLT and local time
                    let island = st.system_island;
                    let clock = steady_label(ui, "clock", clock::text(island), p.muted, egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(clock::hover_text(island));
                    if clock.clicked() {
                        action = BarAction::SystemIsland(crate::settings::SystemIsland {
                            clock_zone: clock::next(island.clock_zone),
                            ..island
                        });
                    }
                    if let Some(b) = st.balance.filter(|_| st.show_balance) {
                        // a click opens the L$ purchase, a right click hides it
                        let balance = ui
                            .add(egui::Label::new(small(format!("{b} L$"), p.ink)).sense(egui::Sense::click()))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(aurora_i18n::tr!("ui-bars-balance-click-to-buy-l"));
                        if balance.clicked() {
                            action = BarAction::BuyCurrency;
                        }
                        menu::context_menu(&balance, p, |ui| balance_menu(ui, p, st.show_balance, &mut action));
                    }
                    // the conversations' thumbnails, left of the balance as
                    // the Legacy Viewer's chiclets (#1134)
                    let size = super::chat::thumbnails::BAR_SIZE;
                    let w = super::chat::thumbnails::top_width(st.conversation_thumbnails, size, 0.0, ui.available_width() / 3.0);
                    if w > 0.0 {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, size), egui::Sense::hover());
                        super::chat::thumbnails::reserve_top(ui.ctx(), rect, size, false);
                    }
                    for (on, text) in [
                        (st.flying, aurora_i18n::tr!("ui-bars-flying")),
                        (st.seated, aurora_i18n::tr!("ui-bars-sitting")),
                    ] {
                        if on {
                            state_chip(ui, p, text);
                        }
                    }
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(p.field)
                        .corner_radius(ui.visuals().window_corner_radius)
                        .inner_margin(egui::Margin::symmetric(8, 1))
                        .show(ui, |ui| {
                            ui.set_width((ui.available_width() - 4.0).max(160.0));
                            // the whole field is clickable (widgets added later, like the
                            // ⓘ button, stay on top of this area)
                            let field = ui.max_rect().expand2(Vec2::new(8.0, 1.0));
                            let field = egui::Rect::from_x_y_ranges(
                                field.x_range(),
                                ui.max_rect().center().y - 10.0..=ui.max_rect().center().y + 10.0,
                            );
                            let bg = ui.interact(field, ui.scope_id().with("nav_location_area"), egui::Sense::click());
                            if panels.nav_edit.is_none() {
                                if bg.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                                }
                                if bg.clicked() {
                                    panels.nav_edit = Some(st.slurl.clone());
                                    panels.nav_edit_new = true;
                                }
                            }
                            let mut loc = String::new();
                            if !st.parcel.is_empty() {
                                loc.push_str(st.parcel);
                                loc.push_str(", ");
                            }
                            loc.push_str(&format!(
                                "{} ({:.0}, {:.0}, {:.0})",
                                st.region, st.position.x, st.position.y, st.position.z
                            ));
                            if st.location_hidden {
                                loc = format!("{}, {}", st.parcel, st.region);
                                panels.nav_edit = None;
                            }
                            if !st.maturity.is_empty() {
                                loc.push_str(&format!(" - {}", st.maturity));
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                if info_button(ui, p, icons) {
                                    panels.about_land = !panels.about_land;
                                }
                                // keep the maturity badge and the parcel icons visible when
                                // the field is narrow; the icons go first in a very narrow
                                // window (they would spill over the clock)
                                let icons_w = if st.parcel_icons.is_empty() {
                                    0.0
                                } else {
                                    super::parcel_icons::width(ui, &st.parcel_icons) + 8.0
                                };
                                let icons_w = if ui.available_width() - 24.0 - STAR_W - icons_w >= 80.0 {
                                    icons_w
                                } else {
                                    0.0
                                };
                                let w = (ui.available_width() - 24.0 - STAR_W - icons_w).max(40.0);
                                ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                                    if let Some(a) = location_field(ui, p, panels, &loc, &st.slurl, w) {
                                        action = a;
                                    }
                                });
                                super::widgets::maturity_badge(ui, p, st.maturity, 14.0);
                                // parcel abilities then the landmark star, anchored to
                                // the right edge of the field
                                if icons_w > 0.0 {
                                    ui.add_space((ui.available_width() - icons_w + 8.0 - STAR_W).max(0.0));
                                    ui.scope(|ui| {
                                        if super::parcel_icons::show(ui, p, icons, &st.parcel_icons, p.field) {
                                            panels.about_land = !panels.about_land;
                                        }
                                    });
                                } else {
                                    ui.add_space((ui.available_width() - STAR_W).max(0.0));
                                }
                                if let Some(star) = landmark_star(ui, p, icons, st.parcel_landmark.as_ref()) {
                                    action = BarAction::LandmarkStar(star);
                                }
                            });
                        });
                });
            });
        });
    if let Some((painter, slot)) = glass {
        super::widgets::finish_bar_glass(&painter, slot, bar.response.rect, p);
    }
    (action, audio_action)
}

/// Execute a toolbar command; returns an action for the app.
pub fn run_command(ctx: &egui::Context, cmd: &str, panels: &mut Panels) -> BarAction {
    // a window shown closes, a minimized one expands (LLFloaterReg::toggleInstance)
    let toggle = |id: &str, open: &mut bool| super::widgets::toggle_floater(ctx, id, open);
    match cmd {
        "conversations" => toggle("conversations", &mut panels.chat),
        "people" => {
            toggle("people", &mut panels.people);
            panels.people_tab = 0;
        }
        "friends" => {
            toggle("people", &mut panels.people);
            panels.people_tab = 1;
        }
        "inventory" => toggle("inventory", &mut panels.inventory),
        "texture_library" => toggle(panels.texture_library_presentation.window(), &mut panels.texture_library),
        "gestures" => toggle("gestures", &mut panels.gestures),
        "appearance" => toggle("appearance", &mut panels.appearance),
        "ao" => toggle("ao", &mut panels.ao),
        "minimap" => panels.minimap = !panels.minimap,
        "worldmap" => toggle("worldmap", &mut panels.world_map),
        "places" => toggle("places", &mut panels.places),
        "search" => toggle(super::search::WINDOW, &mut panels.search),
        "areasearch" => toggle(super::area_search::WINDOW, &mut panels.area_search),
        "soundexplorer" => toggle(super::sound_explorer::WINDOW, &mut panels.sound_explorer),
        "assetblacklist" => toggle(super::blacklist::WINDOW, &mut panels.asset_blacklist),
        "gridstatus" => toggle(super::grid_status::WINDOW, &mut panels.grid_status),
        "performance" => toggle("perf", &mut panels.perf),
        "snapshot" => toggle(super::snapshot::WINDOW, &mut panels.snapshot),
        "preferences" => toggle("options", &mut panels.settings),
        // Firestorm's Move and View buttons open the controls
        "move" => panels.movement_controls = !panels.movement_controls,
        "view" => panels.camera_controls = !panels.camera_controls,
        "fly" => return BarAction::ToggleFly,
        "movelock" | "move_lock" => return BarAction::ToggleMoveLock,
        "water_walk" => return BarAction::ToggleWaterWalk,
        "movement" => return BarAction::ToggleSwimming,
        "mouselook" => return BarAction::ToggleMouselook,
        "sit" => return BarAction::SitGround,
        "home" => return BarAction::TeleportHome,
        // Firestorm's commands on what Aurora already does (commands.xml)
        "aboutland" => panels.about_land = !panels.about_land,
        "regioninfo" => panels.region_estate = !panels.region_estate,
        "myenvironments" => panels.environment = !panels.environment,
        "radar" | "block_list" | "contact_sets" => {
            let tab = match cmd {
                "radar" => super::people::TAB_NEARBY,
                "block_list" => super::people::TAB_BLOCKED,
                _ => super::people::TAB_SETS,
            };
            // the same button again closes People on its tab
            if panels.people && panels.people_tab == tab {
                toggle("people", &mut panels.people);
            } else {
                super::widgets::open_floater(ctx, "people", &mut panels.people);
            }
            panels.people_tab = tab;
        }
        "build" => return BarAction::Build(0),
        "ik_edit" => return BarAction::IkEdit,
        "poser" => return BarAction::Poser,
        "profile" => return BarAction::MyProfile,
        "reporter" => return BarAction::ReportAbuse,
        "teleport_history" => return BarAction::TeleportHistory,
        "landmark_here" => return BarAction::CreateLandmark,
        "marketplacelistings" => return BarAction::MarketplaceListings,
        "stop_animations" => return BarAction::StopAnimations,
        "resync_animations" => return BarAction::ResyncAnimations,
        "immersive" => return BarAction::ToggleImmersive,
        _ => {}
    }
    BarAction::None
}

fn command_active(cmd: &str, panels: &Panels, flying: bool, seated: bool, mouselook: bool) -> bool {
    match cmd {
        "conversations" => panels.chat,
        "people" | "friends" => panels.people,
        "radar" => panels.people && panels.people_tab == super::people::TAB_NEARBY,
        "block_list" => panels.people && panels.people_tab == super::people::TAB_BLOCKED,
        "contact_sets" => panels.people && panels.people_tab == super::people::TAB_SETS,
        "aboutland" => panels.about_land,
        "regioninfo" => panels.region_estate,
        "myenvironments" => panels.environment,
        "inventory" => panels.inventory,
        "texture_library" => panels.texture_library,
        "gestures" => panels.gestures,
        "appearance" => panels.appearance,
        "minimap" => panels.minimap,
        "worldmap" => panels.world_map,
        "places" => panels.places,
        "search" => panels.search,
        "areasearch" => panels.area_search,
        "soundexplorer" => panels.sound_explorer,
        "assetblacklist" => panels.asset_blacklist,
        "gridstatus" => panels.grid_status,
        "performance" => panels.perf,
        "snapshot" => panels.snapshot,
        "preferences" => panels.settings,
        "move" => panels.movement_controls,
        "view" => panels.camera_controls,
        "fly" => flying,
        "sit" => seated,
        "mouselook" => mouselook,
        _ => false,
    }
}

/// Face of a toolbar button: a rounded pill, the accent while its window is
/// open or its mode is on, a neutral one under the pointer, nothing at rest.
/// The pills and the gap between them replace dividers.
fn tool_fill(p: &Palette, active: bool, hovered: bool) -> Option<Color32> {
    match (active, hovered) {
        (true, true) => Some(p.violet.gamma_multiply(0.55)),
        (true, false) => Some(p.violet.gamma_multiply(0.40)),
        (false, true) => Some(p.raised),
        (false, false) => None,
    }
}

/// The id of the toolbar button of `command`, for the demo scenarios that
/// click it.
pub(crate) fn tool_button_id(command: &str) -> egui::Id {
    egui::Id::unique(("toolbar_button", command))
}

/// The size of a toolbar button and of its icon: Firestorm's full-width bar
/// (22 high, the icon follows the width) or the dock of the islands (fixed
/// buttons, 18 px icons, `bars/dock.rs`); its icon, label or both
/// (`toolbar::ButtonMode`), the label after the icon at the left in a side
/// bar, else the pair centred; where its pop-ups open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ButtonLook {
    pub size: Vec2,
    pub icon: f32,
    pub dock: bool,
    pub mode: super::toolbar::ButtonMode,
    pub left: bool,
    pub opens: egui::RectAlign,
    /// The "Toolbar buttons" window is open: every button shows a faint
    /// face, the mockup's buttons that can be dragged.
    pub editing: bool,
}

impl ButtonLook {
    fn bar(width: f32) -> Self {
        ButtonLook {
            size: Vec2::new(width, 22.0),
            icon: (width - 6.0).clamp(10.0, 16.0),
            dock: false,
            mode: super::toolbar::ButtonMode::IconsOnly,
            left: false,
            opens: egui::RectAlign::TOP,
            editing: false,
        }
    }
}

/// The width of a button's label in a toolbar.
pub(crate) fn label_width(ctx: &egui::Context, text: &str) -> f32 {
    ctx.fonts_mut(|f| {
        f.layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(sizing::LABEL_FONT),
            Color32::PLACEHOLDER,
        )
        .size()
        .x
    })
}

/// A button's own width in a toolbar (`sizing::natural`): `square` for an
/// icon (`mic` for Speak), its label's room otherwise; Speak keeps room for
/// its arrow.
pub(crate) fn natural_width(
    ctx: &egui::Context,
    b: &super::skin::ToolButton,
    mode: super::toolbar::ButtonMode,
    square: f32,
    icon: f32,
    mic: f32,
) -> f32 {
    let speak = b.command == "speak";
    if mode == super::toolbar::ButtonMode::IconsOnly {
        return if speak { mic } else { square };
    }
    let extra = if speak { DOCK_MIC_ARROW } else { 0.0 };
    sizing::natural(mode, square, icon, label_width(ctx, super::toolbar::label(b)), extra)
}

/// Where a button's icon and label go in `rect`: the icon's rectangle (None
/// with the label alone) and the label's left end and width (none with the
/// icon alone).
fn content(ui: &egui::Ui, rect: egui::Rect, look: ButtonLook, text: &str) -> (Option<egui::Rect>, Option<(f32, f32)>) {
    use super::toolbar::ButtonMode;
    let icon = look.icon;
    if look.mode == ButtonMode::IconsOnly {
        return (Some(egui::Rect::from_center_size(rect.center(), Vec2::splat(icon))), None);
    }
    let with_icon = look.mode == ButtonMode::IconsWithText;
    let lead = if with_icon { icon + sizing::ICON_GAP } else { 0.0 };
    let room = (rect.width() - sizing::LABEL_PAD * 2.0 - lead).max(0.0);
    let text_w = label_width(ui.ctx(), text).min(room);
    let start = if look.left {
        rect.left() + sizing::LABEL_PAD
    } else {
        (rect.center().x - (lead + text_w) / 2.0).max(rect.left() + sizing::LABEL_PAD)
    };
    let icon_rect = with_icon.then(|| egui::Rect::from_center_size(egui::pos2(start + icon / 2.0, rect.center().y), Vec2::splat(icon)));
    (icon_rect, Some((start + lead, room)))
}

#[allow(clippy::too_many_arguments)]
fn tool_button(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    command: &str,
    icon: &str,
    label: &str,
    tip: &str,
    active: bool,
    look: ButtonLook,
    unread: usize,
    flash: f32,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(look.size, egui::Sense::hover());
    let resp = ui.interact(rect, tool_button_id(command), egui::Sense::click_and_drag());
    let icon_size = look.icon;
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter();
    // a saved Firestorm command Aurora does not run yet: very dimmed, with
    // neither face nor pill under the pointer, as in the window
    let ready = super::toolbar::ready(command);
    let hovered = ready && resp.hovered();
    if let Some(fill) = tool_fill(p, ready && active, hovered).or_else(|| look.editing.then(|| p.raised.gamma_multiply(0.55))) {
        painter.rect_filled(rect, radius, fill);
    }
    // the Conversations button's flash (`ButtonFlash`), the minimized bar's
    // own mark
    if flash > 0.0 {
        super::widgets::attention_mark(painter, p, rect, radius, flash);
    }
    let tint = if !ready {
        p.muted_dim.gamma_multiply(0.55)
    } else if active || hovered {
        p.ink
    } else {
        p.muted
    };
    let (icon_rect, text) = content(ui, rect, look, label);
    if let Some(ir) = icon_rect
        && let Some(t) = icons.get(icon)
    {
        painter.image(
            t.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    if icon == "AO"
        && let Some(ir) = icon_rect
    {
        // centered on the letters' ink like the icons beside it: the line box
        // of a font sits lower than its capitals
        // an icon, not a label: icon-sized whatever the text size
        let size = if look.dock { 13.0 } else { 12.0 } / super::fonts::scale(ui.ctx());
        // bold like the worn names: the letters weigh as much as the filled
        // icons of the other buttons
        let format = super::widgets::bold_format(ui, egui::FontId::proportional(size), tint);
        let galley = painter.layout_job(egui::text::LayoutJob::single_section("AO".to_owned(), format));
        if let Some(offset) = super::widgets::faux_bold_offset(ui) {
            super::widgets::paint_ink_centered(painter, galley.clone(), ir.center() + offset, tint);
        }
        super::widgets::paint_ink_centered(painter, galley, ir.center(), tint);
    }
    if let Some((left, width)) = text {
        super::widgets::paint_elided(
            ui,
            label,
            egui::pos2(left, rect.center().y),
            width,
            egui::FontId::proportional(sizing::LABEL_FONT),
            tint,
        );
    }
    if unread > 0 && look.dock {
        // the one counter of the islands, as on the bell: 3 px in from the
        // button's right edge, 1 px above its top (the mockup's `.upill`)
        let slot = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() - 1.0),
            egui::pos2(rect.right() - 5.0, rect.bottom()),
        );
        islands::count(ui, p, slot, unread, 1.0, false);
    } else if unread > 0 {
        paint_unread_pill(ui, p, rect, icon_size, unread);
    }
    resp.on_hover_text(tip)
}

/// The unread count of the Conversations button: the one counter of the
/// interface (`widgets::UnreadCount`), on the top right corner of the icon
/// like the bell's count, inside the button so that nothing moves. A button
/// too narrow for it keeps a dot of the counter's red.
fn paint_unread_pill(ui: &egui::Ui, p: &Palette, rect: egui::Rect, icon: f32, unread: usize) {
    let painter = ui.painter();
    let count = super::widgets::UnreadCount::new(unread, super::widgets::CountSize::Bar);
    let top = rect.top() - 1.0;
    let placed = count.place(ui.ctx(), p, egui::pos2(rect.center().x + icon * 0.2, top), egui::Align2::LEFT_TOP);
    if rect.width() < placed.rect.width() + 4.0 {
        painter.circle_filled(
            egui::pos2(rect.right() - (rect.width() * 0.18).clamp(4.0, 8.0), rect.center().y),
            (rect.width() * 0.07).clamp(2.0, 3.0),
            p.danger_dim,
        );
    } else if placed.rect.right() > rect.right() - 1.0 {
        count.paint(painter, p, egui::pos2(rect.right() - 1.0, top), egui::Align2::RIGHT_TOP);
    } else {
        placed.paint(painter);
    }
}

/// Microphone button state (Firestorm "Speak" button with its arrow).
#[derive(Default)]
pub struct MicButton<'a> {
    pub ear: crate::settings::VoiceEar,
    pub noise_suppression: u8,
    pub processing_changed: bool,
    pub open_processing: bool,
    /// Toggle mode: microphone switched on.
    pub on: bool,
    /// Why speaking is forbidden here (region or parcel without voice): the
    /// microphone is greyed and inert, its options stay open (the Speak
    /// command's is_enabled_function, Agent.IsActionAllowed).
    pub denied: Option<String>,
    /// Hold-to-talk mode (else click toggles).
    pub hold: bool,
    /// Hold mode: button pressed this frame (output).
    pub held: bool,
    /// Transmitting (button / push-to-talk key).
    pub talking: bool,
    /// Own voice level 0..1.
    pub level: f32,
    /// Voice connected in this region.
    pub connected: bool,
    /// Output: open the "Sound and voice" preferences.
    pub open_prefs: bool,
    /// Output: the mode switch changed.
    pub mode_changed: bool,
    /// Key (or mouse button) of push-to-talk, shown with the hold mode.
    pub key: Option<crate::keybinds::Binding>,
    /// Microphone and audio output in use ("" = the system's default) and
    /// the devices to choose from.
    pub input_device: String,
    pub output_device: String,
    pub input_devices: &'a [String],
    pub output_devices: &'a [String],
    /// Output: a device was chosen.
    pub devices_changed: bool,
    /// Output: the panel opens, its lists of devices are to be read again.
    pub list_devices: bool,
    /// With the islands, the voice connection light moves from the top bar
    /// to a dot on the button (green, orange while connecting, red, grey).
    pub voice_dot: Option<(crate::voice::VoiceLight, String)>,
}

/// Width of the arrow's zone, right of the microphone.
const MIC_ARROW: f32 = 14.0;

/// Width of the dock's Speak button: the microphone and its arrow side by
/// side, centred as a pair (the mockup's `.mic`, 44 px).
pub(crate) const DOCK_MIC_W: f32 = 44.0;
/// The arrow's zone in the dock: a 10 px caret 1 px after the microphone.
const DOCK_MIC_ARROW: f32 = 12.0;

#[allow(clippy::too_many_arguments)]
fn mic_button(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    look: ButtonLook,
    label: &str,
    mic: &mut MicButton,
    toolbar_menu: bool,
    open_requested: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(look.size, egui::Sense::hover());
    let width = look.size.x;
    let icon_size = look.icon;
    let labelled = look.mode != super::toolbar::ButtonMode::IconsOnly;
    // the microphone stands in the middle of the button, in line with the
    // icons of its neighbors, and the arrow right after it; a button too
    // narrow for the arrow keeps the microphone, and a right click on it
    // opens the panel. In the dock the pair is centred instead; with a
    // label, the arrow takes the button's right end.
    let arrow_w = if look.dock { DOCK_MIC_ARROW } else { MIC_ARROW };
    let with_arrow = look.dock || labelled || width >= icon_size + 2.0 * (MIC_ARROW + 1.0);
    let (icon_rect, text, arrow_left) = if labelled {
        let body = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right() - arrow_w, rect.bottom()));
        let (icon_rect, text) = content(ui, body, look, label);
        (icon_rect, text, rect.right() - arrow_w)
    } else {
        let icon_x = if look.dock {
            rect.center().x - arrow_w / 2.0
        } else {
            rect.center().x
        };
        let icon_rect = egui::Rect::from_center_size(egui::pos2(icon_x, rect.center().y), Vec2::splat(icon_size));
        (Some(icon_rect), None, icon_rect.right())
    };
    let resp = ui.interact(rect, ui.scope_id().with("mic_main"), egui::Sense::click_and_drag());
    let arrow = with_arrow.then(|| {
        let zone = egui::Rect::from_min_max(egui::pos2(arrow_left, rect.top()), egui::pos2(arrow_left + arrow_w, rect.bottom()));
        ui.interact(zone, ui.scope_id().with("mic_arrow"), egui::Sense::click())
    });
    let denied = mic.denied.is_some();
    if mic.hold {
        mic.held = !denied && resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
    } else if resp.clicked() && !denied {
        mic.on = !mic.on;
    }
    let hovered = resp.hovered() && !denied;
    // The panel keeps its open state itself. egui's memory holds a single
    // open pop-up, and the list of a device combo is one: kept there, the
    // panel would close as soon as such a list unfolds.
    let open_id = ui.scope_id().with("mic_panel_open");
    let open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let on_arrow = arrow.as_ref().is_some_and(|r| r.hovered());
    let radius = ui.visuals().window_corner_radius;
    let live = mic.talking;
    if let Some(fill) = tool_fill(p, live, hovered || on_arrow || open) {
        ui.painter().rect_filled(rect, radius, fill);
    }
    // own level, along the bottom of the button: it follows the voice
    // closely, only smoothing the jitter between two samples
    let level = if live { mic.level.clamp(0.0, 1.0) } else { 0.0 };
    let level = ui.ctx().animate_value_with_time(ui.scope_id().with("mic_level"), level, 0.08);
    // the dock's bar is a share of the whole button (the mockup's `.level`)
    let level_width = if look.dock { rect.width() } else { rect.width() - 6.0 } * level;
    // 2 px above the dock button's bottom, 1 px above the bar's
    let level_bottom = if look.dock { 2.0 } else { 1.0 };
    if level_width > 0.5 {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.left() + 3.0, rect.bottom() - level_bottom - 2.0),
                Vec2::new(level_width, 2.0),
            ),
            super::widgets::TRACK_RADIUS,
            p.teal,
        );
    }
    let icon = if live || (!mic.hold && mic.on) {
        "microphone-filled"
    } else {
        "microphone-off"
    };
    let tint = if denied {
        p.muted_dim
    } else if live || hovered {
        p.ink
    } else if mic.connected {
        p.muted
    } else {
        p.muted_dim
    };
    if let Some(icon_rect) = icon_rect {
        paint_icon(ui, icons, icon, icon_rect, icon_size, tint);
    }
    if let Some((left, room)) = text {
        super::widgets::paint_elided(
            ui,
            label,
            egui::pos2(left, rect.center().y),
            room,
            egui::FontId::proportional(sizing::LABEL_FONT),
            tint,
        );
    }
    if let Some((light, _)) = &mic.voice_dot {
        // on the microphone, or past the label's end without it
        let anchor = icon_rect.unwrap_or_else(|| {
            let (left, room) = text.unwrap_or((rect.left(), 0.0));
            let end = left + label_width(ui.ctx(), label).min(room);
            egui::Rect::from_center_size(egui::pos2(end, rect.center().y), Vec2::splat(icon_size))
        });
        voice_dot(ui, p, anchor, *light);
    }
    if let Some(arrow) = &arrow {
        // the arrow points to where its panel opens
        let zone = arrow.rect.shrink2(Vec2::new(1.0, 3.0));
        if on_arrow || open {
            ui.painter().rect_filled(zone, radius, p.raised.lerp_to_gamma(p.ink, 0.16));
        }
        let tint = if on_arrow || open { p.ink } else { p.muted };
        paint_icon(ui, icons, "caret-up-filled", zone, 10.0, tint);
    }

    let state = if !mic.connected {
        aurora_i18n::tr!("ui-bars-voice-not-connected-2")
    } else if live {
        aurora_i18n::tr!("ui-bars-you-are-speaking")
    } else {
        aurora_i18n::tr!("ui-bars-microphone-muted")
    };
    let mut tip = if let Some(why) = &mic.denied {
        why.clone()
    } else if mic.hold {
        aurora_i18n::tr!("ui-bars-hold-to-talk", state = state.to_string())
    } else {
        aurora_i18n::tr!("extra-click-to-enable-mute-microphone", state = state.to_string())
    };
    if let Some((light, why)) = mic.voice_dot.as_ref().filter(|_| !denied) {
        // what the top bar's voice light said
        let (_, name) = voice_state(p, *light);
        tip.push_str(&format!("\n{name}"));
        if !why.is_empty() && why != name {
            tip.push_str(&format!("\n{why}"));
        }
    }
    let resp = resp.on_hover_text(tip);
    let arrow = arrow.map(|r| {
        if open {
            r
        } else {
            r.on_hover_text(aurora_i18n::tr!("ui-bars-microphone-options"))
        }
    });

    // demo captures: AURORA_DEMO_MICMENU=1 opens the panel
    static DEMO_OPEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let demo = *DEMO_OPEN.get_or_init(|| std::env::var_os("AURORA_DEMO_MICMENU").is_some()) && ui.ctx().cumulative_frame_nr() == 300;
    let toggle = match &arrow {
        Some(arrow) => arrow.clicked(),
        None => !toolbar_menu && resp.secondary_clicked(),
    };
    let toggled = toggle || demo || (open_requested && !open);
    mic.list_devices = toggled && !open;
    let anchor = arrow.as_ref().unwrap_or(&resp);
    // a click in the list of a device combo is a click outside the panel:
    // the panel closes itself, and not while such a list is (or just was) open
    let list_id = ui.scope_id().with("mic_device_list");
    let list_was_open = ui.ctx().data(|d| d.get_temp::<bool>(list_id)).unwrap_or(false);
    let mut list_open = false;
    let mut open_now = open != toggled;
    // Escape and the panel's own `ui.close()` clear the flag
    let shown = egui::Popup::from_response(anchor)
        .open_bool(&mut open_now)
        .align(if look.opens == egui::RectAlign::TOP {
            egui::RectAlign::TOP_END
        } else {
            look.opens
        })
        .gap(7.0)
        .frame(egui::Frame::NONE)
        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        .show(|ui| mic_panel(ui, p, icons, mic, &mut list_open));
    let in_list = list_open || list_was_open;
    if let Some(shown) = shown {
        let away = open && shown.response.clicked_elsewhere() && anchor.clicked_elsewhere();
        if away && !in_list {
            open_now = false;
        } else if !open_now && in_list && !mic.open_prefs {
            // Escape with a list unfolded closes that list, not the panel
            open_now = true;
        }
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(list_id, list_open);
        d.insert_temp(open_id, open_now);
    });
    arrow.map_or_else(|| resp.clone(), |arrow| resp.union(arrow))
}

/// Panel of the microphone's arrow: the state of the voice, the two ways to
/// talk, the microphone and the audio output in use (one under the other:
/// both ends of the same conversation) and the way to the sound
/// preferences. `list_open`: the list of a device combo is unfolded.
fn mic_panel(ui: &mut egui::Ui, p: &Palette, icons: &Icons, mic: &mut MicButton, list_open: &mut bool) {
    super::widgets::overlay(ui, p, egui::Margin::same(10), |ui| {
        ui.set_width(340.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            super::widgets::glyph(ui, "microphone-filled", p.violet_light, 17.0);
            ui.label(RichText::new(aurora_i18n::tr!("common-voice")).size(13.0).strong().color(p.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let (state, color) = if mic.connected {
                    (aurora_i18n::tr!("common-voice-connected"), p.success)
                } else {
                    (aurora_i18n::tr!("ui-bars-voice-not-connected"), p.muted_dim)
                };
                ui.label(RichText::new(state).size(11.0).color(p.muted));
                super::widgets::status_dot(ui, color);
            });
        });
        ui.add_space(4.0);
        let modes = [
            (
                false,
                "hand-click",
                aurora_i18n::tr!("ui-bars-click-to-talk"),
                aurora_i18n::tr!("ui-bars-one-click-enables-the-microphone-another-disables-it"),
            ),
            (
                true,
                "hand-finger",
                aurora_i18n::tr!("common-hold-to-talk"),
                aurora_i18n::tr!("ui-bars-talk-while-the-button-or-key-is-held-down"),
            ),
        ];
        for (hold, icon, title, hint) in modes {
            let key = if hold { mic.key.as_ref() } else { None };
            if mic_mode(ui, p, icons, mic.hold == hold, icon, title, hint, key) && mic.hold != hold {
                mic.hold = hold;
                mic.mode_changed = true;
                mic.on = false;
            }
        }
        ui.add_space(4.0);
        let devices = [
            (
                "microphone-filled",
                aurora_i18n::tr!("ui-options-microphone"),
                "mic_panel_input",
                &mut mic.input_device,
                mic.input_devices,
            ),
            (
                "headphones-filled",
                aurora_i18n::tr!("ui-options-audio-output"),
                "mic_panel_output",
                &mut mic.output_device,
                mic.output_devices,
            ),
        ];
        for (icon, name, id, device, list) in devices {
            ui.horizontal(|ui| {
                // the icons stand in the column of the modes' icons
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(8.0);
                let (slot, hover) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                paint_icon(ui, icons, icon, slot, 16.0, p.muted);
                hover.on_hover_text(name);
                ui.add_space(9.0);
                let width = ui.available_width();
                let (changed, open) = super::options::device_combo(ui, id, device, list, width);
                mic.devices_changed |= changed;
                *list_open |= open;
            });
        }
        ui.add_space(4.0);
        let size = Vec2::new(ui.available_width(), 28.0);
        let label = aurora_i18n::tr!("ui-bars-sound-and-voice-settings");
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("voice-listen-from")).size(12.0).color(p.muted));
            mic.processing_changed |= super::options::ear_choice(ui, &mut mic.ear);
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("voice-noise-suppression")).size(12.0).color(p.muted));
            mic.processing_changed |= super::options::noise_choice(ui, "mic_noise", &mut mic.noise_suppression, 145.0);
        });
        if super::widgets::action_button(
            ui,
            p,
            "adjustments-filled",
            aurora_i18n::tr!("voice-processing-open"),
            super::widgets::ActionKind::Flat,
            size,
        )
        .clicked()
        {
            mic.open_prefs = true;
            mic.open_processing = true;
            ui.close();
        }
        if super::widgets::action_button(ui, p, "settings-filled", label, super::widgets::ActionKind::Flat, size).clicked() {
            mic.open_prefs = true;
            ui.close();
        }
    });
}

/// One of the two ways to talk: its icon, its name (followed by the key to
/// hold, for the mode that has one) and what it does. The current one is
/// lit like an active button of the bar, and ticked.
#[allow(clippy::too_many_arguments)]
fn mic_mode(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    selected: bool,
    icon: &str,
    title: &str,
    hint: &str,
    key: Option<&crate::keybinds::Binding>,
) -> bool {
    const PAD: f32 = 8.0;
    const ICON: f32 = 18.0;
    const TICK: f32 = 16.0;
    let width = ui.available_width();
    let text_left = PAD + ICON + 9.0;
    let text_width = width - text_left - PAD - TICK - 6.0;
    let title = ui
        .painter()
        .layout_no_wrap(title.to_owned(), egui::FontId::proportional(12.5), p.ink);
    let hint = ui
        .painter()
        .layout(hint.to_owned(), egui::FontId::proportional(11.0), p.muted, text_width);
    let height = PAD + title.size().y + 2.0 + hint.size().y + PAD;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if let Some(fill) = tool_fill(p, selected, resp.hovered()) {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    }
    let lit = selected || resp.hovered();
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + PAD + ICON / 2.0, rect.center().y), Vec2::splat(ICON));
    paint_icon(ui, icons, icon, icon_rect, ICON, if lit { p.ink } else { p.muted });
    let title_center = rect.top() + PAD + title.size().y / 2.0;
    let title_right = rect.left() + text_left + title.size().x;
    let hint_top = rect.top() + PAD + title.size().y + 2.0;
    super::widgets::paint_ink_left(ui.painter(), title, egui::pos2(rect.left() + text_left, title_center), p.ink);
    ui.painter().galley(egui::pos2(rect.left() + text_left, hint_top), hint, p.muted);
    if let Some(key) = key {
        // the caps stand on the panel, lit or not: their names take its color
        let caps = super::widgets::keycap::caps(ui, key, p.panel);
        super::widgets::keycap::paint_row(ui, p, &caps, egui::pos2(title_right + 8.0, title_center), p.panel);
    }
    if selected {
        let tick = egui::Rect::from_center_size(egui::pos2(rect.right() - PAD - TICK / 2.0, rect.center().y), Vec2::splat(TICK));
        paint_icon(ui, icons, "circle-check-filled", tick, TICK, p.violet_light);
    }
    resp.clicked()
}

/// What lights the toolbar's buttons, besides the open windows.
#[derive(Clone, Copy)]
pub(crate) struct Row<'a> {
    pub unread: usize,
    /// The conversation the chat island's button opens while there are
    /// unread lines (`Social::latest_unread`).
    pub latest_unread: Option<uuid::Uuid>,
    /// The violet of the Conversations button's flash (0: none), from
    /// `ButtonFlash::button`.
    pub flash: f32,
    pub flying: bool,
    pub seated: bool,
    pub mouselook: bool,
    pub ao_active: bool,
    pub locked: bool,
    /// The favourites, for the Favorites button (F3, or added by hand).
    pub favorites: &'a favorites::View,
    /// The toolbar drawn, and its settings for the look in use.
    pub bar: super::toolbar::Bar,
    pub style: super::toolbar::Style,
    /// The "Toolbar buttons" window is open (and the toolbars unlocked).
    pub editing: bool,
}

/// The toolbar's buttons one after the other in `ui` (Firestorm's bar or
/// the dock, sized by `look`), with their drags and right-click menus, the
/// Speak button with its panel and the Conversations unread count. Returns
/// their rectangles, in order, for the drop marker.
#[allow(clippy::too_many_arguments)]
pub(crate) fn toolbar_row(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    buttons: &[crate::ui::skin::ToolButton],
    look: impl Fn(&crate::ui::skin::ToolButton) -> ButtonLook,
    toolbar: &mut super::toolbar::ToolbarUi,
    panels: &mut Panels,
    mic: &mut MicButton,
    row: &Row,
    action: &mut BarAction,
) -> Vec<egui::Rect> {
    let mut rects = Vec::with_capacity(buttons.len());
    for b in buttons {
        if b.command == "speak" {
            let open_voice = std::mem::take(&mut toolbar.voice_options);
            let response = mic_button(ui, p, icons, look(b), super::toolbar::label(b), mic, true, open_voice);
            rects.push(response.rect);
            toolbar.button(&response, row.bar, row.style, b, p, row.locked);
            if !row.locked && response.dragged() {
                mic.held = false;
            }
            continue;
        }
        let active = if b.command == "ao" {
            row.ao_active || toolbar.ao_quick.open
        } else if matches!(b.command.as_str(), "movelock" | "move_lock") {
            toolbar.movelock_active
        } else if b.command == "movement" {
            toolbar.movement_active
        } else if b.command == "water_walk" {
            toolbar.water_walk_active
        } else {
            command_active(&b.command, panels, row.flying, row.seated, row.mouselook)
        };
        // Firestorm flashes the button while the conversations are closed or
        // minimized (LLIMMgr, flashCommand)
        let badge = b.command == "conversations"
            && row.unread > 0
            && (!panels.chat || super::widgets::floater_minimized(ui.ctx(), "conversations"));
        let count = if badge { row.unread } else { 0 };
        let tip = if badge {
            aurora_i18n::tr!(
                "ui-bars-unread",
                value0 = super::toolbar::label(b).to_string(),
                unread = format!("{}", row.unread)
            )
        } else if !super::toolbar::ready(&b.command) {
            format!("{}\n{}", super::toolbar::label(b), menu::not_yet())
        } else {
            super::toolbar::label(b).to_owned()
        };
        let tip = if matches!(b.command.as_str(), "movelock" | "move_lock") {
            toolbar.movelock_status.clone()
        } else {
            tip
        };
        // the Favorites button: its grid of favourites opens upwards
        let grid_id = egui::Id::unique("bars_favorites_grid_open");
        let active = active || (b.command == "favorites" && ui.ctx().data(|d| d.get_temp::<bool>(grid_id)).unwrap_or(false));
        let response = tool_button(
            ui,
            p,
            icons,
            &b.command,
            &b.icon,
            super::toolbar::label(b),
            &tip,
            active,
            look(b),
            count,
            // the Grid status button flashes on a new entry of the feed
            // (LLFloaterGridStatus::getGridStatusRSSCoro, flashCommand)
            match b.command.as_str() {
                "conversations" => row.flash,
                "gridstatus" => toolbar.grid_status_flash,
                _ => 0.0,
            },
        );
        rects.push(response.rect);
        toolbar.button(&response, row.bar, row.style, b, p, row.locked);
        if !super::toolbar::ready(&b.command) {
            // a saved Firestorm command Aurora does not have yet
            continue;
        }
        if b.command == "favorites" {
            islands::demo_open(ui.ctx(), &response, demo::Open::Favorites);
            let mut fav = None;
            let open = menu::dropdown_placed(
                &response,
                p,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                Some((look(b).opens, 12.0)),
                |ui| favorites::grid_body(ui, p, icons, row.favorites, &mut fav),
            );
            ui.ctx().data_mut(|d| d.insert_temp(grid_id, open));
            if let Some(a) = fav {
                *action = BarAction::Favorite(a);
            }
            continue;
        }
        if b.command == "ao" {
            toolbar.ao_quick.anchor(&response, look(b).opens);
            continue;
        }
        if response.clicked() {
            let a = run_command(ui.ctx(), &b.command, panels);
            if !matches!(a, BarAction::None) {
                *action = a;
            }
        }
    }
    rects
}

/// Bottom bar: "Nearby chat" input on the left, feature buttons spread over
/// the rest of the width; with the islands, the chat island and the dock
/// (`dock.rs`, `chat_width` is then the island's). Returns (chat, action,
/// top y of what the bar keeps, chat width to save).
#[allow(clippy::too_many_arguments)]
pub fn bottom_bar(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    layout: &Layout,
    toolbar: &mut super::toolbar::ToolbarUi,
    toolbar_settings: &mut super::toolbar::ToolbarSettings,
    panels: &mut Panels,
    chat: &mut super::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    unread: usize,
    latest_unread: Option<uuid::Uuid>,
    flying: bool,
    seated: bool,
    mouselook: bool,
    chat_width: &mut f32,
    mic: &mut MicButton,
    ao_active: bool,
    islands: bool,
    emoji: Option<&mut super::emoji::Emoji>,
    favorites: &favorites::View,
) -> (Option<super::chat::OutgoingChat>, BarAction, f32, bool) {
    let bar = super::toolbar::Bar::Bottom;
    let style = toolbar_settings.style(bar).resolve(bar, islands);
    let editing = toolbar.open && !toolbar_settings.locked;
    let (flash, flashing) = chat.button_flash.button(ui.input(|i| i.time));
    if flashing {
        ui.ctx().request_repaint();
    }
    if islands {
        let row = Row {
            unread,
            latest_unread,
            flash,
            flying,
            seated,
            mouselook,
            ao_active,
            locked: toolbar_settings.locked,
            favorites,
            bar,
            style,
            editing,
        };
        return dock::bottom(
            ui,
            p,
            icons,
            layout,
            toolbar,
            toolbar_settings,
            panels,
            chat,
            gestures,
            emoji,
            &row,
            chat_width,
            mic,
        );
    }
    let mut out = None;
    let mut action = BarAction::None;
    let mut width_committed = false;
    let mut glass = None;
    // autohidden (#1009), the toolbar takes the whole bar
    let chat_shown = chat.bar_shown();
    chat.bar_hovered = false;
    let editor_height = if chat_shown {
        super::chat::input::height(ui.ctx(), super::chat::bar_input_id(), ui.spacing().interact_size.y)
    } else {
        0.0
    };
    let bar_height = layout.bottom_bar_height.max(editor_height + 6.0);
    let resp = egui::Panel::bottom("bottom_bar")
        .frame(bar_frame(p).inner_margin(egui::Margin::symmetric(8, 3)))
        .exact_size(bar_height)
        .show(ui, |ui| {
            glass = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
            ui.set_min_width(ui.available_width());
            ui.horizontal_centered(|ui| {
                let total = ui.available_width();
                let buttons = toolbar_settings.buttons(layout);
                let n = buttons.len().max(1) as f32;
                // the toolbar keeps at least a narrow button per command
                const MIN_BTN: f32 = 22.0;
                if chat_shown {
                    let start = ui.cursor().min;
                    let label_w = 70.0;
                    let max_chat = (total - label_w - 16.0 - n * (MIN_BTN + 2.0)).max(120.0);
                    let wanted = if *chat_width > 0.0 {
                        *chat_width
                    } else {
                        total * layout.chat_bar_fraction
                    };
                    let chat_w = wanted.clamp(120.0, max_chat);
                    out = super::chat::bar(ui, p, chat, gestures, chat_w);

                    // draggable separator (dotted, like Firestorm's) between chat and toolbar
                    let (handle, hresp) = ui.allocate_exact_size(Vec2::new(10.0, 22.0), egui::Sense::drag());
                    let hot = hresp.hovered() || hresp.dragged();
                    if hot {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    let col = if hot { p.violet_light } else { p.muted_dim };
                    let x = handle.center().x;
                    let mut y = handle.top() + 3.0;
                    while y < handle.bottom() - 2.0 {
                        ui.painter().circle_filled(egui::pos2(x, y), 0.9, col);
                        y += 3.0;
                    }
                    if hresp.dragged() {
                        *chat_width = (chat_w + hresp.drag_delta().x).clamp(120.0, max_chat);
                    }
                    if hresp.drag_stopped() {
                        width_committed = true;
                    }
                    if hresp.double_clicked() {
                        // back to the skin's default proportion
                        *chat_width = 0.0;
                        width_committed = true;
                    }
                    chat.bar_hovered = ui.rect_contains_pointer(egui::Rect::from_min_max(start, handle.max));
                }
                // the conversations' thumbnails beside the chat bar (#1134);
                // the toolbar keeps a narrow button per command
                let size = super::chat::thumbnails::BAR_SIZE;
                let room = ui.available_width() - n * (MIN_BTN + 4.0) - 8.0;
                let thumbs = chat.thumbnails.bar_width(size, 0.0, room);
                if thumbs > 0.0 {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(thumbs, size), egui::Sense::hover());
                    chat.thumbnails.reserve(rect, size, false);
                }

                // by default the buttons stretch over the whole remaining
                // width (hover and click area included); the gap and the
                // pill drawn under the pointer show where each one starts
                // and ends. Else Firestorm's layout styles and alignments.
                let remaining = ui.available_width();
                let gap = 4.0;
                let labelled = style.mode != super::toolbar::ButtonMode::IconsOnly;
                // Firestorm's icon buttons when they do not fill the bar
                let square = if style.sizing == super::toolbar::Sizing::Fill {
                    MIN_BTN
                } else {
                    32.0
                };
                let natural: Vec<f32> = buttons
                    .iter()
                    .map(|b| natural_width(ui.ctx(), b, style.mode, square, 16.0, square))
                    .collect();
                let widths = sizing::fit(&natural, style.sizing, labelled, remaining - 1.0, gap, MIN_BTN);
                let widths: Vec<f32> = widths.into_iter().map(|w| w.max(MIN_BTN)).collect();
                let used = widths.iter().sum::<f32>() + gap * (n - 1.0);
                let lead = (sizing::place(style.align, 0.0, remaining - 1.0, used)).max(0.0);
                let zone = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(remaining.max(4.0), 22.0));
                let target = ui.interact(zone, egui::Id::unique("toolbar_background"), egui::Sense::click());
                toolbar.context(&target, p, bar, style, None, toolbar_settings.locked);
                let mut rects = Vec::new();
                egui::ScrollArea::horizontal()
                    .id_salt("toolbar_scroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .max_height(22.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            if lead > 0.0 {
                                ui.add_space(lead - gap);
                            }
                            let row = Row {
                                unread,
                                latest_unread: None,
                                flash,
                                flying,
                                seated,
                                mouselook,
                                ao_active,
                                locked: toolbar_settings.locked,
                                favorites,
                                bar,
                                style,
                                editing,
                            };
                            let look = |b: &crate::ui::skin::ToolButton| {
                                let at = buttons.iter().position(|x| x.command == b.command).unwrap_or(0);
                                let w = widths.get(at).copied().unwrap_or(MIN_BTN);
                                ButtonLook {
                                    icon: if labelled { 16.0 } else { (w - 6.0).clamp(10.0, 16.0) },
                                    mode: style.mode,
                                    editing,
                                    ..ButtonLook::bar(w)
                                }
                            };
                            rects = toolbar_row(ui, p, icons, buttons, look, toolbar, panels, mic, &row, &mut action);
                        });
                    });
                toolbar.drop_bar(&target, bar, &rects, p, toolbar_settings.locked);
            });
        });
    if let Some((painter, slot)) = glass {
        super::widgets::finish_bar_glass(&painter, slot, resp.response.rect, p);
    }
    (out, action, resp.response.rect.top(), width_committed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_chat_bar_bounds_paragraphs_and_releases_space_when_cleared() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let settings = crate::settings::Settings::default();
        let mut panels = Panels::from_settings(&settings);
        let mut chat = super::super::chat::ChatUi::default();
        let mut toolbar = super::super::toolbar::ToolbarUi::default();
        let mut toolbar_settings = super::super::toolbar::ToolbarSettings::default();
        let mut width = 0.0;
        let field = super::super::chat::bar_input_id();
        let mut frame = |text: &str| {
            chat.input = text.into();
            let mut reserved = 0.0;
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                        ..Default::default()
                    },
                    |ui| {
                        let (_, _, height, _) = bottom_bar(
                            ui,
                            &crate::theme::Theme::default().palette(),
                            &Icons::default(),
                            &Layout::default(),
                            &mut toolbar,
                            &mut toolbar_settings,
                            &mut panels,
                            &mut chat,
                            &crate::gestures::Manager::default(),
                            0,
                            None,
                            false,
                            false,
                            false,
                            &mut width,
                            &mut MicButton::default(),
                            false,
                            false,
                            None,
                            &favorites::View::default(),
                        );
                        reserved = height;
                    },
                );
                output.textures_delta.clear();
            }
            (ctx.read_response(field).unwrap().interact_rect, reserved)
        };
        let (empty, before) = frame("");
        let (expanded, after) = frame("one\ntwo\nthree\nfour\nfive\nsix");
        assert!(expanded.height() > empty.height() + 10.0);
        assert!(expanded.height() <= 100.0);
        assert!(after < before, "the world reserves the taller bar");
        let (cleared, last) = frame("");
        assert!((cleared.height() - empty.height()).abs() < 0.5);
        assert_eq!(last, before);
    }

    #[test]
    fn toolbar_microphone_options_still_open_when_the_arrow_does_not_fit() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut mic = MicButton::default();
        let mut open_id = None;
        for requested in [false, true, true] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let response = mic_button(ui, &p, &Icons::default(), ButtonLook::bar(22.0), "", &mut mic, true, requested);
                open_id = Some(response.parent_id().with("mic_panel_open"));
            });
            output.textures_delta.clear();
            assert_eq!(
                ctx.data(|d| d.get_temp::<bool>(open_id.expect("mic parent"))).unwrap_or(false),
                requested
            );
        }
    }

    /// Where the microphone button stands in the test window, and its width.
    const BUTTON: egui::Pos2 = egui::pos2(400.0, 560.0);
    const WIDTH: f32 = 60.0;

    /// The microphone button alone in a window, with two devices to choose.
    #[derive(Default)]
    struct Bench {
        ctx: egui::Context,
        input: String,
        output: String,
        /// Outputs of the last frame.
        changed: bool,
        listed: bool,
        open_prefs: bool,
        /// The microphone's state, kept from frame to frame.
        on: bool,
        hold: bool,
        held: bool,
        denied: Option<String>,
    }

    impl Bench {
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let p = crate::theme::Theme::default().palette();
            let icons = Icons::default();
            let devices = ["Device A".to_owned(), "Device B".to_owned()];
            let mut mic = MicButton {
                connected: true,
                on: self.on,
                hold: self.hold,
                denied: self.denied.clone(),
                input_device: self.input.clone(),
                output_device: self.output.clone(),
                input_devices: &devices,
                output_devices: &devices,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let slot = egui::Rect::from_min_size(BUTTON, Vec2::new(WIDTH, 22.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                        mic_button(ui, &p, &icons, ButtonLook::bar(WIDTH), "", &mut mic, false, false);
                    });
                },
            );
            output.textures_delta.clear();
            self.input = mic.input_device.clone();
            self.output = mic.output_device.clone();
            self.changed = mic.devices_changed;
            self.listed = mic.list_devices;
            self.open_prefs = mic.open_prefs;
            self.on = mic.on;
            self.held = mic.held;
            output
        }

        fn click(&mut self, pos: egui::Pos2) {
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
        }

        /// Where `label` is written this frame, from top to bottom.
        fn texts(&mut self, label: &str) -> Vec<egui::Pos2> {
            fn find(shape: &egui::Shape, label: &str, found: &mut Vec<egui::Pos2>) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == label => found.push(text.pos + egui::vec2(3.0, 5.0)),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| find(shape, label, found)),
                    _ => {}
                }
            }
            let mut found = Vec::new();
            for shape in &self.frame(vec![]).shapes {
                find(&shape.shape, label, &mut found);
            }
            found.sort_by(|a, b| a.y.total_cmp(&b.y));
            found
        }

        fn panel_shown(&mut self) -> bool {
            !self.texts(aurora_i18n::tr!("ui-bars-sound-and-voice-settings")).is_empty()
        }

        /// The arrow right of the microphone, which opens the panel.
        fn arrow() -> egui::Pos2 {
            BUTTON + egui::vec2(WIDTH / 2.0 + 8.0 + MIC_ARROW / 2.0, 11.0)
        }
    }

    #[test]
    fn a_microphone_the_land_forbids_stays_off_and_keeps_its_options() {
        let centre = BUTTON + egui::vec2(WIDTH / 2.0, 11.0);
        let mut bench = Bench {
            denied: Some(aurora_i18n::tr!("voice-voice-disabled-on-this-parcel").into()),
            ..Default::default()
        };
        // a first frame lays the button out
        bench.frame(vec![]);
        bench.click(centre);
        assert!(!bench.on, "a click does not switch it on");
        bench.hold = true;
        bench.frame(vec![
            egui::Event::PointerMoved(centre),
            egui::Event::PointerButton {
                pos: centre,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        assert!(!bench.held, "holding it does not talk");
        bench.frame(vec![egui::Event::PointerButton {
            pos: centre,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        bench.click(Bench::arrow());
        assert!(bench.panel_shown(), "its options stay open");

        // allowed again, the same click switches it on
        let mut bench = Bench::default();
        bench.frame(vec![]);
        bench.click(centre);
        assert!(bench.on);
    }

    /// The landmark star alone at `STAR`: a frame with `events`, its
    /// action and its painted texts.
    fn star_frame(
        ctx: &egui::Context,
        found: Option<&crate::ui::landmark_create::ParcelLandmark>,
        events: Vec<egui::Event>,
    ) -> (Option<StarAction>, Vec<(String, egui::Pos2)>) {
        const STAR: egui::Pos2 = egui::pos2(300.0, 20.0);
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let mut action = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let slot = egui::Rect::from_min_size(STAR, Vec2::new(40.0, 22.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                    action = landmark_star(ui, &p, &icons, found);
                });
            },
        );
        output.textures_delta.clear();
        fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.pos + egui::vec2(3.0, 5.0))),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut found_texts = Vec::new();
        for s in &output.shapes {
            texts(&s.shape, &mut found_texts);
        }
        (action, found_texts)
    }

    fn click_events(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    }

    #[test]
    fn the_outline_star_opens_its_menu_and_the_lit_one_edits_its_landmark() {
        // the glyph's centre: the slot's left plus half a slot, its middle
        use crate::ui::parcel_icons::{SLOT_HEIGHT, SLOT_WIDTH};
        let star = egui::pos2(300.0 + SLOT_WIDTH / 2.0, 20.0 + SLOT_HEIGHT / 2.0);
        let ctx = egui::Context::default();
        let favorites = aurora_i18n::tr!("landmark-star-add-favorites");
        let (_, painted) = star_frame(&ctx, None, vec![]);
        assert!(painted.iter().all(|(t, _)| t != favorites), "menu closed at first");
        let mut acted = Vec::new();
        for pressed in [true, false] {
            acted.push(star_frame(&ctx, None, click_events(star, pressed)).0);
        }
        // a new popup is laid out over two frames
        star_frame(&ctx, None, vec![]);
        let (_, painted) = star_frame(&ctx, None, vec![]);
        let item = painted.iter().find(|(t, _)| t == favorites).map(|(_, pos)| *pos);
        let item = item.expect("the click opens the menu");
        assert!(acted.iter().all(Option::is_none), "opening the menu is no action");
        let mut chosen = None;
        for pressed in [true, false] {
            chosen = chosen.or(star_frame(&ctx, None, click_events(item, pressed)).0);
        }
        assert_eq!(chosen, Some(StarAction::AddFavorite));

        let lit = crate::ui::landmark_create::ParcelLandmark {
            item: uuid::Uuid::from_u128(7),
            name: "Plage".into(),
            favorite: true,
        };
        let ctx = egui::Context::default();
        star_frame(&ctx, Some(&lit), vec![]);
        let mut edit = None;
        for pressed in [true, false] {
            edit = edit.or(star_frame(&ctx, Some(&lit), click_events(star, pressed)).0);
        }
        assert_eq!(edit, Some(StarAction::Edit(lit.item)));
    }

    #[test]
    fn microphone_panel_stays_open_while_its_device_lists_are_used() {
        let default = aurora_i18n::tr!("ui-options-system-default");
        let mut bench = Bench::default();
        bench.frame(vec![]);
        assert!(!bench.panel_shown());
        bench.click(Bench::arrow());
        assert!(bench.listed, "opening the panel reads the devices again");
        assert!(bench.panel_shown());

        // the microphone: its list unfolds over the panel, which stays
        let combos = bench.texts(default);
        assert_eq!(combos.len(), 2, "one combo per device, both on the system's default");
        bench.click(combos[0]);
        for _ in 0..3 {
            assert!(bench.panel_shown(), "unfolding a device list must not close the panel");
        }
        let entry = *bench.texts("Device B").first().expect("the list of microphones is unfolded");
        bench.click(entry);
        assert!(bench.changed, "choosing a device is reported");
        assert_eq!(bench.input, "Device B");
        for _ in 0..3 {
            assert!(bench.panel_shown(), "the panel stays open once a device is chosen");
        }

        // then the audio output, in the same visit of the panel
        let combos = bench.texts(default);
        assert_eq!(combos.len(), 1, "only the output is still on the system's default");
        bench.click(combos[0]);
        assert!(bench.panel_shown());
        let entry = *bench.texts("Device A").first().expect("the list of outputs is unfolded");
        bench.click(entry);
        assert!(bench.changed);
        assert_eq!((bench.input.as_str(), bench.output.as_str()), ("Device B", "Device A"));
        for _ in 0..3 {
            assert!(bench.panel_shown());
        }

        // a click away from the panel and its button closes it
        bench.click(egui::pos2(50.0, 50.0));
        assert!(!bench.panel_shown());
    }

    #[test]
    fn microphone_panel_closes_on_its_arrow_escape_and_the_settings_button() {
        let mut bench = Bench::default();
        bench.frame(vec![]);
        bench.click(Bench::arrow());
        assert!(bench.panel_shown());
        bench.click(Bench::arrow());
        assert!(!bench.panel_shown(), "the arrow closes the panel it opened");

        // Escape closes an unfolded device list first, then the panel
        bench.click(Bench::arrow());
        assert!(bench.panel_shown());
        let combo = bench.texts(aurora_i18n::tr!("ui-options-system-default"))[0];
        bench.click(combo);
        assert!(!bench.texts("Device B").is_empty(), "the list is unfolded");
        let escape = || {
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        };
        bench.frame(escape());
        assert!(bench.texts("Device B").is_empty(), "Escape folds the list");
        assert!(bench.panel_shown(), "and leaves the panel open");
        bench.frame(escape());
        assert!(!bench.panel_shown(), "Escape closes the panel");

        bench.click(Bench::arrow());
        let settings = bench.texts(aurora_i18n::tr!("ui-bars-sound-and-voice-settings"))[0];
        bench.click(settings);
        assert!(bench.open_prefs, "the button opens the sound preferences");
        assert!(!bench.panel_shown(), "and closes the panel");
    }

    /// Issue #269: the button of a minimized window brings it back instead
    /// of closing it; a window shown closes, a closed one opens.
    #[test]
    fn a_toolbar_button_expands_its_minimized_window() {
        let ctx = egui::Context::default();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        for (cmd, id, open) in [
            ("conversations", "conversations", (|p| p.chat) as fn(&Panels) -> bool),
            ("inventory", "inventory", |p| p.inventory),
            ("gestures", "gestures", |p| p.gestures),
            ("people", "people", |p| p.people),
            ("preferences", "options", |p| p.settings),
            ("areasearch", super::super::area_search::WINDOW, |p| p.area_search),
            ("soundexplorer", super::super::sound_explorer::WINDOW, |p| p.sound_explorer),
            ("assetblacklist", super::super::blacklist::WINDOW, |p| p.asset_blacklist),
            ("gridstatus", super::super::grid_status::WINDOW, |p| p.grid_status),
        ] {
            run_command(&ctx, cmd, &mut panels);
            if !open(&panels) {
                run_command(&ctx, cmd, &mut panels);
            }
            assert!(open(&panels), "{cmd} opens");
            super::super::widgets::minimize_floater(&ctx, id);
            run_command(&ctx, cmd, &mut panels);
            assert!(open(&panels), "{cmd}: a minimized window stays open, to expand");
        }
        // shown again (its floater took the request): the button closes it
        ctx.data_mut(|d| d.insert_persisted(egui::Id::unique(("conversations", "minimized")), false));
        run_command(&ctx, "conversations", &mut panels);
        assert!(!panels.chat);
    }

    #[test]
    fn water_toolbar_buttons_dispatch_mode_toggles_without_opening_the_ao() {
        let ctx = egui::Context::default();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        panels.ao = false;
        assert!(matches!(run_command(&ctx, "movement", &mut panels), BarAction::ToggleSwimming));
        assert!(matches!(run_command(&ctx, "water_walk", &mut panels), BarAction::ToggleWaterWalk));
        assert!(!panels.ao);
    }

    #[test]
    fn resync_toolbar_command_is_available_and_dispatches_the_restart() {
        let ctx = egui::Context::default();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        assert!(super::super::toolbar::ready("resync_animations"));
        assert!(matches!(
            run_command(&ctx, "resync_animations", &mut panels),
            BarAction::ResyncAnimations
        ));
    }

    /// The texts and filled circles a button paints with its unread count.
    fn painted_count(width: f32, unread: usize) -> (Vec<String>, usize) {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let rect = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), Vec2::new(width, 22.0));
            paint_unread_pill(ui, &p, rect, 16.0, unread);
        });
        out.textures_delta.clear();
        let mut texts = Vec::new();
        let mut dots = 0;
        for s in &out.shapes {
            match &s.shape {
                egui::Shape::Text(t) => texts.push(t.galley.text().to_owned()),
                egui::Shape::Circle(_) => dots += 1,
                _ => {}
            }
        }
        (texts, dots)
    }

    /// The Conversations button shows how many messages wait, "99+" beyond
    /// 99; a button too narrow for the number keeps the dot.
    #[test]
    fn the_conversations_button_counts_the_unread_messages() {
        assert_eq!(painted_count(60.0, 3), (vec!["3".to_owned()], 0));
        assert_eq!(painted_count(60.0, 250), (vec!["99+".to_owned()], 0));
        assert_eq!(painted_count(30.0, 250), (vec!["99+".to_owned()], 0));
        assert_eq!(painted_count(14.0, 3), (Vec::new(), 1));
    }
}

#[cfg(test)]
mod import_menu_tests;
