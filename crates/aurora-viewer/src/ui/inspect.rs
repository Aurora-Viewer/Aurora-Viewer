//! "Inspect Objects": the prims of what was right-clicked, with
//! their names, owners, creators, creation dates, geometry and texture
//! figures, and the totals of the selection. Port of LLFloaterInspect with
//! Firestorm's additions (indra/newview/llfloaterinspect.cpp,
//! floater_inspect.xml, menu_fs_inspect_options.xml, originally LGPL 2.1):
//! description column, "Improved Object Inspect" figures, complexity,
//! sorting, columns chosen in the options menu (FSInspectColumnConfig).

use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use egui::{Rect, RichText, Sense, Vec2};
use uuid::Uuid;

/// The columns, in floater_inspect.xml's order; `bit` is the column's bit
/// of FSInspectColumnConfig (LLFloaterInspect::mColumnBits).
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

/// Width of the totals, left of the table.
const STATS_WIDTH: f32 = 150.0;

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
    /// Seconds since the epoch.
    pub created: u64,
    pub faces: u32,
    pub vertices: u32,
    pub triangles: u32,
    /// Texture memory and its size on the GPU, in bytes.
    pub texture_memory: u64,
    pub vram: u64,
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

#[derive(Default)]
pub struct InspectUi {
    pub open: bool,
    /// Root prims of the inspected objects.
    pub roots: Vec<Uuid>,
    /// The highlighted row (LLFloaterInspect::getSelectedUUID).
    pub selected: Option<Uuid>,
    /// Column sorted on and whether descending; None keeps the linksets
    /// together, their root first.
    sort: Option<(Column, bool)>,
}

/// Rows in the order of the table.
fn sorted(rows: &[Row], sort: Option<(Column, bool)>) -> Vec<&Row> {
    let mut list: Vec<&Row> = rows.iter().collect();
    let Some((column, descending)) = sort else {
        return list;
    };
    let text = |a: &str, b: &str| a.to_lowercase().cmp(&b.to_lowercase());
    list.sort_by(|a, b| {
        let order = match column {
            Column::Name => text(&a.name, &b.name),
            Column::Description => text(&a.description, &b.description),
            Column::Owner => text(&a.owner_name, &b.owner_name),
            Column::Creator => text(&a.creator_name, &b.creator_name),
            Column::Faces => a.faces.cmp(&b.faces),
            Column::Vertices => a.vertices.cmp(&b.vertices),
            Column::Triangles => a.triangles.cmp(&b.triangles),
            Column::TextureMemory => a.texture_memory.cmp(&b.texture_memory),
            Column::Vram => a.vram.cmp(&b.vram),
            Column::Created => a.created.cmp(&b.created),
        };
        if descending { order.reverse() } else { order }
    });
    list
}

/// The visible columns' cells over `row`: figures at their width, the
/// columns of text sharing the rest, the name a little wider.
fn cells(columns: u32, row: Rect) -> Vec<(Column, Rect)> {
    let visible: Vec<Column> = Column::ALL.into_iter().filter(|c| shown(columns, *c)).collect();
    let fixed: f32 = visible.iter().filter_map(|c| c.fixed_width()).sum();
    let weight = |c: Column| if c == Column::Name { 1.4 } else { 1.0 };
    let weights: f32 = visible.iter().filter(|c| c.fixed_width().is_none()).map(|c| weight(*c)).sum();
    let unit = if weights > 0.0 {
        ((row.width() - fixed) / weights).max(60.0)
    } else {
        0.0
    };
    let mut x = row.left();
    visible
        .into_iter()
        .map(|c| {
            let w = c.fixed_width().unwrap_or(unit * weight(c));
            let cell = Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(x + w, row.bottom()));
            x += w;
            (c, cell)
        })
        .collect()
}

fn kilobytes(bytes: u64) -> String {
    aurora_i18n::integer((bytes / 1024) as i64)
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

fn cell_text(row: &Row, column: Column) -> String {
    match column {
        // a linked prim is indented under its root, as Firestorm's three spaces
        Column::Name if !row.root => format!("    {}", row.name),
        Column::Name => row.name.clone(),
        Column::Description => row.description.clone(),
        Column::Owner => row.owner_name.clone(),
        Column::Creator => row.creator_name.clone(),
        Column::Faces => aurora_i18n::integer(i64::from(row.faces)),
        Column::Vertices => aurora_i18n::integer(i64::from(row.vertices)),
        Column::Triangles => aurora_i18n::integer(i64::from(row.triangles)),
        Column::TextureMemory => kilobytes(row.texture_memory),
        Column::Vram => kilobytes(row.vram),
        Column::Created => slt_datetime(row.created),
    }
}

fn text_right(ui: &egui::Ui, text: String, cell: Rect, color: egui::Color32) {
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(12.0), color);
    let left = egui::pos2(cell.right() - 8.0 - galley.size().x, cell.center().y);
    widgets::paint_ink_left(ui.painter(), galley, left, color);
}

impl InspectUi {
    /// LLFloaterInspect::onOpen on the objects of `roots`.
    pub fn inspect(&mut self, roots: Vec<Uuid>) {
        self.open = true;
        self.selected = roots.first().copied();
        self.roots = roots;
    }

    fn header(&mut self, ui: &mut egui::Ui, p: &Palette, columns: u32) {
        // the columns over those of the rows, left of the list's scroll bar
        let width = widgets::header_width(ui, ui.scope_id().with("inspect_rows"));
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
        for (column, cell) in cells(columns, rect.with_max_x(rect.left() + width)) {
            let resp = ui.interact(cell, ui.scope_id().with(("inspect_header", column.bit())), Sense::click());
            let sorted = self.sort.filter(|(c, _)| *c == column);
            let color = if sorted.is_some() || resp.hovered() { p.ink } else { p.muted };
            let font = egui::FontId::proportional(12.0);
            // the arrow of the sorted column takes the end of its cell
            let room = cell.width() - if sorted.is_some() { 22.0 } else { 12.0 };
            if column.numeric() {
                let end = Rect::from_min_max(cell.min, egui::pos2(cell.left() + room + 8.0, cell.bottom()));
                text_right(ui, column.title().to_owned(), end, color);
            } else {
                let left = egui::pos2(cell.left() + 8.0, cell.center().y);
                widgets::paint_elided(ui, column.title(), left, room, font, color);
            }
            if let Some((_, descending)) = sorted {
                let arrow = if descending { "caret-down-filled" } else { "caret-up-filled" };
                if let Some(texture) = crate::ui::icons::global(arrow) {
                    let at = Rect::from_center_size(egui::pos2(cell.right() - 8.0, cell.center().y), Vec2::splat(8.0));
                    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(texture.id(), at, uv, p.violet_light);
                }
            }
            if resp.clicked() {
                self.sort = match sorted {
                    Some((_, false)) => Some((column, true)),
                    // a third click goes back to the linksets' own order
                    Some((_, true)) => None,
                    None => Some((column, false)),
                };
            }
            resp.on_hover_text(column.title());
        }
    }

    fn rows(&mut self, ui: &mut egui::Ui, p: &Palette, rows: &[Row], columns: u32) {
        let list = sorted(rows, self.sort);
        let grouped = self.sort.is_none();
        let out = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if list.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(aurora_i18n::tr!("inspect-empty")).size(12.0).color(p.muted));
            }
            let radius = ui.visuals().window_corner_radius;
            for row in list {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                if self.selected == Some(row.id) {
                    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                }
                let pointer = resp.hover_pos();
                let mut tip = None;
                for (column, cell) in cells(columns, rect) {
                    // sorted, the prims leave their linkset: no indent
                    let text = if column == Column::Name && !grouped {
                        row.name.clone()
                    } else {
                        cell_text(row, column)
                    };
                    if column.numeric() {
                        text_right(ui, text, cell, p.ink);
                        continue;
                    }
                    let left = egui::pos2(cell.left() + 8.0, cell.center().y);
                    let color = if column == Column::Name { p.ink } else { p.muted };
                    let cut = widgets::paint_elided(ui, &text, left, cell.width() - 12.0, egui::FontId::proportional(12.5), color);
                    if cut && pointer.is_some_and(|at| cell.contains(at)) {
                        tip = Some(text.trim_start().to_owned());
                    }
                }
                if resp.clicked() {
                    self.selected = Some(row.id);
                }
                if let Some(tip) = tip {
                    resp.on_hover_text(tip);
                }
            }
        });
        widgets::remember_scroll_bar(ui, ui.scope_id().with("inspect_rows"), &out);
    }

    /// Draw the window. Returns the avatar whose profile a button asked for.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, rows: &[Row], totals: &Totals, columns: &mut u32) -> Option<Uuid> {
        if !self.open {
            return None;
        }
        // the highlighted prim is gone: the first one (refresh's selectNthItem(0))
        if !rows.iter().any(|r| Some(r.id) == self.selected) {
            self.selected = rows.first().map(|r| r.id);
        }
        let mut open = true;
        let mut profile = None;
        let mut floater = Floater::new(
            "inspect",
            aurora_i18n::tr!("inspect-title"),
            egui::pos2(240.0, 120.0),
            Vec2::new(1040.0, 360.0),
        );
        floater.min_size = Vec2::new(560.0, 300.0);
        floater.show(ctx, p, &mut open, |ui| {
            // the buttons keep the bottom of the window; the totals and the
            // table share what is above them, whatever its height (a guessed
            // button row grew the window at every frame with a larger text)
            const BUTTONS: f32 = 26.0;
            let (mut list, foot) = widgets::above_footer(ui);
            let height = list.available_height();
            list.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(STATS_WIDTH, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(STATS_WIDTH);
                    ui.set_height(height);
                    stats(ui, p, totals);
                });
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_height(height);
                        self.header(ui, p, *columns);
                        self.rows(ui, p, rows, *columns);
                    },
                );
            });
            let current = rows.iter().find(|r| Some(r.id) == self.selected);
            foot.show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.set_height(BUTTONS);
                    let size = Vec2::new(0.0, 26.0);
                    for (label, tip, who) in [
                        (
                            aurora_i18n::tr!("inspect-owner-profile"),
                            aurora_i18n::tr!("inspect-owner-profile-tip"),
                            current.and_then(|r| r.owner),
                        ),
                        (
                            aurora_i18n::tr!("inspect-creator-profile"),
                            aurora_i18n::tr!("inspect-creator-profile-tip"),
                            current.and_then(|r| r.creator),
                        ),
                    ] {
                        let button = widgets::action_button_if(ui, p, "user-filled", label, ActionKind::Flat, size, who.is_some());
                        if button.on_hover_text(tip).clicked() {
                            profile = who;
                        }
                    }
                    // menu_fs_inspect_options.xml: the name stays
                    widgets::icon_menu(ui, p, "adjustments-filled", aurora_i18n::tr!("inspect-options"), |ui| {
                        for column in Column::ALL.into_iter().skip(1) {
                            let mut on = shown(*columns, column);
                            if super::menu::toggle(ui, p, "", column.title(), &mut on) {
                                *columns ^= column.bit();
                            }
                        }
                    });
                });
            });
        });
        if !open {
            self.open = false;
        }
        profile
    }
}

fn stats(ui: &mut egui::Ui, p: &Palette, t: &Totals) {
    let n = |v: u32| aurora_i18n::integer(i64::from(v));
    ui.label(RichText::new(aurora_i18n::tr!("inspect-totals")).size(12.5).strong().color(p.ink));
    ui.add_space(2.0);
    ui.label(
        RichText::new(aurora_i18n::tr!("inspect-totals-objects", objects = t.objects, prims = t.prims))
            .size(12.0)
            .color(p.ink),
    );
    ui.add_space(6.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    // a label and its figure against the right edge, on one short line
    let line = |ui: &mut egui::Ui, label: &str, value: String| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 19.0), Sense::hover());
        let left = egui::pos2(rect.left(), rect.center().y);
        widgets::paint_elided(ui, label, left, rect.width() * 0.62, egui::FontId::proportional(12.0), p.muted);
        text_right(ui, value, rect, p.ink);
    };
    line(ui, aurora_i18n::tr!("inspect-col-faces"), n(t.faces));
    line(ui, aurora_i18n::tr!("inspect-totals-visible"), n(t.visible_faces));
    line(ui, aurora_i18n::tr!("inspect-col-vertices"), n(t.vertices));
    line(ui, aurora_i18n::tr!("inspect-col-triangles"), n(t.triangles));
    line(ui, aurora_i18n::tr!("inspect-totals-complexity"), n(t.complexity));
    ui.add_space(6.0);
    line(ui, aurora_i18n::tr!("inspect-totals-textures"), n(t.textures));
    line(ui, aurora_i18n::tr!("inspect-col-tmem"), kilobytes(t.texture_memory));
    line(ui, aurora_i18n::tr!("inspect-col-vram"), kilobytes(t.vram));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window keeps its size, its buttons inside it.
    #[test]
    fn the_window_keeps_its_size() {
        let rows: Vec<Row> = (0..40)
            .map(|i| Row {
                id: Uuid::from_u128(i),
                root: i % 4 == 0,
                ..Default::default()
            })
            .collect();
        crate::ui::widgets::steady_tests::assert_window_steady(
            "inspect",
            &[Vec2::new(1040.0, 360.0), Vec2::new(560.0, 300.0)],
            aurora_i18n::tr!("inspect-owner-profile"),
            || InspectUi {
                open: true,
                ..Default::default()
            },
            |ui, ctx, p| {
                ui.show(ctx, p, &rows, &Totals::default(), &mut u32::MAX.clone());
            },
        );
    }

    fn row(n: u128, name: &str, triangles: u32) -> Row {
        Row {
            id: Uuid::from_u128(n),
            root: n == 1,
            name: name.into(),
            triangles,
            ..Row::default()
        }
    }

    #[test]
    fn rows_keep_their_linkset_order_until_a_column_is_sorted() {
        let rows = [row(1, "Racine", 12), row(2, "bras", 900), row(3, "Anneau", 40)];
        let names = |sort| sorted(&rows, sort).into_iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names(None), ["Racine", "bras", "Anneau"]);
        // names without regard to case, figures as numbers
        assert_eq!(names(Some((Column::Name, false))), ["Anneau", "bras", "Racine"]);
        assert_eq!(names(Some((Column::Triangles, true))), ["bras", "Anneau", "Racine"]);
    }

    #[test]
    fn hidden_columns_leave_their_width_to_the_columns_of_text() {
        let line = Rect::from_min_size(egui::pos2(10.0, 0.0), Vec2::new(900.0, 22.0));
        let all = cells(ALL_COLUMNS, line);
        assert_eq!(all.len(), 10);
        assert!((all.last().unwrap().1.right() - line.right()).abs() < 0.01);
        // figures keep their width, the name is the widest column of text
        let width = |list: &[(Column, Rect)], c: Column| list.iter().find(|(k, _)| *k == c).map_or(0.0, |(_, r)| r.width());
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(width(&all, Column::Faces), 52.0));
        assert!(width(&all, Column::Name) > width(&all, Column::Owner));
        let few = cells(Column::Name.bit() | Column::Triangles.bit(), line);
        assert_eq!(few.len(), 2);
        assert!(close(width(&few, Column::Name), 900.0 - 70.0));
        assert!(close(width(&few, Column::Triangles), width(&all, Column::Triangles)));
    }

    #[test]
    fn cells_show_firestorms_units_and_indent_linked_prims() {
        let mut r = row(2, "bras", 1234);
        r.texture_memory = 3 * 1024 * 1024;
        r.created = 1_700_000_000;
        assert_eq!(cell_text(&r, Column::Name), "    bras");
        // kilobytes, as Firestorm's TMem and VRAM columns
        assert_eq!(cell_text(&r, Column::TextureMemory), aurora_i18n::integer(3072));
        // 2023-11-14 22:13:20 UTC is 14:13 in SLT (PST)
        let date = aurora_i18n::tr!("inspect-date", year = "2023", month = "11", day = "14", hour = "14", minute = "13");
        assert_eq!(cell_text(&r, Column::Created), date);
        r.created = 0;
        assert_eq!(cell_text(&r, Column::Created), "");
    }
}
