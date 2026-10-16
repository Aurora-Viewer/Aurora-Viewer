//! How much geometry a place needs, remembered from one visit to the next,
//! so that the geometry arena grows behind the loading screen instead of in
//! the middle of the scene.
//!
//! A full buffer of the arena is replaced by a larger one (arena.rs), and
//! the first use of fresh video memory holds the GPU about 21 µs per MB:
//! 8 and 14 ms frames when the vertex arena went to 384 and 768 MB while a
//! busy region loaded (audit of 2026-10-13). That cost cannot be split
//! across frames, but it can be paid while nothing is shown: when the
//! agent arrives in a region (login or teleport), the arena grows at once
//! to what the region needed during the last visit, and, on a first visit,
//! it doubles ahead of need while the screen still covers the view.
//!
//! The need noted for a region is the arena's peak use while the agent was
//! in it, during the latest visit only (what a visit leaves behind is freed
//! after a while: a stale peak must not stay for ever). The file holds one
//! line per region, `<handle> <vertices> <indices>`, the most recent first.

use std::path::{Path, PathBuf};

/// Regions remembered.
const KEPT: usize = 64;
const FILE: &str = "geometry-needs.txt";

/// Elements of the arena: vertices, indices.
pub type Elements = (u32, u32);

#[derive(Default)]
pub struct GeometryNeeds {
    path: Option<PathBuf>,
    /// Most recent visit first.
    regions: Vec<(u64, Elements)>,
    /// The region the agent is in and its peak during this visit.
    current: Option<(u64, Elements)>,
    changed: bool,
}

impl GeometryNeeds {
    /// The needs remembered in `dir` (none when the file is missing or
    /// unreadable: a first visit everywhere).
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let regions = std::fs::read_to_string(&path).map(|text| parse(&text)).unwrap_or_default();
        GeometryNeeds {
            path: Some(path),
            regions,
            current: None,
            changed: false,
        }
    }

    /// What the arena should hold now, if more than `capacity`: the need
    /// of `region` from its last visit, with an eighth to spare, and,
    /// beyond a use of three quarters, twice the capacity. Only behind a
    /// loading or teleport screen (`covered`): elsewhere the arena grows
    /// when full, as before. None when nothing has to grow.
    pub fn wanted(&self, region: u64, used: Elements, capacity: Elements, covered: bool) -> Option<Elements> {
        if !covered {
            return None;
        }
        let known = self.regions.iter().find(|(h, _)| *h == region).map(|(_, n)| *n).unwrap_or((0, 0));
        let one = |known: u32, used: u32, capacity: u32| {
            let remembered = known.saturating_add(known / 8);
            let ahead = if used > capacity / 4 * 3 { capacity.saturating_mul(2) } else { 0 };
            remembered.max(ahead)
        };
        let want = (one(known.0, used.0, capacity.0), one(known.1, used.1, capacity.1));
        (want.0 > capacity.0 || want.1 > capacity.1).then_some(want)
    }

    /// The arena's use this frame, the agent being in `region`: a new
    /// region ends the visit of the one before (its peak is kept).
    pub fn note(&mut self, region: u64, used: Elements) {
        match &mut self.current {
            Some((h, peak)) if *h == region => {
                *peak = (peak.0.max(used.0), peak.1.max(used.1));
            }
            _ => {
                self.end_visit();
                self.current = Some((region, used));
            }
        }
    }

    /// Keep the peak of the visit in progress, as the region's need.
    fn end_visit(&mut self) {
        let Some((region, peak)) = self.current.take() else {
            return;
        };
        if peak == (0, 0) {
            return;
        }
        self.regions.retain(|(h, _)| *h != region);
        self.regions.insert(0, (region, peak));
        self.regions.truncate(KEPT);
        self.changed = true;
    }

    /// Write the file if a visit ended since the last write (with the
    /// visit in progress counted as ended: at the end of the session, when
    /// it is written before this returns; otherwise by a thread of its own).
    pub fn save(&mut self, end_of_session: bool) {
        if end_of_session {
            self.end_visit();
        }
        if !std::mem::replace(&mut self.changed, false) {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        let text = format(&self.regions);
        let write = move || {
            if let Err(e) = crate::atomic_file::write(&path, text.as_bytes()) {
                log::warn!("geometry needs: {} not written: {e}", path.display());
            }
        };
        // (during the session, off the frame: a disk can hold a write)
        if end_of_session {
            write();
        } else if let Err(e) = std::thread::Builder::new().name("aurora-geometry-needs".into()).spawn(write) {
            log::warn!("geometry needs not written: {e}");
        }
    }

    /// A visit ended and the file is behind.
    pub fn unsaved(&self) -> bool {
        self.changed
    }
}

impl Drop for GeometryNeeds {
    /// The session ends (however the viewer closes): the visit in progress
    /// is kept.
    fn drop(&mut self) {
        self.save(true);
    }
}

fn parse(text: &str) -> Vec<(u64, Elements)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace().map(str::parse::<u64>);
            let (Some(Ok(h)), Some(Ok(v)), Some(Ok(i))) = (parts.next(), parts.next(), parts.next()) else {
                return None;
            };
            Some((h, (u32::try_from(v).ok()?, u32::try_from(i).ok()?)))
        })
        .take(KEPT)
        .collect()
}

fn format(regions: &[(u64, Elements)]) -> String {
    regions.iter().map(|(h, (v, i))| format!("{h} {v} {i}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: u32 = 1 << 20;

    #[test]
    fn nothing_grows_ahead_outside_a_covered_screen() {
        let mut n = GeometryNeeds::default();
        n.regions = vec![(7, (100 * M, 300 * M))];
        assert_eq!(n.wanted(7, (M, M), (2 * M, 8 * M), false), None);
        assert_eq!(n.wanted(7, (M, M), (2 * M, 8 * M), true), Some((112 * M + M / 2, 337 * M + M / 2)));
    }

    #[test]
    fn a_first_visit_doubles_beyond_three_quarters() {
        let n = GeometryNeeds::default();
        assert_eq!(n.wanted(1, (M, M), (2 * M, 8 * M), true), None);
        // vertices three quarters full: twice the vertices, indices as they are
        assert_eq!(n.wanted(1, (2 * M - 1, M), (2 * M, 8 * M), true), Some((4 * M, 0)));
        // a remembered need below the capacity changes nothing
        let mut n = GeometryNeeds::default();
        n.regions = vec![(1, (M, M))];
        assert_eq!(n.wanted(1, (0, 0), (2 * M, 8 * M), true), None);
    }

    #[test]
    fn the_peak_of_the_latest_visit_is_the_need() {
        let mut n = GeometryNeeds::default();
        n.note(1, (10, 20));
        n.note(1, (30, 5));
        assert!(!n.unsaved());
        // to another region: the first visit ends with its peak
        n.note(2, (4, 4));
        assert_eq!(n.regions, [(1, (30, 20))]);
        assert!(n.unsaved());
        // back: a smaller visit replaces the bigger one, and comes first
        n.note(1, (8, 8));
        n.note(3, (0, 0));
        assert_eq!(n.regions, [(1, (8, 8)), (2, (4, 4))]);
        // an empty visit is not kept
        n.note(4, (1, 1));
        assert_eq!(n.regions.len(), 2);
    }

    #[test]
    fn the_file_round_trips_and_skips_bad_lines() {
        let regions = vec![(1099511628032000u64, (12, 34)), (5, (0, 9))];
        assert_eq!(parse(&format(&regions)), regions);
        assert_eq!(parse("x 1 2\n3 4\n6 7 8\n9 99999999999 1\n"), [(6, (7, 8))]);
    }

    #[test]
    fn saved_needs_come_back() {
        let dir = std::env::temp_dir().join(format!("aurora-geometry-needs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut n = GeometryNeeds::load(&dir);
        n.note(42, (100, 200));
        n.save(true);
        let back = GeometryNeeds::load(&dir);
        assert_eq!(back.regions, [(42, (100, 200))]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
