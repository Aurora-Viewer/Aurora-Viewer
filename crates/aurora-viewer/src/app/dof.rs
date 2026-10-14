//! The depth of field of a frame (LLPipeline::renderDoF's focus point:
//! the camera's focus, the point under the pointer or the crosshair, a
//! clicked or locked point), and its demo (`AURORA_DEMO_DOF`).

use super::App;
use crate::dof::{self, FocusMode};
use aurora_render::{DepthOfField, Renderer};

/// AURORA_DEMO_DOF=<f-number>:<focus distance m>[:debug]: the effect on,
/// at that aperture, focused at that distance (no transition).
fn demo_lens() -> Option<(f32, f32, bool)> {
    static DEMO: std::sync::OnceLock<Option<(f32, f32, bool)>> = std::sync::OnceLock::new();
    *DEMO.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_DOF").ok()?;
        let mut parts = v.split(':');
        let n = parts.next()?.parse().ok()?;
        let d = parts.next()?.parse().ok()?;
        Some((n, d, parts.next() == Some("debug")))
    })
}

impl App {
    /// The lens of this frame, None when the effect is off (or in edit mode
    /// without RenderDepthOfFieldInEditMode). `fov_y` is the vertical field
    /// of view of the image, `height` its height (a photo's), `window_height`
    /// the window's.
    pub(super) fn dof_frame(
        &mut self,
        renderer: &mut Renderer,
        dt: f32,
        fov_y: f32,
        height: u32,
        window_height: u32,
    ) -> Option<DepthOfField> {
        let demo = self.demo.then(demo_lens).flatten();
        if let Some((n, d, debug)) = demo {
            let s = dof::Settings {
                enabled: true,
                f_number: n,
                ..self.settings.dof.clone()
            };
            self.dof_focus.set(d);
            if self.frame_count == 600 {
                // the scene's distances along the view axis, to read the captures
                let (eye, at) = (self.camera.position, self.camera.target - self.camera.position);
                let avatar = dof::distance_along(eye, at, self.world.agent.position);
                let others: Vec<String> = self
                    .world
                    .objects
                    .avatars()
                    .filter(|(_, a)| a.full_id != self.world.agent_id)
                    .map(|(_, a)| format!("{:.1}", dof::distance_along(eye, at, a.position)))
                    .collect();
                log::info!(
                    "dof demo: f/{n}, focus {d} m; own avatar at {avatar:.1} m, other avatars at {} m",
                    others.join(", ")
                );
            }
            return Some(dof::lens(&s, d, fov_y, height, window_height, debug));
        }
        let s = &self.settings.dof;
        if !s.enabled || (self.build.open && !s.in_edit) || !self.in_world() {
            return None;
        }
        let eye = self.camera.position;
        let at = self.camera.target - self.camera.position;
        let (w, h) = renderer.size();
        let pointer = if self.camera.mouselook() {
            // the crosshair
            renderer.hover_pick(w as f32 * 0.5, h as f32 * 0.5)
        } else if self.egui_ctx.is_pointer_over_egui() {
            None
        } else {
            renderer.hover_pick(self.cursor_pos.0, self.cursor_pos.1)
        };
        let point = match s.focus {
            FocusMode::Camera if !self.camera.mouselook() => Some(self.camera.target),
            FocusMode::Camera | FocusMode::Pointer => pointer.or(self.dof_focus.point),
            FocusMode::Click => {
                let clicked = self.egui_ctx.input(|i| i.pointer.primary_pressed()) && !self.egui_ctx.is_pointer_over_egui();
                if clicked && pointer.is_some() {
                    self.dof_focus.point = pointer;
                }
                self.dof_focus.point.or(Some(self.camera.target))
            }
            // the point focused when it was locked
            FocusMode::Locked => self.dof_focus.point.or(pointer).or(Some(self.camera.target)),
        };
        if s.focus != FocusMode::Click && s.focus != FocusMode::Locked {
            // what a lock keeps
            self.dof_focus.point = point;
        }
        let target = point.map_or(f32::NAN, |p| dof::distance_along(eye, at, p));
        let transition = s.transition;
        let focus = self.dof_focus.update(target, dt, transition);
        Some(dof::lens(&self.settings.dof, focus, fov_y, height, window_height, false))
    }
}
