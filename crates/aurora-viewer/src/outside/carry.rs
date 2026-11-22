//! An item dragged out of one of the viewer's windows and carried to
//! another: a drag does not go from one window of the system to the next.
//!
//! While a button is held, the system sends the pointer to the window the
//! press began in only, at places beyond its edges (checked on Wayland,
//! which besides tells no program where its windows are): a window outside
//! the viewer's (outside.rs) and the viewer's own cannot follow one drag.
//! The release, though, is followed at once by the pointer entering the
//! window it is over. So an item let go of beyond the edges of its window
//! stays in hand: the cargo of the drag is put back as it was, the windows
//! show what would take it under the pointer as during a drag, and one
//! click drops it there, or nowhere when nothing takes it. Escape or
//! another button lets go of it.
//!
//! Nothing here knows what is carried: the cargo is egui's
//! (`egui::DragAndDrop`), read and taken by the same code as a drag inside
//! one window, with the same checks of the creators' permissions. The
//! carry adds no way to drop, only the time between the release and the
//! click.
//!
//! Not in the Legacy Viewer, whose floaters stay inside its window.

use std::collections::HashMap;

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowId;

use crate::theme::Palette;

/// What becomes of an event of the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    /// The window reads it as usual.
    Pass,
    /// The carry took it: no window reads it.
    Swallow,
}

/// What the carry did with an event, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Nothing,
    Began,
    Dropping,
    LetGo,
}

#[derive(Debug, Clone, Copy, Default)]
struct Pointer {
    /// Where it was last, in the window's pixels: beyond its edges while a
    /// button pressed in it is held.
    at: Option<PhysicalPosition<f64>>,
    /// The system says it is over the window.
    over: bool,
}

struct InHand {
    cargo: egui::DragAndDrop,
    /// The cargo is back in egui. Before that, the pass of the window the
    /// drag began in has yet to read the release: egui clears the cargo at
    /// the end of that pass, and it is put back after.
    held: bool,
    /// The press of the click that drops it was swallowed: its release is
    /// the drop.
    pressed: bool,
}

#[derive(Default)]
pub(crate) struct Carry {
    pointers: HashMap<WindowId, Pointer>,
    in_hand: Option<InHand>,
    /// A press the carry swallowed, whose release goes with it.
    swallowed: Option<MouseButton>,
    /// Escape let go of the item: its release goes with the press.
    escape: bool,
}

/// `at`, in a window's pixels, is beyond the edges of a window of `size`.
fn beyond(at: PhysicalPosition<f64>, size: PhysicalSize<u32>) -> bool {
    at.x < 0.0 || at.y < 0.0 || at.x >= f64::from(size.width) || at.y >= f64::from(size.height)
}

/// Whether a release of the left button begins a carry: something is
/// dragged, and the pointer is beyond the edges of the window the drag
/// began in.
fn begins(dragging: bool, at: Option<PhysicalPosition<f64>>, size: PhysicalSize<u32>) -> bool {
    dragging && at.is_some_and(|at| beyond(at, size))
}

fn holding_id() -> egui::Id {
    egui::Id::unique("aurora carry holding")
}

/// An item is in hand.
pub(crate) fn holding(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(holding_id())).unwrap_or(false)
}

/// The left button is held or an item is in hand: what a drag shows while
/// it lasts (its cursor, its badge) shows.
pub(crate) fn dragging(ctx: &egui::Context) -> bool {
    holding(ctx) || ctx.input(|i| i.pointer.primary_down())
}

fn cargo(ctx: &egui::Context) -> Option<egui::DragAndDrop> {
    ctx.with_plugin::<egui::DragAndDrop, _>(|plugin| plugin.clone())
}

impl Carry {
    pub fn holds(&self) -> bool {
        self.in_hand.is_some()
    }

    fn let_go(&mut self, ctx: &egui::Context) {
        if self.in_hand.take().is_some() {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }

    /// An event of the system for the window `id` of the viewer, `size`
    /// its inner size, before any window reads it.
    pub fn window_event(&mut self, ctx: &egui::Context, id: WindowId, size: PhysicalSize<u32>, event: &WindowEvent) -> (Event, Step) {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.pointers.entry(id).or_default().at = Some(*position);
            }
            WindowEvent::CursorEntered { .. } => self.pointers.entry(id).or_default().over = true,
            WindowEvent::CursorLeft { .. } => self.pointers.entry(id).or_default().over = false,
            WindowEvent::Destroyed => {
                self.pointers.remove(&id);
            }
            WindowEvent::MouseInput { state, button, .. } => return self.button(ctx, id, size, *state, *button),
            WindowEvent::KeyboardInput { event, .. } if event.logical_key == Key::Named(NamedKey::Escape) => {
                return self.escape_key(ctx, event.state == ElementState::Pressed, event.repeat);
            }
            // (the release of a key goes to the program that has the focus)
            WindowEvent::Focused(false) => self.escape = false,
            _ => {}
        }
        (Event::Pass, Step::Nothing)
    }

    /// The windows of the viewer are those `open` says: the pointer of one
    /// that closed goes with it, whose leaving no event tells.
    pub fn keep(&mut self, open: impl Fn(WindowId) -> bool) {
        self.pointers.retain(|id, _| open(*id));
    }

    /// Escape lets go of the item in hand, and goes with it to its release:
    /// neither the press, nor what the held key repeats, closes a menu or a
    /// window under it.
    fn escape_key(&mut self, ctx: &egui::Context, pressed: bool, repeat: bool) -> (Event, Step) {
        if pressed {
            if self.holds() {
                self.let_go(ctx);
                self.escape = true;
                return (Event::Swallow, Step::LetGo);
            }
            if self.escape && repeat {
                return (Event::Swallow, Step::Nothing);
            }
            // (a new press: the release of the last one was not ours to read)
            self.escape = false;
        } else if std::mem::take(&mut self.escape) {
            return (Event::Swallow, Step::Nothing);
        }
        (Event::Pass, Step::Nothing)
    }

    fn button(
        &mut self,
        ctx: &egui::Context,
        id: WindowId,
        size: PhysicalSize<u32>,
        state: ElementState,
        button: MouseButton,
    ) -> (Event, Step) {
        let pressed = state == ElementState::Pressed;
        if self.swallowed == Some(button) {
            self.swallowed = None;
            // (pressed again: its release went to another program, and
            // this press is an ordinary one)
            if !pressed {
                return (Event::Swallow, Step::Nothing);
            }
        }
        if let Some(in_hand) = &mut self.in_hand {
            return match (button, pressed) {
                // the click that drops it: its press is not a click on what
                // is under the pointer, its release is the drag's
                (MouseButton::Left, true) => {
                    in_hand.pressed = true;
                    (Event::Swallow, Step::Nothing)
                }
                (MouseButton::Left, false) => (Event::Pass, if in_hand.pressed { Step::Dropping } else { Step::Nothing }),
                (_, true) => {
                    self.let_go(ctx);
                    self.swallowed = Some(button);
                    (Event::Swallow, Step::LetGo)
                }
                (_, false) => (Event::Pass, Step::Nothing),
            };
        }
        if button == MouseButton::Left && !pressed {
            let at = self.pointers.get(&id).and_then(|pointer| pointer.at);
            if begins(egui::DragAndDrop::has_any_payload(ctx), at, size)
                && let Some(cargo) = cargo(ctx)
            {
                self.in_hand = Some(InHand {
                    cargo,
                    held: false,
                    pressed: false,
                });
                return (Event::Pass, Step::Began);
            }
        }
        (Event::Pass, Step::Nothing)
    }

    /// Before the interface's pass. True when the item was let go of here:
    /// the pointer is over none of the viewer's windows once the release
    /// was read (let go of over another program, as a drag that ends
    /// there), or the cargo is gone (dropped, or Escape read by egui).
    pub fn frame_start(&mut self, ctx: &egui::Context) -> bool {
        let over_a_window = self.pointers.values().any(|pointer| pointer.over);
        let cargo_there = egui::DragAndDrop::has_any_payload(ctx);
        let mut ended = false;
        if let Some(in_hand) = &mut self.in_hand {
            if in_hand.held {
                ended = !cargo_there;
            } else if !cargo_there {
                if over_a_window {
                    let cargo = in_hand.cargo.clone();
                    ctx.with_plugin::<egui::DragAndDrop, _>(|plugin| *plugin = cargo);
                    in_hand.held = true;
                } else {
                    ended = true;
                }
            }
        }
        if ended {
            self.in_hand = None;
        }
        let holding = self.in_hand.as_ref().is_some_and(|in_hand| in_hand.held);
        ctx.data_mut(|d| {
            if holding {
                d.insert_temp(holding_id(), true);
            } else {
                d.remove::<bool>(holding_id());
            }
        });
        ended
    }
}

/// What to do with the item in hand, under the pointer of the window being
/// drawn (and under the badge of a drag): in the viewer's pass and in each
/// window outside's.
pub(crate) fn hint(ctx: &egui::Context, p: &Palette) {
    if !holding(ctx) {
        return;
    }
    let Some(pointer) = ctx.pointer_latest_pos() else {
        return;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("carry_hint")));
    let text = painter.layout_no_wrap(
        aurora_i18n::tr!("common-carry-hint").to_string(),
        egui::FontId::proportional(12.0),
        p.ink,
    );
    let size = text.size() + egui::vec2(16.0, 10.0);
    let area = ctx.content_rect();
    let pos = (pointer + egui::vec2(16.0, 46.0)).min(area.max - size).max(area.min);
    let rect = egui::Rect::from_min_size(pos, size);
    let radius = ctx.global_style().visuals.window_corner_radius;
    painter.rect_filled(rect, radius, p.panel);
    painter.rect_stroke(rect, radius, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
    painter.galley(pos + egui::vec2(8.0, 5.0), text, p.ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Item(u32);

    fn size() -> PhysicalSize<u32> {
        PhysicalSize::new(800, 600)
    }

    fn window(n: u64) -> WindowId {
        WindowId::from(n)
    }

    fn moved(x: f64, y: f64) -> WindowEvent {
        WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        }
    }

    fn button(button: MouseButton, state: ElementState) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: winit::event::DeviceId::dummy(),
            state,
            button,
        }
    }

    fn entered() -> WindowEvent {
        WindowEvent::CursorEntered {
            device_id: winit::event::DeviceId::dummy(),
        }
    }

    fn left() -> WindowEvent {
        WindowEvent::CursorLeft {
            device_id: winit::event::DeviceId::dummy(),
        }
    }

    /// The pass of a window that reads a release: egui clears the cargo at
    /// its end.
    fn pass_with_release(ctx: &egui::Context) {
        let input = egui::RawInput {
            events: vec![egui::Event::PointerButton {
                pos: egui::pos2(900.0, 300.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |_| {});
    }

    /// An item dragged in window 1, let go of over window 2: the release
    /// goes to window 1 beyond its edges, then window 2 is entered.
    fn carried(ctx: &egui::Context, carry: &mut Carry) {
        let (source, target) = (window(1), window(2));
        egui::DragAndDrop::set_payload(ctx, Item(7));
        carry.window_event(ctx, source, size(), &entered());
        carry.window_event(ctx, source, size(), &moved(900.0, 300.0));
        let began = carry.window_event(ctx, source, size(), &button(MouseButton::Left, ElementState::Released));
        assert_eq!(began, (Event::Pass, Step::Began));
        carry.window_event(ctx, source, size(), &left());
        carry.window_event(ctx, target, size(), &entered());
        carry.window_event(ctx, target, size(), &moved(120.0, 80.0));
        // a frame begins before the release is read: nothing in hand yet
        assert!(!carry.frame_start(ctx));
        assert!(!holding(ctx));
        pass_with_release(ctx);
        assert!(!egui::DragAndDrop::has_any_payload(ctx), "egui cleared the cargo with the release");
        assert!(!carry.frame_start(ctx));
    }

    #[test]
    fn an_item_let_go_of_beyond_its_window_stays_in_hand() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        assert!(!holding(&ctx));
        carried(&ctx, &mut carry);
        assert!(carry.holds());
        assert!(holding(&ctx));
        assert!(dragging(&ctx));
        // the same cargo, for the same code as during the drag
        assert_eq!(egui::DragAndDrop::payload::<Item>(&ctx).as_deref(), Some(&Item(7)));
        // and it stays, frame after frame
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        assert!(!carry.frame_start(&ctx));
        assert!(egui::DragAndDrop::has_any_payload(&ctx));
    }

    #[test]
    fn one_click_drops_it_and_its_press_reaches_no_window() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        carried(&ctx, &mut carry);
        let target = window(2);
        let press = carry.window_event(&ctx, target, size(), &button(MouseButton::Left, ElementState::Pressed));
        assert_eq!(press, (Event::Swallow, Step::Nothing));
        let release = carry.window_event(&ctx, target, size(), &button(MouseButton::Left, ElementState::Released));
        assert_eq!(release, (Event::Pass, Step::Dropping));
        // the target's pass reads the release with the cargo there: what is
        // under the pointer takes it, or egui clears it
        assert!(egui::DragAndDrop::has_any_payload(&ctx));
        pass_with_release(&ctx);
        assert!(carry.frame_start(&ctx), "dropped: nothing in hand any more");
        assert!(!carry.holds());
        assert!(!holding(&ctx));
        // the next click is an ordinary one
        let press = carry.window_event(&ctx, target, size(), &button(MouseButton::Left, ElementState::Pressed));
        assert_eq!(press, (Event::Pass, Step::Nothing));
    }

    #[test]
    fn another_button_lets_go_of_it_without_reaching_a_window() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        carried(&ctx, &mut carry);
        let target = window(2);
        let press = carry.window_event(&ctx, target, size(), &button(MouseButton::Right, ElementState::Pressed));
        assert_eq!(press, (Event::Swallow, Step::LetGo));
        assert!(!carry.holds());
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
        // (no context menu on what was under the pointer)
        let release = carry.window_event(&ctx, target, size(), &button(MouseButton::Right, ElementState::Released));
        assert_eq!(release, (Event::Swallow, Step::Nothing));
        assert!(!carry.frame_start(&ctx));
        assert!(!holding(&ctx));
        let again = carry.window_event(&ctx, target, size(), &button(MouseButton::Right, ElementState::Pressed));
        assert_eq!(again, (Event::Pass, Step::Nothing));
    }

    #[test]
    fn escape_lets_go_of_it_and_goes_with_it_to_its_release() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        carried(&ctx, &mut carry);
        assert_eq!(carry.escape_key(&ctx, true, false), (Event::Swallow, Step::LetGo));
        assert!(!carry.holds());
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
        // the key is held: what it repeats closes nothing under the pointer
        assert_eq!(carry.escape_key(&ctx, true, true), (Event::Swallow, Step::Nothing));
        assert_eq!(carry.escape_key(&ctx, false, false), (Event::Swallow, Step::Nothing));
        // the next Escape is an ordinary one
        assert_eq!(carry.escape_key(&ctx, true, false), (Event::Pass, Step::Nothing));
        assert_eq!(carry.escape_key(&ctx, false, false), (Event::Pass, Step::Nothing));
    }

    /// The release went to the program that took the focus: the next press
    /// and its release are both the window's.
    #[test]
    fn a_release_read_by_another_program_holds_no_later_one_back() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        carried(&ctx, &mut carry);
        let target = window(2);
        carry.window_event(&ctx, target, size(), &button(MouseButton::Right, ElementState::Pressed));
        let press = carry.window_event(&ctx, target, size(), &button(MouseButton::Right, ElementState::Pressed));
        assert_eq!(press, (Event::Pass, Step::Nothing));
        let release = carry.window_event(&ctx, target, size(), &button(MouseButton::Right, ElementState::Released));
        assert_eq!(release, (Event::Pass, Step::Nothing));

        carried(&ctx, &mut carry);
        assert_eq!(carry.escape_key(&ctx, true, false), (Event::Swallow, Step::LetGo));
        carry.window_event(&ctx, target, size(), &WindowEvent::Focused(false));
        assert_eq!(carry.escape_key(&ctx, true, false), (Event::Pass, Step::Nothing));
        assert_eq!(carry.escape_key(&ctx, false, false), (Event::Pass, Step::Nothing));
    }

    /// A window closed under the pointer tells no leaving: the item let go
    /// of over another program afterwards is not kept in hand for it.
    #[test]
    fn the_pointer_of_a_window_that_closed_goes_with_it() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let (source, closed) = (window(1), window(2));
        carry.window_event(&ctx, closed, size(), &entered());
        carry.keep(|id| id == source);
        egui::DragAndDrop::set_payload(&ctx, Item(7));
        carry.window_event(&ctx, source, size(), &entered());
        carry.window_event(&ctx, source, size(), &moved(-40.0, 300.0));
        carry.window_event(&ctx, source, size(), &button(MouseButton::Left, ElementState::Released));
        carry.window_event(&ctx, source, size(), &left());
        assert!(!carry.frame_start(&ctx));
        pass_with_release(&ctx);
        assert!(carry.frame_start(&ctx));
        assert!(!carry.holds());
    }

    #[test]
    fn let_go_of_over_another_program_it_is_dropped_nowhere() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let source = window(1);
        egui::DragAndDrop::set_payload(&ctx, Item(7));
        carry.window_event(&ctx, source, size(), &entered());
        carry.window_event(&ctx, source, size(), &moved(-40.0, 300.0));
        carry.window_event(&ctx, source, size(), &button(MouseButton::Left, ElementState::Released));
        // the pointer leaves the window and enters none of the viewer's
        carry.window_event(&ctx, source, size(), &left());
        assert!(!carry.frame_start(&ctx));
        pass_with_release(&ctx);
        assert!(carry.frame_start(&ctx));
        assert!(!carry.holds());
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
    }

    #[test]
    fn a_drag_that_ends_in_its_window_is_not_carried() {
        let inside = Some(PhysicalPosition::new(400.0, 300.0));
        let out = Some(PhysicalPosition::new(900.0, 300.0));
        assert!(begins(true, out, size()));
        // released inside the window: an ordinary drop
        assert!(!begins(true, inside, size()));
        // nothing dragged: an ordinary release
        assert!(!begins(false, out, size()));
        assert!(!begins(true, None, size()));
        assert!(beyond(PhysicalPosition::new(800.0, 10.0), size()));
        assert!(beyond(PhysicalPosition::new(10.0, -1.0), size()));
        assert!(!beyond(PhysicalPosition::new(799.0, 599.0), size()));

        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        egui::DragAndDrop::set_payload(&ctx, Item(7));
        carry.window_event(&ctx, window(1), size(), &moved(400.0, 300.0));
        let release = carry.window_event(&ctx, window(1), size(), &button(MouseButton::Left, ElementState::Released));
        assert_eq!(release, (Event::Pass, Step::Nothing));
        assert!(!carry.holds());
    }
}
