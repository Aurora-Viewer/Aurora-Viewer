//! Offline demo scene (`AURORA_DEMO=1`): a synthetic region fed through the
//! same event path as the network, used to test rendering without a grid.

use aurora_llsd::Llsd;
use aurora_net::objects::ObjectUpdate;
use aurora_net::terrain::TerrainPatch;
use aurora_net::{LoginResponse, NetEvent, RegionInfo, SunInfo};
use aurora_prim::params::*;
use aurora_prim::te::{BLANK_TEXTURE, TextureEntry};
use aurora_prim::{ExtraParams, LightParams, RawShape};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const HANDLE: u64 = (256000u64 << 32) | 256000;

/// The local asset server of its `net=` option.
#[path = "demo_asset_server.rs"]
pub mod asset_server;
/// EEP environment answers (AURORA_DEMO_EEP_PARCEL).
#[path = "demo_chatlog.rs"]
pub mod chatlog;
#[path = "demo_eep.rs"]
pub mod eep;
/// Environment selector library and script (AURORA_DEMO_ENV_SELECT).
#[path = "demo_env.rs"]
pub mod env;
/// Meshes filling the geometry arena until it outgrows its buffers
/// (AURORA_DEMO_GEOMETRY).
#[path = "demo_geometry.rs"]
pub mod geometry;
/// Names and chat in the characters residents use (AURORA_DEMO_GLYPHS).
#[path = "demo_glyphs.rs"]
pub mod glyphs;
#[path = "demo_hud.rs"]
pub mod hud;
/// About Land answers (AURORA_DEMO_LAND).
#[path = "demo_land.rs"]
pub mod land;
/// Place details of place links (AURORA_DEMO_PLACE).
#[path = "demo_place.rs"]
pub mod place;
/// Directory answers of the search window (AURORA_DEMO_SEARCH).
#[path = "demo_search.rs"]
pub mod search;
/// Streaming hitch test: objects and textures arriving in waves
/// (AURORA_DEMO_STREAM).
#[path = "demo_stream.rs"]
pub mod stream;
/// Name plates in every state and at each distance (AURORA_DEMO_TAGS).
#[path = "demo_tags.rs"]
pub mod tags;
/// A worn object of ours (AURORA_DEMO_WORN).
#[path = "demo_worn.rs"]
pub mod worn;

/// Walkable floor of the demo (plaza top, else the terrain), for the
/// offline movement stand-in.
pub fn floor_at(x: f32, y: f32) -> f32 {
    let (cx, cy) = (140.0, 130.0);
    let terrain = height(x, y);
    if (x - cx).hypot(y - cy) <= 12.0 {
        terrain.max(height(cx, cy) + 0.35)
    } else {
        terrain
    }
}

/// AURORA_DEMO_ALTITUDE=<m>: the whole demo place (terrain, objects,
/// avatars) raised by that much, to see what single precision does to the
/// picture high above the ground (0.24 mm steps between 2048 and 4096 m).
fn altitude() -> f32 {
    static ALTITUDE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *ALTITUDE.get_or_init(|| {
        std::env::var("AURORA_DEMO_ALTITUDE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(0.0, |v| v.clamp(0.0, 4000.0))
    })
}

/// AURORA_DEMO_AGENT_JITTER=<mm>: our avatar receives small position
/// corrections a few times per second, as a simulator sends for an avatar
/// standing high above the ground. The value is their largest size.
pub fn agent_jitter() -> Option<f32> {
    static JITTER: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *JITTER.get_or_init(|| {
        std::env::var("AURORA_DEMO_AGENT_JITTER")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| v.min(100.0))
    })
}

/// The correction of AURORA_DEMO_AGENT_JITTER at a frame: a new one every
/// 12 frames (5 per second at 60 images/s), the same for a given frame.
pub fn agent_jitter_offset(frame: u64, mm: f32) -> Vec3 {
    let step = frame / 12;
    // (a fixed integer hash: the scenario replays the same corrections)
    let unit = |salt: u64| {
        let mut h = step
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03));
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        (h % 2001) as f32 / 1000.0 - 1.0
    };
    Vec3::new(unit(1), unit(2), unit(3)) * (mm * 0.001)
}

fn height(x: f32, y: f32) -> f32 {
    let hill = 14.0 * (-((x - 190.0).powi(2) + (y - 170.0).powi(2)) / 2500.0).exp();
    let dune = 2.5 * (x / 23.0).sin() * (y / 31.0).cos();
    // a beach and a lagoon to the west (water at 20 m)
    let shore = ((x - 75.0) / 9.0).clamp(-6.0, 9.0);
    21.5 + shore + hill + dune + altitude()
}

fn te(color: [f32; 4], shiny: u8, fullbright: bool, glow: f32) -> Arc<TextureEntry> {
    let mut t = TextureEntry::default();
    for f in t.faces.iter_mut() {
        f.texture = BLANK_TEXTURE;
        f.color = color;
        f.bump_shiny_fullbright = (shiny << 6) | if fullbright { 0x20 } else { 0 };
        f.glow = glow;
    }
    Arc::new(t)
}

fn shape(path: u8, profile: u8, sy: u8, hollow: u16, cut_end: u16) -> aurora_prim::VolumeParams {
    RawShape {
        path_curve: path,
        profile_curve: profile,
        path_scale_x: 100,
        path_scale_y: sy,
        profile_hollow: hollow,
        profile_end: cut_end,
        ..Default::default()
    }
    .to_params()
}

#[allow(clippy::too_many_arguments)]
fn prim(
    id: u32,
    pos: Vec3,
    rot: Quat,
    scale: Vec3,
    volume: aurora_prim::VolumeParams,
    t: Arc<TextureEntry>,
    extra: ExtraParams,
    text: &str,
) -> ObjectUpdate {
    ObjectUpdate {
        local_id: id,
        full_id: Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | id as u128),
        parent_id: 0,
        pcode: LL_PCODE_VOLUME,
        state: 0,
        crc: 0,
        material: 3,
        click_action: 0,
        scale,
        position: pos,
        rotation: rot,
        velocity: Vec3::ZERO,
        acceleration: Vec3::ZERO,
        angular_velocity: Vec3::ZERO,
        update_flags: 0,
        owner_id: Uuid::nil(),
        volume,
        texture_entry: Some(t),
        extra,
        name_values: String::new(),
        text: text.to_owned(),
        text_color: [232, 237, 251, 255],
        media_url: String::new(),
        tree_species: None,
        texture_anim: None,
        foot_plane: None,
        particles: Default::default(),
        sound: Default::default(),
    }
}

/// Synthetic inventory rez stand-in: a cube, never a grid asset download.
pub fn inventory_rez(rez: &aurora_net::inventory::rez::RezInventory) -> ObjectUpdate {
    let id = Uuid::new_v4();
    let mut o = prim(
        id.as_u128() as u32,
        rez.ray_end + Vec3::Z * 0.5,
        Quat::IDENTITY,
        Vec3::ONE,
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.35, 0.85, 0.75, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        &rez.item.name,
    );
    o.full_id = id;
    o.owner_id = DEMO_AGENT;
    o.update_flags = crate::ui::context::flags::OBJECT_MODIFY
        | crate::ui::context::flags::OBJECT_COPY
        | crate::ui::context::flags::OBJECT_YOU_OWNER
        | if rez.selected { 1 << 1 } else { 0 };
    o
}

/// A basic prim created in the offline builder, with the same missing OwnerID
/// as ordinary simulator updates. Selection and permissions use update flags.
pub fn build_rez(p: &aurora_net::build::NewPrim) -> ObjectUpdate {
    let id = Uuid::new_v4();
    let mut object = prim(
        id.as_u128() as u32,
        p.ray_end + Vec3::Z * (p.scale.z * 0.5),
        p.rotation,
        p.scale,
        p.shape.to_params(),
        te([1.0; 4], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    object.full_id = id;
    object.pcode = p.pcode;
    object.state = p.state;
    object.material = p.material;
    object.update_flags = crate::ui::context::flags::OBJECT_YOU_OWNER
        | crate::ui::context::flags::OBJECT_ANY_OWNER
        | crate::ui::context::flags::OBJECT_MODIFY
        | crate::ui::context::flags::OBJECT_COPY
        | crate::ui::context::flags::OBJECT_MOVE
        | p.add_flags;
    object
}

/// Legacy particle system block (LLPartSysData::packLegacy + LLPartData):
/// a teal fountain fading to violet.
fn fountain_particles() -> Vec<u8> {
    use aurora_prim::particles::*;
    let ufix8 = |v: f32, frac: u32| (v * (1u32 << frac) as f32) as u8;
    let ufix16 = |v: f32, frac: u32| ((v * (1u32 << frac) as f32) as u16).to_le_bytes();
    let sfix16 = |v: f32| (((v + 256.0) * 128.0) as u16).to_le_bytes();
    let mut out = Vec::with_capacity(PS_LEGACY_DATA_BLOCK_SIZE);
    out.extend_from_slice(&0xA0E0_0001u32.to_le_bytes()); // crc
    out.extend_from_slice(&LL_PART_USE_NEW_ANGLE.to_le_bytes());
    out.push(LL_PART_SRC_PATTERN_ANGLE_CONE);
    out.extend_from_slice(&ufix16(0.0, 8)); // max age (forever)
    out.extend_from_slice(&ufix16(0.0, 8)); // start age
    out.push(ufix8(0.0, 5)); // inner angle
    out.push(ufix8(0.25, 5)); // outer angle
    out.extend_from_slice(&ufix16(0.05, 8)); // burst rate
    out.extend_from_slice(&ufix16(0.05, 8)); // burst radius
    out.extend_from_slice(&ufix16(4.0, 8)); // speed min
    out.extend_from_slice(&ufix16(5.0, 8)); // speed max
    out.push(4); // particles per burst
    for v in [0.0, 0.0, 0.0] {
        out.extend_from_slice(&sfix16(v)); // angular velocity
    }
    for v in [0.0, 0.0, -7.0] {
        out.extend_from_slice(&sfix16(v)); // acceleration
    }
    out.extend_from_slice(Uuid::nil().as_bytes()); // texture
    out.extend_from_slice(Uuid::nil().as_bytes()); // target
    // particle data
    let flags = LL_PART_INTERP_COLOR_MASK | LL_PART_INTERP_SCALE_MASK | LL_PART_EMISSIVE_MASK;
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&ufix16(1.6, 8)); // particle max age
    out.extend_from_slice(&[94, 234, 212, 230]); // start color (teal)
    out.extend_from_slice(&[139, 92, 246, 0]); // end color (violet, transparent)
    out.push(ufix8(0.18, 5));
    out.push(ufix8(0.18, 5));
    out.push(ufix8(0.45, 5));
    out.push(ufix8(0.45, 5));
    out
}

/// Motorcycle wind / smoke script, using a default soft dot offline. The
/// simulator clamps the wind's requested 5 m height to MAX_PART_SCALE (4 m).
/// Layout follows LLPartSysData::unpack / LLPartData::unpack (llpartdata.cpp).
fn scripted_particles(wind: bool, legacy: bool) -> Vec<u8> {
    use aurora_prim::particles::*;
    let ufix16 = |v: f32| ((v * 256.0) as u16).to_le_bytes();
    let sfix16 = |v: f32| (((v + 256.0) * 128.0) as u16).to_le_bytes();
    let mut out = Vec::new();
    if !legacy {
        out.extend_from_slice(&(PS_SYS_DATA_BLOCK_SIZE as u32).to_le_bytes());
    }
    out.extend_from_slice(&123u32.to_le_bytes());
    out.extend_from_slice(&LL_PART_USE_NEW_ANGLE.to_le_bytes());
    out.push(LL_PART_SRC_PATTERN_ANGLE);
    out.extend_from_slice(&[0; 4]); // source max / start age
    out.extend_from_slice(&[0, (std::f32::consts::PI * 32.0) as u8]); // quantized angle 3.14
    for v in [0.1, 0.9, 0.0, 0.0] {
        // rate, radius, min / max speed
        out.extend_from_slice(&ufix16(v));
    }
    out.push(3);
    for v in [0.0, 0.0, 0.0, 0.0, if wind { -80.0 } else { 0.0 }, if wind { 0.0 } else { 0.2 }] {
        out.extend_from_slice(&sfix16(v));
    }
    out.extend_from_slice(Uuid::nil().as_bytes()); // default dot, no asset fetch
    out.extend_from_slice(Uuid::nil().as_bytes()); // target unused by these flags
    if !legacy {
        out.extend_from_slice(&20u32.to_le_bytes()); // 18 legacy bytes + glow
    }
    let flags = LL_PART_EMISSIVE_MASK | LL_PART_FOLLOW_VELOCITY_MASK | LL_PART_INTERP_COLOR_MASK | LL_PART_INTERP_SCALE_MASK;
    out.extend_from_slice(&(flags | if legacy { 0 } else { LL_PART_DATA_GLOW }).to_le_bytes());
    out.extend_from_slice(&ufix16(2.0));
    out.extend_from_slice(if wind { &[255, 255, 255, 51] } else { &[141, 15, 142, 77] });
    out.extend_from_slice(if wind { &[255, 255, 255, 0] } else { &[200, 106, 134, 0] });
    let scales = if wind {
        [0.125, MAX_PART_SCALE, 0.05, 2.5]
    } else {
        [0.5, 0.5, 0.125, 0.125]
    };
    for v in scales {
        out.push((v * 32.0) as u8);
    }
    if !legacy {
        out.extend_from_slice(&[3, 0]); // start glow 0.01, end glow 0
    }
    out
}

/// Feed the extended block through the compressed-object decoder, including
/// its position after texture data, as LLVOVolume::processUpdateMessage does.
fn compressed_scripted_particles(wind: bool, legacy: bool) -> Option<aurora_net::objects::ParticleUpdate> {
    let block = scripted_particles(wind, legacy);
    let mut data = vec![0; 84];
    data[20] = LL_PCODE_VOLUME;
    data[64..68].copy_from_slice(&(if legacy { 0x8u32 } else { 0x400u32 }).to_le_bytes());
    if legacy {
        data.extend_from_slice(&block);
    }
    data.push(0); // extra params
    data.extend_from_slice(&[0; 23]); // shape (demo object supplies its geometry)
    data.extend_from_slice(&0u32.to_le_bytes()); // texture entry
    if !legacy {
        data.extend_from_slice(&block);
    }
    aurora_net::objects::parse_compressed(&data, 0).map(|o| o.particles)
}

/// First friend of the demo buddy list (AURORA_DEMO_PROFILE=friend).
pub const DEMO_FRIEND: Uuid = Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0000 | 100);

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0000 | n)
}

/// Tiny offline attachment used to exercise outfit changes without grid assets.
pub fn outfit_attachment(item: &aurora_net::AttachRequest) -> ObjectUpdate {
    let local_id = 30_000 + (item.item_id.as_u128() & 0xffff) as u32;
    let mut o = prim(
        local_id,
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(0.001),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.0, 0.0, 0.0, 0.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    o.full_id = item.item_id;
    o.parent_id = 9000;
    let point = if item.point == 0 {
        ((item.flags & 0xff) as u8).max(1)
    } else {
        item.point
    };
    o.state = point.rotate_right(4);
    o.name_values = format!("AttachItemID STRING RW SV {}", item.item_id);
    o
}

fn demo_raw() -> Llsd {
    use aurora_llsd::llsd_map;
    let folder = |id: u128, parent: u128, name: &str, t: i32| {
        llsd_map! {"folder_id" => u(id), "parent_id" => u(parent), "name" => name, "type_default" => t, "version" => 1}
    };
    let mut skel = Vec::new();
    skel.push(folder(1, 0, aurora_i18n::tr!("common-my-inventory"), 8));
    for (i, (name, t)) in [
        (aurora_i18n::tr!("common-animations"), 20),
        (aurora_i18n::tr!("demo-clothes"), 5),
        (aurora_i18n::tr!("common-landmarks"), 3),
        (aurora_i18n::tr!("common-notes"), 7),
        (aurora_i18n::tr!("common-objects"), 6),
        (aurora_i18n::tr!("common-scripts"), 10),
        (aurora_i18n::tr!("common-textures"), 0),
        (aurora_i18n::tr!("common-trash"), 14),
        (aurora_i18n::tr!("common-my-outfits"), 48),
        ("Materials", 57),
        (aurora_i18n::tr!("common-favorites"), 23),
    ]
    .iter()
    .enumerate()
    {
        skel.push(folder(10 + i as u128, 1, name, *t));
    }
    skel.push(folder(
        place::SHOPS_FOLDER,
        place::LANDMARKS_FOLDER,
        aurora_i18n::tr!("common-shopping"),
        -1,
    ));
    if env::enabled() {
        skel.push(env::settings_folder(u(1)));
    }
    let buddies: Vec<Llsd> = (0..4)
        .map(|i| llsd_map! {"buddy_id" => u(100 + i), "buddy_rights_given" => [1, 3, 7, 0][i as usize], "buddy_rights_has" => if i % 2 == 0 { 3 } else { 1 }})
        .collect();
    let mut raw = llsd_map! {
        "inventory-root" => Llsd::Array(vec![llsd_map!{"folder_id" => u(1)}]),
        // an adult account that wants every rating (the search window's boxes)
        "agent_access_max" => "A",
        "agent_region_access" => "A",
        "inventory-skeleton" => Llsd::Array(skel),
        "buddy-list" => Llsd::Array(buddies),
        "classified_categories" => Llsd::Array(
            [(1, aurora_i18n::tr!("common-shopping-2")), (2, "Land Rental"), (3, "Property Rental"), (4, aurora_i18n::tr!("demo-special-attraction"))]
                .into_iter()
                .map(|(id, name)| llsd_map! {"category_id" => id, "category_name" => name})
                .collect(),
        ),
    };
    if env::enabled()
        && let Llsd::Map(m) = &mut raw
    {
        env::library_login(m);
    }
    raw
}

/// Fake replies for name / inventory requests in demo mode.
/// Demo residents and groups (block list and group chat).
pub const DEMO_AGENT: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
pub const DEMO_LOUP: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002);
pub const DEMO_NOVA: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0003);
pub const DEMO_TESS: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0004);
pub const DEMO_GROUP1: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0001);
pub const DEMO_GROUP2: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0002);
/// Hidden from the profile (Contacts › Groups shows it in the other color).
pub const DEMO_GROUP3: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0003);
/// AURORA_DEMO_CONV=noms: residents and a conference whose names do not fit
/// in the lists (long display name and username, wide letters, long title).
pub const DEMO_LONG_NAME: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0010);
pub const DEMO_WIDE_NAME: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0011);
pub const DEMO_LONG_SESSION: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0010);
const LONG_SESSION_NAME: &str = "The Honourable Society of Extremely Long Conference Names (Northern Chapter)";

/// (legacy first, legacy last, display name) of the long-named residents.
fn long_name(id: &Uuid) -> Option<(&'static str, &'static str, &'static str)> {
    match *id {
        DEMO_LONG_NAME => Some((
            "Maximilienne",
            "Wolfeschlegelsteinhausen",
            "Maximilienne ♪ the Wandering Northern Aurora Wolf",
        )),
        DEMO_WIDE_NAME => Some(("wwwmmmwwwmmmwwwmmm", "Resident", "ŴŴŴ ÆØÅ MMMMWWWWMMMM ŒŒŒŒ")),
        _ => None,
    }
}

/// AURORA_DEMO_CONV=names[:narrow|wide]: the long names scenario, and the
/// sizes of the Conversations and People windows it is shown in.
pub fn long_names_scenario() -> Option<(egui::Vec2, egui::Vec2)> {
    let v = std::env::var("AURORA_DEMO_CONV").ok()?;
    let (mode, width) = v.split_once(':').unwrap_or((v.as_str(), ""));
    if !matches!(mode, "names" | "noms") {
        return None;
    }
    let size = |w, h| egui::vec2(w, h);
    Some(match width {
        "narrow" | "etroit" => (size(400.0, 300.0), size(230.0, 420.0)),
        "wide" | "large" => (size(900.0, 360.0), size(520.0, 420.0)),
        _ => (size(600.0, 360.0), size(330.0, 420.0)),
    })
}

/// AURORA_DEMO_CONV=many: more conversations than the list of the
/// Conversations window shows at once.
pub const MANY_CONVERSATIONS: u32 = 24;

pub fn many_conversation(i: u32) -> Uuid {
    Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0001_0000_0000 | i as u128)
}

pub fn many_conversations_scenario() -> bool {
    std::env::var("AURORA_DEMO_CONV").is_ok_and(|v| v == "many")
}

/// One instant message from each of `MANY_CONVERSATIONS` residents.
pub fn many_conversation_events(agent: Uuid) -> Vec<NetEvent> {
    (0..MANY_CONVERSATIONS)
        .map(|i| {
            NetEvent::InstantMessage(aurora_net::InstantMessage {
                from_agent_id: many_conversation(i),
                from_name: format!("Resident{:02} Demo", i + 1),
                to_agent_id: agent,
                dialog: 0,
                session_id: Uuid::nil(),
                message: aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset").into(),
                offline: false,
                from_group: false,
                binary_bucket: Vec::new(),
                timestamp: 0,
            })
        })
        .collect()
}

/// AURORA_DEMO_CONV=bubbles[:busy|:group|:local|:legacy]: the
/// Conversations window as a messenger (ui/chat/bubbles.rs). Without a
/// suffix the conversation with an online friend, over two days; `busy` the
/// one with a resident nearby in do not disturb, after their automatic
/// response; `group` the demo group chat; `local` the nearby chat; `legacy`
/// the friend's conversation in the legacy lines (`ChatStyle::Legacy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bubbles {
    Friend,
    Busy,
    Group,
    Local,
    Legacy,
}

pub fn bubbles_scenario() -> Option<Bubbles> {
    let v = std::env::var("AURORA_DEMO_CONV").ok()?;
    Some(match v.strip_prefix("bubbles")? {
        "" => Bubbles::Friend,
        ":busy" => Bubbles::Busy,
        ":group" => Bubbles::Group,
        ":local" => Bubbles::Local,
        ":legacy" => Bubbles::Legacy,
        _ => return None,
    })
}

/// The conversation of the scenario shown in the window.
pub fn bubbles_conversation(view: Bubbles) -> Option<Uuid> {
    match view {
        Bubbles::Friend | Bubbles::Legacy => Some(DEMO_FRIEND),
        Bubbles::Busy => Some(DEMO_NOVA),
        Bubbles::Group => Some(DEMO_GROUP1),
        Bubbles::Local => None,
    }
}

/// What the network says for the scenario: a resident in the region playing the
/// do not disturb animation; a few lines of
/// nearby chat from residents and an object.
pub fn bubbles_events() -> Vec<NetEvent> {
    let mut ev = vec![
        NetEvent::CoarseLocations {
            handle: HANDLE,
            positions: vec![
                (DEMO_TESS, Vec3::new(96.0, 150.0, 28.0)),
                (DEMO_NOVA, Vec3::new(150.0, 140.0, 24.0)),
            ],
        },
        NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: DEMO_NOVA,
            anims: vec![(crate::world::status::ANIM_DO_NOT_DISTURB, 1)],
        },
    ];
    let say = |from: Uuid, name: &str, text: &str, object: bool| {
        NetEvent::Chat(aurora_net::ChatMessage {
            from_name: name.into(),
            source_id: from,
            owner_id: Uuid::nil(),
            source_type: if object {
                aurora_net::ChatSourceType::Object
            } else {
                aurora_net::ChatSourceType::Agent
            },
            chat_type: aurora_net::ChatType::Normal,
            position: Vec3::ZERO,
            message: text.into(),
        })
    };
    ev.push(say(
        DEMO_NOVA,
        aurora_i18n::tr!("demo-nova-example"),
        aurora_i18n::tr!("demo-bubbles-local-1"),
        false,
    ));
    ev.push(say(
        DEMO_NOVA,
        aurora_i18n::tr!("demo-nova-example"),
        aurora_i18n::tr!("demo-bubbles-local-2"),
        false,
    ));
    ev.push(say(
        u(200),
        aurora_i18n::tr!("common-demo-hud"),
        aurora_i18n::tr!("demo-bubbles-local-3"),
        true,
    ));
    ev
}

/// The lines said before the scenario starts (yesterday's among them) and
/// ours: they are written in the conversations as they would have been.
pub fn bubbles_lines(world: &mut crate::world::World, view: Bubbles) {
    use crate::world::{ChatKind, ChatLine, Origin};
    let now = std::time::SystemTime::now();
    let ago = |minutes: u64| now - std::time::Duration::from_secs(minutes * 60);
    let me = world.own_name();
    let line = |time, from: &str, source: Uuid, kind, text: &str| ChatLine {
        time,
        from: from.to_owned(),
        text: text.to_owned(),
        kind,
        source,
        origin: Origin::Live,
    };
    match view {
        Bubbles::Friend | Bubbles::Legacy => {
            let friend = DEMO_FRIEND;
            let name = "Nova Example";
            for l in [
                line(
                    ago(26 * 60),
                    &me,
                    Uuid::nil(),
                    ChatKind::Own,
                    aurora_i18n::tr!("demo-bubbles-concert"),
                ),
                line(ago(26 * 60 - 3), name, friend, ChatKind::Im, aurora_i18n::tr!("demo-bubbles-cant")),
                line(ago(12), name, friend, ChatKind::Im, aurora_i18n::tr!("demo-bubbles-sorry")),
                line(ago(11), name, friend, ChatKind::Im, aurora_i18n::tr!("demo-bubbles-join")),
                line(ago(2), &me, Uuid::nil(), ChatKind::Own, aurora_i18n::tr!("demo-bubbles-coming")),
                line(ago(1), &me, Uuid::nil(), ChatKind::Own, aurora_i18n::tr!("demo-bubbles-send-tp")),
            ] {
                world.conversation_line(friend, l);
            }
        }
        Bubbles::Busy => {
            // ours, before her automatic response (which the events bring)
            world.conversation_line(
                DEMO_NOVA,
                line(ago(1), &me, Uuid::nil(), ChatKind::Own, aurora_i18n::tr!("demo-bubbles-free")),
            );
            // her automatic response, as world/status.rs writes it
            let reply = aurora_i18n::tr!(
                "world-status-auto-response",
                value0 = aurora_i18n::tr!("demo-bubbles-busy-reply").to_string()
            );
            world.conversation_line(DEMO_NOVA, line(now, "Tess Touch", DEMO_NOVA, ChatKind::Im, &reply));
        }
        Bubbles::Group | Bubbles::Local => {}
    }
}

/// AURORA_DEMO_CONV=tp[:sent|reject]: a conversation with a resident
/// (DEMO_NOVA, whom the demo's names call "Tess Touch"), then (issue #280)
/// the invitation of a teleport offer to them (`tp`), the offer sent to
/// them and to DEMO_TESS, who has no conversation (`tp:sent`), or a
/// teleport request refused while we reject teleports (`tp:reject`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TeleportOfferScene {
    Invitation,
    Sent,
    Reject,
}

pub fn teleport_offer_scene() -> Option<TeleportOfferScene> {
    match std::env::var("AURORA_DEMO_CONV").ok()?.as_str() {
        "tp" => Some(TeleportOfferScene::Invitation),
        "tp:sent" => Some(TeleportOfferScene::Sent),
        "tp:reject" => Some(TeleportOfferScene::Reject),
        _ => None,
    }
}

fn teleport_offer_scenario() -> bool {
    teleport_offer_scene().is_some()
}

/// AURORA_DEMO_LURE=offer|request|god|answers|reject: teleports received
/// from Tess Touch (DEMO_NOVA in the demo's names), with the notification
/// list open: an offer to the plaza, a request (Yes / No), a god's lure
/// (accepted at once), the answers to our offers (declined in her open
/// conversation, accepted while none is open), or an offer and a request
/// while we reject teleports.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LureScene {
    Offer,
    Request,
    God,
    Answers,
    Reject,
    /// The offer's card, her conversation in sight (no toast).
    OfferCard,
    /// The request's card, her conversation in sight.
    RequestCard,
    /// The offer accepted from its card: "Accepted", the entry gone.
    CardAnswered,
    /// The offer declined from the notification centre: the card says so.
    NotificationAnswered,
    /// The offer's card in the legacy lines.
    Legacy,
}

impl LureScene {
    /// Her conversation is open on the card.
    pub fn shows_card(self) -> bool {
        !matches!(self, LureScene::Offer | LureScene::Request | LureScene::God | LureScene::Reject)
    }
}

pub fn lure_scene() -> Option<LureScene> {
    match std::env::var("AURORA_DEMO_LURE").ok()?.as_str() {
        "offer" => Some(LureScene::Offer),
        "request" => Some(LureScene::Request),
        "god" => Some(LureScene::God),
        "answers" => Some(LureScene::Answers),
        "reject" => Some(LureScene::Reject),
        "offer-card" => Some(LureScene::OfferCard),
        "request-card" => Some(LureScene::RequestCard),
        "card-answered" => Some(LureScene::CardAnswered),
        "notification-answered" => Some(LureScene::NotificationAnswered),
        "legacy" => Some(LureScene::Legacy),
        _ => None,
    }
}

/// The instant messages of `lure_scene`, from Tess Touch.
pub fn lure_events(scene: LureScene, agent: Uuid) -> Vec<NetEvent> {
    use crate::world::notifications::im as d;
    let im = |dialog: u8, message: &str, bucket: &[u8]| {
        NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: DEMO_NOVA,
            from_name: "Tess Touch".into(),
            to_agent_id: agent,
            dialog,
            session_id: Uuid::from_u128(0x1A3E_0000_0000_0000_0000_0000_0000_0001),
            message: message.into(),
            offline: false,
            from_group: false,
            binary_bucket: bucket.to_vec(),
            timestamp: 0,
        })
    };
    // the plaza, as a lure's bucket carries it (parse_lure_bucket)
    let plaza = b"256000|256000|128|130|25|0|1|0|PG";
    let invitation = format!(
        "{}\r\nhttp://maps.secondlife.com/secondlife/Aurora%20Demo/128/130/25",
        aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset")
    );
    match scene {
        LureScene::Offer | LureScene::OfferCard | LureScene::CardAnswered | LureScene::NotificationAnswered | LureScene::Legacy => {
            vec![im(d::LURE_USER, &invitation, plaza)]
        }
        LureScene::Request | LureScene::RequestCard => vec![im(d::TELEPORT_REQUEST, aurora_i18n::tr!("demo-can-i-join-you"), b"")],
        LureScene::God => vec![im(d::GODLIKE_LURE_USER, &invitation, plaza)],
        LureScene::Answers => vec![
            im(0, aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset"), b""),
            im(d::LURE_DECLINED, "", b""),
            NetEvent::InstantMessage(aurora_net::InstantMessage {
                from_agent_id: DEMO_TESS,
                from_name: "Nova Example".into(),
                to_agent_id: agent,
                dialog: d::LURE_ACCEPTED,
                session_id: Uuid::nil(),
                message: String::new(),
                offline: false,
                from_group: false,
                binary_bucket: Vec::new(),
                timestamp: 0,
            }),
        ],
        LureScene::Reject => vec![
            im(d::LURE_USER, &invitation, plaza),
            im(d::TELEPORT_REQUEST, aurora_i18n::tr!("demo-can-i-join-you"), b""),
        ],
    }
}

/// The message that opens the conversation of `teleport_offer_scenario`.
pub fn teleport_offer_events(agent: Uuid) -> Vec<NetEvent> {
    vec![NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: DEMO_NOVA,
        from_name: "Tess Touch".into(),
        to_agent_id: agent,
        dialog: 0,
        session_id: Uuid::nil(),
        message: aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset").into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    })]
}

/// Instant messages, a conference invitation, a local chat line and two
/// nearby avatars, all with names longer than their lists.
fn long_name_events(agent: Uuid) -> Vec<NetEvent> {
    let im = |from: Uuid, message: &str| {
        let (first, last, _) = long_name(&from).unwrap_or_default();
        NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: from,
            from_name: format!("{first} {last}"),
            to_agent_id: agent,
            dialog: 0,
            session_id: Uuid::nil(),
            message: message.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        })
    };
    vec![
        im(DEMO_LONG_NAME, aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset")),
        im(DEMO_LONG_NAME, aurora_i18n::tr!("demo-yes-beautiful-at-sunset")),
        im(DEMO_WIDE_NAME, aurora_i18n::tr!("demo-arriving-with-textures")),
        NetEvent::SessionInvite(aurora_net::SessionInvite {
            session: DEMO_LONG_SESSION,
            from: DEMO_TESS,
            from_name: "Tess Touch".into(),
            message: aurora_i18n::tr!("demo-has-anyone-seen-the-new-eep-sky").into(),
            session_name: LONG_SESSION_NAME.into(),
            offline: false,
            timestamp: 0,
        }),
        NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Maximilienne Wolfeschlegelsteinhausen".into(),
            source_id: DEMO_LONG_NAME,
            owner_id: Uuid::nil(),
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: aurora_net::ChatType::Normal,
            position: Vec3::ZERO,
            message: aurora_i18n::tr!("demo-hi-everyone-we-re-building-the-plaza-tonight").into(),
        }),
        NetEvent::CoarseLocations {
            handle: HANDLE,
            positions: vec![
                (DEMO_LONG_NAME, Vec3::new(146.0, 132.0, 25.0)),
                (DEMO_WIDE_NAME, Vec3::new(120.0, 150.0, 25.0)),
            ],
        },
    ]
}

/// AURORA_DEMO_RADAR=tp: the region we are supposed to have teleported
/// from, forty regions east, whose simulator still tells us who is there.
pub const RADAR_LEFT_REGION: u64 = ((256000u64 + 40 * 256) << 32) | 256000;

/// AURORA_DEMO_RADAR: residents around us for the radar of the People
/// window, at every kind of range (chat, shout, farther, unknown altitude);
/// with `left_behind`, three more in `RADAR_LEFT_REGION`.
pub fn radar_events(left_behind: bool) -> Vec<NetEvent> {
    let mut events = vec![NetEvent::CoarseLocations {
        handle: HANDLE,
        positions: vec![
            (DEMO_LOUP, Vec3::new(134.0, 132.0, 24.0)),
            (DEMO_NOVA, Vec3::new(150.0, 140.0, 24.0)),
            (DEMO_TESS, Vec3::new(96.0, 150.0, 28.0)),
            (DEMO_LONG_NAME, Vec3::new(146.0, 132.0, 24.0)),
            (DEMO_WIDE_NAME, Vec3::new(120.0, 150.0, 24.0)),
            (u(100), Vec3::new(20.0, 240.0, 60.0)),
            // no altitude: its range is unknown
            (u(101), Vec3::new(200.0, 60.0, 0.0)),
        ],
    }];
    if left_behind {
        events.push(NetEvent::CoarseLocations {
            handle: RADAR_LEFT_REGION,
            positions: vec![
                (u(102), Vec3::new(120.0, 130.0, 24.0)),
                (u(103), Vec3::new(60.0, 200.0, 32.0)),
                (u(104), Vec3::new(210.0, 40.0, 24.0)),
            ],
        });
    }
    events
}

/// AgentProfile reply of the demo residents (AURORA_DEMO_PROFILE).
fn demo_profile(id: Uuid) -> aurora_net::AvatarProfile {
    use aurora_net::profile::{days_from_civil, flags};
    let date = |y, m, d| Some((days_from_civil(y, m, d) * 86_400) as f64);
    let groups = vec![
        aurora_net::ProfileGroup {
            id: DEMO_GROUP1,
            name: "Aurora Builders".into(),
            insignia: Uuid::nil(),
        },
        aurora_net::ProfileGroup {
            id: DEMO_GROUP2,
            name: aurora_i18n::tr!("demo-northern-wolves").into(),
            insignia: Uuid::nil(),
        },
    ];
    let mut p = aurora_net::AvatarProfile {
        id,
        sl_about: aurora_i18n::tr!("demo-resident-in-aurora-viewer-s-offline-demo").into(),
        born: date(2019, 4, 2),
        hide_age: Some(false),
        online: Some(true),
        flags: flags::IDENTIFIED,
        notes: Some(String::new()),
        sl_image: tags::profile_picture(id),
        ..Default::default()
    };
    if id == DEMO_AGENT {
        p.sl_about = aurora_i18n::tr!("demo-my-demo-profile-it-can-be-edited-here-saving").into();
        p.fl_about = aurora_i18n::tr!("demo-aurora-viewer-developer").into();
        p.born = date(2012, 3, 14);
        p.flags = flags::TRANSACTED | flags::ALLOW_PUBLISH;
        p.customer_type = aurora_i18n::tr!("demo-premium").into();
        p.groups = groups;
        p.picks = vec![(Uuid::from_u128(0x91C0_0001), aurora_i18n::tr!("demo-aurora-beach").into())];
    } else if id == DEMO_LOUP {
        p.sl_about = aurora_i18n::tr!(
            "demo-lone-wolf-weekend-builder-my-partner-secondlife-app-agent",
            DEMO_NOVA = format!("{}", DEMO_NOVA)
        );
        p.fl_about = aurora_i18n::tr!("demo-mountain-enthusiast").into();
        p.partner = DEMO_NOVA;
        p.born = date(2007, 5, 12);
        p.flags = flags::TRANSACTED;
        p.customer_type = "Lifetime".into();
        p.groups = groups;
        p.picks = vec![
            (Uuid::from_u128(0x91C0_0001), aurora_i18n::tr!("demo-aurora-beach").into()),
            (Uuid::from_u128(0x91C0_0002), aurora_i18n::tr!("demo-my-workshop").into()),
        ];
        p.notes = Some(aurora_i18n::tr!("demo-met-at-the-beach-very-friendly").into());
    } else if id == DEMO_TESS {
        // three days old, no payment info: the radar's age alert
        let today = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        p.born = Some(today - 3.5 * 86_400.0);
        p.flags = 0;
    } else if id == DEMO_NOVA {
        p.partner = DEMO_LOUP;
        p.hide_age = Some(true);
        p.online = None;
        p.groups = groups.into_iter().take(1).collect();
    }
    p
}

/// AURORA_DEMO_CONTACTS: two contact sets (one with a non-friend), an alias
/// and a removed display name, written to the demo account's file once.
pub fn seed_contact_sets(world: &mut crate::world::World) {
    if !world.contact_sets.set_names().is_empty() {
        return;
    }
    let friends: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let is_friend = |id: &Uuid| friends.contains(id);
    let cs = &mut world.contact_sets;
    cs.add_set(aurora_i18n::tr!("demo-family"));
    cs.set_color(aurora_i18n::tr!("demo-family"), [0.72, 0.52, 1.0, 1.0]);
    cs.add_to_set(&[u(100), u(102), DEMO_LOUP], aurora_i18n::tr!("demo-family"), is_friend);
    cs.add_set(aurora_i18n::tr!("demo-work"));
    cs.set_color(aurora_i18n::tr!("demo-work"), [0.35, 0.85, 0.8, 1.0]);
    cs.add_to_set(&[u(101), u(102)], aurora_i18n::tr!("demo-work"), is_friend);
    cs.set_pseudonym(&[u(102)], aurora_i18n::tr!("demo-orion-the-great"), is_friend);
    cs.remove_display_name(&[u(103)], is_friend);
}

/// Item of the notification center's sample inventory offer.
pub const DEMO_OFFER: Uuid = Uuid::from_u128(0x0FFE_0000_0000_0000_0000_0000_0000_0001);

/// What fetching `DEMO_OFFER` by id returns: the offered object, filed by
/// the "server" in `parent`.
pub fn offered_item(parent: Uuid, owner: Uuid) -> aurora_net::inventory::InvItem {
    aurora_net::inventory::InvItem {
        id: DEMO_OFFER,
        parent,
        name: aurora_i18n::tr!("demo-aurora-lantern").into(),
        asset_type: 6,
        inv_type: 6,
        creator: Uuid::from_u128(2),
        owner,
        base_mask: 0x7fffffff,
        owner_mask: 0x7fffffff,
        ..Default::default()
    }
}

pub fn demo_reply(cmd: &aurora_net::NetCommand) -> Vec<NetEvent> {
    if let Some(events) = hud::reply(cmd) {
        return events;
    }
    if let Some(events) = search::reply(cmd) {
        return events;
    }
    use aurora_net::inventory::{FolderContents, InvItem};
    match cmd {
        // a lure accepted offline: the teleport starts, then has nowhere to go
        aurora_net::NetCommand::AcceptLure { .. } => vec![
            NetEvent::TeleportStarted,
            NetEvent::TeleportFailed {
                reason: aurora_i18n::tr!("demo-no-teleport-offline").into(),
            },
        ],
        aurora_net::NetCommand::AgentAnimation { anim, start: false } if *anim == SEAT_ANIM => {
            vec![NetEvent::AvatarAnimations {
                avatar: DEMO_AGENT,
                anims: vec![(IDLE_ANIM, 1)],
                sources: Vec::new(),
            }]
        }
        aurora_net::NetCommand::Land(l) => land::reply(l),
        aurora_net::NetCommand::RequestNames(ids) => {
            let names = [
                aurora_i18n::tr!("demo-nova-example"),
                "Pixel Boutique",
                aurora_i18n::tr!("demo-orion-example"),
                "Tess Touch",
            ];
            vec![NetEvent::Names(
                ids.iter()
                    .map(|id| {
                        let n = (id.as_u128() & 0xFF) as usize % names.len();
                        let mut it = names[n].split(' ');
                        (*id, it.next().unwrap_or("").to_owned(), it.next().unwrap_or("").to_owned())
                    })
                    .collect(),
            )]
        }
        // People API names: half of them with a chosen display name
        aurora_net::NetCommand::RequestDisplayNames(ids) => {
            let names = [
                ("Nova", aurora_i18n::tr!("demo-example"), "Nova ♪"),
                ("pixelboutique", "Resident", ""),
                ("Orion", aurora_i18n::tr!("demo-example"), ""),
                ("tesstouch", "Resident", "Tess ✨"),
            ];
            vec![NetEvent::DisplayNames {
                names: ids
                    .iter()
                    .map(|id| {
                        let glyphs = glyphs::resident(id);
                        let (first, last, dn) = if *id == DEMO_AGENT {
                            ("Aurora", "Demo", "")
                        } else if let Some(long) = long_name(id) {
                            long
                        } else if let Some(resident) = tags::name(id) {
                            resident
                        } else if let Some((first, sample)) = &glyphs {
                            (first.as_str(), "Resident", sample.display_name)
                        } else {
                            names[(id.as_u128() & 0xFF) as usize % names.len()]
                        };
                        let username = if last == "Resident" {
                            first.to_lowercase()
                        } else {
                            format!("{first}.{last}").to_lowercase()
                        };
                        aurora_net::AvatarNameData {
                            id: *id,
                            display_name: if dn.is_empty() {
                                if last == "Resident" {
                                    first.into()
                                } else {
                                    format!("{first} {last}")
                                }
                            } else {
                                dn.into()
                            },
                            username,
                            legacy_first: first.into(),
                            legacy_last: last.into(),
                            is_default: dn.is_empty(),
                            next_update: 0.0,
                        }
                    })
                    .collect(),
                bad_ids: Vec::new(),
                max_age: None,
            }]
        }
        // display name change, never sent for real in demo mode: accepted
        // (locked for a week), or refused with AURORA_DEMO_DISPLAYNAME_ERROR
        // as error_tag
        aurora_net::NetCommand::SetDisplayName { old, new } => {
            if let Ok(tag) = std::env::var("AURORA_DEMO_DISPLAYNAME_ERROR") {
                let mut content = Llsd::new_map();
                content.insert("error_tag", tag);
                return vec![NetEvent::SetDisplayNameReply {
                    status: 400,
                    reason: "Bad Request".into(),
                    content,
                }];
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let reset = new.is_empty();
            let name = aurora_net::AvatarNameData {
                id: DEMO_AGENT,
                username: "aurora.demo".into(),
                display_name: if reset { "Aurora Demo".into() } else { new.clone() },
                legacy_first: "Aurora".into(),
                legacy_last: "Demo".into(),
                is_default: reset,
                next_update: now + 7.0 * 86400.0,
            };
            let mut content = Llsd::new_map();
            content.insert("display_name", name.display_name.clone());
            vec![
                NetEvent::SetDisplayNameReply {
                    status: 200,
                    reason: "OK".into(),
                    content,
                },
                NetEvent::DisplayNameUpdate {
                    name,
                    old_display_name: old.clone(),
                },
            ]
        }
        aurora_net::NetCommand::FetchInventory { folders, owner, .. } => {
            let mut out = Vec::new();
            for f in folders {
                if let Some(c) = env::folder_contents(*f, *owner).or_else(|| place::folder_contents(*f, *owner)) {
                    out.push(c);
                    continue;
                }
                let items: Vec<InvItem> = (0..5u128)
                    .map(|i| InvItem {
                        id: Uuid::from_u128(f.as_u128() ^ (0xABC0 + i)),
                        parent: *f,
                        name: aurora_i18n::tr!("demo-demo-object", value0 = format!("{}", i + 1)),
                        desc: String::new(),
                        asset_type: [6, 3, 7, 0, 10][i as usize],
                        inv_type: [6, 3, 7, 0, 10][i as usize],
                        asset_id: Uuid::nil(),
                        flags: 0,
                        favorite: false,
                        creator: Uuid::nil(),
                        created_at: 0,
                        owner: *owner,
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
                let subfolders = if *f == u(1) {
                    aurora_net::inventory::parse_skeleton(&demo_raw()["inventory-skeleton"])
                        .into_iter()
                        .filter(|x| x.parent == u(1))
                        .collect()
                } else {
                    Vec::new()
                };
                out.push(FolderContents {
                    folder_id: *f,
                    owner_id: *owner,
                    version: 2,
                    folders: subfolders,
                    items: if *f == u(1) { Vec::new() } else { items },
                });
            }
            vec![NetEvent::InventoryContents(out)]
        }
        // block list: Violet Wolf, an object, a name and the chat of group 2
        aurora_net::NetCommand::RequestMuteList { .. } => {
            let text = aurora_i18n::tr!(
                "demo-1-violet-wolf-0-2-spam-box-0-0",
                value0 = format!("{}", DEMO_LOUP),
                value1 = format!("{}", Uuid::from_u128(0xB10C_0000_0000_0000_0000_0000_0000_0001)),
                value2 = format!("{}", Uuid::nil()),
                value3 = format!("{}", Uuid::nil()),
                value4 = format!("{}", DEMO_GROUP2)
            );
            // The animesh fixture needs its wearer visible in full.
            let text = if std::env::var_os("AURORA_DEMO_ANIMESH").is_some() {
                text.lines().skip(1).collect::<Vec<_>>().join("\n")
            } else {
                text
            };
            let text = text + &tags::mute_line().unwrap_or_default();
            vec![NetEvent::MuteList(aurora_net::MuteListSource::File(text))]
        }
        // two groups, and someone starts a chat in the first one
        aurora_net::NetCommand::RequestGroups => {
            // GP_SESSION_JOIN: the group chat can be opened from Contacts
            let g = |id: Uuid, name: &str, notices: bool| aurora_net::GroupMembership {
                id,
                name: name.into(),
                insignia: Uuid::nil(),
                powers: 1 << 16,
                accept_notices: notices,
                list_in_profile: true,
                contribution: 0,
            };
            vec![
                NetEvent::Groups(vec![
                    g(DEMO_GROUP1, "Aurora Builders", true),
                    g(DEMO_GROUP2, aurora_i18n::tr!("demo-northern-wolves"), false),
                    aurora_net::GroupMembership {
                        list_in_profile: false,
                        ..g(DEMO_GROUP3, aurora_i18n::tr!("common-plaza-market"), true)
                    },
                ]),
                NetEvent::ActiveGroup {
                    id: DEMO_GROUP1,
                    name: "Aurora Builders".into(),
                    title: aurora_i18n::tr!("demo-builder").into(),
                },
                NetEvent::SessionInvite(aurora_net::SessionInvite {
                    session: DEMO_GROUP1,
                    from: DEMO_NOVA,
                    from_name: aurora_i18n::tr!("demo-nova-example").into(),
                    message: aurora_i18n::tr!("demo-hi-everyone-we-re-building-the-plaza-tonight").into(),
                    session_name: "Aurora Builders".into(),
                    offline: false,
                    timestamp: 0,
                }),
            ]
        }
        aurora_net::NetCommand::ActivateGroup(id) => vec![NetEvent::ActiveGroup {
            id: *id,
            name: String::new(),
            title: String::new(),
        }],
        aurora_net::NetCommand::LeaveGroup(id) => vec![NetEvent::GroupDropped(*id)],
        aurora_net::NetCommand::ChatSession { method, session } => {
            let agents = |ids: &[Uuid]| {
                ids.iter()
                    .map(|a| aurora_net::SessionAgent {
                        agent: *a,
                        present: Some(true),
                        moderator: Some(*a == DEMO_NOVA),
                        text_muted: None,
                    })
                    .collect::<Vec<_>>()
            };
            let me = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
            match method {
                aurora_net::SessionMethod::AcceptInvitation => {
                    let mut ev = vec![NetEvent::ChatSessionReply {
                        method: *method,
                        session: *session,
                        result: Some(aurora_llsd::Llsd::Undef),
                    }];
                    ev.push(NetEvent::SessionAgents {
                        session: *session,
                        agents: agents(&[me, DEMO_NOVA, DEMO_TESS, DEMO_LOUP]),
                    });
                    // the blocked member's message must not show
                    ev.push(NetEvent::InstantMessage(aurora_net::InstantMessage {
                        from_agent_id: DEMO_LOUP,
                        from_name: aurora_i18n::tr!("demo-violet-wolf").into(),
                        to_agent_id: me,
                        dialog: 17,
                        session_id: *session,
                        message: aurora_i18n::tr!("demo-message-from-a-blocked-resident").into(),
                        offline: false,
                        from_group: false,
                        binary_bucket: b"Aurora Builders\0".to_vec(),
                        timestamp: 0,
                    }));
                    ev.push(NetEvent::InstantMessage(aurora_net::InstantMessage {
                        from_agent_id: DEMO_TESS,
                        from_name: "Tess Touch".into(),
                        to_agent_id: me,
                        dialog: 17,
                        session_id: *session,
                        message: aurora_i18n::tr!("demo-arriving-with-textures").into(),
                        offline: false,
                        from_group: false,
                        binary_bucket: b"Aurora Builders\0".to_vec(),
                        timestamp: 0,
                    }));
                    ev
                }
                aurora_net::SessionMethod::FetchHistory => {
                    let line = |from: Uuid, name: &str, text: &str, ago: u64| {
                        let t = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                            .saturating_sub(ago);
                        aurora_llsd::llsd_map! { "from_id" => from, "from" => name, "message" => text, "time" => t as f64 }
                    };
                    let history = aurora_llsd::Llsd::Array(vec![
                        line(
                            DEMO_TESS,
                            "Tess Touch",
                            aurora_i18n::tr!("demo-has-anyone-seen-the-new-eep-sky"),
                            3600,
                        ),
                        line(
                            DEMO_LOUP,
                            aurora_i18n::tr!("demo-violet-wolf"),
                            aurora_i18n::tr!("demo-history-from-a-blocked-resident"),
                            3000,
                        ),
                        line(
                            DEMO_NOVA,
                            aurora_i18n::tr!("demo-nova-example"),
                            aurora_i18n::tr!("demo-yes-beautiful-at-sunset"),
                            2400,
                        ),
                    ]);
                    vec![NetEvent::ChatSessionReply {
                        method: *method,
                        session: *session,
                        result: Some(history),
                    }]
                }
                aurora_net::SessionMethod::DeclineInvitation => Vec::new(),
            }
        }
        // the request made once after login: what the grid kept while we
        // were away (AURORA_DEMO_CHATLOG=offline), else nothing
        aurora_net::NetCommand::RequestOfflineMessages => match chatlog::mode() {
            Some(chatlog::Mode::Offline) => chatlog::offline_messages(DEMO_AGENT),
            _ => Vec::new(),
        },
        // opening a group chat ourselves
        aurora_net::NetCommand::SendImDialog { dialog: 15, id, .. } => vec![NetEvent::SessionStarted {
            temp_session: *id,
            session: *id,
            success: true,
            error: String::new(),
            agents: vec![aurora_net::SessionAgent {
                agent: DEMO_TESS,
                present: Some(true),
                moderator: None,
                text_muted: None,
            }],
        }],
        // local chat echoes back like the simulator does
        aurora_net::NetCommand::Chat { message, chat_type, .. } => vec![NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Aurora Demo".into(),
            source_id: Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001),
            owner_id: Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001),
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: *chat_type,
            position: Vec3::new(132.0, 126.0, 30.0),
            message: message.clone(),
        })],
        aurora_net::NetCommand::RequestProfile(id) => vec![NetEvent::AvatarProfile(Box::new(demo_profile(*id)))],
        aurora_net::NetCommand::PickInfoRequest { creator, pick } => {
            let names = [aurora_i18n::tr!("demo-aurora-beach"), aurora_i18n::tr!("demo-my-workshop")];
            let n = ((pick.as_u128() & 0xF) as usize + 1) % names.len();
            vec![NetEvent::PickInfo(Box::new(aurora_net::PickInfo {
                id: *pick,
                creator: *creator,
                parcel: Uuid::from_u128(0x9A2C_E100 + n as u128),
                name: names[n].into(),
                desc: aurora_i18n::tr!("demo-a-quiet-spot-by-the-water-perfect-for-watching").into(),
                sim_name: aurora_i18n::tr!("common-aurora-demo").into(),
                pos_global: glam::DVec3::new(256_000.0 + 140.0, 256_000.0 + 120.0, 25.0),
                enabled: true,
                ..Default::default()
            }))]
        }
        aurora_net::NetCommand::RemoteParcelRequest { handle, position, .. } => place::remote_parcel(*handle, *position),
        aurora_net::NetCommand::RegionHandleRequest(id) => place::region_handle(*id),
        aurora_net::NetCommand::ParcelInfoRequest(id) => match place::parcel_info(*id) {
            Some(ev) => vec![ev],
            // the parcels of the profile picks
            None => vec![NetEvent::ParcelInfo(Box::new(aurora_net::ParcelSummary {
                id: *id,
                name: aurora_i18n::tr!("common-aurora-plaza").into(),
                sim_name: aurora_i18n::tr!("common-aurora-demo").into(),
                ..Default::default()
            }))],
        },
        aurora_net::NetCommand::ClassifiedsRequest(id) if *id == DEMO_LOUP => vec![NetEvent::AvatarClassifieds {
            target: *id,
            list: vec![(
                Uuid::from_u128(0xC1A5_0001),
                aurora_i18n::tr!("demo-wolf-s-workshop-mesh-furniture").into(),
            )],
        }],
        aurora_net::NetCommand::ClassifiedInfoRequest(id) => vec![NetEvent::ClassifiedInfo(Box::new(aurora_net::ClassifiedInfo {
            id: *id,
            creator: DEMO_LOUP,
            creation_date: 1_759_000_000,
            category: 2,
            name: aurora_i18n::tr!("demo-wolf-s-workshop-mesh-furniture").into(),
            desc: aurora_i18n::tr!("demo-lightweight-mesh-furniture-copyable-and-modifiable-come-visit-the").into(),
            sim_name: aurora_i18n::tr!("common-aurora-demo").into(),
            parcel_name: aurora_i18n::tr!("common-aurora-plaza").into(),
            pos_global: glam::DVec3::new(256_000.0 + 128.0, 256_000.0 + 128.0, 30.0),
            flags: aurora_net::profile::classified_flags::AUTO_RENEW,
            price: 50,
            ..Default::default()
        }))],
        aurora_net::NetCommand::MapBlockRequest {
            min_x,
            min_y,
            max_x,
            max_y,
            null_sims,
        } => {
            let mut blocks: Vec<aurora_net::MapBlock> = demo_sims()
                .iter()
                .filter(|s| (*min_x..=*max_x).contains(&s.0) && (*min_y..=*max_y).contains(&s.1))
                .map(|s| map_block(s.0, s.1, s.2, s.3))
                .collect();
            if *null_sims && blocks.is_empty() {
                blocks.push(map_block(*min_x, *min_y, "", 255));
            }
            vec![NetEvent::MapBlocks {
                blocks,
                null_sims: *null_sims,
            }]
        }
        aurora_net::NetCommand::MapNameRequest { name } => {
            let n = name.trim_end_matches('#').to_lowercase();
            let mut blocks: Vec<aurora_net::MapBlock> = demo_sims()
                .iter()
                .filter(|s| s.2.to_lowercase().starts_with(&n))
                .map(|s| map_block(s.0, s.1, s.2, s.3))
                .collect();
            blocks.push(map_block(0, 0, "", 0));
            vec![NetEvent::MapBlocks { blocks, null_sims: false }]
        }
        aurora_net::NetCommand::MapItemRequest { item_type, handle } => {
            use aurora_net::map_item;
            let item = |x: u32, y: u32, extra: i32, extra2: i32, name: &str| aurora_net::MapItem {
                x: 256000 + x,
                y: 256000 + y,
                id: Uuid::from_u128(0xD0_0000 + (x as u128) * 1000 + y as u128),
                extra,
                extra2,
                name: name.into(),
            };
            let items = match *item_type {
                map_item::AGENT_LOCATIONS if *handle == HANDLE => vec![item(132, 126, 2, 0, ""), item(60, 200, 1, 0, "")],
                map_item::AGENT_LOCATIONS if *handle == (256256u64 << 32) | 256000 => vec![item(300, 80, 3, 0, "")],
                map_item::TELEHUB => vec![
                    item(128, 40, 0, 1, aurora_i18n::tr!("demo-plaza-infohub")),
                    item(400, 128, 0, 0, aurora_i18n::tr!("demo-lagoon-telehub")),
                ],
                map_item::LAND_FOR_SALE => vec![item(220, 30, 1024, 2500, aurora_i18n::tr!("demo-waterfront-parcel"))],
                map_item::PG_EVENT => vec![item(90, 300, 1_791_475_200, 25, aurora_i18n::tr!("demo-northern-lights-party"))],
                _ => Vec::new(),
            };
            vec![NetEvent::MapItems {
                item_type: *item_type,
                items,
            }]
        }
        // AGENT_CONTROL_SIT_ON_GROUND / STAND_UP: the simulator swaps our
        // animations (sit_ground_constrained while sitting)
        aurora_net::NetCommand::OneShotControl(f) if f & (aurora_net::control::SIT_ON_GROUND | aurora_net::control::STAND_UP) != 0 => {
            let anim = if f & aurora_net::control::STAND_UP != 0 {
                IDLE_ANIM
            } else {
                crate::world::body::ANIM_SIT_GROUND_CONSTRAINED
            };
            vec![NetEvent::AvatarAnimations {
                sources: Vec::new(),
                avatar: DEMO_AGENT,
                anims: vec![(anim, 1)],
            }]
        }
        aurora_net::NetCommand::RequestObjectProperties { object, .. } if (970..=982).any(|id| action_id(id) == *object) => {
            let mode = std::env::var("AURORA_DEMO_ACTIONS").unwrap_or_default();
            vec![NetEvent::ObjectProperties(vec![aurora_net::build::ObjectProps {
                object_id: *object,
                owner_id: DEMO_NOVA,
                sale_type: if *object == action_id(971) {
                    match mode.as_str() {
                        "buy-original" => 1,
                        "buy-contents" | "buy-empty" => 3,
                        _ => 2,
                    }
                } else {
                    0
                },
                next_owner_mask: aurora_net::build::perm::TRANSFER,
                sale_price: 10,
                name: if *object == action_id(971) {
                    aurora_i18n::tr!("demo-cube-for-sale")
                } else {
                    aurora_i18n::tr!("common-demo-object")
                }
                .into(),
                description: aurora_i18n::tr!("demo-offline-demo-no-real-l-is-spent").into(),
                ..Default::default()
            }])]
        }
        aurora_net::NetCommand::RequestPayPrice { object, .. } if *object == action_id(972) => {
            let (default, buttons) = match std::env::var("AURORA_DEMO_ACTIONS").unwrap_or_default().as_str() {
                "pay-layout" => (1956, vec![1956, 3913, 5870, 7826]),
                "pay-hidden" => (-1, vec![1956, -1, 5870, -1]),
                "pay-large" => (i32::MAX, vec![i32::MAX, 3913, 5870, 7826]),
                _ => (10, vec![1, 5, 10, 20]),
            };
            vec![NetEvent::PayPrice {
                object: *object,
                default,
                buttons,
            }]
        }
        aurora_net::NetCommand::RequestTaskInventory { object, .. } if *object == action_id(971) => {
            use aurora_net::build::{TaskItem, perm};
            let empty = std::env::var("AURORA_DEMO_ACTIONS").is_ok_and(|m| m == "buy-empty");
            let items = [
                (7, aurora_i18n::tr!("demo-game-rules-french"), perm::COPY | perm::TRANSFER),
                (6, aurora_i18n::tr!("demo-card-game"), perm::TRANSFER),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (ty, name, next))| TaskItem {
                item_id: action_id(995 + index as u32),
                parent_id: *object,
                owner_id: DEMO_NOVA,
                asset_type: ty,
                inv_type: ty,
                name: name.into(),
                owner_mask: if empty { 0 } else { perm::COPY | perm::TRANSFER },
                next_owner_mask: next,
                ..Default::default()
            })
            .collect();
            vec![NetEvent::TaskInventory {
                object: *object,
                serial: None,
                result: Ok(items),
            }]
        }
        aurora_net::NetCommand::RequestTaskInventory { object, .. } if *object == action_id(976) => {
            let contents = [
                (7, aurora_i18n::tr!("demo-welcome-card")),
                (10, aurora_i18n::tr!("common-demo-script")),
                (6, aurora_i18n::tr!("demo-spare-cube")),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (ty, name))| {
                let owner_mask = 0x7fff_ffff_u32
                    & match i {
                        1 => !aurora_net::build::perm::MODIFY,
                        2 => !aurora_net::build::perm::COPY,
                        _ => u32::MAX,
                    };
                aurora_llsd::llsd_map!("item_id" => action_id(990+i as u32),"parent_id" => *object,
                    "permissions" => aurora_llsd::llsd_map!(
                        "owner_id" => DEMO_AGENT,
                        "base_mask" => 0x7fff_ffff_u32,
                        "owner_mask" => owner_mask,
                        "next_owner_mask" => aurora_net::build::perm::TRANSFER),
                    "type" => ty,"inv_type" => ty,"name" => name,"desc" => aurora_i18n::tr!("demo-contents-simulated-offline"))
            })
            .collect();
            let reply = aurora_llsd::llsd_map!("contents" => Llsd::Array(contents));
            vec![NetEvent::TaskInventory {
                object: *object,
                serial: None,
                result: aurora_net::task_inventory::parse_cap(&reply),
            }]
        }
        aurora_net::NetCommand::ObjectGrabUpdate { object, position, .. } if *object == action_id(982) => {
            let mut o = action_object(982, 0, 138.0, 126.0, [0.8, 0.6, 0.3, 1.0], aurora_i18n::tr!("demo-grab-move"));
            o.position = *position;
            vec![NetEvent::ObjectUpdates {
                handle: HANDLE,
                objects: vec![o],
            }]
        }
        aurora_net::NetCommand::RequestSit { target, .. } if *target == action_id(970) || *target == action_id(986) => {
            let mut avatar = action_avatar(true);
            avatar.parent_id = if *target == action_id(986) { 986 } else { 970 };
            vec![
                NetEvent::SitResponse {
                    object: *target,
                    camera_eye: Vec3::ZERO,
                    camera_at: Vec3::ZERO,
                    force_mouselook: false,
                },
                NetEvent::ObjectUpdates {
                    handle: HANDLE,
                    objects: vec![avatar],
                },
            ]
        }
        _ => Vec::new(),
    }
}

pub fn action_id(id: u32) -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | id as u128)
}

pub fn action_avatar(seated: bool) -> ObjectUpdate {
    let mut o = prim(
        9000,
        if seated {
            Vec3::new(0.0, 0.0, 1.2)
        } else {
            Vec3::new(132.0, 126.0, floor_at(132.0, 126.0) + 0.84)
        },
        Quat::from_rotation_z(0.3),
        Vec3::new(0.45, 0.6, 1.9),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.91, 0.93, 0.98, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    o.full_id = DEMO_AGENT;
    o.pcode = LL_PCODE_LEGACY_AVATAR;
    o.parent_id = if seated { 970 } else { 0 };
    o.name_values = "FirstName STRING RW SV Aurora\nLastName STRING RW SV Demo".into();
    o
}

pub fn action_mode_id(mode: &str) -> u32 {
    match mode {
        "linked-touch" => return 985,
        "linked-sit" => return 986,
        _ => {}
    }
    if mode.starts_with("transparent-") || mode.starts_with("invisible-") {
        return 976;
    }
    if mode.starts_with("blend-") || mode.starts_with("mask-") {
        return 976;
    }
    if mode.starts_with("open-media") {
        return 978;
    }
    match mode.split('-').next().unwrap_or("") {
        "sit" => 970,
        "buy" => 971,
        "pay" => 972,
        "none" | "touch" => 975,
        "open" => 976,
        "play" | "pause" => 977,
        "zoom" => 979,
        "disabled" => 980,
        "ignore" => 981,
        "grab" => 982,
        _ => 0,
    }
}

fn action_object(id: u32, action: u8, x: f32, y: f32, color: [f32; 4], label: &str) -> ObjectUpdate {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut o = prim(
        id,
        Vec3::new(x, y, floor_at(x, y) + 0.6),
        Quat::IDENTITY,
        Vec3::splat(1.2),
        boxp,
        te(color, 0, false, 0.0),
        ExtraParams::default(),
        label,
    );
    o.click_action = action;
    o.owner_id = if id == 976 { DEMO_AGENT } else { DEMO_NOVA };
    o.update_flags = match id {
        972 => 1 << 9,
        975 | 980 | 981 => 1 << 7,
        976 => (1 << 5) | (1 << 2),
        982 => 1 | (1 << 8) | (1 << 5),
        _ => 0,
    };
    if id == 978
        && let Some(t) = o.texture_entry.as_mut()
    {
        for f in Arc::make_mut(t).faces.iter_mut() {
            f.texture = ACTION_MEDIA_TEX;
        }
    }
    o
}

/// A visible screen placed in front of Buy: Ignore passes through; Disabled
/// occludes and consumes the click. Geometry remains selectable in build mode.
pub fn action_overlay(id: u32, pos: Vec3, rotation: Quat) -> NetEvent {
    let mut o = action_object(
        id,
        if id == 981 { 9 } else { 8 },
        138.0,
        126.0,
        [0.75, 0.3, 0.35, 1.0],
        if id == 981 {
            aurora_i18n::tr!("demo-ignore-click-through")
        } else {
            aurora_i18n::tr!("demo-disabled-no-click")
        },
    );
    o.position = pos;
    o.rotation = rotation;
    o.scale = Vec3::new(0.12, 2.0, 2.0);
    NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![o],
    }
}

/// A non-actionable face placed before Buy. Blend blocks every texel;
/// Mask's zero-alpha texture lets the click reach Buy.
pub fn alpha_click_overlay(mode: &str, pos: Vec3, rotation: Quat) -> NetEvent {
    let texture = match mode.trim_end_matches("-ready") {
        "blend-faint" => TEX_PICK_FAINT,
        "blend-half" => TEX_PICK_HALF,
        "blend-solid" => BLANK_TEXTURE,
        _ => TEX_PICK_ZERO,
    };
    let material = if mode.starts_with("mask-") { MAT_MASK } else { MAT_BLEND };
    let mut object = action_object(976, 0, 138.0, 126.0, [0.7, 0.55, 0.8, 1.0], "");
    object.position = pos;
    object.rotation = rotation;
    object.scale = Vec3::new(0.12, 2.0, 2.0);
    if let Some(te) = object.texture_entry.as_mut() {
        for face in &mut Arc::make_mut(te).faces {
            face.texture = texture;
            face.material_id = material;
        }
    }
    NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object],
    }
}

/// All click actions, plus an inherited Buy on a child. Named modes isolate
/// a target and App::demo_action_steps exercises the real picking / input.
pub fn action_events() -> Vec<NetEvent> {
    let mode = std::env::var("AURORA_DEMO_ACTIONS").unwrap_or_default();
    if mode.starts_with("blend-") || mode.starts_with("mask-") {
        let behind = action_object(971, 2, 138.0, 126.0, [0.35, 0.65, 0.8, 1.0], "");
        return vec![
            NetEvent::ObjectUpdates {
                handle: HANDLE,
                objects: vec![behind],
            },
            NetEvent::Balance(250),
        ];
    }
    if mode.starts_with("transparent-") || mode.starts_with("invisible-") {
        let alpha = if mode.starts_with("invisible-") { 0.0 } else { 0.4 };
        let o = action_object(976, 0, 138.0, 126.0, [0.7, 0.55, 0.8, alpha], "");
        return vec![NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![o],
        }];
    }
    if mode.starts_with("linked-") {
        // The root's hollow geometry encloses a larger child's seat: its
        // smaller bounding box must not steal hits on the seat inside it.
        let mut table = action_object(985, 0, 138.0, 126.0, [0.7, 0.55, 0.8, 1.0], aurora_i18n::tr!("demo-root-touch"));
        table.scale = Vec3::new(2.4, 2.4, 1.4);
        table.position.z = floor_at(138.0, 126.0) + 0.7;
        table.volume = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 35000, 0);
        table.update_flags = 1 << 7;
        let mut chair = action_object(
            986,
            1,
            138.0,
            126.0,
            [0.3, 0.75, 0.55, 1.0],
            aurora_i18n::tr!("demo-linked-child-sit"),
        );
        chair.parent_id = table.local_id;
        chair.position = Vec3::ZERO;
        chair.scale = Vec3::new(3.0, 3.0, 1.2);
        return vec![NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![table, chair],
        }];
    }
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut objects = Vec::new();
    for (id, action, x, y, color, label) in [
        (970, 1, 138.0, 122.0, [0.48, 0.3, 0.75, 1.0], aurora_i18n::tr!("demo-sit-sit-down")),
        (
            971,
            2,
            138.0,
            126.0,
            [0.35, 0.65, 0.8, 1.0],
            aurora_i18n::tr!("demo-buy-copy-for-l-10"),
        ),
        (
            972,
            3,
            135.0,
            130.0,
            [0.3, 0.75, 0.55, 1.0],
            aurora_i18n::tr!("demo-pay-pay-object"),
        ),
        (
            975,
            0,
            139.0,
            132.0,
            [0.7, 0.55, 0.8, 1.0],
            aurora_i18n::tr!("status-none-touch-touch"),
        ),
        (976, 4, 138.0, 134.0, [0.8, 0.7, 0.4, 1.0], aurora_i18n::tr!("demo-open-contents")),
        (
            977,
            5,
            142.0,
            122.0,
            [0.3, 0.65, 0.7, 1.0],
            aurora_i18n::tr!("extra-play-play-pause"),
        ),
        (
            978,
            6,
            142.0,
            126.0,
            [0.35, 0.65, 0.8, 1.0],
            aurora_i18n::tr!("demo-open-media-parcel-media"),
        ),
        (
            979,
            7,
            142.0,
            130.0,
            [0.6, 0.65, 0.85, 1.0],
            aurora_i18n::tr!("demo-zoom-frame-object"),
        ),
        (
            980,
            8,
            142.0,
            134.0,
            [0.75, 0.3, 0.35, 1.0],
            aurora_i18n::tr!("demo-disabled-no-click"),
        ),
        (
            981,
            9,
            136.5,
            125.8,
            [0.75, 0.3, 0.35, 1.0],
            aurora_i18n::tr!("demo-ignore-click-through"),
        ),
        (982, 0, 139.0, 138.0, [0.8, 0.6, 0.3, 1.0], aurora_i18n::tr!("demo-grab-move")),
    ] {
        objects.push(action_object(id, action, x, y, color, label));
    }
    let mut child = prim(
        974,
        Vec3::new(0.0, 0.0, 1.0),
        Quat::IDENTITY,
        Vec3::splat(0.4),
        boxp,
        te([0.7, 0.7, 0.9, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-inherited-buy"),
    );
    child.parent_id = 971;
    objects.push(child);
    let selected = action_mode_id(&mode);
    if selected != 0 {
        objects.retain(|o| {
            o.local_id == selected || ((selected == 971 || selected == 980 || selected == 981) && (o.local_id == 971 || o.local_id == 974))
        });
        if selected >= 975
            && selected != 980
            && selected != 981
            && let Some(o) = objects.iter_mut().find(|o| o.local_id == selected)
        {
            o.position = Vec3::new(138.0, 126.0, floor_at(138.0, 126.0) + 0.6);
        }
    }
    let balance = if matches!(mode.as_str(), "pay-layout" | "pay-hidden" | "pay-large") {
        i32::MAX
    } else {
        250
    };
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }, NetEvent::Balance(balance)]
}

/// Nested surfaces for cursor-depth selection: two linksets and one single
/// prim all share a center, so their surfaces overlap from every camera angle.
pub fn selection_cycle_events() -> Vec<NetEvent> {
    let mut objects = Vec::new();
    for (id, scale, parent, name) in [
        (99610, 3.0, 0, "A"),
        (99611, 2.4, 99610, "A2"),
        (99612, 1.8, 0, "B"),
        (99613, 1.2, 99612, "B2"),
        (99614, 0.6, 0, "C"),
    ] {
        let label = if parent == 0 {
            aurora_i18n::tr!("demo-cycle-object", name = name)
        } else {
            aurora_i18n::tr!("demo-cycle-part", name = name)
        };
        let mut o = action_object(id, 0, 138.0, 126.0, [0.3, scale / 4.0, 0.8, 1.0], &label);
        o.scale = Vec3::splat(scale);
        o.position.z = floor_at(138.0, 126.0) + 2.0;
        o.parent_id = parent;
        if parent != 0 {
            o.position = Vec3::ZERO;
        }
        objects.push(o);
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// Three separate surfaces for selecting faces: a two-prim linkset and
/// another object, all modifiable. Optional invisible geometry tests overlays
/// whose original face has no material record until it is selected.
pub fn face_selection_events(invisible: bool) -> Vec<NetEvent> {
    let mut objects = Vec::new();
    for (id, y, parent, name) in [(99620, 102.0, 0, "A"), (99621, 105.0, 99620, "A2"), (99622, 99.0, 0, "B")] {
        let label = aurora_i18n::tr!("demo-cycle-object", name = name);
        let mut object = action_object(id, 0, 150.0, y, [0.65, 0.75, 0.85, if invisible { 0.0 } else { 1.0 }], &label);
        object.scale = Vec3::splat(2.0);
        object.position.z = floor_at(150.0, 102.0) + 1.5;
        object.parent_id = parent;
        if parent != 0 {
            object.position = Vec3::new(0.0, 3.0, 0.0);
        }
        object.owner_id = DEMO_AGENT;
        object.update_flags =
            crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY | crate::ui::context::flags::OBJECT_COPY;
        objects.push(object);
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

pub const FACE_SELECTION_MESH: Uuid = Uuid::from_u128(0xa0e0_fa00_0000_0000_0000_0000_0000_0001);

/// Closed curved face, three disconnected islands of one mesh TE, and a
/// hollow cut cylinder, all in the ordinary build selection event path.
pub fn complex_face_selection_events() -> Vec<NetEvent> {
    let mut events = face_selection_events(false);
    for event in &mut events {
        if let NetEvent::ObjectUpdates { objects, .. } = event {
            for object in objects {
                object.volume = match object.local_id {
                    99620 => crate::build::shapes::shapes()[10].params(),
                    99621 => aurora_prim::VolumeParams {
                        sculpt: Some(SculptParams {
                            texture: FACE_SELECTION_MESH,
                            sculpt_type: LL_SCULPT_TYPE_MESH,
                        }),
                        ..Default::default()
                    },
                    _ => shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 30000, 8000),
                };
                if object.local_id == 99620 {
                    object.volume.path.scale = [0.45, 0.20];
                }
                object.rotation = if object.local_id == 99621 {
                    object.position = Vec3::new(3.0, 0.0, 0.0);
                    Quat::IDENTITY
                } else {
                    Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
                };
            }
        }
    }
    events
}

/// AURORA_DEMO_BAN: the south-east lot (x >= 192 m, y < 64 m) is banned.
pub fn ban_collision() -> NetEvent {
    let mut bitmap = vec![0u8; 64 * 64 / 8];
    for j in 0..16usize {
        for i in 48..64usize {
            let k = i + j * 64;
            bitmap[k / 8] |= 1 << (k % 8);
        }
    }
    NetEvent::ParcelCollision {
        handle: HANDLE,
        kind: 1,
        use_pass: false,
        bitmap,
    }
}

/// Offline world map: (grid x, grid y, name, access).
fn demo_sims() -> &'static Vec<(u16, u16, &'static str, u8)> {
    static TABLE: aurora_i18n::Localized<Vec<(u16, u16, &'static str, u8)>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            (1000, 1000, aurora_i18n::tr!("common-aurora-demo"), 13),
            (1001, 1000, aurora_i18n::tr!("common-boreal-lagoon"), 21),
            (999, 1000, aurora_i18n::tr!("common-pine-grove"), 13),
            (1000, 1001, "Nordheim", 42),
            (1001, 1001, aurora_i18n::tr!("common-flaw"), 254),
            (999, 999, aurora_i18n::tr!("common-sandbox"), 13),
        ]
    })
}

fn map_block(x: u16, y: u16, name: &str, access: u8) -> aurora_net::MapBlock {
    aurora_net::MapBlock {
        x,
        y,
        name: name.into(),
        access,
        region_flags: 0,
        water_height: 20,
        agents: 0,
        map_image_id: Uuid::nil(),
        size_x: 256,
        size_y: 256,
    }
}

pub fn events() -> Vec<NetEvent> {
    let agent = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
    let login = LoginResponse {
        agent_id: agent,
        session_id: Uuid::nil(),
        secure_session_id: Uuid::nil(),
        first_name: "Aurora".into(),
        last_name: "Demo".into(),
        sim_ip: std::net::Ipv4Addr::LOCALHOST,
        sim_port: 0,
        circuit_code: 0,
        region_x: 256000,
        region_y: 256000,
        region_size_x: 256,
        region_size_y: 256,
        seed_capability: String::new(),
        look_at: Vec3::new(1.0, 0.3, 0.0),
        message: aurora_i18n::tr!("demo-offline-demo-mode-no-network-connection").into(),
        agent_appearance_service: String::new(),
        inventory_root: Uuid::nil(),
        mfa_hash: None,
        raw: demo_raw(),
    };
    let mut ev = vec![
        NetEvent::LoggedIn(Arc::new(login)),
        NetEvent::MainRegionChanged { handle: HANDLE },
        // only the People API (names) is simulated
        NetEvent::Capabilities {
            handle: HANDLE,
            caps: Arc::new([("GetDisplayNames".to_owned(), "demo://names".to_owned())].into_iter().collect()),
        },
        NetEvent::RegionHandshake(Arc::new(RegionInfo {
            handle: HANDLE,
            name: aurora_i18n::tr!("common-aurora-demo").into(),
            region_id: Uuid::nil(),
            water_height: 20.0,
            sim_access: 13,
            region_flags: aurora_net::region_flags::ALLOW_VOICE,
            terrain_base: [Uuid::nil(); 4],
            terrain_detail: [Uuid::nil(); 4],
            terrain_start_height: [20.0, 20.0, 20.0, 20.0],
            terrain_height_range: [30.0, 30.0, 30.0, 30.0],
            size_x: 256,
            size_y: 256,
            is_main: true,
            owner: DEMO_LOUP,
            is_estate_manager: false,
            product_name: "Estate / Full Region".into(),
        })),
    ];
    // parcel overlay (mini-map property lines): four parcels split at
    // x = y = 128 m and a for-sale lot in the south-east corner
    let mut cells = vec![1u8; 64 * 64];
    for j in 0..64 {
        for i in 0..64 {
            let c = &mut cells[j * 64 + i];
            if i >= 48 && j < 16 {
                *c = 4;
            }
            if i == 32 || (i == 48 && j < 16) {
                *c |= 0x40;
            }
            if j == 32 || (j == 16 && i >= 48) {
                *c |= 0x80;
            }
        }
    }
    for q in 0..4 {
        ev.push(NetEvent::ParcelOverlay {
            handle: HANDLE,
            sequence: q as i32,
            data: cells[q * 1024..(q + 1) * 1024].to_vec(),
        });
    }
    // terrain
    let mut patches = Vec::new();
    for py in 0..16u32 {
        for px in 0..16u32 {
            let mut h = Vec::with_capacity(256);
            for j in 0..16 {
                for i in 0..16 {
                    h.push(height((px * 16 + i) as f32, (py * 16 + j) as f32));
                }
            }
            patches.push(TerrainPatch {
                x: px,
                y: py,
                size: 16,
                heights: h,
            });
        }
    }
    ev.push(NetEvent::Terrain { handle: HANDLE, patches });

    // prims
    let g = |x: f32, y: f32| height(x, y);
    let violet = [0.545, 0.361, 0.965, 1.0];
    let indigo = [0.31, 0.275, 0.898, 1.0];
    let teal = [0.369, 0.918, 0.831, 1.0];
    let ink = [0.91, 0.93, 0.98, 1.0];
    let navy = [0.12, 0.13, 0.22, 1.0];
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let cyl = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0);
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let torus = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, 175, 0, 0);
    let tube = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 30000, 0);
    let cutbox = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 20000, 12500);
    let mut objs = Vec::new();
    let mut id = 100;
    let mut add = |o: ObjectUpdate| {
        objs.push(o);
    };
    // plaza
    let (cx, cy) = (140.0, 130.0);
    add(prim(
        id,
        Vec3::new(cx, cy, g(cx, cy) + 0.1),
        Quat::IDENTITY,
        Vec3::new(24.0, 24.0, 0.5),
        cyl,
        te(navy, 2, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    id += 1;
    for k in 0..8 {
        let a = k as f32 / 8.0 * std::f32::consts::TAU;
        let (x, y) = (cx + a.cos() * 10.0, cy + a.sin() * 10.0);
        let c = if k % 2 == 0 { violet } else { indigo };
        add(prim(
            id,
            Vec3::new(x, y, g(cx, cy) + 3.0),
            Quat::IDENTITY,
            Vec3::new(0.6, 0.6, 5.5),
            cyl,
            te(c, 1, false, 0.0),
            ExtraParams::default(),
            "",
        ));
        id += 1;
        add(prim(
            id,
            Vec3::new(x, y, g(cx, cy) + 6.0),
            Quat::IDENTITY,
            Vec3::new(0.9, 0.9, 0.9),
            sphere,
            te(teal, 0, true, 0.6),
            ExtraParams {
                light: Some(LightParams {
                    color: [0.37, 0.92, 0.83, 1.0],
                    radius: 9.0,
                    cutoff: 0.0,
                    falloff: 0.75,
                }),
                ..Default::default()
            },
            "",
        ));
        id += 1;
    }
    add(prim(
        id,
        Vec3::new(cx, cy, g(cx, cy) + 4.0),
        Quat::from_rotation_x(0.4),
        Vec3::new(5.0, 5.0, 5.0),
        torus,
        te(violet, 3, false, 0.0),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-aurora-viewer-rust-vulkan"),
    ));
    id += 1;
    add(prim(
        id,
        Vec3::new(cx + 18.0, cy - 6.0, g(cx + 18.0, cy - 6.0) + 2.0),
        Quat::from_rotation_z(0.6),
        Vec3::new(4.0, 4.0, 4.0),
        cutbox,
        te(ink, 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    id += 1;
    add(prim(
        id,
        Vec3::new(cx - 16.0, cy + 8.0, g(cx - 16.0, cy + 8.0) + 3.0),
        Quat::IDENTITY,
        Vec3::new(3.0, 3.0, 6.0),
        tube,
        te(indigo, 2, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    id += 1;
    // alpha modes / legacy materials test panels (procedural textures)
    // Keep the manual click-action targets unobstructed. Reserve the same
    // ids so other demo objects and replies retain their identities.
    // AURORA_DEMO_TEXANIM puts its own panels there.
    let texanim = std::env::var_os("AURORA_DEMO_TEXANIM").is_some();
    if !std::env::var("AURORA_DEMO_ACTIONS").is_ok_and(|m| m == "1") && !texanim {
        let yaw = 0.4f32.atan2(0.8);
        let across = Vec3::new(-yaw.sin(), yaw.cos(), 0.0);
        let center = Vec3::new(138.5, 125.5, 0.0);
        let floor = height(cx, cy) + 0.35;
        for (k, (tex, mat, label)) in alpha_panels().iter().enumerate() {
            let p = center + across * ((k as f32 - 2.5) * 2.1);
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = *tex;
                f.color = [1.0, 1.0, 1.0, 1.0];
                f.material_id = mat.unwrap_or(Uuid::nil());
            }
            add(prim(
                id,
                Vec3::new(p.x, p.y, floor + 1.25),
                Quat::from_rotation_z(yaw),
                Vec3::new(0.06, 1.8, 1.8),
                boxp,
                Arc::new(t),
                ExtraParams::default(),
                label,
            ));
            id += 1;
        }
    } else {
        id += alpha_panels().len() as u32;
    }
    // AURORA_DEMO_GLOW_LAYERS=1: a faintly glowing layer over a faintly
    // glowing skin, as mesh bodies wear them (fixed ids: the other demo
    // objects keep theirs). Left panel: skin with glow 0.02; right panel:
    // skin without glow. Over the lower half of both, 1 cm in front, a pink
    // layer (alpha 0 to 1 from left to right) with glow 0.02.
    if let Ok(mode) = std::env::var("AURORA_DEMO_GLOW_LAYERS") {
        let shell = mode == "shell";
        let floor = height(cx, cy) + 0.35;
        let skin = [0.93, 0.84, 0.80, 1.0];
        for (k, skin_glow) in [0.02f32, 0.0].into_iter().enumerate() {
            let y = 110.9 + k as f32 * 2.2;
            add(prim(
                9701 + k as u32 * 2,
                Vec3::new(138.0, y, floor + 1.25),
                Quat::IDENTITY,
                Vec3::new(0.06, 1.8, 1.8),
                boxp,
                te(skin, 0, false, skin_glow),
                ExtraParams::default(),
                "",
            ));
            let mut layer = TextureEntry::default();
            // only the face turned to the west shows (the others are
            // invisible); "shell": all of them, the far side glows too
            for (i, f) in layer.faces.iter_mut().enumerate() {
                let shown = shell || i == 4;
                f.texture = TEX_GRADIENT;
                f.color = [1.0, 0.0, 0.5, if shown { 1.0 } else { 0.0 }];
                f.glow = if shown { 0.02 } else { 0.0 };
            }
            add(prim(
                9702 + k as u32 * 2,
                Vec3::new(137.955, y, floor + 0.8),
                Quat::IDENTITY,
                Vec3::new(0.01, 1.8, 0.9),
                boxp,
                Arc::new(layer),
                ExtraParams::default(),
                "",
            ));
        }
    }
    // AURORA_DEMO_PROJECTOR=1|point: a projector (a light with a light
    // texture) 4.5 m from a wall, turned to it, another one above the
    // ground turned down, with ambiance, a pool light under a slab and an
    // ordinary point light (fixed ids). The first lights the wall through
    // its image; the ground stays dark outside the frustums. `point`: the
    // projectors without their texture, as plain point lights, to compare.
    if let Ok(mode) = std::env::var("AURORA_DEMO_PROJECTOR") {
        let floor = height(cx, cy) + 0.35;
        let wall = [0.82, 0.82, 0.82, 1.0];
        add(prim(
            9711,
            Vec3::new(139.0, 114.5, floor + 2.0),
            Quat::IDENTITY,
            Vec3::new(0.2, 6.0, 4.0),
            boxp,
            te(wall, 0, false, 0.0),
            ExtraParams::default(),
            "",
        ));
        let image = (mode != "point").then_some(aurora_prim::extra::LightImageParams {
            texture: TEX_PROJECTED,
            params: Vec3::new(1.0, 0.0, 0.0),
        });
        // -Z of the prim to the east
        add(prim(
            9712,
            Vec3::new(134.5, 114.5, floor + 2.0),
            Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
            Vec3::new(0.3, 0.3, 0.1),
            boxp,
            te([1.0, 1.0, 1.0, 1.0], 0, true, 0.0),
            ExtraParams {
                light: Some(LightParams {
                    color: [1.0, 1.0, 1.0, 1.0],
                    radius: 10.0,
                    cutoff: 0.0,
                    falloff: 0.5,
                }),
                light_image: image,
                ..Default::default()
            },
            "",
        ));
        // above the ground, turned down, blurred from 1 m on, with ambiance
        add(prim(
            9713,
            Vec3::new(133.0, 114.6, floor + 2.5),
            Quat::IDENTITY,
            Vec3::new(0.3, 0.3, 0.1),
            boxp,
            te([1.0, 1.0, 1.0, 1.0], 0, true, 0.0),
            ExtraParams {
                light: Some(LightParams {
                    color: [0.5, 1.0, 1.0, 1.0],
                    radius: 6.0,
                    cutoff: 0.0,
                    falloff: 0.0,
                }),
                // a plain white image, as a projector left with the blank
                // texture: a single level here, and still soft edges
                light_image: image.map(|_| aurora_prim::extra::LightImageParams {
                    texture: BLANK_TEXTURE,
                    params: Vec3::new(1.4, 1.0, 0.3),
                }),
                ..Default::default()
            },
            "",
        ));
        // a pool light: a very wide projector turned down, 2 m under a
        // slab with a normal map and 4 m beside its middle. It lights
        // nothing above itself; as a point light, it lights the sides of
        // the slab's relief that lean towards it.
        let mut slab = TextureEntry::default();
        for f in slab.faces.iter_mut() {
            f.color = [0.85, 0.82, 0.8, 1.0];
            f.material_id = MAT_BUMPY;
        }
        let ground = g(136.4, 111.0);
        add(prim(
            9714,
            Vec3::new(136.4, 111.0, ground + 0.12),
            Quat::IDENTITY,
            Vec3::new(3.4, 3.4, 0.2),
            boxp,
            Arc::new(slab),
            ExtraParams::default(),
            "",
        ));
        add(prim(
            9715,
            Vec3::new(132.4, 110.6, ground - 2.0),
            Quat::IDENTITY,
            Vec3::new(0.3, 0.39, 0.18),
            boxp,
            te([1.0, 1.0, 1.0, 1.0], 0, true, 0.0),
            ExtraParams {
                light: Some(LightParams {
                    color: [0.5, 1.0, 1.0, 0.7],
                    radius: 20.0,
                    cutoff: 0.0,
                    falloff: 0.0,
                }),
                light_image: image.map(|i| aurora_prim::extra::LightImageParams {
                    params: Vec3::new(3.0, 20.0, 0.0),
                    ..i
                }),
                ..Default::default()
            },
            "",
        ));
        // a post in the first projector's beam: its shadow falls on the wall
        add(prim(
            9717,
            Vec3::new(137.2, 114.2, floor + 2.0),
            Quat::IDENTITY,
            Vec3::new(0.3, 0.3, 1.6),
            boxp,
            te([0.7, 0.7, 0.75, 1.0], 0, false, 0.0),
            ExtraParams::default(),
            "",
        ));
        // an ordinary point light beside them, the same in both modes
        add(prim(
            9716,
            Vec3::new(137.8, 117.2, floor + 1.0),
            Quat::IDENTITY,
            Vec3::splat(0.2),
            boxp,
            te([1.0, 0.7, 0.3, 1.0], 0, true, 0.0),
            ExtraParams {
                light: Some(LightParams {
                    color: [1.0, 0.7, 0.3, 1.0],
                    radius: 3.0,
                    cutoff: 0.0,
                    falloff: 0.5,
                }),
                ..Default::default()
            },
            "",
        ));
    }
    // a little city of boxes
    for i in 0..6 {
        for j in 0..4 {
            let x = 60.0 + i as f32 * 9.0;
            let y = 170.0 + j as f32 * 10.0;
            let hgt = 4.0 + ((i * 7 + j * 3) % 5) as f32 * 3.0;
            let c = [0.55 + 0.07 * (i % 3) as f32, 0.55, 0.62 + 0.05 * (j % 2) as f32, 1.0];
            add(prim(
                id,
                Vec3::new(x, y, g(x, y) + hgt * 0.5 - 0.3),
                Quat::IDENTITY,
                Vec3::new(6.0, 7.0, hgt),
                boxp,
                te(c, (i % 2) as u8, false, 0.0),
                ExtraParams::default(),
                "",
            ));
            id += 1;
        }
    }
    // AURORA_DEMO_OCCLUSION=1: occlusion test, a long wall in front of the
    // start position with 360 detailed objects behind it (a few towers rise
    // above it, so some must stay visible)
    if std::env::var_os("AURORA_DEMO_OCCLUSION").is_some() {
        let wx = 158.0;
        for k in 0..4 {
            let y = 100.0 + k as f32 * 20.0;
            add(prim(
                id,
                Vec3::new(wx, y, g(wx, y) + 12.0),
                Quat::IDENTITY,
                Vec3::new(1.0, 20.0, 30.0),
                boxp,
                te(ink, 0, false, 0.0),
                ExtraParams::default(),
                "",
            ));
            id += 1;
        }
        for i in 0..18 {
            for j in 0..20 {
                let x = 164.0 + i as f32 * 3.4;
                let y = 92.0 + j as f32 * 3.6;
                let tower = i % 6 == 5 && j % 5 == 2;
                let (vol, scale, z) = if tower {
                    (cyl, Vec3::new(1.5, 1.5, 40.0), g(x, y) + 20.0)
                } else if (i + j) % 2 == 0 {
                    (torus, Vec3::splat(2.6), g(x, y) + 2.0)
                } else {
                    (sphere, Vec3::splat(2.4), g(x, y) + 1.5)
                };
                let c = [0.4 + 0.03 * (i % 8) as f32, 0.45, 0.5 + 0.02 * (j % 10) as f32, 1.0];
                add(prim(
                    id,
                    Vec3::new(x, y, z),
                    Quat::from_rotation_z(i as f32 * 0.3),
                    scale,
                    vol,
                    te(c, (j % 3) as u8, false, 0.0),
                    ExtraParams::default(),
                    "",
                ));
                id += 1;
            }
        }
    }
    // AURORA_DEMO_OVERDRAW=<n>: overdraw test, n rows of opaque walls (12
    // if the value is not a number) one behind another east of the plaza,
    // each taller than the one in front, sent from the farthest to the
    // nearest. Every wall shows above the next one, so the occlusion
    // culling keeps them all, and a scene pass that starts from an empty
    // depth shades each of them in full before the nearer ones cover it
    if let Ok(v) = std::env::var("AURORA_DEMO_OVERDRAW") {
        let n = v.trim().parse::<u32>().unwrap_or(12).clamp(1, 40);
        let base = g(160.0, 128.0);
        for k in (0..n).rev() {
            let h = 4.0 + 1.5 * k as f32;
            let c = [0.35 + 0.02 * (k % 8) as f32, 0.4, 0.55 - 0.02 * (k % 6) as f32, 1.0];
            add(prim(
                id,
                Vec3::new(160.0 + 2.0 * k as f32, 128.0, base + h * 0.5),
                Quat::IDENTITY,
                Vec3::new(0.5, 64.0, h),
                boxp,
                te(c, (k % 4) as u8, false, 0.0),
                ExtraParams::default(),
                "",
            ));
            id += 1;
        }
    }
    // AURORA_DEMO_PLANAR=1: planar texgen test, floor slabs of different
    // sizes ahead of the start position. The planar ones (two tile repeats
    // per metre, centers 0.5 m apart) must line up across slabs; the strip at
    // the back uses default mapping (one repeat stretched over 5 m); the cube
    // checks the side faces.
    if std::env::var_os("AURORA_DEMO_PLANAR").is_some() {
        let floor = height(cx, cy) + 0.35;
        let slabs: [(Vec3, Vec3, bool); 5] = [
            (Vec3::new(135.0, 127.0, 0.0), Vec3::new(2.0, 2.0, 0.1), true),
            (Vec3::new(137.5, 127.0, 0.0), Vec3::new(3.0, 2.0, 0.1), true),
            (Vec3::new(136.0, 124.5, 0.0), Vec3::new(4.0, 3.0, 0.1), true),
            (Vec3::new(136.5, 129.5, 0.0), Vec3::new(5.0, 3.0, 0.1), false),
            (Vec3::new(138.0, 124.5, 0.5), Vec3::new(1.0, 1.0, 1.0), true),
        ];
        for (p, scale, planar) in slabs {
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = TEX_TILES;
                f.color = [1.0, 1.0, 1.0, 1.0];
                // TEM_TEX_GEN_PLANAR (LLTextureEntry), under TEM_TEX_GEN_MASK
                if planar {
                    f.media_flags |= 0x02;
                }
            }
            add(prim(
                id,
                Vec3::new(p.x, p.y, floor + p.z + scale.z * 0.5),
                Quat::IDENTITY,
                scale,
                boxp,
                Arc::new(t),
                ExtraParams::default(),
                "",
            ));
            id += 1;
        }
    }
    // AURORA_DEMO_PBR_OVERRIDE=1: two slabs with the same PBR material (one
    // tile over the whole face); the second one gets a GLTF material override
    // (4 × 4 repeats on every map, a tint), sent as the simulator's notation
    // payload so that the network parser is exercised too
    let mut overrides = Vec::new();
    if std::env::var_os("AURORA_DEMO_PBR_OVERRIDE").is_some() {
        let floor = height(cx, cy) + 0.35;
        for (k, (x, y)) in [(137.5f32, 129.0f32), (135.0, 127.5)].into_iter().enumerate() {
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = BLANK_TEXTURE;
                f.color = [1.0, 1.0, 1.0, 1.0];
            }
            let mut o = prim(
                id,
                Vec3::new(x, y, floor + 0.17),
                Quat::IDENTITY,
                Vec3::new(2.0, 2.0, 0.1),
                boxp,
                Arc::new(t),
                ExtraParams {
                    render_materials: (0..6).map(|f| (f, MAT_PBR_TILES)).collect(),
                    ..Default::default()
                },
                "",
            );
            if k == 1 {
                let ti = "{'s':[r4,r4]}";
                let payload = format!("{{'id':i{id},'te':[i0],'od':[{{'bc':[r1,r0.8,r0.7,r1],'ti':[{ti},{ti},{ti},{ti}]}}]}}");
                if let Some((local_id, sides)) = aurora_net::objects::parse_gltf_override(payload.as_bytes()) {
                    overrides.push(NetEvent::GltfOverrides {
                        handle: HANDLE,
                        local_id,
                        sides,
                    });
                }
                o.text = aurora_i18n::tr!("demo-override-4-4-tint").into();
            }
            add(o);
            id += 1;
        }
    }
    // AURORA_DEMO_TEXANIM=1: texture animations (llSetTextureAnim) on a row
    // of panels facing the start position, in place of the alpha panels
    if texanim {
        use aurora_net::objects::TextureAnim as Ta;
        let anim = |mode: u8, face: i8, size: (u8, u8), start: f32, length: f32, rate: f32| Ta {
            mode: Ta::ON | mode,
            face,
            size_x: size.0,
            size_y: size.1,
            start,
            length,
            rate,
        };
        let smooth = Ta::SMOOTH | Ta::LOOP;
        // (texture, legacy material, PBR, animation, hover text); the scaled
        // one is alpha masked (prepass and shadow alpha tests animate too)
        let panels: [(Uuid, Uuid, bool, Ta, &str); 8] = [
            (
                TEX_TILES,
                Uuid::nil(),
                false,
                anim(smooth, -1, (1, 1), 0.0, 1.0, 0.25),
                aurora_i18n::tr!("demo-scrolling-smooth"),
            ),
            (
                TEX_FRAMES,
                Uuid::nil(),
                false,
                anim(Ta::LOOP, -1, (4, 4), 0.0, 0.0, 4.0),
                aurora_i18n::tr!("demo-4-4-grid-loop"),
            ),
            (
                TEX_FRAMES,
                Uuid::nil(),
                false,
                anim(Ta::LOOP | Ta::PING_PONG, -1, (4, 4), 0.0, 4.0, 2.0),
                aurora_i18n::tr!("extra-back-and-forth-ping-pong"),
            ),
            (
                TEX_TILES,
                Uuid::nil(),
                false,
                anim(smooth | Ta::ROTATE, -1, (1, 1), 0.0, std::f32::consts::TAU, 1.0),
                aurora_i18n::tr!("demo-rotation-rotate"),
            ),
            (
                TEX_HOLES,
                Uuid::nil(),
                false,
                anim(smooth | Ta::PING_PONG | Ta::SCALE, -1, (1, 1), 1.0, 2.0, 0.5),
                aurora_i18n::tr!("demo-scale-scale-alpha-mask"),
            ),
            (
                TEX_FRAMES,
                Uuid::nil(),
                false,
                anim(Ta::LOOP, 3, (4, 4), 0.0, 0.0, 4.0),
                aurora_i18n::tr!("demo-one-face-face-3"),
            ),
            (
                TEX_TILES,
                MAT_BUMPY,
                false,
                anim(smooth, -1, (1, 1), 0.0, 1.0, -0.25),
                aurora_i18n::tr!("demo-material-normals-follow"),
            ),
            (BLANK_TEXTURE, Uuid::nil(), true, anim(smooth, -1, (1, 1), 0.0, 1.0, 0.25), "PBR"),
        ];
        // two rows of four, right to left as seen from the start position
        let yaw = 0.4f32.atan2(0.8);
        let across = Vec3::new(-yaw.sin(), yaw.cos(), 0.0);
        let center = Vec3::new(138.5, 125.5, 0.0) + across * 3.3;
        let floor = height(cx, cy) + 0.35;
        for (k, (tex, mat, pbr, ta, label)) in panels.into_iter().enumerate() {
            let p = center + across * (((k % 4) as f32 - 1.5) * 1.75);
            let z = floor + if k < 4 { 2.9 } else { 0.95 };
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = tex;
                f.color = [1.0, 1.0, 1.0, 1.0];
                f.material_id = mat;
            }
            // the single-face one is a cube turned to show two sides (faces 3
            // and 4: +Y and -X)
            let (rot, scale) = if ta.face >= 0 {
                (Quat::from_rotation_z(yaw + std::f32::consts::FRAC_PI_4), Vec3::splat(1.1))
            } else {
                (Quat::from_rotation_z(yaw), Vec3::new(0.06, 1.5, 1.5))
            };
            let extra = if pbr {
                ExtraParams {
                    render_materials: (0..6).map(|f| (f, MAT_PBR_TILES)).collect(),
                    ..Default::default()
                }
            } else {
                ExtraParams::default()
            };
            let mut o = prim(id, Vec3::new(p.x, p.y, z), rot, scale, boxp, Arc::new(t), extra, label);
            o.texture_anim = Some(ta);
            add(o);
            id += 1;
        }
    }
    // Scripted wind / smoke in front of a dark panel for a visible comparison.
    let particles_mode = std::env::var("AURORA_DEMO_PARTICLES").unwrap_or_default();
    if !particles_mode.is_empty() && particles_mode != "load" {
        let mode = particles_mode.as_str();
        let ground = floor_at(136.0, 127.0);
        add(prim(
            id,
            Vec3::new(139.0, 125.0, ground + 3.0),
            Quat::IDENTITY,
            Vec3::new(0.1, 24.0, 6.0),
            boxp,
            te([0.01, 0.01, 0.015, 1.0], 0, true, 0.0),
            ExtraParams::default(),
            "",
        ));
        id += 1;
        for (wind, y, label) in [
            (true, 126.0, aurora_i18n::tr!("common-wind")),
            (false, 130.0, aurora_i18n::tr!("demo-smoke")),
        ] {
            if (mode == "wind" && !wind) || (mode == "smoke" && wind) || (mode == "legacy" && !wind) {
                continue;
            }
            let mut o = prim(
                id,
                Vec3::new(137.0, y, ground + 2.5),
                Quat::IDENTITY,
                Vec3::splat(0.2),
                boxp,
                te(teal, 0, true, 0.0),
                ExtraParams::default(),
                label,
            );
            if let Some(particles) = compressed_scripted_particles(wind, mode == "legacy") {
                o.particles = particles;
            }
            add(o);
            id += 1;
        }
    }
    // particle fountain at the plaza center (legacy 86-byte particle block)
    {
        let mut o = prim(
            id,
            Vec3::new(cx - 5.0, cy - 5.0, g(cx, cy) + 0.6),
            Quat::IDENTITY,
            Vec3::new(0.6, 0.6, 0.4),
            cyl,
            te(teal, 0, true, 0.3),
            ExtraParams::default(),
            "",
        );
        o.particles = aurora_net::objects::ParticleUpdate::Set(fountain_particles());
        add(o);
        id += 1;
    }
    // AURORA_DEMO_PARTICLES=load: a ring of 32 more fountains around the
    // plaza, about 4 000 particles alive (the particle steps of the profile)
    if particles_mode == "load" {
        for i in 0..32 {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            let (x, y) = (cx + 14.0 * a.cos(), cy + 14.0 * a.sin());
            let mut o = prim(
                id,
                Vec3::new(x, y, g(x, y) + 0.6),
                Quat::IDENTITY,
                Vec3::new(0.6, 0.6, 0.4),
                cyl,
                te(teal, 0, true, 0.3),
                ExtraParams::default(),
                "",
            );
            o.particles = aurora_net::objects::ParticleUpdate::Set(fountain_particles());
            add(o);
            id += 1;
        }
    }
    // a mirror (box reflection probe flagged as mirror) with a polished face
    {
        let probe = aurora_prim::extra::ReflectionProbeParams {
            ambiance: 0.0,
            clip_distance: 0.0,
            flags: 0x1 | 0x4,
        };
        let rot = Quat::from_rotation_z(-1.2) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        add(prim(
            id,
            Vec3::new(cx + 2.0, cy - 9.0, g(cx, cy) + 2.0),
            rot,
            Vec3::new(3.0, 2.6, 0.08),
            boxp,
            te([0.85, 0.87, 0.9, 1.0], 3, false, 0.0),
            ExtraParams {
                reflection_probe: Some(probe),
                ..Default::default()
            },
            "",
        ));
        id += 1;
    }
    // AURORA_DEMO_PROBES=1: two manual reflection probes on the plaza, as
    // invisible prims: a box turned on itself, which the crowd's camera
    // (AURORA_DEMO_POS="135,134") stands in, and a sphere east of it
    if std::env::var_os("AURORA_DEMO_PROBES").is_some() {
        for (offset, rot, scale, flags, ambiance) in [
            (
                Vec3::new(-2.0, 4.0, 3.0),
                Quat::from_rotation_z(0.4),
                Vec3::new(10.0, 8.0, 6.0),
                0x1,
                1.0,
            ),
            (Vec3::new(6.0, 2.0, 2.0), Quat::IDENTITY, Vec3::splat(12.0), 0x0, 0.5),
        ] {
            let (x, y) = (cx + offset.x, cy + offset.y);
            add(prim(
                id,
                Vec3::new(x, y, g(x, y) + offset.z),
                rot,
                scale,
                boxp,
                te([1.0, 1.0, 1.0, 0.0], 0, false, 0.0),
                ExtraParams {
                    reflection_probe: Some(aurora_prim::extra::ReflectionProbeParams {
                        ambiance,
                        clip_distance: 0.0,
                        flags,
                    }),
                    ..Default::default()
                },
                "",
            ));
            id += 1;
        }
    }
    // glass panel (alpha blend)
    add(prim(
        id,
        Vec3::new(cx + 6.0, cy + 14.0, g(cx + 6.0, cy + 14.0) + 2.0),
        Quat::IDENTITY,
        Vec3::new(6.0, 0.1, 3.5),
        boxp,
        te([0.6, 0.8, 1.0, 0.35], 3, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    id += 1;
    let _ = id;
    if let Ok(spec) = std::env::var("AURORA_DEMO_FLEXIBLE") {
        objs.clear();
        overrides.clear();
        let edit = spec == "edit";
        let count = if edit {
            1
        } else {
            spec.parse::<usize>().unwrap_or(6).clamp(6, 2000)
        };
        for k in 0..count {
            let mut volume = boxp;
            volume.path.curve_type = LL_PCODE_PATH_FLEXIBLE;
            if k % 3 == 1 {
                volume.profile.curve_type = LL_PCODE_PROFILE_CIRCLE;
            }
            if k % 3 == 2 {
                volume.profile.hollow = 0.45;
                volume.path.twist_end = 0.5;
                volume.path.scale = [0.4, 0.7];
            }
            let flex = aurora_prim::extra::FlexibleParams {
                softness: if edit { 2 } else { (k % 4) as u8 },
                tension: if k % 6 == 1 { 8.0 } else { 1.0 },
                air_friction: 2.0,
                gravity: if k % 6 == 2 { -3.0 } else { 3.0 },
                wind_sensitivity: if k % 6 == 3 { 10.0 } else { 0.0 },
                user_force: if k % 6 == 4 { Vec3::Y * 6.0 } else { Vec3::ZERO },
            };
            let x = 137.0 + (k % 6) as f32 * 1.6;
            let y = 132.0 + (k / 6) as f32 * 1.6;
            let z = height(x, y) + if edit { 5.0 } else { 3.0 };
            let mut o = prim(
                90000 + k as u32,
                Vec3::new(x, y, z),
                Quat::from_rotation_y(if edit {
                    std::f32::consts::FRAC_PI_4
                } else {
                    std::f32::consts::FRAC_PI_2
                }),
                if edit {
                    Vec3::new(1.0, 0.3, 4.0)
                } else {
                    Vec3::new(0.25, 0.4, 4.0)
                },
                volume,
                te([0.25 + 0.1 * (k % 6) as f32, 0.75, 0.6, 1.0], 0, false, 0.0),
                ExtraParams {
                    flexible: Some(flex),
                    ..Default::default()
                },
                "",
            );
            if spec == "rigid" {
                o.extra.flexible = None;
            }
            if k % 6 == 5 {
                // A moving linkset: child follows the rotating rigid root,
                // while its free end keeps inertia in world space.
                let root_id = 95000 + k as u32;
                let mut root = prim(
                    root_id,
                    o.position,
                    Quat::IDENTITY,
                    Vec3::splat(0.35),
                    boxp,
                    te([0.8, 0.3, 0.2, 1.0], 0, false, 0.0),
                    ExtraParams::default(),
                    "",
                );
                root.angular_velocity = Vec3::Z * 0.8;
                objs.push(root);
                o.parent_id = root_id;
                o.position = Vec3::Z;
            }
            objs.push(o);
        }
        ev.push(NetEvent::Wind {
            handle: HANDLE,
            grid: Box::new(aurora_net::terrain::WindGrid {
                velocities: [Vec3::new(3.0, 2.0, 0.0); 256],
            }),
        });
    }
    ev.push(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: objs,
    });
    ev.extend(overrides);

    if std::env::var_os("AURORA_DEMO_REPLACE_PRIM").is_some() {
        ev.extend(crate::build::replace::demo::events(HANDLE, height(135.0, 125.0)));
    }
    // our avatar + a second one
    let mut avatars = Vec::new();
    for (k, (pos, first, last)) in [
        (Vec3::new(132.0, 126.0, 0.0), "Aurora", "Demo"),
        (
            Vec3::new(136.0, 128.5, 0.0),
            aurora_i18n::tr!("demo-wolf"),
            aurora_i18n::tr!("demo-purple"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        // standing on the plaza (top at plaza center height + 0.35)
        let z = height(140.0, 130.0) + 0.35 + 0.84; // object center: half the default body height (1.69 m)
        let mut o = prim(
            9000 + k as u32,
            Vec3::new(pos.x, pos.y, z),
            Quat::from_rotation_z(if k == 0 { 0.3 } else { 3.4 }),
            Vec3::new(0.45, 0.6, 1.9),
            boxp,
            te(ink, 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        o.pcode = LL_PCODE_LEGACY_AVATAR;
        o.full_id = if k == 0 {
            agent
        } else {
            Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002)
        };
        o.name_values = format!("FirstName STRING RW SV {first}\nLastName STRING RW SV {last}\nTitle STRING RW SV Aurora");
        avatars.push(o);
    }
    // Violet Wolf wears a hat (skull) and holds a glowing orb (right hand):
    // non-rigged attachments that must follow the animated bones
    for (id, point, pos, scale, volume, color) in [
        (9100u32, 2u8, Vec3::new(0.0, 0.0, 0.1), Vec3::new(0.26, 0.26, 0.2), cyl, violet),
        (9101, 6, Vec3::new(0.0, 0.0, -0.06), Vec3::splat(0.12), sphere, teal),
    ] {
        let mut o = prim(
            id,
            pos,
            Quat::IDENTITY,
            scale,
            volume,
            te(color, 0, point == 6, if point == 6 { 0.4 } else { 0.0 }),
            ExtraParams::default(),
            "",
        );
        o.parent_id = 9001;
        o.state = ((point & 0x0F) << 4) | ((point & 0xF0) >> 4);
        avatars.push(o);
    }
    let start = Vec3::new(132.0, 126.0, height(140.0, 130.0) + 0.35 + 0.84);
    // AURORA_DEMO_POS="x,y" moves the start position (captures)
    let start = match std::env::var("AURORA_DEMO_POS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
    {
        Some(p) if p.len() >= 2 => Vec3::new(p[0], p[1], height(p[0], p[1]).max(20.0) + 1.0),
        _ => start,
    };
    ev.push(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: avatars,
    });
    ev.push(NetEvent::AgentMovementComplete {
        handle: HANDLE,
        position: start,
        look_at: Vec3::new(0.8, 0.4, 0.0),
    });
    // the parcel the demo avatar stands on (About Land)
    {
        use aurora_net::parcel_flags as pf;
        // AURORA_DEMO_RESTRICTED: a parcel that forbids everything (red icons
        // in the navigation bar), with damage on and 72 % health
        let restricted = std::env::var("AURORA_DEMO_RESTRICTED").is_ok_and(|v| v == "1");
        let flags = if restricted {
            pf::ALLOW_DAMAGE | pf::RESTRICT_PUSHOBJECT
        } else {
            pf::ALLOW_FLY | pf::CREATE_OBJECTS | pf::ALLOW_OTHER_SCRIPTS | pf::ALLOW_VOICE_CHAT | pf::RESTRICT_PUSHOBJECT
        };
        if restricted {
            ev.push(NetEvent::Health(72.0));
        }
        // AURORA_DEMO_MUSIC=<url>|1: parcel music, started by the radio toggle
        let music_url = match std::env::var("AURORA_DEMO_MUSIC").unwrap_or_default().trim() {
            "" | "0" => String::new(),
            "1" => land::MUSIC_URL.into(),
            url => url.to_owned(),
        };
        ev.push(NetEvent::AgentParcel(Arc::new(aurora_net::ParcelInfo {
            local_id: 1,
            name: aurora_i18n::tr!("common-aurora-plaza").into(),
            desc: aurora_i18n::tr!("demo-aurora-viewer-s-offline-demo-parcel").into(),
            owner_id: u(100),
            area: 16384,
            max_prims: 3750,
            owner_prims: 47,
            other_prims: 2,
            sim_max_prims: 15000,
            sim_total_prims: 49,
            flags,
            see_avatars: !restricted,
            any_av_sounds: true,
            region_allow_env_override: true,
            music_url,
            ..Default::default()
        })));
    }
    for a in [agent, Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002)] {
        ev.push(NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: a,
            anims: vec![(IDLE_ANIM, 1)],
        });
    }
    ev.push(NetEvent::Sun(SunInfo {
        sun_direction: Vec3::new(0.55, 0.35, 0.6).normalize(),
        sun_phase: 0.0,
        sec_per_day: 14400,
        usec_since_start: 0,
    }));
    // AURORA_DEMO_SKY=<gamma>[,<haze distance>[,<ambient>]]: a classic EEP sky (no
    // reflection probe ambiance) with the given sky gamma and a sunlight color
    // above 1, as most Second Life skies carry: shows the legacy gamma and
    // the normalized object light (Firestorm mSunDiffuse) without a grid.
    // With a haze distance multiplier (0.8 by default, 37 in a misty sky),
    // what is a few tens of metres away is hazed. With an ambient level (0.25
    // by default, above 1 in the brightest skies), what only dims the ambient
    // light shows: the ambient occlusion
    let sky = std::env::var("AURORA_DEMO_SKY").ok().and_then(|v| {
        let mut values = v.split(',').map(|s| s.trim().parse::<f64>().ok());
        Some((values.next()??, values.next().flatten(), values.next().flatten()))
    });
    if let Some((gamma, haze_distance, ambient)) = sky {
        // an EEP region: without the ExtEnvironment capability its day is
        // not shown (the simulator sun is)
        ev.push(NetEvent::Capabilities {
            handle: HANDLE,
            caps: Arc::new(
                [
                    ("GetDisplayNames".to_owned(), "demo://names".to_owned()),
                    ("ExtEnvironment".to_owned(), "demo://environment".to_owned()),
                ]
                .into_iter()
                .collect(),
            ),
        });
        ev.push(NetEvent::Environment {
            handle: HANDLE,
            parcel_id: -1,
            environment: demo_sky(Vec3::new(0.55, 0.35, 0.6).normalize(), gamma, haze_distance, ambient),
        });
    }
    ev.push(NetEvent::FriendsOnline {
        ids: vec![u(100), u(102)],
        online: true,
    });
    ev.push(NetEvent::Chat(aurora_net::ChatMessage {
        from_name: aurora_i18n::tr!("demo-violet-wolf").into(),
        source_id: Uuid::from_u128(2),
        owner_id: Uuid::nil(),
        source_type: aurora_net::ChatSourceType::Agent,
        chat_type: aurora_net::ChatType::Normal,
        position: Vec3::ZERO,
        message: aurora_i18n::tr!("demo-welcome-to-the-aurora-viewer-demo").into(),
    }));
    // AURORA_DEMO_LSL_BRIDGE=1: a worn Firestorm bridge talking to the viewer
    // (all hidden from chat) between two ordinary llOwnerSay lines (shown)
    if std::env::var_os("AURORA_DEMO_LSL_BRIDGE").is_some() {
        let bridge = Uuid::from_u128(0xB81D_6E00_0000_0000_0000_0000_0000_0001);
        let (hud, bridge_name) = (aurora_i18n::tr!("common-demo-hud"), "#Firestorm LSL Bridge v2.32");
        for (from, name, text) in [
            (u(200), hud, aurora_i18n::tr!("demo-demo-the-next-bridge-line-must-stay-hidden")),
            (
                bridge,
                bridge_name,
                "<bridgeURL>https://sim.example.invalid:12043/cap/demo</bridgeURL><bridgeAuth>demo</bridgeAuth><bridgeVer>2.32</bridgeVer>",
            ),
            (bridge, bridge_name, "<bridgeMovelock state=0>"),
            (u(200), hud, aurora_i18n::tr!("demo-demo-end-of-bridge-test")),
        ] {
            ev.push(NetEvent::Chat(aurora_net::ChatMessage {
                from_name: name.into(),
                source_id: from,
                owner_id: agent,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                position: Vec3::ZERO,
                message: text.into(),
            }));
        }
    }
    let dialog = std::env::var("AURORA_DEMO_DIALOG").ok().filter(|mode| !mode.is_empty());
    let inventory_offer = std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| matches!(mode.as_str(), "offer" | "offer-reject"));
    ev.extend(notification_samples(
        std::env::var_os("AURORA_DEMO_NOTIF").is_some(),
        inventory_offer,
        dialog.as_deref(),
    ));
    if long_names_scenario().is_some() {
        ev.extend(long_name_events(agent));
    }
    if many_conversations_scenario() {
        ev.extend(many_conversation_events(agent));
    }
    if bubbles_scenario().is_some() {
        ev.extend(bubbles_events());
    }
    if teleport_offer_scenario() {
        ev.extend(teleport_offer_events(agent));
    }
    if let Some(scene) = lure_scene() {
        ev.extend(lure_events(scene, agent));
    }
    ev
}

/// Unrelated demos stay quiet; each explicit scenario gets only its fixtures.
fn notification_samples(notifications: bool, inventory_offer: bool, menu: Option<&str>) -> Vec<NetEvent> {
    let mut ev = Vec::new();
    let im = |dialog: u8, message: &str, bucket: Vec<u8>| {
        NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: Uuid::from_u128(2),
            from_name: aurora_i18n::tr!("demo-violet-wolf").into(),
            to_agent_id: Uuid::nil(),
            dialog,
            session_id: Uuid::new_v4(),
            message: message.into(),
            offline: false,
            from_group: false,
            binary_bucket: bucket,
            timestamp: 0,
        })
    };
    if notifications {
        ev.push(im(22, aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset"), Vec::new()));
    }
    // a resident's offer: asset type, then the item already in our inventory
    if notifications || inventory_offer {
        let mut offer = vec![6];
        offer.extend_from_slice(DEMO_OFFER.as_bytes());
        ev.push(im(4, aurora_i18n::tr!("demo-aurora-lantern"), offer));
    }
    if !notifications && menu.is_none() {
        return ev;
    }
    let menu = menu.unwrap_or_default();
    let (object_name, message, buttons) = if !menu.is_empty() {
        let mut buttons = [
            aurora_i18n::tr!("status-close"),
            aurora_i18n::tr!("status-back"),
            "▶▶",
            "Threshold",
            "Triplex*",
            " ",
            "Last One",
            "NewCity 2",
            "Skiplit 2",
            "Bid & Block",
            "Can't Stop",
            "Hearts",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        if menu == "4" {
            buttons.truncate(4);
        }
        if menu == "long" {
            buttons[9] = aurora_i18n::tr!("demo-a-very-long-button-label").into();
        }
        (
            aurora_i18n::tr!("demo-game-table"),
            aurora_i18n::tr!("demo-game-table-current-game-category-8-players-select-a"),
            buttons,
        )
    } else {
        (
            aurora_i18n::tr!("demo-fountain"),
            aurora_i18n::tr!("demo-choose-a-color-for-the-water"),
            vec![
                aurora_i18n::tr!("demo-purple").into(),
                aurora_i18n::tr!("demo-turquoise").into(),
                aurora_i18n::tr!("demo-amber").into(),
            ],
        )
    };
    ev.push(NetEvent::ScriptDialog {
        object_id: Uuid::from_u128(0xF0),
        object_name: object_name.into(),
        owner_name: aurora_i18n::tr!("demo-violet-wolf").into(),
        message: message.into(),
        channel: -4242,
        buttons,
    });
    ev
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[test]
    fn default_demo_is_quiet_and_explicit_scenarios_keep_only_relevant_notifications() {
        assert!(notification_samples(false, false, None).is_empty());
        let events = notification_samples(true, false, None);
        assert_eq!(events.len(), 3);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NetEvent::InstantMessage(im) if im.dialog == 22))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NetEvent::InstantMessage(im) if im.dialog == 4))
        );
        assert!(events.iter().any(|event| matches!(event, NetEvent::ScriptDialog { .. })));
        for mode in ["4", "12", "long"] {
            let events = notification_samples(false, false, Some(mode));
            assert!(
                matches!(events.as_slice(), [NetEvent::ScriptDialog { buttons, .. }] if buttons.len() == if mode == "4" { 4 } else { 12 })
            );
        }
        let events = notification_samples(false, true, None);
        assert!(matches!(events.as_slice(), [NetEvent::InstantMessage(im)] if im.dialog == 4));
    }
}

/// A procedural idle pose: arms down, slight breathing (demo only).
/// A short two-note chime (offline sound test).
pub fn chime() -> (Uuid, aurora_audio::SoundClip) {
    let rate = 48_000u32;
    let mut pcm = Vec::with_capacity(rate as usize);
    for i in 0..rate as usize {
        let t = i as f32 / rate as f32;
        let f = if t < 0.5 { 660.0 } else { 880.0 };
        let env = (1.0 - (t % 0.5) * 2.0).max(0.0).powi(2);
        pcm.push((t * f * std::f32::consts::TAU).sin() * env * 0.3);
    }
    (u(0x50_0001), aurora_audio::SoundClip::from_pcm(&pcm, 1, rate))
}

/// AURORA_DEMO_SOUND: an interface sound for the offline demo, read (never
/// written) from the real sound cache when a grid session already loaded it,
/// else a short tick whose pitch depends on the asset (so that different
/// sounds can be told apart).
pub fn ui_sound(id: Uuid) -> aurora_audio::SoundClip {
    let demo_cache = crate::settings::cache_dir();
    if let Some(real) = demo_cache.parent()
        && let Ok(data) = std::fs::read(real.join("sound").join(format!("{id}.ogg")))
        && let Ok(clip) = aurora_audio::SoundClip::decode(&data)
    {
        return clip;
    }
    let rate = 48_000u32;
    let f = 500.0 + (id.as_u128() % 13) as f32 * 120.0;
    let n = rate as usize * 6 / 100;
    let pcm: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let env = (1.0 - i as f32 / n as f32).powi(3);
            (t * f * std::f32::consts::TAU).sin() * env * 0.25
        })
        .collect();
    aurora_audio::SoundClip::from_pcm(&pcm, 1, rate)
}

/// Default page of AURORA_DEMO_MEDIA: animated (frame updates), a button
/// (clicks) and a text field (keyboard).
pub fn demo_media_page() -> &'static str {
    aurora_i18n::tr!("demo-media-page")
}

/// AURORA_DEMO_MEDIA: a 4 × 2.25 m screen ahead-left of the start position whose
/// west face (towards the avatar) carries media. Returns the update, the
/// object id and the face index.
pub fn media_screen() -> (NetEvent, Uuid, u8) {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    // the box face looking down -X
    let mesh = aurora_prim::generate_volume(&boxp, 1.0);
    let face = mesh
        .faces
        .iter()
        .enumerate()
        .max_by(|a, b| {
            let nx = |f: &aurora_prim::VolumeFace| -f.normals.iter().map(|n| n[0]).sum::<f32>() / f.normals.len().max(1) as f32;
            nx(a.1).partial_cmp(&nx(b.1)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i as u8)
        .unwrap_or(4);
    let mut t = TextureEntry::default();
    for f in t.faces.iter_mut() {
        f.texture = BLANK_TEXTURE;
        f.color = [0.12, 0.12, 0.16, 1.0];
    }
    // media 1024 x 576 in a 1024² texture, aligned like the edit tools'
    // "Align" button: the face shows the bottom 56 % (GL rows) of the texture
    let f = &mut t.faces[face as usize];
    f.color = [1.0, 1.0, 1.0, 1.0];
    f.media_flags |= 1;
    f.scale_t = 576.0 / 1024.0;
    f.offset_t = -(1.0 - f.scale_t) * 0.5;
    let (x, y) = (135.5, 129.5);
    let o = prim(
        900,
        Vec3::new(x, y, floor_at(x, y) + 1.9),
        Quat::IDENTITY,
        Vec3::new(0.1, 4.0, 2.25),
        boxp,
        Arc::new(t),
        ExtraParams::default(),
        "",
    );
    let id = o.full_id;
    (
        NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![o],
        },
        id,
        face,
    )
}

/// AURORA_DEMO_PARCEL_MEDIA: placeholder texture of the parcel media (the
/// first alpha test panel's texture).
pub fn parcel_media_texture() -> Uuid {
    alpha_panels()[0].0
}

/// A pillar of the plaza loops the chime; another one is triggered once.
pub fn sound_events(chime: Uuid) -> Vec<NetEvent> {
    // first pillar of the plaza (prim id 101)
    let pillar = Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | 101u128);
    vec![
        NetEvent::AttachedSound {
            object: pillar,
            sound: chime,
            owner: u(100),
            gain: 0.6,
            flags: 1,
        },
        NetEvent::SoundTrigger {
            sound: chime,
            owner: u(100),
            object: u(102),
            parent: Uuid::nil(),
            handle: HANDLE,
            position: glam::Vec3::new(140.0, 130.0, 25.0),
            gain: 0.8,
        },
    ]
}

pub fn idle_animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, RotKey};
    use glam::Quat;
    let k = |t: f32, q: Quat| RotKey { time: t, rotation: q };
    let joint = |name: &str, keys: Vec<RotKey>| JointMotion {
        joint_name: name.into(),
        priority: -1,
        rot_keys: keys,
        pos_keys: Vec::new(),
    };
    let down = 1.3f32;
    let breathe = |a: f32| Quat::from_rotation_y(a);
    aurora_assets::Animation {
        base_priority: 2,
        duration: 4.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 4.0,
        looping: true,
        ease_in: 0.3,
        ease_out: 0.3,
        hand_pose: 0,
        joints: vec![
            joint(
                "mShoulderLeft",
                vec![
                    k(0.0, Quat::from_rotation_x(-down)),
                    k(2.0, Quat::from_rotation_x(-down + 0.04)),
                    k(4.0, Quat::from_rotation_x(-down)),
                ],
            ),
            joint(
                "mShoulderRight",
                vec![
                    k(0.0, Quat::from_rotation_x(down)),
                    k(2.0, Quat::from_rotation_x(down - 0.04)),
                    k(4.0, Quat::from_rotation_x(down)),
                ],
            ),
            joint("mElbowLeft", vec![k(0.0, Quat::from_rotation_z(0.25))]),
            joint("mElbowRight", vec![k(0.0, Quat::from_rotation_z(-0.25))]),
            joint("mHipLeft", vec![k(0.0, Quat::IDENTITY)]),
            joint("mHipRight", vec![k(0.0, Quat::IDENTITY)]),
            joint("mKneeLeft", vec![k(0.0, Quat::IDENTITY)]),
            joint("mKneeRight", vec![k(0.0, Quat::IDENTITY)]),
            joint("mChest", vec![k(0.0, breathe(0.0)), k(2.0, breathe(-0.03)), k(4.0, breathe(0.0))]),
            joint(
                "mHead",
                vec![
                    k(0.0, Quat::from_rotation_z(0.0)),
                    k(2.0, Quat::from_rotation_z(0.08)),
                    k(4.0, Quat::from_rotation_z(0.0)),
                ],
            ),
        ],
    }
}

pub const IDLE_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0001);

pub const ANIMESH_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0100);
pub const ANIMESH_TEXTURE: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0101);
pub const ANIMESH_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0102);

/// AURORA_DEMO_ANIMESH=churn: the ground animesh changes its mesh asset every
/// `ANIMESH_CHURN_FRAMES` frames, as rigged meshes come and go on a grid.
pub fn animesh_churn() -> bool {
    std::env::var("AURORA_DEMO_ANIMESH").is_ok_and(|v| v == "churn")
}

pub const ANIMESH_CHURN_FRAMES: u64 = 240;

/// Mesh asset of the ground animesh after its `generation`-th change.
pub fn animesh_churn_mesh(generation: u32) -> Uuid {
    Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0001_0000_0000 | generation as u128)
}

/// The ground animesh updated with the mesh of `generation`.
pub fn animesh_churn_events(generation: u32) -> Vec<NetEvent> {
    let mut objects = animesh_objects(animesh_churn_mesh(generation));
    objects.retain(|o| o.local_id == 9200);
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// Root mesh, linked mesh driven by a child's signal, and a worn root mesh.
pub fn animesh_events() -> Vec<NetEvent> {
    let objects = animesh_objects(ANIMESH_MESH);
    let mut events: Vec<_> = objects
        .iter()
        .filter(|o| o.volume.is_mesh())
        .map(|o| NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: o.full_id,
            anims: vec![(ANIMESH_ANIM, 1)],
        })
        .collect();
    // Signals deliberately arrive before their objects, as on a grid.
    events.push(NetEvent::ObjectUpdates { handle: HANDLE, objects });
    events
}

/// Objects of the animesh demo; the ground root mesh (9200) is `ground_mesh`.
fn animesh_objects(ground_mesh: Uuid) -> Vec<ObjectUpdate> {
    let ground = floor_at(134.0, 126.0) + 0.2;
    let mut objects = Vec::new();
    for (id, pos, parent, animated, is_mesh, color) in [
        (9200, Vec3::new(134.0, 126.0, ground), 0, true, true, [0.9, 0.45, 0.1, 1.0]),
        (9201, Vec3::new(134.0, 127.5, ground), 0, true, false, [0.0; 4]),
        (9202, Vec3::ZERO, 9201, false, true, [0.1, 0.85, 0.65, 1.0]),
        (9203, Vec3::new(0.15, 0.0, 0.2), 9001, true, true, [0.55, 0.35, 0.9, 1.0]),
        (
            9204,
            Vec3::new(134.5, 127.0, ground - 0.17),
            0,
            false,
            false,
            [0.7, 0.72, 0.75, 1.0],
        ),
    ] {
        let mut t = (*te(color, 0, false, 0.0)).clone();
        for f in &mut t.faces {
            f.texture = if is_mesh { ANIMESH_TEXTURE } else { BLANK_TEXTURE };
        }
        let mesh = SculptParams {
            texture: if id == 9200 { ground_mesh } else { ANIMESH_MESH },
            sculpt_type: LL_SCULPT_TYPE_MESH,
        };
        let volume = aurora_prim::VolumeParams {
            sculpt: is_mesh.then_some(mesh),
            ..Default::default()
        };
        let mut o = prim(
            id,
            pos,
            Quat::IDENTITY,
            Vec3::splat(0.1),
            volume,
            Arc::new(t),
            ExtraParams {
                sculpt: is_mesh.then_some(mesh),
                extended_mesh_flags: animated.then_some(aurora_prim::extra::EXTENDED_MESH_ANIMATED),
                ..Default::default()
            },
            "",
        );
        o.parent_id = parent;
        if id == 9204 {
            o.scale = Vec3::new(7.0, 5.0, 0.02);
        }
        if parent == 9001 {
            o.state = 0x60;
        } // Right hand.
        objects.push(o);
    }
    objects
}

/// AURORA_DEMO_CLICKS=...,rigged: the rigged demo mesh worn on the chest of
/// the avatar `wearer` (ours), so every kind of rigged target is in the scene
/// with AURORA_DEMO_ANIMESH: an animesh on the ground (9200, 9202), one
/// worn by another avatar (9203), a rigged mesh worn by another avatar (the
/// crowd's) and this one.
pub const OWN_RIGGED: u32 = 9210;

pub fn own_rigged_events(wearer: u32) -> Vec<NetEvent> {
    let mesh = SculptParams {
        texture: ANIMESH_MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let mut t = (*te([0.95, 0.8, 0.3, 1.0], 0, false, 0.0)).clone();
    for f in &mut t.faces {
        f.texture = ANIMESH_TEXTURE;
    }
    let mut worn = prim(
        OWN_RIGGED,
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(0.1),
        aurora_prim::VolumeParams {
            sculpt: Some(mesh),
            ..Default::default()
        },
        Arc::new(t),
        ExtraParams {
            sculpt: Some(mesh),
            ..Default::default()
        },
        "",
    );
    worn.parent_id = wearer;
    worn.state = 0x10; // chest
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![worn],
    }]
}

/// Bakes of AURORA_DEMO_CLASSIC: a skin whose alpha is partly soft and cut
/// by holes (an alpha layer), and hair in strands that fade out.
pub const TEX_BAKE_SKIN: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0030);
pub const TEX_BAKE_HAIR: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0031);

/// Pixels of the classic avatar bakes (RGBA8, 128 × 128).
fn bake_texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    const N: u32 = 128;
    let hair = *id == TEX_BAKE_HAIR;
    if !hair && *id != TEX_BAKE_SKIN {
        return None;
    }
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
            if hair {
                // strands along v, opaque at their root and fading to nothing
                let strand = 0.5 + 0.5 * (u * std::f32::consts::TAU * 24.0).sin();
                let a = (strand * 1.6 - 0.2).clamp(0.0, 1.0) * (1.15 - v).clamp(0.0, 1.0);
                let shade = 0.55 + 0.45 * strand;
                px.extend([(150.0 * shade) as u8, (96.0 * shade) as u8, (52.0 * shade) as u8, (a * 255.0) as u8]);
            } else {
                // soft alpha in a band (blended, the body would show what is
                // behind it) and a row of holes
                let band = if (0.40..0.60).contains(&v) {
                    0.35 + 0.5 * (v - 0.5).abs() / 0.1
                } else {
                    1.0
                };
                let (cx, cy) = ((x % 32) as f32 - 16.0, y as f32 - 100.0);
                let hole = cx * cx + cy * cy < 36.0;
                let a = if hole { 0.0 } else { band };
                let tone = 0.9 + 0.1 * (u * 40.0).sin() * (v * 40.0).sin();
                px.extend([(214.0 * tone) as u8, (168.0 * tone) as u8, (140.0 * tone) as u8, (a * 255.0) as u8]);
            }
        }
    }
    Some((px, N, N))
}

/// AURORA_DEMO_CLASSIC: the two avatars of the plaza as classic avatars, the
/// system body with bakes whose alpha is not only 0 or 1 and two shapes far
/// from the default one: ours male with long hair at the back, the wolf
/// heavy with big hair and pigtails. Their bodies must be solid (never
/// see-through), cut by the holes of the alpha layer, each with its own
/// build; the hair shows its strands.
pub fn classic_events(shapes: &crate::scene::shape::ShapeParams) -> Vec<NetEvent> {
    use crate::scene::avatar::{BAKES, IMG_DEFAULT_AVATAR};
    let mut bakes = TextureEntry::default();
    for (slot, name, _) in BAKES {
        if let Some(f) = bakes.faces.get_mut(slot) {
            f.texture = match name {
                "head" | "upper" | "lower" => TEX_BAKE_SKIN,
                "hair" => TEX_BAKE_HAIR,
                _ => IMG_DEFAULT_AVATAR,
            };
        }
    }
    let bakes = Arc::new(bakes);
    // (parameter, weight): male 80, body fat 637, belly 157, breasts 105,
    // hair volume 763, hair front / sides / back 133 to 135, pigtails 785
    let looks: [(Uuid, &[(i32, f32)]); 2] = [
        (DEMO_AGENT, &[(80, 1.0), (135, 0.8)]),
        (
            DEMO_LOUP,
            &[(637, 1.0), (157, 1.0), (105, 0.9), (763, 1.0), (133, 1.0), (134, 1.0), (785, 1.0)],
        ),
    ];
    looks
        .into_iter()
        .map(|(avatar_id, changed)| {
            NetEvent::Appearance(aurora_net::AvatarAppearance {
                avatar_id,
                texture_entry: Some(bakes.clone()),
                visual_params: shapes.values_with(changed),
                cof_version: 1,
                hover_height: 0.0,
                attachments: 0,
            })
        })
        .collect()
}

/// Mesh worn in AURORA_DEMO_CLOUD=self: an asset the asset server does not
/// have (`MeshStreamer::insert_demo_unavailable`).
pub const CLOUD_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0110);

/// AURORA_DEMO_CLOUD=self (issue #121): the demo agent dressed like the
/// residents who stayed a cloud for the whole 60 s limit. Its appearance has
/// a head bake hidden by an alpha layer (IMG_INVISIBLE) with textured upper
/// and lower bakes, and it wears a mesh that cannot be downloaded next to a
/// plain hat. It must show a few seconds after its parts are there, without
/// its head and without the mesh.
pub fn cloud_self_events() -> Vec<NetEvent> {
    use crate::scene::avatar::{BAKES, IMG_DEFAULT_AVATAR, IMG_INVISIBLE};
    let mut bakes = TextureEntry::default();
    for (slot, name, _) in BAKES {
        if let Some(f) = bakes.faces.get_mut(slot) {
            f.texture = match name {
                "head" => IMG_INVISIBLE,
                "upper" | "lower" => TEX_TILES,
                _ => IMG_DEFAULT_AVATAR,
            };
        }
    }
    let mesh = SculptParams {
        texture: CLOUD_MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let state = |point: u8| ((point & 0x0F) << 4) | ((point & 0xF0) >> 4);
    // chest: the mesh that never comes
    let mut worn_mesh = prim(
        9110,
        Vec3::new(0.12, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::splat(0.3),
        aurora_prim::VolumeParams {
            sculpt: Some(mesh),
            ..Default::default()
        },
        te([0.37, 0.92, 0.83, 1.0], 0, false, 0.0),
        ExtraParams {
            sculpt: Some(mesh),
            ..Default::default()
        },
        "",
    );
    worn_mesh.parent_id = 9000;
    worn_mesh.state = state(1);
    // skull: a hat, where the hidden head would be
    let mut hat = prim(
        9111,
        Vec3::new(0.0, 0.0, 0.1),
        Quat::IDENTITY,
        Vec3::new(0.26, 0.26, 0.2),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
        te([0.77, 0.71, 0.99, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    hat.parent_id = 9000;
    hat.state = state(2);
    vec![
        NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![worn_mesh, hat],
        },
        NetEvent::Appearance(aurora_net::AvatarAppearance {
            avatar_id: DEMO_AGENT,
            texture_entry: Some(Arc::new(bakes)),
            visual_params: Vec::new(),
            cof_version: 1,
            hover_height: 0.0,
            attachments: 0,
        }),
    ]
}

/// AURORA_DEMO_TEXTURES=<n>: texture stress test, `n` small cubes (9000 for
/// any value that is not a number, as many textures as a busy region), each
/// with its own texture. Returns the cube count.
/// AURORA_DEMO_HOVER=1: the cursor swept over the window from frame 300,
/// then left still, in turns of 240 frames (hover search and kept answers,
/// scene/hover.rs). The cursor position at `frame` in a window of `size`.
/// AURORA_DEMO_HOVER="x,y": the cursor left on that pixel instead (on an
/// avatar of the crowd, on the ground beside it).
pub fn hover_sweep(frame: u64, (w, h): (u32, u32)) -> Option<(f32, f32)> {
    static ON: std::sync::OnceLock<Option<Option<(f32, f32)>>> = std::sync::OnceLock::new();
    let fixed = (*ON.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_HOVER").ok()?;
        Some(
            v.split_once(',')
                .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?))),
        )
    }))?;
    if frame < 300 {
        return None;
    }
    if fixed.is_some() {
        return fixed;
    }
    let f = frame - 300;
    // (the last position of a sweep is held while the cursor rests)
    let moving = f / 240 * 120 + (f % 240).min(119);
    let t = moving as f32 * 0.021;
    let x = 0.5 + 0.42 * (t * 1.3).sin();
    let y = 0.55 + 0.3 * (t * 0.9 + 1.0).sin();
    Some(((x * w as f32).floor(), (y * h as f32).floor()))
}

pub fn texture_stress_count() -> Option<u32> {
    let v = std::env::var("AURORA_DEMO_TEXTURES").ok()?;
    Some(v.trim().parse().unwrap_or(9000).min(20_000))
}

/// Texture of stress cube `i`.
pub fn stress_texture(i: u32) -> Uuid {
    Uuid::from_u128(0xDE40_7E57_0000_0000_0000_0000_0000_0000 | i as u128)
}

/// Level-0 sizes of the stress textures, cycled: the common square and
/// oblong powers of two, plus one size that is not a power of two.
const STRESS_SIZES: [(u32, u32); 8] = [(64, 64), (32, 32), (128, 64), (64, 128), (128, 128), (16, 16), (48, 48), (64, 32)];

/// Mip chain of stress texture `i` at a discard level (level 0 halved
/// `discard` times), down to 1×1: a 4 × 4 checker of a color proper to the
/// texture and its inverse, so a texture showing in the wrong place or at
/// the wrong level is visible.
pub fn stress_texture_mips(i: u32, discard: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let (w, h) = STRESS_SIZES[i as usize % STRESS_SIZES.len()];
    let base = [(i * 97 % 256) as u8, (i * 57 % 256) as u8, (i * 31 % 256) as u8];
    let mut out = Vec::new();
    let mut level = discard;
    loop {
        let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
        let mut px = Vec::with_capacity((lw * lh * 4) as usize);
        for y in 0..lh {
            for x in 0..lw {
                let on = (x * 4 / lw + y * 4 / lh) % 2 == 0;
                let c = if on { base } else { base.map(|v| 255 - v) };
                px.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        out.push((lw, lh, px));
        if lw == 1 && lh == 1 {
            break;
        }
        level += 1;
    }
    out
}

/// The cubes of AURORA_DEMO_TEXTURES: a square field east of the plaza,
/// facing the start position, 0.5 m cubes every 0.8 m.
pub fn texture_stress_events(n: u32) -> Vec<NetEvent> {
    stress_cubes(n, |_| true, 0)
}

/// AURORA_DEMO_TP_AWAY=1: the arrival in a region out of reach at frame 760,
/// then back in the demo region at frame 960, before its simulator would
/// have closed it: the objects of the demo region are parked, then come
/// back as they were (world/parked.rs). The events of `frame`, if any.
pub fn away_teleport(frame: u64, position: Vec3, look_at: Vec3) -> Option<Vec<NetEvent>> {
    let handle = match frame {
        760 => HANDLE + ((100 * 256u64) << 32),
        960 => HANDLE,
        _ => return None,
    };
    std::env::var_os("AURORA_DEMO_TP_AWAY")?;
    Some(vec![
        NetEvent::TeleportFinished { handle },
        NetEvent::MainRegionChanged { handle },
        NetEvent::AgentMovementComplete { handle, position, look_at },
    ])
}

/// AURORA_DEMO_TEXTURES_CHURN=1: streaming churn on top of the stress test,
/// as in a busy region where objects come and go. The cubes of
/// `stress_churned` are removed at frame 300 (their textures are evicted a
/// few seconds later, which frees layers and compacts pages), then put back
/// at frame 700 with new textures (placeholders in the freed slots, then
/// low and full resolution).
pub fn texture_churn() -> bool {
    std::env::var_os("AURORA_DEMO_TEXTURES_CHURN").is_some()
}

pub fn stress_churned(i: u32) -> bool {
    i.is_multiple_of(3)
}

pub fn texture_churn_kill(n: u32) -> Vec<NetEvent> {
    vec![NetEvent::ObjectsKilled {
        handle: HANDLE,
        local_ids: (0..n).filter(|&i| stress_churned(i)).map(|i| 50_000 + i).collect(),
    }]
}

/// The removed cubes back, each with texture `n + i`.
pub fn texture_churn_respawn(n: u32) -> Vec<NetEvent> {
    stress_cubes(n, stress_churned, n)
}

fn stress_cubes(n: u32, keep: impl Fn(u32) -> bool, texture_offset: u32) -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let side = (n as f32).sqrt().ceil().max(1.0) as u32;
    let mut objects = Vec::with_capacity(n as usize);
    for i in (0..n).filter(|&i| keep(i)) {
        let (x, y) = (
            146.0 + (i / side) as f32 * 0.8,
            126.0 + ((i % side) as f32 - side as f32 * 0.5) * 0.8,
        );
        let mut t = (*te([1.0; 4], 0, false, 0.0)).clone();
        for f in &mut t.faces {
            f.texture = stress_texture(texture_offset + i);
        }
        objects.push(prim(
            50_000 + i,
            Vec3::new(x, y, height(x, y) + 0.25),
            Quat::IDENTITY,
            Vec3::splat(0.5),
            boxp,
            Arc::new(t),
            ExtraParams::default(),
            "",
        ));
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// AURORA_DEMO_CROWD=<n>: `n` extra avatars (40 for any value that is not a
/// number), as in a busy shop or club. Returns the avatar count.
pub fn crowd_count() -> Option<u32> {
    let v = std::env::var("AURORA_DEMO_CROWD").ok()?;
    Some(v.split(',').next().unwrap_or("").trim().parse().unwrap_or(40).min(400))
}

/// AURORA_DEMO_CROWD=<n>,busy: the crowd with what a busy place adds to
/// the scene sync ([`crowd_busy_events`]).
pub fn crowd_busy() -> bool {
    std::env::var("AURORA_DEMO_CROWD").is_ok_and(|v| v.split(',').skip(1).any(|p| p.trim() == "busy"))
}

/// Prims of the HUD our avatar wears in the busy crowd (its root included).
pub const BUSY_HUD_PRIMS: u32 = 250;
/// Spinning objects on the ground of the busy crowd, each with a prim
/// linked below it.
pub const BUSY_SPINNERS: u32 = 60;

/// What a busy place has besides worn prims and meshes, for `n` crowd
/// avatars: on each of them a flexible ribbon and a spinning ornament with
/// two prims linked below it (none of which the GPU places from an anchor);
/// on our avatar a HUD of [`BUSY_HUD_PRIMS`] prims; and
/// [`BUSY_SPINNERS`] spinning objects on the ground.
pub fn crowd_busy_events(n: u32) -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut flexp = boxp;
    flexp.path.curve_type = aurora_prim::params::LL_PCODE_PATH_FLEXIBLE;
    let worn_on = |o: &mut ObjectUpdate, avatar: u32, point: u8| {
        o.parent_id = avatar;
        o.state = ((point & 0x0F) << 4) | ((point & 0xF0) >> 4);
    };
    let mut objects = Vec::new();
    for k in 0..n {
        // (the crowd's avatars and the ids their attachments leave free)
        let avatar_id = 100_000 + k * 64;
        let mut ribbon = prim(
            avatar_id + 57,
            Vec3::new(-0.12, 0.0, 0.1),
            Quat::from_rotation_y(std::f32::consts::PI),
            Vec3::new(0.02, 0.06, 0.5),
            flexp,
            te([0.9, 0.3, 0.5, 1.0], 0, false, 0.0),
            ExtraParams {
                flexible: Some(crate::build::edits::FLEX_DEFAULT),
                ..Default::default()
            },
            "",
        );
        worn_on(&mut ribbon, avatar_id, 1);
        objects.push(ribbon);
        let mut ornament = prim(
            avatar_id + 58,
            Vec3::new(0.0, 0.0, 0.3),
            Quat::IDENTITY,
            Vec3::splat(0.06),
            boxp,
            te([1.0, 0.85, 0.2, 1.0], 2, false, 0.0),
            ExtraParams::default(),
            "",
        );
        ornament.angular_velocity = Vec3::Z * 1.5;
        worn_on(&mut ornament, avatar_id, 2);
        objects.push(ornament);
        for (c, side) in [(59, -1.0), (61, 1.0)] {
            let mut gem = prim(
                avatar_id + c,
                Vec3::new(0.08 * side, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::splat(0.03),
                boxp,
                te([0.3, 0.7, 1.0, 1.0], 2, false, 0.0),
                ExtraParams::default(),
                "",
            );
            gem.parent_id = avatar_id + 58;
            objects.push(gem);
        }
    }
    for i in 0..BUSY_SPINNERS {
        let (x, y) = (132.0 + (i % 12) as f32 * 2.2, 118.0 + (i / 12) as f32 * 2.2);
        let id = 2_000_000 + i * 2;
        let mut root = prim(
            id,
            Vec3::new(x, y, floor_at(x, y) + 0.6),
            Quat::IDENTITY,
            Vec3::splat(0.3),
            boxp,
            te([0.5, 0.9, 0.6, 1.0], 1, false, 0.0),
            ExtraParams::default(),
            "",
        );
        root.angular_velocity = Vec3::Z * (0.03 + 0.02 * (i % 5) as f32);
        objects.push(root);
        let mut arm = prim(
            id + 1,
            Vec3::new(0.4, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(0.5, 0.06, 0.06),
            boxp,
            te([0.9, 0.9, 0.9, 1.0], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        arm.parent_id = id;
        objects.push(arm);
    }
    // the HUD: a root and a panel of small tiles, bottom centre (point 36)
    let hud_id = 2_100_000;
    for i in 0..BUSY_HUD_PRIMS {
        let mut tile = if i == 0 {
            let mut root = prim(
                hud_id,
                Vec3::new(0.0, 0.17, 0.02),
                Quat::IDENTITY,
                Vec3::new(0.004, 0.012, 0.012),
                boxp,
                te([0.75, 0.65, 1.0, 1.0], 0, true, 0.0),
                ExtraParams::default(),
                "",
            );
            worn_on(&mut root, 9000, 36);
            root
        } else {
            let (col, row) = (i % 25, i / 25);
            let mut tile = prim(
                hud_id + i,
                Vec3::new(0.0, -(col as f32) * 0.014, row as f32 * 0.014),
                Quat::IDENTITY,
                Vec3::new(0.004, 0.012, 0.012),
                boxp,
                te([0.3 + 0.028 * col as f32, 0.9 - 0.08 * row as f32, 0.8, 1.0], 0, true, 0.0),
                ExtraParams::default(),
                "",
            );
            tile.parent_id = hud_id;
            tile
        };
        tile.owner_id = DEMO_AGENT;
        objects.push(tile);
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// AURORA_DEMO_ANIMESH_TRIS=<n>: triangles added to the demo's rigged mesh
/// (0 without the variable), in opaque faces of 45 000.
pub fn animesh_dense_triangles() -> usize {
    std::env::var("AURORA_DEMO_ANIMESH_TRIS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// AURORA_DEMO_CROWD=<n>,<worn>[,<triangles>]: the crowd of a club. Each
/// avatar dances and wears `worn` more rigged meshes (300 at most), one in
/// a hundred of them scripted, see-through and of `triangles` triangles
/// (20,000 unless given): what the hover search meets among dressed
/// avatars. Needs AURORA_DEMO_ANIMESH. Returns (worn, triangles).
pub fn crowd_worn() -> Option<(u32, u32)> {
    let v = std::env::var("AURORA_DEMO_CROWD").ok()?;
    let mut fields = v.split(',').skip(1).map(|f| f.trim().parse::<u32>().ok());
    let worn = fields.next()??.min(300);
    let triangles = fields.next().flatten().unwrap_or(20_000).clamp(12, 200_000);
    (worn > 0).then_some((worn, triangles))
}

pub const CROWD_DENSE_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0110);
pub const CROWD_DANCE_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0111);

/// The dance of AURORA_DEMO_CROWD=<n>,<worn>: the whole body sways half a
/// meter and turns, arms swinging, so everything worn moves at each frame.
pub fn crowd_dance_animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, PosKey, RotKey};
    let mut a = idle_animation();
    a.duration = 2.0;
    a.loop_out = 2.0;
    let swing = |name: &str, axis: Vec3, angle: f32| JointMotion {
        joint_name: name.into(),
        priority: 4,
        rot_keys: [(0.0, -1.0), (1.0, 1.0), (2.0, -1.0)]
            .into_iter()
            .map(|(time, s)| RotKey {
                time,
                rotation: Quat::from_axis_angle(axis, s * angle),
            })
            .collect(),
        pos_keys: Vec::new(),
    };
    a.joints.retain(|j| j.joint_name != "mElbowLeft" && j.joint_name != "mElbowRight");
    a.joints.extend([
        swing("mTorso", Vec3::Z, 0.5),
        swing("mElbowLeft", Vec3::Z, 0.9),
        swing("mElbowRight", Vec3::Z, 0.9),
        JointMotion {
            joint_name: "mPelvis".into(),
            priority: 4,
            rot_keys: Vec::new(),
            pos_keys: [
                (0.0, -0.25, 0.0),
                (0.5, 0.0, 0.06),
                (1.0, 0.25, 0.0),
                (1.5, 0.0, -0.06),
                (2.0, -0.25, 0.0),
            ]
            .into_iter()
            .map(|(time, y, z)| PosKey {
                time,
                position: Vec3::new(0.0, y, z),
            })
            .collect(),
        },
    ]);
    a
}

/// The crowd: avatars on a grid north-east of the start, each playing the
/// demo idle animation and wearing eight attachment linksets (a root and six
/// child prims) on bones all over the body, so every worn prim follows its
/// animated bone each frame. With `rigged` (AURORA_DEMO_ANIMESH, whose mesh
/// and skin are installed), each also wears the rigged demo mesh.
pub fn crowd_events(n: u32, rigged: bool) -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    // chest, skull, shoulders, hands, feet (attachment points 1-8)
    const POINTS: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
    let mut objects = Vec::new();
    let mut anims = Vec::new();
    let club = crowd_worn().filter(|_| rigged);
    for k in 0..n {
        let (x, y) = (138.0 + (k % 10) as f32 * 1.6, 132.0 + (k / 10) as f32 * 1.6);
        let avatar_id = 100_000 + k * 64;
        let mut a = prim(
            avatar_id,
            Vec3::new(x, y, floor_at(x, y) + 0.84),
            Quat::from_rotation_z(k as f32 * 0.7),
            Vec3::new(0.45, 0.6, 1.9),
            boxp,
            te([1.0; 4], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        a.pcode = LL_PCODE_LEGACY_AVATAR;
        a.full_id = Uuid::from_u128(0xA0E0_C0D0_0000_0000_0000_0000_0000_0000 | k as u128);
        a.name_values = format!("FirstName STRING RW SV Foule{k}\nLastName STRING RW SV Resident");
        anims.push(NetEvent::AvatarAnimations {
            avatar: a.full_id,
            anims: vec![(if club.is_some() { CROWD_DANCE_ANIM } else { IDLE_ANIM }, 1)],
            sources: Vec::new(),
        });
        objects.push(a);
        // the meshes of a dressed avatar: body, hair, clothes; some with a
        // script and see-through faces, which the hover search must test
        for i in 0..club.map_or(0, |c| c.0) {
            let dense = i % 100 == 0;
            let mesh = SculptParams {
                texture: if dense { CROWD_DENSE_MESH } else { ANIMESH_MESH },
                sculpt_type: LL_SCULPT_TYPE_MESH,
            };
            let mut t = (*te([0.6, 0.7, 0.9, 1.0], 0, false, 0.0)).clone();
            for f in &mut t.faces {
                f.texture = if dense { ANIMESH_TEXTURE } else { BLANK_TEXTURE };
            }
            let mut worn = prim(
                2_000_000 + k * 512 + i,
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(if dense { 1.0 } else { 0.1 }),
                aurora_prim::VolumeParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                Arc::new(t),
                ExtraParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                "",
            );
            worn.parent_id = avatar_id;
            let point = POINTS[i as usize % POINTS.len()];
            worn.state = ((point & 0x0F) << 4) | ((point & 0xF0) >> 4);
            if dense {
                // (a touch script)
                worn.update_flags = 1 << 7;
            }
            objects.push(worn);
        }
        let hue = k as f32 / n.max(1) as f32;
        for (s, &point) in POINTS.iter().enumerate() {
            let root_id = avatar_id + 1 + s as u32 * 7;
            let color = [0.3 + 0.6 * hue, 0.8 - 0.5 * hue, 0.3 + 0.08 * s as f32, 1.0];
            let mut root = prim(
                root_id,
                Vec3::new(0.0, 0.0, 0.04),
                Quat::IDENTITY,
                Vec3::splat(0.08),
                sphere,
                te(color, 1, false, 0.0),
                ExtraParams::default(),
                "",
            );
            root.parent_id = avatar_id;
            root.state = ((point & 0x0F) << 4) | ((point & 0xF0) >> 4);
            objects.push(root);
            for c in 0..6u32 {
                let angle = c as f32 * std::f32::consts::TAU / 6.0;
                let mut child = prim(
                    root_id + 1 + c,
                    Vec3::new(angle.cos() * 0.07, angle.sin() * 0.07, 0.0),
                    Quat::from_rotation_z(angle),
                    Vec3::new(0.05, 0.02, 0.02),
                    boxp,
                    te(color, 0, false, 0.0),
                    ExtraParams::default(),
                    "",
                );
                child.parent_id = root_id;
                objects.push(child);
            }
        }
        if rigged {
            let mesh = SculptParams {
                texture: ANIMESH_MESH,
                sculpt_type: LL_SCULPT_TYPE_MESH,
            };
            let mut t = (*te([0.85, 0.85, 0.9, 1.0], 0, false, 0.0)).clone();
            // the fur on the mesh's own face; the faces of
            // AURORA_DEMO_ANIMESH_TRIS stay opaque
            let fur = if animesh_dense_triangles() > 0 { 1 } else { t.faces.len() };
            for f in t.faces.iter_mut().take(fur) {
                f.texture = ANIMESH_TEXTURE;
            }
            let mut worn = prim(
                avatar_id + 60,
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(0.1),
                aurora_prim::VolumeParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                Arc::new(t),
                ExtraParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                "",
            );
            worn.parent_id = avatar_id;
            worn.state = 0x10; // chest
            objects.push(worn);
        }
    }
    let mut events = vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }];
    events.extend(anims);
    events
}

/// AURORA_DEMO_LIGHTS=<n>: `n` small local lights (64 for any value that is
/// not a number) over the crowd's place and around it, 1.6 m apart at head
/// height, as the face lights avatars wear and the lamps of a club. The
/// renderer takes the 64 nearest.
pub fn lights_count() -> Option<u32> {
    let v = std::env::var("AURORA_DEMO_LIGHTS").ok()?;
    Some(v.trim().parse().unwrap_or(64).min(256))
}

/// The lights of [`lights_count`]: small glowing spheres, each a light of
/// 3 m (4.5 m once extended as Firestorm's deferred lights are).
pub fn lights_events(n: u32) -> Vec<NetEvent> {
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let objects = (0..n)
        .map(|k| {
            let (x, y) = (138.8 + (k % 10) as f32 * 1.6, 132.8 + (k / 10) as f32 * 1.6);
            let hue = k as f32 * 0.37;
            let color = [
                0.6 + 0.4 * hue.sin(),
                0.6 + 0.4 * (hue + 2.1).sin(),
                0.6 + 0.4 * (hue + 4.2).sin(),
                1.0,
            ];
            prim(
                300_000 + k,
                Vec3::new(x, y, floor_at(x, y) + 1.9),
                Quat::IDENTITY,
                Vec3::splat(0.06),
                sphere,
                te(color, 0, true, 0.3),
                ExtraParams {
                    light: Some(LightParams {
                        color: [color[0], color[1], color[2], 0.6],
                        radius: 3.0,
                        cutoff: 0.0,
                        falloff: 0.75,
                    }),
                    ..Default::default()
                },
                "",
            )
        })
        .collect();
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// AURORA_DEMO_HAIR=<layers>[,<segments>[,<empty %>[,<period>]]]: the load
/// of the blended pass around avatars. Our avatar and each one of the crowd
/// (AURORA_DEMO_CROWD) wear `layers` nested rigged shells around the head
/// and the torso, as the stacked cards of mesh hair and layered clothes:
/// `segments` quads along each side of a shell (its triangle count), a
/// texture of strands `period` texels apart of which `empty` percent of the
/// texels are fully transparent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HairDemo {
    pub layers: u32,
    pub segments: u32,
    pub empty: u32,
    pub period: u32,
}

pub fn hair_demo() -> Option<HairDemo> {
    let v = std::env::var("AURORA_DEMO_HAIR").ok()?;
    Some(hair_demo_from(&v))
}

fn hair_demo_from(v: &str) -> HairDemo {
    let mut p = v.split(',').map(|s| s.trim().parse::<u32>().ok());
    let mut next = |default: u32| p.next().flatten().unwrap_or(default);
    HairDemo {
        layers: next(8).clamp(1, 32),
        // (a shell face keeps 16-bit indices: at most 181² vertices)
        segments: next(24).clamp(4, 180),
        // (above, the texture would count as a mask)
        empty: next(60).min(90),
        period: next(16).clamp(4, 256),
    }
}

/// Strand texture of the hair demo.
pub const TEX_HAIR: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0020);

/// Mesh asset of one layer of the hair demo (a head shell and a torso shell).
pub fn hair_mesh(layer: u32) -> Uuid {
    Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0002_0000_0000 | layer as u128)
}

/// The shells worn by our avatar and by the `crowd` avatars of
/// [`crowd_events`]: one rigged mesh per layer, blended (the texture's alpha).
pub fn hair_events(demo: HairDemo, crowd: u32) -> Vec<NetEvent> {
    let mut objects = Vec::new();
    let avatars = std::iter::once(9000).chain((0..crowd).map(|k| 100_000 + k * 64));
    for (k, avatar_id) in avatars.enumerate() {
        for layer in 0..demo.layers {
            let mesh = SculptParams {
                texture: hair_mesh(layer),
                sculpt_type: LL_SCULPT_TYPE_MESH,
            };
            let shade = 0.55 + 0.45 * (layer as f32 / demo.layers as f32);
            let mut t = (*te([shade, 0.8 * shade, 0.6 * shade, 1.0], 0, false, 0.0)).clone();
            for f in &mut t.faces {
                f.texture = TEX_HAIR;
            }
            let mut worn = prim(
                200_000 + k as u32 * 64 + layer,
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(0.1),
                aurora_prim::VolumeParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                Arc::new(t),
                ExtraParams {
                    sculpt: Some(mesh),
                    ..Default::default()
                },
                "",
            );
            worn.parent_id = avatar_id;
            worn.state = 0x10; // chest
            objects.push(worn);
        }
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// Pixels of [`TEX_HAIR`] (RGBA8, 256×256): strands along t, `period`
/// texels apart, `empty` percent of each period fully transparent; the
/// strands thin out from the roots (top) to the tips.
fn hair_texture(empty: u32, period: u32) -> (Vec<u8>, u32, u32) {
    const N: u32 = 256;
    let period = period.clamp(1, N);
    let filled = (period as f32 * (1.0 - empty.min(100) as f32 / 100.0)).round() as u32;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        let fade = 1.0 - 0.6 * y as f32 / (N - 1) as f32;
        for x in 0..N {
            let (strand, at) = (x / period, x % period);
            let alpha = if at < filled {
                // soft on the strand's first and last texel
                let edge = if filled > 2 && (at == 0 || at + 1 == filled) { 0.5 } else { 1.0 };
                (230.0 * fade * edge) as u8
            } else {
                0
            };
            let tone = 0.75 + 0.25 * ((strand * 37 % 11) as f32 / 10.0);
            px.extend_from_slice(&[(214.0 * tone) as u8, (170.0 * tone) as u8, (110.0 * tone) as u8, alpha]);
        }
    }
    (px, N, N)
}

/// The procedural texture `id` takes a mip chain (the others are the
/// 64-texel test panels, kept as they are).
pub fn local_texture_mips(id: &Uuid) -> bool {
    *id == TEX_HAIR || *id == TEX_PROJECTED
}

#[cfg(test)]
mod hair_tests {
    use super::*;
    use crate::scene::jobs::{AlphaKind, classify_alpha};

    #[test]
    fn hair_demo_reads_its_parameters() {
        let d = hair_demo_from("1");
        assert_eq!((d.layers, d.segments, d.empty, d.period), (1, 24, 60, 16));
        let d = hair_demo_from("on");
        assert_eq!((d.layers, d.segments, d.empty, d.period), (8, 24, 60, 16));
        let d = hair_demo_from("32, 96, 90, 128");
        assert_eq!((d.layers, d.segments, d.empty, d.period), (32, 96, 90, 128));
        // kept within what a shell face and a blended texture can be
        let d = hair_demo_from("500,1000,100,1");
        assert_eq!((d.layers, d.segments, d.empty, d.period), (32, 180, 90, 4));
    }

    /// Whatever the share of empty texels, the strands stay a blended
    /// texture (never a mask, never opaque), and the share is the one asked.
    #[test]
    fn hair_texture_is_blended_with_the_asked_share_of_empty_texels() {
        for (empty, period) in [(0, 16), (60, 16), (90, 16), (60, 128), (90, 256), (50, 4)] {
            let (px, w, h) = hair_texture(empty, period);
            assert_eq!(px.len(), (w * h * 4) as usize);
            assert_eq!(classify_alpha(&px), AlphaKind::Blend, "{empty} % empty, period {period}");
            let transparent = px.as_chunks::<4>().0.iter().filter(|p| p[3] == 0).count() as f32 / (w * h) as f32;
            assert!((transparent - empty as f32 / 100.0).abs() < 0.05, "{empty} % asked, {transparent}");
        }
    }

    #[test]
    fn every_avatar_wears_its_layers() {
        let demo = hair_demo_from("5");
        let events = hair_events(demo, 3);
        let [NetEvent::ObjectUpdates { objects, .. }] = events.as_slice() else {
            panic!("one update of objects");
        };
        // our avatar and three of the crowd, each parent of its five meshes
        assert_eq!(objects.len(), 4 * 5);
        for parent in [9000, 100_000, 100_064, 100_128] {
            assert_eq!(objects.iter().filter(|o| o.parent_id == parent).count(), 5);
        }
        assert!(objects.iter().all(|o| o.volume.is_mesh()));
        let ids: std::collections::HashSet<u32> = objects.iter().map(|o| o.local_id).collect();
        assert_eq!(ids.len(), objects.len());
        assert_eq!(lights_events(70).len(), 1);
    }
}

pub fn animesh_animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, RotKey};
    let mut a = idle_animation();
    a.duration = 2.0;
    a.loop_out = 2.0;
    a.joints = ["mHead", "mTail1"]
        .into_iter()
        .map(|name| JointMotion {
            joint_name: name.into(),
            priority: 4,
            rot_keys: [(0.0, -0.65), (1.0, 0.65), (2.0, -0.65)]
                .into_iter()
                .map(|(time, angle)| RotKey {
                    time,
                    rotation: Quat::from_rotation_z(angle),
                })
                .collect(),
            pos_keys: Vec::new(),
        })
        .collect();
    a
}

/// Offline seam test: different first/last poses, first key at 1/15 s.
pub fn loop_animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, PosKey, RotKey};
    use glam::Quat;
    let mut anim = idle_animation();
    anim.duration = 1.0;
    anim.loop_out = 1.0;
    anim.ease_in = 0.6;
    anim.ease_out = 0.6;
    for joint in &mut anim.joints {
        joint.rot_keys.truncate(1);
    }
    for (name, axis, amplitude) in [
        ("mElbowLeft", Vec3::Z, 0.8),
        ("mElbowRight", Vec3::Z, -0.8),
        ("mHipLeft", Vec3::Y, 0.35),
        ("mHipRight", Vec3::Y, -0.35),
    ] {
        anim.joints.retain(|joint| joint.joint_name != name);
        anim.joints.push(JointMotion {
            joint_name: name.into(),
            priority: -1,
            rot_keys: [(1.0 / 15.0, 1.0), (0.5, -1.0), (1.0, -0.5)]
                .into_iter()
                .map(|(time, value)| RotKey {
                    time,
                    rotation: Quat::from_axis_angle(axis, value * amplitude),
                })
                .collect(),
            pos_keys: Vec::new(),
        });
    }
    anim.joints.push(JointMotion {
        joint_name: "mPelvis".into(),
        priority: -1,
        rot_keys: Vec::new(),
        pos_keys: [(1.0 / 15.0, 0.04), (0.5, 0.0), (1.0, -0.04)]
            .into_iter()
            .map(|(time, z)| PosKey {
                time,
                position: Vec3::Z * z,
            })
            .collect(),
    });
    anim
}

/// Sequence changes every 120 frames; a real stop/restart every 1200.
pub fn loop_animation_events(frame: u64) -> Vec<NetEvent> {
    let step = frame % 1200;
    if step != 1020 && !frame.is_multiple_of(120) {
        return Vec::new();
    }
    log::info!(
        "demo animation loop: frame {frame}, {}",
        if step == 960 { "stop" } else { "sequence" }
    );
    [DEMO_AGENT, Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002)]
        .into_iter()
        .map(|avatar| NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar,
            anims: if step == 960 {
                Vec::new()
            } else {
                vec![(IDLE_ANIM, (frame / 120 + 1) as i32)]
            },
        })
        .collect()
}

/// Offline voice: the other avatar talks in bursts; our own outline follows
/// the transmitted microphone. AURORA_DEMO_VOICE exercises both tags without
/// microphone input: speaking / loud / listening / off / cycle (4 s / 3 s / 2 s).
pub fn voice_levels(
    t: f64,
    me: Uuid,
    enabled: bool,
    talking: bool,
    mic_level: f32,
    mode: Option<&str>,
) -> std::collections::HashMap<Uuid, (f32, bool)> {
    let mut m = std::collections::HashMap::new();
    let t = t as f32;
    let mode = match mode {
        Some("cycle") => Some(if t.rem_euclid(9.0) < 4.0 {
            "speaking"
        } else if t.rem_euclid(9.0) < 7.0 {
            "listening"
        } else {
            "off"
        }),
        mode => mode,
    };
    let synthetic = matches!(mode, Some("speaking" | "loud" | "listening"));
    if mode == Some("off") || (!synthetic && !enabled) {
        return m;
    }
    let other = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002);
    let burst = match mode {
        Some("speaking" | "loud") => true,
        Some("listening") => false,
        _ => t.rem_euclid(6.0) < 3.5,
    };
    let level = if mode == Some("loud") {
        // Reach red with a realistic WebRTC participant level, below clipping.
        0.60
    } else if burst {
        0.35 + 0.26 * (t * 2.3).sin().abs() * (0.6 + 0.4 * (t * 7.1).sin())
    } else {
        0.0
    };
    m.insert(other, (level, burst && level > 0.2));
    let mine = if synthetic {
        level
    } else if talking {
        mic_level
    } else {
        0.0
    };
    m.insert(me, (mine, mine > aurora_voice::SPEAKING_AUDIO_LEVEL));
    m
}

#[cfg(test)]
mod voice_tests {
    use super::*;

    #[test]
    fn own_demo_tag_respects_voice_and_transmission() {
        assert!(voice_levels(1.0, DEMO_AGENT, false, true, 0.6, None).is_empty());
        assert_eq!(voice_levels(1.0, DEMO_AGENT, true, false, 0.6, None)[&DEMO_AGENT], (0.0, false));
        assert_eq!(voice_levels(1.0, DEMO_AGENT, true, true, 0.6, None)[&DEMO_AGENT], (0.6, true));
    }

    #[test]
    fn scripted_voice_exercises_speaking_listening_and_disconnection_on_both_tags() {
        let speaking = voice_levels(1.0, DEMO_AGENT, false, false, 0.0, Some("cycle"));
        assert_eq!(speaking.len(), 2);
        assert!(speaking.values().all(|(_, speaking)| *speaking));
        let loud = voice_levels(1.0, DEMO_AGENT, false, false, 0.0, Some("loud"));
        assert_eq!(loud.len(), 2);
        assert!(loud.values().all(|(level, speaking)| *speaking && (0.58..0.632).contains(level)));
        let listening = voice_levels(5.0, DEMO_AGENT, false, false, 0.0, Some("cycle"));
        assert_eq!(listening.len(), 2);
        assert!(listening.values().all(|v| *v == (0.0, false)));
        assert!(voice_levels(8.0, DEMO_AGENT, true, true, 0.6, Some("cycle")).is_empty());
    }
}

// ---------------------------------------------------------------- alpha tests

/// Procedural demo textures (offline): alpha gradient, holes, bump normals.
pub const TEX_GRADIENT: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0001);
/// Opaque parcel placeholder isolated from the alpha-regression panels.
pub const ACTION_MEDIA_TEX: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0011);
pub const TEX_HOLES: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0002);
pub const TEX_BUMPS: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0003);
/// Floor tiles of AURORA_DEMO_PLANAR: 2 × 2 tiles with grout lines, the
/// top-left tile marked so the orientation shows.
pub const TEX_TILES: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0004);
/// Frames of AURORA_DEMO_TEXANIM: a 4 × 4 grid, one hue per frame (frame 0
/// top left, red, then along the rows), each cell framed in dark.
pub const TEX_FRAMES: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0005);
/// Parcel snapshot of AURORA_DEMO_PLACE: a night landscape (sky with an
/// aurora, hills, water), top row first.
pub const TEX_SNAPSHOT: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0006);
/// Image of the demo projectors (AURORA_DEMO_PROJECTOR): four colored
/// quarters, red at the top left, around a white disc.
pub const TEX_PROJECTED: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0007);
const MAT_NONE: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0001);
const MAT_MASK: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0002);
const MAT_EMISSIVE: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0003);
const MAT_BUMPY: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0004);
const MAT_BLEND: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0005);
const TEX_PICK_ZERO: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0012);
const TEX_PICK_FAINT: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0013);
const TEX_PICK_HALF: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0014);

/// (texture, legacy material, hover text) of the test panels.
fn alpha_panels() -> [(Uuid, Option<Uuid>, &'static str); 6] {
    [
        (TEX_GRADIENT, None, aurora_i18n::tr!("demo-auto-blend")),
        (TEX_GRADIENT, Some(MAT_NONE), aurora_i18n::tr!("demo-alpha-mode-none")),
        (TEX_GRADIENT, Some(MAT_MASK), aurora_i18n::tr!("demo-mask-50-cutoff")),
        (TEX_GRADIENT, Some(MAT_EMISSIVE), aurora_i18n::tr!("common-emissive-mask")),
        (TEX_HOLES, None, aurora_i18n::tr!("demo-auto-mask")),
        (BLANK_TEXTURE, Some(MAT_BUMPY), aurora_i18n::tr!("demo-normal-specular")),
    ]
}

/// Pixels of a procedural demo texture (RGBA8, 64×64; the hair demo's 256×256).
pub fn local_texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    let alpha = match *id {
        TEX_PICK_ZERO => Some(0),
        TEX_PICK_FAINT => Some(1),
        TEX_PICK_HALF => Some(128),
        _ => None,
    };
    if let Some(alpha) = alpha {
        return Some(([196, 181, 253, alpha].repeat(64 * 64), 64, 64));
    }
    if *id == TEX_HAIR {
        let (empty, period) = hair_demo().map_or((60, 16), |h| (h.empty, h.period));
        return Some(hair_texture(empty, period));
    }
    if let Some(picture) = tags::picture_pixels(id) {
        return Some(picture);
    }
    if let Some(bake) = bake_texture(id) {
        return Some(bake);
    }
    const N: u32 = 64;
    let tau = std::f32::consts::TAU;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32, y as f32);
            let p: [u8; 4] = if *id == ACTION_MEDIA_TEX {
                [255, 255, 255, 255]
            } else if *id == TEX_GRADIENT {
                [196, 181, 253, (fx / (N - 1) as f32 * 255.0) as u8]
            } else if *id == TEX_HOLES {
                let (cx, cy) = ((fx % 32.0) - 16.0, (fy % 32.0) - 16.0);
                let hole = cx * cx + cy * cy < 100.0;
                [94, 234, 212, if hole { 0 } else { 255 }]
            } else if *id == TEX_BUMPS {
                // domes on a flat face, never dents, so that a reversed
                // relief shows: height h = max(sin(x) sin(y), 0)², normal =
                // normalize(-dh/dx, -dh/dup, 1) with +Y up the image (rows
                // run down), as SL normal maps
                let k = tau / 16.0;
                let s = ((fx * k).sin() * (fy * k).sin()).max(0.0);
                let dx = 2.0 * s * k * (fx * k).cos() * (fy * k).sin();
                let dy = 2.0 * s * k * (fx * k).sin() * (fy * k).cos();
                let n = Vec3::new(-dx * 3.0, dy * 3.0, 1.0).normalize();
                let e = |v: f32| ((v * 0.5 + 0.5) * 255.0) as u8;
                [e(n.x), e(n.y), e(n.z), 255]
            } else if *id == TEX_FRAMES {
                let (lx, ly) = (x % 16, y % 16);
                let frame = (y / 16) * 4 + x / 16;
                if lx < 1 || ly < 1 || lx > 14 || ly > 14 {
                    [24, 22, 30, 255]
                } else {
                    // hue frame / 16 at full saturation
                    let h = frame as f32 / 16.0 * 6.0;
                    let c = |o: f32| {
                        let k = (o + h) % 6.0;
                        let v = 1.0 - (k.min(4.0 - k).clamp(0.0, 1.0));
                        (v * 230.0 + 20.0) as u8
                    };
                    [c(5.0), c(3.0), c(1.0), 255]
                }
            } else if *id == TEX_SNAPSHOT {
                let (u, v) = (fx / (N - 1) as f32, fy / (N - 1) as f32);
                let hill = 0.58 - 0.07 * (u * 7.0).sin() - 0.04 * (u * 17.0 + 1.0).sin();
                let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
                    let t = t.clamp(0.0, 1.0);
                    [
                        (a[0] + (b[0] - a[0]) * t) as u8,
                        (a[1] + (b[1] - a[1]) * t) as u8,
                        (a[2] + (b[2] - a[2]) * t) as u8,
                        255,
                    ]
                };
                if v > 0.72 {
                    // water, with the aurora's reflection
                    let glint = (1.0 - ((v - 0.72) * 12.0 - (u * 9.0).sin() * 0.4).abs()).max(0.0) * 0.5;
                    mix([24.0, 34.0, 78.0], [94.0, 234.0, 212.0], glint * (0.5 + 0.5 * (u * 3.0).sin()))
                } else if v > hill {
                    mix([28.0, 30.0, 52.0], [18.0, 20.0, 36.0], (v - hill) * 6.0)
                } else {
                    let sky = mix([22.0, 18.0, 64.0], [150.0, 110.0, 200.0], v / 0.6);
                    let band = 0.22 + 0.06 * (u * 6.0).sin();
                    let a = (1.0 - (v - band).abs() / 0.07).max(0.0);
                    let s = [sky[0] as f32, sky[1] as f32, sky[2] as f32];
                    mix(s, [94.0, 234.0, 212.0], a * 0.85)
                }
            } else if *id == TEX_TILES {
                let (tx, ty) = (x / 32, y / 32);
                let (lx, ly) = (x % 32, y % 32);
                if lx < 2 || ly < 2 {
                    [40, 36, 48, 255]
                } else if tx == 0 && ty == 0 && (8..24).contains(&lx) && (8..24).contains(&ly) {
                    [124, 58, 237, 255]
                } else if (tx + ty) % 2 == 0 {
                    [214, 196, 160, 255]
                } else {
                    [170, 120, 84, 255]
                }
            } else if *id == TEX_PROJECTED {
                let (dx, dy) = (fx - 31.5, fy - 31.5);
                if dx * dx + dy * dy < 100.0 {
                    [255, 255, 255, 255]
                } else if dx.abs() < 1.5 || dy.abs() < 1.5 {
                    [0, 0, 0, 255]
                } else {
                    match (dx < 0.0, dy < 0.0) {
                        (true, true) => [230, 40, 40, 255],
                        (false, true) => [40, 200, 60, 255],
                        (true, false) => [50, 80, 230, 255],
                        (false, false) => [230, 210, 40, 255],
                    }
                }
            } else {
                return None;
            };
            px.extend_from_slice(&p);
        }
    }
    Some((px, N, N))
}

/// PBR material of AURORA_DEMO_PBR_OVERRIDE: the floor tiles with the
/// bumps as normal map.
const MAT_PBR_TILES: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0010);

/// PBR materials of the demo (normally fetched as assets).
pub fn pbr_materials() -> Vec<(Uuid, aurora_assets::PbrMaterial)> {
    vec![(
        MAT_PBR_TILES,
        aurora_assets::PbrMaterial {
            base_color_texture: Some(TEX_TILES),
            normal_texture: Some(TEX_BUMPS),
            metallic_factor: 0.0,
            roughness_factor: 0.6,
            ..Default::default()
        },
    )]
}

/// Legacy materials of the test panels (normally fetched from the region).
pub fn legacy_materials() -> Vec<(Uuid, crate::scene::legacy_mat::LegacyMaterial)> {
    use crate::scene::legacy_mat::{LegacyMaterial, alpha_mode};
    let base = LegacyMaterial {
        normal_map: Uuid::nil(),
        normal_st: [1.0, 1.0, 0.0, 0.0],
        normal_rot: 0.0,
        specular_map: Uuid::nil(),
        specular_st: [1.0, 1.0, 0.0, 0.0],
        specular_rot: 0.0,
        specular_color: [255, 255, 255, 255],
        specular_exp: 51,
        env_intensity: 0,
        diffuse_alpha_mode: alpha_mode::BLEND,
        alpha_cutoff: 0,
    };
    vec![
        (MAT_BLEND, base.clone()),
        (
            MAT_NONE,
            LegacyMaterial {
                diffuse_alpha_mode: alpha_mode::NONE,
                ..base.clone()
            },
        ),
        (
            MAT_MASK,
            LegacyMaterial {
                diffuse_alpha_mode: alpha_mode::MASK,
                alpha_cutoff: 128,
                ..base.clone()
            },
        ),
        (
            MAT_EMISSIVE,
            LegacyMaterial {
                diffuse_alpha_mode: alpha_mode::EMISSIVE,
                ..base.clone()
            },
        ),
        (
            MAT_BUMPY,
            LegacyMaterial {
                normal_map: TEX_BUMPS,
                normal_st: [2.0, 2.0, 0.0, 0.0],
                specular_exp: 220,
                env_intensity: 120,
                diffuse_alpha_mode: alpha_mode::NONE,
                ..base
            },
        ),
    ]
}

/// AURORA_DEMO_CAMERA="sit": a seat with a sit camera (llSetCameraEyeOffset
/// / llSetCameraAtOffset); it appears at frame 250, we sit on it at 300, it
/// turns like a vehicle from 400 and we stand up at 700.
pub fn sit_events(frame: u64) -> Vec<NetEvent> {
    const SEAT: u32 = 950;
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let (x, y) = (128.0, 121.0);
    let z = floor_at(x, y) + 0.2;
    let seat = |f: u64| {
        let turn = f.saturating_sub(400) as f32 * 0.01;
        prim(
            SEAT,
            Vec3::new(x, y, z),
            Quat::from_rotation_z(turn),
            Vec3::new(1.0, 1.0, 0.4),
            boxp,
            te([0.48, 0.3, 0.75, 1.0], 0, false, 0.0),
            ExtraParams::default(),
            aurora_i18n::tr!("demo-seat-seat-camera"),
        )
    };
    let me = |parent: u32, pos: Vec3| {
        let mut o = prim(
            9000,
            pos,
            Quat::from_rotation_z(0.3),
            Vec3::new(0.45, 0.6, 1.9),
            boxp,
            te([0.91, 0.93, 0.98, 1.0], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        o.pcode = LL_PCODE_LEGACY_AVATAR;
        o.full_id = DEMO_AGENT;
        o.parent_id = parent;
        o.name_values = "FirstName STRING RW SV Aurora\nLastName STRING RW SV Demo\nTitle STRING RW SV Aurora".into();
        o
    };
    let update = |objects: Vec<ObjectUpdate>| NetEvent::ObjectUpdates { handle: HANDLE, objects };
    match frame {
        250 => {
            let mut child = seat(frame);
            child.local_id = 951;
            child.full_id = Uuid::from_u128(951);
            child.parent_id = SEAT;
            child.position = Vec3::new(0.0, 0.0, -0.1);
            child.scale = Vec3::splat(0.1);
            vec![update(vec![seat(frame), child])]
        }
        300 => vec![
            NetEvent::SitResponse {
                object: seat(frame).full_id,
                camera_eye: Vec3::new(-3.0, 1.5, 1.5),
                camera_at: Vec3::new(0.0, 0.0, 0.8),
                force_mouselook: false,
            },
            update(vec![me(SEAT, Vec3::new(0.0, 0.0, 0.4))]),
            NetEvent::AvatarAnimations {
                avatar: DEMO_AGENT,
                anims: vec![(SEAT_ANIM, 1), (IDLE_ANIM, 1)],
                sources: vec![Uuid::from_u128(951)],
            },
        ],
        f if f > 400 && f < 700 && f.is_multiple_of(5) => vec![update(vec![seat(f)])],
        700 => vec![update(vec![me(0, Vec3::new(x + 1.2, y, floor_at(x + 1.2, y) + 0.84))])],
        _ => Vec::new(),
    }
}

pub const SEAT_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0103);

/// A seat's scripted animation (legs bent, pelvis position explicitly reset).
/// The simulator leaves it signaled at frame 700 until the viewer requests a
/// stop, so AURORA_DEMO_CAMERA=sit exercises the complete source/stop path.
pub fn seat_animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, PosKey, RotKey};
    let mut a = idle_animation();
    a.base_priority = 4;
    for (name, rotation) in [
        ("mHipLeft", Quat::from_rotation_y(-1.45)),
        ("mHipRight", Quat::from_rotation_y(-1.45)),
        ("mKneeLeft", Quat::from_rotation_y(1.55)),
        ("mKneeRight", Quat::from_rotation_y(1.55)),
        ("mPelvis", Quat::IDENTITY),
    ] {
        a.joints.retain(|joint| joint.joint_name != name);
        a.joints.push(JointMotion {
            joint_name: name.into(),
            priority: -1,
            rot_keys: vec![RotKey { time: 0.0, rotation }],
            pos_keys: if name == "mPelvis" {
                vec![PosKey {
                    time: 0.0,
                    position: Vec3::ZERO,
                }]
            } else {
                Vec::new()
            },
        });
    }
    a
}

/// Day cycle with a single classic sky frame (see AURORA_DEMO_SKY).
fn demo_sky(sun: Vec3, gamma: f64, haze_distance: Option<f64>, ambient: Option<f64>) -> Llsd {
    use aurora_llsd::llsd_map;
    let q = Quat::from_rotation_arc(Vec3::X, sun);
    let arr = |v: &[f64]| Llsd::Array(v.iter().map(|x| Llsd::Real(*x)).collect());
    let mut sky = llsd_map! {
        "type" => "sky",
        "sun_rotation" => arr(&[q.x as f64, q.y as f64, q.z as f64, q.w as f64]),
        "sunlight_color" => arr(&[2.2, 2.34, 2.7]),
        "gamma" => gamma,
    };
    let mut haze = aurora_llsd::Map::new();
    if let Some(d) = haze_distance {
        haze.insert("distance_multiplier".into(), Llsd::Real(d));
    }
    if let Some(a) = ambient {
        haze.insert("ambient".into(), arr(&[a, a, a]));
    }
    if let (false, Llsd::Map(m)) = (haze.is_empty(), &mut sky) {
        m.insert("legacy_haze".into(), Llsd::Map(haze));
    }
    let mut frames = aurora_llsd::Map::new();
    frames.insert("sky".into(), sky);
    // a valid day needs a water track too (Firestorm recordEnvironment)
    frames.insert("water".into(), llsd_map! { "type" => "water" });
    let key = |name: &str| Llsd::Array(vec![llsd_map! { "key_keyframe" => 0.0, "key_name" => name }]);
    llsd_map! {
        "day_length" => 14400,
        "day_offset" => 0,
        "day_cycle" => llsd_map! {
            "frames" => Llsd::Map(frames),
            "tracks" => Llsd::Array(vec![key("water"), key("sky")]),
        },
    }
}

#[cfg(test)]
mod particle_tests {
    use super::*;

    #[test]
    fn agent_jitter_stays_within_its_size_and_changes_every_twelve_frames() {
        let mut distinct = std::collections::HashSet::new();
        for frame in 0..1200 {
            let o = agent_jitter_offset(frame, 0.6);
            assert!(o.abs().max_element() <= 0.0006 + 1e-9, "{o:?}");
            assert_eq!(o, agent_jitter_offset(frame / 12 * 12, 0.6));
            distinct.insert(o.to_array().map(f32::to_bits));
        }
        assert!(distinct.len() > 90);
    }

    #[test]
    fn motorcycle_script_survives_compressed_update_and_simulation() {
        use aurora_net::objects::ParticleUpdate;
        use aurora_prim::particles::{self as ps, ParticleSource, SourceContext};
        for (wind, legacy) in [(true, false), (false, false), (true, true)] {
            let ParticleUpdate::Set(bytes) = compressed_scripted_particles(wind, legacy).expect("compressed update") else {
                panic!("scripted particle source missing");
            };
            let data = ps::parse(&bytes).expect("scripted particle block");
            assert_eq!(data.part.max_age, 2.0);
            assert_eq!(data.burst_part_count, 3);
            assert_eq!(data.part_accel.y, if wind { -80.0 } else { 0.0 });
            assert_eq!(data.part.start_scale.y, if wind { 4.0 } else { 0.5 });
            assert_eq!(data.part.start_glow > 0.0, !legacy);
            let mut source = ParticleSource::new(data, 1);
            let ctx = SourceContext {
                pos: Vec3::new(10.0, 10.0, 10.0),
                ..Default::default()
            };
            let mut budget = 100;
            for _ in 0..10 {
                source.update(0.1, &ctx, &mut budget);
            }
            let parts = source.particles();
            assert!(!parts.is_empty());
            assert!(parts.iter().all(|p| p.flags & ps::LL_PART_FOLLOW_VELOCITY_MASK != 0));
            if wind {
                assert!(parts.iter().any(|p| p.vel.y < -50.0 && p.size.y > 2.5));
            } else {
                assert!(parts.iter().any(|p| p.vel.z > 0.1));
            }
        }
    }
}
