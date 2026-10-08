//! Camera preferences, with Firestorm's setting names and defaults
//! (indra/newview/app_settings/settings.xml, app_settings/camera/*.xml and
//! panel_preferences_move.xml).

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Firestorm camera presets (app_settings/camera/*.xml).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CameraPreset {
    /// Rear.xml (SL default).
    #[default]
    Rear,
    /// Front.xml: the camera faces the avatar.
    Front,
    /// Side.xml.
    Side,
    /// TPP.xml ("Shoulder View").
    Shoulder,
    /// Offsets edited by hand.
    Custom,
}

impl CameraPreset {
    pub const ALL: [CameraPreset; 5] = [
        CameraPreset::Rear,
        CameraPreset::Front,
        CameraPreset::Side,
        CameraPreset::Shoulder,
        CameraPreset::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CameraPreset::Rear => "Vue arrière",
            CameraPreset::Front => "Vue de face",
            CameraPreset::Side => "Vue de côté",
            CameraPreset::Shoulder => "Vue épaule",
            CameraPreset::Custom => "Personnalisée",
        }
    }

    /// CameraOffsetRearView, FocusOffsetRearView and CameraOffsetScale of the preset.
    pub fn values(self) -> Option<([f32; 3], [f32; 3], f32)> {
        Some(match self {
            CameraPreset::Rear => ([-3.0, 0.0, 0.75], [1.0, 0.0, 1.0], 1.0),
            CameraPreset::Front => ([2.2, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0),
            CameraPreset::Side => ([-1.0, 0.7, 0.5], [1.5, 0.7, 1.0], 1.0),
            CameraPreset::Shoulder => ([-3.0, -0.4, -0.2], [0.9, -0.7, 0.2], 1.1),
            CameraPreset::Custom => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    pub preset: CameraPreset,
    /// CameraOffsetRearView: camera offset from the avatar (x forward, y left, z up).
    pub camera_offset: [f32; 3],
    /// FocusOffsetRearView: point looked at, relative to the avatar.
    pub focus_offset: [f32; 3],
    /// CameraOffsetScale ("Distance").
    pub offset_scale: f32,
    /// CameraAngle: vertical field of view, radians ("Angle de vue").
    pub fov: f32,
    /// ZoomTime: seconds of a transition between camera modes.
    pub zoom_time: f32,
    /// CameraPositionSmoothing ("Lissage"), × 0.02 s half-life.
    pub smoothing: f32,
    /// DynamicCameraStrength: lag behind the avatar in the air (0 = none, 30 = avatar velocity).
    pub dynamic_strength: f32,
    /// DisableCameraConstraints.
    pub disable_constraints: bool,
    /// FSIgnoreSimulatorCameraConstraints: ignore the simulator's push out of objects.
    pub ignore_sim_constraints: bool,
    /// FSDisableMouseWheelCameraZoom.
    pub disable_wheel_zoom: bool,
    /// ClickOnAvatarKeepsCamera.
    pub click_avatar_keeps_camera: bool,
    /// FSResetCameraOnMovement.
    pub reset_on_movement: bool,
    /// FSResetCameraOnTP.
    pub reset_on_teleport: bool,
    /// ResetViewTurnsAvatar: Escape turns the avatar to the camera direction.
    pub reset_view_turns_avatar: bool,
    /// HoverHeightAffectsCamera.
    pub hover_affects_camera: bool,
    /// TrackFocusObject: the camera follows the moving object it is focused on.
    pub track_focus_object: bool,
    /// EnableMouselook.
    pub enable_mouselook: bool,
    /// FSScrollWheelExitsMouselook.
    pub wheel_exits_mouselook: bool,
}

impl Default for CameraSettings {
    fn default() -> Self {
        let (camera_offset, focus_offset, offset_scale) = CameraPreset::Rear.values().unwrap_or_default();
        CameraSettings {
            preset: CameraPreset::Rear,
            camera_offset,
            focus_offset,
            offset_scale,
            fov: DEFAULT_FOV,
            zoom_time: 0.4,
            smoothing: 1.0,
            dynamic_strength: 2.0,
            disable_constraints: false,
            ignore_sim_constraints: false,
            disable_wheel_zoom: false,
            click_avatar_keeps_camera: false,
            reset_on_movement: true,
            reset_on_teleport: true,
            reset_view_turns_avatar: false,
            hover_affects_camera: false,
            track_focus_object: true,
            enable_mouselook: true,
            wheel_exits_mouselook: true,
        }
    }
}

/// DEFAULT_FIELD_OF_VIEW (60°).
pub const DEFAULT_FOV: f32 = std::f32::consts::FRAC_PI_3;
/// LLViewerCamera min / max view angles of the "Angle de vue" slider.
pub const FOV_RANGE: std::ops::RangeInclusive<f32> = 0.17..=2.97;

impl CameraSettings {
    pub fn camera_offset(&self) -> Vec3 {
        Vec3::from(self.camera_offset)
    }

    pub fn focus_offset(&self) -> Vec3 {
        Vec3::from(self.focus_offset)
    }

    pub fn apply_preset(&mut self, preset: CameraPreset) {
        self.preset = preset;
        if let Some((cam, focus, scale)) = preset.values() {
            self.camera_offset = cam;
            self.focus_offset = focus;
            self.offset_scale = scale;
        }
    }

    /// Settings coming from an edited or older file: finite and in range.
    pub fn sanitized(mut self) -> CameraSettings {
        let d = CameraSettings::default();
        let finite3 = |v: [f32; 3]| v.iter().all(|x| x.is_finite());
        if !finite3(self.camera_offset) || Vec3::from(self.camera_offset).length() < 0.01 {
            self.camera_offset = d.camera_offset;
        }
        if !finite3(self.focus_offset) {
            self.focus_offset = d.focus_offset;
        }
        let fix = |v: f32, lo: f32, hi: f32, def: f32| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.offset_scale = fix(self.offset_scale, 0.5, 3.0, d.offset_scale);
        self.fov = fix(self.fov, *FOV_RANGE.start(), *FOV_RANGE.end(), d.fov);
        self.zoom_time = fix(self.zoom_time, 0.0, 4.0, d.zoom_time);
        self.smoothing = fix(self.smoothing, 0.0, 9.0, d.smoothing);
        self.dynamic_strength = fix(self.dynamic_strength, 0.0, 30.0, d.dynamic_strength);
        self
    }
}
