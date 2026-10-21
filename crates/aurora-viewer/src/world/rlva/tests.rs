use super::*;

const A: Uuid = Uuid::from_u128(1);
const B: Uuid = Uuid::from_u128(2);
const TARGET: Uuid = Uuid::from_u128(3);

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
    assert!(!w.rlva_allows_command(&cmd, Uuid::nil()));
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    let aurora_net::NetCommand::UpdateOutfit { change, .. } = &cmd else {
        unreachable!()
    };
    assert_eq!(change.links.len(), 2, "preparation is idempotent");
    assert!(!w.rlva_can_remove(old, Uuid::nil()));
    assert!(!w.rlva_can_remove(old, watchdog::ACTOR));
    assert!(!w.rlva_can_add(new, 5, watchdog::ACTOR));
    let aurora_net::NetCommand::UpdateOutfit { change, .. } = &mut cmd else {
        unreachable!()
    };
    change.links.clear();
    w.rlva_prepare_outfit(&mut cmd, Uuid::nil());
    assert!(
        !w.rlva_allows_command(&cmd, Uuid::nil()),
        "explicit removal remains subject to locks"
    );
    w.rlva.configure(false);
    assert!(w.rlva_allows_command(&cmd, Uuid::nil()));
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
    assert!(world.rlva_allows_command(&detach, A));
    command(&mut world.rlva, A, "@detachme=force");
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
        "a forced move bypasses only its own source"
    );
    assert!(!world.rlva_allows_command(&move_to_other_point, Uuid::nil()));
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
fn rlva_strict_exceptions_need_every_restricting_object() {
    let mut r = enabled();
    command(&mut r, A, "@sendim_sec=n");
    command(&mut r, B, &format!("@sendim=n,sendim:{TARGET}=n"));
    assert!(!r.allows_im("sendim", TARGET));
    command(&mut r, A, &format!("@sendim:{TARGET}=n"));
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
    assert_eq!(r.camera_settings(&settings).camera_offset, [-4.0, 0.0, 1.0]);
    command(&mut r, A, "@clear");
    assert_eq!(r.camera_limit("setcam_avdistmin", 0.0, true), 10.0);
    r.configure(false);
    assert_eq!(r.camera_settings(&settings), settings);
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
    assert!(w.rlva_can_add(TARGET, 0, A));
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
    assert!(w.rlva_can_remove(Uuid::from_u128(20), B));
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
    assert!(!world.rlva_can_touch_at(key.region, key.local_id, Some(world.agent.position + glam::Vec3::X * 2.0)));
    world.rlva.configure(false);
    assert!(world.rlva_can_touch_at(key.region, key.local_id, Some(world.agent.position + glam::Vec3::X * 100.0)));
}
