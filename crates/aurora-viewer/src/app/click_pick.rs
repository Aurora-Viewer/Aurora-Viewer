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
//! With the depth, a click asks what the next frame draws under its pixel
//! (`Renderer::request_drawn`): the object of a rigged mesh, which only the
//! GPU poses, is designated from that answer (scene/drawn.rs). Camera picks
//! reject transparent texture holes unless the transparency overlay shows
//! their surface; interactions retain invisible Blend surfaces. The answer
//! comes a frame after the depth.
//!
//! `AURORA_CLICK_PICK=wait` reads the depth as before, waiting;
//! `AURORA_CLICK_PICK=check` does both and logs every click (`click
//! check`): whether the two depths are the same, and whether the click
//! lands on the same point and object. `AURORA_DEMO_CLICKS` clicks at a
//! steady rate, to measure a click's cost in a loaded scene.

use super::*;
use std::collections::VecDeque;
use uuid::Uuid;

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
    /// Left press in mouselook that no script took: touch at the crosshair,
    /// or under the cursor Alt freed (app/script_clicks.rs).
    Gun { at_cursor: bool },
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
#[derive(Clone, Debug)]
pub(super) struct Picked {
    pub ray: Option<(Vec3, Vec3)>,
    pub depth: Option<Vec3>,
    /// The faces drawn under the pixel, in the frame after the click:
    /// whose surface it is when the GPU alone poses it (scene/drawn.rs).
    /// Camera clicks use the visible-surface mode of that same pass.
    pub drawn: aurora_render::Drawn,
    /// A displayed name tag captured at the original press, before waiting
    /// for the depth. Camera/avatar motion cannot move its hit rectangle.
    pub tag: Option<(Uuid, Vec3)>,
    pub particles: Option<std::sync::Arc<[crate::scene::particles::ParticleHit]>>,
}

impl Picked {
    pub(super) fn visible_point(&self) -> Option<Vec3> {
        match self.drawn.blend.or(self.drawn.opaque) {
            Some(face) => Some(face.point),
            // The old diagnostic read has no drawn-face pass. Otherwise an
            // empty answer means sky, not an older depth's phantom surface.
            None if pick_mode() == PickMode::Wait => self.depth,
            None => None,
        }
    }
}

/// A click whose depth is asked.
struct Pending {
    ticket: u64,
    /// The request for the drawn faces, when the click asks it, and the
    /// two answers as they come.
    drawn: Option<u64>,
    depth_answer: Option<Option<Vec3>>,
    drawn_answer: Option<aurora_render::Drawn>,
    click: Click,
    cursor: (f32, f32),
    /// Alt, Ctrl, Shift.
    mods: [bool; 3],
    ray: Option<(Vec3, Vec3)>,
    particles: Option<std::sync::Arc<[crate::scene::particles::ParticleHit]>>,
    tags: Vec<(Uuid, Vec3)>,
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

    /// The click is run again, its depth in hand.
    pub(super) fn replaying(&self) -> bool {
        self.in_hand.is_some()
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
    pub(super) fn ui_takes_pointer(&self) -> bool {
        if self.cursor_hold.held() {
            // the hidden cursor is the camera's: the crosshair, never a window (#1343)
            return false;
        }
        let ppp = self.egui_ctx.pixels_per_point();
        ui::widgets::pointer_over_ui(&self.egui_ctx, egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp))
    }

    /// A mouse button of the window, pressed or released: handled now, or
    /// in its turn behind the click that waits for its depth.
    pub(super) fn mouse_input(&mut self, button: MouseButton, pressed: bool, over_ui: bool) {
        if self.clicks.waiting() {
            self.clicks.hold(Held::Button { button, pressed, over_ui });
        } else {
            self.with_mouse_mods(|app| app.mouse_button(button, pressed, over_ui));
        }
    }

    fn mouse_button(&mut self, button: MouseButton, pressed: bool, over_ui: bool) {
        match button {
            MouseButton::Right => self.on_right_button(pressed, over_ui),
            MouseButton::Left if pressed => {
                if !over_ui && self.in_world() && self.camera.mouselook() {
                    self.double_click.reset();
                    self.mouselook_press();
                } else if !over_ui && self.in_world() && !self.camera.mouselook() {
                    // the photo viewfinder: the AF point goes where clicked
                    if self.viewfinder_click() {
                        self.double_click.reset();
                        self.double_press = false;
                        return;
                    }
                    if self.toolbox_pick_press() {
                        self.double_click.reset();
                        self.double_press = false;
                        return;
                    }
                    if self.beacon_hover() {
                        self.world.map.clear_track();
                        self.beacon_hit = None;
                        self.double_click.reset();
                        self.double_press = false;
                        return;
                    }
                    // the second press of a double click on the world
                    // (src/autopilot.rs), read by the press it starts
                    self.double_press = self.double_click.press(Instant::now(), self.cursor_pos);
                    self.click(Click::Press);
                } else {
                    self.double_click.reset();
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
            Click::Gun { at_cursor } => self.gun_press(at_cursor),
        }
        self.clicks.running = None;
    }

    /// The cursor ray and the depth under the cursor for the click being
    /// run. None: just asked from the renderer; the caller returns, and its
    /// click is run again when the answer is there.
    pub(super) fn click_pick(&mut self) -> Option<Picked> {
        if let Some(picked) = self.clicks.in_hand.clone() {
            return self.rlva_filter_pick(picked);
        }
        let (x, y) = self.cursor_pos;
        let g = self.gfx.as_mut()?;
        let ray = g.renderer.cursor_ray(x, y);
        // Capture before waiting for object depth: a moving particle must not
        // have to remain under this pixel for another two or three frames.
        let particles = if !self.settings.world_overlays.hide_particles && matches!(self.clicks.running, None | Some(Click::Menu)) {
            ray.map(|ray| {
                self.scene
                    .particles
                    .pick_all(ray, g.renderer.particle_view(), self.settings.draw_distance)
            })
            .filter(|hits| !hits.is_empty())
            .map(|mut hits| {
                for hit in &mut hits {
                    if let Some(owner) = self
                        .world
                        .objects
                        .index_of_uuid(&hit.source)
                        .and_then(|i| self.world.objects.get(i))
                        .map(|o| o.owner_id)
                        .filter(|id| !id.is_nil())
                    {
                        hit.owner = owner;
                    }
                }
                std::sync::Arc::from(hits)
            })
        } else {
            None
        };
        let mode = pick_mode();
        let Some(click) = self.clicks.running.filter(|_| mode != PickMode::Wait) else {
            let t = Instant::now();
            let depth = g.renderer.pick_world(x, y);
            self.note_pick(format_args!("the read waited {:.2} ms", t.elapsed().as_secs_f32() * 1000.0));
            // (the read that waits knows nothing of the faces drawn: a
            // rigged mesh is not designated, as before)
            let picked = Picked {
                ray,
                depth,
                drawn: Default::default(),
                tag: super::plate_click::visible_tag(&self.name_tag_candidates(ray), depth, ray),
                particles,
            };
            return self.rlva_filter_pick(picked);
        };
        let check = (mode == PickMode::Check).then(|| {
            let t = Instant::now();
            let depth = g.renderer.pick_world(x, y);
            (depth, t.elapsed())
        });
        let ticket = g.renderer.request_pick(x, y);
        // LLToolCamera::handleMouseDown: displayed surfaces, including the
        // transparency overlay. Posed geometry avoids an attachment's bind pose.
        // (Alt in mouselook frees the cursor for the grab tool, not the camera.)
        let camera = (self.alt && !matches!(click, Click::Gun { .. }))
            || self.object_view
            || (self.build.open && self.build.tool == crate::build::Tool::Focus);
        let mode = if camera {
            aurora_render::DrawnMode::Visible
        } else {
            aurora_render::DrawnMode::Interaction
        };
        let drawn = Some(g.renderer.request_drawn(x, y, mode));
        let tags = self.name_tag_candidates(ray);
        let check = check.map(|(depth, waited)| Check {
            depth,
            target: self.click_target(click, ray, depth),
            waited,
        });
        self.clicks.pending = Some(Pending {
            ticket,
            drawn,
            depth_answer: None,
            drawn_answer: None,
            click,
            cursor: (x, y),
            mods: [self.alt, self.ctrl, self.shift],
            ray,
            particles,
            tags,
            asked: Instant::now(),
            frame: self.frame_count,
            check,
        });
        None
    }

    // RlvOverlayEffect::blockTouch removes object hits only: transparent
    // holes, HUD attachments and terrain navigation retain normal behavior.
    fn rlva_filter_pick(&self, picked: Picked) -> Option<Picked> {
        if !self.rlva_overlay_blocks() {
            return Some(picked);
        }
        let object = picked
            .drawn
            .blend
            .or(picked.drawn.opaque)
            .and_then(|face| self.scene.record_object(face.record).map(|(idx, _)| idx))
            .or_else(|| {
                if pick_mode() == PickMode::Wait {
                    picked
                        .depth
                        .zip(picked.ray)
                        .and_then(|(point, ray)| self.scene.interaction_at_ray(&self.world, point, ray, true))
                } else {
                    None
                }
            });
        if picked.tag.is_some() || picked.particles.is_some() || object.is_some_and(|idx| self.world.hud_root(idx).is_none()) {
            None
        } else {
            Some(picked)
        }
    }

    /// The click that waited is run again when its depth has come back,
    /// then what was held behind it, until one of those waits in turn.
    /// Once a frame, with the renderer in `self.gfx`.
    pub(super) fn resolve_clicks(&mut self) {
        while let Some(p) = self.clicks.pending.as_mut() {
            let Some(g) = self.gfx.as_mut() else {
                return;
            };
            if p.depth_answer.is_none() {
                p.depth_answer = g.renderer.take_pick(p.ticket);
            }
            if let Some(ticket) = p.drawn
                && p.drawn_answer.is_none()
            {
                p.drawn_answer = g.renderer.take_drawn(ticket);
            }
            let answer = match (p.depth_answer, p.drawn, p.drawn_answer) {
                (Some(depth), None, _) => Some((depth, Default::default())),
                (Some(depth), Some(_), Some(drawn)) => Some((depth, drawn)),
                _ if p.asked.elapsed() < GIVE_UP => return,
                _ => {
                    log::warn!(
                        "click pick: no answer after {:.1} s, click dropped",
                        p.asked.elapsed().as_secs_f32()
                    );
                    g.renderer.forget_pick(p.ticket);
                    if let Some(ticket) = p.drawn {
                        g.renderer.forget_drawn(ticket);
                    }
                    None
                }
            };
            let Some(p) = self.clicks.pending.take() else {
                return;
            };
            if let Some((depth, drawn)) = answer
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
                let tag = super::plate_click::visible_tag(&p.tags, depth, p.ray);
                self.clicks.in_hand = Some(Picked {
                    ray: p.ray,
                    depth,
                    drawn,
                    tag,
                    particles: p.particles,
                });
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
            Held::Button { button, pressed, over_ui } => {
                self.with_mouse_mods(|app| app.mouse_button(button, pressed, over_ui));
            }
            Held::Motion(dx, dy) => self.camera_mouse_delta(dx, dy),
            Held::Click(click) => self.run_click(click),
            Held::LeftRelease => self.on_left_release(),
            Held::BuildRelease => self.build_release(),
            Held::Note(note) => self.demo_note(note),
        }
    }

    /// The object a click designates from the faces drawn under its pixel
    /// (scene/drawn.rs); None when the search on the CPU speaks for it.
    /// `found`: the point that search settled on.
    pub(super) fn designate(
        &mut self,
        gesture: crate::scene::drawn::Gesture,
        picked: &Picked,
        found: Option<Vec3>,
    ) -> Option<crate::scene::drawn::DrawnObject> {
        let shown = self.scene.designated(
            &self.world,
            gesture,
            &picked.drawn,
            picked.ray,
            found,
            picked.depth,
            self.build.open,
        );
        if self.clicks.scripted()
            || self.demo && (std::env::var_os("AURORA_DEMO_ACTIONS").is_some() || std::env::var_os("AURORA_DEMO_WORLD_CLICKS").is_some())
        {
            // AURORA_DEMO_CLICKS: what the click designates, against two
            // things the CPU knows by itself: the object the click was
            // aimed at (`rigged`), and the posed bounds of the object
            // designated, which must hold the point read from the GPU
            let local = |idx: usize| self.world.objects.get(idx).map(|o| o.key.local_id);
            let got = shown.and_then(|d| local(d.idx));
            let raw = |f: Option<aurora_render::DrawnFace>| f.and_then(|f| self.scene.record_object(f.record)).and_then(|(i, _)| local(i));
            let aimed = match self.clicks.demo.expected.take() {
                Some(expected) if got == Some(expected) => format!("aimed at {expected}: OK"),
                Some(expected) => format!("aimed at {expected}: MISSED"),
                None => "not aimed".to_string(),
            };
            let bounds = shown.and_then(|d| {
                let g = self.scene.gpu.get(d.idx)?;
                let off = d.hit.point.distance(g.center);
                Some(format!(
                    "{} its posed bounds ({off:.2} m from the centre, radius {:.2})",
                    if off <= g.radius * 1.05 + 0.05 { "inside" } else { "OUTSIDE" },
                    g.radius
                ))
            });
            log::info!(
                "demo drawn pick: {gesture:?} {aimed}; designated {got:?}, face {:?}, blended {:?}, {}; drawn: opaque {:?}, blend {:?}",
                shown.and_then(|d| d.face),
                shown.map(|d| d.blend),
                bounds.unwrap_or_else(|| "nothing to bound".into()),
                raw(picked.drawn.opaque),
                raw(picked.drawn.blend),
            );
        }
        shown
    }

    /// The clicks that wait and what is held behind them are given up: the
    /// window lost the focus, the viewer leaves, a teleport starts.
    pub(super) fn drop_clicks(&mut self) {
        if let Some(p) = self.clicks.pending.take()
            && let Some(g) = self.gfx.as_mut()
        {
            g.renderer.forget_pick(p.ticket);
            if let Some(ticket) = p.drawn {
                g.renderer.forget_drawn(ticket);
            }
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
                "demo camera f{frame}: {} (mouse {:?}, agent yaw {:.2}, right orbit={}, menu={}, down={}, cursor held={}, alt free={})",
                self.camera.describe(),
                self.mouse_mode,
                self.world.agent.yaw,
                self.right_gesture.orbit,
                self.context_menu.is_some(),
                self.input.down,
                self.cursor_hold.held(),
                self.alt_cursor.free()
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
            DemoNote::Mmo => log::info!(
                "demo mmo: mode {:?}, pos {:?}, walks={}, pending={}, consumed={}, menu={}",
                self.mouse_mode,
                self.world.agent.position,
                self.mouse_walks(),
                self.world_mouse_walk_pending,
                self.right_gesture.walked,
                self.context_menu.is_some()
            ),
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
        let (point, idx) =
            if click == Click::Menu || tools && matches!(self.build.tool, crate::build::Tool::Edit | crate::build::Tool::Grab) {
                self.scene.selection_pick(
                    &self.world,
                    ray,
                    depth,
                    build,
                    self.settings.build.select_probes,
                    self.settings.draw_distance,
                )
            } else if tools {
                (depth, depth.and_then(|p| self.scene.pick_at(&self.world, p, now)))
            } else {
                let point = self.scene.action_point(&self.world, ray, depth, build, self.settings.draw_distance);
                let idx = if self.alt || self.object_view {
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
            DemoAt::Rigged => {
                self.clicks.demo.count += 1;
                let Some((local, at)) = self.demo_rigged_target(self.clicks.demo.count, (w, h)) else {
                    return;
                };
                self.clicks.demo.expected = Some(local);
                at
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

impl App {
    /// AURORA_DEMO_CLICKS=...,rigged: the `n`th click aims at a rigged mesh
    /// on screen, in turn: its local id and the pixel of the middle of its
    /// posed bounds. The first call completes the fixture: a rigged mesh on
    /// our own avatar, and every rigged mesh scripted for touch, so a left
    /// click shows what it reaches.
    fn demo_rigged_target(&mut self, n: u32, (w, h): (f32, f32)) -> Option<(u32, (f32, f32))> {
        if !std::mem::replace(&mut self.clicks.demo.fixture, true) {
            let me = self.world.objects.index_of_uuid(&self.world.agent_id);
            if let Some(wearer) = me.and_then(|i| self.world.objects.get(i)).map(|o| o.key.local_id) {
                for ev in crate::demo::own_rigged_events(wearer) {
                    self.world.apply(ev);
                }
            }
            return None;
        }
        let mut targets: Vec<(u32, usize, (f32, f32))> = self
            .scene
            .gpu
            .iter()
            .enumerate()
            .filter(|(_, g)| g.rigged && !g.is_avatar && !g.hud && !g.faces.is_empty() && g.radius > 0.0)
            .filter_map(|(idx, g)| {
                let at = self.build.cam.project_px(g.center)?;
                let inside = at.0 > w * 0.03 && at.0 < w * 0.97 && at.1 > h * 0.08 && at.1 < h * 0.95;
                inside.then_some((self.world.objects.get(idx)?.key.local_id, idx, at))
            })
            .collect();
        targets.sort_unstable_by_key(|t| t.0);
        let (local, idx, at) = *targets.get(n as usize % targets.len().max(1))?;
        // (HANDLE_TOUCH: the simulator sets it on a scripted object)
        if let Some(o) = self.world.objects.get_mut(idx) {
            o.update_flags |= ui::context::flags::HANDLE_TOUCH;
        }
        Some((local, (at.0.floor(), at.1.floor())))
    }
}

/// AURORA_DEMO_CLICKS="<ms>[,x,y|sweep|rigged][,left|alt|menu|build|focus]": a
/// click on the world every `ms` milliseconds (250 if it is not a number)
/// once the world shows, held 60 ms: at pixel (x, y) of the window (a
/// quarter of the width, near the bottom, by default), or with `sweep` at a
/// new place of the window each time, or with `rigged` at the rigged meshes
/// on screen in turn (`demo drawn pick` in the log says what each click
/// designated).
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
    Rigged,
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
        _ if parts.contains(&"rigged") => DemoAt::Rigged,
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
    /// `rigged`: the fixture is complete, and the object the click in
    /// flight was aimed at (local id).
    fixture: bool,
    expected: Option<u32>,
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
        let d = demo_spec("300,rigged,menu");
        assert_eq!((d.at, d.kind), (DemoAt::Rigged, DemoKind::Menu));
        // a sweep stays in the window and does not repeat itself soon
        let places: Vec<(f32, f32)> = (1..200).map(sweep_point).collect();
        assert!(places.iter().all(|(u, v)| (0.0..1.0).contains(u) && (0.0..1.0).contains(v)));
        let cells: std::collections::HashSet<(u32, u32)> = places.iter().map(|(u, v)| ((u * 8.0) as u32, (v * 8.0) as u32)).collect();
        assert!(cells.len() > 56, "{} of 64 cells", cells.len());
    }
}
