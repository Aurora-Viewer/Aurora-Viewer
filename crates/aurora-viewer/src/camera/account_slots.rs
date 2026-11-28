//! Account-scoped quick views, following Aperture Viewer's
//! LLAgentCamera::storeCameraPosition and its
//! gSavedPerAccountSettings (indra/newview/llagentcamera.cpp, originally LGPL 2.1).
//! New accounts start empty. One worker orders disk writes.

use super::{CameraShot, presets};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use uuid::Uuid;

pub const FILE: &str = "camera_quick_views.json";

#[derive(Clone, Serialize, Deserialize)]
struct Document {
    version: u32,
    slots: Vec<CameraShot>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum State {
    #[default]
    Inactive,
    Loading,
    Ready,
    Failed,
}

enum Request {
    Load(u64, PathBuf),
    Save(u64, PathBuf, Document),
    Flush(Sender<()>),
}

enum Reply {
    Loaded(u64, Result<Option<Document>, String>),
    Saved(u64, Result<(), String>),
}

#[derive(Default)]
pub struct Store {
    pub account: Option<Uuid>,
    pub state: State,
    pub write_failed: bool,
    pub prompt_open: bool,
    pub slots: Vec<CameraShot>,
    demo_directory: Option<PathBuf>,
    path: PathBuf,
    generation: u64,
    worker: Option<(Sender<Request>, Receiver<Reply>)>,
}

fn quick_only(shots: &[CameraShot]) -> Vec<CameraShot> {
    let mut slots: Vec<_> = shots.iter().filter(|s| s.quick_slot.is_some()).cloned().collect();
    presets::sanitize(&mut slots);
    slots.retain(|s| s.quick_slot.is_some());
    slots
}

fn read(path: &Path) -> Result<Option<Document>, String> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(meta) if meta.len() > 1024 * 1024 => return Err("quick views file exceeds 1 MiB".into()),
        Ok(_) => {}
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if document.version != 1 {
        return Err("unsupported quick views version".into());
    }
    document.slots = quick_only(&document.slots);
    Ok(Some(document))
}

fn write(path: &Path, document: &Document) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(document).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::atomic_file::write(path, &bytes).map_err(|e| e.to_string())
}

impl Store {
    /// Own one unique directory for all synthetic accounts in this run.
    pub fn demo_path(&mut self, account: Uuid) -> std::io::Result<PathBuf> {
        if self.demo_directory.is_none() {
            let root = std::env::temp_dir().join(format!("aurora-camera-demo-{}", Uuid::new_v4()));
            std::fs::create_dir(&root)?;
            self.demo_directory = Some(root);
        }
        Ok(self
            .demo_directory
            .as_ref()
            .expect("demo directory created above")
            .join(account.to_string())
            .join(FILE))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn send(&mut self, request: Request) -> bool {
        if self.worker.is_none() {
            let (tx, requests) = mpsc::channel();
            let (replies, rx) = mpsc::channel();
            let spawned = std::thread::Builder::new().name("aurora-camera-slots".into()).spawn(move || {
                while let Ok(request) = requests.recv() {
                    match request {
                        Request::Load(generation, path) => {
                            let _ = replies.send(Reply::Loaded(generation, read(&path)));
                        }
                        Request::Save(generation, path, document) => {
                            let _ = replies.send(Reply::Saved(generation, write(&path, &document)));
                        }
                        Request::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            });
            if let Err(e) = spawned {
                log::warn!("cannot start camera slot storage: {e}");
                return false;
            }
            self.worker = Some((tx, rx));
        }
        let sent = self.worker.as_ref().is_some_and(|(tx, _)| tx.send(request).is_ok());
        if !sent {
            self.worker = None;
        }
        sent
    }

    pub fn select(&mut self, account: Option<Uuid>, path: PathBuf) {
        self.generation += 1;
        self.account = account.filter(|id| !id.is_nil());
        self.slots.clear();
        self.write_failed = false;
        self.prompt_open = true;
        self.path = path;
        self.state = State::Inactive;
        if self.account.is_some() {
            self.reload();
        }
    }

    pub fn reload(&mut self) {
        if self.account.is_none() {
            return;
        }
        self.state = State::Loading;
        if !self.send(Request::Load(self.generation, self.path.clone())) {
            self.state = State::Failed;
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let reply = match self.worker.as_ref().map(|(_, rx)| rx.try_recv()) {
                Some(Ok(reply)) => reply,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.worker = None;
                    if self.state == State::Loading {
                        self.state = State::Failed;
                        changed = true;
                    } else if self.state == State::Ready {
                        self.write_failed = true;
                    }
                    self.prompt_open = true;
                    log::warn!("camera slot storage worker disconnected");
                    break;
                }
                _ => break,
            };
            match reply {
                Reply::Loaded(generation, result) if generation == self.generation => {
                    changed = true;
                    match result {
                        Ok(Some(document)) => {
                            self.slots = document.slots;
                            self.state = State::Ready;
                        }
                        Ok(None) => {
                            self.state = State::Ready;
                            self.save();
                        }
                        Err(e) => {
                            log::warn!("cannot load account camera slots: {e}");
                            self.state = State::Failed;
                        }
                    }
                }
                Reply::Saved(generation, result) => {
                    if let Err(e) = &result {
                        log::warn!("cannot save account camera slots: {e}");
                    }
                    if generation == self.generation {
                        self.write_failed = result.is_err();
                        if self.write_failed {
                            self.prompt_open = true;
                        }
                    }
                }
                _ => {}
            }
        }
        changed
    }

    pub fn save(&mut self) {
        if self.state != State::Ready {
            return;
        }
        let document = Document {
            version: 1,
            slots: quick_only(&self.slots),
        };
        if !self.send(Request::Save(self.generation, self.path.clone(), document)) {
            self.write_failed = true;
            self.prompt_open = true;
        }
    }

    /// A backup worker can wait without blocking the frame thread.
    pub fn barrier(&mut self) -> Receiver<()> {
        let (done, wait) = mpsc::channel();
        self.send(Request::Flush(done));
        wait
    }

    pub fn flush(&mut self) -> bool {
        if self.worker.is_none() {
            return true;
        }
        self.barrier().recv_timeout(std::time::Duration::from_secs(5)).is_ok()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        if self.demo_directory.is_some()
            && self.flush()
            && let Some(root) = self.demo_directory.take()
            && let Err(error) = std::fs::remove_dir_all(root)
        {
            log::warn!("cannot remove camera demo directory: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        std::env::temp_dir().join(format!("aurora-account-slots-{}", Uuid::new_v4()))
    }

    fn shot(name: &str, slot: u8) -> CameraShot {
        let mut shot = CameraShot::at_avatar(name.into(), [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]);
        shot.quick_slot = Some(slot);
        shot.roll = Some(0.4);
        shot.quick_fov = Some(0.7);
        shot.quick_focus_object = Some(Uuid::from_u128(9));
        shot.anchor = Some(presets::Anchor::World {
            camera: [1000.0, 2000.0, 30.0],
            focus: [1001.0, 2002.0, 31.0],
        });
        shot
    }

    fn settle(store: &mut Store) {
        assert!(store.flush());
        store.poll();
        assert!(store.flush());
        store.poll();
    }

    #[test]
    fn account_slots_start_empty_and_persist_independently_after_restart() {
        let root = directory();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let path = |id: Uuid| root.join(id.to_string()).join(FILE);
        let mut store = Store::default();
        store.select(Some(a), path(a));
        settle(&mut store);
        assert!(store.state == State::Ready && store.slots.is_empty());
        store.slots.push(shot("A", 0));
        store.save();
        store.select(Some(b), path(b));
        settle(&mut store);
        assert!(store.state == State::Ready && store.slots.is_empty());
        store.slots.push(shot("B", 0));
        store.save();
        store.select(Some(a), path(a));
        settle(&mut store);
        assert_eq!(store.slots[0].name, "A");
        store.slots.clear();
        store.save();
        settle(&mut store);
        let mut restarted = Store::default();
        restarted.select(Some(a), path(a));
        settle(&mut restarted);
        assert!(restarted.state == State::Ready && restarted.slots.is_empty());
        restarted.select(Some(b), path(b));
        settle(&mut restarted);
        assert_eq!(restarted.slots, quick_only(&[shot("B", 0)]));
        restarted.select(None, PathBuf::new());
        assert!(restarted.slots.is_empty() && restarted.state == State::Inactive);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn demo_accounts_share_an_owned_directory_removed_after_writes_finish() {
        let mut store = Store::default();
        let a = Uuid::from_u128(1);
        let path = store.demo_path(a).unwrap();
        let root = store.demo_directory.clone().unwrap();
        assert!(store.demo_path(Uuid::from_u128(2)).unwrap().starts_with(&root));
        store.select(Some(a), path);
        settle(&mut store);
        store.slots.push(shot("A", 0));
        store.save();
        drop(store);
        assert!(!root.exists());
    }

    #[test]
    fn account_slots_ignore_late_loads_and_do_not_overwrite_unreadable_files() {
        let root = directory();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let (pa, pb) = (root.join("a.json"), root.join("b.json"));
        write(
            &pa,
            &Document {
                version: 1,
                slots: vec![shot("A", 0)],
            },
        )
        .unwrap();
        write(
            &pb,
            &Document {
                version: 1,
                slots: vec![shot("B", 0)],
            },
        )
        .unwrap();
        let mut store = Store::default();
        store.select(Some(a), pa.clone());
        store.select(Some(b), pb.clone());
        settle(&mut store);
        assert_eq!(store.slots[0].name, "B");
        for bytes in [b"broken".as_slice(), b"{}", b"{\"version\":2,\"slots\":[]}"] {
            std::fs::write(&pa, bytes).unwrap();
            store.select(Some(a), pa.clone());
            settle(&mut store);
            assert!(store.state == State::Failed && store.slots.is_empty());
            store.save();
            settle(&mut store);
            assert_eq!(std::fs::read(&pa).unwrap(), bytes);
        }
        write(
            &pa,
            &Document {
                version: 1,
                slots: vec![shot("Recovered", 0)],
            },
        )
        .unwrap();
        store.reload();
        settle(&mut store);
        assert_eq!(store.slots[0].name, "Recovered");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn account_slots_failed_writes_retain_edits_and_can_be_retried() {
        let root = directory();
        let path = root.join("account").join(FILE);
        let mut store = Store::default();
        store.select(Some(Uuid::from_u128(1)), path.clone());
        settle(&mut store);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        std::fs::write(path.parent().unwrap(), b"blocks directory").unwrap();
        store.slots.push(shot("Unsaved", 0));
        store.save();
        settle(&mut store);
        assert!(store.write_failed && store.prompt_open);
        assert_eq!(store.slots[0].name, "Unsaved");
        std::fs::remove_file(path.parent().unwrap()).unwrap();
        store.save();
        settle(&mut store);
        assert!(!store.write_failed);
        assert_eq!(read(&path).unwrap().unwrap().slots[0].name, "Unsaved");
        let _ = std::fs::remove_dir_all(root);
    }
}
