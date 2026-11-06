use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::{NetEvent, bridge::query::Reply};
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
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
    assert!(state.can_pose(&world, peer));
    let index = world.objects.index_of_uuid(&peer).unwrap();
    world.objects.remove_idx(index);
    state.sync(&world, now);
    assert!(!state.can_pose(&world, peer));
    world.main_region = Some(2);
    state.sync(&world, now);
    assert!(state.accepted.is_empty() && state.granted_to.is_empty());
}
#[test]
fn poser_cooldown_timeout_allows_a_fresh_request_after_two_minutes() {
    let (world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    assert!(!state.request(&world, peer, now), "one pending request per resident");
    state.sync(&world, now + TIMEOUT);
    assert!(matches!(state.status(peer), Status::NoAnswer));
    let ready = now + TIMEOUT + Duration::from_secs(120);
    state.sync(&world, ready - Duration::from_millis(1));
    assert!(!state.request(&world, peer, ready - Duration::from_millis(1)));
    state.sync(&world, ready);
    assert!(state.request(&world, peer, ready));
}
#[test]
fn poser_cooldown_explicit_refusal_still_waits_ten_minutes() {
    let (world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    let refused = now + Duration::from_secs(5);
    state.receive(&world, &wire(id, PoseKind::Refuse, peer), refused);
    assert!(matches!(state.status(peer), Status::Refused));
    let ready = refused + Duration::from_secs(600);
    state.sync(&world, ready - Duration::from_millis(1));
    assert!(!state.request(&world, peer, ready - Duration::from_millis(1)));
    state.sync(&world, ready);
    assert!(state.request(&world, peer, ready));
}
#[test]
fn poser_cooldown_normally_ended_sessions_allow_fresh_nonces_immediately() {
    for requester_ends in [false, true] {
        let (world, peer, mut state) = fixture();
        let now = Instant::now();
        assert!(state.request(&world, peer, now));
        let id = state.outgoing[&peer].id;
        state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
        assert!(state.can_pose(&world, peer));
        if requester_ends {
            state.cancel(peer);
        } else {
            state.receive(&world, &wire(id, PoseKind::Withdraw, peer), now);
        }
        assert!(!state.can_pose(&world, peer));
        assert!(state.request(&world, peer, now));
        assert_ne!(state.outgoing[&peer].id, id);
        state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
        assert!(!state.can_pose(&world, peer), "the ended session cannot grant fresh consent");
    }
}
#[test]
fn poser_cooldown_receiver_timeout_waits_two_minutes_without_sending_a_refusal() {
    let (world, peer, mut state) = fixture();
    state.preferences.policy = Policy::Everyone;
    let now = Instant::now();
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
    state.sync(&world, now + TIMEOUT);
    assert!(state.incoming.is_empty());
    assert!(state.out.iter().all(|(_, kind, _)| *kind != PoseKind::Refuse));
    // a few seconds under the requester's 2:00, the relay's slack
    let ready = now + TIMEOUT + Duration::from_secs(120) - RELAY_SLACK;
    state.sync(&world, ready - Duration::from_millis(1));
    state.receive(
        &world,
        &wire(Uuid::new_v4(), PoseKind::Request, peer),
        ready - Duration::from_millis(1),
    );
    assert!(state.incoming.is_empty());
    state.sync(&world, ready);
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), ready);
    assert_eq!(state.incoming.len(), 1);
}
#[test]
fn poser_cooldown_timeout_cancel_before_sync_preserves_the_receiver_wait() {
    let (world, peer, mut state) = fixture();
    state.preferences.policy = Policy::Everyone;
    let now = Instant::now();
    let id = Uuid::new_v4();
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    state.receive(&world, &wire(id, PoseKind::Cancel, peer), now + TIMEOUT);
    assert!(state.incoming.is_empty());
    let ready = now + TIMEOUT + Duration::from_secs(120) - RELAY_SLACK;
    state.sync(&world, ready - Duration::from_millis(1));
    state.receive(
        &world,
        &wire(Uuid::new_v4(), PoseKind::Request, peer),
        ready - Duration::from_millis(1),
    );
    assert!(state.incoming.is_empty());
    state.sync(&world, ready);
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), ready);
    assert_eq!(state.incoming.len(), 1);
}
#[test]
fn poser_cooldown_timeout_cancel_relayed_just_under_a_minute_still_waits() {
    let (world, peer, mut state) = fixture();
    state.preferences.policy = Policy::Everyone;
    let now = Instant::now();
    let id = Uuid::new_v4();
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    // the requester's own 60 s ran out; its Cancel reached us 2 s "early"
    let cancel = now + TIMEOUT - Duration::from_secs(2);
    state.receive(&world, &wire(id, PoseKind::Cancel, peer), cancel);
    assert!(state.incoming.is_empty());
    state.sync(&world, cancel + Duration::from_secs(60));
    state.receive(
        &world,
        &wire(Uuid::new_v4(), PoseKind::Request, peer),
        cancel + Duration::from_secs(60),
    );
    assert!(state.incoming.is_empty(), "the no-answer wait applies");
    // and ends no later than the requester's 2:00 countdown
    let ready = cancel + Duration::from_secs(120);
    state.sync(&world, ready);
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), ready);
    assert_eq!(state.incoming.len(), 1);
}
#[test]
fn poser_cooldown_early_cancel_by_the_requester_leaves_no_wait() {
    let (world, peer, mut state) = fixture();
    state.preferences.policy = Policy::Everyone;
    let now = Instant::now();
    let id = Uuid::new_v4();
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    let cancel = now + Duration::from_secs(10);
    state.receive(&world, &wire(id, PoseKind::Cancel, peer), cancel);
    state.sync(&world, cancel + Duration::from_secs(1));
    state.receive(
        &world,
        &wire(Uuid::new_v4(), PoseKind::Request, peer),
        cancel + Duration::from_secs(1),
    );
    assert_eq!(state.incoming.len(), 1);
}
#[test]
fn poser_cooldown_receiver_explicit_refusal_still_waits_ten_minutes() {
    let (world, peer, mut state) = fixture();
    state.preferences.policy = Policy::Everyone;
    let now = Instant::now();
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
    let refused = now + Duration::from_secs(5);
    state.answer(state.incoming[0], false, false, refused);
    assert!(state.out.iter().any(|(_, kind, _)| *kind == PoseKind::Refuse));
    let ready = refused + Duration::from_secs(600);
    state.sync(&world, ready - Duration::from_millis(1));
    state.receive(
        &world,
        &wire(Uuid::new_v4(), PoseKind::Request, peer),
        ready - Duration::from_millis(1),
    );
    assert!(state.incoming.is_empty());
    state.sync(&world, ready);
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), ready);
    assert_eq!(state.incoming.len(), 1);
}
#[test]
fn poser_cooldown_ended_receiver_keeps_old_nonces_invalid_but_accepts_new_requests() {
    for requester_ends in [false, true] {
        let (world, peer, mut state) = fixture();
        state.preferences.policy = Policy::Everyone;
        let now = Instant::now();
        let id = Uuid::new_v4();
        state.receive(&world, &wire(id, PoseKind::Request, peer), now);
        state.answer(state.incoming[0], true, false, now);
        if requester_ends {
            state.receive(&world, &wire(id, PoseKind::Cancel, peer), now);
        } else {
            state.withdraw(peer);
        }
        state.receive(&world, &wire(id, PoseKind::Request, peer), now);
        assert!(state.incoming.is_empty());
        state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
        assert_eq!(state.incoming.len(), 1);
        assert!(state.granted_to.is_empty(), "a fresh request needs fresh acceptance");
    }
}
#[test]
fn poser_cooldown_keeps_the_64_request_limit_and_one_pending_request_per_peer() {
    let (world, peer, mut state) = fixture();
    state.preferences.policy = Policy::Everyone;
    let now = Instant::now();
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
    assert_eq!(state.incoming.len(), 1);
    state.incoming.clear();
    for n in 1..=64 {
        let r = Request {
            id: Uuid::new_v4(),
            peer: Uuid::from_u128(n),
            at: now,
        };
        state.incoming.push_back(r);
        state.outgoing.insert(r.peer, r);
    }
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
    assert_eq!(state.incoming.len(), 64);
    assert!(!state.outgoing.contains_key(&peer));
    assert!(!state.request(&world, peer, now));
    assert_eq!(state.outgoing.len(), 64);
}
#[test]
fn incoming_privacy_defaults_to_friends_and_acceptance_is_never_implicit() {
    let (mut world, peer, mut state) = fixture();
    let now = Instant::now();
    let id = Uuid::new_v4();
    assert_eq!(state.preferences.policy, Policy::Friends);
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    assert!(state.incoming.is_empty());
    state.preferences.policy = Policy::Everyone;
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    assert_eq!(state.incoming.len(), 1);
    assert!(state.granted_to.is_empty());
    let request = state.incoming[0];
    state.answer(request, true, false, now);
    assert_eq!(state.granted_to.get(&peer), Some(&id));
    state.withdraw(peer);
    assert!(state.granted_to.is_empty());
    let id = Uuid::new_v4();
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    let request = state.incoming[0];
    state.answer(request, false, true, now);
    assert!(state.preferences.blocked.contains(&peer));
    assert!(state.dirty);
    state.receive(
        &world,
        &wire(Uuid::new_v4(), PoseKind::Request, peer),
        now + Duration::from_secs(600),
    );
    assert!(state.incoming.is_empty());
    state.preferences.policy = Policy::Nobody;
    assert!(state.commands(None).is_empty());
    world.main_region = Some(2);
    state.sync(&world, now);
    assert!(state.preferences.blocked.contains(&peer) && state.dirty);
}
#[test]
fn malformed_or_untrusted_protocol_messages_never_grant_consent_and_transport_has_no_probe() {
    let (mut world, peer, mut state) = fixture();
    let now = Instant::now();
    state.preferences.policy = Policy::Everyone;
    for message in [
        "<auroraPose>bad</auroraPose>".to_owned(),
        wire(Uuid::nil(), PoseKind::Request, peer),
        wire(Uuid::new_v4(), PoseKind::Accept, peer),
    ] {
        state.receive(&world, &message, now);
    }
    assert!(state.incoming.is_empty() && state.accepted.is_empty());
    let chat = aurora_net::ChatMessage {
        from_name: "Fixture attachment".into(),
        source_id: Uuid::from_u128(800),
        owner_id: world.agent_id,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        position: glam::Vec3::ZERO,
        message: wire(Uuid::new_v4(), PoseKind::Request, peer),
    };
    let mut bridge = crate::world::bridge::Bridge::default();
    assert!(bridge.receive(&mut world, NetEvent::Chat(chat), now, now).is_none());
    assert!(world.poser_consent.incoming.is_empty());
    let query = Query::Pose {
        request: Uuid::new_v4(),
        kind: PoseKind::Request,
        target: peer,
    };
    assert!(query.valid());
    assert_eq!(query.decode(b"[]"), Some(Reply::Pose));
    assert!(query.decode(b"[1]").is_none());
    let lsl = include_str!("../bridge/query.lsl");
    for guard in [
        "llGetOwnerKey(sender)",
        "OBJECT_ATTACHED_POINT",
        "llList2Key(fields, 3) != gOwner",
        "llRegionSayTo(target, POSE_CHANNEL",
        "<auroraPose>",
    ] {
        assert!(lsl.contains(guard), "{guard}");
    }
    assert!(!lsl.contains("llInstantMessage("));
    assert!(!lsl.contains("Aurora?"));
}
#[test]
fn friends_are_only_asked_and_mutes_distance_and_cancellation_revoke_access() {
    let (mut world, peer, mut state) = fixture();
    world.social.friends.push(crate::world::social::Friend {
        id: peer,
        online: true,
        rights_given: 1,
        rights_has: 1,
    });
    let now = Instant::now();
    let id = Uuid::new_v4();
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    assert_eq!(state.incoming.len(), 1);
    assert!(state.granted_to.is_empty());
    state.receive(&world, &wire(id, PoseKind::Cancel, peer), now);
    assert!(state.incoming.is_empty());
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
    let index = world.objects.index_of_uuid(&peer).unwrap();
    let original = world.objects.get(index).unwrap().position;
    world.objects.get_mut(index).unwrap().position += glam::Vec3::splat(100.0);
    assert!(!state.can_pose(&world, peer));
    // Withdrawal is still authenticated region-wide when the peer is beyond 50 m.
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
    state.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
    assert!(state.incoming.is_empty());
    assert!(!state.request(&world, peer, now));
}
#[test]
fn reordered_withdraw_and_cancel_cannot_reactivate_a_finished_request() {
    let (world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    let id = state.outgoing[&peer].id;
    state.receive(&world, &wire(id, PoseKind::Withdraw, peer), now);
    state.receive(&world, &wire(id, PoseKind::Accept, peer), now);
    assert!(!state.can_pose(&world, peer));
    assert!(state.out.is_empty());
    state.preferences.policy = Policy::Everyone;
    let id = Uuid::new_v4();
    state.receive(&world, &wire(id, PoseKind::Cancel, peer), now);
    state.receive(&world, &wire(id, PoseKind::Request, peer), now);
    assert!(state.incoming.is_empty());
    assert!(state.request(&world, peer, now));
    state.cancel(peer);
    assert!(state.out.iter().all(|(_, kind, _)| *kind != PoseKind::Request));
}

#[test]
fn poser_cooldown_delayed_sync_does_not_restart_the_wait_after_the_deadline() {
    let (world, peer, mut state) = fixture();
    let now = Instant::now();
    assert!(state.request(&world, peer, now));
    let later = now + TIMEOUT + Duration::from_secs(121);
    state.sync(&world, later);
    assert!(state.request(&world, peer, later));

    let (world, peer, mut receiver) = fixture();
    receiver.preferences.policy = Policy::Everyone;
    receiver.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), now);
    receiver.sync(&world, later);
    receiver.receive(&world, &wire(Uuid::new_v4(), PoseKind::Request, peer), later);
    assert_eq!(receiver.incoming.len(), 1);
}

#[test]
fn poser_cooldown_countdown_reports_the_actual_time_left_for_each_reason() {
    for timeout in [false, true] {
        let (world, peer, mut state) = fixture();
        let now = Instant::now();
        assert!(state.request(&world, peer, now));
        let (start, duration) = if timeout {
            let start = now + TIMEOUT;
            state.sync(&world, start);
            (start, Duration::from_secs(120))
        } else {
            let id = state.outgoing[&peer].id;
            state.receive(&world, &wire(id, PoseKind::Refuse, peer), now);
            (now, Duration::from_secs(600))
        };
        assert_eq!(state.retry_after(peer, start), duration);
        assert_eq!(
            state.retry_after(peer, start + Duration::from_secs(45)),
            duration - Duration::from_secs(45)
        );
        assert_eq!(state.retry_after(peer, start + duration), Duration::ZERO);
        assert_eq!(state.retry_after(peer, start + duration + Duration::from_secs(1)), Duration::ZERO);
    }
}
