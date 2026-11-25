//! What the window draws from the results: the kinds' three states, the
//! distance chart, the distance bands and the owners. Pure: the rows come
//! from `Table`.
use super::table::{Filters, Row};
use std::collections::HashMap;
use uuid::Uuid;

/// A kind of object the filter sheet can require or hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Locked,
    Physical,
    Phantom,
    Temporary,
    Attached,
    Media,
    Probe,
    Sale,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum KindState {
    #[default]
    Any,
    Only,
    Hidden,
}

impl KindState {
    /// A click on the tile: only these, then none of these, then any.
    pub fn next(self) -> Self {
        match self {
            Self::Any => Self::Only,
            Self::Only => Self::Hidden,
            Self::Hidden => Self::Any,
        }
    }
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::Locked,
        Self::Physical,
        Self::Phantom,
        Self::Temporary,
        Self::Attached,
        Self::Media,
        Self::Probe,
        Self::Sale,
    ];

    pub fn state(self, f: &Filters) -> KindState {
        let (only, hidden) = match self {
            Self::Locked => (f.locked, f.exclude_locked),
            Self::Physical => (f.physical, f.exclude_physical),
            Self::Phantom => (f.phantom, f.exclude_phantom),
            Self::Temporary => (f.temporary, f.exclude_temporary),
            Self::Attached => (f.only_attachments, !f.include_attachments),
            Self::Media => (f.media, f.exclude_media),
            Self::Probe => (f.probe, f.exclude_probes),
            Self::Sale => (f.sale, f.exclude_sale),
        };
        // exclusions win, as in Filters::normalize
        match (only, hidden) {
            (_, true) => KindState::Hidden,
            (true, false) => KindState::Only,
            (false, false) => KindState::Any,
        }
    }

    pub fn set(self, f: &mut Filters, state: KindState) {
        let only = state == KindState::Only;
        let hidden = state == KindState::Hidden;
        match self {
            Self::Locked => (f.locked, f.exclude_locked) = (only, hidden),
            Self::Physical => (f.physical, f.exclude_physical) = (only, hidden),
            Self::Phantom => (f.phantom, f.exclude_phantom) = (only, hidden),
            Self::Temporary => (f.temporary, f.exclude_temporary) = (only, hidden),
            Self::Attached => (f.only_attachments, f.include_attachments) = (only, !hidden),
            Self::Media => (f.media, f.exclude_media) = (only, hidden),
            Self::Probe => (f.probe, f.exclude_probes) = (only, hidden),
            Self::Sale => (f.sale, f.exclude_sale) = (only, hidden),
        }
    }
}

/// How far an object can be and still be listed, in metres: as far as the draw
/// distance goes (human request, 2026-11-25; the Legacy Viewer has no limit).
pub const REACH: f32 = crate::settings::DRAW_DISTANCE_MAX;

/// Where the objects are looked for: one choice for the Legacy Viewer's
/// "current parcel" and "neighbour regions".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Parcel,
    Region,
    Neighbours,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Parcel, Scope::Region, Scope::Neighbours];

    /// (a parcel lies in the region: with both, the neighbours add nothing)
    pub fn of(f: &Filters) -> Scope {
        match (f.parcel_only, f.neighbors) {
            (true, _) => Scope::Parcel,
            (false, true) => Scope::Neighbours,
            (false, false) => Scope::Region,
        }
    }

    pub fn set(self, f: &mut Filters) {
        f.parcel_only = self == Scope::Parcel;
        f.neighbors = self == Scope::Neighbours;
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::Parcel => aurora_i18n::tr!("area-search-scope-parcel"),
            Scope::Region => aurora_i18n::tr!("area-search-scope-region"),
            Scope::Neighbours => aurora_i18n::tr!("area-search-scope-neighbours"),
        }
    }
}

/// Bars of the distance chart.
pub(super) const BARS: usize = 32;

/// The distance chart: how many objects the other filters keep at each
/// distance, from the user out to `reach` metres.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Chart {
    pub bars: [u32; BARS],
    pub reach: f32,
}

impl Default for Chart {
    fn default() -> Self {
        Chart::new(std::iter::empty())
    }
}

impl Chart {
    /// `distances` of the rows every filter but the distance keeps. One
    /// region's side at least; as far as the furthest object otherwise, by
    /// whole sides (neighbouring regions, var regions), up to `REACH`.
    pub fn new(distances: impl Iterator<Item = f32> + Clone) -> Self {
        const SIDE: f32 = 256.0;
        let far = distances.clone().filter(|d| d.is_finite()).fold(0.0, f32::max);
        let reach = ((far / SIDE).ceil() * SIDE).clamp(SIDE, REACH);
        let mut bars = [0; BARS];
        for d in distances.filter(|d| d.is_finite()) {
            let bar = (d.max(0.0) / reach * BARS as f32) as usize;
            bars[bar.min(BARS - 1)] += 1;
        }
        Self { bars, reach }
    }

    pub fn tallest(&self) -> u32 {
        self.bars.iter().copied().max().unwrap_or(0)
    }

    /// Whether some of the bar lies in the range `low..=high` metres.
    pub fn bar_in(&self, bar: usize, low: f32, high: f32) -> bool {
        let width = self.reach / BARS as f32;
        let from = bar as f32 * width;
        from < high && from + width > low
    }
}

/// Where a distance band starts, in metres: within reach, close by, around
/// you, further, far.
pub(super) const BANDS: [f32; 5] = [0.0, 10.0, 32.0, 64.0, 128.0];

pub(super) fn band_of(distance: f32) -> usize {
    BANDS.iter().rposition(|from| distance >= *from).unwrap_or(0)
}

/// What a band's heading says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Band {
    pub count: usize,
    pub land_impact: f32,
}

pub(super) fn bands<'a>(rows: impl Iterator<Item = &'a Row>) -> [Band; BANDS.len()] {
    let mut out = [Band::default(); BANDS.len()];
    for row in rows {
        let band = &mut out[band_of(row.distance)];
        band.count += 1;
        band.land_impact += row.land_impact.unwrap_or(0.0);
    }
    out
}

/// One line of the Owners tab.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Owner {
    pub id: Uuid,
    pub name: String,
    pub group: bool,
    pub count: usize,
    pub land_impact: f32,
    /// Rows of theirs, in the list's order.
    pub rows: Vec<usize>,
}

/// The owners of `visible`, heaviest first. Rows whose owner is not known
/// yet wait for it.
pub(super) fn owners(rows: &[Row], visible: &[usize]) -> Vec<Owner> {
    let mut at: HashMap<Uuid, usize> = HashMap::new();
    let mut out: Vec<Owner> = Vec::new();
    for &i in visible {
        let row = &rows[i];
        if row.owner_id.is_nil() {
            continue;
        }
        let slot = *at.entry(row.owner_id).or_insert_with(|| {
            out.push(Owner {
                id: row.owner_id,
                name: row.owner.clone(),
                group: row.owner_id == row.group_id,
                count: 0,
                land_impact: 0.0,
                rows: Vec::new(),
            });
            out.len() - 1
        });
        let owner = &mut out[slot];
        owner.count += 1;
        owner.land_impact += row.land_impact.unwrap_or(0.0);
        owner.rows.push(i);
    }
    out.sort_by(|a, b| {
        b.land_impact
            .total_cmp(&a.land_impact)
            .then(b.count.cmp(&a.count))
            .then_with(|| crate::natural_order::cmp(&a.name, &b.name))
            .then(a.id.cmp(&b.id))
    });
    out
}

/// A line of the list of objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Line {
    /// A distance band's heading.
    Band(usize),
    /// A row, by its place in `Table::visible`.
    Row(usize),
}

/// Everything the window counts, made once per result list.
#[derive(Clone, Debug, Default)]
pub(super) struct Digest {
    pub chart: Chart,
    pub lines: Vec<Line>,
    pub bands: [Band; BANDS.len()],
    pub owners: Vec<Owner>,
    /// How many owners the results have.
    pub owner_count: usize,
    pub land_impact: f32,
    pub kinds: [u32; Kind::ALL.len()],
    /// The rows for sale, cheapest first.
    pub sale: Vec<usize>,
    /// Land impacts of the results, heaviest first.
    weights: Vec<f32>,
}

impl Digest {
    pub fn of(table: &super::Table) -> Self {
        let (rows, visible) = (&table.rows, &table.visible);
        let shown = || visible.iter().map(|i| &rows[*i]);
        // bands only while the list goes from near to far
        let banded = table.sort == super::Column::Distance && !table.descending;
        let mut lines = Vec::with_capacity(visible.len() + BANDS.len());
        let mut last = None;
        for (at, i) in visible.iter().enumerate() {
            let band = band_of(rows[*i].distance);
            if banded && last != Some(band) {
                lines.push(Line::Band(band));
                last = Some(band);
            }
            lines.push(Line::Row(at));
        }
        let mut kinds = [0; Kind::ALL.len()];
        for row in shown() {
            for (count, kind) in kinds.iter_mut().zip(Kind::ALL) {
                *count += u32::from(kind.is(row));
            }
        }
        let mut sale: Vec<usize> = visible.iter().copied().filter(|i| rows[*i].price.is_some()).collect();
        sale.sort_by_key(|i| (rows[*i].price, rows[*i].id));
        let mut weights: Vec<f32> = shown().filter_map(|r| r.land_impact).collect();
        weights.sort_unstable_by(|a, b| b.total_cmp(a));
        let owners = owners(rows, &table.unpicked);
        Digest {
            chart: Chart::new(table.distances.iter().copied()),
            lines,
            bands: bands(shown()),
            owner_count: if table.owner.is_some() { 1 } else { owners.len() },
            owners,
            land_impact: weights.iter().sum(),
            kinds,
            sale,
            weights,
        }
    }

    /// The place of a land impact among the results: 1 is the heaviest.
    pub fn rank(&self, land_impact: f32) -> usize {
        self.weights.partition_point(|w| *w > land_impact) + 1
    }

    pub fn ranked(&self) -> usize {
        self.weights.len()
    }
}

impl Kind {
    /// Whether an object is of this kind, as the filters see it.
    pub fn is(self, row: &Row) -> bool {
        match self {
            Self::Locked => row.loaded && row.owner_mask & aurora_net::build::perm::MOVE == 0,
            Self::Physical => row.flags & 1 != 0,
            Self::Phantom => row.flags & (1 << 10) != 0,
            Self::Temporary => row.flags & (1 << 29) != 0,
            Self::Attached => row.attachment,
            Self::Media => row.media,
            Self::Probe => row.probe,
            Self::Sale => row.price.is_some(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Locked => aurora_i18n::tr!("build-ui-object-locked"),
            Self::Physical => aurora_i18n::tr!("area-search-physical"),
            Self::Phantom => aurora_i18n::tr!("area-search-phantom"),
            Self::Temporary => aurora_i18n::tr!("area-search-temporary"),
            Self::Attached => aurora_i18n::tr!("area-search-kind-attached"),
            Self::Media => aurora_i18n::tr!("area-search-media"),
            Self::Probe => aurora_i18n::tr!("area-search-kind-probes"),
            Self::Sale => aurora_i18n::tr!("area-search-sale"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Locked => "lock-filled",
            Self::Physical => "bounce-right-filled",
            Self::Phantom => "ghost-2-filled",
            Self::Temporary => "clock-filled",
            Self::Attached => "paperclip",
            Self::Media => "device-tv-filled",
            Self::Probe => "sphere",
            Self::Sale => "tag-filled",
        }
    }
}

/// The name of a distance band.
pub(super) fn band_name(band: usize) -> &'static str {
    match band {
        0 => aurora_i18n::tr!("area-search-band-reach"),
        1 => aurora_i18n::tr!("area-search-band-close"),
        2 => aurora_i18n::tr!("area-search-band-around"),
        3 => aurora_i18n::tr!("area-search-band-further"),
        _ => aurora_i18n::tr!("area-search-band-far"),
    }
}

/// A filter that is on, as a chip that takes it away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum On {
    Kind(Kind),
    Click,
    Copy,
    Modify,
    Transfer,
    Parcel,
    Mine,
    Linked,
    Neighbors,
    Name,
    Description,
    Owner,
    Group,
    Creator,
    LastOwner,
}

/// The filters that differ from the original ones, the distance aside (the
/// chart says it). Attachments hidden are one: something is left out.
pub(super) fn filters_on(f: &Filters) -> Vec<On> {
    let mut out: Vec<On> = Kind::ALL
        .into_iter()
        .filter(|k| k.state(f) != KindState::Any)
        .map(On::Kind)
        .collect();
    let flags = [
        (On::Click, f.click_action != 0),
        (On::Copy, f.copy),
        (On::Modify, f.modify),
        (On::Transfer, f.transfer),
        (On::Parcel, f.parcel_only),
        (On::Mine, f.mine),
        (On::Linked, f.children),
        (On::Neighbors, f.neighbors),
        (On::Name, !f.name.is_empty()),
        (On::Description, !f.description.is_empty()),
        (On::Owner, !f.owner.is_empty()),
        (On::Group, !f.group.is_empty()),
        (On::Creator, !f.creator.is_empty()),
        (On::LastOwner, !f.last_owner.is_empty()),
    ];
    out.extend(flags.into_iter().filter(|(_, on)| *on).map(|(what, _)| what));
    out
}

impl On {
    /// Back to any.
    pub fn clear(self, f: &mut Filters) {
        match self {
            On::Kind(kind) => kind.set(f, KindState::Any),
            On::Click => f.click_action = 0,
            On::Copy => f.copy = false,
            On::Modify => f.modify = false,
            On::Transfer => f.transfer = false,
            On::Parcel => f.parcel_only = false,
            On::Mine => f.mine = false,
            On::Linked => f.children = false,
            On::Neighbors => f.neighbors = false,
            On::Name => f.name.clear(),
            On::Description => f.description.clear(),
            On::Owner => f.owner.clear(),
            On::Group => f.group.clear(),
            On::Creator => f.creator.clear(),
            On::LastOwner => f.last_owner.clear(),
        }
    }

    /// Hidden rather than required.
    pub fn hides(self, f: &Filters) -> bool {
        matches!(self, On::Kind(kind) if kind.state(f) == KindState::Hidden)
    }

    pub fn icon(self, f: &Filters) -> &'static str {
        match self {
            On::Kind(kind) => kind.icon(),
            On::Click => CLICK_ICONS[usize::from(f.click_action.min(9))],
            On::Copy => "copy-filled",
            On::Modify => "pencil-filled",
            On::Transfer => "arrows-exchange",
            On::Parcel => "map-pin-filled",
            On::Mine => "user-filled",
            On::Linked => "link",
            On::Neighbors => "map",
            _ => "search-filled",
        }
    }

    pub fn label(self, f: &Filters) -> String {
        let field = |name: &str, text: &str| aurora_i18n::tr!("area-search-chip-field", name = name, text = text);
        match self {
            On::Kind(kind) => kind.label().to_owned(),
            On::Click => aurora_i18n::tr!("area-search-chip-click", action = click_label(f.click_action)),
            On::Copy => aurora_i18n::tr!("area-search-copy-perm").to_owned(),
            On::Modify => aurora_i18n::tr!("area-search-modify-perm").to_owned(),
            On::Transfer => aurora_i18n::tr!("area-search-transfer-perm").to_owned(),
            On::Parcel => aurora_i18n::tr!("area-search-parcel").to_owned(),
            On::Mine => aurora_i18n::tr!("area-search-mine").to_owned(),
            On::Linked => aurora_i18n::tr!("area-search-linked").to_owned(),
            On::Neighbors => aurora_i18n::tr!("area-search-neighbouring").to_owned(),
            On::Name => field(aurora_i18n::tr!("inspect-col-item"), &f.name),
            On::Description => field(aurora_i18n::tr!("inspect-col-description"), &f.description),
            On::Owner => field(aurora_i18n::tr!("inspect-col-owner"), &f.owner),
            On::Group => field(aurora_i18n::tr!("area-search-group"), &f.group),
            On::Creator => field(aurora_i18n::tr!("inspect-col-creator"), &f.creator),
            On::LastOwner => field(aurora_i18n::tr!("area-search-last-owner"), &f.last_owner),
        }
    }
}

/// The mouse click actions, in the order of the Legacy combo.
pub(super) const CLICK_ICONS: [&str; 10] = [
    "",
    "",
    "click-filled",
    "armchair",
    "shopping-cart-filled",
    "coin-filled",
    "package",
    "player-play-filled",
    "device-tv-filled",
    "zoom-in-filled",
];

pub(super) fn click_label(action: u8) -> &'static str {
    match action {
        0 => aurora_i18n::tr!("area-search-click-unrestricted"),
        1 => aurora_i18n::tr!("area-search-click-any"),
        2 => aurora_i18n::tr!("ui-context-touch"),
        3 => aurora_i18n::tr!("area-search-sit"),
        4 => aurora_i18n::tr!("common-buy-object"),
        5 => aurora_i18n::tr!("area-search-click-pay"),
        6 => aurora_i18n::tr!("area-search-click-open"),
        7 => aurora_i18n::tr!("area-search-click-play"),
        8 => aurora_i18n::tr!("area-search-click-media"),
        _ => aurora_i18n::tr!("common-zoom-2"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u128, owner: u128, distance: f32, land_impact: Option<f32>) -> Row {
        let mut row = Row::default();
        row.id = Uuid::from_u128(id);
        row.owner_id = Uuid::from_u128(owner);
        row.owner = format!("Owner {owner}");
        row.distance = distance;
        row.land_impact = land_impact;
        row.loaded = true;
        row
    }

    #[test]
    fn area_search_kinds_cycle_and_survive_normalize() {
        for kind in Kind::ALL {
            let mut f = Filters {
                include_attachments: true,
                ..Default::default()
            };
            let mut state = kind.state(&f);
            assert_eq!(state, KindState::Any, "{kind:?}");
            for expected in [KindState::Only, KindState::Hidden, KindState::Any] {
                state = state.next();
                kind.set(&mut f, state);
                f.normalize();
                assert_eq!(kind.state(&f), expected, "{kind:?}");
                for other in Kind::ALL.into_iter().filter(|o| *o != kind) {
                    assert_eq!(other.state(&f), KindState::Any, "{kind:?} changed {other:?}");
                }
            }
        }
        assert_eq!(
            Kind::Attached.state(&Filters::default()),
            KindState::Hidden,
            "worn objects stay hidden by default"
        );
    }

    #[test]
    fn area_search_hidden_kinds_filter_the_rows() {
        let mut table = super::super::table::Table::default();
        let mut phantom = row(1, 9, 1.0, None);
        phantom.flags = 1 << 10;
        let mut media = row(2, 9, 2.0, None);
        media.media = true;
        let mut sale = row(3, 9, 3.0, None);
        sale.price = Some(10);
        let mut movable = row(4, 9, 4.0, None);
        movable.owner_mask = aurora_net::build::perm::MOVE;
        let mut pending = movable.clone();
        pending.id = Uuid::from_u128(5);
        pending.loaded = false;
        table.replace(vec![phantom, media, sale, movable, pending]);
        let mut left = |kind: Kind| {
            table.filters = Filters::default();
            kind.set(&mut table.filters, KindState::Hidden);
            table.prepare();
            table.visible.iter().map(|i| table.rows[*i].id.as_u128()).collect::<Vec<_>>()
        };
        assert_eq!(left(Kind::Phantom), [2, 3, 4, 5]);
        assert_eq!(left(Kind::Media), [1, 3, 4, 5]);
        assert_eq!(left(Kind::Sale), [1, 2, 4, 5]);
        assert_eq!(left(Kind::Locked), [4], "an object of unknown permissions is not shown as unlocked");
    }

    #[test]
    fn area_search_chart_counts_every_row_and_reaches_the_furthest() {
        let near = [0.0, 1.0, 7.9, 8.0, 255.9, 256.0];
        let chart = Chart::new(near.iter().copied());
        assert_eq!(chart.reach, 256.0);
        assert_eq!((chart.bars[0], chart.bars[1], chart.bars[31]), (3, 1, 2));
        assert_eq!(chart.bars.iter().sum::<u32>(), 6);
        assert!(chart.bar_in(0, 0.0, 64.0) && chart.bar_in(7, 0.0, 64.0) && !chart.bar_in(8, 0.0, 64.0));
        assert_eq!(Chart::new([300.0].into_iter()).reach, 512.0);
        let odd = Chart::new([f32::NAN, f32::INFINITY, -1.0, 1e9].into_iter());
        assert_eq!((odd.reach, odd.bars[0], odd.bars[31]), (1024.0, 1, 1), "bad distances are left out");
        assert_eq!(Chart::new(std::iter::empty()).tallest(), 0);
    }

    #[test]
    fn area_search_scope_is_one_choice_of_two_switches() {
        let mut f = Filters::default();
        assert_eq!(Scope::of(&f), Scope::Region);
        for scope in Scope::ALL {
            scope.set(&mut f);
            assert_eq!(Scope::of(&f), scope);
        }
        // a Legacy link may ask for both
        (f.parcel_only, f.neighbors) = (true, true);
        assert_eq!(Scope::of(&f), Scope::Parcel);
    }

    #[test]
    fn area_search_bands_split_on_their_limits() {
        assert_eq!([0.0, 9.9, 10.0, 32.0, 64.0, 128.0, 900.0].map(band_of), [0, 0, 1, 2, 3, 4, 4]);
        let rows = [row(1, 1, 3.0, Some(2.0)), row(2, 1, 9.0, None), row(3, 1, 70.0, Some(5.0))];
        let bands = bands(rows.iter());
        assert_eq!((bands[0].count, bands[0].land_impact), (2, 2.0));
        assert_eq!((bands[3].count, bands[3].land_impact), (1, 5.0));
    }

    #[test]
    fn area_search_owners_are_ranked_by_land_impact() {
        let mut group = row(4, 7, 1.0, Some(3.0));
        group.group_id = group.owner_id;
        let rows = [
            row(1, 5, 1.0, Some(1.0)),
            row(2, 6, 2.0, Some(8.0)),
            row(3, 5, 3.0, Some(1.5)),
            group,
            row(5, 0, 1.0, Some(99.0)),
            row(6, 6, 1.0, Some(50.0)),
        ];
        // the last row is filtered out; the nil owner is not known yet
        let owners = owners(&rows, &[0, 1, 2, 3, 4]);
        let seen: Vec<_> = owners.iter().map(|o| (o.id.as_u128(), o.count, o.land_impact, o.group)).collect();
        assert_eq!(seen, [(6, 1, 8.0, false), (7, 1, 3.0, true), (5, 2, 2.5, false)]);
        assert_eq!(owners[2].rows, [0, 2]);
    }

    #[test]
    fn area_search_digest_bands_ranks_and_counts() {
        let mut table = super::super::Table::default();
        let mut rows = vec![row(1, 1, 3.0, Some(50.0)), row(2, 1, 40.0, Some(2.0)), row(3, 2, 90.0, None)];
        rows[1].price = Some(30);
        rows[2].price = Some(5);
        rows[0].flags = 1;
        table.replace(rows);
        table.prepare();
        let d = Digest::of(&table);
        assert_eq!(
            d.lines,
            [
                Line::Band(0),
                Line::Row(0),
                Line::Band(2),
                Line::Row(1),
                Line::Band(3),
                Line::Row(2)
            ]
        );
        assert_eq!((d.land_impact, d.owner_count), (52.0, 2));
        assert_eq!((d.rank(50.0), d.rank(2.0), d.ranked()), (1, 2, 2));
        assert_eq!(d.sale, [2, 1], "cheapest first");
        assert_eq!(d.kinds[1], 1);
        // heaviest first: no bands
        table.sort = super::super::Column::LandImpact;
        table.descending = true;
        table.dirty = true;
        table.prepare();
        assert_eq!(Digest::of(&table).lines, [Line::Row(0), Line::Row(1), Line::Row(2)]);
        // an owner picked narrows the list, the others stay to be picked
        table.owner = Some(Uuid::from_u128(2));
        table.prepare();
        let d = Digest::of(&table);
        assert_eq!((table.visible.len(), d.owners.len(), d.owner_count), (1, 2, 1));
        assert_eq!(table.distances, [90.0], "the chart follows the pick");
    }

    #[test]
    fn area_search_chips_name_what_is_on_and_take_it_away() {
        let mut f = Filters {
            children: true,
            click_action: 3,
            name: "Lamp".into(),
            physical: true,
            ..Default::default()
        };
        let on = filters_on(&f);
        assert_eq!(
            on,
            [On::Kind(Kind::Physical), On::Kind(Kind::Attached), On::Click, On::Linked, On::Name]
        );
        assert!(On::Kind(Kind::Attached).hides(&f) && !On::Kind(Kind::Physical).hides(&f));
        for what in on {
            what.clear(&mut f);
        }
        assert!(filters_on(&f).is_empty());
        assert!(f.include_attachments, "any: attachments are listed too");
    }
}
