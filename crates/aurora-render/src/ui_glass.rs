//! Frosted window surfaces. One cached GPU blur of the scene is shared by
//! every visible window; only each rounded window rectangle samples it.
//!
//! The world's labels (`ui_world`) are drawn under the HUD attachments, and
//! the windows over them: the labels sample a blur of the scene alone, made
//! where they are, and the windows a blur of the scene with its labels and
//! the HUD attachments.

use std::sync::Arc;

struct Region {
    radius: f32,
    opacity: f32,
    /// Brightness kept of the blurred scene (1: as it is).
    shade: f32,
}

/// Insert below a window's contents. The viewer resolves the final rectangle
/// after layout, so moving/resizing never samples last frame's position.
pub fn glass_panel(rect: egui::Rect, radius: f32, tint: egui::Color32) -> egui::Shape {
    glass_panel_faded(rect, radius, tint, 1.0)
}

/// `glass_panel` whose blur shows through at `opacity` (0 to 1), for a
/// surface that fades in or out; `tint` carries its own alpha.
pub fn glass_panel_faded(rect: egui::Rect, radius: f32, tint: egui::Color32, opacity: f32) -> egui::Shape {
    egui::Shape::Vec(vec![
        glass_region(rect, radius, opacity, 1.0),
        egui::Shape::rect_filled(rect, radius, tint),
    ])
}

/// The blur alone, at `opacity`, for a surface that draws its own tint over
/// it (the avatar name plates, whose pill is a mesh of its own). `shade`
/// (0 to 1) is the brightness kept of the blurred scene: under a light tint,
/// it keeps a small surface readable over a white wall or a bright sky
/// without making it more opaque over the rest.
pub fn glass_region(rect: egui::Rect, radius: f32, opacity: f32, shade: f32) -> egui::Shape {
    egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(Region {
            radius,
            opacity: opacity.clamp(0.0, 1.0),
            shade: shade.clamp(0.0, 1.0),
        }),
    })
}

pub(crate) fn requested(primitives: &[egui::ClippedPrimitive]) -> bool {
    primitives.iter().any(
        |p| matches!(&p.primitive, egui::epaint::Primitive::Callback(c) if c.callback.is::<Region>() && p.clip_rect.intersects(c.rect)),
    )
}

/// The pixels the visible regions of `primitives` cover together, as
/// [x, y, width, height] on a surface of `size`; None without one.
pub(crate) fn bounds(primitives: &[egui::ClippedPrimitive], scale: f32, size: [u32; 2]) -> Option<[u32; 4]> {
    let mut all = egui::Rect::NOTHING;
    for p in primitives {
        if let egui::epaint::Primitive::Callback(c) = &p.primitive
            && c.callback.is::<Region>()
        {
            let visible = p.clip_rect.intersect(c.rect);
            if visible.is_positive() {
                all = all.union(visible);
            }
        }
    }
    let pixels = |from: f32, to: f32, limit: u32| {
        let from = ((from * scale).floor().max(0.0) as u32).min(limit);
        let to = ((to * scale).ceil().max(0.0) as u32).min(limit);
        (from < to).then(|| (from, to - from))
    };
    let (x, width) = pixels(all.left(), all.right(), size[0])?;
    let (y, height) = pixels(all.top(), all.bottom(), size[1])?;
    Some([x, y, width, height])
}

/// Preserve primitive indices used by the modal blur boundary on surfaces
/// that cannot be copied. The normal tinted frame remains visible.
pub(crate) fn skip(primitives: &mut [egui::ClippedPrimitive]) {
    for primitive in primitives {
        if matches!(&primitive.primitive, egui::epaint::Primitive::Callback(c) if c.callback.is::<Region>()) {
            primitive.primitive = egui::epaint::Primitive::Mesh(egui::Mesh::default());
            // An empty mesh must not reach the draw: egui-wgpu binds the
            // index range of every mesh inside its clip, and wgpu panics on
            // an empty buffer slice. An empty clip only advances its buffers.
            primitive.clip_rect = egui::Rect::NOTHING;
        }
    }
}

pub(crate) struct Glass {
    blur: super::ui_blur::Blur,
    output: wgpu::TextureView,
    pub texture: egui::TextureId,
    /// The scene alone, blurred where the world's labels show it
    /// (`encode_under`): made with the first of them that asks for it.
    under: Option<(wgpu::TextureView, egui::TextureId)>,
    format: wgpu::TextureFormat,
    size: [u32; 2],
}

/// A blurred image the interface samples.
fn target(
    device: &wgpu::Device,
    renderer: &mut egui_wgpu::Renderer,
    label: &str,
    format: wgpu::TextureFormat,
    size: [u32; 2],
) -> (wgpu::TextureView, egui::TextureId) {
    let view = device
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
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let texture = renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear);
    (view, texture)
}

impl Glass {
    pub fn new(device: &wgpu::Device, renderer: &mut egui_wgpu::Renderer, format: wgpu::TextureFormat, size: [u32; 2], scale: f32) -> Self {
        let blur = super::ui_blur::Blur::new(device, format, size, scale);
        let (output, texture) = target(device, renderer, "Window glass", format, size);
        Self {
            blur,
            output,
            texture,
            under: None,
            format,
            size,
        }
    }

    pub fn matches(&self, size: [u32; 2], scale: f32) -> bool {
        self.blur.matches(size, scale)
    }

    pub fn free(self, renderer: &mut egui_wgpu::Renderer) {
        renderer.free_texture(&self.texture);
        if let Some((_, texture)) = &self.under {
            renderer.free_texture(texture);
        }
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, scene: &wgpu::Texture) {
        encoder.copy_texture_to_texture(
            scene.as_image_copy(),
            self.blur.source.as_image_copy(),
            wgpu::Extent3d {
                width: self.size[0],
                height: self.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.blur.encode(encoder, &self.output);
    }

    /// Blur `scene` as it is before the world's labels are drawn on it, in
    /// `area` alone (their `bounds`): the labels cover little of the screen.
    /// Returns the texture their regions sample (`resolve`).
    pub fn encode_under(
        &mut self,
        device: &wgpu::Device,
        renderer: &mut egui_wgpu::Renderer,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::Texture,
        area: [u32; 4],
    ) -> egui::TextureId {
        let (format, size) = (self.format, self.size);
        let (view, texture) = self
            .under
            .get_or_insert_with(|| target(device, renderer, "Label glass", format, size));
        let part = self.blur.part(area);
        let [x, y, width, height] = part.source;
        let origin = wgpu::Origin3d { x, y, z: 0 };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                origin,
                ..scene.as_image_copy()
            },
            wgpu::TexelCopyTextureInfo {
                origin,
                ..self.blur.source.as_image_copy()
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.blur.encode_part(encoder, view, &part);
        *texture
    }

    /// Turn the regions of `primitives` into meshes that sample `texture`.
    pub fn resolve(&self, primitives: &mut [egui::ClippedPrimitive], scale: f32, texture: egui::TextureId) {
        let screen = egui::vec2(self.size[0] as f32, self.size[1] as f32) / scale;
        for primitive in primitives {
            if let egui::epaint::Primitive::Callback(callback) = &primitive.primitive
                && let Some(region) = callback.callback.downcast_ref::<Region>()
            {
                let level = (region.shade * 255.0).round() as u8;
                let color = egui::Color32::from_gray(level).gamma_multiply(region.opacity);
                primitive.primitive = egui::epaint::Primitive::Mesh(rounded_mesh(callback.rect, region.radius, texture, screen, color));
            }
        }
    }
}

fn rounded_mesh(rect: egui::Rect, radius: f32, texture: egui::TextureId, screen: egui::Vec2, color: egui::Color32) -> egui::Mesh {
    let mut mesh = egui::Mesh::with_texture(texture);
    let radius = radius.max(0.0).min(rect.width().min(rect.height()) * 0.5);
    let vertex = |pos: egui::Pos2| egui::epaint::Vertex {
        pos,
        uv: egui::pos2(pos.x / screen.x, pos.y / screen.y),
        color,
    };
    mesh.vertices.push(vertex(rect.center()));
    for (center, start) in [
        (rect.right_top() + egui::vec2(-radius, radius), -std::f32::consts::FRAC_PI_2),
        (rect.right_bottom() - egui::vec2(radius, radius), 0.0),
        (rect.left_bottom() + egui::vec2(radius, -radius), std::f32::consts::FRAC_PI_2),
        (rect.left_top() + egui::vec2(radius, radius), std::f32::consts::PI),
    ] {
        for step in 0..=8 {
            let angle = start + step as f32 / 8.0 * std::f32::consts::FRAC_PI_2;
            mesh.vertices.push(vertex(center + radius * egui::vec2(angle.cos(), angle.sin())));
        }
    }
    let last = mesh.vertices.len() as u32 - 1;
    for n in 1..=last {
        mesh.indices.extend_from_slice(&[0, n, if n == last { 1 } else { n + 1 }]);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_windows_sample_their_own_screen_coordinates_at_any_dpi() {
        for scale in [1.0, 1.5, 2.0] {
            let screen = egui::vec2(1920.0, 1080.0) / scale;
            let rect = egui::Rect::from_min_size(egui::pos2(40.0, 70.0), egui::vec2(344.0, 548.0));
            let mesh = rounded_mesh(rect, 8.0, egui::TextureId::User(7), screen, egui::Color32::WHITE);
            assert!(mesh.is_valid());
            assert!(mesh.vertices.iter().all(|v| rect.expand(0.001).contains(v.pos)));
            for vertex in &mesh.vertices {
                assert!((vertex.uv.x * screen.x - vertex.pos.x).abs() < 0.001);
                assert!((vertex.uv.y * screen.y - vertex.pos.y).abs() < 0.001);
            }
            assert!(!mesh.vertices.iter().any(|v| v.pos == rect.min));
        }
    }

    #[test]
    fn glass_marker_survives_tessellation_and_is_detected_only_when_visible() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.painter().add(glass_panel(ui.max_rect().shrink(10.0), 8.0, egui::Color32::BLACK));
        });
        output.textures_delta.clear();
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(requested(&primitives));
        for primitive in &mut primitives {
            primitive.clip_rect = egui::Rect::NOTHING;
        }
        assert!(!requested(&primitives));
        let count = primitives.len();
        skip(&mut primitives);
        assert_eq!(primitives.len(), count, "modal blur boundary indices stay valid");
        assert!(primitives.iter().all(|p| matches!(p.primitive, egui::epaint::Primitive::Mesh(_))));
    }

    #[test]
    fn the_bounds_of_the_visible_regions_are_whole_pixels_inside_the_surface() {
        let ctx = egui::Context::default();
        let region = |x: f32, y: f32| glass_region(egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(100.0, 20.0)), 10.0, 1.0, 1.0);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.painter().add(region(40.5, 30.0));
            ui.painter().add(region(300.0, 80.25));
            // cut by the left edge of the surface
            ui.painter().add(region(-60.0, 50.0));
        });
        output.textures_delta.clear();
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert_eq!(bounds(&primitives, 1.0, [1920, 1080]), Some([0, 30, 400, 71]));
        assert_eq!(bounds(&primitives, 1.5, [1920, 1080]), Some([0, 45, 600, 106]));
        assert_eq!(bounds(&primitives, 1.0, [350, 90]), Some([0, 30, 350, 60]));
        assert_eq!(bounds(&primitives[..0], 1.0, [1920, 1080]), None);
        for primitive in &mut primitives {
            primitive.clip_rect = egui::Rect::NOTHING;
        }
        assert_eq!(bounds(&primitives, 1.0, [1920, 1080]), None, "nothing visible");
    }

    /// On a surface that cannot be copied, a visible glass panel became an
    /// empty mesh inside its clip: egui-wgpu bound its empty index range and
    /// wgpu panicked ("buffer slice can not be empty").
    #[test]
    fn a_skipped_glass_panel_is_never_drawn() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.painter().add(glass_panel(ui.max_rect().shrink(10.0), 8.0, egui::Color32::BLACK));
        });
        output.textures_delta.clear();
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(requested(&primitives), "the panel is visible");
        skip(&mut primitives);
        let mut tints = 0;
        for primitive in &primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
                panic!("no callback is left");
            };
            if mesh.indices.is_empty() {
                assert!(!primitive.clip_rect.is_positive(), "an empty mesh keeps an empty clip");
            } else {
                assert!(primitive.clip_rect.is_positive());
                tints += 1;
            }
        }
        assert!(tints > 0, "the tinted frame is still drawn");
    }
}
