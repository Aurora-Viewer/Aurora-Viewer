//! Fingers on a touch screen: which part of the viewer a finger belongs to,
//! what the fingers on the world ask for, and the movement stick's reading.
//!
//! Nothing here comes from the Legacy Viewer, which has no touch input. The
//! gestures stand for its mouse:
//!
//! | Fingers on the world            | Mouse it stands for                  |
//! |---------------------------------|--------------------------------------|
//! | one tap                         | left click (touch, sit, walk…)       |
//! | two taps in place               | double click                         |
//! | one finger held in place        | right click: the context menu        |
//! | one finger dragged              | camera orbit (the look in mouselook) |
//! | two fingers pinched or spread   | the wheel: zoom                      |
//! | two fingers dragged together    | camera pan                           |
//! | two fingers tapped together     | Esc: the camera back behind us       |
//!
//! This file is the logic alone, in window pixels and seconds, without the
//! viewer's state: src/app/touch.rs applies what it answers, and
//! src/ui/touch.rs draws the stick and the buttons.

use glam::Vec2;

/// The interface's finger, held still for longer than `LONG_PRESS`,
/// right-clicks where it is: the menus of the windows, of the inventory, of
/// the text fields. Once only per press (`spent`), and not for a finger that
/// drags. `start` and `now` are on the interface's clock, and the test is
/// egui's own for a press that is no longer a click
/// (`PointerState::could_any_button_be_click`): the frame that lets the
/// finger go is the first one whose release clicks nothing.
pub fn interface_long_press(start: f64, now: f64, moved: bool, spent: bool) -> bool {
    !moved && !spent && now - start > LONG_PRESS
}

/// `AURORA_SAFE_AREA="top,right,bottom,left"`: the insets a test gives the
/// window, in points (a tablet's rounded corners and home bar, without one).
pub fn safe_area(spec: &str) -> Option<[f32; 4]> {
    let mut out = [0.0; 4];
    let mut parts = spec.split(',');
    for slot in &mut out {
        let v: f32 = parts.next()?.trim().parse().ok()?;
        if !v.is_finite() || !(0.0..=400.0).contains(&v) {
            return None;
        }
        *slot = v;
    }
    parts.next().is_none().then_some(out)
}

/// What the viewer runs on, for what follows from it: the touch controls
/// from the start, and the size of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Desktop,
    Tablet,
    Phone,
}

impl Device {
    /// `AURORA_DEVICE=tablet|phone|desktop`: a test's device.
    pub fn parse(spec: &str) -> Option<Self> {
        match spec.trim().to_ascii_lowercase().as_str() {
            "desktop" => Some(Self::Desktop),
            "tablet" | "pad" | "ipad" => Some(Self::Tablet),
            "phone" | "iphone" => Some(Self::Phone),
            _ => None,
        }
    }

    /// The Preferences' interface scale is multiplied by this. A finger
    /// needs larger targets than a mouse (44 points, where the theme's
    /// buttons are 26 high): a tablet has the room for a third more. A
    /// phone has not: the windows only fit its screen a fifth smaller, and
    /// its user zooms the interface in from the Preferences if they wish.
    pub fn interface_scale(self) -> f32 {
        match self {
            Self::Desktop => 1.0,
            Self::Tablet => 1.3,
            Self::Phone => 0.8,
        }
    }

    /// A touch screen is the device's own input.
    pub fn touch_first(self) -> bool {
        self != Self::Desktop
    }
}

pub mod demo;

/// A finger's identifier, as the window system gives it.
pub type Id = u64;

/// A finger still for this long asks for the context menu (seconds): the
/// delay of UIKit's long press, which touch users know.
pub const LONG_PRESS: f64 = 0.5;
/// A finger may wander this far and still tap or hold in place (interface
/// points): a little over a fingertip's wobble.
pub const SLOP: f32 = 10.0;
/// Two fingers down and up within this long, in place, are a tap together.
pub const TWO_FINGER_TAP: f64 = 0.35;

/// The buttons the touch layer reads itself, so that they answer while
/// another finger holds the stick (the interface only follows one finger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// Jump, or fly up: held.
    Up,
    /// Crouch, or fly down: held.
    Down,
    /// Fly or land.
    Fly,
    /// The camera back behind the avatar.
    ResetView,
    /// Into or out of the first-person view.
    Mouselook,
}

impl Button {
    /// A button that acts for as long as it is held, like its key.
    pub fn held(self) -> bool {
        matches!(self, Self::Up | Self::Down)
    }
}

/// What a finger was given to when it landed; it keeps it until it lifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// A window, a bar, a menu: the interface handles it as a pointer.
    Interface,
    /// The movement stick.
    Stick,
    Button(Button),
    /// The 3D view.
    World,
}

/// What lies where a finger lands.
#[derive(Debug, Clone, Copy, Default)]
pub struct Landing {
    /// No world to touch: another screen, a menu or a prompt open (a touch
    /// beside a menu closes it).
    pub interface_only: bool,
    /// A window, a bar or a menu is drawn over the touch controls there.
    pub covered: bool,
    /// The touch button drawn there.
    pub button: Option<Button>,
    /// The interface has that point (`widgets::point_over_ui`).
    pub interface: bool,
    /// The stick is shown, free and allowed, and the point is in its zone.
    pub stick: bool,
}

/// Who a finger that lands is given to.
pub fn owner(at: Landing) -> Owner {
    if at.interface_only {
        Owner::Interface
    } else if let Some(button) = at.button.filter(|_| !at.covered) {
        Owner::Button(button)
    } else if at.interface {
        Owner::Interface
    } else if at.stick {
        Owner::Stick
    } else {
        Owner::World
    }
}

/// What the fingers on the world ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// One finger down and up in place, at this point.
    Tap(Vec2),
    /// One finger held in place, at this point.
    LongPress(Vec2),
    /// One finger moved by this much since its last report.
    Drag(Vec2),
    /// The dragging finger lifted.
    DragEnd,
    /// Two fingers moved: `zoom` is the ratio of their distance to the one
    /// before (above 1 when they spread), `pan` what their middle moved by,
    /// `at` where their middle is.
    Pinch { zoom: f32, pan: Vec2, at: Vec2 },
    /// Two fingers down and up together, in place.
    TwoFingerTap,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Finger {
    id: Id,
    start: Vec2,
    pos: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// No finger, or fingers whose gesture is over and that only wait to lift.
    Idle,
    /// One finger down, not moved yet: a tap, a long press or a drag to come.
    Pressed {
        since: f64,
    },
    Dragging,
    /// Two fingers; `still` while neither has moved (a tap together to come).
    Two {
        since: f64,
        still: bool,
    },
}

/// The fingers on the world and the gesture they are making.
#[derive(Debug, Clone)]
pub struct World {
    fingers: Vec<Finger>,
    state: State,
}

impl Default for World {
    fn default() -> Self {
        Self {
            fingers: Vec::with_capacity(2),
            state: State::Idle,
        }
    }
}

impl World {
    /// Fingers on the world now.
    pub fn count(&self) -> usize {
        self.fingers.len()
    }

    /// Whether a drag or a pinch is under way.
    #[cfg(test)]
    pub fn moving(&self) -> bool {
        matches!(self.state, State::Dragging | State::Two { still: false, .. })
    }

    /// A finger lands on the world.
    pub fn start(&mut self, id: Id, pos: Vec2, now: f64) -> Option<Gesture> {
        if self.fingers.iter().any(|f| f.id == id) {
            return None;
        }
        let before = self.state;
        self.fingers.push(Finger { id, start: pos, pos });
        self.state = match (self.fingers.len(), before) {
            (1, _) => State::Pressed { since: now },
            // a second finger: whatever the first was doing becomes a
            // two-finger gesture, timed from the first finger when it had
            // not moved (a tap together starts with its first finger)
            (2, State::Pressed { since }) => State::Two { since, still: true },
            (2, _) => State::Two { since: now, still: false },
            // a third finger and more are ignored, and end nothing
            _ => before,
        };
        // the drag the first finger was making ends with the second
        (self.fingers.len() == 2 && before == State::Dragging).then_some(Gesture::DragEnd)
    }

    /// A finger moves. `slop`: [`SLOP`] in window pixels.
    pub fn moved(&mut self, id: Id, pos: Vec2, slop: f32) -> Option<Gesture> {
        let index = self.fingers.iter().position(|f| f.id == id)?;
        let last = self.fingers[index].pos;
        if index >= 2 {
            self.fingers[index].pos = pos;
            return None;
        }
        match self.state {
            State::Idle => {
                self.fingers[index].pos = pos;
                None
            }
            State::Pressed { .. } => {
                let finger = &mut self.fingers[index];
                if finger.start.distance(pos) <= slop {
                    // (its position is not taken: the drag starts from where
                    // the finger landed, so the slop is not lost)
                    return None;
                }
                let delta = pos - finger.pos;
                finger.pos = pos;
                self.state = State::Dragging;
                Some(Gesture::Drag(delta))
            }
            State::Dragging => {
                self.fingers[index].pos = pos;
                Some(Gesture::Drag(pos - last))
            }
            State::Two { since, still } => {
                if still && self.fingers[index].start.distance(pos) <= slop {
                    return None;
                }
                let (a, b) = (self.fingers[0].pos, self.fingers[1].pos);
                self.fingers[index].pos = pos;
                let (a2, b2) = (self.fingers[0].pos, self.fingers[1].pos);
                self.state = State::Two { since, still: false };
                let (before, after) = (a.distance(b), a2.distance(b2));
                // fingers nearly on each other give no usable ratio
                let zoom = if before > 1.0 && after > 1.0 { after / before } else { 1.0 };
                let at = (a2 + b2) * 0.5;
                Some(Gesture::Pinch {
                    zoom,
                    pan: at - (a + b) * 0.5,
                    at,
                })
            }
        }
    }

    /// A finger lifts, or the system takes it away (`cancelled`: a system
    /// gesture, a call): then it asks for nothing.
    pub fn end(&mut self, id: Id, now: f64, cancelled: bool) -> Option<Gesture> {
        let index = self.fingers.iter().position(|f| f.id == id)?;
        let finger = self.fingers.remove(index);
        if index >= 2 {
            return None;
        }
        let before = self.state;
        // the fingers left wait to lift: a pinch does not turn into a drag
        // of its last finger, which would swing the camera as the hand leaves
        self.state = State::Idle;
        if cancelled {
            return (before == State::Dragging).then_some(Gesture::DragEnd);
        }
        match before {
            // (a finger held past the long press and lifted before a frame
            // saw it, in a hitch: no longer a tap)
            State::Pressed { since } => (now - since < LONG_PRESS).then_some(Gesture::Tap(finger.start)),
            State::Dragging => Some(Gesture::DragEnd),
            State::Two { since, still: true } if now - since <= TWO_FINGER_TAP => Some(Gesture::TwoFingerTap),
            State::Two { .. } | State::Idle => None,
        }
    }

    /// Every frame: the long press of a finger held in place.
    pub fn tick(&mut self, now: f64) -> Option<Gesture> {
        let State::Pressed { since } = self.state else {
            return None;
        };
        if now - since < LONG_PRESS {
            return None;
        }
        // the press is spent: lifting the finger asks for nothing more
        self.state = State::Idle;
        self.fingers.first().map(|f| Gesture::LongPress(f.start))
    }

    /// Every finger is forgotten (the window lost the focus, the world went away).
    pub fn clear(&mut self) -> Option<Gesture> {
        let dragging = self.state == State::Dragging;
        self.fingers.clear();
        self.state = State::Idle;
        dragging.then_some(Gesture::DragEnd)
    }
}

/// The wheel clicks a pinch stands for: spreading the fingers to twice their
/// distance zooms in as [`PINCH_CLICKS`] clicks of the wheel do (a click
/// towards the user, positive, zooms out: `Camera::scroll`).
pub fn pinch_clicks(zoom: f32) -> f32 {
    if !zoom.is_finite() || zoom <= 0.0 {
        return 0.0;
    }
    -zoom.log2() * PINCH_CLICKS
}

/// Wheel clicks for a pinch that doubles or halves the fingers' distance.
pub const PINCH_CLICKS: f32 = 6.0;

/// The movement stick: where the thumb landed and where it is now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stick {
    pub id: Id,
    pub origin: Vec2,
    pub pos: Vec2,
    /// When the thumb landed, on the gestures' clock.
    pub since: f64,
    /// The thumb has moved. Until it does, it may as well be a tap or a
    /// long press on what the world shows in the stick's corner: the
    /// stick's zone takes none of the world away.
    pub engaged: bool,
    /// The thumb landed on the ring that is drawn where the stick waits:
    /// it is the stick's whatever it does, and a thumb resting there
    /// before it pushes opens no menu.
    pub on_ring: bool,
}

/// The stick's travel from its centre to its rim (interface points).
pub const STICK_RADIUS: f32 = 56.0;
/// Under this share of the travel the stick is at rest.
pub const STICK_REST: f32 = 0.2;
/// Past this share of the travel, forward, the avatar runs.
pub const STICK_RUN: f32 = 0.92;
/// The fastest turn, at the rim (radians a second): a half turn in a
/// little under two seconds, near the Legacy Viewer's keyboard turn once its
/// ramp is over.
pub const STICK_TURN: f32 = 1.8;

impl Stick {
    pub fn new(id: Id, pos: Vec2, now: f64) -> Self {
        Self {
            id,
            origin: pos,
            pos,
            since: now,
            engaged: false,
            on_ring: false,
        }
    }

    /// A thumb that landed on the drawn ring.
    pub fn on_ring(id: Id, pos: Vec2, now: f64) -> Self {
        Self {
            on_ring: true,
            ..Self::new(id, pos, now)
        }
    }

    /// The thumb moves. `slop`: [`SLOP`] in window pixels.
    pub fn moved(&mut self, pos: Vec2, slop: f32) {
        self.pos = pos;
        self.engaged |= self.origin.distance(pos) > slop;
    }

    /// The thumb lifts: a tap on the world when it never moved and did not
    /// stay for a long press.
    pub fn lifted(&self, now: f64) -> Option<Gesture> {
        (!self.engaged && !self.on_ring && now - self.since < LONG_PRESS).then_some(Gesture::Tap(self.origin))
    }

    /// The thumb stays where it landed: a long press on the world.
    pub fn held(&self, now: f64) -> Option<Gesture> {
        (!self.engaged && !self.on_ring && now - self.since >= LONG_PRESS).then_some(Gesture::LongPress(self.origin))
    }

    /// The thumb's offset in shares of the travel, inside the unit circle:
    /// x to the right, y forward (up the screen). `radius` in window pixels.
    pub fn vector(&self, radius: f32) -> Vec2 {
        if radius <= 0.0 {
            return Vec2::ZERO;
        }
        let v = (self.pos - self.origin) / radius;
        Vec2::new(v.x, -v.y).clamp_length_max(1.0)
    }
}

/// What the stick asks of the avatar.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StickMove {
    pub forward: bool,
    pub back: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    /// Turn rate, radians a second, positive to the left.
    pub turn: f32,
    pub run: bool,
}

impl StickMove {
    /// The stick asks the avatar to move or to turn.
    pub fn any(&self) -> bool {
        self.forward || self.back || self.strafe_left || self.strafe_right || self.turn != 0.0
    }
}

/// An axis past its rest zone, rescaled so that it leaves the zone at 0 and
/// reaches 1 at the rim.
fn past_rest(v: f32) -> f32 {
    let m = v.abs();
    if m <= STICK_REST {
        0.0
    } else {
        ((m - STICK_REST) / (1.0 - STICK_REST)).min(1.0) * v.signum()
    }
}

/// The stick read as the keyboard is: up and down walk, sideways turns (the
/// further, the faster), and sideways steps aside in mouselook, where the
/// view turns with the look. The simulator moves an avatar by control flags,
/// so the walk itself is all or nothing; the turn alone is gradual.
pub fn stick_move(v: Vec2, mouselook: bool) -> StickMove {
    if v.length() <= STICK_REST {
        return StickMove::default();
    }
    let (x, y) = (past_rest(v.x), past_rest(v.y));
    let mut out = StickMove {
        forward: y > 0.0,
        back: y < 0.0,
        run: y > 0.0 && v.length() >= STICK_RUN,
        ..StickMove::default()
    };
    if mouselook {
        out.strafe_left = x < 0.0;
        out.strafe_right = x > 0.0;
    } else {
        // squared: small corrections near the centre, the full rate at the rim
        out.turn = -x * x.abs() * STICK_TURN;
    }
    out
}

/// Where a thumb may start the stick: the lower left of the window, clear
/// of the bars (`work`: the interface's work area, window pixels as
/// left, top, right, bottom).
pub fn stick_zone(work: [f32; 4]) -> [f32; 4] {
    let [left, top, right, bottom] = work;
    let (w, h) = ((right - left).max(0.0), (bottom - top).max(0.0));
    [left, bottom - h * 0.6, left + w * 0.38, bottom]
}

pub fn inside(rect: [f32; 4], p: Vec2) -> bool {
    p.x >= rect[0] && p.y >= rect[1] && p.x <= rect[2] && p.y <= rect[3]
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: f32 = SLOP;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn a_finger_down_and_up_in_place_is_a_tap_where_it_landed() {
        let mut w = World::default();
        assert_eq!(w.start(1, v(100.0, 200.0), 0.0), None);
        // a wobble inside the slop is not a drag
        assert_eq!(w.moved(1, v(104.0, 203.0), S), None);
        assert_eq!(w.end(1, 0.12, false), Some(Gesture::Tap(v(100.0, 200.0))));
        assert_eq!(w.count(), 0);
        assert_eq!(w.tick(5.0), None);
    }

    #[test]
    fn a_finger_held_in_place_opens_the_menu_once_and_its_lift_asks_nothing() {
        let mut w = World::default();
        w.start(1, v(50.0, 60.0), 10.0);
        assert_eq!(w.tick(10.0 + LONG_PRESS - 0.01), None);
        assert_eq!(w.tick(10.0 + LONG_PRESS), Some(Gesture::LongPress(v(50.0, 60.0))));
        assert_eq!(w.tick(11.0), None);
        // moving it afterwards does not orbit, lifting it does not tap
        assert_eq!(w.moved(1, v(150.0, 60.0), S), None);
        assert_eq!(w.end(1, 11.5, false), None);
    }

    #[test]
    fn a_finger_that_moves_drags_from_where_it_landed_and_never_taps() {
        let mut w = World::default();
        w.start(1, v(0.0, 0.0), 0.0);
        assert_eq!(w.moved(1, v(6.0, 0.0), S), None);
        // past the slop: the whole way from the landing point, slop included
        assert_eq!(w.moved(1, v(14.0, 2.0), S), Some(Gesture::Drag(v(14.0, 2.0))));
        assert!(w.moving());
        assert_eq!(w.moved(1, v(20.0, 5.0), S), Some(Gesture::Drag(v(6.0, 3.0))));
        // a drag is not a long press either
        assert_eq!(w.tick(9.0), None);
        assert_eq!(w.end(1, 9.5, false), Some(Gesture::DragEnd));
        assert!(!w.moving());
    }

    #[test]
    fn two_fingers_pinch_and_pan_and_their_last_finger_does_not_drag() {
        let mut w = World::default();
        w.start(1, v(100.0, 100.0), 0.0);
        w.start(2, v(200.0, 100.0), 0.05);
        // spreading to twice the distance, the middle staying put
        let g = w.moved(2, v(300.0, 100.0), S);
        assert_eq!(
            g,
            Some(Gesture::Pinch {
                zoom: 2.0,
                pan: v(50.0, 0.0),
                at: v(200.0, 100.0)
            })
        );
        // one finger moving down: the middle follows by half, the distance grows a little
        assert_eq!(
            w.moved(1, v(100.0, 140.0), S),
            Some(Gesture::Pinch {
                zoom: (200.0f32.powi(2) + 40.0f32.powi(2)).sqrt() / 200.0,
                pan: v(0.0, 20.0),
                at: v(200.0, 120.0)
            })
        );
        assert_eq!(w.end(2, 1.0, false), None);
        // the finger left behind waits to lift
        assert_eq!(w.moved(1, v(400.0, 400.0), S), None);
        assert_eq!(w.end(1, 1.2, false), None);
        assert_eq!(w.count(), 0);
    }

    #[test]
    fn a_second_finger_ends_the_drag_of_the_first() {
        let mut w = World::default();
        w.start(1, v(0.0, 0.0), 0.0);
        w.moved(1, v(30.0, 0.0), S);
        assert_eq!(w.start(2, v(90.0, 0.0), 0.4), Some(Gesture::DragEnd));
        assert!(matches!(w.moved(2, v(150.0, 0.0), S), Some(Gesture::Pinch { .. })));
    }

    #[test]
    fn two_fingers_tapped_together_reset_the_view_unless_they_move_or_linger() {
        let mut w = World::default();
        w.start(1, v(100.0, 100.0), 0.0);
        w.start(2, v(180.0, 110.0), 0.06);
        assert_eq!(w.moved(2, v(183.0, 112.0), S), None);
        assert_eq!(w.end(1, 0.2, false), Some(Gesture::TwoFingerTap));
        assert_eq!(w.end(2, 0.22, false), None);

        // held too long: nothing
        w.start(1, v(100.0, 100.0), 1.0);
        w.start(2, v(180.0, 110.0), 1.05);
        assert_eq!(w.end(1, 1.0 + TWO_FINGER_TAP + 0.1, false), None);
        w.end(2, 2.0, false);

        // moved: a pinch, nothing at the lift; and no long press meanwhile
        w.start(1, v(100.0, 100.0), 3.0);
        w.start(2, v(180.0, 100.0), 3.02);
        assert_eq!(w.tick(3.0 + LONG_PRESS + 1.0), None);
        assert!(w.moved(2, v(260.0, 100.0), S).is_some());
        assert_eq!(w.end(1, 3.1, false), None);
    }

    #[test]
    fn a_finger_the_system_takes_away_asks_for_nothing() {
        let mut w = World::default();
        w.start(1, v(10.0, 10.0), 0.0);
        assert_eq!(w.end(1, 0.1, true), None);
        w.start(1, v(10.0, 10.0), 1.0);
        w.moved(1, v(60.0, 10.0), S);
        assert_eq!(w.end(1, 1.1, true), Some(Gesture::DragEnd));
        w.start(1, v(10.0, 10.0), 2.0);
        w.moved(1, v(60.0, 10.0), S);
        assert_eq!(w.clear(), Some(Gesture::DragEnd));
        assert_eq!(w.count(), 0);
    }

    #[test]
    fn a_third_finger_changes_nothing() {
        let mut w = World::default();
        w.start(1, v(0.0, 0.0), 0.0);
        w.start(2, v(100.0, 0.0), 0.0);
        assert_eq!(w.start(3, v(50.0, 80.0), 0.1), None);
        assert_eq!(w.moved(3, v(50.0, 300.0), S), None);
        assert_eq!(
            w.moved(2, v(200.0, 0.0), S),
            Some(Gesture::Pinch {
                zoom: 2.0,
                pan: v(50.0, 0.0),
                at: v(100.0, 0.0)
            })
        );
        assert_eq!(w.end(3, 0.5, false), None);
        assert_eq!(w.count(), 2);
        // the same finger reported twice is one finger
        assert_eq!(w.start(2, v(0.0, 0.0), 0.6), None);
        assert_eq!(w.count(), 2);
    }

    #[test]
    fn a_pinch_stands_for_wheel_clicks() {
        assert_eq!(pinch_clicks(1.0), 0.0);
        assert_eq!(pinch_clicks(2.0), -PINCH_CLICKS);
        assert_eq!(pinch_clicks(0.5), PINCH_CLICKS);
        // a step of a frame is a fraction of a click, and steps add up
        let step = pinch_clicks(1.02);
        assert!(step < 0.0 && step > -0.25);
        assert!((pinch_clicks(1.1) + pinch_clicks(1.0 / 1.1)).abs() < 1e-5);
        assert_eq!(pinch_clicks(0.0), 0.0);
        assert_eq!(pinch_clicks(f32::NAN), 0.0);
    }

    #[test]
    fn the_stick_walks_turns_and_runs_like_the_keys() {
        let rest = stick_move(v(0.1, 0.1), false);
        assert_eq!(rest, StickMove::default());
        assert!(!rest.any());

        let ahead = stick_move(v(0.0, 0.6), false);
        assert!(ahead.forward && !ahead.back && !ahead.run && ahead.turn == 0.0);

        let run = stick_move(v(0.0, 1.0), false);
        assert!(run.forward && run.run);
        // backwards never runs
        let back = stick_move(v(0.0, -1.0), false);
        assert!(back.back && !back.forward && !back.run);

        // sideways alone turns on the spot: right is a negative yaw rate
        let right = stick_move(v(1.0, 0.0), false);
        assert!(!right.forward && !right.back && !right.strafe_right);
        assert!((right.turn + STICK_TURN).abs() < 1e-6);
        let left = stick_move(v(-1.0, 0.0), false);
        assert!((left.turn - STICK_TURN).abs() < 1e-6);
        // half way out turns much slower than half the full rate
        let gentle = stick_move(v(0.6, 0.0), false);
        assert!(gentle.turn < 0.0 && gentle.turn.abs() < STICK_TURN * 0.3);

        // forward and to the side: both at once
        let curve = stick_move(v(0.5, 0.7), false);
        assert!(curve.forward && curve.turn < 0.0 && curve.any());
    }

    #[test]
    fn the_stick_steps_aside_in_mouselook() {
        let m = stick_move(v(0.8, 0.5), true);
        assert!(m.forward && m.strafe_right && !m.strafe_left && m.turn == 0.0);
        let m = stick_move(v(-0.8, 0.0), true);
        assert!(m.strafe_left && !m.forward && m.turn == 0.0);
    }

    #[test]
    fn the_stick_vector_stays_in_the_unit_circle_with_forward_up_the_screen() {
        let s = Stick {
            pos: v(100.0, 444.0),
            ..Stick::new(1, v(100.0, 500.0), 0.0)
        };
        assert_eq!(s.vector(56.0), v(0.0, 1.0));
        let far = Stick { pos: v(900.0, 500.0), ..s };
        assert_eq!(far.vector(56.0), v(1.0, 0.0));
        let diag = Stick { pos: v(400.0, 200.0), ..s };
        assert!((diag.vector(56.0).length() - 1.0).abs() < 1e-6);
        assert_eq!(s.vector(0.0), Vec2::ZERO);
    }

    #[test]
    fn the_stick_starts_in_the_lower_left_of_the_work_area() {
        let zone = stick_zone([0.0, 40.0, 1000.0, 740.0]);
        for (got, want) in zone.iter().zip([0.0, 320.0, 380.0, 740.0]) {
            assert!((got - want).abs() < 1e-3, "{zone:?}");
        }
        assert!(inside(zone, v(100.0, 600.0)));
        assert!(!inside(zone, v(500.0, 600.0)));
        assert!(!inside(zone, v(100.0, 200.0)));
        assert!(!inside(zone, v(100.0, 760.0)));
    }

    #[test]
    fn only_the_vertical_buttons_are_held() {
        assert!(Button::Up.held() && Button::Down.held());
        assert!(!Button::Fly.held() && !Button::ResetView.held() && !Button::Mouselook.held());
    }
    #[test]
    fn the_interface_finger_right_clicks_once_when_held_still() {
        assert!(!interface_long_press(1.0, 1.4, false, false));
        // (at the delay itself egui still calls the release a click)
        assert!(!interface_long_press(1.0, 1.0 + LONG_PRESS, false, false));
        assert!(interface_long_press(1.0, 1.52, false, false));
        assert!(!interface_long_press(1.0, 2.0, true, false), "a drag is not a long press");
        assert!(!interface_long_press(1.0, 2.0, false, true), "once per press");
    }

    #[test]
    fn the_test_insets_are_four_sane_numbers() {
        assert_eq!(safe_area("24, 0,20,0"), Some([24.0, 0.0, 20.0, 0.0]));
        assert_eq!(safe_area("24,0,20"), None);
        assert_eq!(safe_area("24,0,20,0,1"), None);
        assert_eq!(safe_area("24,0,-1,0"), None);
        assert_eq!(safe_area("a,0,0,0"), None);
    }

    #[test]
    fn a_thumb_that_does_not_move_in_the_sticks_zone_taps_or_presses_the_world() {
        let at = v(120.0, 600.0);
        let mut s = Stick::new(4, at, 10.0);
        assert_eq!(s.held(10.3), None);
        assert_eq!(s.lifted(10.2), Some(Gesture::Tap(at)));
        assert_eq!(s.held(10.5), Some(Gesture::LongPress(at)));
        // (a jitter under the slop is no move)
        s.moved(v(123.0, 601.0), 10.0);
        assert_eq!(s.lifted(10.2), Some(Gesture::Tap(at)));
        // once it moves it is the stick, and stays so back at its centre
        s.moved(v(160.0, 600.0), 10.0);
        s.moved(at, 10.0);
        assert_eq!((s.lifted(10.2), s.held(11.0)), (None, None));
    }

    #[test]
    fn a_thumb_on_the_drawn_ring_is_the_sticks_and_never_the_worlds() {
        let s = Stick::on_ring(4, v(120.0, 600.0), 10.0);
        // resting there before pushing opens no menu, lifting clicks nothing
        assert_eq!((s.lifted(10.2), s.held(10.6), s.held(13.0)), (None, None, None));
    }

    #[test]
    fn a_finger_lands_on_what_is_drawn_on_top_there() {
        let world = Landing::default();
        assert_eq!(owner(world), Owner::World);
        assert_eq!(owner(Landing { stick: true, ..world }), Owner::Stick);
        // a button wins over its own area, which the interface counts as its own
        let fly = Landing {
            button: Some(Button::Fly),
            interface: true,
            ..world
        };
        assert_eq!(owner(fly), Owner::Button(Button::Fly));
        // a window over the button, or over the stick's corner, takes the finger
        assert_eq!(owner(Landing { covered: true, ..fly }), Owner::Interface);
        assert_eq!(
            owner(Landing {
                interface: true,
                stick: true,
                ..world
            }),
            Owner::Interface
        );
        // a menu open, another screen: nothing reaches the controls
        let menu = Landing {
            interface_only: true,
            button: Some(Button::Fly),
            stick: true,
            ..world
        };
        assert_eq!(owner(menu), Owner::Interface);
    }

    #[test]
    fn a_finger_lifted_after_the_long_press_delay_is_not_a_tap() {
        // a hitch: no frame ran between the delay and the lift
        let mut w = World::default();
        w.start(1, v(50.0, 60.0), 10.0);
        assert_eq!(w.end(1, 10.0 + LONG_PRESS + 0.1, false), None);
        w.start(1, v(50.0, 60.0), 20.0);
        assert_eq!(w.end(1, 20.0 + LONG_PRESS - 0.1, false), Some(Gesture::Tap(v(50.0, 60.0))));
    }

    #[test]
    fn a_device_is_named_by_a_test_and_sizes_the_interface() {
        assert_eq!(Device::parse(" iPad "), Some(Device::Tablet));
        assert_eq!(Device::parse("phone"), Some(Device::Phone));
        assert_eq!(Device::parse("desktop"), Some(Device::Desktop));
        assert_eq!(Device::parse("watch"), None);
        assert_eq!(Device::Desktop.interface_scale(), 1.0);
        // a theme button of 26 points reaches 33 on a tablet; the touch
        // controls' own buttons stay above 40 on a phone
        assert!(Device::Tablet.interface_scale() * 26.0 > 33.0);
        assert!(Device::Phone.interface_scale() * 52.0 > 40.0);
        assert!(Device::Tablet.touch_first() && Device::Phone.touch_first() && !Device::Desktop.touch_first());
    }
}
