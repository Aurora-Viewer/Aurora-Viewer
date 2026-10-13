//! Local lights by screen tile and depth.
//!
//! Every lit fragment used to walk the frame's whole light list (up to 64
//! lights: the face lights of a crowd fill it) to find the two or three
//! whose sphere holds it. The main view is cut in `TILES` columns, `TILES`
//! rows and `SLICES` slices of depth; a light, seen from the camera, covers
//! a range of each. The frame uniforms hold, for every column, row and
//! slice, the set of the lights that cover it, and a fragment walks the
//! lights that are in the sets of its column, of its row and of its slice at
//! once (`light_walk`, `next_light` in common.wgsl), in the list's order.
//!
//! Same image: a light left out holds none of the fragments drawn there
//! (the shaders add nothing for a light whose radius the fragment is out
//! of), and the lights that stay are added in the same order.
//!
//! The other views of a frame (reflections, probe faces, impostor pictures)
//! keep the whole list ([`LightSets::whole`]).
//!
//! The reflection probes of a fragment are found the same way, in sets of
//! their own ([`bin_spheres`], `probes::tiles`).

use crate::types::PointLight;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

/// Columns, and rows, of the screen.
pub const TILES: usize = 32;
/// Slices of depth. Slice `floor(log2(depth / SLICE_NEAR) * SLICES_PER_OCTAVE)`:
/// the first also holds what is nearer than [`SLICE_NEAR`], the last what is
/// beyond 400 m.
pub const SLICES: usize = 32;
/// Must match `LIGHT_SLICE_NEAR` and `LIGHT_SLICES_PER_OCTAVE` in common.wgsl.
pub const SLICE_NEAR: f32 = 0.25;
pub const SLICES_PER_OCTAVE: f32 = 3.0;
/// Lights a set can hold: the frame's light list (`MAX_LIGHTS`).
pub const LIGHTS: usize = 64;
/// Pixels added around a light's rectangle: a fragment is lit at the centre
/// of its pixel, and the column and row of a pixel are found in single
/// precision.
const MARGIN_PX: f32 = 2.0;
/// Depth (clip w, metres in front of the camera) under which nothing is
/// drawn: the box around a light is cut there before it is projected.
pub(crate) const NEAREST: f32 = 1e-3;

/// Where the shaders find the lights of a fragment
/// (AURORA_LIGHT_TILES; the image is the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightCulling {
    /// In the sets of its screen tile and depth (the default).
    Tiles,
    /// In the whole list, as before (`0`).
    All,
    /// Either, each frame choosing with
    /// [`crate::FrameParams::all_lights`] (`alt`).
    Alternate,
    /// In the sets, and the shaders paint magenta any fragment that a light
    /// of the list reaches without being in them (`check`): none, ever.
    Check,
}

impl LightCulling {
    pub fn from_env() -> LightCulling {
        static MODE: std::sync::OnceLock<LightCulling> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| match std::env::var("AURORA_LIGHT_TILES").as_deref().map(str::trim) {
            Ok("0") => LightCulling::All,
            Ok("alt") => LightCulling::Alternate,
            Ok("check") => LightCulling::Check,
            _ => LightCulling::Tiles,
        })
    }
}

/// The light sets of a view, as the frame uniforms hold them (`Frame` in
/// common.wgsl). A set is two words, the low then the high 32 lights (bit
/// `i`: light `i` of the frame's list); the columns, rows and slices hold
/// two sets per element.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightSets {
    /// x, y: columns and rows per pixel.
    pub grid: [f32; 4],
    /// The camera's forward axis: a fragment's depth is its distance from
    /// the camera along it.
    pub forward: [f32; 4],
    /// x: 1 when the sets below narrow the list (0: the whole list, one
    /// light after the other, as before); y: 1 to check them
    /// ([`LightCulling::Check`]).
    pub mode: [u32; 4],
    pub columns: [[u32; 4]; TILES / 2],
    pub rows: [[u32; 4]; TILES / 2],
    pub slices: [[u32; 4]; SLICES / 2],
}

impl LightSets {
    /// The whole list for every fragment.
    pub fn whole() -> LightSets {
        Zeroable::zeroed()
    }

    /// Member `i` in every set: what reaches every fragment whatever its
    /// extent says.
    pub fn hold_everywhere(&mut self, i: usize) {
        let (word, bit) = (i / 32 % 2, 1u32 << (i % 32));
        for pair in self.columns.iter_mut().chain(&mut self.rows).chain(&mut self.slices) {
            pair[word] |= bit;
            pair[2 + word] |= bit;
        }
    }

    /// The members a fragment at pixel (`x`, `y`) and `depth` walks, as the
    /// shaders find them (`light_set`, `probe_tiles` in common.wgsl).
    #[cfg(test)]
    pub(crate) fn at(&self, x: f32, y: f32, depth: f32) -> u64 {
        if self.mode[0] == 0 {
            return u64::MAX;
        }
        let set = |table: &[[u32; 4]], i: usize| {
            let pair = table[i / 2];
            let (lo, hi) = if i.is_multiple_of(2) {
                (pair[0], pair[1])
            } else {
                (pair[2], pair[3])
            };
            lo as u64 | (hi as u64) << 32
        };
        let tile = |px: f32, per_px: f32| ((px * per_px) as usize).min(TILES - 1);
        set(&self.columns, tile(x, self.grid[0])) & set(&self.rows, tile(y, self.grid[1])) & set(&self.slices, slice_of(depth))
    }
}

fn words(set: u64) -> [u32; 2] {
    [set as u32, (set >> 32) as u32]
}

/// The slice of a depth, as `light_walk` finds it in the shaders.
fn slice_of(depth: f32) -> usize {
    let s = ((depth.max(SLICE_NEAR) / SLICE_NEAR).log2() * SLICES_PER_OCTAVE).floor();
    (s.max(0.0) as usize).min(SLICES - 1)
}

/// What a light may reach of the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reach {
    /// Columns `x[0]..=x[1]`, rows `y[0]..=y[1]`, slices `z[0]..=z[1]`.
    x: [usize; 2],
    y: [usize; 2],
    z: [usize; 2],
}

/// The columns, rows and slices a light may reach on a screen of `size`
/// pixels, seen through `view` and `proj` by a camera at `eye` looking
/// along `forward`; None when it reaches nothing that is drawn.
///
/// The box around the sphere, aligned with the camera, is cut by the plane
/// just in front of the camera and what is left is projected: the corners
/// of that convex piece enclose on screen every point of the sphere that can
/// be drawn. A light around the camera takes the whole screen, one beside
/// it its side of the screen, one behind it nothing.
fn reach(view: Mat4, proj: Mat4, eye: Vec3, forward: Vec3, size: [f32; 2], position: Vec3, radius: f32) -> Option<Reach> {
    const WHOLE: [usize; 2] = [0, TILES - 1];
    // (a hair wider than the radius the shaders compare distances with)
    let r = radius * 1.001 + 1e-3;
    // depth, by the shaders' own formula; widened by more than what single
    // precision can make of it at the far end of a region
    let depth = forward.dot(position - eye);
    let (near, far) = ((depth - r) * 0.998 - 0.01, (depth + r) * 1.002 + 0.01);
    if far.is_nan() || near.is_nan() {
        return Some(Reach {
            x: WHOLE,
            y: WHOLE,
            z: [0, SLICES - 1],
        });
    }
    if far < 0.0 {
        return None;
    }
    let z = [slice_of(near), slice_of(far)];
    let c = view.transform_point3(position);
    let corners: [Vec4; 8] = std::array::from_fn(|i| {
        let side = |bit: usize| if i & bit == 0 { -r } else { r };
        proj * (c + Vec3::new(side(1), side(2), side(4))).extend(1.0)
    });
    if corners.iter().any(|p| !p.is_finite()) {
        return Some(Reach { x: WHOLE, y: WHOLE, z });
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    let mut seen = false;
    let mut add = |p: Vec4| {
        let px = [(p.x / p.w * 0.5 + 0.5) * size[0], (0.5 - p.y / p.w * 0.5) * size[1]];
        for axis in 0..2 {
            lo[axis] = lo[axis].min(px[axis]);
            hi[axis] = hi[axis].max(px[axis]);
        }
        seen = true;
    };
    for (i, &p) in corners.iter().enumerate() {
        if p.w >= NEAREST {
            add(p);
        }
        // the edges to the three corners after this one: where they cross
        // the plane, a corner of the piece in front of it
        for bit in [1, 2, 4] {
            let q = corners[i | bit];
            if i & bit == 0 && (p.w >= NEAREST) != (q.w >= NEAREST) {
                add(p + (q - p) * ((NEAREST - p.w) / (q.w - p.w)));
            }
        }
    }
    if !seen {
        // wholly behind the camera
        return None;
    }
    let mut tiles = [WHOLE; 2];
    for axis in 0..2 {
        let (a, b) = (lo[axis] - MARGIN_PX, hi[axis] + MARGIN_PX);
        if a.is_nan() || b.is_nan() {
            continue;
        }
        if b < 0.0 || a > size[axis] {
            return None;
        }
        let per_px = TILES as f32 / size[axis].max(1.0);
        let tile = |px: f32| ((px * per_px).floor().max(0.0) as usize).min(TILES - 1);
        tiles[axis] = [tile(a), tile(b)];
    }
    Some(Reach {
        x: tiles[0],
        y: tiles[1],
        z,
    })
}

/// The light sets of the main view: `lights` in the order of the frame's
/// list (its first [`LIGHTS`]), the screen `size` in pixels, the camera at
/// `eye` (the position the shaders measure depths from).
pub fn bin(view: Mat4, proj: Mat4, eye: Vec3, size: [u32; 2], lights: &[PointLight]) -> LightSets {
    bin_spheres(view, proj, eye, size, lights.iter().map(|l| (l.position, l.radius)))
}

/// The sets of the main view for members that each reach the fragments of
/// a sphere (centre, radius) and no other: the first [`LIGHTS`] of
/// `spheres`, in their order.
pub fn bin_spheres(view: Mat4, proj: Mat4, eye: Vec3, size: [u32; 2], spheres: impl IntoIterator<Item = (Vec3, f32)>) -> LightSets {
    let size = [size[0].max(1) as f32, size[1].max(1) as f32];
    let forward = view.inverse().transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::X);
    let (mut columns, mut rows, mut slices) = ([0u64; TILES], [0u64; TILES], [0u64; SLICES]);
    for (i, (position, radius)) in spheres.into_iter().take(LIGHTS).enumerate() {
        let Some(reach) = reach(view, proj, eye, forward, size, position, radius) else {
            continue;
        };
        for (sets, [from, to]) in [(&mut columns[..], reach.x), (&mut rows[..], reach.y), (&mut slices[..], reach.z)] {
            for set in &mut sets[from..=to] {
                *set |= 1 << i;
            }
        }
    }
    let pack = |sets: &[u64]| -> [[u32; 4]; TILES / 2] {
        std::array::from_fn(|i| {
            let ([a, b], [c, d]) = (words(sets[2 * i]), words(sets[2 * i + 1]));
            [a, b, c, d]
        })
    };
    let mut out = LightSets::whole();
    out.mode[0] = 1;
    out.grid = [TILES as f32 / size[0], TILES as f32 / size[1], 0.0, 0.0];
    out.forward = forward.extend(0.0).to_array();
    out.columns = pack(&columns);
    out.rows = pack(&rows);
    out.slices = pack(&slices);
    out
}

// (the three tables share `pack`)
const _: () = assert!(TILES == SLICES && TILES.is_multiple_of(2));

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [u32; 2] = [1280, 720];

    fn camera() -> (Mat4, Mat4) {
        // at the origin, looking along +X, Z up (the viewer's world axes)
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, SIZE[0] as f32 / SIZE[1] as f32, 0.1);
        (view, proj)
    }

    fn light(position: Vec3, radius: f32) -> PointLight {
        PointLight {
            position,
            radius,
            color: Vec3::ONE,
            falloff: 0.5,
            projector: None,
        }
    }

    fn set(table: &[[u32; 4]], i: usize) -> u64 {
        let pair = table[i / 2];
        let (lo, hi) = if i.is_multiple_of(2) {
            (pair[0], pair[1])
        } else {
            (pair[2], pair[3])
        };
        lo as u64 | (hi as u64) << 32
    }

    /// The lights of a fragment at pixel (`x`, `y`) and `depth`, as
    /// `light_walk` finds them in the shaders.
    fn set_at(sets: &LightSets, x: f32, y: f32, depth: f32) -> u64 {
        sets.at(x, y, depth)
    }

    #[test]
    fn a_member_held_everywhere_is_in_every_set() {
        let (view, proj) = camera();
        // a light behind the camera reaches nothing of the view
        let mut sets = bin(view, proj, Vec3::ZERO, SIZE, &vec![light(Vec3::new(-10.0, 0.0, 0.0), 2.0); 40]);
        assert_eq!(set_at(&sets, 300.0, 200.0, 12.0), 0);
        sets.hold_everywhere(0);
        sets.hold_everywhere(39);
        for (x, y, depth) in [(0.5, 0.5, 0.01), (300.0, 200.0, 12.0), (1279.5, 719.5, 5000.0)] {
            assert_eq!(set_at(&sets, x, y, depth), 1 | 1 << 39, "pixel {x},{y}, depth {depth}");
        }
    }

    fn random_lights(mut seed: u32) -> Vec<PointLight> {
        let mut rnd = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        (0..LIGHTS)
            .map(|_| {
                let p = Vec3::new(rnd() * 60.0 - 15.0, rnd() * 40.0 - 20.0, rnd() * 20.0 - 10.0);
                light(p, 0.2 + rnd() * rnd() * 12.0)
            })
            .collect()
    }

    /// What the sets must never lose: a fragment inside a light's sphere
    /// finds the light. Checked along the view rays through the centre of
    /// every eighth pixel, at depths from the near plane to beyond the
    /// lights, for lights all around the camera (in view, beside it, behind
    /// it, around it, far away), from a camera high above a region too.
    #[test]
    fn a_fragment_finds_every_light_that_reaches_it() {
        let (_, proj) = camera();
        for (seed, eye) in [(12345, Vec3::ZERO), (777, Vec3::new(200.0, 131.0, 3050.0))] {
            let view = glam::camera::rh::view::look_at_mat4(eye, eye + Vec3::X, Vec3::Z);
            let lights: Vec<PointLight> = random_lights(seed).into_iter().map(|l| light(l.position + eye, l.radius)).collect();
            let sets = bin(view, proj, eye, SIZE, &lights);
            assert_eq!(sets.mode, [1, 0, 0, 0]);
            let forward = Vec3::from_slice(&sets.forward[..3]);
            let inv = (proj * view).inverse();
            let (mut reached, mut walked, mut fragments) = (0u32, 0u32, 0u32);
            for y in (0..SIZE[1]).step_by(8) {
                for x in (0..SIZE[0]).step_by(8) {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let ndc = Vec3::new(fx / SIZE[0] as f32 * 2.0 - 1.0, 1.0 - fy / SIZE[1] as f32 * 2.0, 0.5);
                    let dir = (inv.project_point3(ndc) - eye).normalize();
                    let mut depth = 0.1;
                    while depth < 90.0 {
                        let p = eye + dir * (depth / dir.dot(forward));
                        let found = set_at(&sets, fx, fy, forward.dot(p - eye));
                        fragments += 1;
                        walked += found.count_ones();
                        for (i, l) in lights.iter().enumerate() {
                            if p.distance(l.position) < l.radius {
                                reached += 1;
                                assert!(found & (1 << i) != 0, "light {i} ({l:?}) missing at pixel {x},{y}, depth {depth}");
                            }
                        }
                        depth *= 1.07;
                    }
                }
            }
            // and the sets do narrow the list: a few lights a fragment
            let mean = walked as f32 / fragments as f32;
            assert!(reached > 1000 && mean < 8.0, "{reached} reached, {mean} lights walked a fragment");
        }
    }

    #[test]
    fn a_light_in_view_takes_its_rectangle_and_its_depths_only() {
        let (view, proj) = camera();
        // 20 m ahead, 1 m of radius: a small disc at the centre of the screen
        let sets = bin(view, proj, Vec3::ZERO, SIZE, &[light(Vec3::new(20.0, 0.0, 0.0), 1.0)]);
        let centre = (SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0);
        assert_eq!(set_at(&sets, centre.0, centre.1, 20.0), 1);
        assert_eq!(set_at(&sets, centre.0, centre.1, 10.0), 0);
        assert_eq!(set_at(&sets, centre.0, centre.1, 40.0), 0);
        assert_eq!(set_at(&sets, 10.0, 10.0, 20.0), 0);
        assert_eq!(set_at(&sets, SIZE[0] as f32 - 10.0, centre.1, 20.0), 0);
        let held = |table: &[[u32; 4]]| (0..TILES).filter(|&i| set(table, i) != 0).count();
        assert!((2..=6).contains(&held(&sets.columns)), "{} columns", held(&sets.columns));
        assert!((2..=6).contains(&held(&sets.rows)), "{} rows", held(&sets.rows));
        assert!((1..=2).contains(&held(&sets.slices)), "{} slices", held(&sets.slices));
    }

    #[test]
    fn lights_around_beside_or_behind_the_camera() {
        let (view, proj) = camera();
        let sets = bin(
            view,
            proj,
            Vec3::ZERO,
            SIZE,
            &[
                // the camera inside the sphere: the whole screen, up to its far side
                light(Vec3::new(1.0, 0.0, 0.0), 5.0),
                // beside the camera on its left (+Y), across its plane: the
                // left of the screen only
                light(Vec3::new(1.0, 1.5, 0.0), 1.2),
                // far out of view to the left: nothing
                light(Vec3::new(5.0, 80.0, 0.0), 2.0),
                // wholly behind the camera: nothing
                light(Vec3::new(-10.0, 0.0, 0.0), 2.0),
            ],
        );
        let (w, h) = (SIZE[0] as f32, SIZE[1] as f32);
        for (x, y) in [(w / 2.0, 3.0), (w - 1.0, h - 1.0), (w - 1.0, 1.0)] {
            assert_eq!(set_at(&sets, x, y, 0.2), 0b0001, "pixel {x},{y}");
            assert_eq!(set_at(&sets, x, y, 5.0), 0b0001, "pixel {x},{y}");
            assert_eq!(set_at(&sets, x, y, 12.0), 0, "pixel {x},{y}");
        }
        assert_eq!(set_at(&sets, 1.0, h / 2.0, 1.0), 0b0011);
    }

    #[test]
    fn the_whole_list_for_the_other_views() {
        // nothing narrows it: any fragment walks the whole list
        assert_eq!(LightSets::whole().mode[0], 0);
        assert_eq!(set_at(&LightSets::whole(), 500.0, 300.0, 7.0), u64::MAX);
    }

    /// A list longer than the sets can hold is cut like the frame's.
    #[test]
    fn lights_beyond_the_list_are_left_out() {
        let (view, proj) = camera();
        let lights = vec![light(Vec3::new(20.0, 0.0, 0.0), 1.0); LIGHTS + 5];
        let sets = bin(view, proj, Vec3::ZERO, SIZE, &lights);
        assert_eq!(set_at(&sets, SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0, 20.0), u64::MAX);
    }

    /// The slices of the shaders: `LIGHT_SLICE_NEAR`, `LIGHT_SLICES_PER_OCTAVE`.
    #[test]
    fn slices_match_the_shaders() {
        let wgsl = include_str!("shaders/common.wgsl");
        assert!(wgsl.contains(&format!("const LIGHT_SLICE_NEAR: f32 = {SLICE_NEAR:?};")));
        assert!(wgsl.contains(&format!("const LIGHT_SLICES_PER_OCTAVE: f32 = {SLICES_PER_OCTAVE:?};")));
        assert!(wgsl.contains(&format!("const LIGHT_TILES: u32 = {TILES}u;")));
        assert_eq!((slice_of(0.0), slice_of(0.25), slice_of(0.5), slice_of(1.0)), (0, 0, 3, 6));
        assert_eq!(slice_of(1e9), SLICES - 1);
    }
}
