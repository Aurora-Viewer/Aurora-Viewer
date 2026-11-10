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

/// Labels of the land radio (floater_tools.xml, French skin).
pub fn action_names() -> [&'static str; 6] {
    [
        aurora_i18n::tr!("build-land-flatten"),
        aurora_i18n::tr!("build-land-raise"),
        aurora_i18n::tr!("build-land-lower"),
        aurora_i18n::tr!("build-land-smooth"),
        aurora_i18n::tr!("build-land-bump"),
        aurora_i18n::tr!("build-land-cancel-edit"),
    ]
}

/// ParcelPropertiesRequest sequence ids of the land selection (the build
/// floater's capacity line uses the 1000 before them).
const PARCEL_SEQ: i32 = aurora_net::build::BUILD_PARCEL_SEQ + 1000;

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
    /// Parcel properties of the selected land (LLViewerParcelMgr's
    /// selection), asked for when the selection changes.
    pub parcel: Option<(RegionHandle, std::sync::Arc<aurora_net::ParcelInfo>)>,
    parcel_seq: i32,
    request_parcel: bool,
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

/// LLToolSelectLand::handleHover / handleMouseUp (indra/newview/lltoolselectland.cpp):
/// show the same 4 m cells while dragging and after release. Keep the raw anchor.
fn parcel_rect(a: Vec2, b: Vec2) -> (Vec2, Vec2) {
    if (a - b).length() < 0.5 {
        let lo = (a / PARCEL_GRID).floor() * PARCEL_GRID;
        return (lo, lo + Vec2::splat(PARCEL_GRID));
    }
    let lo = (a.min(b) / PARCEL_GRID).floor() * PARCEL_GRID;
    let hi = (a.max(b) / PARCEL_GRID).ceil() * PARCEL_GRID;
    (lo, hi.max(lo + Vec2::splat(PARCEL_GRID)))
}

impl LandTool {
    pub fn cancel(&mut self) {
        self.brushing = None;
        self.dragging = None;
    }

    /// Feedback still held by the land tool, including pending join refusals.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }

    pub fn mouse_up(&mut self) {
        self.brushing = None;
        if let Some((a, b)) = self.dragging.take() {
            self.request_parcel = true;
            self.parcel = None;
            self.selection = Some(parcel_rect(a, b));
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
        self.message = None;
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
            self.message = Some(aurora_i18n::tr!(
                "build-land-region-does-not-allow-terraforming",
                value0 = region_name(world, handle).to_string()
            ));
            return true;
        }
        if let Some(parcel) = world.parcel.as_ref()
            && parcel.flags & parcel_flags::ALLOW_TERRAFORM == 0
            && parcel.owner_id != world.agent_id
        {
            self.message = Some(aurora_i18n::tr!(
                "build-land-you-are-not-allowed-to-edit-terrain-on-parcel",
                value0 = parcel.name.to_string()
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
        if std::mem::take(&mut self.request_parcel) {
            self.ask_parcel(world, out);
        }
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
            self.message = Some(aurora_i18n::tr!("build-land-select-land-first-the-select-land-tool").into());
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
            self.message = Some(aurora_i18n::tr!(
                "build-land-region-does-not-allow-terraforming",
                value0 = region_name(world, *h).to_string()
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

    /// ParcelPropertiesRequest for the selected land, in the region of its
    /// south-west corner (LLViewerParcelMgr::selectLand).
    fn ask_parcel(&mut self, world: &World, out: &mut Vec<BuildCmd>) {
        let Some((lo, hi)) = self.selection else { return };
        let Some((handle, off)) = region_at(world, lo.extend(0.0) + Vec3::new(0.01, 0.01, 0.0)) else {
            return;
        };
        let (a, b) = (lo - off.truncate(), hi - off.truncate());
        self.parcel_seq = PARCEL_SEQ + (self.parcel_seq + 1 - PARCEL_SEQ).rem_euclid(1000);
        out.push(BuildCmd::ParcelRequest {
            handle,
            sequence: self.parcel_seq,
            snap: false,
            west: a.x,
            south: a.y,
            east: b.x,
            north: b.y,
        });
    }

    /// The answer to `ask_parcel`.
    pub fn on_selected_parcel(&mut self, handle: RegionHandle, sequence: i32, parcel: std::sync::Arc<aurora_net::ParcelInfo>) {
        if sequence == self.parcel_seq && !self.request_parcel && self.dragging.is_none() && self.selection.is_some() {
            self.parcel = Some((handle, parcel));
        }
    }

    /// The selection, in its region's coordinates: (region, west, south, east, north).
    pub fn selection_local(&self, world: &World) -> Option<(RegionHandle, f32, f32, f32, f32)> {
        let (lo, hi) = self.selection?;
        let (handle, off) = region_at(world, lo.extend(0.0) + Vec3::new(0.01, 0.01, 0.0))?;
        let (a, b) = (lo - off.truncate(), hi - off.truncate());
        Some((handle, a.x, a.y, b.x, b.y))
    }

    /// Area of the selection (m²).
    pub fn selection_area(&self) -> f32 {
        self.selection.map(|(a, b)| (b.x - a.x) * (b.y - a.y)).unwrap_or(0.0)
    }

    /// LLPanelLandInfo::refresh (indra/newview/llpanelland.cpp): a dragged
    /// rectangle larger than one cell. These requests use SnapSelection=false,
    /// so an equal area does not mean that a whole parcel was selected.
    /// Ownership of an aggregate reply is not a join permission check; the
    /// simulator checks the parcels' rights, as in the Legacy Viewer.
    pub fn can_join(&self, world: &World) -> bool {
        if self.request_parcel || self.dragging.is_some() || self.selection_area() <= PARCEL_GRID * PARCEL_GRID {
            return false;
        }
        let Some((handle, west, south, east, north)) = self.selection_local(world) else {
            return false;
        };
        let Some(region) = world.regions.get(&handle) else { return false };
        // selectLand rejects a rectangle crossing a region boundary. Wait for
        // this rectangle's reply instead of borrowing the avatar's parcel.
        [west, south, east, north].iter().all(|v| v.is_finite())
            && west >= 0.0
            && south >= 0.0
            && east > west
            && north > south
            && east <= region.heightmap.size_x as f32
            && north <= region.heightmap.size_y as f32
            && self.parcel.as_ref().is_some_and(|(h, p)| {
                *h == handle
                    && matches!(
                        p.request_result,
                        aurora_net::land::PARCEL_RESULT_SUCCESS | aurora_net::land::PARCEL_RESULT_MULTIPLE
                    )
            })
    }

    /// LLViewerParcelMgr::startJoinLand / callbackJoinLand
    /// (indra/newview/llviewerparcelmgr.cpp). Used before confirmation and again
    /// before sending, so a stale or single-parcel selection cannot be joined.
    pub fn join_command(&mut self, world: &World) -> Option<BuildCmd> {
        if !self.can_join(world)
            || !self
                .parcel
                .as_ref()
                .is_some_and(|(_, p)| p.request_result == aurora_net::land::PARCEL_RESULT_MULTIPLE)
        {
            self.message = Some(aurora_i18n::tr!("build-land-join-select-multiple-parcels").into());
            return None;
        }
        let (handle, west, south, east, north) = self.selection_local(world)?;
        self.message = None;
        Some(BuildCmd::ParcelJoin {
            handle,
            west,
            south,
            east,
            north,
        })
    }

    /// Undo the last stroke (UndoLand to every region it touched).
    pub fn undo(&mut self, out: &mut Vec<BuildCmd>) {
        // LLToolBrushLand::undo keeps the touched regions: another Ctrl+Z
        // asks the simulator for the next step in its terrain history.
        for handle in self.last_regions.iter().copied() {
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
        // land selection: yellow walls around it (renderRect / renderHighlightSegments);
        // a selected parcel only with RenderParcelSelection
        // (LLViewerParcelMgr::render), the rectangle being dragged always
        let rect = self
            .dragging
            .map(|(a, b)| parcel_rect(a, b))
            .or(self.selection.filter(|_| s.parcel_selection_outline));
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

    fn join_fixture() -> (World, LandTool, RegionHandle) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let handle = world.main_region.expect("demo region");
        let off = world.region_offset(handle).expect("region offset").truncate();
        let tool = LandTool {
            selection: Some((off + Vec2::new(4.0, 4.0), off + Vec2::new(12.0, 8.0))),
            parcel: Some((
                handle,
                std::sync::Arc::new(aurora_net::ParcelInfo {
                    request_result: aurora_net::land::PARCEL_RESULT_MULTIPLE,
                    owner_id: uuid::Uuid::nil(),
                    area: 32,
                    ..Default::default()
                }),
            )),
            ..Default::default()
        };
        (world, tool, handle)
    }

    #[test]
    fn parcel_join_uses_the_aggregate_reply_and_region_local_rectangle() {
        let (world, mut tool, handle) = join_fixture();
        assert!(tool.can_join(&world));
        let Some(BuildCmd::ParcelJoin {
            handle: h,
            west,
            south,
            east,
            north,
        }) = tool.join_command(&world)
        else {
            panic!("multi-parcel selection can be joined");
        };
        assert_eq!((h, west, south, east, north), (handle, 4.0, 4.0, 12.0, 8.0));
        // Matching the parcel's area is not SnapSelection=true.
        assert_eq!(tool.selection_area(), tool.parcel.as_ref().unwrap().1.area as f32);
    }

    #[test]
    fn parcel_join_rechecks_single_pending_stale_and_cross_region_selections() {
        let (world, mut tool, handle) = join_fixture();
        std::sync::Arc::make_mut(&mut tool.parcel.as_mut().unwrap().1).request_result = aurora_net::land::PARCEL_RESULT_SUCCESS;
        assert!(tool.can_join(&world)); // Legacy enables the button, then explains.
        assert!(tool.join_command(&world).is_none());
        assert!(tool.take_message().is_some());
        std::sync::Arc::make_mut(&mut tool.parcel.as_mut().unwrap().1).request_result = aurora_net::land::PARCEL_RESULT_MULTIPLE;
        tool.request_parcel = true;
        assert!(!tool.can_join(&world));
        assert!(tool.join_command(&world).is_none());
        tool.request_parcel = false;
        tool.parcel.as_mut().unwrap().0 = handle.wrapping_add(1);
        assert!(!tool.can_join(&world));
        tool.parcel.as_mut().unwrap().0 = handle;
        let (lo, hi) = tool.selection.unwrap();
        tool.selection = Some((lo, lo + Vec2::splat(4.0)));
        assert!(!tool.can_join(&world));
        tool.selection = Some((lo, hi + Vec2::new(world.regions[&handle].heightmap.size_x as f32, 0.0)));
        assert!(!tool.can_join(&world));
        assert!(tool.join_command(&world).is_none());
        tool.selection = None;
        assert!(tool.join_command(&world).is_none());
    }

    #[test]
    fn parcel_join_ignores_a_previous_selection_reply_before_the_new_request() {
        let (world, mut tool, handle) = join_fixture();
        let parcel = tool.parcel.as_ref().unwrap().1.clone();
        let mut commands = Vec::new();
        tool.ask_parcel(&world, &mut commands);
        let old_seq = tool.parcel_seq;
        tool.dragging = tool.selection;
        tool.mouse_up();
        tool.on_selected_parcel(handle, old_seq, parcel.clone());
        assert!(tool.parcel.is_none());
        tool.update(
            &world,
            &BuildSettings::default(),
            &Cam::default(),
            (0.0, 0.0),
            true,
            0.016,
            &mut commands,
        );
        tool.on_selected_parcel(handle, old_seq, parcel.clone());
        assert!(tool.parcel.is_none());
        tool.on_selected_parcel(handle, tool.parcel_seq, parcel);
        assert!(tool.can_join(&world));
    }

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

    #[test]
    fn selection_preview_stays_on_cells_until_the_cursor_crosses_the_grid() {
        let a = Vec2::new(5.0, 9.0);
        let bounds = (Vec2::new(4.0, 0.0), Vec2::new(16.0, 12.0));
        for b in [Vec2::new(12.1, 0.1), Vec2::new(13.5, 3.0), Vec2::new(15.9, 3.9)] {
            assert_eq!(parcel_rect(a, b), bounds);
            assert_eq!(parcel_rect(b, a), bounds);
        }
        assert_eq!(parcel_rect(a, Vec2::new(16.1, 3.0)), (bounds.0, Vec2::new(20.0, 12.0)));
    }

    #[test]
    fn selection_preview_matches_release_in_every_direction_and_for_clicks() {
        let a = Vec2::new(-5.0, 9.0);
        for b in [
            a,
            a + Vec2::splat(0.1),
            Vec2::new(-13.5, 3.0),
            Vec2::new(1.5, 3.0),
            Vec2::new(-13.5, 17.0),
            Vec2::new(1.5, 17.0),
            Vec2::new(-4.0, 12.0),
            Vec2::new(-5.0, 17.0),
        ] {
            let preview = parcel_rect(a, b);
            let (lo, hi) = preview;
            assert_eq!(lo % PARCEL_GRID, Vec2::ZERO);
            assert_eq!(hi % PARCEL_GRID, Vec2::ZERO);
            assert!(hi.cmpge(lo + Vec2::splat(PARCEL_GRID)).all());
            let mut t = LandTool {
                dragging: Some((a, b)),
                ..Default::default()
            };
            t.mouse_up();
            assert_eq!(t.selection, Some(preview));
        }
        assert_eq!(parcel_rect(a, a), (Vec2::new(-8.0, 8.0), Vec2::new(-4.0, 12.0)));
    }

    #[test]
    fn selection_draws_the_final_cell_walls_during_the_drag() {
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let cam = Cam {
            eye: Vec3::new(12.0, -20.0, 24.0),
            view_proj: glam::Mat4::from_scale(Vec3::splat(0.04)) * glam::Mat4::from_rotation_x(0.7),
            width: 640.0,
            height: 640.0,
            ..Default::default()
        };
        let draw = |tool: &LandTool, outline| {
            let ctx = egui::Context::default();
            let settings = BuildSettings {
                land_action: 6,
                parcel_selection_outline: outline,
                ..Default::default()
            };
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 640.0))),
                    ..Default::default()
                },
                |_| {
                    let mut painter = Painter3d::new(
                        ctx.layer_painter(egui::LayerId::background()),
                        &cam,
                        crate::theme::Theme::default().palette(),
                    );
                    tool.draw(&mut painter, &world, &settings);
                },
            );
            crate::ui::login::test_output(output).shapes
        };
        let committed = LandTool {
            selection: Some((Vec2::new(4.0, 0.0), Vec2::new(16.0, 12.0))),
            ..Default::default()
        };
        let expected = draw(&committed, true);
        let empty = draw(&LandTool::default(), true);
        assert_ne!(expected, empty);
        for b in [Vec2::new(12.1, 0.1), Vec2::new(15.9, 3.9)] {
            let dragging = LandTool {
                dragging: Some((Vec2::new(5.0, 9.0), b)),
                ..Default::default()
            };
            assert_eq!(draw(&dragging, true), expected);
            assert_eq!(draw(&dragging, false), expected);
        }
        assert_eq!(draw(&committed, false), empty);
    }
}
