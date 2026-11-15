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
use std::sync::{Mutex, OnceLock};
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
    integrated: bool,
) -> Result<(wgpu::Texture, wgpu::TextureView), String> {
    if let Some(e) = injected_failure(bytes) {
        return Err(e);
    }
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // in memory of its own when it can (dedicated.rs), else carved out of
    // wgpu's blocks, as when the driver refuses it there (the blocks may
    // still have room)
    let texture = match crate::dedicated::texture(device, desc, integrated) {
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

/// `DeviceExt::create_buffer_init` without its panic: wgpu's helper maps the
/// new buffer and `expect`s the mapping, so a buffer the full card refused
/// ended the viewer ("night sky stars" at start with the card full, "water
/// effects" on a lost device, #1892). Here a refused buffer comes back
/// invalid and unfilled, like any other object the memory refused, and the
/// error scopes around it see the failure.
pub fn buffer_init(device: &wgpu::Device, label: &str, contents: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let align = wgpu::COPY_BUFFER_ALIGNMENT;
    let size = (contents.len() as u64).div_ceil(align).max(1) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: if contents.is_empty() { 0 } else { size },
        usage,
        mapped_at_creation: !contents.is_empty(),
    });
    if contents.is_empty() {
        return buffer;
    }
    match buffer.get_mapped_range_mut(..) {
        Ok(mut view) => {
            view.slice(..contents.len()).copy_from_slice(contents);
            drop(view);
            buffer.unmap();
        }
        Err(e) => log::warn!("{label}: buffer refused ({})", reason(&e.to_string())),
    }
    buffer
}

/// Runs `make`, which makes GPU objects with no check of their own (a set
/// of render targets and what binds them), and says whether the memory
/// refused them. Whatever came back invalid is `make`'s to drop: unlike
/// [`try_texture`] there is nothing to fall back on but trying again.
pub fn scoped<T>(device: &wgpu::Device, make: impl FnOnce() -> T) -> (T, bool) {
    let (made, oom, invalid) = scoped_apart(device, make);
    (made, oom || invalid)
}

/// [`scoped`], with the two signs of a refusal apart: (made, the memory
/// said so, an object came back invalid).
pub fn scoped_apart<T>(device: &wgpu::Device, make: impl FnOnce() -> T) -> (T, bool, bool) {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = make();
    let invalid = pollster::block_on(invalid.pop());
    let oom = pollster::block_on(oom.pop());
    let is_invalid = memory_refused(None, invalid.as_ref().map(ToString::to_string).as_deref());
    // said as outside a scope, refused or not
    for e in oom.iter().chain(&invalid) {
        say(e);
    }
    (made, oom.is_some(), is_invalid)
}

/// An error a scope caught, written as the device's handler writes the
/// others (main_thread.rs): the crash report's `GPU errors` row reads these
/// lines, and an out-of-memory names its call there with its sources
/// (#2482). Throttled the same way: a window dragged while the targets are
/// refused tries at every step.
fn say(error: &wgpu::Error) {
    static THROTTLE: OnceLock<Mutex<LogThrottle>> = OnceLock::new();
    let message = format!("wgpu error: {}", crate::main_thread::error_text(error));
    let line = THROTTLE
        .get_or_init(|| Mutex::new(LogThrottle::new(Duration::from_secs(10))))
        .lock()
        .ok()
        .and_then(|mut t| t.note(&message, Instant::now()));
    if let Some(line) = line {
        log::error!("{line}");
    }
}

/// Whether a scope's errors mean the memory ran out: wgpu said so, or an
/// object came back invalid, which is all that shows of a failed allocation
/// the driver reports as unexpected (see the module). Any other validation
/// error is a bug of ours and retrying would not cure it.
fn memory_refused(oom: Option<&str>, invalid: Option<&str>) -> bool {
    oom.is_some() || invalid.is_some_and(|e| e.contains("is invalid"))
}

/// The error's reason (wgpu's text spreads over several lines, the last
/// one says most).
fn reason(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.last().map_or_else(|| text.trim().to_owned(), |l| (*l).to_owned())
}

/// `AURORA_GPU_OOM=<MB>[,<seconds>]`: every allocation of at least that
/// size fails as if video memory were full (testing; 0 fails them all),
/// for the whole run or, with a duration, that long after the first
/// allocation checked: the memory then comes back, as when another program
/// that held it closes.
fn injected_failure(bytes: u64) -> Option<String> {
    static LIMIT: OnceLock<Option<InjectedOom>> = OnceLock::new();
    let limit = *LIMIT.get_or_init(|| std::env::var("AURORA_GPU_OOM").ok().and_then(|v| parse_oom(&v, Instant::now())));
    limit
        .filter(|oom| bytes >= oom.bytes && oom.active(Instant::now()))
        .map(|_| "out of video memory (AURORA_GPU_OOM)".to_owned())
}

/// `AURORA_GPU_OOM`, read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InjectedOom {
    bytes: u64,
    /// From when (the viewer's start otherwise).
    from: Option<Instant>,
    /// Until when (the whole run otherwise).
    until: Option<Instant>,
}

impl InjectedOom {
    fn active(&self, now: Instant) -> bool {
        self.from.is_none_or(|t| now >= t) && self.until.is_none_or(|t| now < t)
    }
}

/// `<MB>[,<seconds>[,<after seconds>]]` of `AURORA_GPU_OOM`, from `now`
/// (the first allocation): the size, for how long, and after how long it
/// starts. A start after the first frames lets the reserved textures and
/// the first pages be made, as on a card another program fills later.
fn parse_oom(value: &str, now: Instant) -> Option<InjectedOom> {
    let mut parts = value.split(',').map(str::trim);
    let mb = parts.next()?.parse::<u64>().ok()?;
    let seconds = |p: Option<&str>| -> Option<Option<Duration>> {
        match p {
            Some(s) => s.parse().ok().map(|s| Some(Duration::from_secs(s))),
            None => Some(None),
        }
    };
    let length = seconds(parts.next())?;
    let after = seconds(parts.next())?;
    if parts.next().is_some() {
        return None;
    }
    let from = after.map(|d| now + d);
    let start = from.unwrap_or(now);
    Some(InjectedOom {
        bytes: mb << 20,
        from,
        until: length.map(|d| start + d),
    })
}

/// `AURORA_GPU_OOM_TARGETS=<seconds>`: for that long after the first frame
/// target made, the render targets come back invalid as if video memory
/// were full (testing). They are made with no usage, which wgpu refuses:
/// the same invalid texture a full card leaves, in a real device, without
/// filling the card.
pub fn targets_refused() -> bool {
    static UNTIL: OnceLock<Option<Instant>> = OnceLock::new();
    let until = *UNTIL.get_or_init(|| {
        std::env::var("AURORA_GPU_OOM_TARGETS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|seconds| Instant::now() + Duration::from_secs(seconds))
    });
    until.is_some_and(|t| Instant::now() < t)
}

/// `AURORA_GPU_OOM_FIRST_USE=<seconds>`: for that long after the first
/// target made on first use (the selection outline, the depth of field, a
/// photo look, RLVa's spheres, the rigged blend depth…), every such target
/// counts as refused by a full video memory (testing): the target is made,
/// then let go as a refused one would be, which tests what follows without
/// filling the card.
pub fn first_use_refused() -> bool {
    static UNTIL: OnceLock<Option<Instant>> = OnceLock::new();
    let until = *UNTIL.get_or_init(|| {
        std::env::var("AURORA_GPU_OOM_FIRST_USE")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|seconds| Instant::now() + Duration::from_secs(seconds))
    });
    until.is_some_and(|t| Instant::now() < t)
}

/// `AURORA_GPU_OOM_START=<seconds>`: for that long after the renderer is
/// first made, its placeholder textures come back invalid as if video
/// memory were full, at start and at each remake of the renderer
/// (testing). Made with no usage, as for `AURORA_GPU_OOM_TARGETS`.
pub fn start_refused() -> bool {
    static UNTIL: OnceLock<Option<Instant>> = OnceLock::new();
    let until = *UNTIL.get_or_init(|| {
        std::env::var("AURORA_GPU_OOM_START")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|seconds| Instant::now() + Duration::from_secs(seconds))
    });
    until.is_some_and(|t| Instant::now() < t)
}

/// The usage a placeholder texture is made with: none while
/// `AURORA_GPU_OOM_START` refuses it, which wgpu answers with an invalid
/// texture.
pub fn start_usage(usage: wgpu::TextureUsages, refuse: bool) -> wgpu::TextureUsages {
    if refuse { wgpu::TextureUsages::empty() } else { usage }
}

/// Whether a renderer whose start scope saw these errors is made again
/// later. Refused render targets leave invalid views in what binds them,
/// made outside their own scope: with them refused, only the memory's own
/// word counts (any resource it refuses says so when it is made), and the
/// targets' retry is left to bring the world back. A refusal on a device
/// lost meanwhile is the loss, which ends the viewer (or recovers it), not
/// a full video memory.
pub fn start_remade(oom: bool, invalid: bool, targets_refused: bool, lost: bool) -> bool {
    (oom || (invalid && !targets_refused)) && !lost
}

/// The interface's blur and glass of a window this size fall under
/// `AURORA_GPU_OOM`, as a texture of the window's size.
pub fn ui_targets_refused(size: [u32; 2]) -> bool {
    injected_failure(u64::from(size[0]) * u64::from(size[1]) * 4).is_some()
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

    /// A try that failed, which lengthens the wait only when it was due:
    /// the first failure, or a try at or after the scheduled time. A try
    /// made early (the window resized while waiting) keeps the schedule;
    /// dragging the window's edge would otherwise put the next try 30 s
    /// away.
    pub fn failed_try(&mut self, now: Instant) {
        if self.ready(now) {
            self.failed(now);
        }
    }

    pub fn succeeded(&mut self) {
        *self = Self::default();
    }

    /// A try that worked, when what follows it in the same frame can still
    /// be refused (the rigged blend depth after the render targets, the
    /// labels' glass after the glass): the wait is kept, so a refusal right
    /// after lengthens it instead of starting again at 2 s, which would
    /// rebuild the whole set every 2 s for good. [`Self::settle`] ends it
    /// once a frame went through.
    pub fn made(&mut self) {
        self.next = None;
    }

    /// The end of a frame: with no failure waited out, the wait goes back to
    /// nothing, as after [`Self::succeeded`].
    pub fn settle(&mut self) {
        if self.next.is_none() {
            self.wait = Duration::ZERO;
        }
    }

    /// A failure is being waited out.
    pub fn waiting(&self) -> bool {
        self.next.is_some()
    }

    /// When the next try is due, while a failure is being waited out.
    pub fn next_try(&self) -> Option<Instant> {
        self.next
    }
}

/// What a full video memory refused of the targets a frame draws into,
/// and when to try again. The render targets (`targets`) are made as a set
/// at a resize or a setting; others are made on first use, mid-frame:
///
/// - **required** ones the frame cannot go without (the rigged blend
///   depth, RLVa's spheres and reduced scene, the TAA history of a setting
///   change): refused, the frame shows the interface alone, as for the
///   render targets. Drawn without RLVa's targets, a frame would show what
///   RLVa hides. They wait on the render targets' wait, which a remade
///   set keeps until the frame goes through (`Backoff::made`): otherwise
///   every remade set would start it again at 2 s, and the world come and
///   go every 2 s;
/// - **extras** the frame can go without (the selection outline, the depth
///   of field, a photo look): refused, the frame is drawn without them
///   until their wait (`extras`) is over.
#[derive(Debug, Default)]
pub struct Refusals {
    /// The render targets came back invalid (video memory full): frames
    /// show the interface alone until they can be made
    /// (`RenderStats::video_memory_full`). Without this, every frame after
    /// them failed and the window stayed black for the rest of the session.
    pub targets: bool,
    /// When to try the render targets again.
    pub targets_backoff: Backoff,
    /// A stop of the world was said and its end not yet: the end is said
    /// once a frame goes through ([`Self::settle`]), not when the render
    /// targets are remade, since a required target can refuse right after.
    said: bool,
    extras: Backoff,
}

impl Refusals {
    /// The render targets are refused and due another try.
    pub fn targets_due(&self, now: Instant) -> bool {
        self.targets && self.targets_backoff.ready(now)
    }

    /// The render targets were refused; true the first time (to say so).
    pub fn targets_failed(&mut self, now: Instant) -> bool {
        let first = !self.said;
        self.said = true;
        self.targets = true;
        // a resize, a setting or a photo while refused is a try too, but
        // only a due one lengthens the wait
        self.targets_backoff.failed_try(now);
        first
    }

    /// The render targets were made. The wait is kept until the frame ends
    /// ([`Self::settle`]): a required target refused right after lengthens
    /// it (`Backoff::made`).
    pub fn targets_made(&mut self) {
        self.targets = false;
        self.targets_backoff.made();
    }

    /// The outcome of a required target made on first use; true the first
    /// time it stops the world (to say so). Refused, it waits on the render
    /// targets' wait: the next due try makes every target again.
    pub fn first_use(&mut self, refused: bool, now: Instant) -> bool {
        if !refused {
            return false;
        }
        let first = !self.said;
        self.said = true;
        self.targets = true;
        self.targets_backoff.failed_try(now);
        first
    }

    /// The end of a frame that went through (`Backoff::settle`); true the
    /// first time after a stop of the world (to say it is back).
    pub fn settle(&mut self) -> bool {
        self.targets_backoff.settle();
        let back = self.said && !self.targets;
        if back {
            self.said = false;
        }
        back
    }

    /// An extra may be made now.
    pub fn extras_ready(&self, now: Instant) -> bool {
        self.extras.ready(now)
    }

    /// The outcome of an extra made on first use; true the first time it
    /// is refused (to say so).
    pub fn extra(&mut self, refused: bool, now: Instant) -> bool {
        if !refused {
            self.extras.succeeded();
            return false;
        }
        // Nothing tried for a whole wait after it was due: no extra was
        // needed meanwhile, and the memory may have come back and gone
        // again. Without a success to clear it, a refusal then went
        // unsaid and waited twice as long as a first one.
        if self.extras.next.is_some_and(|due| now >= due + self.extras.wait) {
            self.extras.succeeded();
        }
        let first = !self.extras.waiting();
        self.extras.failed_try(now);
        first
    }
}

/// Grow a buffer whose contents the caller uploads again in full. Retain
/// the valid allocation on failure; the caller must limit its upload and
/// draw count to the retained size. No old contents are copied.
pub fn grow_upload_buffer(device: &wgpu::Device, buffer: &mut wgpu::Buffer, backoff: &mut Backoff, label: &str, bytes: u64) -> bool {
    if bytes <= buffer.size() {
        return true;
    }
    let now = Instant::now();
    if !backoff.ready(now) {
        return false;
    }
    let usage = buffer.usage();
    assert!(usage.contains(wgpu::BufferUsages::COPY_DST));
    let limits = device.limits();
    let limit = if usage.contains(wgpu::BufferUsages::STORAGE) {
        limits.max_buffer_size.min(limits.max_storage_buffer_binding_size)
    } else {
        limits.max_buffer_size
    } / wgpu::COPY_BUFFER_ALIGNMENT
        * wgpu::COPY_BUFFER_ALIGNMENT;
    let needed = bytes.checked_next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT).unwrap_or(u64::MAX);
    let preferred = needed.checked_next_power_of_two().unwrap_or(limit).min(limit);
    let mut failure = format!("{bytes} bytes exceed the buffer limit of {limit}");
    if needed <= limit {
        for size in [preferred, needed] {
            match try_buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage,
                    mapped_at_creation: false,
                },
            ) {
                Ok(replacement) => {
                    *buffer = replacement;
                    backoff.succeeded();
                    return true;
                }
                Err(e) => failure = e,
            }
            if preferred == needed {
                break;
            }
        }
    }
    if !backoff.waiting() {
        log::warn!(
            "{label} buffer not grown ({failure}): keeping {} bytes until memory is available",
            buffer.size()
        );
    }
    backoff.failed(now);
    false
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
    fn a_renderer_refused_at_start_is_made_again_unless_the_device_was_lost() {
        // (oom, invalid, targets refused, lost)
        assert!(start_remade(true, false, false, false));
        assert!(start_remade(false, true, false, false), "an invalid object with the targets made");
        assert!(start_remade(true, true, true, false), "the memory refused more than the targets");
        assert!(
            !start_remade(false, true, true, false),
            "what binds the refused targets: their own retry"
        );
        assert!(!start_remade(true, true, false, true), "a lost device is not a full video memory");
        assert!(!start_remade(false, false, false, false));
        assert!(!start_remade(false, false, true, false));
    }

    #[test]
    fn a_refused_start_texture_is_made_with_no_usage() {
        let u = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        assert_eq!(start_usage(u, false), u);
        assert!(start_usage(u, true).is_empty());
    }

    #[test]
    #[ignore]
    fn upload_buffer_growth_respects_limits_and_keeps_the_previous_buffer() {
        let _gpu = crate::test_gpu::lock();
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("Vulkan adapter");
        let mut limits = adapter.limits();
        limits.max_buffer_size = 1_500_000;
        limits.max_storage_buffer_binding_size = 1_500_000;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: limits,
            ..Default::default()
        }))
        .expect("GPU device");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload growth"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut backoff = Backoff::default();
        assert!(grow_upload_buffer(&device, &mut buffer, &mut backoff, "upload growth", 1_100_004));
        assert_eq!(buffer.size(), 1_500_000, "the power-of-two growth was capped to the device limit");
        let previous = buffer.clone();
        assert!(!grow_upload_buffer(&device, &mut buffer, &mut backoff, "upload growth", 1_500_004));
        assert_eq!(buffer, previous);
        assert!(!backoff.ready(Instant::now()));
        queue.write_buffer(&buffer, 0, &[1; 4096]);
        assert!(pollster::block_on(scope.pop()).is_none());
    }

    /// Run in a separate process with AURORA_GPU_OOM=0.
    #[test]
    #[ignore]
    fn refused_upload_buffer_growth_keeps_a_valid_buffer() {
        assert_eq!(std::env::var("AURORA_GPU_OOM").as_deref(), Ok("0"));
        let _gpu = crate::test_gpu::lock();
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("Vulkan adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload growth"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let previous = buffer.clone();
        let mut backoff = Backoff::default();
        assert!(!grow_upload_buffer(&device, &mut buffer, &mut backoff, "upload growth", 8192));
        assert_eq!(buffer, previous);
        assert_eq!(buffer.size(), 4096);
        assert!(!backoff.ready(Instant::now()));
        queue.write_buffer(&buffer, 0, &[1; 4096]);
        assert!(pollster::block_on(scope.pop()).is_none());
    }

    #[test]
    fn only_a_refused_allocation_counts_as_out_of_memory() {
        assert!(memory_refused(Some("Out of Memory"), None));
        assert!(memory_refused(
            None,
            Some("In Texture::create_view\n  Texture with 'hdr' label is invalid")
        ));
        assert!(memory_refused(Some("Out of Memory"), Some("anything")));
        assert!(!memory_refused(None, None));
        // a wrong descriptor is ours: trying again would never end
        assert!(!memory_refused(None, Some("In Device::create_texture\n  Invalid usage flags")));
    }

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
    fn only_a_due_try_lengthens_the_wait() {
        let t0 = Instant::now();
        let mut b = Backoff::default();
        // the first failure starts the wait
        b.failed_try(t0);
        assert!(!b.ready(t0 + Duration::from_millis(1999)));
        assert!(b.ready(t0 + Duration::from_secs(2)));
        // tries before the 2 s are up (resizes) change nothing
        for ms in [100, 500, 1500, 1999] {
            b.failed_try(t0 + Duration::from_millis(ms));
        }
        assert!(b.ready(t0 + Duration::from_secs(2)));
        // the scheduled try at 2 s doubles the wait from then: next at 6 s
        let t2 = t0 + Duration::from_secs(2);
        b.failed_try(t2);
        assert!(!b.ready(t2 + Duration::from_millis(3999)));
        assert!(b.ready(t2 + Duration::from_secs(4)));
        b.succeeded();
        assert!(b.ready(t2) && !b.waiting());
    }

    #[test]
    fn a_required_first_use_refusal_stops_the_world_on_the_targets_wait() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut r = Refusals::default();
        assert!(!r.targets_due(t0));
        // RLVa's spheres refused mid-frame: the world stops, said once
        assert!(r.first_use(true, t0));
        assert!(!r.first_use(true, t0));
        assert!(r.targets && !r.targets_due(t0 + Duration::from_millis(1999)));
        assert!(r.targets_due(t0 + s(2)));
        // the due try remakes the render targets
        let t2 = t0 + s(2);
        r.targets_made();
        assert!(!r.targets);
        // the spheres refused again in the same frame: 4 s now, not 2, and
        // neither the world's return nor a second stop is said
        assert!(!r.first_use(true, t2));
        assert!(!r.settle());
        assert!(!r.targets_due(t2 + Duration::from_millis(3999)));
        assert!(r.targets_due(t2 + s(4)));
        // made at last and the frame went through: the next refusal of the
        // render targets starts at 2 s
        let t6 = t2 + s(4);
        r.targets_made();
        assert!(!r.first_use(false, t6));
        assert!(r.settle(), "the world's return is said once the frame goes through");
        assert!(!r.settle());
        assert!(r.targets_failed(t6));
        assert!(r.targets_due(t6 + s(2)));
        assert!(!r.targets_failed(t6 + s(1)));
    }

    #[test]
    fn the_injected_failure_may_start_late_and_end_after_a_while() {
        let now = Instant::now();
        let s = Duration::from_secs;
        let oom = |bytes, from, until| Some(InjectedOom { bytes, from, until });
        assert_eq!(parse_oom("64", now), oom(64 << 20, None, None));
        assert_eq!(parse_oom(" 0 , 30 ", now), oom(0, None, Some(now + s(30))));
        // from 60 s, for 20 s
        let late = parse_oom("1,20,60", now).expect("parsed");
        assert_eq!(
            late,
            InjectedOom {
                bytes: 1 << 20,
                from: Some(now + s(60)),
                until: Some(now + s(80))
            }
        );
        assert!(!late.active(now + s(59)));
        assert!(late.active(now + s(60)) && late.active(now + s(79)));
        assert!(!late.active(now + s(80)));
        assert_eq!(parse_oom("64,soon", now), None);
        assert_eq!(parse_oom("64,1,2,3", now), None);
        assert_eq!(parse_oom("lots", now), None);
    }

    #[test]
    fn a_refused_extra_waits_without_stopping_the_world() {
        let t0 = Instant::now();
        let mut r = Refusals::default();
        assert!(r.extras_ready(t0));
        assert!(r.extra(true, t0));
        assert!(!r.targets, "the world goes on without the extra");
        assert!(!r.extras_ready(t0 + Duration::from_secs(1)));
        // said once while waiting
        assert!(!r.extra(true, t0 + Duration::from_secs(1)));
        assert!(r.extras_ready(t0 + Duration::from_secs(2)));
        assert!(!r.extra(false, t0 + Duration::from_secs(2)));
        assert!(r.extras_ready(t0 + Duration::from_secs(2)));
        // the render targets' success does not touch the extras' wait
        r.extra(true, t0);
        r.targets_made();
        assert!(!r.extras_ready(t0 + Duration::from_secs(1)));
    }

    #[test]
    fn an_extra_refused_long_after_its_wait_starts_again() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut r = Refusals::default();
        assert!(r.extra(true, t0));
        // retried when due, refused again: still the same episode
        assert!(!r.extra(true, t0 + s(2)));
        assert!(!r.extras_ready(t0 + s(5)), "the wait doubled to 4 s");
        // no extra needed for a minute, then refused: said again, 2 s wait
        assert!(r.extra(true, t0 + s(66)));
        assert!(!r.extras_ready(t0 + s(67)));
        assert!(r.extras_ready(t0 + s(68)));
    }

    #[test]
    fn a_refusal_right_after_a_success_keeps_lengthening_the_wait() {
        let t0 = Instant::now();
        let mut b = Backoff::default();
        b.failed(t0);
        // the set made at 2 s, the blend depth refused in the same frame
        let t2 = t0 + Duration::from_secs(2);
        b.made();
        assert!(b.ready(t2) && !b.waiting());
        b.failed(t2);
        assert!(!b.ready(t2 + Duration::from_millis(3999)));
        assert!(b.ready(t2 + Duration::from_secs(4)));
        // the frame ends while waiting: the wait stays
        b.settle();
        let t6 = t2 + Duration::from_secs(4);
        b.made();
        b.failed(t6);
        assert!(!b.ready(t6 + Duration::from_millis(7999)));
        // up to 30 s, never back to 2 s
        let mut t = t6;
        for _ in 0..6 {
            t += Duration::from_secs(30);
            b.made();
            b.failed(t);
        }
        assert!(!b.ready(t + Duration::from_millis(29_999)));
        assert!(b.ready(t + Duration::from_secs(30)));
        // a frame that went through: the next failure starts at 2 s again
        b.made();
        b.settle();
        b.failed(t);
        assert!(b.ready(t + Duration::from_secs(2)));
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
            size: crate::readback::readback_size(16),
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
