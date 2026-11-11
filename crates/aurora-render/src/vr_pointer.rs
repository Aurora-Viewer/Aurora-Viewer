//! Tracked controllers and the pointer beam, drawn into both eyes after the
//! world.
//!
//! Geometry is built on the CPU each frame from the controller poses of the
//! frame being rendered (`xr_frame::Session::begin`), so the models and the
//! beam stay locked to the hands. It is drawn without depth, over the world:
//! the beam ends at the surface the viewer traced, and a controller is never
//! more than an arm's length away.
use crate::pipeline_cache::CachedPipelines;
use crate::{VrControllerState, VrInputState, VrPointer, VrSide};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3, Vec4};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub(crate) struct Vertex {
    pub position: [f32; 3],
    /// Premultiplied alpha.
    pub color: [f32; 4],
}

/// Uniform slot stride: one view-projection per eye at a dynamic offset.
const SLOT: u64 = 256;
/// Beam radius at the hand, metres.
const BEAM_RADIUS: f32 = 0.0025;

pub(crate) struct Overlay {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
    vertices: Option<wgpu::Buffer>,
    scratch: Vec<Vertex>,
}

impl Overlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VR controllers"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;
struct In { @location(0) position: vec3<f32>, @location(1) color: vec4<f32> }
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> }
@vertex fn vs(v: In) -> Out {
    var o: Out; o.position = view_proj * vec4(v.position, 1.0); o.color = v.color; return o;
}
@fragment fn fs(o: Out) -> @location(0) vec4<f32> { return o.color; }
"#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VR controllers eye"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(64),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("VR controllers"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("VR controllers"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VR controllers eyes"),
            size: SLOT * 2,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VR controllers eye"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(64),
                }),
            }],
        });
        Self {
            pipeline,
            uniforms,
            group,
            vertices: None,
            scratch: Vec::new(),
        }
    }

    /// Draw over both eyes' finished images. Submitted on the queue after
    /// the eyes' own work and before the mirror and the runtime release.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        eyes: [(&wgpu::Texture, Mat4); 2],
        input: &VrInputState,
        pointer: VrPointer,
    ) {
        self.scratch.clear();
        build(input, pointer, &mut self.scratch);
        if self.scratch.is_empty() {
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(&self.scratch);
        if self.vertices.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
            self.vertices = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("VR controllers vertices"),
                size: (bytes.len() as u64).next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(vertices) = &self.vertices else { return };
        queue.write_buffer(vertices, 0, bytes);
        for (i, (_, view_proj)) in eyes.iter().enumerate() {
            queue.write_buffer(&self.uniforms, SLOT * i as u64, bytemuck::bytes_of(&view_proj.to_cols_array()));
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("VR controllers"),
        });
        for (i, (texture, _)) in eyes.iter().enumerate() {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("VR controllers"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[(SLOT * i as u64) as u32]);
            pass.set_vertex_buffer(0, vertices.slice(..bytes.len() as u64));
            pass.draw(0..self.scratch.len() as u32, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

/// The frame's triangles (a list, three vertices each) in world space.
pub(crate) fn build(input: &VrInputState, pointer: VrPointer, out: &mut Vec<Vertex>) {
    for side in [VrSide::Left, VrSide::Right] {
        let hand = input.hand(side);
        if !hand.active || !hand.position.is_finite() || !hand.orientation.is_finite() {
            continue;
        }
        let pointing = pointer.hand == Some(side);
        if pointer.controllers {
            controller(hand, side, pointing, out);
            if side == VrSide::Left
                && let Some(on) = pointer.mic
            {
                mic_light(hand, on, out);
            }
        }
        if pointing && pointer.length > 0.0 {
            beam(hand, pointer, out);
        }
    }
    if let Some(b) = pointer.outline {
        outline(b, out);
    }
    if let Some(a) = pointer.arc {
        arc(a, out);
    }
}

/// The teleport arc as a thin tube, green where it lands, red where it
/// does not, with a marker on the landing point.
fn arc(a: crate::VrArc, out: &mut Vec<Vertex>) {
    let points = &a.points[..a.len.min(crate::VrArc::MAX)];
    if points.len() < 2 || points.iter().any(|p| !p.is_finite()) {
        return;
    }
    let color = if a.lands {
        Vec3::new(0.3, 1.0, 0.45)
    } else {
        Vec3::new(1.0, 0.3, 0.25)
    };
    let premul = |alpha: f32| (color * alpha).extend(alpha);
    let radius = 0.008;
    for pair in points.windows(2) {
        let dir = (pair[1] - pair[0]).normalize_or_zero();
        let side = dir.cross(Vec3::Z).normalize_or(Vec3::X) * radius;
        let up = side.cross(dir).normalize_or_zero() * radius;
        for n in [side, up] {
            push_quad(out, [pair[0] - n, pair[1] - n, pair[1] + n, pair[0] + n], [premul(0.85); 4]);
        }
    }
    if a.lands {
        // a flat ring of quads on the landing spot
        let end = points[points.len() - 1] + Vec3::Z * 0.02;
        const SEGMENTS: usize = 16;
        let (r0, r1) = (0.22, 0.3);
        for i in 0..SEGMENTS {
            let at = |k: usize, r: f32| {
                let t = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                end + Vec3::new(t.cos(), t.sin(), 0.0) * r
            };
            push_quad(out, [at(i, r0), at(i + 1, r0), at(i + 1, r1), at(i, r1)], [premul(0.9); 4]);
        }
    }
}

/// The twelve edges of the pointed object's box, as thin glowing bars.
fn outline(b: crate::VrBox, out: &mut Vec<Vertex>) {
    let half = b.half.abs();
    if !b.center.is_finite() || !half.is_finite() || !b.rotation.is_finite() || half.max_element() > 32.0 {
        return;
    }
    let q = b.rotation.normalize();
    let axes = [q * Vec3::X * half.x, q * Vec3::Y * half.y, q * Vec3::Z * half.z];
    let thickness = (half.max_element() * 0.01).clamp(0.003, 0.02);
    let color = Vec3::new(0.35, 0.85, 1.0);
    let alpha = 0.75;
    let premul = (color * alpha).extend(alpha);
    for a in 0..3 {
        let (b1, b2) = (axes[(a + 1) % 3], axes[(a + 2) % 3]);
        let (n1, n2) = (b1.normalize_or_zero() * thickness, b2.normalize_or_zero() * thickness);
        for (s1, s2) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let mid = b.center + b1 * s1 + b2 * s2;
            let (from, to) = (mid - axes[a], mid + axes[a]);
            // a cross of two quads: visible from any side, to both eyes
            for n in [n1, n2] {
                push_quad(out, [from - n, to - n, to + n, from + n], [premul; 4]);
            }
        }
    }
}

/// Simple shading from a fixed overhead light, so the boxes read as solid.
fn shade(normal: Vec3, color: Vec3) -> Vec3 {
    let light = Vec3::new(0.3, 0.2, 0.93).normalize();
    color * (0.55 + 0.45 * normal.dot(light).max(0.0))
}

fn push_quad(out: &mut Vec<Vertex>, corners: [Vec3; 4], colors: [Vec4; 4]) {
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(Vertex {
            position: corners[i].to_array(),
            color: colors[i].to_array(),
        });
    }
}

/// The controller model (`tools/vr_controller/build_controller.py`): the
/// right hand's, in aim-pose axes, a triangle list of (position, normal,
/// part), and where its thumbstick and trigger turn.
struct Model {
    vertices: Vec<(Vec3, Vec3, u32)>,
    stick_pivot: Vec3,
    trigger_pivot: Vec3,
}

/// The model's parts, as the generator numbers them.
mod part {
    #[cfg_attr(not(test), allow(dead_code))] // (the shell is the default colour)
    pub const SHELL: u32 = 0;
    pub const FACE: u32 = 1;
    pub const TRIGGER: u32 = 2;
    pub const GRIP: u32 = 3;
    pub const STICK: u32 = 4;
    pub const PRIMARY: u32 = 5;
    pub const SECONDARY: u32 = 6;
    pub const MENU: u32 = 7;
}

fn model() -> &'static Model {
    static MODEL: std::sync::OnceLock<Model> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| {
        parse_model(include_bytes!("vr_controller.bin")).unwrap_or(Model {
            vertices: Vec::new(),
            stick_pivot: Vec3::ZERO,
            trigger_pivot: Vec3::ZERO,
        })
    })
}

fn parse_model(bytes: &[u8]) -> Option<Model> {
    let word = |at: usize| bytes.get(at..at + 4).map(|b| [b[0], b[1], b[2], b[3]]);
    if bytes.get(..4)? != b"AVRC" || u32::from_le_bytes(word(4)?) != 2 {
        return None;
    }
    let count = u32::from_le_bytes(word(8)?) as usize;
    const HEADER: usize = 12 + 24;
    const STRIDE: usize = 28;
    if bytes.len() != HEADER + count * STRIDE || count % 3 != 0 {
        return None;
    }
    let f = |at: usize| word(at).map(f32::from_le_bytes);
    let v3 = |at: usize| Some(Vec3::new(f(at)?, f(at + 4)?, f(at + 8)?));
    let vertices = (0..count)
        .map(|i| {
            let at = HEADER + i * STRIDE;
            Some((v3(at)?, v3(at + 12)?, u32::from_le_bytes(word(at + 24)?)))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Model {
        vertices,
        stick_pivot: v3(12)?,
        trigger_pivot: v3(24)?,
    })
}

/// One controller: the model at the hand's aim pose, mirrored for the left
/// hand, its thumbstick tilted with the stick and its trigger pulled in: a
/// light grey shell, a dark face plate, stick and buttons, the pressed
/// parts lit in the hand's colour (as is the pointing hand's plate, faintly).
fn controller(hand: VrControllerState, side: VrSide, pointing: bool, out: &mut Vec<Vertex>) {
    let m = model();
    let q = hand.orientation.normalize();
    let accent = match side {
        VrSide::Left => Vec3::new(0.25, 0.42, 0.85),
        VrSide::Right => Vec3::new(0.85, 0.45, 0.2),
    };
    let left = side == VrSide::Left;
    let mirror = Vec3::new(if left { -1.0 } else { 1.0 }, 1.0, 1.0);
    let stick_turn = Quat::from_rotation_x(-hand.thumbstick.y.clamp(-1.0, 1.0) * 0.35)
        * Quat::from_rotation_z(-hand.thumbstick.x.clamp(-1.0, 1.0) * 0.35);
    let trigger_turn = Quat::from_rotation_x(hand.trigger.clamp(0.0, 1.0) * 0.3);
    let lit = |pressed: f32, rest: Vec3| rest.lerp(accent * 1.2 + Vec3::splat(0.2), pressed.clamp(0.0, 1.0));
    let dark = Vec3::splat(0.07);
    let light = Vec3::splat(0.62);
    let colors = |p: u32| match p {
        part::FACE => dark.lerp(accent, if pointing { 0.12 } else { 0.0 }),
        part::TRIGGER => lit(hand.trigger, light * 0.92),
        part::GRIP => lit(hand.grip, light * 0.92),
        part::STICK => lit(hand.thumbstick_click as u8 as f32, dark),
        part::PRIMARY => lit(hand.primary as u8 as f32, dark * 1.4),
        part::SECONDARY => lit(hand.secondary as u8 as f32, dark * 1.4),
        part::MENU => lit(hand.menu as u8 as f32, dark * 1.4),
        // the shell (and a part a newer model has that this one does not know)
        _ => light,
    };
    let mut tri = [Vertex {
        position: [0.0; 3],
        color: [0.0; 4],
    }; 3];
    for (i, &(mut position, mut normal, p)) in m.vertices.iter().enumerate() {
        // the menu button is the left controller's
        if p == part::MENU && !left {
            continue;
        }
        if left {
            position.x = -position.x;
            normal.x = -normal.x;
        }
        let turn = match p {
            part::STICK => Some((m.stick_pivot * mirror, stick_turn)),
            part::TRIGGER => Some((m.trigger_pivot * mirror, trigger_turn)),
            _ => None,
        };
        if let Some((pivot, r)) = turn {
            position = pivot + r * (position - pivot);
            normal = r * normal;
        }
        let n = (q * normal).normalize_or_zero();
        tri[i % 3] = Vertex {
            position: (hand.position + q * position).to_array(),
            color: shade(n, colors(p)).extend(1.0).to_array(),
        };
        if i % 3 == 2 {
            out.extend_from_slice(&tri);
        }
    }
}

/// The microphone's light on top of the left controller: bright green while
/// sending, a dim grey dot otherwise.
fn mic_light(hand: VrControllerState, on: bool, out: &mut Vec<Vertex>) {
    let q = hand.orientation.normalize();
    let (right, up, forward) = (q * Vec3::X, q * Vec3::Y, q * Vec3::NEG_Z);
    let color = if on { Vec3::new(0.25, 1.0, 0.4) } else { Vec3::splat(0.3) };
    let size = if on { 0.009 } else { 0.006 };
    // (unshaded: a light, not a surface)
    let lit = color.extend(1.0);
    let center = hand.position - forward * 0.05 + up * 0.002;
    let axes = [right * size, up * size, forward * size];
    for (a, b, c) in [(0, 1, 2), (1, 2, 0), (2, 0, 1)] {
        for sign in [1.0f32, -1.0] {
            let face = center + axes[a] * sign;
            let (u, v) = (axes[b], axes[c] * sign);
            push_quad(out, [face - u - v, face + u - v, face + u + v, face - u + v], [lit; 4]);
        }
    }
}

fn beam(hand: VrControllerState, pointer: VrPointer, out: &mut Vec<Vertex>) {
    let q = hand.orientation.normalize();
    let (right, up, forward) = (q * Vec3::X, q * Vec3::Y, q * Vec3::NEG_Z);
    let length = pointer.length.min(1000.0);
    let start = hand.position;
    let end = start + forward * length;
    let color = if pointer.actionable {
        Vec3::new(0.35, 0.85, 1.0)
    } else {
        Vec3::splat(0.92)
    };
    let pressed = hand.trigger >= 0.7;
    let radius = BEAM_RADIUS * if pressed { 1.6 } else { 1.0 };
    let (a0, a1) = (if pressed { 0.95 } else { 0.8 }, if pointer.hit { 0.55 } else { 0.0 });
    let premul = |alpha: f32| (color * alpha).extend(alpha);
    // a hexagonal tube: seen from either eye without facing the camera
    const SIDES: usize = 6;
    let ring = |i: usize| {
        let a = i as f32 / SIDES as f32 * std::f32::consts::TAU;
        (right * a.cos() + up * a.sin()) * radius
    };
    for i in 0..SIDES {
        let (p, n) = (ring(i), ring(i + 1));
        push_quad(
            out,
            [start + p, start + n, end + n * 0.5, end + p * 0.5],
            [premul(a0), premul(a0), premul(a1), premul(a1)],
        );
    }
    if pointer.hit {
        // an octahedron at the hit, growing with distance so it stays visible
        let r = (0.006 * length).max(0.012);
        let tip = premul(1.0);
        let dirs = [right, up, -right, -up];
        for i in 0..4 {
            let (a, b) = (end + dirs[i] * r, end + dirs[(i + 1) % 4] * r);
            for pole in [end + forward * r, end - forward * r] {
                for v in [a, b, pole] {
                    out.push(Vertex {
                        position: v.to_array(),
                        color: tip.to_array(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn hand(position: Vec3) -> VrControllerState {
        VrControllerState {
            active: true,
            position,
            orientation: Quat::IDENTITY,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_without_active_hands_or_options() {
        let mut out = Vec::new();
        build(&VrInputState::default(), VrPointer::default(), &mut out);
        assert!(out.is_empty());
        let input = VrInputState {
            right: hand(Vec3::ZERO),
            ..Default::default()
        };
        build(&input, VrPointer::default(), &mut out);
        assert!(out.is_empty(), "controllers off and no beam hand");
    }

    #[test]
    fn beam_runs_along_aim_to_its_length() {
        let input = VrInputState {
            right: hand(Vec3::new(1.0, 2.0, 3.0)),
            ..Default::default()
        };
        let pointer = VrPointer {
            controllers: false,
            hand: Some(VrSide::Right),
            length: 4.0,
            hit: true,
            actionable: true,
            outline: None,
            arc: None,
            mic: None,
        };
        let mut out = Vec::new();
        build(&input, pointer, &mut out);
        assert_eq!(out.len() % 3, 0);
        // OpenXR aim is -Z of the pose: the far end is 4 m that way
        let far = out.iter().map(|v| v.position[2]).fold(f32::INFINITY, f32::min);
        assert!((far - (3.0 - 4.0 - 0.024)).abs() < 1e-3, "{far}");
        assert!(out.iter().all(|v| v.color[3] <= 1.0 && v.color[0] <= v.color[3] + 1e-6));
    }

    #[test]
    fn both_controllers_drawn_beam_only_from_pointing_hand() {
        let input = VrInputState {
            left: hand(Vec3::X),
            right: hand(-Vec3::X),
            ..Default::default()
        };
        let mut only = Vec::new();
        build(
            &input,
            VrPointer {
                controllers: true,
                ..Default::default()
            },
            &mut only,
        );
        // the model on both hands, the menu button on the left only
        let menu = model().vertices.iter().filter(|v| v.2 == part::MENU).count();
        assert_eq!(only.len(), 2 * model().vertices.len() - menu);
        let mut with_beam = Vec::new();
        build(
            &input,
            VrPointer {
                controllers: true,
                hand: Some(VrSide::Left),
                length: 2.0,
                ..Default::default()
            },
            &mut with_beam,
        );
        assert_eq!(with_beam.len(), only.len() + 6 * 6);
    }

    #[test]
    fn model_parses_with_every_part_and_sensible_size() {
        let m = model();
        assert!(m.vertices.len() > 300 && m.vertices.len() % 3 == 0);
        for p in [part::SHELL, part::FACE, part::TRIGGER, part::GRIP, part::STICK, part::PRIMARY, part::SECONDARY, part::MENU] {
            assert!(m.vertices.iter().any(|v| v.2 == p), "part {p}");
        }
        // a hand-sized thing behind the aim point: within 20 cm, mostly behind it
        assert!(m.vertices.iter().all(|v| v.0.length() < 0.2));
        assert!(m.vertices.iter().all(|v| v.0.z > -0.03));
        assert!(parse_model(b"AVRC").is_none());
    }

    #[test]
    fn outline_has_twelve_edges_and_skips_huge_boxes() {
        let mut out = Vec::new();
        let b = crate::VrBox {
            center: Vec3::ONE,
            rotation: Quat::IDENTITY,
            half: Vec3::splat(0.5),
        };
        build(
            &VrInputState::default(),
            VrPointer {
                outline: Some(b),
                ..Default::default()
            },
            &mut out,
        );
        assert_eq!(out.len(), 12 * 2 * 6);
        out.clear();
        build(
            &VrInputState::default(),
            VrPointer {
                outline: Some(crate::VrBox { half: Vec3::splat(100.0), ..b }),
                ..Default::default()
            },
            &mut out,
        );
        assert!(out.is_empty());
    }
}
