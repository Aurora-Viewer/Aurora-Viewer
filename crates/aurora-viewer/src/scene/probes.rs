//! Reflection probe placement and update scheduling, after
//! LLReflectionMapManager::update / registerSpatialGroup / registerViewerObject
//! and LLReflectionMap::autoAdjustOrigin of the Second Life / Firestorm viewer
//! (originally LGPL 2.1, Copyright (C) 2022-2024 Linden Research, Inc. and the Firestorm
//! project).
//!
//! - The default probe (slot 0) sees the sky and the terrain from 64 m above
//!   the camera; it is refreshed every 2 s (RenderDefaultProbeUpdatePeriod).
//! - Automatic probes: one per 16 m cell holding objects that fit in it (LL
//!   registers a probe per 16 m octree node of the volume partition), its
//!   origin at the objects' bounds center, raised over the ground.
//! - Manual probes: prims with reflection probe parameters (sphere or box).
//!
//! Candidates are found again every 500 ms by a pass over every object,
//! spread over several frames (`ScanPass`) so that no frame pays for all of it.
//!
//! The closest probes get the cube slots; one cube face is captured per frame,
//! the probe needing it most first (never captured, then the oldest).

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
/// Automatic probes are only kept within this distance of the camera.
const AUTO_RANGE: f32 = 96.0;
const DEFAULT_PERIOD: Duration = Duration::from_secs(2);
const SCAN_PERIOD: Duration = Duration::from_millis(500);
/// The candidate scan visits every object of the scene and of the world: it
/// is spread over this many frames (in one frame it made a slow frame every
/// 500 ms in big regions), at least [`SCAN_MIN_SLICE`] slots per frame.
const SCAN_SLICES: usize = 8;
const SCAN_MIN_SLICE: usize = 512;
/// LL_PROBE flags (llprimitive.h LLReflectionProbeParams).
const FLAG_BOX: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Cell(IVec3),
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
}

/// A candidate scan in progress: the next slots to visit and what the
/// visited ones gave.
#[derive(Default)]
struct ScanPass {
    next_gpu: usize,
    next_obj: usize,
    /// Object bounds per 16 m cell (automatic probes).
    cells: HashMap<IVec3, (Vec3, Vec3)>,
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
    probes: HashMap<Key, Probe>,
    default_complete: bool,
    default_last: Option<Instant>,
    updating: Option<Updating>,
    last_scan: Option<Instant>,
    pass: Option<ScanPass>,
    last_frame: Option<Instant>,
}

impl Default for ProbeManager {
    fn default() -> Self {
        ProbeManager {
            level: level::FULL,
            slots: 32,
            probes: HashMap::new(),
            default_complete: false,
            default_last: None,
            updating: None,
            last_scan: None,
            pass: None,
            last_frame: None,
        }
    }
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
        let complete = self.probes.values().filter(|p| p.complete && p.slot.is_some()).count();
        (self.probes.len(), complete + usize::from(self.default_complete))
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
            self.pass = Some(ScanPass::default());
        }
        if let Some(mut pass) = self.pass.take() {
            if self.scan_step(&mut pass, scene, world, eye, now) {
                self.commit(pass, world);
            } else {
                self.pass = Some(pass);
            }
        }
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
                if i >= keep && !updating {
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
        for p in self.probes.values_mut() {
            if p.complete {
                p.fade = (p.fade + dt).min(1.0);
            }
        }
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
            list.push(ProbeGpu {
                center_radius: [p.origin.x, p.origin.y, p.origin.z, p.radius],
                params: [p.ambiance.max(sky_ambiance), 1.0, p.fade, p.kind],
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
    /// when due, else the closest probe never captured, else the oldest.
    fn next_capture(&mut self, order: &[Key], eye: Vec3, now: Instant) -> Option<ProbeCapture> {
        if self.updating.is_none() {
            let default_due = !self.default_complete || self.default_last.is_none_or(|t| now.duration_since(t) >= DEFAULT_PERIOD);
            let candidate = order
                .iter()
                .filter(|k| self.probes.get(k).is_some_and(|p| p.slot.is_some()))
                .min_by(|a, b| {
                    let (pa, pb) = (&self.probes[a], &self.probes[b]);
                    // never captured first (closest), then the oldest capture
                    match (pa.last_update, pb.last_update) {
                        (None, None) => pa.distance.total_cmp(&pb.distance),
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                        (Some(x), Some(y)) => x.cmp(&y),
                    }
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
                    face: u.face,
                    sky_only: true,
                    finish,
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
                        face: u.face,
                        sky_only: false,
                        finish,
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
            // object bounds per 16 m cell (objects that fit in a 16 m node)
            let end = (pass.next_gpu + n.div_ceil(SCAN_SLICES).max(SCAN_MIN_SLICE)).min(n);
            for g in scene.gpu.get(pass.next_gpu..end).unwrap_or_default() {
                if !auto_candidate(g) || (g.center - eye).length() > AUTO_RANGE + g.radius {
                    continue;
                }
                let cell = (g.center / CELL).floor().as_ivec3();
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
                let Some(rp) = o.extra.reflection_probe else {
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
    fn commit(&mut self, pass: ScanPass, world: &World) {
        let ScanPass { cells, mut found, .. } = pass;
        for (cell, (lo, hi)) in cells {
            let mut origin = (lo + hi) * 0.5;
            let half = (hi - lo) * 0.5;
            // radius encompasses all objects, at least 8 m
            let radius = half.length().max(8.0);
            // over the ground, and the near clip (half the radius) too
            let ground = world.ground_height(origin).unwrap_or(f32::MIN) + 2.0;
            origin.z = origin.z.max(ground).max(ground + radius * 0.5);
            found.insert(
                Key::Cell(cell),
                Probe {
                    origin,
                    radius,
                    kind: 0.0,
                    box_inv: Mat4::IDENTITY,
                    ambiance: 0.0,
                    near: (radius * 0.5).max(0.1),
                    slot: None,
                    complete: false,
                    last_update: None,
                    fade: 0.0,
                    distance: 0.0,
                },
            );
        }
        // keep the state of known probes; manual probes track their object
        let mut next = HashMap::with_capacity(found.len());
        for (k, mut p) in found {
            if let Some(old) = self.probes.remove(&k) {
                let moved = (old.origin - p.origin).length() > 0.5 || (old.radius - p.radius).abs() > 0.5;
                p.slot = old.slot;
                p.complete = old.complete && !moved;
                p.last_update = if moved { None } else { old.last_update };
                p.fade = if p.complete { old.fade } else { 0.0 };
            }
            next.insert(k, p);
        }
        self.probes = next;
    }
}

/// Static, non-avatar, in-world objects that fit in a 16 m octree node.
fn auto_candidate(g: &ObjGpu) -> bool {
    !g.faces.is_empty() && !g.is_avatar && g.owner_avatar.is_none() && !g.hud && g.radius <= CELL * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn candidate_scan_is_spread_over_frames() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::texture_stress_events(3000)) {
            world.apply(ev);
        }
        let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let (probe, eye) = world
            .objects
            .iter()
            .find(|(_, o)| o.extra.reflection_probe.is_some() && o.parent_id == 0)
            .map(|(_, o)| (o.full_id, o.position))
            .expect("demo reflection probe");
        let slots = world.objects.slots.len();
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
