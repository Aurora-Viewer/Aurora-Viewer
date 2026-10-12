//! Clicks on the world that do not wait for the GPU.
//!
//! A click needs the depth under its pixel: the opaque surface the cursor
//! is on (terrain, prims, avatars and what they wear as the GPU poses
//! them), which only the depth of the frame on screen gives. Reading it at
//! once made the main thread wait for the frame in flight on the render
//! thread, then for everything the GPU had in its queue: 16 to 20 ms
//! without a frame at each click in a region that keeps the GPU busy.
//!
//! The click now asks for that depth and returns
//! (`Renderer::request_pick`): the request leaves with the next frame, the
//! render thread copies the texel of the frame the click was made on, and
//! the answer comes back with a frame result: one or two frames later
//! under a frame cap, three when the GPU is the limit (about the time the
//! click used to wait).
//! The click is then run again from its start with the answer in hand, the
//! cursor and the modifiers as they were when it was made: same ray, same
//! depth, same code as before, a few frames later.
//!
//! As LLViewerWindow::pickAsync (LLToolCamera, the build tools) does with
//! LLWindow::delayInputProcessing ("delay further event processing until
//! we receive results of pick [...] so that handleMouseUp won't be called
//! until the pick triggered in handleMouseDown has been processed",
//! indra/newview/llviewerwindow.cpp, originally LGPL 2.1), what follows a
//! click that waits is held behind it, in order: mouse buttons (the release
//! of that very click first of all) and the mouse motion that drags the
//! camera. Nothing is lost or reordered; the cursor position itself is
//! never held.
//!
//! `AURORA_CLICK_PICK=wait` reads the depth as before, waiting;
//! `AURORA_CLICK_PICK=check` does both and logs every click (`click
//! check`): whether the two depths are the same, and whether the click
//! lands on the same point and object. `AURORA_DEMO_CLICKS` clicks at a
//! steady rate, to measure a click's cost in a loaded scene.

use super::*;
use std::collections::VecDeque;

/// A click that needs the depth under the cursor: what to run again when
/// the answer is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Click {
    /// Left press on the world: the build tools first, the click itself if
    /// they do not take it.
    Press,
    /// The build tools alone (demo scripts).
    Build,
    /// The click itself (demo scripts): LLToolPie's left press.
    World,
    /// Right click: the menu of what is under the cursor.
    Menu,
}

/// What follows a click that waits for its depth, kept in order.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Held {
    Button {
        button: MouseButton,
        pressed: bool,
        over_ui: bool,
    },
    /// Mouse motion (camera drags), summed while nothing comes between.
    Motion(f32, f32),
    Click(Click),
    LeftRelease,
    /// The build tools' own release (demo script).
    BuildRelease,
    /// A demo script's log line about the click before it.
    Note(DemoNote),
}

/// Log lines the demo scripts write right after a click, once it is done.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum DemoNote {
    /// AURORA_DEMO_CAMERA: the camera at this frame of the script.
    Camera(u64),
    /// AURORA_DEMO_ACTIONS: the result of the mode's click.
    Action(String),
    /// AURORA_DEMO_MEDIA_CLICK: the media focus after the click made there
    /// at this frame.
    Media((f32, f32), u64),
    /// AURORA_DEMO_MMO: the mouse mode after the press on the avatar.
    Mmo,
}

/// The cursor ray and the depth under the cursor of a click, both of the
/// frame the click was made on.
#[derive(Clone, Copy, Debug)]
pub(super) struct Picked {
    pub ray: Option<(Vec3, Vec3)>,
    pub depth: Option<Vec3>,
}

/// A click whose depth is asked.
struct Pending {
    ticket: u64,
    click: Click,
    cursor: (f32, f32),
    /// Alt, Ctrl, Shift.
    mods: [bool; 3],
    ray: Option<(Vec3, Vec3)>,
    asked: Instant,
    frame: u64,
    /// AURORA_CLICK_PICK=check: the waiting read made at the click.
    check: Option<Check>,
}

/// What the read that waits gave at the time of the click.
struct Check {
    depth: Option<Vec3>,
    target: Target,
    waited: Duration,
}

/// The point and the object (local id) a click lands on.
type Target = (Option<Vec3>, Option<u32>);

/// An answer that never comes (frames no longer drawn) drops its click.
const GIVE_UP: Duration = Duration::from_secs(2);

/// AURORA_CLICK_PICK: how a click reads the depth under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickMode {
    /// Asked with the next frame, answered a few frames later.
    Deferred,
    /// `wait`: read at once, waiting for the GPU, as before.
    Wait,
    /// `check`: both, compared in the log.
    Check,
}

fn pick_mode_of(var: Option<&str>) -> PickMode {
    match var.map(str::trim) {
        Some("wait") => PickMode::Wait,
        Some("check") => PickMode::Check,
        _ => PickMode::Deferred,
    }
}

fn pick_mode() -> PickMode {
    static MODE: std::sync::OnceLock<PickMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let mode = pick_mode_of(std::env::var("AURORA_CLICK_PICK").ok().as_deref());
        if mode != PickMode::Deferred {
            log::info!("click pick: {mode:?} (AURORA_CLICK_PICK)");
        }
        mode
    })
}

/// The clicks that wait for their depth and what is held behind them.
#[derive(Default)]
pub(super) struct ClickWait {
    pending: Option<Pending>,
    /// The click being run, and its answer when it is run again.
    running: Option<Click>,
    in_hand: Option<Picked>,
    held: VecDeque<Held>,
    demo: DemoClicks,
}

impl ClickWait {
    /// A click waits for its depth: what comes now goes behind it.
    pub(super) fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// AURORA_DEMO_CLICKS is making the clicks.
    pub(super) fn scripted(&self) -> bool {
        self.demo.spec().is_some()
    }

    pub(super) fn hold(&mut self, input: Held) {
        if let (Held::Motion(dx, dy), Some(Held::Motion(x, y))) = (&input, self.held.back_mut()) {
            *x += dx;
            *y += dy;
        } else {
            self.held.push_back(input);
        }
    }
}

impl App {
    /// A mouse button of the window, pressed or released: handled now, or
    /// in its turn behind the click that waits for its depth.
    pub(super) fn mouse_input(&mut self, button: MouseButton, pressed: bool, over_ui: bool) {
        if self.clicks.waiting() {
            self.clicks.hold(Held::Button { button, pressed, over_ui });
        } else {
            self.mouse_button(button, pressed, over_ui);
        }
    }

    fn mouse_button(&mut self, button: MouseButton, pressed: bool, over_ui: bool) {
        match button {
            MouseButton::Right => self.on_right_button(pressed, over_ui),
            MouseButton::Left if pressed => {
                if !over_ui && self.in_world() && !self.camera.mouselook() {
                    self.click(Click::Press);
                }
            }
            MouseButton::Left => self.on_left_release(),
            other => {
                if let Some(input) = Input::mouse(other) {
                    // bindable buttons (push-to-talk on the wheel click...)
                    if !pressed || !over_ui {
                        self.on_input(input, pressed);
                    }
                }
            }
        }
    }

    /// A click from the mouse or from a demo script. Behind a click that
    /// still waits for its depth, it waits in turn.
    pub(super) fn click(&mut self, click: Click) {
        if self.clicks.waiting() {
            self.clicks.hold(Held::Click(click));
        } else {
            self.run_click(click);
        }
    }

    /// Run a click from its start. When it reaches the depth under the
    /// cursor without having it (`click_pick`), it asks for it and returns:
    /// `resolve_clicks` runs it again with the answer.
    fn run_click(&mut self, click: Click) {
        self.clicks.running = Some(click);
        match click {
            Click::Press => {
                if !self.build_mouse_down() && !self.clicks.waiting() {
                    self.world_press();
                }
            }
            Click::Build => {
                self.build_mouse_down();
            }
            Click::World => self.world_press(),
            Click::Menu => self.world_menu(),
        }
        self.clicks.running = None;
    }

    /// The cursor ray and the depth under the cursor for the click being
    /// run. None: just asked from the renderer; the caller returns, and its
    /// click is run again when the answer is there.
    pub(super) fn click_pick(&mut self) -> Option<Picked> {
        if let Some(picked) = self.clicks.in_hand {
            return Some(picked);
        }
        let (x, y) = self.cursor_pos;
        let g = self.gfx.as_mut()?;
        let ray = g.renderer.cursor_ray(x, y);
        let mode = pick_mode();
        let Some(click) = self.clicks.running.filter(|_| mode != PickMode::Wait) else {
            let t = Instant::now();
            let depth = g.renderer.pick_world(x, y);
            self.note_pick(format_args!("the read waited {:.2} ms", t.elapsed().as_secs_f32() * 1000.0));
            return Some(Picked { ray, depth });
        };
        let check = (mode == PickMode::Check).then(|| {
            let t = Instant::now();
            let depth = g.renderer.pick_world(x, y);
            (depth, t.elapsed())
        });
        let ticket = g.renderer.request_pick(x, y);
        let check = check.map(|(depth, waited)| Check {
            depth,
            target: self.click_target(click, ray, depth),
            waited,
        });
        self.clicks.pending = Some(Pending {
            ticket,
            click,
            cursor: (x, y),
            mods: [self.alt, self.ctrl, self.shift],
            ray,
            asked: Instant::now(),
            frame: self.frame_count,
            check,
        });
        None
    }

    /// The click that waited is run again when its depth has come back,
    /// then what was held behind it, until one of those waits in turn.
    /// Once a frame, with the renderer in `self.gfx`.
    pub(super) fn resolve_clicks(&mut self) {
        while let Some(p) = &self.clicks.pending {
            let Some(g) = self.gfx.as_mut() else {
                return;
            };
            let depth = match g.renderer.take_pick(p.ticket) {
                Some(depth) => Some(depth),
                None if p.asked.elapsed() < GIVE_UP => return,
                None => {
                    log::warn!(
                        "click pick: no answer after {:.1} s, click dropped",
                        p.asked.elapsed().as_secs_f32()
                    );
                    g.renderer.forget_pick(p.ticket);
                    None
                }
            };
            let Some(p) = self.clicks.pending.take() else {
                return;
            };
            if let Some(depth) = depth
                && self.in_world()
            {
                let frames = self.frame_count - p.frame;
                let ms = p.asked.elapsed().as_secs_f32() * 1000.0;
                // the click as it was made: its cursor, its modifiers
                let now = (self.cursor_pos, [self.alt, self.ctrl, self.shift]);
                self.cursor_pos = p.cursor;
                [self.alt, self.ctrl, self.shift] = p.mods;
                match &p.check {
                    Some(c) => self.log_click_check(&p, c, depth, frames, ms),
                    None => self.note_pick(format_args!("answered after {frames} frames, {ms:.2} ms")),
                }
                self.clicks.in_hand = Some(Picked { ray: p.ray, depth });
                if p.click != Click::Press || !self.camera.mouselook() {
                    self.run_click(p.click);
                }
                self.clicks.in_hand = None;
                (self.cursor_pos, [self.alt, self.ctrl, self.shift]) = now;
            }
            while !self.clicks.waiting() {
                let Some(input) = self.clicks.held.pop_front() else {
                    break;
                };
                self.replay(input);
            }
        }
    }

    fn replay(&mut self, input: Held) {
        match input {
            Held::Button { button, pressed, over_ui } => self.mouse_button(button, pressed, over_ui),
            Held::Motion(dx, dy) => self.camera_mouse_delta(dx, dy),
            Held::Click(click) => self.run_click(click),
            Held::LeftRelease => self.on_left_release(),
            Held::BuildRelease => self.build_release(),
            Held::Note(note) => self.demo_note(note),
        }
    }

    /// The clicks that wait and what is held behind them are given up: the
    /// window lost the focus, the viewer leaves, a teleport starts.
    pub(super) fn drop_clicks(&mut self) {
        if let Some(p) = self.clicks.pending.take()
            && let Some(g) = self.gfx.as_mut()
        {
            g.renderer.forget_pick(p.ticket);
        }
        self.clicks.held.clear();
    }

    /// The build tools' release of a demo script, behind its press.
    pub(super) fn build_release(&mut self) {
        if self.clicks.waiting() {
            self.clicks.hold(Held::BuildRelease);
            return;
        }
        self.build.mouse_up(&mut self.world, &self.settings.build);
        self.flush_build();
    }

    /// A demo script's log line about the click it just made: written once
    /// that click is done.
    pub(super) fn demo_note(&mut self, note: DemoNote) {
        if self.clicks.waiting() {
            self.clicks.hold(Held::Note(note));
            return;
        }
        match note {
            DemoNote::Camera(frame) => log::info!(
                "demo camera f{frame}: {} (mouse {:?}, agent yaw {:.2})",
                self.camera.describe(),
                self.mouse_mode,
                self.world.agent.yaw
            ),
            DemoNote::Action(mode) => log::info!(
                "demo action click: {mode}; dialog={}, seated={}, contents={}, parcel_playing={}, parcel_paused={}, selected={:?}, focus={:?}, media_url={}",
                self.interactions.dialog.is_some(),
                self.world.agent.is_sitting(),
                self.interactions
                    .contents
                    .as_ref()
                    .and_then(|c| c.result.as_ref())
                    .and_then(|r| r.as_ref().ok())
                    .map_or(0, Vec::len),
                self.media.parcel_playing(),
                self.media.find(&crate::media::MediaKey::Parcel).is_some_and(|m| m.paused),
                self.build.selection,
                self.camera.focus_point(),
                self.interactions.media_url.is_some(),
            ),
            DemoNote::Media(pos, frame) => log::info!("demo media click at {pos:?} (frame {frame}): focus {:?}", self.media.focus),
            DemoNote::Mmo => log::info!("demo mmo: mode {:?}, pos {:?}", self.mouse_mode, self.world.agent.position),
        }
    }

    /// One line per click in a profiled or scripted session (the profile's
    /// `between` step holds what a click costs the main thread).
    fn note_pick(&self, what: std::fmt::Arguments) {
        if self.frame_profile.enabled() || self.clicks.scripted() {
            log::info!("click pick: {what}");
        } else {
            log::debug!("click pick: {what}");
        }
    }

    /// The point and the object a click with this ray and depth lands on:
    /// the first search of each kind of click (the build tools' pick, the
    /// action point then the surface under it), for AURORA_CLICK_PICK=check.
    fn click_target(&self, click: Click, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>) -> Target {
        let now = Instant::now();
        let build = self.build.open;
        let tools = build && !self.alt && matches!(click, Click::Press | Click::Build);
        let (point, idx) = if tools {
            (depth, depth.and_then(|p| self.scene.pick_at(&self.world, p, now)))
        } else {
            let point = self.scene.action_point(&self.world, ray, depth, build, self.settings.draw_distance);
            let idx = if click == Click::Menu || self.alt {
                point.and_then(|p| self.scene.interaction_at(&self.world, p, now, build))
            } else {
                point
                    .zip(ray)
                    .and_then(|(p, r)| self.scene.interaction_at_ray(&self.world, p, r, true))
            };
            (point, idx)
        };
        (point, idx.and_then(|i| self.world.objects.get(i)).map(|o| o.key.local_id))
    }

    /// AURORA_CLICK_PICK=check: the answer against the read that waited.
    /// The two depths are the same texel of the same frame: any difference
    /// is a defect. The click itself is run a few frames later than before:
    /// its target differs only if what is under the cursor changed meanwhile.
    fn log_click_check(&self, p: &Pending, c: &Check, depth: Option<Vec3>, frames: u64, ms: f32) {
        let target = self.click_target(p.click, p.ray, depth);
        let same_depth = depth == c.depth;
        let same_target = target == c.target;
        let line = format!(
            "click check: {:?} at ({:.0}, {:.0}): depth {}, target {} ({:?}, object {:?}); the read that waits took {:.2} ms, the answer came after {frames} frames, {ms:.2} ms",
            p.click,
            p.cursor.0,
            p.cursor.1,
            if same_depth { "identical" } else { "DIFFERENT" },
            if same_target { "identical" } else { "DIFFERENT" },
            target.0,
            target.1,
            c.waited.as_secs_f32() * 1000.0,
        );
        if same_depth && same_target {
            log::info!("{line}");
        } else {
            log::warn!(
                "{line}; waiting read: depth {:?}, target {:?}, object {:?}; answer: depth {depth:?}",
                c.depth,
                c.target.0,
                c.target.1
            );
        }
    }

    /// AURORA_DEMO_CLICKS: the scripted clicks of this frame.
    pub(super) fn demo_click_steps(&mut self) {
        let Some(spec) = self.clicks.demo.spec() else {
            return;
        };
        if self.frame_count < 600 || !matches!(self.screen, Screen::World) || self.tp_overlay.is_some() {
            return;
        }
        // the same way in as the buttons of the window
        let button = if spec.kind == DemoKind::Menu {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        let now = Instant::now();
        if self.clicks.demo.up.is_some_and(|at| now >= at) {
            self.clicks.demo.up = None;
            self.mouse_input(button, false, false);
            self.alt = false;
        }
        if self.clicks.demo.next.is_some_and(|at| now < at) || self.clicks.demo.up.is_some() {
            return;
        }
        // the click before must be over, its release included
        if self.clicks.waiting() || self.left_down || self.right_drag || self.mouse_mode != MouseMode::None {
            log::warn!(
                "demo clicks: the click before is not over (waiting {}, left button {}, right drag {}, mouse {:?})",
                self.clicks.waiting(),
                self.left_down,
                self.right_drag,
                self.mouse_mode
            );
        }
        // (a late click does not bring a burst of them)
        let due = self.clicks.demo.next.filter(|at| *at + spec.period > now).unwrap_or(now);
        self.clicks.demo.next = Some(due + spec.period);
        let size = self.gfx.as_ref().map_or((1280, 720), |g| g.renderer.size());
        let (w, h) = (size.0 as f32, size.1 as f32);
        self.cursor_pos = match spec.at {
            DemoAt::Pixel(x, y) => (x, y),
            DemoAt::Default => (w * 0.25, h * 0.85),
            DemoAt::Sweep => {
                self.clicks.demo.count += 1;
                let (u, v) = sweep_point(self.clicks.demo.count);
                ((w * (0.04 + 0.92 * u)).floor(), (h * (0.1 + 0.85 * v)).floor())
            }
        };
        match spec.kind {
            DemoKind::Left => {}
            DemoKind::Alt => self.alt = true,
            // (the menu of the click before)
            DemoKind::Menu => self.context_menu = None,
            DemoKind::Build | DemoKind::Focus => {
                let tool = if spec.kind == DemoKind::Focus {
                    crate::build::Tool::Focus
                } else {
                    crate::build::Tool::Edit
                };
                if !self.build.open || self.build.tool != tool {
                    self.build.open_build(tool);
                }
            }
        }
        self.mouse_input(button, true, false);
        // (a short human click)
        self.clicks.demo.up = Some(now + Duration::from_millis(60).min(spec.period / 2));
    }
}

/// AURORA_DEMO_CLICKS="<ms>[,x,y|sweep][,left|alt|menu|build|focus]": a
/// click on the world every `ms` milliseconds (250 if it is not a number)
/// once the world shows, held 60 ms: at pixel (x, y) of the window (a
/// quarter of the width, near the bottom, by default), or with `sweep` at a
/// new place of the window each time.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DemoSpec {
    period: Duration,
    at: DemoAt,
    kind: DemoKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DemoAt {
    Default,
    Pixel(f32, f32),
    Sweep,
}

/// Place of the `n`th click of a sweep, in [0, 1)²: a low-discrepancy
/// sequence (golden-ratio steps), so a few hundred clicks cover the window.
fn sweep_point(n: u32) -> (f32, f32) {
    let n = n as f32;
    ((n * 0.754_877_7).fract(), (n * 0.569_840_3).fract())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DemoKind {
    Left,
    Alt,
    Menu,
    Build,
    Focus,
}

fn demo_spec(var: &str) -> DemoSpec {
    let parts: Vec<&str> = var.split(',').map(str::trim).collect();
    let numbers: Vec<f32> = parts.iter().skip(1).filter_map(|p| p.parse().ok()).collect();
    let kind = match parts.last().copied() {
        Some("alt") => DemoKind::Alt,
        Some("menu") => DemoKind::Menu,
        Some("build") => DemoKind::Build,
        Some("focus") => DemoKind::Focus,
        _ => DemoKind::Left,
    };
    let at = match numbers[..] {
        _ if parts.contains(&"sweep") => DemoAt::Sweep,
        [x, y, ..] => DemoAt::Pixel(x, y),
        _ => DemoAt::Default,
    };
    let ms = parts.first().and_then(|p| p.parse::<u64>().ok()).unwrap_or(250);
    DemoSpec {
        period: Duration::from_millis(ms.clamp(20, 60_000)),
        at,
        kind,
    }
}

#[derive(Default)]
struct DemoClicks {
    /// When the next click is due, and when the button of the last goes up.
    next: Option<Instant>,
    up: Option<Instant>,
    /// Clicks of a sweep so far.
    count: u32,
}

impl DemoClicks {
    fn spec(&self) -> Option<DemoSpec> {
        static SPEC: std::sync::OnceLock<Option<DemoSpec>> = std::sync::OnceLock::new();
        *SPEC.get_or_init(|| std::env::var("AURORA_DEMO_CLICKS").ok().map(|v| demo_spec(&v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_waiting_read_is_only_used_when_asked_for() {
        assert_eq!(pick_mode_of(None), PickMode::Deferred);
        assert_eq!(pick_mode_of(Some("1")), PickMode::Deferred);
        assert_eq!(pick_mode_of(Some(" wait ")), PickMode::Wait);
        assert_eq!(pick_mode_of(Some("check")), PickMode::Check);
    }

    #[test]
    fn what_follows_a_waiting_click_keeps_its_order() {
        let mut w = ClickWait::default();
        assert!(!w.waiting());
        w.hold(Held::Motion(1.0, 2.0));
        w.hold(Held::Motion(3.0, -1.0));
        w.hold(Held::LeftRelease);
        w.hold(Held::Motion(1.0, 1.0));
        w.hold(Held::Click(Click::Menu));
        // motion is summed only while nothing comes between: a drag held
        // before the release must not move the camera after it
        assert_eq!(
            w.held,
            [
                Held::Motion(4.0, 1.0),
                Held::LeftRelease,
                Held::Motion(1.0, 1.0),
                Held::Click(Click::Menu)
            ]
        );
    }

    #[test]
    fn demo_clicks_read_their_period_pixel_and_kind() {
        let d = demo_spec("1");
        assert_eq!(
            (d.period, d.at, d.kind),
            (Duration::from_millis(20), DemoAt::Default, DemoKind::Left)
        );
        let d = demo_spec("yes");
        assert_eq!(
            (d.period, d.at, d.kind),
            (Duration::from_millis(250), DemoAt::Default, DemoKind::Left)
        );
        let d = demo_spec("500, 320, 600");
        assert_eq!(
            (d.period, d.at, d.kind),
            (Duration::from_millis(500), DemoAt::Pixel(320.0, 600.0), DemoKind::Left)
        );
        let d = demo_spec("200,alt");
        assert_eq!(
            (d.period, d.at, d.kind),
            (Duration::from_millis(200), DemoAt::Default, DemoKind::Alt)
        );
        let d = demo_spec("300,640,360,menu");
        assert_eq!(
            (d.period, d.at, d.kind),
            (Duration::from_millis(300), DemoAt::Pixel(640.0, 360.0), DemoKind::Menu)
        );
        let d = demo_spec("100,sweep,build");
        assert_eq!(
            (d.period, d.at, d.kind),
            (Duration::from_millis(100), DemoAt::Sweep, DemoKind::Build)
        );
        assert_eq!(demo_spec("250,focus").kind, DemoKind::Focus);
        // a sweep stays in the window and does not repeat itself soon
        let places: Vec<(f32, f32)> = (1..200).map(sweep_point).collect();
        assert!(places.iter().all(|(u, v)| (0.0..1.0).contains(u) && (0.0..1.0).contains(v)));
        let cells: std::collections::HashSet<(u32, u32)> = places.iter().map(|(u, v)| ((u * 8.0) as u32, (v * 8.0) as u32)).collect();
        assert!(cells.len() > 56, "{} of 64 cells", cells.len());
    }
}
