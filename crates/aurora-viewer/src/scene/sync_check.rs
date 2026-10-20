//! AURORA_SYNC_CHECK: what the scene sync visited during each second, and
//! what came of it. One log line per second: the objects listed per frame
//! by the set that listed them (sync_sets.rs), by what they are, and for
//! each kind how many only had their placement checked, had their records
//! written again, or went through the full sync. Tells which objects keep
//! the sync busy in a place at rest. The line ends with a check of every
//! object: those whose wanted detail level is not the one their distance
//! gives at that moment (a check the sync has not made yet), apart for what
//! the GPU places from an avatar. That check goes through the objects a
//! slice per frame ([`SweepRange`]) and the line gives the last full pass:
//! made at once, it cost 5 ms in one frame of each second at KB (22,000
//! objects), the spikes the check was there to explain. Costs nothing when
//! the variable is not set.

use super::ObjGpu;
use crate::world::World;
use crate::world::objects::Object;
use std::ops::Range;
use std::time::{Duration, Instant};

/// Frames a pass of the detail level check over every object takes at
/// most (under a second at 160 fps).
const SWEEP_FRAMES: usize = 120;
/// Objects the check looks at per frame at least.
const SWEEP_MIN: usize = 64;

/// The slice of the objects the detail level check goes through in a
/// frame, and the counts of the pass so far.
#[derive(Default)]
struct Sweep {
    next: usize,
    sum: [usize; 4],
    /// The counts of the last full pass.
    last: Option<[usize; 4]>,
}

/// What a visited object is, for the sync: the first that applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Avatar,
    /// On a HUD point of our avatar.
    Hud,
    Flexible,
    /// Velocity, acceleration or spin given by the simulator.
    Moving,
    /// Rigged mesh the sync places with its skeleton (not the GPU).
    Rigged,
    /// Placed by the GPU from an anchor of its wearer (anchors.rs).
    Anchored,
    /// Worn by an avatar and placed by the sync.
    Worn,
    /// Linked below an object that moves by itself.
    Follower,
    /// Nothing of the above: an object at rest.
    Still,
}

impl Kind {
    const ALL: [Kind; 9] = [
        Kind::Avatar,
        Kind::Hud,
        Kind::Flexible,
        Kind::Moving,
        Kind::Rigged,
        Kind::Anchored,
        Kind::Worn,
        Kind::Follower,
        Kind::Still,
    ];

    fn name(self) -> &'static str {
        match self {
            Kind::Avatar => "avatar",
            Kind::Hud => "hud",
            Kind::Flexible => "flexible",
            Kind::Moving => "moving",
            Kind::Rigged => "rigged",
            Kind::Anchored => "anchored",
            Kind::Worn => "worn",
            Kind::Follower => "follower",
            Kind::Still => "still",
        }
    }

    /// The kind of object `idx`, from the store and its scene entry.
    pub fn of(world: &World, idx: usize, o: &Object, g: Option<&ObjGpu>) -> Kind {
        if o.is_avatar() {
            return Kind::Avatar;
        }
        if g.is_some_and(|g| g.hud) {
            return Kind::Hud;
        }
        if o.is_flexible() {
            return Kind::Flexible;
        }
        if o.has_motion() {
            return Kind::Moving;
        }
        if g.is_some_and(|g| g.moves_with_skeleton()) {
            return Kind::Rigged;
        }
        if g.is_some_and(|g| g.anchor.is_some()) {
            return Kind::Anchored;
        }
        // up the parent chain: an avatar (worn), or something that moves
        let mut cur = idx;
        for _ in 0..super::sync_sets::MAX_DEPTH {
            let Some(p) = world.objects.get(cur).and_then(|c| world.objects.parent_of(c)) else {
                break;
            };
            match world.objects.get(p) {
                Some(parent) if parent.is_avatar() => return Kind::Worn,
                Some(parent) if parent.has_motion() || parent.is_flexible() => return Kind::Follower,
                Some(_) => cur = p,
                None => break,
            }
        }
        Kind::Still
    }
}

/// What the sync did with a visited object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Placement computed, records already right: nothing written.
    Checked,
    /// Records written again with a new placement.
    Moved,
    /// Full sync (avatars always; geometry, faces, materials).
    Full,
    /// Full sync put off to a later frame (backlog, arrivals).
    Waiting,
}

const KINDS: usize = Kind::ALL.len();
const OUTCOMES: usize = 4;

/// Objects listed by each set of the frame list, in the order they are
/// asked (an object listed by two sets counts for the first).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    /// Objects moving by themselves and what follows them.
    pub moving: usize,
    /// Changed objects (and their linkset when they were placed again).
    pub dirty: usize,
    /// Objects whose geometry became ready.
    pub pending: usize,
    /// Detail level check.
    pub lod: usize,
}

pub struct SyncCheck {
    since: Instant,
    frames: u32,
    sources: [u64; 4],
    /// Objects the detail level check looked at (listed or not).
    lod_checked: u64,
    by_kind: [[u64; OUTCOMES]; KINDS],
    sweep: Sweep,
}

impl SyncCheck {
    /// The check, when AURORA_SYNC_CHECK asks for it.
    pub fn from_env() -> Option<SyncCheck> {
        std::env::var_os("AURORA_SYNC_CHECK")
            .filter(|v| v != "0")
            .map(|_| SyncCheck::new(Instant::now()))
    }

    /// The check started now (the Dev Toolbox's P5).
    pub fn start() -> SyncCheck {
        SyncCheck::new(Instant::now())
    }

    fn new(now: Instant) -> SyncCheck {
        SyncCheck {
            since: now,
            frames: 0,
            sources: [0; 4],
            lod_checked: 0,
            by_kind: [[0; OUTCOMES]; KINDS],
            sweep: Sweep::default(),
        }
    }

    /// The objects the detail level check looks at in this frame, among
    /// `n`.
    pub fn sweep_range(&mut self, n: usize) -> Range<usize> {
        if self.sweep.next >= n {
            self.sweep.next = 0;
            self.sweep.sum = [0; 4];
        }
        let from = self.sweep.next;
        let to = (from + n.div_ceil(SWEEP_FRAMES).max(SWEEP_MIN)).min(n);
        self.sweep.next = to;
        from..to
    }

    /// The counts of the frame's slice ([`SyncCheck::sweep_range`]); `end`:
    /// the slice ended the pass.
    pub fn swept(&mut self, late: [usize; 4], end: bool) {
        for (sum, v) in self.sweep.sum.iter_mut().zip(late) {
            *sum += v;
        }
        if end {
            self.sweep.last = Some(self.sweep.sum);
        }
    }

    /// The frame's list. `lod_checked`: the objects the detail level check
    /// looked at to list its part.
    pub fn listed(&mut self, s: Sources, lod_checked: usize) {
        for (sum, v) in self.sources.iter_mut().zip([s.moving, s.dirty, s.pending, s.lod]) {
            *sum += v as u64;
        }
        self.lod_checked += lod_checked as u64;
    }

    /// A listed object went through the sync.
    pub fn visited(&mut self, kind: Kind, outcome: Outcome) {
        self.by_kind[kind as usize][outcome as usize] += 1;
    }

    /// Once per frame, after the sync: whether the second is over and its
    /// line due ([`SyncCheck::write`]).
    pub fn frame(&mut self) -> bool {
        self.frames += 1;
        self.since.elapsed() >= Duration::from_secs(1)
    }

    /// Write the second's line, with the last full pass of the detail level
    /// check: objects at rest looked at and those whose detail level is
    /// behind their distance, then the same for the objects the GPU places
    /// from an avatar.
    pub fn write(&mut self) {
        let late = self.sweep.last.unwrap_or_default();
        log::info!(
            "sync check: {} | detail level behind: {} of {} at rest, {} of {} placed by the GPU",
            self.line(),
            late[1],
            late[0],
            late[3],
            late[2]
        );
        let sweep = std::mem::take(&mut self.sweep);
        *self = SyncCheck::new(Instant::now());
        self.sweep = sweep;
    }

    /// Averages per frame over the period.
    fn line(&self) -> String {
        let n = self.frames.max(1) as f32;
        let per = |v: u64| v as f32 / n;
        let s = &self.sources;
        let mut out = format!(
            "{} frames, per frame: listed {:.0} (moving {:.0}, changed {:.0}, geometry {:.0}, lod {:.0} of {:.0} looked at) | \
             kind: checked / moved / full / waiting |",
            self.frames,
            per(s.iter().sum()),
            per(s[0]),
            per(s[1]),
            per(s[2]),
            per(s[3]),
            per(self.lod_checked),
        );
        for (kind, c) in Kind::ALL.iter().zip(&self.by_kind) {
            if c.iter().all(|&v| v == 0) {
                continue;
            }
            out.push_str(&format!(
                " {} {:.1} / {:.1} / {:.1} / {:.1},",
                kind.name(),
                per(c[0]),
                per(c[1]),
                per(c[2]),
                per(c[3])
            ));
        }
        out.pop();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_gives_averages_per_frame_and_skips_absent_kinds() {
        let mut c = SyncCheck::new(Instant::now());
        for _ in 0..2 {
            c.listed(
                Sources {
                    moving: 10,
                    dirty: 2,
                    pending: 0,
                    lod: 30,
                },
                40,
            );
            c.visited(Kind::Avatar, Outcome::Full);
            c.visited(Kind::Still, Outcome::Checked);
            c.visited(Kind::Still, Outcome::Checked);
            c.visited(Kind::Moving, Outcome::Moved);
            c.frames += 1;
        }
        c.visited(Kind::Still, Outcome::Waiting);
        let l = c.line();
        assert!(
            l.starts_with("2 frames, per frame: listed 42 (moving 10, changed 2, geometry 0, lod 30 of 40 looked at)"),
            "{l}"
        );
        assert!(l.contains(" avatar 0.0 / 0.0 / 1.0 / 0.0,"), "{l}");
        assert!(l.contains(" moving 0.0 / 1.0 / 0.0 / 0.0,"), "{l}");
        assert!(l.ends_with(" still 2.0 / 0.0 / 0.0 / 0.5"), "{l}");
        assert!(!l.contains("hud") && !l.contains("worn"), "{l}");
    }

    #[test]
    fn the_level_check_goes_through_every_object_over_frames() {
        let mut c = SyncCheck::new(Instant::now());
        let n = 10_000;
        let mut seen = 0;
        let mut frames = 0;
        loop {
            let r = c.sweep_range(n);
            assert_eq!(r.start, seen);
            assert!(r.len() <= n.div_ceil(SWEEP_FRAMES).max(SWEEP_MIN));
            seen = r.end;
            frames += 1;
            c.swept([r.len(), 1, 0, 0], r.end == n);
            if r.end == n {
                break;
            }
        }
        assert!(frames <= SWEEP_FRAMES, "{frames}");
        assert_eq!(c.sweep.last, Some([n, frames, 0, 0]));
        // the next pass starts again from the first object, the line keeps
        // the last full pass across seconds
        assert_eq!(c.sweep_range(n).start, 0);
        c.write();
        assert_eq!(c.sweep.last, Some([n, frames, 0, 0]));
    }
}
