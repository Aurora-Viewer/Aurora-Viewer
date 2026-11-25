//! Visual contacts derived from displayed transforms and animated joints.
//! No messages, forces, scripts or physics changes are sent to the grid.

use super::{Scene, anchors, anim, water_contacts};
use crate::frame_par::Grain;
use crate::world::World;
use aurora_render::water_effects::SEA_EXTENT;
use aurora_render::{WaterEffects, WaterEvent, WaterGround};
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// How the two periodic loops are shared with the pool's workers
/// (frame_par.rs); the costs are those of the dev profile. As plain rayon
/// loops, called from the main thread, they waited for a worker of the pool:
/// the ground heights 10 to 16 ms on a busy region, for 0.1 ms of work.
/// A row of the ground grid: 2 µs.
const GROUND_GRAIN: Grain = Grain { chunk: 4, per_helper: 8 };
/// An object of the candidate scan: 24 ns.
const SCAN_GRAIN: Grain = Grain { chunk: 512, per_helper: 8 };

#[derive(Default)]
pub struct Effects {
    pub geometry: Option<u8>,
    region: Option<u64>,
    candidates: Vec<usize>,
    scan_at: Option<f32>,
    ground_at: f32,
    ground: WaterGround,
    history: HashMap<(Uuid, u8), Contact>,
    events: Vec<WaterEvent>,
    impacts: Vec<crate::movement::water_jump::Impact>,
    bodies: water_contacts::Bodies,
    visible: HashMap<Uuid, u32>,
    /// The wind's sea (aurora_render::water_sea): advanced once a frame,
    /// shared with the renderer through the lists.
    sea: aurora_render::water_sea::SeaDriver,
    /// A sea grid baking on a worker.
    sea_baking: Option<SeaBaking>,
    /// The last sea grid baked: where, when it was last checked, what it
    /// was made of, and the grid.
    sea_grid: Option<SeaGrid>,
    /// When the sea's state was last written to the log (every 10 s).
    sea_logged_at: f32,
    /// The static prims standing in the sea, as last gathered for its grid,
    /// and where its waves crash on them and on steep shores
    /// (water_crash.rs).
    crash_boxes: Vec<super::water_shelter::ShelterBox>,
    /// Each one's object (its index in the world), for its real surface.
    crash_objects: Vec<usize>,
    /// What the last grid baked was made of (`parts` above), to say what
    /// changed when it is baked again.
    sea_parts: [u64; 4],
    crashes: super::water_crash::Crashes,
    /// Water dripping off the wet avatars (avatar_drip.rs).
    drip: super::avatar_drip::AvatarDrip,
    /// Spray blown off the breaking crests in a strong wind
    /// (water_spindrift.rs).
    spindrift: super::water_spindrift::Spindrift,
    /// The sea grid's shelter prims at the last scan (full id, local id,
    /// where and how big, quantized as the grid's signature), and those
    /// seen moving or dropping out while near the water, with when: left out
    /// of the shelter for `MOVER_HOLD` s (a scripted prim that steps now and
    /// then went in and out of it, baking the grid again every few seconds).
    shelter_seen: HashMap<Uuid, (u32, [i32; 10])>,
    shelter_movers: HashMap<Uuid, f32>,
    /// The shelter prims' boxes at the last scan, and those of the prims
    /// that dropped out of it unmoved while still near the water, kept in
    /// it with when (the region sends objects' updates again as the view
    /// turns, which counted as a move of their own: whole linksets dropped
    /// out and came back, a new grid each time).
    shelter_boxes: HashMap<Uuid, (usize, super::water_shelter::ShelterBox)>,
    shelter_pinned: HashMap<Uuid, (usize, super::water_shelter::ShelterBox, f32)>,
}

/// The sea grid baked last (water_shelter.rs) and the one before it,
/// fading out over `SEA_FADE` after each bake.
struct SeaGrid {
    origin: Vec2,
    /// When it was last checked for a change, and what it was made of.
    checked_at: f32,
    signature: u64,
    grid: std::sync::Arc<[[f32; 6]]>,
    /// The previous grid (all of its cell: depth, shelter, refraction offset,
    /// swell reach and chop) on this grid's cells, and when this one was baked.
    prev: std::sync::Arc<[[f32; 6]]>,
    baked_at: f32,
}

/// A sea grid baking on a worker thread (water_shelter::bake_grid).
struct SeaBaking {
    origin: Vec2,
    signature: u64,
    result: std::thread::JoinHandle<Vec<[f32; 6]>>,
}

/// How long a new sea grid takes to replace the previous one (s).
const SEA_FADE: f32 = 6.0;
/// How far past the next grid step (m) the camera goes before the sea grid
/// follows it.
const GRID_HOLD: f32 = 24.0;

impl SeaGrid {
    /// How far the fade has gone at `time` (0 the previous grid, 1 this).
    fn blend(&self, time: f32) -> f32 {
        let t = ((time - self.baked_at) / SEA_FADE).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// What it shows at `time` (all of a cell), moved onto a grid at
    /// `origin` (cells it no longer covers: `fresh`'s), for the next grid to
    /// fade from.
    fn shown(&self, time: f32, origin: Vec2, fresh: &[[f32; 6]]) -> Vec<[f32; 6]> {
        use aurora_render::water_effects::SEA_GRID;
        let blend = self.blend(time);
        let cell = aurora_render::water_effects::SEA_EXTENT / SEA_GRID as f32;
        let shift = ((origin - self.origin) / cell).round().as_ivec2();
        (0..SEA_GRID * SEA_GRID)
            .map(|i| {
                let (x, y) = ((i % SEA_GRID) as i32 + shift.x, (i / SEA_GRID) as i32 + shift.y);
                if x < 0 || y < 0 || x >= SEA_GRID as i32 || y >= SEA_GRID as i32 {
                    return fresh[i];
                }
                let j = y as usize * SEA_GRID + x as usize;
                let g = self.grid[j];
                let old = self.prev.get(j).copied().unwrap_or(g);
                std::array::from_fn(|k| old[k] + (g[k] - old[k]) * blend)
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Contact {
    position: Vec3,
    time: f32,
    ripple_at: f32,
    splash_at: f32,
}

impl Contact {
    fn new(position: Vec3, time: f32) -> Self {
        Self {
            position,
            time,
            ripple_at: time,
            splash_at: time - 0.5,
        }
    }
}

fn water_at(world: &World, p: Vec3) -> Option<f32> {
    world.regions.values().find_map(|r| {
        let off = world.region_offset(r.handle)?;
        let size = Vec2::new(r.heightmap.size_x as f32, r.heightmap.size_y as f32);
        let xy = p.truncate() - off.truncate();
        (xy.cmpge(Vec2::ZERO).all() && xy.cmplt(size).all())
            .then(|| r.info.as_ref().map(|i| i.water_height))
            .flatten()
    })
}

/// The prims near the water ground (`origin`, `extent`) that may shelter
/// it: what stands still (`Scene::moves_for_shelter`, as for the rain's
/// shelter map: a boat rides the sea, it must not calm it), neither an
/// attachment nor a HUD.
fn shelter_boxes(scene: &Scene, world: &World, origin: Vec2, extent: f32) -> Vec<(usize, super::water_shelter::ShelterBox)> {
    let now = Instant::now();
    let (lo, hi) = (origin - Vec2::splat(8.0), origin + Vec2::splat(extent + 8.0));
    // (through frame_par: a plain rayon loop held the frame for a pool
    // worker on a busy region, #4121)
    crate::frame_par::filter_map_indexed(&scene.gpu, SCAN_GRAIN, |i, g| {
        // (whether its shape is loaded or not: turning the camera loads and
        // drops shapes, and the grid was baked again each time)
        if g.hud || g.is_avatar || g.owner_avatar.is_some() {
            return None;
        }
        let c = g.center.truncate();
        if c.cmplt(lo).any() || c.cmpgt(hi).any() {
            return None;
        }
        let o = world.objects.get(i)?;
        // what moves for the rain's shelter moves for the sea's: physical,
        // drawn moving, or stepped by a script a while ago (shelter_hold.rs)
        if o.is_attachment() || o.extra.reflection_probe.is_some() || Scene::moves_for_shelter(world, &scene.gpu, i, now) {
            return None;
        }
        // A prim shelters as its box: a mesh over 64 m (a sim surround,
        // a ring round the whole region) or a prim over 128 m in two
        // directions (a megaprim plate) is nothing like its box, which
        // would calm all the water it spans.
        let size = o.scale.abs();
        let mut sides = size.to_array();
        sides.sort_by(f32::total_cmp);
        if ((o.volume.is_mesh() || o.volume.is_sculpt()) && sides[2] > 64.0) || sides[1] > 128.0 {
            return None;
        }
        let (p, rotation, _) = Scene::object_transform(world, i, now, 0)?;
        // only what reaches the water: from a few metres under it to the
        // highest underside that still holds the sea, and not standing on
        // land (a house on the shore, a tree, a skybox shelter no water,
        // and their least change baked the whole grid again)
        let half = o.scale.abs() * 0.5;
        let reach = (0..3).map(|k| (rotation * Vec3::AXES[k]).z.abs() * half[k]).sum::<f32>();
        let level = water_at(world, p).unwrap_or(world.main_water_height());
        if p.z - reach > level + aurora_render::water_effects::OPEN_CLEARANCE + 0.5 || p.z + reach < level - 3.0 {
            return None;
        }
        if world.ground_height(p).is_some_and(|g| g > level + 2.0) {
            return None;
        }
        let mesh = o.volume.is_mesh() || o.volume.is_sculpt();
        Some((
            i,
            super::water_shelter::ShelterBox {
                center: p,
                rotation,
                half,
                mesh,
            },
        ))
    })
}

/// How long a prim seen moving is left out of the shelter (s).
const MOVER_HOLD: f32 = 120.0;
/// The sea grid is centred on our avatar while the camera is within this
/// of it (m), on the camera otherwise.
const GRID_ON_AVATAR: f32 = 160.0;

/// A shelter prim's place and size as the sea grid's signature counts them
/// (to 5 cm and about a degree).
fn shelter_key(b: &super::water_shelter::ShelterBox) -> [i32; 10] {
    let mut key = [0; 10];
    for (k, v) in b.center.to_array().into_iter().chain(b.half.to_array()).enumerate() {
        key[k] = (v * 20.0).round() as i32;
    }
    for (k, v) in b.rotation.to_array().into_iter().take(4).enumerate() {
        key[6 + k] = (v * 60.0).round() as i32;
    }
    key
}

/// The objects near the water are looked for again every 0.35 s, whether
/// the last scan found some or none.
fn scan_due(last: Option<f32>, time: f32) -> bool {
    last.is_none_or(|last| time - last > 0.35)
}

/// The ground grid's 64 x 64 samples at their cells' centres, row by row.
fn ground_grid(origin: Vec2, extent: f32, sample: impl Fn(Vec2) -> [f32; 2] + Sync) -> Vec<[f32; 2]> {
    let rows: [usize; 64] = std::array::from_fn(|row| row);
    crate::frame_par::fold(&rows, GROUND_GRAIN, Vec::new, |mut heights, &row| {
        heights.extend((0..64).map(|column| sample(origin + (Vec2::new(column as f32, row as f32) + Vec2::splat(0.5)) * extent / 64.0)));
        heights
    })
    .into_iter()
    .flatten()
    .collect()
}

impl Effects {
    /// Show `grid`, baked for `origin` from what `signature` sums up,
    /// fading in from what the grid before it shows now.
    fn install_sea_grid(&mut self, origin: Vec2, signature: u64, grid: Vec<[f32; 6]>, time: f32) {
        let grid: std::sync::Arc<[[f32; 6]]> = grid.into();
        log::info!(
            "water sea grid: a new grid at ({:.0}, {:.0}), {}",
            origin.x,
            origin.y,
            match &self.sea_grid {
                None => "the first",
                Some(old) if old.origin != origin => "the camera moved 64 m",
                Some(_) => "baked again",
            }
        );
        let prev: std::sync::Arc<[[f32; 6]]> = match &self.sea_grid {
            Some(old) => old.shown(time, origin, &grid).into(),
            None => grid.iter().copied().collect::<Vec<_>>().into(),
        };
        self.sea_grid = Some(SeaGrid {
            origin,
            checked_at: time,
            signature,
            grid,
            prev,
            baked_at: time,
        });
    }

    fn retain_events(&mut self, time: f32, quality: u8, eye: Vec2) {
        self.events.retain(|event| {
            let lifetime = match event.params[1] {
                3.0 => 0.3,       // The GPU field has already integrated the impulse.
                4.0 | 8.0 => 2.0, // Swept surface segments spread and settle in world space.
                5.0 | 6.0 => 1.6, // Sheet flight and its expanding foam rim.
                9.0 => 4.0,       // A crash's spray falls back; its mist hangs on.
                10.0 => 4.5,      // The foam it leaves spreads and fades.
                11.0 => 2.0,      // Drops dripping off an avatar fall a metre or two.
                // Spindrift blown off a crest thins out within some 1.5 s,
                // the crests' own spray falls back within some 1.2 s. (Kept
                // 3 s, the Spray slider's spray outgrew the particle budget,
                // and splashes were cut off mid-flight: water_spindrift.rs
                // `room`.)
                12.0 if event.params[0] >= 0.9 => 1.3,
                12.0 => 1.6,
                0.0 | 7.0 => 0.8,
                _ => 3.0,
            };
            // (a crash's spray may be due a moment from now: water_crash.rs
            // staggers its bursts along the face)
            (-1.0..lifetime).contains(&(time - event.position[3]))
        });
        let max = [64, 96, 160, 192][quality.min(3) as usize];
        if self.events.len() > max {
            // Keep an entry's complete silhouette through its short lifetime,
            // even when many ordinary contacts fill the Low-quality history.
            self.events.sort_unstable_by(|a, b| {
                let priority = |e: &WaterEvent| {
                    if matches!(e.params[1], 5.0 | 6.0 | 9.0 | 10.0) {
                        2
                    } else if (Vec2::new(e.position[0], e.position[1]) - eye).abs().max_element() <= 18.0 {
                        1
                    } else {
                        0
                    }
                };
                priority(a).cmp(&priority(b)).then_with(|| a.position[3].total_cmp(&b.position[3]))
            });
            self.events.drain(..self.events.len() - max);
        }
    }

    fn emit_impact(&mut self, impact: crate::movement::water_jump::Impact, time: f32, settings: WaterEffects) {
        use crate::movement::water_jump::Kind;
        if !impact.position.is_finite() || !impact.speed.is_finite() {
            return;
        }
        let big = impact.kind == Kind::Cannonball;
        let strength = (impact.speed * if big { 0.28 } else { 0.18 }).clamp(if big { 1.4 } else { 0.7 }, if big { 3.5 } else { 2.2 });
        if settings.interactions {
            self.events.push(WaterEvent {
                position: impact.position.extend(time).to_array(),
                motion: [0.0, 0.0, -impact.speed, strength],
                params: [if big { 0.85 } else { 0.38 }, 6.0, impact.position.z, 0.0],
            });
        }
        if settings.bubbles {
            self.events.push(WaterEvent {
                position: (impact.position - Vec3::Z * 0.22).extend(time).to_array(),
                motion: [0.0, 0.0, 0.0, 0.18],
                params: [0.06, 1.0, impact.position.z, 0.0],
            });
        }
        if settings.splashes {
            let packets = if big { 10 } else { 6 };
            let arc = std::f32::consts::TAU / packets as f32;
            for i in 0..packets {
                let angle = i as f32 * arc;
                let direction = Vec3::new(angle.cos(), angle.sin(), 0.0);
                self.events.push(WaterEvent {
                    position: impact.position.extend(time).to_array(),
                    motion: direction.extend(strength).to_array(),
                    params: [if big { 0.46 } else { 0.22 }, 5.0, impact.position.z, arc],
                });
            }
        }
    }

    fn emit(&mut self, id: (Uuid, u8), pos: Vec3, radius: f32, water: f32, time: f32, settings: WaterEffects) {
        if !pos.is_finite() || !water.is_finite() || !time.is_finite() || !radius.is_finite() {
            return;
        }
        let previous = self.history.get(&id).copied();
        self.history.insert(
            id,
            Contact {
                position: pos,
                time,
                ..previous.unwrap_or_else(|| Contact::new(pos, time))
            },
        );
        let Some(previous) = previous else {
            return;
        };
        let dt = time - previous.time;
        let delta = pos - previous.position;
        if !(0.001..0.5).contains(&dt) || delta.length() > 8.0 {
            self.history.insert(id, Contact::new(pos, time));
            return;
        }
        let mut velocity = delta / dt;
        if velocity.length() > 40.0 {
            velocity = velocity.normalize() * 40.0;
        }
        let speed = velocity.length();
        let surface = Vec3::new(pos.x, pos.y, water);
        let state = self.history.get_mut(&id).expect("contact inserted above");
        if settings.interactions && speed > 0.2 && time - state.ripple_at >= 0.18 {
            self.events.push(WaterEvent {
                position: surface.extend(time).to_array(),
                motion: velocity.extend((speed * 0.085).clamp(0.02, 0.85)).to_array(),
                params: [radius.clamp(0.15, 3.0), 3.0, water, 0.0],
            });
            state.ripple_at = time;
        }
        if settings.splashes && (pos.z - water).abs() <= radius.max(0.5) && velocity.z < -0.6 && time - state.splash_at >= 0.38 {
            self.events.push(WaterEvent {
                position: surface.extend(time).to_array(),
                motion: velocity.extend((velocity.z.abs() * 0.17).clamp(0.12, 1.2)).to_array(),
                params: [radius.clamp(0.15, 3.0), 0.0, water, 0.0],
            });
            state.splash_at = time;
        }
    }

    fn update(
        &mut self,
        scene: &Scene,
        world: &World,
        eye: Vec3,
        time: f32,
        settings: WaterEffects,
    ) -> (Vec<WaterEvent>, Vec<WaterEvent>, WaterGround) {
        if self.region != world.main_region || settings.flags() == 0 {
            *self = Self {
                region: world.main_region,
                // the sea goes on across regions (advanced just before)
                sea: std::mem::take(&mut self.sea),
                ..Default::default()
            };
        }
        let sea = self.sea.sea();
        if settings.flags() == 0 {
            return Default::default();
        }
        self.geometry = (settings.waves || settings.interactions).then_some(settings.quality);
        let range = settings.range();
        let extent = range * 2.0;
        // Integer multiples of the simulation cell: scrolling preserves wave phase.
        let cell = extent / settings.resolution().max(128) as f32;
        let anchor = (eye.truncate() / (cell * 16.0)).floor() * (cell * 16.0);
        let origin = anchor - Vec2::splat(range);
        if self.ground.extent != extent || self.ground.origin != origin.to_array() || time - self.ground_at > 0.5 {
            self.ground_at = time;
            let heights = ground_grid(origin, extent, |xy| {
                let p = xy.extend(0.0);
                let w = water_at(world, p).unwrap_or(world.main_water_height());
                [world.ground_height(p).unwrap_or(w - 100.0), w]
            });
            // the sea grid (water_shelter.rs): 512 m around the camera,
            // following it in 64 m steps, baked again with the ground; the
            // shelter's lee is downwind of a sea with some wind
            // (baked again when it moves, or every 2 s for what stands in
            // the water: 20,000 heights and the shelter's prims)
            let (sea_grid, sea_prev, sea_origin) = if settings.waves {
                let step = 64.0;
                // centred on our avatar, not on the camera: orbiting or
                // zooming the camera moved it, a new grid each time (the
                // camera's when it is far from the avatar)
                let eye = world
                    .objects
                    .index_of_uuid(&world.agent_id)
                    .and_then(|i| Scene::object_transform(world, i, Instant::now(), 0))
                    .map(|(p, _, _)| p)
                    .filter(|p| p.is_finite() && (p.truncate() - eye.truncate()).length() < GRID_ON_AVATAR)
                    .unwrap_or(eye);
                // (it stays where it is until the camera is well past the
                // next step: a camera orbiting across the line moved it back
                // and forth, baking and fading in a grid each time)
                let ideal = (eye.truncate() / step).floor() * step + Vec2::splat(step * 0.5 - SEA_EXTENT * 0.5);
                let sea_origin = self
                    .sea_grid
                    .as_ref()
                    .map(|g| g.origin)
                    .filter(|o| (*o + Vec2::splat(SEA_EXTENT * 0.5) - eye.truncate()).abs().max_element() < step * 0.5 + GRID_HOLD)
                    .unwrap_or(ideal);
                let due = self
                    .sea_grid
                    .as_ref()
                    .is_none_or(|g| g.origin != sea_origin || !(0.0..2.0).contains(&(time - g.checked_at)));
                if due {
                    // what the grid is made of: the shelter's prims, the lee's
                    // direction (by 15°) and a coarse look at the ground. The
                    // grid is baked again only when one of them changed, as a
                    // new grid costs the frame that takes it.
                    let started = Instant::now();
                    let found = shelter_boxes(scene, world, sea_origin, SEA_EXTENT);
                    // what moved since the last scan, or dropped out of it
                    // while still near the water (left out for moving), is
                    // held out for a while; what changed is logged
                    let mut now_seen = HashMap::with_capacity(found.len());
                    for (i, b) in &found {
                        if let Some(o) = world.objects.get(*i) {
                            now_seen.insert(o.full_id, (o.key.local_id, shelter_key(b)));
                        }
                    }
                    let mut moved = Vec::new();
                    for (id, (local, key)) in &self.shelter_seen {
                        match now_seen.get(id) {
                            Some((_, k)) if k != key => {
                                self.shelter_movers.insert(*id, time);
                                self.shelter_pinned.remove(id);
                                moved.push(format!("{local} moved"));
                            }
                            None if world.objects.index_of_uuid(id).is_some_and(|i| {
                                scene.gpu.get(i).is_some_and(|g| {
                                    let c = g.center.truncate();
                                    c.cmpge(sea_origin).all() && c.cmplt(sea_origin + Vec2::splat(SEA_EXTENT)).all()
                                })
                            }) =>
                            {
                                // dropped out unmoved: kept where it was
                                if let Some(&(i, b)) = self.shelter_boxes.get(id) {
                                    self.shelter_pinned.insert(*id, (i, b, time));
                                }
                                moved.push(format!("{local} kept"));
                            }
                            _ => {}
                        }
                    }
                    self.shelter_movers.retain(|_, at| (0.0..MOVER_HOLD).contains(&(time - *at)));
                    // back in the scan, gone from the region, out of the grid
                    // or kept long enough: no longer pinned
                    self.shelter_pinned.retain(|id, (_, b, at)| {
                        let c = b.center.truncate();
                        !now_seen.contains_key(id)
                            && world.objects.index_of_uuid(id).is_some()
                            && c.cmpge(sea_origin).all()
                            && c.cmplt(sea_origin + Vec2::splat(SEA_EXTENT)).all()
                            && (0.0..MOVER_HOLD).contains(&(time - *at))
                    });
                    let held: Vec<String> = now_seen
                        .iter()
                        .filter(|(id, _)| self.shelter_movers.contains_key(*id) && !self.shelter_seen.contains_key(*id))
                        .map(|(_, (local, _))| format!("{local} back"))
                        .collect();
                    if !moved.is_empty() || !held.is_empty() {
                        log::info!(
                            "water sea grid: shelter prims changing (moved: held out {MOVER_HOLD:.0} s; unmoved: kept): {}",
                            moved.iter().chain(held.iter()).take(12).cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                    self.shelter_seen = now_seen;
                    self.shelter_boxes = found
                        .iter()
                        .filter_map(|(i, b)| world.objects.get(*i).map(|o| (o.full_id, (*i, *b))))
                        .collect();
                    let movers = &self.shelter_movers;
                    // (the pinned ones in a fixed order: the grid's signature
                    // hashes the boxes in order)
                    let mut pinned: Vec<_> = self.shelter_pinned.iter().map(|(id, (i, b, _))| (*id, *i, *b)).collect();
                    pinned.sort_by_key(|p| p.0);
                    let (objects, boxes): (Vec<usize>, Vec<_>) = found
                        .into_iter()
                        .filter(|(i, _)| world.objects.get(*i).is_none_or(|o| !movers.contains_key(&o.full_id)))
                        .chain(pinned.into_iter().map(|(_, i, b)| (i, b)))
                        .unzip();
                    self.crash_boxes.clone_from(&boxes);
                    self.crash_objects = objects;
                    // (the lee is where the incoming swell is blocked: its way, not
                    // the wind's, which turns quicker)
                    let toward = Vec2::new(sea.trains[0][0], sea.trains[0][1]) * if sea.wind[2] > 2.0 { 1.0 } else { 0.0 };
                    let swell = super::water_shelter::Swell {
                        toward: Vec2::new(sea.trains[0][0], sea.trains[0][1]),
                        k: sea.trains[0][2],
                    };
                    let main = world.main_water_height();
                    let ground_water = |xy: Vec2| {
                        let p = xy.extend(0.0);
                        let w = water_at(world, p).unwrap_or(main);
                        [world.ground_height(p).unwrap_or(w - 100.0), w]
                    };
                    // its parts: the prims, the lee, the swell (its way by 7.5°,
                    // its length by 25 %) and a coarse look at the ground
                    let parts: [u64; 4] = {
                        use std::hash::{Hash, Hasher};
                        let hash = |f: &dyn Fn(&mut std::collections::hash_map::DefaultHasher)| {
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            f(&mut h);
                            h.finish()
                        };
                        [
                            hash(&|h| {
                                // (to 5 cm and about a degree: a prim's
                                // jitter on the wire is no change)
                                for b in &boxes {
                                    for v in b.center.to_array().into_iter().chain(b.half.to_array()) {
                                        ((v * 20.0).round() as i32).hash(h);
                                    }
                                    for v in b.rotation.to_array() {
                                        ((v * 60.0).round() as i32).hash(h);
                                    }
                                }
                            }),
                            hash(&|h| ((toward.to_angle().to_degrees() / 15.0).round() as i32 * i32::from(toward != Vec2::ZERO)).hash(h)),
                            hash(&|h| {
                                ((swell.toward.to_angle().to_degrees() / 7.5).round() as i32).hash(h);
                                ((swell.k.max(1e-4).ln() / 1.25f32.ln()).round() as i32).hash(h);
                            }),
                            hash(&|h| {
                                for i in 0..64 {
                                    let xy =
                                        sea_origin + (Vec2::new((i % 8) as f32, (i / 8) as f32) + Vec2::splat(0.5)) * (SEA_EXTENT / 8.0);
                                    // (to 25 cm: the ground streaming in is no change)
                                    ground_water(xy).map(|v| (v * 4.0).round() as i32).hash(h);
                                }
                            }),
                        ]
                    };
                    let signature = {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        parts.hash(&mut h);
                        h.finish()
                    };
                    let same = self
                        .sea_grid
                        .as_ref()
                        .is_some_and(|g| g.origin == sea_origin && g.signature == signature);
                    let pending = self
                        .sea_baking
                        .as_ref()
                        .is_some_and(|b| b.origin == sea_origin && b.signature == signature);
                    if same {
                        if let Some(g) = &mut self.sea_grid {
                            g.checked_at = time;
                        }
                    } else if !pending {
                        if self.sea_grid.as_ref().is_some_and(|g| g.origin == sea_origin) {
                            let names = ["what stands in the water", "the lee", "the swell", "the ground"];
                            let changed: Vec<&str> = (0..4).filter(|&i| parts[i] != self.sea_parts[i]).map(|i| names[i]).collect();
                            log::info!("water sea grid: baking again, changed: {}", changed.join(", "));
                        }
                        self.sea_parts = parts;
                        let heights = super::water_shelter::sample_ground(sea_origin, SEA_EXTENT, ground_water);
                        if self.sea_grid.is_none() {
                            // the first grid at once: the sea never starts out
                            // flat and deep while it bakes
                            let grid = super::water_shelter::bake_grid(sea_origin, SEA_EXTENT, &heights, &boxes, toward, swell);
                            self.install_sea_grid(sea_origin, signature, grid, time);
                        } else {
                            // the next ones on a worker: the marching takes a
                            // few milliseconds; the old grid shows meanwhile
                            let result = std::thread::spawn(move || {
                                super::water_shelter::bake_grid(sea_origin, SEA_EXTENT, &heights, &boxes, toward, swell)
                            });
                            self.sea_baking = Some(SeaBaking {
                                origin: sea_origin,
                                signature,
                                result,
                            });
                        }
                        log::debug!("water sea grid: sampled in {:.1} ms", started.elapsed().as_secs_f32() * 1000.0);
                    }
                }
                // a grid baked on the worker, taken once ready (one baked for
                // where the grid no longer is: dropped)
                if self.sea_baking.as_ref().is_some_and(|b| b.result.is_finished())
                    && let Some(baking) = self.sea_baking.take()
                {
                    match baking.result.join() {
                        Ok(grid) if baking.origin == sea_origin => {
                            self.install_sea_grid(baking.origin, baking.signature, grid, time);
                        }
                        Ok(_) => {}
                        Err(_) => log::warn!("water sea grid: the bake failed"),
                    }
                }
                // (an empty previous grid stays the same buffer: re-sent to
                // the GPU only when it changes)
                let (grid, prev) = self.sea_grid.as_ref().map_or_else(
                    || (std::sync::Arc::from(Vec::new()), std::sync::Arc::from(Vec::new())),
                    |g| (g.grid.clone(), g.prev.clone()),
                );
                (grid, prev, sea_origin)
            } else {
                (std::sync::Arc::from(Vec::new()), std::sync::Arc::from(Vec::new()), Vec2::ZERO)
            };
            self.ground = WaterGround {
                region: world.main_region.unwrap_or(0),
                origin: origin.to_array(),
                extent,
                heights: heights.into(),
                sea: sea_grid,
                sea_prev,
                sea_origin: sea_origin.to_array(),
                sea_extent: SEA_EXTENT,
                ocean: self.sea.maps(),
            };
        }
        if settings.flags() & (4 | 16 | 64 | 128) == 0 {
            self.events.clear();
            self.impacts.clear();
            self.history.clear();
            return (Vec::new(), Vec::new(), self.ground.clone());
        }
        if scan_due(self.scan_at, time) {
            self.scan_at = Some(time);
            let mut near: Vec<(f32, usize)> = crate::frame_par::filter_map_indexed(&scene.gpu, SCAN_GRAIN, |i, g| {
                let object = world.objects.get(i)?;
                if g.hud || g.owner_avatar.is_some() || object.extra.reflection_probe.is_some() {
                    return None;
                }
                let distance = g.center.truncate().distance_squared(eye.truncate());
                if distance >= range * range || (!g.is_avatar && g.faces.is_empty()) {
                    return None;
                }
                let height = object.scale.z.abs().max(if g.is_avatar { 2.0 } else { 0.0 });
                let w = water_at(world, g.center)?;
                let nearby_avatar = g.is_avatar && g.center.distance_squared(eye) < range * range;
                (nearby_avatar || (!g.is_avatar && (g.center.z - w).abs() < height * 0.5 + 8.0)).then_some((distance, i))
            });
            near.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            self.candidates = near.into_iter().take(96).map(|(_, i)| i).collect();
            self.visible = water_contacts::visibility(scene, world, &self.candidates);
        }
        let now = Instant::now();
        let mut contacts = Vec::new();
        let body_samples = crate::frame_par::fold(
            &self.candidates,
            Grain { chunk: 4, per_helper: 1 },
            Vec::new,
            |mut samples, &idx| {
                let Some(o) = world.objects.get(idx).filter(|o| o.is_avatar()) else {
                    return samples;
                };
                if scene.too_complex.contains(&idx)
                    || world.derender.contains(&o.full_id)
                    || world.particles_blocked(&o.full_id)
                    || !world.parcel_object_visible(idx)
                    || !crate::snapshot::isolate::object_visible(world, idx)
                {
                    return samples;
                }
                let Some((p, r, _)) = Scene::object_transform(world, idx, now, 0) else {
                    return samples;
                };
                let Some(water) = water_at(world, p) else {
                    return samples;
                };
                let Some(slot) = scene.palette_slots.get(&o.full_id) else {
                    return samples;
                };
                let base = *slot as usize * anim::PALETTE_JOINTS;
                let model = anchors::skeleton_model(world, &scene.meshes, idx, p, r, 1.0);
                let visible = self.visible.get(&o.full_id).copied().unwrap_or(0);
                for (part, name) in water_contacts::PARTS.iter().enumerate() {
                    if visible & (1 << part) == 0 {
                        continue;
                    }
                    let Some(j) = scene.avatar_lib.rig.names.get(*name) else {
                        continue;
                    };
                    let Some(joint) = scene.palettes.get(base + j) else {
                        continue;
                    };
                    let matrix = model * Mat4::from_cols_array_2d(joint);
                    let center = matrix.w_axis.truncate();
                    // Distant avatars retain sparse head air; fine contact work is
                    // confined to the camera's local water patch.
                    if part != water_contacts::PARTS.len() - 1 && (center.truncate() - eye.truncate()).abs().max_element() > 18.0 {
                        continue;
                    }
                    let water = water_at(world, center).unwrap_or(water);
                    let ground = self.ground.sample(center.truncate()).map(|p| p[0]).unwrap_or(water - 20.0);
                    let at = self.ground.sea_point(center.truncate(), &sea, water - ground);
                    let surface = aurora_render::water_effects::surface_level(center.truncate(), time, settings, &sea, water, &at);
                    samples.push(water_contacts::Sample {
                        id: o.full_id,
                        part,
                        matrix,
                        level: water,
                        surface,
                        ground,
                        carrier_z: world.water_visual_dz(&o.full_id),
                    });
                }
                samples
            },
        );
        for sample in body_samples.into_iter().flatten() {
            let before = self.events.len();
            let mut part_settings = settings;
            if (sample.matrix.w_axis.truncate().truncate() - eye.truncate()).abs().max_element() > 18.0 {
                part_settings.interactions = false;
                part_settings.splashes = false;
            }
            self.bodies.sample(sample, time, part_settings, &mut self.events);
            if self.events.len() > before {
                log::trace!(target: "aurora_viewer::water_contacts", "water part {}: center {:?}, surface {:.3}, events {:?}", water_contacts::PARTS[sample.part], sample.matrix.w_axis.truncate(), sample.surface, self.events[before..].iter().map(|e| (e.params[1], e.params[3], e.motion[3])).collect::<Vec<_>>());
            }
        }

        for &idx in &self.candidates.clone() {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let owner = if o.is_avatar() { o.full_id } else { o.owner_id };
            if scene.too_complex.contains(&idx)
                || world.derender.contains(&o.full_id)
                || world.particles_blocked(&owner)
                || !world.parcel_object_visible(idx)
                || !crate::snapshot::isolate::object_visible(world, idx)
            {
                continue;
            }
            let Some((p, r, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let Some(water) = water_at(world, p) else {
                continue;
            };
            if !o.is_avatar() {
                // Root and linked pieces participate; attachments are excluded above.
                let half = o.scale.abs() * 0.5;
                let z_extent = (r * Vec3::X).z.abs() * half.x + (r * Vec3::Y).z.abs() * half.y + (r * Vec3::Z).z.abs() * half.z;
                if p.z - z_extent > water + 0.2 || p.z + z_extent < water - 0.2 {
                    // Keep the approach velocity for the first visible impact.
                    self.history.insert((o.full_id, 10), Contact::new(p, time - 0.02));
                    continue;
                }
                let emitted = self.events.len();
                self.emit((o.full_id, 10), p, half.truncate().min_element().max(0.2), water, time, settings);
                for event in &mut self.events[emitted..] {
                    event.params[3] = half.truncate().max_element().clamp(0.3, 12.0);
                }
                if settings.contact_foam && contacts.len() < 64 {
                    let axis = (r * Vec3::X).truncate().normalize_or(Vec2::X);
                    contacts.push(WaterEvent {
                        position: p.extend(time).to_array(),
                        motion: [axis.x, axis.y, 0.0, 0.0],
                        params: [half.x.clamp(0.1, 20.0), 2.0, water, half.y.clamp(0.1, 20.0)],
                    });
                }
            }
        }
        for impact in std::mem::take(&mut self.impacts) {
            self.emit_impact(impact, time, settings);
        }
        self.history.retain(|_, contact| time - contact.time < 1.0);
        self.bodies.retain(time);
        self.retain_events(time, settings.quality, eye.truncate());
        (self.events.clone(), contacts, self.ground.clone())
    }
}

impl Scene {
    pub fn avatar_water_contact(&self, world: &World) -> Option<Vec3> {
        let idx = world.objects.index_of_uuid(&world.agent_id)?;
        let (p, r, _) = Self::object_transform(world, idx, Instant::now(), 0)?;
        let base = *self.palette_slots.get(&world.agent_id)? as usize * anim::PALETTE_JOINTS;
        let model = anchors::skeleton_model(world, &self.meshes, idx, p, r, 1.0);
        ["mHead", "mFootLeft", "mFootRight", "mWristLeft", "mWristRight", "mPelvis"]
            .into_iter()
            .filter_map(|name| {
                let j = self.avatar_lib.rig.names.get(name)?;
                let matrix = self.palettes.get(base + j)?;
                let contact = model.transform_point3(Mat4::from_cols_array_2d(matrix).w_axis.truncate()) - Vec3::Z * 0.08;
                contact.is_finite().then_some(contact)
            })
            .min_by(|a, b| a.z.total_cmp(&b.z))
    }

    pub fn water_impact(&mut self, impact: crate::movement::water_jump::Impact) {
        if self.water_effects.impacts.len() < 8 {
            self.water_effects.impacts.push(impact);
        }
    }

    /// `wind`: the wind over the water (m/s, blowing toward), which raises
    /// the sea (`App::sea_wind`); `tune`: how the Sea panel shapes it.
    pub fn update_water_effects(
        &mut self,
        world: &World,
        eye: Vec3,
        time: f32,
        settings: WaterEffects,
        wind: glam::Vec2,
        tune: aurora_render::water_sea::SeaTune,
    ) {
        let mut effects = std::mem::take(&mut self.water_effects);
        // the wind sea as a spectrum (aurora_render::water_fft) while the
        // waves show and the Sea panel draws it so; its FFTs cost nothing
        // otherwise
        let waves = settings.enabled && settings.waves;
        effects.sea.set_tune(aurora_render::water_sea::SeaTune {
            spectrum: tune.spectrum.filter(|_| waves),
            ..tune
        });
        self.lists.water_sea = effects.sea.advance(time, wind, eye.truncate());
        self.lists.water_spray = if waves && settings.splashes { tune.spray } else { 0.0 };
        // (the CPU's samplers read the maps of this frame, as the GPU)
        effects.ground.ocean = effects.sea.maps();
        // waves crashing on walls and rocks throw up spray (water_crash.rs)
        if waves && settings.splashes {
            let sea = &self.lists.water_sea;
            let toward = glam::Vec2::new(sea.trains[0][0], sea.trains[0][1]);
            // (where a prim's box meets the water, its real surface: a
            // rock's box stands well off the rock)
            let objects = &effects.crash_objects;
            let surface = |k: usize, origin: Vec3, dir: Vec3| {
                objects.get(k).map(|&i| {
                    self.geometry_hit(world, i, (origin, dir), Instant::now())
                        .filter(|h| h.t < 8.0 && h.normal.is_finite())
                        .map(|h| (h.t, h.normal.normalize_or_zero()))
                })
            };
            effects
                .crashes
                .scan(&effects.ground, &effects.crash_boxes, &surface, eye.truncate(), toward, time);
            // (half the events kept at this quality, retain_events)
            let budget = [64, 96, 160, 192][settings.quality.min(3) as usize] / 2;
            effects.crashes.update(&effects.ground, sea, time, &mut effects.events, budget);
            // and a strong wind tears spray off the breaking crests
            effects.spindrift.update(
                &effects.ground,
                sea,
                eye,
                time,
                tune.spray,
                spray_room(settings),
                &mut effects.events,
            );
        } else {
            effects.crashes.clear();
        }
        // water drips off the wet avatars (avatar_drip.rs)
        if settings.enabled && settings.wet_avatars && settings.splashes {
            let avatars: Vec<uuid::Uuid> = world.objects.avatars().map(|(_, o)| o.full_id).collect();
            let slots = &self.palette_slots;
            effects.drip.update(
                &self.avatar_wet,
                &self.palettes,
                super::anim::PALETTE_JOINTS,
                |id| slots.get(id).copied(),
                &self.avatar_lib.rig.names,
                &avatars,
                &effects.ground,
                eye,
                time,
                &mut effects.events,
            );
        } else {
            effects.drip.clear();
        }
        // how far the sea grid's crossfade has gone (read alike by the
        // shaders and the CPU's samplers)
        self.lists.water_sea.shore[0] = effects.sea_grid.as_ref().map_or(1.0, |g| g.blend(time));
        // the sea's state now and then, for grid reports (no sea panel there)
        if settings.enabled && settings.waves && (effects.sea_logged_at == 0.0 || !(0.0..10.0).contains(&(time - effects.sea_logged_at))) {
            effects.sea_logged_at = time;
            let s = &self.lists.water_sea;
            log::info!(
                "water sea: wind {:.1} m/s (input {:.1}) toward ({:.2}, {:.2}), roughness {:.2}, swell {:.0} m long {:.2} m high toward ({:.2}, {:.2})",
                s.wind[2],
                wind.length(),
                s.wind[0],
                s.wind[1],
                s.frame[3],
                std::f32::consts::TAU / s.trains[0][2].max(1e-3),
                2.0 * s.trains[0][3],
                s.trains[0][0],
                s.trains[0][1]
            );
        }
        // and the sea where the camera is, the same moment: how deep, where
        // the swell comes from and how much of it reaches here, the shelter
        if effects.sea_logged_at == time && settings.enabled && settings.waves {
            let s = &self.lists.water_sea;
            let here = eye.truncate();
            if let Some([g, w]) = effects.ground.sample(here) {
                let at = effects.ground.sea_point(here, s, w - g);
                let (shadow, chop) = at.swell.map_or((1.0, 1.0), |p| (p.shadow, p.chop));
                let local = s.local(&at, shadow, chop);
                // (compass degrees clockwise from north, the way it comes from)
                let from = (-s.trains[0][0]).atan2(-s.trains[0][1]).to_degrees().rem_euclid(360.0);
                log::info!(
                    "water sea here: depth {:.1} m (sea grid {:.1} m), swell from {:.0} deg reaching {:.0} %, shelter {:.2}, swell {:.2} m high, chop share {:.2}",
                    w - g,
                    at.depth,
                    from,
                    shadow * 100.0,
                    at.shelter,
                    2.0 * local.main,
                    local.chop
                );
            }
            // and from here the way the swell goes, toward the shore, every
            // 5 m: depth (real, sea grid), slope, shelter, cap, and the
            // swell, swash and breaking the sea draws there
            let toward = glam::Vec2::new(s.trains[0][0], s.trains[0][1]);
            let mut row = String::new();
            for step in 0..20 {
                let q = here + toward * (step as f32 * 5.0);
                let Some([g, w]) = effects.ground.sample(q) else {
                    break;
                };
                let at = effects.ground.sea_point(q, s, w - g);
                let (shadow, chop) = at.swell.map_or((1.0, 1.0), |p| (p.shadow, p.chop));
                let l = s.local(&at, shadow, chop);
                row.push_str(&format!(
                    " | {}m d{:.1}/{:.1} sl{:.2} sh{:.2} cap{:.2} rch{:.0}% sw{:.2} swash{:.2} brk{:.2}",
                    step * 5,
                    w - g,
                    at.depth,
                    at.slope.length(),
                    at.shelter,
                    at.cap,
                    shadow * 100.0,
                    2.0 * l.main,
                    l.swash,
                    l.breaking
                ));
                if w - g < -1.5 {
                    break;
                }
            }
            log::info!("water sea toward the shore:{row}");
        }
        let (events, contacts, ground) = effects.update(self, world, eye, time, settings);
        self.water_effects = effects;
        self.lists.water_events = events;
        self.lists.water_contacts = contacts;
        self.lists.water_ground = ground;
        // no ripples nor splashes on a hidden water (render_types.rs)
        if self.hidden_types.water {
            self.lists.water_events.clear();
            self.lists.water_contacts.clear();
        }
    }
}

/// How many spray events (water_spindrift.rs) may be alive at once at this
/// quality: within the water particles' own share of the particle limit
/// (`WATER_PARTICLE_FLOOR` quads, each event `particle_count` of them) and
/// half the event history `retain_events` keeps, so that none is dropped
/// mid-flight however high the Spray slider goes, leaving room for the
/// splashes of what moves in the water.
fn spray_room(settings: WaterEffects) -> usize {
    let per = settings.particle_count().max(1);
    let drawn = (aurora_render::water_effects::WATER_PARTICLE_FLOOR / per) as usize * 3 / 4;
    let kept = [64, 96, 160, 192][settings.quality.min(3) as usize] / 2;
    drawn.min(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prim entering the shelter calms its patch of sea over the fade,
    /// never at once (the depth and the shelter faded with the rest).
    #[test]
    fn a_new_grid_fades_in_all_of_a_cell() {
        use aurora_render::water_effects::SEA_GRID;
        let open = [5.0, 1.0, 3.0, 0.4, 0.9, 0.8];
        let calm = [4.0, 0.2, 1.0, 0.6, 0.5, 0.3];
        let old = SeaGrid {
            origin: Vec2::ZERO,
            checked_at: 0.0,
            signature: 1,
            grid: vec![calm; SEA_GRID * SEA_GRID].into(),
            prev: vec![open; SEA_GRID * SEA_GRID].into(),
            baked_at: 10.0,
        };
        // just baked: it still shows the open sea it fades from
        let fresh = vec![calm; SEA_GRID * SEA_GRID];
        let shown = old.shown(10.0, Vec2::ZERO, &fresh);
        assert_eq!(shown[0], open);
        // half way: every value between the two, calm and depth included
        let half = old.shown(10.0 + SEA_FADE * 0.5, Vec2::ZERO, &fresh);
        for k in 0..6 {
            let (lo, hi) = (open[k].min(calm[k]), open[k].max(calm[k]));
            assert!(half[0][k] > lo && half[0][k] < hi, "value {k}: {}", half[0][k]);
        }
        // done: the new grid
        let done = old.shown(10.0 + SEA_FADE, Vec2::ZERO, &fresh)[0];
        assert!(done.iter().zip(calm).all(|(a, b)| (a - b).abs() < 1e-5), "{done:?}");
    }
    #[test]
    fn a_scan_that_found_nothing_is_not_run_again_at_the_next_frame() {
        assert!(scan_due(None, 0.0));
        assert!(!scan_due(Some(10.0), 10.004));
        assert!(!scan_due(Some(10.0), 10.3));
        assert!(scan_due(Some(10.0), 10.4));
    }

    #[test]
    fn the_ground_grid_is_sampled_at_its_cells_centres_row_by_row() {
        let origin = Vec2::new(100.0, -40.0);
        let grid = ground_grid(origin, 128.0, |xy| [xy.x, xy.y]);
        assert_eq!(grid.len(), 4096);
        for (i, sample) in grid.iter().enumerate() {
            let expected = origin + (Vec2::new((i % 64) as f32, (i / 64) as f32) + Vec2::splat(0.5)) * 128.0 / 64.0;
            assert_eq!(*sample, [expected.x, expected.y], "sample {i}");
        }
    }

    #[test]
    fn low_quality_keeps_entry_sheets_and_expires_spent_contact_impulses() {
        use crate::movement::water_jump::{Impact, Kind};
        let mut effects = Effects::default();
        effects.emit_impact(
            Impact {
                position: Vec3::ZERO,
                speed: 8.0,
                kind: Kind::Cannonball,
            },
            1.0,
            WaterEffects::default(),
        );
        for i in 0..150 {
            effects.events.push(WaterEvent {
                position: [0.0, 0.0, 0.0, 1.02 + i as f32 * 0.0001],
                params: [0.3, 7.0, 0.0, 0.0],
                ..Default::default()
            });
        }
        effects.retain_events(1.1, 0, Vec2::ZERO);
        assert_eq!(effects.events.len(), 64);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 5.0).count(), 10);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 6.0).count(), 1);
        effects.events.push(WaterEvent {
            position: [0.0, 0.0, 0.0, 1.0],
            params: [0.3, 4.0, 0.0, 0.0],
            ..Default::default()
        });
        effects.retain_events(1.4, 0, Vec2::ZERO);
        assert!(!effects.events.iter().any(|e| e.params[1] == 4.0));
        effects.retain_events(2.7, 0, Vec2::ZERO);
        assert!(effects.events.iter().all(|e| e.params[1] == 1.0));
    }

    #[test]
    fn intentional_water_entries_have_one_bounded_burst_and_independent_effect_switches() {
        use crate::movement::water_jump::{Impact, Kind};
        let impact = Impact {
            position: Vec3::new(2.0, 3.0, 20.0),
            speed: 8.0,
            kind: Kind::Cannonball,
        };
        let mut effects = Effects::default();
        effects.emit_impact(impact, 1.0, WaterEffects::default());
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 5.0).count(), 10);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 6.0).count(), 1);
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 1.0).count(), 1);
        let mut settings = WaterEffects {
            bubbles: false,
            splashes: false,
            ..Default::default()
        };
        effects.events.clear();
        effects.emit_impact(impact, 2.0, settings);
        assert_eq!(effects.events.len(), 1);
        assert_eq!(effects.events[0].params[1], 6.0);
        settings.interactions = false;
        effects.events.clear();
        effects.emit_impact(impact, 3.0, settings);
        assert!(effects.events.is_empty());
        effects.emit_impact(Impact { speed: f32::NAN, ..impact }, 3.0, WaterEffects::default());
        assert!(effects.events.is_empty());
        effects.emit_impact(
            Impact {
                kind: Kind::Dive,
                ..impact
            },
            4.0,
            WaterEffects::default(),
        );
        assert_eq!(effects.events.iter().filter(|e| e.params[1] == 5.0).count(), 6);
        assert!(effects.events.iter().all(|e| e.motion[3] <= 3.5));
        let dive: Vec<_> = effects.events.iter().filter(|e| e.params[1] == 5.0).collect();
        let span: f32 = dive.iter().map(|e| e.params[3]).sum();
        assert!(
            (span - std::f32::consts::TAU).abs() < 0.001,
            "one complete, non-overlapping entry sheet"
        );
        let mut cannonball = Effects::default();
        cannonball.emit_impact(impact, 4.0, WaterEffects::default());
        let big = cannonball.events.iter().find(|e| e.params[1] == 5.0).unwrap();
        assert!(big.motion[3] > dive[0].motion[3] && big.params[0] > dive[0].params[0]);
    }

    #[test]
    fn object_contacts_reject_teleports_and_preserve_independent_displacement() {
        let id = (Uuid::nil(), 10);
        let mut e = Effects::default();
        let s = WaterEffects::default();
        e.emit(id, Vec3::ZERO, 0.3, 0.0, 0.0, s);
        e.emit(id, Vec3::X * 100.0, 0.3, 0.0, 0.1, s);
        assert!(e.events.is_empty());
        e.emit(id, Vec3::new(100.2, 0.0, -0.2), 0.3, 0.0, 0.3, s);
        assert!(e.events.iter().any(|e| e.params[1] == 3.0));
        assert!(e.events.iter().any(|e| e.params[1] == 0.0));
        assert!(e.events.iter().all(|e| e.params[1] != 1.0));
    }
}
