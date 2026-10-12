//! Frosted window surfaces. One cached GPU blur of the scene is shared by
//! every visible window; only each rounded window rectangle samples it.

use std::sync::Arc;

struct Region {
    radius: f32,
}

/// Insert below a window's contents. The viewer resolves the final rectangle
/// after layout, so moving/resizing never samples last frame's position.
pub fn glass_panel(rect: egui::Rect, radius: f32, tint: egui::Color32) -> egui::Shape {
    egui::Shape::Vec(vec![
        egui::Shape::Callback(egui::PaintCallback {
            rect,
            callback: Arc::new(Region { radius }),
        }),
        egui::Shape::rect_filled(rect, radius, tint),
    ])
}

pub(crate) fn requested(primitives: &[egui::ClippedPrimitive]) -> bool {
    primitives.iter().any(
        |p| matches!(&p.primitive, egui::epaint::Primitive::Callback(c) if c.callback.is::<Region>() && p.clip_rect.intersects(c.rect)),
    )
}

/// Preserve primitive indices used by the modal blur boundary on surfaces
/// that cannot be copied. The normal tinted frame remains visible.
pub(crate) fn skip(primitives: &mut [egui::ClippedPrimitive]) {
    for primitive in primitives {
        if matches!(&primitive.primitive, egui::epaint::Primitive::Callback(c) if c.callback.is::<Region>()) {
            primitive.primitive = egui::epaint::Primitive::Mesh(egui::Mesh::default());
        }
    }
}

pub(crate) struct Glass {
    blur: super::ui_blur::Blur,
    output: wgpu::TextureView,
    pub texture: egui::TextureId,
    size: [u32; 2],
}

impl Glass {
    pub fn new(device: &wgpu::Device, renderer: &mut egui_wgpu::Renderer, format: wgpu::TextureFormat, size: [u32; 2], scale: f32) -> Self {
        let blur = super::ui_blur::Blur::new(device, format, size, scale);
        let output = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Window glass"),
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
        let texture = renderer.register_native_texture(device, &output, wgpu::FilterMode::Linear);
        Self {
            blur,
            output,
            texture,
            size,
        }
    }

    pub fn matches(&self, size: [u32; 2], scale: f32) -> bool {
        self.blur.matches(size, scale)
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

    pub fn resolve(&self, primitives: &mut [egui::ClippedPrimitive], scale: f32) {
        let screen = egui::vec2(self.size[0] as f32, self.size[1] as f32) / scale;
        for primitive in primitives {
            if let egui::epaint::Primitive::Callback(callback) = &primitive.primitive
                && let Some(region) = callback.callback.downcast_ref::<Region>()
            {
                primitive.primitive = egui::epaint::Primitive::Mesh(rounded_mesh(callback.rect, region.radius, self.texture, screen));
            }
        }
    }
}

fn rounded_mesh(rect: egui::Rect, radius: f32, texture: egui::TextureId, screen: egui::Vec2) -> egui::Mesh {
    let mut mesh = egui::Mesh::with_texture(texture);
    let radius = radius.max(0.0).min(rect.width().min(rect.height()) * 0.5);
    let vertex = |pos: egui::Pos2| egui::epaint::Vertex {
        pos,
        uv: egui::pos2(pos.x / screen.x, pos.y / screen.y),
        color: egui::Color32::WHITE,
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
            let mesh = rounded_mesh(rect, 8.0, egui::TextureId::User(7), screen);
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
}
