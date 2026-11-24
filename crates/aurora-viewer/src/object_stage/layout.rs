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

    pub fn volume(&self) -> f32 {
        let s = self.size().max(Vec3::splat(1e-3));
        s.x * s.y * s.z
    }

    pub fn radius(&self) -> f32 {
        (self.size() * 0.5).length().max(1e-3)
    }

    pub fn moved(&self, by: Vec3) -> Bounds {
        Bounds {
            lo: self.lo + by,
            hi: self.hi + by,
        }
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

/// What each worn item moves by once fully pulled apart. The largest (the
/// body, most often) stays; the others go to its two sides, head to toe, each
/// on the side it is already on, level with where it is worn as long as that
/// place is free.
pub fn worn_apart(boxes: &[Bounds]) -> Vec<Vec3> {
    let mut out = vec![Vec3::ZERO; boxes.len()];
    let Some(anchor) = (0..boxes.len()).max_by(|a, b| boxes[*a].volume().total_cmp(&boxes[*b].volume())) else {
        return out;
    };
    let body = boxes[anchor];
    let height = whole(boxes).size().z.max(0.1);
    let gap = height * 0.025;
    let mut order: Vec<usize> = (0..boxes.len()).filter(|i| *i != anchor).collect();
    order.sort_by(|a, b| boxes[*b].center().z.total_cmp(&boxes[*a].center().z));
    // per side, the columns: their outer edge and how low each one is filled
    let mut sides: [Vec<(f32, f32, f32)>; 2] = [Vec::new(), Vec::new()];
    let bottom = whole(boxes).lo.z - height * 0.1;
    for i in order {
        let (b, size) = (boxes[i], boxes[i].size());
        let used = |side: &[(f32, f32, f32)]| side.iter().map(|c| c.2).sum::<f32>();
        // its own side when it clearly has one, else the emptier one
        let lean = b.center().y - body.center().y;
        let side = if lean > size.y * 0.25 {
            0
        } else if lean < -size.y * 0.25 {
            1
        } else {
            usize::from(used(&sides[0]) > used(&sides[1]))
        };
        let sign = if side == 0 { 1.0 } else { -1.0 };
        let columns = &mut sides[side];
        // the first column where it fits under what is there
        let top_wanted = b.center().z + size.z / 2.0;
        let at = columns.iter().position(|c| c.1.min(top_wanted) - size.z >= bottom);
        let column = match at {
            Some(c) => c,
            None => {
                let start = columns.last().map_or(body.size().y / 2.0 + gap * 2.0, |c| c.0 + gap * 2.0);
                columns.push((start, f32::INFINITY, 0.0));
                columns.len() - 1
            }
        };
        let inner = if column == 0 {
            body.size().y / 2.0 + gap * 2.0
        } else {
            columns[column - 1].0 + gap * 2.0
        };
        let top = columns[column].1.min(top_wanted);
        columns[column].0 = columns[column].0.max(inner + size.y);
        columns[column].1 = top - size.z - gap;
        columns[column].2 += size.z;
        let target = Vec3::new(body.center().x, body.center().y + sign * (inner + size.y / 2.0), top - size.z / 2.0);
        out[i] = target - b.center();
    }
    out
}

/// What each part of one object moves by once fully pulled apart: away from
/// the middle; parts that share it (rigged pieces worn over each other) fan
/// out around it.
pub fn parts_apart(boxes: &[Bounds]) -> Vec<Vec3> {
    if boxes.len() < 2 {
        return vec![Vec3::ZERO; boxes.len()];
    }
    let all = whole(boxes);
    let (middle, radius) = (all.center(), all.radius());
    const GOLDEN: f32 = 2.399_963;
    boxes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let d = b.center() - middle;
            if d.length() > radius * 0.06 {
                d * 1.1
            } else {
                let a = i as f32 * GOLDEN;
                Vec3::new(0.0, a.cos(), a.sin()) * radius * 0.7
            }
        })
        .collect()
}
