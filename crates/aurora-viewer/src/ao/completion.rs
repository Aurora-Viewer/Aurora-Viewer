//! Approved Aurora extensions to the Legacy Viewer's four swim slots.
//! Reuse inventory animation configurations only while planning a new native
//! migration. The normal inventory writer creates fresh folders and links.
use super::{Set, State, StateConfig};

const FALLBACKS: [(State, State); 8] = [
    (State::UnderwaterFloating, State::Floating),
    (State::UnderwaterSwimming, State::SwimmingForward),
    (State::WaterFlightExit, State::FlyingUp),
    (State::WaterShoreExit, State::Jumping),
    (State::WaterDiveJump, State::Flying),
    (State::WaterDiveEntry, State::SwimmingDown),
    (State::WaterCannonball, State::Crouching),
    (State::WaterEntry, State::Falling),
];

fn assigned(config: &StateConfig) -> bool {
    config
        .steps
        .iter()
        .flat_map(|step| &step.members)
        .chain(config.tracks.iter().flat_map(|track| &track.members))
        .any(|animation| !animation.original.is_nil() && !animation.asset.is_nil())
}

pub(super) fn swimming(set: &mut Set) {
    for (target, source) in FALLBACKS {
        if set.states.get(&target).is_some_and(assigned) {
            continue;
        }
        if let Some(config) = set.states.get(&source).filter(|config| assigned(config)).cloned() {
            set.states.insert(target, config);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ao::{Animation, Cycle, Step, Track};
    use uuid::Uuid;

    fn config(id: u128) -> StateConfig {
        StateConfig {
            folder: Uuid::from_u128(id + 100),
            cycle: Cycle {
                cycle: true,
                random: true,
                seconds: 13.0,
            },
            steps: vec![Step {
                id: Uuid::from_u128(id + 200),
                name: format!("Clip {id}"),
                order: 0,
                group: false,
                members: vec![Animation {
                    link: Uuid::from_u128(id + 300),
                    original: Uuid::from_u128(id + 400),
                    asset: Uuid::from_u128(id),
                    name: format!("Clip {id}"),
                    order: 0,
                }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn approved_sources_complete_all_slots_and_keep_their_lists_and_cycles() {
        let mut set = Set::new("Migrated".into());
        for (state, id) in [
            (State::Floating, 1),
            (State::SwimmingForward, 2),
            (State::SwimmingDown, 3),
            (State::SwimmingUp, 4),
            (State::FlyingUp, 5),
            (State::Jumping, 6),
            (State::Flying, 7),
            (State::Crouching, 8),
            (State::Falling, 9),
        ] {
            set.states.insert(state, config(id));
        }
        let source = set.clone();
        swimming(&mut set);
        for (state, id) in [
            (State::UnderwaterFloating, 1),
            (State::UnderwaterSwimming, 2),
            (State::WaterFlightExit, 5),
            (State::WaterShoreExit, 6),
            (State::WaterDiveJump, 7),
            (State::WaterDiveEntry, 3),
            (State::WaterCannonball, 8),
            (State::WaterEntry, 9),
        ] {
            assert_eq!(set.states[&state], config(id));
        }
        for (state, config) in &source.states {
            assert_eq!(&set.states[state], config);
        }
        assert!(set.swimming_ready());
        assert_eq!(set.swimming_slots(), 12);
        let completed = set.clone();
        swimming(&mut set);
        assert_eq!(set, completed, "completion is idempotent");
    }

    #[test]
    fn authored_slots_win_and_missing_or_unresolved_sources_remain_missing() {
        let mut set = Set::new("Partial".into());
        set.states.insert(State::Floating, config(1));
        set.states.insert(State::UnderwaterFloating, config(99));
        let mut unresolved = config(2);
        unresolved.steps[0].members[0].asset = Uuid::nil();
        set.states.insert(State::SwimmingForward, unresolved);
        swimming(&mut set);
        assert_eq!(set.states[&State::UnderwaterFloating], config(99));
        for state in [
            State::UnderwaterSwimming,
            State::WaterFlightExit,
            State::SwimmingUp,
            State::WaterCannonball,
        ] {
            assert!(!set.states.contains_key(&state), "no invented donor for {state:?}");
        }
        assert!(!set.swimming_ready());
        let mut empty = Set::new("Empty".into());
        swimming(&mut empty);
        assert!(empty.states.is_empty());
    }

    #[test]
    fn groups_and_independent_tracks_survive_completion() {
        let mut set = Set::new("Layered".into());
        let mut source = config(2);
        source.steps[0].group = true;
        source.steps[0].members.push(config(3).steps.remove(0).members.remove(0));
        source.tracks.push(Track {
            id: Uuid::from_u128(900),
            name: "Hands".into(),
            cycle: Cycle {
                cycle: true,
                random: false,
                seconds: 7.0,
            },
            members: config(4).steps.remove(0).members,
        });
        set.states.insert(State::SwimmingForward, source.clone());
        swimming(&mut set);
        assert_eq!(set.states[&State::UnderwaterSwimming], source);
        source.steps.clear();
        set.states.remove(&State::UnderwaterSwimming);
        set.states.insert(State::SwimmingForward, source.clone());
        swimming(&mut set);
        assert_eq!(set.states[&State::UnderwaterSwimming], source, "a track alone is a valid source");
    }
}
