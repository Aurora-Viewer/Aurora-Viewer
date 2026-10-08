//! Drawing 3D handles and guides with egui over the scene: lines clipped at
//! the near plane, shaded triangles sorted back to front, labels.

use super::geom::Cam;
use egui::{Color32, FontId, Pos2, Shape, Stroke};
use glam::{Quat, Vec3, Vec4};

const NEAR_W: f32 = 0.02;

pub struct Painter3d<'a> {
    pub painter: egui::Painter,
    pub cam: &'a Cam,
    /// Triangles waiting for the depth sort: (depth, points, color).
    tris: Vec<(f32, [Pos2; 3], Color32)>,
}

pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (r.clamp(0.0, 1.0) * 255.0) as u8,
        (g.clamp(0.0, 1.0) * 255.0) as u8,
        (b.clamp(0.0, 1.0) * 255.0) as u8,
        (a.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// Same color, alpha multiplied.
pub fn fade(c: Color32, a: f32) -> Color32 {
    let [r, g, b, ca] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, (ca as f32 * a.clamp(0.0, 1.0)) as u8)
}

fn shade(c: Color32, k: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let f = |v: u8| ((v as f32) * k).clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(r), f(g), f(b), a)
}

impl<'a> Painter3d<'a> {
    pub fn new(painter: egui::Painter, cam: &'a Cam) -> Self {
        Painter3d {
            painter,
            cam,
            tris: Vec::new(),
        }
    }

    fn clip_segment(&self, a: Vec3, b: Vec3) -> Option<(Pos2, Pos2)> {
        let (mut ca, mut cb) = (self.cam.clip(a), self.cam.clip(b));
        if ca.w < NEAR_W && cb.w < NEAR_W {
            return None;
        }
        let cut = |p: Vec4, q: Vec4| p + (q - p) * ((NEAR_W - p.w) / (q.w - p.w));
        if ca.w < NEAR_W {
            ca = cut(ca, cb);
        } else if cb.w < NEAR_W {
            cb = cut(cb, ca);
        }
        Some((self.cam.clip_to_screen(ca), self.cam.clip_to_screen(cb)))
    }

    pub fn line(&self, a: Vec3, b: Vec3, color: Color32, width: f32) {
        if let Some((pa, pb)) = self.clip_segment(a, b) {
            self.painter.line_segment([pa, pb], Stroke::new(width, color));
        }
    }

    /// Line whose color goes from `ca` to `cb` (split in a few pieces).
    pub fn line_gradient(&self, a: Vec3, b: Vec3, ca: Color32, cb: Color32, width: f32) {
        const N: usize = 6;
        for i in 0..N {
            let (t0, t1) = (i as f32 / N as f32, (i + 1) as f32 / N as f32);
            let c = lerp_color(ca, cb, (t0 + t1) * 0.5);
            self.line(a.lerp(b, t0), a.lerp(b, t1), c, width);
        }
    }

    /// Lines with the LL snap guide passes: a dark shadow offset by one pixel,
    /// then the line.
    pub fn guide_line(&self, a: Vec3, b: Vec3, color: Color32, opacity: f32) {
        if let Some((pa, pb)) = self.clip_segment(a, b) {
            let off = egui::vec2(1.0, 1.0) / self.cam.ppp;
            self.painter
                .line_segment([pa + off, pb + off], Stroke::new(2.0, rgba(0.0, 0.0, 0.0, 0.31 * opacity)));
            self.painter.line_segment([pa, pb], Stroke::new(1.0, fade(color, opacity)));
        }
    }

    /// One triangle, flat shaded by its facing (`normal` in world space).
    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, normal: Vec3, color: Color32) {
        let (ca, cb, cc) = (self.cam.clip(a), self.cam.clip(b), self.cam.clip(c));
        if ca.w < NEAR_W || cb.w < NEAR_W || cc.w < NEAR_W {
            return;
        }
        let to_eye = (self.cam.eye - (a + b + c) / 3.0).normalize_or_zero();
        let light = 0.55 + 0.45 * normal.normalize_or_zero().dot(to_eye).abs();
        let depth = (ca.w + cb.w + cc.w) / 3.0;
        self.tris.push((
            depth,
            [
                self.cam.clip_to_screen(ca),
                self.cam.clip_to_screen(cb),
                self.cam.clip_to_screen(cc),
            ],
            shade(color, light),
        ));
    }

    /// A cone along `dir` from its base center (radius `r`, height `h`).
    pub fn cone(&mut self, base: Vec3, dir: Vec3, r: f32, h: f32, color: Color32) {
        const SIDES: usize = 12;
        let d = dir.normalize_or_zero();
        let u = d.any_orthonormal_vector();
        let v = d.cross(u);
        let apex = base + d * h;
        let ring: Vec<Vec3> = (0..SIDES)
            .map(|i| {
                let a = i as f32 / SIDES as f32 * std::f32::consts::TAU;
                base + (u * a.cos() + v * a.sin()) * r
            })
            .collect();
        for i in 0..SIDES {
            let (p, q) = (ring[i], ring[(i + 1) % SIDES]);
            let n = (q - p).cross(apex - p);
            self.tri(p, q, apex, n, color);
            self.tri(q, p, base, -d, shade(color, 0.8));
        }
    }

    /// An oriented box.
    pub fn cube(&mut self, center: Vec3, rot: Quat, half: Vec3, color: Color32) {
        let c = |x: f32, y: f32, z: f32| center + rot * (half * Vec3::new(x, y, z));
        for axis in 0..3 {
            for s in [-1.0f32, 1.0] {
                let mut n = Vec3::ZERO;
                n[axis] = s;
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let corner = |a: f32, b: f32| {
                    let mut p = Vec3::ZERO;
                    p[axis] = s;
                    p[u] = a;
                    p[v] = b;
                    c(p.x, p.y, p.z)
                };
                let (p0, p1, p2, p3) = (corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0));
                let nw = rot * n;
                self.tri(p0, p1, p2, nw, color);
                self.tri(p0, p2, p3, nw, color);
            }
        }
    }

    /// A flat quad (two triangles).
    pub fn quad(&mut self, p: [Vec3; 4], color: Color32) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        self.tri(p[0], p[1], p[2], n, color);
        self.tri(p[0], p[2], p[3], n, color);
    }

    /// Paint the queued triangles, farthest first.
    pub fn flush(&mut self) {
        self.tris.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut mesh = egui::Mesh::default();
        for (_, p, c) in self.tris.drain(..) {
            let i = mesh.vertices.len() as u32;
            for q in p {
                mesh.colored_vertex(q, c);
            }
            mesh.add_triangle(i, i + 1, i + 2);
        }
        if !mesh.vertices.is_empty() {
            self.painter.add(Shape::mesh(mesh));
        }
    }

    /// Label with a drop shadow, centered on a world point.
    pub fn text(&self, p: Vec3, text: &str, size: f32, color: Color32) {
        let Some(s) = self.cam.project(p) else {
            return;
        };
        let font = FontId::proportional(size);
        self.painter.text(
            s + egui::vec2(1.0, 1.0),
            egui::Align2::CENTER_CENTER,
            text,
            font.clone(),
            fade(Color32::BLACK, color.a() as f32 / 255.0),
        );
        self.painter.text(s, egui::Align2::CENTER_CENTER, text, font, color);
    }

    /// LLManip::renderTickValue: integer part large, fraction and unit small.
    pub fn tick_value(&self, p: Vec3, value: f32, suffix: &str, color: Color32) {
        let Some(s) = self.cam.project(p) else {
            return;
        };
        let neg = value < 0.0;
        let v = value.abs();
        let whole = v.floor();
        let frac = ((v - whole) * 100.0).round() as i32;
        let (whole, frac) = if frac >= 100 { (whole + 1.0, 0) } else { (whole, frac) };
        let big = format!("{}{}", if neg { "-" } else { "" }, whole as i64);
        let small = if frac != 0 {
            format!(".{frac:02}{suffix}")
        } else {
            suffix.to_owned()
        };
        let shadow = fade(Color32::BLACK, color.a() as f32 / 255.0);
        let (fb, fs) = (FontId::proportional(13.0), FontId::proportional(10.0));
        for (off, col) in [(egui::vec2(1.0, 1.0), shadow), (egui::Vec2::ZERO, color)] {
            self.painter.text(s + off, egui::Align2::RIGHT_BOTTOM, &big, fb.clone(), col);
            self.painter
                .text(s + off + egui::vec2(0.0, -1.0), egui::Align2::LEFT_BOTTOM, &small, fs.clone(), col);
        }
    }
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let (a, b) = (a.to_srgba_unmultiplied(), b.to_srgba_unmultiplied());
    let f = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8;
    Color32::from_rgba_unmultiplied(f(0), f(1), f(2), f(3))
}
