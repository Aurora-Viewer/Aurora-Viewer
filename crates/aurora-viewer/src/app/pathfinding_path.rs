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

    /// While Ctrl or Shift is held over the world in the Test path tab, the
    /// point a click would set (LLPathfindingPathTool::handleHover shows the
    /// end under the cursor before the click).
    pub(super) fn path_hover_point(&self, renderer: &mut aurora_render::Renderer, over_ui: bool) -> Option<(bool, Vec3)> {
        if over_ui || !matches!(self.screen, Screen::World) || self.mouse_mode != MouseMode::None {
            return None;
        }
        let end = self.path_click_end()?;
        renderer.hover_pick(self.cursor_pos.0, self.cursor_pos.1).map(|point| (end, point))
    }

    /// The path in the world: a flat ribbon as wide as the character
    /// (renderPath), faint where the scene hides it (the navmesh lies under
    /// the floors one sees), and a capsule of the character's width at each
    /// end (renderPathBookend). Drawn while the console is open (the Legacy
    /// Viewer draws it while its tool is on).
    pub(super) fn draw_path(&self, ctx: &egui::Context, p: &crate::theme::Palette, proj: &ui::hud::Projector) {
        let console = &self.pathfinding.console;
        if !console.open {
            return;
        }
        let test = &console.test;
        let width = test.width;
        let painter = ui::hud::world_painter(ctx);
        let found = match &test.result {
            Some(PathResult::Found(points)) if points.len() >= 2 => Some(points.as_slice()),
            _ => None,
        };
        if let Some(points) = found {
            let pieces = ribbon_pieces(points, width);
            // theme-guard: content — the path's colour, the Legacy "shown in orange"
            let (faint, solid) = (p.amber.gamma_multiply(PATH_XRAY), p.amber);
            // first where the scene hides it, then over it where it shows
            aurora_render::world_label_at(ctx, None);
            for piece in &pieces {
                if let Some(mesh) = ribbon_mesh(piece, proj, faint) {
                    painter.add(mesh);
                }
            }
            for piece in &pieces {
                if let Some(mesh) = ribbon_mesh(piece, proj, solid) {
                    aurora_render::world_line_at(ctx, piece.from, piece.to);
                    painter.add(mesh);
                }
            }
            aurora_render::world_label_at(ctx, None);
        }
        // the bookends stand where the region put the path's ends (on the
        // navmesh, under a clicked box or floor), else at the clicked points
        let ends = match found {
            Some(points) => [points.first().copied(), points.last().copied()],
            None => [test.start, test.end],
        };
        let capsule = |point: Vec3| Capsule {
            center: point + Vec3::Z * width,
            rotation: glam::Quat::IDENTITY,
            length: width * 2.0,
            radius: width / 2.0,
            horizontal: false,
        };
        for (point, color) in ends.into_iter().zip([p.success, p.danger]) {
            if let Some(point) = point {
                ui::pathfinding::capsule::draw_outline(ctx, &painter, proj, &capsule(point), color);
            }
        }
        // the end a click would set, under the cursor
        if let Some((end, point)) = self.path_hover {
            let color = if end { p.danger } else { p.success };
            ui::pathfinding::capsule::draw_outline(ctx, &painter, proj, &capsule(point), color.gamma_multiply(PATH_HOVER));
        }
    }
}

/// The path's opacity where the scene hides it.
const PATH_XRAY: f32 = 0.35;
/// The opacity of the capsule under the cursor before the click.
const PATH_HOVER: f32 = 0.5;
/// Height of the ribbon over the path's points (on the navmesh): enough for
/// a level ribbon to clear ground that slopes across it, as the Legacy band
/// floats over the floor.
const PATH_LIFT: f32 = 0.3;

/// One piece of the ribbon: a metre or so of the path, flat, as wide as the
/// character.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Piece {
    from: Vec3,
    to: Vec3,
    /// Half the width, across the path, level.
    side: Vec3,
}

/// The ribbon in pieces of about a metre: each piece's depth is tested
/// along its own middle line, which stays close to the ribbon's.
fn ribbon_pieces(points: &[Vec3], width: f32) -> Vec<Piece> {
    let mut out = Vec::new();
    for pair in points.windows(2) {
        let (a, b) = (pair[0] + Vec3::Z * PATH_LIFT, pair[1] + Vec3::Z * PATH_LIFT);
        let level = Vec3::new(b.x - a.x, b.y - a.y, 0.0);
        let across = if level.length_squared() > 1e-6 {
            Vec3::new(-level.y, level.x, 0.0).normalize()
        } else {
            Vec3::X
        };
        let side = across * (width * 0.5);
        let count = (a.distance(b).ceil() as usize).clamp(1, 64);
        for i in 0..count {
            out.push(Piece {
                from: a.lerp(b, i as f32 / count as f32),
                to: a.lerp(b, (i + 1) as f32 / count as f32),
                side,
            });
        }
    }
    out
}

/// A piece on the screen, None when a corner is behind the camera.
fn ribbon_mesh(piece: &Piece, proj: &ui::hud::Projector, color: egui::Color32) -> Option<egui::Shape> {
    let corners = [
        piece.from - piece.side,
        piece.from + piece.side,
        piece.to + piece.side,
        piece.to - piece.side,
    ];
    let mut mesh = egui::Mesh::default();
    for corner in corners {
        let c = proj.view_proj * corner.extend(1.0);
        if c.w <= 0.05 {
            return None;
        }
        let ndc = c.truncate() / c.w;
        let at = egui::pos2(
            (ndc.x * 0.5 + 0.5) * proj.width / proj.ppp,
            (0.5 - ndc.y * 0.5) * proj.height / proj.ppp,
        );
        mesh.colored_vertex(at, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Some(egui::Shape::mesh(mesh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ribbon_is_as_wide_as_the_character_in_metre_pieces() {
        let points = [
            Vec3::new(10.0, 10.0, 20.0),
            Vec3::new(13.0, 10.0, 20.0),
            Vec3::new(13.0, 10.0, 20.0),
        ];
        let pieces = ribbon_pieces(&points, 1.0);
        // 3 m in three pieces, and the repeated point gives one level piece
        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces[0].from, Vec3::new(10.0, 10.0, 20.0 + PATH_LIFT));
        assert_eq!(pieces[2].to, Vec3::new(13.0, 10.0, 20.0 + PATH_LIFT));
        assert!((pieces[0].side.length() - 0.5).abs() < 1e-6);
        assert_eq!(pieces[0].side.z, 0.0, "flat: level across the path");
        assert!(pieces[0].side.dot(Vec3::X).abs() < 1e-6, "across the path");
    }
}
