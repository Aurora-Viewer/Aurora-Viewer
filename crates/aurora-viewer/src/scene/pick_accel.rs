//! The triangles of a picking face in a bounding volume hierarchy, for the
//! hover search (hover.rs).
//!
//! `pick::ray_faces` tests every triangle of an object once the ray meets
//! its box: 20,000 to 100,000 for a mesh body, a building or a landscape,
//! 0.1 to 0.5 ms each time. A click pays it once. The hover search pays it
//! again whenever the cursor moves or something moves near its ray, which
//! among dancing avatars is every frame, and the ray meets the same large
//! objects each time.
//!
//! The hierarchy gives the same distance as testing every triangle (the
//! same arithmetic on the triangles it does not rule out, and it only
//! rules out those whose box the ray misses or meets farther than the
//! nearest hit), a few dozen triangles tested instead of all. It is built
//! by a background job the first time the hover search tests a large face
//! of a mesh or a sculpt ([`PickAccel::get_or_ask`]): until it is there,
//! every triangle is tested as before. Clicks and the full search of
//! AURORA_HOVER_CHECK keep testing every triangle.

use crate::media::pick::ray_triangle;
use glam::Vec3;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Faces with fewer triangles are tested one by one: the hierarchy would
/// save less than it costs to build and keep.
pub const MIN_TRIANGLES: usize = 512;

/// Triangles of a leaf at most (more when they cannot be told apart).
const LEAF: usize = 4;

#[derive(Clone, Copy, Debug)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: its first triangle in `order`. Inner node: its first child
    /// (the second follows it).
    first: u32,
    /// Triangles of a leaf; [`INNER`] for an inner node.
    count: u32,
}

/// `Node::count` of an inner node.
const INNER: u32 = u32::MAX;

/// Bounding volume hierarchy over the triangles of one face.
#[derive(Debug)]
pub struct Accel {
    nodes: Vec<Node>,
    /// Triangle numbers (index / 3), leaf by leaf.
    order: Vec<u32>,
    /// Largest side of the root box.
    extent: f32,
}

impl Accel {
    /// Over the triangles of `indices` whose three positions exist (the
    /// others are never hit).
    pub fn build(positions: &[[f32; 3]], indices: &[u16]) -> Accel {
        struct Tri {
            number: u32,
            min: Vec3,
            max: Vec3,
            center: Vec3,
        }
        let mut tris: Vec<Tri> = indices
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
            .filter_map(|(number, t)| {
                let p = |i: u16| positions.get(i as usize).map(|p| Vec3::from_array(*p));
                let (p0, p1, p2) = (p(t[0])?, p(t[1])?, p(t[2])?);
                let (min, max) = (p0.min(p1).min(p2), p0.max(p1).max(p2));
                Some(Tri {
                    number: number as u32,
                    min,
                    max,
                    center: (min + max) * 0.5,
                })
            })
            .collect();
        let bounds = |tris: &[Tri]| {
            tris.iter()
                .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), t| {
                    (lo.min(t.min), hi.max(t.max))
                })
        };
        let (min, max) = if tris.is_empty() { (Vec3::ZERO, Vec3::ZERO) } else { bounds(&tris) };
        let mut nodes = vec![Node {
            min,
            max,
            first: 0,
            count: tris.len() as u32,
        }];
        // (node, its triangles as a range of `tris`)
        let mut todo = vec![(0usize, 0usize, tris.len())];
        while let Some((node, start, end)) = todo.pop() {
            let slice = &mut tris[start..end];
            if slice.len() <= LEAF {
                continue;
            }
            let (lo, hi) = slice
                .iter()
                .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), t| {
                    (lo.min(t.center), hi.max(t.center))
                });
            let spread = hi - lo;
            let axis = if spread.x >= spread.y && spread.x >= spread.z {
                0
            } else if spread.y >= spread.z {
                1
            } else {
                2
            };
            // (all at the same place: nothing to tell them apart by)
            if spread[axis].partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                continue;
            }
            let half = slice.len() / 2;
            slice.select_nth_unstable_by(half, |a, b| a.center[axis].total_cmp(&b.center[axis]));
            let mid = start + half;
            let first = nodes.len();
            for (s, e) in [(start, mid), (mid, end)] {
                let (min, max) = bounds(&tris[s..e]);
                todo.push((nodes.len(), s, e));
                nodes.push(Node {
                    min,
                    max,
                    first: s as u32,
                    count: (e - s) as u32,
                });
            }
            nodes[node].first = first as u32;
            nodes[node].count = INNER;
        }
        Accel {
            nodes,
            order: tris.iter().map(|t| t.number).collect(),
            extent: (max - min).max_element(),
        }
    }

    /// The distance of the nearest triangle the ray `o + t d` (object
    /// space) meets nearer than `best`, or `best`: what testing every
    /// triangle of `indices` in turn gives, with the same back-face policy.
    pub fn nearest(&self, positions: &[[f32; 3]], indices: &[u16], o: Vec3, d: Vec3, two_sided: bool, mut best: f32) -> f32 {
        // The triangle test accepts a ray that passes a hair outside a
        // triangle, by rounding: up to about 1e-6 of the distances it
        // handles. The boxes are widened by a hundred times that, so that a
        // triangle it would accept is never ruled out by its box.
        let pad = 1e-4 * (o.abs().max_element() + self.extent);
        let inv = d.recip();
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let Some(node) = self.nodes.get(stack[top] as usize) else {
                continue;
            };
            let (mut near, mut far) = (0.0f32, best);
            let mut miss = false;
            for axis in 0..3 {
                let (lo, hi) = (node.min[axis] - pad, node.max[axis] + pad);
                if d[axis] == 0.0 {
                    miss |= o[axis] < lo || o[axis] > hi;
                } else {
                    let (a, b) = ((lo - o[axis]) * inv[axis], (hi - o[axis]) * inv[axis]);
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                }
            }
            if miss || far < near {
                continue;
            }
            if node.count == INNER {
                // (the depth is bounded: a node holds half its parent's
                // triangles)
                if top + 2 <= stack.len() {
                    stack[top] = node.first;
                    stack[top + 1] = node.first + 1;
                    top += 2;
                }
                continue;
            }
            let leaf = self.order.get(node.first as usize..(node.first + node.count) as usize);
            for &number in leaf.unwrap_or_default() {
                let Some(t) = indices.get(number as usize * 3..number as usize * 3 + 3) else {
                    continue;
                };
                let p = |i: u16| positions.get(i as usize).map(|p| Vec3::from_array(*p));
                let (Some(p0), Some(p1), Some(p2)) = (p(t[0]), p(t[1]), p(t[2])) else {
                    continue;
                };
                if let Some(hit) = ray_triangle(o, d, p0, p1, p2, two_sided)
                    && hit.t > 0.0
                    && hit.t < best
                {
                    best = hit.t;
                }
            }
        }
        best
    }

    /// Where the segment `o + t d`, `t0 < t < t1` (object space), crosses
    /// triangles, into `out` (`t`, in no order): what testing every
    /// triangle of `indices` in turn finds (sound occlusion,
    /// scene/sound_occlusion.rs).
    pub fn crossings(&self, positions: &[[f32; 3]], indices: &[u16], o: Vec3, d: Vec3, (t0, t1): (f32, f32), out: &mut Vec<f32>) {
        // (the padding of `nearest`)
        let pad = 1e-4 * (o.abs().max_element() + self.extent);
        let inv = d.recip();
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let Some(node) = self.nodes.get(stack[top] as usize) else {
                continue;
            };
            let (mut near, mut far) = (t0, t1);
            let mut miss = false;
            for axis in 0..3 {
                let (lo, hi) = (node.min[axis] - pad, node.max[axis] + pad);
                if d[axis] == 0.0 {
                    miss |= o[axis] < lo || o[axis] > hi;
                } else {
                    let (a, b) = ((lo - o[axis]) * inv[axis], (hi - o[axis]) * inv[axis]);
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                }
            }
            if miss || far < near {
                continue;
            }
            if node.count == INNER {
                if top + 2 <= stack.len() {
                    stack[top] = node.first;
                    stack[top + 1] = node.first + 1;
                    top += 2;
                }
                continue;
            }
            let leaf = self.order.get(node.first as usize..(node.first + node.count) as usize);
            for &number in leaf.unwrap_or_default() {
                let Some(t) = indices.get(number as usize * 3..number as usize * 3 + 3) else {
                    continue;
                };
                let p = |i: u16| positions.get(i as usize).map(|p| Vec3::from_array(*p));
                let (Some(p0), Some(p1), Some(p2)) = (p(t[0]), p(t[1]), p(t[2])) else {
                    continue;
                };
                if let Some(hit) = ray_triangle(o, d, p0, p1, p2, true)
                    && hit.t > t0
                    && hit.t < t1
                {
                    out.push(hit.t);
                }
            }
        }
    }
}

/// The hierarchy of a picking face, built on demand off the main thread.
#[derive(Default)]
pub struct PickAccel {
    built: OnceLock<Accel>,
    asked: AtomicBool,
}

impl PickAccel {
    /// The hierarchy when it is there. Otherwise `ask` is called, once, to
    /// have it built (it calls [`PickAccel::build`] from a job).
    pub fn get_or_ask(&self, ask: impl FnOnce()) -> Option<&Accel> {
        let built = self.built.get();
        if built.is_none() && !self.asked.swap(true, Ordering::Relaxed) {
            ask();
        }
        built
    }

    /// Build the hierarchy of the face (from a background job).
    pub fn build(&self, positions: &[[f32; 3]], indices: &[u16]) {
        let _ = self.built.set(Accel::build(positions, indices));
    }

    /// The hierarchy, built here if it is not there yet: for a caller that
    /// already runs in a background job (sound occlusion).
    pub fn get_or_build(&self, positions: &[[f32; 3]], indices: &[u16]) -> &Accel {
        self.asked.store(true, Ordering::Relaxed);
        self.built.get_or_init(|| Accel::build(positions, indices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::pick::{FaceTris, ray_faces_with_sidedness};
    use glam::Mat4;

    struct Rng(u64);

    impl Rng {
        fn unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }

        fn point(&mut self, size: f32) -> Vec3 {
            Vec3::new(self.unit() - 0.5, self.unit() - 0.5, self.unit() - 0.5) * size
        }
    }

    /// Every triangle in turn, as the full search does.
    fn brute(positions: &[[f32; 3]], indices: &[u16], o: Vec3, d: Vec3, two_sided: bool) -> Option<f32> {
        let face = FaceTris {
            face: 0,
            positions,
            uvs: &[],
            indices,
        };
        ray_faces_with_sidedness(std::iter::once(face), Mat4::IDENTITY, o, d, |_| two_sided).map(|h| h.t)
    }

    fn check(positions: &[[f32; 3]], indices: &[u16], rng: &mut Rng, rays: usize, far: f32) -> usize {
        let accel = Accel::build(positions, indices);
        let mut hits = 0;
        for _ in 0..rays {
            // toward the object, from near and from far, and along the axes
            let target = rng.point(1.2);
            let o = rng.point(2.0) + rng.point(2.0).normalize_or(Vec3::X) * far * rng.unit();
            let mut d = (target - o).normalize_or(Vec3::X) * (0.2 + rng.unit() * 3.0);
            if rng.unit() < 0.1 {
                d = Vec3::AXES[(rng.unit() * 2.99) as usize] * if rng.unit() < 0.5 { 1.0 } else { -1.0 };
            }
            for two_sided in [false, true] {
                let expected = brute(positions, indices, o, d, two_sided);
                let got = accel.nearest(positions, indices, o, d, two_sided, f32::INFINITY);
                assert_eq!(
                    (got < f32::INFINITY).then_some(got).map(f32::to_bits),
                    expected.map(f32::to_bits),
                    "ray {o} {d}, two_sided={two_sided}: {got} for {expected:?}"
                );
                // A hit beyond the current nearest distance cannot replace it.
                if let Some(t) = expected {
                    hits += 1;
                    assert_eq!(accel.nearest(positions, indices, o, d, two_sided, t), t);
                    assert_eq!(accel.nearest(positions, indices, o, d, two_sided, t * 0.5), t * 0.5);
                }
            }
        }
        hits
    }

    #[test]
    fn same_distance_as_every_triangle_in_turn() {
        let mut rng = Rng(7);
        // a soup of triangles of every size
        let positions: Vec<[f32; 3]> = (0..3000).map(|_| rng.point(1.0).to_array()).collect();
        let mut indices: Vec<u16> = Vec::new();
        for _ in 0..4000 {
            let a = (rng.unit() * 2999.0) as u16;
            let size = if rng.unit() < 0.9 { 30.0 } else { 2999.0 };
            let near = |rng: &mut Rng| ((a as f32 + rng.unit() * size) as usize % 3000) as u16;
            indices.extend([a, near(&mut rng), near(&mut rng)]);
        }
        // (an index without a position, and a leftover index)
        indices.extend([1, 2, 60000, 5]);
        let hits = check(&positions, &indices, &mut rng, 3000, 3.0) + check(&positions, &indices, &mut rng, 1500, 2000.0);
        assert!(hits > 1000, "{hits} rays hit");
    }

    #[test]
    fn flat_and_repeated_triangles() {
        let mut rng = Rng(11);
        // a grid in the plane z = 0.25: boxes without thickness, edges
        // and corners shared, rays along the axes
        const N: u16 = 40;
        let mut positions = Vec::new();
        for y in 0..=N {
            for x in 0..=N {
                positions.push([x as f32 / N as f32 - 0.5, y as f32 / N as f32 - 0.5, 0.25]);
            }
        }
        let mut indices = Vec::new();
        for y in 0..N {
            for x in 0..N {
                let a = y * (N + 1) + x;
                let b = a + N + 1;
                indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        // the same triangle many times: nothing to split by
        for _ in 0..40 {
            indices.extend([0, 1, N + 1]);
        }
        let hits = check(&positions, &indices, &mut rng, 3000, 5.0);
        assert!(hits > 500, "{hits} rays hit");
        // straight down onto shared edges and corners
        let accel = Accel::build(&positions, &indices);
        for i in 0..=N {
            let o = Vec3::new(i as f32 / N as f32 - 0.5, 0.1, 3.0);
            for two_sided in [false, true] {
                let got = accel.nearest(&positions, &indices, o, -Vec3::Z, two_sided, f32::INFINITY);
                assert_eq!(
                    Some(got).filter(|t| *t < f32::INFINITY),
                    brute(&positions, &indices, o, -Vec3::Z, two_sided)
                );
            }
        }
    }

    #[test]
    fn crossings_count_what_every_triangle_counts() {
        let mut rng = Rng(23);
        let positions: Vec<[f32; 3]> = (0..2000).map(|_| rng.point(1.0).to_array()).collect();
        let indices: Vec<u16> = (0..9000).map(|_| (rng.unit() * 1999.0) as u16).collect();
        let accel = Accel::build(&positions, &indices);
        let mut crossed = 0;
        for _ in 0..800 {
            let (from, to) = (rng.point(3.0), rng.point(3.0));
            let d = to - from;
            let range = (0.05, 0.95);
            let mut expected: Vec<f32> = indices
                .as_chunks::<3>()
                .0
                .iter()
                .filter_map(|t| {
                    let p = |i: u16| Vec3::from_array(positions[i as usize]);
                    ray_triangle(from, d, p(t[0]), p(t[1]), p(t[2]), true)
                        .map(|h| h.t)
                        .filter(|t| *t > range.0 && *t < range.1)
                })
                .collect();
            let mut got = Vec::new();
            accel.crossings(&positions, &indices, from, d, range, &mut got);
            expected.sort_by(f32::total_cmp);
            got.sort_by(f32::total_cmp);
            assert_eq!(got, expected, "{from} → {to}");
            crossed += expected.len();
        }
        assert!(crossed > 1000, "{crossed} crossings");
        // built on demand by the caller's own job, and kept
        let face = PickAccel::default();
        let built = face.get_or_build(&positions, &indices) as *const Accel;
        assert!(std::ptr::eq(face.get_or_ask(|| panic!("already built")).unwrap(), built));
    }

    #[test]
    fn empty_and_small_faces() {
        let accel = Accel::build(&[], &[]);
        assert_eq!(accel.nearest(&[], &[], Vec3::ZERO, Vec3::X, false, 5.0), 5.0);
        let positions = [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 0.0, 1.0]];
        let accel = Accel::build(&positions, &[0, 1, 2]);
        assert_eq!(accel.nearest(&positions, &[0, 1, 2], Vec3::ZERO, Vec3::X, true, f32::INFINITY), 1.0);
        assert_eq!(
            accel.nearest(&positions, &[0, 1, 2], Vec3::ZERO, Vec3::X, false, f32::INFINITY),
            f32::INFINITY
        );
        assert_eq!(
            accel.nearest(&positions, &[0, 1, 2], Vec3::X * 2.0, -Vec3::X, false, f32::INFINITY),
            1.0
        );
        assert_eq!(
            accel.nearest(&positions, &[0, 1, 2], Vec3::ZERO, -Vec3::X, true, f32::INFINITY),
            f32::INFINITY
        );
    }

    #[test]
    fn asked_once_and_used_when_built() {
        let accel = PickAccel::default();
        let mut asked = 0;
        assert!(accel.get_or_ask(|| asked += 1).is_none());
        assert!(accel.get_or_ask(|| asked += 1).is_none());
        assert_eq!(asked, 1);
        accel.build(&[[0.0; 3]; 3], &[0, 1, 2]);
        assert!(accel.get_or_ask(|| asked += 1).is_some());
        assert_eq!(asked, 1);
    }
}
