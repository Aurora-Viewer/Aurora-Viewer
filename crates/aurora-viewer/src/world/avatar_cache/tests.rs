use super::*;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::NetEvent;
use aurora_net::inventory::{InvFolder, InvItem};
use aurora_net::objects::ObjectUpdate;
use glam::Vec3;

const ME: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
const OTHER: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_0002);
const HAT_ITEM: u128 = 501;
const COF: u128 = 11;

/// A world with our avatar of the demo and another avatar beside it.
fn setup() -> (World, RegionHandle, ObjectUpdate) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.agent_id = ME;
    world.avatar_cache.set_enabled(true);
    let (handle, avatar) = crate::demo::events()
        .into_iter()
        .find_map(|ev| match ev {
            NetEvent::ObjectUpdates { handle, objects } => objects.into_iter().find(|o| o.full_id == ME).map(|o| (handle, o)),
            _ => None,
        })
        .expect("demo contains our avatar");
    world.main_region = Some(handle);
    world.ensure_region(handle, (256, 256));
    let mut other = avatar.clone();
    other.full_id = OTHER;
    other.local_id = avatar.local_id + 1000;
    other.position += Vec3::X;
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: vec![avatar.clone(), other],
    });
    (world, handle, avatar)
}

use aurora_net::RegionHandle;

fn appearance(avatar_id: Uuid, attachments: Vec<(Uuid, u8)>) -> aurora_net::AvatarAppearance {
    aurora_net::AvatarAppearance {
        avatar_id,
        texture_entry: None,
        visual_params: Vec::new(),
        cof_version: 0,
        appearance_version: 1,
        hover_height: 0.0,
        attachments,
    }
}

/// A two-prim object worn at `point` by the avatar `parent` (a local id),
/// from the inventory item `item` when there is one.
fn worn(template: &ObjectUpdate, parent: u32, local: u32, id: u128, point: u8, item: Option<u128>) -> Vec<ObjectUpdate> {
    let mut root = template.clone();
    root.pcode = aurora_prim::params::LL_PCODE_VOLUME;
    root.local_id = local;
    root.full_id = Uuid::from_u128(id);
    root.parent_id = parent;
    root.state = (point & 0x0F) << 4 | (point >> 4);
    root.position = Vec3::ZERO;
    root.name_values = item.map_or_else(String::new, |i| format!("AttachItemID STRING RW SV {}", Uuid::from_u128(i)));
    root.text = "floating text".into();
    let mut child = root.clone();
    child.local_id = local + 1;
    child.full_id = Uuid::from_u128(id + 1);
    child.parent_id = local;
    child.state = 0;
    child.position = Vec3::Z;
    child.name_values.clear();
    vec![root, child]
}

/// Our Current Outfit folder, fetched, linking to `items`.
fn outfit_links(world: &mut World, items: &[u128]) {
    let cof = Uuid::from_u128(COF);
    let links: Vec<Uuid> = items.iter().map(|i| Uuid::from_u128(1000 + i)).collect();
    world.inventory.folders.insert(
        cof,
        Folder {
            info: InvFolder {
                id: cof,
                name: "Current Outfit".into(),
                type_default: crate::world::outfit::FT_CURRENT_OUTFIT,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: links.clone(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    for (&i, link) in items.iter().zip(links) {
        world.inventory.items.insert(
            link,
            InvItem {
                id: link,
                parent: cof,
                asset_type: 24,
                inv_type: 17,
                asset_id: Uuid::from_u128(i),
                ..Default::default()
            },
        );
        world.inventory.items.insert(
            Uuid::from_u128(i),
            InvItem {
                id: Uuid::from_u128(i),
                asset_type: 6,
                inv_type: 17,
                name: format!("item {i}"),
                ..Default::default()
            },
        );
    }
}

fn copies_of(world: &World, avatar: &Uuid) -> Vec<usize> {
    let idx = world.objects.index_of_uuid(avatar).expect("avatar");
    let key = world.objects.get(idx).expect("avatar").key;
    world
        .objects
        .children_of(&key)
        .iter()
        .copied()
        .filter(|&i| world.objects.get(i).is_some_and(|o| o.is_client_preview()))
        .collect()
}

#[test]
fn what_we_wore_is_kept_without_what_a_copy_leaves_out() {
    let (mut world, handle, me) = setup();
    let mut objects = worn(&me, me.local_id, 70_000, 900, 2, Some(HAT_ITEM));
    objects.extend(worn(&me, me.local_id, 70_010, 910, 35, Some(502)));
    world.apply(NetEvent::ObjectUpdates { handle, objects });
    let idx = world.objects.index_of_uuid(&ME).expect("us");
    let set = capture::capture(&world, idx, 7);
    assert_eq!(set.saved_at, 7);
    assert_eq!(set.objects.len(), 1, "the HUD is not kept");
    let hat = &set.objects[0];
    assert_eq!(
        (hat.root_id, hat.item_id, hat.point),
        (Uuid::from_u128(900), Uuid::from_u128(HAT_ITEM), 2)
    );
    assert_eq!(hat.prims.len(), 2);
    let root = aurora_net::objects::parse_compressed(&hat.prims[0].data, 0).expect("root");
    assert!(root.name_values.is_empty() && root.text.is_empty());
    assert_eq!(root.parent_id, me.local_id);
    let child = aurora_net::objects::parse_compressed(&hat.prims[1].data, 0).expect("child");
    assert_eq!(child.parent_id, 70_000);
    // the same objects, the same signature; a change, another
    let before = capture::signature(&world, idx);
    assert_eq!(capture::signature(&world, idx), before);
    let mut again = worn(&me, me.local_id, 70_000, 900, 2, Some(HAT_ITEM));
    again[1].scale = Vec3::splat(3.0);
    world.apply(NetEvent::ObjectUpdates { handle, objects: again });
    assert_ne!(capture::signature(&world, idx), before);
    // and the file keeps it as it is
    assert_eq!(file::decode(&file::encode(&set)), Some(set));
}

#[test]
fn our_outfit_is_drawn_from_the_cache_until_the_region_sends_it() {
    // what the last session kept
    let (mut world, handle, me) = setup();
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: worn(&me, me.local_id, 70_000, 900, 2, Some(HAT_ITEM)),
    });
    let idx = world.objects.index_of_uuid(&ME).expect("us");
    let kept = capture::capture(&world, idx, unix_now());

    // the next login: our avatar, nothing worn yet
    let (mut world, handle, me) = setup();
    world.avatar_cache.insert_set(ME, kept);
    let t0 = Instant::now();
    world.update_avatar_cache(t0);
    assert!(copies_of(&world, &ME).is_empty(), "the outfit is not known yet");
    outfit_links(&mut world, &[HAT_ITEM]);
    world.update_avatar_cache(t0);
    let copies = copies_of(&world, &ME);
    assert_eq!(copies.len(), 1, "the hat, as a copy");
    let copy = world.objects.get(copies[0]).expect("copy");
    assert_eq!(copy.attachment_point(), 2);
    assert_eq!(copy.attachment_item_id(), None, "never taken for the worn hat");
    assert_eq!(world.objects.children_of(&copy.key).len(), 1, "with its child prim");
    assert!(world.outfit_attachment_items().is_empty());
    assert_eq!(world.avatar_cache.copy_count(), 1);
    // drawn once only
    world.update_avatar_cache(t0 + Duration::from_secs(1));
    assert_eq!(copies_of(&world, &ME).len(), 1);

    // the region's hat comes: the copy stays until it is drawn, 5 s at most
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: worn(&me, me.local_id, 80_000, 990, 2, Some(HAT_ITEM)),
    });
    world.update_avatar_cache(t0 + Duration::from_secs(2));
    assert_eq!(copies_of(&world, &ME).len(), 1);
    world.update_avatar_cache(t0 + Duration::from_secs(8));
    assert!(copies_of(&world, &ME).is_empty());
    assert_eq!(world.avatar_cache.copy_count(), 0);
    assert!(world.outfit_attachment_items().contains(&Uuid::from_u128(HAT_ITEM)));
}

#[test]
fn a_copy_goes_when_its_object_is_no_longer_worn() {
    let (mut world, _, me) = setup();
    let mut kept = KeptAvatar {
        saved_at: unix_now(),
        objects: Vec::new(),
        look: None,
        motions: Vec::new(),
    };
    // a hat of ours, and a scarf another avatar wore
    for (id, item) in [(900, HAT_ITEM), (920, 0)] {
        let prims = worn(&me, me.local_id, 70_000 + id as u32, id, 2, (item != 0).then_some(item))
            .iter()
            .map(|u| file::WornPrim {
                update_flags: 0,
                data: aurora_net::objects::encode_compressed(u),
                overrides: Vec::new(),
            })
            .collect();
        kept.objects.push(WornObject {
            root_id: Uuid::from_u128(id),
            item_id: if item == 0 { Uuid::nil() } else { Uuid::from_u128(item) },
            point: 2,
            prims,
        });
    }
    world.avatar_cache.insert_set(ME, kept.clone());
    world.avatar_cache.insert_set(OTHER, kept);
    outfit_links(&mut world, &[HAT_ITEM]);
    let mut appearance = appearance(OTHER, vec![(Uuid::from_u128(920), 2)]);
    world.apply(NetEvent::Appearance(appearance.clone()));
    let t0 = Instant::now();
    world.update_avatar_cache(t0);
    assert_eq!(copies_of(&world, &ME).len(), 1, "our hat only (the scarf is no outfit item)");
    assert_eq!(copies_of(&world, &OTHER).len(), 1, "the listed scarf only (the hat is not listed)");
    // a copy is no worn object: not ours (the arrival's patience), not for
    // RLVa, not against the simulator's list (J2)
    assert_eq!(world.own_attachment_count(), 0);
    assert!(world.rlva_temporary_attachment(copy_id(Uuid::from_u128(900))).is_none());
    let other = world.objects.index_of_uuid(&OTHER).expect("the other avatar");
    let listed = world.worn_vs_listed(other).expect("an avatar");
    assert_eq!((listed.arrived, listed.missing.len()), (0, 1), "the scarf is still missing");
    // but the cloud gate sees it come, as a copy
    assert_eq!(world.avatar_cache.standing_in(&OTHER), 1);

    // we take the hat off elsewhere, they take the scarf off
    outfit_links(&mut world, &[]);
    appearance.attachments.clear();
    world.apply(NetEvent::Appearance(appearance));
    world.update_avatar_cache(t0 + Duration::from_secs(1));
    assert!(copies_of(&world, &ME).is_empty());
    assert!(copies_of(&world, &OTHER).is_empty());
}

#[test]
fn copies_wait_a_limited_time_and_go_with_the_setting() {
    let (mut world, _, me) = setup();
    let prims = worn(&me, me.local_id, 70_000, 920, 2, None)
        .iter()
        .map(|u| file::WornPrim {
            update_flags: 0,
            data: aurora_net::objects::encode_compressed(u),
            overrides: Vec::new(),
        })
        .collect();
    world.avatar_cache.insert_set(
        OTHER,
        KeptAvatar {
            saved_at: unix_now(),
            objects: vec![WornObject {
                root_id: Uuid::from_u128(920),
                item_id: Uuid::nil(),
                point: 2,
                prims,
            }],
            look: None,
            motions: Vec::new(),
        },
    );
    world.apply(NetEvent::Appearance(appearance(OTHER, vec![(Uuid::from_u128(920), 2)])));
    let t0 = Instant::now();
    world.update_avatar_cache(t0);
    assert_eq!(copies_of(&world, &OTHER).len(), 1);
    world.avatar_cache.set_enabled(false);
    world.update_avatar_cache(t0);
    assert!(copies_of(&world, &OTHER).is_empty(), "the setting off takes them away");
    world.avatar_cache.set_enabled(true);
    world.update_avatar_cache(t0 + WINDOW + Duration::from_secs(1));
    assert!(copies_of(&world, &OTHER).is_empty(), "too late: never drawn again");
}

#[test]
fn only_wanted_objects_not_there_yet_are_drawn() {
    let object = |root: u128, item: u128| WornObject {
        root_id: Uuid::from_u128(root),
        item_id: if item == 0 { Uuid::nil() } else { Uuid::from_u128(item) },
        point: 2,
        prims: Vec::new(),
    };
    let set = KeptAvatar {
        saved_at: 0,
        objects: vec![object(1, 10), object(2, 20), object(3, 0)],
        look: None,
        motions: Vec::new(),
    };
    let ids = |v: Vec<&WornObject>| v.iter().map(|o| o.root_id.as_u128()).collect::<Vec<_>>();
    let outfit: HashSet<Uuid> = [Uuid::from_u128(10), Uuid::from_u128(20)].into();
    let listed: HashSet<Uuid> = [Uuid::from_u128(2), Uuid::from_u128(3)].into();
    assert_eq!(ids(wanted(&set, true, &outfit, &listed, |_| false)), vec![1, 2]);
    assert_eq!(
        ids(wanted(&set, true, &outfit, &listed, |o| o.item_id == Uuid::from_u128(10))),
        vec![2]
    );
    assert_eq!(ids(wanted(&set, false, &outfit, &listed, |_| false)), vec![2, 3]);
    assert!(fresh(100, 100 + MAX_AGE.as_secs()));
    assert!(!fresh(100, 101 + MAX_AGE.as_secs()));
}

#[test]
fn a_copy_keeps_the_textures_of_its_object() {
    let (mut world, _, _) = setup();
    let original = crate::demo::wolf_attachments(true);
    let set = KeptAvatar {
        saved_at: unix_now(),
        objects: original
            .iter()
            .map(|o| WornObject {
                root_id: o.full_id,
                item_id: Uuid::nil(),
                point: o.state.rotate_left(4),
                prims: vec![file::WornPrim {
                    update_flags: 0,
                    data: aurora_net::objects::encode_compressed(o),
                    overrides: Vec::new(),
                }],
            })
            .collect(),
        look: None,
        motions: Vec::new(),
    };
    world.avatar_cache.insert_set(OTHER, set);
    let listed = original.iter().map(|o| (o.full_id, o.state.rotate_left(4))).collect();
    world.apply(NetEvent::Appearance(appearance(OTHER, listed)));
    world.update_avatar_cache(Instant::now());
    let copies = copies_of(&world, &OTHER);
    assert_eq!(copies.len(), 2);
    for (copy, o) in copies.iter().zip(&original) {
        let te = world.objects.get(*copy).and_then(|c| c.te.clone()).expect("a texture entry");
        let want = o.texture_entry.as_ref().expect("texture entry");
        assert_eq!(te.faces[0].texture, want.faces[0].texture);
        // (a colour travels as bytes)
        for (got, want) in te.faces[0].color.iter().zip(want.faces[0].color) {
            assert!((got - want).abs() < 1.0 / 255.0, "{got} for {want}");
        }
    }
}

/// An appearance with a shape and a bake (TE face 8, the head's).
fn shaped(avatar_id: Uuid, value: u8, bake: u128) -> aurora_net::AvatarAppearance {
    let mut te = aurora_prim::TextureEntry::default();
    if let Some(face) = te.faces.get_mut(8) {
        face.texture = Uuid::from_u128(bake);
    }
    aurora_net::AvatarAppearance {
        texture_entry: Some(Arc::new(te)),
        visual_params: vec![value; 218],
        hover_height: 0.1,
        cof_version: 7,
        ..appearance(avatar_id, Vec::new())
    }
}

#[test]
fn a_kept_look_is_drawn_until_the_region_sends_its_own() {
    let (mut world, _, _) = setup();
    let idx = world.objects.index_of_uuid(&OTHER).expect("other avatar");
    assert_eq!(world.visual_params(&OTHER), None, "a cloud: nothing to draw yet");
    world.avatar_cache.insert_set(
        OTHER,
        KeptAvatar {
            saved_at: unix_now(),
            objects: Vec::new(),
            look: Some(capture::look_of(&shaped(OTHER, 40, 0xBA4E))),
            motions: Vec::new(),
        },
    );
    world.update_avatar_cache(Instant::now());
    // the kept look is drawn: shape, bakes and hover height
    assert_eq!(world.visual_params(&OTHER), Some(&[40u8; 218][..]));
    assert_eq!(world.server_bake_texture(idx, 8), Some(Uuid::from_u128(0xBA4E)));
    assert!((world.appearance_hover(&OTHER) - 0.1).abs() < 1e-6);
    let kept_generation = world.appearance_generation(&OTHER);
    assert!(kept_generation > 0);
    // never taken for the region's
    assert_eq!(world.appearance_cof_version(&OTHER), -1);
    assert_eq!(world.announced_attachments(&OTHER), 0);
    // the region's replaces it
    world.apply(NetEvent::Appearance(shaped(OTHER, 90, 0xBA4F)));
    assert!(world.avatar_cache.kept_look(&OTHER).is_none());
    assert_eq!(world.server_bake_texture(idx, 8), Some(Uuid::from_u128(0xBA4F)));
    assert_eq!(world.appearance_cof_version(&OTHER), 7);
    assert_ne!(world.appearance_generation(&OTHER), kept_generation);
    world.update_avatar_cache(Instant::now());
    assert!(world.avatar_cache.kept_look(&OTHER).is_none(), "not drawn again");
    // and is what is kept next
    let set = capture::capture(&world, idx, unix_now());
    assert_eq!(set.look, Some(capture::look_of(&shaped(OTHER, 90, 0xBA4F))));
}

#[test]
fn a_kept_look_goes_with_the_setting_and_an_empty_one_is_not_drawn() {
    let (mut world, _, _) = setup();
    let look = capture::look_of(&shaped(OTHER, 40, 0xBA4E));
    world.avatar_cache.insert_set(
        OTHER,
        KeptAvatar {
            saved_at: unix_now(),
            objects: Vec::new(),
            look: Some(look.clone()),
            motions: Vec::new(),
        },
    );
    world.update_avatar_cache(Instant::now());
    assert!(world.visual_params(&OTHER).is_some());
    world.avatar_cache.set_enabled(false);
    world.update_avatar_cache(Instant::now());
    assert_eq!(world.visual_params(&OTHER), None, "the setting off: a cloud again");
    // an appearance the region sent without parameters keeps the kept look
    world.avatar_cache.set_enabled(true);
    world.update_avatar_cache(Instant::now());
    world.apply(NetEvent::Appearance(appearance(OTHER, Vec::new())));
    assert_eq!(world.visual_params(&OTHER), Some(&[40u8; 218][..]));
    // a kept look without parameters is none
    let (mut world, _, _) = setup();
    world.avatar_cache.insert_set(
        OTHER,
        KeptAvatar {
            saved_at: unix_now(),
            objects: Vec::new(),
            look: Some(file::KeptLook {
                visual_params: Vec::new(),
                ..look
            }),
            motions: Vec::new(),
        },
    );
    world.update_avatar_cache(Instant::now());
    assert!(world.avatar_cache.kept_look(&OTHER).is_none());
}

#[test]
fn the_store_stays_for_the_next_login() {
    let (mut world, _, _) = setup();
    world
        .avatar_cache
        .set_store(Some(std::env::temp_dir().join("aurora-avatar-cache-next-login-test")));
    world.avatar_cache.insert_set(
        OTHER,
        KeptAvatar {
            saved_at: unix_now(),
            objects: Vec::new(),
            look: Some(capture::look_of(&shaped(OTHER, 40, 0xBA4E))),
            motions: Vec::new(),
        },
    );
    world.update_avatar_cache(Instant::now());
    assert!(world.avatar_cache.kept_look(&OTHER).is_some());
    world.reset();
    assert!(world.avatar_cache.io.is_some(), "the next login reads the same store");
    assert!(world.avatar_cache.kept_look(&OTHER).is_none(), "nothing of the last session");
}

#[test]
fn the_rez_effect_knows_what_the_cache_draws() {
    let (mut world, handle, avatar) = setup();
    outfit_links(&mut world, &[HAT_ITEM]);
    let hat = worn(&avatar, avatar.local_id, 80_001, 900, 2, Some(HAT_ITEM));
    world.avatar_cache.insert_set(
        ME,
        KeptAvatar {
            saved_at: unix_now(),
            objects: vec![WornObject {
                root_id: Uuid::from_u128(900),
                item_id: Uuid::from_u128(HAT_ITEM),
                point: 2,
                prims: hat
                    .iter()
                    .map(|u| file::WornPrim {
                        update_flags: 0,
                        data: aurora_net::objects::encode_compressed(u),
                        overrides: Vec::new(),
                    })
                    .collect(),
            }],
            look: None,
            motions: Vec::new(),
        },
    );
    assert!(!world.avatar_cache.draws(&ME));
    world.update_avatar_cache(Instant::now());
    assert!(world.avatar_cache.draws(&ME));
    assert!(!world.avatar_cache.draws(&OTHER));
    // the region's hat, another object id this login: matched by its item
    let mut real = worn(&avatar, avatar.local_id, 80_101, 990, 2, Some(HAT_ITEM));
    real.truncate(1);
    assert!(
        world
            .avatar_cache
            .replaced_by(&ME, Uuid::from_u128(990), Some(Uuid::from_u128(HAT_ITEM)))
    );
    assert!(
        !world
            .avatar_cache
            .replaced_by(&ME, Uuid::from_u128(990), Some(Uuid::from_u128(502)))
    );
    assert!(!world.avatar_cache.replaced_by(&ME, Uuid::from_u128(990), Some(Uuid::nil())));
    // a copy goes without a derez: the region's object takes its place
    world.objects.killed.clear();
    world.apply(NetEvent::ObjectUpdates { handle, objects: real });
    world.avatar_cache.set_enabled(false);
    world.update_avatar_cache(Instant::now());
    assert!(world.objects.killed.is_empty());
    assert!(!world.avatar_cache.draws(&ME));
}

const WAG: Uuid = Uuid::from_u128(0x7A11_0000_0000_0000_0000_0000_0000_0001);
const STAND: Uuid = Uuid::from_u128(0x2408_0000_0000_0000_0000_0000_0000_0001);

fn own_animations(world: &mut World, anims: Vec<(Uuid, i32)>, sources: Vec<Uuid>) {
    world.apply(NetEvent::AvatarAnimations {
        avatar: ME,
        anims,
        sources,
    });
}

fn playing(world: &World, id: Uuid) -> Option<crate::world::PlayingAnimation> {
    world.animations_of(&ME).iter().find(|a| a.id == id).copied()
}

/// What the last session kept: the hat, whose child prim's script played
/// `WAG` on us.
fn kept_with_wag() -> KeptAvatar {
    let (mut world, handle, me) = setup();
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: worn(&me, me.local_id, 70_000, 900, 2, Some(HAT_ITEM)),
    });
    own_animations(&mut world, vec![(STAND, 1), (WAG, 3)], vec![Uuid::nil(), Uuid::from_u128(901)]);
    let idx = world.objects.index_of_uuid(&ME).expect("us");
    capture::capture(&world, idx, unix_now())
}

#[test]
fn the_animations_of_our_worn_objects_are_kept() {
    let kept = kept_with_wag();
    assert_eq!(
        kept.motions,
        vec![file::KeptMotion {
            animation: WAG,
            item_id: Uuid::from_u128(HAT_ITEM),
        }],
        "the hat's, not the stand"
    );
}

#[test]
fn a_kept_animation_plays_with_its_copy_until_the_region_lists_it() {
    let kept = kept_with_wag();
    let (mut world, handle, me) = setup();
    world.avatar_cache.insert_set(ME, kept);
    own_animations(&mut world, vec![(STAND, 1)], vec![Uuid::nil()]);
    let t0 = Instant::now();
    world.update_avatar_cache(t0);
    assert!(playing(&world, WAG).is_none(), "no copy of the hat yet");
    outfit_links(&mut world, &[HAT_ITEM]);
    world.update_avatar_cache(t0);
    let played = playing(&world, WAG).expect("the wag plays with the hat's copy");
    assert!(playing(&world, STAND).is_some());
    // a new list from the region without it: it goes on
    own_animations(&mut world, vec![(STAND, 2)], vec![Uuid::nil()]);
    assert_eq!(playing(&world, WAG), Some(played));
    world.update_avatar_cache(t0 + Duration::from_secs(1));
    assert_eq!(playing(&world, WAG), Some(played));
    // the region's hat comes and its script plays it: the region's from now
    // on, without a restart
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: worn(&me, me.local_id, 80_000, 990, 2, Some(HAT_ITEM)),
    });
    own_animations(&mut world, vec![(STAND, 2), (WAG, 7)], vec![Uuid::nil(), Uuid::from_u128(991)]);
    let region = playing(&world, WAG).expect("still playing");
    assert_eq!(region.sequence, 7);
    assert_eq!(region.sequence_start, played.sequence_start, "no restart");
    assert_eq!(world.avatar_cache.played_motions().count(), 0);
    // the copy goes: the region's goes on
    world.update_avatar_cache(t0 + Duration::from_secs(2));
    assert_eq!(playing(&world, WAG), Some(region));
    world.update_avatar_cache(t0 + Duration::from_secs(8));
    assert!(copies_of(&world, &ME).is_empty());
    assert!(playing(&world, WAG).is_some());
}

#[test]
fn a_kept_animation_stops_with_its_copy_and_a_blocked_one_never_plays() {
    let kept = kept_with_wag();
    let (mut world, _, _) = setup();
    world.avatar_cache.insert_set(ME, kept.clone());
    outfit_links(&mut world, &[HAT_ITEM]);
    let t0 = Instant::now();
    world.update_avatar_cache(t0);
    assert!(playing(&world, WAG).is_some());
    // the copy goes before the region lists it (the setting off here)
    world.avatar_cache.set_enabled(false);
    world.update_avatar_cache(t0);
    assert!(playing(&world, WAG).is_none());

    // blocked in the Asset Blacklist: never played
    let (mut world, _, _) = setup();
    world.derender.add(
        WAG,
        crate::world::derender::Entry {
            name: String::new(),
            region: String::new(),
            kind: crate::world::derender::Kind::Animation,
            date: 0,
            // (not saved: a test)
            permanent: false,
        },
    );
    world.avatar_cache.insert_set(ME, kept);
    outfit_links(&mut world, &[HAT_ITEM]);
    world.update_avatar_cache(t0);
    assert_eq!(copies_of(&world, &ME).len(), 1);
    assert!(playing(&world, WAG).is_none());
}
