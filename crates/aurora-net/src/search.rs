//! Directory search ("Search"): the queries of Firestorm's legacy
//! search floater and what the directory answers. Port of the query builders
//! of FSPanelSearchPeople / Groups / Places / Land / Classifieds / Events
//! `::find` and of the fields their `processSearchReply` read
//! (indra/newview/fsfloatersearch.cpp), with the flags of
//! indra/llmessage/llqueryflags.h, llclassifiedflags.cpp and lleventflags.h,
//! the content access rules of indra/newview/llagentaccess.cpp, and the
//! detail replies LLEventInfo::unpack (lleventnotifier.cpp) and
//! LLGroupMgr::processGroupPropertiesReply (llgroupmgr.cpp); all originally
//! LGPL 2.1.
//!
//! Everything here is pure: `session_search.rs` puts the queries on the wire
//! and decodes the replies.

use aurora_llsd::Llsd;
use aurora_msg::field_str;
use aurora_msg::msgs;
use uuid::Uuid;

/// RESULT_PAGE_SIZE: rows per page; the directory sends one more when there
/// is a next page.
pub const RESULT_PAGE_SIZE: usize = 100;
/// MIN_SEARCH_STRING_SIZE: words of this length or less are dropped.
pub const MIN_SEARCH_STRING_SIZE: usize = 2;

/// DFQ_* (llqueryflags.h).
pub mod dfq {
    pub const PEOPLE: u32 = 1 << 0;
    pub const GROUPS: u32 = 1 << 4;
    pub const DATE_EVENTS: u32 = 1 << 5;
    pub const DWELL_SORT: u32 = 1 << 10;
    pub const PG_SIMS_ONLY: u32 = 1 << 11;
    pub const SORT_ASC: u32 = 1 << 15;
    pub const PRICE_SORT: u32 = 1 << 16;
    pub const PER_METER_SORT: u32 = 1 << 17;
    pub const AREA_SORT: u32 = 1 << 18;
    pub const NAME_SORT: u32 = 1 << 19;
    pub const LIMIT_BY_PRICE: u32 = 1 << 20;
    pub const LIMIT_BY_AREA: u32 = 1 << 21;
    pub const INC_PG: u32 = 1 << 24;
    pub const INC_MATURE: u32 = 1 << 25;
    pub const INC_ADULT: u32 = 1 << 26;
}

/// ST_* sell types of DirLandQuery (llqueryflags.h).
pub mod sell_type {
    pub const AUCTION: u32 = 1 << 1;
    pub const MAINLAND: u32 = 1 << 3;
    pub const ESTATE: u32 = 1 << 4;
    pub const ALL: u32 = 0xFFFF_FFFF;
}

/// STATUS_SEARCH_* of the StatusData block (llqueryflags.h). The first four
/// bits mean the same for places, events and classifieds; bit 4 is
/// ESTATEEMPTY for places and NODATEOFFSET for events.
pub mod status {
    pub const BANNED_WORD: u32 = 1 << 0;
    pub const SHORT_STRING: u32 = 1 << 1;
    pub const FOUND_NONE: u32 = 1 << 2;
    pub const SEARCH_DISABLED: u32 = 1 << 3;
    pub const PLACES_ESTATE_EMPTY: u32 = 1 << 4;
    pub const EVENTS_NO_DATE_OFFSET: u32 = 1 << 4;
    pub const EVENTS_NO_CATEGORY: u32 = 1 << 5;
    pub const EVENTS_NO_QUERY: u32 = 1 << 6;
}

/// EVENT_FLAG_* (lleventflags.h).
pub mod event_flags {
    pub const NONE: u32 = 0;
    pub const MATURE: u32 = 1;
    pub const ADULT: u32 = 2;
}

/// SIM_ACCESS_* (indra_constants.h).
pub const SIM_ACCESS_PG: u8 = 13;
pub const SIM_ACCESS_MATURE: u8 = 21;
pub const SIM_ACCESS_ADULT: u8 = 42;

/// LLViewerRegion::accessToShortString.
pub fn access_short_string(access: u8) -> &'static str {
    match access {
        SIM_ACCESS_ADULT => "A",
        SIM_ACCESS_MATURE => "M",
        SIM_ACCESS_PG => "PG",
        _ => "U",
    }
}

/// LLViewerRegion::shortStringToAccess; SIM_ACCESS_MIN (0) when unknown.
pub fn short_string_to_access(text: &str) -> u8 {
    match text {
        "A" => SIM_ACCESS_ADULT,
        "M" => SIM_ACCESS_MATURE,
        "PG" => SIM_ACCESS_PG,
        _ => 0,
    }
}

/// LLParcel::C_ANY: every parcel category of a places search.
pub const PLACE_CATEGORY_ANY: i8 = -1;

/// What content the agent may see and wants to see (LLAgentAccess): the
/// account's ceiling (`agent_access_max` of the login reply) and the
/// preference kept by the grid (`agent_region_access`, PreferredMaturity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAccess {
    pub max: u8,
    pub preferred: u8,
}

impl Default for AgentAccess {
    /// LLAgentAccess starts at SIM_ACCESS_PG.
    fn default() -> Self {
        AgentAccess {
            max: SIM_ACCESS_PG,
            preferred: SIM_ACCESS_PG,
        }
    }
}

impl AgentAccess {
    /// LLAgentAccess::convertTextToMaturity: 'A', 'M', 'P'; SIM_ACCESS_MIN else.
    fn from_text(text: &str) -> u8 {
        match text.as_bytes().first() {
            Some(b'A') => SIM_ACCESS_ADULT,
            Some(b'M') => SIM_ACCESS_MATURE,
            Some(b'P') => SIM_ACCESS_PG,
            _ => 0,
        }
    }

    /// LLStartUp's reading of the login reply: `agent_access_max` sets the
    /// ceiling and, for grids that send nothing else (FIRE-8854), the
    /// preference; `agent_region_access` then sets the preference, which
    /// LLAgentAccess::setMaturity keeps within the ceiling.
    pub fn from_login(access_max: &str, region_access: &str) -> AgentAccess {
        let mut a = AgentAccess::default();
        if !access_max.is_empty() {
            a.max = Self::from_text(access_max);
            a.preferred = a.max;
        }
        if !region_access.is_empty() {
            a.preferred = Self::from_text(region_access);
        }
        // canSetMaturity: adult accounts choose freely, the others fall back
        if a.max < SIM_ACCESS_ADULT && a.preferred >= SIM_ACCESS_ADULT {
            a.preferred = SIM_ACCESS_MATURE;
        }
        if a.max < SIM_ACCESS_MATURE && a.preferred >= SIM_ACCESS_MATURE {
            a.preferred = SIM_ACCESS_PG;
        }
        a
    }

    pub fn prefers_mature(&self) -> bool {
        self.preferred >= SIM_ACCESS_MATURE
    }

    pub fn prefers_adult(&self) -> bool {
        self.preferred >= SIM_ACCESS_ADULT
    }

    /// wantsPGOnly: prefersPG || isTeen (no god mode in Aurora).
    pub fn wants_pg_only(&self) -> bool {
        !self.prefers_mature() || self.max < SIM_ACCESS_MATURE
    }

    /// canAccessMature: prefersMature && !isTeen.
    pub fn can_access_mature(&self) -> bool {
        self.prefers_mature() && self.max >= SIM_ACCESS_MATURE
    }

    /// canAccessAdult: prefersAdult && isAdult.
    pub fn can_access_adult(&self) -> bool {
        self.prefers_adult() && self.max >= SIM_ACCESS_ADULT
    }
}

/// The G / M / A boxes of a tab (ShowPG*, ShowMature*, ShowAdult*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratings {
    pub pg: bool,
    pub mature: bool,
    pub adult: bool,
}

impl Default for Ratings {
    /// settings.xml: general content only.
    fn default() -> Self {
        Ratings {
            pg: true,
            mature: false,
            adult: false,
        }
    }
}

impl Ratings {
    pub fn any(&self) -> bool {
        self.pg || self.mature || self.adult
    }
}

/// Why a search is not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    /// "search_short": nothing left once the short words are dropped.
    TooShort,
    /// NoContentToSearch: no rating box is ticked.
    NoContent,
}

/// One directory query, as it goes on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum DirQuery {
    /// DirFindQuery: people, groups and events (told apart by the flags).
    Find { text: String, flags: u32 },
    /// DirPlacesQuery.
    Places { text: String, flags: u32, category: i8 },
    /// DirLandQuery.
    Land {
        flags: u32,
        search_type: u32,
        price: i32,
        area: i32,
    },
    /// DirClassifiedQuery.
    Classifieds { text: String, flags: u32, category: u32 },
}

/// filterShortWords: the words longer than MIN_SEARCH_STRING_SIZE, and
/// whether some were dropped (SeachFilteredOnShortWords). As in Firestorm a
/// space follows every kept word that is not the last word typed, so a
/// dropped last word leaves a trailing space.
pub fn filter_short_words(query: &str) -> (String, bool) {
    let words: Vec<&str> = query.split(' ').filter(|w| !w.is_empty()).collect();
    let mut out = String::new();
    let mut filtered = false;
    for (i, w) in words.iter().enumerate() {
        if w.len() > MIN_SEARCH_STRING_SIZE {
            out.push_str(w);
            if i + 1 != words.len() {
                out.push(' ');
            }
        } else {
            filtered = true;
        }
    }
    (out, filtered)
}

/// What a people search does with the text typed.
#[derive(Debug, Clone, PartialEq)]
pub enum PeopleSearch {
    /// A key was typed: the name cache answers, not the directory.
    Key(Uuid),
    Query(DirQuery),
}

/// FSPanelSearchPeople::find: the trimmed text must be longer than
/// MIN_SEARCH_STRING_SIZE; a UUID is looked up by the name cache; dots become
/// spaces ("first.last").
pub fn people_query(text: &str) -> Result<PeopleSearch, QueryError> {
    let text = text.trim();
    if text.len() <= MIN_SEARCH_STRING_SIZE {
        return Err(QueryError::TooShort);
    }
    // LLUUID::validate: the 36-character form (or 32 hex digits)
    if (text.len() == 36 || text.len() == 32)
        && let Ok(id) = Uuid::parse_str(text)
    {
        return Ok(PeopleSearch::Key(id));
    }
    Ok(PeopleSearch::Query(DirQuery::Find {
        text: text.replace('.', " "),
        flags: dfq::PEOPLE,
    }))
}

/// The rating scope shared by places, land and events: PG_SIMS_ONLY for an
/// agent who wants general content only, then each ticked rating the agent
/// may access.
fn scope(access: AgentAccess, inc: Ratings) -> u32 {
    let mut s = 0;
    if access.wants_pg_only() {
        s |= dfq::PG_SIMS_ONLY;
    }
    if inc.pg {
        s |= dfq::INC_PG;
    }
    if inc.mature && access.can_access_mature() {
        s |= dfq::INC_MATURE;
    }
    if inc.adult && access.can_access_adult() {
        s |= dfq::INC_ADULT;
    }
    s
}

/// FSPanelSearchGroups::find. `maturity` is FSSearchGroupMaturity: the
/// SIM_ACCESS_* level up to which groups are included (0: none).
pub fn groups_query(text: &str, maturity: u8, access: AgentAccess) -> Result<DirQuery, QueryError> {
    let (text, _) = filter_short_words(text);
    if text.is_empty() {
        return Err(QueryError::TooShort);
    }
    if maturity == 0 {
        return Err(QueryError::NoContent);
    }
    let mut flags = dfq::GROUPS;
    if access.wants_pg_only() {
        flags |= dfq::PG_SIMS_ONLY;
    }
    if maturity >= SIM_ACCESS_PG {
        flags |= dfq::INC_PG;
    }
    if access.can_access_mature() && maturity >= SIM_ACCESS_MATURE {
        flags |= dfq::INC_MATURE;
    }
    if access.can_access_adult() && maturity >= SIM_ACCESS_ADULT {
        flags |= dfq::INC_ADULT;
    }
    Ok(DirQuery::Find { text, flags })
}

/// FSPanelSearchPlaces::find: sorted by traffic, `category` an
/// LLParcel::ECategory or PLACE_CATEGORY_ANY.
pub fn places_query(text: &str, inc: Ratings, category: i8, access: AgentAccess) -> Result<DirQuery, QueryError> {
    let (text, _) = filter_short_words(text);
    if text.is_empty() {
        return Err(QueryError::TooShort);
    }
    if !inc.any() {
        return Err(QueryError::NoContent);
    }
    Ok(DirQuery::Places {
        text,
        flags: scope(access, inc) | dfq::DWELL_SORT,
        category,
    })
}

/// "land_category" of the land sales tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LandKind {
    #[default]
    All,
    Auction,
    Mainland,
    Estate,
}

/// "land_sort_combo" of the land sales tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LandSort {
    #[default]
    Name,
    Price,
    PerMeter,
    Area,
}

impl LandKind {
    /// FindLandType as Aurora saves it: 0 all, 1 auction, 2 mainland,
    /// 3 estate.
    pub fn from_setting(value: u8) -> LandKind {
        match value {
            1 => LandKind::Auction,
            2 => LandKind::Mainland,
            3 => LandKind::Estate,
            _ => LandKind::All,
        }
    }

    pub fn setting(self) -> u8 {
        match self {
            LandKind::All => 0,
            LandKind::Auction => 1,
            LandKind::Mainland => 2,
            LandKind::Estate => 3,
        }
    }
}

/// The filters of a land sales search, as FSPanelSearchLand::find reads its
/// controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandFilter {
    pub kind: LandKind,
    pub sort: LandSort,
    pub ascending: bool,
    /// "price_check" (FindLandPrice): limit by price.
    pub limit_price: bool,
    /// "area_check" (FindLandArea): limit by area.
    pub limit_area: bool,
    /// "edit_price" as an integer, 0 for an empty field; sent whether its
    /// box is ticked or not.
    pub price: i32,
    /// "edit_area" as an integer, 0 for an empty field.
    pub area: i32,
}

impl Default for LandFilter {
    /// The panel as Firestorm first shows it: every type, sorted by name
    /// (the first entry of "land_sort_combo"), descending, FindLandPrice on
    /// and FindLandArea off (settings.xml), both fields empty.
    fn default() -> Self {
        LandFilter {
            kind: LandKind::All,
            sort: LandSort::Name,
            ascending: false,
            limit_price: true,
            limit_area: false,
            price: 0,
            area: 0,
        }
    }
}

/// FSPanelSearchLand::find.
pub fn land_query(inc: Ratings, filter: LandFilter, access: AgentAccess) -> Result<DirQuery, QueryError> {
    if !inc.any() {
        return Err(QueryError::NoContent);
    }
    let search_type = match filter.kind {
        LandKind::All => sell_type::ALL,
        LandKind::Auction => sell_type::AUCTION,
        LandKind::Mainland => sell_type::MAINLAND,
        LandKind::Estate => sell_type::ESTATE,
    };
    let mut flags = scope(access, inc);
    flags |= match filter.sort {
        LandSort::Name => dfq::NAME_SORT,
        LandSort::Price => dfq::PRICE_SORT,
        LandSort::PerMeter => dfq::PER_METER_SORT,
        LandSort::Area => dfq::AREA_SORT,
    };
    if filter.ascending {
        flags |= dfq::SORT_ASC;
    }
    if filter.limit_price {
        flags |= dfq::LIMIT_BY_PRICE;
    }
    if filter.limit_area {
        flags |= dfq::LIMIT_BY_AREA;
    }
    // the values of both fields go out whatever their boxes say
    Ok(DirQuery::Land {
        flags,
        search_type,
        price: filter.price,
        area: filter.area,
    })
}

/// FSPanelSearchLand::processSearchReply filters the rows again on the
/// viewer ("use_price && price > limit_price", "use_area && area <
/// limit_area"): with the price box ticked and its field empty, only rows
/// priced 0 stay, which is what auctions are listed at.
pub fn land_row_kept(filter: &LandFilter, price: i32, area: i32) -> bool {
    !(filter.limit_price && price > filter.price) && !(filter.limit_area && area < filter.area)
}

/// "events_search_mode": events going on and to come, or those of a day
/// counted from today (Pacific time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventsMode {
    #[default]
    Current,
    Day(i32),
}

/// FSPanelSearchEvents::find: the query text is "u|category|text" or
/// "day|category|text"; an empty text lists every event.
pub fn events_query(text: &str, inc: Ratings, mode: EventsMode, category: u32, access: AgentAccess) -> Result<DirQuery, QueryError> {
    let (text, _) = filter_short_words(text);
    if !inc.any() {
        return Err(QueryError::NoContent);
    }
    let when = match mode {
        EventsMode::Current => "u".to_owned(),
        EventsMode::Day(day) => day.to_string(),
    };
    Ok(DirQuery::Find {
        text: format!("{when}|{category}|{text}"),
        flags: dfq::DATE_EVENTS | scope(access, inc),
    })
}

/// FSPanelSearchEvents::processSearchReply: an event without owner is
/// skipped, as is one whose rating box is not ticked.
pub fn event_row_kept(inc: Ratings, owner: Uuid, flags: u32) -> bool {
    if owner.is_nil() {
        return false;
    }
    if flags & (event_flags::ADULT | event_flags::MATURE) == event_flags::NONE && !inc.pg {
        return false;
    }
    if flags & event_flags::MATURE != 0 && !inc.mature {
        return false;
    }
    !(flags & event_flags::ADULT != 0 && !inc.adult)
}

/// pack_classified_flags_request (llclassifiedflags.cpp), without auto-renew.
pub fn classified_query_flags(inc: Ratings) -> u32 {
    const FLAG_MATURE: u32 = 1 << 1;
    const QUERY_INC_PG: u32 = 1 << 2;
    const QUERY_INC_MATURE: u32 = 1 << 3;
    const QUERY_INC_ADULT: u32 = 1 << 6;
    let mut f = 0;
    if inc.pg {
        f |= QUERY_INC_PG;
    }
    if inc.mature {
        f |= QUERY_INC_MATURE;
    }
    if inc.pg && !inc.mature {
        f |= FLAG_MATURE;
    }
    if inc.adult {
        f |= QUERY_INC_ADULT;
    }
    f
}

/// FSPanelSearchClassifieds::find; `category` 0 is every category.
pub fn classifieds_query(text: &str, inc: Ratings, category: u32) -> Result<DirQuery, QueryError> {
    let (text, _) = filter_short_words(text);
    if text.is_empty() {
        return Err(QueryError::TooShort);
    }
    if !inc.any() {
        return Err(QueryError::NoContent);
    }
    Ok(DirQuery::Classifieds {
        text,
        flags: classified_query_flags(inc),
        category,
    })
}

/// The message of a query.
#[derive(Debug, Clone, PartialEq)]
pub enum DirMessage {
    Find(msgs::DirFindQuery),
    Places(msgs::DirPlacesQuery),
    Land(msgs::DirLandQuery),
    Classifieds(msgs::DirClassifiedQuery),
}

/// The message Firestorm builds in `find()`: `start` is mStartSearch (first
/// row of the page, never negative).
pub fn message(agent: Uuid, session: Uuid, query_id: Uuid, start: i32, query: &DirQuery) -> DirMessage {
    let start = start.max(0);
    match query {
        DirQuery::Find { text, flags } => {
            let mut m = msgs::DirFindQuery::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.query_data.query_id = query_id;
            m.query_data.query_text = aurora_msg::str_field(text);
            m.query_data.query_flags = *flags;
            m.query_data.query_start = start;
            DirMessage::Find(m)
        }
        DirQuery::Places { text, flags, category } => {
            let mut m = msgs::DirPlacesQuery::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.query_data.query_id = query_id;
            m.query_data.query_text = aurora_msg::str_field(text);
            m.query_data.query_flags = *flags;
            m.query_data.category = *category;
            // "TODO: Search filter by region name." in Firestorm
            m.query_data.sim_name = aurora_msg::str_field("");
            m.query_data.query_start = start;
            DirMessage::Places(m)
        }
        DirQuery::Land {
            flags,
            search_type,
            price,
            area,
        } => {
            let mut m = msgs::DirLandQuery::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.query_data.query_id = query_id;
            m.query_data.query_flags = *flags;
            m.query_data.search_type = *search_type;
            m.query_data.price = *price;
            m.query_data.area = *area;
            m.query_data.query_start = start;
            DirMessage::Land(m)
        }
        DirQuery::Classifieds { text, flags, category } => {
            let mut m = msgs::DirClassifiedQuery::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.query_data.query_id = query_id;
            m.query_data.query_text = aurora_msg::str_field(text);
            m.query_data.query_flags = *flags;
            m.query_data.category = *category;
            m.query_data.query_start = start;
            DirMessage::Classifieds(m)
        }
    }
}

/// A resident found (DirPeopleReply).
#[derive(Debug, Clone, PartialEq)]
pub struct PersonRow {
    pub id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub online: bool,
}

/// A group found (DirGroupsReply).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRow {
    pub id: Uuid,
    pub name: String,
    pub members: i32,
    pub search_order: f32,
}

/// A place found (DirPlacesReply).
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceRow {
    pub parcel: Uuid,
    pub name: String,
    pub for_sale: bool,
    pub auction: bool,
    pub dwell: f32,
}

/// A parcel for sale (DirLandReply).
#[derive(Debug, Clone, PartialEq)]
pub struct LandRow {
    pub parcel: Uuid,
    pub name: String,
    pub auction: bool,
    pub for_sale: bool,
    pub price: i32,
    pub area: i32,
    /// ProductSKU: the land type, named by ProductInfoRequest.
    pub sku: String,
}

/// An event found (DirEventsReply).
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub owner: Uuid,
    pub name: String,
    pub id: u32,
    /// The date as the directory writes it (Pacific time).
    pub date: String,
    pub unix_time: u32,
    pub flags: u32,
}

/// A classified found (DirClassifiedReply).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassifiedRow {
    pub id: Uuid,
    pub name: String,
    pub flags: u8,
    pub creation_date: u32,
    pub expiration_date: u32,
    pub price: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DirRows {
    People(Vec<PersonRow>),
    Groups(Vec<GroupRow>),
    Places(Vec<PlaceRow>),
    Land(Vec<LandRow>),
    Events(Vec<EventRow>),
    Classifieds(Vec<ClassifiedRow>),
}

impl DirRows {
    pub fn len(&self) -> usize {
        match self {
            DirRows::People(r) => r.len(),
            DirRows::Groups(r) => r.len(),
            DirRows::Places(r) => r.len(),
            DirRows::Land(r) => r.len(),
            DirRows::Events(r) => r.len(),
            DirRows::Classifieds(r) => r.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One reply packet of a query (a page comes in several).
#[derive(Debug, Clone, PartialEq)]
pub struct DirReply {
    pub query_id: Uuid,
    /// First StatusData block, when the reply has one.
    pub status: Option<u32>,
    pub rows: DirRows,
}

pub fn people_reply(m: &msgs::DirPeopleReply) -> DirReply {
    DirReply {
        query_id: m.query_data.query_id,
        status: None,
        rows: DirRows::People(
            m.query_replies
                .iter()
                .map(|r| PersonRow {
                    id: r.agent_id,
                    first_name: field_str(&r.first_name),
                    last_name: field_str(&r.last_name),
                    online: r.online,
                })
                .collect(),
        ),
    }
}

pub fn groups_reply(m: &msgs::DirGroupsReply) -> DirReply {
    DirReply {
        query_id: m.query_data.query_id,
        status: None,
        rows: DirRows::Groups(
            m.query_replies
                .iter()
                .map(|r| GroupRow {
                    id: r.group_id,
                    name: field_str(&r.group_name),
                    members: r.members,
                    search_order: r.search_order,
                })
                .collect(),
        ),
    }
}

/// None when the reply names no query (its QueryData block is variable).
pub fn places_reply(m: &msgs::DirPlacesReply) -> Option<DirReply> {
    Some(DirReply {
        query_id: m.query_data.first()?.query_id,
        status: m.status_data.first().map(|s| s.status),
        rows: DirRows::Places(
            m.query_replies
                .iter()
                .map(|r| PlaceRow {
                    parcel: r.parcel_id,
                    name: field_str(&r.name),
                    for_sale: r.for_sale,
                    auction: r.auction,
                    dwell: r.dwell,
                })
                .collect(),
        ),
    })
}

/// The UDP form, without ProductSKU (grids that still send it).
pub fn land_reply(m: &msgs::DirLandReply) -> DirReply {
    DirReply {
        query_id: m.query_data.query_id,
        status: None,
        rows: DirRows::Land(
            m.query_replies
                .iter()
                .map(|r| LandRow {
                    parcel: r.parcel_id,
                    name: field_str(&r.name),
                    auction: r.auction,
                    for_sale: r.for_sale,
                    price: r.sale_price,
                    area: r.actual_area,
                    sku: String::new(),
                })
                .collect(),
        ),
    }
}

/// DirLandReply as Second Life sends it, through the event queue
/// (UDPDeprecated in the template): the same blocks as LLSD, plus ProductSKU.
/// The agent the reply is for, then the reply.
pub fn land_reply_llsd(b: &Llsd) -> (Uuid, DirReply) {
    let rows = b["QueryReplies"]
        .as_array()
        .iter()
        .map(|r| LandRow {
            parcel: r["ParcelID"].as_uuid(),
            name: r["Name"].to_string_value(),
            auction: r["Auction"].as_bool(),
            for_sale: r["ForSale"].as_bool(),
            price: r["SalePrice"].as_i32(),
            area: r["ActualArea"].as_i32(),
            sku: r["ProductSKU"].to_string_value(),
        })
        .collect();
    (
        b["AgentData"][0]["AgentID"].as_uuid(),
        DirReply {
            query_id: b["QueryData"][0]["QueryID"].as_uuid(),
            status: None,
            rows: DirRows::Land(rows),
        },
    )
}

pub fn events_reply(m: &msgs::DirEventsReply) -> DirReply {
    DirReply {
        query_id: m.query_data.query_id,
        status: m.status_data.first().map(|s| s.status),
        rows: DirRows::Events(
            m.query_replies
                .iter()
                .map(|r| EventRow {
                    owner: r.owner_id,
                    name: field_str(&r.name),
                    id: r.event_id,
                    date: field_str(&r.date),
                    unix_time: r.unix_time,
                    flags: r.event_flags,
                })
                .collect(),
        ),
    }
}

pub fn classifieds_reply(m: &msgs::DirClassifiedReply) -> DirReply {
    DirReply {
        query_id: m.query_data.query_id,
        status: m.status_data.first().map(|s| s.status),
        rows: DirRows::Classifieds(
            m.query_replies
                .iter()
                .map(|r| ClassifiedRow {
                    id: r.classified_id,
                    name: field_str(&r.name),
                    flags: r.classified_flags,
                    creation_date: r.creation_date,
                    expiration_date: r.expiration_date,
                    price: r.price_for_listing,
                })
                .collect(),
        ),
    }
}

/// An event (EventInfoReply, LLEventInfo).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventInfo {
    pub id: u32,
    /// mRunByID: the organizer.
    pub creator: Uuid,
    pub name: String,
    pub category: String,
    pub desc: String,
    /// The date as the directory writes it (Pacific time).
    pub date: String,
    pub date_utc: u32,
    /// Minutes.
    pub duration: u32,
    /// The entry fee, when there is one (mHasCover / mCover).
    pub cover: Option<u32>,
    pub sim_name: String,
    pub global: glam::DVec3,
    pub flags: u32,
}

/// LLEventInfo::unpack.
pub fn event_info(m: &msgs::EventInfoReply) -> EventInfo {
    let d = &m.event_data;
    EventInfo {
        id: d.event_id,
        creator: Uuid::parse_str(field_str(&d.creator).trim()).unwrap_or_default(),
        name: field_str(&d.name),
        category: field_str(&d.category),
        desc: field_str(&d.desc),
        date: field_str(&d.date),
        date_utc: d.date_utc,
        duration: d.duration,
        cover: (d.cover > 0).then_some(d.amount),
        sim_name: field_str(&d.sim_name),
        global: d.global_pos,
        flags: d.event_flags,
    }
}

/// A group seen from outside (GroupProfileReply, LLGroupMgrGroupData).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupProfile {
    pub id: Uuid,
    pub name: String,
    pub charter: String,
    pub show_in_list: bool,
    /// Our title in the group, when we are a member.
    pub member_title: String,
    pub powers: u64,
    pub insignia: Uuid,
    pub founder: Uuid,
    pub membership_fee: i32,
    pub open_enrollment: bool,
    pub members: i32,
    /// With the "everyone" role, as LLGroupMgr counts them.
    pub roles: i32,
    pub allow_publish: bool,
    pub mature_publish: bool,
    pub owner_role: Uuid,
}

/// LLGroupMgr::processGroupPropertiesReply.
pub fn group_profile(m: &msgs::GroupProfileReply) -> GroupProfile {
    let d = &m.group_data;
    GroupProfile {
        id: d.group_id,
        name: field_str(&d.name),
        charter: field_str(&d.charter),
        show_in_list: d.show_in_list,
        member_title: field_str(&d.member_title),
        powers: d.powers_mask,
        insignia: d.insignia_id,
        founder: d.founder_id,
        membership_fee: d.membership_fee,
        open_enrollment: d.open_enrollment,
        members: d.group_membership_count,
        roles: d.group_roles_count + 1,
        allow_publish: d.allow_publish,
        mature_publish: d.mature_publish,
        owner_role: d.owner_role,
    }
}

/// ProductInfoRequest reply (LLProductInfoRequestManager): an array of maps
/// with "sku", "name" and "description"; (sku, description) of each.
pub fn product_info(v: &Llsd) -> Vec<(String, String)> {
    v.as_array()
        .iter()
        .map(|e| (e["sku"].to_string_value(), e["description"].to_string_value()))
        .filter(|(sku, _)| !sku.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    const ADULT: AgentAccess = AgentAccess {
        max: SIM_ACCESS_ADULT,
        preferred: SIM_ACCESS_ADULT,
    };
    const ALL: Ratings = Ratings {
        pg: true,
        mature: true,
        adult: true,
    };

    #[test]
    fn flag_values_are_those_of_llqueryflags() {
        assert_eq!(dfq::PEOPLE, 0x1);
        assert_eq!(dfq::GROUPS, 0x10);
        assert_eq!(dfq::DATE_EVENTS, 0x20);
        assert_eq!(dfq::DWELL_SORT, 0x400);
        assert_eq!(dfq::PG_SIMS_ONLY, 0x800);
        assert_eq!(dfq::SORT_ASC, 0x8000);
        assert_eq!(dfq::PRICE_SORT, 0x1_0000);
        assert_eq!(dfq::PER_METER_SORT, 0x2_0000);
        assert_eq!(dfq::AREA_SORT, 0x4_0000);
        assert_eq!(dfq::NAME_SORT, 0x8_0000);
        assert_eq!(dfq::LIMIT_BY_PRICE, 0x10_0000);
        assert_eq!(dfq::LIMIT_BY_AREA, 0x20_0000);
        assert_eq!(dfq::INC_PG | dfq::INC_MATURE | dfq::INC_ADULT, 0x700_0000);
        assert_eq!((sell_type::AUCTION, sell_type::MAINLAND, sell_type::ESTATE), (2, 8, 16));
    }

    #[test]
    fn access_follows_the_login_reply() {
        // nothing sent: general content only
        let a = AgentAccess::from_login("", "");
        assert!(a.wants_pg_only() && !a.can_access_mature() && !a.can_access_adult());
        // an adult account that prefers moderate content
        let a = AgentAccess::from_login("A", "M");
        assert!(!a.wants_pg_only() && a.can_access_mature() && !a.can_access_adult());
        assert!(a.prefers_mature() && !a.prefers_adult());
        // FIRE-8854: only the ceiling is sent, it is the preference too
        let a = AgentAccess::from_login("A", "");
        assert!(a.can_access_adult());
        // the preference never exceeds the ceiling
        let a = AgentAccess::from_login("M", "A");
        assert_eq!(a.preferred, SIM_ACCESS_MATURE);
        let a = AgentAccess::from_login("PG", "A");
        assert!(a.wants_pg_only() && !a.can_access_mature());
    }

    #[test]
    fn short_words_are_dropped_like_firestorm() {
        assert_eq!(filter_short_words("aurora viewer"), ("aurora viewer".to_owned(), false));
        assert_eq!(filter_short_words("le club du loup"), ("club loup".to_owned(), true));
        // a dropped last word leaves the space after the previous one
        assert_eq!(filter_short_words("club du"), ("club ".to_owned(), true));
        assert_eq!(filter_short_words("  club   loup "), ("club loup".to_owned(), false));
        assert_eq!(filter_short_words("a de"), (String::new(), true));
        assert_eq!(filter_short_words(""), (String::new(), false));
    }

    #[test]
    fn people_text() {
        assert_eq!(people_query(" ab "), Err(QueryError::TooShort));
        assert_eq!(
            people_query(" nova.example "),
            Ok(PeopleSearch::Query(DirQuery::Find {
                text: "nova example".into(),
                flags: dfq::PEOPLE
            }))
        );
        let id = Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        assert_eq!(people_query(&id.to_string()), Ok(PeopleSearch::Key(id)));
    }

    #[test]
    fn groups_flags() {
        let q = |maturity, access| match groups_query("loups", maturity, access) {
            Ok(DirQuery::Find { flags, .. }) => flags,
            other => panic!("{other:?}"),
        };
        assert_eq!(q(SIM_ACCESS_PG, ADULT), dfq::GROUPS | dfq::INC_PG);
        assert_eq!(q(SIM_ACCESS_MATURE, ADULT), dfq::GROUPS | dfq::INC_PG | dfq::INC_MATURE);
        assert_eq!(q(SIM_ACCESS_ADULT, ADULT), dfq::GROUPS | 0x700_0000);
        // an agent limited to general content never asks for more
        assert_eq!(
            q(SIM_ACCESS_ADULT, AgentAccess::default()),
            dfq::GROUPS | dfq::PG_SIMS_ONLY | dfq::INC_PG
        );
        assert_eq!(groups_query("loups", 0, ADULT), Err(QueryError::NoContent));
        assert_eq!(groups_query("de la", SIM_ACCESS_PG, ADULT), Err(QueryError::TooShort));
    }

    #[test]
    fn places_and_classifieds_queries() {
        assert_eq!(
            places_query("plage", Ratings::default(), PLACE_CATEGORY_ANY, ADULT),
            Ok(DirQuery::Places {
                text: "plage".into(),
                flags: dfq::INC_PG | dfq::DWELL_SORT,
                category: -1
            })
        );
        let none = Ratings {
            pg: false,
            mature: false,
            adult: false,
        };
        assert_eq!(places_query("plage", none, 3, ADULT), Err(QueryError::NoContent));
        assert_eq!(places_query("le", ALL, 3, ADULT), Err(QueryError::TooShort));
        // pack_classified_flags_request: PG alone also sets the mature filter bit
        assert_eq!(classified_query_flags(Ratings::default()), (1 << 2) | (1 << 1));
        assert_eq!(classified_query_flags(ALL), (1 << 2) | (1 << 3) | (1 << 6));
        assert_eq!(
            classifieds_query("meubles", ALL, 4),
            Ok(DirQuery::Classifieds {
                text: "meubles".into(),
                flags: 0x4C,
                category: 4
            })
        );
    }

    #[test]
    fn land_flags_and_viewer_filter() {
        // Firestorm's defaults: the price box ticked over an empty field
        let default = LandFilter::default();
        assert_eq!(
            land_query(Ratings::default(), default, ADULT),
            Ok(DirQuery::Land {
                flags: dfq::INC_PG | dfq::NAME_SORT | dfq::LIMIT_BY_PRICE,
                search_type: sell_type::ALL,
                price: 0,
                area: 0
            })
        );
        // only what costs nothing passes the viewer's own filter
        assert!(land_row_kept(&default, 0, 16));
        assert!(!land_row_kept(&default, 1, 65_536));
        // the four combinations of the two boxes; the fields are always sent
        for (limit_price, limit_area, expected) in [
            (false, false, 0),
            (true, false, dfq::LIMIT_BY_PRICE),
            (false, true, dfq::LIMIT_BY_AREA),
            (true, true, dfq::LIMIT_BY_PRICE | dfq::LIMIT_BY_AREA),
        ] {
            let f = LandFilter {
                limit_price,
                limit_area,
                price: 2500,
                area: 1024,
                ..Default::default()
            };
            assert_eq!(
                land_query(Ratings::default(), f, ADULT),
                Ok(DirQuery::Land {
                    flags: dfq::INC_PG | dfq::NAME_SORT | expected,
                    search_type: sell_type::ALL,
                    price: 2500,
                    area: 1024
                })
            );
            assert_eq!(land_row_kept(&f, 2501, 2048), !limit_price);
            assert_eq!(land_row_kept(&f, 100, 1023), !limit_area);
            assert!(land_row_kept(&f, 2500, 1024));
        }
        let f = LandFilter {
            kind: LandKind::Mainland,
            sort: LandSort::PerMeter,
            ascending: true,
            limit_price: true,
            limit_area: true,
            price: 5000,
            area: 512,
        };
        assert_eq!(
            land_query(ALL, f, ADULT),
            Ok(DirQuery::Land {
                flags: 0x700_0000 | dfq::PER_METER_SORT | dfq::SORT_ASC | dfq::LIMIT_BY_PRICE | dfq::LIMIT_BY_AREA,
                search_type: sell_type::MAINLAND,
                price: 5000,
                area: 512
            })
        );
        assert!(land_row_kept(&f, 5000, 512));
        assert!(!land_row_kept(&f, 5001, 512));
        assert!(!land_row_kept(&f, 100, 511));
        for kind in [LandKind::All, LandKind::Auction, LandKind::Mainland, LandKind::Estate] {
            assert_eq!(LandKind::from_setting(kind.setting()), kind);
        }
    }

    #[test]
    fn events_text_and_scope() {
        assert_eq!(
            events_query("", Ratings::default(), EventsMode::Current, 0, ADULT),
            Ok(DirQuery::Find {
                text: "u|0|".into(),
                flags: dfq::DATE_EVENTS | dfq::INC_PG
            })
        );
        assert_eq!(
            events_query("live music", ALL, EventsMode::Day(-1), 20, AgentAccess::from_login("M", "M")),
            Ok(DirQuery::Find {
                text: "-1|20|live music".into(),
                flags: dfq::DATE_EVENTS | dfq::INC_PG | dfq::INC_MATURE
            })
        );
        let owner = Uuid::from_u128(1);
        assert!(event_row_kept(Ratings::default(), owner, event_flags::NONE));
        assert!(!event_row_kept(Ratings::default(), owner, event_flags::MATURE));
        assert!(!event_row_kept(Ratings::default(), Uuid::nil(), event_flags::NONE));
        assert!(event_row_kept(ALL, owner, event_flags::ADULT));
        let no_pg = Ratings {
            pg: false,
            mature: true,
            adult: false,
        };
        assert!(!event_row_kept(no_pg, owner, event_flags::NONE));
    }

    #[test]
    fn messages_carry_the_query() {
        let (agent, session, id) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let q = DirQuery::Find {
            text: "nova".into(),
            flags: dfq::PEOPLE,
        };
        let DirMessage::Find(m) = message(agent, session, id, 100, &q) else {
            panic!("find");
        };
        assert_eq!((m.agent_data.agent_id, m.agent_data.session_id), (agent, session));
        assert_eq!(m.query_data.query_id, id);
        assert_eq!(m.query_data.query_text, b"nova\0");
        assert_eq!((m.query_data.query_flags, m.query_data.query_start), (dfq::PEOPLE, 100));
        // mStartSearch never goes below zero
        let q = DirQuery::Places {
            text: "plage".into(),
            flags: 7,
            category: -1,
        };
        let DirMessage::Places(m) = message(agent, session, id, -100, &q) else {
            panic!("places");
        };
        assert_eq!((m.query_data.category, m.query_data.query_start), (-1, 0));
        assert_eq!(m.query_data.sim_name, b"\0");
        let q = DirQuery::Land {
            flags: 9,
            search_type: sell_type::ESTATE,
            price: 10,
            area: 20,
        };
        let DirMessage::Land(m) = message(agent, session, id, 200, &q) else {
            panic!("land");
        };
        let d = &m.query_data;
        assert_eq!(
            (d.query_flags, d.search_type, d.price, d.area, d.query_start),
            (9, sell_type::ESTATE, 10, 20, 200)
        );
        let q = DirQuery::Classifieds {
            text: "meubles".into(),
            flags: 0x4C,
            category: 4,
        };
        let DirMessage::Classifieds(m) = message(agent, session, id, 0, &q) else {
            panic!("classifieds");
        };
        assert_eq!((m.query_data.query_flags, m.query_data.category), (0x4C, 4));
    }

    #[test]
    fn replies_are_read() {
        let mut m = msgs::DirPlacesReply::default();
        assert!(places_reply(&m).is_none(), "no QueryData block");
        m.query_data.push(msgs::dir_places_reply::QueryData {
            query_id: Uuid::from_u128(9),
        });
        m.query_replies.push(msgs::dir_places_reply::QueryReplies {
            parcel_id: Uuid::from_u128(4),
            name: b"Plage\0".to_vec(),
            for_sale: true,
            auction: false,
            dwell: 1234.6,
        });
        m.status_data.push(msgs::dir_places_reply::StatusData {
            status: status::FOUND_NONE,
        });
        let r = places_reply(&m).expect("reply");
        assert_eq!((r.query_id, r.status), (Uuid::from_u128(9), Some(status::FOUND_NONE)));
        assert_eq!(
            r.rows,
            DirRows::Places(vec![PlaceRow {
                parcel: Uuid::from_u128(4),
                name: "Plage".into(),
                for_sale: true,
                auction: false,
                dwell: 1234.6
            }])
        );
        let mut m = msgs::EventInfoReply::default();
        m.event_data.event_id = 77;
        m.event_data.creator = b"00000000-0000-0000-0000-000000000005\0".to_vec();
        m.event_data.cover = 1;
        m.event_data.amount = 50;
        m.event_data.duration = 90;
        let e = event_info(&m);
        assert_eq!((e.id, e.creator, e.cover, e.duration), (77, Uuid::from_u128(5), Some(50), 90));
        m.event_data.cover = 0;
        assert_eq!(event_info(&m).cover, None);
        let mut m = msgs::GroupProfileReply::default();
        m.group_data.group_roles_count = 3;
        assert_eq!(group_profile(&m).roles, 4, "the everyone role is counted");
    }

    #[test]
    fn land_reply_of_the_event_queue() {
        let agent = Uuid::from_u128(1);
        let b = llsd_map! {
            "AgentData" => Llsd::Array(vec![llsd_map! { "AgentID" => Llsd::Uuid(agent) }]),
            "QueryData" => Llsd::Array(vec![llsd_map! { "QueryID" => Llsd::Uuid(Uuid::from_u128(2)) }]),
            "QueryReplies" => Llsd::Array(vec![llsd_map! {
                "ParcelID" => Llsd::Uuid(Uuid::from_u128(3)),
                "Name" => "Terrain en bord de mer",
                "Auction" => false,
                "ForSale" => true,
                "SalePrice" => 2500,
                "ActualArea" => 1024,
                "ProductSKU" => "023",
            }]),
        };
        let (to, r) = land_reply_llsd(&b);
        assert_eq!((to, r.query_id), (agent, Uuid::from_u128(2)));
        assert_eq!(
            r.rows,
            DirRows::Land(vec![LandRow {
                parcel: Uuid::from_u128(3),
                name: "Terrain en bord de mer".into(),
                auction: false,
                for_sale: true,
                price: 2500,
                area: 1024,
                sku: "023".into()
            }])
        );
        let info = Llsd::Array(vec![
            llsd_map! { "sku" => "023", "name" => "Mainland", "description" => "Mainland / Full Region" },
            llsd_map! { "name" => "no sku" },
        ]);
        assert_eq!(product_info(&info), vec![("023".to_owned(), "Mainland / Full Region".to_owned())]);
    }
}
