//! The touch screen (src/touch.rs): each finger is given to the interface,
//! the movement stick, a touch button or the world when it lands, and keeps
//! that until it lifts. The world's gestures call what the mouse calls
//! (`mouse_input`, `camera_mouse_delta`, `world_wheel`, `open_context_menu`),
//! so a tap is a click with everything a click does: touch, sit, the name
//! plates, the double click on the ground, the build tools.
//!
//! The interface only follows one finger (egui-winit makes the first one its
//! pointer). The fingers the touch layer keeps are therefore not shown to
//! it: the thumb on the stick leaves the pointer free for a button of a
//! window.

use super::*;
use crate::touch::{self, Button, Gesture, Owner, Stick};
use winit::event::TouchPhase;

/// A tap holds the left button for this long: a click, not a drag (the
/// scripted demo clicks hold it as long).
const TAP_HOLD: Duration = Duration::from_millis(60);

/// Where the cursor is put once a finger leaves the world, on a screen
/// without a mouse, so that nothing is hovered: no finger rests there as a
/// mouse does.
const NOWHERE: (f32, f32) = (-1.0e4, -1.0e4);

pub(super) struct State {
    /// Fingers down, and what each was given to.
    owners: Vec<(touch::Id, Owner)>,
    /// Fingers still down whose gesture was dropped (`touch_drop`): they
    /// ask for nothing more, and are not handed to the interface half way.
    dropped: Vec<touch::Id>,
    world: touch::World,
    pub stick: Option<Stick>,
    /// Touch buttons a finger holds now.
    held: Vec<Button>,
    /// Touch buttons the mouse holds now (the last interface pass).
    mouse_held: Vec<Button>,
    /// A touch button the mouse clicked in the last interface pass.
    mouse_clicked: Option<Button>,
    /// A tap's left button, to release at this time.
    tap_release: Option<Instant>,
    /// The orbit the dragging finger started (the mouse mode is its own).
    orbiting: bool,
    /// A finger has touched the window in this session.
    pub seen: bool,
    /// The touch buttons of the last interface pass, in interface points:
    /// none when the pass did not draw them.
    buttons: Vec<(Button, egui::Rect)>,
    /// The last interface pass drew the controls: a thumb can start the stick.
    shown: bool,
    /// Where the mouse last was, to put the cursor back after a gesture.
    pub mouse_at: Option<(f32, f32)>,
    /// The start of the gestures' clock.
    epoch: Instant,
    /// AURORA_DEMO_TOUCH: the script's fingers (src/touch/demo.rs).
    pub demo: demo::Demo,
    /// The interface's finger (the first one on it: the one egui follows).
    iface: Option<Iface>,
    /// Events for the interface's next pass: the long press's right click,
    /// and the script's fingers on the interface.
    pending: Vec<egui::Event>,
}

/// The finger the interface follows, watched for a long press.
struct Iface {
    id: touch::Id,
    origin: glam::Vec2,
    /// When the interface saw it land, on the interface's clock: the first
    /// pass after it landed, as egui times its press.
    pressed: Option<f64>,
    /// It moved, or another finger joined it: not a long press.
    moved: bool,
    /// It has right-clicked: the rest of this press is not the interface's.
    spent: bool,
}

/// What becomes of a finger event of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handoff {
    /// The touch layer's own: the interface does not see it.
    Kept,
    /// The interface's, as it came.
    Interface,
    /// The interface's finger lifted after its long press: the interface
    /// is told the touch was cancelled, so that the lift releases nothing.
    InterfaceCancelled,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owners: Vec::new(),
            dropped: Vec::new(),
            world: touch::World::default(),
            stick: None,
            held: Vec::new(),
            mouse_held: Vec::new(),
            mouse_clicked: None,
            tap_release: None,
            orbiting: false,
            seen: false,
            buttons: Vec::new(),
            shown: false,
            mouse_at: None,
            epoch: Instant::now(),
            demo: demo::Demo::default(),
            iface: None,
            pending: Vec::new(),
        }
    }
}

impl State {
    pub fn held(&self, button: Button) -> bool {
        self.held.contains(&button) || self.mouse_held.contains(&button)
    }

    fn owner(&self, id: touch::Id) -> Option<Owner> {
        self.owners.iter().find(|(i, _)| *i == id).map(|(_, o)| *o)
    }

    fn forget(&mut self, id: touch::Id) -> Option<Owner> {
        let index = self.owners.iter().position(|(i, _)| *i == id)?;
        Some(self.owners.remove(index).1)
    }

    /// The controls this interface pass drew, and what the mouse did on
    /// them: its click acts at the next frame's start, out of the pass.
    pub fn shown(&mut self, shown: ui::touch::Shown) {
        self.buttons = shown.buttons;
        self.shown = true;
        self.mouse_held = shown.held;
        if shown.clicked.is_some() {
            self.mouse_clicked = shown.clicked;
        }
    }

    /// A finger or a gesture is the touch layer's now.
    fn busy(&self) -> bool {
        self.stick.is_some()
            || !self.held.is_empty()
            || self.world.count() > 0
            || self.owners.iter().any(|(_, owner)| *owner != Owner::Interface)
    }
}

/// What the interface is told when its finger has stayed still: the finger
/// lets go, then a right click where it is. The pass that gets them is the
/// first one whose release is no click (`touch::interface_long_press`), so
/// nothing is clicked, no button stays down (a button left down would keep
/// every widget from being hovered, and every press-and-hold of a window
/// running), and egui's own long touch, which needs the button down, does
/// not answer a second time.
fn hold_events(pos: egui::Pos2) -> [egui::Event; 3] {
    let button = |button, pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    [
        button(egui::PointerButton::Primary, false),
        button(egui::PointerButton::Secondary, true),
        button(egui::PointerButton::Secondary, false),
    ]
}

/// How long a press stays a click for the interface: the long press's delay
/// while a finger is its pointer (so that the finger let go by `hold_events`
/// clicks nothing), egui's own for the mouse.
fn click_delay(ctx: &egui::Context, finger: bool) {
    let delay = if finger {
        touch::LONG_PRESS
    } else {
        egui::InputOptions::default().max_click_duration
    };
    ctx.options_mut(|o| o.input_options.max_click_duration = delay);
}

/// `AURORA_TOUCH`: 1 turns the touch controls on from the start, 0 keeps
/// them off whatever touches the window.
fn forced() -> Option<bool> {
    static FORCED: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| match std::env::var("AURORA_TOUCH").ok()?.trim() {
        "1" | "on" => Some(true),
        "0" | "off" => Some(false),
        other => {
            log::warn!("AURORA_TOUCH={other}: expected 1 or 0");
            None
        }
    })
}

/// What the viewer runs on: `AURORA_DEVICE` for a test, else the device
/// itself on iPadOS and iOS, a desktop elsewhere. Asked on the main thread
/// first (the start), as UIKit wants.
pub(super) fn device() -> touch::Device {
    static DEVICE: std::sync::OnceLock<touch::Device> = std::sync::OnceLock::new();
    *DEVICE.get_or_init(|| {
        if let Some(device) = std::env::var("AURORA_DEVICE").ok().and_then(|v| touch::Device::parse(&v)) {
            return device;
        }
        #[cfg(target_os = "ios")]
        return if crate::ios::phone() {
            touch::Device::Phone
        } else {
            touch::Device::Tablet
        };
        #[cfg(not(target_os = "ios"))]
        touch::Device::Desktop
    })
}

impl App {
    /// Whether fingers drive the world and the touch controls are shown.
    pub(super) fn touch_on(&self) -> bool {
        use crate::settings::TouchControls as T;
        match (forced(), self.settings.touch_controls) {
            (Some(on), _) => on,
            (None, T::On) => true,
            (None, T::Off) => false,
            // a tablet or a phone from the start; elsewhere from the first finger
            (None, T::Auto) => device().touch_first() || self.touch.seen,
        }
    }

    /// Whether a finger landing now could be taken by the touch layer at all.
    fn touch_listens(&self) -> bool {
        use crate::settings::TouchControls as T;
        forced().unwrap_or(self.settings.touch_controls != T::Off)
    }

    fn touch_now(&self) -> f64 {
        self.touch.epoch.elapsed().as_secs_f64()
    }

    /// A finger event of the main window, and who it is for.
    pub(super) fn touch_event(&mut self, id: touch::Id, phase: TouchPhase, x: f32, y: f32) -> Handoff {
        self.last_input = Instant::now();
        // with the touch controls off a finger that lands is the
        // interface's pointer; one that landed before keeps what it was given
        if phase == TouchPhase::Started && !self.touch_listens() {
            return Handoff::Interface;
        }
        if self.touch_routed(id, phase, x, y) {
            return Handoff::Kept;
        }
        self.touch_interface(id, phase, glam::Vec2::new(x, y))
    }

    /// The interface's finger: it stays the interface's, and is watched for
    /// the long press (`touch_raw_input`).
    fn touch_interface(&mut self, id: touch::Id, phase: TouchPhase, pos: glam::Vec2) -> Handoff {
        match phase {
            TouchPhase::Started => {
                let on = self.touch_on();
                let others = self.touch.owners.iter().any(|(i, o)| *i != id && *o == Owner::Interface);
                match &mut self.touch.iface {
                    // a second finger on the interface: a pinch of two
                    // hands, not a long press
                    Some(f) => f.moved = true,
                    None if on => {
                        self.touch.iface = Some(Iface {
                            id,
                            origin: pos,
                            pressed: None,
                            moved: others,
                            spent: false,
                        });
                    }
                    None => {}
                }
                Handoff::Interface
            }
            TouchPhase::Moved => {
                // (egui's own distance for a press that is still a click)
                let slop = self.egui_ctx.options(|o| o.input_options.max_click_dist) * self.egui_ctx.pixels_per_point();
                match &mut self.touch.iface {
                    Some(f) if f.id == id && f.spent => Handoff::Kept,
                    Some(f) if f.id == id => {
                        f.moved |= f.origin.distance(pos) > slop;
                        Handoff::Interface
                    }
                    _ => Handoff::Interface,
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => match self.touch.iface.take_if(|f| f.id == id) {
                Some(f) if f.spent => Handoff::InterfaceCancelled,
                _ => Handoff::Interface,
            },
        }
    }

    /// A finger event for the touch layer itself. True when it took it.
    fn touch_routed(&mut self, id: touch::Id, phase: TouchPhase, x: f32, y: f32) -> bool {
        let pos = glam::Vec2::new(x, y);
        let now = self.touch_now();
        let ppp = self.egui_ctx.pixels_per_point();
        match phase {
            TouchPhase::Started => {
                self.touch.seen = true;
                // an identifier the system gives again before its lift came:
                // what it was doing ends there
                self.touch.dropped.retain(|d| *d != id);
                if let Some(owner) = self.touch.forget(id) {
                    self.touch_lift(id, owner, now, true);
                }
                let owner = self.touch_owner(pos);
                self.touch.owners.push((id, owner));
                match owner {
                    Owner::Interface => return false,
                    Owner::Stick => {
                        let rest = ui::touch::stick_rest(ui::layout::work_area(&self.egui_ctx));
                        let rest = glam::Vec2::new(rest.x, rest.y) * ppp;
                        self.touch.stick = Some(if rest.distance(pos) <= self.touch_stick_radius() {
                            Stick::on_ring(id, pos, now)
                        } else {
                            Stick::new(id, pos, now)
                        });
                    }
                    Owner::Button(button) => self.touch_button(button, true),
                    Owner::World => {
                        let gesture = self.touch.world.start(id, pos, now);
                        self.touch_gesture(gesture);
                    }
                }
                true
            }
            TouchPhase::Moved => {
                if self.touch.dropped.contains(&id) {
                    return true;
                }
                match self.touch.owner(id) {
                    None | Some(Owner::Interface) => false,
                    Some(Owner::Stick) => {
                        if let Some(stick) = self.touch.stick.as_mut().filter(|s| s.id == id) {
                            stick.moved(pos, touch::SLOP * ppp);
                        }
                        true
                    }
                    Some(Owner::Button(_)) => true,
                    Some(Owner::World) => {
                        let gesture = self.touch.world.moved(id, pos, touch::SLOP * ppp);
                        self.touch_gesture(gesture);
                        true
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if let Some(index) = self.touch.dropped.iter().position(|d| *d == id) {
                    self.touch.dropped.swap_remove(index);
                    return true;
                }
                match self.touch.forget(id) {
                    None | Some(Owner::Interface) => false,
                    Some(owner) => {
                        self.touch_lift(id, owner, now, phase == TouchPhase::Cancelled);
                        true
                    }
                }
            }
        }
    }

    /// A finger of the touch layer lifts, or is taken away (`cancelled`).
    fn touch_lift(&mut self, id: touch::Id, owner: Owner, now: f64, cancelled: bool) {
        match owner {
            Owner::Interface => {}
            Owner::Stick => {
                // a thumb that never moved was a tap on the world under
                // the stick's zone
                if let Some(stick) = self.touch.stick.take_if(|s| s.id == id)
                    && !cancelled
                {
                    self.touch_gesture(stick.lifted(now));
                }
            }
            Owner::Button(button) => self.touch_button(button, false),
            Owner::World => {
                let gesture = self.touch.world.end(id, now, cancelled);
                self.touch_gesture(gesture);
                if self.touch.world.count() == 0 && self.touch.tap_release.is_none() {
                    self.touch_cursor_rest();
                }
            }
        }
    }

    /// The cursor once no finger is on the world: back where the mouse is,
    /// so that a click made without moving it lands there; nowhere on a
    /// screen that has none.
    fn touch_cursor_rest(&mut self) {
        self.cursor_pos = self.touch.mouse_at.unwrap_or(NOWHERE);
    }

    /// What a finger landing at `pos` (window pixels) is given to.
    fn touch_owner(&self, pos: glam::Vec2) -> Owner {
        let ppp = self.egui_ctx.pixels_per_point();
        let at = egui::pos2(pos.x / ppp, pos.y / ppp);
        touch::owner(touch::Landing {
            interface_only: !matches!(self.screen, Screen::World)
                || self.exit_prompt.open
                || self.context_menu.is_some()
                || egui::Popup::is_any_open(&self.egui_ctx)
                || self.ik_editor.manipulator_owns_pointer(),
            // (the buttons are in the background's order, under everything else)
            covered: self
                .egui_ctx
                .layer_id_at(at)
                .is_some_and(|layer| layer.order != egui::Order::Background),
            button: self
                .touch
                .buttons
                .iter()
                .find(|(_, rect)| rect.contains(at))
                .map(|(button, _)| *button),
            interface: ui::widgets::point_over_ui(&self.egui_ctx, at),
            stick: self.touch.shown
                && self.touch.stick.is_none()
                && self.touch_stick_allowed()
                && touch::inside(self.touch_stick_zone(), pos),
        })
    }

    /// The stick drives the avatar: not where something else has the view
    /// (the avatar studio, the build tools' own drags keep the whole world).
    fn touch_stick_allowed(&self) -> bool {
        !self.avatar_studio.active && !self.build.open && !self.photo_full_frame()
    }

    /// Where a thumb starts the stick, in window pixels.
    pub(super) fn touch_stick_zone(&self) -> [f32; 4] {
        let ppp = self.egui_ctx.pixels_per_point();
        let work = ui::layout::work_area(&self.egui_ctx);
        touch::stick_zone([work.left() * ppp, work.top() * ppp, work.right() * ppp, work.bottom() * ppp])
    }

    /// A touch button pressed or released by a finger.
    fn touch_button(&mut self, button: Button, pressed: bool) {
        if button.held() {
            self.touch.held.retain(|b| *b != button);
            if pressed {
                self.touch.held.push(button);
            }
            return;
        }
        // the others act when the finger lands, like a key
        if pressed {
            self.touch_button_act(button);
            crate::haptics::tap();
        }
    }

    /// What a touch button that is not held does.
    fn touch_button_act(&mut self, button: Button) {
        match button {
            Button::Fly => self.toggle_fly(),
            Button::ResetView => self.reset_camera_view(),
            Button::Mouselook => self.toggle_mouselook(),
            Button::Up | Button::Down => {}
        }
    }

    /// What the fingers on the world ask for, applied as the mouse's.
    fn touch_gesture(&mut self, gesture: Option<Gesture>) {
        let Some(gesture) = gesture else {
            return;
        };
        // (the world left under the fingers: only their orbit is ended)
        if !matches!(self.screen, Screen::World) && gesture != Gesture::DragEnd {
            return;
        }
        match gesture {
            Gesture::Tap(at) => {
                self.touch_release_tap();
                self.cursor_pos = (at.x, at.y);
                if release_world_keyboard(&self.egui_ctx, &mut self.chat_ui, MouseButton::Left) {
                    self.press_while_typing();
                }
                self.mouse_input(MouseButton::Left, true, false);
                self.touch.tap_release = Some(Instant::now() + TAP_HOLD);
            }
            Gesture::LongPress(at) => {
                self.touch_release_tap();
                self.cursor_pos = (at.x, at.y);
                release_world_keyboard(&self.egui_ctx, &mut self.chat_ui, MouseButton::Right);
                if self.camera.mouselook() {
                    return;
                }
                self.open_context_menu();
                crate::haptics::press();
            }
            Gesture::Drag(by) => {
                self.touch_release_tap();
                if !self.camera.mouselook() && self.mouse_mode == MouseMode::None && self.camera.free().is_none() {
                    // the orbit around our avatar that the right drag makes
                    self.mouse_mode = MouseMode::Orbit;
                    self.touch.orbiting = true;
                }
                // in the screen's points, as a mouse counts its motion: a
                // dense screen does not turn the camera twice as fast
                let by = by / self.gfx.as_ref().map_or(1.0, |g| g.window.scale_factor() as f32).max(0.25);
                let by = if self.settings.invert_mouse {
                    glam::Vec2::new(by.x, -by.y)
                } else {
                    by
                };
                let s = self.settings.mouse_sensitivity;
                match self.camera.free_mut() {
                    // the free camera looks around instead of orbiting
                    Some(free) => free.drag(by.x, by.y, s),
                    None => self.camera_mouse_delta(by.x, by.y),
                }
            }
            Gesture::DragEnd => self.touch_end_orbit(),
            Gesture::Pinch { zoom, pan, at } => {
                self.touch_release_tap();
                self.touch_end_orbit();
                let clicks = touch::pinch_clicks(zoom);
                if clicks != 0.0 {
                    // the wheel where the fingers are: the viewfinder, the
                    // avatar studio, a HUD and the build tools take it as
                    // they take the mouse's
                    self.cursor_pos = (at.x, at.y);
                    self.world_wheel(-clicks);
                }
                self.touch_pan(pan);
            }
            Gesture::TwoFingerTap => {
                // with the interface not drawn (Ctrl+Alt+F1) nothing on the
                // screen brings it back: this does, where there are no keys
                if self.interface.ui_off {
                    let ctx = self.egui_ctx.clone();
                    self.interface.set_ui_off(&ctx, false);
                } else {
                    self.reset_camera_view();
                }
                crate::haptics::tap();
            }
        }
    }

    fn touch_end_orbit(&mut self) {
        if std::mem::take(&mut self.touch.orbiting) && self.mouse_mode == MouseMode::Orbit {
            self.mouse_mode = MouseMode::None;
        }
    }

    /// Two fingers dragged together move the view sideways and up or down,
    /// by what the fingers cover at the distance of what the camera looks at.
    fn touch_pan(&mut self, pan: glam::Vec2) {
        if pan == glam::Vec2::ZERO || self.camera.mouselook() || self.camera.free().is_some() || self.avatar_studio.active {
            return;
        }
        let height = self.gfx.as_ref().map_or(900.0, |g| g.renderer.size().1 as f32).max(1.0);
        let distance = (self.camera.position - self.camera.target).length().max(0.5);
        let meters_per_pixel = 2.0 * distance * (self.camera.fov_y * 0.5).tan() / height;
        // the view follows the fingers: the world under them goes with them
        let meters = glam::Vec2::new(pan.x, pan.y) * meters_per_pixel;
        self.camera.gamepad_pan(meters, &mut self.world.agent, &self.settings.camera);
    }

    fn touch_release_tap(&mut self) {
        if self.touch.tap_release.take().is_some() {
            self.mouse_input(MouseButton::Left, false, false);
        }
    }

    /// Every frame, before the movement is read: the world's long press, a
    /// tap's release, a button the mouse clicked, and the fingers the world
    /// or the setting no longer takes.
    pub(super) fn touch_frame(&mut self) {
        if let Some(button) = self.touch.mouse_clicked.take() {
            self.touch_button_act(button);
        }
        if self.touch.tap_release.is_some_and(|at| Instant::now() >= at) {
            self.touch_release_tap();
            if self.touch.world.count() == 0 {
                self.touch_cursor_rest();
            }
        }
        if self.touch.busy() && (!matches!(self.screen, Screen::World) || !self.touch_listens()) {
            self.touch_drop();
            return;
        }
        let now = self.touch_now();
        let gesture = self.touch.world.tick(now);
        self.touch_gesture(gesture);
        // a thumb resting in the stick's zone, off the ring: a long press
        // on the world there (its finger stays the stick's, and does nothing more)
        if let Some(gesture) = self.touch.stick.and_then(|s| s.held(now)) {
            self.touch.stick = None;
            self.touch_gesture(Some(gesture));
        }
    }

    /// Every gesture is dropped: the window lost the focus, the world left
    /// or the touch controls were turned off. The fingers still down are
    /// nobody's until they lift.
    pub(super) fn touch_drop(&mut self) {
        for (id, owner) in std::mem::take(&mut self.touch.owners) {
            if owner == Owner::Interface {
                self.touch.owners.push((id, owner));
            } else if !self.touch.dropped.contains(&id) {
                self.touch.dropped.push(id);
            }
        }
        self.touch.stick = None;
        self.touch.held.clear();
        self.touch.mouse_held.clear();
        self.touch.mouse_clicked = None;
        let gesture = self.touch.world.clear();
        self.touch_gesture(gesture);
        self.touch_release_tap();
        self.touch_cursor_rest();
    }

    /// The interface's input of this frame: what the touch layer adds to it
    /// (the long press of its finger, `pending`), and the room the screen
    /// does not give: the insets of a test (`AURORA_SAFE_AREA`) and the
    /// on-screen keyboard, which the windows and the bars then stay clear of
    /// as they do of a notch.
    pub(super) fn touch_raw_input(&mut self, raw: &mut egui::RawInput) {
        // the controls are the pass's to draw: none where it does not (the
        // interface off, the Director's playback, a photo's full frame)
        self.touch.buttons.clear();
        self.touch.shown = false;
        self.touch.mouse_held.clear();

        click_delay(&self.egui_ctx, self.touch.iface.is_some());
        let now = raw.time.unwrap_or_else(|| self.touch_now());
        // (a press egui already drags: a slider, a scroll bar, a window)
        let dragging = self.egui_ctx.dragged_id().is_some();
        let ppp = self.egui_ctx.pixels_per_point();
        if let Some(f) = &mut self.touch.iface {
            let pressed = *f.pressed.get_or_insert(now);
            f.moved |= dragging;
            if touch::interface_long_press(pressed, now, f.moved, f.spent) {
                f.spent = true;
                self.touch
                    .pending
                    .extend(hold_events(egui::pos2(f.origin.x / ppp, f.origin.y / ppp)));
                crate::haptics::press();
            }
        }
        raw.events.append(&mut self.touch.pending);

        static TEST: std::sync::OnceLock<Option<[f32; 4]>> = std::sync::OnceLock::new();
        let test = *TEST.get_or_init(|| std::env::var("AURORA_SAFE_AREA").ok().and_then(|v| touch::safe_area(&v)));
        #[cfg(target_os = "ios")]
        let keyboard = crate::ios::keyboard_inset();
        #[cfg(not(target_os = "ios"))]
        let keyboard = 0.0_f32;
        if test.is_none() && keyboard <= 0.0 && raw.safe_area_insets.is_none() {
            return;
        }
        let mut insets = raw.safe_area_insets.unwrap_or_default().0;
        if let Some([top, right, bottom, left]) = test {
            insets.top += top;
            insets.right += right;
            insets.bottom += bottom;
            insets.left += left;
        }
        // (the keyboard lies over the home bar's inset, not above it)
        insets.bottom = insets.bottom.max(keyboard);
        raw.safe_area_insets = Some(egui::SafeAreaInsets(insets));
    }

    /// What the touch controls show in this interface pass (ui/touch.rs),
    /// or None where there are none: touch off, no world, a photo's frame.
    pub(super) fn touch_controls(&self) -> Option<ui::touch::State> {
        if !self.touch_on() || !matches!(self.screen, Screen::World) || self.photo_full_frame() {
            return None;
        }
        let ppp = self.egui_ctx.pixels_per_point();
        let point = |v: glam::Vec2| egui::pos2(v.x / ppp, v.y / ppp);
        Some(ui::touch::State {
            stick: self.touch.stick.map(|s| (point(s.origin), point(s.pos))),
            stick_allowed: self.touch_stick_allowed(),
            running: self.touch_runs(),
            flying: self.world.agent.flying,
            mouselook: self.camera.mouselook(),
            held: self.touch.held.clone(),
        })
    }

    /// The stick's radius in window pixels.
    fn touch_stick_radius(&self) -> f32 {
        touch::STICK_RADIUS * self.egui_ctx.pixels_per_point()
    }

    /// What the stick asks now.
    pub(super) fn touch_stick_move(&self) -> touch::StickMove {
        self.touch.stick.map_or_else(touch::StickMove::default, |stick| {
            touch::stick_move(stick.vector(self.touch_stick_radius()), self.camera.mouselook())
        })
    }

    /// The stick pushed to its rim: a run, for as long as it stays there.
    pub(super) fn touch_runs(&self) -> bool {
        self.touch_stick_move().run
    }

    /// The stick and the held buttons, added to the keys' movement.
    pub(super) fn touch_avatar(&self, i: &mut MoveInput) {
        let m = self.touch_stick_move();
        i.forward |= m.forward;
        i.back |= m.back;
        i.strafe_left |= m.strafe_left;
        i.strafe_right |= m.strafe_right;
        if m.turn != 0.0 {
            i.joystick_yaw += m.turn;
        }
        i.up |= self.touch.held(Button::Up);
        i.down |= self.touch.held(Button::Down);
    }

    /// The stick or a held button moves the avatar now.
    pub(super) fn touch_moves(&self) -> bool {
        self.touch_stick_move().any() || self.touch.held(Button::Up) || self.touch.held(Button::Down)
    }
}

pub(super) mod demo {
    //! `AURORA_DEMO_TOUCH`: scripted fingers (src/touch/demo.rs), fed to
    //! `touch_event` as the window's are.
    use super::*;
    use crate::touch::demo::{Step, steps};

    #[derive(Default)]
    pub struct Demo {
        /// Where each scripted finger is, to report its lift where it was.
        fingers: Vec<(touch::Id, f32, f32)>,
    }

    impl App {
        /// A scripted finger, as the window's: one the interface gets is
        /// played to it as the pointer egui-winit makes of a finger.
        fn demo_finger(&mut self, id: touch::Id, phase: TouchPhase, x: f32, y: f32) {
            let handoff = self.touch_event(id, phase, x, y);
            if handoff == Handoff::Kept {
                return;
            }
            let ppp = self.egui_ctx.pixels_per_point();
            let pos = egui::pos2(x / ppp, y / ppp);
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let events = &mut self.touch.pending;
            match (phase, handoff) {
                (TouchPhase::Started, _) => events.extend([egui::Event::PointerMoved(pos), button(true)]),
                (TouchPhase::Moved, _) => events.push(egui::Event::PointerMoved(pos)),
                (TouchPhase::Ended, Handoff::Interface) => events.extend([button(false), egui::Event::PointerGone]),
                _ => events.push(egui::Event::PointerGone),
            }
        }

        pub(in crate::app) fn demo_touch_steps(&mut self, spec: &str) {
            let size = self.gfx.as_ref().map_or((1280.0, 720.0), |g| {
                let (w, h) = g.renderer.size();
                (w as f32, h as f32)
            });
            let frame = self.frame_count;
            let ppp = self.egui_ctx.pixels_per_point();
            let rest = ui::touch::stick_rest(ui::layout::work_area(&self.egui_ctx));
            for step in steps(spec, frame, size, (rest.x * ppp, rest.y * ppp), self.touch_stick_radius()) {
                // a button is pressed at its middle, wherever it is drawn
                let step = match step {
                    Step::Press(id, button) => match self.touch.buttons.iter().find(|(b, _)| *b == button) {
                        Some((_, rect)) => Step::Down(id, rect.center().x * ppp, rect.center().y * ppp),
                        None => {
                            log::warn!("touch demo frame {frame}: the {button:?} button is not shown");
                            continue;
                        }
                    },
                    other => other,
                };
                match step {
                    Step::Press(..) => {}
                    Step::Down(id, x, y) => {
                        self.touch.demo.fingers.retain(|f| f.0 != id);
                        self.touch.demo.fingers.push((id, x, y));
                        self.demo_finger(id, TouchPhase::Started, x, y);
                    }
                    Step::Move(id, x, y) => {
                        if let Some(f) = self.touch.demo.fingers.iter_mut().find(|f| f.0 == id) {
                            (f.1, f.2) = (x, y);
                        }
                        self.demo_finger(id, TouchPhase::Moved, x, y);
                    }
                    Step::Up(id) => {
                        let at = self.touch.demo.fingers.iter().find(|f| f.0 == id).copied();
                        self.touch.demo.fingers.retain(|f| f.0 != id);
                        if let Some((_, x, y)) = at {
                            self.demo_finger(id, TouchPhase::Ended, x, y);
                        }
                    }
                    Step::Log => {
                        let m = self.touch_stick_move();
                        log::info!(
                            "touch demo frame {frame}: camera {:.2?} target {:.2?} agent {:.2?} yaw {:.3} mouselook {} flying {} menu {} stick fwd {} turn {:.2} run {}",
                            self.camera.position,
                            self.camera.target,
                            self.world.agent.position,
                            self.world.agent.yaw,
                            self.camera.mouselook(),
                            self.world.agent.flying,
                            self.context_menu.is_some(),
                            m.forward,
                            m.turn,
                            m.run,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::Frames;

    struct Row {
        frames: Frames,
        clicks: usize,
        menus: usize,
    }

    const AT: egui::Pos2 = egui::pos2(100.0, 100.0);

    impl Row {
        fn new() -> Self {
            Self {
                frames: Frames::new(),
                clicks: 0,
                menus: 0,
            }
        }

        /// A frame (a sixtieth of a second) of a row that clicks and has a menu.
        fn frame(&mut self, events: Vec<egui::Event>) {
            let (clicks, menus) = (&mut self.clicks, &mut self.menus);
            self.frames.run(events, |ui| {
                let rect = egui::Rect::from_center_size(AT, egui::vec2(120.0, 30.0));
                let row = ui.interact(rect, ui.scope_id().with("row"), egui::Sense::click());
                *clicks += usize::from(row.clicked());
                *menus += usize::from(row.secondary_clicked());
            });
        }

        fn wait(&mut self, frames: usize) {
            for _ in 0..frames {
                self.frame(Vec::new());
            }
        }

        fn press(&mut self) {
            self.frame(vec![
                egui::Event::PointerMoved(AT),
                egui::Event::PointerButton {
                    pos: AT,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }

        fn down(&self) -> bool {
            self.frames.ctx.input(|i| i.pointer.any_down())
        }
    }

    #[test]
    fn a_finger_held_on_the_interface_right_clicks_once_and_leaves_no_button_down() {
        let mut row = Row::new();
        click_delay(&row.frames.ctx, true);
        row.press();
        // 31 frames: just past the long press's half second
        row.wait(30);
        assert!(row.down());
        assert_eq!((row.clicks, row.menus), (0, 0));
        row.frame(hold_events(AT).to_vec());
        assert_eq!((row.clicks, row.menus), (0, 1), "the menu, and no click");
        assert!(!row.down(), "a button left down takes the hover from every widget");
        // the finger stays, then lifts (the interface is told it was cancelled)
        row.wait(40);
        row.frame(vec![egui::Event::PointerGone]);
        assert_eq!((row.clicks, row.menus), (0, 1), "once per press, and the lift clicks nothing");
        // the next tap is a plain click
        click_delay(&row.frames.ctx, false);
        row.press();
        row.wait(3);
        row.frame(vec![egui::Event::PointerButton {
            pos: AT,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!((row.clicks, row.menus), (1, 1));
        assert!(!row.down());
    }

    #[test]
    fn the_finger_let_go_would_click_at_the_mouses_delay() {
        // why the delay is the long press's while a finger is the pointer:
        // at egui's own (0.8 s) the release half a second in is a click
        let mut row = Row::new();
        click_delay(&row.frames.ctx, false);
        row.press();
        row.wait(30);
        row.frame(hold_events(AT).to_vec());
        assert_eq!(row.clicks, 1);
    }
}
