//! A picture of the computer drawn through photo looks (the Picture viewer,
//! `ui/picture_viewer.rs` of the viewer; Aurora's own). The same looks as the
//! photo window's (`looks.rs`), but on a picture rather than the 3D view: the
//! picture is the looks' input, their passes draw into a target of its own,
//! shown in egui like the previews (`preview.rs`, an egui callback that
//! becomes a mesh of the target). The target is drawn again only when the
//! picture or the look changes. To save, the full-size picture is drawn the
//! same way and read back without a wait: copied in the pass that asks,
//! mapped at the next pass of the same window (its commands are submitted by
//! then, so the renderer needs no hook of its own), sent to the window by a
//! worker thread (as `thumbnail.rs`).

use crate::looks::{Looks, PhotoLook};
use crate::preview::{Image, Shower, Shown};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Weak};

/// The pictures are sRGB, and so are the targets: the looks work on linear
/// light between the two, as on the frame.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The largest side drawn (a larger picture is refused, its reader told).
pub const MAX_SIDE: u32 = 8192;

const ENCODED: u8 = 0;
const WAITING: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

/// The picture drawn through a look, read back for its caller (`tag`).
pub struct GradedImage {
    pub tag: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA8, rows going down.
    pub rgba: Vec<u8>,
}

/// A full-size picture to draw through the look and read back (to save it).
pub struct Read {
    pub tag: u64,
    pub image: Arc<Image>,
    pub reply: Sender<Result<GradedImage, String>>,
}

/// A picture and its look, to draw in an egui rectangle (`paint`). `id`
/// names its target; `read`, once, asks for a full-size copy.
#[derive(Clone)]
pub struct Graded {
    pub id: egui::Id,
    pub image: Arc<Image>,
    pub look: PhotoLook,
    pub read: Option<Arc<Read>>,
}

struct Region(Graded);

/// Adds the picture drawn through its look to `painter`, over `rect`.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, graded: Graded) {
    painter.add(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(Region(graded)),
    }));
}

/// Whether two callbacks of one place show the same graded picture. `None`:
/// not both graded pictures.
pub(crate) fn same(a: &egui::PaintCallback, b: &egui::PaintCallback) -> Option<bool> {
    let (a, b) = (&a.callback.downcast_ref::<Region>()?.0, &b.callback.downcast_ref::<Region>()?.0);
    Some(a.id == b.id && Arc::ptr_eq(&a.image, &b.image) && a.look == b.look && a.read.is_none() && b.read.is_none())
}

/// The look of `size` pictures, whole (its passes placed on the picture).
pub fn whole(mut look: PhotoLook, size: [u32; 2]) -> PhotoLook {
    look.image = [0.0, 0.0, size[0] as f32, size[1] as f32];
    look.scale = 1.0;
    look.compare = None;
    look.hold = false;
    look
}

struct Target {
    size: [u32; 2],
    view: wgpu::TextureView,
    texture: egui::TextureId,
    /// What it shows: the picture (weak, so that its address is not taken
    /// by another) and the look.
    last: Option<(Weak<Image>, PhotoLook)>,
}

struct Flight {
    tag: u64,
    /// The window whose pass encoded the copy: mapped at its next pass.
    shower: Shower,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    row: u32,
    state: Arc<AtomicU8>,
    reply: Sender<Result<GradedImage, String>>,
}

/// The graded pictures' targets and reads in flight, owned by
/// `preview::Cache`.
#[derive(Default)]
pub(crate) struct Cache {
    looks: Option<Looks>,
    targets: HashMap<egui::Id, Target>,
    seen: HashSet<egui::Id>,
    shown: Shown<egui::Id>,
    flights: Vec<Flight>,
    /// The reads already started (a callback repeated is not read twice).
    read: HashSet<u64>,
    /// The window of the pass under way.
    pass: Option<Shower>,
}

impl Cache {
    /// A pass of `shower` begins: the copies its last pass encoded are
    /// submitted now and can be mapped; those mapped are sent.
    pub(crate) fn begin(&mut self, shower: Shower) {
        self.pass = Some(shower);
        for flight in &self.flights {
            if flight.shower != shower || flight.state.load(Ordering::Acquire) != ENCODED {
                continue;
            }
            flight.state.store(WAITING, Ordering::Release);
            let state = flight.state.clone();
            flight.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
        }
        self.deliver();
    }

    /// The mesh that shows `region` in `rect`, its target drawn first when
    /// the picture or the look changed. One drawing of the looks a frame:
    /// a read asked for goes first, the picture on screen waits a frame.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        egui: &mut egui_wgpu::Renderer,
        callback: &egui::PaintCallback,
        drawn: &mut bool,
    ) -> Option<egui::Mesh> {
        let Region(g) = callback.callback.downcast_ref::<Region>()?;
        self.seen.insert(g.id);
        if let Some(read) = &g.read
            && !*drawn
            && self.read.insert(read.tag)
        {
            *drawn = true;
            self.read(device, queue, encoder, read, &g.look);
        }
        let size = [g.image.size[0].clamp(1, MAX_SIDE), g.image.size[1].clamp(1, MAX_SIDE)];
        if self.targets.get(&g.id).is_none_or(|t| t.size != size) {
            if let Some(old) = self.targets.remove(&g.id) {
                egui.free_texture(&old.texture);
            }
            let view = texture(device, "picture look", size, wgpu::TextureUsages::TEXTURE_BINDING);
            let texture = egui.register_native_texture(device, &view, wgpu::FilterMode::Linear);
            self.targets.insert(
                g.id,
                Target {
                    size,
                    view,
                    texture,
                    last: None,
                },
            );
        }
        let target = self.targets.get_mut(&g.id)?;
        let current = target
            .last
            .as_ref()
            .is_some_and(|(image, look)| *look == g.look && image.upgrade().is_some_and(|a| Arc::ptr_eq(&a, &g.image)));
        if !current && !*drawn && g.image.size == size {
            *drawn = true;
            let looks = self.looks.get_or_insert_with(|| Looks::new(device));
            grade(device, queue, encoder, looks, &g.image, &g.look, &target.view);
            target.last = Some((Arc::downgrade(&g.image), g.look.clone()));
        }
        let mut mesh = egui::Mesh::with_texture(target.texture);
        mesh.add_rect_with_uv(
            callback.rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        Some(mesh)
    }

    /// Draws `read`'s picture through `look` and copies it for reading.
    fn read(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, read: &Read, look: &PhotoLook) {
        let [width, height] = read.image.size;
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            let _ = read.reply.send(Err(format!("{width}x{height} is larger than {MAX_SIDE} pixels")));
            return;
        }
        let looks = self.looks.get_or_insert_with(|| Looks::new(device));
        let target = texture(device, "picture look read", [width, height], wgpu::TextureUsages::COPY_SRC);
        let look = whole(look.clone(), [width, height]);
        grade(device, queue, encoder, looks, &read.image, &look, &target);
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picture look read"),
            size: crate::readback::readback_size(u64::from(row) * u64::from(height)),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.texture().as_image_copy(),
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
            tag: read.tag,
            shower: self.pass.unwrap_or(Shower::Viewer),
            buffer,
            width,
            height,
            row,
            state: Arc::new(AtomicU8::new(ENCODED)),
            reply: read.reply.clone(),
        });
    }

    /// Sends the copies the GPU is through with, from a worker thread.
    fn deliver(&mut self) {
        let mut index = 0;
        while index < self.flights.len() {
            match self.flights[index].state.load(Ordering::Acquire) {
                MAPPED => {
                    let flight = self.flights.swap_remove(index);
                    let spawned = std::thread::Builder::new().name("aurora-picture-read".into()).spawn(move || {
                        let rgba = flight
                            .buffer
                            .get_mapped_range(..)
                            .ok()
                            .map(|view| unpad(&view, flight.width, flight.height, flight.row));
                        flight.buffer.unmap();
                        let answer = match rgba {
                            Some(rgba) => Ok(GradedImage {
                                tag: flight.tag,
                                width: flight.width,
                                height: flight.height,
                                rgba,
                            }),
                            None => Err("the picture could not be read back".into()),
                        };
                        let _ = flight.reply.send(answer);
                    });
                    if let Err(e) = spawned {
                        log::warn!("picture look: no worker thread ({e})");
                    }
                }
                FAILED => {
                    let flight = self.flights.swap_remove(index);
                    let _ = flight.reply.send(Err("the GPU could not give the picture back".into()));
                }
                _ => index += 1,
            }
        }
    }

    /// The pass of `shower` is over: what it drew is kept.
    pub(crate) fn show(&mut self, shower: Shower) {
        self.shown.set(shower, std::mem::take(&mut self.seen));
        self.pass = None;
    }

    pub(crate) fn keep_outside(&mut self, alive: impl Fn(u64) -> bool) -> bool {
        self.shown.keep_outside(alive)
    }

    /// Forgets the targets no window shows; the looks' own images go with
    /// the last one.
    pub(crate) fn retain(&mut self, egui: &mut egui_wgpu::Renderer) {
        let shown = &self.shown;
        self.targets.retain(|id, target| {
            if shown.has(id) {
                true
            } else {
                egui.free_texture(&target.texture);
                false
            }
        });
        if self.targets.is_empty() && self.flights.is_empty() {
            self.looks = None;
            self.read.clear();
        }
    }
}

fn texture(device: &wgpu::Device, label: &str, size: [u32; 2], usage: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// `image` written into the looks' input, then their passes into `target`.
fn grade(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    looks: &mut Looks,
    image: &Image,
    look: &PhotoLook,
    target: &wgpu::TextureView,
) {
    let [width, height] = image.size;
    looks.prepare(device, FORMAT, (width, height));
    let Some(input) = looks.input() else { return };
    let extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let bytes = (width as usize) * (height as usize) * 4;
    if image.rgba.len() < bytes {
        return;
    }
    queue.write_texture(
        input.texture().as_image_copy(),
        &image.rgba[..bytes],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        extent,
    );
    looks.encode(device, queue, encoder, &whole(look.clone(), image.size), target, None);
}

/// Rows of `row` bytes made rows of `width` pixels. A buffer too short gives
/// what it holds, the rest transparent.
fn unpad(src: &[u8], width: u32, height: u32, row: u32) -> Vec<u8> {
    let line = width as usize * 4;
    let mut out = vec![0u8; line * height as usize];
    for (y, dst) in out.chunks_exact_mut(line).enumerate() {
        let start = y * row as usize;
        if let Some(src) = src.get(start..start + line) {
            dst.copy_from_slice(src);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_rows_are_cut_to_the_picture() {
        let (width, height, row) = (2u32, 2u32, 256u32);
        let mut src = vec![9u8; (row * height) as usize];
        src[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        src[256..264].copy_from_slice(&[10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(
            unpad(&src, width, height, row),
            [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15, 16, 17]
        );
        assert_eq!(unpad(&src[..100], width, height, row)[8..], [0; 8], "short: transparent, no panic");
    }

    #[test]
    fn a_whole_look_covers_the_picture() {
        let look = PhotoLook {
            layers: Vec::new(),
            compare: Some(0.5),
            image: [10.0, 20.0, 5.0, 5.0],
            scale: 0.3,
            hold: true,
            focus: None,
        };
        let w = whole(look, [640, 480]);
        assert_eq!(w.image, [0.0, 0.0, 640.0, 480.0]);
        assert_eq!((w.scale, w.compare, w.hold), (1.0, None, false));
    }
}
