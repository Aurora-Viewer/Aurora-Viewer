//! Where the groups and their prims stand in the 3D view: together, or pulled
//! apart. Pure geometry.

use glam::Vec3;

/// A box in the model's space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub lo: Vec3,
    pub hi: Vec3,
}

impl Default for Bounds {
    /// Nothing yet: any point widens it.
    fn default() -> Self {
        Bounds {
            lo: Vec3::splat(f32::INFINITY),
            hi: Vec3::splat(f32::NEG_INFINITY),
        }
    }
}

impl Bounds {
    pub fn add(&mut self, p: Vec3) {
        self.lo = self.lo.min(p);
        self.hi = self.hi.max(p);
    }

    pub fn join(&mut self, other: Bounds) {
        self.lo = self.lo.min(other.lo);
        self.hi = self.hi.max(other.hi);
    }

    pub fn empty(&self) -> bool {
        !(self.lo.cmple(self.hi).all() && self.lo.is_finite() && self.hi.is_finite())
    }

    pub fn center(&self) -> Vec3 {
        (self.lo + self.hi) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.hi - self.lo
    }

    pub fn radius(&self) -> f32 {
        (self.size() * 0.5).length().max(1e-3)
    }

    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            let pick = |bit: usize, lo: f32, hi: f32| if i & bit == 0 { lo } else { hi };
            Vec3::new(
                pick(1, self.lo.x, self.hi.x),
                pick(2, self.lo.y, self.hi.y),
                pick(4, self.lo.z, self.hi.z),
            )
        })
    }

    /// Where a ray enters the box, if it does.
    pub fn hit(&self, from: Vec3, dir: Vec3) -> Option<f32> {
        let inv = dir.recip();
        let (a, b) = ((self.lo - from) * inv, (self.hi - from) * inv);
        let near = a.min(b).max_element();
        let far = a.max(b).min_element();
        (near <= far && far >= 0.0).then_some(near.max(0.0))
    }
}

pub fn whole(boxes: &[Bounds]) -> Bounds {
    boxes.iter().fold(Bounds::default(), |mut all, b| {
        all.join(*b);
        all
    })
}

/// Objects of the world side by side, along the viewer's left to right, each
/// on the same ground: how far each one moves from its own place.
pub fn side_by_side(boxes: &[Bounds]) -> Vec<Vec3> {
    let widths: f32 = boxes.iter().map(|b| b.size().y).sum();
    let gap = (widths / boxes.len().max(1) as f32 * 0.15).max(0.05);
    let total = widths + gap * boxes.len().saturating_sub(1) as f32;
    let mut y = total / 2.0;
    boxes
        .iter()
        .map(|b| {
            let c = b.center();
            let at = Vec3::new(-c.x, y - b.size().y / 2.0 - c.y, -b.lo.z);
            y -= b.size().y + gap;
            at
        })
        .collect()
}
