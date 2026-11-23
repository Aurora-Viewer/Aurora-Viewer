//! Aurora Weather's tumbleweeds (the Debris card's switch, the Dust storm
//! preset): a few real meshes, drawn for this viewer only, that roll across
//! open ground with the wind. Few (five at most), big (0.5 to 1 m) and on
//! the ground, they are simulated here rather than with the GPU debris:
//! rays through the picking's triangles find the ground under them and the
//! walls and fences ahead (the GPU's depth or shelter map would miss a
//! fence post at this size), and they go in the frame's lists like any
//! mesh, with their shadows.
//!
//! They start upwind near the edge of the shelter map's reach (about 36 m),
//! out of view, on open ground (nothing above); they roll with the wind
//! and its gusts (turning as they roll, ω = v / r, never sliding), hop on
//! the terrain's bumps, bounce in a strong gust, stop against a wall and
//! break free on the next gust. They go far downwind, or after being stuck
//! too long, never while in view.

use super::Scene;
use crate::world::World;
use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, Renderer, ShadowCaster, Vertex};
use glam::{Mat4, Quat, Vec2, Vec3};
use std::time::Instant;

/// The most rolling at once.
pub const MAX: usize = 5;
/// How many mesh shapes they take.
const VARIANTS: u32 = 3;
/// Where they start from the camera (m, upwind) and how far they may roll
/// before they go (out of view).
const SPAWN_AT: f32 = 36.0;
const GONE_AT: f32 = 46.0;
/// Stuck this long against a wall (s), they go (out of view).
const STUCK_MAX: f32 = 25.0;
/// Under this wind (m/s), none start.
const CALM: f32 = 2.0;

/// `AURORA_DEMO_TUMBLEWEEDS=1` (demo only): they start 12 m upwind, in view
/// or not, so that a capture shows them.
fn demo_near() -> bool {
    static NEAR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NEAR.get_or_init(|| std::env::var_os("AURORA_DEMO").is_some() && std::env::var("AURORA_DEMO_TUMBLEWEEDS").as_deref() == Ok("1"))
}

struct Weed {
    pos: Vec3,
    vel: Vec3,
    rot: Quat,
    radius: f32,
    variant: usize,
    stuck: f32,
    record: u32,
}

#[derive(Default)]
pub struct Tumbleweeds {
    weeds: Vec<Weed>,
    meshes: Vec<MeshAlloc>,
    failed: bool,
    last: Option<Instant>,
    next_spawn: f32,
    rng: u64,
}

impl Tumbleweeds {
    fn rand(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// One step: `wind` the weather's wind (m/s) and gust (0..1), None with
    /// the switch off (all of them go at once); `eye`, `forward` and
    /// `fov_y` the camera, to start and end them out of view.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        renderer: &mut Renderer,
        scene: &Scene,
        world: &World,
        now: Instant,
        wind: Option<(Vec2, f32)>,
        eye: Vec3,
        forward: Vec3,
        fov_y: f32,
    ) {
        let Some((wind, gust)) = wind else {
            self.clear(renderer);
            return;
        };
        if self.failed {
            return;
        }
        if self.meshes.is_empty() {
            for v in 0..VARIANTS {
                let (vertices, indices) = weed_mesh(v);
                match renderer.upload_mesh(&vertices, &indices) {
                    Some(m) => self.meshes.push(m),
                    None => {
                        log::warn!("tumbleweeds: no room for their meshes");
                        self.clear(renderer);
                        self.failed = true;
                        return;
                    }
                }
            }
            self.rng = 0x9e37_79b9_7f4a_7c15 ^ now.elapsed().as_nanos() as u64;
        }
        let dt = self.last.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32().min(0.1));
        self.last = Some(now);
        let in_view = |p: Vec3| {
            let to = p - eye;
            let d = to.length();
            // (generous: the view is wider than it is tall)
            d < 60.0 && to.dot(forward) > d * (fov_y * 0.95).cos()
        };
        // start one upwind now and then, out of view, on open ground
        self.next_spawn -= dt;
        if self.weeds.len() < MAX && wind.length() > CALM && self.next_spawn <= 0.0 {
            self.next_spawn = 2.0 + 6.0 * self.rand();
            let dir = wind.normalize();
            let near = demo_near();
            let across = Vec2::new(-dir.y, dir.x) * (self.rand() - 0.5) * if near { 8.0 } else { 30.0 };
            let back = if near { 12.0 } else { SPAWN_AT * (0.85 + 0.15 * self.rand()) };
            // (the demo's: ahead in view, a little upwind, to roll across it)
            let ahead = forward.truncate().normalize_or(Vec2::X) * if near { 10.0 } else { 0.0 };
            let back = if near { 5.0 } else { back };
            let at = eye.truncate() + ahead - dir * back + across;
            let radius = 0.25 + 0.25 * self.rand();
            if let Some(ground) = scene.ground_below(world, at.extend(eye.z + 30.0), 80.0) {
                let pos = at.extend(ground + radius);
                if (near || !in_view(pos)) && !scene.covered_above(world, pos + Vec3::Z * radius, 60.0) {
                    let variant = (self.rand() * VARIANTS as f32) as usize % VARIANTS as usize;
                    let record = renderer.records.alloc(DrawRecord::default());
                    let rot = Quat::from_rotation_z(self.rand() * std::f32::consts::TAU);
                    log::info!(
                        "tumbleweeds: one starts {:.0} m upwind ({:.1}, {:.1}), radius {radius:.2} m",
                        (pos - eye).truncate().length(),
                        pos.x,
                        pos.y
                    );
                    self.weeds.push(Weed {
                        pos,
                        vel: (wind * 0.4).extend(0.0),
                        rot,
                        radius,
                        variant,
                        stuck: 0.0,
                        record,
                    });
                }
            }
        }
        let mut i = 0;
        while i < self.weeds.len() {
            let big_bounce = gust > 0.8 && self.rand() < dt * 0.4;
            let bounce_up = 2.0 + 2.5 * self.rand();
            let nudge = (self.rand() - 0.5) * 2.0;
            let w = &mut self.weeds[i];
            step(w, scene, world, dt, wind, gust, big_bounce.then_some(bounce_up), nudge);
            let off = (w.pos - eye).truncate().length();
            // far off downwind (past the camera), or stuck too long
            let downwind = (w.pos - eye).truncate().dot(wind) > 0.0;
            let gone = ((off > GONE_AT && downwind) || off > GONE_AT * 1.5 || w.stuck > STUCK_MAX) && !in_view(w.pos);
            if gone {
                log::info!(
                    "tumbleweeds: one goes {off:.0} m away{}",
                    if w.stuck > STUCK_MAX { ", stuck against a wall" } else { "" }
                );
                let w = self.weeds.swap_remove(i);
                renderer.records.free(w.record);
                continue;
            }
            renderer.records.set(
                w.record,
                DrawRecord {
                    model: Mat4::from_scale_rotation_translation(Vec3::splat(w.radius), w.rot, w.pos).to_cols_array_2d(),
                    // dry stems, a straw brown
                    base_color: [0.5, 0.4, 0.27, 1.0],
                    ..Default::default()
                },
            );
            i += 1;
        }
    }

    /// Adds them to the frame's lists, with their shadows.
    pub fn append(&self, lists: &mut DrawLists) {
        for w in &self.weeds {
            let Some(mesh) = self.meshes.get(w.variant) else {
                continue;
            };
            let cmd = DrawCmd {
                index_count: mesh.index_count,
                first_index: mesh.index_offset,
                base_vertex: mesh.vertex_offset as i32,
                record: w.record,
                bounds: [0.0; 4],
            }
            .with_bounds(w.pos, w.radius * 1.2);
            lists.opaque_two_sided.push(cmd);
            lists.shadow_casters.push(ShadowCaster {
                cmd,
                center: w.pos,
                radius: w.radius * 1.2,
                alpha: false,
            });
        }
    }

    pub fn clear(&mut self, renderer: &mut Renderer) {
        for w in self.weeds.drain(..) {
            renderer.records.free(w.record);
        }
        for m in self.meshes.drain(..) {
            renderer.free_mesh(m);
        }
        self.last = None;
    }
}

/// One weed's step: rolling with the wind (`wind` m/s, `gust` 0..1), on the
/// ground under it, stopped by what is ahead; `bounce` an upward kick (a
/// strong gust's), `nudge` -1..1 to break free sideways.
#[allow(clippy::too_many_arguments)]
fn step(w: &mut Weed, scene: &Scene, world: &World, dt: f32, wind: Vec2, gust: f32, bounce: Option<f32>, nudge: f32) {
    if dt <= 0.0 {
        return;
    }
    let r = w.radius;
    let ground = scene.ground_below(world, w.pos + Vec3::Z * (r + 1.0), r + 6.0);
    let on_ground = ground.is_some_and(|g| w.pos.z - r <= g + 0.03);
    // the air pushes it toward a share of the wind's speed (rolling
    // resistance keeps it slower), more in the gusts
    let target = wind * (0.45 + 0.45 * gust);
    let k = if on_ground { 1.2 } else { 0.4 };
    let mut v = w.vel;
    let flat = v.truncate() + (target - v.truncate()) * (1.0 - (-k * dt).exp());
    v = flat.extend(v.z - 9.81 * dt);
    if on_ground && let Some(up) = bounce {
        v.z = up;
    }
    // what is ahead: a wall or a fence stops it (it rests against it until
    // a gust takes it round)
    let ahead = v.truncate();
    if ahead.length() > 0.05 {
        let dir = ahead.normalize().extend(0.0);
        let far = r + ahead.length() * dt + 0.05;
        if scene.obstacle_ahead(world, w.pos, dir, far) {
            if gust > 0.7 {
                // a gust: it rolls along the wall
                let along = Vec2::new(-dir.y, dir.x) * nudge.signum() * ahead.length() * 0.5;
                v = along.extend(v.z);
            } else {
                v = Vec3::new(0.0, 0.0, v.z);
            }
            w.stuck += dt;
        } else {
            w.stuck = 0.0;
        }
    }
    let mut pos = w.pos + v * dt;
    if let Some(g) = scene.ground_below(world, pos + Vec3::Z * (r + 1.0), r + 6.0)
        && pos.z - r < g
    {
        // on the ground: a bump ahead lifts it (a hop), a fall bounces
        let rise = g + r - pos.z;
        pos.z = g + r;
        if v.z < -1.0 {
            v.z = -v.z * 0.35;
        } else {
            v.z = (rise / dt).min(3.0) * 0.5;
        }
    }
    // turning as it rolls: ω = v / r about the axis across its motion
    let roll = v.truncate();
    let speed = roll.length();
    if speed > 1e-3 {
        let axis = Vec3::Z.cross(roll.extend(0.0) / speed);
        w.rot = (Quat::from_axis_angle(axis, speed * dt / r) * w.rot).normalize();
    }
    w.pos = pos;
    w.vel = v;
}

/// A tumbleweed's mesh, `variant` 0..VARIANTS: a rough ball of crooked
/// twigs, radius about 1 (the record scales it), thin square stems between
/// points inside its shell, each bent once.
pub fn weed_mesh(variant: u32) -> (Vec<Vertex>, Vec<u16>) {
    let mut seed = 0x5bd1_e995u32 ^ variant.wrapping_mul(0x9e37_79b9);
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let point = |inside: f32, rand: &mut dyn FnMut() -> f32| {
        let z = rand() * 2.0 - 1.0;
        let a = rand() * std::f32::consts::TAU;
        let s = (1.0 - z * z).max(0.0).sqrt();
        let squash = 0.85 + 0.15 * (variant as f32 / 2.0);
        Vec3::new(s * a.cos(), s * a.sin(), z * squash) * (inside + (1.0 - inside) * rand())
    };
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let twigs = 70 + variant * 15;
    for _ in 0..twigs {
        let a = point(0.55, &mut rand);
        let b = point(0.55, &mut rand);
        // bent once, outward
        let mid = (a + b) * 0.5;
        let bend = mid + mid.normalize_or(Vec3::Z) * (0.1 + 0.15 * rand());
        let width = 0.012 + 0.01 * rand();
        for (p, q) in [(a, bend), (bend, b)] {
            twig(&mut vertices, &mut indices, p, q, width);
        }
    }
    (vertices, indices)
}

/// A thin square stem from `a` to `b`, `width` across: four flat sides.
fn twig(vertices: &mut Vec<Vertex>, indices: &mut Vec<u16>, a: Vec3, b: Vec3, width: f32) {
    let along = (b - a).normalize_or(Vec3::Z);
    let side = along.any_orthonormal_vector() * width;
    let up = along.cross(side).normalize_or(Vec3::Z) * width;
    let corners = [side, up, -side, -up];
    for k in 0..4 {
        let (c0, c1) = (corners[k], corners[(k + 1) % 4]);
        let n = (c0 + c1).normalize_or(Vec3::Z).to_array();
        let base = vertices.len() as u16;
        for (p, uv) in [
            (a + c0, [0.0, 0.0]),
            (b + c0, [1.0, 0.0]),
            (b + c1, [1.0, 1.0]),
            (a + c1, [0.0, 1.0]),
        ] {
            vertices.push(Vertex::new(p.to_array(), n, uv));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meshes_are_rough_balls_that_fit_their_radius_and_u16_indices() {
        for v in 0..VARIANTS {
            let (vertices, indices) = weed_mesh(v);
            assert!(vertices.len() < u16::MAX as usize, "{}", vertices.len());
            assert_eq!(indices.len() % 3, 0);
            assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
            let far = vertices.iter().map(|v| Vec3::from_array(v.pos).length()).fold(0.0f32, f32::max);
            assert!(far > 0.8 && far < 1.35, "variant {v}: {far}");
        }
        // the variants differ
        assert_ne!(weed_mesh(0).0.len(), weed_mesh(1).0.len());
    }
}
