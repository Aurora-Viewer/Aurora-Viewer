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
//! the Legacy Viewer only has the table, kept as the Details tab.

mod details;
mod list;
pub(crate) mod model;

use super::widgets::{self, Floater};
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

    /// Columns that say nothing about what an avatar wears: every worn
    /// object is the wearer's, and the dates were not wanted there.
    fn for_objects_only(self) -> bool {
        matches!(self, Column::Owner | Column::Created)
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

/// What the window needs from the app this frame.
pub struct Frame<'a> {
    pub rows: &'a [Row],
    pub totals: &'a Totals,
    /// The wearer as the world names them (their stand-in under RLVa's
    /// `@shownames`), for an avatar.
    pub subject_name: &'a str,
    /// The wearer whose profile the card's button opens; None when the world
    /// hides them.
    pub subject_profile: Option<Uuid>,
    pub pics: &'a HashMap<Uuid, TextureHandle>,
    /// Attachment points by id (avatar_lad.xml).
    pub point_names: &'a HashMap<u8, String>,
    /// RenderAvatarMaxComplexity; 0: no limit.
    pub max_complexity: u32,
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
}

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

fn megabytes(bytes: u64) -> String {
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
        self.subject = subject;
        self.roots = roots;
    }

    /// AURORA_DEMO_INSPECT's options: `open` the third object's card,
    /// `creators` or `details` that tab, `view` the View panel.
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
            _ => {}
        }
    }

    fn avatar(&self) -> bool {
        matches!(self.subject, Subject::Avatar(_))
    }

    /// Draw the window. Returns what the window asks of the app.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, frame: Frame<'_>) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
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
            aurora_i18n::tr!("inspect-title"),
            egui::pos2(240.0, 120.0),
            Vec2::new(420.0, 600.0),
        );
        floater.min_size = Vec2::new(360.0, 380.0);
        let mut frame = frame;
        floater.show(ctx, p, &mut open, |ui| {
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
        if !open {
            self.open = false;
        }
        actions
    }

    /// The summary card: who or what is inspected, how much, and the
    /// complexity against the user's limit.
    fn summary(&mut self, ui: &mut egui::Ui, p: &Palette, frame: &Frame<'_>, items: &[model::Item], actions: &mut Vec<Action>) {
        let radius = ui.visuals().window_corner_radius;
        let creators = items
            .iter()
            .filter_map(|i| frame.rows[i.first].creator)
            .collect::<HashSet<_>>()
            .len();
        egui::Frame::new()
            .fill(p.bar)
            .corner_radius(radius)
            .inner_margin(10)
            .show(ui, |ui| {
                let limit = self.avatar().then_some(frame.max_complexity);
                ui.horizontal(|ui| {
                    // egui cannot centre a block before it knows its size: the
                    // row takes its height first (the box, the picture, and the
                    // text block as the last frame measured it), and the box
                    // and the text get the margins that centre them on it
                    let text_id = ui.scope_id().with("inspect_summary_text");
                    let box_h = complexity_box_height(ui, limit.is_some());
                    let text_h = ui.data(|d| d.get_temp::<f32>(text_id)).unwrap_or(box_h);
                    let row_h = box_h.max(38.0).max(text_h);
                    ui.set_min_height(row_h);
                    let (pic, _) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::hover());
                    let title = match self.subject {
                        Subject::Avatar(id) => {
                            self.wanted_pics.insert(id);
                            widgets::avatar_picture(ui, p, pic, frame.pics.get(&id), frame.subject_name, false);
                            frame.subject_name.to_owned()
                        }
                        Subject::Objects => {
                            ui.painter().circle_filled(pic.center(), pic.width() / 2.0, p.raised);
                            crate::ui::sheet::icon_at(ui, "cube", pic.center(), 20.0, p.violet_light);
                            aurora_i18n::tr!("inspect-objects-title", count = items.len())
                        }
                    };
                    ui.add_space(4.0);
                    let mut parts = Vec::new();
                    if self.avatar() {
                        parts.push(aurora_i18n::tr!("inspect-summary-items", count = items.len()));
                        parts.push(aurora_i18n::tr!("inspect-summary-creators", count = creators));
                    }
                    parts.push(aurora_i18n::tr!("inspect-summary-prims", count = frame.rows.len()));
                    if !self.avatar() {
                        parts.push(aurora_i18n::tr!("inspect-summary-creators", count = creators));
                    }
                    // the box first, from the right: the name and the line
                    // take what is left, on one line each, cut short with "…"
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            ui.add_space(((row_h - box_h) / 2.0).max(0.0));
                            complexity_box(ui, p, frame.totals.complexity, limit);
                        });
                        ui.add_space(6.0);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                let pad = ((row_h - text_h) / 2.0).max(0.0);
                                ui.add_space(pad);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    let bold = crate::ui::fonts::semibold(ui.ctx());
                                    let room = (ui.available_width() - 24.0).max(20.0);
                                    ui.scope(|ui| {
                                        ui.set_max_width(room);
                                        let name = RichText::new(&title).size(15.0).family(bold).color(p.ink);
                                        ui.add(egui::Label::new(name).truncate());
                                    });
                                    if let Some(id) = frame.subject_profile
                                        && profile_link(ui, p)
                                    {
                                        actions.push(Action::Profile {
                                            who: id,
                                            named_here_alone: false,
                                        });
                                    }
                                });
                                ui.add(egui::Label::new(figures_line(ui, p, &parts)).truncate());
                                let measured = ui.min_rect().height() - pad;
                                if (measured - text_h).abs() > 0.5 {
                                    ui.data_mut(|d| d.insert_temp(text_id, measured));
                                    ui.ctx().request_repaint();
                                }
                            });
                        });
                    });
                });
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
            widgets::search_field(ui, p, &mut self.query, hint, width);
            let kind = if self.view_open {
                widgets::ActionKind::Primary
            } else {
                widgets::ActionKind::Flat
            };
            if widgets::action_button(ui, p, "adjustments-horizontal-filled", &label, kind, Vec2::new(0.0, 26.0))
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

/// The complexity of the inspected objects; for an avatar, against the
/// user's RenderAvatarMaxComplexity (0: no limit), beyond which the
/// Legacy Viewer draws them as a jelly doll.
fn complexity_box(ui: &mut egui::Ui, p: &Palette, complexity: u32, limit: Option<u32>) {
    let bold = crate::ui::fonts::semibold(ui.ctx());
    egui::Frame::new()
        .fill(p.raised.gamma_multiply(0.5))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(9, 5))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(
                    RichText::new(aurora_i18n::tr!("inspect-totals-complexity").to_uppercase())
                        .size(10.5)
                        .extra_letter_spacing(0.7)
                        .family(bold.clone())
                        .color(p.muted),
                );
                ui.label(
                    RichText::new(aurora_i18n::integer(i64::from(complexity)))
                        .size(14.0)
                        .family(bold)
                        .color(p.ink),
                );
                if let Some(limit) = limit {
                    let (text, color) = if limit == 0 || complexity <= limit {
                        (aurora_i18n::tr!("inspect-within-limit"), p.success)
                    } else {
                        (aurora_i18n::tr!("inspect-over-limit"), p.warn)
                    };
                    ui.label(RichText::new(text).size(11.0).color(color))
                        .on_hover_text(aurora_i18n::tr!(
                            "inspect-limit-tip",
                            limit = aurora_i18n::integer(i64::from(limit))
                        ));
                }
            });
        });
}

/// Height of `complexity_box`: its margins and its two or three lines.
fn complexity_box_height(ui: &egui::Ui, with_limit: bool) -> f32 {
    let bold = crate::ui::fonts::semibold(ui.ctx());
    ui.ctx().fonts_mut(|f| {
        let mut h = 10.0 + f.row_height(&egui::FontId::new(10.5, bold.clone())) + 1.0 + f.row_height(&egui::FontId::new(14.0, bold));
        if with_limit {
            h += 1.0 + f.row_height(&egui::FontId::proportional(11.0));
        }
        h
    })
}

/// The quiet link beside the name in the summary card: a small icon, a face
/// only when pointed at.
fn profile_link(ui: &mut egui::Ui, p: &Palette) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
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

/// The summary line, "12 items · 4 creators · 31 prims": the figures in
/// the text's semi-bold ink, the words muted, whatever the language puts
/// around the number.
fn figures_line(ui: &egui::Ui, p: &Palette, parts: &[String]) -> egui::text::LayoutJob {
    let bold = egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let plain = egui::FontId::proportional(11.5);
    let mut job = egui::text::LayoutJob::default();
    let push = |job: &mut egui::text::LayoutJob, text: &str, figure: bool| {
        let (font, color) = if figure { (bold.clone(), p.ink) } else { (plain.clone(), p.muted) };
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: font,
                color,
                valign: egui::Align::BOTTOM,
                ..Default::default()
            },
        );
    };
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            push(&mut job, "  ·  ", false);
        }
        for (figure, run) in figure_runs(part) {
            push(&mut job, run, figure);
        }
    }
    job
}

/// `text` cut into runs of figures (digits, and the separators between
/// two digits) and of the rest.
fn figure_runs(text: &str) -> Vec<(bool, &str)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let digit = |i: usize| chars.get(i).is_some_and(|c| c.1.is_ascii_digit());
    let mut runs = Vec::new();
    let mut start = 0;
    let mut current = None;
    for (i, &(at, c)) in chars.iter().enumerate() {
        let separator = matches!(c, ',' | '.' | '\u{202F}' | '\u{A0}') && i > 0 && digit(i - 1) && digit(i + 1);
        let figure = c.is_ascii_digit() || separator;
        if let Some(was) = current
            && was != figure
        {
            runs.push((was, &text[start..at]));
            start = at;
        }
        current = Some(figure);
    }
    if let Some(was) = current {
        runs.push((was, &text[start..]));
    }
    runs
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

    #[test]
    fn figures_are_cut_from_their_words() {
        assert_eq!(figure_runs("12 items"), [(true, "12"), (false, " items")]);
        assert_eq!(figure_runs("1,204 prims"), [(true, "1,204"), (false, " prims")]);
        assert_eq!(figure_runs("12個のアイテム"), [(true, "12"), (false, "個のアイテム")]);
        assert_eq!(figure_runs("items, 3"), [(false, "items, "), (true, "3")]);
        assert_eq!(figure_runs(""), []);
    }

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
                ui.show(
                    ctx,
                    p,
                    Frame {
                        rows: &rows,
                        totals: &Totals::default(),
                        subject_name: "Mira Solvane",
                        subject_profile: Some(Uuid::from_u128(99)),
                        pics: &pics,
                        point_names: &names,
                        max_complexity: 350_000,
                        columns: &mut columns,
                        view: &mut view,
                    },
                );
            },
        );
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
    fn owner_and_dates_are_for_objects_of_the_world() {
        let only: Vec<Column> = Column::ALL.into_iter().filter(|c| c.for_objects_only()).collect();
        assert_eq!(only, [Column::Owner, Column::Created]);
    }
}
