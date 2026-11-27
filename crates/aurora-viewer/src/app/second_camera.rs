//! The second camera (camera/second.rs, ui/second_camera.rs,
//! aurora_render::second_view): its window, and the view it asks the
//! renderer for each frame. Aurora's own; the idea comes from SnowPastel's
//! multi-view-viewer.
use super::*;
use crate::camera::second::Subject;
use crate::ui::second_camera::Unavailable;

impl App {
    /// Why the second camera cannot show our avatar now.
    pub(super) fn second_camera_unavailable(&self) -> Option<Unavailable> {
        if !self.in_world() || self.scene_paused() {
            Some(Unavailable::NoAvatar)
        } else if self.world.rlva.second_camera_blocked() {
            Some(Unavailable::Restricted)
        } else if self.camera.first_person_view() {
            Some(Unavailable::Mouselook)
        } else {
            None
        }
    }

    /// Our avatar as posed last frame, as the second camera frames it.
    fn second_camera_subject(&self, now: Instant) -> Option<Subject> {
        self.framed_subject(self.world.agent_id, now)
    }

    /// The second camera's view of this frame, once a frame before the
    /// culling (which takes its frustum in, `CullView::second`): None while
    /// its window is closed or cannot show our avatar.
    pub(super) fn second_camera_frame(&mut self, now: Instant, dt: f32) {
        self.second_camera_job = None;
        let picture = self.second_camera_picture.filter(|_| self.panels.second_camera);
        let subject = match (picture, self.second_camera_unavailable()) {
            (Some(_), None) => self.second_camera_subject(now),
            _ => None,
        };
        let (Some(picture), Some(subject)) = (picture, subject) else {
            // shown again, the camera starts where its shot is
            self.second_camera.cut();
            return;
        };
        let shot = self.second_camera.follow(&subject, picture.aspect, dt);
        self.second_camera_job = Some(aurora_render::second_view::SecondViewJob {
            view: shot.view(),
            proj: shot.proj(picture.aspect),
            eye: shot.eye,
            fov_y: shot.fov_y,
            size: picture.size,
            hide_avatar: None,
        });
    }

    /// The frustum of this frame's second view, for the CPU culling.
    pub(super) fn second_camera_planes(&self) -> Option<[glam::Vec4; 6]> {
        self.second_camera_job
            .map(|job| crate::scene::CullView::frustum_planes(job.proj * job.view))
    }

    /// AURORA_DEMO_SECOND_CAMERA=face|body|back: at frame 300 of the demo,
    /// the window opens on that framing (the main camera stays behind the
    /// avatar: the picture shows what it does not).
    fn second_camera_demo(&mut self) {
        if !self.demo || self.second_camera_ui.demo_done || self.frame_count < 300 || !self.in_world() {
            return;
        }
        self.second_camera_ui.demo_done = true;
        let Ok(mode) = std::env::var("AURORA_DEMO_SECOND_CAMERA") else {
            return;
        };
        use crate::camera::second::Framing;
        let framing = match mode.as_str() {
            "body" => Framing::Body,
            "back" => Framing::Back,
            _ => Framing::Face,
        };
        self.settings.second_camera_framing = framing;
        self.second_camera.set_framing(framing);
        ui::widgets::open_floater(&self.egui_ctx, ui::second_camera::WINDOW, &mut self.panels.second_camera);
        log::info!("second camera demo: {framing:?}");
    }

    pub(super) fn second_camera_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        self.second_camera_demo();
        if !self.panels.second_camera {
            // the framing saved, for the next opening
            self.second_camera.framing = self.settings.second_camera_framing;
            self.second_camera_picture = None;
            return;
        }
        let unavailable = self.second_camera_unavailable();
        self.second_camera_picture =
            self.second_camera_ui
                .show(ctx, p, &mut self.panels.second_camera, &mut self.second_camera, unavailable);
        if std::mem::take(&mut self.second_camera_ui.framing_changed) {
            self.settings.second_camera_framing = self.second_camera.framing;
            self.settings.save();
        }
    }
}
