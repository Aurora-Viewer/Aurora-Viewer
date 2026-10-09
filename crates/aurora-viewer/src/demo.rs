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

/// EEP environment answers (AURORA_DEMO_EEP_PARCEL).
#[path = "demo_eep.rs"]
pub mod eep;
/// About Land answers (AURORA_DEMO_LAND).
#[path = "demo_land.rs"]
pub mod land;

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

fn height(x: f32, y: f32) -> f32 {
    let hill = 14.0 * (-((x - 190.0).powi(2) + (y - 170.0).powi(2)) / 2500.0).exp();
    let dune = 2.5 * (x / 23.0).sin() * (y / 31.0).cos();
    // a beach and a lagoon to the west (water at 20 m)
    let shore = ((x - 75.0) / 9.0).clamp(-6.0, 9.0);
    21.5 + shore + hill + dune
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

fn demo_raw() -> Llsd {
    use aurora_llsd::llsd_map;
    let folder = |id: u128, parent: u128, name: &str, t: i32| {
        llsd_map! {"folder_id" => u(id), "parent_id" => u(parent), "name" => name, "type_default" => t, "version" => 1}
    };
    let mut skel = Vec::new();
    skel.push(folder(1, 0, "Mon inventaire", 8));
    for (i, (name, t)) in [
        ("Animations", 20),
        ("Habits", 5),
        ("Repères", 3),
        ("Notes", 7),
        ("Objets", 6),
        ("Scripts", 10),
        ("Textures", 0),
        ("Corbeille", 14),
        ("Mes tenues", 48),
        ("Materials", 57),
    ]
    .iter()
    .enumerate()
    {
        skel.push(folder(10 + i as u128, 1, name, *t));
    }
    let buddies: Vec<Llsd> = (0..4)
        .map(|i| llsd_map! {"buddy_id" => u(100 + i), "buddy_rights_given" => [1, 3, 7, 0][i as usize], "buddy_rights_has" => if i % 2 == 0 { 3 } else { 1 }})
        .collect();
    llsd_map! {
        "inventory-root" => Llsd::Array(vec![llsd_map!{"folder_id" => u(1)}]),
        "inventory-skeleton" => Llsd::Array(skel),
        "buddy-list" => Llsd::Array(buddies),
        "classified_categories" => Llsd::Array(
            [(1, "Shopping"), (2, "Land Rental"), (3, "Property Rental"), (4, "Special Attraction")]
                .into_iter()
                .map(|(id, name)| llsd_map! {"category_id" => id, "category_name" => name})
                .collect(),
        ),
    }
}

/// Fake replies for name / inventory requests in demo mode.
/// Demo residents and groups (block list and group chat).
pub const DEMO_AGENT: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
pub const DEMO_LOUP: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002);
pub const DEMO_NOVA: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0003);
pub const DEMO_TESS: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0004);
pub const DEMO_GROUP1: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0001);
pub const DEMO_GROUP2: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0002);
/// Hidden from the profile (Contacts › Groupes shows it in the other color).
pub const DEMO_GROUP3: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0003);

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
            name: "Loups du Nord".into(),
            insignia: Uuid::nil(),
        },
    ];
    let mut p = aurora_net::AvatarProfile {
        id,
        sl_about: "Résident de la démo hors-ligne d'Aurora Viewer 🐺✨".into(),
        born: date(2019, 4, 2),
        hide_age: Some(false),
        online: Some(true),
        flags: flags::IDENTIFIED,
        notes: Some(String::new()),
        ..Default::default()
    };
    if id == DEMO_AGENT {
        p.sl_about = "Mon profil de démonstration.
Il se modifie ici, l'enregistrement reste local."
            .into();
        p.fl_about = "Développeur d'Aurora Viewer.".into();
        p.born = date(2012, 3, 14);
        p.flags = flags::TRANSACTED | flags::ALLOW_PUBLISH;
        p.customer_type = "Premium".into();
        p.groups = groups;
        p.picks = vec![(Uuid::from_u128(0x91C0_0001), "La plage d'Aurora".into())];
    } else if id == DEMO_LOUP {
        p.sl_about = format!(
            "Loup solitaire, bâtisseur du dimanche.

Ma partenaire : secondlife:///app/agent/{DEMO_NOVA}/about
Mon site : https://example.com/loup 🐺"
        );
        p.fl_about = "Passionné de montagne.".into();
        p.partner = DEMO_NOVA;
        p.born = date(2007, 5, 12);
        p.flags = flags::TRANSACTED;
        p.customer_type = "Lifetime".into();
        p.groups = groups;
        p.picks = vec![
            (Uuid::from_u128(0x91C0_0001), "La plage d'Aurora".into()),
            (Uuid::from_u128(0x91C0_0002), "Mon atelier".into()),
        ];
        p.notes = Some("Rencontré à la plage, très sympa.".into());
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
    cs.add_set("Famille");
    cs.set_color("Famille", [0.72, 0.52, 1.0, 1.0]);
    cs.add_to_set(&[u(100), u(102), DEMO_LOUP], "Famille", is_friend);
    cs.add_set("Travail");
    cs.set_color("Travail", [0.35, 0.85, 0.8, 1.0]);
    cs.add_to_set(&[u(101), u(102)], "Travail", is_friend);
    cs.set_pseudonym(&[u(102)], "Orion le Grand", is_friend);
    cs.remove_display_name(&[u(103)], is_friend);
}

pub fn demo_reply(cmd: &aurora_net::NetCommand) -> Vec<NetEvent> {
    use aurora_net::inventory::{FolderContents, InvItem};
    match cmd {
        aurora_net::NetCommand::AgentAnimation { anim, start: false } if *anim == SEAT_ANIM => {
            vec![NetEvent::AvatarAnimations {
                avatar: DEMO_AGENT,
                anims: vec![(IDLE_ANIM, 1)],
                sources: Vec::new(),
            }]
        }
        aurora_net::NetCommand::Land(l) => land::reply(l),
        aurora_net::NetCommand::RequestNames(ids) => {
            let names = ["Nova Exemple", "Pixel Boutique", "Orion Exemple", "Tess Touch"];
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
                ("Nova", "Exemple", "Nova ♪"),
                ("pixelboutique", "Resident", ""),
                ("Orion", "Exemple", ""),
                ("tesstouch", "Resident", "Tess ✨"),
            ];
            vec![NetEvent::DisplayNames {
                names: ids
                    .iter()
                    .map(|id| {
                        let (first, last, dn) = if *id == DEMO_AGENT {
                            ("Aurora", "Demo", "")
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
                let items: Vec<InvItem> = (0..5u128)
                    .map(|i| InvItem {
                        id: Uuid::from_u128(f.as_u128() ^ (0xABC0 + i)),
                        parent: *f,
                        name: format!("Objet de démo {}", i + 1),
                        desc: String::new(),
                        asset_type: [6, 3, 7, 0, 10][i as usize],
                        inv_type: [6, 3, 7, 0, 10][i as usize],
                        asset_id: Uuid::nil(),
                        flags: 0,
                        creator: Uuid::nil(),
                        created_at: 0,
                        owner: *owner,
                        group_mask: 0,
                        everyone_mask: 0,
                        next_owner_mask: 0,
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
        // block list: Loup Violet, an object, a name and the chat of group 2
        aurora_net::NetCommand::RequestMuteList { .. } => {
            let text = format!(
                "1 {} Loup Violet|0\n2 {} Boîte à spam|0\n0 {} Panneau publicitaire|\n0 {} Group:{}|\n",
                DEMO_LOUP,
                Uuid::from_u128(0xB10C_0000_0000_0000_0000_0000_0000_0001),
                Uuid::nil(),
                Uuid::nil(),
                DEMO_GROUP2,
            );
            // The animesh fixture needs its wearer visible in full.
            let text = if std::env::var_os("AURORA_DEMO_ANIMESH").is_some() {
                text.lines().skip(1).collect::<Vec<_>>().join("\n")
            } else {
                text
            };
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
                    g(DEMO_GROUP2, "Loups du Nord", false),
                    aurora_net::GroupMembership {
                        list_in_profile: false,
                        ..g(DEMO_GROUP3, "Marché de la Place", true)
                    },
                ]),
                NetEvent::ActiveGroup {
                    id: DEMO_GROUP1,
                    name: "Aurora Builders".into(),
                    title: "Bâtisseuse".into(),
                },
                NetEvent::SessionInvite(aurora_net::SessionInvite {
                    session: DEMO_GROUP1,
                    from: DEMO_NOVA,
                    from_name: "Nova Exemple".into(),
                    message: "Salut le groupe ! On construit la place ce soir 🐺".into(),
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
                        from_name: "Loup Violet".into(),
                        to_agent_id: me,
                        dialog: 17,
                        session_id: *session,
                        message: "(message d'un résident bloqué)".into(),
                        offline: false,
                        from_group: false,
                        binary_bucket: b"Aurora Builders\0".to_vec(),
                    }));
                    ev.push(NetEvent::InstantMessage(aurora_net::InstantMessage {
                        from_agent_id: DEMO_TESS,
                        from_name: "Tess Touch".into(),
                        to_agent_id: me,
                        dialog: 17,
                        session_id: *session,
                        message: "J'arrive avec les textures ✨".into(),
                        offline: false,
                        from_group: false,
                        binary_bucket: b"Aurora Builders\0".to_vec(),
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
                        line(DEMO_TESS, "Tess Touch", "Quelqu'un a vu le nouveau ciel EEP ?", 3600),
                        line(DEMO_LOUP, "Loup Violet", "(historique d'un résident bloqué)", 3000),
                        line(DEMO_NOVA, "Nova Exemple", "Oui, magnifique au coucher du soleil", 2400),
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
            let names = ["La plage d'Aurora", "Mon atelier"];
            let n = ((pick.as_u128() & 0xF) as usize + 1) % names.len();
            vec![NetEvent::PickInfo(Box::new(aurora_net::PickInfo {
                id: *pick,
                creator: *creator,
                parcel: Uuid::from_u128(0x9A2C_E100 + n as u128),
                name: names[n].into(),
                desc: "Un coin tranquille au bord de l'eau, idéal pour regarder le coucher du soleil 🌅".into(),
                sim_name: "Aurora Démo".into(),
                pos_global: glam::DVec3::new(256_000.0 + 140.0, 256_000.0 + 120.0, 25.0),
                enabled: true,
                ..Default::default()
            }))]
        }
        aurora_net::NetCommand::ParcelInfoRequest(id) => vec![NetEvent::ParcelInfo(Box::new(aurora_net::ParcelSummary {
            id: *id,
            name: "Place d'Aurora".into(),
            sim_name: "Aurora Démo".into(),
            ..Default::default()
        }))],
        aurora_net::NetCommand::ClassifiedsRequest(id) if *id == DEMO_LOUP => vec![NetEvent::AvatarClassifieds {
            target: *id,
            list: vec![(Uuid::from_u128(0xC1A5_0001), "Atelier du Loup : meubles en mesh".into())],
        }],
        aurora_net::NetCommand::ClassifiedInfoRequest(id) => vec![NetEvent::ClassifiedInfo(Box::new(aurora_net::ClassifiedInfo {
            id: *id,
            creator: DEMO_LOUP,
            creation_date: 1_759_000_000,
            category: 2,
            name: "Atelier du Loup : meubles en mesh".into(),
            desc: "Meubles en mesh légers, copiables et modifiables. Venez visiter la boutique !".into(),
            sim_name: "Aurora Démo".into(),
            parcel_name: "Place d'Aurora".into(),
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
            let mut blocks: Vec<aurora_net::MapBlock> = DEMO_SIMS
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
            let mut blocks: Vec<aurora_net::MapBlock> = DEMO_SIMS
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
                    item(128, 40, 0, 1, "Infohub de la place"),
                    item(400, 128, 0, 0, "Telehub de la lagune"),
                ],
                map_item::LAND_FOR_SALE => vec![item(220, 30, 1024, 2500, "Parcelle au bord de l'eau")],
                map_item::PG_EVENT => vec![item(90, 300, 1_791_475_200, 25, "Soirée aurores boréales")],
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
                    "Cube à acheter"
                } else {
                    "Objet de démonstration"
                }
                .into(),
                description: "Démo hors ligne : aucun L$ réel n'est dépensé.".into(),
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
                (7, "Règles du jeu – Français", perm::COPY | perm::TRANSFER),
                (6, "Jeu de cartes", perm::TRANSFER),
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
            let contents = [(7, "Carte de bienvenue"), (10, "Script de démonstration"), (6, "Cube de réserve")]
                .into_iter()
                .enumerate()
                .map(|(i, (ty, name))| {
                    aurora_llsd::llsd_map!("item_id" => action_id(990+i as u32),"parent_id" => *object,
                    "type" => ty,"inv_type" => ty,"name" => name,"desc" => "Contenu simulé hors ligne.")
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
            let mut o = action_object(982, 0, 138.0, 126.0, [0.8, 0.6, 0.3, 1.0], "Grab : déplacer");
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
            "IGNORE : cliquer à travers"
        } else {
            "DISABLED : aucun clic"
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

/// All click actions, plus an inherited Buy on a child. Named modes isolate
/// a target and App::demo_action_steps exercises the real picking / input.
pub fn action_events() -> Vec<NetEvent> {
    let mode = std::env::var("AURORA_DEMO_ACTIONS").unwrap_or_default();
    if mode.starts_with("linked-") {
        // The root's hollow geometry encloses a larger child's seat: its
        // smaller bounding box must not steal hits on the seat inside it.
        let mut table = action_object(985, 0, 138.0, 126.0, [0.7, 0.55, 0.8, 1.0], "Racine : Touch");
        table.scale = Vec3::new(2.4, 2.4, 1.4);
        table.position.z = floor_at(138.0, 126.0) + 0.7;
        table.volume = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 35000, 0);
        table.update_flags = 1 << 7;
        let mut chair = action_object(986, 1, 138.0, 126.0, [0.3, 0.75, 0.55, 1.0], "Enfant lié : Sit");
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
        (970, 1, 138.0, 122.0, [0.48, 0.3, 0.75, 1.0], "Sit : s'asseoir"),
        (971, 2, 138.0, 126.0, [0.35, 0.65, 0.8, 1.0], "Buy : copie à L$ 10"),
        (972, 3, 135.0, 130.0, [0.3, 0.75, 0.55, 1.0], "Pay : payer l'objet"),
        (975, 0, 139.0, 132.0, [0.7, 0.55, 0.8, 1.0], "NONE / TOUCH : toucher"),
        (976, 4, 138.0, 134.0, [0.8, 0.7, 0.4, 1.0], "Open : contenu"),
        (977, 5, 142.0, 122.0, [0.3, 0.65, 0.7, 1.0], "Play : lecture / pause"),
        (978, 6, 142.0, 126.0, [0.35, 0.65, 0.8, 1.0], "Open Media : média de parcelle"),
        (979, 7, 142.0, 130.0, [0.6, 0.65, 0.85, 1.0], "Zoom : cadrer l'objet"),
        (980, 8, 142.0, 134.0, [0.75, 0.3, 0.35, 1.0], "DISABLED : aucun clic"),
        (981, 9, 136.5, 125.8, [0.75, 0.3, 0.35, 1.0], "IGNORE : cliquer à travers"),
        (982, 0, 139.0, 138.0, [0.8, 0.6, 0.3, 1.0], "Grab : déplacer"),
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
        "Buy hérité",
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
const DEMO_SIMS: &[(u16, u16, &str, u8)] = &[
    (1000, 1000, "Aurora Démo", 13),
    (1001, 1000, "Lagune Boréale", 21),
    (999, 1000, "Pinède", 13),
    (1000, 1001, "Nordheim", 42),
    (1001, 1001, "Faille", 254),
    (999, 999, "Sablière", 13),
];

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
        message: "Mode démo hors-ligne : aucune connexion réseau.".into(),
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
            name: "Aurora Démo".into(),
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
        "Aurora Viewer\nRust · Vulkan",
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
        for (k, (tex, mat, label)) in ALPHA_PANELS.iter().enumerate() {
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
        id += ALPHA_PANELS.len() as u32;
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
                o.text = "Override : 4 × 4, teinte".into();
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
                "Défilement (SMOOTH)",
            ),
            (
                TEX_FRAMES,
                Uuid::nil(),
                false,
                anim(Ta::LOOP, -1, (4, 4), 0.0, 0.0, 4.0),
                "Grille 4 × 4 (LOOP)",
            ),
            (
                TEX_FRAMES,
                Uuid::nil(),
                false,
                anim(Ta::LOOP | Ta::PING_PONG, -1, (4, 4), 0.0, 4.0, 2.0),
                "Aller-retour (PING_PONG)",
            ),
            (
                TEX_TILES,
                Uuid::nil(),
                false,
                anim(smooth | Ta::ROTATE, -1, (1, 1), 0.0, std::f32::consts::TAU, 1.0),
                "Rotation (ROTATE)",
            ),
            (
                TEX_HOLES,
                Uuid::nil(),
                false,
                anim(smooth | Ta::PING_PONG | Ta::SCALE, -1, (1, 1), 1.0, 2.0, 0.5),
                "Échelle (SCALE), masque alpha",
            ),
            (
                TEX_FRAMES,
                Uuid::nil(),
                false,
                anim(Ta::LOOP, 3, (4, 4), 0.0, 0.0, 4.0),
                "Une seule face (face 3)",
            ),
            (
                TEX_TILES,
                MAT_BUMPY,
                false,
                anim(smooth, -1, (1, 1), 0.0, 1.0, -0.25),
                "Matériau : normales suivent",
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
    if let Ok(mode) = std::env::var("AURORA_DEMO_PARTICLES") {
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
        for (wind, y, label) in [(true, 126.0, "Vent"), (false, 130.0, "Fumée")] {
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
    ev.push(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: objs,
    });
    ev.extend(overrides);

    // our avatar + a second one
    let mut avatars = Vec::new();
    for (k, (pos, first, last)) in [
        (Vec3::new(132.0, 126.0, 0.0), "Aurora", "Demo"),
        (Vec3::new(136.0, 128.5, 0.0), "Loup", "Violet"),
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
    // Loup Violet wears a hat (skull) and holds a glowing orb (right hand):
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
            name: "Place d'Aurora".into(),
            desc: "Parcelle de démonstration hors-ligne d'Aurora Viewer.".into(),
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
    // AURORA_DEMO_SKY=<gamma>: a classic EEP sky (no reflection probe
    // ambiance) with the given sky gamma and a sunlight color above 1, as most
    // Second Life skies carry: shows the legacy gamma and the normalized
    // object light (Firestorm mSunDiffuse) without a grid
    if let Some(gamma) = std::env::var("AURORA_DEMO_SKY").ok().and_then(|v| v.trim().parse::<f64>().ok()) {
        ev.push(NetEvent::Environment {
            handle: HANDLE,
            parcel_id: -1,
            environment: demo_sky(Vec3::new(0.55, 0.35, 0.6).normalize(), gamma),
        });
    }
    ev.push(NetEvent::FriendsOnline {
        ids: vec![u(100), u(102)],
        online: true,
    });
    ev.push(NetEvent::Chat(aurora_net::ChatMessage {
        from_name: "Loup Violet".into(),
        source_id: Uuid::from_u128(2),
        owner_id: Uuid::nil(),
        source_type: aurora_net::ChatSourceType::Agent,
        chat_type: aurora_net::ChatType::Normal,
        position: Vec3::ZERO,
        message: "Bienvenue dans la démo d'Aurora Viewer ! 🐺✨❤️".into(),
    }));
    // AURORA_DEMO_LSL_BRIDGE=1: a worn Firestorm bridge talking to the viewer
    // (all hidden from chat) between two ordinary llOwnerSay lines (shown)
    if std::env::var_os("AURORA_DEMO_LSL_BRIDGE").is_some() {
        let bridge = Uuid::from_u128(0xB81D_6E00_0000_0000_0000_0000_0000_0001);
        let (hud, bridge_name) = ("HUD de démo", "#Firestorm LSL Bridge v2.32");
        for (from, name, text) in [
            (u(200), hud, "Démo : la ligne suivante du bridge doit rester cachée."),
            (
                bridge,
                bridge_name,
                "<bridgeURL>https://sim.example.invalid:12043/cap/demo</bridgeURL><bridgeAuth>demo</bridgeAuth><bridgeVer>2.32</bridgeVer>",
            ),
            (bridge, bridge_name, "<bridgeMovelock state=0>"),
            (u(200), hud, "Démo : fin du test du bridge."),
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
    // notification center samples: an offer, an item, a script menu
    let im = |dialog: u8, message: &str, bucket: Vec<u8>| {
        NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: Uuid::from_u128(2),
            from_name: "Loup Violet".into(),
            to_agent_id: Uuid::nil(),
            dialog,
            session_id: Uuid::new_v4(),
            message: message.into(),
            offline: false,
            from_group: false,
            binary_bucket: bucket,
        })
    };
    ev.push(im(22, "Viens voir le lagon au coucher du soleil !", Vec::new()));
    ev.push(im(4, "Lanterne aurorale", vec![6]));
    let menu = std::env::var("AURORA_DEMO_DIALOG").unwrap_or_default();
    let (object_name, message, buttons) = if !menu.is_empty() {
        let mut buttons = [
            "× Close",
            "« Back",
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
            buttons[9] = "Un libellé de bouton très long".into();
        }
        (
            "Table de jeux",
            "==== Table de jeux ====\n\nCurrent game category: \"8 Players\"\n\nSelect a game",
            buttons,
        )
    } else {
        (
            "Fontaine",
            "Choisis une couleur pour l'eau :",
            vec!["Violet".into(), "Turquoise".into(), "Ambre".into()],
        )
    };
    ev.push(NetEvent::ScriptDialog {
        object_id: Uuid::from_u128(0xF0),
        object_name: object_name.into(),
        owner_name: "Loup Violet".into(),
        message: message.into(),
        channel: -4242,
        buttons,
    });
    ev
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
pub const DEMO_MEDIA_PAGE: &str = "data:text/html;charset=utf-8,<html><body style='margin:0;height:100vh;background:linear-gradient(135deg,%232a1f4a,%230f6d6d);color:white;font:56px sans-serif;text-align:center'><h1 style='margin:40px 0 10px'>Aurora media</h1><p id='t' style='font-size:90px;margin:10px'>--</p><button onclick=\"document.body.style.background='%237a3b9c';this.textContent='Cliqu%C3%A9 !'\" style='font-size:48px;padding:10px 40px'>Cliquer</button><br><input placeholder='Taper ici' style='font-size:44px;margin-top:30px;width:70%'><script>setInterval(function(){document.getElementById('t').textContent=new Date().toLocaleTimeString()},250)</script></body></html>";

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
    ALPHA_PANELS[0].0
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

/// Root mesh, linked mesh driven by a child's signal, and a worn root mesh.
pub fn animesh_events() -> Vec<NetEvent> {
    let mesh = SculptParams {
        texture: ANIMESH_MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
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

/// AURORA_DEMO_TEXTURES=<n>: texture stress test, `n` small cubes (9000 for
/// any value that is not a number, as many textures as a busy region), each
/// with its own texture. Returns the cube count.
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

/// Offline voice stand-in for the voice dots: the other demo avatar talks
/// in bursts; our own dot follows the real microphone (`mic_level`, Firestorm
/// meter scale) while the microphone button / push-to-talk is on.
pub fn voice_levels(t: f64, me: Uuid, talking: bool, mic_level: f32) -> std::collections::HashMap<Uuid, (f32, bool)> {
    let mut m = std::collections::HashMap::new();
    let t = t as f32;
    let other = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002);
    let burst = t.rem_euclid(6.0) < 3.5;
    let level = if burst {
        0.22 + 0.55 * (t * 2.3).sin().abs() * (0.6 + 0.4 * (t * 7.1).sin())
    } else {
        0.0
    };
    m.insert(other, (level, burst && level > 0.2));
    let mine = if talking { mic_level } else { 0.0 };
    // above room noise (about 0.3 on this scale)
    m.insert(me, (mine, talking && mine > 0.35));
    m
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
const MAT_NONE: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0001);
const MAT_MASK: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0002);
const MAT_EMISSIVE: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0003);
const MAT_BUMPY: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0004);

/// (texture, legacy material, hover text) of the test panels.
const ALPHA_PANELS: [(Uuid, Option<Uuid>, &str); 6] = [
    (TEX_GRADIENT, None, "Auto : fondu"),
    (TEX_GRADIENT, Some(MAT_NONE), "Mode alpha : aucun"),
    (TEX_GRADIENT, Some(MAT_MASK), "Masque (seuil 50 %)"),
    (TEX_GRADIENT, Some(MAT_EMISSIVE), "Masque émissif"),
    (TEX_HOLES, None, "Auto : masque"),
    (BLANK_TEXTURE, Some(MAT_BUMPY), "Normales + spéculaire"),
];

/// Pixels of a procedural demo texture (RGBA8, 64×64).
pub fn local_texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
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
                // height h = sin(x) sin(y): normal = normalize(-dh/dx, -dh/dy, 1)
                let k = tau / 16.0;
                let dx = k * (fx * k).cos() * (fy * k).sin();
                let dy = k * (fx * k).sin() * (fy * k).cos();
                let n = Vec3::new(-dx * 2.0, -dy * 2.0, 1.0).normalize();
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
            "Siège (caméra de siège)",
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
fn demo_sky(sun: Vec3, gamma: f64) -> Llsd {
    use aurora_llsd::llsd_map;
    let q = Quat::from_rotation_arc(Vec3::X, sun);
    let arr = |v: &[f64]| Llsd::Array(v.iter().map(|x| Llsd::Real(*x)).collect());
    let mut frames = aurora_llsd::Map::new();
    frames.insert(
        "sky".into(),
        llsd_map! {
            "type" => "sky",
            "sun_rotation" => arr(&[q.x as f64, q.y as f64, q.z as f64, q.w as f64]),
            "sunlight_color" => arr(&[2.2, 2.34, 2.7]),
            "gamma" => gamma,
        },
    );
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
