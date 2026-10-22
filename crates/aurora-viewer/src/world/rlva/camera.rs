//! Camera modifiers share the RLVa exclusive owner and remain session-local.
use super::{Command, Param, Rlva};
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

/// RlvForceHandler<RLV_BHVR_SETCAM_FOCUS>: a UUID defaults to its object's
/// scale length; a region-local position defaults to zero distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusTarget {
    Object(Uuid),
    Position(Vec3),
}

pub struct FocusRequest {
    pub target: FocusTarget,
    distance: Option<f32>,
    direction: Option<Vec3>,
}

impl FocusRequest {
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(';');
        let target = parts.next()?;
        let target = Uuid::parse_str(target)
            .ok()
            .map(FocusTarget::Object)
            .or_else(|| vector(target).map(FocusTarget::Position))?;
        let distance = parts.next().filter(|v| !v.is_empty()).map(str::parse::<f32>).transpose().ok()?;
        if distance.is_some_and(|v| !v.is_finite()) {
            return None;
        }
        let direction = parts
            .next()
            .filter(|v| !v.is_empty())
            .map(vector)
            .map_or(Some(None), |v| v.map(Some))?;
        parts.next().is_none().then_some(Self {
            target,
            distance,
            direction,
        })
    }

    pub fn camera(&self, focus: Vec3, scale: Vec3, current: Vec3) -> Option<Vec3> {
        let default_distance = if matches!(self.target, FocusTarget::Object(_)) {
            scale.length()
        } else {
            0.0
        };
        let distance = self.distance.unwrap_or(default_distance).max(0.001);
        let direction = self.direction.unwrap_or(current - focus).normalize_or(Vec3::X);
        let camera = focus + direction * distance;
        camera.is_finite().then_some(camera)
    }
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
    pub fn silhouette_distance(&self) -> Option<f32> {
        self.camera_value("setcam_avdist")
            .map(|_| self.camera_limit("setcam_avdist", 0.0, true))
    }
    pub fn camera_texture(&self) -> Option<Uuid> {
        self.camera_value("setcam_textures").and_then(|value| {
            if value.is_empty() {
                Some(aurora_prim::te::DEFAULT_TEXTURE)
            } else {
                Uuid::parse_str(value).ok()
            }
        })
    }
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
        self.camera_number(name, minimum).unwrap_or(default)
    }

    fn camera_number(&self, name: &str, minimum: bool) -> Option<f32> {
        self.rules
            .iter()
            .filter(|(id, _)| self.enabled() && self.camera_owner().is_none_or(|owner| **id == owner))
            .flat_map(|(_, rules)| rules.iter())
            .filter(|r| super::canonical(&r.behaviour) == name)
            .filter_map(|r| {
                let value = if r.behaviour.starts_with("camzoom") && r.option.is_empty() {
                    Some(1.0)
                } else {
                    r.option.parse::<f32>().ok()
                };
                value.map(|v| {
                    if r.behaviour.starts_with("camzoom") {
                        std::f32::consts::FRAC_PI_3 / v
                    } else {
                        v
                    }
                })
            })
            .reduce(if minimum { f32::max } else { f32::min })
    }

    /// RlvReplyCamMinMaxModifierHandler returns an empty reply for an
    /// inactive modifier, even when normal camera limits have defaults.
    pub fn camera_reply(&self, command: &Command, distance: f32, fov: f32) -> Option<String> {
        if !command.option.is_empty() || !matches!(command.param, Param::Reply(_)) {
            return None;
        }
        if command.behaviour == "getcam_textures" {
            return Some(self.camera_texture().map(|id| id.to_string()).unwrap_or_default());
        }
        let value = match command.behaviour.as_str() {
            "getcam_avdist" => Some(distance),
            "getcam_fov" => Some(fov),
            "getcam_fovmin" => self.camera_number("setcam_fovmin", true),
            "getcam_fovmax" => self.camera_number("setcam_fovmax", false),
            "getcam_avdistmin" => self.camera_number("setcam_avdistmin", true),
            "getcam_avdistmax" => self.camera_number("setcam_avdistmax", false),
            _ => return None,
        };
        Some(value.map(|v| format!("{v:.3}")).unwrap_or_default())
    }

    pub fn camera_preset_locked(&self, actor: Uuid) -> bool {
        self.camera_owner().is_some_and(|owner| owner != actor)
            || ["setcam_eyeoffset", "setcam_eyeoffsetscale", "setcam_focusoffset"]
                .iter()
                .any(|name| self.has(name))
    }

    pub fn preserve_camera_settings(&self, requested: &mut CameraSettings, before: &CameraSettings) {
        if self.camera_preset_locked(Uuid::nil()) {
            requested.preset = before.preset;
            requested.camera_offset = before.camera_offset;
            requested.focus_offset = before.focus_offset;
            requested.offset_scale = before.offset_scale;
        }
        if self.camera_owner().is_some() {
            requested.fov = before.fov;
        }
    }

    /// Native camera forces share the preset/FOV guards from RlvActions.
    /// Focus and mode commands still use their own native restrictions.
    pub fn camera_force(&mut self, actor: Uuid, c: &Command) -> bool {
        let vector = matches!(c.behaviour.as_str(), "setcam_eyeoffset" | "setcam_focusoffset");
        let scale = c.behaviour == "setcam_eyeoffsetscale";
        let fov = c.behaviour == "setcam_fov";
        if !vector && !scale && !fov {
            return false;
        }
        if !self.enabled()
            || c.param != Param::Force
            || if fov {
                self.camera_owner().is_some_and(|owner| owner != actor)
            } else {
                self.camera_preset_locked(actor)
            }
        {
            return true;
        }
        if c.option.is_empty() {
            if fov {
                self.camera_values
                    .insert(c.behaviour.clone(), crate::camera::DEFAULT_FOV.to_string());
            } else {
                self.camera_values.remove(&c.behaviour);
            }
        } else if (vector && self::vector(&c.option).is_some()) || (!vector && c.option.parse::<f32>().is_ok_and(f32::is_finite)) {
            self.camera_values.insert(c.behaviour.clone(), c.option.clone());
        }
        true
    }

    /// LLViewerCamera::getMinView/getMaxView bound vertical FOV by window
    /// aspect. With no script FOV command the user's native view is intact.
    pub fn camera_fov_limits(&self, aspect: f32) -> Option<(f32, f32)> {
        let low = self.camera_number("setcam_fovmin", true);
        let high = self.camera_number("setcam_fovmax", false);
        if low.is_none() && high.is_none() && self.camera_value("setcam_fov").is_none() {
            return None;
        }
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
        let native_low = (5.0f32.to_radians() / aspect.min(1.0)).min(175.0f32.to_radians());
        let native_high = (175.0f32.to_radians() / aspect.max(1.0)).max(native_low);
        let low = low.unwrap_or(native_low).clamp(native_low, native_high);
        let high = high.unwrap_or(native_high).clamp(low, native_high);
        Some((low, high))
    }

    pub fn camera_settings(&self, original: &CameraSettings, aspect: f32) -> CameraSettings {
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
        if let Some((low, high)) = self.camera_fov_limits(aspect) {
            s.fov = s.fov.clamp(low, high);
        }
        s
    }
}

impl super::super::World {
    /// LLAgent::needsRenderAvatar/Head and own attachment visibility. HUDs
    /// stay visible; showselfhead uses the native attachment group, not names.
    pub fn rlva_object_visible(&self, index: usize) -> bool {
        if !self.rlva.has("showself") && !self.rlva.has("showselfhead") {
            return true;
        }
        let mut root = index;
        for _ in 0..64 {
            let Some(object) = self.objects.get(root) else {
                return true;
            };
            let Some(parent) = self.objects.parent_of(object).and_then(|p| self.objects.get(p).map(|o| (p, o))) else {
                return true;
            };
            if parent.1.is_avatar() {
                if parent.1.full_id != self.agent_id {
                    return true;
                }
                let point = self.avatar_lib.attach_points.get(&object.attachment_point());
                if point.is_some_and(|p| p.hud) {
                    return true;
                }
                return !self.rlva.has("showself") && !(self.rlva.has("showselfhead") && point.is_some_and(|p| p.group == 2));
            }
            root = parent.0;
        }
        true
    }

    pub fn rlva_body_part_visible(&self, own: bool, part: &str) -> bool {
        !own || (!self.rlva.has("showself")
            && !(self.rlva.has("showselfhead") && matches!(part, "head" | "hair" | "eyelashes" | "eye_left" | "eye_right")))
    }
}
