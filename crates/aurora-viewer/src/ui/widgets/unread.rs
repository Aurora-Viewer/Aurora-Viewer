//! The unread counter, one look wherever the interface counts what waits:
//! the islands' bell and chat button, the dock's Conversations button,
//! Firestorm's bars (their bell and Conversations button) and the entries of
//! the Conversations list. The theme's dark red at its radius (the human's
//! choice, it was the accent; white reads on it at 5.3:1, on the light red
//! at 2.8:1 only), the number in `on_accent`, "99+" beyond 99, its digits'
//! ink in the middle of the pill to the pixel. Grey while do not disturb
//! pauses the bell.

use crate::theme::Palette;
use egui::{Align, Align2, Color32, FontId, Pos2, Rect, Vec2};
use std::sync::Arc;

/// The two sizes of the counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountSize {
    /// On a button of a bar: 13 px high, 9.5 px digits.
    Bar,
    /// At the end of an entry of the Conversations list: 15 px high, 10.5 px
    /// digits, beside its 12 px names.
    List,
}

impl CountSize {
    /// The digits' size, the pill's height and the room beside the digits.
    fn metrics(self) -> (f32, f32, f32) {
        match self {
            CountSize::Bar => (9.5, 13.0, 7.0),
            CountSize::List => (10.5, 15.0, 8.0),
        }
    }
}

/// The text of a count: the number, "99+" beyond 99.
pub fn count_text(n: usize) -> String {
    if n > 99 { "99+".to_owned() } else { n.to_string() }
}

/// An unread count to draw: `UnreadCount::new(n, size)`, then `scale` (the
/// bell's bounce) and `paused` (do not disturb) if needed.
#[derive(Clone, Copy, Debug)]
pub struct UnreadCount {
    n: usize,
    size: CountSize,
    scale: f32,
    paused: bool,
}

impl UnreadCount {
    pub fn new(n: usize, size: CountSize) -> Self {
        Self {
            n,
            size,
            scale: 1.0,
            paused: false,
        }
    }

    /// Drawn `scale` times bigger (the bell's bounce).
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Grey, its number muted: the bell paused by do not disturb.
    pub fn paused(mut self, paused: bool) -> Self {
        self.paused = paused;
        self
    }

    /// Laid out with its `align` point on `anchor` (`Align2::RIGHT_TOP`: its
    /// top right corner there), on whole pixels: where it stands, before it
    /// is painted (an entry stops its name before it).
    pub fn place(&self, ctx: &egui::Context, p: &Palette, anchor: Pos2, align: Align2) -> PlacedCount {
        let (fill, ink) = if self.paused {
            (p.raised, p.muted)
        } else {
            (p.danger_dim, p.on_accent)
        };
        let (font, height, pad) = self.size.metrics();
        let font = FontId::new(font * self.scale, crate::ui::fonts::semibold(ctx));
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(count_text(self.n), font, ink));
        let (rect, at) = layout(
            &galley,
            anchor,
            align,
            height * self.scale,
            pad * self.scale,
            ctx.pixels_per_point(),
        );
        PlacedCount {
            rect,
            galley,
            at,
            fill,
            ink,
        }
    }

    /// Placed (see `place`) and painted; returns its rectangle.
    pub fn paint(&self, painter: &egui::Painter, p: &Palette, anchor: Pos2, align: Align2) -> Rect {
        let placed = self.place(painter.ctx(), p, anchor, align);
        let rect = placed.rect;
        placed.paint(painter);
        rect
    }
}

/// An unread count of `size` pinned by its top right corner on `anchor`;
/// returns its rectangle.
pub fn unread_count(painter: &egui::Painter, p: &Palette, anchor: Pos2, n: usize, size: CountSize) -> Rect {
    UnreadCount::new(n, size).paint(painter, p, anchor, Align2::RIGHT_TOP)
}

/// A count laid out: its pill (`rect`) and its number.
pub struct PlacedCount {
    pub rect: Rect,
    galley: Arc<egui::Galley>,
    at: Pos2,
    fill: Color32,
    ink: Color32,
}

impl PlacedCount {
    pub fn paint(self, painter: &egui::Painter) {
        let radius = painter.ctx().global_style().visuals.window_corner_radius;
        painter.rect_filled(self.rect, radius, self.fill);
        painter.galley(self.at, self.galley, self.ink);
    }
}

/// The pill and where its number goes: the digits' ink exactly in the
/// middle, to the pixel. Centring the ink on the pill's centre left it half
/// a pixel off whenever the pill and the digits differ by an odd number of
/// pixels (a "2" sat low and to the right): the pill grows by a pixel to
/// make both margins whole, and stands on whole pixels.
fn layout(galley: &egui::Galley, anchor: Pos2, align: Align2, height: f32, pad: f32, ppp: f32) -> (Rect, Pos2) {
    let ink = super::ink_rect(galley);
    // in physical pixels
    let px = |v: f32| (v * ppp).round();
    let (iw, ih) = (px(ink.width()), px(ink.height()));
    let even = |outer: f32, inner: f32| if (outer - inner) as i32 % 2 == 0 { outer } else { outer + 1.0 };
    let h = even(px(height), ih);
    let w = even(px(galley.size().x + pad).max(h), iw);
    let start = |at: f32, side: f32, align: Align| match align {
        Align::Min => px(at),
        Align::Center => (at * ppp - side / 2.0).round(),
        Align::Max => px(at) - side,
    };
    let (left, top) = (start(anchor.x, w, align.x()), start(anchor.y, h, align.y()));
    let pill = Rect::from_min_size(Pos2::new(left, top) / ppp, Vec2::new(w, h) / ppp);
    // the ink's top left corner on whole pixels, the galley's origin before it
    let ink_at = Pos2::new(left + (w - iw) / 2.0, top + (h - ih) / 2.0) / ppp;
    (pill, ink_at - ink.min.to_vec2())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counter's number stands in the middle of its pill, to the pixel,
    /// whatever the number's width, the screen's scale, the size and where
    /// the pill is pinned; the pill stands on whole pixels.
    #[test]
    fn the_counter_number_is_centred_to_the_pixel() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut checked = 0;
        for ppp in [1.0f32, 1.25, 1.5, 2.0] {
            ctx.set_pixels_per_point(ppp);
            let mut out = ctx.run_ui(Default::default(), |_| {
                for size in [CountSize::Bar, CountSize::List] {
                    for (n, text) in [(1, "1"), (2, "2"), (7, "7"), (12, "12"), (250, "99+")] {
                        for scale in [1.0, 1.3] {
                            for align in [Align2::RIGHT_TOP, Align2::RIGHT_CENTER, Align2::LEFT_TOP] {
                                let c = UnreadCount::new(n, size).scale(scale).place(&ctx, &p, Pos2::new(128.3, 7.6), align);
                                let case = format!("{text} {size:?} ×{scale} {align:?} at {ppp}");
                                assert_eq!(c.galley.text(), text, "{case}");
                                let ink = crate::ui::widgets::ink_rect(&c.galley).translate(c.at.to_vec2());
                                let margin = |a: f32, b: f32| ((b - a) * ppp * 100.0).round() / 100.0;
                                let (l, r) = (margin(c.rect.left(), ink.left()), margin(ink.right(), c.rect.right()));
                                let (t, b) = (margin(c.rect.top(), ink.top()), margin(ink.bottom(), c.rect.bottom()));
                                assert!((l - r).abs() <= 0.51 && (t - b).abs() <= 0.51, "{case}: {l} {r} {t} {b}");
                                for edge in [c.rect.left(), c.rect.top(), c.rect.right(), c.rect.bottom()] {
                                    assert_eq!(((edge * ppp) * 1000.0).round() % 1000.0, 0.0, "{case}: the pill on whole pixels");
                                }
                                assert!(
                                    (c.rect.width() * ppp).round() >= (c.rect.height() * ppp).round(),
                                    "{case}: never narrower than high"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            });
            out.textures_delta.clear();
        }
        assert_eq!(checked, 4 * 2 * 5 * 2 * 3);
    }

    /// Pinned by its top right corner, the pill ends on the anchor (to the
    /// pixel) and grows to the left; the list's counter is the taller one.
    #[test]
    fn the_counter_is_pinned_where_asked() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut out = ctx.run_ui(Default::default(), |_| {
            let at = Pos2::new(200.0, 40.0);
            let one = UnreadCount::new(3, CountSize::Bar).place(&ctx, &p, at, Align2::RIGHT_TOP);
            let many = UnreadCount::new(250, CountSize::Bar).place(&ctx, &p, at, Align2::RIGHT_TOP);
            assert_eq!(one.rect.right_top(), at);
            assert_eq!(many.rect.right_top(), at);
            assert!(many.rect.width() > one.rect.width());
            let list = UnreadCount::new(3, CountSize::List).place(&ctx, &p, at, Align2::RIGHT_CENTER);
            assert!(list.rect.height() > one.rect.height());
            assert!((list.rect.center().y - at.y).abs() <= 0.5);
            // do not disturb greys it
            let paused = UnreadCount::new(3, CountSize::Bar)
                .paused(true)
                .place(&ctx, &p, at, Align2::RIGHT_TOP);
            assert_eq!((one.fill, one.ink), (p.danger_dim, p.on_accent));
            assert_eq!((paused.fill, paused.ink), (p.raised, p.muted));
        });
        out.textures_delta.clear();
    }
}
