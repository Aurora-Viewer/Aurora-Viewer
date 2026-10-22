//! Pointer-driven circular account browsing with a velocity-projected snap.
use egui::{Context, PointerButton, Rect, Response};

pub struct State {
    pub viewport: Rect,
    pub browse_rect: Rect,
    pub height: f32,
    pub offset: f32,
    dragging: bool,
    target: f32,
    stride: f32,
    last_time: Option<f64>,
    pub suppress_click: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            viewport: Rect::NOTHING,
            browse_rect: Rect::NOTHING,
            height: 0.0,
            offset: 0.0,
            dragging: false,
            target: 0.0,
            stride: 0.0,
            last_time: None,
            suppress_click: false,
        }
    }
}

impl State {
    /// Synthetic input only: no OS pointer movement or user account access.
    pub fn demo_events(&self, frame: u64) -> Vec<egui::Event> {
        let mode = std::env::var("AURORA_DEMO_LOGIN").unwrap_or_default();
        if self.viewport.is_positive() {
            if mode == "hover" && frame >= 20 {
                return vec![egui::Event::PointerMoved(self.viewport.left_top() + egui::vec2(120.0, 45.0))];
            }
            if mode == "wheel-gap" && frame == 22 {
                return vec![
                    egui::Event::PointerMoved(self.viewport.left_top() + egui::vec2(self.stride - 8.0, 45.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta: egui::vec2(0.0, -1.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                        modifiers: egui::Modifiers::NONE,
                    },
                ];
            }
            if matches!(mode.as_str(), "arrows" | "static-arrows") && matches!(frame, 22 | 28) {
                return vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }];
            }
        }
        if !matches!(mode.as_str(), "drag" | "drag2" | "drag-card" | "drag-card2")
            || !self.viewport.is_positive()
            || !(20..=44).contains(&frame)
        {
            return Vec::new();
        }
        let distance = self.stride * 0.85 * (frame.saturating_sub(20).min(23) as f32 / 23.0);
        let start = if mode.starts_with("drag-card") {
            self.viewport.left_top() + egui::vec2(self.stride / 2.0, self.viewport.height() * 0.15)
        } else {
            self.viewport.left_center() - egui::vec2(20.0, 0.0)
        };
        let pos = start - egui::vec2(distance, 0.0);
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if matches!(frame, 20 | 44) {
            events.push(egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: frame == 20,
                modifiers: egui::Modifiers::NONE,
            });
        }
        events
    }

    pub fn moving(&self) -> bool {
        self.dragging || self.offset.abs() > 0.1 || self.target.abs() > 0.1
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn stride(&self) -> f32 {
        self.stride.max(1.0)
    }

    pub fn stop(&mut self) {
        self.offset = 0.0;
        self.target = 0.0;
        self.dragging = false;
    }

    fn normalize(&mut self, page: &mut usize, count: usize) {
        while self.offset <= -self.stride {
            *page = (*page + 1) % count;
            self.offset += self.stride;
            self.target += self.stride;
        }
        while self.offset >= self.stride {
            *page = (*page + count - 1) % count;
            self.offset -= self.stride;
            self.target -= self.stride;
        }
    }

    fn advance(&mut self, page: &mut usize, count: usize, dt: f32) {
        if self.dragging {
            return;
        }
        self.offset += (self.target - self.offset) * (1.0 - (-10.0 * dt).exp());
        if (self.offset - self.target).abs() < 0.7 {
            self.offset = self.target;
        }
        self.normalize(page, count);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn interact(&mut self, ctx: &Context, grips: &[Response], enabled: bool, stride: f32, count: usize, page: &mut usize) {
        let now = ctx.input(|i| i.time);
        // An egui sizing pass reuses the same input. Consume its delta once.
        if self.last_time == Some(now) {
            return;
        }
        let dt = self
            .last_time
            .map_or(1.0 / 60.0, |previous| (now - previous).clamp(0.0, 0.1) as f32);
        self.last_time = Some(now);
        self.suppress_click = self.dragging;
        if self.stride > 0.0 && self.stride != stride {
            self.offset *= stride / self.stride;
            self.target *= stride / self.stride;
        }
        self.stride = stride.max(1.0);
        if !enabled || count < 2 {
            self.stop();
            return;
        }
        let started = grips.iter().find(|r| r.drag_started_by(PointerButton::Primary));
        if started.is_some() {
            self.dragging = true;
            self.target = self.offset;
        }
        if self.dragging {
            self.suppress_click = true;
            // The press frame may also contain movement towards the grip.
            // Start at the press origin, rather than counting that approach.
            self.offset += started.map_or_else(
                || ctx.input(|i| i.pointer.delta().x),
                |response| response.total_drag_delta().unwrap_or_default().x,
            );
            self.normalize(page, count);
            self.target = self.offset;
            // Release outside a grip still belongs to the original gesture.
            if !ctx.input(|i| i.pointer.primary_down()) {
                self.dragging = false;
                let velocity = ctx.input(|i| i.pointer.velocity().x).clamp(-stride * 7.0, stride * 7.0);
                self.target = ((self.offset + velocity * 0.14) / stride).round().clamp(-2.0, 2.0) * stride;
            }
        } else {
            self.advance(page, count, dt);
        }
        if self.dragging {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if self.moving() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_in_both_directions_and_settles_on_a_card() {
        let mut state = State {
            stride: 280.0,
            offset: -615.0,
            target: -615.0,
            ..Default::default()
        };
        let mut page = 1;
        state.normalize(&mut page, 2);
        assert_eq!(page, 1);
        assert_eq!(state.offset, -55.0);
        state.offset = 300.0;
        state.target = 300.0;
        state.normalize(&mut page, 2);
        assert_eq!(page, 0);
        assert_eq!(state.offset, 20.0);
        state.target = 280.0;
        for _ in 0..10 {
            state.advance(&mut page, 2, 1.0 / 30.0);
        }
        assert!(state.moving(), "inertia continues before gently settling");
        for _ in 0..20 {
            state.advance(&mut page, 2, 1.0 / 30.0);
        }
        assert_eq!(page, 1);
        assert_eq!(state.offset, 0.0);
        assert!(!state.moving());
    }
}
