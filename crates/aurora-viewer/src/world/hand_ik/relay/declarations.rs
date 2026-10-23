//! Small regional declarations can precede simulator object/parent updates.
//! They contain identities only; no rig is fetched before live authorization.
use super::*;
use std::collections::VecDeque;

const BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub(super) struct Notice {
    pub source: Seat,
    pub owner: Uuid,
    pub bundle: Uuid,
    pub users: Vec<Uuid>,
}
impl Notice {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.users.capacity() * std::mem::size_of::<Uuid>()
    }
}

#[derive(Default)]
pub(crate) struct Declarations {
    entries: VecDeque<Notice>,
}
impl Declarations {
    pub(super) fn remember(&mut self, notice: Notice) {
        self.entries.retain(|entry| entry.source != notice.source);
        if notice.users.is_empty() {
            return;
        }
        while self.bytes() + notice.bytes() > BYTES || self.entries.len() >= 256 {
            if self.entries.pop_front().is_none() {
                return;
            }
        }
        self.entries.push_back(notice);
    }
    pub(super) fn snapshots(&self) -> Vec<Notice> {
        self.entries.iter().cloned().collect()
    }
    pub(in crate::world::hand_ik) fn bytes(&self) -> usize {
        self.entries.iter().map(Notice::bytes).sum()
    }
}

impl World {
    pub(crate) fn retry_ik_declarations(&mut self, now: Instant) {
        if self.teleporting || self.hand_contacts.relay.is_none() {
            return;
        }
        // Called only after full simulator object updates, never by a timer or
        // terse motion. Reappearance and late parent chains need no discovery.
        for notice in self.hand_contacts.declarations.snapshots() {
            self.apply_ik_notice(&notice, now);
        }
    }
}
