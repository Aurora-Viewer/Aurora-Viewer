//! Bytes the viewer reads from and writes to its own files, by origin: the
//! texture, mesh and object caches, the inventory cache, the logs and
//! histories. Counted where the files are read and written (a relaxed
//! atomic addition each time), always: the resource monitor's Disk section
//! turns the totals into rates. What the system or the drivers read on the
//! viewer's behalf (its executable, the graphics driver's shader cache) is
//! not counted.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Where the bytes go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Textures,
    Meshes,
    /// The regions' object caches.
    Objects,
    Inventory,
    /// Chat logs, transcripts, notification history and the like.
    Logs,
    /// Settings, layouts, pictures and the rest.
    Other,
}

impl Origin {
    pub const ALL: [Origin; 6] = [
        Origin::Textures,
        Origin::Meshes,
        Origin::Objects,
        Origin::Inventory,
        Origin::Logs,
        Origin::Other,
    ];
}

static READ: [AtomicU64; Origin::ALL.len()] = [const { AtomicU64::new(0) }; Origin::ALL.len()];
static WRITTEN: [AtomicU64; Origin::ALL.len()] = [const { AtomicU64::new(0) }; Origin::ALL.len()];

/// `bytes` read from a file of `origin`.
#[inline]
pub fn read(origin: Origin, bytes: usize) {
    READ[origin as usize].fetch_add(bytes as u64, Relaxed);
}

/// `bytes` written to a file of `origin`.
#[inline]
pub fn written(origin: Origin, bytes: usize) {
    WRITTEN[origin as usize].fetch_add(bytes as u64, Relaxed);
}

/// Bytes read and written since the start, by origin ([`Origin::ALL`]
/// order).
pub fn totals() -> [(u64, u64); Origin::ALL.len()] {
    std::array::from_fn(|i| (READ[i].load(Relaxed), WRITTEN[i].load(Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_add_up_by_origin() {
        let before = totals();
        read(Origin::Meshes, 1000);
        read(Origin::Meshes, 24);
        written(Origin::Logs, 7);
        let after = totals();
        let m = Origin::Meshes as usize;
        let l = Origin::Logs as usize;
        // (other tests may count at the same time: at least these bytes)
        assert!(after[m].0 - before[m].0 >= 1024);
        assert!(after[l].1 - before[l].1 >= 7);
    }
}
