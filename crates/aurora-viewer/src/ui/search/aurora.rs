//! Aurora's "Search" (the default; Preferences › Aurora gives back the
//! Legacy Viewer's tabs, `ui/search.rs`): one field for every directory,
//! chips for the type with the number of results, the filters of the type
//! chosen on one line, rich rows (the place card of `ui/place_card.rs`) and
//! the details of the selected row beside them (`aurora_details.rs`).
//!
//! The queries, their replies, pages and status codes are those of the
//! tabs (`world/search.rs`), list by list: "All" sends the five text
//! searches at once (people, places, events, groups, classifieds), each into
//! its own list, and shows their first rows in sections; land sales have no
//! text and are searched by their filters alone. A row that shows a parcel
//! or a classified asks for its details once it is on screen
//! (ParcelInfoRequest, ClassifiedInfoRequest), for its snapshot, its region
//! and its rating: the lists of the Legacy Viewer's places panels do the
//! same for each landmark they show (LLRemoteParcelInfoProcessor).

use super::{SearchAction, SearchUi, WebEnv, aurora_details, combo, event_categories, place_categories, status_text, unix_now};
use crate::theme::Palette;
use crate::ui::place_card::{self, RowSize, Thumb};
use crate::ui::place_details::Ctx;
use crate::ui::widgets::{self, ActionKind};
use crate::world::search::{Key, Request, Row, RowIcon, Search, SearchSettings, Status, Tab, Value};
use aurora_net::search::{
    AgentAccess, EventsMode, LandFilter, LandKind, LandSort, PLACE_CATEGORY_ANY, SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG,
};
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The chips: every directory at once, one of them, or the grid's web page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    All,
    People,
    Places,
    Events,
    Groups,
    Classifieds,
    Land,
    Web,
}

/// The sections of "All", in the order of the chips.
const SECTIONS: [Tab; 5] = [Tab::People, Tab::Places, Tab::Events, Tab::Groups, Tab::Classifieds];
/// Rows of a section of "All" before its "See all".
const SECTION_ROWS: usize = 3;
/// Detail requests the rows shown may send in one frame.
const ASKS_PER_FRAME: usize = 6;
/// The details pane beside the results.
const DETAILS_W: f32 = 372.0;

impl Kind {
    const CHIPS: [Kind; 7] = [
        Kind::All,
        Kind::People,
        Kind::Places,
        Kind::Events,
        Kind::Groups,
        Kind::Classifieds,
        Kind::Land,
    ];

    /// The chip of a Legacy tab (a window opened on a tab: a group link,
    /// the demo).
    pub fn of(tab: Tab) -> Kind {
        match tab {
            Tab::Web => Kind::Web,
            Tab::People => Kind::People,
            Tab::Groups => Kind::Groups,
            Tab::Places => Kind::Places,
            Tab::Land => Kind::Land,
            Tab::Events => Kind::Events,
            Tab::Classifieds => Kind::Classifieds,
        }
    }

    fn tab(self) -> Option<Tab> {
        Some(match self {
            Kind::All | Kind::Web => return None,
            Kind::People => Tab::People,
            Kind::Places => Tab::Places,
            Kind::Events => Tab::Events,
            Kind::Groups => Tab::Groups,
            Kind::Classifieds => Tab::Classifieds,
            Kind::Land => Tab::Land,
        })
    }

    /// The lists a search of this chip fills.
    fn tabs(self) -> Vec<Tab> {
        match self {
            Kind::All => SECTIONS.to_vec(),
            k => k.tab().into_iter().collect(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::All => aurora_i18n::tr!("search-aurora-all"),
            Kind::People => aurora_i18n::tr!("common-people"),
            Kind::Places => aurora_i18n::tr!("common-places"),
            Kind::Events => aurora_i18n::tr!("search-tab-events"),
            Kind::Groups => aurora_i18n::tr!("common-groups"),
            Kind::Classifieds => aurora_i18n::tr!("search-tab-classifieds"),
            Kind::Land => aurora_i18n::tr!("search-tab-land"),
            Kind::Web => aurora_i18n::tr!("search-tab-web"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Kind::All => "layout-grid-filled",
            Kind::People => "user-filled",
            Kind::Places => "map-pin-filled",
            Kind::Events => "calendar-event-filled",
            Kind::Groups => "users-group",
            Kind::Classifieds => "article-filled",
            Kind::Land => "tag-filled",
            Kind::Web => "world-filled",
        }
    }
}

/// The state of Aurora's window beside the lists of `world/search.rs`.
pub struct Aurora {
    kind: Kind,
    /// The field: the same text for every directory.
    pub text: String,
    /// The request each list was last searched with here: a chip shows its
    /// list again without a new search while the field and the filters
    /// have not changed.
    searched: HashMap<Tab, Request>,
    /// The row chosen in "All" (one of the sections'); a single type uses
    /// its list's selection.
    selected: Option<(Tab, Key)>,
    /// "Price ≤" and "Area ≥" of land sales: empty, no limit.
    land_price: String,
    land_area: String,
    /// Parcels, classifieds and groups whose details a visible row asked for.
    asked: HashSet<Uuid>,
    /// The text the grid's web page was opened with: the field's when the
    /// Web chip was chosen, or when Enter is pressed on it.
    web_query: String,
}

impl Default for Aurora {
    fn default() -> Self {
        Aurora {
            kind: Kind::All,
            text: String::new(),
            searched: HashMap::new(),
            selected: None,
            land_price: String::new(),
            land_area: String::new(),
            asked: HashSet::new(),
            web_query: String::new(),
        }
    }
}

impl Aurora {
    pub fn open(&mut self, kind: Kind) {
        self.kind = kind;
        self.selected = None;
    }
}

/// "Today · 19:00 SLT", "Tomorrow · …", or the weekday and date, in SLT
/// like the event dates of the directory.
fn event_when(unix: i64, now: i64) -> String {
    let (y, m, d, weekday, h, min, _) = crate::ui::land::slt_parts(unix);
    let same = |t: i64| {
        let (ty, tm, td, ..) = crate::ui::land::slt_parts(t);
        (ty, tm, td) == (y, m, d)
    };
    let day = if same(now) {
        aurora_i18n::tr!("search-events-today").to_owned()
    } else if same(now + 86_400) {
        aurora_i18n::tr!("search-aurora-tomorrow").to_owned()
    } else {
        let date = aurora_i18n::tr!("search-events-date", month = m as i64, day = d);
        format!("{} {date}", crate::ui::land::weekdays()[weekday])
    };
    aurora_i18n::tr!("search-aurora-event-time", day = day, time = format!("{h:02}:{min:02}"))
}

/// The user name of a directory row: "First Last" → "first.last", a
/// Resident's "First" → "first" (LLCacheName::buildUsername).
fn username(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', ".")
}

/// "L$ 12 500".
pub(super) fn linden(amount: i64) -> String {
    aurora_i18n::tr!("search-aurora-linden", amount = aurora_i18n::integer(amount))
}

/// The rating of an event row, as its icon carries it.
fn event_rating(icon: RowIcon) -> &'static str {
    match icon {
        RowIcon::EventAdult => aurora_i18n::tr!("common-adult"),
        RowIcon::EventModerate => aurora_i18n::tr!("common-moderate"),
        _ => aurora_i18n::tr!("common-general"),
    }
}

/// A small pill of a row or of the details (for sale, started, member).
pub(super) fn pill(ui: &mut egui::Ui, text: &str, ink: Color32, fill: Color32) {
    let font = FontId::new(11.0, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let size = Vec2::new(galley.size().x + 12.0, 18.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    widgets::paint_ink_centered(ui.painter(), galley, rect.center(), ink);
}

pub(super) fn sale_pill(ui: &mut egui::Ui, p: &Palette, auction: bool) {
    let text = if auction {
        aurora_i18n::tr!("search-aurora-auction")
    } else {
        aurora_i18n::tr!("search-aurora-for-sale")
    };
    pill(ui, text, p.amber, p.amber.gamma_multiply(0.14));
}

pub(super) fn started_pill(ui: &mut egui::Ui, p: &Palette) {
    pill(ui, aurora_i18n::tr!("search-aurora-started"), p.teal, p.teal.gamma_multiply(0.14));
}

/// A muted figure with its icon before it, in a row laid out from the right.
fn meta(ui: &mut egui::Ui, p: &Palette, icon: Option<&str>, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(p.muted));
    if let Some(i) = icon {
        ui.add_space(-4.0);
        widgets::glyph(ui, i, p.muted, 13.0);
    }
}

/// A G / M / A toggle: its badge and name, lit on `raised` while on; a
/// rating the account may not see stays off and says why.
fn rating_toggle(ui: &mut egui::Ui, p: &Palette, on: &mut bool, name: &str, allowed: bool) -> bool {
    let font = FontId::proportional(12.0);
    let lit = *on && allowed;
    let ink = if lit { p.ink } else { p.muted_dim };
    let galley = ui.painter().layout_no_wrap(name.to_owned(), font, ink);
    let size = Vec2::new(6.0 + 16.0 + 6.0 + galley.size().x + 9.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if allowed { Sense::click() } else { Sense::hover() });
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = if lit {
        p.raised
    } else if allowed && resp.hovered() {
        p.raised.gamma_multiply(0.5)
    } else {
        p.bar
    };
    ui.painter().rect_filled(rect, radius, fill);
    let badge = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + 8.0, rect.center().y), Vec2::splat(16.0));
    let mut painter = ui.painter().clone();
    if !lit {
        painter.multiply_opacity(0.35);
    }
    widgets::paint_maturity_badge(&painter, p, name, badge, widgets::inner_radius(ui.visuals()));
    widgets::paint_row_text(ui.painter(), galley, badge.right() + 6.0, rect.center().y, ink);
    let resp = if allowed {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp.on_hover_text(aurora_i18n::tr!("search-rating-not-allowed"))
    };
    if allowed && resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// A digits-only field of the land filters, its name as its hint; true
/// when its value is committed (Enter, or the field left).
fn number_field(ui: &mut egui::Ui, label: &str, text: &mut String) -> bool {
    let field = ui
        .add(
            egui::TextEdit::singleline(text)
                .hint_text(label)
                .desired_width(90.0)
                .char_limit(9)
                .align(egui::Align2::LEFT_CENTER),
        )
        .on_hover_text(label);
    if field.changed() {
        text.retain(|ch| ch.is_ascii_digit());
    }
    field.lost_focus()
}

impl SearchUi {
    /// The request of a list for Aurora's field and filters. "All" keeps
    /// every category and the current and upcoming events: its line of
    /// filters has the ratings only.
    fn aurora_request(&self, tab: Tab, settings: &SearchSettings) -> Option<Request> {
        let text = self.aurora.text.trim().to_owned();
        let ratings = settings.aurora_ratings.ratings();
        let single = self.aurora.kind != Kind::All;
        Some(match tab {
            Tab::Web => return None,
            Tab::People => Request::People { text },
            Tab::Groups => Request::Groups {
                text,
                maturity: settings.group_maturity,
            },
            Tab::Places => Request::Places {
                text,
                ratings,
                category: if single { self.places_category } else { PLACE_CATEGORY_ANY },
            },
            Tab::Events => Request::Events {
                text,
                ratings,
                mode: if single && self.events_by_day {
                    EventsMode::Day(self.events_day)
                } else {
                    EventsMode::Current
                },
                category: if single { self.events_category } else { 0 },
            },
            Tab::Classifieds => Request::Classifieds {
                text,
                ratings,
                category: if single { self.classifieds_category } else { 0 },
            },
            Tab::Land => {
                let price = self.aurora.land_price.trim();
                let area = self.aurora.land_area.trim();
                Request::Land {
                    ratings,
                    filter: LandFilter {
                        kind: LandKind::from_setting(settings.land_type),
                        sort: self.land_sort,
                        ascending: self.land_ascending,
                        // an empty field sets no limit
                        limit_price: !price.is_empty(),
                        limit_area: !area.is_empty(),
                        price: price.parse().unwrap_or(0),
                        area: area.parse().unwrap_or(0),
                    },
                }
            }
        })
    }

    /// Searches the lists of the chip shown; with `stale_only`, only those
    /// last searched for something else (another text, other filters).
    fn aurora_search(&mut self, settings: &SearchSettings, stale_only: bool) -> Vec<Request> {
        let mut out = Vec::new();
        for tab in self.aurora.kind.tabs() {
            let Some(request) = self.aurora_request(tab, settings) else {
                continue;
            };
            if stale_only && self.aurora.searched.get(&tab) == Some(&request) {
                continue;
            }
            self.aurora.searched.insert(tab, request.clone());
            out.push(request);
        }
        if !out.is_empty() {
            self.aurora.selected = None;
        }
        out
    }

    /// The searches of Aurora's window for its chip, as they are (demo).
    pub fn aurora_requests(&mut self, settings: &SearchSettings) -> Vec<Request> {
        self.aurora_search(settings, false)
    }

    #[cfg(test)]
    pub(super) fn aurora_kind(&self) -> Kind {
        self.aurora.kind
    }

    /// The text the web page of the grid is asked with (none in the Legacy
    /// look, as its web tab).
    pub fn web_query(&self, classic: bool) -> &str {
        if classic { "" } else { &self.aurora.web_query }
    }

    /// A classified or an event opened from a link (`Search::show_one`):
    /// Aurora's window shows its list as searched, the row selected, and
    /// does not search it again for the text it holds.
    pub fn aurora_show_one(&mut self, tab: Tab, key: Key, settings: &SearchSettings) {
        if let Some(request) = self.aurora_request(tab, settings) {
            self.aurora.searched.insert(tab, request);
        }
        self.aurora.selected = Some((tab, key));
    }

    /// Whether Aurora's window searched this list for what it shows now.
    fn aurora_shows(&self, tab: Tab) -> bool {
        self.aurora.searched.contains_key(&tab)
    }

    /// The count of a chip, once its list answered: "100+" when the
    /// directory has a next page.
    fn chip_count(&self, search: &Search, kind: Kind) -> Option<String> {
        let tab = kind.tab()?;
        let list = search.list(tab)?;
        if !self.aurora_shows(tab) || list.status == Status::Searching {
            return None;
        }
        let n = list.rows.len();
        Some(if list.has_next {
            aurora_i18n::tr!("search-aurora-count-more", count = n as i64)
        } else {
            n.to_string()
        })
    }

    /// The row whose details are shown.
    fn aurora_selection(&self, search: &Search) -> Option<(Tab, Key)> {
        let has = |tab: Tab, key: Key| search.list(tab).is_some_and(|l| l.rows.iter().any(|r| r.key == key));
        match self.aurora.kind {
            Kind::All => self.aurora.selected.filter(|(t, k)| has(*t, *k)).or_else(|| {
                SECTIONS.iter().filter(|t| self.aurora_shows(**t)).find_map(|t| {
                    let list = search.list(*t)?;
                    let first = list.sorted().first()?.key;
                    Some((*t, list.selected.filter(|k| has(*t, *k)).unwrap_or(first)))
                })
            }),
            kind => {
                let tab = kind.tab()?;
                let list = search.list(tab)?;
                self.aurora_shows(tab).then_some(())?;
                list.selected.filter(|k| has(tab, *k)).map(|k| (tab, k))
            }
        }
    }

    /// The window's body in Aurora's look.
    pub(super) fn aurora_body(
        &mut self,
        ui: &mut egui::Ui,
        c: &mut Ctx,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        settings: &mut SearchSettings,
        web: &WebEnv,
        actions: &mut Vec<SearchAction>,
    ) {
        let p = c.p;
        let world = c.world;
        let access = world.agent_access();
        let mut find = false;
        // the field
        ui.horizontal(|ui| {
            let hint = if self.aurora.kind == Kind::Land {
                aurora_i18n::tr!("search-aurora-land-hint")
            } else {
                aurora_i18n::tr!("search-aurora-hint")
            };
            let clear = if self.aurora.text.is_empty() { 0.0 } else { 28.0 };
            let field = widgets::search_box(ui, p, &mut self.aurora.text, hint, ui.available_width() - clear);
            if std::mem::take(&mut self.focus) {
                field.request_focus();
            }
            find |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if clear > 0.0 && widgets::icon_button(ui, p, "x", aurora_i18n::tr!("search-aurora-clear"), true) {
                self.aurora.text.clear();
                self.focus = true;
            }
        });
        ui.add_space(6.0);
        // Counts and translated labels can exceed the minimum width. Wrap
        // whole chips, including Web, so every directory stays reachable.
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for kind in Kind::CHIPS {
                let count = self.chip_count(&world.search, kind);
                if widgets::pill_tab(ui, p, kind.icon(), kind.label(), count, false, self.aurora.kind == kind, true).clicked() {
                    picked = Some(kind);
                }
            }
            let web_chip = widgets::pill_tab(
                ui,
                p,
                Kind::Web.icon(),
                Kind::Web.label(),
                None,
                false,
                self.aurora.kind == Kind::Web,
                true,
            );
            if web_chip.on_hover_text(aurora_i18n::tr!("search-aurora-web-tip")).clicked() {
                picked = Some(Kind::Web);
            }
        });
        if let Some(kind) = picked.filter(|k| *k != self.aurora.kind) {
            self.aurora.kind = kind;
            self.aurora.selected = None;
            self.focus = kind != Kind::Web;
            if kind == Kind::Web {
                self.aurora.web_query = self.aurora.text.trim().to_owned();
            }
            actions.extend(self.aurora_search(settings, true).into_iter().map(SearchAction::Find));
        }
        if self.aurora.kind != Kind::Web {
            ui.add_space(6.0);
            find |= self.aurora_filters(ui, p, world, settings, access, actions);
        }
        if find && self.aurora.kind == Kind::Web {
            self.aurora.web_query = self.aurora.text.trim().to_owned();
        } else if find {
            actions.extend(self.aurora_search(settings, false).into_iter().map(SearchAction::Find));
        }
        ui.add_space(4.0);
        widgets::divider(ui, p);
        if self.aurora.kind == Kind::Web {
            ui.add_space(4.0);
            self.web_tab(ui, p, web);
            return;
        }
        // the results, then the details beside them
        let body = Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
        ui.allocate_rect(body, Sense::hover());
        let details_w = DETAILS_W.min(body.width() * 0.45);
        let left = Rect::from_min_max(body.min, Pos2::new(body.right() - details_w - 1.0, body.bottom()));
        let right = Rect::from_min_max(Pos2::new(left.right() + 1.0, body.top()), body.max);
        ui.painter().vline(left.right() + 0.5, body.y_range(), widgets::divider_stroke(p));
        let selection = self.aurora_selection(&world.search);
        let mut list_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("aurora-search-list", self.aurora.kind as usize))
                .max_rect(left.shrink2(Vec2::new(0.0, 2.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        self.aurora_list(&mut list_ui, c, settings, selection, actions);
        let mut detail_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("aurora-search-details")
                .max_rect(right.shrink2(Vec2::new(12.0, 10.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        match selection {
            Some((tab, key)) => {
                let row = world.search.list(tab).and_then(|l| l.rows.iter().find(|r| r.key == key));
                aurora_details::show(&mut detail_ui, c, pics, &mut self.wanted_pics, tab, key, row, actions);
            }
            None => {
                detail_ui.add_space(40.0);
                detail_ui.vertical_centered(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("search-aurora-select")).size(12.5).color(p.muted));
                });
            }
        }
    }

    /// The line of filters of the chip shown, and the number of results at
    /// its right end. True when a search is asked.
    fn aurora_filters(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &crate::world::World,
        settings: &mut SearchSettings,
        access: AgentAccess,
        actions: &mut Vec<SearchAction>,
    ) -> bool {
        let kind = self.aurora.kind;
        let mut find = false;
        let mut saved = false;
        let count = self.results_text(&world.search);
        let count_font = FontId::proportional(12.0);
        let count_w = ui.painter().layout_no_wrap(count.clone(), count_font, p.muted).size().x;
        ui.horizontal(|ui| {
            let room = Vec2::new((ui.available_width() - count_w - 12.0).max(80.0), 28.0);
            ui.allocate_ui_with_layout(room, egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                if matches!(kind, Kind::All | Kind::Places | Kind::Events | Kind::Classifieds | Kind::Land) {
                    let boxes = &mut settings.aurora_ratings;
                    for (on, name, allowed) in [
                        (&mut boxes.pg, aurora_i18n::tr!("common-general"), true),
                        (&mut boxes.mature, aurora_i18n::tr!("common-moderate"), access.can_access_mature()),
                        (&mut boxes.adult, aurora_i18n::tr!("common-adult"), access.can_access_adult()),
                    ] {
                        if rating_toggle(ui, p, on, name, allowed) {
                            saved = true;
                            find = true;
                        }
                    }
                }
                match kind {
                    Kind::People => {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("search-aurora-people-hint"))
                                .size(12.0)
                                .color(p.muted),
                        );
                    }
                    Kind::Groups => {
                        let levels = [
                            (SIM_ACCESS_PG, aurora_i18n::tr!("search-aurora-groups-general")),
                            (SIM_ACCESS_MATURE, aurora_i18n::tr!("search-aurora-groups-moderate")),
                            (SIM_ACCESS_ADULT, aurora_i18n::tr!("search-aurora-groups-adult")),
                        ];
                        if combo(ui, "aurora-search-group-maturity", &mut settings.group_maturity, &levels, 230.0) {
                            saved = true;
                            find = true;
                        }
                    }
                    Kind::Places => {
                        ui.add_space(6.0);
                        find |= combo(
                            ui,
                            "aurora-search-place-category",
                            &mut self.places_category,
                            &place_categories(),
                            180.0,
                        );
                    }
                    Kind::Classifieds => {
                        ui.add_space(6.0);
                        let cats = world.classified_categories();
                        let mut items = vec![(0, aurora_i18n::tr!("search-all-categories"))];
                        items.extend(cats.iter().map(|(id, name)| (*id, name.as_str())));
                        find |= combo(
                            ui,
                            "aurora-search-classified-category",
                            &mut self.classifieds_category,
                            &items,
                            180.0,
                        );
                    }
                    Kind::Events => {
                        ui.add_space(6.0);
                        find |= combo(
                            ui,
                            "aurora-search-event-category",
                            &mut self.events_category,
                            &event_categories(),
                            180.0,
                        );
                        let mut mode = usize::from(self.events_by_day);
                        let options = [
                            (aurora_i18n::tr!("search-events-current"), false),
                            (aurora_i18n::tr!("search-aurora-by-date"), false),
                        ];
                        if widgets::segmented(ui, p, &mut mode, &options) {
                            self.events_by_day = mode == 1;
                            find = true;
                        }
                        if self.events_by_day {
                            if widgets::icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("search-events-previous-day"), true) {
                                self.events_day -= 1;
                                find = true;
                            }
                            let (_, month, day, weekday, ..) = crate::ui::land::slt_parts(unix_now() + self.events_day as i64 * 86_400);
                            let date = aurora_i18n::tr!("search-events-date", month = month as i64, day = day);
                            let label = format!("{} {date}", crate::ui::land::weekdays()[weekday]);
                            ui.label(RichText::new(label).size(12.0).color(p.ink));
                            if widgets::icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("search-events-next-day"), true) {
                                self.events_day += 1;
                                find = true;
                            }
                            if self.events_day != 0 && widgets::flat_button(ui, p, aurora_i18n::tr!("search-events-today")).clicked() {
                                self.events_day = 0;
                                find = true;
                            }
                        }
                    }
                    Kind::Land => {
                        ui.add_space(6.0);
                        let kinds = [
                            (LandKind::All, aurora_i18n::tr!("search-aurora-land-all")),
                            (LandKind::Auction, aurora_i18n::tr!("search-land-auction")),
                            (LandKind::Mainland, aurora_i18n::tr!("search-land-mainland")),
                            (LandKind::Estate, aurora_i18n::tr!("search-land-estate")),
                        ];
                        let mut land_kind = LandKind::from_setting(settings.land_type);
                        if combo(ui, "aurora-search-land-kind", &mut land_kind, &kinds, 140.0) {
                            settings.land_type = land_kind.setting();
                            saved = true;
                            find = true;
                        }
                        find |= number_field(ui, aurora_i18n::tr!("search-land-max-price"), &mut self.aurora.land_price);
                        find |= number_field(ui, aurora_i18n::tr!("search-land-min-area"), &mut self.aurora.land_area);
                        let sorts = [
                            (LandSort::Name, aurora_i18n::tr!("search-aurora-sort-name")),
                            (LandSort::Price, aurora_i18n::tr!("search-aurora-sort-price")),
                            (LandSort::PerMeter, aurora_i18n::tr!("search-aurora-sort-per-meter")),
                            (LandSort::Area, aurora_i18n::tr!("search-aurora-sort-area")),
                        ];
                        find |= combo(ui, "aurora-search-land-sort", &mut self.land_sort, &sorts, 130.0);
                        if widgets::icon_toggle(ui, p, "arrows-sort", aurora_i18n::tr!("search-land-ascending"), self.land_ascending) {
                            self.land_ascending = !self.land_ascending;
                            find = true;
                        }
                    }
                    Kind::All | Kind::Web => {}
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(count).size(12.0).color(p.muted));
            });
        });
        if saved {
            actions.push(SearchAction::SettingsChanged);
        }
        find
    }

    /// "6 results", "> 100 results" for the chip shown, once answered.
    fn results_text(&self, search: &Search) -> String {
        match self.aurora.kind {
            Kind::All => {
                let lists: Vec<_> = SECTIONS
                    .iter()
                    .filter(|t| self.aurora_shows(**t))
                    .filter_map(|t| search.list(*t))
                    .collect();
                let n: usize = lists.iter().map(|l| l.rows.len()).sum();
                results(n, lists.iter().any(|l| l.has_next))
            }
            kind => match kind.tab().and_then(|t| search.list(t)) {
                Some(list) if self.aurora_shows(list.tab) => results(list.rows.len(), list.has_next),
                _ => String::new(),
            },
        }
    }

    /// What "All" says without a row: nothing searched yet, a search going
    /// on, or the first reason a list gave.
    fn all_status(&self, search: &Search) -> String {
        let statuses: Vec<&Status> = SECTIONS
            .iter()
            .filter(|t| self.aurora_shows(**t))
            .filter_map(|t| search.list(*t))
            .map(|l| &l.status)
            .collect();
        if statuses.is_empty() {
            return aurora_i18n::tr!("search-aurora-start").to_owned();
        }
        if let Some(s) = statuses.iter().find(|s| matches!(s, Status::Searching)) {
            return status_text(s);
        }
        let reason = statuses
            .iter()
            .find(|s| !matches!(s, Status::Found | Status::Idle | Status::NotFound(_)))
            .copied()
            .cloned()
            .unwrap_or_else(|| Status::NotFound(self.aurora.text.trim().to_owned()));
        status_text(&reason)
    }

    /// The results: the sections of "All", or the list of one type with its
    /// pages.
    fn aurora_list(
        &mut self,
        ui: &mut egui::Ui,
        c: &mut Ctx,
        settings: &SearchSettings,
        selection: Option<(Tab, Key)>,
        actions: &mut Vec<SearchAction>,
    ) {
        let p = c.p;
        let world = c.world;
        let kind = self.aurora.kind;
        let mut asks = 0;
        let mut goto = None;
        egui::ScrollArea::vertical()
            .id_salt(("aurora-search-rows", kind as usize))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let note = |ui: &mut egui::Ui, text: String| {
                    ui.add_space(16.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(text).size(12.5).color(p.muted));
                    });
                };
                if kind == Kind::All {
                    let mut any = false;
                    for tab in SECTIONS {
                        let Some(list) = world.search.list(tab).filter(|_| self.aurora_shows(tab)) else {
                            continue;
                        };
                        let rows = list.sorted();
                        if rows.is_empty() {
                            continue;
                        }
                        any = true;
                        let total = if list.has_next {
                            aurora_i18n::tr!("search-aurora-count-more", count = rows.len() as i64)
                        } else {
                            rows.len().to_string()
                        };
                        if section_head(ui, p, Kind::of(tab), &total, rows.len() > SECTION_ROWS) {
                            goto = Some(Kind::of(tab));
                        }
                        for row in rows.iter().take(SECTION_ROWS) {
                            self.aurora_row(ui, c, tab, row, selection, &mut asks, actions);
                        }
                    }
                    if !any {
                        note(ui, self.all_status(&world.search));
                    }
                    return;
                }
                let Some(tab) = kind.tab() else {
                    return;
                };
                let Some(list) = world.search.list(tab) else {
                    return;
                };
                if !self.aurora_shows(tab) {
                    note(ui, aurora_i18n::tr!("search-aurora-start").to_owned());
                    return;
                }
                let rows = list.sorted();
                if rows.is_empty() {
                    note(ui, status_text(&list.status));
                    return;
                }
                ui.add_space(3.0);
                for row in &rows {
                    self.aurora_row(ui, c, tab, row, selection, &mut asks, actions);
                }
                // the directory's pages of a hundred rows
                if list.has_back() || list.has_next {
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        let size = Vec2::new(0.0, 26.0);
                        if list.has_next {
                            let next = aurora_i18n::tr!("search-aurora-next-page");
                            if widgets::action_button(ui, p, "caret-right-filled", next, ActionKind::Flat, size).clicked() {
                                actions.push(SearchAction::Page(tab, true));
                            }
                        }
                        if list.has_back() {
                            let back = aurora_i18n::tr!("search-aurora-previous-page");
                            if widgets::action_button(ui, p, "caret-left-filled", back, ActionKind::Flat, size).clicked() {
                                actions.push(SearchAction::Page(tab, false));
                            }
                        }
                        let page = aurora_i18n::tr!("search-page", page = list.page() as i64);
                        ui.label(RichText::new(page).size(11.5).color(p.muted_dim));
                    });
                    ui.add_space(8.0);
                }
            });
        if let Some(kind) = goto {
            self.aurora.kind = kind;
            self.aurora.selected = None;
            actions.extend(self.aurora_search(settings, true).into_iter().map(SearchAction::Find));
        }
    }

    /// One result row; a click selects it (and asks for its details), a
    /// resident's double click opens the profile, a right click its menu.
    #[allow(clippy::too_many_arguments)]
    fn aurora_row(
        &mut self,
        ui: &mut egui::Ui,
        c: &mut Ctx,
        tab: Tab,
        row: &Row,
        selection: Option<(Tab, Key)>,
        asks: &mut usize,
        actions: &mut Vec<SearchAction>,
    ) {
        let p = c.p;
        let world = c.world;
        let size = RowSize::SEARCH;
        let next = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), size.height));
        if !ui.is_rect_visible(next) {
            ui.allocate_exact_size(next.size(), Sense::hover());
            return;
        }
        let selected = selection == Some((tab, row.key));
        let semibold = crate::ui::fonts::semibold(ui.ctx());
        let mut ask = |asked: &mut HashSet<Uuid>, id: Uuid, action: SearchAction| {
            if *asks < ASKS_PER_FRAME && asked.insert(id) {
                *asks += 1;
                actions.push(action);
            }
        };
        let resp = match (tab, row.key) {
            (Tab::Places | Tab::Land, Key::Id(id)) => {
                let parcel = world.profiles.parcels.get(&id);
                if parcel.is_none() {
                    ask(&mut self.aurora.asked, id, SearchAction::WantParcel(id));
                }
                let snapshot = parcel.map(|d| d.snapshot).filter(|s| !s.is_nil());
                if let Some(s) = snapshot {
                    c.wanted_images.insert(s);
                }
                let tex = snapshot.and_then(|s| c.images.get(&s));
                let subtitle = if tab == Tab::Land {
                    land_subtitle(row)
                } else {
                    parcel.filter(|d| !d.sim_name.is_empty()).map_or_else(
                        || aurora_i18n::tr!("common-loading-2").to_owned(),
                        |d| crate::ui::place_details::region_name_pos(&d.sim_name, glam::Vec3::ZERO, d.global),
                    )
                };
                let card = place_card::Row {
                    thumb: Thumb::Picture(tex),
                    title: &row.name,
                    subtitle: &subtitle,
                    rating: parcel.map(|d| crate::ui::place_details::maturity(d.flags)),
                    selected,
                };
                place_card::row(ui, p, size, &card, |ui| {
                    if tab == Tab::Land {
                        let price = match row.values.get(1) {
                            Some(Value::Int(n)) if row.icon != RowIcon::Auction => linden(*n),
                            _ => String::new(),
                        };
                        if !price.is_empty() {
                            meta(ui, p, None, &price);
                        }
                        sale_pill(ui, p, row.icon == RowIcon::Auction);
                    } else {
                        if let Some(Value::Int(traffic)) = row.values.first() {
                            meta(ui, p, Some("walk"), &aurora_i18n::integer(*traffic));
                        }
                        if matches!(row.icon, RowIcon::ForSale | RowIcon::Auction) {
                            sale_pill(ui, p, row.icon == RowIcon::Auction);
                        }
                    }
                })
            }
            (Tab::Classifieds, Key::Id(id)) => {
                let info = world.profiles.classifieds.get(&id);
                if info.is_none() {
                    ask(&mut self.aurora.asked, id, SearchAction::WantClassified(id));
                }
                let snapshot = info.map(|i| i.snapshot).filter(|s| !s.is_nil());
                if let Some(s) = snapshot {
                    c.wanted_images.insert(s);
                }
                let tex = snapshot.and_then(|s| c.images.get(&s));
                let subtitle = info.map_or_else(
                    || aurora_i18n::tr!("common-loading-2").to_owned(),
                    |i| aurora_i18n::tr!("search-aurora-published", date = crate::world::profiles::slt_date(i.creation_date)),
                );
                let mature = info.map(|i| {
                    i.flags & (aurora_net::profile::classified_flags::MATURE | aurora_net::profile::classified_flags::QUERY_INC_MATURE) != 0
                });
                let card = place_card::Row {
                    thumb: Thumb::Picture(tex),
                    title: &row.name,
                    subtitle: &subtitle,
                    rating: mature.map(|m| {
                        if m {
                            aurora_i18n::tr!("common-moderate")
                        } else {
                            aurora_i18n::tr!("common-general")
                        }
                    }),
                    selected,
                };
                place_card::row(ui, p, size, &card, |ui| {
                    if let Some(Value::Int(price)) = row.values.first() {
                        meta(ui, p, None, &linden(*price));
                    }
                })
            }
            (Tab::Events, _) => {
                let unix = match row.values.first() {
                    Some(Value::Dated { unix, .. }) => *unix as i64,
                    _ => 0,
                };
                let now = unix_now();
                let (_, month, day, ..) = crate::ui::land::slt_parts(unix);
                let mon = crate::ui::land::months()[(month.max(1) - 1).min(11)]
                    .trim_end_matches('.')
                    .to_uppercase();
                let day = day.to_string();
                let (violet, ink) = (p.violet_light, p.ink);
                let font = semibold.clone();
                let paint = move |painter: &egui::Painter, rect: Rect| {
                    let m = painter.layout_no_wrap(mon.clone(), FontId::new(9.5, font.clone()), violet);
                    widgets::paint_ink_centered(painter, m, rect.center() - Vec2::new(0.0, 8.0), violet);
                    let d = painter.layout_no_wrap(day.clone(), FontId::new(16.0, font.clone()), ink);
                    widgets::paint_ink_centered(painter, d, rect.center() + Vec2::new(0.0, 6.0), ink);
                };
                let subtitle = if unix > 0 { event_when(unix, now) } else { String::new() };
                let card = place_card::Row {
                    thumb: Thumb::Paint(&paint),
                    title: &row.name,
                    subtitle: &subtitle,
                    rating: Some(event_rating(row.icon)),
                    selected,
                };
                place_card::row(ui, p, size, &card, |ui| {
                    if unix > 0 && unix <= now {
                        started_pill(ui, p);
                    }
                })
            }
            (Tab::Groups, Key::Id(id)) => {
                let profile = world.search.groups.get(&id);
                if profile.is_none() {
                    ask(&mut self.aurora.asked, id, SearchAction::RefreshGroup(id));
                }
                let insignia = profile
                    .map(|g| g.insignia)
                    .filter(|i| !i.is_nil())
                    .or_else(|| world.groups.group(&id).map(|g| g.insignia).filter(|i| !i.is_nil()));
                if let Some(i) = insignia {
                    c.wanted_images.insert(i);
                }
                let texture = insignia.and_then(|i| c.images.get(&i));
                let initials = widgets::initials(&row.name);
                let (pale, font) = (p.violet_pale, semibold.clone());
                let paint = move |painter: &egui::Painter, rect: Rect| {
                    let g = painter.layout_no_wrap(initials.clone(), FontId::new(13.0, font.clone()), pale);
                    widgets::paint_ink_centered(painter, g, rect.center(), pale);
                };
                let count = world.group_member_count(id).or_else(|| match row.values.first() {
                    Some(Value::Int(n)) => Some(*n),
                    _ => None,
                });
                let members = count.map_or_else(String::new, |n| aurora_i18n::tr!("search-group-members", count = n));
                let card = place_card::Row {
                    thumb: texture.map_or(Thumb::Paint(&paint), |t| Thumb::Picture(Some(t))),
                    title: &row.name,
                    subtitle: &members,
                    rating: None,
                    selected,
                };
                let member = world.groups.is_member(&id);
                place_card::row(ui, p, size, &card, |ui| {
                    if member {
                        pill(ui, aurora_i18n::tr!("search-aurora-member"), p.violet_pale, p.raised);
                    }
                })
            }
            (Tab::People, _) => {
                let dim = p.muted_dim;
                let paint = move |painter: &egui::Painter, rect: Rect| {
                    if let Some(t) = crate::ui::icons::global("user-filled") {
                        let at = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
                        painter.image(t.id(), at, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), dim);
                    }
                };
                let name = username(&row.name);
                let card = place_card::Row {
                    thumb: Thumb::Paint(&paint),
                    title: &row.name,
                    subtitle: &name,
                    rating: None,
                    selected,
                };
                place_card::row(ui, p, size, &card, |_| {})
            }
            _ => return,
        };
        let resp = super::place_drag_source(resp, c, tab, row);
        if resp.clicked() && !selected {
            self.aurora.selected = Some((tab, row.key));
            actions.push(SearchAction::Select(tab, row.key));
        }
        match (tab, row.key) {
            (Tab::People, Key::Id(id)) => {
                // LLAvatarActions::showProfile on a double click;
                // gFSAvatarSearchMenu on a right click
                if resp.double_clicked() {
                    crate::ui::profile::request_open(ui.ctx(), id);
                }
                crate::ui::menu::context_menu(&resp, p, |ui| {
                    crate::ui::context::avatar_list_menu(ui, p, world, id, crate::ui::context::AvatarList::Name)
                });
            }
            (Tab::Groups, Key::Id(id)) if resp.double_clicked() => crate::ui::group_profile::request_open(ui.ctx(), id),
            _ => {}
        }
    }
}

/// "6 results", "More than 100 results" when the directory has a next
/// page; nothing before the first row.
fn results(n: usize, more: bool) -> String {
    if n == 0 {
        String::new()
    } else if more {
        aurora_i18n::tr!("search-aurora-results-more", count = n as i64)
    } else {
        aurora_i18n::tr!("search-aurora-results", count = n as i64)
    }
}

/// "Estate · 4096 m² · 3.05 L$/m²": what a land row knows without asking.
fn land_subtitle(row: &Row) -> String {
    let mut parts = Vec::new();
    if let Some(Value::Text(kind)) = row.values.first() {
        parts.push(kind.clone());
    }
    if let Some(Value::Int(area)) = row.values.get(2) {
        parts.push(aurora_i18n::tr!("search-aurora-area", area = aurora_i18n::integer(*area)));
    }
    if row.icon != RowIcon::Auction
        && let Some(Value::Real(per_meter)) = row.values.get(3)
    {
        parts.push(aurora_i18n::tr!(
            "search-aurora-per-meter",
            value = aurora_i18n::number(*per_meter, 2)
        ));
    }
    parts.join(" · ")
}

/// A section of "All": its icon, its name in small capitals and its count,
/// "See all" at the right when it has more rows than it shows. True when
/// "See all" is clicked.
fn section_head(ui: &mut egui::Ui, p: &Palette, kind: Kind, count: &str, more: bool) -> bool {
    let mut clicked = false;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        widgets::glyph(ui, kind.icon(), p.muted, 13.0);
        let caps = RichText::new(kind.label().to_uppercase())
            .size(11.0)
            .color(p.muted)
            .family(crate::ui::fonts::semibold(ui.ctx()));
        ui.label(caps);
        ui.label(RichText::new(count).size(11.0).color(p.muted_dim));
        if more {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                let see = ui
                    .add(
                        egui::Label::new(
                            RichText::new(aurora_i18n::tr!("search-aurora-see-all"))
                                .size(12.0)
                                .color(p.violet_light),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                clicked = see.clicked();
            });
        }
    });
    ui.add_space(2.0);
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_searches_the_five_texts_and_keeps_their_categories_open() {
        let mut ui = SearchUi::default();
        let settings = SearchSettings::default();
        ui.aurora.text = " lune ".into();
        ui.places_category = 9;
        ui.events_by_day = true;
        let requests = ui.aurora_requests(&settings);
        let tabs: Vec<Tab> = requests.iter().map(Request::tab).collect();
        assert_eq!(tabs, SECTIONS.to_vec(), "land sales have no text");
        assert!(requests.contains(&Request::Places {
            text: "lune".into(),
            ratings: settings.aurora_ratings.ratings(),
            category: PLACE_CATEGORY_ANY,
        }));
        assert!(
            requests.iter().any(|r| matches!(
                r,
                Request::Events {
                    mode: EventsMode::Current,
                    category: 0,
                    ..
                }
            )),
            "All shows the current and upcoming events"
        );
        // a chip shows its list without searching again, until a filter of
        // its own changes the request
        ui.aurora.kind = Kind::Places;
        let again = ui.aurora_search(&settings, true);
        assert_eq!(again.len(), 1, "the category of the chip makes another request");
        assert!(ui.aurora_search(&settings, true).is_empty(), "nothing changed since");
        ui.aurora.kind = Kind::People;
        assert!(ui.aurora_search(&settings, true).is_empty(), "people were searched by All");
        ui.aurora.text = "lunes".into();
        assert_eq!(ui.aurora_search(&settings, true).len(), 1);
    }

    #[test]
    fn land_limits_come_from_the_fields() {
        let mut ui = SearchUi::default();
        let mut settings = SearchSettings::default();
        ui.aurora.kind = Kind::Land;
        let Some(Request::Land { filter, .. }) = ui.aurora_request(Tab::Land, &settings) else {
            panic!("land");
        };
        assert!(!filter.limit_price && !filter.limit_area, "empty fields set no limit");
        ui.aurora.land_price = "5000".into();
        ui.aurora.land_area = "512".into();
        settings.land_type = LandKind::Estate.setting();
        let Some(Request::Land { filter, .. }) = ui.aurora_request(Tab::Land, &settings) else {
            panic!("land");
        };
        assert_eq!(
            (filter.limit_price, filter.price, filter.limit_area, filter.area, filter.kind),
            (true, 5000, true, 512, LandKind::Estate)
        );
        assert_eq!(ui.aurora_request(Tab::Web, &settings), None);
    }

    #[test]
    fn rows_say_what_the_directory_gave() {
        assert_eq!(username("Lune Azurine"), "lune.azurine");
        assert_eq!(username("lunette"), "lunette");
        let land = Row {
            key: Key::Id(Uuid::nil()),
            icon: RowIcon::ForSale,
            name: "Rivage".into(),
            values: vec![
                Value::Text("Estate".into()),
                Value::Int(12500),
                Value::Int(4096),
                Value::Real(3.0517),
            ],
        };
        let sub = land_subtitle(&land);
        assert!(sub.starts_with("Estate · ") && sub.contains("3"), "{sub}");
        let auction = Row {
            icon: RowIcon::Auction,
            values: vec![Value::None, Value::Int(0), Value::Int(2048), Value::Real(1.0)],
            ..land
        };
        assert_eq!(
            land_subtitle(&auction),
            aurora_i18n::tr!("search-aurora-area", area = aurora_i18n::integer(2048)),
            "an auction has no price per metre"
        );
        // 10:00 SLT on a day of the demo events: an hour later is the same day
        let base = 1_791_475_200;
        let (.., h, min, sec) = crate::ui::land::slt_parts(base);
        let now = base - h * 3600 - min * 60 - sec + 10 * 3600;
        assert!(event_when(now + 3600, now).starts_with(aurora_i18n::tr!("search-events-today")));
        assert!(event_when(now + 86_400, now).starts_with(aurora_i18n::tr!("search-aurora-tomorrow")));
        assert_eq!(event_rating(RowIcon::EventAdult), aurora_i18n::tr!("common-adult"));
        assert_eq!(Kind::of(Tab::Classifieds).tab(), Some(Tab::Classifieds));
        assert_eq!(Kind::All.tabs(), SECTIONS.to_vec());
        assert!(Kind::Web.tabs().is_empty());
    }
}
