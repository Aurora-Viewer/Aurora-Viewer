//! Where each entry of the Trash came from, so that "Restore" puts it back
//! in its own folder, as Windows' Recycle Bin restores to the original
//! location (#2520). Aurora difference: the Legacy Viewer's Restore
//! (LLItemBridge::restoreItem / LLFolderBridge::restoreItem,
//! indra/newview/llinventorybridge.cpp) moves the entry to the default
//! folder of its type; that stays the fallback when the folder is unknown
//! (deleted from another viewer, or before this was kept) or gone.
//!
//! Every move into the Trash the server accepts is noted, whatever made it;
//! an entry that leaves the Trash is forgotten. The account keeps them in a
//! small file (`trash_origins.json`, the newest [`LIMIT`]), read and written
//! by a thread of its own: they survive a relog. The demo keeps them in
//! memory.

use super::Inventory;
use super::actions as rules;
use super::undo::Moved;
use crossbeam_channel::{self as mpsc, Receiver, Sender};
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;
use uuid::Uuid;

pub const FILE: &str = "trash_origins.json";
/// The entries kept: the newest ones.
pub const LIMIT: usize = 2000;

/// The file: (entry, folder), oldest first.
type Saved = Vec<(Uuid, Uuid)>;

struct Storage {
    loaded: Receiver<Saved>,
    writer: Option<Sender<Saved>>,
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

#[derive(Default)]
pub struct TrashOrigins {
    /// Each entry's folder and when it was noted (oldest smallest).
    origins: HashMap<Uuid, (Uuid, u64)>,
    next: u64,
    /// The moves of requests on their way.
    pending: HashMap<Uuid, Vec<Moved>>,
    storage: Option<Storage>,
}

impl std::fmt::Debug for TrashOrigins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrashOrigins").field("origins", &self.origins.len()).finish()
    }
}

impl TrashOrigins {
    /// The account's file, read off the frame thread.
    pub fn load(path: PathBuf) -> Self {
        let (loaded_tx, loaded) = mpsc::unbounded();
        let (writer, writes) = mpsc::unbounded::<Saved>();
        let worker = std::thread::Builder::new().name("aurora-trash-origins".into()).spawn(move || {
            let saved = match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice::<Saved>(&bytes).unwrap_or_else(|error| {
                    log::warn!("Cannot read the Trash's origins: {error}");
                    Vec::new()
                }),
                Err(error) => {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("Cannot read the Trash's origins: {error}");
                    }
                    Vec::new()
                }
            };
            let _ = loaded_tx.send(saved);
            crate::save_retry::write_latest("Trash's origins", &writes, |saved| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let bytes = serde_json::to_vec(saved).map_err(std::io::Error::other)?;
                crate::atomic_file::write(&path, &bytes)
            });
        });
        match worker {
            Ok(worker) => TrashOrigins {
                storage: Some(Storage {
                    loaded,
                    writer: Some(writer),
                    worker: Some(worker),
                }),
                ..TrashOrigins::default()
            },
            Err(error) => {
                log::warn!("Cannot start the Trash's origins storage: {error}");
                TrashOrigins::default()
            }
        }
    }

    /// Takes the file once it is read: what was noted meanwhile is newer
    /// and stays.
    pub fn poll(&mut self) {
        let Some(saved) = self.storage.as_ref().and_then(|s| s.loaded.try_recv().ok()) else {
            return;
        };
        let noted = std::mem::take(&mut self.origins);
        self.next = 0;
        for (id, folder) in saved {
            self.note(id, folder);
        }
        let mut noted: Vec<_> = noted.into_iter().collect();
        noted.sort_by_key(|(_, (_, at))| *at);
        for (id, (folder, _)) in noted {
            self.note(id, folder);
        }
        self.trim();
    }

    fn note(&mut self, id: Uuid, folder: Uuid) {
        self.origins.insert(id, (folder, self.next));
        self.next += 1;
    }

    fn trim(&mut self) {
        if self.origins.len() > LIMIT {
            let mut by_age: Vec<_> = self.origins.iter().map(|(id, (_, at))| (*at, *id)).collect();
            by_age.sort_unstable();
            for (_, id) in by_age.into_iter().take(self.origins.len() - LIMIT) {
                self.origins.remove(&id);
            }
        }
    }

    fn save(&self) {
        if let Some(writer) = self.storage.as_ref().and_then(|s| s.writer.as_ref()) {
            let mut saved: Vec<_> = self.origins.iter().map(|(id, (folder, at))| (*at, *id, *folder)).collect();
            saved.sort_unstable();
            let _ = writer.send(saved.into_iter().map(|(_, id, folder)| (id, folder)).collect());
        }
    }

    /// The moves of a change leaving as `request` (`moves`), kept until
    /// the server answers it.
    pub fn begin(&mut self, request: Uuid, moves: Vec<Moved>) {
        if !moves.is_empty() {
            self.pending.insert(request, moves);
        }
    }

    /// The moves of `change`, each from where it is now.
    pub fn moves(inv: &Inventory, change: &aurora_net::inventory::operations::Mutation) -> Vec<Moved> {
        change
            .operations
            .iter()
            .filter_map(|op| match op {
                aurora_net::inventory::operations::Operation::Patch { id, body, .. } if body.has("parent_id") => Some(Moved {
                    id: *id,
                    from: rules::parent(inv, *id)?,
                    to: body["parent_id"].as_uuid(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The server answered `request`: its moves into the Trash are noted,
    /// its moves out of it forgotten.
    pub fn finish(&mut self, request: Uuid, inv: &Inventory, accepted: bool) {
        let Some(moves) = self.pending.remove(&request) else { return };
        if accepted && self.moved(inv, &moves) {
            self.save();
        }
    }

    /// Notes `moves` (see `finish`); whether anything changed.
    fn moved(&mut self, inv: &Inventory, moves: &[Moved]) -> bool {
        let Some(trash) = rules::system(inv, 14) else { return false };
        let in_trash = |folder: Uuid| folder == trash || rules::under(inv, folder, trash);
        let mut changed = false;
        for m in moves {
            match (in_trash(m.from), in_trash(m.to)) {
                // into the Trash from a folder of the user's
                (false, true) => {
                    self.note(m.id, m.from);
                    changed = true;
                }
                // out of it
                (true, false) => changed |= self.origins.remove(&m.id).is_some(),
                // within the Trash, or anywhere else: nothing to note
                _ => {}
            }
        }
        if changed {
            self.trim();
        }
        changed
    }

    /// The folder `id` came from, when it is still there to take it back
    /// (not in the Trash itself, and a folder that receives entries).
    pub fn origin(&self, inv: &Inventory, id: Uuid) -> Option<Uuid> {
        self.origins
            .get(&id)
            .map(|(folder, _)| *folder)
            .filter(|folder| rules::destination(inv, *folder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::undo::tests_support::fixture;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn a_move_to_the_trash_remembers_its_folder_and_leaving_it_forgets() {
        let (inv, ids) = fixture();
        let mut origins = TrashOrigins::default();
        let into = [Moved {
            id: ids.shirt,
            from: ids.clothes,
            to: ids.trash,
        }];
        assert!(origins.moved(&inv, &into));
        assert_eq!(origins.origin(&inv, ids.shirt), Some(ids.clothes));
        // within the Trash: the folder it came from stays
        assert!(!origins.moved(
            &inv,
            &[Moved {
                id: ids.shirt,
                from: ids.trash,
                to: ids.trash_sub,
            }]
        ));
        assert_eq!(origins.origin(&inv, ids.shirt), Some(ids.clothes));
        // out of it
        assert!(origins.moved(
            &inv,
            &[Moved {
                id: ids.shirt,
                from: ids.trash,
                to: ids.objects,
            }]
        ));
        assert_eq!(origins.origin(&inv, ids.shirt), None);
        // an ordinary move is nothing to note
        assert!(!origins.moved(
            &inv,
            &[Moved {
                id: ids.hat,
                from: ids.clothes,
                to: ids.objects,
            }]
        ));
    }

    #[test]
    fn a_folder_that_is_gone_or_in_the_trash_is_no_origin() {
        let (mut inv, ids) = fixture();
        let mut origins = TrashOrigins::default();
        origins.note(ids.shirt, ids.clothes);
        origins.note(ids.hat, id(4242));
        assert_eq!(origins.origin(&inv, ids.hat), None, "unknown folder");
        // the folder itself went to the Trash
        let to_trash =
            crate::world::inventory::actions::move_selection(&inv, &[ids.clothes], ids.trash, &Default::default(), &Default::default())
                .expect("to the Trash");
        crate::world::inventory::actions::demo_mutate(&mut inv, ids.agent, to_trash).expect("moved");
        assert_eq!(origins.origin(&inv, ids.shirt), None);
    }

    #[test]
    fn only_the_newest_are_kept_and_the_file_read_late_stays_older() {
        let mut origins = TrashOrigins::default();
        for n in 0..(LIMIT as u128 + 10) {
            origins.note(id(10_000 + n), id(1));
        }
        origins.trim();
        assert_eq!(origins.origins.len(), LIMIT);
        assert!(!origins.origins.contains_key(&id(10_000)), "the oldest go");
        assert!(origins.origins.contains_key(&id(10_000 + LIMIT as u128 + 9)));

        // a file read after a move: the move is newer
        let (tx, loaded) = mpsc::unbounded();
        let mut origins = TrashOrigins {
            storage: Some(Storage {
                loaded,
                writer: None,
                worker: None,
            }),
            ..Default::default()
        };
        origins.note(id(1), id(100));
        tx.send(vec![(id(1), id(200)), (id(2), id(200))]).expect("send");
        origins.poll();
        assert_eq!(origins.origins[&id(1)].0, id(100));
        assert_eq!(origins.origins[&id(2)].0, id(200));
        assert!(origins.origins[&id(1)].1 > origins.origins[&id(2)].1);
    }

    #[test]
    fn the_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("aurora-trash-origins-{}", Uuid::new_v4()));
        let path = dir.join("account").join(FILE);
        let wait = |origins: &TrashOrigins| {
            while origins.storage.as_ref().is_some_and(|s| s.loaded.is_empty()) {
                std::thread::yield_now();
            }
        };
        {
            let mut origins = TrashOrigins::load(path.clone());
            wait(&origins);
            origins.poll();
            origins.note(id(1), id(100));
            origins.note(id(2), id(200));
            origins.save();
        }
        let mut origins = TrashOrigins::load(path);
        wait(&origins);
        origins.poll();
        assert_eq!(origins.origins.get(&id(2)).map(|o| o.0), Some(id(200)));
        assert!(origins.origins[&id(1)].1 < origins.origins[&id(2)].1, "order kept");
        let _ = std::fs::remove_dir_all(dir);
    }
}
