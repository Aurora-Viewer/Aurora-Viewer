//! Top-right audio controls (Firestorm style): parcel media and parcel
//! music play toggles, and the volume panel (Principal, Interface,
//! Ambiance, Sons, Musique, Multimédia, Voix) with mute and enable toggles.

use super::icons::Icons;
use crate::settings::{AudioSettings, audio_channels};
use crate::theme::Palette;
use egui::{Color32, RichText, Vec2};

#[derive(Default)]
pub struct AudioUi {
    pub panel_open: bool,
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
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(20.0, 18.0), egui::Sense::click());
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
    resp.on_hover_text(tip)
}

fn speaker_icon(volume: f32, muted: bool) -> &'static str {
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

/// Draw the controls in a right-to-left layout; returns an action.
pub fn top_controls(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    audio: &mut AudioSettings,
    st: &mut AudioUi,
    media: MediaState,
) -> AudioAction {
    let mut action = AudioAction::None;
    // the three icons form one group: tight, even gaps inside it, the bar's
    // spacing around it
    let sp = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.x = ICON_GAP;
            // speaker: the volume panel on click
            let master = speaker_icon(audio.volume[0], audio.muted[0]);
            let sp = icon_toggle(
                ui,
                p,
                icons,
                master,
                aurora_i18n::tr!("ui-audio-volumes-click"),
                st.panel_open,
                true,
            );
            if sp.clicked() {
                st.panel_open = !st.panel_open;
            }
            let music = icon_toggle(
                ui,
                p,
                icons,
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
            let med = icon_toggle(
                ui,
                p,
                icons,
                "device-tv-filled",
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

    if st.panel_open {
        let anchor = sp.rect.right_bottom() + Vec2::new(4.0, 6.0);
        let area = egui::Area::new(egui::Id::new("volume_panel"))
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
                            let r = ui.add_enabled(
                                audio.enabled[i],
                                egui::Slider::new(&mut audio.volume[i], 0.0..=1.0)
                                    .custom_parser(aurora_i18n::parse_number)
                                    .show_value(false)
                                    .trailing_fill(true),
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
                            } else if ui
                                .checkbox(&mut audio.enabled[i], "")
                                .on_hover_text(if audio.enabled[i] {
                                    aurora_i18n::tr!("ui-audio-disable")
                                } else {
                                    aurora_i18n::tr!("common-enable")
                                })
                                .changed()
                            {
                                action = AudioAction::Changed;
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(4.0);
                    if ui
                        .checkbox(
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
        // click outside closes the panel
        let outside = ui.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && !sp.contains_pointer();
        if outside {
            st.panel_open = false;
        }
    }
    let _ = Color32::TRANSPARENT;
    action
}
