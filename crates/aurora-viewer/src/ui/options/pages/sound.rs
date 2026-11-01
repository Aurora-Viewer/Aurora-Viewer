//! Preferences › Sound and voice: devices and volumes (the same settings
//! as the panel of the top right, so both stay in sync), the interface
//! sounds (FSPanelPreferenceUISounds, fspanelpreferenceuisounds.cpp), the
//! voice and the microphone.

use super::super::cx::{Custom, Cx, Nothing, field};
use super::super::{SLIDER_W, binding_cell, device_combo, parse_uuid_text, uuid_text};
use crate::ui::widgets;
use egui::Vec2;

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

pub fn page(cx: &mut Cx, tab: usize) {
    match tab {
        0 => volumes(cx),
        1 => interface_sounds(cx),
        _ => voice(cx),
    }
    // the streams the music menu remembers: the page's reset clears them,
    // as the reset of the former Sound and voice page did
    cx.track(&field!(audio.saved_streams));
    // a binding cleared by a right click (the push-to-talk key)
    if let Some((a, slot)) = cx.st.clear.take() {
        cx.s.keybinds.set(a, slot, None);
        cx.changed = true;
    }
}

fn volumes(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-devices"), |cx| {
        cx.row_with(
            tr!("ui-options-audio-output"),
            tr!("ui-options-speakers-or-headset-used-by-the-viewer"),
            &field!(audio.output_device),
            |ui, s, st, _| device_combo(ui, "audio_out", &mut s.audio.output_device, &st.output_devices, SLIDER_W + 40.0).0,
        );
        cx.row_with(
            tr!("ui-options-microphone"),
            tr!("ui-options-input-used-for-voice"),
            &field!(audio.input_device),
            |ui, s, st, _| device_combo(ui, "audio_in", &mut s.audio.input_device, &st.input_devices, SLIDER_W + 40.0).0,
        );
        cx.row_with(tr!("ui-options-device-list"), "", &Nothing, |ui, _, _, r| {
            if widgets::flat_button(ui, &p, tr!("ui-options-refresh-list")).clicked() {
                r.refresh_devices = true;
            }
            false
        });
    });
    cx.group(tr!("ui-options-volumes"), |cx| {
        for (i, name) in crate::settings::audio_channels().iter().enumerate() {
            let t = Custom {
                path: "audio.volume",
                modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| {
                    s.audio.volume[i] != d.audio.volume[i]
                        || s.audio.muted[i] != d.audio.muted[i]
                        || s.audio.enabled[i] != d.audio.enabled[i]
                },
                reset: move |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
                    s.audio.volume[i] = d.audio.volume[i];
                    s.audio.muted[i] = d.audio.muted[i];
                    s.audio.enabled[i] = d.audio.enabled[i];
                },
            };
            cx.wide().row(name, "", &t, |ui, s| {
                let a = &mut s.audio;
                let mut ch = false;
                ui.spacing_mut().slider_width = SLIDER_W + 40.0;
                // this page's own copy of the setting: the sliders follow a
                // click on the switch below at once
                let enabled = !a.muted[i] && a.enabled[i];
                ch |= widgets::volume_slider_with(
                    ui,
                    &mut a.volume[i],
                    widgets::VolumeScale::Gain,
                    widgets::perceptual_override().unwrap_or(a.perceptual_volume),
                    enabled,
                    |s| s.custom_parser(aurora_i18n::parse_number).trailing_fill(true).show_value(false),
                )
                .changed();
                // the percentage in a fixed column, painted from its right
                // edge: its text never moves the row (a font without even
                // digits, OpenDyslexic, made " 80 %" wider than "100 %")
                let font = egui::FontId::proportional(12.0);
                let percent_width = ui.painter().layout_no_wrap("100 %".to_owned(), font.clone(), p.muted).size().x;
                let (cell, _) = ui.allocate_exact_size(egui::vec2(percent_width, ui.spacing().interact_size.y), egui::Sense::hover());
                ui.painter().text(
                    cell.right_center(),
                    egui::Align2::RIGHT_CENTER,
                    format!("{:.0} %", a.volume[i] * 100.0),
                    font,
                    p.muted,
                );
                let mut on = !a.muted[i];
                if widgets::switch(ui, &p, &mut on)
                    .on_hover_text(if on { tr!("common-cut") } else { tr!("ui-options-restore") })
                    .changed()
                {
                    a.muted[i] = !on;
                    ch = true;
                }
                if i > 0 {
                    ch |= widgets::checkbox(ui, &p, &mut a.enabled[i], "")
                        .on_hover_text(tr!("ui-options-enable-this-channel"))
                        .changed();
                } else {
                    // the master has no enable box: its room kept, so that
                    // its slider lines up with the channels' (the row is
                    // laid out from the right)
                    ui.scope(|ui| {
                        ui.set_invisible();
                        widgets::checkbox(ui, &p, &mut true, "");
                    });
                }
                ch
            });
        }
        cx.toggle(
            tr!("ui-options-perceptual-volume"),
            tr!("ui-options-perceptual-volume-hint"),
            &field!(audio.perceptual_volume),
        );
    });
    cx.group(tr!("ui-options-music-and-sounds"), |cx| {
        cx.toggle(
            tr!("ui-options-auto-music"),
            tr!("ui-options-play-parcel-music-when-entering"),
            &field!(audio.music_autoplay),
        );
        // Legacy Viewer › Sound & Media: ShowStreamMetadata,
        // StreamMetadataAnnounceToChat / StreamMetadataAnnounceChannel
        use crate::settings::StreamTitleShow;
        cx.choice(
            tr!("ui-options-stream-title"),
            tr!("ui-options-stream-title-help"),
            &field!(audio.stream_title),
            &[
                (StreamTitleShow::Off, tr!("ui-options-stream-title-off"), false),
                (StreamTitleShow::Toast, tr!("ui-options-stream-title-toast"), false),
                (StreamTitleShow::Chat, tr!("ui-options-stream-title-chat"), false),
            ],
        );
        cx.toggle(
            tr!("ui-options-stream-title-announce"),
            tr!("ui-options-stream-title-announce-help"),
            &field!(audio.stream_title_announce),
        );
        let off = !cx.s.audio.stream_title_announce;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-stream-title-channel"),
                "",
                &field!(audio.stream_title_channel),
                |ui, s| {
                    ui.add(
                        egui::DragValue::new(&mut s.audio.stream_title_channel)
                            .custom_parser(aurora_i18n::parse_number)
                            .range(1..=16_777_216)
                            .speed(1.0),
                    )
                    .changed()
                },
            );
        });
        cx.toggle(
            tr!("ui-options-collision-sounds"),
            tr!("ui-options-collisions-between-objects-and-avatars-as-in-firestorm-enabled"),
            &field!(audio.collision_sounds),
        );
        cx.toggle(
            tr!("ui-options-gesture-sounds"),
            tr!("ui-options-sounds-played-by-other-avatars-gestures"),
            &field!(audio.gesture_sounds),
        );
        // Aurora's own (scene/sound_occlusion.rs): off, as in the Legacy Viewer
        cx.toggle(
            tr!("ui-options-sound-occlusion"),
            tr!("ui-options-sound-occlusion-help"),
            &field!(audio.sound_occlusion),
        );
        // Aurora's own (aurora-audio/src/hrtf.rs): off pans the sounds
        // between the ears, as in the Legacy Viewer
        cx.toggle(
            tr!("ui-options-sound-hrtf"),
            tr!("ui-options-sound-hrtf-help"),
            &field!(audio.sound_hrtf),
        );
        // Legacy Viewer › Sound & Media (MuteWhenMinimized)
        cx.toggle(
            tr!("ui-options-mute-when-minimized"),
            tr!("ui-options-mute-when-minimized-help"),
            &field!(audio.mute_when_minimized),
        );
    });
}

/// A sound's play switch. The shutter's is the photos' own setting, as the
/// Legacy Viewer's UISndSnapshot row is PlayModeUISndSnapshot inverted
/// (fspanelpreferenceuisounds.cpp): one switch for "Quiet snapshots". A
/// conversation sound's is its mode (Never or not).
fn ui_sound_plays(s: &crate::settings::Settings, sound: crate::ui_sound::UiSound) -> bool {
    match sound {
        crate::ui_sound::UiSound::Snapshot => s.snapshot.sound,
        _ => s.audio.ui.im_plays(sound).unwrap_or_else(|| s.audio.ui.plays(sound)),
    }
}

fn set_ui_sound_plays(s: &mut crate::settings::Settings, sound: crate::ui_sound::UiSound, on: bool) {
    match sound {
        crate::ui_sound::UiSound::Snapshot => s.snapshot.sound = on,
        _ if s.audio.ui.im_plays(sound).is_some() => s.audio.ui.set_im_plays(sound, on),
        _ => s.audio.ui.set_plays(sound, on),
    }
}

/// Each sound on / off, its asset and a preview, the IM modes and the L$
/// threshold.
fn interface_sounds(cx: &mut Cx) {
    use crate::ui_sound::{CATALOG, ImSoundMode, UiSound};
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-interface-sounds"), |cx| {
        cx.note(tr!("ui-options-same-sounds-and-defaults-as-firestorm-these-are-second"));
        cx.toggle(
            tr!("ui-options-interface-sounds"),
            tr!("ui-options-all-interface-sounds-clicks-windows-conversations-l-teleportation-menus"),
            &field!(audio.ui_sounds),
        );
        cx.row(
            tr!("ui-options-l-sound-threshold"),
            tr!("ui-options-balance-change-above-which-received-spent-l-sounds-are"),
            &field!(audio.ui.money_threshold),
            |ui, s| {
                ui.add(
                    egui::DragValue::new(&mut s.audio.ui.money_threshold)
                        .custom_parser(aurora_i18n::parse_number)
                        .range(0.0..=100_000.0)
                        .speed(1.0)
                        .suffix(" L$"),
                )
                .changed()
            },
        );
        let modes = [
            (
                tr!("ui-options-private-messages"),
                "PlayModeUISndNewIncomingIMSession",
                field!(audio.ui.im_mode),
                UiSound::NewIncomingImSession,
            ),
            (
                tr!("ui-options-group-messages"),
                "PlayModeUISndNewIncomingGroupIMSession",
                field!(audio.ui.group_mode),
                UiSound::NewIncomingGroupImSession,
            ),
            (
                tr!("ui-options-conferences"),
                "PlayModeUISndNewIncomingConfIMSession",
                field!(audio.ui.conf_mode),
                UiSound::NewIncomingConfImSession,
            ),
        ];
        for (label, hint, f, sound) in modes {
            let get = f.get_mut;
            let words: Vec<&str> = ImSoundMode::ALL.iter().map(|m| m.label()).collect();
            cx.wide().keywords(&words.join(" ")).row(label, hint, &f, |ui, s| {
                let mut ch = false;
                let mode = get(s);
                egui::ComboBox::from_id_salt(ui.scope_id().with("mode"))
                    .width(SLIDER_W + 90.0)
                    .selected_text(mode.label())
                    .show_ui(ui, |ui| {
                        for m in ImSoundMode::ALL {
                            ch |= ui.selectable_value(mode, m, m.label()).changed();
                        }
                    });
                if widgets::flat_button(ui, &p, tr!("ui-options-listen")).clicked()
                    && let Some(id) = s.audio.ui.resolve(sound, true)
                {
                    crate::ui::sound_cues::preview(ui.ctx(), id);
                }
                ch
            });
        }
        // every sound one by one, behind the cog; the three conversation
        // sounds too, with their asset, as the UI sounds panel lists them
        // (their switch turns their mode above to Never and back)
        cx.row(
            tr!("ui-options-per-sound-choices"),
            tr!("ui-options-per-sound-choices-help"),
            &Nothing,
            |_, _| false,
        );
        cx.subs(false, |cx| {
            for e in CATALOG.iter() {
                let hint = tr!(
                    "extra-uisnd-sound-asset-empty-no-sound-playmodeuisnd",
                    value0 = e.name.to_string(),
                    value1 = e.name.to_string()
                );
                let sound = e.sound;
                let t = Custom {
                    path: "audio.ui",
                    modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| {
                        ui_sound_plays(s, sound) != ui_sound_plays(d, sound) || s.audio.ui.uuid(sound) != d.audio.ui.uuid(sound)
                    },
                    reset: move |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
                        let (on, id) = (ui_sound_plays(d, sound), d.audio.ui.uuid(sound));
                        set_ui_sound_plays(s, sound, on);
                        s.audio.ui.set_uuid(sound, id);
                    },
                };
                cx.wide().keywords(e.name).row_with((e.label)(), &hint, &t, |ui, s, st, _| {
                    let mut on = ui_sound_plays(s, e.sound);
                    let mut ch = widgets::switch(ui, &p, &mut on).changed();
                    if ch {
                        set_ui_sound_plays(s, e.sound, on);
                    }
                    let u = &mut s.audio.ui;
                    let text = st.ui_sound_text.entry(e.name).or_insert_with(|| uuid_text(u.uuid(e.sound)));
                    let parsed = parse_uuid_text(text);
                    let edit = ui.add(
                        egui::TextEdit::singleline(text)
                            .desired_width(290.0)
                            .font(egui::TextStyle::Monospace)
                            .text_color(if parsed.is_some() { p.ink } else { p.danger })
                            .hint_text(tr!("ui-options-no-sound")),
                    );
                    if edit.changed()
                        && let Some(id) = parse_uuid_text(text)
                    {
                        u.set_uuid(e.sound, id);
                        ch = true;
                    }
                    // force_sound: heard even when switched off
                    if widgets::flat_button(ui, &p, tr!("ui-options-listen")).clicked()
                        && let Some(id) = u.resolve(e.sound, true)
                    {
                        crate::ui::sound_cues::preview(ui.ctx(), id);
                    }
                    if u.uuid(e.sound) != e.uuid && widgets::flat_button(ui, &p, tr!("common-default")).clicked() {
                        u.set_uuid(e.sound, e.uuid);
                        *text = uuid_text(e.uuid);
                        ch = true;
                    }
                    ch
                });
            }
        });
    });
}

fn voice(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("common-voice"), |cx| {
        cx.row_with(
            tr!("ui-options-enable-voice"),
            tr!("ui-options-connect-to-regional-voice-chat-webrtc"),
            &field!(audio.voice_enabled),
            |ui, s, st, _| {
                let c = super::super::toggle(ui, &p, &mut s.audio.voice_enabled);
                if widgets::action_button(
                    ui,
                    &p,
                    "refresh",
                    tr!("voice-restart"),
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
        use crate::settings::VoiceEar;
        cx.choice(
            tr!("voice-listen-from"),
            tr!("voice-local-only"),
            &field!(audio.voice_ear),
            &[
                (VoiceEar::Camera, tr!("common-camera"), false),
                (VoiceEar::Avatar, tr!("voice-ear-avatar"), false),
            ],
        );
        cx.choice(
            tr!("ui-options-microphone-button"),
            tr!("ui-options-toggle-click-once-to-enable-again-to-disable-hold"),
            &field!(audio.mic_hold),
            &[(false, tr!("ui-options-toggle"), false), (true, tr!("common-hold-to-talk"), false)],
        );
        let push = crate::keybinds::Action::PushToTalk;
        let t = Custom {
            path: "keybinds",
            modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| s.keybinds.get(push) != d.keybinds.get(push),
            reset: move |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
                for (slot, binding) in d.keybinds.get(push).into_iter().enumerate() {
                    s.keybinds.set(push, slot, binding);
                }
            },
        };
        cx.wide().row_with(
            tr!("ui-options-push-to-talk-key"),
            tr!("ui-options-key-or-mouse-button-middle-click-by-default-also"),
            &t,
            |ui, s, st, _| {
                let kb = s.keybinds.clone();
                for slot in 0..2 {
                    binding_cell(ui, &p, &kb, st, push, slot);
                }
                false
            },
        );
        cx.slider(tr!("ui-options-microphone-gain"), "", &field!(audio.mic_gain), 0.0..=2.0, "×");
        cx.row_with(
            tr!("ui-options-microphone-level"),
            tr!("ui-options-speak-to-test"),
            &Nothing,
            |ui, _, st, _| {
                let lvl = st.mic_level.clamp(0.0, 1.0);
                let col = if lvl > 0.85 {
                    p.danger
                } else if lvl > 0.6 {
                    p.amber
                } else {
                    p.teal
                };
                widgets::level(ui, &p, "mic_level", lvl, SLIDER_W, col);
                false
            },
        );
    });
    // VoiceCallsRejectGroup / AdHoc / P2P (panel_preferences_sound.xml)
    cx.group(tr!("ui-options-calls"), |cx| {
        cx.toggle(
            tr!("ui-options-reject-group-calls"),
            tr!("ui-options-reject-group-calls-hint"),
            &field!(audio.reject_group_calls),
        );
        cx.toggle(
            tr!("ui-options-reject-adhoc-calls"),
            tr!("ui-options-reject-adhoc-calls-hint"),
            &field!(audio.reject_adhoc_calls),
        );
        cx.toggle(
            tr!("ui-options-reject-p2p-calls"),
            tr!("ui-options-reject-p2p-calls-hint"),
            &field!(audio.reject_p2p_calls),
        );
    });
    cx.group(tr!("voice-processing"), |cx| {
        cx.toggle(
            tr!("voice-automatic-gain"),
            tr!("voice-automatic-gain-hint"),
            &field!(audio.voice_automatic_gain),
        );
        cx.toggle(
            tr!("voice-echo-cancellation"),
            tr!("voice-echo-cancellation-hint"),
            &field!(audio.voice_echo_cancellation),
        );
        cx.row(
            tr!("voice-noise-suppression"),
            tr!("voice-noise-suppression-hint"),
            &field!(audio.voice_noise_suppression),
            |ui, s| noise_choice(ui, "voice_noise", &mut s.audio.voice_noise_suppression, SLIDER_W + 40.0),
        );
    });
    cx.group(tr!("voice-indicators"), |cx| {
        cx.toggle(tr!("voice-show-visualizers"), "", &field!(audio.voice_visualizer));
        cx.toggle(tr!("voice-show-self"), "", &field!(audio.voice_visualizer_self));
        cx.toggle(tr!("voice-show-dot"), "", &field!(audio.voice_visualizer_dot));
        cx.toggle(tr!("voice-show-in-calls"), "", &field!(audio.voice_visualizers_in_calls));
        cx.toggle(tr!("voice-show-conversation"), "", &field!(audio.conversation_voice_indicator));
    });
}
