//! Prepared, virtualized results. Text normalization and ordering run in parallel.
//! Matching follows FSAreaSearch::matchObject (indra/newview/fsareasearch.cpp;
//! originally LGPL 2.1). Compiled filters survive live snapshot replacements.
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    #[default]
    Distance,
    Name,
    Owner,
    Description,
    Price,
    Group,
    Creator,
    LastOwner,
    LandImpact,
    PrimCount,
}
impl Column {
    pub const ALL: [Self; 10] = [
        Self::Distance,
        Self::Name,
        Self::Description,
        Self::Price,
        Self::LandImpact,
        Self::PrimCount,
        Self::Owner,
        Self::Group,
        Self::Creator,
        Self::LastOwner,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Distance => aurora_i18n::tr!("area-search-range"),
            Self::Name => aurora_i18n::tr!("inspect-col-name"),
            Self::Owner => aurora_i18n::tr!("inspect-col-owner"),
            Self::Description => aurora_i18n::tr!("inspect-col-description"),
            Self::Price => aurora_i18n::tr!("area-search-price"),
            Self::Group => aurora_i18n::tr!("area-search-group"),
            Self::Creator => aurora_i18n::tr!("inspect-col-creator"),
            Self::LastOwner => aurora_i18n::tr!("area-search-last-owner"),
            Self::LandImpact => aurora_i18n::tr!("area-search-land-impact"),
            Self::PrimCount => aurora_i18n::tr!("area-search-prim-count"),
        }
    }
}

pub(crate) use crate::settings::area_search::Filters;

#[derive(Clone, Default)]
pub(crate) struct Row {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub creator: String,
    pub last_owner: String,
    pub last_owner_id: Uuid,
    pub owner_mask: u32,
    pub attachment: bool,
    pub media: bool,
    pub probe: bool,
    pub click_action: u8,
    pub land_impact: Option<f32>,
    pub prim_count: u32,
    pub owner_id: Uuid,
    pub creator_id: Uuid,
    pub group: String,
    pub group_id: Uuid,
    pub distance: f32,
    pub price: Option<i32>,
    pub flags: u32,
    pub on_parcel: bool,
    pub child: bool,
    pub mine: bool,
    pub loaded: bool,
    keys: [String; 11],
}
impl Row {
    pub fn cell(&self, column: Column) -> String {
        match column {
            Column::Distance => format!("{:.0} m", self.distance),
            Column::Name => {
                if self.loaded {
                    self.name.clone()
                } else {
                    aurora_i18n::tr!("common-loading-2").into()
                }
            }
            Column::Description => self.description.clone(),
            Column::Group => self.group.clone(),
            Column::Creator => self.creator.clone(),
            Column::LastOwner => self.last_owner.clone(),
            Column::LandImpact => self.land_impact.map_or_else(|| "…".into(), |n| format!("{n:.1}")),
            Column::PrimCount => self.prim_count.to_string(),
            Column::Owner => self.owner.clone(),
            Column::Price => self.price.map_or_else(String::new, |n| format!("L$ {n}")),
        }
    }
    fn normalize(&mut self) {
        self.keys = [
            self.name.to_lowercase(),
            self.description.to_lowercase(),
            self.owner.to_lowercase(),
            self.creator.to_lowercase(),
            self.group.to_lowercase(),
            self.id.to_string(),
            self.owner_id.to_string(),
            self.creator_id.to_string(),
            self.group_id.to_string(),
            self.last_owner.to_lowercase(),
            self.last_owner_id.to_string(),
        ];
    }
    fn matches(&self, f: &Filters, patterns: &[Option<fancy_regex::Regex>], limit: &AtomicBool) -> bool {
        // A failed expression invalidates the batch. Other workers can stop
        // before repeating the same expensive failure on every remaining row.
        if limit.load(Ordering::Relaxed) {
            return false;
        }
        (!f.parcel_only || self.on_parcel)
            && (f.children || !self.child)
            && (!f.mine || self.mine)
            && (f.include_attachments || !self.attachment)
            && (!f.only_attachments || self.attachment)
            && (!f.sale || self.price.is_some_and(|p| p >= f.min_price && p <= f.max_price))
            && (!f.locked || (self.loaded && self.owner_mask & aurora_net::build::perm::MOVE == 0))
            && (!f.copy || self.owner_mask & aurora_net::build::perm::COPY != 0)
            && (!f.modify || self.owner_mask & aurora_net::build::perm::MODIFY != 0)
            && (!f.transfer || self.owner_mask & aurora_net::build::perm::TRANSFER != 0)
            && (!f.media || self.media)
            && (!f.probe || self.probe)
            && (!f.exclude_probes || !self.probe)
            && (!f.exclude_physical || self.flags & 1 == 0)
            && (!f.exclude_temporary || self.flags & (1 << 29) == 0)
            && match f.click_action {
                0 => true,
                1 => self.flags & (1 << 7) != 0 || self.click_action != 0,
                2 => self.flags & (1 << 7) != 0,
                n => self.click_action == n - 2,
            }
            && (!f.physical || self.flags & 1 != 0)
            && (!f.phantom || self.flags & (1 << 10) != 0)
            && (!f.temporary || self.flags & (1 << 29) != 0)
            && (!f.distance_on || self.distance >= f.min_distance && self.distance <= f.max_distance)
            && (f.query.is_empty() || self.keys.iter().any(|s| s.contains(&f.query)))
            && if f.regex {
                // FSAreaSearch::matchObject uses case-sensitive whole-field regex matching.
                [
                    &self.name,
                    &self.description,
                    &self.owner,
                    &self.creator,
                    &self.group,
                    &self.last_owner,
                ]
                .iter()
                .zip(patterns)
                .all(|(text, pattern)| {
                    pattern.as_ref().is_none_or(|r| match r.is_match(text) {
                        Ok(matched) => matched,
                        Err(_) => {
                            limit.store(true, Ordering::Relaxed);
                            false
                        }
                    })
                })
            } else {
                self.keys[0].contains(&f.name)
                    && self.keys[1].contains(&f.description)
                    && (self.keys[2].contains(&f.owner) || self.keys[6].contains(&f.owner))
                    && (self.keys[3].contains(&f.creator) || self.keys[7].contains(&f.creator))
                    && (self.keys[4].contains(&f.group) || self.keys[8].contains(&f.group))
                    && (self.keys[9].contains(&f.last_owner) || self.keys[10].contains(&f.last_owner))
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_search_matches_group_last_owner_and_whole_field_regex() {
        let mut table = Table::default();
        let mut object = row(1, "Violet Chair", 12.0);
        object.group = "Demo builders".into();
        object.last_owner = "Nova Example".into();
        object.last_owner_id = Uuid::from_u128(999);
        table.replace(vec![object]);
        table.filters.group = "BUILDERS".into();
        table.filters.last_owner = Uuid::from_u128(999).to_string();
        assert_eq!(ids(&mut table), [1]);
        table.filters.group = String::new();
        table.filters.last_owner = "Nova .*".into();
        table.filters.name = "Violet (Chair|Lamp)".into();
        table.filters.regex = true;
        assert_eq!(ids(&mut table), [1]);
        table.filters.name = "Chair".into();
        assert!(ids(&mut table).is_empty(), "regex matches the complete field");
        table.filters.name = "violet .*".into();
        assert!(ids(&mut table).is_empty(), "Legacy regex is case sensitive");
        table.filters.name = "(".into();
        assert!(ids(&mut table).is_empty());
        assert!(table.regex_error.is_some());
        table.filters.name = "Violet .*".into();
        assert_eq!(ids(&mut table), [1]);
        assert!(table.regex_error.is_none());
    }

    #[test]
    fn area_search_permissions_ranges_media_clicks_and_exclusions_intersect() {
        let mut table = Table::default();
        let mut object = row(1, "Cabinet", 12.0);
        object.owner_mask = aurora_net::build::perm::COPY | aurora_net::build::perm::MODIFY | aurora_net::build::perm::TRANSFER;
        object.flags = 1 | (1 << 7) | (1 << 10) | (1 << 29);
        object.price = Some(25);
        object.media = true;
        object.probe = true;
        object.attachment = true;
        table.replace(vec![object]);
        table.filters = Filters {
            locked: true,
            copy: true,
            modify: true,
            transfer: true,
            media: true,
            probe: true,
            include_attachments: true,
            only_attachments: true,
            physical: true,
            phantom: true,
            temporary: true,
            sale: true,
            min_price: 25,
            max_price: 25,
            distance_on: true,
            min_distance: 12.0,
            max_distance: 12.0,
            click_action: 2,
            ..Default::default()
        };
        assert_eq!(ids(&mut table), [1]);
        table.filters.exclude_physical = true;
        assert!(ids(&mut table).is_empty());
        table.filters.exclude_physical = false;
        table.filters.exclude_temporary = true;
        assert!(ids(&mut table).is_empty());
        table.filters.exclude_temporary = false;
        table.filters.exclude_probes = true;
        assert!(ids(&mut table).is_empty());
        table.filters.exclude_probes = false;
        table.filters.min_price = 26;
        assert!(ids(&mut table).is_empty());
        table.filters.min_price = 25;
        table.filters.click_action = 3;
        assert!(ids(&mut table).is_empty(), "touch handler is not Sit");
    }
    #[test]
    fn area_search_keeps_precise_distance_and_numeric_new_columns() {
        let mut table = Table::default();
        let mut near = row(1, "Near", 12.1);
        near.land_impact = Some(12.0);
        near.prim_count = 20;
        let mut far = row(2, "Far", 12.9);
        far.land_impact = Some(2.0);
        far.prim_count = 3;
        table.replace(vec![near, far]);
        table.filters.distance_on = true;
        table.filters.min_distance = 12.2;
        table.filters.max_distance = 12.9;
        assert_eq!(ids(&mut table), [2], "retain precise distance rather than truncating metres");
        table.filters.distance_on = false;
        table.sort = Column::LandImpact;
        table.dirty = true;
        assert_eq!(ids(&mut table), [2, 1]);
        table.sort = Column::PrimCount;
        table.dirty = true;
        assert_eq!(ids(&mut table), [2, 1]);
        for click in 1..=7 {
            table.rows[0].click_action = click;
            table.filters.click_action = click + 2;
            table.dirty = true;
            assert_eq!(ids(&mut table), [1]);
        }
    }
    #[test]
    fn area_search_regex_updates_with_live_metadata_and_retains_parse_errors_until_corrected() {
        let mut table = Table::default();
        table.filters.regex = true;
        table.filters.owner = "Nova .*".into();
        let mut object = row(1, "Chair", 1.0);
        table.replace(vec![object.clone()]);
        assert!(ids(&mut table).is_empty());
        object.owner = "Nova Example".into();
        table.replace(vec![object.clone()]);
        assert_eq!(ids(&mut table), [1], "the cached expression sees new metadata");
        table.filters.owner = "(".into();
        assert!(ids(&mut table).is_empty());
        assert!(table.regex_error.is_some());
        table.replace(vec![object]);
        assert!(ids(&mut table).is_empty());
        assert!(table.regex_error.is_some(), "a new snapshot cannot discard a parse error");
        table.filters.owner = "Nova .*".into();
        assert_eq!(ids(&mut table), [1]);
        assert!(table.regex_error.is_none());
    }

    #[test]
    fn area_search_regex_limit_cancels_remaining_matches_and_recovers_on_edit() {
        let mut table = Table::default();
        // A literal c exists, so the engine's literal prefilter cannot avoid
        // the ambiguous alternatives; the final ! makes a whole match fail.
        let text = format!("{}c!", "ab".repeat(28));
        table.replace((1..=128).map(|id| row(id, &text, 1.0)).collect());
        table.filters.regex = true;
        table.filters.name = "(?i)(a|b|ab)*(?>c)".into();
        assert!(ids(&mut table).is_empty());
        assert_eq!(table.regex_error.as_deref(), Some(aurora_i18n::tr!("area-search-regex-limit")));
        let cancelled = AtomicBool::new(true);
        assert!(
            !table.rows[0].matches(&Filters::default(), &[], &cancelled),
            "a cancelled worker skips remaining rows"
        );
        table.filters.name = ".*".into();
        assert_eq!(ids(&mut table).len(), 128);
        assert!(table.regex_error.is_none());
    }
    fn row(id: u128, name: &str, distance: f32) -> Row {
        Row {
            id: Uuid::from_u128(id),
            name: name.into(),
            distance,
            loaded: true,
            ..Default::default()
        }
    }
    fn ids(t: &mut Table) -> Vec<u128> {
        t.prepare();
        t.visible.iter().map(|i| t.rows[*i].id.as_u128()).collect()
    }

    #[test]
    fn area_search_combines_case_insensitive_fields_and_late_identity_replies() {
        let mut t = Table::default();
        let mut a = row(1, "Violet Chair", 12.0);
        a.description = "A quiet seat".into();
        a.owner_id = Uuid::from_u128(123);
        a.creator_id = Uuid::from_u128(456);
        a.group = "Demo builders".into();
        t.replace(vec![a.clone(), row(2, "Lamp", 3.0)]);
        t.filters.query = "  BUILDERS ".into();
        t.filters.name = "CHAIR".into();
        t.filters.description = "quiet".into();
        assert_eq!(ids(&mut t), [1]);
        t.filters.owner = "nova".into();
        assert!(ids(&mut t).is_empty());
        a.owner = "Nova Example".into();
        a.creator = "Tess Example".into();
        t.replace(vec![a.clone()]);
        t.filters.creator = " tess ".into();
        assert_eq!(ids(&mut t), [1], "late name replies affect hidden rows too");
        t.filters.owner = a.owner_id.to_string();
        assert_eq!(ids(&mut t), [1]);
        t.filters.query = a.id.to_string();
        assert_eq!(ids(&mut t), [1]);
    }

    #[test]
    fn area_search_filters_parcel_flags_distance_sale_and_linked_prims_together() {
        let mut t = Table::default();
        let mut a = row(1, "Chair", 12.0);
        a.on_parcel = true;
        a.mine = true;
        a.price = Some(25);
        a.flags = 1 | (1 << 10) | (1 << 29);
        let mut child = a.clone();
        child.id = Uuid::from_u128(2);
        child.child = true;
        t.replace(vec![a.clone(), child, row(3, "Outside", 1.0)]);
        t.filters = Filters {
            parcel_only: true,
            mine: true,
            sale: true,
            physical: true,
            phantom: true,
            temporary: true,
            distance_on: true,
            max_distance: 12.0,
            ..Default::default()
        };
        assert_eq!(ids(&mut t), [1]);
        t.filters.children = true;
        assert_eq!(ids(&mut t), [1, 2]);
        t.filters.max_distance = 11.0;
        assert!(ids(&mut t).is_empty());
        t.filters = Filters::default();
        assert_eq!(ids(&mut t), [3, 1]);
        t.replace(Vec::new());
        assert!(ids(&mut t).is_empty(), "removed objects leave the results");
    }

    #[test]
    fn area_search_orders_numbers_numerically_and_keeps_filters_during_refresh() {
        let mut t = Table::default();
        let mut a = row(1, "Alpha", 20.0);
        a.price = Some(100);
        let mut b = row(2, "zulu", 3.0);
        b.price = Some(20);
        t.replace(vec![a.clone(), b.clone()]);
        assert_eq!(ids(&mut t), [2, 1]);
        t.sort = Column::Price;
        t.dirty = true;
        assert_eq!(ids(&mut t), [2, 1]);
        t.descending = true;
        t.dirty = true;
        assert_eq!(ids(&mut t), [1, 2]);
        t.sort = Column::Name;
        t.descending = false;
        t.dirty = true;
        assert_eq!(ids(&mut t), [1, 2]);
        t.filters.query = "alpha".into();
        t.replace(vec![b, a]);
        assert_eq!(ids(&mut t), [1]);
        assert_eq!(t.sort, Column::Name);
        assert_eq!(t.filters.query, "alpha");
    }
}

/// Compilation depends on filter text and switches, not incoming object data.
struct CompiledFilters {
    normalized: Filters,
    patterns: Vec<Option<fancy_regex::Regex>>,
    error: Option<String>,
}
impl CompiledFilters {
    fn new(source: &Filters) -> Self {
        let mut f = source.clone();
        f.query = f.query.trim().to_lowercase();
        let mut patterns = Vec::new();
        let mut error = None;
        for s in [
            &mut f.name,
            &mut f.description,
            &mut f.owner,
            &mut f.creator,
            &mut f.group,
            &mut f.last_owner,
        ] {
            if f.regex && !s.is_empty() {
                match fancy_regex::RegexBuilder::new(&format!("\\A(?:{s})\\z"))
                    .backtrack_limit(100_000)
                    .build()
                {
                    Ok(pattern) => patterns.push(Some(pattern)),
                    Err(problem) => {
                        error = Some(problem.to_string());
                        patterns.push(None);
                    }
                }
            } else {
                *s = s.trim().to_lowercase();
                patterns.push(None);
            }
        }
        Self {
            normalized: f,
            patterns,
            error,
        }
    }
}

#[derive(Default)]
pub(crate) struct Table {
    pub rows: Vec<Row>,
    pub visible: Vec<usize>,
    pub filters: Filters,
    pub sort: Column,
    pub descending: bool,
    pub pending: usize,
    pub dirty: bool,
    /// Counts the recomputed result lists (the selection is checked once each).
    pub generation: u64,
    pub regex_error: Option<String>,
    applied: Option<Filters>,
    compiled: Option<CompiledFilters>,
}
impl Table {
    pub fn replace(&mut self, mut rows: Vec<Row>) {
        rows.par_iter_mut().for_each(Row::normalize);
        self.rows = rows;
        self.dirty = true;
    }
    pub fn prepare(&mut self) {
        if !self.dirty && self.applied.as_ref() == Some(&self.filters) {
            return;
        }
        if self.applied.as_ref() != Some(&self.filters) || self.compiled.is_none() {
            self.compiled = Some(CompiledFilters::new(&self.filters));
        }
        let Some(compiled) = &self.compiled else {
            return;
        };
        self.regex_error.clone_from(&compiled.error);
        let limit = AtomicBool::new(false);
        self.visible = self
            .rows
            .par_iter()
            .enumerate()
            .filter_map(|(i, r)| (self.regex_error.is_none() && r.matches(&compiled.normalized, &compiled.patterns, &limit)).then_some(i))
            .collect();
        if limit.load(Ordering::Relaxed) {
            self.regex_error = Some(aurora_i18n::tr!("area-search-regex-limit").into());
            self.visible.clear();
        }
        let rows = &self.rows;
        let sort = self.sort;
        let descending = self.descending;
        self.visible.par_sort_unstable_by(|a, b| {
            let (a, b) = (&rows[*a], &rows[*b]);
            let order = match sort {
                Column::Distance => a.distance.total_cmp(&b.distance),
                Column::Price => a.price.cmp(&b.price),
                Column::Name => a.keys[0].cmp(&b.keys[0]),
                Column::Description => a.keys[1].cmp(&b.keys[1]),
                Column::Owner => a.keys[2].cmp(&b.keys[2]),
                Column::Group => a.keys[4].cmp(&b.keys[4]),
                Column::Creator => a.keys[3].cmp(&b.keys[3]),
                Column::LastOwner => a.keys[9].cmp(&b.keys[9]),
                Column::LandImpact => a.land_impact.unwrap_or(-1.0).total_cmp(&b.land_impact.unwrap_or(-1.0)),
                Column::PrimCount => a.prim_count.cmp(&b.prim_count),
            };
            (if descending { order.reverse() } else { order }).then(a.id.cmp(&b.id))
        });
        self.applied = Some(self.filters.clone());
        self.dirty = false;
        self.generation += 1;
    }
}
