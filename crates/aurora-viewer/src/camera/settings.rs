//! Camera preferences, with Firestorm's setting names and defaults
//! (indra/newview/app_settings/settings.xml, app_settings/camera/*.xml and
//! panel_preferences_move.xml).

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod first_person_tests {
    use super::*;

    #[test]
    fn first_person_options_default_off_and_keep_a_saved_choice() {
        let migrated: CameraSettings = serde_json::from_str("{}").unwrap();
        assert!(!migrated.show_avatar_in_mouselook);
        assert!(!migrated.mouselook_follow_eyes);
        assert_eq!(migrated.third_person_follow_joint, None);
        assert_eq!(migrated.third_person_follow_intensity, 100);
        let chosen = CameraSettings {
            show_avatar_in_mouselook: true,
            mouselook_follow_eyes: true,
            third_person_follow_joint: Some(CameraFollowJoint::Head),
            third_person_follow_intensity: 75,
            ..migrated
        };
        let saved = serde_json::to_string(&chosen).unwrap();
        let loaded: CameraSettings = serde_json::from_str(&saved).unwrap();
        assert!(loaded.show_avatar_in_mouselook);
        assert!(loaded.mouselook_follow_eyes);
        assert_eq!(loaded.third_person_follow_joint, Some(CameraFollowJoint::Head));
        assert_eq!(loaded.third_person_follow_intensity, 75);
    }
}

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
            CameraPreset::Rear => aurora_i18n::tr!("camera-settings-rear-view"),
            CameraPreset::Front => aurora_i18n::tr!("camera-settings-front-view"),
            CameraPreset::Side => aurora_i18n::tr!("camera-settings-side-view"),
            CameraPreset::Shoulder => aurora_i18n::tr!("camera-settings-shoulder-view"),
            CameraPreset::Custom => aurora_i18n::tr!("camera-settings-custom"),
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

/// Skeleton joint available as the anchor for the third-person bone camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraFollowJoint {
    Head,
    Neck,
    Chest,
    Torso,
    Pelvis,
    LeftCollar,
    RightCollar,
    LeftShoulder,
    RightShoulder,
    LeftElbow,
    RightElbow,
    LeftWrist,
    RightWrist,
    LeftHip,
    RightHip,
    LeftKnee,
    RightKnee,
    LeftAnkle,
    RightAnkle,
    LeftFoot,
    RightFoot,
    LeftToe,
    RightToe,
}

impl CameraFollowJoint {
    pub const ALL: [Self; 23] = [
        Self::Head,
        Self::Neck,
        Self::Chest,
        Self::Torso,
        Self::Pelvis,
        Self::LeftCollar,
        Self::RightCollar,
        Self::LeftShoulder,
        Self::RightShoulder,
        Self::LeftElbow,
        Self::RightElbow,
        Self::LeftWrist,
        Self::RightWrist,
        Self::LeftHip,
        Self::RightHip,
        Self::LeftKnee,
        Self::RightKnee,
        Self::LeftAnkle,
        Self::RightAnkle,
        Self::LeftFoot,
        Self::RightFoot,
        Self::LeftToe,
        Self::RightToe,
    ];

    pub const fn rig_name(self) -> &'static str {
        match self {
            Self::Head => "mHead",
            Self::Neck => "mNeck",
            Self::Chest => "mChest",
            Self::Torso => "mTorso",
            Self::Pelvis => "mPelvis",
            Self::LeftCollar => "mCollarLeft",
            Self::RightCollar => "mCollarRight",
            Self::LeftShoulder => "mShoulderLeft",
            Self::RightShoulder => "mShoulderRight",
            Self::LeftElbow => "mElbowLeft",
            Self::RightElbow => "mElbowRight",
            Self::LeftWrist => "mWristLeft",
            Self::RightWrist => "mWristRight",
            Self::LeftHip => "mHipLeft",
            Self::RightHip => "mHipRight",
            Self::LeftKnee => "mKneeLeft",
            Self::RightKnee => "mKneeRight",
            Self::LeftAnkle => "mAnkleLeft",
            Self::RightAnkle => "mAnkleRight",
            Self::LeftFoot => "mFootLeft",
            Self::RightFoot => "mFootRight",
            Self::LeftToe => "mToeLeft",
            Self::RightToe => "mToeRight",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Head => aurora_i18n::tr!("ui-options-camera-joint-head"),
            Self::Neck => aurora_i18n::tr!("ui-options-camera-joint-neck"),
            Self::Chest => aurora_i18n::tr!("ui-options-camera-joint-chest"),
            Self::Torso => aurora_i18n::tr!("ui-options-camera-joint-torso"),
            Self::Pelvis => aurora_i18n::tr!("ui-options-camera-joint-pelvis"),
            Self::LeftCollar => aurora_i18n::tr!("ui-options-camera-joint-left-collar"),
            Self::RightCollar => aurora_i18n::tr!("ui-options-camera-joint-right-collar"),
            Self::LeftShoulder => aurora_i18n::tr!("ui-options-camera-joint-left-shoulder"),
            Self::RightShoulder => aurora_i18n::tr!("ui-options-camera-joint-right-shoulder"),
            Self::LeftElbow => aurora_i18n::tr!("ui-options-camera-joint-left-elbow"),
            Self::RightElbow => aurora_i18n::tr!("ui-options-camera-joint-right-elbow"),
            Self::LeftWrist => aurora_i18n::tr!("ui-options-camera-joint-left-wrist"),
            Self::RightWrist => aurora_i18n::tr!("ui-options-camera-joint-right-wrist"),
            Self::LeftHip => aurora_i18n::tr!("ui-options-camera-joint-left-hip"),
            Self::RightHip => aurora_i18n::tr!("ui-options-camera-joint-right-hip"),
            Self::LeftKnee => aurora_i18n::tr!("ui-options-camera-joint-left-knee"),
            Self::RightKnee => aurora_i18n::tr!("ui-options-camera-joint-right-knee"),
            Self::LeftAnkle => aurora_i18n::tr!("ui-options-camera-joint-left-ankle"),
            Self::RightAnkle => aurora_i18n::tr!("ui-options-camera-joint-right-ankle"),
            Self::LeftFoot => aurora_i18n::tr!("ui-options-camera-joint-left-foot"),
            Self::RightFoot => aurora_i18n::tr!("ui-options-camera-joint-right-foot"),
            Self::LeftToe => aurora_i18n::tr!("ui-options-camera-joint-left-toe"),
            Self::RightToe => aurora_i18n::tr!("ui-options-camera-joint-right-toe"),
        }
    }

    pub fn choices() -> [(Option<Self>, &'static str); 24] {
        let mut choices: [(Option<Self>, &'static str); 24] = [(None, aurora_i18n::tr!("common-disabled")); 24];
        for (choice, joint) in choices[1..].iter_mut().zip(Self::ALL) {
            *choice = (Some(joint), joint.label());
        }
        choices
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
    /// CameraAngle: vertical field of view, radians ("Viewing angle").
    pub fov: f32,
    /// ZoomTime: seconds of a transition between camera modes.
    pub zoom_time: f32,
    /// CameraPositionSmoothing ("Smoothing"), × 0.02 s half-life.
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
    /// FirstPersonAvatarVisible (Legacy Viewer), off by default.
    pub show_avatar_in_mouselook: bool,
    /// Optional animated eye position; mouse aim never follows head rotation.
    pub mouselook_follow_eyes: bool,
    /// Move the third-person avatar camera and focus with this animated joint.
    pub third_person_follow_joint: Option<CameraFollowJoint>,
    /// Strength of the selected joint's translation, in percent (100 = full).
    pub third_person_follow_intensity: u8,
    /// FSScrollWheelExitsMouselook.
    pub wheel_exits_mouselook: bool,
    /// The angle of the right-button zoom in mouselook, remembered from one
    /// hold to the next (`_NACL_MLFovValues` y; mouselook_zoom.rs); none
    /// until the wheel set one, the hold then keeping the normal angle.
    pub mouselook_zoom_fov: Option<f32>,
    /// FSEnableRightclickMenuInMouselook: Alt + right click opens the
    /// context menu in mouselook (otherwise the right button only zooms).
    pub mouselook_alt_right_menu: bool,
    /// FSShowMouselookInstructions: "Press Esc to return to world view".
    pub show_mouselook_instructions: bool,
    /// FSShowInterfaceInMouselook (FIRE-8868, off): the bars stay in
    /// mouselook; otherwise they go (LLAgent::changeCameraToMouselook) and
    /// the chat bar only shows with text in it.
    pub show_interface_in_mouselook: bool,
    /// Where the headset view starts when VR is entered (Aurora-specific).
    pub vr_view: VrView,
}

/// Where the headset view starts on VR entry (Preferences › Camera › VR).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VrView {
    /// At the avatar's eyes, as in mouselook: the head and what is worn on
    /// it are hidden the same way.
    #[default]
    FirstPerson,
    /// Where the desktop camera was when VR was entered.
    DesktopCamera,
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
            // both on, unlike the Legacy Viewer (human choice of 2026-10-16):
            // the camera goes where it is put, walls and objects included
            disable_constraints: true,
            ignore_sim_constraints: true,
            disable_wheel_zoom: false,
            click_avatar_keeps_camera: false,
            reset_on_movement: true,
            reset_on_teleport: true,
            reset_view_turns_avatar: false,
            hover_affects_camera: false,
            track_focus_object: true,
            enable_mouselook: true,
            show_avatar_in_mouselook: false,
            mouselook_follow_eyes: false,
            third_person_follow_joint: None,
            third_person_follow_intensity: 100,
            wheel_exits_mouselook: true,
            // no zoom until the wheel is turned once while the button is held
            mouselook_zoom_fov: None,
            mouselook_alt_right_menu: false,
            show_mouselook_instructions: true,
            show_interface_in_mouselook: false,
            vr_view: VrView::FirstPerson,
        }
    }
}

/// A camera preset saved by name (LLFloaterSaveCameraPreset: the camera
/// and its focus where they are, in the avatar's frame), used again as a
/// custom view; Aurora's may hold the camera elsewhere and keep the lens,
/// the looks and the light too (`presets`, issue #1164).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraShot {
    pub name: String,
    pub camera_offset: [f32; 3],
    pub focus_offset: [f32; 3],
    /// What holds the camera; None: the preset leaves it where it is. A
    /// position saved before presets is on the avatar.
    #[serde(default = "avatar_anchor")]
    pub anchor: Option<super::presets::Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<super::presets::Lens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub looks: Option<super::presets::Looks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<super::presets::Light>,
}

fn avatar_anchor() -> Option<super::presets::Anchor> {
    Some(super::presets::Anchor::Avatar)
}

impl CameraShot {
    /// A camera position in the avatar's frame, nothing else (the Legacy
    /// Viewer's camera preset).
    pub fn at_avatar(name: String, camera_offset: [f32; 3], focus_offset: [f32; 3]) -> CameraShot {
        CameraShot {
            name,
            camera_offset,
            focus_offset,
            anchor: avatar_anchor(),
            lens: None,
            looks: None,
            light: None,
        }
    }

    /// The preset keeps something to apply.
    pub fn has_any(&self) -> bool {
        self.anchor.is_some() || self.lens.is_some() || self.looks.is_some() || self.light.is_some()
    }
}

/// LLZoomer (indra/newview/llviewermenu.cpp) changes the saved default lens,
/// bounded by LLCamera::getMinView/getMaxView and any RLVa limits.
pub fn zoom_lens(fov: f32, factor: Option<f32>, aspect: f32, limits: Option<(f32, f32)>) -> f32 {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    let low = (5.0f32.to_radians() / aspect.min(1.0)).min(175.0f32.to_radians());
    let high = (175.0f32.to_radians() / aspect.max(1.0)).max(low);
    let (low, high) = limits.map_or((low, high), |(a, b)| {
        let a = a.clamp(low, high);
        (a, b.clamp(a, high))
    });
    factor.map_or(DEFAULT_FOV, |factor| fov * factor).clamp(low, high)
}

impl CameraSettings {
    /// The view of `shot` (Firestorm loads the preset's offsets).
    pub fn apply_shot(&mut self, shot: &CameraShot) {
        self.preset = CameraPreset::Custom;
        self.camera_offset = shot.camera_offset;
        self.focus_offset = shot.focus_offset;
        // the saved offset is where the camera was: at its own distance
        self.offset_scale = 1.0;
    }
}

/// DEFAULT_FIELD_OF_VIEW (60°).
pub const DEFAULT_FOV: f32 = std::f32::consts::FRAC_PI_3;
/// LLViewerCamera min / max view angles of the "Viewing angle" slider.
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
        self.fov = fix(self.fov, 5.0f32.to_radians(), 175.0f32.to_radians(), d.fov);
        self.mouselook_zoom_fov = super::mouselook_zoom::sanitize(self.mouselook_zoom_fov);
        self.zoom_time = fix(self.zoom_time, 0.0, 4.0, d.zoom_time);
        self.smoothing = fix(self.smoothing, 0.0, 9.0, d.smoothing);
        self.dynamic_strength = fix(self.dynamic_strength, 0.0, 30.0, d.dynamic_strength);
        self
    }
}

#[cfg(test)]
mod lens_zoom_tests {
    use super::*;

    #[test]
    fn lens_steps_reset_and_respect_window_and_script_limits() {
        let wide = 16.0 / 9.0;
        let inside = zoom_lens(DEFAULT_FOV, Some(1.0 / 1.2), wide, None);
        assert!((inside - DEFAULT_FOV / 1.2).abs() < 1e-6);
        assert!((zoom_lens(inside, Some(1.2), wide, None) - DEFAULT_FOV).abs() < 1e-6);
        assert_eq!(zoom_lens(inside, None, wide, None), DEFAULT_FOV);
        assert_eq!(zoom_lens(0.001, Some(1.0), wide, None), 5.0f32.to_radians());
        assert_eq!(zoom_lens(10.0, Some(1.0), wide, None), 175.0f32.to_radians() / wide);
        assert_eq!(zoom_lens(0.001, Some(1.0), 0.5, None), 10.0f32.to_radians());
        assert_eq!(zoom_lens(DEFAULT_FOV, None, wide, Some((0.3, 0.7))), 0.7);
        let saved = CameraSettings {
            fov: 5.0f32.to_radians(),
            ..Default::default()
        };
        assert_eq!(saved.sanitized().fov, 5.0f32.to_radians());
    }
}
