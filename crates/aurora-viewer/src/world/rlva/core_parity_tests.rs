//! The command core against the Legacy Viewer: parsing, the dictionary, the
//! login, the order of the replies and the debug output (RlvCommand,
//! RlvBehaviourDictionary and RlvHandler, indra/newview/rlvhelper.cpp and
//! rlvhandler.cpp; process_chat_from_simulator, llviewermessage.cpp).
use super::*;
use crate::world::World;
use std::time::{Duration, Instant};

const A: Uuid = Uuid::from_u128(0xc0_01);
const B: Uuid = Uuid::from_u128(0xc0_02);
const TARGET: Uuid = Uuid::from_u128(0xc0_03);

fn owner_say(source: Uuid, text: &str) -> ChatMessage {
    ChatMessage {
        from_name: "Core parity object".to_owned(),
        source_id: source,
        owner_id: TARGET,
        source_type: ChatSourceType::Object,
        chat_type: ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: text.to_owned(),
    }
}

fn enabled() -> Rlva {
    let mut r = Rlva::default();
    r.configure(true);
    r
}

fn say(r: &mut Rlva, source: Uuid, text: &str) {
    assert!(r.receive(&owner_say(source, text)));
}

fn replies(r: &mut Rlva) -> Vec<(i32, String)> {
    std::mem::take(&mut r.out)
        .into_iter()
        .filter_map(|c| match c {
            NetCommand::ChatReply { channel, message } => Some((channel, message)),
            _ => None,
        })
        .collect()
}

fn reply(channel: i32, message: &str) -> (i32, String) {
    (channel, message.to_owned())
}

fn world() -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.rlva_configure(true);
    world
}

/// process_chat_from_simulator returns for a blocked object, or an object of
/// a blocked owner, before its commands reach RlvHandler.
#[test]
fn core_g1_a_blocked_object_or_owner_issues_no_command() {
    use crate::world::mutes::MuteType;
    let mut world = world();
    world.block(A, "Core parity object", MuteType::Object).expect("blocked");
    world.apply(aurora_net::NetEvent::Chat(owner_say(A, "@fly=n,version=123")));
    assert!(!world.rlva.has("fly") && world.rlva.out.is_empty());
    let mut other = owner_say(B, "@jump=n");
    other.owner_id = Uuid::from_u128(0xc0_99);
    world.block(other.owner_id, "Blocked Owner", MuteType::Agent).expect("blocked");
    world.apply(aurora_net::NetEvent::Chat(other));
    assert!(!world.rlva.has("jump"));
    world.apply(aurora_net::NetEvent::Chat(owner_say(TARGET, "@sit=n")));
    assert!(world.rlva.has("sit"), "an object nobody blocked still issues commands");
}

/// RlvCommand::RlvCommand reads the reply channel with
/// LLStringUtil::convertToS32: spaces around it and what follows the number
/// do not make it invalid.
#[test]
fn core_g2_reply_channel_trailing_text_and_spaces_still_answer() {
    let mut r = enabled();
    say(&mut r, A, "@versionnum=123 ,versionnum= 124,versionnum=125abc,versionnum=abc");
    assert_eq!(
        replies(&mut r),
        [reply(123, "3040300"), reply(124, "3040300"), reply(125, "3040300")]
    );
    say(&mut r, A, "@notify:5abc=add");
    replies(&mut r);
    say(&mut r, A, "@fly=n");
    assert_eq!(replies(&mut r), [reply(5, "/fly=n")], "@notify reads its channel the same way");
}

/// RlvObject::addCommand and RlvHandler keep every command: no cap drops a
/// restriction or a query of a busy script.
#[test]
fn core_g3_hundreds_of_rules_and_queries_are_all_kept() {
    let mut r = enabled();
    let rules: Vec<String> = (0..600)
        .map(|i| format!("recvchatfrom:{}=n", Uuid::from_u128(0x1000 + i)))
        .collect();
    for chunk in rules.chunks(20) {
        say(&mut r, A, &format!("@{}", chunk.join(",")));
    }
    assert_eq!(r.rules_for("recvchatfrom", Uuid::nil()).count(), 600);
    for _ in 0..30 {
        say(&mut r, A, &format!("@{}", vec!["getinv=7"; 20].join(",")));
    }
    assert_eq!(r.pending.len(), 600, "every query waits for its answer");
}

/// RlvBehaviourDictionary::getCommands: the registration order, a name
/// registered for several types once per type, the "_sec" name after a
/// strict one, dictionary entries only; RlvReplyHandler<GETCOMMAND> splits
/// its option on every ';'.
#[test]
fn core_g6_getcommand_lists_the_dictionary_in_order_with_its_duplicates() {
    assert_eq!(command_list("fly"), "fly;fly");
    assert_eq!(command_list("fly;add"), "fly");
    assert_eq!(command_list("fly;force"), "fly");
    assert_eq!(
        command_list("detach;;|"),
        "detach|detachthis|detachallthis|detachthis_except|detachallthis_except|detach|detachall|detachthis|detachallthis|detachme"
    );
    assert_eq!(command_list("sit;any;;"), "sitsittpunsitsitsitgroundunsitgetsitid");
    assert_eq!(command_list("sendim"), "sendim;sendim_sec;sendimto;sendimto_sec");
    assert_eq!(command_list("version;reply"), "version;versionnew;versionnum");
    assert_eq!(command_list("fly;bogus"), "", "an unknown type fails, with an empty reply");
    for absent in [
        "setenv_",
        "getenv_",
        "setdebug_",
        "getdebug_",
        "setrot",
        "setsphere_",
        "setoverlay_alpha",
    ] {
        assert!(!command_list(absent).split(';').any(|name| name.starts_with(absent)), "{absent}");
    }
    let all = command_list("");
    assert!(all.starts_with("acceptpermission;accepttp;accepttp_sec;accepttprequest;accepttprequest_sec;addattach;"));
    assert!(all.ends_with(";getsitid;getstatus;getstatusall;version;versionnew;versionnum"));
    let preferences = preferences::Preferences {
        experimental_commands: false,
        ..Default::default()
    };
    assert_eq!(command_list_with("setcam_fov", &preferences), "setcam_fovmin;setcam_fovmax");
    let mut r = enabled();
    say(&mut r, A, "@getcommand:showself=5,getcommand:camzoom;add;|=6");
    assert_eq!(
        replies(&mut r),
        [reply(5, "showself;showselfhead"), reply(6, "camzoommin|camzoommax")]
    );
}

/// RlvSettings::initClass and RlvBehaviourDictionary's constructor read
/// these settings once, when RLVa starts at the login.
#[test]
fn core_g7_read_once_settings_keep_their_value_from_the_start_of_rlva() {
    let mut r = enabled();
    let mut live = preferences::Preferences::default();
    r.apply_preferences(&live, startup::Phase::Loading);
    live.allow_environment_locks = false;
    live.allow_ooc = false;
    live.experimental_commands = false;
    live.experience_maturity = 3;
    live.blocked_experiences = "x".into();
    live.compatibility_list = "name:x".into();
    live.show_ellipsis = false;
    r.apply_preferences(&live, startup::Phase::Started);
    let p = &r.preferences;
    assert!(p.allow_environment_locks && p.allow_ooc && p.experimental_commands);
    assert_eq!((p.experience_maturity, p.compatibility_list.as_str()), (2, ""));
    assert_eq!(p.blocked_experiences, preferences::Preferences::default().blocked_experiences);
    assert!(!p.show_ellipsis, "the other settings follow at once");
    say(&mut r, A, "@setenv=n");
    assert!(r.has("setenv"));
    say(&mut r, A, "@setenv=y");
    assert!(!r.has("setenv"), "a lock set before the change can still be released");
    r.apply_preferences(&live, startup::Phase::Login);
    assert!(
        !r.preferences.allow_environment_locks,
        "on the login screen, the settings apply as set"
    );
    r.apply_preferences(&preferences::Preferences::default(), startup::Phase::Loading);
    r.configure(false);
    r.configure(true);
    r.apply_preferences(&live, startup::Phase::Started);
    assert!(!r.preferences.allow_ooc, "RLVa turned on again reads them again");
}

/// RlvHandler::processCommand retains every command before STATE_STARTED;
/// onIdleStartup answers @version* every 2 s during the login, and
/// onLoginComplete runs the others in order.
#[test]
fn core_g4_commands_before_login_completes_are_retained_then_run_in_order() {
    let mut world = world();
    let live = world.rlva.preferences.clone();
    let start = Instant::now();
    world.rlva_startup(startup::Phase::Loading, &live, start);
    world.apply(aurora_net::NetEvent::Chat(owner_say(
        A,
        "@fly=n,versionnum=5,getstatus=6,clear=fly,jump=n",
    )));
    assert!(!world.rlva.has("fly") && world.rlva.out.is_empty(), "nothing runs during the login");
    world.rlva_startup(startup::Phase::Loading, &live, start + Duration::from_millis(1500));
    assert!(world.rlva.out.is_empty());
    world.rlva_startup(startup::Phase::Loading, &live, start + Duration::from_secs(2));
    assert_eq!(replies(&mut world.rlva), [reply(5, "3040300")], "only the version is answered");
    assert!(!world.rlva.has("fly"));
    world.rlva_startup(startup::Phase::Started, &live, start + Duration::from_secs(3));
    assert!(!world.rlva.has("fly") && world.rlva.has("jump"), "the rest ran in order");
    assert_eq!(replies(&mut world.rlva), [reply(6, "/fly")]);
    world.apply(aurora_net::NetEvent::Chat(owner_say(A, "@sit=n")));
    assert!(world.rlva.has("sit"), "once started, a command runs at once");
}

/// Each query is answered as RlvHandler processes it: a reply from RLVa's
/// own state comes after the queued query before it.
#[test]
fn core_g5_replies_keep_the_order_of_their_commands() {
    let mut world = world();
    world.apply(aurora_net::NetEvent::Chat(owner_say(
        A,
        "@getgroup=5,versionnum=5,getsitid=5,getstatus=5",
    )));
    assert!(world.rlva.out.is_empty(), "the version waits behind @getgroup");
    world.rlva_pending();
    let order: Vec<_> = replies(&mut world.rlva).into_iter().map(|(_, text)| text).collect();
    assert_eq!(order.len(), 4);
    assert_eq!(order[0], "none");
    assert_eq!(order[1], "3040300");
    assert_eq!(order[3], "");
    let mut r = enabled();
    say(&mut r, A, "@versionnum=5,getstatus=5");
    assert_eq!(replies(&mut r).len(), 2, "with nothing queued, the answers are immediate");
}

/// RlvCommand accepts any behaviour text: an unknown add still reaches
/// RlvBehaviourNotifyHandler, as "/ fly=n" or "/foo1=n".
#[test]
fn core_g9_unknown_names_are_still_notified() {
    let mut r = enabled();
    say(&mut r, A, "@notify:5=n");
    replies(&mut r);
    say(&mut r, A, "@detach=n, fly=n,foo1=n");
    assert_eq!(replies(&mut r), [reply(5, "/detach=n"), reply(5, "/ fly=n"), reply(5, "/foo1=n")]);
    assert!(!r.has("fly"));
}

/// parseOption<LLUUID> is LLUUID::set: the empty text (the null key) and
/// the 36 or 35 character forms only.
#[test]
fn core_g10_uuid_options_take_the_legacy_forms_only() {
    let mut r = enabled();
    let id = TARGET.to_string();
    say(&mut r, A, &format!("@recvchatfrom:{}=n", TARGET.simple()));
    say(&mut r, A, &format!("@recvchatfrom:{{{id}}}=n"));
    let held = |r: &Rlva, name: &str| r.rules_for(name, Uuid::nil()).count();
    assert_eq!(held(&r, "recvchatfrom"), 0, "the 32-hex and braced forms are invalid options");
    say(&mut r, A, &format!("@recvchatfrom:{id}=n"));
    assert_eq!(held(&r, "recvchatfrom"), 1);
    say(&mut r, A, "@recvchat:=n");
    assert!(r.has("recvchat"));
    let short = format!("{}{}", &id[..23], &id[24..]);
    say(&mut r, A, &format!("@sendimto:{short}=n"));
    assert_eq!(held(&r, "sendimto"), 1, "the form without its last dash");
    let cut: String = id.chars().take(35).collect();
    say(&mut r, A, &format!("@recvimfrom:{cut}=n"));
    assert_eq!(held(&r, "recvimfrom"), 0, "35 characters with the last dash read a wrong digit");
}

/// RlvBehaviourHandler<RLV_BHVR_SETOVERLAY_TOUCH>::onCommand
/// (rlvhandler.cpp) never reads its option: with one, the restriction is
/// still held and still blocks the touch through the overlay.
#[test]
fn core_setoverlay_touch_takes_any_option() {
    let mut r = enabled();
    say(&mut r, A, "@setoverlay_touch:anything=n");
    say(&mut r, B, "@setoverlay_touch=n");
    let held = |r: &Rlva| {
        r.rules_for("setoverlay_touch", Uuid::nil())
            .map(|(source, _)| source)
            .collect::<Vec<_>>()
    };
    let mut sources = held(&r);
    sources.sort();
    assert_eq!(sources, [A, B]);
    say(&mut r, A, "@setoverlay_touch:anything=y");
    assert_eq!(held(&r), [B]);
}

/// RlvExtGetSet: convertToBOOL takes "t" and "f", convertToU32 wraps a
/// minus, and no "default" exists; @getdebug_<name>_sec asks for a setting
/// of that name, which has none.
#[test]
fn core_g11_setdebug_reads_values_as_the_legacy_converters() {
    let mut r = enabled();
    let force = |text: &str| Command::parse(text).unwrap();
    r.debug_force(A, &force("setdebug_avatarsex: t =force"));
    assert_eq!(r.pseudo_sex.as_deref(), Some(" t "));
    r.debug_force(A, &force("setdebug_renderresolutiondivisor:2.5=force"));
    assert_eq!(r.resolution_divisor, Some(2));
    r.debug_force(A, &force("setdebug_renderresolutiondivisor:-1=force"));
    assert_eq!(r.resolution_divisor, Some(u32::MAX));
    r.debug_force(A, &force("setdebug_renderresolutiondivisor:default=force"));
    assert_eq!(r.resolution_divisor, Some(u32::MAX), "no default value");
    let parsed = force("getdebug_avatarsex_sec=5");
    assert!(!parsed.strict && parsed.behaviour == "getdebug_avatarsex_sec");
    say(&mut r, A, "@getdebug_=5");
    assert!(r.viewer.is_empty() && r.pending.is_empty(), "no setting has an empty name");
}

/// RlvFloaterConsole::onClose runs @clear for the agent: every @notify
/// listener hears "/clear".
#[test]
fn core_g12_closing_the_console_notifies_the_clear() {
    let mut world = world();
    world.agent_id = Uuid::from_u128(0xc0_a9);
    let agent = world.agent_id;
    say(&mut world.rlva, A, "@notify:5=n");
    world.rlva_console_input("@fly=n");
    replies(&mut world.rlva);
    world.rlva.close_console(agent);
    assert!(!world.rlva.has_except("fly", A));
    assert_eq!(replies(&mut world.rlva), [reply(5, "/clear")]);
}

/// RlvSettings::isCompatibilityModeObject asks the issuing prim: a child
/// prim of a listed attachment is not an attachment itself.
#[test]
fn core_g13_a_child_prim_of_a_compatibility_attachment_gets_the_current_version() {
    use aurora_net::inventory::InvItem;
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    let item = Uuid::from_u128(0xc013);
    let creator = Uuid::from_u128(0xc014);
    world.inventory.items.insert(
        item,
        InvItem {
            id: item,
            creator,
            name: "Old controller".into(),
            asset_type: 6,
            ..Default::default()
        },
    );
    let root = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
        item_id: item,
        owner_id: world.agent_id,
        point: 1,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: "Old controller".into(),
        desc: String::new(),
    });
    let mut child = root.clone();
    child.full_id = Uuid::from_u128(0xc015);
    child.local_id = root.local_id + 1;
    child.parent_id = root.local_id;
    child.state = 0;
    child.name_values.clear();
    let region = world.main_region.unwrap();
    world.objects.upsert(region, root);
    world.objects.upsert(region, child.clone());
    world.rlva.preferences.compatibility_list = format!("creator:{creator}");
    for (source, channel) in [(item, 5), (child.full_id, 6)] {
        let mut chat = owner_say(source, &format!("@versionnum={channel}"));
        chat.owner_id = world.agent_id;
        world.apply(aurora_net::NetEvent::Chat(chat));
    }
    assert_eq!(replies(&mut world.rlva), [reply(5, "2092800"), reply(6, "3040300")]);
}

/// process_chat_from_simulator's RestrainedLoveDebug line, with
/// RlvStrings::getStringFromReturnCode's suffixes; @clear is a success.
#[test]
fn core_g14_debug_line_lists_what_ran_failed_or_was_retained() {
    let mut r = enabled();
    r.preferences.debug_commands = true;
    say(&mut r, A, "@fly=n");
    assert_eq!(r.take_debug_line().as_deref(), Some(" executes: @fly=n"));
    say(&mut r, A, "@fly=n,jump=y,clear=nothing");
    assert_eq!(
        r.take_debug_line().as_deref(),
        Some(" executes: @fly=n (duplicate),jump=y (unset),clear=nothing")
    );
    say(&mut r, A, "@foo=n,fly:x=n,getinv=0,bar=force,bad");
    assert_eq!(
        r.take_debug_line().as_deref(),
        Some(
            " failed: @foo=n (invalid param),fly:x=n (invalid option),getinv=0 (invalid param),bar=force (unknown command),bad (thingy error)"
        )
    );
    say(&mut r, B, "@setcam=n");
    say(&mut r, A, "@sit=n,setcam=n");
    assert_eq!(
        r.take_debug_line().as_deref(),
        Some(": @sit=n,setcam=n\n    - executed: @sit=n\n    - failed: @setcam=n (locked command)")
    );
    r.preferences.debug_hide_unset_duplicate = true;
    say(&mut r, A, "@sit=n");
    assert_eq!(r.take_debug_line(), None);
    r.preferences.debug_commands = false;
    say(&mut r, A, "@unsit=n");
    assert_eq!(r.take_debug_line(), None);

    let mut world = world();
    world.rlva.preferences.debug_commands = true;
    let live = world.rlva.preferences.clone();
    world.rlva_startup(startup::Phase::Loading, &live, Instant::now());
    let mut chat = owner_say(A, "@fly=n");
    chat.owner_id = world.agent_id;
    world.apply(aurora_net::NetEvent::Chat(chat));
    let line = world.chat.back().map(|line| line.text.clone()).unwrap_or_default();
    assert!(line.ends_with(" retained: @fly=n"), "{line}");
}

/// Option parsing at the use sites: the shared converters of `numeric`.
#[test]
fn core_numeric_options_follow_the_legacy_converters_at_their_use_sites() {
    let mut r = enabled();
    say(&mut r, A, "@shownametags:-5=n");
    assert!(r.nametag_visible(TARGET, B, 4.0), "a negative distance works as its opposite");
    assert!(!r.nametag_visible(TARGET, B, 6.0));
    assert_eq!(im_range("5;"), None, "a ';' needs a maximum");
    assert_eq!(im_range("5abc;10"), Some((5.0, 10.0)));
    assert!(im_range("nan").is_some());
    say(&mut r, A, "@setcam_focusoffset:1/2/3=n");
    assert!(!r.has("setcam_focusoffset"), "convertOptionValue has no LLVector3d");
    say(&mut r, A, "@sendchannel:5x=n,camzoommin:inf=n");
    assert_eq!(
        r.rules_for("sendchannel", Uuid::nil()).count() + r.rules_for("camzoommin", Uuid::nil()).count(),
        2
    );
    assert!(matches!(
        policy::teleport("Region/1/2/3junk;1.5"),
        Some(NetCommand::TeleportToRegion { position, .. }) if position == glam::Vec3::new(1.0, 2.0, 3.0)
    ));
    assert!(policy::teleport("256.5/512/20").is_some());
    assert!(policy::teleport("Region/1/2").is_none() && policy::teleport("inf/0/0").is_none());
}

/// RlvBehaviourNotifyHandler sends its line as RlvHandler processes the
/// command, after the answer to the query before it: the line waits behind
/// a queued query, and the filters of that moment decide who hears it.
#[test]
fn core_g5_notify_lines_wait_behind_the_queued_replies() {
    let mut world = world();
    world.apply(aurora_net::NetEvent::Chat(owner_say(B, "@notify:6=n")));
    replies(&mut world.rlva);
    world.apply(aurora_net::NetEvent::Chat(owner_say(A, "@getgroup=5,fly=n,versionnum=5")));
    assert!(world.rlva.out.is_empty(), "the notification waits behind @getgroup");
    world.apply(aurora_net::NetEvent::Chat(owner_say(B, "@notify:6=y")));
    world.rlva_pending();
    assert_eq!(
        replies(&mut world.rlva),
        [reply(5, "none"), reply(6, "/fly=n"), reply(5, "3040300")],
        "in the order of the commands, to the listener of that moment"
    );
    let mut r = enabled();
    say(&mut r, B, "@notify:6=n");
    say(&mut r, A, "@fly=n");
    assert_eq!(
        replies(&mut r)[1..],
        [reply(6, "/fly=n")],
        "with nothing queued, the line is immediate"
    );
}
