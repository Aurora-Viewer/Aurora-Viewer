//! What is drawn under a pixel, read back from the GPU: the draw record of
//! the nearest opaque or masked face, and of the nearest blended face in
//! front of it, each with its point on the surface.
//!
//! The depth under the cursor (pick.rs) says where the surface is, not
//! whose it is. For most objects the scene finds that out on the CPU, from
//! triangles it keeps; it cannot for a rigged mesh, which only the GPU
//! poses: its triangles are known in their bind pose, somewhere else than
//! where they are shown. Firestorm skins the mesh again on the CPU to pick
//! it (LLVOVolume::lineSegmentIntersect, updateRiggedVolume) and leaves it
//! out of most clicks because of that cost.
//!
//! Here the frame that carries a request draws the faces of the view once
//! more, with the vertex shader of the scene (so posed, placed and animated
//! exactly as shown) and a camera narrowed to the pixel asked and its right
//! and lower neighbours: a 2 × 2 target whose texels hold the record, the
//! depth and the surface coordinates of the face there. Two passes: opaque
//! and masked faces (alpha tested as displayed), then blended faces in
//! front of them (including zero-alpha texels for interactions; visible
//! texture texels only for the camera). Nothing is drawn nor allocated
//! per frame when nobody asks: the cost is one pass over the visible
//! geometry, vertex work only, in the frame of a click.
//! The red/blue transparency overlays also enter the second pass with
//! their full triangles, as displayed, even for camera picks.
//!
//! The neighbours give the geometric normal of the surface and the
//! direction of its texture coordinates (the binormal of a touch) when
//! they lie on the same face.

use crate::pick::PickPixel;
use glam::{Mat4, Vec2, Vec3};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Record, depth and surface coordinates of a texel.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
/// The pixel asked, its right and its lower neighbour (and the diagonal).
const SIZE: u32 = 2;
const TEXEL: u64 = 16;
/// `copy_texture_to_buffer` rows are 256 bytes apart.
const ROW: u64 = 256;
/// One target: two rows.
const TARGET_BYTES: u64 = ROW * SIZE as u64;
/// Requests in flight; one more waits for a later frame.
const SLOTS: usize = 3;
const IDLE: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// A face drawn under a pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnFace {
    /// Its draw record.
    pub record: u32,
    /// Where on it, in the world.
    pub point: Vec3,
    /// Surface coordinates there, before the texture transform.
    pub st: Vec2,
    /// Geometric normal toward the eye and direction of increasing `st.y`
    /// across the surface; zero at an edge of the face.
    pub normal: Vec3,
    pub binormal: Vec3,
}

/// What is drawn under a pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drawn {
    /// The nearest opaque or alpha-masked face.
    pub opaque: Option<DrawnFace>,
    /// The nearest alpha-blended face in front of it.
    pub blend: Option<DrawnFace>,
}

/// Touch/build includes invisible Blend texels; the camera follows visible
/// surfaces (LLToolCamera::handleMouseDown: pick_transparent = false).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawnMode {
    Interaction,
    Visible,
}

/// A request: its ticket, pixel and transparency rule.
pub type DrawnRequest = (u64, u32, u32, DrawnMode);
pub type DrawnAnswer = (u64, Drawn);

/// The camera of a frame narrowed to the 2 × 2 pixels at (x, y): applied
/// after its projection, in clip space.
pub fn narrow(x: u32, y: u32, width: u32, height: u32) -> Mat4 {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    // centre of the block in normalized device coordinates (y up)
    let cx = (x as f32 + 1.0) / w * 2.0 - 1.0;
    let cy = 1.0 - (y as f32 + 1.0) / h * 2.0;
    let (sx, sy) = (w / SIZE as f32, h / SIZE as f32);
    Mat4::from_cols(
        glam::Vec4::new(sx, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, sy, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
        glam::Vec4::new(-cx * sx, -cy * sy, 0.0, 1.0),
    )
}

/// One texel of a target: record + 1 (0: nothing drawn), depth, st.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Texel {
    record: u32,
    depth: f32,
    st: Vec2,
}

fn texel(bytes: &[u8], col: u64, row: u64) -> Texel {
    let at = (row * ROW + col * TEXEL) as usize;
    let word = |i: usize| {
        bytes
            .get(at + i * 4..at + i * 4 + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    Texel {
        record: word(0),
        depth: f32::from_bits(word(1)),
        st: Vec2::new(f32::from_bits(word(2)), f32::from_bits(word(3))),
    }
}

/// The face of a target's texels: `px` is the pixel asked.
fn face(bytes: &[u8], px: PickPixel) -> Option<DrawnFace> {
    let at = texel(bytes, 0, 0);
    if at.record == 0 {
        return None;
    }
    let point = px.world(at.depth)?;
    let (right, down) = (texel(bytes, 1, 0), texel(bytes, 0, 1));
    let mut face = DrawnFace {
        record: at.record - 1,
        point,
        st: at.st,
        normal: Vec3::ZERO,
        binormal: Vec3::ZERO,
    };
    // the same face on both neighbours: the surface around the point
    if right.record == at.record && down.record == at.record {
        let world = |dx: u32, dy: u32, t: Texel| {
            PickPixel {
                x: px.x + dx,
                y: px.y + dy,
                ..px
            }
            .world(t.depth)
        };
        if let (Some(pr), Some(pd), Some(eye)) = (world(1, 0, right), world(0, 1, down), px.world(1.0)) {
            let (e1, e2) = (pr - point, pd - point);
            let n = e1.cross(e2).normalize_or_zero();
            face.normal = if n.dot(eye - point) < 0.0 { -n } else { n };
            // direction along which st.y grows (media/pick.rs does the same
            // from the triangle's corners)
            let (d1, d2) = (right.st - at.st, down.st - at.st);
            let det = d1.x * d2.y - d2.x * d1.y;
            if det.abs() > 1e-12 {
                face.binormal = ((e2 * d1.x - e1 * d2.x) / det).normalize_or_zero();
            }
        }
    }
    Some(face)
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    pending: Option<(u64, PickPixel)>,
}

pub struct DrawnPick {
    /// Opaque and blended targets, and the depth they share.
    textures: [wgpu::Texture; 2],
    pub views: [wgpu::TextureView; 2],
    pub depth: wgpu::TextureView,
    slots: Vec<Slot>,
    queue: Vec<DrawnRequest>,
    /// The slot whose copy this frame encoded: to map once submitted.
    encoded: Option<usize>,
    answers: Vec<DrawnAnswer>,
}

impl DrawnPick {
    pub fn new(device: &wgpu::Device, depth_format: wgpu::TextureFormat) -> DrawnPick {
        let texture = |label: &str, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let textures = [
            texture("drawn pick opaque", FORMAT, color),
            texture("drawn pick blend", FORMAT, color),
        ];
        let views = [
            textures[0].create_view(&Default::default()),
            textures[1].create_view(&Default::default()),
        ];
        let depth = texture("drawn pick depth", depth_format, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("drawn pick readback"),
                    size: TARGET_BYTES * 2,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(IDLE)),
                pending: None,
            })
            .collect();
        DrawnPick {
            textures,
            views,
            depth,
            slots,
            queue: Vec::new(),
            encoded: None,
            answers: Vec::new(),
        }
    }

    /// The requests of a frame join those still waiting.
    pub fn ask(&mut self, asked: Vec<DrawnRequest>) {
        self.queue.extend(asked);
    }

    /// The request this frame draws for, if any and if a readback is free:
    /// its pixel. One a frame; the others wait for the next.
    pub fn begin(&mut self, size: (u32, u32), inv_view_proj: Mat4) -> Option<(u32, u32, DrawnMode)> {
        while let Some(&(ticket, x, y, mode)) = self.queue.first() {
            if x >= size.0 || y >= size.1 {
                self.queue.remove(0);
                self.answers.push((ticket, Drawn::default()));
                continue;
            }
            let i = self.slots.iter().position(|s| s.pending.is_none())?;
            self.queue.remove(0);
            self.slots[i].pending = Some((
                ticket,
                PickPixel {
                    x,
                    y,
                    width: size.0,
                    height: size.1,
                    inv_view_proj,
                },
            ));
            self.encoded = Some(i);
            return Some((x, y, mode));
        }
        None
    }

    /// A frame that cannot be drawn answers what it began with nothing.
    pub fn abandon(&mut self) {
        if let Some(i) = self.encoded.take()
            && let Some((ticket, _)) = self.slots[i].pending.take()
        {
            self.answers.push((ticket, Drawn::default()));
        }
    }

    /// Copy both targets of the request begun this frame to its readback
    /// (after the two passes).
    pub fn encode_copy(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(i) = self.encoded else {
            return;
        };
        for (k, texture) in self.textures.iter().enumerate() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.slots[i].buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: k as u64 * TARGET_BYTES,
                        bytes_per_row: Some(ROW as u32),
                        rows_per_image: Some(SIZE),
                    },
                },
                wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Map the readback copied this frame, once its commands are submitted.
    pub fn after_submit(&mut self) {
        if let Some(i) = self.encoded.take() {
            let s = &self.slots[i];
            let state = s.state.clone();
            s.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
        }
    }

    /// Read the answers the GPU has delivered (after a poll of the device).
    pub fn harvest(&mut self) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            let Some((ticket, px)) = s.pending else {
                continue;
            };
            if self.encoded == Some(i) {
                continue;
            }
            let drawn = match s.state.load(Ordering::Acquire) {
                MAPPED => {
                    let drawn = s.buffer.get_mapped_range(..).ok().map(|view| {
                        let (opaque, blend) = view.split_at((TARGET_BYTES as usize).min(view.len()));
                        Drawn {
                            opaque: face(opaque, px),
                            blend: face(blend, px),
                        }
                    });
                    s.buffer.unmap();
                    drawn.unwrap_or_default()
                }
                FAILED => Drawn::default(),
                _ => continue,
            };
            s.state.store(IDLE, Ordering::Release);
            s.pending = None;
            self.answers.push((ticket, drawn));
        }
    }

    pub fn take_answers(&mut self) -> Vec<DrawnAnswer> {
        std::mem::take(&mut self.answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> (Mat4, Mat4) {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 800.0 / 600.0, 0.1);
        let view = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, -5.0, 1.5), Vec3::new(0.0, 0.0, 1.0), Vec3::Z);
        (proj, view)
    }

    /// Pixel centre of a clip-space position in a target of this size.
    fn pixel(clip: glam::Vec4, w: f32, h: f32) -> Vec2 {
        let ndc = clip.truncate() / clip.w;
        Vec2::new((ndc.x * 0.5 + 0.5) * w, (0.5 - ndc.y * 0.5) * h)
    }

    #[test]
    fn the_narrowed_camera_shows_the_pixel_asked_and_its_neighbours() {
        let (proj, view) = camera();
        let vp = proj * view;
        let (w, h) = (800u32, 600u32);
        for (x, y) in [(0u32, 0u32), (417, 233), (799, 599), (12, 580)] {
            let m = narrow(x, y, w, h);
            // a point seen at the centre of pixel (x + dx, y + dy) lands at
            // the centre of texel (dx, dy) of the 2 x 2 target
            for (dx, dy) in [(0u32, 0u32), (1, 0), (0, 1)] {
                let px = PickPixel {
                    x: x + dx,
                    y: y + dy,
                    width: w,
                    height: h,
                    inv_view_proj: vp.inverse(),
                };
                let p = px.world(0.02).expect("a point in front of the camera");
                let screen = pixel(vp * p.extend(1.0), w as f32, h as f32);
                assert!(
                    (screen - Vec2::new((x + dx) as f32 + 0.5, (y + dy) as f32 + 0.5)).length() < 0.02,
                    "{screen:?}"
                );
                let narrowed = pixel(m * vp * p.extend(1.0), SIZE as f32, SIZE as f32);
                assert!(
                    (narrowed - Vec2::new(dx as f32 + 0.5, dy as f32 + 0.5)).length() < 0.02,
                    "pixel {x},{y} +{dx},{dy}: {narrowed:?}"
                );
            }
        }
    }

    /// Bytes of a target whose three useful texels see a plane.
    fn target(record: u32, px: PickPixel, vp: Mat4, plane_y: f32, st_of: impl Fn(Vec3) -> Vec2) -> Vec<u8> {
        let mut bytes = vec![0u8; TARGET_BYTES as usize];
        for (dx, dy) in [(0u32, 0u32), (1, 0), (0, 1)] {
            // the point of the plane y = plane_y seen through that pixel
            let p = PickPixel {
                x: px.x + dx,
                y: px.y + dy,
                ..px
            };
            let (near, far) = (p.world(1.0).expect("near"), p.world(0.001).expect("far"));
            let t = (plane_y - near.y) / (far.y - near.y);
            let point = near + (far - near) * t;
            let clip = vp * point.extend(1.0);
            let st = st_of(point);
            let words = [record + 1, (clip.z / clip.w).to_bits(), st.x.to_bits(), st.y.to_bits()];
            let at = (dy as u64 * ROW + dx as u64 * TEXEL) as usize;
            for (i, w) in words.iter().enumerate() {
                bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn a_face_gives_its_record_point_normal_and_binormal() {
        let (proj, view) = camera();
        let vp = proj * view;
        let px = PickPixel {
            x: 400,
            y: 300,
            width: 800,
            height: 600,
            inv_view_proj: vp.inverse(),
        };
        // a wall facing the camera (-y), textured with s along x, t along z
        let bytes = target(41, px, vp, 2.0, |p| Vec2::new(p.x * 0.5, p.z * 0.25));
        let f = face(&bytes, px).expect("a face");
        assert_eq!(f.record, 41);
        assert!((f.point.y - 2.0).abs() < 1e-2, "{:?}", f.point);
        assert!((f.st - Vec2::new(f.point.x * 0.5, f.point.z * 0.25)).length() < 1e-3);
        assert!(f.normal.distance(-Vec3::Y) < 1e-2, "{:?}", f.normal);
        assert!(f.binormal.distance(Vec3::Z) < 2e-2, "{:?}", f.binormal);
        // nothing drawn there
        assert_eq!(face(&vec![0u8; TARGET_BYTES as usize], px), None);
        // another face on a neighbour: the point, without a surface frame
        let mut edge = bytes.clone();
        edge[TEXEL as usize..TEXEL as usize + 4].copy_from_slice(&7u32.to_le_bytes());
        let f = face(&edge, px).expect("a face");
        assert_eq!((f.record, f.normal, f.binormal), (41, Vec3::ZERO, Vec3::ZERO));
    }
}
