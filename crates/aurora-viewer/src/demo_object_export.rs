//! Offline objects of the object export scenario (AURORA_DEMO_OBJECT_EXPORT):
//! a garden lamp of four linked prims of ours. We created three of them;
//! the fourth (a torus) another resident did, so the window counts 3/4
//! exportable prims. The root bears a texture we uploaded (an item of ours
//! in the inventory), the sphere one we bought: one texture of two may be
//! saved. The post bears a PBR material we made (an item of ours: brushed
//! steel with our tiles as its base colour map, repeated by a transform),
//! the globe one we bought: one PBR material of two goes to a glTF file,
//! the globe keeping its texture entry. They go through the same
//! ObjectUpdate, ObjectProperties and inventory paths as objects of the
//! grid; `build/demo_sim.rs` answers their properties with these creators.

use super::*;
use aurora_net::inventory::InvItem;

pub const ROOT: u32 = 9500;
pub const POST: u32 = 9501;
pub const RING: u32 = 9502;
pub const GLOBE: u32 = 9503;

/// The resident who made the ring and sold us the globe's texture.
pub const OTHER_CREATOR: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0E01);
/// The inventory items of the two textures.
pub const OUR_TEXTURE_ITEM: Uuid = Uuid::from_u128(0xA0E0_0E10_0000_0000_0000_0000_0000_0001);
pub const BOUGHT_TEXTURE_ITEM: Uuid = Uuid::from_u128(0xA0E0_0E10_0000_0000_0000_0000_0000_0002);
/// The PBR materials of the post (ours) and of the globe (bought), and
/// their inventory items.
pub const OUR_MATERIAL: Uuid = Uuid::from_u128(0xA0E0_3A70_0000_0000_0000_0000_0000_0001);
pub const BOUGHT_MATERIAL: Uuid = Uuid::from_u128(0xA0E0_3A70_0000_0000_0000_0000_0000_0002);
pub const OUR_MATERIAL_ITEM: Uuid = Uuid::from_u128(0xA0E0_0E10_0000_0000_0000_0000_0000_0003);
pub const BOUGHT_MATERIAL_ITEM: Uuid = Uuid::from_u128(0xA0E0_0E10_0000_0000_0000_0000_0000_0004);

/// The materials the demo holds locally (scene::meshes::MaterialStreamer::insert).
pub fn materials() -> Vec<(Uuid, aurora_assets::material::PbrMaterial)> {
    use aurora_assets::material::{PbrMaterial, TextureTransform};
    let steel = PbrMaterial {
        base_color_factor: [0.85, 0.85, 0.9, 1.0],
        base_color_texture: Some(TEX_TILES),
        metallic_factor: 0.9,
        roughness_factor: 0.35,
        transforms: [
            TextureTransform {
                scale: [1.0, 4.0],
                ..Default::default()
            },
            TextureTransform::default(),
            TextureTransform::default(),
            TextureTransform::default(),
        ],
        ..Default::default()
    };
    let gold = PbrMaterial {
        base_color_factor: [1.0, 0.77, 0.34, 1.0],
        base_color_texture: Some(TEX_FRAMES),
        metallic_factor: 1.0,
        roughness_factor: 0.2,
        ..Default::default()
    };
    vec![(OUR_MATERIAL, steel), (BOUGHT_MATERIAL, gold)]
}

/// `AURORA_DEMO_OBJECT_EXPORT=collada|obj`: the window opened in that
/// format on the lamp.
pub fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_OBJECT_EXPORT").ok()).as_deref()
}

/// `AURORA_DEMO_OBJECT_EXPORT_FILE=<path>`: Save as writes there without
/// the file dialog (an automated run).
pub fn file() -> Option<std::path::PathBuf> {
    std::env::var_os("AURORA_DEMO_OBJECT_EXPORT_FILE")
        .filter(|f| !f.is_empty())
        .map(std::path::PathBuf::from)
}

/// The creator the demo simulator gives a prim of the lamp.
pub fn creator(local_id: u32) -> Option<Uuid> {
    match local_id {
        ROOT | POST | GLOBE => Some(DEMO_AGENT),
        RING => Some(OTHER_CREATOR),
        _ => None,
    }
}

pub fn id(local_id: u32) -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | local_id as u128)
}

/// The lamp, its root at `at` (region metres).
pub fn events(at: Vec3) -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let cylinder = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0);
    let torus = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, 25, 0, 0);
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let textured = |texture: Uuid, color: [f32; 4]| {
        let mut t = (*te(color, 0, false, 0.0)).clone();
        for f in t.faces.iter_mut() {
            f.texture = texture;
            f.scale_s = 2.0;
            f.scale_t = 2.0;
        }
        Arc::new(t)
    };
    // the base: an invisible bottom face, left out with "Skip transparent"
    let mut base_te = (*textured(TEX_TILES, [1.0; 4])).clone();
    base_te.faces[5].color[3] = 0.0;
    let root = prim(
        ROOT,
        at,
        Quat::IDENTITY,
        Vec3::new(0.8, 0.8, 0.2),
        boxp,
        Arc::new(base_te),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-object-export-lamp"),
    );
    let child = |local, offset: Vec3, rot: Quat, scale: Vec3, volume, t| {
        let mut o = prim(local, offset, rot, scale, volume, t, ExtraParams::default(), "");
        o.parent_id = ROOT;
        o
    };
    let mut post = child(
        POST,
        Vec3::new(0.0, 0.0, 0.8),
        Quat::IDENTITY,
        Vec3::new(0.12, 0.12, 1.4),
        cylinder,
        te([0.2, 0.22, 0.28, 1.0], 0, false, 0.0),
    );
    post.extra.render_materials = (0..3).map(|f| (f, OUR_MATERIAL)).collect();
    let ring = child(
        RING,
        Vec3::new(0.0, 0.0, 1.45),
        Quat::IDENTITY,
        Vec3::new(0.5, 0.5, 0.12),
        torus,
        te([0.545, 0.361, 0.965, 1.0], 0, false, 0.0),
    );
    let mut globe = child(
        GLOBE,
        Vec3::new(0.0, 0.0, 1.75),
        Quat::IDENTITY,
        Vec3::splat(0.45),
        sphere,
        textured(TEX_FRAMES, [1.0, 0.95, 0.8, 1.0]),
    );
    globe.extra.render_materials = vec![(0, BOUGHT_MATERIAL)];
    let mut objects = vec![root, post, ring, globe];
    for o in &mut objects {
        o.owner_id = DEMO_AGENT;
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// The texture and material items: ours (we made them) and bought ones.
pub fn inventory_items(parent: Uuid) -> Vec<InvItem> {
    let item = |id, name: &str, asset, creator, (asset_type, inv_type): (i32, i32)| InvItem {
        id,
        parent,
        name: name.into(),
        asset_type,
        inv_type,
        asset_id: asset,
        creator,
        owner: DEMO_AGENT,
        base_mask: 0x7fff_ffff,
        owner_mask: 0x7fff_ffff,
        next_owner_mask: 0x0008_2000,
        ..Default::default()
    };
    vec![
        item(OUR_TEXTURE_ITEM, "Garden tiles", TEX_TILES, DEMO_AGENT, (0, 0)),
        item(BOUGHT_TEXTURE_ITEM, "Bought frames", TEX_FRAMES, OTHER_CREATOR, (0, 0)),
        // AT_MATERIAL, LLInventoryType::IT_MATERIAL
        item(OUR_MATERIAL_ITEM, "Brushed steel", OUR_MATERIAL, DEMO_AGENT, (57, 26)),
        item(BOUGHT_MATERIAL_ITEM, "Bought gold", BOUGHT_MATERIAL, OTHER_CREATOR, (57, 26)),
    ]
}
