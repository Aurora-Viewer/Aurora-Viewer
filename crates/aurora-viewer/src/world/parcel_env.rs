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
    /// "Save" was asked: done once the parcel's id and the settings are in.
    saving: bool,
}

/// What came of a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Saved(Entry),
    /// The parcel could not be identified: nothing was saved.
    NotSaved,
}

impl ParcelEnv {
    /// Save the local environment for this parcel, once it has loaded.
    pub fn request_save(&mut self) {
        self.saving = true;
    }

    /// The saved parcel under the agent, if any.
    pub fn saved_here(&self, saved: &[Entry]) -> Option<Uuid> {
        let id = self.here.as_ref()?.id?;
        saved.iter().any(|e| e.parcel == id).then_some(id)
    }

    /// RemoteParcelRequest answer. A nil or missing id is asked again.
    pub fn on_parcel_id(&mut self, local_id: i32, id: Option<Uuid>) {
        let Some(id) = id.filter(|id| !id.is_nil()) else { return };
        if let Some(here) = self.here.as_mut().filter(|h| h.key.1 == local_id) {
            here.id = Some(id);
            self.known.insert(here.key, id);
        }
    }

    /// Follow the agent to `key`. True when its id must be asked now;
    /// `Err` when it is asked in vain.
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
    /// One frame: ask for the parcel's id while it matters, put on or take
    /// off its saved environment, and do a save that was asked. What was
    /// shown stays until the parcel is known, so that a teleport does not
    /// flash the environment left behind.
    pub fn parcel_environment_frame(&mut self, saved: &[Entry], now: Instant) -> (Outcome, Option<NetCommand>) {
        let (Some(handle), Some(parcel)) = (self.main_region, self.parcel.clone()) else {
            return (Outcome::Nothing, None);
        };
        let key = (handle, parcel.local_id);
        let wanted = !saved.is_empty() || self.parcel_env.saving;
        let mut ask = None;
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
            Err(()) if self.parcel_env.saving => {
                self.parcel_env.saving = false;
                return (Outcome::NotSaved, None);
            }
            Err(()) => {}
        }
        // RLVa holds the environment (@setenv): decided once it lets go
        let Some(here) = self.parcel_env.here.as_mut().filter(|_| !self.eep.rlva_locked) else {
            return (Outcome::Nothing, ask);
        };
        let waiting = wanted && here.id.is_none() && here.asks < MAX_ASKS;
        if waiting {
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
        if !self.parcel_env.saving || self.eep.local_loading() {
            return (Outcome::Nothing, ask);
        }
        self.parcel_env.saving = false;
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
        env.on_parcel_id(3, Some(id));
        assert_eq!(env.follow(KEY, true, now + ASK_EVERY), Ok(false));
        // away and back: no second request
        assert_eq!(env.follow((7, 4), true, now), Ok(true));
        assert_eq!(env.follow(KEY, true, now), Ok(false));
        assert_eq!(env.here.as_ref().and_then(|h| h.id), Some(id));
    }

    #[test]
    fn an_answer_for_another_parcel_is_ignored() {
        let mut env = ParcelEnv::default();
        let _ = env.follow(KEY, true, Instant::now());
        env.on_parcel_id(4, Some(Uuid::from_u128(9)));
        assert_eq!(env.here.as_ref().and_then(|h| h.id), None);
    }

    #[test]
    fn a_failed_request_is_retried_then_given_up() {
        let mut env = ParcelEnv::default();
        let mut now = Instant::now();
        for _ in 0..MAX_ASKS {
            assert_eq!(env.follow(KEY, true, now), Ok(true));
            env.on_parcel_id(3, None);
            env.on_parcel_id(3, Some(Uuid::nil()));
            now += ASK_EVERY;
        }
        assert_eq!(env.follow(KEY, true, now), Err(()));
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
        env.on_parcel_id(3, Some(id));
        assert_eq!(env.saved_here(&[]), None);
        assert_eq!(env.saved_here(std::slice::from_ref(&entry)), Some(id));
    }
}
