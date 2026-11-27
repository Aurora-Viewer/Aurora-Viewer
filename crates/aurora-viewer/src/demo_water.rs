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
/// The `sea` modes' reef: a mesh whose shape is a thin wall running corner
/// to corner across its box (scene/water_effects.rs `install_demo_reef`),
/// nothing like its box: the sea grid's shore map shelters along the wall
/// only (aurora_render::water_shore).
pub const SEA_REEF_MESH: Uuid = Uuid::from_u128(0x9950_2aa0_0000_0000_0000_0000_0000_0002);

/// Once, in a `sea` mode: whether the reef's geometry is to be installed now.
pub fn reef_to_install() -> bool {
    static DONE: AtomicBool = AtomicBool::new(false);
    mode().is_some_and(|m| m.contains("sea")) && !DONE.swap(true, Ordering::Relaxed)
}

pub fn mode() -> Option<&'static str> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_WATER").ok()).as_deref()
}

/// The `sea` modes' `AURORA_DEMO_SEA_PRESET=<calm|breeze|swell|storm|stylized>`:
/// the Sea panel's preset, set there; and `AURORA_DEMO_SEA_SHAPE=<wind m/s>,
/// <swell>,<chop>,<crests>,<foam>,<spray>` (scales: 1 is 100 %), its
/// sliders set to those (on the preset, or on the default shape), to
/// reproduce a sea someone tuned on the grid.
fn sea_preset() -> Option<crate::weather::sea::SeaShape> {
    mode().filter(|m| m.contains("sea"))?;
    let preset = std::env::var("AURORA_DEMO_SEA_PRESET")
        .ok()
        .and_then(|p| crate::weather::sea::preset(p.trim()));
    let shape = std::env::var("AURORA_DEMO_SEA_SHAPE").ok().map(|v| {
        let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let mut shape = preset.clone().unwrap_or_default();
        let at = |i: usize, or: f32| n.get(i).copied().filter(|x| x.is_finite()).unwrap_or(or);
        shape.state = at(0, shape.state * crate::weather::sea::MAX_WIND) / crate::weather::sea::MAX_WIND;
        shape.swell = at(1, shape.swell);
        shape.chop = at(2, shape.chop);
        shape.choppiness = at(3, shape.choppiness);
        shape.foam = at(4, shape.foam);
        shape.spray = at(5, shape.spray);
        shape
    });
    shape.or(preset)
}

/// Once: `AURORA_DEMO_SEA_PRESET` sets the sea (set here, that preset),
/// and `sea-panel` opens the Aurora Weather window on its Sea section.
/// Whether to open it now.
pub fn sea_panel_setup(settings: &mut Settings) -> bool {
    static DONE: AtomicBool = AtomicBool::new(false);
    if mode().is_none() || DONE.swap(true, Ordering::Relaxed) {
        return false;
    }
    if let Some(shape) = sea_preset() {
        settings.sea.source = crate::weather::sea::SeaSource::Mine;
        settings.sea.shape = shape;
    }
    mode().is_some_and(|m| m.contains("sea-panel"))
}

/// The wind over the demo's water (m/s, blowing toward). The `sea` modes
/// take it from their panel (`sea_panel`) or sweep it (`sea-sweep`); the
/// other fixtures keep the light breeze they were made under.
pub fn sea_wind() -> Option<glam::Vec2> {
    let mode = mode()?;
    if !mode.contains("sea") {
        return Some(glam::Vec2::new(0.88, 0.475) * 5.0);
    }
    let (speed, from) = if mode.contains("sea-sweep") {
        (sweep_speed(sea_clock().elapsed().as_secs_f32()), sea_controls().1)
    } else if mode.contains("sea-turn") {
        // the wind going round, 3° a second (the region's wind on the grid
        // often turns like this): the sea must follow without a jump
        let (speed, from) = sea_controls();
        (speed, from + 3.0 * sea_clock().elapsed().as_secs_f32())
    } else {
        sea_controls()
    };
    // compass degrees clockwise from north (+y), the way it comes from
    let from = from.to_radians();
    Some(-glam::Vec2::new(from.sin(), from.cos()) * speed)
}

/// `sea-sweep`: calm, a breeze, a gale and back over two minutes, holding
/// each for a while so a recording shows the sea build and settle.
pub fn sweep_speed(t: f32) -> f32 {
    const KEYS: [(f32, f32); 7] = [
        (0.0, 0.0),
        (10.0, 0.0),
        (30.0, 6.0),
        (45.0, 6.0),
        (70.0, 20.0),
        (95.0, 20.0),
        (120.0, 0.0),
    ];
    let t = t.rem_euclid(KEYS[KEYS.len() - 1].0);
    KEYS.windows(2)
        .find(|w| t <= w[1].0)
        .map_or(0.0, |w| w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) / (w[1].0 - w[0].0))
}

fn sea_clock() -> &'static std::time::Instant {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now)
}

/// The panel's wind: speed (m/s) and where it comes from (degrees), at
/// first `AURORA_DEMO_SEA_WIND=<speed>[,<from>]` (6 m/s from the west: onto
/// the demo's beach).
fn sea_controls() -> (f32, f32) {
    *sea_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn sea_state() -> &'static std::sync::Mutex<(f32, f32)> {
    static STATE: OnceLock<std::sync::Mutex<(f32, f32)>> = OnceLock::new();
    STATE.get_or_init(|| {
        let spec = std::env::var("AURORA_DEMO_SEA_WIND").unwrap_or_default();
        let mut parts = spec.split(',').map(|p| p.trim().parse::<f32>().ok());
        let speed = parts.next().flatten().unwrap_or(6.0).clamp(0.0, 30.0);
        let from = parts.next().flatten().unwrap_or(270.0).rem_euclid(360.0);
        std::sync::Mutex::new((speed, from))
    })
}

/// The `sea` modes' panel: the wind's speed and direction, and the sea it
/// makes (demo text, not translated, like the other demo tools).
pub fn sea_panel(ctx: &egui::Context, sea: &aurora_render::water_sea::Sea) {
    if !mode().is_some_and(|m| m.contains("sea")) {
        return;
    }
    let sweep = mode().is_some_and(|m| m.contains("sea-sweep"));
    egui::Window::new("Sea wind (demo)")
        .id(egui::Id::unique("demo_sea_wind"))
        .default_pos(egui::pos2(16.0, 80.0))
        .resizable(false)
        .show(ctx, |ui| {
            let mut state = sea_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            ui.add_enabled(!sweep, egui::Slider::new(&mut state.0, 0.0..=25.0).text("wind (m/s)"));
            ui.add(egui::Slider::new(&mut state.1, 0.0..=359.0).text("from (°)"));
            let [_, _, speed, _] = sea.wind;
            ui.label(format!(
                "sea: {speed:.1} m/s (Beaufort {}), roughness {:.2}, swell {:.0} m long, {:.2} m high",
                beaufort(speed),
                sea.frame[3],
                std::f32::consts::TAU / sea.trains[0][2].max(1e-3),
                2.0 * sea.trains[0][3]
            ));
        });
}

fn beaufort(speed: f32) -> u32 {
    [0.5, 1.6, 3.4, 5.5, 8.0, 10.8, 13.9, 17.2, 20.8, 24.5, 28.5, 32.7]
        .iter()
        .take_while(|limit| speed >= **limit)
        .count() as u32
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
    sea: &aurora_render::water_sea::Sea,
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
        // the fixtures' sea is the demo's wind as it is: what a `:sea-panel`
        // run left in the demo's settings (a preset set here) is undone
        if !mode.contains("sea-panel") && sea_preset().is_none() {
            settings.sea = Default::default();
        }
        settings.water_effects.interactions = !mode.contains("no-wake") && !mode.contains("waterline") && !mode.contains("optics-");
        settings.water_effects.absorption = !mode.contains("no-absorption");
        settings.water_effects.fresnel = !mode.contains("no-fresnel");
        settings.water_effects.wet_avatars = !mode.contains("no-wet");
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
        let (eye, target) = if mode.contains("deep-lights") {
            // Looking down at the deep floor of the `deep-lights` set, 48 m
            // under the water line, its lamp in the middle.
            (Vec3::new(32.0, 92.0, -21.0), Vec3::new(40.0, 100.0, -27.6))
        } else if mode.contains("glint") {
            // Looking down 56 degrees at the lagoon from 12 m above it,
            // toward `glint-<heading>` (degrees from east, 45 by default:
            // the sun of AURORA_DEMO_TOD=5), at the sun's glint on the ripples.
            let heading = mode
                .split(':')
                .find_map(|part| part.strip_prefix("glint-")?.parse::<f32>().ok())
                .unwrap_or(45.0)
                .to_radians();
            let target = Vec3::new(40.0, 112.0, 20.0);
            let back = glam::Vec2::new(heading.cos(), heading.sin()) * 8.1;
            (target - back.extend(-12.0), target)
        } else if mode.contains("caustics") {
            // Under the lagoon, down at its sunny bed 2 to 4 m deep.
            (Vec3::new(28.0, 110.0, 19.2), Vec3::new(37.0, 119.0, 15.8))
        } else if mode.contains("puddles") {
            // Down at the plaza's floor south of the arrival, 3 m away:
            // with AURORA_RAIN its puddles, with AURORA_SNOW_AFTER their ice.
            (Vec3::new(128.0, 118.0, 34.6), Vec3::new(132.0, 123.5, 32.0))
        } else if mode.contains("rain-sea") {
            // Across the lagoon from 2.5 m above it: with AURORA_RAIN, the
            // rain on open water.
            (Vec3::new(30.0, 104.0, 22.5), Vec3::new(50.0, 124.0, 19.5))
        } else if mode.contains("exclusion") {
            (Vec3::new(48.0, 111.0, 28.0), Vec3::new(48.0, 127.0, 19.0))
        } else if mode.contains("alpha-layers") || mode.contains("alpha-cross") {
            (Vec3::new(46.0, 117.0, 24.0), Vec3::new(56.0, 130.0, 19.5))
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
            let level = aurora_render::water_effects::surface_level(
                xy,
                time,
                settings.water_effects.render(),
                sea,
                20.0,
                &aurora_render::water_sea::SeaPoint::open(20.0),
            );
            let eye = xy.extend(level + if mode.contains("under") { -0.01 } else { 0.01 });
            (eye, eye + Vec3::new(12.0, 10.0, 0.0))
        } else if mode.contains("waterline") {
            // Keep a camera above native water under a crest, or below native
            // water in a trough. The old flat-plane switch inverted both views.
            let crest = !mode.contains("trough");
            let mut xy = glam::Vec2::new(12.0, 118.0);
            let sample = |p: glam::Vec2| {
                aurora_render::water_effects::surface_level(
                    p,
                    time,
                    settings.water_effects.render(),
                    sea,
                    20.0,
                    &aurora_render::water_sea::SeaPoint::open(20.0),
                )
            };
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
        } else if mode.contains("wet-close") {
            // close on the sealed suit (the second bather), for its beads
            (Vec3::new(71.4, 123.9, 21.9), Vec3::new(70.0, 123.0, 21.5))
        } else if mode.contains("wet") {
            // the wet look's bathers (`wet_bather`), side by side on the
            // beach after their soak, from the sunny side
            (Vec3::new(74.5, 129.0, 22.4), Vec3::new(70.0, 125.0, 21.5))
        } else if mode.contains("sea-floor") {
            // low over the floor laid at the water
            (Vec3::new(58.0, 106.0, 21.6), Vec3::new(48.0, 116.0, 19.9))
        } else if mode.contains("sea-lakeshore") {
            // over the inland lake, onto its downwind (east) shore with the
            // default west wind: the lake's own waves breaking and running
            // up it
            (Vec3::new(172.0, 77.0, 22.6), Vec3::new(181.5, 81.0, 19.9))
        } else if mode.contains("sea-lake") {
            // the inland lake, from its shore
            (Vec3::new(178.0, 64.0, 26.0), Vec3::new(146.0, 84.0, 19.5))
        } else if mode.contains("sea-high") {
            // from far up, over the lagoon and the open sea beyond
            (Vec3::new(110.0, 60.0, 150.0), Vec3::new(20.0, 140.0, 20.0))
        } else if mode.contains("sea-island") {
            // over the lagoon's island, the swell wrapping round it
            (Vec3::new(58.0, 118.0, 34.0), Vec3::new(28.0, 150.0, 19.0))
        } else if mode.contains("sea-boat") {
            // beside the boat and a buoy, the dock behind
            (Vec3::new(35.0, 121.0, 22.2), Vec3::new(42.0, 131.0, 20.0))
        } else if mode.contains("sea-reef") {
            // down onto the reef: calm along its wall, not over its box
            (Vec3::new(4.0, 115.0, 31.0), Vec3::new(4.0, 134.0, 19.8))
        } else if mode.contains("sea-kelp") {
            // under the water, along the kelp bed of `sea-kelp`
            (Vec3::new(30.5, 113.0, 18.4), Vec3::new(38.0, 115.5, 17.2))
        } else if mode.contains("sea-roof") {
            // off the four walls of `sea-roof`, from the open sea
            (Vec3::new(1.0, 112.0, 24.5), Vec3::new(16.0, 112.0, 20.6))
        } else if mode.contains("sea-crash") {
            // off the breakwater's seaward face: the swell crashing on it
            (Vec3::new(14.0, 96.0, 23.5), Vec3::new(27.0, 108.0, 20.5))
        } else if mode.contains("sea-beach") {
            // down onto the lagoon's beach: the swash running up and back
            (Vec3::new(80.0, 120.0, 24.5), Vec3::new(67.0, 129.0, 19.8))
        } else if mode.contains("sea-low") {
            // at the waterline, along the beach, the breakers coming in
            (Vec3::new(64.0, 112.0, 20.9), Vec3::new(40.0, 136.0, 19.8))
        } else if mode.contains("sea") {
            // from the dunes, out over the lagoon to the open water
            (Vec3::new(82.0, 128.0, 27.5), Vec3::new(30.0, 128.0, 19.5))
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
    if mode.contains("sea") {
        // sheltered water (scene/water_shelter.rs): a dock on pilings off the
        // beach, a house on stilts in the lagoon and a breakwater whose lee
        // (east, with the default west wind) stays calm
        let wood = [0.45, 0.32, 0.2, 1.0];
        let mut parts = vec![
            (99560, Vec3::new(50.0, 124.0, 20.75), Vec3::new(16.0, 3.0, 0.2), wood),
            (99570, Vec3::new(34.0, 140.0, 21.0), Vec3::new(8.0, 8.0, 0.25), wood),
            (
                99571,
                Vec3::new(34.0, 140.0, 22.6),
                Vec3::new(7.0, 7.0, 3.0),
                [0.85, 0.82, 0.74, 1.0],
            ),
            (
                99580,
                Vec3::new(28.0, 108.0, 19.5),
                Vec3::new(2.5, 16.0, 3.5),
                [0.42, 0.42, 0.44, 1.0],
            ),
            // a thin floor laid at the water: no sea shows through it
            (
                99581,
                Vec3::new(50.0, 114.0, 20.05),
                Vec3::new(8.0, 6.0, 0.15),
                [0.3, 0.45, 0.25, 1.0],
            ),
        ];
        for (n, x) in [44.0, 48.0, 52.0, 56.0].into_iter().enumerate() {
            for (m, y) in [122.8, 125.2].into_iter().enumerate() {
                parts.push((99561 + (n * 2 + m) as u32, Vec3::new(x, y, 18.5), Vec3::new(0.3, 0.3, 4.4), wood));
            }
        }
        for (n, (x, y)) in [(30.5, 136.5), (37.5, 136.5), (30.5, 143.5), (37.5, 143.5)].into_iter().enumerate() {
            parts.push((99572 + n as u32, Vec3::new(x, y, 18.5), Vec3::new(0.35, 0.35, 4.8), wood));
        }
        // `sea-roof`: four short walls in the open sea facing the swell, with
        // open sky over the first and decks 3 m, 1.5 m and 0.4 m above the
        // water over the others' seaward faces: the spray stops under a
        // deck, and under the lowest nothing is thrown (scene/water_crash.rs)
        if mode.contains("sea-roof") {
            let stone = [0.42, 0.42, 0.44, 1.0];
            for (n, (y, under)) in [(103.0f32, None), (109.0, Some(3.0f32)), (115.0, Some(1.5)), (121.0, Some(0.4))]
                .into_iter()
                .enumerate()
            {
                parts.push((99640 + n as u32 * 2, Vec3::new(16.0, y, 19.5), Vec3::new(2.0, 4.0, 3.5), stone));
                if let Some(under) = under {
                    parts.push((
                        99641 + n as u32 * 2,
                        Vec3::new(14.5, y, 20.0 + under + 0.15),
                        Vec3::new(5.0, 4.4, 0.3),
                        wood,
                    ));
                }
            }
        }
        for (id, pos, scale, color) in parts {
            objects.push(super::prim(
                id,
                pos,
                Quat::IDENTITY,
                scale,
                box_shape,
                super::te(color, 0, false, 0.0),
                Default::default(),
                "",
            ));
        }
        // the reef: a mesh, a thin wall across its 14 m box from corner to
        // corner, through the water (`sea-reef` looks down onto it)
        let reef = aurora_prim::params::SculptParams {
            texture: SEA_REEF_MESH,
            sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
        };
        objects.push(super::prim(
            99630,
            Vec3::new(4.0, 134.0, 19.5),
            Quat::IDENTITY,
            Vec3::new(14.0, 14.0, 3.0),
            aurora_prim::VolumeParams {
                sculpt: Some(reef),
                ..Default::default()
            },
            super::te([0.42, 0.4, 0.36, 1.0], 0, false, 0.0),
            aurora_prim::ExtraParams {
                sculpt: Some(reef),
                ..Default::default()
            },
            "",
        ));
        // `sea-kelp`: a bed of kelp, flexible blades on the lagoon's floor
        // some 4.7 m down, swaying with the swell's surge under the water
        // (scene/flexible.rs `FlexiWater`); half of them with the wind
        // sensitivity at 0, the Legacy Viewer's default
        if mode.contains("sea-kelp") {
            let blade = super::shape(
                aurora_prim::params::LL_PCODE_PATH_FLEXIBLE,
                aurora_prim::params::LL_PCODE_PROFILE_CIRCLE,
                100,
                0,
                0,
            );
            for n in 0..25u32 {
                let (col, row) = ((n % 5) as f32, (n / 5) as f32);
                // (a fixed scatter, so the bed does not look planted in rows)
                let jitter = Vec3::new((n as f32 * 2.3).sin(), (n as f32 * 3.7).cos(), 0.0) * 0.45;
                let at = Vec3::new(34.0 + col * 1.5, 112.0 + row * 1.5, 0.0) + jitter;
                let bed = super::height(at.x, at.y);
                let length = (20.0 - bed - 0.5).clamp(1.0, 4.2) * (0.8 + 0.2 * (n as f32 * 1.3).sin().abs());
                let flex = aurora_prim::extra::FlexibleParams {
                    softness: 3,
                    tension: 1.0,
                    air_friction: 2.0,
                    gravity: 1.0,
                    wind_sensitivity: if n % 2 == 0 { 0.0 } else { 2.0 },
                    user_force: Vec3::ZERO,
                };
                objects.push(super::prim(
                    99660 + n,
                    Vec3::new(at.x, at.y, bed + length * 0.5),
                    Quat::from_rotation_z(n as f32 * 0.9),
                    Vec3::new(0.1, 0.3, length),
                    blade,
                    super::te([0.32, 0.38, 0.12, 1.0], 0, false, 0.0),
                    aurora_prim::ExtraParams {
                        flexible: Some(flex),
                        ..Default::default()
                    },
                    "",
                ));
            }
        }
        // floating, physical: they ride the waves (scene/water_ride.rs) -
        // a boat with a cabin linked to its hull, and three buoys
        let physical = |mut o: aurora_net::objects::ObjectUpdate| {
            o.update_flags |= crate::build::edits::FLAGS_USE_PHYSICS;
            o
        };
        let heading = Quat::from_rotation_z(0.5);
        objects.push(physical(super::prim(
            99590,
            Vec3::new(42.0, 131.0, 20.05),
            heading,
            Vec3::new(6.0, 2.2, 0.8),
            box_shape,
            super::te([0.92, 0.92, 0.95, 1.0], 0, false, 0.0),
            Default::default(),
            "",
        )));
        let mut cabin = super::prim(
            99591,
            Vec3::new(-0.8, 0.0, 0.9),
            Quat::IDENTITY,
            Vec3::new(2.2, 1.8, 1.0),
            box_shape,
            super::te([0.15, 0.35, 0.6, 1.0], 0, false, 0.0),
            Default::default(),
            "",
        );
        cabin.parent_id = 99590;
        objects.push(cabin);
        for (n, (x, y)) in [(38.0, 118.0), (52.0, 141.0), (24.0, 128.0)].into_iter().enumerate() {
            objects.push(physical(super::prim(
                99592 + n as u32,
                Vec3::new(x, y, 20.0),
                Quat::IDENTITY,
                Vec3::new(0.7, 0.7, 0.9),
                box_shape,
                super::te([0.95, 0.4, 0.05, 1.0], 0, false, 0.0),
                Default::default(),
                "",
            )));
        }
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
                || mode.contains("optics-")
                || mode.contains("alpha-layers")
                || mode.contains("alpha-cross"))
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
    if mode.contains("deep-lights") {
        // A floor 48 m under the water line (region water at 20 m) with a
        // point light just above it and one at 30 m under: the lamps must
        // light it in their own colour, not as dim as the sunlight down
        // there (water_scene_lit).
        for (id, pos, scale, color, light) in [
            (
                99540,
                Vec3::new(40.0, 100.0, -28.2),
                Vec3::new(16.0, 16.0, 0.4),
                [0.8, 0.8, 0.78, 1.0],
                None,
            ),
            (
                99541,
                Vec3::new(36.0, 104.0, -26.0),
                Vec3::new(1.0, 1.0, 4.0),
                [0.55, 0.5, 0.45, 1.0],
                None,
            ),
            (
                99542,
                Vec3::new(41.0, 99.0, -27.4),
                Vec3::new(0.3, 0.3, 0.3),
                [1.0, 0.85, 0.6, 1.0],
                Some([1.0, 0.8, 0.55, 1.0]),
            ),
            (
                99543,
                Vec3::new(46.0, 104.0, -10.0),
                Vec3::new(4.0, 4.0, 0.3),
                [0.8, 0.8, 0.78, 1.0],
                None,
            ),
            (
                99544,
                Vec3::new(46.0, 104.0, -9.0),
                Vec3::new(0.3, 0.3, 0.3),
                [0.5, 0.8, 1.0, 1.0],
                Some([0.5, 0.8, 1.0, 1.0]),
            ),
        ] {
            objects.push(super::prim(
                id,
                pos,
                Quat::IDENTITY,
                scale,
                box_shape,
                super::te(color, 0, light.is_some(), 0.0),
                aurora_prim::ExtraParams {
                    light: light.map(|color| aurora_prim::LightParams {
                        color,
                        radius: 8.0,
                        cutoff: 0.0,
                        falloff: 0.75,
                    }),
                    ..Default::default()
                },
                "",
            ));
        }
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
    if mode.contains("alpha-layers") {
        // Blended layers stacked under the water, as the animated surfaces
        // of a pond lying in region water: two wide sheets tilted opposite
        // ways, crossing along a line, and a small tile under both. Each
        // must show through the others on both sides of the crossing; none
        // may cut another along a straight edge.
        for (id, pos, tilt, scale, color) in [
            (
                99533,
                Vec3::new(56.0, 130.0, 19.55),
                0.05,
                Vec3::new(16.0, 12.0, 0.05),
                [0.2, 0.45, 1.0, 0.3],
            ),
            (
                99534,
                Vec3::new(56.0, 130.0, 19.5),
                -0.05,
                Vec3::new(16.0, 12.0, 0.05),
                [1.0, 0.85, 0.3, 0.45],
            ),
            (
                99535,
                Vec3::new(52.0, 128.0, 18.9),
                0.0,
                Vec3::new(2.0, 2.0, 0.05),
                [0.95, 0.1, 0.1, 0.7],
            ),
        ] {
            objects.push(super::prim(
                id,
                pos,
                Quat::from_rotation_y(tilt),
                scale,
                box_shape,
                super::te(color, 0, false, 0.0),
                Default::default(),
                "",
            ));
        }
    }
    if mode.contains("alpha-cross") {
        // A faint blended sheet crossing the water plane, as the animated
        // surface of a pond lying in region water: one half just under the
        // water, the other just above it. The water must keep its fog and
        // its reflection on both sides of the crossing line; the half under
        // the water must not show the bed clear, as through a hole.
        objects.push(super::prim(
            99536,
            Vec3::new(56.0, 130.0, 20.0),
            Quat::from_rotation_y(0.05),
            Vec3::new(16.0, 12.0, 0.02),
            box_shape,
            super::te([0.9, 0.95, 1.0, 0.15], 0, false, 0.0),
            Default::default(),
            "",
        ));
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
    if mode.contains("wet") {
        // the wet look's bathers (scene/avatar_wet.rs): side by side, they
        // soak chest deep in the lagoon, walk up the beach and stand drying
        // in the sun, each in another outfit (`WET_OUTFITS`), worn at the
        // pelvis: how each kind of surface takes the water
        let x = wet_bather(time);
        for (n, outfit) in WET_OUTFITS.iter().enumerate() {
            let id = 99760 + n as u32 * 2;
            let y = 121.0 + 2.0 * n as f32;
            let mut avatar = super::action_avatar(false);
            avatar.local_id = id;
            avatar.full_id = Uuid::from_u128(u128::from(id));
            let half = avatar.scale.z.abs() * 0.5;
            avatar.position = Vec3::new(x, y, super::floor_at(x, y) + half);
            avatar.rotation = Quat::from_rotation_z(std::f32::consts::PI);
            avatar.name_values = format!("FirstName STRING RW SV {}\nLastName STRING RW SV Wet", outfit.0);
            objects.push(avatar);
            if let Some(look) = wet_outfit(id + 1, id, outfit.1) {
                objects.push(look);
            }
        }
    }
    if mode.contains("sea") {
        // someone standing on the boat's foredeck (the hull's top is 20.45 m):
        // drawn, they ride with the deck (scene/water_ride.rs)
        let mut avatar = super::action_avatar(false);
        avatar.local_id = 99596;
        avatar.full_id = Uuid::from_u128(99596);
        avatar.position = Vec3::new(43.6, 131.9, 20.45 + avatar.scale.z.abs() * 0.5);
        avatar.name_values = "FirstName STRING RW SV Deck\nLastName STRING RW SV Demo".into();
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

/// The wet look's bathers: a name, and what each wears at the pelvis (a
/// cylinder from chest to knees): nothing (the plain body, which has no
/// material), a sealed glossy suit (PBR, off-white, roughness 0.2: plating, skin with a
/// material, latex), a dark cloth, a white cloth, and fur (see-through).
const WET_OUTFITS: [(&str, WetOutfit); 5] = [
    ("Plain", WetOutfit::None),
    ("Sealed", WetOutfit::Sealed),
    ("Cloth", WetOutfit::Cloth(0)),
    ("White", WetOutfit::Cloth(1)),
    ("Fur", WetOutfit::Fur),
];

#[derive(Clone, Copy)]
enum WetOutfit {
    None,
    Sealed,
    /// Its material in `MAT_WET_CLOTH`.
    Cloth(usize),
    Fur,
}

/// The sealed suit's material and the cloths' (`super::pbr_materials`):
/// clothing has a material as a rule (a rough PBR one here), which makes it
/// cloth to the wet look; a face with none is taken as in between.
pub const MAT_WET_SEALED: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0A01);
pub const MAT_WET_CLOTH: [Uuid; 2] = [
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0A02),
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0A03),
];

pub fn materials() -> Vec<(Uuid, aurora_assets::PbrMaterial)> {
    let pbr = |color: [f32; 4], roughness: f32| aurora_assets::PbrMaterial {
        base_color_factor: color,
        metallic_factor: 0.0,
        roughness_factor: roughness,
        ..Default::default()
    };
    vec![
        (MAT_WET_SEALED, pbr([0.85, 0.85, 0.82, 1.0], 0.2)),
        (MAT_WET_CLOTH[0], pbr([0.12, 0.16, 0.32, 1.0], 0.95)),
        (MAT_WET_CLOTH[1], pbr([0.92, 0.92, 0.9, 1.0], 0.95)),
    ]
}

/// The outfit `outfit` as local id `id`, worn by the avatar `wearer`.
fn wet_outfit(id: u32, wearer: u32, outfit: WetOutfit) -> Option<aurora_net::objects::ObjectUpdate> {
    use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE};
    let (te, extra) = match outfit {
        WetOutfit::None => return None,
        WetOutfit::Sealed => (
            super::te([1.0; 4], 0, false, 0.0),
            aurora_prim::ExtraParams {
                render_materials: (0..6).map(|f| (f, MAT_WET_SEALED)).collect(),
                ..Default::default()
            },
        ),
        WetOutfit::Cloth(k) => (
            super::te([1.0; 4], 0, false, 0.0),
            aurora_prim::ExtraParams {
                render_materials: (0..6).map(|f| (f, MAT_WET_CLOTH[k.min(1)])).collect(),
                ..Default::default()
            },
        ),
        WetOutfit::Fur => (super::te([0.45, 0.3, 0.18, 0.85], 0, false, 0.0), Default::default()),
    };
    let mut o = super::prim(
        id,
        Vec3::new(0.0, 0.0, 0.05),
        Quat::IDENTITY,
        Vec3::new(0.6, 0.52, 1.1),
        super::shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
        te,
        extra,
        "",
    );
    // worn at the pelvis (attachment point 10)
    o.parent_id = wearer;
    o.state = 10u8.rotate_left(4);
    Some(o)
}

/// Where the wet look's bather stands along the lagoon at `time` (s), in
/// a 90 s round: 15 s chest deep in the water, 10 s walking up the beach,
/// then dry sand until it walks back in.
fn wet_bather(time: f32) -> f32 {
    let t = time.rem_euclid(90.0);
    let (deep, beach) = (58.0, 70.0);
    match t {
        t if t < 15.0 => deep,
        t if t < 25.0 => deep + (beach - deep) * (t - 15.0) / 10.0,
        t if t < 80.0 => beach,
        t => beach + (deep - beach) * (t - 80.0) / 10.0,
    }
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
