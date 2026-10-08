//! Volume (prim shape) parameters, ported from `llmath/llvolume.h` and
//! `llprimitive/llvolumemessage.cpp`.

pub const CUT_QUANTA: f32 = 0.00002;
pub const SCALE_QUANTA: f32 = 0.01;
pub const SHEAR_QUANTA: f32 = 0.01;
pub const TAPER_QUANTA: f32 = 0.01;
pub const REV_QUANTA: f32 = 0.015;
pub const HOLLOW_QUANTA: f32 = 0.00002;

// Profile curve types (low nibble)
pub const LL_PCODE_PROFILE_MASK: u8 = 0x0f;
pub const LL_PCODE_PROFILE_CIRCLE: u8 = 0x00;
pub const LL_PCODE_PROFILE_SQUARE: u8 = 0x01;
pub const LL_PCODE_PROFILE_ISOTRI: u8 = 0x02;
pub const LL_PCODE_PROFILE_EQUALTRI: u8 = 0x03;
pub const LL_PCODE_PROFILE_RIGHTTRI: u8 = 0x04;
pub const LL_PCODE_PROFILE_CIRCLE_HALF: u8 = 0x05;

// Hole types (high nibble)
pub const LL_PCODE_HOLE_MASK: u8 = 0xf0;
pub const LL_PCODE_HOLE_SAME: u8 = 0x00;
pub const LL_PCODE_HOLE_CIRCLE: u8 = 0x10;
pub const LL_PCODE_HOLE_SQUARE: u8 = 0x20;
pub const LL_PCODE_HOLE_TRIANGLE: u8 = 0x30;

// Path curve types
pub const LL_PCODE_PATH_LINE: u8 = 0x10;
pub const LL_PCODE_PATH_CIRCLE: u8 = 0x20;
pub const LL_PCODE_PATH_CIRCLE2: u8 = 0x30;
pub const LL_PCODE_PATH_TEST: u8 = 0x40;
pub const LL_PCODE_PATH_FLEXIBLE: u8 = 0x80;

// Object pcodes
pub const LL_PCODE_VOLUME: u8 = 9;
pub const LL_PCODE_LEGACY_AVATAR: u8 = 0x2F;
pub const LL_PCODE_LEGACY_GRASS: u8 = 0x5F;
pub const LL_PCODE_LEGACY_PART_SYS: u8 = 0x8F;
pub const LL_PCODE_LEGACY_TREE: u8 = 0xFF;
pub const LL_PCODE_TREE_NEW: u8 = 0x6F;

// Sculpt types
pub const LL_SCULPT_TYPE_NONE: u8 = 0;
pub const LL_SCULPT_TYPE_SPHERE: u8 = 1;
pub const LL_SCULPT_TYPE_TORUS: u8 = 2;
pub const LL_SCULPT_TYPE_PLANE: u8 = 3;
pub const LL_SCULPT_TYPE_CYLINDER: u8 = 4;
pub const LL_SCULPT_TYPE_MESH: u8 = 5;
pub const LL_SCULPT_TYPE_MASK: u8 = 0x07;
pub const LL_SCULPT_FLAG_INVERT: u8 = 64;
pub const LL_SCULPT_FLAG_MIRROR: u8 = 128;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileParams {
    pub curve_type: u8,
    pub begin: f32,
    pub end: f32,
    pub hollow: f32,
}

impl Default for ProfileParams {
    fn default() -> Self {
        Self {
            curve_type: LL_PCODE_PROFILE_SQUARE,
            begin: 0.0,
            end: 1.0,
            hollow: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathParams {
    pub curve_type: u8,
    pub begin: f32,
    pub end: f32,
    pub scale: [f32; 2],
    pub shear: [f32; 2],
    pub twist_begin: f32,
    pub twist_end: f32,
    pub radius_offset: f32,
    pub taper: [f32; 2],
    pub revolutions: f32,
    pub skew: f32,
}

impl Default for PathParams {
    fn default() -> Self {
        Self {
            curve_type: LL_PCODE_PATH_LINE,
            begin: 0.0,
            end: 1.0,
            scale: [1.0, 1.0],
            shear: [0.0, 0.0],
            twist_begin: 0.0,
            twist_end: 0.0,
            radius_offset: 0.0,
            taper: [0.0, 0.0],
            revolutions: 1.0,
            skew: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SculptParams {
    pub texture: uuid::Uuid,
    pub sculpt_type: u8,
}

impl SculptParams {
    pub fn kind(&self) -> u8 {
        self.sculpt_type & LL_SCULPT_TYPE_MASK
    }
    pub fn is_mesh(&self) -> bool {
        self.kind() == LL_SCULPT_TYPE_MESH
    }
    pub fn invert(&self) -> bool {
        self.sculpt_type & LL_SCULPT_FLAG_INVERT != 0
    }
    pub fn mirror(&self) -> bool {
        self.sculpt_type & LL_SCULPT_FLAG_MIRROR != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VolumeParams {
    pub profile: ProfileParams,
    pub path: PathParams,
    pub sculpt: Option<SculptParams>,
}

/// Raw quantized shape fields as sent in the full `ObjectUpdate` message.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawShape {
    pub path_curve: u8,
    pub profile_curve: u8,
    pub path_begin: u16,
    pub path_end: u16,
    pub path_scale_x: u8,
    pub path_scale_y: u8,
    pub path_shear_x: u8,
    pub path_shear_y: u8,
    pub path_twist: i8,
    pub path_twist_begin: i8,
    pub path_radius_offset: i8,
    pub path_taper_x: i8,
    pub path_taper_y: i8,
    pub path_revolutions: u8,
    pub path_skew: i8,
    pub profile_begin: u16,
    pub profile_end: u16,
    pub profile_hollow: u16,
}

impl RawShape {
    /// Dequantize (`LLVolumeMessage::unpackPathParams/unpackProfileParams`)
    /// and constrain the result.
    pub fn to_params(&self) -> VolumeParams {
        let path = PathParams {
            curve_type: self.path_curve,
            begin: self.path_begin as f32 * CUT_QUANTA,
            end: (50000u32.saturating_sub(self.path_end as u32)) as f32 * CUT_QUANTA,
            scale: [
                (200 - self.path_scale_x as i32) as f32 * SCALE_QUANTA,
                (200 - self.path_scale_y as i32) as f32 * SCALE_QUANTA,
            ],
            shear: [
                self.path_shear_x as i8 as f32 * SHEAR_QUANTA,
                self.path_shear_y as i8 as f32 * SHEAR_QUANTA,
            ],
            twist_end: self.path_twist as f32 * SCALE_QUANTA,
            twist_begin: self.path_twist_begin as f32 * SCALE_QUANTA,
            radius_offset: self.path_radius_offset as f32 * SCALE_QUANTA,
            taper: [self.path_taper_x as f32 * TAPER_QUANTA, self.path_taper_y as f32 * TAPER_QUANTA],
            revolutions: self.path_revolutions as f32 * REV_QUANTA + 1.0,
            skew: self.path_skew as f32 * SCALE_QUANTA,
        };
        let profile = ProfileParams {
            curve_type: self.profile_curve,
            begin: self.profile_begin as f32 * CUT_QUANTA,
            end: (50000u32.saturating_sub(self.profile_end as u32)) as f32 * CUT_QUANTA,
            hollow: self.profile_hollow as f32 * HOLLOW_QUANTA,
        };
        let mut v = VolumeParams {
            profile,
            path,
            sculpt: None,
        };
        v.constrain();
        v
    }
}

const MIN_CUT_DELTA: f32 = 0.02;
const MAX_HOLLOW: f32 = 0.99;

impl VolumeParams {
    /// Apply the clamping done by `LLVolumeParams`' checked setters.
    pub fn constrain(&mut self) {
        let p = &mut self.profile;
        p.begin = p.begin.clamp(0.0, 1.0 - MIN_CUT_DELTA);
        p.end = p.end.clamp(MIN_CUT_DELTA, 1.0);
        if p.begin > p.end - MIN_CUT_DELTA {
            p.begin = (p.end - MIN_CUT_DELTA).max(0.0);
        }
        p.hollow = p.hollow.clamp(0.0, MAX_HOLLOW);
        if !p.hollow.is_finite() {
            p.hollow = 0.0;
        }

        let t = &mut self.path;
        t.begin = t.begin.clamp(0.0, 1.0 - MIN_CUT_DELTA);
        t.end = t.end.clamp(MIN_CUT_DELTA, 1.0);
        if t.begin > t.end - MIN_CUT_DELTA {
            t.begin = (t.end - MIN_CUT_DELTA).max(0.0);
        }
        // Scale range differs between line and circular paths.
        let (min_s, max_s) = if t.curve_type == LL_PCODE_PATH_LINE || t.curve_type == LL_PCODE_PATH_FLEXIBLE {
            (0.0, 2.0)
        } else {
            (0.0, 1.0)
        };
        t.scale[0] = t.scale[0].clamp(min_s, max_s);
        t.scale[1] = t.scale[1].clamp(min_s, max_s);
        t.shear[0] = t.shear[0].clamp(-0.5, 0.5);
        t.shear[1] = t.shear[1].clamp(-0.5, 0.5);
        t.twist_begin = t.twist_begin.clamp(-1.0, 1.0);
        t.twist_end = t.twist_end.clamp(-1.0, 1.0);
        t.radius_offset = t.radius_offset.clamp(-1.0, 1.0);
        t.taper[0] = t.taper[0].clamp(-1.0, 1.0);
        t.taper[1] = t.taper[1].clamp(-1.0, 1.0);
        t.revolutions = t.revolutions.clamp(1.0, 4.0);
        t.skew = t.skew.clamp(-0.95, 0.95);
    }

    pub fn is_sculpt(&self) -> bool {
        self.sculpt.is_some_and(|s| s.kind() != LL_SCULPT_TYPE_NONE && !s.is_mesh())
    }

    pub fn is_mesh(&self) -> bool {
        self.sculpt.is_some_and(|s| s.is_mesh())
    }

    /// A hashable key for geometry caching (quantized params).
    pub fn cache_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let q = |f: f32| (f * 50000.0).round() as i32;
        self.profile.curve_type.hash(&mut h);
        q(self.profile.begin).hash(&mut h);
        q(self.profile.end).hash(&mut h);
        q(self.profile.hollow).hash(&mut h);
        let t = &self.path;
        t.curve_type.hash(&mut h);
        for f in [
            t.begin,
            t.end,
            t.scale[0],
            t.scale[1],
            t.shear[0],
            t.shear[1],
            t.twist_begin,
            t.twist_end,
            t.radius_offset,
            t.taper[0],
            t.taper[1],
            t.revolutions,
            t.skew,
        ] {
            q(f).hash(&mut h);
        }
        if let Some(s) = self.sculpt {
            s.texture.hash(&mut h);
            s.sculpt_type.hash(&mut h);
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_box_dequantizes() {
        let raw = RawShape {
            path_curve: LL_PCODE_PATH_LINE,
            profile_curve: LL_PCODE_PROFILE_SQUARE,
            path_scale_x: 100,
            path_scale_y: 100,
            ..Default::default()
        };
        let v = raw.to_params();
        assert!((v.path.scale[0] - 1.0).abs() < 1e-6);
        assert!((v.path.end - 1.0).abs() < 1e-6);
        assert!((v.profile.end - 1.0).abs() < 1e-6);
        assert!((v.path.revolutions - 1.0).abs() < 1e-6);
    }
}
