//! The Pathfinding view / test console in the app: opening it, the Ctrl /
//! Shift clicks that set the path's ends (LLPathfindingPathTool::handleMouseDown),
//! the path asked of the region through the bridge, and the path drawn in the
//! world (the Legacy Viewer's renderPath / renderPathBookend, done by Havok;
//! Aurora draws them itself, scaled by the character width).
use super::*;
use crate::ui::pathfinding::capsule::Capsule;
use crate::ui::pathfinding::console::{ConsoleAction, PathResult, SimulatorStatus};
use crate::ui::pathfinding::rebake::Mode;
use crate::world::bridge::path::Outcome;
use uuid::Uuid;

/// The widest the path's ribbon is drawn, in pixels, however close.
const MAX_RIBBON: f32 = 24.0;

impl App {
    /// Build › Pathfinding › View / test.
    pub(super) fn open_pathfinding_console(&mut self) {
        self.pathfinding.console.open = true;
        ui::widgets::bring_floater_front(&self.egui_ctx, ui::pathfinding::console::WINDOW);
    }

    /// The agent's region size: the path test's points must be inside it.
    fn region_size(&self) -> (f32, f32) {
        self.world
            .main()
            .map_or((256.0, 256.0), |r| (r.heightmap.size_x as f32, r.heightmap.size_y as f32))
    }

    /// The simulator status block, from the rebake state.
    fn simulator_status(&self) -> SimulatorStatus {
        let pathfinding = self.world.main_has_cap("NavMeshGenerationStatus") || self.pathfinding_demo.region();
        if !pathfinding {
            return SimulatorStatus::NotEnabled;
        }
        match self.pathfinding.rebake.mode() {
            Mode::Available => SimulatorStatus::Pending,
            Mode::RequestSent | Mode::InProgress => SimulatorStatus::Building,
            Mode::NotAvailable => SimulatorStatus::Complete,
            Mode::Default => SimulatorStatus::Unknown,
        }
    }

    /// The console, once a frame.
    pub(super) fn pathfinding_console(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let simulator = self.simulator_status();
        let region_size = self.region_size();
        let ready = self.path_bridge_ready;
        match self.pathfinding.console.show(ctx, p, simulator, ready, region_size) {
            Some(ConsoleAction::Recompute) => self.ask_path(),
            Some(ConsoleAction::ClearPath) => self.path_queue.cancel(),
            None => {}
        }
    }

    /// Ask the region for the path between the chosen points.
    pub(super) fn ask_path(&mut self) {
        let size = self.region_size();
        let test = &mut self.pathfinding.console.test;
        test.result = None;
        match test.query(size) {
            Some(Ok(query)) => {
                test.asking = true;
                self.path_queue.ask(query);
            }
            _ => {
                test.asking = false;
                self.path_queue.cancel();
            }
        }
    }

    /// The bridge side, from the bridge loop: the agent's region and
    /// session, the request to send, a timeout.
    pub(super) fn pathfinding_bridge(&mut self, endpoint: Option<&aurora_net::bridge::Endpoint>) {
        self.path_bridge_ready = endpoint.is_some();
        if self.path_queue.sync(self.world.main_region, endpoint) {
            // another region: the points are no longer this region's
            self.pathfinding.console.test.clear();
        }
        let (command, outcome) = self.path_queue.update(endpoint, Instant::now());
        if let Some(command) = command {
            self.send(command);
        }
        if let Some(outcome) = outcome {
            self.path_outcome(outcome);
        }
    }

    /// A bridge answer: true when it was the path test's.
    pub(super) fn path_reply(&mut self, request: Uuid, result: &Result<aurora_net::bridge::query::Reply, String>) -> bool {
        match self.path_queue.reply(request, result.clone()) {
            Some(outcome) => {
                self.path_outcome(outcome);
                true
            }
            None => false,
        }
    }

    fn path_outcome(&mut self, outcome: Outcome) {
        let test = &mut self.pathfinding.console.test;
        if self.path_queue.busy() {
            // a newer request is on its way: its answer is the one to show
            return;
        }
        test.asking = false;
        test.result = Some(match outcome {
            Outcome::Path(path) if path.status == 0 => PathResult::Found(
                path.points
                    .iter()
                    .map(|p| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
                    .collect(),
            ),
            Outcome::Path(path) => PathResult::NotFound(path.status),
            Outcome::Failed(error) => PathResult::Error(error),
        });
    }

    /// LLPathfindingPathTool::handleMouseDown with Ctrl (start) or Shift
    /// (end) while the Test path tab shows; None: not a path click.
    pub(super) fn path_click_end(&self) -> Option<bool> {
        if !self.pathfinding.console.testing() {
            return None;
        }
        if self.ctrl {
            Some(false)
        } else if self.shift {
            Some(true)
        } else {
            None
        }
    }

    /// The point under the cursor of a path click (the visible surface), as
    /// one end of the path; the path is asked again.
    pub(super) fn path_point(&mut self, end: bool) {
        let Some(picked) = self.click_pick() else { return };
        let Some(point) = picked.visible_point() else { return };
        self.pathfinding.console.test.set_point(end, point);
        self.ask_path();
    }

    /// The path in the world: a ribbon as wide as half the character, and
    /// a capsule of the character's width at each end. Drawn while the
    /// console is open (the Legacy Viewer draws it while its tool is on).
    pub(super) fn draw_path(&self, ctx: &egui::Context, p: &crate::theme::Palette, proj: &ui::hud::Projector) {
        let console = &self.pathfinding.console;
        if !console.open {
            return;
        }
        let test = &console.test;
        let width = test.width;
        let painter = ui::hud::world_painter(ctx);
        // the screen size of `width` metres standing at a point
        let pixels = |at: Vec3| {
            let a = proj.project(at)?.0;
            let b = proj.project(at + Vec3::Z * width)?.0;
            Some((a - b).length())
        };
        if let Some(PathResult::Found(points)) = &test.result {
            for pair in points.windows(2) {
                let (a, b) = (pair[0] + Vec3::Z * 0.1, pair[1] + Vec3::Z * 0.1);
                // pieces of about a metre, so the ribbon narrows with distance
                let pieces = (a.distance(b).ceil() as usize).clamp(1, 64);
                for i in 0..pieces {
                    let (from, to) = (a.lerp(b, i as f32 / pieces as f32), a.lerp(b, (i + 1) as f32 / pieces as f32));
                    let Some(segment) =
                        ui::minimap::beacon_segment(proj.view_proj * from.extend(1.0), proj.view_proj * to.extend(1.0), proj)
                    else {
                        continue;
                    };
                    let thickness = pixels((from + to) * 0.5).map_or(2.0, |px| (px * 0.5).clamp(1.5, MAX_RIBBON));
                    // hidden by what stands in front of it, as the Legacy path is
                    aurora_render::world_line_at(ctx, from, to);
                    // theme-guard: content — the path's colour, the Legacy "shown in orange"
                    painter.line_segment(segment, egui::Stroke::new(thickness, p.amber));
                }
            }
            aurora_render::world_label_at(ctx, None);
        }
        // the bookends: a capsule of the character's width, twice as tall
        for (point, color) in [(test.start, p.success), (test.end, p.danger)] {
            let Some(point) = point else { continue };
            let capsule = Capsule {
                center: point + Vec3::Z * width,
                rotation: glam::Quat::IDENTITY,
                length: width * 2.0,
                radius: width / 2.0,
                horizontal: false,
            };
            ui::pathfinding::capsule::draw_outline(ctx, &painter, proj, &capsule, color);
        }
    }
}
