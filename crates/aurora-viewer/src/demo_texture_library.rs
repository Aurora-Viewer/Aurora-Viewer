//! Offline inventory and procedural PBR resources for the compact library.

use crate::{scene::Scene, ui::texture_library::catalogue, world::inventory::Inventory};
use aurora_assets::PbrMaterial;
use aurora_net::inventory::{InvFolder, InvItem};
use uuid::Uuid;

const BASE: u128 = 0xde40_a170_0000_0000_0000_0000_0000_0000;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(BASE + n)
}

pub fn first_texture() -> Uuid {
    id(300)
}

pub fn first_object() -> Uuid {
    id(900)
}

pub fn preview_items() -> [Uuid; 4] {
    [id(300), id(301), id(400), id(401)]
}

pub fn cascade_items(kind: i32) -> [Uuid; 6] {
    let start = match kind {
        57 => 400,
        7 => 600,
        10 => 700,
        _ => 300,
    };
    std::array::from_fn(|n| id(start + n as u128))
}

pub fn seed_cascade(inv: &mut Inventory, agent: Uuid, kind: i32) {
    if !matches!(kind, 7 | 10) {
        return;
    }
    let name = if kind == 7 {
        aurora_i18n::tr!("common-new-notecard")
    } else {
        aurora_i18n::tr!("common-demo-script")
    };
    inv.add_items(
        cascade_items(kind)
            .into_iter()
            .enumerate()
            .map(|(n, item)| InvItem {
                id: item,
                parent: inv.root,
                asset_id: id(800 + n as u128),
                asset_type: kind,
                inv_type: kind,
                name: format!("{name} {:02}", n + 1),
                owner: agent,
                creator: agent,
                base_mask: 0x7fffffff,
                owner_mask: 0x7fffffff,
                next_owner_mask: 0x7fffffff,
                ..Default::default()
            })
            .collect(),
    );
}

pub fn inventory_drag_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("AURORA_DEMO").as_deref() == Ok("1")
            && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
                .is_ok_and(|mode| matches!(mode.as_str(), "preview" | "preview-drag" | "inventory-drag"))
    })
}

pub fn seed(inv: &mut Inventory, scene: &mut Scene, agent: Uuid, empty: bool) {
    let aurora = catalogue::child(inv, inv.root, catalogue::AURORA).unwrap_or(id(1));
    let root = catalogue::root(inv).unwrap_or(id(2));
    let objects = catalogue::child(inv, root, "Objects").unwrap_or(id(4));
    inv.created_folders(vec![
        InvFolder {
            id: aurora,
            parent: inv.root,
            name: catalogue::AURORA.into(),
            type_default: -1,
            ..Default::default()
        },
        InvFolder {
            id: root,
            parent: aurora,
            name: catalogue::LIBRARY.into(),
            type_default: -1,
            ..Default::default()
        },
        InvFolder {
            id: objects,
            parent: root,
            name: "Objects".into(),
            type_default: -1,
            ..Default::default()
        },
        InvFolder {
            id: id(3),
            parent: inv.root,
            name: aurora_i18n::tr!("texture-library-demo-sources").into(),
            type_default: -1,
            ..Default::default()
        },
    ]);
    inv.created_folders(
        catalogue::DEFAULT_FOLDERS
            .into_iter()
            .enumerate()
            .filter(|(_, name)| *name != "Objects")
            .map(|(n, name)| InvFolder {
                id: id(30 + n as u128),
                parent: root,
                name: name.into(),
                type_default: -1,
                ..Default::default()
            })
            .collect(),
    );
    let labels = [
        aurora_i18n::tr!("texture-library-demo-wood"),
        aurora_i18n::tr!("texture-library-demo-stone"),
        aurora_i18n::tr!("texture-library-demo-metal"),
        aurora_i18n::tr!("texture-library-demo-fabric"),
    ];
    let mut items = Vec::new();
    for (group, label) in labels.iter().enumerate() {
        let folder = id(10 + group as u128);
        inv.created_folders(vec![InvFolder {
            id: folder,
            parent: if empty {
                id(3)
            } else if group < 2 {
                root
            } else {
                objects
            },
            name: (*label).into(),
            type_default: -1,
            ..Default::default()
        }]);
        let nested = if group == 1 {
            let nested = id(14);
            inv.created_folders(vec![InvFolder {
                id: nested,
                parent: folder,
                name: aurora_i18n::tr!("texture-library-demo-polished").into(),
                type_default: -1,
                ..Default::default()
            }]);
            Some(nested)
        } else {
            None
        };
        for variant in 0..3 {
            let map = id(100 + group as u128 * 3 + variant);
            let mat = id(200 + group as u128 * 3 + variant);
            scene.materials.insert(
                mat,
                PbrMaterial {
                    base_color_texture: Some(map),
                    metallic_factor: if group == 2 { 1.0 } else { 0.0 },
                    roughness_factor: [0.72, 0.22, 0.28, 0.96][group] + variant as f32 * 0.02,
                    ..Default::default()
                },
            );
            for (material, asset, number) in [(false, map, 300), (true, mat, 400)] {
                items.push(InvItem {
                    id: id(number + group as u128 * 3 + variant),
                    parent: nested.filter(|_| variant == 2).unwrap_or(folder),
                    asset_id: asset,
                    asset_type: if material { 57 } else { 0 },
                    inv_type: if material { 26 } else { 0 },
                    name: format!("{label} {:02}{}", variant + 1, if material { " · PBR" } else { "" }),
                    owner: agent,
                    creator: agent,
                    base_mask: 0x7fffffff,
                    owner_mask: 0x7fffffff,
                    next_owner_mask: 0x7fffffff,
                    ..Default::default()
                });
            }
        }
    }
    let folder = id(15);
    inv.created_folders(vec![InvFolder {
        id: folder,
        parent: if empty { id(3) } else { objects },
        name: aurora_i18n::tr!("texture-library-objects").into(),
        type_default: -1,
        ..Default::default()
    }]);
    for (n, label) in [
        aurora_i18n::tr!("texture-library-demo-object-image"),
        aurora_i18n::tr!("texture-library-demo-object"),
        aurora_i18n::tr!("texture-library-demo-object-group"),
        aurora_i18n::tr!("texture-library-demo-object-group-image"),
    ]
    .into_iter()
    .enumerate()
    {
        items.push(InvItem {
            id: id(900 + n as u128),
            parent: if n % 2 == 0 || empty { folder } else { root },
            asset_type: 6,
            inv_type: 6,
            name: format!("{} {label}", n + 1),
            thumbnail: if n == 0 || n == 3 { id(100 + n as u128) } else { Uuid::nil() },
            flags: if n >= 2 { 0x200000 } else { 0 },
            owner: agent,
            creator: agent,
            base_mask: 0x7fffffff,
            owner_mask: if n == 3 {
                0x7fffffff & !crate::world::inventory::actions::COPY
            } else {
                0x7fffffff
            },
            next_owner_mask: 0x7fffffff,
            ..Default::default()
        });
    }
    for (n, kind) in [20, 1, 7, 3, 10].into_iter().enumerate() {
        let folder = ["Animations", "Sounds", "Notecards", "Landmarks", "Scripts"][n];
        items.push(InvItem {
            id: id(950 + n as u128),
            parent: if empty {
                id(3)
            } else {
                catalogue::child(inv, root, folder).unwrap_or(root)
            },
            asset_type: kind,
            inv_type: if kind == 20 { 19 } else { kind },
            name: folder.into(),
            owner: agent,
            creator: agent,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            next_owner_mask: 0x7fffffff,
            ..Default::default()
        });
    }
    inv.add_items(items);
}

/// Tileable content pixels, independent of interface palette colors.
pub fn texture(uuid: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    let number = uuid.as_u128().checked_sub(BASE + 100)?;
    if number >= 12 {
        return None;
    }
    let (group, variant) = (number / 3, number % 3);
    let mut pixels = Vec::with_capacity(128 * 128 * 4);
    let tau = std::f32::consts::TAU;
    for y in 0..128 {
        for x in 0..128 {
            let (u, v) = (x as f32 / 128.0, y as f32 / 128.0);
            let noise = ((x * 19 + y * 29 + x * y * 13) % 31) as f32 / 31.0;
            let tint = variant as f32 * 0.12;
            let color = match group {
                0 => {
                    let grain = (u * tau * 11.0 + (v * tau * 2.0).sin() * 2.0).sin() * 0.10 + noise * 0.035;
                    [0.52 + grain - tint, 0.34 + grain * 0.7 - tint * 0.6, 0.19 + grain * 0.4]
                }
                1 => {
                    let vein = (u * tau * 3.0 + (v * tau * 2.0).sin() * 1.6).sin().abs().powf(13.0);
                    let c = 0.84 - vein * 0.33 - tint + noise * 0.035;
                    [c, c * 0.98, c * 0.94]
                }
                2 => {
                    let c = 0.52 + (v * tau * 60.0).sin() * 0.045 + noise * 0.03;
                    [c + tint, c, c * 0.91 - tint]
                }
                _ => {
                    let weave = if (x + y) % 4 < 2 { 0.05 } else { -0.05 };
                    [0.28 + tint + weave, 0.41 - tint + weave, 0.40 + weave]
                }
            };
            pixels.extend(color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
            pixels.push(255);
        }
    }
    Some((pixels, 128, 128))
}
