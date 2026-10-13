//! Preferences › Commands: the chat bar as a command line, Firestorm's
//! "Chat › CmdLine" tab (FSCmdLine* settings).

use super::{group, row, slider, toggle};
use crate::cmdline::ChatCommandSettings;
use crate::theme::Palette;
use egui::RichText;

/// The name of a chat command, with its usage next to it (Firestorm's
/// "CmdLine" tab: "usage: cmd x y z").
fn command_name(ui: &mut egui::Ui, p: &Palette, label: &str, usage: &str, hint: &str, name: &mut String) -> bool {
    row(ui, p, label, hint, |ui| {
        let c = ui.add(egui::TextEdit::singleline(name).desired_width(110.0)).changed();
        ui.label(RichText::new(usage).size(11.5).color(p.muted));
        c
    })
}

/// The page (panel_preferences_chat.xml, tab-CmdLine).
pub fn page(ui: &mut egui::Ui, p: &Palette, s: &mut ChatCommandSettings) -> bool {
    let mut c = false;
    group(ui, p, aurora_i18n::tr!("common-command-line"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-commands-chat-bar"),
            aurora_i18n::tr!("ui-options-commands-use-the-chat-bar-for-commands-a-line-starting"),
            |ui| toggle(ui, p, &mut s.enabled),
        );
        ui.add_enabled_ui(s.enabled, |ui| {
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-output-to-a-channel"),
                aurora_i18n::tr!("ui-options-commands-also-send-command-responses-on-this-script-channel-whispered"),
                |ui| {
                    let mut ch = toggle(ui, p, &mut s.announce_to_channel);
                    ch |= ui
                        .add_enabled(
                            s.announce_to_channel,
                            egui::DragValue::new(&mut s.announce_channel)
                                .custom_parser(aurora_i18n::parse_number)
                                .range(-2_147_483_648..=2_147_483_647),
                        )
                        .changed();
                    ch
                },
            );
        });
    });
    ui.add_enabled_ui(s.enabled, |ui| {
        group(ui, p, aurora_i18n::tr!("common-teleport-commands"), |ui| {
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-in-region"),
                "cmd x y z",
                aurora_i18n::tr!("ui-options-commands-teleport-to-a-position-in-the-region"),
                &mut s.pos,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-on-the-ground"),
                "cmd",
                aurora_i18n::tr!("ui-options-commands-teleport-to-ground-below-avatar"),
                &mut s.ground,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-at-altitude"),
                "cmd z",
                aurora_i18n::tr!("ui-options-commands-teleport-to-this-altitude-z-or-z-relative"),
                &mut s.height,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-at-camera"),
                "cmd",
                aurora_i18n::tr!("ui-options-commands-teleport-to-camera-position"),
                &mut s.teleport_to_cam,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-to-an-avatar"),
                aurora_i18n::tr!("ui-options-commands-cmd-name"),
                aurora_i18n::tr!("ui-options-commands-case-insensitive-partial-name-among-nearby-avatars"),
                &mut s.tp2,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("common-offer-teleport"),
                aurora_i18n::tr!("ui-options-commands-cmd-key"),
                aurora_i18n::tr!("ui-options-commands-offer-teleport-to-an-avatar-uuid"),
                &mut s.offer_tp,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("common-home"),
                "cmd",
                aurora_i18n::tr!("ui-options-commands-teleport-home"),
                &mut s.teleport_home,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-to-a-region"),
                aurora_i18n::tr!("ui-options-commands-cmd-region"),
                aurora_i18n::tr!("ui-options-commands-teleport-to-a-region-region-x-y-z-for"),
                &mut s.map_to,
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-keep-position"),
                aurora_i18n::tr!("ui-options-commands-use-the-same-position-across-regions-otherwise-128-128"),
                |ui| toggle(ui, p, &mut s.map_to_keep_pos),
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-track-on-map"),
                aurora_i18n::tr!("status-cmd-x-y-z-name"),
                aurora_i18n::tr!("ui-options-commands-place-a-world-map-beacon-at-a-position-cpcampos"),
                &mut s.track_pos,
            );
        });
        group(ui, p, aurora_i18n::tr!("common-camera-and-display-commands"), |ui| {
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("common-draw-distance"),
                aurora_i18n::tr!("ui-options-commands-cmd-meters"),
                aurora_i18n::tr!("ui-options-commands-change-draw-distance-n-or-n-relative"),
                &mut s.draw_distance,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-maximum-bandwidth"),
                aurora_i18n::tr!("ui-options-commands-cmd-kbps"),
                aurora_i18n::tr!("common-not-available-in-aurora-yet"),
                &mut s.bandwidth,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-copy-camera"),
                "cmd",
                aurora_i18n::tr!("ui-options-commands-copy-camera-position-to-clipboard"),
                &mut s.copy_cam,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-restore-camera"),
                aurora_i18n::tr!("ui-options-commands-cmd-pos-focus-roll"),
                aurora_i18n::tr!("ui-options-commands-move-the-camera-to-a-copied-position"),
                &mut s.paste_cam,
            );
        });
        group(ui, p, aurora_i18n::tr!("common-other-commands"), |ui| {
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-calculator"),
                "cmd SIN(2+2)",
                aurora_i18n::tr!("ui-options-commands-evaluate-an-expression"),
                &mut s.calc,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-avatar-name"),
                aurora_i18n::tr!("ui-options-commands-cmd-key"),
                aurora_i18n::tr!("ui-options-commands-get-an-avatar-s-name-from-its-uuid"),
                &mut s.key_to_name,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-dice-roll"),
                aurora_i18n::tr!("ui-options-commands-cmd-dice-faces"),
                aurora_i18n::tr!("ui-options-commands-cmd-dice-count-face-count-modifier-type-value-examples"),
                &mut s.roll_dice,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-clear-chat"),
                "cmd",
                aurora_i18n::tr!("ui-options-commands-clear-nearby-chat-to-limit-spam"),
                &mut s.clear_chat,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-media"),
                aurora_i18n::tr!("ui-options-commands-cmd-url-type"),
                aurora_i18n::tr!("ui-options-commands-set-and-play-parcel-media"),
                &mut s.media,
            );
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("common-music"),
                aurora_i18n::tr!("ui-options-commands-cmd-url"),
                aurora_i18n::tr!("ui-options-commands-set-and-play-streaming-music"),
                &mut s.music,
            );
            c |= command_name(ui, p, "AO", "cmd on/off", aurora_i18n::tr!("ao-command-help"), &mut s.ao);
            c |= command_name(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-platform"),
                aurora_i18n::tr!("ui-options-commands-cmd-width"),
                aurora_i18n::tr!("ui-options-commands-create-a-platform-under-your-avatar"),
                &mut s.rez_platform,
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-commands-platform-width"),
                aurora_i18n::tr!("ui-options-commands-width-used-when-the-command-does-not-specify-one"),
                |ui| slider(ui, &mut s.platform_size, 5.0..=64.0, " m"),
            );
        });
    });
    c
}
