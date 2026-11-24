//! Click to copy: a name drawn as a label (a resident's display name and
//! username, a group's name) copies itself on a left click. The name is
//! underlined for a moment and a small "Copied" follows the pointer, both
//! fading out within a second, so the click visibly did something. An Aurora
//! addition: the Legacy Viewer copies names only through menus.

use crate::theme::Palette;
use egui::{Align2, Id, Pos2, Rect, RichText, Stroke, Vec2, pos2, vec2};

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

/// The last copy: which widget, when, what the label says and what it sits
/// beside.
#[derive(Clone)]
struct Copied {
    id: Id,
    at: f64,
    anchor: Rect,
    text: String,
}

fn key() -> Id {
    Id::unique("aurora_copy_on_click")
}

/// Starts the copy feedback of the widget `id`: `text` shows under `anchor`
/// for a second, then fades out. Drawn by [`copied_feedback`]; a new copy
/// anywhere replaces it.
pub fn show_copied(ctx: &egui::Context, id: Id, anchor: Rect, text: impl Into<String>) {
    let at = ctx.input(|i| i.time);
    let text = text.into();
    ctx.data_mut(|d| d.insert_temp(key(), Copied { id, at, anchor, text }));
    ctx.request_repaint();
}

/// Draws the feedback [`show_copied`] started for `id`, while it lasts:
/// under the anchor, its right edge on the anchor's, and kept inside
/// `within` (the window the control is in).
pub fn copied_feedback(ctx: &egui::Context, p: &Palette, id: Id, within: Rect) {
    if let Some((copied, alpha)) = active(ctx, id) {
        let at = copied.anchor.right_bottom() + vec2(0.0, 4.0);
        label(ctx, p, at, Align2::RIGHT_TOP, Some(within), &copied.text, alpha);
    }
}

/// The feedback of `id` and its opacity while it shows; forgets it after.
fn active(ctx: &egui::Context, id: Id) -> Option<(Copied, f32)> {
    let copied = ctx.data(|d| d.get_temp::<Copied>(key())).filter(|c| c.id == id)?;
    match feedback_alpha(ctx.input(|i| i.time) - copied.at) {
        Some(alpha) => {
            ctx.request_repaint();
            Some((copied, alpha))
        }
        None => {
            ctx.data_mut(|d| d.remove::<Copied>(key()));
            None
        }
    }
}

/// `resp` comes from a widget that senses clicks (a `Label` with
/// `.sense(Sense::click())`). A left click copies `text`; the feedback is
/// drawn while it lasts. Returns the response with its "Click to copy" tip.
pub fn copy_on_click(ui: &egui::Ui, p: &Palette, resp: egui::Response, text: &str) -> egui::Response {
    let ctx = ui.ctx().clone();
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
        show_copied(
            &ctx,
            resp.id,
            Rect::from_min_size(pointer, Vec2::ZERO),
            aurora_i18n::tr!("common-copied"),
        );
    }
    if let Some((copied, alpha)) = active(&ctx, resp.id) {
        // The name is underlined and the label follows the pointer.
        let r = resp.rect;
        ui.painter().line_segment(
            [pos2(r.left(), r.bottom() + 1.0), pos2(r.right(), r.bottom() + 1.0)],
            Stroke::new(1.5, p.violet_light.gamma_multiply(alpha)),
        );
        let pointer = ctx.pointer_latest_pos().unwrap_or(copied.anchor.min);
        label(&ctx, p, pointer + vec2(14.0, 12.0), Align2::LEFT_TOP, None, &copied.text, alpha);
    }
    resp.on_hover_text(aurora_i18n::tr!("common-click-to-copy"))
}

/// The copied label, its `pivot` corner on `at`, on one line (the area keeps
/// the size of the last label shown, so a longer one would wrap), inside
/// `within` or the screen.
fn label(ctx: &egui::Context, p: &Palette, at: Pos2, pivot: Align2, within: Option<Rect>, text: &str, alpha: f32) {
    let mut area = egui::Area::new(key().with("label"))
        .order(egui::Order::Tooltip)
        .fade_in(false)
        .interactable(false)
        .pivot(pivot)
        .fixed_pos(at);
    if let Some(within) = within {
        area = area.constrain_to(within);
    }
    area.show(ctx, |ui| {
        ui.multiply_opacity(alpha);
        super::overlay(ui, p, egui::Margin::symmetric(8, 3), |ui| {
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.ink)).extend());
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
