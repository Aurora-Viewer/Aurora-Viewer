//! Area Search: rezzed objects received by the viewer, following
//! FSAreaSearch::isSearchableObject / matchObject (indra/newview/fsareasearch.cpp).
//! Avatars are excluded; attachments and linked prims are optional.
//! The right-click menu of the results follows menu_fs_area_search.xml and
//! FSPanelAreaSearchList::onContextMenuItemClick / onContextMenuItemEnable.

mod chart;
mod list;
pub(crate) mod model;
mod selection;
mod sheet;
mod small;
mod table;
#[cfg(test)]
mod tests;
mod wide;
pub(crate) use selection::Selection;
pub(crate) use table::{Column, Filters, Row, Table};

use super::board;
use super::menu;
use super::widgets::{self, Floater};
use crate::settings::area_search::{AreaSearch, DEFAULT_COLUMNS};
use crate::theme::Palette;
use egui::{RichText, TextureHandle, Vec2};
use model::Digest;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

pub const WINDOW: &str = "area_search";

/// The small window and the wide one.
const VIEWS: board::Views = board::Views {
    window: WINDOW,
    small: Vec2::new(430.0, 760.0),
    wide: Vec2::new(800.0, 760.0),
    from: 680.0,
};

#[derive(Default)]
pub struct AreaSearchUi {
    pub(crate) table: Table,
    visible_columns: Option<[bool; 10]>,
    pub(crate) save_defaults: bool,
    pub(crate) selection: Selection,
    pub(crate) refresh: bool,
    /// Clicks add to the selection ("Select several").
    several: bool,
    /// The details card shows every figure.
    card_open: bool,
    /// Objects, Owners, Details: the small window's tabs.
    tab: usize,
    /// The small window shows its filters.
    sheet: bool,
    /// The wide window's grown tile.
    grown: Option<wide::Tile>,
    /// The places to look in that differ here, and whether the wide window's
    /// header has room for them.
    scopes: Vec<model::Scope>,
    scope_shown: bool,
    /// The 3D view of the wide window.
    pub(crate) stage: super::object_stage::StageUi,
    /// The prim outlined in the 3D view.
    prim: Option<Uuid>,
    /// Profile pictures the window shows, for the app to fetch.
    pub(crate) wanted_pics: std::cell::RefCell<HashSet<Uuid>>,
    /// What the results come to, and the list it was made from.
    digest: Option<(u64, Arc<Digest>)>,
    /// The window was open last frame.
    was_open: bool,
    /// AURORA_DEMO_AREA_SEARCH's view; None: the one chosen last.
    pub(crate) forced_wide: Option<bool>,
    /// Where the right-click menu opened.
    menu_at: Option<egui::Pos2>,
    /// What RLVa hides of that menu, asked once as it opens: the selection
    /// can hold thousands of rows. The actions are checked again when sent.
    menu_locks: MenuLocks,
    /// The result list the selection was last checked against.
    checked: u64,
    /// AURORA_DEMO_AREA_SEARCH=menu: right click the second row once shown.
    pub(crate) demo_menu: bool,
    /// `select`: take the third row once shown.
    pub(crate) demo_select: bool,
}

/// What the resets take back (the "…" menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    All,
    Search,
    Filters,
    Ranges,
    Columns,
}

/// Figures of the single selected object that the list does not hold.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Facts {
    pub triangles: u32,
    pub texture_memory: u64,
}

/// What the window needs from the app this frame.
pub struct Env<'a> {
    /// The boundaries of the current parcel are known.
    pub parcel_known: bool,
    /// The current parcel is only a part of its region (or not known yet).
    pub parcel_part: bool,
    /// Other regions are in sight.
    pub neighbours: bool,
    /// Zoom is enabled within it (onContextMenuItemEnable "in_dd").
    pub draw_distance: f32,
    /// What RLVa refuses on the rows a menu opens on.
    pub locks: &'a dyn Fn(&[Uuid]) -> MenuLocks,
    pub pics: &'a HashMap<Uuid, TextureHandle>,
    pub facts: Option<Facts>,
}

/// What the window asks of the viewer. Several objects: every selected row,
/// as the Legacy Viewer acts on getAllSelected().
pub enum Action {
    Inspect(Vec<Uuid>),
    Zoom(Uuid),
    ScriptInfo(Vec<Uuid>),
    Touch(Vec<Uuid>),
    Sit(Uuid),
    Teleport(Uuid),
    Edit(Vec<Uuid>),
    Return(Vec<Uuid>),
    Delete(Vec<Uuid>),
    Blacklist(Vec<Uuid>),
    Buy(Uuid),
    /// "Filter My Objects": the owner field set to our name.
    FilterMine,
    /// The profile of an owner or a creator, or of the group that owns.
    Profile(Uuid),
    Group(Uuid),
}

/// What RLVa refuses on at least one selected row, by the words of the
/// menu's `on_visible` parameters (menu_fs_area_search.xml): an entry is
/// hidden, not greyed, when one of its words is refused
/// (FSPanelAreaSearchList::onContextMenuItemVisibleRLV, indra/newview/
/// fsareasearch.cpp). Under @interact, or for a row whose object is gone,
/// every word is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MenuLocks {
    /// RlvActions::canTouch.
    pub touch: bool,
    /// RlvActions::canEdit.
    pub edit: bool,
    /// rlvCanDeleteOrReturn.
    pub delete: bool,
    /// RlvActions::canSit.
    pub sit: bool,
    /// RlvActions::canBuyObject.
    pub buy: bool,
    /// RlvActions::canTeleportToLocal, to where the object is.
    pub teleport: bool,
}

impl MenuLocks {
    /// Every RLVa-checked entry hidden.
    pub const ALL: Self = Self {
        touch: true,
        edit: true,
        delete: true,
        sit: true,
        buy: true,
        teleport: true,
    };
}

impl AreaSearchUi {
    /// Whether the window shows its board of tiles this frame.
    pub(crate) fn is_wide(&self, ctx: &egui::Context, saved: bool) -> bool {
        board::is_wide(ctx, &VIEWS, saved, !self.was_open)
    }

    /// The window is closed: what it keeps only while shown goes.
    pub(crate) fn closed(&mut self) {
        self.was_open = false;
        self.digest = None;
        self.several = false;
    }

    /// The objects the 3D view shows: the selected ones, a few at most.
    pub(crate) fn staged(&self) -> &[Uuid] {
        const MOST: usize = 12;
        &self.selection.ids[..self.selection.ids.len().min(MOST)]
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        open: &mut bool,
        saved_wide: &mut bool,
        env: &Env<'_>,
    ) -> Option<Action> {
        let mut action = None;
        let wide = self.is_wide(ctx, *saved_wide);
        self.was_open = true;
        self.scopes = model::Scope::here(env.parcel_part, env.neighbours);
        let mut window = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-area-search"),
            egui::pos2(250.0, 60.0),
            VIEWS.size(ctx, *saved_wide),
        );
        window.min_size = Vec2::new(380.0, 420.0);
        let (mut resize, mut refresh) = (false, false);
        let (icon, tip) = board::size_button(wide);
        let mut window = window.action(icon, tip, &mut resize);
        if !wide {
            window = window
                .action("refresh", aurora_i18n::tr!("area-search-refresh"), &mut refresh)
                .title_state(p.success, aurora_i18n::tr!("area-search-live"));
        }
        window.show(ctx, p, open, |ui| {
            let d = self.digest();
            let (mut body, footer) = widgets::above_footer(ui);
            action = if wide {
                wide::show(self, &mut body, p, env, &d)
            } else {
                small::show(self, &mut body, p, env, &d)
            };
            footer.show(ui, |ui| {
                if let Some(asked) = self.footer(ui, p, env, &d, wide) {
                    action = Some(asked);
                }
            });
            self.table.filters.normalize();
            if let Some(pos) = self.menu_at {
                let mut chosen = None;
                menu::context_menu_at(ui, menu_id(), pos, p, |ui| chosen = self.menu(ui, p, env.draw_distance));
                if !egui::Popup::is_id_open(ui.ctx(), menu_id()) {
                    self.menu_at = None;
                }
                if chosen.is_some() {
                    action = chosen;
                }
            }
        });
        self.refresh |= refresh;
        board::after(ctx, &VIEWS, resize, wide, saved_wide);
        action
    }

    /// What the results come to, made again with each new list.
    fn digest(&mut self) -> Arc<Digest> {
        self.table.prepare();
        // an owner who has nothing left here holds nothing back
        if let Some(owner) = self.table.owner
            && !self.table.rows.iter().any(|r| r.owner_id == owner)
            && !self.table.rows.is_empty()
        {
            self.table.owner = None;
            self.table.prepare();
        }
        if self.demo_select && self.table.visible.len() > 2 && self.table.pending == 0 {
            self.demo_select = false;
            self.selection
                .click(self.table.rows[self.table.visible[2]].id, false, false, Vec::new);
        }
        self.check_selection();
        let generation = self.table.generation;
        if let Some((made, digest)) = &self.digest
            && *made == generation
        {
            return digest.clone();
        }
        let digest = Arc::new(Digest::of(&self.table));
        self.digest = Some((generation, digest.clone()));
        digest
    }

    /// The count, the notices and the buttons of what is selected.
    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, env: &Env<'_>, _d: &Digest, wide: bool) -> Option<Action> {
        let mut action = None;
        let count = aurora_i18n::tr!(
            "area-search-count",
            shown = self.table.visible.len() as i64,
            total = self.table.rows.len() as i64,
            pending = self.table.pending as i64
        );
        let notice = if let Some(error) = &self.table.regex_error {
            Some((
                aurora_i18n::tr!("area-search-regex-invalid").to_owned(),
                aurora_i18n::tr!("area-search-regex-error", error = error.as_str()),
            ))
        } else if self.table.filters.parcel_only && !env.parcel_known {
            let text = aurora_i18n::tr!("area-search-parcel-pending").to_owned();
            Some((text.clone(), text))
        } else {
            None
        };
        let any = !self.selection.is_empty();
        let single = self.selection.single();
        let size = Vec2::new(0.0, 26.0);
        let kind = |on: bool| {
            if on {
                widgets::ActionKind::Primary
            } else {
                widgets::ActionKind::Flat
            }
        };
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if any {
                    let b = widgets::action_button(ui, p, "", aurora_i18n::tr!("area-search-actions"), kind(false), size);
                    if b.clicked() {
                        let at = b.rect.left_top();
                        self.open_menu(ui, env, at);
                    }
                }
                // (the small window has the keyboard's and the menu's ways)
                if wide || !any {
                    let label = aurora_i18n::tr!("area-search-several");
                    if widgets::action_button(ui, p, "", label, kind(self.several), size)
                        .on_hover_text(aurora_i18n::tr!("area-search-several-tip"))
                        .clicked()
                    {
                        self.several = !self.several;
                    }
                }
                if any {
                    if widgets::action_button(ui, p, "", aurora_i18n::tr!("area-search-copy"), kind(false), size).clicked() {
                        let ids: Vec<_> = self.selection.ids.iter().map(Uuid::to_string).collect();
                        ui.ctx().copy_text(ids.join("\n"));
                    }
                    let zoom = aurora_i18n::tr!("area-search-zoom");
                    if widgets::action_button_if(ui, p, "", zoom, kind(false), size, single.is_some()).clicked()
                        && let Some(id) = single
                    {
                        action = Some(Action::Zoom(id));
                    }
                    if widgets::action_button(ui, p, "", aurora_i18n::tr!("area-search-inspect"), kind(false), size).clicked() {
                        action = Some(Action::Inspect(self.selection.ids.clone()));
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    widgets::status_dot(ui, p.violet);
                    let text = match &notice {
                        Some((short, _)) => RichText::new(short).size(11.5).color(p.warn),
                        None if any && !wide => {
                            let n = aurora_i18n::tr!("inspect-selected-count", count = self.selection.ids.len());
                            RichText::new(n).size(11.5).color(p.muted)
                        }
                        None => RichText::new(&count).size(11.5).color(p.muted),
                    };
                    let tip = match &notice {
                        Some((_, long)) => format!("{long}\n{count}"),
                        None => format!("{count}\n{}", aurora_i18n::tr!("area-search-scope")),
                    };
                    if ui.available_width() > 40.0 {
                        ui.add(egui::Label::new(text).truncate()).on_hover_text(tip);
                    }
                });
            });
        });
        action
    }

    /// AURORA_DEMO_AREA_SEARCH's options, separated by commas.
    pub(crate) fn demo_controls(&mut self, mode: &str) {
        for option in mode.split(',') {
            match option {
                "wide" => self.forced_wide = Some(true),
                "small" => self.forced_wide = Some(false),
                "filters" | "scope" | "ranges" | "fields" => {
                    self.sheet = true;
                    self.grown = Some(wide::Tile::Filters);
                }
                "regex" => {
                    self.table.filters.regex = true;
                    self.table.filters.name = "(".into();
                }
                "owners" => {
                    self.tab = 1;
                    self.grown = Some(wide::Tile::Owners);
                }
                "table" | "columns" => {
                    self.tab = 2;
                    self.grown = Some(wide::Tile::Objects);
                }
                "distance" => self.grown = Some(wide::Tile::Distance),
                "sale-list" => self.grown = Some(wide::Tile::Sale),
                "3d" => self.grown = Some(wide::Tile::Stage),
                "card" => self.card_open = true,
                "all-columns" => {
                    self.tab = 2;
                    self.grown = Some(wide::Tile::Objects);
                    self.visible_columns = Some([true; 10]);
                }
                "link" => {
                    let f = Filters {
                        name: aurora_i18n::tr!("demo-fountain").into(),
                        ..Default::default()
                    };
                    if let Some(crate::slapp::AppLink::AreaSearch(filters)) = crate::slapp::parse(&f.link()) {
                        self.open_filters(*filters);
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn from_saved(saved: &AreaSearch) -> Self {
        let mut saved = saved.clone();
        saved.normalize();
        let mut ui = Self::default();
        ui.table.filters = saved.filters;
        ui.visible_columns = Some(saved.columns);
        ui
    }

    pub(crate) fn saved_columns(&self) -> [bool; 10] {
        self.visible_columns.unwrap_or(DEFAULT_COLUMNS)
    }

    pub(crate) fn open_filters(&mut self, filters: Filters) {
        self.table.filters = filters;
        self.table.filters.normalize();
        self.table.owner = None;
        self.refresh = true;
    }

    /// "Filter my objects" fills a field: the sheet shows it.
    pub(crate) fn reveal_search(&mut self) {
        self.sheet = true;
        self.grown = Some(wide::Tile::Filters);
    }

    fn reset_section(&mut self, section: Section) {
        let defaults = Filters::default();
        let f = &mut self.table.filters;
        match section {
            Section::All => {
                *f = defaults;
                self.table.owner = None;
            }
            Section::Search => *f = f.without_search(),
            // everything but the search texts and the ranges
            Section::Filters => {
                let kept = f.clone();
                *f = Filters {
                    query: kept.query,
                    name: kept.name,
                    description: kept.description,
                    owner: kept.owner,
                    creator: kept.creator,
                    group: kept.group,
                    last_owner: kept.last_owner,
                    regex: kept.regex,
                    sale: kept.sale,
                    min_price: kept.min_price,
                    max_price: kept.max_price,
                    distance_on: kept.distance_on,
                    min_distance: kept.min_distance,
                    max_distance: kept.max_distance,
                    ..defaults
                };
                self.table.owner = None;
            }
            Section::Ranges => {
                f.sale = defaults.sale;
                f.min_price = defaults.min_price;
                f.max_price = defaults.max_price;
                f.distance_on = defaults.distance_on;
                f.min_distance = defaults.min_distance;
                f.max_distance = defaults.max_distance;
            }
            Section::Columns => {
                self.visible_columns = Some(DEFAULT_COLUMNS);
                return;
            }
        }
        self.table.filters.normalize();
        self.table.prepare();
        self.check_selection();
    }

    /// Selected rows that were filtered out or left the list cannot be
    /// acted on; checked once per new result list.
    fn check_selection(&mut self) {
        if self.checked == self.table.generation {
            return;
        }
        self.checked = self.table.generation;
        if self.selection.is_empty() {
            return;
        }
        let visible: HashSet<_> = self.table.visible.iter().map(|i| self.table.rows[*i].id).collect();
        self.selection.retain(&visible);
    }

    /// menu_fs_area_search.xml with its enable rules (onContextMenuItemEnable):
    /// "one" for Sit, Teleport to and Buy, "in_dd" for Zoom. Buy also needs
    /// the object to be for sale; the Legacy Viewer only says it is not
    /// after the click. The third field of an entry: RLVa hides it
    /// (`MenuLocks`, the `on_visible` parameter of each).
    fn menu(&mut self, ui: &mut egui::Ui, p: &Palette, draw_distance: f32) -> Option<Action> {
        let l = self.menu_locks;
        let ids = &self.selection.ids;
        let any = !ids.is_empty();
        let one = self.selection.single();
        let row = one.and_then(|id| self.table.rows.iter().find(|r| r.id == id));
        let in_dd = row.is_some_and(|r| r.distance < draw_distance);
        let for_sale = row.is_some_and(|r| r.price.is_some());
        // built on click: the selection can hold thousands of rows
        enum Make {
            Each(fn(Vec<Uuid>) -> Action),
            One(fn(Uuid) -> Action),
        }
        let entries = [
            (
                "code",
                aurora_i18n::tr!("ui-context-script-information"),
                any,
                l.edit,
                Make::Each(Action::ScriptInfo),
            ),
            (
                "click-filled",
                aurora_i18n::tr!("ui-context-touch"),
                any,
                l.touch,
                Make::Each(Action::Touch),
            ),
            (
                "armchair",
                aurora_i18n::tr!("area-search-sit"),
                one.is_some(),
                l.sit,
                Make::One(Action::Sit),
            ),
            (
                "location-filled",
                aurora_i18n::tr!("common-teleport-to"),
                one.is_some(),
                l.teleport,
                Make::One(Action::Teleport),
            ),
            (
                "zoom-in-filled",
                aurora_i18n::tr!("common-zoom-2"),
                in_dd,
                false,
                Make::One(Action::Zoom),
            ),
            (
                "zoom-scan-filled",
                aurora_i18n::tr!("ui-context-inspect-2"),
                any,
                l.edit,
                Make::Each(Action::Inspect),
            ),
            (
                "pencil-filled",
                aurora_i18n::tr!("common-edit"),
                any,
                l.edit,
                Make::Each(Action::Edit),
            ),
            (
                "arrow-back-up",
                aurora_i18n::tr!("ui-context-return"),
                any,
                l.delete || l.touch,
                Make::Each(Action::Return),
            ),
            (
                "trash-filled",
                aurora_i18n::tr!("common-delete"),
                any,
                l.delete || l.touch,
                Make::Each(Action::Delete),
            ),
            (
                "eye-closed",
                aurora_i18n::tr!("ui-context-derender-blacklist"),
                any,
                l.delete,
                Make::Each(Action::Blacklist),
            ),
            (
                "shopping-cart-filled",
                aurora_i18n::tr!("common-buy-object"),
                for_sale,
                l.edit || l.buy || l.touch,
                Make::One(Action::Buy),
            ),
        ];
        let mut action = None;
        for (icon, label, enabled, hidden, make) in entries {
            if !hidden && menu::item_if(ui, p, icon, label, enabled) {
                action = Some(match make {
                    Make::Each(make) => make(ids.clone()),
                    Make::One(make) => make(one.unwrap_or_default()),
                });
            }
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "filter-filled", aurora_i18n::tr!("area-search-filter-mine")) {
            action = Some(Action::FilterMine);
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "select-all", aurora_i18n::tr!("common-select-all")) {
            let all = self.table.visible.iter().map(|i| self.table.rows[*i].id).collect();
            self.selection.select_all(all);
        }
        if menu::item_if(ui, p, "deselect", aurora_i18n::tr!("area-search-clear-selection"), any) {
            self.selection.clear();
        }
        action
    }
}

fn menu_id() -> egui::Id {
    egui::Id::unique("area_search_menu")
}
