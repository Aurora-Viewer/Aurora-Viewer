//! Preferences › Commands: the chat bar as a command line, the Legacy
//! Viewer's "Chat › CmdLine" tab (FSCmdLine* settings).

use super::super::cx::{Cx, F, field};
use egui::RichText;

/// The name of a chat command, with its usage next to it ("usage: cmd x
/// y z").
fn command(cx: &mut Cx, label: &str, usage: &str, hint: &str, f: &F<String>) {
    let (p, get) = (cx.p, f.get_mut);
    // found by its name too ("gtp")
    let name = (f.get)(cx.s).clone();
    cx.keywords(&format!("{usage} {name}")).row(label, hint, f, |ui, s| {
        ui.label(RichText::new(usage).size(11.5).color(p.muted));
        ui.add(egui::TextEdit::singleline(get(s)).desired_width(124.0)).changed()
    });
}

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("common-command-line"), |cx| {
        cx.toggle(
            tr!("ui-options-commands-chat-bar"),
            tr!("ui-options-commands-use-the-chat-bar-for-commands-a-line-starting"),
            &field!(chat_commands.enabled),
        );
        cx.greyed(!cx.s.chat_commands.enabled);
        cx.toggle(
            tr!("ui-options-commands-output-to-a-channel"),
            tr!("ui-options-commands-also-send-command-responses-on-this-script-channel-whispered"),
            &field!(chat_commands.announce_to_channel),
        );
        let off = !cx.s.chat_commands.announce_to_channel;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-commands-channel"),
                "",
                &field!(chat_commands.announce_channel),
                |ui, s| {
                    ui.add(
                        egui::DragValue::new(&mut s.chat_commands.announce_channel)
                            .custom_parser(aurora_i18n::parse_number)
                            .range(-2_147_483_648..=2_147_483_647),
                    )
                    .changed()
                },
            );
        });
    });
    let off = !cx.s.chat_commands.enabled;
    cx.group(tr!("common-teleport-commands"), |cx| {
        cx.greyed(off);
        command(
            cx,
            tr!("ui-options-commands-in-region"),
            "cmd x y z",
            tr!("ui-options-commands-teleport-to-a-position-in-the-region"),
            &field!(chat_commands.pos),
        );
        command(
            cx,
            tr!("ui-options-commands-on-the-ground"),
            "cmd",
            tr!("ui-options-commands-teleport-to-ground-below-avatar"),
            &field!(chat_commands.ground),
        );
        command(
            cx,
            tr!("ui-options-commands-at-altitude"),
            "cmd z",
            tr!("ui-options-commands-teleport-to-this-altitude-z-or-z-relative"),
            &field!(chat_commands.height),
        );
        command(
            cx,
            tr!("ui-options-commands-at-camera"),
            "cmd",
            tr!("ui-options-commands-teleport-to-camera-position"),
            &field!(chat_commands.teleport_to_cam),
        );
        command(
            cx,
            tr!("ui-options-commands-to-an-avatar"),
            tr!("ui-options-commands-cmd-name"),
            tr!("ui-options-commands-case-insensitive-partial-name-among-nearby-avatars"),
            &field!(chat_commands.tp2),
        );
        command(
            cx,
            tr!("common-offer-teleport"),
            tr!("ui-options-commands-cmd-key"),
            tr!("ui-options-commands-offer-teleport-to-an-avatar-uuid"),
            &field!(chat_commands.offer_tp),
        );
        command(
            cx,
            tr!("common-home"),
            "cmd",
            tr!("ui-options-commands-teleport-home"),
            &field!(chat_commands.teleport_home),
        );
        command(
            cx,
            tr!("ui-options-commands-to-a-region"),
            tr!("ui-options-commands-cmd-region"),
            tr!("ui-options-commands-teleport-to-a-region-region-x-y-z-for"),
            &field!(chat_commands.map_to),
        );
        cx.toggle(
            tr!("ui-options-commands-keep-position"),
            tr!("ui-options-commands-use-the-same-position-across-regions-otherwise-128-128"),
            &field!(chat_commands.map_to_keep_pos),
        );
        command(
            cx,
            tr!("ui-options-commands-track-on-map"),
            tr!("status-cmd-x-y-z-name"),
            tr!("ui-options-commands-place-a-world-map-beacon-at-a-position-cpcampos"),
            &field!(chat_commands.track_pos),
        );
    });
    cx.group(tr!("common-camera-and-display-commands"), |cx| {
        cx.greyed(off);
        command(
            cx,
            tr!("common-draw-distance"),
            tr!("ui-options-commands-cmd-meters"),
            tr!("ui-options-commands-change-draw-distance-n-or-n-relative"),
            &field!(chat_commands.draw_distance),
        );
        command(
            cx,
            tr!("ui-options-commands-maximum-bandwidth"),
            tr!("ui-options-commands-cmd-kbps"),
            tr!("common-not-available-in-aurora-yet"),
            &field!(chat_commands.bandwidth),
        );
        command(
            cx,
            tr!("ui-options-commands-copy-camera"),
            "cmd",
            tr!("ui-options-commands-copy-camera-position-to-clipboard"),
            &field!(chat_commands.copy_cam),
        );
        command(
            cx,
            tr!("ui-options-commands-restore-camera"),
            tr!("ui-options-commands-cmd-pos-focus-roll"),
            tr!("ui-options-commands-move-the-camera-to-a-copied-position"),
            &field!(chat_commands.paste_cam),
        );
    });
    cx.group(tr!("common-other-commands"), |cx| {
        cx.greyed(off);
        command(
            cx,
            tr!("ui-options-commands-calculator"),
            "cmd SIN(2+2)",
            tr!("ui-options-commands-evaluate-an-expression"),
            &field!(chat_commands.calc),
        );
        command(
            cx,
            tr!("ui-options-commands-avatar-name"),
            tr!("ui-options-commands-cmd-key"),
            tr!("ui-options-commands-get-an-avatar-s-name-from-its-uuid"),
            &field!(chat_commands.key_to_name),
        );
        command(
            cx,
            tr!("ui-options-commands-dice-roll"),
            tr!("ui-options-commands-cmd-dice-faces"),
            tr!("ui-options-commands-cmd-dice-count-face-count-modifier-type-value-examples"),
            &field!(chat_commands.roll_dice),
        );
        command(
            cx,
            tr!("ui-options-commands-clear-chat"),
            "cmd",
            tr!("ui-options-commands-clear-nearby-chat-to-limit-spam"),
            &field!(chat_commands.clear_chat),
        );
        command(
            cx,
            tr!("ui-options-commands-media"),
            tr!("ui-options-commands-cmd-url-type"),
            tr!("ui-options-commands-set-and-play-parcel-media"),
            &field!(chat_commands.media),
        );
        command(
            cx,
            tr!("common-music"),
            tr!("ui-options-commands-cmd-url"),
            tr!("ui-options-commands-set-and-play-streaming-music"),
            &field!(chat_commands.music),
        );
        command(cx, "AO", "cmd on/off", tr!("ao-command-help"), &field!(chat_commands.ao));
        command(
            cx,
            tr!("ui-options-commands-platform"),
            tr!("ui-options-commands-cmd-width"),
            tr!("ui-options-commands-create-a-platform-under-your-avatar"),
            &field!(chat_commands.rez_platform),
        );
        cx.slider(
            tr!("ui-options-commands-platform-width"),
            tr!("ui-options-commands-width-used-when-the-command-does-not-specify-one"),
            &field!(chat_commands.platform_size),
            5.0..=64.0,
            " m",
        );
    });
    cx.track(&field!(chat_commands));
}
