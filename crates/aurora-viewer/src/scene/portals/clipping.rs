//! Geometry-driven portal cuts. An avatar root may stay still while its posed
//! body or a worn object reaches the plane. Entry side is independent of camera.
use aurora_render::PortalClip;
use glam::Vec3;
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
    pub bounds: Vec<(Vec3, f32)>,
}

#[derive(Clone, Copy)]
struct Cut {
    key: Key,
    side: f32,
}

#[derive(Default)]
pub struct Clipping {
    cuts: HashMap<Uuid, Cut>,
}

impl Aperture {
    fn near(&self, (center, radius): (Vec3, f32)) -> bool {
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return false;
        }
        let delta = center - self.center;
        if delta.dot(self.normal).abs() > radius + 3.5 {
            return false;
        }
        let tangent = Vec3::new(-self.normal.y, self.normal.x, 0.0);
        let width = crate::world::portals::WIDTH + radius + 0.25;
        let height = crate::world::portals::HEIGHT + radius + 0.25;
        (delta.dot(tangent) / width).powi(2) + (delta.z / height).powi(2) <= 1.0
    }

    fn plane(&self, side: f32) -> Option<PortalClip> {
        PortalClip::new(self.center, self.normal * side)
    }
}

impl Clipping {
    pub fn clear(&mut self) {
        self.cuts.clear();
    }

    pub fn update(&mut self, portals: &[Aperture], avatars: &[Avatar]) -> HashMap<usize, PortalClip> {
        let mut planes = HashMap::new();
        let mut next = HashMap::new();
        for avatar in avatars {
            let near = |portal: &&Aperture| avatar.bounds.iter().any(|&sphere| portal.near(sphere));
            let previous = self.cuts.get(&avatar.id);
            let selected = previous
                .and_then(|old| portals.iter().find(|portal| portal.key == old.key).filter(near))
                .or_else(|| {
                    portals.iter().filter(near).min_by(|a, b| {
                        let distance = |p: &Aperture| (avatar.root - p.center).dot(p.normal).abs();
                        distance(a).total_cmp(&distance(b))
                    })
                });
            let Some(portal) = selected else { continue };
            let side = previous.filter(|old| old.key == portal.key).map_or_else(
                || {
                    let distance = (avatar.root - portal.center).dot(portal.normal);
                    if distance.abs() > 1e-3 {
                        distance.signum()
                    } else if avatar.forward.dot(portal.normal) > 0.0 {
                        -1.0
                    } else {
                        1.0
                    }
                },
                |old| old.side,
            );
            if let Some(clip) = portal.plane(side) {
                next.insert(avatar.id, Cut { key: portal.key, side });
                planes.insert(avatar.index, clip);
            }
        }
        self.cuts = next;
        planes
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
    fn avatar(root: Vec3, bounds: Vec<(Vec3, f32)>) -> Avatar {
        Avatar {
            id: Uuid::from_u128(3),
            index: 7,
            root,
            forward: Vec3::NEG_X,
            bounds,
        }
    }
    fn visible(plane: PortalClip, vertex: Vec3) -> bool {
        plane.keeps_point(vertex)
    }

    #[test]
    fn a_stationary_root_keeps_its_plane_when_an_animated_arm_or_large_attachment_crosses() {
        let mut clipping = Clipping::default();
        let root = Vec3::new(6.0, 0.0, 0.0);
        let before = avatar(root, vec![(root, 0.5)]);
        assert!(clipping.update(&[portal()], &[before]).is_empty());
        // Only the deformed/worn bound changes; the root never moved.
        let stretched = avatar(root, vec![(root, 0.5), (Vec3::new(1.0, 0.0, 0.0), 2.0)]);
        let plane = clipping.update(&[portal()], &[stretched])[&7];
        assert!(visible(plane, root));
        assert!(visible(plane, Vec3::new(0.2, 0.0, 0.0)));
        assert!(!visible(plane, Vec3::new(-0.2, 0.0, 0.0)));
        assert!(
            visible(plane, Vec3::new(-0.2, 1.1, 0.0)),
            "a wide attachment beyond the rim remains visible"
        );
        assert!(
            visible(plane, Vec3::new(-0.2, 0.0, 1.6)),
            "an extended limb above the opening remains visible"
        );
    }

    #[test]
    fn the_entry_side_is_latched_as_the_root_crosses_and_cut_is_released_when_leaving() {
        let mut clipping = Clipping::default();
        let plane = clipping.update(&[portal()], &[avatar(Vec3::X, vec![(Vec3::X, 1.0)])])[&7];
        let after = clipping.update(&[portal()], &[avatar(-Vec3::X, vec![(-Vec3::X, 1.0)])])[&7];
        assert_eq!(after, plane);
        assert!(!visible(after, -Vec3::X));
        assert!(clipping.update(&[], &[avatar(-Vec3::X, vec![(-Vec3::X, 1.0)])]).is_empty());
        let reverse = clipping.update(&[portal()], &[avatar(-Vec3::X, vec![(-Vec3::X, 1.0)])])[&7];
        assert!(visible(reverse, -Vec3::X));
        assert!(!visible(reverse, Vec3::X));
    }

    #[test]
    fn unrelated_avatars_outside_the_aperture_and_malformed_bounds_are_not_cut() {
        let mut clipping = Clipping::default();
        for root in [Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, 0.0, 10.0)] {
            assert!(clipping.update(&[portal()], &[avatar(root, vec![(root, 0.5)])]).is_empty());
        }
        assert!(clipping.update(&[portal()], &[avatar(Vec3::X, vec![(Vec3::NAN, 1.0)])]).is_empty());
    }
}
