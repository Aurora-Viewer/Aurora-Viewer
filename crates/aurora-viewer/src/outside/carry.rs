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
//! Where the system does tell where the windows are (Windows, macOS), the
//! held drag goes on over the other window: the place of the pointer,
//! which the window the drag began in still gets, is found in the window
//! under it (`under`), which reads it as its own pointer and shows what
//! would take the item. The release there is the drop, with no click, once
//! the system says the pointer is over that window (it enters it when the
//! button is let go of, or when the pointer next moves; `DROP_WAIT`): the
//! carry begins and ends at once. Over a window of the viewer that another
//! program covers, nothing says so and the item is dropped nowhere.
//!
//! Nothing here knows what is carried: the cargo is egui's
//! (`egui::DragAndDrop`), read and taken by the same code as a drag inside
//! one window, with the same checks of the creators' permissions. The
//! carry adds no way to drop, only the time between the release and the
//! click.
//!
//! Not in the Legacy Viewer, whose floaters stay inside its window.

use std::collections::HashMap;
use std::time::{Duration, Instant};

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
    /// The drag was let go of over this window of the viewer, at this
    /// place in its pixels: its drop, with no click, once the system says
    /// the pointer is over that window, which it is waited for until
    /// `until`.
    drop: Option<Over>,
    until: Instant,
}

/// How long the drop of a held drag waits for the system to say the pointer
/// is over the window it was let go of over: it says so at once (Windows)
/// or when the pointer next moves.
const DROP_WAIT: Duration = Duration::from_millis(250);

/// A window of the viewer under a held drag that began in another, and the
/// pointer in its pixels.
pub(crate) type Over = (WindowId, PhysicalPosition<f64>);

/// A window of the viewer on the screen, where the system tells it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placed {
    pub id: WindowId,
    /// Where its inside begins on the screen, in the units all the windows
    /// share: pixels, or points on macOS, whose screens of different
    /// scales have only points in common.
    pub at: (f64, f64),
    pub size: PhysicalSize<u32>,
    /// Those units in one pixel of this window.
    pub unit: f64,
    /// When it last took the focus (`Carry::front`): the later is in front.
    pub front: u64,
}

impl Placed {
    /// None where the system tells no program where its windows are
    /// (Wayland).
    pub fn of(window: &winit::window::Window, front: u64) -> Option<Self> {
        let at = window.inner_position().ok()?;
        // (winit gives a place in pixels of the window's own scale there)
        let unit = if cfg!(target_os = "macos") {
            1.0 / window.scale_factor()
        } else {
            1.0
        };
        Some(Placed {
            id: window.id(),
            at: (f64::from(at.x) * unit, f64::from(at.y) * unit),
            size: window.inner_size(),
            unit,
            front,
        })
    }
}

/// The window of the viewer under the pointer of a drag held in `from`,
/// at `at` in the pixels of `from`, and the pointer in that window's
/// pixels: the one in front where several are there.
pub(crate) fn under(windows: &[Placed], from: WindowId, at: PhysicalPosition<f64>) -> Option<Over> {
    let origin = windows.iter().find(|window| window.id == from)?;
    let on_screen = (origin.at.0 + at.x * origin.unit, origin.at.1 + at.y * origin.unit);
    windows
        .iter()
        .filter(|window| window.id != from)
        .filter_map(|window| {
            let there = PhysicalPosition::new((on_screen.0 - window.at.0) / window.unit, (on_screen.1 - window.at.1) / window.unit);
            (!beyond(there, window.size)).then_some((window, there))
        })
        .max_by_key(|(window, _)| window.front)
        .map(|(window, there)| (window.id, there))
}

/// What a window under a held drag reads of it: the keys held, which it
/// does not hear without the keyboard, and the pointer at `at`, in its
/// points, with the release that drops when `released`; or the pointer
/// gone, and no key held.
pub(crate) fn pointer_events(at: Option<egui::Pos2>, released: bool, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let Some(at) = at else {
        return vec![egui::Event::PointerGone, egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
    };
    let mut events = vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(at)];
    if released {
        events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers,
        });
    }
    events
}

#[derive(Default)]
pub(crate) struct Carry {
    pointers: HashMap<WindowId, Pointer>,
    in_hand: Option<InHand>,
    /// A press the carry swallowed, whose release goes with it.
    swallowed: Option<MouseButton>,
    /// Escape let go of the item: its release goes with the press.
    escape: bool,
    /// The window the left button was pressed in, while it is held.
    pressed_in: Option<WindowId>,
    /// When each window last took the focus.
    fronts: HashMap<WindowId, u64>,
    focuses: u64,
    /// A held drag is over another window than the one it began in.
    over: Option<Over>,
    /// The drop of a drag let go of over another window, for this frame.
    drop_now: Option<Over>,
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

fn over_id() -> egui::Id {
    egui::Id::unique("aurora carry over")
}

/// An item is in hand.
pub(crate) fn holding(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(holding_id())).unwrap_or(false)
}

/// The left button is held or an item is in hand: what a drag shows while
/// it lasts (its cursor, its badge) shows. Held in another window too,
/// over this one, whose own pointer has no button down.
pub(crate) fn dragging(ctx: &egui::Context) -> bool {
    holding(ctx) || ctx.data(|d| d.get_temp::<bool>(over_id())).unwrap_or(false) || ctx.input(|i| i.pointer.primary_down())
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
                // the pointer moved in the window a drop is waited for in:
                // its place there now, from the system itself
                if let Some((_, at)) = self
                    .in_hand
                    .as_mut()
                    .and_then(|in_hand| in_hand.drop.as_mut().filter(|(to, _)| *to == id))
                {
                    *at = *position;
                }
            }
            WindowEvent::CursorEntered { .. } => self.pointers.entry(id).or_default().over = true,
            WindowEvent::CursorLeft { .. } => self.pointers.entry(id).or_default().over = false,
            WindowEvent::Destroyed => {
                self.pointers.remove(&id);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // (a click while a drop is waited for is what drops)
                if let Some(in_hand) = self.in_hand.as_mut().filter(|_| *state == ElementState::Pressed) {
                    in_hand.drop = None;
                }
                let took = self.button(ctx, id, size, *state, *button);
                if *button == MouseButton::Left {
                    self.pressed_in = (*state == ElementState::Pressed && took.0 == Event::Pass).then_some(id);
                    self.over = None;
                }
                return took;
            }
            WindowEvent::Focused(true) => {
                self.focuses += 1;
                self.fronts.insert(id, self.focuses);
            }
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
        self.fronts.retain(|id, _| open(*id));
        self.over = self.over.filter(|(id, _)| open(*id));
        if let Some(in_hand) = &mut self.in_hand {
            in_hand.drop = in_hand.drop.filter(|(id, _)| open(*id));
        }
    }

    /// When the window `id` last took the focus, for `Placed::front`.
    pub fn front(&self, id: WindowId) -> u64 {
        self.fronts.get(&id).copied().unwrap_or(0)
    }

    /// Where the pointer of a drag held in the window `id`, of the size
    /// `size`, is beyond its edges: what `under` looks another window up
    /// with.
    pub fn held_beyond(&self, ctx: &egui::Context, id: WindowId, size: PhysicalSize<u32>) -> Option<PhysicalPosition<f64>> {
        if self.pressed_in != Some(id) {
            return None;
        }
        let at = self.pointers.get(&id)?.at.filter(|at| beyond(*at, size))?;
        egui::DragAndDrop::has_any_payload(ctx).then_some(at)
    }

    /// The system says the pointer is over the window `id`.
    pub fn pointer_over(&self, id: WindowId) -> bool {
        self.pointers.get(&id).is_some_and(|pointer| pointer.over)
    }

    /// The window a held drag is over, other than its own.
    pub fn over(&self) -> Option<WindowId> {
        self.over.map(|(id, _)| id)
    }

    /// The held drag is over `over` now: the window it was over and has
    /// left, if any.
    pub fn set_over(&mut self, over: Option<Over>) -> Option<WindowId> {
        let was = self.over().filter(|was| Some(*was) != over.map(|(id, _)| id));
        self.over = over;
        was
    }

    /// The drop of a drag let go of over another window, once per drop:
    /// that window reads the release in this frame's pass.
    pub fn take_drop(&mut self) -> Option<Over> {
        self.drop_now.take()
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
                    drop: self.over.take(),
                    until: Instant::now() + DROP_WAIT,
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
        self.frame_start_at(ctx, Instant::now())
    }

    fn frame_start_at(&mut self, ctx: &egui::Context, now: Instant) -> bool {
        let over_a_window = self.pointers.values().any(|pointer| pointer.over);
        let cargo_there = egui::DragAndDrop::has_any_payload(ctx);
        let mut ended = false;
        if let Some(in_hand) = &mut self.in_hand {
            // the drop of a held drag, once the system says the pointer is
            // over that window: where the windows are does not tell what
            // another program covers, and an item is never dropped in a
            // window that is hidden
            // (at the place worked out, or where the pointer moved to since:
            // the place that window's pointer had before is an old one)
            let said = in_hand
                .drop
                .filter(|(id, _)| self.pointers.get(id).is_some_and(|pointer| pointer.over));
            if said.is_none() && now >= in_hand.until {
                in_hand.drop = None;
            }
            if in_hand.held {
                ended = !cargo_there;
            } else if !cargo_there {
                if over_a_window {
                    let cargo = in_hand.cargo.clone();
                    ctx.with_plugin::<egui::DragAndDrop, _>(|plugin| *plugin = cargo);
                    in_hand.held = true;
                } else {
                    ended = in_hand.drop.is_none();
                }
            }
            if in_hand.held && !ended && said.is_some() {
                in_hand.drop = None;
                self.drop_now = said;
            }
            if in_hand.drop.is_some() {
                ctx.request_repaint();
            }
        }
        if ended {
            self.in_hand = None;
        }
        // (the frame of a drop with no click is the drag's last, not an
        // item in hand that waits for a click)
        let waited = self.in_hand.as_ref().is_some_and(|in_hand| in_hand.drop.is_some());
        let dropping = self.drop_now.is_some() || waited;
        let holding = !dropping && self.in_hand.as_ref().is_some_and(|in_hand| in_hand.held);
        let over = dropping || self.over.is_some();
        ctx.data_mut(|d| {
            for (id, on) in [(holding_id(), holding), (over_id(), over)] {
                if on {
                    d.insert_temp(id, true);
                } else {
                    d.remove::<bool>(id);
                }
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

    fn placed(id: u64, at: (f64, f64), size: (u32, u32), unit: f64, front: u64) -> Placed {
        Placed {
            id: window(id),
            at,
            size: PhysicalSize::new(size.0, size.1),
            unit,
            front,
        }
    }

    /// Where the windows share pixels (Windows): the pointer of the window
    /// the drag began in, beyond its edges, is found in the one under it.
    #[test]
    fn the_window_under_a_held_drag_is_found_in_pixels() {
        let windows = [
            placed(1, (100.0, 100.0), (800, 600), 1.0, 5),
            placed(2, (1000.0, 200.0), (400, 300), 1.0, 1),
        ];
        // 950 pixels right of window 1's inside: 50 into window 2
        assert_eq!(
            under(&windows, window(1), PhysicalPosition::new(950.0, 150.0)),
            Some((window(2), PhysicalPosition::new(50.0, 50.0)))
        );
        // between the two, and past the far edge of window 2: no window
        assert_eq!(under(&windows, window(1), PhysicalPosition::new(850.0, 150.0)), None);
        assert_eq!(under(&windows, window(1), PhysicalPosition::new(1300.0, 150.0)), None);
        // the other way round, to the left of window 2's inside
        assert_eq!(
            under(&windows, window(2), PhysicalPosition::new(-500.0, 100.0)),
            Some((window(1), PhysicalPosition::new(400.0, 200.0)))
        );
        // a window the system gave no place: nothing to look up from
        assert_eq!(under(&windows[1..], window(1), PhysicalPosition::new(950.0, 150.0)), None);
    }

    /// macOS: the screens share points, a window on a screen at 200 % has
    /// two pixels in each.
    #[test]
    fn the_window_under_a_held_drag_is_found_across_screens_of_two_scales() {
        let windows = [
            // 800 x 600 points on the screen at 200 %
            placed(1, (100.0, 100.0), (1600, 1200), 0.5, 5),
            // on the screen at 100 %, to its right
            placed(2, (1000.0, 200.0), (400, 300), 1.0, 1),
        ];
        // 1900 pixels of window 1 are 950 points: 50 points into window 2
        assert_eq!(
            under(&windows, window(1), PhysicalPosition::new(1900.0, 300.0)),
            Some((window(2), PhysicalPosition::new(50.0, 50.0)))
        );
        // and back: 500 points left of window 2 are 400 points into
        // window 1, 800 of its pixels
        assert_eq!(
            under(&windows, window(2), PhysicalPosition::new(-500.0, 100.0)),
            Some((window(1), PhysicalPosition::new(800.0, 400.0)))
        );
    }

    #[test]
    fn of_two_windows_under_a_held_drag_the_one_in_front_takes_it() {
        let behind = placed(2, (1000.0, 200.0), (400, 300), 1.0, 3);
        let front = placed(3, (1020.0, 220.0), (400, 300), 1.0, 7);
        let source = placed(1, (100.0, 100.0), (800, 600), 1.0, 9);
        let at = PhysicalPosition::new(950.0, 150.0);
        assert_eq!(
            under(&[source, behind, front], window(1), at),
            Some((window(3), PhysicalPosition::new(30.0, 30.0)))
        );
        assert_eq!(
            under(&[source, front, behind], window(1), at),
            Some((window(3), PhysicalPosition::new(30.0, 30.0)))
        );
        // where only the one behind is
        let at = PhysicalPosition::new(910.0, 110.0);
        assert_eq!(
            under(&[source, behind, front], window(1), at),
            Some((window(2), PhysicalPosition::new(10.0, 10.0)))
        );
    }

    /// The front window is the one that took the focus last.
    #[test]
    fn the_focus_tells_which_window_is_in_front() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        assert_eq!(carry.front(window(1)), 0);
        carry.window_event(&ctx, window(1), size(), &WindowEvent::Focused(true));
        carry.window_event(&ctx, window(2), size(), &WindowEvent::Focused(true));
        assert!(carry.front(window(2)) > carry.front(window(1)));
        carry.window_event(&ctx, window(1), size(), &WindowEvent::Focused(true));
        assert!(carry.front(window(1)) > carry.front(window(2)));
        carry.keep(|id| id == window(1));
        assert_eq!(carry.front(window(2)), 0);
    }

    /// Only a drag (the left button held, something dragged) beyond the
    /// edges of its window looks for another window.
    #[test]
    fn a_drag_held_beyond_its_window_looks_for_another() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let source = window(1);
        carry.window_event(&ctx, source, size(), &moved(900.0, 300.0));
        // no button held
        egui::DragAndDrop::set_payload(&ctx, Item(7));
        assert_eq!(carry.held_beyond(&ctx, source, size()), None);
        carry.window_event(&ctx, source, size(), &button(MouseButton::Left, ElementState::Pressed));
        assert_eq!(carry.held_beyond(&ctx, source, size()), Some(PhysicalPosition::new(900.0, 300.0)));
        // another window's pointer is not this drag's
        assert_eq!(carry.held_beyond(&ctx, window(2), size()), None);
        // inside its own window
        carry.window_event(&ctx, source, size(), &moved(400.0, 300.0));
        assert_eq!(carry.held_beyond(&ctx, source, size()), None);
        // nothing dragged
        carry.window_event(&ctx, source, size(), &moved(900.0, 300.0));
        egui::DragAndDrop::clear_payload(&ctx);
        assert_eq!(carry.held_beyond(&ctx, source, size()), None);
    }

    /// A drag let go of over another window, where the system tells where
    /// the windows are: that window reads the release in the frame the
    /// cargo is back, with no item left in hand for a click.
    #[test]
    fn a_drag_let_go_of_over_another_window_is_dropped_there_with_no_click() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let (source, target) = (window(1), window(2));
        carry.window_event(&ctx, source, size(), &button(MouseButton::Left, ElementState::Pressed));
        egui::DragAndDrop::set_payload(&ctx, Item(7));
        carry.window_event(&ctx, source, size(), &moved(900.0, 300.0));
        let there = PhysicalPosition::new(120.0, 80.0);
        assert_eq!(carry.set_over(Some((target, there))), None);
        assert_eq!(carry.over(), Some(target));
        assert!(!carry.frame_start(&ctx));
        assert!(dragging(&ctx), "the window under the drag shows it");
        assert!(!holding(&ctx));
        assert_eq!(carry.take_drop(), None);
        let began = carry.window_event(&ctx, source, size(), &button(MouseButton::Left, ElementState::Released));
        assert_eq!(began, (Event::Pass, Step::Began));
        assert_eq!(carry.over(), None);
        // the button let go of, the system gives the pointer to the window
        // under it, where it really is
        carry.window_event(&ctx, target, size(), &entered());
        carry.window_event(&ctx, target, size(), &moved(121.0, 81.0));
        // the window the drag began in reads its release first
        assert!(!carry.frame_start(&ctx));
        assert_eq!(carry.take_drop(), None);
        pass_with_release(&ctx);
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
        // the cargo is back, and the drop is this frame's
        assert!(!carry.frame_start(&ctx));
        assert_eq!(egui::DragAndDrop::payload::<Item>(&ctx).as_deref(), Some(&Item(7)));
        assert!(dragging(&ctx));
        assert!(!holding(&ctx), "no note that asks for a click");
        assert_eq!(carry.take_drop(), Some((target, PhysicalPosition::new(121.0, 81.0))));
        assert_eq!(carry.take_drop(), None, "once");
        // the target's pass reads the release: taken, or cleared by egui
        pass_with_release(&ctx);
        assert!(carry.frame_start(&ctx), "nothing in hand any more");
        assert!(!carry.holds());
        assert!(!dragging(&ctx));
    }

    /// A held drag let go of where a window of the viewer is, with nothing
    /// from the system to say the pointer is over it.
    fn let_go_of_over(ctx: &egui::Context, carry: &mut Carry, target: WindowId) {
        let source = window(1);
        carry.window_event(ctx, source, size(), &button(MouseButton::Left, ElementState::Pressed));
        egui::DragAndDrop::set_payload(ctx, Item(7));
        carry.window_event(ctx, source, size(), &moved(900.0, 300.0));
        carry.set_over(Some((target, PhysicalPosition::new(120.0, 80.0))));
        let began = carry.window_event(ctx, source, size(), &button(MouseButton::Left, ElementState::Released));
        assert_eq!(began, (Event::Pass, Step::Began));
    }

    /// Another program covers the window of the viewer the drag was let go
    /// of over: the pointer enters none of the viewer's, and the item is
    /// dropped nowhere.
    #[test]
    fn a_drag_let_go_of_over_a_window_another_program_covers_is_dropped_nowhere() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let_go_of_over(&ctx, &mut carry, window(2));
        assert!(!carry.frame_start(&ctx));
        pass_with_release(&ctx);
        // waited for a moment, the cargo not back in egui
        assert!(!carry.frame_start(&ctx));
        assert_eq!(carry.take_drop(), None);
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
        assert!(!holding(&ctx));
        assert!(carry.frame_start_at(&ctx, Instant::now() + DROP_WAIT), "nothing in hand");
        assert_eq!(carry.take_drop(), None);
        assert!(!carry.holds());
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
        assert!(!dragging(&ctx));
    }

    /// The system says the pointer is over the window a few frames after
    /// the button was let go of (when the pointer next moves): the drop
    /// waited for it.
    #[test]
    fn the_drop_of_a_held_drag_waits_for_the_pointer_to_enter_its_window() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let target = window(2);
        carry.window_event(&ctx, target, size(), &entered());
        carry.window_event(&ctx, target, size(), &moved(700.0, 500.0));
        carry.window_event(&ctx, target, size(), &left());
        let_go_of_over(&ctx, &mut carry, target);
        assert!(!carry.frame_start(&ctx));
        pass_with_release(&ctx);
        assert!(!carry.frame_start(&ctx));
        assert!(!carry.frame_start(&ctx));
        assert_eq!(carry.take_drop(), None);
        // no place with the entry: the one worked out, not the one the
        // pointer had when it last left that window
        carry.window_event(&ctx, target, size(), &entered());
        assert!(!carry.frame_start(&ctx));
        assert_eq!(egui::DragAndDrop::payload::<Item>(&ctx).as_deref(), Some(&Item(7)));
        assert!(!holding(&ctx), "no note that asks for a click");
        assert_eq!(carry.take_drop(), Some((target, PhysicalPosition::new(120.0, 80.0))));
        pass_with_release(&ctx);
        assert!(carry.frame_start(&ctx));
        assert!(!carry.holds());
    }

    /// The pointer is in another window of the viewer than the one worked
    /// out: the item stays in hand there, for a click.
    #[test]
    fn a_drag_let_go_of_over_another_window_than_the_pointer_enters_stays_in_hand() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let_go_of_over(&ctx, &mut carry, window(2));
        carry.window_event(&ctx, window(3), size(), &entered());
        carry.window_event(&ctx, window(3), size(), &moved(40.0, 40.0));
        assert!(!carry.frame_start(&ctx));
        pass_with_release(&ctx);
        assert!(!carry.frame_start(&ctx));
        assert_eq!(carry.take_drop(), None);
        assert!(carry.holds());
        assert!(egui::DragAndDrop::has_any_payload(&ctx));
        assert!(!holding(&ctx), "no note while the drop is waited for");
        assert!(!carry.frame_start_at(&ctx, Instant::now() + DROP_WAIT));
        assert_eq!(carry.take_drop(), None);
        assert!(holding(&ctx), "the note asks for a click");
        // the window worked out, entered later: the click drops, not it
        carry.window_event(&ctx, window(2), size(), &entered());
        assert!(!carry.frame_start(&ctx));
        assert_eq!(carry.take_drop(), None);
    }

    /// A click while the drop is waited for is the drop: no second one
    /// when the pointer then enters the window worked out.
    #[test]
    fn a_click_while_a_drop_is_waited_for_is_the_only_drop() {
        let ctx = egui::Context::default();
        let mut carry = Carry::default();
        let_go_of_over(&ctx, &mut carry, window(2));
        carry.window_event(&ctx, window(3), size(), &entered());
        assert!(!carry.frame_start(&ctx));
        pass_with_release(&ctx);
        assert!(!carry.frame_start(&ctx));
        let press = carry.window_event(&ctx, window(3), size(), &button(MouseButton::Left, ElementState::Pressed));
        assert_eq!(press, (Event::Swallow, Step::Nothing));
        carry.window_event(&ctx, window(2), size(), &entered());
        assert!(!carry.frame_start(&ctx));
        assert_eq!(carry.take_drop(), None);
    }

    /// The drag comes back into its own window, or its target closes: no
    /// window is under it any more.
    #[test]
    fn a_held_drag_leaves_the_window_it_was_over() {
        let mut carry = Carry::default();
        let there = PhysicalPosition::new(120.0, 80.0);
        assert_eq!(carry.set_over(Some((window(2), there))), None);
        // moved on in the same window: nothing left
        assert_eq!(carry.set_over(Some((window(2), PhysicalPosition::new(130.0, 80.0)))), None);
        // to another window, then to none
        assert_eq!(carry.set_over(Some((window(3), there))), Some(window(2)));
        assert_eq!(carry.set_over(None), Some(window(3)));
        assert_eq!(carry.set_over(None), None);
        carry.set_over(Some((window(2), there)));
        carry.keep(|id| id == window(1));
        assert_eq!(carry.over(), None);
    }

    #[test]
    fn the_window_under_a_held_drag_reads_the_pointer_and_the_keys_held() {
        let ctrl = egui::Modifiers::CTRL;
        let at = egui::pos2(40.0, 30.0);
        let moved = pointer_events(Some(at), false, ctrl);
        assert_eq!(moved, vec![egui::Event::ModifiersChanged(ctrl), egui::Event::PointerMoved(at)]);
        let dropped = pointer_events(Some(at), true, ctrl);
        assert_eq!(dropped.len(), 3);
        assert!(matches!(
            dropped[2],
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            }
        ));
        let gone = pointer_events(None, false, ctrl);
        assert_eq!(
            gone,
            vec![egui::Event::PointerGone, egui::Event::ModifiersChanged(egui::Modifiers::NONE)]
        );
    }
}
