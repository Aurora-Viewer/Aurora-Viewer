//! "Pathfinding view / test": the Legacy console's layout (viewer and
//! simulator status, then the View and Test path tabs), with the path test
//! answered by the region through the bridge. Port of
//! LLFloaterPathfindingConsole and LLPathfindingPathTool (the Ctrl / Shift
//! points, the character width and type, getPathStatus), indra/newview,
//! floater_pathfinding_console.xml, originally LGPL 2.1.
//!
//! No Havok: the navmesh itself cannot be shown (its source is a Havok blob
//! only Linden's licensed library reads), so the viewer status says so and
//! the Navmesh and walkability controls stay disabled. The View tab's other
//! overlays are drawn by Aurora from the linksets' own settings.
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind, Floater};
use aurora_net::bridge::query::{PATH_RADIUS, Query};
use egui::{RichText, Vec2};
use glam::Vec3;

pub const WINDOW: &str = "pathfinding_console";

/// The console's character width slider (floater_pathfinding_console.xml).
pub const WIDTH_RANGE: std::ops::RangeInclusive<f32> = 0.2..=2.0;
pub const DEFAULT_WIDTH: f32 = 1.0;

/// The Character type dropdown, in the Legacy order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CharacterType {
    #[default]
    None,
    A,
    B,
    C,
    D,
}

impl CharacterType {
    pub const ALL: [CharacterType; 5] = [Self::None, Self::A, Self::B, Self::C, Self::D];

    /// The LSL CHARACTER_TYPE value (CHARACTER_TYPE_A is 0 … D 3, NONE 4);
    /// the Legacy dropdown numbers them None 0, A 1 … D 4.
    pub fn lsl(self) -> i32 {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::C => 2,
            Self::D => 3,
            Self::None => 4,
        }
    }

    fn label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::None => "pathfinding-type-none",
            Self::A => "pathfinding-type-a",
            Self::B => "pathfinding-type-b",
            Self::C => "pathfinding-type-c",
            Self::D => "pathfinding-type-d",
        })
    }
}

/// What the region answered for the chosen points.
#[derive(Debug, Clone, PartialEq)]
pub enum PathResult {
    /// The waypoints, in render coordinates.
    Found(Vec<Vec3>),
    /// llGetStaticPath's PU_FAILURE_* code.
    NotFound(i32),
    /// The bridge could not answer.
    Error(String),
}

/// The Test path tab (LLPathfindingPathTool's final points).
#[derive(Debug, Clone, PartialEq)]
pub struct PathTest {
    pub start: Option<Vec3>,
    pub end: Option<Vec3>,
    pub width: f32,
    pub character: CharacterType,
    pub result: Option<PathResult>,
    /// Asked of the region, no answer yet.
    pub asking: bool,
}

impl Default for PathTest {
    fn default() -> Self {
        Self {
            start: None,
            end: None,
            width: DEFAULT_WIDTH,
            character: CharacterType::None,
            result: None,
            asking: false,
        }
    }
}

/// Why the points cannot be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unaskable {
    /// A point is outside the agent's region (llGetStaticPath works in its
    /// script's region only).
    OutsideRegion,
}

impl PathTest {
    /// llGetStaticPath takes a radius; the console's width is a diameter.
    /// Only the narrowest width is raised to LSL's 0.125 m minimum.
    pub fn radius(&self) -> f32 {
        (self.width / 2.0).clamp(*PATH_RADIUS.start(), *PATH_RADIUS.end())
    }

    /// Ctrl-click (start, setFinalA) or Shift-click (end, setFinalB).
    pub fn set_point(&mut self, end: bool, point: Vec3) {
        if end {
            self.end = Some(point);
        } else {
            self.start = Some(point);
        }
        self.result = None;
    }

    /// Clear path.
    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
        self.result = None;
        self.asking = false;
    }

    /// The query for the chosen points, None until both are chosen. Points
    /// are render coordinates, the agent's region starting at 0
    /// (`region_size` metres wide).
    pub fn query(&self, region_size: (f32, f32)) -> Option<Result<Query, Unaskable>> {
        let (start, end) = (self.start?, self.end?);
        let inside = |p: Vec3| (0.0..region_size.0).contains(&p.x) && (0.0..region_size.1).contains(&p.y);
        if !inside(start) || !inside(end) {
            return Some(Err(Unaskable::OutsideRegion));
        }
        Some(Ok(Query::StaticPath {
            start: start.to_array(),
            end: end.to_array(),
            radius: self.radius(),
            character: self.character.lsl(),
        }))
    }

    /// The status line and, under it, the reason Aurora can add
    /// (getPathStatus and the console's pathing_* strings).
    pub fn status(&self, enabled: bool, bridge_ready: bool, region_size: (f32, f32)) -> (String, Option<String>) {
        let text = |key| aurora_i18n::text(key).to_owned();
        if !enabled {
            return (text("pathfinding-path-not-enabled"), None);
        }
        match (self.start, self.end) {
            (None, None) => return (text("pathfinding-path-choose-both"), None),
            (None, Some(_)) => return (text("pathfinding-path-choose-start"), None),
            (Some(_), None) => return (text("pathfinding-path-choose-end"), None),
            _ => {}
        }
        if let Some(Err(Unaskable::OutsideRegion)) = self.query(region_size) {
            return (text("pathfinding-path-invalid"), Some(text("pathfinding-path-outside-region")));
        }
        match &self.result {
            Some(PathResult::Found(_)) => (text("pathfinding-path-valid"), None),
            Some(PathResult::NotFound(code)) => (text("pathfinding-path-invalid"), Some(failure_reason(*code))),
            Some(PathResult::Error(error)) => (text("pathfinding-path-error"), Some(error.clone())),
            None if !bridge_ready => (text("pathfinding-path-asking"), Some(text("pathfinding-path-no-bridge"))),
            None => (text("pathfinding-path-asking"), None),
        }
    }
}

/// The PU_FAILURE_* codes llGetStaticPath can answer, in words.
pub fn failure_reason(code: i32) -> String {
    aurora_i18n::text(match code {
        2 => "pathfinding-reason-start",
        3 => "pathfinding-reason-goal",
        4 => "pathfinding-reason-unreachable",
        9 => "pathfinding-reason-no-navmesh",
        _ => "pathfinding-reason-other",
    })
    .to_owned()
}

/// The View tab's choices (the console's show_* checkboxes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewOptions {
    pub world: bool,
    pub movables_only: bool,
    pub walkables: bool,
    pub material_volumes: bool,
    pub static_obstacles: bool,
    pub exclusion_volumes: bool,
    pub water_plane: bool,
    pub xray: bool,
}

impl Default for ViewOptions {
    /// The Legacy defaults: World on, everything else off.
    fn default() -> Self {
        Self {
            world: true,
            movables_only: false,
            walkables: false,
            material_volumes: false,
            static_obstacles: false,
            exclusion_volumes: false,
            water_plane: false,
            xray: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    View,
    Test,
}

/// The simulator status block (navmesh_simulator_status_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatorStatus {
    NotEnabled,
    Unknown,
    Pending,
    Building,
    Complete,
}

#[derive(Default)]
pub struct Console {
    pub open: bool,
    pub tab: Tab,
    pub view: ViewOptions,
    pub test: PathTest,
}

/// What the console asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleAction {
    /// The width or the type changed: ask the path again.
    Recompute,
    ClearPath,
}

impl Console {
    /// The Test path tab is showing: Ctrl / Shift clicks set the points
    /// (switchIntoTestPathMode).
    pub fn testing(&self) -> bool {
        self.open && self.tab == Tab::Test
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        simulator: SimulatorStatus,
        bridge_ready: bool,
        region_size: (f32, f32),
    ) -> Option<ConsoleAction> {
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut action = None;
        let mut window = Floater::new(
            WINDOW,
            aurora_i18n::tr!("pathfinding-console-title"),
            egui::pos2(60.0, 140.0),
            Vec2::new(340.0, 640.0),
        );
        window.min_size = Vec2::new(320.0, 560.0);
        // the Legacy console's fixed width: only its height changes
        window.height_only = true;
        window.show(ctx, p, &mut open, |ui| {
            let enabled = simulator != SimulatorStatus::NotEnabled;
            // Viewer status: what the Legacy Viewer reports about its navmesh
            // download; Aurora never downloads it (no Havok)
            ui.label(RichText::new(aurora_i18n::tr!("pathfinding-viewer-status")).strong().color(p.ink));
            ui.label(RichText::new(aurora_i18n::tr!("pathfinding-console-no-havok")).color(p.warn));
            ui.add_space(8.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("pathfinding-simulator-status"))
                    .strong()
                    .color(p.ink),
            );
            let (status, color) = match simulator {
                SimulatorStatus::NotEnabled => (aurora_i18n::tr!("pathfinding-not-enabled"), p.warn),
                SimulatorStatus::Unknown => ("", p.ink),
                SimulatorStatus::Pending => (aurora_i18n::tr!("pathfinding-simulator-pending"), p.warn),
                SimulatorStatus::Building => (aurora_i18n::tr!("pathfinding-simulator-building"), p.warn),
                SimulatorStatus::Complete => (aurora_i18n::tr!("pathfinding-simulator-complete"), p.success),
            };
            ui.label(RichText::new(status).color(color));
            ui.add_space(10.0);
            let mut tab = match self.tab {
                Tab::View => 0,
                Tab::Test => 1,
            };
            let labels = [
                (aurora_i18n::text("pathfinding-tab-view"), true),
                (aurora_i18n::text("pathfinding-tab-test"), true),
            ];
            widgets::stretched_tabs(ui, p, &mut tab, &labels);
            self.tab = if tab == 1 { Tab::Test } else { Tab::View };
            ui.add_space(6.0);
            match self.tab {
                Tab::View => view_tab(ui, p, &mut self.view, enabled),
                Tab::Test => action = self.test_tab(ui, p, enabled, bridge_ready, region_size),
            }
        });
        if !open {
            self.open = false;
            self.test.clear();
            action = Some(ConsoleAction::ClearPath);
        }
        action
    }

    fn test_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        enabled: bool,
        bridge_ready: bool,
        region_size: (f32, f32),
    ) -> Option<ConsoleAction> {
        let mut action = None;
        ui.label(aurora_i18n::tr!("pathfinding-ctrl-click"));
        ui.label(aurora_i18n::tr!("pathfinding-shift-click"));
        widgets::divider(ui, p);
        ui.add_enabled_ui(enabled, |ui| {
            ui.label(aurora_i18n::tr!("pathfinding-character-width"));
            ui.horizontal(|ui| {
                let slider = egui::Slider::new(&mut self.test.width, WIDTH_RANGE)
                    .step_by(0.1)
                    .fixed_decimals(1)
                    .suffix(aurora_i18n::text("pathfinding-width-unit"));
                if ui.add(slider).changed() {
                    action = Some(ConsoleAction::Recompute);
                }
            });
            ui.add_space(6.0);
            ui.label(aurora_i18n::tr!("pathfinding-character-type"));
            egui::ComboBox::from_id_salt("pathfinding_character_type")
                .selected_text(self.test.character.label())
                .width(180.0)
                .show_ui(ui, |ui| {
                    for character in CharacterType::ALL {
                        if ui
                            .selectable_value(&mut self.test.character, character, character.label())
                            .changed()
                        {
                            action = Some(ConsoleAction::Recompute);
                        }
                    }
                });
        });
        ui.add_space(8.0);
        let (status, reason) = self.test.status(enabled, bridge_ready, region_size);
        let color = match self.test.result {
            Some(PathResult::Found(_)) => p.success,
            Some(_) => p.danger,
            None => p.warn,
        };
        ui.label(RichText::new(status).color(color));
        if let Some(reason) = reason {
            ui.label(RichText::new(reason).size(12.0).color(p.muted));
        }
        ui.add_space(8.0);
        let any = self.test.start.is_some() || self.test.end.is_some();
        if widgets::action_button_if(
            ui,
            p,
            "x",
            aurora_i18n::tr!("pathfinding-clear-path"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
            any,
        )
        .clicked()
        {
            self.test.clear();
            action = Some(ConsoleAction::ClearPath);
        }
        action
    }
}

/// The View tab (onShow*Set). Navmesh and its walkability map need the
/// Havok navmesh: they stay disabled. The other overlays are Aurora's.
fn view_tab(ui: &mut egui::Ui, p: &Palette, view: &mut ViewOptions, enabled: bool) {
    ui.label(aurora_i18n::tr!("pathfinding-show"));
    ui.add_enabled_ui(enabled, |ui| {
        widgets::checkbox(ui, p, &mut view.world, aurora_i18n::tr!("pathfinding-show-world"));
        ui.indent("pathfinding_movables", |ui| {
            ui.add_enabled_ui(view.world, |ui| {
                widgets::checkbox(ui, p, &mut view.movables_only, aurora_i18n::tr!("pathfinding-show-movables"));
            });
        });
        if !view.world {
            view.movables_only = false;
        }
        let havok = aurora_i18n::tr!("pathfinding-needs-havok");
        ui.add_enabled_ui(false, |ui| {
            let mut off = false;
            widgets::checkbox(ui, p, &mut off, aurora_i18n::tr!("pathfinding-show-navmesh"));
            ui.indent("pathfinding_walkability", |ui| {
                ui.label(aurora_i18n::tr!("pathfinding-show-walkability"));
                egui::ComboBox::from_id_salt("pathfinding_walkability")
                    .selected_text(aurora_i18n::tr!("pathfinding-walkability-none"))
                    .width(180.0)
                    .show_ui(ui, |_| {});
            });
        })
        .response
        .on_disabled_hover_text(havok);
        widgets::checkbox(ui, p, &mut view.walkables, aurora_i18n::tr!("pathfinding-show-walkables"));
        widgets::checkbox(ui, p, &mut view.material_volumes, aurora_i18n::tr!("pathfinding-show-material"));
        widgets::checkbox(ui, p, &mut view.static_obstacles, aurora_i18n::tr!("pathfinding-show-static"));
        widgets::checkbox(ui, p, &mut view.exclusion_volumes, aurora_i18n::tr!("pathfinding-show-exclusion"));
        widgets::checkbox(ui, p, &mut view.water_plane, aurora_i18n::tr!("pathfinding-show-water"));
        widgets::checkbox(ui, p, &mut view.xray, aurora_i18n::tr!("pathfinding-show-xray"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGION: (f32, f32) = (256.0, 256.0);

    #[test]
    fn the_width_is_a_diameter_and_types_map_to_lsl() {
        let mut test = PathTest::default();
        assert_eq!(test.radius(), 0.5);
        test.width = 0.2;
        assert_eq!(test.radius(), 0.125, "only the narrowest width is raised");
        test.width = 2.0;
        assert_eq!(test.radius(), 1.0);
        assert_eq!(
            CharacterType::ALL.map(CharacterType::lsl),
            [4, 0, 1, 2, 3],
            "None, A, B, C, D in the Legacy order"
        );
    }

    #[test]
    fn the_status_follows_get_path_status() {
        let text = |key| aurora_i18n::text(key).to_owned();
        let mut test = PathTest::default();
        assert_eq!(test.status(false, true, REGION).0, text("pathfinding-path-not-enabled"));
        assert_eq!(test.status(true, true, REGION).0, text("pathfinding-path-choose-both"));
        test.set_point(true, Vec3::new(50.0, 50.0, 20.0));
        assert_eq!(test.status(true, true, REGION).0, text("pathfinding-path-choose-start"));
        test.set_point(false, Vec3::new(10.0, 10.0, 20.0));
        assert_eq!(test.status(true, true, REGION), (text("pathfinding-path-asking"), None));
        assert_eq!(test.status(true, false, REGION).1, Some(text("pathfinding-path-no-bridge")));
        test.result = Some(PathResult::Found(vec![]));
        assert_eq!(test.status(true, true, REGION).0, text("pathfinding-path-valid"));
        test.result = Some(PathResult::NotFound(2));
        assert_eq!(
            test.status(true, true, REGION),
            (text("pathfinding-path-invalid"), Some(text("pathfinding-reason-start")))
        );
        test.set_point(true, Vec3::new(300.0, 50.0, 20.0));
        assert!(test.result.is_none(), "a new point forgets the answer");
        assert_eq!(test.status(true, true, REGION).1, Some(text("pathfinding-path-outside-region")));
        test.clear();
        assert_eq!(test.status(true, true, REGION).0, text("pathfinding-path-choose-both"));
    }

    #[test]
    fn the_query_carries_the_points_radius_and_type() {
        let mut test = PathTest {
            character: CharacterType::B,
            ..Default::default()
        };
        assert!(test.query(REGION).is_none());
        test.set_point(false, Vec3::new(10.0, 20.0, 21.0));
        test.set_point(true, Vec3::new(30.0, 40.0, 22.0));
        assert_eq!(
            test.query(REGION),
            Some(Ok(Query::StaticPath {
                start: [10.0, 20.0, 21.0],
                end: [30.0, 40.0, 22.0],
                radius: 0.5,
                character: 1,
            }))
        );
        test.set_point(true, Vec3::new(-1.0, 40.0, 22.0));
        assert_eq!(test.query(REGION), Some(Err(Unaskable::OutsideRegion)));
        assert!(failure_reason(1_000_000).contains(aurora_i18n::text("pathfinding-reason-other")));
    }
}
