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
//! drawn_pick.rs does. Nothing is drawn nor allocated while nobody asks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The dominant joint + 1 of a pixel (0: none of the subject).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// One texel read back, in a row the copy accepts.
const ROW: u64 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
const SLOTS: usize = 3;
const IDLE: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    pending: bool,
}

pub struct PartPick {
    texture: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    slots: Vec<Slot>,
    /// The slot and texel this frame copies: mapped once submitted.
    encoded: Option<(usize, [u32; 2])>,
    /// The last answer read back: the dominant joint + 1, 0 for none.
    answer: Option<u32>,
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
            })
            .collect();
        PartPick {
            texture: None,
            slots,
            encoded: None,
            answer: None,
        }
    }

    /// The target for this frame's pick of `pixel` at `size`, if a readback
    /// is free (the pick waits a frame otherwise): its view.
    pub fn begin(&mut self, device: &wgpu::Device, size: [u32; 2], pixel: [u32; 2]) -> Option<&wgpu::TextureView> {
        if pixel[0] >= size[0] || pixel[1] >= size[1] {
            return None;
        }
        let slot = self.slots.iter().position(|s| !s.pending)?;
        if self.texture.as_ref().is_none_or(|(_, _, at)| *at != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
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
            });
            let view = texture.create_view(&Default::default());
            self.texture = Some((texture, view, size));
        }
        self.slots[slot].pending = true;
        self.encoded = Some((slot, pixel));
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
                self.answer = Some(joint);
            }
        }
    }

    /// The newest answer since the last call: Some(0) when nothing of the
    /// subject is under the cursor, Some(joint + 1) otherwise.
    pub fn take_answer(&mut self) -> Option<u32> {
        self.answer.take()
    }
}
