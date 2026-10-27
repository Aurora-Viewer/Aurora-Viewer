//! The animations objects and residents started on our avatar, for the
//! Animation explorer. Port of RecentAnimationList
//! (indra/newview/animationexplorer.cpp, originally LGPL 2.1): the newest
//! 100, recorded when the simulator first names a source for a motion
//! (process_avatar_animation, indra/newview/llviewermessage.cpp). Our own
//! motions (AO, gestures, the default stand) are only kept while the
//! window is open, so they do not push the others out of the list.

use std::collections::VecDeque;
use std::time::Instant;
use uuid::Uuid;

/// RecentAnimationList's MAX_ANIMATIONS.
pub const MAX: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Stable row id: the window's selection survives the list moving.
    pub id: u64,
    pub animation: Uuid,
    /// The object or avatar that played it (AnimationSourceList).
    pub source: Uuid,
    pub started: Instant,
}

#[derive(Default)]
pub struct AnimationLog {
    entries: VecDeque<Entry>,
    next_id: u64,
    /// The window is open: our own motions are recorded too.
    pub record_own: bool,
    /// Bumped on every change (the window's rows).
    pub generation: u64,
}

impl AnimationLog {
    /// RecentAnimationList::addAnimation.
    pub fn record(&mut self, animation: Uuid, source: Uuid, own: bool, now: Instant) {
        if own && !self.record_own {
            return;
        }
        self.next_id += 1;
        self.entries.push_back(Entry {
            id: self.next_id,
            animation,
            source,
            started: now,
        });
        while self.entries.len() > MAX {
            self.entries.pop_front();
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// Oldest first.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Entry> {
        self.entries.iter()
    }

    pub fn clear(&mut self) {
        *self = Self {
            record_own: self.record_own,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_hundred_and_our_own_motions_only_while_the_window_is_open() {
        let mut log = AnimationLog::default();
        let now = Instant::now();
        let (anim, source) = (Uuid::from_u128(1), Uuid::from_u128(2));
        log.record(anim, source, true, now);
        assert_eq!(log.entries().count(), 0, "own motions stay out while closed");
        log.record_own = true;
        log.record(anim, source, true, now);
        assert_eq!(log.entries().count(), 1);
        for i in 0..MAX as u128 + 5 {
            log.record(Uuid::from_u128(100 + i), source, false, now);
        }
        assert_eq!(log.entries().count(), MAX);
        assert_eq!(log.entries().next().map(|e| e.animation), Some(Uuid::from_u128(105)));
        let ids: Vec<_> = log.entries().map(|e| e.id).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "row ids stay unique and ordered");
        log.clear();
        assert_eq!(log.entries().count(), 0);
        assert!(log.record_own, "clearing keeps the window's state");
    }
}
