//! "Inspect Objects": what was right-clicked (everything an avatar wears, or
//! objects of the world) as a list of objects, one per linkset, with their
//! creators; their figures on demand; and the Legacy prim table with its
//! totals. Port of LLFloaterInspect with Firestorm's additions
//! (indra/newview/llfloaterinspect.cpp, floater_inspect.xml,
//! menu_fs_inspect_options.xml, originally LGPL 2.1): description column,
//! "Improved Object Inspect" figures, complexity, sorting, columns chosen in
//! the options menu (FSInspectColumnConfig).
//!
//! The Outfit and Creators lists, the summary card and the View panel are
//! Aurora's own presentation of the same data (human request, 2026-11-23):
//! the Legacy Viewer only has the table, kept as the Details tab. Wide, the
//! window is a board of tiles (`wide.rs`).

mod details;
mod list;
pub(crate) mod model;
mod people;
pub(crate) mod wide;

use super::board;
use super::widgets::{self, Floater};
use crate::object_stage::Stance;
use crate::theme::Palette;
use egui::{RichText, Sense, TextureHandle, Vec2};
use model::Sort;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The columns of the Details table, in floater_inspect.xml's order; `bit`
/// is the column's bit of FSInspectColumnConfig (LLFloaterInspect::mColumnBits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Description,
    Owner,
    Creator,
    Faces,
    Vertices,
    Triangles,
    TextureMemory,
    Vram,
    Created,
}

/// FSInspectColumnConfig's default: every column.
pub const ALL_COLUMNS: u32 = 1023;
/// What an avatar wears: the owner (its wearer) and the dates are there to be
/// chosen, not shown at first.
pub const AVATAR_COLUMNS: u32 = ALL_COLUMNS & !(4 | 512);

impl Column {
    pub const ALL: [Column; 10] = [
        Column::Name,
        Column::Description,
        Column::Owner,
        Column::Creator,
        Column::Faces,
        Column::Vertices,
        Column::Triangles,
        Column::TextureMemory,
        Column::Vram,
        Column::Created,
    ];

    fn bit(self) -> u32 {
        match self {
            Column::Name => 1,
            Column::Description => 2,
            Column::Owner => 4,
            Column::Creator => 8,
            Column::Faces => 16,
            Column::Vertices => 32,
            Column::Triangles => 64,
            Column::TextureMemory => 128,
            Column::Vram => 256,
            Column::Created => 512,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Column::Name => aurora_i18n::tr!("inspect-col-name"),
            Column::Description => aurora_i18n::tr!("inspect-col-description"),
            Column::Owner => aurora_i18n::tr!("inspect-col-owner"),
            Column::Creator => aurora_i18n::tr!("inspect-col-creator"),
            Column::Faces => aurora_i18n::tr!("inspect-col-faces"),
            Column::Vertices => aurora_i18n::tr!("inspect-col-vertices"),
            Column::Triangles => aurora_i18n::tr!("inspect-col-triangles"),
            Column::TextureMemory => aurora_i18n::tr!("inspect-col-tmem"),
            Column::Vram => aurora_i18n::tr!("inspect-col-vram"),
            Column::Created => aurora_i18n::tr!("inspect-col-created"),
        }
    }

    /// Width of a column of figures; None for the columns of text, which
    /// share what is left (dynamic_width of floater_inspect.xml).
    fn fixed_width(self) -> Option<f32> {
        match self {
            Column::Faces => Some(52.0),
            Column::Vertices | Column::Triangles => Some(70.0),
            Column::TextureMemory | Column::Vram => Some(78.0),
            Column::Created => Some(124.0),
            _ => None,
        }
    }

    fn numeric(self) -> bool {
        matches!(
            self,
            Column::Faces | Column::Vertices | Column::Triangles | Column::TextureMemory | Column::Vram
        )
    }
}

fn shown(columns: u32, column: Column) -> bool {
    columns & column.bit() != 0
}

/// One prim of the inspected objects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub id: Uuid,
    /// Top of its linkset: the others are indented under it.
    pub root: bool,
    pub name: String,
    pub description: String,
    /// Shown owner and creator ("Loading…" until the name is known).
    pub owner_name: String,
    pub creator_name: String,
    /// Whose profile "See Owner Profile" opens: none for an
    /// object owned by a group.
    pub owner: Option<Uuid>,
    /// The group that owns it.
    pub owner_group: Option<Uuid>,
    pub creator: Option<Uuid>,
    /// RLVa's `@shownames` hides the creator in the world while this row
    /// names them (they neither own the object nor stand around): their
    /// profile opens from here, as LLFloaterInspect::onClickCreatorProfile
    /// (indra/newview/llfloaterinspect.cpp) opens it.
    pub creator_named_here_alone: bool,
    /// Seconds since the epoch.
    pub created: u64,
    pub faces: u32,
    pub vertices: u32,
    pub triangles: u32,
    /// Texture memory and its size on the GPU, in bytes.
    pub texture_memory: u64,
    pub vram: u64,
    /// Attachment point of its linkset (0: an object in the world).
    pub point: u8,
    /// Complexity of its linkset, on the root row only.
    pub complexity: u32,
    /// Land impact of its linkset, on the root row of an object of the world,
    /// once the region has answered.
    pub land_impact: Option<f32>,
}

/// "Total stats" of floater_inspect.xml.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Totals {
    pub objects: u32,
    pub prims: u32,
    pub faces: u32,
    pub visible_faces: u32,
    pub vertices: u32,
    pub triangles: u32,
    pub complexity: u32,
    pub textures: u32,
    pub texture_memory: u64,
    pub vram: u64,
}

/// What the window inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Subject {
    /// Everything an avatar wears (their menu's Inspect).
    Avatar(Uuid),
    /// Objects of the world, or one attachment.
    #[default]
    Objects,
}

/// How the creator shows on the rows of the lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CreatorShown {
    #[default]
    Picture,
    Name,
    Hidden,
}

/// The View panel's choices, kept in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InspectView {
    pub sort: Sort,
    pub creator: CreatorShown,
    pub worn_on: bool,
    pub complexity: bool,
}

impl Default for InspectView {
    fn default() -> Self {
        InspectView {
            sort: Sort::Area,
            creator: CreatorShown::Picture,
            worn_on: false,
            complexity: true,
        }
    }
}

impl InspectView {
    /// How many choices differ from the defaults (the View button's count).
    fn changes(&self) -> usize {
        let d = InspectView::default();
        [
            self.sort != d.sort,
            self.creator != d.creator,
            self.worn_on != d.worn_on,
            self.complexity != d.complexity,
        ]
        .into_iter()
        .filter(|c| *c)
        .count()
    }
}

/// Someone here, for the complexity strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: Uuid,
    /// Whose picture shows; None when RLVa hides them.
    pub pic: Option<Uuid>,
    /// As the world shows it.
    pub name: String,
    pub complexity: u32,
    pub you: bool,
    pub friend: bool,
    /// Not above the limit (or an exception to it).
    pub drawn: bool,
}

impl Person {
    /// Never gathered with others on the scale.
    fn kept(&self) -> bool {
        self.you || self.friend
    }

    fn shown_name(&self) -> &str {
        if self.you { aurora_i18n::tr!("inspect-you") } else { &self.name }
    }
}

/// What the window needs from the app this frame.
pub struct Frame<'a> {
    pub rows: &'a [Row],
    pub totals: &'a Totals,
    /// The wearer's display name as the world shows it (their stand-in
    /// under RLVa's `@shownames`), for an avatar.
    pub subject_name: &'a str,
    /// Their username ("orion.example") under the name; None when the world
    /// hides them, or for objects.
    pub subject_username: Option<&'a str>,
    /// The wearer whose profile the card's button opens; None when the world
    /// hides them.
    pub subject_profile: Option<Uuid>,
    pub pics: &'a HashMap<Uuid, TextureHandle>,
    /// Attachment points by id (avatar_lad.xml).
    pub point_names: &'a HashMap<u8, String>,
    /// RenderAvatarMaxComplexity; 0: no limit.
    pub max_complexity: u32,
    /// The people here with their complexity (the wide window, for an avatar).
    pub people: &'a [Person],
    /// Land impact used and allowed on the parcel the objects are on.
    pub parcel: Option<(i32, i32)>,
    pub columns: &'a mut u32,
    pub view: &'a mut InspectView,
}

/// What the window asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A profile, opened as the world names people, or as a place that names
    /// its people for the creator this window names while the world hides
    /// them (`named_here_alone`).
    Profile { who: Uuid, named_here_alone: bool },
    /// Move the camera to an object (the menu's Zoom).
    LookAt(Uuid),
    /// The profile of a group that owns an object.
    Group(Uuid),
}

/// The compact window and the wide one.
const VIEWS: board::Views = board::Views {
    window: "inspect",
    small: Vec2::new(420.0, 600.0),
    wide: Vec2::new(800.0, 760.0),
    from: 700.0,
};

#[derive(Default)]
pub struct InspectUi {
    pub open: bool,
    /// Root prims of the inspected objects.
    pub roots: Vec<Uuid>,
    /// The highlighted prim, outlined in the world (LLFloaterInspect::getSelectedUUID).
    pub selected: Option<Uuid>,
    pub subject: Subject,
    /// Outfit (Objects), Creators, Details.
    tab: usize,
    query: String,
    view_open: bool,
    /// The object opened in the lists, by its root.
    opened: Option<Uuid>,
    /// Column sorted on in Details and whether descending; None keeps the
    /// linksets together, their root first.
    sort: Option<(Column, bool)>,
    /// The prim opened in Details.
    detail_open: Option<Uuid>,
    /// Profile pictures the window shows, for the app to fetch.
    pub wanted_pics: HashSet<Uuid>,
    /// AURORA_DEMO_INSPECT has opened the window (or is not set).
    pub demo_done: bool,
    /// The 3D view of the wide window.
    pub stage: super::object_stage::StageUi,
    wide: wide::Wide,
    /// AURORA_DEMO_INSPECT's `hover`.
    demo_hover: bool,
    demo_stance: Option<Stance>,
    demo_light: bool,
    /// AURORA_DEMO_INSPECT's view; None: the one chosen last.
    pub forced_wide: Option<bool>,
    /// The window opens this frame: it takes the view chosen last.
    opening: bool,
    /// One object was inspected: the window opens wide, on its 3D view.
    single: bool,
}

/// Date and time in SLT (the timeStamp of floater_inspect.xml, kept short).
pub(crate) fn slt_datetime(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let (y, m, d, _, h, mi, _) = crate::ui::land::slt_parts(secs as i64);
    aurora_i18n::tr!(
        "inspect-date",
        year = format!("{y:04}"),
        month = format!("{m:02}"),
        day = format!("{d:02}"),
        hour = format!("{h:02}"),
        minute = format!("{mi:02}")
    )
}

fn kilobytes(bytes: u64) -> String {
    aurora_i18n::integer((bytes / 1024) as i64)
}

pub(crate) fn megabytes(bytes: u64) -> String {
    aurora_i18n::tr!("inspect-megabytes", n = aurora_i18n::number(bytes as f64 / (1024.0 * 1024.0), 1))
}

impl InspectUi {
    /// LLFloaterInspect::onOpen on the objects of `roots`. One object opens
    /// on its card; what an avatar wears opens on the list.
    pub fn inspect(&mut self, roots: Vec<Uuid>, subject: Subject) {
        self.open = true;
        self.selected = roots.first().copied();
        self.opened = (roots.len() == 1).then(|| roots[0]);
        self.detail_open = None;
        self.query.clear();
        self.tab = 0;
        self.opening = true;
        self.single = roots.len() == 1;
        self.wide.restart();
        self.stage.restart();
        self.subject = subject;
        self.roots = roots;
    }

    /// AURORA_DEMO_INSPECT's options: `wide` or `small` the view; small, `open`
    /// the third object's card, `creators` or `details` that tab, `view` the
    /// View panel; wide, `item` the third object, `grow-…` a tile grown,
    /// `hover` its 3D view with the fourth pointed at,
    /// `tpose`, `apose` and `light` how it stands and its ground.
    pub fn demo(&mut self, option: &str) {
        match option {
            "open" => {
                // the third: the wolf's head, with its linked prims
                self.opened = self.roots.get(2).or(self.roots.first()).copied();
                self.selected = self.opened;
            }
            "creators" => self.tab = 1,
            "details" => self.tab = 2,
            "view" => self.view_open = true,
            "wide" => self.forced_wide = Some(true),
            "small" => self.forced_wide = Some(false),
            "item" => {
                let root = self.roots.get(2).or(self.roots.first()).copied();
                self.open_item(root);
            }
            "grow-items" => self.wide.grown = Some(wide::Tile::Items),
            "grow-complexity" => self.wide.grown = Some(wide::Tile::Weight),
            "grow-makers" => self.wide.grown = Some(wide::Tile::Makers),
            "grow-places" => self.wide.grown = Some(wide::Tile::Places),
            "grow-figures" => self.wide.grown = Some(wide::Tile::Figures),
            "grow-prims" => self.wide.grown = Some(wide::Tile::Prims),
            "grow-3d" => self.wide.grown = Some(wide::Tile::Stage),
            "hover" => self.demo_hover = true,
            "tpose" => self.demo_stance = Some(Stance::Wide),
            "apose" => self.demo_stance = Some(Stance::Relaxed),
            "light" => self.demo_light = true,
            _ => {}
        }
    }

    /// Open an item (its root prim outlined in the world), or none.
    fn open_item(&mut self, root: Option<Uuid>) {
        self.opened = root;
        if root.is_some() {
            self.selected = root;
        }
    }

    /// The demo chose the window's view: its choices are not kept.
    pub fn demo_forced(&self) -> bool {
        self.forced_wide.is_some()
    }

    /// AURORA_DEMO_INSPECT's stance and ground.
    pub fn demo_stage(&mut self) {
        if let Some(stance) = self.demo_stance {
            self.stage.stance = stance;
        }
        self.stage.light |= self.demo_light;
    }

    fn avatar(&self) -> bool {
        matches!(self.subject, Subject::Avatar(_))
    }

    /// Whether the window shows its board of tiles this frame.
    pub fn is_wide(&self, ctx: &egui::Context, saved: bool) -> bool {
        self.open && board::is_wide(ctx, &VIEWS, self.starts_wide(saved), self.opening)
    }

    /// The view the window opens on: the one chosen last, the wide one for one
    /// object (a demo's own view always).
    fn starts_wide(&self, saved: bool) -> bool {
        saved || (self.single && self.forced_wide.is_none())
    }

    /// Draw the window. Returns what the window asks of the app.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, frame: Frame<'_>, saved_wide: &mut bool) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        let wide = self.is_wide(ctx, *saved_wide);
        self.opening = false;
        let rows = frame.rows;
        // the highlighted prim is gone: the first one (refresh's selectNthItem(0))
        if !rows.iter().any(|r| Some(r.id) == self.selected) {
            self.selected = rows.first().map(|r| r.id);
        }
        let items = model::items(rows);
        if self.opened.is_some_and(|id| !items.iter().any(|i| i.root == id)) {
            self.opened = None;
        }
        let mut open = true;
        let mut floater = Floater::new(
            "inspect",
            aurora_i18n::tr!("inspect-window"),
            egui::pos2(240.0, 60.0),
            VIEWS.size(ctx, self.starts_wide(*saved_wide)),
        );
        floater.min_size = Vec2::new(360.0, 380.0);
        let mut resize = false;
        let (icon, tip) = board::size_button(wide);
        let floater = floater.action(icon, tip, &mut resize);
        let mut frame = frame;
        floater.show(ctx, p, &mut open, |ui| {
            if wide {
                wide::show(self, ui, p, &mut frame, &items, &mut actions);
                return;
            }
            self.summary(ui, p, &frame, &items, &mut actions);
            ui.add_space(8.0);
            self.search_row(ui, p, frame.view);
            if self.view_open {
                ui.add_space(6.0);
                self.view_panel(ui, p, frame.view);
            }
            ui.add_space(8.0);
            let creators = items.iter().filter_map(|i| rows[i.first].creator).collect::<HashSet<_>>().len();
            let first = if self.avatar() {
                aurora_i18n::tr!("inspect-tab-outfit")
            } else {
                aurora_i18n::tr!("inspect-tab-objects")
            };
            let tabs = [
                (first, items.len()),
                (aurora_i18n::tr!("inspect-tab-creators"), creators),
                (aurora_i18n::tr!("inspect-tab-details"), rows.len()),
            ];
            widgets::count_tabs(ui, p, "inspect_tabs", &mut self.tab, &tabs);
            ui.add_space(6.0);
            let (mut body, foot) = widgets::above_footer(ui);
            match self.tab {
                0 | 1 => list::show(self, &mut body, p, &frame, &items, &mut actions),
                _ => details::show(self, &mut body, p, &mut frame, &items, &mut actions),
            }
            foot.show(ui, |ui| self.footer(ui, p, rows));
        });
        board::after(ctx, &VIEWS, resize, wide, saved_wide);
        if !open {
            self.open = false;
        }
        actions
    }

    /// The summary card: who or what is inspected, how much, and the
    /// complexity against the user's limit.
    fn summary(&mut self, ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, items: &[model::Item], actions: &mut Vec<Action>) {
        let radius = ui.visuals().window_corner_radius;
        egui::Frame::new()
            .fill(p.bar)
            .corner_radius(radius)
            .inner_margin(10)
            .show(ui, |ui| {
                let m = SummaryMetrics::new(ui);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), m.height), Sense::hover());
                let limit = self.avatar().then_some(frame.max_complexity);
                // the box takes the card's whole height, so its edges, the
                // picture's and the two text blocks' share one middle line
                let boxed = complexity_box(ui, p, rect, &m, frame.totals.complexity, limit);
                let pic = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + SUMMARY_PICTURE / 2.0, rect.center().y),
                    Vec2::splat(SUMMARY_PICTURE),
                );
                let title = match self.subject {
                    Subject::Avatar(id) => {
                        self.wanted_pics.insert(id);
                        let tex = frame.pics.get(&id);
                        widgets::avatar_disc(ui, p, pic, tex, "", false);
                        if tex.is_none() {
                            initials_at(ui, p, pic, frame.subject_name);
                        }
                        frame.subject_name.to_owned()
                    }
                    Subject::Objects => {
                        ui.painter().rect_filled(pic, radius, p.raised);
                        crate::ui::sheet::icon_at(ui, "cube", pic.center(), 20.0, p.violet_light);
                        aurora_i18n::tr!("inspect-objects-title", count = items.len())
                    }
                };
                // one line each, cut short with "…" (the counts are on the
                // tabs below)
                let left = pic.right() + 12.0;
                let right = boxed.left() - 10.0;
                let base = if frame.subject_username.is_some() {
                    m.top(rect) + m.name.cap
                } else {
                    rect.center().y + m.name.cap / 2.0
                };
                let link_w = if frame.subject_profile.is_some() { 24.0 } else { 0.0 };
                let room = (right - left - link_w).max(20.0);
                let name = RichText::new(&title).font(m.name.font.clone()).color(p.ink);
                let drawn = text_on(ui, &m.name, egui::pos2(left, base), room, name);
                if let Some(id) = frame.subject_profile {
                    let middle = egui::pos2(drawn.right() + 14.0, base - m.name.cap / 2.0);
                    if profile_link(ui, p, middle) {
                        actions.push(Action::Profile {
                            who: id,
                            named_here_alone: false,
                        });
                    }
                }
                if let Some(username) = frame.subject_username {
                    let text = RichText::new(username).font(m.username.font.clone()).color(p.muted);
                    let at = egui::pos2(left, m.bottom(rect));
                    text_on(ui, &m.username, at, (right - left).max(20.0), text);
                }
            });
    }

    fn search_row(&mut self, ui: &mut egui::Ui, p: &Palette, view: &InspectView) {
        ui.horizontal(|ui| {
            let changes = view.changes();
            let label = if changes > 0 {
                aurora_i18n::tr!("inspect-view-count", count = changes)
            } else {
                aurora_i18n::tr!("inspect-view").to_owned()
            };
            let button_w = widgets::action_button_fit(ui, "adjustments-horizontal-filled", &label);
            let hint = if self.tab == 2 {
                aurora_i18n::tr!("inspect-search-prims")
            } else {
                aurora_i18n::tr!("inspect-search")
            };
            let width = (ui.available_width() - button_w - ui.spacing().item_spacing.x).max(80.0);
            // as tall as the button beside it, its capitals in its middle
            let mut margin = widgets::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
            margin.left += 8;
            margin.right += 4;
            widgets::search_text_edit_with_margin(ui, p, &mut self.query, margin, |edit| {
                edit.hint_text(hint)
                    .align(egui::Align2::LEFT_CENTER)
                    .desired_width(width)
                    .min_size(Vec2::new(0.0, SEARCH_HEIGHT))
            });
            let kind = if self.view_open {
                widgets::ActionKind::Primary
            } else {
                widgets::ActionKind::Flat
            };
            if widgets::action_button(ui, p, "adjustments-horizontal-filled", &label, kind, Vec2::new(0.0, SEARCH_HEIGHT))
                .on_hover_text(aurora_i18n::tr!("inspect-view-tip"))
                .clicked()
            {
                self.view_open = !self.view_open;
            }
        });
    }

    /// The View panel: order, how the creator shows, what else the rows say.
    fn view_panel(&mut self, ui: &mut egui::Ui, p: &Palette, view: &mut InspectView) {
        let radius = ui.visuals().window_corner_radius;
        egui::Frame::new()
            .fill(p.bar)
            .corner_radius(radius)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let heading = |ui: &mut egui::Ui, text: &str| {
                    ui.label(
                        RichText::new(text)
                            .size(11.5)
                            .family(crate::ui::fonts::semibold(ui.ctx()))
                            .color(p.muted),
                    );
                };
                heading(ui, aurora_i18n::tr!("inspect-view-sort"));
                let first = if self.avatar() {
                    aurora_i18n::tr!("inspect-sort-area")
                } else {
                    aurora_i18n::tr!("inspect-sort-listed")
                };
                let sorts = [Sort::Area, Sort::Name, Sort::Heaviest, Sort::Newest];
                let mut at = sorts.iter().position(|s| *s == view.sort).unwrap_or(0);
                let labels = [
                    (first, true),
                    (aurora_i18n::tr!("inspect-sort-name"), true),
                    (aurora_i18n::tr!("inspect-sort-heaviest"), true),
                    (aurora_i18n::tr!("inspect-sort-newest"), true),
                ];
                if widgets::segmented(ui, p, &mut at, &labels) {
                    view.sort = sorts[at];
                }
                ui.add_space(8.0);
                heading(ui, aurora_i18n::tr!("inspect-view-creator"));
                let shows = [CreatorShown::Picture, CreatorShown::Name, CreatorShown::Hidden];
                let mut at = shows.iter().position(|s| *s == view.creator).unwrap_or(0);
                let labels = [
                    (aurora_i18n::tr!("inspect-creator-picture"), true),
                    (aurora_i18n::tr!("inspect-creator-name"), true),
                    (aurora_i18n::tr!("inspect-creator-hidden"), true),
                ];
                if widgets::segmented(ui, p, &mut at, &labels) {
                    view.creator = shows[at];
                }
                ui.add_space(8.0);
                heading(ui, aurora_i18n::tr!("inspect-view-show"));
                ui.horizontal(|ui| {
                    if self.avatar() {
                        widgets::checkbox(ui, p, &mut view.worn_on, aurora_i18n::tr!("inspect-show-worn-on"));
                    }
                    widgets::checkbox(ui, p, &mut view.complexity, aurora_i18n::tr!("inspect-show-complexity"));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "refresh",
                        aurora_i18n::tr!("common-reset"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 26.0),
                    )
                    .clicked()
                    {
                        *view = InspectView::default();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::action_button(
                            ui,
                            p,
                            "check",
                            aurora_i18n::tr!("inspect-view-done"),
                            widgets::ActionKind::Primary,
                            Vec2::new(0.0, 26.0),
                        )
                        .clicked()
                        {
                            self.view_open = false;
                        }
                    });
                });
            });
    }

    fn footer(&self, ui: &mut egui::Ui, p: &Palette, rows: &[Row]) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let selected = rows.iter().find(|r| Some(r.id) == self.selected);
            let open = if self.tab == 2 {
                self.detail_open.is_some()
            } else {
                self.opened.is_some()
            };
            let text = match selected {
                Some(row) if open => aurora_i18n::tr!("inspect-outlined", name = row.name.trim()),
                _ if self.tab == 2 => aurora_i18n::tr!("inspect-hint-details").to_owned(),
                _ => aurora_i18n::tr!("inspect-hint-right").to_owned(),
            };
            widgets::status_dot(ui, p.violet);
            ui.label(RichText::new(text).size(11.5).color(p.muted));
        });
    }
}

/// Height of the search field and of the View button beside it.
const SEARCH_HEIGHT: f32 = 26.0;

/// Side of the picture in the summary card.
const SUMMARY_PICTURE: f32 = 38.0;

/// A font and where its capitals stand in a line, as drawn: a line's box
/// leaves more room under the letters than above, and the face is drawn a
/// fraction of a pixel off its baseline, so texts of different sizes centred
/// by their boxes do not look centred.
struct Ink {
    font: egui::FontId,
    /// From the line's top to the foot of the capitals, on a whole pixel.
    base: f32,
    /// Height of a capital.
    cap: f32,
}

impl Ink {
    fn new(ui: &egui::Ui, font: egui::FontId) -> Self {
        let galley = ui
            .painter()
            .layout_no_wrap("H".to_owned(), font.clone(), egui::Color32::PLACEHOLDER);
        let glyph = galley
            .rows
            .first()
            .and_then(|row| row.glyphs.first().map(|g| (row.pos.y + g.pos.y, g.uv_rect)));
        let (base, cap) = match glyph {
            Some((line, uv)) if uv.size.y > 0.0 => (line + uv.offset.y + uv.size.y, uv.size.y),
            _ => (font.size, font.size * 0.7),
        };
        let px = ui.ctx().pixels_per_point();
        let base = (base * px).round() / px;
        Self { font, base, cap }
    }
}

/// The summary card's lines: the name over the username on the left, the
/// label over the figure in the complexity box. Both blocks have the same
/// height, from the top of their capitals to their last baseline, and so the
/// same top, middle and bottom.
struct SummaryMetrics {
    name: Ink,
    username: Ink,
    label: Ink,
    figure: Ink,
    /// Height of a text block.
    block: f32,
    /// Height of the card's inside, and of the complexity box.
    height: f32,
}

impl SummaryMetrics {
    fn new(ui: &egui::Ui) -> Self {
        let bold = crate::ui::fonts::semibold(ui.ctx());
        let name = Ink::new(ui, egui::FontId::new(15.0, bold.clone()));
        let username = Ink::new(ui, egui::FontId::proportional(12.0));
        let label = Ink::new(ui, egui::FontId::new(10.5, bold.clone()));
        let figure = Ink::new(ui, egui::FontId::new(14.0, bold));
        let gap = 6.0 * crate::ui::fonts::scale(ui.ctx());
        let block = (name.cap + username.cap).max(label.cap + figure.cap) + gap;
        Self {
            name,
            username,
            label,
            figure,
            block,
            height: (block + 12.0).max(SUMMARY_PICTURE),
        }
    }

    /// Top of the blocks' capitals.
    fn top(&self, card: egui::Rect) -> f32 {
        card.center().y - self.block / 2.0
    }

    /// Last baseline of the blocks.
    fn bottom(&self, card: egui::Rect) -> f32 {
        card.center().y + self.block / 2.0
    }
}

/// A line of text with its baseline at `at`, at most `width` wide.
fn text_on(ui: &mut egui::Ui, ink: &Ink, at: egui::Pos2, width: f32, text: impl Into<egui::WidgetText>) -> egui::Rect {
    let line = egui::Rect::from_min_size(egui::pos2(at.x, at.y - ink.base), Vec2::new(width, ink.base + ink.cap));
    let layout = egui::Layout::top_down(egui::Align::Min);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(line).layout(layout));
    child.add(egui::Label::new(text).truncate()).rect
}

/// The initials of `name` in the disc of a resident without a picture, their
/// capitals in its middle.
fn initials_at(ui: &egui::Ui, p: &Palette, disc: egui::Rect, name: &str) {
    let size = disc.height() * 0.42 / crate::ui::fonts::scale(ui.ctx());
    let ink = Ink::new(ui, egui::FontId::proportional(size));
    let galley = ui.painter().layout_no_wrap(widgets::initials(name), ink.font, p.violet_pale);
    let at = egui::pos2(disc.center().x - galley.size().x / 2.0, disc.center().y + ink.cap / 2.0 - ink.base);
    ui.painter().galley(at, galley, p.violet_pale);
}

/// The complexity of the inspected objects. For an avatar above the user's
/// RenderAvatarMaxComplexity (0: no limit), beyond which the Legacy Viewer
/// draws them as a jelly doll, the figure turns orange; the tooltip says
/// which side of the limit it is.
fn complexity_box(ui: &mut egui::Ui, p: &Palette, card: egui::Rect, m: &SummaryMetrics, complexity: u32, limit: Option<u32>) -> egui::Rect {
    const SPACING: f32 = 0.7;
    let above = limit.is_some_and(|limit| limit != 0 && complexity > limit);
    let mut format = egui::TextFormat::simple(m.label.font.clone(), p.muted);
    format.extra_letter_spacing = SPACING;
    let label = aurora_i18n::tr!("inspect-totals-complexity").to_uppercase();
    let label = ui.painter().layout_job(egui::text::LayoutJob::single_section(label, format));
    let ink = if above { p.warn } else { p.ink };
    let figure = ui
        .painter()
        .layout_no_wrap(aurora_i18n::integer(i64::from(complexity)), m.figure.font.clone(), ink);
    // the last letter's spacing is not part of the word
    let label_w = label.size().x - SPACING;
    let width = label_w.max(figure.size().x) + 18.0;
    let rect = egui::Rect::from_min_max(egui::pos2(card.right() - width, card.top()), card.right_bottom());
    ui.painter()
        .rect_filled(rect, ui.visuals().window_corner_radius, p.raised.gamma_multiply(0.5));
    let right = rect.right() - 9.0;
    let (label_room, figure_w) = (label.size().x, figure.size().x);
    text_on(
        ui,
        &m.label,
        egui::pos2(right - label_w, m.top(card) + m.label.cap),
        label_room,
        label,
    );
    text_on(ui, &m.figure, egui::pos2(right - figure_w, m.bottom(card)), figure_w, figure);
    if let Some(limit) = limit.filter(|limit| *limit != 0) {
        let side = if above {
            aurora_i18n::tr!("inspect-over-limit")
        } else {
            aurora_i18n::tr!("inspect-within-limit")
        };
        let tip = format!(
            "{side}\n{}",
            aurora_i18n::tr!("inspect-limit-tip", limit = aurora_i18n::integer(i64::from(limit)))
        );
        ui.interact(rect, ui.scope_id().with("inspect_complexity"), Sense::hover())
            .on_hover_text(tip);
    }
    rect
}

/// The quiet link beside the name in the summary card, 20 px around
/// `middle`: a small icon, a face only when pointed at.
fn profile_link(ui: &mut egui::Ui, p: &Palette, middle: egui::Pos2) -> bool {
    let r = egui::Rect::from_center_size(middle, Vec2::splat(20.0));
    let resp = ui.interact(r, ui.scope_id().with("inspect_profile_link"), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let ink = if hovered { p.ink } else { p.muted };
    crate::ui::sheet::icon_at(ui, "external-link-filled", r.center(), 14.0, ink);
    let tip = aurora_i18n::tr!("inspect-open-profile");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, tip));
    resp.on_hover_text(tip).clicked()
}

/// How a profile button opens `who`: handed back to the app, which opens it
/// as the world names people, or asked as a place that names its people asks
/// (`profile::request_open_in`) for the creator this window names while the
/// world hides them, whom the first way would refuse.
pub fn profile_route(who: Option<Uuid>, named_here_alone: bool) -> (Option<Uuid>, Option<Uuid>) {
    if named_here_alone { (None, who) } else { (who, None) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window keeps its size, its footer inside it.
    #[test]
    fn the_window_keeps_its_size() {
        let rows: Vec<Row> = (0..40)
            .map(|i| Row {
                id: Uuid::from_u128(i),
                root: i % 4 == 0,
                name: format!("Prim {i}"),
                point: (i % 12) as u8 + 1,
                complexity: if i % 4 == 0 { 1_000 * i as u32 } else { 0 },
                ..Default::default()
            })
            .collect();
        let pics = HashMap::new();
        let names = HashMap::new();
        crate::ui::widgets::steady_tests::assert_window_steady(
            "inspect",
            &[Vec2::new(420.0, 600.0), Vec2::new(360.0, 380.0)],
            aurora_i18n::tr!("inspect-hint-right"),
            || InspectUi {
                open: true,
                subject: Subject::Avatar(Uuid::from_u128(99)),
                ..Default::default()
            },
            |ui, ctx, p| {
                let mut columns = ALL_COLUMNS;
                let mut view = InspectView::default();
                let mut saved = false;
                ui.show(
                    ctx,
                    p,
                    Frame {
                        rows: &rows,
                        totals: &Totals::default(),
                        subject_name: "Mira Solvane",
                        subject_username: Some("mira.solvane"),
                        subject_profile: Some(Uuid::from_u128(99)),
                        pics: &pics,
                        point_names: &names,
                        max_complexity: 350_000,
                        people: &[],
                        parcel: None,
                        columns: &mut columns,
                        view: &mut view,
                    },
                    &mut saved,
                );
            },
        );
    }

    /// The board of tiles too, every tile grown in turn.
    #[test]
    fn the_wide_window_keeps_its_size() {
        let rows: Vec<Row> = (0..40)
            .map(|i| Row {
                id: Uuid::from_u128(i),
                root: i % 4 == 0,
                name: format!("Prim {i}"),
                creator_name: format!("Maker {}", i % 3),
                point: (i % 12) as u8 + 1,
                complexity: if i % 4 == 0 { 1_000 * i as u32 } else { 0 },
                ..Default::default()
            })
            .collect();
        let people: Vec<Person> = (0..30)
            .map(|i| Person {
                id: Uuid::from_u128(500 + i),
                pic: None,
                name: format!("Person {i}"),
                complexity: 9_000 * i as u32,
                you: i == 3,
                friend: i == 7,
                drawn: i < 25,
            })
            .collect();
        let (pics, names) = (HashMap::new(), HashMap::new());
        let grown = [
            None,
            Some(wide::Tile::Stage),
            Some(wide::Tile::Items),
            Some(wide::Tile::Weight),
            Some(wide::Tile::Makers),
            Some(wide::Tile::Places),
            Some(wide::Tile::Figures),
            Some(wide::Tile::Prims),
        ];
        for (tile, opened) in grown.into_iter().flat_map(|t| [(t, None), (t, Some(Uuid::from_u128(4)))]) {
            for subject in [Subject::Avatar(Uuid::from_u128(99)), Subject::Objects] {
                crate::ui::widgets::steady_tests::assert_window_steady(
                    "inspect",
                    &[Vec2::new(800.0, 760.0)],
                    aurora_i18n::tr!("inspect-creator-button"),
                    || {
                        let mut ui = InspectUi {
                            open: true,
                            subject,
                            opened,
                            selected: opened,
                            ..Default::default()
                        };
                        ui.wide.grown = tile;
                        ui
                    },
                    |ui, ctx, p| {
                        let (mut columns, mut view, mut saved) = (ALL_COLUMNS, InspectView::default(), true);
                        ui.show(
                            ctx,
                            p,
                            Frame {
                                rows: &rows,
                                totals: &Totals::default(),
                                subject_name: "Mira Solvane",
                                subject_username: Some("mira.solvane"),
                                subject_profile: Some(Uuid::from_u128(99)),
                                pics: &pics,
                                point_names: &names,
                                max_complexity: 150_000,
                                people: &people,
                                parcel: Some((900, 1_000)),
                                columns: &mut columns,
                                view: &mut view,
                            },
                            &mut saved,
                        );
                    },
                );
            }
        }
    }

    #[test]
    fn the_creator_the_window_alone_names_opens_as_a_named_resident() {
        let id = Uuid::from_u128(7);
        // the owner, or a creator the world names: handed back to the app
        assert_eq!(profile_route(Some(id), false), (Some(id), None));
        // a creator `@shownames` hides in the world and this window names
        assert_eq!(profile_route(Some(id), true), (None, Some(id)));
        // a disabled button opens nothing either way
        assert_eq!(profile_route(None, false), (None, None));
        assert_eq!(profile_route(None, true), (None, None));
    }

    #[test]
    fn the_view_button_counts_the_choices_changed() {
        let mut view = InspectView::default();
        assert_eq!(view.changes(), 0);
        view.sort = Sort::Heaviest;
        view.creator = CreatorShown::Name;
        assert_eq!(view.changes(), 2);
    }

    #[test]
    fn owner_and_dates_wait_to_be_chosen_for_an_avatar() {
        let hidden: Vec<Column> = Column::ALL.into_iter().filter(|c| !shown(AVATAR_COLUMNS, *c)).collect();
        assert_eq!(hidden, [Column::Owner, Column::Created]);
        assert!(Column::ALL.into_iter().all(|c| shown(ALL_COLUMNS, c)));
    }
}
