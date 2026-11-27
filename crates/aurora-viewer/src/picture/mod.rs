//! The Picture viewer's pictures (Aurora's own: the Legacy Viewer only opens
//! textures of the inventory): a picture of the computer, opened, changed by
//! a list of edits that is played again from the original (undo, redo), its
//! light and colour set, shown in 2D or as a card in 3D (`relief`), and
//! saved to a new file. Pure logic: the window is `ui/picture_viewer.rs`, its
//! driver `app/picture_viewer.rs`.

pub mod adjust;
pub mod geometry;
pub mod io;
pub mod relief;

use std::sync::Arc;

/// A picture in memory: RGBA8, rows top to bottom.
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<[u8]>,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture({}x{})", self.w, self.h)
    }
}

impl Picture {
    /// `rgba` must hold `w * h * 4` bytes (checked: a short buffer is padded
    /// with transparent black, a long one cut).
    pub fn new(w: u32, h: u32, mut rgba: Vec<u8>) -> Self {
        rgba.resize(w as usize * h as usize * 4, 0);
        Self { w, h, rgba: rgba.into() }
    }
}

/// A crop rectangle, in fractions of the picture (0..1), so that it keeps its
/// place on the full-size picture as on the smaller one on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Crop {
    pub const FULL: Crop = Crop {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };

    /// Kept inside the picture, at least 1% wide and high.
    pub fn clamped(self) -> Crop {
        let w = self.w.clamp(0.01, 1.0);
        let h = self.h.clamp(0.01, 1.0);
        Crop {
            x: self.x.clamp(0.0, 1.0 - w),
            y: self.y.clamp(0.0, 1.0 - h),
            w,
            h,
        }
    }

    /// The pixels of a `w`×`h` picture it keeps: x, y, width, height (at
    /// least one pixel).
    pub fn pixels(self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let c = self.clamped();
        let x = ((c.x * w as f32).round() as u32).min(w.saturating_sub(1));
        let y = ((c.y * h as f32).round() as u32).min(h.saturating_sub(1));
        let cw = ((c.w * w as f32).round() as u32).clamp(1, w - x);
        let ch = ((c.h * h as f32).round() as u32).clamp(1, h - y);
        (x, y, cw, ch)
    }
}

/// One change of the picture's shape, kept in the window's history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit {
    Crop(Crop),
    /// A quarter turn clockwise.
    RotateRight,
    /// A quarter turn counter-clockwise.
    RotateLeft,
    FlipHorizontal,
    FlipVertical,
    /// A new size, as a fraction of the current one (so that it means the
    /// same on the screen's smaller copy and on the full-size picture).
    Resize {
        sx: f32,
        sy: f32,
    },
}

/// What the window asks of a picture: its edits in order, then its light and
/// colour. Played on the original each time (`render`), so that undo is a
/// shorter list and nothing is lost to repeated resampling.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Recipe {
    pub edits: Vec<Edit>,
    pub adjust: adjust::Adjust,
}

/// The picture `recipe` makes of `original`.
pub fn render(original: &Picture, recipe: &Recipe) -> Picture {
    let mut pic = original.clone();
    for edit in &recipe.edits {
        pic = geometry::apply(&pic, *edit);
    }
    adjust::apply(&pic, &recipe.adjust)
}

/// The window's history: what was done, what was undone (to redo).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub done: Vec<Recipe>,
    pub undone: Vec<Recipe>,
}

impl History {
    /// Remember `before`, the recipe a change replaces; a new change forgets
    /// what was undone.
    pub fn push(&mut self, before: Recipe) {
        if self.done.last() != Some(&before) {
            self.done.push(before);
            // a long session keeps the last 200 steps
            if self.done.len() > 200 {
                self.done.remove(0);
            }
        }
        self.undone.clear();
    }

    pub fn undo(&mut self, current: &Recipe) -> Option<Recipe> {
        let previous = self.done.pop()?;
        self.undone.push(current.clone());
        Some(previous)
    }

    pub fn redo(&mut self, current: &Recipe) -> Option<Recipe> {
        let next = self.undone.pop()?;
        self.done.push(current.clone());
        Some(next)
    }
}

/// The demo's test picture (AURORA_DEMO_PICTURE_VIEWER): made by the viewer,
/// nothing read from disk. A colour wheel's hues across, light to dark down,
/// a bright disc (the relief's hill) and a grid that shows the crops and
/// turns.
pub fn demo_image(w: u32, h: u32) -> Picture {
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    let (cx, cy, r) = (w as f32 * 0.62, h as f32 * 0.45, w.min(h) as f32 * 0.22);
    for y in 0..h {
        for x in 0..w {
            let hue = x as f32 / w as f32;
            let value = 1.0 - 0.75 * y as f32 / h as f32;
            let [mut red, mut green, mut blue] = hsv(hue, 0.65, value);
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if d < r {
                let t = 1.0 - d / r;
                let lift = t * t * (3.0 - 2.0 * t);
                red += (1.0 - red) * lift;
                green += (1.0 - green) * lift;
                blue += (1.0 - blue) * lift;
            }
            if x % 64 == 0 || y % 64 == 0 {
                red *= 0.35;
                green *= 0.35;
                blue *= 0.35;
            }
            let i = (y as usize * w as usize + x as usize) * 4;
            rgba[i] = (red * 255.0) as u8;
            rgba[i + 1] = (green * 255.0) as u8;
            rgba[i + 2] = (blue * 255.0) as u8;
            rgba[i + 3] = 255;
        }
    }
    Picture::new(w, h, rgba)
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match i as i32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests;
