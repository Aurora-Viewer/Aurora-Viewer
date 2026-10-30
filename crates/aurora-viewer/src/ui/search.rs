//! "Search": Firestorm's legacy search floater (FSFloaterSearch,
//! indra/newview/fsfloatersearch.cpp, floater_fs_search.xml and
//! panel_fs_search_legacy_*.xml, originally LGPL 2.1) drawn the Aurora way:
//! one window with the tabs People, Groups, Places, Land Sales,
//! Events, Classifieds and Web. Each directory tab has its form (the fields
//! `find()` reads), the result list with sortable headers, the count and the
//! page buttons, and the details of the selected row (`search/details.rs`);
//! the web tab hosts the search page in the embedded browser
//! (FSPanelSearchWeb).
//!
//! The window only reads the world: what it asks for comes back as
//! [`SearchAction`]s. State and rules: `world/search.rs`. This is the
//! Legacy look (Preferences › Aurora); Aurora's own, one field and the
//! details beside rich rows, is `search/aurora.rs`, on the same lists.

mod aurora;
mod aurora_details;
#[cfg(test)]
mod aurora_tests;
mod details;

use super::place_details::Ctx;
use super::web_view::WebView;
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::search::{Key, List, RatingBoxes, Request, Row, RowIcon, SearchSettings, Status, Tab, Value};
use aurora_net::search::{
    AgentAccess, EventsMode, LandFilter, LandKind, LandSort, PLACE_CATEGORY_ANY, SIM_ACCESS_ADULT, SIM_ACCESS_MATURE,
};
use egui::{Rect, RichText, Sense, Vec2};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Id of the window (layout, raise, demo placement).
pub const WINDOW: &str = "search";

/// The size the window opens with: room for the list beside the details
/// (floater_fs_search.xml opens at 780 x 590, with a smaller font).
const DEFAULT_SIZE: Vec2 = Vec2::new(900.0, 600.0);
const MIN_SIZE: Vec2 = Vec2::new(720.0, 440.0);

/// The window before the user moves or resizes it: in the middle of the
/// work area (`layout::work_area`; floater_fs_search.xml is "centered"),
/// with the gap of the first launch windows to the bars in a low viewer
/// window. It stands over the middle of the screen, which those windows
/// leave free: each of them stays visible beside it, down to 1280 x 720.
pub(crate) fn default_rect(work: Rect) -> Rect {
    let room = work.size() - Vec2::splat(2.0 * super::layout::GAP);
    let size = DEFAULT_SIZE.min(room).max(MIN_SIZE);
    Rect::from_min_size((work.center() - size * 0.5).round(), size)
}

/// The group pane of the details in a window of its own (the group profile
/// window): `standalone` leaves out the button that opens that window.
pub(crate) fn group_details(ui: &mut egui::Ui, c: &mut Ctx, id: Uuid, name: &str, standalone: bool, actions: &mut Vec<SearchAction>) {
    details::group(ui, c, id, name, standalone, actions);
}

/// What the window asks the app to do.
#[derive(Clone)]
pub enum SearchAction {
    /// onBtnFind of a tab.
    Find(Request),
    /// onBtnNext (true) / onBtnBack.
    Page(Tab, bool),
    /// A row was clicked (onSelectItem).
    Select(Tab, Key),
    /// A column header was clicked.
    Sort(Tab, usize),
    /// onBtnTeleport: teleportViaLocation + trackLocation, then
    /// FSLegacySearchActionOnTeleport.
    Teleport(DVec3),
    /// onBtnMap: trackLocation and the world map.
    ShowOnMap(DVec3),
    /// Ask for a group's profile for a visible row or an explicit refresh.
    RefreshGroup(Uuid),
    /// LLGroupActions::join: use the group's fee and limit confirmation.
    JoinGroup(Uuid),
    /// The saved choices changed.
    SettingsChanged,
    /// A row of Aurora's window shows a parcel (place, land sale) whose
    /// snapshot and region it does not have yet: ParcelInfoRequest.
    WantParcel(Uuid),
    /// The same for a classified: ClassifiedInfoRequest.
    WantClassified(Uuid),
}

/// What the web tab needs from the app.
pub struct WebEnv<'a> {
    pub paths: Option<&'a aurora_media::PluginPaths>,
    pub browser: &'a aurora_media::BrowserSettings,
    /// The OpenID cookie of the session, given to the browser before its
    /// first page like LLViewerMedia::getOpenIDCookie does for the search
    /// floater's "webbrowser"; the browser only sends it to its own domain.
    pub cookie: Option<&'a aurora_media::Cookie>,
    /// The search page of the grid, None when it has none (or offline). On a
    /// grid it carries the session id (`sid`): loaded, never shown.
    pub home: Option<String>,
    /// The session id to keep out of every address shown or handed over.
    pub session: Option<Uuid>,
    /// The grid's search page is opened without the session (beta grid):
    /// said in the tab, so that the user knows why the page ignores them.
    pub anonymous: bool,
}

/// The address of the web tab's field: the page's, without the session.
fn shown_address(location: &str, session: Option<Uuid>) -> String {
    crate::world::search::without_session(location, session)
}

/// What "Open in browser" opens: the page shown, or the search
/// page, as an anonymous address (the system browser never gets the
/// session).
fn external_address(location: &str, home: &str, session: Option<Uuid>) -> String {
    let url = if location.is_empty() { home } else { location };
    crate::world::search::without_session(url, session)
}

pub struct SearchUi {
    tab: usize,
    /// The saved tab was restored (once per run).
    restored: bool,
    /// The text typed in each directory tab.
    texts: [String; 6],
    places_category: i8,
    events_category: u32,
    classifieds_category: u32,
    land_sort: LandSort,
    land_ascending: bool,
    land_price: String,
    land_area: String,
    /// "events_search_mode": by date instead of current and upcoming.
    events_by_day: bool,
    /// mDay: days from today.
    events_day: i32,
    /// Give the keyboard to the tab's search field (window or tab opened).
    focus: bool,
    /// The window was shown on the previous frame.
    was_open: bool,
    web: WebView,
    /// The address being typed in the web tab (None: follows the page).
    web_edit: Option<String>,
    /// The search page the browser was started on: another one (a new
    /// session) is loaded in its place.
    web_home: String,
    /// Profile pictures of the residents shown.
    pub wanted_pics: HashSet<Uuid>,
    /// Aurora's window: its field, type and selection.
    aurora: aurora::Aurora,
}

impl Default for SearchUi {
    fn default() -> Self {
        SearchUi {
            tab: 0,
            restored: false,
            texts: Default::default(),
            places_category: PLACE_CATEGORY_ANY,
            events_category: 0,
            classifieds_category: 0,
            land_sort: LandSort::Name,
            land_ascending: false,
            // the price box is ticked by default and limits to 0 L$: the
            // released Firestorm shows that 0 (its sources leave "edit_price"
            // empty, which sends the same 0); an empty field would read as
            // "no limit"
            land_price: "0".into(),
            land_area: String::new(),
            events_by_day: false,
            events_day: 0,
            focus: true,
            was_open: false,
            // the search page is loaded with the session: nothing of it
            // stays in the shared browser cache
            web: WebView::private(crate::settings::cache_dir().join(crate::media::private_profile::ROOT)),
            web_edit: None,
            web_home: String::new(),
            wanted_pics: HashSet::new(),
            aurora: aurora::Aurora::default(),
        }
    }
}

/// LLParcel::ECategory of a places search, in Firestorm's order
/// (C_LINDEN..C_RENTAL), after "all categories".
fn place_categories() -> [(i8, &'static str); 15] {
    [
        (PLACE_CATEGORY_ANY, aurora_i18n::tr!("search-all-categories")),
        (1, aurora_i18n::tr!("search-place-linden")),
        (2, aurora_i18n::tr!("common-adult")),
        (3, aurora_i18n::tr!("search-place-arts")),
        (4, aurora_i18n::tr!("search-place-business")),
        (5, aurora_i18n::tr!("search-place-educational")),
        (6, aurora_i18n::tr!("search-place-gaming")),
        (7, aurora_i18n::tr!("search-place-hangout")),
        (8, aurora_i18n::tr!("search-place-newcomer")),
        (9, aurora_i18n::tr!("search-place-parks")),
        (10, aurora_i18n::tr!("search-place-residential")),
        (11, aurora_i18n::tr!("search-place-shopping")),
        (12, aurora_i18n::tr!("search-place-stage")),
        (13, aurora_i18n::tr!("search-place-other")),
        (14, aurora_i18n::tr!("search-place-rental")),
    ]
}

/// "events_category" of panel_fs_search_legacy_events.xml (value, label).
fn event_categories() -> [(u32, &'static str); 13] {
    [
        (0, aurora_i18n::tr!("search-all-categories")),
        (18, aurora_i18n::tr!("search-event-discussion")),
        (19, aurora_i18n::tr!("search-event-sports")),
        (30, aurora_i18n::tr!("search-event-dj")),
        (20, aurora_i18n::tr!("search-event-music")),
        (22, aurora_i18n::tr!("search-event-commercial")),
        (23, aurora_i18n::tr!("search-event-nightlife")),
        (24, aurora_i18n::tr!("search-event-games")),
        (25, aurora_i18n::tr!("search-event-pageants")),
        (26, aurora_i18n::tr!("search-event-education")),
        (27, aurora_i18n::tr!("search-event-arts")),
        (28, aurora_i18n::tr!("search-event-charity")),
        (29, aurora_i18n::tr!("search-event-misc")),
    ]
}

/// Titles of a tab's columns after the name, with their widths.
fn columns(tab: Tab) -> Vec<(&'static str, f32)> {
    match tab {
        Tab::Groups => vec![(aurora_i18n::tr!("search-col-members"), 74.0)],
        Tab::Places => vec![(aurora_i18n::tr!("search-col-traffic"), 70.0)],
        Tab::Land => vec![
            (aurora_i18n::tr!("common-type"), 96.0),
            (aurora_i18n::tr!("search-col-price"), 64.0),
            (aurora_i18n::tr!("search-col-area"), 68.0),
            (aurora_i18n::tr!("search-col-per-meter"), 58.0),
        ],
        Tab::Events => vec![(aurora_i18n::tr!("search-col-time"), 118.0)],
        Tab::Classifieds => vec![(aurora_i18n::tr!("search-col-price"), 70.0)],
        Tab::People | Tab::Web => Vec::new(),
    }
}

/// The text of a cell and whether it is a figure (written against the right
/// edge of its column).
fn cell_text(v: &Value) -> (String, bool) {
    match v {
        Value::None => (String::new(), false),
        Value::Int(i) => (i.to_string(), true),
        Value::Real(r) => (aurora_i18n::number_text(&format!("{r:.1}")), true),
        Value::Text(t) => (t.clone(), false),
        Value::Dated { text, .. } => (text.clone(), false),
    }
}

/// "> 100 found" when a next page exists, else the number of rows.
fn count_text(list: &List) -> String {
    if list.has_next {
        aurora_i18n::tr!("search-found-more", count = list.rows.len() as i64)
    } else {
        aurora_i18n::tr!("search-found", count = list.rows.len() as i64)
    }
}

/// The comment of an empty list (LLScrollListCtrl::setCommentText).
fn status_text(status: &Status) -> String {
    match status {
        Status::Idle | Status::Found => aurora_i18n::tr!("search-no-results").to_owned(),
        Status::Searching => aurora_i18n::tr!("search-searching").to_owned(),
        Status::NotFound(text) => aurora_i18n::tr!("search-not-found", text = text.to_string()),
        Status::TooShort => aurora_i18n::tr!("search-too-short").to_owned(),
        Status::Banned => aurora_i18n::tr!("search-banned").to_owned(),
        Status::Disabled => aurora_i18n::tr!("search-disabled").to_owned(),
        Status::NoDateOffset => aurora_i18n::tr!("search-no-date-offset").to_owned(),
        Status::NoCategory => aurora_i18n::tr!("search-no-event-category").to_owned(),
        Status::NoQuery => aurora_i18n::tr!("search-no-query").to_owned(),
        Status::NoContent => aurora_i18n::tr!("search-no-content").to_owned(),
    }
}

fn row_icon(icon: RowIcon) -> &'static str {
    match icon {
        RowIcon::Person => "user-filled",
        RowIcon::Group => "users-group",
        RowIcon::Place => "map-pin-filled",
        RowIcon::ForSale => "tag-filled",
        RowIcon::Auction => "gavel",
        RowIcon::Classified => "article-filled",
        RowIcon::EventGeneral | RowIcon::EventModerate | RowIcon::EventAdult => "calendar-event-filled",
    }
}

/// A rating box: check box and the letter badge of the rating, in reading
/// order whichever way the row is laid out. A rating the account may not see
/// stays unticked and says why.
fn rating_box(ui: &mut egui::Ui, p: &Palette, on: &mut bool, rating: &str, allowed: bool) -> bool {
    let rtl = ui.layout().prefer_right_to_left();
    let mut shown = *on && allowed;
    let mut check = |ui: &mut egui::Ui| {
        let resp = ui
            .add_enabled_ui(allowed, |ui| {
                // a box without a label is as wide as a row is high, its mark
                // at the left: here only as wide as the mark, so that the
                // badge stands against the box it belongs to
                ui.spacing_mut().interact_size.y = ui.spacing().icon_width;
                widgets::checkbox(ui, p, &mut shown, "")
            })
            .inner;
        let changed = resp.changed();
        let _ = resp.on_disabled_hover_text(aurora_i18n::tr!("search-rating-not-allowed"));
        changed
    };
    // box and badge close together, the pairs apart
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.x, 5.0);
    let changed = if rtl {
        ui.add_space(6.0);
        widgets::maturity_badge(ui, p, rating, 16.0);
        check(ui)
    } else {
        let changed = check(ui);
        widgets::maturity_badge(ui, p, rating, 16.0);
        ui.add_space(6.0);
        changed
    };
    ui.spacing_mut().item_spacing.x = spacing;
    if changed {
        *on = shown;
    }
    changed
}

/// The G / M / A boxes of a tab, read from left to right also in a row laid
/// out from the right; true when one changed.
fn rating_boxes(ui: &mut egui::Ui, p: &Palette, boxes: &mut RatingBoxes, access: AgentAccess) -> bool {
    let mut items = [
        (&mut boxes.pg, aurora_i18n::tr!("common-general"), true),
        (&mut boxes.mature, aurora_i18n::tr!("common-moderate"), access.can_access_mature()),
        (&mut boxes.adult, aurora_i18n::tr!("common-adult"), access.can_access_adult()),
    ];
    if ui.layout().prefer_right_to_left() {
        items.reverse();
    }
    let mut changed = false;
    for (on, rating, allowed) in items {
        changed |= rating_box(ui, p, on, rating, allowed);
    }
    changed
}

/// A form row whose search field takes the room its other controls leave:
/// `controls` lays them out from the right edge, the search button first.
fn field_row(ui: &mut egui::Ui, text: &mut String, hint: &str, focus: &mut bool, controls: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let mut find = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            find |= controls(ui);
            let width = ui.available_width();
            find |= query_field(ui, text, hint, width, focus);
        });
    });
    find
}

/// A drop-down list of (value, label); true when the choice changed.
fn combo<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, value: &mut T, items: &[(T, &str)], width: f32) -> bool {
    let current = items.iter().find(|i| i.0 == *value).map_or("", |i| i.1);
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(width)
        .show_ui(ui, |ui| {
            for (v, label) in items {
                if ui.selectable_label(*v == *value, *label).clicked() && *v != *value {
                    *value = *v;
                    changed = true;
                }
            }
        });
    changed
}

/// The search field of a tab, exactly `width` wide with its frame (a width
/// asked to the field itself would not count its margins, and a row wider
/// than the window makes it grow); true on Enter.
fn query_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32, focus: &mut bool) -> bool {
    let size = Vec2::new(width.max(80.0), ui.spacing().interact_size.y);
    let resp = ui.add_sized(
        size,
        egui::TextEdit::singleline(text).hint_text(hint).align(egui::Align2::LEFT_CENTER),
    );
    if std::mem::take(focus) {
        resp.request_focus();
    }
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

fn find_button(ui: &mut egui::Ui, p: &Palette) -> bool {
    widgets::action_button(
        ui,
        p,
        "search-filled",
        aurora_i18n::tr!("search-find"),
        ActionKind::Primary,
        Vec2::new(0.0, 24.0),
    )
    .clicked()
}

/// Seconds since the Unix epoch.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl SearchUi {
    /// The system id of the search page's browser process, while one runs.
    pub fn web_process_id(&self) -> Option<u32> {
        self.web.process_id()
    }

    /// Open on a tab (FSFloaterSearch::onOpen with a "tab" key, demo).
    pub fn open_tab(&mut self, tab: Tab) {
        self.tab = tab.index();
        self.restored = true;
        self.focus = true;
        self.aurora.open(aurora::Kind::of(tab));
    }

    /// Aurora's window on its "All" chip (demo).
    pub fn open_all(&mut self) {
        self.aurora.open(aurora::Kind::All);
        self.focus = true;
    }

    pub fn tab(&self) -> Tab {
        Tab::from_index(self.tab)
    }

    /// The web page that has the keyboard, if any.
    pub fn web_has_focus(&self) -> bool {
        self.tab() == Tab::Web && self.web.focused && self.web.running()
    }

    /// The web page that has the keyboard, if any.
    pub fn focused_web(&mut self) -> Option<&mut aurora_media::MediaPlugin> {
        (self.tab() == Tab::Web && self.web.focused)
            .then(|| self.web.plugin_mut())
            .flatten()
    }

    /// Demo: the text typed in the tab's field (the price field of the land
    /// sales, which have no search field).
    pub fn set_text(&mut self, tab: Tab, text: &str) {
        self.aurora.text = text.to_owned();
        if tab == Tab::Land {
            self.land_price = text.to_owned();
        } else if let Some(t) = tab.list_index().and_then(|i| self.texts.get_mut(i)) {
            *t = text.to_owned();
        }
    }

    /// The request of the tab's form (the fields `find()` reads).
    pub fn request(&self, tab: Tab, settings: &SearchSettings) -> Option<Request> {
        let text = |t: Tab| t.list_index().and_then(|i| self.texts.get(i)).cloned().unwrap_or_default();
        Some(match tab {
            Tab::People => Request::People { text: text(tab) },
            Tab::Groups => Request::Groups {
                text: text(tab),
                maturity: settings.group_maturity,
            },
            Tab::Places => Request::Places {
                text: text(tab),
                ratings: settings.places.ratings(),
                category: self.places_category,
            },
            Tab::Land => Request::Land {
                ratings: settings.land.ratings(),
                filter: LandFilter {
                    kind: LandKind::from_setting(settings.land_type),
                    sort: self.land_sort,
                    ascending: self.land_ascending,
                    limit_price: settings.land_limit_price,
                    limit_area: settings.land_limit_area,
                    // childGetValue(...).asInteger(): 0 for an empty field
                    price: self.land_price.trim().parse().unwrap_or(0),
                    area: self.land_area.trim().parse().unwrap_or(0),
                },
            },
            Tab::Events => Request::Events {
                text: text(tab),
                ratings: settings.events.ratings(),
                mode: if self.events_by_day {
                    EventsMode::Day(self.events_day)
                } else {
                    EventsMode::Current
                },
                category: self.events_category,
            },
            Tab::Classifieds => Request::Classifieds {
                text: text(tab),
                ratings: settings.classifieds.ratings(),
                category: self.classifieds_category,
            },
            Tab::Web => return None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        c: &mut Ctx,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        settings: &mut SearchSettings,
        web: &WebEnv,
        classic: bool,
        open: &mut bool,
    ) -> Vec<SearchAction> {
        let mut actions = Vec::new();
        if !*open {
            // the browser does not run behind a closed window
            if self.web.running() {
                self.web.close();
            }
            self.was_open = false;
            return actions;
        }
        // focusDefaultElement: the keyboard goes to the search field
        if !std::mem::replace(&mut self.was_open, true) {
            self.focus = true;
        }
        if !std::mem::replace(&mut self.restored, true) {
            self.tab = (settings.last_tab as usize).min(Tab::ALL.len() - 1);
        }
        let p = c.p;
        let default = default_rect(super::layout::work_area(ctx));
        let mut floater = Floater::new(WINDOW, aurora_i18n::tr!("search-title"), default.min, default.size());
        floater.min_size = MIN_SIZE;
        floater.show(ctx, p, open, |ui| {
            if !classic {
                self.aurora_body(ui, c, pics, settings, web, &mut actions);
                return;
            }
            // "ls_tabs" of floater_fs_search.xml, in its order (Tab::ALL)
            let labels = [
                (aurora_i18n::tr!("search-tab-web"), true),
                (aurora_i18n::tr!("common-people"), true),
                (aurora_i18n::tr!("common-groups"), true),
                (aurora_i18n::tr!("common-places"), true),
                (aurora_i18n::tr!("search-tab-land"), true),
                (aurora_i18n::tr!("search-tab-events"), true),
                (aurora_i18n::tr!("search-tab-classifieds"), true),
            ];
            if widgets::tabs(ui, p, &mut self.tab, &labels) {
                self.focus = true;
                settings.last_tab = self.tab as u8;
                actions.push(SearchAction::SettingsChanged);
            }
            ui.add_space(6.0);
            let tab = self.tab();
            if tab == Tab::Web {
                self.web_tab(ui, p, web);
                return;
            }
            if self.form(ui, c, tab, settings, &mut actions)
                && let Some(request) = self.request(tab, settings)
            {
                actions.push(SearchAction::Find(request));
            }
            ui.add_space(6.0);
            let Some(list) = c.world.search.list(tab) else {
                return;
            };
            // list on the left, details on the right; land has more columns
            let body = Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
            ui.allocate_rect(body, Sense::hover());
            let fraction = if tab == Tab::Land { 0.56 } else { 0.42 };
            let left_w = (body.width() * fraction).clamp(260.0, (body.width() - 300.0).max(260.0));
            let left = Rect::from_min_max(body.min, egui::pos2(body.left() + left_w, body.bottom()));
            let right = Rect::from_min_max(egui::pos2(left.right() + 10.0, body.top()), body.max);
            let mut list_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("search-list", self.tab))
                    .max_rect(left)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            results(&mut list_ui, c, list, &mut actions);
            let mut detail_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("search-details", self.tab))
                    .max_rect(right)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            details::show(&mut detail_ui, c, pics, &mut self.wanted_pics, list, &mut actions);
        });
        actions
    }

    /// The form of a directory tab. Returns true when a search is asked
    /// (the button, Enter in a field, or a filter changed: "CommitSearch").
    fn form(&mut self, ui: &mut egui::Ui, c: &mut Ctx, tab: Tab, settings: &mut SearchSettings, actions: &mut Vec<SearchAction>) -> bool {
        let p = c.p;
        let access = c.world.agent_access();
        let mut find = false;
        let mut saved = false;
        let hint = aurora_i18n::tr!("common-search");
        // the web tab has no form
        let Some(i) = tab.list_index() else {
            return false;
        };
        match tab {
            Tab::People => {
                find |= field_row(ui, &mut self.texts[i], hint, &mut self.focus, |ui| find_button(ui, p));
            }
            Tab::Groups => {
                let maturity = &mut settings.group_maturity;
                find |= field_row(ui, &mut self.texts[i], hint, &mut self.focus, |ui| {
                    let mut go = find_button(ui, p);
                    // "groups_filter" of panel_fs_search_legacy_groups.xml
                    let levels = [
                        (aurora_net::search::SIM_ACCESS_PG, aurora_i18n::tr!("common-general")),
                        (SIM_ACCESS_MATURE, aurora_i18n::tr!("search-maturity-gm")),
                        (SIM_ACCESS_ADULT, aurora_i18n::tr!("search-maturity-gma")),
                    ];
                    if combo(ui, "search-group-maturity", maturity, &levels, 190.0) {
                        saved = true;
                        go = true;
                    }
                    go
                });
            }
            Tab::Places => {
                let (category, boxes) = (&mut self.places_category, &mut settings.places);
                find |= field_row(ui, &mut self.texts[i], hint, &mut self.focus, |ui| {
                    let mut go = find_button(ui, p);
                    if rating_boxes(ui, p, boxes, access) {
                        saved = true;
                        go = true;
                    }
                    go | combo(ui, "search-place-category", category, &place_categories(), 170.0)
                });
            }
            Tab::Classifieds => {
                let cats = c.world.classified_categories();
                let mut items = vec![(0, aurora_i18n::tr!("search-all-categories"))];
                items.extend(cats.iter().map(|(id, name)| (*id, name.as_str())));
                let (category, boxes) = (&mut self.classifieds_category, &mut settings.classifieds);
                find |= field_row(ui, &mut self.texts[i], hint, &mut self.focus, |ui| {
                    let mut go = find_button(ui, p);
                    if rating_boxes(ui, p, boxes, access) {
                        saved = true;
                        go = true;
                    }
                    go | combo(ui, "search-classified-category", category, &items, 170.0)
                });
            }
            Tab::Land => {
                ui.horizontal(|ui| {
                    let kinds = [
                        (LandKind::All, aurora_i18n::tr!("search-all-categories")),
                        (LandKind::Auction, aurora_i18n::tr!("search-land-auction")),
                        (LandKind::Mainland, aurora_i18n::tr!("search-land-mainland")),
                        (LandKind::Estate, aurora_i18n::tr!("search-land-estate")),
                    ];
                    // "land_category", kept in FindLandType
                    let mut kind = LandKind::from_setting(settings.land_type);
                    if combo(ui, "search-land-kind", &mut kind, &kinds, 170.0) {
                        settings.land_type = kind.setting();
                        saved = true;
                        find = true;
                    }
                    let mut limit = |ui: &mut egui::Ui, on: &mut bool, label: &str, text: &mut String| {
                        let mut go = false;
                        if widgets::checkbox(ui, p, on, label).changed() {
                            saved = true;
                            go = true;
                        }
                        let field = ui.add_enabled(*on, egui::TextEdit::singleline(text).desired_width(54.0).char_limit(9));
                        if field.changed() {
                            text.retain(|ch| ch.is_ascii_digit());
                        }
                        go |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        go
                    };
                    find |= limit(
                        ui,
                        &mut settings.land_limit_price,
                        aurora_i18n::tr!("search-land-max-price"),
                        &mut self.land_price,
                    );
                    find |= limit(
                        ui,
                        &mut settings.land_limit_area,
                        aurora_i18n::tr!("search-land-min-area"),
                        &mut self.land_area,
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("search-land-sort")).size(12.0).color(p.muted));
                    let sorts = [
                        (LandSort::Name, aurora_i18n::tr!("common-name")),
                        (LandSort::Price, aurora_i18n::tr!("search-col-price")),
                        (LandSort::PerMeter, aurora_i18n::tr!("search-land-sort-per-meter")),
                        (LandSort::Area, aurora_i18n::tr!("search-col-area")),
                    ];
                    find |= combo(ui, "search-land-sort", &mut self.land_sort, &sorts, 130.0);
                    find |= widgets::checkbox(ui, p, &mut self.land_ascending, aurora_i18n::tr!("search-land-ascending")).changed();
                    ui.add_space(8.0);
                    if rating_boxes(ui, p, &mut settings.land, access) {
                        saved = true;
                        find = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        find |= find_button(ui, p);
                    });
                });
            }
            Tab::Events => {
                ui.horizontal(|ui| {
                    // "events_search_mode": the day buttons only work by date
                    let mut by_day = self.events_by_day;
                    widgets::radio_value(ui, p, &mut by_day, false, aurora_i18n::tr!("search-events-current"));
                    widgets::radio_value(ui, p, &mut by_day, true, aurora_i18n::tr!("search-events-by-date"));
                    self.events_by_day = by_day;
                    ui.add_space(6.0);
                    let step = |ui: &mut egui::Ui, icon: &str, tip: &str| widgets::icon_button(ui, p, icon, tip, by_day);
                    if step(ui, "caret-left-filled", aurora_i18n::tr!("search-events-previous-day")) {
                        self.events_day -= 1;
                        find = true;
                    }
                    // setDay: "month/day" in Pacific time
                    let (_, month, day, ..) = super::land::slt_parts(unix_now() + self.events_day as i64 * 86_400);
                    let date = aurora_i18n::tr!("search-events-date", month = month as i64, day = day);
                    ui.add_sized(
                        [44.0, 20.0],
                        egui::Label::new(RichText::new(date).size(12.5).color(if by_day { p.ink } else { p.muted_dim })),
                    );
                    if step(ui, "caret-right-filled", aurora_i18n::tr!("search-events-next-day")) {
                        self.events_day += 1;
                        find = true;
                    }
                    if ui
                        .add_enabled(
                            by_day,
                            egui::Button::new(RichText::new(aurora_i18n::tr!("search-events-today")).size(12.0)),
                        )
                        .clicked()
                    {
                        self.events_day = 0;
                        find = true;
                    }
                });
                let (category, boxes) = (&mut self.events_category, &mut settings.events);
                find |= field_row(ui, &mut self.texts[i], hint, &mut self.focus, |ui| {
                    let mut go = find_button(ui, p);
                    if rating_boxes(ui, p, boxes, access) {
                        saved = true;
                        go = true;
                    }
                    go | combo(ui, "search-event-category", category, &event_categories(), 170.0)
                });
            }
            Tab::Web => {}
        }
        if saved {
            actions.push(SearchAction::SettingsChanged);
        }
        // SearchWordBanned, said once under the form
        if tab == Tab::Classifieds && c.world.search.banned_notice {
            ui.label(RichText::new(aurora_i18n::tr!("search-banned")).size(11.5).color(p.warn));
        }
        find
    }

    /// The web tab: back, forward, reload, home, the address and the page.
    fn web_tab(&mut self, ui: &mut egui::Ui, p: &Palette, env: &WebEnv) {
        let Some(home) = env.home.as_deref() else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("search-web-unavailable")).size(12.5).color(p.muted));
            });
            return;
        };
        // loadURL(SearchQuery()): the search page at once. Firestorm loads it
        // each time the floater opens, whatever the tab; Aurora waits for the
        // web tab to be shown (no browser and no request before that). A page
        // of an earlier session is replaced.
        if self.web_home != home {
            self.web_home = home.to_owned();
            self.web_edit = None;
            self.web.navigate_again(home);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let running = self.web.running();
            if widgets::icon_button(ui, p, "arrow-left", aurora_i18n::tr!("search-web-back"), running) {
                self.web.back();
            }
            if widgets::icon_button(ui, p, "arrow-right", aurora_i18n::tr!("search-web-forward"), running) {
                self.web.forward();
            }
            if widgets::icon_button(ui, p, "reload", aurora_i18n::tr!("search-web-reload"), running) {
                self.web.reload();
            }
            if widgets::icon_button(ui, p, "home-filled", aurora_i18n::tr!("search-web-home"), true) {
                self.web_edit = None;
                self.web.navigate_again(home);
            }
            let external = Vec2::new(0.0, 22.0);
            let label = aurora_i18n::tr!("search-web-open-external");
            let reserve = ui
                .painter()
                .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), p.ink)
                .size()
                .x
                + 62.0;
            // the address follows the page until it is edited; the session
            // id of the search page is never part of what is shown
            let mut address = self
                .web_edit
                .clone()
                .unwrap_or_else(|| shown_address(&self.web.location, env.session));
            let field = ui.add(
                egui::TextEdit::singleline(&mut address)
                    .desired_width((ui.available_width() - reserve).max(120.0))
                    .font(egui::FontId::proportional(12.0)),
            );
            if field.changed() {
                self.web_edit = Some(address.clone());
            }
            if field.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let url = address.trim();
                    // only web pages, as the links of the chat
                    if url.starts_with("https://") || url.starts_with("http://") {
                        self.web.navigate_again(url);
                    }
                }
                self.web_edit = None;
            }
            if widgets::action_button(ui, p, "external-link-filled", label, ActionKind::Flat, external).clicked() {
                let url = external_address(&self.web.location, home, env.session);
                super::context::request(ui.ctx(), super::context::CtxAction::OpenUrl(url));
            }
        });
        if env.anonymous {
            ui.label(RichText::new(aurora_i18n::tr!("search-web-anonymous")).size(11.5).color(p.muted));
        }
        ui.add_space(4.0);
        let size = Vec2::new(ui.available_width(), (ui.available_height() - 2.0).max(120.0));
        self.web.show(ui, p, env.paths, env.browser, env.cookie, size);
    }
}

/// The result list: sortable headers, rows, then the count and the page
/// buttons.
fn results(ui: &mut egui::Ui, c: &mut Ctx, list: &List, actions: &mut Vec<SearchAction>) {
    let p = c.p;
    let tab = list.tab;
    let cols = columns(tab);
    let fixed: f32 = cols.iter().map(|c| c.1).sum();
    let radius = ui.visuals().window_corner_radius;
    // cells of a row: the name takes what the other columns leave
    let cells = |row: Rect| -> Vec<Rect> {
        let name_w = (row.width() - fixed).max(80.0);
        let mut x = row.left();
        std::iter::once(name_w)
            .chain(cols.iter().map(|c| c.1))
            .map(|w| {
                let r = Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(x + w, row.bottom()));
                x += w;
                r
            })
            .collect()
    };
    let font = egui::FontId::proportional(12.0);
    // header
    // the columns over those of the rows, left of the list's scroll bar
    let rows_list = ui.scope_id().with(("search-rows", tab.index()));
    let columns = widgets::header_width(ui, rows_list);
    let (head, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    ui.painter().rect_filled(head, radius, p.field);
    let head = head.with_max_x(head.left() + columns);
    let titles: Vec<&str> = std::iter::once(aurora_i18n::tr!("common-name"))
        .chain(cols.iter().map(|c| c.0))
        .collect();
    for (index, (cell, title)) in cells(head).into_iter().zip(&titles).enumerate() {
        let resp = ui.interact(cell, ui.scope_id().with(("search-header", index)), Sense::click());
        let sorted = list.sort.filter(|s| s.0 == index);
        let color = if sorted.is_some() || resp.hovered() { p.ink } else { p.muted };
        let left = egui::pos2(cell.left() + if index == 0 { 26.0 } else { 6.0 }, cell.center().y);
        widgets::paint_elided(ui, title, left, cell.right() - left.x - 14.0, font.clone(), color);
        if let Some((_, descending)) = sorted {
            let arrow = if descending { "caret-down-filled" } else { "caret-up-filled" };
            if let Some(t) = super::icons::global(arrow) {
                // where the radar puts the arrow of its name column
                let at = Rect::from_center_size(egui::pos2(cell.right() - 10.0, cell.center().y), Vec2::splat(8.0));
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(t.id(), at, uv, p.violet_light);
            }
        }
        if resp.on_hover_text(*title).clicked() {
            actions.push(SearchAction::Sort(tab, index));
        }
    }
    // footer first, so that the rows take what is left
    let footer_h = 30.0;
    let rows_h = (ui.available_height() - footer_h).max(60.0);
    let rows = list.sorted();
    let out = egui::ScrollArea::vertical()
        .id_salt(("search-rows", tab.index()))
        .auto_shrink([false, false])
        .max_height(rows_h)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if rows.is_empty() {
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(status_text(&list.status)).size(12.0).color(p.muted));
                });
                return;
            }
            for row in &rows {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let selected = list.selected == Some(row.key);
                if selected {
                    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                }
                let cut = paint_row(ui, p, row, &cells(rect), &font);
                let resp = if cut { resp.on_hover_text(&row.name) } else { resp };
                if resp.clicked() && !selected {
                    actions.push(SearchAction::Select(tab, row.key));
                }
                if let (Tab::People, Key::Id(id)) = (tab, row.key) {
                    // LLAvatarActions::showProfile on a double click;
                    // gFSAvatarSearchMenu on a right click
                    if resp.double_clicked() {
                        super::profile::request_open(ui.ctx(), id);
                    }
                    super::menu::context_menu(&resp, p, |ui| {
                        super::context::avatar_list_menu(ui, p, c.world, id, super::context::AvatarList::Name)
                    });
                }
            }
        });
    widgets::remember_scroll_bar(ui, rows_list, &out);
    // count on the left, pages on the right
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let count = if list.rows.is_empty() { String::new() } else { count_text(list) };
        ui.label(RichText::new(count).size(12.0).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let size = Vec2::new(0.0, 22.0);
            let next = aurora_i18n::tr!("search-next-page");
            if widgets::action_button_if(ui, p, "caret-right-filled", next, ActionKind::Flat, size, list.has_next).clicked() {
                actions.push(SearchAction::Page(tab, true));
            }
            let back = aurora_i18n::tr!("search-previous-page");
            if widgets::action_button_if(ui, p, "caret-left-filled", back, ActionKind::Flat, size, list.has_back()).clicked() {
                actions.push(SearchAction::Page(tab, false));
            }
            if list.page() > 1 {
                let page = aurora_i18n::tr!("search-page", page = list.page() as i64);
                ui.label(RichText::new(page).size(12.0).color(p.muted));
            }
        });
    });
}

/// Paint a row in its cells; true when the name was cut short.
fn paint_row(ui: &egui::Ui, p: &Palette, row: &Row, cells: &[Rect], font: &egui::FontId) -> bool {
    let Some(name_cell) = cells.first() else {
        return false;
    };
    let icon_rect = Rect::from_center_size(egui::pos2(name_cell.left() + 13.0, name_cell.center().y), Vec2::splat(14.0));
    // the rating of an event tints its icon like the rating badges
    let tint = match row.icon {
        RowIcon::EventModerate => p.ink,
        RowIcon::EventAdult => p.danger,
        RowIcon::Auction | RowIcon::ForSale => p.amber,
        _ => p.violet_light,
    };
    if let Some(t) = super::icons::global(row_icon(row.icon)) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(t.id(), icon_rect, uv, tint);
    }
    let left = egui::pos2(name_cell.left() + 26.0, name_cell.center().y);
    let cut = widgets::paint_elided(ui, &row.name, left, name_cell.right() - left.x - 4.0, font.clone(), p.ink);
    for (value, cell) in row.values.iter().zip(cells.iter().skip(1)) {
        let (text, figure) = cell_text(value);
        if text.is_empty() {
            continue;
        }
        if figure {
            let galley = ui.painter().layout_no_wrap(text, font.clone(), p.ink);
            let at = crate::ui::widgets::row_text_origin(ui.painter(), &galley, cell.right() - 14.0 - galley.size().x, cell.center().y);
            ui.painter().galley(at, galley, p.ink);
        } else {
            let left = egui::pos2(cell.left() + 6.0, cell.center().y);
            widgets::paint_elided(ui, &text, left, cell.width() - 10.0, font.clone(), p.muted);
        }
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_builds_the_request_of_its_tab() {
        let mut ui = SearchUi::default();
        let mut settings = SearchSettings::default();
        ui.set_text(Tab::Places, "plage");
        // the last directory tab has its own text too
        ui.set_text(Tab::Classifieds, "meubles");
        assert!(matches!(
            ui.request(Tab::Classifieds, &settings),
            Some(Request::Classifieds { text, .. }) if text == "meubles"
        ));
        // the window first opens on the web search, as FSLastSearchTab = 0
        assert_eq!(SearchUi::default().tab(), Tab::Web);
        assert_eq!(
            ui.request(Tab::Places, &settings),
            Some(Request::Places {
                text: "plage".into(),
                ratings: Default::default(),
                category: PLACE_CATEGORY_ANY
            })
        );
        // as Firestorm first shows the panel: FindLandPrice on over a field
        // that shows the 0 it limits to, FindLandArea off over an empty field
        assert_eq!((ui.land_price.as_str(), ui.land_area.as_str()), ("0", ""));
        let Some(Request::Land { filter, .. }) = ui.request(Tab::Land, &settings) else {
            panic!("land");
        };
        assert_eq!(filter, LandFilter::default());
        assert!(filter.limit_price && !filter.limit_area);
        assert_eq!((filter.price, filter.area), (0, 0));
        // a cleared price field still sends 0, and is not filled again
        ui.land_price.clear();
        let Some(Request::Land { filter: cleared, .. }) = ui.request(Tab::Land, &settings) else {
            panic!("land");
        };
        assert_eq!(cleared, filter);
        assert_eq!(ui.land_price, "");
        ui.land_price = "5000".into();
        ui.land_area = "512".into();
        let Some(Request::Land { filter, .. }) = ui.request(Tab::Land, &settings) else {
            panic!("land");
        };
        // the area goes out even with its box unticked
        assert_eq!(
            (filter.limit_price, filter.price, filter.limit_area, filter.area),
            (true, 5000, false, 512)
        );
        settings.land_limit_price = false;
        settings.land_limit_area = true;
        settings.land_type = LandKind::Estate.setting();
        let Some(Request::Land { filter, .. }) = ui.request(Tab::Land, &settings) else {
            panic!("land");
        };
        assert_eq!(
            (filter.limit_price, filter.price, filter.limit_area, filter.area, filter.kind),
            (false, 5000, true, 512, LandKind::Estate)
        );
        // events: current and upcoming unless a day is chosen
        assert!(matches!(
            ui.request(Tab::Events, &settings),
            Some(Request::Events {
                mode: EventsMode::Current,
                category: 0,
                ..
            })
        ));
        ui.events_by_day = true;
        ui.events_day = -2;
        assert!(matches!(
            ui.request(Tab::Events, &settings),
            Some(Request::Events {
                mode: EventsMode::Day(-2),
                ..
            })
        ));
        assert_eq!(ui.request(Tab::Web, &settings), None);
    }

    #[test]
    fn the_web_tab_never_shows_or_hands_over_the_session() {
        let session = Uuid::from_u128(0x5E55_1011_0000_4000_8000_0000_0000_00A1);
        let access = AgentAccess::default();
        let home = crate::world::search::web_url("secondlife.com", "", access, "fr", Some(session));
        assert!(home.contains(&session.to_string()), "the page itself is loaded with it");
        let anonymous = crate::world::search::web_url("secondlife.com", "", access, "fr", None);
        // the field, before and after the first page reported its address
        assert_eq!(shown_address(&home, Some(session)), anonymous);
        assert_eq!(shown_address("", Some(session)), "");
        // the system browser gets the anonymous address
        assert_eq!(external_address("", &home, Some(session)), anonymous);
        assert_eq!(external_address(&home, &home, Some(session)), anonymous);
        let page = format!("https://search.secondlife.com/viewer/p?x=1&sid={session}");
        let external = external_address(&page, &home, Some(session));
        assert_eq!(external, "https://search.secondlife.com/viewer/p?x=1");
        assert!(!external.contains(&session.to_string()));
    }

    #[test]
    fn categories_follow_firestorm() {
        // LLParcel::C_LINDEN..C_RENTAL after "all"
        let ids: Vec<i8> = place_categories().iter().map(|c| c.0).collect();
        assert_eq!(ids, [-1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
        // panel_fs_search_legacy_events.xml (21 is commented out there)
        let ids: Vec<u32> = event_categories().iter().map(|c| c.0).collect();
        assert_eq!(ids, [0, 18, 19, 30, 20, 22, 23, 24, 25, 26, 27, 28, 29]);
        for tab in Tab::ALL {
            assert!(columns(tab).iter().all(|c| !c.0.is_empty()));
        }
        assert_eq!(columns(Tab::Land).len(), 4);
    }

    #[test]
    fn cells_and_counts() {
        assert_eq!(cell_text(&Value::Int(2500)), ("2500".to_owned(), true));
        assert_eq!(cell_text(&Value::None), (String::new(), false));
        let dated = Value::Dated {
            text: "10/12 18:00".into(),
            unix: 5,
        };
        assert_eq!(cell_text(&dated), ("10/12 18:00".to_owned(), false));
        assert_eq!(row_icon(RowIcon::Auction), "gavel");
    }
}
