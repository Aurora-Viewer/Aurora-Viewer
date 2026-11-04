//! One staging buffer for the buffer writes of a frame's journal.
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
//! copies recorded before it in front of its own commands, and the writes
//! after it go through the queue as before (rare). Texture writes still go
//! through the queue: they touch no buffer.
//!
//! A few staging buffers take turns: a used one is mapped again
//! (`map_async`) and comes back once the GPU has done its copies. When none
//! is ready (the GPU is several frames behind, or the frame writes more
//! than any of them holds: one of the right size is then made for the
//! frames after) or a frame writes more than [`MAX_STAGED`] bytes, the
//! frame's writes go through the queue. At most `MAX_SLOTS` buffers of
//! [`MAX_STAGED`] bytes stay allocated (a busy region: 1 to 16 MB each).
//! `AURORA_WRITE_STAGING=0` keeps the queue writes, `alt` alternates.

use crate::writes::{TexelDst, WriteSink};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

/// Staging buffers taking turns: one being written, the others waiting for
/// the GPU's copies of the frames before.
const MAX_SLOTS: usize = 3;
/// Smallest staging buffer (bytes).
const MIN_SIZE: u64 = 1 << 20;
/// A frame that writes more than this (a region's arrival) goes through the
/// queue: a staging buffer that large would stay allocated.
pub const MAX_STAGED: u64 = 32 << 20;

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

struct Slot {
    buffer: wgpu::Buffer,
    /// Mapped, and no copy of a submitted frame reads it any more.
    ready: Arc<AtomicBool>,
}

/// The staging buffers of the render thread.
#[derive(Default)]
pub struct WriteStaging {
    slots: Vec<Slot>,
    /// A larger staging buffer being made on a thread of its own.
    making: Option<mpsc::Receiver<wgpu::Buffer>>,
    /// One made, waiting for a slot to take.
    made: Option<wgpu::Buffer>,
    /// The size of one to make after this frame's writes.
    wanted: Option<u64>,
}

/// Maps `buffer` again; `ready` once it is (after the GPU's last use).
fn remap(buffer: &wgpu::Buffer, ready: &Arc<AtomicBool>) {
    let ready = ready.clone();
    buffer.map_async(wgpu::MapMode::Write, .., move |r| ready.store(r.is_ok(), Ordering::Release));
}

impl WriteStaging {
    /// Starts making the staging buffer wanted, on a thread of its own.
    fn make_wanted(&mut self, device: &wgpu::Device) {
        let Some(size) = self.wanted.take() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let device = device.clone();
        let spawned = std::thread::Builder::new().name("journal staging".into()).spawn(move || {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("journal staging"),
                size,
                usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            });
            let _ = tx.send(buffer);
        });
        match spawned {
            Ok(_) => self.making = Some(rx),
            Err(e) => log::warn!("journal staging: no thread to make a staging buffer ({e})"),
        }
    }

    /// A ready slot of at least `bytes`, or None to use the queue this
    /// frame. A staging buffer is made mapped, and wgpu fills a buffer
    /// mapped at its creation with zeros on the CPU (7.7 ms for 16 MB at
    /// the demo crowd's arrival): it is made on a thread of its own and
    /// joins the slots when it is there. (Zeroing it on the GPU is not
    /// allowed: a mappable buffer cannot be a copy destination.)
    fn acquire(&mut self, bytes: u64) -> Option<usize> {
        let ready = |s: &Slot| s.ready.load(Ordering::Acquire);
        let arrived = match self.making.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(buffer)) => {
                self.making = None;
                Some(buffer)
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.making = None;
                None
            }
            _ => None,
        };
        if let Some(buffer) = self.made.take().or(arrived) {
            let size = buffer.size();
            let slot = Slot {
                buffer,
                ready: Arc::new(AtomicBool::new(true)),
            };
            // one more, or in place of a ready one smaller
            if self.slots.len() < MAX_SLOTS {
                self.slots.push(slot);
            } else if let Some(i) = self.slots.iter().position(|s| ready(s) && s.buffer.size() < size) {
                self.slots[i] = slot;
            } else {
                self.made = Some(slot.buffer);
            }
        }
        if let Some(i) = self.slots.iter().position(|s| ready(s) && s.buffer.size() >= bytes) {
            return Some(i);
        }
        // none fits, or they all wait for the GPU and there is room for one
        // more: one is made for the frames after, once this frame's writes
        // are done (the zeros and the queue's copies share the memory
        // bandwidth: made before them, the demo crowd's second arrival frame
        // took 4.3 ms of writes, 2.7 made after, 1.4 without staging; the
        // frames after are 1.5 times faster)
        let fits = self.slots.iter().any(|s| s.buffer.size() >= bytes);
        if self.making.is_none() && self.made.is_none() && (!fits || self.slots.len() < MAX_SLOTS) {
            self.wanted = Some(slot_size(bytes));
        }
        None
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
        let open = if bytes > 0 && bytes <= MAX_STAGED && staging_on() {
            self.acquire(bytes).and_then(|slot| {
                let s = &self.slots[slot];
                let view = s.buffer.get_mapped_range_mut(..).ok()?;
                s.ready.store(false, Ordering::Release);
                Some(Open { slot, view, cursor: 0 })
            })
        } else {
            None
        };
        StagedSink {
            staging: self,
            device,
            inner,
            open,
            copies: Vec::new(),
            staged: 0,
        }
    }
}

struct Open {
    slot: usize,
    view: wgpu::BufferViewMut,
    cursor: u64,
}

/// A journal replay that stages its buffer writes (see the module).
pub struct StagedSink<'a, S> {
    staging: &'a mut WriteStaging,
    device: &'a wgpu::Device,
    inner: &'a mut S,
    open: Option<Open>,
    copies: Vec<CopyOp<wgpu::Buffer>>,
    /// Buffer writes staged (diagnostics).
    staged: usize,
}

impl<S: WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>> StagedSink<'_, S> {
    /// Submits the copies staged so far, in front of `then`, and sends the
    /// staging buffer to be mapped again; the writes after go through the
    /// queue.
    fn close(&mut self, then: Vec<wgpu::CommandEncoder>) {
        let Some(open) = self.open.take() else {
            if !then.is_empty() {
                self.inner.submit(then);
            }
            return;
        };
        drop(open.view);
        let slot = &self.staging.slots[open.slot];
        if self.copies.is_empty() {
            // nothing written: it is still mapped
            slot.ready.store(true, Ordering::Release);
            if !then.is_empty() {
                self.inner.submit(then);
            }
            return;
        }
        slot.buffer.unmap();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("journal copies"),
        });
        for c in self.copies.drain(..) {
            enc.copy_buffer_to_buffer(&slot.buffer, c.src_offset, &c.dst, c.dst_offset, c.size);
        }
        self.inner.submit(std::iter::once(enc).chain(then).collect());
        remap(&slot.buffer, &slot.ready);
    }

    /// The end of the replay: the staged copies submitted. The number of
    /// buffer writes staged.
    pub fn finish(mut self) -> usize {
        self.close(Vec::new());
        self.staging.make_wanted(self.device);
        self.staged
    }
}

impl<S: WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>> WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>
    for StagedSink<'_, S>
{
    fn write_buffer(&mut self, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) {
        let len = data.len() as u64;
        let aligned = len.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) && offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        if let Some(open) = &mut self.open
            && aligned
            && open.cursor + len <= open.view.len() as u64
        {
            let at = open.cursor as usize;
            open.view.slice(at..at + data.len()).copy_from_slice(data);
            push_copy(&mut self.copies, buffer, offset, open.cursor, len);
            open.cursor += len;
            self.staged += 1;
            return;
        }
        // (what was staged goes first: the queue's write lands at the next
        // submit, which must not pass it)
        self.close(Vec::new());
        self.inner.write_buffer(buffer, offset, data);
    }

    fn write_texture(&mut self, texture: &wgpu::Texture, dst: TexelDst, data: &[u8]) {
        self.inner.write_texture(texture, dst, data);
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
                size: 1024,
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
}
