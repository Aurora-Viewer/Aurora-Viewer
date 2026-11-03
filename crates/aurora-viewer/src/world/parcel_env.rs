//! Environments kept for a parcel: the local environment the user saved
//! there comes back each time they stand on that parcel, and what they had
//! before comes back when they leave it. An Aurora feature (the Legacy Viewer
//! keeps one local environment, LLEnvironment::saveToSettings); nothing is
//! sent to the server but the RemoteParcelRequest that names the parcel, and
//! none at all until something is saved.

use super::World;
use aurora_net::land::LandCommand;
use aurora_net::{NetCommand, RegionHandle};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A parcel's id may be asked before the region's capabilities are in.
const ASK_EVERY: Duration = Duration::from_secs(5);
const MAX_ASKS: u8 = 5;

/// A saved environment, for the parcel with this server UUID (a local id
/// only means something within its region, and is reused).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub parcel: Uuid,
    pub name: String,
    pub region_name: String,
    /// `EnvSelector::saved_local`, in LLSD notation.
    pub environment: Option<String>,
    /// Time of day preset (0: none).
    pub time_of_day: u8,
}

/// The parcel under the agent and what was decided for it.
#[derive(Debug)]
struct Here {
    key: (RegionHandle, i32),
    id: Option<Uuid>,
    asks: u8,
    asked: Option<Instant>,
    /// The selector was told about this parcel.
    settled: bool,
    /// The saved environment shown here.
    shown: Option<Uuid>,
}

#[derive(Debug, Default)]
pub struct ParcelEnv {
    here: Option<Here>,
    /// Ids already asked this session, so that walking back and forth does
    /// not ask again.
    known: HashMap<(RegionHandle, i32), Uuid>,
    /// "Save" was asked on this parcel: done once its id and the settings
    /// are in, dropped if the agent leaves it first.
    saving: Option<(RegionHandle, i32)>,
}

/// What came of a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Saved(Entry),
    /// The parcel could not be identified: nothing was saved.
    NotSaved,
    /// The agent left the parcel before the save could be done.
    Left,
}

impl ParcelEnv {
    /// The saved parcel under the agent, if any.
    pub fn saved_here(&self, saved: &[Entry]) -> Option<Uuid> {
        let id = self.here.as_ref()?.id?;
        saved.iter().any(|e| e.parcel == id).then_some(id)
    }

    /// RemoteParcelRequest answer, which may come after the agent left that
    /// parcel. A nil or missing id is asked again.
    pub fn on_parcel_id(&mut self, handle: RegionHandle, local_id: i32, id: Option<Uuid>) {
        let Some(id) = id.filter(|id| !id.is_nil()) else { return };
        let key = (handle, local_id);
        self.known.insert(key, id);
        if let Some(here) = self.here.as_mut().filter(|h| h.key == key) {
            here.id = Some(id);
        }
    }

    /// Follow the agent to `key`. True when its id must be asked now;
    /// `Err` once the last request had its time to answer.
    fn follow(&mut self, key: (RegionHandle, i32), wanted: bool, now: Instant) -> Result<bool, ()> {
        if self.here.as_ref().is_none_or(|h| h.key != key) {
            self.here = Some(Here {
                key,
                id: self.known.get(&key).copied(),
                asks: 0,
                asked: None,
                settled: false,
                shown: None,
            });
        }
        let Some(here) = self.here.as_mut().filter(|h| wanted && h.id.is_none()) else {
            return Ok(false);
        };
        if here.asked.is_some_and(|at| now.duration_since(at) < ASK_EVERY) {
            return Ok(false);
        }
        if here.asks >= MAX_ASKS {
            return Err(());
        }
        here.asks += 1;
        here.asked = Some(now);
        Ok(true)
    }
}

impl World {
    /// Save the local environment for the parcel under the agent, once it
    /// has loaded.
    pub fn save_parcel_environment(&mut self) {
        self.eep.arm_parcel_save();
        self.parcel_env.saving = self.main_region.zip(self.parcel.as_ref()).map(|(handle, p)| (handle, p.local_id));
    }

    /// One frame: ask for the parcel's id while it matters, put on or take
    /// off its saved environment, and do a save that was asked. What was
    /// shown stays until the parcel is known, so that a teleport does not
    /// flash the environment left behind.
    pub fn parcel_environment_frame(&mut self, saved: &[Entry], now: Instant) -> (Outcome, Option<NetCommand>) {
        let (Some(handle), Some(parcel)) = (self.main_region, self.parcel.clone()) else {
            return (Outcome::Nothing, None);
        };
        let key = (handle, parcel.local_id);
        if self.parcel_env.saving.is_some_and(|asked| asked != key) {
            self.parcel_env.saving = None;
            return (Outcome::Left, None);
        }
        let wanted = !saved.is_empty() || self.parcel_env.saving.is_some();
        let mut ask = None;
        let mut gave_up = false;
        match self.parcel_env.follow(key, wanted, now) {
            Ok(true) => {
                let region = self.regions.get(&handle);
                ask = Some(NetCommand::Land(LandCommand::ParcelIdRequest {
                    handle,
                    local_id: parcel.local_id,
                    region_id: region.and_then(|r| r.info.as_ref()).map_or(Uuid::nil(), |i| i.region_id),
                    // the agent stands on the parcel, whatever its shape
                    position: self.agent.position,
                }));
            }
            Ok(false) => {}
            Err(()) if self.parcel_env.saving.is_some() => {
                self.parcel_env.saving = None;
                return (Outcome::NotSaved, None);
            }
            Err(()) => gave_up = true,
        }
        // RLVa holds the environment (@setenv): decided once it lets go
        let Some(here) = self.parcel_env.here.as_mut().filter(|_| !self.eep.rlva_locked) else {
            return (Outcome::Nothing, ask);
        };
        if wanted && here.id.is_none() && !gave_up {
            return (Outcome::Nothing, ask);
        }
        let target = here.id.and_then(|id| saved.iter().find(|e| e.parcel == id));
        if !here.settled || here.shown != target.map(|e| e.parcel) {
            match target {
                Some(entry) => {
                    let data = entry
                        .environment
                        .as_ref()
                        .and_then(|e| aurora_llsd::from_notation(e.as_bytes()).ok());
                    self.eep.hold_parcel(data.as_ref(), entry.time_of_day);
                }
                None => self.eep.settle_parcel(),
            }
            here.settled = true;
            here.shown = target.map(|e| e.parcel);
        }
        if self.parcel_env.saving.is_none() || self.eep.local_loading() {
            return (Outcome::Nothing, ask);
        }
        self.parcel_env.saving = None;
        let Some(id) = here.id else {
            return (Outcome::NotSaved, ask);
        };
        // a picked item that could not load (FailedToFindSettings says so)
        let Some((environment, time_of_day)) = self.eep.capture_parcel() else {
            return (Outcome::Nothing, ask);
        };
        here.shown = Some(id);
        let entry = Entry {
            parcel: id,
            name: parcel.name.clone(),
            region_name: self.regions.get(&handle).map_or_else(String::new, |r| r.name.clone()),
            environment: environment.map(|e| aurora_llsd::to_notation(&e)),
            time_of_day,
        };
        (Outcome::Saved(entry), ask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const KEY: (RegionHandle, i32) = (7, 3);

    #[test]
    fn nothing_is_asked_until_something_is_saved() {
        let mut env = ParcelEnv::default();
        assert_eq!(env.follow(KEY, false, Instant::now()), Ok(false));
        assert!(env.here.is_some());
    }

    #[test]
    fn a_parcel_is_asked_once_and_remembered() {
        let mut env = ParcelEnv::default();
        let now = Instant::now();
        assert_eq!(env.follow(KEY, true, now), Ok(true));
        assert_eq!(env.follow(KEY, true, now), Ok(false), "the answer is awaited");
        let id = Uuid::from_u128(9);
        env.on_parcel_id(7, 3, Some(id));
        assert_eq!(env.follow(KEY, true, now + ASK_EVERY), Ok(false));
        // away and back: no second request
        assert_eq!(env.follow((7, 4), true, now), Ok(true));
        assert_eq!(env.follow(KEY, true, now), Ok(false));
        assert_eq!(env.here.as_ref().and_then(|h| h.id), Some(id));
    }

    #[test]
    fn an_answer_for_another_parcel_names_that_parcel() {
        let mut env = ParcelEnv::default();
        let now = Instant::now();
        let _ = env.follow(KEY, true, now);
        let id = Uuid::from_u128(9);
        env.on_parcel_id(7, 4, Some(id));
        assert_eq!(env.here.as_ref().and_then(|h| h.id), None);
        // the same local id in the region left behind
        env.on_parcel_id(8, 3, Some(id));
        assert_eq!(env.here.as_ref().and_then(|h| h.id), None);
        assert_eq!(env.follow((8, 3), true, now), Ok(false), "known for when the agent is there");
    }

    #[test]
    fn a_failed_request_is_retried_then_given_up() {
        let mut env = ParcelEnv::default();
        let mut now = Instant::now();
        for _ in 0..MAX_ASKS {
            assert_eq!(env.follow(KEY, true, now), Ok(true));
            env.on_parcel_id(7, 3, None);
            env.on_parcel_id(7, 3, Some(Uuid::nil()));
            now += ASK_EVERY;
        }
        assert_eq!(env.follow(KEY, true, now), Err(()));
    }

    fn a_world_on(local_id: i32) -> World {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.main_region = Some(7);
        put_on(&mut world, local_id);
        world
    }

    fn put_on(world: &mut World, local_id: i32) {
        world.parcel = Some(Arc::new(aurora_net::ParcelInfo {
            local_id,
            ..Default::default()
        }));
    }

    #[test]
    fn a_save_does_not_follow_the_agent_to_the_next_parcel() {
        let mut world = a_world_on(3);
        let now = Instant::now();
        world.save_parcel_environment();
        let (outcome, ask) = world.parcel_environment_frame(&[], now);
        assert_eq!(outcome, Outcome::Nothing);
        assert!(ask.is_some());
        put_on(&mut world, 4);
        world.parcel_env.on_parcel_id(7, 4, Some(Uuid::from_u128(9)));
        assert_eq!(world.parcel_environment_frame(&[], now).0, Outcome::Left);
        assert_eq!(world.parcel_env.saving, None);
        assert_eq!(world.parcel_environment_frame(&[], now).0, Outcome::Nothing);
    }

    #[test]
    fn the_last_request_has_its_time_to_answer() {
        let mut world = a_world_on(3);
        let mut now = Instant::now();
        world.save_parcel_environment();
        for _ in 0..MAX_ASKS {
            let (outcome, ask) = world.parcel_environment_frame(&[], now);
            assert_eq!(outcome, Outcome::Nothing);
            assert!(ask.is_some());
            now += ASK_EVERY;
        }
        assert_eq!(world.parcel_environment_frame(&[], now).0, Outcome::NotSaved);
    }

    #[test]
    fn saved_here_needs_the_parcel_in_the_list() {
        let mut env = ParcelEnv::default();
        let id = Uuid::from_u128(9);
        let entry = Entry {
            parcel: id,
            name: String::new(),
            region_name: String::new(),
            environment: None,
            time_of_day: 0,
        };
        let _ = env.follow(KEY, true, Instant::now());
        env.on_parcel_id(7, 3, Some(id));
        assert_eq!(env.saved_here(&[]), None);
        assert_eq!(env.saved_here(std::slice::from_ref(&entry)), Some(id));
    }
}
