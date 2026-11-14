//! The 360° snapshot's panorama (LLFloater360Capture, indra/newview/
//! llfloater360capture.cpp, originally LGPL 2.1): the Legacy Viewer draws
//! six square faces from the camera's position and hands them to a web
//! page that stitches them with three.js. Aurora draws the same six faces
//! through its photo frames, then projects them to an equirectangular image
//! on the GPU (`shaders/equirect.wgsl`): each face's photo frame is copied
//! into a layer of a cube texture that stays on the GPU (`CubeTarget`), the
//! hardware samples it across the faces' edges, and the image is read back
//! asynchronously (`Readbacks`): the job rides a frame
//! (`FramePacket::equirect`), the answer comes with a later one
//! (`FrameResult::equirect`). No frame, and no thread of the renderer, waits
//! on the GPU for any of it.
//!
//! A cube map's faces are mirror images of what a camera draws looking
//! the same way (the cube is laid out as seen from outside). Rather than
//! flipping six images, the faces are drawn by ordinary cameras
//! ([`CAPTURE_CAMERAS`]), the +X and -X captures swap layers, and the cube
//! is sampled at (-x, y, z): `cube_layout_matches_the_capture_cameras`
//! proves it against the cube face formulas.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
    mpsc,
};

/// One capture camera: where it looks, its up, and the cube layer its
/// image fills.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureCamera {
    pub forward: Vec3,
    pub up: Vec3,
    pub layer: usize,
}

/// The six cameras of a 360° capture, square and 90° wide, in the Legacy
/// Viewer's order (+X, +Y, +Z, -X, -Y, -Z).
pub const CAPTURE_CAMERAS: [CaptureCamera; 6] = [
    CaptureCamera {
        forward: Vec3::X,
        up: Vec3::Y,
        layer: 1,
    },
    CaptureCamera {
        forward: Vec3::Y,
        up: Vec3::NEG_Z,
        layer: 2,
    },
    CaptureCamera {
        forward: Vec3::Z,
        up: Vec3::Y,
        layer: 4,
    },
    CaptureCamera {
        forward: Vec3::NEG_X,
        up: Vec3::Y,
        layer: 0,
    },
    CaptureCamera {
        forward: Vec3::NEG_Y,
        up: Vec3::Z,
        layer: 3,
    },
    CaptureCamera {
        forward: Vec3::NEG_Z,
        up: Vec3::Y,
        layer: 5,
    },
];

/// The direction a pixel of the panorama looks at, in the z-up world: the
/// centre column north (+Y), east (+X) a quarter of the width to its
/// right, the top row the zenith (the shader's `direction`).
pub fn pixel_direction(x: f32, y: f32, width: u32, height: u32) -> Vec3 {
    let heading = (x / width.max(1) as f32 - 0.5) * std::f32::consts::TAU;
    let pitch = (0.5 - y / height.max(1) as f32) * std::f32::consts::PI;
    let c = pitch.cos();
    Vec3::new(heading.sin() * c, heading.cos() * c, pitch.sin())
}

/// The panorama wanted from the six faces a capture drew into the cube
/// (`Backend::cube`): its size, and the number of the capture it is for, which
/// comes back with the answer (a capture given up has its answer thrown away,
/// whenever, or whether, it arrives).
pub struct EquirectJob {
    pub id: u64,
    pub width: u32,
    pub height: u32,
}

/// The faces of a 360° capture, kept on the GPU between their frames: each
/// photo frame of a face is copied straight into its layer, with no readback
/// and no re-upload, and the projection samples the cube from there.
pub(crate) struct CubeTarget {
    texture: wgpu::Texture,
    face_size: u32,
    format: wgpu::TextureFormat,
    /// The layers drawn since the capture's first face.
    filled: u8,
}

impl CubeTarget {
    /// None when the card refuses the six faces (100 MB at the largest).
    pub(crate) fn new(device: &wgpu::Device, face_size: u32, format: wgpu::TextureFormat) -> Option<Self> {
        let desc = wgpu::TextureDescriptor {
            label: Some("equirect cube"),
            size: wgpu::Extent3d {
                width: face_size,
                height: face_size,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            // (the faces hold the encoded values a readback would give:
            // read as is, whatever the swapchain's sRGB)
            view_formats: &[format.remove_srgb_suffix()],
        };
        let bytes = u64::from(face_size) * u64::from(face_size) * 24;
        let (texture, _) = match crate::gpu_alloc::try_texture(device, &desc, &Default::default(), bytes, false) {
            Ok(made) => made,
            Err(e) => {
                log::warn!("360 capture: the cube of {face_size} px faces was refused: {e}");
                return None;
            }
        };
        Some(CubeTarget {
            texture,
            face_size,
            format,
            filled: 0,
        })
    }

    /// A capture starts: no layer is drawn yet.
    pub(crate) fn reset(&mut self) {
        self.filled = 0;
    }

    pub(crate) fn fits(&self, face_size: u32, format: wgpu::TextureFormat) -> bool {
        self.face_size == face_size && self.format == format
    }

    /// The face of capture camera `step` (the first one starts a capture),
    /// copied from the photo frame `source`.
    pub(crate) fn add_face(&mut self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture, step: usize) {
        if step == 0 {
            self.filled = 0;
        }
        let layer = CAPTURE_CAMERAS[step].layer;
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.face_size,
                height: self.face_size,
                depth_or_array_layers: 1,
            },
        );
        self.filled |= layer_bit(layer);
    }

    pub(crate) fn complete(&self) -> bool {
        self.filled == ALL_LAYERS
    }
}

const ALL_LAYERS: u8 = 0b11_1111;

fn layer_bit(layer: usize) -> u8 {
    1 << layer
}

/// The panoramas being read back, in the order they were asked: each is
/// mapped by the GPU (`state`), then copied out of its buffer on a worker
/// thread (the render thread never copies 128 MB), then answered with the
/// number of its capture, one a call at most.
#[derive(Default)]
pub(crate) struct Readbacks {
    items: std::collections::VecDeque<Entry>,
}

/// The panorama read back: width, height, RGBA8.
pub(crate) type Pixels = (u32, u32, Vec<u8>);

/// The answer to a job: its capture's number, and the panorama (None when it
/// could not be made). Public through `Renderer::equirect`.
pub type Answer = (u64, Option<Pixels>);

struct Entry {
    id: u64,
    state: Arc<AtomicU8>,
    copy: Option<Box<dyn FnOnce() -> Option<Pixels> + Send>>,
    running: Option<mpsc::Receiver<Option<Pixels>>>,
}

pub(crate) const MAPPING: u8 = 0;
pub(crate) const MAPPED: u8 = 1;
pub(crate) const FAILED: u8 = 2;

impl Readbacks {
    /// A readback begun: `state` turns `MAPPED` or `FAILED` when the GPU is
    /// done; `copy` then reads the pixels out.
    pub(crate) fn push(&mut self, id: u64, state: Arc<AtomicU8>, copy: impl FnOnce() -> Option<Pixels> + Send + 'static) {
        self.items.push_back(Entry {
            id,
            state,
            copy: Some(Box::new(copy)),
            running: None,
        });
    }

    /// A panorama that could not even be asked for.
    pub(crate) fn push_failed(&mut self, id: u64) {
        self.items.push_back(Entry {
            id,
            state: Arc::new(AtomicU8::new(FAILED)),
            copy: None,
            running: None,
        });
    }

    /// The oldest panorama's answer when it is ready: Some(None) when it
    /// failed.
    pub(crate) fn poll(&mut self) -> Option<Answer> {
        let front = self.items.front_mut()?;
        let id = front.id;
        if let Some(rx) = &front.running {
            return match rx.try_recv() {
                Ok(pixels) => {
                    self.items.pop_front();
                    Some((id, pixels))
                }
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.items.pop_front();
                    Some((id, None))
                }
            };
        }
        match front.state.load(Ordering::Acquire) {
            MAPPED => {
                let Some(copy) = front.copy.take() else {
                    self.items.pop_front();
                    return Some((id, None));
                };
                let (tx, rx) = mpsc::channel();
                match std::thread::Builder::new().name("aurora-360-readback".into()).spawn(move || {
                    let _ = tx.send(copy());
                }) {
                    Ok(_) => front.running = Some(rx),
                    Err(e) => {
                        log::warn!("360 capture: cannot start the readback thread: {e}");
                        self.items.pop_front();
                        return Some((id, None));
                    }
                }
                None
            }
            FAILED => {
                self.items.pop_front();
                Some((id, None))
            }
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    width: u32,
    height: u32,
    _pad: [u32; 2],
}

/// The pass, made on first use and kept.
pub(crate) struct Equirect {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
}

impl Equirect {
    pub(crate) fn new(device: &wgpu::Device) -> Equirect {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("equirect"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("equirect"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/equirect.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("equirect"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("equirect"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_equirect"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("equirect"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Equirect { layout, pipeline, sampler }
    }

    /// The panorama of `job` projected from the capture's cube, its
    /// readback begun: the result goes to `readbacks` and comes out of it a
    /// few frames later, with no wait here. A failure (faces missing, sizes
    /// beyond the card) is answered as a failed panorama.
    pub(crate) fn start(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lost: &crate::device_lost::DeviceLost,
        frame: u64,
        cube: Option<&CubeTarget>,
        job: EquirectJob,
        readbacks: &mut Readbacks,
    ) {
        let EquirectJob { id, width, height } = job;
        let limit = device.limits().max_texture_dimension_2d;
        let Some(cube) = cube.filter(|c| c.complete()) else {
            log::warn!("360 capture: the faces are not all drawn");
            readbacks.push_failed(id);
            return;
        };
        if width == 0 || height == 0 || width > limit || height > limit {
            log::warn!("360 capture: {width} x {height} is beyond the card ({limit})");
            readbacks.push_failed(id);
            return;
        }
        // (up to 128 MB each: refused, the capture fails instead of the device)
        let out_desc = wgpu::TextureDescriptor {
            label: Some("equirect out"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let out_bytes = u64::from(width) * u64::from(height) * 4;
        let Ok((out, out_view)) = crate::gpu_alloc::try_texture(device, &out_desc, &Default::default(), out_bytes, false) else {
            log::warn!("360 capture: the {width} x {height} panorama was refused by the card");
            readbacks.push_failed(id);
            return;
        };
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("equirect params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &params,
            0,
            bytemuck::bytes_of(&Params {
                width,
                height,
                _pad: [0; 2],
            }),
        );
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("equirect"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&cube.texture.create_view(&wgpu::TextureViewDescriptor {
                        format: Some(cube.format.remove_srgb_suffix()),
                        dimension: Some(wgpu::TextureViewDimension::Cube),
                        ..Default::default()
                    })),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        // rows padded to the copy alignment
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let Ok(readback) = crate::gpu_alloc::try_buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("equirect readback"),
                size: crate::readback::readback_size(u64::from(row) * u64::from(height)),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            },
        ) else {
            log::warn!("360 capture: the readback buffer of the panorama was refused by the card");
            readbacks.push_failed(id);
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("equirect") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("equirect"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &out,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
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
        // (noted for the line of a device loss, device_lost.rs)
        crate::device_lost::submit_tracked(lost, queue, frame, [encoder.finish()]);
        let state = Arc::new(AtomicU8::new(MAPPING));
        let done = state.clone();
        readback.map_async(wgpu::MapMode::Read, .., move |result| {
            done.store(if result.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
        });
        readbacks.push(id, state, move || {
            let pixels = {
                let view = readback.get_mapped_range(..).ok()?;
                let mut out = Vec::with_capacity(width as usize * height as usize * 4);
                for y in 0..height as usize {
                    out.extend_from_slice(&view[y * row as usize..y * row as usize + width as usize * 4]);
                }
                out
            };
            readback.unmap();
            Some((width, height, pixels))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec4Swizzles};

    /// The cube face and texel the hardware reads for `d` (the Vulkan cube
    /// map face selection: major axis, then (sc, tc) per face).
    fn cube_lookup(d: Vec3) -> (usize, f32, f32) {
        let a = d.abs();
        let (layer, sc, tc, ma) = if a.x >= a.y && a.x >= a.z {
            if d.x > 0.0 { (0, -d.z, -d.y, a.x) } else { (1, d.z, -d.y, a.x) }
        } else if a.y >= a.z {
            if d.y > 0.0 { (2, d.x, d.z, a.y) } else { (3, d.x, -d.z, a.y) }
        } else if d.z > 0.0 {
            (4, d.x, -d.y, a.z)
        } else {
            (5, -d.x, -d.y, a.z)
        };
        (layer, (sc / ma + 1.0) * 0.5, (tc / ma + 1.0) * 0.5)
    }

    /// Every direction inside a capture camera's view lands on the layer
    /// its image fills, at the texel the camera drew it, once the cube is
    /// read at (-x, y, z): the panorama shows each thing where it is.
    #[test]
    fn cube_layout_matches_the_capture_cameras() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_2, 1.0, 0.1);
        let origin = Vec3::new(3.0, -7.0, 20.0);
        let mut layers = [false; 6];
        for cam in CAPTURE_CAMERAS {
            layers[cam.layer] = true;
            let view = glam::camera::rh::view::look_to_mat4(origin, cam.forward, cam.up);
            let vp: Mat4 = proj * view;
            let right = cam.forward.cross(cam.up);
            for (a, b) in [(0.0, 0.0), (0.7, 0.3), (-0.5, 0.9), (0.95, -0.95), (-0.2, -0.6)] {
                let d = (cam.forward + right * a + cam.up * b).normalize();
                let clip = vp * (origin + d).extend(1.0);
                let ndc = clip.xy() / clip.w;
                let (col, row) = ((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5);
                let (layer, s, t) = cube_lookup(Vec3::new(-d.x, d.y, d.z));
                assert_eq!(layer, cam.layer, "{cam:?} at {d:?}");
                assert!(
                    (s - col).abs() < 1e-4 && (t - row).abs() < 1e-4,
                    "{cam:?} at {d:?}: ({s}, {t}) vs ({col}, {row})"
                );
            }
        }
        assert!(layers.iter().all(|l| *l), "every layer is filled");
    }

    /// The panorama's columns go around the compass from the west edge,
    /// north in the middle, east at three quarters; its rows from the
    /// zenith to the nadir.
    #[test]
    fn pixel_directions_follow_the_compass() {
        let (w, h) = (4096, 2048);
        let close = |a: Vec3, b: Vec3| (a - b).length() < 1e-4;
        assert!(close(pixel_direction(2048.0, 1024.0, w, h), Vec3::Y));
        assert!(close(pixel_direction(3072.0, 1024.0, w, h), Vec3::X));
        assert!(close(pixel_direction(1024.0, 1024.0, w, h), Vec3::NEG_X));
        assert!(close(pixel_direction(0.0, 1024.0, w, h), Vec3::NEG_Y));
        assert!(close(pixel_direction(2048.0, 0.0, w, h), Vec3::Z));
        assert!(close(pixel_direction(2048.0, 2048.0, w, h), Vec3::NEG_Z));
    }

    #[test]
    fn equirect_shader_is_valid_for_vulkan_and_metal() {
        let source = include_str!("shaders/equirect.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
        for ep in &module.entry_points {
            let spv = naga::back::spv::PipelineOptions {
                shader_stage: ep.stage,
                entry_point: ep.name.clone(),
            };
            naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), Some(&spv))
                .unwrap_or_else(|e| panic!("{}: {e:?}", ep.name));
            let msl = naga::back::msl::PipelineOptions {
                entry_point: Some((ep.stage, ep.name.clone())),
                ..Default::default()
            };
            naga::back::msl::write_string(
                &module,
                &info,
                &naga::back::msl::Options {
                    lang_version: (2, 4),
                    ..Default::default()
                },
                &msl,
            )
            .unwrap_or_else(|e| panic!("{}: {e:?}", ep.name));
        }
    }

    fn ready(value: u8) -> Arc<AtomicU8> {
        Arc::new(AtomicU8::new(value))
    }

    fn wait_for(readbacks: &mut Readbacks) -> Answer {
        for _ in 0..2000 {
            if let Some(answer) = readbacks.poll() {
                return answer;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("no answer");
    }

    /// Answers come in the order asked, one at a time, each pixel read once;
    /// one still mapping holds the ones behind it.
    #[test]
    fn readbacks_answer_in_order_and_copy_once() {
        let mut readbacks = Readbacks::default();
        assert!(readbacks.poll().is_none());
        let first = ready(MAPPING);
        let copies = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = copies.clone();
        readbacks.push(7, first.clone(), move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Some((2, 1, vec![1; 8]))
        });
        readbacks.push(8, ready(MAPPED), || Some((1, 1, vec![2; 4])));
        assert!(readbacks.poll().is_none(), "the first is still mapping and holds the second");
        first.store(MAPPED, Ordering::Release);
        assert_eq!(wait_for(&mut readbacks), (7, Some((2, 1, vec![1; 8]))));
        assert_eq!(wait_for(&mut readbacks), (8, Some((1, 1, vec![2; 4]))));
        assert_eq!(copies.load(Ordering::SeqCst), 1, "mapped once, copied once");
        assert!(readbacks.poll().is_none());
    }

    #[test]
    fn a_failed_readback_is_answered_as_failed_and_does_not_hold_the_next() {
        let mut readbacks = Readbacks::default();
        readbacks.push_failed(1);
        readbacks.push(2, ready(FAILED), || unreachable!("a failed map is never read"));
        readbacks.push(3, ready(MAPPED), || Some((1, 1, vec![9; 4])));
        assert_eq!(readbacks.poll(), Some((1, None)));
        assert_eq!(readbacks.poll(), Some((2, None)));
        assert_eq!(wait_for(&mut readbacks), (3, Some((1, 1, vec![9; 4]))));
    }

    #[test]
    fn a_copy_that_fails_answers_none() {
        let mut readbacks = Readbacks::default();
        readbacks.push(4, ready(MAPPED), || None);
        assert_eq!(wait_for(&mut readbacks), (4, None));
        assert!(readbacks.poll().is_none(), "answered once");
    }

    /// Every layer of the cube is named by exactly one capture camera, and a
    /// capture is complete only with all six.
    #[test]
    fn the_cube_needs_all_six_layers() {
        let mut mask = 0u8;
        for cam in CAPTURE_CAMERAS {
            assert_eq!(mask & layer_bit(cam.layer), 0, "layer {} twice", cam.layer);
            mask |= layer_bit(cam.layer);
        }
        assert_eq!(mask, ALL_LAYERS);
    }

    #[test]
    fn params_match_the_shader_layout() {
        assert_eq!(std::mem::size_of::<Params>(), 16);
    }
}
