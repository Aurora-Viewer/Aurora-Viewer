//! Background of the loading and teleport screens: the last view of the
//! world (captured without the interface when teleporting or leaving),
//! reduced and blurred off the main thread, kept on disk for the next
//! session. Falls back to the login background when there is none.

use crate::theme::Palette;
use egui::{Color32, Pos2, Rect};
use std::path::PathBuf;

/// Width of the stored, blurred image (the blur hides the low resolution).
const WIDTH: u32 = 384;
/// Box blur radius (pixels of the reduced image) and passes (≈ gaussian).
const RADIUS: i32 = 6;
const PASSES: usize = 3;

fn path() -> PathBuf {
    crate::settings::cache_dir().join("last_view.png")
}

#[derive(Default)]
pub struct Backdrop {
    tex: Option<egui::TextureHandle>,
    rx: Option<crossbeam_channel::Receiver<egui::ColorImage>>,
}

impl Backdrop {
    /// Load the image saved by the previous session (background thread).
    pub fn load_saved(&mut self) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let Ok(img) = image::open(path()) else {
                return;
            };
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            let _ = tx.send(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()));
        });
        self.rx = Some(rx);
    }

    /// A new capture (RGBA8, opaque): reduce, blur, keep and save it.
    /// `wait`: do it now (the viewer is closing), else in a thread.
    pub fn set_capture(&mut self, w: u32, h: u32, rgba: Vec<u8>, wait: bool) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let job = move || {
            let Some((sw, sh, small)) = blurred(w, h, &rgba) else {
                return;
            };
            let p = path();
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = image::save_buffer(&p, &small, sw, sh, image::ExtendedColorType::Rgba8) {
                log::debug!("last view not saved: {e}");
            }
            let _ = tx.send(egui::ColorImage::from_rgba_unmultiplied([sw as usize, sh as usize], &small));
        };
        if wait {
            job();
        } else {
            std::thread::spawn(job);
        }
        self.rx = Some(rx);
    }

    /// Upload a finished image as a texture.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else {
            return;
        };
        if let Ok(img) = rx.try_recv() {
            self.tex = Some(ctx.load_texture("last_view", img, egui::TextureOptions::LINEAR));
            self.rx = None;
        }
    }

    /// Paint the image over `rect` ("cover"), darkened, with `alpha`.
    pub fn paint(&self, painter: &egui::Painter, p: &Palette, rect: Rect, alpha: f32) -> bool {
        let Some(t) = &self.tex else {
            return false;
        };
        let [tw, th] = t.size();
        let (ta, ra) = (tw as f32 / th.max(1) as f32, rect.width() / rect.height().max(1.0));
        // crop the texture to the screen aspect ratio
        let uv = if ta > ra {
            let k = ra / ta;
            Rect::from_min_max(Pos2::new(0.5 - k * 0.5, 0.0), Pos2::new(0.5 + k * 0.5, 1.0))
        } else {
            let k = ta / ra;
            Rect::from_min_max(Pos2::new(0.0, 0.5 - k * 0.5), Pos2::new(1.0, 0.5 + k * 0.5))
        };
        let a = alpha.clamp(0.0, 1.0);
        painter.image(t.id(), rect, uv, Color32::WHITE.gamma_multiply(a));
        // darken a little so the loading text stays readable
        let n = p.navy;
        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(n.r(), n.g(), n.b(), (110.0 * a) as u8));
        true
    }
}

/// Reduce to `WIDTH` (area average) and blur (box passes).
fn blurred(w: u32, h: u32, rgba: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
        return None;
    }
    let sw = WIDTH.min(w);
    let sh = ((h as u64 * sw as u64) / w as u64).max(1) as u32;
    let mut small = vec![0f32; (sw * sh * 3) as usize];
    for y in 0..sh {
        let (y0, y1) = (y * h / sh, ((y + 1) * h / sh).max(y * h / sh + 1));
        for x in 0..sw {
            let (x0, x1) = (x * w / sw, ((x + 1) * w / sw).max(x * w / sw + 1));
            let mut acc = [0f32; 3];
            let mut n = 0f32;
            // sample a sparse grid of the block (enough for a blurred result)
            let step_y = ((y1 - y0) / 4).max(1);
            let step_x = ((x1 - x0) / 4).max(1);
            let mut yy = y0;
            while yy < y1 {
                let mut xx = x0;
                while xx < x1 {
                    let i = ((yy * w + xx) * 4) as usize;
                    acc[0] += rgba[i] as f32;
                    acc[1] += rgba[i + 1] as f32;
                    acc[2] += rgba[i + 2] as f32;
                    n += 1.0;
                    xx += step_x;
                }
                yy += step_y;
            }
            let o = ((y * sw + x) * 3) as usize;
            for c in 0..3 {
                small[o + c] = acc[c] / n.max(1.0);
            }
        }
    }
    let (w, h) = (sw as i32, sh as i32);
    let mut tmp = vec![0f32; small.len()];
    for _ in 0..PASSES {
        box_blur(&small, &mut tmp, w, h, true);
        box_blur(&tmp, &mut small, w, h, false);
    }
    let out = small
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|c| [c[0] as u8, c[1] as u8, c[2] as u8, 255])
        .collect();
    Some((sw, sh, out))
}

/// One separable box blur pass (clamped edges).
fn box_blur(src: &[f32], dst: &mut [f32], w: i32, h: i32, horizontal: bool) {
    let norm = 1.0 / (2 * RADIUS + 1) as f32;
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    for o in 0..outer {
        let idx = |i: i32| -> usize {
            let i = i.clamp(0, inner - 1);
            let (x, y) = if horizontal { (i, o) } else { (o, i) };
            ((y * w + x) * 3) as usize
        };
        let mut acc = [0f32; 3];
        for k in -RADIUS..=RADIUS {
            let j = idx(k);
            for c in 0..3 {
                acc[c] += src[j + c];
            }
        }
        for i in 0..inner {
            let d = idx(i);
            for c in 0..3 {
                dst[d + c] = acc[c] * norm;
            }
            let (add, sub) = (idx(i + RADIUS + 1), idx(i - RADIUS));
            for c in 0..3 {
                acc[c] += src[add + c] - src[sub + c];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn blur_keeps_average_and_smooths() {
        // half black / half white 800x400
        let (w, h) = (800u32, 400u32);
        let mut px = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 0 } else { 255 };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let (sw, sh, out) = super::blurred(w, h, &px).expect("blur");
        assert_eq!((sw, sh), (384, 192));
        let at = |x: u32| out[((sh / 2 * sw + x) * 4) as usize];
        assert!(at(0) < 10 && at(sw - 1) > 245, "edges keep their color");
        let mid = at(sw / 2);
        assert!(mid > 60 && mid < 200, "the border is blurred: {mid}");
    }
}
