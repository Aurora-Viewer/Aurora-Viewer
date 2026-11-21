//! Camera modifiers share the RLVa exclusive owner and remain session-local.
use super::{Command, Param, Rlva, numeric};
use crate::camera::CameraSettings;
use glam::Vec3;
use uuid::Uuid;

/// RlvCommandOptionHelper::parseOption<LLVector3> (rlvhelper.cpp):
/// `sscanf("%f/%f/%f")`, what follows the third number ignored. The Legacy
/// Viewer also takes `inf` and `nan` and moves its camera or colours with
/// them; a vector that is not finite is refused here instead.
pub fn vector(text: &str) -> Option<Vec3> {
    let p = Vec3::from_array(numeric::scan_floats::<3>(text)?);
    p.is_finite().then_some(p)
}

/// What RLVa holds of the avatar's own view. The camera reads it once a
/// frame and keeps it, so that whatever zooms or changes the mode between
/// two frames (wheel, keys, joystick, a forced mode) works on the values the
/// view is placed with: LLAgentCamera::getCameraOffsetInitial,
/// getFocusOffsetInitial and getCameraOffsetScale under
/// CAMERA_RLV_SETCAM_VIEW (indra/newview/llagentcamera.cpp).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CameraView {
    /// `@setcam_eyeoffset`, `@setcam_focusoffset` and
    /// `@setcam_eyeoffsetscale`: None leaves the user's own value.
    pub eye_offset: Option<Vec3>,
    pub focus_offset: Option<Vec3>,
    pub eye_offset_scale: Option<f32>,
    /// RlvActions::canChangeToMouselook refuses mouselook.
    pub no_mouselook: bool,
    /// RlvActions::isCameraPresetLocked: the user's offsets are not theirs
    /// to change for now.
    pub preset_locked: bool,
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
    /// RlvForceHandler<RLV_BHVR_SETCAM_FOCUS>::onCommand (rlvhandler.cpp):
    /// the option split on every ';' (RlvCommandOptionHelper::
    /// parseStringList, tokens after the third ignored), a key (LLUUID::set)
    /// or a region position, then a distance (`std::stof`) and a direction.
    /// A distance that is not finite is refused: the Legacy Viewer would
    /// place its camera at an infinite or NaN position.
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let mut parts = text.split(';');
        let target = parts.next()?;
        let target = numeric::uuid_set(target)
            .map(FocusTarget::Object)
            .or_else(|| vector(target).map(FocusTarget::Position))?;
        let distance = match parts.next().filter(|v| !v.is_empty()) {
            Some(v) => Some(numeric::finite(numeric::stof(v))?),
            None => None,
        };
        let direction = match parts.next().filter(|v| !v.is_empty()) {
            Some(v) => Some(vector(v)?),
            None => None,
        };
        Some(Self {
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
        // llmax(F_APPROXIMATELY_ZERO, camDistance) (RlvForceHandler<RLV_BHVR_SETCAM_FOCUS>)
        let distance = self.distance.unwrap_or(default_distance).max(1e-5);
        let direction = self.direction.unwrap_or(current - focus).normalize_or(Vec3::X);
        let camera = focus + direction * distance;
        camera.is_finite().then_some(camera)
    }
}

/// RlvCommandOptionAdjustHeight (rlvhelper.cpp): the one-value form is
/// centimetres; the extended form adjusts the current pelvis-to-foot
/// measurement. Each value is read by LLStringUtil::convertToF32.
pub fn height_offset(text: &str, pelvis_to_foot: f32) -> Option<f32> {
    let parts: Vec<_> = text.split(';').map(numeric::convert_to_f32).collect::<Option<_>>()?;
    let offset = match parts.as_slice() {
        [value] => *value / 100.0,
        [height, factor] => (height - pelvis_to_foot) * factor,
        [height, factor, offset] => (height - pelvis_to_foot) * factor + offset,
        _ => return None,
    };
    super::super::hover_height::bounded(offset)
}

/// The modifiers created without a comparator (addModifier, rlvhelper.cpp).
const ORDERED_MODIFIERS: [&str; 4] = ["setcam_eyeoffset", "setcam_eyeoffsetscale", "setcam_focusoffset", "setcam_textures"];

impl Rlva {
    /// LLVOAvatar::isRlvSilhouette (indra/newview/llvoavatar.cpp): on while
    /// any object holds `@setcam_avdist`; under another object's `@setcam`
    /// the value is the modifier's default, 0 (RlvBehaviourModifier::
    /// hasValue keeps the primary object's values only).
    pub fn silhouette_distance(&self) -> Option<f32> {
        self.camera_held("setcam_avdist")
            .then(|| self.camera_limit("setcam_avdist", 0.0, true))
    }

    /// Some object holds the behaviour, whoever holds `@setcam`
    /// (RlvHandler::hasBehaviour).
    pub(super) fn camera_held(&self, name: &str) -> bool {
        self.enabled() && self.rules.values().flatten().any(|r| super::canonical(&r.behaviour) == name)
    }

    /// RlvBehaviourModifier::addValue: the modifiers with no comparator keep
    /// their values in the order added (eye offset, its scale, focus offset,
    /// forced texture; addModifier, indra/newview/rlvhelper.cpp).
    pub(super) fn note_modifier_added(&mut self, source: Uuid, rule: &super::Rule) {
        if ORDERED_MODIFIERS.contains(&super::canonical(&rule.behaviour)) {
            self.modifier_seq += 1;
            self.modifier_order
                .insert((source, rule.behaviour.clone(), rule.option.clone()), self.modifier_seq);
        }
    }
    pub fn camera_texture(&self) -> Option<Uuid> {
        self.camera_value("setcam_textures").and_then(|value| {
            if value.is_empty() {
                // RlvBehaviourModifier's default is IMG_DEFAULT (addModifier
                // RLV_BHVR_SETCAM_TEXTURES, rlvhelper.cpp), the flat grey the
                // Legacy Viewer paints locally. The plywood texture is a
                // different id.
                Some(aurora_prim::te::IMG_DEFAULT)
            } else {
                // A key modifier: LLUUID::parseUUID.
                numeric::parse_uuid(value)
            }
        })
    }
    pub fn camera_owner(&self) -> Option<Uuid> {
        self.rules_for("setcam", Uuid::nil()).next().map(|(id, _)| id)
    }

    /// Something holds the camera (@setcam) or bounds it (where it may be,
    /// what it looks at, how wide).
    pub fn camera_restricted(&self) -> bool {
        self.camera_owner().is_some()
            || [
                "setcam_unlock",
                "setcam_avdistmin",
                "setcam_avdistmax",
                "setcam_origindistmin",
                "setcam_origindistmax",
                "setcam_eyeoffset",
                "setcam_focusoffset",
                "setcam_mouselook",
                "setcam_fovmin",
                "setcam_fovmax",
            ]
            .iter()
            .any(|name| self.has(name))
    }

    /// The view is the object's (@setcam) or its focus is pinned on the
    /// avatar (@setcam_unlock: LLAgentCamera::updateCamera puts the focus
    /// back on the avatar every frame, indra/newview/llagentcamera.cpp).
    /// Aurora's own views that place the eye and the focus themselves (the
    /// photo free camera, the Director's track and free switch) give way
    /// then; under the other camera rules they stay, bounded as the avatar's
    /// view is (the distance and field of view limits apply to them in
    /// `Camera::update_moving`), as the Legacy Viewer's Alt-camera is. The
    /// joystick flycam, which RLVa never reaches, stays exempt.
    pub fn camera_view_held(&self) -> bool {
        self.camera_owner().is_some() || self.has("setcam_unlock")
    }

    /// The second camera (Aurora's own, `camera/second.rs`) pauses while
    /// RLVa holds the avatar's view in any way: the camera itself
    /// (`camera_restricted`), its distances and zoom, what the view may show
    /// (spheres, overlays, textures) and whether our avatar may be seen.
    /// A second camera would see around each of them; the Legacy Viewer has
    /// no such view to compare with, so this is the strictest rule.
    pub fn second_camera_blocked(&self) -> bool {
        self.camera_restricted()
            || [
                "setcam",
                "setcam_avdist",
                "setcam_textures",
                "setcam_eyeoffsetscale",
                "camavdist",
                "camtextures",
                "camdistmin",
                "camdistmax",
                "camzoommin",
                "camzoommax",
                "camunlock",
                "setsphere",
                "setoverlay",
                "showself",
                "showselfhead",
            ]
            .iter()
            .any(|name| self.has(name))
    }

    pub fn camera_value(&self, name: &str) -> Option<&str> {
        if !self.enabled() {
            return None;
        }
        self.camera_rule(name)
            .map(|(_, v)| v)
            .or_else(|| self.camera_values.get(name).map(String::as_str))
    }

    /// The rule behind `camera_value`, with the object that issued it: the
    /// `@setcam` holder's only, and of the modifiers with no comparator the
    /// first value added (RlvBehaviourModifierComp keeps the order of
    /// addValue, the primary object's values first).
    pub(super) fn camera_rule(&self, name: &str) -> Option<(Uuid, &str)> {
        let owner = self.camera_owner();
        let name = super::canonical(name);
        let candidates = self
            .rules
            .iter()
            .filter(|(id, _)| self.enabled() && owner.is_none_or(|owner| owner == **id))
            .flat_map(|(id, rs)| rs.iter().map(move |r| (*id, r)))
            .filter(move |(_, r)| super::canonical(&r.behaviour) == name);
        if ORDERED_MODIFIERS.contains(&name) {
            candidates
                .min_by_key(|(id, r)| {
                    self.modifier_order
                        .get(&(*id, r.behaviour.clone(), r.option.clone()))
                        .copied()
                        .unwrap_or(u64::MAX)
                })
                .map(|(id, r)| (id, r.option.as_str()))
        } else {
            candidates.map(|(id, r)| (id, r.option.as_str())).next()
        }
    }

    pub fn camera_limit(&self, name: &str, default: f32, minimum: bool) -> f32 {
        self.camera_number(name, minimum).unwrap_or(default)
    }

    /// RlvActions::getCameraAvatarDistanceLimits and
    /// getCameraOriginDistanceLimits (indra/newview/rlvactions.cpp): the two
    /// limits once one of them is set, the other at its default.
    pub fn camera_distance_limits(&self, low: &str, high: &str) -> Option<(f32, f32)> {
        let (min, max) = (self.camera_number(low, true), self.camera_number(high, false));
        (min.is_some() || max.is_some()).then(|| (min.unwrap_or(0.0), max.unwrap_or(f32::INFINITY)))
    }

    fn camera_number(&self, name: &str, minimum: bool) -> Option<f32> {
        self.camera_winner(name, minimum).map(|(value, _)| value)
    }

    /// The limit that applies and the object that set it.
    pub(super) fn camera_winner(&self, name: &str, largest: bool) -> Option<(f32, Uuid)> {
        let values = self
            .rules
            .iter()
            .filter(|(id, _)| self.enabled() && self.camera_owner().is_none_or(|owner| **id == owner))
            .flat_map(|(id, rules)| rules.iter().map(move |r| (*id, r)))
            .filter(|(_, r)| super::canonical(&r.behaviour) == name)
            .filter_map(|(id, r)| {
                // RlvBehaviourCamZoomMinMaxHandler::onCommand stores
                // DEFAULT_FIELD_OF_VIEW / multiplier; the others the
                // `std::stof` value. A limit that is not finite (a zero
                // multiplier, `inf`, `nan`) is left out: the Legacy Viewer
                // would clamp its view with it.
                let value = if r.behaviour.starts_with("camzoom") && r.option.is_empty() {
                    Some(1.0)
                } else {
                    numeric::stof(&r.option)
                };
                value
                    .map(|v| {
                        if r.behaviour.starts_with("camzoom") {
                            std::f32::consts::FRAC_PI_3 / v
                        } else {
                            v
                        }
                    })
                    .filter(|v| v.is_finite())
                    .map(|v| (v, id))
            });
        super::modifiers::best(values, largest)
    }

    /// RlvReplyCamMinMaxModifierHandler returns an empty reply for an
    /// inactive modifier, even when normal camera limits have defaults.
    /// An option these queries do not take fails them, and a failed query
    /// still answers, with nothing, so that the script does not wait
    /// (RlvCommandHandlerBaseImpl<RLV_TYPE_REPLY>::processCommand,
    /// rlvhandler.cpp).
    pub fn camera_reply(&self, command: &Command, distance: f32, fov: f32) -> Option<String> {
        if !matches!(command.param, Param::Reply(_)) {
            return None;
        }
        if !command.option.is_empty() {
            return matches!(
                command.behaviour.as_str(),
                "getcam_avdist"
                    | "getcam_avdistmin"
                    | "getcam_avdistmax"
                    | "getcam_fov"
                    | "getcam_fovmin"
                    | "getcam_fovmax"
                    | "getcam_textures"
            )
            .then(String::new);
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

    /// RlvActions::canChangeToMouselook (indra/newview/rlvactions.cpp): not
    /// under `@setcam_mouselook`, nor while a minimum distance to the avatar
    /// above zero is set (`@setcam_avdistmin`, `@camdistmin`). Under an
    /// exclusive `@setcam` only its owner's restrictions count.
    pub fn can_mouselook(&self) -> bool {
        // (`== 0` as there: a negative minimum refuses it too)
        self.camera_value("setcam_mouselook").is_none() && self.camera_limit("setcam_avdistmin", 0.0, true) == 0.0
    }

    pub fn camera_view(&self) -> CameraView {
        if !self.enabled() {
            return CameraView::default();
        }
        CameraView {
            eye_offset: self.camera_value("setcam_eyeoffset").and_then(vector),
            focus_offset: self.camera_value("setcam_focusoffset").and_then(vector),
            eye_offset_scale: self
                .camera_value("setcam_eyeoffsetscale")
                .and_then(|v| numeric::finite(numeric::stof(v))),
            no_mouselook: !self.can_mouselook(),
            preset_locked: self.camera_preset_locked(Uuid::nil()),
        }
    }

    /// A change of the user's camera settings (Preferences, Debug Settings,
    /// the views): the view's offsets stay while RLVa locks them
    /// (RlvActions::isCameraPresetLocked). The field of view stays the
    /// user's, bounded by the limits only: the Legacy Viewer's zoom and its
    /// CameraAngle setting have no RLVa check (LLZoomer,
    /// indra/newview/llviewermenu.cpp; LLViewerCamera::setDefaultFOV).
    pub fn preserve_camera_settings(&mut self, requested: &mut CameraSettings, before: &CameraSettings) {
        if self.camera_preset_locked(Uuid::nil()) {
            requested.preset = before.preset;
            requested.camera_offset = before.camera_offset;
            requested.focus_offset = before.focus_offset;
            requested.offset_scale = before.offset_scale;
        } else if requested.preset != before.preset
            || requested.camera_offset != before.camera_offset
            || requested.focus_offset != before.focus_offset
            || requested.offset_scale != before.offset_scale
        {
            self.user_chose_camera_view();
        }
        if requested.fov != before.fov {
            self.user_chose_fov();
        }
    }

    /// The user picks a view while nothing locks it: RLVa's view
    /// (CAMERA_RLV_SETCAM_VIEW) is left, and the offsets a force put there
    /// no longer apply (LLAgentCamera::switchCameraPreset,
    /// RlvForceCamEyeFocusOffsetHandler, rlvhandler.cpp).
    pub fn user_chose_camera_view(&mut self) {
        if self.camera_preset_locked(Uuid::nil()) {
            return;
        }
        for name in ["setcam_eyeoffset", "setcam_eyeoffsetscale", "setcam_focusoffset"] {
            self.camera_values.remove(name);
        }
    }

    /// The user's own lens change replaces a forced field of view:
    /// `@setcam_fov=force` sets the lens once (setDefaultFOV), it does not
    /// hold it.
    pub fn user_chose_fov(&mut self) {
        self.camera_values.remove("setcam_fov");
    }

    /// Once a frame: true when `@setcam_unlock` just came
    /// (RlvBehaviourToggleHandler<RLV_BHVR_SETCAM_UNLOCK> runs
    /// handle_reset_view), and when RLVa's view just took over
    /// (RlvHandler::setCameraOverride → switchCameraPreset: the zoom back
    /// to 1, the focus on the avatar).
    pub fn camera_toggles(&mut self) -> (bool, bool) {
        let unlock = self.camera_value("setcam_unlock").is_some();
        let view = self.camera_preset_locked(Uuid::nil())
            || ["setcam_eyeoffset", "setcam_eyeoffsetscale", "setcam_focusoffset"]
                .iter()
                .any(|name| self.camera_values.contains_key(*name));
        let (was_unlock, was_view) = std::mem::replace(&mut self.camera_seen, (unlock, view));
        (unlock && !was_unlock, view && !was_view)
    }

    /// RlvBehaviourSetCamFovHandler (rlvhandler.cpp): the user's field of
    /// view is kept when the first fov limit comes, and given back when the
    /// last goes. A field of view RLVa forced was not persisted there, and
    /// DEFAULT_FIELD_OF_VIEW is kept instead. True when `fov` changed.
    pub fn track_fov_limits(&mut self, fov: &mut f32) -> bool {
        let held = self.camera_held("setcam_fovmin") || self.camera_held("setcam_fovmax");
        match (held, self.fov_before_limits) {
            (true, None) => {
                self.fov_before_limits = Some(if self.camera_values.contains_key("setcam_fov") {
                    crate::camera::DEFAULT_FOV
                } else {
                    *fov
                });
                false
            }
            (false, Some(before)) => {
                self.fov_before_limits = None;
                let changed = *fov != before;
                *fov = before;
                changed
            }
            _ => false,
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
        } else if (vector && self::vector(&c.option).is_some()) || (!vector && numeric::finite(numeric::stof(&c.option)).is_some()) {
            self.camera_values.insert(c.behaviour.clone(), c.option.clone());
        }
        true
    }

    /// LLViewerCamera::getMinView/getMaxView bound vertical FOV by window
    /// aspect. With no script FOV command the user's native view is intact.
    /// RlvActions::getCameraFOVLimits: a limit held by an object other than
    /// the `@setcam` holder counts, at the modifier's default
    /// (DEFAULT_FIELD_OF_VIEW), the holder's value missing.
    pub fn camera_fov_limits(&self, aspect: f32) -> Option<(f32, f32)> {
        let held = |name| self.camera_held(name).then_some(crate::camera::DEFAULT_FOV);
        let low = self.camera_number("setcam_fovmin", true).or_else(|| held("setcam_fovmin"));
        let high = self.camera_number("setcam_fovmax", false).or_else(|| held("setcam_fovmax"));
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
        let view = self.camera_view();
        if let Some(v) = view.eye_offset {
            s.camera_offset = v.to_array();
        }
        if let Some(v) = view.focus_offset {
            s.focus_offset = v.to_array();
        }
        if let Some(v) = view.eye_offset_scale {
            s.offset_scale = v;
        }
        if let Some(v) = self.camera_value("setcam_fov").and_then(|v| numeric::finite(numeric::stof(v))) {
            s.fov = v;
        }
        if let Some((low, high)) = self.camera_fov_limits(aspect) {
            s.fov = s.fov.clamp(low, high);
        }
        s
    }
}

impl super::super::World {
    /// LLAgentCamera::changeCameraToCustomizeAvatar refuses appearance mode
    /// only when the avatar may not stand (RlvActions::canStand,
    /// indra/newview/rlvactions.cpp), as it stands the avatar up; no camera
    /// rule refuses it and none bounds that view.
    pub fn rlva_studio_refused(&self) -> bool {
        self.rlva.enabled() && self.rlva.has("unsit") && self.agent.is_sitting()
    }

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
