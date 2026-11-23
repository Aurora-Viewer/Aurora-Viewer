//! Small pictures of the 3D view without the interface (the Director's
//! keyframe thumbnails): the view is copied into a buffer of its own on the
//! frame that asks, mapped without waiting, and made small on a worker
//! thread once the GPU is through with it. The answer comes a few frames
//! later, tagged with the number its caller gave: never a wait on the GPU,
//! neither on the main thread nor on the render thread. Aurora's own.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// The largest picture made: the view fits in it, its proportions kept.
pub const THUMBNAIL_MAX: (u32, u32) = (192, 108);
/// Copies in flight at once: the pictures are asked for by a click, never
/// each frame; past this a request is dropped (its caller shows nothing).
const IN_FLIGHT: usize = 4;

const ENCODED: u8 = 0;
const WAITING: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

/// A small picture of the view, as its caller asked it (`tag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub tag: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA8, rows of `width` pixels, opaque.
    pub rgba: Vec<u8>,
}

struct Flight {
    tag: u64,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    row: u32,
    bgra: bool,
    state: Arc<AtomicU8>,
}

pub(crate) struct Thumbnails {
    flights: Vec<Flight>,
    tx: Sender<Thumbnail>,
    rx: Receiver<Thumbnail>,
}

impl Thumbnails {
    pub(crate) fn new() -> Thumbnails {
        let (tx, rx) = channel();
        Thumbnails {
            flights: Vec::new(),
            tx,
            rx,
        }
    }

    /// Copies `texture` (the view of `width` by `height`, in `format`) for
    /// the picture `tag`. Nothing when the copies in flight are too many,
    /// or the format is not 8-bit RGBA or BGRA.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        tag: u64,
    ) {
        use wgpu::TextureFormat as F;
        let bgra = match format {
            F::Bgra8Unorm | F::Bgra8UnormSrgb => true,
            F::Rgba8Unorm | F::Rgba8UnormSrgb => false,
            other => {
                log::debug!("thumbnail: a view in {other:?} is not read back");
                return;
            }
        };
        if self.flights.len() >= IN_FLIGHT || width == 0 || height == 0 {
            log::debug!("thumbnail {tag} dropped: {} copies in flight", self.flights.len());
            return;
        }
        let row = (width * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("thumbnail"),
            size: crate::readback::readback_size(u64::from(row) * u64::from(height)),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
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
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.flights.push(Flight {
            tag,
            buffer,
            width,
            height,
            row,
            bgra,
            state: Arc::new(AtomicU8::new(ENCODED)),
        });
    }

    /// Maps the copies encoded this frame, once its commands are submitted.
    pub(crate) fn after_submit(&mut self) {
        for flight in &self.flights {
            if flight.state.load(Ordering::Acquire) != ENCODED {
                continue;
            }
            flight.state.store(WAITING, Ordering::Release);
            let state = flight.state.clone();
            flight.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
        }
    }

    /// The pictures made since the last call. A copy the GPU is through
    /// with goes to a worker, which makes it small and unmaps it.
    pub(crate) fn take(&mut self) -> Vec<Thumbnail> {
        let mut index = 0;
        while index < self.flights.len() {
            match self.flights[index].state.load(Ordering::Acquire) {
                MAPPED => {
                    let flight = self.flights.swap_remove(index);
                    let tx = self.tx.clone();
                    let spawned = std::thread::Builder::new().name("aurora-thumbnail".into()).spawn(move || {
                        let picture = flight.buffer.get_mapped_range(..).ok().map(|view| {
                            let (width, height, rgba) =
                                downscale(&view, flight.width, flight.height, flight.row, flight.bgra, THUMBNAIL_MAX);
                            Thumbnail {
                                tag: flight.tag,
                                width,
                                height,
                                rgba,
                            }
                        });
                        flight.buffer.unmap();
                        if let Some(picture) = picture {
                            let _ = tx.send(picture);
                        }
                    });
                    if let Err(e) = spawned {
                        log::warn!("thumbnail: no worker thread ({e})");
                    }
                }
                FAILED => {
                    self.flights.swap_remove(index);
                }
                _ => index += 1,
            }
        }
        self.rx.try_iter().collect()
    }
}

/// The size of a picture of `width` by `height` that fits in `max`, its
/// proportions kept, never larger than the picture.
pub fn fitted(width: u32, height: u32, max: (u32, u32)) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let scale = (max.0 as f32 / width as f32).min(max.1 as f32 / height as f32).min(1.0);
    (
        ((width as f32 * scale).round() as u32).clamp(1, width),
        ((height as f32 * scale).round() as u32).clamp(1, height),
    )
}

/// The picture `src` (rows of `row` bytes, `width` by `height` pixels of
/// RGBA or BGRA) made small to fit in `max`: each pixel the mean of the
/// pixels it covers (a box filter), opaque RGBA. A buffer too short for its
/// size gives an empty picture rather than a panic.
pub fn downscale(src: &[u8], width: u32, height: u32, row: u32, bgra: bool, max: (u32, u32)) -> (u32, u32, Vec<u8>) {
    let (w, h) = fitted(width, height, max);
    let needed = u64::from(row) * u64::from(height.saturating_sub(1)) + u64::from(width) * 4;
    if w == 0 || h == 0 || row < width * 4 || (src.len() as u64) < needed {
        return (0, 0, Vec::new());
    }
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for ty in 0..h {
        let (y0, y1) = (ty * height / h, ((ty + 1) * height / h).max(ty * height / h + 1));
        for tx in 0..w {
            let (x0, x1) = (tx * width / w, ((tx + 1) * width / w).max(tx * width / w + 1));
            let mut sum = [0u32; 3];
            for y in y0..y1 {
                let line = (y * row) as usize;
                for x in x0..x1 {
                    let at = line + (x * 4) as usize;
                    for (s, c) in sum.iter_mut().zip(&src[at..at + 3]) {
                        *s += u32::from(*c);
                    }
                }
            }
            let count = (y1 - y0) * (x1 - x0);
            let [a, b, c] = sum.map(|s| ((s + count / 2) / count) as u8);
            out.extend_from_slice(&if bgra { [c, b, a, 255] } else { [a, b, c, 255] });
        }
    }
    (w, h, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_fits_in_the_thumbnail_and_keeps_its_proportions() {
        assert_eq!(fitted(1920, 1080, THUMBNAIL_MAX), (192, 108));
        assert_eq!(fitted(1080, 1920, THUMBNAIL_MAX), (61, 108));
        assert_eq!(fitted(100, 50, THUMBNAIL_MAX), (100, 50), "never larger");
        assert_eq!(fitted(0, 50, THUMBNAIL_MAX), (0, 0));
        assert_eq!(fitted(10_000, 1, THUMBNAIL_MAX), (192, 1));
    }

    #[test]
    fn each_pixel_is_the_mean_of_those_it_covers() {
        // 4 by 2, BGRA, rows padded to 32 bytes: left half black, right white
        let (width, height, row) = (4, 2, 32);
        let mut src = vec![0u8; (row * height) as usize];
        for y in 0..height {
            for x in 2..4 {
                let at = (y * row + x * 4) as usize;
                src[at..at + 4].copy_from_slice(&[255, 255, 255, 0]);
            }
        }
        let (w, h, out) = downscale(&src, width, height, row, true, (2, 1));
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, [0, 0, 0, 255, 255, 255, 255, 255]);
        // BGRA becomes RGBA
        let blue = [255u8, 0, 0, 0];
        let (_, _, out) = downscale(&blue, 1, 1, 4, true, (8, 8));
        assert_eq!(out, [0, 0, 255, 255]);
        let (_, _, out) = downscale(&blue, 1, 1, 4, false, (8, 8));
        assert_eq!(out, [255, 0, 0, 255]);
        // a mean of grey levels
        let src = [10u8, 10, 10, 0, 20, 20, 20, 0];
        assert_eq!(downscale(&src, 2, 1, 8, false, (1, 1)).2, [15, 15, 15, 255]);
    }

    #[test]
    fn a_short_buffer_gives_no_picture() {
        assert_eq!(downscale(&[0; 7], 2, 1, 8, false, (1, 1)), (0, 0, Vec::new()));
        assert_eq!(downscale(&[0; 64], 4, 2, 8, false, (2, 1)).0, 0, "rows shorter than the width");
    }
}
