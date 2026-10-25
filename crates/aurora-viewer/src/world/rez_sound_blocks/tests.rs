use super::*;
use crate::world::mutes::{Mute, MuteType};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn scope() -> Scope {
    Scope {
        agent: id(99),
        region: Some(1),
        session: Some(id(9)),
    }
}
fn setup() -> (Tracker, MuteList) {
    let mut mutes = MuteList::default();
    mutes.reset(id(99));
    mutes
        .add(
            Mute {
                id: id(100),
                name: "Rezzer".into(),
                kind: MuteType::Object,
                flags: 0,
            },
            0,
        )
        .unwrap();
    let mut tracker = Tracker::default();
    tracker.configure(true, scope(), &mutes);
    (tracker, mutes)
}
fn link(object: u128, root: u128, rezzer: u128) -> Rezzer {
    Rezzer {
        object: id(object),
        root: id(root),
        rezzer: id(rezzer),
    }
}

#[test]
fn rez_sound_blocks_follow_verified_cascades_and_leave_unrelated_objects_alone() {
    let (mut t, mut mutes) = setup();
    let now = Instant::now();
    t.set_live(&[id(1), id(2), id(3)], now);
    assert!(t.waiting(id(1), 1, now, now));
    assert!(!t.blocked(id(1)));
    let (request, query) = t.request(now).unwrap();
    assert!(matches!(query, Query::Rezzers(ids) if ids.len() == 3));
    t.reply(
        request,
        Ok(Reply::Rezzers(vec![link(1, 1, 100), link(2, 2, 1), link(3, 3, 0)])),
        true,
        scope(),
        now,
    );
    assert!(t.blocked(id(1)) && t.blocked(id(2)));
    assert!(!t.blocked(id(3)) && !t.waiting(id(3), 1, now, now));
    mutes.remove(&id(100), "", 0);
    t.configure(true, scope(), &mutes);
    assert!(!t.blocked(id(1)) && !t.blocked(id(2)));
}

#[test]
fn rez_sound_blocks_use_root_identity_and_reject_stale_scope_or_link_replies() {
    let (mut t, mutes) = setup();
    let now = Instant::now();
    t.watch(id(1), 1, Some((id(10), id(20))), now);
    t.set_live(&[id(1)], now);
    let (request, _) = t.request(now).unwrap();
    t.watch(id(1), 1, Some((id(11), id(20))), now);
    t.reply(request, Ok(Reply::Rezzers(vec![link(1, 100, 0)])), true, scope(), now);
    assert!(!t.blocked(id(1)), "relinked source rejects the old request");
    let (request, _) = t.request(now + INTERVAL).unwrap();
    let changed = Scope {
        session: Some(id(8)),
        ..scope()
    };
    t.reply(request, Ok(Reply::Rezzers(vec![link(1, 100, 0)])), true, changed, now);
    assert!(!t.blocked(id(1)), "rotated endpoint cannot apply the old request");
    t.configure(true, changed, &mutes);
    t.set_live(&[id(1)], now);
    let (request, _) = t.request(now).unwrap();
    t.reply(request, Ok(Reply::Rezzers(vec![link(1, 100, 0)])), true, changed, now);
    assert!(t.blocked(id(1)), "linked prim inherits its blocked linkset root");
    t.configure(false, changed, &mutes);
    assert!(!t.blocked(id(1)));
}

#[test]
fn rez_sound_blocks_bound_waiting_batch_sizes_cycles_and_bridge_loss() {
    let (mut t, mutes) = setup();
    let now = Instant::now();
    t.set_live(&(1..=40).map(id).collect::<Vec<_>>(), now);
    let (request, query) = t.request(now).unwrap();
    assert!(matches!(query, Query::Rezzers(ids) if ids.len() == MAX_REZZERS));
    assert!(!t.waiting(id(1), 2, now, now), "neighbor region cannot be queried by this bridge");
    assert!(!t.waiting(Uuid::nil(), 1, now, now));
    assert!(!t.waiting(id(1), 1, now, now + GRACE));
    assert!(t.waiting(id(500), 1, now, now));
    assert!(
        !t.waiting(id(500), 1, now, now + GRACE),
        "untracked sources also have a bounded wait"
    );
    t.reply(request, Ok(Reply::Rezzers(vec![link(1, 1, 2), link(2, 2, 1)])), true, scope(), now);
    assert!(
        !t.blocked(id(1)) && !t.waiting(id(1), 1, now, now),
        "cycles terminate without a guessed mute"
    );
    t.configure(true, Scope { session: None, ..scope() }, &mutes);
    assert!(!t.available() && !t.waiting(id(1), 1, now, now));
    assert!(t.request(now).is_none());
}

#[test]
fn rez_sound_blocks_discover_silent_ancestors_in_later_batches_and_retry_timeouts() {
    let (mut t, _) = setup();
    let now = Instant::now();
    t.set_live(&[id(2)], now);
    let (request, query) = t.request(now).unwrap();
    assert_eq!(query, Query::Rezzers(vec![id(2)]));
    t.reply(request, Ok(Reply::Rezzers(vec![link(2, 2, 1)])), true, scope(), now);
    assert!(!t.blocked(id(2)) && t.waiting(id(2), 1, now, now));
    let later = now + INTERVAL;
    t.set_live(&[id(2)], later);
    let (expired, query) = t.request(later).unwrap();
    assert_eq!(query, Query::Rezzers(vec![id(1)]), "silent ancestor is requested separately");
    assert!(t.request(later + INTERVAL).is_none(), "one request in flight");
    t.reply(expired, Ok(Reply::Rezzers(vec![link(1, 1, 100)])), true, scope(), later + TIMEOUT);
    assert!(!t.blocked(id(2)), "late results cannot mute a source");
    let (request, _) = t.request(later + TIMEOUT).unwrap();
    t.reply(request, Ok(Reply::Rezzers(vec![link(1, 1, 100)])), true, scope(), later + TIMEOUT);
    assert!(t.blocked(id(2)));
}
