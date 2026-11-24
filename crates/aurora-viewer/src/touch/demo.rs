//! AURORA_DEMO_TOUCH scenarios (demo mode): scripted fingers, so the touch
//! controls can be checked and captured without a touch screen. The fingers
//! go through the same entry as the window's (`App::touch_event`). The
//! camera, the avatar and the stick are logged at the frames marked `Log`.
//!
//! Positions are shares of the window (0,0 top left; 1,1 bottom right).
//!
//! - `orbit`: one finger dragged to the right from frame 600 to 660
//! - `pinch`: two fingers spread from 600 to 640 (zoom in)
//! - `spread-out`: two fingers brought together from 600 to 640 (zoom out)
//! - `pan`: two fingers dragged upwards together from 600 to 640
//! - `tap[,x,y]`: a tap at 600 (the window's middle by default)
//! - `double-tap[,x,y]`: two taps, at 600 and 612
//! - `hold[,x,y]`: a finger held from 600 to 720: the context menu
//! - `reset`: the `orbit` drag, then two fingers tapped together at 700
//! - `stick`: the thumb lands in the stick's zone at 600, pushes forward to
//!   the rim by 615 (a run), eases to a walk at 700, turns right from 760
//!   and lifts at 840
//! - `stick-hold`: the thumb pushes forward at 600 and stays: for a capture
//!   of the stick
//! - `fly`: the Fly button at 600, the Up button held from 620 to 700, the
//!   Fly button again at 760
//! - `mouselook`: the Mouselook button at 600, a look drag from 640 to 700,
//!   the button again at 760
//! - `tour`: `orbit`, then `pinch` from 700, a two-finger tap at 800, the
//!   stick from 900 to 1080

use crate::touch::{Button, Id};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// A finger lands at this point (window pixels).
    Down(Id, f32, f32),
    Move(Id, f32, f32),
    Up(Id),
    /// A finger lands on a touch button, wherever it is drawn.
    Press(Id, Button),
    Log,
}

/// A value going from `a` to `b` between the frames `from` and `to`.
fn ramp(frame: u64, from: u64, to: u64, a: f32, b: f32) -> f32 {
    let t = (frame.saturating_sub(from)) as f32 / (to.saturating_sub(from)).max(1) as f32;
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn orbit(frame: u64, start: u64, (w, h): (f32, f32), out: &mut Vec<Step>) {
    let y = 0.45 * h;
    match frame.checked_sub(start) {
        Some(0) => out.extend([Step::Log, Step::Down(1, 0.55 * w, y)]),
        Some(1..=60) => out.push(Step::Move(1, ramp(frame, start, start + 60, 0.55, 0.75) * w, y)),
        Some(61) => out.push(Step::Up(1)),
        Some(70) => out.push(Step::Log),
        _ => {}
    }
}

/// Two fingers whose half distance goes from `a` to `b` (shares of the
/// width) while their middle goes from `m0` to `m1` (shares of the height).
fn two(frame: u64, start: u64, (w, h): (f32, f32), (a, b): (f32, f32), (m0, m1): (f32, f32), out: &mut Vec<Step>) {
    let at = |f: u64| {
        let half = ramp(f, start, start + 40, a, b) * w;
        let y = ramp(f, start, start + 40, m0, m1) * h;
        ((0.5 * w - half, y), (0.5 * w + half, y))
    };
    match frame.checked_sub(start) {
        Some(0) => {
            let (l, r) = at(frame);
            out.extend([Step::Log, Step::Down(1, l.0, l.1), Step::Down(2, r.0, r.1)]);
        }
        Some(1..=40) => {
            let (l, r) = at(frame);
            out.extend([Step::Move(1, l.0, l.1), Step::Move(2, r.0, r.1)]);
        }
        Some(41) => out.extend([Step::Up(1), Step::Up(2)]),
        Some(50) => out.push(Step::Log),
        _ => {}
    }
}

fn two_finger_tap(frame: u64, start: u64, (w, h): (f32, f32), out: &mut Vec<Step>) {
    match frame.checked_sub(start) {
        Some(0) => out.extend([Step::Down(1, 0.45 * w, 0.4 * h), Step::Down(2, 0.55 * w, 0.4 * h)]),
        Some(3) => out.extend([Step::Up(1), Step::Up(2)]),
        Some(60) => out.push(Step::Log),
        _ => {}
    }
}

/// The thumb on the stick: forward to the rim, back to a walk, a right
/// turn, then off. `(x, y)`: where the thumb lands; `travel`: the stick's
/// travel in window pixels.
fn stick(frame: u64, start: u64, (x, y): (f32, f32), travel: f32, out: &mut Vec<Step>) {
    match frame.checked_sub(start) {
        Some(0) => out.extend([Step::Log, Step::Down(7, x, y)]),
        Some(1..=15) => out.push(Step::Move(7, x, y - ramp(frame, start, start + 15, 0.0, 1.0) * travel)),
        Some(90) => out.push(Step::Log),
        Some(100..=110) => out.push(Step::Move(7, x, y - ramp(frame, start + 100, start + 110, 1.0, 0.6) * travel)),
        Some(150) => out.push(Step::Log),
        Some(160..=170) => out.push(Step::Move(
            7,
            x + ramp(frame, start + 160, start + 170, 0.0, 0.9) * travel,
            y - 0.6 * travel,
        )),
        Some(230) => out.push(Step::Log),
        Some(240) => out.push(Step::Up(7)),
        Some(250) => out.push(Step::Log),
        _ => {}
    }
}

/// The steps of frame `frame`. `size`: the window in pixels; `thumb`: where
/// the stick waits for the thumb; `travel`: the stick's travel in pixels.
pub fn steps(spec: &str, frame: u64, size: (f32, f32), thumb: (f32, f32), travel: f32) -> Vec<Step> {
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    let share = |i: usize, default: f32| {
        parts
            .get(i)
            .and_then(|v| v.parse::<f32>().ok())
            .map_or(default, |v| v.clamp(0.0, 1.0))
    };
    let (w, h) = size;
    let at = (share(1, 0.5) * w, share(2, 0.5) * h);
    let mut out = Vec::new();
    match parts.first().copied().unwrap_or("") {
        "orbit" => orbit(frame, 600, size, &mut out),
        "pinch" => two(frame, 600, size, (0.08, 0.2), (0.45, 0.45), &mut out),
        "spread-out" => two(frame, 600, size, (0.2, 0.08), (0.45, 0.45), &mut out),
        "pan" => two(frame, 600, size, (0.08, 0.08), (0.55, 0.35), &mut out),
        "tap" => match frame {
            600 => out.push(Step::Down(1, at.0, at.1)),
            603 => out.push(Step::Up(1)),
            660 => out.push(Step::Log),
            _ => {}
        },
        "double-tap" => match frame {
            600 | 612 => out.push(Step::Down(1, at.0, at.1)),
            603 | 615 => out.push(Step::Up(1)),
            700 => out.push(Step::Log),
            _ => {}
        },
        "hold" => match frame {
            600 => out.push(Step::Down(1, at.0, at.1)),
            720 => out.extend([Step::Log, Step::Up(1)]),
            _ => {}
        },
        "reset" => {
            orbit(frame, 600, size, &mut out);
            two_finger_tap(frame, 700, size, &mut out);
        }
        "stick" => stick(frame, 600, thumb, travel, &mut out),
        "stick-hold" => match frame {
            600 => out.push(Step::Down(7, thumb.0, thumb.1)),
            601..=615 => out.push(Step::Move(
                7,
                thumb.0 + ramp(frame, 600, 615, 0.0, 0.35) * travel,
                thumb.1 - ramp(frame, 600, 615, 0.0, 0.8) * travel,
            )),
            _ => {}
        },
        "fly" => match frame {
            600 | 760 => out.push(Step::Press(1, Button::Fly)),
            603 | 763 => out.push(Step::Up(1)),
            620 => out.push(Step::Press(2, Button::Up)),
            700 => out.extend([Step::Log, Step::Up(2)]),
            820 => out.push(Step::Log),
            _ => {}
        },
        "mouselook" => match frame {
            600 | 760 => out.push(Step::Press(1, Button::Mouselook)),
            603 | 763 => out.push(Step::Up(1)),
            630 => out.push(Step::Log),
            640 => out.push(Step::Down(2, 0.5 * w, 0.5 * h)),
            641..=700 => out.push(Step::Move(2, ramp(frame, 640, 700, 0.5, 0.62) * w, 0.5 * h)),
            701 => out.extend([Step::Up(2), Step::Log]),
            820 => out.push(Step::Log),
            _ => {}
        },
        "tour" => {
            orbit(frame, 600, size, &mut out);
            two(frame, 700, size, (0.08, 0.2), (0.45, 0.45), &mut out);
            two_finger_tap(frame, 800, size, &mut out);
            stick(frame, 900, thumb, travel, &mut out);
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f32, f32) = (1000.0, 800.0);
    const THUMB: (f32, f32) = (140.0, 624.0);

    fn all(spec: &str, range: std::ops::RangeInclusive<u64>) -> Vec<(u64, Step)> {
        range
            .flat_map(|f| steps(spec, f, SIZE, THUMB, 100.0).into_iter().map(move |s| (f, s)))
            .collect()
    }

    #[test]
    fn every_finger_that_lands_lifts() {
        for spec in [
            "orbit",
            "pinch",
            "spread-out",
            "pan",
            "tap",
            "double-tap",
            "hold",
            "reset",
            "stick",
            "fly",
            "mouselook",
            "tour",
        ] {
            let mut down: Vec<Id> = Vec::new();
            for (frame, step) in all(spec, 0..=1400) {
                match step {
                    Step::Down(id, ..) | Step::Press(id, _) => {
                        assert!(!down.contains(&id), "{spec}: finger {id} lands twice at {frame}");
                        down.push(id);
                    }
                    Step::Move(id, ..) => assert!(down.contains(&id), "{spec}: finger {id} moves in the air at {frame}"),
                    Step::Up(id) => {
                        assert!(down.contains(&id), "{spec}: finger {id} lifts without landing at {frame}");
                        down.retain(|d| *d != id);
                    }
                    Step::Log => {}
                }
            }
            assert!(down.is_empty(), "{spec}: fingers left down: {down:?}");
        }
    }

    #[test]
    fn the_held_stick_keeps_its_thumb_down_for_the_capture() {
        let steps = all("stick-hold", 0..=1400);
        assert_eq!(steps.iter().filter(|(_, s)| matches!(s, Step::Down(..))).count(), 1);
        assert!(!steps.iter().any(|(_, s)| matches!(s, Step::Up(_))));
        let (_, last) = steps.last().expect("the thumb moves");
        let Step::Move(_, x, y) = *last else {
            panic!("the last step is a move: {last:?}")
        };
        assert!(x > THUMB.0 && y < THUMB.1, "pushed forward and to the right: {x}, {y}");
    }

    #[test]
    fn nothing_happens_before_the_world_shows_or_for_an_unknown_scenario() {
        assert!(all("tour", 0..=599).is_empty());
        assert!(all("nothing", 0..=1400).is_empty());
        assert!(all("", 0..=1400).is_empty());
    }

    #[test]
    fn a_tap_lands_where_it_is_told_inside_the_window() {
        assert_eq!(steps("tap,0.25,0.5", 600, SIZE, THUMB, 100.0), [Step::Down(1, 250.0, 400.0)]);
        assert_eq!(steps("tap", 600, SIZE, THUMB, 100.0), [Step::Down(1, 500.0, 400.0)]);
        assert_eq!(steps("tap,7,-3", 600, SIZE, THUMB, 100.0), [Step::Down(1, 1000.0, 0.0)]);
    }

    #[test]
    fn the_stick_script_reaches_the_rim_then_eases_and_turns() {
        let moves: Vec<_> = all("stick", 600..=900)
            .into_iter()
            .filter_map(|(f, s)| match s {
                Step::Move(_, x, y) => Some((f, x, y)),
                _ => None,
            })
            .collect();
        let at = |frame: u64| moves.iter().rev().find(|m| m.0 <= frame).map(|m| (m.1, m.2)).expect("a move");
        let origin = (140.0, 624.0);
        assert_eq!(at(615), (origin.0, origin.1 - 100.0));
        assert!((at(710).1 - (origin.1 - 60.0)).abs() < 1e-3);
        assert!((at(770).0 - (origin.0 + 90.0)).abs() < 1e-3);
    }

    #[test]
    fn a_pinch_spreads_about_the_middle_of_the_window() {
        let first = steps("pinch", 600, SIZE, THUMB, 100.0);
        assert_eq!(first, [Step::Log, Step::Down(1, 420.0, 360.0), Step::Down(2, 580.0, 360.0)]);
        let last = steps("pinch", 640, SIZE, THUMB, 100.0);
        assert_eq!(last, [Step::Move(1, 300.0, 360.0), Step::Move(2, 700.0, 360.0)]);
    }
}
