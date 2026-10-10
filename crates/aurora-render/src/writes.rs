//! Write journal of a frame.
//!
//! `Queue::write_buffer` / `write_texture` take effect at the next
//! `Queue::submit`, whoever calls it. With the frame encoded and submitted
//! by the render thread while the main thread already prepares the next one
//! (render_thread.rs), a write made directly by the main thread for frame
//! N+1 would land in frame N: records moved before their palette, geometry
//! overwritten under draws that still read the old mesh.
//!
//! So the main thread never touches the queue. Everything it would have
//! written or submitted is recorded here, in order, with the frame being
//! built; the render thread replays the journal of frame N right before it
//! encodes frame N. The replay makes exactly the queue calls the main
//! thread used to make, in the same order, so uploads keep their place
//! relative to each other and to the draws that use them (a copy submitted
//! mid-frame, as an arena growth, still sees the writes recorded before it
//! and not those recorded after).
//!
//! The bytes of every write live in one buffer reused from frame to frame.

use std::ops::Range;

/// Where tightly packed RGBA8 rows go in a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexelDst {
    pub mip: u32,
    pub x: u32,
    pub y: u32,
    pub layer: u32,
    pub width: u32,
    pub height: u32,
}

impl TexelDst {
    /// Bytes of the rectangle.
    pub fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// One recorded queue call.
#[derive(Debug)]
enum Op<B, T, E> {
    Buffer { buffer: B, offset: u64, data: Range<usize> },
    Texture { texture: T, dst: TexelDst, data: Range<usize> },
    /// Copies submitted before the end of the frame (an arena growth, with
    /// the staged copies recorded before it): they must see the writes
    /// above and none of those below.
    Submit(Vec<E>),
}

/// What replays a journal: the queue, or a recorder in the tests.
pub trait WriteSink<B, T, E> {
    fn write_buffer(&mut self, buffer: &B, offset: u64, data: &[u8]);
    fn write_texture(&mut self, texture: &T, dst: TexelDst, data: &[u8]);
    fn submit(&mut self, encoders: Vec<E>);
}

/// Ordered queue calls of one frame, generic over the GPU handles so the
/// ordering is tested without a device.
#[derive(Debug)]
pub struct Journal<B, T, E> {
    ops: Vec<Op<B, T, E>>,
    data: Vec<u8>,
}

impl<B, T, E> Default for Journal<B, T, E> {
    fn default() -> Self {
        Journal {
            ops: Vec::new(),
            data: Vec::new(),
        }
    }
}

impl<B: Clone, T: Clone, E> Journal<B, T, E> {
    fn push_data(&mut self, data: &[u8]) -> Range<usize> {
        let start = self.data.len();
        self.data.extend_from_slice(data);
        start..self.data.len()
    }

    /// As `Queue::write_buffer`. Nothing is recorded for empty data.
    pub fn write_buffer(&mut self, buffer: &B, offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let data = self.push_data(data);
        self.ops.push(Op::Buffer {
            buffer: buffer.clone(),
            offset,
            data,
        });
    }

    /// As `Queue::write_texture` for RGBA8 rows of `dst.width` pixels;
    /// `data` must hold the whole rectangle (longer data is cut).
    pub fn write_texture(&mut self, texture: &T, dst: TexelDst, data: &[u8]) {
        let Some(data) = data.get(..dst.bytes()).filter(|d| !d.is_empty()) else {
            return;
        };
        let data = self.push_data(data);
        self.ops.push(Op::Texture {
            texture: texture.clone(),
            dst,
            data,
        });
    }

    /// As `Queue::submit`, at this point of the frame.
    pub fn submit(&mut self, encoders: Vec<E>) {
        if !encoders.is_empty() {
            self.ops.push(Op::Submit(encoders));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Bytes recorded since the last replay.
    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    /// Make the recorded calls, in order; the journal comes back empty with
    /// its memory kept for the next frame.
    pub fn replay(&mut self, sink: &mut impl WriteSink<B, T, E>) {
        for op in self.ops.drain(..) {
            match op {
                Op::Buffer { buffer, offset, data } => sink.write_buffer(&buffer, offset, &self.data[data]),
                Op::Texture { texture, dst, data } => sink.write_texture(&texture, dst, &self.data[data]),
                Op::Submit(encoders) => sink.submit(encoders),
            }
        }
        self.data.clear();
    }
}

/// The journal of the renderer: what the main thread records for a frame.
pub type GpuWrites = Journal<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder>;

/// Replays a journal on the queue (render thread).
pub struct QueueSink<'a>(pub &'a wgpu::Queue);

impl WriteSink<wgpu::Buffer, wgpu::Texture, wgpu::CommandEncoder> for QueueSink<'_> {
    fn write_buffer(&mut self, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) {
        self.0.write_buffer(buffer, offset, data);
    }

    fn write_texture(&mut self, texture: &wgpu::Texture, dst: TexelDst, data: &[u8]) {
        self.0.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: dst.mip,
                origin: wgpu::Origin3d {
                    x: dst.x,
                    y: dst.y,
                    z: dst.layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dst.width * 4),
                rows_per_image: Some(dst.height),
            },
            wgpu::Extent3d {
                width: dst.width,
                height: dst.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn submit(&mut self, encoders: Vec<wgpu::CommandEncoder>) {
        self.0.submit(encoders.into_iter().map(|e| e.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a replay did, in order.
    #[derive(Debug, PartialEq)]
    enum Call {
        Buffer(u32, u64, Vec<u8>),
        Texture(u32, TexelDst, Vec<u8>),
        Submit(Vec<&'static str>),
    }

    #[derive(Default)]
    struct Recorder(Vec<Call>);

    impl WriteSink<u32, u32, &'static str> for Recorder {
        fn write_buffer(&mut self, buffer: &u32, offset: u64, data: &[u8]) {
            self.0.push(Call::Buffer(*buffer, offset, data.to_vec()));
        }

        fn write_texture(&mut self, texture: &u32, dst: TexelDst, data: &[u8]) {
            self.0.push(Call::Texture(*texture, dst, data.to_vec()));
        }

        fn submit(&mut self, encoders: Vec<&'static str>) {
            self.0.push(Call::Submit(encoders));
        }
    }

    fn texel(layer: u32) -> TexelDst {
        TexelDst {
            mip: 0,
            x: 0,
            y: 0,
            layer,
            width: 1,
            height: 2,
        }
    }

    #[test]
    fn replay_keeps_the_recorded_order() {
        let mut j: Journal<u32, u32, &'static str> = Journal::default();
        // a mesh written, then the arena grows (copy of the old buffer,
        // after the staged copies recorded so far), then a write into the
        // new buffer: the growth must see the first write and not the last
        j.write_buffer(&1, 16, &[1, 2, 3, 4]);
        j.submit(vec!["staged copies", "grow"]);
        j.write_buffer(&2, 32, &[5, 6]);
        j.write_texture(&7, texel(3), &[9; 8]);
        assert_eq!(j.bytes(), 14);
        let mut r = Recorder::default();
        j.replay(&mut r);
        assert_eq!(
            r.0,
            vec![
                Call::Buffer(1, 16, vec![1, 2, 3, 4]),
                Call::Submit(vec!["staged copies", "grow"]),
                Call::Buffer(2, 32, vec![5, 6]),
                Call::Texture(7, texel(3), vec![9; 8]),
            ]
        );
    }

    #[test]
    fn a_replayed_journal_is_empty_and_reusable() {
        let mut j: Journal<u32, u32, &'static str> = Journal::default();
        j.write_buffer(&1, 0, &[1; 64]);
        let mut r = Recorder::default();
        j.replay(&mut r);
        assert!(j.is_empty());
        assert_eq!(j.bytes(), 0);
        // the next frame starts from offset 0 of the same memory
        let cap = j.data.capacity();
        j.write_buffer(&2, 8, &[2; 64]);
        assert_eq!(j.data.capacity(), cap);
        j.replay(&mut r);
        assert_eq!(r.0[1], Call::Buffer(2, 8, vec![2; 64]));
        // nothing recorded twice
        j.replay(&mut r);
        assert_eq!(r.0.len(), 2);
    }

    #[test]
    fn empty_calls_are_not_recorded() {
        let mut j: Journal<u32, u32, &'static str> = Journal::default();
        j.write_buffer(&1, 0, &[]);
        j.submit(Vec::new());
        // data shorter than the rectangle: refused, as the texture table
        // checks before; longer: cut to the rectangle
        j.write_texture(&1, texel(0), &[0; 7]);
        assert!(j.is_empty());
        j.write_texture(&1, texel(0), &[4; 12]);
        let mut r = Recorder::default();
        j.replay(&mut r);
        assert_eq!(r.0, vec![Call::Texture(1, texel(0), vec![4; 8])]);
    }

    #[test]
    fn writes_of_two_frames_never_mix() {
        // frame N is handed over (its journal taken), frame N+1 records
        // while N is still to be replayed: N's replay holds only N's writes
        let mut building: Journal<u32, u32, &'static str> = Journal::default();
        building.write_buffer(&1, 0, &[10]);
        let mut frame_n = std::mem::take(&mut building);
        building.write_buffer(&1, 0, &[11]);
        let mut r = Recorder::default();
        frame_n.replay(&mut r);
        assert_eq!(r.0, vec![Call::Buffer(1, 0, vec![10])]);
        building.replay(&mut r);
        assert_eq!(r.0[1], Call::Buffer(1, 0, vec![11]));
    }
}
