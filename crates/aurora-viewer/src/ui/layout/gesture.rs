//! The gestures on the windows: a window dragged by its title or resized by
//! an edge is offered a snap (`snap::find`), the windows snapped side by
//! side are glued and move as one block. Moving only offers a snap while
//! Alt is held; Alt also takes the dragged window out of its group.
//!
//! egui moves the dragged window itself (`egui::Area`: where it stood when
//! the drag began plus the travel of the pointer, kept inside the work area).
//! What is added around it each frame, before the windows are shown: the
//! position of the windows glued to it, the limits that keep the whole
//! block inside the work area, the snap offered and, when the mouse is
//! released, its application.

use super::{Live, Shown, Windows};
use crate::ui::snap::{self, Snap, overlay};
use egui::{Id, Pos2, Rect, Vec2};

/// What the pointer does this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Pointer {
    /// The widget dragged, and the one whose drag ended this frame.
    pub dragged: Option<Id>,
    pub stopped: Option<Id>,
    /// How far the pointer went since it was pressed.
    pub total: Vec2,
    /// Alt held: the window leaves its group and can snap to a new target.
    pub alt: bool,
    pub time: f64,
}

/// A window dragged by its title.
#[derive(Clone, Debug)]
pub(super) struct Move {
    /// The dragged window first, then the windows glued to it, each where
    /// it stood when the drag began.
    start: Vec<(u64, Rect)>,
    /// The other windows, as the snap of the last frame was found among
    /// them.
    others: Vec<Shown>,
    snap: Snap,
    began: f64,
    /// Alt was held during this drag: the window left its group.
    detached: bool,
}

/// A window resized by an edge, from one frame to the next.
#[derive(Clone, Debug)]
pub(super) struct Resize {
    window: u64,
    snap: Snap,
    began: f64,
    frame: u64,
}

pub(super) struct ResizeRequest {
    pub window: u64,
    /// Where the pointer has the window.
    pub rect: Rect,
    pub edges: snap::Edges,
    pub min_size: Vec2,
    pub alt: bool,
    pub time: f64,
    /// The mouse button is up: the last frame of the gesture.
    pub released: bool,
}

/// The id of the widget egui drags a window by (`egui::Area`).
fn move_id(window: Id) -> Id {
    window.with("move")
}

/// `window` kept inside `limits` as egui does it for a window it moves
/// (Context::constrain_window_rect_to_area, then the position rounded to
/// pixels): where the dragged window is this frame, known before it is
/// shown.
fn constrained(window: Rect, limits: Rect, pixels_per_point: f32) -> Pos2 {
    use egui::emath::GuiRounding as _;
    let window = window.round_ui();
    let margin = (window.size() - limits.size()).max(Vec2::ZERO);
    let x = window
        .left()
        .min(limits.right() + margin.x - window.width())
        .max(limits.left() - margin.x);
    let y = window
        .top()
        .min(limits.bottom() + margin.y - window.height())
        .max(limits.top() - margin.y);
    Rect::from_min_size(Pos2::new(x, y), window.size())
        .round_ui()
        .min
        .round_to_pixels(pixels_per_point)
        .round_ui()
}

impl Live {
    /// The windows shown in the frame before `frame`, or since.
    fn open(&self, frame: u64) -> Vec<Shown> {
        self.shown.values().filter(|s| s.frame + 1 >= frame).copied().collect()
    }
}

impl Windows {
    /// One frame of the gestures; see `layout::frame`. `area`: the work
    /// area, what a window snaps to; `room`: where it moves
    /// (`layout::move_area`), `edges` stopping it.
    pub(super) fn drive(
        &mut self,
        area: Rect,
        room: Rect,
        edges: super::Edges,
        pixels_per_point: f32,
        pointer: Pointer,
        snap_enabled: bool,
    ) -> Option<overlay::Overlay> {
        self.live.frame += 1;
        self.live.snap = snap_enabled;
        self.live.places.clear();
        self.live.limits = None;
        let frame = self.live.frame;
        // closed windows leave
        self.live.shown.retain(|_, s| s.frame + 1 >= frame);
        let open = self.live.open(frame);
        if open.is_empty() {
            // no window at all: the login screen, a hidden interface. The
            // groups wait for their windows.
            self.live.moved = None;
            return None;
        }
        let find = |id: u64| open.iter().find(|s| s.id.value() == id).copied();
        // glued windows are open, expanded and still side by side: closing
        // or minimizing one, or pulling an edge away, frees it
        self.links.retain(|(a, b)| match (find(*a), find(*b)) {
            (Some(a), Some(b)) => !a.minimized && !b.minimized && snap::touching(a.rect, b.rect),
            _ => false,
        });
        if pointer.dragged.is_none() {
            self.refresh_anchors(room);
        }

        if self.live.moved.is_none()
            && let Some(window) = open.iter().find(|s| pointer.dragged == Some(move_id(s.id)))
        {
            let start = snap::group(&self.links, window.id.value())
                .into_iter()
                .filter_map(|id| find(id).map(|s| (id, s.rect)))
                .collect();
            self.live.moved = Some(Move {
                start,
                others: Vec::new(),
                snap: Snap::default(),
                began: pointer.time,
                detached: false,
            });
        }
        let mut moved = self.live.moved.take()?;
        let (window, start) = *moved.start.first()?;
        let shown = find(window)?;
        let dragged = pointer.dragged == Some(move_id(shown.id));
        let released = pointer.stopped == Some(move_id(shown.id));
        if !dragged && !released {
            return None;
        }
        if pointer.alt && !moved.detached {
            moved.detached = true;
            self.links.retain(|(a, b)| *a != window && *b != window);
            self.live.reanchor.extend(moved.start.iter().map(|(id, _)| *id));
            moved.start.truncate(1);
        }
        let in_group = |id: u64| moved.start.iter().any(|(member, _)| *member == id);

        if released {
            self.live.reanchor.extend(moved.start.iter().map(|(id, _)| *id));
            // egui leaves the windows where the last frame had them, and the
            // snap shown then is the one applied
            let snap = if snap_enabled && pointer.alt { moved.snap } else { Snap::default() };
            if snap.is_none() {
                return None;
            }
            for (id, _) in &moved.start {
                if let Some(member) = find(*id) {
                    self.live.places.insert(*id, member.rect.min + snap.shift());
                }
            }
            for (mover, other) in snap.adjacent() {
                let (Some((id, _)), Some(other)) = (moved.start.get(mover), moved.others.get(other)) else {
                    continue;
                };
                let Some(member) = find(*id) else { continue };
                let landed = member.rect.translate(snap.shift());
                if !member.minimized && !other.minimized && snap::touching(landed, other.rect) {
                    self.links.insert(snap::link(*id, other.id.value()));
                }
            }
            return None;
        }

        // where egui has the dragged window this frame: alone, within reach
        // in the room (`layout::reach`, as `widgets::Floater` constrains
        // it); a group stays whole inside the room, so the window stops
        // where its block does
        let block = moved.start.iter().fold(Rect::NOTHING, |all, (_, rect)| all.union(*rect));
        let limits = if moved.start.len() > 1 {
            let limits = snap::block_limits(room, start, block);
            self.live.limits = Some((window, limits));
            limits
        } else {
            super::reach(room, start.size(), if shown.partial_outside { super::Edges::OPEN } else { edges })
        };
        let travel = constrained(start.translate(pointer.total), limits, pixels_per_point) - start.min;
        for (id, rect) in moved.start.iter().skip(1) {
            self.live.places.insert(*id, rect.min + travel);
        }
        let movers: Vec<Rect> = moved.start.iter().map(|(_, rect)| rect.translate(travel)).collect();
        moved.others = open.iter().filter(|s| !in_group(s.id.value())).copied().collect();
        let others: Vec<Rect> = moved.others.iter().map(|s| s.rect).collect();
        let scene = snap::Scene {
            movers: &movers,
            edges: snap::Edges::ALL,
            others: &others,
            area,
            min_size: Vec2::ZERO,
            max_size: Vec2::INFINITY,
        };
        // Deliberate difference from Firestorm's LLDragHandle::handleHover:
        // only an Alt drag can attach a window, avoiding accidental groups.
        moved.snap = if snap_enabled && pointer.alt {
            snap::find(&scene, moved.snap)
        } else {
            Snap::default()
        };
        let hint = if pointer.alt {
            None
        } else if movers.len() > 1 {
            Some(overlay::Hint::Detach)
        } else if snap_enabled {
            Some(overlay::Hint::Snap)
        } else {
            None
        };
        let guide = if moved.snap.is_none() {
            snap::Guide::default()
        } else {
            snap::guide(&scene, moved.snap)
        };
        let began = moved.began;
        self.live.moved = Some(moved);
        (hint.is_some() || !guide.lines.is_empty()).then_some(overlay::Overlay {
            guide,
            window: movers[0],
            hint,
            began,
        })
    }

    /// One frame of a resize; see `layout::resizing`.
    pub(super) fn resize(&mut self, request: ResizeRequest, area: Rect) -> (Rect, Option<overlay::Overlay>) {
        if request.released {
            self.live.reanchor.insert(request.window);
        }
        let frame = self.live.frame;
        let previous = self
            .live
            .resized
            .take()
            .filter(|r| r.window == request.window && r.frame + 1 >= frame);
        if !self.live.snap || request.alt {
            return (request.rect, None);
        }
        let others: Vec<Shown> = self
            .live
            .open(frame)
            .into_iter()
            .filter(|s| s.id.value() != request.window)
            .collect();
        let rects: Vec<Rect> = others.iter().map(|s| s.rect).collect();
        let movers = [request.rect];
        let scene = snap::Scene {
            movers: &movers,
            edges: request.edges,
            others: &rects,
            area,
            min_size: request.min_size,
            max_size: Vec2::INFINITY,
        };
        let snap = snap::find(&scene, previous.as_ref().map_or(Snap::default(), |r| r.snap));
        let rect = snap::apply(request.rect, request.edges, snap.shift());
        if request.released {
            // its edge left against a neighbor: the two are glued
            for (_, other) in snap.adjacent() {
                if let Some(other) = others.get(other)
                    && !other.minimized
                    && snap::touching(rect, other.rect)
                {
                    self.links.insert(snap::link(request.window, other.id.value()));
                }
            }
            return (rect, None);
        }
        let began = previous.map_or(request.time, |r| r.began);
        self.live.resized = Some(Resize {
            window: request.window,
            snap,
            began,
            frame,
        });
        if snap.is_none() {
            return (rect, None);
        }
        // the window is already where the snap has it: only the edges
        // that met are shown
        let mut guide = snap::guide(&scene, snap);
        guide.landing.clear();
        let overlay = overlay::Overlay {
            guide,
            window: rect,
            hint: Some(overlay::Hint::NoSnap),
            began,
        };
        (rect, Some(overlay))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{GAP, default_work_area, frame, restore, set_work_area, windows};
    use crate::ui::snap;
    use egui::{Id, Pos2, Rect, Vec2};

    const A: &str = "gesture_a";
    const B: &str = "gesture_b";
    const MAP: &str = "gesture_map";

    /// Two floaters and a bare window (as the mini-map is), with the
    /// gestures of the frame, driven by pointer events.
    struct Session {
        ctx: egui::Context,
        frame: usize,
        screen: Vec2,
        snap: bool,
        alt: bool,
        b_open: bool,
    }

    impl Session {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                frame: 0,
                screen: Vec2::new(1600.0, 900.0),
                snap: true,
                alt: false,
                b_open: true,
            }
        }

        /// The session that follows, restored from the layout of this one.
        fn restart(&self) -> Session {
            let saved = ron::to_string(&self.ctx.memory(|m| m.clone())).expect("serialize");
            let next = Session::new();
            restore(&next.ctx, &saved).expect("restore");
            next
        }

        fn run(&mut self, mut events: Vec<egui::Event>) {
            self.frame += 1;
            let modifiers = egui::Modifiers {
                alt: self.alt,
                ..Default::default()
            };
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let palette = crate::theme::Theme::default().palette();
            let screen = Rect::from_min_size(Pos2::ZERO, self.screen);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            };
            let b_open = self.b_open;
            let snap = self.snap;
            let mut output = self.ctx.run_ui(input, |ui| {
                let ctx = ui.ctx();
                set_work_area(ctx, default_work_area(screen));
                frame(ctx, &palette, snap);
                let floater = |id, pos, open: bool| {
                    crate::ui::widgets::Floater::new(id, "Window", pos, Vec2::new(300.0, 200.0))
                        .silent()
                        .show(ctx, &palette, &mut { open }, |ui| {
                            ui.label("content");
                        });
                };
                floater(A, Pos2::new(200.0, 200.0), true);
                floater(B, Pos2::new(800.0, 400.0), b_open);
                let default = Rect::from_min_size(Pos2::new(1200.0, 100.0), Vec2::splat(200.0));
                let map_frame = egui::Frame::new().inner_margin(egui::Margin::same(2));
                crate::ui::widgets::bare_window(ctx, &palette, MAP, "Map", map_frame, default, Vec2::splat(64.0), |ui| {
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
                });
            });
            output.textures_delta.clear();
        }

        fn idle(&mut self, frames: usize) {
            for _ in 0..frames {
                self.run(Vec::new());
            }
        }

        fn rect(&self, id: &str) -> Rect {
            self.ctx.memory(|m| m.area_rect(Id::unique(id))).expect("the window was shown")
        }

        fn linked(&self, a: &str, b: &str) -> bool {
            let link = snap::link(Id::unique(a).value(), Id::unique(b).value());
            windows(&self.ctx, |w| w.links.contains(&link))
        }

        fn button(pos: Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }
        }

        /// Press at `from` and hold the pointer at `to`.
        fn press_and_hold(&mut self, from: Pos2, to: Pos2) {
            self.run(vec![egui::Event::PointerMoved(from), Self::button(from, true)]);
            self.run(vec![egui::Event::PointerMoved(from.lerp(to, 0.5))]);
            for _ in 0..3 {
                self.run(vec![egui::Event::PointerMoved(to)]);
            }
        }

        fn release(&mut self, at: Pos2) {
            self.run(vec![Self::button(at, false)]);
            self.idle(3);
        }

        /// The title bar of a floater, where it is dragged by (past its
        /// title: egui's own style lets the text of a label be selected).
        fn title(&self, id: &str) -> Pos2 {
            self.rect(id).min + Vec2::new(170.0, 16.0)
        }

        /// Drag window `id` by its title so that its top left corner would
        /// be at `to`, and hold it there.
        fn hold_at(&mut self, id: &str, to: Pos2) -> Pos2 {
            let from = self.title(id);
            let target = from + (to - self.rect(id).min);
            self.press_and_hold(from, target);
            target
        }

        fn drop_at(&mut self, id: &str, to: Pos2) {
            let at = self.hold_at(id, to);
            self.release(at);
        }

        /// B dropped against the right side of A: the two are glued.
        fn glue(&mut self) {
            self.idle(3);
            let a = self.rect(A);
            self.alt = true;
            self.drop_at(B, Pos2::new(a.right() + GAP + 6.0, a.top() + 5.0));
            self.alt = false;
            assert!(self.linked(A, B));
        }
    }

    /// The window follows the pointer freely; the snap is applied when the
    /// mouse is released, and the two windows are glued.
    #[test]
    fn a_window_dropped_near_another_with_alt_snaps_to_it() {
        let mut s = Session::new();
        s.idle(3);
        let a = s.rect(A);
        let near = Pos2::new(a.right() + GAP + 6.0, a.top() + 5.0);
        s.alt = true;
        let at = s.hold_at(B, near);
        assert_eq!(s.rect(B).min, near, "no jump under the pointer");
        assert!(!s.linked(A, B));
        s.release(at);
        assert_eq!(s.rect(B).min, Pos2::new(a.right() + GAP, a.top()));
        assert_eq!(s.rect(A), a);
        assert!(s.linked(A, B));
    }

    #[test]
    fn a_window_dropped_far_from_everything_stays_there() {
        let mut s = Session::new();
        s.idle(3);
        let a = s.rect(A);
        s.drop_at(B, Pos2::new(900.0, 520.0));
        assert_eq!(s.rect(B).min, Pos2::new(900.0, 520.0));
        assert_eq!(s.rect(A), a);
        assert!(windows(&s.ctx, |w| w.links.is_empty()));
    }

    #[test]
    fn a_completed_move_replaces_the_anchor_and_keeps_its_offset_on_viewer_resize() {
        let mut s = Session::new();
        s.idle(3);
        let saved = windows(&s.ctx, |w| w.placed[&Id::unique(A).value()].anchor);
        let at = s.hold_at(A, Pos2::new(650.0, 80.0));
        assert_eq!(
            windows(&s.ctx, |w| w.placed[&Id::unique(A).value()].anchor),
            saved,
            "choose only on release"
        );
        s.release(at);
        let rect = s.rect(A);
        assert_eq!(rect.min, Pos2::new(650.0, 80.0), "invisible anchoring does not snap the window");
        let anchor = windows(&s.ctx, |w| w.placed[&Id::unique(A).value()].anchor.expect("released"));
        assert_eq!(anchor.point, egui::Align2::CENTER_TOP);
        s.screen = Vec2::new(1920.0, 1080.0);
        s.idle(3);
        assert_eq!(s.rect(A), rect.translate(Vec2::new(160.0, 0.0)));
        assert_eq!(windows(&s.ctx, |w| w.placed[&Id::unique(A).value()].anchor), Some(anchor));
        // A later move can select a different side midpoint.
        s.drop_at(A, Pos2::new(810.0, 800.0));
        let rect = s.rect(A);
        let anchor = windows(&s.ctx, |w| w.placed[&Id::unique(A).value()].anchor.expect("released again"));
        assert_eq!(anchor.point, egui::Align2::CENTER_BOTTOM);
        s.screen = Vec2::new(2240.0, 1200.0);
        s.idle(3);
        assert_eq!(s.rect(A), rect.translate(Vec2::new(160.0, 120.0)));
    }

    #[test]
    fn the_work_area_snaps_a_window_and_glues_nothing() {
        let mut s = Session::new();
        s.idle(3);
        let work = default_work_area(Rect::from_min_size(Pos2::ZERO, s.screen));
        s.alt = true;
        s.drop_at(A, Pos2::new(work.left() + GAP + 5.0, work.top() + GAP + 6.0));
        assert_eq!(s.rect(A).min, work.left_top() + Vec2::splat(GAP));
        assert!(windows(&s.ctx, |w| w.links.is_empty()));
    }

    #[test]
    fn a_window_dropped_without_alt_neither_snaps_nor_attaches() {
        let mut s = Session::new();
        s.idle(3);
        let a = s.rect(A);
        // Near a target and exactly touching it must both leave B independent.
        for offset in [Vec2::new(6.0, 5.0), Vec2::ZERO] {
            let near = Pos2::new(a.right() + GAP, a.top()) + offset;
            s.drop_at(B, near);
            assert_eq!(s.rect(B).min, near);
            assert!(!s.linked(A, B));
        }
        let b = s.rect(B);
        s.drop_at(A, a.min + Vec2::new(70.0, 50.0));
        assert_eq!(s.rect(B), b, "merely touching windows do not move together");
    }

    #[test]
    fn the_snap_setting_still_disables_an_alt_drag() {
        let mut s = Session::new();
        s.idle(3);
        let a = s.rect(A);
        let near = Pos2::new(a.right() + GAP + 6.0, a.top() + 5.0);
        s.snap = false;
        s.alt = true;
        s.drop_at(B, near);
        assert_eq!(s.rect(B).min, near);
        assert!(!s.linked(A, B));
    }

    #[test]
    fn releasing_alt_before_the_mouse_cancels_the_offered_snap() {
        let mut s = Session::new();
        s.idle(3);
        let a = s.rect(A);
        let near = Pos2::new(a.right() + GAP + 6.0, a.top() + 5.0);
        s.alt = true;
        let at = s.hold_at(B, near);
        s.alt = false;
        s.release(at);
        assert_eq!(s.rect(B).min, near);
        assert!(!s.linked(A, B));
    }

    #[test]
    fn pressing_alt_during_a_drag_enables_attachment() {
        let mut s = Session::new();
        s.idle(3);
        let a = s.rect(A);
        let at = s.hold_at(B, Pos2::new(a.right() + GAP + 6.0, a.top() + 5.0));
        s.alt = true;
        s.idle(2);
        s.release(at);
        assert_eq!(s.rect(B).min, Pos2::new(a.right() + GAP, a.top()));
        assert!(s.linked(A, B));
    }

    #[test]
    fn glued_windows_move_as_one_block() {
        let mut s = Session::new();
        s.glue();
        let (a, b) = (s.rect(A), s.rect(B));
        // either of them takes the other along, during the drag already
        for (dragged, travel) in [(A, Vec2::new(70.0, 50.0)), (B, Vec2::new(-40.0, 130.0))] {
            let (a, b) = (s.rect(A), s.rect(B));
            let start = s.rect(dragged).min;
            let at = s.hold_at(dragged, start + travel);
            assert_eq!(s.rect(A), a.translate(travel), "{dragged} held");
            assert_eq!(s.rect(B), b.translate(travel), "{dragged} held");
            s.release(at);
            assert_eq!(s.rect(A), a.translate(travel), "{dragged} dropped");
            assert_eq!(s.rect(B), b.translate(travel), "{dragged} dropped");
            assert!(s.linked(A, B));
        }
        assert_eq!(s.rect(B).min - s.rect(A).min, b.min - a.min);
    }

    /// The block stops at the edge of the work area as a whole (the bars
    /// are not covered): the window dragged does not go on alone.
    #[test]
    fn a_block_stays_whole_at_the_edge_of_the_screen() {
        let mut s = Session::new();
        s.glue();
        let offset = s.rect(B).min - s.rect(A).min;
        let work = default_work_area(Rect::from_min_size(Pos2::ZERO, s.screen));
        // A dragged far to the right: B, on its right, reaches the edge first
        let start = s.rect(A).min;
        let at = s.hold_at(A, start + Vec2::new(2000.0, 0.0));
        assert_eq!(s.rect(B).right(), 1600.0);
        assert_eq!(s.rect(B).min - s.rect(A).min, offset);
        // dropped there, the block snaps to the work area
        s.release(at);
        assert_eq!(s.rect(B).right(), work.right() - GAP);
        assert_eq!(s.rect(B).min - s.rect(A).min, offset);
        // B dragged far to the left and up: A reaches the corner first
        let start = s.rect(B).min;
        let at = s.hold_at(B, start + Vec2::new(-3000.0, -3000.0));
        assert_eq!(s.rect(A).min, work.min);
        assert_eq!(s.rect(B).min - s.rect(A).min, offset);
        // dropped in the corner of the work area, under the top bar
        s.release(at);
        assert_eq!(s.rect(A).min, Pos2::new(work.left() + GAP, work.top() + GAP));
        assert_eq!(s.rect(B).min - s.rect(A).min, offset);
        assert!(s.linked(A, B));
    }

    #[test]
    fn alt_takes_a_window_out_of_its_group() {
        let mut s = Session::new();
        s.glue();
        let b = s.rect(B);
        let start = s.rect(A).min;
        s.alt = true;
        s.drop_at(A, start + Vec2::new(-60.0, 200.0));
        s.alt = false;
        s.idle(2);
        assert_eq!(s.rect(A).min, start + Vec2::new(-60.0, 200.0));
        assert_eq!(s.rect(B), b);
        assert!(!s.linked(A, B));
        // free: the next drag moves it alone too
        s.drop_at(A, Pos2::new(100.0, 600.0));
        assert_eq!(s.rect(B), b);
    }

    #[test]
    fn an_alt_drag_detaches_a_window_and_attaches_it_to_another() {
        let mut s = Session::new();
        s.glue();
        let a = s.rect(A);
        let map = s.rect(MAP);
        s.alt = true;
        s.drop_at(B, Pos2::new(map.left() - s.rect(B).width() - GAP - 6.0, map.top() + 5.0));
        assert_eq!(s.rect(A), a);
        assert!(!s.linked(A, B));
        assert!(s.linked(B, MAP));
        s.alt = false;
        let b = s.rect(B);
        s.drop_at(B, b.min + Vec2::new(-60.0, 100.0));
        assert_eq!(s.rect(MAP), map.translate(Vec2::new(-60.0, 100.0)));
        assert_eq!(s.rect(A), a);
    }

    #[test]
    fn closing_or_minimizing_a_window_frees_it() {
        let mut s = Session::new();
        s.glue();
        s.b_open = false;
        s.idle(3);
        assert!(!s.linked(A, B));
        s.b_open = true;
        s.idle(3);
        let b = s.rect(B);
        let start = s.rect(A).min;
        s.drop_at(A, start + Vec2::new(-80.0, 300.0));
        assert_eq!(s.rect(B), b);

        let mut s = Session::new();
        s.glue();
        s.ctx.data_mut(|d| d.insert_persisted(Id::unique((B, "minimized")), true));
        s.idle(3);
        assert!(!s.linked(A, B));
    }

    /// A group shares the perimeter anchor nearest to its bounds, so its
    /// windows stay side by side through viewer resizing.
    #[test]
    fn a_group_follows_the_edges_as_a_block() {
        let mut s = Session::new();
        s.screen = Vec2::new(1280.0, 720.0);
        s.idle(3);
        // A against the left half, B on its right, past the middle
        s.drop_at(A, Pos2::new(330.0, 200.0));
        let a = s.rect(A);
        s.alt = true;
        s.drop_at(B, Pos2::new(a.right() + GAP + 4.0, a.top()));
        s.alt = false;
        assert!(s.linked(A, B));
        let (a, b) = (s.rect(A), s.rect(B));
        let screen = Rect::from_min_size(Pos2::ZERO, s.screen);
        let wide = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        let travel = Vec2::new((wide.width() - screen.width()) * 0.5, 0.0);
        s.screen = wide.size();
        s.idle(3);
        assert_eq!(
            (s.rect(A), s.rect(B)),
            (a.translate(travel), b.translate(travel)),
            "the block follows the top midpoint"
        );
        assert!(s.linked(A, B));
        // Near the right midpoint, both follow its horizontal and vertical
        // movement while keeping their individual offsets.
        let start = s.rect(B).min;
        s.drop_at(B, start + Vec2::new(2000.0, 0.0));
        let (a, b) = (s.rect(A), s.rect(B));
        assert_eq!(b.right(), 1600.0 - GAP);
        assert_eq!(
            windows(&s.ctx, |w| w.placed[&Id::unique(A).value()].anchor.expect("group anchor").point),
            egui::Align2::RIGHT_CENTER
        );
        s.screen = Vec2::new(1920.0, 1000.0);
        s.idle(3);
        assert_eq!(
            (s.rect(A), s.rect(B)),
            (a.translate(Vec2::new(320.0, 50.0)), b.translate(Vec2::new(320.0, 50.0)))
        );
        assert!(s.linked(A, B));
    }

    /// The groups are saved with the layout: glued windows are still glued
    /// in the next session.
    #[test]
    fn groups_survive_a_restart() {
        let mut first = Session::new();
        first.glue();
        let (a, b) = (first.rect(A), first.rect(B));
        let saved = windows(&first.ctx, |w| ron::to_string(w).expect("serialize"));
        let back: super::Windows = ron::from_str(&saved).expect("deserialize");
        assert_eq!(back.links, windows(&first.ctx, |w| w.links.clone()));
        assert_eq!(back.links.len(), 1);
        assert_eq!(back.placed, windows(&first.ctx, |w| w.placed.clone()));

        let mut next = first.restart();
        next.idle(4);
        assert_eq!((next.rect(A), next.rect(B)), (a, b));
        assert!(next.linked(A, B));
        let start = next.rect(A).min;
        next.drop_at(A, start + Vec2::new(90.0, 60.0));
        assert_eq!(next.rect(B), b.translate(Vec2::new(90.0, 60.0)));
    }

    /// An edge dragged near another window snaps under the pointer, and the
    /// two are glued once the mouse is released; pulled away again, they
    /// are free.
    #[test]
    fn a_resized_edge_snaps_and_glues() {
        let mut s = Session::new();
        s.idle(3);
        let (a, b) = (s.rect(A), s.rect(B));
        // B under the line of A's top, so that they share a stretch of edge
        s.drop_at(B, Pos2::new(b.left(), a.top() + 60.0));
        let b = s.rect(B);
        let grab = Pos2::new(a.right(), a.center().y);
        let near = Pos2::new(b.left() - GAP - 4.0, grab.y);
        s.press_and_hold(grab, near);
        let resized = s.rect(A);
        assert_eq!((resized.left(), resized.right()), (a.left(), b.left() - GAP), "snapped while held");
        assert_eq!((resized.top(), resized.bottom()), (a.top(), a.bottom()));
        assert!(!s.linked(A, B));
        s.release(near);
        assert_eq!(s.rect(A), resized);
        assert!(s.linked(A, B));
        // the edge pulled back: no longer side by side
        let grab = Pos2::new(resized.right(), resized.center().y);
        let away = grab - Vec2::new(120.0, 0.0);
        s.press_and_hold(grab, away);
        s.release(away);
        assert_eq!(s.rect(A).right(), resized.right() - 120.0);
        assert!(!s.linked(A, B));
        // Alt: the edge stays under the pointer
        let now = s.rect(A);
        let grab = Pos2::new(now.right(), now.center().y);
        let near = Pos2::new(b.left() - GAP - 4.0, grab.y);
        s.alt = true;
        s.press_and_hold(grab, near);
        assert_eq!(s.rect(A).right(), b.left() - GAP - 4.0);
        s.release(near);
        s.alt = false;
        s.idle(2);
        assert!(!s.linked(A, B));
    }

    /// The window without a title bar is dragged by its surface and resized
    /// by its edges, and snaps like the floaters.
    #[test]
    fn the_bare_window_moves_resizes_and_snaps() {
        let mut s = Session::new();
        s.idle(3);
        let map = s.rect(MAP);
        assert_eq!(map, Rect::from_min_size(Pos2::new(1200.0, 100.0), Vec2::splat(200.0)));
        let a = s.rect(A);
        // dragged by its middle, dropped under A
        let from = map.center();
        let to = from + (Pos2::new(a.left() + 3.0, a.bottom() + GAP - 5.0) - map.min);
        s.alt = true;
        s.press_and_hold(from, to);
        assert_eq!(s.rect(MAP).min, Pos2::new(a.left() + 3.0, a.bottom() + GAP - 5.0));
        s.release(to);
        s.alt = false;
        let map = s.rect(MAP);
        assert_eq!(map.min, Pos2::new(a.left(), a.bottom() + GAP));
        assert_eq!(map.size(), Vec2::splat(200.0));
        assert!(s.linked(A, MAP));
        // its right edge dragged to the line of A's right edge
        let grab = Pos2::new(map.right(), map.center().y);
        let near = Pos2::new(a.right() - 6.0, grab.y);
        s.press_and_hold(grab, near);
        s.release(near);
        let resized = s.rect(MAP);
        assert_eq!((resized.left(), resized.right()), (a.left(), a.right()));
        assert_eq!((resized.top(), resized.bottom()), (map.top(), map.bottom()));
        // and it does not go under its least size
        let grab = Pos2::new(resized.right(), resized.center().y);
        let far = Pos2::new(resized.left() - 100.0, grab.y);
        s.press_and_hold(grab, far);
        s.release(far);
        assert_eq!(s.rect(MAP).left(), a.left());
        assert!(s.rect(MAP).width() >= 64.0 && s.rect(MAP).width() < 80.0, "{:?}", s.rect(MAP));
    }
}
