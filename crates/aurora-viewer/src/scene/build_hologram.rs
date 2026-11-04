//! The build tools' holograms, in the loading effect's HOLOGRAM mode:
//! - copies of the selection's own faces where a click would put it
//!   ("Place on surface", build/place.rs) or where a move began
//!   (`MoveHologram::Start`), drawn through the creation hologram's pass
//!   (creation_preview.rs) with copies of their draw records: no geometry is
//!   uploaded, and an unchanged record is not sent again;
//! - the prims a move drags, shown as a hologram themselves
//!   (`MoveHologram::Moving`): their own records take the mode until the
//!   move ends, then get the rez effect's value back (or none).
//!
//! Rigged and worn faces (placed by the GPU) are left out.

use super::Scene;
use crate::build::Bounds;
use crate::build::place::Landing;
use aurora_render::{DrawCmd, DrawLists, Renderer, flags, rez_mode};
use glam::{Mat4, Vec3};
use std::time::Instant;

/// Faces drawn at most (a large linkset shows in part).
const MAX_FACES: usize = 4096;

/// Where copies of the faces go: `placement` applied to them, the effect's
/// gradient running from `bottom` up `height` metres.
#[derive(Debug, Clone, Copy)]
pub struct Ghost {
    pub placement: Mat4,
    pub bottom: f32,
    pub height: f32,
}

impl Ghost {
    /// Where "Place on surface" would put the selection: turned about its
    /// box's centre, then moved to where it lands.
    pub fn landing(l: &Landing) -> Self {
        let placement = Mat4::from_translation(l.to.center) * Mat4::from_quat(l.turn) * Mat4::from_translation(-l.from.center);
        let (bottom, height) = span(&l.to);
        Self { placement, bottom, height }
    }

    /// Where a move began: the selection `delta` back, its box then `start`.
    pub fn start(start: &Bounds, delta: Vec3) -> Self {
        let (bottom, height) = span(start);
        Self {
            placement: Mat4::from_translation(-delta),
            bottom,
            height,
        }
    }
}

/// The lowest point of a box and its height.
pub fn span(b: &Bounds) -> (f32, f32) {
    let axes = glam::Mat3::from_quat(b.rotation);
    let lift = Vec3::new(axes.x_axis.z, axes.y_axis.z, axes.z_axis.z).abs().dot(b.half);
    (b.center.z - lift, lift * 2.0)
}

fn hologram(start: u32, bottom: f32, height: f32) -> [u32; 4] {
    [start, rez_mode::HOLOGRAM | (3 << 20), bottom.to_bits(), height.to_bits()]
}

/// Faces the GPU places itself: not ours to move or to change.
fn placed_by_gpu(flags0: u32) -> bool {
    flags0 & (flags::SKINNED | flags::ANCHORED) != 0
}

#[derive(Default)]
pub struct BuildHologram {
    records: Vec<u32>,
    cmds: Vec<DrawCmd>,
    start: Option<u32>,
    /// The prims shown as a hologram in place, and since when.
    held: Vec<usize>,
    held_start: Option<u32>,
}

impl BuildHologram {
    /// Frees the copies past the first `keep`.
    fn release(&mut self, renderer: &mut Renderer, keep: usize) {
        for record in self.records.drain(keep.min(self.records.len())..) {
            renderer.records.free(record);
        }
    }

    pub fn append(&self, lists: &mut DrawLists) {
        lists.creation_preview.extend_from_slice(&self.cmds);
    }

    /// On logout: the records go with the scene's.
    pub fn clear(&mut self, renderer: &mut Renderer) {
        self.release(renderer, 0);
        self.cmds.clear();
        self.start = None;
        self.held.clear();
        self.held_start = None;
    }
}

impl Scene {
    /// This frame's holograms: copies of `copies`' prims at their ghost,
    /// and the prims of `in_place` (with their box now) drawn as one.
    pub fn update_build_hologram(
        &mut self,
        renderer: &mut Renderer,
        copies: Option<(&[usize], Ghost)>,
        in_place: Option<(&[usize], &Bounds)>,
    ) {
        let mut h = std::mem::take(&mut self.build_hologram);
        self.copies(renderer, &mut h, copies);
        self.in_place(renderer, &mut h, in_place);
        self.build_hologram = h;
    }

    fn copies(&self, renderer: &mut Renderer, h: &mut BuildHologram, copies: Option<(&[usize], Ghost)>) {
        h.cmds.clear();
        let Some((prims, ghost)) = copies else {
            h.release(renderer, 0);
            h.start = None;
            return;
        };
        let start = *h.start.get_or_insert_with(|| renderer.anim_clock().ms(Instant::now()));
        let rez = hologram(start, ghost.bottom, ghost.height);
        let mut n = 0;
        'prims: for &idx in prims {
            let Some(g) = self.gpu.get(idx) else { continue };
            for f in &g.faces {
                if n >= MAX_FACES {
                    break 'prims;
                }
                let Some(mut r) = renderer.records.get(f.record).copied() else {
                    continue;
                };
                if placed_by_gpu(r.flags[0]) {
                    continue;
                }
                r.model = (ghost.placement * Mat4::from_cols_array_2d(&r.model)).to_cols_array_2d();
                r.flags[0] &= !(flags::HIGHLIGHT | flags::PORTAL_CLIP);
                r.rez = rez;
                let record = if let Some(&record) = h.records.get(n) {
                    renderer.records.set_if_changed(record, r);
                    record
                } else {
                    let record = renderer.records.alloc(r);
                    h.records.push(record);
                    record
                };
                h.cmds.push(DrawCmd {
                    record,
                    bounds: [0.0; 4],
                    ..f.cmd
                });
                n += 1;
            }
        }
        h.release(renderer, n);
    }

    fn in_place(&self, renderer: &mut Renderer, h: &mut BuildHologram, in_place: Option<(&[usize], &Bounds)>) {
        let prims = in_place.map_or(&[][..], |(prims, _)| prims);
        // the prims let go get the rez effect's value back
        for idx in std::mem::take(&mut h.held) {
            if prims.contains(&idx) {
                continue;
            }
            let value = self.rez.value_of(idx).map_or([0; 4], |(v, _)| v);
            for f in self.gpu.get(idx).map_or(&[][..], |g| &g.faces[..]) {
                renderer.records.set_rez(f.record, value);
            }
        }
        let Some((_, now)) = in_place else {
            h.held_start = None;
            return;
        };
        let start = *h.held_start.get_or_insert_with(|| renderer.anim_clock().ms(Instant::now()));
        let (bottom, height) = span(now);
        let rez = hologram(start, bottom, height);
        // (every frame: a rebuilt face gets new records)
        for &idx in prims {
            for f in self.gpu.get(idx).map_or(&[][..], |g| &g.faces[..]) {
                if renderer.records.get(f.record).is_some_and(|r| !placed_by_gpu(r.flags[0])) {
                    renderer.records.set_rez(f.record, rez);
                }
            }
        }
        h.held = prims.to_vec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn the_place_hologram_sits_where_the_box_lands() {
        let from = Bounds {
            center: Vec3::new(10.0, 10.0, 25.0),
            rotation: Quat::IDENTITY,
            half: Vec3::new(0.5, 0.2, 0.3),
        };
        let turn = Quat::from_rotation_z(1.0);
        let l = crate::build::place::landing(&from, Vec3::new(3.0, 4.0, 20.0), Vec3::Z, turn, 0.0);
        let m = Ghost::landing(&l).placement;
        // the box's centre and a corner go where the landing puts them
        assert!(m.transform_point3(from.center).abs_diff_eq(l.to.center, 1e-5));
        let corner = from.center + from.half;
        assert!(m.transform_point3(corner).abs_diff_eq(l.apply(corner), 1e-5));
        assert!(
            m.transform_point3(corner)
                .abs_diff_eq(l.to.center + l.to.rotation * from.half, 1e-5)
        );
    }

    #[test]
    fn the_start_hologram_stays_where_the_move_began() {
        let start = Bounds {
            center: Vec3::new(10.0, 10.0, 25.0),
            rotation: Quat::from_rotation_x(0.5),
            half: Vec3::new(0.5, 0.2, 0.3),
        };
        let delta = Vec3::new(1.0, -2.0, 0.5);
        let g = Ghost::start(&start, delta);
        // a point of the moved selection is drawn back where it was
        let corner = start.center + start.rotation * start.half;
        assert!(g.placement.transform_point3(corner + delta).abs_diff_eq(corner, 1e-5));
        // the gradient runs up the box it was
        let (bottom, height) = span(&start);
        assert!((bottom + height * 0.5 - start.center.z).abs() < 1e-5);
        assert_eq!((g.bottom, g.height), (bottom, height));
        assert!(height > 0.6 && height < 2.0 * (0.2f32.hypot(0.3) + 1e-3));
    }
}
