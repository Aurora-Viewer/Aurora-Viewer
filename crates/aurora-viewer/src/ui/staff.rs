//! The staff's tag after their name in the windows (`world::staff`): the
//! Aurora team's "Aurora Team" and Linden Lab's "Linden Lab", in the
//! People lists, the chat and the profile, with the light of their mark
//! over the profile's banner. The plate's tag (`nameplate/mark.rs`) with
//! the windows' corners: the plates' night tint, the mark's moving border,
//! its icon and its letters.

use crate::theme::Palette;
use crate::ui::nameplate::{aurora, mark::Look};
use crate::world::staff::Mark;
use egui::text::LayoutJob;
use egui::{Galley, Mesh, Pos2, Rect, Shape, Vec2};
use std::sync::Arc;

/// The tag laid out for a text `font` points high.
pub struct Tag {
    look: Look,
    label: Arc<Galley>,
    font: f32,
    t: f32,
    size: Vec2,
}

/// The time of the light in `ui`, held still with "Reduce animations"
/// (which ends egui's transitions, `theme::apply_motion`); asks for the
/// next frame while it moves.
fn time(ui: &egui::Ui) -> f32 {
    let still = ui.style().animation_time <= 0.0;
    if !still {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    aurora::time(ui.input(|i| i.time), still)
}

/// The light of a staff member's mark over their profile's banner, inside
/// it: the team's curtains rising from its bottom edge, Linden Lab's stars.
pub fn banner_light(ui: &egui::Ui, p: &Palette, banner: Rect, mark: Option<Mark>) {
    let Some(mark) = mark else { return };
    let mut mesh = Mesh::default();
    Look::new(mark).banner(&mut mesh, p, banner, ui.painter().pixels_per_point(), time(ui));
    ui.painter().with_clip_rect(banner.intersect(ui.clip_rect())).add(Shape::mesh(mesh));
}

impl Tag {
    pub fn new(ui: &egui::Ui, p: &Palette, mark: Mark, font: f32) -> Tag {
        let t = time(ui);
        let look = Look::new(mark);
        let text = look.label();
        let n = text.chars().count();
        let semibold = crate::ui::fonts::semibold(ui.ctx());
        let mut job = LayoutJob {
            break_on_newline: false,
            ..Default::default()
        };
        for (i, ch) in text.chars().enumerate() {
            let format = crate::ui::fonts::semibold_format(&semibold, font, look.letter(p, i, n, t));
            job.append(ch.encode_utf8(&mut [0; 4]), 0.0, format);
        }
        let label = ui.painter().layout_job(job);
        let size = Vec2::new(
            (2.0 * Self::pad(font) + font + Self::gap(font) + label.size().x).ceil(),
            (label.size().y + 2.0 * (font * 0.12).round()).ceil(),
        );
        Tag {
            look,
            label,
            font,
            t,
            size,
        }
    }

    fn pad(font: f32) -> f32 {
        (font * 0.5).round()
    }

    fn gap(font: f32) -> f32 {
        (font * 0.3).round()
    }

    pub fn size(&self) -> Vec2 {
        self.size
    }

    pub fn paint(&self, ui: &egui::Ui, painter: &egui::Painter, p: &Palette, rect: Rect) {
        let ppp = painter.pixels_per_point();
        let radius = f32::from(ui.visuals().window_corner_radius.nw);
        let fill = p.navy.lerp_to_gamma(p.bar, 0.5).gamma_multiply(0.95);
        let mut mesh = Mesh::default();
        self.look.tag(&mut mesh, p, rect, radius, ppp, self.t, fill, 1.0, 1.0);
        painter.add(Shape::mesh(mesh));
        let pad = Self::pad(self.font);
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + pad + self.font * 0.5, rect.center().y),
            Vec2::splat(self.font),
        );
        if let Some(icon) = crate::ui::icons::global(self.look.icon()) {
            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(icon.id(), icon_rect, whole, self.look.icon_color(p, self.t));
        }
        let at = Pos2::new(
            icon_rect.right() + Self::gap(self.font),
            rect.center().y - self.label.size().y * 0.5,
        );
        painter.galley(at, self.label.clone(), p.ink);
    }
}

/// Size of a tag text for a line whose text is `font` points high.
pub fn font_for(font: f32) -> f32 {
    (font * 0.74).max(8.0)
}

/// The tag of `mark` after a name in a layout, its text `font` points
/// high, with what it means under the pointer.
pub fn after_name(ui: &mut egui::Ui, p: &Palette, mark: Option<Mark>, font: f32) -> Option<egui::Response> {
    let tag = Tag::new(ui, p, mark?, font);
    let (rect, response) = ui.allocate_exact_size(tag.size(), egui::Sense::hover());
    tag.paint(ui, ui.painter(), p, rect);
    Some(response.on_hover_text(tag.look.tooltip()))
}

/// The tag of `mark` painted from `left_center` by a row that places its
/// own text (`font` the size of its name): its rectangle.
pub fn paint_at(ui: &egui::Ui, painter: &egui::Painter, p: &Palette, mark: Option<Mark>, left_center: Pos2, font: f32) -> Option<Rect> {
    let tag = Tag::new(ui, p, mark?, font_for(font));
    let ppp = painter.pixels_per_point();
    let min = left_center - Vec2::new(0.0, tag.size().y * 0.5);
    let rect = Rect::from_min_size(Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp), tag.size());
    tag.paint(ui, painter, p, rect);
    Some(rect)
}

/// Width `paint_at` takes for `mark` (0 without one), for a row that
/// elides the name before it.
pub fn width(ui: &egui::Ui, p: &Palette, mark: Option<Mark>, font: f32) -> f32 {
    mark.map_or(0.0, |mark| Tag::new(ui, p, mark, font_for(font)).size().x)
}
