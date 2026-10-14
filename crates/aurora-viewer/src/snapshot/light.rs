//! Quick light for photos (Firestorm's Phototools, floater_phototools.xml:
//! Quick Environments, Pause Clouds, Enable Attached Lights, Exposure): a
//! time of day, an exposure, the clouds held still and the lights worn by
//! avatars (face lights) off, for the photo only, while the photo window is
//! open. Firestorm changes the environment, the cloud scroll and
//! RenderAttachedLights until they are changed back; here nothing is saved
//! and closing the window gives back the user's own (`app::snapshot`).

/// The photo window's light, all at the user's own by default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoLight {
    /// A time of day for the photo (`Panels::time_of_day`: 1 sunrise,
    /// 2 noon, 3 sunset, 4 midnight); None: the one shown.
    pub time: Option<u8>,
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
            time: None,
            exposure: None,
            clouds_paused: false,
            attached_lights: true,
        }
    }
}

/// The range of the exposure (`Settings::exposure`).
pub const EXPOSURE: std::ops::RangeInclusive<f32> = 0.25..=4.0;

/// The time of day the photo set over the user's, and theirs to give back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldTime {
    before: u8,
    chosen: u8,
}

/// One frame of the photo's time of day: `shown` is `Panels::time_of_day`
/// (changed in place), `want` the photo's choice. A time the user picks
/// elsewhere meanwhile (World menu, environment window) is theirs: the
/// photo's choice is dropped and theirs stays.
pub fn apply_time(held: &mut Option<HeldTime>, want: &mut Option<u8>, shown: &mut u8) {
    if let Some(h) = *held
        && *shown != h.chosen
    {
        *held = None;
        *want = None;
        return;
    }
    match (*held, *want) {
        (None, Some(t)) => {
            *held = Some(HeldTime { before: *shown, chosen: t });
            *shown = t;
        }
        (Some(h), Some(t)) if t != h.chosen => {
            *held = Some(HeldTime { chosen: t, ..h });
            *shown = t;
        }
        (Some(h), None) => {
            *shown = h.before;
            *held = None;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_photos_time_of_day_gives_the_users_back() {
        let (mut held, mut want, mut shown) = (None, Some(3), 0);
        apply_time(&mut held, &mut want, &mut shown);
        assert_eq!(shown, 3);
        want = Some(4);
        apply_time(&mut held, &mut want, &mut shown);
        assert_eq!(shown, 4);
        // the window closes (or "Current"): the user's own again
        want = None;
        apply_time(&mut held, &mut want, &mut shown);
        assert_eq!((shown, held), (0, None));
    }

    #[test]
    fn a_time_of_day_picked_elsewhere_stays_the_users() {
        let (mut held, mut want, mut shown) = (None, Some(2), 1);
        apply_time(&mut held, &mut want, &mut shown);
        // the World menu sets midnight meanwhile
        shown = 4;
        apply_time(&mut held, &mut want, &mut shown);
        assert_eq!((shown, held, want), (4, None, None));
        // closing the window leaves it
        apply_time(&mut held, &mut want, &mut shown);
        assert_eq!(shown, 4);
    }
}
