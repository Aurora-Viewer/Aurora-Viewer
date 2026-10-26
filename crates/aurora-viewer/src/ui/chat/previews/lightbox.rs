//! Image enlargement scoped to the Conversations floater (#806). This is an
//! Aurora interaction: LLChatHistory / FSChatHistory retain text-only links.

use super::{Shown, lp};
use crate::theme::Palette;
use crate::ui::widgets;
use egui::{Context, Id, Pos2, Rect, Vec2};
use std::sync::mpsc;

struct View {
    /// One is fitted to the canvas. Zero is never a valid zoom.
    zoom: f32,
    offset: Vec2,
}

impl View {
    fn fitted() -> Self {
        Self {
            zoom: 1.0,
            offset: Vec2::ZERO,
        }
    }

    fn zoom_at(&mut self, factor: f32, anchor: Vec2) {
        let next = (self.zoom * factor).clamp(0.1, 16.0);
        self.offset = anchor + (self.offset - anchor) * (next / self.zoom);
        self.zoom = next;
    }

    fn rect(&self, source: Vec2, canvas: Rect) -> Rect {
        let scale = (canvas.width() / source.x).min(canvas.height() / source.y);
        Rect::from_center_size(canvas.center() + self.offset, source * scale * self.zoom)
    }
}

pub(super) struct Lightbox {
    pub url: String,
    shown: Shown,
    view: View,
    pub pending: Option<mpsc::Receiver<Result<lp::Picture, lp::Failure>>>,
    pub task: Option<tokio::task::JoinHandle<()>>,
    failed: bool,
    pub previous_focus: Option<Id>,
    focus: bool,
}

impl Lightbox {
    pub fn new(ctx: &Context, url: String, shown: Shown) -> Self {
        Self {
            url,
            shown,
            view: View::fitted(),
            pending: None,
            task: None,
            failed: false,
            previous_focus: ctx.memory(|m| m.focused()),
            focus: true,
        }
    }

    pub fn demo(&mut self, mode: &str) {
        if matches!(mode, "zoom" | "pan") {
            self.view.zoom = 2.0;
        }
        if mode == "pan" {
            self.view.offset = Vec2::new(80.0, -40.0);
        }
    }

    fn poll(&mut self, ctx: &Context) {
        if let Some(result) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            self.task = None;
            match result {
                Ok(picture) => self.shown = Shown::new(ctx, "chat-enlarged", picture),
                Err(_) => self.failed = true,
            }
        }
    }

    /// True when dismissed. A child layer keeps it with this floater while
    /// allowing another window to cover both normally.
    pub fn show(&mut self, ctx: &Context, p: &Palette, bounds: Rect) -> bool {
        self.poll(ctx);
        let owner = egui::LayerId::new(egui::Order::Middle, Id::unique("conversations"));
        let id = owner.id.with("image_enlargement");
        let layer = egui::LayerId::new(owner.order, id);
        ctx.set_sublayer(owner, layer);
        egui::Area::new(id)
            .order(owner.order)
            .fixed_pos(bounds.min)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| self.contents(ui, p, bounds, id, true))
            .inner
    }

    /// Independent from the conversation's visibility and position.
    pub fn window(&mut self, ctx: &Context, p: &Palette) -> bool {
        self.poll(ctx);
        let id = Id::unique("link_preview_image");
        let layer = egui::LayerId::new(egui::Order::Middle, id);
        if ctx.top_layer_id() == Some(layer)
            && ctx.memory(|m| m.top_modal_layer()).is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            return true;
        }
        let size = Vec2::new(640.0, 520.0);
        let work = crate::ui::layout::work_area(ctx);
        let mut open = true;
        widgets::Floater::new(
            "link_preview_image",
            aurora_i18n::tr!("ui-chat-preview-image-window"),
            work.center() - size * 0.5,
            size,
        )
        .steady_size()
        .show(ctx, p, &mut open, |ui| {
            let bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            let mut content = ui.new_child(egui::UiBuilder::new().max_rect(bounds));
            self.contents(&mut content, p, bounds, id, false);
        });
        !open
    }

    fn contents(&mut self, ui: &mut egui::Ui, p: &Palette, bounds: Rect, id: Id, dim: bool) -> bool {
        let ctx = ui.ctx().clone();
        let mut close = false;
        ui.set_min_size(bounds.size());
        ui.set_max_size(bounds.size());
        ui.set_clip_rect(bounds);
        let background = dim.then(|| ui.interact(bounds, id.with("background"), egui::Sense::click()));
        // A scrim over the conversation, not another panel surface.
        if dim {
            ui.painter().rect_filled(bounds, 0.0, p.navy.gamma_multiply(0.90));
        }
        let toolbar = ui
            .scope_builder(egui::UiBuilder::new().max_rect(bounds.shrink(8.0)), |ui| {
                widgets::overlay_frame(&ctx, p)
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if widgets::icon_button(
                                ui,
                                p,
                                "zoom-out-filled",
                                aurora_i18n::tr!("ui-worldmap-zoom-out"),
                                self.view.zoom > 0.1,
                            ) {
                                self.view.zoom_at(1.0 / 1.25, Vec2::ZERO);
                            }
                            if widgets::icon_button(
                                ui,
                                p,
                                "zoom-in-filled",
                                aurora_i18n::tr!("ui-worldmap-zoom-in"),
                                self.view.zoom < 16.0,
                            ) {
                                self.view.zoom_at(1.25, Vec2::ZERO);
                            }
                            if widgets::icon_button(ui, p, "arrows-maximize", aurora_i18n::tr!("resource-preview-fit"), true) {
                                self.view = View::fitted();
                            }
                            ui.label(format!("{:.0}%", self.view.zoom * 100.0));
                            if widgets::icon_button(ui, p, "external-link-filled", &super::link_tip(&self.url, false), true) {
                                super::super::open_web_link(&ctx, &self.url);
                            }
                            if widgets::icon_button(ui, p, "copy-filled", super::clipboard::tip(&ctx, &self.shown.copy), true) {
                                super::clipboard::request(&ctx, &self.shown.copy);
                            }
                            if widgets::icon_button(ui, p, "link-filled", &super::link_tip(&self.url, true), true) {
                                ctx.copy_text(self.url.clone());
                            }
                            close = dim && widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true);
                            if self.pending.is_some() {
                                ui.spinner();
                            } else if self.failed {
                                ui.label(egui::RichText::new(aurora_i18n::tr!("ui-chat-preview-enlarge-failed")).color(p.muted));
                            }
                        });
                    })
                    .response
                    .rect
            })
            .inner;
        let hint = aurora_i18n::tr!("ui-chat-preview-enlarge-help");
        let font = egui::TextStyle::Small.resolve(ui.style());
        let galley = ui
            .painter()
            .layout(hint.to_owned(), font, p.muted, (bounds.width() - 24.0).max(1.0));
        let hint_pos = Pos2::new(bounds.center().x - galley.size().x * 0.5, bounds.bottom() - galley.size().y - 8.0);
        let canvas = Rect::from_min_max(
            Pos2::new(bounds.left() + 12.0, toolbar.bottom() + 10.0),
            Pos2::new(bounds.right() - 12.0, (hint_pos.y - 8.0).max(toolbar.bottom() + 11.0)),
        );
        let response = ui.interact(canvas, id.with("canvas"), egui::Sense::click_and_drag());
        if self.focus {
            response.request_focus();
            self.focus = false;
        }
        if response.hovered() {
            let (wheel, pinch) = ui.input_mut(|i| {
                let wheel = i.smooth_scroll_delta.y;
                i.smooth_scroll_delta = Vec2::ZERO;
                (wheel, i.zoom_delta())
            });
            let factor = if pinch != 1.0 { pinch } else { (wheel * 0.005).exp() };
            let anchor = response.hover_pos().unwrap_or(canvas.center()) - canvas.center();
            self.view.zoom_at(factor, anchor);
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            self.view.offset += response.drag_delta();
        }
        if response.double_clicked() {
            self.view = View::fitted();
        }
        let clicked = response.clicked();
        let clicked_at = response.interact_pointer_pos();
        let cursor = if response.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        };
        response.on_hover_cursor(cursor);
        let image = self.view.rect(
            Vec2::new(self.shown.size[0].max(1) as f32, self.shown.size[1].max(1) as f32),
            canvas,
        );
        let (texture, next) = self.shown.frame(ui.input(|i| i.time));
        let mut image_ui = ui.new_child(egui::UiBuilder::new().max_rect(canvas));
        image_ui.set_clip_rect(canvas.intersect(bounds));
        egui::Image::new(texture).paint_at(&image_ui, image);
        if let Some(next) = next {
            ctx.request_repaint_after(next);
        }
        ui.painter().galley(hint_pos, galley, p.muted);
        // Clicking the dim margin closes; a drag never dismisses it.
        close |= background.is_some_and(|background| {
            background.clicked()
                && background
                    .interact_pointer_pos()
                    .is_some_and(|at| !image.contains(at) && !toolbar.contains(at))
        });
        close |= dim && clicked && clicked_at.is_some_and(|at| !image.contains(at));
        close
    }
}

impl Drop for Lightbox {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
#[path = "lightbox_tests.rs"]
mod tests;
