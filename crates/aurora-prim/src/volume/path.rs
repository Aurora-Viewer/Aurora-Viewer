//! `LLPath` port (sweep path generation).
//!
//! Ported from `indra/llmath/llvolume.cpp` of the Second Life viewer,
//! Copyright (C) Linden Research, Inc., originally LGPL 2.1.

use std::f32::consts::PI;

use glam::{Mat3, Quat, Vec3};

use super::{MIN_DETAIL_FACES, lerp, llfloor};
use crate::params::{LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_CIRCLE2, PathParams};

/// `LLPath::PathPt`. `rot` is the rotation as a column-vector matrix
/// (equivalent to LL's row-vector `LLMatrix3`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PathPt {
    pub pos: Vec3,
    pub rot: Mat3,
    pub scale: [f32; 2],
    pub tex_t: f32,
}

/// `LLPath`.
#[derive(Debug, Default)]
pub(crate) struct Path {
    pub points: Vec<PathPt>,
    pub open: bool,
}

// LL's literal value (not exactly 1/sqrt(2)) is kept on purpose.
#[allow(clippy::approx_constant)]
const TABLE_SCALE: [f32; 8] = [1.0, 1.0, 1.0, 0.5, 0.707107, 0.53, 0.525, 0.5];

/// Defensive cap on the number of path points.
const MAX_PATH_POINTS: usize = 4096;

fn begin_scale(p: &PathParams) -> [f32; 2] {
    let mut s = [1.0f32, 1.0];
    if p.scale[0] > 1.0 {
        s[0] = 2.0 - p.scale[0];
    }
    if p.scale[1] > 1.0 {
        s[1] = 2.0 - p.scale[1];
    }
    s
}

fn end_scale(p: &PathParams) -> [f32; 2] {
    let mut s = [1.0f32, 1.0];
    if p.scale[0] < 1.0 {
        s[0] = p.scale[0];
    }
    if p.scale[1] < 1.0 {
        s[1] = p.scale[1];
    }
    s
}

impl Path {
    /// `LLPath::genNGon` (circular path in the YZ plane, starting at +Y).
    fn gen_ngon(&mut self, params: &PathParams, sides: i32, end_scale: f32, twist_scale: f32) {
        let sides = sides.max(1);
        let sides_f = sides as f32;

        let revolutions = params.revolutions;
        let skew = params.skew;
        let skew_mag = skew.abs();
        let hole_x = params.scale[0] * (1.0 - skew_mag);
        let hole_y = params.scale[1];

        // Calculate taper begin/end for x,y (Negative means taper the beginning)
        let mut taper_x_begin = 1.0f32;
        let mut taper_x_end = 1.0 - params.taper[0];
        let mut taper_y_begin = 1.0f32;
        let mut taper_y_end = 1.0 - params.taper[1];
        if taper_x_end > 1.0 {
            taper_x_begin = 2.0 - taper_x_end;
            taper_x_end = 1.0;
        }
        if taper_y_end > 1.0 {
            taper_y_begin = 2.0 - taper_y_end;
            taper_y_end = 1.0;
        }

        // For spheres, the radius is usually zero.
        let mut radius_start = 0.5f32;
        if sides < 8 {
            radius_start = TABLE_SCALE[sides as usize];
        }
        // Scale the radius to take the hole size into account.
        radius_start *= 1.0 - hole_y;

        let mut radius_end = radius_start;
        let radius_offset = params.radius_offset;
        if radius_offset < 0.0 {
            radius_start *= 1.0 + radius_offset;
        } else {
            radius_end *= 1.0 - radius_offset;
        }

        // Is the path NOT a closed loop?
        self.open = (params.end * end_scale - params.begin < 1.0)
            || (skew_mag > 0.001)
            || ((taper_x_end - taper_x_begin).abs() > 0.001)
            || ((taper_y_end - taper_y_begin).abs() > 0.001)
            || ((radius_end - radius_start).abs() > 0.001);

        let twist_begin = params.twist_begin * twist_scale;
        let twist_end = params.twist_end * twist_scale;
        let shear = params.shear;

        let make = |t: f32| -> PathPt {
            let ang = 2.0 * PI * revolutions * t;
            let radius = lerp(radius_start, radius_end, t);
            let s = ang.sin() * radius;
            let c = ang.cos() * radius;
            let pos = Vec3::new(lerp(0.0, shear[0], s) + lerp(-skew, skew, t) * 0.5, c + lerp(0.0, shear[1], s), s);
            let scale = [
                hole_x * lerp(taper_x_begin, taper_x_end, t),
                hole_y * lerp(taper_y_begin, taper_y_end, t),
            ];
            // LL: rot = twist * qang (LL quaternion order: twist applied first).
            let twist_ang = lerp(twist_begin, twist_end, t) * 2.0 * PI - PI;
            let q = Quat::from_rotation_x(ang) * Quat::from_rotation_z(twist_ang);
            PathPt {
                pos,
                rot: Mat3::from_quat(q),
                scale,
                tex_t: t,
            }
        };

        // We run through this once before the main loop, to make sure
        // the path begins at the correct cut.
        let step = 1.0f32 / sides_f;
        let mut t = params.begin;
        self.points.push(make(t));

        t += step;
        // Snap to a quantized parameter, so that cut does not
        // affect most sample points.
        t = ((t * sides_f) as i32) as f32 / sides_f;

        // Run through the non-cut dependent points.
        while t < params.end && self.points.len() < MAX_PATH_POINTS {
            self.points.push(make(t));
            t += step;
        }

        // Make one final pass for the end cut.
        self.points.push(make(params.end));
    }

    /// `LLPath::generate`. `sculpt_size` is `Some(n)` for sculpted generation.
    /// Path types other than circle/circle2 (line, test, flexible) are
    /// generated as a straight line.
    pub(crate) fn generate(&mut self, params: &PathParams, detail: f32, split: i32, sculpt_size: Option<i32>) {
        let detail = detail.max(0.0);
        self.points.clear();
        self.open = true;

        match params.curve_type & 0xf0 {
            LL_PCODE_PATH_CIRCLE => {
                // Increase the detail as the revolutions and twist increase.
                let twist_mag = (params.twist_begin - params.twist_end).abs();
                let mut sides =
                    llfloor(llfloor(MIN_DETAIL_FACES as f32 * detail + twist_mag * 3.5 * (detail - 0.5)) as f32 * params.revolutions);
                if let Some(s) = sculpt_size {
                    sides = s.max(1);
                }
                if sides > 0 {
                    self.gen_ngon(params, sides, 1.0, 1.0);
                }
            }
            LL_PCODE_PATH_CIRCLE2 => {
                if params.end - params.begin >= 0.99 && params.scale[0] >= 0.99 {
                    self.open = false;
                }
                self.gen_ngon(params, llfloor(MIN_DETAIL_FACES as f32 * detail), 1.0, 1.0);
                let mut toggle = 0.5f32;
                for p in &mut self.points {
                    p.pos.x = toggle;
                    toggle = if toggle == 0.5 { -0.5 } else { 0.5 };
                }
            }
            _ => {
                // LL_PCODE_PATH_LINE (and test/flexible, treated as a line).
                // Take the begin/end twist into account for detail.
                let mut np = llfloor((params.twist_begin - params.twist_end).abs() * 3.5 * (detail - 0.5)) + 2;
                if np < split + 2 {
                    np = split + 2;
                }
                let np = np.clamp(2, MAX_PATH_POINTS as i32);
                let step = 1.0f32 / (np - 1) as f32;
                let bs = begin_scale(params);
                let es = end_scale(params);
                self.points.reserve(np as usize);
                for i in 0..np {
                    let t = lerp(params.begin, params.end, i as f32 * step);
                    let pos = Vec3::new(lerp(0.0, params.shear[0], t), lerp(0.0, params.shear[1], t), t - 0.5);
                    let rot = Mat3::from_quat(Quat::from_rotation_z(lerp(PI * params.twist_begin, PI * params.twist_end, t)));
                    self.points.push(PathPt {
                        pos,
                        rot,
                        scale: [lerp(bs[0], es[0], t), lerp(bs[1], es[1], t)],
                        tex_t: t,
                    });
                }
            }
        }

        if params.twist_end != params.twist_begin {
            self.open = true;
        }
    }
}
