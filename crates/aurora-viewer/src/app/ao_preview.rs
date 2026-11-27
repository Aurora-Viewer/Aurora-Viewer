//! The AO window's preview (ui/ao/preview.rs, world/ao_preview.rs): while
//! its stage is shown, a copy of our avatar plays the previewed entry where
//! we stand, and the second camera (aurora_render::second_view) frames it,
//! our own avatar left out of that picture. The Second Camera window keeps
//! the second view while it is open: the stage then says so.
use super::*;
use crate::camera::second::{Framing, Subject};
use uuid::Uuid;

impl App {
    /// An avatar as posed last frame, as a second camera frames it.
    pub(super) fn framed_subject(&self, id: Uuid, now: Instant) -> Option<Subject> {
        let face = self.scene.face_world(&self.world, &id, now)?;
        let idx = self.world.objects.index_of_uuid(&id)?;
        let (_, rotation, _) = crate::scene::Scene::object_transform(&self.world, idx, now, 0)?;
        let feet = ["mFootLeft", "mFootRight"]
            .iter()
            .filter_map(|joint| self.scene.joint_world(&self.world, &id, joint, now))
            .map(|p| p.z)
            .reduce(f32::min)
            .unwrap_or(face.head.z - 1.6);
        Some(Subject {
            head: face.head,
            eyes: face.eyes.map(|(left, right)| (left + right) * 0.5),
            facing: rotation * Vec3::X,
            feet,
        })
    }

    /// The preview's copy and view for this frame, after the second camera
    /// (`second_camera_frame`): the copy goes while the stage is not shown.
    pub(super) fn ao_preview_frame(&mut self, now: Instant, dt: f32) {
        let stage = self.ao_ui.take_stage().filter(|_| self.panels.ao);
        let Some(stage) = stage else {
            self.world.remove_ao_preview();
            self.ao_preview_camera.cut();
            return;
        };
        let blocked = if self.second_camera_job.is_some() || self.panels.second_camera {
            Some(aurora_i18n::tr!("ao-preview-second-camera"))
        } else {
            match self.second_camera_unavailable() {
                Some(ui::second_camera::Unavailable::Restricted) => Some(aurora_i18n::tr!("second-camera-restricted")),
                Some(ui::second_camera::Unavailable::NoAvatar) => Some(aurora_i18n::tr!("second-camera-no-avatar")),
                // in mouselook the copy is still whole
                Some(ui::second_camera::Unavailable::Mouselook) | None => None,
            }
        };
        if let Some(notice) = blocked {
            self.world.remove_ao_preview();
            self.ao_preview_camera.cut();
            self.ao_ui.set_stage(false, Some(notice));
            return;
        }
        let selected = self
            .settings
            .ao
            .accounts
            .get(&self.world.agent_id)
            .map_or(Uuid::nil(), |a| a.selected);
        let assets = self.ao_ui.preview_assets(self.ao_catalog.selected(selected), &self.ao_runtime);
        let Some(copy) = self.world.update_ao_preview(&assets, now) else {
            self.ao_ui.set_stage(false, None);
            return;
        };
        // the copy appears a frame after it is made: framed once it is posed
        let Some(subject) = self.framed_subject(copy, now) else {
            self.ao_ui.set_stage(false, None);
            return;
        };
        if self.ao_preview_camera.framing != Framing::Body {
            self.ao_preview_camera.set_framing(Framing::Body);
        }
        if stage.drag != egui::Vec2::ZERO {
            self.ao_preview_camera.orbit.drag(stage.drag.x, stage.drag.y);
        }
        let shot = self.ao_preview_camera.follow(&subject, stage.aspect, dt);
        let own = self.world.objects.index_of_uuid(&self.world.agent_id).map(|i| i as u32);
        self.second_camera_job = Some(aurora_render::second_view::SecondViewJob {
            view: shot.view(),
            proj: shot.proj(stage.aspect),
            eye: shot.eye,
            fov_y: shot.fov_y,
            size: stage.size,
            hide_avatar: own,
        });
        self.ao_ui.set_stage(true, None);
    }
}
