//! The pathfinding console's View tab in the app: the linksets it colours
//! (asked of the region with RegionObjects, as the Region objects window
//! does), the scene's hiding and shapes (`scene::pathfinding_view`), and the
//! water plane. LLFloaterPathfindingConsole's show options and the
//! pathfinding pass of pipeline.cpp (renderSimpleShapes for the water
//! plane), indra/newview, originally LGPL 2.1.
use super::*;
use crate::scene::pathfinding_view::PathfindingView;
use aurora_net::pathfinding::Kind;
use aurora_net::pathfinding::linkset::Use;
use std::collections::HashMap;

/// How often the shapes are matched to the prims again (objects arrive and
/// leave while the console is open).
const REMATCH: Duration = Duration::from_millis(500);
/// The water plane's grid spacing.
const WATER_GRID: f32 = 8.0;

/// What the app remembers between frames for the View tab.
#[derive(Default)]
pub(super) struct ViewFeed {
    /// The overlay list was asked for this region.
    asked_for: Option<aurora_net::RegionHandle>,
    /// The navmesh status the list was asked under (a rebake changes uses).
    asked_status: Option<crate::ui::pathfinding::rebake::Mode>,
    matched_at: Option<Instant>,
}

/// The shape list of a use, if the View tab shows that use
/// (getRenderShapeFlags: movable obstacles and phantoms are no shape).
fn shape(use_: Use, view: &crate::ui::pathfinding::console::ViewOptions) -> Option<usize> {
    match use_ {
        Use::Walkable if view.walkables => Some(aurora_render::PATHFINDING_WALKABLE),
        Use::MaterialVolume if view.material_volumes => Some(aurora_render::PATHFINDING_MATERIAL),
        Use::StaticObstacle if view.static_obstacles => Some(aurora_render::PATHFINDING_STATIC),
        Use::ExclusionVolume if view.exclusion_volumes => Some(aurora_render::PATHFINDING_EXCLUSION),
        _ => None,
    }
}

impl App {
    /// Once a frame: the overlay list, then the scene's View state.
    pub(super) fn pathfinding_view_frame(&mut self, now: Instant) {
        let console = &self.pathfinding.console;
        let view = console.view;
        let shapes = view.walkables || view.material_volumes || view.static_obstacles || view.exclusion_volumes;
        let open = console.open;
        if !open {
            if self.scene.pathfinding_view.active() {
                self.set_pathfinding_view(PathfindingView::default());
            }
            return;
        }
        // the list: asked when shapes are wanted, again in another region
        // or once the navmesh changed
        let region = self.world.main_region;
        let status = self.pathfinding.rebake.mode();
        let feed = &mut self.pathfinding_view_feed;
        if shapes && !self.pathfinding.overlay_list.is_pending() && (feed.asked_for != region || feed.asked_status != Some(status)) {
            feed.asked_for = region;
            feed.asked_status = Some(status);
            self.fetch_overlay_list();
        }
        if self
            .pathfinding_view_feed
            .matched_at
            .is_some_and(|at| now.duration_since(at) < REMATCH)
        {
            return;
        }
        self.pathfinding_view_feed.matched_at = Some(now);
        let mut overlay = HashMap::new();
        if shapes {
            let w = &self.world;
            for row in self.pathfinding.overlay_list.rows() {
                let (Some(linkset), false) = (row.linkset(), row.id.is_nil()) else {
                    continue;
                };
                let Some(shape) = shape(linkset.use_, &view) else { continue };
                let Some(root) = w.objects.index_of_uuid(&row.id) else { continue };
                for prim in crate::build::family(w, root) {
                    overlay.insert(prim, shape);
                }
            }
        }
        let next = PathfindingView {
            world_hidden: !view.world,
            movables_only: view.world && view.movables_only,
            overlay,
            xray: view.xray,
            version: self.scene.pathfinding_view.version,
        };
        if next != self.scene.pathfinding_view {
            self.set_pathfinding_view(next);
        }
    }

    fn set_pathfinding_view(&mut self, mut next: PathfindingView) {
        next.version = self.scene.pathfinding_view.version.wrapping_add(1);
        self.scene.pathfinding_view = next;
    }

    /// RegionObjects for the shapes (the terrain row is not needed).
    fn fetch_overlay_list(&mut self) {
        let region = self.world.main_region.unwrap_or_default();
        let request = self.pathfinding.overlay_list.start_get(region);
        let command = NetCommand::FetchPathfinding {
            request,
            region,
            kind: Kind::Objects,
            terrain: false,
        };
        if self.demo {
            self.demo_overlay_answer(command);
        } else {
            self.send(command);
        }
    }

    /// Linksets were edited: the shapes follow their new uses.
    pub(super) fn overlay_list_stale(&mut self) {
        self.pathfinding_view_feed.asked_status = None;
    }

    /// The water plane (renderSimpleShapes at the region's water height): a
    /// grid of lines over the region, hidden by what stands in front of it.
    /// The Legacy Viewer draws a translucent plane; a depth-tested fill is
    /// not available to the interface layer, and a fill drawn over
    /// everything would cover what stands above the water.
    pub(super) fn draw_water_plane(&self, ctx: &egui::Context, p: &crate::theme::Palette, proj: &ui::hud::Projector) {
        let console = &self.pathfinding.console;
        if !console.open || !console.view.water_plane {
            return;
        }
        let Some(region) = self.world.main() else { return };
        let height = region.info.as_ref().map_or(20.0, |i| i.water_height);
        let (sx, sy) = (region.heightmap.size_x as f32, region.heightmap.size_y as f32);
        let painter = ui::hud::world_painter(ctx);
        // theme-guard: content — the water plane, in the palette's water blue
        let stroke = egui::Stroke::new(1.0, p.indigo_light.gamma_multiply(0.6));
        // one depth-tested segment per line: a straight line's depth is
        // exact along it on the screen
        let line = |a: Vec3, b: Vec3| {
            if let Some(segment) = ui::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
                aurora_render::world_line_at(ctx, a, b);
                painter.line_segment(segment, stroke);
            }
        };
        let mut x = 0.0;
        while x <= sx {
            line(Vec3::new(x, 0.0, height), Vec3::new(x, sy, height));
            x += WATER_GRID;
        }
        let mut y = 0.0;
        while y <= sy {
            line(Vec3::new(0.0, y, height), Vec3::new(sx, y, height));
            y += WATER_GRID;
        }
        aurora_render::world_label_at(ctx, None);
    }
}
