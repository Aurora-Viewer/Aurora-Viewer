//! Explicit offline, read-only preview of a local native water animation pack.
//! No inventory items, animation imports, uploads or persisted settings.
use super::{Animation, Set, State, StateConfig, Step};
use crate::{
    agent::MoveInput,
    scene::{Scene, anim::BoundAnim},
    world::World,
};
use glam::Vec3;
use std::{
    sync::{Arc, OnceLock},
    time::Instant,
};
use uuid::Uuid;

const SET: Uuid = Uuid::from_u128(0x9950_0000_0000_0000_0000_0000_0001_0000);
const FILES: [&str; 12] = [
    "01-flotter-surface-P6.anim",
    "02-brasse-P6.anim",
    "03-crawl-P6.anim",
    "04-nage-arriere-P6.anim",
    "05-flotter-sous-eau-P6.anim",
    "06-nager-bas-P6.anim",
    "07-nager-haut-P6.anim",
    "08-entree-eau-inertie-P6.anim",
    "09-sortie-eau-vers-vol-P6.anim",
    "10-sortie-eau-atterrissage-P6.anim",
    "11-plongeon-saut-P6.anim",
    "12-plongeon-entree-eau-P5.anim",
];

pub fn mode() -> Option<&'static str> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_WATER_ACTIONS").ok()).as_deref()
}

pub fn asset(index: usize) -> Uuid {
    Uuid::from_u128(SET.as_u128() + 1 + index as u128)
}

/// Called only after App's demo gate. Parse once, then keep decoded assets in memory.
pub fn install(scene: &mut Scene, catalog: &mut super::inventory::Catalog) -> Option<Uuid> {
    static PACK: OnceLock<Option<Vec<aurora_assets::Animation>>> = OnceLock::new();
    let pack = PACK
        .get_or_init(|| {
            let root = std::env::var_os("AURORA_DEMO_WATER_ANIMS")?;
            let root = std::path::Path::new(&root);
            let mut pack = Vec::new();
            for name in FILES {
                let result = std::fs::read(root.join(name))
                    .map_err(|e| e.to_string())
                    .and_then(|data| aurora_assets::parse_animation(&data).map_err(|e| e.to_string()));
                match result {
                    Ok(animation) => pack.push(animation),
                    Err(reason) => {
                        log::warn!("offline water animation {name}: {reason}");
                        return None;
                    }
                }
            }
            match aurora_assets::parse_animation(include_bytes!("../../assets/animations/swimming/cannonball.anim")) {
                Ok(animation) => pack.push(animation),
                Err(reason) => {
                    log::warn!("offline cannonball: {reason}");
                    return None;
                }
            }
            log::info!("offline water pack: {} native clips decoded, no inventory changes", pack.len());
            Some(pack)
        })
        .as_ref()?;
    if catalog.sets.iter().any(|s| s.id == SET) {
        return Some(SET);
    }
    let mut set = Set::new("Local water animation preview".into());
    set.id = SET;
    for (state, i) in [
        (State::Floating, 0),
        (State::SwimmingForward, 1),
        (State::UnderwaterSwimming, 2),
        (State::UnderwaterFloating, 4),
        (State::SwimmingDown, 5),
        (State::SwimmingUp, 6),
        (State::WaterEntry, 7),
        (State::WaterFlightExit, 8),
        (State::WaterShoreExit, 9),
        (State::WaterDiveJump, 10),
        (State::WaterDiveEntry, 11),
        (State::WaterCannonball, 12),
    ] {
        if mode() == Some("missing") && i >= 10 {
            continue;
        }
        let id = asset(i);
        scene
            .anims
            .insert(id, Arc::new(BoundAnim::bind(pack[i].clone(), &scene.avatar_lib.rig)));
        let name = FILES.get(i).copied().unwrap_or("13-cannonball-P6.anim").to_string();
        set.states.insert(
            state,
            StateConfig {
                folder: id,
                steps: vec![Step {
                    id,
                    name: name.clone(),
                    order: 0,
                    group: false,
                    members: vec![Animation {
                        link: id,
                        original: id,
                        asset: id,
                        name,
                        order: 0,
                    }],
                }],
                ..Default::default()
            },
        );
    }
    // Surface swimming is one ordinary AO list, not a style selector.
    for i in [2, 3] {
        let id = asset(i + 40);
        scene
            .anims
            .insert(id, Arc::new(BoundAnim::bind(pack[i].clone(), &scene.avatar_lib.rig)));
        let name = FILES[i].to_owned();
        set.states.get_mut(&State::SwimmingForward)?.steps.push(Step {
            id,
            name: name.clone(),
            order: (i - 1) as i32,
            group: false,
            members: vec![Animation {
                link: id,
                original: id,
                asset: id,
                name,
                order: (i - 1) as i32,
            }],
        });
    }
    if mode() == Some("swim-cycle") {
        set.states.get_mut(&State::SwimmingForward)?.cycle = super::Cycle {
            cycle: true,
            random: false,
            seconds: 1.5,
        };
    }
    if mode().is_some_and(|m| m.starts_with("wade") || m.starts_with("water-")) {
        let id = asset(32);
        scene.anims.insert(
            id,
            Arc::new(BoundAnim::bind(crate::demo::water::animation(), &scene.avatar_lib.rig)),
        );
        set.states.insert(
            State::Walking,
            StateConfig {
                folder: id,
                steps: vec![Step {
                    id,
                    name: "Local contact gait".into(),
                    order: 0,
                    group: false,
                    members: vec![Animation {
                        link: id,
                        original: id,
                        asset: id,
                        name: "Local contact gait".into(),
                        order: 0,
                    }],
                }],
                ..Default::default()
            },
        );
        if mode().is_some_and(|m| m.starts_with("water-")) {
            set.states.insert(State::Running, set.states[&State::Walking].clone());
            for (state, index) in [(State::Crouching, 35), (State::CrouchWalk, 36)] {
                let mut animation = crate::demo::link::animations()
                    .into_iter()
                    .find(|(id, _)| *id == State::CrouchWalk.motion())?
                    .1;
                if state == State::Crouching {
                    for joint in &mut animation.joints {
                        joint.rot_keys.truncate(1);
                        joint.pos_keys.truncate(1);
                    }
                }
                let id = asset(index);
                scene.anims.insert(id, Arc::new(BoundAnim::bind(animation, &scene.avatar_lib.rig)));
                let name = format!("Local {state:?}");
                set.states.insert(
                    state,
                    StateConfig {
                        folder: id,
                        steps: vec![Step {
                            id,
                            name: name.clone(),
                            order: 0,
                            group: false,
                            members: vec![Animation {
                                link: id,
                                original: id,
                                asset: id,
                                name,
                                order: 0,
                            }],
                        }],
                        ..Default::default()
                    },
                );
            }
        }
    }
    catalog.sets.push(set);
    catalog.loading = false;
    catalog.failed = false;
    Some(SET)
}

pub fn trajectory_input(frame: u64) -> Option<MoveInput> {
    let mode = mode()?;
    if frame < 720 {
        return None;
    }
    if mode.starts_with("water-") {
        return Some(MoveInput {
            forward: !matches!(mode, "water-crouch" | "water-jump" | "water-jump-hold"),
            run: matches!(mode, "water-run" | "water-jump-run"),
            down: matches!(mode, "water-crouch" | "water-crouch-walk"),
            up: mode.starts_with("water-jump")
                && (0.5..if mode == "water-jump-hold" { 8.8 } else { 0.55 }).contains(&(((frame - 720) % 540) as f32 / 60.0)),
            ..Default::default()
        });
    }
    if mode.starts_with("wade") || mode.starts_with("swim") {
        let time = ((frame - 720) % 540) as f32 / 60.0;
        return Some(MoveInput {
            forward: !mode.starts_with("swim-float") && mode != "swim-back",
            back: mode == "swim-back",
            run: mode == "swim-crawl",
            down: (mode == "swim-submerged" && time < 0.05) || (mode.starts_with("swim-depth") && (1.5..1.55).contains(&time)),
            up: mode.starts_with("swim-depth") && (4.5..if mode.ends_with("-hold") { 8.8 } else { 4.55 }).contains(&time),
            ..Default::default()
        });
    }
    let t = ((frame - 720) % 540) as f32 / 60.0 - 0.5;
    Some(MoveInput {
        forward: t < 1.55,
        up: mode != "cannonball-flight" && (0.0..0.12).contains(&t),
        down: if mode == "cannonball-late" {
            (1.25..1.30).contains(&t)
        } else if matches!(mode, "dive" | "cancel" | "dry" | "missing") {
            (0.20..0.25).contains(&t)
        } else {
            mode.starts_with("cannonball") && (0.55..0.65).contains(&t)
        },
        ..Default::default()
    })
}

pub struct Sample {
    pub input: MoveInput,
    pub grounded: bool,
    pub jumping: bool,
}

/// Synthetic trajectory, not a physics prediction or simulator validation.
/// The AO, bridge wire messages and posed-joint splash detector remain real.
pub fn trajectory(world: &mut World, frame: u64, now: Instant, float_height: f32) -> Option<Sample> {
    let mode = mode()?;
    if frame < 720 {
        return None;
    }
    let time = ((frame - 720) % 540) as f32 / 60.0;
    let t = time - 0.5;
    let air = (0.0..1.55).contains(&t);
    let (mut position, mut velocity) = if t < 0.0 {
        (Vec3::new(40.0, 124.0, 24.0), Vec3::ZERO)
    } else if air {
        (
            Vec3::new(40.0 + 4.0 * t, 124.0, 24.0 + 4.5 * t - 4.905 * t * t),
            Vec3::new(4.0, 0.0, 4.5 - 9.81 * t),
        )
    } else {
        let recovery = ((t - 1.55) / 3.0).clamp(0.0, 1.0);
        (
            Vec3::new(46.2 + recovery * 0.6, 124.0, 19.2 + recovery * 0.4),
            Vec3::new(0.2, 0.0, 0.13),
        )
    };
    let water_walk = mode.starts_with("water-");
    let surface_motion = mode.starts_with("wade") || mode.starts_with("swim") || water_walk;
    if surface_motion {
        let period = if mode == "wade-run" { 5.0 } else { 9.0 };
        let motion_time = if mode.starts_with("swim-float") {
            0.0
        } else if mode == "swim-stop" {
            time.min(1.5)
        } else {
            time
        };
        let phase = motion_time * std::f32::consts::TAU / period;
        let (center, extent) = if water_walk {
            let height = world
                .objects
                .index_of_uuid(&world.agent_id)
                .and_then(|i| world.objects.get(i))
                .map_or(1.7, |o| o.scale.z);
            (
                Vec3::new(46.0, 124.0, world.main_water_height() + height * 0.5),
                match mode {
                    "water-run" | "water-jump-run" => 5.0,
                    "water-crouch" | "water-jump" | "water-jump-hold" => 0.0,
                    "water-crouch-walk" => 1.2 * period / std::f32::consts::TAU,
                    _ => 3.2,
                },
            )
        } else if mode.starts_with("wade") {
            (Vec3::new(62.0, 124.0, 20.55), if mode == "wade-run" { 3.0 } else { 1.8 })
        } else {
            let height = world
                .objects
                .index_of_uuid(&world.agent_id)
                .and_then(|i| world.objects.get(i))
                .map_or(1.7, |o| o.scale.z.clamp(0.6, 3.0));
            let surface = world.main_water_height() - (height * 0.5 - 0.4).clamp(0.25, 0.8)
                + float_height
                + if mode.ends_with("-offset") && time >= 1.0 { 0.4 } else { 0.0 };
            let depth = if mode == "swim-submerged" {
                1.6
            } else if mode.starts_with("swim-depth") {
                1.6 * ((time - 1.5).clamp(0.0, 1.0) - ((time - 4.5) / 0.7).clamp(0.0, 1.0))
            } else {
                0.0
            };
            (Vec3::new(46.0, 124.0, surface - depth), 3.2)
        };
        position = center + Vec3::X * extent * phase.sin();
        velocity = Vec3::X * extent * (std::f32::consts::TAU / period) * phase.cos();
        if mode.starts_with("water-jump") && (0.0..aurora_net::bridge::WATER_WALK_JUMP_DURATION).contains(&t) {
            let phase = t / aurora_net::bridge::WATER_WALK_JUMP_DURATION;
            let scale = 4.0 * aurora_net::bridge::WATER_WALK_JUMP_HEIGHT;
            position.z += scale * phase * (1.0 - phase);
            velocity.z = scale * (1.0 - 2.0 * phase) / aurora_net::bridge::WATER_WALK_JUMP_DURATION;
        }
        if mode.starts_with("swim-depth") {
            velocity.z = if (1.5..2.5).contains(&time) {
                -1.6
            } else if (4.5..5.2).contains(&time) {
                1.6 / 0.7
            } else {
                0.0
            };
        }
        if mode == "swim-turn" {
            position.y += 2.0 * (1.0 - phase.cos());
            velocity.y = 2.0 * (std::f32::consts::TAU / period) * phase.sin();
        }
        if mode.starts_with("swim-float") || (mode == "swim-stop" && time >= 1.5) {
            velocity = Vec3::ZERO;
        }
    }
    if mode == "cannonball-flight" {
        let fall = (t - 0.55).max(0.0);
        position = Vec3::new(44.0, 124.0, (26.0 - 4.905 * fall * fall).max(19.0));
        velocity = Vec3::new(0.0, 0.0, if position.z > 19.0 { -9.81 * fall } else { 0.0 });
    }
    if mode == "dry" {
        position.x += 44.0;
        let ground = world.ground_height(position).unwrap_or(24.0);
        let jump = if t > 0.0 { (4.5 * t - 4.905 * t * t).max(0.0) } else { 0.0 };
        position.z = ground + 0.9 + jump;
        velocity.z = if jump > 0.0 { 4.5 - 9.81 * t } else { 0.0 };
    }
    let cancelled = mode == "cancel" && t > 0.5;
    world.agent.on_server_update_at(position, velocity, Vec3::ZERO, false, true, now);
    world.agent.position = position;
    world.agent.velocity = velocity;
    let rotation = if surface_motion && velocity.truncate().length() > 0.01 {
        velocity.y.atan2(velocity.x) + if mode == "swim-back" { std::f32::consts::PI } else { 0.0 }
    } else {
        world.agent.yaw
    };
    world.agent.yaw = rotation;
    world.agent.flying = cancelled || (mode == "cannonball-flight" && t < 0.55);
    if let Some(index) = world.objects.index_of_uuid(&world.agent_id)
        && let Some(object) = world.objects.get_mut(index)
    {
        object.position = position;
        object.rotation = glam::Quat::from_rotation_z(rotation);
    }
    let input = trajectory_input(frame)?;
    let grounded = mode.starts_with("wade") || (!surface_motion && mode != "cannonball-flight" && (t < 0.0 || (mode == "dry" && t > 0.92)));
    let jumping = !surface_motion && mode != "cannonball-flight" && (0.0..0.8).contains(&t);
    let state = if water_walk || mode == "cannonball-flight" {
        State::Falling
    } else if mode.starts_with("swim") {
        State::Flying
    } else if mode.starts_with("wade") {
        State::Walking
    } else if jumping {
        State::Jumping
    } else if air {
        State::Falling
    } else {
        State::Standing
    };
    let previous = world
        .animations
        .get(&world.agent_id)
        .and_then(|a| a.iter().find(|a| a.id == state.motion()));
    let signal = crate::world::PlayingAnimation::from_signal(state.motion(), 1, now, previous);
    let raw = world.animations.entry(world.agent_id).or_default();
    raw.retain(|a| State::from_motion(a.id).is_none());
    raw.push(signal);
    Some(Sample { input, grounded, jumping })
}
