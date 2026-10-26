//! The "Aurora Staff" tag after the name of a member of the Aurora team
//! (`world::staff`) in the windows: the People lists, the chat and the
//! profile. The plate's tag (`nameplate/aurora.rs`) with the windows'
//! corners: the plates' night tint, a border flowing along the borealis,
//! the sparkles icon and the letters from green to violet.

use crate::theme::Palette;
use crate::ui::nameplate::aurora;
use egui::text::LayoutJob;
use egui::{Galley, Mesh, Pos2, Rect, Shape, Vec2};
use std::sync::Arc;
use uuid::Uuid;

/// The tag laid out for a text `font` points high.
pub struct Tag {
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

/// The northern light over a team member's profile banner: the plate's
/// curtains rising from its bottom edge, inside it.
pub fn banner_light(ui: &egui::Ui, p: &Palette, banner: Rect) {
    let mut mesh = Mesh::default();
    let foot = Rect::from_min_size(Pos2::new(banner.left(), banner.bottom() - 1.0), banner.size());
    aurora::curtains(&mut mesh, p, foot, ui.painter().pixels_per_point(), time(ui), 1.0);
    ui.painter().with_clip_rect(banner.intersect(ui.clip_rect())).add(Shape::mesh(mesh));
}

impl Tag {
    pub fn new(ui: &egui::Ui, p: &Palette, font: f32) -> Tag {
        let t = time(ui);
        let text = aurora_i18n::tr!("nameplate-aurora-staff").to_owned();
        let n = text.chars().count();
        let semibold = crate::ui::fonts::semibold(ui.ctx());
        let mut job = LayoutJob {
            break_on_newline: false,
            ..Default::default()
        };
        for (i, ch) in text.chars().enumerate() {
            let format = crate::ui::fonts::semibold_format(&semibold, font, aurora::letter_color(p, i, n, t));
            job.append(ch.encode_utf8(&mut [0; 4]), 0.0, format);
        }
        let label = ui.painter().layout_job(job);
        let size = Vec2::new(
            (2.0 * Self::pad(font) + font + Self::gap(font) + label.size().x).ceil(),
            (label.size().y + 2.0 * (font * 0.12).round()).ceil(),
        );
        Tag { label, font, t, size }
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
        aurora::tag(&mut mesh, p, rect, radius, ppp, self.t, fill, 1.0, 1.0);
        painter.add(Shape::mesh(mesh));
        let pad = Self::pad(self.font);
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + pad + self.font * 0.5, rect.center().y),
            Vec2::splat(self.font),
        );
        let icon = crate::ui::icons::global("sparkles-filled");
        if let Some(icon) = icon {
            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(icon.id(), icon_rect, whole, aurora::letter_color(p, 0, 2, self.t));
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

/// The tag of `id` after its name in a layout, its text `font` points
/// high, if it is a member of the team, with what it means under the
/// pointer.
pub fn after_name(ui: &mut egui::Ui, p: &Palette, id: &Uuid, font: f32) -> Option<egui::Response> {
    if !crate::world::staff::is_staff(id) {
        return None;
    }
    let tag = Tag::new(ui, p, font);
    let (rect, response) = ui.allocate_exact_size(tag.size(), egui::Sense::hover());
    tag.paint(ui, ui.painter(), p, rect);
    Some(response.on_hover_text(aurora_i18n::tr!("nameplate-aurora-staff-tooltip")))
}

/// The tag of `id` painted from `left_center` by a row that places its own
/// text (`font` the size of its name), if it is a member of the team: its
/// rectangle.
pub fn paint_at(ui: &egui::Ui, painter: &egui::Painter, p: &Palette, id: &Uuid, left_center: Pos2, font: f32) -> Option<Rect> {
    if !crate::world::staff::is_staff(id) {
        return None;
    }
    let tag = Tag::new(ui, p, font_for(font));
    let ppp = painter.pixels_per_point();
    let min = left_center - Vec2::new(0.0, tag.size().y * 0.5);
    let rect = Rect::from_min_size(Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp), tag.size());
    tag.paint(ui, painter, p, rect);
    Some(rect)
}

/// Width `paint_at` takes for `id` (0 when not a member of the team), for
/// a row that elides the name before it.
pub fn width(ui: &egui::Ui, p: &Palette, id: &Uuid, font: f32) -> f32 {
    if crate::world::staff::is_staff(id) {
        Tag::new(ui, p, font_for(font)).size().x
    } else {
        0.0
    }
}
