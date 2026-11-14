//! Where the particles come among the blended faces.
//!
//! The Legacy Viewer draws the particles as ordinary alpha spatial groups
//! (LLParticlePartition::mRenderPass = PASS_ALPHA, llvopartgroup.cpp):
//! sorted with the blended faces' groups by a view depth
//! (LLSpatialPartition::calcDistance, llspatialpartition.cpp;
//! LLPipeline::stateSort, pipeline.cpp), the particles of a group back to
//! front by view depth (LLParticlePartition::addGeometryCount,
//! llvopartgroup.cpp), and split around the water like the faces
//! (LLDrawPoolAlpha::renderAlpha, lldrawpoolalpha.cpp). A plume behind a
//! window is therefore painted under the glass.
//!
//! Here every particle has its own view depth key, in the space of the
//! faces' keys (`blend_order::BlendEntry::distance`), which is finer than
//! the Legacy Viewer's group boxes: a plume crossing a pane is split exactly
//! at the pane. The particle list, sorted back to front by these keys, is
//! merged with the non-rigged blended faces by a linear two-pointer pass
//! (`interleave`) into `ParticleRun`s the renderer draws between the faces,
//! at most `MAX_RUNS` of them (a switch of pipeline each).
//!
//! `AURORA_PARTICLE_INTERLEAVE=0` (aurora_render::particle_interleave) gives
//! back the former order: one radial sort, all drawn after the faces.

use std::time::Instant;

use glam::Vec3;

use super::Scene;
use super::blend_order::BlendEntry;
use crate::frame_profile::Part;
use aurora_render::{DrawLists, ParticleInstance, ParticleRun, particle_flags};

/// Most runs of particles drawn before a blended face: each one is a switch
/// to the particle pipeline and back (about 0.75 us of recording each).
pub const MAX_RUNS: usize = 256;

/// What the keys measure, taken from the frame's blended-face sort so that
/// particles and faces share one space even under frozen culling.
#[derive(Clone, Copy, Debug)]
pub struct SortBasis {
    pub eye: Vec3,
    /// The view direction: depth along it, signed (a large particle beside
    /// the camera can sit behind the near plane). `None`: the radial
    /// distance, as `AURORA_ALPHA_FACE_SORT=0` sorts the faces.
    pub forward: Option<Vec3>,
}

impl SortBasis {
    /// Depth of a position; NaN counts as 0, as the key does.
    pub fn depth(&self, p: Vec3) -> f32 {
        let d = match self.forward {
            Some(f) => (p - self.eye).dot(f),
            None => p.distance(self.eye),
        };
        if d.is_nan() { 0.0 } else { d }
    }
}

/// Sort key of a particle by its signed `depth`: ascending keys go from the
/// farthest to the nearest, then by `index` (source order) so that two
/// particles at the same depth keep their order from a frame to the next.
/// The map of the float bits is order preserving for negative depths too.
pub fn far_first_signed(depth: f32, index: usize) -> u64 {
    let b = if depth.is_nan() { 0.0f32 } else { depth }.to_bits();
    let ordered = if b & 0x8000_0000 != 0 { !b } else { b | 0x8000_0000 };
    ((!ordered as u64) << 32) | index as u64
}

/// Memory the merge reuses from a frame to the next.
#[derive(Default)]
pub struct Scratch {
    gaps: Vec<(f32, u32)>,
    merge: Vec<bool>,
}

/// Merge the particles into the blended faces. `faces` are the depths of
/// the non-rigged blended faces in drawing order (non-increasing), `parts`
/// those of the particles in drawing order (non-increasing); `first_face` is
/// the index in the whole blended list of `faces[0]` (the runs' `before`
/// count from the start of that list). A particle goes before the first
/// face nearer than it; at the same depth the face comes first, the particle
/// over it; a face whose depth is not a number counts as depth 0. Past `cap`
/// runs before a face, the runs with the smallest depth gap between them are
/// merged, the merged run drawn at the later place (its farther particles
/// then over the faces in between, the error the whole list had before).
pub fn interleave(faces: &[f32], parts: &[f32], first_face: u32, cap: usize, scratch: &mut Scratch, out: &mut Vec<ParticleRun>) {
    debug_assert!(cap >= 1);
    let cap = cap.max(1);
    out.clear();
    let mut f = 0usize;
    for (i, &d) in parts.iter().enumerate() {
        while f < faces.len() && face_depth(faces[f]) >= d {
            f += 1;
        }
        let before = first_face + f as u32;
        match out.last_mut() {
            Some(r) if r.before == before => r.end = i as u32 + 1,
            _ => out.push(ParticleRun { before, end: i as u32 + 1 }),
        }
    }
    let after_last = first_face + faces.len() as u32;
    let inner = out.iter().filter(|r| r.before < after_last).count();
    if inner <= cap || out.len() < 2 {
        return;
    }
    let excess = inner - cap;
    // gap k: between run k's nearest particle and run k+1's farthest
    scratch.gaps.clear();
    scratch.gaps.extend(out.windows(2).enumerate().map(|(k, w)| {
        let a = parts[w[0].end as usize - 1];
        let b = parts[w[0].end as usize];
        ((a - b).max(0.0), k as u32)
    }));
    // the `excess` smallest gaps, ties by place (stable from frame to frame)
    let cmp = |x: &(f32, u32), y: &(f32, u32)| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1));
    scratch.gaps.select_nth_unstable_by(excess - 1, cmp);
    scratch.merge.clear();
    scratch.merge.resize(out.len(), false);
    for &(_, k) in &scratch.gaps[..excess] {
        scratch.merge[k as usize] = true;
    }
    let mut w = 0;
    for k in 0..out.len() {
        let r = out[k];
        if k > 0 && scratch.merge[k - 1] {
            // run k-1 joins run k: drawn at k's place, up to k's end
            out[w - 1] = r;
        } else {
            out[w] = r;
            w += 1;
        }
    }
    out.truncate(w);
}

/// A face's depth as the merge reads it: NaN counts as 0, as the particles'.
fn face_depth(d: f32) -> f32 {
    if d.is_nan() { 0.0 } else { d }
}

/// Depths of the particles, in list order.
pub fn depths(basis: &SortBasis, parts: &[ParticleInstance], out: &mut Vec<f32>) {
    out.clear();
    out.extend(parts.iter().map(|p| basis.depth(Vec3::from_array(p.pos))));
}

/// Radius of the sphere around a particle's quad (for the side of the
/// water it is drawn on): half the diagonal of
/// a billboard, half the diagonal of a ribbon's width and length (a ribbon's
/// `size[1]` is not used: its length is its axis, as `vs_particle` has it).
pub fn radius(p: &ParticleInstance) -> f32 {
    let along = if p.flags & particle_flags::AXIS != 0 {
        Vec3::from_array(p.axis).length()
    } else {
        p.size[1]
    };
    (0.5 * p.size[0].hypot(along)).max(0.01)
}

/// Split `parts` (sorted back to front) around the water surface at height
/// `plane`, the camera being above it: those reaching above it stay in
/// `parts`, those reaching below it go to `under`, a particle crossing it
/// is in both (`blend_order::water_sides`), and how many are. Both keep their order.
pub fn split_water(parts: &mut Vec<ParticleInstance>, under: &mut Vec<ParticleInstance>, plane: f32) -> usize {
    under.clear();
    let mut crossing = 0;
    let mut w = 0;
    for i in 0..parts.len() {
        let p = parts[i];
        let (after, before) = super::blend_order::water_sides(p.pos[2], radius(&p), Some(plane));
        if before {
            under.push(p);
        }
        if after {
            parts[w] = p;
            w += 1;
        }
        crossing += usize::from(before && after);
    }
    parts.truncate(w);
    crossing
}

// ------------------------------------------------------------- placement

/// What the frame's blended-face sort gave, for the particles.
#[derive(Clone, Copy, Debug)]
pub struct AlphaView {
    pub basis: SortBasis,
    /// The water height the lists were split at (the camera above it).
    pub plane: Option<f32>,
}

/// Memory `place_particles` reuses from a frame to the next.
#[derive(Default)]
pub struct Work {
    scratch: Scratch,
    depths: Vec<f32>,
    face_depths: Vec<f32>,
}

/// The runs of `parts` among `faces` (the non-rigged tail of a sorted blended
/// list, `first` its index in the whole list), by view depth.
fn place_list(view: &AlphaView, faces: &[BlendEntry], first: u32, parts: &[ParticleInstance], work: &mut Work, out: &mut Vec<ParticleRun>) {
    work.face_depths.clear();
    work.face_depths.extend(faces.iter().map(|e| e.distance));
    depths(&view.basis, parts, &mut work.depths);
    interleave(&work.face_depths, &work.depths, first, MAX_RUNS, &mut work.scratch, out);
}

/// Place the sorted particles of `lists` among the blended faces
/// (`DrawLists::particle_runs`), and split them at the water like the faces
/// (`particles_under`, a crossing one in both and counted in
/// `particle_crossing`). `blend` is the frame's sorted blended list whose
/// first `blend_depth` entries are drawn apart (rigged), `under` the one
/// under the water with `under_rigged` of those. Without `interleave`, or
/// without a view, everything is left as it was: no runs, nothing split.
pub fn place_particles(
    view: Option<&AlphaView>,
    interleave: bool,
    blend: &[BlendEntry],
    blend_depth: usize,
    under: &[BlendEntry],
    under_rigged: usize,
    lists: &mut DrawLists,
    work: &mut Work,
) {
    lists.particle_runs.clear();
    lists.particles_under.clear();
    lists.particle_under_runs.clear();
    lists.particle_crossing = 0;
    let Some(view) = view.filter(|_| interleave) else {
        return;
    };
    if lists.particles.is_empty() {
        return;
    }
    if let Some(h) = view.plane {
        lists.particle_crossing = split_water(&mut lists.particles, &mut lists.particles_under, h) as u32;
    }
    // (the lists of the faces were made by this frame's `build_lists`)
    let first = blend_depth.min(blend.len());
    place_list(
        view,
        &blend[first..],
        first as u32,
        &lists.particles,
        work,
        &mut lists.particle_runs,
    );
    if !lists.particles_under.is_empty() {
        let rigged = under_rigged.min(under.len());
        place_list(
            view,
            &under[rigged..],
            rigged as u32,
            &lists.particles_under,
            work,
            &mut lists.particle_under_runs,
        );
    }
}

impl Scene {
    /// What this frame's particles are sorted by: the basis of the blended
    /// faces' sort, so that both share one space. Without the interleave, or
    /// before the first list of the faces, the radial distance from the
    /// camera, as before.
    pub fn particle_basis(&self, camera: Vec3) -> SortBasis {
        match self.alpha_view {
            Some(view) if aurora_render::particle_interleave() => view.basis,
            _ => SortBasis {
                eye: camera,
                forward: None,
            },
        }
    }

    /// Place the sorted `lists.particles` (`place_particles`). Run after the
    /// particle update and the Hide-particles clear, and after `build_lists`
    /// of the frame.
    pub fn order_particles(&mut self) {
        let t = Instant::now();
        // `Exact` lists the faces nearest first and draws every particle after
        // them (the exact blend order has no particles yet): no faces to
        // place them among, one trailing run
        let faces: &[BlendEntry] = if self.lists.blend_depth_mode == aurora_render::RiggedBlendDepth::Exact {
            &[]
        } else {
            &self.blend_sorted
        };
        place_particles(
            self.alpha_view.as_ref(),
            aurora_render::particle_interleave(),
            faces,
            self.lists.blend_depth,
            &self.blend_under_sorted,
            self.blend_under_rigged,
            &mut self.lists,
            &mut self.particle_work,
        );
        self.parts.lap(Part::ParamsPartOrder, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(before: u32, end: u32) -> ParticleRun {
        ParticleRun { before, end }
    }

    fn merged(faces: &[f32], parts: &[f32], first: u32, cap: usize) -> Vec<ParticleRun> {
        let mut out = Vec::new();
        interleave(faces, parts, first, cap, &mut Scratch::default(), &mut out);
        out
    }

    #[test]
    fn a_tie_puts_the_face_first_and_the_particle_over_it() {
        assert_eq!(merged(&[10.0, 5.0], &[5.0], 0, 256), [run(2, 1)]);
    }

    #[test]
    fn no_faces_is_one_trailing_run_and_no_particles_none() {
        assert_eq!(merged(&[], &[3.0, 2.0], 0, 256), [run(0, 2)]);
        assert!(merged(&[3.0], &[], 0, 256).is_empty());
    }

    #[test]
    fn particles_nearer_than_every_face_are_one_trailing_run() {
        assert_eq!(merged(&[9.0, 8.0], &[3.0, 2.0, 1.0], 0, 256), [run(2, 3)]);
    }

    #[test]
    fn particles_farther_than_every_face_are_one_run_before_the_first() {
        assert_eq!(merged(&[9.0, 8.0], &[30.0, 20.0], 0, 256), [run(0, 2)]);
    }

    #[test]
    fn particles_fall_between_the_faces_they_lie_between() {
        // 10 | 8 7.5 | 6 | 2
        assert_eq!(
            merged(&[9.0, 7.0, 5.0, 3.0], &[10.0, 8.0, 7.5, 6.0, 2.0], 0, 256),
            [run(0, 1), run(1, 3), run(2, 4), run(4, 5)]
        );
    }

    #[test]
    fn first_face_offsets_the_runs() {
        assert_eq!(merged(&[9.0, 7.0], &[8.0], 5, 256), [run(6, 1)]);
        assert_eq!(merged(&[9.0, 7.0], &[1.0], 5, 256), [run(7, 1)]);
    }

    /// `n` faces at 10, 20 .. n * 10 (drawn far first), a particle 5 above
    /// each (so before it) and one nearer than all: n inner runs and the
    /// trailing one.
    fn comb(n: usize) -> (Vec<f32>, Vec<f32>) {
        let faces: Vec<f32> = (0..n).map(|i| (n - i) as f32 * 10.0).collect();
        let mut parts: Vec<f32> = faces.iter().map(|d| d + 5.0).collect();
        parts.push(-1.0);
        (faces, parts)
    }

    #[test]
    fn the_cap_is_kept_exactly_and_by_one() {
        let (faces, parts) = comb(300);
        let inner = |out: &[ParticleRun]| out.iter().filter(|r| r.before < 300).count();
        assert_eq!(inner(&merged(&faces, &parts, 0, 300)), 300);
        assert_eq!(inner(&merged(&faces, &parts, 0, 299)), 299);
        let out = merged(&faces, &parts, 0, 100);
        assert_eq!(inner(&out), 100);
        assert_eq!(out.last().map(|r| r.end as usize), Some(parts.len()));
        assert!(out.windows(2).all(|w| w[0].before < w[1].before && w[0].end < w[1].end));
    }

    #[test]
    fn the_smallest_gap_is_merged_at_the_later_place() {
        // faces 9 7 5 3; runs: before0 (10), before1 (8, 7.5), before2 (6), before4 (2)
        // gaps: 10-8 = 2, 7.5-6 = 1.5, 6-2 = 4: with one inner run less, the
        // 1.5 gap goes (runs 1 and 2 join, drawn before face 2)
        let out = merged(&[9.0, 7.0, 5.0, 3.0], &[10.0, 8.0, 7.5, 6.0, 2.0], 0, 2);
        assert_eq!(out, [run(0, 1), run(2, 4), run(4, 5)]);
        // two less: the 2 gap too
        let out = merged(&[9.0, 7.0, 5.0, 3.0], &[10.0, 8.0, 7.5, 6.0, 2.0], 0, 1);
        assert_eq!(out, [run(2, 4), run(4, 5)]);
    }

    #[test]
    fn equal_gaps_merge_the_same_runs_every_frame() {
        let faces: Vec<f32> = (0..8).map(|i| 80.0 - i as f32 * 10.0 - 0.5).collect();
        let parts: Vec<f32> = (0..9).map(|i| 80.0 - i as f32 * 10.0).collect();
        let a = merged(&faces, &parts, 0, 4);
        let b = merged(&faces, &parts, 0, 4);
        assert_eq!(a, b);
        assert_eq!(a.iter().filter(|r| r.before < 8).count(), 4);
        // the earliest places go first on equal gaps
        assert_eq!(a[0].before, 4);
    }

    #[test]
    fn the_last_inner_run_may_join_the_trailing_one() {
        // runs before face 0 (10 m), before face 1 (8 m) and after it (6.9 m):
        // the smallest gap is the last, the middle run joins the trailing one
        let out = merged(&[9.0, 7.0], &[10.0, 8.0, 6.9], 0, 1);
        assert_eq!(out, [run(0, 1), run(2, 3)]);
    }

    #[test]
    fn radial_depths_merge_like_view_depths() {
        let basis = SortBasis {
            eye: Vec3::ZERO,
            forward: None,
        };
        let parts: Vec<f32> = [Vec3::new(20.0, 0.0, 0.0), Vec3::new(0.0, 6.0, 0.0)]
            .iter()
            .map(|p| basis.depth(*p))
            .collect();
        assert_eq!(merged(&[10.0], &parts, 0, 256), [run(0, 1), run(1, 2)]);
    }

    #[test]
    fn view_depth_is_signed_along_the_view() {
        let basis = SortBasis {
            eye: Vec3::new(1.0, 0.0, 0.0),
            forward: Some(Vec3::X),
        };
        assert_eq!(basis.depth(Vec3::new(4.0, 9.0, 9.0)), 3.0);
        assert_eq!(basis.depth(Vec3::new(-1.0, 0.0, 0.0)), -2.0);
        assert_eq!(basis.depth(Vec3::new(f32::NAN, 0.0, 0.0)), 0.0);
    }

    #[test]
    fn signed_keys_order_far_to_near_across_zero() {
        let mut keys: Vec<u64> = [-3.0f32, 5.0, 0.0, -0.5, 12.0, 0.5, f32::NAN]
            .iter()
            .enumerate()
            .map(|(i, d)| far_first_signed(*d, i))
            .collect();
        keys.sort_unstable();
        let order: Vec<usize> = keys.iter().map(|k| (k & 0xFFFF_FFFF) as usize).collect();
        // 12, 5, 0.5, then 0 and NaN (both 0, source order), -0.5, -3
        assert_eq!(order, [4, 1, 5, 2, 6, 3, 0]);
    }

    #[test]
    fn equal_depths_keep_the_source_order() {
        assert!(far_first_signed(2.0, 3) < far_first_signed(2.0, 4));
    }

    fn at(z: f32, size: f32) -> ParticleInstance {
        ParticleInstance {
            pos: [0.0, 0.0, z],
            size: [size, size],
            ..Default::default()
        }
    }

    #[test]
    fn the_water_split_sends_each_particle_to_its_side_or_both() {
        // radius of a 2 m billboard: 0.5 * hypot(2, 2) = 1.41
        let mut parts = vec![at(30.0, 2.0), at(20.5, 2.0), at(19.0, 2.0), at(10.0, 2.0)];
        let mut under = Vec::new();
        split_water(&mut parts, &mut under, 20.0);
        let z = |v: &Vec<ParticleInstance>| v.iter().map(|p| p.pos[2]).collect::<Vec<_>>();
        assert_eq!(z(&parts), [30.0, 20.5, 19.0]);
        assert_eq!(z(&under), [20.5, 19.0, 10.0]);
    }

    #[test]
    fn a_ribbon_reaches_by_half_its_length() {
        let mut ribbon = at(25.0, 0.2);
        ribbon.flags = particle_flags::AXIS;
        ribbon.axis = [0.0, 0.0, 12.0];
        assert!((radius(&ribbon) - 0.5 * 0.2f32.hypot(12.0)).abs() < 1e-6);
        let (mut parts, mut under) = (vec![ribbon], Vec::new());
        split_water(&mut parts, &mut under, 20.0);
        assert_eq!((parts.len(), under.len()), (1, 1), "a ribbon crossing the surface is in both");
    }

    // -------------------------------------------------------- placement

    fn view(plane: Option<f32>) -> AlphaView {
        AlphaView {
            basis: SortBasis {
                eye: Vec3::ZERO,
                forward: Some(Vec3::X),
            },
            plane,
        }
    }

    /// A blended face at view depth `depth`.
    fn face_at(depth: f32) -> BlendEntry {
        BlendEntry {
            distance: depth,
            cmd: aurora_render::DrawCmd {
                index_count: 3,
                first_index: 0,
                base_vertex: 0,
                record: 0,
                bounds: [0.0; 4],
            },
            glow: false,
            double_sided: false,
            rigged: false,
            object: 0,
            face: 0,
            rig: None,
        }
    }

    fn particle_at(depth: f32, side: f32) -> ParticleInstance {
        ParticleInstance {
            pos: [depth, side * depth * 0.4, 0.0],
            size: [1.0, 1.0],
            ..Default::default()
        }
    }

    fn placed(
        v: Option<&AlphaView>,
        on: bool,
        blend: &[BlendEntry],
        depth: usize,
        under: &[BlendEntry],
        rigged: usize,
        lists: &mut DrawLists,
    ) {
        place_particles(v, on, blend, depth, under, rigged, lists, &mut Work::default());
    }

    #[test]
    fn without_the_interleave_nothing_is_placed_or_split() {
        let mut lists = DrawLists {
            particles: vec![particle_at(30.0, 0.0), particle_at(10.0, 0.0)],
            ..Default::default()
        };
        lists.particle_runs.push(run(1, 1));
        lists.particles_under.push(particle_at(1.0, 0.0));
        let before = lists.particles.clone();
        let v = view(Some(0.0));
        placed(Some(&v), false, &[face_at(20.0)], 0, &[], 0, &mut lists);
        assert!(lists.particle_runs.is_empty() && lists.particles_under.is_empty() && lists.particle_under_runs.is_empty());
        assert_eq!(lists.particles.len(), before.len());
        assert!(lists.particles.iter().zip(&before).all(|(a, b)| a.pos == b.pos));
        // and without a view (before the first list of the faces)
        placed(None, true, &[], 0, &[], 0, &mut lists);
        assert!(lists.particle_runs.is_empty());
    }

    #[test]
    fn the_rigged_prefix_offsets_the_runs_by_its_length() {
        // two rigged faces drawn apart, then faces at 30 and 10
        let blend = [face_at(50.0), face_at(40.0), face_at(30.0), face_at(10.0)];
        let mut lists = DrawLists {
            particles: vec![particle_at(20.0, 0.0)],
            ..Default::default()
        };
        let v = view(None);
        placed(Some(&v), true, &blend, 2, &[], 0, &mut lists);
        assert_eq!(lists.particle_runs, [run(3, 1)]);
        // farther than the first non-rigged face: before it
        lists.particles = vec![particle_at(35.0, 0.0)];
        placed(Some(&v), true, &blend, 2, &[], 0, &mut lists);
        assert_eq!(lists.particle_runs, [run(2, 1)]);
    }

    #[test]
    fn a_particle_crossing_the_water_is_in_both_lists_and_counted_once() {
        let at = |depth: f32, z: f32| ParticleInstance {
            pos: [depth, 0.0, z],
            size: [1.0, 1.0],
            ..Default::default()
        };
        let mut lists = DrawLists {
            particles: vec![at(30.0, 30.0), at(20.0, 20.2), at(10.0, 10.0)],
            ..Default::default()
        };
        let v = view(Some(20.0));
        let under = [face_at(25.0), face_at(5.0)];
        placed(Some(&v), true, &[face_at(15.0)], 0, &under, 0, &mut lists);
        let z = |l: &[ParticleInstance]| l.iter().map(|p| p.pos[2]).collect::<Vec<_>>();
        assert_eq!(z(&lists.particles), [30.0, 20.2]);
        assert_eq!(z(&lists.particles_under), [20.2, 10.0]);
        assert_eq!(lists.particle_crossing, 1);
        // each list against its own faces: above, both before the face at
        // 15 m; under, the 20 m one after the face at 25 m and before the
        // one at 5 m, the 10 m one before the one at 5 m too
        assert_eq!(lists.particle_runs, [run(0, 2)]);
        assert_eq!(lists.particle_under_runs, [run(1, 2)]);
    }

    #[test]
    fn a_face_that_is_not_a_number_counts_as_depth_zero() {
        // as depth 0 (not as "nearer than everything"): a particle at 3 is
        // not past it, one at -1 is
        assert_eq!(merged(&[f32::NAN, 5.0], &[3.0, -1.0], 0, 256), [run(0, 1), run(2, 2)]);
    }

    #[test]
    fn a_cap_of_one_run_keeps_one_run_before_the_faces() {
        let out = merged(&[9.0, 7.0], &[10.0, 8.0, 1.0], 0, 1);
        assert_eq!(out.last().map(|r| r.end), Some(3));
        assert!(out.iter().filter(|r| r.before < 2).count() <= 1);
    }
}
