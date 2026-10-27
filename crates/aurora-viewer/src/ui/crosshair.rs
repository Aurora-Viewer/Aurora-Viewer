//! The crosshair in the middle of the screen in mouselook (issue #1166),
//! and its settings (Preferences › Camera › Mouselook › Crosshair).
//!
//! The Legacy Viewer only has ShowCrosshairs (on): LLToolGun::draw puts the
//! 8×8 `crosshairs.tga` (four white corners with a black border) in the
//! middle of the world view. Aurora keeps that switch and that look (the
//! `Legacy` shape), and adds the choices of a game's crosshair: a dot (the
//! default, the human's choice of 2026-10-27), a cross, a circle or a
//! picture of one's own, with their size, thickness, gap, colour and
//! outline. Drawn with the painter, crisp at any interface scale.

use crate::settings::Rgba;
use crate::theme::Palette;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Stroke, TextureHandle, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

/// What is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Dot,
    Cross,
    Circle,
    /// The Legacy Viewer's four corners (crosshairs.tga).
    Legacy,
    /// A picture chosen by the user.
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// ShowCrosshairs (on).
    pub show: bool,
    pub shape: Shape,
    /// The cross's width, the circle's diameter, the Legacy corners' and the
    /// picture's size (points).
    pub size: f32,
    /// The dot's diameter, alone or in the middle of a cross or a circle.
    pub dot_size: f32,
    /// The lines of the cross and the circle.
    pub thickness: f32,
    /// The empty middle of the cross.
    pub gap: f32,
    /// A dot in the middle of the cross or the circle.
    pub center_dot: bool,
    pub color: Rgba,
    /// A thin dark border, readable over a bright scene.
    pub outline: bool,
    /// The picture of the `Image` shape (PNG, JPEG, TGA…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show: true,
            shape: Shape::Dot,
            size: 16.0,
            dot_size: 4.0,
            thickness: 2.0,
            gap: 6.0,
            center_dot: false,
            color: [255, 255, 255, 230],
            outline: true,
            image: None,
        }
    }
}

/// Ranges of the sliders (points).
pub const SIZE: std::ops::RangeInclusive<f32> = 4.0..=128.0;
pub const DOT_SIZE: std::ops::RangeInclusive<f32> = 1.0..=16.0;
pub const THICKNESS: std::ops::RangeInclusive<f32> = 1.0..=8.0;
pub const GAP: std::ops::RangeInclusive<f32> = 0.0..=32.0;

/// Width of the outline around each mark.
const OUTLINE: f32 = 1.0;

/// crosshairs.tga, top row first: `#` its white pixels, `o` its black
/// border, `.` transparent.
const LEGACY: [&[u8; 8]; 8] = [
    b".oo..oo.",
    b"o##..##o",
    b"o#....#o",
    b"........",
    b"........",
    b"o#....#o",
    b"o##..##o",
    b".oo..oo.",
];

/// One mark of the crosshair, before its colours.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mark {
    Rect(Rect),
    Disc {
        radius: f32,
    },
    Ring {
        radius: f32,
        width: f32,
    },
    /// A Legacy border pixel: drawn in the outline's colour, and only with
    /// the outline.
    Border(Rect),
}

/// The marks of `s` around `c`, each edge on a whole physical pixel
/// (`ppp`) so that a thin line stays sharp.
fn marks(c: Pos2, s: &Settings, ppp: f32) -> Vec<Mark> {
    let snap = |v: f32| (v * ppp).round() / ppp;
    let c = pos2(snap(c.x), snap(c.y));
    let dot = |out: &mut Vec<Mark>| {
        out.push(Mark::Disc {
            radius: s.dot_size.max(1.0 / ppp) / 2.0,
        })
    };
    let mut out = Vec::new();
    match s.shape {
        Shape::Dot | Shape::Image => dot(&mut out),
        Shape::Cross => {
            let half = snap(s.thickness.max(1.0 / ppp) / 2.0);
            let (inner, outer) = (snap(s.gap / 2.0), snap(s.size / 2.0));
            if outer > inner {
                for (x, y) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    let near = c + vec2(x, y) * inner;
                    let far = c + vec2(x, y) * outer;
                    let across = vec2(y, x).abs() * half;
                    out.push(Mark::Rect(Rect::from_two_pos(near - across, far + across)));
                }
            }
            if s.center_dot {
                dot(&mut out);
            }
        }
        Shape::Circle => {
            let width = s.thickness.max(1.0 / ppp);
            out.push(Mark::Ring {
                radius: ((s.size - width) / 2.0).max(width / 2.0),
                width,
            });
            if s.center_dot {
                dot(&mut out);
            }
        }
        Shape::Legacy => {
            // drawn at 8 points by the Legacy Viewer; whole pixels per cell
            let cell = snap(s.size / 8.0).max(1.0 / ppp);
            let corner = c - Vec2::splat(cell * 4.0);
            for (row, line) in LEGACY.iter().enumerate() {
                for (col, px) in line.iter().enumerate() {
                    let rect = Rect::from_min_size(corner + vec2(col as f32, row as f32) * cell, Vec2::splat(cell));
                    match px {
                        b'#' => out.push(Mark::Rect(rect)),
                        b'o' => out.push(Mark::Border(rect)),
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// Draws the crosshair of `s` centred on `c`. `picture`: the loaded picture
/// of the `Image` shape (a dot stands in while it loads or if it cannot be
/// read).
pub fn draw(painter: &Painter, c: Pos2, s: &Settings, picture: Option<&TextureHandle>) {
    if !s.show {
        return;
    }
    let ppp = painter.ctx().pixels_per_point();
    if s.shape == Shape::Image
        && let Some(tex) = picture
    {
        let size = tex.size_vec2();
        let scale = s.size / size.x.max(size.y).max(1.0);
        let rect = Rect::from_center_size(c, size * scale);
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        let tint = crate::ui::widgets::PICTURE_TINT.gamma_multiply(f32::from(s.color[3]) / 255.0);
        painter.image(tex.id(), rect, uv, tint);
        return;
    }
    let fill = crate::ui::colors::c(s.color);
    // theme-guard: content — the dark border that keeps the crosshair readable over a bright scene, as crosshairs.tga's
    let dark = Color32::BLACK.gamma_multiply(f32::from(s.color[3]) / 255.0);
    let marks = marks(c, s, ppp);
    let c = pos2((c.x * ppp).round() / ppp, (c.y * ppp).round() / ppp);
    // every border first, so that none covers a neighbouring fill
    let borders = marks.iter().filter(|_| s.outline).map(|mark| match *mark {
        Mark::Rect(r) => egui::Shape::rect_filled(r.expand(OUTLINE), 0.0, dark),
        Mark::Disc { radius } => egui::Shape::circle_filled(c, radius + OUTLINE, dark),
        Mark::Ring { radius, width } => egui::Shape::circle_stroke(c, radius, Stroke::new(width + 2.0 * OUTLINE, dark)),
        Mark::Border(r) => egui::Shape::rect_filled(r, 0.0, dark),
    });
    let fills = marks.iter().filter_map(|mark| match *mark {
        Mark::Rect(r) => Some(egui::Shape::rect_filled(r, 0.0, fill)),
        Mark::Disc { radius } => Some(egui::Shape::circle_filled(c, radius, fill)),
        Mark::Ring { radius, width } => Some(egui::Shape::circle_stroke(c, radius, Stroke::new(width, fill))),
        Mark::Border(_) => None,
    });
    painter.extend(borders.chain(fills).collect::<Vec<_>>());
}

/// The preview of the preferences: the crosshair over a patch of dark and
/// bright, to judge its outline on both.
pub fn preview(ui: &mut egui::Ui, p: &Palette, s: &Settings) {
    let side = s.size.max(s.dot_size).clamp(48.0, 96.0) + 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(side * 2.0, side), egui::Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    let (left, right) = rect.split_left_right_at_fraction(0.5);
    let left_radius = CornerRadius { ne: 0, se: 0, ..radius };
    let right_radius = CornerRadius { nw: 0, sw: 0, ..radius };
    ui.painter().rect_filled(left, left_radius, p.navy);
    ui.painter().rect_filled(right, right_radius, p.violet_pale);
    let picture = picture(ui.ctx(), s);
    let shown = Settings { show: true, ..s.clone() };
    let painter = ui.painter_at(rect);
    draw(&painter, left.center(), &shown, picture.as_ref());
    draw(&painter, right.center(), &shown, picture.as_ref());
}

/// The picture of the `Image` shape, read on a background thread the first
/// time it is asked for (and again when the path changes), kept in the
/// context's memory. None while it loads, without a picture, or when it
/// cannot be read (logged once).
pub fn picture(ctx: &egui::Context, s: &Settings) -> Option<TextureHandle> {
    if s.shape != Shape::Image {
        return None;
    }
    let path = s.image.as_deref()?;
    let id = egui::Id::unique("crosshair-picture");
    let cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<Picture>>>(id).clone());
    let mut cache = cache.lock().ok()?;
    cache.get(ctx, path)
}

/// Biggest side kept of a picture: a crosshair needs no more, and a huge
/// file chosen by mistake costs nothing.
const PICTURE_MAX: u32 = 256;

#[derive(Default)]
struct Picture {
    path: Option<PathBuf>,
    texture: Option<TextureHandle>,
    loading: Option<mpsc::Receiver<Option<egui::ColorImage>>>,
}

impl Picture {
    fn get(&mut self, ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
        if self.path.as_deref() != Some(path) {
            self.path = Some(path.to_owned());
            self.texture = None;
            let (tx, rx) = mpsc::channel();
            self.loading = Some(rx);
            let (owned, repaint) = (path.to_owned(), ctx.clone());
            let spawned = std::thread::Builder::new().name("aurora-crosshair-picture".into()).spawn(move || {
                let _ = tx.send(read_picture(&owned));
                repaint.request_repaint();
            });
            if let Err(e) = spawned {
                log::warn!("crosshair picture: cannot start its reading: {e}");
                self.loading = None;
            }
        }
        if let Some(rx) = &self.loading {
            match rx.try_recv() {
                Ok(image) => {
                    self.loading = None;
                    self.texture = image.map(|image| ctx.load_texture("crosshair-picture", image, egui::TextureOptions::LINEAR));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.loading = None,
            }
        }
        self.texture.clone()
    }
}

fn read_picture(path: &Path) -> Option<egui::ColorImage> {
    let image = match image::open(path) {
        Ok(image) => image,
        Err(e) => {
            log::warn!("crosshair picture {}: {e}", path.display());
            return None;
        }
    };
    let image = if image.width().max(image.height()) > PICTURE_MAX {
        image.thumbnail(PICTURE_MAX, PICTURE_MAX)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// AURORA_DEMO_CROSSHAIR=dot|cross|circle|legacy|off|image:<file>: the
/// crosshair of that shape for the session (nothing saved); `cross` and
/// `circle` with their centre dot.
pub fn demo(s: &Settings) -> Option<Settings> {
    let spec = std::env::var("AURORA_DEMO_CROSSHAIR").ok()?;
    let mut shown = s.clone();
    match spec.as_str() {
        "dot" => shown.shape = Shape::Dot,
        "cross" | "circle" => {
            shown.shape = if spec == "cross" { Shape::Cross } else { Shape::Circle };
            shown.center_dot = true;
        }
        "legacy" => shown.shape = Shape::Legacy,
        "off" => shown.show = false,
        _ => {
            let file = spec.strip_prefix("image:")?;
            shown.shape = Shape::Image;
            shown.image = Some(PathBuf::from(file));
        }
    }
    Some(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(marks: &[Mark]) -> Vec<Rect> {
        marks
            .iter()
            .filter_map(|m| match m {
                Mark::Rect(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_default_is_a_small_dot() {
        let s = Settings::default();
        assert!(s.show);
        assert_eq!(marks(pos2(100.0, 100.0), &s, 1.0), vec![Mark::Disc { radius: 2.0 }]);
    }

    #[test]
    fn a_cross_leaves_its_gap_and_reaches_its_size() {
        let s = Settings {
            shape: Shape::Cross,
            size: 20.0,
            gap: 6.0,
            thickness: 2.0,
            ..Settings::default()
        };
        let arms = rects(&marks(pos2(100.0, 100.0), &s, 1.0));
        assert_eq!(arms.len(), 4);
        let bounds = arms.iter().fold(Rect::NOTHING, |b, r| b.union(*r));
        assert_eq!(bounds, Rect::from_center_size(pos2(100.0, 100.0), Vec2::splat(20.0)));
        for arm in &arms {
            assert!(!arm.contains(pos2(100.0, 100.0)));
            assert_eq!(arm.width().min(arm.height()), 2.0);
            assert_eq!(arm.width().max(arm.height()), 7.0);
        }
    }

    #[test]
    fn a_cross_without_room_outside_its_gap_is_only_its_dot() {
        let s = Settings {
            shape: Shape::Cross,
            size: 6.0,
            gap: 8.0,
            center_dot: true,
            ..Settings::default()
        };
        assert_eq!(marks(pos2(10.0, 10.0), &s, 1.0), vec![Mark::Disc { radius: 2.0 }]);
    }

    #[test]
    fn edges_fall_on_whole_pixels() {
        let s = Settings {
            shape: Shape::Cross,
            size: 15.0,
            gap: 5.0,
            thickness: 3.0,
            ..Settings::default()
        };
        let ppp = 1.5;
        for r in rects(&marks(pos2(100.3, 50.7), &s, ppp)) {
            for v in [r.min.x, r.min.y, r.max.x, r.max.y] {
                assert!(((v * ppp) - (v * ppp).round()).abs() < 1e-4, "{v}");
            }
        }
    }

    #[test]
    fn the_legacy_corners_are_crosshairs_tga() {
        let s = Settings {
            shape: Shape::Legacy,
            size: 8.0,
            ..Settings::default()
        };
        let m = marks(pos2(4.0, 4.0), &s, 1.0);
        let white = m.iter().filter(|m| matches!(m, Mark::Rect(_))).count();
        let border = m.iter().filter(|m| matches!(m, Mark::Border(_))).count();
        assert_eq!((white, border), (12, 16));
        // the top left corner of the 8×8 square, at the Legacy size
        assert!(m.contains(&Mark::Rect(Rect::from_min_size(pos2(1.0, 1.0), Vec2::splat(1.0)))));
    }

    #[test]
    fn a_circle_fits_its_size() {
        let s = Settings {
            shape: Shape::Circle,
            size: 20.0,
            thickness: 2.0,
            ..Settings::default()
        };
        assert_eq!(marks(pos2(0.0, 0.0), &s, 1.0), vec![Mark::Ring { radius: 9.0, width: 2.0 }]);
    }

    #[test]
    fn older_settings_read_with_the_defaults() {
        let s: Settings = serde_json::from_str(r#"{"shape":"cross"}"#).expect("valid settings");
        assert_eq!(s.shape, Shape::Cross);
        assert!(s.show);
        assert_eq!(s.dot_size, Settings::default().dot_size);
    }
}
