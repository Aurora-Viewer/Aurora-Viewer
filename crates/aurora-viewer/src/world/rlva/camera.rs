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

/// RlvCommandOptionAdjustHeight: the one-value form is centimetres; the
/// extended form adjusts the current pelvis-to-foot measurement.
pub fn height_offset(text: &str, pelvis_to_foot: f32) -> Option<f32> {
    let parts: Vec<_> = text.split(';').map(str::parse::<f32>).collect::<Result<_, _>>().ok()?;
    if parts.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let offset = match parts.as_slice() {
        [value] => *value / 100.0,
        [height, factor] => (height - pelvis_to_foot) * factor,
        [height, factor, offset] => (height - pelvis_to_foot) * factor + offset,
        _ => return None,
    };
    super::super::hover_height::bounded(offset)
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
        self.rules
            .iter()
            .filter(|(id, _)| self.enabled() && self.camera_owner().is_none_or(|owner| **id == owner))
            .flat_map(|(_, rules)| rules.iter())
            .filter(|r| super::canonical(&r.behaviour) == name)
            .filter_map(|r| {
                r.option.parse::<f32>().ok().map(|v| {
                    if r.behaviour.starts_with("camzoom") {
                        std::f32::consts::FRAC_PI_3 / v
                    } else {
                        v
                    }
                })
            })
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
