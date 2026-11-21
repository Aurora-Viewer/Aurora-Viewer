//! The app's side of the interface's windows outside the viewer's
//! (outside.rs): what opens and closes them, and their place in a frame.

use super::*;
use crate::outside::{Asked, demo};

impl App {
    /// Before the interface's pass: what the render thread answered, and
    /// the windows to open (the pass only draws in windows that exist).
    pub(super) fn outside_frame_start(&mut self, event_loop: &ActiveEventLoop, renderer: &mut Renderer) {
        for id in self.outside.answers(renderer) {
            if id == demo::id() {
                self.outside_demo = None;
            }
        }
        let Some(scenario) = self.outside_demo else {
            return;
        };
        if self.frame_count < demo::OPENS_AT || self.outside.is_open(demo::id()) {
            return;
        }
        let attributes = crate::cli::with_desktop_id(Window::default_attributes())
            .with_title(format!("{} - AURORA_DEMO_OUTSIDE", crate::cli::window_title()))
            .with_inner_size(winit::dpi::LogicalSize::new(640, 220))
            .with_window_icon(self.window_icon.clone());
        if let Err(e) = self.outside.open(event_loop, renderer, demo::id(), attributes, scenario.sync) {
            log::warn!("AURORA_DEMO_OUTSIDE: the window could not be opened: {e}");
            self.outside_demo = None;
        }
    }

    /// In the interface's pass: the windows outside that the app itself
    /// fills.
    pub(super) fn outside_windows(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if let Some(scenario) = self.outside_demo {
            let frame = self.frame_count;
            self.outside.show(ctx, demo::id(), |ui| demo::show(ui, p, scenario, frame));
        }
    }

    /// An event of the system for a window outside; false when `id` is the
    /// viewer's own window.
    pub(super) fn outside_window_event(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        let Some(asked) = self.outside.window_event(id, event) else {
            return false;
        };
        if let Asked::Close(viewport) = asked {
            if viewport == demo::id() {
                self.outside_demo = None;
            }
            if let Some(g) = &mut self.gfx {
                self.outside.close(&mut g.renderer, viewport);
            }
        }
        true
    }
}
