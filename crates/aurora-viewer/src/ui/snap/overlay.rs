//! What is drawn over the windows while one is moved or resized: the guide
//! of the snap offered, and the reminder of Alt's role in the gesture.

use super::Guide;
use crate::keybinds::{Binding, Input};
use crate::theme::Palette;
use crate::ui::widgets::{self, keycap};
use egui::{Align2, Id, LayerId, Order, Pos2, Rect, RichText, Stroke, Vec2};
use winit::keyboard::KeyCode;

/// What Alt does in the gesture under way; shown after its keycap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hint {
    /// A window moves freely: Alt enables snapping and attachment.
    Snap,
    /// A snap is offered: Alt leaves the window where it is dropped.
    NoSnap,
    /// The window moves with the ones glued to it: Alt takes it out.
    Detach,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Overlay {
    pub guide: Guide,
    /// The window under the pointer, where it is this frame.
    pub window: Rect,
    pub hint: Option<Hint>,
    /// When the gesture began (input time).
    pub began: f64,
}

/// The hint waits this long, then fades in: a quick drag does not flash it.
const HINT_DELAY: f64 = 0.15;
const HINT_FADE: f64 = 0.12;
/// Room between the hint and the window it speaks of.
const HINT_GAP: f32 = 6.0;

fn hint_id() -> Id {
    Id::unique("window_snap_hint")
}

pub(crate) fn paint(ctx: &egui::Context, p: &Palette, overlay: &Overlay) {
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("window_snap_guide")));
    let radius = ctx.global_style().visuals.window_corner_radius;
    let accent = p.violet_light;
    for rect in &overlay.guide.landing {
        painter.rect(
            *rect,
            radius,
            accent.gamma_multiply(0.06),
            Stroke::new(1.0, accent.gamma_multiply(0.45)),
            egui::StrokeKind::Inside,
        );
    }
    for line in &overlay.guide.lines {
        painter.line_segment(*line, Stroke::new(2.0, accent));
    }
    let Some(hint) = overlay.hint else { return };
    let age = ctx.input(|i| i.time) - overlay.began - HINT_DELAY;
    if age <= 0.0 {
        ctx.request_repaint();
        return;
    }
    let opacity = (age / HINT_FADE).min(1.0) as f32;
    if opacity < 1.0 {
        ctx.request_repaint();
    }
    // its size is the one of the last frame it was shown in
    let size = ctx.memory(|m| m.area_rect(hint_id())).map_or(Vec2::new(190.0, 28.0), |r| r.size());
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    let (pivot, at) = hint_place(overlay, size, ctx.content_rect(), pointer);
    egui::Area::new(hint_id())
        .order(Order::Tooltip)
        .interactable(false)
        .fade_in(false)
        .pivot(pivot)
        .fixed_pos(at)
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            widgets::overlay(ui, p, egui::Margin::symmetric(8, 5), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let alt = Binding {
                        input: Input::key(KeyCode::AltLeft),
                        ctrl: false,
                        shift: false,
                        alt: false,
                    };
                    let caps = keycap::caps(ui, &alt, p.panel);
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(keycap::row_width(&caps), keycap::HEIGHT), egui::Sense::hover());
                    keycap::paint_row(ui, p, &caps, rect.left_center(), p.panel);
                    let text = match hint {
                        Hint::Snap => aurora_i18n::tr!("ui-snap-alt-snap"),
                        Hint::NoSnap => aurora_i18n::tr!("ui-snap-alt-no-snap"),
                        Hint::Detach => aurora_i18n::tr!("ui-snap-alt-detach"),
                    };
                    ui.label(RichText::new(text).size(12.0).color(p.muted));
                });
            });
        });
}

/// Where the hint goes: centered above the window, else under it, else
/// inside its bottom edge; the first place that is inside the screen, off
/// the lines of the guide and not under the pointer.
fn hint_place(overlay: &Overlay, size: Vec2, screen: Rect, pointer: Option<Pos2>) -> (Align2, Pos2) {
    let window = overlay.window;
    let x = window.center().x.clamp(
        screen.left() + size.x / 2.0,
        (screen.right() - size.x / 2.0).max(screen.left() + size.x / 2.0),
    );
    let places = [
        (Align2::CENTER_BOTTOM, Pos2::new(x, window.top() - HINT_GAP)),
        (Align2::CENTER_TOP, Pos2::new(x, window.bottom() + HINT_GAP)),
        (Align2::CENTER_BOTTOM, Pos2::new(x, window.bottom() - HINT_GAP)),
    ];
    let rect_of = |(pivot, at): (Align2, Pos2)| pivot.anchor_size(at, size);
    let free = |rect: Rect| {
        screen.contains_rect(rect)
            && pointer.is_none_or(|pos| !rect.expand(4.0).contains(pos))
            && overlay
                .guide
                .lines
                .iter()
                .all(|line| !rect.expand(2.0).intersects(Rect::from_two_pos(line[0], line[1])))
    };
    places.into_iter().find(|place| free(rect_of(*place))).unwrap_or(places[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1600.0, 900.0));
    const SIZE: Vec2 = Vec2::new(180.0, 28.0);

    fn overlay(window: Rect, lines: Vec<[Pos2; 2]>) -> Overlay {
        Overlay {
            guide: Guide {
                lines,
                landing: Vec::new(),
            },
            window,
            hint: Some(Hint::NoSnap),
            began: 0.0,
        }
    }

    fn placed(overlay: &Overlay, pointer: Option<Pos2>) -> Rect {
        let (pivot, at) = hint_place(overlay, SIZE, SCREEN, pointer);
        pivot.anchor_size(at, SIZE)
    }

    #[test]
    fn the_hint_stands_above_the_window_off_the_guide_and_the_pointer() {
        let window = Rect::from_min_size(Pos2::new(500.0, 300.0), Vec2::new(400.0, 300.0));
        // a guide along its left edge: above the window
        let left = overlay(window, vec![[window.left_top(), window.left_bottom()]]);
        let hint = placed(&left, Some(Pos2::new(700.0, 314.0)));
        assert_eq!(hint.center_bottom(), Pos2::new(700.0, 300.0 - HINT_GAP));
        // a guide along its top edge and the one it meets: under the window
        let top = overlay(
            window,
            vec![
                [window.left_top(), window.right_top()],
                [Pos2::new(500.0, 292.0), Pos2::new(900.0, 292.0)],
            ],
        );
        let hint = placed(&top, Some(Pos2::new(700.0, 314.0)));
        assert_eq!(hint.center_top(), Pos2::new(700.0, 600.0 + HINT_GAP));
        // against the top of the screen: under it too
        let high = overlay(window.translate(Vec2::new(0.0, -290.0)), Vec::new());
        assert_eq!(placed(&high, None).top(), 310.0 + HINT_GAP);
        // no room above nor under: inside its bottom edge
        let tall = overlay(Rect::from_min_max(Pos2::new(500.0, 10.0), Pos2::new(900.0, 890.0)), Vec::new());
        assert_eq!(placed(&tall, None).bottom(), 890.0 - HINT_GAP);
        // never under the pointer (the mini-map is dragged anywhere)
        let hint = placed(&left, Some(Pos2::new(700.0, 290.0)));
        assert!(!hint.contains(Pos2::new(700.0, 290.0)));
        // and inside the screen at its edges
        let corner = overlay(Rect::from_min_size(Pos2::new(1500.0, 300.0), Vec2::new(92.0, 200.0)), Vec::new());
        assert!(SCREEN.contains_rect(placed(&corner, None)));
    }
}
