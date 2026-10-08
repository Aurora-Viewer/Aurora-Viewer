//! Terraforming (LLToolBrushLand) and land selection (LLToolSelectLand).
//!
//! The brush sends one ModifyLand per frame and per region under it while
//! the mouse is held (Seconds = force / smoothed frame rate). "Select land"
//! drags a rectangle on the 4 m parcel grid; "Apply" then works on it.

use super::draw::{Painter3d, rgba};
use super::geom::Cam;
use super::{BuildSettings, REGION_FLAGS_BLOCK_TERRAFORM};
use crate::world::World;
use aurora_net::build::{BuildCmd, land_action};
use aurora_net::{RegionHandle, parcel_flags};
use glam::{Vec2, Vec3};

/// LLParcel PARCEL_GRID_STEP_METERS.
const PARCEL_GRID: f32 = 4.0;
/// LLViewerParcelMgr PARCEL_POST_HEIGHT.
const POST_HEIGHT: f32 = 0.666;

pub const ACTION_NAMES: [&str; 6] = ["Aplanir", "Élever", "Abaisser", "Lisser", "Rendre irrégulier", "Rétablir"];

#[derive(Debug, Default)]
pub struct LandTool {
    /// Brush held: terrain height where it started (mStartingZ).
    brushing: Option<f32>,
    /// Point under the mouse (render space, rounded to the meter).
    pub hover: Option<Vec3>,
    /// gFPSClamped of LL, smoothed.
    fps: f32,
    /// Regions touched by the last stroke (UndoLand).
    pub last_regions: Vec<RegionHandle>,
    /// Land selection being dragged / made (render-space x, y corners).
    dragging: Option<(Vec2, Vec2)>,
    pub selection: Option<(Vec2, Vec2)>,
    message: Option<String>,
}

/// Where the mouse ray meets the terrain (marching the height maps).
pub fn terrain_hit(world: &World, o: Vec3, d: Vec3) -> Option<Vec3> {
    let mut prev = o;
    let mut t = 0.0;
    let step = 0.5;
    while t < 512.0 {
        t += step;
        let p = o + d * t;
        if let Some(h) = world.ground_height(p)
            && p.z <= h
        {
            // refine between the last two samples
            let (mut a, mut b) = (prev, p);
            for _ in 0..12 {
                let m = (a + b) * 0.5;
                match world.ground_height(m) {
                    Some(hm) if m.z <= hm => b = m,
                    _ => a = m,
                }
            }
            return Some(b);
        }
        prev = p;
    }
    None
}

/// Region containing a render-space point, and that region's offset.
fn region_at(world: &World, p: Vec3) -> Option<(RegionHandle, Vec3)> {
    world.regions.iter().find_map(|(&h, r)| {
        let off = world.region_offset(h)?;
        let (x, y) = (p.x - off.x, p.y - off.y);
        (x >= 0.0 && y >= 0.0 && x < r.heightmap.size_x as f32 && y < r.heightmap.size_y as f32).then_some((h, off))
    })
}

/// LLToolBrushLand::determineAffectedRegions: the regions under the brush corners.
fn affected_regions(world: &World, spot: Vec3, size: f32) -> Vec<(RegionHandle, Vec3)> {
    let mut out: Vec<(RegionHandle, Vec3)> = Vec::new();
    let h = size * 0.5;
    for (dx, dy) in [(-h, -h), (h, -h), (-h, h), (h, h)] {
        if let Some(r) = region_at(world, spot + Vec3::new(dx, dy, 0.0))
            && !out.iter().any(|(x, _)| *x == r.0)
        {
            out.push(r);
        }
    }
    out
}

fn region_name(world: &World, h: RegionHandle) -> String {
    world.regions.get(&h).map(|r| r.name.clone()).unwrap_or_default()
}

fn can_terraform_region(world: &World, h: RegionHandle) -> bool {
    world
        .regions
        .get(&h)
        .and_then(|r| r.info.as_ref())
        .is_none_or(|i| i.region_flags & REGION_FLAGS_BLOCK_TERRAFORM == 0)
}

impl LandTool {
    pub fn cancel(&mut self) {
        self.brushing = None;
        self.dragging = None;
    }

    pub fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }

    pub fn mouse_up(&mut self) {
        self.brushing = None;
        if let Some((a, b)) = self.dragging.take() {
            if (a - b).length() >= 0.5 {
                // round outwards to the parcel grid (LLToolSelectLand::roundXY)
                let lo = (a.min(b) / PARCEL_GRID).floor() * PARCEL_GRID;
                let hi = (a.max(b) / PARCEL_GRID).ceil() * PARCEL_GRID;
                self.selection = Some((lo, hi.max(lo + Vec2::splat(PARCEL_GRID))));
            } else {
                // a click: the 4 m square under it
                let lo = (a / PARCEL_GRID).floor() * PARCEL_GRID;
                self.selection = Some((lo, lo + Vec2::splat(PARCEL_GRID)));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mouse_down(
        &mut self,
        world: &World,
        s: &BuildSettings,
        cam: &Cam,
        hit: Option<Vec3>,
        target: Option<usize>,
        cursor: (f32, f32),
        _out: &mut Vec<BuildCmd>,
    ) -> bool {
        let (o, d) = cam.ray(cursor.0, cursor.1);
        // land under the mouse: the picked point when it is not an object
        let p = match (hit, target) {
            (Some(h), None) => Some(h),
            _ => terrain_hit(world, o, d),
        };
        let Some(p) = p else {
            return true;
        };
        if s.land_action > land_action::REVERT {
            // select land
            self.selection = None;
            self.dragging = Some((p.truncate(), p.truncate()));
            return true;
        }
        let spot = Vec3::new((p.x + 0.5).floor(), (p.y + 0.5).floor(), p.z);
        let Some((handle, _)) = region_at(world, spot) else {
            return true;
        };
        if !can_terraform_region(world, handle) {
            self.message = Some(format!(
                "La région {} n'autorise pas la modification du terrain.",
                region_name(world, handle)
            ));
            return true;
        }
        if let Some(parcel) = world.parcel.as_ref()
            && parcel.flags & parcel_flags::ALLOW_TERRAFORM == 0
            && parcel.owner_id != world.agent_id
        {
            self.message = Some(format!(
                "Vous n'êtes pas autorisé à modifier le terrain de la parcelle {}.",
                parcel.name
            ));
        }
        self.brushing = Some(world.ground_height(spot).unwrap_or(p.z));
        self.hover = Some(spot);
        self.last_regions.clear();
        true
    }

    /// Hover point, and one brush stroke per frame while held.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        world: &World,
        s: &BuildSettings,
        cam: &Cam,
        cursor: (f32, f32),
        over_ui: bool,
        dt: f32,
        out: &mut Vec<BuildCmd>,
    ) {
        // gFPSClamped: frame rate clamped to 1..200, smoothed (fps + 4 old) / 5
        let fps = (1.0 / dt.max(1e-4)).clamp(1.0, 200.0);
        self.fps = if self.fps <= 0.0 { 10.0 } else { (fps + 4.0 * self.fps) / 5.0 };
        let (o, d) = cam.ray(cursor.0, cursor.1);
        let hit = if over_ui && self.brushing.is_none() && self.dragging.is_none() {
            None
        } else {
            terrain_hit(world, o, d)
        };
        self.hover = hit.map(|p| Vec3::new((p.x + 0.5).floor(), (p.y + 0.5).floor(), p.z));
        if let (Some((a, _)), Some(p)) = (self.dragging, hit) {
            self.dragging = Some((a, p.truncate()));
        }
        let (Some(start_z), Some(spot)) = (self.brushing, self.hover) else {
            return;
        };
        let action = s.land_action.min(land_action::REVERT);
        let size = s.land_brush_size.clamp(1.0, 11.0);
        let seconds = s.land_brush_force.clamp(0.1, 100.0) / self.fps;
        for (handle, off) in affected_regions(world, spot, size) {
            if !can_terraform_region(world, handle) {
                continue;
            }
            let local = spot - off;
            out.push(BuildCmd::ModifyLand {
                handle,
                action,
                brush_size: size,
                seconds,
                height: start_z,
                parcel_local_id: -1,
                west: local.x,
                south: local.y,
                east: local.x,
                north: local.y,
            });
            if !self.last_regions.contains(&handle) {
                self.last_regions.push(handle);
            }
        }
    }

    /// "Apply" on the selected land (LLToolBrushLand::modifyLandInSelectionGlobal).
    pub fn apply_to_selection(&mut self, world: &World, s: &BuildSettings, out: &mut Vec<BuildCmd>) {
        let Some((lo, hi)) = self.selection else {
            self.message = Some("Sélectionnez d'abord du terrain (outil « Sélectionner le terrain »).".into());
            return;
        };
        let action = s.land_action.min(land_action::REVERT);
        let force = s.land_brush_force.clamp(0.1, 100.0);
        let seconds = match action {
            land_action::SMOOTH => force * 5.0,
            land_action::ROUGHEN => force * 0.5,
            land_action::REVERT => 0.5,
            _ => force * 0.25,
        };
        let mid = ((lo + hi) * 0.5).extend(0.0);
        let height = world.ground_height(mid).unwrap_or(0.0);
        let mut regions: Vec<(RegionHandle, Vec3)> = Vec::new();
        for c in [
            lo,
            Vec2::new(hi.x - 0.01, lo.y),
            Vec2::new(lo.x, hi.y - 0.01),
            hi - Vec2::splat(0.01),
        ] {
            if let Some(r) = region_at(world, c.extend(0.0))
                && !regions.iter().any(|(h, _)| *h == r.0)
            {
                regions.push(r);
            }
        }
        if let Some((h, _)) = regions.iter().find(|(h, _)| !can_terraform_region(world, *h)) {
            self.message = Some(format!(
                "La région {} n'autorise pas la modification du terrain.",
                region_name(world, *h)
            ));
            return;
        }
        self.last_regions.clear();
        for (handle, off) in regions {
            let Some(r) = world.regions.get(&handle) else { continue };
            let (w, h) = (r.heightmap.size_x as f32, r.heightmap.size_y as f32);
            let a = (lo - off.truncate()).clamp(Vec2::ZERO, Vec2::new(w, h));
            let b = (hi - off.truncate()).clamp(Vec2::ZERO, Vec2::new(w, h));
            out.push(BuildCmd::ModifyLand {
                handle,
                action,
                brush_size: s.land_brush_size.clamp(1.0, 11.0),
                seconds,
                height,
                parcel_local_id: -1,
                west: a.x,
                south: a.y,
                east: b.x,
                north: b.y,
            });
            self.last_regions.push(handle);
        }
    }

    /// Undo the last stroke (UndoLand to every region it touched).
    pub fn undo(&mut self, out: &mut Vec<BuildCmd>) {
        for handle in self.last_regions.drain(..) {
            out.push(BuildCmd::UndoLand { handle });
        }
    }

    // ---- drawing

    pub fn draw(&self, p: &mut Painter3d, world: &World, s: &BuildSettings) {
        // brush (LLToolBrushLand::renderOverlay): white vertical lines on
        // every meter of the brush, taller in the middle and with the force
        if s.land_action <= land_action::REVERT
            && let Some(spot) = self.hover
        {
            let half = s.land_brush_size.clamp(1.0, 11.0).floor() as i32;
            let force = s.land_brush_force;
            let tic = 0.075;
            let white = rgba(1.0, 1.0, 1.0, 1.0);
            let a = s.land_action;
            for di in -half..=half {
                for dj in -half..=half {
                    let x = spot.x + di as f32;
                    let y = spot.y + dj as f32;
                    let Some(z) = world.ground_height(Vec3::new(x, y, 0.0)) else {
                        continue;
                    };
                    let z = z + 1.0;
                    let norm = ((di * di + dj * dj) as f32).sqrt() / half.max(1) as f32;
                    let z2 = z + 0.2 + (0.2 + force / 100.0) * (std::f32::consts::SQRT_2 - norm);
                    let (b, t) = (Vec3::new(x, y, z), Vec3::new(x, y, z2));
                    p.line(b, t, white, 1.0);
                    if a == land_action::RAISE || a == land_action::ROUGHEN {
                        p.line(t, t + Vec3::new(tic, 0.0, -tic), white, 1.0);
                        p.line(t, t + Vec3::new(-tic, 0.0, -tic), white, 1.0);
                    }
                    if a == land_action::LOWER || a == land_action::ROUGHEN {
                        p.line(b, b + Vec3::new(tic, 0.0, tic), white, 1.0);
                        p.line(b, b + Vec3::new(-tic, 0.0, tic), white, 1.0);
                    }
                    if a == land_action::REVERT || a == land_action::SMOOTH {
                        p.line(t - Vec3::X * tic, t + Vec3::X * tic, white, 1.0);
                    }
                    if a == land_action::FLATTEN || a == land_action::SMOOTH {
                        p.line(b - Vec3::X * tic, b + Vec3::X * tic, white, 1.0);
                    }
                }
            }
        }
        // land selection: yellow walls around it (renderRect / renderHighlightSegments)
        let rect = self.dragging.or(self.selection);
        if let Some((a, b)) = rect {
            let (lo, hi) = (a.min(b), a.max(b));
            let yellow = rgba(1.0, 1.0, 0.0, 1.0);
            let wall = rgba(1.0, 1.0, 0.0, 0.2);
            let ground = |x: f32, y: f32| world.ground_height(Vec3::new(x, y, 0.0)).unwrap_or(0.0);
            let corners = [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)];
            for k in 0..4 {
                let (c0, c1) = (corners[k], corners[(k + 1) % 4]);
                let len = c0.distance(c1);
                let n = ((len / PARCEL_GRID).ceil() as usize).clamp(1, 256);
                for i in 0..n {
                    let (q0, q1) = (c0.lerp(c1, i as f32 / n as f32), c0.lerp(c1, (i + 1) as f32 / n as f32));
                    let (z0, z1) = (ground(q0.x, q0.y), ground(q1.x, q1.y));
                    let (b0, b1) = (q0.extend(z0), q1.extend(z1));
                    let up = Vec3::Z * POST_HEIGHT;
                    p.quad([b0, b1, b1 + up, b0 + up], wall);
                }
                let z = ground(c0.x, c0.y);
                p.line(c0.extend(z), c0.extend(z + POST_HEIGHT), yellow, 2.0);
            }
            p.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_rounds_to_parcel_grid() {
        let mut t = LandTool {
            dragging: Some((Vec2::new(5.0, 9.0), Vec2::new(13.5, 3.0))),
            ..Default::default()
        };
        t.mouse_up();
        let (lo, hi) = t.selection.unwrap();
        assert_eq!(lo, Vec2::new(4.0, 0.0));
        assert_eq!(hi, Vec2::new(16.0, 12.0));
    }
}
