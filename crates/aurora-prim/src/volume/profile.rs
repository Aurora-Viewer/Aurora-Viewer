//! `LLProfile` port (cross-section generation).
//!
//! Ported from `indra/llmath/llvolume.cpp` of the Second Life viewer,
//! Copyright (C) Linden Research, Inc., originally LGPL 2.1.

use std::f32::consts::PI;

use glam::Vec3;

use super::{
    LL_FACE_INNER_SIDE, LL_FACE_OUTER_SIDE_0, LL_FACE_PATH_BEGIN, LL_FACE_PATH_END, LL_FACE_PROFILE_BEGIN, LL_FACE_PROFILE_END,
    MIN_DETAIL_FACES, ll_round, llceil, llfloor,
};
use crate::params::{
    LL_PCODE_HOLE_CIRCLE, LL_PCODE_HOLE_MASK, LL_PCODE_HOLE_SQUARE, LL_PCODE_HOLE_TRIANGLE, LL_PCODE_PROFILE_CIRCLE,
    LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PROFILE_EQUALTRI, LL_PCODE_PROFILE_ISOTRI, LL_PCODE_PROFILE_MASK, LL_PCODE_PROFILE_RIGHTTRI,
    LL_PCODE_PROFILE_SQUARE, ProfileParams,
};

/// `LLProfile::Face`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProfileFace {
    pub index: i32,
    pub count: i32,
    #[allow(dead_code)]
    pub scale_u: f32,
    pub cap: bool,
    pub flat: bool,
    pub face_id: u16,
}

/// `LLProfile`. Points are (x, y, texture-s) like LL's `mProfile`.
#[derive(Debug, Default)]
pub(crate) struct Profile {
    pub points: Vec<Vec3>,
    pub faces: Vec<ProfileFace>,
    pub open: bool,
    #[allow(dead_code)]
    pub concave: bool,
    pub total: i32,
    pub total_out: i32,
}

// LL's literal value (not exactly 1/sqrt(2)) is kept on purpose.
#[allow(clippy::approx_constant)]
const TABLE_SCALE: [f32; 8] = [1.0, 1.0, 1.0, 0.5, 0.707107, 0.53, 0.525, 0.5];

/// Hard cap on the number of profile points (defensive; LL never comes
/// close to this with valid parameters).
const MAX_PROFILE_POINTS: usize = 4096;

impl Profile {
    fn add_cap(&mut self, face_id: u16) {
        self.faces.push(ProfileFace {
            index: 0,
            count: self.total,
            scale_u: 1.0,
            cap: true,
            flat: false,
            face_id,
        });
    }

    fn add_face(&mut self, index: i32, count: i32, scale_u: f32, face_id: u16, flat: bool) {
        self.faces.push(ProfileFace {
            index,
            count,
            scale_u,
            cap: false,
            flat,
            face_id,
        });
    }

    #[inline]
    fn push(&mut self, p: Vec3) {
        if self.points.len() < MAX_PROFILE_POINTS {
            self.points.push(p);
        }
    }

    fn push_split(&mut self, target: Vec3, split: i32) {
        if let Some(&p) = self.points.last() {
            for i in 0..split {
                let f = 1.0f32 / (split + 1) as f32 * (i + 1) as f32;
                self.push((target - p) * f + p);
            }
        }
    }

    /// `LLProfile::genNGon`.
    fn gen_ngon(&mut self, params: &ProfileParams, sides: i32, offset: f32, ang_scale: f32, split: i32) {
        let sides = sides.max(1);
        let split = split.max(0);
        let mut scale = 0.5f32;
        let begin = params.begin;
        let end = params.end;
        let sides_f = sides as f32;

        let t_step = 1.0f32 / sides_f;
        let ang_step = 2.0 * PI * t_step * ang_scale;

        let total_sides = ll_round(sides_f / ang_scale);
        if (0..8).contains(&total_sides) {
            scale = TABLE_SCALE[total_sides as usize];
        }

        let t_first = (begin * sides_f).floor() / sides_f;

        let mut t = t_first;
        let mut ang = 2.0 * PI * (t * ang_scale + offset);
        let mut pt1 = Vec3::new(ang.cos() * scale, ang.sin() * scale, t);

        t += t_step;
        ang += ang_step;
        let mut pt2 = Vec3::new(ang.cos() * scale, ang.sin() * scale, t);

        let mut t_fraction = (begin - t_first) * sides_f;

        if t_fraction < 0.9999 {
            self.push(pt1.lerp(pt2, t_fraction));
        }

        while t < end {
            pt1 = Vec3::new(ang.cos() * scale, ang.sin() * scale, t);
            self.push_split(pt1, split);
            self.push(pt1);
            t += t_step;
            ang += ang_step;
            if self.points.len() >= MAX_PROFILE_POINTS {
                break;
            }
        }

        pt2 = Vec3::new(ang.cos() * scale, ang.sin() * scale, t);
        t_fraction = (end - (t - t_step)) * sides_f;
        if t_fraction > 0.0001 {
            let new_pt = pt1.lerp(pt2, t_fraction);
            self.push_split(new_pt, split);
            self.push(new_pt);
        }

        if (end - begin) * ang_scale < 0.99 {
            self.concave = (end - begin) * ang_scale > 0.5;
            self.open = true;
            if params.hollow <= 0.0 {
                self.push(Vec3::ZERO);
            }
        } else {
            self.open = false;
            self.concave = false;
        }

        self.total = self.points.len() as i32;
    }

    /// `LLProfile::addHole`.
    #[allow(clippy::too_many_arguments)]
    fn add_hole(&mut self, params: &ProfileParams, flat: bool, sides: f32, offset: f32, box_hollow: f32, ang_scale: f32, split: i32) {
        self.total_out = self.total;
        self.gen_ngon(params, llfloor(sides), offset, ang_scale, split);
        self.add_face(self.total_out, self.total - self.total_out, 0.0, LL_FACE_INNER_SIDE, flat);

        let lo = (self.total_out.max(0) as usize).min(self.points.len());
        let hi = (self.total.max(0) as usize).min(self.points.len());
        if lo < hi {
            let hole = &mut self.points[lo..hi];
            for p in hole.iter_mut() {
                *p *= box_hollow;
            }
            hole.reverse();
        }

        for f in &mut self.faces {
            if f.cap {
                f.count *= 2;
            }
        }
    }

    /// `LLProfile::generate`. Returns `false` if the parameters are invalid
    /// (LL then produces no geometry). `sculpt_size` is `Some(n)` for
    /// sculpted generation.
    pub(crate) fn generate(&mut self, params: &ProfileParams, path_open: bool, detail: f32, split: i32, sculpt_size: Option<i32>) -> bool {
        let detail = detail.max(0.0);
        self.points.clear();
        self.faces.clear();
        self.open = false;
        self.concave = false;
        self.total = 0;
        self.total_out = 0;

        let begin = params.begin;
        let end = params.end;
        let hollow = params.hollow;
        let is_hollow = hollow != 0.0;

        if begin.is_nan() || end.is_nan() || begin > end - 0.01 {
            return false;
        }

        let mut face_num = 0i32;
        let curve = params.curve_type;
        let hole_bits = curve & LL_PCODE_HOLE_MASK;

        match curve & LL_PCODE_PROFILE_MASK {
            LL_PCODE_PROFILE_SQUARE => {
                self.gen_ngon(params, 4, -0.375, 1.0, split);
                if path_open {
                    self.add_cap(LL_FACE_PATH_BEGIN);
                }
                let first = llfloor(begin * 4.0);
                let last = llfloor(end * 4.0 + 0.999);
                for i in first..last {
                    let id = if (0..4).contains(&i) {
                        LL_FACE_OUTER_SIDE_0 << i
                    } else {
                        LL_FACE_OUTER_SIDE_0
                    };
                    self.add_face(face_num * (split + 1), split + 2, 1.0, id, true);
                    face_num += 1;
                }
                for p in &mut self.points {
                    // Scale by 4 to generate proper tex coords.
                    p.z *= 4.0;
                }
                if is_hollow {
                    match hole_bits {
                        LL_PCODE_HOLE_TRIANGLE => {
                            // This offset is not correct, but we can't change it now... DK 11/17/04
                            self.add_hole(params, true, 3.0, -0.375, hollow, 1.0, split)
                        }
                        LL_PCODE_HOLE_CIRCLE => self.add_hole(params, false, MIN_DETAIL_FACES as f32 * detail, -0.375, hollow, 1.0, 0),
                        _ => self.add_hole(params, true, 4.0, -0.375, hollow, 1.0, split),
                    }
                }
                if path_open && let Some(f) = self.faces.first_mut() {
                    f.count = self.total;
                }
            }
            LL_PCODE_PROFILE_ISOTRI | LL_PCODE_PROFILE_RIGHTTRI | LL_PCODE_PROFILE_EQUALTRI => {
                self.gen_ngon(params, 3, 0.0, 1.0, split);
                for p in &mut self.points {
                    // Scale by 3 to generate proper tex coords.
                    p.z *= 3.0;
                }
                if path_open {
                    self.add_cap(LL_FACE_PATH_BEGIN);
                }
                let first = llfloor(begin * 3.0);
                let last = llfloor(end * 3.0 + 0.999);
                for i in first..last {
                    let id = if (0..4).contains(&i) {
                        LL_FACE_OUTER_SIDE_0 << i
                    } else {
                        LL_FACE_OUTER_SIDE_0
                    };
                    self.add_face(face_num * (split + 1), split + 2, 1.0, id, true);
                    face_num += 1;
                }
                if is_hollow {
                    // Swept triangles need smaller hollowness values,
                    // because the triangle doesn't fill the bounding box.
                    let triangle_hollow = hollow / 2.0;
                    match hole_bits {
                        LL_PCODE_HOLE_CIRCLE => {
                            self.add_hole(params, false, MIN_DETAIL_FACES as f32 * detail, 0.0, triangle_hollow, 1.0, 0)
                        }
                        LL_PCODE_HOLE_SQUARE => self.add_hole(params, true, 4.0, 0.0, triangle_hollow, 1.0, split),
                        _ => self.add_hole(params, true, 3.0, 0.0, triangle_hollow, 1.0, split),
                    }
                }
            }
            LL_PCODE_PROFILE_CIRCLE => {
                let mut hole_type = 0u8;
                let mut circle_detail = MIN_DETAIL_FACES as f32 * detail;
                if is_hollow {
                    hole_type = hole_bits;
                    if hole_type == LL_PCODE_HOLE_SQUARE {
                        // Snap to the next multiple of four sides,
                        // so that corners line up.
                        circle_detail = llceil(circle_detail / 4.0) as f32 * 4.0;
                    }
                }
                let mut sides = circle_detail as i32;
                if let Some(s) = sculpt_size {
                    sides = s;
                }
                self.gen_ngon(params, sides, 0.0, 1.0, 0);

                if path_open {
                    self.add_cap(LL_FACE_PATH_BEGIN);
                }
                if self.open && !is_hollow {
                    self.add_face(0, self.total - 1, 0.0, LL_FACE_OUTER_SIDE_0, false);
                } else {
                    self.add_face(0, self.total, 0.0, LL_FACE_OUTER_SIDE_0, false);
                }

                if is_hollow {
                    match hole_type {
                        LL_PCODE_HOLE_SQUARE => self.add_hole(params, true, 4.0, 0.0, hollow, 1.0, split),
                        LL_PCODE_HOLE_TRIANGLE => self.add_hole(params, true, 3.0, 0.0, hollow, 1.0, split),
                        _ => self.add_hole(params, false, circle_detail, 0.0, hollow, 1.0, 0),
                    }
                }
            }
            LL_PCODE_PROFILE_CIRCLE_HALF => {
                let mut hole_type = 0u8;
                // Number of faces is cut in half because it's only a half-circle.
                let mut circle_detail = MIN_DETAIL_FACES as f32 * detail * 0.5;
                if is_hollow {
                    hole_type = hole_bits;
                    if hole_type == LL_PCODE_HOLE_SQUARE {
                        // Snap to the next multiple of four sides (div 2),
                        // so that corners line up.
                        circle_detail = llceil(circle_detail / 2.0) as f32 * 2.0;
                    }
                }
                self.gen_ngon(params, llfloor(circle_detail), 0.5, 0.5, 0);
                if path_open {
                    self.add_cap(LL_FACE_PATH_BEGIN);
                }
                if self.open && !is_hollow {
                    self.add_face(0, self.total - 1, 0.0, LL_FACE_OUTER_SIDE_0, false);
                } else {
                    self.add_face(0, self.total, 0.0, LL_FACE_OUTER_SIDE_0, false);
                }

                if is_hollow {
                    match hole_type {
                        LL_PCODE_HOLE_SQUARE => self.add_hole(params, true, 2.0, 0.5, hollow, 0.5, split),
                        LL_PCODE_HOLE_TRIANGLE => self.add_hole(params, true, 3.0, 0.5, hollow, 0.5, split),
                        _ => self.add_hole(params, false, circle_detail, 0.5, hollow, 0.5, 0),
                    }
                }

                // Special case for openness of sphere
                if (params.end - params.begin) < 1.0 {
                    self.open = true;
                } else if !is_hollow {
                    self.open = false;
                    if let Some(&p0) = self.points.first() {
                        self.points.push(p0);
                        self.total += 1;
                    }
                }
            }
            _ => {
                // Unknown profile (LL_ERRS in LL).
                return false;
            }
        }

        if path_open {
            self.add_cap(LL_FACE_PATH_END); // bottom
        }

        if self.open {
            // interior edge caps
            self.add_face(self.total - 1, 2, 0.5, LL_FACE_PROFILE_BEGIN, true);
            if is_hollow {
                self.add_face(self.total_out - 1, 2, 0.5, LL_FACE_PROFILE_END, true);
            } else {
                self.add_face(self.total - 2, 2, 0.5, LL_FACE_PROFILE_END, true);
            }
        }

        true
    }
}
