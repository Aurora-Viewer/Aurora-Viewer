//! The poser's part under the cursor, read from the meshes (Aurora's own):
//! the joint that carries the most skinning weight on the subject's surface
//! seen at a pixel.
//!
//! The part the poser offers with Ctrl must be the one drawn under the
//! pointer, as the overlay draws it from the meshes (`fs_part_mask`): the
//! bones alone miss a skirt, hair, a fitted body's volume. While a pick is
//! asked, the subject's skinned faces (`part_faces`) are drawn once more at
//! the display's size, with the scene's depth test (so what stands in front
//! of them, another avatar or an object, hides them), each pixel holding
//! its dominant palette joint + 1 (`fs_part_pick`); the texel under the
//! cursor alone is copied and read back a frame or two later, as
//! drawn_pick.rs does. Only that pixel is rasterized (a scissor); nothing
//! is drawn while nobody asks, and the target is freed a few seconds after
//! the last pick.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The dominant joint + 1 of a pixel (0: none of the subject).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// One texel read back, in a row the copy accepts.
const ROW: u64 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
const SLOTS: usize = 3;
/// Frames without a pick before its target is freed (about 4 s at 60 fps).
const IDLE_FRAMES: u32 = 240;
const IDLE: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    pending: bool,
    /// The window pixel asked, given back with the answer: the poser trusts
    /// it only while the cursor is still there.
    tag: [u32; 2],
}

/// A window pixel on a target drawn at another size (a lower render scale,
/// RLVa's divisor): the texel that shows it.
pub fn target_pixel([x, y]: [u32; 2], window: [u32; 2], target: [u32; 2]) -> [u32; 2] {
    let scale = |v: u32, from: u32, to: u32| (u64::from(v) * u64::from(to) / u64::from(from.max(1))) as u32;
    [scale(x, window[0], target[0]), scale(y, window[1], target[1])]
}

pub struct PartPick {
    texture: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    slots: Vec<Slot>,
    /// The slot and texel this frame copies: mapped once submitted.
    encoded: Option<(usize, [u32; 2])>,
    /// The last answer read back: the dominant joint + 1 (0 for none) and
    /// the window pixel it was read at.
    answer: Option<(u32, [u32; 2])>,
    /// Frames since the last pick (`idle`).
    idle: u32,
}

impl PartPick {
    pub fn new(device: &wgpu::Device) -> PartPick {
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("poser part pick readback"),
                    size: crate::readback::readback_size(ROW),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(IDLE)),
                pending: false,
                tag: [0, 0],
            })
            .collect();
        PartPick {
            texture: None,
            slots,
            encoded: None,
            answer: None,
            idle: 0,
        }
    }

    /// The target a pick at `size` needs (`make_target`), when the one
    /// held is missing or of another size: the renderer allocates it under
    /// its video memory guard (`extra`) and hands it over (`set_target`).
    pub fn needs_target(&self, size: [u32; 2]) -> bool {
        self.texture.as_ref().is_none_or(|(_, _, at)| *at != size)
    }

    pub fn make_target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poser part pick"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// The target allocated for `size` (None: refused, the pick waits).
    pub fn set_target(&mut self, texture: Option<wgpu::Texture>, size: [u32; 2]) {
        self.texture = texture.map(|t| {
            let view = t.create_view(&Default::default());
            (t, view, size)
        });
        self.idle = 0;
    }

    /// A frame without a pick: the target goes after `IDLE_FRAMES` of them
    /// (a few seconds), so nothing stays allocated while nobody asks.
    pub fn idle(&mut self) {
        self.idle = self.idle.saturating_add(1);
        if self.idle > IDLE_FRAMES && self.encoded.is_none() {
            self.texture = None;
        }
    }

    /// The target for this frame's pick of `pixel` at `size` (`tag`: the
    /// window pixel it shows), if it is allocated and a readback is free
    /// (the pick waits a frame otherwise): its view.
    pub fn begin(&mut self, size: [u32; 2], pixel: [u32; 2], tag: [u32; 2]) -> Option<&wgpu::TextureView> {
        if pixel[0] >= size[0] || pixel[1] >= size[1] || self.needs_target(size) {
            return None;
        }
        let slot = self.slots.iter().position(|s| !s.pending)?;
        self.slots[slot].pending = true;
        self.slots[slot].tag = tag;
        self.encoded = Some((slot, pixel));
        self.idle = 0;
        self.texture.as_ref().map(|(_, view, _)| view)
    }

    /// Copy the texel under the cursor to the readback begun this frame
    /// (after the pass).
    pub fn encode_copy(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some((slot, [x, y])), Some((texture, _, _))) = (self.encoded, &self.texture) else {
            return;
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.slots[slot].buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW as u32),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// A frame that cannot be drawn gives its slot back.
    pub fn abandon(&mut self) {
        if let Some((slot, _)) = self.encoded.take() {
            self.slots[slot].pending = false;
        }
    }

    /// Map the readback copied this frame, once its commands are submitted.
    pub fn after_submit(&mut self) {
        if let Some((slot, _)) = self.encoded.take() {
            let s = &self.slots[slot];
            let state = s.state.clone();
            s.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
        }
    }

    /// Read what the GPU delivered (after a poll of the device): the newest
    /// answer wins.
    pub fn harvest(&mut self) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            if !s.pending || self.encoded.is_some_and(|(slot, _)| slot == i) {
                continue;
            }
            let joint = match s.state.load(Ordering::Acquire) {
                MAPPED => {
                    let joint = s
                        .buffer
                        .get_mapped_range(..)
                        .ok()
                        .and_then(|view| view.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
                    s.buffer.unmap();
                    joint
                }
                FAILED => None,
                _ => continue,
            };
            s.state.store(IDLE, Ordering::Release);
            s.pending = false;
            if let Some(joint) = joint {
                self.answer = Some((joint, s.tag));
            }
        }
    }

    /// The newest answer since the last call, with the window pixel it was
    /// read at: 0 when nothing of the subject is there, joint + 1 otherwise.
    pub fn take_answer(&mut self) -> Option<(u32, [u32; 2])> {
        self.answer.take()
    }
}

#[cfg(test)]
mod tests {
    use super::target_pixel;

    #[test]
    fn a_window_pixel_lands_on_the_texel_that_shows_it_at_any_render_scale() {
        // full scale: the same pixel
        assert_eq!(target_pixel([1000, 500], [1920, 1080], [1920, 1080]), [1000, 500]);
        // half scale: the chest at (1000, 500) is the target's (500, 250), not
        // outside a 960 x 540 target
        assert_eq!(target_pixel([1000, 500], [1920, 1080], [960, 540]), [500, 250]);
        // the far corner stays inside
        assert_eq!(target_pixel([1919, 1079], [1920, 1080], [960, 540]), [959, 539]);
        // a zero-sized window gives no division by zero
        assert_eq!(target_pixel([3, 4], [0, 0], [10, 10]), [30, 40]);
    }
}
