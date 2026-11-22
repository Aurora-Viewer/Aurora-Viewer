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
    /// windows to open (the pass only draws in windows that exist) and to
    /// let go of, and the scenario's scripted pointer. `own`: the viewer's
    /// own window.
    pub(super) fn outside_frame_start(&mut self, event_loop: &ActiveEventLoop, own: &Window, renderer: &mut Renderer) {
        let own_id = own.id();
        self.carry.keep(|id| id == own_id || self.outside.size_of(id).is_some());
        if self.carry.frame_start(&self.egui_ctx) {
            log::info!("carry: nothing in hand any more");
        }
        for id in self.outside.answers(renderer, &self.egui_ctx) {
            if id == demo::id() {
                self.outside_demo = None;
            }
        }
        let zoom = self.egui_ctx.zoom_factor();
        let icon = self.window_icon.clone();
        // (where the system tells a program where its window is: not Wayland)
        let origin = own.inner_position().ok().map(|at| (at, own.scale_factor()));
        self.outside.reconcile(event_loop, own, renderer, &self.egui_ctx, |ask, was| {
            let attributes = crate::cli::with_desktop_id(Window::default_attributes())
                .with_title(ask.title.clone())
                .with_inner_size(logical(ask.rect.size(), zoom))
                .with_min_inner_size(logical(if ask.content_sized { ask.rect.size() } else { ask.min_size }, zoom))
                .with_window_icon(icon.clone())
                // (the one a recovery replaces does not take the focus
                // from the window that had it)
                .with_active(was.is_none_or(|was| was.focused));
            // (a window sized by its content: the user does not resize it)
            let attributes = if ask.content_sized {
                attributes.with_max_inner_size(logical(ask.rect.size(), zoom))
            } else {
                attributes
            };
            match opens_at(was.and_then(|was| was.at), origin, ask.rect.min, zoom) {
                Some(at) => attributes.with_position(at),
                None => attributes,
            }
        });
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

    /// An event of the system, before any window reads it: true when the
    /// carry of an item between two windows took it (outside/carry.rs).
    pub(super) fn carry_window_event(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        use crate::outside::carry::{Event, Step};
        let size = match &self.gfx {
            Some(g) if g.window.id() == id => Some(g.window.inner_size()),
            _ => self.outside.size_of(id),
        };
        let Some(size) = size else {
            return false;
        };
        let (went, step) = self.carry.window_event(&self.egui_ctx, id, size, event);
        match step {
            Step::Nothing => {}
            Step::Began => log::info!("carry: an item let go of beyond its window stays in hand"),
            Step::Dropping => log::info!("carry: a click drops the item in hand"),
            Step::LetGo => log::info!("carry: the item in hand is let go of"),
        }
        if step != Step::Nothing {
            self.last_input = Instant::now();
            self.egui_ctx.request_repaint();
        }
        went == Event::Swallow
    }

    /// A Windows Alt code typed in a field of the window outside `id`
    /// (alt_codes.rs), as `alt_code_key` reads it in the viewer's own
    /// window: true for one of its digits, which neither the field nor the
    /// viewer sees. Left Alt's release hands the character to the window's
    /// next pass.
    fn outside_alt_code_key(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        if self.outside.size_of(id).is_none() {
            return false;
        }
        let ke = match event {
            WindowEvent::KeyboardInput { event: ke, .. } => ke,
            WindowEvent::Focused(false) => {
                self.alt_codes.reset();
                return false;
            }
            _ => return false,
        };
        let PhysicalKey::Code(code) = ke.physical_key else {
            return false;
        };
        let pressed = ke.state == ElementState::Pressed;
        let reads = self.outside.reads_keys(id);
        if code == KeyCode::AltLeft {
            if pressed {
                if !ke.repeat {
                    self.alt_codes.alt_pressed();
                }
            } else if let Some(text) = self.alt_codes.alt_released(ke.text.as_deref())
                && reads
            {
                self.outside.text(id, text);
            }
            return false;
        }
        pressed && self.alt_codes.active() && reads && self.alt_codes.key_pressed(code, ke.repeat)
    }

    /// An event of the system, first: a window outside reads its own, and
    /// says whether the viewer's handling takes it too.
    pub(super) fn outside_window_event(&mut self, id: WindowId, event: &WindowEvent) -> Went {
        if self.outside_alt_code_key(id, event) {
            return Went::Outside;
        }
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
                    if let Some(g) = &mut self.gfx {
                        self.outside.close(&mut g.renderer, &self.egui_ctx, viewport);
                    }
                } else {
                    // a window of the interface: closed as its cross inside
                    // closes it; its window outside goes once nothing draws
                    // in it (`Outside::reconcile`)
                    crate::ui::widgets::close_window(&self.egui_ctx, viewport.0);
                }
                Went::Outside
            }
            Asked::Viewer => went_on(event, self.focused),
        }
    }
}

/// A size of the interface, in points, as the system counts a window's.
fn logical(size: egui::Vec2, zoom: f32) -> winit::dpi::LogicalSize<f32> {
    winit::dpi::LogicalSize::new(size.x * zoom, size.y * zoom)
}

/// Where the point `at` of the viewer's interface is on the screen, the
/// viewer's own window being at `origin` at the scale `scale`.
fn on_screen(origin: winit::dpi::PhysicalPosition<i32>, scale: f64, at: egui::Pos2, zoom: f32) -> winit::dpi::PhysicalPosition<i32> {
    let pixels = |v: f32| (f64::from(v * zoom) * scale).round() as i32;
    winit::dpi::PhysicalPosition::new(origin.x + pixels(at.x), origin.y + pixels(at.y))
}

/// Where a window outside opens: where the window it replaces was (after a
/// GPU recovery), else over its place in the viewer's window, whose own
/// place and scale are `origin`. None where the system places the windows
/// itself and tells no program where (Wayland).
fn opens_at(
    was: Option<winit::dpi::PhysicalPosition<i32>>,
    origin: Option<(winit::dpi::PhysicalPosition<i32>, f64)>,
    at: egui::Pos2,
    zoom: f32,
) -> Option<winit::dpi::PhysicalPosition<i32>> {
    was.or_else(|| origin.map(|(origin, scale)| on_screen(origin, scale, at, zoom)))
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
    fn a_window_opens_outside_where_and_as_large_as_it_was_inside() {
        // the interface at 125 % on a screen at 200 %: a point is 2.5 pixels
        let (zoom, scale) = (1.25, 2.0);
        // the viewer's window at (300, 100) on the screen, the window of
        // the interface at (40, 80) points in it
        let origin = winit::dpi::PhysicalPosition::new(300, 100);
        assert_eq!(
            on_screen(origin, scale, egui::pos2(40.0, 80.0), zoom),
            winit::dpi::PhysicalPosition::new(400, 300)
        );
        // 480 x 320 points: 600 x 400 as the system counts at that scale
        assert_eq!(logical(egui::vec2(480.0, 320.0), zoom), winit::dpi::LogicalSize::new(600.0, 400.0));
    }

    #[test]
    fn the_window_that_replaces_one_after_a_recovery_opens_where_it_was() {
        let origin = Some((winit::dpi::PhysicalPosition::new(300, 100), 2.0));
        let inside = egui::pos2(40.0, 80.0);
        // sent outside: over its place in the viewer's window
        assert_eq!(
            opens_at(None, origin, inside, 1.0),
            Some(winit::dpi::PhysicalPosition::new(380, 260))
        );
        // after a recovery: where the user had moved it, on another screen
        let was = winit::dpi::PhysicalPosition::new(-1500, 420);
        assert_eq!(opens_at(Some(was), origin, inside, 1.0), Some(was));
        // the system places the windows itself and says nothing of it
        assert_eq!(opens_at(None, None, inside, 1.0), None);
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
