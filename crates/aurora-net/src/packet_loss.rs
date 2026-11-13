//! Confirm missing UDP packets after the grace period of
//! LLCircuitData::checkPacketInID / updateWatchDogTimers (llcircuit.cpp).
//! Pending ranges keep a burst loss cheap without allocating once per ID.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

const MAX_TIMEOUT: Duration = Duration::from_secs(5);
const TIMEOUT_PINGS: u32 = 16;
/// LL_AVERAGED_PING_MIN (llmessage/llcircuit.h).
const MIN_PING: Duration = Duration::from_millis(100);
const SEQUENCE_RESET: u32 = 100_000;
const MAX_RANGES: usize = 65_536;

#[derive(Clone, Copy)]
struct Gap {
    last: u32,
    since: Instant,
}

#[derive(Default)]
pub(crate) struct PacketLoss {
    highest: Option<u32>,
    pending: BTreeMap<u32, Gap>,
    ahead: BTreeSet<u32>,
}

impl PacketLoss {
    /// A resend can fill a hole, but cannot establish a new sequence gap.
    /// Returns the newly skipped IDs and the holes filled by this packet.
    pub fn receive(&mut self, seq: u32, resent: bool, now: Instant) -> (u64, u64) {
        if let Some((&first, &gap)) = self.pending.range(..=seq).next_back()
            && seq <= gap.last
        {
            self.pending.remove(&first);
            if first < seq {
                self.pending.insert(first, Gap { last: seq - 1, ..gap });
            }
            if seq < gap.last {
                self.pending.insert(seq + 1, gap);
            }
            self.bound();
            return (0, 1);
        }
        let Some(high) = self.highest else {
            self.highest = Some(seq);
            return (0, 0);
        };
        if resent {
            if seq > high && seq - high <= SEQUENCE_RESET {
                // A future ordinary packet may reveal a gap around this
                // resend. Remember that this ID has already arrived.
                self.ahead.insert(seq);
                self.bound();
            }
            return (0, 0);
        }
        if high.abs_diff(seq) > SEQUENCE_RESET {
            // A restarted/wrapped sequence is not evidence of link loss.
            self.pending.clear();
            self.ahead.clear();
            self.highest = Some(seq);
            return (0, 0);
        }
        if seq <= high {
            return (0, 0);
        }
        self.highest = Some(seq);
        let mut first = high + 1;
        let mut missing = 0;
        while let Some(&arrived) = self.ahead.first()
            && arrived < seq
        {
            self.ahead.pop_first();
            if first < arrived {
                self.pending.insert(
                    first,
                    Gap {
                        last: arrived - 1,
                        since: now,
                    },
                );
                missing += u64::from(arrived - first);
            }
            first = arrived + 1;
        }
        self.ahead.remove(&seq);
        if first < seq {
            self.pending.insert(first, Gap { last: seq - 1, since: now });
            missing += u64::from(seq - first);
        }
        self.bound();
        (missing, 0)
    }

    fn bound(&mut self) {
        // Even a hostile circuit cannot grow bookkeeping without bound.
        // Rebaseline rather than turn untracked IDs into apparent losses.
        if self.pending.len() + self.ahead.len() > MAX_RANGES {
            self.pending.clear();
            self.ahead.clear();
        }
    }

    /// Confirm each hole once, even if no further datagram arrives. Like
    /// LL_MAX_LOST_TIMEOUT / LL_LOST_TIMEOUT_FACTOR (llmessage/message.h),
    /// allow sixteen smoothed round trips, at most five seconds, with the
    /// reference's 100 ms ping floor to tolerate stalls on fast links.
    pub fn expire(&mut self, now: Instant, rtt: Duration) -> u64 {
        let timeout = rtt.max(MIN_PING).saturating_mul(TIMEOUT_PINGS).min(MAX_TIMEOUT);
        let mut lost = 0;
        // New holes always extend the high sequence, so key order is also
        // age order. Splitting a range preserves its age: a normal tick
        // examines only the oldest range instead of scanning every hole.
        while let Some((&first, &gap)) = self.pending.first_key_value()
            && now.saturating_duration_since(gap.since) > timeout
        {
            self.pending.pop_first();
            lost += u64::from(gap.last - first) + 1;
        }
        lost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_loss_reordering_across_samples_is_not_a_loss() {
        let mut loss = PacketLoss::default();
        let start = Instant::now();
        let rtt = Duration::from_millis(325);
        loss.receive(100, false, start);
        assert_eq!(loss.receive(104, false, start), (3, 0));
        assert_eq!(loss.expire(start + Duration::from_secs(1), rtt), 0);
        // Fill the middle of a range, its beginning, then its end.
        for seq in [102, 101, 103] {
            assert_eq!(loss.receive(seq, true, start + Duration::from_secs(2)), (0, 1));
            assert_eq!(loss.receive(seq, true, start + Duration::from_secs(2)), (0, 0));
        }
        assert_eq!(loss.expire(start + Duration::from_secs(10), rtt), 0);
    }

    #[test]
    fn packet_loss_confirms_real_burst_loss_once_after_the_ping_grace() {
        let mut loss = PacketLoss::default();
        let start = Instant::now();
        let rtt = Duration::from_millis(100);
        loss.receive(10, false, start);
        assert_eq!(loss.receive(111, false, start), (100, 0));
        assert_eq!(loss.pending.len(), 1, "a burst is one range, not 100 allocations");
        loss.receive(50, false, start);
        assert_eq!(loss.expire(start + Duration::from_millis(1600), rtt), 0);
        assert_eq!(loss.expire(start + Duration::from_millis(1601), rtt), 99);
        assert_eq!(loss.expire(start + Duration::from_secs(20), rtt), 0);
        assert_eq!(loss.receive(51, true, start + Duration::from_secs(21)), (0, 0));
    }

    #[test]
    fn packet_loss_grace_is_capped_at_five_seconds() {
        let mut loss = PacketLoss::default();
        let start = Instant::now();
        loss.receive(1, false, start);
        loss.receive(3, false, start);
        let rtt = Duration::from_secs(2);
        assert_eq!(loss.expire(start + MAX_TIMEOUT, rtt), 0);
        assert_eq!(loss.expire(start + MAX_TIMEOUT + Duration::from_millis(1), rtt), 1);
        // A very fast link still gets the reference's 100 ms ping floor.
        let mut loss = PacketLoss::default();
        loss.receive(1, false, start);
        loss.receive(3, false, start);
        assert_eq!(loss.expire(start + Duration::from_millis(1600), Duration::ZERO), 0);
        assert_eq!(loss.expire(start + Duration::from_millis(1601), Duration::ZERO), 1);
    }

    #[test]
    fn packet_loss_a_resend_ahead_of_ordinary_packets_is_not_later_counted_missing() {
        let mut loss = PacketLoss::default();
        let start = Instant::now();
        loss.receive(10, false, start);
        assert_eq!(loss.receive(15, true, start), (0, 0));
        assert_eq!(loss.receive(15, true, start), (0, 0));
        // 11..14 and 16 are missing; 15 has already arrived.
        assert_eq!(loss.receive(17, false, start), (5, 0));
        assert_eq!(loss.expire(start + Duration::from_secs(6), Duration::from_millis(325)), 5);
        assert!(loss.ahead.is_empty());
    }

    #[test]
    fn packet_loss_filling_a_range_keeps_each_holes_original_deadline() {
        let mut loss = PacketLoss::default();
        let start = Instant::now();
        let rtt = Duration::from_millis(100);
        loss.receive(1, false, start);
        loss.receive(5, false, start);
        let later = start + Duration::from_secs(1);
        loss.receive(8, false, later);
        loss.receive(3, true, later);
        // The split first range is due; the newer range must still wait.
        assert_eq!(loss.expire(start + Duration::from_secs(2), rtt), 2);
        assert_eq!(loss.receive(6, true, start + Duration::from_secs(2)), (0, 1));
        assert_eq!(loss.expire(start + Duration::from_secs(3), rtt), 1);
    }

    #[test]
    fn packet_loss_resends_duplicates_and_sequence_resets_do_not_invent_holes() {
        let mut loss = PacketLoss::default();
        let start = Instant::now();
        loss.receive(500_000, false, start);
        assert_eq!(loss.receive(600_000, true, start), (0, 0));
        assert_eq!(loss.receive(500_001, false, start), (0, 0));
        assert_eq!(loss.receive(500_001, false, start), (0, 0));
        loss.receive(500_003, false, start);
        assert_eq!(loss.receive(1, false, start), (0, 0));
        assert_eq!(loss.expire(start + Duration::from_secs(10), Duration::from_millis(100)), 0);
        assert_eq!(loss.receive(u32::MAX - 1, false, start), (0, 0));
        assert_eq!(loss.receive(0, false, start), (0, 0));
        assert_eq!(loss.receive(1, false, start), (0, 0));
    }

    #[test]
    fn packet_loss_tracking_is_bounded_under_fragmented_input() {
        let mut loss = PacketLoss::default();
        let start = Instant::now();
        loss.receive(0, false, start);
        for seq in 1..=MAX_RANGES as u32 + 1 {
            loss.receive(seq * 2, false, start);
        }
        assert!(loss.pending.len() <= MAX_RANGES);
        let high = loss.highest.expect("packets received");
        for offset in 1..=MAX_RANGES as u32 + 1 {
            loss.receive(high + offset, true, start);
        }
        assert!(loss.pending.len() + loss.ahead.len() <= MAX_RANGES);
    }
}
