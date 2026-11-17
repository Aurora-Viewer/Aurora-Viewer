//! The server's bounds. The Legacy Viewer has none of these but the
//! handshake's 30 seconds (llscripteditorws.cpp, HANDSHAKE_TIMEOUT) and the
//! 120 seconds of a call (lljsonrpcws.h, REQUEST_TIMEOUT_SECONDS): its
//! frames go up to websocketpp's 32 MB, it takes any number of clients and
//! messages. Stricter here, as a server open on the user's computer must be.

use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Limits {
    /// Largest message (and frame) a client may send. The core sync's
    /// messages are a few hundred bytes.
    pub max_message: usize,
    /// Clients that have proven themselves, at once.
    pub max_clients: usize,
    /// Connections still in their handshake, at once.
    pub max_pending: usize,
    /// For the HTTP upgrade itself (a client that sends its request slowly).
    pub upgrade_timeout: Duration,
    /// For the answer to `session.handshake` (Legacy: 30 s).
    pub handshake_timeout: Duration,
    /// A WebSocket ping this often.
    pub ping_interval: Duration,
    /// Closed after this long without a single frame from the client.
    pub idle_timeout: Duration,
    /// Messages a client may send per second, and in a burst.
    pub rate: f64,
    pub burst: f64,
    /// Messages refused for the rate, within [`STRIKE_WINDOW`], before the
    /// client is closed.
    pub strikes: u32,
    /// Messages waiting to be written to one client; a client that does not
    /// read is closed rather than kept in memory.
    pub outgoing_queue: usize,
    /// The bytes of those messages: a few syntax files (about 2 MB each).
    pub outgoing_bytes: usize,
}

/// Rate refusals older than this are forgiven: a long-lived editor that
/// goes over the rate now and then is not closed for it.
pub const STRIKE_WINDOW: Duration = Duration::from_secs(10);

/// The rate refusals of one connection, counted within [`STRIKE_WINDOW`].
#[derive(Debug, Clone)]
pub struct Strikes {
    count: u32,
    since: Instant,
}

impl Strikes {
    pub fn new(now: Instant) -> Strikes {
        Strikes { count: 0, since: now }
    }

    /// One more refusal: how many in the current window.
    pub fn add(&mut self, now: Instant) -> u32 {
        if now.saturating_duration_since(self.since) > STRIKE_WINDOW {
            self.count = 0;
            self.since = now;
        }
        self.count += 1;
        self.count
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_message: 64 * 1024,
            max_clients: 4,
            max_pending: 4,
            upgrade_timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(75),
            rate: 20.0,
            burst: 40.0,
            strikes: 40,
            outgoing_queue: 256,
            outgoing_bytes: 16 * 1024 * 1024,
        }
    }
}

/// A token bucket: `rate` tokens a second, at most `burst`.
#[derive(Debug, Clone)]
pub struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    last: Instant,
}

impl Bucket {
    pub fn new(rate: f64, burst: f64, now: Instant) -> Bucket {
        Bucket {
            tokens: burst,
            rate,
            burst,
            last: now,
        }
    }

    /// Takes a token if there is one.
    pub fn take(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_then_the_rate() {
        let start = Instant::now();
        let mut bucket = Bucket::new(20.0, 40.0, start);
        assert_eq!((0..100).filter(|_| bucket.take(start)).count(), 40);
        // half a second later: ten more
        let later = start + Duration::from_millis(500);
        assert_eq!((0..100).filter(|_| bucket.take(later)).count(), 10);
        // an hour of silence refills the burst, not more
        let much_later = later + Duration::from_secs(3600);
        assert_eq!((0..100).filter(|_| bucket.take(much_later)).count(), 40);
        // a clock that goes back never gives tokens
        assert!(!bucket.take(start));
    }

    #[test]
    fn strikes_are_counted_within_their_window() {
        let start = Instant::now();
        let mut strikes = Strikes::new(start);
        assert_eq!(strikes.add(start), 1);
        assert_eq!(strikes.add(start + Duration::from_secs(5)), 2);
        assert_eq!(strikes.add(start + STRIKE_WINDOW), 3);
        // past the window: forgiven
        let later = start + STRIKE_WINDOW + Duration::from_secs(1);
        assert_eq!(strikes.add(later), 1);
        assert_eq!(strikes.add(later), 2);
        // a clock that goes back never resets them
        assert_eq!(strikes.add(start), 3);
    }
}
