//! LLVOVolume::lineSegmentIntersect: CLICK_ACTION_IGNORE is excluded outside
//! build mode (Firestorm llvovolume.cpp / llspatialpartition.cpp, LGPL 2.1).
//! CPU triangles share the lifetime of the uploaded geometry, including sculpts.
use super::pick_accel::{self, PickAccel};
use super::*;
use crate::media::pick::{self, FaceTris};
use aurora_net::TouchSurface;
use glam::Vec2;
use rayon::prelude::*;
use std::sync::atomic::Ordering::Relaxed;

pub struct PickFace {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    /// Hierarchy of the triangles, for the hover search (built on demand).
    pub accel: PickAccel,
}

/// The ray searches over their candidates (hover every frame, clicks) run
/// the frame's way (frame_par.rs): the main thread starts on them at once
/// and the pool's workers join as they wake up. A rayon `par_iter` called
/// from the main thread, which is not a worker of the pool, slept until the
/// last worker was done: 25 to 45 ms per frame in the demo crowd while
/// other programs kept the cores busy, for a search of a fraction of a
/// millisecond.
const RAY_GRAIN: Grain = Grain { chunk: 32, per_helper: 8 };

/// `hit` on every item of `items`, in parallel as above; the hits, in no
/// particular order.
fn ray_hits<T: Sync, H: Send>(items: &[T], hit: impl Fn(&T) -> Option<H> + Sync) -> Vec<H> {
    frame_par::fold(items, RAY_GRAIN, Vec::new, |mut found, item| {
        found.extend(hit(item));
        found
    })
    .into_iter()
    .flatten()
    .collect()
}

/// The nearest of `hits` by distance (`.0`), the lowest object index
/// (`.1`) among equals: the same answer whatever the order of the hits.
fn nearest_hit<H>(hits: Vec<H>, key: impl Fn(&H) -> (f32, usize)) -> Option<H> {
    hits.into_iter().min_by(|a, b| {
        let (a, b) = (key(a), key(b));
        a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1))
    })
}

/// The box around the box `min`..`max` brought by `m`.
fn transformed_box(m: Mat4, min: Vec3, max: Vec3) -> (Vec3, Vec3) {
    (0..8).fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), i| {
        let corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let p = m.transform_point3(corner);
        (lo.min(p), hi.max(p))
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FacePick {
    Surface,
    Inventory,
    Blend,
    HudMove,
    /// Left button: only world Blend faces test exact nonzero texels.
    Left,
    LeftBlend,
}

impl PickFace {
    pub fn new(positions: Vec<[f32; 3]>, uvs: Vec<[f32; 2]>, indices: Vec<u16>) -> PickFace {
        PickFace {
            positions,
            uvs,
            indices,
            accel: PickAccel::default(),
        }
    }
}

/// Whether a ray can meet an object, from its world bounding sphere kept by
/// the sync: a few multiplications instead of the transform, matrix inverse
/// and box test of `face_hit`, for every object, every hovered frame. Rigged
/// meshes (sphere of the wearer, not of the bind pose) and objects without
/// bounds yet always pass.
pub(super) fn ray_may_hit(g: &super::ObjGpu, ray: (Vec3, Vec3)) -> bool {
    if g.rigged || g.radius <= 0.0 {
        return true;
    }
    let r = g.radius * 1.05 + 0.05;
    let to = g.center - ray.0;
    let along = to.dot(ray.1);
    if along < -r {
        return false;
    }
    to.length_squared() - along * along <= r * r
}

/// A mesh without a skin worn by an avatar. It is placed by its attachment
/// point like a worn prim, but the GPU draws its faces at a newer pose: the
/// CPU only answers for the faces the GPU does not draw, those at alpha 0
/// (#416, #419). These are the invisible clickers worn on an avatar, which
/// the Legacy Viewer picks when they have a touch script or a click action
/// (LLVOVolume::lineSegmentIntersect, specialHoverCursor) (#488).
fn worn_unrigged_mesh(o: &Object, g: &super::ObjGpu) -> bool {
    !g.rigged && !g.is_avatar && g.owner_avatar.is_some() && o.visual_volume().is_mesh()
}

pub fn ray_box(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut near = 0.0f32;
    let mut far = f32::INFINITY;
    for i in 0..3 {
        if dir[i].abs() < 1e-8 {
            if origin[i] < min[i] || origin[i] > max[i] {
                return None;
            }
        } else {
            let a = (min[i] - origin[i]) / dir[i];
            let b = (max[i] - origin[i]) / dir[i];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if far < near {
                return None;
            }
        }
    }
    Some(near)
}

impl Scene {
    fn movement_solid(&self, world: &World, idx: usize) -> bool {
        let Some(o) = world.objects.get(idx) else { return false };
        o.pcode == aurora_prim::params::LL_PCODE_VOLUME
            && o.update_flags & crate::build::edits::FLAGS_PHANTOM == 0
            && self.gpu.get(idx).is_some_and(|g| !g.hud && !g.rigged && g.owner_avatar.is_none())
    }
    /// Veto cannonballs over a loaded solid deck before releasing native flight.
    /// Five rays cover the body footprint, stopping at the water surface so
    /// submerged floors are not mistaken for obstacles above the water.
    pub fn cannonball_clear(&self, world: &World, frame: &crate::movement::Frame) -> bool {
        let Some(water) = frame.water.filter(|water| water.is_finite()) else {
            return false;
        };
        if !frame.position.is_finite() || !frame.height.is_finite() || frame.position.z <= water {
            return false;
        }
        let radius = (frame.height * 0.16).clamp(0.18, 0.35);
        let distance = frame.position.z - water;
        let now = Instant::now();
        let mut candidates = Vec::new();
        for offset in [Vec3::ZERO, Vec3::X * radius, -Vec3::X * radius, Vec3::Y * radius, -Vec3::Y * radius] {
            let ray = (frame.position + offset, -Vec3::Z);
            candidates.clear();
            self.pick.ray_candidates(ray, &mut candidates);
            if candidates.iter().any(|&idx| {
                self.movement_solid(world, idx) && self.face_hit(world, idx, ray, now, true).is_some_and(|hit| hit.t <= distance)
            }) {
                return false;
            }
        }
        true
    }
    /// Independent support check for water exits when Falling/foot-plane
    /// messages lag. Only a nearby upward-facing solid top counts as land.
    pub fn movement_support(&self, world: &World, position: Vec3, height: f32) -> bool {
        if !position.is_finite() || !height.is_finite() {
            return false;
        }
        let ray = (position, -Vec3::Z);
        let now = Instant::now();
        let mut candidates = Vec::new();
        self.pick.ray_candidates(ray, &mut candidates);
        candidates.iter().any(|&idx| {
            self.movement_solid(world, idx)
                && self
                    .face_hit(world, idx, ray, now, true)
                    .is_some_and(|hit| hit.normal.z > 0.7 && (height * 0.35..=height * 0.6 + 0.12).contains(&hit.t))
        })
    }
    /// Presentation prediction only: reject known solid decks, walls and roofs
    /// along the native jump arc. The simulator retains collision authority.
    pub fn water_jump_clear(
        &self,
        world: &World,
        frame: &crate::movement::Frame,
        horizontal: Vec3,
        landing: crate::movement::water_jump::Landing,
    ) -> bool {
        let now = Instant::now();
        let mut candidates = Vec::new();
        let mut clear_segment = |from: Vec3, to: Vec3| {
            let delta = to - from;
            let distance = delta.length();
            if distance < 0.001 {
                return true;
            }
            let ray = (from, delta / distance);
            candidates.clear();
            self.pick.ray_candidates(ray, &mut candidates);
            !candidates.iter().any(|&idx| {
                self.movement_solid(world, idx) && self.face_hit(world, idx, ray, now, true).is_some_and(|hit| hit.t <= distance)
            })
        };
        let up = if frame.on_ground { 4.5 } else { frame.velocity.z };
        let mut previous = frame.position;
        for i in 1..=8 {
            let t = landing.seconds * i as f32 / 8.0;
            let next = frame.position + horizontal.with_z(0.0) * t + Vec3::Z * (up * t - 4.905 * t * t);
            for offset in [-frame.height * 0.35, 0.0, frame.height * 0.35] {
                if !clear_segment(previous + Vec3::Z * offset, next + Vec3::Z * offset) {
                    return false;
                }
            }
            previous = next;
        }
        clear_segment(landing.position + Vec3::Z * 0.05, landing.position - Vec3::Z * (frame.height * 0.7))
    }
    /// A short forward probe for swimming. Uses rendered surface normals,
    /// not height alone, so a shallow beach never triggers the exit jump.
    pub fn movement_obstacle(&self, world: &World, origin: Vec3, direction: Vec3) -> bool {
        let ray = (origin, direction);
        let mut candidates = Vec::new();
        self.pick.ray_candidates(ray, &mut candidates);
        let now = Instant::now();
        candidates.par_iter().any(|&idx| {
            self.movement_solid(world, idx)
                && self.gpu.get(idx).is_some_and(|g| ray_may_hit(g, ray))
                && self
                    .face_hit(world, idx, ray, now, true)
                    .is_some_and(|h| h.t < 1.4 && h.normal.z.abs() < 0.55)
        })
    }
    /// Find a nearby walkable top beyond a steep face, with room above it.
    /// The server retains collision authority; this is only a bounded jump goal.
    pub fn swim_landing(&self, world: &World, origin: Vec3, direction: Vec3, height: f32) -> Option<Vec3> {
        let now = Instant::now();
        let mut candidates = Vec::new();
        self.pick.ray_candidates((origin, direction), &mut candidates);
        let distance = candidates
            .par_iter()
            .filter_map(|&idx| {
                if !self.movement_solid(world, idx) {
                    return None;
                }
                let hit = self.face_hit(world, idx, (origin, direction), now, true)?;
                (hit.t <= 1.4 && hit.normal.z.abs() < 0.55).then_some(hit.t)
            })
            .min_by(f32::total_cmp)?;
        for ahead in [distance + 0.45, distance + 0.8, distance + 1.1] {
            let top = origin + direction * ahead + Vec3::Z * 3.5;
            let ray = (top, -Vec3::Z);
            candidates.clear();
            self.pick.ray_candidates(ray, &mut candidates);
            let hit = candidates
                .par_iter()
                .filter_map(|&idx| {
                    self.movement_solid(world, idx)
                        .then(|| self.face_hit(world, idx, ray, now, true))
                        .flatten()
                })
                .filter(|hit| hit.t <= 4.0 && hit.normal.z > 0.7)
                .min_by(|a, b| a.t.total_cmp(&b.t));
            let Some(hit) = hit else { continue };
            let feet = top - Vec3::Z * hit.t;
            let destination = feet + Vec3::Z * (height * 0.5 + 0.08);
            let delta = destination - origin;
            if !(0.1..=3.5).contains(&delta.z) || delta.length_squared() > 16.0 {
                continue;
            }
            let head_ray = (feet + Vec3::Z * 0.12, Vec3::Z);
            candidates.clear();
            self.pick.ray_candidates(head_ray, &mut candidates);
            let blocked = candidates.par_iter().any(|&idx| {
                self.movement_solid(world, idx)
                    && self
                        .face_hit(world, idx, head_ray, now, true)
                        .is_some_and(|hit| hit.t < height + 0.4)
            });
            let occupied = self.gpu.iter().enumerate().any(|(idx, gpu)| {
                gpu.is_avatar
                    && world.objects.get(idx).is_some_and(|object| object.full_id != world.agent_id)
                    && (gpu.center - destination).truncate().length() < 0.7
                    && (gpu.center.z - destination.z).abs() < height
            });
            if !blocked && !occupied {
                return Some(destination);
            }
        }
        None
    }
    /// Conservative avatar-sized free space behind a nearby solid object.
    /// Visual geometry cannot reproduce Havok shapes; boxes deliberately
    /// reject uncertain destinations before a bounded scripted physical passage.
    pub fn movement_exit(&self, world: &World, origin: Vec3, direction: Vec3, height: f32) -> Option<Vec3> {
        let ray = (origin, direction);
        let mut candidates = Vec::new();
        self.pick.ray_candidates(ray, &mut candidates);
        let now = Instant::now();
        let (distance, idx) = candidates
            .par_iter()
            .filter_map(|&idx| {
                if !self.movement_solid(world, idx) {
                    return None;
                }
                let hit = self.face_hit(world, idx, ray, now, true)?;
                // Allow activation before the avatar is pressed against it.
                (hit.t <= 3.0).then_some((hit.t, idx))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))?;
        let (center, size) = self.click_bounds(world, idx)?;
        let (_, rotation, _) = Self::object_transform(world, idx, now, 0)?;
        let local_dir = rotation.inverse() * direction;
        let local_origin = rotation.inverse() * (origin - center);
        let half = size.abs() * 0.5 + Vec3::splat(0.4);
        // Backwards ray finds the far end of the same inflated box.
        let far = 8.0;
        let back = ray_box(local_origin + local_dir * far, -local_dir, -half, half)?;
        // Together with the inflated box, leave 1 m beyond a perpendicular
        // wall so the temporary anchor continues pulling until fully clear.
        let travel = (far - back + 0.60).max(distance + 0.5);
        if !(0.5..=8.0).contains(&travel) {
            return None;
        }
        let mut destination = origin + direction * travel;
        let region = world.main()?;
        if destination.x < 0.4
            || destination.y < 0.4
            || destination.x > region.heightmap.size_x as f32 - 0.4
            || destination.y > region.heightmap.size_y as f32 - 0.4
        {
            return None;
        }
        let ground = world.ground_height(destination)?;
        destination.z = destination.z.max(ground + height * 0.5 + 0.1);
        let occupied = self.gpu.par_iter().enumerate().any(|(idx, g)| {
            if g.is_avatar
                && let Some(object) = world.objects.get(idx)
            {
                if object.full_id == world.agent_id {
                    return false;
                }
                let delta = g.center - destination;
                return delta.truncate().length() < 0.7 && delta.z.abs() < (height + object.scale.z) * 0.5;
            }
            if !self.movement_solid(world, idx) {
                return false;
            }
            let Some((center, size)) = self.click_bounds(world, idx) else {
                return true;
            };
            if g.geom.is_none() {
                return center.distance(destination) < g.radius + height;
            }
            let Some((_, rotation, _)) = Self::object_transform(world, idx, now, 0) else {
                return true;
            };
            let local = (rotation.inverse() * (destination - center)).abs();
            // Rotate the capsule's conservative world box into object space.
            let inverse = glam::Mat3::from_quat(rotation.inverse());
            let ext = inverse.x_axis.abs() * 0.35 + inverse.y_axis.abs() * 0.35 + inverse.z_axis.abs() * (height * 0.5);
            local.cmple(size.abs() * 0.5 + ext).all()
        });
        (!occupied && destination.is_finite()).then_some(destination)
    }
    /// LLToolPie::handleRightMouseDown / LLToolSelectRect::handleMouseDown:
    /// pick_transparent includes every volume surface, without requiring a
    /// touch handler or click action. The displayed depth still bounds the
    /// search so terrain and GPU-posed surfaces occlude CPU geometry.
    /// `select_probes` follows SelectReflectionProbes in LLToolSelect::handleMouseDown.
    pub fn selection_pick(
        &self,
        world: &World,
        ray: Option<(Vec3, Vec3)>,
        depth: Option<Vec3>,
        build: bool,
        select_probes: bool,
        far: f32,
    ) -> (Option<Vec3>, Option<usize>) {
        self.selection_pick_mode(world, ray, depth, build, select_probes, far, FacePick::Surface)
    }

    pub fn left_selection_pick(
        &self,
        world: &World,
        ray: Option<(Vec3, Vec3)>,
        depth: Option<Vec3>,
        build: bool,
        select_probes: bool,
        far: f32,
    ) -> (Option<Vec3>, Option<usize>) {
        self.selection_pick_mode(world, ray, depth, build, select_probes, far, FacePick::Left)
    }

    #[allow(clippy::too_many_arguments)] // Selection inputs plus the button's alpha policy.
    fn selection_pick_mode(
        &self,
        world: &World,
        ray: Option<(Vec3, Vec3)>,
        depth: Option<Vec3>,
        build: bool,
        select_probes: bool,
        far: f32,
        mode: FacePick,
    ) -> (Option<Vec3>, Option<usize>) {
        let now = Instant::now();
        let mut candidates = Vec::new();
        if let Some(ray) = ray {
            self.pick.ray_candidates(ray, &mut candidates);
        }
        let point = if !build && ray.is_some_and(|r| self.ignored_before(world, r, depth, far, now, Some(&candidates))) {
            ray.and_then(|r| self.point_behind_ignored_mode(world, r, far, now, Some(&candidates), mode))
        } else {
            depth
        };
        if let Some(ray) = ray {
            let limit = point.map_or(far, |p| (p - ray.0).dot(ray.1) + 0.06).min(far);
            let hits = ray_hits(&candidates, |&idx| {
                let g = self.gpu.get(idx)?;
                let o = world.objects.get(idx)?;
                if g.hud
                    || g.is_avatar
                    || g.rigged
                    || (g.owner_avatar.is_some() && o.visual_volume().is_mesh() && !worn_unrigged_mesh(o, g))
                    || (!select_probes && o.extra.reflection_probe.is_some())
                    || (!build && o.click_action == crate::interaction::code::IGNORE)
                    || !ray_may_hit(g, ray)
                {
                    return None;
                }
                let hit = self.face_hit_mode(world, idx, ray, now, true, mode)?;
                (hit.t.is_finite() && hit.t <= limit).then_some((hit.t, idx))
            });
            let hit = nearest_hit(hits, |h| *h);
            if let Some((t, idx)) = hit {
                return (Some(ray.0 + ray.1 * t), Some(idx));
            }
        }
        let target = point
            .and_then(|p| self.interaction_at(world, p, now, build))
            .filter(|&idx| select_probes || world.objects.get(idx).is_some_and(|o| o.extra.reflection_probe.is_none()));
        (point, target)
    }

    /// LLToolPipette: sample the texture entry tint of the nearest volume face,
    /// independent of lighting, texture pixels, selection and modify rights.
    /// Candidate bounds and rendered triangles avoid a GPU readback.
    pub fn color_at_ray(&self, world: &World, ray: (Vec3, Vec3), hud: bool, far: f32) -> Option<(Uuid, [f32; 3])> {
        let mut candidates = Vec::new();
        if hud {
            let own = world.objects.index_of_uuid(&world.agent_id);
            candidates.extend(
                self.gpu
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, g)| (g.hud && g.owner_avatar == own).then_some(idx)),
            );
        } else {
            self.pick.ray_candidates(ray, &mut candidates);
        }
        let now = Instant::now();
        let hits = ray_hits(&candidates, |&idx| {
            let g = self.gpu.get(idx)?;
            let o = world.objects.get(idx)?;
            if g.hud != hud
                || g.rigged
                || o.extra.reflection_probe.is_some()
                || o.pcode != aurora_prim::params::LL_PCODE_VOLUME
                || !ray_may_hit(g, ray)
            {
                return None;
            }
            let hit = self.face_hit(world, idx, ray, now, false)?;
            if !hit.t.is_finite() || !hud && hit.t > far {
                return None;
            }
            let color = o.te.as_ref()?.faces.get(hit.face)?.color;
            Some((hit.t, idx, o.full_id, [color[0], color[1], color[2]]))
        });
        nearest_hit(hits, |h| (h.0, h.1)).map(|(_, _, id, rgb)| (id, rgb))
    }

    /// Exact edit surfaces along the whole cursor ray, including hidden and
    /// occluded prims. Reuses hover's bounds index and the rendered triangles;
    /// the wheel never waits for a GPU depth readback.
    pub fn edit_ray_hits(&self, world: &World, ray: (Vec3, Vec3), hud: bool, select_probes: bool, select_limit: Option<f32>) -> Vec<usize> {
        self.edit_ray_hits_mode(world, ray, hud, select_probes, select_limit, FacePick::Surface)
    }

    pub fn left_edit_ray_hits(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        hud: bool,
        select_probes: bool,
        select_limit: Option<f32>,
    ) -> Vec<usize> {
        self.edit_ray_hits_mode(world, ray, hud, select_probes, select_limit, FacePick::Left)
    }

    fn edit_ray_hits_mode(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        hud: bool,
        select_probes: bool,
        select_limit: Option<f32>,
        mode: FacePick,
    ) -> Vec<usize> {
        let mut candidates = Vec::new();
        if hud {
            let own = world.objects.index_of_uuid(&world.agent_id);
            candidates.extend(
                self.gpu
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, g)| (g.hud && g.owner_avatar == own).then_some(idx)),
            );
        } else {
            self.pick.ray_candidates(ray, &mut candidates);
        }
        let now = Instant::now();
        let mut hits = ray_hits(&candidates, |&idx| {
            let g = self.gpu.get(idx)?;
            let o = world.objects.get(idx)?;
            if g.hud != hud
                || (!select_probes && o.extra.reflection_probe.is_some())
                || !crate::build::selectable(world, idx)
                || !ray_may_hit(g, ray)
            {
                return None;
            }
            let hit = self.face_hit_mode(world, idx, ray, now, true, mode)?;
            if !hit.t.is_finite() || (!hud && select_limit.is_some_and(|max| (ray.0 + ray.1 * hit.t).distance(world.agent.position) > max))
            {
                return None;
            }
            Some((idx, hit.t))
        });
        hits.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        hits.into_iter().map(|(idx, _)| idx).collect()
    }

    /// Inventory placement follows LLToolDragAndDrop::pick: hidden and rigged
    /// surfaces are excluded and texture holes let the ray through. CPU
    /// triangles use the maintained pick index without a GPU readback or wait.
    pub fn inventory_surface(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        far: f32,
        candidates: &mut Vec<usize>,
    ) -> Option<(Vec3, Option<usize>)> {
        self.pick.ray_candidates(ray, candidates);
        self.inventory_surface_among(world, ray, far, Some(candidates))
    }

    /// LLToolPlacer::raycastForNewObjPos: visible surfaces, excluding flora
    /// and avatars. Shared by the creation hologram and the actual click,
    /// without sampling the hologram or waiting for GPU depth.
    pub fn creation_surface(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        far: f32,
        candidates: &mut Vec<usize>,
    ) -> Option<(Vec3, Option<usize>)> {
        self.pick.ray_candidates(ray, candidates);
        candidates.retain(|&idx| world.objects.get(idx).is_some_and(|o| !o.is_avatar() && !o.is_tree()));
        self.inventory_surface_among(world, ray, far, Some(candidates))
    }

    fn inventory_surface_among(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        far: f32,
        among: Option<&[usize]>,
    ) -> Option<(Vec3, Option<usize>)> {
        let now = Instant::now();
        let mut nearest = far;
        let mut target = None;
        let mut found = false;
        for (idx, g) in self.pick_objects(among) {
            if g.hud || g.rigged {
                continue;
            }
            let t = if g.is_avatar {
                ray_box(
                    ray.0,
                    ray.1,
                    g.center - Vec3::new(0.35, 0.35, 1.0),
                    g.center + Vec3::new(0.35, 0.35, 1.0),
                )
            } else {
                self.face_hit_mode(world, idx, ray, now, false, FacePick::Inventory).map(|h| h.t)
            };
            if let Some(t) = t
                && t > 0.0
                && t < nearest
            {
                nearest = t;
                target = Some(idx);
                found = true;
            }
        }
        // Same heightfield refinement as picking through IGNORE objects.
        // Only an active inventory drag queries it; release always recomputes.
        let mut previous = 0.0;
        let mut t = 0.25;
        while t <= nearest {
            let p = ray.0 + ray.1 * t;
            if world.ground_height(p).is_some_and(|z| p.z <= z) {
                let (mut lo, mut hi) = (previous, t);
                for _ in 0..12 {
                    let mid = (lo + hi) * 0.5;
                    let p = ray.0 + ray.1 * mid;
                    if world.ground_height(p).is_some_and(|z| p.z <= z) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return Some((ray.0 + ray.1 * hi, None));
            }
            previous = t;
            if t == nearest {
                break;
            }
            t = (t + 1.0).min(nearest);
        }
        found.then_some((ray.0 + ray.1 * nearest, target))
    }

    /// The objects a search looks at, in index order: all of them, or the
    /// candidates the pick index found for the ray (hover.rs). The exact
    /// tests are the same either way.
    fn pick_objects<'a>(&'a self, among: Option<&'a [usize]>) -> impl Iterator<Item = (usize, &'a ObjGpu)> + 'a {
        let all = if among.is_some() { 0..0 } else { 0..self.gpu.len() };
        among
            .unwrap_or_default()
            .iter()
            .copied()
            .chain(all)
            .filter_map(|idx| Some((idx, self.gpu.get(idx)?)))
            .filter(|(idx, gpu)| self.first_person_object_visible(*idx, gpu))
    }

    pub fn click_bounds(&self, world: &World, idx: usize) -> Option<(Vec3, Vec3)> {
        let now = Instant::now();
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        let g = self.gpu.get(idx);
        let geom = g.and_then(|g| g.geom).and_then(|key| self.geom_ready(&key));
        let (mut min, mut max) = geom.map_or((-Vec3::splat(0.5), Vec3::splat(0.5)), |g| (g.min, g.max));
        if let Some(unbind) = g.and_then(|g| self.unbind_of(g)) {
            (min, max) = transformed_box(unbind, min, max);
        }
        let scale = world.visual_scale_at(idx, now);
        Some((pos + rot * ((min + max) * 0.5 * scale), (max - min) * scale))
    }

    /// The geometry drawn for object `idx` and its model matrix; its
    /// `pick_faces` hold the vertices on the CPU. The build tool's rectangle
    /// tests them, as LLViewerCamera::areVertsVisible walks the volume faces.
    /// None while it loads, and for a rigged mesh (bind pose).
    pub fn drawn_geometry(&self, world: &World, idx: usize, now: Instant) -> Option<(Mat4, Arc<GpuGeom>)> {
        let g = self.gpu.get(idx)?;
        let o = world.objects.get(idx)?;
        if g.rigged {
            return None;
        }
        let key = g.geom?;
        let geom = self.geom_ready(&key)?;
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        Some((self.drawn_model(g, o, world.visual_scale_at(idx, now), Some(key), pos, rot), geom))
    }

    /// What undoes the bind shape of the geometry `g` draws: a skinned mesh
    /// drawn without a skeleton (`sync_plan`'s placement).
    fn unbind_of(&self, g: &ObjGpu) -> Option<Mat4> {
        match g.geom? {
            GeomKey::Mesh { id, .. } if !g.rigged => self.meshes.unbind(&id),
            _ => None,
        }
    }

    /// The model matrix of the geometry `g` draws for object `o`
    /// (`plants::geometry_model`, then `unbind_of`).
    pub(super) fn drawn_model(&self, g: &ObjGpu, o: &Object, scale: Vec3, key: Option<GeomKey>, pos: Vec3, rot: Quat) -> Mat4 {
        let model = super::plants::geometry_model(scale, o, key, pos, rot);
        self.unbind_of(g).map_or(model, |unbind| model * unbind)
    }

    pub(super) fn face_hit(
        &self,
        world: &World,
        idx: usize,
        ray: (Vec3, Vec3),
        now: Instant,
        include_hidden: bool,
    ) -> Option<pick::FaceHit> {
        self.face_hit_mode(world, idx, ray, now, include_hidden, FacePick::Surface)
    }

    pub(super) fn face_hit_mode(
        &self,
        world: &World,
        idx: usize,
        ray: (Vec3, Vec3),
        now: Instant,
        include_hidden: bool,
        mode: FacePick,
    ) -> Option<pick::FaceHit> {
        self.face_hit_in(world, idx, ray, now, include_hidden, mode, None)
    }

    /// `face_hit` against the full-detail geometry held for foot placement
    /// (`foot_support`) once it is built, the drawn one until then.
    pub(super) fn support_hit(&self, world: &World, idx: usize, ray: (Vec3, Vec3), now: Instant) -> Option<pick::FaceHit> {
        let id = world.objects.get(idx)?.full_id;
        let held = self.foot_support.key(idx, id).filter(|key| self.geom_ready(key).is_some());
        self.face_hit_in(world, idx, ray, now, true, FacePick::Surface, held)
    }

    /// `face_hit_mode` against `geometry` (another detail level of the
    /// drawn shape), or the drawn geometry.
    #[allow(clippy::too_many_arguments)] // face_hit_mode's inputs plus the geometry.
    fn face_hit_in(
        &self,
        world: &World,
        idx: usize,
        ray: (Vec3, Vec3),
        now: Instant,
        include_hidden: bool,
        mode: FacePick,
        geometry: Option<GeomKey>,
    ) -> Option<pick::FaceHit> {
        let o = world.objects.get(idx)?;
        let g = self.gpu.get(idx)?;
        if !self.first_person_object_visible(idx, g) {
            return None;
        }
        // The CPU triangles of a rigged mesh are in the bind pose. They must
        // never divert a touch from the surface actually drawn by the GPU
        // (#416, #419).
        if !g.hud && g.rigged {
            return None;
        }
        let only_hidden = !g.hud && worn_unrigged_mesh(o, g);
        if only_hidden && !include_hidden {
            return None;
        }
        let key = geometry.or(g.geom)?;
        let geom = self.geom_ready(&key)?;
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        let model = self.drawn_model(g, o, world.visual_scale_at(idx, now), Some(key), pos, rot);
        let inv = model.inverse();
        let counts = &self.exact_tests;
        counts.objects.fetch_add(1, Relaxed);
        ray_box(inv.transform_point3(ray.0), inv.transform_vector3(ray.1), geom.min, geom.max)?;
        counts.boxes.fetch_add(1, Relaxed);
        let faces = geom.pick_faces.iter().enumerate().filter_map(|(face, f)| {
            if mode == FacePick::Inventory {
                let draw = geom
                    .faces
                    .get(face)
                    .and_then(Option::as_ref)
                    .and_then(|a| g.faces.iter().find(|f| f.cmd.first_index == a.index_offset));
                if draw.is_none_or(|f| matches!(f.pass, Pass::Hidden)) {
                    return None;
                }
            }
            if matches!(mode, FacePick::Blend | FacePick::LeftBlend)
                && !self.blend_face_mode(world, o, g, face, mode == FacePick::LeftBlend)
            {
                return None;
            }
            let hidden = o.te.as_ref().is_some_and(|te| te.face(face).color[3] < 0.004);
            if only_hidden && !hidden {
                return None;
            }
            if !include_hidden && super::plants::plant_face(o).is_none() && hidden && !(g.hud && mode == FacePick::Surface) {
                return None;
            }
            let f = f.as_ref()?;
            counts.triangles.fetch_add(f.indices.len() as u64 / 3, Relaxed);
            Some(FaceTris {
                face,
                positions: &f.positions,
                uvs: &f.uvs,
                indices: &f.indices,
            })
        });
        if matches!(mode, FacePick::Left | FacePick::LeftBlend) && !g.hud && g.owner_avatar.is_none() {
            // LLVolume::lineSegmentIntersect rejects back faces. After a
            // texture hole, an invisible exit face must not reclaim the
            // click from the surface the GPU actually drew (#1574).
            return pick::ray_faces_filtered(
                faces,
                model,
                ray.0,
                ray.1,
                |face| self.face_double_sided(world, o, face),
                |hit| {
                    if !self.blend_face_mode(world, o, g, hit.face, true) {
                        return true;
                    }
                    let texture = g
                        .faces
                        .iter()
                        .find(|draw| usize::from(draw.te_index) == hit.face)
                        .map_or_else(|| o.visual_texture_face(hit.face).texture, |draw| draw.tex_id);
                    self.textures
                        .pick_masks
                        .get(&texture)
                        .is_none_or(|mask| mask.hits_nonzero(self.inventory_uv(world, o, hit, model, ray, now)))
                },
            );
        }
        if mode == FacePick::Inventory || (g.hud && !include_hidden) {
            return faces
                .filter_map(|face| {
                    let hit = if g.hud {
                        pick::ray_faces_with_sidedness(std::iter::once(face), model, ray.0, ray.1, |face| {
                            self.face_double_sided(world, o, face)
                        })?
                    } else {
                        pick::ray_faces(std::iter::once(face), model, ray.0, ray.1)?
                    };
                    let tf = o.te.as_ref().map(|te| *te.face(hit.face)).unwrap_or_default();
                    // LLToolPie::handleMouseDown prioritizes transparent
                    // attachments. Left clicks own the whole HUD triangle in
                    // every alpha mode, including Mask cutouts. Hover follows
                    // the same target; quick movement keeps texture holes.
                    // Right-click selection already includes hidden surfaces.
                    if g.hud && mode == FacePick::Surface {
                        return Some(hit);
                    }
                    let draw = geom
                        .faces
                        .get(hit.face)
                        .and_then(Option::as_ref)
                        .and_then(|a| g.faces.iter().find(|f| f.cmd.first_index == a.index_offset))?;
                    if g.hud && (draw.pass == Pass::Hidden || draw.te_alpha < 0.004) {
                        return (mode == FacePick::Surface).then_some(hit);
                    }
                    let ignore_alpha = draw.pbr_alpha == Some(0)
                        || matches!(
                            draw.legacy_alpha,
                            Some(legacy_mat::alpha_mode::NONE | legacy_mat::alpha_mode::EMISSIVE)
                        )
                        || (draw.legacy_alpha == Some(legacy_mat::alpha_mode::MASK)
                            && self.legacy_mats.peek(&tf.material_id).is_some_and(|m| m.alpha_cutoff == 0));
                    if ignore_alpha {
                        return Some(hit);
                    }
                    let Some(mask) = self.textures.pick_masks.get(&draw.tex_id) else {
                        return Some(hit);
                    };
                    mask.hits(self.inventory_uv(world, o, &hit, model, ray, now)).then_some(hit)
                })
                .min_by(|a, b| a.t.total_cmp(&b.t));
        }
        if !g.hud {
            return pick::ray_faces(faces, model, ray.0, ray.1);
        }
        // LLVolume::lineSegmentIntersect rejects back-facing triangles.
        // Explicit double-sided materials remain interactive on both sides.
        pick::ray_faces_with_sidedness(faces, model, ray.0, ray.1, |face| self.face_double_sided(world, o, face))
    }

    fn face_double_sided(&self, world: &World, o: &Object, face: usize) -> bool {
        let Some(id) = o.visual_material(face) else {
            return false;
        };
        let mut material = self.materials.peek(&id).map(|m| (*m).clone()).unwrap_or_default();
        if let Some((_, ov)) = world
            .gltf_overrides
            .get(&o.key)
            .and_then(|s| s.iter().rev().find(|(fi, _)| *fi as usize == face))
        {
            material.apply_override(ov);
        }
        material.double_sided
    }

    /// The material mode, independent of texel opacity. In particular, a
    /// hidden Blend face still blocks presses and hover, while a Mask face keeps its
    /// displayed cutout. Draw records already hold the resolved modes;
    /// omitted TE faces need the material and its override read directly.
    fn blend_face(&self, world: &World, o: &Object, g: &ObjGpu, face: usize) -> bool {
        self.blend_face_mode(world, o, g, face, false)
    }

    /// With no material, the build UI's alpha-channel default is Blend,
    /// even when binary texture pixels are rendered as an automatic mask.
    fn blend_face_mode(&self, world: &World, o: &Object, g: &ObjGpu, face: usize, left: bool) -> bool {
        if let Some(draw) = g.faces.iter().find(|draw| usize::from(draw.te_index) == face) {
            if let Some(mode) = draw.pbr_alpha {
                return mode == 1;
            }
            if let Some(mode) = draw.legacy_alpha {
                return mode == legacy_mat::alpha_mode::BLEND;
            }
            let alpha = self
                .textures
                .alpha_by_slot
                .get(draw.base_slot as usize)
                .copied()
                .unwrap_or_default();
            return draw.te_alpha < 0.999
                || alpha == AlphaKind::Blend
                || (left && !g.hud && g.owner_avatar.is_none() && alpha == AlphaKind::Mask);
        }
        if let Some(id) = o.visual_material(face) {
            let mut material = self.materials.peek(&id).map(|m| (*m).clone()).unwrap_or_default();
            if let Some((_, ov)) = world
                .gltf_overrides
                .get(&o.key)
                .and_then(|s| s.iter().rev().find(|(fi, _)| usize::from(*fi) == face))
            {
                material.apply_override(ov);
            }
            return material.alpha_mode == aurora_assets::AlphaMode::Blend;
        }
        let tf = o.visual_texture_face(face);
        if let Some(material) = self.legacy_mats.peek(&tf.material_id) {
            return material.diffuse_alpha_mode == legacy_mat::alpha_mode::BLEND;
        }
        tf.color[3] < 0.999
    }

    /// Includes faces omitted from the draw records because their TE alpha
    /// is zero, so the hover index can keep their material class as well.
    pub(super) fn has_blend_face(&self, world: &World, idx: usize) -> bool {
        self.has_blend_face_mode(world, idx, false)
    }

    fn has_blend_face_mode(&self, world: &World, idx: usize, left: bool) -> bool {
        let (Some(o), Some(g)) = (world.objects.get(idx), self.gpu.get(idx)) else {
            return false;
        };
        if let Some(geom) = g.geom.and_then(|key| self.geom_ready(&key)) {
            return geom
                .pick_faces
                .iter()
                .enumerate()
                .any(|(face, geometry)| geometry.is_some() && self.blend_face_mode(world, o, g, face, left));
        }
        g.faces
            .iter()
            .any(|draw| self.blend_face_mode(world, o, g, usize::from(draw.te_index), left))
    }

    /// The distance at which the ray meets the triangles of an object
    /// (`face_hit(…).t`). For the candidates of the hover search (`among`),
    /// through the hierarchies of its large faces when they are built.
    fn hit_distance(
        &self,
        world: &World,
        idx: usize,
        ray: (Vec3, Vec3),
        now: Instant,
        include_hidden: bool,
        among: Option<&[usize]>,
    ) -> Option<f32> {
        if among.is_some() && !self.hover.legacy() && self.gpu.get(idx).is_some_and(|g| !g.hud) {
            self.face_distance(world, idx, ray, now, include_hidden, FacePick::Surface)
        } else {
            self.face_hit(world, idx, ray, now, include_hidden).map(|h| h.t)
        }
    }

    /// `face_hit(…).t` of an object of the world (not a HUD): the same
    /// box, the same faces, the same test of the same triangles, except
    /// those the hierarchy of a face rules out (pick_accel.rs). A large
    /// face of a mesh or a sculpt without its hierarchy asks for it and is
    /// tested triangle by triangle meanwhile. (Not the prims: a flexible
    /// one replaces its triangles at each frame.)
    fn face_distance(
        &self,
        world: &World,
        idx: usize,
        ray: (Vec3, Vec3),
        now: Instant,
        include_hidden: bool,
        mode: FacePick,
    ) -> Option<f32> {
        let o = world.objects.get(idx)?;
        let g = self.gpu.get(idx)?;
        if g.rigged {
            return None;
        }
        let only_hidden = worn_unrigged_mesh(o, g);
        if only_hidden && !include_hidden {
            return None;
        }
        let key = g.geom?;
        let geom = self.geom_ready(&key)?;
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        let model = self.drawn_model(g, o, world.visual_scale_at(idx, now), Some(key), pos, rot);
        let inv = model.inverse();
        // (the ray in object space, as `pick::ray_faces` takes it there)
        let (origin, dir) = (inv.transform_point3(ray.0), inv.transform_vector3(ray.1));
        let counts = &self.exact_tests;
        counts.objects.fetch_add(1, Relaxed);
        ray_box(origin, dir, geom.min, geom.max)?;
        counts.boxes.fetch_add(1, Relaxed);
        let large = !matches!(key, GeomKey::Prim { .. });
        let mut best = f32::INFINITY;
        for (face, f) in geom.pick_faces.iter().enumerate() {
            if mode == FacePick::Blend && !self.blend_face(world, o, g, face) {
                continue;
            }
            let hidden = o.te.as_ref().is_some_and(|te| te.face(face).color[3] < 0.004);
            if only_hidden && !hidden {
                continue;
            }
            if !include_hidden && super::plants::plant_face(o).is_none() && hidden {
                continue;
            }
            let Some(f) = f else { continue };
            let accel = if large && f.indices.len() >= pick_accel::MIN_TRIANGLES * 3 {
                f.accel.get_or_ask(|| {
                    let geom = geom.clone();
                    self.jobs.run(move || {
                        if let Some(Some(f)) = geom.pick_faces.get(face) {
                            f.accel.build(&f.positions, &f.indices);
                        }
                    });
                })
            } else {
                None
            };
            if let Some(accel) = accel {
                counts.hierarchies.fetch_add(1, Relaxed);
                best = accel.nearest(&f.positions, &f.indices, origin, dir, best);
                continue;
            }
            counts.triangles.fetch_add(f.indices.len() as u64 / 3, Relaxed);
            for tri in f.indices.as_chunks::<3>().0 {
                let p = |i: u16| f.positions.get(i as usize).map(|p| Vec3::from_array(*p));
                let (Some(p0), Some(p1), Some(p2)) = (p(tri[0]), p(tri[1]), p(tri[2])) else {
                    continue;
                };
                if let Some(hit) = pick::ray_triangle(origin, dir, p0, p1, p2, true)
                    && hit.t > 0.0
                    && hit.t < best
                {
                    best = hit.t;
                }
            }
        }
        (best < f32::INFINITY).then_some(best)
    }

    /// Same coordinate conventions as object.wgsl, evaluated only for a hit
    /// with a resident alpha mask. Material clones contain fixed-size fields.
    fn inventory_uv(&self, world: &World, o: &Object, hit: &pick::FaceHit, model: Mat4, ray: (Vec3, Vec3), now: Instant) -> Vec2 {
        use aurora_render::tex_anim::{AnimClock, TexAnimParams, TexXform, clock_counter, frame_from_counter};
        let tf = super::plants::plant_face(o).unwrap_or_else(|| o.te.as_ref().map(|te| *te.face(hit.face)).unwrap_or_default());
        let mut st = hit.uv;
        if tf.planar() {
            let p = model.inverse().transform_point3(ray.0 + ray.1 * hit.t) * o.visual_scale_at(now);
            let n = model.transpose().transform_vector3(hit.normal).normalize_or_zero();
            let b = if n.x.abs() >= 0.5 {
                Vec3::new(0.0, if n.x < 0.0 { -1.0 } else { 1.0 }, 0.0)
            } else if n.y > 0.0 {
                -Vec3::X
            } else {
                Vec3::X
            };
            st = Vec2::new(0.5 + 2.0 * b.dot(p), 0.5 - 2.0 * b.cross(n).dot(p));
        }
        let material = o.visual_material(hit.face);
        let pbr = material.is_some();
        let clock = AnimClock::new(o.tex_anim_clock.start);
        let animated = tex_anim_record(o, hit.face, &tf, pbr, clock);
        if let Some(anim) = animated
            && let Some(ta) = o.tex_anim
        {
            let params = TexAnimParams {
                mode: ta.mode,
                size_x: ta.size_x,
                size_y: ta.size_y,
                start: ta.start,
                length: ta.length,
                rate: ta.rate,
            }
            .sanitized();
            let time = clock.now(now);
            let counter = clock_counter(
                &params,
                time[0].wrapping_sub(anim.anim[0]) as i32,
                f32::from_bits(time[1]),
                f32::from_bits(anim.anim[0]),
            );
            st = Vec2::from_array(
                frame_from_counter(
                    &params,
                    counter,
                    &TexXform {
                        rot: tf.rotation,
                        scale: [tf.scale_s, tf.scale_t],
                        offset: [tf.offset_s, tf.offset_t],
                    },
                )
                .apply(st.to_array()),
            );
        }
        if let Some(id) = material {
            let loaded = self.materials.peek(&id);
            let mut m = loaded.as_ref().map(|m| (**m).clone()).unwrap_or_default();
            if loaded.is_some()
                && let Some((_, ov)) = world
                    .gltf_overrides
                    .get(&o.key)
                    .and_then(|s| s.iter().rev().find(|(fi, _)| *fi as usize == hit.face))
            {
                m.apply_override(ov);
            }
            let t = m.transforms[0];
            let v = Vec2::new(st.x, 1.0 - st.y) * Vec2::from_array(t.scale);
            let (sin, cos) = t.rotation.sin_cos();
            let uv = Vec2::new(cos * v.x + sin * v.y, -sin * v.x + cos * v.y) + Vec2::from_array(t.offset);
            Vec2::new(uv.x, 1.0 - uv.y)
        } else if animated.is_some() {
            st
        } else {
            pick::te_transform(st, [tf.scale_s, tf.scale_t], [tf.offset_s, tf.offset_t], tf.rotation)
        }
    }

    pub fn touch_surface(&self, world: &World, idx: usize, ray: (Vec3, Vec3)) -> Option<TouchSurface> {
        self.touch_surface_mode(world, idx, ray, FacePick::Surface)
    }

    pub fn left_touch_surface(&self, world: &World, idx: usize, ray: (Vec3, Vec3)) -> Option<TouchSurface> {
        self.touch_surface_mode(world, idx, ray, FacePick::Left)
    }

    fn touch_surface_mode(&self, world: &World, idx: usize, ray: (Vec3, Vec3), mode: FacePick) -> Option<TouchSurface> {
        let o = world.objects.get(idx)?;
        let hit = self.face_hit_mode(world, idx, ray, Instant::now(), true, mode)?;
        let tf = o.te.as_ref().map(|t| *t.face(hit.face)).unwrap_or_default();
        let uv = pick::te_transform(hit.uv, [tf.scale_s, tf.scale_t], [tf.offset_s, tf.offset_t], tf.rotation);
        let mut position = ray.0 + ray.1 * hit.t;
        if let Some(root) = world.hud_root(idx) {
            let id = world.objects.get(root)?.full_id;
            let pivot = Self::object_transform(world, root, Instant::now(), 0)?.0;
            // Scripts receive the original HUD coordinates, never visual zoom.
            position = pivot - world.hud_scales.offset(id) + (position - pivot) / world.hud_scales.scale(id);
        }
        Some(TouchSurface {
            uv: uv.extend(0.0),
            st: hit.uv.extend(0.0),
            face: hit.face as i32,
            position: position
                - if self.gpu.get(idx)?.hud {
                    Vec3::ZERO
                } else {
                    world.region_offset(o.key.region)?
                },
            normal: hit.normal,
            binormal: hit.binormal,
        })
    }

    /// Usually keep the GPU depth pick. Only trace past it when an ignored
    /// object's triangles actually intercept the ray; it stays visible.
    pub fn interaction_point(&self, world: &World, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, build: bool, far: f32) -> Option<Vec3> {
        self.interaction_point_among(world, ray, depth, build, far, None)
    }

    /// `interaction_point` over the objects of `among` (None: all).
    pub(super) fn interaction_point_among(
        &self,
        world: &World,
        ray: Option<(Vec3, Vec3)>,
        depth: Option<Vec3>,
        build: bool,
        far: f32,
        among: Option<&[usize]>,
    ) -> Option<Vec3> {
        if build {
            return depth;
        }
        let ray = ray?;
        let now = Instant::now();
        if !self.ignored_before(world, ray, depth, far, now, among) {
            return self.transparent_action_point(world, ray, depth, false, far, among);
        }
        self.point_behind_ignored(world, ray, far, now, among)
    }

    /// First step of `interaction_point`: whether the triangles of an
    /// object set to IGNORE meet the ray before the depth point.
    pub(super) fn ignored_before(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        depth: Option<Vec3>,
        far: f32,
        now: Instant,
        among: Option<&[usize]>,
    ) -> bool {
        let limit = depth.map_or(far, |p| (p - ray.0).dot(ray.1) + 0.1);
        self.pick_objects(among).any(|(idx, g)| {
            !g.faces.is_empty()
                && !g.hud
                && world
                    .objects
                    .get(idx)
                    .is_some_and(|o| o.click_action == crate::interaction::code::IGNORE && o.extra.reflection_probe.is_none())
                && ray_may_hit(g, ray)
                && self.hit_distance(world, idx, ray, now, true, among).is_some_and(|t| t <= limit)
        })
    }

    /// `interaction_point` once an ignored object is in the way: the
    /// nearest surface that is not ignored, traced on the CPU.
    pub(super) fn point_behind_ignored(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        far: f32,
        now: Instant,
        among: Option<&[usize]>,
    ) -> Option<Vec3> {
        self.point_behind_ignored_mode(world, ray, far, now, among, FacePick::Surface)
    }

    fn point_behind_ignored_mode(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        far: f32,
        now: Instant,
        among: Option<&[usize]>,
        mode: FacePick,
    ) -> Option<Vec3> {
        let mut nearest = far;
        let mut found = false;
        for (idx, g) in self.pick_objects(among) {
            let Some(o) = world.objects.get(idx) else { continue };
            if g.faces.is_empty() || g.hud || o.extra.reflection_probe.is_some() || o.click_action == crate::interaction::code::IGNORE {
                continue;
            }
            if !g.is_avatar && !ray_may_hit(g, ray) {
                continue;
            }
            let t = if g.is_avatar {
                ray_box(
                    ray.0,
                    ray.1,
                    g.center - Vec3::new(0.35, 0.35, 1.0),
                    g.center + Vec3::new(0.35, 0.35, 1.0),
                )
            } else if mode == FacePick::Left {
                self.face_hit_mode(world, idx, ray, now, false, mode).map(|h| h.t)
            } else {
                self.hit_distance(world, idx, ray, now, false, among)
            };
            if let Some(t) = t
                && t > 0.0
                && t < nearest
            {
                nearest = t;
                found = true;
            }
        }
        // Terrain can also be the target behind an ignored screen.
        let mut previous = 0.0;
        let mut t = 1.0;
        while t < nearest {
            let p = ray.0 + ray.1 * t;
            if world.ground_height(p).is_some_and(|z| p.z <= z) {
                let mut lo = previous;
                let mut hi = t;
                for _ in 0..10 {
                    let mid = (lo + hi) * 0.5;
                    let p = ray.0 + ray.1 * mid;
                    if world.ground_height(p).is_some_and(|z| p.z <= z) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                nearest = hi;
                found = true;
                break;
            }
            previous = t;
            t += 1.0;
        }
        found.then_some(ray.0 + ray.1 * nearest)
    }

    /// The nearest surface the CPU finds along `ray` between `from` and
    /// `limit`, leaving out what it cannot place (rigged meshes, meshes worn
    /// by an avatar) and the avatar `wearer`, whose box holds what it wears.
    /// What a left click finds behind a rigged mesh it passes through
    /// (drawn.rs): nothing before that mesh's face is a surface (the GPU
    /// drew it nearest), not even a box of our avatar the ray grazes.
    pub(super) fn point_behind_rigged(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        from: f32,
        limit: f32,
        wearer: Option<usize>,
    ) -> Option<Vec3> {
        let now = Instant::now();
        let mut nearest = limit;
        let mut found = false;
        for (idx, g) in self.pick_objects(None) {
            let Some(o) = world.objects.get(idx) else { continue };
            if g.hud
                || g.rigged
                || Some(idx) == wearer
                || o.extra.reflection_probe.is_some()
                || o.click_action == crate::interaction::code::IGNORE
            {
                continue;
            }
            // an invisible clicker under the mesh, often partly inside the
            // body it is worn on (#1241): its alpha-0 faces count, as in the
            // Legacy Viewer's transparent pick, which never sees the rigged
            // mesh in front of it
            let clicker = !g.is_avatar && self.invisible_clicker(world, idx, g, o);
            if (g.faces.is_empty() && !clicker)
                || (g.owner_avatar.is_some() && !g.is_avatar && o.visual_volume().is_mesh() && !(clicker && worn_unrigged_mesh(o, g)))
            {
                continue;
            }
            let t = if g.is_avatar {
                ray_box(
                    ray.0,
                    ray.1,
                    g.center - Vec3::new(0.35, 0.35, 1.0),
                    g.center + Vec3::new(0.35, 0.35, 1.0),
                )
            } else if ray_may_hit(g, ray) {
                self.face_hit_mode(world, idx, ray, now, clicker, FacePick::Left).map(|h| h.t)
            } else {
                None
            };
            if let Some(t) = t
                && t >= from
                && t < nearest
            {
                nearest = t;
                found = true;
            }
        }
        // the ground behind an animated object
        let mut t = from.max(0.0);
        while t < nearest {
            let p = ray.0 + ray.1 * t;
            if world.ground_height(p).is_some_and(|z| p.z <= z) {
                return Some(p);
            }
            t += 1.0;
        }
        found.then_some(ray.0 + ray.1 * nearest)
    }

    /// Firestorm also picks actionable transparent faces: they do not write
    /// the GPU depth buffer. Hidden touch panels take clicks but no hover.
    pub(super) fn transparent_action_point(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        depth: Option<Vec3>,
        include_hidden: bool,
        far: f32,
        among: Option<&[usize]>,
    ) -> Option<Vec3> {
        self.transparent_action_point_mode(world, ray, depth, include_hidden, far, among, FacePick::Surface)
    }

    #[allow(clippy::too_many_arguments)] // Existing search inputs plus the gesture's alpha policy.
    fn transparent_action_point_mode(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        depth: Option<Vec3>,
        include_hidden: bool,
        far: f32,
        among: Option<&[usize]>,
        mode: FacePick,
    ) -> Option<Vec3> {
        let mut nearest = depth.map_or(far, |p| (p - ray.0).dot(ray.1));
        let mut found = false;
        let props = HashMap::new();
        for (idx, g) in self.pick_objects(among) {
            let Some(o) = world.objects.get(idx) else { continue };
            if g.hud
                || g.is_avatar
                || o.extra.reflection_probe.is_some()
                || o.click_action == crate::interaction::code::IGNORE
                || !ray_may_hit(g, ray)
            {
                continue;
            }
            let hidden = include_hidden && o.click_action != crate::interaction::code::DISABLED;
            // (a face at alpha 0 has no draw record at all, LL's
            // PASS_ALPHA_INVISIBLE: the object built fewer records than faces)
            if !(self.see_through_face(g, hidden) || (hidden && g.faces.len() < g.built_faces))
                || crate::interaction::target(world, idx, &props).is_none()
            {
                continue;
            }
            let t = if mode == FacePick::Left {
                self.face_hit_mode(world, idx, ray, Instant::now(), include_hidden, mode)
                    .map(|h| h.t)
            } else {
                self.hit_distance(world, idx, ray, Instant::now(), include_hidden, among)
            };
            if let Some(t) = t
                && t < nearest
            {
                nearest = t;
                found = true;
            }
        }
        if found { Some(ray.0 + ray.1 * nearest) } else { depth }
    }

    /// An object that `transparent_action_point` reaches through its hidden
    /// faces: one the GPU does not draw whole (alpha 0, the transparent
    /// texture) with something to do with a left click, the invisible
    /// clickers of LLToolPie::handleMouseDown's transparent pick.
    fn invisible_clicker(&self, world: &World, idx: usize, g: &ObjGpu, o: &Object) -> bool {
        o.click_action != crate::interaction::code::DISABLED
            && (self.see_through_face(g, true) || g.faces.len() < g.built_faces)
            && crate::interaction::target(world, idx, &HashMap::new()).is_some()
    }

    /// Whether an object has a face `transparent_action_point` looks at:
    /// blended (absent from the GPU depth), or hidden when `hidden` counts.
    /// By the alpha class of its texture as it is now.
    pub(super) fn see_through_face(&self, g: &ObjGpu, hidden: bool) -> bool {
        g.faces.iter().any(|f| {
            let alpha = self
                .textures
                .alpha_by_slot
                .get(f.base_slot as usize)
                .copied()
                .unwrap_or(AlphaKind::Opaque);
            match classify(f, alpha) {
                Pass::Blend => true,
                Pass::Hidden => hidden,
                _ => false,
            }
        })
    }

    /// Whether a surface of the world stands between `from` and `to`: the
    /// triangles of the objects the pick index finds along the way, apart
    /// from `skip` and from objects with a face that can be seen through
    /// (blended or hidden). Terrain, avatars and rigged meshes do not
    /// count. The rez effect's lines are only for what is seen (rez.rs).
    pub(super) fn segment_blocked(&self, world: &World, from: Vec3, to: Vec3, skip: &[usize], now: Instant) -> bool {
        let d = to - from;
        let len = d.length();
        if len < 0.1 {
            return false;
        }
        let ray = (from, d / len);
        let mut candidates = Vec::new();
        self.pick.ray_candidates(ray, &mut candidates);
        candidates.iter().any(|&idx| {
            !skip.contains(&idx)
                && self
                    .gpu
                    .get(idx)
                    .is_some_and(|g| !g.hud && ray_may_hit(g, ray) && !self.see_through_face(g, true))
                && self
                    .face_distance(world, idx, ray, now, false, FacePick::Surface)
                    .is_some_and(|t| t < len - 0.05)
        })
    }

    pub fn action_point(&self, world: &World, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, build: bool, far: f32) -> Option<Vec3> {
        if build {
            return depth;
        }
        let ray = ray?;
        let point = if self.ignored_before(world, ray, depth, far, Instant::now(), None) {
            self.point_behind_ignored_mode(world, ray, far, Instant::now(), None, FacePick::Left)
        } else {
            depth
        };
        let point = self.transparent_action_point_mode(world, ray, point, true, far, None, FacePick::Left);
        // Unlike HUDs and worn clickers, world Blend faces let alpha-zero
        // texels through (#1574). Hover keeps its existing geometric policy.
        let mut candidates = Vec::new();
        self.pick.ray_candidates(ray, &mut candidates);
        self.blend_hit_mode(world, ray, point, far, Some(&candidates), FacePick::LeftBlend)
            .map(|(t, _)| ray.0 + ray.1 * t)
            .or(point)
    }

    /// Nearest Blend surface in front of the displayed/action point. The
    /// hover search reuses its candidate list and large-face hierarchies;
    /// a full search remains available to check the index independently.
    /// Ordinary picks exclude probes, as LLToolPie::handleMouseDown and
    /// LLOctreeIntersect::check (indra/newview/llspatialpartition.cpp) do;
    /// only the selection paths honor SelectReflectionProbes.
    pub(super) fn blend_hit(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        point: Option<Vec3>,
        far: f32,
        among: Option<&[usize]>,
    ) -> Option<(f32, usize)> {
        self.blend_hit_mode(world, ray, point, far, among, FacePick::Blend)
    }

    fn blend_hit_mode(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        point: Option<Vec3>,
        far: f32,
        among: Option<&[usize]>,
        mode: FacePick,
    ) -> Option<(f32, usize)> {
        let limit = point.map_or(far, |p| (p - ray.0).dot(ray.1)).min(far);
        let now = Instant::now();
        let hit = |idx: usize| {
            let g = self.gpu.get(idx)?;
            let o = world.objects.get(idx)?;
            if g.hud
                || g.is_avatar
                || g.rigged
                || (g.owner_avatar.is_some() && o.visual_volume().is_mesh())
                || o.extra.reflection_probe.is_some()
                || o.click_action == crate::interaction::code::IGNORE
                || !ray_may_hit(g, ray)
            {
                return None;
            }
            // Skip the transform and box test for entirely non-Blend
            // objects, including the many opaque hover candidates.
            if !self.has_blend_face_mode(world, idx, mode == FacePick::LeftBlend) {
                return None;
            }
            let t = if mode == FacePick::LeftBlend {
                self.face_hit_mode(world, idx, ray, now, true, mode)?.t
            } else if among.is_some() && !self.hover.legacy() {
                self.face_distance(world, idx, ray, now, true, FacePick::Blend)?
            } else {
                self.face_hit_mode(world, idx, ray, now, true, FacePick::Blend)?.t
            };
            (t.is_finite() && t < limit).then_some((t, idx))
        };
        let hits = match among {
            Some(indices) => ray_hits(indices, |&idx| hit(idx)),
            None => {
                let all: Vec<usize> = (0..self.gpu.len()).collect();
                ray_hits(&all, |&idx| hit(idx))
            }
        };
        nearest_hit(hits, |h| *h)
    }

    pub fn interaction_at(&self, world: &World, point: Vec3, now: Instant, build: bool) -> Option<usize> {
        self.pick_at_filtered(world, point, now, !build)
    }

    /// LLViewerWindow::cursorIntersect / LLVOVolume::lineSegmentIntersect:
    /// identify the hit surface, not the smallest overlapping prim box. The
    /// depth point bounds the ray so terrain and avatars still occlude objects.
    pub fn interaction_at_ray(&self, world: &World, point: Vec3, ray: (Vec3, Vec3), include_hidden: bool) -> Option<usize> {
        self.interaction_at_ray_among(world, point, ray, include_hidden, None)
    }

    /// `interaction_at_ray` over the objects of `among` (None: all).
    pub(super) fn interaction_at_ray_among(
        &self,
        world: &World,
        point: Vec3,
        ray: (Vec3, Vec3),
        include_hidden: bool,
        among: Option<&[usize]>,
    ) -> Option<usize> {
        let now = Instant::now();
        const TOLERANCE: f32 = 0.06;
        let ray = (point - ray.1 * TOLERANCE, ray.1);
        let mut nearest = TOLERANCE * 2.0;
        let mut picked = None;
        for (idx, g) in self.pick_objects(among) {
            let Some(o) = world.objects.get(idx) else { continue };
            if (g.faces.is_empty() && g.geom.is_none())
                || g.hud
                || g.is_avatar
                // (of a worn mesh, only the invisible faces of one without a
                // skin: `worn_unrigged_mesh`)
                || (g.owner_avatar.is_some() && o.visual_volume().is_mesh() && !(include_hidden && worn_unrigged_mesh(o, g)))
                || o.extra.reflection_probe.is_some()
                || o.click_action == crate::interaction::code::IGNORE
            {
                continue;
            }
            if g.radius > 0.0 && g.center.distance_squared(point) > (g.radius + TOLERANCE).powi(2) {
                continue;
            }
            let Some(geom) = g.geom.and_then(|key| self.geom_ready(&key)) else {
                continue;
            };
            let Some((pos, rot, _)) = Self::object_transform(world, idx, now, 0) else {
                continue;
            };
            let inv = self.drawn_model(g, o, o.visual_scale_at(now), g.geom, pos, rot).inverse();
            if !ray_box(inv.transform_point3(ray.0), inv.transform_vector3(ray.1), geom.min, geom.max).is_some_and(|t| t < nearest) {
                continue;
            }
            if let Some(t) = self.hit_distance(world, idx, ray, now, include_hidden, among)
                && t < nearest
            {
                nearest = t;
                picked = Some(idx);
            }
        }
        picked
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    /// LimitSelectDistance on, at the Legacy Viewer's default distance.
    const LIMIT: Option<f32> = Some(crate::build::MAX_SELECT_DISTANCE);
    #[test]
    fn world_click_posed_attachment_cannot_intercept_with_its_bind_pose() {
        use aurora_render::{Drawn, DrawnFace};
        let (mut world, mut scene, front, _) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let displayed = Vec3::new(6.0, 0.0, 50.0);
        world.objects.get_mut(front).unwrap().update_flags = 1 << 7;
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::TOUCH;
        scene.gpu[front].rigged = true;
        // another resident's: a rigged mesh we wear never takes the touch
        scene.gpu[front].owner_avatar = world
            .objects
            .avatars()
            .find(|(_, a)| a.full_id != world.agent_id)
            .map(|(idx, _)| idx);
        scene.gpu[front].faces[0].te_alpha = 0.5;
        let hit = scene.action_point(&world, Some(ray), Some(displayed), false, 100.0);
        assert_eq!(
            hit,
            Some(displayed),
            "rest triangles at x=2.5 must not replace the posed surface at x=6"
        );
        let drawn = Drawn {
            blend: Some(DrawnFace {
                record: scene.gpu[front].faces[0].record,
                point: displayed,
                st: glam::Vec2::splat(0.5),
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            ..Default::default()
        };
        let designated = scene
            .designated(
                &world,
                super::super::drawn::Gesture::Touch,
                &drawn,
                Some(ray),
                hit,
                Some(displayed),
                false,
            )
            .unwrap();
        assert_eq!(designated.idx, front);
        assert_eq!(
            crate::interaction::target(&world, designated.idx, &HashMap::new()).unwrap().action,
            crate::interaction::Action::Touch
        );
        // Neither the reference search nor the accelerated hover may use
        // the rest geometry in the empty space before the real surface.
        assert!(scene.face_hit(&world, front, ray, Instant::now(), true).is_none());
        assert_eq!(
            scene.face_distance(&world, front, ray, Instant::now(), true, FacePick::Surface),
            None
        );
        // A fast-moving mesh is still touchable after the delayed read;
        // actual CPU geometry in front must continue to stop the press.
        let moved = Drawn {
            blend: drawn.blend.map(|f| DrawnFace {
                point: displayed + Vec3::X * 0.25,
                ..f
            }),
            ..drawn
        };
        assert!(
            scene
                .designated(
                    &world,
                    super::super::drawn::Gesture::Touch,
                    &moved,
                    Some(ray),
                    hit,
                    Some(displayed),
                    false
                )
                .is_some()
        );
        assert!(
            scene
                .designated(
                    &world,
                    super::super::drawn::Gesture::Touch,
                    &moved,
                    Some(ray),
                    Some(displayed - Vec3::X),
                    Some(displayed),
                    false
                )
                .is_none()
        );
        // Local mesh previews use the rendered volume, not the original
        // server prim, for both CPU exclusion and GPU designation.
        scene.gpu[front].rigged = false;
        let mut volume = world.objects.get(front).unwrap().volume;
        volume.sculpt = Some(aurora_prim::params::SculptParams {
            texture: Uuid::from_u128(0x416),
            sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
        });
        world.objects.get_mut(front).unwrap().render.local_volume = Some(Box::new(volume));
        assert!(scene.drawn_status(&world, front).posed_by_gpu);
        assert!(scene.face_hit(&world, front, ray, Instant::now(), true).is_none());
    }

    #[test]
    fn invisible_worn_mesh_without_a_skin_takes_the_touch() {
        // #488: a clicker worn by another avatar, a mesh without a skin
        // whose faces are all at alpha 0. The GPU never draws it.
        let (mut world, mut scene, front, _) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let behind = Vec3::new(6.5, 0.0, 50.0);
        let o = world.objects.get_mut(front).unwrap();
        o.update_flags = 1 << 7;
        o.click_action = crate::interaction::code::TOUCH;
        let mut volume = o.volume;
        volume.sculpt = Some(aurora_prim::params::SculptParams {
            texture: Uuid::from_u128(0x488),
            sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
        });
        o.render.local_volume = Some(Box::new(volume));
        scene.gpu[front].owner_avatar = world.objects.avatars().next().map(|(idx, _)| idx);
        scene.gpu[front].built_faces = scene.gpu[front].faces.len();
        // Visible, its surface is the GPU's alone.
        assert!(scene.face_hit(&world, front, ray, Instant::now(), true).is_none());
        // Invisible: no draw record, the CPU answers for its faces.
        for f in &mut Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces {
            f.color[3] = 0.0;
        }
        scene.gpu[front].faces.clear();
        assert!(scene.face_hit(&world, front, ray, Instant::now(), false).is_none());
        assert_eq!(scene.action_point(&world, Some(ray), Some(behind), false, 100.0), Some(front_point));
        assert_eq!(scene.interaction_at_ray(&world, front_point, ray, true), Some(front));
        // The right-click menu reaches it too (pick_transparent).
        scene.gpu[front].center = Vec3::new(2.0, 0.0, 50.0);
        scene.gpu[front].radius = 1.0;
        scene.pick.set(front, super::super::hover::PickEntry::of(&scene.gpu[front]));
        assert_eq!(
            scene.selection_pick(&world, Some(ray), Some(behind), false, false, 100.0),
            (Some(front_point), Some(front))
        );
        // Hover does not see it, and with nothing to do it stops nothing.
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(behind), false, 100.0), Some(behind));
        let o = world.objects.get_mut(front).unwrap();
        o.update_flags = 0;
        o.click_action = crate::interaction::code::NONE;
        assert_eq!(scene.action_point(&world, Some(ray), Some(behind), false, 100.0), Some(behind));
        // A rigged mesh stays out of the CPU's reach (bind pose).
        scene.gpu[front].rigged = true;
        assert!(scene.face_hit(&world, front, ray, Instant::now(), true).is_none());
    }

    #[test]
    fn sphere_test_keeps_what_the_ray_can_reach() {
        let g = |center: Vec3, radius: f32| super::super::ObjGpu {
            center,
            radius,
            ..Default::default()
        };
        let ray = (Vec3::ZERO, Vec3::X);
        assert!(ray_may_hit(&g(Vec3::new(10.0, 0.5, 0.0), 1.0), ray));
        assert!(!ray_may_hit(&g(Vec3::new(10.0, 3.0, 0.0), 1.0), ray));
        // behind the eye, but the eye is inside it
        assert!(ray_may_hit(&g(Vec3::new(-0.5, 0.0, 0.0), 1.0), ray));
        assert!(!ray_may_hit(&g(Vec3::new(-5.0, 0.0, 0.0), 1.0), ray));
        // no bounds yet, rigged: never rejected
        assert!(ray_may_hit(&g(Vec3::new(0.0, 50.0, 0.0), 0.0), ray));
        let mut rigged = g(Vec3::new(0.0, 50.0, 0.0), 1.0);
        rigged.rigged = true;
        assert!(ray_may_hit(&rigged, ray));
    }

    #[test]
    fn box_rejects_misses_and_parallel_rays() {
        assert_eq!(ray_box(Vec3::new(-2.0, 0.0, 0.0), Vec3::X, -Vec3::ONE, Vec3::ONE), Some(1.0));
        assert!(ray_box(Vec3::new(-2.0, 3.0, 0.0), Vec3::X, -Vec3::ONE, Vec3::ONE).is_none());
        assert!(ray_box(Vec3::new(-2.0, 0.0, 0.0), -Vec3::X, -Vec3::ONE, Vec3::ONE).is_none());
    }
    #[test]
    fn movement_pass_finds_free_space_and_rejects_blocked_or_phantom_obstacles() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        for (idx, x) in [(front, 129.0), (back, 137.0)] {
            let object = world.objects.get_mut(idx).expect("object");
            object.position = Vec3::new(x, 128.0, 50.0);
            object.update_flags = 0;
            scene.gpu[idx].center = object.position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let origin = Vec3::new(128.0, 128.0, 50.0);
        assert!(scene.movement_obstacle(&world, origin, Vec3::X));
        let exit = scene.movement_exit(&world, origin, Vec3::X, 1.7).expect("clear far side");
        assert!(
            exit.distance(Vec3::new(130.5, 128.0, 50.0)) < 0.0001,
            "leave one metre beyond the wall's far face at x=129.5"
        );
        for distance in [1.6, 2.5, 3.0] {
            let approach = origin.with_x(128.5 - distance);
            let farther_exit = scene.movement_exit(&world, approach, Vec3::X, 1.7).expect("wall in approach range");
            assert!(
                farther_exit.distance(exit) < 0.0001,
                "the approach distance must not move the exit farther away"
            );
        }
        assert!(
            scene.movement_exit(&world, origin.with_x(125.4), Vec3::X, 1.7).is_none(),
            "wall beyond three metres"
        );
        world.objects.get_mut(back).expect("back").position.x = exit.x;
        assert!(scene.movement_exit(&world, origin, Vec3::X, 1.7).is_none(), "occupied destination");
        world.objects.get_mut(front).expect("front").update_flags |= crate::build::edits::FLAGS_PHANTOM;
        assert!(!scene.movement_obstacle(&world, origin, Vec3::X));
        assert!(
            scene.movement_exit(&world, origin, Vec3::X, 1.7).is_some(),
            "solid obstacle behind the phantom is now within reach"
        );
        world.objects.get_mut(back).expect("back").position.x = 137.0;
        assert!(scene.movement_exit(&world, origin, Vec3::X, 1.7).is_none());
    }
    #[test]
    fn swim_landing_finds_a_walkable_top_and_rejects_phantom_and_low_ceiling() {
        for ceiling in [false, true] {
            let (mut world, mut scene, front, back) = scene_with_two_objects();
            for (idx, x) in [(front, 129.0), (back, 137.0)] {
                let object = world.objects.get_mut(idx).expect("object");
                object.position = Vec3::new(x, 128.0, 50.0);
                object.update_flags = 0;
                scene.gpu[idx].center = object.position;
                scene.gpu[idx].radius = 2.0;
                scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
            }
            let key = scene.gpu[front].geom.expect("geometry");
            let GeomState::Ready(geometry) = &mut scene.geoms.get_mut(&key).expect("geometry").state else {
                panic!("ready geometry");
            };
            // One solid front and its horizontal top; the optional low
            // underside leaves insufficient avatar-sized landing room.
            let mut vertices = vec![
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ];
            let mut indices = vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7];
            if ceiling {
                vertices.extend([[-0.5, -0.5, 1.2], [0.5, -0.5, 1.2], [0.5, 0.5, 1.2], [-0.5, 0.5, 1.2]]);
                indices.extend([8, 10, 9, 8, 11, 10]);
            }
            let uvs = vec![[0.0; 2]; vertices.len()];
            let geometry = Arc::get_mut(geometry).expect("unshared test geometry");
            geometry.max.z = if ceiling { 1.2 } else { 0.5 };
            geometry.pick_faces[0] = Some(PickFace::new(vertices, uvs, indices));
            let origin = Vec3::new(128.0, 128.0, 50.0);
            assert!(scene.movement_obstacle(&world, origin, Vec3::X));
            let landing = scene.swim_landing(&world, origin, Vec3::X, 1.7);
            if ceiling {
                assert!(landing.is_none(), "do not guide the avatar under a low ceiling");
            } else {
                let landing = landing.expect("clear top");
                assert!((128.5..129.5).contains(&landing.x));
                assert!((landing.z - 51.43).abs() < 0.01);
                world.objects.get_mut(front).expect("front").update_flags |= crate::build::edits::FLAGS_PHANTOM;
                assert!(!scene.movement_obstacle(&world, origin, Vec3::X));
                assert!(scene.swim_landing(&world, origin, Vec3::X, 1.7).is_none());
            }
        }
    }

    pub(in crate::scene) fn scene_with_two_objects() -> (World, Scene, usize, usize) {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            world.apply(ev);
        }
        let front = world.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let back = world.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        for (idx, x) in [(front, 3.0), (back, 7.0)] {
            let o = world.objects.get_mut(idx).unwrap();
            o.position = Vec3::new(x, 0.0, 50.0);
            o.rotation = Quat::IDENTITY;
            o.scale = Vec3::ONE;
        }
        let mut scene = Scene::new(std::path::PathBuf::new(), lib);
        scene.gpu.resize_with(front.max(back) + 1, Default::default);
        let key = GeomKey::Prim { hash: 0, lod: 0 };
        scene.geoms.insert(
            key,
            GeomEntry {
                refs: 2,
                zero_since: 0,
                state: GeomState::Ready(Arc::new(GpuGeom {
                    faces: vec![Some(aurora_render::MeshAlloc {
                        vertex_offset: 0,
                        vertex_count: 4,
                        index_offset: 0,
                        index_count: 6,
                    })],
                    min: -Vec3::splat(0.5),
                    max: Vec3::splat(0.5),
                    joint_bounds: Vec::new(),
                    pick_faces: vec![Some(PickFace::new(
                        vec![[-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [-0.5, -0.5, 0.5]],
                        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                        vec![0, 2, 1, 0, 3, 2],
                    ))],
                })),
            },
        );
        let face = FaceDraw {
            te_index: 0,
            record: 0,
            cmd: DrawCmd {
                index_count: 6,
                first_index: 0,
                base_vertex: 0,
                record: 0,
                bounds: [0.0; 4],
            },
            base_slot: 0,
            tex_id: Uuid::nil(),
            aux_tex: [Uuid::nil(); 3],
            te_alpha: 1.0,
            pbr_alpha: None,
            legacy_alpha: None,
            two_sided: false,
            repeats: 1.0,
            glow: false,
            pass: super::Pass::Opaque,
            glow_pool: false,
            rez_glow: false,
            system: None,
            first_person_hidden: false,
        };
        for idx in [front, back] {
            scene.gpu[idx].geom = Some(key);
            scene.gpu[idx].faces.push(face);
        }
        (world, scene, front, back)
    }

    #[test]
    fn cannonball_checks_the_body_footprint_above_water_and_platform_support() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        for (idx, position) in [(front, Vec3::new(0.0, 0.0, 22.0)), (back, Vec3::new(10.0, 0.0, 22.0))] {
            let object = world.objects.get_mut(idx).unwrap();
            object.position = position;
            object.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
            object.update_flags = 0;
            scene.gpu[idx].center = position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let mut frame = crate::movement::Frame {
            position: Vec3::new(0.0, 0.0, 26.0),
            velocity: Vec3::ZERO,
            height: 1.7,
            water: Some(20.0),
            water_contact: None,
            ground: Some(16.0),
            flying: true,
            can_fly: true,
            seated: false,
            suspended: false,
            locked: false,
            bridge_ready: true,
            running: false,
            obstacle: false,
            landing: None,
            on_ground: false,
            jumping: false,
        };
        assert!(!scene.cannonball_clear(&world, &frame), "solid pontoon below flight");
        frame.position.x = 0.65;
        assert!(
            !scene.cannonball_clear(&world, &frame),
            "body overlaps the deck edge despite a clear centre ray"
        );
        frame.position.x = 1.0;
        assert!(scene.cannonball_clear(&world, &frame), "open water beside the deck");
        frame.position = Vec3::new(0.0, 0.0, 23.35);
        assert!(
            scene.movement_support(&world, frame.position, frame.height),
            "support independent of native Falling"
        );
        assert!(
            !scene.movement_support(&world, frame.position + Vec3::Z, frame.height),
            "hovering above a deck is not contact"
        );
        world.objects.get_mut(front).unwrap().update_flags = crate::build::edits::FLAGS_PHANTOM;
        assert!(scene.cannonball_clear(&world, &frame));
        assert!(!scene.movement_support(&world, frame.position, frame.height));
        let object = world.objects.get_mut(front).unwrap();
        object.update_flags = 0;
        object.position.z = 18.0;
        scene.gpu[front].center = object.position;
        scene.pick.set(front, super::super::hover::PickEntry::of(&scene.gpu[front]));
        assert!(scene.cannonball_clear(&world, &frame), "a submerged floor must not prevent entry");
    }

    #[test]
    fn predicted_water_jump_rejects_a_solid_wall_but_ignores_phantoms() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        for idx in [front, back] {
            let object = world.objects.get_mut(idx).unwrap();
            object.update_flags = 0;
            object.scale = Vec3::new(1.0, 6.0, 12.0);
            scene.gpu[idx].center = object.position;
            scene.gpu[idx].radius = 8.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let frame = crate::movement::Frame {
            position: Vec3::new(0.0, 0.0, 50.0),
            velocity: Vec3::new(4.0, 0.0, 4.5),
            height: 1.8,
            water: Some(46.0),
            water_contact: None,
            ground: Some(40.0),
            flying: false,
            can_fly: true,
            seated: false,
            suspended: false,
            locked: false,
            bridge_ready: true,
            running: false,
            obstacle: false,
            landing: None,
            on_ground: true,
            jumping: true,
        };
        let horizontal = frame.velocity.with_z(0.0);
        let landing = crate::movement::water_jump::predict(&frame, horizontal, |_| Some(40.0)).expect("deep water");
        assert!(!scene.water_jump_clear(&world, &frame, horizontal, landing));
        for idx in [front, back] {
            world.objects.get_mut(idx).unwrap().update_flags = crate::build::edits::FLAGS_PHANTOM;
        }
        assert!(scene.water_jump_clear(&world, &frame, horizontal, landing));
    }

    #[test]
    fn local_mesh_geometry_uses_an_overlay_and_preserves_server_shape() {
        let (mut world, _, front, _) = scene_with_two_objects();
        let object = world.objects.get_mut(front).expect("object");
        let original = object.volume;
        let original_scale = object.scale;
        let asset = Uuid::new_v4();
        let mut volume = original;
        volume.sculpt = Some(aurora_prim::params::SculptParams {
            texture: asset,
            sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
        });
        object.render.local_volume = Some(Box::new(volume));
        object.render.local_scale = Some(Vec3::splat(2.0));
        assert_eq!(Scene::object_geom_key(object, 3), Some(GeomKey::Mesh { id: asset, lod: 3 }));
        assert_eq!(object.volume, original);
        assert_eq!(object.scale, original_scale);
        assert_eq!(object.visual_scale(), Vec3::splat(2.0));
        object.render.local_volume = None;
        object.render.local_scale = None;
        assert!(matches!(Scene::object_geom_key(object, 3), Some(GeomKey::Prim { .. })));
        assert_eq!(object.visual_scale(), original_scale);
    }

    #[test]
    fn color_pipette_samples_the_nearest_face_tint_without_lighting_or_modify_rights() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        for (idx, rgb) in [(front, [0.1, 0.2, 0.3]), (back, [0.4, 0.5, 0.6])] {
            let o = world.objects.get_mut(idx).unwrap();
            o.update_flags = 0;
            let te = Arc::make_mut(o.te.as_mut().unwrap());
            te.faces[0].color = [rgb[0], rgb[1], rgb[2], 1.0];
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        assert_eq!(
            scene.color_at_ray(&world, ray, false, 100.0),
            Some((world.objects.get(front).unwrap().full_id, [0.1, 0.2, 0.3]))
        );
        scene.gpu[front].rigged = true;
        assert_eq!(
            scene.color_at_ray(&world, ray, false, 100.0),
            Some((world.objects.get(back).unwrap().full_id, [0.4, 0.5, 0.6]))
        );
        assert_eq!(scene.color_at_ray(&world, ray, false, 6.0), None);
        assert_eq!(scene.color_at_ray(&world, (ray.0 + Vec3::Y * 2.0, ray.1), false, 100.0), None);
        assert_eq!(scene.color_at_ray(&world, ray, true, 100.0), None);
        scene.gpu[front].rigged = false;
        scene.gpu[front].hud = true;
        scene.gpu[front].owner_avatar = world.objects.index_of_uuid(&world.agent_id);
        assert_eq!(
            scene.color_at_ray(&world, ray, true, 100.0),
            Some((world.objects.get(front).unwrap().full_id, [0.1, 0.2, 0.3]))
        );
    }

    #[test]
    fn reflection_probes_do_not_intercept_actions_or_hover() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let back_point = Vec3::new(6.5, 0.0, 50.0);
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::TOUCH;
        world.objects.get_mut(back).unwrap().update_flags = 1 << 7;
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        // Ordinary transparent surfaces still stop clicks, even without an action.
        scene.gpu[front].faces[0].te_alpha = 0.4;
        let point = scene.action_point(&world, Some(ray), Some(back_point), false, 100.0).unwrap();
        assert_eq!(point, front_point);
        assert_eq!(scene.interaction_at_ray(&world, point, ray, true), Some(front));

        for flags in [0, 1, 1 | 4] {
            world.objects.get_mut(front).unwrap().extra.reflection_probe = Some(aurora_prim::extra::ReflectionProbeParams {
                flags,
                ..crate::build::edits::PROBE_DEFAULT
            });
            // Both non-actionable and scripted probes must let the click through.
            for action in [crate::interaction::code::TOUCH, crate::interaction::code::IGNORE] {
                for scripted in [false, true] {
                    let probe = world.objects.get_mut(front).unwrap();
                    probe.click_action = action;
                    probe.update_flags = if scripted { 1 << 7 } else { 0 };
                    for alpha in [0.0, 0.4] {
                        scene.gpu[front].faces[0].te_alpha = alpha;
                        scene.pick.changed(front);
                        let point = scene.action_point(&world, Some(ray), Some(back_point), false, 100.0).unwrap();
                        assert_eq!(
                            point, back_point,
                            "probe flags={flags}, action={action}, scripted={scripted}, alpha={alpha}"
                        );
                        assert_eq!(scene.interaction_at_ray(&world, point, ray, true), Some(back));
                        assert_eq!(
                            crate::interaction::target(&world, back, &HashMap::new()).unwrap().action,
                            crate::interaction::Action::Touch
                        );
                        assert_eq!(
                            scene.hover_pick(&world, Some(ray), Some(back_point), 100.0),
                            (Some(back_point), Some(back))
                        );
                        assert_eq!(
                            scene.interaction_point(&world, Some(ray), Some(back_point), false, 100.0),
                            Some(back_point)
                        );
                        assert_eq!(scene.blend_hit(&world, ray, Some(back_point), 100.0, None), None);
                        assert_eq!(
                            scene.point_behind_ignored(&world, ray, 100.0, Instant::now(), None),
                            Some(back_point)
                        );
                        assert_eq!(scene.point_behind_rigged(&world, ray, 0.0, 100.0, None), Some(back_point));
                        // Foreground depth still occludes the scripted cube.
                        let occluder = Vec3::new(1.0, 0.0, 50.0);
                        assert_eq!(scene.action_point(&world, Some(ray), Some(occluder), false, 100.0), Some(occluder));
                        assert_eq!(scene.interaction_at_ray(&world, front_point, ray, true), None);
                    }
                }
            }
        }
        // Removing the probe parameters restores an ordinary Blend surface.
        let object = world.objects.get_mut(front).unwrap();
        object.extra.reflection_probe = None;
        object.click_action = crate::interaction::code::TOUCH;
        scene.pick.changed(front);
        assert_eq!(
            scene.hover_pick(&world, Some(ray), Some(back_point), 100.0),
            (Some(front_point), Some(front))
        );
    }

    #[test]
    fn reflection_probe_selection_toggle_passes_through_invisible_probes_for_menu_edit_and_wheel() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        world.agent.position = ray.0;
        let probe = world.objects.get_mut(front).unwrap();
        probe.extra.reflection_probe = Some(crate::build::edits::PROBE_DEFAULT);
        for face in &mut Arc::make_mut(probe.te.as_mut().unwrap()).faces {
            face.color[3] = 0.0;
        }
        scene.gpu[front].faces.clear();
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let back_point = Vec3::new(6.5, 0.0, 50.0);
        for enabled in [false, true, false] {
            let expected = if enabled { (front_point, front) } else { (back_point, back) };
            for build in [false, true] {
                for depth in [Some(back_point), None] {
                    assert_eq!(
                        scene.selection_pick(&world, Some(ray), depth, build, enabled, 100.0),
                        (Some(expected.0), Some(expected.1))
                    );
                }
                // An actual foreground surface still occludes the probe.
                let occluder = Vec3::new(1.0, 0.0, 50.0);
                assert_eq!(
                    scene.selection_pick(&world, Some(ray), Some(occluder), build, enabled, 100.0),
                    (Some(occluder), None)
                );
            }
            assert_eq!(
                scene.edit_ray_hits(&world, ray, false, enabled, LIMIT),
                if enabled { vec![front, back] } else { vec![back] }
            );
        }
        // A visible probe cannot reappear through the depth-only fallback.
        let face = scene.gpu[back].faces[0];
        scene.gpu[front].faces.push(face);
        assert_eq!(scene.selection_pick(&world, None, Some(front_point), true, false, 100.0).1, None);
        assert_eq!(
            scene.selection_pick(&world, None, Some(front_point), true, true, 100.0).1,
            Some(front)
        );
    }

    #[test]
    fn edit_stack_reaches_occluded_and_ignored_prims_and_uses_triangles() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        world.agent.position = Vec3::new(0.0, 0.0, 50.0);
        for idx in [front, back] {
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (world.agent.position, Vec3::X);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        assert_eq!(scene.edit_ray_hits(&world, ray, false, false, LIMIT), [front, back]);
        // Bounds still cover this corner, but the only remaining triangle does not.
        let key = scene.gpu[front].geom.unwrap();
        let GeomState::Ready(geom) = &mut scene.geoms.get_mut(&key).unwrap().state else {
            panic!("ready geometry")
        };
        Arc::get_mut(geom).unwrap().pick_faces[0].as_mut().unwrap().indices = vec![0, 2, 1];
        let ray = (Vec3::new(0.0, -0.4, 50.4), Vec3::X);
        assert!(scene.edit_ray_hits(&world, ray, false, false, LIMIT).is_empty());
        assert!(
            scene
                .edit_ray_hits(&world, (Vec3::new(0.0, 2.0, 50.0), Vec3::X), false, false, LIMIT)
                .is_empty()
        );
    }

    #[test]
    fn edit_stack_respects_select_distance_and_keeps_hud_space_separate() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        world.agent.position = ray.0;
        for idx in [front, back] {
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        world.objects.get_mut(back).unwrap().position.x = 200.0;
        assert_eq!(scene.edit_ray_hits(&world, ray, false, false, LIMIT), [front]);
        // LimitSelectDistance off: the far prim stays on the stack
        assert_eq!(scene.edit_ray_hits(&world, ray, false, false, None), [front, back]);
        world.objects.get_mut(back).unwrap().position.x = 7.0;
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        scene.gpu[front].hud = true;
        scene.gpu[front].owner_avatar = Some(own);
        assert_eq!(scene.edit_ray_hits(&world, ray, false, false, LIMIT), [back]);
        assert_eq!(scene.edit_ray_hits(&world, ray, true, false, LIMIT), [front]);
        scene.gpu[front].owner_avatar = Some(back);
        assert!(scene.edit_ray_hits(&world, ray, true, false, LIMIT).is_empty());
    }

    #[test]
    fn hud_back_faces_do_not_block_visible_buttons_unless_material_is_double_sided() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let parent = world.objects.get(own).unwrap().key.local_id;
        for (idx, x) in [(front, 3.0), (back, 7.0)] {
            let o = world.objects.get_mut(idx).unwrap();
            o.parent_id = parent;
            o.state = 31u8.rotate_left(4);
            o.position = Vec3::new(x, 0.0, 0.0);
            let g = &mut scene.gpu[idx];
            g.hud = true;
            g.owner_avatar = Some(own);
            g.center = o.position;
            g.radius = 1.0;
        }
        world.objects.get_mut(front).unwrap().rotation = Quat::from_rotation_z(std::f32::consts::PI);
        scene.index_objects();
        scene.build_huds(&world, [1280, 720], true);
        let cursor = (640.0, 360.0);
        for hidden in [false, true] {
            assert_eq!(scene.hud_pick(&world, cursor, hidden).unwrap().0, back);
        }
        assert_eq!(scene.hud_edit_pick(&world, cursor).unwrap().0, back);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        assert!(
            scene
                .touch_surface(&world, front, scene.lists.hud_view.unwrap().ray(cursor.0, cursor.1))
                .is_none()
        );

        let mat = Uuid::from_u128(42);
        world.objects.get_mut(front).unwrap().extra.render_materials = vec![(0, mat)];
        scene.materials.insert(
            mat,
            aurora_assets::PbrMaterial {
                double_sided: true,
                ..Default::default()
            },
        );
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, front);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(front));
        let key = world.objects.get(front).unwrap().key;
        world.gltf_overrides.insert(
            key,
            vec![(
                0,
                aurora_assets::material::PbrOverride {
                    double_sided: Some(false),
                    ..Default::default()
                },
            )]
            .into(),
        );
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, back);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        world.gltf_overrides.insert(
            key,
            vec![(
                0,
                aurora_assets::material::PbrOverride {
                    double_sided: Some(true),
                    ..Default::default()
                },
            )]
            .into(),
        );
        scene.materials.insert(mat, aurora_assets::PbrMaterial::default());
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(front));
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, front);
        world.objects.get_mut(front).unwrap().render.blinn_phong_preview = true;
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, back);
        world.objects.get_mut(front).unwrap().render.blinn_phong_preview = false;
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, front);
    }

    #[test]
    fn hud_left_clicks_ignore_alpha_while_move_picks_respect_texture_holes() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let parent = world.objects.get(own).unwrap().key.local_id;
        for (idx, x) in [(front, 3.0), (back, 7.0)] {
            let o = world.objects.get_mut(idx).unwrap();
            o.parent_id = parent;
            o.state = 31u8.rotate_left(4);
            o.position = Vec3::new(x, 0.0, 0.0);
            let g = &mut scene.gpu[idx];
            g.hud = true;
            g.owner_avatar = Some(own);
            g.center = o.position;
            g.radius = 1.0;
        }
        scene.index_objects();
        scene.build_huds(&world, [1280, 720], true);
        let cursor = (640.0, 360.0);
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        let texture = Uuid::from_u128(8888);
        scene.gpu[front].faces[0].tex_id = texture;
        scene.gpu[front].faces[0].pass = Pass::Blend;
        scene.textures.alpha_by_slot = vec![AlphaKind::Blend];
        scene
            .textures
            .pick_masks
            .insert(texture, super::super::textures::PickMask::from_rgba(2, 2, &[0; 16]).unwrap());
        for (pbr, legacy, texture_alpha) in [
            (None, None, AlphaKind::Blend),
            (Some(1), None, AlphaKind::Blend),
            (None, Some(legacy_mat::alpha_mode::BLEND), AlphaKind::Blend),
            (None, None, AlphaKind::Mask),
            (Some(2), None, AlphaKind::Blend),
            (None, Some(legacy_mat::alpha_mode::MASK), AlphaKind::Blend),
        ] {
            scene.textures.alpha_by_slot[0] = texture_alpha;
            for (alpha, texel_alpha) in [1.0, 0.5, 0.001, 0.0].into_iter().flat_map(|a| [0, 127, 255].map(|t| (a, t))) {
                scene.textures.pick_masks.insert(
                    texture,
                    super::super::textures::PickMask::from_rgba(1, 1, &[255, 255, 255, texel_alpha]).unwrap(),
                );
                let draw = &mut scene.gpu[front].faces[0];
                draw.pbr_alpha = pbr;
                draw.legacy_alpha = legacy;
                draw.te_alpha = alpha;
                draw.pass = if alpha < 0.004 {
                    Pass::Hidden
                } else if pbr == Some(2) || legacy == Some(legacy_mat::alpha_mode::MASK) || texture_alpha == AlphaKind::Mask {
                    Pass::Mask
                } else {
                    Pass::Blend
                };
                Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].color[3] = alpha;
                let (idx, _, ray) = scene.hud_pick(&world, cursor, false).unwrap();
                assert_eq!(idx, front, "material {pbr:?}/{legacy:?}, alpha {alpha}, texel {texel_alpha}");
                let touch = scene.touch_surface(&world, idx, ray).unwrap();
                assert_eq!(scene.left_touch_surface(&world, idx, ray), Some(touch));
                assert_eq!(touch.face, 0);
                assert!(touch.st.abs_diff_eq(Vec3::new(0.5, 0.5, 0.0), 1e-5));
                assert_eq!(scene.hover_hud(&world, cursor), Some(front));
                let moved = if alpha >= 0.004 && texel_alpha > 0 { front } else { back };
                assert_eq!(scene.hud_move_pick(&world, cursor), Some(moved));
                assert_eq!(scene.hud_edit_pick(&world, cursor).unwrap().0, front);
            }
        }
        scene.gpu[front].faces[0].te_alpha = 1.0;
        scene.gpu[front].faces[0].pass = Pass::Blend;
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].color[3] = 1.0;
        scene.gpu[front].faces[0].legacy_alpha = None;
        scene.gpu[front].faces[0].pbr_alpha = None;
        scene.textures.alpha_by_slot[0] = AlphaKind::Blend;
        // Follow repeats/offsets in HUD space, rather than the raw mesh UV.
        let mut pixels = vec![255; 4 * 4 * 4];
        for y in 0..4 {
            for x in 2..4 {
                pixels[(y * 4 + x) * 4 + 3] = 0;
            }
        }
        scene
            .textures
            .pick_masks
            .insert(texture, super::super::textures::PickMask::from_rgba(4, 4, &pixels).unwrap());
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].offset_s = 0.5;
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(front));
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].offset_s = 0.0;
        // Opaque materials display the texture without its alpha channel.
        scene.gpu[front].faces[0].pbr_alpha = Some(0);
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(front));
        scene.gpu[front].faces[0].pbr_alpha = Some(2);
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hud_edit_pick(&world, cursor).unwrap().0, front);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        scene.gpu[front].faces[0].pbr_alpha = None;
        scene.textures.alpha_by_slot[0] = AlphaKind::Mask;
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        scene.textures.alpha_by_slot[0] = AlphaKind::Blend;
        scene.gpu[front].faces[0].legacy_alpha = Some(legacy_mat::alpha_mode::MASK);
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        scene.gpu[front].faces[0].legacy_alpha = None;
        scene.gpu[front].faces[0].pbr_alpha = Some(2);
        scene.build_huds(&world, [1280, 720], true);
        assert!(scene.lists.hud_debug_blue.is_empty());
        // The overlay's displayed triangles become pickable, with no texture test.
        scene.debug_alpha = Some(false);
        scene.build_huds(&world, [1280, 720], true);
        assert_eq!(scene.lists.hud_debug_red.len(), 1);
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        scene.gpu[front].faces[0].pass = Pass::Mask;
        scene.build_huds(&world, [1280, 720], true);
        assert_eq!(scene.lists.hud_debug_blue.len(), 1);
        assert!(scene.lists.hud_debug_red.is_empty());
        scene.debug_alpha = None;
        scene.build_huds(&world, [1280, 720], true);
        assert!(scene.lists.hud_debug_blue.is_empty());
        // Faces that draw nothing still take ordinary clicks: a zero alpha,
        // or the library's transparent texture behind a Mask-classed slot.
        scene.gpu[front].faces[0].te_alpha = 0.0;
        scene.textures.pick_masks.remove(&texture);
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        scene.gpu[front].faces[0].te_alpha = 1.0;
        scene.gpu[front].faces[0].pbr_alpha = None;
        scene.gpu[front].faces[0].pass = Pass::Hidden;
        scene.textures.alpha_by_slot[0] = AlphaKind::Mask;
        scene
            .textures
            .pick_masks
            .insert(texture, super::super::textures::PickMask::from_rgba(2, 2, &[0; 16]).unwrap());
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hover_hud(&world, cursor), Some(front));
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        scene.textures.pick_masks.remove(&texture);

        // A Blend material can be drawn in the opaque pool when its texture
        // is opaque. The material mode still determines button interaction.
        scene.gpu[front].faces[0].te_alpha = 1.0;
        scene.gpu[front].faces[0].legacy_alpha = Some(legacy_mat::alpha_mode::BLEND);
        scene.gpu[front].faces[0].pbr_alpha = None;
        scene.gpu[front].faces[0].pass = Pass::Opaque;
        scene.textures.alpha_by_slot[0] = AlphaKind::Opaque;
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);

        // Zero TE alpha omits all the draw records. Keep the CPU triangles
        // for ordinary clicks and hover, whatever the material mode.
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].color[3] = 0.0;
        scene.gpu[front].faces.clear();
        scene.build_huds(&world, [1280, 720], true);
        let (_, _, ray) = scene.hud_pick(&world, cursor, false).unwrap();
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hover_hud(&world, cursor), Some(front));
        assert_eq!(scene.touch_surface(&world, front, ray).unwrap().face, 0);
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        let legacy_material = Uuid::from_u128(8889);
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].material_id = legacy_material;
        for mode in [legacy_mat::alpha_mode::BLEND, legacy_mat::alpha_mode::MASK] {
            scene.legacy_mats.insert(
                legacy_material,
                legacy_mat::LegacyMaterial {
                    diffuse_alpha_mode: mode,
                    alpha_cutoff: 128,
                    ..Default::default()
                },
            );
            assert_eq!(
                scene.hud_pick(&world, cursor, false).unwrap().0,
                front,
                "omitted legacy face: {mode}"
            );
            assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        }
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].material_id = Uuid::nil();
        let material = Uuid::from_u128(8890);
        let object = world.objects.get_mut(front).unwrap();
        object.extra.render_materials.push((0, material));
        let key = object.key;
        scene.materials.insert(material, aurora_assets::PbrMaterial::default());
        for mode in [
            aurora_assets::AlphaMode::Blend,
            aurora_assets::AlphaMode::Mask,
            aurora_assets::AlphaMode::Opaque,
        ] {
            world.gltf_overrides.insert(
                key,
                vec![(
                    0,
                    aurora_assets::material::PbrOverride {
                        alpha_mode: Some(mode),
                        ..Default::default()
                    },
                )]
                .into(),
            );
            assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front, "omitted face: {mode:?}");
            assert_eq!(scene.hover_hud(&world, cursor), Some(front));
            assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        }
    }

    #[test]
    fn scaled_hud_pick_and_uvs_follow_the_visual_but_script_coordinates_stay_original() {
        let (mut world, mut scene, front, _) = scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let parent = world.objects.get(own).unwrap().key.local_id;
        let o = world.objects.get_mut(front).unwrap();
        o.parent_id = parent;
        o.state = 31u8.rotate_left(4);
        o.position = Vec3::new(3.0, 0.0, 0.0);
        o.te = None;
        let id = o.full_id;
        scene.gpu[front].hud = true;
        scene.gpu[front].owner_avatar = Some(own);
        let original_ray = (Vec3::new(0.0, 0.4, 0.2), Vec3::X);
        let original = scene.touch_surface(&world, front, original_ray).unwrap();
        let enlarged_ray = (Vec3::new(0.0, 0.8, 0.4), Vec3::X);
        assert!(scene.touch_surface(&world, front, enlarged_ray).is_none());
        world.hud_scales.set(id, 2.0);
        let enlarged = scene.touch_surface(&world, front, enlarged_ray).unwrap();
        assert!(enlarged.st.abs_diff_eq(original.st, 1e-5));
        assert!(enlarged.uv.abs_diff_eq(original.uv, 1e-5));
        assert!(enlarged.position.abs_diff_eq(original.position, 1e-5));
        world.hud_scales.set_offset(id, Vec3::new(0.0, 0.1, 0.2));
        let shifted = scene
            .touch_surface(&world, front, (enlarged_ray.0 + Vec3::new(0.0, 0.1, 0.2), Vec3::X))
            .unwrap();
        assert!(shifted.position.abs_diff_eq(original.position, 1e-5));
    }

    #[test]
    fn inventory_placement_uses_triangles_and_excludes_texture_holes_rigged_and_hud_faces() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let mut candidates = Vec::new();
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        let hit = scene.inventory_surface(&world, ray, 20.0, &mut candidates).unwrap();
        assert_eq!(hit.1, Some(front));
        assert!((hit.0.x - 2.5).abs() < 1e-5);
        // Touch-only IGNORE rules must not affect inventory placement.
        scene.gpu[front].faces[0].pass = Pass::Blend;
        let texture = Uuid::from_u128(8888);
        scene.gpu[front].faces[0].tex_id = texture;
        scene
            .textures
            .pick_masks
            .insert(texture, super::super::textures::PickMask::from_rgba(2, 2, &[0; 16]).unwrap());
        assert_eq!(scene.inventory_surface(&world, ray, 20.0, &mut candidates).unwrap().1, Some(back));
        scene.gpu[front].faces[0].pass = Pass::Opaque;
        scene.textures.pick_masks.remove(&texture);
        scene.gpu[front].rigged = true;
        assert_eq!(scene.inventory_surface(&world, ray, 20.0, &mut candidates).unwrap().1, Some(back));
        scene.gpu[front].rigged = false;
        scene.gpu[front].hud = true;
        assert_eq!(scene.inventory_surface(&world, ray, 20.0, &mut candidates).unwrap().1, Some(back));
        assert!(scene.inventory_surface(&world, ray, 2.0, &mut candidates).is_none());
        let down = (Vec3::new(140.0, 130.0, 60.0), -Vec3::Z);
        let terrain = scene.inventory_surface(&world, down, 50.0, &mut candidates).unwrap();
        assert!(terrain.1.is_none());
        assert!((terrain.0.z - world.ground_height(terrain.0).unwrap()).abs() < 0.001);
    }

    #[test]
    fn creation_placement_ignores_invisible_surfaces_and_never_targets_flora_or_avatars() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let mut candidates = Vec::new();
        assert_eq!(scene.creation_surface(&world, ray, 20.0, &mut candidates).unwrap().1, Some(front));
        let pcode = world.objects.get(front).unwrap().pcode;
        for code in [
            aurora_prim::params::LL_PCODE_LEGACY_AVATAR,
            aurora_prim::params::LL_PCODE_LEGACY_TREE,
        ] {
            world.objects.get_mut(front).unwrap().pcode = code;
            assert_eq!(scene.creation_surface(&world, ray, 20.0, &mut candidates).unwrap().1, Some(back));
        }
        world.objects.get_mut(front).unwrap().pcode = pcode;
        scene.gpu[front].faces[0].pass = Pass::Hidden;
        assert_eq!(scene.creation_surface(&world, ray, 20.0, &mut candidates).unwrap().1, Some(back));
        assert!(scene.creation_surface(&world, ray, 2.0, &mut candidates).is_none());
        let down = (Vec3::new(140.0, 130.0, 60.0), -Vec3::Z);
        let (point, target) = scene.creation_surface(&world, down, 50.0, &mut candidates).unwrap();
        assert!(target.is_none());
        assert!((point.z - world.ground_height(point).unwrap()).abs() < 0.001);
    }

    #[test]
    fn inventory_alpha_uv_matches_planar_projection_and_pbr_overrides() {
        let (mut world, mut scene, front, _) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, -0.15, 50.15), Vec3::X);
        let now = Instant::now();
        let model = Mat4::from_translation(world.objects.get(front).unwrap().position);
        let hit = scene.face_hit(&world, front, ray, now, true).unwrap();
        let uv = |world: &World, scene: &Scene| scene.inventory_uv(world, world.objects.get(front).unwrap(), &hit, model, ray, now);
        assert!((uv(&world, &scene) - Vec2::new(0.35, 0.65)).length() < 0.0001);
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].media_flags = 2;
        assert!((uv(&world, &scene) - Vec2::splat(0.8)).length() < 0.0001);
        let material = Uuid::from_u128(8890);
        let key = world.objects.get(front).unwrap().key;
        let o = world.objects.get_mut(front).unwrap();
        let tf = &mut Arc::make_mut(o.te.as_mut().unwrap()).faces[0];
        tf.media_flags = 0;
        tf.scale_s = 3.0;
        tf.offset_s = 0.2;
        o.extra.render_materials = vec![(0, material)];
        let mut m = aurora_assets::PbrMaterial::default();
        m.transforms[0] = aurora_assets::material::TextureTransform {
            scale: [2.0, 1.0],
            offset: [-0.6, 0.2],
            rotation: 0.0,
        };
        scene.materials.insert(material, m);
        assert!((uv(&world, &scene) - Vec2::new(0.1, 0.45)).length() < 0.0001);
        let mut ov = aurora_assets::material::PbrOverride::default();
        ov.transforms[0].offset = [0.2, 0.1];
        world.gltf_overrides.insert(key, vec![(0, ov)].into());
        assert!((uv(&world, &scene) - Vec2::new(0.9, 0.55)).length() < 0.0001);
        world.objects.get_mut(front).unwrap().render.blinn_phong_preview = true;
        assert!((uv(&world, &scene) - Vec2::new(0.25, 0.65)).length() < 0.0001);
        world.objects.get_mut(front).unwrap().render.blinn_phong_preview = false;
        assert!((uv(&world, &scene) - Vec2::new(0.9, 0.55)).length() < 0.0001);
    }

    #[test]
    #[ignore = "explicit placement performance comparison"]
    fn inventory_placement_profile() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let template = aurora_net::inventory::rez::RezInventory {
            handle: world.main_region.unwrap(),
            group: Uuid::nil(),
            item: Default::default(),
            ray_start: Vec3::ZERO,
            ray_end: Vec3::ZERO,
            target: Uuid::nil(),
            selected: false,
            remove: false,
            source_folder: Uuid::nil(),
        };
        let updates: Vec<_> = (0..16_000)
            .map(|n| {
                let mut o = crate::demo::inventory_rez(&template);
                o.local_id = 20_000 + n;
                o.position = Vec3::new(3.0 + (n % 100) as f32, 10.0 + (n / 100) as f32, 50.0);
                o.text.clear();
                o
            })
            .collect();
        let ids: Vec<_> = updates.iter().map(|o| o.full_id).collect();
        world.apply(aurora_net::NetEvent::ObjectUpdates {
            handle: template.handle,
            objects: updates,
        });
        scene
            .gpu
            .resize_with(world.objects.iter().map(|(i, _)| i).max().unwrap() + 1, Default::default);
        let faces = scene.gpu[front].faces.clone();
        let geom = scene.gpu[front].geom;
        for idx in ids.iter().filter_map(|id| world.objects.index_of_uuid(id)).chain([front, back]) {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.gpu[idx].geom = geom;
            scene.gpu[idx].faces.clone_from(&faces);
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let mut candidates = Vec::new();
        let expected = scene.inventory_surface_among(&world, ray, 200.0, None);
        assert_eq!(scene.inventory_surface(&world, ray, 200.0, &mut candidates), expected);
        let start = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(scene.inventory_surface_among(&world, ray, 200.0, None));
        }
        let full = start.elapsed().as_secs_f64() * 10_000.0;
        let start = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(scene.inventory_surface(&world, ray, 200.0, &mut candidates));
        }
        let indexed = start.elapsed().as_secs_f64() * 10_000.0;
        println!(
            "inventory placement: full={full:.1} us, indexed={indexed:.1} us, candidates={} / 16002",
            candidates.len()
        );
    }

    #[test]
    fn hud_picks_linked_surfaces_hides_others_and_never_uses_world_coordinates() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let avatar_id = world.objects.get(own).unwrap().key.local_id;
        for (idx, x) in [(front, 3.0), (back, 7.0)] {
            let o = world.objects.get_mut(idx).unwrap();
            o.parent_id = avatar_id;
            o.state = 31u8.rotate_left(4);
            o.position = Vec3::new(x, 0.0, 0.0);
            let g = &mut scene.gpu[idx];
            g.hud = true;
            g.owner_avatar = Some(own);
            g.center = o.position;
            g.radius = 1.0;
        }
        let front_id = world.objects.get(front).unwrap().key.local_id;
        let child = world.objects.get_mut(back).unwrap();
        child.parent_id = front_id;
        child.state = 0;
        child.position = Vec3::new(4.0, 0.0, 0.0);
        scene.index_objects();
        scene.build_huds(&world, [1280, 720], true);
        let cursor = (640.0, 360.0);
        let (picked, p, ray) = scene.hud_pick(&world, cursor, true).unwrap();
        assert_eq!(picked, front);
        assert!((p - Vec3::new(2.5, 0.0, 0.0)).length() < 1e-5);
        let surface = scene.touch_surface(&world, front, ray).unwrap();
        assert_eq!(surface.face, 0);
        assert!((surface.st - Vec3::new(0.5, 0.5, 0.0)).length() < 1e-5);
        assert!((surface.position - p).length() < 1e-5);
        assert_eq!(scene.object_cursor_ray(front, cursor, None), Some(ray));
        // Ignore passes through the front HUD, Disabled still occludes.
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, back);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::DISABLED;
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, front);
        // Invisible touch panels take clicks and hover, whatever their
        // material, but quick movement passes through them.
        for f in &mut Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces {
            f.color[3] = 0.0;
        }
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, front);
        assert_eq!(scene.hud_pick(&world, cursor, false).unwrap().0, front);
        assert_eq!(scene.hover_hud(&world, cursor), Some(front));
        assert_eq!(scene.hud_move_pick(&world, cursor), Some(back));
        scene.gpu[front].owner_avatar = None;
        scene.build_huds(&world, [1280, 720], true);
        assert_eq!(scene.lists.hud_opaque.len(), 1);
        assert_eq!(scene.hud_pick(&world, cursor, true).unwrap().0, back);
        scene.build_huds(&world, [1280, 720], false);
        assert!(scene.lists.hud_opaque.is_empty());
        assert!(scene.hud_pick(&world, cursor, true).is_none());
        assert!(scene.hud_move_pick(&world, cursor).is_none());
    }

    #[test]
    fn selection_reaches_transparent_surfaces_without_an_action_or_touch_handler() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        for idx in [front, back] {
            let o = world.objects.get_mut(idx).unwrap();
            o.click_action = crate::interaction::code::NONE;
            o.update_flags = 0;
            scene.gpu[idx].center = o.position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let depth = Vec3::new(6.5, 0.0, 50.0);
        assert!(crate::interaction::target(&world, front, &HashMap::new()).is_none());
        for (te_alpha, pbr_alpha, legacy_alpha, tex_alpha) in [
            (0.5, None, None, AlphaKind::Opaque),
            (1.0, Some(1), None, AlphaKind::Opaque),
            (1.0, Some(2), None, AlphaKind::Opaque),
            (1.0, None, Some(1), AlphaKind::Opaque),
            (1.0, None, None, AlphaKind::Blend),
        ] {
            scene.gpu[front].faces[0].te_alpha = te_alpha;
            scene.gpu[front].faces[0].pbr_alpha = pbr_alpha;
            scene.gpu[front].faces[0].legacy_alpha = legacy_alpha;
            scene.gpu[front].faces[0].pass = Pass::Blend;
            scene.textures.alpha_by_slot = vec![tex_alpha];
            for build in [false, true] {
                assert_eq!(
                    scene.selection_pick(&world, Some(ray), Some(depth), build, false, 100.0),
                    (Some(front_point), Some(front))
                );
                // No opaque depth (sky behind it) must still allow selection.
                assert_eq!(
                    scene.selection_pick(&world, Some(ray), None, build, false, 100.0),
                    (Some(front_point), Some(front))
                );
            }
            // Mask keeps its displayed cutout; Blend now stops left presses.
            let action = if pbr_alpha == Some(2) { depth } else { front_point };
            assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(action));
            assert_eq!(scene.interaction_point(&world, Some(ray), Some(depth), false, 100.0), Some(depth));
        }
        // Fully invisible TE faces are omitted from draw records altogether.
        scene.gpu[front].faces.clear();
        for f in &mut Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces {
            f.color[3] = 0.0;
        }
        scene.pick.set(front, super::super::hover::PickEntry::of(&scene.gpu[front]));
        for build in [false, true] {
            assert_eq!(
                scene.selection_pick(&world, Some(ray), Some(depth), build, false, 100.0),
                (Some(front_point), Some(front))
            );
        }
    }

    #[test]
    fn selection_obeys_depth_distance_and_ignore_but_disabled_remains_selectable() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let back_point = Vec3::new(6.5, 0.0, 50.0);
        let occluder = Vec3::new(1.0, 0.0, 50.0);
        for build in [false, true] {
            assert_eq!(
                scene.selection_pick(&world, Some(ray), Some(occluder), build, false, 100.0),
                (Some(occluder), None)
            );
            assert_eq!(scene.selection_pick(&world, Some(ray), None, build, false, 2.0), (None, None));
        }
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        assert_eq!(
            scene.selection_pick(&world, Some(ray), Some(front_point), false, false, 100.0),
            (Some(back_point), Some(back))
        );
        assert_eq!(
            scene.selection_pick(&world, Some(ray), Some(back_point), true, false, 100.0),
            (Some(front_point), Some(front))
        );
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::DISABLED;
        scene.gpu[front].faces[0].te_alpha = 0.5;
        assert_eq!(
            scene.selection_pick(&world, Some(ray), Some(back_point), false, false, 100.0),
            (Some(front_point), Some(front))
        );
    }

    #[test]
    fn selection_uses_the_hit_triangles_instead_of_an_overlapping_prim_box() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let o = world.objects.get_mut(front).unwrap();
        o.position.y = 2.0;
        o.scale.y = 6.0; // box contains the ray; the only face is above it
        o.click_action = crate::interaction::code::NONE;
        o.update_flags = 0;
        let key = GeomKey::Prim { hash: 123, lod: 0 };
        let source = scene.geom_ready(&scene.gpu[front].geom.unwrap()).unwrap();
        let geom = GpuGeom {
            faces: source.faces.clone(),
            min: source.min,
            max: source.max,
            joint_bounds: Vec::new(),
            pick_faces: vec![Some(PickFace::new(
                vec![[-0.5, -0.1, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [-0.5, -0.1, 0.5]],
                vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                vec![0, 2, 1, 0, 3, 2],
            ))],
        };
        scene.geoms.insert(
            key,
            GeomEntry {
                refs: 1,
                zero_since: 0,
                state: GeomState::Ready(Arc::new(geom)),
            },
        );
        scene.gpu[front].geom = Some(key);
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 4.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let depth = Vec3::new(6.5, 0.0, 50.0);
        for build in [false, true] {
            assert_eq!(
                scene.selection_pick(&world, Some(ray), Some(depth), build, false, 100.0),
                (Some(depth), Some(back))
            );
        }
    }

    #[test]
    fn world_blend_zero_texels_pass_left_clicks_while_hover_keeps_its_policy() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        world.objects.get_mut(front).unwrap().update_flags = 0;
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::BUY;
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let texture = Uuid::from_u128(8889);
        scene
            .textures
            .pick_masks
            .insert(texture, super::super::textures::PickMask::from_rgba(2, 2, &[0; 16]).unwrap());
        scene.textures.alpha_by_slot = vec![AlphaKind::Blend];
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let depth = Vec3::new(6.5, 0.0, 50.0);
        for (pbr, legacy) in [(None, None), (Some(1), None), (None, Some(legacy_mat::alpha_mode::BLEND))] {
            for alpha in [1.0, 0.5, 0.001, 0.0] {
                let draw = &mut scene.gpu[front].faces[0];
                draw.tex_id = texture;
                draw.pbr_alpha = pbr;
                draw.legacy_alpha = legacy;
                draw.te_alpha = alpha;
                for f in &mut Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces {
                    f.color[3] = alpha;
                }
                scene.pick.changed(front);
                assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(depth));
                assert_eq!(scene.action_point(&world, Some(ray), None, false, 100.0), Some(depth));
                assert_eq!(scene.interaction_at_ray(&world, front_point, ray, true), Some(front));
                assert!(crate::interaction::target(&world, front, &HashMap::new()).is_none());
                let hovered = scene.hover_pick(&world, Some(ray), Some(depth), 100.0);
                assert_eq!(hovered, (Some(front_point), Some(front)));
                assert_eq!(crate::interaction::cursor_action(&world, hovered.1.unwrap(), &HashMap::new()), None);
                let reused = scene.hover.reused;
                assert_eq!(scene.hover_pick(&world, Some(ray), Some(depth), 100.0), hovered);
                assert_eq!(scene.hover.reused, reused + 1);
                assert_eq!(scene.hover_pick(&world, Some(ray), None, 100.0), hovered);
                // A genuinely nearer opaque surface still occludes Blend.
                let nearer = Vec3::new(1.0, 0.0, 50.0);
                assert_eq!(scene.action_point(&world, Some(ray), Some(nearer), false, 100.0), Some(nearer));
                assert_eq!(scene.hover_pick(&world, Some(ray), Some(nearer), 100.0), (Some(nearer), None));
            }
        }
        // Mask's transparent pixels retain their existing pass-through.
        scene.gpu[front].faces[0].te_alpha = 1.0;
        for f in &mut Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces {
            f.color[3] = 1.0;
        }
        for (pbr, legacy) in [(Some(2), None), (None, Some(legacy_mat::alpha_mode::MASK))] {
            scene.gpu[front].faces[0].pbr_alpha = pbr;
            scene.gpu[front].faces[0].legacy_alpha = legacy;
            scene.pick.changed(front);
            assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(depth));
            let hovered = scene.hover_pick(&world, Some(ray), Some(depth), 100.0);
            assert_eq!(hovered, (Some(depth), Some(back)));
            assert_eq!(
                crate::interaction::cursor_action(&world, hovered.1.unwrap(), &HashMap::new()),
                Some(crate::interaction::Action::Buy)
            );
        }
        scene.gpu[front].faces[0].pbr_alpha = Some(1);
        scene.gpu[front].faces[0].legacy_alpha = None;
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        scene.pick.changed(front);
        assert_eq!(scene.hover_object(&world, Some(ray), Some(depth), 100.0), Some(back));
    }

    #[test]
    fn world_blend_left_clicks_test_exact_texels_but_right_clicks_and_worn_prims_do_not() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let texture = Uuid::from_u128(8891);
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let depth = Vec3::new(6.5, 0.0, 50.0);
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::BUY;
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        scene.textures.alpha_by_slot = vec![AlphaKind::Blend];
        scene.gpu[front].faces[0].tex_id = texture;
        for (pbr, legacy, texture_alpha) in [
            (None, None, AlphaKind::Blend),
            (None, None, AlphaKind::Mask),
            (Some(1), None, AlphaKind::Mask),
            (None, Some(legacy_mat::alpha_mode::BLEND), AlphaKind::Mask),
        ] {
            scene.textures.alpha_by_slot[0] = texture_alpha;
            scene.gpu[front].faces[0].pbr_alpha = pbr;
            scene.gpu[front].faces[0].legacy_alpha = legacy;
            for texel_alpha in [0, 1, 32, 127, 255] {
                scene.textures.pick_masks.insert(
                    texture,
                    super::super::textures::PickMask::from_rgba(1, 1, &[255, 255, 255, texel_alpha]).unwrap(),
                );
                let expected = if texel_alpha == 0 { depth } else { front_point };
                assert_eq!(scene.point_behind_rigged(&world, ray, 0.0, 100.0, None), Some(expected));
                assert_eq!(scene.left_touch_surface(&world, front, ray).is_some(), texel_alpha > 0);
                assert!(scene.touch_surface(&world, front, ray).is_some());
                for action in [
                    crate::interaction::code::NONE,
                    crate::interaction::code::TOUCH,
                    crate::interaction::code::DISABLED,
                ] {
                    let o = world.objects.get_mut(front).unwrap();
                    o.click_action = action;
                    o.update_flags = crate::ui::context::flags::HANDLE_TOUCH | crate::build::edits::FLAGS_PHANTOM;
                    assert_eq!(
                        scene.action_point(&world, Some(ray), Some(depth), false, 100.0),
                        Some(expected),
                        "mode {pbr:?}/{legacy:?}, texel {texel_alpha}, action {action}"
                    );
                }
                assert_eq!(
                    scene.selection_pick(&world, Some(ray), Some(depth), false, false, 100.0),
                    (Some(front_point), Some(front))
                );
                let target = if texel_alpha == 0 { back } else { front };
                assert_eq!(
                    scene.left_selection_pick(&world, Some(ray), Some(depth), true, false, 100.0),
                    (Some(expected), Some(target))
                );
                // Worn prims retain their existing Blend/Mask policy.
                scene.gpu[front].owner_avatar = Some(own);
                let worn_point = if pbr.is_none() && legacy.is_none() && texture_alpha == AlphaKind::Mask {
                    depth
                } else {
                    front_point
                };
                assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(worn_point));
                assert!(scene.left_touch_surface(&world, front, ray).is_some());
                scene.gpu[front].owner_avatar = None;
            }
        }
        // Resident texture coordinates, not mesh ST: offset the sampled zero
        // texel onto its opaque neighbour within the old mask's same 2x2 block.
        scene.gpu[front].faces[0].legacy_alpha = Some(legacy_mat::alpha_mode::BLEND);
        let mut rgba = vec![255; 4 * 4 * 4];
        rgba[(4 + 2) * 4 + 3] = 0;
        scene
            .textures
            .pick_masks
            .insert(texture, super::super::textures::PickMask::from_rgba(4, 4, &rgba).unwrap());
        assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(depth));
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].offset_s = 0.25;
        assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(front_point));
        // Unknown texture pixels remain pickable during streaming.
        scene.textures.pick_masks.remove(&texture);
        assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(front_point));
        // The default Blend mode on the library's transparent texture must
        // not catch the exit face of a plaque whose top has a texture hole.
        scene.gpu[front].faces[0].legacy_alpha = None;
        scene.gpu[front].faces[0].tex_id = aurora_prim::te::TRANSPARENT_TEXTURE;
        assert_eq!(
            scene.left_selection_pick(&world, Some(ray), Some(depth), true, false, 100.0),
            (Some(depth), Some(back))
        );
        assert_eq!(
            scene.selection_pick(&world, Some(ray), Some(depth), true, false, 100.0),
            (Some(front_point), Some(front))
        );
    }

    #[test]
    fn world_blend_left_clicks_ignore_the_exit_faces_of_a_transparent_cube() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let mesh = aurora_prim::generate_volume(&aurora_prim::params::VolumeParams::default(), aurora_prim::volume::DETAIL_SCALES[3]);
        assert_eq!(mesh.faces.len(), 6);
        let template = scene.gpu[front].faces[0];
        let hole = Uuid::from_u128(8892);
        scene
            .textures
            .pick_masks
            .insert(hole, super::super::textures::PickMask::from_rgba(1, 1, &[0; 4]).unwrap());
        scene.textures.alpha_by_slot = vec![AlphaKind::Blend];
        let mut geometry = GpuGeom {
            faces: Vec::new(),
            min: -Vec3::splat(0.5),
            max: Vec3::splat(0.5),
            joint_bounds: Vec::new(),
            pick_faces: Vec::new(),
        };
        scene.gpu[front].faces.clear();
        for (face, mesh) in mesh.faces.into_iter().enumerate() {
            let allocation = aurora_render::MeshAlloc {
                vertex_offset: 0,
                vertex_count: mesh.positions.len() as u32,
                index_offset: face as u32 * 100,
                index_count: mesh.indices.len() as u32,
            };
            let mut draw = template;
            draw.te_index = face as u8;
            draw.cmd.first_index = allocation.index_offset;
            draw.cmd.index_count = allocation.index_count;
            draw.legacy_alpha = Some(legacy_mat::alpha_mode::BLEND);
            // The entry face has a texture hole; the exit face is solid.
            draw.tex_id = if mesh.normals[0][0] < -0.9 { hole } else { Uuid::nil() };
            scene.gpu[front].faces.push(draw);
            geometry.faces.push(Some(allocation));
            geometry
                .pick_faces
                .push(Some(PickFace::new(mesh.positions, mesh.uvs, mesh.indices)));
        }
        let key = GeomKey::Prim { hash: 8892, lod: 3 };
        scene.geoms.insert(
            key,
            GeomEntry {
                refs: 1,
                zero_since: 0,
                state: GeomState::Ready(Arc::new(geometry)),
            },
        );
        scene.gpu[front].geom = Some(key);
        world.objects.get_mut(front).unwrap().update_flags = 0;
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        world.objects.get_mut(back).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        for idx in [front, back] {
            let g = &mut scene.gpu[idx];
            g.center = world.objects.get(idx).unwrap().position;
            g.radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(g));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let depth = Vec3::new(6.5, 0.0, 50.0);
        assert!(
            scene.left_touch_surface(&world, front, ray).is_none(),
            "invisible exit face must not catch a touch"
        );
        assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(depth));
        assert_eq!(scene.action_point(&world, Some(ray), None, false, 100.0), Some(depth));
        assert_eq!(
            scene.left_selection_pick(&world, Some(ray), Some(depth), true, false, 100.0),
            (Some(depth), Some(back))
        );

        // An explicitly double-sided PBR cube really displays the exit face.
        let material = Uuid::from_u128(8893);
        scene.materials.insert(
            material,
            aurora_assets::PbrMaterial {
                alpha_mode: aurora_assets::AlphaMode::Blend,
                double_sided: true,
                ..Default::default()
            },
        );
        world.objects.get_mut(front).unwrap().extra.render_materials = (0..6).map(|face| (face, material)).collect();
        let exit = scene.left_touch_surface(&world, front, ray).unwrap().position;
        assert!(exit.distance(Vec3::new(3.5, 0.0, 50.0)) < 1e-5);
        let object = world.objects.get(front).unwrap();
        world.gltf_overrides.insert(
            object.key,
            (0..6)
                .map(|face| {
                    (
                        face,
                        aurora_assets::material::PbrOverride {
                            double_sided: Some(false),
                            ..Default::default()
                        },
                    )
                })
                .collect::<Vec<_>>()
                .into(),
        );
        assert!(scene.left_touch_surface(&world, front, ray).is_none());
    }

    #[test]
    fn blend_press_tests_the_intersected_face_in_a_mixed_material_object() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        world.objects.get_mut(front).unwrap().update_flags = 0;
        let source = scene.geom_ready(&scene.gpu[front].geom.unwrap()).unwrap();
        let geometry = GpuGeom {
            faces: vec![
                source.faces[0],
                Some(aurora_render::MeshAlloc {
                    index_offset: 6,
                    ..source.faces[0].unwrap()
                }),
            ],
            min: source.min,
            max: source.max,
            joint_bounds: Vec::new(),
            pick_faces: vec![
                Some(PickFace::new(
                    source.pick_faces[0].as_ref().unwrap().positions.clone(),
                    source.pick_faces[0].as_ref().unwrap().uvs.clone(),
                    source.pick_faces[0].as_ref().unwrap().indices.clone(),
                )),
                Some(PickFace::new(
                    vec![[-0.5, 0.2, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [-0.5, 0.2, 0.5]],
                    vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                    vec![0, 2, 1, 0, 3, 2],
                )),
            ],
        };
        let key = GeomKey::Prim { hash: 124, lod: 0 };
        scene.geoms.insert(
            key,
            GeomEntry {
                refs: 1,
                zero_since: 0,
                state: GeomState::Ready(Arc::new(geometry)),
            },
        );
        scene.gpu[front].geom = Some(key);
        scene.gpu[front].faces[0].pbr_alpha = Some(2);
        let mut blend = scene.gpu[front].faces[0];
        blend.te_index = 1;
        blend.cmd.first_index = 6;
        blend.pbr_alpha = Some(1);
        scene.gpu[front].faces.push(blend);
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        // The Mask face is crossed, but the Blend face elsewhere on the same
        // object must not fill its transparent hole.
        for y in [0.0, 0.3] {
            let ray = (Vec3::new(0.0, y, 50.0), Vec3::X);
            let depth = Vec3::new(6.5, y, 50.0);
            let expected = if y == 0.0 { depth } else { Vec3::new(2.5, y, 50.0) };
            assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(expected));
            let object = if y == 0.0 { back } else { front };
            assert_eq!(
                scene.hover_pick(&world, Some(ray), Some(depth), 100.0),
                (Some(expected), Some(object))
            );
        }
    }

    #[test]
    fn blend_press_reaches_omitted_faces_and_respects_material_overrides() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let material = Uuid::from_u128(8890);
        let object = world.objects.get_mut(front).unwrap();
        object.click_action = crate::interaction::code::NONE;
        object.update_flags = 0;
        object.extra.render_materials.push((0, material));
        for f in &mut Arc::make_mut(object.te.as_mut().unwrap()).faces {
            f.color[3] = 0.0;
        }
        let object_key = object.key;
        scene.materials.insert(material, aurora_assets::PbrMaterial::default());
        scene.gpu[front].faces.clear();
        for idx in [front, back] {
            scene.gpu[idx].center = world.objects.get(idx).unwrap().position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let front_point = Vec3::new(2.5, 0.0, 50.0);
        let depth = Vec3::new(6.5, 0.0, 50.0);
        for mode in [aurora_assets::AlphaMode::Blend, aurora_assets::AlphaMode::Mask] {
            world.gltf_overrides.insert(
                object_key,
                vec![(
                    0,
                    aurora_assets::material::PbrOverride {
                        alpha_mode: Some(mode),
                        ..Default::default()
                    },
                )]
                .into(),
            );
            let expected = if mode == aurora_assets::AlphaMode::Blend {
                front_point
            } else {
                depth
            };
            scene.pick.changed(front);
            assert_eq!(scene.action_point(&world, Some(ray), Some(depth), false, 100.0), Some(expected));
            let object = if mode == aurora_assets::AlphaMode::Blend { front } else { back };
            assert_eq!(
                scene.hover_pick(&world, Some(ray), Some(depth), 100.0),
                (Some(expected), Some(object))
            );
        }
        assert_eq!(scene.interaction_at_ray(&world, front_point, ray, true), Some(front));
    }

    #[test]
    fn ignore_picks_through_disabled_occludes_and_build_includes_both() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let depth = Vec3::new(2.5, 0.0, 50.0);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        let behind = scene.interaction_point(&world, Some(ray), Some(depth), false, 100.0).unwrap();
        assert_eq!(behind, Vec3::new(6.5, 0.0, 50.0));
        assert_eq!(scene.interaction_at(&world, behind, Instant::now(), false), Some(back));
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(depth), true, 100.0), Some(depth));
        assert_eq!(scene.interaction_at(&world, depth, Instant::now(), true), Some(front));
        let surface = scene.touch_surface(&world, back, ray).unwrap();
        assert_eq!(surface.face, 0);
        assert_eq!(surface.st, Vec3::new(0.5, 0.5, 0.0));
        assert_eq!(surface.position, behind);
        assert!((surface.normal.length() - 1.0).abs() < 1e-5);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::DISABLED;
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(depth), false, 100.0), Some(depth));
        assert_eq!(scene.interaction_at(&world, depth, Instant::now(), false), Some(front));
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::IGNORE;
        assert!(scene.interaction_point(&world, Some(ray), Some(depth), false, 100.0).is_none());
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::BUY;
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        world.objects.get_mut(front).unwrap().update_flags = 1 << 7;
        scene.gpu[front].faces[0].te_alpha = 0.5;
        let through = Vec3::new(6.5, 0.0, 50.0);
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(through), false, 100.0), Some(depth));
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::DISABLED;
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(through), false, 100.0), Some(depth));
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        scene.gpu[front].faces[0].te_alpha = 0.0;
        let te = world.objects.get_mut(front).unwrap().te.as_mut().unwrap();
        for face in Arc::make_mut(te).faces.iter_mut() {
            face.color[3] = 0.0;
        }
        assert_eq!(
            scene.interaction_point(&world, Some(ray), Some(through), false, 100.0),
            Some(through)
        );
        assert_eq!(scene.action_point(&world, Some(ray), Some(through), false, 100.0), Some(depth));
    }

    #[test]
    fn linked_chair_surface_wins_over_touch_root_with_overlapping_bounds() {
        let (mut world, mut scene, front, back) = scene_with_two_objects();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let depth = Vec3::new(2.5, 0.0, 50.0);
        // A hollow table's smaller box contains the chair's surface, but its
        // triangles do not intersect this ray. Both belong to one linkset.
        {
            let table = world.objects.get_mut(front).unwrap();
            table.click_action = crate::interaction::code::TOUCH;
            table.update_flags = 1 << 7;
            for f in Arc::make_mut(table.te.as_mut().unwrap()).faces.iter_mut() {
                f.color[3] = 1.0;
            }
        }
        let table_local_id = world.objects.get(front).unwrap().key.local_id;
        {
            let chair = world.objects.get_mut(back).unwrap();
            chair.parent_id = table_local_id;
            chair.position = Vec3::ZERO;
            chair.scale = Vec3::splat(2.0);
            chair.click_action = crate::interaction::code::SIT;
            chair.update_flags = 0;
        }
        let chair_key = GeomKey::Prim { hash: 1, lod: 0 };
        scene.geoms.insert(
            chair_key,
            GeomEntry {
                refs: 1,
                zero_since: 0,
                state: GeomState::Ready(Arc::new(GpuGeom {
                    faces: vec![None],
                    min: -Vec3::splat(0.5),
                    max: Vec3::splat(0.5),
                    joint_bounds: Vec::new(),
                    pick_faces: vec![Some(PickFace::new(
                        vec![[-0.5, -0.5, 0.2], [0.5, -0.5, 0.2], [0.5, 0.5, 0.2], [-0.5, 0.5, 0.2]],
                        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                        vec![0, 1, 2, 0, 2, 3],
                    ))],
                })),
            },
        );
        scene.gpu[back].geom = Some(chair_key);
        let chair_ray = (Vec3::new(3.0, 0.0, 55.0), -Vec3::Z);
        let chair_point = Vec3::new(3.0, 0.0, 50.4);
        for include_hidden in [false, true] {
            let idx = scene.interaction_at_ray(&world, chair_point, chair_ray, include_hidden).unwrap();
            assert_eq!(idx, back);
            let target = crate::interaction::target(&world, idx, &HashMap::new()).unwrap();
            assert_eq!(target.action, crate::interaction::Action::Sit);
            assert_eq!(target.object, world.objects.get(back).unwrap().full_id);
            assert_eq!(crate::interaction::cursor_action(&world, idx, &HashMap::new()), Some(target.action));
            let idx = scene.interaction_at_ray(&world, depth, ray, include_hidden).unwrap();
            assert_eq!(idx, front);
            assert_eq!(
                crate::interaction::cursor_action(&world, idx, &HashMap::new()),
                Some(crate::interaction::Action::Touch)
            );
        }
        assert!(
            scene
                .interaction_at_ray(&world, Vec3::new(3.0, 0.0, 53.0), chair_ray, false)
                .is_none()
        );
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::IGNORE;
        assert!(scene.interaction_at_ray(&world, chair_point, chair_ray, true).is_none());
    }

    #[test]
    fn ray_pick_uses_real_geometry_bounds_and_preserves_hidden_face_rules() {
        let (mut world, mut scene, front, _) = scene_with_two_objects();
        for key in [
            GeomKey::Prim { hash: 2, lod: 0 },
            GeomKey::Sculpt { hash: 2, lod: 0 },
            GeomKey::Mesh { id: Uuid::nil(), lod: 0 },
        ] {
            scene.geoms.insert(
                key,
                GeomEntry {
                    refs: 1,
                    zero_since: 0,
                    state: GeomState::Ready(Arc::new(GpuGeom {
                        faces: vec![None],
                        min: Vec3::new(2.0, -0.5, -0.5),
                        max: Vec3::new(2.0, 0.5, 0.5),
                        joint_bounds: Vec::new(),
                        pick_faces: vec![Some(PickFace::new(
                            vec![[2.0, -0.5, -0.5], [2.0, 0.5, -0.5], [2.0, 0.5, 0.5], [2.0, -0.5, 0.5]],
                            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                            vec![0, 1, 2, 0, 2, 3],
                        ))],
                    })),
                },
            );
            scene.gpu[front].geom = Some(key);
            let point = Vec3::new(5.0, 0.0, 50.0);
            let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
            assert_eq!(scene.interaction_at_ray(&world, point, ray, false), Some(front));
            for f in Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap())
                .faces
                .iter_mut()
            {
                f.color[3] = 0.0;
            }
            assert!(scene.interaction_at_ray(&world, point, ray, false).is_none());
            assert_eq!(scene.interaction_at_ray(&world, point, ray, true), Some(front));
            for f in Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap())
                .faces
                .iter_mut()
            {
                f.color[3] = 1.0;
            }
        }
    }
}
