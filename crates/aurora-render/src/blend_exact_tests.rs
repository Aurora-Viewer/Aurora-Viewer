//! The exact blend order on a GPU, through the production fragment
//! shaders: layers drawn in every order must give the pixel a sort of them
//! by depth gives, and more layers than a front holds what the shader says
//! of them (`Pixel`, the same steps on the CPU).
use crate::blend_exact::{
    EARLY_TEST, HAZE_FORMAT, Lists, PAGES, SLOTS, TWO_SOURCES, WIDE_RANGE, both_blend, layouts, once_blend, range_blend, range_format, range_shift,
    scene_source,
};
use crate::types::flags;
use wgpu::util::DeviceExt;

/// A layer over the whole view.
#[derive(Clone, Copy)]
struct Layer {
    rgb: [f32; 3],
    alpha: f32,
    /// Reverse depth: a greater one is nearer.
    z: f32,
    rank: u32,
    glow: f32,
}

const BACKGROUND: [f32; 3] = [0.2, 0.4, 0.6];
/// The glow of the scene behind the layers.
const SCENE_GLOW: f32 = 0.4;
const SIZE: u32 = 8;

/// `BLEND_CULL`, `BLEND_FAINT`, `BLEND_FAINT_GLOW` and `BLEND_OPAQUE` of
/// object.wgsl.
const CULL: f32 = 0.002;
const FAINT: u32 = 8;
const FAINT_GLOW: u32 = 16;
const OPAQUE: f32 = 8.3;

/// The most fragments a pixel keeps in their order (`BLEND_MOST`).
const MOST: usize = (SLOTS * (1 + PAGES)) as usize;

/// Pages of the pool of the bench: more than its pixels take in any view
/// here.
const POOL: u64 = (SIZE * SIZE * PAGES * 8) as u64;

/// The alpha a layer is filed with, in 255ths.
fn alpha8(alpha: f32) -> u32 {
    (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
}

/// The alpha a layer is filed with.
fn filed(alpha: f32) -> f32 {
    alpha8(alpha) as f32 / 255.0
}

/// The pixel behind `layers`, sorted here: back to front, the layers of
/// one depth and rank sharing the light they stop together.
fn sorted(layers: &[Layer]) -> [f32; 3] {
    let key = |l: &Layer| (l.z, l.rank);
    let mut order: Vec<&Layer> = layers.iter().collect();
    order.sort_by(|a, b| key(b).partial_cmp(&key(a)).expect("finite depths"));
    let mut colour = [0.0_f32; 3];
    let mut through = 1.0_f32;
    let mut at = 0;
    while at < order.len() {
        let end = at + order[at..].iter().take_while(|l| key(l) == key(order[at])).count();
        let group = &order[at..end];
        let stops: f32 = 1.0 - group.iter().map(|l| 1.0 - filed(l.alpha)).product::<f32>();
        let share = stops / group.iter().map(|l| filed(l.alpha)).sum::<f32>();
        for layer in group {
            for (sum, c) in colour.iter_mut().zip(layer.rgb) {
                *sum += c * layer.alpha * through * share;
            }
        }
        through *= 1.0 - stops;
        at = end;
    }
    std::array::from_fn(|i| colour[i] + BACKGROUND[i] * through)
}

/// `blend_place` of object.wgsl.
fn place(layer: &Layer) -> u32 {
    let bits = layer.z.clamp(0.0, 1.0).to_bits();
    let e = (bits >> 23).clamp(96, 127) - 96;
    let m = if (bits >> 23) < 96 { 0 } else { (bits >> 7) & 0xffff };
    (e << 19) | (m << 3) | layer.rank
}

/// `blend_pack` then `blend_unpack` of object.wgsl.
fn packed(light: [f32; 3]) -> [f32; 3] {
    let v = light.map(|c| c.clamp(0.0, 65408.0));
    let top = v[0].max(v[1]).max(v[2]);
    let mut e = top.max(1.0e-12).log2().floor().max(-16.0) + 16.0;
    let mut scale = (24.0 - e).exp2();
    if (top * scale + 0.5).floor() >= 512.0 {
        e += 1.0;
        scale *= 0.5;
    }
    v.map(|c| (c * scale + 0.5).floor() * (e - 24.0).exp2())
}

/// `blend_glow_pack` of object.wgsl.
fn glow8(glow: f32) -> u32 {
    ((glow * 0.5).clamp(0.0, 1.0).sqrt() * 255.0 + 0.5) as u32
}

/// `blend_glow_unpack` of object.wgsl.
fn unglow8(byte: u32) -> f32 {
    let root = byte as f32 / 255.0;
    root * root * 2.0
}

/// The glow a layer is filed with.
fn filed_glow(glow: f32) -> f32 {
    unglow8(glow8(glow))
}

/// `blend_stops` of object.wgsl.
fn stops(cover: f32) -> f32 {
    (-(1.0 - cover).max(1.0e-6).ln()).min(OPAQUE)
}

/// `blend_slab` of object.wgsl.
fn slab(stops: f32, from: f32, to: f32) -> f32 {
    if stops > 1.0e-4 {
        ((-stops * from).exp() - (-stops * to).exp()) / stops
    } else {
        to - from
    }
}

/// Place and alpha, light and glow of a fragment of a front.
type Slot = (u32, [f32; 3], u32);

/// A pixel as `fs_blend_once` files the layers that come to it, one after
/// the other, and as `blend_compose` composes it.
struct Pixel {
    /// The fragments of the front, as they came.
    slots: Vec<Slot>,
    /// The slots it may have: its own and those of the pages the pool has
    /// for it.
    room: usize,
    cut: u32,
    /// The low bits the places of its haze go without
    /// (`BlendOrderInfo::shift`).
    shift: u32,
    /// An alpha of this many 255ths or less takes no slot
    /// (`BlendOrderInfo::faint`).
    faint: u32,
    /// The haze: what it stops, the greatest of the reverse of its places
    /// and of its places, its light and its glow.
    stops: f32,
    far: u32,
    near: u32,
    haze: [f32; 3],
    haze_glow: f32,
}

impl Default for Pixel {
    fn default() -> Pixel {
        // (`BLEND_CHAIN` pages)
        Pixel::with_room(SLOTS as usize * 128)
    }
}

impl Pixel {
    fn with_room(room: usize) -> Pixel {
        Pixel {
            slots: Vec::new(),
            room,
            cut: 0,
            shift: 0,
            faint: FAINT,
            stops: 0.0,
            far: 0,
            near: 0,
            haze: [0.0; 3],
            haze_glow: 0.0,
        }
    }

    /// As the bench files it: with the places of the haze its device
    /// holds.
    fn of(bench: &Bench) -> Pixel {
        Pixel {
            shift: bench.shift,
            ..Pixel::default()
        }
    }

    fn file(&mut self, layer: &Layer) {
        let mut a8 = alpha8(layer.alpha);
        let shows = layer.alpha >= 0.004 && a8 != 0;
        let glow = glow8(layer.glow);
        if !shows {
            if glow == 0 {
                return;
            }
            a8 = 0;
        }
        let place = place(layer);
        let key = place << 8;
        if key < self.cut {
            return;
        }
        let mut light = [0.0; 3];
        if shows {
            let lit = layer.alpha.clamp(0.0, 1.0);
            light = layer.rgb.map(|c| c * lit);
            a8 = (lit * 255.0 + 0.5) as u32;
        }
        let cover = a8 as f32 / 255.0;
        if shows && 1.0 - cover < CULL {
            self.cut = self.cut.max(key);
        }
        if ((shows && a8 > self.faint) || glow > FAINT_GLOW) && self.slots.len() < self.room {
            self.slots.push(((key | a8).max(1), packed(light), glow));
            return;
        }
        // of the haze: too faint for a slot, or no page was left for it
        let stopped = stops(cover);
        self.stops += stopped;
        self.far = self.far.max((0x00ff_ffff - place) >> self.shift);
        self.near = self.near.max(place >> self.shift);
        let share = if a8 != 0 { stopped / cover } else { 1.0 };
        for (sum, c) in self.haze.iter_mut().zip(light) {
            *sum += c * share;
        }
        if glow != 0 {
            self.haze_glow += layer.glow;
        }
    }

    /// `fs_blend_cut`: the cut raised to where the fragments of the front
    /// let nothing through.
    fn raise_cut(&mut self) {
        let mut slots = self.slots.clone();
        slots.sort_by_key(|slot| std::cmp::Reverse(slot.0));
        let mut t = 1.0_f32;
        for (key, ..) in slots {
            t *= 1.0 - (key & 0xff) as f32 / 255.0;
            if t < CULL {
                self.cut = self.cut.max(key & 0xffff_ff00);
                break;
            }
        }
    }

    /// `blend_walk`: the fragments of the front over the cut, the nearest
    /// `most` of them from the nearest, and the others, its tail.
    fn split(&self, most: usize) -> (Vec<Slot>, Vec<Slot>) {
        // as they are read: its own slots, then its pages from the newest
        let own = self.slots.len().min(SLOTS as usize);
        let mut read: Vec<Slot> = self.slots[..own].to_vec();
        for page in self.slots[own..].chunks(SLOTS as usize).rev() {
            read.extend_from_slice(page);
        }
        read.retain(|slot| slot.0 >> 8 >= self.cut >> 8);
        read.sort_by_key(|slot| std::cmp::Reverse(slot.0 >> 8));
        let tail = read.split_off(most.min(read.len()));
        (read, tail)
    }

    /// Colour and glow of the pixel.
    fn composed(&self) -> [f32; 4] {
        let (slots, tail) = self.split(MOST);
        let step = (1u32 << self.shift) as f32;
        let near = (self.near as f32 + 1.0) * step - 1.0;
        let far = 16_777_216.0 - (self.far as f32 + 1.0) * step;
        let width = (near - far).max(1.0);
        let stops_all = self.stops;
        let mut light = [0.0_f32; 3];
        let mut glow = 0.0;
        let mut t = 1.0_f32;
        let mut shown = 0.0;
        let mut from = 0.0;
        let mut at = 0;
        while at < slots.len() {
            let place = slots[at].0 >> 8;
            let end = at + slots[at..].iter().take_while(|slot| slot.0 >> 8 == place).count();
            let group = &slots[at..end];
            let tie_t: f32 = group.iter().map(|slot| 1.0 - (slot.0 & 0xff) as f32 / 255.0).product();
            let tie_a: f32 = group.iter().map(|slot| (slot.0 & 0xff) as f32 / 255.0).sum();
            let to = ((near - place as f32) / width).clamp(0.0, 1.0);
            shown += t * slab(stops_all, from, to);
            from = to;
            let before = t * (-stops_all * to).exp();
            let share = before * (1.0 - tie_t) / tie_a.max(1.0e-6);
            for (_, lit, glows) in group {
                for (sum, c) in light.iter_mut().zip(lit) {
                    *sum += c * share;
                }
                glow += unglow8(*glows) * before;
            }
            t *= tie_t;
            at = end;
            if t < CULL {
                break;
            }
        }
        shown += t * slab(stops_all, from, 1.0);
        for (sum, c) in light.iter_mut().zip(self.haze) {
            *sum += c * shown;
        }
        glow += self.haze_glow * shown;
        t *= (-stops_all).exp();
        // the tail: one layer behind all of them, in which its fragments
        // are mixed
        let mut tail_stops = 0.0;
        let mut tail_light = [0.0_f32; 3];
        let mut tail_glow = 0.0;
        for (key, lit, glows) in tail {
            let cover = (key & 0xff) as f32 / 255.0;
            let mut share = 1.0;
            if cover > 0.0 {
                let stopped = stops(cover);
                tail_stops += stopped;
                share = stopped / cover;
            }
            for (sum, c) in tail_light.iter_mut().zip(lit) {
                *sum += c * share;
            }
            tail_glow += unglow8(glows);
        }
        let tail_shown = t * slab(tail_stops, 0.0, 1.0);
        for (sum, c) in light.iter_mut().zip(tail_light) {
            *sum += c * tail_shown;
        }
        glow += tail_glow * tail_shown;
        t *= (-tail_stops).exp();
        [
            light[0] + BACKGROUND[0] * t,
            light[1] + BACKGROUND[1] * t,
            light[2] + BACKGROUND[2] * t,
            SCENE_GLOW * t + glow,
        ]
    }
}

/// The pixel `layers` give, drawn in the order given.
fn filed_in_order(layers: &[Layer]) -> [f32; 4] {
    let mut pixel = Pixel::default();
    for layer in layers {
        pixel.file(layer);
    }
    pixel.composed()
}

fn function(object: &str, name: &str) -> String {
    let start = object.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("production {name}"));
    let end = start + object[start..].find("\n}").expect("function end") + 2;
    object[start..end]
        .replace("@builtin(front_facing) ", "")
        .replace("@builtin(position) ", "")
        .replace("@builtin(frag_depth) ", "")
        .replace("@location(0) ", "")
        + "\n"
}

fn between(object: &str, from: &str, to: &str) -> String {
    let start = object.find(from).unwrap_or_else(|| panic!("production {from}"));
    let end = start + object[start..].find(to).unwrap_or_else(|| panic!("production {to}")) + to.len();
    object[start..end].to_string() + "\n"
}

/// The production shader of the exact order with what it needs around it.
fn shader_source() -> String {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    // XDBG: the shader as it is on the disk, without a build
    let object = match std::env::var("AURORA_XT") {
        Ok(path) => std::fs::read_to_string(path).expect("AURORA_XT").replace("\r\n", "\n"),
        Err(_) => object,
    };
    let mut production = between(&object, "struct BlendOrderInfo {", "const BLEND_OPAQUE: f32 = 8.3;");
    for name in [
        "water_clip",
        "blend_pixel",
        "blend_slot",
        "blend_slot_light",
        "blend_page_at",
        "blend_page",
        "blend_glow_pack",
        "blend_glow_unpack",
        "blend_nan",
        "blend_finite",
        "blend_finite1",
        "blend_stops",
        "blend_place",
        "blend_pack",
        "blend_unpack",
        "fs_blend_once",
        "blend_cut_deep",
        "fs_blend_cut",
        "blend_slab",
        "blend_word",
        "blend_kept",
        "blend_walk",
        "blend_compose",
        "fs_blend_over",
        "fs_blend_glow",
    ] {
        if name == "fs_blend_once" {
            production += &between(&object, "struct BlendOnce {", "};");
        }
        if name == "blend_word" {
            production += &between(&object, "struct BlendPixel {", "};");
        }
        production += &function(&object, name);
    }
    format!(
        r#"
const FLAG_DOUBLE_SIDED:u32={double_sided}u;
const FLAG_PORTAL_EFFECT:u32={portal}u;
const FLAG_ALPHA_MASK:u32={mask}u;
const LAYER_RANK_SHIFT:u32=24u;
const LAYER_RANK_MAX:u32=7u;
const PORTAL_SURFACE_SCALE:f32=1.16;
const HDR_MAX:f32=65504.0;
struct DrawRecord {{flags:vec4<u32>,tex:vec4<u32>,base_color:vec4<f32>,params:vec4<f32>,emissive:vec4<f32>,rez:vec4<u32>}};
@group(0) @binding(0) var<storage,read> records:array<DrawRecord>;
struct Frame {{sky_cloud_pd2:vec4<f32>,params:vec4<f32>,camera_pos:vec4<f32>}};
const frame=Frame(vec4<f32>(1.0),vec4<f32>(0.0),vec4<f32>(0.0,0.0,1.0,1.0));
override WATER_CLIP:u32=0u;
var<private> rez_fw:vec3<f32>;
struct VsOut {{@builtin(position) clip:vec4<f32>,@location(0) @interpolate(flat) record:u32,
    @location(1) world_pos:vec3<f32>,@location(2) uv:vec2<f32>}};
struct Material {{color:vec4<f32>}};
struct Shaded {{color:vec4<f32>}};
struct Surface {{alpha:f32,color:vec3<f32>}};
fn portal_surface(r:DrawRecord,uv:vec2<f32>,aa:f32)->Surface {{return Surface(1.0,vec3<f32>(1.0));}}
fn apply_fog(c:vec3<f32>,p:vec3<f32>)->vec3<f32> {{return c;}}
fn eval_material(v:VsOut,front:bool)->Material {{return Material(records[v.record].base_color);}}
fn legacy_may_glare(r:DrawRecord,m:Material,p:vec3<f32>,at:vec2<f32>)->bool {{return false;}}
fn shade_material(v:VsOut,m:Material)->Shaded {{return Shaded(m.color);}}
fn rez_alpha(r:DrawRecord,v:VsOut)->f32 {{return 1.0;}}
fn blend_glow(v:VsOut,dx:vec2<f32>,dy:vec2<f32>)->f32 {{return records[v.record].params.y;}}
{production}
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->VsOut {{
    let positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return VsOut(vec4<f32>(positions[vertex],records[instance].params.x,1.0),instance,vec3<f32>(0.0,0.0,1.0),vec2<f32>(0.5));
}}
@fragment
@early_depth_test(force)
fn once(v:VsOut,@builtin(front_facing) front:bool)->BlendOnce {{return fs_blend_once(v,front);}}
@vertex fn vs_whole(@builtin(vertex_index) vertex:u32)->@builtin(position) vec4<f32> {{
    let positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(positions[vertex],0.0,1.0);
}}
@fragment fn cut(@builtin(position) at:vec4<f32>)->@builtin(frag_depth) f32 {{return fs_blend_cut(at);}}
@fragment fn over(@builtin(position) at:vec4<f32>)->@location(0) vec4<f32> {{return fs_blend_over(at);}}
@fragment fn glow(@builtin(position) at:vec4<f32>)->@location(0) vec4<f32> {{return fs_blend_glow(at);}}
"#,
        double_sided = flags::DOUBLE_SIDED,
        portal = flags::PORTAL_EFFECT,
        mask = flags::ALPHA_MASK,
    )
}

/// How the layers of a view are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drawn {
    /// In one pass, as the renderer draws them: their shaders may run
    /// together.
    Together,
    /// In one pass, the cut of the pixels into the depth after each.
    Cut,
    /// Each one finished before the next: what `Pixel` says.
    Apart,
}

struct Bench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    lists: Lists,
    /// The low bits the places of a haze go without on this device.
    shift: u32,
    records_layout: wgpu::BindGroupLayout,
    /// `Pipelines::blend_once`, `blend_cut`, `blend_over`, `blend_glow` and
    /// `blend_both`.
    once: wgpu::RenderPipeline,
    cut: wgpu::RenderPipeline,
    over: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    both: Option<wgpu::RenderPipeline>,
    target: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Bench {
    fn new() -> Option<Bench> {
        Bench::with(POOL, false)
    }

    /// With a pool of `pages` for the fronts of its view; `narrow`: as a
    /// device without blending into 32-bit floats.
    fn with(pages: u64, narrow: bool) -> Option<Bench> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        if !adapter.features().contains(EARLY_TEST) {
            // (the blended faces are drawn sorted on such a device)
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: EARLY_TEST | (adapter.features() & (if narrow { TWO_SOURCES } else { TWO_SOURCES | WIDE_RANGE })),
            ..Default::default()
        }))
        .expect("blend order GPU device");
        let two_sources = device.features().contains(TWO_SOURCES);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production exact blend order"),
            source: wgpu::ShaderSource::Wgsl(scene_source(shader_source(), device.features()).into()),
        });
        let records_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layers"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let format = wgpu::TextureFormat::Rgba8Unorm;
        // the blends of the renderer's pipelines
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let summed = wgpu::BlendState { color: add, alpha: add };
        let laid_over = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let order_layouts = layouts(&device);
        let haze_blend = once_blend();
        let range_format = range_format(device.features());
        struct Desc {
            entry: &'static str,
            whole: bool,
            /// Whether the depth is written, when there is one.
            depth: Option<bool>,
            format: wgpu::TextureFormat,
            blend: wgpu::BlendState,
            write_mask: wgpu::ColorWrites,
            /// With the second target of the pass of the layers after its own.
            range: bool,
        }
        let pipeline = |fronts: &wgpu::BindGroupLayout, d: Desc| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(d.entry),
                bind_group_layouts: &[Some(&records_layout), None, None, Some(fronts)],
                immediate_size: 0,
            });
            let target = Some(wgpu::ColorTargetState {
                format: d.format,
                blend: Some(d.blend),
                write_mask: d.write_mask,
            });
            let mut targets = vec![target];
            if d.range {
                targets.push(Some(wgpu::ColorTargetState {
                    format: range_format,
                    blend: Some(range_blend()),
                    write_mask: d.write_mask,
                }));
            }
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(d.entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(if d.whole { "vs_whole" } else { "vs" }),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: d.depth.map(|write| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(if write {
                        wgpu::CompareFunction::Greater
                    } else {
                        wgpu::CompareFunction::GreaterEqual
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(d.entry),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let once = pipeline(
            &order_layouts[0],
            Desc {
                entry: "once",
                whole: false,
                depth: Some(false),
                format: HAZE_FORMAT,
                blend: haze_blend,
                write_mask: wgpu::ColorWrites::ALL,
                range: true,
            },
        );
        let cut = pipeline(
            &order_layouts[0],
            Desc {
                entry: "cut",
                whole: true,
                depth: Some(true),
                format: HAZE_FORMAT,
                blend: haze_blend,
                write_mask: wgpu::ColorWrites::empty(),
                range: true,
            },
        );
        let over = pipeline(
            &order_layouts[1],
            Desc {
                entry: "over",
                whole: true,
                depth: None,
                format,
                blend: laid_over,
                write_mask: wgpu::ColorWrites::ALL,
                range: false,
            },
        );
        let glow = pipeline(
            &order_layouts[1],
            Desc {
                entry: "glow",
                whole: true,
                depth: None,
                format,
                blend: summed,
                write_mask: wgpu::ColorWrites::ALPHA,
                range: false,
            },
        );
        let both = two_sources.then(|| {
            pipeline(
                &order_layouts[1],
                Desc {
                    entry: "fs_blend_both",
                    whole: true,
                    depth: None,
                    format,
                    blend: both_blend(),
                    write_mask: wgpu::ColorWrites::ALL,
                    range: false,
                },
            )
        });
        let lists = Lists::with_pool(&device, (SIZE, SIZE), wgpu::TextureFormat::Depth32Float, pages)?;
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let (target, view) = crate::renderer::make_view(&device, "blend order pixels", SIZE, SIZE, format, 1, usage);
        Some(Bench {
            device,
            queue,
            lists,
            shift: range_shift(range_format),
            records_layout,
            once,
            cut,
            over,
            glow,
            both,
            target,
            view,
        })
    }

    /// One view: `layers` drawn in the order given, in front of an opaque
    /// scene at `wall`, over the background; colour and glow of every pixel.
    fn draw(&self, layers: &[Layer], wall: f32, drawn: Drawn) -> Vec<[f32; 4]> {
        // (a view without a layer still binds a record)
        let none = [layer(WHITE, 0.0, 0.0)];
        let records: Vec<[u32; 24]> = if layers.is_empty() { &none[..] } else { layers }
            .iter()
            .map(|l| {
                let mut rec = [0; 24];
                rec[0] = flags::ALPHA_BLEND | flags::DOUBLE_SIDED;
                rec[3] = l.rank << 24;
                for (word, c) in rec[8..11].iter_mut().zip(l.rgb) {
                    *word = c.to_bits();
                }
                rec[11] = l.alpha.to_bits();
                rec[12] = l.z.to_bits();
                rec[13] = l.glow.to_bits();
                // (emissive.w: what makes the pass ask for its glow)
                rec[19] = l.glow.to_bits();
                rec
            })
            .collect();
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("layers"),
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layers"),
            layout: &self.records_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blend order readback"),
            size: crate::readback::readback_size(u64::from(SIZE) * 256),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        // the pass of the layers `range`, the first of them after the
        // targets are emptied
        let once = |encoder: &mut wgpu::CommandEncoder, range: std::ops::Range<u32>| {
            let first = range.start == 0;
            let target = |view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if first {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[target(&self.lists.haze), target(&self.lists.range)],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.lists.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: if first { wgpu::LoadOp::Clear(wall) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(3, &self.lists.write, &[]);
            for i in range {
                pass.set_pipeline(&self.once);
                pass.draw(0..3, i..i + 1);
                if drawn == Drawn::Cut {
                    pass.set_pipeline(&self.cut);
                    pass.draw(0..3, 0..1);
                }
            }
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.lists.clear(&mut encoder);
        let n = layers.len() as u32;
        if drawn == Drawn::Apart {
            for i in 0..n {
                once(&mut encoder, i..i + 1);
                self.queue.submit([encoder.finish()]);
                self.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("blend order layer");
                encoder = self.device.create_command_encoder(&Default::default());
            }
        }
        if drawn != Drawn::Apart || n == 0 {
            once(&mut encoder, 0..n);
        }
        // over the background, in two draws then in one where the device can:
        // the same pixels
        let two = self.shown(encoder, &group, &readback, false);
        if self.both.is_some() {
            let one = self.shown(self.device.create_command_encoder(&Default::default()), &group, &readback, true);
            for (a, b) in two.iter().zip(&one) {
                assert!(near(a, b, 1.5 / 255.0), "in one draw {b:?}, in two {a:?}");
            }
        }
        two
    }

    /// The pass over the screen of the view drawn so far, in one draw or in
    /// two; colour and glow of every pixel.
    fn shown(&self, mut encoder: wgpu::CommandEncoder, group: &wgpu::BindGroup, readback: &wgpu::Buffer, one_draw: bool) -> Vec<[f32; 4]> {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(BACKGROUND[0]),
                            g: f64::from(BACKGROUND[1]),
                            b: f64::from(BACKGROUND[2]),
                            a: f64::from(SCENE_GLOW),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(3, &self.lists.read, &[]);
            match self.both.as_ref().filter(|_| one_draw) {
                Some(both) => {
                    pass.set_pipeline(both);
                    pass.draw(0..3, 0..1);
                }
                None => {
                    pass.set_pipeline(&self.over);
                    pass.draw(0..3, 0..1);
                    pass.set_pipeline(&self.glow);
                    pass.draw(0..3, 0..1);
                }
            }
        }
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            self.target.size(),
        );
        self.queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("blend order readback"));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("blend order completion");
        let pixels = {
            let bytes = readback.get_mapped_range(..).expect("blend order pixels");
            (0..SIZE as usize * SIZE as usize)
                .map(|i| {
                    let at = (i / SIZE as usize) * 256 + (i % SIZE as usize) * 4;
                    std::array::from_fn(|c| f32::from(bytes[at + c]) / 255.0)
                })
                .collect()
        };
        readback.unmap();
        pixels
    }

    /// Colour and glow every pixel of a view of `layers` has.
    fn pixel(&self, layers: &[Layer], wall: f32, drawn: Drawn, case: &str) -> [f32; 4] {
        let pixels = self.draw(layers, wall, drawn);
        for p in &pixels {
            assert!(
                near(&p[..], &pixels[0], 1.5 / 255.0),
                "{case}: every pixel alike, {p:?} and {:?}",
                pixels[0]
            );
        }
        pixels[0]
    }

    /// The colour of `pixel`.
    fn colour(&self, layers: &[Layer], wall: f32, drawn: Drawn, case: &str) -> [f32; 3] {
        let p = self.pixel(layers, wall, drawn, case);
        [p[0], p[1], p[2]]
    }
}

fn near(a: &[f32], b: &[f32], by: f32) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= by)
}

/// Every order of `items`.
fn orders<T: Copy>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut all = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let first = rest.remove(i);
        for mut tail in orders(&rest) {
            tail.insert(0, first);
            all.push(tail);
        }
    }
    all
}

const RED: [f32; 3] = [1.0, 0.1, 0.1];
const GREEN: [f32; 3] = [0.1, 1.0, 0.1];
const BLUE: [f32; 3] = [0.1, 0.1, 1.0];
const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

fn layer(rgb: [f32; 3], alpha: f32, z: f32) -> Layer {
    Layer {
        rgb,
        alpha,
        z,
        rank: 0,
        glow: 0.0,
    }
}

/// The rounding of the 8-bit target of the test, and that of the lights of
/// a front (`blend_pack`: 1/512 of the brightest part of each).
const EXACT: f32 = 2.0 / 255.0;

const EVERY_WAY: [Drawn; 3] = [Drawn::Together, Drawn::Cut, Drawn::Apart];

/// How far from their sort the layers of a pixel without a page may be: six
/// of them, of an alpha of 0.12, mixed in its haze (the six that came last;
/// twice that for any six, where the device lights them together).
const NO_PAGE: f32 = 0.05;

#[test]
fn layers_drawn_in_any_order_compose_as_their_sort_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    // four layers, the 24 orders they can be drawn in: one result, the
    // sort's (and the fronts of each view are emptied for the next)
    let four = [
        layer(RED, 0.5, 0.8),
        layer(GREEN, 0.25, 0.6),
        layer(BLUE, 0.75, 0.4),
        layer(WHITE, 0.4, 0.2),
    ];
    let want = sorted(&four);
    for order in orders(&four) {
        let drawn: Vec<f32> = order.iter().map(|l| l.z).collect();
        for way in [Drawn::Together, Drawn::Cut] {
            let got = bench.colour(&order, 0.0, way, "four layers");
            assert!(near(&got, &want, EXACT), "drawn {drawn:?}: {got:?}, sorted {want:?}");
        }
        assert!(near(&filed_in_order(&order)[..3], &want, EXACT), "drawn {drawn:?}");
    }
    // the sort is not the order of drawing: back to front and front to
    // back differ without it
    let naive = |layers: &[Layer]| {
        layers.iter().fold(BACKGROUND, |under, l| {
            std::array::from_fn(|i| under[i] * (1.0 - l.alpha) + l.rgb[i] * l.alpha)
        })
    };
    let mut reversed = four;
    reversed.reverse();
    assert!(near(&naive(&reversed), &want, EXACT) && !near(&naive(&four), &want, 0.1));
    // as many layers as a front holds
    let full: Vec<Layer> = (0..SLOTS as usize)
        .map(|i| layer([RED, GREEN, BLUE][i % 3], 0.3, 0.1 + 0.8 * i as f32 / SLOTS as f32))
        .collect();
    let want = sorted(&full);
    let mut shuffled = full.clone();
    shuffled.sort_by_key(|l| ((l.z * 1000.0) as u32 * 7919) % 31);
    let reversed: Vec<Layer> = full.iter().rev().copied().collect();
    for order in [&full, &reversed, &shuffled] {
        for way in EVERY_WAY {
            let got = bench.colour(order, 0.0, way, "a full front");
            assert!(near(&got, &want, EXACT), "{got:?}, sorted {want:?}");
        }
    }
}

#[test]
fn layers_lying_on_one_another_share_the_light_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    // two layers at one depth, under a third: neither is over the other
    let tie = [layer(RED, 0.6, 0.5), layer(BLUE, 0.6, 0.5), layer(GREEN, 0.3, 0.7)];
    let want = sorted(&tie);
    for order in orders(&tie) {
        let got = bench.colour(&order, 0.0, Drawn::Together, "two on one depth");
        assert!(near(&got, &want, EXACT), "{got:?}, sorted {want:?}");
    }
    // as much red as blue, over what the background leaves of each
    let behind: f32 = tie.iter().map(|l| 1.0 - filed(l.alpha)).product();
    let over = |i: usize| want[i] - BACKGROUND[i] * behind;
    assert!((over(0) - over(2)).abs() < 1.0e-4);
    // a layer rank orders two layers of one depth: the higher one is over
    let ranked = |red: u32, blue: u32| {
        [
            Layer {
                rank: red,
                ..layer(RED, 0.6, 0.5)
            },
            Layer {
                rank: blue,
                ..layer(BLUE, 0.6, 0.5)
            },
        ]
    };
    for (red, blue) in [(1, 0), (0, 1), (7, 3)] {
        let layers = ranked(red, blue);
        let want = sorted(&layers);
        for order in orders(&layers) {
            let got = bench.colour(&order, 0.0, Drawn::Together, "two ranks on one depth");
            assert!(near(&got, &want, EXACT), "ranks {red}/{blue}: {got:?}, sorted {want:?}");
        }
        assert_eq!(want[0] > want[2], red > blue);
    }
}

#[test]
fn the_opaque_scene_hides_the_layers_behind_it_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    // a wall at 0.5: the blue layer behind it neither shows nor dims
    let layers = [layer(BLUE, 0.9, 0.3), layer(RED, 0.5, 0.8), layer(GREEN, 0.5, 0.5)];
    let want = sorted(&[layers[1], layers[2]]);
    for order in orders(&layers) {
        for way in [Drawn::Together, Drawn::Cut] {
            let got = bench.colour(&order, 0.5, way, "a wall");
            assert!(near(&got, &want, EXACT), "{got:?}, sorted {want:?}");
        }
    }
    // no layer at all: the scene as it was, colour and glow
    let got = bench.pixel(&[], 0.0, Drawn::Together, "no layer");
    assert!(near(&got, &[BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], SCENE_GLOW], EXACT), "{got:?}");
}

#[test]
fn more_layers_than_a_pixel_has_slots_compose_as_their_sort_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    let stack = |count: usize| -> Vec<Layer> {
        (0..count)
            .map(|i| layer([RED, GREEN, BLUE][i % 3], 0.12, 0.1 + 0.8 * i as f32 / count as f32))
            .collect()
    };
    let shuffle = |layers: &[Layer]| {
        let mut shuffled = layers.to_vec();
        // (a fixed shuffle)
        shuffled.sort_by_key(|l| ((l.z * 1000.0) as u32 * 7919) % 31);
        shuffled
    };
    // twice the slots of a pixel, then all that its pages hold, faint
    // enough for all of them to show: their sort, in whatever order they
    // come and however they are drawn
    for count in [SLOTS as usize * 2, MOST] {
        let deep = stack(count);
        let nearest_first: Vec<Layer> = deep.iter().rev().copied().collect();
        let shuffled = shuffle(&deep);
        let want = sorted(&deep);
        // what all the layers let through of the scene: its glow, which
        // none of them adds to
        let through: f32 = deep.iter().map(|l| 1.0 - filed(l.alpha)).product();
        for (name, order) in [("farthest first", &deep), ("nearest first", &nearest_first), ("shuffled", &shuffled)] {
            assert!(near(&filed_in_order(order)[..3], &want, 2.0e-3), "{count} layers, {name}");
            for way in EVERY_WAY {
                for got in bench.draw(order, 0.0, way) {
                    assert!(
                        near(&got[..3], &want, EXACT),
                        "{count} layers, {name}, {way:?}: {got:?}, sorted {want:?}"
                    );
                    assert!((got[3] - SCENE_GLOW * through).abs() <= EXACT, "{} of {}", got[3], SCENE_GLOW * through);
                }
            }
        }
        // an opaque layer in front: nothing of the others shows, whenever
        // it comes
        for at in [0, 3, count] {
            let mut capped = nearest_first.clone();
            capped.insert(at, layer(WHITE, 1.0, 0.95));
            for way in EVERY_WAY {
                let got = bench.pixel(&capped, 0.0, way, "deep under an opaque layer");
                assert!(near(&got, &[1.0, 1.0, 1.0, 0.0], EXACT), "{count} layers: {got:?}");
            }
        }
    }
    // more than a pixel keeps: every one of them has a slot, the nearest
    // are composed each in its place whatever order they come in and
    // however they are drawn, and the others are its tail, as the steps of
    // the shader say. One pixel, the same in every order.
    let over = stack(MOST + 8);
    let nearest_first: Vec<Layer> = over.iter().rev().copied().collect();
    let shuffled = shuffle(&over);
    let orders = [("farthest first", &over), ("nearest first", &nearest_first), ("shuffled", &shuffled)];
    let mut every = Vec::new();
    for (name, order) in orders {
        let mut pixel = Pixel::default();
        for l in order {
            pixel.file(l);
        }
        assert_eq!(pixel.slots.len(), MOST + 8);
        let (kept, tail) = pixel.split(MOST);
        assert_eq!((kept.len(), tail.len()), (MOST, 8));
        let last = kept.last().map(|slot| slot.0);
        assert_eq!(last, Some((place(&over[8]) << 8) | alpha8(over[8].alpha)), "{name}: the nearest are kept");
        let want = pixel.composed();
        every.push(want);
        // (the cut, where it is raised between the layers, hides what shows
        // for less than `CULL`)
        for (way, by) in [(Drawn::Apart, EXACT), (Drawn::Together, EXACT), (Drawn::Cut, EXACT + CULL)] {
            let got = bench.pixel(order, 0.0, way, name);
            assert!(near(&got, &want, by), "{name}, {way:?}: {got:?}, filed {want:?}");
        }
    }
    for want in &every {
        assert!(near(want, &every[0], 1.0e-4), "one pixel in every order: {want:?}, {:?}", every[0]);
    }
    // what is wrong lies under all the others, the light they let through
    // at most, however the layers are drawn
    let want = sorted(&over);
    let under: f32 = over[8..].iter().map(|l| 1.0 - l.alpha).product();
    for (name, order) in orders {
        for way in EVERY_WAY {
            for got in bench.draw(order, 0.0, way) {
                assert!(near(&got[..3], &want, under + EXACT), "{name}, {way:?}: {got:?}, sorted {want:?}");
            }
        }
    }
    // an opaque layer that comes last, to a full pixel, hides them all
    let mut capped = shuffled.clone();
    capped.push(layer(WHITE, 1.0, 0.95));
    for way in EVERY_WAY {
        let got = bench.pixel(&capped, 0.0, way, "a full pixel under an opaque layer");
        assert!(near(&got, &[1.0, 1.0, 1.0, 0.0], EXACT), "{way:?}: {got:?}");
    }
}

#[test]
fn a_pixel_without_a_page_mixes_the_layers_past_its_slots_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    // no pool at all: a pixel has its own slots, and the layers that come
    // after them are of its haze
    let Some(bench) = Bench::with(0, false) else {
        return;
    };
    let count = SLOTS as usize * 2;
    let deep: Vec<Layer> = (0..count)
        .map(|i| layer([RED, GREEN, BLUE][i % 3], 0.12, 0.1 + 0.8 * i as f32 / count as f32))
        .collect();
    let nearest_first: Vec<Layer> = deep.iter().rev().copied().collect();
    let mut shuffled = deep.clone();
    shuffled.sort_by_key(|l| ((l.z * 1000.0) as u32 * 7919) % 31);
    let sort = sorted(&deep);
    for (name, order) in [("farthest first", &deep), ("nearest first", &nearest_first), ("shuffled", &shuffled)] {
        let mut pixel = Pixel {
            room: SLOTS as usize,
            ..Pixel::of(&bench)
        };
        for l in order {
            pixel.file(l);
        }
        // the first that came have the slots
        let kept: Vec<u32> = pixel.slots.iter().map(|slot| slot.0 >> 8).collect();
        let first: Vec<u32> = order[..SLOTS as usize].iter().map(place).collect();
        assert_eq!(kept, first, "{name}");
        let want = pixel.composed();
        let got = bench.pixel(order, 0.0, Drawn::Apart, name);
        assert!(near(&got, &want, EXACT), "{name}: {got:?}, filed {want:?}");
        // not far from their sort: the others, layers this faint, are
        // mixed among them
        assert!(near(&want[..3], &sort, NO_PAGE), "{name}: {want:?}, sorted {sort:?}");
        for way in [Drawn::Together, Drawn::Cut] {
            for got in bench.draw(order, 0.0, way) {
                assert!(near(&got[..3], &sort, 2.0 * NO_PAGE), "{name}, {way:?}: {got:?}, sorted {sort:?}");
            }
        }
    }
    // an opaque layer that comes last, and finds no slot, hides the scene
    // and is mixed with the others of the haze, of which it is most
    let mut capped = shuffled.clone();
    capped.push(layer(WHITE, 1.0, 0.95));
    for way in EVERY_WAY {
        for got in bench.draw(&capped, 0.0, way) {
            assert!(near(&got, &[1.0, 1.0, 1.0, 0.0], 2.0 * NO_PAGE), "{way:?}: {got:?}");
        }
    }
    // a view of fewer layers than slots needs no pool
    let four = [
        layer(RED, 0.5, 0.8),
        layer(GREEN, 0.25, 0.6),
        layer(BLUE, 0.75, 0.4),
        layer(WHITE, 0.4, 0.2),
    ];
    let got = bench.colour(&four, 0.0, Drawn::Together, "four layers, no pool");
    assert!(near(&got, &sorted(&four), EXACT), "{got:?}");
}

#[test]
fn faint_layers_take_no_slot_and_dim_what_lies_behind_them_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    // (the second time as a device that keeps the places of a haze without
    // their low bits)
    for narrow in [false, true] {
        let Some(bench) = Bench::with(POOL, narrow) else {
            return;
        };
        assert!(!narrow || bench.shift != 0);
        faint_layers(&bench);
    }
}

fn faint_layers(bench: &Bench) {
    // ten layers next to clear (the texels a lossy texture leaves in its
    // empty areas) around two strong ones: more than a front holds, and the
    // strong ones still each in their order
    let mut layers: Vec<Layer> = (0..10).map(|i| layer(WHITE, 0.02, 0.9 - 0.05 * i as f32)).collect();
    layers.push(layer(RED, 0.6, 0.72));
    layers.push(layer(BLUE, 0.6, 0.47));
    let nearest_first = {
        let mut order = layers.clone();
        order.sort_by(|a, b| b.z.partial_cmp(&a.z).expect("finite depths"));
        order
    };
    let mut pixel = Pixel::of(bench);
    for l in &nearest_first {
        pixel.file(l);
    }
    assert_eq!(pixel.slots.len(), 2, "the strong layers alone in the front");
    let want = pixel.composed();
    let got = bench.pixel(&nearest_first, 0.0, Drawn::Apart, "faint layers");
    assert!(near(&got, &want, EXACT), "{got:?}, filed {want:?}");
    // within an alpha times an alpha of their sort, in whatever order they
    // come: the haze is sums
    let sort = sorted(&layers);
    let farthest_first: Vec<Layer> = nearest_first.iter().rev().copied().collect();
    for order in [&nearest_first, &farthest_first, &layers] {
        assert!(near(&filed_in_order(order)[..3], &sort, 0.02), "{:?}", filed_in_order(order));
        for way in EVERY_WAY {
            let got = bench.colour(order, 0.0, way, "faint layers, sorted");
            assert!(near(&got, &sort, 0.02 + EXACT), "{got:?}, sorted {sort:?}");
        }
    }
    // one faint layer alone, before or behind a strong one: exact
    for z in [0.9, 0.3] {
        let two = [layer(WHITE, 0.03, z), layer(RED, 0.6, 0.6)];
        let want = sorted(&two);
        for order in orders(&two) {
            let got = bench.colour(&order, 0.0, Drawn::Apart, "one faint layer");
            assert!(near(&got, &want, EXACT), "faint at {z}: {got:?}, sorted {want:?}");
        }
    }
    // a faint glow on a faint layer takes no slot either: summed with the
    // haze, and dimmed by what is before it
    let dim = |z: f32| Layer {
        glow: 0.004,
        ..layer(WHITE, 0.02, z)
    };
    assert!(glow8(0.004) != 0 && glow8(0.004) <= FAINT_GLOW);
    let glows = [dim(0.3), dim(0.25), layer(RED, 0.5, 0.6)];
    let mut pixel = Pixel::of(bench);
    for l in &glows {
        pixel.file(l);
    }
    assert_eq!(pixel.slots.len(), 1, "the strong layer alone in the front");
    let want = pixel.composed();
    let half = 1.0 - filed(0.5);
    let through = half * (1.0 - filed(0.02)).powi(2);
    assert!((want[3] - (SCENE_GLOW * through + 0.008 * half)).abs() < 2.0e-4, "{}", want[3]);
    for order in orders(&glows) {
        for way in EVERY_WAY {
            let got = bench.pixel(&order, 0.0, way, "faint glows");
            assert!(near(&got, &want, EXACT), "{way:?}: {got:?}, filed {want:?}");
        }
    }
}

/// How far from their sort two layers of an alpha of 0.1 may be, mixed in
/// the haze around a strong one (`BlendPrecision::Balanced`).
const BALANCED: f32 = 0.05;

#[test]
fn a_coarser_precision_mixes_fainter_layers_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    // two soft layers (26/255) around a strong one
    let layers = [layer(WHITE, 0.1, 0.8), layer(RED, 0.6, 0.6), layer(BLUE, 0.1, 0.4)];
    let sort = sorted(&layers);
    // at the finest level each in its place, in whatever order they come
    for order in orders(&layers) {
        for way in EVERY_WAY {
            let got = bench.colour(&order, 0.0, way, "soft layers, full");
            assert!(near(&got, &sort, EXACT), "{way:?}: {got:?}, sorted {sort:?}");
        }
    }
    // at a coarser one they are of the haze: the pixel the shader says,
    // the same in any order, a little off their sort
    let faint = crate::BlendPrecision::Balanced.faint();
    assert!(alpha8(0.1) > FAINT && alpha8(0.1) <= faint);
    bench.lists.set_faint(&bench.queue, faint);
    for order in orders(&layers) {
        let mut pixel = Pixel {
            faint,
            ..Pixel::of(&bench)
        };
        for l in &order {
            pixel.file(l);
        }
        assert_eq!(pixel.slots.len(), 1, "the strong layer alone in the front");
        let want = pixel.composed();
        assert!(near(&want[..3], &sort, BALANCED), "{want:?}, sorted {sort:?}");
        for way in EVERY_WAY {
            let got = bench.pixel(&order, 0.0, way, "soft layers, balanced");
            assert!(near(&got, &want, EXACT), "{way:?}: {got:?}, filed {want:?}");
        }
    }
}

#[test]
fn a_light_that_is_not_a_number_stays_out_of_the_pixel_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    // a layer whose light is not a number, of the haze then of the front:
    // lit black, it still dims what lies behind it
    for alpha in [0.02, 0.5] {
        let bad = [layer([f32::NAN; 3], alpha, 0.5), layer(GREEN, 0.4, 0.3)];
        let black = [layer([0.0; 3], alpha, 0.5), layer(GREEN, 0.4, 0.3)];
        for way in EVERY_WAY {
            let want = bench.pixel(&black, 0.0, way, "a black layer");
            assert!(want[1] > 0.1, "the layer behind shows: {want:?}");
            for order in orders(&bad) {
                let got = bench.pixel(&order, 0.0, way, "a light that is not a number");
                assert!(near(&got, &want, EXACT), "alpha {alpha}, {way:?}: {got:?}, black {want:?}");
            }
        }
    }
    // a faint layer brighter than the target of the haze holds: behind an
    // opaque one nothing of it shows (its sum is infinite, and infinite
    // times nothing is not a number), before it it is as bright as can be
    let hidden = [layer([1.0e30; 3], 0.02, 0.3), layer(RED, 1.0, 0.9)];
    let shown = [layer([1.0e30; 3], 0.02, 0.9), layer(RED, 1.0, 0.3)];
    for way in EVERY_WAY {
        for order in orders(&hidden) {
            let got = bench.colour(&order, 0.0, way, "a hidden bright layer");
            assert!(near(&got, &RED, EXACT), "{way:?}: {got:?}");
        }
        for order in orders(&shown) {
            let got = bench.colour(&order, 0.0, way, "a bright layer");
            assert!(got.iter().all(|c| *c > 0.99), "{way:?}: {got:?}");
        }
    }
    // an infinite glow is the brightest a slot holds
    let glowing = |glow: f32| [Layer { glow, ..layer(WHITE, 0.5, 0.5) }];
    let want = bench.pixel(&glowing(4.0), 0.0, Drawn::Apart, "a bright glow");
    let got = bench.pixel(&glowing(f32::INFINITY), 0.0, Drawn::Apart, "an infinite glow");
    assert!(near(&got, &want, EXACT), "{got:?}, {want:?}");
}

#[test]
fn what_a_stack_hides_stays_hidden_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    // three layers that let 0.1 % through together, two layers behind them:
    // whichever comes after the stack leaves, on what is filed or on the cut
    let stack = [layer(WHITE, 0.9, 0.8), layer(GREEN, 0.9, 0.7), layer(WHITE, 0.9, 0.6)];
    let hidden = [layer(RED, 0.8, 0.4), layer(BLUE, 0.6, 0.3)];
    let all: Vec<Layer> = stack.iter().chain(&hidden).copied().collect();
    let want = sorted(&all);
    for order in orders(&all) {
        let drawn: Vec<f32> = order.iter().map(|l| l.z).collect();
        for way in [Drawn::Together, Drawn::Cut] {
            let got = bench.colour(&order, 0.0, way, "a stack over two layers");
            assert!(near(&got, &want, EXACT), "drawn {drawn:?}: {got:?}, sorted {want:?}");
        }
    }
    // nearest first: the pass of the cut finds what the stack hides, under
    // its last layer, and writes it into the depth before the others come
    let mut pixel = Pixel::default();
    for l in &stack {
        pixel.file(l);
    }
    assert_eq!(pixel.cut, 0, "none of them is opaque");
    pixel.raise_cut();
    assert_eq!(pixel.cut, place(&stack[2]) << 8);
    // an opaque layer says so itself, as it comes
    let mut pixel = Pixel::default();
    pixel.file(&layer(WHITE, 1.0, 0.6));
    assert_eq!(pixel.cut, place(&stack[2]) << 8);
    // a layer nearer than the stack, come after the cut was written into
    // the depth, still shows
    let mut late = all.clone();
    late.push(layer(BLUE, 0.5, 0.9));
    let want = sorted(&late);
    for way in EVERY_WAY {
        let got = bench.colour(&late, 0.0, way, "a layer before the stack, last");
        assert!(near(&got, &want, EXACT), "{got:?}, sorted {want:?}");
    }
}

#[test]
fn the_glow_of_a_layer_is_dimmed_by_those_before_it_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    let glowing = |glow: f32, alpha: f32, z: f32| Layer {
        glow,
        ..layer(RED, alpha, z)
    };
    // a glowing layer behind one that stops half the light: half its glow,
    // over what both leave of the glow of the scene, whichever is drawn
    // first
    let two = [layer(WHITE, 0.5, 0.8), glowing(0.6, 0.2, 0.5)];
    let through = (1.0 - filed(0.5)) * (1.0 - filed(0.2));
    let want = SCENE_GLOW * through + filed_glow(0.6) * (1.0 - filed(0.5));
    for order in orders(&two) {
        for way in EVERY_WAY {
            let got = bench.pixel(&order, 0.0, way, "a glow behind a layer");
            assert!((got[3] - want).abs() <= EXACT, "{} of {want}", got[3]);
            assert!(near(&got[..3], &sorted(&two), EXACT));
        }
        assert!((filed_in_order(&order)[3] - want).abs() < 1.0e-4);
    }
    // the shells of a lamp, each glowing and each dimming those behind it:
    // one glow and three quarters, not three, in whatever order they come
    let shells = [glowing(0.3, 0.5, 0.7), glowing(0.3, 0.5, 0.5), glowing(0.3, 0.5, 0.3)];
    let half = 1.0 - filed(0.5);
    let want = SCENE_GLOW * half.powi(3) + filed_glow(0.3) * (1.0 + half + half * half);
    for order in orders(&shells) {
        let drawn: Vec<f32> = order.iter().map(|l| l.z).collect();
        for way in EVERY_WAY {
            let got = bench.pixel(&order, 0.0, way, "glowing shells");
            assert!((got[3] - want).abs() <= EXACT, "drawn {drawn:?}: {} of {want}", got[3]);
        }
    }
    // a halo, a glow with next to no alpha, behind a layer and before it
    for (z, dimmed) in [(0.3, half), (0.9, 1.0)] {
        let halo = [layer(WHITE, 0.5, 0.6), glowing(0.5, 0.02, z)];
        let want = SCENE_GLOW * half * (1.0 - filed(0.02)) + filed_glow(0.5) * dimmed;
        for order in orders(&halo) {
            let got = bench.pixel(&order, 0.0, Drawn::Together, "a halo");
            assert!((got[3] - want).abs() <= EXACT, "halo at {z}: {} of {want}", got[3]);
        }
    }
    // a glowing texel that shows nothing of itself still glows, and stops
    // nothing
    let bare = [glowing(0.3, 0.0, 0.5), layer(BLUE, 0.5, 0.3)];
    let want = sorted(&bare[1..]);
    for order in orders(&bare) {
        let got = bench.pixel(&order, 0.0, Drawn::Together, "a glow alone");
        let glow = SCENE_GLOW * half + filed_glow(0.3);
        assert!(near(&got, &[want[0], want[1], want[2], glow], EXACT), "{got:?}");
    }
    let got = bench.pixel(&bare[..1], 0.0, Drawn::Together, "a glow and nothing else");
    let glow = SCENE_GLOW + filed_glow(0.3);
    assert!(near(&got, &[BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], glow], EXACT), "{got:?}");
    // more glowing layers than a pixel has slots of its own: each dimmed by
    // those before it, in whatever order they come
    let count = SLOTS as usize * 2;
    let deep: Vec<Layer> = (0..count)
        .map(|i| glowing(0.04, 0.12, 0.1 + 0.8 * i as f32 / count as f32))
        .collect();
    let nearest_first: Vec<Layer> = deep.iter().rev().copied().collect();
    let mut exact = SCENE_GLOW;
    for l in &deep {
        exact = exact * (1.0 - filed(l.alpha)) + filed_glow(l.glow);
    }
    for (name, order) in [("farthest first", &deep), ("nearest first", &nearest_first)] {
        assert!((filed_in_order(order)[3] - exact).abs() < 1.0e-4, "{name}");
        for way in EVERY_WAY {
            let got = bench.pixel(order, 0.0, way, "deep glows");
            assert!((got[3] - exact).abs() <= EXACT, "{name}, {way:?}: {} of {exact}", got[3]);
        }
    }
}
