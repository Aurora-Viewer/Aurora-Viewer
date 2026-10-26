//! GPU allocations that can fail, and wgpu errors that must not flood the
//! log.
//!
//! When video memory runs out (other programs on the card, several viewers
//! at once) wgpu does not refuse `create_buffer`: it hands back an invalid
//! buffer, and the failure itself may never reach the error handler (a
//! failed allocation the driver reports as unexpected is filed as a lost
//! device, which wgpu keeps silent). Installed in place of a working buffer,
//! that invalid one failed every write of every frame after it: millions
//! of "Buffer with 'vertex arena' label is invalid" a minute, and nothing
//! drawn. The growth sites go through [`try_buffer`] / [`try_texture`]
//! instead: on failure they keep the buffer they had, say so once, and try
//! again later ([`Backoff`]) or smaller.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A buffer, or why it could not be made (the device is out of memory, or
/// the buffer came back invalid).
pub fn try_buffer(device: &wgpu::Device, desc: &wgpu::BufferDescriptor) -> Result<wgpu::Buffer, String> {
    if let Some(e) = injected_failure(desc.size) {
        return Err(e);
    }
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let buffer = device.create_buffer(desc);
    probe(device, &buffer);
    let invalid = pollster::block_on(invalid.pop());
    let oom = pollster::block_on(oom.pop());
    match invalid.or(oom) {
        None => Ok(buffer),
        Some(e) => Err(reason(&e.to_string())),
    }
}

/// An invalid buffer only shows when it is used: a clear of its first bytes
/// is recorded into an encoder that is finished and dropped, never
/// submitted (nothing reaches the GPU), for the error scope around it to
/// catch a "Buffer … is invalid" (`invalid_buffers_are_seen_and_valid_ones_untouched`).
/// Buffers without COPY_DST are not checked (every growing buffer has it).
fn probe(device: &wgpu::Device, buffer: &wgpu::Buffer) {
    if buffer.usage().contains(wgpu::BufferUsages::COPY_DST) && buffer.size() >= wgpu::COPY_BUFFER_ALIGNMENT {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("allocation check"),
        });
        enc.clear_buffer(buffer, 0, Some(wgpu::COPY_BUFFER_ALIGNMENT));
        drop(enc.finish());
    }
}

/// A texture and its view, or why they could not be made.
pub fn try_texture(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor,
    view: &wgpu::TextureViewDescriptor,
    bytes: u64,
) -> Result<(wgpu::Texture, wgpu::TextureView), String> {
    if let Some(e) = injected_failure(bytes) {
        return Err(e);
    }
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // in memory of its own when it can (dedicated.rs), else carved out of
    // wgpu's blocks, as when the driver refuses it there (the blocks may
    // still have room)
    let texture = match crate::dedicated::texture(device, desc) {
        Some(Ok(texture)) => texture,
        Some(Err(e)) => {
            log::debug!("texture '{}' in wgpu's memory blocks: {e}", desc.label.unwrap_or("?"));
            device.create_texture(desc)
        }
        None => device.create_texture(desc),
    };
    // (an invalid texture makes its view fail validation)
    let view = texture.create_view(view);
    let invalid = pollster::block_on(invalid.pop());
    let oom = pollster::block_on(oom.pop());
    match invalid.or(oom) {
        None => Ok((texture, view)),
        Some(e) => Err(reason(&e.to_string())),
    }
}

/// The error's reason (wgpu's text spreads over several lines, the last
/// one says most).
fn reason(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.last().map_or_else(|| text.trim().to_owned(), |l| (*l).to_owned())
}

/// `AURORA_GPU_OOM=<MB>`: every allocation of at least that size fails as
/// if video memory were full (testing; 0 fails them all).
fn injected_failure(bytes: u64) -> Option<String> {
    static LIMIT: OnceLock<Option<u64>> = OnceLock::new();
    let limit = *LIMIT.get_or_init(|| {
        std::env::var("AURORA_GPU_OOM")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|mb| mb << 20)
    });
    limit
        .filter(|l| bytes >= *l)
        .map(|_| "out of video memory (AURORA_GPU_OOM)".to_owned())
}

/// When to try an allocation again after it failed: 2 s, then twice as
/// long each time up to 30 s, back to nothing after a success. Memory frees
/// up when another program closes; trying every frame meanwhile would cost
/// a failed allocation per frame.
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    next: Option<Instant>,
    wait: Duration,
}

impl Backoff {
    const FIRST: Duration = Duration::from_secs(2);
    const LONGEST: Duration = Duration::from_secs(30);

    pub fn ready(&self, now: Instant) -> bool {
        self.next.is_none_or(|t| now >= t)
    }

    pub fn failed(&mut self, now: Instant) {
        self.wait = if self.wait.is_zero() {
            Self::FIRST
        } else {
            (self.wait * 2).min(Self::LONGEST)
        };
        self.next = Some(now + self.wait);
    }

    pub fn succeeded(&mut self) {
        *self = Self::default();
    }

    /// A failure is being waited out.
    pub fn waiting(&self) -> bool {
        self.next.is_some()
    }
}

/// Repeated log lines kept to one: the first time a message is seen it is
/// written, then at most once per period with how many times it came back.
#[derive(Debug)]
pub struct LogThrottle {
    period: Duration,
    seen: HashMap<String, (Instant, u64)>,
}

impl LogThrottle {
    pub fn new(period: Duration) -> Self {
        Self {
            period,
            seen: HashMap::new(),
        }
    }

    /// What to write for `message` now: the message itself the first time,
    /// a repeat line once the period is over, nothing in between.
    pub fn note(&mut self, message: &str, now: Instant) -> Option<String> {
        match self.seen.get_mut(message) {
            None => {
                // (a few kinds at most: forget the oldest beyond that)
                if self.seen.len() >= 64
                    && let Some(old) = self.seen.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone())
                {
                    self.seen.remove(&old);
                }
                self.seen.insert(message.to_owned(), (now, 0));
                Some(message.to_owned())
            }
            Some((since, count)) => {
                *count += 1;
                if now.saturating_duration_since(*since) < self.period {
                    return None;
                }
                let line = format!(
                    "{message} (repeated {} times in the last {} s)",
                    count,
                    now.saturating_duration_since(*since).as_secs()
                );
                *since = now;
                *count = 0;
                Some(line)
            }
        }
    }
}

/// Capacities to try, largest first, for a growth from `old` elements that
/// wanted `cap` and needs at least `old + need`: `cap`, then halfway back
/// towards the minimum each time, then the minimum itself (rounded up to
/// `align` elements).
pub fn fallback_capacities(old: u32, need: u32, cap: u32, align: u32) -> Vec<u32> {
    let align = align.max(1);
    let min = old.saturating_add(need).div_ceil(align).saturating_mul(align);
    let mut out = Vec::new();
    if cap < min {
        return out;
    }
    let mut c = cap;
    while c > min {
        out.push(c);
        let next = (old + (c - old) / 2).div_ceil(align).saturating_mul(align);
        if next >= c {
            break;
        }
        c = next;
    }
    out.push(min);
    out
}

/// Bytes a growth ahead of need may take: what the video memory budget
/// leaves free, less a margin for everything else this frame and the next
/// ones need (a tenth of the budget, at least 512 MB).
pub fn ahead_allowance(budget: u64, usage: u64) -> u64 {
    let margin = (budget / 10).max(512 << 20);
    budget.saturating_sub(usage).saturating_sub(margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_message_is_written_once_then_counted() {
        let t0 = Instant::now();
        let mut log = LogThrottle::new(Duration::from_secs(10));
        assert_eq!(log.note("buffer invalid", t0).as_deref(), Some("buffer invalid"));
        for i in 1..=4123u64 {
            assert_eq!(log.note("buffer invalid", t0 + Duration::from_millis(i)), None);
        }
        // another message is not held back by the first
        assert_eq!(log.note("texture invalid", t0).as_deref(), Some("texture invalid"));
        let line = log.note("buffer invalid", t0 + Duration::from_secs(10)).expect("repeat line");
        assert_eq!(line, "buffer invalid (repeated 4124 times in the last 10 s)");
        assert_eq!(log.note("buffer invalid", t0 + Duration::from_secs(11)), None);
    }

    #[test]
    fn retries_wait_longer_after_each_failure_and_reset_on_success() {
        let t0 = Instant::now();
        let mut b = Backoff::default();
        assert!(b.ready(t0) && !b.waiting());
        b.failed(t0);
        assert!(!b.ready(t0 + Duration::from_millis(1999)));
        assert!(b.ready(t0 + Duration::from_secs(2)));
        b.failed(t0);
        assert!(!b.ready(t0 + Duration::from_secs(3)));
        assert!(b.ready(t0 + Duration::from_secs(4)));
        for _ in 0..10 {
            b.failed(t0);
        }
        assert!(b.ready(t0 + Duration::from_secs(30)));
        b.succeeded();
        assert!(b.ready(t0) && !b.waiting());
    }

    #[test]
    fn a_failed_growth_tries_halfway_sizes_down_to_what_is_needed() {
        // 2 M elements, doubled to 4 M, a mesh of 100 k
        let sizes = fallback_capacities(2_000_000, 100_000, 4_000_000, 1);
        assert_eq!(sizes.first(), Some(&4_000_000));
        assert_eq!(sizes.last(), Some(&2_100_000));
        assert!(sizes.windows(2).all(|w| w[0] > w[1]));
        assert!(sizes.len() <= 6, "{sizes:?}");
        // even index counts stay even
        assert!(fallback_capacities(10, 3, 40, 2).iter().all(|c| c % 2 == 0));
        assert_eq!(fallback_capacities(10, 3, 40, 2).last(), Some(&14));
        // nothing when even the wish does not hold the need
        assert!(fallback_capacities(100, 50, 120, 1).is_empty());
    }

    #[test]
    fn growth_ahead_keeps_a_margin_of_the_budget() {
        const MB: u64 = 1 << 20;
        // 23.4 GB budget, 21 GB used: 2.37 GB free less a 2.34 GB margin
        assert_eq!(ahead_allowance(23_370 * MB, 21_000 * MB), 33 * MB);
        assert_eq!(ahead_allowance(23_370 * MB, 22_000 * MB), 0);
        // half used: the free part less a tenth of the budget
        assert_eq!(ahead_allowance(20_000 * MB, 10_000 * MB), 8_000 * MB);
        // a small card keeps at least 512 MB
        assert_eq!(ahead_allowance(2_000 * MB, 1_000 * MB), 488 * MB);
        assert_eq!(ahead_allowance(1_000 * MB, 2_000 * MB), 0);
    }

    /// A buffer the device cannot make comes back as an error, not as an
    /// invalid buffer (and nothing reaches the uncaptured error handler).
    /// Needs a GPU: `cargo test --workspace refused_buffer -- --ignored`.
    #[test]
    #[ignore]
    fn a_refused_buffer_is_an_error_not_an_invalid_buffer() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let uncaught = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let seen = uncaught.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |_| {
            seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        let desc = |size| wgpu::BufferDescriptor {
            label: Some("probe"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        assert!(try_buffer(&device, &desc(1 << 20)).is_ok());
        // larger than the device allows: refused
        let too_big = device.limits().max_buffer_size.saturating_mul(2);
        let err = try_buffer(&device, &desc(too_big)).expect_err("refused");
        assert!(!err.is_empty());
        assert_eq!(uncaught.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    /// The probe of `try_buffer`: a buffer whose creation failed without a
    /// word (its error swallowed here, as wgpu does when it files a failed
    /// allocation as a lost device) is caught by the probe alone; a valid
    /// buffer is left as it was (the probe's clear never reaches the GPU)
    /// and the probe costs microseconds. Needs a GPU:
    /// `cargo test --workspace invalid_buffers_are_seen -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn invalid_buffers_are_seen_and_valid_ones_untouched() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let desc = |size| wgpu::BufferDescriptor {
            label: Some("probe"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        };
        let probed = |buffer: &wgpu::Buffer| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            probe(&device, buffer);
            pollster::block_on(scope.pop())
        };
        // an invalid buffer whose creation error nobody saw
        let silent = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let bad = device.create_buffer(&desc(device.limits().max_buffer_size.saturating_mul(2)));
        assert!(pollster::block_on(silent.pop()).is_some(), "the creation did fail");
        let seen = probed(&bad).expect("the probe sees the invalid buffer");
        assert!(reason(&seen.to_string()).contains("invalid"), "{seen}");
        // a valid buffer: no error, and its content stays
        let good = device.create_buffer(&desc(256));
        queue.write_buffer(&good, 0, &[7u8; 16]);
        queue.submit([]);
        let t0 = Instant::now();
        for _ in 0..1000 {
            assert!(probed(&good).is_none());
        }
        let per_probe = t0.elapsed() / 1000;
        eprintln!("probe of a valid buffer: {per_probe:?}");
        assert!(per_probe < Duration::from_millis(1), "{per_probe:?}");
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read back"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_buffer_to_buffer(&good, 0, &read, 0, 16);
        queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let bytes = read.slice(..).get_mapped_range().expect("mapped");
        assert_eq!(&bytes[..], &[7u8; 16], "the probe's clear never ran");
    }

    #[test]
    fn the_reason_of_a_wgpu_error_is_its_last_line() {
        let text = "Validation Error\n\nCaused by:\n  In Queue::write_buffer\n    Buffer with 'vertex arena' label is invalid\n";
        assert_eq!(reason(text), "Buffer with 'vertex arena' label is invalid");
        assert_eq!(reason("Out of Memory"), "Out of Memory");
    }
}
