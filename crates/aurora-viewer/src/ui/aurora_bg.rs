//! Animated aurora background (login, loading screens without a last view):
//! night sky gradient, twinkling stars, slowly moving aurora curtains in the
//! brand colors (additive light) and mountain silhouettes on the horizon.
//! Drawn with egui meshes: a few hundred vertices, no textures.

use crate::theme::Palette;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, Shape};

fn v(pos: Pos2, color: Color32) -> Vertex {
    Vertex { pos, uv: WHITE_UV, color }
}

/// Additive light: premultiplied color with zero alpha adds to what is below.
fn glow(c: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    Color32::from_rgba_premultiplied((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8, 0)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Deterministic pseudo-random in 0..1.
fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Vertical gradient quad.
fn gradient(mesh: &mut Mesh, r: Rect, top: Color32, bottom: Color32) {
    let i = mesh.vertices.len() as u32;
    mesh.vertices.push(v(r.left_top(), top));
    mesh.vertices.push(v(r.right_top(), top));
    mesh.vertices.push(v(r.right_bottom(), bottom));
    mesh.vertices.push(v(r.left_bottom(), bottom));
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// Paint the background over `rect`; `t` = time in seconds, `alpha` fades it.
pub fn paint(painter: &egui::Painter, p: &Palette, rect: Rect, t: f32, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    let (w, h) = (rect.width(), rect.height());
    let fade = |c: Color32| c.gamma_multiply(a);

    // ---- sky
    let mut sky = Mesh::default();
    let night = p.navy;
    let deep = mix(p.navy, Color32::BLACK, 0.45);
    let horizon = mix(p.navy, p.indigo, 0.22);
    let mid = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + h * 0.55));
    gradient(&mut sky, mid, fade(deep), fade(night));
    gradient(
        &mut sky,
        Rect::from_min_max(mid.left_bottom(), rect.max),
        fade(night),
        fade(horizon),
    );
    painter.add(Shape::mesh(sky));

    // ---- stars (upper sky), twinkling
    for i in 0..170u32 {
        let x = rect.left() + hash(i * 3) * w;
        let y = rect.top() + hash(i * 3 + 1).powf(1.6) * h * 0.72;
        let s = 0.5 + hash(i * 3 + 2) * 1.2;
        let tw = 0.55 + 0.45 * (t * (0.6 + hash(i + 977) * 1.8) + hash(i + 31) * std::f32::consts::TAU).sin();
        let c = if i % 9 == 0 {
            p.teal
        } else if i % 7 == 0 {
            p.violet_pale
        } else {
            Color32::WHITE
        };
        painter.circle_filled(Pos2::new(x, y), s, glow(c, 0.55 * tw * a));
    }

    // ---- aurora curtains (additive)
    let mut aur = Mesh::default();
    let curtains: [(f32, f32, Color32, Color32, f32, f32); 4] = [
        // base y, height, top color, bottom color, speed, phase
        (0.30, 0.26, p.violet, p.teal, 0.050, 0.0),
        (0.38, 0.20, p.indigo, p.violet, -0.035, 1.7),
        (0.24, 0.16, p.teal, p.indigo, 0.028, 3.1),
        (0.46, 0.14, p.violet_light, p.teal, -0.020, 4.6),
    ];
    const N: usize = 120;
    for (k, (base, height, top_c, low_c, speed, phase)) in curtains.iter().enumerate() {
        let start = aur.vertices.len() as u32;
        for s in 0..=N {
            let u = s as f32 / N as f32;
            let x = rect.left() + u * w;
            let wave = (u * 5.3 + t * speed * 6.0 + phase).sin() * 0.045
                + (u * 11.7 - t * speed * 9.0 + phase * 2.0).sin() * 0.018
                + (u * 2.1 + t * 0.03 + phase).sin() * 0.03;
            let y = rect.top() + (base + wave) * h;
            // vertical "rays": brightness varies along the curtain
            let rays = 0.55 + 0.45 * ((u * 38.0 + t * 0.35 + phase * 3.0).sin() * 0.5 + 0.5).powf(1.5);
            let ends = (u * std::f32::consts::PI).sin().powf(0.6); // dims at the edges
            let breathe = 0.8 + 0.2 * (t * 0.25 + phase + u * 3.0).sin();
            let strength = (0.34 - k as f32 * 0.05) * rays * ends * breathe * a;
            let tint = mix(*top_c, *low_c, 0.5 + 0.5 * (u * 3.0 + phase + t * 0.05).sin());
            let ht = height * h * (0.75 + 0.25 * (u * 7.0 + phase + t * 0.1).sin());
            // top (faint, tall), bright lower edge, quick fade below
            aur.vertices.push(v(Pos2::new(x, y - ht), glow(tint, 0.0)));
            aur.vertices.push(v(Pos2::new(x, y - ht * 0.35), glow(tint, strength * 0.55)));
            aur.vertices.push(v(Pos2::new(x, y), glow(*low_c, strength)));
            aur.vertices.push(v(Pos2::new(x, y + h * 0.035), glow(*low_c, 0.0)));
        }
        for s in 0..N as u32 {
            let i = start + s * 4;
            for r in 0..3 {
                let (a0, a1, b0, b1) = (i + r, i + r + 1, i + 4 + r, i + 4 + r + 1);
                aur.add_triangle(a0, b0, b1);
                aur.add_triangle(a0, b1, a1);
            }
        }
    }
    painter.add(Shape::mesh(aur));

    // ---- horizon glow
    let mut haze = Mesh::default();
    let hz = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + h * 0.62),
        Pos2::new(rect.right(), rect.top() + h * 0.86),
    );
    let i = haze.vertices.len() as u32;
    haze.vertices.push(v(hz.left_top(), glow(p.violet, 0.0)));
    haze.vertices.push(v(hz.right_top(), glow(p.violet, 0.0)));
    haze.vertices.push(v(hz.right_bottom(), glow(p.violet, 0.10 * a)));
    haze.vertices.push(v(hz.left_bottom(), glow(p.indigo, 0.10 * a)));
    haze.add_triangle(i, i + 1, i + 2);
    haze.add_triangle(i, i + 2, i + 3);
    painter.add(Shape::mesh(haze));

    // ---- mountains: two ridgelines (far lighter, near darker)
    let ridge = |seed: f32, base: f32, amp: f32, color: Color32| {
        let mut m = Mesh::default();
        const R: usize = 96;
        for s in 0..=R {
            let u = s as f32 / R as f32;
            let x = rect.left() + u * w;
            let n = (u * 3.1 + seed).sin() * 0.5
                + (u * 7.3 + seed * 2.0).sin() * 0.28
                + (u * 17.0 + seed * 3.0).sin() * 0.12
                + ((u * 41.0 + seed).sin() * 0.06);
            let y = rect.top() + (base - amp * (0.5 + 0.5 * n)) * h;
            m.vertices.push(v(Pos2::new(x, y), color));
            m.vertices.push(v(Pos2::new(x, rect.bottom()), color));
        }
        for s in 0..R as u32 {
            let i = s * 2;
            m.add_triangle(i, i + 2, i + 1);
            m.add_triangle(i + 1, i + 2, i + 3);
        }
        painter.add(Shape::mesh(m));
    };
    ridge(0.7, 0.86, 0.16, fade(mix(p.navy, p.indigo, 0.16)));
    ridge(2.9, 0.93, 0.13, fade(mix(p.navy, Color32::BLACK, 0.35)));
}

/// Soft radial light (additive): `intensity` at the center fading to zero
/// at `radius`, smooth (no visible rings).
pub fn radial_glow(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32, intensity: f32) {
    let mut m = Mesh::default();
    const SEG: usize = 48;
    const RINGS: [(f32, f32); 4] = [(0.0, 1.0), (0.35, 0.62), (0.7, 0.22), (1.0, 0.0)];
    for (r, k) in RINGS {
        for s in 0..SEG {
            let a = s as f32 / SEG as f32 * std::f32::consts::TAU;
            let pos = center + egui::vec2(a.cos(), a.sin()) * radius * r;
            m.vertices.push(v(pos, glow(color, intensity * k)));
        }
    }
    for ring in 0..RINGS.len() as u32 - 1 {
        for s in 0..SEG as u32 {
            let (a0, a1) = (ring * SEG as u32 + s, ring * SEG as u32 + (s + 1) % SEG as u32);
            let (b0, b1) = (a0 + SEG as u32, a1 + SEG as u32);
            m.add_triangle(a0, b0, b1);
            m.add_triangle(a0, b1, a1);
        }
    }
    painter.add(Shape::mesh(m));
}
