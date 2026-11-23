use super::super::cx::Cx;
use crate::extension_host::Command;
use crate::ui::widgets;
use aurora_extension_api::{PanelControl, PanelEventValue};
use aurora_extension_runtime::{ExtensionState, PublisherTrustStatus};
use aurora_i18n::tr;

fn permission_description(key: &str) -> String {
    match key {
        "chat.nearby.read" => tr!("extensions-permission-chat-nearby-read").to_owned(),
        "ui.chat.annotate" => tr!("extensions-permission-ui-chat-annotate").to_owned(),
        "ui.notification.show" => tr!("extensions-permission-ui-notification-show").to_owned(),
        "ui.panel.register" => tr!("extensions-permission-ui-panel-register").to_owned(),
        _ => tr!("extensions-manager-unknown-permission", permission = key),
    }
}

/// The Preferences › Extensions package manager.
pub fn page(cx: &mut Cx) {
    if !cx.shows_independent_controls() {
        return;
    }
    let pending_ack_ids: std::collections::HashSet<_> = cx
        .st
        .extensions
        .installed
        .iter()
        .filter(|extension| {
            !extension.unverified_acknowledged && matches!(&extension.publisher_trust, PublisherTrustStatus::UnverifiedPlugin { .. })
        })
        .map(|extension| extension.manifest.id.clone())
        .collect();
    cx.st.extension_enable_acknowledgements.retain(|id| pending_ack_ids.contains(id));
    let p = cx.p;
    cx.ui.label(tr!("extensions-manager-intro"));
    cx.ui.label(tr!("extensions-manager-sandbox"));
    widgets::divider(cx.ui, &p);

    let mut safe_mode = cx.st.extensions.safe_mode;
    if widgets::switch_labeled(cx.ui, &p, &mut safe_mode, tr!("extensions-manager-safe-mode")).changed() {
        cx.st.extensions.safe_mode = safe_mode;
        cx.r.extension_command = Some(Command::SafeMode(safe_mode));
    }
    if let Some(error) = &cx.st.extensions.error {
        cx.ui.colored_label(cx.p.danger, error);
    }
    if let Some(status) = &cx.st.extensions.status {
        cx.ui.label(status);
    }

    widgets::divider(cx.ui, &p);
    cx.ui.heading(tr!("extensions-manager-trusted-publishers"));
    cx.ui.label(tr!("extensions-manager-trusted-publishers-help"));
    if cx
        .ui
        .add_enabled(
            cx.st.extensions.ready && cx.st.extensions.pending.is_none() && cx.st.extensions.pending_publisher.is_none(),
            egui::Button::new(tr!("extensions-manager-import-publisher-key")),
        )
        .clicked()
        && let Some(path) = rfd::FileDialog::new()
            .set_title(tr!("extensions-manager-publisher-key-dialog"))
            .add_filter(tr!("extensions-manager-publisher-key-filter"), &["json"])
            .pick_file()
    {
        cx.r.extension_command = Some(Command::ChoosePublisherKey(path));
    }
    for publisher in cx.st.extensions.trusted_publishers.clone() {
        widgets::surface(cx.ui, &p, |ui| {
            ui.label(tr!(
                "extensions-manager-trusted-publisher",
                label = publisher.label.as_str(),
                key = publisher.key_id.as_str()
            ));
            if ui
                .add_enabled(
                    cx.st.extensions.pending.is_none() && cx.st.extensions.pending_publisher.is_none(),
                    egui::Button::new(tr!("extensions-manager-untrust-publisher")),
                )
                .clicked()
            {
                cx.r.extension_command = Some(Command::UntrustPublisher(publisher.key_id.clone()));
            }
        });
    }
    if let Some(publisher) = cx.st.extensions.pending_publisher.clone() {
        widgets::surface(cx.ui, &p, |ui| {
            ui.heading(tr!("extensions-manager-review-publisher-key"));
            ui.label(tr!("extensions-manager-key-fingerprint", key = publisher.key_id.as_str()));
            ui.label(tr!("extensions-manager-key-trust-warning"));
            ui.horizontal(|ui| {
                if ui.button(tr!("extensions-manager-trust-publisher")).clicked() {
                    cx.r.extension_command = Some(Command::CommitPublisherKey);
                }
                if ui.button(tr!("common-cancel")).clicked() {
                    cx.r.extension_command = Some(Command::CancelPublisherKey);
                }
            });
        });
    }

    if let Some(review) = cx.st.extensions.pending.clone() {
        if cx.st.extension_review_id.as_deref() != Some(review.id.as_str()) {
            cx.st.extension_review_id = Some(review.id.clone());
            cx.st.extension_grants.clear();
            cx.st.extension_enable_after_install = false;
            cx.st.extension_ack_unverified = false;
            cx.st.extension_ack_publisher_change = false;
        }
        widgets::divider(cx.ui, &p);
        cx.ui.heading(if review.is_update {
            tr!("extensions-manager-review-update")
        } else {
            tr!("extensions-manager-review-install")
        });
        cx.ui.label(format!("{} ({})", review.name, review.version));
        cx.ui.label(tr!("extensions-manager-id", id = review.id.as_str()));
        cx.ui
            .label(tr!("extensions-manager-publisher", publisher = review.publisher_claim.as_str()));
        match &review.publisher_trust {
            PublisherTrustStatus::VerifiedPublisher { key_id, label } => {
                cx.ui.label(tr!(
                    "extensions-manager-trust-verified",
                    label = label.as_str(),
                    key = key_id.as_str()
                ));
            }
            PublisherTrustStatus::UnverifiedPlugin { key_id: Some(key) } => {
                cx.ui
                    .colored_label(cx.p.warn, tr!("extensions-manager-trust-unverified-signed", key = key.as_str()));
            }
            PublisherTrustStatus::UnverifiedPlugin { key_id: None } => {
                cx.ui.colored_label(cx.p.warn, tr!("extensions-manager-trust-unverified-unsigned"));
            }
        }
        cx.ui.label(tr!("extensions-manager-hash", hash = review.content_sha256.as_str()));
        if review.permissions_changed {
            let added = review.added_permissions.iter().map(String::as_str).collect::<Vec<_>>().join(", ");
            cx.ui.colored_label(
                cx.p.warn,
                tr!("extensions-manager-permissions-changed", permissions = added.as_str()),
            );
        }
        if review.publisher_identity_changed {
            let previous = review.previous_signature_key.as_deref().unwrap_or("unsigned");
            let current = match &review.publisher_trust {
                PublisherTrustStatus::VerifiedPublisher { key_id, .. }
                | PublisherTrustStatus::UnverifiedPlugin { key_id: Some(key_id) } => key_id.as_str(),
                PublisherTrustStatus::UnverifiedPlugin { key_id: None } => "unsigned",
            };
            cx.ui.colored_label(
                cx.p.warn,
                tr!("extensions-manager-publisher-changed", previous = previous, current = current),
            );
        }
        cx.ui.label(tr!("extensions-manager-requested-permissions"));
        for permission in &review.requested_permissions {
            let mut checked = cx.st.extension_grants.contains(permission);
            let changed = widgets::checkbox(cx.ui, &p, &mut checked, permission).changed();
            cx.ui.label(permission_description(permission));
            if changed {
                if checked {
                    cx.st.extension_grants.insert(permission.clone());
                } else {
                    cx.st.extension_grants.remove(permission);
                    cx.st.extension_enable_after_install = false;
                }
            }
        }
        if matches!(&review.publisher_trust, PublisherTrustStatus::UnverifiedPlugin { .. }) {
            let mut acknowledged = cx.st.extension_ack_unverified;
            if widgets::checkbox(cx.ui, &p, &mut acknowledged, tr!("extensions-manager-ack-unverified")).changed() {
                cx.st.extension_ack_unverified = acknowledged;
            }
        }
        if review.publisher_identity_changed {
            let mut acknowledged = cx.st.extension_ack_publisher_change;
            if widgets::checkbox(cx.ui, &p, &mut acknowledged, tr!("extensions-manager-ack-publisher-change")).changed() {
                cx.st.extension_ack_publisher_change = acknowledged;
            }
        }
        let all_approved = review
            .requested_permissions
            .iter()
            .all(|permission| cx.st.extension_grants.contains(permission));
        let mut enable = cx.st.extension_enable_after_install;
        if widgets::switch_labeled(
            cx.ui,
            &p,
            &mut enable,
            if review.is_update {
                tr!("extensions-manager-enable-update")
            } else {
                tr!("extensions-manager-enable-install")
            },
        )
        .changed()
        {
            cx.st.extension_enable_after_install = enable;
        }
        let unverified_acknowledged =
            !matches!(&review.publisher_trust, PublisherTrustStatus::UnverifiedPlugin { .. }) || cx.st.extension_ack_unverified;
        let publisher_change_acknowledged = !review.publisher_identity_changed || cx.st.extension_ack_publisher_change;
        let can_install = (!enable || all_approved) && unverified_acknowledged && publisher_change_acknowledged;
        cx.ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    can_install,
                    egui::Button::new(if review.is_update {
                        tr!("extensions-manager-update")
                    } else {
                        tr!("extensions-manager-install")
                    }),
                )
                .clicked()
            {
                cx.r.extension_command = Some(Command::CommitInstall {
                    permissions: review
                        .requested_permissions
                        .iter()
                        .filter(|permission| cx.st.extension_grants.contains(*permission))
                        .cloned()
                        .collect(),
                    enable: cx.st.extension_enable_after_install,
                    acknowledge_unverified: cx.st.extension_ack_unverified,
                    acknowledge_publisher_change: cx.st.extension_ack_publisher_change,
                });
            }
            if ui.button(tr!("common-cancel")).clicked() {
                cx.st.extension_grants.clear();
                cx.st.extension_review_id = None;
                cx.st.extension_enable_after_install = false;
                cx.st.extension_ack_unverified = false;
                cx.st.extension_ack_publisher_change = false;
                cx.r.extension_command = Some(Command::CancelInstall);
            }
        });
    } else {
        cx.st.extension_review_id = None;
        cx.st.extension_grants.clear();
        cx.st.extension_enable_after_install = false;
        cx.st.extension_ack_unverified = false;
        cx.st.extension_ack_publisher_change = false;
        if cx
            .ui
            .add_enabled(
                cx.st.extensions.ready && cx.st.extensions.pending_publisher.is_none(),
                egui::Button::new(tr!("extensions-manager-install-file")),
            )
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title(tr!("extensions-manager-install-dialog"))
                .add_filter(tr!("extensions-manager-package-filter"), &["ext"])
                .pick_file()
        {
            cx.r.extension_command = Some(Command::ChooseInstall(path));
        }
    }

    if !cx.st.extensions.panels.is_empty() {
        widgets::divider(cx.ui, &p);
        cx.ui.heading(tr!("extensions-manager-panels"));
        for panel in cx.st.extensions.panels.clone() {
            widgets::surface(cx.ui, &p, |ui| {
                ui.heading(&panel.title);
                ui.label(tr!(
                    "extensions-manager-panel-identity",
                    extension = panel.extension_id.as_str(),
                    panel = panel.panel_id.as_str()
                ));
                ui.add_enabled_ui(!cx.st.extensions.safe_mode, |ui| {
                    for control in &panel.controls {
                        match control {
                            PanelControl::Label { text, .. } => {
                                ui.label(text);
                            }
                            PanelControl::Toggle { id, label, value } => {
                                let mut value = *value;
                                if widgets::switch_labeled(ui, &p, &mut value, label).changed() {
                                    cx.r.extension_command = Some(Command::PanelAction {
                                        id: panel.extension_id.clone(),
                                        panel_id: panel.panel_id.clone(),
                                        control_id: id.clone(),
                                        value: PanelEventValue::Toggled { value },
                                    });
                                }
                            }
                            PanelControl::Button { id, label } => {
                                if ui.button(label).clicked() {
                                    cx.r.extension_command = Some(Command::PanelAction {
                                        id: panel.extension_id.clone(),
                                        panel_id: panel.panel_id.clone(),
                                        control_id: id.clone(),
                                        value: PanelEventValue::Clicked,
                                    });
                                }
                            }
                        }
                    }
                });
            });
        }
    }

    widgets::divider(cx.ui, &p);
    cx.ui.heading(tr!("extensions-manager-installed"));
    if !cx.st.extensions.ready {
        cx.ui.label(tr!("extensions-manager-loading"));
    } else if cx.st.extensions.installed.is_empty() {
        cx.ui.label(tr!("extensions-manager-none"));
    }

    for extension in cx.st.extensions.installed.clone() {
        let id = extension.manifest.id.clone();
        widgets::surface(cx.ui, &p, |ui| {
            ui.heading(tr!(
                "extensions-manager-name-version",
                name = extension.manifest.name.as_str(),
                version = extension.manifest.version.as_str()
            ));
            ui.label(tr!(
                "extensions-manager-publisher-line",
                id = id.as_str(),
                publisher = extension.manifest.publisher.as_str()
            ));
            match &extension.publisher_trust {
                PublisherTrustStatus::VerifiedPublisher { key_id, label } => {
                    ui.label(tr!(
                        "extensions-manager-trust-verified",
                        label = label.as_str(),
                        key = key_id.as_str()
                    ));
                }
                PublisherTrustStatus::UnverifiedPlugin { key_id: Some(key) } => {
                    ui.colored_label(p.warn, tr!("extensions-manager-trust-unverified-signed", key = key.as_str()));
                }
                PublisherTrustStatus::UnverifiedPlugin { key_id: None } => {
                    ui.colored_label(p.warn, tr!("extensions-manager-trust-unverified-unsigned"));
                }
            }
            let state = match extension.state {
                ExtensionState::Disabled => tr!("extensions-manager-state-disabled"),
                ExtensionState::Enabled => tr!("extensions-manager-state-enabled"),
                ExtensionState::Quarantined => tr!("extensions-manager-state-quarantined"),
            };
            ui.label(tr!("extensions-manager-state", state = state));
            let requested = extension
                .requested_permissions
                .iter()
                .map(|permission| permission.manifest_key())
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(tr!("extensions-manager-requested", permissions = requested.as_str()));
            for permission in &extension.requested_permissions {
                ui.small(permission_description(permission.manifest_key()));
            }
            let granted = extension
                .granted_permissions
                .iter()
                .map(|permission| permission.manifest_key())
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(tr!("extensions-manager-granted", permissions = granted.as_str()));
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        cx.st.extensions.ready && cx.st.extensions.pending_publisher.is_none() && cx.st.extensions.pending.is_none(),
                        egui::Button::new(tr!("extensions-manager-update-file")),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title(tr!("extensions-manager-update-dialog"))
                        .add_filter(tr!("extensions-manager-package-filter"), &["ext"])
                        .pick_file()
                {
                    cx.r.extension_command = Some(Command::ChooseUpdate { id: id.clone(), path });
                }
                match extension.state {
                    ExtensionState::Enabled => {
                        if ui.button(tr!("extensions-manager-disable")).clicked() {
                            cx.r.extension_command = Some(Command::Disable { id: id.clone() });
                        }
                    }
                    ExtensionState::Disabled => {
                        let grants = extension
                            .requested_permissions
                            .iter()
                            .map(|p| p.manifest_key().to_owned())
                            .collect();
                        let unverified = matches!(&extension.publisher_trust, PublisherTrustStatus::UnverifiedPlugin { .. });
                        let mut acknowledged = extension.unverified_acknowledged || cx.st.extension_enable_acknowledgements.contains(&id);
                        if unverified
                            && !extension.unverified_acknowledged
                            && widgets::checkbox(ui, &p, &mut acknowledged, tr!("extensions-manager-ack-unverified-enable")).changed()
                        {
                            if acknowledged {
                                cx.st.extension_enable_acknowledgements.insert(id.clone());
                            } else {
                                cx.st.extension_enable_acknowledgements.remove(&id);
                            }
                        }
                        if ui
                            .add_enabled(
                                !cx.st.extensions.safe_mode && (!unverified || acknowledged),
                                egui::Button::new(tr!("extensions-manager-grant-enable")),
                            )
                            .clicked()
                        {
                            cx.r.extension_command = Some(Command::Enable {
                                id: id.clone(),
                                permissions: grants,
                                acknowledge_unverified: acknowledged,
                            });
                        }
                    }
                    ExtensionState::Quarantined => {
                        if ui.button(tr!("extensions-manager-clear-quarantine")).clicked() {
                            cx.r.extension_command = Some(Command::ClearQuarantine { id: id.clone() });
                        }
                    }
                }
                for permission in &extension.granted_permissions {
                    let key = permission.manifest_key().to_owned();
                    if ui.button(tr!("extensions-manager-revoke", permission = key.as_str())).clicked() {
                        cx.r.extension_command = Some(Command::Revoke {
                            id: id.clone(),
                            permission: key,
                        });
                    }
                }
                if ui.button(tr!("extensions-manager-uninstall")).clicked() {
                    cx.r.extension_command = Some(Command::Uninstall { id: id.clone() });
                }
            });
        });
    }
}
