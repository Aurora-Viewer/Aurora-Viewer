//! Camera modifiers share the RLVa exclusive owner and remain session-local.
use super::Rlva;
use crate::camera::CameraSettings;
use glam::Vec3;
use uuid::Uuid;

pub fn vector(text: &str) -> Option<Vec3> {
    let mut parts = text.split('/');
    let p = Vec3::new(
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    (parts.next().is_none() && p.is_finite()).then_some(p)
}

impl Rlva {
    pub fn camera_owner(&self) -> Option<Uuid> {
        self.rules_for("setcam", Uuid::nil()).next().map(|(id, _)| id)
    }

    pub fn camera_value(&self, name: &str) -> Option<&str> {
        if !self.enabled() {
            return None;
        }
        self.rules_for(name, Uuid::nil())
            .find(|(id, _)| self.camera_owner().is_none_or(|owner| owner == *id))
            .map(|(_, v)| v)
            .or_else(|| self.camera_values.get(name).map(String::as_str))
    }

    pub fn camera_limit(&self, name: &str, default: f32, minimum: bool) -> f32 {
        self.rules_for(name, Uuid::nil())
            .filter(|(id, _)| self.camera_owner().is_none_or(|owner| *id == owner))
            .filter_map(|(_, v)| v.parse::<f32>().ok())
            .fold(default, if minimum { f32::max } else { f32::min })
    }

    pub fn camera_settings(&self, original: &CameraSettings) -> CameraSettings {
        let mut s = original.clone();
        if let Some(v) = self.camera_value("setcam_eyeoffset").and_then(vector) {
            s.camera_offset = v.to_array();
        }
        if let Some(v) = self.camera_value("setcam_focusoffset").and_then(vector) {
            s.focus_offset = v.to_array();
        }
        if let Some(v) = self.camera_value("setcam_eyeoffsetscale").and_then(|v| v.parse::<f32>().ok()) {
            s.offset_scale = v;
        }
        if let Some(v) = self.camera_value("setcam_fov").and_then(|v| v.parse::<f32>().ok()) {
            s.fov = v;
        }
        let lo = self.camera_limit("setcam_fovmin", 0.1, true);
        let hi = self.camera_limit("setcam_fovmax", std::f32::consts::PI - 0.1, false).max(lo);
        s.fov = s.fov.clamp(lo, hi);
        s
    }
}
