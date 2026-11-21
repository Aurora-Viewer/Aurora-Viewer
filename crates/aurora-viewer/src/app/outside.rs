//! The app's side of the interface's windows outside the viewer's
//! (outside.rs): what opens and closes them, their place in a frame, and
//! where their events of the system go.

use super::*;
use crate::outside::{Asked, demo};

/// Where an event of the system goes once the windows outside have seen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Went {
    /// To the viewer's own window: the event is its own.
    Own,
    /// Nowhere else: a window outside took it whole.
    Outside,
    /// From a window outside on to the viewer's own handling, without the
    /// viewer's interface (`Asked::Viewer`).
    On,
}

impl App {
    /// The viewer is in front: one of its windows has the focus, its own
    /// or one outside (`outside::in_front`).
    pub(super) fn in_front(&self) -> bool {
        crate::outside::in_front(self.focused, self.outside.focused())
    }

    /// Before the interface's pass: what the render thread answered, the
    /// windows to open (the pass only draws in windows that exist), and
    /// the scenario's scripted pointer.
    pub(super) fn outside_frame_start(&mut self, event_loop: &ActiveEventLoop, renderer: &mut Renderer) {
        for id in self.outside.answers(renderer) {
            if id == demo::id() {
                self.outside_demo = None;
            }
        }
        let Some(scenario) = self.outside_demo.as_ref().map(|demo| demo.scenario) else {
            return;
        };
        if self.frame_count < demo::OPENS_AT {
            return;
        }
        if !self.outside.is_open(demo::id()) {
            let attributes = crate::cli::with_desktop_id(Window::default_attributes())
                .with_title(format!("{} - AURORA_DEMO_OUTSIDE", crate::cli::window_title()))
                .with_inner_size(winit::dpi::LogicalSize::new(640, 220))
                .with_window_icon(self.window_icon.clone());
            if let Err(e) = self
                .outside
                .open(event_loop, renderer, &self.egui_ctx, demo::id(), attributes, scenario.sync)
            {
                log::warn!("AURORA_DEMO_OUTSIDE: the window could not be opened: {e}");
                self.outside_demo = None;
            }
            return;
        }
        let step = self.outside_demo.as_ref().and_then(|demo| demo.scripted(self.frame_count));
        if let Some(step) = step {
            self.outside_scripted(step);
        }
    }

    /// A step of the scenario's pointer, by the way the system's events
    /// take (`outside_window_event`).
    fn outside_scripted(&mut self, step: demo::Pointer) {
        let at = match step {
            demo::Pointer::Move(at) => at,
            demo::Pointer::Press | demo::Pointer::Release => egui::Pos2::ZERO,
        };
        let Some((window, position)) = self.outside.pixel(demo::id(), at, self.egui_ctx.zoom_factor()) else {
            return;
        };
        let device_id = winit::event::DeviceId::dummy();
        let event = match step {
            demo::Pointer::Move(_) => WindowEvent::CursorMoved { device_id, position },
            demo::Pointer::Press | demo::Pointer::Release => WindowEvent::MouseInput {
                device_id,
                state: if step == demo::Pointer::Press {
                    ElementState::Pressed
                } else {
                    ElementState::Released
                },
                button: MouseButton::Left,
            },
        };
        self.outside_window_event(window, &event);
    }

    /// In the interface's pass: the windows outside that the app itself
    /// fills.
    pub(super) fn outside_windows(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if let Some(demo) = &mut self.outside_demo {
            let frame = self.frame_count;
            self.outside.show(ctx, demo::id(), |ui| demo.show(ui, p, frame));
        }
    }

    /// An event of the system, first: a window outside reads its own, and
    /// says whether the viewer's handling takes it too.
    pub(super) fn outside_window_event(&mut self, id: WindowId, event: &WindowEvent) -> Went {
        let Some(asked) = self.outside.window_event(id, event) else {
            return Went::Own;
        };
        match event {
            // the keyboard is where the focus is: a field of the viewer's
            // own window lets go of it, as when a window inside is clicked
            WindowEvent::Focused(true) => self.egui_ctx.memory_mut(|m| {
                if let Some(field) = m.focused() {
                    m.surrender_focus(field);
                }
            }),
            // (the keys note it in the viewer's own handling)
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
            | WindowEvent::MouseWheel { .. } => self.last_input = Instant::now(),
            _ => {}
        }
        match asked {
            Asked::Nothing => Went::Outside,
            Asked::Close(viewport) => {
                if viewport == demo::id() {
                    self.outside_demo = None;
                }
                if let Some(g) = &mut self.gfx {
                    self.outside.close(&mut g.renderer, viewport);
                }
                Went::Outside
            }
            Asked::Viewer => went_on(event, self.focused),
        }
    }
}

/// An event a window outside hands on (`Asked::Viewer`). Its focus that
/// goes lets go of what the keyboard and the pointer held, as the viewer's
/// own does, unless the viewer's own window has the focus already: what
/// is held there is its own.
fn went_on(event: &WindowEvent, own_focused: bool) -> Went {
    if matches!(event, WindowEvent::Focused(false)) && own_focused {
        Went::Outside
    } else {
        Went::On
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_background_cap_waits_for_another_program_in_front() {
        let settings = Settings::default();
        assert!(settings.background_fps_cap);
        let cap = |own: bool, outside: bool| frame_cap(&settings, crate::outside::in_front(own, outside), false, true, 60.0, false, None);
        // the focus in a window outside: the 3D view at its full rate
        assert_eq!(cap(false, true), cap(true, false));
        // another program in front: the background cap
        assert_eq!(cap(false, false), settings.background_fps_limit as f32);
        assert_ne!(cap(false, false), cap(false, true));
    }

    #[test]
    fn the_focus_a_window_outside_loses_lets_go_of_what_was_held() {
        let lost = WindowEvent::Focused(false);
        // to another program
        assert_eq!(went_on(&lost, false), Went::On);
        // to the viewer's own window, which has it already
        assert_eq!(went_on(&lost, true), Went::Outside);
        // the modifiers are the viewer's whichever window has the focus
        let modifiers = WindowEvent::ModifiersChanged(winit::event::Modifiers::default());
        assert_eq!(went_on(&modifiers, true), Went::On);
        assert_eq!(went_on(&modifiers, false), Went::On);
    }
}
