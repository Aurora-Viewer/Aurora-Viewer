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
        let chosen = CameraSettings {
            show_avatar_in_mouselook: true,
            mouselook_follow_eyes: true,
            ..migrated
        };
        let saved = serde_json::to_string(&chosen).unwrap();
        let loaded: CameraSettings = serde_json::from_str(&saved).unwrap();
        assert!(loaded.show_avatar_in_mouselook);
        assert!(loaded.mouselook_follow_eyes);
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

    /// CameraOffsetRearView, FocusOffsetRearView and CameraOffsetScale of the
    /// preset's template (app_settings/camera/<Name>.xml).
    pub fn values(self) -> Option<([f32; 3], [f32; 3], f32)> {
        Some(match self {
            CameraPreset::Rear => ([-3.0, 0.0, 0.75], [1.0, 0.0, 1.0], 1.0),
            CameraPreset::Front => ([2.2, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0),
            CameraPreset::Side => ([-1.0, 0.7, 0.5], [1.5, 0.7, 1.0], 1.0),
            CameraPreset::Shoulder => ([-3.0, -0.4, -0.2], [0.9, -0.7, 0.2], 1.1),
            CameraPreset::Custom => return None,
        })
    }

    /// The view as the user saved it, else its template: the Legacy Viewer
    /// loads "<Name> View.xml" of user_settings/presets/camera, a copy of
    /// the template until "Save as preset" replaced it
    /// (LLFloaterCamera::switchToPreset, LLPresetsManager::loadPreset and
    /// createDefaultCameraPreset).
    pub fn view(self, saved: &SavedViews) -> Option<ViewValues> {
        saved.get(self).or_else(|| {
            self.values().map(|(camera_offset, focus_offset, offset_scale)| ViewValues {
                camera_offset,
                focus_offset,
                offset_scale,
            })
        })
    }

    /// The view a preset name stands for: the Legacy Viewer's file names
    /// ("Front View", PRESETS_FRONT_VIEW; "TPP View" for the shoulder) or
    /// the view's own label, whatever the case. Saving a new preset under
    /// such a name replaces the view (LLFloaterSaveCameraPreset::onBtnSave
    /// hides the views from its name check, LLPresetsManager::savePreset
    /// then writes over "Front View.xml").
    pub fn from_preset_name(name: &str) -> Option<CameraPreset> {
        let name = name.trim();
        [
            (CameraPreset::Rear, "Rear View"),
            (CameraPreset::Front, "Front View"),
            (CameraPreset::Side, "Side View"),
            (CameraPreset::Shoulder, "TPP View"),
        ]
        .into_iter()
        .find(|(view, legacy)| name.eq_ignore_ascii_case(legacy) || name.to_lowercase() == view.label().to_lowercase())
        .map(|(view, _)| view)
    }
}

/// What a view holds: CameraOffsetRearView, FocusOffsetRearView and
/// CameraOffsetScale, the parts of a Legacy Viewer camera preset that place
/// the camera behind the avatar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewValues {
    pub camera_offset: [f32; 3],
    pub focus_offset: [f32; 3],
    pub offset_scale: f32,
}

/// The user's own Rear, Front, Side and Shoulder views, saved over their
/// templates (the Legacy Viewer's "Rear View.xml", "Front View.xml",
/// "Side View.xml" and "TPP View.xml"); none: the template. Kept apart
/// from `CameraSettings`, which "Reset" sets back, as the Legacy Viewer's
/// reset keeps its preset files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedViews {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rear: Option<ViewValues>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub front: Option<ViewValues>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<ViewValues>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shoulder: Option<ViewValues>,
}

impl SavedViews {
    fn slot(&mut self, view: CameraPreset) -> Option<&mut Option<ViewValues>> {
        match view {
            CameraPreset::Rear => Some(&mut self.rear),
            CameraPreset::Front => Some(&mut self.front),
            CameraPreset::Side => Some(&mut self.side),
            CameraPreset::Shoulder => Some(&mut self.shoulder),
            CameraPreset::Custom => None,
        }
    }

    pub fn get(&self, view: CameraPreset) -> Option<ViewValues> {
        match view {
            CameraPreset::Rear => self.rear,
            CameraPreset::Front => self.front,
            CameraPreset::Side => self.side,
            CameraPreset::Shoulder => self.shoulder,
            CameraPreset::Custom => None,
        }
    }

    /// Replace `view` with `values`; false for Custom, which is no view.
    pub fn set(&mut self, view: CameraPreset, values: ViewValues) -> bool {
        self.slot(view).map(|slot| *slot = Some(values)).is_some()
    }

    /// Back to the template (LLPresetsManager::resetCameraPreset); true if
    /// the view had been saved.
    pub fn restore(&mut self, view: CameraPreset) -> bool {
        self.slot(view).and_then(Option::take).is_some()
    }

    /// Values from an edited file: a broken view goes back to its template.
    pub fn sanitize(&mut self) {
        for view in CameraPreset::ALL {
            if let Some(slot) = self.slot(view)
                && slot.is_some_and(|v| !v.is_sound())
            {
                *slot = None;
            }
        }
    }
}

impl ViewValues {
    fn is_sound(&self) -> bool {
        let finite = |v: [f32; 3]| v.iter().all(|x| x.is_finite());
        finite(self.camera_offset)
            && finite(self.focus_offset)
            && Vec3::from(self.camera_offset).length() >= 0.01
            && self.offset_scale.is_finite()
            && (0.5..=3.0).contains(&self.offset_scale)
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
    /// CameraPositionSmoothing ("Smoothing"), × 0.02 s half-life. Limits:
    /// `SMOOTHING_RANGE`.
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
    /// Set up VR at each start (Preferences › Camera › VR), so the VR
    /// button enters it at once. Off by default: loading a VR runtime at
    /// every start crashed PCs that never use VR (#2309); the first
    /// "Restart in VR" turns it on.
    pub vr_at_start: bool,
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
            // both off, as DisableCameraConstraints and
            // FSIgnoreSimulatorCameraConstraints in the Legacy Viewer: the
            // camera keeps within the draw distance and in front of walls.
            // They were on from 2026-10-16; version 33 of the settings turns
            // them off once (`Settings::sanitized`)
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
            show_avatar_in_mouselook: false,
            mouselook_follow_eyes: false,
            wheel_exits_mouselook: true,
            // no zoom until the wheel is turned once while the button is held
            mouselook_zoom_fov: None,
            mouselook_alt_right_menu: false,
            show_mouselook_instructions: true,
            show_interface_in_mouselook: false,
            vr_view: VrView::FirstPerson,
            vr_at_start: false,
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
/// CameraPositionSmoothing: the limits of Phototools' "Cam. Smoothing" slider
/// (floater_phototools_camera.xml, floater_phototools.xml), up to a half-life
/// of 6 s, and not the 9 of panel_preferences_move.xml: both sliders drive the
/// same setting, and Aurora has one.
pub const SMOOTHING_RANGE: std::ops::RangeInclusive<f32> = 0.0..=300.0;

impl CameraSettings {
    pub fn camera_offset(&self) -> Vec3 {
        Vec3::from(self.camera_offset)
    }

    pub fn focus_offset(&self) -> Vec3 {
        Vec3::from(self.focus_offset)
    }

    /// Switch to `preset`: its offsets as the user saved them, else its
    /// template (`CameraPreset::view`); Custom keeps the offsets.
    pub fn apply_view(&mut self, preset: CameraPreset, saved: &SavedViews) {
        self.preset = preset;
        if let Some(v) = preset.view(saved) {
            self.camera_offset = v.camera_offset;
            self.focus_offset = v.focus_offset;
            self.offset_scale = v.offset_scale;
        }
    }

    /// The camera where it is becomes `view` (LLFloaterSaveCameraPreset::
    /// onBtnSave over "Front View"): the offsets of the camera as it is,
    /// at its own distance, which the view shows from now on. False for
    /// Custom.
    pub fn save_view(&mut self, view: CameraPreset, camera_offset: [f32; 3], focus_offset: [f32; 3], saved: &mut SavedViews) -> bool {
        let values = ViewValues {
            camera_offset,
            focus_offset,
            offset_scale: 1.0,
        };
        if !values.is_sound() || !saved.set(view, values) {
            return false;
        }
        self.apply_view(view, saved);
        true
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
        self.smoothing = fix(self.smoothing, *SMOOTHING_RANGE.start(), *SMOOTHING_RANGE.end(), d.smoothing);
        self.dynamic_strength = fix(self.dynamic_strength, 0.0, 30.0, d.dynamic_strength);
        self
    }
}

#[cfg(test)]
mod saved_view_tests {
    use super::*;

    /// Issue #2755: a Front view saved by the user is the one the Front
    /// button gives back, after a restart too, until it is restored.
    #[test]
    fn a_saved_view_comes_back_after_a_restart_until_restored() {
        let mut camera = CameraSettings::default();
        let mut saved = SavedViews::default();
        let (cam, focus) = ([1.8, -0.6, 0.4], [0.2, 0.1, 0.9]);
        assert!(camera.save_view(CameraPreset::Front, cam, focus, &mut saved));
        assert_eq!(
            (camera.preset, camera.camera_offset, camera.focus_offset, camera.offset_scale),
            (CameraPreset::Front, cam, focus, 1.0)
        );

        // elsewhere, then back to the Front view
        camera.apply_view(CameraPreset::Rear, &saved);
        assert_eq!(camera.camera_offset, CameraPreset::Rear.values().unwrap().0);
        camera.apply_view(CameraPreset::Front, &saved);
        assert_eq!((camera.camera_offset, camera.focus_offset), (cam, focus));

        // the settings file keeps it
        let json = serde_json::to_string(&saved).unwrap();
        let mut loaded: SavedViews = serde_json::from_str(&json).unwrap();
        loaded.sanitize();
        assert_eq!(loaded, saved);
        let mut fresh = CameraSettings::default();
        fresh.apply_view(CameraPreset::Front, &loaded);
        assert_eq!((fresh.camera_offset, fresh.focus_offset), (cam, focus));

        // restored: the template again; the other views never moved
        assert!(loaded.restore(CameraPreset::Front));
        assert!(!loaded.restore(CameraPreset::Front));
        fresh.apply_view(CameraPreset::Front, &loaded);
        let (t_cam, t_focus, t_scale) = CameraPreset::Front.values().unwrap();
        assert_eq!(
            (fresh.camera_offset, fresh.focus_offset, fresh.offset_scale),
            (t_cam, t_focus, t_scale)
        );
        assert_eq!(loaded, SavedViews::default());
    }

    #[test]
    fn custom_and_broken_views_are_never_kept() {
        let mut camera = CameraSettings::default();
        let mut saved = SavedViews::default();
        assert!(!camera.save_view(CameraPreset::Custom, [1.0, 0.0, 0.0], [0.0; 3], &mut saved));
        assert!(!camera.save_view(CameraPreset::Side, [f32::NAN, 0.0, 0.0], [0.0; 3], &mut saved));
        assert!(!camera.save_view(CameraPreset::Side, [0.0; 3], [0.0; 3], &mut saved));
        assert_eq!(saved, SavedViews::default());
        assert_eq!(camera, CameraSettings::default());
        // an edited file: the broken view goes back to its template
        let mut edited: SavedViews =
            serde_json::from_str(r#"{"side":{"camera_offset":[0,0,0],"focus_offset":[0,0,0],"offset_scale":1},"rear":{"camera_offset":[-2,0,1],"focus_offset":[1,0,1],"offset_scale":9}}"#)
                .unwrap();
        edited.sanitize();
        assert_eq!(edited, SavedViews::default());
        // older settings have none
        let old: SavedViews = serde_json::from_str("{}").unwrap();
        assert_eq!(old, SavedViews::default());
    }

    #[test]
    fn the_legacy_view_names_and_the_labels_name_the_views() {
        assert_eq!(CameraPreset::from_preset_name("Front View"), Some(CameraPreset::Front));
        assert_eq!(CameraPreset::from_preset_name(" rear view "), Some(CameraPreset::Rear));
        assert_eq!(CameraPreset::from_preset_name("TPP View"), Some(CameraPreset::Shoulder));
        assert_eq!(CameraPreset::from_preset_name(CameraPreset::Side.label()), Some(CameraPreset::Side));
        // the templates' own names and the custom presets are not views
        assert_eq!(CameraPreset::from_preset_name("Front"), None);
        assert_eq!(CameraPreset::from_preset_name("My front"), None);
        assert_eq!(CameraPreset::from_preset_name(CameraPreset::Custom.label()), None);
    }
}

#[cfg(test)]
mod smoothing_range_tests {
    use super::*;

    #[test]
    fn a_saved_smoothing_keeps_the_whole_range_and_only_the_range() {
        let saved = |smoothing| {
            CameraSettings {
                smoothing,
                ..Default::default()
            }
            .sanitized()
            .smoothing
        };
        assert_eq!(saved(300.0), 300.0);
        assert_eq!(saved(301.0), 300.0);
        assert_eq!(saved(-1.0), 0.0);
        assert_eq!(saved(f32::NAN), CameraSettings::default().smoothing);
        // the default stays the Legacy Viewer's
        assert_eq!(CameraSettings::default().smoothing, 1.0);
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
