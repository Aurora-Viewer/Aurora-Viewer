//! The picture in 3D: a card standing up, facing the preview's camera, that
//! the window turns and zooms (`aurora_render::preview`). Its Relief pushes
//! the light parts of the picture towards the viewer: a grid whose depth is
//! the picture's brightness, smoothed so that noise does not become spikes.

use super::Picture;
use aurora_render::preview::{Image, Material, Model, Part, Vertex, View};
use glam::Vec3;
use std::sync::Arc;

/// Grid cells on the card's long side once it has relief.
pub const CELLS: u32 = 256;
/// Depth at the brightest point at full relief, for a card 2 units long.
const DEPTH: f32 = 0.6;

/// The card of `pic`, its relief from 0 (flat) to 1. The card is 2 units on
/// its long side, in the Y (right) / Z (up) plane, its front towards +X, the
/// side the preview's camera looks from at a yaw of 0.
pub fn card(pic: &Picture, relief: f32) -> Model {
    let relief = if relief.is_finite() { relief.clamp(0.0, 1.0) } else { 0.0 };
    let long = pic.w.max(pic.h).max(1) as f32;
    let (half_w, half_h) = (pic.w.max(1) as f32 / long, pic.h.max(1) as f32 / long);
    let (nx, ny) = if relief > 0.0 { cells(pic.w, pic.h) } else { (1, 1) };
    let heights = if relief > 0.0 {
        brightness_grid(pic, nx, ny)
    } else {
        vec![0.0; ((nx + 1) * (ny + 1)) as usize]
    };
    let depth = relief * DEPTH;
    let at = |i: u32, j: u32| heights[(j * (nx + 1) + i) as usize] * depth;
    let mut vertices = Vec::with_capacity(((nx + 1) * (ny + 1)) as usize);
    // the size of one cell, for the slopes of the normals
    let (dy, dz) = (2.0 * half_w / nx as f32, 2.0 * half_h / ny as f32);
    for j in 0..=ny {
        for i in 0..=nx {
            let (u, v) = (i as f32 / nx as f32, j as f32 / ny as f32);
            let y = -half_w + u * 2.0 * half_w;
            let z = half_h - v * 2.0 * half_h;
            // slopes from the neighbours (one-sided at the edges)
            let (i0, i1) = (i.saturating_sub(1), (i + 1).min(nx));
            let (j0, j1) = (j.saturating_sub(1), (j + 1).min(ny));
            let slope_y = (at(i1, j) - at(i0, j)) / ((i1 - i0).max(1) as f32 * dy);
            // j grows downwards, z upwards
            let slope_z = (at(i, j0) - at(i, j1)) / ((j1 - j0).max(1) as f32 * dz);
            let normal = Vec3::new(1.0, -slope_y, -slope_z).normalize_or(Vec3::X);
            vertices.push(Vertex::new([at(i, j), y, z], normal.to_array(), [u, v]));
        }
    }
    let mut indices = Vec::with_capacity((nx * ny * 6) as usize);
    for j in 0..ny {
        for i in 0..nx {
            let a = j * (nx + 1) + i;
            let b = a + 1;
            let c = a + nx + 1;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let image = Arc::new(Image {
        size: [pic.w, pic.h],
        rgba: pic.rgba.clone(),
    });
    let material = Material {
        // lit enough for the relief to show its shapes, glowing enough for
        // the picture to keep its colours when turned away from the light
        color: [0.55, 0.55, 0.55, 1.0],
        emissive: [0.6, 0.6, 0.6],
        roughness: 0.85,
        double_sided: true,
        ..Material::default()
    };
    let part = Part {
        diagnostic: false,
        vertices: vertices.into(),
        indices: indices.into(),
        textures: [Some(image.clone()), None, None, Some(image)],
        material,
        skin: None,
    };
    Model {
        parts: vec![part],
        center: Vec3::new(depth * 0.5, 0.0, 0.0),
        radius: (half_w * half_w + half_h * half_h).sqrt().max(0.01),
    }
}

/// The camera the window starts with, and comes back to on a double click:
/// in front of the card, a little above it.
pub fn front_view(background: [f32; 4]) -> View {
    View {
        yaw: 0.0,
        pitch: 0.12,
        distance: 2.2,
        background,
        ambient: [0.35; 3],
        ..View::default()
    }
}

/// The grid of a `w`×`h` picture: `CELLS` on its long side, the short one in
/// proportion (at least one).
pub fn cells(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h).max(1) as f32;
    let side = |n: u32| ((n as f32 / long * CELLS as f32).round() as u32).max(1);
    (side(w), side(h))
}

/// The picture's brightness (0..1) at each corner of an `nx`×`ny` grid: the
/// mean of the pixels around each corner, then smoothed once over the grid.
fn brightness_grid(pic: &Picture, nx: u32, ny: u32) -> Vec<f32> {
    let (w, h) = (pic.w.max(1), pic.h.max(1));
    let (gw, gh) = (nx + 1, ny + 1);
    let luma = |x: u32, y: u32| {
        let i = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
        let p = &pic.rgba[i..i + 4];
        // a transparent pixel stays flat
        let a = p[3] as f32 / 255.0;
        (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0 * a
    };
    let mut grid = vec![0.0; (gw * gh) as usize];
    // the pixels a corner stands for: half a cell each way, sampled at most
    // 4×4 times (enough to smooth, cheap on a big picture)
    let (cell_x, cell_y) = (w as f32 / nx as f32, h as f32 / ny as f32);
    for j in 0..gh {
        for i in 0..gw {
            let (cx, cy) = (i as f32 * cell_x, j as f32 * cell_y);
            let mut sum = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = cx + (sx as f32 - 1.5) / 4.0 * cell_x;
                    let y = cy + (sy as f32 - 1.5) / 4.0 * cell_y;
                    sum += luma(x.max(0.0) as u32, y.max(0.0) as u32);
                }
            }
            grid[(j * gw + i) as usize] = sum / 16.0;
        }
    }
    // one 3×3 box pass over the grid
    let mut smooth = grid.clone();
    for j in 0..gh {
        for i in 0..gw {
            let mut sum = 0.0;
            let mut n = 0.0;
            for dj in -1i32..=1 {
                for di in -1i32..=1 {
                    let (x, y) = (i as i32 + di, j as i32 + dj);
                    if x >= 0 && y >= 0 && x < gw as i32 && y < gh as i32 {
                        sum += grid[(y as u32 * gw + x as u32) as usize];
                        n += 1.0;
                    }
                }
            }
            smooth[(j * gw + i) as usize] = sum / n;
        }
    }
    smooth
}
