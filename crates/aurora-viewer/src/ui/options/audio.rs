//! Sound and voice preferences and microphone quick controls.

use super::super::widgets;
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AudioSection {
    Voice,
    Processing,
    Indicators,
}

fn section(ui: &mut egui::Ui, st: &mut OptionsUi, section: AudioSection) {
    if st.audio_section == Some(section) {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
        st.audio_section = None;
    }
}

pub(crate) fn ear_choice(ui: &mut egui::Ui, value: &mut crate::settings::VoiceEar) -> bool {
    use crate::settings::VoiceEar;
    ui.selectable_value(value, VoiceEar::Camera, aurora_i18n::tr!("common-camera"))
        .changed()
        | ui.selectable_value(value, VoiceEar::Avatar, aurora_i18n::tr!("voice-ear-avatar"))
            .changed()
}

pub(crate) fn noise_choice(ui: &mut egui::Ui, id: &str, value: &mut u8, width: f32) -> bool {
    let labels = [
        aurora_i18n::tr!("voice-noise-off"),
        aurora_i18n::tr!("voice-noise-low"),
        aurora_i18n::tr!("voice-noise-moderate"),
        aurora_i18n::tr!("voice-noise-high"),
        aurora_i18n::tr!("voice-noise-max"),
    ];
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(labels[usize::from((*value).min(4))])
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                changed |= ui.selectable_value(value, i as u8, *label).changed();
            }
        });
    changed
}

/// "Sound and voice": devices, the volumes of the top-right panel (same
/// settings, so both stay in sync), voice and microphone behaviour.
pub(super) fn page(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut Settings,
    st: &mut OptionsUi,
    audio_changed: &mut bool,
    refresh: &mut bool,
) -> bool {
    let mut c = false;
    let a = &mut s.audio;
    group(ui, p, aurora_i18n::tr!("ui-options-devices"), |ui| {
        let out = row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-audio-output"),
            aurora_i18n::tr!("ui-options-speakers-or-headset-used-by-the-viewer"),
            |ui| device_combo(ui, "audio_out", &mut a.output_device, &st.output_devices, SLIDER_W).0,
        );
        let inp = row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-microphone"),
            aurora_i18n::tr!("ui-options-input-used-for-voice"),
            |ui| device_combo(ui, "audio_in", &mut a.input_device, &st.input_devices, SLIDER_W).0,
        );
        c |= out | inp;
        *audio_changed |= out | inp;
        row(ui, p, "", "", |ui| {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-refresh-list")).clicked() {
                *refresh = true;
            }
            false
        });
    });
    group(ui, p, aurora_i18n::tr!("ui-options-volumes"), |ui| {
        for (i, name) in crate::settings::audio_channels().iter().enumerate() {
            let changed = row(ui, p, name, "", |ui| {
                let mut ch = false;
                ui.spacing_mut().slider_width = SLIDER_W - 40.0;
                ch |= ui
                    .add_enabled(
                        !a.muted[i] && a.enabled[i],
                        egui::Slider::new(&mut a.volume[i], 0.0..=1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .trailing_fill(true)
                            .show_value(false),
                    )
                    .changed();
                ui.label(RichText::new(format!("{:>3.0} %", a.volume[i] * 100.0)).size(12.0).color(p.muted));
                let mut on = !a.muted[i];
                if widgets::switch(ui, p, &mut on)
                    .on_hover_text(if on {
                        aurora_i18n::tr!("common-cut")
                    } else {
                        aurora_i18n::tr!("ui-options-restore")
                    })
                    .changed()
                {
                    a.muted[i] = !on;
                    ch = true;
                }
                if i > 0 {
                    ch |= widgets::checkbox(ui, p, &mut a.enabled[i], "")
                        .on_hover_text(aurora_i18n::tr!("ui-options-enable-this-channel"))
                        .changed();
                }
                ch
            });
            c |= changed;
            *audio_changed |= changed;
        }
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-auto-music"),
            aurora_i18n::tr!("ui-options-play-parcel-music-when-entering"),
            |ui| toggle(ui, p, &mut a.music_autoplay),
        );
        // Legacy Viewer › Sound & Media: ShowStreamMetadata,
        // StreamMetadataAnnounceToChat / StreamMetadataAnnounceChannel
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-stream-title"),
            aurora_i18n::tr!("ui-options-stream-title-help"),
            |ui| {
                use crate::settings::StreamTitleShow;
                let options = [
                    (aurora_i18n::tr!("ui-options-stream-title-off"), false),
                    (aurora_i18n::tr!("ui-options-stream-title-toast"), false),
                    (aurora_i18n::tr!("ui-options-stream-title-chat"), false),
                ];
                let mut i = match a.stream_title {
                    StreamTitleShow::Off => 0,
                    StreamTitleShow::Toast => 1,
                    StreamTitleShow::Chat => 2,
                };
                let changed = widgets::segmented(ui, p, &mut i, &options);
                a.stream_title = [StreamTitleShow::Off, StreamTitleShow::Toast, StreamTitleShow::Chat][i.min(2)];
                changed
            },
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-stream-title-announce"),
            aurora_i18n::tr!("ui-options-stream-title-announce-help"),
            |ui| {
                let mut changed = toggle(ui, p, &mut a.stream_title_announce);
                ui.add_enabled_ui(a.stream_title_announce, |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut a.stream_title_channel)
                                .custom_parser(aurora_i18n::parse_number)
                                .range(1..=16_777_216)
                                .speed(1.0),
                        )
                        .on_hover_text(aurora_i18n::tr!("ui-options-stream-title-channel"))
                        .changed();
                });
                changed
            },
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-collision-sounds"),
            aurora_i18n::tr!("ui-options-collisions-between-objects-and-avatars-as-in-firestorm-enabled"),
            |ui| toggle(ui, p, &mut a.collision_sounds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-gesture-sounds"),
            aurora_i18n::tr!("ui-options-sounds-played-by-other-avatars-gestures"),
            |ui| toggle(ui, p, &mut a.gesture_sounds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-interface-sounds"),
            aurora_i18n::tr!("ui-options-all-interface-sounds-clicks-windows-conversations-l-teleportation-menus"),
            |ui| toggle(ui, p, &mut a.ui_sounds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-auto-media"),
            aurora_i18n::tr!("ui-options-auto-play-parcel-media-after-standing-still-for-5"),
            |ui| toggle(ui, p, &mut a.media_autoplay),
        );
    });
    c |= ui_sounds_group(ui, p, a, st);
    section(ui, st, AudioSection::Voice);
    group(ui, p, aurora_i18n::tr!("common-voice"), |ui| {
        let v = row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-enable-voice"),
            aurora_i18n::tr!("ui-options-connect-to-regional-voice-chat-webrtc"),
            |ui| {
                let c = toggle(ui, p, &mut a.voice_enabled);
                if widgets::action_button(
                    ui,
                    p,
                    "refresh",
                    aurora_i18n::tr!("voice-restart"),
                    widgets::ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                )
                .clicked()
                {
                    st.restart_voice = true;
                }
                c
            },
        );
        c |= v;
        *audio_changed |= v;
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("voice-listen-from"),
            aurora_i18n::tr!("voice-local-only"),
            |ui| ear_choice(ui, &mut a.voice_ear),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-microphone-button"),
            aurora_i18n::tr!("ui-options-toggle-click-once-to-enable-again-to-disable-hold"),
            |ui| {
                let mut ch = false;
                ch |= ui
                    .selectable_value(&mut a.mic_hold, false, aurora_i18n::tr!("ui-options-toggle"))
                    .changed();
                ch |= ui
                    .selectable_value(&mut a.mic_hold, true, aurora_i18n::tr!("common-hold-to-talk"))
                    .changed();
                ch
            },
        );
        let g = row(ui, p, aurora_i18n::tr!("ui-options-microphone-gain"), "", |ui| {
            slider(ui, &mut a.mic_gain, 0.0..=2.0, "×")
        });
        c |= g;
        *audio_changed |= g;
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-microphone-level"),
            aurora_i18n::tr!("ui-options-speak-to-test"),
            |ui| {
                let lvl = st.mic_level.clamp(0.0, 1.0);
                let col = if lvl > 0.85 {
                    p.danger
                } else if lvl > 0.6 {
                    p.amber
                } else {
                    p.teal
                };
                widgets::level(ui, p, "mic_level", lvl, SLIDER_W, col);
                false
            },
        );
        // VoiceCallsRejectGroup / AdHoc / P2P (panel_preferences_sound.xml)
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-reject-group-calls"),
            aurora_i18n::tr!("ui-options-reject-group-calls-hint"),
            |ui| toggle(ui, p, &mut a.reject_group_calls),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-reject-adhoc-calls"),
            aurora_i18n::tr!("ui-options-reject-adhoc-calls-hint"),
            |ui| toggle(ui, p, &mut a.reject_adhoc_calls),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-reject-p2p-calls"),
            aurora_i18n::tr!("ui-options-reject-p2p-calls-hint"),
            |ui| toggle(ui, p, &mut a.reject_p2p_calls),
        );
    });
    section(ui, st, AudioSection::Processing);
    group(ui, p, aurora_i18n::tr!("voice-processing"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("voice-automatic-gain"),
            aurora_i18n::tr!("voice-automatic-gain-hint"),
            |ui| toggle(ui, p, &mut a.voice_automatic_gain),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("voice-echo-cancellation"),
            aurora_i18n::tr!("voice-echo-cancellation-hint"),
            |ui| toggle(ui, p, &mut a.voice_echo_cancellation),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("voice-noise-suppression"),
            aurora_i18n::tr!("voice-noise-suppression-hint"),
            |ui| noise_choice(ui, "voice_noise", &mut a.voice_noise_suppression, SLIDER_W),
        );
    });
    section(ui, st, AudioSection::Indicators);
    group(ui, p, aurora_i18n::tr!("voice-indicators"), |ui| {
        for (key, setting) in [
            ("voice-lip-sync", &mut a.lip_sync),
            ("voice-show-visualizers", &mut a.voice_visualizer),
            ("voice-show-self", &mut a.voice_visualizer_self),
            ("voice-show-dot", &mut a.voice_visualizer_dot),
            ("voice-show-in-calls", &mut a.voice_visualizers_in_calls),
            ("voice-show-conversation", &mut a.conversation_voice_indicator),
        ] {
            c |= row(ui, p, aurora_i18n::text(key), "", |ui| toggle(ui, p, setting));
        }
    });
    group(ui, p, aurora_i18n::tr!("ui-options-push-to-talk"), |ui| {
        let kb = s.keybinds.clone();
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-push-to-talk-key"),
            aurora_i18n::tr!("ui-options-key-or-mouse-button-middle-click-by-default-also"),
            |ui| {
                for slot in 0..2 {
                    binding_cell(ui, p, &kb, st, crate::keybinds::Action::PushToTalk, slot);
                }
                false
            },
        );
    });
    let m = &mut s.media;
    group(ui, p, aurora_i18n::tr!("ui-options-media-web-and-video"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-enable-media"),
            aurora_i18n::tr!("ui-options-parcel-and-prim-web-pages-and-videos-audiostreamingmedia"),
            |ui| toggle(ui, p, &mut m.enabled),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-prim-media"),
            aurora_i18n::tr!("ui-options-shared-media-displayed-on-object-faces-primmediamasterenabled"),
            |ui| toggle(ui, p, &mut m.prim_media),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-on-other-avatars"),
            aurora_i18n::tr!("ui-options-media-on-other-people-s-attachments-mediashowonothers-disabled-by"),
            |ui| toggle(ui, p, &mut m.show_on_others),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-first-click-interaction"),
            aurora_i18n::tr!("ui-options-the-first-click-goes-directly-to-the-page-when"),
            |ui| toggle(ui, p, &mut m.first_click_interact),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-hud-media"),
            aurora_i18n::tr!("ui-options-auto-play-hud-media-mediaautoplayhuds"),
            |ui| toggle(ui, p, &mut m.autoplay_huds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-scripted-media"),
            aurora_i18n::tr!("ui-options-allow-scripts-to-start-parcel-media-permallowscriptedmedia-disabled-by"),
            |ui| toggle(ui, p, &mut m.allow_scripted),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-simultaneous-players"),
            aurora_i18n::tr!("ui-options-maximum-pages-videos-loaded-at-once-plugininstancestotal"),
            |ui| {
                ui.spacing_mut().slider_width = SLIDER_W - 40.0;
                let mut n = m.max_instances as u32;
                let ch = ui
                    .add(
                        egui::Slider::new(&mut n, 1..=16)
                            .custom_parser(aurora_i18n::parse_number)
                            .trailing_fill(true),
                    )
                    .changed();
                m.max_instances = n as usize;
                ch
            },
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-plugins-folder"),
            aurora_i18n::tr!("ui-options-folder-containing-llplugin-blank-aurora-s-own-plugins"),
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut m.plugin_dir)
                        .desired_width(SLIDER_W)
                        .hint_text(aurora_i18n::tr!("ui-options-plugins-folder-automatic")),
                )
                .changed()
            },
        );
        use crate::media::pack::PackState;
        let megabytes = |bytes: u64| aurora_i18n::integer((bytes / (1024 * 1024)) as i64);
        // plugins found: their folder; otherwise where Aurora's pack stands
        let (status, download, retry) = if !st.media_plugins.is_empty() {
            let text = aurora_i18n::tr!(
                "status-active-player-s",
                value0 = st.media_plugins.to_string(),
                value1 = format!("{}", st.media_running)
            );
            (text, None, false)
        } else {
            match st.media_pack {
                PackState::Downloading(d) if !d.preparing => {
                    let text = aurora_i18n::tr!(
                        "media-plugins-downloading",
                        received = megabytes(d.received),
                        total = megabytes(d.total)
                    );
                    (text, Some(d.received as f32 / d.total.max(1) as f32), false)
                }
                PackState::Downloading(_) => (aurora_i18n::tr!("media-plugins-installing").to_string(), Some(1.0), false),
                PackState::Failed(failure) => (media_pack_error(failure).to_string(), None, true),
                PackState::Idle | PackState::Ready => (aurora_i18n::tr!("media-plugins-missing").to_string(), None, false),
            }
        };
        row(ui, p, aurora_i18n::tr!("ui-options-plugins"), "", |ui| {
            if let Some(fraction) = download {
                widgets::progress(ui, p, "media_pack", fraction, SLIDER_W, &status, None);
                return false;
            }
            if retry
                && ui
                    .button(RichText::new(aurora_i18n::tr!("media-plugins-retry")).size(12.0))
                    .clicked()
            {
                st.media_pack_retry = true;
            }
            // the path of a folder, of any length: cut to the row (whole on
            // hover), it made the window wider on this tab
            let room = ui.available_width();
            ui.add(widgets::elided_label(ui, RichText::new(status).size(12.0).color(p.muted), room));
            false
        });
    });
    if let Some((a, slot)) = st.clear.take() {
        s.keybinds.set(a, slot, None);
        c = true;
    }
    c
}
