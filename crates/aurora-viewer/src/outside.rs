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
//! The native windows are made and let go between passes
//! ([`Outside::open`], [`Outside::close`]), with the event loop the frame
//! has. A closed window stays until the render thread has destroyed its
//! surface, and all of them go at exit while the event loop still lives
//! (`App::exiting`: on Wayland an object destroyed after the connection is
//! a crash).

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
}

pub(crate) struct Outside {
    /// Shared with egui's function for the passes of this thread.
    shared: Rc<RefCell<Shared>>,
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
}

struct Native {
    key: u64,
    window: Arc<Window>,
    /// What its pass left this frame.
    pass: Option<Pass>,
    /// What was handed to the renderer last, and at which size.
    shown: Vec<ClippedShape>,
    shown_size: (u32, u32),
    /// Draw it again whatever it shows (a new window, one the system
    /// uncovered or resized).
    redraw: bool,
}

struct Pass {
    shapes: Vec<ClippedShape>,
    pixels_per_point: f32,
}

/// What the pass of a window outside starts from: a screen of the native
/// window's size at its scale, on the viewer's clock.
fn pass_input(
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
        ..Default::default()
    }
}

/// Whether a window whose pass left `shapes` at `size` is drawn again.
fn drawn_again(
    redraw: bool,
    textures_changed: bool,
    size: (u32, u32),
    shown_size: (u32, u32),
    shapes: &[ClippedShape],
    shown: &[ClippedShape],
) -> bool {
    redraw || textures_changed || size != shown_size || shapes != shown
}

/// egui's function for the passes outside: the pass of `viewport`, run at
/// once inside the pass that asked for it.
fn pass(shared: &Rc<RefCell<Shared>>, ctx: &egui::Context, viewport: egui::ImmediateViewport<'_>) {
    let egui::ImmediateViewport {
        ids, mut viewport_ui_cb, ..
    } = viewport;
    let clock = ctx.input(|i| (i.time, i.predicted_dt, i.max_texture_side));
    let zoom = ctx.zoom_factor();
    let input = shared.borrow().windows.get(&ids.this).map(|native| {
        let size = native.window.inner_size();
        pass_input(
            ids,
            (size.width, size.height),
            native.window.scale_factor() as f32,
            zoom,
            native.window.has_focus(),
            clock,
        )
    });
    // (egui wants the window's code run whatever happens: `Outside::show`
    // only asks for a window that is open, so this is never the case)
    let input = input.unwrap_or_else(|| pass_input(ids, (640, 480), 1.0, zoom, false, clock));
    // (nothing of `shared` is borrowed while the window's code runs)
    let out = ctx.run_ui(input, |ui| viewport_ui_cb(ui));
    let mut shared = shared.borrow_mut();
    shared.textures.append(out.textures_delta);
    if let Some(native) = shared.windows.get_mut(&ids.this) {
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
            closing: Vec::new(),
            keys: 0,
        }
    }

    pub fn is_open(&self, id: egui::ViewportId) -> bool {
        self.shared.borrow().windows.contains_key(&id)
    }

    /// A native window for `id`, drawn by `renderer` from the next frame
    /// on. `sync` synchronizes it with its screen: only to measure
    /// (`Renderer::open_outside`).
    pub fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        renderer: &mut Renderer,
        id: egui::ViewportId,
        attributes: WindowAttributes,
        sync: bool,
    ) -> Result<(), String> {
        if self.is_open(id) {
            return Ok(());
        }
        let window = Arc::new(event_loop.create_window(attributes).map_err(|e| e.to_string())?);
        let size = window.inner_size();
        self.keys += 1;
        let key = self.keys;
        renderer
            .open_outside(key, window.clone(), size.width, size.height, sync)
            .map_err(|e| e.to_string())?;
        self.shared.borrow_mut().windows.insert(
            id,
            Native {
                key,
                window,
                pass: None,
                shown: Vec::new(),
                shown_size: (0, 0),
                redraw: true,
            },
        );
        Ok(())
    }

    /// The window of `id` goes: hidden at once, gone when the render
    /// thread has destroyed its surface.
    pub fn close(&mut self, renderer: &mut Renderer, id: egui::ViewportId) {
        let Some(native) = self.shared.borrow_mut().windows.remove(&id) else {
            return;
        };
        native.window.set_visible(false);
        renderer.close_outside(native.key);
        self.closing.push((native.key, native.window));
    }

    /// What the render thread answered since the last frame; returns the
    /// windows it cannot draw, which are closed.
    pub fn answers(&mut self, renderer: &mut Renderer) -> Vec<egui::ViewportId> {
        let mut failed = Vec::new();
        for event in renderer.outside_events() {
            match event {
                OutsideEvent::Closed(key) => self.closing.retain(|(k, _)| *k != key),
                // (its surface is destroyed already)
                OutsideEvent::Failed(key, _) => {
                    let mut shared = self.shared.borrow_mut();
                    let id = shared.windows.iter().find(|(_, native)| native.key == key).map(|(id, _)| *id);
                    if let Some(id) = id {
                        shared.windows.remove(&id);
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

    /// An event of the system: `None` when `id` is not a window outside
    /// (the viewer's own), else what the window asks.
    pub fn window_event(&mut self, id: WindowId, event: &WindowEvent) -> Option<Asked> {
        if self.closing.iter().any(|(_, window)| window.id() == id) {
            return Some(Asked::Nothing);
        }
        let mut shared = self.shared.borrow_mut();
        let (viewport, native) = shared.windows.iter_mut().find(|(_, native)| native.window.id() == id)?;
        Some(match event {
            WindowEvent::CloseRequested => Asked::Close(*viewport),
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::RedrawRequested
            | WindowEvent::Occluded(false) => {
                native.redraw = true;
                Asked::Nothing
            }
            _ => Asked::Nothing,
        })
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
        let input = pass_input(ids(), (1200, 800), 2.0, 1.25, true, (12.5, 1.0 / 144.0, 8192));
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
        // no event of the viewer's own window is the window's outside
        assert!(input.events.is_empty());
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
        assert!(!drawn_again(false, false, size, size, &same, &first));
        assert!(drawn_again(false, false, size, size, &other, &first));
        // resized, uncovered by the system, or a picture that arrived
        assert!(drawn_again(false, false, (800, 480), size, &same, &first));
        assert!(drawn_again(true, false, size, size, &same, &first));
        assert!(drawn_again(false, true, size, size, &same, &first));
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
            let input = pass_input(ids, (600, 400), 2.0, ctx.zoom_factor(), false, clock);
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
