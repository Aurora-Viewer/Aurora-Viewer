//! Per-object flexies: world-space physics and swept geometry in parallel;
//! vertex allocations and face records stay in place between frames.
//! Source: LLVolumeImplFlexible (indra/newview/llflexibleobject.cpp, LGPL 2.1).
//!
//! The physics of every flexie within draw distance runs each frame, but
//! its mesh is swept again (and its vertices sent) only while a view can
//! show it: the camera, the water's reflection, the mirror drawn, or the
//! shadow it casts toward the sun or the moon. Measured on Agni on
//! 2026-11-04 (two busy regions at rest, ~640 flexies each): the sweeps
//! were 1.3 to 2.3 MB of vertices per frame, the largest share of the
//! render thread's `r_resources`. A held mesh keeps the last shape it was
//! seen with and takes the current one the frame it can be seen again,
//! from a simulation that never stopped. The Legacy Viewer skips the
//! flexies that are not visible too (LLVolumeImplFlexible::doIdleUpdate,
//! `isVisible`), simulation included, and spaces the updates of the small
//! ones out over up to 32 frames, which Aurora does not.
//! `AURORA_FLEXI_CULL=0` sweeps every one each frame, as before.

use super::*;
use crate::build::silhouette::{self, Topology};
use aurora_prim::flexible::Flexible;
use glam::Vec2;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// How far a held flexie's shadow is followed toward the light (m): past
/// it, a flexie outside every view is taken as casting nothing seen.
const SHADOW_REACH: f32 = 64.0;

/// The wind sensitivity a flexie takes at the least while part of it is
/// under the water, only so that `Flexible::step` takes a push at all: the
/// water carries a blade whatever its creator's wind setting (0 by default
/// in the Legacy Viewer, which would leave kelp standing still in the
/// surge), and how far does not depend on it (`WaterDrive`). Its air wind
/// keeps its own sensitivity.
const WATER_SENSITIVITY: f32 = 1.0;
/// How much of its gravity a flexie's section under the water loses to
/// buoyancy: a little more than all of it, so kelp stands up and sways
/// rather than lying on the bed.
const WATER_LIFT: f32 = 1.25;
/// How fast a section under the water moves for the water around it, left
/// to itself (times the water's speed). Water is 800 times as dense as
/// air, and its drag carries a free blade along at the water's own speed;
/// but `Flexible::step`'s tension, made for prims in the air, holds a blade
/// far stiffer than kelp: at the water's own speed a 4 m blade leant 17 cm
/// in a steady 0.4 m/s current. Four times it gives back the lean of kelp
/// in the surge. (As the air speed that drags as hard, some 48 times it
/// once the momentum is counted, a 1 m/s surge folded the demo's kelp over
/// into hooks.)
const WATER_CARRY: f32 = 4.0;
/// `Flexible::step`'s clock (s): aurora_prim's 60 Hz.
const FLEXI_STEP: f32 = 1.0 / 60.0;

/// How a flexie with some of it under the water is simulated: the wind
/// sensitivity it takes (`WATER_SENSITIVITY` at the least), and the
/// pushes, as `Flexible::update`'s wind, that carry a section along at the
/// water's speed (`carry` times it) and lift it against its gravity.
///
/// `Flexible::step` moves a section each step by its gravity times its
/// length and dt downward, by the wind times a tenth of the sensitivity,
/// its length and dt along it, and by its velocity times dt and the
/// momentum its air friction leaves (`10^-(2 f + 1) dt`): a steady push
/// `w` settles at a speed of `w · sensitivity / 10 · length / (1 -
/// momentum)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WaterDrive {
    sensitivity: f32,
    carry: f32,
    lift: f32,
}

impl WaterDrive {
    /// For a flexie of `p`, `length` m long.
    fn new(p: &aurora_prim::extra::FlexibleParams, length: f32) -> Self {
        // (the values as Flexible::new and Flexible::step take them)
        let finite = |v: f32, default: f32| if v.is_finite() { v } else { default };
        let sensitivity = finite(p.wind_sensitivity, 0.0).clamp(0.0, 10.0).max(WATER_SENSITIVITY);
        let segment = finite(length, 1.0).clamp(0.001, 64.0) / (1u32 << p.softness.min(3)) as f32;
        let friction = finite(p.air_friction, 2.0).clamp(0.0, 10.0);
        let momentum = 10.0f32.powf(-(friction * 2.0 + 1.0) * FLEXI_STEP);
        let unit = sensitivity * 0.1 * segment;
        let gravity = finite(p.gravity, 0.0).clamp(0.0, 10.0);
        Self {
            sensitivity,
            carry: WATER_CARRY * (1.0 - momentum) / unit,
            lift: gravity * WATER_LIFT / (sensitivity * 0.1),
        }
    }
}

/// `AURORA_FLEXI_WATER=0`: the flexies under the water take the air's wind
/// as the Legacy Viewer does, to compare.
fn flexi_water_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_FLEXI_WATER").as_deref() != Ok("0"))
}

/// The water a flexie stands in, sampled once a frame where it is anchored
/// (the swell's motion is the same over its few metres): the still water's
/// level, the bed under it, how fast the water moves sideways at the surface
/// (`Sea::flow`, the swell's orbital motion) and the main swell's
/// wavenumber (rad/m). Enhanced water's waves only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FlexiWater {
    pub level: f32,
    pub bed: f32,
    pub flow: Vec2,
    pub wavenumber: f32,
}

impl FlexiWater {
    /// The water where a flexie anchored at `pos` and reaching `reach` m
    /// from it stands, None when it stays above the still water.
    fn at(sea: &aurora_render::water_sea::Sea, ground: &aurora_render::WaterGround, pos: Vec3, reach: f32, clock: f32) -> Option<Self> {
        let xy = pos.truncate();
        let [bed, level] = ground.sample(xy)?;
        if pos.z - reach >= level {
            return None;
        }
        let at = ground.sea_point(xy, sea, level - bed);
        Some(Self {
            level,
            bed,
            flow: sea.flow(xy, clock, 4, &at),
            wavenumber: sea.trains[0][2],
        })
    }

    /// How fast the water moves sideways at a section at `p` (m/s); None
    /// above the still water. The water's orbital motion at the section's
    /// depth `d` over a bed `h` deep, `cosh(k (h - d)) / cosh(k h)` of the
    /// surface's (Airy wave theory: `exp(-k d)` in deep water, most of it
    /// down to the bed in the shallows).
    fn flow_at(&self, p: Vec3) -> Option<Vec2> {
        let d = self.level - p.z;
        if d.is_nan() || d <= 0.0 {
            return None;
        }
        let h = (self.level - self.bed).max(d);
        let k = self.wavenumber.max(0.0);
        // (the ratio of the cosh written with decaying exponentials only:
        // a long swell over deep water overflows the cosh itself)
        let share = (-k * d).exp() * (1.0 + (-2.0 * k * (h - d)).exp()) / (1.0 + (-2.0 * k * h).exp());
        let flow = self.flow * share.clamp(0.0, 1.0);
        flow.is_finite().then_some(flow)
    }

    /// The push on a section at `p` under the water, as `Flexible::update`'s
    /// wind for a flexie driven by `drive`; None above the still water.
    fn push(&self, p: Vec3, drive: &WaterDrive) -> Option<Vec3> {
        let push = (self.flow_at(p)? * drive.carry).extend(drive.lift);
        push.is_finite().then_some(push)
    }
}

/// AURORA_FLEXI_CULL: `0` sweeps every flexie's mesh each frame, as before;
/// `alt` does so one second in three.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CullMode {
    On,
    Off,
    Alternate,
}

fn cull_mode() -> CullMode {
    static MODE: std::sync::OnceLock<CullMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_FLEXI_CULL").as_deref() {
        Ok("0") => CullMode::Off,
        Ok("alt") => CullMode::Alternate,
        _ => CullMode::On,
    })
}

/// The old way this second, with `AURORA_FLEXI_CULL=alt`.
static OLD_NOW: AtomicBool = AtomicBool::new(false);

/// `AURORA_FLEXI_CULL=alt`: the way of second `n` (true: the old way).
/// None without `alt`.
pub fn alternate(n: u64) -> Option<bool> {
    (cull_mode() == CullMode::Alternate).then(|| {
        let old = n.is_multiple_of(3);
        OLD_NOW.store(old, Ordering::Relaxed);
        old
    })
}

fn flexi_cull() -> bool {
    match cull_mode() {
        CullMode::On => true,
        CullMode::Off => false,
        CullMode::Alternate => !OLD_NOW.load(Ordering::Relaxed),
    }
}

/// The views a flexie's mesh can be seen in this frame.
pub(super) struct FlexiViews<'a> {
    pub(super) view: &'a CullView,
    /// Directions a shadow goes from its caster: away from the sun and the
    /// moon (unit vectors).
    pub(super) shadows: [Vec3; 2],
    /// The plane (point, normal) of the mirror drawn.
    pub(super) mirror: Option<(Vec3, Vec3)>,
}

impl FlexiViews<'_> {
    /// A flexie within the sphere (`c`, `r`), over water at height `water`,
    /// can be seen: in the camera, mirrored by the water or the mirror, or
    /// by its shadow within [`SHADOW_REACH`].
    pub(super) fn show(&self, c: Vec3, r: f32, water: Option<f32>) -> bool {
        let v = self.view;
        if v.sphere_visible(c, r) {
            return true;
        }
        if water.is_some_and(|h| v.sphere_visible(Vec3::new(c.x, c.y, 2.0 * h - c.z), r)) {
            return true;
        }
        if self.mirror.is_some_and(|(p, n)| v.sphere_visible(c - 2.0 * (c - p).dot(n) * n, r)) {
            return true;
        }
        self.shadows.iter().any(|&d| capsule_visible(v, c, c + d * SHADOW_REACH, r))
    }
}

/// The capsule from `a` to `b` of radius `r` is not wholly outside any
/// plane of the view.
fn capsule_visible(v: &CullView, a: Vec3, b: Vec3, r: f32) -> bool {
    v.planes.iter().all(|p| {
        let n = p.truncate();
        (n.dot(a) + p.w).max(n.dot(b) + p.w) >= -r
    })
}

/// The sphere a flexie's mesh stays in, whatever its bend: from its centre
/// `pos`, the anchor end is half its length away and the free end at most
/// its length further, with its section around it.
fn flexi_bounds(scale: Vec3) -> f32 {
    1.5 * scale.z.abs() + scale.truncate().length()
}

pub(super) struct State {
    id: Uuid,
    pub(super) sim: Flexible,
    last: Instant,
    outline: Option<Topology>,
}

impl Scene {
    /// LLVolumeImplFlexible::getPivotPosition / getAnchorPosition: the
    /// negative-Z end, independent of the bent geometry's bounds.
    pub fn object_edit_pivot(world: &World, idx: usize, now: Instant) -> Option<Vec3> {
        let o = world.objects.get(idx)?;
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        Some(if o.is_flexible() {
            pos - rot * Vec3::Z * (world.visual_scale(idx).z * 0.5)
        } else {
            pos
        })
    }

    pub(crate) fn flexible_outline(&self, idx: usize) -> Option<&Topology> {
        self.flexibles.get(&idx)?.outline.as_ref()
    }

    pub(super) fn sync_flexibles(&mut self, renderer: &mut Renderer, world: &World, synced: &[usize], now: Instant, view: &CullView) {
        self.flexibles
            .retain(|&idx, s| world.objects.get(idx).is_some_and(|o| o.full_id == s.id && o.is_flexible()));
        for &idx in synced {
            let Some(o) = world.objects.get(idx).filter(|o| o.is_flexible()) else {
                continue;
            };
            let p = o.extra.flexible.unwrap_or(crate::build::edits::FLEX_DEFAULT);
            let Some((pos, rot, _)) = self.xf_cache.get(idx).or_else(|| Self::object_transform(world, idx, now, 0)) else {
                continue;
            };
            self.flexibles.entry(idx).or_insert_with(|| State {
                id: o.full_id,
                sim: Flexible::new(p.softness, world.visual_scale(idx).z, pos, rot),
                last: now,
                outline: None,
            });
        }
        let views = FlexiViews {
            view,
            shadows: [-world.sky_directions.0, -world.sky_directions.1],
            mirror: self.mirror_plane,
        };
        let cull = flexi_cull();
        let held = AtomicUsize::new(0);
        // enhanced water's waves: the flexies under the water sway with it
        let water_clock = self
            .flexi_water
            .filter(|(s, _)| s.enabled && s.waves && flexi_water_on())
            .map(|(_, clock)| clock);
        let (sea, ground) = (&self.lists.water_sea, &self.lists.water_ground);
        let mut states: Vec<(usize, &mut State)> = self.flexibles.iter_mut().map(|(&idx, state)| (idx, state)).collect();
        let updates = frame_par::filter_map_mut(&mut states, FLEXIBLE_GRAIN, |(idx, state)| {
            let idx = *idx;
            let o = world.objects.get(idx)?;
            let p = o.extra.flexible.unwrap_or(crate::build::edits::FLEX_DEFAULT);
            let g = self.gpu.get(idx)?;
            let key = g.geom.filter(|k| matches!(k, GeomKey::Flexible { .. }))?;
            let Some(GeomState::Ready(_)) = self.geoms.get(&key).map(|e| &e.state) else {
                return None;
            };
            if g.owner_avatar.is_some_and(|a| self.too_complex.contains(&a)) {
                return None;
            }
            let (pos, rot, hud) = self.xf_cache.get(idx).or_else(|| Self::object_transform(world, idx, now, 0))?;
            let scale = world.visual_scale(idx).max(Vec3::splat(0.001));
            if !hud && pos.distance(view.eye) > self.draw_distance + scale.length() {
                return None;
            }
            if !state.sim.matches(p.softness, scale.z) {
                state.sim.reconfigure(p.softness, scale.z, pos, rot);
            }
            let dt = now.saturating_duration_since(state.last).as_secs_f64();
            state.last = now;
            let region = world.regions.get(&o.key.region);
            let offset = world.region_offset(o.key.region).unwrap_or(Vec3::ZERO);
            let air = |sample: Vec3| region.map_or(Vec3::ZERO, |r| r.wind.velocity(sample - offset, r.heightmap.size_x as f32));
            let water = water_clock
                .filter(|_| !hud)
                .and_then(|clock| FlexiWater::at(sea, ground, pos, flexi_bounds(scale), clock));
            if let Some(water) = water {
                // its sections under the water take the water's push, those
                // above the air's wind as before (scaled back from the
                // sensitivity the water gives the whole flexie)
                let own = if p.wind_sensitivity.is_finite() {
                    p.wind_sensitivity.clamp(0.0, 10.0)
                } else {
                    0.0
                };
                let drive = WaterDrive::new(&p, scale.z);
                let sim = aurora_prim::extra::FlexibleParams {
                    wind_sensitivity: drive.sensitivity,
                    ..p
                };
                state.sim.update(sim, pos, rot, dt, |sample| {
                    water
                        .push(sample, &drive)
                        .unwrap_or_else(|| air(sample) * (own / drive.sensitivity))
                });
            } else {
                state
                    .sim
                    .update(p, pos, rot, dt, |sample| if hud { Vec3::ZERO } else { air(sample) });
            }
            // (a selected one keeps its outline in step with its mesh)
            if cull && !hud && !self.selection_silhouettes.contains(&idx) {
                let water = region.and_then(|r| r.info.as_ref()).map(|i| i.water_height);
                if !views.show(pos, flexi_bounds(scale), water) {
                    held.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            }
            let path = state.sim.render_path(pos, rot);
            let mesh = aurora_prim::volume::generate_flexible_volume(
                &o.volume,
                aurora_prim::volume::DETAIL_SCALES[g.lod as usize % 4],
                &path,
                scale,
            );
            // Only selected flexies need silhouette topology. Build it
            // from the exact rendered mesh on the same thread.
            let outline = self.selection_silhouettes.contains(&idx).then(|| silhouette::from_mesh(&mesh));
            let (faces, min, max) = jobs::volume_to_faces(&mesh);
            let prepared = jobs::PreparedGeom::new(faces, min, max, true, None);
            Some((idx, key, prepared, outline, pos, rot, scale))
        });
        self.stats.flexies = updates.len();
        self.stats.flexies_held = held.into_inner();
        for (idx, key, prepared, outline, pos, rot, scale) in updates {
            let Some(GeomEntry {
                state: GeomState::Ready(geom),
                ..
            }) = self.geoms.get_mut(&key)
            else {
                continue;
            };
            let Some(geom) = Arc::get_mut(geom) else {
                continue;
            };
            // A shape or LOD change binds another key; the topology of this
            // one stays fixed. Validate before any write, including empty faces.
            if geom.faces.len() != prepared.faces.len()
                || geom.faces.iter().zip(&prepared.faces).any(|(alloc, face)| match (alloc, face) {
                    (Some(a), Some(jobs::FaceUpload::Memory(f))) => a.vertex_count as usize != f.vertices.len(),
                    (None, None) => false,
                    _ => true,
                })
            {
                continue;
            }
            for (alloc, face) in geom.faces.iter().zip(prepared.faces) {
                if let (Some(alloc), Some(jobs::FaceUpload::Memory(face))) = (alloc, face) {
                    renderer.update_mesh_vertices(*alloc, &face.vertices);
                }
            }
            geom.min = prepared.min;
            geom.max = prepared.max;
            geom.pick_faces = prepared.pick_faces;
            if let Some(state) = self.flexibles.get_mut(&idx) {
                state.outline = outline;
            }
            let model = Mat4::from_scale_rotation_translation(scale, rot, pos);
            let g = &mut self.gpu[idx];
            g.center = model.transform_point3((geom.min + geom.max) * 0.5);
            g.radius = (((geom.max - geom.min) * scale * 0.5).length()).max(0.05);
            self.pick.changed(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(eye: Vec3, at: Vec3) -> CullView {
        let fov = 60f32.to_radians();
        let v = glam::camera::rh::view::look_at_mat4(eye, at, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 16.0 / 9.0, 0.1);
        CullView::new(proj * v, eye, 900.0, fov)
    }

    /// Shadows straight up: they reach no view of these tests.
    const NO_SHADOW: [Vec3; 2] = [Vec3::Z, Vec3::Z];

    #[test]
    fn a_flexie_in_front_shows_and_one_behind_is_held() {
        let v = view(Vec3::ZERO, Vec3::X);
        let views = FlexiViews {
            view: &v,
            shadows: NO_SHADOW,
            mirror: None,
        };
        assert!(views.show(Vec3::new(10.0, 0.0, 0.0), 0.5, None));
        assert!(!views.show(Vec3::new(-10.0, 0.0, 0.0), 0.5, None));
        // its bounds reach into the view
        assert!(views.show(Vec3::new(-0.5, 0.0, 0.0), 1.0, None));
    }

    #[test]
    fn a_flexie_behind_shows_when_its_shadow_falls_in_view() {
        // looking ahead and down from 10 m: the ground ahead is in view
        let v = view(Vec3::new(0.0, 0.0, 10.0), Vec3::new(20.0, 0.0, 0.0));
        let flexie = Vec3::new(-10.0, 0.0, 12.0);
        // sun ahead and high: the shadow goes further behind
        let ahead = FlexiViews {
            view: &v,
            shadows: [-Vec3::new(1.0, 0.0, 1.0).normalize(), Vec3::Z],
            mirror: None,
        };
        assert!(!ahead.show(flexie, 0.5, None));
        // sun behind the camera: the shadow is cast forward, into the view
        let behind = FlexiViews {
            view: &v,
            shadows: [-Vec3::new(-1.0, 0.0, 1.0).normalize(), Vec3::Z],
            mirror: None,
        };
        assert!(behind.show(flexie, 0.5, None));
        // past the reach it is held
        assert!(!behind.show(Vec3::new(-SHADOW_REACH - 10.0, 0.0, 12.0), 0.5, None));
    }

    #[test]
    fn a_flexie_shows_in_the_mirror_and_on_the_water() {
        // a mirror ahead, facing the camera, shows what is behind it
        let v = view(Vec3::ZERO, Vec3::X);
        let behind = Vec3::new(-5.0, 0.0, 0.0);
        let mirror = FlexiViews {
            view: &v,
            shadows: NO_SHADOW,
            mirror: Some((Vec3::new(10.0, 0.0, 0.0), -Vec3::X)),
        };
        assert!(mirror.show(behind, 0.5, None));
        // looking down at water 20 m high: what is above the view shows in it
        let v = view(Vec3::new(0.0, 0.0, 22.0), Vec3::new(10.0, 0.0, 10.0));
        let views = FlexiViews {
            view: &v,
            shadows: NO_SHADOW,
            mirror: None,
        };
        let above = Vec3::new(10.0, 0.0, 60.0);
        assert!(!views.show(above, 0.5, None));
        assert!(views.show(above, 0.5, Some(20.0)));
    }

    /// A swell 20 m long over water at 20 m, its bed `bed`, moving `flow`
    /// at the surface.
    fn water(bed: f32, flow: Vec2) -> FlexiWater {
        FlexiWater {
            level: 20.0,
            bed,
            flow,
            wavenumber: std::f32::consts::TAU / 20.0,
        }
    }

    #[test]
    fn the_water_s_motion_fades_with_the_depth_as_the_swell_s() {
        let w = water(-80.0, Vec2::X);
        let at = |z: f32| w.flow_at(Vec3::new(0.0, 0.0, z));
        assert_eq!(at(20.5), None, "above the still water");
        assert_eq!(at(f32::NAN), None);
        let top = at(19.99).expect("under the water").x;
        assert!((top - 1.0).abs() < 0.01, "all of it at the surface: {top}");
        // deep water: exp(-k d), a third left 3.2 m down, almost none 20 m down
        let mid = at(20.0 - 20.0 / std::f32::consts::TAU).expect("under").x;
        assert!((mid / top - (-1.0f32).exp()).abs() < 0.01, "{}", mid / top);
        assert!(at(0.0).expect("under").x < top * 0.002);
        // in the shallows, most of it down to the bed (1 / cosh(k h): 0.83
        // at the bed 2 m down, where it is none in deep water)
        let shallow = water(18.0, Vec2::X);
        let bed = shallow.flow_at(Vec3::new(0.0, 0.0, 18.01)).expect("under").x;
        assert!((bed / top - 0.83).abs() < 0.01, "{}", bed / top);
        // a long swell over a deep bed does not overflow
        let long = FlexiWater {
            wavenumber: 2.0,
            ..water(-4000.0, Vec2::X)
        };
        assert!(long.flow_at(Vec3::new(0.0, 0.0, 10.0)).is_some_and(|p| p.is_finite()));
    }

    /// The tip of a 4 m flexie of wind sensitivity `wind`, anchored on a bed
    /// at 16 m, tilted `tilt` (rad) from upright, after `seconds` in `water`
    /// (None: the air, no wind), as `sync_flexibles` drives it.
    fn kelp_tip(water: Option<FlexiWater>, wind: f32, tilt: f32, seconds: usize) -> Vec3 {
        let p = aurora_prim::extra::FlexibleParams {
            softness: 2,
            tension: 1.0,
            air_friction: 2.0,
            gravity: 3.0,
            wind_sensitivity: wind,
            user_force: Vec3::ZERO,
        };
        let (pos, rot) = (Vec3::new(0.0, 0.0, 18.0), Quat::from_rotation_y(tilt));
        let mut f = Flexible::new(p.softness, 4.0, pos, rot);
        let drive = WaterDrive::new(&p, 4.0);
        for _ in 0..seconds * 60 {
            match water {
                Some(w) => {
                    let sim = aurora_prim::extra::FlexibleParams {
                        wind_sensitivity: drive.sensitivity,
                        ..p
                    };
                    f.update(sim, pos, rot, 1.0 / 60.0, |s| w.push(s, &drive).unwrap_or(Vec3::ZERO));
                }
                None => f.update(p, pos, rot, 1.0 / 60.0, |_| Vec3::ZERO),
            }
        }
        // (the path is in the prim's frame, from its centre)
        pos + rot * f.path(pos, rot)[8].position
    }

    #[test]
    fn kelp_stands_up_in_still_water_and_leans_with_its_flow() {
        // planted 29° off upright: its tip 3.51 m over its foot at 16 m
        let tilt = 0.5f32;
        let start = 16.0 + 4.0 * tilt.cos();
        let air = kelp_tip(None, 0.0, tilt, 4);
        let still = kelp_tip(Some(water(16.0, Vec2::ZERO)), 0.0, tilt, 4);
        assert!(air.z < start - 0.3, "it droops in the air: {air}");
        assert!(still.z > start + 0.1, "it rises in the water: {still}");
        // upright, the water's flow leans it over, but not flat: tension
        // holds it back from the water's whole drift
        let flowing = kelp_tip(Some(water(16.0, Vec2::new(0.4, 0.0))), 0.0, 0.0, 4);
        assert!(flowing.x > 0.3 && flowing.z > 18.5, "it leans with the water: {flowing}");
    }

    #[test]
    fn the_water_carries_a_blade_whatever_its_wind_sensitivity() {
        let flow = Some(water(16.0, Vec2::new(0.4, 0.0)));
        let none = kelp_tip(flow, 0.0, 0.0, 3);
        let full = kelp_tip(flow, 10.0, 0.0, 3);
        assert!(none.distance(full) < 0.05, "{none} against {full}");
        // the push is set for the steady speed to be the water's
        let p = aurora_prim::extra::FlexibleParams {
            wind_sensitivity: 10.0,
            ..crate::build::edits::FLEX_DEFAULT
        };
        let drive = WaterDrive::new(&p, 4.0);
        let momentum = 10.0f32.powf(-(p.air_friction * 2.0 + 1.0) / 60.0);
        let segment = 4.0 / (1u32 << p.softness.min(3)) as f32;
        let speed = drive.carry * drive.sensitivity * 0.1 * segment / (1.0 - momentum);
        assert!((speed - WATER_CARRY).abs() < 1e-4, "{speed}");
    }

    #[test]
    fn the_bounds_hold_a_flexie_bent_any_way() {
        // anchor end at -z/2; the free end at most the length from it
        let scale = Vec3::new(0.2, 0.3, 4.0);
        let far = (scale.z * 0.5 + scale.z) + scale.truncate().length() * 0.5;
        assert!(flexi_bounds(scale) >= far);
    }
}
