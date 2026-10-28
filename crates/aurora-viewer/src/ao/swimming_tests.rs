//! AO transition contracts shared by local playback and simulator observers.
use super::{runtime::Runtime, tests::*, *};
use crate::{agent::MoveInput, world::PlayingAnimation};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

fn unique(set: &mut Set, state: State, id: u128) {
    let config = set.states.get_mut(&state).expect("state");
    for (i, a) in config.steps.iter_mut().flat_map(|s| &mut s.members).enumerate() {
        a.asset = Uuid::from_u128(id + i as u128);
    }
}

#[test]
fn horizontal_swimming_selects_depth_instead_of_style_speed_or_direction() {
    let mut runtime = crate::movement::Runtime::default();
    for (depth, expected) in [
        (crate::movement::State::Surface, State::SwimmingForward),
        (crate::movement::State::Submerged, State::UnderwaterSwimming),
    ] {
        runtime.state = depth;
        for input in [
            MoveInput {
                forward: true,
                ..Default::default()
            },
            MoveInput {
                back: true,
                ..Default::default()
            },
            MoveInput {
                strafe_left: true,
                ..Default::default()
            },
            MoveInput {
                forward: true,
                run: true,
                ..Default::default()
            },
        ] {
            assert_eq!(runtime.swim_intent(&input), Some(expected));
        }
        assert_eq!(
            runtime.swim_intent(&MoveInput::default()),
            Some(if depth == crate::movement::State::Surface {
                State::Floating
            } else {
                State::UnderwaterFloating
            })
        );
    }
    assert!(!State::ALL.iter().any(|s| State::LEGACY_SWIMMING.contains(s)));
}

#[test]
fn both_swimming_states_support_groups_parallel_tracks_and_cycles() {
    let (_, mut set) = fixture();
    let now = Instant::now();
    for (n, state) in [State::SwimmingForward, State::UnderwaterSwimming].into_iter().enumerate() {
        let config = set.states.get_mut(&state).expect("state");
        let mut other = config.steps[0].clone();
        other.id = Uuid::from_u128(21000 + n as u128);
        other.group = true;
        other.members.push(other.members[0].clone());
        config.steps.push(other);
        for (i, a) in config.steps.iter_mut().flat_map(|s| &mut s.members).enumerate() {
            a.asset = Uuid::from_u128(22000 + n as u128 * 10 + i as u128);
        }
        config.tracks.push(Track {
            id: Uuid::from_u128(23000 + n as u128),
            name: "Hands".into(),
            cycle: Cycle::default(),
            members: vec![config.steps[0].members[0].clone()],
        });
        config.tracks[0].members[0].asset = Uuid::from_u128(24000 + n as u128);
        config.cycle = Cycle {
            cycle: true,
            random: false,
            seconds: 1.0,
        };
        assert!(set.swimming_ready(), "lists/groups do not disable the swim motor");
        let mut runtime = Runtime::default();
        let mut f = frame(&[]);
        f.enabled = false;
        f.swim_hold = Some((state, signal(state, now)));
        let (first, _) = runtime.update(Some(&set), f, now, |_| true, |_| 0);
        assert_eq!(first.len(), 3, "two body group members and a parallel track");
        let mut f = frame(&[]);
        f.enabled = false;
        f.swim_hold = Some((state, signal(state, now)));
        let (second, changes) = runtime.update(Some(&set), f, now + Duration::from_millis(1100), |_| true, |_| 0);
        assert_eq!(second.len(), 2);
        assert!(changes.iter().any(|(_, start)| !start) && changes.iter().any(|(_, start)| *start));
        assert_eq!(runtime.current, Some(state));
    }
}

#[test]
fn a_loading_cycle_or_depth_change_keeps_a_complete_pose_until_replacement_is_ready() {
    let (_, mut set) = fixture();
    unique(&mut set, State::SwimmingForward, 25001);
    unique(&mut set, State::UnderwaterSwimming, 25003);
    let c = set.states.get_mut(&State::SwimmingForward).expect("surface");
    let mut second = c.steps[0].clone();
    second.id = Uuid::from_u128(25002);
    second.members[0].asset = second.id;
    c.steps.push(second);
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame(&[]);
    f.swim_hold = Some((State::SwimmingForward, signal(State::SwimmingForward, now)));
    let (first, _) = runtime.update(Some(&set), f, now, |_| true, |_| 0);
    let old = first[0];
    runtime.choose(State::SwimmingForward, set.states[&State::SwimmingForward].folder, 1);
    let mut f = frame(&[]);
    f.swim_hold = Some((State::SwimmingForward, signal(State::SwimmingForward, now)));
    let (held, changes) = runtime.update(Some(&set), f, now + Duration::from_millis(20), |id| *id == old.id, |_| 0);
    assert_eq!(held, first);
    assert!(changes.is_empty());
    let mut f = frame(&[]);
    f.swim_hold = Some((State::UnderwaterSwimming, signal(State::UnderwaterSwimming, now)));
    let (held, changes) = runtime.update(Some(&set), f, now + Duration::from_millis(40), |id| *id == old.id, |_| 0);
    assert_eq!(held, first);
    assert!(changes.is_empty());
    let mut f = frame(&[]);
    f.swim_hold = Some((State::UnderwaterSwimming, signal(State::UnderwaterSwimming, now)));
    let (ready, changes) = runtime.update(Some(&set), f, now + Duration::from_millis(60), |_| true, |_| 0);
    assert_eq!(ready[0].id, Uuid::from_u128(25003));
    assert!(changes.contains(&(old.id, false)) && changes.contains(&(ready[0].id, true)));
}

#[test]
fn old_sets_share_the_entire_surface_list_until_an_underwater_list_is_assigned() {
    let (_, mut set) = fixture();
    set.states.remove(&State::UnderwaterSwimming);
    let c = set.states.get_mut(&State::SwimmingForward).expect("surface");
    c.steps[0].group = true;
    let extra = c.steps[0].members[0].clone();
    c.steps[0].members.push(extra);
    c.steps[0].members[0].asset = Uuid::from_u128(26001);
    c.steps[0].members[1].asset = Uuid::from_u128(26002);
    assert!(set.swimming_ready());
    assert_eq!(
        set.animation_config(State::UnderwaterSwimming),
        set.states.get(&State::SwimmingForward)
    );
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame(&[]);
    f.swim_hold = Some((State::UnderwaterSwimming, signal(State::UnderwaterSwimming, now)));
    let (pose, changes) = runtime.update(Some(&set), f, now, |_| true, |_| 0);
    assert_eq!(pose.len(), 2);
    assert_eq!(runtime.current, Some(State::UnderwaterSwimming));
    assert!(changes.contains(&(Uuid::from_u128(26001), true)) && changes.contains(&(Uuid::from_u128(26002), true)));
    // Ordinary simulator flight below water uses the same fallback even
    // without an active bridge swimming context.
    let mut runtime = Runtime::default();
    let raw = [signal(State::Flying, now)];
    let mut f = frame(&raw);
    f.underwater = true;
    let (pose, _) = runtime.update(Some(&set), f, now, |_| true, |_| 0);
    assert_eq!(runtime.current, Some(State::UnderwaterSwimming));
    assert!(pose.iter().any(|a| a.id == Uuid::from_u128(26001)));
}

fn apply(server: &mut BTreeMap<Uuid, PlayingAnimation>, changes: &[(Uuid, bool)], now: Instant) {
    for &(id, start) in changes {
        if start {
            let old = server.get(&id);
            server.insert(id, PlayingAnimation::from_signal(id, 1, now, old));
        } else {
            server.remove(&id);
        }
    }
}

#[test]
fn default_stop_echoes_keep_the_ao_state_and_observers_receive_the_same_owned_selection() {
    let (_, mut set) = fixture();
    unique(&mut set, State::Walking, 27001);
    unique(&mut set, State::Standing, 27002);
    let now = Instant::now();
    let walk = signal(State::Walking, now);
    let gesture = PlayingAnimation::from_signal(Uuid::from_u128(27099), 1, now, None);
    let mut server = BTreeMap::from([(walk.id, walk), (gesture.id, gesture)]);
    let mut runtime = Runtime::default();
    let raw = server.values().copied().collect::<Vec<_>>();
    let (pose, changes) = runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert!(
        changes.contains(&(walk.id, false)),
        "a low-priority AO need not compete with the stock walk"
    );
    apply(&mut server, &changes, now);
    assert_eq!(
        pose.iter().map(|a| a.id).collect::<std::collections::BTreeSet<_>>(),
        server.keys().copied().collect()
    );
    for i in 1..10 {
        let raw = server.values().copied().collect::<Vec<_>>();
        let (pose, changes) = runtime.update(Some(&set), frame(&raw), now + Duration::from_secs(i), |_| true, |_| 0);
        assert_eq!(runtime.current, Some(State::Walking));
        assert!(changes.is_empty());
        assert_eq!(
            pose.iter().map(|a| a.id).collect::<std::collections::BTreeSet<_>>(),
            server.keys().copied().collect()
        );
    }
    let stand = signal(State::Standing, now + Duration::from_secs(10));
    server.insert(stand.id, stand);
    let raw = server.values().copied().collect::<Vec<_>>();
    let (_, changes) = runtime.update(Some(&set), frame(&raw), now + Duration::from_secs(10), |_| true, |_| 0);
    assert_eq!(runtime.current, Some(State::Standing));
    apply(&mut server, &changes, now);
    let raw = server.values().copied().collect::<Vec<_>>();
    let mut f = frame(&raw);
    f.enabled = false;
    let (pose, changes) = runtime.update(Some(&set), f, now + Duration::from_secs(11), |_| true, |_| 0);
    assert!(changes.contains(&(stand.id, true)) && !changes.contains(&(walk.id, true)));
    apply(&mut server, &changes, now);
    assert_eq!(
        pose.iter().map(|a| a.id).collect::<std::collections::BTreeSet<_>>(),
        server.keys().copied().collect()
    );
    assert!(server.contains_key(&gesture.id));
}

#[test]
fn default_reassertion_does_not_restart_the_cycle_and_jump_control_motions_survive() {
    let (_, set) = fixture();
    let now = Instant::now();
    let walk = signal(State::Walking, now);
    let mut runtime = Runtime::default();
    let (first, _) = runtime.update(Some(&set), frame(&[walk]), now, |_| true, |_| 0);
    let later = PlayingAnimation::from_signal(walk.id, 2, now + Duration::from_secs(1), None);
    let (pose, changes) = runtime.update(Some(&set), frame(&[later]), now + Duration::from_secs(1), |_| true, |_| 0);
    assert!(changes.contains(&(walk.id, false)));
    assert!(!changes.iter().any(|(_, start)| *start));
    assert_eq!(first, pose);
    let jump = signal(State::Jumping, now + Duration::from_secs(2));
    let (pose, changes) = runtime.update(Some(&set), frame(&[jump]), now + Duration::from_secs(2), |_| true, |_| 0);
    // The AO jump replaces the stock one locally (#1284); the simulator keeps
    // its jump control motion, as in the Legacy AO.
    assert!(pose.iter().any(|a| a.id == step(&set, State::Jumping, 0)));
    assert!(!pose.contains(&jump));
    assert!(!changes.contains(&(jump.id, false)));
}

#[test]
fn loop_echo_repair_is_bounded_and_never_replays_a_finished_one_shot() {
    let (_, set) = fixture();
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let stand = signal(State::Standing, now);
    let (pose, _) = runtime.update(Some(&set), frame(&[stand]), now, |_| true, |_| 0);
    let id = pose.last().expect("AO").id;
    assert!(runtime.reconcile_loops(&[], now + Duration::from_secs(3), |_| false).is_empty());
    assert_eq!(
        runtime.reconcile_loops(&[], now + Duration::from_secs(3), |_| true),
        vec![(id, true)]
    );
    assert!(runtime.reconcile_loops(&[], now + Duration::from_millis(3500), |_| true).is_empty());
    assert!(runtime.reconcile_loops(&pose, now + Duration::from_secs(7), |_| true).is_empty());
    runtime.finished(id);
    assert!(runtime.reconcile_loops(&[], now + Duration::from_secs(10), |_| true).is_empty());
}

#[test]
fn ao_prefetch_is_bounded_deduplicated_and_refreshes_after_assignment_changes() {
    let (_, set) = fixture();
    let mut runtime = Runtime::default();
    let mut requested = Vec::new();
    for _ in 0..100 {
        let before = requested.len();
        runtime.prefetch(Some(&set), 1, |id| requested.push(*id));
        assert!(requested.len() - before <= 4);
    }
    assert!(!requested.is_empty());
    let unique = requested.iter().copied().collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), requested.len());
    let before = requested.len();
    runtime.prefetch(Some(&set), 1, |id| requested.push(*id));
    assert_eq!(requested.len(), before);
    runtime.prefetch(Some(&set), 2, |id| requested.push(*id));
    assert!(requested.len() > before);
}

#[test]
fn yielding_to_a_scripted_seat_does_not_restart_the_stock_sit_over_its_pose() {
    let (_, mut set) = fixture();
    unique(&mut set, State::Sitting, 28001);
    let now = Instant::now();
    let sit = signal(State::Sitting, now);
    let mut runtime = Runtime::default();
    let (_, changes) = runtime.update(Some(&set), frame(&[sit]), now, |_| true, |_| 0);
    assert!(changes.contains(&(sit.id, false)));
    let chair = PlayingAnimation::from_signal(Uuid::from_u128(28002), 1, now + Duration::from_millis(20), None);
    let raw = [chair];
    let mut f = frame(&raw);
    f.on_object = true;
    f.seat_animations = true;
    let (pose, changes) = runtime.update(Some(&set), f, now + Duration::from_millis(20), |_| true, |_| 0);
    assert!(!changes.contains(&(sit.id, true)));
    assert_eq!(pose, raw);
    let mut runtime = Runtime::default();
    let walk = signal(State::Walking, now);
    runtime.update(Some(&set), frame(&[walk]), now, |_| true, |_| 0);
    let mut f = frame(&raw);
    f.on_object = true;
    f.seat_animations = true;
    let (pose, changes) = runtime.update(Some(&set), f, now + Duration::from_millis(20), |_| true, |_| 0);
    assert_eq!(pose, raw);
    assert!(!changes.contains(&(walk.id, true)));
}

#[test]
fn disabling_then_reenabling_before_the_echo_keeps_one_ao_and_no_stale_default_pose() {
    let (_, set) = fixture();
    let now = Instant::now();
    let walk = signal(State::Walking, now);
    let mut runtime = Runtime::default();
    let (pose, _) = runtime.update(Some(&set), frame(&[walk]), now, |_| true, |_| 0);
    let raw = pose;
    let mut f = frame(&raw);
    f.enabled = false;
    runtime.update(Some(&set), f, now + Duration::from_millis(10), |_| true, |_| 0);
    let (pose, changes) = runtime.update(Some(&set), frame(&raw), now + Duration::from_millis(20), |_| true, |_| 0);
    assert!(!pose.iter().any(|a| a.id == walk.id));
    assert!(changes.contains(&(walk.id, false)));
    let (later, changes) = runtime.update(Some(&set), frame(&raw), now + Duration::from_secs(3), |_| true, |_| 0);
    assert_eq!(runtime.current, Some(State::Walking));
    assert_eq!(pose, later);
    assert!(changes.is_empty());
}
