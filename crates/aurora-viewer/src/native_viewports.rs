//! Native egui viewport support for tear-off viewer floaters.
//!
//! The viewer's scene renderer owns the main surface. This host follows egui's
//! immediate-viewport integration model for secondary UI-only windows; it does
//! not create another Aurora scene renderer.

use std::{cell::Cell, collections::HashMap, num::NonZeroU32, rc::Rc, sync::Arc, time::Instant};

use egui::{Context, ImmediateViewport, ViewportId, ViewportInfo};
use egui_wgpu::{RendererOptions, WgpuConfiguration, winit::Painter};
use winit::{
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

thread_local! {
    static CURRENT_EVENT_LOOP: Cell<Option<*const ActiveEventLoop>> = const { Cell::new(None) };
}

/// Run egui's root pass with access to the live event-loop handle from an
/// immediate-viewport callback. The pointer never escapes this synchronous
/// call; see `with_current_event_loop` for the matching safety argument.
pub fn with_event_loop<R>(event_loop: &ActiveEventLoop, f: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CURRENT_EVENT_LOOP.with(|slot| slot.set(None));
        }
    }

    CURRENT_EVENT_LOOP.with(|slot| {
        assert!(slot.get().is_none(), "nested active event-loop scope");
        slot.set(Some(std::ptr::from_ref(event_loop)));
    });
    let _guard = Guard;
    f()
}

fn with_current_event_loop<R>(f: impl FnOnce(&ActiveEventLoop) -> R) -> Option<R> {
    CURRENT_EVENT_LOOP.with(|slot| {
        slot.get().map(|ptr| {
            // SAFETY: `with_event_loop` stores a pointer to its borrowed event
            // loop only for the duration of the synchronous root egui pass.
            // Its guard clears the slot on every exit path, and this callback
            // cannot outlive that pass.
            let event_loop = unsafe { &*ptr };
            f(event_loop)
        })
    })
}

#[derive(Clone, Default)]
pub struct Shared(Rc<std::cell::RefCell<NativeViewports>>);

pub fn floater_viewport_id(floater_id: &str) -> ViewportId {
    ViewportId::from_hash_of(("aurora-floater", floater_id))
}

impl Shared {
    pub fn new(ctx: &Context) -> Self {
        ctx.set_embed_viewports(false);
        let shared = Self(Rc::new(std::cell::RefCell::new(NativeViewports {
            repaint_ctx: Some(ctx.clone()),
            ..NativeViewports::default()
        })));
        let weak = Rc::downgrade(&shared.0);
        Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            if let Some(host) = weak.upgrade() {
                render_immediate(&host, ctx, viewport);
            }
        });
        shared
    }

    /// Route an event addressed to a secondary native window. Returns false
    /// for the root window so the normal application handler can process it.
    pub fn window_event(&self, window_id: WindowId, event: &WindowEvent) -> bool {
        let mut host = self.0.borrow_mut();
        let Some(viewport_id) = host.window_to_viewport.get(&window_id).copied() else {
            return false;
        };
        let repaint_ctx = host.repaint_ctx.clone();
        let NativeViewports {
            viewports,
            window_to_viewport,
            painter,
            ..
        } = &mut *host;
        let Some(viewport) = viewports.get_mut(&viewport_id) else {
            return true;
        };

        if matches!(event, WindowEvent::CloseRequested) {
            viewport.info.events.push(egui::ViewportEvent::Close);
        }
        if let WindowEvent::Resized(size) = event
            && size.width > 0
            && size.height > 0
            && let Some(painter) = painter.as_mut()
        {
            painter.on_window_resized(
                viewport_id,
                NonZeroU32::new(size.width).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(size.height).unwrap_or(NonZeroU32::MIN),
            );
        }
        let response = viewport.egui_winit.on_window_event(&viewport.window, event);
        if response.repaint {
            viewport.window.request_redraw();
            if let Some(ctx) = &repaint_ctx {
                ctx.request_repaint();
            }
        }
        if matches!(event, WindowEvent::Destroyed) {
            window_to_viewport.remove(&window_id);
            viewports.remove(&viewport_id);
            if let Some(painter) = painter.as_mut() {
                let active = viewports.keys().copied().collect();
                painter.gc_viewports(&active);
            }
        }
        true
    }

    /// Retain root texture changes for the shared secondary painter.
    pub fn root_textures(&self, delta: &egui::TexturesDelta) {
        self.0.borrow_mut().root_textures.append(delta.clone());
    }

    /// The egui Context is shared by root and detached viewports, so every pass
    /// must use a texture limit supported by the native painter as well.
    pub fn max_texture_side(&self) -> Option<usize> {
        self.0.borrow().painter.as_ref()?.max_texture_side()
    }

    /// A detached UI window is still part of the interactive viewer. Keep the
    /// main frame loop out of its background FPS cap while one has focus.
    pub fn has_focused_window(&self) -> bool {
        self.0.borrow().viewports.values().any(|viewport| viewport.window.has_focus())
    }

    /// Changes drained by nested viewport passes reach the root before the root
    /// pass's own delta, preserving chronological partial atlas updates.
    pub fn root_texture_deltas(&self, root: egui::TexturesDelta) -> egui::TexturesDelta {
        let nested = std::mem::take(&mut self.0.borrow_mut().secondary_textures);
        if nested.set.contains_key(&egui::TextureId::default()) || root.set.contains_key(&egui::TextureId::default()) {
            log::debug!(
                "multi-screen font atlas deltas: nested [{}], root [{}]",
                font_delta_trace(&nested),
                font_delta_trace(&root)
            );
        }
        merge_root_texture_deltas(nested, root)
    }

    pub fn prune(&self, ctx: &Context, active: &egui::OrderedViewportIdMap<egui::ViewportOutput>) {
        let mut host = self.0.borrow_mut();
        let NativeViewports {
            viewports,
            window_to_viewport,
            painter,
            ..
        } = &mut *host;
        // Immediate viewport passes are nested in the root pass. egui puts
        // their viewport commands on the outermost pass's output, not the
        // nested pass's output; process them here while the live event loop is
        // available (notably StartDrag for frameless floater windows).
        for (id, output) in active {
            if let Some(viewport) = viewports.get_mut(id) {
                let mut commands = output.commands.clone();
                if commands.iter().any(|command| matches!(command, egui::ViewportCommand::StartDrag)) {
                    viewport.pending_start_drag = true;
                    commands.retain(|command| !matches!(command, egui::ViewportCommand::StartDrag));
                }
                if viewport.pending_start_drag && viewport.window.has_focus() {
                    commands.push(egui::ViewportCommand::StartDrag);
                    viewport.pending_start_drag = false;
                }
                egui_winit::process_viewport_commands(ctx, &mut viewport.info, commands, &viewport.window, &mut viewport.actions_requested);
            }
        }
        let dead: Vec<_> = viewports.keys().filter(|id| !active.contains_key(id)).copied().collect();
        for id in dead {
            if let Some(viewport) = viewports.remove(&id) {
                window_to_viewport.remove(&viewport.window.id());
            }
        }
        if let Some(painter) = painter.as_mut() {
            let active = viewports.keys().copied().collect();
            painter.gc_viewports(&active);
        }
    }

    /// Drop viewport windows and their clipboard backends while the winit
    /// event loop and its display connection are still alive.
    pub fn shutdown(&self) {
        let mut host = self.0.borrow_mut();
        if let Some(painter) = host.painter.as_mut() {
            painter.destroy();
        }
        host.viewports.clear();
        host.window_to_viewport.clear();
        host.painter = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floater_viewport_ids_are_stable_and_distinct() {
        let chat = floater_viewport_id("chat");
        assert_eq!(chat, floater_viewport_id("chat"));
        assert_ne!(chat, floater_viewport_id("inventory"));
    }

    #[test]
    fn shared_texture_limit_uses_the_smallest_renderer_limit() {
        assert_eq!(shared_texture_limit(Some(32_768), 8_192), 8_192);
        assert_eq!(shared_texture_limit(Some(4_096), 8_192), 4_096);
        assert_eq!(shared_texture_limit(None, 8_192), 8_192);
    }

    #[test]
    fn root_texture_deltas_keep_nested_pass_order() {
        let image = egui::ColorImage::filled([1, 1], egui::Color32::WHITE);
        let mut nested = egui::TexturesDelta::default();
        nested.push(
            egui::TextureId::Managed(7),
            egui::epaint::ImageDelta::partial([0, 0], image.clone(), egui::TextureOptions::LINEAR),
        );
        let mut root = egui::TexturesDelta::default();
        root.push(
            egui::TextureId::Managed(7),
            egui::epaint::ImageDelta::partial([1, 0], image, egui::TextureOptions::LINEAR),
        );

        let merged = merge_root_texture_deltas(nested, root);

        let image_updates = &merged.set[&egui::TextureId::Managed(7)];
        assert_eq!(image_updates.len(), 2);
        assert_eq!(image_updates[0].pos, Some([0, 0]));
        assert_eq!(image_updates[1].pos, Some([1, 0]));
    }
}

#[derive(Default)]
struct NativeViewports {
    repaint_ctx: Option<Context>,
    painter: Option<Painter>,
    viewports: HashMap<ViewportId, NativeViewport>,
    window_to_viewport: HashMap<WindowId, ViewportId>,
    root_textures: egui::TexturesDelta,
    secondary_textures: egui::TexturesDelta,
    started: Option<Instant>,
}

struct NativeViewport {
    window: Arc<Window>,
    egui_winit: egui_winit::State,
    info: ViewportInfo,
    pending_textures: egui::TexturesDelta,
    actions_requested: Vec<egui_winit::ActionRequested>,
    pending_start_drag: bool,
}

fn render_immediate(host: &Rc<std::cell::RefCell<NativeViewports>>, ctx: &Context, mut viewport: ImmediateViewport<'_>) {
    let pass_started = Instant::now();
    let id = viewport.ids.this;
    let Some((window, raw_input)) = with_current_event_loop(|event_loop| {
        let mut host = host.borrow_mut();
        if host.painter.is_none() {
            host.painter = Some(pollster::block_on(Painter::new(
                ctx.clone(),
                WgpuConfiguration::default(),
                false,
                RendererOptions::default(),
            )));
        }

        if !host.viewports.contains_key(&id) {
            let window = match egui_winit::create_window(ctx, event_loop, &viewport.builder) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    log::error!("Could not create native floater window {id:?}: {error}");
                    return None;
                }
            };
            let painter = host.painter.as_mut()?;
            if let Err(error) = pollster::block_on(painter.set_window(id, Some(Arc::clone(&window)))) {
                log::error!("Could not create renderer surface for native floater {id:?}: {error}");
                return None;
            }
            let mut egui_winit = egui_winit::State::new(
                ctx.clone(),
                id,
                event_loop,
                Some(window.scale_factor() as f32),
                event_loop.system_theme(),
                painter.max_texture_side(),
            );
            egui_winit.set_clipboard_shortcuts(true);
            let mut info = ViewportInfo::default();
            egui_winit::update_viewport_info(&mut info, ctx, &window, true);
            host.window_to_viewport.insert(window.id(), id);
            let pending_textures = std::mem::take(&mut host.root_textures);
            host.viewports.insert(
                id,
                NativeViewport {
                    window,
                    egui_winit,
                    info,
                    pending_textures,
                    actions_requested: Vec::new(),
                    pending_start_drag: false,
                },
            );
        }

        let mut all_viewports = ctx.input(|input| input.raw.viewports.clone());
        for (viewport_id, viewport) in &host.viewports {
            all_viewports.insert(*viewport_id, viewport.info.clone());
        }
        let viewport = host.viewports.get_mut(&id)?;
        let window = Arc::clone(&viewport.window);
        egui_winit::update_viewport_info(&mut viewport.info, ctx, &window, false);
        let mut raw_input = viewport.egui_winit.take_egui_input(&window);
        raw_input.viewport_id = id;
        raw_input.viewports = all_viewports;
        raw_input.viewports.insert(id, viewport.info.clone());
        raw_input.time = Some(host.started.get_or_insert_with(Instant::now).elapsed().as_secs_f64());
        Some((window, raw_input))
    })
    .flatten() else {
        return;
    };

    // The app callback may borrow its live state. Run it without holding the
    // viewport map borrow so nested egui operations cannot deadlock the host.
    let output = ctx.run_ui(raw_input, |ui| (viewport.viewport_ui_cb)(ui));
    let ui_pass_time = pass_started.elapsed();
    let mut host = host.borrow_mut();
    let NativeViewports {
        viewports,
        painter,
        secondary_textures,
        root_textures,
        ..
    } = &mut *host;
    let Some(viewport_state) = viewports.get_mut(&id) else {
        return;
    };
    viewport_state.info.events.clear();
    viewport_state.pending_textures.append(std::mem::take(root_textures));
    viewport_state.pending_textures.append(output.textures_delta.clone());
    secondary_textures.append(output.textures_delta);
    let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    // Native floater windows have no Aurora scene texture to sample. In
    // particular, the viewer's glass callbacks are renderer-specific payloads,
    // not egui-wgpu callbacks; leaving them in the list makes egui-wgpu warn
    // every frame and can leave a detached window looking blank. The adjacent
    // tinted meshes still provide a readable opaque surface in these windows.
    for primitive in &mut primitives {
        if matches!(primitive.primitive, egui::epaint::Primitive::Callback(_)) {
            primitive.primitive = egui::epaint::Primitive::Mesh(egui::Mesh::default());
            primitive.clip_rect = egui::Rect::NOTHING;
        }
    }
    if let Some(painter) = painter.as_mut() {
        if let Err(error) = pollster::block_on(painter.set_window(id, Some(Arc::clone(&window)))) {
            log::error!("Could not prepare native floater surface {id:?} for rendering: {error}");
            return;
        }
        painter.paint_and_update_textures(
            id,
            output.pixels_per_point,
            [0.0, 0.0, 0.0, 0.0],
            &primitives,
            &mut viewport_state.pending_textures,
            vec![],
            &window,
        );
    }
    let total_time = pass_started.elapsed();
    if total_time >= std::time::Duration::from_millis(50) {
        log::warn!(
            "detached viewport {id:?} pass took {:.1} ms (egui {:.1} ms)",
            total_time.as_secs_f64() * 1000.0,
            ui_pass_time.as_secs_f64() * 1000.0
        );
    }
    viewport_state.egui_winit.handle_platform_output(&window, output.platform_output);
    for (child_id, child_output) in output.viewport_output {
        if child_id != id && child_output.class == egui::ViewportClass::Immediate {
            log::warn!("Nested native egui viewport {child_id:?} is not supported by the floater host yet");
        }
    }
}

fn font_delta_trace(delta: &egui::TexturesDelta) -> String {
    let updates = delta
        .set
        .get(&egui::TextureId::default())
        .map(|updates| {
            updates
                .iter()
                .map(|update| {
                    let [width, height] = update.image.size();
                    format!("{}x{}@{:?}", width, height, update.pos)
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "none".to_owned());
    format!("set=[{updates}], free={}", delta.free.contains(&egui::TextureId::default()))
}

fn merge_root_texture_deltas(nested: egui::TexturesDelta, root: egui::TexturesDelta) -> egui::TexturesDelta {
    let mut deltas = nested;
    deltas.append(root);
    deltas
}

pub fn shared_texture_limit(root: Option<usize>, native: usize) -> usize {
    root.map_or(native, |limit| limit.min(native))
}
