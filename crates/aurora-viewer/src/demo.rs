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

#[path = "demo_anti_spam.rs"]
pub mod anti_spam;
#[path = "demo_beacons.rs"]
pub mod beacons;
#[path = "demo_parcel_visibility.rs"]
pub mod parcel_visibility;
#[path = "demo_pointing.rs"]
pub mod pointing;
#[path = "demo_poser.rs"]
pub mod poser;

#[path = "demo_texture_library.rs"]
pub mod texture_library;

#[path = "demo_water.rs"]
pub mod water;

const HANDLE: u64 = (256000u64 << 32) | 256000;

/// Help › About Aurora Viewer on one of its tabs (AURORA_DEMO_ABOUT).
#[path = "demo_about.rs"]
pub mod about;
/// AURORA_DEMO_ARRIVAL: an arrival in a busy region, every asset downloaded.
#[path = "demo_arrival.rs"]
pub mod arrival;
/// The local asset server of its `net=` option.
#[path = "demo_asset_server.rs"]
pub mod asset_server;
/// Taskbar flashes and system notifications in the background
/// (AURORA_DEMO_ATTENTION).
#[path = "demo_attention.rs"]
pub mod attention;
#[path = "demo_auto_hover.rs"]
pub mod auto_hover;
/// Blended panes standing in the sun, for their shadows
/// (AURORA_DEMO_BLEND_SHADOWS).
#[path = "demo_blend_shadows.rs"]
pub mod blend_shadows;
/// A box of bake-on-mesh placeholders on the ground (AURORA_DEMO_BOM_REZZED).
#[path = "demo_bom_rezzed.rs"]
pub mod bom_rezzed;
/// Voice calls with a simulated voice server (AURORA_DEMO_CALL).
#[path = "demo_calls.rs"]
pub mod calls;
/// EEP environment answers (AURORA_DEMO_EEP_PARCEL).
#[path = "demo_chatlog.rs"]
pub mod chatlog;
/// The classifieds of the profile scenarios, ours kept for the run.
#[path = "demo_classifieds.rs"]
pub mod classifieds;
/// Bumps, pushes and hits, and their window (AURORA_DEMO_COLLISIONS).
#[path = "demo_collisions.rs"]
pub mod collisions;
/// Conference invitations from a stranger and a friend in turn
/// (AURORA_DEMO_CONFERENCE).
#[path = "demo_conference.rs"]
pub mod conference;
/// The Conversations button flashing (AURORA_DEMO_CONV_BUTTON).
#[path = "demo_conv_button.rs"]
pub mod conv_button;
/// A deformed avatar to reset (AURORA_DEMO_DEFORM).
#[path = "demo_deform.rs"]
pub mod deform;
/// A Marketplace delivery's notification (AURORA_DEMO_DELIVERY).
#[path = "demo_delivery.rs"]
pub mod delivery;
#[path = "demo_eep.rs"]
pub mod eep;
/// A facial expression and a hand pose (AURORA_DEMO_EMOTE).
#[path = "demo_emote.rs"]
pub mod emote;
/// Environment selector library and script (AURORA_DEMO_ENV_SELECT).
#[path = "demo_env.rs"]
pub mod env;
/// An experience's teleport door and its permission question
/// (AURORA_DEMO_EXPERIENCE).
#[path = "demo_experience.rs"]
pub mod experience;
/// Meshes filling the geometry arena until it outgrows its buffers
/// (AURORA_DEMO_GEOMETRY).
/// Friendship offers in the middle of the screen (AURORA_DEMO_FRIEND_PROMPT).
#[path = "demo_friend_prompt.rs"]
pub mod friend_prompt;
#[path = "demo_geometry.rs"]
pub mod geometry;
/// Names and chat in the characters residents use (AURORA_DEMO_GLYPHS).
#[path = "demo_glyphs.rs"]
pub mod glyphs;
/// The grid status feed and its button (AURORA_DEMO_GRID_STATUS).
#[path = "demo_grid_status.rs"]
pub mod grid_status;
#[path = "demo_grounding.rs"]
pub mod grounding;
/// The demo group server (AURORA_DEMO_GROUP_PROFILE).
#[path = "demo_group.rs"]
pub mod group;
#[path = "demo_hand_ik.rs"]
pub mod hand_ik;
/// Set Home to Here and Privacy's account settings (AURORA_DEMO_HOME,
/// AURORA_DEMO_USERINFO).
#[path = "demo_home.rs"]
pub mod home;
/// The world's hover tips in one of their forms (AURORA_DEMO_HOVER_TIPS).
#[path = "demo_hover_tips.rs"]
pub mod hover_tips;
#[path = "demo_hud.rs"]
pub mod hud;
/// Keyword alerts on lines of every kind (AURORA_DEMO_KEYWORDS).
#[path = "demo_keywords.rs"]
pub mod keywords;
/// About Land answers (AURORA_DEMO_LAND).
#[path = "demo_land.rs"]
pub mod land;
#[path = "demo_link.rs"]
pub mod link;
#[path = "demo_missing_texture.rs"]
pub mod missing_texture;
/// The notification center's mockup scenario, in either style
/// (AURORA_DEMO_NOTIFS).
#[path = "demo_notifs.rs"]
pub mod notifs;
/// What objects say in the nearby chat among residents, as compact rows or
/// not (AURORA_DEMO_OBJECT_CHAT).
#[path = "demo_object_chat.rs"]
pub mod object_chat;
/// Objects' messages from near and far, filtered (AURORA_DEMO_OBJECT_IM).
#[path = "demo_object_im.rs"]
pub mod object_im;
/// The object menu scenario's objects (AURORA_DEMO_OBJECT_MENU).
#[path = "demo_object_menu.rs"]
pub mod object_menu;
/// AURORA_DEMO_OUTFIT_RESTORE: the login restore of the outfit against a
/// region that puts part of it back by itself.
#[path = "demo_outfit_restore.rs"]
pub mod outfit_restore;
/// Place details of place links (AURORA_DEMO_PLACE).
#[path = "demo_people.rs"]
pub mod people;
/// Avatar body physics: a physics layer and balls on the body (AURORA_DEMO_PHYSICS).
#[path = "demo_physics.rs"]
pub mod physics;
#[path = "demo_place.rs"]
pub mod place;
/// Linden trees and grass, near, far or as a forest (AURORA_DEMO_PLANTS).
#[path = "demo_plants.rs"]
pub mod plants;
/// The friends' online / offline toasts at one of their places
/// (AURORA_DEMO_PRESENCE).
#[path = "demo_presence.rs"]
pub mod presence;
/// Region / Estate answers (AURORA_DEMO_REGION).
#[path = "demo_region.rs"]
pub mod region;
/// The rez effect of objects and avatars appearing (AURORA_DEMO_REZ_EFFECT).
#[path = "demo_rez.rs"]
pub mod rez;
/// A rigged mesh rezzed on a cube and the same one worn
/// (AURORA_DEMO_RIGGED_PROP).
#[path = "demo_rigged_prop.rs"]
pub mod rigged_prop;
#[path = "demo_sculpt.rs"]
pub mod sculpt;
/// Directory answers of the search window (AURORA_DEMO_SEARCH).
#[path = "demo_search.rs"]
pub mod search;
/// A seat deleted under the resident sitting on it (AURORA_DEMO_SEAT_KILL).
#[path = "demo_seat_kill.rs"]
pub mod seat_kill;
/// Freeze, region console and voice moderation (AURORA_DEMO_FREEZE,
/// AURORA_DEMO_SIM_CONSOLE, AURORA_DEMO_VOICE_MODERATION).
#[path = "demo_sim_extras.rs"]
pub mod sim_extras;
/// A rainbow or a 22° halo in the region's sky (AURORA_DEMO_SKY_OPTICS).
#[path = "demo_sky.rs"]
pub mod sky;
/// A resident's "Invite to group", "Add to contact set" and "Block
/// particles" (AURORA_DEMO_SOCIAL_MENU).
#[path = "demo_social_menu.rs"]
pub mod social_menu;
#[path = "demo_sound_parcels.rs"]
pub mod sound_parcels;
/// The Aurora team's marks on demo residents (AURORA_DEMO_STAFF).
#[path = "demo_staff.rs"]
pub mod staff;
/// Streaming hitch test: objects and textures arriving in waves
/// (AURORA_DEMO_STREAM).
#[path = "demo_stream.rs"]
pub mod stream;
/// The parcel radio's titles and the stream title window
/// (AURORA_DEMO_STREAM_TITLE).
#[path = "demo_stream_title.rs"]
pub mod stream_title;
/// Name plates in every state and at each distance (AURORA_DEMO_TAGS).
#[path = "demo_tags.rs"]
pub mod tags;
/// A low-poly ball with a baked normal map (AURORA_DEMO_BUMP=sphere).
#[path = "demo_tangents.rs"]
pub mod tangents;
/// "Refresh textures" on a panel whose disk cache file is stale
/// (AURORA_DEMO_TEXREFRESH).
#[path = "demo_tex_refresh.rs"]
pub mod tex_refresh;
/// Toasts of friends and messages, and the list under the bell
/// (AURORA_DEMO_TOASTS).
#[path = "demo_toasts.rs"]
pub mod toasts;
/// Translated private conversations (AURORA_DEMO_TRANSLATE).
#[path = "demo_translate.rs"]
pub mod translate;
/// Fast camera turns (AURORA_DEMO_TURN).
#[path = "demo_turn.rs"]
pub mod turn;
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
    static TERRAIN_SLOPES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let slopes = *TERRAIN_SLOPES.get_or_init(|| std::env::var("AURORA_DEMO_REGION").is_ok_and(|v| v.split(':').any(|p| p == "pbr-slopes")));
    let relief = if slopes {
        7.0 * (-((x - 123.0).powi(2) / 9.0 + (y - 49.0).powi(2) / 25.0)).exp()
            - 2.0 * (-((x - 128.0).powi(2) + (y - 46.0).powi(2)) / 8.0).exp()
    } else {
        0.0
    };
    21.5 + shore + hill + dune + relief + altitude()
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

/// The hat Violet Wolf wears (AURORA_DEMO_DERENDER=hat): `prim`'s id of
/// local id 9100.
pub fn wolf_hat() -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | 9100)
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
        // Ordinary demo prims have the same editable ownership advertised by
        // DemoSim's properties. Special interaction fixtures override these.
        update_flags: crate::ui::context::flags::OBJECT_YOU_OWNER
            | crate::ui::context::flags::OBJECT_ANY_OWNER
            | crate::ui::context::flags::OBJECT_MODIFY
            | crate::ui::context::flags::OBJECT_MOVE
            | crate::ui::context::flags::OBJECT_COPY,
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

/// Leash script (legacy block): a ribbon chain dropped at the collar that
/// steers towards `target` and sags under gravity, two links per burst. Its
/// y scale is long because ribbons ignore it (#1049).
fn leash_particles(target: Uuid) -> Vec<u8> {
    use aurora_prim::particles::*;
    let mut out = fountain_particles();
    out[0..4].copy_from_slice(&0xA0E0_0002u32.to_le_bytes()); // crc
    out[4..8].copy_from_slice(&0u32.to_le_bytes()); // source flags
    out[8] = LL_PART_SRC_PATTERN_DROP;
    out[15..17].copy_from_slice(&0u16.to_le_bytes()); // burst rate (clamped)
    out[23] = 2; // particles per burst
    out[34..36].copy_from_slice(&(((-1.0f32 + 256.0) * 128.0) as u16).to_le_bytes()); // gravity
    out[52..68].copy_from_slice(target.as_bytes());
    let part = PS_SYS_DATA_BLOCK_SIZE;
    let flags = LL_PART_RIBBON_MASK | LL_PART_FOLLOW_SRC_MASK | LL_PART_TARGET_POS_MASK | LL_PART_EMISSIVE_MASK;
    out[part..part + 4].copy_from_slice(&flags.to_le_bytes());
    out[part + 4..part + 6].copy_from_slice(&(((3.5 * 256.0) as u16).to_le_bytes())); // max age
    out[part + 6..part + 14].copy_from_slice(&[226, 232, 240, 255, 226, 232, 240, 255]);
    out[part + 14..part + 18].copy_from_slice(&[(0.07 * 32.0) as u8, (2.5 * 32.0) as u8, (0.07 * 32.0) as u8, (2.5 * 32.0) as u8]);
    out
}

/// Two otherwise identical smoke emitters, with only LL_PART_WIND_MASK
/// differing. Keep the ordinary compressed-update path and default texture.
fn region_wind_particles(affected: bool) -> Vec<u8> {
    use aurora_prim::particles::*;
    let mut out = scripted_particles(false, true);
    let flags = LL_PART_EMISSIVE_MASK | LL_PART_INTERP_COLOR_MASK | LL_PART_INTERP_SCALE_MASK;
    let part = PS_SYS_DATA_BLOCK_SIZE;
    out[part..part + 4].copy_from_slice(&(flags | if affected { LL_PART_WIND_MASK } else { 0 }).to_le_bytes());
    out[part + 4..part + 6].copy_from_slice(&(6u16 * 256).to_le_bytes());
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
    compressed_particle_block(&scripted_particles(wind, legacy), legacy)
}

fn compressed_particle_block(block: &[u8], legacy: bool) -> Option<aurora_net::objects::ParticleUpdate> {
    let mut data = vec![0; 84];
    data[20] = LL_PCODE_VOLUME;
    data[64..68].copy_from_slice(&(if legacy { 0x8u32 } else { 0x400u32 }).to_le_bytes());
    if legacy {
        data.extend_from_slice(block);
    }
    data.push(0); // extra params
    data.extend_from_slice(&[0; 23]); // shape (demo object supplies its geometry)
    data.extend_from_slice(&0u32.to_le_bytes()); // texture entry
    if !legacy {
        data.extend_from_slice(block);
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

/// Own chest and facial attachments, including a linked skull child, for
/// the first-person visibility scenarios. No grid assets are requested.
pub fn first_person_visibility_events() -> Vec<NetEvent> {
    let volume = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut objects = Vec::new();
    for (local, point, color) in [
        (29_001, 2u8, [0.9, 0.1, 0.1, 1.0]),
        (29_002, 47, [0.9, 0.2, 0.1, 1.0]),
        (29_003, 1, [0.9, 0.8, 0.1, 1.0]),
    ] {
        let mut object = prim(
            local,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::splat(0.22),
            volume,
            te(color, 0, true, 0.0),
            ExtraParams::default(),
            "",
        );
        object.parent_id = 9000;
        object.state = point.rotate_right(4);
        objects.push(object);
    }
    let mut child = objects[0].clone();
    child.local_id = 29_004;
    child.full_id = action_id(child.local_id);
    child.parent_id = 29_001;
    child.position = Vec3::new(0.3, 0.0, 0.0);
    child.state = 0;
    objects.push(child);
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// Head turn and nod for testing eye-position following without camera rotation.
pub fn first_person_eye_animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, RotKey};
    let mut animation = idle_animation();
    animation.base_priority = 6;
    animation.joints.clear();
    animation.joints.push(JointMotion {
        joint_name: "mHead".into(),
        priority: -1,
        rot_keys: [(0.0, 0.0), (1.0, 0.9), (2.0, 0.0), (3.0, -0.9), (4.0, 0.0)]
            .into_iter()
            .map(|(time, angle)| RotKey {
                time,
                rotation: Quat::from_rotation_z(angle) * Quat::from_rotation_y(angle * 0.6),
            })
            .collect(),
        pos_keys: Vec::new(),
    });
    animation
}

/// Ground-sit fixture: the demo asset fallback is an idle, so supply a
/// folded-leg pose with a lowered pelvis without requesting a grid asset.
pub fn first_person_ground_animation() -> aurora_assets::Animation {
    let mut animation = seat_animation();
    animation.ease_in = 0.0;
    if let Some(pelvis) = animation.joints.iter_mut().find(|joint| joint.joint_name == "mPelvis") {
        for key in &mut pelvis.pos_keys {
            key.position.z = -0.65;
        }
    }
    animation
}

/// The Photo Album (FT_SNAPSHOT_CATEGORY), empty until a photo is saved.
const PHOTO_ALBUM_FOLDER: u128 = 31;

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
    skel.push(folder(PHOTO_ALBUM_FOLDER, 1, aurora_i18n::tr!("demo-photo-album"), 15));
    skel.push(folder(
        place::SHOPS_FOLDER,
        place::LANDMARKS_FOLDER,
        aurora_i18n::tr!("common-shopping"),
        -1,
    ));
    if place::many_landmarks() {
        skel.push(folder(
            place::CLUBS_FOLDER,
            place::LANDMARKS_FOLDER,
            aurora_i18n::tr!("demo-map-clubs"),
            -1,
        ));
        skel.push(folder(
            place::BEACHES_FOLDER,
            place::LANDMARKS_FOLDER,
            aurora_i18n::tr!("demo-map-beaches"),
            -1,
        ));
        skel.push(folder(
            place::WALKS_FOLDER,
            place::LANDMARKS_FOLDER,
            aurora_i18n::tr!("demo-map-walks"),
            -1,
        ));
        skel.push(folder(
            place::MARKETS_FOLDER,
            place::LANDMARKS_FOLDER,
            aurora_i18n::tr!("demo-map-markets"),
            -1,
        ));
    }
    if env::enabled() {
        skel.push(env::settings_folder(u(1)));
    }
    // AURORA_DEMO_PEOPLE: its friends and their rights
    let people = people::friends();
    let mut buddies: Vec<Llsd> = (0..4)
        .map(|i| u(100 + i))
        .filter(|id| people.iter().all(|(f, _, _)| f != id) && !people::hides_buddy(id))
        .map(|id| {
            let i = (id.as_u128() & 0xFF) as usize - 100;
            llsd_map! {"buddy_id" => id, "buddy_rights_given" => [1, 3, 7, 0][i], "buddy_rights_has" => if i.is_multiple_of(2) { 3 } else { 1 }}
        })
        .collect();
    buddies.extend(
        people
            .into_iter()
            .map(|(id, given, has)| llsd_map! {"buddy_id" => id, "buddy_rights_given" => given, "buddy_rights_has" => has}),
    );
    let mut raw = llsd_map! {
        "inventory-root" => Llsd::Array(vec![llsd_map!{"folder_id" => u(1)}]),
        // an adult account that wants every rating (the search window's boxes);
        // AURORA_DEMO_LURE=exceeded: it wants General and Moderate only,
        // =blocked: it is not adult
        "agent_access_max" => demo_access().0,
        "agent_region_access" => demo_access().1,
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
    // AURORA_DEMO_PEOPLE's groups tab: "3 groups out of 42"
    if let Some(limit) = people::group_limit()
        && let Llsd::Map(m) = &mut raw
    {
        m.insert("max-agent-groups".to_owned(), Llsd::from(limit));
    }
    // AURORA_DEMO_GROUP_CREATE: the membership's fee and group limit
    for (key, value) in group::login_benefits().unwrap_or_default() {
        raw.insert(key, value);
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

/// AURORA_DEMO_FLASH=1: Tess writes to us every 8 seconds from the 10th
/// (`ui/chat/flash.rs`): with another conversation shown, her entry
/// flashes. The message due at `secs` seconds of the session, once.
pub fn flash_message(agent: Uuid, secs: f64) -> Option<NetEvent> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SENT: AtomicU64 = AtomicU64::new(0);
    if std::env::var("AURORA_DEMO_FLASH").is_err() || secs < 10.0 {
        return None;
    }
    let due = ((secs - 10.0) / 8.0) as u64 + 1;
    if SENT.swap(due, Ordering::Relaxed) == due {
        return None;
    }
    Some(NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: DEMO_TESS,
        from_name: "Tess Touch".into(),
        to_agent_id: agent,
        dialog: 0,
        session_id: Uuid::from_u128(agent.as_u128() ^ DEMO_TESS.as_u128()),
        message: aurora_i18n::tr!("demo-flash-message", n = due as i64),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    }))
}

/// AURORA_DEMO_SNOOZE=<seconds>[,auto][,ask]: a group chat's snooze
/// (`world/groups.rs`). Tess writes in the Aurora Builders chat every 6
/// seconds from the 10th in world; a snooze lasts `<seconds>` instead of
/// the setting's minutes, to watch it end; `auto` snoozes that chat by
/// itself at the 14th second; `ask` asks the duration first, as the setting
/// would, in minutes (`<seconds>` rounded up).
#[derive(Clone, Copy)]
struct SnoozeScene {
    seconds: u32,
    auto: bool,
    ask: bool,
}

fn snooze_scenario() -> Option<SnoozeScene> {
    static SCENE: std::sync::OnceLock<Option<SnoozeScene>> = std::sync::OnceLock::new();
    *SCENE.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_SNOOZE").ok()?;
        let mut parts = v.split(',').map(str::trim);
        let seconds = parts.next()?.parse().ok()?;
        let flags: Vec<&str> = parts.collect();
        Some(SnoozeScene {
            seconds,
            auto: flags.contains(&"auto"),
            ask: flags.contains(&"ask"),
        })
    })
}

/// The snooze's duration of `AURORA_DEMO_SNOOZE`.
pub fn snooze_seconds() -> Option<u32> {
    snooze_scenario().map(|s| s.seconds)
}

/// `AURORA_DEMO_SNOOZE=…,ask`: the duration is asked at each snooze.
pub fn snooze_ask() -> bool {
    snooze_scenario().is_some_and(|s| s.ask)
}

/// Seconds since the scenario's first look, made in world.
fn snooze_clock() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// `AURORA_DEMO_SNOOZE=…,auto`: true once, when the demo snoozes the group
/// chat itself. Asked every frame in world.
pub fn snooze_now() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    snooze_scenario().is_some_and(|s| s.auto) && snooze_clock() >= 14.0 && !DONE.swap(true, Ordering::Relaxed)
}

/// `AURORA_DEMO_SNOOZE`: Tess's message in the group chat now due, once.
/// Asked every frame in world.
pub fn snooze_message(agent: Uuid) -> Option<NetEvent> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SENT: AtomicU64 = AtomicU64::new(0);
    snooze_scenario()?;
    let secs = snooze_clock();
    if secs < 10.0 {
        return None;
    }
    let due = ((secs - 10.0) / 6.0) as u64 + 1;
    if SENT.swap(due, Ordering::Relaxed) == due {
        return None;
    }
    Some(NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: DEMO_TESS,
        from_name: "Tess Touch".into(),
        to_agent_id: agent,
        dialog: 17,
        session_id: DEMO_GROUP1,
        message: aurora_i18n::tr!("demo-snooze-message", n = due as i64),
        offline: false,
        from_group: false,
        binary_bucket: b"Aurora Builders\0".to_vec(),
        timestamp: 0,
    }))
}

/// AURORA_DEMO_CONV_PREVIEWS=net: the previews of the demo's links come from
/// the real sites; otherwise the demo makes them (`link_preview::demo`).
pub fn previews_offline() -> bool {
    static NET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    !*NET.get_or_init(|| std::env::var("AURORA_DEMO_CONV_PREVIEWS").is_ok_and(|v| v == "net"))
}

/// AURORA_DEMO_CONV_PREVIEWS=services: the made-up Nextcloud of the demo's
/// links is one of the picture sites (#1063), without being added to the
/// user's settings.
pub fn picture_sites() -> &'static [&'static str] {
    let on =
        std::env::var_os("AURORA_DEMO").is_some() && std::env::var("AURORA_DEMO_CONV_PREVIEWS").is_ok_and(|v| v.starts_with("services"));
    if on { &["cloud.example.org"] } else { &[] }
}

/// AURORA_DEMO_GIFS: the GIF picker of the Conversations field (#932).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gifs {
    /// `1` / `open`: open on GIPHY's trending, made here.
    Open,
    /// `klipy`: open on KLIPY's tab, a category searched.
    Klipy,
    /// `sent`: the first GIF sent, its preview under the message.
    Sent,
    /// `menu`: the GIF button's right-click menu open.
    Menu,
    /// `nokey`: the real settings, no key: no button.
    NoKey,
    /// `net`: open, the real services with the keys of Preferences.
    Net,
}

impl Gifs {
    /// The picker answers with the demo's GIFs, as if both services had a
    /// key (the button shows).
    pub fn made_here(self) -> bool {
        !matches!(self, Gifs::NoKey | Gifs::Net)
    }
}

pub fn gifs_scenario() -> Option<Gifs> {
    static MODE: std::sync::OnceLock<Option<Gifs>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_DEMO_GIFS").ok()?.trim() {
        "1" | "open" => Some(Gifs::Open),
        "klipy" => Some(Gifs::Klipy),
        "sent" => Some(Gifs::Sent),
        "menu" => Some(Gifs::Menu),
        "nokey" => Some(Gifs::NoKey),
        "net" => Some(Gifs::Net),
        other => {
            log::warn!("AURORA_DEMO_GIFS={other}: unknown");
            None
        }
    })
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
    if let Some(scene) = aurora_scenario() {
        return Some(scene.view);
    }
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

/// `AURORA_DEMO_CONV_POPOUT=1|option|nearby|nearby-out` (with `AURORA_DEMO_CONV=aurora…` or
/// `bubbles…`): conversations in their own windows (#717).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popout {
    /// The friend's IM and the Aurora Builders group popped out by hand.
    Hand,
    /// The option on, and the group opened as from a notification.
    Option,
    /// Nearby chat alone, before any private or group conversation opens.
    Nearby,
    /// The same nearby chat detached into its own window.
    NearbyOut,
}

pub fn popout_scenario() -> Option<Popout> {
    match std::env::var("AURORA_DEMO_CONV_POPOUT").ok()?.trim() {
        "option" => Some(Popout::Option),
        "nearby" => Some(Popout::Nearby),
        "nearby-out" => Some(Popout::NearbyOut),
        "" | "0" => None,
        _ => Some(Popout::Hand),
    }
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

/// `AURORA_DEMO_CHAT_LINES=1|legacy`: nearby chat said at the arrival, to
/// see the lines above the chat bar (`ui/chat/console.rs`): two lines of
/// one resident, an emote, an object's line, a long line with a link, a
/// shout. `legacy` shows them as the Legacy Viewer's lines for the session
/// (nothing is saved). Some(true) for `legacy`.
pub fn chat_lines_scenario() -> Option<bool> {
    match std::env::var("AURORA_DEMO_CHAT_LINES").ok()?.as_str() {
        "legacy" => Some(true),
        "" | "0" => None,
        _ => Some(false),
    }
}

pub fn chat_lines_events() -> Vec<NetEvent> {
    let object = Uuid::from_u128(0xC4A7_0000_0000_0000_0000_0000_0000_0001);
    let nova = aurora_i18n::tr!("demo-nova-example");
    let orion = aurora_i18n::tr!("demo-orion-example");
    let say = |from: Uuid, name: &str, text: &str, object: bool, chat_type| {
        NetEvent::Chat(aurora_net::ChatMessage {
            from_name: name.into(),
            source_id: from,
            owner_id: Uuid::nil(),
            source_type: if object {
                aurora_net::ChatSourceType::Object
            } else {
                aurora_net::ChatSourceType::Agent
            },
            chat_type,
            position: Vec3::ZERO,
            message: text.into(),
        })
    };
    let normal = aurora_net::ChatType::Normal;
    vec![
        say(DEMO_NOVA, nova, aurora_i18n::tr!("demo-bubbles-local-1"), false, normal),
        say(DEMO_NOVA, nova, aurora_i18n::tr!("demo-bubbles-local-2"), false, normal),
        say(DEMO_FRIEND, orion, aurora_i18n::tr!("demo-chat-lines-emote"), false, normal),
        say(
            object,
            aurora_i18n::tr!("common-demo-hud"),
            aurora_i18n::tr!("demo-bubbles-local-3"),
            true,
            normal,
        ),
        say(DEMO_FRIEND, orion, aurora_i18n::tr!("demo-chat-lines-long"), false, normal),
        say(
            DEMO_NOVA,
            nova,
            aurora_i18n::tr!("demo-chat-lines-shout"),
            false,
            aurora_net::ChatType::Shout,
        ),
    ]
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
            // AURORA_DEMO_CONV_EMOJI=1: messages of emoji alone, drawn large
            // (#483), among the others
            let emoji_lines = std::env::var("AURORA_DEMO_CONV_EMOJI").is_ok().then(|| {
                [
                    line(ago(10), name, friend, ChatKind::Im, "🎉🎶🐺"),
                    line(ago(3), &me, Uuid::nil(), ChatKind::OWN, "👍"),
                ]
            });
            let mut lines = vec![
                line(
                    ago(26 * 60),
                    &me,
                    Uuid::nil(),
                    ChatKind::OWN,
                    aurora_i18n::tr!("demo-bubbles-concert"),
                ),
                line(ago(26 * 60 - 3), name, friend, ChatKind::Im, aurora_i18n::tr!("demo-bubbles-cant")),
                line(ago(12), name, friend, ChatKind::Im, aurora_i18n::tr!("demo-bubbles-sorry")),
                line(ago(11), name, friend, ChatKind::Im, aurora_i18n::tr!("demo-bubbles-join")),
                line(ago(2), &me, Uuid::nil(), ChatKind::OWN, aurora_i18n::tr!("demo-bubbles-coming")),
                line(ago(1), &me, Uuid::nil(), ChatKind::OWN, aurora_i18n::tr!("demo-bubbles-send-tp")),
            ];
            if let Some(extra) = emoji_lines {
                lines.splice(4..4, extra);
            }
            // AURORA_DEMO_CONV_PREVIEWS=1|states|videos|clips: links with their
            // cards (#538, #739, #848), made here without network
            // (`link_preview::demo`)
            let preview_lines = std::env::var("AURORA_DEMO_CONV_PREVIEWS").ok().map(|mode| {
                let with = |key: &str, url: &str| format!("{key} {url}");
                if mode == "net" {
                    // real sites: a site to click on, a YouTube video
                    vec![
                        line(
                            ago(9),
                            name,
                            friend,
                            ChatKind::Im,
                            "https://upload.wikimedia.org/wikipedia/commons/2/2c/Rotating_earth_%28large%29.gif",
                        ),
                        line(
                            ago(8),
                            name,
                            friend,
                            ChatKind::Im,
                            "https://upload.wikimedia.org/wikipedia/commons/4/47/PNG_transparency_demonstration_1.png",
                        ),
                        line(
                            ago(7),
                            &me,
                            Uuid::nil(),
                            ChatKind::OWN,
                            "https://www.youtube.com/watch?v=jNQXAC9IVRw&t=5s",
                        ),
                    ]
                } else if let Some(only) = mode.strip_prefix("links") {
                    let only = only.trim_start_matches(':');
                    [
                        ("gif", "https://i.gyazo.com/demo-dance.gif"),
                        ("image", "https://i.imgur.com/demo-outfit.jpg"),
                        ("video", "https://youtu.be/jNQXAC9IVRw"),
                    ]
                    .into_iter()
                    .filter(|(kind, _)| only.is_empty() || only == *kind)
                    .enumerate()
                    .map(|(i, (kind, url))| {
                        let text = if kind == "image" {
                            with(aurora_i18n::tr!("demo-previews-outfit"), url)
                        } else {
                            url.to_owned()
                        };
                        line(ago(8 - i as u64), name, friend, ChatKind::Im, &text)
                    })
                    .collect()
                } else if mode == "videos" {
                    // a 4:3 clip and an upright short (#848)
                    vec![
                        line(
                            ago(8),
                            name,
                            friend,
                            ChatKind::Im,
                            &with(aurora_i18n::tr!("demo-previews-teaser"), "https://youtu.be/jNQXAC9IVRw"),
                        ),
                        line(ago(5), &me, Uuid::nil(), ChatKind::OWN, "https://youtube.com/shorts/DemoShort01"),
                    ]
                } else if mode == "clips" {
                    // video files and an animated WebP (#983): an unknown
                    // site's .mp4 stays a link, a GIPHY WebP, then the
                    // video cards in sight: a Gyazo video capture (its
                    // thumbnail from the saved answer), an Imgur .gifv
                    vec![
                        line(ago(9), name, friend, ChatKind::Im, "https://videos.example.net/clip.mp4"),
                        line(ago(8), name, friend, ChatKind::Im, "https://media.giphy.com/media/demo/giphy.webp"),
                        line(
                            ago(7),
                            name,
                            friend,
                            ChatKind::Im,
                            "https://gyazo.com/9f8e7d6c5b4a39281706f5e4d3c2b1a0",
                        ),
                        line(ago(6), &me, Uuid::nil(), ChatKind::OWN, "https://i.imgur.com/DemoClp.gifv"),
                    ]
                } else if let Some(only) = mode.strip_prefix("services") {
                    // the pages of image services (#923) and the posts of
                    // Bluesky and X (#1051), from saved answers: a removed
                    // screenshot and a text post first (no card), then a
                    // picture per service, the labelled Bluesky post's
                    // blurred, then the shares of a folder (no picture) and
                    // of a picture on a self-hosted Nextcloud (#1063);
                    // `services:gyazo,imgur` keeps those services only
                    let only: Vec<&str> = only.trim_start_matches(':').split(',').filter(|s| !s.is_empty()).collect();
                    [
                        ("none", "https://prnt.sc/removed1"),
                        ("none", "https://redd.it/demo02"),
                        ("gyazo", "https://gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9"),
                        ("flickr", "https://flic.kr/p/2nDemoAb"),
                        ("imgur", "https://imgur.com/a/DemoAlb"),
                        ("lightshot", "https://prnt.sc/demo42"),
                        ("reddit", "https://www.reddit.com/r/AuroraDemo/comments/demo01/a_new_build/"),
                        ("bluesky", "https://bsky.app/profile/aurora-demo.bsky.social/post/3demopost2abc"),
                        ("x", "https://x.com/AuroraDemo/status/1850000000000000001"),
                        ("bluesky", "https://bsky.app/profile/aurora-demo.bsky.social/post/3demolabelled"),
                        ("nextcloud", "https://cloud.example.org/s/DemoFolder3Pq9"),
                        ("nextcloud", "https://cloud.example.org/s/DemoShare7xKq2"),
                    ]
                    .iter()
                    .filter(|(key, _)| only.is_empty() || only.contains(key))
                    .map(|(_, url)| url)
                    .enumerate()
                    .map(|(i, url)| {
                        let (who, id, kind) = if i % 2 == 1 {
                            (me.as_str(), Uuid::nil(), ChatKind::OWN)
                        } else {
                            (name, friend, ChatKind::Im)
                        };
                        // half a minute apart from 9 minutes ago: the twelve
                        // links stay before the last lines of the scenario
                        let at = now - std::time::Duration::from_secs(540u64.saturating_sub(30 * i as u64));
                        line(at, who, id, kind, url)
                    })
                    .collect()
                } else if mode == "states" {
                    vec![
                        line(ago(9), name, friend, ChatKind::Im, "https://i.imgur.com/loading-poster.png"),
                        line(ago(8), name, friend, ChatKind::Im, "https://i.imgur.com/missing-poster.png"),
                        line(ago(7), name, friend, ChatKind::Im, "https://i.imgur.com/large-poster.png"),
                    ]
                } else {
                    // the picture and the video last: in sight when the
                    // conversation opens at its end
                    vec![
                        line(
                            ago(9),
                            name,
                            friend,
                            ChatKind::Im,
                            &with(aurora_i18n::tr!("demo-previews-stage"), "http://pics.example.net/scene.png"),
                        ),
                        line(ago(8), name, friend, ChatKind::Im, "https://i.gyazo.com/demo-dance.gif"),
                        line(
                            ago(7),
                            name,
                            friend,
                            ChatKind::Im,
                            &with(aurora_i18n::tr!("demo-previews-outfit"), "https://i.imgur.com/demo-outfit.jpg"),
                        ),
                        line(
                            ago(6),
                            &me,
                            Uuid::nil(),
                            ChatKind::OWN,
                            &with(aurora_i18n::tr!("demo-previews-teaser"), "https://youtu.be/jNQXAC9IVRw"),
                        ),
                    ]
                }
            });
            if let Some(extra) = preview_lines {
                let at = lines.len() - 2;
                lines.splice(at..at, extra);
            }
            for l in lines {
                world.conversation_line(friend, l);
            }
        }
        Bubbles::Busy => {
            // ours, before her automatic response (which the events bring)
            world.conversation_line(
                DEMO_NOVA,
                line(ago(1), &me, Uuid::nil(), ChatKind::OWN, aurora_i18n::tr!("demo-bubbles-free")),
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

/// AURORA_DEMO_CONV_SCROLL=up|new: the friend's conversation (the one
/// `AURORA_DEMO_CONV=aurora`, `bubbles` or `bubbles:legacy` shows) a few
/// screens long, shown from its top (`ui/chat/latest.rs`); with `new`, the
/// friend writes three messages once it is there. True for `new`.
pub fn conv_scroll_scenario() -> Option<bool> {
    match std::env::var("AURORA_DEMO_CONV_SCROLL").ok()?.trim() {
        "new" => Some(true),
        _ => Some(false),
    }
}

/// The scroll scenario's earlier talk, a day before the bubbles' lines
/// (which come after it).
pub fn conv_scroll_lines(world: &mut crate::world::World) {
    use crate::world::{ChatKind, ChatLine, Origin};
    let start = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86_400);
    let me = world.own_name();
    for n in 1..=40u64 {
        let (from, source, kind) = if n % 3 == 0 {
            (me.clone(), Uuid::nil(), ChatKind::OWN)
        } else {
            ("Nova Example".to_owned(), DEMO_FRIEND, ChatKind::Im)
        };
        let line = ChatLine {
            time: start + std::time::Duration::from_secs(n * 90),
            from,
            text: aurora_i18n::tr!("demo-scroll-earlier", n = n as i64),
            kind,
            source,
            origin: Origin::Live,
        };
        world.conversation_line(DEMO_FRIEND, line);
    }
}

/// The `new` scroll scenario: the friend's three messages, once the
/// conversation was scrolled to its top.
pub fn conv_scroll_new(world: &mut crate::world::World, scrolled: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SENT: AtomicBool = AtomicBool::new(false);
    if !scrolled || conv_scroll_scenario() != Some(true) || SENT.swap(true, Ordering::Relaxed) {
        return;
    }
    for n in 1..=3i64 {
        let line = crate::world::ChatLine {
            time: std::time::SystemTime::now(),
            from: "Nova Example".into(),
            text: aurora_i18n::tr!("demo-flash-message", n = n),
            kind: crate::world::ChatKind::Im,
            source: DEMO_FRIEND,
            origin: crate::world::Origin::Live,
        };
        world.conversation_line(DEMO_FRIEND, line);
    }
}

/// AURORA_DEMO_CONV=aurora[:flag…]: the Conversations window in Aurora's
/// workspace (ui/chat/workspace.rs), on the lines of the bubbles scenario
/// and a few more. Flags, joined by ":": `local`, `im` (the default),
/// `busy`, `group` (members' panel open) for the conversation shown;
/// `compact` for the Compact style; `narrow` (460 px: the rail),
/// `medium` (700 px) or a width in points; `call-incoming` (Tess calls),
/// `call` (in the group's call), `call-mini` (its stage reduced),
/// `call-ongoing` (Nova's group call, which we let ring), `call-away` (in
/// the group's call with the nearby chat shown: the column's call panel),
/// `call-incoming-group` (Nova's group call ringing), `call-p2p` (our call
/// to Tess), `conference` (a voice conference with Tess and Nova), `new`
/// (the menu of the column's "new conversation" button open), `volume`
/// (the menu of the nearby chat's volume open, with `local`), `channel`
/// (its channel selector shown, as the preferences can), `friend-away` or
/// `friend-offline` (the friend of `im` away in the region, or offline:
/// their crescent or ring on the selected entry, #744), `sets` (two
/// contact sets shown apart in the column, #1104).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuroraScene {
    pub view: Bubbles,
    pub width: f32,
    pub compact: bool,
    pub call: Option<AuroraCall>,
    /// `call-away`: the nearby chat shown once the call is connected.
    pub away: bool,
    /// `new`: the menu of the column's "new conversation" button open.
    pub new_menu: bool,
    /// `volume`: the menu of the nearby chat's volume open.
    pub volume_menu: bool,
    /// `channel`: the nearby chat's channel selector shown.
    pub channel: bool,
    /// `friend-away` / `friend-offline`: the friend's status, online
    /// otherwise.
    pub friend: Option<crate::ui::presence::Status>,
    /// `sets`: Family (the friend and Loup) and Clubbing (Loup and Tess)
    /// shown apart, Family first (Loup goes to it) with its heart symbol.
    pub sets: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuroraCall {
    Incoming,
    /// Nova starts a call in the group, which rings.
    IncomingGroup,
    InCall,
    Mini,
    Ongoing,
    /// We call Tess (a resident's call).
    P2p,
    /// A voice conference with Tess and Nova.
    Conference,
}

pub fn aurora_scenario() -> Option<AuroraScene> {
    parse_aurora_scene(&std::env::var("AURORA_DEMO_CONV").ok()?)
}

fn parse_aurora_scene(v: &str) -> Option<AuroraScene> {
    let mut flags = v.split(':');
    if flags.next()? != "aurora" {
        return None;
    }
    let mut scene = AuroraScene {
        view: Bubbles::Friend,
        width: 960.0,
        compact: false,
        call: None,
        away: false,
        new_menu: false,
        volume_menu: false,
        channel: false,
        friend: None,
        sets: false,
    };
    let mut away = false;
    for flag in flags {
        match flag {
            "local" => scene.view = Bubbles::Local,
            "im" => scene.view = Bubbles::Friend,
            "busy" => scene.view = Bubbles::Busy,
            "group" => scene.view = Bubbles::Group,
            "compact" => scene.compact = true,
            "narrow" => scene.width = 460.0,
            "medium" => scene.width = 700.0,
            "call-incoming" => scene.call = Some(AuroraCall::Incoming),
            "call" => scene.call = Some(AuroraCall::InCall),
            "call-mini" => scene.call = Some(AuroraCall::Mini),
            "call-ongoing" => scene.call = Some(AuroraCall::Ongoing),
            "call-incoming-group" => scene.call = Some(AuroraCall::IncomingGroup),
            "call-p2p" => scene.call = Some(AuroraCall::P2p),
            "conference" => scene.call = Some(AuroraCall::Conference),
            "new" => scene.new_menu = true,
            "volume" => scene.volume_menu = true,
            "channel" => scene.channel = true,
            "friend-away" => scene.friend = Some(crate::ui::presence::Status::Away),
            "friend-offline" => scene.friend = Some(crate::ui::presence::Status::Offline),
            "sets" => scene.sets = true,
            "call-away" => {
                scene.call = Some(AuroraCall::InCall);
                away = true;
            }
            w => {
                if let Ok(w) = w.parse::<f32>() {
                    scene.width = w.clamp(300.0, 1600.0);
                }
            }
        }
    }
    // a call shows in its conversation
    match scene.call {
        Some(AuroraCall::Incoming | AuroraCall::P2p) => scene.view = Bubbles::Busy,
        Some(_) if away => {
            scene.view = Bubbles::Local;
            scene.away = true;
        }
        Some(_) => scene.view = Bubbles::Group,
        None => {}
    }
    Some(scene)
}

/// The conversation of the scene shown in the window (Tess's for her call).
pub fn aurora_conversation(scene: AuroraScene) -> Option<Uuid> {
    match scene.call {
        // the caller of `incoming` (whom the demo's names call Tess)
        Some(AuroraCall::Incoming | AuroraCall::P2p) => Some(DEMO_NOVA),
        _ => bubbles_conversation(scene.view),
    }
}

/// The scene's lines beyond the bubbles scenario's: the group's afternoon
/// (an emote, a place, a mention of us), a few private conversations and a
/// second group's for the column; the last ones of the conversation shown are unread, for its
/// "New" line.
pub fn aurora_lines(world: &mut crate::world::World, scene: AuroraScene) {
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
    let names = |id: Uuid| world_name(id);
    let group = [
        (34, DEMO_TESS, aurora_i18n::tr!("demo-aurora-group-1").to_owned()),
        (31, DEMO_NOVA, aurora_i18n::tr!("demo-aurora-group-2").to_owned()),
        (31, DEMO_NOVA, aurora_i18n::tr!("demo-aurora-group-3").to_owned()),
        (28, DEMO_LOUP, aurora_i18n::tr!("demo-aurora-group-4").to_owned()),
        (27, Uuid::nil(), aurora_i18n::tr!("demo-aurora-group-5").to_owned()),
        (3, DEMO_NOVA, aurora_i18n::tr!("demo-aurora-mention", name = me.as_str())),
    ];
    // AURORA_DEMO_CONV_PREVIEWS: a picture in the group, blurred until clicked
    let previews = std::env::var("AURORA_DEMO_CONV_PREVIEWS").is_ok().then(|| {
        (
            2,
            DEMO_LOUP,
            format!("{} https://i.imgur.com/demo-build.jpg", aurora_i18n::tr!("demo-previews-build")),
        )
    });
    for (minutes, from, text) in group.into_iter().chain(previews) {
        let l = if from.is_nil() {
            line(ago(minutes), &me, Uuid::nil(), ChatKind::OWN, &text)
        } else {
            line(ago(minutes), &names(from), from, ChatKind::Im, &text)
        };
        world.conversation_line(DEMO_GROUP1, l);
    }
    if scene.view != Bubbles::Busy {
        world.conversation_line(
            DEMO_NOVA,
            line(
                ago(50),
                &names(DEMO_NOVA),
                DEMO_NOVA,
                ChatKind::Im,
                aurora_i18n::tr!("demo-bubbles-busy-reply"),
            ),
        );
    }
    world.conversation_line(
        DEMO_LOUP,
        line(
            ago(70),
            &names(DEMO_LOUP),
            DEMO_LOUP,
            ChatKind::Im,
            aurora_i18n::tr!("demo-aurora-loup"),
        ),
    );
    world.conversation_line(
        DEMO_TESS,
        line(
            ago(40),
            &names(DEMO_TESS),
            DEMO_TESS,
            ChatKind::Im,
            aurora_i18n::tr!("demo-aurora-tess"),
        ),
    );
    // a second group, with its insignia (Aurora Builders has none)
    let wolves = world.groups.sessions.entry(DEMO_GROUP2).or_default();
    wolves.name = aurora_i18n::tr!("demo-northern-wolves").to_owned();
    wolves.group = true;
    wolves.active = true;
    world.conversation_line(
        DEMO_GROUP2,
        line(
            ago(95),
            &names(DEMO_LOUP),
            DEMO_LOUP,
            ChatKind::Im,
            aurora_i18n::tr!("demo-aurora-wolves"),
        ),
    );
    // unread: the group's last line, and the friend's last two
    let unread = |world: &mut crate::world::World, id: Uuid, n: usize| {
        if let Some(s) = world.social.ims.iter_mut().find(|s| s.other == id) {
            s.unread = n;
        }
    };
    unread(world, DEMO_GROUP1, 1);
    unread(world, DEMO_LOUP, 1);
    if matches!(scene.view, Bubbles::Friend) {
        unread(world, DEMO_FRIEND, 3);
    }
    if scene.sets {
        // in memory only: the demo account's file stays as it was
        let sets = &mut world.contact_sets;
        sets.keep_in_memory();
        let family = aurora_i18n::tr!("demo-family");
        for (name, color, members) in [
            (family, [0.957, 0.447, 0.714, 1.0], [DEMO_FRIEND, DEMO_LOUP]),
            ("Clubbing", [0.369, 0.918, 0.831, 1.0], [DEMO_LOUP, DEMO_TESS]),
        ] {
            sets.add_set(name);
            sets.set_color(name, color);
            sets.add_to_set(&members, name, |_| true);
            sets.set_separate_chats(name, true);
        }
        sets.move_priority(family, true);
        // Family's symbol heads its section; Clubbing, without one, its dot
        sets.set_visuals(
            family,
            crate::world::contact_sets::Visuals {
                icon: Some(0),
                ..Default::default()
            },
        );
    }
}

/// The demo's name of a resident of the scene.
fn world_name(id: Uuid) -> String {
    match id {
        DEMO_NOVA => aurora_i18n::tr!("demo-nova-example").to_owned(),
        DEMO_LOUP => aurora_i18n::tr!("demo-violet-wolf").to_owned(),
        _ => "Tess Touch".to_owned(),
    }
}

/// The nearby chat of the scene: a shout, a whisper, ours, our emote and
/// a mention of us, after the bubbles scenario's lines; then the friend's
/// status of `friend-away` / `friend-offline`.
pub fn aurora_events(scene: &AuroraScene) -> Vec<NetEvent> {
    let say = |from: Uuid, name: &str, text: String, chat_type| {
        NetEvent::Chat(aurora_net::ChatMessage {
            from_name: name.into(),
            source_id: from,
            owner_id: Uuid::nil(),
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type,
            position: Vec3::ZERO,
            message: text,
        })
    };
    use aurora_net::ChatType;
    let mut ev = vec![
        say(
            DEMO_TESS,
            "Tess Touch",
            aurora_i18n::tr!("demo-aurora-local-shout").to_owned(),
            ChatType::Shout,
        ),
        say(
            DEMO_LOUP,
            aurora_i18n::tr!("demo-violet-wolf"),
            aurora_i18n::tr!("demo-aurora-local-whisper").to_owned(),
            ChatType::Whisper,
        ),
        // ours, whispered then shouted, tagged like anyone's (#1323)
        say(
            DEMO_AGENT,
            "Aurora Demo",
            aurora_i18n::tr!("demo-aurora-local-own-whisper").to_owned(),
            ChatType::Whisper,
        ),
        say(
            DEMO_AGENT,
            "Aurora Demo",
            aurora_i18n::tr!("demo-aurora-local-own-shout").to_owned(),
            ChatType::Shout,
        ),
        say(
            DEMO_AGENT,
            "Aurora Demo",
            aurora_i18n::tr!("demo-aurora-local-emote").to_owned(),
            ChatType::Normal,
        ),
        say(
            DEMO_NOVA,
            aurora_i18n::tr!("demo-nova-example"),
            aurora_i18n::tr!("demo-aurora-mention", name = "Aurora Demo"),
            ChatType::Normal,
        ),
    ];
    match scene.friend {
        // in the region (with the bubbles scenario's residents), playing
        // the away animation
        Some(crate::ui::presence::Status::Away) => {
            ev.push(NetEvent::CoarseLocations {
                handle: HANDLE,
                positions: vec![
                    (DEMO_TESS, Vec3::new(96.0, 150.0, 28.0)),
                    (DEMO_NOVA, Vec3::new(150.0, 140.0, 24.0)),
                    (DEMO_FRIEND, Vec3::new(120.0, 110.0, 24.0)),
                ],
            });
            ev.push(NetEvent::AvatarAnimations {
                sources: Vec::new(),
                avatar: DEMO_FRIEND,
                anims: vec![(crate::world::status::ANIM_AWAY, 1)],
            });
        }
        Some(crate::ui::presence::Status::Offline) => ev.push(NetEvent::FriendsOnline {
            ids: vec![DEMO_FRIEND],
            online: false,
        }),
        _ => {}
    }
    ev
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
    /// An offer to an Adult region while the account wants General and
    /// Moderate: "Change and Continue" (part C).
    Exceeded,
    /// The same, answered by "Change and Continue": the rating is asked
    /// for, then the teleport leaves.
    ExceededContinue,
    /// An offer to an Adult region for an account that is not adult: no
    /// teleport, the sender is told.
    Blocked,
}

impl LureScene {
    /// Her conversation is open on the card.
    pub fn shows_card(self) -> bool {
        !matches!(self, LureScene::Offer | LureScene::Request | LureScene::God | LureScene::Reject)
    }
}

/// The demo account's content ceiling and preference (login letters).
fn demo_access() -> (&'static str, &'static str) {
    match lure_scene() {
        Some(LureScene::Exceeded | LureScene::ExceededContinue) => ("A", "M"),
        Some(LureScene::Blocked) => ("M", "M"),
        _ => ("A", "A"),
    }
}

/// AURORA_DEMO_MATURITY=agree|disagree|fail: the Preferences open on the
/// content rating, which asks the server for "General" (the demo account is
/// adult and wants every rating). The demo server agrees, keeps "Adult"
/// (the request is sent three more times, then MaturityChangeError), or
/// fails (MaturityChangeError at once).
pub fn maturity_scene() -> Option<String> {
    std::env::var("AURORA_DEMO_MATURITY").ok()
}

/// The demo server's answer to a maturity preference request.
pub fn maturity_reply(asked: u8) -> Option<u8> {
    match maturity_scene().as_deref() {
        Some("disagree") => Some(aurora_net::search::SIM_ACCESS_ADULT),
        Some("fail") => None,
        _ => Some(asked),
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
        "exceeded" => Some(LureScene::Exceeded),
        "exceeded-continue" => Some(LureScene::ExceededContinue),
        "blocked" => Some(LureScene::Blocked),
        _ => None,
    }
}

/// AURORA_DEMO_SLURLS=1: place links in every form, in Tess Touch's
/// conversation, in the chat console and in a notification: the plaza
/// (General, connected), the lagoon (Moderate) and Nordheim (Adult) known
/// from the map, the pine grove unknown until the demo server answers its
/// lookup (`SLURLS_LOOKUP_DELAY` later), and a region that does not exist,
/// left without a badge.
pub fn slurls_scene() -> bool {
    std::env::var_os("AURORA_DEMO_SLURLS").is_some()
}

/// How late the demo server answers a region lookup in `slurls_scene`, so
/// the unknown region's badge is seen to arrive.
pub const SLURLS_LOOKUP_DELAY: std::time::Duration = std::time::Duration::from_secs(3);

/// The text of `slurls_scene`, its links in each SLURL form.
pub fn slurls_text() -> String {
    let link = |prefix: &str, region: &str, pos: (f32, f32, f32)| {
        crate::slurl::make(region, glam::Vec3::new(pos.0, pos.1, pos.2)).replace("http://maps.secondlife.com/secondlife/", prefix)
    };
    [
        link(
            "http://maps.secondlife.com/secondlife/",
            aurora_i18n::tr!("common-aurora-demo"),
            (128.0, 130.0, 25.0),
        ),
        link(
            "secondlife:///app/teleport/",
            aurora_i18n::tr!("common-boreal-lagoon"),
            (60.0, 200.0, 21.0),
        ),
        link("secondlife://", "Nordheim", (100.0, 100.0, 30.0)),
        link(
            "secondlife:///app/region/",
            aurora_i18n::tr!("common-pine-grove"),
            (40.0, 80.0, 22.0),
        ),
        link("https://slurl.com/secondlife/", "Atlantide", (1.0, 2.0, 3.0)),
    ]
    .join(" · ")
}

/// AURORA_DEMO_APPLINKS: a message with a `secondlife:///app/...` link of
/// each kind (`slapp.rs`), in a conversation with Nova and in the bubbles.
pub fn applinks_scene() -> bool {
    std::env::var_os("AURORA_DEMO_APPLINKS").is_some()
}

pub fn applinks_actions_scene() -> bool {
    std::env::var("AURORA_DEMO_APPLINKS").is_ok_and(|mode| mode == "actions")
}

/// `AURORA_DEMO_APPLINKS=preview`: a short message with an avatar, a group
/// and a place link, whose previews (the Conversations column, the
/// message's card) must show the names, not the addresses (#947).
pub fn applinks_preview_scene() -> bool {
    std::env::var("AURORA_DEMO_APPLINKS").is_ok_and(|mode| mode == "preview")
}

/// The text of `applinks_scene`; `item` is an item of the demo inventory
/// (its id and name) for the inventory link.
pub fn applinks_text(item: Option<(Uuid, String)>) -> String {
    let app = "secondlife:///app";
    if applinks_preview_scene() {
        return aurora_i18n::tr!(
            "demo-applinks-preview",
            who = format!("{app}/agent/{DEMO_LOUP}/about"),
            group = format!("{app}/group/{DEMO_GROUP2}/about"),
            place = format!("{app}/teleport/Aurora%20Demo/128/128/25")
        );
    }
    if applinks_actions_scene() {
        // Identical labels make the action marker carry the distinction;
        // custom labels must never be able to disguise the actual command.
        let resident = "Orion Exemple";
        let mut links: Vec<_> = ["about", "im", "pay", "offerteleport", "requestfriend", "reportAbuse"]
            .iter()
            .map(|verb| format!("[{app}/agent/{DEMO_LOUP}/{verb} {resident}]"))
            .collect();
        links.extend([
            format!("[{app}/group/{DEMO_GROUP1}/about Aurora Builders]"),
            format!("[{app}/parcel/{}/about Aurora Demo]", land::PARCEL_UUID),
            format!("[{app}/worldmap/Aurora%20Demo/128/128/25 Aurora Demo]"),
            format!("[{app}/teleport/Aurora%20Demo/128/128/25 Aurora Demo]"),
            format!("[{app}/inventory/show inventory]"),
            format!("[{app}/search/people/Orion%20Exemple {resident}]"),
            format!("[{app}/frobnicate/1 {resident}]"),
        ]);
        let mut text = links.join(" · ");
        // Bare URLs exercise the same path as pasting into the chat input;
        // surrounding words expose any mismatch in the link's baseline.
        for url in [
            format!("{app}/agent/{DEMO_LOUP}/about"),
            format!("{app}/agent/{DEMO_LOUP}/im"),
            format!("{app}/agent/{DEMO_LOUP}/pay"),
            format!("{app}/group/{DEMO_GROUP1}/about"),
            format!("{app}/frobnicate/1"),
        ] {
            text.push('\n');
            text.push_str(&aurora_i18n::tr!("demo-applinks-pasted", link = url));
        }
        return text;
    }
    let classified = Uuid::from_u128(0x5EA2_C1A5_0000_0000_0000_0000_0000_0001);
    // the region's treasure hunt (its name only) and the experience of
    // AURORA_DEMO_EXPERIENCE (its profile)
    let experience = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0007);
    let profiled = experience::EXPERIENCE;
    let mut links = vec![
        format!("{app}/group/{DEMO_GROUP2}/about"),
        format!("[{app}/group/{DEMO_GROUP1}/about our group]"),
        format!("{app}/parcel/{}/about", land::PARCEL_UUID),
        format!(
            "{app}/objectim/{}?name=Welcome%20Sign&owner={DEMO_LOUP}&slurl=Aurora%20Demo/140/120/25",
            Uuid::from_u128(0x5106)
        ),
        format!("{app}/experience/{experience}/profile"),
        format!("{app}/experience/{profiled}/profile"),
        format!("{app}/classified/{classified}/about"),
        format!("{app}/event/5001/details"),
        format!("{app}/worldmap/Pine%20Grove/40/80/22"),
        format!("{app}/chat/42/Hello%20from%20a%20link"),
        format!("{app}/search/places/beach"),
        format!("{app}/agent/{DEMO_LOUP}/im"),
        // a verb glued to the sentence, as objects write it (LLUrlEntryAgent)
        format!("{app}/agent/{DEMO_LOUP}/about's hat"),
        format!("{app}/wear_folder/?folder_id={}", Uuid::from_u128(0xF01D)),
        format!("{app}/frobnicate/1"),
    ];
    // the demo inventory may not be loaded yet: a made-up item then
    let (id, name) = item.unwrap_or_else(|| (Uuid::from_u128(0x17E3), "Demo notecard".into()));
    let name: String = name
        .chars()
        .map(|c| if c == ' ' { "%20".to_owned() } else { c.to_string() })
        .collect();
    links.insert(4, format!("{app}/inventory/{id}/select?name={name}"));
    links.join(" · ")
}

/// `AURORA_DEMO_APPLINKS=<command>` (group, parcel, objectim, inventory,
/// experience, classified, event, worldmap, chat, search, agent,
/// wear_folder, frobnicate): the first link of that command in `text` is
/// clicked once the scene is set.
pub fn applinks_click(text: &str) -> Option<crate::slapp::AppLink> {
    let kind = std::env::var("AURORA_DEMO_APPLINKS").ok()?;
    let kind = kind.split(':').next()?;
    let needle = format!("secondlife:///app/{}/", kind.trim());
    let at = text.find(&needle)?;
    let url = text[at..].split([' ', ']']).next()?;
    crate::slapp::parse(url)
}

/// Additional URI destinations for the minimize/reopen scenario.
pub fn applinks_action(text: &str) -> Option<crate::ui::context::CtxAction> {
    use crate::slapp::{AgentVerb, AppLink};
    use crate::ui::context::CtxAction;
    let mode = std::env::var("AURORA_DEMO_APPLINKS").ok()?;
    Some(match mode.split(':').next()? {
        "profile" => CtxAction::Profile(DEMO_LOUP),
        "pay" => CtxAction::AppLink(AppLink::Agent {
            id: DEMO_LOUP,
            verb: AgentVerb::Pay,
        }),
        "report" => CtxAction::AppLink(AppLink::Agent {
            id: DEMO_LOUP,
            verb: AgentVerb::ReportAbuse,
        }),
        "inventory-show" => CtxAction::AppLink(AppLink::InventoryShow),
        _ => CtxAction::AppLink(applinks_click(text)?),
    })
}

/// What the map would have taught before `slurls_scene`: the lagoon and
/// Nordheim, not the pine grove.
pub fn slurls_events() -> Vec<NetEvent> {
    let known = [aurora_i18n::tr!("common-boreal-lagoon"), "Nordheim"];
    vec![NetEvent::MapBlocks {
        blocks: demo_sims()
            .iter()
            .filter(|s| known.contains(&s.2))
            .map(|s| map_block(s.0, s.1, s.2, s.3))
            .collect(),
        null_sims: false,
    }]
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
    // as send_lures writes it: the text, then the sender's place
    let invitation = format!(
        "{}\r\n{}",
        aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset"),
        crate::slurl::make(aurora_i18n::tr!("common-aurora-demo"), glam::Vec3::new(130.0, 130.0, 24.0))
    );
    match scene {
        LureScene::Offer | LureScene::OfferCard | LureScene::CardAnswered | LureScene::NotificationAnswered | LureScene::Legacy => {
            vec![im(d::LURE_USER, &invitation, plaza)]
        }
        // the plaza, rated Adult
        LureScene::Exceeded | LureScene::ExceededContinue | LureScene::Blocked => {
            vec![im(d::LURE_USER, &invitation, b"256000|256000|128|130|25|0|1|0|A")]
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
        positions: radar_residents(),
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

fn radar_residents() -> Vec<(Uuid, Vec3)> {
    vec![
        (DEMO_LOUP, Vec3::new(134.0, 132.0, 24.0)),
        (DEMO_NOVA, Vec3::new(150.0, 140.0, 24.0)),
        (DEMO_TESS, Vec3::new(96.0, 150.0, 28.0)),
        (DEMO_LONG_NAME, Vec3::new(146.0, 132.0, 24.0)),
        (DEMO_WIDE_NAME, Vec3::new(120.0, 150.0, 24.0)),
        (u(100), Vec3::new(20.0, 240.0, 60.0)),
        // no altitude: its range is unknown
        (u(101), Vec3::new(200.0, 60.0, 0.0)),
    ]
}

/// Seconds of one visit of the AURORA_DEMO_RADAR=walk resident.
const RADAR_WALK_CYCLE: f64 = 30.0;
/// The AURORA_DEMO_RADAR=walk resident (`u(105)`).
pub const RADAR_WALKER: Uuid = Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0069);

/// AURORA_DEMO_RADAR=walk: the residents of `radar_events`, and one more who
/// arrives in the region every thirty seconds 60 m from `me`, walks up to
/// 6 m and back, then leaves: every alert of the radar in turn (region,
/// draw distance, chat range, in and out), with its sound and its messages
/// to scripts. `t` is in seconds.
pub fn radar_walk(me: Vec3, t: f64) -> NetEvent {
    let mut positions = radar_residents();
    let phase = t % RADAR_WALK_CYCLE;
    // away for the first third, then 60 m → 6 m → 60 m
    if phase >= RADAR_WALK_CYCLE / 3.0 {
        let half = RADAR_WALK_CYCLE / 3.0;
        let d = 6.0 + 54.0 * ((phase - 2.0 * half).abs() / half) as f32;
        positions.push((RADAR_WALKER, me + Vec3::new(d, 0.0, 0.0)));
    }
    NetEvent::CoarseLocations { handle: HANDLE, positions }
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
        p.picks = vec![
            (Uuid::from_u128(0x91C0_0001), aurora_i18n::tr!("demo-aurora-beach").into()),
            (Uuid::from_u128(0x91C0_0002), aurora_i18n::tr!("demo-my-workshop").into()),
        ];
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
    } else if id == DEMO_FRIEND {
        // the friend of the profile mock-up: a place link and a web link in
        // About, a partner, a group we are not in, three favorite places
        let region = aurora_i18n::tr!("common-boreal-lagoon");
        let slurl = format!("http://maps.secondlife.com/secondlife/{}/128/64/22", url_path(region));
        p.sl_about = aurora_i18n::tr!("demo-nova-about", slurl = slurl);
        // the names of the demo call this one "Tess ✨"
        p.partner = DEMO_NOVA;
        p.customer_type = "Premium".into();
        p.groups = vec![
            groups[0].clone(),
            aurora_net::ProfileGroup {
                id: DEMO_FIREFLIES,
                name: aurora_i18n::tr!("demo-fireflies-club").into(),
                insignia: Uuid::nil(),
            },
        ];
        p.picks = DEMO_PICKS.iter().map(|(pick, _)| (*pick, demo_pick_name(*pick).into())).collect();
        p.notes = Some(aurora_i18n::tr!("demo-nova-notes").into());
        p.fl_image = DEMO_FIRST_LIFE;
        p.fl_about = aurora_i18n::tr!("demo-nova-first-life").into();
    }
    // AURORA_DEMO_PAY: the payee's card shows her picture (issue #1195)
    if p.sl_image.is_nil() && id == DEMO_NOVA && std::env::var_os("AURORA_DEMO_PAY").is_some() {
        p.sl_image = tags::picture_texture(2);
    }
    // AURORA_DEMO_PROFILE: the residents of the scenarios have a picture
    // (and so a banner in the Aurora window)
    if p.sl_image.is_nil() && std::env::var_os("AURORA_DEMO_PROFILE").is_some() {
        p.sl_image = tags::scenario_picture(id);
    }
    // AURORA_DEMO_CHATLOG: the conversation's resident and we have one, for
    // the pictures of the history lines
    if p.sl_image.is_nil() && chatlog::mode().is_some() {
        if id == chatlog::PEER {
            p.sl_image = tags::picture_texture(3);
        } else if id == DEMO_AGENT {
            p.sl_image = tags::picture_texture(0);
        }
    }
    // AURORA_DEMO_PEOPLE: what the rows and the sheet show of them
    people::profile(&mut p);
    p
}

/// A group of the demo friend's profile that we are not in.
const DEMO_FIREFLIES: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0010);

/// The favorite places of the demo profiles and their snapshots.
const DEMO_PICKS: [(Uuid, Uuid); 3] = [
    (Uuid::from_u128(0x91C0_0001), demo_snapshot(1)),
    (Uuid::from_u128(0x91C0_0003), demo_snapshot(3)),
    (Uuid::from_u128(0x91C0_0002), demo_snapshot(2)),
];

fn demo_pick_name(pick: Uuid) -> &'static str {
    match pick.as_u128() & 0xF {
        1 => aurora_i18n::tr!("demo-aurora-beach"),
        3 => aurora_i18n::tr!("demo-lagoon-stage"),
        _ => aurora_i18n::tr!("demo-my-workshop"),
    }
}

/// A region name in the path of a place link.
fn url_path(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A demo snapshot (favorite places, classifieds, the first life picture):
/// `0x5A90…n`.
pub const fn demo_snapshot(n: u128) -> Uuid {
    Uuid::from_u128(0x5A90_0000_0000_0000_0000_0000_0000_0000 | n)
}

/// The demo first life picture.
const DEMO_FIRST_LIFE: Uuid = demo_snapshot(6);

/// The snapshot of a demo favorite place, classified or first life: a sky
/// gradient over three colors (the mock-up's pictures), 128 x 80.
fn pick_snapshot(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    const SKIES: [[[f32; 3]; 3]; 6] = [
        [[255.0, 179.0, 107.0], [196.0, 93.0, 214.0], [42.0, 47.0, 107.0]],
        [[163.0, 230.0, 53.0], [63.0, 98.0, 18.0], [26.0, 46.0, 5.0]],
        [[125.0, 211.0, 252.0], [31.0, 95.0, 122.0], [11.0, 42.0, 58.0]],
        [[253.0, 230.0, 138.0], [180.0, 83.0, 9.0], [69.0, 26.0, 3.0]],
        [[196.0, 181.0, 253.0], [99.0, 102.0, 241.0], [30.0, 27.0, 75.0]],
        [[252.0, 211.0, 77.0], [249.0, 115.0, 22.0], [28.0, 25.0, 23.0]],
    ];
    let n = id.as_u128() ^ demo_snapshot(0).as_u128();
    let sky = *SKIES.get(usize::try_from(n).ok()?.checked_sub(1)?)?;
    let (w, h) = (128u32, 80u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            // 160°: mostly down the picture, a little across
            let t = ((y as f32 / (h - 1) as f32) * 0.85 + (x as f32 / (w - 1) as f32) * 0.15).clamp(0.0, 1.0);
            let (a, b, k) = if t < 0.55 {
                (sky[0], sky[1], t / 0.55)
            } else {
                (sky[1], sky[2], (t - 0.55) / 0.45)
            };
            for c in 0..3 {
                px.push((a[c] + (b[c] - a[c]) * k) as u8);
            }
            px.push(255);
        }
    }
    Some((px, w, h))
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

/// Who sends the notification center's samples, the giver of `DEMO_OFFER`
/// (Violet Wolf in the offers, Orion Example in the demo's names).
pub const DEMO_GIVER: Uuid = Uuid::from_u128(2);

/// AURORA_DEMO_INVENTORY=offer-card[-legacy][-keep|-reject|-show]: the
/// sample offer's card in Violet Wolf's conversation, opened in sight (no
/// toast), with the bubbles or the legacy lines; `-keep` keeps it from the
/// card at frame 700, `-reject` rejects it from its notification, `-show`
/// keeps it with "Show accepted items in the inventory" on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OfferCardScene {
    pub legacy: bool,
    pub keep: bool,
    pub reject: bool,
}

pub fn offer_card_scene() -> Option<OfferCardScene> {
    let mode = std::env::var("AURORA_DEMO_INVENTORY").ok()?;
    let rest = mode.strip_prefix("offer-card")?;
    let (legacy, rest) = match rest.strip_prefix("-legacy") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    match rest {
        "" | "-keep" | "-reject" | "-show" => Some(OfferCardScene {
            legacy,
            keep: matches!(rest, "-keep" | "-show"),
            reject: rest == "-reject",
        }),
        _ => None,
    }
}

/// AURORA_DEMO_INVENTORY=offer-object-show: an object's offer (its
/// transaction) and what accepting it delivers.
pub const DEMO_TASK_OFFER: Uuid = Uuid::from_u128(0x0FFE_0000_0000_0000_0000_0000_0000_0002);
pub const DEMO_TASK_GIFT: Uuid = Uuid::from_u128(0x0FFE_0000_0000_0000_0000_0000_0000_0003);

/// "Show accepted items in the inventory" demos (`AURORA_DEMO_INVENTORY=
/// offer-show|offer-card-show|offer-object-show`): the inventory closed and
/// the option on, the offer kept at frame 700.
pub fn show_accepted_scene() -> bool {
    std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| mode.starts_with("offer") && mode.ends_with("-show"))
}

/// The simulator's answer to keeping `DEMO_TASK_OFFER`: the gift added to
/// the folder of the acceptance (BulkUpdateInventory).
pub fn task_offer_delivery(command: &aurora_net::NetCommand, owner: Uuid) -> Option<NetEvent> {
    let aurora_net::NetCommand::SendImDialog {
        dialog: crate::world::notifications::im::TASK_INVENTORY_ACCEPTED,
        id: DEMO_TASK_OFFER,
        bucket,
        ..
    } = command
    else {
        return None;
    };
    let parent = Uuid::from_slice(bucket).ok()?;
    Some(NetEvent::InventoryUpdated {
        folders: Vec::new(),
        items: vec![aurora_net::inventory::InvItem {
            id: DEMO_TASK_GIFT,
            parent,
            name: aurora_i18n::tr!("received-filing-demo-object").into(),
            asset_type: 6,
            inv_type: 6,
            owner,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            ..Default::default()
        }],
    })
}

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

/// The demo's listing of one inventory folder: five demo objects, the
/// skeleton's folders under the root, or a scenario's own contents.
fn demo_folder(f: Uuid, owner: Uuid) -> aurora_net::inventory::FolderContents {
    use aurora_net::inventory::{FolderContents, InvItem};
    if let Some(c) = env::folder_contents(f, owner).or_else(|| place::folder_contents(f, owner)) {
        return c;
    }
    let items: Vec<InvItem> = (0..5u128)
        .map(|i| InvItem {
            id: Uuid::from_u128(f.as_u128() ^ (0xABC0 + i)),
            parent: f,
            name: aurora_i18n::tr!("demo-demo-object", value0 = format!("{}", i + 1)),
            desc: String::new(),
            asset_type: [6, 3, 7, 0, 10][i as usize],
            inv_type: [6, 3, 7, 0, 10][i as usize],
            asset_id: Uuid::nil(),
            flags: 0,
            favorite: false,
            favorite_known: false,
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
    let subfolders = if f == u(1) {
        aurora_net::inventory::parse_skeleton(&demo_raw()["inventory-skeleton"])
            .into_iter()
            .filter(|x| x.parent == u(1))
            .collect()
    } else {
        Vec::new()
    };
    FolderContents {
        folder_id: f,
        owner_id: owner,
        version: 2,
        folders: subfolders,
        items: if f == u(1) || f == u(PHOTO_ALBUM_FOLDER) {
            Vec::new()
        } else {
            items
        },
    }
}

pub fn demo_reply(cmd: &aurora_net::NetCommand) -> Vec<NetEvent> {
    // what scripts would hear of a click (AURORA_DEMO_WORLD_CLICKS)
    use aurora_net::control::{LBUTTON_DOWN, LBUTTON_UP, ML_LBUTTON_DOWN, ML_LBUTTON_UP};
    match cmd {
        aurora_net::NetCommand::OneShotControl(f) if f & (LBUTTON_DOWN | LBUTTON_UP | ML_LBUTTON_DOWN | ML_LBUTTON_UP) != 0 => {
            log::info!("demo server: click bits {f:#010x}");
        }
        aurora_net::NetCommand::ObjectGrab { local_id, .. } => log::info!("demo server: ObjectGrab {local_id}"),
        _ => {}
    }
    if let Some(events) = hud::reply(cmd) {
        return events;
    }
    if let Some(events) = group::reply(cmd) {
        return events;
    }
    if let Some(events) = search::reply(cmd) {
        return events;
    }
    if let Some(events) = social_menu::reply(cmd) {
        return events;
    }
    match cmd {
        // a lure accepted offline: the teleport starts, then has nowhere to go
        aurora_net::NetCommand::AcceptLure { .. } => vec![
            NetEvent::TeleportStarted,
            NetEvent::TeleportFailed {
                reason: aurora_i18n::tr!("demo-no-teleport-offline").into(),
            },
        ],
        aurora_net::NetCommand::AgentAnimation { anim, start: false }
            if *anim == SEAT_ANIM
                || (*anim == CROWD_DANCE_ANIM
                    && matches!(
                        std::env::var("AURORA_DEMO_WORN").as_deref(),
                        Ok("temporary-detach" | "temporary-drop")
                    )) =>
        {
            vec![NetEvent::AvatarAnimations {
                avatar: DEMO_AGENT,
                anims: vec![(IDLE_ANIM, 1)],
                sources: Vec::new(),
            }]
        }
        // the region environment is Region / Estate's simulator's
        aurora_net::NetCommand::Land(l) => region::environment_reply(l).unwrap_or_else(|| land::reply(l)),
        aurora_net::NetCommand::Estate(e) => region::reply(e),
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
                        if let Some((first, last, _)) = group::member_name(id) {
                            return (*id, first.to_owned(), last.to_owned());
                        }
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
                        } else if let Some(resident) = people::name(id) {
                            resident
                        } else if let Some(resident) = tags::name(id) {
                            resident
                        } else if let Some(member) = group::member_name(id) {
                            member
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
            let out = folders.iter().map(|f| demo_folder(*f, *owner)).collect();
            vec![NetEvent::InventoryContents(out)]
        }
        // no depth limit to reach here: whole subtrees, parents first
        aurora_net::NetCommand::FetchInventoryTree {
            request,
            parent,
            children,
            depth,
            owner,
            ..
        } => {
            let mut todo: Vec<(Uuid, i32)> = if children.is_empty() {
                vec![(*parent, *depth)]
            } else {
                children.iter().rev().map(|c| (*c, depth - 1)).collect()
            };
            let mut out = Vec::new();
            while let Some((f, depth)) = todo.pop() {
                let c = demo_folder(f, *owner);
                if depth > 0 {
                    todo.extend(c.folders.iter().rev().filter(|x| x.parent == f).map(|x| (x.id, depth - 1)));
                }
                out.push(c);
            }
            vec![NetEvent::InventoryTreeFetched {
                request: *request,
                result: Ok(out),
            }]
        }
        // block list: Violet Wolf, an object, a name and the chat of group 2
        aurora_net::NetCommand::RequestMuteList { .. } => {
            // AURORA_DEMO_PEOPLE's tabs: their own block list
            if let Some(text) = people::mute_list() {
                return vec![NetEvent::MuteList(aurora_net::MuteListSource::File(text))];
            }
            let text = aurora_i18n::tr!(
                "demo-1-violet-wolf-0-2-spam-box-0-0",
                value0 = format!("{}", DEMO_LOUP),
                value1 = format!("{}", Uuid::from_u128(0xB10C_0000_0000_0000_0000_0000_0000_0001)),
                value2 = format!("{}", Uuid::nil()),
                value3 = format!("{}", Uuid::nil()),
                value4 = format!("{}", DEMO_GROUP2)
            );
            // The animesh fixture needs its wearer visible in full, and
            // AURORA_DEMO_PEOPLE the wolf as a friend. The attachment-click,
            // derender and seat-kill fixtures need the same visible,
            // unblocked wearer.
            let text = if std::env::var_os("AURORA_DEMO_ANIMESH").is_some()
                || std::env::var_os("AURORA_DEMO_DERENDER").is_some()
                || seat_kill::on()
                || std::env::var("AURORA_DEMO_WORLD_CLICKS").is_ok_and(|m| m.starts_with("worn-"))
                || people::mode().is_some()
                || social_menu::step().is_some()
            {
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
            // AURORA_DEMO_LAND=…:officer adds the land powers of the parcel's group
            let g = |id: Uuid, name: &str, notices: bool| aurora_net::GroupMembership {
                id,
                name: name.into(),
                insignia: Uuid::nil(),
                powers: 1 << 16 | land::group_powers(id),
                accept_notices: notices,
                list_in_profile: true,
                contribution: 0,
            };
            // our abilities in the first one follow the group profile's
            // scenario; the groups joined or left since are in the list
            let mut groups = vec![
                aurora_net::GroupMembership {
                    powers: group::my_powers(),
                    contribution: group::MY_CONTRIBUTION,
                    ..g(DEMO_GROUP1, "Aurora Builders", true)
                },
                // the one with an insignia (its search profile's): the
                // lists show it, the others the group glyph
                aurora_net::GroupMembership {
                    insignia: TEX_SNAPSHOT,
                    ..g(DEMO_GROUP2, aurora_i18n::tr!("demo-northern-wolves"), false)
                },
                aurora_net::GroupMembership {
                    list_in_profile: false,
                    ..g(DEMO_GROUP3, aurora_i18n::tr!("common-plaza-market"), true)
                },
            ];
            if !group::adjust_groups(&mut groups) {
                return vec![NetEvent::Groups(groups)];
            }
            vec![
                NetEvent::Groups(groups),
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
                        entered: false,
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
                        not_found: false,
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
                        not_found: false,
                    }]
                }
                aurora_net::SessionMethod::DeclineInvitation => Vec::new(),
            }
        }
        // voice calls: the simulated ChatSessionRequest (AURORA_DEMO_CALL)
        aurora_net::NetCommand::Call { method, session, .. } => calls::reply(method.clone(), *session),
        // the request made once after login: what the grid kept while we
        // were away (AURORA_DEMO_CHATLOG=offline), else nothing
        aurora_net::NetCommand::RequestOfflineMessages => match chatlog::mode() {
            Some(chatlog::Mode::Offline) => chatlog::offline_messages(DEMO_AGENT),
            _ => Vec::new(),
        },
        // a conference started by message (AURORA_DEMO_CALL=conference-legacy)
        aurora_net::NetCommand::SendImDialog {
            dialog: 16, id, bucket, ..
        } => calls::legacy_conference(*id, bucket),
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
                entered: false,
            }],
        }],
        // the typing notices are no lines
        aurora_net::NetCommand::Chat {
            chat_type: aurora_net::ChatType::StartTyping | aurora_net::ChatType::StopTyping,
            ..
        } => Vec::new(),
        // the simulator plays our typing animation (and the stand again
        // once it stops), as for anyone in range
        aurora_net::NetCommand::AgentAnimation { anim, start } if *anim == crate::world::radar::ANIM_AGENT_TYPE => {
            let mut anims = vec![(IDLE_ANIM, 1)];
            if *start {
                anims.push((*anim, 2));
            }
            vec![NetEvent::AvatarAnimations {
                avatar: DEMO_AGENT,
                anims,
                sources: Vec::new(),
            }]
        }
        // A line on a script channel is heard by listening scripts only,
        // never echoed as local chat: the demo's worn listener answers it,
        // as a HUD driven by a gesture would (#952).
        aurora_net::NetCommand::Chat { message, channel, .. } if *channel != 0 => {
            log::info!("demo channel chat: channel={channel} message={message:?}");
            let channel = channel.to_string();
            vec![NetEvent::Chat(aurora_net::ChatMessage {
                from_name: aurora_i18n::tr!("demo-channel-listener-name").into(),
                source_id: Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0952),
                owner_id: DEMO_AGENT,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                position: Vec3::ZERO,
                message: aurora_i18n::tr!(
                    "demo-channel-listener-heard",
                    channel = channel.as_str(),
                    message = message.as_str()
                ),
            })]
        }
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
            // the beach (parcel 0), the workshop (1), the lagoon's stage (2)
            let n = match pick.as_u128() & 0xF {
                1 => 0,
                3 => 2,
                _ => 1,
            };
            let (desc, sim) = match n {
                0 => (
                    aurora_i18n::tr!("demo-a-quiet-spot-by-the-water-perfect-for-watching"),
                    aurora_i18n::tr!("common-aurora-demo"),
                ),
                1 => (aurora_i18n::tr!("demo-workshop-desc"), aurora_i18n::tr!("common-sandbox")),
                _ => (aurora_i18n::tr!("demo-lagoon-stage-desc"), aurora_i18n::tr!("common-boreal-lagoon")),
            };
            vec![NetEvent::PickInfo(Box::new(aurora_net::PickInfo {
                id: *pick,
                creator: *creator,
                parcel: Uuid::from_u128(0x9A2C_E100 + n as u128),
                name: demo_pick_name(*pick).into(),
                desc: desc.into(),
                snapshot: DEMO_PICKS.iter().find(|(p, _)| p == pick).map_or(Uuid::nil(), |(_, s)| *s),
                sim_name: sim.into(),
                pos_global: glam::DVec3::new(256_000.0 + 140.0, 256_000.0 + 120.0, 25.0),
                enabled: true,
                ..Default::default()
            }))]
        }
        aurora_net::NetCommand::RemoteParcelRequest { handle, position, .. } => place::remote_parcel(*handle, *position),
        aurora_net::NetCommand::RegionHandleRequest(id) => place::region_handle(*id),
        aurora_net::NetCommand::ParcelInfoRequest(id) => match place::parcel_info(*id) {
            Some(ev) => vec![ev],
            // the parcels of the profile picks: the plaza, the sandbox's
            // workshop, the club of the lagoon (moderate)
            None => {
                let (name, sim, flags) = match id.as_u128() {
                    0x9A2C_E101 => ("", aurora_i18n::tr!("common-sandbox"), 0),
                    0x9A2C_E102 => (aurora_i18n::tr!("demo-fireflies-club"), aurora_i18n::tr!("common-boreal-lagoon"), 1),
                    _ => (aurora_i18n::tr!("common-aurora-plaza"), aurora_i18n::tr!("common-aurora-demo"), 0),
                };
                vec![NetEvent::ParcelInfo(Box::new(aurora_net::ParcelSummary {
                    id: *id,
                    name: name.into(),
                    sim_name: sim.into(),
                    flags,
                    ..Default::default()
                }))]
            }
        },
        cmd @ (aurora_net::NetCommand::ClassifiedsRequest(_)
        | aurora_net::NetCommand::ClassifiedInfoRequest(_)
        | aurora_net::NetCommand::ClassifiedUpdate(_)
        | aurora_net::NetCommand::ClassifiedDelete(_)
        | aurora_net::NetCommand::ClassifiedStatsRequest(_)
        | aurora_net::NetCommand::ClassifiedClick { .. }) => classifieds::reply(cmd).unwrap_or_default(),
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
        // the two online friends who let us map them: one in our region, one
        // in the Boreal Lagoon
        aurora_net::NetCommand::FindAgent(prey) if [u(100), u(102)].contains(prey) => {
            let remote = std::env::var_os("AURORA_DEMO_FRIEND_LOCATION").is_some();
            let (x, y) = if *prey == u(100) && !remote {
                (256_020.0, 256_240.0)
            } else {
                (256_316.0, 256_200.0)
            };
            vec![NetEvent::AgentFound { prey: *prey, x, y }]
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
        // "Stop animations": the simulator answers with the stand restarted
        // (LLAgent::stopCurrentAnimations sends it right after)
        aurora_net::NetCommand::AnimationStateReset => vec![NetEvent::AvatarAnimations {
            sources: Vec::new(),
            avatar: DEMO_AGENT,
            anims: vec![(IDLE_ANIM, 1)],
        }],
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
                // Nova may sell copies (for_sale_selection, the hover tips)
                owner_mask: aurora_net::build::perm::TRANSFER | aurora_net::build::perm::COPY | aurora_net::build::perm::MODIFY,
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
        // the other objects' names for the hover tips (AURORA_DEMO_HOVER_TIPS)
        aurora_net::NetCommand::RequestObjectProperties { object, .. } if hover_tips::scenario().is_some() => {
            vec![NetEvent::ObjectProperties(vec![hover_tips::object_props(*object)])]
        }
        // Report Abuse: the region's categories, the reported object (owned
        // by Nova) and a report the demo keeps to itself
        aurora_net::NetCommand::RequestAbuseCategories { .. } => {
            vec![NetEvent::AbuseCategories(crate::report_abuse::default_categories())]
        }
        aurora_net::NetCommand::RequestReportObjectProperties { object, .. } => {
            vec![NetEvent::ObjectProperties(vec![aurora_net::build::ObjectProps {
                object_id: *object,
                owner_id: DEMO_NOVA,
                name: aurora_i18n::tr!("common-demo-object").into(),
                ..Default::default()
            }])]
        }
        aurora_net::NetCommand::SendAbuseReport(_) => vec![NetEvent::AbuseReportSent(Ok(()))],
        aurora_net::NetCommand::RequestPayPrice { object, .. }
            if *object == action_id(972)
                || (*object == action_id(974) && std::env::var("AURORA_DEMO_ACTIONS").is_ok_and(|m| m.starts_with("pay-linked-menu"))) =>
        {
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
        // AURORA_DEMO_ACTIONS=tp-door: the touched object answers like a
        // club's teleport door scripted with llMapDestination (issue #283)
        aurora_net::NetCommand::ObjectGrab { local_id: 975, .. }
            if std::env::var("AURORA_DEMO_ACTIONS").is_ok_and(|m| m.starts_with("tp-door")) =>
        {
            vec![NetEvent::ScriptTeleportRequest {
                object_name: "Club door".into(),
                region: aurora_i18n::tr!("common-aurora-demo").into(),
                position: Vec3::new(168.4, 149.7, 118.2),
                flags: aurora_net::beacon::SHOW_MAP | aurora_net::beacon::FOCUS_MAP,
            }]
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

/// Our avatar's centre above the top of a prim it sits on (no sit target:
/// the demo's simulator puts it on top).
const SEAT_LIFT: f32 = 0.5;

/// The simulator's answer to a sit on any prim of the demo (a root, its
/// local id and size): the sit response, our avatar parented to it on its
/// top, the seat's animation.
pub fn sit_on_prim(local_id: u32, full_id: Uuid, size: Vec3) -> Vec<NetEvent> {
    let mut avatar = action_avatar(true);
    avatar.parent_id = local_id;
    avatar.position = Vec3::Z * (0.5 * size.z + SEAT_LIFT);
    avatar.rotation = Quat::IDENTITY;
    vec![
        NetEvent::SitResponse {
            object: full_id,
            camera_eye: Vec3::ZERO,
            camera_at: Vec3::ZERO,
            force_mouselook: false,
        },
        NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![avatar],
        },
        NetEvent::AvatarAnimations {
            avatar: DEMO_AGENT,
            anims: vec![(SEAT_ANIM, 1)],
            sources: Vec::new(),
        },
    ]
}

/// Standing up from a prim at `seat` (region coordinates) of `size`: our
/// avatar unparented beside it, on the land or at the height of the seat's
/// base, whichever is higher.
pub fn stand_from_prim(seat: Vec3, size: Vec3) -> Vec<NetEvent> {
    let (x, y) = (seat.x, seat.y - 0.5 * size.y - 0.6);
    let half_height = 0.5 * action_avatar(false).scale.z;
    let z = (floor_at(x, y) + half_height).max(seat.z - 0.5 * size.z + half_height);
    let mut avatar = action_avatar(false);
    avatar.position = Vec3::new(x, y, z);
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![avatar],
    }]
}

pub fn action_mode_id(mode: &str) -> u32 {
    if mode.starts_with("probe-") {
        return 975;
    }
    if mode.starts_with("pay-linked-menu") {
        return 974;
    }
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
        "none" | "touch" | "tp" => 975,
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

/// Invisible probe enclosing the scripted Touch cube (#1401).
pub fn click_probe(mode: &str, pos: Vec3) -> NetEvent {
    let mut object = action_object(976, 0, 138.0, 126.0, [1.0, 1.0, 1.0, 0.0], "");
    object.position = pos;
    object.scale = Vec3::splat(4.0);
    object.extra.reflection_probe = Some(aurora_prim::extra::ReflectionProbeParams {
        flags: u8::from(mode.starts_with("probe-box")),
        ..crate::build::edits::PROBE_DEFAULT
    });
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
    child.parent_id = if mode.starts_with("pay-linked-menu") { 972 } else { 971 };
    objects.push(child);
    if (mode.starts_with("pay-menu") || mode.starts_with("pay-linked-menu"))
        && let Some(object) = objects.iter_mut().find(|o| o.local_id == 972)
    {
        object.click_action = crate::interaction::code::TOUCH;
        object.update_flags |= 1 << 7;
    }
    let selected = action_mode_id(&mode);
    if selected != 0 {
        objects.retain(|o| {
            o.local_id == selected
                || (selected == 974 && o.local_id == 972)
                || ((selected == 971 || selected == 980 || selected == 981) && (o.local_id == 971 || o.local_id == 974))
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
/// AURORA_DEMO_WORLD_CLICKS=worn-other-mesh: a worn cube mesh, no skin.
pub const WORN_UNRIGGED_MESH: Uuid = Uuid::from_u128(0xa0e0_fa00_0000_0000_0000_0000_0000_0002);

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

pub fn friend_join_scenario() -> bool {
    std::env::var("AURORA_DEMO_FRIEND_LOCATION").is_ok_and(|m| {
        matches!(
            m.as_str(),
            "join-map" | "join-contact" | "join-fly" | "join-fly-blocked" | "join-fly-late" | "join-fly-custom"
        )
    })
}

pub fn friend_join_flight_scenario() -> bool {
    std::env::var("AURORA_DEMO_FRIEND_LOCATION")
        .is_ok_and(|m| matches!(m.as_str(), "join-fly" | "join-fly-blocked" | "join-fly-late" | "join-fly-custom"))
}

pub fn friend_join_late_flight_scenario() -> bool {
    std::env::var("AURORA_DEMO_FRIEND_LOCATION").as_deref() == Ok("join-fly-late")
}

pub fn friend_join_custom_flight_scenario() -> bool {
    std::env::var("AURORA_DEMO_FRIEND_LOCATION").as_deref() == Ok("join-fly-custom")
}

pub fn friend_join_parcel(position: Vec3, allow_fly: bool) -> aurora_net::ParcelInfo {
    let mut bitmap = vec![0; 512];
    let cell = (position.y as usize / 4).min(63) * 64 + (position.x as usize / 4).min(63);
    bitmap[cell / 8] |= 1 << (cell % 8);
    aurora_net::ParcelInfo {
        bitmap,
        flags: if allow_fly { aurora_net::parcel_flags::ALLOW_FLY } else { 0 },
        ..Default::default()
    }
}

/// Make the conversation's friend remote without keeping a nearby object
/// or coarse position. Combine with existing contacts/profile/map scenarios.
pub fn friend_location(world: &mut crate::world::World) {
    let Ok(mode) = std::env::var("AURORA_DEMO_FRIEND_LOCATION") else {
        return;
    };
    if !matches!(
        mode.as_str(),
        "allowed"
            | "denied"
            | "offline"
            | "altitude"
            | "join-map"
            | "join-contact"
            | "join-fly"
            | "join-fly-blocked"
            | "join-fly-late"
            | "join-fly-custom"
    ) {
        return;
    }
    if let Some(idx) = world.objects.index_of_uuid(&DEMO_FRIEND) {
        world.objects.remove_idx(idx);
    }
    for positions in world.coarse.values_mut() {
        positions.retain(|(id, _)| *id != DEMO_FRIEND);
    }
    if let Some(f) = world.social.friends.iter_mut().find(|f| f.id == DEMO_FRIEND) {
        f.online = mode != "offline";
        f.rights_given = 3;
        f.rights_has = if mode == "denied" { 1 } else { 3 };
    }
    if friend_join_flight_scenario() {
        world.agent.flying = false;
    } else if mode == "altitude" || friend_join_scenario() {
        world
            .agent
            .get_off(glam::Vec3::new(world.agent.position.x, world.agent.position.y, 3000.0));
        world.agent.flying = true;
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
        // only the People API (names) is simulated, and what the
        // scenarios of sim_extras add
        NetEvent::Capabilities {
            handle: HANDLE,
            caps: Arc::new(
                [("GetDisplayNames".to_owned(), "demo://names".to_owned())]
                    .into_iter()
                    .chain(sim_extras::caps())
                    .collect(),
            ),
        },
        NetEvent::RegionHandshake(Arc::new(RegionInfo {
            handle: HANDLE,
            name: aurora_i18n::tr!("common-aurora-demo").into(),
            region_id: Uuid::nil(),
            water_height: 20.0,
            sim_access: 13,
            region_flags: aurora_net::region_flags::ALLOW_VOICE | home::region_flag(),
            terrain_base: [Uuid::nil(); 4],
            terrain_detail: region::terrain_ids().unwrap_or([Uuid::nil(); 4]),
            terrain_start_height: region::terrain_start_height(),
            terrain_height_range: if region::terrain_ids().is_some() { [120.0; 4] } else { [30.0; 4] },
            size_x: 256,
            size_y: 256,
            is_main: true,
            // AURORA_DEMO_REGION: the avatar may own the estate or manage it;
            // AURORA_DEMO_ENV_EDITOR=region: the avatar manages the estate
            owner: region::estate_owner(),
            is_estate_manager: region::is_manager() || env::estate_manager() || sim_extras::estate_manager(),
            product_name: "Estate / Full Region".into(),
            billable_factor: 1.0,
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
    if !std::env::var("AURORA_DEMO_ACTIONS").is_ok_and(|m| m == "1") && std::env::var_os("AURORA_DEMO_WORLD_CLICKS").is_none() && !texanim {
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
                panel_scale(k, label),
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
    // AURORA_DEMO_FOLIAGE=1: the same cut-out foliage panel (2.5 m, alpha a
    // mask) at 5, 10, 20, 35 and 60 m along the start view, one side then
    // the other, to see it thin out (or not, AURORA_ALPHA_COVERAGE) with the
    // distance; from the start position, or AURORA_DEMO_POS="x,y" (an open
    // place: "80,100")
    if std::env::var_os("AURORA_DEMO_FOLIAGE").is_some() {
        let look = Vec3::new(0.8, 0.4, 0.0).normalize();
        let across = Vec3::new(-look.y, look.x, 0.0);
        let yaw = look.y.atan2(look.x);
        let eye = std::env::var("AURORA_DEMO_POS")
            .ok()
            .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
            .filter(|p| p.len() >= 2)
            .map_or(Vec3::new(132.0, 126.0, 0.0), |p| Vec3::new(p[0], p[1], 0.0));
        for (k, dist) in [5.0f32, 10.0, 20.0, 35.0, 60.0].into_iter().enumerate() {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let p = eye + look * dist + across * side * (1.6 + dist * 0.06);
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = TEX_FOLIAGE;
                f.color = [1.0, 1.0, 1.0, 1.0];
            }
            add(prim(
                id,
                Vec3::new(p.x, p.y, height(p.x, p.y) + 1.25),
                Quat::from_rotation_z(yaw),
                Vec3::new(0.02, 2.5, 2.5),
                boxp,
                Arc::new(t),
                ExtraParams::default(),
                "",
            ));
            id += 1;
        }
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
        // AURORA_DEMO_PBR_OVERRIDE=alpha: three upright panels of the same
        // opaque material, facing the start position, whose override zeroes
        // the base color alpha, as seen on a worn mesh attachment. Like the
        // Legacy Viewer, the opaque one stays drawn; the masked one is cut
        // by the shader, the blended one is not drawn.
        if std::env::var("AURORA_DEMO_PBR_OVERRIDE").is_ok_and(|v| v == "alpha") {
            let modes = [
                ("", aurora_i18n::tr!("demo-override-alpha-zero-opaque")),
                (",'am':i2", aurora_i18n::tr!("demo-override-alpha-zero-mask")),
                (",'am':i1", aurora_i18n::tr!("demo-override-alpha-zero-blend")),
            ];
            for (k, (am, label)) in modes.into_iter().enumerate() {
                let mut t = TextureEntry::default();
                for f in t.faces.iter_mut() {
                    f.texture = BLANK_TEXTURE;
                    f.color = [1.0, 1.0, 1.0, 1.0];
                }
                let mut o = prim(
                    id,
                    Vec3::new(134.5, 129.6 - 1.6 * k as f32, floor + 1.6),
                    Quat::IDENTITY,
                    Vec3::new(0.1, 1.2, 1.2),
                    boxp,
                    Arc::new(t),
                    ExtraParams {
                        render_materials: (0..6).map(|f| (f, MAT_PBR_TILES)).collect(),
                        ..Default::default()
                    },
                    "",
                );
                let od = format!("{{'bc':[r1,r1,r1,r0]{am}}}");
                let payload = format!("{{'id':i{id},'te':[i0,i1,i2,i3,i4,i5],'od':[{od},{od},{od},{od},{od},{od}]}}");
                if let Some((local_id, sides)) = aurora_net::objects::parse_gltf_override(payload.as_bytes()) {
                    overrides.push(NetEvent::GltfOverrides {
                        handle: HANDLE,
                        local_id,
                        sides,
                    });
                }
                o.text = label.into();
                add(o);
                id += 1;
            }
        }
    }
    // AURORA_DEMO_PBR_METAL=1: white metal (PBR, metallic 1) ahead of the
    // start position, three spheres from polished to rough and a slab: a
    // silver that reflects the place, not a tint of its own; then a polished
    // metal cube whose occlusion map is black but for a disc, which only
    // reflects in that disc (AURORA_PBR_OCCLUSION)
    if std::env::var_os("AURORA_DEMO_PBR_METAL").is_some() {
        let floor = height(cx, cy) + 0.35;
        let metal = |id: u32, pos: Vec3, scale: Vec3, path: _, mat: Uuid| {
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = BLANK_TEXTURE;
                f.color = [1.0, 1.0, 1.0, 1.0];
            }
            prim(
                id,
                pos,
                Quat::IDENTITY,
                scale,
                path,
                Arc::new(t),
                ExtraParams {
                    render_materials: (0..6).map(|f| (f, mat)).collect(),
                    ..Default::default()
                },
                "",
            )
        };
        for (k, mat) in MAT_PBR_METAL.into_iter().enumerate() {
            let y = 130.0 - 1.6 * k as f32;
            add(metal(id, Vec3::new(136.0, y, floor + 0.8), Vec3::splat(1.2), sphere, mat));
            id += 1;
        }
        add(metal(
            id,
            Vec3::new(136.0, 131.7, floor + 0.6),
            Vec3::splat(1.0),
            boxp,
            MAT_PBR_ORM_DISC,
        ));
        id += 1;
        add(metal(
            id,
            Vec3::new(137.5, 128.4, floor + 0.05),
            Vec3::new(1.5, 4.5, 0.1),
            boxp,
            MAT_PBR_METAL[0],
        ));
        id += 1;
        // left of the cube, an inside corner of the half-polished metal
        // (floor, back and side): its reflections fade into the corner with
        // GTAO's specular occlusion (AURORA_GTAO_SPECULAR)
        for (pos, scale) in [
            (Vec3::new(135.3, 133.6, floor + 0.05), Vec3::new(1.4, 1.4, 0.1)),
            (Vec3::new(136.0, 133.6, floor + 0.6), Vec3::new(0.1, 1.4, 1.2)),
            (Vec3::new(135.3, 134.25, floor + 0.6), Vec3::new(1.4, 0.1, 1.2)),
        ] {
            add(metal(id, pos, scale, boxp, MAT_PBR_METAL[1]));
            id += 1;
        }
    }
    // AURORA_DEMO_PBR_EMISSIVE=1: three dark spheres ahead of the start
    // position with the same orange emissive, opaque, alpha-masked and
    // blended: without HDR and Emissive (AURORA_HDR=0) the first two lose
    // their light, the blended one keeps it (HAS_EMISSIVE)
    if std::env::var_os("AURORA_DEMO_PBR_EMISSIVE").is_some() {
        let floor = height(cx, cy) + 0.35;
        for (k, mat) in MAT_PBR_EMISSIVE.into_iter().enumerate() {
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = BLANK_TEXTURE;
                f.color = [1.0, 1.0, 1.0, 1.0];
            }
            add(prim(
                id,
                Vec3::new(136.0, 124.0 + 2.0 * k as f32, floor + 2.6),
                Quat::IDENTITY,
                Vec3::splat(1.2),
                sphere,
                Arc::new(t),
                ExtraParams {
                    render_materials: (0..6).map(|f| (f, mat)).collect(),
                    ..Default::default()
                },
                "",
            ));
            id += 1;
        }
    }
    // AURORA_DEMO_SHIMMER=1: a long glossy plate ahead of the start position,
    // its fine normal map repeated many times, seen at a grazing angle: far
    // away its relief is smaller than a pixel and its highlights shimmer as
    // the camera moves, unless the specular anti-aliasing widens them
    // (AURORA_SPECULAR_AA)
    if std::env::var_os("AURORA_DEMO_SHIMMER").is_some() {
        let floor = height(cx, cy) + 0.35;
        let mut t = TextureEntry::default();
        for f in t.faces.iter_mut() {
            f.texture = BLANK_TEXTURE;
            f.color = [0.35, 0.36, 0.4, 1.0];
            f.material_id = MAT_SHIMMER;
        }
        add(prim(
            id,
            Vec3::new(142.0, 127.0, floor + 0.03),
            Quat::IDENTITY,
            Vec3::new(16.0, 6.0, 0.06),
            boxp,
            Arc::new(t),
            ExtraParams::default(),
            "",
        ));
        id += 1;
    }
    // AURORA_DEMO_BOUNCE=1: a white floor between a red wall and a green
    // one, with a white box against them, left of the start position: with
    // GTAO's indirect light the floor and the box take the walls' colours
    // near them (AURORA_SSIL)
    if std::env::var_os("AURORA_DEMO_BOUNCE").is_some() {
        let floor = height(cx, cy) + 0.35;
        let plain = |id: u32, pos: Vec3, scale: Vec3, color: [f32; 4]| {
            let mut t = TextureEntry::default();
            for f in t.faces.iter_mut() {
                f.texture = BLANK_TEXTURE;
                f.color = color;
            }
            prim(id, pos, Quat::IDENTITY, scale, boxp, Arc::new(t), ExtraParams::default(), "")
        };
        let white = [0.9, 0.9, 0.9, 1.0];
        for (pos, scale, color) in [
            (Vec3::new(135.0, 137.0, floor + 0.05), Vec3::new(3.0, 3.0, 0.1), white),
            (
                Vec3::new(135.0, 138.55, floor + 1.0),
                Vec3::new(3.0, 0.1, 2.0),
                [0.9, 0.1, 0.08, 1.0],
            ),
            (
                Vec3::new(136.55, 137.0, floor + 1.0),
                Vec3::new(0.1, 3.0, 2.0),
                [0.1, 0.8, 0.15, 1.0],
            ),
            (Vec3::new(135.9, 137.9, floor + 0.4), Vec3::new(0.7, 0.7, 0.7), white),
        ] {
            add(plain(id, pos, scale, color));
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
            if (mode == "wind" && !wind) || (mode == "smoke" && wind) || (mode == "legacy" && !wind) || mode == "leash" {
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
            let particles = if matches!(mode, "region-wind" | "region-calm") {
                compressed_particle_block(&region_wind_particles(wind), true)
            } else {
                compressed_scripted_particles(wind, mode == "legacy")
            };
            if let Some(particles) = particles {
                o.particles = particles;
            }
            add(o);
            id += 1;
        }
        if mode == "leash" {
            // a collar and the hand that holds it, 5 m apart
            let hand = prim(
                id,
                Vec3::new(137.0, 126.5, ground + 3.5),
                Quat::IDENTITY,
                Vec3::splat(0.1),
                boxp,
                te(teal, 0, true, 0.0),
                ExtraParams::default(),
                "",
            );
            let target = hand.full_id;
            add(hand);
            id += 1;
            let mut collar = prim(
                id,
                Vec3::new(137.0, 121.5, ground + 3.0),
                Quat::IDENTITY,
                Vec3::splat(0.1),
                boxp,
                te(teal, 0, true, 0.0),
                ExtraParams::default(),
                "",
            );
            collar.particles = aurora_net::objects::ParticleUpdate::Set(leash_particles(target));
            add(collar);
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
        // AURORA_DEMO_SOCIAL_MENU: the wolf's, for "Block particles"
        if social_menu::step().is_some() {
            o.owner_id = social_menu::RESIDENT;
        }
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
    // a mirror as built in SL: an invisible box reflection probe flagged as
    // mirror (its plane through the center, along its +Z), and a shiny
    // 10 cm panel through that plane, its face 5 cm in front of it, where
    // the Legacy Viewer gives it almost the whole mirror (tapHeroProbe)
    {
        let probe = aurora_prim::extra::ReflectionProbeParams {
            ambiance: 0.0,
            clip_distance: 0.0,
            flags: 0x1 | 0x4,
        };
        let rot = Quat::from_rotation_z(-1.2) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let center = Vec3::new(cx + 2.0, cy - 9.0, g(cx, cy) + 2.0);
        add(prim(
            id,
            center,
            rot,
            Vec3::new(3.4, 3.0, 1.0),
            boxp,
            te([1.0, 1.0, 1.0, 0.0], 0, false, 0.0),
            ExtraParams {
                reflection_probe: Some(probe),
                ..Default::default()
            },
            "",
        ));
        id += 1;
        add(prim(
            id,
            center,
            rot,
            Vec3::new(3.0, 2.6, 0.1),
            boxp,
            te([0.85, 0.87, 0.9, 1.0], 3, false, 0.0),
            ExtraParams::default(),
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
    // AURORA_DEMO_HALO: a wide translucent pane 3 m ahead of the start
    // position, facing the camera, so that the avatar stands between the
    // camera and the pane (issue #556: no halo around the avatar on it)
    if std::env::var_os("AURORA_DEMO_HALO").is_some() {
        let (sx, sy) = std::env::var("AURORA_DEMO_POS")
            .ok()
            .and_then(|v| {
                let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                (p.len() >= 2).then(|| (p[0], p[1]))
            })
            .unwrap_or((132.0, 126.0));
        // the start look-at of AgentMovementComplete, (0.8, 0.4)
        let (dx, dy) = (0.894_427_2, 0.447_213_6);
        let (px, py) = (sx + dx * 3.0, sy + dy * 3.0);
        add(prim(
            id,
            Vec3::new(px, py, g(px, py) + 2.0),
            Quat::from_rotation_z(dy.atan2(dx)),
            Vec3::new(0.1, 8.0, 4.0),
            boxp,
            te([0.6, 0.8, 1.0, 0.35], 0, false, 0.0),
            ExtraParams::default(),
            "",
        ));
        id += 1;
    }
    // AURORA_DEMO_BLEND_AO: a face ahead of the start position, its left
    // and right halves alike but one blended (alpha 0.99, opaque to the
    // eye), with a nose across the middle, under a wide hat brim and
    // between two curtains of hair that occlude it (issue #882: the blended
    // half took no ambient occlusion, a line down the middle of the head)
    if std::env::var_os("AURORA_DEMO_BLEND_AO").is_some() {
        let (sx, sy) = std::env::var("AURORA_DEMO_POS")
            .ok()
            .and_then(|v| {
                let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                (p.len() >= 2).then(|| (p[0], p[1]))
            })
            .unwrap_or((132.0, 126.0));
        // the start look-at of AgentMovementComplete, (0.8, 0.4)
        let (dx, dy) = (0.894_427_2, 0.447_213_6);
        // to the left of our avatar and above its head, in view of the
        // start camera, the face turned towards it
        let (px, py) = (sx + dx * 4.0 - dy * 1.6, sy + dy * 4.0 + dx * 1.6);
        let head = Vec3::new(px, py, g(px, py) + 2.4);
        let facing = Quat::from_rotation_z(dy.atan2(dx));
        let ahead = Vec3::new(dx, dy, 0.0);
        let side = Vec3::new(-dy, dx, 0.0);
        let skin = |alpha: f32| te([0.92, 0.78, 0.72, alpha], 0, false, 0.0);
        let mut parts = vec![
            (head + side * 0.225, Vec3::new(0.04, 0.45, 0.7), boxp, skin(0.99)),
            (head - side * 0.225, Vec3::new(0.04, 0.45, 0.7), boxp, skin(1.0)),
            (
                head - ahead * 0.06,
                Vec3::splat(0.22),
                shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0),
                skin(1.0),
            ),
            (
                head + Vec3::Z * 0.37,
                Vec3::new(2.4, 2.4, 0.04),
                boxp,
                te([0.15, 0.12, 0.1, 1.0], 0, false, 0.0),
            ),
        ];
        for s in [-1.0, 1.0] {
            let hair = head + side * (s * 0.5) - ahead * 0.25 - Vec3::Z * 0.1;
            parts.push((hair, Vec3::new(0.5, 0.06, 1.0), boxp, te([0.85, 0.82, 0.8, 1.0], 0, false, 0.0)));
        }
        for (pos, scale, volume, entry) in parts {
            add(prim(id, pos, facing, scale, volume, entry, ExtraParams::default(), ""));
            id += 1;
        }
    }
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
    if matches!(particles_mode.as_str(), "region-wind" | "region-calm") {
        ev.push(NetEvent::Wind {
            handle: HANDLE,
            grid: Box::new(aurora_net::terrain::WindGrid {
                velocities: [if particles_mode == "region-wind" {
                    Vec3::new(0.0, -3.0, 0.0)
                } else {
                    Vec3::ZERO
                }; 256],
            }),
        });
    }
    if let Some((wind, _)) = crate::scene::wind_sound::demo_spec() {
        ev.push(NetEvent::Wind {
            handle: HANDLE,
            grid: Box::new(aurora_net::terrain::WindGrid { velocities: [wind; 256] }),
        });
    }

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
        // AURORA_DEMO_VOICE_MODERATION: the estate's voice channel
        let flags = if sim_extras::estate_voice() {
            flags | pf::USE_ESTATE_VOICE_CHAN
        } else {
            flags
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
        // the parcel's cells: the 128 m square around the plaza (x and y
        // 64..192), one bit per 4 m cell (ParcelProperties Bitmap)
        let mut bitmap = vec![0u8; 512];
        for y in 16..48 {
            for x in 16..48 {
                let i = y * 64 + x;
                bitmap[i / 8] |= 1 << (i % 8);
            }
        }
        ev.push(NetEvent::AgentParcel(Arc::new(aurora_net::ParcelInfo {
            local_id: 1,
            name: aurora_i18n::tr!("common-aurora-plaza").into(),
            desc: aurora_i18n::tr!("demo-aurora-viewer-s-offline-demo-parcel").into(),
            // the environment editors' parcel scenarios: the avatar's own
            owner_id: if env::parcel_scenario() { DEMO_AGENT } else { u(100) },
            area: 16384,
            bitmap,
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
    // light shows: the ambient occlusion. With a reflection probe ambiance
    // (4th value, above 0) the sky is no longer classic: it is tone mapped.
    // AURORA_DEMO_SKY=midday: the library Midday's values instead (World ›
    // Environment), a non-classic sky with dense haze and half the sky clouded
    let sky = std::env::var("AURORA_DEMO_SKY").ok().and_then(|v| {
        if v.trim().eq_ignore_ascii_case("midday") {
            return Some(demo_day(midday_sky()));
        }
        let mut values = v.split(',').map(|s| s.trim().parse::<f64>().ok());
        let gamma = values.next()??;
        Some(demo_sky(
            Vec3::new(0.55, 0.35, 0.6).normalize(),
            gamma,
            values.next().flatten(),
            values.next().flatten(),
            values.next().flatten(),
        ))
    });
    if let Some(environment) = sky {
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
            environment,
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
    let inventory_offer = std::env::var("AURORA_DEMO_INVENTORY")
        .is_ok_and(|mode| matches!(mode.as_str(), "offer" | "offer-reject" | "offer-show"))
        || offer_card_scene().is_some();
    if std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| mode == "offer-object-show") {
        ev.push(NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: DEMO_GIVER,
            from_name: aurora_i18n::tr!("demo-gift-dispenser").into(),
            to_agent_id: agent,
            dialog: crate::world::notifications::im::TASK_INVENTORY_OFFERED,
            session_id: DEMO_TASK_OFFER,
            message: aurora_i18n::tr!("received-filing-demo-object").into(),
            offline: false,
            from_group: false,
            binary_bucket: vec![6],
            timestamp: 0,
        }));
    }
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
    if chat_lines_scenario().is_some() {
        ev.extend(chat_lines_events());
    }
    if object_chat::scene().is_some() {
        ev.extend(object_chat::events(agent));
    }
    if let Some(scene) = aurora_scenario() {
        ev.extend(aurora_events(&scene));
    }
    if teleport_offer_scenario() {
        ev.extend(teleport_offer_events(agent));
    }
    if let Some(scene) = lure_scene() {
        ev.extend(lure_events(scene, agent));
    }
    ev.extend(sculpt::events());
    ev.extend(plants::events());
    ev.extend(blend_shadows::events());
    ev.extend(bom_rezzed::events());
    ev
}

/// Unrelated demos stay quiet; each explicit scenario gets only its fixtures.
fn notification_samples(notifications: bool, inventory_offer: bool, menu: Option<&str>) -> Vec<NetEvent> {
    let mut ev = Vec::new();
    let im = |dialog: u8, message: &str, bucket: Vec<u8>| {
        NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: DEMO_GIVER,
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
    // `links` (#898): a menu whose text names residents, a group and a
    // place by their secondlife:///app links, as listing scripts write them
    if menu == "links" {
        let message = format!(
            "{}\n#1 secondlife:///app/agent/{DEMO_FRIEND}/about\n#2 secondlife:///app/agent/{DEMO_GIVER}/about\n\
             #3 secondlife:///app/group/{DEMO_GROUP1}/about\n#4 secondlife:///app/region/Aurora%20D%C3%A9mo/128/128/25",
            aurora_i18n::tr!("demo-pick-a-resident")
        );
        ev.push(NetEvent::ScriptDialog {
            sender: ([127, 0, 0, 1], 13000).into(),
            object_id: Uuid::from_u128(0xF1),
            object_name: aurora_i18n::tr!("demo-resident-picker").into(),
            owner_name: aurora_i18n::tr!("demo-violet-wolf").into(),
            message,
            channel: -4243,
            buttons: ["1", "2", "3", "4"].map(str::to_owned).to_vec(),
        });
        return ev;
    }
    // `scripted`: every window a scripted object sends (llDialog, llTextBox,
    // llRequestPermissions, llLoadURL, llGiveInventory), then four pieces of
    // information after them; the windows stay out until answered
    if menu == "scripted" {
        let sender: std::net::SocketAddr = ([127, 0, 0, 1], 13000).into();
        let owner = aurora_i18n::tr!("demo-violet-wolf");
        ev.push(NetEvent::ScriptDialog {
            sender,
            object_id: Uuid::from_u128(0xF0),
            object_name: aurora_i18n::tr!("demo-fountain").into(),
            owner_name: owner.into(),
            message: aurora_i18n::tr!("demo-choose-a-color-for-the-water").into(),
            channel: -4242,
            buttons: vec![
                aurora_i18n::tr!("demo-purple").into(),
                aurora_i18n::tr!("demo-turquoise").into(),
                aurora_i18n::tr!("demo-amber").into(),
            ],
        });
        ev.push(NetEvent::ScriptDialog {
            sender,
            object_id: Uuid::from_u128(0xF2),
            object_name: aurora_i18n::tr!("demo-land-script-sign").into(),
            owner_name: owner.into(),
            message: aurora_i18n::tr!("demo-scripted-textbox").into(),
            channel: -4244,
            buttons: vec!["!!llTextBox!!".into()],
        });
        ev.push(NetEvent::ScriptQuestion {
            task_id: Uuid::from_u128(0xF3),
            item_id: Uuid::from_u128(0xF4),
            object_name: aurora_i18n::tr!("demo-keywords-object-name").into(),
            owner_name: owner.into(),
            questions: 0x10,
            experience: Uuid::nil(),
            region: 0,
        });
        ev.push(NetEvent::LoadUrl {
            object_name: aurora_i18n::tr!("demo-shop").into(),
            object_id: Uuid::from_u128(0xF5),
            message: aurora_i18n::tr!("demo-scripted-url").into(),
            url: "https://example.invalid/shop".into(),
        });
        ev.push(NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: DEMO_GIVER,
            from_name: aurora_i18n::tr!("demo-gift-dispenser").into(),
            to_agent_id: Uuid::nil(),
            dialog: crate::world::notifications::im::TASK_INVENTORY_OFFERED,
            session_id: Uuid::from_u128(0xF6),
            message: aurora_i18n::tr!("received-filing-demo-object").into(),
            offline: false,
            from_group: false,
            binary_bucket: vec![7],
            timestamp: 0,
        }));
        for info in [
            aurora_i18n::tr!("demo-notifs-paid"),
            aurora_i18n::tr!("demo-notifs-group-notice"),
            aurora_i18n::tr!("demo-group-notice-sandbox"),
            aurora_i18n::tr!("demo-group-notice-rules"),
        ] {
            ev.push(im(crate::world::notifications::im::MESSAGEBOX, info, Vec::new()));
        }
        return ev;
    }
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
        sender: ([127, 0, 0, 1], 13000).into(),
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
        // every scripted window, then the information after them
        let events = notification_samples(false, false, Some("scripted"));
        assert!(matches!(
            events.as_slice(),
            [
                NetEvent::ScriptDialog { .. },
                NetEvent::ScriptDialog { buttons, .. },
                NetEvent::ScriptQuestion { .. },
                NetEvent::LoadUrl { .. },
                NetEvent::InstantMessage(gift),
                ..
            ] if buttons[0] == "!!llTextBox!!" && gift.dialog == crate::world::notifications::im::TASK_INVENTORY_OFFERED
        ));
        assert_eq!(events.len(), 9);
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
        // relaxed, the upload window's default (hands at rest)
        hand_pose: 1,
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

/// `AURORA_DEMO_TYPING=1`: we type in the nearby chat from our arrival on,
/// without a key (our typing animation and our plate's dots).
pub fn typing_scenario() -> bool {
    std::env::var_os("AURORA_DEMO_TYPING").is_some()
}

/// The typing animation (ANIM_AGENT_TYPE), a grid asset the demo has no
/// server for: the arms only, over the stand, forearms forward and the
/// wrists tapping, for our typing in the nearby chat and the crowd's.
pub fn typing_animation() -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, RotKey};
    let joint = |name: &str, keys: Vec<(f32, Quat)>| JointMotion {
        joint_name: name.into(),
        priority: -1,
        rot_keys: keys.into_iter().map(|(time, rotation)| RotKey { time, rotation }).collect(),
        pos_keys: Vec::new(),
    };
    // the upper arms hang (as the stand's), the elbows bend forward about
    // the same lateral axis: -Z for the left arm, +Z for the right
    let (down, bend) = (1.3f32, 1.45f32);
    // a wrist up every other quarter second, the two hands in turn
    let tap = |side: f32, phase: usize| {
        (0..=4usize)
            .map(|i| {
                (
                    i as f32 * 0.25,
                    Quat::from_rotation_z(if (i + phase).is_multiple_of(2) { 0.0 } else { side * 0.25 }),
                )
            })
            .collect::<Vec<_>>()
    };
    aurora_assets::Animation {
        base_priority: 3,
        duration: 1.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 1.0,
        looping: true,
        ease_in: 0.3,
        ease_out: 0.3,
        hand_pose: 0,
        joints: vec![
            joint("mShoulderLeft", vec![(0.0, Quat::from_rotation_x(-down))]),
            joint("mShoulderRight", vec![(0.0, Quat::from_rotation_x(down))]),
            joint("mElbowLeft", vec![(0.0, Quat::from_rotation_z(-bend))]),
            joint("mElbowRight", vec![(0.0, Quat::from_rotation_z(bend))]),
            joint("mWristLeft", tap(1.0, 0)),
            joint("mWristRight", tap(-1.0, 1)),
        ],
    }
}

pub const IDLE_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0001);
/// AURORA_DEMO_ANIMATION_EXPLORER: a worn object's motion, stopped since.
pub const EXPLORER_WAVE_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0130);

pub const ANIMESH_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0100);
pub const ANIMESH_TEXTURE: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0101);
pub const ANIMESH_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0102);
pub const ANIMESH_UNRIGGED_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0103);

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
    let mut objects = animesh_objects(ANIMESH_MESH);
    if std::env::var("AURORA_DEMO_ANIMESH").is_ok_and(|v| matches!(v.as_str(), "alpha" | "alpha-cycle")) {
        make_animesh_invisible(&mut objects);
    }
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

/// Enable after the normal zero-record path has loaded, then exercise
/// cleanup, the non-rigged filter and reactivation without a scene reload.
pub fn animesh_alpha_step(frame: u64) -> Option<Option<bool>> {
    if !std::env::var("AURORA_DEMO_ANIMESH").is_ok_and(|v| v == "alpha-cycle") {
        return None;
    }
    match frame {
        720 | 1440 => Some(Some(true)),
        960 => Some(None),
        1200 => Some(Some(false)),
        _ => None,
    }
}

/// Invisible unrigged mesh children on ground/worn animesh linksets, plus
/// a skinned zero-alpha neighbour for the independent rigged filter.
pub(crate) fn make_animesh_invisible(objects: &mut Vec<ObjectUpdate>) {
    if let Some(te) = objects
        .iter_mut()
        .find(|o| o.local_id == 9202)
        .and_then(|o| o.texture_entry.as_mut())
    {
        for face in &mut Arc::make_mut(te).faces {
            face.color[3] = 0.0;
        }
    }
    for (local, parent) in [(9205, 9200), (9206, 9203)] {
        let sculpt = SculptParams {
            texture: ANIMESH_UNRIGGED_MESH,
            sculpt_type: LL_SCULPT_TYPE_MESH,
        };
        let mut object = prim(
            local,
            Vec3::new(0.2, 0.0, 0.4),
            Quat::IDENTITY,
            Vec3::new(1.7, 0.8, 0.9),
            aurora_prim::VolumeParams {
                sculpt: Some(sculpt),
                ..Default::default()
            },
            te([0.0; 4], 0, false, 0.0),
            ExtraParams {
                sculpt: Some(sculpt),
                ..Default::default()
            },
            "",
        );
        object.parent_id = parent;
        objects.push(object);
    }
}

/// Objects of the animesh demo; the ground root mesh (9200) is `ground_mesh`.
pub(crate) fn animesh_objects(ground_mesh: Uuid) -> Vec<ObjectUpdate> {
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
                appearance_version: 1,
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
pub fn cloud_self_events(shapes: &crate::scene::shape::ShapeParams) -> Vec<NetEvent> {
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
            visual_params: shapes.values_with(&[]),
            cof_version: 1,
            appearance_version: 1,
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
/// back as they were (world/parked.rs). The demo region's simulator still
/// sends its coarse locations at frame 800, as the grid's does while we are
/// its child (`kept` holds them from the departure): they wait with its
/// objects (issue #1000). The events of `frame`, if any.
pub fn away_teleport(frame: u64, world: &crate::world::World, kept: &mut Vec<(Uuid, Vec3)>) -> Option<Vec<NetEvent>> {
    let handle = match frame {
        760 => HANDLE + ((100 * 256u64) << 32),
        800 => {
            std::env::var_os("AURORA_DEMO_TP_AWAY")?;
            return Some(vec![NetEvent::CoarseLocations {
                handle: HANDLE,
                positions: std::mem::take(kept),
            }]);
        }
        960 => HANDLE,
        _ => return None,
    };
    std::env::var_os("AURORA_DEMO_TP_AWAY")?;
    if frame == 760 {
        *kept = world.coarse.get(&HANDLE).cloned().unwrap_or_default();
    }
    let (position, look_at) = (world.agent.position, world.agent.forward());
    Some(vec![
        NetEvent::TeleportFinished { handle, flying: false },
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

/// AURORA_DEMO_MATERIAL_ARRIVALS=<seconds>: the crowd's worn prims and
/// meshes use a legacy material, and a material nobody uses arrives every
/// `seconds` (2 if not a number), as the materials of the avatars coming in
/// do in a busy place; `AURORA_MATERIAL_REBUILD=alt` compares the sync that
/// rebuilt every material user at each arrival (mat_changes.rs).
pub fn material_arrivals() -> Option<f32> {
    let v = std::env::var("AURORA_DEMO_MATERIAL_ARRIVALS").ok()?;
    Some(v.trim().parse::<f32>().ok().filter(|s| *s > 0.0).unwrap_or(2.0))
}

/// The `n`th material arriving with `material_arrivals`: used by nothing.
pub fn arriving_material(n: u32) -> (Uuid, crate::scene::legacy_mat::LegacyMaterial) {
    let (_, m) = legacy_materials().into_iter().next().expect("the demo has legacy materials");
    (Uuid::from_u128(0xDE40_3A70_0000_0000_0001_0000_0000_0000 | n as u128), m)
}

/// The material arriving now, when `material_arrivals`' period has passed
/// since the last one (the first after 5 s, once the crowd is built).
pub fn material_arrival_due() -> Option<(Uuid, crate::scene::legacy_mat::LegacyMaterial)> {
    static NEXT: std::sync::Mutex<Option<(std::time::Instant, u32)>> = std::sync::Mutex::new(None);
    let every = material_arrivals()?;
    let now = std::time::Instant::now();
    let mut next = NEXT.lock().ok()?;
    let (at, n) = next.get_or_insert((now + std::time::Duration::from_secs(5), 0));
    if now < *at {
        return None;
    }
    *at = now + std::time::Duration::from_secs_f32(every);
    *n += 1;
    Some(arriving_material(*n))
}

/// AURORA_DEMO_IMPOSTOR_FLIP=<seconds>: the avatar limit alternates between
/// its setting and one every `seconds` (2 if not a number), so that the
/// crowd's avatars go from impostor to drawn in full and back.
pub fn impostor_flip(limit: usize) -> usize {
    static START: std::sync::OnceLock<Option<(std::time::Instant, f32)>> = std::sync::OnceLock::new();
    let start = START.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_IMPOSTOR_FLIP").ok()?;
        Some((
            std::time::Instant::now(),
            v.trim().parse().ok().filter(|&s: &f32| s > 0.0).unwrap_or(2.0),
        ))
    });
    match start {
        Some((at, every)) if (at.elapsed().as_secs_f32() / every) as u64 % 2 == 1 => 1,
        _ => limit,
    }
}

/// A crowd prim's faces: with `material_arrivals`, a legacy material too.
fn crowd_te(color: [f32; 4], shiny: u8) -> Arc<TextureEntry> {
    let t = te(color, shiny, false, 0.0);
    if material_arrivals().is_none() {
        return t;
    }
    let mut t = (*t).clone();
    for f in &mut t.faces {
        f.material_id = MAT_NONE;
    }
    Arc::new(t)
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
                if material_arrivals().is_some() {
                    f.material_id = MAT_NONE;
                }
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
                crowd_te(color, 1),
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
                    crowd_te(color, 0),
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
/// renderer takes 64, those that reach the view first (#1032).
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
/// texels are fully transparent. AURORA_DEMO_LASHES=1: our avatar also
/// wears a small red ball inside the head shell, blended, on the side the
/// camera sees (eyelashes behind a fringe). AURORA_DEMO_HAIR_CARDS=<n>
/// (with or without AURORA_DEMO_HAIR): our avatar also wears one rigged
/// mesh whose single face holds `n` strand cards hanging around the back of
/// the head, overlapping in a shuffled order, their strands fading softly
/// (#935: the triangles of one face, not drawn back to front).
/// AURORA_DEMO_HAIR_CARDS=<n>,layered: the cards of a hair as it is made:
/// listed from the head outwards, the inner ones dark, the outer ones light,
/// strands mostly opaque with a short soft edge and nothing between them,
/// drawn in the Legacy Viewer's triangle order like a mesh asset (#935);
/// `<n>,layered,outer`: listed from the outside in instead.
/// AURORA_DEMO_SHEER=1 (with or without the others): our avatar also wears
/// a sheer white dress around the torso, its alpha wavering from texel to
/// texel between 0.25 and 0.75 (a knit), and under it a red layer (a body
/// add-on), attached first on the same point so that it is drawn after the
/// dress (#1300).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HairDemo {
    pub layers: u32,
    pub segments: u32,
    pub empty: u32,
    pub period: u32,
    pub lashes: bool,
    pub cards: u32,
    pub layered: bool,
    pub outer_first: bool,
    pub sheer: bool,
}

pub fn hair_demo() -> Option<HairDemo> {
    let cards_var = std::env::var("AURORA_DEMO_HAIR_CARDS").unwrap_or_default();
    let mut cards_args = cards_var.split(',').map(str::trim);
    let cards = cards_args.next().and_then(|v| v.parse::<u32>().ok()).map_or(0, |n| n.clamp(1, 400));
    let cards_args: Vec<&str> = cards_args.collect();
    let layered = cards > 0 && cards_args.contains(&"layered");
    let outer_first = layered && cards_args.contains(&"outer");
    let sheer = std::env::var("AURORA_DEMO_SHEER").is_ok_and(|v| v.trim() == "1");
    let demo = match std::env::var("AURORA_DEMO_HAIR").ok() {
        Some(v) => hair_demo_from(&v),
        None if cards > 0 || sheer => HairDemo {
            layers: 0,
            ..hair_demo_from("")
        },
        None => return None,
    };
    Some(HairDemo {
        lashes: std::env::var("AURORA_DEMO_LASHES").is_ok_and(|v| v.trim() == "1"),
        cards,
        layered,
        outer_first,
        sheer,
        ..demo
    })
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
        lashes: false,
        cards: 0,
        layered: false,
        outer_first: false,
        sheer: false,
    }
}

/// Knit texture of the dress of AURORA_DEMO_SHEER.
pub const TEX_SHEER: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0023);

/// Mesh assets of AURORA_DEMO_SHEER: the dress, and the layer under it.
pub const SHEER_DRESS_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0002_0000_0300);
pub const SHEER_LAYER_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0002_0000_0301);

/// Strand texture of the hair demo.
pub const TEX_HAIR: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0020);

/// Strand texture of the cards of AURORA_DEMO_HAIR_CARDS.
pub const TEX_HAIR_CARD: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0021);

/// Strand texture of `AURORA_DEMO_HAIR_CARDS=<n>,layered`.
pub const TEX_HAIR_CARD_LAYERED: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0022);

/// Mesh asset of AURORA_DEMO_HAIR_CARDS (one face of strand cards).
pub const HAIR_CARDS_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0002_0000_0200);

/// Mesh asset of the ball of AURORA_DEMO_LASHES.
pub const LASHES_MESH: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0002_0000_0100);

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
            // the inner layer attached last (the greatest local id), so
            // that it is drawn first, as a hair made for the Legacy Viewer
            // (blend_order.rs)
            let mut worn = prim(
                200_000 + k as u32 * 64 + (demo.layers - 1 - layer),
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
    if demo.cards > 0 {
        let mesh = SculptParams {
            texture: HAIR_CARDS_MESH,
            sculpt_type: LL_SCULPT_TYPE_MESH,
        };
        let (tint, texture) = if demo.layered {
            ([1.0, 1.0, 1.0, 1.0], TEX_HAIR_CARD_LAYERED)
        } else {
            ([0.55, 0.42, 0.3, 1.0], TEX_HAIR_CARD)
        };
        let mut t = (*te(tint, 0, false, 0.0)).clone();
        for f in &mut t.faces {
            f.texture = texture;
        }
        let mut cards = prim(
            200_050,
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
        cards.parent_id = 9000;
        cards.state = 0x10; // chest
        objects.push(cards);
    }
    if demo.lashes {
        let mesh = SculptParams {
            texture: LASHES_MESH,
            sculpt_type: LL_SCULPT_TYPE_MESH,
        };
        // blended by its color's alpha, nearly opaque
        let mut lashes = prim(
            200_040,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::splat(0.1),
            aurora_prim::VolumeParams {
                sculpt: Some(mesh),
                ..Default::default()
            },
            te([1.0, 0.1, 0.1, 0.95], 0, true, 0.0),
            ExtraParams {
                sculpt: Some(mesh),
                ..Default::default()
            },
            "",
        );
        lashes.parent_id = 9000;
        lashes.state = 0x10; // chest
        objects.push(lashes);
    }
    if demo.sheer {
        // the layer attached first (the smaller local id): drawn after the
        // dress, as a body add-on worn before the clothes over it
        for (local, mesh, color, texture) in [
            (200_060, SHEER_LAYER_MESH, [0.85, 0.12, 0.15, 0.95], None),
            (200_061, SHEER_DRESS_MESH, [1.0, 1.0, 1.0, 1.0], Some(TEX_SHEER)),
        ] {
            let mesh = SculptParams {
                texture: mesh,
                sculpt_type: LL_SCULPT_TYPE_MESH,
            };
            let mut t = (*te(color, 0, false, 0.0)).clone();
            if let Some(texture) = texture {
                for f in &mut t.faces {
                    f.texture = texture;
                }
            }
            let mut worn = prim(
                local,
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
            worn.parent_id = 9000;
            worn.state = 0x10; // chest
            objects.push(worn);
        }
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// AURORA_DEMO_FEET=opaque|blend: our avatar and each one of the crowd
/// (AURORA_DEMO_CROWD) wear a shoe on each foot, a box on the foot's
/// attachment point, opaque or blended (its color's alpha 0.95, as the
/// many mesh bodies and shoes whose faces are blended though they look
/// opaque): what the contact shadows see of what touches the ground. Two
/// blocks also stand on the ground in front of the start position of
/// AURORA_DEMO_POS="129,134", left of the view: an opaque one and, further
/// left, a blended one; nearer, a hoof on the ground and a shoe hovering
/// 2 cm above it (the thick casters of issue 988).
pub fn feet_demo() -> Option<bool> {
    match std::env::var("AURORA_DEMO_FEET").ok()?.trim() {
        "blend" => Some(true),
        "opaque" => Some(false),
        _ => None,
    }
}

/// AURORA_DEMO_SUN=<azimuth>,<elevation> (degrees, azimuth 0 east, 90
/// north): the direction towards the sun for the lighting and the shadows,
/// the sky keeping its own. The demo camera looks east, so `0,10` puts a low
/// sun in front of it, the light that shows the contact shadows' outlines
/// under thick casters (issue 988).
pub fn sun_override() -> Option<Vec3> {
    static SUN: std::sync::OnceLock<Option<Vec3>> = std::sync::OnceLock::new();
    *SUN.get_or_init(|| parse_sun(&std::env::var("AURORA_DEMO_SUN").ok()?))
}

fn parse_sun(v: &str) -> Option<Vec3> {
    let mut it = v.split(',').map(|s| s.trim().parse::<f32>().ok());
    let (azimuth, elevation) = (it.next()??.to_radians(), it.next()??.to_radians());
    Some(Vec3::new(
        elevation.cos() * azimuth.cos(),
        elevation.cos() * azimuth.sin(),
        elevation.sin(),
    ))
}

/// The shoes of [`feet_demo`] (`blend`: blended faces).
pub fn feet_events(blend: bool, crowd: u32) -> Vec<NetEvent> {
    let alpha = if blend { 0.95 } else { 1.0 };
    let mut objects = Vec::new();
    let avatars = std::iter::once(9000).chain((0..crowd).map(|k| 100_000 + k * 64));
    for (k, avatar_id) in avatars.enumerate() {
        // left foot, right foot
        for (side, point) in [7u8, 8].into_iter().enumerate() {
            let mut shoe = prim(
                210_000 + k as u32 * 4 + side as u32,
                Vec3::new(0.0, 0.04, -0.05),
                Quat::IDENTITY,
                Vec3::new(0.12, 0.26, 0.1),
                aurora_prim::VolumeParams::default(),
                te([0.25, 0.12, 0.35, alpha], 0, false, 0.0),
                ExtraParams::default(),
                "",
            );
            shoe.parent_id = avatar_id;
            shoe.state = point;
            objects.push(shoe);
        }
    }
    for (i, (x, y, a)) in [(130.6, 137.4, 1.0), (131.4, 138.8, 0.95)].into_iter().enumerate() {
        objects.push(prim(
            210_900 + i as u32,
            Vec3::new(x, y, floor_at(x, y) + 0.3),
            Quat::IDENTITY,
            Vec3::new(0.3, 0.3, 0.6),
            aurora_prim::VolumeParams::default(),
            te([0.25, 0.12, 0.35, a], 0, false, 0.0),
            ExtraParams::default(),
            "",
        ));
    }
    // thick casters about a foot's size nearer the camera (issue 988): a
    // hoof standing on the ground and a shoe hovering 2 cm above it, blended
    // as the shoes are
    let hoof = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0);
    let shoe = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    for (i, (x, y, lift, size, volume)) in [
        (130.2, 136.3, 0.0, Vec3::new(0.14, 0.14, 0.1), hoof),
        (130.3, 135.9, 0.02, Vec3::new(0.26, 0.12, 0.1), shoe),
    ]
    .into_iter()
    .enumerate()
    {
        objects.push(prim(
            210_902 + i as u32,
            Vec3::new(x, y, floor_at(x, y) + lift + size.z * 0.5),
            Quat::IDENTITY,
            size,
            volume,
            te([0.25, 0.12, 0.35, alpha], 0, false, 0.0),
            ExtraParams::default(),
            "",
        ));
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

/// Pixels of [`TEX_HAIR_CARD`] (RGBA8, 256×256): eight strands across,
/// each 0.95 opaque in its middle and fading over many texels on both
/// sides (most of the texture is nearly transparent, never quite empty
/// near a strand), thinning from the roots (top) to the tips.
fn hair_card_texture() -> (Vec<u8>, u32, u32) {
    const N: u32 = 256;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        let fade = 1.0 - 0.75 * y as f32 / (N - 1) as f32;
        for x in 0..N {
            let d = (x % 32) as f32 - 15.5;
            let alpha = 0.95 * (-(d / 6.0) * (d / 6.0)).exp() * fade;
            let tone = 0.8 + 0.2 * (((x / 32) * 37 % 11) as f32 / 10.0);
            px.extend_from_slice(&[
                (200.0 * tone) as u8,
                (150.0 * tone) as u8,
                (100.0 * tone) as u8,
                (255.0 * alpha) as u8,
            ]);
        }
    }
    (px, N, N)
}

/// Pixels of [`TEX_HAIR_CARD_LAYERED`] (RGBA8, 256×256), as the strand
/// texture of a mesh hair: the left half dark blue strands (the inner cards,
/// so that one showing over an outer card stands out), the right half
/// near-white ones (the outer cards), 16 texels apart, each 88 %
/// opaque across most of its width with a soft edge of about three texels
/// and nothing between strands, thinning from the roots (top) to the tips.
fn hair_card_layered_texture() -> (Vec<u8>, u32, u32) {
    const N: u32 = 256;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        let fade = 1.0 - 0.5 * y as f32 / (N - 1) as f32;
        for x in 0..N {
            let d = ((x % 16) as f32 - 7.5).abs();
            let alpha = 0.88 * ((6.5 - d) / 3.0).clamp(0.0, 1.0) * fade;
            let tone = 0.85 + 0.15 * (((x / 16) * 37 % 11) as f32 / 10.0);
            let rgb = if x < N / 2 { [20.0, 30.0, 140.0] } else { [255.0, 250.0, 245.0] };
            px.extend_from_slice(&[
                (rgb[0] * tone) as u8,
                (rgb[1] * tone) as u8,
                (rgb[2] * tone) as u8,
                (255.0 * alpha) as u8,
            ]);
        }
    }
    (px, N, N)
}

/// Pixels of [`TEX_SHEER`] (RGBA8, 256×256): a white knit whose alpha
/// wavers between 0.25 and 0.75 from one 4-texel stitch to the next, so
/// that it crosses one half everywhere, even through the coarser mips.
fn sheer_texture() -> (Vec<u8>, u32, u32) {
    const N: u32 = 256;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let stitch = (x / 4).wrapping_mul(73_856_093) ^ (y / 4).wrapping_mul(19_349_663);
            let h = stitch.wrapping_mul(2_654_435_761) >> 24;
            let alpha = 0.25 + 0.5 * h as f32 / 255.0;
            px.extend_from_slice(&[245, 243, 240, (255.0 * alpha) as u8]);
        }
    }
    (px, N, N)
}

/// The procedural texture `id` takes a mip chain (the others are the
/// 64-texel test panels, kept as they are).
pub fn local_texture_mips(id: &Uuid) -> bool {
    *id == TEX_HAIR
        || *id == TEX_HAIR_CARD
        || *id == TEX_HAIR_CARD_LAYERED
        || *id == TEX_SHEER
        || *id == TEX_PROJECTED
        || *id == TEX_FOLIAGE
        || plants::is_texture(id)
}

#[cfg(test)]
mod sun_tests {
    use super::*;

    #[test]
    fn demo_sun_is_east_at_azimuth_zero_and_rises_with_elevation() {
        let east = parse_sun("0, 10").expect("two angles");
        assert!(east.x > 0.98 && east.y.abs() < 1e-6 && (east.z - 10f32.to_radians().sin()).abs() < 1e-6);
        let north = parse_sun("90,0").expect("two angles");
        assert!(north.x.abs() < 1e-6 && (north.y - 1.0).abs() < 1e-6);
        assert!((parse_sun("200,35").expect("two angles").length() - 1.0).abs() < 1e-6);
        assert_eq!(parse_sun("10"), None);
        assert_eq!(parse_sun("east,10"), None);
    }
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

    /// The strands of `AURORA_DEMO_HAIR_CARDS=<n>,layered` are a blended
    /// texture, most of each strand at least half covered (drawn first,
    /// `RiggedBlendDepth::Split`) and a soft edge under it.
    #[test]
    fn layered_card_strands_are_blended_mostly_half_covered() {
        let (px, w, h) = hair_card_layered_texture();
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert_eq!(classify_alpha(&px), AlphaKind::Blend);
        // the top row: the roots, at full strength
        let row = &px[..(w * 4) as usize];
        let alphas: Vec<u8> = row.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        let half = alphas.iter().filter(|&&a| a >= 128).count();
        let soft = alphas.iter().filter(|&&a| a > 0 && a < 128).count();
        assert!(half > soft && soft > 0, "{half} half covered, {soft} soft");
        assert!(alphas.contains(&0));
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

pub const SYNC_LEADER_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0021);
pub const SYNC_DELAYED_ANIM: Uuid = Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0022);

pub fn animation_sync_demo() -> bool {
    std::env::var("AURORA_DEMO_ANIM_LOOP").is_ok_and(|v| matches!(v.as_str(), "sync" | "sync-edit"))
}

pub fn animation_edit_demo() -> bool {
    std::env::var("AURORA_DEMO_ANIM_LOOP").is_ok_and(|v| v == "sync-edit")
}

pub fn animation_resync_demo() -> bool {
    std::env::var("AURORA_DEMO_ANIM_LOOP").is_ok_and(|v| v == "resync")
}

/// Sequence changes every 120 frames; a real stop/restart every 1200.
/// In sync mode the second avatar's asset is withheld until frame 300.
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
                let id = if animation_sync_demo() || animation_resync_demo() {
                    if avatar == DEMO_AGENT {
                        SYNC_LEADER_ANIM
                    } else {
                        SYNC_DELAYED_ANIM
                    }
                } else {
                    IDLE_ANIM
                };
                vec![(id, (frame / 120 + 1) as i32)]
            },
        })
        .collect()
}

/// Offline voice: the other avatar talks in bursts; our own outline follows
/// the transmitted microphone. AURORA_DEMO_VOICE exercises both tags without
/// microphone input: speaking / loud / listening / off / cycle (4 s / 3 s / 2 s).
/// `sweep` keeps the real microphone here and makes a plate of the name
/// plates scene go from a whisper to a shout (`demo::tags::voice`).
pub fn voice_levels(
    t: f64,
    me: Uuid,
    enabled: bool,
    talking: bool,
    mic_level: f32,
    mode: Option<&str>,
) -> std::collections::HashMap<Uuid, (f32, bool)> {
    let mut m = std::collections::HashMap::new();
    // Menu fixtures use the ordinary per-frame participant list. A one-shot
    // calls.voice_peers insertion would be replaced before the menu is drawn.
    if matches!(
        object_menu::mode(),
        Some("avatar-menu" | "attachment-menu" | "customize-avatar" | "customize-other-attachment")
    ) {
        m.insert(DEMO_NOVA, (0.0, false));
    }
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
        participant_level(mic_level)
    } else {
        0.0
    };
    m.insert(me, (mine, mine > aurora_voice::SPEAKING_AUDIO_LEVEL));
    m
}

/// The microphone meter's level (the tuning scale of `MicCapture::level`:
/// 0.8 at full scale, 0.01 a decibel) on the participant scale our own
/// level has on the grid (`0.18 + 0.005 × dB` of a 16-bit RMS, about 0.63
/// at full scale, aurora_voice::transmit), both over the same 300 ms: the
/// demo's own tag then lights up, and turns red, at the same loudness.
fn participant_level(tuning: f32) -> f32 {
    if tuning <= 0.0 {
        return 0.0;
    }
    crate::ui::nameplate::level_at_dbfs((tuning - 0.8) * 100.0).max(0.0)
}

#[cfg(test)]
mod voice_tests {
    use super::*;

    #[test]
    fn own_demo_tag_respects_voice_and_transmission() {
        assert!(voice_levels(1.0, DEMO_AGENT, false, true, 0.6, None).is_empty());
        assert_eq!(voice_levels(1.0, DEMO_AGENT, true, false, 0.6, None)[&DEMO_AGENT], (0.0, false));
        let (level, speaking) = voice_levels(1.0, DEMO_AGENT, true, true, 0.6, None)[&DEMO_AGENT];
        assert!((level - participant_level(0.6)).abs() < 1e-6 && speaking, "{level}");
    }

    #[test]
    fn a_real_shout_on_the_microphone_turns_our_plate_red_and_normal_speech_does_not() {
        use crate::ui::nameplate::overdrive;
        // the meter at full scale is the participant level at full scale
        assert!((participant_level(0.8) - 0.6315).abs() < 1e-4);
        // the meter's levels (0.01 a dB, 0.8 at full scale) of a voice
        let at = |dbfs: f32| participant_level(0.8 + dbfs / 100.0);
        let speaking = |dbfs: f32| at(dbfs) > aurora_voice::SPEAKING_AUDIO_LEVEL;
        // silence and room noise: no voice; a whisper speaks
        assert_eq!(participant_level(0.0), 0.0);
        assert!(!speaking(-70.0) && speaking(-45.0));
        // normal speech, even raised: green
        for dbfs in [-30.0, -27.0, -24.0, -20.0] {
            assert_eq!(overdrive(at(dbfs)), 0.0, "{dbfs} dBFS");
        }
        // a shout turns red on the way, a hard shout fully
        assert!(overdrive(at(-15.0)) > 0.4, "{}", overdrive(at(-15.0)));
        assert_eq!(overdrive(at(-11.0)), 1.0);
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
/// Packed occlusion-roughness-metal map of AURORA_DEMO_PBR_METAL: occlusion
/// (red) 0 but for a centred disc, roughness 0.1, metallic 1.
pub const TEX_ORM_DISC: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0008);
/// Foliage of AURORA_DEMO_FOLIAGE (256 × 256, alpha a mask): thin blades
/// and small leaves, about a sixth of the texels drawn.
pub const TEX_FOLIAGE: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0009);

#[cfg(test)]
mod foliage_tests {
    use super::*;
    use crate::scene::jobs::{AlphaKind, classify_alpha};
    use aurora_render::{MASK_CUTOFF, alpha_coverage, build_mips, preserve_alpha_coverage};

    #[test]
    fn the_demo_foliage_is_a_mask_that_keeps_its_coverage() {
        let (px, w, h) = foliage_texture();
        assert_eq!(classify_alpha(&px), AlphaKind::Mask);
        let plain = build_mips(w, h, px, 16);
        let mut kept = plain.clone();
        preserve_alpha_coverage(&mut kept, MASK_CUTOFF);
        let target = alpha_coverage(&plain[0].2, MASK_CUTOFF);
        let row = |m: &[(u32, u32, Vec<u8>)]| {
            m.iter()
                .map(|(_, _, d)| format!("{:.3}", alpha_coverage(d, MASK_CUTOFF)))
                .collect::<Vec<_>>()
                .join(" ")
        };
        eprintln!("foliage coverage per level, plain: {}", row(&plain));
        eprintln!("foliage coverage per level, kept:  {}", row(&kept));
        // plain mips lose it a few levels down; kept ones hold it to 8 × 8
        // (within what the alpha values of a level allow: 2-texel blades are
        // 128 or 255 one level down)
        assert!(alpha_coverage(&plain[4].2, MASK_CUTOFF) < target * 0.5);
        for (w, _, d) in kept.iter().take_while(|(w, _, _)| *w >= 8) {
            assert!(
                (alpha_coverage(d, MASK_CUTOFF) - target).abs() < 0.08,
                "{w}: {}",
                alpha_coverage(d, MASK_CUTOFF)
            );
        }
    }
}

/// Blades of grass and small leaves on a transparent ground: the thin cut
/// out shapes that thin out with the distance when their mips are averaged.
fn foliage_texture() -> (Vec<u8>, u32, u32) {
    const N: u32 = 256;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32, y as f32);
            // blades: 2 texels wide, leaning, one every 12 texels
            let lean = fx + fy * 0.18;
            let blade = (lean % 12.0) < 2.0 && fy > (x * 37 % 97) as f32 * 0.6;
            // leaves: small ellipses on a 24-texel grid, shifted every row
            let (cx, cy) = ((fx + (y / 24 % 2) as f32 * 12.0) % 24.0 - 12.0, fy % 24.0 - 12.0);
            let leaf = (cx / 5.0).powi(2) + (cy / 2.5).powi(2) < 1.0;
            let on = blade || leaf;
            let g = 120 + ((x * 7 + y * 3) % 60) as u8;
            px.extend_from_slice(&[40, g, 52, if on { 255 } else { 0 }]);
        }
    }
    (px, N, N)
}
const MAT_NONE: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0001);
const MAT_MASK: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0002);
const MAT_EMISSIVE: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0003);
const MAT_BUMPY: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0004);
const MAT_BLEND: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0005);
/// Mask with a cutoff of 0 (AURORA_DEMO_MASK_ZERO): every texel drawn.
const MAT_MASK_ZERO: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0006);
/// Glossy plate of AURORA_DEMO_SHIMMER: the bumps repeated many times.
const MAT_SHIMMER: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0020);
const TEX_PICK_ZERO: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0012);
const TEX_PICK_FAINT: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0013);
const TEX_PICK_HALF: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0014);

/// Size of a test panel; AURORA_DEMO_BUMP=tall or wide stretches the
/// normal-mapped one 4:1, its texture four times denser across or along
/// it (the tangent frame of `perturb_normal`, AURORA_TANGENTS).
fn panel_scale(k: usize, label: &str) -> Vec3 {
    let bumpy = k == 5 && label == aurora_i18n::tr!("demo-normal-specular");
    match std::env::var("AURORA_DEMO_BUMP").as_deref() {
        Ok("tall") if bumpy => Vec3::new(0.06, 0.45, 1.8),
        Ok("wide") if bumpy => Vec3::new(0.06, 1.8, 0.45),
        _ => Vec3::new(0.06, 1.8, 1.8),
    }
}

/// (texture, legacy material, hover text) of the test panels;
/// AURORA_DEMO_MASK_ZERO=1 gives the holed one a Mask material with a
/// cutoff of 0, drawn whole like the Legacy Viewer.
fn alpha_panels() -> [(Uuid, Option<Uuid>, &'static str); 6] {
    let holes = if std::env::var_os("AURORA_DEMO_MASK_ZERO").is_some() {
        (TEX_HOLES, Some(MAT_MASK_ZERO), aurora_i18n::tr!("demo-mask-zero-cutoff"))
    } else {
        (TEX_HOLES, None, aurora_i18n::tr!("demo-auto-mask"))
    };
    [
        (TEX_GRADIENT, None, aurora_i18n::tr!("demo-auto-blend")),
        (TEX_GRADIENT, Some(MAT_NONE), aurora_i18n::tr!("demo-alpha-mode-none")),
        (TEX_GRADIENT, Some(MAT_MASK), aurora_i18n::tr!("demo-mask-50-cutoff")),
        (TEX_GRADIENT, Some(MAT_EMISSIVE), aurora_i18n::tr!("common-emissive-mask")),
        holes,
        (BLANK_TEXTURE, Some(MAT_BUMPY), aurora_i18n::tr!("demo-normal-specular")),
    ]
}

/// Pixels of a procedural demo texture (RGBA8, 64×64; the hair demo's 256×256).
pub fn local_texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if let Some(pixels) = texture_library::texture(id) {
        return Some(pixels);
    }
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
    if *id == TEX_HAIR_CARD_LAYERED {
        return Some(hair_card_layered_texture());
    }
    if *id == TEX_SHEER {
        return Some(sheer_texture());
    }
    if *id == TEX_HAIR_CARD {
        return Some(hair_card_texture());
    }
    if *id == TEX_FOLIAGE {
        return Some(foliage_texture());
    }
    if let Some(species) = plants::texture(id) {
        return Some(species);
    }
    if let Some(placeholder) = bom_rezzed::texture(id) {
        return Some(placeholder);
    }
    if let Some(pane) = blend_shadows::texture(id) {
        return Some(pane);
    }
    if let Some(picture) = tags::picture_pixels(id) {
        return Some(picture);
    }
    if let Some(snapshot) = pick_snapshot(id) {
        return Some(snapshot);
    }
    if let Some(bake) = bake_texture(id) {
        return Some(bake);
    }
    if let Some(sky) = sky::texture(id) {
        return Some(sky);
    }
    if let Some(baked) = tangents::texture(id) {
        return Some(baked);
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
            } else if *id == TEX_ORM_DISC {
                let (dx, dy) = (fx - 31.5, fy - 31.5);
                [if dx * dx + dy * dy < 256.0 { 255 } else { 0 }, 26, 255, 255]
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

/// White metals of AURORA_DEMO_PBR_METAL, polished to rough.
const MAT_PBR_METAL: [Uuid; 3] = [
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0011),
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0012),
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0013),
];

/// Polished metal cube of AURORA_DEMO_PBR_METAL, occluded but for a disc.
const MAT_PBR_ORM_DISC: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0014);

/// Orange emissive spheres of AURORA_DEMO_PBR_EMISSIVE: opaque, alpha-masked,
/// blended.
const MAT_PBR_EMISSIVE: [Uuid; 3] = [
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0030),
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0031),
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0032),
];

/// PBR materials of the demo (normally fetched as assets).
pub fn pbr_materials() -> Vec<(Uuid, aurora_assets::PbrMaterial)> {
    let mut list = vec![(
        MAT_PBR_TILES,
        aurora_assets::PbrMaterial {
            base_color_texture: Some(TEX_TILES),
            normal_texture: Some(TEX_BUMPS),
            metallic_factor: 0.0,
            roughness_factor: 0.6,
            ..Default::default()
        },
    )];
    for (mat, roughness) in MAT_PBR_METAL.into_iter().zip([0.05, 0.3, 0.6]) {
        list.push((
            mat,
            aurora_assets::PbrMaterial {
                metallic_factor: 1.0,
                roughness_factor: roughness,
                ..Default::default()
            },
        ));
    }
    list.push((
        MAT_PBR_ORM_DISC,
        aurora_assets::PbrMaterial {
            metallic_roughness_texture: Some(TEX_ORM_DISC),
            metallic_factor: 1.0,
            roughness_factor: 1.0,
            ..Default::default()
        },
    ));
    for (mat, (alpha_mode, alpha)) in MAT_PBR_EMISSIVE.into_iter().zip([
        (aurora_assets::AlphaMode::Opaque, 1.0),
        (aurora_assets::AlphaMode::Mask, 1.0),
        (aurora_assets::AlphaMode::Blend, 0.6),
    ]) {
        list.push((
            mat,
            aurora_assets::PbrMaterial {
                base_color_factor: [0.08, 0.08, 0.08, alpha],
                metallic_factor: 0.0,
                roughness_factor: 0.5,
                emissive_factor: [1.0, 0.45, 0.1],
                alpha_mode,
                alpha_cutoff: 0.5,
                ..Default::default()
            },
        ));
    }
    list.push((
        region::terrain_replacement(),
        aurora_assets::PbrMaterial {
            base_color_factor: [0.035, 0.3, 0.055, 1.0],
            metallic_factor: 0.0,
            roughness_factor: 0.9,
            ..Default::default()
        },
    ));
    list.push((
        region::terrain_mask(),
        aurora_assets::PbrMaterial {
            base_color_texture: Some(TEX_HOLES),
            normal_texture: Some(TEX_BUMPS),
            metallic_factor: 0.0,
            roughness_factor: 0.7,
            alpha_mode: aurora_assets::AlphaMode::Mask,
            alpha_cutoff: 0.5,
            ..Default::default()
        },
    ));
    list
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
        (tangents::MAT, tangents::material()),
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
            MAT_MASK_ZERO,
            LegacyMaterial {
                diffuse_alpha_mode: alpha_mode::MASK,
                alpha_cutoff: 0,
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
                ..base.clone()
            },
        ),
        (
            MAT_SHIMMER,
            LegacyMaterial {
                normal_map: TEX_BUMPS,
                normal_st: [48.0, 18.0, 0.0, 0.0],
                specular_exp: 245,
                env_intensity: 60,
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
fn demo_sky(sun: Vec3, gamma: f64, haze_distance: Option<f64>, ambient: Option<f64>, probe_ambiance: Option<f64>) -> Llsd {
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
    if let (Some(a), Llsd::Map(m)) = (probe_ambiance, &mut sky) {
        m.insert("reflection_probe_ambiance".into(), Llsd::Real(a));
    }
    demo_day(sky)
}

/// The library Midday sky (AURORA_DEMO_SKY=midday): what the World ›
/// Environment menu applies, a non-classic sky (probe ambiance 1) without
/// ambient, with a strong haze, half the sky in clouds and a bluish sun.
fn midday_sky() -> Llsd {
    use aurora_llsd::llsd_map;
    let arr = |v: &[f64]| Llsd::Array(v.iter().map(|x| Llsd::Real(*x)).collect());
    llsd_map! {
        "type" => "sky",
        "sun_rotation" => arr(&[0.171733, -0.481749, 0.288542, 0.809425]),
        "sunlight_color" => arr(&[1.74118, 1.74118, 1.92941]),
        "gamma" => 1.0,
        "reflection_probe_ambiance" => 1.0,
        "max_y" => 2272.7,
        "glow" => arr(&[5.0, 0.001, -0.48]),
        "cloud_color" => arr(&[0.541176, 0.541176, 0.541176]),
        "cloud_pos_density1" => arr(&[0.73, 0.71, 0.26]),
        "cloud_pos_density2" => arr(&[0.62, 0.49, 0.89]),
        "cloud_scale" => 0.75,
        "cloud_scroll_rate" => arr(&[0.0, 0.0]),
        "cloud_shadow" => 0.54,
        "cloud_variance" => 0.0,
        "star_brightness" => 0.0,
        "legacy_haze" => llsd_map! {
            "ambient" => arr(&[0.0, 0.0, 0.0]),
            "blue_density" => arr(&[0.1792, 0.3696, 0.9408]),
            "blue_horizon" => arr(&[0.345098, 0.345098, 0.345098]),
            "density_multiplier" => 0.0001126,
            "distance_multiplier" => 6.54,
            "haze_density" => 0.78,
            "haze_horizon" => 0.25,
        },
    }
}

/// Day cycle with a single sky frame (and the default water).
fn demo_day(sky: Llsd) -> Llsd {
    use aurora_llsd::llsd_map;
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
    fn particle_wind_demo_emitters_only_differ_by_the_wind_mask() {
        use aurora_prim::particles as ps;
        let parse = |wind| {
            let Some(aurora_net::objects::ParticleUpdate::Set(block)) = compressed_particle_block(&region_wind_particles(wind), true)
            else {
                panic!("compressed particle update");
            };
            ps::parse(&block).expect("region wind particle data")
        };
        let mut wind = parse(true);
        let calm = parse(false);
        assert_eq!(wind.part.max_age, 6.0);
        assert_ne!(wind.part.flags & ps::LL_PART_WIND_MASK, 0);
        wind.part.flags &= !ps::LL_PART_WIND_MASK;
        assert_eq!(wind, calm);
    }

    #[test]
    fn demo_leash_is_a_sagging_ribbon_towards_its_target() {
        use aurora_prim::particles as ps;
        let target = Uuid::from_u128(1049);
        let d = ps::parse(&leash_particles(target)).expect("leash particle data");
        assert_eq!(d.pattern, ps::LL_PART_SRC_PATTERN_DROP);
        assert_eq!(d.burst_rate, ps::MIN_BURST_RATE);
        assert_eq!(d.burst_part_count, 2);
        assert_eq!(d.part_accel, Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(d.target_id, target);
        assert_eq!(d.part.flags & ps::LL_PART_RIBBON_MASK, ps::LL_PART_RIBBON_MASK);
        assert_eq!(d.part.max_age, 3.5);
        assert_eq!(d.part.start_scale, glam::Vec2::new(0.0625, 2.5));
    }

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
