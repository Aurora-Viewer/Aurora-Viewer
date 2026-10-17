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
    seed_options(inv, owner, false)
}

pub fn seed_firestorm(inv: &mut Inventory, owner: Uuid) -> Uuid {
    let selected = seed(inv, owner);
    let Some(brand) = inventory::child(inv, inv.root, "#Aurora") else {
        return selected;
    };
    let Some(root) = inventory::child(inv, brand, inventory::FOLDER) else {
        return selected;
    };
    let inventory_root = inv.root;
    let fs = inventory::child(inv, inv.root, "#Firestorm").unwrap_or_else(|| {
        let id = Uuid::new_v4();
        folder(inv, inventory_root, id, "#Firestorm", -1);
        id
    });
    if let Some(parent) = inv.folders.get_mut(&brand) {
        parent.children.retain(|id| *id != root);
    }
    if let Some(parent) = inv.folders.get_mut(&fs) {
        parent.children.push(root);
    }
    if let Some(folder) = inv.folders.get_mut(&root) {
        folder.info.parent = fs;
        folder.info.name = "#AO".into();
    }
    inv.generation = inv.generation.wrapping_add(1);
    selected
}

/// Synthetic cached source matching the reported set name/options. No grid.
pub fn seed_firestorm_wait(inv: &mut Inventory, owner: Uuid) -> Uuid {
    let selected = seed_firestorm(inv, owner);
    if let Some(set) = inv.folders.get_mut(&selected) {
        set.info.name = "MonSet".into();
    }
    if let Some(standing) = inventory::child(inv, selected, "Standing:CT6.00:CY")
        && let Some(folder) = inv.folders.get_mut(&standing)
    {
        folder.info.name = "Standing:CT15.00:CY:RN".into();
    }
    inv.generation = inv.generation.wrapping_add(1);
    selected
}

/// Two legacy sets beside an existing native AO, for explicit migration only.
pub fn seed_migration(inv: &mut Inventory, owner: Uuid) -> Uuid {
    seed_firestorm(inv, owner);
    let mut source = inventory::Catalog::default();
    source.refresh_firestorm(inv);
    let Some(mut set) = source.sets.first().cloned() else {
        return Uuid::nil();
    };
    set.name = "Firestorm Walk Set".into();
    set.default = false;
    // Fixture construction only: production mutations always use the native
    // catalogue, and editable rejects destinations under #Firestorm.
    if let Ok((change, _)) = inventory::create(inv, &source, &set) {
        let _ = crate::world::inventory::actions::demo_mutate(inv, owner, change);
    }
    set.name = "Aurora AO".into();
    set.default = true;
    let mut native = inventory::Catalog::default();
    native.refresh(inv);
    inventory::create(inv, &native, &set)
        .and_then(|(change, id)| {
            crate::world::inventory::actions::demo_mutate(inv, owner, change).map(|reply| reply.created.get(&id).copied().unwrap_or(id))
        })
        .unwrap_or_else(|reason| {
            log::warn!("AO migration fixture: {reason}");
            Uuid::nil()
        })
}

/// Only called for explicit offline AO fixtures. The real writable gate is
/// shared with production; these placeholder services never reach a client.
pub fn inventory_services(world: &mut crate::world::World, ready: bool) {
    let Some(region) = world.main_region.and_then(|id| world.regions.get_mut(&id)) else {
        return;
    };
    let services = ["InventoryAPIv3", "FetchInventoryDescendents2"]; // gitleaks:allow -- public capability names
    for name in services {
        if ready && !region.caps.contains_key(name) {
            Arc::make_mut(&mut region.caps).insert(name.into(), "https://inventory.invalid/offline".into());
        } else if !ready && region.caps.contains_key(name) {
            Arc::make_mut(&mut region.caps).remove(name);
        }
    }
}

pub fn seed_movement_design(inv: &mut Inventory, owner: Uuid) -> Uuid {
    seed_options(inv, owner, true)
}

/// An existing native AO and bridge directory under their previous names.
pub fn seed_folder_names(inv: &mut Inventory, owner: Uuid) -> Uuid {
    let selected = seed_movement_design(inv, owner);
    if let Some(brand) = inventory::child(inv, inv.root, "#Aurora") {
        if let Some(root) = inventory::child(inv, brand, inventory::FOLDER)
            && let Some(root) = inv.folders.get_mut(&root)
        {
            root.info.name = inventory::OLD_FOLDER.into();
        }
        folder(inv, brand, Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_1010), "Bridge", -1);
        inv.generation = inv.generation.wrapping_add(1);
    }
    selected
}

/// Explicit offline fixtures only; never add sample entries to a user's AO.
pub fn seed_interface(inv: &mut Inventory, owner: Uuid, mode: &str) -> Uuid {
    let id = if mode == "interface-swim" {
        seed_movement_design(inv, owner)
    } else {
        seed(inv, owner)
    };
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(inv);
    let original = inv
        .items
        .values()
        .find(|a| a.name == "Aurora Stand A" && a.asset_type == 20)
        .cloned();
    if let Some(original) = original {
        for (index, letter) in ['C', 'D', 'E', 'F'].into_iter().enumerate() {
            let mut item = original.clone();
            item.id = Uuid::new_v4();
            item.name = format!("Aurora Stand {letter}");
            item.asset_id = ASSETS[index % 2];
            let item_id = item.id;
            inv.add_items(vec![item]);
            if let Some(set) = catalog.selected(id)
                && let Ok(change) = inventory::add(inv, set, State::Standing, item_id, None)
            {
                let _ = crate::world::inventory::actions::demo_mutate(inv, owner, change);
                catalog.invalidate();
                catalog.refresh(inv);
            }
        }
    }
    if matches!(mode, "interface-group" | "interface-track")
        && let Some(set) = catalog.selected(id)
        && let Some(c) = set.states.get(&State::Standing)
    {
        let selected = c.steps.iter().take(2).map(|s| s.id).collect();
        let name = if mode == "interface-track" {
            "Independent gestures"
        } else {
            "Body and hands"
        };
        if let Ok((change, _)) = inventory::editor::combine(inv, set, State::Standing, name, mode == "interface-track", &selected) {
            let _ = crate::world::inventory::actions::demo_mutate(inv, owner, change);
        }
    }
    id
}

pub fn seed_swimming_incomplete(inv: &mut Inventory, owner: Uuid) -> Uuid {
    let selected = seed_movement_design(inv, owner);
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(inv);
    if let Some(folder) = catalog
        .selected(selected)
        .and_then(|s| s.states.get(&State::WaterEntry))
        .map(|c| c.folder)
        && let Ok(change) = inventory::remove(inv, folder)
    {
        let _ = crate::world::inventory::actions::demo_mutate(inv, owner, change);
    }
    selected
}

fn seed_options(inv: &mut Inventory, owner: Uuid, swimming: bool) -> Uuid {
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
    let mut names: Vec<String> = [
        "Aurora Stand A",
        "Aurora Stand B",
        "Aurora Walk",
        "Aurora Run",
        "Aurora Sit",
        "Aurora Fly",
        "Aurora Type",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if swimming {
        names.extend(crate::movement::Style::ALL.into_iter().map(|style| style.name().to_owned()));
        names.extend(crate::movement::strokes::WATER_CLIPS.into_iter().map(|(_, name)| name.to_owned()));
    }
    for (index, name) in names.iter().enumerate() {
        let id = Uuid::from_u128(0xd0d0_a000_0000_0000_0000_0000_0000_2000 + index as u128);
        inv.add_items(vec![InvItem {
            id,
            parent: originals,
            name: name.clone(),
            asset_id: if index < ASSETS.len() {
                ASSETS[index]
            } else {
                crate::movement::strokes::local_id(index - ASSETS.len())
            },
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
        let indexes = match state {
            State::Standing => vec![0, 1],
            State::Floating if swimming => vec![10],
            State::SwimmingForward if swimming => vec![7],
            State::SwimmingUp if swimming => vec![9],
            State::SwimmingDown if swimming => vec![12],
            State::UnderwaterFloating if swimming => vec![11],
            State::WaterFlightExit if swimming => vec![9],
            State::WaterEntry | State::WaterShoreExit if swimming => vec![12],
            _ => vec![index],
        };
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
