//! The 360° snapshot's panorama (LLFloater360Capture, indra/newview/
//! llfloater360capture.cpp, originally LGPL 2.1): the Legacy Viewer draws
//! six square faces from the camera's position and hands them to a web
//! page that stitches them with three.js. Aurora draws the same six faces
//! through its photo frames, then projects them to an equirectangular image
//! on the GPU (`shaders/equirect.wgsl`): the faces fill a cube texture, the
//! hardware samples it across the faces' edges, and the image is read back
//! with the frame that carried the job (`FramePacket::equirect`,
//! `FrameResult::equirect`).
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

/// The six faces of a capture and the panorama wanted from them.
pub struct EquirectJob {
    /// Side of each face.
    pub face_size: u32,
    /// RGBA8, rows top to bottom, `face_size` square each, by cube layer.
    pub faces: [Vec<u8>; 6],
    pub width: u32,
    pub height: u32,
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

    /// The panorama of `job` (width, height, RGBA8), read back here: None
    /// when the sizes are beyond the card or the device is lost.
    pub(crate) fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lost: &crate::device_lost::DeviceLost,
        frame: u64,
        job: EquirectJob,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let EquirectJob {
            face_size,
            faces,
            width,
            height,
        } = job;
        let limit = device.limits().max_texture_dimension_2d;
        let face_bytes = (face_size as usize).checked_mul(face_size as usize)?.checked_mul(4)?;
        if face_size == 0 || width == 0 || height == 0 || face_size > limit || width > limit || height > limit {
            log::warn!("360 capture: {face_size} px faces to {width} x {height} are beyond the card ({limit})");
            return None;
        }
        if faces.iter().any(|f| f.len() != face_bytes) {
            log::warn!("360 capture: a face is not {face_size} x {face_size}");
            return None;
        }
        let cube = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("equirect cube"),
            size: wgpu::Extent3d {
                width: face_size,
                height: face_size,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, face) in faces.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &cube,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                face,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(face_size * 4),
                    rows_per_image: Some(face_size),
                },
                wgpu::Extent3d {
                    width: face_size,
                    height: face_size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let out = device.create_texture(&wgpu::TextureDescriptor {
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
        });
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
                    resource: wgpu::BindingResource::TextureView(&cube.create_view(&wgpu::TextureViewDescriptor {
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
                    resource: wgpu::BindingResource::TextureView(&out.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        // rows padded to the copy alignment
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("equirect readback"),
            size: crate::readback::readback_size(u64::from(row) * u64::from(height)),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
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
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        if !lost.wait(device) {
            return None;
        }
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

    #[test]
    fn params_match_the_shader_layout() {
        assert_eq!(std::mem::size_of::<Params>(), 16);
    }
}
