//! Offline AO assets, inventory links and simulator state transitions.

use super::*;
use crate::world::{
    PlayingAnimation,
    inventory::{FetchState, Folder, Inventory},
};
use aurora_net::inventory::{InvFolder, InvItem};
use std::sync::Arc;
use std::time::Instant;

pub const NOTECARD: Uuid = Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0010);
pub const NOTECARD_TEXT: &str = "# Aurora AO\n[ Standing ]Aurora Stand A|Aurora Stand B\n[ Walking ]Aurora Walk\n[ Running ]Aurora Run\n[ Sitting ]Aurora Sit\n[ Hovering ]Aurora Fly\n[ Typing ]Aurora Type\n";
const ASSETS: [Uuid; 7] = [
    Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0001),
    Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0002),
    Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0003),
    Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0004),
    Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0005),
    Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0006),
    Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_0007),
];

fn folder(inv: &mut Inventory, parent: Uuid, id: Uuid, name: &str, kind: i32) {
    inv.folders.insert(
        id,
        Folder {
            info: InvFolder {
                id,
                parent,
                name: name.into(),
                type_default: kind,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    if let Some(p) = inv.folders.get_mut(&parent) {
        p.children.push(id);
    }
}

pub fn seed(inv: &mut Inventory, owner: Uuid) -> Uuid {
    if inv.root.is_nil() {
        inv.root = Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_1000);
    }
    if !inv.folders.contains_key(&inv.root) {
        folder(inv, Uuid::nil(), inv.root, "Inventory", 8);
    }
    let root = inv.root;
    if let Some(f) = inv.folders.get_mut(&root) {
        f.state = FetchState::Fetched;
    }
    if !inv.folders.values().any(|f| f.info.type_default == 14) {
        folder(inv, root, Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_1001), "Trash", 14);
    }
    let originals = Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_1002);
    folder(inv, root, originals, aurora_i18n::tr!("ao-demo-folder"), -1);
    let mut animations = Vec::new();
    for (index, name) in [
        "Aurora Stand A",
        "Aurora Stand B",
        "Aurora Walk",
        "Aurora Run",
        "Aurora Sit",
        "Aurora Fly",
        "Aurora Type",
    ]
    .iter()
    .enumerate()
    {
        let id = Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_2000 + index as u128);
        inv.add_items(vec![InvItem {
            id,
            parent: originals,
            name: (*name).into(),
            asset_id: ASSETS[index],
            asset_type: 20,
            inv_type: 19,
            owner,
            owner_mask: 0x7fff_ffff,
            ..Default::default()
        }]);
        if let Some(a) = inventory::animation(inv, id) {
            animations.push(a);
        }
    }
    inv.add_items(vec![InvItem {
        id: NOTECARD,
        parent: originals,
        name: "Aurora AO import".into(),
        asset_type: 7,
        inv_type: 7,
        owner,
        asset_id: NOTECARD,
        owner_mask: 0x7fff_ffff,
        ..Default::default()
    }]);
    let mut set = Set::new("Aurora AO".into());
    set.sits = true;
    set.smart = true;
    set.default = true;
    for state in State::ALL.into_iter().filter(|s| *s != State::Always) {
        let index = match state {
            State::Standing => 0,
            State::Walking | State::CrouchWalk => 2,
            State::Running => 3,
            State::Sitting | State::GroundSit => 4,
            State::Typing => 6,
            _ => 5,
        };
        let mut config = StateConfig::default();
        let indexes = if state == State::Standing { vec![0, 1] } else { vec![index] };
        for index in indexes {
            if let Some(a) = animations.get(index) {
                config.steps.push(Step {
                    id: Uuid::new_v4(),
                    name: a.name.clone(),
                    order: config.steps.len() as i32,
                    group: false,
                    members: vec![a.clone()],
                });
            }
        }
        if state == State::Standing {
            config.cycle = Cycle {
                cycle: true,
                random: false,
                seconds: 6.0,
            };
        }
        set.states.insert(state, config);
    }
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(inv);
    match inventory::create(inv, &catalog, &set).and_then(|(change, id)| {
        crate::world::inventory::actions::demo_mutate(inv, owner, change).map(|reply| reply.created.get(&id).copied().unwrap_or(id))
    }) {
        Ok(id) => id,
        Err(reason) => {
            log::warn!("AO demo inventory: {reason}");
            Uuid::nil()
        }
    }
}

pub fn assets(scene: &mut crate::scene::Scene) {
    use aurora_assets::anim::{JointMotion, RotKey};
    use glam::Quat;
    let rig = scene.avatar_lib.rig.clone();
    for state in State::ALL.into_iter().filter(|s| *s != State::Always) {
        let a = if matches!(state, State::Sitting | State::GroundSit) {
            crate::demo::seat_animation()
        } else {
            crate::demo::idle_animation()
        };
        scene
            .anims
            .insert(state.motion(), Arc::new(crate::scene::anim::BoundAnim::bind(a, &rig)));
    }
    scene.anims.insert(
        SIT_GENERIC,
        Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::seat_animation(), &rig)),
    );
    for (index, id) in ASSETS.into_iter().enumerate() {
        let mut a = if index == 4 {
            crate::demo::seat_animation()
        } else {
            crate::demo::idle_animation()
        };
        a.base_priority = 4;
        let (joint, angle) = match index {
            0 => ("mChest", 0.22),
            1 => ("mChest", -0.22),
            2 => ("mHipLeft", 0.55),
            3 => ("mHipLeft", 0.9),
            5 => ("mShoulderLeft", -0.3),
            6 => ("mElbowRight", -1.2),
            _ => ("mHead", 0.18),
        };
        a.joints.retain(|j| j.joint_name != joint);
        a.joints.push(JointMotion {
            joint_name: joint.into(),
            priority: -1,
            pos_keys: Vec::new(),
            rot_keys: vec![
                RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_y(angle),
                },
                RotKey {
                    time: 2.0,
                    rotation: Quat::from_rotation_y(-angle),
                },
                RotKey {
                    time: 4.0,
                    rotation: Quat::from_rotation_y(angle),
                },
            ],
        });
        scene.anims.insert(id, Arc::new(crate::scene::anim::BoundAnim::bind(a, &rig)));
    }
}

pub fn signal(world: &mut crate::world::World, frame: u64, mode: &str, now: Instant) {
    let state = if mode == "transitions" {
        [
            State::Standing,
            State::Walking,
            State::Running,
            State::GroundSit,
            State::Hovering,
            State::Flying,
            State::Standing,
        ][((frame.saturating_sub(620)) / 180 % 7) as usize]
    } else if world.agent.ground_sit {
        State::GroundSit
    } else if world.agent.flying {
        if world.agent.velocity.z > 0.2 {
            State::FlyingUp
        } else if world.agent.velocity.z < -0.2 {
            State::FlyingDown
        } else if world.agent.velocity.length() > 0.2 {
            State::Flying
        } else {
            State::Hovering
        }
    } else if world.agent.velocity.length() > 4.0 {
        State::Running
    } else if world.agent.velocity.length() > 0.2 {
        State::Walking
    } else {
        State::Standing
    };
    let raw = world.animations.entry(world.agent_id).or_default();
    let id = state.motion();
    let next = PlayingAnimation::from_signal(id, 1, now, raw.iter().find(|a| a.id == id));
    raw.clear();
    raw.push(next);
}
