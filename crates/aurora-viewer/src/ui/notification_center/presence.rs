//! The friends' online / offline toasts of Aurora's notification center:
//! small toasts apart from the bell (never a card nor a count), at one of
//! six places the user chooses (Preferences › Chat › Toasts,
//! `Settings::presence_position`), top right by default.
//!
//! They stand in the room the bars leave (`ui::layout::work_area`, which
//! the bars report each frame from their own rectangles, in both looks),
//! never over a bar. At the bottom they start from the bottom of that room
//! and stack upwards, at the top from its top and stack downwards; in the
//! middle they are centred. What else stands across their column (the
//! nearby chat lines over the chat bar) shortens the room from below: the
//! toasts stack above those lines. In a window too low for all of them,
//! only the newest that fit show.
//!
//! At the top right they stand where the bell's cards unroll: while a card
//! is out or the list is open, a new toast waits; it shows when the place
//! is free if it arrived less than `PRESENCE` seconds before, else it is
//! dropped (it never enters the bell's list), and one already shown leaves.

use super::{PRESENCE, PRESENCE_FADE};
use crate::settings::PresencePosition;
use egui::{Rect, Vec2};

/// Between two toasts, and between the toasts and the chat lines.
pub const GAP: f32 = 6.0;
/// From Firestorm's full-width bars and from the window's sides (the
/// islands' room already keeps their 8 px).
pub const EDGE: f32 = 8.0;
/// The size a toast is laid out with before its first frame measured it
/// (it is drawn invisible that frame).
const GUESS: Vec2 = Vec2::new(220.0, 44.0);

/// A friend's toast: when the notice arrived, and since when it shows
/// (`None` while it waits for the place under the bell).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub arrived: f64,
    pub shown: Option<f64>,
}

/// Advance the toasts: the new ones wait while `busy` (at the top right
/// with the bell's cards or list out), else show; a waiting toast older
/// than `PRESENCE` is dropped, a shown one leaves when busy or once faded.
pub fn advance(toasts: &mut Vec<Toast>, now: f64, busy: bool) {
    toasts.retain(|t| match t.shown {
        Some(_) if busy => false,
        Some(since) => now - since < PRESENCE + PRESENCE_FADE,
        None => now - t.arrived < PRESENCE,
    });
    if !busy {
        for t in toasts.iter_mut().filter(|t| t.shown.is_none()) {
            t.shown = Some(now);
        }
    }
}

/// The room the toasts take: the bars' work area, `EDGE` inside it with
/// Firestorm's bars (they leave the whole width up to their edges).
pub fn room(work_area: Rect, classic_bars: bool) -> Rect {
    if classic_bars { work_area.shrink(EDGE) } else { work_area }
}

/// Where each toast of `sizes` (oldest first) stands at `at` in `room`,
/// above the rectangles of `avoid` that stand across their column; `None`
/// for those that do not fit (the oldest go first).
pub fn stack(room: Rect, at: PresencePosition, sizes: &[Vec2], avoid: &[Rect]) -> Vec<Option<Rect>> {
    let x_of = |w: f32| match at {
        PresencePosition::TopLeft | PresencePosition::BottomLeft => room.left(),
        PresencePosition::TopRight | PresencePosition::BottomRight => room.right() - w,
        PresencePosition::TopCenter | PresencePosition::BottomCenter => (room.center().x - w / 2.0).round(),
    };
    let sizes: Vec<Vec2> = sizes.iter().map(|s| Vec2::new(s.x.min(room.width()), s.y)).collect();
    let (mut left, mut right) = (f32::INFINITY, f32::NEG_INFINITY);
    for s in &sizes {
        left = left.min(x_of(s.x));
        right = right.max(x_of(s.x) + s.x);
    }
    // what stands across the column shortens the room from below (the
    // chat lines, anchored on the bottom bar) or from above
    let whole = room;
    let mut room = room;
    for r in avoid
        .iter()
        .filter(|r| r.left() < right && r.right() > left && r.top() < whole.bottom() && r.bottom() > whole.top())
    {
        if r.center().y >= whole.center().y {
            room.max.y = room.max.y.min(r.top() - GAP);
        } else {
            room.min.y = room.min.y.max(r.bottom() + GAP);
        }
    }
    // the newest that fit, from the newest back
    let mut first = sizes.len();
    let mut used = 0.0;
    for (i, s) in sizes.iter().enumerate().rev() {
        let need = used + s.y + if used > 0.0 { GAP } else { 0.0 };
        if need > room.height() {
            break;
        }
        used = need;
        first = i;
    }
    let mut out = vec![None; sizes.len()];
    let mut y = if at.top() { room.top() } else { room.bottom() };
    for (i, s) in sizes.iter().enumerate().skip(first) {
        let top = if at.top() { y } else { y - s.y };
        out[i] = Some(Rect::from_min_size(egui::pos2(x_of(s.x), top), *s));
        y = if at.top() { y + s.y + GAP } else { y - s.y - GAP };
    }
    out
}

/// The size `id` was drawn with last, if it was.
pub fn size_of(sizes: &std::collections::HashMap<u64, Vec2>, id: u64) -> (Vec2, bool) {
    sizes.get(&id).map_or((GUESS, false), |s| (*s, true))
}

#[cfg(test)]
mod tests;
