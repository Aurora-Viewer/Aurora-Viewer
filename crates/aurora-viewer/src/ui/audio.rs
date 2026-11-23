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
    /// The media button this frame, and whether the pointer is on it: the
    /// nearby media panel (`ui/nearby_media.rs`) opens under it.
    pub media_button: Option<egui::Rect>,
    pub media_button_hovered: bool,
}

/// How long the panel stays once the pointer left the speaker and the panel:
/// long enough to travel from the icon down to the panel.
pub(crate) const CLOSE_DELAY: f64 = 0.35;

/// Hover state of a panel opened from a bar icon, for one frame. It opens
/// as soon as the pointer is on the icon, stays while the pointer is on the
/// icon or the panel (or a button is held, a slider being dragged past its
/// edge), and closes `CLOSE_DELAY` after the pointer left both. Returns
/// whether a repaint is needed to close it on time.
pub(crate) fn hover_step(open: &mut bool, left_at: &mut Option<f64>, on_icon: bool, on_panel: bool, held: bool, now: f64) -> bool {
    if on_icon || (*open && (on_panel || held)) {
        *open = true;
        *left_at = None;
        return false;
    }
    if !*open {
        return false;
    }
    let left = *left_at.get_or_insert(now);
    if now - left >= CLOSE_DELAY {
        *open = false;
        *left_at = None;
        false
    } else {
        true
    }
}

/// Channels that get an enable box in the panel (indices of
/// `audio_channels`): the music, the media and the voice, the ones people
/// switch off. The other channels keep theirs in Preferences › Sound.
const PANEL_TOGGLES: [usize; 3] = [4, 5, 6];

impl AudioUi {
    /// Hover state of the volume panel for one frame ([`hover_step`]).
    fn hover(&mut self, on_icon: bool, on_panel: bool, held: bool, now: f64) -> bool {
        hover_step(&mut self.panel_open, &mut self.left_at, on_icon, on_panel, held, now)
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
        _ if PANEL_TOGGLES.contains(&i) => Some(audio.channel_box(i)),
        _ => None,
    }
}

/// The requested radio can be on while its connection is still silent.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MusicStatus {
    #[default]
    Unavailable,
    Stopped,
    Disabled,
    Muted,
    Waiting,
    Blocked,
    Connecting,
    /// Heard; with the failed certificate of an `https` station played
    /// anyway (aurora-audio `tls.rs`), shown as a red warning.
    Playing(Option<aurora_audio::CertificateIssue>),
    /// The last attempt failed, with its reason; kept while a retry connects.
    Error(aurora_audio::StreamError),
    NoDevice,
}

/// Why the station cannot be heard, in the resident's language; the server's
/// own text stays in the log.
fn error_reason(error: aurora_audio::StreamError) -> String {
    use aurora_audio::StreamError;
    match error {
        StreamError::WebPage => aurora_i18n::tr!("music-error-web-page").to_owned(),
        StreamError::Playlist => aurora_i18n::tr!("music-error-playlist").to_owned(),
        StreamError::HttpStatus(code) => aurora_i18n::tr!("music-error-http", code = code.to_string()),
        StreamError::Tls => aurora_i18n::tr!("music-error-tls").to_owned(),
        StreamError::Connection => aurora_i18n::tr!("music-error-connection").to_owned(),
        StreamError::Address => aurora_i18n::tr!("music-error-address").to_owned(),
        StreamError::AudioFormat => aurora_i18n::tr!("music-error-format").to_owned(),
        StreamError::Interrupted => aurora_i18n::tr!("music-error-interrupted").to_owned(),
        StreamError::Worker => aurora_i18n::tr!("music-error-worker").to_owned(),
    }
}

/// What is wrong with the certificate of a station played anyway, and that
/// its connection is not secure. An expiry says when, in local time.
pub fn certificate_warning(issue: aurora_audio::CertificateIssue) -> String {
    use aurora_audio::CertificateIssue;
    use chrono::{Datelike, Timelike};
    let reason = match issue {
        CertificateIssue::Expired { not_after: Some(secs) } => match chrono::DateTime::from_timestamp(secs, 0) {
            Some(at) => {
                let at = at.with_timezone(&chrono::Local);
                aurora_i18n::tr!(
                    "music-certificate-expired-on",
                    date = super::land::day_month(Some(i64::from(at.year())), at.month() as usize, i64::from(at.day())),
                    time = format!("{:02}:{:02}", at.hour(), at.minute())
                )
            }
            None => aurora_i18n::tr!("music-certificate-expired").to_owned(),
        },
        CertificateIssue::Expired { not_after: None } => aurora_i18n::tr!("music-certificate-expired").to_owned(),
        CertificateIssue::NotYetValid => aurora_i18n::tr!("music-certificate-not-yet-valid").to_owned(),
        CertificateIssue::Untrusted => aurora_i18n::tr!("music-certificate-untrusted").to_owned(),
        CertificateIssue::WrongName => aurora_i18n::tr!("music-certificate-wrong-name").to_owned(),
        CertificateIssue::Revoked => aurora_i18n::tr!("music-certificate-revoked").to_owned(),
        CertificateIssue::Other => aurora_i18n::tr!("music-certificate-other").to_owned(),
    };
    aurora_i18n::tr!("music-certificate-warning", reason = reason)
}

/// A red warning line: the filled triangle, then `text` wrapped beside it.
/// The music tooltips of both bar styles show the certificate warning so.
pub fn warning_line(ui: &mut egui::Ui, p: &Palette, icons: &Icons, text: &str, size: f32) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if let Some(icon) = icons.get(WARNING_ICON) {
            ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(size + 2.0)).tint(p.danger));
        }
        ui.add(egui::Label::new(RichText::new(text).size(size).color(p.danger)).wrap());
    });
}

/// The warning sign of the interface (a script error, a refused action).
const WARNING_ICON: &str = "alert-triangle-filled";

impl MusicStatus {
    pub fn read(
        available: bool,
        requested: bool,
        audio: &AudioSettings,
        minimized: bool,
        verdict: crate::media::filter::Verdict,
        stream: Option<&aurora_audio::StreamStatus>,
    ) -> Self {
        use crate::media::filter::Verdict;
        if !available {
            return Self::Unavailable;
        }
        if !requested {
            return Self::Stopped;
        }
        if !audio.enabled[4] {
            return Self::Disabled;
        }
        match verdict {
            Verdict::Ask => return Self::Waiting,
            Verdict::Deny => return Self::Blocked,
            Verdict::Allow => {}
        }
        if minimized || audio.muted[0] || audio.muted[4] || audio.volume[0] <= 0.0 || audio.volume[4] <= 0.0 {
            return Self::Muted;
        }
        match stream {
            None => Self::NoDevice,
            Some(aurora_audio::StreamStatus::Playing { certificate, .. }) => Self::Playing(*certificate),
            Some(aurora_audio::StreamStatus::Error(error)) => Self::Error(*error),
            Some(_) => Self::Connecting,
        }
    }

    pub fn text(self) -> String {
        match self {
            Self::Unavailable => aurora_i18n::tr!("ui-bars-tip-music-none"),
            Self::Stopped => aurora_i18n::tr!("ui-bars-tip-music-stopped"),
            Self::Playing(_) => aurora_i18n::tr!("ui-bars-tip-music-playing"),
            Self::Disabled => aurora_i18n::tr!("ui-music-disabled"),
            Self::Muted => aurora_i18n::tr!("ui-music-muted"),
            Self::Waiting => aurora_i18n::tr!("ui-music-waiting"),
            Self::Blocked => aurora_i18n::tr!("ui-music-blocked"),
            Self::Connecting => aurora_i18n::tr!("ui-music-connecting"),
            Self::NoDevice => aurora_i18n::tr!("ui-music-no-device"),
            Self::Error(error) => return aurora_i18n::tr!("music-state-error", reason = error_reason(error)),
        }
        .to_owned()
    }

    /// The certificate issue of the station heard, if any.
    pub fn certificate(self) -> Option<aurora_audio::CertificateIssue> {
        match self {
            Self::Playing(issue) => issue,
            _ => None,
        }
    }
}

/// What the parcel offers and what is playing.
#[derive(Default, Clone, Copy)]
pub struct MediaState {
    pub music_available: bool,
    pub music_playing: bool,
    pub music_status: MusicStatus,
    /// Media around to start or stop: the parcel's or on a prim
    /// (LLStatusBar::refresh: hasInWorldMedia, hasParcelMedia).
    pub media_available: bool,
    /// A media is on.
    pub media_playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAction {
    None,
    ToggleMusic,
    /// Start or stop all media (LLStatusBar::onClickMediaToggle,
    /// LLViewerMedia::setAllMediaEnabled).
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
    controls(ui, p, icons, audio, st, media, slot, None)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopControl {
    Media,
    Music,
    Volume,
}

/// The same native control and volume panel, independently placed in a toolbar.
/// Click rules follow LLStatusBar::onClickVolume/onClickStreamToggle/onClickMediaToggle
/// (indra/newview/llstatusbar.cpp); moving a control does not change its availability.
#[allow(clippy::too_many_arguments)]
pub(crate) fn top_control(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    audio: &mut AudioSettings,
    st: &mut AudioUi,
    media: MediaState,
    slot: Vec2,
    control: TopControl,
) -> AudioAction {
    controls(ui, p, icons, audio, st, media, slot, Some(control))
}

#[allow(clippy::too_many_arguments)]
fn controls(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    audio: &mut AudioSettings,
    st: &mut AudioUi,
    media: MediaState,
    slot: Vec2,
    only: Option<TopControl>,
) -> AudioAction {
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
            let sp = only.is_none_or(|control| control == TopControl::Volume).then(|| {
                let master = speaker_icon(audio.volume[0], audio.muted[0] || demo_muted);
                let sp = slot_button(ui, p, icons, master, st.panel_open, true, slot);
                if sp.clicked() {
                    toggle_master_mute(audio);
                    action = AudioAction::Changed;
                }
                sp
            });
            if only.is_none_or(|control| control == TopControl::Music) {
                let action_tip = if !media.music_available {
                    aurora_i18n::tr!("ui-audio-no-music-on-this-parcel")
                } else if media.music_playing {
                    aurora_i18n::tr!("ui-audio-stop-parcel-music")
                } else {
                    aurora_i18n::tr!("ui-audio-listen-to-parcel-music")
                };
                // A parcel without music says so alone, without a second "none".
                let music_tip = if media.music_available {
                    format!("{action_tip}\n{}", media.music_status.text())
                } else {
                    action_tip.to_owned()
                };
                let music = slot_button(ui, p, icons, "radio", media.music_playing, media.music_available, slot);
                // a station heard under a failed certificate: the warning sign on
                // the radio's corner, the reason in red in its tooltip
                let warning = media.music_status.certificate().map(certificate_warning);
                let music = match &warning {
                    Some(text) => {
                        if let Some(icon) = icons.get(WARNING_ICON) {
                            let badge = egui::Rect::from_min_size(music.rect.right_bottom() - Vec2::splat(9.0), Vec2::splat(9.0));
                            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                            ui.painter().image(icon.id(), badge, uv, p.danger);
                        }
                        music.on_hover_ui(|ui| {
                            ui.set_max_width(300.0);
                            ui.label(&music_tip);
                            warning_line(ui, p, icons, text, 12.0);
                        })
                    }
                    None => music.on_hover_text(&music_tip),
                };
                if music.clicked() && media.music_available {
                    action = AudioAction::ToggleMusic;
                }
            }
            if only.is_none_or(|control| control == TopControl::Media) {
                // media: all media on / off on click, the nearby media panel on
                // hover (no tooltip, as for the speaker: the panel shows there)
                let med = slot_button(
                    ui,
                    p,
                    icons,
                    "brand-youtube-filled",
                    media.media_playing,
                    media.media_available,
                    slot,
                );
                if med.clicked() && media.media_available {
                    action = AudioAction::ToggleMedia;
                }
                if ui.is_enabled() {
                    st.media_button = Some(med.rect);
                    st.media_button_hovered = med.contains_pointer();
                }
            }
            sp
        })
        .inner;

    let Some(sp) = sp else { return action };
    if !ui.is_enabled() {
        return action;
    }
    let on_icon = sp.contains_pointer() || demo.is_some();
    if on_icon {
        st.hover(true, false, false, 0.0);
    }
    if st.panel_open {
        let id = egui::Id::unique("volume_panel");
        let wanted = ui.ctx().memory(|memory| memory.area_rect(id)).map_or(250.0, |rect| rect.height());
        let (anchor_y, above, _) = super::widgets::popup_vertical(ui.ctx().content_rect(), sp.rect, wanted, 6.0);
        let anchor = egui::pos2(sp.rect.right() + 4.0, anchor_y);
        let area = egui::Area::new(id)
            .fade_in(false)
            .order(egui::Order::Foreground)
            .pivot(if above {
                egui::Align2::RIGHT_BOTTOM
            } else {
                egui::Align2::RIGHT_TOP
            })
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
    fn independently_placed_media_retains_its_hover_anchor_and_edit_previews_do_not_register_one() {
        for enabled in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let p = crate::theme::Theme::default().palette();
            let mut state = AudioUi::default();
            let mut audio = AudioSettings::default();
            let icons = Icons::default();
            let _ = ctx.run_ui(Default::default(), |ui| {
                if !enabled {
                    ui.disable();
                }
                top_control(
                    ui,
                    &p,
                    &icons,
                    &mut audio,
                    &mut state,
                    MediaState::default(),
                    Vec2::splat(32.0),
                    TopControl::Media,
                );
                let media = state.media_button;
                for control in [TopControl::Music, TopControl::Volume] {
                    top_control(
                        ui,
                        &p,
                        &icons,
                        &mut audio,
                        &mut state,
                        MediaState::default(),
                        Vec2::splat(32.0),
                        control,
                    );
                    assert_eq!(state.media_button, media, "other audio controls retain the media anchor");
                }
            });
            assert_eq!(state.media_button.is_some(), enabled);
            if let Some(rect) = state.media_button {
                assert!(rect.is_positive());
            }
        }
    }

    #[test]
    fn parcel_music_status_distinguishes_intent_consent_and_engine_playback() {
        use crate::media::filter::Verdict;
        use aurora_audio::StreamStatus;
        let mut audio = AudioSettings::default();
        let playing = StreamStatus::Playing {
            title: None,
            certificate: None,
        };
        let failed = StreamStatus::Error(aurora_audio::StreamError::Connection);
        let read = |audio: &AudioSettings, verdict, stream| MusicStatus::read(true, true, audio, false, verdict, stream);
        assert_eq!(
            read(&audio, Verdict::Allow, Some(&StreamStatus::Connecting)),
            MusicStatus::Connecting
        );
        assert_eq!(
            read(&audio, Verdict::Allow, Some(&failed)),
            MusicStatus::Error(aurora_audio::StreamError::Connection)
        );
        assert_eq!(read(&audio, Verdict::Ask, Some(&playing)), MusicStatus::Waiting);
        assert_eq!(read(&audio, Verdict::Deny, Some(&playing)), MusicStatus::Blocked);
        assert_eq!(read(&audio, Verdict::Allow, Some(&playing)), MusicStatus::Playing(None));
        audio.enabled[4] = false;
        assert_eq!(read(&audio, Verdict::Allow, Some(&playing)), MusicStatus::Disabled);
        audio.enabled[4] = true;
        audio.muted[4] = true;
        assert_eq!(read(&audio, Verdict::Allow, Some(&playing)), MusicStatus::Muted);
        audio.muted[4] = false;
        assert_eq!(read(&audio, Verdict::Allow, None), MusicStatus::NoDevice);
        assert_eq!(
            MusicStatus::read(true, false, &audio, false, Verdict::Allow, Some(&playing)),
            MusicStatus::Stopped
        );
        assert_eq!(
            MusicStatus::read(false, true, &audio, false, Verdict::Allow, Some(&playing)),
            MusicStatus::Unavailable
        );
        assert_eq!(
            MusicStatus::read(true, true, &audio, true, Verdict::Allow, Some(&playing)),
            MusicStatus::Muted
        );
    }

    #[test]
    fn a_station_under_a_failed_certificate_plays_with_its_warning() {
        use crate::media::filter::Verdict;
        use aurora_audio::{CertificateIssue, StreamStatus};
        let audio = AudioSettings::default();
        // 2026-11-18 12:00 UTC: the same year in every time zone
        let expired = CertificateIssue::Expired {
            not_after: Some(1_795_003_200),
        };
        let playing = StreamStatus::Playing {
            title: Some("Song".into()),
            certificate: Some(expired),
        };
        let status = MusicStatus::read(true, true, &audio, false, Verdict::Allow, Some(&playing));
        assert_eq!(status, MusicStatus::Playing(Some(expired)));
        assert_eq!(status.certificate(), Some(expired));
        assert_eq!(status.text(), MusicStatus::Playing(None).text(), "it plays: the warning is beside");
        assert_eq!(MusicStatus::Playing(None).certificate(), None);
        assert_eq!(MusicStatus::Connecting.certificate(), None);
        // the date of the expiry, once readable
        assert!(certificate_warning(expired).contains("2026"), "{}", certificate_warning(expired));
        let issues = [
            CertificateIssue::Expired { not_after: None },
            CertificateIssue::NotYetValid,
            CertificateIssue::Untrusted,
            CertificateIssue::WrongName,
            CertificateIssue::Revoked,
            CertificateIssue::Other,
        ];
        let texts: std::collections::HashSet<_> = issues.iter().map(|issue| certificate_warning(*issue)).collect();
        assert_eq!(texts.len(), issues.len(), "each issue has its own words: {texts:?}");
        assert!(!texts.contains(&certificate_warning(expired)));
    }

    #[test]
    fn a_music_error_names_its_reason() {
        use aurora_audio::StreamError;
        let text = MusicStatus::Error(StreamError::HttpStatus(404)).text();
        assert!(text.contains("404"), "{text}");
        let reasons: Vec<_> = [
            StreamError::WebPage,
            StreamError::Playlist,
            StreamError::Tls,
            StreamError::Connection,
        ]
        .into_iter()
        .map(|error| MusicStatus::Error(error).text())
        .collect();
        for (i, text) in reasons.iter().enumerate() {
            assert!(reasons.iter().skip(i + 1).all(|other| other != text), "{reasons:?}");
        }
    }

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
