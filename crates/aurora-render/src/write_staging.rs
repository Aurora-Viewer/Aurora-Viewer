//! One staging buffer for the writes of a frame's journal.
//!
//! `Queue::write_buffer` creates a staging buffer of its own for each call
//! (wgpu-core `Queue::write_buffer` → `StagingBuffer::new`: a Vulkan buffer
//! created, its memory allocated and mapped, then freed once the submit is
//! done). A busy region writes thousands of small ranges per frame —
//! records, culling tables, palettes, the vertices of flexible prims — and
//! the replay of the journal (`r_resources`, render thread) followed their
//! number: on Agni on 2026-11-04, 3.3 to 5.5 ms per frame in busy regions,
//! 3,900 buffer writes in one slow frame.
//!
//! Here the replay copies the bytes of every buffer write into one staging
//! buffer, mapped ahead of time, and records a GPU copy per write (two
//! writes that follow each other in the same buffer make one copy), in an
//! encoder submitted at the end of the replay, before the frame's own
//! submit. Same order as before: the copies keep the journal's order among
//! themselves, a submit recorded in the journal (an arena growth) gets the
//! copies recorded before it in front of its own commands, and the buffer
//! writes after it go through the queue as before (rare).
//!
//! Texture writes are staged too. `Queue::write_texture` also makes a
//! staging buffer for each call, and wgpu takes a refusal of that buffer's
//! memory as the device lost (wgpu-core `Queue::write_texture` →
//! `StagingBuffer::new` → `Device::handle_hal_error`, which loses the
//! device on an out-of-memory), where a refused `create_buffer` is an error
//! that can be caught. A texture write's rows are copied into the staging
//! buffer at the stride a buffer-to-texture copy wants (256 bytes), when
//! they fit beside the room kept for the frame's buffer writes still to
//! come. One that does not fit, or a frame with no staging buffer ready,
//! gets a staging buffer of its own, made through `gpu_alloc::try_buffer`;
//! its copy goes into the same encoder. The texture copies follow the
//! buffer copies there, in the journal's order among themselves (a texture
//! copy and a buffer copy never touch the same memory). A buffer of its own
//! that the driver refuses is asked for again for half the write's rows,
//! then half of that, down to [`MIN_PIECE`] bytes; refused at that size
//! (or come back but not writable), the write is not made, nor any texture
//! write of the frame after it, nor what was staged for its slot earlier in
//! the frame (the levels before a refused one), and the texture table's
//! slots of those
//! writes are handed back ([`Staged::lost`]): the main thread has their
//! owners send them again later, at the slot's place and size then (a
//! write kept for a later frame would land behind the frame's page moves
//! and newer uploads of the slot, recorded in the upload encoder). Said in
//! the log; the texture shows what its layer held until then. The buffers
//! of their own are kept, up to [`SPARE_BYTES`] and [`MAX_SPARES`] of them,
//! mapped again for the frames after: wgpu fills a new mapped buffer with
//! zeros on the CPU, which the queue's own staging buffers skip.
//!
//! A few staging buffers take turns: a used one is mapped again
//! (`map_async`) and comes back once the GPU has done its copies. When none
//! is ready (the GPU is several frames behind, or the frame writes more
//! than any of them holds: one of the right size is then made for the
//! frames after) or a frame writes more than [`MAX_STAGED`] bytes into
//! buffers, the frame's buffer writes go through the queue. At most
//! `MAX_SLOTS` buffers of [`MAX_STAGED`] bytes stay allocated (a busy
//! region: 1 to 16 MB each). A staging buffer the driver refuses is not
//! asked for again before a wait (`gpu_alloc::Backoff`), and then for the
//! frame's buffer writes alone when no slot holds them; one that comes
//! back without its mapping counts as refused, and one whose mapping again
//! fails is let go.
//! `AURORA_WRITE_STAGING=0` keeps the queue writes, buffers and textures,
//! `alt` alternates. `AURORA_STAGING_OOM=<KB>` refuses every staging buffer
//! of this module of at least that size, as a full card would (testing; 0
//! refuses them all), and `AURORA_STAGING_OOM_SECONDS=<s>` ends the refusal
//! that long after the first staging buffer asked for (memory that comes
//! back).

use crate::gpu_alloc::{self, Backoff, LogThrottle};
use crate::writes::{NO_SLOT, TexelDst, WriteSink};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Staging buffers taking turns: one being written, the others waiting for
/// the GPU's copies of the frames before.
const MAX_SLOTS: usize = 3;
/// Smallest staging buffer (bytes).
const MIN_SIZE: u64 = 1 << 20;
/// A frame that writes more than this into buffers (a region's arrival)
/// sends them through the queue: a staging buffer that large would stay
/// allocated.
pub const MAX_STAGED: u64 = 32 << 20;
/// The smallest piece a texture write is cut into when its staging buffer
/// is refused (bytes; one row at least).
pub const MIN_PIECE: u64 = 64 << 10;
/// The staging buffers of texture writes kept for the frames after, at
/// most (bytes).
pub const SPARE_BYTES: u64 = 32 << 20;
/// The staging buffers of texture writes kept, at most (a region's first
/// frame makes hundreds of small ones).
pub const MAX_SPARES: usize = 64;
/// How long a refused size is remembered: the texture writes of the frames
/// after are cut below it at once instead of being refused again.
const CEILING_KEPT: Duration = Duration::from_secs(2);

/// `AURORA_WRITE_STAGING`: `0` queue writes as before, `alt` alternates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    On,
    Off,
    Alternate,
}

fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_WRITE_STAGING").as_deref() {
        Ok("0") => Mode::Off,
        Ok("alt") => Mode::Alternate,
        _ => Mode::On,
    })
}

/// The old way this second, with `alt`.
static OLD_NOW: AtomicBool = AtomicBool::new(false);

/// `AURORA_WRITE_STAGING=alt`: the way of second `n` (true: queue writes,
/// one second in three). None without `alt`.
pub fn alternate(n: u64) -> Option<bool> {
    (mode() == Mode::Alternate).then(|| {
        let old = n.is_multiple_of(3);
        OLD_NOW.store(old, Ordering::Relaxed);
        old
    })
}

fn staging_on() -> bool {
    match mode() {
        Mode::On => true,
        Mode::Off => false,
        Mode::Alternate => !OLD_NOW.load(Ordering::Relaxed),
    }
}

/// `AURORA_STAGING_OOM=<KB>`, in bytes.
fn parse_refusal(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok().map(|kb| kb << 10)
}

/// `AURORA_STAGING_OOM_SECONDS=<s>`.
fn parse_refusal_time(value: Option<&str>) -> Option<Duration> {
    value?
        .trim()
        .parse::<f64>()
        .ok()
        // (negative, not a number or past what a Duration holds: none)
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
}

/// A staging buffer, or why it could not be made: refused at or above
/// `refuse_at` (`AURORA_STAGING_OOM`), otherwise asked of the device.
fn try_staging(device: &wgpu::Device, label: &'static str, size: u64, refuse_at: Option<u64>) -> Result<wgpu::Buffer, String> {
    if refuse_at.is_some_and(|at| size >= at) {
        return Err("out of video memory (AURORA_STAGING_OOM)".to_owned());
    }
    let buffer = gpu_alloc::try_buffer(
        device,
        &wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        },
    )?;
    writable(&buffer)?;
    Ok(buffer)
}

/// Whether a staging buffer made mapped has its mapping. One that came
/// back invalid without an error the scopes saw (gpu_alloc.rs: its probe
/// needs COPY_DST, which a mappable buffer cannot have) has none: refused
/// here, it is neither installed as the frame's slot (picked every frame
/// after, never written) nor taken whole for a texture write that could
/// have been cut in pieces.
fn writable(buffer: &wgpu::Buffer) -> Result<(), String> {
    match buffer.get_mapped_range_mut(..) {
        Ok(_) => Ok(()),
        Err(_) => Err("a staging buffer came back that cannot be written".to_owned()),
    }
}

/// A copy from the staging buffer into `dst`.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyOp<B> {
    pub dst: B,
    pub dst_offset: u64,
    pub src_offset: u64,
    pub size: u64,
}

/// Adds the copy of `size` bytes staged at `src_offset` to `dst` at
/// `dst_offset`, extending the last copy when it ends where this one
/// starts, in the staging buffer and in `dst`.
pub fn push_copy<B: PartialEq + Clone>(copies: &mut Vec<CopyOp<B>>, dst: &B, dst_offset: u64, src_offset: u64, size: u64) {
    if let Some(last) = copies.last_mut()
        && last.dst == *dst
        && last.dst_offset + last.size == dst_offset
        && last.src_offset + last.size == src_offset
    {
        last.size += size;
        return;
    }
    copies.push(CopyOp {
        dst: dst.clone(),
        dst_offset,
        src_offset,
        size,
    });
}

/// The size of the staging buffer made for `bytes`.
pub fn slot_size(bytes: u64) -> u64 {
    bytes.max(MIN_SIZE).next_power_of_two()
}

/// The stride of a staged row of RGBA8 texels `width` wide: a
/// buffer-to-texture copy wants rows at a multiple of 256 bytes.
pub fn padded_row(width: u32) -> u64 {
    (u64::from(width) * 4).next_multiple_of(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
}

/// Bytes that `rows` rows of a texture write `width` texels wide take
/// staged: the padded stride for each row but the last, which stops at its
/// texels (what the copy reads).
pub fn staged_len(width: u32, rows: u32) -> u64 {
    match rows {
        0 => 0,
        n => padded_row(width) * u64::from(n - 1) + u64::from(width) * 4,
    }
}

/// Rows `row..row + rows` of `data` (packed RGBA8, `width` texels a row)
/// handed to `put` with their offsets from the first one staged: in one
/// piece when the rows are already at the padded stride, one row at a time
/// otherwise.
pub fn stage_rows(data: &[u8], width: u32, row: u32, rows: u32, mut put: impl FnMut(usize, &[u8])) {
    let tight = width as usize * 4;
    let padded = padded_row(width) as usize;
    let src = &data[row as usize * tight..(row + rows) as usize * tight];
    if tight == padded {
        put(0, src);
    } else {
        for (i, r) in src.chunks_exact(tight).enumerate() {
            put(i * padded, r);
        }
    }
}

/// A run of rows of one texture write and the buffer it is staged in.
#[derive(Debug, PartialEq)]
pub struct Piece<A> {
    pub buffer: A,
    pub row: u32,
    pub rows: u32,
}

/// Staging buffers from `alloc` for the `rows` rows of a texture write
/// `width` texels wide: one for them all if `alloc` makes it, otherwise one
/// for each half, and so on down to [`MIN_PIECE`] bytes (one row at
/// least). `ceiling` is the smallest size refused lately: the pieces are cut
/// below it before they are asked for, and a refusal lowers it. When even
/// the smallest piece is refused, the buffers made so far come back as the
/// error, and the write is not staged.
pub fn split_rows<A>(width: u32, rows: u32, ceiling: &mut u64, mut alloc: impl FnMut(u64) -> Option<A>) -> Result<Vec<Piece<A>>, Vec<A>> {
    let min_rows = u32::try_from(MIN_PIECE / padded_row(width)).unwrap_or(u32::MAX).max(1);
    let half = |n: u32| (n / 2).max(min_rows);
    let mut pieces = Vec::new();
    let mut row = 0;
    while row < rows {
        let mut n = rows - row;
        while n > min_rows && staged_len(width, n) >= *ceiling {
            n = half(n);
        }
        loop {
            let len = staged_len(width, n);
            if let Some(buffer) = alloc(len) {
                pieces.push(Piece { buffer, row, rows: n });
                row += n;
                break;
            }
            *ceiling = (*ceiling).min(len);
            if n <= min_rows {
                return Err(pieces.into_iter().map(|p| p.buffer).collect());
            }
            n = half(n);
        }
    }
    Ok(pieces)
}

/// Picks the staging buffer of a frame among those `ready`, by their sizes:
/// the first of at least `want` bytes (the frame's buffer and texture
/// writes), otherwise the first of at least `need` (its buffer writes).
pub fn pick_slot(sizes: impl Iterator<Item = (u64, bool)> + Clone, need: u64, want: u64) -> Option<usize> {
    let first = |least: u64| sizes.clone().position(|(size, ready)| ready && size >= least);
    first(want).or_else(|| first(need))
}

/// The size of the staging buffer to make for a frame that writes `need`
/// bytes into buffers and `want` in all: the one that holds them all,
/// unless that size (or more) was refused since the last one made and no
/// slot holds the buffer writes alone; then one that does (buffer writes
/// not staged go through the queue, whose own staging buffer is fatal when
/// refused).
pub fn slot_wanted(need: u64, want: u64, refused: Option<u64>, need_held: bool) -> u64 {
    let size = slot_size(want);
    let small = slot_size(need);
    match refused {
        Some(r) if size >= r && small < r && !need_held => small,
        _ => size,
    }
}

/// Whether a texture write of `len` bytes fits in an open staging buffer of
/// `size` bytes written up to `cursor`, beside the `buffer_left` bytes of
/// buffer writes still to come.
pub fn texture_fits(size: u64, cursor: u64, buffer_left: u64, len: u64) -> bool {
    cursor.saturating_add(buffer_left).saturating_add(len) <= size
}

struct Slot {
    buffer: wgpu::Buffer,
    /// Mapped, and no copy of a submitted frame reads it any more.
    ready: Arc<AtomicBool>,
    /// Its mapping again failed: it is let go (it would never be ready).
    failed: Arc<AtomicBool>,
}

impl Slot {
    fn new(buffer: wgpu::Buffer, ready: bool) -> Self {
        Self {
            buffer,
            ready: Arc::new(AtomicBool::new(ready)),
            failed: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn has_failed(s: &Slot) -> bool {
    s.failed.load(Ordering::Acquire)
}

fn is_ready(s: &Slot) -> bool {
    s.ready.load(Ordering::Acquire)
}

/// What a frame's replay staged (diagnostics).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Staged {
    /// Buffer writes copied from the frame's staging buffer.
    pub buffers: usize,
    /// Texture writes copied from it.
    pub textures: usize,
    /// Texture writes copied from staging buffers of their own.
    pub own: usize,
    /// Of those, the ones cut in pieces after a refusal.
    pub split: usize,
    /// Texture writes not made: no staging buffer could be made for them,
    /// or for one before them.
    pub refused: usize,
    /// The texture table's slots of those writes, each once, in the order
    /// met (writes that named none left out): their textures are to be
    /// sent again.
    pub lost: Vec<u32>,
}

/// The staging buffers of the render thread.
pub struct WriteStaging {
    slots: Vec<Slot>,
    /// A larger staging buffer being made on a thread of its own.
    making: Option<mpsc::Receiver<Result<wgpu::Buffer, String>>>,
    /// One made, waiting for a slot to take.
    made: Option<wgpu::Buffer>,
    /// The size of one to make after this frame's writes.
    wanted: Option<u64>,
    /// The size of the one being made.
    asked: u64,
    /// The smallest size refused since the last one made.
    slot_refused: Option<u64>,
    /// When to ask for a staging buffer again after one was refused.
    backoff: Backoff,
    /// The staging buffers of texture writes, kept mapped for the frames
    /// after.
    spares: Vec<Slot>,
    /// The smallest staging buffer of a texture write refused lately, and
    /// when.
    ceiling: Option<(u64, Instant)>,
    /// `AURORA_STAGING_OOM`: staging buffers of at least this size refused.
    refuse_at: Option<u64>,
    /// `AURORA_STAGING_OOM_SECONDS`: how long, from the first staging
    /// buffer asked for, and when that was.
    refuse_for: Option<Duration>,
    refusing_since: Option<Instant>,
    log: LogThrottle,
}

impl Default for WriteStaging {
    fn default() -> Self {
        let mut staging = Self::with_refusal(parse_refusal(std::env::var("AURORA_STAGING_OOM").ok().as_deref()));
        staging.refuse_for = parse_refusal_time(std::env::var("AURORA_STAGING_OOM_SECONDS").ok().as_deref());
        staging
    }
}

/// Maps the slot's buffer again; `ready` once it is (after the GPU's last
/// use), `failed` if it cannot be.
fn remap(slot: &Slot) {
    let (ready, failed) = (slot.ready.clone(), slot.failed.clone());
    slot.buffer.map_async(wgpu::MapMode::Write, .., move |r| {
        failed.store(r.is_err(), Ordering::Release);
        ready.store(r.is_ok(), Ordering::Release);
    });
}

impl WriteStaging {
    /// Staging buffers of at least `refuse_at` bytes refused, as
    /// `AURORA_STAGING_OOM` does (tests).
    pub fn with_refusal(refuse_at: Option<u64>) -> Self {
        Self {
            slots: Vec::new(),
            making: None,
            made: None,
            wanted: None,
            asked: 0,
            slot_refused: None,
            backoff: Backoff::default(),
            spares: Vec::new(),
            ceiling: None,
            refuse_at,
            refuse_for: None,
            refusing_since: None,
            log: LogThrottle::new(Duration::from_secs(10)),
        }
    }

    /// Refuses staging buffers of at least `refuse_at` bytes from now on
    /// (tests: a card that fills, then frees memory).
    pub fn set_refusal(&mut self, refuse_at: Option<u64>) {
        self.refuse_at = refuse_at;
        self.ceiling = None;
    }

    /// The size from which staging buffers are refused now
    /// (`AURORA_STAGING_OOM`), until `AURORA_STAGING_OOM_SECONDS` is over.
    fn refusal(&mut self) -> Option<u64> {
        let (Some(_), Some(over)) = (self.refuse_at, self.refuse_for) else {
            return self.refuse_at;
        };
        let now = Instant::now();
        let since = *self.refusing_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= over {
            self.refuse_at = None;
            log::info!("journal staging: AURORA_STAGING_OOM over, staging buffers are made again");
        }
        self.refuse_at
    }

    fn warn(&mut self, message: &str) {
        if let Some(line) = self.log.note(message, Instant::now()) {
            log::warn!("{line}");
        }
    }

    /// Starts making the staging buffer wanted, on a thread of its own.
    fn make_wanted(&mut self, device: &wgpu::Device) {
        if !self.backoff.ready(Instant::now()) {
            self.wanted = None;
            return;
        }
        let Some(size) = self.wanted.take() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let device = device.clone();
        let refuse_at = self.refusal();
        let spawned = std::thread::Builder::new().name("journal staging".into()).spawn(move || {
            let _ = tx.send(try_staging(&device, "journal staging", size, refuse_at));
        });
        match spawned {
            Ok(_) => {
                self.making = Some(rx);
                self.asked = size;
            }
            Err(e) => log::warn!("journal staging: no thread to make a staging buffer ({e})"),
        }
    }

    /// A ready slot for a frame that writes `need` bytes into buffers and
    /// `want` bytes in all (buffers, then textures), or None. A staging
    /// buffer is made mapped, and wgpu fills a buffer mapped at its creation
    /// with zeros on the CPU (7.7 ms for 16 MB at the demo crowd's
    /// arrival): it is made on a thread of its own and joins the slots when
    /// it is there. (Zeroing it on the GPU is not allowed: a mappable buffer
    /// cannot be a copy destination.)
    fn acquire(&mut self, need: u64, want: u64) -> Option<usize> {
        self.slots.retain(|s| !has_failed(s));
        let arrived = match self.making.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(made)) => {
                self.making = None;
                match made {
                    Ok(buffer) => {
                        self.backoff.succeeded();
                        self.slot_refused = None;
                        Some(buffer)
                    }
                    Err(e) => {
                        // nothing installed: the frames go on with the slots
                        // they have, and the next try waits
                        self.backoff.failed(Instant::now());
                        self.slot_refused = Some(self.slot_refused.map_or(self.asked, |r| r.min(self.asked)));
                        self.warn(&format!("journal staging: a staging buffer was refused ({e})"));
                        None
                    }
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.making = None;
                None
            }
            _ => None,
        };
        if let Some(buffer) = self.made.take().or(arrived) {
            let size = buffer.size();
            let slot = Slot::new(buffer, true);
            // one more, or in place of a ready one smaller
            if self.slots.len() < MAX_SLOTS {
                self.slots.push(slot);
            } else if let Some(i) = self.slots.iter().position(|s| is_ready(s) && s.buffer.size() < size) {
                self.slots[i] = slot;
            } else {
                self.made = Some(slot.buffer);
            }
        }
        let picked = pick_slot(self.slots.iter().map(|s| (s.buffer.size(), is_ready(s))), need, want);
        // none holds the frame's writes, or they all wait for the GPU and
        // there is room for one more: one is made for the frames after, once
        // this frame's writes are done (the zeros and the queue's copies
        // share the memory bandwidth: made before them, the demo crowd's
        // second arrival frame took 4.3 ms of writes, 2.7 made after, 1.4
        // without staging; the frames after are 1.5 times faster)
        let fits = self.slots.iter().any(|s| s.buffer.size() >= want);
        let busy = picked.is_none() && self.slots.len() < MAX_SLOTS;
        if self.making.is_none() && self.made.is_none() && (!fits || busy) {
            let need_held = self.slots.iter().any(|s| s.buffer.size() >= need);
            self.wanted = Some(slot_wanted(need, want, self.slot_refused, need_held));
        }
        picked
    }

    /// The sink the journal is replayed into: its buffer writes, `bytes` at
    /// most, staged when a slot is ready, the rest passed to `inner`. Call
    /// [`StagedSink::finish`] once the journal is replayed.
    pub fn sink<'a, S: WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>>(
        &'a mut self,
        device: &'a wgpu::Device,
        inner: &'a mut S,
        bytes: u64,
    ) -> StagedSink<'a, S> {
        self.sink_with_textures(device, inner, bytes, 0)
    }

    /// [`Self::sink`] for a journal whose texture writes take `textures`
    /// bytes staged (`Journal::texture_staged_bytes`): the slot taken holds
    /// them too when one does.
    pub fn sink_with_textures<'a, S: WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>>(
        &'a mut self,
        device: &'a wgpu::Device,
        inner: &'a mut S,
        bytes: u64,
        textures: u64,
    ) -> StagedSink<'a, S> {
        let on = staging_on();
        let want = bytes.saturating_add(textures).min(MAX_STAGED).max(bytes);
        let open = if on && want > 0 && bytes <= MAX_STAGED {
            self.acquire(bytes, want).and_then(|slot| {
                let s = &self.slots[slot];
                let view = s.buffer.get_mapped_range_mut(..).ok()?;
                s.ready.store(false, Ordering::Release);
                Some(Open { slot, view, cursor: 0 })
            })
        } else {
            None
        };
        let now = Instant::now();
        if self
            .ceiling
            .is_some_and(|(_, at)| now.saturating_duration_since(at) >= CEILING_KEPT)
        {
            self.ceiling = None;
        }
        StagedSink {
            staging: self,
            device,
            inner,
            on,
            open,
            buffer_left: bytes,
            copies: Vec::new(),
            textures: Vec::new(),
            own: Vec::new(),
            refused: None,
            staged: Staged::default(),
        }
    }

    /// A staging buffer of at least `bytes` for a texture write: a spare
    /// one, or a new one. A refusal lets the spare ones go (memory for what
    /// comes after).
    fn own_buffer(&mut self, device: &wgpu::Device, bytes: u64) -> Result<wgpu::Buffer, String> {
        self.spares.retain(|s| !has_failed(s));
        let spare = (self.spares.iter().enumerate())
            .filter(|(_, s)| is_ready(s) && s.buffer.size() >= bytes)
            .min_by_key(|(_, s)| s.buffer.size())
            .map(|(i, _)| i);
        if let Some(i) = spare {
            return Ok(self.spares.swap_remove(i).buffer);
        }
        let size = bytes.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        let made = try_staging(device, "texture write staging", size, self.refusal());
        if made.is_err() {
            self.spares.clear();
        }
        made
    }

    /// Keeps a used staging buffer of a texture write for the frames after,
    /// mapped again, while the spare ones hold less than [`SPARE_BYTES`].
    fn keep_spare(&mut self, buffer: wgpu::Buffer) {
        self.spares.retain(|s| !has_failed(s));
        let kept: u64 = self.spares.iter().map(|s| s.buffer.size()).sum();
        if kept + buffer.size() <= SPARE_BYTES && self.spares.len() < MAX_SPARES {
            let slot = Slot::new(buffer, false);
            remap(&slot);
            self.spares.push(slot);
        }
    }
}

struct Open {
    slot: usize,
    view: wgpu::BufferViewMut,
    cursor: u64,
}

/// Where a texture copy reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Src {
    /// The frame's staging buffer.
    Slot,
    /// A staging buffer of its own (`StagedSink::own`).
    Own(usize),
}

/// A copy of staged rows into a texture.
struct TexCopy {
    /// The texture table's slot of the write ([`NO_SLOT`] if none).
    slot: u32,
    src: Src,
    offset: u64,
    texture: wgpu::Texture,
    dst: TexelDst,
}

/// A journal replay that stages its writes (see the module).
pub struct StagedSink<'a, S> {
    staging: &'a mut WriteStaging,
    device: &'a wgpu::Device,
    inner: &'a mut S,
    /// Staging on this frame (`AURORA_WRITE_STAGING`).
    on: bool,
    open: Option<Open>,
    /// Bytes of the buffer writes not replayed yet: room the texture writes
    /// leave in the slot.
    buffer_left: u64,
    copies: Vec<CopyOp<wgpu::Buffer>>,
    textures: Vec<TexCopy>,
    /// The staging buffers of their own of the texture copies, mapped until
    /// the copies are submitted.
    own: Vec<wgpu::Buffer>,
    /// The size of the staging buffer refused at the smallest piece this
    /// frame, and why: the frame's next texture writes are not made, without
    /// asking.
    refused: Option<(u64, String)>,
    staged: Staged,
}

impl<S: WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>> StagedSink<'_, S> {
    /// Submits the copies staged so far, in front of `then`, and sends the
    /// staging buffers to be mapped again; the buffer writes after go
    /// through the queue.
    fn close(&mut self, then: Vec<wgpu::CommandEncoder>) {
        let slot = self.open.take().map(|open| {
            drop(open.view);
            open.slot
        });
        let slot_used = !self.copies.is_empty() || self.textures.iter().any(|t| t.src == Src::Slot);
        if let Some(i) = slot {
            if slot_used {
                self.staging.slots[i].buffer.unmap();
            } else {
                // nothing written: it is still mapped
                self.staging.slots[i].ready.store(true, Ordering::Release);
            }
        }
        if self.copies.is_empty() && self.textures.is_empty() {
            if !then.is_empty() {
                self.inner.submit(then);
            }
            return;
        }
        for b in &self.own {
            b.unmap();
        }
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("journal copies"),
        });
        // (a slot is open whenever a copy reads it)
        if let Some(i) = slot.filter(|_| slot_used) {
            let src = &self.staging.slots[i].buffer;
            for c in self.copies.drain(..) {
                enc.copy_buffer_to_buffer(src, c.src_offset, &c.dst, c.dst_offset, c.size);
            }
        }
        for t in self.textures.drain(..) {
            let buffer = match t.src {
                Src::Slot => slot.map(|i| &self.staging.slots[i].buffer),
                Src::Own(i) => self.own.get(i),
            };
            let Some(buffer) = buffer else {
                continue;
            };
            enc.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: t.offset,
                        bytes_per_row: Some(padded_row(t.dst.width) as u32),
                        rows_per_image: Some(t.dst.height),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &t.texture,
                    mip_level: t.dst.mip,
                    origin: wgpu::Origin3d {
                        x: t.dst.x,
                        y: t.dst.y,
                        z: t.dst.layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: t.dst.width,
                    height: t.dst.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.inner.submit(std::iter::once(enc).chain(then).collect());
        if let Some(i) = slot.filter(|_| slot_used) {
            remap(&self.staging.slots[i]);
        }
        for b in std::mem::take(&mut self.own) {
            self.staging.keep_spare(b);
        }
    }

    /// The end of the replay: the staged copies submitted. The number of
    /// buffer writes staged.
    pub fn finish(self) -> usize {
        self.finish_staged().buffers
    }

    /// The end of the replay: the staged copies submitted. What was staged.
    pub fn finish_staged(mut self) -> Staged {
        self.close(Vec::new());
        self.staging.make_wanted(self.device);
        if let Some((len, e)) = self.refused.take() {
            // (the throttle keys on the text: the size takes a few values,
            // the frame's counts stay out of it)
            self.staging.warn(&format!(
                "journal staging: texture writes not made, a staging buffer of {} KB was refused ({e}): their textures are sent again",
                len.div_ceil(1 << 10)
            ));
        }
        self.staged
    }

    /// A texture write not made: its slot handed back, and what was staged
    /// for it earlier in the frame not made either (the mip levels before
    /// a refused one: the texture would show the new texels at some levels
    /// and the old at the others until it is sent again).
    fn lose(&mut self, slot: u32) {
        self.staged.refused += 1;
        if slot != NO_SLOT && !self.staged.lost.contains(&slot) {
            self.staged.lost.push(slot);
            self.textures.retain(|t| t.slot != slot);
        }
    }

    /// Stages a texture write in buffers of its own, cut in pieces if the
    /// first is refused. False when it could not be staged.
    fn stage_own(&mut self, texture: &wgpu::Texture, dst: TexelDst, data: &[u8], slot: u32) -> bool {
        let mut ceiling = self.staging.ceiling.map_or(u64::MAX, |(c, _)| c);
        let before = ceiling;
        let (staging, device) = (&mut *self.staging, self.device);
        let mut last_error = None;
        let pieces = split_rows(dst.width, dst.height, &mut ceiling, |len| match staging.own_buffer(device, len) {
            Ok(b) => Some(b),
            Err(e) => {
                last_error = Some((len, e));
                None
            }
        });
        if ceiling < before {
            self.staging.ceiling = Some((ceiling, Instant::now()));
        }
        let pieces = match pieces {
            Ok(p) => p,
            Err(made) => {
                // nothing recorded for this write, and its pieces made so far
                // let go: the memory is short
                drop(made);
                self.refused = Some(last_error.unwrap_or_default());
                return false;
            }
        };
        // a buffer that came back but cannot be written (made invalid by a
        // full card without an error the scope sees, gpu_alloc.rs) is a
        // refusal too: nothing of the write is recorded, rather than a
        // texture with rows missing
        let mut views = Vec::with_capacity(pieces.len());
        for p in &pieces {
            match p.buffer.get_mapped_range_mut(..) {
                Ok(view) => views.push(view),
                Err(_) => {
                    let len = p.buffer.size();
                    drop(views);
                    drop(pieces);
                    self.refused = Some((len, "a staging buffer came back that cannot be written".to_owned()));
                    return false;
                }
            }
        }
        if pieces.len() > 1 {
            self.staged.split += 1;
        }
        let first = self.own.len();
        for (i, (p, mut view)) in pieces.iter().zip(views).enumerate() {
            stage_rows(data, dst.width, p.row, p.rows, |at, rows| {
                view.slice(at..at + rows.len()).copy_from_slice(rows);
            });
            self.textures.push(TexCopy {
                slot,
                src: Src::Own(first + i),
                offset: 0,
                texture: texture.clone(),
                dst: TexelDst {
                    y: dst.y + p.row,
                    height: p.rows,
                    ..dst
                },
            });
        }
        self.own.extend(pieces.into_iter().map(|p| p.buffer));
        true
    }
}

impl<S: WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>> WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>
    for StagedSink<'_, S>
{
    fn write_buffer(&mut self, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) {
        let len = data.len() as u64;
        self.buffer_left = self.buffer_left.saturating_sub(len);
        let aligned = len.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) && offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        if let Some(open) = &mut self.open
            && aligned
            && open.cursor + len <= open.view.len() as u64
        {
            let at = open.cursor as usize;
            open.view.slice(at..at + data.len()).copy_from_slice(data);
            push_copy(&mut self.copies, buffer, offset, open.cursor, len);
            open.cursor += len;
            self.staged.buffers += 1;
            return;
        }
        // (what was staged goes first: the queue's write lands at the next
        // submit, which must not pass it; texture copies alone can wait, they
        // touch no buffer)
        if self.open.is_some() {
            self.close(Vec::new());
        }
        self.inner.write_buffer(buffer, offset, data);
    }

    fn write_texture(&mut self, texture: &wgpu::Texture, dst: TexelDst, data: &[u8]) {
        self.write_slot_texture(texture, dst, data, NO_SLOT);
    }

    fn write_slot_texture(&mut self, texture: &wgpu::Texture, dst: TexelDst, data: &[u8], slot: u32) {
        if !self.on {
            self.inner.write_slot_texture(texture, dst, data, slot);
            return;
        }
        if dst.width == 0 || dst.height == 0 || data.len() < dst.bytes() {
            return;
        }
        if self.refused.is_some() {
            // (the memory is short: not asked again this frame)
            self.lose(slot);
            return;
        }
        let len = staged_len(dst.width, dst.height);
        if let Some(open) = &mut self.open
            && texture_fits(open.view.len() as u64, open.cursor, self.buffer_left, len)
        {
            let base = open.cursor as usize;
            stage_rows(data, dst.width, 0, dst.height, |at, rows| {
                open.view.slice(base + at..base + at + rows.len()).copy_from_slice(rows);
            });
            self.textures.push(TexCopy {
                slot,
                src: Src::Slot,
                offset: open.cursor,
                texture: texture.clone(),
                dst,
            });
            // (the next write starts 4-aligned: `len` is whole texels)
            open.cursor += len;
            self.staged.textures += 1;
            return;
        }
        if self.stage_own(texture, dst, data, slot) {
            self.staged.own += 1;
        } else {
            self.lose(slot);
        }
    }

    fn submit(&mut self, encoders: Vec<wgpu::CommandEncoder>) {
        self.close(encoders);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_that_follow_each_other_make_one_copy() {
        let mut c: Vec<CopyOp<u32>> = Vec::new();
        push_copy(&mut c, &1, 0, 0, 64);
        push_copy(&mut c, &1, 64, 64, 32);
        // another buffer, then back to the first one out of line
        push_copy(&mut c, &2, 96, 96, 16);
        push_copy(&mut c, &1, 96, 112, 16);
        // the same buffer but a gap in its range
        push_copy(&mut c, &1, 256, 128, 8);
        assert_eq!(
            c,
            vec![
                CopyOp {
                    dst: 1,
                    dst_offset: 0,
                    src_offset: 0,
                    size: 96
                },
                CopyOp {
                    dst: 2,
                    dst_offset: 96,
                    src_offset: 96,
                    size: 16
                },
                CopyOp {
                    dst: 1,
                    dst_offset: 96,
                    src_offset: 112,
                    size: 16
                },
                CopyOp {
                    dst: 1,
                    dst_offset: 256,
                    src_offset: 128,
                    size: 8
                },
            ]
        );
    }

    /// GPU round trip: a journal with writes that merge, a submit in the
    /// middle that copies what was written before it, and a write after it
    /// (through the queue), replayed frame after frame until the staging
    /// buffer made on its thread takes the writes; then read back. Needs a
    /// GPU, so not run by check.ps1:
    /// `cargo test --workspace staged_writes -- --ignored`.
    #[test]
    #[ignore]
    fn staged_writes_land_in_order_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let buffer = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 1024,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let (a, b) = (buffer(), buffer());
        let mut staging = WriteStaging::default();
        let mut frame = |n: u8| -> usize {
            let mut j = crate::writes::GpuWrites::default();
            j.write_buffer(&a, 0, &[n; 64]);
            j.write_buffer(&a, 64, &[n + 1; 32]);
            j.write_buffer(&b, 0, &[n + 2; 16]);
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(&a, 0, &b, 256, 96);
            j.submit(vec![enc]);
            j.write_buffer(&a, 0, &[n + 3; 8]);
            let mut q = crate::writes::QueueSink(&queue);
            let mut sink = staging.sink(&device, &mut q, j.buffer_bytes() as u64);
            j.replay(&mut sink);
            let staged = sink.finish();
            queue.submit([]);
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            staged
        };
        // the first frames go through the queue while the buffer is made
        let mut n = 0;
        while frame(n * 4) == 0 {
            n += 1;
            assert!(n < 60, "the staging buffer never came");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // and the next ones through the same buffer, mapped again
        let n = (n + 1) * 4;
        assert_eq!(frame(n), 3);
        let read = |buffer: &wgpu::Buffer| -> Vec<u8> {
            let out = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: crate::readback::readback_size(1024),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(buffer, 0, &out, 0, 1024);
            queue.submit([enc.finish()]);
            out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            out.get_mapped_range(..).expect("mapped").to_vec()
        };
        let (got_a, got_b) = (read(&a), read(&b));
        assert_eq!(got_a[..8], [n + 3; 8], "the write after the submit lands last");
        assert_eq!(got_a[8..64], [n; 56]);
        assert_eq!(got_a[64..96], [n + 1; 32]);
        assert_eq!(got_b[..16], [n + 2; 16]);
        // the submit saw the staged writes before it, not the one after
        assert_eq!(got_b[256..320], [n; 64]);
        assert_eq!(got_b[320..352], [n + 1; 32]);
    }

    #[test]
    fn slots_are_sized_in_powers_of_two_from_a_megabyte() {
        assert_eq!(slot_size(1), 1 << 20);
        assert_eq!(slot_size((1 << 20) + 4), 2 << 20);
        assert_eq!(slot_size(13 << 20), 16 << 20);
    }

    #[test]
    fn texture_rows_are_staged_at_the_copy_stride() {
        assert_eq!(padded_row(1), 256);
        assert_eq!(padded_row(64), 256);
        assert_eq!(padded_row(65), 512);
        assert_eq!(staged_len(65, 0), 0);
        assert_eq!(staged_len(65, 1), 260);
        assert_eq!(staged_len(65, 3), 512 * 2 + 260);
        assert_eq!(staged_len(64, 3), 768);
        // rows that need padding go one by one, at the stride: rows 1 and 2
        // of a write 2 texels wide
        let data: Vec<u8> = (0..2 * 3 * 4).map(|i| i as u8).collect();
        let mut out = vec![0xff; staged_len(2, 2) as usize];
        stage_rows(&data, 2, 1, 2, |at, rows| out[at..at + rows.len()].copy_from_slice(rows));
        assert_eq!(out[..8], data[8..16]);
        assert!(out[8..256].iter().all(|&b| b == 0xff));
        assert_eq!(out[256..264], data[16..24]);
        // rows at the stride already go in one piece
        let wide = vec![7u8; 64 * 4 * 3];
        let mut calls = Vec::new();
        stage_rows(&wide, 64, 0, 3, |at, rows| calls.push((at, rows.len())));
        assert_eq!(calls, vec![(0, 768)]);
    }

    #[test]
    fn a_refused_texture_write_is_cut_in_rows_in_order() {
        // 1000 rows of 1024 texels (4 KB each), refused from 1 MB
        let (width, rows, limit) = (1024, 1000, 1 << 20);
        let mut ceiling = u64::MAX;
        let mut asked = Vec::new();
        let pieces = split_rows(width, rows, &mut ceiling, |len| {
            asked.push(len);
            (len < limit).then_some(len)
        })
        .expect("staged");
        // every row once, in order, each piece under the refused size
        let mut next = 0;
        for p in &pieces {
            assert_eq!(p.row, next);
            assert!(p.rows > 0);
            assert_eq!(p.buffer, staged_len(width, p.rows));
            assert!(p.buffer < limit);
            next += p.rows;
        }
        assert_eq!(next, rows);
        // the smallest size refused is remembered
        let refused = asked.iter().copied().filter(|&len| len >= limit).min();
        assert_eq!(Some(ceiling), refused);
        assert_eq!(
            asked[..3],
            [staged_len(width, 1000), staged_len(width, 500), staged_len(width, 250)]
        );
        // and the next write is cut below it before asking: nothing refused
        let mut again = Vec::new();
        let pieces = split_rows(width, rows, &mut ceiling, |len| {
            again.push(len);
            (len < limit).then_some(len)
        })
        .expect("staged");
        assert!(again.iter().all(|&len| len < limit), "{again:?}");
        assert_eq!(pieces.iter().map(|p| p.rows).sum::<u32>(), rows);
    }

    #[test]
    fn a_write_refused_at_the_smallest_piece_is_not_staged() {
        // the first piece made, then everything refused
        let mut ceiling = u64::MAX;
        let mut asked = Vec::new();
        let mut made = 0;
        let r = split_rows(1024, 1000, &mut ceiling, |len| {
            asked.push(len);
            let ok = len < 1 << 20 && made == 0;
            made += usize::from(ok);
            ok.then_some(len)
        });
        // the piece made comes back, to be let go; the last size asked is
        // the smallest piece (16 rows of 4 KB)
        assert_eq!(r, Err(vec![staged_len(1024, 250)]));
        assert_eq!(asked.last(), Some(&MIN_PIECE));
        assert_eq!(ceiling, MIN_PIECE);
        // a row larger than the smallest piece is tried once, alone
        let mut asked = Vec::new();
        let mut ceiling = u64::MAX;
        let r = split_rows(32768, 4, &mut ceiling, |len| {
            asked.push(len);
            None::<u64>
        });
        assert_eq!(r, Err(vec![]));
        assert_eq!(asked, vec![staged_len(32768, 4), staged_len(32768, 2), staged_len(32768, 1)]);
    }

    #[test]
    fn a_frame_takes_the_slot_that_holds_its_textures_too() {
        let slots = [(1 << 20, true), (4 << 20, false), (8 << 20, true)];
        let pick = |need, want| pick_slot(slots.iter().copied(), need, want);
        // buffer writes of 512 KB and textures of 6 MB: the 8 MB one
        assert_eq!(pick(512 << 10, 6 << 20), Some(2));
        // textures more than any holds: the first for the buffer writes
        assert_eq!(pick(512 << 10, 16 << 20), Some(0));
        // textures only: any ready one
        assert_eq!(pick(0, 64 << 20), Some(0));
        // the one that holds them all waits for the GPU: the next one
        assert_eq!(pick(2 << 20, 3 << 20), Some(2));
        assert_eq!(pick_slot([(1 << 20, true)].into_iter(), 2 << 20, 3 << 20), None);
    }

    #[test]
    fn a_refused_slot_size_asks_next_for_one_that_holds_the_buffer_writes() {
        let (need, want) = (512 << 10, 6 << 20);
        // nothing refused: one that holds them all
        assert_eq!(slot_wanted(need, want, None, false), 8 << 20);
        // that size refused, and no slot holds the buffer writes: one that
        // does, so that they are not sent through the queue
        assert_eq!(slot_wanted(need, want, Some(8 << 20), false), 1 << 20);
        // one holds them already: the larger one, asked again later
        assert_eq!(slot_wanted(need, want, Some(8 << 20), true), 8 << 20);
        // the buffer writes' own size refused too: no smaller ask helps
        assert_eq!(slot_wanted(need, want, Some(1 << 20), false), 8 << 20);
        // only a larger size refused
        assert_eq!(slot_wanted(need, want, Some(16 << 20), false), 8 << 20);
    }

    #[test]
    fn textures_leave_room_for_the_buffer_writes_still_to_come() {
        // a 1 MB slot written to 100 KB, 600 KB of buffer writes to come
        let (size, cursor, left) = (1 << 20, 100 << 10, 600 << 10);
        assert!(texture_fits(size, cursor, left, 324 << 10));
        assert!(!texture_fits(size, cursor, left, (324 << 10) + 4));
        assert!(!texture_fits(size, cursor, u64::MAX, 4));
    }

    #[test]
    fn the_refusal_size_is_read_in_kilobytes() {
        assert_eq!(parse_refusal(Some("256")), Some(256 << 10));
        assert_eq!(parse_refusal(Some("0")), Some(0));
        assert_eq!(parse_refusal(Some("big")), None);
        assert_eq!(parse_refusal(None), None);
    }

    /// GPU round trip of texture writes: through buffers of their own while
    /// no staging buffer is ready, then through the frame's staging buffer,
    /// two writes of one frame landing in order; cut in pieces when staging
    /// buffers are refused from 100 KB; not made while they are all
    /// refused, their slots handed back each once, the texture keeping its
    /// texels, nothing carried to the next frame and the device still
    /// working. Needs a
    /// GPU, so not run by check.ps1:
    /// `cargo test --workspace staged_texture -- --ignored`.
    #[test]
    #[ignore]
    fn staged_texture_writes_land_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        // rows of 400 bytes, padded to 512 when staged: 150 KB in all
        let (w, h) = (100u32, 300u32);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let whole = TexelDst {
            mip: 0,
            x: 0,
            y: 0,
            layer: 0,
            width: w,
            height: h,
        };
        let pattern = |seed: u32| -> Vec<u8> { (0..w * h * 4).map(|i| (i.wrapping_mul(7) ^ seed) as u8).collect() };
        // each write names the texture table's slot 10 + its index
        let frame = |staging: &mut WriteStaging, writes: &[&[u8]]| -> Staged {
            let mut j = crate::writes::GpuWrites::default();
            for (i, data) in writes.iter().enumerate() {
                j.write_slot_texture(&texture, whole, data, 10 + i as u32);
            }
            let mut q = crate::writes::QueueSink(&queue);
            let mut sink = staging.sink_with_textures(&device, &mut q, j.buffer_bytes() as u64, j.texture_staged_bytes());
            j.replay(&mut sink);
            let staged = sink.finish_staged();
            queue.submit([]);
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            staged
        };
        let read = || -> Vec<u8> {
            let row = padded_row(w);
            let out = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: crate::readback::readback_size(row * u64::from(h)),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &out,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row as u32),
                        rows_per_image: Some(h),
                    },
                },
                texture.size(),
            );
            queue.submit([enc.finish()]);
            out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let mapped = out.get_mapped_range(..).expect("mapped").to_vec();
            mapped
                .chunks(row as usize)
                .take(h as usize)
                .flat_map(|r| r[..w as usize * 4].to_vec())
                .collect()
        };

        // no staging buffer yet: one of its own, and one wanted
        let mut staging = WriteStaging::with_refusal(None);
        let s = frame(&mut staging, &[&pattern(1)]);
        assert_eq!((s.textures, s.own, s.refused), (0, 1, 0));
        assert!(read() == pattern(1), "staged in a buffer of its own");
        // the frame's staging buffer, once made; two writes, the last wins
        let mut n = 0;
        loop {
            let s = frame(&mut staging, &[&pattern(2), &pattern(3)]);
            if s.textures == 2 {
                break;
            }
            n += 1;
            assert!(n < 60, "the staging buffer never came");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(read() == pattern(3), "two writes of a frame in order");

        // staging buffers refused from 100 KB: cut in two, every texel there
        let mut refused = WriteStaging::with_refusal(Some(100 << 10));
        let s = frame(&mut refused, &[&pattern(4)]);
        assert_eq!((s.own, s.split, s.refused), (1, 1, 0));
        assert!(read() == pattern(4), "staged in pieces");

        // all refused: not made, the slots handed back, and the texture
        // keeps what it had
        let mut full = WriteStaging::with_refusal(Some(0));
        let s = frame(&mut full, &[&pattern(5), &pattern(7)]);
        assert_eq!((s.own, s.refused, s.lost.clone()), (0, 2, vec![10, 11]));
        assert!(read() == pattern(4), "a refused write leaves the texels");
        // a slot written twice is handed back once; a write that names no
        // slot is counted and not handed back
        let mut j = crate::writes::GpuWrites::default();
        j.write_slot_texture(&texture, whole, &pattern(8), 12);
        j.write_slot_texture(&texture, whole, &pattern(9), 12);
        j.write_texture(&texture, whole, &pattern(9));
        let mut q = crate::writes::QueueSink(&queue);
        let mut sink = full.sink_with_textures(&device, &mut q, 0, j.texture_staged_bytes());
        j.replay(&mut sink);
        let s = sink.finish_staged();
        assert_eq!((s.refused, s.lost), (3, vec![12]));
        // nothing is carried to the next frame
        let s = frame(&mut full, &[]);
        assert_eq!((s.own, s.refused, s.lost.len()), (0, 0, 0));
        // memory again: the slot's texture sent again lands
        full.set_refusal(None);
        let s = frame(&mut full, &[&pattern(7)]);
        assert_eq!((s.own, s.refused), (1, 0));
        assert!(read() == pattern(7), "sent again once memory is back");
        // and the device still takes writes
        let s = frame(&mut staging, &[&pattern(6)]);
        assert_eq!(s.refused, 0);
        assert!(read() == pattern(6), "the device still works");
    }

    /// GPU: a texture write refused after the levels before it were staged
    /// in the same frame (mip 0 in the frame's staging buffer, mip 1 refused
    /// a buffer of its own) takes them along: the texture keeps all its old
    /// levels rather than a new one beside old ones, and its slot is handed
    /// back once. A staging buffer that comes back without its mapping is
    /// refused where it is made. Needs a GPU:
    /// `cargo test --workspace staged_texture -- --ignored`.
    #[test]
    #[ignore]
    fn staged_texture_refused_at_a_later_level_leaves_every_level() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        // mip 0 is 1 MB staged (rows of 2048 bytes), the smallest staging
        // slot: it fills it, and mip 1 (256 KB) needs a buffer of its own
        let (w, h) = (512u32, 512u32);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 2,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let dst = |mip: u32| TexelDst {
            mip,
            x: 0,
            y: 0,
            layer: 0,
            width: w >> mip,
            height: h >> mip,
        };
        let level =
            |mip: u32, seed: u32| -> Vec<u8> { (0..(w >> mip) * (h >> mip) * 4).map(|i| (i.wrapping_mul(7) ^ seed) as u8).collect() };
        let frame = |staging: &mut WriteStaging, levels: &[(u32, &[u8])]| -> Staged {
            let mut j = crate::writes::GpuWrites::default();
            for (mip, data) in levels {
                j.write_slot_texture(&texture, dst(*mip), data, 20);
            }
            let mut q = crate::writes::QueueSink(&queue);
            let mut sink = staging.sink_with_textures(&device, &mut q, j.buffer_bytes() as u64, j.texture_staged_bytes());
            j.replay(&mut sink);
            let staged = sink.finish_staged();
            queue.submit([]);
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            staged
        };
        let read = |mip: u32| -> Vec<u8> {
            let (mw, mh) = (w >> mip, h >> mip);
            let row = padded_row(mw);
            let out = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: crate::readback::readback_size(row * u64::from(mh)),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &out,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row as u32),
                        rows_per_image: Some(mh),
                    },
                },
                wgpu::Extent3d {
                    width: mw,
                    height: mh,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([enc.finish()]);
            out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let mapped = out.get_mapped_range(..).expect("mapped").to_vec();
            mapped
                .chunks(row as usize)
                .take(mh as usize)
                .flat_map(|r| r[..mw as usize * 4].to_vec())
                .collect()
        };

        // the old texels: mip 1 by the queue, mip 0 until the frame's 1 MB
        // staging buffer is there
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 1,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &level(1, 2),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((w >> 1) * 4),
                rows_per_image: Some(h >> 1),
            },
            wgpu::Extent3d {
                width: w >> 1,
                height: h >> 1,
                depth_or_array_layers: 1,
            },
        );
        let mut staging = WriteStaging::with_refusal(None);
        let mut n = 0;
        loop {
            let s = frame(&mut staging, &[(0, &level(0, 1))]);
            if s.textures == 1 {
                break;
            }
            n += 1;
            assert!(n < 60, "the staging buffer never came");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(read(0) == level(0, 1) && read(1) == level(1, 2), "the old texels");

        // every staging buffer refused from now on, none kept: mip 0 is
        // staged in the frame's buffer, mip 1 refused one of its own
        staging.set_refusal(Some(0));
        staging.spares.clear();
        let s = frame(&mut staging, &[(0, &level(0, 3)), (1, &level(1, 4))]);
        assert_eq!((s.refused, s.lost.clone()), (1, vec![20]));
        assert!(read(0) == level(0, 1), "mip 0 not made without mip 1");
        assert!(read(1) == level(1, 2), "mip 1 keeps its texels");
        // memory again: the texture sent again lands whole
        staging.set_refusal(None);
        let s = frame(&mut staging, &[(0, &level(0, 3)), (1, &level(1, 4))]);
        assert_eq!(s.refused, 0);
        assert!(read(0) == level(0, 3) && read(1) == level(1, 4), "sent again whole");

        // a staging buffer made without its mapping (here an invalid one,
        // its creation error seen by nobody) is not writable; a good one is
        let silent = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let bad = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            // (a mappable buffer cannot be a storage buffer)
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        assert!(pollster::block_on(silent.pop()).is_some(), "the creation did fail");
        assert!(writable(&bad).is_err());
        let good = try_staging(&device, "good", 256, None).expect("a staging buffer");
        assert!(writable(&good).is_ok(), "and the view taken to check it is let go");
    }

    /// GPU round trip of texture writes into part of a texture, as the
    /// texture pages take them: an offset, a layer past the first, a mip
    /// past the first, rows narrower than the copy's 256 bytes, through a
    /// buffer of their own and through the frame's staging buffer, and a
    /// write cut in pieces that starts below the first row. Needs a GPU:
    /// `cargo test --workspace staged_texture -- --ignored`.
    #[test]
    #[ignore]
    fn staged_texture_writes_land_in_part_of_a_texture() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let (w, h, layers) = (128u32, 600u32, 2u32);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: layers,
            },
            mip_level_count: 2,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let size = |mip: u32| (w >> mip, h >> mip);
        // what each (mip, layer) should hold, tightly packed
        let mut want: Vec<Vec<u8>> = (0..2 * layers)
            .map(|i| {
                let (mw, mh) = size(i / layers);
                vec![0u8; (mw * mh * 4) as usize]
            })
            .collect();
        let pattern =
            |dst: TexelDst, seed: u32| -> Vec<u8> { (0..dst.width * dst.height * 4).map(|i| (i.wrapping_mul(13) ^ seed) as u8).collect() };
        let frame = |staging: &mut WriteStaging, writes: &[(TexelDst, Vec<u8>)]| -> Staged {
            let mut j = crate::writes::GpuWrites::default();
            for (dst, data) in writes {
                j.write_texture(&texture, *dst, data);
            }
            let mut q = crate::writes::QueueSink(&queue);
            let mut sink = staging.sink_with_textures(&device, &mut q, j.buffer_bytes() as u64, j.texture_staged_bytes());
            j.replay(&mut sink);
            let staged = sink.finish_staged();
            queue.submit([]);
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            staged
        };
        let apply = |want: &mut Vec<Vec<u8>>, dst: TexelDst, data: &[u8]| {
            let (mw, _) = size(dst.mip);
            let level = &mut want[(dst.mip * layers + dst.layer) as usize];
            for r in 0..dst.height {
                let at = (((dst.y + r) * mw + dst.x) * 4) as usize;
                let from = (r * dst.width * 4) as usize;
                level[at..at + dst.width as usize * 4].copy_from_slice(&data[from..from + dst.width as usize * 4]);
            }
        };
        let read = |mip: u32, layer: u32| -> Vec<u8> {
            let (mw, mh) = size(mip);
            let row = padded_row(mw);
            let out = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: crate::readback::readback_size(row * u64::from(mh)),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &out,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row as u32),
                        rows_per_image: Some(mh),
                    },
                },
                wgpu::Extent3d {
                    width: mw,
                    height: mh,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([enc.finish()]);
            out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let mapped = out.get_mapped_range(..).expect("mapped").to_vec();
            mapped
                .chunks(row as usize)
                .take(mh as usize)
                .flat_map(|r| r[..mw as usize * 4].to_vec())
                .collect()
        };
        let check = |want: &Vec<Vec<u8>>, what: &str| {
            for mip in 0..2 {
                for layer in 0..layers {
                    assert!(
                        read(mip, layer) == want[(mip * layers + layer) as usize],
                        "{what}: mip {mip}, layer {layer}"
                    );
                }
            }
        };
        // every level cleared first, through the queue
        let mut staging = WriteStaging::with_refusal(None);
        let mut clear = Vec::new();
        for mip in 0..2 {
            for layer in 0..layers {
                let (mw, mh) = size(mip);
                let dst = TexelDst {
                    mip,
                    x: 0,
                    y: 0,
                    layer,
                    width: mw,
                    height: mh,
                };
                clear.push((dst, vec![0u8; (mw * mh * 4) as usize]));
            }
        }
        let mut q = crate::writes::QueueSink(&queue);
        for (dst, data) in &clear {
            q.write_texture(&texture, *dst, data);
        }
        queue.submit([]);
        check(&want, "cleared");

        // 37 texels wide (148 bytes a row, 256 staged) at (10, 20) of layer
        // 1, and 40 wide at (5, 7) of mip 1, layer 1: through buffers of
        // their own (no staging buffer yet), then through the frame's
        let narrow = TexelDst {
            mip: 0,
            x: 10,
            y: 20,
            layer: 1,
            width: 37,
            height: 50,
        };
        let small_mip = TexelDst {
            mip: 1,
            x: 5,
            y: 7,
            layer: 1,
            width: 40,
            height: 100,
        };
        let mut seed = 1;
        let mut through_slot = false;
        for _ in 0..60 {
            let writes = [(narrow, pattern(narrow, seed)), (small_mip, pattern(small_mip, seed + 1))];
            let s = frame(&mut staging, &writes);
            for (dst, data) in &writes {
                apply(&mut want, *dst, data);
            }
            check(
                &want,
                if s.textures == 2 {
                    "through the frame's staging buffer"
                } else {
                    "through buffers of their own"
                },
            );
            seed += 2;
            if s.textures == 2 {
                through_slot = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(through_slot, "the staging buffer never came");

        // 100 wide, 500 rows from row 40 of layer 1 at x = 8 (250 KB staged),
        // buffers refused from 200 KB: cut in two, the second from row 290
        let tall = TexelDst {
            mip: 0,
            x: 8,
            y: 40,
            layer: 1,
            width: 100,
            height: 500,
        };
        let mut refused = WriteStaging::with_refusal(Some(200 << 10));
        let data = pattern(tall, 99);
        let s = frame(&mut refused, &[(tall, data.clone())]);
        assert_eq!((s.own, s.split, s.refused), (1, 1, 0));
        apply(&mut want, tall, &data);
        check(&want, "cut in pieces below the first row");
    }

    #[test]
    fn the_test_refusal_ends_after_its_seconds() {
        assert_eq!(parse_refusal_time(Some(" 20 ")), Some(Duration::from_secs(20)));
        assert_eq!(parse_refusal_time(Some("0.5")), Some(Duration::from_millis(500)));
        assert_eq!(parse_refusal_time(Some("-1")), None);
        assert_eq!(parse_refusal_time(Some("soon")), None);
        // past what a Duration holds, or not a number: none, no panic
        assert_eq!(parse_refusal_time(Some("1e30")), None);
        assert_eq!(parse_refusal_time(Some("inf")), None);
        assert_eq!(parse_refusal_time(Some("NaN")), None);
        assert_eq!(parse_refusal_time(None), None);
        // without a time: refused for good
        let mut forever = WriteStaging::with_refusal(Some(0));
        assert_eq!(forever.refusal(), Some(0));
        assert_eq!(forever.refusal(), Some(0));
        // a time not over yet, then one over
        let mut later = WriteStaging::with_refusal(Some(64 << 10));
        later.refuse_for = Some(Duration::from_secs(3600));
        assert_eq!(later.refusal(), Some(64 << 10));
        let mut over = WriteStaging::with_refusal(Some(0));
        over.refuse_for = Some(Duration::ZERO);
        assert_eq!(over.refusal(), None);
        assert_eq!(over.refusal(), None);
    }
}
