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

/// Bars of the distance chart.
pub(super) const BARS: usize = 32;

/// The distance chart: how many objects the other filters keep at each
/// distance, from the user out to `reach` metres.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Chart {
    pub bars: [u32; BARS],
    pub reach: f32,
}

impl Chart {
    /// `distances` of the rows every filter but the distance keeps. One
    /// region's side at least; as far as the furthest object otherwise, by
    /// whole sides (neighbouring regions, var regions), up to `MAX_REACH`.
    pub fn new(distances: impl Iterator<Item = f32> + Clone) -> Self {
        const SIDE: f32 = 256.0;
        const MAX_REACH: f32 = 4096.0;
        let far = distances.clone().filter(|d| d.is_finite()).fold(0.0, f32::max);
        let reach = ((far / SIDE).ceil() * SIDE).clamp(SIDE, MAX_REACH);
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
        assert_eq!((odd.reach, odd.bars[0], odd.bars[31]), (4096.0, 1, 1), "bad distances are left out");
        assert_eq!(Chart::new(std::iter::empty()).tallest(), 0);
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
}
