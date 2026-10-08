//! Quantizing volume parameters for the wire (`LLVolumeMessage::packPathParams`
//! / `packProfileParams`), used by ObjectAdd and ObjectShape.

use crate::params::*;

fn q(v: f32, quanta: f32) -> i32 {
    (v / quanta).round() as i32
}

impl VolumeParams {
    /// The quantized fields of this shape (inverse of [`RawShape::to_params`]).
    pub fn to_raw(&self) -> RawShape {
        let p = &self.path;
        let f = &self.profile;
        RawShape {
            path_curve: p.curve_type,
            profile_curve: f.curve_type,
            path_begin: q(p.begin, CUT_QUANTA).clamp(0, 50000) as u16,
            path_end: (50000 - q(p.end, CUT_QUANTA)).clamp(0, 50000) as u16,
            path_scale_x: (200 - q(p.scale[0], SCALE_QUANTA)).clamp(0, 255) as u8,
            path_scale_y: (200 - q(p.scale[1], SCALE_QUANTA)).clamp(0, 255) as u8,
            path_shear_x: q(p.shear[0], SHEAR_QUANTA).clamp(-128, 127) as i8 as u8,
            path_shear_y: q(p.shear[1], SHEAR_QUANTA).clamp(-128, 127) as i8 as u8,
            path_twist: q(p.twist_end, SCALE_QUANTA).clamp(-128, 127) as i8,
            path_twist_begin: q(p.twist_begin, SCALE_QUANTA).clamp(-128, 127) as i8,
            path_radius_offset: q(p.radius_offset, SCALE_QUANTA).clamp(-128, 127) as i8,
            path_taper_x: q(p.taper[0], TAPER_QUANTA).clamp(-128, 127) as i8,
            path_taper_y: q(p.taper[1], TAPER_QUANTA).clamp(-128, 127) as i8,
            path_revolutions: q(p.revolutions - 1.0, REV_QUANTA).clamp(0, 255) as u8,
            path_skew: q(p.skew, SCALE_QUANTA).clamp(-128, 127) as i8,
            profile_begin: q(f.begin, CUT_QUANTA).clamp(0, 50000) as u16,
            profile_end: (50000 - q(f.end, CUT_QUANTA)).clamp(0, 50000) as u16,
            profile_hollow: q(f.hollow, HOLLOW_QUANTA).clamp(0, 50000) as u16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let raw = RawShape {
            path_curve: LL_PCODE_PATH_CIRCLE,
            profile_curve: LL_PCODE_PROFILE_SQUARE,
            path_begin: 1000,
            path_end: 2000,
            path_scale_x: 100,
            path_scale_y: 175,
            path_shear_x: (-50i8) as u8,
            path_twist: 20,
            path_taper_x: -30,
            path_revolutions: 10,
            profile_begin: 12500,
            profile_end: 12500,
            profile_hollow: 25000,
            ..Default::default()
        };
        let back = raw.to_params().to_raw();
        assert_eq!(back.path_begin, 1000);
        assert_eq!(back.path_end, 2000);
        assert_eq!(back.path_scale_y, 175);
        assert_eq!(back.path_shear_x, 206);
        assert_eq!(back.path_twist, 20);
        assert_eq!(back.path_taper_x, -30);
        assert_eq!(back.path_revolutions, 10);
        assert_eq!(back.profile_begin, 12500);
        assert_eq!(back.profile_end, 12500);
        assert_eq!(back.profile_hollow, 25000);
    }
}
