//! CPU painter for the small recovery window. No graphics adapter, surface,
//! shader, GPU thread or driver is initialized after a rendering crash.
//! Only egui meshes are accepted: the recovery UI has no paint callbacks.

use egui::{
    Color32, ColorImage, TextureId,
    epaint::{ClippedPrimitive, Primitive, Vertex},
};
use std::collections::HashMap;

#[derive(Default)]
pub struct Painter {
    textures: HashMap<TextureId, ColorImage>,
}

impl Painter {
    pub fn paint(
        &mut self,
        pixels: &mut [u32],
        size: [usize; 2],
        primitives: &[ClippedPrimitive],
        delta: &egui::TexturesDelta,
        scale: f32,
        background: Color32,
    ) {
        for (id, changes) in &delta.set {
            for change in changes {
                let egui::ImageData::Color(image) = &change.image;
                if let Some([x, y]) = change.pos {
                    if let Some(texture) = self.textures.get_mut(id) {
                        for row in 0..image.height() {
                            if y + row >= texture.height() || x + image.width() > texture.width() {
                                continue;
                            }
                            let start = (y + row) * texture.width() + x;
                            texture.pixels[start..start + image.width()]
                                .copy_from_slice(&image.pixels[row * image.width()..(row + 1) * image.width()]);
                        }
                    }
                } else {
                    self.textures.insert(*id, image.as_ref().clone());
                }
            }
        }
        pixels.fill(rgb(background));
        for primitive in primitives {
            let Primitive::Mesh(mesh) = &primitive.primitive else { continue };
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            if texture.width() == 0 || texture.height() == 0 {
                continue;
            }
            let clip = [
                (primitive.clip_rect.min.x * scale).floor().max(0.0) as usize,
                (primitive.clip_rect.min.y * scale).floor().max(0.0) as usize,
                (primitive.clip_rect.max.x * scale).ceil().max(0.0).min(size[0] as f32) as usize,
                (primitive.clip_rect.max.y * scale).ceil().max(0.0).min(size[1] as f32) as usize,
            ];
            for indices in mesh.indices.as_chunks::<3>().0 {
                if let (Some(a), Some(b), Some(c)) = (
                    mesh.vertices.get(indices[0] as usize),
                    mesh.vertices.get(indices[1] as usize),
                    mesh.vertices.get(indices[2] as usize),
                ) {
                    triangle(pixels, size, clip, [*a, *b, *c], texture, scale);
                }
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
    }
}

fn rgb(color: Color32) -> u32 {
    (u32::from(color.r()) << 16) | (u32::from(color.g()) << 8) | u32::from(color.b())
}

fn edge(a: egui::Pos2, b: egui::Pos2, p: egui::Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

fn top_left(a: egui::Pos2, b: egui::Pos2) -> bool {
    b.y < a.y || (b.y == a.y && b.x > a.x)
}

fn triangle(pixels: &mut [u32], size: [usize; 2], clip: [usize; 4], mut v: [Vertex; 3], texture: &ColorImage, scale: f32) {
    for vertex in &mut v {
        vertex.pos *= scale;
    }
    let mut area = edge(v[0].pos, v[1].pos, v[2].pos);
    if area.abs() < f32::EPSILON {
        return;
    }
    if area < 0.0 {
        v.swap(1, 2);
        area = -area;
    }
    let min_x = v.iter().map(|v| v.pos.x).fold(f32::INFINITY, f32::min).floor().max(clip[0] as f32) as usize;
    let min_y = v.iter().map(|v| v.pos.y).fold(f32::INFINITY, f32::min).floor().max(clip[1] as f32) as usize;
    let max_x = v
        .iter()
        .map(|v| v.pos.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(0.0)
        .min(clip[2] as f32) as usize;
    let max_y = v
        .iter()
        .map(|v| v.pos.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(0.0)
        .min(clip[3] as f32) as usize;
    let inclusive = [
        top_left(v[1].pos, v[2].pos),
        top_left(v[2].pos, v[0].pos),
        top_left(v[0].pos, v[1].pos),
    ];
    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            let weights = [
                edge(v[1].pos, v[2].pos, p),
                edge(v[2].pos, v[0].pos, p),
                edge(v[0].pos, v[1].pos, p),
            ];
            if weights.iter().zip(inclusive).any(|(&w, include)| w < 0.0 || (w == 0.0 && !include)) {
                continue;
            }
            let w = weights.map(|w| w / area);
            let uv = v[0].uv.to_vec2() * w[0] + v[1].uv.to_vec2() * w[1] + v[2].uv.to_vec2() * w[2];
            let texel = sample(texture, uv);
            let mut color = [0.0; 4];
            for (channel, value) in color.iter_mut().enumerate() {
                *value = (0..3).map(|i| f32::from(v[i].color[channel]) * w[i]).sum::<f32>() * texel[channel] / 255.0;
            }
            let target = &mut pixels[y * size[0] + x];
            let alpha = color[3] / 255.0;
            let mut blended = 0;
            for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                let destination = ((*target >> shift) & 255) as f32;
                blended |= ((color[channel] + destination * (1.0 - alpha)).round().clamp(0.0, 255.0) as u32) << shift;
            }
            *target = blended;
        }
    }
}

fn sample(texture: &ColorImage, uv: egui::Vec2) -> [f32; 4] {
    let x = (uv.x * texture.width() as f32 - 0.5).clamp(0.0, texture.width().saturating_sub(1) as f32);
    let y = (uv.y * texture.height() as f32 - 0.5).clamp(0.0, texture.height().saturating_sub(1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(texture.width() - 1);
    let y1 = (y0 + 1).min(texture.height() - 1);
    let a = texture[(x0, y0)];
    let b = texture[(x1, y0)];
    let c = texture[(x0, y1)];
    let d = texture[(x1, y1)];
    let (tx, ty) = (x - x.floor(), y - y.floor());
    std::array::from_fn(|i| {
        (f32::from(a[i]) * (1.0 - tx) + f32::from(b[i]) * tx) * (1.0 - ty) + (f32::from(c[i]) * (1.0 - tx) + f32::from(d[i]) * tx) * ty
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_triangle_edges_blend_once_and_obey_clip() {
        let texture = ColorImage::filled([1, 1], Color32::WHITE);
        let vertex = |x, y| Vertex {
            pos: egui::pos2(x, y),
            uv: egui::pos2(0.5, 0.5),
            color: Color32::from_rgba_premultiplied(128, 0, 0, 128),
        };
        let mut pixels = [0; 16];
        let v = [vertex(0.0, 0.0), vertex(4.0, 0.0), vertex(4.0, 4.0), vertex(0.0, 4.0)];
        triangle(&mut pixels, [4, 4], [1, 1, 3, 3], [v[0], v[1], v[2]], &texture, 1.0);
        triangle(&mut pixels, [4, 4], [1, 1, 3, 3], [v[0], v[2], v[3]], &texture, 1.0);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    pixels[y * 4 + x],
                    if (1..3).contains(&x) && (1..3).contains(&y) { 0x800000 } else { 0 }
                );
            }
        }
    }
}
