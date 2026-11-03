//! The attachment point each object was last seen attached to, kept per
//! account (Aurora addition, human decision of 2026-10-31). An inventory
//! item often carries no saved point (flags 0) and the simulator only takes
//! the point from the object when it attaches, so the wear menus remember
//! where each object went to grey Wear under an RLVa lock, across sessions.

use crossbeam_channel::{self as mpsc, Sender};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FILE: &str = "rlva_attach_points.json";
/// Objects remembered; the ones seen longest ago go first.
const LIMIT: usize = 4096;
/// One write a few seconds after the last change, not one per attachment.
const SAVE_DELAY: Duration = Duration::from_secs(5);

/// The file: oldest first.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    points: Vec<(Uuid, u8)>,
}

/// One background writer, started at the first save: it writes the newest
/// snapshot it has, in order, and finishes the last one when dropped.
struct Writer {
    snapshots: Option<Sender<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    fn start(path: PathBuf) -> Option<Self> {
        let (snapshots, received) = mpsc::unbounded::<Vec<u8>>();
        let worker = std::thread::Builder::new().name("aurora-rlva-points".into()).spawn(move || {
            while let Ok(mut bytes) = received.recv() {
                while let Ok(newer) = received.try_recv() {
                    bytes = newer;
                }
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| crate::atomic_file::write(&path, &bytes));
                if let Err(e) = written {
                    log::warn!("rlva: attachment points not saved to {}: {e}", path.display());
                }
            }
        });
        match worker {
            Ok(worker) => Some(Self {
                snapshots: Some(snapshots),
                worker: Some(worker),
            }),
            Err(e) => {
                log::warn!("rlva: attachment points writer not started: {e}");
                None
            }
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.snapshots.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Dropped on logout, account switch and quit: the pending changes are
/// written then (`Drop`).
#[derive(Default)]
pub struct LastPoints {
    /// Point and the order it was seen in.
    points: HashMap<Uuid, (u8, u64)>,
    seq: u64,
    path: Option<PathBuf>,
    dirty_since: Option<Instant>,
    writer: Option<Writer>,
}

impl Drop for LastPoints {
    fn drop(&mut self) {
        self.flush();
    }
}

impl LastPoints {
    pub fn load(path: PathBuf) -> Self {
        let mut this = Self::default();
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Saved>(&bytes) {
                Ok(saved) => {
                    // Only the newest LIMIT entries of a file that holds more
                    // (edited by hand): the others would be evicted anyway.
                    let skip = saved.points.len().saturating_sub(LIMIT);
                    for (item, point) in saved.points.into_iter().skip(skip).filter(|(_, point)| *point != 0) {
                        this.insert(item, point);
                    }
                }
                Err(e) => log::warn!("rlva: attachment points file {} unreadable: {e}", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("rlva: attachment points file {} unreadable: {e}", path.display()),
        }
        this.path = Some(path);
        this
    }

    pub fn get(&self, item: &Uuid) -> Option<u8> {
        self.points.get(item).map(|(point, _)| *point)
    }

    pub fn snapshot(&self) -> HashMap<Uuid, u8> {
        self.points.iter().map(|(item, (point, _))| (*item, *point)).collect()
    }

    /// `item` is attached to `point` now.
    pub fn note(&mut self, item: Uuid, point: u8, now: Instant) {
        if point == 0 || self.get(&item) == Some(point) {
            return;
        }
        self.insert(item, point);
        self.dirty_since.get_or_insert(now);
    }

    /// One entry at a time: past the limit, only the oldest one goes.
    fn insert(&mut self, item: Uuid, point: u8) {
        self.seq += 1;
        self.points.insert(item, (point, self.seq));
        if self.points.len() > LIMIT
            && let Some(oldest) = self.points.iter().min_by_key(|(_, (_, seq))| *seq).map(|(item, _)| *item)
        {
            self.points.remove(&oldest);
        }
    }

    /// Writes the file in the background once the changes have settled.
    pub fn save_due(&mut self, now: Instant) {
        if self
            .dirty_since
            .is_some_and(|since| now.saturating_duration_since(since) >= SAVE_DELAY)
        {
            self.flush();
        }
    }

    /// Hands the pending changes to the writer now.
    fn flush(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        if self.dirty_since.take().is_none() {
            return;
        }
        let mut order: Vec<_> = self.points.iter().map(|(item, (point, seq))| (*seq, *item, *point)).collect();
        order.sort_unstable();
        let saved = Saved {
            points: order.into_iter().map(|(_, item, point)| (item, point)).collect(),
        };
        let bytes = match serde_json::to_vec(&saved) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!("rlva: attachment points not saved: {e}");
                return;
            }
        };
        if self.writer.is_none() {
            self.writer = Writer::start(path.clone());
        }
        if let Some(sender) = self.writer.as_ref().and_then(|w| w.snapshots.as_ref())
            && sender.send(bytes).is_err()
        {
            log::warn!("rlva: attachment points writer stopped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(path: &std::path::Path) {
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn points_survive_a_reload_and_the_oldest_go_first() {
        let dir = std::env::temp_dir().join(format!("aurora-rlva-points-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        let now = Instant::now();
        let mut points = LastPoints::load(path.clone());
        assert_eq!(points.get(&Uuid::from_u128(1)), None);
        points.note(Uuid::from_u128(1), 2, now);
        points.note(Uuid::from_u128(2), 0, now);
        assert_eq!(points.get(&Uuid::from_u128(1)), Some(2));
        assert_eq!(points.get(&Uuid::from_u128(2)), None, "point 0 is no point");
        points.save_due(now);
        assert!(!path.exists(), "the write waits for the changes to settle");
        points.save_due(now + SAVE_DELAY);
        wait_for(&path);
        assert_eq!(LastPoints::load(path.clone()).get(&Uuid::from_u128(1)), Some(2));
        for n in 0..LIMIT as u128 + 1 {
            points.note(Uuid::from_u128(100 + n), 5, now);
        }
        assert_eq!(points.points.len(), LIMIT);
        assert_eq!(points.get(&Uuid::from_u128(1)), None, "the oldest went first");
        drop(points);
        std::fs::write(&path, b"not json").expect("write");
        assert_eq!(LastPoints::load(path).get(&Uuid::from_u128(1)), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pending_points_are_written_when_dropped() {
        let dir = std::env::temp_dir().join(format!("aurora-rlva-points-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        let mut points = LastPoints::load(path.clone());
        points.note(Uuid::from_u128(1), 2, Instant::now());
        drop(points);
        assert_eq!(
            LastPoints::load(path).get(&Uuid::from_u128(1)),
            Some(2),
            "logout or quit within the save delay keeps the point"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_oversized_file_keeps_only_its_newest_entries() {
        let dir = std::env::temp_dir().join(format!("aurora-rlva-points-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        std::fs::create_dir_all(&dir).expect("directory");
        let saved = Saved {
            points: (0..LIMIT as u128 + 10).map(|n| (Uuid::from_u128(n + 1), 3)).collect(),
        };
        std::fs::write(&path, serde_json::to_vec(&saved).expect("json")).expect("write");
        let points = LastPoints::load(path);
        assert_eq!(points.points.len(), LIMIT);
        assert_eq!(points.get(&Uuid::from_u128(10)), None, "the oldest are skipped");
        assert_eq!(points.get(&Uuid::from_u128(LIMIT as u128 + 10)), Some(3));
        drop(points);
        let _ = std::fs::remove_dir_all(dir);
    }
}
