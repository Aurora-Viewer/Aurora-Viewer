//! Other Aurora viewers running on this PC, and what they hold of the card.
//!
//! The driver's memory budget (aurora_render::vram) already falls when a
//! second viewer takes video memory, but only once it has taken it: two
//! viewers loading a busy region together each read a large budget and grow
//! at the same time, until the card is full, Windows moves memory to system
//! RAM and the driver resets the GPU (a tester's crash with two viewers
//! open: "GPU device lost"). So the viewers tell each other. Each one
//! rewrites a small file of its own in the `instances` folder every
//! [`PERIOD`] (its video memory in use and the time) and reads the others'.
//! A file not rewritten for [`STALE_AFTER`] is a viewer that closed or
//! crashed, and is ignored. Plain files, no lock and no shared memory: a
//! file read half-written or unreadable is skipped, and the next pass has it.
//!
//! Two uses, both only while another viewer is there:
//! - the memory the texture streamer may count on is its fair share of what
//!   the viewers have together ([`share_cap`], `scene/textures.rs`), each
//!   viewer leaving the others the room up to their share;
//! - a window that is not in front draws at [`BEHIND_FPS`] at most, whatever
//!   the background cap says ([`throttle`]): two viewers drawing a busy
//!   scene at full rate kept the GPU so busy that frames took 73 ms before
//!   the driver gave up on it. The Legacy Viewer has neither: it does not
//!   know that another viewer is running.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Time between two rewrites of this viewer's file (and two reads of the
/// others').
const PERIOD: Duration = Duration::from_secs(2);
/// A file this old (seconds) belongs to a viewer that is gone.
const STALE_AFTER: u64 = 8;
/// A file this old (seconds) is deleted by whoever finds it.
const REMOVE_AFTER: u64 = 120;
/// Frames per second of a window behind, with another viewer running.
pub const BEHIND_FPS: f32 = 10.0;

const EXTENSION: &str = "instance";

/// The other viewers alive now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peers {
    /// Bytes of video memory each one holds (what the driver counts).
    pub usage: Vec<u64>,
}

impl Peers {
    pub fn any(&self) -> bool {
        !self.usage.is_empty()
    }
}

/// This viewer's view of the others, refreshed every [`PERIOD`].
pub struct Instances {
    dir: PathBuf,
    file: PathBuf,
    last: Option<Instant>,
    peers: Peers,
}

impl Instances {
    pub fn new() -> Self {
        let dir = crate::logging::logs_dir().with_file_name("instances");
        let file = dir.join(format!("{}.{EXTENSION}", std::process::id()));
        Instances {
            dir,
            file,
            last: None,
            peers: Peers::default(),
        }
    }

    /// The others, as of the last pass.
    pub fn peers(&self) -> &Peers {
        &self.peers
    }

    /// Each frame: when [`PERIOD`] has passed, publish `usage` (bytes of
    /// video memory this viewer holds, 0 when unknown) and read the others.
    pub fn tick(&mut self, now: Instant, usage: u64) {
        if self.last.is_some_and(|t| now.duration_since(t) < PERIOD) {
            return;
        }
        self.last = Some(now);
        let secs = unix_secs();
        if std::fs::create_dir_all(&self.dir).is_ok() {
            // (a failed write leaves the others without us: logged once is
            // enough, the next pass tries again)
            if let Err(e) = crate::atomic_file::publish(&self.file, format!("{secs} {usage}").as_bytes()) {
                log::debug!("instances: could not publish {}: {e}", self.file.display());
            }
        }
        let mut entries = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != EXTENSION) || path == self.file {
                    continue;
                }
                // (a file half written, or not ours, is skipped)
                if let Some(stamp) = std::fs::read_to_string(&path).ok().as_deref().and_then(parse) {
                    if secs.saturating_sub(stamp.0) > REMOVE_AFTER {
                        let _ = std::fs::remove_file(&path);
                    }
                    entries.push(stamp);
                }
            }
        }
        let peers = alive(&entries, secs);
        if peers.usage.len() != self.peers.usage.len() {
            log::info!(
                "instances: {} other viewer(s) running, holding {} MB of video memory",
                peers.usage.len(),
                peers.usage.iter().sum::<u64>() >> 20
            );
        }
        self.peers = peers;
    }

    /// The viewer closes: its file goes, the others do not wait for it to
    /// turn stale.
    pub fn leave(&self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `"<unix seconds> <bytes>"`.
fn parse(text: &str) -> Option<(u64, u64)> {
    let mut parts = text.split_whitespace();
    let stamp = parts.next()?.parse().ok()?;
    let usage = parts.next()?.parse().ok()?;
    Some((stamp, usage))
}

/// The entries (time, bytes) of viewers still alive at `now`.
fn alive(entries: &[(u64, u64)], now: u64) -> Peers {
    Peers {
        usage: entries
            .iter()
            .filter(|(stamp, _)| now.saturating_sub(*stamp) <= STALE_AFTER)
            .map(|&(_, usage)| usage)
            .collect(),
    }
}

/// The part of the driver's `budget` this viewer should count on with
/// `peers` running: the viewers share what they have together (the budget
/// plus what the others hold), and each is left the room up to its share
/// before this one takes the rest. A viewer holding more than its share
/// leaves nothing: the driver's own budget, already lower, rules. Never under
/// an equal slice of the budget: a neighbour far past its share must not
/// take the room of one that is not.
pub fn share_cap(budget: u64, peers: &Peers) -> u64 {
    if !peers.any() {
        return budget;
    }
    let held: u64 = peers.usage.iter().sum();
    let share = (budget as u128 + held as u128) / (peers.usage.len() as u128 + 1);
    let room: u128 = peers.usage.iter().map(|&u| share.saturating_sub(u as u128)).sum();
    let slice = budget as u128 / (peers.usage.len() as u128 + 1);
    (budget as u128).saturating_sub(room).max(slice) as u64
}

/// The frame cap (0 = none) with the card shared (`shared`: another viewer
/// runs, or other programs keep the card busy, gpu_load.rs): a window that is
/// not in front draws at [`BEHIND_FPS`] at most. `timed` runs (captures,
/// profiles) are measured in windows that never had the focus: left alone.
pub fn throttle(cap: f32, focused: bool, shared: bool, timed: bool) -> f32 {
    if focused || timed || !shared {
        return cap;
    }
    if cap > 0.0 { cap.min(BEHIND_FPS) } else { BEHIND_FPS }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    fn peers(usage: &[u64]) -> Peers {
        Peers {
            usage: usage.iter().map(|u| u * MB).collect(),
        }
    }

    #[test]
    fn alone_the_driver_budget_stands() {
        assert_eq!(share_cap(4423 * MB, &peers(&[])), 4423 * MB);
    }

    #[test]
    fn a_neighbour_holding_more_than_its_share_leaves_the_budget_alone() {
        // the tester's card: 4.4 GB left to us, the other viewer holds 5 GB
        assert_eq!(share_cap(4423 * MB, &peers(&[5000])), 4423 * MB);
    }

    #[test]
    fn a_neighbour_still_growing_is_left_room_up_to_its_share() {
        // two viewers loading together: 8 GB offered, the other holds 1 GB;
        // together 9 GB, 4.5 each
        assert_eq!(share_cap(8000 * MB, &peers(&[1000])), 4500 * MB);
        // it holds 4 GB: 6 GB each, we keep 6 GB
        assert_eq!(share_cap(8000 * MB, &peers(&[4000])), 6000 * MB);
    }

    #[test]
    fn three_viewers_each_leave_the_others_their_share() {
        // 9 GB offered, two others holding 1 GB each: 11 GB, a third each
        let cap = share_cap(9000 * MB, &peers(&[1000, 1000]));
        // (a byte or two over from the divisions)
        assert!(cap >= 11_000 * MB / 3 && cap - 11_000 * MB / 3 < 3, "{cap}");
    }

    #[test]
    fn the_cap_stays_between_an_equal_slice_of_the_budget_and_the_budget() {
        for held in [0, 1, 500, 3000, 9000] {
            for budget in [1000u64, 4000, 12_000] {
                let cap = share_cap(budget * MB, &peers(&[held, held / 2]));
                assert!(cap >= budget * MB / 3 && cap <= budget * MB, "{budget} {held}");
            }
        }
    }

    #[test]
    fn files_are_read_back_and_stale_ones_are_ignored() {
        assert_eq!(parse("1700000000 3130000000"), Some((1_700_000_000, 3_130_000_000)));
        assert_eq!(parse("17000"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("a b"), None);
        let found = alive(&[(100, 5), (92, 6), (91, 7), (0, 8)], 100);
        assert_eq!(found.usage, vec![5, 6]);
    }

    #[test]
    fn a_window_behind_draws_slowly_only_beside_another_viewer() {
        let (alone, with) = (false, true);
        assert_eq!(throttle(60.0, false, alone, false), 60.0);
        assert_eq!(throttle(60.0, true, with, false), 60.0);
        assert_eq!(throttle(60.0, false, with, false), BEHIND_FPS);
        // no cap at all, or a lower one
        assert_eq!(throttle(0.0, false, with, false), BEHIND_FPS);
        assert_eq!(throttle(5.0, false, with, false), 5.0);
        // measured runs are left alone
        assert_eq!(throttle(60.0, false, with, true), 60.0);
    }

    #[test]
    fn two_viewers_see_each_other_through_their_folder() {
        let root = std::env::temp_dir().join(format!("aurora-instances-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("test folder");
        let make = |pid: u32| Instances {
            dir: root.clone(),
            file: root.join(format!("{pid}.{EXTENSION}")),
            last: None,
            peers: Peers::default(),
        };
        let (mut a, mut b) = (make(1), make(2));
        let t0 = Instant::now();
        a.tick(t0, 2000 * MB);
        b.tick(t0, 3000 * MB);
        // a published before b existed: it sees b on its next pass
        a.tick(t0 + PERIOD, 2000 * MB);
        assert_eq!(a.peers().usage, vec![3000 * MB]);
        assert_eq!(b.peers().usage, vec![2000 * MB]);
        // b closes
        b.leave();
        a.tick(t0 + PERIOD * 2, 2000 * MB);
        assert!(!a.peers().any());
        let _ = std::fs::remove_dir_all(&root);
    }
}
