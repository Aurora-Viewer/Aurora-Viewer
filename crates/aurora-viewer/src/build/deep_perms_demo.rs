//! `AURORA_DEMO_DEEP_PERMS`: a chest near the avatar whose contents hold
//! objects inside objects, for "Include objects inside objects"
//! (`build/deep_perms.rs`). The demo simulator (`demo_sim.rs`) keeps what
//! each object item holds, rezzes it with those contents and saves a copy
//! back with what it holds then.
//!
//! - `1`: the chest only;
//! - `run`: with `AURORA_DEMO_BUILD=contents-ready,object:99760` and
//!   `AURORA_DEMO_BUILD_TAB=contents+permissions`, the option is ticked and
//!   Apply pressed once the chest is selected; the log ends with the report;
//! - `run-replace`: the same, the simulator replacing a saved item in place;
//! - `run-refuse`: the simulator refuses every rez with an alert.

use aurora_net::build::{TaskItem, perm};
use aurora_net::{NetEvent, objects::ObjectUpdate};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams, te::TextureFace};
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub const CHEST: u32 = 99760;
const PERM_ALL: u32 = 0x7fff_ffff;

pub fn id(local: u32) -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | local as u128)
}

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_DEEP_PERMS")
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// `run`, `run-replace`, `run-refuse`: the window starts the job itself.
pub fn run() -> bool {
    mode().is_some_and(|m| m.starts_with("run"))
}

pub fn replace_in_place() -> bool {
    mode().is_some_and(|m| m == "run-replace")
}

pub fn refuse() -> bool {
    mode().is_some_and(|m| m == "run-refuse")
}

fn item(parent: Uuid, n: u128, name: &str, asset_type: i8, owner_mask: u32) -> TaskItem {
    TaskItem {
        item_id: Uuid::from_u128(0xA0E0_DEE9_0000_0000_0000_0000_0000_0000 | n),
        parent_id: parent,
        creator_id: crate::demo::DEMO_AGENT,
        owner_id: crate::demo::DEMO_AGENT,
        last_owner_id: crate::demo::DEMO_AGENT,
        base_mask: PERM_ALL,
        owner_mask,
        next_owner_mask: perm::MOVE | perm::TRANSFER,
        asset_id: Uuid::from_u128(0xA0E0_DEE9_A55E_0000_0000_0000_0000_0000 | n),
        asset_type,
        inv_type: asset_type,
        sale_price: 10,
        name: name.into(),
        creation_date: 1_700_000_000,
        ..Default::default()
    }
}

/// The chest's listing, what each of its object items holds in its root
/// (by item id, at every level) and in its child prims: a notecard; "Outer
/// box" with a script, a notecard and "Inner box", which holds a notecard
/// and "Tiny box" with a script; "Linked box", two linked prims, a notecard
/// in its root and a script and "Pebble box" (a notecard) in its child; a
/// no-copy and a no-modify object, which stay closed.
pub fn contents(chest: Uuid) -> (Vec<TaskItem>, HashMap<Uuid, Vec<TaskItem>>, HashMap<Uuid, Vec<Vec<TaskItem>>>) {
    let mut stored = HashMap::new();
    let mut links = HashMap::new();
    let outer = item(chest, 2, aurora_i18n::tr!("content-perms-demo-outer"), 6, PERM_ALL);
    let inner = item(outer.item_id, 5, aurora_i18n::tr!("content-perms-demo-inner"), 6, PERM_ALL);
    let tiny = item(inner.item_id, 7, aurora_i18n::tr!("content-perms-demo-tiny"), 6, PERM_ALL);
    stored.insert(
        tiny.item_id,
        vec![item(tiny.item_id, 8, aurora_i18n::tr!("content-perms-demo-spin"), 10, PERM_ALL)],
    );
    stored.insert(
        inner.item_id,
        vec![
            item(inner.item_id, 6, aurora_i18n::tr!("content-perms-demo-inner-note"), 7, PERM_ALL),
            tiny,
        ],
    );
    stored.insert(
        outer.item_id,
        vec![
            item(outer.item_id, 3, aurora_i18n::tr!("content-perms-demo-glow"), 10, PERM_ALL),
            item(outer.item_id, 4, aurora_i18n::tr!("content-perms-demo-readme"), 7, PERM_ALL),
            inner,
        ],
    );
    let heirloom = item(chest, 9, aurora_i18n::tr!("content-perms-demo-heirloom"), 6, PERM_ALL & !perm::COPY);
    let sealed = item(
        chest,
        10,
        aurora_i18n::tr!("content-perms-demo-sealed"),
        6,
        PERM_ALL & !perm::MODIFY,
    );
    stored.insert(heirloom.item_id, Vec::new());
    stored.insert(sealed.item_id, Vec::new());
    // the child prim's item ids are made when it is rezzed: `parent` is a
    // placeholder
    let linked = item(chest, 11, aurora_i18n::tr!("content-perms-demo-linked"), 6, PERM_ALL);
    let pebble = item(Uuid::nil(), 14, aurora_i18n::tr!("content-perms-demo-pebble"), 6, PERM_ALL);
    stored.insert(
        pebble.item_id,
        vec![item(
            pebble.item_id,
            15,
            aurora_i18n::tr!("content-perms-demo-inner-note"),
            7,
            PERM_ALL,
        )],
    );
    stored.insert(
        linked.item_id,
        vec![item(linked.item_id, 12, aurora_i18n::tr!("content-perms-demo-readme"), 7, PERM_ALL)],
    );
    links.insert(
        linked.item_id,
        vec![vec![
            item(Uuid::nil(), 13, aurora_i18n::tr!("content-perms-demo-spin"), 10, PERM_ALL),
            pebble,
        ]],
    );
    let items = vec![
        item(chest, 1, aurora_i18n::tr!("content-perms-demo-notes"), 7, PERM_ALL),
        outer,
        heirloom,
        sealed,
        linked,
    ];
    (items, stored, links)
}

/// The chest, a wooden box in front of the avatar.
pub fn events(handle: u64, ground: f32) -> Vec<NetEvent> {
    let object = ObjectUpdate {
        local_id: CHEST,
        full_id: id(CHEST),
        parent_id: 0,
        pcode: aurora_prim::params::LL_PCODE_VOLUME,
        state: 0,
        crc: 0,
        material: 3,
        click_action: 0,
        scale: Vec3::new(1.0, 0.7, 0.6),
        position: Vec3::new(134.5, 127.5, ground + 0.3),
        rotation: Quat::IDENTITY,
        velocity: Vec3::ZERO,
        acceleration: Vec3::ZERO,
        angular_velocity: Vec3::ZERO,
        update_flags: crate::ui::context::flags::OBJECT_MODIFY
            | crate::ui::context::flags::OBJECT_MOVE
            | crate::ui::context::flags::OBJECT_COPY
            | crate::ui::context::flags::OBJECT_TRANSFER
            | crate::ui::context::flags::OBJECT_YOU_OWNER,
        owner_id: Uuid::nil(),
        volume: VolumeParams::default(),
        texture_entry: Some(Arc::new(TextureEntry {
            faces: vec![
                TextureFace {
                    texture: aurora_prim::te::BLANK_TEXTURE,
                    color: [0.45, 0.3, 0.16, 1.0],
                    ..Default::default()
                };
                6
            ],
        })),
        extra: ExtraParams::default(),
        name_values: String::new(),
        text: aurora_i18n::tr!("content-perms-demo-chest").into(),
        text_color: [232, 237, 251, 255],
        media_url: String::new(),
        tree_species: None,
        texture_anim: None,
        foot_plane: None,
        particles: Default::default(),
        sound: Default::default(),
    };
    vec![NetEvent::ObjectUpdates {
        handle,
        objects: vec![object],
    }]
}
