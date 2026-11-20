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
    assert!(!world.rlva_can_pay());
    assert!(world.rlva_allows_command(&object, Uuid::nil()));
    assert!(!world.rlva_allows_command(&resident, Uuid::nil()));
    command(&mut world.rlva, A, "@pay=y,buy=n");
    assert!(world.rlva_can_pay());
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
        // rlvAttachGroupFromIndex (rlvhelper.cpp): "hud" is avatar_lad
        // group 8, the Bento additions; the real HUD points (group 9) are in
        // no group, so the worn Center 2 is not listed
        ("hud", ""),
    ] {
        let text = format!("getattachnames:{group}=123");
        let c = Command::parse(&text).unwrap();
        assert_eq!(world.rlva_inventory_query(&c, A).as_deref(), Some(expected));
    }
    let hud_points = world.rlva_inventory_query(&query("getaddattachnames:hud=123"), A).unwrap();
    assert!(!hud_points.split(',').any(|p| p == "Center 2"));
    assert!(
        hud_points.split(',').any(|p| p == "Left Ring Finger"),
        "the Bento group 8 is the \"hud\" group"
    );
    let head = Uuid::from_u128(702);
    command(&mut world.rlva, head, "@detach=n");
    assert_eq!(
        world.rlva_inventory_query(&query("getremattachnames:head=123"), head).as_deref(),
        Some(""),
        "queries do not waive their issuer's lock"
    );
    // RlvCommandOptionHelper::parseOption (rlvhelper.cpp) tries a shared
    // folder before a group: a group waits for the inventory to tell
    command(&mut world.rlva, head, "@remattach:head=force");
    world.rlva_pending();
    assert_eq!(world.rlva.pending.len(), 1, "waits for the inventory");
    let root = world.inventory.root;
    world.inventory.folders.entry(root).or_insert_with(|| Folder {
        info: InvFolder {
            id: root,
            ..Default::default()
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    });
    world.inventory.folders.get_mut(&root).unwrap().state = FetchState::Fetched;
    world.rlva_pending();
    assert!(world.rlva.pending.is_empty());
    assert!(world.rlva.appearance.is_empty(), "nothing removable, no outfit change");
    command(&mut world.rlva, head, "@detach=y,remattach:head=force");
    world.rlva_pending();
    assert!(
        matches!(world.rlva.appearance.pop(), Some((_, crate::world::appearance::Action::Force { remove, wear })) if remove == vec![head] && wear.is_empty())
    );
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
    let saved_position = pick.pos_global;
    pick.pos_global.x += 1.0;
    let mut move_pick = NetCommand::PickUpdate(Box::new(pick));
    world.profile_command(&move_pick); // the profile's optimistic cache runs before send_from
    assert_eq!(world.rlva_send_boundary(&mut move_pick, Uuid::nil()), super::Boundary::Refused);
    assert_eq!(world.profiles.picks[&TARGET].pos_global, saved_position);
    let mut read_pick = NetCommand::PickInfoRequest {
        creator: world.agent_id,
        pick: TARGET,
    };
    world.profile_command(&read_pick);
    assert_eq!(world.rlva_send_boundary(&mut read_pick, Uuid::nil()), super::Boundary::Send);
    world.rlva_note_sent(&mut read_pick, Uuid::nil(), std::time::Instant::now());
    assert_eq!(world.profiles.picks[&TARGET].pos_global, saved_position);
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
        region: None,
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
    // home is locked only by @tplm and @tploc together (teleportViaLandmark)
    command(&mut world.rlva, A, "@tplm=n,tploc=n");
    assert_eq!(maturity.answer(Some(SIM_ACCESS_MATURE)), Outcome::Synced);
    let (actor, teleport) = maturity.take_commands().pop().unwrap();
    assert_eq!(actor, A);
    assert!(
        world.rlva_allows_command(&teleport, actor),
        "the original issuer can still force through its own lock"
    );
    command(&mut world.rlva, B, "@tplm=n,tploc=n");
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
fn rlva_temporary_attachment_locks_do_not_require_inventory_or_cof() {
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
        !world
            .rlva_pending()
            .iter()
            .any(|(actor, cmd)| *actor == A && matches!(cmd, NetCommand::DetachObject { .. })),
        "LLVOAvatarSelf::detachAttachmentIntoInventory needs an inventory item: a temporary attachment stays on"
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
fn rlva_camera_view_holds_the_offsets_and_the_mouselook_rule() {
    use camera::CameraView;
    let mut r = enabled();
    assert_eq!(r.camera_view(), CameraView::default());
    assert!(r.can_mouselook());
    command(&mut r, A, "@setcam_eyeoffset:-4/0/1=n,setcam_eyeoffsetscale:NaN=n");
    let view = r.camera_view();
    assert_eq!(view.eye_offset, Some(glam::Vec3::new(-4.0, 0.0, 1.0)));
    assert_eq!((view.focus_offset, view.eye_offset_scale), (None, None), "not a number is no scale");
    assert!(view.preset_locked && !view.no_mouselook);
    command(&mut r, A, "@clear");
    assert!(!r.camera_view().preset_locked);
    // RlvActions::canChangeToMouselook
    for (set, lift) in [
        ("@camdistmin:1=n", "@camdistmin:1=y"),
        ("@setcam_avdistmin:0.5=n", "@setcam_avdistmin:0.5=y"),
        ("@setcam_mouselook=n", "@setcam_mouselook=y"),
    ] {
        command(&mut r, A, set);
        assert!(!r.can_mouselook() && r.camera_view().no_mouselook, "{set}");
        command(&mut r, A, lift);
        assert!(r.can_mouselook() && !r.camera_view().no_mouselook, "{lift}");
    }
    command(&mut r, A, "@setcam_avdistmin:0=n");
    assert!(r.can_mouselook(), "a minimum of zero forbids nothing");
    command(&mut r, A, "@clear");
    // under an exclusive @setcam only its owner's restrictions count
    command(&mut r, B, "@setcam_mouselook=n,setcam_avdistmin:3=n");
    assert!(!r.can_mouselook());
    command(&mut r, A, "@setcam=n");
    assert!(r.can_mouselook());
    command(&mut r, A, "@setcam_mouselook=n");
    assert!(!r.can_mouselook());
    r.configure(false);
    assert_eq!(r.camera_view(), CameraView::default());
    assert!(r.can_mouselook());
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
        local_ids: vec![key.local_id],
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
    assert_eq!(w.rlva.appearance.len(), 1, "the whole folder is one outfit change");
    let crate::world::appearance::Action::Force { remove, wear } = &w.rlva.appearance[0].1 else {
        panic!("a forced folder wear is one Force batch");
    };
    assert!(remove.is_empty());
    // LLInventoryModel::collectDescendentsIf (llinventorymodel.cpp): the
    // subfolders first, depth first, then the folder's own items; the first
    // shirt replaces, the next ones add (fSeenWType, RlvForceWear::forceFolder)
    let order: Vec<(Uuid, bool)> = wear.iter().map(|w| (w.item, w.replace)).collect();
    assert_eq!(
        order,
        vec![
            (Uuid::from_u128(20), true),
            (Uuid::from_u128(21), false),
            (object, true),
            (Uuid::from_u128(22), false),
            (TARGET, false),
        ]
    );
    assert_eq!(wear[2].point, 1, "a folded attachment folder chooses the attachment point");
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

/// RlvHandler::processCommand runs a force when it is received: another
/// object's lock released before the queue runs does not let it through.
/// One set after it still stops it (stricter than the Legacy Viewer, where
/// the avatar takes off and the lock then lands it).
#[test]
fn rlva_queued_force_reads_the_rules_it_was_received_under() {
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    command(&mut w.rlva, B, "@fly=n");
    command(&mut w.rlva, A, "@fly=force");
    command(&mut w.rlva, B, "@fly=y");
    let revision = w.rlva.revision;
    w.rlva_pending();
    assert!(!w.agent.flying, "refused under the lock held when it was received");
    assert!(!w.rlva.has("fly"), "the current rules are back once the queue has run");
    assert_eq!(w.rlva.revision, revision, "running the queue changes no rule");
    command(&mut w.rlva, A, "@fly=force");
    command(&mut w.rlva, B, "@fly=n");
    w.rlva_pending();
    assert!(!w.agent.flying, "refused under the lock set since");
    command(&mut w.rlva, B, "@fly=y");
    command(&mut w.rlva, A, "@fly=force");
    w.rlva_pending();
    assert!(w.agent.flying);
}

#[test]
fn run_restrictions_each_leave_the_other_kind_of_run() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    let run = NetCommand::SetAlwaysRun(true);
    let walk = NetCommand::SetAlwaysRun(false);
    command(&mut world.rlva, A, "@alwaysrun=n");
    assert!(world.rlva_allows_command(&run, Uuid::nil()), "@alwaysrun leaves the double tap");
    command(&mut world.rlva, A, "@alwaysrun=y,temprun=n");
    assert!(world.rlva_allows_command(&run, Uuid::nil()), "@temprun leaves Always run");
    command(&mut world.rlva, A, "@alwaysrun=n");
    assert!(!world.rlva_allows_command(&run, Uuid::nil()), "both leave nothing that may run");
    assert!(world.rlva_allows_command(&walk, Uuid::nil()));
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
        !world.rlva_allows_command(&teleport(handle, Vec3::new(250.0, 128.0, 1000.0)), Uuid::nil()),
        "tplocal measures in 3D: going straight up does not get around it"
    );
    assert!(world.rlva_allows_command(&teleport(handle, Vec3::new(250.0, 128.0, 45.0)), Uuid::nil()));
    assert!(!world.rlva_allows_command(&teleport(neighbor, Vec3::new(250.0, 128.0, 30.0)), Uuid::nil()));
    assert!(
        world.rlva_allows_command(&teleport(neighbor, Vec3::new(250.0, 128.0, 30.0)), A),
        "a forced action bypasses its own issuer"
    );
}

#[test]
fn rlva_teleport_home_landmarks_and_bare_tplocal_follow_the_legacy_rules() {
    use glam::Vec3;
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    world.agent.position = Vec3::new(128.0, 128.0, 30.0);
    let here = world.agent_global().unwrap();
    let at = |dx: f64, dz: f64| here + glam::DVec3::new(dx, 0.0, dz);
    // LLAgent::teleportViaLandmark: home only under both @tplm and @tploc
    command(&mut world.rlva, A, "@tplm=n");
    assert!(world.rlva_can_teleport_home() && !world.rlva_can_teleport_landmark());
    assert!(world.rlva_allows_command(&NetCommand::TeleportHome, Uuid::nil()));
    command(&mut world.rlva, A, "@tploc=n");
    assert!(!world.rlva_can_teleport_home());
    assert!(!world.rlva_allows_command(&NetCommand::TeleportHome, Uuid::nil()));
    command(&mut world.rlva, A, "@tplm=y");
    assert!(world.rlva_can_teleport_home() && world.rlva_can_teleport_landmark());
    assert!(!world.rlva_can_teleport_global(at(1000.0, 0.0)), "@tploc closes distant places");
    assert!(world.rlva_can_teleport_global(at(100.0, 0.0)), "a local teleport ignores @tploc");
    assert!(
        world.rlva_can_teleport_local(at(1000.0, 0.0)),
        "the area search still shows its entry: @tploc refuses once asked"
    );
    // RLV_MODIFIER_TPLOCAL_DEFAULT: a bare @tplocal allows 1.5 m, in 3D
    command(&mut world.rlva, A, "@tploc=y,tplocal=n");
    assert!(world.rlva_can_teleport_global(at(1.0, 0.0)));
    assert!(!world.rlva_can_teleport_global(at(2.0, 0.0)));
    assert!(!world.rlva_can_teleport_global(at(0.0, 2.0)));
    assert!(world.rlva_can_teleport_global(at(1000.0, 0.0)), "beyond a region it is not local");
    assert!(world.rlva_can_teleport_local(at(1.0, 0.0)));
    assert!(
        !world.rlva_can_teleport_local(at(1000.0, 0.0)),
        "asked as a local one, as the area search does"
    );
    command(&mut world.rlva, A, "@tplocal=y,sittp=n");
    assert!(world.rlva_can_teleport_local(at(1.0, 0.0)) && !world.rlva_can_teleport_local(at(1000.0, 0.0)));
    command(&mut world.rlva, A, "@sittp=y,tplocal=n");
    // seated under @unsit, nothing goes, home and landmarks included
    command(&mut world.rlva, A, "@tplocal=y,unsit=n");
    world.agent.ground_sit = true;
    assert!(!world.rlva_can_teleport_home() && !world.rlva_can_teleport_landmark());
    assert!(!world.rlva_can_teleport_global(at(1.0, 0.0)) && !world.rlva_can_teleport_local(at(1.0, 0.0)));
    world.agent.ground_sit = false;
    world.rlva.configure(false);
    assert!(world.rlva_can_teleport_home() && world.rlva_can_teleport_landmark() && world.rlva_can_teleport_global(at(0.0, 2.0)));
}

#[test]
fn rlva_render_aliases_release_without_mutating_saved_settings_and_touch_measures_the_surface() {
    let mut r = enabled();
    command(&mut r, A, "@camtextures=n,camavdist:4=n,showselfhead=n");
    // Empty @camtextures / @setcam_textures is IMG_DEFAULT, the flat grey
    // (rlvhelper.cpp), not the plywood texture.
    assert_eq!(r.camera_texture(), Some(aurora_prim::te::IMG_DEFAULT));
    let expected = aurora_prim::te::IMG_DEFAULT.to_string();
    assert_eq!(
        r.camera_reply(&Command::parse("getcam_textures=123").unwrap(), 1.0, 1.0).as_deref(),
        Some(expected.as_str())
    );
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
    // the shirt lives outside #RLV too, worn through its link in #RLV/Cuffs
    item(&mut world, 0xC004, cuffs, Some(shirt));
    let free = item(&mut world, 0xC005, other, None);
    for n in 0..2000 {
        item(&mut world, 0xD000 + n, other, None);
    }
    let handle = world.main_region.unwrap();
    for (n, worn) in [collar_item, cuff, shirt, free].into_iter().enumerate() {
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
    // RlvFolderLocks::getLockedFolders, ST_ATTACHMENT (rlvlocks.cpp): an
    // issuer's own lock takes the folder of its own item only, so the
    // collar, whose item is outside #RLV, names its link's folder
    command(&mut world.rlva, collar_item, "@detachallthis=n,attachallthis=n");
    let free_locks = super::inventory::FolderLocks::default();
    assert!(
        world.rlva_folder_allowed_in(&free_locks, collar, false, Uuid::nil()),
        "the issuer's own item is outside #RLV"
    );
    command(&mut world.rlva, collar_item, "@clear");
    command(&mut world.rlva, collar_item, "@detachallthis:Collar=n,attachallthis:Collar=n");
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
    // RlvFolderLocks::getLockedItems: a worn item is locked by the folder of
    // any of its links under #RLV, not only by its own folder (issue #2677)
    assert!(
        !world.rlva_can_remove(collar_item, Uuid::nil()),
        "worn through a link in the issuer's folder"
    );
    assert!(!world.rlva_can_remove(cuff, Uuid::nil()), "worn from the named folder");
    assert!(
        !world.rlva_can_remove(shirt, Uuid::nil()),
        "worn through a link in the named folder"
    );
    assert!(world.rlva_can_remove(free, Uuid::nil()), "no link in a locked folder");
    world.rlva_watchdog_tick(std::time::Instant::now());
    world.rlva_watchdog_tick(std::time::Instant::now());
    assert!(world.rlva.watchdog.rejected.is_empty(), "nothing worn is rejected");
    assert_eq!(
        world.rlva.watchdog.checked.as_ref().expect("checked").locked,
        BTreeSet::from([collar_item, cuff, shirt]),
        "the watchdog holds the linked items too"
    );
    command(&mut world.rlva, cuff, "@detachthis:Cuffs=y");
    assert!(world.rlva_can_remove(shirt, Uuid::nil()), "released with the rule");
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

/// RlvForceWear::done (rlvhelper.cpp): a JasX-style state change, one
/// folder off and the next on in a single message, is one COF update that
/// takes the old folder off even while its object is still on the avatar
/// (#1265). One update per item rebaked once per step, and a step planned
/// before the previous detach landed put the object back.
#[test]
fn rlva_folder_state_change_is_one_outfit_change_that_drops_the_old_folder() {
    use crate::world::appearance::{self, Action, WearRequest};
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    let root = Uuid::from_u128(10);
    let (shared, jasx, dressed, underwear, cof) = (
        Uuid::from_u128(11),
        Uuid::from_u128(12),
        Uuid::from_u128(13),
        Uuid::from_u128(14),
        Uuid::from_u128(20),
    );
    let id = Uuid::from_u128;
    w.inventory.root = root;
    for (folder, parent, name, kind, children, contents) in [
        (root, Uuid::nil(), "My Inventory", -1, vec![shared, cof], vec![]),
        (shared, root, "#RLV", -1, vec![jasx], vec![]),
        (jasx, shared, "JasX", -1, vec![dressed, underwear], vec![]),
        (dressed, jasx, "Dressed", -1, vec![], vec![id(31), id(32)]),
        (underwear, jasx, "Underwear", -1, vec![], vec![id(33), id(34)]),
        (cof, root, "Current Outfit", 46, vec![], vec![id(131), id(132)]),
    ] {
        w.inventory.folders.insert(
            folder,
            Folder {
                info: InvFolder {
                    id: folder,
                    parent,
                    name: name.to_owned(),
                    type_default: kind,
                    ..Default::default()
                },
                children,
                items: contents,
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    // (id, folder, name, asset type, flags): shirt and jacket dressed, the
    // undershirt and a chest attachment underneath.
    for (item, parent, name, asset_type, flags) in [
        (31, dressed, "Shirt", 5, 4),
        (32, dressed, "Jacket", 6, 5),
        (33, underwear, "Undershirt", 5, 10),
        (34, underwear, "Bra", 6, 1),
    ] {
        w.inventory.items.insert(
            id(item),
            InvItem {
                id: id(item),
                parent,
                name: name.to_owned(),
                asset_type,
                flags,
                ..Default::default()
            },
        );
    }
    for item in [31, 32] {
        w.inventory.items.insert(
            id(item + 100),
            InvItem {
                id: id(item + 100),
                parent: cof,
                asset_id: id(item),
                asset_type: 24,
                ..Default::default()
            },
        );
    }
    command(&mut w.rlva, A, "@detachall:JasX/Dressed=force,attachallover:JasX/Underwear=force");
    w.rlva_pending();
    assert_eq!(w.rlva.appearance.len(), 1, "both commands share one outfit change");
    let (actor, action) = w.rlva.appearance[0].clone();
    assert_eq!(actor, A);
    let Action::Force { remove, wear } = &action else {
        panic!("a forced state change is one Force batch");
    };
    assert_eq!(remove, &[id(31), id(32)]);
    let mut worn: Vec<_> = wear.iter().map(|w| (w.item.as_u128(), w.replace)).collect();
    worn.sort_unstable();
    assert_eq!(worn, [(33, false), (34, false)], "attachallover adds");
    // The jacket's object is still on the avatar: its detach is in flight.
    let on_avatar = std::collections::HashMap::from([(id(32), 5)]);
    let (change, sync) = appearance::plan(&w.inventory, action.clone(), &on_avatar).expect("planned");
    let mut links: Vec<_> = change.links.iter().map(|l| l.target.as_u128()).collect();
    links.sort_unstable();
    assert_eq!(links, [33, 34], "the old folder goes, the new one comes, in one write");
    assert!(sync);
    let rez = appearance::attachments_for_action(&w.inventory, A, &action);
    assert_eq!(rez.iter().map(|a| (a.item_id.as_u128(), a.add)).collect::<Vec<_>>(), [(34, true)]);
    // A wear planned on its own while that object is still on: the planner
    // folds it back, why the queue waits for detaches (rlva_detach_in_flight).
    let alone = Action::WearItem {
        item: id(33),
        replace: false,
        point: 0,
    };
    let (change, _) = appearance::plan(&w.inventory, alone, &on_avatar).expect("planned");
    assert!(change.links.iter().any(|l| l.target == id(32)));
    // A wear after a removal of the same item keeps it on.
    w.rlva.appearance.clear();
    command(&mut w.rlva, A, "@remattach=force,attach:JasX/Dressed=force");
    w.rlva_pending();
    let [(_, Action::Force { remove, wear })] = w.rlva.appearance.as_slice() else {
        panic!("one batch");
    };
    assert!(!remove.contains(&id(32)), "re-worn in the same batch");
    assert!(wear.iter().any(|w| w.item == id(32)));
    // A removal after a wear cancels it.
    w.rlva.appearance.clear();
    command(&mut w.rlva, A, "@attach:JasX/Dressed=force,detachall:JasX/Dressed=force");
    w.rlva_pending();
    assert!(
        matches!(w.rlva.appearance.as_slice(), [(_, Action::Force { wear, .. })] if wear.is_empty()),
        "the later removal cancels the wear"
    );
    // A later replacing wear of the same layer wins (RlvForceWear::addWearable).
    w.rlva.appearance.clear();
    let other_shirt = id(35);
    w.inventory.items.insert(
        other_shirt,
        InvItem {
            id: other_shirt,
            asset_type: 5,
            flags: 4,
            ..Default::default()
        },
    );
    let shirt = WearRequest {
        item: id(31),
        replace: true,
        point: 0,
    };
    w.rlva_force_wear(A, shirt);
    w.rlva_force_wear(
        A,
        WearRequest {
            item: other_shirt,
            replace: true,
            point: 0,
        },
    );
    assert!(matches!(w.rlva.appearance.as_slice(), [(_, Action::Force { wear, .. })] if wear.len() == 1 && wear[0].item == other_shirt));
    // Another object's commands open their own batch.
    w.rlva_force_wear(B, shirt);
    assert_eq!(w.rlva.appearance.len(), 2);
}

/// The COF write of an outfit change is acknowledged before its detaches
/// land: RLVa's next step waits while a detach it asked for is still worn,
/// for a bounded time.
#[test]
fn rlva_waits_for_requested_detaches_to_leave_the_avatar() {
    use aurora_net::AttachRequest;
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let item = Uuid::from_u128(900);
    world.inventory.items.insert(
        item,
        aurora_net::inventory::InvItem {
            id: item,
            asset_type: 6,
            flags: 5,
            ..Default::default()
        },
    );
    let region = world.main_region.unwrap();
    let object = crate::demo::outfit_attachment(&AttachRequest {
        item_id: item,
        owner_id: world.agent_id,
        point: 5,
        add: true,
        flags: 5,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    });
    let index = world.objects.upsert(region, object);
    assert!(world.outfit_attachment_items().contains(&item));
    let now = std::time::Instant::now();
    assert!(!world.rlva_detach_in_flight(now), "nothing requested");
    world.rlva_note_attachment_command(&NetCommand::DetachAttachments(vec![item]), Uuid::nil(), now);
    assert!(world.rlva_detach_in_flight(now));
    assert!(
        !world.rlva_detach_in_flight(now + std::time::Duration::from_secs(11)),
        "a detach that never lands stops holding the queue"
    );
    world.objects.remove_idx(index);
    assert!(!world.rlva_detach_in_flight(now), "landed");
}

#[test]
fn empty_trash_ignores_rlva_locks_without_exempting_other_purges() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{
        InvFolder,
        operations::{Mutation, Operation},
    };
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    command(&mut world.rlva, A, "@detach=n");
    assert!(world.rlva.inventory_locked());
    let trash = Uuid::from_u128(90);
    let unloaded = Uuid::from_u128(91);
    for (id, kind, state) in [(trash, 14, FetchState::Fetched), (unloaded, -1, FetchState::Unknown)] {
        world.inventory.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    type_default: kind,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state,
                library: false,
            },
        );
    }
    let purge = |id| Mutation {
        operations: vec![Operation::Purge(id)],
        ..Default::default()
    };
    assert!(world.rlva_inventory_mutation_allowed(&purge(trash), Uuid::nil()));
    assert!(!world.rlva_inventory_mutation_allowed(&purge(unloaded), Uuid::nil()));
    assert!(!world.rlva_inventory_mutation_allowed(&purge(Uuid::from_u128(92)), Uuid::nil()));
}

#[test]
fn rlva_linking_refuses_the_locked_seat_like_can_stand_but_sittp_alone_does_not() {
    use crate::world::World;
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events()
        .into_iter()
        .chain(crate::demo::selection_cycle_events())
        .chain(crate::demo::sit_events(250))
        .chain(crate::demo::sit_events(300))
    {
        world.apply(event);
    }
    world.agent.seated = true;
    world.rlva_configure(true);
    let index = |world: &World, id| world.objects.iter().find(|(_, o)| o.key.local_id == id).unwrap().0;
    let seat = index(&world, 950);
    let other = index(&world, 99610);
    command(&mut world.rlva, A, "@sittp=n");
    assert!(world.rlva_can_link(&[seat, other]));
    command(&mut world.rlva, A, "@unsit=n");
    assert!(!world.rlva_can_link(&[seat, other]));
    assert!(world.rlva_can_link(&[other]));
    let object = world.objects.get(seat).unwrap();
    assert!(!world.rlva_allows_build(&aurora_net::build::BuildCmd::Link {
        handle: object.key.region,
        local_ids: vec![object.key.local_id]
    }));
    command(&mut world.rlva, A, "@unsit=y");
    assert!(world.rlva_can_link(&[seat, other]));
    command(&mut world.rlva, A, "@editworld=n");
    assert!(!world.rlva_can_link(&[seat, other]));
}

/// #3154: `@setenv_daytime:-1=force` and `@setenv_reset=force` go back to the
/// shared environment, unless `@setenv=n` came before them.
#[test]
fn rlva_environment_reset_returns_to_the_shared_environment_unless_locked() {
    use crate::world::{World, eep::SkyFrame, eep_env::SourceRights};
    for reset in ["setenv_daytime:-1=force", "setenv_reset=force"] {
        for (lines, shared) in [
            (vec![format!("@{reset}")], true),
            (vec![format!("@{reset},setenv=n")], true),
            (vec![format!("@setenv=n,{reset}")], false),
            (vec!["@setenv=n".to_owned(), format!("@{reset}")], false),
            (vec!["@setenv=n".to_owned(), "@setenv=y".to_owned(), format!("@{reset}")], true),
        ] {
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            world.rlva.configure(true);
            // a sky the user picked, then an RLVa sky over it
            world.eep.apply_local_sky(SkyFrame::default(), SourceRights::Flags(0));
            world.on_chat(chat("@setenv_ambient:0.2/0.4/0.6=force"));
            world.rlva_pending();
            assert!(world.rlva.environment.is_some());
            for line in &lines {
                world.on_chat(chat(line));
                world.rlva_pending();
            }
            assert_eq!(world.eep.has_local(), !shared, "{lines:?}: the user's local sky");
            assert!(world.rlva.environment.is_none(), "{lines:?}: RLVa's sky stays");
        }
    }
    assert!(environment::supports("setenv_reset", true));
    assert!(!environment::supports("getenv_reset", false));
}

#[test]
fn rlva_environment_forces_respect_other_objects_at_receipt_and_at_dispatch() {
    use crate::world::{World, eep::SkyFrame, eep_env::SourceRights};
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    world.eep.apply_local_sky(SkyFrame::default(), SourceRights::Flags(0));
    let say = |world: &mut World, source_id, line| world.on_chat(ChatMessage { source_id, ..chat(line) });

    say(&mut world, A, "@setenv=n");
    say(&mut world, B, "@setenv_ambient:0.2/0.4/0.6=force");
    say(&mut world, A, "@setenv=y");
    world.rlva_pending();
    assert!(
        world.rlva.environment.is_none(),
        "releasing a lock cannot revive another object's lighting request"
    );

    say(&mut world, A, "@setenv=n");
    for cmd in [
        "@setenv_reset=force",
        "@setenv_daytime:-1=force",
        "@setenv_ambient:0.2/0.4/0.6=force",
    ] {
        say(&mut world, B, cmd);
        assert!(world.rlva.pending.is_empty(), "{cmd}: the other object's lock already applies");
    }
    say(&mut world, A, "@setenv=y");
    world.rlva_pending();
    assert!(world.eep.has_local() && world.rlva.environment.is_none());

    say(&mut world, B, "@setenv_reset=force");
    say(&mut world, A, "@setenv=n");
    world.rlva_pending();
    assert!(world.eep.has_local());
    assert_eq!(
        world.rlva.environment.map(|(actor, _)| actor),
        Some(A),
        "the new lock also protects its sky"
    );
    say(&mut world, A, "@setenv_reset=force");
    world.rlva_pending();
    assert!(
        world.eep.has_local() && world.rlva.environment.is_none(),
        "the holder may clear its own sky"
    );
    assert!(world.eep.rlva_locked);

    say(&mut world, A, "@setenv=y");
    say(&mut world, B, "@setenv_preset:midday=force");
    world.rlva_pending();
    assert!(world.rlva.environment_request.is_some());
    say(&mut world, B, "@setenv_reset=force");
    assert!(!world.rlva.pending.is_empty());
    world.reset();
    world.rlva.configure(true);
    world.eep.apply_local_sky(SkyFrame::default(), SourceRights::Flags(0));
    world.rlva_pending();
    assert!(
        world.eep.has_local(),
        "the previous session's reset cannot clear a new local choice"
    );
    assert!(!world.eep.rlva_locked && world.rlva.environment.is_none());
    assert!(world.rlva.environment_request.is_none());
}

/// An attachment on the demo avatar, with its inventory item.
fn worn_attachment(world: &mut crate::world::World, id: u128, point: u8) -> Uuid {
    let item = Uuid::from_u128(id);
    world.inventory.items.insert(
        item,
        aurora_net::inventory::InvItem {
            id: item,
            asset_type: 6,
            flags: u32::from(point),
            ..Default::default()
        },
    );
    let object = crate::demo::outfit_attachment(&attach_request(item, point, true));
    let region = world.main_region.unwrap();
    world.objects.upsert(region, object);
    item
}

fn attach_request(item: Uuid, point: u8, add: bool) -> aurora_net::AttachRequest {
    aurora_net::AttachRequest {
        item_id: item,
        owner_id: Uuid::nil(),
        point,
        add,
        flags: u32::from(point),
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    }
}

/// LLAgentWearables::userRemoveMultipleAttachments: the locked attachments
/// leave the request, the others are detached, and nothing is refused.
#[test]
fn rlva_detach_request_leaves_locked_attachments_out_instead_of_being_refused() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let locked = worn_attachment(&mut world, 900, 5);
    let free = worn_attachment(&mut world, 901, 6);
    let mut request = NetCommand::DetachAttachments(vec![locked, free]);
    assert!(world.rlva_prepare_attachments(&mut request, Uuid::nil()));
    assert!(
        matches!(&request, NetCommand::DetachAttachments(items) if *items == [locked, free]),
        "RLVa off: untouched"
    );
    world.rlva.configure(true);
    command(&mut world.rlva, A, "@detach:left hand=n");
    assert!(
        !world.rlva_allows_command(&request, Uuid::nil()),
        "the whole request would be refused"
    );
    assert!(world.rlva_prepare_attachments(&mut request, Uuid::nil()));
    assert!(matches!(&request, NetCommand::DetachAttachments(items) if *items == [free]));
    assert!(world.rlva_allows_command(&request, Uuid::nil()));
    let mut request = NetCommand::DetachAttachments(vec![locked]);
    assert!(
        !world.rlva_prepare_attachments(&mut request, Uuid::nil()),
        "nothing is left to send, and no notice is due"
    );
    // The watchdog's own requests are exact: they are not filtered here.
    let mut request = NetCommand::DetachAttachments(vec![locked, free]);
    assert!(world.rlva_prepare_attachments(&mut request, super::watchdog::ACTOR));
    assert!(matches!(&request, NetCommand::DetachAttachments(items) if items.len() == 2));
}

/// LLAttachmentsMgr::addAttachmentRequest: an attachment a lock refuses is
/// dropped from the request, the others are still attached.
#[test]
fn rlva_attach_request_leaves_refused_attachments_out_instead_of_being_refused() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let (closed, open) = (Uuid::from_u128(910), Uuid::from_u128(911));
    for (item, point) in [(closed, 6_u32), (open, 1)] {
        world.inventory.items.insert(
            item,
            aurora_net::inventory::InvItem {
                id: item,
                asset_type: 6,
                flags: point,
                ..Default::default()
            },
        );
    }
    command(&mut world.rlva, A, "@addattach:right hand=n");
    let mut request = NetCommand::RezAttachments(vec![attach_request(closed, 6, true), attach_request(open, 1, true)]);
    assert!(!world.rlva_allows_command(&request, Uuid::nil()));
    assert!(world.rlva_prepare_attachments(&mut request, Uuid::nil()));
    assert!(matches!(&request, NetCommand::RezAttachments(items) if items.len() == 1 && items[0].item_id == open));
    assert!(world.rlva_allows_command(&request, Uuid::nil()));
    let mut request = NetCommand::RezAttachments(vec![attach_request(closed, 6, true)]);
    assert!(!world.rlva_prepare_attachments(&mut request, Uuid::nil()));
}

/// RlvForceWear::addAttachment: attachments with no known point (the default
/// point) each replace what their own point holds (#3635). The point is the
/// one RlvAttachPtLookup::getAttachPointIndex reads from the names: a point
/// saved on the item alone is no slot, so all four replace.
#[test]
fn rlva_forced_replace_wears_every_attachment_of_unknown_point_as_a_replacement() {
    use crate::world::appearance::Action;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    let id = Uuid::from_u128;
    let (root, shared, a, cof) = (id(10), id(11), id(12), id(20));
    w.inventory.root = root;
    for (folder, parent, name, kind, children, contents) in [
        (root, Uuid::nil(), "My Inventory", -1, vec![shared, cof], vec![]),
        (shared, root, "#RLV", -1, vec![a], vec![]),
        (a, shared, "a", -1, vec![], vec![id(31), id(32), id(33), id(34)]),
        (cof, root, "Current Outfit", 46, vec![], vec![]),
    ] {
        w.inventory.folders.insert(
            folder,
            Folder {
                info: InvFolder {
                    id: folder,
                    parent,
                    name: name.to_owned(),
                    type_default: kind,
                    ..Default::default()
                },
                children,
                items: contents,
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    // Two objects with no saved point, two saved on the chest.
    for (item, name, flags) in [(31, "TestA", 0), (32, "TestA2", 0), (33, "TestB", 1), (34, "TestB2", 1)] {
        w.inventory.items.insert(
            id(item),
            InvItem {
                id: id(item),
                parent: a,
                name: name.to_owned(),
                asset_type: 6,
                flags,
                ..Default::default()
            },
        );
    }
    command(&mut w.rlva, A, "@attachall:a=force");
    w.rlva_pending();
    let [(_, Action::Force { wear, .. })] = w.rlva.appearance.as_slice() else {
        panic!("one batch");
    };
    let mut modes: Vec<_> = wear.iter().map(|w| (w.item.as_u128(), w.replace)).collect();
    modes.sort_unstable();
    assert_eq!(modes, [(31, true), (32, true), (33, true), (34, true)]);
}

/// A world with RLVa on and a fetched inventory: folders (id, parent, name,
/// type) from the root down, items (id, folder, asset type, flags, target of
/// a link or 0).
fn inventory_world(folders: &[(u128, u128, &str, i32)], items: &[(u128, u128, i32, u32, u128)]) -> crate::world::World {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    let id = Uuid::from_u128;
    let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.rlva.configure(true);
    w.inventory.root = id(folders[0].0);
    for &(folder, parent, name, kind) in folders {
        w.inventory.folders.insert(
            id(folder),
            Folder {
                info: InvFolder {
                    id: id(folder),
                    parent: if parent == 0 { Uuid::nil() } else { id(parent) },
                    name: name.to_owned(),
                    type_default: kind,
                    ..Default::default()
                },
                children: folders.iter().filter(|f| f.1 == folder).map(|f| id(f.0)).collect(),
                items: items.iter().filter(|it| it.1 == folder).map(|it| id(it.0)).collect(),
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    for &(item, parent, asset_type, flags, target) in items {
        w.inventory.items.insert(
            id(item),
            InvItem {
                id: id(item),
                parent: id(parent),
                asset_type,
                flags,
                asset_id: if target == 0 { Uuid::nil() } else { id(target) },
                ..Default::default()
            },
        );
    }
    w
}

/// The wear menus ask for the locks every frame: the snapshot is built once
/// and handed out again until a rule, the inventory or what is worn changes.
#[test]
fn rlva_wear_locks_are_built_once_and_rebuilt_only_when_their_inputs_change() {
    let id = Uuid::from_u128;
    let mut w = inventory_world(
        &[
            (10, 0, "My Inventory", -1),
            (11, 10, "#RLV", -1),
            (12, 11, "Locked", -1),
            (13, 12, "Inside", -1),
            (14, 11, "Free", -1),
            (15, 10, "Clothing", -1),
        ],
        &[(31, 12, 5, 4, 0), (32, 13, 5, 5, 0), (33, 14, 5, 6, 0), (34, 15, 5, 7, 0)],
    );
    let unlocked = w.rlva_wear_locks();
    assert!(
        std::sync::Arc::ptr_eq(&unlocked, &w.rlva_wear_locks()),
        "no lock: one shared answer"
    );
    command(&mut w.rlva, A, "@attachallthis:Locked=n");
    let locks = w.rlva_wear_locks();
    assert!(
        std::sync::Arc::ptr_eq(&locks, &w.rlva_wear_locks()),
        "same inputs: the same snapshot"
    );
    assert!(!locks.folder_allows(id(12), true) && !locks.folder_allows(id(13), true));
    assert!(locks.folder_allows(id(14), true) && locks.folder_allows(id(15), true));
    assert!(locks.folder_allows(id(12), false), "an attach lock is not a detach lock");
    let none = crate::world::rlva::wear_locks::Wear {
        add: false,
        replace: false,
    };
    assert_eq!(locks.wear(&w.inventory, id(32), 0), none);
    assert_ne!(locks.wear(&w.inventory, id(33), 0), none);
    // A rule changes the answer and the snapshot with it.
    command(&mut w.rlva, A, "@detachthis:Free=n");
    let relocked = w.rlva_wear_locks();
    assert!(!std::sync::Arc::ptr_eq(&locks, &relocked));
    assert!(!relocked.folder_allows(id(14), false));
    // So does the inventory: the folder moved out of the locked one is free.
    w.inventory.folders.get_mut(&id(12)).unwrap().children.clear();
    w.inventory.folders.get_mut(&id(11)).unwrap().children.push(id(13));
    w.inventory.folders.get_mut(&id(13)).unwrap().info.parent = id(11);
    assert!(
        !w.rlva_wear_locks().folder_allows(id(13), true),
        "the inventory did not say it changed"
    );
    w.inventory.generation += 1;
    let moved = w.rlva_wear_locks();
    assert!(!std::sync::Arc::ptr_eq(&relocked, &moved));
    assert!(moved.folder_allows(id(13), true) && !moved.folder_allows(id(12), true));
    // A lock by placement reaches the folders outside #RLV too.
    command(&mut w.rlva, A, "@unsharedwear=n");
    let placed = w.rlva_wear_locks();
    assert!(!placed.folder_allows(id(15), true) && placed.folder_allows(id(14), true));
}

/// A "…this" lock on what is worn names the folder of the worn item under
/// #RLV only (RlvCommandOptionGetPath::getItemIDs reads the COF links'
/// targets, RlvInventory::getPath their folders; rlvhelper.cpp,
/// rlvinventory.cpp): the folder of another link to it is not locked, and
/// the rest of the inventory is not read to find it.
#[test]
fn rlva_this_lock_on_a_layer_finds_the_shared_folder_of_the_worn_item() {
    let id = Uuid::from_u128;
    let mut w = inventory_world(
        &[
            (10, 0, "My Inventory", -1),
            (11, 10, "#RLV", -1),
            (12, 11, "Original", -1),
            (13, 11, "Linked", -1),
            (14, 10, "Elsewhere", -1),
            (20, 10, "Current Outfit", 46),
        ],
        &[(31, 12, 5, 4, 0), (41, 13, 24, 0, 31), (42, 14, 24, 0, 31), (43, 20, 24, 0, 31)],
    );
    command(&mut w.rlva, A, "@detachthis:shirt=n");
    let locks = w.rlva_wear_locks();
    assert!(!locks.folder_allows(id(12), false), "the folder of the worn shirt");
    assert!(locks.folder_allows(id(13), false), "the folder of another link to it");
    assert!(locks.folder_allows(id(14), false), "a link outside #RLV locks nothing");
    assert!(locks.folder_allows(id(20), false));
    assert!(!locks.can_remove(id(31)));
}

/// rlvCanDeleteOrReturn: the seat the agent cannot leave stays where it is,
/// and so does what the agent owns or may modify while it could not rez it
/// again; a copy only keeps the seat (handle_take_copy).
#[test]
fn rlva_delete_take_return_and_duplicate_follow_the_delete_or_return_rule() {
    use crate::ui::context::flags::{OBJECT_GROUP_OWNED, OBJECT_MODIFY, OBJECT_YOU_OWNER};
    use crate::world::World;
    use aurora_net::build::{BuildCmd, derez};
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events()
        .into_iter()
        .chain(crate::demo::selection_cycle_events())
        .chain(crate::demo::sit_events(250))
        .chain(crate::demo::sit_events(300))
    {
        world.apply(event);
    }
    world.agent.seated = true;
    world.rlva_configure(true);
    let index = |world: &World, id| world.objects.iter().find(|(_, o)| o.key.local_id == id).unwrap().0;
    let (seat, other) = (index(&world, 950), index(&world, 99610));
    let own = OBJECT_YOU_OWNER | OBJECT_MODIFY | OBJECT_GROUP_OWNED;
    world.objects.get_mut(seat).unwrap().update_flags |= OBJECT_YOU_OWNER;
    world.objects.get_mut(other).unwrap().update_flags &= !own;
    let derez = |world: &World, i: usize, destination| {
        let key = world.objects.get(i).unwrap().key;
        NetCommand::Build(BuildCmd::DeRez {
            handle: key.region,
            local_ids: vec![key.local_id],
            destination,
            destination_id: Uuid::nil(),
            group_id: Uuid::nil(),
        })
    };
    let allowed = |world: &World, i: usize, destination| world.rlva_allows_command(&derez(world, i, destination), Uuid::nil());
    let moving = [derez::TRASH, derez::TAKE_INTO_AGENT_INVENTORY, derez::RETURN_TO_OWNER];
    assert!(moving.iter().all(|d| allowed(&world, seat, *d)), "no restriction yet");
    command(&mut world.rlva, A, "@unsit=n");
    assert!(!world.rlva_can_delete_or_return(seat) && world.rlva_can_delete_or_return(other));
    assert!(moving.iter().all(|d| !allowed(&world, seat, *d) && allowed(&world, other, *d)));
    assert!(
        !allowed(&world, seat, derez::ACQUIRE_TO_AGENT_INVENTORY),
        "nor a copy of the locked seat"
    );
    assert!(allowed(&world, other, derez::ACQUIRE_TO_AGENT_INVENTORY));
    // The seat's contents take nothing in and let nothing out, but an item
    // already inside keeps its name and its properties
    // (LLTaskInvFVBridge::isItemRenameable tests a locked attachment only,
    // LLSidepanelItemInfo nothing).
    // derenderObject: the prim sat on stays, the prims linked to it go.
    let seat_id = world.objects.get(seat).unwrap().full_id;
    assert_eq!(world.rlva_seat_derender(seat, true), Some(Vec::new()), "the root alone is kept");
    let linked = world.rlva_seat_derender(seat, false).expect("the seat held by @unsit");
    assert!(!linked.contains(&seat_id) && !linked.contains(&world.agent_id));
    for prim in &linked {
        let i = world.objects.index_of_uuid(prim).unwrap();
        assert_eq!(world.rlva_seat_derender(i, true), Some(vec![*prim]), "a linked prim is hidden");
    }
    assert_eq!(world.rlva_seat_derender(other, false), None);
    let tool = crate::build::BuildTool::default();
    world.god_level = 1;
    assert!(!tool.contents_editable(&world, seat) && tool.contents_editable(&world, other));
    assert!(tool.contents_items_editable(&world, seat) && tool.contents_modifiable(&world, seat));
    world.god_level = 0;
    let update = |world: &World, item_id: Uuid| {
        let key = world.objects.get(seat).unwrap().key;
        world.rlva_allows_command(
            &NetCommand::Build(BuildCmd::UpdateTaskInventory {
                handle: key.region,
                local_id: key.local_id,
                item: Box::new(aurora_net::build::TaskItem {
                    item_id,
                    ..Default::default()
                }),
            }),
            Uuid::nil(),
        )
    };
    let ours = Uuid::from_u128(9_951);
    world.inventory.items.insert(
        ours,
        aurora_net::inventory::InvItem {
            id: ours,
            ..Default::default()
        },
    );
    assert!(update(&world, Uuid::from_u128(9_950)), "an item of the seat is renamed");
    assert!(!update(&world, ours), "an item of our inventory is a drop");
    command(&mut world.rlva, A, "@unsit=y");
    assert!(update(&world, ours));
    assert!(moving.iter().all(|d| allowed(&world, seat, *d)));
    // @rez: what the agent owns, may modify or holds through a group stays.
    command(&mut world.rlva, A, "@rez=n");
    assert!(
        moving.iter().all(|d| !allowed(&world, seat, *d)),
        "our own object could not be rezzed again"
    );
    assert!(
        moving.iter().all(|d| allowed(&world, other, *d)),
        "someone else's object can still be returned"
    );
    assert!(
        allowed(&world, seat, derez::ACQUIRE_TO_AGENT_INVENTORY),
        "a copy leaves the object in place"
    );
    for flag in [OBJECT_MODIFY, OBJECT_GROUP_OWNED] {
        world.objects.get_mut(other).unwrap().update_flags |= flag;
        assert!(!world.rlva_can_delete_or_return(other));
        world.objects.get_mut(other).unwrap().update_flags &= !own;
    }
    let key = world.objects.get(seat).unwrap().key;
    let duplicate = NetCommand::Build(BuildCmd::DuplicateOnRay {
        handle: key.region,
        local_ids: vec![key.local_id],
        group_id: Uuid::nil(),
        ray_start: glam::Vec3::ZERO,
        ray_end: glam::Vec3::ZERO,
        ray_target: Uuid::nil(),
        bypass_raycast: true,
        copy_centers: false,
        copy_rotates: false,
    });
    // LLSelectMgr::selectDuplicate (Ctrl+D): the original answers like a delete.
    let ctrl_d = |world: &World, i: usize| {
        let key = world.objects.get(i).unwrap().key;
        world.rlva_allows_command(
            &NetCommand::Build(BuildCmd::Duplicate {
                handle: key.region,
                local_ids: vec![key.local_id],
                offset: glam::Vec3::ZERO,
                flags: 0,
                group_id: Uuid::nil(),
            }),
            Uuid::nil(),
        )
    };
    assert!(!world.rlva_allows_command(&duplicate, Uuid::nil()), "a copy on the ground is a rez");
    assert!(!ctrl_d(&world, seat) && ctrl_d(&world, other));
    command(&mut world.rlva, A, "@rez=y");
    assert!(world.rlva_allows_command(&duplicate, Uuid::nil()) && ctrl_d(&world, seat));
    // @editworld alone keeps no object in place: the rule is not asked.
    command(&mut world.rlva, A, "@editworld=n");
    assert!(!world.rlva_can_edit(seat) && world.rlva_can_delete_or_return(seat));
    assert!(moving.iter().all(|d| allowed(&world, seat, *d)) && ctrl_d(&world, seat));
    command(&mut world.rlva, A, "@editworld=y,edit=n");
    assert!(!allowed(&world, seat, derez::TRASH) && !ctrl_d(&world, seat));
    assert!(
        world.rlva_allows_command(&duplicate, Uuid::nil()),
        "LLToolPlacer::placeObject asks @rez and @interact alone"
    );
    assert!(
        !world.rlva_can_edit(other) && allowed(&world, other, derez::ACQUIRE_TO_AGENT_INVENTORY),
        "an edit lock does not refuse a copy"
    );
}

/// LLGroupActions::leave: the active group cannot be left under @setgroup.
#[test]
fn rlva_setgroup_refuses_leaving_the_active_group_only() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    let (active, other) = (Uuid::from_u128(71), Uuid::from_u128(72));
    world.groups.active = active;
    assert!(world.rlva_allows_command(&NetCommand::LeaveGroup(active), Uuid::nil()));
    command(&mut world.rlva, A, "@setgroup=n");
    assert!(!world.rlva_allows_command(&NetCommand::LeaveGroup(active), Uuid::nil()));
    assert!(world.rlva_allows_command(&NetCommand::LeaveGroup(other), Uuid::nil()));
}

/// RlvHandler::onGetAttach and onGetAttachNames, RlvForceWear::isForceDetachable:
/// a viewer's own bridge on Center 2 is not part of what the avatar wears, and
/// a script never takes it off. Its name alone does not make a bridge.
#[test]
fn rlva_attachment_queries_and_forced_detach_leave_the_viewer_bridge_out() {
    use crate::world::World;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::InvFolder;
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let id = Uuid::from_u128;
    let (root, aurora, legacy, other, cof) = (id(940), id(941), id(942), id(943), id(945));
    world.inventory.root = root;
    // A forced detach waits for a complete Current Outfit folder.
    for (folder, parent, name, kind) in [
        (root, Uuid::nil(), "My Inventory", -1),
        (aurora, root, crate::world::bridge::FOLDER, -1),
        (legacy, root, "#Firestorm", -1),
        (other, root, "Objects", -1),
        (cof, root, "Current Outfit", crate::world::outfit::FT_CURRENT_OUTFIT),
    ] {
        world.inventory.folders.insert(
            folder,
            Folder {
                info: InvFolder {
                    id: folder,
                    parent,
                    name: name.to_owned(),
                    type_default: kind,
                    ..Default::default()
                },
                children: if folder == root {
                    vec![aurora, legacy, other, cof]
                } else {
                    Vec::new()
                },
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    let reply = |world: &mut World, text: &str| world.rlva_inventory_query(&Command::parse(text).unwrap(), A).unwrap();
    let listed = |world: &mut World, text: &str| reply(world, text).split(',').any(|p| p == "Center 2");
    assert!(!listed(&mut world, "getattachnames=123"), "nothing on Center 2 yet");
    let bridge = worn_attachment(&mut world, 944, 31);
    let agent = world.agent_id;
    for (name, parent, hidden) in [
        ("Some HUD", other, false),
        (crate::world::bridge::NAME, aurora, true),
        (crate::world::bridge::NAME, other, false),
        ("#Firestorm LSL Bridge v2.32", legacy, true),
        ("#Firestorm LSL Bridge v2.", legacy, false),
        ("#Firestorm LSL Bridge v2.32", other, false),
    ] {
        let item = world.inventory.items.get_mut(&bridge).unwrap();
        (item.name, item.parent, item.owner) = (name.to_owned(), parent, agent);
        assert_eq!(listed(&mut world, "getattachnames=123"), !hidden, "{name}");
        assert_eq!(listed(&mut world, "getremattachnames=123"), !hidden, "{name}");
        assert_eq!(reply(&mut world, "getattach=123").as_bytes()[31] == b'1', !hidden, "{name}");
        command(&mut world.rlva, A, "@remattach:center 2=force");
        world.rlva_pending();
        let removed = matches!(
            world.rlva.appearance.pop(),
            Some((_, crate::world::appearance::Action::Force { remove, .. })) if remove == vec![bridge]
        );
        assert_eq!(removed, !hidden, "{name}");
        world.rlva.appearance.clear();
    }
    // Someone else's object with the bridge's name is an ordinary attachment.
    let item = world.inventory.items.get_mut(&bridge).unwrap();
    (item.name, item.parent, item.owner) = (crate::world::bridge::NAME.to_owned(), aurora, B);
    assert!(listed(&mut world, "getattachnames=123"));
}

/// FSLSLBridge::canDetach (LLVOAvatarSelf::detachAttachmentIntoInventory,
/// RlvAttachmentLockWatchdog::onDetach) and the fRlvForce of
/// LLAttachmentsMgr::addAttachmentRequest: the viewer takes its own bridge
/// off, wears it again and starts its script whatever locks the points and
/// the editing. Any other attachment on Center 2 obeys them.
#[test]
fn rlva_locks_leave_the_viewer_its_own_bridge() {
    use crate::world::World;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::build::BuildCmd;
    use aurora_net::inventory::InvFolder;
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let id = Uuid::from_u128;
    let (root, aurora, other) = (id(940), id(941), id(943));
    world.inventory.root = root;
    for (folder, parent, name) in [
        (root, Uuid::nil(), "My Inventory"),
        (aurora, root, crate::world::bridge::FOLDER),
        (other, root, "Objects"),
    ] {
        world.inventory.folders.insert(
            folder,
            Folder {
                info: InvFolder {
                    id: folder,
                    parent,
                    name: name.to_owned(),
                    type_default: -1,
                    ..Default::default()
                },
                children: if folder == root { vec![aurora, other] } else { Vec::new() },
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    let bridge = worn_attachment(&mut world, 944, 31);
    let agent = world.agent_id;
    let object = world.worn_attachment(&bridge).and_then(|i| world.objects.get(i)).unwrap();
    let (handle, object_id) = (object.key.region, object.full_id);
    command(&mut world.rlva, A, "@remattach=n,addattach=n,edit=n");
    let wear = |world: &World| {
        let item = &world.inventory.items[&bridge];
        NetCommand::RezAttachments(vec![aurora_net::AttachRequest {
            item_id: bridge,
            owner_id: agent,
            point: 31,
            add: true,
            flags: item.flags,
            group_mask: item.group_mask,
            everyone_mask: item.everyone_mask,
            next_owner_mask: item.next_owner_mask,
            name: item.name.clone(),
            desc: item.desc.clone(),
        }])
    };
    let script = NetCommand::Build(BuildCmd::SetScriptRunning {
        handle,
        object_id,
        item_id: id(946),
        running: true,
    });
    for (name, parent, ours) in [(crate::world::bridge::NAME, aurora, true), ("Some HUD", other, false)] {
        let item = world.inventory.items.get_mut(&bridge).unwrap();
        (item.name, item.parent, item.owner) = (name.to_owned(), parent, agent);
        let allowed = |command: &NetCommand| world.rlva_allows_command(command, Uuid::nil());
        assert_eq!(allowed(&NetCommand::DetachAttachments(vec![bridge])), ours, "{name}: taken off");
        assert_eq!(allowed(&wear(&world)), ours, "{name}: worn again");
        assert_eq!(allowed(&script), ours, "{name}: its script started");
    }
}

/// LLToolPlacer::placeObject, LLToolDragAndDrop::dad3dRezObjectOnLand and
/// dad3dRezObjectOnObject: nothing is rezzed under @rez or @interact; a Ctrl
/// drop into an object's contents passes @rez, and @interact only onto a HUD.
#[test]
fn rlva_rez_and_interact_refuse_rezzing_and_creating() {
    use crate::world::World;
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    let hud = worn_attachment(&mut world, 950, 31);
    let hud = world.worn_attachment(&hud).unwrap();
    let prim = world.objects.iter().find(|(_, o)| o.key.local_id == 99610).unwrap().0;
    assert!(world.rlva_can_rez(None) && world.rlva_can_rez(Some(prim)), "RLVa off");
    world.rlva.configure(true);
    assert!(world.rlva_can_rez(None) && world.rlva_can_rez(Some(prim)));
    command(&mut world.rlva, A, "@rez=n");
    assert!(!world.rlva_can_rez(None));
    assert!(
        world.rlva_can_rez(Some(prim)),
        "putting an object into another's contents is not a rez"
    );
    command(&mut world.rlva, A, "@rez=y,interact=n");
    assert!(!world.rlva_can_rez(None) && !world.rlva_can_rez(Some(prim)));
    assert!(world.rlva_can_rez(Some(hud)), "our own HUD stays reachable under @interact");
}

/// A prim of the demo world: `parent` 0 puts it on the ground, the local id
/// of an attachment makes it one of its child prims.
fn parity_prim(world: &mut crate::world::World, id: u128, parent: u32) -> Uuid {
    let mut prim = crate::demo::outfit_attachment(&attach_request(Uuid::from_u128(id), 5, true));
    prim.parent_id = parent;
    prim.name_values.clear();
    if parent == 0 {
        prim.state = 0;
    }
    let full_id = prim.full_id;
    let region = world.main_region.unwrap();
    world.objects.upsert(region, prim);
    full_id
}

fn parity_local(world: &crate::world::World, id: Uuid) -> u32 {
    let index = world.objects.index_of_uuid(&id).unwrap();
    world.objects.get(index).unwrap().key.local_id
}

/// The chat replies queued since the last call, with their channel.
fn parity_replies(r: &mut Rlva) -> Vec<(i32, String)> {
    std::mem::take(&mut r.out)
        .into_iter()
        .filter_map(|c| match c {
            NetCommand::ChatReply { channel, message } => Some((channel, message)),
            _ => None,
        })
        .collect()
}

fn parity_reply(channel: i32, message: &str) -> (i32, String) {
    (channel, message.to_owned())
}

fn parity_demo_world() -> crate::world::World {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    world
}

/// RlvHandler::onDetach: a killed attachment clears the restrictions of
/// every object rooted on it and the listeners hear "/clear"; a kill ignored
/// during a teleport keeps the attachment and its restrictions.
#[test]
fn rlva_killed_attachment_clears_the_rules_rooted_on_it_but_not_during_a_teleport() {
    use aurora_net::NetEvent;
    let mut world = parity_demo_world();
    let handle = world.main_region.unwrap();
    let root = worn_attachment(&mut world, 0x7001, 5);
    let root_local = parity_local(&world, root);
    let child = parity_prim(&mut world, 0x7002, root_local);
    let early = parity_prim(&mut world, 0x7003, root_local);
    let early_local = parity_local(&world, early);
    let other = worn_attachment(&mut world, 0x7004, 6);
    command(&mut world.rlva, root, "@fly=n,detach=n");
    command(&mut world.rlva, child, "@jump=n");
    command(&mut world.rlva, early, "@alwaysrun=n");
    command(&mut world.rlva, other, "@unsit=n");
    command(&mut world.rlva, B, "@notify:123=n,sittp=n");
    // the watchdog remembers which attachment each issuer is part of
    world.movement_complete = true;
    world.rlva_watchdog_tick(std::time::Instant::now());
    assert_eq!(world.rlva.watchdog.sources.get(&early), Some(&root));
    world.rlva.out.clear();

    world.apply(NetEvent::TeleportStarted { flags: 0 });
    world.apply(NetEvent::ObjectsKilled {
        handle,
        local_ids: vec![root_local],
    });
    assert!(world.objects.index_of_uuid(&root).is_some(), "a kill during a teleport is ignored");
    assert!(world.rlva.has("fly") && world.rlva.has("jump") && world.rlva.has("alwaysrun"));
    assert!(parity_replies(&mut world.rlva).iter().all(|(channel, _)| *channel != 123));
    world.apply(NetEvent::TeleportFailed { reason: "no".into() });

    // a child prim killed on its own, before its root: nothing is cleared yet
    world.apply(NetEvent::ObjectsKilled {
        handle,
        local_ids: vec![early_local],
    });
    assert!(world.objects.index_of_uuid(&early).is_none());
    assert!(world.rlva.has("alwaysrun"));
    world.rlva.out.clear();
    world.apply(NetEvent::ObjectsKilled {
        handle,
        local_ids: vec![root_local],
    });
    assert!(world.objects.index_of_uuid(&root).is_none());
    assert!(!world.rlva.has("fly") && !world.rlva.has("detach"), "the root's rules");
    assert!(!world.rlva.has("jump"), "a child prim's rules");
    assert!(!world.rlva.has("alwaysrun"), "the rules of the child prim killed first");
    assert!(world.rlva.has("unsit"), "another attachment keeps its rules");
    assert!(world.rlva.has("sittp"), "an object of the world keeps its rules");
    let heard: Vec<_> = parity_replies(&mut world.rlva)
        .into_iter()
        .filter(|(channel, _)| *channel == 123)
        .collect();
    assert_eq!(heard, vec![parity_reply(123, "/clear"); 3], "one per object cleared");
    assert_eq!(
        world.rlva.restrictions().len(),
        3,
        "the other attachment's rule and the listener's two"
    );
}

/// RlvObject::RlvObject: the issuer is looked up when its first rule is
/// stored, so one that was there and is gone at the first collection expires
/// at once; RlvHandler::onGC tells the listeners as a detach does.
#[test]
fn rlva_issuer_seen_when_its_first_rule_is_stored_expires_at_the_first_collection_it_misses() {
    let mut r = enabled();
    command(&mut r, A, "@fly=n");
    command(&mut r, B, "@jump=n");
    command(&mut r, TARGET, "@notify:123=n");
    r.resolve_sources(|id| id != B);
    r.out.clear();
    let now = std::time::Instant::now();
    r.collect(now, |id| id == TARGET);
    assert!(!r.has("fly"), "seen once, gone at the first collection");
    assert!(r.has("jump"), "never seen: twenty more attempts");
    assert_eq!(parity_replies(&mut r), [parity_reply(123, "/clear")]);
    // a new first rule is a new lookup
    command(&mut r, B, "@clear");
    command(&mut r, B, "@jump=n");
    r.resolve_sources(|_| true);
    r.collect(now + std::time::Duration::from_secs(30), |id| id == TARGET);
    assert!(!r.has("jump"));

    // through the world: the lookup is made when the OwnerSay is applied
    let mut world = parity_demo_world();
    let prim = parity_prim(&mut world, 0x7101, 0);
    let mut say = chat("@fly=n");
    say.source_id = prim;
    say.owner_id = world.agent_id;
    world.apply(aurora_net::NetEvent::Chat(say));
    assert!(world.rlva.has("fly"));
    let index = world.objects.index_of_uuid(&prim).unwrap();
    world.objects.remove_idx(index);
    let objects = &world.objects;
    world.rlva.collect(now, |id| objects.index_of_uuid(&id).is_some());
    assert!(!world.rlva.has("fly"), "rezzed, heard and gone before the first collection");
}

/// RlvCommand::asString: `@clear=<filter>` is "clear:<filter>" for the
/// listeners and their filters, and an empty option leaves no colon.
/// RlvHandler::processCommand: a command that is refused (disabled command,
/// blocked object) tells no one.
#[test]
fn rlva_notifications_write_clear_filters_with_a_colon_and_skip_refused_commands() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        "@notify:123;clear=n,notify:124;clear:fl=n,notify:125;clear:x=n,notify:126=n",
    );
    command(&mut r, B, "@fly=n,jump=n");
    r.out.clear();
    command(&mut r, B, "@clear=fly");
    assert!(!r.has("fly") && r.has("jump"));
    assert_eq!(
        parity_replies(&mut r),
        [
            parity_reply(123, "/clear:fly"),
            parity_reply(124, "/clear:fly"),
            parity_reply(126, "/clear:fly")
        ]
    );
    command(&mut r, B, "@jump:=add");
    assert_eq!(parity_replies(&mut r), [parity_reply(126, "/jump=add")]);
    command(&mut r, B, "@clear");
    assert_eq!(parity_replies(&mut r), [parity_reply(123, "/clear"), parity_reply(126, "/clear")]);
    r.preferences.allow_environment_locks = false;
    command(&mut r, B, "@setenv=n,setenv=y,fly:bad=n");
    assert_eq!(
        parity_replies(&mut r),
        [parity_reply(126, "/fly:bad=n")],
        "a failed command is still told, a disabled one is not"
    );

    let mut world = parity_demo_world();
    command(&mut world.rlva, A, "@notify:126=n");
    world.rlva_demo_experience(13, false);
    world.rlva_controllers_tick(std::time::Instant::now());
    world.rlva_demo_experience(13, true);
    let blocked = Uuid::from_u128(0x726c766165787072);
    assert!(world.rlva.controllers.blocked(blocked));
    world.rlva.out.clear();
    command(&mut world.rlva, blocked, "@fly=n,fly=y,clear");
    assert!(!world.rlva.has("fly"));
    assert_eq!(
        parity_replies(&mut world.rlva),
        [parity_reply(126, "/fly=y"), parity_reply(126, "/clear")],
        "a blocked object is heard only when it releases"
    );
}

/// LLViewerWindow::cursorIntersect drops the pick under @interact before
/// RlvActions::canTouch is asked: @touchme does not bring the touch back.
#[test]
fn rlva_touchme_overrides_touchall_but_not_interact() {
    let mut world = parity_demo_world();
    let region = world.main_region.unwrap();
    let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
    let own_local = world.objects.get(own).unwrap().key.local_id;
    let inviting = parity_prim(&mut world, 0x7201, 0);
    let plain = parity_prim(&mut world, 0x7202, 0);
    let hud_point = *world.avatar_lib.attach_points.iter().find(|(_, p)| p.hud).unwrap().0;
    let mut hud = crate::demo::outfit_attachment(&attach_request(Uuid::from_u128(0x7203), hud_point, true));
    hud.parent_id = own_local;
    let hud_local = hud.local_id;
    world.objects.upsert(region, hud);
    let touch = |world: &crate::world::World, id: Uuid| world.rlva_can_touch(region, parity_local(world, id));
    command(&mut world.rlva, inviting, "@touchme=n");
    command(&mut world.rlva, A, "@touchall=n");
    assert!(touch(&world, inviting), "@touchme lifts @touchall for its own object");
    assert!(!touch(&world, plain));
    command(&mut world.rlva, A, "@interact=n");
    assert!(!touch(&world, inviting), "@interact is decided before @touchme");
    assert!(!touch(&world, plain));
    assert!(world.rlva_can_touch(region, hud_local), "the HUDs stay");
    command(&mut world.rlva, A, "@interact=y");
    assert!(touch(&world, inviting));
}

/// RlvObject::getStatusString lists the restrictions in the order they were
/// issued; rlvParseGetStatusOption reads the filter and the separator only.
#[test]
fn rlva_status_lists_rules_in_issue_order_and_ignores_a_third_option_token() {
    let mut r = enabled();
    let status = |r: &mut Rlva, source: Uuid, query: &str| {
        r.out.clear();
        command(r, source, query);
        parity_replies(r).pop().map(|(_, message)| message).unwrap()
    };
    command(&mut r, A, "@unsit=n,fly=n,detach=n,alwaysrun=n,fly=n");
    assert_eq!(status(&mut r, A, "@getstatus=999"), "/unsit/fly/detach/alwaysrun");
    command(&mut r, A, "@fly=y,fly=n");
    assert_eq!(
        status(&mut r, A, "@getstatus=999"),
        "/unsit/detach/alwaysrun/fly",
        "issued again, listed last"
    );
    command(&mut r, B, "@tploc=n,jump=n");
    assert_eq!(
        status(&mut r, B, "@getstatusall=999"),
        "/unsit/detach/alwaysrun/fly/tploc/jump",
        "object after object, each in its own order"
    );
    assert_eq!(status(&mut r, B, "@getstatus=999"), "/tploc/jump");
    command(&mut r, A, "@clear=a");
    assert_eq!(status(&mut r, A, "@getstatus=999"), "/unsit/fly");
    assert_eq!(status(&mut r, A, "@getstatus:;|;x=999"), "|unsit|fly");
    assert_eq!(status(&mut r, A, "@getstatus:f;sep;x=999"), "sepfly");
    assert_eq!(
        status(&mut r, A, "@getstatus:;;x=999"),
        "/unsit/fly",
        "an empty separator is the default one"
    );
    assert_eq!(r.restrictions().len(), 4);
    command(&mut r, A, "@clear");
    command(&mut r, B, "@clear");
    assert_eq!(status(&mut r, A, "@getstatusall=999"), "");
    assert!(r.issued.is_empty() && r.rules.is_empty());
}

/// RlvCommandHandlerBaseImpl<RLV_TYPE_REPLY>::processCommand: a query that
/// fails on an option it does not take still answers, with nothing.
#[test]
fn rlva_camera_queries_with_an_unexpected_option_answer_with_nothing() {
    let r = enabled();
    for name in [
        "getcam_avdist",
        "getcam_avdistmin",
        "getcam_avdistmax",
        "getcam_fov",
        "getcam_fovmin",
        "getcam_fovmax",
        "getcam_textures",
    ] {
        let query = Command::parse(&format!("{name}:x=123")).unwrap();
        assert_eq!(r.camera_reply(&query, 5.0, 0.8).as_deref(), Some(""), "{name}");
    }
    let force = Command::parse("setcam_fov:1=force").unwrap();
    assert!(r.camera_reply(&force, 5.0, 0.8).is_none(), "not a query");
    let unknown = Command::parse("getcam_unknown:x=123").unwrap();
    assert!(r.camera_reply(&unknown, 5.0, 0.8).is_none());
}

/// RlvHandler::processReplyCommand reads no option for @getgroup and
/// @getsitid: with one they answer as without.
#[test]
fn rlva_getgroup_and_getsitid_ignore_their_option() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    command(&mut world.rlva, A, "@getgroup:x=123,getsitid:y=124,getgroup=125,getsitid=126");
    assert!(world.rlva_pending().is_empty());
    assert_eq!(
        parity_replies(&mut world.rlva),
        [
            parity_reply(123, "none"),
            parity_reply(124, &Uuid::nil().to_string()),
            parity_reply(125, "none"),
            parity_reply(126, &Uuid::nil().to_string())
        ]
    );
}

/// RlvCommandOptionHelper::parseOption<bool>: 0 and 1, then true and false.
#[test]
fn rlva_force_fly_reads_numbers_and_words() {
    for (text, value) in [
        ("0", Some(false)),
        ("1", Some(true)),
        ("01", Some(true)),
        ("+1", Some(true)),
        ("-0", Some(false)),
        ("1x", Some(true)),
        ("true", Some(true)),
        ("false", Some(false)),
        ("2", None),
        ("-1", None),
        ("99999999999", None),
        ("yes", None),
        ("", None),
    ] {
        assert_eq!(option_bool(text), value, "{text:?}");
    }
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    let fly = |world: &mut crate::world::World, source: Uuid, option: &str| {
        command(&mut world.rlva, source, &format!("@fly{option}=force"));
        assert!(world.rlva_pending().is_empty());
        world.agent.flying
    };
    assert!(fly(&mut world, A, ":1"));
    assert!(!fly(&mut world, A, ":0"));
    assert!(fly(&mut world, A, ":true"));
    assert!(!fly(&mut world, A, ":false"));
    assert!(fly(&mut world, A, ""));
    assert!(fly(&mut world, A, ":2"), "an invalid option changes nothing");
    assert!(!fly(&mut world, A, ":0"));
    command(&mut world.rlva, B, "@fly=n");
    assert!(!fly(&mut world, A, ":1"), "another object's @fly still forbids taking off");
    assert!(fly(&mut world, B, ":1"), "its own does not");
    assert!(!fly(&mut world, A, ":0"), "landing is always allowed");
}

/// RlvForceHandler<RLV_BHVR_SIT>: no option is the null key, a ground sit;
/// RlvForceHandler<RLV_BHVR_SITGROUND> reads no option at all.
#[test]
fn rlva_force_sit_without_a_target_and_sitground_with_an_option_sit_on_the_ground() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    for text in ["@sit=force", "@sitground:anything=force", "@sitground=force"] {
        command(&mut world.rlva, A, text);
        let sent = world.rlva_pending();
        assert!(
            matches!(
                sent.as_slice(),
                [(actor, NetCommand::OneShotControl(flags))] if *actor == A && *flags == aurora_net::control::SIT_ON_GROUND
            ),
            "{text}"
        );
    }
    command(&mut world.rlva, A, "@sit:not-a-key=force");
    assert!(world.rlva_pending().is_empty());
    command(&mut world.rlva, B, "@sit=n");
    for text in ["@sit=force", "@sitground:anything=force"] {
        command(&mut world.rlva, A, text);
        assert!(world.rlva_pending().is_empty(), "{text}: the same locks as @sitground=force");
    }
}

/// RlvBehaviourGenericHandler<RLV_OPTION_EXCEPTION>: without an option the
/// restriction is stored and listed, and names no one.
#[test]
fn rlva_named_restrictions_without_an_option_are_listed_and_name_no_one() {
    let mut r = enabled();
    command(
        &mut r,
        A,
        "@recvchatfrom=n,recvemotefrom=n,recvimfrom=n,startimto=n,touchthis=n,editobj=n,sendimto=n,getstatus=999",
    );
    assert_eq!(
        parity_replies(&mut r),
        [parity_reply(
            999,
            "/recvchatfrom/recvemotefrom/recvimfrom/startimto/touchthis/editobj/sendimto"
        )]
    );
    for text in ["hello", "/me waves"] {
        let mut heard = chat(text);
        heard.source_type = ChatSourceType::Agent;
        heard.chat_type = ChatType::Normal;
        heard.source_id = TARGET;
        assert!(r.receive_chat(&mut heard, Uuid::nil()));
        assert_eq!(heard.message, text);
    }
    for id in [TARGET, Uuid::nil()] {
        assert!(r.allows_im("recvim", id) && r.allows_im("startim", id) && r.allows_im("sendim", id));
        assert!(!r.targets("touchthis", &id.to_string()) && !r.targets("editobj", &id.to_string()));
    }
    assert!(r.rules_for("sendimto", Uuid::nil()).next().is_some());
    command(&mut r, A, &format!("@recvchatfrom:{TARGET}=n,touchthis=y,getstatus:touch=999"));
    assert_eq!(parity_replies(&mut r), [parity_reply(999, "")]);
    let mut heard = chat("hello");
    heard.source_type = ChatSourceType::Agent;
    heard.chat_type = ChatType::Normal;
    heard.source_id = TARGET;
    let shown = r.receive_chat(&mut heard, Uuid::nil());
    assert!(!shown || heard.message != "hello", "the named resident is still filtered");
}

/// RlvBehaviourInfo::processModifier: a modifier needs its behaviour held by
/// the same object, and leaves nothing behind when it is not.
#[test]
fn rlva_visual_modifiers_need_their_own_behaviour_held() {
    let mut r = enabled();
    let now = std::time::Instant::now();
    command(
        &mut r,
        A,
        "@fly=n,setoverlay_alpha:0.5=force,setoverlay_tint:0/0/0=force,setoverlay_texture:00000000-0000-0000-0000-000000000003=force,setsphere_mode:1=force",
    );
    assert!(r.visuals.overlays.is_empty() && r.visuals.spheres.is_empty());
    command(&mut r, A, "@setoverlay=n");
    assert_eq!(
        r.visuals.overlays[&A].color(now),
        [1.0; 4],
        "nothing set before @setoverlay is kept"
    );
    assert!(r.visuals.overlays[&A].texture.is_nil());
    command(&mut r, A, "@setsphere_mode:1=force");
    assert!(r.visuals.spheres.is_empty(), "@setoverlay does not open the sphere modifiers");
    command(&mut r, B, "@setsphere=n");
    command(&mut r, A, "@setsphere_mode:1=force");
    assert!(!r.visuals.spheres.contains_key(&A), "nor does another object's @setsphere");
    assert_eq!(r.visuals.spheres[&B].mode, 0);
    command(&mut r, A, "@setsphere=n,setsphere_mode:1=force,setoverlay_alpha:0.5=force");
    assert_eq!(r.visuals.spheres[&A].mode, 1);
    assert_eq!(r.visuals.overlays[&A].color(now + std::time::Duration::from_secs(1))[3], 0.5);
}

/// rlvParseNotifyOption refuses a third token; RlvBehaviourSendChannelHandler
/// stores a channel as a number.
#[test]
fn rlva_notify_refuses_a_third_token_and_channel_exceptions_are_numbers() {
    let mut r = enabled();
    command(&mut r, A, "@notify:123;a;b=n,notify:124;=n");
    let listeners: Vec<_> = r.rules_for("notify", Uuid::nil()).map(|(_, option)| option.to_owned()).collect();
    assert_eq!(listeners, ["124;"]);
    command(&mut r, A, "@clear");
    command(&mut r, A, "@sendchannel_except=n,sendchannel_except:07=n");
    assert!(!r.allows_channel(7), "07 is channel 7");
    assert!(r.allows_channel(8));
    command(&mut r, A, "@sendchannel=n,sendchannel:05=n,sendchannel:+9=n");
    assert!(r.allows_channel(5) && r.allows_channel(9));
    assert!(!r.allows_channel(6));
    r.out.clear();
    command(&mut r, A, "@getstatus::=999");
    assert_eq!(
        parity_replies(&mut r),
        [parity_reply(999, "/sendchannel_except:07/sendchannel:05/sendchannel:+9")],
        "listed as they were written"
    );
}

/// The seated demo world of the build-tool rules: the agent sits on the
/// linkset of prim 950, another object (99610) stands beside it. Returns
/// their indices.
fn build_escape_world() -> (crate::world::World, usize, usize) {
    use crate::world::World;
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events()
        .into_iter()
        .chain(crate::demo::selection_cycle_events())
        .chain(crate::demo::sit_events(250))
        .chain(crate::demo::sit_events(300))
    {
        world.apply(event);
    }
    world.agent.seated = true;
    world.rlva_configure(true);
    let index = |world: &World, id| world.objects.iter().find(|(_, o)| o.key.local_id == id).unwrap().0;
    let (seat, other) = (index(&world, 950), index(&world, 99610));
    (world, seat, other)
}

/// LLTaskInvFVBridge::isItemRemovable / isItemMovable, LLPanelContents::getState
/// and willObjectAcceptInventory: the contents of an attachment that cannot be
/// taken off, and of the seat under @unsit or @sittp, stay as they are; the
/// seat does not move either (selectGetEditMoveLinksetPermissions), and under
/// @unsit it stays linked (enableUnlinkObjects). The scripts of the locked
/// attachment are neither reset, stopped nor opened
/// (handle_selected_script_action, LLTaskLSLBridge::openItem). An item
/// already inside keeps its name and its properties, which the windows
/// decide (`rlva_delete_take_return_and_duplicate_follow_the_delete_or_return_rule`):
/// the update tried here is a drop, an item of our inventory.
#[test]
fn rlva_locked_attachment_and_restrained_seat_keep_their_contents_place_and_links() {
    use crate::world::World;
    use aurora_net::build::{BuildCmd, TaskItem, TransformUpdate};
    let (mut world, seat, other) = build_escape_world();
    let chest = worn_attachment(&mut world, 9101, 1);
    let worn = world
        .objects
        .iter()
        .find(|(_, o)| o.attachment_item_id() == Some(chest))
        .map(|(i, _)| i)
        .expect("worn");
    let contents = |world: &World, i: usize| -> Vec<NetCommand> {
        let o = world.objects.get(i).unwrap();
        let (handle, local_id, item_id) = (o.key.region, o.key.local_id, Uuid::from_u128(9102));
        let item = || {
            Box::new(TaskItem {
                // (an item of our inventory: `worn_attachment`)
                item_id: chest,
                parent_id: o.full_id,
                ..Default::default()
            })
        };
        [
            BuildCmd::RemoveTaskInventory { handle, local_id, item_id },
            BuildCmd::MoveTaskInventory {
                handle,
                local_id,
                item_id,
                folder_id: Uuid::nil(),
            },
            BuildCmd::UpdateTaskInventory {
                handle,
                local_id,
                item: item(),
            },
            BuildCmd::RezScript {
                handle,
                local_id,
                enabled: true,
                group_id: Uuid::nil(),
                item: item(),
            },
            BuildCmd::CreateTaskItem {
                handle,
                item: item(),
                tag: 1,
            },
        ]
        .into_iter()
        .map(NetCommand::Build)
        .collect()
    };
    let scripts = |world: &World, i: usize| -> Vec<NetCommand> {
        let o = world.objects.get(i).unwrap();
        let (handle, object_id, item_id) = (o.key.region, o.full_id, Uuid::from_u128(9102));
        vec![
            NetCommand::Build(BuildCmd::ScriptReset {
                handle,
                object_id,
                item_id,
            }),
            NetCommand::Build(BuildCmd::SetScriptRunning {
                handle,
                object_id,
                item_id,
                running: false,
            }),
        ]
    };
    let open = |world: &World, i: usize| -> Vec<NetCommand> {
        let o = world.objects.get(i).unwrap();
        vec![
            NetCommand::LoadScript {
                request: Uuid::from_u128(9103),
                target: aurora_net::scripts::ScriptTarget {
                    object: o.full_id,
                    item: Uuid::from_u128(9102),
                },
                handle: o.key.region,
                owner: Uuid::nil(),
                asset: Uuid::nil(),
            },
            NetCommand::PreviewTaskNotecard {
                handle: o.key.region,
                object: o.full_id,
                item: Box::default(),
            },
        ]
    };
    let transform = |world: &World, i: usize| {
        let key = world.objects.get(i).unwrap().key;
        vec![NetCommand::Build(BuildCmd::Transform {
            handle: key.region,
            updates: vec![TransformUpdate {
                local_id: key.local_id,
                position: Some(glam::Vec3::ONE),
                rotation: None,
                scale: None,
                linked: true,
                uniform: false,
            }],
        })]
    };
    let delink = |world: &World, i: usize| {
        let key = world.objects.get(i).unwrap().key;
        vec![NetCommand::Build(BuildCmd::Delink {
            handle: key.region,
            local_ids: vec![key.local_id],
        })]
    };
    let all = |world: &World, commands: Vec<NetCommand>| commands.iter().all(|c| world.rlva_allows_command(c, Uuid::nil()));
    let none = |world: &World, commands: Vec<NetCommand>| commands.iter().all(|c| !world.rlva_allows_command(c, Uuid::nil()));
    for i in [worn, seat, other] {
        assert!(all(&world, contents(&world, i)) && all(&world, scripts(&world, i)) && all(&world, open(&world, i)));
        assert!(
            all(&world, transform(&world, i)) && all(&world, delink(&world, i)),
            "no restriction yet"
        );
    }
    // an attachment that cannot be taken off
    command(&mut world.rlva, A, "@remattach:chest=n");
    assert!(world.rlva_locked_attachment(worn) && world.rlva_contents_locked(worn));
    assert!(none(&world, contents(&world, worn)) && none(&world, scripts(&world, worn)) && none(&world, open(&world, worn)));
    assert!(all(&world, transform(&world, worn)), "a locked attachment is still adjusted");
    for i in [seat, other] {
        assert!(!world.rlva_contents_locked(i));
        assert!(all(&world, contents(&world, i)) && all(&world, scripts(&world, i)) && all(&world, open(&world, i)));
    }
    command(&mut world.rlva, A, "@remattach:chest=y");
    assert!(all(&world, contents(&world, worn)) && all(&world, scripts(&world, worn)) && all(&world, open(&world, worn)));
    // the seat under @sittp: contents and place, not the links (RlvActions::canStand)
    command(&mut world.rlva, A, "@sittp=n");
    assert!(world.rlva_restrained_seat(seat) && world.rlva_contents_locked(seat) && !world.rlva_contents_locked(other));
    assert!(none(&world, contents(&world, seat)) && none(&world, transform(&world, seat)));
    assert!(
        all(&world, scripts(&world, seat)) && all(&world, open(&world, seat)),
        "only a locked attachment keeps its scripts as they are"
    );
    assert!(all(&world, delink(&world, seat)) && world.rlva_can_unlink(seat));
    assert!(all(&world, contents(&world, other)) && all(&world, transform(&world, other)));
    command(&mut world.rlva, A, "@sittp=y");
    // the seat under @unsit: the links too
    command(&mut world.rlva, A, "@unsit=n");
    assert!(none(&world, contents(&world, seat)) && none(&world, transform(&world, seat)) && none(&world, delink(&world, seat)));
    assert!(!world.rlva_can_unlink(seat) && world.rlva_can_unlink(other));
    assert!(all(&world, contents(&world, other)) && all(&world, transform(&world, other)) && all(&world, delink(&world, other)));
    // standing, the object is one like any other
    world.agent.seated = false;
    assert!(!world.rlva_restrained_seat(seat) && world.rlva_can_unlink(seat));
    assert!(all(&world, contents(&world, seat)) && all(&world, transform(&world, seat)) && all(&world, delink(&world, seat)));
}

/// LLToolSelect::handleObjectSelection (lltoolselect.cpp): what RLVa keeps
/// from edits is not selected by the build tools, whose click leaves the
/// selection as it is, and is selected by a right click for the time of its
/// menu (`temp_select`), with the build tools closed. The menu's Delete and
/// Take then follow rlvCanDeleteOrReturn, which asks nothing under
/// @editworld alone. LLSelectMgr::deselectAllIfTooFar keeps such a
/// selection while it is the menu's, and @interact drops it.
#[test]
fn rlva_a_right_click_selects_what_cannot_be_edited_for_its_menu_only() {
    use crate::build::{BuildTool, menu_selectable, rlva_keeps_menu_selection, rlva_selectable};
    let (mut world, _, other) = build_escape_world();
    let s = crate::build::BuildSettings::default();
    let mut tool = BuildTool::default();
    let key = world.objects.get(other).unwrap().key;
    tool.click_select(&mut world, &s, Some(other), false);
    assert_eq!(tool.selection, [key], "no restriction yet");
    tool.deselect_all(&world);
    command(&mut world.rlva, A, "@editworld=n");
    assert!(!world.rlva_can_edit(other) && !rlva_selectable(&world, other));
    tool.click_select(&mut world, &s, Some(other), false);
    assert!(tool.selection.is_empty(), "the build tools select nothing");
    // open build tools refuse the menu's selection until they are closed
    assert!(BuildTool::menu_only(&world, other) && menu_selectable(&world, other, false));
    tool.open = true;
    assert!(!tool.select_for_menu_near(&mut world, &s, other, false, false));
    tool.open = false;
    assert!(tool.select_for_menu_near(&mut world, &s, other, false, false));
    assert!(tool.menu_selection && tool.selection == [key]);
    assert!(rlva_keeps_menu_selection(&world, other), "kept while the menu is open");
    assert!(world.rlva_can_delete_or_return(other) && world.rlva_can_take_copy(other));
    assert!(
        world.rlva_allows_command(
            &NetCommand::Build(aurora_net::build::BuildCmd::Select {
                handle: key.region,
                local_ids: vec![key.local_id],
            }),
            Uuid::nil()
        ),
        "the temporary selection is sent"
    );
    // a build-tool click on it changes nothing of the selection either
    tool.click_select(&mut world, &s, Some(other), false);
    assert_eq!(tool.selection, [key]);
    // The menu closes on the entry clicked and releases its selection: the
    // entry takes it again for its action, where a build-tool click would
    // select nothing and leave Take and Delete without an object.
    let derez = |tool: &BuildTool, destination: u8| {
        tool.out.iter().any(|c| {
            matches!(
                c,
                NetCommand::Build(aurora_net::build::BuildCmd::DeRez { local_ids, destination: d, .. })
                    if *d == destination && *local_ids == [key.local_id]
            )
        })
    };
    for (copy, destination) in [
        (false, aurora_net::build::derez::TAKE_INTO_AGENT_INVENTORY),
        (true, aurora_net::build::derez::ACQUIRE_TO_AGENT_INVENTORY),
    ] {
        tool.release_menu_selection(&world);
        assert!(tool.selection.is_empty());
        tool.out.clear();
        assert!(tool.select_for_action(&mut world, &s, other));
        assert_eq!(tool.selection, [key]);
        tool.take(&mut world, copy);
        assert!(derez(&tool, destination), "copy {copy}: {:?}", tool.out);
        assert!(
            tool.out.iter().all(|c| world.rlva_allows_command(c, Uuid::nil())),
            "what is sent is allowed"
        );
    }
    tool.release_menu_selection(&world);
    tool.out.clear();
    assert!(tool.select_for_action(&mut world, &s, other));
    tool.delete(&mut world);
    assert!(derez(&tool, aurora_net::build::derez::TRASH), "{:?}", tool.out);
    assert!(tool.out.iter().all(|c| world.rlva_allows_command(c, Uuid::nil())));
    tool.release_menu_selection(&world);
    assert!(tool.select_for_menu_near(&mut world, &s, other, false, false));
    command(&mut world.rlva, A, "@interact=n");
    assert!(!menu_selectable(&world, other, true) && !rlva_keeps_menu_selection(&world, other));
}

/// object_selected_and_point_valid (llviewermenu.cpp): a rezzed object is
/// not worn under @rez, on its default point with an attachment lock of any
/// kind, nor on a point that takes no attachment. RlvActions::canBuild: the
/// build window needs editing or rezzing. LLViewerObject::allowOpen: "Open"
/// needs an object that may be edited.
#[test]
fn rlva_wearing_a_rezzed_object_opening_it_and_the_build_window_follow_their_locks() {
    use crate::world::World;
    use aurora_net::build::BuildCmd;
    let (mut world, _, other) = build_escape_world();
    let key = world.objects.get(other).unwrap().key;
    let allowed = |world: &World, point: u8| {
        world.rlva_allows_command(
            &NetCommand::Build(BuildCmd::Attach {
                handle: key.region,
                point,
                objects: vec![(key.local_id, glam::Quat::IDENTITY)],
            }),
            Uuid::nil(),
        )
    };
    // 0x80: ATTACHMENT_ADD, as "Add" and "Attach to" send their point
    assert!(allowed(&world, 0) && allowed(&world, 0x80) && allowed(&world, 0x80 | 1));
    command(&mut world.rlva, A, "@addattach:chest=n");
    assert!(
        !allowed(&world, 0) && !allowed(&world, 0x80),
        "the default point needs no lock at all"
    );
    assert!(!allowed(&world, 0x80 | 1) && allowed(&world, 0x80 | 2));
    assert!(!world.rlva_can_attach_object(0) && !world.rlva_can_attach_object(1) && world.rlva_can_attach_object(2));
    command(&mut world.rlva, A, "@addattach:chest=y");
    command(&mut world.rlva, A, "@remattach:skull=n");
    assert!(!allowed(&world, 0), "a remove lock counts for the default point");
    assert!(
        allowed(&world, 0x80 | 2),
        "a point locked against removal still takes an attachment"
    );
    command(&mut world.rlva, A, "@remattach:skull=y");
    assert!(allowed(&world, 0));
    command(&mut world.rlva, A, "@rez=n");
    assert!(
        !allowed(&world, 0) && !allowed(&world, 0x80 | 2),
        "attaching takes the object from the world"
    );
    assert!(world.rlva_can_build(), "@rez alone leaves editing");
    command(&mut world.rlva, A, "@edit=n");
    assert!(!world.rlva_can_build());
    command(&mut world.rlva, A, "@rez=y");
    assert!(world.rlva_can_build(), "@edit alone leaves rezzing");
    command(&mut world.rlva, A, "@edit=y");
    // "Open": our object with contents
    let flags = &mut world.objects.get_mut(other).unwrap().update_flags;
    *flags = (*flags & !(1 << 11)) | (1 << 2);
    assert!(crate::interaction::allow_open(&world, other));
    for restriction in ["edit", "editworld"] {
        command(&mut world.rlva, A, &format!("@{restriction}=n"));
        assert!(!crate::interaction::allow_open(&world, other), "{restriction}");
        command(&mut world.rlva, A, &format!("@{restriction}=y"));
    }
    let id = world.objects.get(other).unwrap().full_id;
    command(&mut world.rlva, A, &format!("@editobj:{id}=n"));
    assert!(!crate::interaction::allow_open(&world, other));
}

/// LLToolSelect::handleObjectSelection and LLSelectMgr::deselectAllIfTooFar:
/// under @fartouch the build tools select nothing beyond its distance, what
/// the agent wears aside, and the distance stands for the selection limit.
#[test]
fn rlva_fartouch_keeps_far_objects_out_of_the_build_selection() {
    let (mut world, _, other) = build_escape_world();
    let chest = worn_attachment(&mut world, 9104, 1);
    let worn = world
        .objects
        .iter()
        .find(|(_, o)| o.attachment_item_id() == Some(chest))
        .map(|(i, _)| i)
        .expect("worn");
    let (position, _, _) = crate::scene::Scene::object_transform_unshifted(&world, other, std::time::Instant::now()).expect("placed");
    let distance = position.distance(world.agent.position);
    assert!(distance > 0.01 && distance < 1000.0, "{distance}");
    assert!(world.rlva_select_limit().is_none() && world.rlva_within_fartouch(other));
    assert!(crate::build::selectable(&world, other));
    command(&mut world.rlva, A, "@fartouch:1000=n");
    assert_eq!(world.rlva_select_limit(), Some(1000.0));
    assert!(world.rlva_within_fartouch(other) && crate::build::selectable(&world, other));
    command(&mut world.rlva, B, "@fartouch:0.01=n");
    assert_eq!(world.rlva_select_limit(), Some(0.01), "the nearest limit holds");
    assert!(!world.rlva_within_fartouch(other) && !crate::build::selectable(&world, other));
    assert!(
        world.rlva_within_fartouch(worn),
        "our own attachments are in reach wherever they hang"
    );
    command(&mut world.rlva, B, "@fartouch:0.01=y");
    assert!(world.rlva_within_fartouch(other) && crate::build::selectable(&world, other));
}

/// render_hud_attachments (llviewerdisplay.cpp): while a HUD is locked the
/// HUD view zooms out to 85 % at most, and Aurora's own zoom of that HUD
/// does not shrink it; the saved size comes back with the release.
#[test]
fn rlva_locked_hud_bounds_the_hud_view_zoom_and_keeps_its_own_size() {
    use crate::world::hud_scale::{LOCKED_VIEW_MIN, MIN};
    let (mut world, _, _) = build_escape_world();
    let (hud, chest) = (worn_attachment(&mut world, 9105, 35), worn_attachment(&mut world, 9106, 1));
    // the demo attachment's object id is its item's
    world.hud_scales.set(hud, 0.5);
    world.hud_zoom = 0.2;
    let locked = world.rlva_has_locked_hud();
    world.rlva_limit_hud_zoom(locked);
    assert!(!locked);
    assert_eq!((world.hud_zoom, world.hud_scales.scale(hud), world.hud_zoom_min()), (0.2, 0.5, MIN));
    // a locked attachment that is no HUD changes nothing
    command(&mut world.rlva, A, "@remattach:chest=n");
    assert!(!world.rlva_can_remove(chest, Uuid::nil()) && !world.rlva_has_locked_hud());
    assert_eq!(world.hud_zoom_min(), MIN);
    command(&mut world.rlva, A, "@remattach=n");
    let locked = world.rlva_has_locked_hud();
    assert!(locked);
    world.rlva_limit_hud_zoom(locked);
    assert_eq!(
        (world.hud_zoom, world.hud_scales.scale(hud), world.hud_zoom_min()),
        (LOCKED_VIEW_MIN, 1.0, LOCKED_VIEW_MIN)
    );
    world.hud_zoom = f32::NAN;
    world.rlva_limit_hud_zoom(locked);
    assert_eq!(world.hud_zoom, LOCKED_VIEW_MIN, "a non-finite zoom is replaced");
    command(&mut world.rlva, A, "@remattach=y");
    command(&mut world.rlva, A, "@remattach:chest=y");
    let locked = world.rlva_has_locked_hud();
    world.rlva_limit_hud_zoom(locked);
    assert!(!locked);
    assert_eq!(world.hud_scales.scale(hud), 0.5, "the saved size is back");
}

/// LLAgent::setAFK / clearAFK (llagent.cpp): "Sit when away" asks for no sit
/// under @sit and no stand under @unsit, which the send would refuse.
#[test]
fn rlva_sit_when_away_asks_for_nothing_rlva_would_refuse() {
    use aurora_net::control;
    let (mut world, _, _) = build_escape_world();
    world.agent.seated = false;
    world.status.cfg.sit_when_away = true;
    let sits = |world: &mut crate::world::World| -> Vec<u32> {
        std::mem::take(&mut world.groups.out)
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::OneShotControl(flags) => Some(flags),
                _ => None,
            })
            .collect()
    };
    command(&mut world.rlva, A, "@sit=n");
    world.set_away(true);
    world.agent.ground_sit = true;
    world.set_away(false);
    assert!(sits(&mut world).is_empty(), "no sit under @sit, so no stand on the way back");
    world.agent.ground_sit = false;
    command(&mut world.rlva, A, "@sit=y");
    world.set_away(true);
    assert_eq!(sits(&mut world), vec![control::SIT_ON_GROUND]);
    world.agent.ground_sit = true;
    command(&mut world.rlva, A, "@unsit=n");
    world.set_away(false);
    assert!(sits(&mut world).is_empty(), "seated under @unsit");
}

/// LLAttachmentsMgr::onAttachmentArrived: a forced wear that has arrived is
/// no longer on its way. Taken off within the repeat window, the same object
/// is asked for again (seen on the grid: the second of two forced wears of
/// one object, a second apart with a detach in between, was dropped).
#[test]
fn rlva_forced_wear_is_asked_again_once_its_object_came_and_left() {
    use crate::world::appearance::{Action, WearRequest};
    use aurora_net::NetEvent;
    let mut world = parity_demo_world();
    let handle = world.main_region.unwrap();
    let item = Uuid::from_u128(0x7101);
    world.inventory.items.insert(
        item,
        aurora_net::inventory::InvItem {
            id: item,
            asset_type: 6,
            flags: 5,
            ..Default::default()
        },
    );
    let action = Action::Force {
        remove: Vec::new(),
        wear: vec![WearRequest {
            item,
            replace: true,
            point: 0,
        }],
    };
    let start = std::time::Instant::now();
    assert!(!world.rlva_force_settled(&action, start), "the first request goes out");
    assert!(world.rlva.requested_wears.contains_key(&item), "and is on its way");

    // it arrives, then is taken off
    worn_attachment(&mut world, 0x7101, 5);
    let local = parity_local(&world, item);
    world.apply(NetEvent::ObjectsKilled {
        handle,
        local_ids: vec![local],
    });
    assert!(world.objects.index_of_uuid(&item).is_none());
    assert!(
        !world.rlva.requested_wears.contains_key(&item),
        "it came and left: nothing is on its way"
    );
    assert!(
        !world.rlva_force_settled(&action, start + std::time::Duration::from_secs(1)),
        "the same object is asked for again"
    );
}

/// #2718: an object worn from the ground while something else is @detach
/// locked: its item is not in the inventory yet.
#[test]
fn rlva_detach_lock_keeps_an_object_worn_from_the_ground() {
    use aurora_net::AttachRequest;
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva.configure(true);
    let handle = world.main_region.expect("demo region");
    let attachment = |world: &crate::world::World, item_id: Uuid, point: u8| {
        crate::demo::outfit_attachment(&AttachRequest {
            item_id,
            owner_id: world.agent_id,
            point,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        })
    };
    let (locked, ground) = (Uuid::from_u128(0xE001), Uuid::from_u128(0xE002));
    let object = attachment(&world, locked, 2);
    world.objects.upsert(handle, object);
    let source = world
        .worn_attachment(&locked)
        .map(|i| world.objects.get(i).unwrap().full_id)
        .expect("worn");
    world.movement_complete = true;
    let tick = |world: &mut crate::world::World| {
        world.rlva_watchdog_tick(std::time::Instant::now());
        world.rlva_watchdog_tick(std::time::Instant::now());
    };
    tick(&mut world);
    command(&mut world.rlva, source, "@detach=n");
    tick(&mut world);

    // the object keeps its own id: only its item is new
    let mut object = attachment(&world, ground, 3);
    object.full_id = Uuid::from_u128(0xE102);
    world.objects.upsert(handle, object);
    assert!(!world.inventory.items.contains_key(&ground));
    tick(&mut world);
    assert!(
        world.rlva.watchdog.rejected.is_empty(),
        "the wearer's other lock does not refuse it"
    );
    assert!(world.worn_attachment(&ground).is_some());

    // a lock on the point itself still does
    let (held, refused) = (Uuid::from_u128(0xE003), 4);
    command(&mut world.rlva, source, "@detach:chest=n,addattach=n");
    let mut object = attachment(&world, held, refused);
    object.full_id = Uuid::from_u128(0xE103);
    world.objects.upsert(handle, object);
    tick(&mut world);
    assert!(world.rlva.watchdog.rejected.contains(&held), "@addattach still refuses it");
    assert!(
        !world.rlva_can_add(Uuid::from_u128(0xE00F), 3, Uuid::nil()),
        "an item neither known nor worn stays refused"
    );
}

#[test]
fn inventory_offer_receipts_preserve_the_share_restriction_and_its_resident_exception() {
    use aurora_net::NetCommand;
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva.configure(true);
    let (receipt, _) = crossbeam_channel::unbounded();
    let offer = |to| NetCommand::OfferInventory {
        session: Uuid::nil(),
        to,
        id: Uuid::nil(),
        message: String::new(),
        bucket: vec![],
        receipt: receipt.clone(),
    };
    command(&mut world.rlva, A, "@share=n");
    assert!(!world.rlva_allows_command(&offer(TARGET), Uuid::nil()));
    command(&mut world.rlva, A, &format!("@share:{TARGET}=add"));
    assert!(world.rlva_allows_command(&offer(TARGET), Uuid::nil()));
    assert!(!world.rlva_allows_command(&offer(B), Uuid::nil()));
}

#[test]
fn the_second_camera_pauses_while_rlva_holds_the_view() {
    for restriction in [
        "@setcam_unlock=n",
        "@camdistmax:2=n",
        "@camunlock=n",
        "@setsphere=n",
        "@setoverlay=n",
        "@camtextures=n",
        "@showself=n",
        "@showselfhead=n",
    ] {
        let mut r = enabled();
        assert!(!r.second_camera_blocked(), "{restriction}");
        command(&mut r, A, restriction);
        assert!(r.second_camera_blocked(), "{restriction}");
        command(&mut r, A, "@clear");
        assert!(!r.second_camera_blocked(), "{restriction} cleared");
    }
    // a restriction that does not touch the view leaves it alone
    let mut r = enabled();
    command(&mut r, A, "@tplm=n");
    assert!(!r.second_camera_blocked());
}
