use super::*;
use crate::world::{PlayingAnimation, inventory::Inventory};
use runtime::{Frame, Runtime};
use std::time::{Duration, Instant};

fn fixture() -> (Inventory, Set) {
    let mut inv = Inventory::default();
    let id = demo::seed(&mut inv, Uuid::from_u128(1));
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    let set = catalog.selected(id).expect("demo AO set").clone();
    (inv, set)
}
fn frame(raw: &[PlayingAnimation]) -> Frame<'_> {
    Frame {
        raw,
        run_hold: None,
        swim_hold: None,
        native_running: false,
        enabled: true,
        mouselook: false,
        on_object: false,
        seat_animations: false,
        underwater: false,
        region: Some(1),
    }
}
fn signal(state: State, now: Instant) -> PlayingAnimation {
    PlayingAnimation::from_signal(state.motion(), 1, now, None)
}
fn step(set: &Set, state: State, index: usize) -> Uuid {
    set.states[&state].steps[index].members[0].asset
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
fn locomotion_changes_stop_only_owned_animations_and_preserve_gestures() {
    let (_, set) = fixture();
    let mut runtime = Runtime::default();
    let now = Instant::now();
    let gesture = PlayingAnimation::from_signal(Uuid::from_u128(900), 1, now, None);
    let stand = signal(State::Standing, now);
    let raw = [stand, gesture];
    let (pose, cmd) = runtime.update(Some(&set), frame(&raw), now, |_| true, |_| 0);
    let standing = step(&set, State::Standing, 1);
    assert_eq!(cmd, vec![(standing, true)]);
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
    assert_eq!(cmd, vec![(old.id, false), (step(&set, State::Walking, 0), true)]);
    runtime.paused = true;
    let (pose, cmd) = runtime.update(Some(&set), frame(&raw), now + Duration::from_secs(3), |_| true, |_| 0);
    assert_eq!(pose, raw);
    assert_eq!(cmd, vec![(step(&set, State::Walking, 0), false)]);
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
    assert!(cmd.is_empty());
    assert_eq!(same, pose);
    let mut f = frame(&raw);
    f.region = Some(2);
    let (same, cmd) = r.update(Some(&set), f, now + Duration::from_secs(2), |_| true, |_| 0);
    assert_eq!(same, pose);
    assert_eq!(cmd, vec![(pose[0].id, true)]);
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
    assert_eq!(r.current, Some(State::SwimmingForward));
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
    assert_eq!(cmd, vec![(step(&set, State::Typing, 0), true)]);
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
            .all(|(_, s)| !s)
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
    assert_eq!(cmd.len(), 4);
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
    set.states.insert(State::SwimmingForward, swim);
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
    assert!(commands.contains(&(step(&set, State::SwimmingForward, 0), true)));
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
fn protected_operation_targets_cannot_be_changed_but_originals_remain_readable() {
    let (inv, mut set) = fixture();
    let none = std::collections::HashSet::new();
    set.name = "Renamed".into();
    let rename = inventory::save_set(&inv, &set).expect("rename");
    assert!(inventory::editable(&inv, &rename, &none));
    assert!(!inventory::editable(&inv, &rename, &std::collections::HashSet::from([set.id])));
    let config = &set.states[&State::Standing];
    let cycle = inventory::save_cycle(&inv, State::Standing, config).expect("cycle");
    assert!(!inventory::editable(
        &inv,
        &cycle,
        &std::collections::HashSet::from([config.folder])
    ));
    let a = &config.steps[0].members[0];
    let remove = inventory::remove(&inv, a.link).expect("remove");
    assert!(!inventory::editable(&inv, &remove, &std::collections::HashSet::from([a.link])));
    let add = inventory::add(&inv, &set, State::Walking, a.original, None).expect("add");
    assert!(inventory::editable(&inv, &add, &std::collections::HashSet::from([a.original])));
    assert!(!inventory::editable(&inv, &add, &std::collections::HashSet::from([set.id])));
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
fn swimming_requires_all_eight_resolved_single_slots_in_the_selected_aurora_set() {
    let (mut inv, set) = fixture();
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    assert!(catalog.swimming_ready(set.id));
    for state in State::SWIMMING {
        let mut incomplete = set.clone();
        incomplete.states.remove(&state);
        assert_eq!(incomplete.swimming_slots(), State::SWIMMING.len() - 1);
        let mut broken = set.clone();
        broken.states.get_mut(&state).expect("slot").steps[0].members[0].asset = Uuid::nil();
        assert_eq!(broken.swimming_slots(), State::SWIMMING.len() - 1);
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
fn replacing_a_swimming_slot_keeps_only_one_link_and_preserves_originals_and_other_slots() {
    use crate::world::inventory::actions;
    let (mut inv, set) = fixture();
    let original = set.swimming_animation(State::SwimmingDown).expect("other animation").original;
    let old = set.states[&State::Floating].folder;
    let unchanged = set.states[&State::SwimmingDown].clone();
    let before = format!("{:?}", inv.items[&original]);
    let change = inventory::set_swimming(&inv, &set, State::Floating, original).expect("replace");
    assert!(inventory::editable(&inv, &change, &Default::default()));
    actions::demo_mutate(&mut inv, Uuid::from_u128(1), change).expect("saved");
    let mut catalog = inventory::Catalog::default();
    catalog.refresh(&mut inv);
    let after = catalog.selected(set.id).expect("set");
    assert_eq!(after.swimming_animation(State::Floating).expect("one slot").original, original);
    assert_ne!(after.states[&State::Floating].folder, old);
    assert_eq!(after.states[&State::SwimmingDown], unchanged);
    assert_eq!(format!("{:?}", inv.items[&original]), before);
    assert!(catalog.swimming_ready(set.id));
    let removed = inventory::remove(&inv, after.states[&State::Floating].folder).expect("clear slot");
    actions::demo_mutate(&mut inv, Uuid::from_u128(1), removed).expect("cleared");
    catalog.refresh(&mut inv);
    assert!(!catalog.swimming_ready(set.id));
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
