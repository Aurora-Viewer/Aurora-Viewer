//! LLModelPreview::createPreviewAvatar (indra/newview/llmodelpreview.cpp,
//! LGPL 2.1): an independent avatar plays ANIM_AGENT_STAND. Only its joint
//! palette changes each frame; preview geometry stays on the GPU.

use super::Draft;
use crate::scene::anim::{AnimStreamer, Controller, Rig, SkeletonBase};
use glam::{Mat4, Quat};
use std::time::Instant;

pub struct Player {
    controller: Controller,
    started: Instant,
    pub pose: Option<aurora_render::preview::Pose>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            controller: Controller::default(),
            started: Instant::now(),
            pose: None,
        }
    }
}

impl Player {
    pub fn tick(&mut self, draft: &Draft, rig: &Rig, streamer: &mut AnimStreamer, now: Instant) -> bool {
        if !draft.preview_weights || draft.preview_physics || !draft.scene.models.iter().any(|model| model.skin.is_some()) {
            self.pose = None;
            return false;
        }
        let playing = [crate::world::PlayingAnimation::from_signal(
            crate::own_motion::ANIM_STAND,
            0,
            self.started,
            None,
        )];
        self.controller.sync(&playing, now, |id| streamer.get(id));
        let base = draft.show_overrides.then(|| imported_base(draft, rig));
        let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); rig.len()];
        self.controller.evaluate(rig, now, base.as_ref(), &mut pose);
        finish_pose(rig, &mut pose, draft.options.pelvis, draft.preview_bend);
        self.pose = Some(pose.into());
        true
    }
}

fn imported_base(draft: &Draft, rig: &Rig) -> SkeletonBase {
    let mut base = SkeletonBase {
        local_pos: rig.local_pos.clone(),
        scale: rig.scale.clone(),
    };
    for skin in draft.scene.models.iter().filter_map(|model| model.skin.as_ref()) {
        for (name, bind) in skin.names.iter().zip(&skin.alternate_bind) {
            if let Some(&joint) = rig.names.get(name) {
                base.local_pos[joint] = bind.w_axis.truncate();
            }
        }
    }
    base
}

fn finish_pose(rig: &Rig, pose: &mut [[[f32; 4]; 4]], pelvis: f32, bend: f32) {
    let bend = bend.clamp(0.0, 90.0).to_radians();
    if bend > 0.0 {
        for (name, turn) in [
            ("mElbowLeft", Quat::from_rotation_z(-bend)),
            ("mElbowRight", Quat::from_rotation_z(bend)),
            ("mKneeLeft", Quat::from_rotation_y(bend)),
            ("mKneeRight", Quat::from_rotation_y(bend)),
        ] {
            let Some(&joint) = rig.names.get(name) else { continue };
            let frame = Mat4::from_cols_array_2d(&pose[joint]);
            let change = frame * Mat4::from_quat(turn) * frame.inverse();
            for (index, matrix) in pose.iter_mut().enumerate() {
                let mut ancestor = Some(index);
                while let Some(parent) = ancestor {
                    if parent == joint {
                        *matrix = (change * Mat4::from_cols_array_2d(matrix)).to_cols_array_2d();
                        break;
                    }
                    ancestor = rig.parent[parent];
                }
            }
        }
    }
    for matrix in pose {
        matrix[3][2] += pelvis;
    }
}

#[cfg(test)]
mod tests;
