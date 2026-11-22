//! Folder-specific Explorer presentation. Only visible rows, columns, or tiles are visited when painting.
use super::super::organized_materials::MaterialTiles;
use super::*;
use aurora_i18n::tr;
use chrono::{Datelike, Days, Local, Months, NaiveDate, Offset, TimeZone};
use model::{DateBucket, DirectoryView, GroupBy, GroupKey, IconSize, NameBucket, SortKey, ViewLayout};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Clone)]
pub(super) struct Section {
    pub(super) key: Option<GroupKey>,
    pub(super) label: String,
    pub(super) entries: Range<usize>,
    /// Matching rows before the collapsed sections are projected out.
    pub(super) count: usize,
}

impl Section {
    fn new(key: Option<GroupKey>, label: String, entries: Range<usize>) -> Self {
        Self {
            key,
            label,
            count: entries.len(),
            entries,
        }
    }
}

fn type_group(inv: &Inventory, row: Row) -> GroupKey {
    match row {
        Row::Place(Location::Folder(_) | Location::Library) => GroupKey::Folder,
        Row::Place(Location::Custom(_)) => GroupKey::VirtualFolder,
        Row::Place(_) => GroupKey::Location,
        Row::Item(id) => {
            GroupKey::Item(aurora::rows::item_type(inv, id).unwrap_or(aurora::rows::ItemType::Asset(aurora::rows::AssetClass::Other)))
        }
    }
}

pub(super) fn kind(inv: &Inventory, row: Row) -> String {
    group_label(type_group(inv, row))
}

fn date(inv: &Inventory, row: Row) -> i64 {
    // SL folders have no native creation timestamp. Leave their date blank.
    row.id().and_then(|id| inv.items.get(&id)).map_or(0, |item| item.created_at)
}

fn name_bucket(name: &str) -> NameBucket {
    match name.trim_start().chars().next().and_then(|c| c.to_uppercase().next()) {
        Some('A'..='H') => NameBucket::AtoH,
        Some('I'..='P') => NameBucket::ItoP,
        Some('Q'..='Z') => NameBucket::QtoZ,
        Some(c) if c.is_numeric() => NameBucket::Numbers,
        Some(c) if c.is_alphabetic() => NameBucket::Initial(c),
        _ => NameBucket::Symbols,
    }
}

fn day_in_zone<T: TimeZone>(utc: chrono::DateTime<chrono::Utc>, zone: &T) -> Option<NaiveDate> {
    utc.naive_utc()
        .checked_add_offset(zone.offset_from_utc_datetime(&utc.naive_utc()).fix())
        .map(|d| d.date())
}

fn local_day(seconds: i64) -> Option<NaiveDate> {
    if seconds <= 0 {
        return None;
    }
    day_in_zone(chrono::DateTime::from_timestamp(seconds, 0)?, &Local)
}

fn date_bucket(day: Option<NaiveDate>, today: NaiveDate) -> DateBucket {
    let Some(day) = day else { return DateBucket::Unknown };
    let age = (today - day).num_days();
    let week_start = i64::from(today.weekday().num_days_from_monday());
    match age {
        ..0 => DateBucket::Future,
        0 => DateBucket::Today,
        1 => DateBucket::Yesterday,
        age if age <= week_start => DateBucket::ThisWeek,
        age if age <= week_start + 7 => DateBucket::LastWeek,
        _ => {
            let months = (today.year() - day.year()) * 12 + today.month() as i32 - day.month() as i32;
            if months == 0 {
                DateBucket::ThisMonth
            } else if months == 1 {
                DateBucket::LastMonth
            } else if today.year() == day.year() {
                DateBucket::ThisYear
            } else if today.year() == day.year() + 1 {
                DateBucket::LastYear
            } else {
                DateBucket::Older
            }
        }
    }
}

/// Resolve the local calendar cutoffs once per refresh. Windows may query the OS
/// on each conversion, which becomes expensive for a search across a large inventory.
struct DateGroups<T> {
    zone: T,
    today: NaiveDate,
    cutoffs: Option<Vec<(i64, DateBucket)>>,
}

impl<T: TimeZone> DateGroups<T> {
    fn new(zone: T, today: NaiveDate) -> Self {
        let monday = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
        let month = today.with_day(1).unwrap();
        let year = month.with_month(1).unwrap();
        let periods = [
            (today + Days::new(1), DateBucket::Future),
            (today, DateBucket::Today),
            (today - Days::new(1), DateBucket::Yesterday),
            (monday, DateBucket::ThisWeek),
            (monday - Days::new(7), DateBucket::LastWeek),
            (month, DateBucket::ThisMonth),
            (month - Months::new(1), DateBucket::LastMonth),
            (year, DateBucket::ThisYear),
            (year - Months::new(12), DateBucket::LastYear),
        ];
        // A more specific period already covers overlapping month/year starts.
        let mut previous = None;
        let cutoffs = periods
            .into_iter()
            .filter(|(start, _)| {
                if previous.is_some_and(|date| *start >= date) {
                    return false;
                }
                previous = Some(*start);
                true
            })
            .map(|(start, bucket)| {
                let midnight = start.and_time(chrono::NaiveTime::MIN);
                zone.from_local_datetime(&midnight).single().map(|time| (time.timestamp(), bucket))
            })
            .collect();
        Self { zone, today, cutoffs }
    }

    fn bucket(&self, seconds: i64) -> DateBucket {
        if seconds <= 0 {
            return DateBucket::Unknown;
        }
        let Some(utc) = chrono::DateTime::from_timestamp(seconds, 0) else {
            return DateBucket::Unknown;
        };
        if let Some(cutoffs) = &self.cutoffs {
            return cutoffs
                .iter()
                .find(|(start, _)| seconds >= *start)
                .map_or(DateBucket::Older, |(_, bucket)| *bucket);
        }
        // A zone can skip or repeat midnight. Classify actual local dates in that case.
        date_bucket(day_in_zone(utc, &self.zone), self.today)
    }
}

fn local_date_text(seconds: i64, now: i64) -> String {
    let Some(day) = local_day(seconds) else { return String::new() };
    let year = (local_day(now).is_none_or(|today| day.year() != today.year())).then_some(i64::from(day.year()));
    crate::ui::land::day_month(year, day.month() as usize, i64::from(day.day()))
}

fn group_label(key: GroupKey) -> String {
    match key {
        GroupKey::Folder => tr!("explorer-panel-folder"),
        GroupKey::VirtualFolder => tr!("explorer-panel-virtual-folder"),
        GroupKey::Location => tr!("explorer-panel-location"),
        GroupKey::Item(ty) => return ty.label(),
        GroupKey::Name(bucket) => match bucket {
            NameBucket::Symbols => tr!("explorer-group-symbols"),
            NameBucket::Numbers => "0–9",
            NameBucket::AtoH => "A–H",
            NameBucket::ItoP => "I–P",
            NameBucket::QtoZ => "Q–Z",
            NameBucket::Initial(c) => return c.to_string(),
        },
        GroupKey::Date(bucket) => match bucket {
            DateBucket::Older => tr!("explorer-group-older"),
            DateBucket::LastYear => tr!("explorer-group-last-year"),
            DateBucket::ThisYear => tr!("explorer-group-this-year"),
            DateBucket::LastMonth => tr!("explorer-group-last-month"),
            DateBucket::ThisMonth => tr!("explorer-group-this-month"),
            DateBucket::LastWeek => tr!("explorer-group-last-week"),
            DateBucket::ThisWeek => tr!("explorer-group-this-week"),
            DateBucket::Yesterday => tr!("world-tphistory-yesterday"),
            DateBucket::Today => tr!("world-tphistory-today"),
            DateBucket::Future => tr!("explorer-group-future"),
            DateBucket::Unknown => tr!("ui-inventory-unknown-date"),
        },
        GroupKey::HomeCategories => tr!("explorer-home-categories"),
        GroupKey::HomeUtilities => tr!("explorer-home-utilities"),
        GroupKey::HomePinned => tr!("explorer-home-pinned-folders"),
        GroupKey::HomeOther => tr!("explorer-home-other-locations"),
        GroupKey::HomeFolders => tr!("explorer-home-folders"),
        // The pin group owns its display name, even after it is renamed.
        GroupKey::HomePinGroup(_) => unreachable!("pin group labels come from preferences"),
    }
    .to_owned()
}

fn compare_groups(view: DirectoryView, a: Option<GroupKey>, a_label: &str, b: Option<GroupKey>, b_label: &str) -> Ordering {
    let unknown = Some(GroupKey::Date(DateBucket::Unknown));
    let fixed = (a == unknown).cmp(&(b == unknown));
    let natural = if view.group == GroupBy::Type {
        crate::natural_order::cmp(a_label, b_label).then(a.cmp(&b))
    } else {
        a.cmp(&b)
    };
    fixed.then(if view.group_descending { natural.reverse() } else { natural })
}

pub(super) fn order(
    inv: &Inventory,
    layout: &Preferences,
    rows: &mut Vec<Row>,
    view: DirectoryView,
    raw: bool,
    avatar: &str,
) -> Vec<Section> {
    let calendar = (view.group == GroupBy::Date).then(|| DateGroups::new(Local, Local::now().date_naive()));
    let mut buckets = BTreeMap::new();
    if view.group == GroupBy::None {
        buckets.insert(None, Vec::with_capacity(rows.len()));
    }
    for row in rows.drain(..) {
        let name = row_name(inv, layout, row, raw, avatar);
        let created = date(inv, row);
        let key = match view.group {
            GroupBy::None => None,
            GroupBy::Name => Some(GroupKey::Name(name_bucket(&name))),
            GroupBy::Type => Some(type_group(inv, row)),
            GroupBy::Date => Some(GroupKey::Date(calendar.as_ref().unwrap().bucket(created))),
        };
        let ty = if view.sort == SortKey::Type && view.group != GroupBy::Type {
            crate::natural_order::key(&kind(inv, row))
        } else {
            Default::default()
        };
        buckets
            .entry(key)
            .or_default()
            .push((row, crate::natural_order::key(&name), ty, created));
    }

    let mut buckets: Vec<_> = buckets
        .into_iter()
        .map(|(key, entries)| (key, key.map(group_label).unwrap_or_default(), entries))
        .collect();
    buckets.sort_by(|a, b| compare_groups(view, a.0, &a.1, b.0, &b.1));
    let mut sections = Vec::with_capacity(buckets.len());
    for (key, label, mut entries) in buckets {
        entries.sort_by(|a, b| {
            let folders_first = matches!(a.0, Row::Item(_)).cmp(&matches!(b.0, Row::Item(_)));
            let dated_first = if view.sort == SortKey::Date {
                (a.3 <= 0).cmp(&(b.3 <= 0))
            } else {
                Ordering::Equal
            };
            let primary = match view.sort {
                SortKey::Name => Ordering::Equal,
                SortKey::Type => a.2.cmp(&b.2),
                SortKey::Date => a.3.cmp(&b.3),
            }
            .then_with(|| a.1.cmp(&b.1));
            folders_first
                .then(dated_first)
                .then(if view.descending { primary.reverse() } else { primary })
                .then_with(|| a.0.cmp(&b.0))
        });
        let start = rows.len();
        rows.extend(entries.into_iter().map(|entry| entry.0));
        sections.push(Section::new(key, label, start..rows.len()));
    }
    sections
}

/// Keep the unconfirmed row stationary while editing, at the start of its group.
pub(super) fn insert_draft(
    rows: &mut Vec<Row>,
    sections: &mut Vec<Section>,
    row: Row,
    draft: &folder::Draft,
    view: DirectoryView,
    home: bool,
) {
    let key = if home {
        Some(GroupKey::HomeFolders)
    } else {
        match view.group {
            GroupBy::None => None,
            GroupBy::Name => Some(GroupKey::Name(name_bucket(draft.initial_name()))),
            GroupBy::Type => Some(draft.item_type().map_or(GroupKey::Folder, GroupKey::Item)),
            GroupBy::Date => Some(GroupKey::Date(if matches!(row, Row::Item(_)) {
                DateBucket::Today
            } else {
                DateBucket::Unknown
            })),
        }
    };
    let index = sections.iter().position(|s| s.key == key).unwrap_or_else(|| {
        let label = key.map(group_label).unwrap_or_default();
        let i = if home {
            sections.len()
        } else {
            sections.partition_point(|s| compare_groups(view, s.key, &s.label, key, &label).is_lt())
        };
        let start = sections.get(i).map_or(rows.len(), |s| s.entries.start);
        sections.insert(i, Section::new(key, label, start..start));
        i
    });
    rows.insert(sections[index].entries.start, row);
    sections[index].entries.end += 1;
    sections[index].count += 1;
    for section in &mut sections[index + 1..] {
        section.entries.start += 1;
        section.entries.end += 1;
    }
}

/// Landing page shortcuts share the regular tile renderer and its bounded thumbnail requests.
pub(super) fn home(inv: &Inventory, layout: &Preferences, avatar: &str) -> (Vec<Row>, Vec<Section>) {
    let mut rows = Vec::new();
    let mut sections = Vec::new();
    let view = layout.view(Location::Home).for_dashboard();
    let roots = layout.home_roots(inv);
    let builtins: HashSet<_> = roots.iter().copied().collect();
    let home_folders = layout.home_children(inv);
    let local: HashSet<_> = home_folders.iter().copied().collect();
    let mut groups = layout.pin_sections(inv);
    let ungrouped = groups.pop().map(|(_, pins)| pins).unwrap_or_default();
    let mut categories = vec![Location::MyFolders];
    let mut utilities = Vec::new();
    let mut personal = Vec::new();
    for place in ungrouped {
        if !builtins.contains(&place) {
            personal.push(place);
        } else if matches!(place, Location::Library | Location::Group(Group::System))
            || matches!(place, Location::Folder(id) if inv.folders.get(&id).is_some_and(|f| matches!(f.info.type_default, 14 | 16 | 50 | 53)))
        {
            utilities.push(place);
        } else {
            categories.push(place);
        }
    }

    let mut seen = HashSet::new();
    let mut add = |key: GroupKey, label: String, places: Vec<Location>, home_folders: bool| {
        // A pin for a real Home folder belongs to the sidebar. Its tile
        // remains in Home folders, where file actions operate on the folder.
        let mut entries: Vec<_> = places
            .into_iter()
            .filter(|p| (home_folders || !local.contains(p)) && seen.insert(*p))
            .map(Row::Place)
            .collect();
        if !entries.is_empty() {
            order(inv, layout, &mut entries, view, false, avatar);
            let start = rows.len();
            rows.extend(entries);
            sections.push(Section::new(Some(key), label, start..rows.len()));
        }
    };
    add(GroupKey::HomeCategories, group_label(GroupKey::HomeCategories), categories, false);
    add(GroupKey::HomeUtilities, group_label(GroupKey::HomeUtilities), utilities, false);
    add(GroupKey::HomePinned, group_label(GroupKey::HomePinned), personal, false);
    for (group, pins) in groups {
        if let Some(group) = group {
            add(GroupKey::HomePinGroup(group.id), group.name.clone(), pins, false);
        }
    }
    add(GroupKey::HomeOther, group_label(GroupKey::HomeOther), roots, false);
    add(GroupKey::HomeFolders, group_label(GroupKey::HomeFolders), home_folders, true);
    (rows, sections)
}

fn view_options() -> [(ViewLayout, &'static str); 6] {
    [
        (ViewLayout::Details, tr!("explorer-mode-details")),
        (ViewLayout::List, tr!("explorer-mode-list")),
        (ViewLayout::Icons(IconSize::Small), tr!("explorer-mode-small")),
        (ViewLayout::Icons(IconSize::Medium), tr!("explorer-mode-medium")),
        (ViewLayout::Icons(IconSize::Large), tr!("explorer-mode-large")),
        (ViewLayout::Icons(IconSize::ExtraLarge), tr!("explorer-mode-extra-large")),
    ]
}
pub(super) fn sort_label(key: SortKey) -> &'static str {
    match key {
        SortKey::Name => tr!("common-name"),
        SortKey::Type => tr!("common-type"),
        SortKey::Date => tr!("explorer-mode-date"),
    }
}

fn group_commands(ui: &mut Ui, layout: &mut Preferences, state: &ExplorerState) {
    let place = state.location();
    for (collapse, label) in [
        (true, tr!("explorer-collapse-all-groups")),
        (false, tr!("explorer-expand-all-groups")),
    ] {
        let keys = state.sections.iter().filter_map(|section| section.key.as_ref());
        let enabled = keys.clone().any(|key| layout.group_collapsed(place, key) != collapse);
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            layout.set_groups_collapsed(place, keys, collapse);
            ui.close();
        }
    }
}

fn group_menu(ui: &mut Ui, layout: &mut Preferences, state: &ExplorerState, view: &mut DirectoryView) {
    let dashboard = state.on_dashboard();
    let has_groups = state.sections.iter().any(|section| section.key.is_some());
    if dashboard && !has_groups {
        return;
    }
    ui.menu_button(tr!("explorer-mode-group"), |ui| {
        if !dashboard {
            for (group, label) in [
                (GroupBy::None, tr!("common-none")),
                (GroupBy::Name, tr!("common-name")),
                (GroupBy::Type, tr!("common-type")),
                (GroupBy::Date, tr!("explorer-mode-date")),
            ] {
                let option = ui.selectable_value(&mut view.group, group, label);
                if option.changed() {
                    view.group_descending = group == GroupBy::Date;
                }
                if option.clicked() {
                    ui.close();
                }
            }
            if view.group == GroupBy::None {
                return;
            }
            ui.add_space(4.0);
            for (descending, label) in [(false, tr!("explorer-mode-ascending")), (true, tr!("explorer-mode-descending"))] {
                if ui.selectable_value(&mut view.group_descending, descending, label).clicked() {
                    ui.close();
                }
            }
            if has_groups {
                ui.add_space(4.0);
            }
        }
        if has_groups {
            group_commands(ui, layout, state);
        }
    });
}

fn apply_view(layout: &mut Preferences, state: &mut ExplorerState, prior: DirectoryView, view: DirectoryView) {
    if prior != view {
        layout.set_view(state.location(), view);
        if prior.layout != view.layout || prior.group != view.group || prior.group_descending != view.group_descending {
            state.here.scroll = 0.0;
            state.restore_scroll = true;
        }
    }
}

pub(super) fn sort_options(ui: &mut Ui, layout: &mut Preferences, state: &mut ExplorerState) {
    let place = state.here.location;
    let prior = layout.view(place);
    let mut view = prior;
    let dashboard = state.on_dashboard();
    let visible_sort = if dashboard { view.for_dashboard().sort } else { view.sort };
    for key in [SortKey::Name, SortKey::Type, SortKey::Date] {
        let available = !dashboard || key != SortKey::Date;
        let response = ui.add_enabled(available, egui::Button::selectable(visible_sort == key, sort_label(key)));
        if !available {
            response.on_disabled_hover_text(tr!("explorer-home-no-dates"));
        } else if response.clicked() {
            view.sort = key;
            ui.close();
        }
    }
    ui.add_space(4.0);
    for (descending, label) in [(false, tr!("explorer-mode-ascending")), (true, tr!("explorer-mode-descending"))] {
        if ui.selectable_value(&mut view.descending, descending, label).clicked() {
            ui.close();
        }
    }
    ui.add_space(4.0);
    group_menu(ui, layout, state, &mut view);
    apply_view(layout, state, prior, view);
}

pub(super) fn view_options_menu(ui: &mut Ui, palette: &Palette, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState) {
    let place = state.here.location;
    let prior = layout.view(place);
    let mut view = prior;
    for (mode, label) in view_options() {
        if ui.selectable_value(&mut view.layout, mode, label).clicked() {
            ui.close();
        }
    }
    ui.add_space(4.0);
    group_menu(ui, layout, state, &mut view);
    let config = egui::containers::menu::MenuConfig::find(ui).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::containers::menu::SubMenuButton::new(tr!("explorer-mode-pins"))
        .config(config)
        .ui(ui, |ui| pin_manager(ui, palette, inv, layout, state));
    apply_view(layout, state, prior, view);
}

pub(super) fn menu(ui: &mut Ui, palette: &Palette, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState) {
    ui.menu_button(tr!("explorer-mode-view"), |ui| view_options_menu(ui, palette, inv, layout, state));
    ui.menu_button(tr!("inventory-sort-menu"), |ui| sort_options(ui, layout, state));
}

// Wheel steps run from dense lists to large icon tiles. High-resolution
// wheels produce fractional lines or points; accumulate those per active tab.
const WHEEL_LAYOUTS: [ViewLayout; 6] = [
    ViewLayout::Details,
    ViewLayout::List,
    ViewLayout::Icons(IconSize::Small),
    ViewLayout::Icons(IconSize::Medium),
    ViewLayout::Icons(IconSize::Large),
    ViewLayout::Icons(IconSize::ExtraLarge),
];

fn wheel_layout(ui: &Ui, layout: &mut Preferences, state: &mut ExplorerState) {
    let hovered = ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| {
        ui.available_rect_before_wrap().intersect(ui.clip_rect()).contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
    });
    if !hovered || egui::Popup::is_any_open(ui.ctx()) || ui.memory(|m| m.top_modal_layer()).is_some() {
        return;
    }
    let mut delta = 0.0_f32;
    ui.input_mut(|i| {
        i.events.retain(|event| {
            let step = match event {
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    source,
                    ..
                } if (modifiers.ctrl || modifiers.command) && !modifiers.alt && !modifiers.shift => {
                    if *source == egui::MouseWheelSource::Momentum {
                        return false;
                    }
                    match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Point => delta.y / 48.0,
                        egui::MouseWheelUnit::Page if delta.y != 0.0 => delta.y.signum(),
                        egui::MouseWheelUnit::Page => 0.0,
                    }
                }
                // winit exposes native magnification as a factor with no
                // modifier, independently of Ctrl+wheel's line/point events.
                egui::Event::Zoom(factor) if *factor > 0.0 => factor.ln() / 1.1_f32.ln(),
                _ => return true,
            };
            if step.is_finite() {
                delta += step;
            }
            false
        });
    });
    if delta == 0.0 {
        return;
    }
    let place = state.here.location;
    let mut view = layout.view(place);
    let now = ui.input(|i| i.time);
    let id = ui.scope_id().with("explorer-wheel-layout");
    let prior = ui.data(|d| d.get_temp::<(Location, ViewLayout, f64, f32)>(id));
    let carry = prior
        .filter(|(last, mode, at, part)| *last == place && *mode == view.layout && now - *at < 0.65 && part.signum() == delta.signum())
        .map_or(0.0, |(_, _, _, part)| part);
    let amount = (delta + carry).clamp(-6.0, 6.0);
    let steps = amount.trunc() as i32;
    let old = WHEEL_LAYOUTS.iter().position(|mode| *mode == view.layout).unwrap_or(0);
    let next = (old as i32 + steps).clamp(0, WHEEL_LAYOUTS.len() as i32 - 1) as usize;
    if old != next {
        view.layout = WHEEL_LAYOUTS[next];
        layout.set_view(place, view);
        state.here.scroll = 0.0;
        state.restore_scroll = true;
    }
    ui.data_mut(|d| d.insert_temp(id, (place, view.layout, now, amount.fract())));
}

fn icon_side(size: IconSize) -> f32 {
    match size {
        IconSize::Small => 24.0,
        IconSize::Medium => 44.0,
        IconSize::Large => 76.0,
        IconSize::ExtraLarge => 120.0,
    }
}

#[derive(Copy, Clone)]
struct Grid {
    mode: ViewLayout,
    item: egui::Vec2,
    per_line: usize,
    viewport: egui::Vec2,
    heading_height: f32,
}
impl Grid {
    fn new(mode: ViewLayout, available: egui::Vec2, scroll_width: f32, search: bool, line_height: f32) -> Self {
        let viewport = available.max(vec2(40.0, 36.0));
        // Reserve native scrollbar width so a scrollbar appearing never oscillates the number of columns.
        let width = (viewport.x - scroll_width).max(35.0);
        let heading_height = (line_height + 8.0).max(27.0);
        let (item, per_line) = match mode {
            ViewLayout::Details => (vec2(width, 27.0_f32.max(line_height + 6.0)), 1),
            ViewLayout::List => {
                let col = width.clamp(130.0, 228.0);
                let line = if search {
                    2.0 * line_height + 6.0
                } else {
                    (line_height + 6.0).max(25.0)
                };
                (
                    vec2(col, line),
                    ((viewport.y - scroll_width - heading_height - 4.0) / line).floor().max(1.0) as usize,
                )
            }
            ViewLayout::Icons(size) => {
                let side = icon_side(size);
                let cols = (width / (side + 48.0).max(5.5 * line_height).max(96.0)).floor().max(1.0) as usize;
                let captions = 2.0 * line_height + if search { line_height + 3.0 } else { 0.0 };
                (vec2(width / cols as f32, side + 14.0 + captions), cols)
            }
        };
        Self {
            mode,
            item,
            per_line,
            viewport,
            heading_height,
        }
    }
    fn horizontal(self) -> bool {
        self.mode == ViewLayout::List
    }
    fn heading(self, label: &str) -> f32 {
        if label.is_empty() { 0.0 } else { self.heading_height }
    }
    fn advance(self, section: &Section) -> f32 {
        let bands = section.entries.len().div_ceil(self.per_line) as f32;
        if self.horizontal() {
            bands.max(if section.key.is_some() { 1.0 } else { 0.0 }) * self.item.x
        } else {
            self.heading(&section.label) + bands * self.item.y
        }
    }
    fn position(self, section: &Section, start: f32, local: usize) -> egui::Pos2 {
        let band = local / self.per_line;
        let slot = local % self.per_line;
        let heading = self.heading(&section.label);
        if self.horizontal() {
            pos2(start + band as f32 * self.item.x, heading + slot as f32 * self.item.y)
        } else {
            pos2(slot as f32 * self.item.x, start + heading + band as f32 * self.item.y)
        }
    }
    fn visible(self, section: &Section, start: f32, viewport: Rect) -> Range<usize> {
        let (min, max, base, pitch) = if self.horizontal() {
            (viewport.left(), viewport.right(), start, self.item.x)
        } else {
            (viewport.top(), viewport.bottom(), start + self.heading(&section.label), self.item.y)
        };
        let count = section.entries.len();
        let start = (((min - base) / pitch).floor().max(0.0) as usize)
            .saturating_mul(self.per_line)
            .min(count);
        let end = (((max - base) / pitch).ceil().max(0.0) as usize)
            .saturating_mul(self.per_line)
            .min(count);
        start..end.max(start)
    }
}

fn caption_line_height(ui: &Ui) -> f32 {
    ui.fonts_mut(|fonts| fonts.row_height(&FontId::proportional(13.0))).ceil() + 1.0
}

fn name_in(ui: &Ui, rect: Rect, text: &str, color: Color32, center: bool, lines: usize) -> Rect {
    let font = FontId::proportional(13.0);
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, rect.width().max(1.0));
    job.wrap.max_rows = lines;
    job.wrap.break_anywhere = lines == 1;
    if center {
        job.halign = egui::Align::Center;
    }
    let galley = ui.painter().layout_job(job);
    let x = if center { rect.center().x } else { rect.left() };
    // Tiles share a first caption line below their icons. Rows and columns
    // align their letters to the same optical center as the neighboring glyph.
    let line_center = if center {
        rect.top() + galley.size().y * 0.5
    } else {
        rect.center().y
    };
    let origin = crate::ui::widgets::row_text_origin(ui.painter(), &galley, x, line_center);
    let painted = galley.rect.translate(origin.to_vec2());
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(origin, galley, color);
    painted
}

fn section_heading(
    ui: &mut Ui,
    palette: &Palette,
    rect: Rect,
    id: egui::Id,
    section: &Section,
    collapsed: bool,
) -> (Response, Option<bool>) {
    let response = ui.interact(rect, id, Sense::click()).on_hover_text(&section.label);
    let count = format!("({})", aurora_i18n::integer(section.count as i64));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::DisclosureTriangle,
            ui.is_enabled(),
            format!("{} {count}", section.label),
        )
    });
    ui.ctx().accesskit_node_builder(id, |node| node.set_expanded(!collapsed));
    if response.clicked() {
        response.request_focus();
    }
    let mut change = response.clicked().then_some(!collapsed);
    if response.has_focus() {
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
        ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, Key::ArrowLeft) {
                change = Some(true);
            } else if input.consume_key(egui::Modifiers::NONE, Key::ArrowRight) {
                change = Some(false);
            }
        });
    }
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if response.hovered() {
        painter.rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let caret = Rect::from_center_size(pos2(rect.left() + 11.0, rect.center().y), vec2(9.0, 7.0));
    super::item_art::paint_tabler(
        &painter,
        if collapsed { "chevron-right" } else { "chevron-down" },
        caret,
        palette.ink,
    );
    let count_width = painter
        .layout_no_wrap(count.clone(), FontId::proportional(13.0), palette.muted)
        .size()
        .x
        .ceil();
    let label_left = rect.left() + 23.0;
    let text_right = rect.right() - 5.0;
    let label_right = (text_right - count_width - 6.0).max(label_left);
    let label_rect = Rect::from_x_y_ranges(label_left..=label_right, rect.y_range());
    let painted = name_in(ui, label_rect, &section.label, palette.ink, false, 1);
    let count_left = painted.right().min(label_right) + 6.0;
    if count_left < text_right {
        name_in(
            ui,
            Rect::from_x_y_ranges(count_left..=text_right, rect.y_range()),
            &count,
            palette.muted,
            false,
            1,
        );
    }
    painter.hline(rect.x_range(), rect.bottom(), ui.visuals().widgets.noninteractive.bg_stroke);
    (response, change)
}

fn picture(
    ui: &Ui,
    inv: &Inventory,
    row: Row,
    rect: Rect,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut Vec<(Uuid, u32)>,
    materials: &mut MaterialTiles,
) -> Option<Rect> {
    let id = row.id()?;
    // LLInventoryGallery prefers assigned inventory thumbnails. Only material assets
    // without one fall through to the same source the Texture Library opens, including
    // links. RLVa texture restrictions apply equally to generated swatches and images.
    if !inv.can_preview(0) {
        return None;
    }
    let (asset, tint) = if let Some(image) = super::super::hover::asset(inv, id) {
        (image, crate::ui::widgets::PICTURE_TINT)
    } else {
        let item = crate::ui::texture_library::catalogue::resource(inv, id)?;
        if item.asset_type != 57 || !inv.can_preview(item.asset_type) {
            return None;
        }
        let material = materials.tile(crate::ui::texture_library::catalogue::asset(item))?;
        let Some(map) = material.map else {
            ui.painter()
                .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, material.tint);
            ui.painter().rect_stroke(
                rect,
                ui.visuals().widgets.inactive.corner_radius,
                ui.visuals().widgets.noninteractive.bg_stroke,
                egui::StrokeKind::Inside,
            );
            return Some(rect);
        };
        (map, material.tint)
    };
    wanted.push((asset, rect.width().max(rect.height()).ceil() as u32));
    if let Some(handle) = images.get(&asset) {
        let source = handle.size_vec2().max(vec2(1.0, 1.0));
        let scale = (rect.width() / source.x).min(rect.height() / source.y);
        let dest = Rect::from_center_size(rect.center(), source * scale);
        ui.painter()
            .image(handle.id(), dest, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), tint);
        return Some(dest);
    }
    None
}

fn cell(
    ui: &mut Ui,
    art: &RowStyle,
    inv: &Inventory,
    row: Row,
    custom_icon: Option<&'static str>,
    rect: Rect,
    view: ViewLayout,
    columns: [f32; 4],
    text: &str,
    location: Option<&str>,
    selected: bool,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut Vec<(Uuid, u32)>,
    materials: &mut MaterialTiles,
) -> (Response, Option<Rect>, Rect) {
    let frame = rect.shrink2(if matches!(view, ViewLayout::Icons(_)) {
        vec2(3.0, 2.5)
    } else {
        vec2(1.0, 0.8)
    });
    let response = ui.interact(frame, ui.scope_id().with(("explorer-entry", row)), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, ui.is_enabled(), selected, text));
    let color = paint_row_selection(ui, art.palette, &response, frame, selected);
    let glyph = art.resolve(inv, row, false, selected).with_symbol(custom_icon);
    let favorite = row.id().is_some_and(|id| rules::is_favorite(inv, id));
    let activity = row.id().and_then(|id| art.facts.worn_label(inv, id));
    let mut gutter = None;
    let edit_rect;
    match view {
        ViewLayout::Details | ViewLayout::List => {
            let [name_width, type_width, date_width, location_width] = if view == ViewLayout::Details {
                columns
            } else {
                [rect.width(), 0.0, 0.0, 0.0]
            };
            art.glyph(
                ui.painter(),
                glyph,
                Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(17.0, 17.0)),
            );
            let two_lines = view == ViewLayout::List && location.is_some();
            let name_top = rect.top();
            let name_bottom = if two_lines { rect.center().y } else { rect.bottom() };
            let favorite_room = if favorite { 17.0 } else { 0.0 };
            let name_rect = Rect::from_min_max(
                pos2(rect.left() + 30.0, name_top),
                pos2(rect.left() + name_width - 4.0 - favorite_room, name_bottom),
            );
            edit_rect = name_rect;
            let label = name_in(ui, name_rect, text, color, false, 1);
            if favorite {
                let area = Rect::from_center_size(pos2(name_rect.right() + 8.0, name_rect.center().y), vec2(13.0, 13.0));
                art.glyph(ui.painter(), Glyph::navigation("star-filled", art.palette.amber), area);
            }
            let mut used = label.right();
            if let Some(status) = activity {
                let remaining = Rect::from_min_max(pos2(used + 7.0, name_top), name_rect.right_bottom());
                if remaining.width() >= 34.0 {
                    used = name_in(ui, remaining, status, detail_ink(art.palette, art.palette.teal, selected), false, 1).right();
                }
            }
            if name_rect.right() - used > 10.0 {
                gutter = Some(Rect::from_min_max(
                    pos2(used + 5.0, frame.top()),
                    pos2(name_rect.right(), frame.bottom()),
                ));
            }
            let ty_x = rect.left() + name_width;
            if type_width > 0.0 {
                let col = Rect::from_min_size(pos2(ty_x + 4.0, rect.top()), vec2((type_width - 8.0).max(1.0), rect.height()));
                name_in(ui, col, &kind(inv, row), color, false, 1);
            }
            if date_width > 0.0 {
                let col = Rect::from_min_size(
                    pos2(ty_x + type_width + 4.0, rect.top()),
                    vec2((date_width - 8.0).max(1.0), rect.height()),
                );
                let date = local_date_text(date(inv, row), super::super::view::now());
                name_in(ui, col, &date, color, false, 1);
            }
            if let Some(path) = location
                && (two_lines || location_width > 0.0)
            {
                let col = if two_lines {
                    Rect::from_min_max(pos2(rect.left() + 30.0, rect.center().y), pos2(rect.right() - 5.0, rect.bottom()))
                } else {
                    Rect::from_min_max(
                        pos2(rect.right() - location_width + 4.0, rect.top()),
                        pos2(rect.right() - 4.0, rect.bottom()),
                    )
                };
                name_in(ui, col, path, detail_ink(art.palette, art.palette.muted, selected), false, 1);
            }
        }
        ViewLayout::Icons(size) => {
            let side = icon_side(size).min((frame.width() - 7.0).max(1.0));
            let picture_rect = Rect::from_center_size(pos2(rect.center().x, frame.top() + 4.0 + side / 2.0), egui::Vec2::splat(side));
            if custom_icon.is_none()
                && let Some(image_rect) = picture(ui, inv, row, picture_rect, images, wanted, materials)
            {
                art.thumbnail_badge(ui.painter(), glyph, image_rect, side);
            } else {
                art.glyph(
                    ui.painter(),
                    glyph,
                    Rect::from_center_size(picture_rect.center(), egui::Vec2::splat(side * 0.84)),
                );
            }
            if favorite {
                let size = (side * 0.22).clamp(11.0, 19.0);
                let star = Rect::from_min_size(pos2(frame.right() - size - 2.0, frame.top() + 2.0), vec2(size, size));
                art.glyph(ui.painter(), Glyph::navigation("star-filled", art.palette.amber), star);
            }
            let label = Rect::from_min_max(
                pos2(frame.left() + 2.0, picture_rect.bottom() + 3.0),
                pos2(frame.right() - 2.0, frame.bottom()),
            );
            let path_height = if location.is_some() { caption_line_height(ui) + 3.0 } else { 0.0 };
            let title_rect = Rect::from_min_max(label.min, pos2(label.right(), label.bottom() - path_height));
            edit_rect = title_rect;
            name_in(
                ui,
                title_rect,
                text,
                if activity.is_some() && !selected { art.palette.teal } else { color },
                true,
                2,
            );
            if let Some(path) = location {
                let path_rect = Rect::from_min_max(pos2(label.left(), title_rect.bottom() + 3.0), label.max);
                name_in(ui, path_rect, path, detail_ink(art.palette, art.palette.muted, selected), true, 1);
            }
        }
    }
    (response, gutter, edit_rect)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn contents(
    ui: &mut Ui,
    art: &RowStyle,
    inv: &Inventory,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    avatar: &str,
    open: &mut Option<Row>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut Vec<(Uuid, u32)>,
    materials: &mut MaterialTiles,
    files: &file_actions::Menu,
    actions: &mut Vec<InvAction>,
) -> Option<(Row, Rect, Rect)> {
    let mut editor = None;
    wheel_layout(ui, layout, state);
    let view = layout.view(state.here.location);
    let scrollbar = ui.spacing().scroll.allocated_width().max(7.0);
    let columns = if view.layout == ViewLayout::Details {
        let widths = dimensions::header(ui, layout, state, view, (ui.available_width() - scrollbar).max(1.0));
        ui.add_space(4.0);
        widths
    } else {
        [0.0; 4]
    };
    let height = ui.available_height().max(1.0);
    let line_height = caption_line_height(ui);
    let grid = Grid::new(
        view.layout,
        vec2(ui.available_width(), height),
        scrollbar,
        !state.here.query.trim().is_empty(),
        line_height,
    );
    let mut area = if grid.horizontal() {
        egui::ScrollArea::horizontal()
    } else {
        egui::ScrollArea::vertical()
    };
    area = area
        .auto_shrink([false, false])
        .min_scrolled_height(1.0)
        .max_height(height)
        .content_margin(egui::Margin::ZERO)
        .scroll_source(egui::scroll_area::ScrollSource {
            drag: egui::scroll_area::DragScroll::Never,
            ..Default::default()
        });
    let sections = state.sections.clone();
    if let Some(row) = state.reveal_row.take()
        && let Some(index) = state.rows.iter().position(|r| *r == row)
    {
        let mut base: f32 = 0.0;
        for section in &sections {
            if section.entries.contains(&index) {
                let pos = grid.position(section, base, index - section.entries.start);
                state.here.scroll = if grid.horizontal() {
                    (pos.x - grid.viewport.x * 0.35).max(0.0)
                } else {
                    (pos.y - height * 0.35).max(0.0)
                };
                state.restore_scroll = true;
                break;
            }
            base += grid.advance(section);
        }
    }
    area = crate::ui::widgets::scroll_offset::show(ui, area, ("explorer-contents", grid.horizontal()));
    if state.restore_scroll {
        area = if grid.horizontal() {
            area.horizontal_scroll_offset(state.here.scroll)
        } else {
            area.vertical_scroll_offset(state.here.scroll)
        };
        state.restore_scroll = false;
    }
    let before = (state.here.location, state.here.raw);
    input::begin(state);
    state.input.columns = if grid.horizontal() { 1 } else { grid.per_line };
    state.input.column_height = if grid.horizontal() { grid.per_line } else { 0 };
    let mut collapse_anchor = None;
    let output = area.show_viewport(ui, |canvas, viewport| {
        let origin = canvas.min_rect().min.to_vec2();
        let screen = viewport.translate(origin).intersect(canvas.clip_rect());
        let background = input::surface(canvas, screen, viewport.min.to_vec2(), state);
        let length = sections.iter().map(|s| grid.advance(s)).sum::<f32>();
        canvas.set_min_size(if grid.horizontal() {
            vec2(length, (height - scrollbar).max(1.0))
        } else {
            vec2(grid.item.x * grid.per_line as f32, length)
        });
        let mut base: f32 = 0.0;
        for section in &sections {
            if let Some(key) = &section.key {
                let pos = if grid.horizontal() {
                    pos2(base.max(viewport.left()), 0.0)
                } else {
                    pos2(0.0, base)
                };
                let width = if grid.horizontal() {
                    (base + grid.advance(section) - pos.x).min(grid.viewport.x)
                } else {
                    grid.item.x * grid.per_line as f32
                };
                let header = Rect::from_min_size(pos + origin, vec2(width.max(0.0), grid.heading_height - 2.0));
                let hit = header.intersect(screen);
                if hit.is_positive() {
                    let place = state.location();
                    let collapsed = layout.group_collapsed(place, key);
                    let id = background.id.with((place, state.here.raw, key));
                    // Fit the count beside the visible label. Vertical scrolling keeps the original text baseline.
                    let heading_rect = Rect::from_x_y_ranges(hit.x_range(), header.y_range());
                    let (response, change) = section_heading(canvas, art.palette, heading_rect, id, section, collapsed);
                    input::section_header(state, hit, id);
                    if let Some(collapse) = change
                        && layout.set_group_collapsed(place, key, collapse)
                    {
                        // When a wide List group closes via its sticky title, keep that title within reach.
                        if collapse && grid.horizontal() && viewport.left() > base {
                            collapse_anchor = Some(base);
                        }
                        canvas.ctx().request_repaint();
                    }
                    context_menu(&response, art.palette, |ui| group_commands(ui, layout, state));
                }
            }
            for offset in grid.visible(section, base, viewport) {
                let index = section.entries.start + offset;
                let row = state.rows[index];
                let rect = Rect::from_min_size(grid.position(section, base, offset) + origin, grid.item);
                if !screen.intersects(rect) {
                    continue;
                }
                let location = search_location(inv, layout, state, row, avatar);
                let draft = state.folder_draft.as_ref().filter(|draft| draft.row(state) == Some(row));
                let name = draft.map_or_else(
                    || row_name(inv, layout, row, state.here.raw, avatar),
                    |d| d.display_name().to_owned(),
                );
                let opacity = canvas.opacity();
                if let Some(id) = row.id() {
                    canvas.multiply_opacity(super::super::clipboard::opacity(inv, files.native, id));
                }
                let custom_icon = draft.map(|d| d.glyph()).or_else(|| super::folder_icons::for_row(layout, row));
                let (r, gutter, name_rect) = cell(
                    canvas,
                    art,
                    inv,
                    row,
                    custom_icon,
                    rect,
                    view.layout,
                    columns,
                    &name,
                    location.as_deref(),
                    state.here.selected.contains(&row),
                    images,
                    wanted,
                    materials,
                );
                canvas.set_opacity(opacity);
                if draft.is_some() || (state.action_rename.is_some() && state.action_rename == row.id()) {
                    editor = Some((row, name_rect, screen));
                    continue;
                }
                if input::row(canvas, &r, row, files.prefs, inv, layout, state, open, gutter) {
                    let glyph = art.resolve(inv, row, false, true).with_symbol(custom_icon);
                    let thumbnail = row
                        .id()
                        .filter(|_| custom_icon.is_none())
                        .and_then(|id| super::super::hover::asset(inv, id))
                        .and_then(|asset| images.get(&asset))
                        .cloned();
                    super::drag_preview::start(canvas.ctx(), inv, art, &name, glyph, thumbnail);
                }
                if let Row::Place(Location::Folder(folder)) = row {
                    super::super::task_drop::target(canvas, art.palette, &r, inv, folder, actions);
                }
                let r = r.on_hover_ui(|tip| {
                    tip.label(&name);
                    if let Some(path) = &location {
                        tip.weak(path);
                    }
                    if let Some(id) = row.id() {
                        if let Some(label) = art.facts.worn_label(inv, id) {
                            tip.label(label);
                        }
                        if rules::is_favorite(inv, id) {
                            tip.label(tr!("ui-emoji-favorite"));
                        }
                    }
                    if location.is_none()
                        && let Some(id) = row.id()
                        && let Some(p) = rules::parent(inv, id)
                    {
                        tip.weak(super::super::inventory_path(inv, p));
                    }
                });
                context_menu(&r, art.palette, |ui| {
                    menus::selected(ui, art.palette, inv, layout, state, avatar, row, open, files, true)
                });
            }
            base += grid.advance(section);
        }
        let over_header = canvas
            .input(|input| input.pointer.latest_pos())
            .is_some_and(|pos| input::over_header(state, pos));
        if !over_header && background.secondary_clicked() {
            state.here.clear_selection();
            state.anchor = None;
            state.input.caret = None;
            background.request_focus();
        }
        // Object contents can be taken into a real inventory destination.
        // A virtual category or search spanning several folders has no single
        // physical parent; its real folder rows above are still valid targets.
        if !over_header && state.here.query.trim().is_empty() {
            let destination = match state.here.location {
                Location::Folder(folder) => Some(folder),
                Location::FullTree | Location::MyFolders => Some(inv.root),
                _ => None,
            };
            if let Some(folder) = destination {
                super::super::task_drop::target(canvas, art.palette, &background, inv, folder, actions);
            }
        }
        if !over_header || !background.secondary_clicked() {
            context_menu(&background, art.palette, |ui| {
                menus::background(ui, art.palette, inv, layout, state, avatar, files, true)
            });
        }
    });
    if before == (state.here.location, state.here.raw) {
        let offset = output.state.clamped_offset();
        if let Some(anchor) = collapse_anchor {
            state.here.scroll = anchor;
            state.restore_scroll = true;
        } else {
            state.here.scroll = if grid.horizontal() { offset.x } else { offset.y };
        }
        input::background(ui, output.inner_rect.intersect(ui.clip_rect()), art.palette, inv, layout, state);
        super::super::drag::navigation::scroll_view(ui, &output, grid.horizontal(), input::selecting(state));
    }
    editor
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explorer_name_groups_keep_folders_and_items_together_and_sort_members_independently() {
        use super::super::tests::{fixture, id};
        let mut inv = fixture();
        let layout = Preferences::default();
        for (n, name) in [
            (31, "Apple 10"),
            (32, "apple 2"),
            (33, "Iris"),
            (34, "Z"),
            (35, "1A"),
            (36, "!"),
            (37, "北"),
        ] {
            let mut item = inv.items[&id(20)].clone();
            item.id = id(n);
            item.name = name.into();
            inv.items.insert(item.id, item);
        }
        let archive = Row::Place(Location::Folder(id(4)));
        let mut rows: Vec<_> = (31..=37).rev().map(|n| Row::Item(id(n))).chain([archive]).collect();
        let mut view = DirectoryView {
            group: GroupBy::Name,
            ..Default::default()
        };
        let expected = [
            NameBucket::Symbols,
            NameBucket::Numbers,
            NameBucket::AtoH,
            NameBucket::ItoP,
            NameBucket::QtoZ,
            NameBucket::Initial('北'),
        ];
        for groups_descending in [false, true] {
            view.group_descending = groups_descending;
            for items_descending in [false, true] {
                view.descending = items_descending;
                let sections = order(&inv, &layout, &mut rows, view, false, "");
                let mut keys = expected.map(|key| Some(GroupKey::Name(key)));
                if groups_descending {
                    keys.reverse();
                }
                assert!(sections.iter().map(|s| s.key).eq(keys));
                let apples = sections.iter().find(|s| s.key == Some(GroupKey::Name(NameBucket::AtoH))).unwrap();
                let ids = if items_descending { [31, 32] } else { [32, 31] };
                assert_eq!(
                    rows[apples.entries.clone()],
                    [archive, Row::Item(id(ids[0])), Row::Item(id(ids[1]))]
                );
                assert_eq!(apples.count, 3);
            }
        }
        assert_eq!(name_bucket("   Öland"), NameBucket::Initial('Ö'));
        assert_eq!(name_bucket(""), NameBucket::Symbols);
        assert_eq!(name_bucket("９"), NameBucket::Numbers);
    }

    #[test]
    fn explorer_date_groups_follow_calendar_boundaries_and_keep_undated_last() {
        use super::super::tests::{fixture, id};
        use DateBucket::*;
        use chrono::TimeZone;
        let jan = |day| NaiveDate::from_ymd_opt(2025, 1, day).unwrap();
        let dec = |day| NaiveDate::from_ymd_opt(2024, 12, day).unwrap();
        // January 8 is Wednesday. The previous calendar week crosses the year edge.
        let today = jan(8);
        for (date, expected) in [
            (None, Unknown),
            (Some(jan(9)), Future),
            (Some(today), Today),
            (Some(jan(7)), Yesterday),
            (Some(jan(6)), ThisWeek),
            (Some(jan(1)), LastWeek),
            (Some(dec(30)), LastWeek),
            (Some(dec(29)), LastMonth),
            (NaiveDate::from_ymd_opt(2024, 11, 1), LastYear),
            (NaiveDate::from_ymd_opt(2023, 12, 31), Older),
        ] {
            assert_eq!(date_bucket(date, today), expected);
        }
        assert_eq!(date_bucket(Some(dec(31)), jan(1)), Yesterday);
        assert_eq!(date_bucket(Some(jan(1)), jan(30)), ThisMonth);
        let april = NaiveDate::from_ymd_opt(2025, 4, 7).unwrap();
        assert_eq!(date_bucket(Some(jan(1)), april), ThisYear);
        assert_eq!(date_bucket(NaiveDate::from_ymd_opt(2025, 3, 31), april), LastWeek);
        assert_eq!(date_bucket(NaiveDate::from_ymd_opt(2024, 2, 29), april), LastYear);
        assert_eq!(local_day(0), None);
        assert_eq!(local_day(i64::MAX), None);

        let mut inv = fixture();
        let prefs = Preferences::default();
        inv.items.get_mut(&id(20)).unwrap().created_at = Local::now().timestamp();
        let mut rows = vec![Row::Place(Location::Folder(id(4))), Row::Item(id(20))];
        for reverse in [false, true] {
            let view = DirectoryView {
                group: GroupBy::Date,
                group_descending: reverse,
                ..Default::default()
            };
            let sections = order(&inv, &prefs, &mut rows, view, false, "");
            assert_eq!(
                sections.iter().map(|s| s.key).collect::<Vec<_>>(),
                [Some(GroupKey::Date(Today)), Some(GroupKey::Date(Unknown))]
            );
        }
        let noon = Local.with_ymd_and_hms(2025, 1, 8, 12, 0, 0).single().unwrap().timestamp();
        assert_eq!(local_day(noon), Some(today));
        assert_eq!(local_date_text(noon, noon), crate::ui::land::day_month(None, 1, 8));
    }

    #[test]
    fn explorer_fast_date_buckets_match_local_dates_at_dst_and_calendar_edges() {
        let zones = [chrono_tz::America::New_York, chrono_tz::America::Santiago, chrono_tz::Pacific::Apia];
        for zone in zones {
            for (y, m, d) in [(2024, 3, 11), (2024, 11, 4), (2024, 9, 9), (2011, 12, 31), (2025, 1, 2)] {
                let today = NaiveDate::from_ymd_opt(y, m, d).unwrap();
                let groups = DateGroups::new(zone, today);
                let midnight_utc = today.and_time(chrono::NaiveTime::MIN).and_utc();
                for hours in (-24 * 370..=48).step_by(3) {
                    let sample = midnight_utc + chrono::TimeDelta::hours(hours);
                    let local = sample.with_timezone(&zone);
                    let expected = date_bucket(Some(local.date_naive()), today);
                    assert_eq!(groups.bucket(sample.timestamp()), expected, "{zone} {local} relative to {today}");
                }
                // Exercise the one-second edges of each actual midnight, including
                // 23/25-hour New York days and Santiago/Apia's missing midnights.
                for delta in -14..=2 {
                    let date = today + chrono::TimeDelta::days(delta);
                    let base = date.and_time(chrono::NaiveTime::MIN);
                    for instant in [zone.from_local_datetime(&base).earliest(), zone.from_local_datetime(&base).latest()]
                        .into_iter()
                        .flatten()
                    {
                        for delta in [-1, 0, 1] {
                            let sample = instant + chrono::TimeDelta::seconds(delta);
                            assert_eq!(groups.bucket(sample.timestamp()), date_bucket(Some(sample.date_naive()), today));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn real_home_folders_are_separate_from_pins_and_loose_items_have_a_visible_parent() {
        use super::super::tests::{fixture, id};
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let container = id(3);
        let child = id(4);
        inv.folders.get_mut(&child).unwrap().info.parent = container;
        inv.folders.get_mut(&container).unwrap().children.push(child);
        let (initial, sections) = home(&inv, &layout, "Tester");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].label, tr!("explorer-home-categories"));
        assert_eq!(sections[1].label, tr!("explorer-home-utilities"));
        assert!(!initial.contains(&Row::Place(Location::Folder(child))));
        layout.set_home_folder(container);
        assert!(layout.pin_location(&inv, Location::Folder(child)));
        let mut view = layout.view(Location::Home);
        view.group = GroupBy::Type;
        layout.set_view(Location::Home, view);
        let (rows, sections) = home(&inv, &layout, "Tester");
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].label, tr!("explorer-home-categories"));
        assert_eq!(sections[1].label, tr!("explorer-home-utilities"));
        assert_eq!(sections[2].label, tr!("explorer-home-folders"));
        assert_eq!(rows[sections[2].entries.clone()], [Row::Place(Location::Folder(child))]);
        assert_eq!(rows.iter().filter(|r| **r == Row::Place(Location::Folder(child))).count(), 1);
        let mut state = ExplorerState::default();
        state.select(Row::Place(Location::Folder(child)), egui::Modifiers::NONE);
        assert!(!state.dashboard_shortcuts(&inv, &layout));
        state.here.selected.insert(Row::Place(Location::MyFolders));
        assert!(state.dashboard_shortcuts(&inv, &layout));

        assert_eq!(layout.location_parent(&inv, Location::Folder(child)), Some(Location::Home));
        assert!(!layout.children(&inv, Location::MyFolders).contains(&container));
        assert!(layout.children(&inv, Location::MyFolders).contains(&id(20)));
        assert_eq!(layout.parent(&inv, id(20)), Location::MyFolders);
        assert!(layout.children(&inv, Location::FullTree).contains(&container));
        assert!(layout.place(&inv, child, Location::MyFolders));
        assert!(layout.home_children(&inv).is_empty());
        assert!(layout.restore(child));
        assert_eq!(layout.home_children(&inv), vec![Location::Folder(child)]);
        // Only the saved UUID qualifies, and a moved or reserved folder stops
        // serving as the container even if a stale index still references it.
        inv.folders.get_mut(&container).unwrap().info.name = "Renamed".into();
        let layout: Preferences = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(model::home_folder(&inv, &layout), Some(container));
        inv.folders.get_mut(&container).unwrap().info.name = "#RLV".into();
        assert_eq!(model::home_folder(&inv, &layout), None);
        inv.folders.get_mut(&container).unwrap().info.parent = inv.lib_root;
        assert!(layout.home_children(&inv).is_empty());
    }

    #[test]
    fn organized_home_unpinned_builtins_remain_browsable_without_duplicates_or_personal_clutter() {
        use super::super::tests::{fixture, id};
        let inv = fixture();
        let mut layout = Preferences::default();
        let photos = Location::Folder(id(7));
        let appearance = Location::Group(Group::Appearance);
        let builtins = [Location::Library, photos, appearance];
        let personal = Location::Folder(id(3));
        let other_label = tr!("explorer-home-other-locations");
        let (_, sections) = home(&inv, &layout, "Tester");
        assert!(sections.iter().all(|s| s.label != other_label));
        let group = layout.create_pin_group("Projects").unwrap();
        assert!(layout.pin_location(&inv, personal));
        assert!(layout.move_pin_to_group(&inv, personal, Some(group)));
        for place in builtins {
            assert!(layout.unpin_location(&inv, place));
        }
        let (rows, sections) = home(&inv, &layout, "Tester");
        let others = sections.last().unwrap();
        assert_eq!(others.label, other_label);
        let actual: HashSet<_> = rows[others.entries.clone()].iter().copied().collect();
        assert_eq!(actual, HashSet::from(builtins.map(Row::Place)));
        for place in builtins {
            assert_eq!(rows.iter().filter(|&&r| r == Row::Place(place)).count(), 1);
        }
        let projects = sections.iter().find(|s| s.label == "Projects").unwrap();
        assert_eq!(rows[projects.entries.clone()], [Row::Place(personal)]);
        assert!(!rows.contains(&Row::Place(Location::Folder(id(4)))));
        assert!(!rows.contains(&Row::Item(id(20))));
        assert!(layout.unpin_location(&inv, personal));
        assert!(!home(&inv, &layout, "Tester").0.contains(&Row::Place(personal)));
        assert!(layout.children(&inv, Location::MyFolders).contains(&id(3)));
        assert_eq!(inv.folders[&id(7)].info.parent, inv.root);
        assert_eq!(inv.folders[&id(3)].info.parent, inv.root);
        assert_eq!(inv.items[&id(20)].parent, id(3));
        for place in builtins {
            assert!(layout.pin_location(&inv, place));
        }
        assert!(home(&inv, &layout, "Tester").1.iter().all(|s| s.label != other_label));
    }

    #[test]
    fn organized_home_sorts_every_tile_within_named_sections_without_changing_sidebar_order() {
        use super::super::tests::{fixture, id};
        let inv = fixture();
        let mut prefs = Preferences::default();
        for place in [Location::Folder(id(4)), Location::Folder(id(3))] {
            assert!(prefs.pin_location(&inv, place));
        }
        let pins = prefs.pins(&inv);
        let (asc, sections) = home(&inv, &prefs, "Tester");
        let mut view = prefs.view(Location::Home);
        view.descending = true;
        prefs.set_view(Location::Home, view);
        let (desc, after) = home(&inv, &prefs, "Tester");
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[2].label, tr!("explorer-home-pinned-folders"));
        for (a, b) in sections.iter().zip(&after) {
            assert_eq!(a.label, b.label);
            assert!(asc[a.entries.clone()].iter().rev().eq(desc[b.entries.clone()].iter()));
        }
        assert_eq!(
            asc[sections[2].entries.clone()],
            [Row::Place(Location::Folder(id(4))), Row::Place(Location::Folder(id(3)))]
        );
        assert!(asc[sections[0].entries.clone()].contains(&Row::Place(Location::MyFolders)));
        assert!(asc[sections[1].entries.clone()].contains(&Row::Place(Location::Library)));
        assert_eq!(prefs.pins(&inv), pins);
        view.sort = SortKey::Date;
        view.group = GroupBy::Type;
        prefs.set_view(Location::Home, view);
        assert_eq!(home(&inv, &prefs, "Tester").0, desc);
        assert_eq!(prefs.view(Location::Home), view);
    }

    #[test]
    fn organized_folder_and_search_sort_by_type_date_and_name_in_every_layout() {
        use super::super::tests::{fixture, id};
        let mut inv = fixture();
        let place = Location::Folder(id(3));
        let original = inv.items[&id(20)].clone();
        for (n, number, asset, ty, date) in [
            (41, 10, 7, 7, 100),
            (42, 2, 7, 7, 100),
            (43, 100, 6, 17, 90),
            (44, 5, 6, 6, 0),
            (45, 3, 24, 7, 70),
        ] {
            let mut item = original.clone();
            item.id = id(n);
            item.name = format!("Sort test {number}");
            item.asset_type = asset;
            item.inv_type = ty;
            item.created_at = date;
            if asset == 24 {
                item.asset_id = id(43);
            }
            inv.items.insert(item.id, item);
        }
        inv.folders.get_mut(&id(3)).unwrap().items = (41..=45).map(id).collect();
        let mut prefs = Preferences::default();
        for location in [place, Location::Home] {
            let mut state = ExplorerState::at(location);
            if location == Location::Home {
                state.here.query = "Sort test".into();
            }
            for mode in WHEEL_LAYOUTS {
                let mut view = DirectoryView {
                    layout: mode,
                    sort: SortKey::Name,
                    ..Default::default()
                };
                for (sort, descending, expected) in [
                    (SortKey::Name, false, [42, 45, 44, 41, 43]),
                    (SortKey::Name, true, [43, 41, 44, 45, 42]),
                    (SortKey::Date, false, [45, 43, 42, 41, 44]),
                    (SortKey::Date, true, [41, 42, 43, 45, 44]),
                ] {
                    view.sort = sort;
                    view.descending = descending;
                    prefs.set_view(location, view);
                    refresh_rows(&inv, &mut prefs, &mut state, "Tester");
                    assert_eq!(state.rows, expected.map(|n| Row::Item(id(n))));
                }
                view.sort = SortKey::Type;
                view.group = GroupBy::Type;
                view.descending = false;
                prefs.set_view(location, view);
                refresh_rows(&inv, &mut prefs, &mut state, "Tester");
                let asc = state.rows.clone();
                assert_eq!(state.sections.len(), 3);
                let headings: Vec<_> = state.sections.iter().map(|s| s.label.clone()).collect();
                view.descending = true;
                prefs.set_view(location, view);
                refresh_rows(&inv, &mut prefs, &mut state, "Tester");
                assert!(headings.iter().eq(state.sections.iter().map(|s| &s.label)));
                for section in &state.sections {
                    assert!(
                        asc[section.entries.clone()]
                            .iter()
                            .rev()
                            .eq(state.rows[section.entries.clone()].iter())
                    );
                }
                view.group_descending = true;
                prefs.set_view(location, view);
                refresh_rows(&inv, &mut prefs, &mut state, "Tester");
                assert!(asc.iter().rev().eq(state.rows.iter()));
                assert!(headings.iter().rev().eq(state.sections.iter().map(|s| &s.label)));

                // Objects, worn attachments, and links to those objects match the same filter.
                state.here.type_filter = Some(6);
                state.rows_key = None;
                refresh_rows(&inv, &mut prefs, &mut state, "Tester");
                assert_eq!(
                    state.rows.iter().copied().collect::<HashSet<_>>(),
                    HashSet::from([43, 44, 45].map(|n| Row::Item(id(n))))
                );
                state.clear_filter();
            }
        }
        assert_eq!(prefs.view(Location::MyFolders), DirectoryView::default());
    }

    #[test]
    fn organized_large_icons_list_and_details_have_distinct_virtualized_geometry() {
        let size = vec2(600.0, 310.0);
        let section = Section {
            key: None,
            label: String::new(),
            entries: 0..10_000,
            count: 10_000,
        };
        let area = Rect::from_min_size(egui::Pos2::ZERO, size);
        let details = Grid::new(ViewLayout::Details, size, 10.0, false, 18.0);
        let small = Grid::new(ViewLayout::Icons(IconSize::Small), size, 10.0, false, 18.0);
        let large = Grid::new(ViewLayout::Icons(IconSize::Large), size, 10.0, false, 18.0);
        let list = Grid::new(ViewLayout::List, size, 10.0, false, 18.0);
        assert_eq!(details.per_line, 1);
        assert!(small.per_line > large.per_line);
        assert!(large.item.y > small.item.y);
        assert!(list.horizontal());
        assert_eq!(list.position(&section, 0.0, 1).x, 0.0);
        assert!(list.position(&section, 0.0, list.per_line).x > 0.0);
        for grid in [details, small, large, list] {
            let visible = grid.visible(&section, 0.0, area);
            assert!(!visible.is_empty() && visible.len() < 100);
            let delta = if grid.horizontal() {
                vec2(3.0 * grid.item.x, 0.0)
            } else {
                vec2(0.0, 3.0 * grid.item.y)
            };
            let scrolled = grid.visible(&section, 0.0, area.translate(delta));
            assert_eq!(scrolled.start, grid.per_line * 3);
        }
    }
}

#[cfg(test)]
#[path = "organized_art_tests.rs"]
mod art_tests;
