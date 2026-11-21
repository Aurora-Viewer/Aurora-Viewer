//! The sea grid (`WaterGround::sea`, 128 cells over 512 m around the
//! camera): the depth the wind's sea (aurora_render::water_sea) feels, and
//! sheltered water, where what stands still in the water or just above it
//! calms it. The shaders and the CPU samplers read it the same way.
//!
//! Two things per cell: how calm it is kept by something crossing the water
//! there (a pier's piling, a hull, a wall, a house's foundations), spread a
//! few metres around and further downwind, its lee; and the clearance up to
//! the lowest underside above the water (a deck, a floor on stilts), under
//! which the crests are kept. The Legacy Viewer's water has no waves to
//! shelter.
//!
//! Prims are boxes here (their scale and rotation): a mesh shelters its
//! bounding box. What moves or is physical is left to the caller to leave
//! out: a boat must not calm the water it rides.

use aurora_render::water_effects::{OPEN_CLEARANCE, OPEN_DEPTH};
use glam::{Quat, Vec2, Vec3};

use aurora_render::water_effects::{LAND_DEPTH, SEA_GRID};

/// Cells baked around the grid on every side, so that its smoothing
/// finds the same neighbours wherever the grid is centred: a cell keeps
/// its value as the grid follows the camera (no waves popping).
const MARGIN: usize = 24;
/// How much of a mesh's box (across) calms the water round it.
const MESH_CORE: f32 = 0.6;
/// How far under the still water a prim must reach to cross it (m).
const PIERCE: f32 = 0.3;
/// Undersides higher than this above the water leave the waves alone (m).
const OVERHEAD: f32 = OPEN_CLEARANCE;
/// How much sea is left right at what crosses the water.
const CALMEST: f32 = 0.15;
/// Radius of the calm around a crossing (m), and how much further it
/// reaches downwind (its lee).
const RADIUS: f32 = 5.0;
const LEE: f32 = 3.0;

/// One prim, as its world box.
#[derive(Debug, Clone, Copy)]
pub struct ShelterBox {
    pub center: Vec3,
    pub rotation: Quat,
    /// Half its scale.
    pub half: Vec3,
    /// A mesh or a sculpt: its box is a poor stand-in for its shape (a reef
    /// rock's box covers far more water than the rock), so it calms only
    /// round its core and caps no clearance (decks and floors are boxes).
    pub mesh: bool,
}

impl ShelterBox {
    /// The heights where a vertical line at `xy` enters and leaves the
    /// box, if it meets it (slab test in the box's own frame).
    pub(super) fn span(&self, xy: Vec2, from: f32) -> Option<(f32, f32)> {
        let inv = self.rotation.inverse();
        let o = inv * (xy.extend(from) - self.center);
        let d = inv * Vec3::Z;
        let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
        for axis in 0..3 {
            let (o, d, h) = (o[axis], d[axis], self.half[axis]);
            if d.abs() < 1e-6 {
                if o.abs() > h {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-h - o) / d, (h - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        (near <= far).then_some((from + near, from + far))
    }

    /// The box's corners' bounds: xy, and lowest and highest z.
    fn bounds(&self) -> (Vec2, Vec2, f32, f32) {
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            ) * self.half;
            let p = self.center + self.rotation * corner;
            lo = lo.min(p);
            hi = hi.max(p);
        }
        (lo.truncate(), hi.truncate(), lo.z, hi.z)
    }
}

/// The grid over `origin`..`origin + extent` (cells as the water ground's,
/// `levels` its water height per cell), for `boxes` and a wind blowing
/// toward `wind` (zero: no lee). Per cell: [calm, clearance].
pub fn bake(origin: Vec2, extent: f32, size: usize, levels: &[f32], boxes: &[ShelterBox], wind: Vec2) -> Vec<[f32; 2]> {
    let mut grid = vec![[1.0, OPEN_CLEARANCE]; size * size];
    if levels.len() != size * size || !extent.is_finite() || extent <= 0.0 || boxes.is_empty() {
        return grid;
    }
    let cell = extent / size as f32;
    let centre = |x: usize, y: usize| origin + (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) * cell;
    let index = |p: Vec2| {
        let c = ((p - origin) / cell).floor();
        (c.x >= 0.0 && c.y >= 0.0 && c.x < size as f32 && c.y < size as f32).then(|| c.y as usize * size + c.x as usize)
    };
    let mut crossed = vec![false; size * size];
    for b in boxes {
        if !(b.center.is_finite() && b.rotation.is_finite() && b.half.is_finite()) {
            continue;
        }
        // a mesh shelters round the core of its box only
        let core;
        let b = if b.mesh {
            core = ShelterBox {
                half: Vec3::new(b.half.x * MESH_CORE, b.half.y * MESH_CORE, b.half.z),
                ..*b
            };
            &core
        } else {
            b
        };
        let (lo, hi, bottom, top) = b.bounds();
        let Some(home) = index(b.center.truncate()) else {
            continue;
        };
        let water = levels[home];
        if top < water - PIERCE || bottom > water + OVERHEAD {
            continue;
        }
        // a box thinner than a cell (a piling) still marks the cell it
        // stands in
        let mesh = b.mesh;
        let reach = |water: f32, (enter, leave): (f32, f32), i: usize, grid: &mut Vec<[f32; 2]>, crossed: &mut Vec<bool>| {
            if leave < water - PIERCE {
                return;
            }
            if enter <= water {
                crossed[i] = true;
                // (a mesh's box is not its shape: a reef rock's would hold
                // the sea flat over all the water it spans)
                if mesh {
                    return;
                }
                // what crosses the water holds it under its top too: a floor
                // laid at the water keeps the sea from showing through it
                // (a piling or a wall stands higher than any crest)
                grid[i][1] = grid[i][1].min((leave - water).max(0.0));
            } else if enter - water < OVERHEAD {
                grid[i][1] = grid[i][1].min(enter - water);
            }
        };
        reach(water, (bottom, top), home, &mut grid, &mut crossed);
        let first = ((lo - origin) / cell)
            .floor()
            .clamp(Vec2::ZERO, Vec2::splat(size as f32 - 1.0))
            .as_uvec2();
        let last = ((hi - origin) / cell)
            .floor()
            .clamp(Vec2::ZERO, Vec2::splat(size as f32 - 1.0))
            .as_uvec2();
        for y in first.y as usize..=last.y as usize {
            for x in first.x as usize..=last.x as usize {
                let i = y * size + x;
                if let Some(span) = b.span(centre(x, y), levels[i] - PIERCE) {
                    reach(levels[i], span, i, &mut grid, &mut crossed);
                }
            }
        }
    }
    // the calm spreads from what crosses the water, further downwind
    let toward = wind.try_normalize();
    let reach_cells = ((RADIUS * if toward.is_some() { LEE } else { 1.0 }) / cell).ceil() as i32;
    for (i, _) in crossed.iter().enumerate().filter(|(_, c)| **c) {
        let (cx, cy) = ((i % size) as i32, (i / size) as i32);
        for dy in -reach_cells..=reach_cells {
            for dx in -reach_cells..=reach_cells {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
                    continue;
                }
                let offset = Vec2::new(dx as f32, dy as f32) * cell;
                let distance = match toward {
                    Some(t) => {
                        let along = offset.dot(t);
                        let across = offset.perp_dot(t);
                        let along = if along > 0.0 { along / LEE } else { along };
                        (along * along + across * across).sqrt()
                    }
                    None => offset.length(),
                };
                let t = (distance / RADIUS).clamp(0.0, 1.0);
                let calm = CALMEST + (1.0 - CALMEST) * t * t * (3.0 - 2.0 * t);
                let j = y as usize * size + x as usize;
                grid[j][0] = grid[j][0].min(calm);
            }
        }
    }
    grid
}

/// The grid the waves read (`WaterGround::sea`): per cell the wave depth
/// (`heights`' depth, bounded to `OPEN_DEPTH`, land at 0, smoothed over
/// about 2 cells either side so that a step of the bed, a region's edge or
/// a cell of unknown ground slopes the sea instead of raising a wall of
/// water) and the shelter (smoothed over a cell, then quantized as the
/// texture keeps it). (The viewer bakes in two halves, `sample_ground`
/// then `bake_grid` on a worker; the tests in one.)
#[cfg(test)]
pub fn sea_grid(
    origin: Vec2,
    extent: f32,
    ground_water: impl Fn(Vec2) -> [f32; 2] + Sync,
    boxes: &[ShelterBox],
    wind: Vec2,
    swell: Swell,
) -> Vec<[f32; 6]> {
    bake_grid(origin, extent, &sample_ground(origin, extent, ground_water), boxes, wind, swell)
}

/// The ground and the water heights `sea_grid` bakes from, its margin
/// included (the part that reads the world: quick, on the caller's thread).
pub fn sample_ground(origin: Vec2, extent: f32, ground_water: impl Fn(Vec2) -> [f32; 2] + Sync) -> Vec<[f32; 2]> {
    let n = SEA_GRID + 2 * MARGIN;
    let cell = extent / SEA_GRID as f32;
    let from = origin - Vec2::splat(MARGIN as f32 * cell);
    // row by row through frame_par: called from the frame, a plain rayon
    // loop held it for a pool worker on a busy region (#4121)
    let rows: Vec<usize> = (0..n).collect();
    crate::frame_par::fold(
        &rows,
        crate::frame_par::Grain { chunk: 4, per_helper: 8 },
        Vec::new,
        |mut heights, &row| {
            heights.extend((0..n).map(|column| ground_water(from + (Vec2::new(column as f32, row as f32) + Vec2::splat(0.5)) * cell)));
            heights
        },
    )
    .into_iter()
    .flatten()
    .collect()
}

/// `sea_grid` from `sample_ground`'s heights (the slow part: the
/// refraction's marching, the shadow and the shelter; on a worker).
pub fn bake_grid(origin: Vec2, extent: f32, heights: &[[f32; 2]], boxes: &[ShelterBox], wind: Vec2, swell: Swell) -> Vec<[f32; 6]> {
    let n = SEA_GRID + 2 * MARGIN;
    if heights.len() != n * n {
        return Vec::new();
    }
    let cell = extent / SEA_GRID as f32;
    let from = origin - Vec2::splat(MARGIN as f32 * cell);
    let depth: Vec<f32> = heights.iter().map(|h| (h[1] - h[0]).clamp(-LAND_DEPTH, OPEN_DEPTH)).collect();
    let depth = smooth(&smooth(&smooth(&depth, n, 2), n, 2), n, 2);
    // (the marching's first-order fronts leave a cell-sized ripple in the
    // shallows, which the swell's direction would draw as tiles: smoothed,
    // a linear field stays as it is)
    let toward = swell.toward.try_normalize().unwrap_or(Vec2::X);
    let travel = smooth(&smooth(&refract(&depth, n, cell, from, origin, swell), n, 1), n, 1);
    let (reach, chop) = exposure(&depth, n, cell, swell.toward);
    // (land takes the water's beside it before the smoothing: counted as
    // open, it made a sheltered lake's shore look exposed to the ocean)
    let (reach, chop) = (from_water(&reach, &depth, n), from_water(&chop, &depth, n));
    let (reach, chop) = (smooth(&smooth(&reach, n, 2), n, 2), smooth(&smooth(&chop, n, 2), n, 2));
    let inner = |i: usize| (i / SEA_GRID + MARGIN) * n + i % SEA_GRID + MARGIN;
    let levels: Vec<f32> = (0..SEA_GRID * SEA_GRID).map(|i| heights[inner(i)][1]).collect();
    let shelter = bake(origin, extent, SEA_GRID, &levels, boxes, wind);
    let calm: Vec<f32> = shelter.iter().map(|c| c[0]).collect();
    let clearance: Vec<f32> = shelter.iter().map(|c| c[1]).collect();
    // (the clearance spreads a cell before it is smoothed: averaged with
    // open neighbours, a small floor's would no longer hold the crests)
    let (calm, clearance) = (smooth(&calm, SEA_GRID, 1), smooth(&lowest(&clearance, SEA_GRID), SEA_GRID, 1));
    (0..SEA_GRID * SEA_GRID)
        .map(|i| {
            let [c, h] = aurora_render::water_effects::quantize_shelter(calm[i], clearance[i]);
            // the refraction as its offset from the open sea's plane wave:
            // open water then follows the swell as it turns, with no jump
            // when the grid is baked again for a new direction
            let centre = origin + (Vec2::new((i % SEA_GRID) as f32, (i / SEA_GRID) as f32) + Vec2::splat(0.5)) * cell;
            let offset = travel[inner(i)] - (centre - origin).dot(toward);
            let [r, k] = aurora_render::water_effects::quantize_exposure(reach[inner(i)], chop[inner(i)]);
            [depth[inner(i)], c, h, offset, r, k]
        })
        .collect()
}

/// The main swell the sea grid refracts: the way it travels (unit) and its
/// wavenumber in open water (rad/m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swell {
    pub toward: Vec2,
    pub k: f32,
}

/// How much slower than in open water (`OPEN_DEPTH`) the swell travels
/// `depth` deep, the refraction index: shallow water slows it (Eckart's
/// form of the dispersion relation, k·d ≈ k0·d / √tanh(k0·d)), up to 6
/// times; land it cannot cross, but it can be reached across (for the
/// swash) at a cost no wave takes when water goes round.
fn index(depth: f32, k: f32) -> f32 {
    if depth <= 0.02 {
        return 40.0;
    }
    ((k * OPEN_DEPTH).tanh() / (k * depth.max(0.05)).tanh()).sqrt().clamp(1.0, 6.0)
}

/// The swell's travel per cell of an `n`-square grid of `cell` m from
/// `from`: how far along it the swell has come, in open-water metres from
/// `origin` (its phase there is k · travel). Fast marching (the eikonal
/// |∇travel| = index) from the edges the swell comes in by, where it is
/// the open sea's plane wave: it slows and shortens in the shallows, turns
/// to face every shore and wraps round islands, its two arms crossing in
/// their lee. In open water it is the plane wave exactly (the first-order
/// update keeps linear fields). Cells it cannot reach keep the plane.
fn refract(depth: &[f32], n: usize, cell: f32, from: Vec2, origin: Vec2, swell: Swell) -> Vec<f32> {
    use std::cmp::Reverse;
    #[derive(PartialEq)]
    struct Key(f32);
    impl Eq for Key {}
    impl PartialOrd for Key {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Key {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
    let toward = swell.toward.try_normalize().unwrap_or(Vec2::X);
    let centre = |i: usize| from + (Vec2::new((i % n) as f32, (i / n) as f32) + Vec2::splat(0.5)) * cell;
    let plane = |i: usize| (centre(i) - origin).dot(toward);
    let mut travel = vec![f32::INFINITY; n * n];
    let mut done = vec![false; n * n];
    let mut heap = std::collections::BinaryHeap::new();
    for i in 0..n * n {
        let (x, y) = (i % n, i / n);
        // the edges the swell comes in by (their outward normal against it)
        let entering = (x == 0 && toward.x > 1e-3)
            || (x == n - 1 && toward.x < -1e-3)
            || (y == 0 && toward.y > 1e-3)
            || (y == n - 1 && toward.y < -1e-3);
        if entering && depth[i] > 0.02 {
            travel[i] = plane(i);
            heap.push(Reverse((Key(travel[i]), i)));
        }
    }
    let k = swell.k.max(1e-4);
    while let Some(Reverse((Key(t), i))) = heap.pop() {
        if done[i] || t > travel[i] {
            continue;
        }
        done[i] = true;
        let (x, y) = ((i % n) as i32, (i / n) as i32);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                continue;
            }
            let j = ny as usize * n + nx as usize;
            if done[j] {
                continue;
            }
            // the upwind (done) neighbours on each axis
            let at = |x: i32, y: i32| {
                if x < 0 || y < 0 || x >= n as i32 || y >= n as i32 {
                    return f32::INFINITY;
                }
                let c = y as usize * n + x as usize;
                if done[c] { travel[c] } else { f32::INFINITY }
            };
            let a = at(nx - 1, ny).min(at(nx + 1, ny));
            let b = at(nx, ny - 1).min(at(nx, ny + 1));
            let f = cell * index(depth[j], k);
            let t = if (a - b).abs() >= f || !a.is_finite() || !b.is_finite() {
                a.min(b) + f
            } else {
                (a + b + (2.0 * f * f - (a - b) * (a - b)).sqrt()) * 0.5
            };
            if t < travel[j] {
                travel[j] = t;
                heap.push(Reverse((Key(t), j)));
            }
        }
    }
    (0..n * n)
        .map(|i| if travel[i].is_finite() { travel[i] } else { plane(i) })
        .collect()
}

/// How exposed each cell is to the sea (1 open, less sheltered by land):
/// how much of the open sea's swell reaches it, and how much wind chop it
/// can build. A fan of rays looks back up the swell (`toward` is the way it
/// goes), up to 400 m: the share that find open water (beyond that, or
/// beyond the grid) is how much swell comes in, bending round an island but
/// not into a lake; the open water upwind along the middle ray, the fetch,
/// is how much chop the wind raises there (a lake of 200 m builds short
/// chop and no swell). The caller smooths both, which softens the shadow's
/// edges the way diffraction does.
fn exposure(depth: &[f32], n: usize, cell: f32, toward: Vec2) -> (Vec<f32>, Vec<f32>) {
    const RAYS: [(f32, f32); 7] = [
        (0.0, 3.0),
        (-20.0, 2.0),
        (20.0, 2.0),
        (-45.0, 1.5),
        (45.0, 1.5),
        (-70.0, 1.0),
        (70.0, 1.0),
    ];
    const REACH: f32 = 400.0;
    let back = -toward.try_normalize().unwrap_or(Vec2::X);
    let steps = (REACH / cell).ceil() as usize;
    let total: f32 = RAYS.iter().map(|r| r.1).sum();
    let (reach, chop): (Vec<f32>, Vec<f32>) = (0..n * n)
        .map(|i| {
            if depth[i] <= 0.0 {
                return (f32::NAN, f32::NAN);
            }
            let start = Vec2::new((i % n) as f32, (i / n) as f32) + Vec2::splat(0.5);
            // how far each ray goes over water before land (None: open)
            let land = |angle: f32| {
                let dir = Vec2::from_angle(angle.to_radians()).rotate(back);
                for s in 1..=steps {
                    let p = start + dir * s as f32;
                    if p.x < 0.0 || p.y < 0.0 || p.x >= n as f32 || p.y >= n as f32 {
                        return None;
                    }
                    if depth[p.y as usize * n + p.x as usize] < 0.3 {
                        return Some(s as f32 * cell);
                    }
                }
                None
            };
            let mut swell = 0.0;
            let mut fetch = REACH * 5.0;
            for (k, &(angle, weight)) in RAYS.iter().enumerate() {
                let hit = land(angle);
                let t = hit.map_or(1.0, |d| ((d - 100.0) / (REACH - 100.0)).clamp(0.0, 1.0));
                swell += weight * t * t * (3.0 - 2.0 * t);
                if k == 0 {
                    fetch = hit.unwrap_or(fetch);
                }
            }
            swell /= total;
            // the swell needs a long way of open water to build, the chop a
            // short one (and grows with the swell that comes in)
            let built = (fetch / 1500.0).clamp(0.0, 1.0);
            let reach = swell.max(built * built * (3.0 - 2.0 * built));
            let chop = (fetch / 800.0).sqrt().clamp(0.05, 1.0) * (0.6 + 0.4 * reach);
            (reach, chop)
        })
        .unzip();
    (reach, chop)
}

/// `values` on land (NaN there) filled from the water beside it, a cell a
/// pass for 6 passes (the swash runs up the beach with its sea's values);
/// land further in takes 1.
fn from_water(values: &[f32], depth: &[f32], n: usize) -> Vec<f32> {
    let mut v = values.to_vec();
    if v.len() != n * n || depth.len() != n * n {
        return v;
    }
    for _ in 0..6 {
        let prev = v.clone();
        for i in 0..n * n {
            if !prev[i].is_nan() {
                continue;
            }
            let (x, y) = ((i % n) as i32, (i / n) as i32);
            let (mut sum, mut count) = (0.0, 0);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && nx < n as i32 && ny < n as i32 {
                    let o = prev[(ny * n as i32 + nx) as usize];
                    if !o.is_nan() {
                        sum += o;
                        count += 1;
                    }
                }
            }
            if count > 0 {
                v[i] = sum / count as f32;
            }
        }
    }
    v.iter().map(|x| if x.is_nan() { 1.0 } else { *x }).collect()
}

/// Each cell's lowest value among its own and its 8 neighbours'.
fn lowest(values: &[f32], size: usize) -> Vec<f32> {
    if values.len() != size * size {
        return values.to_vec();
    }
    let n = size as i32;
    (0..size * size)
        .map(|i| {
            let (x, y) = ((i % size) as i32, (i / size) as i32);
            let mut low = values[i];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < n && ny < n {
                        low = low.min(values[(ny * n + nx) as usize]);
                    }
                }
            }
            low
        })
        .collect()
}

/// A box blur of `radius` cells over a `size`-square grid (edges clamped).
fn smooth(values: &[f32], size: usize, radius: i32) -> Vec<f32> {
    if values.len() != size * size {
        return values.to_vec();
    }
    let n = size as i32;
    let pass = |src: &[f32], dx: i32, dy: i32| -> Vec<f32> {
        (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as i32, (i / size) as i32);
                let sum: f32 = (-radius..=radius)
                    .map(|k| {
                        let (sx, sy) = ((x + k * dx).clamp(0, n - 1), (y + k * dy).clamp(0, n - 1));
                        src[(sy * n + sx) as usize]
                    })
                    .sum();
                sum / (2 * radius + 1) as f32
            })
            .collect()
    };
    pass(&pass(values, 1, 0), 0, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 64;
    const SWELL: Swell = Swell { toward: Vec2::X, k: 0.1 };
    fn bake_one(b: ShelterBox, wind: Vec2) -> Vec<[f32; 2]> {
        // 128 m from the origin: 2 m cells, still water at 20 m
        bake(Vec2::ZERO, 128.0, SIZE, &[20.0; SIZE * SIZE], &[b], wind)
    }
    fn at(grid: &[[f32; 2]], x: f32, y: f32) -> [f32; 2] {
        grid[(y / 2.0) as usize * SIZE + (x / 2.0) as usize]
    }

    #[test]
    fn a_mesh_rock_does_not_hold_the_sea_over_its_whole_box() {
        // a rock's box 20 m across through the water, its top 0.3 m above it
        let rock = |mesh: bool| ShelterBox {
            center: Vec3::new(64.0, 64.0, 19.0),
            rotation: Quat::IDENTITY,
            half: Vec3::new(10.0, 10.0, 1.3),
            mesh,
        };
        let levels = vec![20.0; 128 * 128];
        let at = |grid: &[[f32; 2]], x: f32| grid[64 * 128 + x as usize];
        let as_box = bake(Vec2::ZERO, 128.0, 128, &levels, &[rock(false)], Vec2::ZERO);
        let as_mesh = bake(Vec2::ZERO, 128.0, 128, &levels, &[rock(true)], Vec2::ZERO);
        // a box prim crossing the water caps it under its top (a floor at
        // the water); a mesh's box does not
        assert!(at(&as_box, 58.0)[1] < 0.5);
        assert_eq!(at(&as_mesh, 58.0)[1], OPEN_CLEARANCE);
        // near the edge of its box (8 m off its centre) the mesh's water is
        // calmer than open water no more than it is round its core
        assert!(at(&as_mesh, 56.0)[0] > at(&as_box, 56.0)[0]);
    }

    #[test]
    fn a_piling_calms_around_it_and_further_downwind() {
        let piling = ShelterBox {
            center: Vec3::new(64.5, 64.5, 19.0),
            rotation: Quat::IDENTITY,
            half: Vec3::new(0.15, 0.15, 3.0),
            mesh: false,
        };
        let grid = bake_one(piling, Vec2::X);
        assert!((at(&grid, 64.5, 64.5)[0] - CALMEST).abs() < 1e-5, "{:?}", at(&grid, 64.5, 64.5));
        assert!(at(&grid, 72.5, 64.5)[0] < 0.9, "the lee, 8 m downwind: {:?}", at(&grid, 72.5, 64.5));
        assert_eq!(at(&grid, 56.5, 64.5)[0], 1.0, "8 m upwind is open");
        assert_eq!(at(&grid, 20.5, 20.5), [1.0, OPEN_CLEARANCE]);
    }

    #[test]
    fn a_deck_caps_the_waves_under_it_without_calming_them() {
        let deck = ShelterBox {
            center: Vec3::new(64.0, 64.0, 20.9),
            rotation: Quat::IDENTITY,
            half: Vec3::new(6.0, 2.0, 0.1),
            mesh: false,
        };
        let grid = bake_one(deck, Vec2::ZERO);
        let [calm, clearance] = at(&grid, 61.0, 64.5);
        assert_eq!(calm, 1.0);
        assert!((clearance - 0.8).abs() < 1e-4, "{clearance}");
        assert_eq!(at(&grid, 64.5, 70.5)[1], OPEN_CLEARANCE, "beside the deck");
    }

    #[test]
    fn a_floor_at_the_water_holds_the_sea_under_it() {
        let floor = ShelterBox {
            center: Vec3::new(64.0, 64.0, 20.0),
            rotation: Quat::IDENTITY,
            half: Vec3::new(5.0, 5.0, 0.1),
            mesh: false,
        };
        let grid = bake_one(floor, Vec2::ZERO);
        let [_, clearance] = at(&grid, 64.5, 64.5);
        assert!(clearance <= 0.1 + 1e-5, "{clearance}");
    }

    #[test]
    fn what_lies_under_the_water_or_high_above_it_shelters_nothing() {
        for z in [10.0, 30.0] {
            let b = ShelterBox {
                center: Vec3::new(64.0, 64.0, z),
                rotation: Quat::IDENTITY,
                half: Vec3::splat(2.0),
                mesh: false,
            };
            assert!(bake_one(b, Vec2::X).iter().all(|c| *c == [1.0, OPEN_CLEARANCE]), "{z}");
        }
    }

    #[test]
    fn a_cliff_of_the_bed_or_unknown_ground_slopes_the_wave_depth() {
        // shallow (1 m) west of x 256, unknown ground (100 m under) east
        let ground_water = |p: Vec2| if p.x < 256.0 { [19.0, 20.0] } else { [-80.0, 20.0] };
        let grid = sea_grid(Vec2::ZERO, 512.0, ground_water, &[], Vec2::ZERO, SWELL);
        let row: Vec<f32> = (56..72).map(|x| grid[10 * SEA_GRID + x][0]).collect();
        assert!(row.windows(2).all(|w| w[1] >= w[0]), "monotonic: {row:?}");
        assert!(row.windows(2).all(|w| w[1] - w[0] < OPEN_DEPTH * 0.3), "no step: {row:?}");
        assert_eq!(grid[10 * SEA_GRID][0], 1.0);
        assert_eq!(grid[10 * SEA_GRID + 127][0], OPEN_DEPTH);
        assert_eq!([grid[0][1], grid[0][2]], [1.0, OPEN_CLEARANCE]);
        // land counts below 0, and a cell keeps its value as the grid moves
        let island = |p: Vec2| {
            if p.distance(Vec2::splat(300.0)) < 40.0 {
                [23.0, 20.0]
            } else {
                [10.0, 20.0]
            }
        };
        let a = sea_grid(Vec2::ZERO, 512.0, island, &[], Vec2::ZERO, SWELL);
        let b = sea_grid(Vec2::splat(64.0), 512.0, island, &[], Vec2::ZERO, SWELL);
        let at =
            |g: &[[f32; 6]], origin: f32, x: f32, y: f32| g[((y - origin) / 4.0) as usize * SEA_GRID + ((x - origin) / 4.0) as usize][0];
        assert!(at(&a, 0.0, 302.0, 302.0) < 0.0, "land: {}", at(&a, 0.0, 302.0, 302.0));
        assert_eq!(at(&a, 0.0, 338.0, 302.0), at(&b, 64.0, 338.0, 302.0), "stable as the grid moves");
    }

    #[test]
    fn the_swell_refracts_round_an_island_and_leaves_a_shadow() {
        // open water 20 m deep, an island at (256, 256) in a shallow shelf,
        // the swell going east (+x), its plane travel x from the origin
        let island = |p: Vec2| {
            let r = p.distance(Vec2::splat(256.0));
            let ground = if r < 30.0 { 22.0 } else { 20.0 - (r - 30.0).min(60.0) / 3.0 };
            [ground, 20.0]
        };
        let swell = Swell {
            toward: Vec2::new(0.8, 0.6),
            k: 0.08,
        };
        let g = sea_grid(Vec2::ZERO, 512.0, island, &[], Vec2::ZERO, swell);
        let cell = |x: f32, y: f32| g[(y / 4.0) as usize * SEA_GRID + (x / 4.0) as usize];
        // open water far from the island: the plane wave
        // (the travel is kept as its offset from that plane)
        let far = cell(50.0, 450.0);
        assert!(far[3].abs() < 2.0, "{}", far[3]);
        assert_eq!(far[4], 1.0, "no shadow in open water");
        // the shallows slow it: more travel than the plane on the way in
        let shelf = Vec2::splat(256.0) - swell.toward * 45.0;
        let s = cell(shelf.x, shelf.y);
        assert!(s[3] > 1.0, "the shallows lag behind the plane: {}", s[3]);
        // and behind the island, downwave, the swell is dimmed
        let lee = Vec2::splat(256.0) + swell.toward * 70.0;
        assert!(cell(lee.x, lee.y)[4] < 0.7, "shadow: {}", cell(lee.x, lee.y)[4]);
    }

    #[test]
    fn a_lake_gets_no_swell_and_short_chop() {
        // a round lake 160 m across in open land
        let lake = |p: Vec2| {
            let r = p.distance(Vec2::splat(256.0));
            [if r < 80.0 { 16.0 } else { 22.0 }, 20.0]
        };
        let swell = Swell { toward: Vec2::X, k: 0.08 };
        let g = sea_grid(Vec2::ZERO, 512.0, lake, &[], Vec2::ZERO, swell);
        let middle = g[64 * SEA_GRID + 64];
        assert!(middle[4] < 0.15, "no ocean swell in a lake: {}", middle[4]);
        assert!(middle[5] > 0.05 && middle[5] < 0.5, "short chop: {}", middle[5]);
        // open sea all round: both whole
        let sea = sea_grid(Vec2::ZERO, 512.0, |_| [0.0, 20.0], &[], Vec2::ZERO, swell);
        assert_eq!([sea[64 * SEA_GRID + 64][4], sea[64 * SEA_GRID + 64][5]], [1.0, 1.0]);
    }

    #[test]
    fn a_tilted_ramp_meets_the_water_where_it_dips() {
        // a 20 m plank sloping 2 m over its length, its middle at the water
        let ramp = ShelterBox {
            center: Vec3::new(64.0, 64.0, 20.0),
            rotation: Quat::from_rotation_y(0.1),
            half: Vec3::new(10.0, 1.0, 0.05),
            mesh: false,
        };
        let grid = bake_one(ramp, Vec2::ZERO);
        assert!(at(&grid, 59.0, 64.5)[1] > 0.2, "raised end: {:?}", at(&grid, 59.0, 64.5));
        assert!(at(&grid, 69.0, 64.5)[0] < 0.5, "dipped end crosses: {:?}", at(&grid, 69.0, 64.5));
    }
}
