//! How long a teleport or a crossing waits for the region to confirm the
//! arrival (AgentMovementComplete) before the viewer stops waiting.
//!
//! Port of the give-up at the top of the teleport display code
//! (TELEPORT_EXPIRY, indra/newview/llviewerdisplay.cpp, originally LGPL 2.1):
//! "Give up. Don't keep the UI locked forever." The session goes on, and a late
//! confirmation is still taken.
//!
//! Differences from the Legacy Viewer, both for the resident stuck in that
//! state: giving up says so in a notice, where the Legacy Viewer is silent. And
//! a crossing waits the same time: the Legacy Viewer keeps sending agent
//! updates during one, while Aurora holds them until the arrival, so it must
//! let them go at some point.

use std::time::{Duration, Instant};

use aurora_net::NetEvent;

/// TELEPORT_EXPIRY.
const EXPIRY: Duration = Duration::from_secs(15);
/// TELEPORT_EXPIRY_PER_ATTACHMENT.
const EXPIRY_PER_ATTACHMENT: Duration = Duration::from_secs(3);

/// The longest wait, for an avatar wearing this many attachments.
fn patience(attachments: usize) -> Duration {
    EXPIRY + EXPIRY_PER_ATTACHMENT * attachments as u32
}

impl super::World {
    /// Whether a teleport or crossing has waited too long for its arrival.
    /// Called every frame: the wait starts the first time it sees one, so every
    /// way into a teleport is covered, the region's own included. The login has
    /// its own time-out in the session.
    pub fn arrival_overdue(&mut self, now: Instant, attachments: usize) -> bool {
        if !self.arrived_once || !(self.teleporting || !self.movement_complete) {
            self.arrival_wait_since = None;
            return false;
        }
        let since = *self.arrival_wait_since.get_or_insert(now);
        now.saturating_duration_since(since) > patience(attachments)
    }

    /// Stop waiting: the world is usable again and the resident knows why
    /// nothing arrived. The caller tells the session (`ArrivalGivenUp`).
    pub fn give_up_arrival(&mut self) {
        self.arrival_wait_since = None;
        self.movement_complete = true;
        self.crossing = false;
        let reason = aurora_i18n::tr!("world-arrival-not-confirmed");
        if self.teleporting {
            self.apply(NetEvent::TeleportFailed { reason: reason.into() });
        } else {
            self.system_notice(reason, super::notice_topics::Topic::Teleport);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patience_follows_the_legacy_formula() {
        assert_eq!(patience(0), Duration::from_secs(15));
        assert_eq!(patience(38), Duration::from_secs(129));
    }
}
