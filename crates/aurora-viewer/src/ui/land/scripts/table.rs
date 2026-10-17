//! Cached script-resource rows: numeric sorting, owner-name arrival and search.

use crate::world::{land::Land, social::Social};
use aurora_net::{GroupMembership, land::ScriptObject};
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum Column {
    #[default]
    Memory,
    Urls,
    Name,
    Owner,
    Place,
    Position,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Sort {
    pub fn click(&mut self, column: Column) {
        self.descending = self.column == column && !self.descending;
        self.column = column;
    }
}

pub(super) struct Row {
    pub object: ScriptObject,
    pub parcel_id: Option<i32>,
    pub place: String,
    pub owner: String,
    name_key: String,
    owner_key: String,
    place_key: String,
}

impl Row {
    pub fn attachment(mut object: ScriptObject, place: String, agent: Uuid) -> Self {
        // AttachmentResources can omit owner_id: these are the agent's
        // attachments, so owner search still uses the known wearer.
        if object.owner.is_nil() {
            object.owner = agent;
            object.group_owned = false;
        }
        Self::new(object, None, place)
    }

    pub fn new(object: ScriptObject, parcel_id: Option<i32>, place: String) -> Self {
        Self {
            name_key: object.name.to_lowercase(),
            place_key: place.to_lowercase(),
            object,
            parcel_id,
            place,
            owner: String::new(),
            owner_key: String::new(),
        }
    }

    fn compare(&self, other: &Self, column: Column) -> Ordering {
        match column {
            Column::Memory => self.object.memory.cmp(&other.object.memory),
            Column::Urls => self.object.urls.cmp(&other.object.urls),
            Column::Name => self.name_key.cmp(&other.name_key),
            Column::Owner => self.owner_key.cmp(&other.owner_key),
            Column::Place => self.place_key.cmp(&other.place_key),
            Column::Position => match (self.object.location, other.object.location) {
                (Some(a), Some(b)) => a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        }
    }
}

#[derive(Default)]
pub(super) struct Table {
    pub filter: String,
    pub sort: Sort,
    pub rows: Vec<Row>,
    pub visible: Vec<usize>,
    pub visible_ids: HashSet<Uuid>,
    pub revision: Option<u64>,
    owners: HashMap<(Uuid, bool), String>,
    applied_filter: String,
    applied_sort: Option<Sort>,
    dirty: bool,
}

impl Table {
    pub fn replace(&mut self, rows: Vec<Row>, revision: u64) {
        self.owners.clear();
        self.owners
            .extend(rows.iter().map(|r| ((r.object.owner, r.object.group_owned), String::new())));
        self.rows = rows;
        self.revision = Some(revision);
        self.dirty = true;
    }

    pub fn sync_names(&mut self, land: &mut Land, social: &mut Social, groups: &[GroupMembership]) {
        let mut changed = false;
        // Resolve each distinct owner once, including owners hidden by the
        // filter, so a late name reply can make their objects appear.
        for ((id, group), name) in &mut self.owners {
            let resolved = if *group {
                land.group_name(id, groups).unwrap_or_default()
            } else {
                social.want_name(*id);
                social.name_of(id)
            };
            if *name != resolved {
                *name = resolved;
                changed = true;
            }
        }
        if changed {
            for row in &mut self.rows {
                if let Some(name) = self.owners.get(&(row.object.owner, row.object.group_owned)) {
                    row.owner.clone_from(name);
                    row.owner_key = name.to_lowercase();
                }
            }
            self.dirty = true;
        }
    }

    pub fn prepare(&mut self) {
        if !self.dirty && self.applied_filter == self.filter && self.applied_sort == Some(self.sort) {
            return;
        }
        let query = self.filter.trim().to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| query.is_empty() || r.name_key.contains(&query) || r.owner_key.contains(&query))
            .map(|(i, _)| i)
            .collect();
        let rows = &self.rows;
        let sort = self.sort;
        self.visible.par_sort_unstable_by(|a, b| {
            let order = rows[*a].compare(&rows[*b], sort.column);
            let order = if sort.descending { order.reverse() } else { order };
            order.then(rows[*a].object.id.cmp(&rows[*b].object.id))
        });
        self.visible_ids = self.visible.iter().map(|i| rows[*i].object.id).collect();
        self.applied_filter.clone_from(&self.filter);
        self.applied_sort = Some(self.sort);
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u128, name: &str, memory: i64, urls: i64, x: f32, place: &str) -> Row {
        Row::new(
            ScriptObject {
                id: Uuid::from_u128(id),
                name: name.into(),
                owner: Uuid::from_u128(id + 10),
                group_owned: false,
                memory,
                urls,
                location: Some(glam::Vec3::new(x, 0.0, 0.0)),
            },
            Some(1),
            place.into(),
        )
    }

    fn ids(table: &Table) -> Vec<u128> {
        table.visible.iter().map(|i| table.rows[*i].object.id.as_u128()).collect()
    }

    #[test]
    fn columns_use_numeric_values_and_names_with_reversible_deterministic_order() {
        let mut table = Table::default();
        table.replace(
            vec![entry(2, "Zulu", 10_000, 10, 100.0, "A"), entry(1, "alpha", 900, 2, 20.0, "z")],
            1,
        );
        for column in [Column::Memory, Column::Urls, Column::Name, Column::Position] {
            table.sort = Sort { column, descending: false };
            table.prepare();
            assert_eq!(ids(&table), [1, 2], "{column:?}");
            table.sort.click(column);
            table.prepare();
            assert_eq!(ids(&table), [2, 1], "{column:?} reversed");
        }
        table.sort.click(Column::Place);
        table.prepare();
        assert_eq!(ids(&table), [2, 1], "new column starts ascending, including attachment points");
        table.rows[0].object.memory = 900;
        table.sort = Sort::default();
        table.prepare();
        assert_eq!(ids(&table), [1, 2], "ties use object IDs, not reply order");
    }

    #[test]
    fn search_matches_object_or_owner_and_refresh_preserves_the_search_and_sort() {
        let mut land = Land::default();
        let mut social = Social::default();
        social.names.insert(Uuid::from_u128(11), "Étoile Resident".into());
        land.group_names.insert(Uuid::from_u128(12), "Aurora Builders".into());
        let mut table = Table::default();
        let mut group_row = entry(2, "Door", 64, 0, 0.0, "Parcel");
        group_row.object.group_owned = true;
        table.replace(vec![entry(1, "FONTAINE", 128, 0, 0.0, "Parcel"), group_row], 1);
        table.sync_names(&mut land, &mut social, &[]);
        for (query, expected) in [
            (" fontaine ", vec![1]),
            ("éTOILE", vec![1]),
            ("BUILDERS", vec![2]),
            ("missing", vec![]),
        ] {
            table.filter = query.into();
            table.prepare();
            assert_eq!(ids(&table), expected, "{query}");
            assert_eq!(
                table.visible_ids.len(),
                expected.len(),
                "hidden rows are excluded from the action selection"
            );
        }
        table.filter = "door".into();
        table.sort = Sort {
            column: Column::Name,
            descending: true,
        };
        table.replace(
            vec![entry(3, "Door B", 1, 0, 0.0, "Parcel"), entry(4, "Door A", 2, 0, 0.0, "Parcel")],
            2,
        );
        table.prepare();
        assert_eq!(ids(&table), [3, 4]);
        assert_eq!(table.filter, "door");
    }

    #[test]
    fn owner_arriving_after_a_search_updates_the_filtered_rows_and_owner_sort() {
        let mut land = Land::default();
        let mut social = Social::default();
        let mut table = Table {
            filter: "builder".into(),
            sort: Sort {
                column: Column::Owner,
                descending: false,
            },
            ..Default::default()
        };
        table.replace(vec![entry(1, "One", 1, 0, 0.0, ""), entry(2, "Two", 1, 0, 0.0, "")], 1);
        table.sync_names(&mut land, &mut social, &[]);
        table.prepare();
        assert!(table.visible.is_empty());
        social.names.insert(Uuid::from_u128(11), "Zulu Builder".into());
        social.names.insert(Uuid::from_u128(12), "Alpha Builder".into());
        table.sync_names(&mut land, &mut social, &[]);
        table.prepare();
        assert_eq!(ids(&table), [2, 1]);
        table.sort.click(Column::Owner);
        table.prepare();
        assert_eq!(ids(&table), [1, 2]);
    }

    #[test]
    fn attachments_without_owner_fields_are_searchable_by_the_wearer() {
        let mut land = Land::default();
        let mut social = Social::default();
        let agent = Uuid::from_u128(99);
        social.names.insert(agent, "Aurora Demo".into());
        let mut object = entry(1, "HUD", 1, 0, 0.0, "").object;
        object.owner = Uuid::nil();
        let mut table = Table {
            filter: "aurora".into(),
            ..Default::default()
        };
        table.replace(vec![Row::attachment(object, "Skull".into(), agent)], 1);
        table.sync_names(&mut land, &mut social, &[]);
        table.prepare();
        assert_eq!(ids(&table), [1]);
        assert_eq!(table.rows[0].parcel_id, None);
    }
}
