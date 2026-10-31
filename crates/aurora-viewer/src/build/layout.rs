//! Personal construction layout and portable settings. Object properties and
//! transforms never belong to a layout or its notecard.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Item {
    #[serde(rename = "manipulators")]
    Manipulators,
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
    #[serde(rename = "actions")]
    Actions,
    #[serde(rename = "nav-section-general")]
    General,
    #[serde(rename = "nav-section-shape")]
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

pub const ITEMS: [Item; 25] = [
    Item::Manipulators,
    Item::Face,
    Item::StretchTextures,
    Item::Linked,
    Item::ScaleBoth,
    Item::RootAxis,
    Item::Snap,
    Item::Highlight,
    Item::Probes,
    Item::OwnedOnly,
    Item::MovableOnly,
    Item::Surrounding,
    Item::LandGroup,
    Item::FollowEdit,
    Item::Actions,
    Item::General,
    Item::Shape,
    Item::Materials,
    Item::Features,
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
    Navigation,
}

impl Item {
    pub fn navigation(self) -> bool {
        self >= Self::General
    }
    pub fn accepts(self, panel: Panel) -> bool {
        panel == Panel::Hidden
            || if self.navigation() {
                panel == Panel::Navigation
            } else {
                matches!(panel, Panel::Footer | Panel::Settings)
            }
    }
    pub fn default_panel(self) -> Panel {
        if self.navigation() {
            Panel::Navigation
        } else if matches!(
            self,
            Self::Snap
                | Self::Highlight
                | Self::Probes
                | Self::OwnedOnly
                | Self::MovableOnly
                | Self::Surrounding
                | Self::LandGroup
                | Self::FollowEdit
        ) {
            Panel::Settings
        } else {
            Panel::Footer
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub order: Vec<Item>,
    pub locations: BTreeMap<Item, Panel>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            order: ITEMS.to_vec(),
            locations: ITEMS.into_iter().map(|item| (item, item.default_panel())).collect(),
        }
    }
}

impl Layout {
    /// Import old preferences without duplicates or inaccessible destinations.
    pub fn normalize(&mut self) {
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
    }
    pub fn in_panel(&self, panel: Panel) -> Vec<Item> {
        self.order
            .iter()
            .copied()
            .filter(|item| self.locations.get(item) == Some(&panel))
            .collect()
    }
    pub fn place(&mut self, item: Item, panel: Panel, target: Option<Item>, after: bool) -> bool {
        if !item.accepts(panel) || target == Some(item) {
            return false;
        }
        if target.is_some_and(|target| self.locations.get(&target) != Some(&panel)) {
            return false;
        }
        self.order.retain(|key| *key != item);
        let index = target
            .and_then(|target| self.order.iter().position(|key| *key == target))
            .map(|index| index + usize::from(after))
            .unwrap_or(self.order.len());
        self.order.insert(index, item);
        self.locations.insert(item, panel);
        true
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
                highlight: s.show_highlight,
                probes: s.select_probes,
                owned_only: s.select_owned_only,
                movable_only: s.select_movable_only,
                surrounding: s.select_surrounding,
                land_group: s.rez_under_land_group,
                follow_edit: s.follow_edit_object,
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
        s.show_highlight = self.options.highlight;
        s.select_probes = self.options.probes;
        s.select_owned_only = self.options.owned_only;
        s.select_movable_only = self.options.movable_only;
        s.select_surrounding = self.options.surrounding;
        s.rez_under_land_group = self.options.land_group;
        s.follow_edit_object = self.options.follow_edit;
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
