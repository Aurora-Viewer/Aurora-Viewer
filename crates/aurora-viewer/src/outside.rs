//! The interface's windows outside the viewer's window: native windows of
//! their own, on any screen, drawn by the viewer's renderer
//! (aurora-render's outside.rs). Not in the Legacy Viewer, whose windows
//! all stay inside its own.
//!
//! egui calls such a window a viewport. Its pass runs inside the viewer's
//! own (`Context::show_viewport_immediate`), where the code that draws the
//! window is: that code is the same inside and outside, and borrows what
//! it borrows today. egui hands the pass to one function per thread
//! (`Context::set_immediate_viewport_renderer`), which gives it the native
//! window's size and scale and keeps what it drew; [`Outside::hand`] then
//! gives the renderer the windows whose picture changed. A window that
//! shows the same thing costs its pass and a comparison, no drawing.
//!
//! Each window reads its own events of the system, as the viewer's does
//! (`egui_winit::State`): the pointer, the wheel, the keys, the text. A
//! key that no field or menu of the window reads goes on to the viewer's
//! own handling (shortcuts, movement), as under a window inside; so do
//! the releases, the modifiers and the focus that goes, which let go of
//! what was held. What a window copies leaves with the viewer's own
//! output: one clipboard, and the same rules on what may be copied.
//! [`Outside::focused`]: the viewer is in front while any of its windows
//! has the focus (`App::in_front`).
//!
//! A window of the interface goes outside when its title bar's button asks
//! for it (`widgets::Floater::goes_outside`: every window whose title bar
//! the window outside draws, but the Dev Toolbox; none goes by itself).
//! The wish is kept with the interface's layout
//! ([`wanted`]); the window draws its body in its window outside once
//! there is one ([`native`]), asks for it otherwise ([`ask`]), and says at
//! each pass that it drew there ([`seen`]). [`Outside::reconcile`], before
//! the next pass, opens what was asked and lets go of the windows nothing
//! drew in: closed, hidden with the interface, or back inside.
//!
//! The native windows are made and let go between passes
//! ([`Outside::open`], [`Outside::close`]), with the event loop the frame
//! has. A closed window stays until the render thread has destroyed its
//! surface, and all of them go at exit while the event loop still lives
//! (`App::exiting`: on Wayland an object destroyed after the connection is
//! a crash).

pub(crate) mod carry;
pub(crate) mod demo;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use aurora_render::{EguiHandOver, OutsideEvent, OutsideFrame, Renderer};
use egui::epaint::ClippedShape;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowId};

/// What a window outside asks with an event of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    Nothing,
    /// The system's close button.
    Close(egui::ViewportId),
    /// The viewer's own handling takes the event too, without its
    /// interface: a key the window does not read, a release, the
    /// modifiers, the focus that goes.
    Viewer,
}

/// Whether a key of a window outside goes on to the viewer's own handling:
/// a press only when no field or menu of the window reads the keyboard, a
/// release always (what the press held is let go, wherever it was read).
fn key_goes_on(pressed: bool, reads_keys: bool) -> bool {
    !pressed || !reads_keys
}

/// The viewer is in front: its own window or one of its windows outside
/// has the focus. The frame rate, the textures and the pause of a viewer
/// behind follow this, not the focus of the viewer's own window alone.
pub(crate) fn in_front(own_window: bool, window_outside: bool) -> bool {
    own_window || window_outside
}

/// A window of the interface asks for its window outside.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ask {
    /// The window's id in the interface.
    pub window: egui::Id,
    pub title: String,
    /// Where it is in the viewer's window, in points: its window outside
    /// opens there, where the system lets a program place its windows.
    pub rect: egui::Rect,
    pub min_size: egui::Vec2,
    /// Its size is its content's (`resize` with `held`): the user does not
    /// resize it.
    pub content_sized: bool,
}

/// The viewport of the window `window` of the interface.
pub(crate) fn viewport(window: egui::Id) -> egui::ViewportId {
    egui::ViewportId(window)
}

fn asks_id() -> egui::Id {
    egui::Id::unique("aurora outside asks")
}

fn seen_id() -> egui::Id {
    egui::Id::unique("aurora outside seen")
}

/// The user sent the window `window` outside: kept with the interface's
/// layout, like its place inside.
pub(crate) fn wanted(ctx: &egui::Context, window: egui::Id) -> bool {
    ctx.data_mut(|d| d.get_persisted::<bool>(window.with("outside"))).unwrap_or(false)
}

pub(crate) fn want(ctx: &egui::Context, window: egui::Id, outside: bool) {
    ctx.data_mut(|d| d.insert_persisted(window.with("outside"), outside));
}

/// The window `window` has its window outside: it draws there.
pub(crate) fn native(ctx: &egui::Context, window: egui::Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(window.with("outside native"))).unwrap_or(false)
}

pub(crate) fn set_native(ctx: &egui::Context, window: egui::Id, native: bool) {
    ctx.data_mut(|d| {
        if native {
            d.insert_temp(window.with("outside native"), true);
        } else {
            d.remove::<bool>(window.with("outside native"));
        }
    });
}

fn front_id() -> egui::Id {
    egui::Id::unique("aurora outside front")
}

fn raises_id() -> egui::Id {
    egui::Id::unique("aurora outside raises")
}

fn sizes_id() -> egui::Id {
    egui::Id::unique("aurora outside sizes")
}

/// Asks the system to give the window outside of `window` the size `size`
/// (in the interface's points), at the next frame: what sizes a window
/// inside by its name (`widgets::size_floater`), and the size of its
/// content for a window that follows it (`held`: the user cannot change
/// it). The last size asked wins; a size larger than the screen is brought
/// down to it (`fitting`).
pub(crate) fn resize(ctx: &egui::Context, window: egui::Id, size: egui::Vec2, held: bool) {
    ctx.data_mut(|d| {
        let sizes = d.get_temp_mut_or_default::<Vec<Resize>>(sizes_id());
        sizes.retain(|(asked, ..)| *asked != window);
        sizes.push((window, size, held));
    });
}

/// A size asked of a window outside: the window, the size, held or not.
type Resize = (egui::Id, egui::Vec2, bool);

/// The share of the screen's height a window sized by its content takes at
/// most: the system's bars take the rest, and no system tells their size
/// through winit.
const SCREEN_SHARE: f32 = 0.9;

/// `size` brought down to the screen the window is on, `room` in the
/// interface's points: its content scrolls in what is left.
fn fitting(size: egui::Vec2, room: Option<egui::Vec2>) -> egui::Vec2 {
    room.map_or(size, |room| egui::vec2(size.x.min(room.x), size.y.min(room.y * SCREEN_SHARE)))
}

/// The window `window` is in its window outside and that one has the
/// system's focus: it is the front window, and the keyboard is its own.
pub(crate) fn in_focus(ctx: &egui::Context, window: egui::Id) -> bool {
    ctx.data(|d| d.get_temp::<egui::Id>(front_id())) == Some(window)
}

/// Asks the system to bring the window outside of `window` in front, at
/// the next frame: what brings a window inside to the top (a click on a
/// notification, its toolbar button). The system may only signal it where
/// it lets no program take the focus by itself (Wayland).
pub(crate) fn raise(ctx: &egui::Context, window: egui::Id) {
    ctx.data_mut(|d| {
        let raises = d.get_temp_mut_or_default::<Vec<egui::Id>>(raises_id());
        if !raises.contains(&window) {
            raises.push(window);
        }
    });
}

/// The sizes asked and not given yet.
#[cfg(test)]
pub(crate) fn resizes(ctx: &egui::Context) -> Vec<Resize> {
    ctx.data(|d| d.get_temp(sizes_id())).unwrap_or_default()
}

/// Asks for a window outside, opened before the next pass.
pub(crate) fn ask(ctx: &egui::Context, ask: Ask) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Ask>>(asks_id()).push(ask));
}

/// The window `window` drew in its window outside at this pass.
pub(crate) fn seen(ctx: &egui::Context, window: egui::Id) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<egui::Id>>(seen_id()).push(window));
}

/// The windows outside to let go of: those of the interface that none
/// drew in at the last pass.
fn unseen(open: impl Iterator<Item = (egui::ViewportId, bool)>, seen: &[egui::Id]) -> Vec<egui::ViewportId> {
    open.filter(|(id, of_interface)| *of_interface && !seen.contains(&id.0))
        .map(|(id, _)| id)
        .collect()
}

/// What a window outside was when the renderer was replaced, for the one
/// that takes its place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Was {
    /// Where it was on the screen, where the system says so (not Wayland).
    pub at: Option<winit::dpi::PhysicalPosition<i32>>,
    pub focused: bool,
}

pub(crate) struct Outside {
    /// Shared with egui's function for the passes of this thread.
    shared: Rc<RefCell<Shared>>,
    /// The windows of the interface a replaced renderer left without a
    /// surface (`renderer_replaced`), until each asks for its new window.
    replaced: std::collections::HashMap<egui::ViewportId, Was>,
    /// Closed windows whose surface the render thread still has, by key.
    closing: Vec<(u64, Arc<Window>)>,
    /// Keys given so far: one names a window for the renderer.
    keys: u64,
}

#[derive(Default)]
struct Shared {
    windows: HashMap<egui::ViewportId, Native>,
    /// The texture changes of this frame's passes outside, older than the
    /// viewer's own pass's: they leave with its interface, before them.
    textures: egui::TexturesDelta,
    /// What this frame's passes outside ask of the system besides their
    /// own window (a text to copy, a link to open): the viewer's output
    /// takes them.
    commands: Vec<egui::OutputCommand>,
}

struct Native {
    key: u64,
    window: Arc<Window>,
    /// A window of the interface (`Ask`), let go of when nothing draws in
    /// it; the others are closed by who opened them.
    of_interface: bool,
    /// The title the system shows.
    title: Option<String>,
    /// The window's events of the system, as egui reads them.
    input: egui_winit::State,
    focused: bool,
    /// A field or a menu of the window read the keyboard at its last pass.
    reads_keys: bool,
    /// What its pass left this frame.
    pass: Option<Pass>,
    /// What its pass asks of its own window: the cursor, the text input.
    output: Option<egui::PlatformOutput>,
    /// What was handed to the renderer last, and at which size.
    shown: Vec<ClippedShape>,
    shown_size: (u32, u32),
    /// Draw it again whatever it shows (a new window, one the system
    /// uncovered or resized).
    redraw: bool,
    /// The system asked for its first picture. Until then nothing is
    /// drawn in it: where the program draws the title bar itself (Wayland),
    /// the window library draws it just before it asks, and a picture
    /// shown earlier has the system count the window without its title
    /// bar, then shrink it by that height.
    asked_for: bool,
}

struct Pass {
    shapes: Vec<ClippedShape>,
    pixels_per_point: f32,
}

/// What the pass of a window outside starts from: the events `raw` of the
/// window, on a screen of its size at its scale, on the viewer's clock.
fn pass_input(
    raw: egui::RawInput,
    ids: egui::ViewportIdPair,
    size: (u32, u32),
    scale: f32,
    zoom: f32,
    focused: bool,
    (time, predicted_dt, max_texture_side): (f64, f32, usize),
) -> egui::RawInput {
    let points = egui::vec2(size.0 as f32, size.1 as f32) / (scale * zoom).max(0.01);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
    let info = egui::ViewportInfo {
        parent: Some(ids.parent),
        native_pixels_per_point: Some(scale),
        inner_rect: Some(screen),
        focused: Some(focused),
        ..Default::default()
    };
    egui::RawInput {
        viewport_id: ids.this,
        viewports: std::iter::once((ids.this, info)).collect(),
        screen_rect: Some(screen),
        max_texture_side: Some(max_texture_side),
        time: Some(time),
        predicted_dt,
        focused,
        ..raw
    }
}

/// A field or a menu of the pass under way reads the keyboard.
fn reads_keys(ctx: &egui::Context) -> bool {
    ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx)
}

/// Whether a window whose pass left `shapes` at `size` is drawn again;
/// never before the system asked for its first picture (`asked_for`).
fn drawn_again(
    asked_for: bool,
    redraw: bool,
    textures_changed: bool,
    size: (u32, u32),
    shown_size: (u32, u32),
    shapes: &[ClippedShape],
    shown: &[ClippedShape],
) -> bool {
    asked_for && (redraw || textures_changed || size != shown_size || shapes != shown)
}

/// egui's function for the passes outside: the pass of `viewport`, run at
/// once inside the pass that asked for it.
fn pass(shared: &Rc<RefCell<Shared>>, ctx: &egui::Context, viewport: egui::ImmediateViewport<'_>) {
    let egui::ImmediateViewport {
        ids,
        builder,
        mut viewport_ui_cb,
    } = viewport;
    let clock = ctx.input(|i| (i.time, i.predicted_dt, i.max_texture_side));
    let zoom = ctx.zoom_factor();
    let input = shared.borrow_mut().windows.get_mut(&ids.this).map(|native| {
        if builder.title.is_some() && builder.title != native.title {
            native.window.set_title(builder.title.as_deref().unwrap_or_default());
            native.title = builder.title;
        }
        let size = native.window.inner_size();
        pass_input(
            native.input.take_egui_input(&native.window),
            ids,
            (size.width, size.height),
            native.window.scale_factor() as f32,
            zoom,
            native.focused,
            clock,
        )
    });
    // (egui wants the window's code run whatever happens: `Outside::show`
    // only asks for a window that is open, so this is never the case)
    let input = input.unwrap_or_else(|| pass_input(egui::RawInput::default(), ids, (640, 480), 1.0, zoom, false, clock));
    let mut keys = false;
    // One pass, where egui may run two to lay a new content out: the
    // window's code is run once (a `Floater`'s body is `FnOnce`), and what
    // asks for a second pass settles at the next frame instead.
    let passes = ctx.options(|o| o.max_passes);
    ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::MIN);
    // (nothing of `shared` is borrowed while the window's code runs)
    let mut out = ctx.run_ui(input, |ui| {
        viewport_ui_cb(ui);
        keys = reads_keys(ui.ctx());
    });
    ctx.options_mut(|o| o.max_passes = passes);
    let mut shared = shared.borrow_mut();
    shared.textures.append(out.textures_delta);
    shared.commands.append(&mut out.platform_output.commands);
    if let Some(native) = shared.windows.get_mut(&ids.this) {
        native.reads_keys = keys;
        native.output = Some(out.platform_output);
        native.pass = Some(Pass {
            shapes: out.shapes,
            pixels_per_point: out.pixels_per_point,
        });
    }
}

impl Outside {
    /// Once, on the thread of the interface.
    pub fn new(ctx: &egui::Context) -> Outside {
        // (egui draws a viewport inside the window that asks for it until
        // it is told there are native ones)
        ctx.set_embed_viewports(false);
        let shared = Rc::new(RefCell::new(Shared::default()));
        let for_passes = shared.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| pass(&for_passes, ctx, viewport));
        Outside {
            shared,
            replaced: Default::default(),
            closing: Vec::new(),
            keys: 0,
        }
    }

    pub fn is_open(&self, id: egui::ViewportId) -> bool {
        self.shared.borrow().windows.contains_key(&id)
    }

    /// The renderer was replaced (a GPU recovery, the VR runtime closed):
    /// the new one has no surface, and none can be made on a window the
    /// lost device's surface is left on, which is why the viewer's own
    /// window is replaced too. The windows outside go; those of the
    /// interface ask for new ones at their next pass, opened where these
    /// were (`reconcile`), and a scenario opens its own again.
    pub fn renderer_replaced(&mut self, ctx: &egui::Context) {
        self.closing.clear();
        let windows = std::mem::take(&mut self.shared.borrow_mut().windows);
        for (id, native) in windows {
            set_native(ctx, id.0, false);
            if native.of_interface {
                self.replaced.insert(
                    id,
                    Was {
                        at: native.window.outer_position().ok(),
                        focused: native.focused,
                    },
                );
            }
        }
    }

    /// Before the interface's pass: opens the windows outside that the
    /// interface's windows asked for at the last pass, with the attributes
    /// `attributes` gives each (and what the window it replaces was, after
    /// `renderer_replaced`), and lets go of those none drew in.
    pub fn reconcile(
        &mut self,
        event_loop: &ActiveEventLoop,
        renderer: &mut Renderer,
        ctx: &egui::Context,
        attributes: impl Fn(&Ask, Option<Was>) -> WindowAttributes,
    ) {
        let (asks, seen, raises, sizes) = ctx.data_mut(|d| {
            (
                d.remove_temp::<Vec<Ask>>(asks_id()).unwrap_or_default(),
                d.remove_temp::<Vec<egui::Id>>(seen_id()).unwrap_or_default(),
                d.remove_temp::<Vec<egui::Id>>(raises_id()).unwrap_or_default(),
                d.remove_temp::<Vec<Resize>>(sizes_id()).unwrap_or_default(),
            )
        });
        let zoom = ctx.zoom_factor();
        for (window, size, held) in sizes {
            if let Some(native) = self.shared.borrow().windows.get(&viewport(window)) {
                let room = native.window.current_monitor().map(|monitor| {
                    let room = monitor.size().to_logical::<f32>(monitor.scale_factor());
                    egui::vec2(room.width / zoom, room.height / zoom)
                });
                let size = fitting(size, room);
                let size = winit::dpi::LogicalSize::new(size.x * zoom, size.y * zoom);
                // a held size is the least and the most the system allows:
                // both are lifted for the change, then set to the new size
                if held {
                    native.window.set_min_inner_size(None::<winit::dpi::LogicalSize<f32>>);
                    native.window.set_max_inner_size(None::<winit::dpi::LogicalSize<f32>>);
                }
                // (the system answers with a resize of the window, or not at all)
                let _ = native.window.request_inner_size(size);
                if held {
                    native.window.set_min_inner_size(Some(size));
                    native.window.set_max_inner_size(Some(size));
                }
            }
        }
        for window in raises {
            if let Some(native) = self.shared.borrow().windows.get(&viewport(window)) {
                native.window.set_minimized(false);
                native.window.focus_window();
            }
        }
        let front = self
            .shared
            .borrow()
            .windows
            .iter()
            .find(|(_, native)| native.of_interface && native.focused)
            .map(|(id, _)| id.0);
        ctx.data_mut(|d| match front {
            Some(front) => {
                d.insert_temp(front_id(), front);
            }
            None => {
                d.remove::<egui::Id>(front_id());
            }
        });
        let gone = unseen(
            self.shared.borrow().windows.iter().map(|(id, native)| (*id, native.of_interface)),
            &seen,
        );
        for id in gone {
            self.close(renderer, ctx, id);
        }
        for ask in asks {
            let id = viewport(ask.window);
            if self.is_open(id) {
                continue;
            }
            let was = self.replaced.remove(&id);
            match self.open(event_loop, renderer, ctx, id, attributes(&ask, was), false) {
                Ok(()) => {
                    if let Some(native) = self.shared.borrow_mut().windows.get_mut(&id) {
                        native.of_interface = true;
                        native.title = Some(ask.title);
                    }
                    set_native(ctx, ask.window, true);
                }
                // the window stays inside the viewer's
                Err(e) => {
                    log::warn!("window outside: \"{}\" could not be opened: {e}", ask.title);
                    want(ctx, ask.window, false);
                }
            }
        }
    }

    /// A native window for `id`, drawn by `renderer` from the next frame
    /// on. `sync` synchronizes it with its screen: only to measure
    /// (`Renderer::open_outside`).
    pub fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        renderer: &mut Renderer,
        ctx: &egui::Context,
        id: egui::ViewportId,
        attributes: WindowAttributes,
        sync: bool,
    ) -> Result<(), String> {
        if self.is_open(id) {
            return Ok(());
        }
        let window = Arc::new(event_loop.create_window(attributes).map_err(|e| e.to_string())?);
        // (its first picture waits for the system to ask for one)
        window.request_redraw();
        let size = window.inner_size();
        self.keys += 1;
        let key = self.keys;
        renderer
            .open_outside(key, window.clone(), size.width, size.height, sync)
            .map_err(|e| e.to_string())?;
        let input = egui_winit::State::new(
            ctx.clone(),
            id,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(renderer.device.limits().max_texture_dimension_2d as usize),
        );
        let focused = window.has_focus();
        self.shared.borrow_mut().windows.insert(
            id,
            Native {
                key,
                window,
                of_interface: false,
                title: None,
                input,
                focused,
                reads_keys: false,
                pass: None,
                output: None,
                shown: Vec::new(),
                shown_size: (0, 0),
                redraw: true,
                asked_for: false,
            },
        );
        Ok(())
    }

    /// The window of `id` goes: hidden at once, gone when the render
    /// thread has destroyed its surface.
    pub fn close(&mut self, renderer: &mut Renderer, ctx: &egui::Context, id: egui::ViewportId) {
        let Some(native) = self.shared.borrow_mut().windows.remove(&id) else {
            return;
        };
        set_native(ctx, id.0, false);
        native.window.set_visible(false);
        renderer.close_outside(native.key);
        self.closing.push((native.key, native.window));
    }

    /// What the render thread answered since the last frame; returns the
    /// windows it cannot draw, which are closed: one of the interface goes
    /// back inside the viewer's.
    pub fn answers(&mut self, renderer: &mut Renderer, ctx: &egui::Context) -> Vec<egui::ViewportId> {
        let mut failed = Vec::new();
        for event in renderer.outside_events() {
            match event {
                OutsideEvent::Closed(key) => self.closing.retain(|(k, _)| *k != key),
                // (its surface is destroyed already)
                OutsideEvent::Failed(key, _) => {
                    let mut shared = self.shared.borrow_mut();
                    let id = shared.windows.iter().find(|(_, native)| native.key == key).map(|(id, _)| *id);
                    if let Some(id) = id
                        && let Some(native) = shared.windows.remove(&id)
                    {
                        set_native(ctx, id.0, false);
                        if native.of_interface {
                            want(ctx, id.0, false);
                        }
                        failed.push(id);
                    }
                }
            }
        }
        failed
    }

    /// Draws `add` in the window outside of `id`, if it has one: its pass,
    /// inside the pass under way.
    pub fn show<R>(&self, ctx: &egui::Context, id: egui::ViewportId, mut add: impl FnMut(&mut egui::Ui) -> R) -> Option<R> {
        if !self.is_open(id) {
            return None;
        }
        Some(ctx.show_viewport_immediate(id, egui::ViewportBuilder::default(), |ui, _| add(ui)))
    }

    /// The inner size of the window outside `id`, in pixels.
    pub fn size_of(&self, id: WindowId) -> Option<winit::dpi::PhysicalSize<u32>> {
        self.shared
            .borrow()
            .windows
            .values()
            .find(|native| native.window.id() == id)
            .map(|native| native.window.inner_size())
    }

    /// One of the windows outside has the focus.
    pub fn focused(&self) -> bool {
        self.shared.borrow().windows.values().any(|native| native.focused)
    }

    /// An event of the system: `None` when `id` is not a window outside
    /// (the viewer's own), else what the window asks. The window's pass
    /// reads the event at the next frame.
    pub fn window_event(&mut self, id: WindowId, event: &WindowEvent) -> Option<Asked> {
        if self.closing.iter().any(|(_, window)| window.id() == id) {
            return Some(Asked::Nothing);
        }
        let mut shared = self.shared.borrow_mut();
        let (viewport, native) = shared.windows.iter_mut().find(|(_, native)| native.window.id() == id)?;
        // (what egui answers is about the viewer's own window between two
        // frames: `reads_keys`, from the window's last pass, is its own)
        let _ = native.input.on_window_event(&native.window, event);
        if matches!(event, WindowEvent::RedrawRequested) {
            native.asked_for = true;
        }
        Some(match event {
            WindowEvent::CloseRequested => Asked::Close(*viewport),
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::RedrawRequested
            | WindowEvent::Occluded(false) => {
                native.redraw = true;
                Asked::Nothing
            }
            WindowEvent::Focused(focused) => {
                native.focused = *focused;
                if *focused { Asked::Nothing } else { Asked::Viewer }
            }
            WindowEvent::ModifiersChanged(_) => Asked::Viewer,
            WindowEvent::KeyboardInput { event: key, .. }
                if key_goes_on(key.state == winit::event::ElementState::Pressed, native.reads_keys) =>
            {
                Asked::Viewer
            }
            _ => Asked::Nothing,
        })
    }

    /// The window of `id` and where the point `at` of its pass is in it,
    /// in pixels: for a scripted pointer.
    pub fn pixel(&self, id: egui::ViewportId, at: egui::Pos2, zoom: f32) -> Option<(WindowId, winit::dpi::PhysicalPosition<f64>)> {
        let shared = self.shared.borrow();
        let native = shared.windows.get(&id)?;
        let scale = native.window.scale_factor() * f64::from(zoom);
        Some((
            native.window.id(),
            winit::dpi::PhysicalPosition::new(f64::from(at.x) * scale, f64::from(at.y) * scale),
        ))
    }

    /// What this frame's passes outside ask of the system besides their
    /// own window, to put before what the viewer's own pass asks.
    pub fn take_commands(&mut self) -> Vec<egui::OutputCommand> {
        std::mem::take(&mut self.shared.borrow_mut().commands)
    }

    /// After the viewer's pass: each window outside takes what its pass
    /// asks of it (the cursor's shape, the text input's place).
    pub fn platform(&mut self, event_loop: &ActiveEventLoop) {
        for native in self.shared.borrow_mut().windows.values_mut() {
            if let Some(output) = native.output.take() {
                native
                    .input
                    .handle_platform_output_with_event_loop(&native.window, event_loop, output);
            }
        }
    }

    /// The texture changes of this frame's passes outside, to put before
    /// those of the viewer's own pass.
    pub fn take_textures(&mut self) -> egui::TexturesDelta {
        std::mem::take(&mut self.shared.borrow_mut().textures)
    }

    /// After the viewer's pass: hands the renderer the windows outside
    /// whose picture changed. `textures_changed`: a texture of the
    /// interface was written this frame (a picture that arrived shows in
    /// the same shapes).
    pub fn hand(
        &mut self,
        ctx: &egui::Context,
        hand_over: &mut EguiHandOver,
        renderer: &mut Renderer,
        clear: egui::Color32,
        textures_changed: bool,
    ) {
        for native in self.shared.borrow_mut().windows.values_mut() {
            let Some(pass) = native.pass.take() else {
                continue;
            };
            let size = native.window.inner_size();
            let size = (size.width, size.height);
            // (a minimized window has no picture)
            if size.0 == 0 || size.1 == 0 {
                continue;
            }
            if !drawn_again(
                native.asked_for,
                native.redraw,
                textures_changed,
                size,
                native.shown_size,
                &pass.shapes,
                &native.shown,
            ) {
                continue;
            }
            native.shown.clone_from(&pass.shapes);
            native.shown_size = size;
            native.redraw = false;
            renderer.draw_outside(
                native.key,
                OutsideFrame {
                    width: size.0,
                    height: size.1,
                    clear,
                    ui: hand_over.frame(ctx, pass.shapes, egui::TexturesDelta::default(), pass.pixels_per_point),
                },
            );
        }
    }

    /// At exit, once the renderer is gone and its surfaces with it: the
    /// native windows go while the event loop still lives.
    pub fn exit(&mut self) {
        self.closing.clear();
        self.shared.borrow_mut().windows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> egui::ViewportIdPair {
        egui::ViewportIdPair {
            this: egui::ViewportId::from_hash_of("window outside"),
            parent: egui::ViewportId::ROOT,
        }
    }

    #[test]
    fn the_pass_of_a_window_outside_has_its_size_in_its_own_points() {
        // a 1200 x 800 window on a screen at 200 %, the interface at 125 %
        let events = egui::RawInput {
            events: vec![egui::Event::PointerMoved(egui::pos2(10.0, 20.0))],
            ..Default::default()
        };
        let input = pass_input(events, ids(), (1200, 800), 2.0, 1.25, true, (12.5, 1.0 / 144.0, 8192));
        assert_eq!(input.viewport_id, ids().this);
        assert_eq!(
            input.screen_rect,
            Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 320.0)))
        );
        let info = &input.viewports[&ids().this];
        assert_eq!(info.native_pixels_per_point, Some(2.0));
        assert_eq!(info.parent, Some(egui::ViewportId::ROOT));
        assert_eq!(info.focused, Some(true));
        // the viewer's clock: an animation runs at one pace in every window
        assert_eq!(input.time, Some(12.5));
        assert_eq!(input.max_texture_side, Some(8192));
        // the window's own events, and no other
        assert_eq!(input.events, [egui::Event::PointerMoved(egui::pos2(10.0, 20.0))]);
    }

    #[test]
    fn a_window_outside_nothing_drew_in_is_let_go() {
        let window = |name: &str| egui::Id::unique(name);
        let open = [
            (viewport(window("conversations")), true),
            (viewport(window("people")), true),
            // (opened by a scenario, which closes it itself)
            (viewport(window("test picture")), false),
        ];
        // Conversations drew in its window; People was closed, hidden with
        // the interface or brought back inside
        let seen = [window("conversations")];
        assert_eq!(unseen(open.into_iter(), &seen), [viewport(window("people"))]);
        assert_eq!(unseen(open.into_iter(), &[]).len(), 2);
    }

    #[test]
    fn a_window_asks_to_go_outside_and_the_wish_is_kept() {
        let ctx = egui::Context::default();
        let window = egui::Id::unique("conversations");
        assert!(!wanted(&ctx, window));
        assert!(!native(&ctx, window));
        want(&ctx, window, true);
        assert!(wanted(&ctx, window));
        // (the wish alone opens nothing: the window asks, then draws)
        assert!(!native(&ctx, window));
        set_native(&ctx, window, true);
        assert!(native(&ctx, window));
        set_native(&ctx, window, false);
        assert!(!native(&ctx, window));
        want(&ctx, window, false);
        assert!(!wanted(&ctx, window));
    }

    #[test]
    fn a_window_outside_is_raised_once_however_often_it_is_asked() {
        let ctx = egui::Context::default();
        let (chat, people) = (egui::Id::unique("conversations"), egui::Id::unique("people"));
        raise(&ctx, chat);
        raise(&ctx, people);
        raise(&ctx, chat);
        let raises = ctx.data(|d| d.get_temp::<Vec<egui::Id>>(raises_id())).unwrap_or_default();
        assert_eq!(raises, [chat, people]);
        // (no window outside has the focus until the host says so)
        assert!(!in_focus(&ctx, chat));
        ctx.data_mut(|d| d.insert_temp(front_id(), chat));
        assert!(in_focus(&ctx, chat));
        assert!(!in_focus(&ctx, people));
    }

    #[test]
    fn the_last_size_asked_of_a_window_outside_wins() {
        let ctx = egui::Context::default();
        let (inventory, people) = (egui::Id::unique("inventory"), egui::Id::unique("people"));
        resize(&ctx, inventory, egui::vec2(600.0, 400.0), false);
        resize(&ctx, people, egui::vec2(300.0, 500.0), true);
        resize(&ctx, inventory, egui::vec2(1180.0, 400.0), false);
        assert_eq!(
            resizes(&ctx),
            [
                (people, egui::vec2(300.0, 500.0), true),
                (inventory, egui::vec2(1180.0, 400.0), false)
            ]
        );
    }

    #[test]
    fn a_size_larger_than_the_screen_is_brought_down_to_it() {
        let room = Some(egui::vec2(1280.0, 800.0));
        assert_eq!(fitting(egui::vec2(400.0, 600.0), room), egui::vec2(400.0, 600.0));
        // taller than the screen less the system's bars: its content scrolls
        assert_eq!(fitting(egui::vec2(400.0, 1100.0), room), egui::vec2(400.0, 720.0));
        assert_eq!(fitting(egui::vec2(1600.0, 300.0), room), egui::vec2(1280.0, 300.0));
        // (a system that does not tell the screen: as asked)
        assert_eq!(fitting(egui::vec2(400.0, 1100.0), None), egui::vec2(400.0, 1100.0));
    }

    /// A restart: the layout is written at exit and read at the next start.
    #[test]
    fn a_window_sent_outside_is_outside_again_after_a_restart() {
        let ctx = egui::Context::default();
        let window = egui::Id::unique("conversations");
        want(&ctx, window, true);
        set_native(&ctx, window, true);
        let saved = crate::ui::layout::text(&ctx).expect("the layout is written");
        let next = egui::Context::default();
        crate::ui::layout::apply(&next, &saved).expect("the layout is read");
        assert!(wanted(&next, window));
        // (its window of the last session is gone: it asks for a new one)
        assert!(!native(&next, window));
        assert!(!wanted(&next, egui::Id::unique("people")));
    }

    #[test]
    fn a_reset_of_the_windows_is_left_for_the_window_to_take() {
        let ctx = egui::Context::default();
        let window = egui::Id::unique("conversations");
        assert!(!crate::ui::layout::reset_pending(&ctx, window));
        crate::ui::layout::reset_windows(&ctx);
        // asked twice, still there: the window outside comes back inside,
        // then takes the reset as a window inside does
        assert!(crate::ui::layout::reset_pending(&ctx, window));
        assert!(crate::ui::layout::reset_pending(&ctx, window));
        assert!(crate::ui::layout::take_reset(&ctx, window));
        assert!(!crate::ui::layout::reset_pending(&ctx, window));
    }

    #[test]
    fn a_key_goes_on_to_the_viewer_unless_the_window_outside_reads_it() {
        // a list, a picture: the key is a shortcut or a movement, as under
        // a window inside
        assert!(key_goes_on(true, false));
        // a text field or an open menu keeps the press
        assert!(!key_goes_on(true, true));
        // what a press held is let go wherever the release arrives
        assert!(key_goes_on(false, true));
        assert!(key_goes_on(false, false));
    }

    #[test]
    fn the_viewer_is_in_front_with_the_focus_in_any_of_its_windows() {
        assert!(in_front(true, false));
        assert!(in_front(false, true));
        assert!(in_front(true, true));
        // another program has the focus
        assert!(!in_front(false, false));
    }

    #[test]
    fn a_focused_field_or_an_open_menu_reads_the_keys() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let mut read = Vec::new();
        for focus in [false, true, true] {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let field = ui.text_edit_singleline(&mut text);
                if focus {
                    field.request_focus();
                }
                read.push(reads_keys(ui.ctx()));
            });
        }
        assert_eq!(read, [false, true, true]);
    }

    #[test]
    fn a_window_outside_is_drawn_again_only_when_its_picture_changed() {
        let ctx = egui::Context::default();
        let shapes = |text: &str| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label(text);
            });
            out.textures_delta.clear();
            out.shapes
        };
        let (first, same, other) = (shapes("one"), shapes("one"), shapes("two"));
        let size = (640, 480);
        assert!(!drawn_again(true, false, false, size, size, &same, &first));
        assert!(drawn_again(true, false, false, size, size, &other, &first));
        // resized, uncovered by the system, or a picture that arrived
        assert!(drawn_again(true, false, false, (800, 480), size, &same, &first));
        assert!(drawn_again(true, true, false, size, size, &same, &first));
        assert!(drawn_again(true, false, true, size, size, &same, &first));
    }

    /// A new window's first picture waits for the system to ask for one:
    /// shown before its title bar is drawn, it made the system count the
    /// window without the title bar and shrink it by that height at each
    /// opening.
    #[test]
    fn nothing_is_drawn_in_a_window_outside_before_the_system_asks() {
        let size = (640, 480);
        // a new window: nothing shown yet, everything to draw
        assert!(!drawn_again(false, true, true, size, (0, 0), &[], &[]));
        assert!(drawn_again(true, true, true, size, (0, 0), &[], &[]));
    }

    /// The pass of a window outside, as egui runs it inside the viewer's:
    /// both leave their own shapes, at their own scale.
    #[test]
    fn a_pass_outside_runs_inside_the_viewer_s_pass() {
        let ctx = egui::Context::default();
        let outside = Outside::new(&ctx);
        let id = ids().this;
        // (no native window in a test: the pass is run by hand, as egui's
        // function does)
        let kept: Rc<RefCell<Option<egui::FullOutput>>> = Rc::default();
        let for_pass = kept.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            let egui::ImmediateViewport {
                ids, mut viewport_ui_cb, ..
            } = viewport;
            let clock = ctx.input(|i| (i.time, i.predicted_dt, i.max_texture_side));
            let input = pass_input(egui::RawInput::default(), ids, (600, 400), 2.0, ctx.zoom_factor(), false, clock);
            *for_pass.borrow_mut() = Some(ctx.run_ui(input, |ui| viewport_ui_cb(ui)));
        });
        let mut root = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("inside");
            let answer = ui.ctx().show_viewport_immediate(id, egui::ViewportBuilder::default(), |ui, class| {
                assert!(class == egui::ViewportClass::Immediate);
                assert_eq!(ui.ctx().viewport_id(), id);
                assert_eq!(ui.ctx().content_rect().size(), egui::vec2(300.0, 200.0));
                ui.label("outside");
                7
            });
            assert_eq!(answer, 7);
            assert_eq!(ui.ctx().viewport_id(), egui::ViewportId::ROOT);
        });
        let mut out = kept.borrow_mut().take().expect("the pass outside ran");
        assert_eq!(out.pixels_per_point, 2.0);
        assert_eq!(root.pixels_per_point, 1.0);
        let texts = |shapes: &[ClippedShape]| -> Vec<String> {
            shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(texts(&out.shapes), ["outside"]);
        assert_eq!(texts(&root.shapes), ["inside"]);
        root.textures_delta.clear();
        out.textures_delta.clear();
        // (without a native window nothing is asked of it)
        assert!(!outside.is_open(id));
        assert_eq!(outside.show(&ctx, id, |_| ()), None);
    }
}
