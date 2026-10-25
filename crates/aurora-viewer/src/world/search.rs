//! "Search": the state of Firestorm's legacy search floater
//! (FSFloaterSearch and its FSPanelSearch* panels,
//! indra/newview/fsfloatersearch.cpp, originally LGPL 2.1). One result list
//! per tab with its query, page, status and selection; the replies are
//! applied the way each `processSearchReply` does (a reply of an older query
//! is ignored, the row past the page size only says that a next page exists),
//! and selecting a row asks for its details like
//! FSFloaterSearch::onSelectedItem / onSelectedEvent.
//!
//! The queries and replies themselves are in `aurora-net/src/search.rs`.

use aurora_net::NetCommand;
use aurora_net::search::{
    self, DirQuery, DirReply, DirRows, EventInfo, EventsMode, GroupProfile, LandFilter, RESULT_PAGE_SIZE, Ratings, event_flags, status,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a key typed in the people tab may stay without a name before it
/// is reported as not found (Firestorm waits for the name cache's answer,
/// "(???).(???)" for an unknown key).
pub const KEY_LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The tabs, in the order of "ls_tabs" in floater_fs_search.xml: the web
/// search first, then people, groups, places, land sales, events and
/// classifieds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Web,
    People,
    Groups,
    Places,
    Land,
    Events,
    Classifieds,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Web,
        Tab::People,
        Tab::Groups,
        Tab::Places,
        Tab::Land,
        Tab::Events,
        Tab::Classifieds,
    ];

    /// Place in the tab strip, which is also what FSLastSearchTab holds.
    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// selectTab(FSLastSearchTab), the first tab when it fails.
    pub fn from_index(i: usize) -> Tab {
        Tab::ALL.get(i).copied().unwrap_or(Tab::Web)
    }

    /// Place of a directory tab among the result lists; None for the web
    /// tab, which has no list.
    pub fn list_index(self) -> Option<usize> {
        self.index().checked_sub(1)
    }

    /// The names of `AURORA_DEMO_SEARCH`.
    pub fn from_name(name: &str) -> Option<Tab> {
        Some(match name.trim().to_lowercase().as_str() {
            "people" => Tab::People,
            "groups" => Tab::Groups,
            "places" => Tab::Places,
            "land" => Tab::Land,
            "events" => Tab::Events,
            "classifieds" => Tab::Classifieds,
            "web" => Tab::Web,
            _ => return None,
        })
    }
}

/// The G / M / A boxes of a tab, as saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RatingBoxes {
    pub pg: bool,
    pub mature: bool,
    pub adult: bool,
}

impl Default for RatingBoxes {
    /// ShowPG* 1, ShowMature* 0, ShowAdult* 0.
    fn default() -> Self {
        RatingBoxes {
            pg: true,
            mature: false,
            adult: false,
        }
    }
}

impl RatingBoxes {
    pub fn ratings(self) -> Ratings {
        Ratings {
            pg: self.pg,
            mature: self.mature,
            adult: self.adult,
        }
    }
}

/// The saved choices of the search window, with Firestorm's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchSettings {
    /// FSLastSearchTab: the tab the window opens on (0, the web search, the
    /// first time).
    pub last_tab: u8,
    /// FSSearchGroupMaturity: 13 general, 21 + moderate, 42 + adult.
    pub group_maturity: u8,
    /// ShowPGSims / ShowMatureSims / ShowAdultSims.
    pub places: RatingBoxes,
    /// ShowPGLand...
    pub land: RatingBoxes,
    /// ShowPGEvents...
    pub events: RatingBoxes,
    /// ShowPGClassifieds...
    pub classifieds: RatingBoxes,
    /// FindLandPrice / FindLandArea.
    pub land_limit_price: bool,
    pub land_limit_area: bool,
    /// FindLandType: 0 all, 1 auction, 2 mainland, 3 estate.
    pub land_type: u8,
    /// FSLegacySearchActionOnTeleport: 0 nothing, 1 close the window,
    /// 2 minimize it.
    pub teleport_action: u8,
    /// The G / M / A toggles of Aurora's window, one set for every type
    /// (the Legacy Viewer's defaults: General only).
    pub aurora_ratings: RatingBoxes,
}

impl Default for SearchSettings {
    fn default() -> Self {
        SearchSettings {
            last_tab: 0,
            group_maturity: search::SIM_ACCESS_PG,
            places: RatingBoxes::default(),
            land: RatingBoxes::default(),
            events: RatingBoxes::default(),
            classifieds: RatingBoxes::default(),
            land_limit_price: true,
            land_limit_area: false,
            land_type: 0,
            teleport_action: 1,
            aurora_ratings: RatingBoxes::default(),
        }
    }
}

/// What a row is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// An avatar, a group, a parcel or a classified.
    Id(Uuid),
    Event(u32),
}

/// The icon of a row (Firestorm's first column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIcon {
    Person,
    Group,
    Place,
    ForSale,
    Auction,
    Classified,
    EventGeneral,
    EventModerate,
    EventAdult,
}

/// A cell after the name.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    None,
    Int(i64),
    Real(f64),
    Text(String),
    /// A date written by the directory, sorted by its Unix time.
    Dated {
        text: String,
        unix: u32,
    },
}

impl Value {
    fn order(&self, other: &Value) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let number = |v: &Value| match v {
            Value::Int(i) => Some(*i as f64),
            Value::Real(r) => Some(*r),
            Value::Dated { unix, .. } => Some(*unix as f64),
            _ => None,
        };
        match (number(self), number(other)) {
            (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => match (self, other) {
                (Value::Text(a), Value::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
                (Value::Text(_), _) => Ordering::Greater,
                (_, Value::Text(_)) => Ordering::Less,
                _ => Ordering::Equal,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: Key,
    pub icon: RowIcon,
    pub name: String,
    /// The columns after the name, per tab: members (groups); traffic
    /// (places); type, price, area, L$/m² (land); time (events); price
    /// (classifieds).
    pub values: Vec<Value>,
}

/// What the list says in place of rows (the scroll list's comment text).
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// "no_results": nothing searched yet.
    Idle,
    /// "searching".
    Searching,
    /// Rows are shown.
    Found,
    /// "not_found" with the text searched.
    NotFound(String),
    /// "search_short".
    TooShort,
    /// "search_banned".
    Banned,
    /// "search_disabled".
    Disabled,
    /// STATUS_SEARCH_EVENTS_NODATEOFFSET / NOCATEGORY / NOQUERY.
    NoDateOffset,
    NoCategory,
    NoQuery,
    /// NoContentToSearch: no rating box is ticked.
    NoContent,
}

/// The result list of one tab.
#[derive(Debug, Clone)]
pub struct List {
    pub tab: Tab,
    /// The query of the current page (sent again for another page).
    query: Option<DirQuery>,
    /// The text typed, for "not_found".
    pub text: String,
    /// mQueryID: replies of any other query are ignored.
    pub query_id: Uuid,
    /// mStartSearch: first row of the page.
    pub start: i32,
    /// mResultsReceived: rows received for this page, over all its packets.
    received: usize,
    /// mNumResultsReturned: reply packets of this page.
    replies: u32,
    pub rows: Vec<Row>,
    pub status: Status,
    /// "…_next" enabled: the directory sent more than a page.
    pub has_next: bool,
    pub selected: Option<Key>,
    /// Column sorted by a click on its header (0: name) and descending;
    /// None keeps the directory's order.
    pub sort: Option<(usize, bool)>,
    /// The filters the viewer applies again to the replies of this query.
    ratings: Ratings,
    land: LandFilter,
    /// A key typed in the people tab, waiting for its name.
    key_lookup: Option<(Uuid, Instant)>,
}

impl List {
    fn new(tab: Tab) -> List {
        List {
            tab,
            query: None,
            text: String::new(),
            query_id: Uuid::nil(),
            start: 0,
            received: 0,
            replies: 0,
            rows: Vec::new(),
            status: Status::Idle,
            has_next: false,
            selected: None,
            // panel_fs_search_legacy_events.xml: sort_column "time", ascending
            sort: (tab == Tab::Events).then_some((1, false)),
            ratings: Ratings::default(),
            land: LandFilter::default(),
            key_lookup: None,
        }
    }

    /// "…_back" enabled.
    pub fn has_back(&self) -> bool {
        self.start > 0
    }

    /// The page shown, from 1.
    pub fn page(&self) -> usize {
        self.start.max(0) as usize / RESULT_PAGE_SIZE + 1
    }

    /// The rows in display order.
    pub fn sorted(&self) -> Vec<&Row> {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        if let Some((column, descending)) = self.sort {
            rows.sort_by(|a, b| {
                let order = match column {
                    0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    c => match (a.values.get(c - 1), b.values.get(c - 1)) {
                        (Some(x), Some(y)) => x.order(y),
                        _ => std::cmp::Ordering::Equal,
                    },
                };
                if descending { order.reverse() } else { order }
            });
        }
        rows
    }

    /// A click on a column header: sort by it, then reverse.
    pub fn sort_by(&mut self, column: usize) {
        self.sort = match self.sort {
            Some((c, descending)) if c == column => Some((column, !descending)),
            _ => Some((column, false)),
        };
    }

    /// `find()` after the query was built: a new query id, the list emptied
    /// and "searching".
    fn begin(&mut self, query: DirQuery) -> NetCommand {
        self.query_id = Uuid::new_v4();
        self.start = self.start.max(0);
        self.received = 0;
        self.replies = 0;
        self.rows.clear();
        self.selected = None;
        self.has_next = false;
        self.key_lookup = None;
        self.status = Status::Searching;
        self.query = Some(query.clone());
        NetCommand::DirQuery {
            id: self.query_id,
            start: self.start,
            query,
        }
    }

    /// A search that is not sent: the list says why.
    fn refuse(&mut self, error: search::QueryError) {
        self.rows.clear();
        self.selected = None;
        self.has_next = false;
        self.key_lookup = None;
        self.query_id = Uuid::nil();
        self.status = match error {
            search::QueryError::TooShort => Status::TooShort,
            search::QueryError::NoContent => Status::NoContent,
        };
    }

    /// processSearchReply. Returns true when rows were added.
    fn apply(&mut self, reply: &DirReply, land_types: &HashMap<String, String>) -> bool {
        // "floater is closed or these are not results from our last request"
        if self.query_id.is_nil() || reply.query_id != self.query_id {
            return false;
        }
        // the first results clear the "Searching" label
        if self.replies == 0 {
            self.rows.clear();
        }
        self.replies += 1;
        if let Some(s) = reply.status
            && let Some(st) = self.status_of(s)
        {
            self.status = st;
            return false;
        }
        let new_rows = reply.rows.len();
        if new_rows == 0 && self.received == 0 {
            self.status = Status::NotFound(self.text.clone());
        }
        self.received += new_rows;
        // showNextButton: past a page, the extra rows only announce the next
        self.has_next = self.received > RESULT_PAGE_SIZE;
        let keep = if self.has_next && self.tab != Tab::Land {
            new_rows.saturating_sub(self.received - RESULT_PAGE_SIZE)
        } else {
            // land: auctions "don't count towards the page limit", every row
            // is shown
            new_rows
        };
        let before = self.rows.len();
        let mut null_row = false;
        let mut push = |rows: &mut Vec<Row>, id: Uuid, row: Row| {
            if id.is_nil() {
                null_row = true;
            } else {
                rows.push(row);
            }
        };
        match &reply.rows {
            DirRows::People(list) => {
                for r in list.iter().take(keep) {
                    // LLCacheName::buildFullName: "First Last", "First" for a
                    // Resident
                    let name = if r.last_name.is_empty() || r.last_name == "Resident" {
                        r.first_name.clone()
                    } else {
                        format!("{} {}", r.first_name, r.last_name)
                    };
                    push(
                        &mut self.rows,
                        r.id,
                        Row {
                            key: Key::Id(r.id),
                            icon: RowIcon::Person,
                            name,
                            values: Vec::new(),
                        },
                    );
                }
            }
            DirRows::Groups(list) => {
                for r in list.iter().take(keep) {
                    push(
                        &mut self.rows,
                        r.id,
                        Row {
                            key: Key::Id(r.id),
                            icon: RowIcon::Group,
                            name: r.name.clone(),
                            values: vec![Value::Int(r.members as i64)],
                        },
                    );
                }
            }
            DirRows::Places(list) => {
                for r in list.iter().take(keep) {
                    push(
                        &mut self.rows,
                        r.parcel,
                        Row {
                            key: Key::Id(r.parcel),
                            icon: place_icon(r.auction, r.for_sale),
                            name: r.name.clone(),
                            // llformat("%.0f", dwell)
                            values: vec![Value::Int(r.dwell.round() as i64)],
                        },
                    );
                }
            }
            DirRows::Land(list) => {
                for r in list.iter().take(keep) {
                    if !r.parcel.is_nil() && !search::land_row_kept(&self.land, r.price, r.area) {
                        continue;
                    }
                    // Firestorm writes "1.0" for an auction
                    let per_meter = if r.auction {
                        Value::Real(1.0)
                    } else if r.area > 0 {
                        Value::Real(r.price as f64 / r.area as f64)
                    } else {
                        Value::Real(0.0)
                    };
                    let kind = land_types.get(&r.sku).cloned().map_or(Value::None, Value::Text);
                    push(
                        &mut self.rows,
                        r.parcel,
                        Row {
                            key: Key::Id(r.parcel),
                            icon: place_icon(r.auction, r.for_sale),
                            name: r.name.clone(),
                            values: vec![kind, Value::Int(r.price as i64), Value::Int(r.area as i64), per_meter],
                        },
                    );
                }
            }
            DirRows::Events(list) => {
                for r in list.iter().take(keep) {
                    if !search::event_row_kept(self.ratings, r.owner, r.flags) {
                        continue;
                    }
                    let icon = if r.flags == event_flags::ADULT {
                        RowIcon::EventAdult
                    } else if r.flags == event_flags::MATURE {
                        RowIcon::EventModerate
                    } else {
                        RowIcon::EventGeneral
                    };
                    self.rows.push(Row {
                        key: Key::Event(r.id),
                        icon,
                        name: r.name.clone(),
                        values: vec![Value::Dated {
                            text: r.date.clone(),
                            unix: r.unix_time,
                        }],
                    });
                }
            }
            DirRows::Classifieds(list) => {
                for r in list.iter().take(keep) {
                    push(
                        &mut self.rows,
                        r.id,
                        Row {
                            key: Key::Id(r.id),
                            icon: RowIcon::Classified,
                            name: r.name.clone(),
                            values: vec![Value::Int(r.price as i64)],
                        },
                    );
                }
            }
        }
        let added = self.rows.len() > before;
        if added {
            self.status = Status::Found;
        } else if null_row && self.rows.is_empty() {
            // a null row is the directory's way to say "nothing"
            self.status = if self.tab == Tab::Land {
                Status::Idle
            } else {
                Status::NotFound(self.text.clone())
            };
        }
        added
    }

    /// The StatusData checks of each processSearchReply, in their order.
    /// People, groups and land replies carry no status.
    fn status_of(&self, s: u32) -> Option<Status> {
        let common = |s: u32| {
            if s & status::FOUND_NONE != 0 {
                Some(Status::NotFound(self.text.clone()))
            } else if s & status::SHORT_STRING != 0 {
                Some(Status::TooShort)
            } else if s & status::BANNED_WORD != 0 {
                Some(Status::Banned)
            } else if s & status::SEARCH_DISABLED != 0 {
                Some(Status::Disabled)
            } else {
                None
            }
        };
        match self.tab {
            Tab::Places => common(s).or((s & status::PLACES_ESTATE_EMPTY != 0).then_some(Status::Disabled)),
            Tab::Classifieds => common(s),
            Tab::Events => common(s).or(if s & status::EVENTS_NO_DATE_OFFSET != 0 {
                Some(Status::NoDateOffset)
            } else if s & status::EVENTS_NO_CATEGORY != 0 {
                Some(Status::NoCategory)
            } else if s & status::EVENTS_NO_QUERY != 0 {
                Some(Status::NoQuery)
            } else {
                None
            }),
            _ => None,
        }
    }
}

fn place_icon(auction: bool, for_sale: bool) -> RowIcon {
    if auction {
        RowIcon::Auction
    } else if for_sale {
        RowIcon::ForSale
    } else {
        RowIcon::Place
    }
}

/// What a tab's form asks for (the fields `find()` reads).
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    People {
        text: String,
    },
    Groups {
        text: String,
        maturity: u8,
    },
    Places {
        text: String,
        ratings: Ratings,
        category: i8,
    },
    Land {
        ratings: Ratings,
        filter: LandFilter,
    },
    Events {
        text: String,
        ratings: Ratings,
        mode: EventsMode,
        category: u32,
    },
    Classifieds {
        text: String,
        ratings: Ratings,
        category: u32,
    },
}

impl Request {
    pub fn tab(&self) -> Tab {
        match self {
            Request::People { .. } => Tab::People,
            Request::Groups { .. } => Tab::Groups,
            Request::Places { .. } => Tab::Places,
            Request::Land { .. } => Tab::Land,
            Request::Events { .. } => Tab::Events,
            Request::Classifieds { .. } => Tab::Classifieds,
        }
    }
}

pub struct Search {
    lists: [List; 6],
    /// GroupProfileReply of selected groups and visible group rows.
    pub groups: HashMap<Uuid, GroupProfile>,
    /// EventInfoReply of the events selected.
    pub events: HashMap<u32, EventInfo>,
    /// Land types by ProductSKU (ProductInfoRequest), asked once.
    pub land_types: HashMap<String, String>,
    land_types_asked: bool,
    /// SearchWordBanned: a classified search used a banned word (shown once
    /// by the window).
    pub banned_notice: bool,
    out: Vec<NetCommand>,
}

impl Default for Search {
    fn default() -> Self {
        Search {
            lists: [
                List::new(Tab::People),
                List::new(Tab::Groups),
                List::new(Tab::Places),
                List::new(Tab::Land),
                List::new(Tab::Events),
                List::new(Tab::Classifieds),
            ],
            groups: HashMap::new(),
            events: HashMap::new(),
            land_types: HashMap::new(),
            land_types_asked: false,
            banned_notice: false,
            out: Vec::new(),
        }
    }
}

impl Search {
    /// The list of a tab (the web tab has none).
    pub fn list(&self, tab: Tab) -> Option<&List> {
        self.lists.get(tab.list_index()?)
    }

    pub fn list_mut(&mut self, tab: Tab) -> Option<&mut List> {
        self.lists.get_mut(tab.list_index()?)
    }

    /// The land types (ProductInfoRequest), asked once when a list of
    /// parcels needs them.
    pub fn want_land_types(&mut self) {
        if !self.land_types_asked {
            self.land_types_asked = true;
            self.out.push(NetCommand::ProductInfoRequest);
        }
    }

    /// onBtnFind: back to the first page (resetSearch), then `find()`.
    pub fn find(&mut self, request: Request, access: search::AgentAccess, now: Instant) {
        let tab = request.tab();
        if tab == Tab::Land {
            self.want_land_types();
        }
        if tab == Tab::Classifieds {
            self.banned_notice = false;
        }
        let Some(list) = tab.list_index().and_then(|i| self.lists.get_mut(i)) else {
            return;
        };
        list.start = 0;
        let query = match &request {
            Request::People { text } => {
                list.text = text.trim().to_owned();
                match search::people_query(text) {
                    Ok(search::PeopleSearch::Query(q)) => Ok(q),
                    Ok(search::PeopleSearch::Key(id)) => {
                        // the name cache answers: one row, or "not found"
                        list.rows.clear();
                        list.selected = None;
                        list.has_next = false;
                        list.query_id = Uuid::nil();
                        list.status = Status::Searching;
                        list.key_lookup = Some((id, now));
                        return;
                    }
                    Err(e) => Err(e),
                }
            }
            Request::Groups { text, maturity } => {
                list.text = text.clone();
                search::groups_query(text, *maturity, access)
            }
            Request::Places { text, ratings, category } => {
                list.text = text.clone();
                search::places_query(text, *ratings, *category, access)
            }
            Request::Land { ratings, filter } => {
                list.text = String::new();
                list.land = *filter;
                search::land_query(*ratings, *filter, access)
            }
            Request::Events {
                text,
                ratings,
                mode,
                category,
            } => {
                list.text = text.clone();
                list.ratings = *ratings;
                search::events_query(text, *ratings, *mode, *category, access)
            }
            Request::Classifieds { text, ratings, category } => {
                list.text = text.clone();
                search::classifieds_query(text, *ratings, *category)
            }
        };
        match query {
            Ok(q) => {
                let cmd = list.begin(q);
                self.out.push(cmd);
            }
            Err(e) => list.refuse(e),
        }
    }

    /// onBtnNext / onBtnBack: the same query a page further or back.
    pub fn turn_page(&mut self, tab: Tab, forward: bool) {
        let Some(list) = tab.list_index().and_then(|i| self.lists.get_mut(i)) else {
            return;
        };
        let Some(query) = list.query.clone() else {
            return;
        };
        if forward && !list.has_next || !forward && !list.has_back() {
            return;
        }
        let step = RESULT_PAGE_SIZE as i32;
        list.start = (list.start + if forward { step } else { -step }).max(0);
        let cmd = list.begin(query);
        self.out.push(cmd);
    }

    /// The key typed in the people tab: its name once the cache has it
    /// (`name`), "not found" after KEY_LOOKUP_TIMEOUT. Returns the key while
    /// it waits, for the caller to ask the name cache.
    pub fn resolve_key(&mut self, name: impl Fn(&Uuid) -> Option<String>, now: Instant) -> Option<Uuid> {
        let list = self.lists.get_mut(Tab::People.list_index()?)?;
        let (id, since) = list.key_lookup?;
        if let Some(name) = name(&id) {
            list.key_lookup = None;
            list.rows = vec![Row {
                key: Key::Id(id),
                icon: RowIcon::Person,
                name,
                values: Vec::new(),
            }];
            list.status = Status::Found;
            self.select(Tab::People, Key::Id(id));
            return None;
        }
        if now.duration_since(since) >= KEY_LOOKUP_TIMEOUT {
            list.key_lookup = None;
            list.status = Status::NotFound(list.text.clone());
            return None;
        }
        Some(id)
    }

    /// onSelectItem: the selection and the request for its details
    /// (FSFloaterSearch::onSelectedItem / onSelectedEvent).
    pub fn select(&mut self, tab: Tab, key: Key) {
        let Some(list) = tab.list_index().and_then(|i| self.lists.get_mut(i)) else {
            return;
        };
        list.selected = Some(key);
        let cmd = match (tab, key) {
            (Tab::People, Key::Id(id)) => NetCommand::RequestProfile(id),
            (Tab::Groups, Key::Id(id)) => NetCommand::GroupProfileRequest(id),
            (Tab::Places | Tab::Land, Key::Id(id)) => NetCommand::ParcelInfoRequest(id),
            (Tab::Classifieds, Key::Id(id)) => NetCommand::ClassifiedInfoRequest(id),
            (Tab::Events, Key::Event(id)) => NetCommand::EventInfoRequest(id),
            _ => return,
        };
        self.out.push(cmd);
    }

    /// A classified or an event opened from a link (LLClassifiedHandler,
    /// LLEventHandler show it in a window of its own): the tab's list holds
    /// it alone, selected, its details asked; its name comes with them
    /// (`name_row`). Replies of the tab's former query no longer apply.
    pub fn show_one(&mut self, tab: Tab, key: Key) {
        let Some(list) = tab.list_index().and_then(|i| self.lists.get_mut(i)) else {
            return;
        };
        list.query = None;
        list.query_id = Uuid::nil();
        list.start = 0;
        list.has_next = false;
        list.key_lookup = None;
        list.rows = vec![Row {
            key,
            icon: if tab == Tab::Events {
                RowIcon::EventGeneral
            } else {
                RowIcon::Classified
            },
            name: String::new(),
            values: Vec::new(),
        }];
        list.status = Status::Found;
        self.select(tab, key);
    }

    /// The details of a row answered: a row shown without its name
    /// (`show_one`) takes it, and its columns (an event's date).
    pub fn name_row(&mut self, tab: Tab, key: Key, name: &str, values: Vec<Value>) {
        let Some(list) = tab.list_index().and_then(|i| self.lists.get_mut(i)) else {
            return;
        };
        for row in list.rows.iter_mut().filter(|r| r.key == key && r.name.is_empty()) {
            row.name = name.to_owned();
            row.values = values.clone();
        }
    }

    /// A Dir*Reply: applied to the list whose query it answers. The first
    /// rows of a page select the first one shown (selectFirstItem).
    pub fn apply_reply(&mut self, reply: &DirReply) {
        // classifieds: the banned word notice comes even for an old query
        if matches!(reply.rows, DirRows::Classifieds(_)) && reply.status.is_some_and(|s| s & status::BANNED_WORD != 0) {
            self.banned_notice = true;
        }
        let tab = match reply.rows {
            DirRows::People(_) => Tab::People,
            DirRows::Groups(_) => Tab::Groups,
            DirRows::Places(_) => Tab::Places,
            DirRows::Land(_) => Tab::Land,
            DirRows::Events(_) => Tab::Events,
            DirRows::Classifieds(_) => Tab::Classifieds,
        };
        let Some(list) = tab.list_index().and_then(|i| self.lists.get_mut(i)) else {
            return;
        };
        if list.apply(reply, &self.land_types) && list.selected.is_none() {
            let first = list.sorted().first().map(|r| r.key);
            if let Some(key) = first {
                self.select(tab, key);
            }
        }
    }

    /// Reconcile directory counts when profiles, member lists or a new
    /// search page arrive. Update the values so sorting uses them too.
    pub(super) fn refresh_group_counts(&mut self, data: &super::group_data::GroupMgr) {
        let Some(list) = Tab::Groups.list_index().and_then(|i| self.lists.get_mut(i)) else {
            return;
        };
        for row in &mut list.rows {
            if let Key::Id(id) = row.key
                && let Some(count) = data.member_count(id, self.groups.get(&id))
                && let Some(value) = row.values.first_mut()
            {
                *value = Value::Int(count);
            }
        }
    }

    /// ProductInfoRequest answered: the types of the rows already listed.
    pub fn apply_land_types(&mut self, types: Vec<(String, String)>) {
        self.land_types = types.into_iter().collect();
    }

    /// GroupProfileRequest for the group profile window (LLGroupMgr's
    /// sendGroupPropertiesRequest when the panel opens on a group).
    pub fn request_group(&mut self, id: Uuid) {
        self.out.push(NetCommand::GroupProfileRequest(id));
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

/// LLURI::escape: everything but the unreserved characters is percent-encoded.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// SearchURL of Firestorm's settings.xml: the search page of a Second Life
/// grid, with the substitutions of LLWeb::expandURLSubstitutions.
const SEARCH_URL: &str =
    "https://search.[GRID]/viewer/?query_term=[QUERY]&search_type=[TYPE][COLLECTION]&maturity=[MATURITY]&lang=[LANGUAGE]&sid=[SESSION_ID]";

/// The search page of the web tab, built like FSPanelSearchWeb::loadURL on
/// a Second Life grid: "&maturity=" with what the agent prefers ("g", "gm",
/// "gma") and "&" with the (empty) path of the default category are appended
/// to SearchURL, then LLWeb::expandURLSubstitutions fills [GRID], [QUERY]
/// (LLURI::escape), [MATURITY] and [CATEGORY] (both empty on Second Life),
/// [LANGUAGE] and [SESSION_ID], and escapes spaces and backslashes
/// (LLWeb::escapeURL). Firestorm's panel gives no value to [TYPE] and
/// [COLLECTION] (only LLFloaterSearch::initiateSearch does) and
/// LLStringUtil::format leaves a token without substitution as it is: they
/// stay in the address, as Firestorm sends it.
///
/// `session` is the agent's session id (`sid`): it identifies the session
/// to the grid's own search page, the way Firestorm does. It only goes into
/// an address of that host (`is_search_host`), and an address built without
/// a session has no `sid` at all. Never log or save the result.
pub fn web_url(grid_domain: &str, query: &str, access: search::AgentAccess, language: &str, session: Option<Uuid>) -> String {
    expand(SEARCH_URL, grid_domain, query, access, language, session)
}

fn expand(template: &str, grid_domain: &str, query: &str, access: search::AgentAccess, language: &str, session: Option<Uuid>) -> String {
    let maturity = if access.prefers_adult() {
        "gma"
    } else if access.prefers_mature() {
        "gm"
    } else {
        "g"
    };
    let query = escape(query);
    let mut url = format!("{template}&maturity={maturity}&");
    for (token, value) in [
        ("[GRID]", grid_domain),
        ("[QUERY]", query.as_str()),
        ("[MATURITY]", ""),
        ("[CATEGORY]", ""),
        ("[LANGUAGE]", language),
    ] {
        url = url.replace(token, value);
    }
    url = match session.filter(|_| is_search_host(&url, grid_domain)) {
        Some(id) => url.replace("[SESSION_ID]", &id.to_string()),
        None => without_session(&url.replace("[SESSION_ID]", ""), None),
    };
    url.replace(' ', "%20").replace('\\', "%5C")
}

/// The grid a session was opened on, as far as the web search goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchGrid {
    /// The main grid: search.secondlife.com is its own search site.
    Main,
    /// Another Second Life grid (the beta grid): Firestorm's [GRID] still
    /// gives "secondlife.com" for it, which is the main grid's site.
    Beta,
    /// Not a Second Life grid: no search page.
    Other,
}

/// The search page of the web tab and how it is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebHome {
    pub url: String,
    /// Opened with the session: `sid` is in the address and the OpenID
    /// cookie goes to the browser. False: the page is anonymous, and the
    /// browser must not get the cookie either.
    pub identified: bool,
}

/// The search page of `grid`, None when it has none. The session only
/// identifies the agent to the search site of the grid it was opened on: on
/// the main grid the page is opened with it, as Firestorm does; on the beta
/// grid the address derived is still the main grid's site, so the page is
/// opened anonymously. This is a deliberate deviation from Firestorm, asked
/// by the human: Firestorm sends the beta session id (and gives the beta
/// OpenID cookie to the browser) to search.secondlife.com. `query` is the
/// text of Aurora's field (the Legacy tab opens the page without one).
pub fn web_home(grid: SearchGrid, query: &str, access: search::AgentAccess, language: &str, session: Option<Uuid>) -> Option<WebHome> {
    // LLWeb::expandURLSubstitutions: [GRID] is "secondlife.com" for every
    // grid but Linden Lab's staging one
    let domain = match grid {
        SearchGrid::Main | SearchGrid::Beta => "secondlife.com",
        SearchGrid::Other => return None,
    };
    let session = session.filter(|_| grid == SearchGrid::Main);
    let url = web_url(domain, query, access, language, session);
    let identified = session.is_some_and(|id| url.contains(&id.to_string()));
    Some(WebHome { url, identified })
}

/// Whether `url` is a page of the grid's search site: https, and the host
/// "search.<grid domain>" exactly (no user info, no port, no look-alike
/// prefix or suffix). The session id is put in no other address.
pub fn is_search_host(url: &str, grid_domain: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !grid_domain.is_empty() && authority.eq_ignore_ascii_case(&format!("search.{grid_domain}"))
}

/// `url` as it may be shown, copied or handed to another program: without
/// its `sid` parameter, and without the text of `session` anywhere else.
pub fn without_session(url: &str, session: Option<Uuid>) -> String {
    let (main, fragment) = match url.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (url, None),
    };
    let mut out = match main.split_once('?') {
        Some((base, query)) => {
            let kept: Vec<&str> = query
                .split('&')
                .filter(|p| !(p.eq_ignore_ascii_case("sid") || p.get(..4).is_some_and(|k| k.eq_ignore_ascii_case("sid="))))
                .collect();
            format!("{base}?{}", kept.join("&"))
        }
        None => main.to_owned(),
    };
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    if let Some(id) = session {
        // whatever form a page gave it in another parameter
        for form in [id.hyphenated().to_string(), id.simple().to_string()] {
            while let Some(at) = out.to_ascii_lowercase().find(&form) {
                out.replace_range(at..at + form.len(), "");
            }
        }
    }
    out
}

/// "string.duration": hours and minutes of an event ("%d:%.2d").
pub fn event_duration(minutes: u32) -> String {
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// FSFloaterSearch::displayEventDetails: the rating of an event is its
/// flags, compared whole.
pub fn event_rating(flags: u32) -> u8 {
    match flags {
        event_flags::ADULT => search::SIM_ACCESS_ADULT,
        event_flags::MATURE => search::SIM_ACCESS_MATURE,
        _ => search::SIM_ACCESS_PG,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::search::{AgentAccess, ClassifiedRow, EventRow, GroupRow, LandRow, PersonRow, PlaceRow};

    const ACCESS: AgentAccess = AgentAccess {
        max: search::SIM_ACCESS_ADULT,
        preferred: search::SIM_ACCESS_ADULT,
    };

    fn people(n: usize, from: u128) -> DirRows {
        DirRows::People(
            (0..n)
                .map(|i| PersonRow {
                    id: Uuid::from_u128(from + i as u128),
                    first_name: format!("Nova{i:03}"),
                    last_name: "Resident".into(),
                    online: false,
                })
                .collect(),
        )
    }

    fn searching(s: &mut Search, request: Request) -> Uuid {
        let tab = request.tab();
        s.find(request, ACCESS, Instant::now());
        s.list(tab).expect("list").query_id
    }

    fn reply(s: &mut Search, query_id: Uuid, status: Option<u32>, rows: DirRows) {
        s.apply_reply(&DirReply { query_id, status, rows });
    }

    #[test]
    fn a_page_keeps_a_hundred_rows_and_offers_the_next() {
        let mut s = Search::default();
        let id = searching(&mut s, Request::People { text: "nova".into() });
        assert!(matches!(&s.take_commands()[..], [NetCommand::DirQuery { start: 0, .. }]));
        assert_eq!(s.list(Tab::People).expect("list").status, Status::Searching);
        // the page comes in several packets: 60 + 41 rows
        reply(&mut s, id, None, people(60, 1));
        reply(&mut s, id, None, people(41, 1000));
        let list = s.list(Tab::People).expect("list");
        assert_eq!(list.rows.len(), 100);
        assert!(list.has_next && !list.has_back());
        assert_eq!(list.status, Status::Found);
        // the first row was selected once and its profile asked
        assert_eq!(list.selected, Some(Key::Id(Uuid::from_u128(1))));
        assert!(matches!(&s.take_commands()[..], [NetCommand::RequestProfile(a)] if *a == Uuid::from_u128(1)));
        // next page: the same query from row 100, with another id
        s.turn_page(Tab::People, true);
        let cmds = s.take_commands();
        let [
            NetCommand::DirQuery {
                id: next,
                start: 100,
                query: DirQuery::Find { text, .. },
            },
        ] = &cmds[..]
        else {
            panic!("{cmds:?}");
        };
        assert_eq!(text, "nova");
        assert_ne!(*next, id);
        let list = s.list(Tab::People).expect("list");
        assert!(list.has_back() && list.rows.is_empty());
        assert_eq!(list.page(), 2);
        // a late packet of the first page is ignored
        reply(&mut s, id, None, people(3, 5000));
        assert!(s.list(Tab::People).expect("list").rows.is_empty());
        reply(&mut s, *next, None, people(12, 9000));
        let list = s.list(Tab::People).expect("list");
        assert_eq!((list.rows.len(), list.has_next), (12, false));
        s.turn_page(Tab::People, true);
        assert!(
            s.take_commands().iter().all(|c| !matches!(c, NetCommand::DirQuery { .. })),
            "no next page"
        );
        s.turn_page(Tab::People, false);
        assert!(matches!(&s.take_commands()[..], [.., NetCommand::DirQuery { start: 0, .. }]));
    }

    #[test]
    fn names_and_empty_answers() {
        let mut s = Search::default();
        let id = searching(&mut s, Request::People { text: "orion".into() });
        reply(
            &mut s,
            id,
            None,
            DirRows::People(vec![PersonRow {
                id: Uuid::from_u128(7),
                first_name: "Orion".into(),
                last_name: "Exemple".into(),
                online: true,
            }]),
        );
        assert_eq!(s.list(Tab::People).expect("list").rows[0].name, "Orion Exemple");
        // no row at all
        let id = searching(&mut s, Request::People { text: "zzzz".into() });
        reply(&mut s, id, None, people(0, 0));
        assert_eq!(s.list(Tab::People).expect("list").status, Status::NotFound("zzzz".into()));
        // a null row
        let id = searching(&mut s, Request::People { text: "yyyy".into() });
        reply(
            &mut s,
            id,
            None,
            DirRows::People(vec![PersonRow {
                id: Uuid::nil(),
                first_name: String::new(),
                last_name: String::new(),
                online: false,
            }]),
        );
        let list = s.list(Tab::People).expect("list");
        assert!(list.rows.is_empty());
        assert_eq!(list.status, Status::NotFound("yyyy".into()));
        // too short: nothing is sent
        s.take_commands();
        s.find(Request::People { text: "ab".into() }, ACCESS, Instant::now());
        assert!(s.take_commands().is_empty());
        assert_eq!(s.list(Tab::People).expect("list").status, Status::TooShort);
    }

    #[test]
    fn a_key_waits_for_its_name() {
        let mut s = Search::default();
        let key = Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        let t0 = Instant::now();
        s.find(Request::People { text: key.to_string() }, ACCESS, t0);
        assert!(s.take_commands().is_empty(), "the directory is not asked");
        assert_eq!(s.resolve_key(|_| None, t0), Some(key));
        assert_eq!(s.resolve_key(|_| Some("Nova Exemple".into()), t0), None);
        let list = s.list(Tab::People).expect("list");
        assert_eq!((list.rows.len(), list.selected), (1, Some(Key::Id(key))));
        assert!(matches!(&s.take_commands()[..], [NetCommand::RequestProfile(a)] if *a == key));
        // an unknown key gives up
        s.find(Request::People { text: key.to_string() }, ACCESS, t0);
        assert_eq!(s.resolve_key(|_| None, t0 + KEY_LOOKUP_TIMEOUT), None);
        assert!(matches!(s.list(Tab::People).expect("list").status, Status::NotFound(_)));
    }

    #[test]
    fn status_codes() {
        let mut s = Search::default();
        let places = |text: &str| Request::Places {
            text: text.into(),
            ratings: Ratings::default(),
            category: search::PLACE_CATEGORY_ANY,
        };
        let empty = || DirRows::Places(Vec::new());
        for (code, expected) in [
            (status::FOUND_NONE, Status::NotFound("plage".into())),
            (status::SHORT_STRING, Status::TooShort),
            (status::BANNED_WORD, Status::Banned),
            (status::SEARCH_DISABLED, Status::Disabled),
            (status::PLACES_ESTATE_EMPTY, Status::Disabled),
        ] {
            let id = searching(&mut s, places("plage"));
            reply(&mut s, id, Some(code), empty());
            assert_eq!(s.list(Tab::Places).expect("list").status, expected, "{code}");
        }
        assert!(!s.banned_notice, "only classifieds raise the notice");
        // a zero status is no status: the rows are read
        let id = searching(&mut s, places("plage"));
        reply(
            &mut s,
            id,
            Some(0),
            DirRows::Places(vec![PlaceRow {
                parcel: Uuid::from_u128(4),
                name: "Plage".into(),
                for_sale: true,
                auction: false,
                dwell: 1234.6,
            }]),
        );
        let list = s.list(Tab::Places).expect("list");
        assert_eq!(list.rows[0].icon, RowIcon::ForSale);
        assert_eq!(list.rows[0].values, vec![Value::Int(1235)]);
        // events have their own codes
        let events = Request::Events {
            text: String::new(),
            ratings: Ratings::default(),
            mode: EventsMode::Current,
            category: 0,
        };
        for (code, expected) in [
            (status::EVENTS_NO_DATE_OFFSET, Status::NoDateOffset),
            (status::EVENTS_NO_CATEGORY, Status::NoCategory),
            (status::EVENTS_NO_QUERY, Status::NoQuery),
        ] {
            let id = searching(&mut s, events.clone());
            reply(&mut s, id, Some(code), DirRows::Events(Vec::new()));
            assert_eq!(s.list(Tab::Events).expect("list").status, expected);
        }
        // classifieds: the banned word is also a notice
        let id = searching(
            &mut s,
            Request::Classifieds {
                text: "meubles".into(),
                ratings: Ratings::default(),
                category: 0,
            },
        );
        reply(&mut s, id, Some(status::BANNED_WORD), DirRows::Classifieds(Vec::new()));
        assert_eq!(s.list(Tab::Classifieds).expect("list").status, Status::Banned);
        assert!(s.banned_notice);
    }

    #[test]
    fn land_is_filtered_and_typed() {
        let mut s = Search::default();
        let filter = LandFilter {
            limit_price: true,
            limit_area: true,
            price: 3000,
            area: 512,
            ..Default::default()
        };
        let id = searching(
            &mut s,
            Request::Land {
                ratings: Ratings::default(),
                filter,
            },
        );
        let cmds = s.take_commands();
        assert!(matches!(&cmds[..], [NetCommand::ProductInfoRequest, NetCommand::DirQuery { .. }]));
        s.apply_land_types(vec![("023".into(), "Continent / Région entière".into())]);
        let row = |n: u128, price: i32, area: i32, auction: bool| LandRow {
            parcel: Uuid::from_u128(n),
            name: format!("Parcelle {n}"),
            auction,
            for_sale: !auction,
            price,
            area,
            sku: "023".into(),
        };
        reply(
            &mut s,
            id,
            None,
            DirRows::Land(vec![
                row(1, 2500, 1024, false),
                row(2, 3001, 1024, false),
                row(3, 100, 256, false),
                row(4, 0, 2048, true),
            ]),
        );
        let list = s.list(Tab::Land).expect("list");
        assert_eq!(list.rows.len(), 2);
        assert_eq!(
            list.rows[0].values,
            vec![
                Value::Text("Continent / Région entière".into()),
                Value::Int(2500),
                Value::Int(1024),
                Value::Real(2500.0 / 1024.0)
            ]
        );
        assert_eq!(list.rows[1].icon, RowIcon::Auction);
        assert_eq!(list.rows[1].values[3], Value::Real(1.0), "Firestorm's figure for an auction");
        assert!(matches!(&s.take_commands()[..], [NetCommand::ParcelInfoRequest(p)] if *p == Uuid::from_u128(1)));
        // the types are asked once
        searching(
            &mut s,
            Request::Land {
                ratings: Ratings::default(),
                filter,
            },
        );
        assert!(matches!(&s.take_commands()[..], [NetCommand::DirQuery { .. }]));
    }

    #[test]
    fn events_follow_the_rating_boxes_and_sort_by_time() {
        let mut s = Search::default();
        let id = searching(
            &mut s,
            Request::Events {
                text: String::new(),
                ratings: Ratings::default(),
                mode: EventsMode::Day(1),
                category: 20,
            },
        );
        let cmds = s.take_commands();
        assert!(matches!(&cmds[..], [NetCommand::DirQuery { query: DirQuery::Find { text, .. }, .. }] if text == "1|20|"));
        let ev = |id: u32, unix: u32, flags: u32, owner: u128| EventRow {
            owner: Uuid::from_u128(owner),
            name: format!("Concert {id}"),
            id,
            date: format!("10/12 {id}:00"),
            unix_time: unix,
            flags,
        };
        reply(
            &mut s,
            id,
            None,
            DirRows::Events(vec![
                ev(1, 300, event_flags::NONE, 9),
                ev(2, 100, event_flags::NONE, 9),
                ev(3, 50, event_flags::MATURE, 9),
                ev(4, 10, event_flags::NONE, 0),
            ]),
        );
        let list = s.list(Tab::Events).expect("list");
        let shown: Vec<Key> = list.sorted().iter().map(|r| r.key).collect();
        assert_eq!(
            shown,
            vec![Key::Event(2), Key::Event(1)],
            "time ascending, moderate and ownerless dropped"
        );
        // the first one shown is selected
        assert_eq!(list.selected, Some(Key::Event(2)));
        assert!(matches!(&s.take_commands()[..], [NetCommand::EventInfoRequest(2)]));
    }

    #[test]
    fn header_clicks_sort_and_reverse() {
        let mut s = Search::default();
        let id = searching(
            &mut s,
            Request::Groups {
                text: "loups".into(),
                maturity: search::SIM_ACCESS_PG,
            },
        );
        let g = |n: u128, name: &str, members: i32| GroupRow {
            id: Uuid::from_u128(n),
            name: name.into(),
            members,
            search_order: 0.0,
        };
        reply(
            &mut s,
            id,
            None,
            DirRows::Groups(vec![g(1, "loups gris", 40), g(2, "Aurora", 7), g(3, "Meute", 310)]),
        );
        assert!(matches!(&s.take_commands()[..], [.., NetCommand::GroupProfileRequest(a)] if *a == Uuid::from_u128(1)));
        let list = s.list_mut(Tab::Groups).expect("list");
        let names = |l: &List| l.sorted().iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(list), ["loups gris", "Aurora", "Meute"], "the directory's order");
        list.sort_by(0);
        assert_eq!(names(list), ["Aurora", "loups gris", "Meute"]);
        list.sort_by(1);
        assert_eq!(names(list), ["Aurora", "loups gris", "Meute"]);
        list.sort_by(1);
        assert_eq!(names(list), ["Meute", "loups gris", "Aurora"]);
        // a selection asks for the details of its kind
        s.select(Tab::Groups, Key::Id(Uuid::from_u128(3)));
        assert!(matches!(&s.take_commands()[..], [NetCommand::GroupProfileRequest(a)] if *a == Uuid::from_u128(3)));
        let id = searching(
            &mut s,
            Request::Classifieds {
                text: "meubles".into(),
                ratings: Ratings::default(),
                category: 0,
            },
        );
        reply(
            &mut s,
            id,
            None,
            DirRows::Classifieds(vec![ClassifiedRow {
                id: Uuid::from_u128(5),
                name: "Meubles".into(),
                flags: 0,
                creation_date: 0,
                expiration_date: 0,
                price: 50,
            }]),
        );
        assert!(matches!(&s.take_commands()[..], [.., NetCommand::ClassifiedInfoRequest(a)] if *a == Uuid::from_u128(5)));
    }

    #[test]
    fn no_rating_box_no_search() {
        let mut s = Search::default();
        let none = Ratings {
            pg: false,
            mature: false,
            adult: false,
        };
        s.find(
            Request::Places {
                text: "plage".into(),
                ratings: none,
                category: 3,
            },
            ACCESS,
            Instant::now(),
        );
        assert!(s.take_commands().is_empty());
        assert_eq!(s.list(Tab::Places).expect("list").status, Status::NoContent);
    }

    #[test]
    fn web_address_is_the_one_firestorm_builds() {
        let session = Uuid::from_u128(0x5E55_1011_0000_4000_8000_0000_0000_00A1);
        // FSPanelSearchWeb::loadURL at the opening of the floater: no query,
        // the default category, an agent who prefers adult content
        assert_eq!(
            web_url("secondlife.com", "", ACCESS, "fr", Some(session)),
            format!(
                "https://search.secondlife.com/viewer/?query_term=&search_type=[TYPE][COLLECTION]&maturity=&lang=fr&sid={session}&maturity=gma&"
            )
        );
        // the query is escaped like LLURI::escape, the maturity follows the
        // preference
        let url = web_url("secondlife.com", "club de jazz & co", AgentAccess::default(), "en", Some(session));
        assert!(url.contains("?query_term=club%20de%20jazz%20%26%20co&"));
        assert!(url.ends_with(&format!("&lang=en&sid={session}&maturity=g&")));
        // without a session there is no sid at all
        let anonymous = web_url("secondlife.com", "", ACCESS, "fr", None);
        assert_eq!(
            anonymous,
            "https://search.secondlife.com/viewer/?query_term=&search_type=[TYPE][COLLECTION]&maturity=&lang=fr&maturity=gma&"
        );
        assert!(!anonymous.contains("sid"));
    }

    #[test]
    fn the_session_only_goes_to_the_search_host() {
        let session = Uuid::from_u128(0x5E55_1011_0000_4000_8000_0000_0000_00A1);
        assert!(is_search_host("https://search.secondlife.com/viewer/?a=b", "secondlife.com"));
        assert!(is_search_host("https://SEARCH.secondlife.com", "secondlife.com"));
        for other in [
            "http://search.secondlife.com/viewer/",
            "https://search.secondlife.com.example.org/viewer/",
            "https://evil.example/search.secondlife.com/",
            "https://user@search.secondlife.com/",
            "https://search.secondlife.com:8443/",
            "https://xsearch.secondlife.com/",
            "https://secondlife.com/",
            "file:///C:/page.html",
        ] {
            assert!(!is_search_host(other, "secondlife.com"), "{other}");
        }
        assert!(!is_search_host("https://search./", ""));
        // a template that would send the address elsewhere gets no session
        for template in [
            "https://search.example.org/?q=[QUERY]&sid=[SESSION_ID]",
            "http://search.[GRID]/viewer/?q=[QUERY]&sid=[SESSION_ID]",
            "https://search.[GRID].example.org/?sid=[SESSION_ID]&q=[QUERY]",
        ] {
            let url = expand(template, "secondlife.com", "x", ACCESS, "fr", Some(session));
            assert!(!url.contains(&session.to_string()), "{url}");
            assert!(!url.contains("sid="), "{url}");
        }
    }

    #[test]
    fn the_session_stays_on_the_grid_it_was_opened_on() {
        let session = Uuid::from_u128(0x5E55_1011_0000_4000_8000_0000_0000_00A1);
        // main grid: as Firestorm, the page is opened with the session
        let main = web_home(SearchGrid::Main, "", ACCESS, "fr", Some(session)).expect("search page");
        assert!(main.identified);
        assert_eq!(main.url, web_url("secondlife.com", "", ACCESS, "fr", Some(session)));
        assert!(main.url.contains(&format!("&sid={session}&")));
        // beta grid: the same site, but neither sid nor cookie
        let beta = web_home(SearchGrid::Beta, "", ACCESS, "fr", Some(session)).expect("search page");
        assert!(!beta.identified, "the browser gets no OpenID cookie");
        assert_eq!(beta.url, web_url("secondlife.com", "", ACCESS, "fr", None));
        assert!(!beta.url.contains("sid") && !beta.url.contains(&session.to_string()));
        // without a session the main grid's page is anonymous too
        let none = web_home(SearchGrid::Main, "", ACCESS, "fr", None).expect("search page");
        assert!(!none.identified && !none.url.contains("sid"));
        // any other grid: no page, as before
        assert_eq!(web_home(SearchGrid::Other, "", ACCESS, "fr", Some(session)), None);
        // the text of Aurora's field goes to the page, escaped
        let asked = web_home(SearchGrid::Main, "lune rousse", ACCESS, "fr", None).expect("search page");
        assert!(asked.url.contains("query_term=lune%20rousse&"), "{}", asked.url);
    }

    #[test]
    fn the_address_shown_never_carries_the_session() {
        let session = Uuid::from_u128(0x5E55_1011_0000_4000_8000_0000_0000_00A1);
        let home = web_url("secondlife.com", "aurora", ACCESS, "fr", Some(session));
        assert!(home.contains(&session.to_string()));
        let shown = without_session(&home, Some(session));
        assert_eq!(shown, web_url("secondlife.com", "aurora", ACCESS, "fr", None));
        assert!(!shown.contains(&session.to_string()) && !shown.contains("sid="));
        // wherever a page of the site puts it afterwards
        for (url, expected) in [
            (
                format!("https://search.secondlife.com/viewer/?sid={session}"),
                "https://search.secondlife.com/viewer/?".to_owned(),
            ),
            (
                format!("https://search.secondlife.com/viewer/x?a=1&SID={session}&b=2#top"),
                "https://search.secondlife.com/viewer/x?a=1&b=2#top".to_owned(),
            ),
            (
                format!(
                    "https://search.secondlife.com/r?next={}&c=3",
                    session.simple().to_string().to_uppercase()
                ),
                "https://search.secondlife.com/r?next=&c=3".to_owned(),
            ),
            (
                format!("https://search.secondlife.com/p/{session}/x"),
                "https://search.secondlife.com/p//x".to_owned(),
            ),
            (
                "https://search.secondlife.com/viewer/".to_owned(),
                "https://search.secondlife.com/viewer/".to_owned(),
            ),
        ] {
            assert_eq!(without_session(&url, Some(session)), expected);
        }
        // a sid is hidden even when the session is not known
        assert_eq!(without_session("https://a.example/?sid=abc&x=1", None), "https://a.example/?x=1");
    }

    #[test]
    fn durations_ratings_and_tab_names() {
        assert_eq!(event_duration(90), "1:30");
        assert_eq!(event_duration(5), "0:05");
        assert_eq!(event_rating(event_flags::ADULT), search::SIM_ACCESS_ADULT);
        assert_eq!(event_rating(event_flags::MATURE), search::SIM_ACCESS_MATURE);
        assert_eq!(event_rating(0), search::SIM_ACCESS_PG);
        assert_eq!(Tab::from_name("Land"), Some(Tab::Land));
        // floater_fs_search.xml: the web search is the first tab, and the
        // one FSLastSearchTab (0) opens the first time
        assert_eq!(
            Tab::ALL,
            [
                Tab::Web,
                Tab::People,
                Tab::Groups,
                Tab::Places,
                Tab::Land,
                Tab::Events,
                Tab::Classifieds
            ]
        );
        assert_eq!(Tab::from_index(SearchSettings::default().last_tab as usize), Tab::Web);
        assert_eq!(Tab::from_index(99), Tab::Web);
        for (i, tab) in Tab::ALL.into_iter().enumerate() {
            assert_eq!((tab.index(), Tab::from_index(i)), (i, tab));
        }
        // every directory tab has its own list, the web tab none
        let s = Search::default();
        assert!(s.list(Tab::Web).is_none());
        for tab in Tab::ALL.into_iter().skip(1) {
            assert_eq!(s.list(tab).map(|l| l.tab), Some(tab));
        }
    }

    #[test]
    fn a_classified_or_an_event_shown_alone() {
        let mut s = Search::default();
        let id = Uuid::from_u128(5);
        s.show_one(Tab::Classifieds, Key::Id(id));
        assert!(matches!(&s.take_commands()[..], [NetCommand::ClassifiedInfoRequest(a)] if *a == id));
        let list = s.list(Tab::Classifieds).expect("classifieds");
        assert_eq!(list.selected, Some(Key::Id(id)));
        assert_eq!(list.status, Status::Found);
        assert!(list.query_id.is_nil(), "an older query's replies no longer apply");
        s.name_row(Tab::Classifieds, Key::Id(id), "Atelier", Vec::new());
        assert_eq!(s.list(Tab::Classifieds).map(|l| l.rows[0].name.as_str()), Some("Atelier"));
        s.show_one(Tab::Events, Key::Event(5001));
        assert!(matches!(&s.take_commands()[..], [NetCommand::EventInfoRequest(5001)]));
        let date = Value::Dated {
            text: "10/08 09:00".into(),
            unix: 1_791_475_200,
        };
        s.name_row(Tab::Events, Key::Event(5001), "Jazz", vec![date.clone()]);
        let row = &s.list(Tab::Events).expect("events").rows[0];
        assert_eq!((row.name.as_str(), &row.values[..]), ("Jazz", &[date][..]));
    }
}
