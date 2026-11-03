//! The hologram of "Place on surface" (build/place.rs): the selection's own
//! faces drawn again where a click would put it, in the loading effect's
//! HOLOGRAM mode, through the creation hologram's pass (creation_preview.rs).
//! Its records are copies of the faces' with the landing's placement: no
//! geometry is uploaded, and an unchanged record is not sent again.

use super::Scene;
use crate::build::place::Landing;
use aurora_render::{DrawCmd, DrawLists, Renderer, flags, rez_mode};
use glam::{Mat4, Vec3};
use std::time::Instant;

/// Faces drawn at most (a large linkset shows in part).
const MAX_FACES: usize = 4096;

#[derive(Default)]
pub struct PlacePreview {
    records: Vec<u32>,
    cmds: Vec<DrawCmd>,
    start: Option<u32>,
}

impl PlacePreview {
    /// Frees the records past the first `keep`.
    fn release(&mut self, renderer: &mut Renderer, keep: usize) {
        for record in self.records.drain(keep.min(self.records.len())..) {
            renderer.records.free(record);
        }
    }

    pub fn append(&self, lists: &mut DrawLists) {
        lists.creation_preview.extend_from_slice(&self.cmds);
    }

    pub fn clear(&mut self, renderer: &mut Renderer) {
        self.release(renderer, 0);
        self.cmds.clear();
        self.start = None;
    }
}

/// The landing's placement: the selection turned about its box's centre,
/// then moved to where it lands.
fn placement(l: &Landing) -> Mat4 {
    Mat4::from_translation(l.to.center) * Mat4::from_quat(l.turn) * Mat4::from_translation(-l.from.center)
}

impl Scene {
    /// The hologram of the prims `prims` at `landing`, or none.
    pub fn update_place_preview(&mut self, renderer: &mut Renderer, prims: &[usize], landing: Option<&Landing>) {
        let mut preview = std::mem::take(&mut self.place_preview);
        preview.cmds.clear();
        let Some(l) = landing else {
            preview.clear(renderer);
            self.place_preview = preview;
            return;
        };
        let start = *preview.start.get_or_insert_with(|| renderer.anim_clock().ms(Instant::now()));
        let m = placement(l);
        // the effect's gradient runs up the landed box
        let axes = glam::Mat3::from_quat(l.to.rotation);
        let lift = Vec3::new(axes.x_axis.z, axes.y_axis.z, axes.z_axis.z).abs().dot(l.to.half);
        let rez = [
            start,
            rez_mode::HOLOGRAM | (3 << 20),
            (l.to.center.z - lift).to_bits(),
            (lift * 2.0).to_bits(),
        ];
        let mut n = 0;
        'prims: for &idx in prims {
            let Some(g) = self.gpu.get(idx) else { continue };
            for f in &g.faces {
                if n >= MAX_FACES {
                    break 'prims;
                }
                // (rigged and worn faces are placed by the GPU: not ours)
                let Some(mut r) = renderer.records.get(f.record).copied() else {
                    continue;
                };
                if r.flags[0] & (flags::SKINNED | flags::ANCHORED) != 0 {
                    continue;
                }
                r.model = (m * Mat4::from_cols_array_2d(&r.model)).to_cols_array_2d();
                r.flags[0] &= !(flags::HIGHLIGHT | flags::PORTAL_CLIP);
                r.rez = rez;
                let record = if let Some(&record) = preview.records.get(n) {
                    renderer.records.set_if_changed(record, r);
                    record
                } else {
                    let record = renderer.records.alloc(r);
                    preview.records.push(record);
                    record
                };
                preview.cmds.push(DrawCmd {
                    record,
                    bounds: [0.0; 4],
                    ..f.cmd
                });
                n += 1;
            }
        }
        preview.release(renderer, n);
        self.place_preview = preview;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::Bounds;
    use glam::Quat;

    #[test]
    fn the_hologram_sits_where_the_box_lands() {
        let from = Bounds {
            center: Vec3::new(10.0, 10.0, 25.0),
            rotation: Quat::IDENTITY,
            half: Vec3::new(0.5, 0.2, 0.3),
        };
        let turn = Quat::from_rotation_z(1.0);
        let l = crate::build::place::landing(&from, Vec3::new(3.0, 4.0, 20.0), Vec3::Z, turn);
        let m = placement(&l);
        // the box's centre and a corner go where the landing puts them
        assert!(m.transform_point3(from.center).abs_diff_eq(l.to.center, 1e-5));
        let corner = from.center + from.half;
        assert!(m.transform_point3(corner).abs_diff_eq(l.apply(corner), 1e-5));
        assert!(
            m.transform_point3(corner)
                .abs_diff_eq(l.to.center + l.to.rotation * from.half, 1e-5)
        );
    }
}
