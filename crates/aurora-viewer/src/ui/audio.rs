//! Top-right audio controls (Firestorm style): parcel media and parcel
//! music play toggles, and the volume panel (Principal, Interface,
//! Ambiance, Sons, Musique, Multimédia, Voix) with mute and enable toggles.

use super::icons::Icons;
use crate::settings::{AUDIO_CHANNELS, AudioSettings};
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
        ui.painter().rect_filled(rect, 2.0, p.raised);
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
        "speaker-x"
    } else if volume <= 0.001 {
        "speaker-none"
    } else if volume < 0.5 {
        "speaker-low"
    } else {
        "speaker-high"
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
            let sp = icon_toggle(ui, p, icons, master, "Volumes (clic)", st.panel_open, true);
            if sp.clicked() {
                st.panel_open = !st.panel_open;
            }
            let music = icon_toggle(
                ui,
                p,
                icons,
                "radio",
                if !media.music_available {
                    "Pas de musique sur cette parcelle"
                } else if media.music_playing {
                    "Arrêter la musique de la parcelle"
                } else {
                    "Écouter la musique de la parcelle"
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
                "monitor-play",
                if !media.media_available {
                    "Pas de média sur cette parcelle"
                } else if media.media_playing {
                    "Arrêter le média de la parcelle"
                } else {
                    "Lire le média de la parcelle"
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
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(egui::Stroke::new(1.0, p.raised))
                    .corner_radius(egui::CornerRadius::same(3))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.spacing_mut().slider_width = 120.0;
                        egui::Grid::new("volumes").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                            for (i, name) in AUDIO_CHANNELS.iter().enumerate() {
                                let label = if i == 0 {
                                    RichText::new(*name).size(12.5).strong().color(p.ink)
                                } else {
                                    RichText::new(*name).size(12.0).color(p.muted)
                                };
                                ui.label(label);
                                let r = ui.add_enabled(
                                    audio.enabled[i],
                                    egui::Slider::new(&mut audio.volume[i], 0.0..=1.0)
                                        .show_value(false)
                                        .trailing_fill(true),
                                );
                                if r.changed() {
                                    action = AudioAction::Changed;
                                }
                                let icon = speaker_icon(audio.volume[i], audio.muted[i]);
                                let tip = if audio.muted[i] { "Rétablir le son" } else { "Couper le son" };
                                if icon_toggle(ui, p, icons, icon, tip, audio.muted[i], audio.enabled[i]).clicked() {
                                    audio.muted[i] = !audio.muted[i];
                                    action = AudioAction::Changed;
                                }
                                if i == 0 {
                                    if icon_toggle(ui, p, icons, "gear-six", "Préférences", false, true).clicked() {
                                        action = AudioAction::OpenPreferences;
                                    }
                                } else if ui
                                    .checkbox(&mut audio.enabled[i], "")
                                    .on_hover_text(if audio.enabled[i] { "Désactiver" } else { "Activer" })
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
                                RichText::new("Musique de parcelle automatique").size(11.5).color(p.muted),
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
