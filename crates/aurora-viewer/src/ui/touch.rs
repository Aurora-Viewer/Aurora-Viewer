//! The touch controls over the world: the movement stick under the left
//! thumb and the buttons under the right one (src/touch.rs, app/touch.rs).
//!
//! The touch layer tests the fingers itself against the rectangles this
//! pass returns, so that a thumb on a button never becomes the interface's
//! pointer: the other hand keeps it for the windows. A mouse, which is the
//! interface's pointer, presses the buttons here. They lie under the
//! windows, in the layer of the bars.

use super::widgets;
use crate::theme::Palette;
use crate::touch::{Button, STICK_RADIUS};
use egui::{Pos2, Rect, Vec2};

/// Side of a touch button: a thumb's target (44 points at least).
pub const BUTTON: f32 = 52.0;
const GAP: f32 = 10.0;
/// From the work area's sides.
const EDGE: f32 = 16.0;
const GLYPH: f32 = 24.0;
/// The knob against the ring.
const KNOB: f32 = 0.42;

/// What a frame shows.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// The stick under a thumb: where it started and where the thumb is.
    pub stick: Option<(Pos2, Pos2)>,
    /// The stick can be used (not in the avatar studio, the build tools…).
    pub stick_allowed: bool,
    pub running: bool,
    pub flying: bool,
    pub mouselook: bool,
    pub held: Vec<Button>,
}

/// Under this height of the work area (a phone held sideways) the buttons
/// take two rows instead of three, and leave the upper half to the windows.
const SHORT: f32 = 560.0;

/// The buttons, in the lower right corner of the work area: the two held
/// ones (up, down) under the thumb, the toggles around them. Two columns of
/// three rows, or three columns of two rows where the screen is short.
pub fn buttons(work: Rect) -> Vec<(Button, Rect)> {
    let cell = |col: f32, row: f32| {
        let max = Pos2::new(
            work.right() - EDGE - col * (BUTTON + GAP),
            work.bottom() - EDGE - row * (BUTTON + GAP),
        );
        Rect::from_min_max(max - Vec2::splat(BUTTON), max)
    };
    if work.height() < SHORT {
        return vec![
            (Button::Down, cell(0.0, 0.0)),
            (Button::Up, cell(0.0, 1.0)),
            (Button::Fly, cell(1.0, 0.0)),
            (Button::Mouselook, cell(1.0, 1.0)),
            (Button::ResetView, cell(2.0, 1.0)),
        ];
    }
    vec![
        (Button::Down, cell(0.0, 0.0)),
        (Button::Up, cell(0.0, 1.0)),
        (Button::Fly, cell(1.0, 1.0)),
        (Button::Mouselook, cell(0.0, 2.0)),
        (Button::ResetView, cell(1.0, 2.0)),
    ]
}

/// Where the stick waits for a thumb: at the left side, two thirds down,
/// where a hand holding the tablet has its thumb, and above the lines of
/// the nearby chat in the corner.
pub fn stick_rest(work: Rect) -> Pos2 {
    let inset = EDGE + STICK_RADIUS + 8.0;
    let y = (work.top() + work.height() * 0.66)
        .min(work.bottom() - inset)
        .max(work.top() + inset);
    Pos2::new(work.left() + inset, y)
}

/// Where the nearby chat's lines start on a short screen (a phone held
/// sideways), where they would lie over the stick in the corner: beside it.
/// None on a taller screen, where the stick waits above them.
pub fn chat_lines_left(work: Rect) -> Option<f32> {
    (work.height() < SHORT).then(|| stick_rest(work).x + STICK_RADIUS + GAP)
}

/// The icon of a button: the one its action has everywhere else (the
/// Controls window, the toolbar).
fn icon(button: Button) -> &'static str {
    match button {
        Button::Up => "chevrons-up",
        Button::Down => "chevrons-down",
        Button::Fly => "plane-tilt-filled",
        Button::Mouselook => "eye-filled",
        Button::ResetView => "refresh",
    }
}

fn lit(button: Button, s: &State) -> bool {
    match button {
        Button::Fly => s.flying,
        Button::Mouselook => s.mouselook,
        _ => s.held.contains(&button),
    }
}

/// What a pass drew, and what the mouse did on the buttons.
#[derive(Debug, Clone, Default)]
pub struct Shown {
    /// The buttons' rectangles, for the touch layer's own hit test.
    pub buttons: Vec<(Button, Rect)>,
    /// The button the mouse clicked, of those that act once.
    pub clicked: Option<Button>,
    /// The held buttons the mouse presses now.
    pub held: Vec<Button>,
}

/// The area of a button. Each has its own, so that the world between two
/// buttons stays the world's.
fn area(button: Button) -> egui::Id {
    egui::Id::unique("touch_button").with(button)
}

/// Draws the controls.
pub fn show(ctx: &egui::Context, p: &Palette, s: &State, work: Rect) -> Shown {
    let radius = ctx.global_style().visuals.window_corner_radius;
    let mut shown = Shown {
        buttons: buttons(work),
        ..Shown::default()
    };
    for &(button, rect) in &shown.buttons {
        let id = area(button);
        // a panel of its own: a mouse click on it is not the world's too
        widgets::solid_background(ctx, id);
        let response = egui::Area::new(id)
            .order(egui::Order::Background)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
                let on = lit(button, s) || (button.held() && response.is_pointer_button_down_on());
                ui.painter().add(widgets::island_shape(ctx, p, rect));
                if on {
                    ui.painter().rect_filled(rect.shrink(1.0), radius, p.violet);
                }
                widgets::paint_glyph(
                    ui.painter(),
                    icon(button),
                    rect.center(),
                    GLYPH,
                    if on { p.on_accent } else { p.ink },
                );
                response
            })
            .inner;
        if button.held() {
            if response.is_pointer_button_down_on() {
                shown.held.push(button);
            }
        } else if response.clicked() {
            shown.clicked = Some(button);
        }
    }
    if s.stick_allowed {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("touch_stick")));
        stick(&painter, p, s, work);
    }
    shown
}

/// The stick: its ring where the thumb landed (or where it waits, dimmer),
/// the four directions, and the knob under the thumb, lit while it runs.
fn stick(painter: &egui::Painter, p: &Palette, s: &State, work: Rect) {
    let (center, knob, held) = match s.stick {
        Some((origin, at)) => {
            let pulled = at - origin;
            let reach = STICK_RADIUS;
            let offset = if pulled.length() > reach {
                pulled.normalized() * reach
            } else {
                pulled
            };
            (origin, origin + offset, true)
        }
        None => {
            let rest = stick_rest(work);
            (rest, rest, false)
        }
    };
    let dim = if held { 1.0 } else { 0.4 };
    painter.circle_filled(center, STICK_RADIUS, p.bar.gamma_multiply(0.55 * dim));
    painter.circle_stroke(center, STICK_RADIUS - 0.5, egui::Stroke::new(1.0, p.raised.gamma_multiply(dim)));
    for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
        let d = Vec2::new(dx, dy);
        let tip = center + d * (STICK_RADIUS - 7.0);
        let base = center + d * (STICK_RADIUS - 14.0);
        let side = Vec2::new(-d.y, d.x) * 5.0;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, base + side, base - side],
            p.muted.gamma_multiply(dim),
            egui::Stroke::NONE,
        ));
    }
    let face = if s.running {
        p.violet
    } else if held {
        p.violet_light
    } else {
        p.raised.gamma_multiply(0.8)
    };
    painter.circle_filled(knob, STICK_RADIUS * KNOB, face);
    let glyph = if s.running { "run" } else { "walk" };
    let tint = if held { p.on_accent } else { p.muted };
    widgets::paint_glyph(painter, glyph, knob, 20.0, tint);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> Rect {
        Rect::from_min_max(Pos2::new(8.0, 60.0), Pos2::new(1358.0, 960.0))
    }

    /// A phone held sideways, at its interface scale.
    fn short() -> Rect {
        Rect::from_min_max(Pos2::new(82.0, 52.0), Pos2::new(1010.0, 440.0))
    }

    #[test]
    fn the_buttons_are_thumb_sized_apart_and_inside_the_work_area() {
        for work in [work(), short()] {
            let all = buttons(work);
            assert_eq!(all.len(), 5);
            for (i, (button, rect)) in all.iter().enumerate() {
                assert!(work.contains_rect(*rect), "{button:?} leaves the work area");
                assert!(rect.width() >= 44.0 && rect.height() >= 44.0);
                for (other, r) in &all[i + 1..] {
                    assert!(!rect.intersects(*r), "{button:?} overlaps {other:?}");
                }
            }
        }
    }

    #[test]
    fn the_chat_lines_start_beside_the_stick_on_a_short_screen_only() {
        assert_eq!(chat_lines_left(work()), None);
        let left = chat_lines_left(short()).expect("a short screen");
        assert!(left > stick_rest(short()).x + STICK_RADIUS);
        // room is left for a line of chat before the buttons
        let buttons_left = buttons(short()).iter().map(|(_, r)| r.left()).fold(f32::MAX, f32::min);
        assert!(buttons_left - left > 300.0, "{} points for the chat lines", buttons_left - left);
    }

    #[test]
    fn a_short_screen_keeps_its_upper_half_free_of_buttons() {
        let top = |work: Rect| buttons(work).iter().map(|(_, r)| r.top()).fold(f32::MAX, f32::min);
        assert!(top(short()) > short().center().y);
        // (the tall layout is the one a tablet gets)
        assert_eq!(work().bottom() - top(work()), EDGE + 3.0 * BUTTON + 2.0 * GAP);
    }

    #[test]
    fn the_held_buttons_are_in_the_corner_under_the_thumb() {
        let all = buttons(work());
        let rect = |b: Button| all.iter().find(|(x, _)| *x == b).map(|(_, r)| *r).expect("button");
        let (down, up) = (rect(Button::Down), rect(Button::Up));
        assert_eq!(down.right(), work().right() - EDGE);
        assert_eq!(down.bottom(), work().bottom() - EDGE);
        assert_eq!(up.left(), down.left());
        assert!(up.bottom() < down.top());
    }

    #[test]
    fn the_stick_waits_inside_its_own_zone_and_away_from_the_buttons() {
        let rest = stick_rest(work());
        let w = work();
        let zone = crate::touch::stick_zone([w.left(), w.top(), w.right(), w.bottom()]);
        assert!(crate::touch::inside(zone, glam::Vec2::new(rest.x, rest.y)));
        let ring = Rect::from_center_size(rest, Vec2::splat(2.0 * STICK_RADIUS));
        assert!(w.contains_rect(ring));
        for (_, rect) in buttons(w) {
            assert!(!ring.intersects(rect));
        }
        let s = short();
        let ring = Rect::from_center_size(stick_rest(s), Vec2::splat(2.0 * STICK_RADIUS));
        assert!(s.contains_rect(ring));
        assert!(buttons(s).iter().all(|(_, rect)| !ring.intersects(*rect)));
    }

    #[test]
    fn a_toggle_is_lit_by_its_state_and_a_held_button_by_the_thumb() {
        let s = State {
            flying: true,
            held: vec![Button::Up],
            ..State::default()
        };
        assert!(lit(Button::Fly, &s) && lit(Button::Up, &s));
        assert!(!lit(Button::Mouselook, &s) && !lit(Button::Down, &s) && !lit(Button::ResetView, &s));
    }
}
