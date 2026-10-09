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
        ..shape("Prisme droit", LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_LINE, (200, 100))
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

/// Species of trees.xml / grass.xml, in species id order: the Create
/// tool's tree / grass combo (FIRE-7802), the id goes in ObjectAdd State.
pub const TREE_SPECIES: [&str; 21] = [
    "Pine 1",
    "Oak",
    "Tropical Bush 1",
    "Palm 1",
    "Dogwood",
    "Tropical Bush 2",
    "Palm 2",
    "Cypress 1",
    "Cypress 2",
    "Pine 2",
    "Plumeria",
    "Winter Pine 1",
    "Winter Aspen",
    "Winter Pine 2",
    "Eucalyptus",
    "Fern",
    "Eelgrass",
    "Sea Sword",
    "Kelp 1",
    "Beach Grass 1",
    "Kelp 2",
];
pub const GRASS_SPECIES: [&str; 6] = ["Grass 0", "Grass 1", "Grass 2", "Grass 3", "Grass 4", "undergrowth_1"];

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
