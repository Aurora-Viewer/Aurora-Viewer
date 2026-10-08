//! Bindless texture table: every texture lives in a slot of one big
//! `binding_array<texture_2d<f32>>`; draw records reference slots by index.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

/// Reserved slots.
pub const WHITE: u32 = 0;
pub const FLAT_NORMAL: u32 = 1;
pub const BLACK: u32 = 2;
pub const TRANSPARENT: u32 = 3;
const RESERVED: u32 = 4;

struct Slot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bytes: u64,
}

pub struct TextureTable {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    slots: Vec<Option<Slot>>,
    free: Vec<u32>,
    capacity: u32,
    dirty: bool,
    last_rebuild: Instant,
    bytes: u64,
    live: u32,
}

/// Mip level data, tightly packed RGBA8.
pub struct MipLevel<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
}

fn solid(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4], label: &str) -> Slot {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Slot { texture, view, bytes: 4 }
}

impl TextureTable {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, capacity: u32, anisotropy: u16) -> Self {
        let capacity = capacity.max(RESERVED + 1);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: NonZeroU32::new(capacity),
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = Self::make_sampler(device, anisotropy);
        let mut slots: Vec<Option<Slot>> = Vec::with_capacity(capacity as usize);
        slots.push(Some(solid(device, queue, [255, 255, 255, 255], "white")));
        slots.push(Some(solid(device, queue, [128, 128, 255, 255], "flat normal")));
        slots.push(Some(solid(device, queue, [0, 0, 0, 255], "black")));
        slots.push(Some(solid(device, queue, [0, 0, 0, 0], "transparent")));
        let bind_group = Self::build(device, &layout, &sampler, &slots, capacity);
        Self {
            layout,
            bind_group,
            sampler,
            slots,
            free: Vec::new(),
            capacity,
            dirty: false,
            last_rebuild: Instant::now(),
            bytes: 0,
            live: 0,
        }
    }

    fn make_sampler(device: &wgpu::Device, anisotropy: u16) -> wgpu::Sampler {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
            compare: None,
            anisotropy_clamp: anisotropy.clamp(1, 16),
            border_color: None,
        })
    }

    /// Change the anisotropic filtering level (1 = off, up to 16).
    pub fn set_anisotropy(&mut self, device: &wgpu::Device, anisotropy: u16) {
        self.sampler = Self::make_sampler(device, anisotropy);
        self.dirty = true;
        self.maintain(device, true);
    }

    fn build(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        slots: &[Option<Slot>],
        capacity: u32,
    ) -> wgpu::BindGroup {
        let white = match &slots[0] {
            Some(s) => &s.view,
            None => unreachable!("white slot always present"),
        };
        let views: Vec<&wgpu::TextureView> = (0..capacity as usize)
            .map(|i| match slots.get(i) {
                Some(Some(s)) => &s.view,
                _ => white,
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("textures"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(&views),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    pub fn live(&self) -> u32 {
        self.live
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn is_full(&self) -> bool {
        self.free.is_empty() && self.slots.len() as u32 >= self.capacity
    }

    /// Create a texture from a full mip chain (level 0 first). Returns the slot.
    pub fn create(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mips: &[MipLevel]) -> Option<u32> {
        let first = mips.first()?;
        if first.width == 0 || first.height == 0 {
            return None;
        }
        let max_dim = device.limits().max_texture_dimension_2d;
        if first.width > max_dim || first.height > max_dim {
            return None;
        }
        let slot = match self.free.pop() {
            Some(s) => s,
            None => {
                if self.slots.len() as u32 >= self.capacity {
                    return None;
                }
                self.slots.push(None);
                (self.slots.len() - 1) as u32
            }
        };
        let s = self.make(device, queue, mips);
        self.bytes += s.bytes;
        self.live += 1;
        self.slots[slot as usize] = Some(s);
        self.dirty = true;
        Some(slot)
    }

    /// Replace the contents of an existing slot (e.g. higher resolution).
    pub fn replace(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, slot: u32, mips: &[MipLevel]) -> bool {
        if slot < RESERVED || mips.is_empty() {
            return false;
        }
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return false;
        };
        let s = Self::make_static(device, queue, mips);
        if let Some(old) = entry.take() {
            // dropped, not destroyed: the current bind group may still use it
            self.bytes = self.bytes.saturating_sub(old.bytes);
        } else {
            self.live += 1;
        }
        self.bytes += s.bytes;
        *entry = Some(s);
        self.dirty = true;
        true
    }

    /// Size of the texture in a slot (level 0).
    pub fn size_of(&self, slot: u32) -> Option<(u32, u32)> {
        let s = self.slots.get(slot as usize)?.as_ref()?;
        Some((s.texture.width(), s.texture.height()))
    }

    /// Overwrite a rectangle of level 0 in place (media textures updated
    /// every frame: no new texture, no bind group rebuild). `data` holds
    /// `h` rows of `w` RGBA8 pixels.
    pub fn write_region(&self, queue: &wgpu::Queue, slot: u32, x: u32, y: u32, w: u32, h: u32, data: &[u8]) -> bool {
        let Some(Some(s)) = self.slots.get(slot as usize) else {
            return false;
        };
        if slot < RESERVED
            || w == 0
            || h == 0
            || x + w > s.texture.width()
            || y + h > s.texture.height()
            || data.len() < (w * h * 4) as usize
        {
            return false;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &s.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &data[..(w * h * 4) as usize],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        true
    }

    pub fn free(&mut self, slot: u32) {
        if slot < RESERVED {
            return;
        }
        if let Some(entry) = self.slots.get_mut(slot as usize)
            && let Some(old) = entry.take()
        {
            self.bytes = self.bytes.saturating_sub(old.bytes);
            self.live = self.live.saturating_sub(1);
            // Keep the texture alive until the bind group is rebuilt: the
            // old bind group holds a reference, so dropping is safe.
            drop(old);
            self.free.push(slot);
            self.dirty = true;
        }
    }

    fn make(&self, device: &wgpu::Device, queue: &wgpu::Queue, mips: &[MipLevel]) -> Slot {
        Self::make_static(device, queue, mips)
    }

    fn make_static(device: &wgpu::Device, queue: &wgpu::Queue, mips: &[MipLevel]) -> Slot {
        let w = mips[0].width;
        let h = mips[0].height;
        // Only keep levels that form a valid chain.
        let mut count = 0u32;
        for (i, m) in mips.iter().enumerate() {
            let ew = (w >> i).max(1);
            let eh = (h >> i).max(1);
            if m.width != ew || m.height != eh || m.data.len() < (ew * eh * 4) as usize {
                break;
            }
            count += 1;
        }
        let count = count.max(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut bytes = 0u64;
        for (i, m) in mips.iter().take(count as usize).enumerate() {
            let len = (m.width * m.height * 4) as usize;
            if m.data.len() < len {
                break;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: i as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &m.data[..len],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(m.width * 4),
                    rows_per_image: Some(m.height),
                },
                wgpu::Extent3d {
                    width: m.width,
                    height: m.height,
                    depth_or_array_layers: 1,
                },
            );
            bytes += len as u64;
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Slot { texture, view, bytes }
    }

    /// Rebuild the bind group if slots changed (throttled).
    pub fn maintain(&mut self, device: &wgpu::Device, force: bool) {
        if !self.dirty {
            return;
        }
        if !force && self.last_rebuild.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.bind_group = Self::build(device, &self.layout, &self.sampler, &self.slots, self.capacity);
        self.dirty = false;
        self.last_rebuild = Instant::now();
    }
}

/// Generate a box-filtered mip chain from RGBA8 data (level 0 included).
pub fn build_mips(width: u32, height: u32, data: Vec<u8>, max_levels: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let (mut w, mut h) = (width, height);
    out.push((w, h, data));
    while (w > 1 || h > 1) && (out.len() as u32) < max_levels {
        let nw = (w / 2).max(1);
        let nh = (h / 2).max(1);
        let Some((_, _, prev)) = out.last() else {
            break;
        };
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let x0 = (x * 2).min(w - 1);
                let x1 = (x * 2 + 1).min(w - 1);
                let y0 = (y * 2).min(h - 1);
                let y1 = (y * 2 + 1).min(h - 1);
                let idx = |xx: u32, yy: u32| ((yy * w + xx) * 4) as usize;
                let (a, b, c, d) = (idx(x0, y0), idx(x1, y0), idx(x0, y1), idx(x1, y1));
                // alpha-weighted color average to avoid dark fringes
                let wa = prev[a + 3] as u32;
                let wb = prev[b + 3] as u32;
                let wc = prev[c + 3] as u32;
                let wd = prev[d + 3] as u32;
                let wsum = wa + wb + wc + wd;
                let o = ((y * nw + x) * 4) as usize;
                for ch in 0..3 {
                    let weighted = prev[a + ch] as u32 * wa
                        + prev[b + ch] as u32 * wb
                        + prev[c + ch] as u32 * wc
                        + prev[d + ch] as u32 * wd
                        + wsum / 2;
                    let v = weighted
                        .checked_div(wsum)
                        .unwrap_or((prev[a + ch] as u32 + prev[b + ch] as u32 + prev[c + ch] as u32 + prev[d + ch] as u32 + 2) / 4);
                    next[o + ch] = v as u8;
                }
                next[o + 3] = ((wsum + 2) / 4) as u8;
            }
        }
        out.push((nw, nh, next));
        w = nw;
        h = nh;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_chain() {
        let m = build_mips(8, 4, vec![255u8; 8 * 4 * 4], 16);
        let dims: Vec<_> = m.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(dims, vec![(8, 4), (4, 2), (2, 1), (1, 1)]);
        assert!(m.iter().all(|(w, h, d)| d.len() == (w * h * 4) as usize));
        assert_eq!(m[3].2, vec![255, 255, 255, 255]);
    }
}
