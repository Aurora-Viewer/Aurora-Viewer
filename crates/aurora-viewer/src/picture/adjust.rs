//! The picture's light and colour: brightness, contrast, saturation, warmth
//! and sharpness, on the processor, the rows shared between the cores
//! (rayon). Each setting is 0 when untouched, from -1 to 1 (sharpness 0 to
//! 1), and a picture with every setting at 0 comes back unchanged.

use super::Picture;
use rayon::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Adjust {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warmth: f32,
    pub sharpen: f32,
}

impl Adjust {
    pub fn is_neutral(&self) -> bool {
        *self == Adjust::default()
    }

    /// Every setting inside its range (a value from the demo or a slider
    /// typed in).
    pub fn clamped(self) -> Adjust {
        let c = |v: f32| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
        Adjust {
            brightness: c(self.brightness),
            contrast: c(self.contrast),
            saturation: c(self.saturation),
            warmth: c(self.warmth),
            sharpen: c(self.sharpen).max(0.0),
        }
    }
}

pub fn apply(pic: &Picture, adjust: &Adjust) -> Picture {
    let a = adjust.clamped();
    if a.is_neutral() {
        return pic.clone();
    }
    let source = if a.sharpen > 0.0 {
        sharpen(pic, a.sharpen)
    } else {
        pic.rgba.to_vec()
    };
    let mut out = source;
    let tone = Tone::new(&a);
    out.par_chunks_mut(4 * pic.w.max(1) as usize).for_each(|row| {
        for px in row.as_chunks_mut::<4>().0 {
            let rgb = tone.apply([px[0], px[1], px[2]]);
            px[..3].copy_from_slice(&rgb);
        }
    });
    Picture::new(pic.w, pic.h, out)
}

/// The settings that work pixel by pixel, prepared once.
struct Tone {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    warm: [f32; 3],
}

impl Tone {
    fn new(a: &Adjust) -> Self {
        Self {
            // ±40% of the range at the ends: strong, never a white or black page
            brightness: a.brightness * 0.4,
            // a curve around the middle grey: x2 at +1, flat at -1
            contrast: if a.contrast >= 0.0 {
                1.0 + a.contrast
            } else {
                1.0 + a.contrast * 0.9
            },
            saturation: 1.0 + a.saturation,
            // warmer: more red, less blue (and the other way)
            warm: [1.0 + a.warmth * 0.18, 1.0 + a.warmth * 0.04, 1.0 - a.warmth * 0.18],
        }
    }

    fn apply(&self, rgb: [u8; 3]) -> [u8; 3] {
        let mut c = rgb.map(|v| v as f32 / 255.0);
        for (v, w) in c.iter_mut().zip(self.warm) {
            *v *= w;
        }
        for v in &mut c {
            *v = (*v - 0.5) * self.contrast + 0.5 + self.brightness;
        }
        // Rec. 709 luma, the greys a saturation of -1 gives
        let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        c.map(|v| ((luma + (v - luma) * self.saturation).clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

/// An unsharp mask: each pixel pushed away from the mean of its 3×3
/// neighbours, by up to 1.5 times the difference.
fn sharpen(pic: &Picture, amount: f32) -> Vec<u8> {
    let (w, h) = (pic.w as usize, pic.h as usize);
    let src = &pic.rgba;
    let k = amount * 1.5;
    let mut out = src.to_vec();
    out.par_chunks_mut(4 * w.max(1)).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            for c in 0..3 {
                let mut sum = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                        let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                        sum += src[(sy * w + sx) * 4 + c] as f32;
                    }
                }
                let v = src[(y * w + x) * 4 + c] as f32;
                let blurred = sum / 9.0;
                row[x * 4 + c] = (v + (v - blurred) * k).round().clamp(0.0, 255.0) as u8;
            }
        }
    });
    out
}
