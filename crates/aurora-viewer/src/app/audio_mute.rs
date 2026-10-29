//! Transient audio override from LLViewerWindow::handleActivate and
//! audio_update_volume (indra/newview/llviewerwindow.cpp, llvieweraudio.cpp).

use crate::settings::AudioSettings;

#[derive(Default)]
pub(super) struct MinimizedMute {
    minimized: bool,
    pub active: bool,
}

impl MinimizedMute {
    /// The renderer temporarily owns the window during a frame. An absent
    /// window is not a restoration; keep the last native state in that case.
    pub fn update(&mut self, enabled: bool, minimized: Option<bool>) -> bool {
        if let Some(minimized) = minimized {
            self.minimized = minimized;
        }
        let active = enabled && self.minimized;
        let changed = self.active != active;
        self.active = active;
        changed
    }
}

/// Plugins play outside the audio mixer and need its transient master mute.
pub(super) fn media_volume(audio: &AudioSettings, minimized_mute: bool) -> f32 {
    if minimized_mute {
        return 0.0;
    }
    let channel = |i: usize| {
        if audio.muted[i] || !audio.enabled[i] {
            0.0
        } else {
            audio.volume[i]
        }
    };
    channel(0) * channel(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimized_mute_survives_the_renderer_taking_the_window() {
        let mut mute = MinimizedMute::default();
        assert!(mute.update(true, Some(true)));
        for _ in 0..10 {
            assert!(!mute.update(true, None));
            assert!(mute.active);
        }
        assert!(mute.update(true, Some(false)));
        assert!(!mute.active);
        assert!(!mute.update(true, None));
    }

    #[test]
    fn minimized_mute_follows_the_option_without_changing_window_state() {
        let mut mute = MinimizedMute::default();
        assert!(!mute.update(false, Some(true)));
        assert!(mute.update(true, None));
        assert!(mute.active);
        assert!(mute.update(false, None));
        assert!(!mute.active);
        assert!(!mute.update(true, Some(false)), "losing focus alone keeps sound on");
    }

    #[test]
    fn minimized_mute_restores_media_gain_and_preserves_saved_mutes() {
        let mut audio = AudioSettings::default();
        audio.volume[0] = 0.5;
        audio.volume[5] = 0.4;
        assert_eq!(media_volume(&audio, false), 0.2);
        assert_eq!(media_volume(&audio, true), 0.0);
        assert_eq!(media_volume(&audio, false), 0.2);
        for i in [0, 5] {
            audio.muted[i] = true;
            assert_eq!(media_volume(&audio, false), 0.0);
            audio.muted[i] = false;
            audio.enabled[i] = false;
            assert_eq!(media_volume(&audio, false), 0.0);
            audio.enabled[i] = true;
        }
    }
}
