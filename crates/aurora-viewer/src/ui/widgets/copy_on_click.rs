//! Click to copy: a name drawn as a label (a resident's display name and
//! username, a group's name) copies itself on a left click. The name is
//! underlined for a moment and a small "Copied" follows the pointer, both
//! fading out within a second, so the click visibly did something. An Aurora
//! addition: the Legacy Viewer copies names only through menus.

use crate::theme::Palette;
use egui::{Id, Pos2, RichText, Stroke, pos2, vec2};

/// How long the feedback shows, and the end part of it that fades out (s).
const SHOW: f64 = 1.0;
const FADE: f64 = 0.35;

/// The feedback's opacity `elapsed` seconds after the click: full, then
/// fading over the last `FADE`, gone after `SHOW`.
pub(crate) fn feedback_alpha(elapsed: f64) -> Option<f32> {
    if !(0.0..SHOW).contains(&elapsed) {
        return None;
    }
    let left = SHOW - elapsed;
    Some(if left >= FADE { 1.0 } else { (left / FADE) as f32 })
}

/// The last copy: which widget, when, where the pointer was.
#[derive(Clone, Copy)]
struct Copied {
    id: Id,
    at: f64,
    pointer: Pos2,
}

fn key() -> Id {
    Id::unique("aurora_copy_on_click")
}

/// `resp` comes from a widget that senses clicks (a `Label` with
/// `.sense(Sense::click())`). A left click copies `text`; the feedback is
/// drawn while it lasts. Returns the response with its "Click to copy" tip.
pub fn copy_on_click(ui: &egui::Ui, p: &Palette, resp: egui::Response, text: &str) -> egui::Response {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    if text.is_empty() {
        return resp;
    }
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        ctx.copy_text(text.to_owned());
        let pointer = resp
            .interact_pointer_pos()
            .or_else(|| ctx.pointer_latest_pos())
            .unwrap_or(resp.rect.right_center());
        ctx.data_mut(|d| {
            d.insert_temp(
                key(),
                Copied {
                    id: resp.id,
                    at: now,
                    pointer,
                },
            )
        });
    }
    if let Some(copied) = ctx.data(|d| d.get_temp::<Copied>(key())).filter(|c| c.id == resp.id) {
        match feedback_alpha(now - copied.at) {
            Some(alpha) => {
                feedback(ui, p, &resp, copied, alpha);
                ctx.request_repaint();
            }
            None => ctx.data_mut(|d| d.remove::<Copied>(key())),
        }
    }
    resp.on_hover_text(aurora_i18n::tr!("common-click-to-copy"))
}

/// The underline under the name and the "Copied" label next to the pointer.
fn feedback(ui: &egui::Ui, p: &Palette, resp: &egui::Response, copied: Copied, alpha: f32) {
    let r = resp.rect;
    ui.painter().line_segment(
        [pos2(r.left(), r.bottom() + 1.0), pos2(r.right(), r.bottom() + 1.0)],
        Stroke::new(1.5, p.violet_light.gamma_multiply(alpha)),
    );
    let ctx = ui.ctx();
    let at = ctx.pointer_latest_pos().unwrap_or(copied.pointer) + vec2(14.0, 12.0);
    egui::Area::new(key().with("label"))
        .order(egui::Order::Tooltip)
        .fade_in(false)
        .interactable(false)
        .fixed_pos(at)
        .show(ctx, |ui| {
            ui.multiply_opacity(alpha);
            super::overlay(ui, p, egui::Margin::symmetric(8, 3), |ui| {
                ui.label(RichText::new(aurora_i18n::tr!("common-copied")).size(12.0).color(p.ink));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::feedback_alpha;

    #[test]
    fn the_feedback_shows_fully_then_fades_out_within_a_second() {
        assert_eq!(feedback_alpha(0.0), Some(1.0));
        assert_eq!(feedback_alpha(0.5), Some(1.0));
        let fading = feedback_alpha(0.9).expect("still shown");
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        assert!(feedback_alpha(0.95).expect("shown") < fading);
        assert_eq!(feedback_alpha(1.0), None);
        assert_eq!(feedback_alpha(-0.1), None);
    }
}
