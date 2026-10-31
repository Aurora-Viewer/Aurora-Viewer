//! The attachment point each object was last seen attached to, kept per
//! account (Aurora addition, human decision of 2026-10-31). An inventory
//! item often carries no saved point (flags 0) and the simulator only takes
//! the point from the object when it attaches, so the wear menus remember
//! where each object went to grey Wear under an RLVa lock, across sessions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
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

#[derive(Default)]
pub struct LastPoints {
    /// Point and the order it was seen in.
    points: HashMap<Uuid, (u8, u64)>,
    seq: u64,
    path: Option<PathBuf>,
    dirty_since: Option<Instant>,
}

impl LastPoints {
    pub fn load(path: PathBuf) -> Self {
        let mut this = Self::default();
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Saved>(&bytes) {
                Ok(saved) => {
                    for (item, point) in saved.points.into_iter().filter(|(_, point)| *point != 0) {
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

    fn insert(&mut self, item: Uuid, point: u8) {
        self.seq += 1;
        self.points.insert(item, (point, self.seq));
        if self.points.len() > LIMIT {
            let mut order: Vec<_> = self.points.iter().map(|(item, (_, seq))| (*seq, *item)).collect();
            order.sort_unstable();
            let extra = self.points.len() - LIMIT;
            for (_, item) in order.into_iter().take(extra) {
                self.points.remove(&item);
            }
        }
    }

    /// Writes the file in the background once the changes have settled.
    pub fn save_due(&mut self, now: Instant) {
        let (Some(path), Some(since)) = (&self.path, self.dirty_since) else {
            return;
        };
        if now.saturating_duration_since(since) < SAVE_DELAY {
            return;
        }
        self.dirty_since = None;
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
        let path = path.clone();
        std::thread::spawn(move || {
            let temporary = path.with_extension("json.tmp");
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&temporary, &bytes))
                .and_then(|()| std::fs::rename(&temporary, &path));
            if let Err(e) = written {
                log::warn!("rlva: attachment points not saved to {}: {e}", path.display());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(LastPoints::load(path.clone()).get(&Uuid::from_u128(1)), Some(2));
        for n in 0..LIMIT as u128 + 1 {
            points.note(Uuid::from_u128(100 + n), 5, now);
        }
        assert_eq!(points.points.len(), LIMIT);
        assert_eq!(points.get(&Uuid::from_u128(1)), None, "the oldest went first");
        std::fs::write(&path, b"not json").expect("write");
        assert_eq!(LastPoints::load(path).get(&Uuid::from_u128(1)), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
