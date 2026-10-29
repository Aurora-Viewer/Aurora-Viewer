//! The account's saved smart searches (advanced inventory search), shared by
//! every inventory window: the sidebar lists them, the chip bar and the rule
//! builder save into them. Aurora's own feature, no Legacy Viewer source.
//! Kept in `inventory-smart-searches.json` in the account's folder, read and
//! written by a thread of its own like the favourites' order
//! (`world/favorites.rs`); the demo keeps them in memory.

use super::smart::SmartQuery;
use crossbeam_channel::{self as mpsc, Sender};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

pub const FILE: &str = "inventory-smart-searches.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub id: u64,
    pub name: String,
    pub query: SmartQuery,
}

#[derive(Default)]
struct Store {
    saved: Vec<Saved>,
    /// Bumped by every change: lets a caller notice one cheaply.
    revision: u64,
    writer: Option<Sender<Vec<Saved>>>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(guard.get_or_insert_with(Store::default))
}

/// Changes the list, then hands the new list to the writer, if any.
fn change(f: impl FnOnce(&mut Store)) {
    with(|store| {
        f(store);
        store.revision += 1;
        if let Some(writer) = &store.writer {
            let _ = writer.send(store.saved.clone());
        }
    });
}

/// Opens the account's file at login: the searches are read off the frame
/// thread, then every change is written back atomically. Replaces the
/// searches of a previous account.
pub fn load(path: PathBuf) {
    let (writer, writes) = mpsc::unbounded::<Vec<Saved>>();
    with(|store| {
        *store = Store {
            revision: store.revision + 1,
            ..Store::default()
        };
        store.writer = Some(writer.clone());
    });
    let spawned = std::thread::Builder::new().name("aurora-smart-searches".into()).spawn(move || {
        let read = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Vec<Saved>>(&bytes).unwrap_or_else(|error| {
                log::warn!("Cannot read the saved smart searches: {error}");
                Vec::new()
            }),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("Cannot read the saved smart searches: {error}");
                }
                Vec::new()
            }
        };
        with(|store| {
            // this account still, and what was saved before the read stays after
            if store.writer.as_ref().is_some_and(|w| w.same_channel(&writer)) {
                let added = std::mem::replace(&mut store.saved, read);
                // The early searches keep their ids, which a window may
                // already hold (`active_smart`); a read one that collides
                // takes a fresh id, nothing refers to it yet.
                let mut next = next_id(&store.saved).max(next_id(&added));
                for saved in &mut store.saved {
                    if added.iter().any(|s| s.id == saved.id) {
                        saved.id = next;
                        next += 1;
                    }
                }
                let early = !added.is_empty();
                store.saved.extend(added);
                store.revision += 1;
                // supersedes the queued write of the early list alone
                if early {
                    let _ = writer.send(store.saved.clone());
                }
            }
        });
        drop(writer);
        while let Ok(mut saved) = writes.recv() {
            while let Ok(newer) = writes.try_recv() {
                saved = newer;
            }
            let save = || -> std::io::Result<()> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let bytes = serde_json::to_vec_pretty(&saved).map_err(std::io::Error::other)?;
                crate::atomic_file::write(&path, &bytes)
            };
            if let Err(error) = save() {
                log::warn!("Cannot save the smart searches: {error}");
            }
        }
    });
    if let Err(error) = spawned {
        log::warn!("Cannot start the smart searches' storage: {error}");
        with(|store| store.writer = None);
    }
}

fn next_id(saved: &[Saved]) -> u64 {
    saved.iter().map(|s| s.id).max().map_or(1, |id| id + 1)
}

/// The saved searches, in the order they were saved.
pub fn list() -> Vec<Saved> {
    with(|store| store.saved.clone())
}

pub fn get(id: u64) -> Option<Saved> {
    with(|store| store.saved.iter().find(|s| s.id == id).cloned())
}

/// Saves a new search; returns its id.
pub fn add(name: &str, query: SmartQuery) -> u64 {
    let mut id = 0;
    change(|store| {
        id = next_id(&store.saved);
        store.saved.push(Saved {
            id,
            name: name.trim().to_owned(),
            query,
        });
    });
    id
}

pub fn rename(id: u64, name: &str) {
    change(|store| {
        if let Some(saved) = store.saved.iter_mut().find(|s| s.id == id) {
            saved.name = name.trim().to_owned();
        }
    });
}

/// Replaces the rules of a saved search.
pub fn update(id: u64, query: SmartQuery) {
    change(|store| {
        if let Some(saved) = store.saved.iter_mut().find(|s| s.id == id) {
            saved.query = query;
        }
    });
}

pub fn delete(id: u64) {
    change(|store| store.saved.retain(|s| s.id != id));
}
