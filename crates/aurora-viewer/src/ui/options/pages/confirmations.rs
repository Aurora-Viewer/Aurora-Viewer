//! Preferences › Confirmations: what Aurora asks before acting (the
//! "ignore" boxes of the Legacy Viewer's alerts: TeleportFromLandmark,
//! ReturnToOwner, AttachmentDrop, ConfirmScriptModify, FSConfirmPayments,
//! ChangeObjectBonusFactor, ConfirmTextureHeights), the alerts of the
//! region (FSShowServerVersionChangeNotice, FSReportTotalScriptCountChanges,
//! FSUseNewRegionRestartNotification…) and the collisions
//! (FSCollisionMessagesInChat, FSReportCollisionMessages).

use super::super::cx::{Cx, field, fields};

/// A script channel.
fn channel(ui: &mut egui::Ui, v: &mut i32) -> bool {
    ui.add(
        egui::DragValue::new(v)
            .custom_parser(aurora_i18n::parse_number)
            .range(i32::MIN..=i32::MAX)
            .speed(1.0),
    )
    .changed()
}

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-before-acting"), |cx| {
        cx.toggle(
            tr!("ui-options-confirm-teleports"),
            tr!("ui-options-ask-before-teleporting-to-a-landmark-a-history-entry"),
            &field!(confirm_teleports),
        );
        cx.toggle(
            tr!("ui-options-confirm-return-to-owner"),
            tr!("ui-options-confirm-return-to-owner-help"),
            &field!(confirm_return_to_owner),
        );
        cx.toggle(
            tr!("ui-options-confirm-attachment-drop"),
            tr!("ui-options-confirm-attachment-drop-help"),
            &field!(confirm_attachment_drop),
        );
        cx.toggle(
            tr!("ui-options-confirm-script-modify"),
            tr!("ui-options-confirm-script-modify-help"),
            &field!(confirm_script_modify),
        );
        cx.toggle(
            tr!("ui-options-confirm-payments"),
            tr!("ui-options-confirm-payments-help"),
            &field!(confirm_payments),
        );
        let off = !cx.s.confirm_payments;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-payment-threshold"),
                tr!("ui-options-payment-threshold-help"),
                &field!(payment_confirm_threshold),
                |ui, s| {
                    ui.add(
                        egui::DragValue::new(&mut s.payment_confirm_threshold)
                            .custom_parser(aurora_i18n::parse_number)
                            .range(0..=i32::MAX)
                            .speed(1.0)
                            .suffix(" L$"),
                    )
                    .changed()
                },
            );
        });
        cx.toggle(
            tr!("ui-options-confirm-object-bonus"),
            tr!("ui-options-confirm-object-bonus-help"),
            &field!(confirm_object_bonus),
        );
        cx.toggle(
            tr!("ui-options-check-texture-heights"),
            tr!("ui-options-check-texture-heights-help"),
            &field!(region_check_texture_heights),
        );
    });
    cx.group(tr!("ui-options-region-alerts"), |cx| {
        cx.toggle(
            tr!("ui-options-sim-version-alert"),
            tr!("ui-options-sim-version-alert-help"),
            &field!(sim_version_alert),
        );
        cx.toggle(
            tr!("ui-options-script-count-alert"),
            tr!("ui-options-script-count-alert-help"),
            &field!(script_count_alert),
        );
        let off = !cx.s.script_count_alert;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-script-count-threshold"),
                "",
                &field!(script_count_threshold),
                |ui, s| {
                    ui.add(
                        egui::DragValue::new(&mut s.script_count_threshold)
                            .custom_parser(aurora_i18n::parse_number)
                            .range(0..=10000)
                            .speed(10.0),
                    )
                    .changed()
                },
            );
        });
        cx.toggle(
            tr!("ui-options-region-restart-alert"),
            tr!("ui-options-region-restart-alert-help"),
            &field!(region_restart.show_alert),
        );
        let off = !cx.s.region_restart.show_alert;
        cx.subs(off, |cx| {
            use crate::world::region_restart::Shake;
            cx.choice(
                tr!("ui-options-region-restart-shake"),
                tr!("ui-options-region-restart-shake-help"),
                &field!(region_restart.shake),
                &[
                    (Shake::Alert, tr!("ui-options-region-restart-shake-alert"), false),
                    (Shake::View, tr!("ui-options-region-restart-shake-view"), false),
                    (Shake::None, tr!("ui-options-region-restart-shake-none"), false),
                ],
            );
            cx.toggle(
                tr!("ui-options-region-restart-announce"),
                tr!("ui-options-region-restart-announce-help"),
                &field!(region_restart.announce),
            );
            let silent = !cx.s.region_restart.announce;
            cx.row(
                tr!("ui-options-region-restart-channel"),
                "",
                &field!(region_restart.channel),
                |ui, s| ui.add_enabled_ui(!silent, |ui| channel(ui, &mut s.region_restart.channel)).inner,
            );
        });
    });
    cx.group(tr!("bumps-title"), |cx| {
        cx.toggle(
            tr!("ui-options-collisions-in-chat"),
            tr!("ui-options-collisions-in-chat-help"),
            &field!(collisions.in_chat),
        );
        cx.toggle(
            tr!("ui-options-collisions-report"),
            tr!("ui-options-collisions-report-help"),
            &field!(collisions.report),
        );
        let off = !cx.s.collisions.report;
        cx.subs(off, |cx| {
            cx.row(tr!("ui-options-collisions-channel"), "", &field!(collisions.channel), |ui, s| {
                channel(ui, &mut s.collisions.channel)
            });
        });
    });
    // the rest of these two, as the former Interface page reset them whole
    cx.track(&fields!(region_restart, collisions));
}
