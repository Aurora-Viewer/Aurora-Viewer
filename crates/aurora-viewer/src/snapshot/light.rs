//! Quick light for photos (Firestorm's Phototools, floater_phototools.xml:
//! Quick Environments, the sky, water and day cycle lists of its Env tab,
//! Pause Clouds, Enable Attached Lights, Exposure, and the sun of "Personal
//! lighting"): a time of day, a sky, a water, a day cycle, the sun's
//! azimuth and elevation, an exposure, the clouds held still and the lights
//! worn by avatars (face lights) off, the softness and the darkness of the
//! sun's shadows, for the photo only, while the photo
//! window is open. Firestorm changes the environment, the cloud scroll and
//! RenderAttachedLights until they are changed back; here nothing is saved
//! and closing the window gives back the user's own (`app::snapshot`; the
//! environment is a layer of its own, `world::eep_photo`), unless "Keep
//! this light" makes the photo's environment theirs.
//!
//! The shadow quality and the ambient occlusion are the Phototools' Shadow
//! detail (RenderShadowDetail) and Ambient Occlusion (RenderDeferredSSAO),
//! on Preferences' own levels; the Legacy Viewer changes the settings
//! themselves, here they hold for the photo only, like the rest.

use crate::world::eep_photo::PhotoChoice;

/// The photo window's light, all at the user's own by default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoLight {
    /// The photo's time of day (1 sunrise, 2 noon, 3 sunset, 4 midnight),
    /// sky, water, day cycle and sun over the environment shown.
    pub env: PhotoChoice,
    /// The exposure for the photo; None: Preferences' one.
    pub exposure: Option<f32>,
    /// The shadow quality for the photo (0 off, as Preferences' levels);
    /// None: Preferences' one.
    pub shadow_quality: Option<u8>,
    /// The ambient occlusion for the photo (0 off, as Preferences'
    /// levels); None: Preferences' one.
    pub ambient_occlusion: Option<u8>,
    /// The softness of the sun's shadows for the photo (soft shadows on);
    /// None: Preferences' one.
    pub shadow_softness: Option<f32>,
    /// How dark the sun's shadows are for the photo; None: Preferences' one.
    pub shadow_intensity: Option<f32>,
    /// The clouds stop moving (LLEnvironment::pauseCloudScroll).
    pub clouds_paused: bool,
    /// The lights worn by avatars light the scene
    /// (LLPipeline::sRenderAttachedLights).
    pub attached_lights: bool,
}

impl Default for PhotoLight {
    fn default() -> Self {
        Self {
            env: PhotoChoice::default(),
            exposure: None,
            shadow_quality: None,
            ambient_occlusion: None,
            shadow_softness: None,
            shadow_intensity: None,
            clouds_paused: false,
            attached_lights: true,
        }
    }
}

/// The range of the exposure (`Settings::exposure`).
pub const EXPOSURE: std::ops::RangeInclusive<f32> = 0.25..=4.0;
/// The ranges of the shadow softness and intensity (`Settings`).
pub const SHADOW_SOFTNESS: std::ops::RangeInclusive<f32> = 0.0..=crate::settings::SHADOW_SOFTNESS_MAX;
pub const SHADOW_INTENSITY: std::ops::RangeInclusive<f32> = 0.0..=1.0;

/// The level a photo override (shadow quality, ambient occlusion) draws
/// with: the photo's while its window is `open`, else the user's. Under the
/// photo quality's boost (`snapshot::quality`) the user's level is the
/// boost's, fitted to the video memory: the photo may lower it, never raise
/// it past what the fit allows.
pub fn photo_level(open: bool, chosen: Option<u8>, user: u8, boosted: bool) -> u8 {
    match chosen {
        Some(level) if open && boosted => level.min(user),
        Some(level) if open => level,
        _ => user,
    }
}

/// The shadow quality and ambient occlusion the world is drawn with: the
/// photo's while its window is `open`, else Preferences'.
pub fn drawn_levels(settings: &crate::settings::Settings, light: &PhotoLight, open: bool) -> (u8, u8) {
    let boost = settings.photo_held.is_some();
    (
        photo_level(open, light.shadow_quality, settings.shadow_quality, boost),
        photo_level(open, light.ambient_occlusion, settings.ssao, boost),
    )
}

/// The softness and the intensity of the sun's shadows the world is drawn
/// with: the photo's while its window is `open`, else Preferences'. The
/// photo's light is reset as the window closes, but not while the Director
/// gives the sun (`App::photo_light`): closed, the photo's values would stay
/// on the world until its track ended.
pub fn drawn_shadows(settings: &crate::settings::Settings, light: &PhotoLight, open: bool) -> (f32, f32) {
    let chosen = |value: Option<f32>| value.filter(|_| open);
    (
        chosen(light.shadow_softness).unwrap_or(settings.shadow_softness),
        chosen(light.shadow_intensity).unwrap_or(settings.shadow_intensity),
    )
}

/// Preferences' render settings with the photo's shadow quality and
/// ambient occlusion over them. Every `apply_settings` goes through it: a
/// change made elsewhere while the photo window is open (Preferences, a
/// resize, a recovered GPU) keeps the photo's. The contact shadows keep
/// their setting: the renderer draws them only with the shadows.
pub fn render_settings(settings: &crate::settings::Settings, light: &PhotoLight, open: bool) -> aurora_render::RenderSettings {
    let (shadow_quality, ssao) = drawn_levels(settings, light, open);
    let mut out = settings.render_settings();
    out.shadow_resolution = crate::settings::Settings::shadow_params_of(shadow_quality).0;
    out.ssao = ssao;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_photo_level_holds_while_the_window_is_open() {
        // nothing chosen: the user's
        assert_eq!(photo_level(true, None, 3, false), 3);
        // chosen: the photo's, lower or higher
        assert_eq!(photo_level(true, Some(0), 3, false), 0);
        assert_eq!(photo_level(true, Some(4), 1, false), 4);
        // the window closed: the user's again
        assert_eq!(photo_level(false, Some(0), 3, false), 3);
    }

    #[test]
    fn the_photo_shadow_sliders_hold_while_the_window_is_open() {
        let settings = crate::settings::Settings {
            shadow_softness: 0.25,
            shadow_intensity: 1.0,
            ..Default::default()
        };
        let light = PhotoLight {
            shadow_softness: Some(2.0),
            shadow_intensity: Some(0.3),
            ..Default::default()
        };
        assert_eq!(drawn_shadows(&settings, &light, true), (2.0, 0.3));
        // closed with the light not reset yet (the Director gives the sun)
        assert_eq!(drawn_shadows(&settings, &light, false), (0.25, 1.0));
        // one chosen, the other Preferences'
        let light = PhotoLight {
            shadow_intensity: Some(0.5),
            ..Default::default()
        };
        assert_eq!(drawn_shadows(&settings, &light, true), (0.25, 0.5));
    }

    #[test]
    fn the_photo_quality_boost_is_a_ceiling() {
        assert_eq!(photo_level(true, Some(4), 3, true), 3, "never past the fit");
        assert_eq!(photo_level(true, Some(0), 3, true), 0, "lowered at will");
        assert_eq!(photo_level(true, None, 3, true), 3);
    }

    #[test]
    fn the_renderer_gets_the_photo_levels_only_while_the_window_is_open() {
        let settings = crate::settings::Settings {
            shadow_quality: 3,
            ssao: 2,
            ..Default::default()
        };
        let light = PhotoLight {
            shadow_quality: Some(0),
            ambient_occlusion: Some(0),
            ..Default::default()
        };
        let open = render_settings(&settings, &light, true);
        assert_eq!((open.shadow_resolution, open.ssao), (0, 0));
        assert_eq!(open.contact_shadows, settings.contact_shadows, "kept: drawn only with the shadows");
        let closed = render_settings(&settings, &light, false);
        assert_eq!(closed, settings.render_settings());
    }
}
