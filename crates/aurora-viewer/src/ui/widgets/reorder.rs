//! Drag to reorder a list of rows of one height: the row pressed and
//! dragged is carried under the pointer as a lifted card, the others slide
//! out of its way, and once released it waits where it was let go until
//! the caller's new order shows, then glides into place. Shared by the
//! lists the user arranges by hand (Favorite wearables; the contact sets'
//! priority list moves onto it once both are merged).
//!
//! The caller allocates its rows (`Sense::click_and_drag`), calls `update`
//! with their keys in the order shown, paints each row at `Layout::top`
//! (the one `lifted` through `lifted_card`), and applies `Layout::moved` to
//! its own order. Every row's drawn place is remembered by key, so any
//! change of order (a drop applied, a sort, a row added) glides from where
//! the rows were drawn instead of jumping.

use crate::theme::Palette;
use egui::{Rect, Vec2};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// How fast the rows ease toward their places (per second).
const EASE: f32 = 22.0;
/// The band at the top and bottom of the visible list where a carried card
/// scrolls it, and the speed at its edge (points per second).
const SCROLL_EDGE: f32 = 28.0;
const SCROLL_SPEED: f32 = 600.0;
/// Frames a drop waits for the caller's new order before giving up.
const DROP_WAIT: u32 = 30;

/// A row carried: its key, where it stood, and where the pointer holds the
/// card (from the card's top).
#[derive(Clone, Debug)]
struct Carry<K> {
    key: K,
    from: usize,
    grab: f32,
}

/// A row released at `to`, its card at `card_top` (from the first row's
/// top), over `order` (the keys when it was released); `frames` waited.
#[derive(Clone, Debug)]
struct Dropped<K> {
    carry: Carry<K>,
    to: usize,
    card_top: f32,
    order: Vec<K>,
    frames: u32,
}

#[derive(Clone, Debug)]
struct State<K> {
    /// Where each row was drawn last frame, from the first row's top.
    drawn: HashMap<K, f32>,
    carry: Option<Carry<K>>,
    dropped: Option<Dropped<K>>,
    /// The frame the list was last drawn.
    frame: u64,
}

impl<K> Default for State<K> {
    fn default() -> Self {
        Self {
            drawn: HashMap::new(),
            carry: None,
            dropped: None,
            frame: 0,
        }
    }
}

/// One frame of the list.
#[derive(Debug, Default)]
pub struct Layout {
    /// The top of each row as drawn this frame (screen points).
    pub tops: Vec<f32>,
    /// The row drawn as the lifted card: carried, or dropped and waiting.
    pub lifted: Option<usize>,
    /// The row picked up this frame.
    pub started: Option<usize>,
    /// The row released this frame at another place: (from, to).
    pub moved: Option<(usize, usize)>,
}

impl Layout {
    /// While a row is carried or settling, the others take no hover.
    pub fn busy(&self) -> bool {
        self.lifted.is_some()
    }

    /// The rect a row is drawn at, given the rect it was allocated.
    pub fn rect(&self, i: usize, allocated: Rect) -> Rect {
        let top = self.tops.get(i).copied().unwrap_or(allocated.top());
        allocated.translate(Vec2::new(0.0, top - allocated.top()))
    }
}

/// The place a card whose top is `card_top` below the first row takes
/// among `n` rows one every `pitch`.
pub fn target_slot(card_top: f32, pitch: f32, n: usize) -> usize {
    if n == 0 || pitch <= 0.0 {
        return 0;
    }
    ((card_top / pitch).round().max(0.0) as usize).min(n - 1)
}

/// Where the row `i` stands while the row at `from` is carried to `to`:
/// the rows between make way, one step toward the place it left.
pub fn shifted_slot(i: usize, from: usize, to: usize) -> usize {
    if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

/// Where something dropped at `y` (screen) goes among `n` rows starting at
/// `top0`, one every `pitch`: before the row under the pointer when over
/// its upper half, after it over its lower half (0..=n).
pub fn insert_slot(y: f32, top0: f32, pitch: f32, n: usize) -> usize {
    if pitch <= 0.0 {
        return n;
    }
    (((y - top0) / pitch).round().max(0.0) as usize).min(n)
}

/// One frame of the list `keys`, whose rows were allocated as `rows` (same
/// order, one height, evenly spaced). `enabled` false lets no row be picked
/// up (and drops one carried); `gap` opens room before that row, for
/// something carried from elsewhere.
pub fn update<K>(ui: &egui::Ui, id: egui::Id, keys: &[K], rows: &[egui::Response], enabled: bool, gap: Option<usize>) -> Layout
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
{
    let n = keys.len().min(rows.len());
    if n == 0 {
        ui.ctx().data_mut(|d| d.remove::<State<K>>(id));
        return Layout::default();
    }
    let mut st: State<K> = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    // A list not drawn for a frame or more (its window closed, another
    // folder shown) forgets its carry: the button let go meanwhile would
    // otherwise read as a drop on its return, and its rows start in place.
    let frame = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
    if st.frame + 1 < frame {
        st = State::default();
    }
    st.frame = frame;
    let keys = &keys[..n];
    let top0 = rows[0].rect.top();
    let height = rows[0].rect.height();
    let pitch = if n > 1 { rows[1].rect.top() - top0 } else { height };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let down = ui.input(|i| i.pointer.primary_down());
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let mut layout = Layout::default();
    if !enabled {
        st.carry = None;
    }
    if enabled
        && st.carry.is_none()
        && let Some(i) = rows[..n].iter().position(|r| r.drag_started())
        // held where it was pressed, not where the drag began to count
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(pointer)
    {
        st.carry = Some(Carry {
            key: keys[i].clone(),
            from: i,
            grab: at.y - rows[i].rect.top(),
        });
        st.dropped = None;
        layout.started = Some(i);
    }
    // the carry ends if its row went away meanwhile
    if st.carry.as_ref().is_some_and(|carry| keys.get(carry.from) != Some(&carry.key)) {
        st.carry = None;
    }
    // a drop waits for the new order (or gives up): every row then glides
    // on from where it is drawn
    if let Some(drop) = st.dropped.as_mut() {
        drop.frames += 1;
        if drop.order != keys || drop.frames > DROP_WAIT {
            st.dropped = None;
        }
    }
    let clip = ui.clip_rect();
    // the row held (carried, or dropped and waiting): where it stood, the
    // top of its card, the place it takes
    let held: Option<(usize, f32, usize)> = match (&st.carry, &st.dropped) {
        (Some(carry), _) => pointer.map(|at| {
            // kept inside the visible list, which scrolls under it instead
            let top = (at.y - carry.grab).clamp(clip.top(), (clip.bottom() - height).max(clip.top())) - top0;
            (carry.from, top, target_slot(top, pitch, n))
        }),
        (None, Some(drop)) => Some((drop.carry.from, drop.card_top, drop.to)),
        _ => None,
    };
    let ease = 1.0 - (-dt * EASE).exp();
    let mut moving = false;
    for (i, key) in keys.iter().enumerate() {
        let carried = held.is_some_and(|(from, _, _)| from == i);
        let target = match held {
            Some((_, top, _)) if carried => top,
            Some((from, _, to)) => shifted_slot(i, from, to) as f32 * pitch,
            None => (i + usize::from(gap.is_some_and(|g| i >= g))) as f32 * pitch,
        };
        let was = st.drawn.get(key).copied().unwrap_or(target);
        // the card follows the pointer at once, the others glide
        let mut at = if carried { target } else { was + (target - was) * ease };
        if (target - at).abs() > 0.5 {
            moving = true;
        } else {
            at = target;
        }
        st.drawn.insert(key.clone(), at);
        layout.tops.push(top0 + at);
    }
    let present: HashSet<&K> = keys.iter().collect();
    st.drawn.retain(|key, _| present.contains(key));
    // released: the caller applies the move, the card waits for it
    if st.carry.is_some() && !down {
        if let (Some(carry), Some((from, card_top, to))) = (st.carry.take(), held)
            && to != from
        {
            layout.moved = Some((from, to));
            st.dropped = Some(Dropped {
                carry,
                to,
                card_top,
                order: keys.to_vec(),
                frames: 0,
            });
        }
        moving = true;
    }
    // near the top or bottom of the visible list, it scrolls under the card
    if st.carry.is_some()
        && let Some(at) = pointer
    {
        let push = if at.y < clip.top() + SCROLL_EDGE {
            (clip.top() + SCROLL_EDGE - at.y) / SCROLL_EDGE
        } else if at.y > clip.bottom() - SCROLL_EDGE {
            -(at.y - (clip.bottom() - SCROLL_EDGE)) / SCROLL_EDGE
        } else {
            0.0
        };
        if push != 0.0 {
            ui.scroll_with_delta(Vec2::new(0.0, push.clamp(-1.0, 1.0) * SCROLL_SPEED * dt));
            moving = true;
        }
    }
    layout.lifted = st
        .carry
        .as_ref()
        .map(|carry| carry.from)
        .or(st.dropped.as_ref().map(|drop| drop.carry.from));
    if layout.lifted.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if moving || layout.lifted.is_some() {
        ui.ctx().request_repaint();
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, st));
    layout
}

/// The card in the hand at `rect`, lifted over everything: its shadow, a
/// raised face, what `paint` draws on it, then a violet edge.
pub fn lifted_card(ui: &mut egui::Ui, p: &Palette, id: egui::Id, rect: Rect, paint: impl FnOnce(&mut egui::Ui)) {
    let layer = egui::LayerId::new(egui::Order::Tooltip, id.with("lifted"));
    let mut card = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(rect));
    card.set_clip_rect(ui.ctx().content_rect());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    card.painter()
        .rect_filled(rect.translate(Vec2::new(0.0, 4.0)).expand(2.0), radius, p.navy.gamma_multiply(0.45));
    card.painter().rect_filled(rect, radius, p.raised.lerp_to_gamma(p.ink, 0.06));
    paint(&mut card);
    card.painter()
        .rect_stroke(rect, radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
}

#[cfg(test)]
#[path = "reorder_tests.rs"]
mod tests;
