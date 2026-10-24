//! Top-right audio controls (Firestorm style): parcel media and parcel
//! music play toggles, and the speaker. A click on the speaker mutes or
//! restores all sound (LLStatusBar::onClickVolume, MuteAudio); hovering it
//! shows the volume panel (LLStatusBar::onMouseEnterVolume,
//! panel_volume_pulldown.xml): Main, Interface, Ambient, Sounds, Music,
//! Multimedia, Voice, each with its mute, and an enable box for the music,
//! the media and the voice.

use super::icons::Icons;
use crate::settings::{AudioSettings, audio_channels};
use crate::theme::Palette;
use egui::{RichText, Vec2};

#[derive(Default)]
pub struct AudioUi {
    pub panel_open: bool,
    /// When the pointer left both the speaker and the open panel.
    left_at: Option<f64>,
}

/// How long the panel stays once the pointer left the speaker and the panel:
/// long enough to travel from the icon down to the panel.
const CLOSE_DELAY: f64 = 0.35;

/// Channels that get an enable box in the panel (indices of
/// `audio_channels`): the music, the media and the voice, the ones people
/// switch off. The other channels keep theirs in Preferences › Sound.
const PANEL_TOGGLES: [usize; 3] = [4, 5, 6];

impl AudioUi {
    /// Hover state of the panel for one frame. It opens as soon as the
    /// pointer is on the speaker, stays while the pointer is on the speaker
    /// or the panel (or a button is held, a slider being dragged past its
    /// edge), and closes `CLOSE_DELAY` after the pointer left both. Returns
    /// whether a repaint is needed to close it on time.
    fn hover(&mut self, on_icon: bool, on_panel: bool, held: bool, now: f64) -> bool {
        if on_icon || (self.panel_open && (on_panel || held)) {
            self.panel_open = true;
            self.left_at = None;
            return false;
        }
        if !self.panel_open {
            return false;
        }
        let left = *self.left_at.get_or_insert(now);
        if now - left >= CLOSE_DELAY {
            self.panel_open = false;
            self.left_at = None;
            false
        } else {
            true
        }
    }
}

/// The click on the speaker: all sound off, or back on.
fn toggle_master_mute(audio: &mut AudioSettings) {
    audio.muted[0] = !audio.muted[0];
}

/// What the enable box of a channel of the panel switches. The voice one is
/// the voice connection itself (EnableVoiceChat of panel_volume_pulldown.xml):
/// unticked, the voice disconnects, microphone included, the same setting as
/// Preferences › Sound › Voice. Music and media switch their channel.
fn enable_box(audio: &mut AudioSettings, i: usize) -> Option<&mut bool> {
    match i {
        6 => Some(&mut audio.voice_enabled),
        _ if PANEL_TOGGLES.contains(&i) => Some(&mut audio.enabled[i]),
        _ => None,
    }
}

/// What the parcel offers and what is playing.
#[derive(Default, Clone, Copy)]
pub struct MediaState {
    pub music_available: bool,
    pub music_playing: bool,
    pub media_available: bool,
    pub media_playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAction {
    None,
    ToggleMusic,
    ToggleMedia,
    /// Volumes / mutes / toggles changed: apply to the audio engine.
    Changed,
    OpenPreferences,
}

/// Gap between the speaker, radio and media icons (their 20 px slots
/// already pad the 15 px glyphs).
const ICON_GAP: f32 = 2.0;

fn icon_toggle(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, tip: &str, active: bool, enabled: bool) -> egui::Response {
    let resp = icon_button(ui, p, icons, icon, active, enabled);
    resp.on_hover_text(tip)
}

/// Slot of the speaker, radio and media buttons in Firestorm's top bar (the
/// islands give theirs).
pub const BAR_SLOT: Vec2 = Vec2::new(20.0, 18.0);

fn icon_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, active: bool, enabled: bool) -> egui::Response {
    slot_button(ui, p, icons, icon, active, enabled, BAR_SLOT)
}

fn slot_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, active: bool, enabled: bool, slot: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(slot, egui::Sense::click());
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if !enabled {
            p.muted_dim.gamma_multiply(0.6)
        } else if active {
            p.violet_light
        } else if resp.hovered() {
            p.ink
        } else {
            p.muted
        };
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(15.0));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp
}

/// The speaker of a volume (0..1), struck through when muted: the audio
/// panel's channels and the residents' voice rows of the menus.
pub(crate) fn speaker_icon(volume: f32, muted: bool) -> &'static str {
    if muted {
        "volume-off"
    } else if volume <= 0.001 {
        "volume-3"
    } else if volume < 0.5 {
        "volume-2"
    } else {
        "volume"
    }
}

/// Draw the controls in a right-to-left layout, each button in a `slot`;
/// returns an action.
pub fn top_controls(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    audio: &mut AudioSettings,
    st: &mut AudioUi,
    media: MediaState,
    slot: Vec2,
) -> AudioAction {
    let toggle = |ui: &mut egui::Ui, icon: &str, tip: &str, active: bool, enabled: bool| {
        slot_button(ui, p, icons, icon, active, enabled, slot).on_hover_text(tip)
    };
    let mut action = AudioAction::None;
    // demo captures: AURORA_DEMO_VOLUME=open pins the panel open, `muted`
    // also draws the speaker muted (display only, the settings stay)
    static DEMO: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let demo = DEMO.get_or_init(|| std::env::var("AURORA_DEMO_VOLUME").ok()).as_deref();
    let demo_muted = demo == Some("muted");
    // the three icons form one group: tight, even gaps inside it, the bar's
    // spacing around it
    let sp = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.x = ICON_GAP;
            // speaker: all sound on / off on click, the volume panel on
            // hover (no tooltip: the panel opens where it would show)
            let master = speaker_icon(audio.volume[0], audio.muted[0] || demo_muted);
            let sp = slot_button(ui, p, icons, master, st.panel_open, true, slot);
            if sp.clicked() {
                toggle_master_mute(audio);
                action = AudioAction::Changed;
            }
            let music = toggle(
                ui,
                "radio",
                if !media.music_available {
                    aurora_i18n::tr!("ui-audio-no-music-on-this-parcel")
                } else if media.music_playing {
                    aurora_i18n::tr!("ui-audio-stop-parcel-music")
                } else {
                    aurora_i18n::tr!("ui-audio-listen-to-parcel-music")
                },
                media.music_playing,
                media.music_available,
            );
            if music.clicked() && media.music_available {
                action = AudioAction::ToggleMusic;
            }
            let med = toggle(
                ui,
                "brand-youtube-filled",
                if !media.media_available {
                    aurora_i18n::tr!("ui-audio-no-media-on-this-parcel")
                } else if media.media_playing {
                    aurora_i18n::tr!("ui-audio-stop-parcel-media")
                } else {
                    aurora_i18n::tr!("ui-audio-play-parcel-media")
                },
                media.media_playing,
                media.media_available,
            );
            if med.clicked() && media.media_available {
                action = AudioAction::ToggleMedia;
            }
            sp
        })
        .inner;

    let on_icon = sp.contains_pointer() || demo.is_some();
    if on_icon {
        st.hover(true, false, false, 0.0);
    }
    if st.panel_open {
        let anchor = sp.rect.right_bottom() + Vec2::new(4.0, 6.0);
        let area = egui::Area::new(egui::Id::unique("volume_panel"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(anchor)
            .show(ui.ctx(), |ui| {
                super::widgets::overlay(ui, p, egui::Margin::symmetric(10, 8), |ui| {
                    ui.spacing_mut().slider_width = 120.0;
                    egui::Grid::new("volumes").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                        for (i, name) in audio_channels().iter().enumerate() {
                            let label = if i == 0 {
                                RichText::new(*name).size(12.5).strong().color(p.ink)
                            } else {
                                RichText::new(*name).size(12.0).color(p.muted)
                            };
                            ui.label(label);
                            let r = super::widgets::volume_slider(
                                ui,
                                &mut audio.volume[i],
                                super::widgets::VolumeScale::Gain,
                                audio.enabled[i],
                                |s| s.custom_parser(aurora_i18n::parse_number).show_value(false).trailing_fill(true),
                            );
                            if r.changed() {
                                action = AudioAction::Changed;
                            }
                            let icon = speaker_icon(audio.volume[i], audio.muted[i]);
                            let tip = if audio.muted[i] {
                                aurora_i18n::tr!("common-unmute")
                            } else {
                                aurora_i18n::tr!("common-mute")
                            };
                            if icon_toggle(ui, p, icons, icon, tip, audio.muted[i], audio.enabled[i]).clicked() {
                                audio.muted[i] = !audio.muted[i];
                                action = AudioAction::Changed;
                            }
                            if i == 0 {
                                if icon_toggle(ui, p, icons, "settings-filled", aurora_i18n::tr!("common-preferences"), false, true)
                                    .clicked()
                                {
                                    action = AudioAction::OpenPreferences;
                                }
                            } else if let Some(on) = enable_box(audio, i) {
                                let tip = if *on {
                                    aurora_i18n::tr!("ui-audio-disable")
                                } else {
                                    aurora_i18n::tr!("common-enable")
                                };
                                if super::widgets::checkbox(ui, p, on, "").on_hover_text(tip).changed() {
                                    action = AudioAction::Changed;
                                }
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(4.0);
                    if super::widgets::checkbox(
                        ui,
                        p,
                        &mut audio.music_autoplay,
                        RichText::new(aurora_i18n::tr!("ui-audio-auto-play-parcel-music"))
                            .size(11.5)
                            .color(p.muted),
                    )
                    .changed()
                    {
                        action = AudioAction::Changed;
                    }
                });
            });
        let on_panel = area.response.contains_pointer();
        let (held, pressed, now) = ui.input(|i| (i.pointer.any_down(), i.pointer.any_pressed(), i.time));
        if demo.is_none() {
            // a click elsewhere closes it at once, like the other pop-ups
            if pressed && !on_panel && !on_icon {
                st.panel_open = false;
                st.left_at = None;
            } else if st.hover(on_icon, on_panel, held && !pressed, now) {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(CLOSE_DELAY));
            }
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speaker_click_mutes_and_restores_all_sound() {
        let mut a = AudioSettings::default();
        let before = a.clone();
        toggle_master_mute(&mut a);
        assert!(a.muted[0]);
        assert_eq!(speaker_icon(a.volume[0], a.muted[0]), "volume-off");
        // the channels keep their own volume, mute and enable state
        assert_eq!(a.muted[1..], before.muted[1..]);
        assert_eq!(a.enabled, before.enabled);
        assert_eq!(a.volume, before.volume);
        toggle_master_mute(&mut a);
        assert_eq!(a, before);
        assert_ne!(speaker_icon(a.volume[0], a.muted[0]), "volume-off");
    }

    #[test]
    fn the_panel_opens_on_hover_and_closes_after_the_delay() {
        let mut st = AudioUi::default();
        assert!(!st.hover(false, false, false, 0.0));
        assert!(!st.panel_open);
        st.hover(true, false, false, 1.0);
        assert!(st.panel_open);
        // on the way from the icon to the panel: still open
        assert!(st.hover(false, false, false, 1.1));
        assert!(st.hover(false, false, false, 1.1 + CLOSE_DELAY - 0.01));
        assert!(st.panel_open);
        // reached the panel: stays open as long as the pointer is on it
        st.hover(false, true, false, 1.3);
        st.hover(false, true, false, 9.0);
        assert!(st.panel_open);
        // left both: closes once the delay is over
        assert!(st.hover(false, false, false, 10.0));
        assert!(st.panel_open);
        assert!(!st.hover(false, false, false, 10.0 + CLOSE_DELAY + 0.01));
        assert!(!st.panel_open);
    }

    #[test]
    fn a_held_button_keeps_the_open_panel_but_does_not_open_it() {
        let mut st = AudioUi::default();
        st.hover(false, false, true, 0.0);
        assert!(!st.panel_open);
        st.hover(true, false, false, 1.0);
        // a slider dragged past the edge of the panel
        st.hover(false, false, true, 2.0);
        st.hover(false, false, true, 5.0);
        assert!(st.panel_open);
    }

    #[test]
    fn the_voice_box_is_the_voice_connection_of_the_preferences() {
        let mut a = AudioSettings::default();
        assert!(a.voice_enabled);
        *enable_box(&mut a, 6).expect("the voice has a box") = false;
        // the very setting the voice system and Preferences › Sound read:
        // disconnected, microphone included; the voice channel itself stays
        assert!(!a.voice_enabled);
        assert!(a.enabled[6]);
        *enable_box(&mut a, 6).expect("the voice has a box") = true;
        assert!(a.voice_enabled);
        // music and media switch their channel
        *enable_box(&mut a, 4).expect("the music has a box") = false;
        assert!(!a.enabled[4] && a.voice_enabled);
        for i in [0, 1, 2, 3] {
            assert!(enable_box(&mut a, i).is_none());
        }
    }

    #[test]
    fn only_music_media_and_voice_have_an_enable_box() {
        let channels = audio_channels();
        let boxed: Vec<_> = PANEL_TOGGLES.iter().map(|&i| channels[i]).collect();
        assert_eq!(
            boxed,
            [
                aurora_i18n::tr!("common-music"),
                aurora_i18n::tr!("settings-multimedia"),
                aurora_i18n::tr!("common-voice")
            ]
        );
    }
}
