//! Personal construction layout and portable settings. Object properties and
//! transforms never belong to a layout or its notecard.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// A block of the Aurora build window that can live in a window of its
/// own: a section, or the Position / Size / Rotation block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Block {
    General,
    Shape,
    Features,
    Materials,
    Contents,
    Transform,
}

impl Block {
    pub const ALL: [Block; 6] = [
        Block::General,
        Block::Shape,
        Block::Features,
        Block::Materials,
        Block::Contents,
        Block::Transform,
    ];

    /// The section's rail item; None for the transform block.
    pub fn item(self) -> Option<Item> {
        match self {
            Block::General => Some(Item::General),
            Block::Shape => Some(Item::Shape),
            Block::Features => Some(Item::Features),
            Block::Materials => Some(Item::Materials),
            Block::Contents => Some(Item::Contents),
            Block::Transform => None,
        }
    }

    pub fn of_item(item: Item) -> Option<Block> {
        Block::ALL.into_iter().find(|block| block.item() == Some(item))
    }

    /// The build tool's tab of the section (`BuildTool::tab`).
    pub fn tab(self) -> Option<usize> {
        match self {
            Block::General => Some(6),
            Block::Shape => Some(5),
            Block::Features => Some(2),
            Block::Materials => Some(3),
            Block::Contents => Some(4),
            Block::Transform => None,
        }
    }

    pub fn of_tab(tab: usize) -> Option<Block> {
        match tab {
            0 | 1 | 6 => Some(Block::General),
            _ => Block::ALL.into_iter().find(|block| block.tab() == Some(tab)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Item {
    #[serde(rename = "manipulators")]
    Manipulators,
    #[serde(rename = "mode-move")]
    MoveMode,
    #[serde(rename = "mode-rotate")]
    RotateMode,
    #[serde(rename = "mode-stretch")]
    StretchMode,
    #[serde(rename = "mode-align")]
    AlignMode,
    #[serde(rename = "face")]
    Face,
    #[serde(rename = "stretchTextures")]
    StretchTextures,
    #[serde(rename = "linked")]
    Linked,
    #[serde(rename = "scaleBoth")]
    ScaleBoth,
    #[serde(rename = "rootAxis")]
    RootAxis,
    #[serde(rename = "snap")]
    Snap,
    #[serde(rename = "highlight")]
    Highlight,
    #[serde(rename = "probes")]
    Probes,
    #[serde(rename = "ownedOnly")]
    OwnedOnly,
    #[serde(rename = "movableOnly")]
    MovableOnly,
    #[serde(rename = "surrounding")]
    Surrounding,
    #[serde(rename = "landGroup")]
    LandGroup,
    #[serde(rename = "followEdit")]
    FollowEdit,
    #[serde(rename = "wheelOnHover")]
    WheelOnHover,
    #[serde(rename = "actions")]
    Actions,
    #[serde(rename = "nav-section-general")]
    General,
    #[serde(rename = "nav-section-shape")]
    // Keep the saved identifier of the section now titled Object.
    Shape,
    #[serde(rename = "nav-section-materials")]
    Materials,
    #[serde(rename = "nav-section-features")]
    Features,
    #[serde(rename = "nav-section-contents")]
    Contents,
    #[serde(rename = "nav-tool-camera")]
    Camera,
    #[serde(rename = "nav-tool-grab")]
    Grab,
    #[serde(rename = "nav-tool-edit")]
    Edit,
    #[serde(rename = "nav-tool-create")]
    Create,
    #[serde(rename = "nav-tool-land")]
    Land,
}

pub const MANIPULATION_ITEMS: [Item; 4] = [Item::MoveMode, Item::RotateMode, Item::StretchMode, Item::AlignMode];

pub const ITEMS: [Item; 29] = [
    Item::MoveMode,
    Item::ScaleBoth,
    Item::StretchTextures,
    Item::RotateMode,
    Item::Snap,
    Item::StretchMode,
    Item::RootAxis,
    Item::Face,
    Item::Highlight,
    Item::AlignMode,
    Item::Probes,
    Item::Linked,
    Item::Actions,
    Item::OwnedOnly,
    Item::MovableOnly,
    Item::Surrounding,
    Item::LandGroup,
    Item::FollowEdit,
    Item::WheelOnHover,
    Item::General,
    Item::Shape,
    Item::Features,
    Item::Materials,
    Item::Contents,
    Item::Camera,
    Item::Grab,
    Item::Edit,
    Item::Create,
    Item::Land,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Panel {
    Footer,
    Settings,
    Hidden,
    /// The bar above the editor (the sections, by default).
    Navigation,
    /// The bar at the bottom of the window (the tools, by default). Either
    /// bar takes any of the section and tool buttons, as many as wanted.
    Tools,
}

impl Panel {
    /// One of the two button bars.
    pub fn bar(self) -> bool {
        matches!(self, Self::Navigation | Self::Tools)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FooterColumn {
    Left,
    Right,
}

impl FooterColumn {
    pub fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FooterRow {
    Full(Item),
    Cells([Option<Item>; 2]),
}

impl Item {
    pub fn navigation(self) -> bool {
        self >= Self::General
    }
    pub fn footer_column_item(self) -> bool {
        !self.navigation() && !matches!(self, Self::Manipulators | Self::Actions)
    }
    /// A tool's button (Camera, Grab, Edit, Create, Land), not a section's.
    pub fn tool(self) -> bool {
        self >= Self::Camera
    }
    pub fn accepts(self, panel: Panel) -> bool {
        panel == Panel::Hidden
            || if self.navigation() {
                panel.bar()
            } else {
                matches!(panel, Panel::Footer | Panel::Settings)
            }
    }
    pub fn default_panel(self) -> Panel {
        if self.tool() {
            Panel::Tools
        } else if self.navigation() {
            Panel::Navigation
        } else if matches!(
            self,
            Self::OwnedOnly | Self::MovableOnly | Self::Surrounding | Self::LandGroup | Self::FollowEdit | Self::WheelOnHover
        ) {
            Panel::Settings
        } else {
            Panel::Footer
        }
    }
}

/// How a bar button shows: its icon, its label, or both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonDisplay {
    #[default]
    Icon,
    Label,
    Both,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub order: Vec<Item>,
    pub locations: BTreeMap<Item, Panel>,
    /// Independent column choices keep a drop from shifting other controls.
    /// Older settings and version-one notecards infer their existing columns.
    #[serde(default, rename = "footerColumns")]
    pub footer_columns: BTreeMap<Item, FooterColumn>,
    /// Upgrade the former default section order once, retaining custom orders.
    #[serde(default, rename = "sectionOrderVersion")]
    section_order_version: u8,
    /// Move a saved layout that is still the former default to the new
    /// default option order (root axis above the highlight), once.
    #[serde(default, rename = "footerOrderVersion")]
    footer_order_version: u8,
    /// Layouts from before the two bars took any button kept the tool
    /// buttons in the navigation panel: they go to the bottom bar, once.
    #[serde(default, rename = "barsVersion")]
    bars_version: u8,
    /// The bar buttons shown otherwise than by their icon alone.
    #[serde(default, rename = "buttonDisplay", skip_serializing_if = "BTreeMap::is_empty")]
    pub button_display: BTreeMap<Item, ButtonDisplay>,
}

impl Default for Layout {
    fn default() -> Self {
        let mut layout = Self {
            order: ITEMS.to_vec(),
            locations: ITEMS.into_iter().map(|item| (item, item.default_panel())).collect(),
            footer_columns: [
                (Item::MoveMode, FooterColumn::Left),
                (Item::RotateMode, FooterColumn::Left),
                (Item::StretchMode, FooterColumn::Left),
                (Item::Face, FooterColumn::Left),
                (Item::AlignMode, FooterColumn::Left),
                (Item::Linked, FooterColumn::Left),
                (Item::ScaleBoth, FooterColumn::Right),
                (Item::StretchTextures, FooterColumn::Right),
                (Item::Snap, FooterColumn::Right),
                (Item::RootAxis, FooterColumn::Right),
                (Item::Highlight, FooterColumn::Right),
                (Item::Probes, FooterColumn::Right),
            ]
            .into_iter()
            .collect(),
            section_order_version: 1,
            footer_order_version: 1,
            bars_version: 1,
            button_display: BTreeMap::new(),
        };
        layout.normalize();
        layout
    }
}

impl Layout {
    /// Import old preferences without duplicates or inaccessible destinations.
    pub fn normalize(&mut self) {
        let old_position = self.order.iter().position(|item| *item == Item::Manipulators);
        let old_panel = self.locations.remove(&Item::Manipulators);
        if old_position.is_some() || old_panel.is_some() {
            let panel = old_panel
                .filter(|panel| Item::Manipulators.accepts(*panel))
                .unwrap_or(Panel::Footer);
            self.order.retain(|item| *item != Item::Manipulators);
            let mut position = old_position.unwrap_or(0).min(self.order.len());
            for item in MANIPULATION_ITEMS {
                self.locations.entry(item).or_insert(panel);
                if !self.order.contains(&item) {
                    self.order.insert(position, item);
                    position += 1;
                }
            }
        }
        self.button_display
            .retain(|item, display| item.navigation() && *display != ButtonDisplay::Icon);
        if self.bars_version == 0 {
            for (item, panel) in self.locations.iter_mut() {
                if item.tool() && *panel == Panel::Navigation {
                    *panel = Panel::Tools;
                }
            }
            self.bars_version = 1;
        }
        let mut seen = HashSet::new();
        self.order.retain(|item| seen.insert(*item));
        for (i, item) in ITEMS.into_iter().enumerate() {
            if seen.insert(item) {
                // an item added in a later version goes right after its
                // predecessor in ITEMS (new selection filters under the
                // reflection probes), not at the end of a saved layout
                let after = i
                    .checked_sub(1)
                    .and_then(|prev| self.order.iter().position(|key| *key == ITEMS[prev]));
                match after {
                    Some(at) => self.order.insert(at + 1, item),
                    None => self.order.push(item),
                }
            }
            let panel = self.locations.entry(item).or_insert(item.default_panel());
            if !item.accepts(*panel) {
                *panel = item.default_panel();
            }
        }
        if self.section_order_version == 0 {
            let former_default = self
                .in_panel(Panel::Navigation)
                .into_iter()
                .filter(|item| {
                    matches!(
                        item,
                        Item::General | Item::Shape | Item::Materials | Item::Features | Item::Contents
                    )
                })
                .eq([Item::General, Item::Shape, Item::Materials, Item::Features, Item::Contents]);
            if former_default
                && let Some(materials) = self.order.iter().position(|item| *item == Item::Materials)
                && let Some(features) = self.order.iter().position(|item| *item == Item::Features)
            {
                self.order.swap(materials, features);
            }
            self.section_order_version = 1;
        }
        // The action bar belongs at an edge of the options block. Repair
        // older placements without changing either column's own order.
        let footer = self.in_panel(Panel::Footer);
        if let Some(actions) = footer.iter().position(|item| *item == Item::Actions)
            && footer[..actions].iter().any(|item| item.footer_column_item())
            && footer[actions + 1..].iter().any(|item| item.footer_column_item())
        {
            self.anchor_footer_actions(false);
        }
        self.footer_columns.retain(|item, _| item.footer_column_item());
        let mut column = FooterColumn::Left;
        for item in self.in_panel(Panel::Footer) {
            if matches!(item, Item::Face | Item::Linked) && !self.footer_columns.contains_key(&item) {
                // Infer the former full-row selectors once; explicit column
                // choices, including Right, always remain user-controlled.
                self.footer_columns.insert(item, FooterColumn::Left);
                column = FooterColumn::Left;
            } else if item.footer_column_item() {
                let chosen = *self.footer_columns.entry(item).or_insert(column);
                column = match chosen {
                    FooterColumn::Left => FooterColumn::Right,
                    FooterColumn::Right => FooterColumn::Left,
                };
            } else {
                column = FooterColumn::Left;
            }
        }
        if self.footer_order_version == 0 {
            // Only a layout nobody arranged: the former default exactly.
            let mut former = Self::default();
            former.swap_order(Item::RootAxis, Item::Highlight);
            if self.order == former.order && self.locations == former.locations && self.footer_columns == former.footer_columns {
                self.swap_order(Item::Highlight, Item::RootAxis);
            }
            self.footer_order_version = 1;
        }
    }

    fn swap_order(&mut self, a: Item, b: Item) {
        if let Some(i) = self.order.iter().position(|item| *item == a)
            && let Some(j) = self.order.iter().position(|item| *item == b)
        {
            self.order.swap(i, j);
        }
    }
    /// How the bar button `item` shows.
    pub fn display(&self, item: Item) -> ButtonDisplay {
        self.button_display.get(&item).copied().unwrap_or_default()
    }

    pub fn set_display(&mut self, item: Item, display: ButtonDisplay) {
        if display == ButtonDisplay::Icon || !item.navigation() {
            self.button_display.remove(&item);
        } else {
            self.button_display.insert(item, display);
        }
    }

    /// The buttons of both bars, the top one first.
    pub fn in_bars(&self) -> Vec<Item> {
        let mut items = self.in_panel(Panel::Navigation);
        items.extend(self.in_panel(Panel::Tools));
        items
    }

    pub fn in_panel(&self, panel: Panel) -> Vec<Item> {
        self.order
            .iter()
            .copied()
            .filter(|item| self.locations.get(item) == Some(&panel))
            .collect()
    }

    pub fn footer_actions_top(&self) -> bool {
        let footer = self.in_panel(Panel::Footer);
        match (
            footer.iter().position(|item| *item == Item::Actions),
            footer.iter().position(|item| item.footer_column_item()),
        ) {
            (Some(actions), Some(first)) => actions < first,
            _ => false,
        }
    }

    fn anchor_footer_actions(&mut self, top: bool) {
        if self.locations.get(&Item::Actions) != Some(&Panel::Footer) {
            return;
        }
        let mut columns = self
            .order
            .iter()
            .copied()
            .filter(|item| item.footer_column_item() && self.locations.get(item) == Some(&Panel::Footer));
        let target = if top { columns.next() } else { columns.next_back() };
        let Some(target) = target else { return };
        self.order.retain(|item| *item != Item::Actions);
        if let Some(index) = self.order.iter().position(|item| *item == target) {
            self.order.insert(index + usize::from(!top), Item::Actions);
        }
    }

    pub fn place(&mut self, item: Item, panel: Panel, target: Option<Item>, after: bool) -> bool {
        if !item.accepts(panel) || target == Some(item) {
            return false;
        }
        if target.is_some_and(|target| self.locations.get(&target) != Some(&panel)) {
            return false;
        }
        self.normalize();
        if item == Item::Actions
            && panel == Panel::Footer
            && let Some(target) = target
        {
            let columns: Vec<_> = self
                .in_panel(Panel::Footer)
                .into_iter()
                .filter(|item| item.footer_column_item())
                .collect();
            if !((columns.first() == Some(&target) && !after) || (columns.last() == Some(&target) && after)) {
                return false;
            }
        }
        let actions_top = self.footer_actions_top();
        self.order.retain(|key| *key != item);
        let index = target
            .and_then(|target| self.order.iter().position(|key| *key == target))
            .map(|index| index + usize::from(after))
            .unwrap_or(self.order.len());
        self.order.insert(index, item);
        self.locations.insert(item, panel);
        if item.footer_column_item() {
            self.anchor_footer_actions(actions_top);
        }
        self.normalize();
        true
    }

    pub fn place_in_footer(&mut self, item: Item, column: FooterColumn, target: Option<Item>, after: bool) -> bool {
        if target == Some(item) {
            if !item.footer_column_item() || self.locations.get(&item) != Some(&Panel::Footer) {
                return false;
            }
        } else if !self.place(item, Panel::Footer, target, after) {
            return false;
        }
        if item.footer_column_item() {
            self.footer_columns.insert(item, column);
        }
        true
    }

    pub fn footer_rows(&self) -> Vec<FooterRow> {
        let mut rows = Vec::new();
        let mut columns = [Vec::new(), Vec::new()];
        let flush = |columns: &mut [Vec<Item>; 2], rows: &mut Vec<FooterRow>| {
            for index in 0..columns[0].len().max(columns[1].len()) {
                rows.push(FooterRow::Cells([columns[0].get(index).copied(), columns[1].get(index).copied()]));
            }
            columns.iter_mut().for_each(Vec::clear);
        };
        for item in self.in_panel(Panel::Footer) {
            if !item.footer_column_item() {
                flush(&mut columns, &mut rows);
                rows.push(FooterRow::Full(item));
                continue;
            }
            let column = self
                .footer_columns
                .get(&item)
                .copied()
                .unwrap_or(if columns[0].len() <= columns[1].len() {
                    FooterColumn::Left
                } else {
                    FooterColumn::Right
                });
            columns[column.index()].push(item);
        }
        flush(&mut columns, &mut rows);
        rows
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub face: bool,
    pub stretch_textures: bool,
    pub linked: bool,
    pub scale_both: bool,
    pub root_axis: bool,
    pub snap: bool,
    /// Absent from notecards exported before it existed: off.
    #[serde(default)]
    pub object_snap: bool,
    #[serde(default)]
    pub snap_objects_preferred: bool,
    pub highlight: bool,
    pub probes: bool,
    /// Absent from notecards exported before these options existed.
    #[serde(default)]
    pub owned_only: bool,
    #[serde(default)]
    pub movable_only: bool,
    /// On when absent, its default.
    #[serde(default = "surrounding_default")]
    pub surrounding: bool,
    pub land_group: bool,
    pub follow_edit: bool,
    /// Absent from notecards exported before it existed: off.
    #[serde(default)]
    pub wheel_on_hover: bool,
    /// Absent from notecards exported before they existed: add or take out
    /// the object, the default.
    #[serde(default)]
    pub shift_click: crate::build::face_buttons::ModifierClick,
    #[serde(default)]
    pub ctrl_click: crate::build::face_buttons::ModifierClick,
    #[serde(default)]
    pub ctrl_right_click: crate::build::CtrlRightClick,
    #[serde(default = "double_click_linked_default")]
    pub double_click_linked: bool,
}
fn double_click_linked_default() -> bool {
    true
}
fn surrounding_default() -> bool {
    true
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ruler {
    pub grid_unit: f32,
    pub grid_draw: f32,
    pub precision: u8,
    pub grid_angle: f32,
    pub grid_opacity: f32,
    pub grid_ticks: bool,
    #[serde(default)]
    pub grid_mode: u8,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    format: String,
    version: u8,
    pub layout: Layout,
    pub options: Options,
    pub ruler: Ruler,
    pub manipulation_mode: String,
}
impl Profile {
    pub fn capture(tool: &super::BuildTool, s: &super::BuildSettings) -> Self {
        use super::EditMode;
        Self {
            format: "aurora-construction-settings".into(),
            version: 1,
            layout: s.layout.clone(),
            options: Options {
                face: tool.edit_mode == EditMode::Face,
                stretch_textures: s.scale_stretch_textures,
                linked: s.edit_linked,
                scale_both: s.scale_uniform,
                root_axis: s.actual_root,
                snap: s.snap,
                object_snap: s.object_snap,
                snap_objects_preferred: s.snap_objects_preferred,
                highlight: s.show_highlight,
                probes: s.select_probes,
                owned_only: s.select_owned_only,
                movable_only: s.select_movable_only,
                surrounding: s.select_surrounding,
                land_group: s.rez_under_land_group,
                follow_edit: s.follow_edit_object,
                wheel_on_hover: s.wheel_on_hover,
                shift_click: s.shift_click,
                ctrl_click: s.ctrl_click,
                ctrl_right_click: s.ctrl_right_click,
                double_click_linked: s.double_click_linked,
            },
            ruler: Ruler {
                grid_unit: s.grid_resolution,
                grid_draw: s.grid_draw_size,
                precision: s.decimal_precision,
                grid_angle: s.rotation_step,
                grid_opacity: s.grid_opacity,
                grid_ticks: s.grid_sub_unit,
                grid_mode: s.grid_mode,
            },
            manipulation_mode: match tool.edit_mode {
                EditMode::Move => "move",
                EditMode::Rotate => "rotate",
                EditMode::Stretch => "stretch",
                EditMode::Face => "face",
                EditMode::Align => "align",
            }
            .into(),
        }
    }
    pub fn read(text: &str) -> Result<Self, String> {
        let invalid = || aurora_i18n::tr!("build-layout-invalid-notecard").to_owned();
        if text.len() > aurora_net::inventory::notecard::MAX_TEXT_BYTES {
            return Err(invalid());
        }
        let mut profile: Self = serde_json::from_str(text).map_err(|_| invalid())?;
        let r = &profile.ruler;
        let ranges = [
            (r.grid_unit, 0.01, 5.0),
            (r.grid_draw, 1.0, 50.0),
            (r.grid_angle, 0.01, 359.99),
            (r.grid_opacity, 0.0, 1.0),
        ];
        let mut seen = HashSet::new();
        if profile.format != "aurora-construction-settings"
            || profile.version != 1
            || profile.layout.order.is_empty()
            || profile.layout.locations.iter().any(|(item, panel)| !item.accepts(*panel))
            || profile.layout.footer_columns.keys().any(|item| !item.footer_column_item())
            || profile
                .layout
                .order
                .iter()
                .any(|item| !seen.insert(*item) || profile.layout.locations.get(item).is_none_or(|panel| !item.accepts(*panel)))
            || ranges
                .iter()
                .any(|(value, min, max)| !value.is_finite() || value < min || value > max)
            || profile.options.face != (profile.manipulation_mode == "face")
            || r.precision > 7
            || r.grid_mode > 2
            || !matches!(profile.manipulation_mode.as_str(), "move" | "rotate" | "stretch" | "face" | "align")
        {
            return Err(invalid());
        }
        profile.layout.normalize();
        Ok(profile)
    }
    pub fn apply(self, tool: &mut super::BuildTool, world: &mut crate::world::World, s: &mut super::BuildSettings) {
        use super::EditMode;
        s.layout = self.layout;
        s.scale_stretch_textures = self.options.stretch_textures;
        s.scale_uniform = self.options.scale_both;
        s.actual_root = self.options.root_axis;
        s.snap = self.options.snap;
        s.object_snap = self.options.object_snap && !self.options.snap;
        s.snap_objects_preferred = self.options.snap_objects_preferred;
        s.show_highlight = self.options.highlight;
        s.select_probes = self.options.probes;
        s.select_owned_only = self.options.owned_only;
        s.select_movable_only = self.options.movable_only;
        s.select_surrounding = self.options.surrounding;
        s.rez_under_land_group = self.options.land_group;
        s.follow_edit_object = self.options.follow_edit;
        s.wheel_on_hover = self.options.wheel_on_hover;
        s.shift_click = self.options.shift_click;
        s.ctrl_click = self.options.ctrl_click;
        s.ctrl_right_click = self.options.ctrl_right_click;
        s.double_click_linked = self.options.double_click_linked;
        s.grid_resolution = self.ruler.grid_unit;
        s.grid_draw_size = self.ruler.grid_draw;
        s.decimal_precision = self.ruler.precision;
        s.rotation_step = self.ruler.grid_angle;
        s.grid_opacity = self.ruler.grid_opacity;
        s.grid_sub_unit = self.ruler.grid_ticks;
        s.grid_mode = self.ruler.grid_mode;
        if s.grid_mode == 2 && tool.grid_objects.is_empty() {
            tool.use_selection_for_grid(world, s);
        }
        tool.set_edit_linked(world, s, self.options.linked);
        let mode = match self.manipulation_mode.as_str() {
            "rotate" => EditMode::Rotate,
            "stretch" => EditMode::Stretch,
            "face" => EditMode::Face,
            "align" => EditMode::Align,
            _ => EditMode::Move,
        };
        tool.set_edit_mode(world, s, mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn former_mode_group_expands_in_place_and_retains_its_destination() {
        for panel in [Panel::Footer, Panel::Settings, Panel::Hidden] {
            let mut old = Layout::default();
            old.order.retain(|item| !MANIPULATION_ITEMS.contains(item));
            old.locations.retain(|item, _| !MANIPULATION_ITEMS.contains(item));
            old.footer_columns.retain(|item, _| !MANIPULATION_ITEMS.contains(item));
            old.order.insert(3, Item::Manipulators);
            old.locations.insert(Item::Manipulators, panel);
            let other_columns = old.footer_columns.clone();
            old.normalize();
            assert_eq!(&old.order[3..7], &MANIPULATION_ITEMS);
            assert!(!old.locations.contains_key(&Item::Manipulators));
            assert!(!old.order.contains(&Item::Manipulators));
            for item in MANIPULATION_ITEMS {
                assert_eq!(old.locations[&item], panel);
            }
            for (item, column) in other_columns {
                assert_eq!(old.footer_columns[&item], column);
            }
            let expected = old.clone();
            old.normalize();
            assert_eq!(old, expected);
        }
    }

    #[test]
    fn section_order_upgrade_preserves_customization_after_restart() {
        let expected = Layout::default();
        let mut old = expected.clone();
        let materials = old.order.iter().position(|item| *item == Item::Materials).unwrap();
        let features = old.order.iter().position(|item| *item == Item::Features).unwrap();
        old.order.swap(materials, features);
        let mut value = serde_json::to_value(&old).unwrap();
        value.as_object_mut().unwrap().remove("sectionOrderVersion");
        let mut migrated: Layout = serde_json::from_value(value).unwrap();
        migrated.normalize();
        assert_eq!(migrated, expected, "Only the former default section order upgrades");
        migrated.place(Item::Materials, Panel::Navigation, Some(Item::Features), false);
        let custom = migrated.clone();
        let mut reopened: Layout = serde_json::from_str(&serde_json::to_string(&custom).unwrap()).unwrap();
        reopened.normalize();
        assert_eq!(reopened, custom, "A deliberate custom order survives normalization and restart");

        old.place(Item::Contents, Panel::Navigation, Some(Item::General), false);
        let mut value = serde_json::to_value(&old).unwrap();
        value.as_object_mut().unwrap().remove("sectionOrderVersion");
        let mut custom_old: Layout = serde_json::from_value(value).unwrap();
        custom_old.normalize();
        assert_eq!(custom_old.order, old.order, "Existing non-default arrangements are retained");
        assert_eq!(custom_old.locations, old.locations);
        assert_eq!(custom_old.footer_columns, old.footer_columns);
    }

    #[test]
    fn old_layouts_keep_their_column_choices_when_migrated() {
        let mut legacy = Layout {
            order: [
                Item::MoveMode,
                Item::RotateMode,
                Item::StretchMode,
                Item::AlignMode,
                Item::Face,
                Item::StretchTextures,
                Item::Linked,
                Item::ScaleBoth,
                Item::RootAxis,
            ]
            .into_iter()
            .chain(ITEMS)
            .collect(),
            ..Default::default()
        };
        for item in [Item::Snap, Item::Highlight, Item::Probes] {
            legacy.locations.insert(item, Panel::Settings);
        }
        legacy.normalize();
        let mut value = serde_json::to_value(legacy).unwrap();
        value.as_object_mut().unwrap().remove("footerColumns");
        let mut layout: Layout = serde_json::from_value(value).unwrap();
        layout.normalize();
        assert_eq!(layout.footer_columns[&Item::StretchTextures], FooterColumn::Left);
        assert_eq!(layout.footer_columns[&Item::ScaleBoth], FooterColumn::Left);
        assert_eq!(layout.footer_columns[&Item::RootAxis], FooterColumn::Right);
        assert_eq!(layout.footer_columns[&Item::Face], FooterColumn::Left);
        assert_eq!(layout.footer_columns[&Item::Linked], FooterColumn::Left);
        let restored: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(layout, restored);
    }

    #[test]
    fn tool_buttons_of_an_older_layout_move_to_the_bottom_bar_once() {
        let mut old = Layout::default();
        for item in ITEMS.into_iter().filter(|item| item.tool()) {
            old.locations.insert(item, Panel::Navigation);
        }
        old.locations.insert(Item::Land, Panel::Hidden);
        old.bars_version = 0;
        old.normalize();
        assert!(old.in_panel(Panel::Navigation).iter().all(|item| !item.tool()));
        assert_eq!(old.in_panel(Panel::Tools), [Item::Camera, Item::Grab, Item::Edit, Item::Create]);
        assert_eq!(old.locations[&Item::Land], Panel::Hidden);
        // once moved, a tool put back in the top bar stays there
        assert!(old.place(Item::Grab, Panel::Navigation, Some(Item::General), false));
        old.normalize();
        assert_eq!(old.in_panel(Panel::Navigation)[0], Item::Grab);
    }

    #[test]
    fn either_bar_takes_any_button_and_any_number_of_them() {
        let mut layout = Layout::default();
        for item in layout.in_bars() {
            assert!(layout.place(item, Panel::Tools, None, false), "{item:?}");
        }
        assert!(layout.in_panel(Panel::Navigation).is_empty());
        assert_eq!(layout.in_panel(Panel::Tools).len(), 10);
        assert!(layout.place(Item::Contents, Panel::Navigation, None, false));
        assert_eq!(layout.in_panel(Panel::Navigation), [Item::Contents]);
        assert_eq!(layout.in_bars().len(), 10);
        // options never go to a bar, buttons never to the options
        assert!(!layout.place(Item::Snap, Panel::Tools, None, false));
        assert!(!layout.place(Item::Edit, Panel::Footer, None, false));
    }

    #[test]
    fn bar_buttons_keep_their_display_and_icons_are_not_stored() {
        let mut layout = Layout::default();
        layout.set_display(Item::General, ButtonDisplay::Both);
        layout.set_display(Item::Land, ButtonDisplay::Label);
        layout.set_display(Item::Shape, ButtonDisplay::Icon);
        // only the bar buttons have a display
        layout.set_display(Item::Snap, ButtonDisplay::Label);
        let restored: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(restored.display(Item::General), ButtonDisplay::Both);
        assert_eq!(restored.display(Item::Land), ButtonDisplay::Label);
        assert_eq!(restored.display(Item::Shape), ButtonDisplay::Icon);
        assert_eq!(restored.button_display.len(), 2);
        layout.set_display(Item::General, ButtonDisplay::Icon);
        layout.set_display(Item::Land, ButtonDisplay::Icon);
        assert_eq!(layout, Layout::default());
        assert!(!serde_json::to_string(&layout).unwrap().contains("buttonDisplay"));
    }

    #[test]
    fn detachable_blocks_match_their_sections() {
        for block in Block::ALL {
            match block.item() {
                Some(item) => {
                    assert_eq!(Block::of_item(item), Some(block));
                    assert_eq!(block.tab().and_then(Block::of_tab), Some(block));
                }
                None => assert_eq!(block, Block::Transform),
            }
        }
        assert_eq!(Block::of_tab(0), Some(Block::General));
        assert_eq!(Block::of_tab(1), Some(Block::General));
        assert_eq!(Block::of_item(Item::Edit), None);
    }

    #[test]
    fn former_default_layout_moves_root_axis_above_highlight_once() {
        let mut former = Layout::default();
        former.swap_order(Item::RootAxis, Item::Highlight);
        former.footer_order_version = 0;
        let mut migrated = former.clone();
        migrated.normalize();
        assert_eq!(migrated, Layout::default());
        // an arranged layout keeps its order
        let mut arranged = former.clone();
        arranged.swap_order(Item::Snap, Item::Probes);
        let expected = arranged.order.clone();
        arranged.normalize();
        assert_eq!(arranged.order, expected);
        assert_eq!(arranged.footer_order_version, 1);
    }

    #[test]
    fn default_footer_uses_the_customized_two_column_profile() {
        let layout = Layout::default();
        assert_eq!(
            layout.footer_rows(),
            vec![
                FooterRow::Cells([Some(Item::MoveMode), Some(Item::ScaleBoth)]),
                FooterRow::Cells([Some(Item::RotateMode), Some(Item::StretchTextures)]),
                FooterRow::Cells([Some(Item::StretchMode), Some(Item::Snap)]),
                FooterRow::Cells([Some(Item::Face), Some(Item::RootAxis)]),
                FooterRow::Cells([Some(Item::AlignMode), Some(Item::Highlight)]),
                FooterRow::Cells([Some(Item::Linked), Some(Item::Probes)]),
                FooterRow::Full(Item::Actions),
            ]
        );
        assert_eq!(
            layout.in_panel(Panel::Settings),
            vec![
                Item::OwnedOnly,
                Item::MovableOnly,
                Item::Surrounding,
                Item::LandGroup,
                Item::FollowEdit,
                Item::WheelOnHover,
            ]
        );
    }

    #[test]
    fn changing_one_column_keeps_the_other_columns_item_rows() {
        let mut base = Layout::default();
        for panel in base.locations.values_mut() {
            *panel = Panel::Hidden;
        }
        let left = [Item::MoveMode, Item::Face, Item::Linked];
        let right = [Item::RotateMode, Item::RootAxis, Item::ScaleBoth];
        base.order = left.into_iter().chain(right).chain([Item::Actions]).chain(ITEMS).collect();
        for (items, column) in [(left, FooterColumn::Left), (right, FooterColumn::Right)] {
            for item in items {
                base.locations.insert(item, Panel::Footer);
                base.footer_columns.insert(item, column);
            }
        }
        base.locations.insert(Item::Actions, Panel::Footer);
        base.normalize();
        assert_eq!(
            base.footer_rows(),
            vec![
                FooterRow::Cells([Some(left[0]), Some(right[0])]),
                FooterRow::Cells([Some(left[1]), Some(right[1])]),
                FooterRow::Cells([Some(left[2]), Some(right[2])]),
                FooterRow::Full(Item::Actions),
            ]
        );
        let positions = |layout: &Layout, column: FooterColumn| {
            layout
                .footer_rows()
                .into_iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    if let FooterRow::Cells(cells) = row {
                        cells[column.index()].map(|item| (index, item))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        for (column, opposite, items) in [
            (FooterColumn::Left, FooterColumn::Right, left),
            (FooterColumn::Right, FooterColumn::Left, right),
        ] {
            for (target, after) in [(items[0], false), (items[1], true), (Item::Actions, false)] {
                let mut layout = base.clone();
                let expected = positions(&layout, opposite);
                assert!(layout.place_in_footer(Item::Snap, column, Some(target), after));
                assert_eq!(positions(&layout, opposite), expected);
                assert!(layout.place(Item::Snap, Panel::Hidden, None, false));
                assert_eq!(positions(&layout, opposite), expected);
                assert!(layout.place_in_footer(items[2], column, Some(items[0]), false));
                assert_eq!(positions(&layout, opposite), expected);
            }
        }
    }

    #[test]
    fn moving_an_option_preserves_other_columns_and_right_only_rows() {
        let mut layout = Layout::default();
        let last_cells = |layout: &Layout| layout.footer_rows().into_iter().rfind(|row| matches!(row, FooterRow::Cells(_)));
        let columns = layout.footer_columns.clone();
        assert!(layout.place_in_footer(Item::ScaleBoth, FooterColumn::Right, None, false));
        assert_eq!(layout.footer_columns[&Item::RootAxis], columns[&Item::RootAxis]);
        assert_eq!(layout.footer_columns[&Item::StretchTextures], columns[&Item::StretchTextures]);
        assert_eq!(
            last_cells(&layout),
            Some(FooterRow::Cells([Some(Item::Linked), Some(Item::ScaleBoth)]))
        );
        assert!(layout.place(Item::ScaleBoth, Panel::Hidden, None, false));
        assert!(layout.place(Item::ScaleBoth, Panel::Footer, None, false));
        assert_eq!(layout.footer_columns[&Item::ScaleBoth], FooterColumn::Right);
        assert!(layout.place_in_footer(Item::ScaleBoth, FooterColumn::Left, Some(Item::ScaleBoth), false));
        assert_eq!(last_cells(&layout), Some(FooterRow::Cells([Some(Item::ScaleBoth), None])));
        for item in [Item::Face, Item::Linked, Item::Snap] {
            assert!(layout.place_in_footer(item, FooterColumn::Right, None, false));
            assert!(
                layout
                    .footer_rows()
                    .iter()
                    .any(|row| matches!(row, FooterRow::Cells([_, Some(right)]) if *right == item))
            );
            assert_eq!(layout.footer_columns[&item], FooterColumn::Right);
            let saved = serde_json::to_string(&layout).unwrap();
            let mut restored: Layout = serde_json::from_str(&saved).unwrap();
            restored.normalize();
            assert_eq!(layout, restored);
            assert!(layout.place_in_footer(item, FooterColumn::Left, Some(item), false));
            assert_eq!(last_cells(&layout), Some(FooterRow::Cells([Some(item), None])));
        }
        assert!(layout.place_in_footer(Item::Actions, FooterColumn::Right, None, false));
        assert_eq!(layout.footer_rows().last(), Some(&FooterRow::Full(Item::Actions)));
        assert!(!layout.footer_columns.contains_key(&Item::Actions));
        assert!(!layout.place_in_footer(Item::General, FooterColumn::Left, None, false));
    }

    #[test]
    fn action_bar_only_brackets_the_complete_footer_columns() {
        let mut layout = Layout::default();
        let columns: Vec<_> = layout
            .in_panel(Panel::Footer)
            .into_iter()
            .filter(|item| item.footer_column_item())
            .collect();
        let original = layout.clone();
        for target in &columns[1..columns.len() - 1] {
            for after in [false, true] {
                assert!(!layout.place(Item::Actions, Panel::Footer, Some(*target), after));
                assert_eq!(layout, original);
            }
        }
        for top in [true, false] {
            let current: Vec<_> = layout
                .in_panel(Panel::Footer)
                .into_iter()
                .filter(|item| item.footer_column_item())
                .collect();
            let target = if top { current[0] } else { current[current.len() - 1] };
            assert!(layout.place(Item::Actions, Panel::Footer, Some(target), !top));
            assert_eq!(layout.footer_actions_top(), top);
            assert!(layout.place_in_footer(Item::OwnedOnly, FooterColumn::Left, None, false));
            assert!(layout.place_in_footer(Item::MovableOnly, FooterColumn::Right, Some(Item::Actions), top));
            let rows = layout.footer_rows();
            assert_eq!(if top { rows.first() } else { rows.last() }, Some(&FooterRow::Full(Item::Actions)));
            assert_eq!(rows.iter().filter(|row| matches!(row, FooterRow::Full(_))).count(), 1);
        }
        let expected_columns = layout.footer_columns.clone();
        layout.order.retain(|item| *item != Item::Actions);
        let middle = layout.order.iter().position(|item| *item == columns[3]).unwrap();
        layout.order.insert(middle, Item::Actions);
        layout.normalize();
        assert_eq!(layout.footer_rows().last(), Some(&FooterRow::Full(Item::Actions)));
        assert_eq!(layout.footer_columns, expected_columns);
        let restored = layout.clone();
        layout.normalize();
        assert_eq!(layout, restored);
    }
}
