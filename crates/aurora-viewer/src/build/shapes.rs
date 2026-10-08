//! Shapes of the Create tool, with the parameters LLToolPlacer::addObject
//! sends for each (lltoolplacer.cpp; quantized as in ObjectAdd).

use aurora_prim::RawShape;
use aurora_prim::params::*;
use glam::Quat;

pub struct Shape {
    pub name: &'static str,
    profile: u8,
    path: u8,
    scale: (u8, u8),
    shear_x: i8,
    profile_cut: (u16, u16),
    path_end: u16,
    /// Turned 90° about Y (sphere and tori).
    turned: bool,
}

const fn shape(name: &'static str, profile: u8, path: u8, scale: (u8, u8)) -> Shape {
    Shape {
        name,
        profile,
        path,
        scale,
        shear_x: 0,
        profile_cut: (0, 0),
        path_end: 0,
        turned: false,
    }
}

pub const SHAPES: [Shape; 13] = [
    shape("Cube", LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_LINE, (100, 100)),
    Shape {
        shear_x: -50,
        ..shape("Prisme", LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_LINE, (200, 100))
    },
    shape("Pyramide", LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_LINE, (200, 200)),
    shape("Tétraèdre", LL_PCODE_PROFILE_EQUALTRI, LL_PCODE_PATH_LINE, (200, 200)),
    shape("Cylindre", LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_LINE, (100, 100)),
    Shape {
        profile_cut: (12500, 12500),
        ..shape("Demi-cylindre", LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_LINE, (100, 100))
    },
    shape("Cône", LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_LINE, (200, 200)),
    Shape {
        profile_cut: (12500, 12500),
        ..shape("Demi-cône", LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_LINE, (200, 200))
    },
    Shape {
        turned: true,
        ..shape("Sphère", LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PATH_CIRCLE, (100, 100))
    },
    Shape {
        path_end: 25000,
        ..shape("Demi-sphère", LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PATH_CIRCLE, (100, 100))
    },
    Shape {
        turned: true,
        ..shape("Tore", LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_CIRCLE, (100, 175))
    },
    Shape {
        turned: true,
        ..shape("Tube", LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_CIRCLE, (100, 175))
    },
    Shape {
        turned: true,
        ..shape("Anneau", LL_PCODE_PROFILE_EQUALTRI, LL_PCODE_PATH_CIRCLE, (100, 175))
    },
];

impl Shape {
    pub fn raw(&self) -> RawShape {
        RawShape {
            path_curve: self.path,
            profile_curve: self.profile,
            path_scale_x: self.scale.0,
            path_scale_y: self.scale.1,
            path_shear_x: self.shear_x as u8,
            profile_begin: self.profile_cut.0,
            profile_end: self.profile_cut.1,
            path_end: self.path_end,
            ..Default::default()
        }
    }

    pub fn rotation(&self) -> Quat {
        if self.turned {
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        }
    }

    /// The shape as volume parameters (for the icon preview).
    pub fn params(&self) -> VolumeParams {
        self.raw().to_params()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prism_and_hemi_shapes() {
        let prism = SHAPES[1].params();
        assert!((prism.path.scale[0] - 0.0).abs() < 1e-5);
        assert!((prism.path.shear[0] + 0.5).abs() < 1e-5);
        let hemi = SHAPES[5].params();
        assert!((hemi.profile.begin - 0.25).abs() < 1e-5);
        assert!((hemi.profile.end - 0.75).abs() < 1e-5);
        let hs = SHAPES[9].params();
        assert!((hs.path.end - 0.5).abs() < 1e-5);
        let torus = SHAPES[10].params();
        assert!((torus.path.scale[1] - 0.25).abs() < 1e-5);
    }
}
