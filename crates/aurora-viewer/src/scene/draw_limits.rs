//! How far from its object an animated or worn object is drawn, as the
//! Legacy Viewer limits it.
//!
//! - An animesh's figure stays within 3 m of its object and at most 64 m
//!   across: an animation that carries the pelvis away, or scales it up,
//!   is pulled back or shrunk. Port of LLControlAvatar::getNewConstraintFixups
//!   and matchVolumeTransform (indra/newview/llcontrolavatar.cpp, settings
//!   `AnimatedObjectsMaxLegalOffset` and `AnimatedObjectsMaxLegalSize`),
//!   originally LGPL 2.1. The fixups are worked out at each frame from the
//!   animated box (`anim_box.rs`, the port of calculateSpatialExtents),
//!   held in `World::animesh_fixups` and applied by
//!   `anchors::skeleton_model`, which every rigged draw, the culling and
//!   the GPU picking place the skeleton with.
//! - The root of every attachment, of every avatar, is drawn at most 3.5 m
//!   from its attachment point. Port of
//!   LLViewerJointAttachment::clampObjectPosition
//!   (indra/newview/llviewerjointattachment.cpp), which
//!   LLVOAvatar::clampAttachmentPositions runs at each update of an
//!   attachment (LLViewerObject::processUpdateMessage). The Legacy Viewer
//!   writes the clamped offset into the object; Aurora keeps the offset the
//!   simulator sent and clamps where the attachment's transform is composed
//!   (`Scene::object_transform`, `anchors::anchored_or_moving`), so the
//!   draw, the picking and the selection see the same place whichever of
//!   the attachment and its wearer arrived first.

use super::anim_box::AnimBox;
use glam::Vec3;

pub use crate::build::MAX_ATTACHMENT_DIST;

/// `AnimatedObjectsMaxLegalOffset` (LLControlAvatar::MAX_LEGAL_OFFSET):
/// how far an animesh's box may be from its object.
pub const MAX_LEGAL_OFFSET: f32 = 3.0;

/// `AnimatedObjectsMaxLegalSize` (LLControlAvatar::MAX_LEGAL_SIZE): the
/// largest side of an animesh's box, unscaled.
pub const MAX_LEGAL_SIZE: f32 = 64.0;

/// The offset of an attachment root from its attachment point, as drawn.
pub fn attachment_offset(position: Vec3) -> Vec3 {
    position.clamp_length_max(MAX_ATTACHMENT_DIST)
}

/// From `pos` to the nearest point of the box: zero inside it or on it
/// (point_to_box_offset, indra/llmath/v3math.cpp).
pub fn point_to_box_offset(pos: Vec3, min: Vec3, max: Vec3) -> Vec3 {
    let axis = |p: f32, lo: f32, hi: f32| {
        if p < lo {
            p - lo
        } else if p > hi {
            p - hi
        } else {
            0.0
        }
    };
    Vec3::new(axis(pos.x, min.x, max.x), axis(pos.y, min.y, max.y), axis(pos.z, min.z, max.z))
}

/// What an animesh's skeleton is drawn with on top of its object's place
/// (mPositionConstraintFixup, mScaleConstraintFixup): a world translation
/// and a uniform scale about the pelvis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fixup {
    pub offset: Vec3,
    pub scale: f32,
}

impl Default for Fixup {
    fn default() -> Self {
        Self::NONE
    }
}

impl Fixup {
    pub const NONE: Fixup = Fixup {
        offset: Vec3::ZERO,
        scale: 1.0,
    };

    /// The fixups of the next frame, from the box `drawn` the skeleton
    /// filled with `self` applied, and its object at `object` (render
    /// space). A worn animesh is only scaled: it follows its attachment
    /// point, which the attachment limit holds. getNewConstraintFixups: no
    /// smoothing, a fixup is recomputed from scratch at each frame (the
    /// current one taken out of the box first), and is none while the box is
    /// unknown.
    pub fn next(self, object: Vec3, drawn: AnimBox, worn: bool) -> Fixup {
        let mut next = Fixup::NONE;
        // box_valid_and_non_zero
        if !drawn.min.is_finite() || !drawn.max.is_finite() || (drawn.min == Vec3::ZERO && drawn.max == Vec3::ZERO) {
            return next;
        }
        if !worn {
            let offset = point_to_box_offset(object, drawn.min - self.offset, drawn.max - self.offset);
            let dist = offset.length();
            if dist > MAX_LEGAL_OFFSET {
                next.offset = offset * ((dist - MAX_LEGAL_OFFSET) / dist);
            }
        }
        let size = (drawn.max - drawn.min).max_element();
        if size / self.scale > MAX_LEGAL_SIZE {
            next.scale = self.scale * MAX_LEGAL_SIZE / size;
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(min: [f32; 3], max: [f32; 3]) -> AnimBox {
        AnimBox {
            min: Vec3::from(min),
            max: Vec3::from(max),
        }
    }

    #[test]
    fn a_figure_within_the_limits_is_left_alone() {
        let object = Vec3::new(10.0, 10.0, 20.0);
        // around its object, then 2.9 m beside it: no fixup
        for b in [
            boxed([9.5, 9.5, 20.0], [10.5, 10.5, 22.0]),
            boxed([12.9, 9.5, 20.0], [13.5, 10.5, 22.0]),
        ] {
            assert_eq!(Fixup::NONE.next(object, b, false), Fixup::NONE);
        }
        // 60 m tall is under the size limit
        assert_eq!(
            Fixup::NONE.next(object, boxed([9.0, 9.0, 20.0], [11.0, 11.0, 80.0]), false),
            Fixup::NONE
        );
        // an unknown box resets the fixups
        let f = Fixup {
            offset: Vec3::X,
            scale: 0.5,
        };
        assert_eq!(f.next(object, boxed([0.0; 3], [0.0; 3]), false), Fixup::NONE);
        assert_eq!(f.next(object, boxed([f32::NAN; 3], [1.0; 3]), false), Fixup::NONE);
    }

    #[test]
    fn a_figure_carried_beyond_three_metres_is_pulled_back() {
        let object = Vec3::new(10.0, 10.0, 20.0);
        // the animation took the figure 8 m east of its object: the box's
        // nearest point is 7.5 m away, brought back to 3 m
        let away = boxed([17.5, 9.5, 20.0], [18.5, 10.5, 22.0]);
        let f = Fixup::NONE.next(object, away, false);
        assert!(f.offset.abs_diff_eq(Vec3::new(-4.5, 0.0, 0.0), 1e-5), "{f:?}");
        assert_eq!(f.scale, 1.0);
        // drawn with it next frame, the box keeps the same fixup (no drift)
        let drawn = boxed((away.min + f.offset).into(), (away.max + f.offset).into());
        let again = f.next(object, drawn, false);
        assert!(again.offset.abs_diff_eq(f.offset, 1e-5));
        let near = point_to_box_offset(object, drawn.min, drawn.max).length();
        assert!((near - MAX_LEGAL_OFFSET).abs() < 1e-4);
        // diagonally too: the distance is to the box's nearest corner
        let corner = boxed([14.0, 14.0, 25.0], [15.0, 15.0, 26.0]);
        let f = Fixup::NONE.next(object, corner, false);
        let gap = point_to_box_offset(object, corner.min + f.offset, corner.max + f.offset);
        assert!((gap.length() - MAX_LEGAL_OFFSET).abs() < 1e-4);
        // worn: the attachment point holds it, only the size is limited
        assert_eq!(Fixup::NONE.next(object, away, true), Fixup::NONE);
    }

    #[test]
    fn a_figure_wider_than_sixty_four_metres_is_scaled_down() {
        let object = Vec3::new(10.0, 10.0, 20.0);
        // 128 m tall: half its size
        let tall = boxed([9.0, 9.0, 20.0], [11.0, 11.0, 148.0]);
        let f = Fixup::NONE.next(object, tall, false);
        assert!((f.scale - 0.5).abs() < 1e-6);
        assert_eq!(f.offset, Vec3::ZERO);
        // drawn at half size next frame, it keeps that scale
        let half = boxed([9.5, 9.5, 20.0], [10.5, 10.5, 84.0]);
        assert!((f.next(object, half, false).scale - 0.5).abs() < 1e-6);
        // the same when worn
        assert!((Fixup::NONE.next(object, tall, true).scale - 0.5).abs() < 1e-6);
    }

    #[test]
    fn an_attachment_root_is_drawn_at_most_three_and_a_half_metres_from_its_point() {
        let far = attachment_offset(Vec3::new(3.0, 4.0, 0.0));
        assert!(far.abs_diff_eq(Vec3::new(2.1, 2.8, 0.0), 1e-5), "{far:?}");
        assert!((far.length() - MAX_ATTACHMENT_DIST).abs() < 1e-5);
        for near in [Vec3::ZERO, Vec3::new(0.15, 0.0, 0.2), Vec3::new(0.0, -3.5, 0.0)] {
            assert_eq!(attachment_offset(near), near);
        }
    }
}
