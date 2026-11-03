//! Other Aurora viewers running on this PC, and what they hold of the card.
//!
//! The driver's memory budget (aurora_render::vram) already falls when a
//! second viewer takes video memory, but only once it has taken it: two
//! viewers loading a busy region together each read a large budget and grow
//! at the same time, until the card is full, Windows moves memory to system
//! RAM and the driver resets the GPU (a tester's crash with two viewers
//! open: "GPU device lost"). So the viewers tell each other. Each one
//! rewrites a small file of its own in the `instances` folder every
//! [`PERIOD`] (its video memory in use, whether its window is in front, the
//! card it draws on, and the time) and reads the others'. A file not
//! rewritten for [`STALE_AFTER`] is a viewer that closed or crashed, and is
//! ignored. Plain files, no lock and no shared memory: a file read
//! half-written or unreadable is skipped, and the next pass has it. The disk
//! work is done by a thread of its own, never by the frame.
//!
//! Two uses, both only while another viewer is there:
//! - the memory the texture streamer may count on is its fair share of what
//!   the viewers have together ([`share_cap`], `scene/textures.rs`), each
//!   viewer leaving room to the others that are loading (in front);
//! - a window that is not in front draws at [`BEHIND_FPS`] at most while
//!   another viewer is in front, unless the user turned the background cap
//!   off ([`throttle`]): two viewers
//!   drawing a busy scene at full rate kept the GPU so busy that frames took
//!   73 ms before the driver gave up on it. The Legacy Viewer has neither: it
//!   does not know that another viewer is running.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// What a viewer says of itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Own {
    /// Bytes of video memory it holds (what the driver counts), 0 when unknown.
    pub usage: u64,
    /// Its window is in front and not minimised: the one that is loading.
    pub active: bool,
    /// The card it draws on (aurora_render::vram, `card`), 0 when unknown.
    pub luid: u64,
}

/// The other viewers alive now, on this viewer's card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peers {
    pub list: Vec<Own>,
}

impl Peers {
    /// One of them has the focus: the card is theirs to load.
    pub fn in_front(&self) -> bool {
        self.list.iter().any(|p| p.active)
    }

    /// The bytes held by the ones that are loading: in front and holding
    /// something. A window behind, paused or still at the login screen needs
    /// no room.
    fn growing(&self) -> Vec<u64> {
        self.list.iter().filter(|p| p.active && p.usage > 0).map(|p| p.usage).collect()
    }
}

struct State {
    own: Own,
    peers: Peers,
    stop: bool,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.0.lock().unwrap_or_else(|e| e.into_inner())
}

/// This viewer's view of the others, refreshed every [`PERIOD`] by a thread
/// (a folder to create, a file to rewrite and rename, the others' to read:
/// an antivirus may hold any of them).
pub struct Instances {
    dir: PathBuf,
    file: PathBuf,
    shared: Shared,
    started: bool,
}

impl Instances {
    pub fn new() -> Self {
        let dir = crate::logging::logs_dir().with_file_name("instances");
        let file = dir.join(format!("{}.{EXTENSION}", std::process::id()));
        Instances {
            dir,
            file,
            shared: Arc::new((
                Mutex::new(State {
                    own: Own::default(),
                    peers: Peers::default(),
                    stop: false,
                }),
                Condvar::new(),
            )),
            started: false,
        }
    }

    /// Each frame: what this viewer says of itself. Nothing is published or
    /// read while `enabled` is false (demo and timed runs: a viewer that
    /// shows a fake world or measures must not take room from the one on the
    /// grid, nor have its own figures skewed by it).
    pub fn tick(&mut self, enabled: bool, own: Own) {
        if !enabled {
            return;
        }
        lock(&self.shared).own = own;
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let (dir, file, shared) = (self.dir.clone(), self.file.clone(), self.shared.clone());
        let spawned = std::thread::Builder::new()
            .name("instances".into())
            .spawn(move || run(&dir, &file, &shared));
        if let Err(e) = spawned {
            log::warn!("instances: the thread could not start: {e}");
        }
    }

    /// The others, as of the last pass.
    pub fn peers(&self) -> Peers {
        lock(&self.shared).peers.clone()
    }

    /// The viewer closes: its file goes, the others do not wait for it to
    /// turn stale.
    pub fn leave(&self) {
        if !self.started {
            return;
        }
        lock(&self.shared).stop = true;
        self.shared.1.notify_all();
        let _ = std::fs::remove_file(&self.file);
    }
}

fn run(dir: &Path, file: &Path, shared: &Shared) {
    let mut count = 0;
    loop {
        let own = {
            let state = lock(shared);
            if state.stop {
                return;
            }
            state.own
        };
        let peers = pass(dir, file, own, unix_secs());
        let mut state = lock(shared);
        // (a viewer that closed meanwhile removed its file: written again
        // here it would only turn stale)
        if state.stop {
            let _ = std::fs::remove_file(file);
            return;
        }
        if peers.list.len() != count {
            count = peers.list.len();
            log::info!(
                "instances: {count} other viewer(s) running, holding {} MB of video memory",
                peers.list.iter().fold(0u64, |s, p| s.saturating_add(p.usage)) >> 20
            );
        }
        state.peers = peers;
        let (state, _) = shared
            .1
            .wait_timeout_while(state, PERIOD, |s| !s.stop)
            .unwrap_or_else(|e| e.into_inner());
        if state.stop {
            return;
        }
    }
}

/// One pass: publish `own`, read the others on the same card.
fn pass(dir: &Path, file: &Path, own: Own, secs: u64) -> Peers {
    if std::fs::create_dir_all(dir).is_ok() {
        // (a failed write leaves the others without us: the next pass tries
        // again)
        if let Err(e) = crate::atomic_file::publish(
            file,
            format!("{secs} {} {} {:x}", own.usage, u8::from(own.active), own.luid).as_bytes(),
        ) {
            log::debug!("instances: could not publish {}: {e}", file.display());
        }
    }
    let mut entries = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != EXTENSION) || path == file {
                continue;
            }
            // (a file half written, or not ours, is skipped)
            if let Some(entry) = std::fs::read_to_string(&path).ok().as_deref().and_then(parse) {
                if secs.saturating_sub(entry.0) > REMOVE_AFTER {
                    let _ = std::fs::remove_file(&path);
                }
                entries.push(entry);
            }
        }
    }
    alive(&entries, secs, own.luid)
}

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `"<unix seconds> <bytes> <1 if in front> <card, hexadecimal>"`; the last
/// two are optional (a viewer that does not say is taken as in front on a
/// card unknown).
fn parse(text: &str) -> Option<(u64, Own)> {
    let mut parts = text.split_whitespace();
    let stamp = parts.next()?.parse().ok()?;
    let usage = parts.next()?.parse().ok()?;
    let active = parts.next().is_none_or(|a| a != "0");
    let luid = parts.next().and_then(|l| u64::from_str_radix(l, 16).ok()).unwrap_or(0);
    Some((stamp, Own { usage, active, luid }))
}

/// The viewers still alive at `now` on the card `luid` (0: unknown, every
/// viewer counts; two viewers on different GPUs share nothing).
fn alive(entries: &[(u64, Own)], now: u64, luid: u64) -> Peers {
    Peers {
        list: entries
            .iter()
            .filter(|(stamp, o)| now.saturating_sub(*stamp) <= STALE_AFTER && (luid == 0 || o.luid == 0 || o.luid == luid))
            .map(|&(_, o)| o)
            .collect(),
    }
}

/// The part of the driver's `budget` this viewer should count on with
/// `peers` running: the viewers that are loading (in front) share what they
/// have together (the budget plus what they hold), and each is left the room
/// up to its share before this one takes the rest. A viewer holding more than
/// its share leaves nothing: the driver's own budget, already lower, rules.
/// Never under an equal slice of the budget: a neighbour far past its share
/// must not take the room of one that is not. A window behind, a paused
/// viewer or one that holds nothing yet leaves nothing aside: the viewer in
/// front keeps the whole budget.
pub fn share_cap(budget: u64, peers: &Peers) -> u64 {
    let growing = peers.growing();
    if growing.is_empty() {
        return budget;
    }
    let held: u128 = growing.iter().map(|&u| u as u128).sum();
    let share = (budget as u128 + held) / (growing.len() as u128 + 1);
    let room: u128 = growing.iter().map(|&u| share.saturating_sub(u as u128)).sum();
    let slice = budget as u128 / (growing.len() as u128 + 1);
    (budget as u128).saturating_sub(room).max(slice) as u64
}

/// The frame cap (0 = none) with the card shared (`shared`: another viewer is
/// in front): a window that is not in front draws at [`BEHIND_FPS`] at most. `exempt` leaves the cap
/// alone: `timed` runs (captures, profiles) are measured in windows that
/// never had the focus, and the user may have turned the background cap off.
pub fn throttle(cap: f32, focused: bool, shared: bool, exempt: bool) -> f32 {
    if focused || exempt || !shared {
        return cap;
    }
    if cap > 0.0 { cap.min(BEHIND_FPS) } else { BEHIND_FPS }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    fn peer(usage: u64, active: bool) -> Own {
        Own {
            usage: usage * MB,
            active,
            luid: 0,
        }
    }

    /// Viewers in front holding `usage` MB each.
    fn peers(usage: &[u64]) -> Peers {
        Peers {
            list: usage.iter().map(|&u| peer(u, true)).collect(),
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
    fn a_neighbour_that_is_not_loading_is_left_nothing() {
        // an alt paused or behind at 1.5 GB: the viewer in front keeps the
        // whole 10 GB budget
        let behind = Peers {
            list: vec![peer(1500, false)],
        };
        assert_eq!(share_cap(10_000 * MB, &behind), 10_000 * MB);
        // nothing held yet (login screen)
        assert_eq!(share_cap(10_000 * MB, &peers(&[0])), 10_000 * MB);
        // only the one in front counts among several
        let mixed = Peers {
            list: vec![peer(1500, false), peer(1000, true)],
        };
        assert_eq!(share_cap(8000 * MB, &mixed), 4500 * MB);
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
        // sizes read from a file are not trusted to add up
        let huge = Peers {
            list: vec![
                Own {
                    usage: u64::MAX,
                    active: true,
                    luid: 0
                };
                3
            ],
        };
        assert!(share_cap(1000 * MB, &huge) >= 250 * MB);
    }

    #[test]
    fn files_are_read_back_and_stale_ones_are_ignored() {
        let own = |usage, active, luid| Own { usage, active, luid };
        assert_eq!(
            parse("1700000000 3130000000 1 1095e"),
            Some((1_700_000_000, own(3_130_000_000, true, 0x1095e)))
        );
        assert_eq!(parse("1700000000 5 0 0"), Some((1_700_000_000, own(5, false, 0))));
        // the fields a viewer may leave out
        assert_eq!(parse("1700000000 5"), Some((1_700_000_000, own(5, true, 0))));
        assert_eq!(parse("17000"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("a b"), None);
        let found = alive(
            &[
                (100, own(5, true, 0)),
                (92, own(6, true, 0)),
                (91, own(7, true, 0)),
                (0, own(8, true, 0)),
            ],
            100,
            0,
        );
        assert_eq!(found.list.iter().map(|p| p.usage).collect::<Vec<_>>(), vec![5, 6]);
    }

    #[test]
    fn viewers_on_another_card_share_nothing() {
        let own = |luid| Own {
            usage: 1,
            active: true,
            luid,
        };
        let entries = [(100, own(0xa)), (100, own(0xb)), (100, own(0))];
        let found = alive(&entries, 100, 0xa);
        assert_eq!(found.list.iter().map(|p| p.luid).collect::<Vec<_>>(), vec![0xa, 0]);
        // this viewer's card not known yet: every viewer counts
        assert_eq!(alive(&entries, 100, 0).list.len(), 3);
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
        // measured runs, and the background cap turned off, are left alone
        assert_eq!(throttle(60.0, false, with, true), 60.0);
        assert_eq!(throttle(0.0, false, with, true), 0.0);
    }

    #[test]
    fn two_viewers_see_each_other_through_their_folder() {
        let root = std::env::temp_dir().join(format!("aurora-instances-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("test folder");
        let file = |pid: u32| root.join(format!("{pid}.{EXTENSION}"));
        let own = |usage: u64, active| Own {
            usage: usage * MB,
            active,
            luid: 7,
        };
        let t0 = unix_secs();
        pass(&root, &file(1), own(2000, true), t0);
        let seen_by_b = pass(&root, &file(2), own(3000, false), t0);
        // a published before b existed: it sees b on its next pass
        let seen_by_a = pass(&root, &file(1), own(2000, true), t0 + 2);
        assert_eq!(seen_by_a.list, vec![own(3000, false)]);
        assert_eq!(seen_by_b.list, vec![own(2000, true)]);
        // b closes
        let _ = std::fs::remove_file(file(2));
        assert!(pass(&root, &file(1), own(2000, true), t0 + 4).list.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
