//! Environments kept for a parcel: the local environment the user saved
//! there comes back each time they stand on that parcel, and what they had
//! before comes back when they leave it. An Aurora feature (the Legacy Viewer
//! keeps one local environment, LLEnvironment::saveToSettings).
//!
//! A parcel is known by its region's UUID (RegionHandshake) and its local id
//! (ParcelProperties), as the Legacy Viewer tells parcels apart for media
//! autoplay (LLViewerParcelMediaAutoPlay::idle,
//! indra/newview/llviewerparcelmediaautoplay.cpp): nothing is asked of the
//! server. A local id is given out again once parcels are joined or divided,
//! so a save also keeps the parcel's area and bounding box, and is not put on
//! where they no longer match.
//!
//! The saves hold the asset ids of what was applied, like the Legacy Viewer's
//! own local environment: they stay in the account's file for the grid
//! (`parcel_environments-<grid>.json`), never in the shared settings nor in
//! a settings backup.

use super::World;
use crossbeam_channel::{self as mpsc, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::thread::JoinHandle;
use uuid::Uuid;

/// The account's file on one grid (`grid` from `chat_log::grid_name`).
pub fn file_name(grid: &str) -> String {
    format!("parcel_environments-{grid}.json")
}

/// A parcel, and what tells it from a parcel that took its local id later.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    pub region: Uuid,
    pub local_id: i32,
    /// The parcel's area (m²) and bounding box: a join or a division
    /// changes them.
    pub area: i32,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}

impl Place {
    /// None before the region's UUID or the parcel's local id is known.
    pub fn of(region: Uuid, parcel: &aurora_net::ParcelInfo) -> Option<Self> {
        (!region.is_nil() && parcel.local_id > 0).then(|| Place {
            region,
            local_id: parcel.local_id,
            area: parcel.area,
            aabb_min: parcel.aabb_min.to_array(),
            aabb_max: parcel.aabb_max.to_array(),
        })
    }

    /// The same region and local id, whatever the parcel's shape now.
    pub fn same_id(&self, other: &Place) -> bool {
        self.region == other.region && self.local_id == other.local_id
    }
}

/// A saved environment, for one parcel.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub place: Place,
    pub name: String,
    pub region_name: String,
    /// `EnvSelector::saved_local`, in LLSD notation.
    pub environment: Option<String>,
    /// Time of day preset (0: none).
    pub time_of_day: u8,
}

/// A row of Preferences' list: what the user reads, never an asset id.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub region: Uuid,
    pub local_id: i32,
    pub name: String,
    pub region_name: String,
}

/// A new save, in place of the one it replaces.
fn store_in(saved: &mut Vec<Entry>, entry: Entry) {
    saved.retain(|e| !e.place.same_id(&entry.place));
    saved.push(entry);
}

/// The account's file: an entry that cannot be read, or names no parcel, is
/// left out, never the whole list.
pub fn read(bytes: &[u8]) -> Vec<Entry> {
    let values: Vec<serde_json::Value> = match serde_json::from_slice(bytes) {
        Ok(values) => values,
        Err(error) => {
            log::warn!("Cannot read the parcels' saved environments: {error}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for value in values {
        match serde_json::from_value::<Entry>(value) {
            Ok(entry) if !entry.place.region.is_nil() && entry.place.local_id > 0 => store_in(&mut out, entry),
            Ok(_) => log::warn!("A parcel's saved environment names no parcel: left out"),
            Err(error) => log::warn!("A parcel's saved environment cannot be read, left out: {error}"),
        }
    }
    out
}

/// The account's file, read and written by a thread of its own
/// (`favorites.rs`).
struct Storage {
    loaded: Receiver<Vec<Entry>>,
    writer: Option<Sender<Vec<Entry>>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The saves of the account on the grid of the login; the demo keeps them
/// in memory (Default).
#[derive(Default)]
pub struct Saves {
    entries: Vec<Entry>,
    storage: Option<Storage>,
}

impl Saves {
    /// The account's file, read off the frame thread.
    pub fn load(path: PathBuf) -> Self {
        let (loaded_tx, loaded) = mpsc::unbounded();
        let (writer, writes) = mpsc::unbounded::<Vec<Entry>>();
        let worker = std::thread::Builder::new()
            .name("aurora-parcel-environments".into())
            .spawn(move || {
                let entries = match std::fs::read(&path) {
                    Ok(bytes) => read(&bytes),
                    Err(error) => {
                        if error.kind() != std::io::ErrorKind::NotFound {
                            log::warn!("Cannot read the parcels' saved environments: {error}");
                        }
                        Vec::new()
                    }
                };
                let _ = loaded_tx.send(entries);
                crate::save_retry::write_latest("parcels' saved environments", &writes, |entries| {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = serde_json::to_vec(entries).map_err(std::io::Error::other)?;
                    crate::atomic_file::write(&path, &bytes)
                });
            });
        match worker {
            Ok(worker) => Saves {
                storage: Some(Storage {
                    loaded,
                    writer: Some(writer),
                    worker: Some(worker),
                }),
                ..Saves::default()
            },
            Err(error) => {
                log::warn!("Cannot start the parcels' saved environments' storage: {error}");
                Saves::default()
            }
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Preferences' list.
    pub fn rows(&self) -> Vec<Row> {
        self.entries
            .iter()
            .map(|e| Row {
                region: e.place.region,
                local_id: e.place.local_id,
                name: e.name.clone(),
                region_name: e.region_name.clone(),
            })
            .collect()
    }

    /// The file's entries once read: a save made before they came stays, and
    /// the file is written again with both (that save's own write held only
    /// it).
    pub fn poll(&mut self) {
        if let Some(loaded) = self.storage.as_ref().and_then(|s| s.loaded.try_recv().ok()) {
            let saved_before = !self.entries.is_empty();
            for entry in loaded {
                if !self.entries.iter().any(|e| e.place.same_id(&entry.place)) {
                    self.entries.push(entry);
                }
            }
            if saved_before {
                self.changed();
            }
        }
    }

    pub fn store(&mut self, entry: Entry) {
        store_in(&mut self.entries, entry);
        self.changed();
    }

    pub fn forget(&mut self, region: Uuid, local_id: i32) {
        let before = self.entries.len();
        self.entries.retain(|e| !(e.place.region == region && e.place.local_id == local_id));
        if self.entries.len() != before {
            self.changed();
        }
    }

    /// "Remove all": every save of this grid (the file holds no other).
    pub fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.changed();
        }
    }

    fn changed(&mut self) {
        if let Some(writer) = self.storage.as_ref().and_then(|s| s.writer.as_ref()) {
            let _ = writer.send(self.entries.clone());
        }
    }
}

/// A save asked: for this parcel, or before the parcel was known.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Saving {
    At(Place),
    Unknown,
}

/// The parcel under the agent and what was decided for it.
#[derive(Debug)]
struct Here {
    place: Place,
    /// The selector was told about this parcel.
    settled: bool,
    /// Its saved environment is shown.
    shown: bool,
    /// A save for its local id was left off, its shape having changed (told
    /// once in the log).
    told_mismatch: bool,
    /// That environment as it stands once loaded (`saved_local` form, preset):
    /// a save read back need not compare equal to what was saved.
    baseline: Option<(Option<aurora_llsd::Llsd>, u8)>,
    /// `EnvSelector::local_stamp` of the last comparison with it.
    stamp: Option<(u64, u8)>,
    /// The user changed the environment shown.
    changed: bool,
}

impl Here {
    fn new(place: Place) -> Self {
        Here {
            place,
            settled: false,
            shown: false,
            told_mismatch: false,
            baseline: None,
            stamp: None,
            changed: false,
        }
    }

    /// The saved environment was put on, or replaced: compare with it anew.
    fn forget_baseline(&mut self) {
        self.baseline = None;
        self.stamp = None;
        self.changed = false;
    }
}

#[derive(Default)]
pub struct ParcelEnv {
    here: Option<Here>,
    /// "Save" was asked: done once the settings are in, dropped if the agent
    /// leaves the parcel first.
    saving: Option<Saving>,
    pub saves: Saves,
}

/// What came of a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Nothing,
    Saved(Entry),
    /// The parcel was not known yet: nothing was saved.
    NotSaved,
    /// The agent left the parcel before the save could be done.
    Left,
}

impl ParcelEnv {
    /// A save fits the parcel under the agent, as it stands.
    pub fn saved_here(&self) -> bool {
        self.here
            .as_ref()
            .is_some_and(|h| self.saves.entries.iter().any(|e| e.place == h.place))
    }

    /// The pin, lit: the parcel's save is forgotten.
    pub fn forget_here(&mut self) {
        if let Some(place) = self.here.as_ref().map(|h| h.place) {
            self.saves.forget(place.region, place.local_id);
        }
    }
}

impl World {
    /// The parcel under the agent, once its region and local id are known.
    fn agent_place(&self) -> Option<Place> {
        let region = self.regions.get(&self.main_region?)?.info.as_ref()?.region_id;
        Place::of(region, self.parcel.as_deref()?)
    }

    /// Save the local environment for the parcel under the agent, once it
    /// has loaded. Refused while RLVa holds the environment (@setenv): a
    /// save left waiting would be made once the lock lifts, unasked.
    pub fn save_parcel_environment(&mut self) {
        if self.eep.rlva_locked {
            return;
        }
        self.eep.arm_parcel_save();
        self.parcel_env.saving = Some(self.agent_place().map_or(Saving::Unknown, Saving::At));
    }

    /// One frame: put on or take off the parcel's saved environment, and do
    /// a save that was asked. Decided from the parcel as its latest
    /// ParcelProperties tell it, so a new local id, area or bounding box (a
    /// join, a division) decides anew. What was shown stays until the parcel
    /// is known, so that a teleport does not flash the environment left
    /// behind.
    pub fn parcel_environment_frame(&mut self) -> Outcome {
        self.parcel_env.saves.poll();
        // RLVa holds the environment (@setenv): a save asked before the lock
        // came is dropped quietly, as the lock refuses the pick itself; the
        // rest is decided once it lets go
        if self.eep.rlva_locked {
            self.parcel_env.saving = None;
            return Outcome::Nothing;
        }
        if self.parcel_env.saving == Some(Saving::Unknown) {
            self.parcel_env.saving = None;
            return Outcome::NotSaved;
        }
        let Some(place) = self.agent_place() else {
            return Outcome::Nothing;
        };
        if let Some(Saving::At(asked)) = self.parcel_env.saving
            && !asked.same_id(&place)
        {
            self.parcel_env.saving = None;
            return Outcome::Left;
        }
        let ParcelEnv { here, saving, saves } = &mut self.parcel_env;
        if here.as_ref().is_none_or(|h| h.place != place) {
            *here = Some(Here::new(place));
        }
        let Some(here) = here.as_mut() else {
            return Outcome::Nothing;
        };
        let found = saves.entries.iter().find(|e| e.place.same_id(&place));
        let target = found.filter(|e| e.place == place);
        if found.is_some() && target.is_none() && !here.told_mismatch {
            here.told_mismatch = true;
            log::info!(
                "Parcel {} of region {}: its area or bounds changed since its environment was saved, not put on",
                place.local_id,
                place.region
            );
        }
        // a save takes what is shown: putting the parcel's own on now would
        // replace the pick it waits for
        if saving.is_none() && (!here.settled || here.shown != target.is_some()) {
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
            here.shown = target.is_some();
            here.forget_baseline();
        }
        if saving.is_none() || self.eep.local_loading() {
            return Outcome::Nothing;
        }
        *saving = None;
        // a picked item that could not load (FailedToFindSettings says so)
        let Some((environment, time_of_day)) = self.eep.capture_parcel() else {
            return Outcome::Nothing;
        };
        here.settled = true;
        here.shown = true;
        here.forget_baseline();
        let entry = Entry {
            place,
            name: self.parcel.as_ref().map_or_else(String::new, |p| p.name.clone()),
            region_name: self
                .main_region
                .and_then(|h| self.regions.get(&h))
                .map_or_else(String::new, |r| r.name.clone()),
            environment: environment.map(|e| aurora_llsd::to_notation(&e)),
            time_of_day,
        };
        saves.store(entry.clone());
        Outcome::Saved(entry)
    }

    /// The environment shown here is no longer the parcel's saved one, so
    /// that saving would change something. Compared only when the local
    /// environment changes.
    pub fn parcel_environment_changed(&mut self) -> bool {
        let Some(here) = self.parcel_env.here.as_mut() else {
            return false;
        };
        if !here.shown || self.eep.local_loading() {
            return here.changed;
        }
        let stamp = self.eep.local_stamp();
        if here.stamp != Some(stamp) {
            here.stamp = Some(stamp);
            let now = self.eep.parcel_form();
            here.changed = here.baseline.as_ref().is_some_and(|b| *b != now);
            here.baseline.get_or_insert(now);
        }
        here.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const HANDLE: aurora_net::RegionHandle = 7;
    const REGION: Uuid = Uuid::from_u128(0x946);

    fn region_info(handle: aurora_net::RegionHandle, region_id: Uuid) -> aurora_net::RegionInfo {
        aurora_net::RegionInfo {
            handle,
            name: "Test Region".into(),
            region_id,
            water_height: 20.0,
            sim_access: 0,
            region_flags: 0,
            terrain_base: [Uuid::nil(); 4],
            terrain_detail: [Uuid::nil(); 4],
            terrain_start_height: [0.0; 4],
            terrain_height_range: [0.0; 4],
            size_x: 256,
            size_y: 256,
            is_main: true,
            owner: Uuid::nil(),
            is_estate_manager: false,
            product_name: String::new(),
            billable_factor: 1.0,
        }
    }

    fn a_world_on(local_id: i32) -> World {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.main_region = Some(HANDLE);
        world.ensure_region(HANDLE, (256, 256)).info = Some(Arc::new(region_info(HANDLE, REGION)));
        put_on(&mut world, local_id, 1024);
        world
    }

    fn put_on(world: &mut World, local_id: i32, area: i32) {
        world.parcel = Some(Arc::new(aurora_net::ParcelInfo {
            local_id,
            area,
            aabb_min: glam::Vec3::ZERO,
            aabb_max: glam::Vec3::new(32.0, area as f32 / 32.0, 0.0),
            ..Default::default()
        }));
    }

    fn place_of(world: &World) -> Place {
        world.agent_place().expect("a parcel")
    }

    fn entry(place: Place, time_of_day: u8) -> Entry {
        Entry {
            place,
            time_of_day,
            ..Entry::default()
        }
    }

    fn sky(time_of_day: u8, version: f32) -> (Uuid, Option<Arc<crate::world::eep::Settings>>) {
        let sky = crate::world::env::time_of_day_sky(time_of_day).expect("a library sky");
        (sky, Some(crate::world::eep_env::tests::sky_asset(version)))
    }

    fn pick(world: &mut World, time_of_day: u8, version: f32) {
        world.eep.set_time_of_day(time_of_day);
        let (sky, asset) = sky(time_of_day, version);
        world.eep.on_settings(sky, asset);
    }

    #[test]
    fn a_parcel_is_known_by_its_region_and_local_id() {
        let world = a_world_on(3);
        let place = place_of(&world);
        assert_eq!((place.region, place.local_id, place.area), (REGION, 3, 1024));
    }

    #[test]
    fn a_parcel_is_not_known_before_its_region_says_its_uuid() {
        let mut world = a_world_on(3);
        world.ensure_region(HANDLE, (256, 256)).info = Some(Arc::new(region_info(HANDLE, Uuid::nil())));
        assert_eq!(world.agent_place(), None);
        world.ensure_region(HANDLE, (256, 256)).info = Some(Arc::new(region_info(HANDLE, REGION)));
        put_on(&mut world, 0, 1024);
        assert_eq!(world.agent_place(), None, "no local id yet");
    }

    #[test]
    fn a_saved_parcel_gets_its_environment_without_asking_the_server() {
        let mut world = a_world_on(3);
        let place = place_of(&world);
        world.parcel_env.saves.store(entry(place, 2));
        // the frame has nothing to send: no RemoteParcelRequest, no other
        let outcome: Outcome = world.parcel_environment_frame();
        assert_eq!(outcome, Outcome::Nothing);
        assert_eq!(world.eep.parcel_form().1, 2, "its preset is on");
        assert!(world.parcel_env.saved_here());
    }

    #[test]
    fn the_same_local_id_in_another_region_is_another_parcel() {
        let mut world = a_world_on(3);
        let elsewhere = Place {
            region: Uuid::from_u128(0x947),
            ..place_of(&world)
        };
        world.parcel_env.saves.store(entry(elsewhere, 2));
        let _ = world.parcel_environment_frame();
        assert!(!world.parcel_env.saved_here());
        assert_eq!(world.eep.parcel_form().1, 0, "its preset stays off");
    }

    #[test]
    fn a_save_for_a_parcel_that_changed_shape_is_not_put_on() {
        let mut world = a_world_on(3);
        let place = place_of(&world);
        world.parcel_env.saves.store(entry(place, 2));
        // the parcel was joined to its neighbour: same local id, larger
        put_on(&mut world, 3, 2048);
        let _ = world.parcel_environment_frame();
        assert!(!world.parcel_env.saved_here());
        assert_eq!(world.eep.parcel_form().1, 0, "its preset stays off");
        assert_eq!(world.parcel_env.saves.entries().len(), 1, "the save is kept");
    }

    #[test]
    fn a_change_of_shape_on_the_parcel_decides_anew() {
        let mut world = a_world_on(3);
        let place = place_of(&world);
        world.parcel_env.saves.store(entry(place, 2));
        let _ = world.parcel_environment_frame();
        assert_eq!(world.eep.parcel_form().1, 2);
        // divided: the agent's part keeps the local id, smaller
        put_on(&mut world, 3, 512);
        let _ = world.parcel_environment_frame();
        assert_eq!(world.eep.parcel_form().1, 0, "taken off");
        put_on(&mut world, 3, 1024);
        let _ = world.parcel_environment_frame();
        assert_eq!(world.eep.parcel_form().1, 2, "put back on");
    }

    #[test]
    fn a_save_does_not_follow_the_agent_to_the_next_parcel() {
        let mut world = a_world_on(3);
        world.save_parcel_environment();
        // walked off before the settings were in
        put_on(&mut world, 4, 1024);
        assert_eq!(world.parcel_environment_frame(), Outcome::Left);
        assert_eq!(world.parcel_env.saving, None);
        assert_eq!(world.parcel_environment_frame(), Outcome::Nothing);
    }

    #[test]
    fn a_save_before_the_parcel_is_known_is_not_made() {
        let mut world = a_world_on(3);
        world.parcel = None;
        world.save_parcel_environment();
        assert_eq!(world.parcel_env.saving, Some(Saving::Unknown));
        put_on(&mut world, 3, 1024);
        assert_eq!(world.parcel_environment_frame(), Outcome::NotSaved);
        assert!(world.parcel_env.saves.entries().is_empty());
    }

    #[test]
    fn a_save_is_made_for_the_parcel_and_kept() {
        let mut world = a_world_on(3);
        pick(&mut world, 4, 1.1);
        world.save_parcel_environment();
        let Outcome::Saved(new) = world.parcel_environment_frame() else {
            panic!("saved");
        };
        assert_eq!((new.place, new.time_of_day), (place_of(&world), 4));
        assert_eq!(world.parcel_env.saves.entries(), std::slice::from_ref(&new));
        assert!(world.parcel_env.saved_here());
    }

    #[test]
    fn a_save_on_a_saved_parcel_keeps_what_was_picked() {
        let mut world = a_world_on(3);
        let place = place_of(&world);
        world.parcel_env.saves.store(entry(place, 2));
        let _ = world.parcel_environment_frame();
        pick(&mut world, 4, 1.1);
        world.save_parcel_environment();
        let Outcome::Saved(new) = world.parcel_environment_frame() else {
            panic!("saved");
        };
        assert_eq!(new.time_of_day, 4);
        assert_eq!(world.parcel_environment_frame(), Outcome::Nothing);
        assert_eq!(world.eep.parcel_form().1, 4, "the pick stays on");
        assert_eq!(world.parcel_env.saves.entries().len(), 1, "replaced");
    }

    #[test]
    fn a_saved_environment_reads_as_changed_after_a_change_only() {
        let mut world = a_world_on(3);
        let place = place_of(&world);
        world.parcel_env.saves.store(entry(place, 2));
        let _ = world.parcel_environment_frame();
        assert!(!world.parcel_environment_changed(), "still loading");
        let (sky2, asset) = sky(2, 1.1);
        world.eep.on_settings(sky2, asset);
        assert!(!world.parcel_environment_changed(), "as saved");
        pick(&mut world, 4, 1.2);
        assert!(world.parcel_environment_changed());
        world.save_parcel_environment();
        let Outcome::Saved(_) = world.parcel_environment_frame() else {
            panic!("saved");
        };
        let _ = world.parcel_environment_frame();
        assert!(!world.parcel_environment_changed(), "saved over");
    }

    #[test]
    fn forgetting_here_takes_the_save_away() {
        let mut world = a_world_on(3);
        let place = place_of(&world);
        world.parcel_env.saves.store(entry(place, 2));
        let _ = world.parcel_environment_frame();
        world.parcel_env.forget_here();
        assert!(world.parcel_env.saves.entries().is_empty());
        let _ = world.parcel_environment_frame();
        assert_eq!(world.eep.parcel_form().1, 0, "taken off");
    }

    #[test]
    fn no_save_is_asked_while_rlva_holds_the_environment() {
        let mut world = a_world_on(3);
        world.eep.rlva_locked = true;
        world.save_parcel_environment();
        assert_eq!(world.parcel_env.saving, None);
        world.eep.rlva_locked = false;
        assert_eq!(world.parcel_environment_frame(), Outcome::Nothing);
        assert_eq!(world.parcel_env.saving, None, "nothing saved once the lock lifts");
    }

    #[test]
    fn a_save_waiting_when_rlva_holds_the_environment_is_dropped() {
        let mut world = a_world_on(3);
        world.save_parcel_environment();
        assert!(world.parcel_env.saving.is_some());
        world.eep.rlva_locked = true;
        assert_eq!(world.parcel_environment_frame(), Outcome::Nothing);
        assert_eq!(world.parcel_env.saving, None, "dropped when the lock came");
        world.eep.rlva_locked = false;
        assert_eq!(world.parcel_environment_frame(), Outcome::Nothing);
        assert!(world.parcel_env.saves.entries().is_empty());
    }

    fn place(local_id: i32) -> Place {
        Place {
            region: REGION,
            local_id,
            area: 1024,
            ..Place::default()
        }
    }

    #[test]
    fn a_save_replaces_the_one_for_its_parcel_only() {
        let mut saves = Saves::default();
        saves.store(entry(place(3), 2));
        saves.store(entry(place(4), 2));
        saves.store(entry(place(3), 4));
        assert_eq!(saves.entries(), [entry(place(4), 2), entry(place(3), 4)]);
        saves.forget(REGION, 4);
        assert_eq!(saves.entries(), [entry(place(3), 4)]);
        saves.clear();
        assert!(saves.entries().is_empty());
    }

    #[test]
    fn an_entry_that_cannot_be_read_is_left_out_not_the_file() {
        let good = serde_json::to_value(entry(place(3), 2)).unwrap();
        let file = serde_json::json!([
            good,
            {"place": {"region": "not a uuid"}},
            {"place": {"region": REGION, "local_id": 0}},
            {"name": "no place at all"},
            {"place": {"region": REGION, "local_id": 5}, "time_of_day": 3, "unknown": true},
        ]);
        let entries = read(file.to_string().as_bytes());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], entry(place(3), 2));
        assert_eq!(
            (entries[1].place.local_id, entries[1].time_of_day),
            (5, 3),
            "missing fields default"
        );
        assert!(read(b"{not json").is_empty());
    }

    #[test]
    fn the_list_shows_no_asset_id() {
        let asset = Uuid::from_u128(0xabcdef);
        let mut saves = Saves::default();
        saves.store(Entry {
            name: "Home".into(),
            environment: Some(format!("{{'sky_id':u{asset}}}")),
            ..entry(place(3), 2)
        });
        let rows = saves.rows();
        assert_eq!(rows.len(), 1);
        assert!(!format!("{rows:?}").contains(&asset.to_string()));
    }

    #[test]
    fn the_saves_survive_a_restart_in_their_file() {
        let dir = std::env::temp_dir().join(format!("aurora-parcel-env-{}", Uuid::new_v4()));
        let path = dir.join("account").join(file_name("agni"));
        {
            let mut saves = Saves::load(path.clone());
            while saves.storage.as_ref().is_some_and(|s| s.loaded.is_empty()) {
                std::thread::yield_now();
            }
            saves.poll();
            saves.store(entry(place(3), 2));
        }
        let mut saves = Saves::load(path);
        while saves.storage.as_ref().is_some_and(|s| s.loaded.is_empty()) {
            std::thread::yield_now();
        }
        saves.poll();
        assert_eq!(saves.entries(), [entry(place(3), 2)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_save_made_before_the_file_is_read_stays() {
        let dir = std::env::temp_dir().join(format!("aurora-parcel-env-{}", Uuid::new_v4()));
        let path = dir.join(file_name("agni"));
        std::fs::create_dir_all(&dir).unwrap();
        let old = vec![entry(place(3), 2), entry(place(4), 2)];
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let mut saves = Saves::load(path.clone());
        saves.store(entry(place(3), 4));
        while saves.storage.as_ref().is_some_and(|s| s.loaded.is_empty()) {
            std::thread::yield_now();
        }
        saves.poll();
        assert_eq!(saves.entries(), [entry(place(3), 4), entry(place(4), 2)]);
        drop(saves);
        let written = read(&std::fs::read(&path).unwrap());
        assert_eq!(written, [entry(place(3), 4), entry(place(4), 2)], "both are in the file");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn each_grid_has_its_own_file() {
        assert_eq!(file_name("agni"), "parcel_environments-agni.json");
        assert_ne!(file_name("agni"), file_name("aditi"));
    }
}
