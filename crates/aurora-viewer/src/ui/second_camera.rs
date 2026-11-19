//! The second camera's window (Aurora's own; the idea comes from
//! SnowPastel's multi-view-viewer, see `camera/second.rs`): our avatar seen
//! by a camera of its own, edge to edge in the window, while the main
//! camera looks elsewhere. Dragged, the picture turns the camera around
//! the avatar; the wheel zooms, a double click goes back to the framing.
//! The framing's choice floats over the bottom of the picture while it is
//! hovered, so the picture stays clean otherwise.
use super::widgets::{self, Floater};
use crate::camera::second::{Framing, SecondCamera};
use crate::theme::Palette;

pub const WINDOW: &str = "second_camera";

/// Why the window cannot show its picture now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// Out of the world, or our avatar not there yet.
    NoAvatar,
    /// In mouselook the window's lists leave out what is hidden in first
    /// person (worn hair, alpha layers): half an avatar would be shown.
    Mouselook,
    /// An RLVa restriction holds the camera: a second one would get around
    /// it (no side route around a restriction).
    Restricted,
}

/// What the window shows this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Picture {
    /// The picture's size in pixels (`aurora_render::second_view::target_size`).
    pub size: [u32; 2],
    /// Width over height.
    pub aspect: f32,
}

#[derive(Default)]
pub struct SecondCameraUi {
    /// The framing changed: it is saved with the settings.
    pub framing_changed: bool,
    /// `AURORA_DEMO_SECOND_CAMERA` was looked at (app/second_camera.rs).
    pub demo_done: bool,
}

impl SecondCameraUi {
    /// Show the window, when `open`; returns the picture to draw, if any.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        open: &mut bool,
        camera: &mut SecondCamera,
        unavailable: Option<Unavailable>,
    ) -> Option<Picture> {
        if !*open {
            return None;
        }
        let mut reset = false;
        let reset_tip = aurora_i18n::tr!("second-camera-reset");
        let work = crate::ui::layout::work_area(ctx);
        let size = egui::vec2(300.0, 380.0);
        let pos = egui::pos2(work.right() - size.x - 24.0, work.top() + 80.0);
        let shown = Floater::new(WINDOW, aurora_i18n::tr!("second-camera-title"), pos, size)
            .action("restore", reset_tip, &mut reset)
            .flush_body()
            .show(ctx, p, open, |ui| {
                let rect = ui.available_rect_before_wrap();
                let rect = egui::Rect::from_min_size(rect.min, rect.size().max(egui::vec2(160.0, 160.0)));
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                if let Some(reason) = unavailable {
                    let text = match reason {
                        Unavailable::NoAvatar => aurora_i18n::tr!("second-camera-no-avatar"),
                        Unavailable::Mouselook => aurora_i18n::tr!("second-camera-mouselook"),
                        Unavailable::Restricted => aurora_i18n::tr!("second-camera-restricted"),
                    };
                    let inner = rect.shrink(24.0);
                    ui.put(
                        inner,
                        egui::Label::new(egui::RichText::new(text).color(p.muted)).wrap_mode(egui::TextWrapMode::Wrap),
                    );
                    return None;
                }
                aurora_render::second_view::paint(ui, rect);
                if response.dragged_by(egui::PointerButton::Primary) {
                    let delta = response.drag_delta();
                    camera.orbit.drag(delta.x, delta.y);
                }
                if response.hovered() {
                    let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                    if wheel != 0.0 {
                        ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                        camera.orbit.wheel(wheel);
                    }
                }
                if response.double_clicked() {
                    camera.reset();
                }
                let response = response.on_hover_text(aurora_i18n::tr!("second-camera-tip"));
                if response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
                // the framing, floating over the bottom of the picture
                // while the pointer is over it
                let hovered = ui.rect_contains_pointer(rect) && !response.dragged();
                let fade = ui.ctx().animate_bool(egui::Id::unique((WINDOW, "framing")), hovered);
                if fade > 0.01 {
                    self.framing_choice(ui, p, camera, rect, fade);
                }
                Some(Picture {
                    size: aurora_render::second_view::target_size(rect.size(), ui.ctx().pixels_per_point()),
                    aspect: rect.width() / rect.height().max(1.0),
                })
            })
            .flatten();
        if reset {
            camera.reset();
        }
        shown
    }

    fn framing_choice(&mut self, ui: &mut egui::Ui, p: &Palette, camera: &mut SecondCamera, rect: egui::Rect, fade: f32) {
        let labels = [
            aurora_i18n::tr!("second-camera-face"),
            aurora_i18n::tr!("second-camera-body"),
            aurora_i18n::tr!("second-camera-back"),
        ];
        let options: Vec<(&str, bool)> = labels.iter().map(|l| (*l, true)).collect();
        let size = widgets::segmented_size(ui, &options);
        let pill = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - 12.0 - size.y * 0.5), size);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(pill));
        child.set_opacity(fade);
        let mut selected = Framing::ALL.iter().position(|f| *f == camera.framing).unwrap_or(0);
        if widgets::segmented(&mut child, p, &mut selected, &options)
            && let Some(framing) = Framing::ALL.get(selected)
        {
            camera.set_framing(*framing);
            self.framing_changed = true;
        }
    }
}
