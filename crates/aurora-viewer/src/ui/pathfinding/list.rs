//! The list shared by the Region objects and Characters windows: rows,
//! filters, sort, selection and the line under the list. Port of
//! LLFloaterPathfindingObjects (rebuildObjectsScrollList, the messaging
//! states) with the filters of LLFloaterPathfindingLinksets
//! (buildObjectsScrollList) and the cells of buildLinksetScrollListItemData /
//! buildCharacterScrollListItemData (indra/newview, originally LGPL 2.1).
use crate::ui::area_search::Selection;
use aurora_net::RegionHandle;
use aurora_net::pathfinding::linkset::Use;
use aurora_net::pathfinding::{Detail, Error, Kind, Row};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Description,
    Owner,
    Scripted,
    Impact,
    Distance,
    Use,
    A,
    B,
    C,
    D,
    Cpu,
    Altitude,
}

impl Column {
    /// floater_pathfinding_linksets.xml / floater_pathfinding_characters.xml.
    pub const LINKSETS: [Column; 11] = [
        Column::Name,
        Column::Description,
        Column::Owner,
        Column::Scripted,
        Column::Impact,
        Column::Distance,
        Column::Use,
        Column::A,
        Column::B,
        Column::C,
        Column::D,
    ];
    pub const CHARACTERS: [Column; 5] = [Column::Name, Column::Description, Column::Owner, Column::Cpu, Column::Altitude];

    pub fn of(kind: Kind) -> &'static [Column] {
        match kind {
            Kind::Objects => &Self::LINKSETS,
            Kind::Characters => &Self::CHARACTERS,
        }
    }

    pub fn label(self, kind: Kind) -> &'static str {
        aurora_i18n::text(match (self, kind) {
            (Column::Name, Kind::Objects) => "pathfinding-column-name-root",
            (Column::Name, Kind::Characters) => "pathfinding-column-name",
            (Column::Description, Kind::Objects) => "pathfinding-column-description-root",
            (Column::Description, Kind::Characters) => "pathfinding-column-description",
            (Column::Owner, _) => "pathfinding-column-owner",
            (Column::Scripted, _) => "pathfinding-column-scripted",
            (Column::Impact, _) => "pathfinding-column-impact",
            (Column::Distance, _) => "pathfinding-column-distance",
            (Column::Use, _) => "pathfinding-column-use",
            (Column::A, _) => "pathfinding-column-a",
            (Column::B, _) => "pathfinding-column-b",
            (Column::C, _) => "pathfinding-column-c",
            (Column::D, _) => "pathfinding-column-d",
            (Column::Cpu, _) => "pathfinding-column-cpu",
            (Column::Altitude, _) => "pathfinding-column-altitude",
        })
    }

    /// The share of the width each column starts with (the XML's widths).
    pub fn weight(self) -> f32 {
        match self {
            Column::Name | Column::Description => 0.17,
            Column::Owner => 0.16,
            Column::Use => 0.17,
            Column::Scripted | Column::Impact | Column::Distance => 0.065,
            Column::A | Column::B | Column::C | Column::D => 0.045,
            Column::Cpu | Column::Altitude => 0.1,
        }
    }

    /// The narrowest a column draws before the table scrolls sideways.
    pub fn min_width(self) -> f32 {
        match self {
            Column::Name | Column::Description | Column::Owner => 120.0,
            Column::Use => 130.0,
            Column::Scripted | Column::Impact | Column::Distance | Column::Cpu | Column::Altitude => 64.0,
            Column::A | Column::B | Column::C | Column::D => 44.0,
        }
    }
}

/// Use type labels (linkset_use_*), shared by the column, the filter and
/// the edit dropdown.
pub fn use_label(use_: Use) -> &'static str {
    aurora_i18n::text(match use_ {
        Use::Walkable => "pathfinding-use-walkable",
        Use::StaticObstacle => "pathfinding-use-static-obstacle",
        Use::MovableObstacle => "pathfinding-use-movable-obstacle",
        Use::MaterialVolume => "pathfinding-use-material-volume",
        Use::ExclusionVolume => "pathfinding-use-exclusion-volume",
        Use::MovablePhantom => "pathfinding-use-movable-phantom",
    })
}

/// EMessagingState: what the line under the list says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Messaging {
    #[default]
    Initial,
    Getting,
    GetError,
    Setting,
    SetError,
    NotEnabled,
    Complete,
}

/// The filters (onApplyAllFilters): applied with Apply or Enter, not while
/// typing; matched case-insensitively.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub description: String,
    pub use_: Option<Use>,
}

/// What the app resolves for the cells: owner names and the agent's
/// position in the list's region.
pub struct Context<'a> {
    /// The owner's label; None while the name loads.
    pub owners: &'a HashMap<Uuid, Option<String>>,
    pub agent: [f64; 3],
}

/// One row's cells as shown, and its sort keys.
#[derive(Debug, Clone, Default)]
struct Cells {
    name: String,
    description: String,
    owner: String,
    owner_loading: bool,
    distance: f64,
    text: [String; 4],
}

#[derive(Default)]
pub struct List {
    pub kind: Option<Kind>,
    pub region: RegionHandle,
    rows: Vec<Row>,
    /// Row by id: selections can hold thousands of rows.
    index: HashMap<Uuid, usize>,
    cells: Vec<Cells>,
    /// Rows shown, in display order (indices into `rows`).
    visible: Vec<usize>,
    pub selection: Selection,
    pub sort: Option<Column>,
    pub descending: bool,
    /// Being typed; `applied` is what the list shows.
    pub filter: Filter,
    applied: Filter,
    pub messaging: Messaging,
    pending: Option<Uuid>,
    dirty: bool,
    /// Rows to select once the list arrives (showFloaterWithSelectionObjects).
    to_select: Option<Vec<Uuid>>,
    /// Counts the rebuilt lists, so the app notices a new one.
    pub generation: u64,
}

impl List {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind: Some(kind),
            sort: Some(Column::Name),
            ..Default::default()
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind.unwrap_or(Kind::Objects)
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, id: &Uuid) -> Option<&Row> {
        self.index.get(id).and_then(|&i| self.rows.get(i))
    }

    /// The selected rows, in selection order.
    pub fn selected_rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.selection.ids.iter().filter_map(|id| self.row(id))
    }

    fn reindex(&mut self) {
        self.index = self.rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    }

    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub fn visible_row(&self, n: usize) -> Option<&Row> {
        self.visible.get(n).and_then(|&i| self.rows.get(i))
    }

    pub fn visible_ids(&self) -> Vec<Uuid> {
        self.visible.iter().map(|&i| self.rows[i].id).collect()
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// requestGetObjects: a new request id, the list kept until it answers.
    pub fn start_get(&mut self, region: RegionHandle) -> Uuid {
        let request = Uuid::new_v4();
        self.region = region;
        self.pending = Some(request);
        self.messaging = Messaging::Getting;
        request
    }

    pub fn start_set(&mut self) -> Uuid {
        let request = Uuid::new_v4();
        self.pending = Some(request);
        self.messaging = Messaging::Setting;
        request
    }

    pub fn select_when_loaded(&mut self, ids: Vec<Uuid>) {
        self.to_select = Some(ids);
    }

    /// handleNewObjectList: the answer to the latest request only.
    pub fn on_list(&mut self, request: Uuid, region: RegionHandle, result: Result<Vec<Row>, Error>) {
        if self.pending != Some(request) || self.region != region {
            return;
        }
        self.pending = None;
        match result {
            Ok(rows) => {
                self.rows = rows;
                self.messaging = Messaging::Complete;
            }
            Err(Error::Unsupported) => {
                self.rows.clear();
                self.messaging = Messaging::NotEnabled;
            }
            Err(_) => {
                self.rows.clear();
                self.messaging = Messaging::GetError;
            }
        }
        self.reindex();
        if let Some(ids) = self.to_select.take() {
            self.selection.select_all(ids);
        }
        self.dirty = true;
    }

    /// handleUpdateObjectList: the changed linksets replace theirs.
    pub fn on_update(&mut self, request: Uuid, region: RegionHandle, result: Result<Vec<Row>, Error>) {
        if self.pending != Some(request) || self.region != region {
            return;
        }
        self.pending = None;
        match result {
            Ok(changed) => {
                for row in changed {
                    match self.index.get(&row.id) {
                        Some(&i) => self.rows[i] = row,
                        None => {
                            self.index.insert(row.id, self.rows.len());
                            self.rows.push(row);
                        }
                    }
                }
                self.messaging = Messaging::Complete;
            }
            Err(Error::Unsupported) => self.messaging = Messaging::NotEnabled,
            Err(_) => self.messaging = Messaging::SetError,
        }
        self.dirty = true;
    }

    /// The region changed or the window closed: nothing is kept.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.index.clear();
        self.cells.clear();
        self.visible.clear();
        self.selection.clear();
        self.pending = None;
        self.messaging = Messaging::Initial;
        self.generation += 1;
    }

    pub fn apply_filters(&mut self) {
        self.applied = self.filter.clone();
        self.dirty = true;
    }

    /// onClearFiltersClicked.
    pub fn clear_filters(&mut self) {
        self.filter = Filter::default();
        self.apply_filters();
    }

    pub fn sort_by(&mut self, column: Column) {
        self.descending = self.sort == Some(column) && !self.descending;
        self.sort = Some(column);
        self.dirty = true;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Owner names still loading, for the app to fill in later.
    pub fn owners_loading(&self) -> bool {
        self.cells.iter().any(|c| c.owner_loading)
    }

    pub fn owners(&self) -> impl Iterator<Item = (Uuid, bool)> + '_ {
        self.rows.iter().map(|r| (r.owner, r.group_owned))
    }

    /// The cell text of a visible row.
    pub fn cell(&self, n: usize, column: Column) -> &str {
        let Some(&i) = self.visible.get(n) else { return "" };
        let cells = &self.cells[i];
        match column {
            Column::Name => &cells.name,
            Column::Description => &cells.description,
            Column::Owner => &cells.owner,
            Column::Scripted | Column::Cpu => &cells.text[0],
            Column::Impact | Column::Altitude => &cells.text[1],
            Column::Distance => &cells.text[2],
            Column::Use => &cells.text[3],
            Column::A | Column::B | Column::C | Column::D => self.coefficient(i, column),
        }
    }

    fn coefficient(&self, i: usize, column: Column) -> &'static str {
        let Some(linkset) = self.rows[i].linkset() else { return "" };
        let n = match column {
            Column::A => 0,
            Column::B => 1,
            Column::C => 2,
            _ => 3,
        };
        NUMBERS[usize::from(linkset.walkability[n].min(100))]
    }

    /// rebuildObjectsScrollList: cells, filters and order, once per change
    /// (in parallel: a region can hold thousands of linksets).
    pub fn prepare(&mut self, cx: &Context) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.cells = self.rows.par_iter().map(|row| cells(row, cx)).collect();
        let name = self.applied.name.trim().to_lowercase();
        let description = self.applied.description.trim().to_lowercase();
        let use_ = self.applied.use_;
        let rows = &self.rows;
        let cells = &self.cells;
        self.visible = (0..rows.len())
            .into_par_iter()
            .filter(|&i| {
                (name.is_empty() || cells[i].name.to_lowercase().contains(&name))
                    && (description.is_empty() || rows[i].description.to_lowercase().contains(&description))
                    && use_.is_none_or(|u| rows[i].linkset().is_some_and(|l| l.use_ == u))
            })
            .collect();
        if let Some(sort) = self.sort {
            let descending = self.descending;
            self.visible.par_sort_by(|&a, &b| {
                let order = compare(&rows[a], &cells[a], &rows[b], &cells[b], sort);
                (if descending { order.reverse() } else { order }).then(rows[a].id.cmp(&rows[b].id))
            });
        }
        // a selected row the filters hide can no longer be acted on
        let shown: HashSet<Uuid> = self.visible.iter().map(|&i| rows[i].id).collect();
        self.selection.retain(&shown);
        self.generation += 1;
    }

    /// The line under the list (updateMessagingStatus).
    pub fn status(&self) -> String {
        let key = match self.messaging {
            Messaging::Initial => return String::new(),
            Messaging::Getting => self.text("pathfinding-linksets-getting", "pathfinding-characters-getting"),
            Messaging::GetError => self.text("pathfinding-linksets-get-error", "pathfinding-characters-get-error"),
            Messaging::Setting => "pathfinding-linksets-setting",
            Messaging::SetError => "pathfinding-linksets-set-error",
            Messaging::NotEnabled => "pathfinding-not-enabled",
            Messaging::Complete if self.visible.is_empty() => self.text("pathfinding-linksets-none", "pathfinding-characters-none"),
            Messaging::Complete => {
                let (selected, total) = (self.selection.ids.len() as i64, self.visible.len() as i64);
                return match self.kind() {
                    Kind::Objects => aurora_i18n::tr!("pathfinding-linksets-count", selected = selected, total = total),
                    Kind::Characters => aurora_i18n::tr!("pathfinding-characters-count", selected = selected, total = total),
                };
            }
        };
        aurora_i18n::text(key).to_owned()
    }

    fn text(&self, linksets: &'static str, characters: &'static str) -> &'static str {
        match self.kind() {
            Kind::Objects => linksets,
            Kind::Characters => characters,
        }
    }
}

/// "0" to "100", so the coefficient cells borrow instead of allocating.
static NUMBERS: [&str; 101] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23",
    "24", "25", "26", "27", "28", "29", "30", "31", "32", "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45",
    "46", "47", "48", "49", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59", "60", "61", "62", "63", "64", "65", "66", "67",
    "68", "69", "70", "71", "72", "73", "74", "75", "76", "77", "78", "79", "80", "81", "82", "83", "84", "85", "86", "87", "88", "89",
    "90", "91", "92", "93", "94", "95", "96", "97", "98", "99", "100",
];

fn yes_no(value: Option<bool>) -> String {
    aurora_i18n::text(match value {
        Some(true) => "pathfinding-scripted-yes",
        Some(false) => "pathfinding-scripted-no",
        None => "pathfinding-scripted-unknown",
    })
    .to_owned()
}

fn cells(row: &Row, cx: &Context) -> Cells {
    let terrain = row.linkset().is_some_and(|l| l.terrain);
    if terrain {
        let dash = || aurora_i18n::text("pathfinding-terrain-dash").to_owned();
        let use_ = row.linkset().map_or(Use::Walkable, |l| l.use_);
        return Cells {
            name: aurora_i18n::text("pathfinding-terrain-name").to_owned(),
            description: dash(),
            owner: dash(),
            owner_loading: false,
            distance: f64::MAX,
            text: [
                dash(),
                dash(),
                dash(),
                format!("{} {}", use_label(use_), aurora_i18n::text("pathfinding-use-terrain")),
            ],
        };
    }
    let owner = cx.owners.get(&row.owner).cloned().flatten();
    let owner_loading = owner.is_none() && !row.owner.is_nil();
    let owner = match owner {
        _ if row.owner.is_nil() => aurora_i18n::text("pathfinding-owner-unknown").to_owned(),
        Some(name) if row.group_owned => format!("{name} {}", aurora_i18n::text("pathfinding-owner-group")),
        Some(name) => name,
        None => aurora_i18n::text("pathfinding-owner-loading").to_owned(),
    };
    let distance = (0..3).map(|i| (row.position[i] - cx.agent[i]).powi(2)).sum::<f64>().sqrt();
    let text = match &row.detail {
        Detail::Object {
            land_impact,
            scripted,
            linkset,
        } => {
            // the state suffixes of buildLinksetScrollListItemData
            let suffix = match (linkset.modifiable, linkset.can_be_volume) {
                (false, true) => Some("pathfinding-use-restricted"),
                (true, false) => Some("pathfinding-use-concave"),
                (false, false) => Some("pathfinding-use-restricted-concave"),
                (true, true) => None,
            };
            let use_text = match suffix {
                Some(key) => format!("{} {}", use_label(linkset.use_), aurora_i18n::text(key)),
                None => use_label(linkset.use_).to_owned(),
            };
            [
                yes_no(*scripted),
                land_impact.to_string(),
                aurora_i18n::tr!("pathfinding-distance", meters = format!("{distance:.0}")),
                use_text,
            ]
        }
        Detail::Character { cpu_us, .. } => [
            aurora_i18n::tr!("pathfinding-cpu", time = format!("{:.0}", cpu_us.round())),
            aurora_i18n::tr!("pathfinding-distance", meters = format!("{:.0}", row.position[2])),
            String::new(),
            String::new(),
        ],
    };
    Cells {
        name: row.name.clone(),
        description: row.description.clone(),
        owner,
        owner_loading,
        distance,
        text,
    }
}

/// Text columns compare their shown text without case, number columns
/// by value (the Legacy Viewer's lists sort numbers as numbers).
fn compare(a: &Row, ca: &Cells, b: &Row, cb: &Cells, column: Column) -> std::cmp::Ordering {
    let text = |x: &str, y: &str| x.chars().flat_map(char::to_lowercase).cmp(y.chars().flat_map(char::to_lowercase));
    let walk = |r: &Row, n: usize| r.linkset().map_or(0, |l| l.walkability[n]);
    match column {
        Column::Name => text(&ca.name, &cb.name),
        Column::Description => text(&ca.description, &cb.description),
        Column::Owner => text(&ca.owner, &cb.owner),
        Column::Scripted => text(&ca.text[0], &cb.text[0]),
        Column::Use => text(&ca.text[3], &cb.text[3]),
        Column::Impact => impact(a).cmp(&impact(b)),
        Column::Distance => ca.distance.total_cmp(&cb.distance),
        Column::A => walk(a, 0).cmp(&walk(b, 0)),
        Column::B => walk(a, 1).cmp(&walk(b, 1)),
        Column::C => walk(a, 2).cmp(&walk(b, 2)),
        Column::D => walk(a, 3).cmp(&walk(b, 3)),
        Column::Cpu => cpu(a).total_cmp(&cpu(b)),
        Column::Altitude => a.position[2].total_cmp(&b.position[2]),
    }
}

fn impact(row: &Row) -> u32 {
    match row.detail {
        Detail::Object { land_impact, .. } => land_impact,
        Detail::Character { .. } => 0,
    }
}

fn cpu(row: &Row) -> f64 {
    match row.detail {
        Detail::Character { cpu_us, .. } => cpu_us,
        Detail::Object { .. } => 0.0,
    }
}
