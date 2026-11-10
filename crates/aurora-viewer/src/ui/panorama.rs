//! The panorama viewer's controls: a 2:1 equirectangular image looked into
//! with the mouse and the keys (`aurora_render::panorama` draws it). Drag
//! looks around, the wheel zooms the field of view between 30° and 100°
//! (it starts at the Legacy Viewer's 75°), a double-click puts the view
//! back and restarts the slow auto-rotate (~1.2°/s, the Legacy Viewer's
//! OrbitControls speed, which a press stops), the arrows look and `+`/`-`
//! zoom while the viewer has the focus, and a button enlarges it to the
//! whole window. Used by the inventory's 360° texture preview and by the
//! photo window's 360° result; both give it an image and keep a `PanoView`.

use super::widgets;
use crate::theme::Palette;
use aurora_render::panorama::{self, PanoParams};
use aurora_render::preview::Image;
use egui::{Key, Vec2};
use std::sync::Arc;

/// The field of view the Legacy Viewer's page opens with.
pub const START_FOV: f32 = 75.0 * DEG;
pub const MIN_FOV: f32 = 30.0 * DEG;
pub const MAX_FOV: f32 = 100.0 * DEG;
/// OrbitControls' `autoRotateSpeed` of 0.2 at the page's frame rate.
const AUTO_ROTATE: f32 = 1.2 * DEG;
/// The arrows' speed at the start field of view; it scales with the field.
const KEY_LOOK: f32 = 60.0 * DEG;
/// Zoom per second of a held `+` or `-` (a factor of e).
const KEY_ZOOM: f32 = 1.0;
/// Wheel points per factor of e on the field of view.
const WHEEL_ZOOM: f32 = 0.002;
const MAX_PITCH: f32 = 89.0 * DEG;
const DEG: f32 = std::f32::consts::PI / 180.0;

/// Which of the viewer's keys are held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keys {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub zoom_in: bool,
    pub zoom_out: bool,
}

impl Keys {
    fn any(self) -> bool {
        self.left || self.right || self.up || self.down || self.zoom_in || self.zoom_out
    }
}

/// Where the viewer looks, and whether it turns by itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanoView {
    /// Radians, the compass heading (0 north, a quarter turn east).
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
    pub rotating: bool,
    /// Enlarged to the whole window.
    pub full: bool,
    /// The heading the view starts at and returns to.
    start_yaw: f32,
}

impl PanoView {
    /// A view at heading `yaw` (radians), turning slowly.
    pub fn new(yaw: f32) -> Self {
        PanoView {
            yaw,
            pitch: 0.0,
            fov: START_FOV,
            rotating: true,
            full: false,
            start_yaw: yaw,
        }
    }

    pub fn params(&self) -> PanoParams {
        PanoParams {
            yaw: self.yaw,
            pitch: self.pitch,
            fov_y: self.fov,
        }
    }

    /// A press or a key stops the slow turn.
    pub fn stop(&mut self) {
        self.rotating = false;
    }

    /// Double-click: the start view, turning again.
    pub fn reset(&mut self) {
        let full = self.full;
        *self = PanoView::new(self.start_yaw);
        self.full = full;
    }

    /// A drag of `delta` points over a viewer `height` points high: the
    /// image follows the cursor.
    pub fn drag(&mut self, delta: Vec2, height: f32) {
        self.stop();
        let per_point = self.fov / height.max(1.0);
        self.yaw = (self.yaw - delta.x * per_point).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + delta.y * per_point).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// The wheel (points; positive zooms in).
    pub fn wheel(&mut self, amount: f32) {
        self.stop();
        self.fov = (self.fov * (-amount * WHEEL_ZOOM).exp()).clamp(MIN_FOV, MAX_FOV);
    }

    /// `dt` seconds: the held keys look and zoom, otherwise the slow turn.
    pub fn step(&mut self, dt: f32, keys: Keys) {
        if keys.any() {
            self.stop();
            let look = KEY_LOOK * dt * self.fov / START_FOV;
            let axis = |plus: bool, minus: bool| f32::from(u8::from(plus)) - f32::from(u8::from(minus));
            self.yaw = (self.yaw + axis(keys.right, keys.left) * look).rem_euclid(std::f32::consts::TAU);
            self.pitch = (self.pitch + axis(keys.up, keys.down) * look).clamp(-MAX_PITCH, MAX_PITCH);
            let zoom = axis(keys.zoom_out, keys.zoom_in);
            self.fov = (self.fov * (zoom * KEY_ZOOM * dt).exp()).clamp(MIN_FOV, MAX_FOV);
        } else if self.rotating {
            self.yaw = (self.yaw + AUTO_ROTATE * dt).rem_euclid(std::f32::consts::TAU);
        }
    }

    /// The viewer in `size` points, with its buttons over its corner.
    /// Handles the mouse and, while it has the focus, the keys.
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, id: egui::Id, image: &Arc<Image>, size: Vec2) -> egui::Response {
        if self.full {
            // enlarged: the room stays, the picture is the enlarged one's
            // (one viewer steps and draws, not two)
            return ui.allocate_exact_size(size, egui::Sense::hover()).1;
        }
        self.show_viewer(ui, p, id, image, size)
    }

    fn show_viewer(&mut self, ui: &mut egui::Ui, p: &Palette, id: egui::Id, image: &Arc<Image>, size: Vec2) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        self.interact(ui, &response);
        if ui.is_rect_visible(rect) {
            panorama::paint_in(ui.painter(), rect, id, image.clone(), self.params());
        }
        self.buttons(ui, p, rect);
        response.on_hover_text(aurora_i18n::tr!("panorama-help"))
    }

    fn interact(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.is_pointer_button_down_on() || response.clicked() {
            // (a click gives the viewer the keys)
            response.request_focus();
        }
        if response.double_clicked() {
            self.reset();
        } else if response.dragged_by(egui::PointerButton::Primary) {
            let delta = ui.input(|i| i.pointer.delta());
            self.drag(delta, response.rect.height());
        } else if response.is_pointer_button_down_on() {
            // (a press stops the turn, as the Legacy Viewer's does)
            self.stop();
        }
        if response.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                self.wheel(wheel);
            }
        }
        let focused = response.has_focus();
        let keys = if focused {
            // the arrows are the viewer's, not a move to the next widget
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
            // (Escape leaves the focus, and nothing else acts on it)
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                response.surrender_focus();
            }
            ui.input(|i| Keys {
                left: i.key_down(Key::ArrowLeft),
                right: i.key_down(Key::ArrowRight),
                up: i.key_down(Key::ArrowUp),
                down: i.key_down(Key::ArrowDown),
                zoom_in: i.key_down(Key::Plus) || i.key_down(Key::Equals),
                zoom_out: i.key_down(Key::Minus),
            })
        } else {
            Keys::default()
        };
        let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.1);
        let before = *self;
        self.step(dt, keys);
        if self.rotating || keys.any() || response.dragged() || *self != before {
            ui.ctx().request_repaint();
        }
    }

    /// Reset and enlarge (or leave the enlarged view), over the corner.
    fn buttons(&mut self, ui: &mut egui::Ui, p: &Palette, rect: egui::Rect) {
        let strip = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 2.0 * 24.0 - 6.0, rect.top() + 6.0),
            Vec2::new(2.0 * 24.0, 22.0),
        );
        // (a child, not a scope: a scope would move the parent's cursor back
        // up to the corner)
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(strip)
                .layout(egui::Layout::right_to_left(egui::Align::Min)),
        );
        let ui = &mut child;
        let (icon, tip) = if self.full {
            ("arrows-minimize", aurora_i18n::tr!("panorama-leave-full"))
        } else {
            ("arrows-maximize", aurora_i18n::tr!("panorama-enlarge"))
        };
        if widgets::icon_button(ui, p, icon, tip, true) {
            self.full = !self.full;
        }
        if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("panorama-reset"), true) {
            self.reset();
        }
    }

    /// The enlarged viewer over the whole window, while `self.full`; Escape
    /// or its button leaves it. `id` is the viewer's, as for `show`.
    pub fn show_full(&mut self, ctx: &egui::Context, p: &Palette, id: egui::Id, image: &Arc<Image>) {
        if !self.full {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.full = false;
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(id.with("full"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .constrain(false)
            .show(ctx, |ui| {
                let response = self.show_viewer(ui, p, id.with("full-view"), image, screen.size());
                // (the enlarged viewer takes the keys at once)
                if !response.has_focus() && response.ctx.memory(|m| m.focused().is_none()) {
                    response.request_focus();
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_view_starts_where_the_legacy_page_does() {
        let v = PanoView::new(1.0);
        assert!(approx(v.fov, 75.0 * DEG) && v.rotating && v.pitch == 0.0 && !v.full);
        assert!(approx(v.params().yaw, 1.0));
    }

    #[test]
    fn dragging_makes_the_image_follow_the_cursor() {
        let mut v = PanoView::new(1.0);
        // a drag of one viewer height to the right turns the view back by one field
        v.drag(Vec2::new(200.0, 0.0), 200.0);
        assert!(approx(v.yaw, (1.0 - 75.0 * DEG).rem_euclid(std::f32::consts::TAU)), "{}", v.yaw);
        assert!(!v.rotating, "a drag stops the turn");
        // down drags the image down: the view looks up
        let mut v = PanoView::new(0.0);
        v.drag(Vec2::new(0.0, 100.0), 200.0);
        assert!(approx(v.pitch, 37.5 * DEG));
        v.drag(Vec2::new(0.0, 10_000.0), 200.0);
        assert!(approx(v.pitch, 89.0 * DEG), "the pitch stops short of the poles");
    }

    #[test]
    fn the_wheel_zooms_between_30_and_100_degrees() {
        let mut v = PanoView::new(0.0);
        v.wheel(100.0);
        assert!(v.fov < START_FOV);
        v.wheel(100_000.0);
        assert!(approx(v.fov, MIN_FOV));
        v.wheel(-1_000_000.0);
        assert!(approx(v.fov, MAX_FOV));
    }

    #[test]
    fn the_slow_turn_runs_until_something_stops_it_and_double_click_restarts_it() {
        let mut v = PanoView::new(0.0);
        v.step(10.0, Keys::default());
        assert!(approx(v.yaw, 12.0 * DEG), "1.2 degrees a second");
        v.stop();
        v.step(10.0, Keys::default());
        assert!(approx(v.yaw, 12.0 * DEG), "stopped");
        v.drag(Vec2::new(-20.0, 5.0), 200.0);
        v.wheel(50.0);
        v.full = true;
        v.reset();
        assert!(v.rotating && v.pitch == 0.0 && approx(v.fov, START_FOV) && approx(v.yaw, 0.0));
        assert!(v.full, "the reset keeps the enlarged view");
    }

    #[test]
    fn held_keys_look_and_zoom_and_stop_the_turn() {
        let mut v = PanoView::new(0.0);
        v.step(
            1.0,
            Keys {
                right: true,
                ..Default::default()
            },
        );
        assert!(approx(v.yaw, 60.0 * DEG) && !v.rotating);
        v.step(
            1.0,
            Keys {
                up: true,
                ..Default::default()
            },
        );
        assert!(approx(v.pitch, 60.0 * DEG));
        // zoomed in, the arrows are slower
        let mut z = PanoView::new(0.0);
        z.fov = 37.5 * DEG;
        z.step(
            1.0,
            Keys {
                right: true,
                ..Default::default()
            },
        );
        assert!(approx(z.yaw, 30.0 * DEG));
        let mut z = PanoView::new(0.0);
        z.step(
            1.0,
            Keys {
                zoom_in: true,
                ..Default::default()
            },
        );
        assert!(z.fov < START_FOV);
        z.step(
            100.0,
            Keys {
                zoom_out: true,
                ..Default::default()
            },
        );
        assert!(approx(z.fov, MAX_FOV));
    }

    /// One frame of `ui_fn` with `events`; returns the viewer's id.
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, view: &mut PanoView, image: &Arc<Image>) -> egui::Response {
        let p = crate::theme::Theme::default().palette();
        let mut out = None;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 400.0))),
                events,
                ..Default::default()
            },
            |ui| {
                out = Some(view.show(ui, &p, egui::Id::unique("pano-test"), image, Vec2::new(200.0, 100.0)));
            },
        );
        out.expect("the viewer was shown")
    }

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// The arrows reach the viewer only after a click gave it the focus.
    #[test]
    fn keys_reach_the_viewer_only_while_it_has_the_focus() {
        let ctx = egui::Context::default();
        let image = Arc::new(Image {
            size: [2, 1],
            rgba: Arc::from([0u8; 8]),
        });
        let mut view = PanoView::new(0.0);
        view.rotating = false;
        let first = frame(&ctx, Vec::new(), &mut view, &image);
        let centre = first.rect.center();
        // not focused: the arrow does nothing
        frame(&ctx, vec![key(egui::Key::ArrowRight, true)], &mut view, &image);
        frame(&ctx, Vec::new(), &mut view, &image);
        frame(&ctx, vec![key(egui::Key::ArrowRight, false)], &mut view, &image);
        assert_eq!(view.yaw, 0.0, "no focus, no look");
        // a click focuses it
        frame(&ctx, vec![egui::Event::PointerMoved(centre)], &mut view, &image);
        for pressed in [true, false] {
            frame(
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: centre,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
                &mut view,
                &image,
            );
        }
        view.rotating = false;
        let focused = frame(&ctx, Vec::new(), &mut view, &image);
        assert!(focused.has_focus(), "a click gives the viewer the focus");
        frame(&ctx, vec![key(egui::Key::ArrowRight, true)], &mut view, &image);
        frame(&ctx, Vec::new(), &mut view, &image);
        assert!(view.yaw > 0.0, "the arrow looks east while focused: {}", view.yaw);
    }
}
