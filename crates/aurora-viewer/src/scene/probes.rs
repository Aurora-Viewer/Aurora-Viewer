//! Reflection probe placement and update scheduling, after
//! LLReflectionMapManager::update / registerSpatialGroup / registerViewerObject
//! and LLReflectionMap::autoAdjustOrigin of the Second Life / Firestorm viewer
//! (originally LGPL 2.1, Copyright (C) 2022-2024 Linden Research, Inc. and the Firestorm
//! project).
//!
//! - The default probe (slot 0) sees the sky and the terrain from 64 m above
//!   the camera; it is refreshed every 2 s (RenderDefaultProbeUpdatePeriod).
//! - Automatic probes: one per cell holding objects that fit in a 16 m node
//!   (LL registers a probe per 16 m octree node of the volume partition),
//!   its origin at the objects' bounds center, raised over the ground. The
//!   cells are 16 m near the camera and twice as large at each doubling of
//!   the distance (`probe_cell`), up to 256 m: LL's 16 m everywhere gave a
//!   forest some 660 probes within the draw distance, the 64 slots held the
//!   nearest 70 m of them and the trees beyond lit up from the default
//!   probe, on a line around the camera. A far probe covers few pixels and
//!   shows little detail; the whole draw distance now fits the slots.
//! - Manual probes: prims with reflection probe parameters (sphere or box).
//!
//! Candidates are found again every 500 ms by a pass over every object,
//! spread over several frames (`ScanPass`) so that no frame pays for all of it.
//! An automatic probe that changes or leaves keeps showing its capture until
//! what replaces it is ready (`commit`, `retire`).
//!
//! The closest probes get the cube slots; one cube face is captured per frame,
//! the probe needing it most first (never captured by distance, then the
//! best `update_score`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aurora_render::{ProbeCapture, ProbeFrame, ProbeGpu};
use glam::{IVec3, Mat4, Vec3};
use uuid::Uuid;

use super::{ObjGpu, Scene};
use crate::world::World;

/// LLReflectionMap::ProbeLevel (RenderReflectionProbeLevel).
pub mod level {
    #[allow(dead_code)] // setting value, compared by range
    pub const NONE: u8 = 0;
    pub const MANUAL: u8 = 1;
    #[allow(dead_code)] // setting value, compared by range
    pub const MANUAL_AND_TERRAIN: u8 = 2;
    pub const FULL: u8 = 3;
}

const CELL: f32 = 16.0;
/// Coarsest automatic probe cell: 16 m << 4 = 256 m.
const MAX_LEVEL: u8 = 4;
/// The camera point the cell sizes are measured from moves on this grid
/// (m): cells change size only when the camera crosses it, not every frame.
const ANCHOR_GRID: f32 = 32.0;
/// How far the camera goes from the anchor on one axis before the anchor
/// moves (m), past the half grid where it would round to the next point: a
/// camera going back and forth over that point does not resize the cells.
const ANCHOR_STICK: f32 = 20.0;
/// How long a probe no scan finds any more waits for the new probes around
/// it to be captured, at most (`retired`).
const RETIRE_LIMIT: Duration = Duration::from_secs(5);
/// Near clip of an automatic probe's capture, at most (m): half its radius
/// (LL), which in a large cell would clip the trees around its centre away.
const MAX_AUTO_NEAR: f32 = 8.0;
/// Part of the probe field, at its far end, over which the probes fade into
/// the default one (`edge_fade`).
const EDGE_FADE: f32 = 0.3;
/// The narrowest of that fade (m), when the slots end close to the camera.
const EDGE_FADE_MIN: f32 = 8.0;
/// How far a capture draws objects (m): RenderReflectionProbeDrawDistance.
/// An automatic probe whose cell is wider sees its whole cell, what its
/// light is for (the Legacy Viewer's cells are all 16 m).
const CAPTURE_REACH: f32 = 64.0;
/// How fast the edge of the field follows its target (time constant, s).
const EDGE_EASE: f32 = 0.5;
/// How fast the edge of the field comes in at most (m/s).
const EDGE_IN_SPEED: f32 = 16.0;
const DEFAULT_PERIOD: Duration = Duration::from_secs(2);
const SCAN_PERIOD: Duration = Duration::from_millis(500);
/// The candidate scan visits every object of the scene and of the world: it
/// is spread over this many frames (in one frame it made a slow frame every
/// 500 ms in big regions), at least [`SCAN_MIN_SLICE`] slots per frame.
const SCAN_SLICES: usize = 8;
const SCAN_MIN_SLICE: usize = 512;
/// LL_PROBE flags (llprimitive.h LLReflectionProbeParams).
const FLAG_BOX: u8 = 0x01;
const FLAG_MIRROR: u8 = 0x04;
/// RenderReflectionProbeMaxLocalLightAmbiance (settings.xml): the highest
/// ambiance the local lights of an irradiance capture count for.
const MAX_LOCAL_LIGHT_AMBIANCE: f32 = 4.0;

/// What a probe's irradiance capture multiplies the local lights by
/// (LLReflectionMapManager::updateProbeFace): the capture is multiplied by the
/// probe's ambiance when sampled, so that a high ambiance does not multiply a
/// lamp by as much as the rest of the room.
fn local_light_scale(ambiance: f32) -> f32 {
    if ambiance > MAX_LOCAL_LIGHT_AMBIANCE {
        MAX_LOCAL_LIGHT_AMBIANCE / ambiance
    } else {
        1.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    /// An automatic probe's cell: its level (16 m << level) and index.
    Cell(u8, IVec3),
    Object(Uuid),
}

#[derive(Debug, Clone)]
struct Probe {
    origin: Vec3,
    radius: f32,
    /// 0 automatic, 1 manual sphere, -1 manual box.
    kind: f32,
    box_inv: Mat4,
    ambiance: f32,
    near: f32,
    slot: Option<u32>,
    complete: bool,
    last_update: Option<Instant>,
    fade: f32,
    /// Distance to the camera minus the radius (sorting, LL mDistance).
    distance: f32,
    /// When a scan stopped finding this automatic probe (its cell changed
    /// size, its objects left). Its capture stays shown until the new probes
    /// around it are captured, then it fades out (`retire`): a large cell
    /// dropped at once would flash a wide area to the default probe.
    retired: Option<Instant>,
}

/// A candidate scan in progress: the next slots to visit and what the
/// visited ones gave.
#[derive(Default)]
struct ScanPass {
    /// The camera the cell sizes and the range of this pass are measured
    /// from, on the anchor grid (`next_anchor`), taken at its start: one pass
    /// never mixes two sizes of the same place.
    anchor: Vec3,
    next_gpu: usize,
    next_obj: usize,
    /// Object bounds per cell (automatic probes, `probe_cell`).
    cells: HashMap<(u8, IVec3), (Vec3, Vec3)>,
    /// Manual probes.
    found: HashMap<Key, Probe>,
}

/// Probe currently being captured (one face per frame).
#[derive(Debug, Clone, Copy)]
struct Updating {
    /// None: the default probe.
    key: Option<Key>,
    face: u32,
}

pub struct ProbeManager {
    pub level: u8,
    /// Cube slots including the default probe's.
    pub slots: u32,
    /// "HDR and Emissive" the captures were drawn with: when it changes
    /// they are all drawn again (`refresh`).
    pub hdr: bool,
    probes: HashMap<Key, Probe>,
    default_complete: bool,
    default_last: Option<Instant>,
    updating: Option<Updating>,
    last_scan: Option<Instant>,
    pass: Option<ScanPass>,
    last_frame: Option<Instant>,
    /// The last scan's anchor (`next_anchor`).
    anchor: Option<Vec3>,
    /// Where the field of automatic probes ends now (`eased_edge`).
    edge: Option<f32>,
    /// How far from the camera automatic probes are placed (m): the
    /// Legacy Viewer keeps every probe of the objects within the draw
    /// distance (LLReflectionMapManager::update sorts them by distance and
    /// keeps the closest RenderReflectionProbeCount); Aurora's probe range
    /// setting, within the draw distance.
    pub range: f32,
}

impl Default for ProbeManager {
    fn default() -> Self {
        ProbeManager {
            level: level::FULL,
            slots: 32,
            hdr: true,
            probes: HashMap::new(),
            default_complete: false,
            default_last: None,
            updating: None,
            last_scan: None,
            pass: None,
            last_frame: None,
            anchor: None,
            edge: None,
            range: 512.0,
        }
    }
}

/// Weight of an automatic probe at `distance` (to the camera, minus its
/// radius) in a field that ends at `edge`: 1, then down to 0 over the last
/// [`EDGE_FADE`] of the field, so that the field ends in a blend into the
/// default probe, not on a line around the camera (the Legacy Viewer has the
/// line, further out with its 256 probes). A probe around the camera has its
/// full weight wherever the edge is: with few slots they can all be around it.
fn edge_fade(distance: f32, edge: f32) -> f32 {
    if distance <= 0.0 {
        return 1.0;
    }
    if edge <= 0.0 {
        return 0.0;
    }
    let band = (edge * EDGE_FADE).max(EDGE_FADE_MIN).min(edge);
    ((edge - distance) / band).clamp(0.0, 1.0)
}

/// What an automatic probe weighs in the shaders, from its fade and its place
/// in the field. Cubed: the default probe weighs so little (one over its 64 m
/// and more) that a probe holds most of a point's light down to a few percent
/// of its weight, and a straight fade would do all it shows in its last
/// moments, a step in the light. A difference with the Legacy Viewer, whose
/// mFadeIn is straight and which has no fade out.
fn auto_weight(fade: f32, edge: f32) -> f32 {
    (fade * edge).powi(3)
}

/// The edge of the field after `dt`, going towards `target`: where the slots
/// run out jumps when a probe comes or goes, and far out, where the probes are
/// sparse, two that change places can move it by tens of metres. The ambient
/// light of what is around that edge would pop with it. It comes in slowly:
/// what falls past the slots in the meantime fades out by itself.
fn eased_edge(edge: Option<f32>, target: f32, dt: f32) -> f32 {
    let Some(edge) = edge else {
        return target;
    };
    let eased = edge + (target - edge) * (1.0 - (-dt / EDGE_EASE).exp());
    eased.max(edge - EDGE_IN_SPEED * dt)
}

/// Which of two probes is captured first (check_priority,
/// indra/newview/llreflectionmapmanager.cpp): the one never captured, the
/// closer of two such; else the higher score, its age less a tenth of its
/// distance (update_score), so that a near probe is refreshed more often.
fn capture_order(a: (Option<Instant>, f32), b: (Option<Instant>, f32), now: Instant) -> std::cmp::Ordering {
    let score = |at: Instant, distance: f32| now.duration_since(at).as_secs_f32() - distance * 0.1;
    match (a.0, b.0) {
        (None, None) => a.1.total_cmp(&b.1),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(x), Some(y)) => score(y, b.1).total_cmp(&score(x, a.1)),
    }
}

/// The point of the anchor grid the cells are measured from: the nearest to
/// the camera, kept on each axis until the camera is [`ANCHOR_STICK`] from it.
fn next_anchor(previous: Option<Vec3>, eye: Vec3) -> Vec3 {
    let nearest = (eye / ANCHOR_GRID).round() * ANCHOR_GRID;
    let Some(previous) = previous else {
        return nearest;
    };
    let axis = |prev: f32, eye: f32, near: f32| if (eye - prev).abs() <= ANCHOR_STICK { prev } else { near };
    Vec3::new(
        axis(previous.x, eye.x, nearest.x),
        axis(previous.y, eye.y, nearest.y),
        axis(previous.z, eye.z, nearest.z),
    )
}

impl ProbeManager {
    /// Forget every capture (settings change, teleport).
    pub fn reset(&mut self) {
        self.probes.clear();
        self.default_complete = false;
        self.default_last = None;
        self.updating = None;
        self.last_scan = None;
        self.pass = None;
        self.anchor = None;
        self.edge = None;
    }

    /// Draw every capture again, the default probe first then the closest,
    /// each kept and shown until its new capture replaces it.
    /// handleEnableHDR resets the Legacy Viewer's probes instead: the
    /// reflections and the ambient light they give went away and came back
    /// one probe at a time, the whole scene seeming to reload.
    pub fn refresh(&mut self) {
        for p in self.probes.values_mut() {
            p.last_update = None;
        }
        self.default_last = None;
        self.updating = None;
    }

    /// Probes for the debug overlay: (origin, radius, kind: 0 automatic,
    /// 1 manual sphere, -1 manual box, box world-to-unit transform, captured).
    pub fn debug_list(&self) -> Vec<(Vec3, f32, f32, Mat4, bool)> {
        self.probes
            .values()
            .map(|p| (p.origin, p.radius, p.kind, p.box_inv, p.complete && p.slot.is_some()))
            .collect()
    }

    pub fn counts(&self) -> (usize, usize) {
        let live = || self.probes.values().filter(|p| p.retired.is_none());
        let complete = live().filter(|p| p.complete && p.slot.is_some()).count();
        (live().count(), complete + usize::from(self.default_complete))
    }

    /// Fade out and forget the retired probes whose place is taken: no probe
    /// around them is still waiting for its capture, or [`RETIRE_LIMIT`] has
    /// passed.
    fn retire(&mut self, dt: f32, now: Instant) {
        let waiting: Vec<(Vec3, f32)> = self
            .probes
            .values()
            .filter(|p| p.kind == 0.0 && p.retired.is_none() && !p.complete)
            .map(|p| (p.origin, p.radius))
            .collect();
        self.probes.retain(|_, p| {
            let Some(since) = p.retired else {
                return true;
            };
            let held =
                now.duration_since(since) < RETIRE_LIMIT && p.complete && waiting.iter().any(|(o, r)| o.distance(p.origin) < r + p.radius);
            if !held {
                p.fade -= dt;
            }
            p.fade > 0.0
        });
    }

    /// Probes to sample and the face to capture this frame.
    /// `sky_ambiance`: the sky's reflection_probe_ambiance, the minimum
    /// ambiance of every probe (LLReflectionMapManager::updateUniforms).
    pub fn update(&mut self, scene: &Scene, world: &World, eye: Vec3, sky_ambiance: f32, now: Instant) -> ProbeFrame {
        let dt = self
            .last_frame
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0)
            .min(0.25);
        self.last_frame = Some(now);
        if self.pass.is_none() && self.last_scan.is_none_or(|t| now.duration_since(t) >= SCAN_PERIOD) {
            self.last_scan = Some(now);
            let anchor = next_anchor(self.anchor, eye);
            self.anchor = Some(anchor);
            self.pass = Some(ScanPass {
                anchor,
                ..Default::default()
            });
        }
        if let Some(mut pass) = self.pass.take() {
            if self.scan_step(&mut pass, scene, world, eye, now) {
                self.commit(pass, world, now);
            } else {
                self.pass = Some(pass);
            }
        }
        self.retire(dt, now);
        // distances, then cube slots for the closest probes (slot 0 = default)
        for p in self.probes.values_mut() {
            p.distance = (p.origin - eye).length() - p.radius;
        }
        let mut order: Vec<Key> = self.probes.keys().copied().collect();
        order.sort_by(|a, b| self.probes[a].distance.total_cmp(&self.probes[b].distance));
        let keep = self.slots.saturating_sub(1) as usize;
        let mut used = vec![false; self.slots as usize];
        if !used.is_empty() {
            used[0] = true;
        }
        for (i, k) in order.iter().enumerate() {
            let updating = self.updating.is_some_and(|u| u.key == Some(*k));
            if let Some(p) = self.probes.get_mut(k) {
                // an automatic probe past the slots fades out before its cube
                // goes, and is back at once if it returns: the default probe's
                // weight is so small that even the end of a fade cut short is
                // a step in the light. A manual probe goes at once, as in
                // LLReflectionMapManager::update.
                let leaving = i >= keep && !updating;
                if leaving {
                    p.fade -= dt;
                }
                if leaving && (p.kind != 0.0 || !p.complete || p.fade <= 0.0) {
                    // free the cube index of distant probes
                    p.slot = None;
                    p.complete = false;
                    p.fade = 0.0;
                } else if let Some(s) = p.slot
                    && let Some(u) = used.get_mut(s as usize)
                {
                    *u = true;
                }
            }
        }
        for k in order.iter().take(keep) {
            if let Some(p) = self.probes.get_mut(k)
                && p.slot.is_none()
                && let Some(free) = used.iter().position(|u| !u)
            {
                used[free] = true;
                p.slot = Some(free as u32);
            }
        }
        for k in order.iter().take(keep) {
            if let Some(p) = self.probes.get_mut(k)
                && p.complete
                && p.retired.is_none()
            {
                p.fade = (p.fade + dt).min(1.0);
            }
        }
        // where the field ends: the farthest probe kept when the slots run
        // out, else the range
        let live: Vec<&Key> = order.iter().filter(|k| self.probes[*k].retired.is_none()).collect();
        let full = (keep > 0 && live.len() > keep).then(|| self.probes[live[keep - 1]].distance);
        let edge = eased_edge(self.edge, full.map_or(self.range, |f| f.min(self.range)), dt);
        self.edge = Some(edge);
        let capture = self.next_capture(&order, eye, now);
        // probe list for the shaders, closest first (refSphere order)
        let mut list = Vec::with_capacity(self.slots as usize);
        list.push(ProbeGpu {
            center_radius: [eye.x, eye.y, eye.z + 64.0, 4096.0],
            params: [sky_ambiance.max(0.0), 1.0, if self.default_complete { 1.0 } else { 0.0 }, 0.0],
            slot: [0; 4],
            box_inv: Mat4::IDENTITY.to_cols_array_2d(),
        });
        for k in &order {
            let Some(p) = self.probes.get(k) else {
                continue;
            };
            let Some(slot) = p.slot.filter(|_| p.complete) else {
                continue;
            };
            if list.len() >= self.slots as usize {
                break;
            }
            // (manual probes do not fade: LLReflectionMapManager keeps or
            // drops them by distance, and they are the creator's)
            let weight = if p.kind == 0.0 {
                auto_weight(p.fade, edge_fade(p.distance, edge))
            } else {
                p.fade
            };
            list.push(ProbeGpu {
                center_radius: [p.origin.x, p.origin.y, p.origin.z, p.radius],
                params: [p.ambiance.max(sky_ambiance), 1.0, weight, p.kind],
                slot: [slot, 0, 0, 0],
                box_inv: p.box_inv.to_cols_array_2d(),
            });
        }
        ProbeFrame {
            enabled: self.default_complete,
            probes: list,
            capture,
        }
    }

    /// Pick the probe face to render (LLReflectionMapManager::update /
    /// doProbeUpdate): finish the probe in progress, else the default probe
    /// when due, else the first by `capture_order`.
    fn next_capture(&mut self, order: &[Key], eye: Vec3, now: Instant) -> Option<ProbeCapture> {
        if self.updating.is_none() {
            let default_due = !self.default_complete || self.default_last.is_none_or(|t| now.duration_since(t) >= DEFAULT_PERIOD);
            // (a probe fading out past the slots is not drawn again)
            let keep = self.slots.saturating_sub(1) as usize;
            let candidate = order
                .iter()
                .take(keep)
                .filter(|k| self.probes.get(k).is_some_and(|p| p.slot.is_some() && p.retired.is_none()))
                .min_by(|a, b| {
                    let (pa, pb) = (&self.probes[a], &self.probes[b]);
                    capture_order((pa.last_update, pa.distance), (pb.last_update, pb.distance), now)
                })
                .copied();
            let first_pass = candidate.is_some_and(|k| self.probes.get(&k).is_some_and(|p| !p.complete));
            self.updating = if default_due && !(self.default_complete && first_pass) {
                Some(Updating { key: None, face: 0 })
            } else {
                candidate.map(|k| Updating { key: Some(k), face: 0 })
            };
        }
        let u = self.updating?;
        let finish = u.face == 5;
        let capture = match u.key {
            None => {
                if finish {
                    self.default_complete = true;
                    self.default_last = Some(now);
                }
                Some(ProbeCapture {
                    slot: 0,
                    origin: eye + Vec3::Z * 64.0,
                    near: 1.0,
                    reach: 0.0,
                    face: u.face,
                    sky_only: true,
                    finish,
                    light_scale: 1.0,
                })
            }
            Some(k) => match self.probes.get_mut(&k) {
                Some(p) if p.slot.is_some() => {
                    let slot = p.slot.unwrap_or(0);
                    if finish {
                        p.complete = true;
                        p.last_update = Some(now);
                    }
                    Some(ProbeCapture {
                        slot,
                        origin: p.origin,
                        near: p.near,
                        reach: if p.kind == 0.0 {
                            p.radius.max(CAPTURE_REACH)
                        } else {
                            CAPTURE_REACH
                        },
                        face: u.face,
                        sky_only: false,
                        finish,
                        light_scale: local_light_scale(p.ambiance),
                    })
                }
                // the probe went away or lost its slot: start over next frame
                _ => None,
            },
        };
        self.updating = if capture.is_none() || finish {
            None
        } else {
            Some(Updating { face: u.face + 1, ..u })
        };
        capture
    }

    /// One slice of the candidate scan; true when the pass has covered every
    /// object of the scene and of the world.
    fn scan_step(&self, pass: &mut ScanPass, scene: &Scene, world: &World, eye: Vec3, now: Instant) -> bool {
        let n = scene.gpu.len();
        if self.level >= level::FULL {
            // object bounds per cell (objects that fit in a 16 m node)
            // (from the anchor, not the camera: walking does not move a
            // large cell's bounds at each scan as objects enter its range)
            let anchor = pass.anchor;
            let end = (pass.next_gpu + n.div_ceil(SCAN_SLICES).max(SCAN_MIN_SLICE)).min(n);
            for g in scene.gpu.get(pass.next_gpu..end).unwrap_or_default() {
                if !auto_candidate(g) || (g.center - anchor).length() > self.range + g.radius {
                    continue;
                }
                let cell = probe_cell(g.center, anchor);
                let lo = g.center - Vec3::splat(g.radius);
                let hi = g.center + Vec3::splat(g.radius);
                pass.cells
                    .entry(cell)
                    .and_modify(|(a, b)| {
                        *a = a.min(lo);
                        *b = b.max(hi);
                    })
                    .or_insert((lo, hi));
            }
            pass.next_gpu = end;
        } else {
            pass.next_gpu = n;
        }
        let slots = world.objects.slots.len();
        if self.level >= level::MANUAL {
            let end = (pass.next_obj + slots.div_ceil(SCAN_SLICES).max(SCAN_MIN_SLICE)).min(slots);
            for idx in pass.next_obj..end {
                let Some(o) = world.objects.get(idx) else {
                    continue;
                };
                // a mirror is a hero probe only, never a cube map of its own
                // (LLVOVolume::updateReflectionProbePtr): its box would show
                // the place without avatars wherever the mirror fades out
                let Some(rp) = o.extra.reflection_probe.filter(|rp| rp.flags & FLAG_MIRROR == 0) else {
                    continue;
                };
                let Some((pos, rot, hud)) = Scene::object_transform(world, idx, now, 0) else {
                    continue;
                };
                if hud || (pos - eye).length() > 256.0 {
                    continue;
                }
                let is_box = rp.flags & FLAG_BOX != 0;
                let (radius, kind, box_inv) = if is_box {
                    let half = o.scale * 0.5;
                    let m = Mat4::from_scale_rotation_translation(half.max(Vec3::splat(0.01)), rot, pos);
                    (half.length(), -1.0, m.inverse())
                } else {
                    (o.scale.x * 0.5, 1.0, Mat4::IDENTITY)
                };
                pass.found.insert(
                    Key::Object(o.full_id),
                    Probe {
                        origin: pos,
                        radius: radius.max(0.1),
                        kind,
                        box_inv,
                        ambiance: rp.ambiance.max(0.0),
                        near: rp.clip_distance.max(0.1),
                        slot: None,
                        complete: false,
                        last_update: None,
                        fade: 0.0,
                        distance: 0.0,
                        retired: None,
                    },
                );
            }
            pass.next_obj = end;
        } else {
            pass.next_obj = slots;
        }
        pass.next_gpu >= scene.gpu.len() && pass.next_obj >= world.objects.slots.len()
    }

    /// A finished pass becomes the candidate probes.
    fn commit(&mut self, pass: ScanPass, world: &World, now: Instant) {
        let ScanPass { cells, mut found, .. } = pass;
        for (cell, (lo, hi)) in cells {
            let mut origin = (lo + hi) * 0.5;
            let half = (hi - lo) * 0.5;
            // radius encompasses all objects, at least 8 m
            let radius = half.length().max(8.0);
            // over the ground, and the near clip (half the radius, at most
            // MAX_AUTO_NEAR) too: lifted by half the radius of a large cell,
            // a probe floated over the canopy or a tunnel's roof and saw
            // the open sky
            let near = (radius * 0.5).clamp(0.1, MAX_AUTO_NEAR);
            let ground = world.ground_height(origin).unwrap_or(f32::MIN) + 2.0;
            origin.z = origin.z.max(ground).max(ground + near);
            found.insert(
                Key::Cell(cell.0, cell.1),
                Probe {
                    origin,
                    radius,
                    kind: 0.0,
                    box_inv: Mat4::IDENTITY,
                    ambiance: 0.0,
                    near,
                    slot: None,
                    complete: false,
                    last_update: None,
                    fade: 0.0,
                    distance: 0.0,
                    retired: None,
                },
            );
        }
        // keep the state of known probes; manual probes track their object
        let mut next = HashMap::with_capacity(found.len());
        for (k, mut p) in found {
            if let Some(old) = self.probes.remove(&k) {
                let moved = (old.origin - p.origin).length() > 0.5 || (old.radius - p.radius).abs() > 0.5;
                p.slot = old.slot;
                // a manual probe that moved is drawn again before it shows,
                // as in the Legacy Viewer. An automatic one keeps showing its
                // capture until the new one replaces it (`refresh`), its
                // first in line: objects entering a large cell would else
                // send a wide area to the default probe at each scan.
                p.complete = old.complete && (!moved || p.kind == 0.0);
                p.last_update = if moved { None } else { old.last_update };
                p.fade = if p.complete { old.fade } else { 0.0 };
            }
            next.insert(k, p);
        }
        for (k, mut old) in self.probes.drain() {
            if old.kind == 0.0 && old.complete && old.slot.is_some() {
                old.retired.get_or_insert(now);
                next.insert(k, old);
            }
        }
        self.probes = next;
    }
}

/// The automatic probe cell of a point: its level (16 m << level) and its
/// index at that level. A cell is used when its centre is at least its own
/// size from `anchor`, the coarsest level first: about ten cells per
/// doubling of the distance, and since the cells nest, the points of one
/// cell all get it, and no two levels cover the same place.
fn probe_cell(p: Vec3, anchor: Vec3) -> (u8, IVec3) {
    for level in (1..=MAX_LEVEL).rev() {
        let size = CELL * f32::from(1u16 << level);
        let cell = (p / size).floor();
        if ((cell + Vec3::splat(0.5)) * size).distance(anchor) >= size {
            return (level, cell.as_ivec3());
        }
    }
    (0, (p / CELL).floor().as_ivec3())
}

/// Static, non-avatar, in-world objects that fit in a 16 m octree node.
fn auto_candidate(g: &ObjGpu) -> bool {
    !g.faces.is_empty() && !g.is_avatar && g.owner_avatar.is_none() && !g.hud && g.radius <= CELL * 0.5
}

#[cfg(test)]
#[path = "probes_walk.rs"]
mod walk;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn only_a_high_ambiance_dims_the_local_lights_of_its_capture() {
        for ambiance in [0.0, 1.0, 4.0] {
            assert_eq!(local_light_scale(ambiance), 1.0, "ambiance {ambiance}");
        }
        assert_eq!(local_light_scale(8.0), 0.5);
        assert_eq!(local_light_scale(40.0), 0.1);
    }

    #[test]
    fn probe_cells_grow_with_the_distance() {
        let anchor = Vec3::new(128.0, 128.0, 32.0);
        let size = |p: Vec3| CELL * f32::from(1u16 << probe_cell(p, anchor).0);
        assert_eq!(size(anchor + Vec3::new(5.0, 3.0, 0.0)), 16.0);
        assert!(size(anchor + Vec3::new(100.0, 0.0, 0.0)) >= 32.0);
        assert!(size(anchor + Vec3::new(300.0, 0.0, 0.0)) >= 128.0);
        // the points of a cell share it: nothing in it gets a finer probe
        let far = anchor + Vec3::new(200.0, 40.0, 0.0);
        let (level, cell) = probe_cell(far, anchor);
        let s = CELL * f32::from(1u16 << level);
        let lo = cell.as_vec3() * s;
        for k in 0..64 {
            let t = Vec3::new((k % 4) as f32, ((k / 4) % 4) as f32, (k / 16) as f32) / 4.0 + Vec3::splat(0.1);
            assert_eq!(probe_cell(lo + t * s, anchor), (level, cell));
        }
        // one layer of a forest within 230 m takes less than 64 cells (one
        // per column); a real forest spreads over its height too, about 213
        // on the grid, which needs more slots than the default
        let mut cells = HashSet::new();
        for x in -15..15 {
            for y in -15..15 {
                let p = anchor + Vec3::new(x as f32 * 16.0 + 8.0, y as f32 * 16.0 + 8.0, -20.0);
                if (p - anchor).length() <= 230.0 {
                    cells.insert(probe_cell(p, anchor));
                }
            }
        }
        assert!(cells.len() < 64, "{}", cells.len());
    }

    #[test]
    fn a_probe_past_the_slots_fades_out_before_it_loses_its_cube() {
        let world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let start = Instant::now();
        let mut m = ProbeManager {
            slots: 3,
            default_complete: true,
            default_last: Some(start),
            last_scan: Some(start),
            last_frame: Some(start),
            ..Default::default()
        };
        // (four probes for two slots: the last two are past them)
        let at = |x: f32, slot: u32| Probe {
            slot: Some(slot),
            last_update: Some(start),
            ..cell_probe(Vec3::new(x, 0.0, 0.0), 1.0, true)
        };
        m.probes.insert(cell_key(0), at(10.0, 1));
        m.probes.insert(cell_key(1), at(20.0, 2));
        let mut far = at(30.0, 4);
        far.fade = 0.5;
        m.probes.insert(cell_key(2), far);
        let manual = Key::Object(Uuid::from_u128(1));
        m.probes.insert(manual, Probe { kind: 1.0, ..at(40.0, 5) });
        let tick = Duration::from_millis(100);
        m.update(&scene, &world, Vec3::ZERO, 0.0, start + tick);
        // the automatic one keeps its cube and fades; the manual one goes
        let p = &m.probes[&cell_key(2)];
        assert!(p.complete && p.slot.is_some() && (p.fade - 0.4).abs() < 1e-3, "{}", p.fade);
        assert!(m.probes[&manual].slot.is_none() && !m.probes[&manual].complete);
        // back within the slots: it stays and fades in, with no new capture
        m.probes.remove(&cell_key(0));
        m.update(&scene, &world, Vec3::ZERO, 0.0, start + 2 * tick);
        let p = &m.probes[&cell_key(2)];
        assert!(p.complete && p.slot.is_some() && (p.fade - 0.5).abs() < 1e-3, "{}", p.fade);
        // still past them: at zero, the cube is free
        m.probes.insert(cell_key(0), at(10.0, 1));
        m.probes.insert(cell_key(3), at(15.0, 6));
        m.update(&scene, &world, Vec3::ZERO, 0.0, start + 10 * tick);
        let p = &m.probes[&cell_key(2)];
        assert!(p.slot.is_none() && !p.complete && p.fade == 0.0);
    }

    #[test]
    fn captures_follow_the_legacy_priority() {
        use std::cmp::Ordering::{Greater, Less};
        let now = Instant::now() + Duration::from_secs(100);
        let ago = |s: u64| Some(now - Duration::from_secs(s));
        // never captured first, the closer of two
        assert_eq!(capture_order((None, 500.0), (ago(90), 0.0), now), Less);
        assert_eq!(capture_order((ago(90), 0.0), (None, 500.0), now), Greater);
        assert_eq!(capture_order((None, 5.0), (None, 50.0), now), Less);
        // the older of two at the same distance
        assert_eq!(capture_order((ago(20), 10.0), (ago(10), 10.0), now), Less);
        // a near probe before an older far one: 5 - 0 against 8 - 10
        assert_eq!(capture_order((ago(5), 0.0), (ago(8), 100.0), now), Less);
        // until the far one has waited long enough: 5 - 0 against 16 - 10
        assert_eq!(capture_order((ago(5), 0.0), (ago(16), 100.0), now), Greater);
    }

    #[test]
    fn the_anchor_stays_while_the_camera_crosses_its_rounding_point() {
        let first = next_anchor(None, Vec3::new(15.0, 0.0, 0.0));
        assert_eq!(first, Vec3::ZERO);
        // back and forth over 16 m, where rounding alone would flip it
        for x in [17.0, 15.0, 18.0, 14.0, 20.0] {
            assert_eq!(next_anchor(Some(first), Vec3::new(x, 0.0, 0.0)), first, "{x}");
        }
        let moved = next_anchor(Some(first), Vec3::new(21.0, 0.0, 0.0));
        assert_eq!(moved, Vec3::new(32.0, 0.0, 0.0));
        // and it does not come back at the same point
        assert_eq!(next_anchor(Some(moved), Vec3::new(15.0, 0.0, 0.0)), moved);
        // each axis on its own
        assert_eq!(next_anchor(Some(first), Vec3::new(5.0, -40.0, 3.0)), Vec3::new(0.0, -32.0, 0.0));
    }

    fn cell_probe(origin: Vec3, radius: f32, complete: bool) -> Probe {
        Probe {
            origin,
            radius,
            kind: 0.0,
            box_inv: Mat4::IDENTITY,
            ambiance: 0.0,
            near: 1.0,
            slot: complete.then_some(1),
            complete,
            last_update: None,
            fade: if complete { 1.0 } else { 0.0 },
            distance: 0.0,
            retired: None,
        }
    }

    fn cell_key(x: i32) -> Key {
        Key::Cell(0, IVec3::new(x, 0, 0))
    }

    #[test]
    fn a_retired_probe_stays_until_the_probes_around_it_are_captured() {
        let now = Instant::now();
        let mut m = ProbeManager::default();
        let mut old = cell_probe(Vec3::ZERO, 50.0, true);
        old.retired = Some(now);
        m.probes.insert(cell_key(0), old);
        m.probes.insert(cell_key(1), cell_probe(Vec3::new(30.0, 0.0, 0.0), 20.0, false));
        // the new probe around it is not captured: full weight
        m.retire(0.5, now);
        assert_eq!(m.probes[&cell_key(0)].fade, 1.0);
        assert_eq!(m.counts().0, 1, "the retired probe is not a candidate");
        // captured: it fades out, then goes
        m.probes.get_mut(&cell_key(1)).expect("new probe").complete = true;
        m.retire(0.5, now);
        assert_eq!(m.probes[&cell_key(0)].fade, 0.5);
        m.retire(0.5, now);
        assert!(!m.probes.contains_key(&cell_key(0)));
    }

    #[test]
    fn a_retired_probe_does_not_wait_for_ever_nor_for_a_probe_elsewhere() {
        let now = Instant::now();
        let mut m = ProbeManager::default();
        let mut old = cell_probe(Vec3::ZERO, 10.0, true);
        old.retired = Some(now);
        m.probes.insert(cell_key(0), old);
        let mut near = cell_probe(Vec3::new(15.0, 0.0, 0.0), 10.0, false);
        near.slot = None;
        m.probes.insert(cell_key(1), near);
        m.retire(0.25, now + RETIRE_LIMIT);
        assert_eq!(m.probes[&cell_key(0)].fade, 0.75, "past the limit");
        m.probes.get_mut(&cell_key(0)).expect("old probe").fade = 1.0;
        m.probes.get_mut(&cell_key(1)).expect("new probe").origin = Vec3::new(500.0, 0.0, 0.0);
        m.retire(0.25, now);
        assert_eq!(m.probes[&cell_key(0)].fade, 0.75, "nothing waits around it");
    }

    #[test]
    fn a_scan_keeps_what_it_no_longer_finds_shown() {
        let world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let now = Instant::now();
        let mut m = ProbeManager::default();
        let cell = |lo: f32, hi: f32| (Vec3::splat(lo), Vec3::splat(hi));
        let pass = |cells: &[((u8, IVec3), (Vec3, Vec3))]| ScanPass {
            cells: cells.iter().copied().collect(),
            ..Default::default()
        };
        let (a, b) = ((0u8, IVec3::ZERO), (1u8, IVec3::ONE));
        m.commit(pass(&[(a, cell(40.0, 50.0))]), &world, now);
        let ka = Key::Cell(a.0, a.1);
        {
            let p = m.probes.get_mut(&ka).expect("probe");
            p.complete = true;
            p.slot = Some(3);
            p.fade = 1.0;
            p.last_update = Some(now);
        }
        // its bounds move: still shown, and first in line for a new capture
        m.commit(pass(&[(a, cell(40.0, 60.0))]), &world, now);
        let p = &m.probes[&ka];
        assert!(p.complete && p.fade == 1.0 && p.retired.is_none() && p.slot == Some(3));
        assert!(p.last_update.is_none());
        // gone from the scan: retired, not dropped; found again: back
        m.commit(pass(&[(b, cell(40.0, 60.0))]), &world, now);
        assert_eq!(m.probes[&ka].retired, Some(now));
        assert!(!m.probes[&Key::Cell(b.0, b.1)].complete);
        m.commit(pass(&[(a, cell(40.0, 60.0))]), &world, now);
        assert!(m.probes[&ka].retired.is_none() && m.probes[&ka].complete);
    }

    #[test]
    fn a_manual_probe_that_moved_is_captured_again_before_it_shows() {
        let world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let now = Instant::now();
        let mut m = ProbeManager::default();
        let id = Uuid::from_u128(7);
        let manual = |x: f32| Probe {
            kind: 1.0,
            ..cell_probe(Vec3::new(x, 0.0, 0.0), 5.0, false)
        };
        m.probes.insert(
            Key::Object(id),
            Probe {
                complete: true,
                slot: Some(2),
                fade: 1.0,
                ..manual(0.0)
            },
        );
        let pass = |x: f32| ScanPass {
            found: [(Key::Object(id), manual(x))].into_iter().collect(),
            ..Default::default()
        };
        m.commit(pass(0.0), &world, now);
        assert!(m.probes[&Key::Object(id)].complete, "it did not move");
        m.commit(pass(3.0), &world, now);
        assert!(!m.probes[&Key::Object(id)].complete);
        // and one no scan finds goes at once
        m.commit(ScanPass::default(), &world, now);
        assert!(!m.probes.contains_key(&Key::Object(id)));
    }

    #[test]
    fn probes_fade_out_at_the_edge_of_the_field() {
        // full weight in the near 70 %, none at the edge, a ramp between
        assert_eq!(edge_fade(10.0, 100.0), 1.0);
        assert!((edge_fade(70.0, 100.0) - 1.0).abs() < 1e-5);
        assert!((edge_fade(85.0, 100.0) - 0.5).abs() < 1e-5);
        assert_eq!(edge_fade(100.0, 100.0), 0.0);
        assert_eq!(edge_fade(120.0, 100.0), 0.0);
        // around the camera: full weight, wherever the edge is
        assert_eq!(edge_fade(-5.0, -5.0), 1.0);
        assert_eq!(edge_fade(-100.0, 100.0), 1.0);
        assert_eq!(edge_fade(0.0, 3.0), 1.0);
        // slots ending close to the camera: no step at its surface, a ramp
        // wider than a metre, and nothing past an edge inside the camera's
        assert_eq!(edge_fade(0.5, 20.0), 1.0);
        assert!((edge_fade(16.0, 20.0) - 0.5).abs() < 1e-5);
        assert!((edge_fade(1.5, 3.0) - 0.5).abs() < 1e-5);
        assert_eq!(edge_fade(5.0, -5.0), 0.0);
    }

    #[test]
    fn an_automatic_probe_ends_its_fade_gently() {
        assert_eq!(auto_weight(1.0, 1.0), 1.0);
        assert_eq!(auto_weight(0.0, 1.0), 0.0);
        assert_eq!(auto_weight(1.0, 0.0), 0.0);
        // a tenth of the way from the end, a thousandth of the weight
        assert!((auto_weight(0.1, 1.0) - 0.001).abs() < 1e-6);
    }

    #[test]
    fn the_edge_of_the_field_eases_to_its_target() {
        assert_eq!(eased_edge(None, 230.0, 0.016), 230.0, "the first frame takes it");
        // the slots run out: from the range towards a closer edge
        let mut edge = 230.0;
        let first = eased_edge(Some(edge), 60.0, 0.016);
        assert!((first - 230.0).abs() < 7.0, "no jump in a frame: {first}");
        for _ in 0..1000 {
            edge = eased_edge(Some(edge), 60.0, 0.016);
        }
        assert!((edge - 60.0).abs() < 1e-1, "{edge}");
        // however far it has to go in, it comes in slowly; out, it eases
        assert!((eased_edge(Some(230.0), 10.0, 0.1) - 228.4).abs() < 1e-3);
        assert!(eased_edge(Some(60.0), 230.0, 0.1) > 90.0);
        // a long frame does not overshoot
        assert!(eased_edge(Some(230.0), 60.0, 100.0) >= 60.0);
    }

    #[test]
    fn only_automatic_probes_fade_at_the_edge() {
        let world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let now = Instant::now();
        let mut m = ProbeManager {
            slots: 4,
            default_complete: true,
            default_last: Some(now),
            last_scan: Some(now),
            edge: Some(10.0),
            ..Default::default()
        };
        let at = |x: f32| cell_probe(Vec3::new(x, 0.0, 0.0), 1.0, true);
        m.probes.insert(cell_key(0), at(5.0));
        m.probes.insert(Key::Object(Uuid::from_u128(1)), Probe { kind: 1.0, ..at(9.0) });
        m.probes.insert(Key::Object(Uuid::from_u128(2)), Probe { kind: -1.0, ..at(10.0) });
        m.probes.insert(cell_key(1), at(11.0));
        m.probes.insert(cell_key(2), at(400.0));
        for p in m.probes.values_mut() {
            p.last_update = Some(now);
        }
        let frame = m.update(&scene, &world, Vec3::ZERO, 0.0, now);
        let weights: Vec<(f32, f32)> = frame.probes[1..].iter().map(|p| (p.params[3], p.params[2])).collect();
        // the first three by distance; both manual ones whole, past the edge
        assert_eq!(weights.len(), 3, "{weights:?}");
        assert_eq!(weights[1], (1.0, 1.0));
        assert_eq!(weights[2], (-1.0, 1.0));
        assert_eq!(weights[0].0, 0.0);
        assert!(weights[0].1 > 0.0 && weights[0].1 <= 1.0, "{weights:?}");
    }

    #[test]
    fn candidate_scan_is_spread_over_frames() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::texture_stress_events(3000)) {
            world.apply(ev);
        }
        let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let (idx, probe, eye) = world
            .objects
            .iter()
            .find(|(_, o)| o.extra.reflection_probe.is_some() && o.parent_id == 0)
            .map(|(idx, o)| (idx, o.full_id, o.position))
            .expect("demo reflection probe");
        // the demo's only probe is the mirror's, which is no cube map: the
        // whole scan passes over it
        let slots = world.objects.slots.len();
        let mut m = ProbeManager::default();
        let now = Instant::now();
        for _ in 0..slots.div_ceil(SCAN_MIN_SLICE) {
            m.update(&scene, &world, eye, 0.0, now);
        }
        assert!(m.pass.is_none(), "scan not finished");
        assert!(!m.probes.contains_key(&Key::Object(probe)), "mirror probe made a cube map");
        // as an ordinary box probe it is found at the end of the scan
        if let Some(rp) = world.objects.get_mut(idx).and_then(|o| o.extra.reflection_probe.as_mut()) {
            rp.flags &= !FLAG_MIRROR;
        }
        let steps = slots.div_ceil(slots.div_ceil(SCAN_SLICES).max(SCAN_MIN_SLICE));
        assert!(steps > 1, "{slots} slots");
        let mut m = ProbeManager::default();
        let now = Instant::now();
        for _ in 1..steps {
            m.update(&scene, &world, eye, 0.0, now);
            assert!(!m.probes.contains_key(&Key::Object(probe)), "pass finished too early");
        }
        m.update(&scene, &world, eye, 0.0, now);
        assert!(m.probes.contains_key(&Key::Object(probe)));
        // the next pass waits for the scan period
        assert!(m.pass.is_none());
        m.update(&scene, &world, eye, 0.0, now);
        assert!(m.pass.is_none());
        m.update(&scene, &world, eye, 0.0, now + SCAN_PERIOD);
        assert!(m.pass.is_some());
    }
}
