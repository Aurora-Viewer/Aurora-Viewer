//! Open-conversation order, shared by the workspace and the older tab styles.
//! Follows LLTabContainer's locked-tab rearrangement (lltabcontainer.cpp).
//! Aurora additionally keeps each conversation in its original category
//! (its kind, or the contact set shown apart that it belongs to, #1104).
//!
//! The entries of the list move apart where the dragged one would land
//! (#910), and an entry dropped outside the Conversations window pops its
//! conversation out into a window of its own there, as a tab of the Legacy
//! Viewer's IM container is torn off (`popout.rs`). The nearby chat's
//! entry, pinned, does not move in the list but drags out the same way.

use super::sidebar::{Section, section};
use crate::{theme::Palette, world::World};
use egui::{Pos2, Rect, Response, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

struct Row {
    id: Uuid,
    section: Section,
    rect: Rect,
    /// Where the entry stands with no gap open above it: the drop is found
    /// on these, so that the gap opening does not move what it points at.
    rest: Rect,
}

/// The entry being dragged: its conversation (None: the nearby chat), where
/// the pointer took it in the entry, and the entry's size.
#[derive(Clone, Copy, Debug)]
struct Grab {
    conversation: Option<Uuid>,
    offset: Vec2,
    size: Vec2,
}

/// Where a conversation dropped in the list goes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    /// Before the conversation, or after it (the last of its section).
    Move(Uuid, bool),
    /// Where it already is.
    Stay,
}

/// What a drag let go of asks: the conversation (None: the nearby chat) in
/// its own window, its top left corner at `pos`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PopOut {
    pub conversation: Option<Uuid>,
    pub pos: Pos2,
}

/// Where the dragged entry's ghost stands, and whether a drop there pops
/// the conversation out (the pointer outside the window).
#[derive(Clone, Copy, Debug)]
pub(super) struct Ghost {
    pub pos: Pos2,
    pub size: Vec2,
    pub out: bool,
}

/// `AURORA_DEMO_CONV_REORDER`: what the synthetic drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demo {
    /// `1`: the first conversation dropped after the next of its category.
    Drop,
    /// `hold`: the same drag, held over its landing place (the gap).
    Hold,
    /// `out`: the first conversation dropped at the left of the window.
    Out,
    /// `nearby`: the nearby chat dropped at the left of the window.
    Nearby,
}

impl Demo {
    pub fn parse(value: &str) -> Demo {
        match value.trim() {
            "hold" => Demo::Hold,
            "out" => Demo::Out,
            "nearby" => Demo::Nearby,
            _ => Demo::Drop,
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    order: Vec<Uuid>,
    grab: Option<Grab>,
    rows: Vec<Row>,
    /// The nearby chat's entry this frame.
    local: Option<Rect>,
    /// Where the gap stands, found on the last frame's pointer.
    gap: Option<Landing>,
    /// How open each gap is, 0 to 1: before (false) or after (true) an
    /// entry.
    gaps: HashMap<(Uuid, bool), f32>,
    /// The height of the gaps drawn so far this frame, above the next row.
    shift: f32,
    /// The pointer's last position: it may leave the viewer's window before
    /// the button is let go.
    last_pointer: Option<Pos2>,
}

impl State {
    pub fn begin(&mut self, ctx: &egui::Context, world: &World) {
        let mut remaining: HashSet<_> = world.social.ims.iter().map(|s| s.other).collect();
        self.order.retain(|id| remaining.remove(id));
        self.order
            .extend(world.social.ims.iter().map(|s| s.other).filter(|id| remaining.remove(id)));
        self.rows.clear();
        self.local = None;
        self.shift = 0.0;
        if self.grab.is_some()
            && (ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                || ctx.input(|i| !i.focused || (!i.pointer.primary_down() && !i.pointer.primary_released())))
        {
            self.cancel();
        }
    }

    fn cancel(&mut self) {
        self.grab = None;
        self.gap = None;
        self.last_pointer = None;
    }

    pub fn sessions(&self, world: &World) -> Vec<(Uuid, usize)> {
        let unread: HashMap<_, _> = world.social.ims.iter().map(|s| (s.other, s.unread)).collect();
        self.order.iter().filter_map(|id| unread.get(id).map(|n| (*id, *n))).collect()
    }

    /// The drag of an entry starts: where the pointer took it.
    fn take(&mut self, conversation: Option<Uuid>, response: &Response) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            // where the button went down: the drag starts a few points further
            let at = response
                .ctx
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos())
                .unwrap_or(response.rect.center());
            self.grab = Some(Grab {
                conversation,
                offset: at - response.rect.min,
                size: response.rect.size(),
            });
        }
    }

    pub fn row(&mut self, world: &World, id: Uuid, response: &Response) {
        self.take(Some(id), response);
        self.rows.push(Row {
            id,
            section: section(world, &id),
            rect: response.rect,
            rest: response.rect.translate(Vec2::new(0.0, -self.shift)),
        });
    }

    /// The nearby chat's entry: it drags out of the window, never within
    /// the list.
    pub fn local_row(&mut self, response: &Response) {
        self.take(None, response);
        self.local = Some(response.rect);
    }

    /// The conversation's entry is the one dragged (None: the nearby chat).
    pub fn dragging(&self, conversation: Option<Uuid>) -> bool {
        self.grab.is_some_and(|g| g.conversation == conversation)
    }

    /// The gap that opens before `id`'s entry, drawn before it.
    pub fn space_before(&mut self, ui: &mut egui::Ui, id: Uuid) {
        self.space(ui, (id, false));
    }

    /// The gap that opens after `id`'s entry, the last of its section,
    /// drawn after it.
    pub fn space_after(&mut self, ui: &mut egui::Ui, id: Uuid) {
        self.space(ui, (id, true));
    }

    fn space(&mut self, ui: &mut egui::Ui, key: (Uuid, bool)) {
        let open = self.gap == Some(Landing::Move(key.0, key.1));
        if !open && !self.gaps.contains_key(&key) {
            return;
        }
        let step = ui.input(|i| i.stable_dt.min(0.1)) / GAP_TIME;
        let t = self.gaps.entry(key).or_insert(0.0);
        *t = if open { (*t + step).min(1.0) } else { (*t - step).max(0.0) };
        let t = *t;
        if t <= 0.0 {
            self.gaps.remove(&key);
            return;
        }
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        let slot = self.grab.map_or(ENTRY_SLOT, |g| g.size.y) + ui.spacing().item_spacing.y;
        let h = slot * ease(t);
        ui.add_space(h);
        self.shift += h;
    }

    /// The ghost of the entry dragged, under the pointer where it took it.
    pub fn ghost(&self, ctx: &egui::Context) -> Option<Ghost> {
        let grab = self.grab?;
        let at = ctx.pointer_interact_pos().or(self.last_pointer)?;
        let out = ctx.data(|d| d.get_temp::<bool>(out_id())).unwrap_or(false);
        Some(Ghost {
            pos: at - grab.offset,
            size: grab.size,
            out,
        })
    }

    fn landing(&self, grab: Grab, at: Pos2, clip: Rect) -> Option<Landing> {
        let id = grab.conversation?;
        let source = self.rows.iter().find(|row| row.id == id)?;
        if !clip.contains(at) {
            return None;
        }
        // the section of the entry, as its entries stand without a gap: a
        // drop on a pinned entry, a section head or another section is none
        let same: Vec<&Row> = self.rows.iter().filter(|row| row.section == source.section).collect();
        let (first, last) = (same.first()?, same.last()?);
        let half = source.rest.height() * 0.5;
        if at.y < first.rest.top() - half || at.y > last.rest.bottom() + half {
            return None;
        }
        let others: Vec<&Row> = same.iter().copied().filter(|row| row.id != id).collect();
        let index = others.iter().filter(|row| row.rest.center().y < at.y).count();
        let now = same.iter().position(|row| row.id == id)?;
        Some(if index == now {
            Landing::Stay
        } else if let Some(row) = others.get(index) {
            Landing::Move(row.id, false)
        } else {
            Landing::Move(others.last()?.id, true)
        })
    }

    /// After the list's entries: the drag's gap, cursor and scrolling, and
    /// its drop. A drop outside the window asks for the conversation's own
    /// window there.
    pub fn finish(&mut self, ui: &mut egui::Ui, p: &Palette, demo: Option<Demo>) -> Option<PopOut> {
        // the window holding the list (the Conversations window)
        let window = ui.ctx().memory(|m| m.area_rect(ui.layer_id().id)).unwrap_or(ui.clip_rect());
        if demo.is_some() && self.grab.is_none() {
            let first = self.rows.first().map(|first| first.rect);
            let second = self
                .rows
                .first()
                .and_then(|first| self.rows.iter().skip(1).find(|row| row.section == first.section))
                .map(|row| row.rect);
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    demo_id(),
                    DemoRects {
                        demo: demo.unwrap_or(Demo::Drop),
                        first,
                        second,
                        local: self.local,
                        window,
                    },
                )
            });
        }
        let Some(grab) = self.grab else {
            self.gap = None;
            return None;
        };
        let source = match grab.conversation {
            Some(id) => self.rows.iter().find(|row| row.id == id).map(|row| row.rect),
            None => self.local,
        };
        let Some(source) = source else {
            self.cancel();
            return None;
        };
        let clip = ui.clip_rect();
        let pointer = ui.input(|i| i.pointer.interact_pos());
        if pointer.is_some() {
            self.last_pointer = pointer;
        }
        let at = pointer.or(self.last_pointer);
        let out = at.is_some_and(|at| !window.contains(at));
        ui.ctx().data_mut(|d| d.insert_temp(out_id(), out));
        let landing = if out {
            None
        } else {
            at.and_then(|at| self.landing(grab, at, clip))
        };
        self.gap = landing.filter(|l| *l != Landing::Stay);
        ui.painter()
            .rect_filled(source, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.12));
        if ui.input(|i| i.pointer.primary_released()) {
            let drop = if out {
                at.map(|at| PopOut {
                    conversation: grab.conversation,
                    pos: at - grab.offset,
                })
            } else {
                if let (Some(Landing::Move(target, after)), Some(id)) = (landing, grab.conversation) {
                    self.move_to(id, target, after);
                    // the entry takes the gap's place at once
                    self.gaps.clear();
                }
                None
            };
            self.cancel();
            return drop;
        }
        let valid = landing.is_some() || out || at.is_some_and(|at| source.contains(at));
        ui.ctx().set_cursor_icon(if valid {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
        if let Some(at) = pointer.filter(|at| clip.contains(*at)) {
            let edge = 24.0;
            let speed = if at.y < clip.top() + edge {
                (clip.top() + edge - at.y) / edge
            } else if at.y > clip.bottom() - edge {
                -(at.y - clip.bottom() + edge) / edge
            } else {
                0.0
            };
            let dt = ui.input(|i| i.stable_dt.min(0.1));
            ui.scroll_with_delta(egui::vec2(0.0, speed * 420.0 * dt));
        }
        ui.ctx().request_repaint();
        None
    }

    fn move_to(&mut self, source: Uuid, target: Uuid, after: bool) {
        let Some(from) = self.order.iter().position(|id| *id == source) else {
            return;
        };
        self.order.remove(from);
        if let Some(to) = self.order.iter().position(|id| *id == target) {
            self.order.insert(to + usize::from(after), source);
        } else {
            self.order.insert(from, source);
        }
    }
}

/// How long the gap takes to open or close, in seconds.
const GAP_TIME: f32 = 0.12;
/// An entry's height before one is dragged (the workspace's column).
const ENTRY_SLOT: f32 = 44.0;

/// Smooth start and end of the gap's opening.
fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Whether the ghost stands outside the window this frame (`finish`).
fn out_id() -> egui::Id {
    egui::Id::unique("conversation_drag_out")
}

fn demo_id() -> egui::Id {
    egui::Id::unique("conversation_reorder_demo")
}

/// What the synthetic drag aims at, kept by `finish`.
#[derive(Clone, Copy)]
struct DemoRects {
    demo: Demo,
    first: Option<Rect>,
    second: Option<Rect>,
    local: Option<Rect>,
    window: Rect,
}

/// Synthetic pointer events, enabled only by the explicit offline fixture:
/// pressed at frame 701, moved for 19 frames, let go at frame 760 (held on
/// with `hold`).
pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if !(700..=760).contains(&frame) {
        return Vec::new();
    }
    let Some(rects) = ctx.data(|d| d.get_temp::<DemoRects>(demo_id())) else {
        return Vec::new();
    };
    let demo = rects.demo;
    let source = match demo {
        Demo::Nearby => rects.local,
        _ => rects.first,
    };
    let Some(source) = source else {
        return Vec::new();
    };
    let start = egui::pos2(source.left() + source.width() * 0.4, source.center().y);
    let end = match demo {
        Demo::Drop | Demo::Hold => {
            let Some(second) = rects.second else {
                return Vec::new();
            };
            // held, the ghost stands in the gap that opens under the second
            let y = if demo == Demo::Hold {
                second.bottom() + (source.center().y - source.top()) + 2.0
            } else {
                second.bottom() - 6.0
            };
            egui::pos2(second.left() + second.width() * 0.4, y)
        }
        // at the left of the window, where there is room
        Demo::Out | Demo::Nearby => egui::pos2((rects.window.left() - 220.0).max(60.0), rects.window.top() + 60.0),
    };
    let pos = start.lerp(end, (frame.saturating_sub(701) as f32 / 19.0).min(1.0));
    let mut events = vec![egui::Event::PointerMoved(pos)];
    if frame == 701 || (frame == 760 && demo != Demo::Hold) {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: frame == 701,
            modifiers: egui::Modifiers::NONE,
        });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);
    const C: Uuid = Uuid::from_u128(3);

    /// Three private conversations at rest, 44 high, 2 apart, from y 100.
    fn state(grabbed: Uuid) -> (State, Grab) {
        let mut st = State {
            order: vec![A, B, C],
            ..State::default()
        };
        for (n, id) in [A, B, C].into_iter().enumerate() {
            let rect = Rect::from_min_size(egui::pos2(10.0, 100.0 + 46.0 * n as f32), egui::vec2(200.0, 44.0));
            st.rows.push(Row {
                id,
                section: Section::Kind(super::super::sidebar::Kind::Private),
                rect,
                rest: rect,
            });
        }
        let grab = Grab {
            conversation: Some(grabbed),
            offset: egui::vec2(20.0, 22.0),
            size: egui::vec2(200.0, 44.0),
        };
        st.grab = Some(grab);
        (st, grab)
    }

    const CLIP: Rect = Rect {
        min: egui::pos2(0.0, 0.0),
        max: egui::pos2(400.0, 600.0),
    };

    #[test]
    fn the_landing_is_found_on_the_entries_at_rest() {
        let (st, grab) = state(A);
        // over its own place, then between B and C, then below C
        assert_eq!(st.landing(grab, egui::pos2(50.0, 120.0), CLIP), Some(Landing::Stay));
        assert_eq!(st.landing(grab, egui::pos2(50.0, 150.0), CLIP), Some(Landing::Stay));
        assert_eq!(st.landing(grab, egui::pos2(50.0, 170.0), CLIP), Some(Landing::Move(C, false)));
        assert_eq!(st.landing(grab, egui::pos2(50.0, 230.0), CLIP), Some(Landing::Move(C, true)));
        // half an entry past the section still lands at its end, further is none
        assert_eq!(st.landing(grab, egui::pos2(50.0, 250.0), CLIP), Some(Landing::Move(C, true)));
        assert_eq!(st.landing(grab, egui::pos2(50.0, 280.0), CLIP), None);
        assert_eq!(st.landing(grab, egui::pos2(50.0, 60.0), CLIP), None);
        // C taken up above A
        let (st, grab) = state(C);
        assert_eq!(st.landing(grab, egui::pos2(50.0, 105.0), CLIP), Some(Landing::Move(A, false)));
        assert_eq!(st.landing(grab, egui::pos2(50.0, 200.0), CLIP), Some(Landing::Stay));
        // outside the list's clip
        assert_eq!(st.landing(grab, egui::pos2(500.0, 105.0), CLIP), None);
    }

    #[test]
    fn a_gap_drawn_above_an_entry_does_not_move_its_rest() {
        let (mut st, _) = state(A);
        st.rows.clear();
        st.shift = 46.0;
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        frames.run(Vec::new(), |ui| {
            let rect = Rect::from_min_size(egui::pos2(10.0, 146.0), egui::vec2(200.0, 44.0));
            let resp = ui.interact(rect, egui::Id::unique("reorder_test_row"), egui::Sense::click_and_drag());
            st.row(&world, B, &resp);
        });
        assert_eq!(st.rows[0].rest.top(), 100.0);
        assert_eq!(st.rows[0].rect.top(), 146.0);
    }

    #[test]
    fn the_nearby_chat_never_lands_in_the_list() {
        let (mut st, mut grab) = state(A);
        grab.conversation = None;
        st.grab = Some(grab);
        assert_eq!(st.landing(grab, egui::pos2(50.0, 170.0), CLIP), None);
        assert!(st.dragging(None) && !st.dragging(Some(A)));
    }

    #[test]
    fn the_demo_modes_parse() {
        assert_eq!(Demo::parse("1"), Demo::Drop);
        assert_eq!(Demo::parse("hold"), Demo::Hold);
        assert_eq!(Demo::parse(" out "), Demo::Out);
        assert_eq!(Demo::parse("nearby"), Demo::Nearby);
    }
}
