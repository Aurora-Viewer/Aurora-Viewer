//! Sound log, following LLAudioSource::logSoundPlay / logSoundStop /
//! pruneSoundLog (indra/llaudio/llaudioengine.cpp). UI and local previews
//! never enter this log. Positions are global, so old rows survive crossings.
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: u64,
    pub asset: Uuid,
    pub source: Uuid,
    pub owner: Uuid,
    pub position: DVec3,
    pub started: Instant,
    pub stopped: Option<Instant>,
    pub triggered: bool,
    pub looped: bool,
}

#[derive(Default)]
pub struct History {
    entries: HashMap<u64, Entry>,
    pub generation: u64,
}

impl History {
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn sync(&mut self, playing: impl Iterator<Item = Entry>, now: Instant) {
        let mut active = HashSet::new();
        for entry in playing {
            active.insert(entry.id);
            match self.entries.entry(entry.id) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(entry);
                    self.generation = self.generation.wrapping_add(1);
                }
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    if slot.get().owner.is_nil() && !entry.owner.is_nil() {
                        slot.get_mut().owner = entry.owner;
                        self.generation = self.generation.wrapping_add(1);
                    }
                }
            }
        }
        for entry in self.entries.values_mut() {
            if entry.stopped.is_none() && !active.contains(&entry.id) {
                entry.stopped = Some(now);
                self.generation = self.generation.wrapping_add(1);
            }
        }
        // Keep active rows, then the newest stopped rows, up to 256 total.
        // Unlike the legacy periodic sweep this never briefly grows to 2048.
        if self.entries.len() > 256 {
            let mut stopped: Vec<_> = self.entries.values().filter_map(|e| e.stopped.map(|t| (t, e.id))).collect();
            stopped.sort_unstable();
            for (_, id) in stopped.into_iter().take(self.entries.len() - 256) {
                self.entries.remove(&id);
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }

    pub fn stop(&mut self, id: u64, now: Instant) {
        if let Some(entry) = self.entries.get_mut(&id).filter(|e| e.stopped.is_none()) {
            entry.stopped = Some(now);
            self.generation = self.generation.wrapping_add(1);
        }
    }
}
