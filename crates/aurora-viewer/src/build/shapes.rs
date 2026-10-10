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

pub fn shapes() -> [Shape; 13] {
    [
        shape(
            aurora_i18n::tr!("build-shapes-cube"),
            LL_PCODE_PROFILE_SQUARE,
            LL_PCODE_PATH_LINE,
            (100, 100),
        ),
        Shape {
            shear_x: -50,
            ..shape(
                aurora_i18n::tr!("build-shapes-right-prism"),
                LL_PCODE_PROFILE_SQUARE,
                LL_PCODE_PATH_LINE,
                (200, 100),
            )
        },
        shape(
            aurora_i18n::tr!("build-shapes-pyramid"),
            LL_PCODE_PROFILE_SQUARE,
            LL_PCODE_PATH_LINE,
            (200, 200),
        ),
        shape(
            aurora_i18n::tr!("build-shapes-tetrahedron"),
            LL_PCODE_PROFILE_EQUALTRI,
            LL_PCODE_PATH_LINE,
            (200, 200),
        ),
        shape(
            aurora_i18n::tr!("common-cylinder"),
            LL_PCODE_PROFILE_CIRCLE,
            LL_PCODE_PATH_LINE,
            (100, 100),
        ),
        Shape {
            profile_cut: (12500, 12500),
            ..shape(
                aurora_i18n::tr!("build-shapes-half-cylinder"),
                LL_PCODE_PROFILE_CIRCLE,
                LL_PCODE_PATH_LINE,
                (100, 100),
            )
        },
        shape(
            aurora_i18n::tr!("build-shapes-cone"),
            LL_PCODE_PROFILE_CIRCLE,
            LL_PCODE_PATH_LINE,
            (200, 200),
        ),
        Shape {
            profile_cut: (12500, 12500),
            ..shape(
                aurora_i18n::tr!("build-shapes-half-cone"),
                LL_PCODE_PROFILE_CIRCLE,
                LL_PCODE_PATH_LINE,
                (200, 200),
            )
        },
        Shape {
            turned: true,
            ..shape(
                aurora_i18n::tr!("common-sphere"),
                LL_PCODE_PROFILE_CIRCLE_HALF,
                LL_PCODE_PATH_CIRCLE,
                (100, 100),
            )
        },
        Shape {
            path_end: 25000,
            ..shape(
                aurora_i18n::tr!("build-shapes-half-sphere"),
                LL_PCODE_PROFILE_CIRCLE_HALF,
                LL_PCODE_PATH_CIRCLE,
                (100, 100),
            )
        },
        Shape {
            turned: true,
            ..shape(
                aurora_i18n::tr!("common-torus"),
                LL_PCODE_PROFILE_CIRCLE,
                LL_PCODE_PATH_CIRCLE,
                (100, 175),
            )
        },
        Shape {
            turned: true,
            ..shape(
                aurora_i18n::tr!("common-tube"),
                LL_PCODE_PROFILE_SQUARE,
                LL_PCODE_PATH_CIRCLE,
                (100, 175),
            )
        },
        Shape {
            turned: true,
            ..shape(
                aurora_i18n::tr!("common-ring"),
                LL_PCODE_PROFILE_EQUALTRI,
                LL_PCODE_PATH_CIRCLE,
                (100, 175),
            )
        },
    ]
}

/// Species of trees.xml / grass.xml, in species id order: the Create
/// tool's tree / grass combo (FIRE-7802), the id goes in ObjectAdd State.
pub fn tree_species() -> [&'static str; 21] {
    [
        aurora_i18n::tr!("extra-pine-1"),
        aurora_i18n::tr!("extra-oak"),
        aurora_i18n::tr!("extra-tropical-bush-1"),
        aurora_i18n::tr!("extra-palm-1"),
        aurora_i18n::tr!("extra-dogwood"),
        aurora_i18n::tr!("extra-tropical-bush-2"),
        aurora_i18n::tr!("extra-palm-2"),
        aurora_i18n::tr!("extra-cypress-1"),
        aurora_i18n::tr!("extra-cypress-2"),
        aurora_i18n::tr!("extra-pine-2"),
        aurora_i18n::tr!("extra-plumeria"),
        aurora_i18n::tr!("extra-winter-pine-1"),
        aurora_i18n::tr!("extra-winter-aspen"),
        aurora_i18n::tr!("extra-winter-pine-2"),
        aurora_i18n::tr!("extra-eucalyptus"),
        aurora_i18n::tr!("extra-fern"),
        aurora_i18n::tr!("extra-eelgrass"),
        aurora_i18n::tr!("extra-sea-sword"),
        aurora_i18n::tr!("extra-kelp-1"),
        aurora_i18n::tr!("extra-beach-grass-1"),
        aurora_i18n::tr!("extra-kelp-2"),
    ]
}
pub fn grass_species() -> [&'static str; 6] {
    [
        aurora_i18n::tr!("extra-grass-0"),
        aurora_i18n::tr!("extra-grass-1"),
        aurora_i18n::tr!("extra-grass-2"),
        aurora_i18n::tr!("extra-grass-3"),
        aurora_i18n::tr!("extra-grass-4"),
        "undergrowth_1",
    ]
}

/// get_selected_plant (lltoolplacer.cpp): the named species, else a random one.
pub fn plant_species(list: &[&str], name: &str, random: u32) -> u8 {
    list.iter().position(|n| *n == name).unwrap_or(random as usize % list.len().max(1)) as u8
}

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
        let prism = shapes()[1].params();
        assert!((prism.path.scale[0] - 0.0).abs() < 1e-5);
        assert!((prism.path.shear[0] + 0.5).abs() < 1e-5);
        let hemi = shapes()[5].params();
        assert!((hemi.profile.begin - 0.25).abs() < 1e-5);
        assert!((hemi.profile.end - 0.75).abs() < 1e-5);
        let hs = shapes()[9].params();
        assert!((hs.path.end - 0.5).abs() < 1e-5);
        let torus = shapes()[10].params();
        assert!((torus.path.scale[1] - 0.25).abs() < 1e-5);
    }
}
