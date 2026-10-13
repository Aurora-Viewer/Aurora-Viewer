//! Account-row dragging: identity-based destinations and scroll-edge feedback.
use crate::theme::Palette;
use egui::{Context, Painter, Pos2, Rect, Response, Vec2};

type Key = (String, String);

pub struct Move {
    pub from: Key,
    pub target: Key,
    pub after: bool,
}

#[derive(Default)]
pub struct State {
    from: Option<Key>,
    pub rows: Vec<(Key, Rect)>,
    pub scroll: f32,
    last_time: Option<f64>,
    dt: f32,
    enabled: bool,
    demo_gesture: Option<(Pos2, Pos2)>,
}

impl State {
    pub fn is_dragging(&self) -> bool {
        self.from.is_some()
    }

    pub fn is_source(&self, key: &Key) -> bool {
        self.from.as_ref() == Some(key)
    }

    pub fn source(&self) -> Option<&Key> {
        self.from.as_ref()
    }

    pub fn begin(&mut self, ctx: &Context, enabled: bool) {
        let now = ctx.input(|i| i.time);
        self.dt = self
            .last_time
            .map_or(1.0 / 60.0, |previous| (now - previous).clamp(0.0, 0.1) as f32);
        self.last_time = Some(now);
        self.enabled = enabled;
        if !enabled || (self.is_dragging() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))) {
            self.from = None;
        }
        self.rows.clear();
    }

    pub fn row(&mut self, key: Key, response: &Response) {
        if self.enabled && response.drag_started_by(egui::PointerButton::Primary) {
            self.from = Some(key.clone());
        }
        self.rows.push((key, response.rect));
    }

    fn destination(&self, at: Pos2, clip: Rect) -> Option<Move> {
        let from = self.from.as_ref()?;
        if !clip.contains(at) {
            return None;
        }
        let (target, rect) = self
            .rows
            .iter()
            .filter(|(_, rect)| rect.intersects(clip))
            .min_by(|(_, a), (_, b)| {
                let distance = |rect: &Rect| (at.y - at.y.clamp(rect.top(), rect.bottom())).abs();
                distance(a).total_cmp(&distance(b))
            })?;
        (target != from).then(|| Move {
            from: from.clone(),
            target: target.clone(),
            after: at.y > rect.center().y,
        })
    }

    pub fn finish(&mut self, ctx: &Context, p: &Palette, painter: &Painter, clip: Rect) -> Option<Move> {
        let from = self.from.as_ref()?;
        if !self.rows.iter().any(|(key, _)| key == from) {
            self.from = None;
            return None;
        }
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let destination = pointer.and_then(|at| self.destination(at, clip));
        if let Some(destination) = &destination
            && let Some((_, rect)) = self.rows.iter().find(|(key, _)| *key == destination.target)
        {
            let y = if destination.after { rect.bottom() - 1.0 } else { rect.top() + 1.0 };
            // The insertion marker shows the destination, not a row separator.
            painter.with_clip_rect(clip).line_segment(
                [egui::pos2(rect.left() + 3.0, y), egui::pos2(rect.right() - 3.0, y)],
                egui::Stroke::new(2.0, p.violet_light),
            );
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.from = None;
            return destination;
        }
        if let Some(at) = pointer.filter(|at| clip.contains(*at)) {
            let velocity = if at.y < clip.top() + 28.0 {
                -420.0 * ((clip.top() + 28.0 - at.y) / 28.0)
            } else if at.y > clip.bottom() - 28.0 {
                420.0 * ((at.y - clip.bottom() + 28.0) / 28.0)
            } else {
                0.0
            };
            self.scroll = (self.scroll + velocity * self.dt).max(0.0);
        }
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.request_repaint();
        None
    }

    /// Synthetic input only; it never moves the OS pointer or loads profiles.
    pub fn demo_events(&mut self, frame: u64) -> Vec<egui::Event> {
        if std::env::var("AURORA_DEMO_LOGIN").as_deref() != Ok("manage-drag") || !(20..=40).contains(&frame) {
            return Vec::new();
        }
        if self.demo_gesture.is_none() && self.rows.len() >= 2 {
            self.demo_gesture = Some((
                self.rows[0].1.left_top() + Vec2::new(55.0, 45.0),
                self.rows[1].1.left_bottom() + Vec2::new(55.0, -20.0),
            ));
        }
        let Some((start, end)) = self.demo_gesture else {
            return Vec::new();
        };
        let pos = start.lerp(end, (frame.saturating_sub(20) as f32 / 18.0).min(1.0));
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if matches!(frame, 20 | 40) {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: frame == 20,
                modifiers: egui::Modifiers::NONE,
            });
        }
        events
    }
}
