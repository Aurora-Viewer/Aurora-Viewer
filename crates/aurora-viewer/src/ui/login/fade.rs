//! Fade neighbour cards into the sky using ordinary GPU-interpolated mesh alpha.
use egui::epaint::{Mesh, Primitive};
use egui::layers::ShapeIdx;
use egui::{Rect, Shape, Ui};

pub(super) fn opacity(card: Rect, center: Rect) -> f32 {
    let overlap = (card.right().min(center.right()) - card.left().max(center.left())).max(0.0);
    0.45 + 0.55 * (overlap / card.width().max(1.0)).clamp(0.0, 1.0)
}

fn edge_alpha(x: f32, bounds: Rect, center: Rect) -> f32 {
    let left = ((x - bounds.left()) / (center.left() - bounds.left()).max(1.0)).clamp(0.0, 1.0);
    let right = ((bounds.right() - x) / (bounds.right() - center.right()).max(1.0)).clamp(0.0, 1.0);
    left.min(right)
}

fn faded_mesh(mesh: &Mesh, bounds: Rect, viewport: Rect, center: Rect) -> Mesh {
    let mut result = Mesh::with_texture(mesh.texture_id);
    // Split at fade boundaries so images never fade across a central card.
    let edges = [
        bounds.left(),
        center.left().clamp(bounds.left(), bounds.right()),
        center.right().clamp(bounds.left(), bounds.right()),
        bounds.right(),
    ];
    for indices in mesh.indices.as_chunks::<3>().0 {
        let triangle = [
            mesh.vertices[indices[0] as usize],
            mesh.vertices[indices[1] as usize],
            mesh.vertices[indices[2] as usize],
        ];
        let min_x = triangle.iter().map(|v| v.pos.x).fold(f32::INFINITY, f32::min);
        let max_x = triangle.iter().map(|v| v.pos.x).fold(f32::NEG_INFINITY, f32::max);
        for span in edges.windows(2) {
            if span[1] <= span[0] || max_x <= span[0] || min_x >= span[1] {
                continue;
            }
            let mut polygon = super::derez::clip(triangle.to_vec(), 0, span[0], true);
            polygon = super::derez::clip(polygon, 0, span[1], false);
            polygon = super::derez::clip(polygon, 1, bounds.top(), true);
            polygon = super::derez::clip(polygon, 1, bounds.bottom(), false);
            if polygon.len() < 3 {
                continue;
            }
            let base = result.vertices.len() as u32;
            for mut vertex in polygon.iter().copied() {
                vertex.color = vertex.color.gamma_multiply(edge_alpha(vertex.pos.x, viewport, center));
                result.vertices.push(vertex);
            }
            for index in 1..polygon.len() as u32 - 1 {
                result.indices.extend_from_slice(&[base, base + index, base + index + 1]);
            }
        }
    }
    result
}

/// Reuse the card's tessellation once, preserving texture UVs and paint order.
/// Central cards keep their original text/vector shapes and incur no work here.
pub(super) fn apply(ui: &Ui, marker: ShapeIdx, viewport: Rect, center: Rect) {
    let ctx = ui.ctx();
    let layer = ui.layer_id();
    let sources = ctx.graphics_mut(|graphics| {
        let list = graphics.entry(layer);
        let sources = list.all_entries().skip(marker.0).cloned().collect::<Vec<_>>();
        for index in marker.0..list.next_idx().0 {
            list.reset_shape(ShapeIdx(index));
        }
        sources
    });
    let mut shapes = Vec::new();
    for primitive in ctx.tessellate(sources, ctx.pixels_per_point()) {
        let bounds = primitive.clip_rect.intersect(viewport);
        if !bounds.is_positive() {
            continue;
        }
        match primitive.primitive {
            Primitive::Mesh(mesh) => {
                let mesh = faded_mesh(&mesh, bounds, viewport, center);
                if !mesh.indices.is_empty() {
                    shapes.push(Shape::mesh(mesh));
                }
            }
            Primitive::Callback(callback) => shapes.push(Shape::Callback(callback)),
        }
    }
    ui.painter()
        .with_clip_rect(viewport.intersect(ui.clip_rect()))
        .set(marker, Shape::Vec(shapes));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_fade_preserves_the_center_image_and_texture_coordinates() {
        let viewport = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, 100.0));
        let center = viewport.shrink2(egui::vec2(60.0, 0.0));
        let mut mesh = Mesh::with_texture(egui::TextureId::User(7));
        mesh.add_rect_with_uv(
            viewport,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        let faded = faded_mesh(&mesh, viewport, viewport, center);
        assert!(faded.is_valid());
        assert_eq!(faded.texture_id, mesh.texture_id);
        for vertex in &faded.vertices {
            assert!((vertex.uv.x * 400.0 - vertex.pos.x).abs() < 0.01);
            let expected = if vertex.pos.x == 0.0 || vertex.pos.x == 400.0 { 0 } else { 255 };
            assert_eq!(vertex.color.a(), expected);
        }
        assert_eq!(opacity(center, center), 1.0);
        assert_eq!(
            opacity(Rect::from_min_max(egui::pos2(-200.0, 0.0), egui::pos2(40.0, 100.0)), center),
            0.45
        );
        assert!(edge_alpha(30.0, viewport, center) > 0.4 && edge_alpha(30.0, viewport, center) < 0.6);
    }
}
