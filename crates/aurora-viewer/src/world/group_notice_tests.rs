//! Group notices in the notification center: titled by their group, their
//! sender first in the text, their attachment saved or declined from the
//! card (LLToastGroupNotifyPanel); their line in the group's chat or
//! transcript (LLHandlerUtil::logGroupNoticeToIMGroup).

use super::notifications::{Data, NameTarget, Response, im as d};
use super::*;

const GROUP: Uuid = Uuid::from_u128(0x6E);
const SENDER: Uuid = Uuid::from_u128(0x5E);
const TX: Uuid = Uuid::from_u128(0x7A);

fn world() -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(9);
    world.groups.groups.push(aurora_net::GroupMembership {
        id: GROUP,
        name: "Northern Wolves".into(),
        insignia: Uuid::from_u128(0x1C),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    world
}

fn notice(attachment: Option<(&str, u8)>) -> InstantMessage {
    let mut bucket = vec![u8::from(attachment.is_some()), attachment.map_or(0, |a| a.1)];
    bucket.extend_from_slice(GROUP.as_bytes());
    bucket.extend_from_slice(attachment.map_or("", |a| a.0).as_bytes());
    bucket.push(0);
    InstantMessage {
        from_agent_id: SENDER,
        from_name: "Violet Wolf".into(),
        to_agent_id: Uuid::from_u128(9),
        dialog: d::GROUP_NOTICE,
        session_id: TX,
        message: "Workshop|Bring the kit.".into(),
        offline: false,
        from_group: false,
        binary_bucket: bucket,
        timestamp: 0,
    }
}

fn answer(cmds: &[aurora_net::NetCommand]) -> Option<(u8, Vec<u8>)> {
    cmds.iter().find_map(|c| match c {
        aurora_net::NetCommand::SendImDialog {
            to, dialog, id, bucket, ..
        } if *to == SENDER && *id == TX => Some((*dialog, bucket.clone())),
        _ => None,
    })
}

#[test]
fn a_notice_is_titled_by_its_group_and_names_its_sender() {
    let mut world = world();
    world.apply(NetEvent::InstantMessage(notice(None)));
    let n = world.notifications.list.last().expect("notice listed");
    assert_eq!(n.title, "Northern Wolves : Workshop");
    let sent_by = aurora_i18n::tr!("world-mod-group-notice-sent-by", sender = "Violet Wolf".to_string());
    assert_eq!(n.body, format!("{sent_by}\nBring the kit."));
    assert!(matches!(
        n.data,
        Data::GroupNotice {
            group: GROUP,
            attachment: None
        }
    ));
    // without an attachment it is information
    assert!(!n.interactive() && !n.close_declines());
    assert!(n.name_links.iter().any(|l| l.target == NameTarget::Group(GROUP)));
    assert!(n.name_links.iter().any(|l| l.target == NameTarget::Avatar(SENDER)));
}

#[test]
fn an_attachment_is_saved_from_the_card_and_the_notice_stays() {
    let mut world = world();
    world.apply(NetEvent::InstantMessage(notice(Some(("Texture kit", 6)))));
    let n = world.notifications.list.last().expect("notice listed");
    let id = n.id;
    let a = n.offered_attachment().expect("attachment offered");
    assert_eq!((a.name.as_str(), a.asset_type), ("Texture kit", 6));
    assert!(n.interactive() && n.close_declines());
    let (cmds, _) = world.respond_notification(id, Response::Accept);
    // IM_GROUP_NOTICE_INVENTORY_ACCEPTED with the destination folder
    let (dialog, bucket) = answer(&cmds).expect("accepted");
    assert_eq!((dialog, bucket.len()), (33, 16));
    let n = world.notifications.list.iter().find(|n| n.id == id).expect("notice kept");
    assert!(matches!(&n.data, Data::GroupNotice { attachment: Some(a), .. } if a.saved));
    // now information: closing it answers nothing more
    assert!(!n.interactive() && !n.close_declines());
    let (cmds, _) = world.respond_notification(id, Response::Accept);
    assert!(answer(&cmds).is_none(), "saved once");
    let (cmds, _) = world.respond_notification(id, Response::Dismiss);
    assert!(answer(&cmds).is_none());
}

#[test]
fn closing_a_notice_declines_its_attachment() {
    let mut world = world();
    world.apply(NetEvent::InstantMessage(notice(Some(("Rules", 7)))));
    let id = world.notifications.list.last().expect("notice listed").id;
    let (cmds, _) = world.respond_notification(id, Response::Dismiss);
    // IM_GROUP_NOTICE_INVENTORY_DECLINED
    assert_eq!(answer(&cmds).map(|a| a.0), Some(34));
    assert!(world.notifications.list.iter().all(|n| n.id != id));
}

#[test]
fn a_notice_without_a_readable_bucket_still_shows_by_its_sender() {
    let mut world = world();
    let mut im = notice(None);
    im.binary_bucket = vec![0];
    world.apply(NetEvent::InstantMessage(im));
    let n = world.notifications.list.last().expect("notice listed");
    assert_eq!(n.title, "Violet Wolf : Workshop");
    assert!(matches!(n.data, Data::None));
}

// The notice's line in its group's chat (FSGroupNoticesToIMLog,
// LLHandlerUtil::logGroupNoticeToIMGroup).

fn notice_lines(world: &World, session: Uuid) -> Vec<String> {
    world
        .social
        .ims
        .iter()
        .find(|s| s.other == session)
        .map(|s| {
            s.lines
                .iter()
                .filter(|l| l.kind == ChatKind::System)
                .map(|l| l.text.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_chat_line_has_the_legacy_formats() {
    // the Legacy Viewer's "%s %s: %s" of GroupNotifyGroupNotice and GroupNotifySentBy
    assert_eq!(
        aurora_i18n::tr_in!("en", "world-groups-notice-in-chat", sender = "Violet Wolf".to_string()),
        "Group Notice Sent by: Violet Wolf"
    );
    let head = aurora_i18n::tr!("world-groups-notice-in-chat", sender = "Violet Wolf".to_string());
    assert_eq!(
        groups::notice_chat_text("Violet Wolf", "Workshop", "Bring the kit.", true).as_deref(),
        Some(format!("{head}\nWorkshop\nBring the kit.").as_str())
    );
    // without subjects and authors: the text alone
    assert_eq!(
        groups::notice_chat_text("Violet Wolf", "Workshop", "Bring the kit.\n", false).as_deref(),
        Some("Bring the kit.")
    );
    // an empty part leaves no empty line, nothing left writes nothing
    assert_eq!(
        groups::notice_chat_text("Violet Wolf", "Workshop", "", true).as_deref(),
        Some(format!("{head}\nWorkshop").as_str())
    );
    assert_eq!(groups::notice_chat_text("Violet Wolf", "Workshop", "  ", false), None);
}

#[test]
fn the_line_goes_to_the_open_group_chat_not_unread_and_stays_when_the_card_closes() {
    let mut world = world();
    world.session_lines(GROUP);
    world.apply(NetEvent::InstantMessage(notice(None)));
    let head = aurora_i18n::tr!("world-groups-notice-in-chat", sender = "Violet Wolf".to_string());
    assert_eq!(notice_lines(&world, GROUP), [format!("{head}\nWorkshop\nBring the kit.")]);
    let s = world.social.ims.iter().find(|s| s.other == GROUP).expect("group chat");
    assert_eq!(s.unread, 0, "a notice's line is not counted as unread");
    // the group's session, never the sender's or the notice's ids
    assert!(!world.social.has_session(&SENDER) && !world.social.has_session(&TX));
    // the card is still there, and closing it keeps the line
    let id = world.notifications.list.last().expect("notice listed").id;
    world.respond_notification(id, Response::Dismiss);
    assert!(world.notifications.list.iter().all(|n| n.id != id));
    assert_eq!(notice_lines(&world, GROUP).len(), 1);
}

#[test]
fn without_subjects_the_line_is_the_text_alone() {
    let mut world = world();
    world.groups.notices_in_chat.subjects = false;
    world.session_lines(GROUP);
    world.apply(NetEvent::InstantMessage(notice(None)));
    assert_eq!(notice_lines(&world, GROUP), ["Bring the kit."]);
}

#[test]
fn turned_off_or_not_our_group_writes_no_line() {
    let mut off = world();
    off.groups.notices_in_chat.on = false;
    off.session_lines(GROUP);
    off.apply(NetEvent::InstantMessage(notice(None)));
    assert!(notice_lines(&off, GROUP).is_empty());
    assert!(off.notifications.list.last().is_some(), "the card still shows");

    // a group we are not in (gAgent.getGroupData fails)
    let mut stranger = world();
    stranger.groups.groups.clear();
    stranger.session_lines(GROUP);
    stranger.apply(NetEvent::InstantMessage(notice(None)));
    assert!(notice_lines(&stranger, GROUP).is_empty());
}

#[test]
fn without_an_open_group_chat_the_line_goes_to_the_transcript_only() {
    use std::time::Duration;
    let root = std::env::temp_dir().join(format!("aurora-group-notice-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("temp folder");
    let mut world = world();
    world.chat_log = chat_log::ChatLog::in_folder(root.clone());
    world.chat_log.set_grid("agni".into());
    world.chat_log.login(world.agent_id);
    world.apply(NetEvent::InstantMessage(notice(None)));
    // no conversation opened for it
    assert!(!world.social.has_session(&GROUP));
    assert!(world.chat_log.flush(Duration::from_secs(10)));
    let path = world
        .chat_log
        .file_of(&chat_log::names::group("Northern Wolves"))
        .expect("group transcript");
    let log: Vec<String> = chat_log::store::read_tail(&path)
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].contains("Workshop") && log[0].contains("Bring the kit."), "{log:?}");
    assert!(world.chat_log.flush(Duration::from_secs(10)));
    std::fs::remove_dir_all(&root).expect("temp folder removed");
}
