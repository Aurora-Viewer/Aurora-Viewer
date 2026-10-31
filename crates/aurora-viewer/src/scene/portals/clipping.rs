//! The aperture starts a per-avatar crossing, never a fixed occlusion volume.
//! Partial lateral exits restore the mesh; completed crossings stay hidden.
use aurora_render::PortalClip;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Key(pub Uuid, pub Uuid, pub u64);

pub struct Aperture {
    pub key: Key,
    pub center: Vec3,
    pub normal: Vec3,
}

pub struct Avatar {
    pub id: Uuid,
    pub index: usize,
    pub root: Vec3,
    pub forward: Vec3,
    pub placement_serial: u64,
    pub bounds: Vec<Bounds>,
}

/// Posed joint/rigid box. Keeping its axes avoids treating a tall body's
/// culling-sphere radius as its depth through the doorway.
pub struct Bounds {
    center: Vec3,
    axes: [Vec3; 3],
}

impl Bounds {
    pub fn new(min: Vec3, max: Vec3, model: Mat4) -> Option<Self> {
        if !min.is_finite() || !max.is_finite() || !model.is_finite() || !min.cmple(max).all() {
            return None;
        }
        let half = (max - min) * 0.5;
        let bounds = Self {
            center: model.transform_point3((min + max) * 0.5),
            axes: [
                model.x_axis.truncate() * half.x,
                model.y_axis.truncate() * half.y,
                model.z_axis.truncate() * half.z,
            ],
        };
        (bounds.center.is_finite() && bounds.axes.iter().all(|v| v.is_finite())).then_some(bounds)
    }

    fn range(&self, origin: Vec3, normal: Vec3) -> (f32, f32) {
        let center = (self.center - origin).dot(normal);
        let radius: f32 = self.axes.iter().map(|axis| axis.dot(normal).abs()).sum();
        (center - radius, center + radius)
    }

    fn touches(&self, portal: &Aperture) -> bool {
        let (a, b) = self.range(portal.center, portal.normal);
        if a >= aurora_render::PORTAL_DISSOLVE_WIDTH || b < -aurora_render::PORTAL_DISSOLVE_WIDTH {
            return false;
        }
        let tangent = Vec3::new(-portal.normal.y, portal.normal.x, 0.0);
        let (lo, hi) = self.range(portal.center, tangent);
        let x = 0.0_f32.clamp(lo, hi) / crate::world::portals::WIDTH;
        let (lo, hi) = self.range(portal.center, Vec3::Z);
        let z = 0.0_f32.clamp(lo, hi) / crate::world::portals::HEIGHT;
        x * x + z * z <= 1.0
    }
}

impl Avatar {
    fn range(&self, portal: &Aperture, side: f32) -> Option<(f32, f32)> {
        self.bounds
            .iter()
            .map(|b| b.range(portal.center, portal.normal * side))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }
}

#[derive(Clone, Copy)]
struct Crossing {
    key: Key,
    side: f32,
    complete: bool,
    backtrack: f32,
}

#[derive(Default)]
pub struct Clipping {
    crossings: HashMap<Uuid, Crossing>,
    samples: HashMap<Uuid, (Vec3, u64)>,
}

impl Aperture {
    pub fn inside(&self, root: Vec3) -> bool {
        let delta = root - self.center;
        let tangent = Vec3::new(-self.normal.y, self.normal.x, 0.0);
        (delta.dot(tangent) / crate::world::portals::WIDTH).powi(2) + (delta.z / crate::world::portals::HEIGHT).powi(2) <= 1.0
    }

    pub fn near(&self, (center, radius): (Vec3, f32)) -> bool {
        center.is_finite()
            && radius.is_finite()
            && radius >= 0.0
            && (center - self.center).dot(self.normal).abs() <= radius + aurora_render::PORTAL_DISSOLVE_WIDTH
    }

    fn crossed(&self, from: Vec3, to: Vec3) -> bool {
        let a = (from - self.center).dot(self.normal);
        let b = (to - self.center).dot(self.normal);
        a * b <= 0.0 && (a - b).abs() > 1e-5 && self.inside(from.lerp(to, (a / (a - b)).clamp(0.0, 1.0)))
    }
}

impl Clipping {
    pub fn clear(&mut self) {
        self.crossings.clear();
        self.samples.clear();
    }

    pub fn forget(&mut self, id: Uuid) {
        self.crossings.remove(&id);
        self.samples.remove(&id);
    }

    pub fn active(&self, id: Uuid) -> bool {
        self.crossings.contains_key(&id)
    }

    pub fn update(&mut self, portals: &[Aperture], avatars: &[Avatar]) -> HashMap<usize, PortalClip> {
        let mut visible = HashMap::new();
        let mut next = HashMap::new();
        for avatar in avatars {
            if !avatar.root.is_finite() {
                continue;
            }
            // Simulator/region placement is not a continuation of the old cut.
            let from = self
                .samples
                .get(&avatar.id)
                .filter(|(p, serial)| *serial == avatar.placement_serial && p.distance(avatar.root) <= 3.0)
                .map(|(p, _)| *p);
            let old = from
                .and_then(|_| self.crossings.get(&avatar.id))
                .copied()
                .filter(|old| old.complete || portals.iter().any(|p| p.key == old.key && p.inside(avatar.root)));
            let contact = |portal: &&Aperture| {
                portal.inside(avatar.root)
                    && (avatar.bounds.iter().any(|b| b.touches(portal)) || from.is_some_and(|p| portal.crossed(p, avatar.root)))
            };
            let selected = old.and_then(|c| portals.iter().find(|p| p.key == c.key)).or_else(|| {
                portals.iter().filter(contact).min_by(|a, b| {
                    let distance = |p: &Aperture| (avatar.root - p.center).dot(p.normal).abs();
                    distance(a).total_cmp(&distance(b))
                })
            });
            let Some(portal) = selected else { continue };
            let side = old.filter(|c| c.key == portal.key).map_or_else(
                || {
                    let start = from.filter(|&p| portal.crossed(p, avatar.root)).unwrap_or(avatar.root);
                    let distance = (start - portal.center).dot(portal.normal);
                    if distance.abs() > 1e-3 {
                        distance.signum()
                    } else if avatar.forward.dot(portal.normal) > 0.0 {
                        -1.0
                    } else {
                        1.0
                    }
                },
                |c| c.side,
            );
            let mut complete = old.filter(|c| c.key == portal.key).is_some_and(|c| c.complete);
            let mut backtrack = old.filter(|c| c.key == portal.key).map_or(0.0, |c| c.backtrack);
            let range = avatar.range(portal, side);
            let inside = portal.inside(avatar.root);
            if complete {
                // A canceled teleport can be retraced through the same opening.
                // Animation alone cannot resurrect a completed traveler.
                backtrack = if inside {
                    (backtrack + from.map_or(0.0, |p| (avatar.root - p).dot(portal.normal * side))).max(0.0)
                } else {
                    0.0
                };
                let reverse_crossed =
                    from.is_some_and(|p| portal.crossed(p, avatar.root) && (avatar.root - p).dot(portal.normal * side) > 0.0);
                if inside
                    && backtrack >= 0.02
                    && (reverse_crossed || range.is_some_and(|(a, b)| a < aurora_render::PORTAL_DISSOLVE_WIDTH && b > 0.0))
                {
                    complete = false;
                }
            } else if !inside || range.is_some_and(|(a, _)| a >= aurora_render::PORTAL_DISSOLVE_WIDTH) {
                continue;
            } else if range.is_some_and(|(_, b)| b <= 0.0) {
                complete = true;
            }
            let clip = if complete {
                Some(PortalClip::hidden())
            } else {
                PortalClip::new(portal.center, portal.normal * side)
            };
            if let Some(clip) = clip {
                next.insert(
                    avatar.id,
                    Crossing {
                        key: portal.key,
                        side,
                        complete,
                        backtrack,
                    },
                );
                visible.insert(avatar.index, clip);
            }
        }
        self.crossings = next;
        self.samples = avatars
            .iter()
            .filter(|a| a.root.is_finite())
            .map(|a| (a.id, (a.root, a.placement_serial)))
            .collect();
        visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn portal() -> Aperture {
        Aperture {
            key: Key(Uuid::from_u128(1), Uuid::from_u128(2), 256),
            center: Vec3::ZERO,
            normal: Vec3::X,
        }
    }
    fn avatar(root: Vec3, half: Vec3) -> Avatar {
        Avatar {
            id: Uuid::from_u128(3),
            index: 7,
            root,
            forward: Vec3::NEG_X,
            placement_serial: 0,
            bounds: vec![Bounds::new(-half, half, Mat4::from_translation(root)).unwrap()],
        }
    }
    fn body(root: Vec3) -> Avatar {
        avatar(root, Vec3::new(0.2, 0.3, 1.1))
    }

    #[test]
    fn adjacent_and_already_distant_avatars_do_not_enter_an_occlusion_zone() {
        let mut state = Clipping::default();
        for root in [Vec3::new(0.15, 2.4, 0.0), Vec3::new(-2.0, 0.0, 0.0), Vec3::new(0.1, 0.0, 4.0)] {
            let mut a = body(root);
            if root.y != 0.0 {
                a.bounds
                    .push(Bounds::new(Vec3::splat(-3.0), Vec3::splat(3.0), Mat4::IDENTITY).unwrap());
            }
            assert!(state.update(&[portal()], &[a]).is_empty());
            state.clear();
        }
    }

    #[test]
    fn an_animated_limb_can_start_contact_without_moving_the_root() {
        let mut state = Clipping::default();
        let root = Vec3::new(6.0, 0.0, 0.0);
        assert!(state.update(&[portal()], &[body(root)]).is_empty());
        let mut reaching = body(root);
        reaching
            .bounds
            .push(Bounds::new(Vec3::new(-0.2, -0.1, -0.1), Vec3::new(0.3, 0.1, 0.1), Mat4::IDENTITY).unwrap());
        let clip = state.update(&[portal()], &[reaching])[&7];
        assert!(clip.keeps_point(root));
        assert!(!clip.keeps_point(Vec3::new(-0.1, 0.0, 0.0)));
    }

    #[test]
    fn a_partial_lateral_exit_restores_the_mesh_without_leaving_feet_or_caps() {
        let mut state = Clipping::default();
        let clip = state.update(&[portal()], &[body(Vec3::X * 0.15)])[&7];
        assert!(!clip.keeps_point(Vec3::new(-0.1, 0.0, -2.0)));
        assert!(!clip.keeps_point(Vec3::new(-0.1, 2.0, 0.0)));
        assert!(state.update(&[portal()], &[body(Vec3::new(0.15, 2.4, 0.0))]).is_empty());
        let again = state.update(&[portal()], &[body(Vec3::X * 0.15)])[&7];
        assert_eq!(again, clip);
    }

    #[test]
    fn a_fully_crossed_mesh_stays_gone_beside_the_portal_until_arrival_or_reverse_entry() {
        let mut state = Clipping::default();
        state.update(&[portal()], &[body(Vec3::X * 0.3)]);
        let complete = state.update(&[portal()], &[body(Vec3::X * -0.4)])[&7];
        assert!(complete.is_hidden());
        let animated = state.update(&[portal()], &[avatar(Vec3::X * -0.4, Vec3::new(0.8, 0.3, 1.1))])[&7];
        assert!(animated.is_hidden(), "a pose change cannot resurrect an already vanished mesh");
        for root in [Vec3::new(-0.4, 2.4, 0.0), Vec3::new(0.2, 2.4, 0.0)] {
            let clip = state.update(&[portal()], &[body(root)])[&7];
            assert!(clip.is_hidden());
            for point in [root, root - Vec3::Z * 2.0, root + Vec3::Y * 4.0] {
                assert!(!clip.keeps_point(point));
            }
        }
        let root = Vec3::new(0.2, 2.4, 0.0);
        let mut neighbor = body(root);
        neighbor.id = Uuid::from_u128(4);
        neighbor.index = 8;
        let same_position = state.update(&[portal()], &[body(root), neighbor]);
        assert!(same_position[&7].is_hidden());
        assert!(!same_position.contains_key(&8), "the same position must not hide a non-traveler");
        assert!(state.update(&[portal()], &[body(Vec3::new(-0.4, 2.4, 0.0))])[&7].is_hidden());
        assert!(state.update(&[portal()], &[body(Vec3::X * -0.4)])[&7].is_hidden());
        let reverse = state.update(&[portal()], &[body(Vec3::X * 0.15)])[&7];
        assert!(!reverse.is_hidden());
        assert!(!reverse.keeps_point(Vec3::X * -0.1));
        assert!(reverse.keeps_point(Vec3::X * 0.6));
        assert!(state.update(&[portal()], &[body(Vec3::X * 0.8)]).is_empty());
    }

    #[test]
    fn retreat_restores_the_same_fragments_without_flipping_the_entry_side() {
        for side in [-1.0, 1.0] {
            let mut state = Clipping::default();
            let first = state.update(&[portal()], &[body(Vec3::X * (side * 0.3))])[&7];
            for distance in [0.15, -0.1, -0.5, -0.1, 0.15, 0.3] {
                let mut a = body(Vec3::X * (distance * side));
                a.forward = Vec3::X * side;
                let clip = state.update(&[portal()], &[a])[&7];
                assert_eq!(state.crossings[&Uuid::from_u128(3)].side, side);
                if !clip.is_hidden() {
                    assert_eq!(clip, first);
                }
                assert_eq!(clip.keeps_point(Vec3::X * (side * 0.6)), !clip.is_hidden());
            }
        }
    }

    #[test]
    fn arrivals_restore_the_body_as_it_emerges() {
        for side in [-1.0, 1.0] {
            let mut state = Clipping::default();
            let mut counts = Vec::new();
            for distance in [0.15, 0.3, 0.7] {
                let root = Vec3::X * (side * distance);
                let clip = state.update(&[portal()], &[body(root)]).get(&7).copied();
                counts.push(
                    (0..256)
                        .filter(|&i| {
                            clip.is_none_or(|p| {
                                p.keeps_point(root + Vec3::Y * ((i % 16) as f32 * 0.06) + Vec3::Z * ((i / 16) as f32 * 0.06))
                            })
                        })
                        .count(),
                );
            }
            assert!((1..256).contains(&counts[0]));
            assert!(counts.windows(2).all(|p| p[0] <= p[1]));
            assert_eq!(counts[2], 256);
        }
    }

    #[test]
    fn swept_crossings_handle_fast_motion_but_teleports_and_arrivals_clear_the_state() {
        let mut state = Clipping::default();
        state.update(&[portal()], &[body(Vec3::X * 0.8)]);
        assert!(state.update(&[portal()], &[body(Vec3::X * -0.8)])[&7].is_hidden());
        state.forget(Uuid::from_u128(3));
        assert!(state.update(&[portal()], &[body(Vec3::X * -0.8)]).is_empty());
        state.update(&[portal()], &[body(Vec3::X * -0.3)]);
        assert!(state.update(&[portal()], &[body(Vec3::X * 8.0)]).is_empty());
    }

    #[test]
    fn explicit_short_placements_do_not_sweep_or_retain_the_old_crossing() {
        let mut state = Clipping::default();
        state.update(&[portal()], &[body(Vec3::X * 0.3)]);
        assert!(state.update(&[portal()], &[body(Vec3::X * -0.4)])[&7].is_hidden());
        let mut arrived = body(Vec3::X * -0.15);
        arrived.placement_serial = 1;
        let clip = state.update(&[portal()], &[arrived])[&7];
        assert!(!clip.is_hidden());
        assert!(clip.normal().dot(Vec3::NEG_X) > 0.99, "arrival starts on its own side");
    }

    #[test]
    fn tall_posed_boxes_keep_their_true_crossing_depth_and_invalid_data_is_ignored() {
        let b = Bounds::new(Vec3::new(-0.2, -0.3, -3.0), Vec3::new(0.2, 0.3, 3.0), Mat4::IDENTITY).unwrap();
        assert_eq!(b.range(Vec3::ZERO, Vec3::X), (-0.2, 0.2));
        assert!(Bounds::new(Vec3::NAN, Vec3::ONE, Mat4::IDENTITY).is_none());
        assert!(Bounds::new(Vec3::ONE, Vec3::ZERO, Mat4::IDENTITY).is_none());
        let mut state = Clipping::default();
        state.update(&[portal()], &[body(Vec3::X * 0.15)]);
        assert!(state.update(&[], &[body(Vec3::X * 0.15)]).is_empty());
        assert!(!state.active(Uuid::from_u128(3)));
    }
}
