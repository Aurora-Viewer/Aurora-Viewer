use super::*;

const A: Uuid = Uuid::from_u128(1);
const B: Uuid = Uuid::from_u128(2);
const TARGET: Uuid = Uuid::from_u128(3);

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
    assert_eq!(replies[1].0, "");
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
    assert_eq!(w.rlva_inventory_query(&query).as_deref(), Some("Test outfit"));
    query.option = "../outside".to_owned();
    assert_eq!(w.rlva_inventory_query(&query).as_deref(), Some(""));
    query = Command::parse("getinvworn=123").expect("query");
    assert_eq!(w.rlva_inventory_query(&query).as_deref(), Some("|01,Test outfit|10"));
    w.inventory.folders.get_mut(&child).expect("folder").state = FetchState::Unknown;
    assert!(w.rlva_inventory_query(&query).is_none(), "unloaded is not empty");
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
