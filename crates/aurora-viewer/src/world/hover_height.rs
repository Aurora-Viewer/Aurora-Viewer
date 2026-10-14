//! LLFloaterHoverHeight / LLVOAvatarSelf::setHoverOffset (Firestorm,
//! originally LGPL 2.1): preview locally, persist and send on final commit.
use aurora_net::{NetCommand, RegionHandle};
use crossbeam_channel::{self as mpsc, Receiver, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

pub const MIN: f32 = -3.0;
pub const MAX: f32 = 3.0;
pub const FILE: &str = "hover_height.json";
static REQUEST: AtomicU64 = AtomicU64::new(1);

pub fn bounded(value: f32) -> Option<f32> {
    value.is_finite().then(|| value.clamp(MIN, MAX))
}

/// Firestorm's +/- buttons: 1 cm, Alt 10 cm, Ctrl/Shift 1 mm.
pub fn step(alt: bool, fine: bool) -> f32 {
    if alt {
        0.1
    } else if fine {
        0.001
    } else {
        0.01
    }
}

struct Storage {
    loaded: Receiver<f32>,
    writer: Option<Sender<f32>>,
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
pub struct HoverHeight {
    value: f32,
    committed: f32,
    region: Option<RegionHandle>,
    sent: Option<f32>,
    pending: Option<(u64, RegionHandle, f32)>,
    pub failed: bool,
    storage: Option<Storage>,
    loading: bool,
    edited: bool,
}

impl HoverHeight {
    /// The account path is chosen only after login. Reading and ordered,
    /// atomic writes stay off the frame thread; demo uses Default in memory.
    pub fn load(path: PathBuf) -> Self {
        let (loaded_tx, loaded) = mpsc::unbounded();
        let (writer, writes) = mpsc::unbounded::<f32>();
        let worker = std::thread::Builder::new().name("aurora-hover-height".into()).spawn(move || {
            let value = match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice::<f32>(&bytes).ok().and_then(bounded).unwrap_or(0.0),
                Err(error) => {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("Cannot read hover height: {error}");
                    }
                    0.0
                }
            };
            let _ = loaded_tx.send(value);
            while let Ok(mut value) = writes.recv() {
                while let Ok(newer) = writes.try_recv() {
                    value = newer;
                }
                let save = || -> std::io::Result<()> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    crate::atomic_file::write(&path, value.to_string().as_bytes())
                };
                if let Err(error) = save() {
                    log::warn!("Cannot save hover height: {error}");
                }
            }
        });
        match worker {
            Ok(worker) => Self {
                storage: Some(Storage {
                    loaded,
                    writer: Some(writer),
                    worker: Some(worker),
                }),
                loading: true,
                ..Self::default()
            },
            Err(error) => {
                log::warn!("Cannot start hover height storage: {error}");
                Self::default()
            }
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn ready(&self) -> bool {
        !self.loading
    }

    pub fn preview(&mut self, value: f32) {
        if let Some(value) = bounded(value) {
            self.value = value;
        }
    }

    pub fn commit(&mut self) {
        if self.value != self.committed {
            self.committed = self.value;
            self.edited = true;
            self.failed = false;
            if let Some(writer) = self.storage.as_ref().and_then(|storage| storage.writer.as_ref()) {
                let _ = writer.send(self.committed);
            }
        }
    }

    pub fn retry(&mut self) {
        self.failed = false;
    }

    /// One HTTP write at a time, coalescing rapid commits. A teleport must
    /// send the saved value again, even when it equals the previous region's.
    pub fn next_update(&mut self, region: Option<RegionHandle>) -> Option<NetCommand> {
        if self.loading
            && let Some(storage) = &self.storage
        {
            match storage.loaded.try_recv() {
                Ok(value) => {
                    if !self.edited {
                        self.value = value;
                        self.committed = value;
                    }
                    self.loading = false;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.loading = false,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.region != region {
            self.region = region;
            self.sent = None;
            self.failed = false;
            self.value = self.committed;
        }
        let region = region?;
        if self.loading || self.pending.is_some() || self.failed || self.sent == Some(self.committed) {
            return None;
        }
        let request = REQUEST.fetch_add(1, Ordering::Relaxed);
        self.pending = Some((request, region, self.committed));
        Some(NetCommand::SetHoverHeight {
            region,
            request,
            value: self.committed,
        })
    }

    pub fn reply(&mut self, request: u64, region: RegionHandle, success: bool) {
        let Some((pending, target, value)) = self.pending else { return };
        if request != pending || region != target {
            return;
        }
        self.pending = None;
        if self.region == Some(region) {
            if success {
                self.sent = Some(value);
            }
            // A failed older value must not block a newer user edit.
            self.failed = !success && self.committed == value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(state: &mut HoverHeight, region: u64) -> (u64, f32) {
        let Some(NetCommand::SetHoverHeight { request, value, .. }) = state.next_update(Some(region)) else {
            panic!("queued height")
        };
        (request, value)
    }

    #[test]
    fn previews_wait_for_commit_and_writes_are_ordered_and_coalesced() {
        let mut height = HoverHeight::default();
        let (first, _) = request(&mut height, 1);
        height.reply(first, 1, true);
        height.preview(0.25);
        assert_eq!(height.value(), 0.25);
        assert!(height.next_update(Some(1)).is_none());
        height.commit();
        let (second, value) = request(&mut height, 1);
        assert_eq!(value, 0.25);
        for value in [0.5, 0.75] {
            height.preview(value);
            height.commit();
        }
        assert!(height.next_update(Some(1)).is_none());
        height.reply(first, 1, true);
        assert!(height.next_update(Some(1)).is_none());
        height.reply(second, 1, true);
        assert_eq!(request(&mut height, 1).1, 0.75);
    }

    #[test]
    fn region_changes_failures_and_retry_preserve_the_committed_preference() {
        let mut height = HoverHeight::default();
        height.preview(0.2);
        height.commit();
        let (first, _) = request(&mut height, 1);
        assert!(height.next_update(None).is_none());
        height.reply(first, 1, false);
        let (second, value) = request(&mut height, 2);
        assert_eq!(value, 0.2);
        height.reply(second, 2, false);
        assert!(height.failed);
        assert!(height.next_update(Some(2)).is_none());
        height.retry();
        let (third, _) = request(&mut height, 2);
        height.reply(third, 2, true);
        assert!(height.next_update(Some(2)).is_none());
        assert_eq!(request(&mut height, 1).1, 0.2);
    }

    #[test]
    fn limits_reject_non_finite_values_and_buttons_keep_firestorm_steps() {
        assert_eq!(bounded(f32::NAN), None);
        assert_eq!(bounded(f32::INFINITY), None);
        assert_eq!(bounded(4.0), Some(MAX));
        assert_eq!(bounded(-4.0), Some(MIN));
        assert_eq!([step(false, false), step(false, true), step(true, true)], [0.01, 0.001, 0.1]);
    }

    #[test]
    fn persisted_heights_are_per_account_and_the_latest_commit_survives_restart() {
        let root = std::env::temp_dir().join(format!("aurora-hover-{}", uuid::Uuid::new_v4()));
        let path = root.join("first").join(FILE);
        let mut height = HoverHeight::load(path.clone());
        height.preview(0.2);
        height.commit();
        height.preview(-0.135);
        height.commit();
        drop(height); // drains the ordered writer
        assert_eq!(serde_json::from_slice::<f32>(&std::fs::read(&path).unwrap()).unwrap(), -0.135);
        let mut reloaded = HoverHeight::load(path);
        for _ in 0..1000 {
            if let Some(NetCommand::SetHoverHeight { value, .. }) = reloaded.next_update(Some(1)) {
                assert_eq!(value, -0.135);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!reloaded.loading);
        assert!(!root.join("second").join(FILE).exists());
        drop(reloaded);
        std::fs::remove_dir_all(root).unwrap();
    }
}
