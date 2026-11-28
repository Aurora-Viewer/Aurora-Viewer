//! The Director's film frames, read back without waiting for the GPU.
//!
//! Aurora's own (the Legacy Viewer records no film). A photo's capture is
//! waited for: the main thread holds its frame until the pixels are back
//! (`CAPTURE_WAIT`). A film asks for a frame every 1 / fps second, or every
//! frame of an offline render: waiting for each would stall the frame loop
//! on the GPU. Here the frame's picture is copied into a readback buffer,
//! its mapping begun after the submit, and the pixels handed back with a
//! later frame's result once the GPU is done (`FrameResult::films`), in the
//! order they were asked. The buffers are kept in a small ring and used
//! again: a film's frames all have one size.
//!
//! The main thread bounds the frames in flight (`Renderer::films_in_flight`):
//! a film whose encoder or GPU is behind waits, it does not queue frames.

use crate::readback::readback_size;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const MAPPING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// Readback buffers kept for the next frames of a film.
const POOL: usize = 4;

/// What a film asks of the renderer for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilmRequest {
    /// The caller's number for the frame: its answer carries it.
    pub tag: u64,
    /// `None`: the 3D view of the window's frame, before the interface (a
    /// live film). `Some`: the whole frame drawn offscreen at this size
    /// instead of the window's, never presented (an offline render), with
    /// the HUD attachments or not.
    pub size: Option<(u32, u32)>,
    pub huds: bool,
    /// An offline frame drawn as sub-frames over the time its shutter
    /// stays open (shutter.rs): this one's place among them. Only the last
    /// is read back and answered, with their average.
    pub shutter: Option<Shutter>,
}

/// A sub-frame of an offline film frame (shutter.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shutter {
    pub index: u32,
    pub count: u32,
}

impl Shutter {
    /// The frame's last sub-frame: it writes their average and is read back.
    pub fn last(&self) -> bool {
        self.index + 1 >= self.count
    }
}

/// A film frame read back: its tag, and its pixels (width, height, RGBA8),
/// `None` when it could not be drawn or read (no swapchain image, a lost
/// device, a full video memory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilmFrame {
    pub tag: u64,
    pub pixels: Option<(u32, u32, Vec<u8>)>,
}

/// How the rows of a readback are laid out, and how its texels read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub width: u32,
    pub height: u32,
    /// Bytes from a row to the next (a multiple of 256).
    pub row: u32,
    /// The texture is BGRA: red and blue swap.
    pub bgra: bool,
    /// Keep the alpha (a cut-out picture); else every pixel is opaque.
    pub cutout: bool,
}

impl Layout {
    pub fn new(width: u32, height: u32, bgra: bool, cutout: bool) -> Layout {
        Layout {
            width,
            height,
            row: (width * 4).div_ceil(256) * 256,
            bgra,
            cutout,
        }
    }

    pub fn bytes(&self) -> u64 {
        readback_size(u64::from(self.row) * u64::from(self.height))
    }
}

/// The RGBA8 pixels of a mapped readback, without its rows' padding (the
/// same reading as `Backend::read_capture`).
pub(crate) fn decode(view: &[u8], layout: Layout) -> Option<Vec<u8>> {
    let (w, h, row) = (layout.width as usize, layout.height as usize, layout.row as usize);
    if view.len() < row * h.saturating_sub(1) + w * 4 {
        return None;
    }
    let opaque = if layout.cutout { 0 } else { 255 };
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let line = &view[y * row..y * row + w * 4];
        for px in line.as_chunks::<4>().0 {
            let a = px[3] | opaque;
            if layout.bgra {
                out.extend_from_slice(&[px[2], px[1], px[0], a]);
            } else {
                out.extend_from_slice(&[px[0], px[1], px[2], a]);
            }
        }
    }
    Some(out)
}

struct Pending {
    tag: u64,
    buffer: wgpu::Buffer,
    layout: Layout,
    state: Arc<AtomicU8>,
    /// Its mapping was begun (after the frame's submit).
    mapping: bool,
}

/// The film frames on their way back, and the buffers kept for the next.
#[derive(Default)]
pub(crate) struct FilmReads {
    pending: VecDeque<Pending>,
    pool: Vec<(Layout, wgpu::Buffer)>,
    /// The film frame the frame being drawn carries, until its copy is
    /// encoded (`encode`): a frame that ends before (no swapchain image)
    /// answers it as failed.
    wanted: Option<u64>,
    /// The frame being drawn carries a film frame.
    this_frame: bool,
    answers: Vec<FilmFrame>,
}

impl FilmReads {
    /// A frame begins: the film frame it carries, if any.
    pub fn begin(&mut self, tag: Option<u64>) {
        if let Some(old) = std::mem::replace(&mut self.wanted, tag) {
            // (a frame that never reached its capture)
            self.answers.push(FilmFrame { tag: old, pixels: None });
        }
        self.this_frame = tag.is_some();
    }

    /// The frame ends, however (`Backend::result`): whether it carried a
    /// film frame; one it did not copy is answered as failed.
    pub fn end_frame(&mut self) -> bool {
        let carried = self.this_frame;
        self.begin(None);
        carried
    }

    /// The frame being drawn carries a film frame (it is drawn offscreen
    /// at the film's size when that was asked).
    pub fn carried(&self) -> bool {
        self.wanted.is_some()
    }

    /// Copy `texture` (the frame's picture) into a readback buffer; its
    /// mapping begins with `map`, after the frame's submit.
    pub fn encode(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, bgra: bool, cutout: bool) {
        let Some(tag) = self.wanted.take() else {
            return;
        };
        let layout = Layout::new(texture.width(), texture.height(), bgra, cutout);
        let kept = self.pool.iter().position(|(l, _)| l.row == layout.row && l.height == layout.height);
        let buffer = match kept {
            Some(i) => self.pool.swap_remove(i).1,
            None => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("film frame"),
                size: readback_size(layout.bytes()),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(layout.row),
                    rows_per_image: Some(layout.height),
                },
            },
            wgpu::Extent3d {
                width: layout.width,
                height: layout.height,
                depth_or_array_layers: 1,
            },
        );
        self.pending.push_back(Pending {
            tag,
            buffer,
            layout,
            state: Arc::new(AtomicU8::new(MAPPING)),
            mapping: false,
        });
    }

    /// After the frame's submit: the mappings of the copies it carried
    /// begin (a buffer cannot be submitted while a mapping is pending).
    pub fn map(&mut self) {
        for p in self.pending.iter_mut().filter(|p| !p.mapping) {
            p.mapping = true;
            let done = p.state.clone();
            p.buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                done.store(if result.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
        }
    }

    /// Some readback is on its way: the device is to be polled.
    pub fn busy(&self) -> bool {
        self.pending.iter().any(|p| p.mapping)
    }

    /// The frames whose pixels are back, in the order they were asked
    /// (and those that failed). Never waits.
    pub fn answers(&mut self) -> Vec<FilmFrame> {
        while let Some(front) = self.pending.front() {
            if !front.mapping {
                break;
            }
            match front.state.load(Ordering::Acquire) {
                MAPPED => {
                    let Some(p) = self.pending.pop_front() else {
                        break;
                    };
                    let pixels = p
                        .buffer
                        .get_mapped_range(..)
                        .ok()
                        .and_then(|view| decode(&view, p.layout))
                        .map(|rgba| (p.layout.width, p.layout.height, rgba));
                    p.buffer.unmap();
                    if pixels.is_some() {
                        // the next frame of the film takes it again
                        self.pool.retain(|(l, _)| l.row == p.layout.row && l.height == p.layout.height);
                        if self.pool.len() < POOL {
                            self.pool.push((p.layout, p.buffer));
                        }
                    }
                    self.answers.push(FilmFrame { tag: p.tag, pixels });
                }
                FAILED => {
                    if let Some(p) = self.pending.pop_front() {
                        self.answers.push(FilmFrame { tag: p.tag, pixels: None });
                    }
                }
                _ => break,
            }
        }
        std::mem::take(&mut self.answers)
    }
}

/// An overlay's rectangle (`OverlayEffect::texture`'s `y`, `z` and `w`): its
/// corners in the frame, 0 to 1 from the top left, as 16-bit fractions; `w`
/// is 1 for a rectangle, 0 for the whole frame (RLVa's overlays).
pub fn overlay_rect(min: [f32; 2], max: [f32; 2]) -> [u32; 3] {
    let unorm = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u32;
    [unorm(min[0]) | (unorm(min[1]) << 16), unorm(max[0]) | (unorm(max[1]) << 16), 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_readback_is_read_without_its_rows_padding() {
        // 3 x 2 pixels: rows of 12 bytes padded to 256
        let layout = Layout::new(3, 2, true, false);
        assert_eq!(layout.row, 256);
        assert!(layout.bytes() >= 512);
        let mut view = vec![0u8; 512];
        for y in 0..2 {
            for x in 0..3 {
                let at = y * 256 + x * 4;
                view[at..at + 4].copy_from_slice(&[10 + x as u8, 20, 30 + y as u8, 7]);
            }
        }
        let rgba = decode(&view, layout).expect("pixels");
        assert_eq!(rgba.len(), 3 * 2 * 4);
        // BGRA to RGBA, opaque
        assert_eq!(&rgba[..4], &[30, 20, 10, 255]);
        assert_eq!(&rgba[20..24], &[31, 20, 12, 255]);
        // a cut-out keeps its alpha, an RGBA texture its order
        let keep = decode(&view, Layout::new(3, 2, false, true)).expect("pixels");
        assert_eq!(&keep[..4], &[10, 20, 30, 7]);
        // a view too short is no picture
        assert_eq!(decode(&view[..267], layout), None);
        assert!(decode(&view[..268], layout).is_some(), "the last row needs no padding");
    }

    #[test]
    fn a_frame_that_never_reached_its_capture_is_answered_as_failed() {
        let mut reads = FilmReads::default();
        reads.begin(Some(4));
        assert!(reads.carried());
        // the frame ends before its capture (no swapchain image)
        assert!(reads.end_frame());
        assert!(!reads.carried());
        assert!(!reads.end_frame());
        assert_eq!(reads.answers(), [FilmFrame { tag: 4, pixels: None }]);
        assert!(reads.answers().is_empty());
        assert!(!reads.busy());
    }

    #[test]
    fn an_overlay_rectangle_packs_its_corners() {
        let [a, b, flag] = overlay_rect([0.0, 0.5], [1.0, 2.0]);
        assert_eq!(flag, 1);
        assert_eq!(a & 0xffff, 0);
        assert_eq!(a >> 16, 32768);
        // (clamped to the frame)
        assert_eq!(b, 65535 | (65535 << 16));
    }
}
