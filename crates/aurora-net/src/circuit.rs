//! Reliable UDP circuit bookkeeping (port of the essentials of
//! `llmessage/llcircuit.cpp`): sequence numbers, resends, acks, dedupe.

use crate::packet_loss::PacketLoss;
use crate::stats::NetStats;
use aurora_msg::packet::{PacketFlags, build_raw};
use aurora_msg::{Msg, Writer};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

/// Resends of an ordinary reliable packet before it is given up.
/// `LL_DEFAULT_RELIABLE_RETRIES` (indra/llmessage/message.h): three resends,
/// then one more wait (`LLCircuitData::resendUnackedPackets`).
const MAX_RESENDS: u8 = 3;
const DEDUPE_WINDOW: usize = 2048;
const ACK_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

/// Wait before an ordinary reliable packet is resent.
///
/// `LLMessageSystem::sendReliable` (indra/llmessage/message.cpp) uses
/// `max(LL_MINIMUM_RELIABLE_TIMEOUT_SECONDS, LL_RELIABLE_TIMEOUT_FACTOR *
/// getPingDelayAveraged())`. `LLCircuitData::getPingDelayAveraged`
/// (indra/llmessage/llcircuit.cpp) is the smoothed ping, at least the time
/// the oldest ping has been unanswered, and at most `LL_AVERAGED_PING_MAX`
/// (2 s, llcircuit.h). A 1 s average therefore waits 5 s. There is no
/// separate ceiling on the wait. The 100 ms ping floor
/// (`LL_AVERAGED_PING_MIN`) cannot change this: five times 100 ms is under
/// the 1 s minimum. `averaged` is already smoothed in `on_ping_reply`, not
/// a raw sample.
fn reliable_timeout(averaged: Duration, ping_out: Option<Duration>) -> Duration {
    /// `LL_AVERAGED_PING_MAX`.
    const PING_MAX: Duration = Duration::from_secs(2);
    /// `LL_MINIMUM_RELIABLE_TIMEOUT_SECONDS`.
    const MINIMUM: Duration = Duration::from_secs(1);
    let delay = ping_out.unwrap_or_default().max(averaged).min(PING_MAX);
    (delay * 5).max(MINIMUM)
}

/// Resend policy of a reliable packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resend {
    /// Fixed resend timeout; `None` derives it from the ping time.
    pub timeout: Option<Duration>,
    /// Resends before the packet is given up on.
    pub max_retries: u8,
}

impl Resend {
    pub const DEFAULT: Resend = Resend {
        timeout: None,
        max_retries: MAX_RESENDS,
    };
    /// The login UseCircuitCode (llstartup.cpp STATE_WORLD_INIT: sendReliable
    /// with UseCircuitCodeMaxRetries = 3 and UseCircuitCodeTimeout = 5 s).
    pub const LOGIN_USE_CIRCUIT_CODE: Resend = Resend {
        timeout: Some(Duration::from_secs(5)),
        max_retries: 3,
    };
}

struct Unacked {
    payload: Vec<u8>,
    zerocoded: bool,
    sent: Instant,
    retries: u8,
    policy: Resend,
}

pub struct Circuit {
    pub addr: SocketAddr,
    next_seq: u32,
    unacked: HashMap<u32, Unacked>,
    pending_acks: Vec<u32>,
    last_ack_flush: Instant,
    recent: VecDeque<u32>,
    recent_set: HashSet<u32>,
    pub last_recv: Instant,
    pub rtt: Duration,
    ping_id: u8,
    ping_sent: Option<(u8, Instant)>,
    /// Oldest ping still unanswered: new pings must not shorten the loss
    /// grace during a stall (getPingInTransitTime, llcircuit.cpp).
    ping_wait_started: Option<Instant>,
    buf: Vec<u8>,
    /// Reliable packets given up on since the last `take_dropped`.
    dropped: Vec<u32>,
    packet_loss: PacketLoss,
}

impl Circuit {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            next_seq: 1,
            unacked: HashMap::new(),
            pending_acks: Vec::new(),
            last_ack_flush: Instant::now(),
            recent: VecDeque::with_capacity(DEDUPE_WINDOW),
            recent_set: HashSet::with_capacity(DEDUPE_WINDOW),
            last_recv: Instant::now(),
            rtt: Duration::from_millis(300),
            ping_id: 0,
            ping_sent: None,
            ping_wait_started: None,
            buf: Vec::with_capacity(1500),
            dropped: Vec::new(),
            packet_loss: PacketLoss::default(),
        }
    }

    /// Record provisional holes and recoveries; only the circuit tick
    /// confirms losses after the ping-dependent grace period.
    pub fn note_sequence(&mut self, seq: u32, resent: bool, stats: &NetStats) {
        let (gaps, late) = self.packet_loss.receive(seq, resent, Instant::now());
        if gaps > 0 {
            stats.seq_gaps.fetch_add(gaps, Ordering::Relaxed);
        }
        if late > 0 {
            stats.seq_late.fetch_add(late, Ordering::Relaxed);
        }
    }

    fn raw_send(&mut self, sock: &UdpSocket, stats: &NetStats) {
        match sock.try_send_to(&self.buf, self.addr) {
            Ok(n) => {
                stats.packets_out.fetch_add(1, Ordering::Relaxed);
                stats.bytes_out.fetch_add(n as u64, Ordering::Relaxed);
            }
            Err(e) => {
                log::trace!("udp send to {} failed: {e}", self.addr);
                stats.send_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Encode and send a message, piggybacking pending acks. Returns the sequence number.
    pub fn send<M: Msg>(&mut self, sock: &UdpSocket, stats: &NetStats, msg: &M, reliable: bool) -> u32 {
        self.send_with(sock, stats, msg, reliable.then_some(Resend::DEFAULT))
    }

    /// Send a reliable message with its own resend policy.
    pub fn send_reliable_with<M: Msg>(&mut self, sock: &UdpSocket, stats: &NetStats, msg: &M, policy: Resend) -> u32 {
        self.send_with(sock, stats, msg, Some(policy))
    }

    fn send_with<M: Msg>(&mut self, sock: &UdpSocket, stats: &NetStats, msg: &M, reliable: Option<Resend>) -> u32 {
        let mut w = Writer::new();
        M::ID.encode(&mut w.buf);
        msg.encode_body(&mut w);
        let seq = self.next_seq;
        self.send_payload(sock, stats, w.buf, M::ZEROCODED, reliable);
        seq
    }

    /// True while a reliable packet is still waiting for its ack.
    pub fn is_unacked(&self, seq: u32) -> bool {
        self.unacked.contains_key(&seq)
    }

    fn send_payload(&mut self, sock: &UdpSocket, stats: &NetStats, payload: Vec<u8>, zerocoded: bool, reliable: Option<Resend>) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let mut buf = std::mem::take(&mut self.buf);
        let n = build_raw(&payload, zerocoded, seq, reliable.is_some(), false, &self.pending_acks, &mut buf);
        self.pending_acks.drain(..n);
        crate::probe::message_out(&payload, buf.len());
        self.buf = buf;
        self.raw_send(sock, stats);
        if let Some(policy) = reliable {
            self.unacked.insert(
                seq,
                Unacked {
                    payload,
                    zerocoded,
                    sent: Instant::now(),
                    retries: 0,
                    policy,
                },
            );
        }
    }

    /// Process the header of an incoming packet (the acks it carries).
    /// Returns `false` for a resend of a packet already processed: it is
    /// acked again to stop further resends and must not be processed.
    /// A new packet is acked only once processed, see [`Circuit::accept`].
    pub fn on_receive(&mut self, flags: u8, seq: u32, acks: &[u32]) -> bool {
        self.last_recv = Instant::now();
        for a in acks {
            self.unacked.remove(a);
        }
        // Like LL (message.cpp), only packets flagged RESENT can be duplicates;
        // this also survives a simulator resetting its sequence numbers.
        if flags & PacketFlags::RESENT != 0 && self.recent_set.contains(&seq) {
            if flags & PacketFlags::RELIABLE != 0 {
                self.pending_acks.push(seq);
            }
            return false;
        }
        true
    }

    /// A packet was decoded and handled: ack it if reliable and remember it
    /// for duplicate suppression. Like LLMessageSystem::checkMessages, a
    /// packet that fails to decode is neither acked nor remembered, so the
    /// simulator resends it instead of it being lost for good.
    pub fn accept(&mut self, flags: u8, seq: u32) {
        if flags & PacketFlags::RELIABLE != 0 {
            self.pending_acks.push(seq);
        }
        if self.recent_set.contains(&seq) {
            return;
        }
        if self.recent.len() >= DEDUPE_WINDOW
            && let Some(old) = self.recent.pop_front()
        {
            self.recent_set.remove(&old);
        }
        self.recent.push_back(seq);
        self.recent_set.insert(seq);
    }

    /// Reliable packets given up on since the last call.
    pub fn take_dropped(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.dropped)
    }

    pub fn ack_received(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            self.unacked.remove(&id);
        }
    }

    pub fn unacked_count(&self) -> usize {
        self.unacked.len()
    }

    /// Resend timed-out reliable packets and flush standalone acks.
    pub fn tick(&mut self, sock: &UdpSocket, stats: &NetStats) {
        let now = Instant::now();
        let loss_rtt = self
            .ping_wait_started
            .map_or(self.rtt, |at| self.rtt.max(now.saturating_duration_since(at)));
        let lost = self.packet_loss.expire(now, loss_rtt);
        if lost > 0 {
            stats.packets_lost.fetch_add(lost, Ordering::Relaxed);
        }
        // Ordinary reliable packets wait as `LLMessageSystem::sendReliable` does.
        // `ping_out` stands in for `getPingInTransitTime`. A fixed policy (the
        // login circuit code) keeps its own timeout.
        let ping_out = self.ping_wait_started.map(|at| now.saturating_duration_since(at));
        let ping_timeout = reliable_timeout(self.rtt, ping_out);
        let mut to_resend = Vec::new();
        let mut to_drop = Vec::new();
        for (seq, u) in &self.unacked {
            if now.duration_since(u.sent) > u.policy.timeout.unwrap_or(ping_timeout) {
                if u.retries >= u.policy.max_retries {
                    to_drop.push(*seq);
                } else {
                    to_resend.push(*seq);
                }
            }
        }
        for seq in to_drop {
            self.unacked.remove(&seq);
            self.dropped.push(seq);
            stats.dropped_reliable.fetch_add(1, Ordering::Relaxed);
        }
        for seq in to_resend {
            let Some(u) = self.unacked.get_mut(&seq) else {
                continue;
            };
            u.retries += 1;
            // New expiration, as `resendUnackedPackets` sets after each resend.
            u.sent = now;
            let mut buf = std::mem::take(&mut self.buf);
            build_raw(&u.payload, u.zerocoded, seq, true, true, &[], &mut buf);
            crate::probe::message_out(&u.payload, buf.len());
            self.buf = buf;
            self.raw_send(sock, stats);
            stats.resent.fetch_add(1, Ordering::Relaxed);
        }
        if !self.pending_acks.is_empty() && now.duration_since(self.last_ack_flush) >= ACK_FLUSH_INTERVAL {
            self.flush_acks(sock, stats);
        }
    }

    pub fn flush_acks(&mut self, sock: &UdpSocket, stats: &NetStats) {
        self.last_ack_flush = Instant::now();
        while !self.pending_acks.is_empty() {
            let n = self.pending_acks.len().min(250);
            let ids: Vec<u32> = self.pending_acks.drain(..n).collect();
            let m = aurora_msg::msgs::PacketAck {
                packets: ids.into_iter().map(|id| aurora_msg::msgs::packet_ack::Packets { id }).collect(),
            };
            let mut w = Writer::new();
            aurora_msg::msgs::PacketAck::ID.encode(&mut w.buf);
            m.encode_body(&mut w);
            let seq = self.next_seq;
            self.next_seq = self.next_seq.wrapping_add(1);
            let mut buf = std::mem::take(&mut self.buf);
            build_raw(&w.buf, false, seq, false, false, &[], &mut buf);
            crate::probe::message_out(&w.buf, buf.len());
            self.buf = buf;
            self.raw_send(sock, stats);
        }
    }

    /// Start a ping; returns the ping id and the oldest unacked sequence.
    pub fn next_ping(&mut self) -> (u8, u32) {
        self.ping_id = self.ping_id.wrapping_add(1);
        let now = Instant::now();
        self.ping_sent = Some((self.ping_id, now));
        self.ping_wait_started.get_or_insert(now);
        let oldest = self.unacked.keys().min().copied().unwrap_or(self.next_seq);
        (self.ping_id, oldest)
    }

    pub fn on_ping_reply(&mut self, id: u8) -> Option<Duration> {
        if let Some((pid, at)) = self.ping_sent
            && pid == id
        {
            let rtt = at.elapsed();
            // Smooth like TCP's SRTT.
            self.rtt = (self.rtt * 7 + rtt) / 8;
            self.ping_sent = None;
            self.ping_wait_started = None;
            return Some(rtt);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELIABLE: u8 = PacketFlags::RELIABLE;
    const RESENT: u8 = PacketFlags::RELIABLE | PacketFlags::RESENT;

    fn circuit() -> Circuit {
        Circuit::new("127.0.0.1:9".parse().expect("address"))
    }

    #[test]
    fn undecodable_packet_is_not_acked_and_its_resend_is_processed() {
        let mut c = circuit();
        // first copy: received but its decode failed, so never accepted
        assert!(c.on_receive(RELIABLE, 7, &[]));
        assert!(c.pending_acks.is_empty(), "acked before being processed");
        // the simulator resends it: not a duplicate, processed this time
        assert!(c.on_receive(RESENT, 7, &[]));
        c.accept(RESENT, 7);
        assert_eq!(c.pending_acks, vec![7]);
    }

    #[test]
    fn duplicate_resend_is_acked_but_not_processed_again() {
        let mut c = circuit();
        assert!(c.on_receive(RELIABLE, 3, &[]));
        c.accept(RELIABLE, 3);
        // our ack was lost: the resend is acked again, not reprocessed
        assert!(!c.on_receive(RESENT, 3, &[]));
        assert_eq!(c.pending_acks, vec![3, 3]);
        // without RESENT a reused sequence number is a new packet
        assert!(c.on_receive(RELIABLE, 3, &[]));
    }

    #[test]
    fn sequence_numbers_count_gaps_and_late_packets() {
        let stats = NetStats::default();
        let mut c = circuit();
        let seen = |stats: &NetStats| (stats.seq_gaps.load(Ordering::Relaxed), stats.seq_late.load(Ordering::Relaxed));
        // the first packet is where the count starts, whatever its number
        c.note_sequence(10, false, &stats);
        c.note_sequence(11, false, &stats);
        assert_eq!(seen(&stats), (0, 0));
        // 12 and 13 are missing, then 12 arrives (resent): one still missing
        c.note_sequence(14, false, &stats);
        c.note_sequence(12, true, &stats);
        assert_eq!(seen(&stats), (2, 1));
        // the same number twice is neither
        c.note_sequence(14, false, &stats);
        assert_eq!(seen(&stats), (2, 1));
        // the simulator starts its numbers over: not a late packet
        c.note_sequence(500_000, false, &stats);
        let gaps = stats.seq_gaps.load(Ordering::Relaxed);
        c.note_sequence(1, false, &stats);
        c.note_sequence(2, false, &stats);
        assert_eq!(seen(&stats), (gaps, 1));
    }

    #[test]
    fn unreliable_packets_are_never_acked() {
        let mut c = circuit();
        assert!(c.on_receive(0, 1, &[]));
        c.accept(0, 1);
        assert!(c.pending_acks.is_empty());
    }

    #[tokio::test]
    async fn packet_loss_circuit_tick_confirms_a_loss_without_more_datagrams() {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let stats = NetStats::default();
        let mut c = circuit();
        let before = Instant::now() - Duration::from_secs(10);
        c.packet_loss.receive(1, false, before);
        c.packet_loss.receive(3, false, before);
        assert_eq!(stats.snapshot().packets_lost, 0);
        c.tick(&sock, &stats);
        assert_eq!(stats.snapshot().packets_lost, 1);
        c.tick(&sock, &stats);
        assert_eq!(stats.snapshot().packets_lost, 1);
    }

    #[tokio::test]
    async fn packet_loss_an_unanswered_ping_extends_the_grace_until_a_reply() {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let stats = NetStats::default();
        let mut c = circuit();
        c.rtt = Duration::from_millis(10);
        let before = Instant::now() - Duration::from_secs(2);
        c.packet_loss.receive(1, false, before);
        c.packet_loss.receive(3, false, before);
        c.ping_wait_started = Some(before);
        let (id, _) = c.next_ping();
        assert_eq!(c.ping_wait_started, Some(before), "a newer ping must not reset the wait");
        c.tick(&sock, &stats);
        assert_eq!(stats.snapshot().packets_lost, 0);
        assert!(c.on_ping_reply(id).is_some());
        assert!(c.ping_wait_started.is_none());
        c.tick(&sock, &stats);
        assert_eq!(stats.snapshot().packets_lost, 1);
    }

    #[tokio::test]
    async fn login_use_circuit_code_keeps_its_own_resend_policy() {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let stats = NetStats::default();
        let mut c = circuit();
        let m = aurora_msg::msgs::UseCircuitCode::default();
        let login = c.send_reliable_with(&sock, &stats, &m, Resend::LOGIN_USE_CIRCUIT_CODE);
        let other = c.send(&sock, &stats, &m, true);
        // 4 s later: past the ping-based timeout, not yet the 5 s one
        for u in c.unacked.values_mut() {
            u.sent -= Duration::from_secs(4);
        }
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&login].retries, 0);
        assert_eq!(c.unacked[&other].retries, 1);
        // 3 resends 5 s apart, then given up on and reported
        for _ in 0..4 {
            c.unacked.get_mut(&login).expect("unacked").sent -= Duration::from_secs(6);
            c.tick(&sock, &stats);
        }
        assert!(!c.is_unacked(login));
        assert_eq!(c.take_dropped(), vec![login]);
        assert!(c.take_dropped().is_empty());
    }

    #[test]
    fn reliable_timeout_matches_the_legacy_formula() {
        assert_eq!(reliable_timeout(Duration::from_millis(1), None), Duration::from_secs(1));
        assert_eq!(reliable_timeout(Duration::from_millis(400), None), Duration::from_secs(2));
        assert_eq!(reliable_timeout(Duration::from_secs(1), None), Duration::from_secs(5));
        // The averaged ping stops at 2 s, so five times that is 10 s.
        assert_eq!(reliable_timeout(Duration::from_secs(3), None), Duration::from_secs(10));
        assert_eq!(
            reliable_timeout(Duration::from_millis(1), Some(Duration::from_millis(400))),
            Duration::from_secs(2)
        );
        // An unanswered ping is also capped at 2 s.
        assert_eq!(
            reliable_timeout(Duration::from_millis(1), Some(Duration::from_secs(9))),
            Duration::from_secs(10)
        );
    }

    fn rewind(c: &mut Circuit, age: Duration) {
        let sent = Instant::now() - age;
        for packet in c.unacked.values_mut() {
            packet.sent = sent;
        }
    }

    async fn bound() -> UdpSocket {
        UdpSocket::bind("127.0.0.1:0").await.expect("bind")
    }

    fn one_reliable(c: &mut Circuit, sock: &UdpSocket, stats: &NetStats) -> u32 {
        let message = aurora_msg::msgs::UseCircuitCode::default();
        c.send(sock, stats, &message, true)
    }

    /// A 1 ms ping used to resend at 0.5 s (`rtt * 3` clamped up to 500 ms).
    /// The wait is at least 1 s, and a resend starts that wait again.
    #[tokio::test]
    async fn lan_ping_does_not_resend_a_reliable_packet_before_one_second() {
        let sock = bound().await;
        let stats = NetStats::default();
        let mut c = circuit();
        c.rtt = Duration::from_millis(1);
        let seq = one_reliable(&mut c, &sock, &stats);
        rewind(&mut c, Duration::from_millis(600));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 0, "a 1 ms ping still waits out the 1 s minimum");
        rewind(&mut c, Duration::from_millis(1_001));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 1);
        // Same moment: the resend's clock was just started.
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 1, "a resend starts its wait again");
        rewind(&mut c, Duration::from_millis(600));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 1);
    }

    /// 400 ms × 5 = 2 s. 1 s × 5 = 5 s, past the old 3 s cap.
    /// A ping above 2 s is capped there first, so the wait is 10 s.
    #[tokio::test]
    async fn averaged_ping_sets_the_reliable_resend_wait() {
        let sock = bound().await;
        let stats = NetStats::default();

        let mut c = circuit();
        c.rtt = Duration::from_millis(400);
        let seq = one_reliable(&mut c, &sock, &stats);
        rewind(&mut c, Duration::from_millis(1_999));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 0, "a 400 ms average waits 2 s");
        rewind(&mut c, Duration::from_millis(2_001));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 1);

        let mut c = circuit();
        c.rtt = Duration::from_secs(1);
        let seq = one_reliable(&mut c, &sock, &stats);
        rewind(&mut c, Duration::from_millis(3_001));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 0, "a 1 s average is not cut at 3 s");
        rewind(&mut c, Duration::from_millis(4_999));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 0);
        rewind(&mut c, Duration::from_millis(5_001));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 1);

        let mut c = circuit();
        c.rtt = Duration::from_secs(3);
        let seq = one_reliable(&mut c, &sock, &stats);
        rewind(&mut c, Duration::from_millis(5_001));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 0, "an averaged ping stops at 2 s, so this waits 10 s");
        rewind(&mut c, Duration::from_millis(10_001));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 1);
    }

    /// `getPingDelayAveraged` is at least the time the oldest ping has been out.
    #[tokio::test]
    async fn an_unanswered_ping_lengthens_the_reliable_resend_wait() {
        let sock = bound().await;
        let stats = NetStats::default();
        let mut c = circuit();
        c.rtt = Duration::from_millis(1);
        let seq = one_reliable(&mut c, &sock, &stats);
        // 400 ms unanswered → 5 × 400 ms = 2 s, above the 1 s floor.
        c.ping_wait_started = Some(Instant::now() - Duration::from_millis(400));
        rewind(&mut c, Duration::from_millis(1_500));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 0, "an unanswered ping raises the wait above 1 s");
        c.ping_wait_started = Some(Instant::now() - Duration::from_millis(400));
        rewind(&mut c, Duration::from_millis(2_500));
        c.tick(&sock, &stats);
        assert_eq!(c.unacked[&seq].retries, 1);
    }

    /// `LL_DEFAULT_RELIABLE_RETRIES` is 3: three resends, then one more wait, then give up.
    #[tokio::test]
    async fn three_resends_then_the_reliable_packet_is_given_up() {
        let sock = bound().await;
        let stats = NetStats::default();
        let mut c = circuit();
        c.rtt = Duration::from_millis(1);
        let seq = one_reliable(&mut c, &sock, &stats);
        for n in 1..=3 {
            rewind(&mut c, Duration::from_millis(1_001));
            c.tick(&sock, &stats);
            assert_eq!(c.unacked[&seq].retries, n);
        }
        rewind(&mut c, Duration::from_millis(1_001));
        c.tick(&sock, &stats);
        assert!(!c.is_unacked(seq), "given up after the third resend's wait");
        assert_eq!(c.take_dropped(), vec![seq]);
        assert_eq!(stats.resent.load(Ordering::Relaxed), 3);
    }
}
