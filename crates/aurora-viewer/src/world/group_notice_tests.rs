//! Group notices in the notification center: titled by their group, their
//! sender first in the text, their attachment saved or declined from the
//! card (LLToastGroupNotifyPanel).

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
