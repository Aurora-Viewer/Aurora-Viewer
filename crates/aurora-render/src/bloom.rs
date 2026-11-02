//! Aurora's bloom of the bright parts of the image (the Legacy Viewer has
//! none: its glow, `Glow` in renderer.rs, only lights faces with a glow
//! value and stays as it is beside this). The mip chain of Jimenez, "Next
//! Generation Post Processing in Call of Duty: Advanced Warfare" (SIGGRAPH
//! 2014): the light over a threshold extracted at half resolution with a
//! Karis average (post.wgsl `fs_bloom_prefilter`), smaller levels by a
//! 13-tap downsample, then each level blurred by a 3 x 3 tent and added to
//! the one above it (`shaders/bloom.wgsl`); the post pass takes the light
//! over the threshold away from each pixel and puts it back spread by the
//! levels, [`GAIN`] times brighter. Its targets exist only while the effect
//! is on: off, it costs nothing.

use crate::pipeline_cache::CachedPipelines;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The most levels: the last one, 1/64 of the image, is about as wide as
/// the halo should reach.
const MAX_LEVELS: usize = 6;
/// No level smaller than this (pixels on its short side).
const MIN_SIDE: u32 = 8;
/// Threshold and knee in exposed units (1 = white once tone mapped): what
/// comes close to white blooms, softly from half of it.
pub(crate) const THRESHOLD: f32 = 0.8;
pub(crate) const KNEE: f32 = 0.3;
/// How much brighter the halo is than the light it takes away: the light of
/// Second Life scenes seldom goes far past white (classic skies are not
/// tone mapped), so a halo of only the light over the threshold would hardly
/// show (measured in the demo: 1 and 3 barely visible, 6 a clear halo).
pub(crate) const GAIN: f32 = 5.0;
/// The share spread when `AURORA_BLOOM=1` or `alt` turns it on while the
/// setting is off.
pub(crate) const DEFAULT_STRENGTH: f32 = 0.5;

/// How far, in pixels of the image, the bloom of a pixel reads the image
/// around it: the 13 taps of each downsample reach 2 texels of the level
/// above and their bilinear half texel (2.5 x 2^(k-1) pixels into level
/// k), the tent of each upsample a texel of the smaller level and a half
/// (1.5 x 2^k into level k - 1), the post pass a texel of the first level.
/// A photo drawn in tiles draws this much around each one (346 pixels).
pub const TILE_REACH: u32 = (5 * ((1u32 << MAX_LEVELS) - 1)).div_ceil(2) + 3 * ((1u32 << (MAX_LEVELS + 1)) - 4) / 2 + 2;

/// The pixels a tile of a photo must draw around what it keeps for the
/// bloom of `setting` (its strength, 0 off; `AURORA_BLOOM` included): 0
/// when it is off.
pub fn tile_reach(setting: f32) -> u32 {
    if Override::from_env().strength(setting, 0).1 {
        TILE_REACH
    } else {
        0
    }
}

/// The size of each level: half the image, then halved, while the short
/// side stays at least [`MIN_SIDE`].
pub(crate) fn levels(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let (mut w, mut h) = (width.max(1).div_ceil(2), height.max(1).div_ceil(2));
    while out.len() < MAX_LEVELS && (out.is_empty() || w.min(h) >= MIN_SIDE) {
        out.push((w, h));
        w = w.div_ceil(2);
        h = h.div_ceil(2);
    }
    out
}

/// `AURORA_BLOOM`: `0` off whatever the setting, `1` on, `alt` on one
/// second out of two (to compare in the same session).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Override {
    None,
    Off,
    On,
    Alternate,
}

impl Override {
    pub(crate) fn parse(v: Option<&str>) -> Override {
        match v.map(str::trim) {
            Some("0") | Some("off") => Override::Off,
            Some("1") | Some("on") => Override::On,
            Some("alt") => Override::Alternate,
            _ => Override::None,
        }
    }

    pub(crate) fn from_env() -> Override {
        static ENV: std::sync::OnceLock<Override> = std::sync::OnceLock::new();
        *ENV.get_or_init(|| Override::parse(std::env::var("AURORA_BLOOM").ok().as_deref()))
    }

    /// The share spread this frame from the setting's (`setting`, 0 =
    /// off) at `secs` seconds into the session, and whether the levels must
    /// be kept ready.
    pub(crate) fn strength(self, setting: f32, secs: u64) -> (f32, bool) {
        let on = if setting > 0.0 { setting } else { DEFAULT_STRENGTH };
        match self {
            Override::None => (setting.max(0.0), setting > 0.0),
            Override::Off => (0.0, false),
            Override::On => (on, true),
            Override::Alternate => (if secs.is_multiple_of(2) { on } else { 0.0 }, true),
        }
    }
}

struct Level {
    view: wgpu::TextureView,
    /// Reads this level (downsample to the next one, upsample to the one
    /// before).
    bg: wgpu::BindGroup,
}

struct Targets {
    size: (u32, u32),
    levels: Vec<Level>,
    /// The post pass's group 1 reading the first level.
    post_bg: wgpu::BindGroup,
}

pub(crate) struct Bloom {
    /// Group 1 of the post pipelines: the bloom they spread.
    pub(crate) post_layout: wgpu::BindGroupLayout,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    /// Black, for the post passes while the bloom is off.
    off_bg: wgpu::BindGroup,
    targets: Option<Targets>,
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl Bloom {
    /// `main_post_layout`: group 0 of the post pass (the scene image and
    /// its parameters), which the first level is extracted from.
    pub(crate) fn new(device: &wgpu::Device, main_post_layout: &wgpu::BindGroupLayout) -> Bloom {
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom post"),
            entries: &[tex_entry(0)],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let pipe = |label: &str, module: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, vs: &str, fs: &str, add: bool| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            let add_blend = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            };
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: add.then_some(wgpu::BlendState {
                            color: add_blend,
                            alpha: add_blend,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom prefilter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/bloom.wgsl").into()),
        });
        let prefilter = pipe(
            "bloom prefilter",
            &post_module,
            main_post_layout,
            "vs_post",
            "fs_bloom_prefilter",
            false,
        );
        let down = pipe("bloom down", &module, &layout, "vs_bloom", "fs_bloom_down", false);
        let up = pipe("bloom up", &module, &layout, "vs_bloom", "fs_bloom_up", true);
        // a new texture reads zero: black
        let black = crate::renderer::make_view(device, "bloom off", 1, 1, FORMAT, 1, wgpu::TextureUsages::TEXTURE_BINDING).1;
        let off_bg = Self::post_group(device, &post_layout, &black);
        Bloom {
            post_layout,
            layout,
            sampler,
            prefilter,
            down,
            up,
            off_bg,
            targets: None,
        }
    }

    fn post_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom post"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            }],
        })
    }

    /// What the post pass binds as its group 1: the levels while they
    /// exist, black otherwise.
    pub(crate) fn post_bg(&self) -> &wgpu::BindGroup {
        self.targets.as_ref().map_or(&self.off_bg, |t| &t.post_bg)
    }

    /// 1 / the number of levels added up (0 while off).
    pub(crate) fn norm(&self) -> f32 {
        self.targets.as_ref().map_or(0.0, |t| 1.0 / t.levels.len() as f32)
    }

    /// The levels are dropped with the effect.
    pub(crate) fn release(&mut self) {
        self.targets = None;
    }

    /// Levels for an image of `size`, made again when it changes.
    pub(crate) fn ensure(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let levels: Vec<Level> = levels(size.0, size.1)
            .into_iter()
            .enumerate()
            .map(|(i, (w, h))| {
                let view = crate::renderer::make_view(device, &format!("bloom {i}"), w, h, FORMAT, 1, usage).1;
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bloom"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                Level { view, bg }
            })
            .collect();
        let post_bg = Self::post_group(device, &self.post_layout, &levels[0].view);
        self.targets = Some(Targets { size, levels, post_bg });
    }

    /// The levels from the image the post pass reads (`scene_bg`, its
    /// group 0), after [`Bloom::ensure`].
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, scene_bg: &wgpu::BindGroup) {
        let Some(t) = &self.targets else {
            return;
        };
        let run = |encoder: &mut wgpu::CommandEncoder,
                   target: &wgpu::TextureView,
                   keep: bool,
                   pipe: &wgpu::RenderPipeline,
                   bg: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if keep {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        };
        run(encoder, &t.levels[0].view, false, &self.prefilter, scene_bg);
        for i in 1..t.levels.len() {
            run(encoder, &t.levels[i].view, false, &self.down, &t.levels[i - 1].bg);
        }
        for i in (1..t.levels.len()).rev() {
            run(encoder, &t.levels[i - 1].view, true, &self.up, &t.levels[i].bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_halve_from_half_resolution_down_to_eight_pixels() {
        assert_eq!(
            levels(2560, 1369),
            vec![(1280, 685), (640, 343), (320, 172), (160, 86), (80, 43), (40, 22)]
        );
        // a small window stops early, its short side at 8 or more
        assert_eq!(levels(100, 40), vec![(50, 20), (25, 10)]);
        // never none
        assert_eq!(levels(1, 1), vec![(1, 1)]);
    }

    #[test]
    fn downsample_and_tent_weights_keep_the_light() {
        // bloom.wgsl fs_bloom_down: centre 0.125, 4 corners 0.03125,
        // 4 edges 0.0625, 4 inner 0.125
        let down: f32 = 0.125 + 4.0 * 0.03125 + 4.0 * 0.0625 + 4.0 * 0.125;
        assert!((down - 1.0).abs() < 1e-6);
        // fs_bloom_up: (4 + 4 x 2 + 4) / 16
        let tent: f32 = (4.0 + 4.0 * 2.0 + 4.0) / 16.0;
        assert!((tent - 1.0).abs() < 1e-6);
        // fs_bloom_prefilter: one inner box at 0.5, four at 0.125
        let boxes: f32 = 0.5 + 4.0 * 0.125;
        assert!((boxes - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_switch_forces_or_alternates_the_bloom() {
        assert_eq!(Override::parse(None), Override::None);
        assert_eq!(Override::parse(Some("0")), Override::Off);
        assert_eq!(Override::parse(Some(" 1 ")), Override::On);
        assert_eq!(Override::parse(Some("alt")), Override::Alternate);
        assert_eq!(Override::None.strength(0.0, 0), (0.0, false));
        assert_eq!(Override::None.strength(0.3, 0), (0.3, true));
        assert_eq!(Override::Off.strength(0.3, 0), (0.0, false));
        assert_eq!(Override::On.strength(0.0, 0), (DEFAULT_STRENGTH, true));
        assert_eq!(Override::Alternate.strength(0.3, 4), (0.3, true));
        // off every other second, levels kept ready
        assert_eq!(Override::Alternate.strength(0.3, 5), (0.0, true));
    }
}
