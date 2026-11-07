//! The detail level of a Linden tree, and whether trees and grass are drawn
//! at all: the Legacy Viewer's "Trees" slider (RenderTreeLODFactor).
//!
//! Port of LLVOTree::idleUpdate, LLVOTree::setPixelAreaAndAngle and
//! LLVOTree::isTreeRenderingStopped (indra/newview/llvotree.cpp), with
//! LLVolumeLODGroup::getDetailFromTan (indra/llmath/llvolume.cpp) and the
//! grass's stop (LLVOGrass::idleUpdate, llvograss.cpp), originally LGPL 2.1.
//!
//! A tree's trunk level is the coarser of two: the angle the tree spans,
//! times the factor, against 30°, 20° and 15°; and the tangent of its
//! billboard over its distance (shortened by the volume LOD factor, as
//! LLVOVolume::sDistanceFactor), times the factor, against the volume
//! thresholds. Level 4 is not drawn, which only a factor of 0 reaches; then
//! grass is not drawn either. Deliberate difference: the Legacy Viewer also
//! gives the finest level to a tree whose pixel area exceeds the screen's;
//! such a tree spans more than 30° at any factor of the slider, so the angle
//! already gives it.

use aurora_prim::tree::TreeSpecies;
use glam::Vec3;

/// LLVOTree::sMAX_NUM_TREE_LOD_LEVELS: trunk levels 0 (finest) to 3; 4 is
/// not drawn.
pub const HIDDEN_TRUNK: usize = 4;
/// LLVOTree::sLODAngles: the angles (degrees) of the trunk levels.
const ANGLES: [f32; 3] = [30.0, 20.0, 15.0];
/// F_ALMOST_ZERO: the last of sLODAngles, and the factor under which trees
/// and grass stop.
const ALMOST_ZERO: f32 = 0.0001;
/// The volume detail thresholds of getDetailFromTan, finest first.
const TANS: [f32; 3] = [0.24, 0.06, 0.03];
/// Distances closer than this to a level's change are checked again.
const MARGIN: f32 = 0.02;

/// What a tree's level is judged on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeSize {
    /// The largest side of the object's scale (getMaxScale).
    pub max_scale: f32,
    /// Half the length of its scale (setPixelAreaAndAngle's radius).
    pub radius: f32,
    /// Its billboard: scale x ratio x length of its scale (idleUpdate's sz).
    pub billboard: f32,
}

impl TreeSize {
    pub fn of(species: &TreeSpecies, scale: Vec3) -> TreeSize {
        let length = scale.length();
        TreeSize {
            max_scale: scale.max_element(),
            radius: length * 0.5,
            billboard: species.billboard_scale * species.billboard_ratio * length,
        }
    }
}

/// Trees and grass are not drawn (LLVOTree::isTreeRenderingStopped).
pub fn stopped(tree_factor: f32) -> bool {
    tree_factor < ALMOST_ZERO
}

/// LLVOVolume::sDistanceFactor times PI/3, as idleUpdate scales the distance.
fn distance_scale(lod_factor: f32) -> f32 {
    (1.0 - lod_factor * 0.1) * std::f32::consts::PI / 3.0
}

/// The trunk level (0 finest, `HIDDEN_TRUNK` not drawn) of a tree of `size`
/// at `distance` from the camera.
pub fn trunk_level(size: TreeSize, distance: f32, tree_factor: f32, lod_factor: f32) -> usize {
    let range = distance - size.radius;
    let angle = if range < ALMOST_ZERO {
        180.0
    } else {
        size.max_scale.atan2(range).to_degrees()
    };
    let app_angle = angle * tree_factor;
    let by_angle = ANGLES
        .iter()
        .chain(&[ALMOST_ZERO])
        .position(|a| app_angle > *a)
        .unwrap_or(HIDDEN_TRUNK);
    let scaled = distance * distance_scale(lod_factor);
    let round = |v: f32| (v * 100.0).round() / 100.0;
    let detail = if scaled <= 0.0 {
        3
    } else {
        let tan = round(tree_factor * 64.0 * size.billboard / scaled);
        3 - TANS.iter().position(|t| tan > *t).unwrap_or(3)
    };
    by_angle.max(3 - detail).min(HIDDEN_TRUNK)
}

/// How far the camera can move before the level of a tree at `distance`
/// may change: the distance to the nearest of the angle and tangent changes,
/// less a margin for the rounding.
pub fn slack(size: TreeSize, distance: f32, tree_factor: f32, lod_factor: f32) -> f32 {
    // stopped, nothing changes until the factor does, which checks every
    // object again (LodWatch::advance)
    if stopped(tree_factor) {
        return f32::MAX;
    }
    if !distance.is_finite() {
        return 0.0;
    }
    let mut changes = vec![size.radius];
    for a in ANGLES {
        let angle = a / tree_factor;
        if angle < 90.0 {
            changes.push(size.radius + size.max_scale / angle.to_radians().tan());
        }
    }
    let scale = distance_scale(lod_factor);
    if scale > 0.0 {
        for t in TANS {
            for edge in [t - 0.005, t + 0.005] {
                changes.push(tree_factor * 64.0 * size.billboard / edge / scale);
            }
        }
    }
    changes
        .into_iter()
        .map(|d| (distance - d).abs() - d * MARGIN)
        .fold(f32::MAX, f32::min)
        .max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree of the default species' proportions, 10 m tall.
    fn tree() -> TreeSize {
        TreeSize::of(aurora_prim::tree::tree_species(Some(0)), Vec3::new(4.0, 4.0, 10.0))
    }

    #[test]
    fn nearer_trees_are_finer_and_none_is_hidden_above_zero() {
        let mut last = 0;
        for d in (1..2000).map(|d| d as f32) {
            let level = trunk_level(tree(), d, 0.5, 2.0);
            assert!(level >= last, "coarser nearer at {d} m");
            assert!(level < HIDDEN_TRUNK, "hidden at {d} m");
            last = level;
        }
        assert_eq!(trunk_level(tree(), 1.0, 0.5, 2.0), 0);
        assert_eq!(last, 3);
    }

    #[test]
    fn a_higher_factor_keeps_detail_further() {
        let far = |factor| (1..2000).find(|d| trunk_level(tree(), *d as f32, factor, 2.0) == 3).unwrap_or(2000);
        assert!(far(1.0) > far(0.5));
        assert!(far(0.5) > far(0.125));
    }

    #[test]
    fn zero_hides_trees_and_stops_grass() {
        assert!(stopped(0.0));
        assert!(!stopped(0.125));
        assert_eq!(trunk_level(tree(), 50.0, 0.0, 2.0), HIDDEN_TRUNK);
        // inside the tree the angle is 180°, still nothing at 0
        assert_eq!(trunk_level(tree(), 0.0, 0.0, 2.0), HIDDEN_TRUNK);
    }

    #[test]
    fn the_angle_rule_matches_the_legacy_thresholds() {
        // only the angle: a huge billboard keeps the tangent at its finest
        let size = TreeSize {
            max_scale: 10.0,
            radius: 0.0,
            billboard: 1.0e6,
        };
        // atan2(10, d) x 1.0 against 30, 20, 15 degrees
        let at = |deg: f32| 10.0 / deg.to_radians().tan();
        assert_eq!(trunk_level(size, at(30.0) - 0.1, 1.0, 2.0), 0);
        assert_eq!(trunk_level(size, at(30.0) + 0.1, 1.0, 2.0), 1);
        assert_eq!(trunk_level(size, at(20.0) + 0.1, 1.0, 2.0), 2);
        assert_eq!(trunk_level(size, at(15.0) + 0.1, 1.0, 2.0), 3);
    }

    #[test]
    fn no_level_changes_within_the_slack() {
        for factor in [0.125, 0.5, 1.0, 2.0] {
            for lod in [0.5, 2.0, 4.0] {
                for d in (1..800).map(|d| d as f32 * 0.5) {
                    let level = trunk_level(tree(), d, factor, lod);
                    let s = slack(tree(), d, factor, lod);
                    for step in [-s, -s * 0.5, s * 0.5, s] {
                        let other = (d + step).max(0.0);
                        assert_eq!(
                            trunk_level(tree(), other, factor, lod),
                            level,
                            "factor {factor} lod {lod}: {d} m to {other} m (slack {s})"
                        );
                    }
                }
            }
        }
    }
}
