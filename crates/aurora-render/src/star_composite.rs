//! The real night sky's stars drawn after the anti-aliasing (env.wgsl
//! `fs_star_composite`).
//!
//! Drawn into the scene with the sky, a star of a pixel or less went through
//! the temporal resolve: TAA smeared it, and FSR drew it at the render size
//! (two thirds of the display in Quality), then upscaled and sharpened it, so
//! the night sky looked like a blurred photograph. Here the sky pass keeps
//! the Milky Way and leaves the real stars' points out (bit 1 of the main
//! frame's `features.w`); this pass draws them at the display's size over the
//! resolved image, after TAA or FSR and before the depth of field, the glow,
//! the bloom and the tone mapping, into a target of its own (TAA's output is
//! also its history: stars added there would pile up frame after frame).
//!
//! The stars need no anti-aliasing: each is a smooth point of its own, its
//! light averaged over its pixel (`sky_star_light`). Their sizes are tuned on
//! a display of [`REFERENCE_LINES`]; on more, a star's pixel covers several
//! of the display's ([`star_scale`]), so a 4K display shows the same stars,
//! drawn finer, rather than stars half their size.
//!
//! The Legacy Viewer's stars, the reflections, the probes and the view under
//! water keep the sky pass's stars. Their target exists only while the stars
//! are drawn this way: in daylight it costs nothing.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// Lines of the display the stars' sizes are tuned for.
pub const REFERENCE_LINES: f32 = 1080.0;

/// The size of a star's pixel in pixels of an image of `lines` lines: 1 up
/// to [`REFERENCE_LINES`], then as the square root of the ratio (to 2): a
/// 4K display's stars 1.4 times the pixels of a 1080-line one's, so finer
/// in the sky, as its sharper picture allows, without shrinking to half
/// their size.
pub fn star_scale(lines: u32) -> f32 {
    (lines as f32 / REFERENCE_LINES).sqrt().clamp(1.0, 2.0)
}

/// `AURORA_STARS_AFTER_AA=0`: the real stars drawn in the sky pass, before
/// the anti-aliasing, as before (comparison; read once).
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_STARS_AFTER_AA").map_or(true, |v| v.trim() != "0"))
}

/// `StarComposite` in env.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StarCompositeU {
    inv_view_proj_rot: [[f32; 4]; 4],
    size: [f32; 4],
    misc: [f32; 4],
}

struct Targets {
    size: (u32, u32),
    out: wgpu::TextureView,
    /// The post passes' bind group reading `out`.
    post_bg: wgpu::BindGroup,
}

/// What one frame's pass reads.
pub(crate) struct Inputs<'a> {
    /// The resolved image (after TAA or FSR).
    pub color: &'a wgpu::TextureView,
    /// The scene's opaque depth at the render size (the prepass).
    pub depth: &'a wgpu::TextureView,
    /// The scene at the render size before the blended faces and after
    /// them: how much they let the stars through.
    pub before: &'a wgpu::TextureView,
    pub after: &'a wgpu::TextureView,
    /// The volumetric clouds' image when they are drawn this frame.
    pub clouds: Option<&'a wgpu::TextureView>,
    /// The main view's frame and texture bind groups (groups 0 and 2).
    pub frame: &'a wgpu::BindGroup,
    pub textures: &'a wgpu::BindGroup,
    /// The camera without the jitter: `inv_view_proj_rot(f.proj, f.view)`.
    pub inv_view_proj_rot: Mat4,
    /// [`star_scale`] of the image.
    pub scale: f32,
}

pub(crate) struct StarComposite {
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    /// A clear 1 x 1 cloud image (transmittance 1), when no volumetric
    /// clouds are drawn.
    clear_clouds: wgpu::TextureView,
    targets: Option<Targets>,
}

fn tex(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The pass's own bind group (group 3 of env.wgsl `fs_star_composite`),
/// made before the scene's pipelines, which take it.
pub(crate) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let float = wgpu::TextureSampleType::Float { filterable: false };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("star composite"),
        entries: &[
            tex(40, float),
            tex(41, wgpu::TextureSampleType::Depth),
            tex(42, float),
            tex(44, float),
            tex(45, float),
            wgpu::BindGroupLayoutEntry {
                binding: 43,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

impl StarComposite {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: wgpu::BindGroupLayout) -> StarComposite {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("star composite"),
            size: std::mem::size_of::<StarCompositeU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("star composite clear clouds"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // (0, 0, 0, 1) in half floats: no light, all transmitted
        let texel: [u16; 4] = [0, 0, 0, 0x3C00];
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&texel),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        StarComposite {
            layout,
            buffer,
            clear_clouds: texture.create_view(&Default::default()),
            targets: None,
        }
    }

    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// The target is dropped while the stars are drawn in the sky pass.
    pub(crate) fn release(&mut self) {
        self.targets = None;
    }

    /// Whether the target for `size` is made.
    pub(crate) fn has_targets(&self, size: (u32, u32)) -> bool {
        self.targets.as_ref().is_some_and(|t| t.size == size)
    }

    /// The target for `size`, made if needed (the renderer makes it first,
    /// checked against a full video memory).
    pub(crate) fn ensure_targets(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        hdr: wgpu::TextureFormat,
        post: &crate::dof::PostInputs,
    ) {
        if self.has_targets(size) {
            return;
        }
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let out = crate::renderer::make_view(device, "star composite", size.0, size.1, hdr, 1, usage).1;
        let post_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post (stars)"),
            layout: post.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&out),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(post.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: post.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(post.glow),
                },
            ],
        });
        self.targets = Some(Targets { size, out, post_bg });
    }

    /// Draws the stars over `inputs.color` into the target of `size` (made
    /// by `ensure_targets`); returns the image and the post passes' bind
    /// group reading it.
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        size: (u32, u32),
        inputs: &Inputs,
    ) -> Option<(&wgpu::TextureView, &wgpu::BindGroup)> {
        let t = self.targets.as_ref().filter(|t| t.size == size)?;
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let u = StarCompositeU {
            inv_view_proj_rot: inputs.inv_view_proj_rot.to_cols_array_2d(),
            size: [w, h, 1.0 / w, 1.0 / h],
            misc: [inputs.scale, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("star composite"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 40,
                    resource: wgpu::BindingResource::TextureView(inputs.color),
                },
                wgpu::BindGroupEntry {
                    binding: 41,
                    resource: wgpu::BindingResource::TextureView(inputs.depth),
                },
                wgpu::BindGroupEntry {
                    binding: 42,
                    resource: wgpu::BindingResource::TextureView(inputs.clouds.unwrap_or(&self.clear_clouds)),
                },
                wgpu::BindGroupEntry {
                    binding: 43,
                    resource: self.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 44,
                    resource: wgpu::BindingResource::TextureView(inputs.before),
                },
                wgpu::BindGroupEntry {
                    binding: 45,
                    resource: wgpu::BindingResource::TextureView(inputs.after),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("star composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, inputs.frame, &[]);
        pass.set_bind_group(2, inputs.textures, &[]);
        pass.set_bind_group(3, &group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        Some((&t.out, &t.post_bg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_pixel_grows_with_the_display_past_1080_lines() {
        assert_eq!(star_scale(720), 1.0);
        assert_eq!(star_scale(1080), 1.0);
        assert!((star_scale(2160) - std::f32::consts::SQRT_2).abs() < 1e-6);
        assert!(star_scale(1440) > 1.1 && star_scale(1440) < star_scale(2160));
        assert_eq!(star_scale(8640), 2.0);
    }

    #[test]
    fn the_uniforms_match_the_shader() {
        let env = include_str!("shaders/env.wgsl");
        assert_eq!(std::mem::size_of::<StarCompositeU>(), 64 + 16 + 16);
        for binding in [
            "@group(3) @binding(40) var sc_color: texture_2d<f32>;",
            "@group(3) @binding(41) var sc_depth: texture_depth_2d;",
            "@group(3) @binding(42) var sc_clouds: texture_2d<f32>;",
            "@group(3) @binding(43) var<uniform> sc: StarComposite;",
            "@group(3) @binding(44) var sc_before: texture_2d<f32>;",
            "@group(3) @binding(45) var sc_after: texture_2d<f32>;",
        ] {
            assert!(env.contains(binding), "{binding}");
        }
    }
}
