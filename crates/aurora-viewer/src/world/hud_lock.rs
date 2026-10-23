//! Viewer-local protection against accidental HUD move gestures (#862).
//! Inventory UUIDs survive reattachment; temporary HUDs keep an instance lock.
use crossbeam_channel::{self as mpsc, Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::thread::JoinHandle;
use uuid::Uuid;

pub const FILE: &str = "hud_locks.json";

#[derive(Default)]
struct Local {
    item: Option<Uuid>,
    edited: Option<bool>,
}

#[derive(Default)]
pub struct HudLocks {
    saved: HashSet<Uuid>,
    local: HashMap<Uuid, Local>,
    loaded: Option<Receiver<HashSet<Uuid>>>,
    writer: Option<Sender<(Uuid, bool)>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for HudLocks {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl HudLocks {
    pub fn load(path: PathBuf) -> Self {
        let (loaded_tx, loaded) = mpsc::unbounded();
        let (writer, writes) = mpsc::unbounded::<(Uuid, bool)>();
        let worker = std::thread::Builder::new().name("aurora-hud-locks".into()).spawn(move || {
            let mut saved: HashSet<Uuid> = match std::fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(saved) => saved,
                    Err(error) => {
                        log::warn!("Cannot read HUD locks: {error}");
                        crate::tolerant::backup(&path);
                        HashSet::new()
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
                Err(error) => {
                    log::warn!("Cannot read HUD locks: {error}");
                    HashSet::new()
                }
            };
            saved.remove(&Uuid::nil());
            let _ = loaded_tx.send(saved.clone());
            while let Ok((item, locked)) = writes.recv() {
                apply(&mut saved, item, locked);
                while let Ok((item, locked)) = writes.try_recv() {
                    apply(&mut saved, item, locked);
                }
                let save = || -> Result<(), Box<dyn std::error::Error>> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut items: Vec<_> = saved.iter().copied().collect();
                    items.sort_unstable();
                    let tmp = path.with_extension("json.tmp");
                    std::fs::write(&tmp, serde_json::to_vec(&items)?)?;
                    std::fs::rename(tmp, &path)?;
                    Ok(())
                };
                if let Err(error) = save() {
                    log::warn!("Cannot save HUD locks: {error}");
                }
            }
        });
        match worker {
            Ok(worker) => Self {
                saved: Default::default(),
                local: Default::default(),
                loaded: Some(loaded),
                writer: Some(writer),
                worker: Some(worker),
            },
            Err(error) => {
                log::warn!("Cannot start HUD lock storage: {error}");
                Self::default()
            }
        }
    }

    pub fn refresh(&mut self, roots: &[(Uuid, Option<Uuid>)]) {
        if let Some(saved) = self.loaded.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.saved = saved;
            // A toggle before the asynchronous read completes wins over disk.
            for local in self.local.values() {
                if let (Some(item), Some(locked)) = (local.item, local.edited) {
                    apply(&mut self.saved, item, locked);
                }
            }
            self.loaded = None;
        }
        self.local.retain(|id, _| roots.iter().any(|(root, _)| root == id));
        for &(id, item) in roots {
            let local = self.local.entry(id).or_default();
            // Partial updates can omit AttachItemID. Keep the known stable ID.
            let item = item.or(local.item);
            if local.item != item {
                local.item = item;
                if let (Some(item), Some(locked)) = (item, local.edited) {
                    self.remember(item, locked);
                }
            }
        }
    }

    pub fn locked(&self, id: Uuid) -> bool {
        self.local.get(&id).is_some_and(|local| {
            local
                .edited
                .unwrap_or_else(|| local.item.is_some_and(|item| self.saved.contains(&item)))
        })
    }

    pub fn set(&mut self, id: Uuid, locked: bool) {
        let local = self.local.entry(id).or_default();
        local.edited = Some(locked);
        if let Some(item) = local.item {
            self.remember(item, locked);
        }
    }

    fn remember(&mut self, item: Uuid, locked: bool) {
        apply(&mut self.saved, item, locked);
        if let Some(writer) = &self.writer {
            let _ = writer.send((item, locked));
        }
    }
}

fn apply(saved: &mut HashSet<Uuid>, item: Uuid, locked: bool) {
    if locked {
        saved.insert(item);
    } else {
        saved.remove(&item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_lock_survives_reattachment_late_metadata_and_keeps_copies_independent() {
        let (id, next, item, copy) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let mut locks = HudLocks::default();
        locks.refresh(&[(id, None)]);
        locks.set(id, true);
        locks.refresh(&[(id, Some(item)), (copy, Some(copy))]);
        assert!(locks.locked(id));
        assert!(!locks.locked(copy));
        locks.refresh(&[(next, Some(item)), (copy, Some(copy))]);
        assert!(locks.locked(next));
        assert!(!locks.locked(id));
        locks.set(next, false);
        locks.refresh(&[(id, Some(item))]);
        assert!(!locks.locked(id));
    }

    #[test]
    fn hud_unlock_before_background_read_preserves_other_saved_items() {
        let (id, item, other) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let (tx, rx) = mpsc::unbounded();
        let mut locks = HudLocks {
            loaded: Some(rx),
            saved: Default::default(),
            local: Default::default(),
            writer: None,
            worker: None,
        };
        locks.refresh(&[(id, Some(item)), (other, Some(other))]);
        locks.set(id, false);
        tx.send(HashSet::from([item, other])).unwrap();
        locks.refresh(&[(id, Some(item)), (other, Some(other))]);
        assert!(!locks.locked(id));
        assert!(locks.locked(other));
    }

    #[test]
    fn hud_lock_disk_roundtrip_and_early_unlock_win_over_background_load() {
        let path = std::env::temp_dir().join(format!("aurora-hud-locks-{}.json", Uuid::new_v4()));
        let (id, item) = (Uuid::from_u128(1), Uuid::from_u128(2));
        {
            let mut locks = HudLocks::load(path.clone());
            locks.refresh(&[(id, Some(item))]);
            locks.set(id, true);
        }
        assert_eq!(
            serde_json::from_slice::<Vec<Uuid>>(&std::fs::read(&path).unwrap()).unwrap(),
            vec![item]
        );
        {
            let mut locks = HudLocks::load(path.clone());
            locks.saved = locks.loaded.take().unwrap().recv().unwrap();
            locks.refresh(&[(id, Some(item))]);
            assert!(locks.locked(id));
            locks.set(id, false);
            assert!(!locks.locked(id));
        }
        assert!(
            serde_json::from_slice::<Vec<Uuid>>(&std::fs::read(&path).unwrap())
                .unwrap()
                .is_empty()
        );
        std::fs::remove_file(path).unwrap();
    }
}
