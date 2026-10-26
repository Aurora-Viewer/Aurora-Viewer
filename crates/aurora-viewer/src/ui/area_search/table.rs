//! Prepared, virtualized results. Text normalization and ordering run in parallel.
use rayon::prelude::*;
use uuid::Uuid;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    #[default]
    Distance,
    Name,
    Owner,
    Description,
    Price,
}
impl Column {
    pub fn label(self) -> &'static str {
        match self {
            Self::Distance => aurora_i18n::tr!("area-search-range"),
            Self::Name => aurora_i18n::tr!("inspect-col-name"),
            Self::Owner => aurora_i18n::tr!("inspect-col-owner"),
            Self::Description => aurora_i18n::tr!("inspect-col-description"),
            Self::Price => aurora_i18n::tr!("area-search-price"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Filters {
    pub query: String,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub creator: String,
    pub parcel_only: bool,
    pub children: bool,
    pub neighbors: bool,
    pub mine: bool,
    pub sale: bool,
    pub physical: bool,
    pub phantom: bool,
    pub temporary: bool,
    pub distance_on: bool,
    pub max_distance: f32,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            query: String::new(),
            name: String::new(),
            description: String::new(),
            owner: String::new(),
            creator: String::new(),
            parcel_only: false,
            children: false,
            neighbors: false,
            mine: false,
            sale: false,
            physical: false,
            phantom: false,
            temporary: false,
            distance_on: false,
            max_distance: 96.0,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Row {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub creator: String,
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
    keys: [String; 9],
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
        ];
    }
    fn matches(&self, f: &Filters) -> bool {
        (!f.parcel_only || self.on_parcel)
            && (f.children || !self.child)
            && (!f.mine || self.mine)
            && (!f.sale || self.price.is_some())
            && (!f.physical || self.flags & 1 != 0)
            && (!f.phantom || self.flags & (1 << 10) != 0)
            && (!f.temporary || self.flags & (1 << 29) != 0)
            && (!f.distance_on || self.distance <= f.max_distance)
            && (f.query.is_empty() || self.keys.iter().any(|s| s.contains(&f.query)))
            && self.keys[0].contains(&f.name)
            && self.keys[1].contains(&f.description)
            && (self.keys[2].contains(&f.owner) || self.keys[6].contains(&f.owner))
            && (self.keys[3].contains(&f.creator) || self.keys[7].contains(&f.creator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    applied: Option<Filters>,
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
        let mut f = self.filters.clone();
        for s in [&mut f.query, &mut f.name, &mut f.description, &mut f.owner, &mut f.creator] {
            *s = s.trim().to_lowercase();
        }
        self.visible = self
            .rows
            .par_iter()
            .enumerate()
            .filter_map(|(i, r)| r.matches(&f).then_some(i))
            .collect();
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
            };
            (if descending { order.reverse() } else { order }).then(a.id.cmp(&b.id))
        });
        self.applied = Some(self.filters.clone());
        self.dirty = false;
        self.generation += 1;
    }
}
