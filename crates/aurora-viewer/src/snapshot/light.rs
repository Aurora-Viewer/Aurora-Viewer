//! Quick light for photos (Firestorm's Phototools, floater_phototools.xml:
//! Quick Environments, the sky, water and day cycle lists of its Env tab,
//! Pause Clouds, Enable Attached Lights, Exposure, and the sun of "Personal
//! lighting"): a time of day, a sky, a water, a day cycle, the sun's
//! azimuth and elevation, an exposure, the clouds held still and the lights
//! worn by avatars (face lights) off, for the photo only, while the photo
//! window is open. Firestorm changes the environment, the cloud scroll and
//! RenderAttachedLights until they are changed back; here nothing is saved
//! and closing the window gives back the user's own (`app::snapshot`; the
//! environment is a layer of its own, `world::eep_photo`), unless "Keep
//! this light" makes the photo's environment theirs.

use crate::world::eep_photo::PhotoChoice;

/// The photo window's light, all at the user's own by default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoLight {
    /// The photo's time of day (1 sunrise, 2 noon, 3 sunset, 4 midnight),
    /// sky, water, day cycle and sun over the environment shown.
    pub env: PhotoChoice,
    /// The exposure for the photo; None: Preferences' one.
    pub exposure: Option<f32>,
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
            clouds_paused: false,
            attached_lights: true,
        }
    }
}

/// The range of the exposure (`Settings::exposure`).
pub const EXPOSURE: std::ops::RangeInclusive<f32> = 0.25..=4.0;
