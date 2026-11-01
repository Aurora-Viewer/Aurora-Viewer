//! Deterministic offline water contacts, sent through the ordinary object path.

use crate::camera::Camera;
use crate::settings::Settings;
use crate::world::World;
use aurora_net::NetEvent;
use glam::{Quat, Vec3};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

pub const CONTACT_ANIM: Uuid = Uuid::from_u128(0x9950_2aa0_0000_0000_0000_0000_0000_0001);

pub fn mode() -> Option<&'static str> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_WATER").ok()).as_deref()
}

/// Fixed wave and actor phase for same-camera optical comparisons.
pub fn inspection_time(time: f32) -> f32 {
    if mode().is_some_and(|mode| mode.contains("optics-")) {
        8.0
    } else {
        time
    }
}

pub fn update(
    camera: &mut Camera,
    settings: &mut Settings,
    world: &mut World,
    panels: &mut crate::ui::Panels,
    frame: u64,
    time: f32,
) -> bool {
    let Some(mode) = mode() else {
        return false;
    };
    let time = inspection_time(time);
    let mut changed = false;
    static CONFIGURED: AtomicBool = AtomicBool::new(false);
    if !CONFIGURED.swap(true, Ordering::Relaxed) || frame == 720 {
        changed = true;
        // Keep the explicit water fixture clear of restored demo floaters;
        // reassert its view after the loading camera has finished.
        panels.chat = false;
        panels.people = false;
        panels.perf = false;
        panels.minimap = false;
        panels.inventory = false;
        panels.appearance = false;
        if !crate::ao::water_demo::mode().is_some_and(|m| m.starts_with("slots")) {
            panels.ao = false;
        }
        settings.water_effects.enabled = mode.split(':').next() != Some("off");
        settings.water_effects.waves = !mode.contains("no-waves");
        settings.water_effects.interactions = !mode.contains("no-wake") && !mode.contains("waterline") && !mode.contains("optics-");
        settings.water_effects.absorption = !mode.contains("no-absorption");
        settings.water_effects.fresnel = !mode.contains("no-fresnel");
        settings.water_effects.quality = match mode.split(':').next().unwrap_or("") {
            "low" => 0,
            "high" => 2,
            "ultra" => 3,
            _ => 1,
        };
        if mode.contains("parts-") {
            settings.water_effects.shore_foam = false;
            settings.water_effects.caustics = false;
        }
    }
    if crate::ao::water_demo::mode().is_none() && camera.free().is_none() {
        let (eye, target) = if mode.contains("exclusion") {
            (Vec3::new(48.0, 111.0, 28.0), Vec3::new(48.0, 127.0, 19.0))
        } else if mode.contains("parts-") {
            let x = if mode.contains("parts-feet") { 62.0 } else { 52.0 };
            if mode.contains("parts-feet") {
                (Vec3::new(x - 1.5, 122.2, 21.2), Vec3::new(x, 124.0, 20.05))
            } else {
                (Vec3::new(x - 2.4, 120.2, 21.9), Vec3::new(x, 124.0, 20.1))
            }
        } else if mode.contains("optics-") {
            let angle = mode
                .split(':')
                .find_map(|part| part.strip_prefix("optics-")?.parse::<f32>().ok())
                .unwrap_or(45.0)
                .clamp(0.0, 89.0)
                .to_radians();
            let eye = Vec3::new(48.0, 120.0, if mode.contains("near-eye") { 19.985 } else { 18.9 });
            let horizontal = glam::Vec2::new(12.0, 10.0).normalize() * angle.cos();
            (eye, eye + horizontal.extend(angle.sin()))
        } else if mode.contains("waterline-split") {
            let xy = glam::Vec2::new(43.0, 118.0);
            let level = aurora_render::water_effects::surface_level(xy, time, settings.water_effects.render(), 20.0, 0.0);
            let eye = xy.extend(level + if mode.contains("under") { -0.01 } else { 0.01 });
            (eye, eye + Vec3::new(12.0, 10.0, 0.0))
        } else if mode.contains("waterline") {
            // Keep a camera above native water under a crest, or below native
            // water in a trough. The old flat-plane switch inverted both views.
            let crest = !mode.contains("trough");
            let mut xy = glam::Vec2::new(12.0, 118.0);
            let sample = |p: glam::Vec2| aurora_render::water_effects::surface_level(p, time, settings.water_effects.render(), 20.0, 0.0);
            for i in 1..96 {
                let candidate = glam::Vec2::new(12.0 + i as f32 * 0.4, 118.0);
                if (crest && sample(candidate) > sample(xy)) || (!crest && sample(candidate) < sample(xy)) {
                    xy = candidate;
                }
            }
            let eye = xy.extend(if crest { 20.05 } else { 19.95 });
            if frame == 900 {
                log::info!("waterline fixture: camera_z={}, macro_level={}, native_level=20", eye.z, sample(xy));
            }
            (eye, eye + Vec3::new(12.0, 10.0, -1.5))
        } else if mode.contains("window-grazing") {
            (Vec3::new(54.0, 128.0, 18.9), Vec3::new(66.0, 143.0, 22.0))
        } else if mode.contains("window") {
            (Vec3::new(55.0, 128.0, 18.9), Vec3::new(59.0, 130.2, 24.0))
        } else if mode.contains("surface") {
            (Vec3::new(45.0, 124.0, 21.5), Vec3::new(49.0, 128.0, 20.0))
        } else if mode.contains("underwater-deep") {
            (Vec3::new(25.0, 110.0, 15.4), Vec3::new(38.0, 124.0, 15.7))
        } else if mode.contains("underwater") {
            (Vec3::new(43.0, 118.0, 18.7), Vec3::new(58.0, 130.0, 18.5))
        } else if mode.contains("foam") {
            (Vec3::new(61.0, 128.7, 22.1), Vec3::new(63.0, 130.0, 19.9))
        } else if mode.contains("avatar") {
            (Vec3::new(58.0, 120.0, 22.0), Vec3::new(60.0, 125.0, 19.7))
        } else if mode.contains("shore") {
            (Vec3::new(72.0, 133.0, 22.2), Vec3::new(48.0, 119.0, 19.4))
        } else if mode.contains("close") {
            (Vec3::new(48.0, 113.0, 21.6), Vec3::new(55.0, 128.0, 19.5))
        } else {
            (Vec3::new(39.0, 108.0, 25.0), Vec3::new(60.0, 131.0, 19.4))
        };
        camera.set_view(eye, target, &mut world.agent, &settings.camera);
        camera.position = eye;
        camera.target = target;
    }
    if mode.split(':').next() == Some("cycle") {
        let step = (frame / 60) % 13;
        let mut effects = crate::settings::WaterEffectsSettings {
            quality: (step % 4) as u8,
            ..Default::default()
        };
        match step {
            0 => effects.enabled = false,
            1 => effects.waves = false,
            2 => effects.absorption = false,
            3 => effects.fresnel = false,
            4 => effects.shore_foam = false,
            5 => effects.contact_foam = false,
            6 => effects.crest_foam = false,
            7 => effects.interactions = false,
            8 => effects.caustics = false,
            9 => effects.splashes = false,
            10 => effects.bubbles = false,
            11 => effects.wet_ground = false,
            _ => {}
        }
        if effects != settings.water_effects {
            log::info!(
                "water demo cycle: step {step}, quality {}, flags {}",
                effects.quality,
                effects.render().flags()
            );
            settings.water_effects = effects;
            changed = true;
        }
    }
    if !frame.is_multiple_of(6) {
        return changed;
    }
    let mut objects = Vec::new();
    let box_shape = super::shape(
        aurora_prim::params::LL_PCODE_PATH_LINE,
        aurora_prim::params::LL_PCODE_PROFILE_SQUARE,
        100,
        0,
        0,
    );
    if mode.contains("exclusion") {
        // Matched floors remain visible through an invisible face above water
        // (left) and below water (right). Only the left surface must hide water.
        for (id, x, z, scale, color) in [
            (99520, 44.0, 18.5, Vec3::new(6.0, 7.0, 0.3), [0.9, 0.12, 0.04, 1.0]),
            (99521, 52.0, 18.5, Vec3::new(6.0, 7.0, 0.3), [0.04, 0.7, 0.2, 1.0]),
            (99522, 44.0, 20.6, Vec3::new(6.0, 7.0, 0.1), [1.0; 4]),
            (99523, 52.0, 19.2, Vec3::new(6.0, 7.0, 0.1), [1.0; 4]),
        ] {
            let mut te = super::te(color, 0, true, 0.0);
            if id >= 99522 {
                for face in &mut Arc::make_mut(&mut te).faces {
                    face.texture = if mode.contains("exclusion-control") {
                        aurora_prim::te::TRANSPARENT_TEXTURE
                    } else {
                        aurora_prim::te::WATER_EXCLUSION_TEXTURE
                    };
                }
            }
            objects.push(super::prim(
                id,
                Vec3::new(x, 127.0, z),
                Quat::IDENTITY,
                scale,
                box_shape,
                te,
                Default::default(),
                "",
            ));
        }
        if let Some(handle) = world.main_region {
            world.apply(NetEvent::ObjectUpdates { handle, objects });
        }
        return changed;
    }
    for (id, pos, scale, color) in [
        (
            99500,
            Vec3::new(48.0 + 10.0 * (time * 0.28).cos(), 130.0 + 10.0 * (time * 0.28).sin(), 20.0),
            Vec3::new(6.0, 2.0, 0.8),
            [0.65, 0.22, 0.12, 1.0],
        ),
        (
            99501,
            Vec3::new(62.0, 132.0, 19.5),
            Vec3::new(1.2, 1.2, 4.0),
            [0.5, 0.45, 0.35, 1.0],
        ),
        // Colored above-water crossbar makes transmission through the underside
        // distinguishable from sky-only or opaque-fog fallbacks.
        (
            99505,
            Vec3::new(62.0, 132.0, 24.0),
            Vec3::new(4.0, 1.0, 1.0),
            [0.9, 0.18, 0.04, 1.0],
        ),
        (
            99506,
            Vec3::new(64.0, 132.0, 22.0),
            Vec3::new(0.4, 0.4, 4.0),
            [0.08, 0.25, 0.8, 1.0],
        ),
    ] {
        if id == 99500
            && (crate::ao::water_demo::mode().is_some()
                || mode.contains("parts-")
                || mode.contains("waterline")
                || mode.contains("optics-"))
        {
            continue;
        }
        objects.push(super::prim(
            id,
            pos,
            Quat::from_rotation_z(time * 0.28 + std::f32::consts::FRAC_PI_2),
            scale,
            box_shape,
            super::te(color, 0, false, 0.0),
            Default::default(),
            "",
        ));
    }
    if mode.contains("optics-") {
        for (id, x, z, color) in [
            (99510, 57.0, 22.5, [1.0, 0.02, 0.02, 1.0]),
            (99511, 60.0, 22.5, [0.02, 1.0, 0.02, 1.0]),
            (99512, 57.0, 25.5, [0.02, 0.02, 1.0, 1.0]),
            (99513, 60.0, 25.5, [1.0, 1.0, 0.02, 1.0]),
        ] {
            objects.push(super::prim(
                id,
                Vec3::new(x, 130.0, z),
                Quat::IDENTITY,
                Vec3::new(2.8, 0.25, 2.8),
                box_shape,
                super::te(color, 0, true, 0.0),
                Default::default(),
                "",
            ));
        }
    }
    if mode.contains("alpha") {
        // #1678: blended (semi-transparent) boxes under the water, across its
        // surface and above it. The one under it and the lower half of the
        // crossing one must be refracted and fogged by the water, not drawn
        // crisp over it; the upper half and the one above stay as they were.
        for (id, x, z, height, color) in [
            (99530, 52.0, 18.0, 2.0, [0.1, 0.9, 0.2, 0.6]),
            (99531, 55.0, 20.0, 4.0, [0.95, 0.85, 0.1, 0.6]),
            (99532, 58.0, 22.5, 1.5, [0.9, 0.1, 0.8, 0.6]),
        ] {
            objects.push(super::prim(
                id,
                Vec3::new(x, 127.0, z),
                Quat::IDENTITY,
                Vec3::new(1.5, 0.3, height),
                box_shape,
                super::te(color, 0, false, 0.0),
                Default::default(),
                "",
            ));
        }
    }
    for (id, pos) in [
        // Shin-deep wading: feet cross the surface during the fixture's gait.
        (
            99502,
            Vec3::new(
                60.0 + 2.0 * (time * if mode.contains("run") { 2.2 } else { 0.8 }).sin(),
                125.0,
                20.55,
            ),
        ),
        (99503, Vec3::new(49.0 + 4.0 * (time * 0.5).sin(), 128.0, 18.2)),
        (99504, Vec3::new(49.0 + 4.0 * (time * 0.5).sin(), 129.0, 20.05)),
    ] {
        if mode.contains("parts-") && id != 99502 {
            continue;
        }
        let pos = if mode.contains("parts-head") {
            Vec3::new(52.0, 124.0, 19.15 + 0.25 * (time * 2.5).sin())
        } else if mode.contains("parts-hands") {
            Vec3::new(52.0, 124.0, 19.8)
        } else if mode.contains("parts-feet") {
            Vec3::new(62.0, 124.0, 20.55)
        } else {
            pos
        };
        let mut avatar = super::action_avatar(false);
        avatar.local_id = id;
        avatar.full_id = Uuid::from_u128(id as u128);
        avatar.position = pos;
        if mode.contains("parts-")
            && let Some(te) = avatar.texture_entry.as_mut()
            && let Some(skirt) = Arc::make_mut(te).faces.get_mut(19)
        {
            skirt.texture = crate::scene::avatar::IMG_INVISIBLE;
        }
        if id == 99504 {
            avatar.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        }
        avatar.name_values = "FirstName STRING RW SV Water\nLastName STRING RW SV Demo".into();
        objects.push(avatar);
    }
    if let Some(handle) = world.main_region {
        world.apply(NetEvent::ObjectUpdates { handle, objects });
        static ANIMATED: AtomicBool = AtomicBool::new(false);
        if !ANIMATED.swap(true, Ordering::Relaxed) {
            for id in [99502u128, 99503, 99504] {
                world.apply(NetEvent::AvatarAnimations {
                    sources: Vec::new(),
                    avatar: Uuid::from_u128(id),
                    anims: vec![(CONTACT_ANIM, 1)],
                });
            }
        }
    }
    changed
}

/// Local fixture: moving limbs exercise contact emission even without downloaded animations.
pub fn animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, RotKey};
    let mut animation = super::idle_animation();
    let running = mode().is_some_and(|m| m.contains("run")) || crate::ao::water_demo::mode() == Some("wade-run");
    animation.base_priority = 4;
    animation.duration = if running { 0.8 } else { 1.2 };
    animation.loop_in = 0.0;
    animation.loop_out = animation.duration;
    animation.looping = true;
    animation.joints.clear();
    if mode().is_some_and(|m| m.contains("parts-hands")) {
        for (name, sign, elbow) in [
            ("mShoulderLeft", -1.0, false),
            ("mShoulderRight", 1.0, false),
            ("mElbowLeft", -1.0, true),
            ("mElbowRight", 1.0, true),
        ] {
            let rot_keys = (0..=8)
                .map(|i| {
                    let phase = i as f32 * std::f32::consts::TAU / 8.0;
                    let rotation = if elbow {
                        Quat::from_rotation_z(sign * (0.2 + phase.sin() * 0.55))
                    } else {
                        Quat::from_rotation_x(sign * (1.15 + phase.cos() * 0.28))
                    };
                    RotKey {
                        time: i as f32 * 0.15,
                        rotation,
                    }
                })
                .collect();
            animation.joints.push(JointMotion {
                joint_name: name.into(),
                rot_keys,
                pos_keys: Vec::new(),
                priority: 4,
            });
        }
        return animation;
    }
    if mode().is_some_and(|m| m.contains("parts-head")) {
        return animation;
    }
    for (name, phase, kind) in [
        ("mHipLeft", 0.0, 0),
        ("mHipRight", std::f32::consts::PI, 0),
        ("mKneeLeft", 0.0, 1),
        ("mKneeRight", std::f32::consts::PI, 1),
        ("mShoulderLeft", 0.0, 2),
        ("mShoulderRight", std::f32::consts::PI, 3),
    ] {
        let rot_keys = (0..=4)
            .map(|i| {
                let wave = (i as f32 * std::f32::consts::FRAC_PI_2 + phase).sin();
                let rotation = match kind {
                    0 => Quat::from_rotation_y(wave * if running { 0.65 } else { 0.45 }),
                    1 => Quat::from_rotation_y(wave.max(0.0) * 0.8),
                    2 => Quat::from_rotation_x(-1.2) * Quat::from_rotation_y(wave * 0.25),
                    _ => Quat::from_rotation_x(1.2) * Quat::from_rotation_y(wave * 0.25),
                };
                RotKey {
                    time: i as f32 * animation.duration / 4.0,
                    rotation,
                }
            })
            .collect();
        animation.joints.push(JointMotion {
            joint_name: name.into(),
            priority: 4,
            rot_keys,
            pos_keys: Vec::new(),
        });
    }
    animation
}
