//! Environment selector in the offline demo
//! (`AURORA_DEMO_ENV_SELECT=1|lighting`): a small library with an
//! "Environments" folder, a Settings folder in the inventory, and their
//! settings assets known locally (nothing is fetched), then a scripted run.
//!
//! - `1`: the selector opens; a sky is picked (frame 2600), a water on top
//!   (2900), the next sky with the > arrow (3200), a day cycle (3500), then
//!   « Environnement partagé » (3800, 5 s crossfade back to the region).
//! - `lighting`: the selector and « Éclairage personnel » open on the region
//!   sky; a sky is picked (2600), then edited (2900: brightness, haze, sun).
//! - `list`: the selector with its sky list open (2500).

use crate::world::eep::{Settings, SkyFrame};
use crate::world::env_select::{AT_SETTINGS, SettingsKind};
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::inventory::{FolderContents, InvFolder, InvItem};
use glam::{Quat, Vec3};
use std::sync::OnceLock;
use uuid::Uuid;

fn scenario() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| std::env::var("AURORA_DEMO_ENV_SELECT").unwrap_or_default().trim().to_owned())
}

pub fn enabled() -> bool {
    !scenario().is_empty() && scenario() != "0"
}

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0xE0E0_0000_0000_0000_0000_0000_0000_0000 | n)
}

/// Library owner, root and folders.
const LIB_OWNER: u128 = 0x100;
const LIB_ROOT: u128 = 0x101;
const LIB_ENVIRONMENTS: u128 = 0x102;
const LIB_SKIES: u128 = 0x103;
const LIB_WATER: u128 = 0x104;
const LIB_DAYS: u128 = 0x105;
const LIB_TEXTURES: u128 = 0x106;
/// The user's Settings folder (FT_SETTINGS), under the demo inventory root.
pub const USER_SETTINGS: u128 = 0x110;

/// (folder, item name, settings type, asset).
const ITEMS: &[(u128, &str, SettingsKind, u128)] = &[
    (LIB_SKIES, "A-12AM", SettingsKind::Sky, 0x201),
    (LIB_SKIES, "A-6PM", SettingsKind::Sky, 0x202),
    (LIB_SKIES, "Midday", SettingsKind::Sky, 0x203),
    (LIB_SKIES, "Coastal Sunset", SettingsKind::Sky, 0x204),
    (LIB_SKIES, "Nacon's Natural Sunset", SettingsKind::Sky, 0x205),
    (LIB_SKIES, "Annan Adored Pink Sky", SettingsKind::Sky, 0x206),
    (LIB_WATER, "Default Water", SettingsKind::Water, 0x301),
    (LIB_WATER, "Murky Water", SettingsKind::Water, 0x302),
    (LIB_WATER, "Clear Water", SettingsKind::Water, 0x303),
    (LIB_DAYS, "Default Day Cycle", SettingsKind::Day, 0x401),
    (LIB_DAYS, "Dynamic Day", SettingsKind::Day, 0x402),
    (USER_SETTINGS, "mon ciel violet", SettingsKind::Sky, 0x501),
    (USER_SETTINGS, "Eau turquoise", SettingsKind::Water, 0x502),
];

fn folder(id: u128, parent: u128, name: &str, t: i32) -> Llsd {
    let parent = if parent == 0 { Uuid::nil() } else { u(parent) };
    llsd_map! {"folder_id" => u(id), "parent_id" => parent, "name" => name, "type_default" => t, "version" => 1}
}

/// Login keys of the library (inventory-lib-root, -lib-owner, -skel-lib).
pub fn library_login(raw: &mut aurora_llsd::Map) {
    raw.insert(
        "inventory-lib-root".into(),
        Llsd::Array(vec![llsd_map! {"folder_id" => u(LIB_ROOT)}]),
    );
    raw.insert(
        "inventory-lib-owner".into(),
        Llsd::Array(vec![llsd_map! {"agent_id" => u(LIB_OWNER)}]),
    );
    raw.insert(
        "inventory-skel-lib".into(),
        Llsd::Array(vec![
            folder(LIB_ROOT, 0, "Library", 8),
            folder(LIB_ENVIRONMENTS, LIB_ROOT, "Environments", -1),
            folder(LIB_SKIES, LIB_ENVIRONMENTS, "Skies", -1),
            folder(LIB_WATER, LIB_ENVIRONMENTS, "Water", -1),
            folder(LIB_DAYS, LIB_ENVIRONMENTS, "Day Cycles", -1),
            folder(LIB_TEXTURES, LIB_ROOT, "Textures", 0),
        ]),
    );
}

/// The user's Settings folder for the inventory skeleton.
pub fn settings_folder(root: Uuid) -> Llsd {
    llsd_map! {"folder_id" => u(USER_SETTINGS), "parent_id" => root, "name" => "Paramètres", "type_default" => 56, "version" => 1}
}

/// FetchInventoryDescendents2 answer for the demo's environment folders.
pub fn folder_contents(folder_id: Uuid, owner: Uuid) -> Option<FolderContents> {
    let n = folder_id.as_u128() & 0xFFFF;
    if !enabled() || u(n) != folder_id || !(LIB_ROOT..=USER_SETTINGS).contains(&n) {
        return None;
    }
    let sub = |id: u128, name: &str| InvFolder {
        id: u(id),
        parent: folder_id,
        name: name.into(),
        type_default: -1,
        version: 2,
        ..Default::default()
    };
    let folders = match n {
        LIB_ROOT => vec![sub(LIB_ENVIRONMENTS, "Environments"), sub(LIB_TEXTURES, "Textures")],
        LIB_ENVIRONMENTS => vec![sub(LIB_SKIES, "Skies"), sub(LIB_WATER, "Water"), sub(LIB_DAYS, "Day Cycles")],
        _ => Vec::new(),
    };
    let items = ITEMS
        .iter()
        .filter(|(f, ..)| *f == n)
        .map(|(_, name, kind, a)| InvItem {
            id: u(a + 0x1000),
            parent: folder_id,
            name: (*name).into(),
            desc: String::new(),
            asset_type: AT_SETTINGS,
            inv_type: 25,
            asset_id: u(*a),
            flags: match kind {
                SettingsKind::Sky => 0,
                SettingsKind::Water => 1,
                SettingsKind::Day => 2,
            },
            favorite: false,
            creator: Uuid::nil(),
            created_at: 0,
            owner,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            thumbnail: Uuid::nil(),
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            last_owner: uuid::Uuid::nil(),
            group_id: uuid::Uuid::nil(),
            group_owned: false,
            sale_type: 0,
            sale_price: 0,
        })
        .collect();
    Some(FolderContents {
        folder_id,
        owner_id: owner,
        version: 2,
        folders,
        items,
    })
}

fn arr(v: &[f32]) -> Llsd {
    Llsd::Array(v.iter().map(|x| Llsd::Real(*x as f64)).collect())
}

/// A classic sky (LLSettingsSky keys) with the sun towards `sun`.
fn sky(sun: Vec3, sunlight: [f32; 3], horizon: [f32; 3], density: [f32; 3], haze: f32, gamma: f32) -> Llsd {
    let q = Quat::from_rotation_arc(Vec3::X, sun.normalize());
    let m = Quat::from_rotation_arc(Vec3::X, -sun.normalize());
    llsd_map! {
        "type" => "sky",
        "sun_rotation" => arr(&[q.x, q.y, q.z, q.w]),
        "moon_rotation" => arr(&[m.x, m.y, m.z, m.w]),
        "sunlight_color" => arr(&sunlight),
        "star_brightness" => 250.0,
        "gamma" => gamma,
        "cloud_shadow" => 0.35,
        "legacy_haze" => llsd_map! {
            "blue_horizon" => arr(&horizon),
            "blue_density" => arr(&density),
            "ambient" => arr(&[0.35, 0.33, 0.38]),
            "haze_density" => haze,
            "haze_horizon" => 0.19,
            "density_multiplier" => 0.00018,
            "distance_multiplier" => 0.8,
        },
    }
}

fn water(fog: [f32; 3], density: f32) -> Llsd {
    llsd_map! {
        "type" => "water",
        "water_fog_color" => arr(&fog),
        "water_fog_density" => density,
        "underwater_fog_mod" => 0.25,
        "fresnel_scale" => 0.4,
        "fresnel_offset" => 0.5,
    }
}

fn day(keys: &[(f32, Llsd)], water_frame: Llsd) -> Llsd {
    let mut frames = aurora_llsd::Map::new();
    let mut sky_track = Vec::new();
    for (i, (pos, frame)) in keys.iter().enumerate() {
        let name = format!("sky{i}");
        frames.insert(name.clone(), frame.clone());
        sky_track.push(llsd_map! {"key_keyframe" => *pos, "key_name" => name});
    }
    frames.insert("water".into(), water_frame);
    llsd_map! {
        "type" => "daycycle",
        "frames" => Llsd::Map(frames),
        "tracks" => Llsd::Array(vec![
            Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "water"}]),
            Llsd::Array(sky_track),
        ]),
    }
}

fn midnight() -> Llsd {
    sky(
        Vec3::new(0.2, 0.1, -0.97),
        [0.1, 0.12, 0.25],
        [0.05, 0.08, 0.2],
        [0.02, 0.04, 0.12],
        0.4,
        1.0,
    )
}

fn noon() -> Llsd {
    sky(
        Vec3::new(0.3, 0.2, 0.93),
        [0.9, 0.9, 0.85],
        [0.25, 0.45, 0.8],
        [0.25, 0.45, 0.76],
        0.7,
        1.0,
    )
}

fn sunset() -> Llsd {
    sky(
        Vec3::new(-0.96, 0.1, 0.15),
        [1.6, 0.7, 0.3],
        [0.9, 0.45, 0.3],
        [0.2, 0.25, 0.5],
        0.9,
        1.0,
    )
}

/// Settings assets of the demo items (parsed like downloaded ones).
pub fn settings_assets() -> Vec<(Uuid, Settings)> {
    let defs: Vec<(u128, Llsd)> = vec![
        (0x201, midnight()),
        (
            0x202,
            sky(
                Vec3::new(-0.9, -0.2, 0.3),
                [1.4, 0.9, 0.5],
                [0.7, 0.5, 0.45],
                [0.25, 0.35, 0.6],
                0.8,
                1.0,
            ),
        ),
        (0x203, noon()),
        (0x204, sunset()),
        (
            0x205,
            sky(
                Vec3::new(-0.97, 0.0, 0.08),
                [1.8, 0.6, 0.2],
                [1.0, 0.5, 0.2],
                [0.3, 0.2, 0.35],
                1.2,
                1.1,
            ),
        ),
        (
            0x206,
            sky(
                Vec3::new(0.5, 0.5, 0.6),
                [1.2, 0.8, 1.0],
                [1.0, 0.55, 0.8],
                [0.6, 0.3, 0.6],
                1.0,
                1.2,
            ),
        ),
        (0x301, water([0.0156, 0.149, 0.2509], 2.0)),
        (0x302, water([0.2, 0.22, 0.08], 12.0)),
        (0x303, water([0.1, 0.4, 0.5], 0.5)),
        (
            0x401,
            day(
                &[(0.0, midnight()), (0.25, sunset()), (0.5, noon()), (0.75, sunset())],
                water([0.0156, 0.149, 0.2509], 2.0),
            ),
        ),
        (0x402, day(&[(0.0, sunset()), (0.5, noon())], water([0.05, 0.35, 0.4], 3.0))),
        (
            0x501,
            sky(
                Vec3::new(0.4, -0.3, 0.5),
                [1.0, 0.7, 1.4],
                [0.55, 0.35, 0.95],
                [0.4, 0.2, 0.7],
                0.8,
                1.0,
            ),
        ),
        (0x502, water([0.05, 0.45, 0.45], 1.5)),
    ];
    defs.into_iter()
        .filter_map(|(n, v)| Settings::from_llsd(&v).map(|s| (u(n), s)))
        .collect()
}

/// A scripted step of the scenario.
#[derive(Debug, Clone, Copy)]
pub enum Step {
    /// Open the selector (and « Éclairage personnel »).
    Open {
        lighting: bool,
    },
    Pick(SettingsKind, Uuid),
    /// The > arrow of a list.
    Next(SettingsKind),
    Shared,
    /// « Éclairage personnel »: brightness and a lower sun.
    EditLighting,
    /// Drop the list down.
    OpenList(SettingsKind),
}

pub fn step(frame: u64) -> Option<Step> {
    if !enabled() {
        return None;
    }
    let lighting = scenario() == "lighting";
    let s = match frame {
        30 => Step::Open { lighting },
        2500 if scenario() == "list" => Step::OpenList(SettingsKind::Sky),
        _ if scenario() == "list" => return None,
        2600 => Step::Pick(SettingsKind::Sky, u(0x204)),
        2900 if lighting => Step::EditLighting,
        2900 => Step::Pick(SettingsKind::Water, u(0x302)),
        3200 if !lighting => Step::Next(SettingsKind::Sky),
        3500 if !lighting => Step::Pick(SettingsKind::Day, u(0x402)),
        3800 if !lighting => Step::Shared,
        _ => return None,
    };
    log::info!("demo environment selector: frame {frame}: {s:?}");
    Some(s)
}

/// The edit of [`Step::EditLighting`].
pub fn edit_lighting(s: &mut SkyFrame) -> bool {
    s.gamma = 1.6;
    s.haze_density = 2.0;
    s.sun_rotation = crate::ui::environment::rotation_from(250.0, 12.0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_assets_parse_with_the_right_types() {
        let assets = settings_assets();
        assert_eq!(assets.len(), ITEMS.len());
        for (_, _, kind, a) in ITEMS {
            let s = assets.iter().find(|(id, _)| *id == u(*a)).map(|(_, s)| s);
            let ok = match (kind, s) {
                (SettingsKind::Sky, Some(Settings::Sky(_))) | (SettingsKind::Water, Some(Settings::Water(_))) => true,
                (SettingsKind::Day, Some(Settings::Day(d))) => d.is_valid(),
                _ => false,
            };
            assert!(ok, "{a:x}");
        }
    }
}
