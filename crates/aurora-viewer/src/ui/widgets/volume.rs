//! The volume sliders of the viewer (Preferences › Sound, the volume panel
//! of the bars, the media controls, a resident's voice): one slider, which
//! with "Perceptual volume sliders" on (Preferences › Sound, off by
//! default; issue #463) moves along a loudness curve instead of the gain.
//!
//! The settings keep the linear gain they always held, as the Legacy
//! Viewer's (LLPanelVolumePulldown's sliders set the audio engine's gain
//! directly; LLSpeakerVolumeStorage keeps a resident's 0..1 with 0.5 heard
//! as sent): turning the option on or off moves the knobs, never how loud
//! anything is. Only the knob's position maps through the curve, and the
//! value is written back only when the slider is moved.
//!
//! The curve is the cubic one of PulseAudio's and ALSA's mixers: gain =
//! position³. It follows loudness closely over the useful range (a tenth
//! of the travel is −60 dB, the middle −18 dB), reaches silence at the
//! bottom without the jump a pure dB scale needs there, and its inverse is
//! exact (the cube root).

/// What a slider's value means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeScale {
    /// A gain from 0 (silent) to 1 (full): the audio channels, a media.
    Gain,
    /// A resident's voice, 0..1 with 0.5 heard as they send it (100 %) and
    /// 1 at 200 % (LLSpeakerVolumeStorage). The lower half follows the
    /// curve up to 100 % at the middle, where the slider puts it today; the
    /// upper half, only +6 dB, stays as it is.
    Voice,
}

fn key() -> egui::Id {
    egui::Id::unique("perceptual_volume_sliders")
}

/// `AURORA_PERCEPTUAL_VOLUME=0|1`: the curve forced for this launch, for
/// demo captures, without touching the saved setting.
pub fn perceptual_override() -> Option<bool> {
    static OVERRIDE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| match std::env::var("AURORA_PERCEPTUAL_VOLUME").as_deref().map(str::trim) {
        Ok("1") => Some(true),
        Ok("0") => Some(false),
        _ => None,
    })
}

/// Publishes the setting for the frame's sliders (once per UI frame, by
/// the app); `AURORA_PERCEPTUAL_VOLUME` wins over it.
pub fn set_perceptual_volume(ctx: &egui::Context, on: bool) {
    let on = perceptual_override().unwrap_or(on);
    ctx.data_mut(|d| d.insert_temp(key(), on));
}

/// Whether the volume sliders follow the loudness curve this frame.
pub fn perceptual_volume(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(key())).unwrap_or(false)
}

/// The knob's position (0..1) of a stored `value`.
pub fn volume_position(value: f32, scale: VolumeScale, perceptual: bool) -> f32 {
    let v = value.clamp(0.0, 1.0);
    if !perceptual {
        return v;
    }
    match scale {
        VolumeScale::Gain => v.cbrt(),
        VolumeScale::Voice if v <= 0.5 => 0.5 * (2.0 * v).cbrt(),
        VolumeScale::Voice => v,
    }
}

/// The stored value of a knob's `position` (0..1): the inverse of
/// [`volume_position`].
pub fn volume_value(position: f32, scale: VolumeScale, perceptual: bool) -> f32 {
    let p = position.clamp(0.0, 1.0);
    if !perceptual {
        return p;
    }
    match scale {
        VolumeScale::Gain => p * p * p,
        VolumeScale::Voice if p <= 0.5 => {
            let q = 2.0 * p;
            0.5 * q * q * q
        }
        VolumeScale::Voice => p,
    }
}

/// A volume slider over `value`, following the setting the app published
/// for this frame. `style` finishes the slider (`show_value`,
/// `trailing_fill`…) as each place wants it.
pub fn volume_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    scale: VolumeScale,
    enabled: bool,
    style: impl for<'a> FnOnce(egui::Slider<'a>) -> egui::Slider<'a>,
) -> egui::Response {
    let perceptual = perceptual_volume(ui.ctx());
    volume_slider_with(ui, value, scale, perceptual, enabled, style)
}

/// [`volume_slider`] with the curve given: Preferences › Sound follows its
/// own copy of the setting, which a click there changes before the app
/// publishes it again.
pub fn volume_slider_with(
    ui: &mut egui::Ui,
    value: &mut f32,
    scale: VolumeScale,
    perceptual: bool,
    enabled: bool,
    style: impl for<'a> FnOnce(egui::Slider<'a>) -> egui::Slider<'a>,
) -> egui::Response {
    let mut position = volume_position(*value, scale, perceptual);
    let r = ui.add_enabled(enabled, style(egui::Slider::new(&mut position, 0.0..=1.0)));
    if r.changed() {
        *value = volume_value(position, scale, perceptual);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALES: [VolumeScale; 2] = [VolumeScale::Gain, VolumeScale::Voice];

    #[test]
    fn linear_is_the_identity() {
        for scale in SCALES {
            for i in 0..=100 {
                let x = i as f32 / 100.0;
                assert_eq!(volume_position(x, scale, false), x);
                assert_eq!(volume_value(x, scale, false), x);
            }
        }
    }

    #[test]
    fn round_trip_keeps_the_stored_value() {
        for scale in SCALES {
            for i in 0..=1000 {
                let v = i as f32 / 1000.0;
                let back = volume_value(volume_position(v, scale, true), scale, true);
                assert!((back - v).abs() < 1e-5, "{scale:?} {v} -> {back}");
                let p = v;
                let again = volume_position(volume_value(p, scale, true), scale, true);
                assert!((again - p).abs() < 1e-5, "{scale:?} position {p} -> {again}");
            }
        }
    }

    #[test]
    fn ends_stay_and_the_curve_rises() {
        for scale in SCALES {
            assert_eq!(volume_value(0.0, scale, true), 0.0);
            assert_eq!(volume_value(1.0, scale, true), 1.0);
            assert_eq!(volume_position(0.0, scale, true), 0.0);
            assert_eq!(volume_position(1.0, scale, true), 1.0);
            let mut last = -1.0;
            for i in 0..=200 {
                let v = volume_value(i as f32 / 200.0, scale, true);
                assert!(v >= last, "{scale:?} falls at step {i}");
                last = v;
            }
        }
    }

    #[test]
    fn gain_gives_the_low_end_more_travel() {
        // the middle of the slider is −18 dB, not −6 dB
        assert!((volume_value(0.5, VolumeScale::Gain, true) - 0.125).abs() < 1e-6);
        // half the gain sits at about four fifths of the travel
        assert!((volume_position(0.5, VolumeScale::Gain, true) - 0.7937).abs() < 1e-3);
    }

    #[test]
    fn voice_keeps_100_percent_in_the_middle() {
        // 0.5 is heard as sent (100 %), 1 is 200 %
        assert_eq!(volume_value(0.5, VolumeScale::Voice, true), 0.5);
        assert_eq!(volume_position(0.5, VolumeScale::Voice, true), 0.5);
        // the upper half is unchanged, the lower one follows the curve
        assert_eq!(volume_value(0.75, VolumeScale::Voice, true), 0.75);
        assert!((volume_value(0.25, VolumeScale::Voice, true) - 0.0625).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        for scale in SCALES {
            for perceptual in [false, true] {
                assert_eq!(volume_position(-0.2, scale, perceptual), 0.0);
                assert_eq!(volume_position(1.4, scale, perceptual), 1.0);
                assert_eq!(volume_value(1.4, scale, perceptual), 1.0);
            }
        }
    }
}
