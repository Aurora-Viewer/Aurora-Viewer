//! "Inspect Objects": what was right-clicked (everything an avatar wears, or
//! objects of the world) as a list of objects, one per linkset, with their
//! creators; their figures on demand; and the Legacy prim table with its
//! totals. Port of LLFloaterInspect with Firestorm's additions
//! (indra/newview/llfloaterinspect.cpp, floater_inspect.xml,
//! menu_fs_inspect_options.xml, originally LGPL 2.1): description column,
//! "Improved Object Inspect" figures, complexity, sorting, columns chosen in
//! the options menu (FSInspectColumnConfig).
//!
//! The window is a board of tiles (`wide.rs`), Aurora's own presentation of
//! the same data (human request, 2026-11-23; one view only since
//! 2026-11-26): the Legacy Viewer only has the table, kept as the Prims tile.

mod details;
pub(crate) mod model;
mod people;
pub(crate) mod wide;

use super::widgets::Floater;
use crate::theme::Palette;
use egui::{TextureHandle, Vec2};
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
    /// The people here with their complexity, for an avatar.
    pub people: &'a [Person],
    /// Land impact used and allowed on the parcel the objects are on.
    pub parcel: Option<(i32, i32)>,
    pub columns: &'a mut u32,
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

/// The window's size when it opens, as much of it as the screen has room
/// for, and the least it shrinks to (the board's tiles need it).
const SIZE: Vec2 = Vec2::new(800.0, 760.0);
const MIN_SIZE: Vec2 = Vec2::new(680.0, 520.0);

#[derive(Default)]
pub struct InspectUi {
    pub open: bool,
    /// Root prims of the inspected objects.
    pub roots: Vec<Uuid>,
    /// The prims whose properties were asked for (ObjectSelect).
    pub asked: HashSet<Uuid>,
    /// The parts of the avatar's own body that show, with their bake, when
    /// they were last looked at, and whether there are some.
    pub own_parts: Vec<(usize, Option<Uuid>)>,
    pub own_parts_at: Option<std::time::Instant>,
    pub own_body: bool,
    /// The highlighted prim, outlined in the world (LLFloaterInspect::getSelectedUUID).
    pub selected: Option<Uuid>,
    pub subject: Subject,
    query: String,
    /// The object opened in the lists, by its root.
    opened: Option<Uuid>,
    /// Column the prims are sorted on and whether descending; None keeps the
    /// linksets together, their root first.
    sort: Option<(Column, bool)>,
    /// Profile pictures the window shows, for the app to fetch.
    pub wanted_pics: HashSet<Uuid>,
    /// AURORA_DEMO_INSPECT has opened the window (or is not set).
    pub demo_done: bool,
    /// The 3D view.
    pub stage: super::object_stage::StageUi,
    wide: wide::Wide,
    /// AURORA_DEMO_INSPECT's `hover`.
    demo_hover: bool,
    demo_paused: bool,
    demo_light: bool,
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
        self.query.clear();
        self.wide.restart();
        // one object: its 3D view, large
        self.wide.grown = (roots.len() == 1).then_some(wide::Tile::Stage);
        self.stage.restart();
        self.subject = subject;
        self.roots = roots;
        self.own_parts_at = None;
        self.own_body = false;
    }

    /// AURORA_DEMO_INSPECT's options: `item` the third object, `grow-…` a
    /// tile grown, `hover` its 3D view with the fourth pointed at, `paused`
    /// and `light` its animation and its ground.
    pub fn demo(&mut self, option: &str) {
        match option {
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
            "paused" => self.demo_paused = true,
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

    /// AURORA_DEMO_INSPECT's pause and ground.
    pub fn demo_stage(&mut self) {
        if self.demo_paused {
            self.stage.pause();
        }
        self.stage.light |= self.demo_light;
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
            aurora_i18n::tr!("inspect-window"),
            egui::pos2(240.0, 60.0),
            SIZE.min(super::layout::work_area(ctx).size() - Vec2::splat(2.0 * super::layout::GAP)),
        );
        floater.min_size = MIN_SIZE;
        let mut frame = frame;
        floater.show(ctx, p, &mut open, |ui| {
            wide::show(self, ui, p, &mut frame, &items, &mut actions);
        });
        if !open {
            self.open = false;
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window keeps its size, its footer inside it, every tile grown in
    /// turn, at its first size and at the least it shrinks to.
    #[test]
    fn the_window_keeps_its_size() {
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
                    // as short as it goes too: the tiles beside a grown one scroll (#5027)
                    &[SIZE, MIN_SIZE],
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
                        let mut columns = ALL_COLUMNS;
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
                            },
                        );
                    },
                );
            }
        }
    }

    #[test]
    fn owner_and_dates_wait_to_be_chosen_for_an_avatar() {
        let hidden: Vec<Column> = Column::ALL.into_iter().filter(|c| !shown(AVATAR_COLUMNS, *c)).collect();
        assert_eq!(hidden, [Column::Owner, Column::Created]);
        assert!(Column::ALL.into_iter().all(|c| shown(ALL_COLUMNS, c)));
    }
}
