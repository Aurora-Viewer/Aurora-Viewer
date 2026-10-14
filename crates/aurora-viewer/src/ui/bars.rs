//! Firestorm-like chrome: menu/status bar and navigation bar at the top,
//! chat input + feature buttons at the bottom. Flat, skinnable via
//! `skin::Layout` and the theme palette.

use super::Panels;
use super::icons::Icons;
use super::menu;
use super::skin::Layout;
use crate::theme::Palette;
use egui::{Color32, RichText, Vec2};

#[derive(Default)]
pub enum BarAction {
    CheckUpdates,
    ReportBug,
    /// Help › Report Abuse (LLFloaterReporter::showFromMenu).
    ReportAbuse,
    Import {
        choice: crate::imports::Choice,
        pick_files: bool,
    },
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
    ToggleFly,
    ToggleMouselook,
    SitGround,
    StandUp,
    ResetCamera,
    /// Teleport history (navigation bar arrows).
    TeleportBack,
    TeleportForward,
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
    /// World › Teleport history (ToggleTeleportHistory, Alt+H).
    TeleportHistory,
    /// The balance, or Me › Buy L$ (LLStatusBar::onClickBuyCurrency).
    BuyCurrency,
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
}

pub struct StatusInfo<'a> {
    pub show_bug_report: bool,
    pub show_huds: bool,
    pub hud_zoom: f32,
    pub region: &'a str,
    /// Parcel the agent stands on (may be empty).
    pub parcel: &'a str,
    /// Destinations of the back / forward arrows (None = disabled).
    pub tp_back: Option<String>,
    pub tp_forward: Option<String>,
    pub maturity: &'a str,
    pub position: glam::Vec3,
    pub fps: f32,
    pub flying: bool,
    pub seated: bool,
    pub ping_ms: u32,
    pub balance: Option<i32>,
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
    /// What a double click on the ground does (Me › Double-click on the ground).
    pub double_click: crate::autopilot::DoubleClick,
    /// The grid has a Marketplace (World › SL Marketplace is shown only on
    /// the Second Life grids, GridCheck "secondlife").
    pub marketplace: bool,
    /// The landmark lying on the agent's parcel (hasParcelLandmark): the
    /// star is lit and World › Landmark this place greyed out.
    pub parcel_landmark: Option<super::landmark_create::ParcelLandmark>,
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

fn slt_time() -> String {
    // SLT is US Pacific time; DST approximated by day of year (mid-March .. early Nov).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let year_day = (secs / 86400).rem_euclid(365);
    let off = if (68..=307).contains(&year_day) { -7 } else { -8 };
    let t = (secs + off * 3600).rem_euclid(86400);
    format!("{:02}:{:02}:{:02} SLT", t / 3600, (t / 60) % 60, t % 60)
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

fn icon_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, tip: &str, size: f32) -> bool {
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
    resp.on_hover_text(tip).clicked()
}

/// Local time of day choices (index = `Panels::time_of_day`).
pub fn time_of_day() -> [&'static str; 5] {
    [
        aurora_i18n::tr!("ui-bars-shared-environments-region"),
        aurora_i18n::tr!("ui-bars-sunrise"),
        aurora_i18n::tr!("common-midday"),
        aurora_i18n::tr!("ui-bars-sunset"),
        aurora_i18n::tr!("common-midnight"),
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

/// Color of the online status: grey invisible, red do not disturb, violet
/// away, orange automatic response, green available.
fn status_color(p: &Palette, m: &crate::world::status::StatusModes) -> Color32 {
    if m.invisible {
        p.muted
    } else if m.dnd {
        p.danger
    } else if m.away {
        p.violet
    } else if m.autorespond || m.autorespond_nonfriends {
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
        c |= flip(ui, p, "message-circle-filled", report, &mut m.report_ignored_adhoc, enabled);
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
    if status.invisible {
        // hollow: nobody sees the light
        ui.painter().circle_stroke(rect.center(), 4.25, egui::Stroke::new(1.5, col));
    } else {
        ui.painter().circle_filled(rect.center(), 5.0, col);
        ui.painter()
            .circle_stroke(rect.center(), 5.0, egui::Stroke::new(1.0, col.gamma_multiply(0.55)));
    }
    // refusals on: a thin ring around the dot
    if status.reject_teleports || status.reject_group_invites || status.reject_friendship || status.ignore_adhoc {
        ui.painter().circle_stroke(rect.center(), 7.5, egui::Stroke::new(1.0, p.muted));
    }
    let mut tip = aurora_i18n::tr!("ui-bars-status", value0 = status_name(&status).to_string());
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
    let resp = resp.on_hover_text(tip);
    // demo captures: AURORA_DEMO_STATUSMENU=1 opens it (=invisible: with
    // that status, set by the viewer)
    static DEMO_OPEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *DEMO_OPEN.get_or_init(|| std::env::var_os("AURORA_DEMO_STATUSMENU").is_some()) && ui.ctx().cumulative_frame_nr() == 300 {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
    }
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
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
        for (i, label, col) in [
            (0, aurora_i18n::tr!("ui-bars-available"), p.success),
            (1, aurora_i18n::tr!("common-away"), p.violet),
            (2, aurora_i18n::tr!("common-do-not-disturb"), p.danger),
            (3, aurora_i18n::tr!("ui-bars-invisible"), p.muted),
        ] {
            if menu::swatch_toggle(ui, p, col, label, current == i) && current != i {
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
    });
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
        menu::separator(ui, p);
        if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("ui-bars-preferences")) {
            super::widgets::open_floater(ui.ctx(), "options", &mut panels.settings);
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
        if menu::item_keys(ui, p, "hand-move", aurora_i18n::tr!("gesture-title"), "Ctrl+G") {
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
        menu::submenu(ui, p, "device-desktop-filled", "HUDs", true, |ui| {
            let mut visible = st.show_huds;
            if flip(ui, p, "eye-filled", aurora_i18n::tr!("ui-bars-show-huds"), &mut visible, true) {
                *action = BarAction::ShowHuds(visible);
            }
            for (zoom, icon, label) in [
                (1.0, "restore", aurora_i18n::tr!("ui-bars-normal-size")),
                ((st.hud_zoom / 1.2).max(0.1), "zoom-out-filled", aurora_i18n::tr!("common-collapse")),
                ((st.hud_zoom * 1.2).min(1.0), "zoom-in-filled", aurora_i18n::tr!("ui-bars-zoom-in")),
            ] {
                if menu::item(ui, p, icon, label) {
                    *action = BarAction::HudZoom(zoom);
                }
            }
        });
        // menu_viewer.xml: Teleport history (Alt+H), Places
        if menu::item_keys(ui, p, "history", aurora_i18n::tr!("common-teleport-history"), "Alt+H") {
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
        if menu::check(ui, p, "info-circle-filled", aurora_i18n::tr!("common-about-land"), panels.about_land, true) {
            panels.about_land = !panels.about_land;
        }
        if menu::item_keys(ui, p, "building-castle", aurora_i18n::tr!("region-title"), "Alt+R") {
            super::widgets::toggle_floater(ui.ctx(), super::region::WINDOW, &mut panels.region_estate);
        }
        // menu_viewer.xml: Search (Ctrl+F)
        if menu::item_keys(ui, p, "search-filled", aurora_i18n::tr!("search-title"), "Ctrl+F") {
            super::widgets::toggle_floater(ui.ctx(), super::search::WINDOW, &mut panels.search);
        }
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
        // menu_viewer.xml: Landmark this Place, after the maps
        if menu::item_if(
            ui,
            p,
            "map-pin-filled",
            aurora_i18n::tr!("landmark-create-here"),
            st.parcel_landmark.is_none(),
        ) {
            *action = BarAction::CreateLandmark;
        }
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
        menu::submenu(ui, p, "sunset-2-filled", aurora_i18n::tr!("common-environment"), true, |ui| {
            // back to the region / parcel EEP cycle, in real time
            let shared = !st.local_env;
            if menu::check(
                ui,
                p,
                "world-filled",
                aurora_i18n::tr!("ui-bars-use-shared-environments"),
                shared,
                true,
            ) {
                panels.time_of_day = 0;
                *action = BarAction::SharedEnvironment;
            }
            menu::separator(ui, p);
            let icons = ["", "sunrise-filled", "sun-high-filled", "sunset-2-filled", "moon-filled"];
            for (i, label) in time_of_day().iter().enumerate().skip(1) {
                if menu::check(ui, p, icons[i], label, panels.time_of_day == i as u8, true) {
                    panels.time_of_day = i as u8;
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
        });
        menu::separator(ui, p);
        if menu::item(ui, p, "eye-filled", aurora_i18n::tr!("common-mouselook-m")) {
            *action = BarAction::ToggleMouselook;
        }
        if menu::item(ui, p, "camera-filled", aurora_i18n::tr!("ui-bars-reset-camera-esc")) {
            *action = BarAction::ResetCamera;
        }
        if menu::item_keys(ui, p, "photo-filled", aurora_i18n::tr!("photo-title"), "Ctrl+Shift+S") {
            super::widgets::toggle_floater(ui.ctx(), super::snapshot::WINDOW, &mut panels.snapshot);
        }
    });
    menu::submenu(ui, p, "box", aurora_i18n::tr!("common-build"), true, |ui| {
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
    });
    menu::submenu(ui, p, "briefcase-filled", aurora_i18n::tr!("common-contents"), true, |ui| {
        flip_window(
            ui,
            p,
            "briefcase-filled",
            aurora_i18n::tr!("common-inventory"),
            "inventory",
            &mut panels.inventory,
        );
        import_menu(ui, p, action);
        if menu::item_keys(ui, p, "hand-move", aurora_i18n::tr!("gesture-title"), "Ctrl+G") {
            super::widgets::toggle_floater(ui.ctx(), "gestures", &mut panels.gestures);
        }
    });
    if advanced {
        menu::submenu(ui, p, "adjustments-filled", aurora_i18n::tr!("common-advanced"), true, |ui| {
            let perf = aurora_i18n::tr!("ui-bars-performance-ctrl-shift-1");
            flip_window(ui, p, "gauge-filled", perf, "perf", &mut panels.perf);
            let graphics = aurora_i18n::tr!("ui-bars-graphics-preferences");
            flip_window(ui, p, "settings-filled", graphics, "options", &mut panels.settings);
        });
    }
    menu::submenu(ui, p, "help-circle-filled", aurora_i18n::tr!("common-help"), true, |ui| {
        if crate::updater::checks_enabled() && menu::item(ui, p, "download-filled", aurora_i18n::tr!("update-check")) {
            *action = BarAction::CheckUpdates;
        }
        if menu::item(ui, p, "bug-filled", aurora_i18n::tr!("bug-title")) {
            *action = BarAction::ReportBug;
        }
        if menu::item(ui, p, "flag-filled", aurora_i18n::tr!("report-abuse-title")) {
            *action = BarAction::ReportAbuse;
        }
        menu::separator(ui, p);
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

fn import_menu(ui: &mut egui::Ui, p: &Palette, action: &mut BarAction) {
    use crate::imports::Choice;

    let heading = menu::submenu_heading(ui, p, "file-upload-filled", aurora_i18n::tr!("import-menu"), true, |ui| {
        // each kind keeps the icon it has in the inventory
        for (choice, icon) in [
            (Choice::Texture, "photo-filled"),
            (Choice::Sound, "file-music-filled"),
            (Choice::Animation, "run"),
            (Choice::Material, "droplet-filled"),
            (Choice::Mesh, "cube-3d-sphere"),
            (Choice::Bulk, "files-filled"),
            (Choice::Local, "eye-filled"),
        ] {
            if menu::item(ui, p, icon, choice.label()) {
                *action = BarAction::Import { choice, pick_files: true };
            }
        }
    });
    // Deliberate extension of Firestorm's upload submenu: its heading also opens the inspector.
    if heading.clicked() {
        *action = BarAction::Import {
            choice: Choice::Texture,
            pick_files: false,
        };
        ui.close();
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
fn nav_arrow(ui: &mut egui::Ui, p: &Palette, icons: &Icons, left: bool, dest: Option<&str>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(TOP_SLOT, egui::Sense::click());
    let enabled = dest.is_some();
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    let col = if !enabled {
        p.muted_dim.gamma_multiply(0.6)
    } else if resp.hovered() {
        p.ink
    } else {
        p.muted
    };
    paint_icon(ui, icons, if left { "arrow-left" } else { "arrow-right" }, rect, TOP_ICON, col);
    let tip = match dest {
        Some(d) if left => aurora_i18n::tr!("ui-bars-back", d = d.to_string()),
        Some(d) => aurora_i18n::tr!("ui-bars-next", d = d.to_string()),
        None if left => aurora_i18n::tr!("ui-bars-no-previous-destination").to_owned(),
        None => aurora_i18n::tr!("ui-bars-no-next-destination").to_owned(),
    };
    resp.on_hover_text(tip).clicked() && enabled
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
        if menu::item(ui, p, "star-filled", aurora_i18n::tr!("landmark-star-add-favorites")) {
            out = Some(StarAction::AddFavorite);
        }
        if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("landmark-star-add-landmarks")) {
            out = Some(StarAction::AddLandmark);
        }
        if menu::item(ui, p, "dots-filled", aurora_i18n::tr!("landmark-star-more")) {
            out = Some(StarAction::MoreOptions);
        }
    });
    resp.on_hover_text(aurora_i18n::tr!("landmark-star-add"));
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

/// Voice connection light: green connected, orange connecting, red not
/// connected, grey turned off. Returns true when clicked (voice settings).
fn voice_light(ui: &mut egui::Ui, p: &Palette, icons: &Icons, light: crate::voice::VoiceLight, why: &str) -> bool {
    use crate::voice::VoiceLight as L;
    let (col, state) = match light {
        L::Connected => (p.success, aurora_i18n::tr!("common-voice-connected")),
        L::Connecting => (p.amber, aurora_i18n::tr!("common-connecting-to-voice")),
        L::Down => (p.danger, aurora_i18n::tr!("ui-bars-voice-not-connected")),
        L::Off => (p.muted_dim, aurora_i18n::tr!("common-voice-disabled")),
    };
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
    let id = egui::Id::new("nav_location_edit");
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
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
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

/// The bell of the navigation bar: its count, its style (Preferences ›
/// Aurora › Notifications) and do not disturb (Aurora's paused bell).
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
                        main_menus(ui, p, panels, st, &mut action, true);
                    })
                    .on_hover_text(aurora_i18n::tr!("ui-bars-aurora-menu"));
                }
                // Firestorm's notification well: the bell next to the wolf
                if !bell.aurora {
                    let (clicked, rect) = super::notifications::bell(ui, p, icons, bell.unread, notif.open);
                    notif.anchor = Some(rect.left_bottom());
                    notif.bell = Some(rect);
                    if clicked {
                        notif.open = !notif.open;
                    }
                }
                // online status (away, do not disturb, automatic responses)
                if st.show_bug_report && icon_button(ui, p, icons, "bug-filled", aurora_i18n::tr!("bug-title"), TOP_ICON) {
                    action = BarAction::ReportBug;
                }
                status_dot(ui, p, st.status, &mut action);
                // back / forward through the teleport history, home (Firestorm navigation bar)
                if nav_arrow(ui, p, icons, true, st.tp_back.as_deref()) {
                    action = BarAction::TeleportBack;
                }
                if nav_arrow(ui, p, icons, false, st.tp_forward.as_deref()) {
                    action = BarAction::TeleportForward;
                }
                if icon_button(ui, p, icons, "Home_Off", aurora_i18n::tr!("common-go-home"), TOP_ICON) {
                    action = BarAction::TeleportHome;
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
                        let (clicked, rect) =
                            super::notification_center::bell(ui, p, icons, &notif.center, bell.unread, notif.open, bell.dnd);
                        notif.anchor = Some(rect.left_bottom());
                        notif.bell = Some(rect);
                        if clicked {
                            notif.open = !notif.open;
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
                    let fps = ui
                        .add(egui::Label::new(small(aurora_i18n::number((st.fps) as f64, 1), fps_col)).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(aurora_i18n::tr!("ui-bars-frames-per-second-click-for-performance-details"));
                    if fps.clicked() {
                        panels.perf = !panels.perf;
                    }
                    audio_action = super::audio::top_controls(ui, p, icons, audio, audio_ui, media);
                    if voice_light(ui, p, icons, st.voice.0, &st.voice.1) {
                        action = BarAction::VoicePrefs;
                    }
                    ui.label(small(format!("{} ms", st.ping_ms), p.muted))
                        .on_hover_text(aurora_i18n::tr!("ui-bars-region-latency"));
                    ui.label(small(slt_time(), p.muted))
                        .on_hover_text(aurora_i18n::tr!("ui-bars-second-life-time-pacific"));
                    if let Some(b) = st.balance {
                        // a click opens the L$ purchase
                        let balance = ui
                            .add(egui::Label::new(small(format!("{b} L$"), p.ink)).sense(egui::Sense::click()))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(aurora_i18n::tr!("ui-bars-balance-click-to-buy-l"));
                        if balance.clicked() {
                            action = BarAction::BuyCurrency;
                        }
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
                            let bg = ui.interact(field, ui.id().with("nav_location_area"), egui::Sense::click());
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
        "gestures" => toggle("gestures", &mut panels.gestures),
        "appearance" => toggle("appearance", &mut panels.appearance),
        "ao" => toggle("ao", &mut panels.ao),
        "minimap" => panels.minimap = !panels.minimap,
        "worldmap" => toggle("worldmap", &mut panels.world_map),
        "places" => toggle("places", &mut panels.places),
        "search" => toggle(super::search::WINDOW, &mut panels.search),
        "performance" => toggle("perf", &mut panels.perf),
        "snapshot" => toggle(super::snapshot::WINDOW, &mut panels.snapshot),
        "preferences" => toggle("options", &mut panels.settings),
        "fly" => return BarAction::ToggleFly,
        "mouselook" => return BarAction::ToggleMouselook,
        "sit" => return BarAction::SitGround,
        "home" => return BarAction::TeleportHome,
        _ => {}
    }
    BarAction::None
}

fn command_active(cmd: &str, panels: &Panels, flying: bool, seated: bool, mouselook: bool) -> bool {
    match cmd {
        "conversations" => panels.chat,
        "people" | "friends" => panels.people,
        "inventory" => panels.inventory,
        "gestures" => panels.gestures,
        "appearance" => panels.appearance,
        "minimap" => panels.minimap,
        "worldmap" => panels.world_map,
        "places" => panels.places,
        "search" => panels.search,
        "performance" => panels.perf,
        "snapshot" => panels.snapshot,
        "preferences" => panels.settings,
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
    egui::Id::new(("toolbar_button", command))
}

#[allow(clippy::too_many_arguments)]
fn tool_button(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    command: &str,
    icon: &str,
    tip: &str,
    active: bool,
    width: f32,
    unread: usize,
) -> bool {
    let size = Vec2::new(width, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let resp = ui.interact(rect, tool_button_id(command), egui::Sense::click());
    let icon_size = (width - 6.0).clamp(10.0, 16.0);
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter();
    if let Some(fill) = tool_fill(p, active, resp.hovered()) {
        painter.rect_filled(rect, radius, fill);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if active || resp.hovered() { p.ink } else { p.muted };
        let ir = egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
        painter.image(
            t.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    if icon == "AO" {
        // centered on the letters' ink like the icons beside it: the line box
        // of a font sits lower than its capitals
        let color = if active || resp.hovered() { p.ink } else { p.muted };
        let galley = painter.layout_no_wrap("AO".to_owned(), egui::FontId::proportional(12.0), color);
        // bold like the worn names (LLFontGL BOLD_OFFSET): the letters weigh
        // as much as the filled icons of the other buttons
        let bold = super::widgets::bold_text_offset(ui);
        super::widgets::paint_ink_centered(painter, galley.clone(), rect.center(), color);
        super::widgets::paint_ink_centered(painter, galley, rect.center() + bold, color);
    }
    if unread > 0 {
        paint_unread_pill(ui, p, rect, icon_size, unread);
    }
    resp.on_hover_text(tip).clicked()
}

/// The unread count of the Conversations button: the pill of the entries of
/// the conversation list (violet, its number in `on_accent`), on the top
/// right corner of the icon like the bell's count, inside the button so
/// that nothing moves. A button too narrow for it keeps a dot.
fn paint_unread_pill(ui: &egui::Ui, p: &Palette, rect: egui::Rect, icon: f32, unread: usize) {
    let painter = ui.painter();
    let text = if unread > 99 { "99+".to_owned() } else { unread.to_string() };
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(9.5), p.on_accent);
    const H: f32 = 13.0;
    let w = (galley.size().x + 7.0).max(H);
    if rect.width() < w + 4.0 {
        painter.circle_filled(
            egui::pos2(rect.right() - (rect.width() * 0.18).clamp(4.0, 8.0), rect.center().y),
            (rect.width() * 0.07).clamp(2.0, 3.0),
            p.violet,
        );
        return;
    }
    let left = (rect.center().x + icon * 0.2).min(rect.right() - 1.0 - w);
    let pill = egui::Rect::from_min_size(egui::pos2(left, rect.top() - 1.0), Vec2::new(w, H));
    painter.rect_filled(pill, ui.visuals().window_corner_radius, p.violet);
    super::widgets::paint_ink_centered(painter, galley, pill.center(), p.on_accent);
}

/// Microphone button state (Firestorm "Speak" button with its arrow).
#[derive(Default)]
pub struct MicButton<'a> {
    /// Toggle mode: microphone switched on.
    pub on: bool,
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
}

/// Width of the arrow's zone, right of the microphone.
const MIC_ARROW: f32 = 14.0;

fn mic_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, width: f32, mic: &mut MicButton) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), egui::Sense::hover());
    let icon_size = (width - 6.0).clamp(10.0, 16.0);
    // the microphone stands in the middle of the button, in line with the
    // icons of its neighbors, and the arrow right after it; a button too
    // narrow for the arrow keeps the microphone, and a right click on it
    // opens the panel
    let with_arrow = width >= icon_size + 2.0 * (MIC_ARROW + 1.0);
    let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
    let resp = ui.interact(rect, ui.id().with("mic_main"), egui::Sense::click_and_drag());
    let arrow = with_arrow.then(|| {
        let zone = egui::Rect::from_min_max(
            egui::pos2(icon_rect.right(), rect.top()),
            egui::pos2(icon_rect.right() + MIC_ARROW, rect.bottom()),
        );
        ui.interact(zone, ui.id().with("mic_arrow"), egui::Sense::click())
    });
    if mic.hold {
        mic.held = resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
    } else if resp.clicked() {
        mic.on = !mic.on;
    }
    // The panel keeps its open state itself. egui's memory holds a single
    // open pop-up, and the list of a device combo is one: kept there, the
    // panel would close as soon as such a list unfolds.
    let open_id = ui.id().with("mic_panel_open");
    let open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let on_arrow = arrow.as_ref().is_some_and(|r| r.hovered());
    let radius = ui.visuals().window_corner_radius;
    let live = mic.talking;
    if let Some(fill) = tool_fill(p, live, resp.hovered() || on_arrow || open) {
        ui.painter().rect_filled(rect, radius, fill);
    }
    // own level, along the bottom of the button: it follows the voice
    // closely, only smoothing the jitter between two samples
    let level = if live { mic.level.clamp(0.0, 1.0) } else { 0.0 };
    let level = ui.ctx().animate_value_with_time(ui.id().with("mic_level"), level, 0.08);
    let level_width = (rect.width() - 6.0) * level;
    if level_width > 0.5 {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(rect.left() + 3.0, rect.bottom() - 3.0), Vec2::new(level_width, 2.0)),
            super::widgets::TRACK_RADIUS,
            p.teal,
        );
    }
    let icon = if live || (!mic.hold && mic.on) {
        "microphone-filled"
    } else {
        "microphone-off"
    };
    let tint = if live || resp.hovered() {
        p.ink
    } else if mic.connected {
        p.muted
    } else {
        p.muted_dim
    };
    paint_icon(ui, icons, icon, icon_rect, icon_size, tint);
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
    let tip = if mic.hold {
        aurora_i18n::tr!("ui-bars-hold-to-talk", state = state.to_string())
    } else {
        aurora_i18n::tr!("extra-click-to-enable-mute-microphone", state = state.to_string())
    };
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
        None => resp.secondary_clicked(),
    };
    let toggled = toggle || demo;
    mic.list_devices = toggled && !open;
    let anchor = arrow.as_ref().unwrap_or(&resp);
    // a click in the list of a device combo is a click outside the panel:
    // the panel closes itself, and not while such a list is (or just was) open
    let list_id = ui.id().with("mic_device_list");
    let list_was_open = ui.ctx().data(|d| d.get_temp::<bool>(list_id)).unwrap_or(false);
    let mut list_open = false;
    let mut open_now = open != toggled;
    // Escape and the panel's own `ui.close()` clear the flag
    let shown = egui::Popup::from_response(anchor)
        .open_bool(&mut open_now)
        .align(egui::RectAlign::TOP_END)
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

/// Bottom bar: "Nearby chat" input on the left, feature buttons spread over
/// the rest of the width. Returns (chat, action, top y of the bar).
#[allow(clippy::too_many_arguments)]
pub fn bottom_bar(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    layout: &Layout,
    panels: &mut Panels,
    chat: &mut super::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    unread: usize,
    flying: bool,
    seated: bool,
    mouselook: bool,
    chat_width: &mut f32,
    mic: &mut MicButton,
    ao_active: bool,
) -> (Option<super::chat::OutgoingChat>, BarAction, f32, bool) {
    let mut out = None;
    let mut action = BarAction::None;
    let mut width_committed = false;
    let mut glass = None;
    let resp = egui::Panel::bottom("bottom_bar")
        .frame(bar_frame(p).inner_margin(egui::Margin::symmetric(8, 3)))
        .exact_size(layout.bottom_bar_height)
        .show(ui, |ui| {
            glass = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
            ui.set_min_width(ui.available_width());
            ui.horizontal_centered(|ui| {
                let total = ui.available_width();
                let n = layout.toolbar.len().max(1) as f32;
                // the toolbar keeps at least a narrow button per command
                const MIN_BTN: f32 = 22.0;
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

                // buttons stretch over the whole remaining width (hover and
                // click area included); the gap and the pill drawn under the
                // pointer show where each one starts and ends
                let remaining = ui.available_width();
                let gap = 4.0;
                let btn_w = ((remaining - gap * (n - 1.0) - 1.0) / n).floor().max(MIN_BTN);
                ui.spacing_mut().item_spacing.x = gap;
                for b in &layout.toolbar {
                    if b.command == "speak" {
                        mic_button(ui, p, icons, btn_w, mic);
                        continue;
                    }
                    let active = if b.command == "ao" {
                        ao_active
                    } else {
                        command_active(&b.command, panels, flying, seated, mouselook)
                    };
                    // Firestorm flashes the button while the conversations
                    // are closed or minimized (LLIMMgr, flashCommand)
                    let badge = b.command == "conversations"
                        && unread > 0
                        && (!panels.chat || super::widgets::floater_minimized(ui.ctx(), "conversations"));
                    let count = if badge { unread } else { 0 };
                    let tip = if badge {
                        aurora_i18n::tr!("ui-bars-unread", value0 = b.tooltip.to_string(), unread = format!("{}", unread))
                    } else {
                        b.tooltip.clone()
                    };
                    if tool_button(ui, p, icons, &b.command, &b.icon, &tip, active, btn_w, count) {
                        let a = run_command(ui.ctx(), &b.command, panels);
                        if !matches!(a, BarAction::None) {
                            action = a;
                        }
                    }
                }
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
    }

    impl Bench {
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let p = crate::theme::Theme::default().palette();
            let icons = Icons::default();
            let devices = ["Device A".to_owned(), "Device B".to_owned()];
            let mut mic = MicButton {
                connected: true,
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
                        mic_button(ui, &p, &icons, WIDTH, &mut mic);
                    });
                },
            );
            output.textures_delta.clear();
            self.input = mic.input_device.clone();
            self.output = mic.output_device.clone();
            self.changed = mic.devices_changed;
            self.listed = mic.list_devices;
            self.open_prefs = mic.open_prefs;
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
        ctx.data_mut(|d| d.insert_persisted(egui::Id::new(("conversations", "minimized")), false));
        run_command(&ctx, "conversations", &mut panels);
        assert!(!panels.chat);
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
mod import_menu_tests {
    use super::*;
    use crate::imports::Choice;

    fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .next_back()
            .expect("menu item visible")
    }

    #[test]
    fn parent_click_opens_texture_and_submenu_choices_request_their_file_picker() {
        for choice in [
            None,
            Some(Choice::Texture),
            Some(Choice::Sound),
            Some(Choice::Animation),
            Some(Choice::Material),
            Some(Choice::Mesh),
            Some(Choice::Bulk),
            Some(Choice::Local),
        ] {
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let mut time = 0.0;
            let mut frame = |events| {
                time += 0.2;
                let mut action = BarAction::None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        egui::MenuBar::new().ui(ui, |ui| {
                            ui.menu_button(aurora_i18n::tr!("common-contents"), |ui| import_menu(ui, &p, &mut action));
                        });
                    },
                );
                output.textures_delta.clear();
                (output, action)
            };
            let click_events = |pos, pressed| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let (output, _) = frame(Vec::new());
            let contents = text_position(&output, aurora_i18n::tr!("common-contents"));
            for pressed in [true, false] {
                frame(click_events(contents, pressed));
            }
            let mut output = frame(Vec::new()).0;
            for _ in 0..3 {
                output = frame(Vec::new()).0;
            }
            let heading = text_position(&output, aurora_i18n::tr!("import-menu"));
            // Hover must expose the submenu without opening an inspector or a native dialog.
            for _ in 0..3 {
                let (next, action) = frame(vec![egui::Event::PointerMoved(heading)]);
                assert!(matches!(action, BarAction::None));
                output = next;
            }
            let pos = choice.map_or(heading, |choice| text_position(&output, choice.label()));
            frame(click_events(pos, true));
            let (_, action) = frame(click_events(pos, false));
            assert!(matches!(action, BarAction::Import { choice: selected, pick_files }
                if selected == choice.unwrap_or(Choice::Texture) && pick_files == choice.is_some()));
            let (output, _) = frame(Vec::new());
            assert!(
                !output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::epaint::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-menu"))),
                "menu closes after activation"
            );
        }
    }
}
