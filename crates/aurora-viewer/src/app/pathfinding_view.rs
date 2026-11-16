//! The pathfinding console's View tab in the app: the linksets it colours
//! (asked of the region with RegionObjects, as the Region objects window
//! does), their prims' collision shapes (GetObjectPhysicsData, as
//! LLViewerObjectList::fetchPhysicsFlags asks them), the scene's hiding and
//! shapes (`scene::pathfinding_view`), and the water plane.
//! LLFloaterPathfindingConsole's show options and the pathfinding pass of
//! pipeline.cpp (renderSimpleShapes for the water plane), indra/newview,
//! originally LGPL 2.1.
use super::*;
use crate::scene::pathfinding_view::PathfindingView;
use aurora_net::pathfinding::Kind;
use aurora_net::pathfinding::linkset::Use;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// How often the shapes are matched to the prims again (objects arrive and
/// leave while the console is open).
const REMATCH: Duration = Duration::from_millis(500);
/// The water plane's grid spacing.
const WATER_GRID: f32 = 8.0;
/// Ids per GetObjectPhysicsData request
/// (llviewerobjectlist.cpp MAX_CONCURRENT_PHYSICS_REQUESTS).
const PHYSICS_BATCH: usize = 256;
/// GetObjectPhysicsData requests on their way at once.
const PHYSICS_IN_FLIGHT: usize = 2;
/// A request without an answer by then is given up: its prims keep their
/// visible faces.
const PHYSICS_TIMEOUT: Duration = Duration::from_secs(30);
/// The known shape types are forgotten past this many (a long session).
const PHYSICS_KEPT: usize = 100_000;

/// What the app remembers between frames for the View tab.
#[derive(Default)]
pub(super) struct ViewFeed {
    /// The overlay list was asked for this region.
    asked_for: Option<aurora_net::RegionHandle>,
    /// The navmesh status the list was asked under (a rebake changes uses).
    asked_status: Option<crate::ui::pathfinding::rebake::Mode>,
    matched_at: Option<Instant>,
    /// Prim id → its PhysicsShapeType.
    shape_types: HashMap<Uuid, u8>,
    /// Asked once: answered, on its way or failed (then its faces stay).
    asked: HashSet<Uuid>,
    /// Capability tag → when it was sent.
    in_flight: HashMap<u64, Instant>,
    /// The scene view the shape sets were last built for, and when.
    shapes_version: u64,
    shapes_at: Option<Instant>,
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

/// The GetObjectPhysicsData requests for the prims whose shape type is
/// wanted: by region, at most `slots` requests of `PHYSICS_BATCH` ids.
fn physics_batches(
    wanted: impl IntoIterator<Item = (aurora_net::RegionHandle, Uuid)>,
    slots: usize,
) -> Vec<(aurora_net::RegionHandle, Vec<Uuid>)> {
    let mut by_region: Vec<(aurora_net::RegionHandle, Vec<Uuid>)> = Vec::new();
    for (region, id) in wanted {
        if let Some((_, ids)) = by_region.iter_mut().find(|(r, ids)| *r == region && ids.len() < PHYSICS_BATCH) {
            ids.push(id);
        } else if by_region.len() < slots {
            by_region.push((region, vec![id]));
        }
    }
    by_region
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
        let mut physics = HashMap::new();
        let mut wanted = Vec::new();
        if shapes {
            let w = &self.world;
            let feed = &self.pathfinding_view_feed;
            for row in self.pathfinding.overlay_list.rows() {
                let (Some(linkset), false) = (row.linkset(), row.id.is_nil()) else {
                    continue;
                };
                let Some(shape) = shape(linkset.use_, &view) else { continue };
                let Some(root) = w.objects.index_of_uuid(&row.id) else { continue };
                for prim in crate::build::family(w, root) {
                    overlay.insert(prim, shape);
                    let Some(object) = w.objects.get(prim) else { continue };
                    match feed.shape_types.get(&object.full_id) {
                        Some(&kind) if crate::scene::physics_shapes::has_shape(object) => {
                            physics.insert(prim, kind);
                        }
                        Some(_) => {}
                        None if !feed.asked.contains(&object.full_id) => wanted.push((object.key.region, object.full_id)),
                        None => {}
                    }
                }
            }
        }
        self.ask_shape_types(wanted, now);
        let next = PathfindingView {
            world_hidden: !view.world,
            movables_only: view.world && view.movables_only,
            overlay,
            physics,
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

    /// GetObjectPhysicsData for the prims whose shape type is not known
    /// (fetchPhisicsFlagsCoro), a few requests at a time.
    fn ask_shape_types(&mut self, wanted: Vec<(aurora_net::RegionHandle, Uuid)>, now: Instant) {
        let feed = &mut self.pathfinding_view_feed;
        feed.in_flight.retain(|_, sent| now.duration_since(*sent) < PHYSICS_TIMEOUT);
        let slots = PHYSICS_IN_FLIGHT.saturating_sub(feed.in_flight.len());
        if wanted.is_empty() || slots == 0 {
            return;
        }
        for (region, ids) in physics_batches(wanted, slots) {
            let body = aurora_llsd::llsd_map! { "object_ids" => aurora_llsd::Llsd::Array(ids.iter().copied().map(aurora_llsd::Llsd::from).collect()) };
            let tag = self.build.cap(
                region,
                "GetObjectPhysicsData",
                false,
                body,
                crate::build::edits::CapPurpose::PathfindingPhysics,
            );
            let feed = &mut self.pathfinding_view_feed;
            feed.asked.extend(ids);
            feed.in_flight.insert(tag, now);
        }
        self.flush_build();
    }

    /// A capability answer: the shape types, when it is one of ours.
    pub(super) fn on_pathfinding_physics(&mut self, tag: u64, result: &Result<aurora_llsd::Llsd, String>) {
        let feed = &mut self.pathfinding_view_feed;
        if feed.in_flight.remove(&tag).is_none() {
            return;
        }
        let Ok(reply) = result else {
            log::warn!("pathfinding: GetObjectPhysicsData failed; those prims keep their faces");
            return;
        };
        if feed.shape_types.len() > PHYSICS_KEPT {
            feed.shape_types.clear();
            feed.asked.clear();
        }
        // the reply is a map of the ids asked; LLViewerObjectList keeps a
        // type of 0 to 2 only
        if let aurora_llsd::Llsd::Map(map) = reply {
            for (id, data) in map {
                let kind = data["PhysicsShapeType"].as_i32();
                if let (Ok(id), true) = (id.parse::<Uuid>(), (0..=2).contains(&kind) && data.has("PhysicsShapeType")) {
                    feed.shape_types.insert(id, kind as u8);
                }
            }
        }
        // matched at the next frame, not half a second later
        feed.matched_at = None;
    }

    /// Once a frame, after the scene's lists: the shapes drawn from their
    /// physics, in their use's list. Built again when the View changes and
    /// twice a second (the shapes' prims are permanent: they do not move),
    /// and at every frame while a shape is still being built.
    pub(super) fn pathfinding_shapes_frame(&mut self, renderer: &mut aurora_render::Renderer, now: Instant) {
        let view = &self.scene.pathfinding_view;
        let feed = &mut self.pathfinding_view_feed;
        let due = feed.shapes_version != view.version
            || feed.shapes_at.is_none_or(|at| now.duration_since(at) >= REMATCH)
            || self.scene.pathfinding_shapes.iter().any(|s| s.pending());
        if due {
            feed.shapes_version = view.version;
            feed.shapes_at = Some(now);
            let mut sets: [Vec<(usize, u8)>; 4] = Default::default();
            for (&prim, &kind) in &view.physics {
                // a prim of shape None is not part of the navmesh: nothing
                if let (Some(&shape), true) = (view.overlay.get(&prim), kind != 1) {
                    sets[shape].push((prim, kind));
                }
            }
            let scene = &mut self.scene;
            for (shapes, set) in scene.pathfinding_shapes.iter_mut().zip(&sets) {
                shapes.update(renderer, &self.world, &mut scene.meshes, set);
            }
        }
        let scene = &mut self.scene;
        for (shapes, list) in scene.pathfinding_shapes.iter().zip(scene.lists.pathfinding.iter_mut()) {
            shapes.append_to(list);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_types_are_asked_by_region_in_batches_and_slots() {
        let ids = |n: u128| (0..n).map(Uuid::from_u128);
        let wanted: Vec<_> = ids(300).map(|id| (1, id)).chain(ids(3).map(|id| (2, id))).collect();
        let batches = physics_batches(wanted.clone(), 2);
        assert_eq!(batches.len(), 2);
        assert_eq!((batches[0].0, batches[0].1.len()), (1, PHYSICS_BATCH));
        assert_eq!(
            (batches[1].0, batches[1].1.len()),
            (1, 300 - PHYSICS_BATCH),
            "the rest waits for a slot"
        );
        let batches = physics_batches(wanted, 3);
        assert_eq!((batches[2].0, batches[2].1.len()), (2, 3));
        assert!(physics_batches(vec![(1, Uuid::nil())], 0).is_empty());
    }
}
