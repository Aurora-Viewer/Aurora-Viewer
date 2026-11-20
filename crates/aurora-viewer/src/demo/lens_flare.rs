//! Offline scene with ordinary HDR emitters and view-dependent reflections.

use std::sync::LazyLock;

use aurora_assets::PbrMaterial;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::params::{LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PROFILE_SQUARE};
use aurora_prim::{ExtraParams, LightParams};
use aurora_render::lens_flare::Quality;
use glam::{Quat, Vec3};
use uuid::Uuid;

use super::{floor_at, prim, shape, te};
use crate::settings::lens_flare::BuiltIn;

const EMITTER: Uuid = Uuid::from_u128(0xde40_1e45_0001_0000_0000_0000_0000_0001);
const POLISHED: Uuid = Uuid::from_u128(0xde40_1e45_0001_0000_0000_0000_0000_0002);

#[derive(Clone, Copy)]
pub struct Scenario {
    pub preset: BuiltIn,
    pub quality: Quality,
    pub enabled: bool,
    pub panel: bool,
    pub advanced: bool,
    pub threshold: Option<f32>,
    pub blur: Option<f32>,
    sweep: bool,
    occluded: bool,
    dark: bool,
}

pub fn scenario() -> Option<&'static Scenario> {
    static SCENE: LazyLock<Option<Scenario>> = LazyLock::new(|| {
        if std::env::var("AURORA_DEMO").as_deref() != Ok("1") {
            return None;
        }
        let setting = std::env::var("AURORA_DEMO_LENS_FLARE").ok()?;
        let mut parts = setting.split(':');
        let name = parts.next()?;
        let preset = match name {
            "subtle" | "off" | "occluded" | "dark" => BuiltIn::SubtleOptical,
            "golden" => BuiltIn::GoldenHour,
            "blue" => BuiltIn::BlueAnamorphic,
            "star" => BuiltIn::StarFilter,
            _ => return None,
        };
        let mut scene = Scenario {
            preset,
            quality: Quality::Balanced,
            enabled: name != "off",
            panel: false,
            advanced: false,
            threshold: None,
            blur: None,
            sweep: false,
            occluded: name == "occluded",
            dark: name == "dark",
        };
        for part in parts {
            match part {
                "performance" => scene.quality = Quality::Performance,
                "balanced" => scene.quality = Quality::Balanced,
                "high" => scene.quality = Quality::High,
                "panel" => scene.panel = true,
                "advanced" => scene.advanced = true,
                "sweep" => scene.sweep = true,
                _ => match part.split_once('=')? {
                    ("threshold", value) => scene.threshold = Some(value.parse().ok()?),
                    ("blur", value) => scene.blur = Some(value.parse().ok()?),
                    _ => return None,
                },
            }
        }
        Some(scene)
    });
    SCENE.as_ref()
}

fn origin() -> Vec3 {
    let (x, y) = (205.0, 181.0);
    Vec3::new(x, y, floor_at(x, y) + 4.0)
}

impl Scenario {
    pub fn camera(self, frame: u64) -> (Vec3, Vec3) {
        let pan = if self.sweep && !self.occluded {
            (((frame % 720) as f32 - 180.0) * std::f32::consts::TAU / 720.0).sin()
        } else {
            0.0
        };
        let origin = origin();
        (origin + Vec3::new(-8.0, pan * 1.8, 0.0), origin + Vec3::new(0.0, pan * 4.4, -0.10))
    }
}

pub fn materials() -> [(Uuid, PbrMaterial); 2] {
    [
        (
            EMITTER,
            PbrMaterial {
                base_color_factor: [0.15, 0.15, 0.15, 1.0],
                emissive_factor: [16.0, 12.0, 8.0],
                metallic_factor: 0.0,
                ..Default::default()
            },
        ),
        (
            POLISHED,
            PbrMaterial {
                base_color_factor: [0.85, 0.88, 0.95, 1.0],
                metallic_factor: 1.0,
                roughness_factor: 0.13,
                ..Default::default()
            },
        ),
    ]
}

pub fn objects(scene: Scenario, first: u32) -> Vec<ObjectUpdate> {
    let origin = origin();
    let box_shape = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let mut objects = Vec::new();
    let mut add = |offset, scale, shape, color, extra, fullbright| {
        objects.push(prim(
            first + objects.len() as u32,
            origin + offset,
            Quat::IDENTITY,
            scale,
            shape,
            te(color, 0, fullbright, 0.0),
            extra,
            "",
        ));
    };
    let charcoal = [0.075, 0.084, 0.10, 1.0];
    add(
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(0.3, 32.0, 17.0),
        box_shape,
        charcoal,
        ExtraParams::default(),
        true,
    );
    add(
        Vec3::new(0.0, 0.0, -2.15),
        Vec3::new(17.0, 32.0, 0.3),
        box_shape,
        [0.13, 0.15, 0.18, 1.0],
        ExtraParams::default(),
        true,
    );
    let lamp = Vec3::new(1.0, 2.50, 0.65);
    let materials = |mat| ExtraParams {
        render_materials: (0..6).map(|face| (face, mat)).collect(),
        ..Default::default()
    };
    if !scene.dark {
        add(lamp, Vec3::splat(0.60), sphere, [1.0; 4], materials(EMITTER), false);
    }
    if scene.occluded {
        let eye = scene.camera(0).0 - origin;
        add(
            eye.lerp(lamp, 0.70),
            Vec3::new(0.20, 1.3, 1.15),
            box_shape,
            charcoal,
            ExtraParams::default(),
            true,
        );
    }
    add(
        Vec3::new(0.7, -2.10, -0.80),
        Vec3::splat(2.35),
        sphere,
        [1.0; 4],
        materials(POLISHED),
        false,
    );
    if !scene.dark && !scene.occluded {
        add(
            Vec3::new(-3.8, -2.6, 1.35),
            Vec3::splat(0.18),
            sphere,
            [1.0, 1.0, 1.0, 0.0],
            ExtraParams {
                light: Some(LightParams {
                    color: [1.0, 1.0, 1.0, 1.0],
                    radius: 8.0,
                    cutoff: 0.0,
                    falloff: 0.5,
                }),
                ..Default::default()
            },
            false,
        );
    }
    objects
}
