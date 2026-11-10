use super::*;
use crate::world::{PlayingAnimation, inventory::Inventory};
use runtime::{Frame, Runtime};
use std::time::{Duration, Instant};

pub(super) fn fixture() -> (Inventory, Set) {
    let mut inv = Inventory::default();
    let id = demo::seed(&mut inv, Uuid::from_u128(1));
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    let set = catalog.selected(id).expect("demo AO set").clone();
    (inv, set)
}
pub(super) fn frame(raw: &[PlayingAnimation]) -> Frame<'_> {
    Frame {
        raw,
        run_hold: None,
        swim_hold: None,
        water_walk_hold: None,
        anchor_hold: None,
        swim_outro: false,
        native_running: false,
        enabled: true,
        mouselook: false,
        on_object: false,
        seat_animations: false,
        underwater: false,
        region: Some(1),
    }
}
pub(super) fn signal(state: State, now: Instant) -> PlayingAnimation {
    PlayingAnimation::from_signal(state.motion(), 1, now, None)
}
pub(super) fn step(set: &Set, state: State, index: usize) -> Uuid {
    set.states[&state].steps[index].members[0].asset
}

#[test]
fn ao_choices_survive_settings_restart_reordering_groups_tracks_and_inactive_states() {
    let (_, mut set) = fixture();
    let config = set.states.get_mut(&State::Standing).expect("standing");
    config.cycle = Cycle::default();
    config.steps[1].group = true;
    let member = config.steps[0].members[0].clone();
    config.steps[1].members.push(member);
    let track_id = Uuid::new_v4();
    config.tracks.push(Track {
        id: track_id,
        name: "Independent".into(),
        cycle: Cycle::default(),
        members: config.steps.iter().map(|s| s.members[0].clone()).collect(),
    });
    let standing_folder = config.folder;
    let group = config.steps[1].id;
    let track_link = config.tracks[0].members[1].link;
    let walking = set.states.get_mut(&State::Walking).expect("walking");
    let mut extra = walking.steps[0].clone();
    extra.id = Uuid::new_v4();
    walking.steps.push(extra);
    let walking_folder = walking.folder;
    let walk = walking.steps[1].id;
    let now = Instant::now();
    let raw = [signal(State::Standing, now)];
    let mut runtime = Runtime::default();
    runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    runtime.choose(State::Standing, standing_folder, 1);
    runtime.choose(State::Standing, track_id, 1);
    runtime.choose(State::Walking, walking_folder, 1);
    let account_id = Uuid::new_v4();
    let mut settings = crate::settings::Settings::default();
    settings.ao.accounts.insert(
        account_id,
        Account {
            enabled: true,
            selected: set.id,
            mini_open: true,
            animations: runtime.take_choices(Some(&set)).expect("new selection"),
        },
    );
    assert!(
        runtime.take_choices(Some(&set)).is_none(),
        "unchanged frames never queue another save"
    );
    let saved = serde_json::to_string(&settings).expect("settings");
    let restored: crate::settings::Settings = serde_json::from_str(&saved).expect("restart");
    let account = &restored.ao.accounts[&account_id];
    assert_eq!(account.animations[&standing_folder], group);
    assert_eq!(account.animations[&track_id], track_link);
    assert_eq!(account.animations[&walking_folder], walk);
    assert!(account.mini_open, "the mini window preference survives a restart");
    let standing = set.states.get_mut(&State::Standing).expect("standing");
    standing.steps.swap(0, 1);
    standing.tracks[0].members.swap(0, 1);
    let mut runtime = Runtime::with_choices(account.animations.clone());
    let (pose, _) = runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert_eq!(runtime.index(State::Standing, standing_folder), 0, "group restored by identity");
    assert_eq!(runtime.index(State::Standing, track_id), 0, "track restored by link");
    assert_eq!(runtime.index(State::Walking, walking_folder), 1, "inactive choice restored too");
    for animation in &set.states[&State::Standing].steps[0].members {
        assert!(pose.iter().any(|a| a.id == animation.asset), "whole group still plays");
    }
}

#[test]
fn ao_saved_choice_is_used_before_cycling_and_survives_set_switches_and_loading() {
    let (_, mut set) = fixture();
    let state = State::Standing;
    let config = set.states.get_mut(&state).expect("standing");
    config.cycle = Cycle {
        cycle: true,
        random: false,
        seconds: 5.0,
    };
    let folder = config.folder;
    let chosen = config.steps[0].id;
    let mut runtime = Runtime::with_choices(HashMap::from([(folder, chosen)]));
    let now = Instant::now();
    let raw = [signal(state, now)];
    let (pose, _) = runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert!(
        pose.iter().any(|a| a.id == step(&set, state, 0)),
        "first frame retains the saved stand"
    );
    runtime.update(Some(&set), frame(&raw), now + Duration::from_secs(6), |_| true, |_| 0);
    let choices = runtime.take_choices(Some(&set)).expect("timed cycle changed choice");
    assert_eq!(choices[&folder], set.states[&state].steps[1].id);
    let other_set = Set::new("Another set".into());
    runtime.update(Some(&other_set), frame(&raw), now, |_| true, |_| 0);
    runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert_eq!(runtime.index(state, folder), 1, "set re-entry retains the latest choice");
    set.states.get_mut(&state).expect("standing").steps.swap(0, 1);
    runtime.prefetch(Some(&set), 1, |_| {});
    runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert_eq!(runtime.index(state, folder), 0, "live reordering also follows the saved identity");
    runtime.choose(state, folder, 1);
    runtime.prefetch(Some(&set), 2, |_| {});
    assert_eq!(
        runtime.index(state, folder),
        1,
        "inventory refresh preserves a pending editor choice"
    );
    set.states.get_mut(&state).expect("standing").steps.swap(0, 1);
    let mut incomplete = set.clone();
    incomplete.states.clear();
    let mut runtime = Runtime::with_choices(choices);
    runtime.update(Some(&incomplete), frame(&raw), now, |_| true, |_| 0);
    runtime.prefetch(Some(&set), 2, |_| {});
    runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert_eq!(runtime.index(state, folder), 1, "restore after inventory finishes loading");
}

#[test]
fn ao_missing_saved_animation_falls_back_and_old_account_settings_still_load() {
    let (_, mut set) = fixture();
    let config = set.states.get_mut(&State::Standing).expect("standing");
    config.cycle = Cycle::default();
    let folder = config.folder;
    let mut runtime = Runtime::with_choices(HashMap::from([(folder, Uuid::new_v4())]));
    let now = Instant::now();
    let raw = [signal(State::Standing, now)];
    let (pose, _) = runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert!(pose.iter().any(|a| a.id == step(&set, State::Standing, 0)));
    let account: Account = serde_json::from_str(r#"{"enabled":true}"#).expect("old account");
    assert!(account.enabled && account.animations.is_empty());
}

#[test]
fn stock_jump_covers_the_pose_until_the_ao_jump_is_loaded() {
    let (_, set) = fixture();
    let asset = step(&set, State::Jumping, 0);
    let now = Instant::now();
    let raw = [signal(State::Jumping, now)];
    let mut r = Runtime::default();
    let (pose, commands) = r.update(Some(&set), frame(&raw), now, |id| *id != asset, |_| 0);
    assert!(pose.contains(&raw[0]));
    assert!(commands.contains(&(asset, true)));
    let (pose, commands) = r.update(Some(&set), frame(&raw), now + Duration::from_millis(100), |_| true, |_| 0);
    assert!(pose.iter().any(|a| a.id == asset));
    assert!(!pose.contains(&raw[0]));
    assert!(!commands.iter().any(|(id, _)| *id == raw[0].id));
}

#[test]
fn native_movelock_uses_standing_cycles_without_stopping_the_constrained_sit() {
    let (_, set) = fixture();
    let mut r = Runtime::default();
    let now = Instant::now();
    let raw = [signal(State::GroundSit, now)];
    for elapsed in [0, 7, 14] {
        let mut f = frame(&raw);
        f.anchor_hold = Some(signal(State::Standing, now));
        let (pose, changes) = r.update(Some(&set), f, now + Duration::from_secs(elapsed), |_| true, |_| 0);
        assert_eq!(r.current, Some(State::Standing));
        assert!(pose.iter().any(|a| {
            set.states[&State::Standing]
                .steps
                .iter()
                .any(|s| s.members.iter().any(|m| m.asset == a.id))
        }));
        assert!(!pose.contains(&raw[0]));
        assert!(!changes.iter().any(|(id, _)| *id == State::GroundSit.motion()));
    }
    let raw = [signal(State::Walking, now + Duration::from_secs(15))];
    let (_, changes) = r.update(Some(&set), frame(&raw), now + Duration::from_secs(15), |_| true, |_| 0);
    assert_eq!(r.current, Some(State::Walking));
    assert!(changes.contains(&(step(&set, State::Walking, 0), true)));
}

#[test]
fn native_movelock_idle_waits_for_coasting_then_prepares_before_sitting() {
    use crate::movement::movelock::{Controller, Frame as LockFrame};
    for state in [State::Walking, State::Running] {
        let (_, set) = fixture();
        let t = Instant::now();
        let mut lock = Controller::default();
        let mut ao = Runtime::default();
        let mut echoes: Vec<PlayingAnimation> = Vec::new();
        let mut seated = false;
        for ms in [0, 100, 300, 700, 950, 1000, 1100, 1400, 1600, 1800, 2000] {
            let now = t + Duration::from_millis(ms);
            let moving = ms < 1000;
            let command = lock.update(
                LockFrame {
                    requested: true,
                    region: Some(1),
                    available: true,
                    voluntary: ms == 0,
                    moving,
                    travel: crate::movement::movelock::Travel::Ready,
                    pose_ready: true,
                    pose_warm: ao.anchor_prepared(Some(1)),
                    ground_sit: seated,
                    on_object: false,
                },
                now,
            );
            if command == Some(aurora_net::control::SIT_ON_GROUND) {
                assert!(ms >= 1800, "sitting still waits for echoed/eased standing coverage");
                seated = true;
            }
            // Even a late locomotion signal cannot preempt the prepared sit.
            let mut raw = vec![signal(if seated { State::GroundSit } else { state }, t)];
            raw.extend_from_slice(&echoes);
            let mut f = frame(&raw);
            f.anchor_hold = lock.standing_since(seated).map(|since| signal(State::Standing, since));
            let (pose, changes) = ao.update_with_easing(Some(&set), f, now, |_| true, |_| Some(Duration::from_millis(400)), |_| 0);
            if moving {
                assert_eq!(ao.current, Some(state), "retain locomotion after key release at {ms} ms");
                assert!(pose.iter().any(|a| a.id == step(&set, state, 0)));
                assert!(
                    !set.states[&State::Standing]
                        .steps
                        .iter()
                        .any(|s| { s.members.iter().any(|m| changes.contains(&(m.asset, true))) }),
                    "no premature standing broadcast to observers"
                );
                assert!(!seated);
            } else {
                assert_eq!(ao.current, Some(State::Standing));
            }
            for (id, start) in changes {
                if State::from_motion(id).is_some() {
                    continue;
                }
                if start && !echoes.iter().any(|a| a.id == id) {
                    echoes.push(PlayingAnimation::from_signal(id, 1, now, None));
                } else if !start {
                    echoes.retain(|a| a.id != id);
                }
            }
        }
        assert!(seated);
    }
}

#[test]
fn native_movelock_keeps_verified_standing_until_stand_up_is_acknowledged() {
    let (_, mut set) = fixture();
    set.mouselook = true;
    let now = Instant::now();
    let raw = [signal(State::GroundSit, now)];
    let mut r = Runtime::default();
    let mut f = frame(&raw);
    f.anchor_hold = Some(signal(State::Standing, now));
    r.update(Some(&set), f, now, |_| true, |_| 0);
    r.paused = true;
    r.stands = false;
    let mut f = frame(&raw);
    f.anchor_hold = Some(signal(State::Standing, now));
    f.enabled = false;
    f.mouselook = true;
    let (pose, changes) = r.update(Some(&set), f, now + Duration::from_millis(50), |_| true, |_| 0);
    assert!(pose.iter().any(|a| {
        set.states[&State::Standing]
            .steps
            .iter()
            .any(|s| s.members.iter().any(|m| m.asset == a.id))
    }));
    assert!(!pose.contains(&raw[0]));
    assert!(changes.is_empty(), "no restart or system sit stop while waiting for stand-up");
    let mut f = frame(&[]);
    f.enabled = false;
    let (pose, changes) = r.update(Some(&set), f, now + Duration::from_millis(100), |_| true, |_| 0);
    assert!(pose.is_empty());
    assert!(changes.iter().any(|(_, start)| !start));
}

#[test]
fn native_movelock_preserves_the_stand_cycle_and_broadcasts_walk_before_stopping_its_cover() {
    let (_, set) = fixture();
    let t = Instant::now();
    let at = |ms| t + Duration::from_millis(ms);
    let mut r = Runtime::default();
    let stock = signal(State::Standing, t);
    let (_, changes) = r.update_with_easing(
        Some(&set),
        frame(&[stock]),
        t,
        |_| true,
        |_| Some(Duration::from_millis(400)),
        |_| 0,
    );
    let stand = changes.iter().find(|(_, start)| *start).expect("stand start").0;
    let echo = PlayingAnimation::from_signal(stand, 1, at(100), None);
    let raw = [stock, echo];
    let mut f = frame(&raw);
    f.anchor_hold = Some(signal(State::Standing, at(1000)));
    let (_, changes) = r.update_with_easing(Some(&set), f, at(1000), |_| true, |_| Some(Duration::from_millis(400)), |_| 0);
    assert!(r.anchor_prepared(Some(1)));
    assert!(!r.anchor_prepared(Some(2)), "a different region cannot reuse the old preparation");
    assert!(changes.is_empty(), "taking ownership must not cycle or restart the existing stand");
    let sit = signal(State::GroundSit, at(1250));
    let raw = [sit, echo];
    let mut f = frame(&raw);
    f.anchor_hold = Some(signal(State::Standing, at(1000)));
    let (_, changes) = r.update_with_easing(Some(&set), f, at(1250), |_| true, |_| Some(Duration::from_millis(400)), |_| 0);
    assert!(changes.is_empty(), "the native sit echo must not change the standing group");
    let walking = signal(State::Walking, at(2000));
    let (pose, changes) = r.update_with_easing(
        Some(&set),
        frame(&[walking, echo]),
        at(2000),
        |_| true,
        |_| Some(Duration::from_millis(800)),
        |_| 0,
    );
    let walk = step(&set, State::Walking, 0);
    assert!(changes.contains(&(walk, true)));
    assert!(!changes.contains(&(stand, false)));
    assert!(
        pose.iter().any(|a| a.id == stand),
        "the observer and owner retain the same full cover"
    );
    let walk_echo = PlayingAnimation::from_signal(walk, 1, at(2100), None);
    let (_, changes) = r.update_with_easing(
        Some(&set),
        frame(&[walking, echo, walk_echo]),
        at(3049),
        |_| true,
        |_| Some(Duration::from_millis(800)),
        |_| 0,
    );
    assert!(
        !changes.contains(&(stand, false)),
        "wait for the incoming clip's complete easing after its echo"
    );
    let (pose, changes) = r.update_with_easing(
        Some(&set),
        frame(&[walking, echo, walk_echo]),
        at(3050),
        |_| true,
        |_| Some(Duration::from_millis(800)),
        |_| 0,
    );
    assert!(changes.contains(&(stand, false)));
    assert!(!pose.iter().any(|a| a.id == stand));
    assert!(pose.iter().any(|a| a.id == walk));
}

#[test]
fn water_walk_uses_ordinary_ao_states_and_broadcasts_their_replacements() {
    let (_, set) = fixture();
    let now = Instant::now();
    let mut r = Runtime::default();
    for state in [State::Standing, State::Walking, State::Running, State::Crouching, State::CrouchWalk] {
        let fall = signal(State::Falling, now);
        let mut f = frame(std::slice::from_ref(&fall));
        f.water_walk_hold = Some((state, signal(state, now)));
        let (pose, commands) = r.update(Some(&set), f, now, |_| true, |_| 0);
        // The demo stand list cycles on entry, just like an ordinary stand.
        let asset = step(&set, state, if state == State::Standing { 1 } else { 0 });
        assert_eq!(r.current, Some(state));
        assert!(pose.iter().any(|a| a.id == asset));
        assert!(!pose.contains(&fall));
        assert!(commands.contains(&(asset, true)));
    }
}

#[test]
fn water_walk_without_ao_owns_stock_states_then_releases_them_immediately() {
    let now = Instant::now();
    for state in [
        State::Standing,
        State::Walking,
        State::Running,
        State::Crouching,
        State::CrouchWalk,
        State::Jumping,
    ] {
        let mut r = Runtime::default();
        let fall = signal(State::Falling, now);
        let stock = signal(state, now);
        let mut f = frame(std::slice::from_ref(&fall));
        f.enabled = false;
        f.water_walk_hold = Some((state, stock));
        let (pose, commands) = r.update(None, f, now, |_| true, |_| 0);
        assert!(pose.iter().any(|a| a.id == stock.id));
        assert!(!pose.contains(&fall));
        assert!(commands.contains(&(stock.id, true)));
        let mut f = frame(std::slice::from_ref(&stock));
        f.enabled = false;
        let (pose, commands) = r.update(None, f, now + Duration::from_millis(50), |_| true, |_| 0);
        assert!(pose.contains(&stock), "normal simulator locomotion resumes immediately");
        assert!(commands.contains(&(stock.id, false)));
    }
}

#[test]
fn water_walk_jump_fall_replacements_and_stock_fallback_release_native_signals() {
    let (_, mut set) = fixture();
    let now = Instant::now();
    for state in [State::Jumping, State::Falling] {
        // Give both airborne states an ordinary inventory replacement.
        set.states.insert(state, set.states[&State::Running].clone());
        for enabled in [false, true] {
            let mut r = Runtime::default();
            let raw = [signal(State::Walking, now)];
            let mut f = frame(&raw);
            f.enabled = enabled;
            f.water_walk_hold = Some((state, signal(state, now)));
            let (pose, commands) = r.update(Some(&set), f, now, |_| true, |_| 0);
            let asset = if enabled { step(&set, state, 0) } else { state.motion() };
            assert!(pose.iter().any(|a| a.id == asset));
            assert!(commands.contains(&(asset, true)));
            assert!(!pose.contains(&raw[0]));
            let native = [signal(state, now + Duration::from_millis(50))];
            let mut f = frame(&native);
            f.enabled = false;
            let (pose, commands) = r.update(None, f, now + Duration::from_millis(50), |_| true, |_| 0);
            assert!(pose.contains(&native[0]), "{state:?} resumes after releasing the water hold");
            assert!(commands.contains(&(asset, false)));
        }
    }
}

#[test]
fn water_walk_jump_suppresses_native_fall_through_descent_and_releases_on_contact_or_disable() {
    let (_, mut set) = fixture();
    set.states.insert(State::Jumping, set.states[&State::Running].clone());
    let now = Instant::now();
    for enabled in [false, true] {
        for land in [false, true] {
            let mut movement = crate::movement::water_walk::Runtime::default();
            let mut physical = crate::movement::Frame {
                position: glam::Vec3::new(40.0, 124.0, 20.9),
                velocity: glam::Vec3::ZERO,
                height: 1.8,
                water: Some(20.0),
                water_contact: None,
                ground: Some(16.0),
                flying: false,
                can_fly: false,
                seated: false,
                suspended: false,
                locked: false,
                bridge_ready: true,
                running: false,
                obstacle: false,
                landing: None,
                on_ground: false,
                jumping: false,
            };
            let input = crate::agent::MoveInput {
                up: true,
                ..Default::default()
            };
            movement.update(true, &physical, &crate::agent::MoveInput::default(), now);
            let mut ao = Runtime::default();
            let asset = if enabled {
                step(&set, State::Jumping, 0)
            } else {
                State::Jumping.motion()
            };
            for (ms, lift, velocity) in [
                (0, 0.0, 8.0),
                (125, 0.9, 6.4),
                (625, 2.5, 0.0),
                (1000, 1.6, -4.8),
                (1125, 0.9, -6.4),
            ] {
                let time = now + Duration::from_millis(ms);
                physical.position.z = 20.9 + lift;
                physical.velocity.z = velocity;
                movement.update(true, &physical, &input, time);
                let hold = movement.signal(time).expect("jump owns the entire supported arc");
                assert_eq!(hold.0, State::Jumping);
                assert_eq!(hold.1.continuous_start, now);
                // Repeated simulator fall signals must stay hidden locally
                // and stopped for observers, including below the apex.
                let raw = [signal(State::Falling, time)];
                let mut f = frame(&raw);
                f.enabled = enabled;
                f.water_walk_hold = Some(hold);
                let (pose, commands) = ao.update(Some(&set), f, time, |_| true, |_| 0);
                let jump = pose.iter().find(|a| a.id == asset).expect("jump pose");
                assert_eq!(jump.continuous_start, now);
                assert!(!pose.contains(&raw[0]));
                assert!(!commands.contains(&(raw[0].id, true)));
                if ms == 0 {
                    assert!(commands.contains(&(raw[0].id, false)));
                }
            }
            let time = now + Duration::from_millis(1400);
            if land {
                physical.position.z = 20.9;
                physical.velocity.z = 0.0;
            }
            movement.update(land, &physical, &input, time);
            let hold = movement.signal(time);
            assert_eq!(hold.map(|(s, _)| s), land.then_some(State::Standing));
            let raw = [signal(State::Falling, time)];
            let mut f = frame(&raw);
            f.enabled = false;
            f.water_walk_hold = hold;
            let (pose, commands) = ao.update(Some(&set), f, time, |_| true, |_| 0);
            assert!(!pose.iter().any(|a| a.id == asset));
            assert!(commands.contains(&(asset, false)));
            if land {
                assert!(pose.iter().any(|a| a.id == State::Standing.motion()));
                assert!(!pose.contains(&raw[0]));
            } else {
                assert!(pose.contains(&raw[0]), "ordinary falling resumes after disabling water support");
                assert!(commands.contains(&(raw[0].id, true)));
            }
        }
    }
}

#[test]
fn a_standalone_cannonball_slot_works_with_regular_ao_and_swimming_disabled() {
    let (_, mut set) = fixture();
    let config = set.states[&State::Running].clone();
    let asset = config.steps[0].members[0].asset;
    set.states.clear();
    set.states.insert(State::WaterCannonball, config);
    assert!(!set.swimming_ready());
    let now = Instant::now();
    let fall = signal(State::Falling, now);
    let mut f = frame(std::slice::from_ref(&fall));
    f.enabled = false;
    f.swim_hold = Some((State::WaterCannonball, signal(State::WaterCannonball, now)));
    let (pose, commands) = Runtime::default().update(Some(&set), f, now, |_| true, |_| 0);
    assert!(pose.iter().any(|a| a.id == asset));
    assert!(!pose.contains(&fall));
    assert!(commands.contains(&(asset, true)));
}

#[test]
fn enhanced_run_keeps_the_selected_ao_course_across_brief_fall_signals() {
    let (_, set) = fixture();
    let now = Instant::now();
    let run = signal(State::Running, now);
    let raw = [run];
    let mut runtime = Runtime::default();
    let mut f = frame(&raw);
    f.run_hold = Some(run);
    runtime.update(Some(&set), f, now, |_| true, |_| 0);
    let asset = step(&set, State::Running, 0);
    let gesture = PlayingAnimation::from_signal(Uuid::from_u128(907), 1, now, None);
    let fall = signal(State::Falling, now + Duration::from_millis(50));
    let land = signal(State::Landing, now + Duration::from_millis(60));
    let raw = [fall, land, gesture];
    let mut f = frame(&raw);
    f.run_hold = Some(run);
    let (pose, commands) = runtime.update(Some(&set), f, now + Duration::from_millis(100), |_| true, |_| 0);
    assert_eq!(runtime.current, Some(State::Running));
    assert!(pose.iter().any(|a| a.id == asset));
    assert!(!pose.iter().any(|a| a.id == fall.id));
    assert!(pose.contains(&gesture));
    assert!(pose.contains(&land), "landing must still finish simulator-controlled transitions");
    assert!(!commands.contains(&(asset, false)));
    assert_eq!(raw[0], fall, "the authoritative raw signals stay unchanged");
    runtime.update(Some(&set), frame(&[fall]), now + Duration::from_millis(350), |_| true, |_| 0);
    assert_eq!(runtime.current, Some(State::Falling));
}

#[test]
fn enhanced_run_without_aurora_ao_broadcasts_the_standard_course_then_releases_it() {
    let now = Instant::now();
    let run = signal(State::Running, now);
    let fall = signal(State::Falling, now);
    let gesture = PlayingAnimation::from_signal(Uuid::from_u128(908), 1, now, None);
    let raw = [fall, gesture];
    let mut runtime = Runtime::default();
    let mut f = frame(&raw);
    f.enabled = false;
    f.run_hold = Some(run);
    let (pose, commands) = runtime.update(None, f, now, |_| true, |_| 0);
    assert!(pose.iter().any(|a| a.id == run.id));
    assert!(!pose.contains(&fall));
    assert!(pose.contains(&gesture));
    assert!(commands.contains(&(run.id, true)));
    assert!(!commands.iter().any(|(id, _)| *id == gesture.id));
    let mut f = frame(&raw);
    f.enabled = false;
    let (pose, commands) = runtime.update(None, f, now + Duration::from_millis(300), |_| true, |_| 0);
    assert!(pose.contains(&fall));
    assert!(commands.contains(&(run.id, false)));
}

#[test]
fn disabling_the_boost_during_native_running_does_not_stop_its_stock_course() {
    let now = Instant::now();
    let run = signal(State::Running, now);
    let raw = [run];
    let mut runtime = Runtime::default();
    let mut f = frame(&raw);
    f.enabled = false;
    f.run_hold = Some(run);
    runtime.update(None, f, now, |_| true, |_| 0);
    let mut f = frame(&raw);
    f.enabled = false;
    f.native_running = true;
    let (pose, commands) = runtime.update(None, f, now + Duration::from_millis(100), |_| true, |_| 0);
    assert!(pose.iter().any(|a| a.id == run.id));
    assert!(!commands.contains(&(run.id, false)));
    let mut f = frame(&[]);
    f.enabled = false;
    let (_, commands) = runtime.update(None, f, now + Duration::from_millis(200), |_| true, |_| 0);
    assert!(commands.contains(&(run.id, false)));
    let mut f = frame(&raw);
    f.enabled = false;
    f.native_running = true;
    let (pose, commands) = runtime.update(None, f, now + Duration::from_millis(300), |_| true, |_| 0);
    assert!(
        pose.contains(&run),
        "a later native run must not be mistaken for an old override echo"
    );
    assert!(commands.is_empty());
}

#[test]
fn enhanced_run_uses_stock_fallback_until_the_ao_asset_loads_and_reasserts_only_on_a_new_fall() {
    let (_, set) = fixture();
    let now = Instant::now();
    let run = signal(State::Running, now);
    let asset = step(&set, State::Running, 0);
    let mut runtime = Runtime::default();
    let mut f = frame(&[]);
    f.run_hold = Some(run);
    let (pose, commands) = runtime.update(Some(&set), f, now, |id| *id == run.id, |_| 0);
    assert!(pose.iter().any(|a| a.id == run.id));
    assert!(commands.contains(&(run.id, true)));
    let fall = signal(State::Falling, now + Duration::from_millis(80));
    let raw = [fall];
    let mut f = frame(&raw);
    f.run_hold = Some(run);
    let (pose, commands) = runtime.update(Some(&set), f, now + Duration::from_millis(100), |_| true, |_| 0);
    assert!(pose.iter().any(|a| a.id == asset));
    assert!(commands.contains(&(run.id, false)));
    assert!(commands.contains(&(asset, true)));
}

#[test]
fn inventory_round_trip_preserves_firestorm_options_and_originals() {
    let (mut inv, mut set) = fixture();
    assert!(set.default && set.sits && set.smart);
    let original = set.states[&State::Standing].steps[0].members[0].original;
    let original_item = inv.items[&original].clone();
    set.name = "Renamed".into();
    set.mouselook = true;
    let change = inventory::save_set(&inv, &set).expect("rename");
    crate::world::inventory::actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("mutation");
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    assert_eq!(catalog.selected(set.id).expect("reloaded"), &set);
    let link = set.states[&State::Standing].steps[0].id;
    let change = inventory::remove(&inv, link).expect("remove link");
    crate::world::inventory::actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("mutation");
    assert!(!inv.items.contains_key(&link));
    assert_eq!(inv.items[&original].asset_id, original_item.asset_id);
}

#[test]
fn import_aliases_groups_and_invalid_lines_without_losing_valid_entries() {
    let (inv, _) = fixture();
    let note =
        "# comment\n[Stand.1]Aurora Stand A, Aurora Type|Aurora Stand B\n[Walk.N]Aurora Walk\n[Bad]Nope\n[Running]Missing|Aurora Run\n";
    let (set, warnings) = inventory::import(&inv, demo::NOTECARD, note).expect("import");
    assert_eq!(warnings.len(), 2);
    assert!(set.states[&State::Standing].steps[0].group);
    assert_eq!(set.states[&State::Standing].steps[0].members.len(), 2);
    assert_eq!(set.states[&State::Standing].steps.len(), 2);
    assert_eq!(set.states[&State::Walking].steps.len(), 1);
    assert!(inventory::import(&inv, demo::NOTECARD, "[Unknown]nothing").is_err());
    assert!(inventory::valid_name("Bad:name").is_err());
    assert_eq!(Cycle::parse(&["CTNaN", "CY"]).seconds, 0.0);
}

#[test]
fn cloned_groups_tracks_and_order_round_trip_through_inventory() {
    let (mut inv, mut set) = fixture();
    set.name = "Clone".into();
    set.default = false;
    let config = set.states.get_mut(&State::Standing).expect("state");
    let a = config.steps[0].members[0].clone();
    let b = config.steps[1].members[0].clone();
    config.steps[0].group = true;
    config.steps[0].name = "Together".into();
    config.steps[0].members.push(b.clone());
    config.tracks.push(Track {
        id: Uuid::new_v4(),
        name: "Independent".into(),
        cycle: Cycle {
            cycle: true,
            random: true,
            seconds: 3.0,
        },
        members: vec![a, b],
    });
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    let (change, id) = inventory::create(&inv, &catalog, &set).expect("create");
    assert!(change.copies.is_empty());
    crate::world::inventory::actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("mutation");
    catalog.refresh(&mut inv);
    let loaded = catalog.selected(id).expect("clone");
    let config = &loaded.states[&State::Standing];
    assert_eq!(config.steps[0].members.len(), 2);
    assert_eq!(config.tracks[0].cycle.seconds, 3.0);
    let change = inventory::editor::reorder(
        &inv,
        loaded,
        State::Standing,
        inventory::editor::List::Main,
        config.steps[0].id,
        config.steps[1].id,
        true,
    )
    .expect("reorder");
    crate::world::inventory::actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("mutation");
    catalog.refresh(&mut inv);
    assert_eq!(
        catalog.selected(id).expect("clone").states[&State::Standing].steps[1].name,
        "Together"
    );
}

#[test]
fn locomotion_changes_stop_owned_and_replaced_system_animations_and_preserve_gestures() {
    let (_, set) = fixture();
    let mut runtime = Runtime::default();
    let now = Instant::now();
    let gesture = PlayingAnimation::from_signal(Uuid::from_u128(900), 1, now, None);
    let stand = signal(State::Standing, now);
    let raw = [stand, gesture];
    let (pose, cmd) = runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    let standing = step(&set, State::Standing, 1);
    assert_eq!(cmd, vec![(stand.id, false), (standing, true)]);
    assert!(pose.contains(&gesture));
    assert!(!pose.contains(&stand));
    let old = *pose.iter().find(|a| a.id == standing).expect("stand");
    assert!(
        runtime
            .update(Some(&set), frame(&raw), now + Duration::from_secs(1), |_| true, |_| 0)
            .1
            .is_empty()
    );
    let raw = [signal(State::Walking, now + Duration::from_secs(2)), gesture];
    let (_, cmd) = runtime.update(Some(&set), frame(&raw), now + Duration::from_secs(2), |_| true, |_| 0);
    assert_eq!(
        cmd,
        vec![
            (old.id, false),
            (State::Walking.motion(), false),
            (step(&set, State::Walking, 0), true)
        ]
    );
    runtime.paused = true;
    let (pose, cmd) = runtime.update(Some(&set), frame(&raw), now + Duration::from_secs(3), |_| true, |_| 0);
    assert_eq!(pose, raw);
    assert_eq!(cmd, vec![(step(&set, State::Walking, 0), false), (State::Walking.motion(), true)]);
}

#[test]
fn timers_sequences_and_region_reassertion_keep_continuous_clocks() {
    let (_, set) = fixture();
    let mut r = Runtime::default();
    let now = Instant::now();
    let mut raw = [signal(State::Standing, now)];
    let (pose, _) = r.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    raw[0] = PlayingAnimation::from_signal(raw[0].id, 2, now + Duration::from_secs(1), Some(&raw[0]));
    let (same, cmd) = r.update(Some(&set), frame(&raw), now + Duration::from_secs(1), |_| true, |_| 0);
    assert_eq!(
        cmd,
        vec![(State::Standing.motion(), false)],
        "a reasserted default is stopped without restarting the AO"
    );
    assert_eq!(same, pose);
    let mut f = frame(&raw);
    f.region = Some(2);
    let (same, cmd) = r.update(Some(&set), f, now + Duration::from_secs(2), |_| true, |_| 0);
    assert_eq!(same, pose);
    assert_eq!(cmd, vec![(State::Standing.motion(), false), (pose[0].id, true)]);
    let (_, cmd) = r.update(Some(&set), frame(&raw), now + Duration::from_secs(6), |_| true, |_| 0);
    assert!(cmd.contains(&(step(&set, State::Standing, 0), true)));
}

#[test]
fn sits_mouselook_swimming_and_transient_controls_follow_firestorm() {
    let (_, mut set) = fixture();
    let mut r = Runtime::default();
    let now = Instant::now();
    let raw = [signal(State::Sitting, now)];
    let (pose, _) = r.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert!(pose.iter().any(|a| a.id == SIT_GENERIC));
    let mut f = frame(&raw);
    f.on_object = true;
    f.seat_animations = true;
    let (pose, cmd) = r.update(Some(&set), f, now, |_| true, |_| 0);
    assert_eq!(pose, raw);
    assert!(cmd.iter().all(|(_, start)| !start));
    let raw = [signal(State::GroundSit, now)];
    let mut f = frame(&raw);
    f.on_object = true;
    assert_eq!(r.update(Some(&set), f, now, |_| true, |_| 0).0, raw);
    let (pose, _) = r.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert!(pose.contains(&raw[0]), "ground sit drives agent controls");
    set.mouselook = true;
    let raw = [signal(State::Standing, now)];
    let mut f = frame(&raw);
    f.mouselook = true;
    assert_eq!(r.update(Some(&set), f, now, |_| true, |_| 0).0, raw);
    let raw = [signal(State::Flying, now)];
    let mut f = frame(&raw);
    f.underwater = true;
    r.update(Some(&set), f, now, |_| true, |_| 0);
    assert_eq!(r.current, Some(State::UnderwaterSwimming));
}

#[test]
fn unloaded_replacements_keep_stock_pose_even_when_always_is_ready() {
    let (_, mut set) = fixture();
    let mut r = Runtime::default();
    let now = Instant::now();
    set.states.insert(State::Always, set.states[&State::Typing].clone());
    let ready = step(&set, State::Always, 0);
    let raw = [signal(State::Walking, now)];
    let (pose, _) = r.update(Some(&set), frame(&raw), now, |id| *id == ready, |_| 0);
    assert!(pose.contains(&raw[0]));
    assert!(pose.iter().any(|a| a.id == ready));
}

#[test]
fn typing_is_concurrent_and_completed_nonloops_do_not_restart() {
    let (_, set) = fixture();
    let mut r = Runtime::default();
    let now = Instant::now();
    let raw = [signal(State::Walking, now)];
    let walk = step(&set, State::Walking, 0);
    r.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    r.finished(walk);
    let raw = [raw[0], signal(State::Typing, now + Duration::from_secs(1))];
    let (_, cmd) = r.update(Some(&set), frame(&raw), now + Duration::from_secs(1), |_| true, |_| 0);
    assert_eq!(cmd, vec![(step(&set, State::Typing, 0), true), (State::Walking.motion(), true)]);
    let (pose, cmd) = r.update(Some(&set), frame(&raw), now + Duration::from_secs(2), |_| true, |_| 0);
    assert!(cmd.is_empty());
    assert!(!pose.iter().any(|a| a.id == walk));
}

#[test]
fn changing_cycle_options_arms_the_timer_and_pausing_resets_it() {
    let (_, mut set) = fixture();
    let mut r = Runtime::default();
    let now = Instant::now();
    let raw = [signal(State::Standing, now)];
    set.states.get_mut(&State::Standing).expect("state").cycle.cycle = false;
    r.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    set.states.get_mut(&State::Standing).expect("state").cycle = Cycle {
        cycle: true,
        random: false,
        seconds: 1.0,
    };
    assert!(
        r.update(Some(&set), frame(&raw), now + Duration::from_secs(1), |_| true, |_| 0)
            .1
            .is_empty()
    );
    assert!(
        r.update(Some(&set), frame(&raw), now + Duration::from_secs(2), |_| true, |_| 0)
            .1
            .contains(&(step(&set, State::Standing, 1), true))
    );
    r.stands = false;
    assert!(
        r.update(Some(&set), frame(&raw), now + Duration::from_secs(3), |_| true, |_| 0)
            .1
            .iter()
            .all(|(id, s)| !s || *id == State::Standing.motion())
    );
    r.stands = true;
    assert!(
        r.update(Some(&set), frame(&raw), now + Duration::from_secs(4), |_| true, |_| 0)
            .1
            .contains(&(step(&set, State::Standing, 0), true))
    );
}

#[test]
fn delayed_simulator_echo_cannot_replay_a_stopped_override() {
    let (_, set) = fixture();
    let mut r = Runtime::default();
    let now = Instant::now();
    let raw = [signal(State::Standing, now)];
    let (pose, _) = r.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    let ao = pose[0];
    let raw = [signal(State::Walking, now + Duration::from_millis(10))];
    r.update(Some(&set), frame(&raw), now + Duration::from_millis(10), |_| true, |_| 0);
    let raw = [raw[0], ao];
    let (pose, cmd) = r.update(Some(&set), frame(&raw), now + Duration::from_millis(100), |_| true, |_| 0);
    assert!(cmd.is_empty());
    assert!(!pose.iter().any(|a| a.id == ao.id));
}

#[test]
fn animation_groups_and_tracks_cycle_independently_of_locomotion() {
    let (_, mut set) = fixture();
    let now = Instant::now();
    let always = set.states[&State::Standing].clone();
    let a = step(&set, State::Standing, 0);
    let b = step(&set, State::Standing, 1);
    let walk = set.states.get_mut(&State::Walking).expect("walk");
    walk.steps[0].members.push(always.steps[0].members[0].clone());
    walk.tracks.push(Track {
        id: Uuid::from_u128(950),
        name: "track".into(),
        cycle: Cycle {
            cycle: true,
            random: false,
            seconds: 1.0,
        },
        members: always.steps.iter().flat_map(|s| s.members.clone()).collect(),
    });
    set.states.insert(State::Always, set.states[&State::Typing].clone());
    let mut r = Runtime::default();
    let raw = [signal(State::Walking, now)];
    let (pose, cmd) = r.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    assert_eq!(cmd.iter().filter(|(_, start)| *start).count(), 4);
    assert!(cmd.contains(&(State::Walking.motion(), false)));
    assert!(pose.iter().any(|p| p.id == a));
    assert!(pose.iter().any(|p| p.id == b));
    let (_, cmd) = r.update(Some(&set), frame(&raw), now + Duration::from_secs(1), |_| true, |_| 0);
    assert_eq!(cmd, vec![(b, false)], "the group continues to own the track's new asset");
    let raw = [signal(State::Running, now + Duration::from_secs(2))];
    let (_, cmd) = r.update(Some(&set), frame(&raw), now + Duration::from_secs(2), |_| true, |_| 0);
    assert!(!cmd.contains(&(step(&set, State::Always, 0), false)));
}

#[test]
fn swimming_cycles_follow_the_remapped_state_without_reentering_each_frame() {
    let (_, mut set) = fixture();
    let mut swim = set.states[&State::Standing].clone();
    swim.cycle.seconds = 1.0;
    set.states.insert(State::UnderwaterSwimming, swim);
    let now = Instant::now();
    let raw = [signal(State::Flying, now)];
    let mut r = Runtime::default();
    let mut f = frame(&raw);
    f.underwater = true;
    let (first, _) = r.update(Some(&set), f, now, |_| true, |_| 0);
    let mut f = frame(&raw);
    f.underwater = true;
    let (same, commands) = r.update(Some(&set), f, now + Duration::from_millis(500), |_| true, |_| 0);
    assert_eq!(same, first);
    assert!(commands.is_empty());
    let mut f = frame(&raw);
    f.underwater = true;
    let (_, commands) = r.update(Some(&set), f, now + Duration::from_secs(1), |_| true, |_| 0);
    assert!(commands.contains(&(first[0].id, false)));
    assert!(commands.contains(&(step(&set, State::UnderwaterSwimming, 0), true)));
}

#[test]
fn failed_fetch_is_reported_without_retrying_or_losing_the_loaded_set() {
    let (mut inv, set) = fixture();
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    let folder = set.states[&State::Standing].folder;
    inv.folders.get_mut(&folder).expect("folder").state = crate::world::inventory::FetchState::Unknown;
    inv.generation += 1;
    catalog.refresh(&mut inv);
    assert!(catalog.loading && !catalog.failed);
    inv.queue.clear();
    inv.failed(&[folder]); // Fetch failures do not increment the inventory generation.
    catalog.refresh(&mut inv);
    assert!(catalog.failed);
    assert_eq!(catalog.selected(set.id), Some(&set));
    catalog.refresh(&mut inv);
    assert!(inv.queue.is_empty(), "failed fetch must wait for an explicit reload");
    catalog.reload(&mut inv);
    catalog.refresh(&mut inv);
    assert!(!catalog.failed);
    assert!(inv.queue.iter().any(|(id, _)| *id == folder));
}

#[test]
fn locked_operation_targets_are_never_removed_but_stay_editable_like_the_legacy_viewer() {
    let (inv, mut set) = fixture();
    let none = std::collections::HashSet::new();
    set.name = "Renamed".into();
    let rename = inventory::save_set(&inv, &set).expect("rename");
    assert!(inventory::editable(&inv, &rename, &none));
    // LLFolderBridge::isProtected holds a folder in place, not its name nor
    // its contents
    assert!(inventory::editable(&inv, &rename, &std::collections::HashSet::from([set.id])));
    let config = &set.states[&State::Standing];
    let cycle = inventory::save_cycle(&inv, State::Standing, config).expect("cycle");
    assert!(inventory::editable(&inv, &cycle, &std::collections::HashSet::from([config.folder])));
    let a = &config.steps[0].members[0];
    let remove = inventory::remove(&inv, a.link).expect("remove");
    assert!(inventory::editable(&inv, &remove, &none));
    assert!(!inventory::editable(&inv, &remove, &std::collections::HashSet::from([a.link])));
    let add = inventory::add(&inv, &set, State::Walking, a.original, None).expect("add");
    assert!(inventory::editable(&inv, &add, &std::collections::HashSet::from([a.original])));
    assert!(inventory::editable(&inv, &add, &std::collections::HashSet::from([set.id])));
}

#[test]
fn swim_motor_uses_ao_selection_and_broadcasts_it_without_changing_raw_signals() {
    let (_, set) = fixture();
    let now = Instant::now();
    let stand = signal(State::Standing, now);
    let raw = [stand];
    let swim = signal(State::SwimmingForward, now);
    let mut f = frame(&raw);
    f.swim_hold = Some((State::SwimmingForward, swim));
    let mut runtime = Runtime::default();
    let asset = step(&set, State::SwimmingForward, 0);
    let (pose, commands) = runtime.update(Some(&set), f, now, |_| true, |_| 0);
    assert_eq!(runtime.current, Some(State::SwimmingForward));
    assert!(commands.contains(&(asset, true)));
    assert!(pose.iter().any(|a| a.id == asset));
    assert!(!pose.contains(&stand));
    assert_eq!(raw, [stand]);
    let mut f = frame(&raw);
    f.swim_hold = Some((State::SwimmingForward, swim));
    f.enabled = false;
    let (pose, commands) = runtime.update(Some(&set), f, now + Duration::from_millis(100), |_| true, |_| 0);
    assert!(commands.is_empty(), "the swimming owner survives ordinary AO disable");
    assert!(pose.iter().any(|a| a.id == asset));
    assert!(!pose.contains(&stand));
}

#[test]
fn swimming_requires_all_eight_resolved_lists_in_the_selected_aurora_set() {
    let (mut inv, set) = fixture();
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    assert!(catalog.swimming_ready(set.id));
    for state in State::CORE_SWIMMING {
        let mut incomplete = set.clone();
        incomplete.states.remove(&state);
        assert_eq!(incomplete.core_swimming_slots(), State::CORE_SWIMMING.len() - 1);
        let mut broken = set.clone();
        broken.states.get_mut(&state).expect("slot").steps[0].members[0].asset = Uuid::nil();
        assert_eq!(broken.core_swimming_slots(), State::CORE_SWIMMING.len() - 1);
    }
    let empty = Set::new("No swimming".into());
    let id = empty.id;
    catalog.sets.push(empty);
    assert!(!catalog.swimming_ready(id), "another complete set cannot unlock this selection");
    catalog.loading = true;
    assert!(!catalog.swimming_ready(set.id));
    catalog.loading = false;
    catalog.failed = true;
    assert!(!catalog.swimming_ready(set.id));
}

#[test]
fn adding_to_a_swimming_list_appends_without_replacing_links_or_originals() {
    use crate::world::inventory::actions;
    let (mut inv, set) = fixture();
    let original = set.swimming_animation(State::SwimmingDown).expect("animation").original;
    let old = set.states[&State::Floating].clone();
    let before = format!("{:?}", inv.items[&original]);
    let change = inventory::add(&inv, &set, State::Floating, original, None).expect("append");
    assert!(inventory::editable(&inv, &change, &Default::default()));
    actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("saved");
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    let after = catalog.selected(set.id).expect("set");
    assert_eq!(after.states[&State::Floating].folder, old.folder);
    assert_eq!(after.states[&State::Floating].steps.len(), old.steps.len() + 1);
    assert_eq!(after.states[&State::Floating].steps[0], old.steps[0]);
    assert_eq!(format!("{:?}", inv.items[&original]), before);
    assert!(catalog.swimming_ready(set.id));
}

#[test]
fn adding_to_an_ordered_state_sends_only_the_link() {
    use crate::world::inventory::actions;
    use aurora_net::inventory::operations::Operation;
    let (mut inv, set) = fixture();
    let reload = |inv: &mut Inventory| {
        let mut catalog = inventory::Catalog::default();
        catalog.refresh(inv);
        catalog.selected(set.id).expect("set").clone()
    };
    let patches = |ops: &[Operation]| -> Vec<(Uuid, String)> {
        ops.iter()
            .filter_map(|op| match op {
                Operation::Patch { id, body, .. } => {
                    Some((*id, format!("{}{}", body["desc"].to_string_value(), body["name"].to_string_value())))
                }
                _ => None,
            })
            .collect()
    };
    let walk = set.states[&State::Walking].steps[0].members[0].original;
    let change = inventory::add(&inv, &set, State::Standing, walk, None).expect("add");
    actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("saved");
    let mut set = reload(&mut inv);
    let steps = set.states[&State::Standing].steps.clone();
    assert_eq!(steps.iter().map(|s| s.order).collect::<Vec<_>>(), [0, 1, 2]);
    // AOEngine::addAnimation: one link, no rewrite of the stands already there
    let change = inventory::add(&inv, &set, State::Standing, walk, None).expect("add");
    assert!(
        matches!(change.operations.as_slice(), [Operation::Links { links, .. }] if links[0].desc == "3"),
        "{:?}",
        change.operations
    );
    // updateSortOrder rewrites only the entry out of place
    inv.items.get_mut(&steps[2].id).expect("link").desc = "5".into();
    set = reload(&mut inv);
    let change = inventory::add(&inv, &set, State::Standing, walk, None).expect("add");
    let rewritten = patches(&change.operations);
    assert_eq!(rewritten.len(), 1);
    assert_eq!(rewritten[0], (steps[2].id, "2".into()));
    assert!(matches!(change.operations.last(), Some(Operation::Links { links, .. }) if links[0].desc == "3"));
    // the same inside a group, and for the group folder's OR value
    let selected = steps[..2].iter().map(|s| s.id).collect();
    let (change, group) = inventory::editor::combine(&inv, &set, State::Standing, "Together", false, &selected).expect("group");
    let reply = actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("saved");
    set = reload(&mut inv);
    let group = reply.created.get(&group).copied().unwrap_or(group);
    let change = inventory::add(&inv, &set, State::Standing, walk, Some(group)).expect("add to group");
    assert!(
        matches!(change.operations.as_slice(), [Operation::Links { links, .. }] if links[0].desc == "2"),
        "{:?}",
        change.operations
    );
    inv.folders.get_mut(&group).expect("group").info.name = "Together:OR7".into();
    set = reload(&mut inv);
    let change = inventory::add(&inv, &set, State::Standing, walk, None).expect("add");
    let rewritten = patches(&change.operations);
    assert_eq!(rewritten.len(), 2, "{rewritten:?}");
    assert!(rewritten.contains(&(group, "Together:OR1".into())));
}

#[test]
fn every_swimming_context_broadcasts_its_selected_animation_and_hands_back_on_exit() {
    let (_, mut set) = fixture();
    for (index, state) in State::SWIMMING.into_iter().enumerate() {
        set.states.get_mut(&state).expect("slot").steps[0].members[0].asset = Uuid::from_u128(0x123000 + index as u128);
    }
    let now = Instant::now();
    let raw = [signal(State::Falling, now)];
    let mut runtime = Runtime::default();
    runtime.paused = true;
    let mut previous = None;
    for (index, state) in State::SWIMMING.into_iter().enumerate() {
        let time = now + Duration::from_secs(index as u64);
        let mut f = frame(&raw);
        f.enabled = false;
        f.swim_hold = Some((state, signal(state, time)));
        let (pose, commands) = runtime.update(Some(&set), f, time, |_| true, |_| 0);
        let asset = set.swimming_animation(state).expect("selected asset").asset;
        assert!(commands.contains(&(asset, true)));
        if let Some(previous) = previous {
            assert!(commands.contains(&(previous, false)));
        }
        assert_eq!(pose.iter().map(|a| a.id).collect::<Vec<_>>(), vec![asset]);
        previous = Some(asset);
    }
    let mut f = frame(&raw);
    f.enabled = false;
    let (pose, commands) = runtime.update(Some(&set), f, now + Duration::from_secs(10), |_| true, |_| 0);
    assert!(commands.contains(&(previous.expect("last animation"), false)));
    assert_eq!(pose, raw);
}

#[test]
fn completed_water_transition_does_not_loop_with_regular_ao_off_and_replays_on_a_new_entry() {
    let (_, set) = fixture();
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let asset = set.swimming_animation(State::WaterEntry).expect("entry animation").asset;
    let mut f = frame(&[]);
    f.enabled = false;
    let start = signal(State::WaterEntry, now);
    f.swim_hold = Some((State::WaterEntry, start));
    assert!(runtime.update(Some(&set), f, now, |_| true, |_| 0).1.contains(&(asset, true)));
    runtime.finished(asset);
    let mut f = frame(&[]);
    f.enabled = false;
    f.swim_hold = Some((State::WaterEntry, start));
    let (pose, commands) = runtime.update(Some(&set), f, now + Duration::from_millis(500), |_| true, |_| 0);
    assert!(
        commands.is_empty() && pose.is_empty(),
        "a finished transition must not be replayed on the next frame"
    );
    let mut f = frame(&[]);
    f.enabled = false;
    runtime.update(Some(&set), f, now + Duration::from_secs(1), |_| true, |_| 0);
    let mut f = frame(&[]);
    f.enabled = false;
    f.swim_hold = Some((State::WaterEntry, signal(State::WaterEntry, now + Duration::from_secs(2))));
    assert!(
        runtime
            .update(Some(&set), f, now + Duration::from_secs(2), |_| true, |_| 0)
            .1
            .contains(&(asset, true))
    );
}

#[test]
fn optional_water_slots_preserve_old_sets_and_forward_fallbacks() {
    let (_, mut set) = fixture();
    for state in State::SWIMMING.into_iter().filter(|s| !State::CORE_SWIMMING.contains(s)) {
        set.states.remove(&state);
    }
    assert!(set.swimming_ready());
    assert_eq!(set.swimming_slots(), 8);
    let now = Instant::now();
    {
        let state = State::UnderwaterSwimming;
        let mut runtime = Runtime::default();
        let mut f = frame(&[]);
        f.enabled = false;
        f.swim_hold = Some((state, signal(state, now)));
        let (pose, commands) = runtime.update(Some(&set), f, now, |_| true, |_| 0);
        let forward = step(&set, State::SwimmingForward, 0);
        assert!(pose.iter().any(|a| a.id == forward));
        assert!(commands.contains(&(forward, true)));
    }
    for state in State::SWIMMING.into_iter().chain(State::LEGACY_SWIMMING) {
        assert_eq!(State::parse(state.name()), Some(state));
    }
}

#[test]
fn an_authored_water_recovery_stops_the_hold_without_starting_priority_six_swimming() {
    let (_, set) = fixture();
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame(&[]);
    f.swim_hold = Some((State::WaterDiveEntry, signal(State::WaterDiveEntry, now)));
    runtime.update(Some(&set), f, now, |_| true, |_| 0);
    let held = step(&set, State::WaterDiveEntry, 0);
    let mut f = frame(&[]);
    f.swim_outro = true;
    f.swim_hold = Some((State::UnderwaterFloating, signal(State::UnderwaterFloating, now)));
    let (_, commands) = runtime.update(Some(&set), f, now + Duration::from_millis(180), |_| true, |_| 0);
    assert!(commands.contains(&(held, false)));
    assert!(commands.iter().all(|(_, start)| !start));
}

/// Issue #2700: like the Legacy AO (LLVOAvatar::startMotion, then
/// AOEngine::override), ours leaves the stock sit to the simulator and only
/// adds its sit and cover. The simulator's lists decide: a failed teleport
/// keeps the stock sit and the AO's sit; a teleport that unseats us drops the
/// stock sit, which ends the AO's sit with no stock state restored; an
/// arrival list still carrying the stock sit keeps it only while listed. A
/// teleport the simulator started unseats us after the arrival, which is the
/// same list. A new sit afterwards is an ordinary sit.
#[test]
fn the_stock_sit_stays_to_the_simulator_and_its_stop_ends_the_ao_sit() {
    let (_, set) = fixture();
    let now = Instant::now();
    let at = |ms: u64| now + Duration::from_millis(ms);
    let sit_override = step(&set, State::Sitting, 0);
    let seated = |r: &mut Runtime, start: Instant| {
        let raw = [PlayingAnimation::from_signal(State::Sitting.motion(), 1, start, None)];
        let mut f = frame(&raw);
        f.on_object = true;
        let (pose, cmd) = r.update(Some(&set), f, start, |_| true, |_| 0);
        assert_eq!(cmd, vec![(sit_override, true), (SIT_GENERIC, true)], "the stock sit is not stopped");
        assert!(
            !pose.iter().any(|a| a.id == State::Sitting.motion()),
            "nor drawn under the AO's sit"
        );
        // the simulator's echo: the stock sit, our sit and its cover
        let echo = [
            raw[0],
            PlayingAnimation::from_signal(sit_override, 2, start + Duration::from_millis(100), None),
            PlayingAnimation::from_signal(SIT_GENERIC, 3, start + Duration::from_millis(100), None),
        ];
        let mut f = frame(&echo);
        f.on_object = true;
        assert!(
            r.update(Some(&set), f, start + Duration::from_millis(100), |_| true, |_| 0)
                .1
                .is_empty()
        );
        assert_eq!(r.current, Some(State::Sitting));
        echo
    };
    let ends_the_sit = |cmd: &[(Uuid, bool)]| {
        cmd.contains(&(sit_override, false))
            && cmd.contains(&(SIT_GENERIC, false))
            && !cmd.contains(&(sit_override, true))
            && !cmd.iter().any(|(id, _)| *id == State::Sitting.motion())
    };

    let mut r = Runtime::default();
    let echo = seated(&mut r, now);
    // a failed teleport: off the seat locally (teleportCore), still seated
    // for the simulator, which keeps listing the stock sit
    assert!(r.update(Some(&set), frame(&echo), at(200), |_| true, |_| 0).1.is_empty());
    assert_eq!(r.current, Some(State::Sitting));
    // the teleport unseats us: the stock sit leaves the list
    let after = [echo[1], echo[2]];
    let (pose, cmd) = r.update(Some(&set), frame(&after), at(3000), |_| true, |_| 0);
    assert!(ends_the_sit(&cmd), "{cmd:?}");
    assert_ne!(r.current, Some(State::Sitting));
    assert!(!pose.iter().any(|a| a.id == sit_override || a.id == SIT_GENERIC));
    // the next walk is an ordinary walk
    let walk = [signal(State::Walking, at(4000))];
    let (_, cmd) = r.update(Some(&set), frame(&walk), at(4000), |_| true, |_| 0);
    assert_eq!(r.current, Some(State::Walking));
    assert!(cmd.contains(&(step(&set, State::Walking, 0), true)));

    // the arrival list still carries the stock sit, then drops it
    let mut r = Runtime::default();
    let echo = seated(&mut r, now);
    let mut f = frame(&echo);
    f.region = Some(2);
    r.update(Some(&set), f, at(3000), |_| true, |_| 0);
    assert_eq!(r.current, Some(State::Sitting), "overridden while listed");
    let after = [echo[1], echo[2]];
    let mut f = frame(&after);
    f.region = Some(2);
    let (_, cmd) = r.update(Some(&set), f, at(3200), |_| true, |_| 0);
    assert!(ends_the_sit(&cmd), "{cmd:?}");
    let mut f = frame(&[]);
    f.region = Some(2);
    r.update(Some(&set), f, at(5000), |_| true, |_| 0);
    assert_ne!(r.current, Some(State::Sitting), "and stays ended");

    // sitting on an object again is an ordinary sit
    let echo = seated(&mut r, at(6000));
    let mut f = frame(&echo);
    f.on_object = true;
    assert!(r.update(Some(&set), f, at(9000), |_| true, |_| 0).1.is_empty());
    assert_eq!(r.current, Some(State::Sitting), "held while the simulator lists it");
}

/// The changes held back after a teleport to another region are sent as
/// their net result: the last change of each animation, stops first.
#[test]
fn held_changes_keep_the_last_change_of_each_animation() {
    let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
    let mut held = Vec::new();
    runtime::hold_changes(&mut held, vec![(a, false), (b, true)]);
    runtime::hold_changes(&mut held, vec![(b, false), (c, true)]);
    runtime::hold_changes(&mut held, vec![(a, true)]);
    assert_eq!(held, vec![(b, false), (c, true), (a, true)]);
}
