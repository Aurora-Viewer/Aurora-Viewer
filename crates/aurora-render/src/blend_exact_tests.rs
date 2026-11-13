//! The exact blend order on a GPU, through the production fragment
//! shaders and the production clear: layers drawn in every order must give
//! the pixel a sort of them by depth gives.
use crate::blend_exact::{CLEAR_SHADER, Clear, EARLY_TEST, Lists, SLOTS, layouts, scene_source};
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
}

const BACKGROUND: [f32; 3] = [0.2, 0.4, 0.6];
const SIZE: u32 = 8;

/// The alpha a layer is filed with.
fn filed(alpha: f32) -> f32 {
    (alpha * 255.0 + 0.5).floor() / 255.0
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
    // (the scene behind is dimmed by the blending, with the alphas as they are)
    let behind: f32 = layers.iter().map(|l| 1.0 - l.alpha).product();
    std::array::from_fn(|i| colour[i] + BACKGROUND[i] * behind)
}

fn function(object: &str, name: &str) -> String {
    let start = object.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("production {name}"));
    let end = start + object[start..].find("\n}").expect("function end") + 2;
    object[start..end]
        .replace("@builtin(front_facing) ", "")
        .replace("@builtin(sample_mask) ", "")
        .replace("@location(0) ", "")
        + "\n"
}

fn between(object: &str, from: &str, to: &str) -> String {
    let start = object.find(from).unwrap_or_else(|| panic!("production {from}"));
    let end = start + object[start..].find(to).unwrap_or_else(|| panic!("production {to}")) + to.len();
    object[start..end].to_string() + "\n"
}

fn shader_source() -> String {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let mut production = between(&object, "struct BlendOrderInfo {", "const BLEND_TAIL_OPAQUE: f32 = 16.0;");
    production += &between(&object, "struct BlendShare {", "\n};");
    for name in [
        "water_clip",
        "blend_pixel",
        "blend_rest",
        "blend_place",
        "blend_cover",
        "fs_blend_vis",
        "blend_share",
        "fs_blend_exact",
    ] {
        production += &function(&object, name);
    }
    format!(
        r#"
const FLAG_DOUBLE_SIDED:u32={double_sided}u;
const FLAG_PORTAL_EFFECT:u32={portal}u;
const FLAG_ALPHA_MASK:u32={mask}u;
const FLAG_EMISSIVE_MASK:u32={emissive}u;
const LAYER_RANK_SHIFT:u32=24u;
const LAYER_RANK_MAX:u32=7u;
const PORTAL_SURFACE_SCALE:f32=1.16;
struct DrawRecord {{flags:vec4<u32>,tex:vec4<u32>,base_color:vec4<f32>,params:vec4<f32>}};
@group(0) @binding(0) var<storage,read> records:array<DrawRecord>;
struct Frame {{sky_cloud_pd2:vec4<f32>,params:vec4<f32>,camera_pos:vec4<f32>}};
const frame=Frame(vec4<f32>(1.0),vec4<f32>(0.0),vec4<f32>(0.0,0.0,1.0,1.0));
override WATER_CLIP:u32=0u;
struct VsOut {{@builtin(position) clip:vec4<f32>,@location(0) @interpolate(flat) record:u32,
    @location(1) world_pos:vec3<f32>,@location(2) uv:vec2<f32>}};
struct Material {{color:vec4<f32>}};
struct Surface {{alpha:f32,color:vec3<f32>}};
fn portal_surface(r:DrawRecord,uv:vec2<f32>,aa:f32)->Surface {{return Surface(1.0,vec3<f32>(1.0));}}
fn apply_fog(c:vec3<f32>,p:vec3<f32>)->vec3<f32> {{return c;}}
fn portal_clip(r:DrawRecord,p:vec3<f32>) {{}}
fn sample_tex(t:u32,uv:vec2<f32>)->vec4<f32> {{return vec4<f32>(1.0);}}
fn eval_material(v:VsOut,front:bool)->Material {{return Material(records[v.record].base_color);}}
fn shade_material(v:VsOut,m:Material)->Material {{return m;}}
fn rez_alpha(r:DrawRecord,v:VsOut)->f32 {{return 1.0;}}
{production}
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->VsOut {{
    let positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return VsOut(vec4<f32>(positions[vertex],records[instance].params.x,1.0),instance,vec3<f32>(0.0,0.0,1.0),vec2<f32>(0.5));
}}
@fragment
@early_depth_test(force)
fn visibility(v:VsOut,@builtin(front_facing) front:bool,@builtin(sample_mask) mask:u32)->@location(0) vec4<f32> {{
    return fs_blend_vis(v,front,mask);
}}
@fragment fn colour(v:VsOut,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {{return fs_blend_exact(v,front);}}
"#,
        double_sided = flags::DOUBLE_SIDED,
        portal = flags::PORTAL_EFFECT,
        mask = flags::ALPHA_MASK,
        emissive = flags::EMISSIVE_MASK,
    )
}

struct Bench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    clear: Clear,
    lists: Lists,
    records_layout: wgpu::BindGroupLayout,
    visibility: wgpu::RenderPipeline,
    colour: wgpu::RenderPipeline,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

impl Bench {
    fn new() -> Option<Bench> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features() & EARLY_TEST,
            ..Default::default()
        }))
        .expect("blend order GPU device");
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
        // the blends of `Pipelines::blend_vis` and `blend_exact`
        let cover = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let order_layouts = layouts(&device);
        let pipeline = |entry: &'static str, fronts: &wgpu::BindGroupLayout, blend: wgpu::BlendState, write_mask| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(&records_layout), None, None, Some(fronts)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let visibility = pipeline(
            "visibility",
            &order_layouts[0],
            wgpu::BlendState {
                color: cover,
                alpha: cover,
            },
            wgpu::ColorWrites::ALL,
        );
        let colour = pipeline(
            "colour",
            &order_layouts[1],
            wgpu::BlendState {
                color: add,
                alpha: wgpu::BlendComponent::REPLACE,
            },
            wgpu::ColorWrites::COLOR,
        );
        let clear = Clear::new(&device);
        let lists = Lists::new(&device, &clear, (SIZE, SIZE));
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let (target, view) = crate::renderer::make_view(&device, "blend order pixels", SIZE, SIZE, format, 1, usage);
        let (_, depth) = crate::renderer::make_view(
            &device,
            "blend order depth",
            SIZE,
            SIZE,
            wgpu::TextureFormat::Depth32Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        Some(Bench {
            device,
            queue,
            clear,
            lists,
            records_layout,
            visibility,
            colour,
            target,
            view,
            depth,
        })
    }

    /// One view: `layers` drawn in the order given over the background,
    /// in front of an opaque scene at `wall`; every pixel of the result.
    fn draw(&self, layers: &[Layer], wall: f32) -> Vec<[f32; 3]> {
        let records: Vec<[u32; 16]> = layers
            .iter()
            .map(|l| {
                let mut rec = [0; 16];
                rec[0] = flags::ALPHA_BLEND | flags::DOUBLE_SIDED;
                rec[3] = l.rank << 24;
                for (word, c) in rec[8..11].iter_mut().zip(l.rgb) {
                    *word = c.to_bits();
                }
                rec[11] = l.alpha.to_bits();
                rec[12] = l.z.to_bits();
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
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.lists.clear(&mut encoder, &self.clear);
        for (first, pipeline, fronts) in [(true, &self.visibility, &self.lists.write), (false, &self.colour, &self.lists.read)] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if first {
                            wgpu::LoadOp::Clear(wgpu::Color {
                                r: f64::from(BACKGROUND[0]),
                                g: f64::from(BACKGROUND[1]),
                                b: f64::from(BACKGROUND[2]),
                                a: 1.0,
                            })
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: if first { wgpu::LoadOp::Clear(wall) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(3, fronts, &[]);
            pass.draw(0..3, 0..layers.len() as u32);
        }
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
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
        let bytes = readback.get_mapped_range(..).expect("blend order pixels");
        (0..SIZE as usize * SIZE as usize)
            .map(|i| {
                let at = (i / SIZE as usize) * 256 + (i % SIZE as usize) * 4;
                std::array::from_fn(|c| f32::from(bytes[at + c]) / 255.0)
            })
            .collect()
    }

    /// The colour every pixel of a view of `layers` has.
    fn pixel(&self, layers: &[Layer], wall: f32, case: &str) -> [f32; 3] {
        let pixels = self.draw(layers, wall);
        for p in &pixels {
            assert!(
                near(*p, pixels[0], 1.5 / 255.0),
                "{case}: every pixel alike, {p:?} and {:?}",
                pixels[0]
            );
        }
        pixels[0]
    }
}

fn near(a: [f32; 3], b: [f32; 3], by: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= by)
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
    Layer { rgb, alpha, z, rank: 0 }
}

/// Two 8-bit steps: the target's own rounding, twice (two passes).
const EXACT: f32 = 2.5 / 255.0;

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
        let got = bench.pixel(&order, 0.0, "four layers");
        let drawn: Vec<f32> = order.iter().map(|l| l.z).collect();
        assert!(near(got, want, EXACT), "drawn {drawn:?}: {got:?}, sorted {want:?}");
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
    assert!(near(naive(&reversed), want, EXACT) && !near(naive(&four), want, 0.1));
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
        let got = bench.pixel(&order, 0.0, "two on one depth");
        assert!(near(got, want, EXACT), "{got:?}, sorted {want:?}");
    }
    // as much red as blue, over what the background leaves of each
    let behind: f32 = tie.iter().map(|l| 1.0 - l.alpha).product();
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
            let got = bench.pixel(&order, 0.0, "two ranks on one depth");
            assert!(near(got, want, EXACT), "ranks {red}/{blue}: {got:?}, sorted {want:?}");
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
        let got = bench.pixel(&order, 0.5, "a wall");
        assert!(near(got, want, EXACT), "{got:?}, sorted {want:?}");
    }
}

#[test]
fn more_layers_than_slots_stay_free_of_their_order_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let Some(bench) = Bench::new() else {
        return;
    };
    // twice the slots of a front, faint enough for the tail to show
    let count = SLOTS as usize * 2;
    let deep: Vec<Layer> = (0..count)
        .map(|i| layer([RED, GREEN, BLUE][i % 3], 0.12, 0.1 + 0.8 * i as f32 / count as f32))
        .collect();
    let want = sorted(&deep);
    let mut shuffled = deep.clone();
    // a fixed shuffle: every third, then the rest backwards
    shuffled.sort_by_key(|l| ((l.z * 1000.0) as u32 * 7919) % 31);
    let mut reversed = deep.clone();
    reversed.reverse();
    let first = bench.pixel(&deep, 0.0, "deep, back to front");
    for (name, order) in [("front to back", &reversed), ("shuffled", &shuffled)] {
        let got = bench.pixel(order, 0.0, name);
        assert!(near(got, first, EXACT), "{name}: {got:?}, back to front {first:?}");
    }
    // the front is exact, the tail a mean under it: close to the sort
    assert!(near(first, want, 0.06), "{first:?}, sorted {want:?}");
    // an opaque layer in front: nothing of the sixteen shows, whatever
    // fell in the tail
    let mut capped = deep.clone();
    capped.insert(3, layer(WHITE, 1.0, 0.95));
    let got = bench.pixel(&capped, 0.0, "deep under an opaque layer");
    assert!(near(got, WHITE, EXACT), "{got:?}");
}

#[test]
fn the_clear_shader_is_the_one_the_pipeline_uses() {
    assert!(CLEAR_SHADER.contains("fn cs_blend_clear("));
}
