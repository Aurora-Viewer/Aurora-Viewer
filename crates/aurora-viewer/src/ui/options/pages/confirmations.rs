//! Preferences › Confirmations: what Aurora asks before acting (the
//! "ignore" boxes of the Legacy Viewer's alerts: TeleportFromLandmark,
//! DeleteItems, ReturnToOwner, AttachmentDrop, ConfirmScriptModify,
//! FSConfirmPayments, ChangeObjectBonusFactor, ConfirmTextureHeights, and
//! the Marketplace listings' ConfirmMerchantMoveInventory,
//! ConfirmMerchantActiveChange, ConfirmMerchantClearVersion,
//! ConfirmListingCutOrDelete, ConfirmMerchantUnlist), the alerts of the
//! region (FSShowServerVersionChangeNotice, FSReportTotalScriptCountChanges,
//! FSUseNewRegionRestartNotification…) and the collisions
//! (FSCollisionMessagesInChat, FSReportCollisionMessages).
//!
//! It is also the one list of every "don't ask / don't show again" answer,
//! as the Legacy Viewer's Alerts tab (panel_preferences_alerts.xml,
//! `all_popups`, over the ignore state of each LLNotifications template):
//! the questions other pages own are shown here too (`Cx::borrow`, the same
//! rows), and the warnings and notices with no page of their own (the
//! Debug Settings warning, ConfirmHideUI, FirstUseFlyOverride, the poser's
//! close question, the old graphics driver and the skipped update) can be
//! turned back on.

use super::super::Page;
use super::super::cx::{Cx, field, fields};
use crate::ui::widgets;

/// A switch that is on while the warning `ignored` is not: the settings
/// that remember a dismissal ("ignored", "warned") read the other way.
fn shown(ui: &mut egui::Ui, p: &crate::theme::Palette, ignored: &mut bool) -> bool {
    let mut on = !*ignored;
    let changed = widgets::switch(ui, p, &mut on).changed();
    if changed {
        *ignored = !on;
    }
    changed
}

/// "Show again" for a notice dismissed for one thing (a driver, a
/// version): `dismissed` names it, empty while nothing is dismissed.
fn show_again(ui: &mut egui::Ui, p: &crate::theme::Palette, dismissed: &mut String) -> bool {
    ui.horizontal(|ui| {
        if !dismissed.is_empty() {
            ui.label(egui::RichText::new(dismissed.as_str()).size(12.0).color(p.muted));
        }
        let again = widgets::action_button_if(
            ui,
            p,
            "eye-filled",
            aurora_i18n::tr!("ui-options-show-again"),
            widgets::ActionKind::Flat,
            egui::vec2(0.0, 26.0),
            !dismissed.is_empty(),
        )
        .clicked();
        if again {
            dismissed.clear();
        }
        again
    })
    .inner
}

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
            tr!("ui-options-confirm-inventory-delete"),
            tr!("ui-options-confirm-inventory-delete-help"),
            &field!(inventory.confirm_delete),
        );
        cx.toggle(
            tr!("ui-options-warn-hypergrid-suitcase"),
            tr!("ui-options-warn-hypergrid-suitcase-help"),
            &field!(warn_hypergrid_suitcase),
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
                            .suffix(format!(" {}", aurora_i18n::currency())),
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
    cx.group(tr!("common-marketplace-listings"), |cx| {
        cx.toggle(
            tr!("ui-options-confirm-merchant-move"),
            tr!("ui-options-confirm-merchant-move-help"),
            &field!(confirm_merchant_move),
        );
        cx.toggle(
            tr!("ui-options-confirm-merchant-active-change"),
            tr!("ui-options-confirm-merchant-active-change-help"),
            &field!(confirm_merchant_active_change),
        );
        cx.toggle(
            tr!("ui-options-confirm-merchant-clear-version"),
            tr!("ui-options-confirm-merchant-clear-version-help"),
            &field!(confirm_merchant_clear_version),
        );
        cx.toggle(
            tr!("ui-options-confirm-merchant-listing-delete"),
            tr!("ui-options-confirm-merchant-listing-delete-help"),
            &field!(confirm_merchant_listing_delete),
        );
        cx.toggle(
            tr!("ui-options-confirm-merchant-unlist"),
            tr!("ui-options-confirm-merchant-unlist-help"),
            &field!(confirm_merchant_unlist),
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
    // the questions of other pages, their own rows: one list of them all
    cx.group(tr!("ui-options-asked-elsewhere"), |cx| {
        cx.borrow(
            Page::Inventory,
            0,
            &["inventory.warn_task_move", "inventory.warn_scripted_task_move"],
        );
        cx.borrow(Page::Graphics, 2, &["confirm_no_transfer_settings"]);
        cx.borrow(Page::Protections, 0, &["warn_external_links"]);
        cx.borrow(Page::Chat, 5, &["group_snooze_ask"]);
    });
    let p = cx.p;
    cx.group(tr!("ui-options-warnings-and-notices"), |cx| {
        cx.toggle(
            tr!("ui-options-confirm-hide-interface"),
            tr!("ui-options-confirm-hide-interface-help"),
            &field!(confirm_hide_interface),
        );
        cx.toggle(
            tr!("poser-confirm-close"),
            tr!("ui-options-poser-confirm-close-help"),
            &field!(poser.confirm_close),
        );
        cx.row(
            tr!("ui-options-debug-settings-warning"),
            tr!("ui-options-debug-settings-warning-help"),
            &field!(debug_settings_warning_ignored),
            |ui, s| shown(ui, &p, &mut s.debug_settings_warning_ignored),
        );
        cx.row(
            tr!("ui-options-fly-override-caution"),
            tr!("ui-options-fly-override-caution-help"),
            &field!(fly_override_warned),
            |ui, s| shown(ui, &p, &mut s.fly_override_warned),
        );
        cx.row(
            tr!("ui-options-driver-warning"),
            tr!("ui-options-driver-warning-help"),
            &field!(gpu_driver_dismissed),
            |ui, s| show_again(ui, &p, &mut s.gpu_driver_dismissed),
        );
        cx.row(
            tr!("ui-options-skipped-update"),
            tr!("ui-options-skipped-update-help"),
            &field!(ignored_update),
            |ui, s| show_again(ui, &p, &mut s.ignored_update),
        );
    });
    // the rest of these two, as the former Interface page reset them whole
    cx.track(&fields!(region_restart, collisions));
}
