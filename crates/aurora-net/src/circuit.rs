//! Reliable UDP circuit bookkeeping (port of the essentials of
//! `llmessage/llcircuit.cpp`): sequence numbers, resends, acks, dedupe.

use crate::stats::NetStats;
use aurora_msg::packet::{PacketFlags, build_raw};
use aurora_msg::{Msg, Writer};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const MAX_RESENDS: u8 = 4;
const DEDUPE_WINDOW: usize = 2048;
const ACK_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

struct Unacked {
    payload: Vec<u8>,
    zerocoded: bool,
    sent: Instant,
    retries: u8,
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
    buf: Vec<u8>,
    /// Reliable packets given up on since the last check.
    pub dropped: Vec<u32>,
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
            buf: Vec::with_capacity(1500),
            dropped: Vec::new(),
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

    fn send_payload(&mut self, sock: &UdpSocket, stats: &NetStats, payload: Vec<u8>, zerocoded: bool, reliable: bool) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let mut buf = std::mem::take(&mut self.buf);
        let n = build_raw(&payload, zerocoded, seq, reliable, false, &self.pending_acks, &mut buf);
        self.pending_acks.drain(..n);
        self.buf = buf;
        self.raw_send(sock, stats);
        if reliable {
            self.unacked.insert(
                seq,
                Unacked {
                    payload,
                    zerocoded,
                    sent: Instant::now(),
                    retries: 0,
                },
            );
        }
    }

    /// Process the header of an incoming packet. Returns `false` if it is a
    /// duplicate that must not be processed again.
    pub fn on_receive(&mut self, flags: u8, seq: u32, acks: &[u32]) -> bool {
        self.last_recv = Instant::now();
        for a in acks {
            self.unacked.remove(a);
        }
        if flags & PacketFlags::RELIABLE != 0 {
            self.pending_acks.push(seq);
        }
        // Like LL (message.cpp), only packets flagged RESENT can be duplicates;
        // this also survives a simulator resetting its sequence numbers.
        if flags & PacketFlags::RESENT != 0 && self.recent_set.contains(&seq) {
            return false;
        }
        if self.recent.len() >= DEDUPE_WINDOW
            && let Some(old) = self.recent.pop_front()
        {
            self.recent_set.remove(&old);
        }
        self.recent.push_back(seq);
        self.recent_set.insert(seq);
        true
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
        let timeout = (self.rtt * 3).clamp(Duration::from_millis(500), Duration::from_secs(3));
        let mut to_resend = Vec::new();
        let mut to_drop = Vec::new();
        for (seq, u) in &self.unacked {
            if now.duration_since(u.sent) > timeout {
                if u.retries >= MAX_RESENDS {
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
            u.sent = now;
            let mut buf = std::mem::take(&mut self.buf);
            build_raw(&u.payload, u.zerocoded, seq, true, true, &[], &mut buf);
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
            self.buf = buf;
            self.raw_send(sock, stats);
        }
    }

    /// Start a ping; returns the ping id and the oldest unacked sequence.
    pub fn next_ping(&mut self) -> (u8, u32) {
        self.ping_id = self.ping_id.wrapping_add(1);
        self.ping_sent = Some((self.ping_id, Instant::now()));
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
            return Some(rtt);
        }
        None
    }
}
