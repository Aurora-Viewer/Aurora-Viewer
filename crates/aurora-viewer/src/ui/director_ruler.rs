//! The Director's timeline strip, for long tracks and at a glance: the
//! seconds the ruler shows (its zoom: Ctrl + wheel, the strip's zoom
//! buttons; a wheel pans), and the thumbnails above the keyframes
//! (`director_thumbs`). Aurora's own; drawn by `ui::director::ruler`, whose
//! clicks, drags and double clicks stay as they are.

use super::widgets;
use crate::camera::track::Keyframe;
use crate::director_thumbs::PoseKey;
use crate::theme::Palette;
use egui::{Id, Pos2, Rect, Sense, Stroke, TextureId, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// The ruler never shows fewer seconds than this.
pub const MIN_VIEW: f32 = 2.0;
/// How much a press of a zoom button zooms.
const STEP: f32 = 1.6;
/// The row of thumbnails above the ruler, and a thumbnail's height in it.
pub const THUMB_ROW: f32 = 32.0;
const THUMB_H: f32 = 26.0;
/// The zoom buttons' room after the ruler.
pub const BUTTONS_W: f32 = 3.0 * 20.0 + 2.0 * 4.0;

/// The seconds the ruler shows: from `start`, `span` long.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub start: f32,
    pub span: f32,
}

impl View {
    /// The whole ruler (`full`: the seconds it shows unzoomed).
    pub fn fit(full: f32) -> View {
        View {
            start: 0.0,
            span: full.max(MIN_VIEW),
        }
    }

    /// Within the ruler: never wider than it, nor before its start.
    pub fn clamped(self, full: f32) -> View {
        let full = full.max(MIN_VIEW);
        let span = if self.span.is_finite() {
            self.span.clamp(MIN_VIEW, full)
        } else {
            full
        };
        let start = if self.start.is_finite() {
            self.start.clamp(0.0, full - span)
        } else {
            0.0
        };
        View { start, span }
    }

    pub fn zoomed(&self, full: f32) -> bool {
        self.span < full.max(MIN_VIEW) - 1e-3
    }

    /// Zoomed by `factor` (above 1: closer) around the second `about`, which
    /// stays where it is under the pointer.
    pub fn zoom(self, factor: f32, about: f32, full: f32) -> View {
        if !factor.is_finite() || factor <= 0.0 {
            return self.clamped(full);
        }
        let span = (self.span / factor).clamp(MIN_VIEW, full.max(MIN_VIEW));
        let share = if self.span > 0.0 {
            ((about - self.start) / self.span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        View {
            start: about - share * span,
            span,
        }
        .clamped(full)
    }

    /// Moved by `seconds` (later when positive).
    pub fn pan(self, seconds: f32, full: f32) -> View {
        View {
            start: self.start + if seconds.is_finite() { seconds } else { 0.0 },
            ..self
        }
        .clamped(full)
    }

    /// The playhead kept in sight: a view it left follows it.
    pub fn follow(self, time: f32, full: f32) -> View {
        if !time.is_finite() || (self.start..=self.start + self.span).contains(&time) {
            return self.clamped(full);
        }
        let start = if time < self.start {
            time - self.span * 0.2
        } else {
            time - self.span * 0.8
        };
        View { start, ..self }.clamped(full)
    }

    pub fn contains(&self, time: f32) -> bool {
        time >= self.start - 1e-4 && time <= self.start + self.span + 1e-4
    }

    /// Points a second, on a lane `width` points wide.
    pub fn scale(&self, width: f32) -> f32 {
        width.max(1.0) / self.span.max(1e-3)
    }

    /// Where `time` is on a lane from `left`, `width` wide (a time out of
    /// the view is at its edge).
    pub fn x_of(&self, time: f32, left: f32, width: f32) -> f32 {
        left + (time - self.start).clamp(0.0, self.span) * self.scale(width)
    }

    /// The second under `x`, within the view.
    pub fn time_at(&self, x: f32, left: f32, width: f32) -> f32 {
        (self.start + (x - left) / self.scale(width)).clamp(self.start, self.start + self.span)
    }
}

fn view_id() -> Id {
    Id::unique("director_ruler_view")
}

/// The view the ruler had, kept from frame to frame (`None`: the whole of it).
fn stored(ctx: &egui::Context) -> Option<View> {
    ctx.data(|data| data.get_temp::<View>(view_id()))
}

fn store(ctx: &egui::Context, view: View, full: f32) {
    ctx.data_mut(|data| {
        if view.zoomed(full) {
            data.insert_temp(view_id(), view);
        } else {
            data.remove::<View>(view_id());
        }
    });
}

/// This frame's view of a ruler of `full` seconds at `rect` (its lane from
/// `left`, `width` wide): the wheel over it zooms (with Ctrl) or pans, and
/// unless the pointer holds it, the view follows the playhead `time`.
pub fn view(ui: &egui::Ui, rect: Rect, left: f32, width: f32, full: f32, time: f32, held: bool) -> View {
    let ctx = ui.ctx();
    let mut view = stored(ctx).unwrap_or(View::fit(full)).clamped(full);
    if ui.rect_contains_pointer(rect)
        && let Some(pointer) = ctx.pointer_hover_pos()
    {
        let zoom = ui.input(|input| input.zoom_delta());
        if (zoom - 1.0).abs() > 1e-4 {
            view = view.zoom(zoom, view.time_at(pointer.x, left, width), full);
        }
        let wheel = ui.input(|input| input.smooth_scroll_delta);
        // (a vertical wheel pans too: nothing else scrolls on the strip)
        let pan = if wheel.x != 0.0 { wheel.x } else { wheel.y };
        if pan != 0.0 && view.zoomed(full) {
            ui.input_mut(|input| input.smooth_scroll_delta = Vec2::ZERO);
            view = view.pan(-pan / view.scale(width), full);
        }
    }
    if !held {
        view = view.follow(time, full);
    }
    store(ctx, view, full);
    view
}

/// The zoom buttons after the ruler: closer, farther, the whole track.
pub fn zoom_buttons(ui: &mut egui::Ui, p: &Palette, full: f32, time: f32) {
    let ctx = ui.ctx().clone();
    let view = stored(&ctx).unwrap_or(View::fit(full)).clamped(full);
    let about = if view.contains(time) { time } else { view.start + view.span / 2.0 };
    ui.spacing_mut().item_spacing.x = 4.0;
    let closer = view.span > MIN_VIEW + 1e-3;
    let tip = aurora_i18n::tr!("director-zoom-in-tip");
    if widgets::icon_button(ui, p, "zoom-in-filled", tip, closer) {
        store(&ctx, view.zoom(STEP, about, full), full);
    }
    let zoomed = view.zoomed(full);
    if widgets::icon_button(ui, p, "zoom-out-filled", aurora_i18n::tr!("director-zoom-out-tip"), zoomed) {
        store(&ctx, view.zoom(1.0 / STEP, about, full), full);
    }
    if widgets::icon_button(ui, p, "arrows-horizontal", aurora_i18n::tr!("director-zoom-fit-tip"), zoomed) {
        store(&ctx, View::fit(full), full);
    }
}

/// The thumbnails the app has, by keyframe view: published each frame
/// before the strip is drawn.
#[derive(Clone, Default)]
pub struct Pictures {
    pub by_view: HashMap<PoseKey, (TextureId, Vec2)>,
}

fn pictures_id() -> Id {
    Id::unique("director_thumbnails")
}

pub fn publish(ctx: &egui::Context, pictures: Arc<Pictures>) {
    ctx.data_mut(|data| data.insert_temp(pictures_id(), pictures));
}

pub fn published(ctx: &egui::Context) -> Arc<Pictures> {
    ctx.data(|data| data.get_temp::<Arc<Pictures>>(pictures_id())).unwrap_or_default()
}

/// The size a thumbnail of `size` takes in the row.
fn shown(size: Vec2) -> Vec2 {
    let aspect = if size.y > 0.0 {
        (size.x / size.y).clamp(0.4, 3.0)
    } else {
        16.0 / 9.0
    };
    Vec2::new(THUMB_H * aspect, THUMB_H)
}

/// The row of thumbnails (`band`, above the ruler): each over its keyframe
/// (`x_of`), the selected one on top, a placeholder where there is none.
/// A click on one selects its keyframe, a double click goes there: the
/// clicked and double-clicked keyframes. Never over the keyframe markers,
/// which are in the lane below.
pub fn thumbnails(
    ui: &mut egui::Ui,
    p: &Palette,
    band: Rect,
    keys: &[Keyframe],
    selected: Option<usize>,
    visible: impl Fn(f32) -> bool,
    x_of: impl Fn(f32) -> f32,
) -> (Option<usize>, Option<usize>) {
    let pictures = published(ui.ctx());
    let painter = ui.painter_at(band);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let (mut clicked, mut double) = (None, None);
    let order = (0..keys.len())
        .filter(|i| Some(*i) != selected)
        .chain(selected.filter(|i| *i < keys.len()));
    for index in order {
        let key = &keys[index];
        if !visible(key.time) {
            continue;
        }
        let picture = pictures.by_view.get(&PoseKey::of(&key.pose));
        let size = shown(picture.map_or(Vec2::new(16.0, 9.0), |(_, size)| *size));
        let center = Pos2::new(x_of(key.time), band.center().y);
        let rect = Rect::from_center_size(center, size);
        let response = ui.interact(rect, ui.scope_id().with(("director_thumb", index)), Sense::click());
        if response.double_clicked() {
            double = Some(index);
        } else if response.clicked() {
            clicked = Some(index);
        }
        match picture {
            Some((texture, _)) => {
                painter.image(
                    *texture,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    // theme-guard: content — a picture of the view, in its own colours
                    egui::Color32::WHITE,
                );
            }
            None => {
                // the placeholder: no picture of this view yet
                painter.rect_filled(rect, radius, p.field);
                if let Some(icon) = super::icons::global("photo-filled") {
                    painter.image(
                        icon.id(),
                        Rect::from_center_size(rect.center(), Vec2::splat(THUMB_H * 0.5)),
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        p.muted_dim,
                    );
                }
            }
        }
        let outline = if selected == Some(index) {
            Stroke::new(2.0, p.violet_light)
        } else if response.hovered() {
            Stroke::new(1.0, p.ink)
        } else {
            Stroke::new(1.0, p.muted_dim)
        };
        painter.rect_stroke(rect, radius, outline, egui::StrokeKind::Inside);
        response.on_hover_text(aurora_i18n::tr!("director-thumbnail-tip"));
    }
    (clicked, double)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_ruler_maps_as_before_zoom() {
        let view = View::fit(12.0);
        assert!(!view.zoomed(12.0));
        // 12 s across 600 points: 50 points a second
        assert_eq!(view.x_of(3.0, 10.0, 600.0), 160.0);
        assert_eq!(view.time_at(160.0, 10.0, 600.0), 3.0);
        assert_eq!(view.time_at(-50.0, 10.0, 600.0), 0.0);
        assert_eq!(view.time_at(9_999.0, 10.0, 600.0), 12.0);
    }

    #[test]
    fn a_zoom_keeps_the_second_under_the_pointer() {
        let full = 600.0;
        let view = View::fit(full).zoom(4.0, 300.0, full);
        assert_eq!(view.span, 150.0);
        // 300 s was in the middle: it still is
        assert!((view.x_of(300.0, 0.0, 1000.0) - 500.0).abs() < 1e-3);
        assert!(view.zoomed(full));
        // never closer than MIN_VIEW, never farther than the whole
        assert_eq!(view.zoom(1e6, 10.0, full).span, MIN_VIEW);
        assert_eq!(view.zoom(1e-6, 10.0, full), View::fit(full));
        assert_eq!(view.zoom(f32::NAN, 1.0, full), view);
        // round trip: x and time agree within the view
        let time = view.time_at(123.0, 0.0, 1000.0);
        assert!((view.x_of(time, 0.0, 1000.0) - 123.0).abs() < 1e-3);
    }

    #[test]
    fn a_pan_stays_within_the_ruler() {
        let full = 100.0;
        let view = View { start: 10.0, span: 20.0 };
        assert_eq!(view.pan(5.0, full).start, 15.0);
        assert_eq!(view.pan(-50.0, full).start, 0.0);
        assert_eq!(view.pan(500.0, full).start, 80.0);
        // a track that got shorter brings the view in
        assert_eq!(View { start: 90.0, span: 20.0 }.clamped(50.0).start, 30.0);
    }

    #[test]
    fn the_playhead_stays_in_sight() {
        let full = 100.0;
        let view = View { start: 10.0, span: 20.0 };
        assert_eq!(view.follow(15.0, full), view);
        let later = view.follow(60.0, full);
        assert!(later.contains(60.0) && later.start > 10.0);
        let earlier = view.follow(2.0, full);
        assert!(earlier.contains(2.0));
        assert_eq!(view.follow(f32::NAN, full), view);
    }
}
