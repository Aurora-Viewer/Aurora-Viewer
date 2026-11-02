use super::*;

const A: Uuid = Uuid::from_u128(1);
const B: Uuid = Uuid::from_u128(2);
const TARGET: Uuid = Uuid::from_u128(3);

#[test]
fn rlva_channel_exclusions_require_their_general_restriction_and_use_native_secure_agreement() {
    let mut r = enabled();
    command(&mut r, A, "@sendchannel:-5=n,sendchannel_except:-6=n");
    assert!(r.restrictions().is_empty(), "channel exceptions must be positive");
    command(&mut r, A, "@sendchannel_except:123=n");
    assert!(r.allows_channel(123), "an exception entry alone does not enable its restriction");
    command(&mut r, A, "@sendchannel_except_sec=n");
    assert!(!r.allows_channel(123));
    command(&mut r, B, "@sendchannel_except=n");
    assert!(
        !r.allows_channel(123),
        "the normal issuer need not repeat a secure issuer's exception"
    );
    command(&mut r, B, "@permissive=n");
    assert!(
        r.allows_channel(123),
        "permissive=n requires both restricting issuers to list the blocked channel"
    );
    command(&mut r, B, "@sendchannel_except:123=n");
    assert!(!r.allows_channel(123));
    assert!(r.allows_channel(124));
    command(&mut r, A, "@clear");
    command(&mut r, B, "@clear");
    assert!(r.allows_channel(123));
}

#[test]
fn rlva_permissive_affects_only_behaviours_with_strict_exceptions() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        &format!("@edit=n,touchworld=n,edit:{TARGET}=n,touchworld:{TARGET}=n,sendim_sec=n,sendim:{TARGET}=n"),
    );
    command(&mut r, B, "@edit=n,touchworld=n,sendim_sec=n,permissive=n");
    assert!(r.allows("edit", &TARGET.to_string()) && r.allows("touchworld", &TARGET.to_string()));
    assert!(!r.allows_im("sendim", TARGET), "two secure issuers both need an exception");
    command(&mut r, B, &format!("@sendim:{TARGET}=n"));
    assert!(r.allows_im("sendim", TARGET));
}

#[test]
fn rlva_vendor_payments_follow_buy_while_resident_payments_follow_pay() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva_configure(true);
    let object = NetCommand::PayObject {
        handle: 1,
        object: TARGET,
        amount: 1,
        description: String::new(),
    };
    let resident = NetCommand::PayResident {
        to: TARGET,
        group: false,
        amount: 1,
        message: String::new(),
    };
    command(&mut world.rlva, A, "@pay=n");
    assert!(world.rlva_allows_command(&object, Uuid::nil()));
    assert!(!world.rlva_allows_command(&resident, Uuid::nil()));
    command(&mut world.rlva, A, "@pay=y,buy=n");
    assert!(!world.rlva_allows_command(&object, Uuid::nil()));
    assert!(world.rlva_allows_command(&resident, Uuid::nil()));
}

#[test]
fn rlva_only_an_actual_remove_locked_hud_prevents_hiding_huds_and_using_wireframe() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    let id = Uuid::from_u128(0x726c7661687564);
    let point = *world.avatar_lib.attach_points.iter().find(|(_, p)| p.hud).unwrap().0;
    let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
        item_id: id,
        owner_id: world.agent_id,
        point,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    });
    let handle = world.main_region.unwrap();
    world.objects.upsert(handle, object.clone());
    command(
        &mut world.rlva,
        A,
        &format!("@addattach:{}=n,remattach:chest=n", inventory::POINTS[point as usize]),
    );
    assert!(
        !world.rlva_has_locked_hud(),
        "add locks and locks on unrelated points leave it removable"
    );
    command(&mut world.rlva, A, &format!("@remattach:{}=n", inventory::POINTS[point as usize]));
    assert!(world.rlva_has_locked_hud());
    command(&mut world.rlva, A, "@clear");
    command(&mut world.rlva, id, "@detach=n");
    assert!(world.rlva_has_locked_hud(), "temporary HUDs do not require inventory metadata");
    object.state = 1_u8.rotate_right(4);
    world.objects.upsert(handle, object);
    assert!(!world.rlva_has_locked_hud(), "a locked ordinary attachment is not a HUD");
    world.rlva_configure(false);
    assert!(!world.rlva_has_locked_hud());
}

#[test]
fn rlva_owner_say_folds_options_and_parameters_without_accepting_malformed_clear() {
    let mut r = enabled();
    assert!(!r.receive(&chat("@a")), "short non-command OwnerSay remains chat");
    command(&mut r, A, "@FLY=N,DETACHTHIS:OutFit=N,NOTIFY:123;FlY=N,JUMP=N");
    assert!(r.has("fly") && r.has("jump"));
    assert!(r.restrictions().iter().any(|(_, rule)| rule == "detachthis:outfit"));
    command(&mut r, A, "@FLY=Y");
    assert!(matches!(r.out.last(), Some(NetCommand::ChatReply { message, .. }) if message == "/fly=y"));
    command(&mut r, A, "@clear:missing");
    assert!(r.has("jump"), "a missing equals sign cannot clear the issuer's restrictions");
    assert!(Command::parse("clear:missing").is_none());
    command(&mut r, A, "@CLEAR=");
    assert!(r.restrictions().is_empty());
}

#[test]
fn rlva_fov_bounds_follow_window_aspect_and_leave_unrestricted_native_views_intact() {
    let mut r = enabled();
    let native = crate::camera::CameraSettings {
        fov: 2.97,
        ..Default::default()
    };
    assert_eq!(r.camera_settings(&native, 2.0), native);
    r.camera_force(A, &Command::parse("setcam_fov:0.09=force").unwrap());
    assert!((r.camera_settings(&native, 1.0).fov - 0.09).abs() < 1e-6);
    assert!((r.camera_settings(&native, 0.5).fov - 10.0f32.to_radians()).abs() < 1e-6);
    r.camera_force(A, &Command::parse("setcam_fov:100=force").unwrap());
    assert!((r.camera_settings(&native, 2.0).fov - 87.5f32.to_radians()).abs() < 1e-6);
    command(&mut r, A, "@setcam=n,setcam_fovmin:0.5=n,setcam_fovmax:0.6=n");
    command(&mut r, B, "@setcam_fovmin:1.0=n");
    assert_eq!(
        r.camera_settings(&native, 1.0).fov,
        0.6,
        "only the camera owner's limits are active"
    );
    command(&mut r, A, "@clear");
    command(&mut r, B, "@clear");
    assert_eq!(
        r.camera_settings(&native, 1.0).fov,
        2.97,
        "releasing the last owned FOV limits restores the user's value"
    );
    for aspect in [f32::NAN, f32::INFINITY, 0.0, 0.001, 1000.0] {
        assert!(r.camera_settings(&native, aspect).fov.is_finite());
    }
    r.configure(false);
    assert_eq!(r.camera_settings(&native, 0.5), native);
}

#[test]
fn rlva_inventory_wire_order_attachment_groups_and_own_locks_match_the_reference() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        AttachRequest,
        inventory::{InvFolder, InvItem},
    };
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let cof = Uuid::from_u128(500);
    let mut items = Vec::new();
    for kind in [0, 4, 9] {
        let id = Uuid::from_u128(600 + kind);
        world.inventory.items.insert(
            id,
            InvItem {
                id,
                parent: cof,
                name: format!("Layer {kind}"),
                asset_type: if kind == 0 { 13 } else { 5 },
                flags: kind as u32,
                ..Default::default()
            },
        );
        items.push(id);
    }
    world.inventory.folders.insert(
        cof,
        Folder {
            info: InvFolder {
                id: cof,
                parent: world.inventory.root,
                type_default: crate::world::outfit::FT_CURRENT_OUTFIT,
                ..Default::default()
            },
            children: Vec::new(),
            items,
            state: FetchState::Fetched,
            library: false,
        },
    );
    let query = |text| Command::parse(text).unwrap();
    assert_eq!(
        world.rlva_inventory_query(&query("getoutfit=123"), A).as_deref(),
        Some("10010000000010000")
    );
    assert_eq!(world.rlva_inventory_query(&query("getoutfit:shirt=123"), A).as_deref(), Some("1"));
    assert_eq!(
        world.rlva_inventory_query(&query("getoutfitnames:shirt=123"), A).as_deref(),
        Some("")
    );
    for (item, point) in [(701, 1), (702, 2), (705, 5), (707, 7), (731, 31)] {
        let id = Uuid::from_u128(item);
        let update = crate::demo::outfit_attachment(&AttachRequest {
            item_id: id,
            owner_id: world.agent_id,
            point,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        });
        world.objects.upsert(world.main_region.unwrap(), update);
    }
    for (group, expected) in [
        ("head", "Skull"),
        ("torso", "Chest"),
        ("arms", "Left Hand"),
        ("legs", "Left Foot"),
        ("hud", "Center 2"),
    ] {
        let text = format!("getattachnames:{group}=123");
        let c = Command::parse(&text).unwrap();
        assert_eq!(world.rlva_inventory_query(&c, A).as_deref(), Some(expected));
    }
    let hud_points = world.rlva_inventory_query(&query("getaddattachnames:hud=123"), A).unwrap();
    assert!(hud_points.split(',').any(|p| p == "Center 2"));
    assert!(
        !hud_points.split(',').any(|p| p == "Left Ring Finger"),
        "Bento group 8 is not a HUD"
    );
    let head = Uuid::from_u128(702);
    command(&mut world.rlva, head, "@detach=n");
    assert_eq!(
        world.rlva_inventory_query(&query("getremattachnames:head=123"), head).as_deref(),
        Some(""),
        "queries do not waive their issuer's lock"
    );
    command(&mut world.rlva, head, "@remattach:head=force");
    world.rlva_pending();
    assert!(matches!(world.rlva.appearance.pop(), Some((_, crate::world::appearance::Action::RemoveItems(ids))) if ids.is_empty()));
    command(&mut world.rlva, head, "@detach=y,remattach:head=force");
    world.rlva_pending();
    assert!(matches!(world.rlva.appearance.pop(), Some((_, crate::world::appearance::Action::RemoveItems(ids))) if ids == vec![head]));
    command(&mut world.rlva, A, "@detach:skull=n,remoutfit:shirt=n");
    assert!(
        !world
            .rlva_inventory_query(&query("getaddattachnames:head=123"), A)
            .unwrap()
            .split(',')
            .any(|p| p == "Skull")
    );
    assert_eq!(
        world.rlva_inventory_query(&query("getremoutfitnames=123"), A).as_deref(),
        Some("gloves")
    );
    command(&mut world.rlva, A, "@showselfhead=n");
    let i = world.objects.index_of_uuid(&head).unwrap();
    assert!(!world.rlva_object_visible(i), "head attachments use group 2 from avatar_lad.xml");
    let chest = world.objects.index_of_uuid(&Uuid::from_u128(701)).unwrap();
    assert!(world.rlva_object_visible(chest));
}

#[test]
fn rlva_privacy_strict_exceptions_landmarks_profile_locations_and_nearby_release_together() {
    use aurora_net::{
        NetCommand,
        profile::{ClassifiedInfo, PickInfo},
    };
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    command(&mut world.rlva, A, "@shownames_sec=n,showloc=n,shownearby=n");
    command(&mut world.rlva, A, &format!("@shownames_sec:{TARGET}=n"));
    assert!(world.rlva_can_name(TARGET));
    command(&mut world.rlva, B, "@shownames_sec=n");
    assert!(!world.rlva_can_name(TARGET));
    command(&mut world.rlva, B, &format!("@shownames_sec:{TARGET}=n"));
    assert!(world.rlva_can_name(TARGET) && world.rlva_can_name(world.agent_id));
    assert!(!world.rlva_can_name(Uuid::from_u128(99)));
    world.refresh_rlva_privacy();
    assert!(crate::ui::people::nearby(&world).is_empty());
    let landmark = NetCommand::CreateInventoryItem {
        parent: world.inventory.root,
        kind: aurora_net::inventory::operations::NewItem::Landmark,
        name: "private".into(),
        permissions: None,
    };
    assert!(!world.rlva_allows_command(&landmark, Uuid::nil()));
    let mut dialog = crate::ui::landmark_create::CreateLandmarkUi::default();
    assert!(dialog.open(&world, None).is_none() && !dialog.open);
    let mut pick = PickInfo {
        id: TARGET,
        creator: world.agent_id,
        pos_global: world.agent_global().unwrap(),
        ..Default::default()
    };
    assert!(!world.rlva_allows_command(&NetCommand::PickUpdate(Box::new(pick.clone())), Uuid::nil()));
    world.profiles.picks.insert(pick.id, pick.clone());
    assert!(world.rlva_allows_command(&NetCommand::PickUpdate(Box::new(pick.clone())), Uuid::nil()));
    pick.pos_global.x += 1.0;
    assert!(!world.rlva_allows_command(&NetCommand::PickUpdate(Box::new(pick)), Uuid::nil()));
    let mut classified = ClassifiedInfo {
        id: TARGET,
        creator: world.agent_id,
        pos_global: world.agent_global().unwrap(),
        ..Default::default()
    };
    assert!(!world.rlva_allows_command(&NetCommand::ClassifiedUpdate(Box::new(classified.clone())), Uuid::nil()));
    world.profiles.classifieds.insert(classified.id, classified.clone());
    assert!(world.rlva_allows_command(&NetCommand::ClassifiedUpdate(Box::new(classified.clone())), Uuid::nil()));
    classified.pos_global.x += 1.0;
    assert!(!world.rlva_allows_command(&NetCommand::ClassifiedUpdate(Box::new(classified)), Uuid::nil()));
    world.rlva.configure(false);
    world.refresh_rlva_privacy();
    assert!(world.rlva_can_name(Uuid::from_u128(99)) && !crate::ui::people::nearby(&world).is_empty());
    assert!(world.rlva_allows_command(&landmark, Uuid::nil()));
}

#[test]
fn rlva_debug_resolution_writes_are_allowlisted_locked_ordered_and_released() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        "@setdebug=n,setdebug_renderresolutiondivisor:12=force,getdebug_renderresolutiondivisor=123",
    );
    assert_eq!(r.pending.len(), 2);
    let writes = std::mem::take(&mut r.pending);
    assert!(r.debug_force(A, &writes[0].1));
    assert_eq!(r.resolution_divisor, Some(12));
    assert_eq!(writes[1].1.behaviour, "getdebug_renderresolutiondivisor");
    let force = |value| Command::parse(&format!("setdebug_renderresolutiondivisor:{value}=force")).unwrap();
    for value in ["-1", "NaN", "4294967296", "2.5"] {
        r.debug_force(A, &force(value));
    }
    assert_eq!(r.resolution_divisor, Some(12));
    r.debug_force(B, &force("4"));
    assert_eq!(r.resolution_divisor, Some(12), "another issuer cannot bypass the debug lock");
    r.debug_force(A, &force("default"));
    assert_eq!(r.resolution_divisor, None);
    r.debug_force(A, &force("4"));
    r.configure(false);
    assert_eq!(r.resolution_divisor, None);
    r.debug_force(A, &force("4"));
    assert_eq!(r.resolution_divisor, None);
    assert!(command_list("renderresolutiondivisor;force").contains("setdebug_renderresolutiondivisor"));
}

#[test]
fn rlva_attachment_replacement_preserves_locked_cof_links_but_explicit_removal_is_denied() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        inventory::{InvFolder, InvItem},
        outfits::{OutfitLink, OutfitMutation},
    };
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    let cof = Uuid::from_u128(10);
    let old = Uuid::from_u128(11);
    let new = Uuid::from_u128(12);
    let link = Uuid::from_u128(13);
    w.inventory.folders.insert(
        cof,
        Folder {
            info: InvFolder {
                id: cof,
                type_default: 46,
                ..Default::default()
            },
            items: vec![link],
            children: vec![],
            state: FetchState::Fetched,
            library: false,
        },
    );
    for id in [old, new] {
        w.inventory.items.insert(
            id,
            InvItem {
                id,
                asset_type: 6,
                flags: 5,
                ..Default::default()
            },
        );
    }
    w.inventory.items.insert(
        link,
        InvItem {
            id: link,
            parent: cof,
            asset_id: old,
            asset_type: 24,
            ..Default::default()
        },
    );
    command(&mut w.rlva, A, "@remattach:left hand=n");
    let mut cmd = aurora_net::NetCommand::UpdateOutfit {
        request: TARGET,
        change: OutfitMutation {
            folder: cof,
            create: None,
            cof: None,
            links: vec![OutfitLink {
                target: new,
                name: "new".into(),
                desc: String::new(),
                folder: false,
                inv_type: 6,
            }],
        },
    };
    let targets = |cmd: &aurora_net::NetCommand| {
        let aurora_net::NetCommand::UpdateOutfit { change, .. } = cmd else {
            unreachable!()
        };
        change.links.iter().map(|l| l.target).collect::<Vec<_>>()
    };
    let unprepared = cmd.clone();
    assert!(
        !w.rlva_allows_command(&cmd, Uuid::nil()),
        "the raw change would take off the locked item"
    );
    w.rlva.preferences.replace_unlocked = false;
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    assert_eq!(
        targets(&cmd),
        [old],
        "strict replacement keeps the locked item and leaves the new one out"
    );
    assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
    w.rlva.preferences.replace_unlocked = true;
    let mut cmd = unprepared;
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    assert_eq!(targets(&cmd), [new, old], "preparation is idempotent");
    assert!(!w.rlva_can_remove(old, Uuid::nil()));
    assert!(!w.rlva_can_remove(old, watchdog::ACTOR));
    assert!(!w.rlva_can_add(new, 5, watchdog::ACTOR));
    let aurora_net::NetCommand::UpdateOutfit { change, .. } = &mut cmd else {
        unreachable!()
    };
    change.links.clear();
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    assert_eq!(targets(&cmd), [old], "taking everything off still leaves the locked item on");
    assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
    w.rlva.configure(false);
    assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
}

/// LLAppearanceMgr::updateCOF (llappearancemgr.cpp): replacing or taking
/// off an outfit under clothing and body-part locks keeps the locked layers
/// and blocked additions out, and changes everything else.
#[test]
fn rlva_outfit_changes_keep_locked_wearables_and_skip_blocked_additions() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        inventory::{InvFolder, InvItem},
        outfits::{OutfitLink, OutfitMutation},
    };
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    let cof = Uuid::from_u128(20);
    // (id, asset type, wearable type): shirt, shape, pants, new shape,
    // new jacket, new socks, new attachment.
    let items = [(21, 5, 4), (22, 13, 0), (23, 5, 5), (24, 13, 0), (25, 5, 8), (26, 5, 7), (27, 6, 9)];
    let link = |id: u128| OutfitLink {
        target: Uuid::from_u128(id),
        name: String::new(),
        desc: String::new(),
        folder: false,
        inv_type: 0,
    };
    w.inventory.folders.insert(
        cof,
        Folder {
            info: InvFolder {
                id: cof,
                type_default: 46,
                ..Default::default()
            },
            items: vec![],
            children: vec![],
            state: FetchState::Fetched,
            library: false,
        },
    );
    for (n, (id, asset_type, flags)) in items.into_iter().enumerate() {
        w.inventory.items.insert(
            Uuid::from_u128(id),
            InvItem {
                id: Uuid::from_u128(id),
                asset_type,
                flags,
                ..Default::default()
            },
        );
        if n < 3 {
            let cof_link = Uuid::from_u128(id + 100);
            w.inventory.items.insert(
                cof_link,
                InvItem {
                    id: cof_link,
                    parent: cof,
                    asset_id: Uuid::from_u128(id),
                    asset_type: 24,
                    ..Default::default()
                },
            );
            w.inventory.folders.get_mut(&cof).expect("cof").items.push(cof_link);
        }
    }
    command(&mut w.rlva, A, "@remoutfit:shirt=n");
    command(&mut w.rlva, A, "@remoutfit:shape=n");
    command(&mut w.rlva, A, "@addoutfit:socks=n");
    let mut cmd = aurora_net::NetCommand::UpdateOutfit {
        request: TARGET,
        change: OutfitMutation {
            folder: cof,
            create: None,
            cof: None,
            links: vec![link(24), link(25), link(26), link(27)],
        },
    };
    assert!(!w.rlva_allows_command(&cmd, Uuid::nil()));
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    let aurora_net::NetCommand::UpdateOutfit { change, .. } = &cmd else {
        unreachable!()
    };
    let mut kept: Vec<_> = change.links.iter().map(|l| l.target.as_u128()).collect();
    kept.sort_unstable();
    assert_eq!(
        kept,
        [21, 22, 25, 27],
        "locked shirt and shape stay, unlocked pants go, the new shape and the socks are left out"
    );
    assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
}

/// Wearing three attachments, one on a point locked by @addattach: the
/// outfit write and the rez that follows both leave out the blocked one
/// only, in strict mode too, instead of RLVa refusing the whole rez.
#[test]
fn rlva_multi_wear_leaves_out_only_the_blocked_attachment() {
    use crate::world::appearance::{Action, attachments_for_action};
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        inventory::{InvFolder, InvItem},
        outfits::{OutfitLink, OutfitMutation},
    };
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    let cof = Uuid::from_u128(40);
    w.inventory.folders.insert(
        cof,
        Folder {
            info: InvFolder {
                id: cof,
                type_default: 46,
                ..Default::default()
            },
            items: vec![],
            children: vec![],
            state: FetchState::Fetched,
            library: false,
        },
    );
    // (id, attachment point): spine, skull, left hand.
    let items = [(41, 9), (42, 2), (43, 5)];
    for (id, point) in items {
        w.inventory.items.insert(
            Uuid::from_u128(id),
            InvItem {
                id: Uuid::from_u128(id),
                asset_type: 6,
                flags: point,
                ..Default::default()
            },
        );
    }
    command(&mut w.rlva, A, "@addattach:spine=n");
    let ids: Vec<_> = items.iter().map(|(id, _)| Uuid::from_u128(*id)).collect();
    for replace_unlocked in [true, false] {
        w.rlva.preferences.replace_unlocked = replace_unlocked;
        let mut cmd = aurora_net::NetCommand::UpdateOutfit {
            request: TARGET,
            change: OutfitMutation {
                folder: cof,
                create: None,
                cof: None,
                links: ids
                    .iter()
                    .map(|id| OutfitLink {
                        target: *id,
                        name: String::new(),
                        desc: String::new(),
                        folder: false,
                        inv_type: 18,
                    })
                    .collect(),
            },
        };
        let dropped = w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
        assert_eq!(dropped, [ids[0]]);
        assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
        let action = Action::WearItems {
            items: ids.clone(),
            replace: true,
            point: 0,
        };
        let mut attachments = attachments_for_action(&w.inventory, TARGET, &action);
        assert_eq!(attachments.len(), 3);
        assert!(
            !w.rlva_allows_command(&aurora_net::NetCommand::RezAttachments(attachments.clone()), Uuid::nil()),
            "the unfiltered rez is refused as a whole"
        );
        attachments.retain(|a| !dropped.contains(&a.item_id));
        assert_eq!(attachments.iter().map(|a| a.item_id).collect::<Vec<_>>(), ids[1..]);
        assert!(w.rlva_allows_command(&aurora_net::NetCommand::RezAttachments(attachments), Uuid::nil()));
    }
}

fn chat(text: &str) -> ChatMessage {
    ChatMessage {
        from_name: "RLVa test object".to_owned(),
        source_id: A,
        owner_id: TARGET,
        source_type: ChatSourceType::Object,
        chat_type: ChatType::OwnerSay,
        position: glam::Vec3::ZERO,
        message: text.to_owned(),
    }
}

#[test]
fn rlva_presence_queries_hide_locked_slots_but_exempt_the_requester_and_keep_bodyparts() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        AttachRequest,
        inventory::{InvFolder, InvItem},
    };
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    world.rlva.preferences.hide_locked_layers = true;
    world.rlva.preferences.hide_locked_attachments = true;
    let cof = Uuid::from_u128(900);
    let shirt = Uuid::from_u128(901);
    let shape = Uuid::from_u128(902);
    let attachment = Uuid::from_u128(903);
    world.inventory.folders.insert(
        cof,
        Folder {
            info: InvFolder {
                id: cof,
                parent: world.inventory.root,
                type_default: 46,
                ..Default::default()
            },
            children: vec![],
            items: vec![shirt, shape, attachment],
            state: FetchState::Fetched,
            library: false,
        },
    );
    for (id, kind, flags) in [(shirt, 5, 4), (shape, 13, 0), (attachment, 6, 1)] {
        world.inventory.items.insert(
            id,
            InvItem {
                id,
                parent: cof,
                name: "Item".into(),
                asset_type: kind,
                flags,
                ..Default::default()
            },
        );
    }
    let object = crate::demo::outfit_attachment(&AttachRequest {
        item_id: attachment,
        owner_id: world.agent_id,
        point: 1,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    });
    world.objects.upsert(world.main_region.unwrap(), object);
    command(&mut world.rlva, A, "@remoutfit=n,remattach:chest=n");
    let shirt_query = Command::parse("getoutfit:shirt=123").unwrap();
    let attachment_query = Command::parse("getattach:chest=123").unwrap();
    for query in [&shirt_query, &attachment_query] {
        assert_eq!(world.rlva_inventory_query(query, A).as_deref(), Some("1"));
        assert_eq!(world.rlva_inventory_query(query, B).as_deref(), Some("0"));
    }
    assert_eq!(
        world
            .rlva_inventory_query(&Command::parse("getoutfit:shape=123").unwrap(), B)
            .as_deref(),
        Some("1")
    );
    assert!(
        world
            .rlva_inventory_query(&Command::parse("getoutfitnames=123").unwrap(), B)
            .unwrap()
            .split(',')
            .any(|name| name == "shirt")
    );
    assert!(
        !world.rlva_can_remove(shirt, A),
        "query exemption does not authorize a forced removal through its own lock"
    );
    command(&mut world.rlva, B, "@remoutfit=n,remattach:chest=n");
    for query in [&shirt_query, &attachment_query] {
        assert_eq!(world.rlva_inventory_query(query, A).as_deref(), Some("0"));
    }
    command(&mut world.rlva, A, "@clear");
    command(&mut world.rlva, B, "@clear");
    world.inventory.items.get_mut(&shirt).unwrap().name = "nostrip".into();
    assert_eq!(world.rlva_inventory_query(&shirt_query, A).as_deref(), Some("0"));
    world.rlva.preferences.hide_locked_layers = false;
    assert_eq!(world.rlva_inventory_query(&shirt_query, A).as_deref(), Some("1"));
}

#[test]
fn rlva_maturity_deferred_teleports_keep_the_actor_recheck_locks_and_cancel_on_disable() {
    use crate::world::maturity::{MaturityPreference, Outcome};
    use aurora_net::search::{SIM_ACCESS_MATURE, SIM_ACCESS_PG};
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    let mut maturity = MaturityPreference::default();
    maturity.request(SIM_ACCESS_MATURE, SIM_ACCESS_PG);
    maturity.take_commands();
    maturity.hold(NetCommand::TeleportHome, A);
    command(&mut world.rlva, A, "@tplm=n");
    assert_eq!(maturity.answer(Some(SIM_ACCESS_MATURE)), Outcome::Synced);
    let (actor, teleport) = maturity.take_commands().pop().unwrap();
    assert_eq!(actor, A);
    assert!(
        world.rlva_allows_command(&teleport, actor),
        "the original issuer can still force through its own lock"
    );
    command(&mut world.rlva, B, "@tplm=n");
    assert!(
        !world.rlva_allows_command(&teleport, actor),
        "a new other issuer's lock applies before the delayed send"
    );
    for release_first in [false, true] {
        let mut maturity = MaturityPreference::default();
        maturity.request(SIM_ACCESS_MATURE, SIM_ACCESS_PG);
        maturity.take_commands();
        maturity.hold(NetCommand::TeleportHome, A);
        if release_first {
            maturity.answer(Some(SIM_ACCESS_MATURE));
        }
        maturity.cancel_script_teleports();
        if !release_first {
            maturity.answer(Some(SIM_ACCESS_MATURE));
        }
        assert!(maturity.take_commands().is_empty());
        maturity.hold(NetCommand::TeleportHome, Uuid::nil());
        maturity.cancel_script_teleports();
        assert!(
            maturity.holds_teleport(),
            "the user's own queued action survives switching RLVa off"
        );
    }
}

#[test]
fn rlva_temporary_attachment_locks_and_detachme_do_not_require_inventory_or_cof() {
    use aurora_net::{AttachRequest, NetCommand};
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let region = world.main_region.unwrap();
    let mut temporary = crate::demo::outfit_attachment(&AttachRequest {
        item_id: A,
        owner_id: world.agent_id,
        point: 5,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    });
    temporary.name_values.clear();
    world.objects.upsert(region, temporary);
    let key = world.rlva_temporary_attachment(A).unwrap();
    assert_eq!(world.rlva_worn_attachment_points().get(&A), Some(&5));
    assert_eq!(world.rlva_attachment_item(A), Some(A));
    command(&mut world.rlva, A, "@detach=n");
    let detach = NetCommand::DetachObject {
        handle: key.region,
        local_id: key.local_id,
    };
    assert!(!world.rlva_allows_command(&detach, Uuid::nil()));
    assert!(
        !world.rlva_allows_command(&detach, A),
        "force commands obey their own attachment locks"
    );
    command(&mut world.rlva, A, "@detachme=force");
    assert!(world.rlva_pending().is_empty(), "detachme skips nostrip, not attachment locks");
    command(&mut world.rlva, A, "@detach=y,detachme=force");
    assert!(
        world
            .rlva_pending()
            .iter()
            .any(|(actor, cmd)| *actor == A && matches!(cmd, NetCommand::DetachObject { .. }))
    );
    command(&mut world.rlva, B, "@remattach:left hand=n");
    assert!(
        !world.rlva_allows_command(&detach, A),
        "another source's point lock remains authoritative"
    );
    world.inventory.items.insert(
        A,
        aurora_net::inventory::InvItem {
            id: A,
            asset_type: 6,
            flags: 5,
            ..Default::default()
        },
    );
    let move_to_other_point = NetCommand::RezAttachments(vec![AttachRequest {
        item_id: A,
        owner_id: world.agent_id,
        point: 6,
        add: true,
        flags: 5,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    }]);
    assert!(
        !world.rlva_allows_command(&move_to_other_point, A),
        "changing points would remove a locked attachment"
    );
    command(&mut world.rlva, B, "@clear");
    assert!(
        world.rlva_allows_command(&move_to_other_point, A),
        "releasing all point locks permits moving the attachment"
    );
    assert!(world.rlva_allows_command(&move_to_other_point, Uuid::nil()));
    world.inventory.items.remove(&A);
    world.rlva_watchdog_tick(std::time::Instant::now());
    world.objects.remove(&key);
    world.rlva_watchdog_tick(std::time::Instant::now());
    assert!(
        world.rlva.watchdog.pending.is_empty(),
        "a temporary object has no inventory asset to restore"
    );
    world.rlva.configure(false);
    assert!(world.rlva.watchdog.known.is_empty() && world.rlva.watchdog.sources.is_empty());
}

fn command(r: &mut Rlva, source: Uuid, text: &str) {
    let mut c = chat(text);
    c.source_id = source;
    assert!(r.receive(&c));
}

fn enabled() -> Rlva {
    let mut r = Rlva::default();
    r.configure(true);
    r
}

#[test]
fn rlva_visual_effect_lifetimes_capacity_and_disable_follow_the_issuing_object() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        "@setsphere=n,setsphere_color:0.1/0.2/0.3=force,setoverlay=n,setoverlay_texture:00000000-0000-0000-0000-000000000003=force",
    );
    assert_eq!(r.visuals.spheres[&A].sample(std::time::Instant::now()).0, [0.1, 0.2, 0.3, 0.0]);
    assert_eq!(r.visuals.overlays[&A].texture, TARGET);
    command(&mut r, B, "@setsphere_color:1/0/0=force");
    assert!(!r.visuals.spheres.contains_key(&B), "modifiers require an active issuing object");
    for i in 10..16 {
        command(&mut r, Uuid::from_u128(i), "@setsphere=n");
    }
    assert_eq!(r.visuals.sphere_order.len(), 6);
    command(&mut r, A, "@clear=setsphere");
    assert!(!r.visuals.spheres.contains_key(&A));
    assert!(r.visuals.overlays.contains_key(&A));
    r.configure(false);
    assert!(r.visuals.spheres.is_empty() && r.visuals.overlays.is_empty());
}

#[test]
fn rlva_overlay_touch_mask_keeps_transparent_holes_and_matches_the_reference_threshold() {
    let mut rgba = vec![0; 4 * 2 * 4];
    rgba[3] = 32;
    rgba[2 * 4 + 3] = 33;
    let mask = effects::AlphaMask::from_rgba(4, 2, &rgba).unwrap();
    assert!(!mask.hit(0.1, 0.5));
    assert!(mask.hit(0.9, 0.5));
    assert!(effects::AlphaMask::from_rgba(4, 2, &[0]).is_none());
}

#[test]
fn rlva_is_explicitly_opt_in_and_disable_releases_everything() {
    assert!(!crate::settings::Settings::default().rlva_enabled);
    let decoded: crate::settings::Settings = serde_json::from_str("{}").expect("old settings");
    assert!(!decoded.rlva_enabled);
    let mut r = Rlva::default();
    assert!(!r.receive(&chat("@fly=n,version=123")));
    assert!(r.out.is_empty());
    r.configure(true);
    command(&mut r, A, "@fly=n,version=123,sitground=force");
    assert!(r.has("fly"));
    assert!(!r.out.is_empty());
    assert!(!r.pending.is_empty());
    r.configure(false);
    assert!(!r.has("fly"));
    assert!(r.out.is_empty());
    assert!(r.pending.is_empty());
    r.configure(true);
    assert!(!r.has("fly"));
}

#[test]
fn rlva_only_object_owner_say_can_add_restrictions() {
    let mut r = enabled();
    for (source, kind) in [
        (ChatSourceType::Agent, ChatType::OwnerSay),
        (ChatSourceType::Object, ChatType::Normal),
        (ChatSourceType::System, ChatType::OwnerSay),
    ] {
        let mut c = chat("@fly=n");
        c.source_type = source;
        c.chat_type = kind;
        assert!(!r.receive(&c));
    }
    assert!(!r.has("fly"));
    command(&mut r, A, "@fly=n");
    assert!(r.has("fly"));
}

#[test]
fn rlva_rules_are_idempotent_scoped_to_the_issuer_and_clear_filters() {
    let mut r = enabled();
    command(&mut r, A, "@fly=n,fly=n,jump=add");
    command(&mut r, B, "@fly=n");
    command(&mut r, A, "@fly=y");
    assert!(r.has("fly"));
    command(&mut r, B, "@clear=fly");
    assert!(!r.has("fly"));
    assert!(r.has("jump"));
    command(&mut r, A, "@clear");
    assert!(!r.has("jump"));
}

#[test]
fn rlva_strict_exceptions_require_secure_issuers_and_permissive_requires_all_issuers() {
    let mut r = enabled();
    command(&mut r, A, "@sendim_sec=n");
    command(&mut r, B, &format!("@sendim=n,sendim:{TARGET}=n"));
    assert!(!r.allows_im("sendim", TARGET));
    command(&mut r, A, &format!("@sendim:{TARGET}=n"));
    assert!(r.allows_im("sendim", TARGET));
    command(&mut r, B, &format!("@sendim:{TARGET}=y"));
    assert!(
        r.allows_im("sendim", TARGET),
        "a non-secure issuer need not repeat a secure issuer's exception"
    );
    command(&mut r, B, "@permissive=n");
    assert!(!r.allows_im("sendim", TARGET));
    command(&mut r, B, &format!("@sendim:{TARGET}=n"));
    assert!(r.allows_im("sendim", TARGET));
    command(&mut r, B, &format!("@sendimto:{TARGET}=n"));
    assert!(!r.allows_im("sendim", TARGET));
}

#[test]
fn rlva_bad_options_unknown_commands_and_public_reply_channels_have_no_effect() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        "@fly:bad=n,unknown=n,fly_sec=n,sendim:no-uuid=n,version=0,version=-3,version=2147483647",
    );
    assert!(r.rules.is_empty());
    assert!(r.out.is_empty());
    assert!(Command::parse("=n").is_none());
    assert!(Command::parse("fly=").is_none());
    assert!(Command::parse("fly=2147483648").is_none());
    command(&mut r, A, "@fly=n,jump=invalid,unsit=n");
    assert!(r.has("fly") && r.has("unsit"));
}

fn sent(commands: Vec<NetCommand>) -> Vec<(String, i32, ChatType)> {
    commands
        .into_iter()
        .map(|c| match c {
            NetCommand::Chat {
                message,
                channel,
                chat_type,
            } => (message, channel, chat_type),
            NetCommand::ChatReply { message, channel } => (message, channel, ChatType::Shout),
            _ => panic!("chat"),
        })
        .collect()
}

#[test]
fn rlva_redirection_is_deduplicated_and_never_leaks_to_public_chat() {
    let mut r = enabled();
    command(&mut r, A, "@sendchat=n,redirchat:456=n");
    command(&mut r, B, "@redirchat:456=n,redirchat:789=n");
    assert_eq!(
        sent(r.outgoing_chat("hello", 0, ChatType::Normal)),
        vec![
            ("hello".to_owned(), 456, ChatType::Shout),
            ("hello".to_owned(), 789, ChatType::Shout)
        ]
    );
    command(&mut r, A, "@sendchannel=n");
    assert!(r.outgoing_chat("hello", 0, ChatType::Normal).is_empty());
    assert_eq!(sent(r.outgoing_chat("((test))", 0, ChatType::Normal))[0].0, "((test))");
    assert!(r.outgoing_chat("hello", 99, ChatType::Normal).is_empty());
}

#[test]
fn rlva_chat_filter_is_utf8_safe_and_preserves_ooc_and_short_commands() {
    let mut r = enabled();
    command(&mut r, A, "@sendchat=n,chatshout=n,chatnormal=n");
    assert_eq!(
        sent(r.outgoing_chat("hello", 0, ChatType::Shout)),
        vec![("...".to_owned(), 0, ChatType::Whisper)]
    );
    let text = "/me éééééééééééééééééééééééé";
    let result = sent(r.outgoing_chat(text, 0, ChatType::Normal));
    assert_eq!(result[0].0.chars().count(), 20);
    assert_eq!(sent(r.outgoing_chat("/test", 0, ChatType::Normal))[0].0, "/test");
    assert_eq!(sent(r.outgoing_chat("/me says (hello)", 0, ChatType::Normal))[0].0, "...");
}

#[test]
fn rlva_queries_report_only_consumed_commands_and_object_scoped_status() {
    let mut r = enabled();
    command(&mut r, A, "@fly=n,getstatus=999,getcommand:setoverlay;any=998");
    command(&mut r, B, "@getstatus=997");
    let replies = sent(std::mem::take(&mut r.out));
    assert_eq!(replies[0].0, "/fly");
    assert!(replies[1].0.split(';').any(|name| name == "setoverlay"));
    assert_eq!(replies[2].0, "");
}

#[test]
fn rlva_garbage_collection_keeps_unknown_issuers_and_releases_lost_known_objects() {
    let mut r = enabled();
    command(&mut r, A, "@fly=n");
    command(&mut r, B, "@jump=n");
    let now = std::time::Instant::now();
    r.collect(now, |id| id == A);
    r.collect(now + std::time::Duration::from_secs(29), |_| false);
    assert!(r.has("fly"));
    r.collect(now + std::time::Duration::from_secs(30), |_| false);
    assert!(!r.has("fly"));
    assert!(r.has("jump"));
    for n in 2..=20 {
        r.collect(now + std::time::Duration::from_secs(n * 30), |_| false);
    }
    assert!(!r.has("jump"));
}

#[test]
fn rlva_camera_owner_and_clamps_are_temporary_and_reject_nonfinite_values() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        "@setcam=n,setcam_avdistmin:2=n,setcam_fovmin:0.8=n,setcam_eyeoffset:-4/0/1=n",
    );
    command(&mut r, B, "@setcam=n,setcam_avdistmin:10=n,setcam_fovmin:NaN=n,setcam_fovmax:100=n");
    assert_eq!(r.camera_owner(), Some(A));
    assert_eq!(r.camera_limit("setcam_avdistmin", 0.0, true), 2.0);
    let settings = crate::camera::CameraSettings::default();
    assert_eq!(r.camera_settings(&settings, 1.0).camera_offset, [-4.0, 0.0, 1.0]);
    command(&mut r, A, "@clear");
    assert_eq!(r.camera_limit("setcam_avdistmin", 0.0, true), 10.0);
    r.configure(false);
    assert_eq!(r.camera_settings(&settings, 1.0), settings);
}

#[test]
fn rlva_camera_replies_and_forces_match_modifier_lifetimes_and_preset_locks() {
    let mut r = enabled();
    let query = |name: &str| Command::parse(&format!("{name}=123")).unwrap();
    for name in [
        "getcam_fovmin",
        "getcam_fovmax",
        "getcam_avdistmin",
        "getcam_avdistmax",
        "getcam_textures",
    ] {
        assert_eq!(r.camera_reply(&query(name), 5.0, 0.8).as_deref(), Some(""));
    }
    assert_eq!(r.camera_reply(&query("getcam_avdist"), 1.23456, 0.8).as_deref(), Some("1.235"));
    assert_eq!(r.camera_reply(&query("getcam_fov"), 5.0, 0.8).as_deref(), Some("0.800"));
    command(&mut r, B, "@setcam_avdistmin:20=n,setcam_fovmax:2=n");
    command(&mut r, A, "@setcam=n");
    assert_eq!(
        r.camera_reply(&query("getcam_avdistmin"), 5.0, 0.8).as_deref(),
        Some(""),
        "non-owner modifiers stay inactive"
    );
    let force = |name, option| Command::parse(&format!("{name}:{option}=force")).unwrap();
    r.camera_force(B, &force("setcam_fov", "1.2"));
    assert!(r.camera_values.is_empty());
    r.camera_force(A, &force("setcam_eyeoffsetscale", "0"));
    assert_eq!(r.camera_settings(&crate::camera::CameraSettings::default(), 1.0).offset_scale, 0.0);
    command(&mut r, A, "@setcam_eyeoffset:-4/0/1=n,setcam_fovmin:0.5=n");
    r.camera_force(A, &force("setcam_eyeoffsetscale", "3"));
    assert_eq!(
        r.camera_values["setcam_eyeoffsetscale"], "0",
        "an offset restriction locks forced preset writes too"
    );
    r.camera_force(A, &force("setcam_fov", "1.2"));
    assert_eq!(r.camera_settings(&crate::camera::CameraSettings::default(), 1.0).fov, 1.2);
    r.camera_force(A, &force("setcam_fov", ""));
    assert_eq!(
        r.camera_settings(&crate::camera::CameraSettings::default(), 1.0).fov,
        crate::camera::DEFAULT_FOV
    );
    command(&mut r, A, "@clear");
    assert_eq!(r.camera_reply(&query("getcam_avdistmin"), 5.0, 0.8).as_deref(), Some("20.000"));
    command(&mut r, B, "@clear");
    assert!(
        !r.camera_values.contains_key("setcam_fov"),
        "last limit release restores the user's FOV"
    );
    command(&mut r, A, "@camzoommin=n");
    assert_eq!(r.camera_reply(&query("getcam_fovmin"), 5.0, 0.8).as_deref(), Some("1.047"));
    r.configure(false);
    assert_eq!(
        r.camera_settings(&crate::camera::CameraSettings::default(), 1.0),
        crate::camera::CameraSettings::default()
    );
}

#[test]
fn rlva_camera_focus_defaults_fit_objects_and_positions_without_changing_identity() {
    use camera::{FocusRequest, FocusTarget};
    use glam::Vec3;
    let object = FocusRequest::parse(&TARGET.to_string()).unwrap();
    assert_eq!(object.target, FocusTarget::Object(TARGET));
    let at = Vec3::new(10.0, 20.0, 30.0);
    let camera = object.camera(at, Vec3::new(2.0, 3.0, 6.0), at + Vec3::X * 100.0).unwrap();
    assert_eq!(
        camera,
        at + Vec3::X * 7.0,
        "default object distance is its scale length, not current distance"
    );
    let position = FocusRequest::parse("10/20/30").unwrap();
    assert_eq!(position.target, FocusTarget::Position(at));
    assert!(position.camera(at, Vec3::ZERO, at).unwrap().distance(at) < 0.002);
    let explicit = FocusRequest::parse(&format!("{TARGET};4;0/0/-2")).unwrap();
    assert_eq!(explicit.camera(at, Vec3::ZERO, at).unwrap(), at - Vec3::Z * 4.0);
    for text in ["", "10/20/NaN", "10/20/30;NaN", "10/20/30;;1/0", "10/20/30;4;1/0/0;extra"] {
        assert!(FocusRequest::parse(text).is_none(), "{text}");
    }
}

#[test]
fn rlva_camera_preference_mutations_preserve_locked_fields_and_allow_unrelated_options() {
    let mut r = enabled();
    let before = crate::camera::CameraSettings::default();
    let mut requested = before.clone();
    requested.camera_offset = [-8.0, 2.0, 3.0];
    requested.fov = 0.8;
    requested.zoom_time = 2.0;
    command(&mut r, A, "@setcam_eyeoffset:0/0/2=n");
    r.preserve_camera_settings(&mut requested, &before);
    assert_eq!(requested.camera_offset, before.camera_offset);
    assert_eq!(requested.fov, 0.8, "preset locks do not lock the FOV");
    assert_eq!(requested.zoom_time, 2.0);
    command(&mut r, A, "@setcam=n");
    r.preserve_camera_settings(&mut requested, &before);
    assert_eq!(requested.fov, before.fov);
    r.configure(false);
    requested.camera_offset = [-8.0, 2.0, 3.0];
    r.preserve_camera_settings(&mut requested, &before);
    assert_eq!(requested.camera_offset, [-8.0, 2.0, 3.0]);
}

#[test]
fn rlva_command_discovery_includes_strict_and_supported_environment_variants() {
    assert_eq!(command_list("shownames;add"), "shownames;shownames_sec");
    assert_eq!(command_list("shownames;add;"), "shownamesshownames_sec");
    for set in [false, true] {
        let commands = environment::command_names(set);
        assert!(commands.iter().all(|name| environment::supports(name, set)));
        let listed = command_list(if set { "setenv_;force" } else { "getenv_;reply" });
        for name in commands {
            assert!(listed.split(';').any(|n| n == name), "{name}");
        }
    }
    let mut r = enabled();
    command(&mut r, A, "@setcam_fov_sec:1=force,getcam_fov_sec=123,getenv_daytime_sec=123");
    assert!(r.pending.is_empty() && r.out.is_empty(), "only restrictions accept strict suffixes");
}

#[test]
fn rlva_shared_wear_and_nostrip_apply_to_scripted_undressing() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.inventory.root = Uuid::from_u128(10);
    let root = Uuid::from_u128(11);
    w.inventory.folders.insert(
        root,
        Folder {
            info: InvFolder {
                id: root,
                parent: w.inventory.root,
                name: "#RLV".to_owned(),
                ..Default::default()
            },
            children: Vec::new(),
            items: vec![TARGET],
            state: FetchState::Fetched,
            library: false,
        },
    );
    w.inventory.items.insert(
        TARGET,
        InvItem {
            id: TARGET,
            parent: root,
            name: "nostrip shirt".to_owned(),
            asset_type: 5,
            flags: 4,
            ..Default::default()
        },
    );
    w.rlva.configure(true);
    assert!(w.rlva_can_remove(TARGET, Uuid::nil()));
    assert!(!w.rlva_can_remove(TARGET, A));
    command(&mut w.rlva, A, "@addoutfit:shirt=n,remoutfit:shirt=n");
    assert!(!w.rlva_can_add(TARGET, 0, Uuid::nil()));
    assert!(!w.rlva_can_remove(TARGET, Uuid::nil()));
    assert!(!w.rlva_can_add(TARGET, 0, A), "own wearable locks still apply to forces");
    command(&mut w.rlva, A, "@clear");
    command(&mut w.rlva, B, "@sharedwear=n");
    assert!(!w.rlva_can_add(TARGET, 0, A));
    w.rlva.configure(false);
    assert!(w.rlva_can_add(TARGET, 0, Uuid::nil()));
    assert!(w.rlva_can_remove(TARGET, A));
}

#[test]
fn rlva_inventory_queries_keep_the_shared_root_and_report_descendants() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    let root = Uuid::from_u128(11);
    let child = Uuid::from_u128(12);
    w.inventory.root = Uuid::from_u128(10);
    for (id, parent, name, children, items) in [
        (root, w.inventory.root, "#RLV", vec![child], vec![]),
        (child, root, "Test outfit", vec![], vec![TARGET]),
    ] {
        w.inventory.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.to_owned(),
                    ..Default::default()
                },
                children,
                items,
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    w.inventory.items.insert(
        TARGET,
        InvItem {
            id: TARGET,
            parent: child,
            asset_type: 5,
            ..Default::default()
        },
    );
    let mut query = Command::parse("getinv=123").expect("query");
    assert_eq!(w.rlva_inventory_query(&query, A).as_deref(), Some("Test outfit"));
    query.option = "../outside".to_owned();
    assert_eq!(w.rlva_inventory_query(&query, A).as_deref(), Some(""));
    query = Command::parse("getinvworn=123").expect("query");
    assert_eq!(w.rlva_inventory_query(&query, A).as_deref(), Some("|01,Test outfit|10"));
    w.inventory.folders.get_mut(&child).expect("folder").state = FetchState::Unknown;
    assert!(w.rlva_inventory_query(&query, A).is_none(), "unloaded is not empty");
}

#[test]
fn rlva_world_receives_commands_before_chat_and_blocks_real_action_variants() {
    let lib = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut w = crate::world::World::new(lib);
    w.rlva.configure(true);
    w.apply(aurora_net::NetEvent::Chat(chat("@unsit=n,pay=n,tploc=n")));
    assert!(w.rlva.has("unsit"));
    assert!(!w.rlva_allows_command(&NetCommand::OneShotControl(aurora_net::control::STAND_UP), Uuid::nil()));
    assert!(w.rlva_allows_command(&NetCommand::OneShotControl(aurora_net::control::STAND_UP), A));
    assert!(!w.rlva_allows_command(
        &NetCommand::PayResident {
            to: TARGET,
            group: false,
            amount: 1,
            message: String::new()
        },
        Uuid::nil()
    ));
    w.rlva.configure(false);
    assert!(w.rlva_allows_command(&NetCommand::OneShotControl(aurora_net::control::STAND_UP), Uuid::nil()));
}

#[test]
fn rlva_notifications_include_duplicate_and_rejected_commands_and_stop_on_release() {
    let mut r = enabled();
    command(&mut r, A, "@notify:123;fly=n");
    assert!(matches!(r.out.last(), Some(NetCommand::ChatReply { message, .. }) if message == "/notify:123;fly=n"));
    r.out.clear();
    command(&mut r, B, "@fly=n,fly=n,fly:invalid=n,jump=n,fly=y");
    let messages: Vec<_> = r
        .out
        .iter()
        .filter_map(|c| match c {
            NetCommand::ChatReply { channel: 123, message, .. } => Some(message.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(messages, ["/fly=n", "/fly=n", "/fly:invalid=n", "/fly=y"]);
    r.out.clear();
    command(&mut r, A, "@clear");
    command(&mut r, B, "@fly=n");
    assert!(r.out.is_empty());
    command(&mut r, A, "@notify:0=n,notify:-1=n,notify:2147483647=n");
    assert!(r.rules_for("notify", Uuid::nil()).next().is_none());
}

#[test]
fn rlva_environment_preserves_slider_scales_components_angles_and_session_lifetime() {
    use super::environment::property;
    let mut sky = crate::world::eep::SkyFrame::default();
    property(&mut sky, "ambient", Some("0.2/0.4/0.6")).unwrap();
    assert!(sky.ambient.abs_diff_eq(glam::Vec3::new(0.6, 1.2, 1.8), 1e-5));
    property(&mut sky, "ambienti", Some("0.3")).unwrap();
    assert_eq!(property(&mut sky, "ambient", None).unwrap(), "0.100000/0.200000/0.300000");
    property(&mut sky, "densitymultiplier", Some("0.5")).unwrap();
    assert!((sky.density_multiplier - 0.0005).abs() < 1e-7);
    property(&mut sky, "eastangle", Some("0.25")).unwrap();
    property(&mut sky, "sunmoonposition", Some("0.125")).unwrap();
    assert!(sky.sun_direction().abs_diff_eq(-sky.moon_direction(), 1e-5));
    assert_eq!(property(&mut sky, "eastangle", None).unwrap(), "0.250000");
    assert!(property(&mut sky, "ambient", Some("NaN/1/1")).is_none());
    assert!(property(&mut sky, "sunazimuth", Some("inf")).is_none());
    let mut r = enabled();
    r.environment = Some((A, sky));
    command(&mut r, A, "@setenv=n");
    command(&mut r, B, "@setenv=n");
    assert!(!r.has_except("setenv", A));
    command(&mut r, A, "@setenv=y");
    assert!(r.environment.is_none());
    r.environment = Some((A, sky));
    r.forget(A);
    assert!(r.environment.is_none());
}

#[test]
fn rlva_editing_and_permissions_are_enforced_on_linksets_and_wire_commands() {
    use aurora_net::{NetEvent, build::BuildCmd};
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for ev in crate::demo::events().into_iter().chain(crate::demo::worn::events()) {
        w.apply(ev);
    }
    w.rlva.configure(true);
    let root = w
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == crate::demo::worn::ROOT)
        .map(|(i, _)| i)
        .unwrap();
    let child = w
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == crate::demo::worn::CHILD)
        .map(|(i, _)| i)
        .unwrap();
    let object = w.objects.get(root).unwrap();
    let (id, key) = (object.full_id, object.key);
    command(&mut w.rlva, A, "@edit=n");
    assert!(!crate::build::selectable(&w, child));
    let edit = NetCommand::Build(BuildCmd::SetName {
        handle: key.region,
        local_id: key.local_id,
        name: "new".into(),
    });
    assert!(!w.rlva_allows_command(&edit, Uuid::nil()));
    command(&mut w.rlva, A, &format!("@edit:{id}=n"));
    assert!(w.rlva_can_edit(child));
    command(&mut w.rlva, B, &format!("@editobj:{id}=n"));
    assert!(!w.rlva_can_edit(child));
    for (cap, put, body) in [
        (
            "ModifyMaterialParams",
            false,
            aurora_llsd::Llsd::Array(vec![aurora_llsd::llsd_map! { "object_id" => id }]),
        ),
        (
            "ObjectMedia",
            false,
            aurora_llsd::llsd_map! { "object_id" => id, "verb" => "UPDATE" },
        ),
        (
            "RenderMaterials",
            true,
            crate::scene::legacy_mat::put_body(&[(0, key.local_id, None)]),
        ),
    ] {
        let command = NetCommand::Build(BuildCmd::Cap {
            handle: key.region,
            cap: cap.into(),
            put,
            body,
            tag: 0,
        });
        assert!(!w.rlva_allows_command(&command, Uuid::nil()), "{cap} is an edit too");
    }
    command(&mut w.rlva, A, "@acceptpermission=n,addattach=n,tploc=n");
    assert_eq!(w.rlva_script_permissions(id, 1 << 2), (1 << 2, true));
    assert_eq!(w.rlva_script_permissions(id, 1 << 4), (1 << 4, false));
    assert_eq!(w.rlva_script_permissions(id, 1 << 1), (1 << 1, false));
    assert_eq!(w.rlva_script_permissions(id, (1 << 5) | (1 << 12)), (0, true));
    let response = w.apply(NetEvent::ScriptQuestion {
        task_id: id,
        item_id: TARGET,
        object_name: "test".into(),
        owner_name: "owner".into(),
        questions: 1 << 2,
        experience: Uuid::nil(),
        region: key.region,
    });
    assert!(response.is_none());
    assert!(matches!(w.rlva.out.last(), Some(NetCommand::ScriptAnswer { questions: 4, .. })));
    command(&mut w.rlva, A, "@tplure_sec=n,tprequest=n");
    assert!(!w.rlva_can_accept_teleport(TARGET, false));
    command(&mut w.rlva, B, &format!("@tplure:{TARGET}=n"));
    assert!(!w.rlva_can_accept_teleport(TARGET, false));
    command(
        &mut w.rlva,
        A,
        &format!("@tplure:{TARGET}=n,tprequest:{TARGET}=n,accepttp:{TARGET}=n"),
    );
    assert!(w.rlva_can_accept_teleport(TARGET, false));
    assert!(w.rlva_can_accept_teleport(TARGET, true));
    assert!(w.rlva.auto_accept("accepttp", TARGET));
    assert!(!w.rlva.auto_accept("accepttp", B));
    w.rlva.configure(false);
    assert!(w.rlva_can_edit(child));
    assert_eq!(w.rlva_script_permissions(id, 1 << 2), (1 << 2, false));
}

#[test]
fn rlva_folder_locks_respect_scope_nearest_exception_other_issuers_and_folded_folders() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    w.inventory.root = Uuid::from_u128(10);
    let ids: Vec<_> = (11..18).map(Uuid::from_u128).collect();
    for (id, parent, name, kind) in [
        (ids[0], w.inventory.root, "#RLV", -1),
        (ids[1], ids[0], "Outfit", -1),
        (ids[2], ids[1], "Open", -1),
        (ids[3], ids[1], ".(chest)", -1),
        (ids[4], w.inventory.root, "Current Outfit", 46),
        (ids[5], ids[1], "Closed", -1),
        (ids[6], ids[1], "Other closed", -1),
    ] {
        w.inventory.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: kind,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    for id in [ids[1], ids[2], ids[3], ids[5], ids[6]] {
        let parent = w.inventory.folders[&id].info.parent;
        w.inventory.folders.get_mut(&parent).unwrap().children.push(id);
    }
    for (id, parent) in [
        (TARGET, ids[1]),
        (Uuid::from_u128(20), ids[2]),
        (Uuid::from_u128(21), ids[3]),
        (Uuid::from_u128(22), ids[5]),
    ] {
        w.inventory.items.insert(
            id,
            InvItem {
                id,
                parent,
                asset_type: 5,
                flags: 4,
                ..Default::default()
            },
        );
        w.inventory.folders.get_mut(&parent).unwrap().items.push(id);
    }
    command(
        &mut w.rlva,
        A,
        "@attachthis:Outfit=n,detachallthis:Outfit=n,detachallthis_except:Outfit/Open=n",
    );
    assert!(!w.rlva_can_add(TARGET, 0, Uuid::nil()));
    assert!(
        w.rlva_can_add(Uuid::from_u128(20), 0, Uuid::nil()),
        "node locks do not cover descendants"
    );
    assert!(
        !w.rlva_can_add(Uuid::from_u128(21), 0, Uuid::nil()),
        "folded items belong to the parent"
    );
    assert!(!w.rlva_can_remove(TARGET, Uuid::nil()));
    assert!(w.rlva_can_remove(Uuid::from_u128(20), Uuid::nil()));
    assert!(!w.rlva_can_remove(Uuid::from_u128(22), Uuid::nil()));
    assert!(!w.rlva_inventory_move_allowed(TARGET, ids[2], Uuid::nil()));
    assert!(
        w.rlva_inventory_move_allowed(Uuid::from_u128(22), ids[6], Uuid::nil()),
        "items may move between folders subject to the same subtree lock"
    );
    assert!(
        !w.rlva_inventory_move_allowed(ids[1], w.inventory.root, Uuid::nil()),
        "moving a parent cannot invalidate its explicit descendant locks"
    );
    command(&mut w.rlva, B, "@detachallthis:Outfit=n");
    assert!(
        !w.rlva_can_remove(Uuid::from_u128(20), Uuid::nil()),
        "another issuer's lock survives the exception"
    );
    assert!(
        !w.rlva_can_remove(Uuid::from_u128(20), B),
        "an issuer cannot bypass its own folder lock"
    );
    command(&mut w.rlva, A, "@clear");
    command(&mut w.rlva, B, "@clear");
    let object = Uuid::from_u128(23);
    w.inventory.items.insert(
        object,
        InvItem {
            id: object,
            parent: ids[3],
            asset_type: 6,
            ..Default::default()
        },
    );
    w.inventory.folders.get_mut(&ids[3]).unwrap().items.push(object);
    let c = Command::parse("attachalloverorreplace:Outfit=force").unwrap();
    assert!(w.rlva_inventory_force(A, &c));
    assert_eq!(w.rlva.appearance.len(), 5);
    assert!(matches!(
        w.rlva.appearance[0],
        (_, crate::world::appearance::Action::WearItem { replace: true, .. })
    ));
    assert!(
        w.rlva
            .appearance
            .iter()
            .skip(1)
            .take(3)
            .all(|(_, action)| matches!(action, crate::world::appearance::Action::WearItem { replace: false, .. }))
    );
    assert!(
        matches!(w.rlva.appearance[4], (_, crate::world::appearance::Action::WearItem { item, point: 1, replace: true }) if item == object),
        "a folded attachment folder chooses the attachment point"
    );
    w.rlva.appearance.clear();
    command(&mut w.rlva, A, "@attach:Outfit=force,getgroup=123");
    w.rlva_pending();
    assert_eq!(w.rlva.pending.len(), 1, "the query waits for the outfit transaction");
    assert!(w.rlva.out.is_empty());
    w.rlva.appearance.clear();
    w.rlva.appearance_busy = true;
    w.rlva_pending();
    assert_eq!(w.rlva.pending.len(), 1);
    w.rlva.appearance_busy = false;
    w.rlva_pending();
    assert!(matches!(w.rlva.out.last(), Some(NetCommand::ChatReply { channel: 123, message, .. }) if message == "none"));
}

#[test]
fn rlva_camera_aliases_share_limits_without_losing_independent_removal() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        "@camdistmax:2=n,setcam_avdistmax:5=n,camzoommin:2=n,camunlock=n,TOUCHFAR:3=n",
    );
    assert_eq!(r.camera_limit("setcam_avdistmax", f32::MAX, false), 2.0);
    assert!((r.camera_limit("setcam_fovmin", 0.1, true) - std::f32::consts::PI / 6.0).abs() < 1e-5);
    assert!(r.has("setcam_unlock"));
    assert_eq!(r.distance_limit("fartouch", 1.5, Uuid::nil()), 3.0);
    command(&mut r, A, "@camdistmax:2=y");
    assert_eq!(r.camera_limit("setcam_avdistmax", f32::MAX, false), 5.0);
    assert_eq!(camera::height_offset("3", 0.95), Some(0.03));
    assert!((camera::height_offset("1;2;0.01", 0.95).unwrap() - 0.11).abs() < 1e-5);
    assert_eq!(camera::height_offset("9999", 0.95), Some(crate::world::hover_height::MAX));
    assert!(camera::height_offset("NaN", 0.95).is_none());
    assert!(camera::height_offset("1;2;3;4", 0.95).is_none());
}

#[test]
fn rlva_im_ranges_and_nametag_distances_follow_exclusions_and_strict_exceptions() {
    let mut r = enabled();
    command(&mut r, A, "@sendim:5;20=n,shownametags:10=n");
    assert!(r.has("sendim"));
    assert!(!r.allows_im_at("sendim", TARGET, Some(2.0)));
    assert!(r.allows_im_at("sendim", TARGET, Some(5.0)));
    assert!(r.allows_im_at("sendim", TARGET, Some(20.0)));
    assert!(!r.allows_im_at("sendim", TARGET, None));
    command(&mut r, B, "@sendim:7;15=n");
    assert!(!r.allows_im_at("sendim", TARGET, Some(6.0)));
    command(&mut r, A, &format!("@sendimto:{TARGET}=n"));
    assert!(!r.allows_im_at("sendim", TARGET, Some(10.0)));
    assert!(r.nametag_visible(TARGET, TARGET, 999.0));
    assert!(r.nametag_visible(TARGET, B, 9.0));
    assert!(!r.nametag_visible(TARGET, B, 10.0));
    command(&mut r, B, "@shownametags_sec=n");
    command(&mut r, A, &format!("@shownametags:{TARGET}=n"));
    assert!(!r.nametag_visible(TARGET, B, 20.0));
    command(&mut r, B, &format!("@shownametags:{TARGET}=n"));
    assert!(r.nametag_visible(TARGET, B, 20.0));
    command(&mut r, A, "@clear");
    command(&mut r, B, "@clear");
    command(&mut r, A, "@sendim:20=n");
    assert!(
        r.allows_im_at("sendim", TARGET, None),
        "an unbounded exclusion also covers off-region recipients"
    );
}

#[test]
fn rlva_stand_return_waits_for_a_successful_sit_and_is_cancelled_by_disabling() {
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    command(&mut w.rlva, A, "@standtp=n,tploc=n");
    let origin = (42, glam::Vec3::new(100.0, 100.0, 30.0));
    w.rlva.stand_origin = Some(origin);
    w.rlva_update_stand();
    assert!(w.rlva.stand_return.is_none(), "an unanswered sit is not a stand");
    w.agent.ground_sit = true;
    w.rlva_update_stand();
    w.agent.ground_sit = false;
    w.rlva_update_stand();
    assert_eq!(w.rlva.stand_return, Some(origin));
    let c = NetCommand::TeleportTo {
        handle: 42,
        position: origin.1,
        look_at: glam::Vec3::X,
    };
    assert!(w.rlva_allows_command(&c, Uuid::nil()));
    w.rlva.stand_return = None;
    assert!(!w.rlva_allows_command(&c, Uuid::nil()));
    w.rlva.stand_origin = Some(origin);
    w.rlva.configure(false);
    assert!(w.rlva.stand_origin.is_none() && w.rlva.stand_return.is_none());
}

#[test]
fn rlva_local_teleports_use_global_xy_and_sit_radius_uses_xyz_across_region_edges() {
    use glam::Vec3;
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    world.agent.position = Vec3::new(250.0, 128.0, 30.0);
    let handle = world.main_region.unwrap();
    let (x, y) = aurora_net::handle_to_origin(handle);
    let neighbor = aurora_net::origin_to_handle(x + 256, y);
    command(&mut world.rlva, A, "@tploc=n,tplocal:20=n,sittp:10=n");
    let teleport = |handle, position| NetCommand::TeleportTo {
        handle,
        position,
        look_at: Vec3::X,
    };
    assert!(world.rlva_allows_command(&teleport(neighbor, Vec3::new(0.0, 128.0, 30.0)), Uuid::nil()));
    assert!(!world.rlva_allows_command(&teleport(handle, Vec3::new(250.0, 128.0, 41.0)), Uuid::nil()));
    assert!(
        !world.rlva_allows_command(&teleport(neighbor, Vec3::new(4.0, 128.0, 30.0)), Uuid::nil()),
        "the radius is exclusive"
    );
    command(&mut world.rlva, A, "@sittp=y");
    command(&mut world.rlva, A, "@sittp:10=y");
    assert!(
        world.rlva_allows_command(&teleport(handle, Vec3::new(250.0, 128.0, 1000.0)), Uuid::nil()),
        "tplocal measures XY, not height"
    );
    assert!(!world.rlva_allows_command(&teleport(neighbor, Vec3::new(250.0, 128.0, 30.0)), Uuid::nil()));
    assert!(
        world.rlva_allows_command(&teleport(neighbor, Vec3::new(250.0, 128.0, 30.0)), A),
        "a forced action bypasses its own issuer"
    );
}

#[test]
fn rlva_render_aliases_release_without_mutating_saved_settings_and_touch_measures_the_surface() {
    let mut r = enabled();
    command(&mut r, A, "@camtextures=n,camavdist:4=n,showselfhead=n");
    assert_eq!(r.camera_texture(), Some(aurora_prim::te::DEFAULT_TEXTURE));
    assert_eq!(r.silhouette_distance(), Some(4.0));
    command(&mut r, A, "@camtextures=y,camavdist:4=y");
    assert_eq!(r.camera_texture(), None);
    assert_eq!(r.silhouette_distance(), None);
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    command(&mut world.rlva, A, "@fartouch:2=n");
    let key = world
        .objects
        .iter()
        .find(|(_, o)| !o.is_avatar() && o.parent_id == 0)
        .map(|(_, o)| o.key)
        .unwrap();
    assert!(world.rlva_can_touch_at(key.region, key.local_id, Some(world.agent.position + glam::Vec3::X)));
    assert!(world.rlva_can_touch_at(key.region, key.local_id, Some(world.agent.position + glam::Vec3::X * 2.0)));
    assert!(!world.rlva_can_touch_at(key.region, key.local_id, Some(world.agent.position + glam::Vec3::X * 2.01)));
    world.rlva.configure(false);
    assert!(world.rlva_can_touch_at(key.region, key.local_id, Some(world.agent.position + glam::Vec3::X * 100.0)));
}

/// The watchdog checks every worn attachment each frame. Its folder locks,
/// resolved once for all of them (one walk of the inventory, links
/// included), lock exactly what the one-off checks lock.
#[test]
fn rlva_watchdog_folder_locks_resolved_once_match_single_checks() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    world.movement_complete = true;
    let add_folder = |world: &mut crate::world::World, id: Uuid, parent: Uuid, name: &str| {
        world.inventory.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(parent) = world.inventory.folders.get_mut(&parent) {
            parent.children.push(id);
        }
    };
    let root = Uuid::from_u128(0xF000);
    world.inventory.root = root;
    add_folder(&mut world, root, Uuid::nil(), "My Inventory");
    let rlv = Uuid::from_u128(0xF001);
    let collar = Uuid::from_u128(0xF002);
    let cuffs = Uuid::from_u128(0xF003);
    let other = Uuid::from_u128(0xF004);
    add_folder(&mut world, rlv, root, "#RLV");
    add_folder(&mut world, collar, rlv, "Collar");
    add_folder(&mut world, cuffs, rlv, "Cuffs");
    add_folder(&mut world, other, root, "Clothes");
    let item = |world: &mut crate::world::World, n: u128, parent: Uuid, link_to: Option<Uuid>| {
        let id = Uuid::from_u128(n);
        world.inventory.items.insert(
            id,
            InvItem {
                id,
                parent,
                asset_type: if link_to.is_some() { 24 } else { 6 },
                asset_id: link_to.unwrap_or_default(),
                flags: 5,
                ..Default::default()
            },
        );
        world.inventory.folders.get_mut(&parent).unwrap().items.push(id);
        id
    };
    // the collar lives outside #RLV; its link in #RLV/Collar carries the lock
    let collar_item = item(&mut world, 0xC000, other, None);
    item(&mut world, 0xC001, collar, Some(collar_item));
    let cuff = item(&mut world, 0xC002, cuffs, None);
    let shirt = item(&mut world, 0xC003, other, None);
    for n in 0..2000 {
        item(&mut world, 0xD000 + n, other, None);
    }
    let handle = world.main_region.unwrap();
    for (n, worn) in [collar_item, cuff, shirt].into_iter().enumerate() {
        world.objects.upsert(
            handle,
            crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: worn,
                owner_id: world.agent_id,
                point: n as u8 + 1,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            }),
        );
    }
    command(&mut world.rlva, collar_item, "@detachallthis=n,attachallthis=n");
    command(&mut world.rlva, cuff, "@detachthis:Cuffs=n");
    let locks = super::inventory::FolderLocks::default();
    for folder in [root, rlv, collar, cuffs, other] {
        for add in [false, true] {
            assert_eq!(
                world.rlva_folder_allowed_in(&locks, folder, add, Uuid::nil()),
                world.rlva_folder_allowed(folder, add, Uuid::nil()),
                "folder {folder} add {add}"
            );
        }
    }
    assert!(
        !world.rlva_folder_allowed_in(&locks, collar, false, Uuid::nil()),
        "the link's folder is locked"
    );
    assert!(
        !world.rlva_folder_allowed_in(&locks, cuffs, false, Uuid::nil()),
        "a named folder is locked"
    );
    assert!(
        world.rlva_folder_allowed_in(&locks, other, false, Uuid::nil()),
        "outside #RLV stays free"
    );
    world.rlva_watchdog_tick(std::time::Instant::now());
    world.rlva_watchdog_tick(std::time::Instant::now());
    assert!(world.rlva.watchdog.rejected.is_empty(), "nothing worn is rejected");
}

/// The watchdog keeps its lock checks, and the folder locks their inventory
/// walk, while nothing they read changes: a rule, an inventory move or a
/// worn change makes them run again, with the same answers as before.
#[test]
fn rlva_watchdog_checks_run_again_only_after_a_change() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    world.movement_complete = true;
    let add_folder = |world: &mut crate::world::World, id: Uuid, parent: Uuid, name: &str| {
        world.inventory.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(parent) = world.inventory.folders.get_mut(&parent) {
            parent.children.push(id);
        }
    };
    let root = Uuid::from_u128(0xF000);
    world.inventory.root = root;
    add_folder(&mut world, root, Uuid::nil(), "My Inventory");
    let rlv = Uuid::from_u128(0xF001);
    let collar = Uuid::from_u128(0xF002);
    let clothes = Uuid::from_u128(0xF003);
    add_folder(&mut world, rlv, root, "#RLV");
    add_folder(&mut world, collar, rlv, "Collar");
    add_folder(&mut world, clothes, root, "Clothes");
    let item = |n: u128, parent: Uuid| InvItem {
        id: Uuid::from_u128(n),
        parent,
        asset_type: 6,
        flags: 5,
        ..Default::default()
    };
    let collar_item = Uuid::from_u128(0xC000);
    let shirt = Uuid::from_u128(0xC001);
    world.inventory.add_items(vec![item(0xC000, collar), item(0xC001, clothes)]);
    for n in 0..2000 {
        world.inventory.add_items(vec![item(0xD000 + n, clothes)]);
    }
    let handle = world.main_region.unwrap();
    for (n, worn) in [collar_item, shirt].into_iter().enumerate() {
        world.objects.upsert(
            handle,
            crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: worn,
                owner_id: world.agent_id,
                point: n as u8 + 1,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            }),
        );
    }
    let tick = |world: &mut crate::world::World| {
        world.rlva_watchdog_tick(std::time::Instant::now());
        world.rlva_watchdog_tick(std::time::Instant::now());
        world.rlva.watchdog.checked.as_ref().expect("checked").locked.clone()
    };
    let resolution = |world: &crate::world::World| {
        let cache = world.rlva.locked_folders.0.lock().expect("cache");
        cache.as_ref().map(|(_, folders)| std::sync::Arc::as_ptr(folders))
    };
    assert!(tick(&mut world).is_empty(), "no rule, nothing locked");
    command(&mut world.rlva, collar_item, "@detachallthis=n");
    assert_eq!(tick(&mut world), BTreeSet::from([collar_item]), "a new rule is checked at once");
    let first = resolution(&world);
    assert!(first.is_some());
    assert_eq!(tick(&mut world), BTreeSet::from([collar_item]));
    assert_eq!(resolution(&world), first, "nothing changed: the inventory is not walked again");
    world.inventory.add_items(vec![item(0xC001, collar)]);
    assert_eq!(
        tick(&mut world),
        BTreeSet::from([collar_item, shirt]),
        "moved into the locked folder"
    );
    assert_ne!(resolution(&world), first, "an inventory change resolves the locks again");
    world.objects.upsert(
        handle,
        crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: Uuid::from_u128(0xD000),
            owner_id: world.agent_id,
            point: 3,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        }),
    );
    world.rlva_watchdog_tick(std::time::Instant::now());
    assert!(
        world
            .rlva
            .watchdog
            .checked
            .as_ref()
            .expect("checked")
            .allowed
            .contains_key(&Uuid::from_u128(0xD000)),
        "the new attachment is in the checks"
    );
    command(&mut world.rlva, collar_item, "@detachallthis=y");
    assert!(tick(&mut world).is_empty(), "a removed rule is checked at once");
}

/// RlvAttachmentLocks::canAttach / RlvWearableLocks::canWear and the folder
/// locks of LLAppearanceMgr::getCanAddToCOF (rlvlocks.h, llappearancemgr.cpp):
/// what the wear menus grey out.
#[test]
fn rlva_wear_locks_follow_the_legacy_wear_masks() {
    use super::wear_locks::Wear;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let root = Uuid::from_u128(30);
    let cof = Uuid::from_u128(31);
    let clothes = Uuid::from_u128(32);
    w.inventory.root = root;
    for (id, parent, kind, name) in [
        (root, Uuid::nil(), 8, "Inventory"),
        (cof, root, 46, "Current Outfit"),
        (clothes, root, -1, "Clothes"),
    ] {
        w.inventory.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: kind,
                    ..Default::default()
                },
                items: vec![],
                children: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    // (id, asset type, flags, name): worn shirt and shape, then a new shirt,
    // jacket, shape, an object naming the chest, one naming no point and
    // never worn, and one naming no point last worn on the chest.
    let items = [
        (40, 5, 4, "Old shirt"),
        (41, 13, 0, "Old shape"),
        (42, 5, 4, "New shirt"),
        (43, 5, 8, "Jacket"),
        (44, 13, 0, "New shape"),
        (45, 6, 0, "Collar (chest)"),
        (46, 6, 0, "Bracelet"),
        (47, 6, 1, "Necklace"),
    ];
    for (n, (id, asset_type, flags, name)) in items.into_iter().enumerate() {
        let id = Uuid::from_u128(id);
        w.inventory.items.insert(
            id,
            InvItem {
                id,
                parent: clothes,
                name: name.into(),
                asset_type,
                flags,
                owner_mask: 0x4000,
                ..Default::default()
            },
        );
        w.inventory.folders.get_mut(&clothes).expect("clothes").items.push(id);
        if n < 2 {
            let link = Uuid::from_u128(id.as_u128() + 100);
            w.inventory.items.insert(
                link,
                InvItem {
                    id: link,
                    parent: cof,
                    asset_id: id,
                    asset_type: 24,
                    ..Default::default()
                },
            );
            w.inventory.folders.get_mut(&cof).expect("cof").items.push(link);
        }
    }
    let id = |n: u128| Uuid::from_u128(n);
    let all = Wear { add: true, replace: true };
    let locked = Wear {
        add: false,
        replace: false,
    };

    let locks = w.rlva_wear_locks();
    assert_eq!(locks.wear(&w.inventory, id(42), 0), all, "RLVa off allows everything");
    w.rlva.configure(true);
    let locks = w.rlva_wear_locks();
    assert!(
        locks.can_remove(id(40)) && locks.folder_allows(clothes, true),
        "no lock, no greying"
    );

    command(
        &mut w.rlva,
        A,
        "@remoutfit:shirt=n,remoutfit:shape=n,addoutfit:jacket=n,addattach:chest=n",
    );
    let locks = w.rlva_wear_locks();
    assert!(!locks.can_remove(id(40)) && !locks.can_remove(id(41)));
    assert_eq!(
        locks.wear(&w.inventory, id(42), 0),
        Wear { add: true, replace: false },
        "a shirt stacks beside the locked one"
    );
    assert_eq!(locks.wear(&w.inventory, id(43), 0), locked, "@addoutfit:jacket");
    assert_eq!(
        locks.wear(&w.inventory, id(44), 0),
        locked,
        "a body part cannot be added beside the locked one"
    );
    assert_eq!(locks.wear(&w.inventory, id(45), 0), locked, "the item names the locked chest");
    assert_eq!(locks.wear(&w.inventory, id(46), 0), all, "no point named: it attaches where it can");
    assert_eq!(locks.wear(&w.inventory, id(46), 1), locked, "Attach To the chest");
    assert_eq!(locks.wear(&w.inventory, id(46), 2), all);
    assert_eq!(
        locks.wear(&w.inventory, id(47), 0),
        locked,
        "Wear goes to the chest it was last worn on"
    );
    assert_eq!(locks.wear(&w.inventory, id(47), 2), all, "Attach To another point still works");
    w.rlva.last_points.note(id(46), 1, std::time::Instant::now());
    assert_eq!(
        w.rlva_wear_locks().wear(&w.inventory, id(46), 0),
        locked,
        "without a saved point, the point it was last seen on"
    );
    w.rlva.last_points = Default::default();
    assert!(locks.folder_allows(clothes, true) && locks.folder_allows(clothes, false));

    command(&mut w.rlva, A, "@clear");
    command(&mut w.rlva, A, "@unsharedwear=n");
    let locks = w.rlva_wear_locks();
    assert!(!locks.folder_allows(clothes, true), "outside #RLV nothing can be worn");
    assert!(locks.folder_allows(clothes, false));
    assert_eq!(locks.wear(&w.inventory, id(42), 0), locked);
    assert!(locks.can_remove(id(40)));
    w.rlva.configure(false);
    assert_eq!(w.rlva_wear_locks().wear(&w.inventory, id(42), 0), all);
}

/// Wear onto a point a locked attachment holds is greyed (deliberate
/// difference, human decision of 2026-10-31), Add stays: RLV lets an object
/// attach beside a locked one.
#[test]
fn rlva_wear_on_a_point_held_by_a_locked_attachment_keeps_add() {
    use super::wear_locks::Wear;
    use aurora_net::{AttachRequest, inventory::InvItem};
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let region = world.main_region.expect("demo region");
    let (held, named, unnamed) = (Uuid::from_u128(60), Uuid::from_u128(61), Uuid::from_u128(62));
    for (id, flags) in [(held, 1), (named, 1), (unnamed, 0)] {
        world.inventory.items.insert(
            id,
            InvItem {
                id,
                asset_type: 6,
                flags,
                owner_mask: 0x4000,
                ..Default::default()
            },
        );
    }
    world.objects.upsert(
        region,
        crate::demo::outfit_attachment(&AttachRequest {
            item_id: held,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 1,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        }),
    );
    let all = Wear { add: true, replace: true };
    let add_only = Wear { add: true, replace: false };
    assert_eq!(world.rlva_wear_locks().wear(&world.inventory, named, 0), all, "no lock yet");
    command(&mut world.rlva, A, "@remattach:chest=n");
    let locks = world.rlva_wear_locks();
    assert!(!locks.can_remove(held));
    assert_eq!(locks.wear(&world.inventory, named, 0), add_only, "the item names the held chest");
    assert_eq!(locks.wear(&world.inventory, named, 2), all, "Attach To another point");
    assert_eq!(
        locks.wear(&world.inventory, unnamed, 0),
        all,
        "no point known: it attaches where it can"
    );
    world.rlva.last_points.note(unnamed, 1, std::time::Instant::now());
    assert_eq!(
        world.rlva_wear_locks().wear(&world.inventory, unnamed, 0),
        add_only,
        "last worn on the held chest"
    );
}

#[test]
fn rlva_drop_and_detach_of_a_worn_object_follow_its_remove_lock_and_rez() {
    use aurora_net::{AttachRequest, build::BuildCmd, inventory::InvItem};
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let region = world.main_region.expect("demo region");
    let (chest, skull) = (Uuid::from_u128(70), Uuid::from_u128(71));
    for (id, point) in [(chest, 1), (skull, 2)] {
        world.inventory.items.insert(
            id,
            InvItem {
                id,
                asset_type: 6,
                flags: point,
                owner_mask: 0x4000,
                ..Default::default()
            },
        );
        world.objects.upsert(
            region,
            crate::demo::outfit_attachment(&AttachRequest {
                item_id: id,
                owner_id: world.agent_id,
                point: point as u8,
                add: true,
                flags: 1,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            }),
        );
    }
    let key = |item: Uuid| {
        world
            .objects
            .iter()
            .find(|(_, o)| o.attachment_item_id() == Some(item))
            .map(|(_, o)| o.key)
            .expect("worn")
    };
    let (chest_key, skull_key) = (key(chest), key(skull));
    let drop = |world: &crate::world::World, k: crate::world::objects::ObjKey| {
        let cmd = NetCommand::Build(BuildCmd::DropAttachment {
            handle: k.region,
            local_ids: vec![k.local_id],
        });
        (
            world.rlva_can_drop(k.region, &[k.local_id]),
            world.rlva_allows_command(&cmd, Uuid::nil()),
        )
    };
    assert_eq!(drop(&world, chest_key), (true, true), "no lock yet");
    command(&mut world.rlva, A, "@remattach:chest=n");
    // LLAttachmentDrop / enable_detach: the locked one stays, the other goes
    assert_eq!(drop(&world, chest_key), (false, false));
    assert!(!world.rlva_can_remove(chest, Uuid::nil()));
    assert_eq!(drop(&world, skull_key), (true, true));
    assert!(world.rlva_can_remove(skull, Uuid::nil()));
    // @rez: nothing can be put in-world, a detach still can
    command(&mut world.rlva, A, "@rez=n");
    assert_eq!(drop(&world, skull_key), (false, false));
    assert!(world.rlva_can_remove(skull, Uuid::nil()));
}
