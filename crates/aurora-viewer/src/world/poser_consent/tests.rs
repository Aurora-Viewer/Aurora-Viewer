use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::{
    NetEvent,
    bridge::query::{POSE_LABEL_BYTES, POSE_MESSAGE_BYTES, POSE_NOTICE_BYTES, Reply},
};
use std::sync::Arc;

fn fixture() -> (World, Uuid, State) {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![demo::action_avatar(false)],
    });
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![demo::poser::peer()],
    });
    let peer = world
        .objects
        .iter()
        .find(|(_, o)| o.is_avatar() && o.full_id != world.agent_id)
        .unwrap()
        .1
        .full_id;
    let mut state = State::default();
    state.sync(&world, Instant::now());
    (world, peer, state)
}
fn wire(id: Uuid, kind: PoseKind, peer: Uuid) -> String {
    format!("<auroraPose>{id}|{}|{peer}</auroraPose>", kind.wire())
}
fn endpoint() -> Endpoint {
    Endpoint {
        url: "https://simhost.example:12043/cap/00000000-0000-0000-0000-000000000001".into(),
        token: Uuid::from_u128(9),
    }
}
/// The bridge queries sent: (request, target, whether it shows the dialog).
fn sent(state: &mut State) -> Vec<(Uuid, Uuid, bool)> {
    state
        .commands(Some(endpoint()))
        .into_iter()
        .map(|command| match command {
            NetCommand::BridgeQuery {
                query: Query::Pose { request, target, ask },
                ..
            } => (request, target, ask.is_some()),
            other => panic!("not a pose query: {other:?}"),
        })
        .collect()
}
#[test]
fn outgoing_consent_requires_matching_nonce_and_expires_on_withdraw_departure_or_region_change() {
    let (mut world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Accept, peer), now);
    assert!(!state.can_pose(&world, peer));
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
    assert!(state.can_pose(&world, peer));
    state.receive(&world, &wire(id, PoseKind::Withdraw, peer), now);
    assert!(!state.can_pose(&world, peer));
    assert_eq!(state.notice, Some("poser-consent-ended"));
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
    assert!(state.can_pose(&world, peer));
    sent(&mut state);
    let index = world.objects.index_of_uuid(&peer).unwrap();
    world.objects.remove_idx(index);
    state.sync(&world, now);
    assert!(!state.can_pose(&world, peer));
    // the resident left: our bridge forgets their consent
    assert_eq!(sent(&mut state), [(id, peer, false)]);
    world.main_region = Some(2);
    state.sync(&world, now);
    assert!(state.accepted.is_empty() && state.outgoing.is_empty());
}
#[test]
fn request_timeouts_and_refusals_enforce_one_pending_request_and_ten_minute_cooldown() {
    let (world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    assert!(!state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    assert_eq!(sent(&mut state), [(id, peer, true)]);
    state.sync(&world, now + TIMEOUT);
    assert!(matches!(state.status(peer), Status::NoAnswer));
    // an answer after the timeout is not taken, and the bridge forgets the ask
    assert_eq!(sent(&mut state), [(id, peer, false)]);
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now + TIMEOUT);
    assert!(!state.can_pose(&world, peer));
    assert!(!state.request(&world, peer, now + TIMEOUT));
    state.sync(&world, now + TIMEOUT + COOLDOWN);
    assert!(state.request(&world, peer, now + TIMEOUT + COOLDOWN));
    let id = state.outgoing[&peer].id;
    state.receive(&world, &wire(id, PoseKind::Refuse, peer), now + TIMEOUT + COOLDOWN);
    assert!(matches!(state.status(peer), Status::Refused));
    assert!(!state.request(&world, peer, now + TIMEOUT + COOLDOWN));
}
#[test]
fn the_bridge_shows_the_dialog_in_our_language_and_is_told_to_forget() {
    let (world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.commands(None).is_empty(), "no bridge, nothing sent");
    assert!(state.request(&world, peer, now));
    let commands = state.commands(Some(endpoint()));
    let [
        NetCommand::BridgeQuery {
            query: Query::Pose {
                target, ask: Some(ask), ..
            },
            ..
        },
    ] = commands.as_slice()
    else {
        panic!("one dialog: {commands:?}");
    };
    assert_eq!(*target, peer);
    assert_eq!(*ask, dialog());
    // both texts tell the resident how to stop
    let command = format!("/{POSE_STOP_CHANNEL} stop");
    assert!(ask.question.contains(&command) && ask.granted.contains(&command));
    assert_ne!(POSE_STOP_CHANNEL, 99, "99 is taken by many HUDs");
    // a cancel once the ask went out has the bridge forget it; before, it
    // sends nothing
    let id = match &commands[0] {
        NetCommand::BridgeQuery {
            query: Query::Pose { request, .. },
            ..
        } => *request,
        _ => unreachable!(),
    };
    state.cancel(peer);
    assert_eq!(sent(&mut state), [(id, peer, false)]);
    state.sync(&world, now + COOLDOWN);
    assert!(state.request(&world, peer, now + COOLDOWN));
    state.cancel(peer);
    assert!(sent(&mut state).is_empty());
    // once accepted, our cancel tells the bridge to forget it
    state.sync(&world, now + COOLDOWN * 2);
    assert!(state.request(&world, peer, now + COOLDOWN * 2));
    let id = state.outgoing[&peer].id;
    sent(&mut state);
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now + COOLDOWN * 2);
    state.cancel(peer);
    assert!(!state.can_pose(&world, peer));
    assert_eq!(sent(&mut state), [(id, peer, false)]);
}
#[test]
fn the_dialog_texts_fit_the_script_dialog_in_every_language() {
    let mut args = aurora_i18n::Args::new();
    args.set("channel", POSE_STOP_CHANNEL.to_string());
    for language in ["en", "fr", "ja"] {
        let text = |key| aurora_i18n::format_in(language, key, &args);
        for key in ["poser-consent-dialog-question", "poser-consent-dialog-granted"] {
            let t = text(key);
            assert!(t.len() <= POSE_MESSAGE_BYTES, "{language} {key}: {} bytes", t.len());
            assert!(t.contains(&format!("/{POSE_STOP_CHANNEL} stop")), "{language} {key}");
        }
        assert!(text("poser-consent-dialog-stopped").len() <= POSE_NOTICE_BYTES, "{language}");
        let labels = ["poser-consent-dialog-yes", "poser-consent-dialog-no", "poser-consent-dialog-stop"].map(text);
        assert!(labels.iter().all(|l| l.len() <= POSE_LABEL_BYTES), "{language}: {labels:?}");
        assert!(labels[0] != labels[1], "{language}: the bridge tells Yes from No by its label");
    }
}
#[test]
fn malformed_or_untrusted_messages_never_grant_consent_and_no_viewer_is_spoken_to() {
    let (mut world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    for message in [
        "<auroraPose>bad</auroraPose>".to_owned(),
        wire(Uuid::nil(), PoseKind::Accept, peer),
        wire(Uuid::new_v4(), PoseKind::Accept, peer),
        format!("<auroraPose>{id}|request|{peer}</auroraPose>"),
    ] {
        state.receive(&world, &message, now);
    }
    assert!(state.accepted.is_empty());
    // an owner say from anything but our worn bridge is not read
    let chat = aurora_net::ChatMessage {
        from_name: "Fixture attachment".into(),
        source_id: Uuid::from_u128(800),
        owner_id: world.agent_id,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: wire(id, PoseKind::Accept, peer),
    };
    let mut bridge = crate::world::bridge::Bridge::default();
    assert!(bridge.receive(&mut world, NetEvent::Chat(chat), now, now).is_none());
    assert!(world.poser_consent.accepted.is_empty());
    let query = Query::Pose {
        request: Uuid::new_v4(),
        target: peer,
        ask: Some(dialog()),
    };
    assert!(query.valid());
    assert_eq!(query.decode(b"[]"), Some(Reply::Pose));
    assert!(query.decode(b"[1]").is_none());
    let lsl = include_str!("../bridge/query.lsl");
    for guard in [
        "llDialog(target,",
        "llDialog(sender, gGrantedText, [gStop], POSE_DIALOG_CHANNEL)",
        &format!("integer STOP_CHANNEL = {POSE_STOP_CHANNEL};"),
        "llListFindList(gAsked, [sender])",
        "llListFindList(gGranted, [sender])",
        "llInstantMessage(sender, gGrantedText)",
        "<auroraPose>",
    ] {
        assert!(lsl.contains(guard), "{guard}");
    }
    // the old viewer-to-viewer protocol is gone
    for gone in ["pose1|", "POSE_CHANNEL =", "-184735220"] {
        assert!(!lsl.contains(gone), "{gone}");
    }
}
#[test]
fn mutes_distance_and_withdrawal_revoke_access() {
    let (mut world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
    let index = world.objects.index_of_uuid(&peer).unwrap();
    let original = world.objects.get(index).unwrap().position;
    world.objects.get_mut(index).unwrap().position += glam::Vec3::splat(100.0);
    assert!(!state.can_pose(&world, peer));
    // Stop is still heard region-wide when the resident is beyond 50 m.
    state.receive(&world, &wire(id, PoseKind::Withdraw, peer), now);
    assert!(state.accepted.is_empty());
    world.objects.get_mut(index).unwrap().position = original;
    use crate::world::mutes::{Mute, MuteType};
    world
        .mutes
        .add(
            Mute {
                id: peer,
                name: "Fixture resident".into(),
                kind: MuteType::Agent,
                flags: 0,
            },
            0,
        )
        .unwrap();
    assert!(!state.request(&world, peer, now));
}
#[test]
fn a_finished_request_cannot_be_reactivated() {
    let (world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    state.cancel(peer);
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
    assert!(!state.can_pose(&world, peer));
    // a withdrawal of an old consent leaves a newer one alone
    state.sync(&world, now + COOLDOWN);
    assert!(state.request(&world, peer, now + COOLDOWN));
    let newer = state.outgoing[&peer].id;
    state.receive(&world, &wire(newer, PoseKind::Accept, peer), now + COOLDOWN);
    state.receive(&world, &wire(id, PoseKind::Withdraw, peer), now + COOLDOWN);
    assert!(state.can_pose(&world, peer));
}
#[test]
fn the_reply_demo_asks_nothing_and_accepts_what_the_user_asked_two_seconds_later() {
    let (world, peer, mut state) = fixture();
    for frame in [450, 480, 560] {
        state.demo(&world, peer, "consent-reply", frame);
    }
    assert!(state.outgoing.is_empty(), "nothing by itself");
    assert!(state.request(&world, peer, Instant::now()));
    state.demo(&world, peer, "consent-reply", 600);
    assert!(!state.can_pose(&world, peer), "not at once: Waiting shows first");
    if let Some(r) = state.outgoing.get_mut(&peer) {
        r.at -= Duration::from_secs(3);
    }
    state.demo(&world, peer, "consent-reply", 700);
    assert!(state.can_pose(&world, peer), "two seconds later, accepted");
    assert!(state.outgoing.is_empty());
}
