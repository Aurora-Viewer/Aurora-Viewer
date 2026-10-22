//! Preferences › Language and updates › Backup: the confirmation of a
//! restore, with the parts of the backup to put back (the Legacy Viewer's
//! lists of files to restore, panel_preferences_backup.xml). The files are
//! read and written by the app (`app/settings_backup.rs`).

use super::{OptionsResult, OptionsUi, Page};
use crate::settings::Settings;
use crate::settings::backup::{Backup, Parts};
use crate::theme::Palette;
use egui::{RichText, Vec2};
use std::sync::Arc;

/// A backup read, waiting for its confirmation.
pub struct Pending {
    backup: Arc<Backup>,
    /// The parts ticked.
    parts: Parts,
    /// The account logged in, whose files are not written.
    logged_in: Option<uuid::Uuid>,
}

impl Pending {
    pub fn new(backup: Backup, logged_in: Option<uuid::Uuid>) -> Pending {
        Pending {
            parts: backup.parts(),
            backup: Arc::new(backup),
            logged_in,
        }
    }
}

/// A restore confirmed: the settings are already back (`restore_modal`),
/// the app puts back the rest.
pub struct Restored {
    pub backup: Arc<Backup>,
    pub parts: Parts,
}

pub(super) fn restore_modal(ctx: &egui::Context, p: &Palette, s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult, open: bool) {
    if !open {
        st.restore = None;
    }
    let Some(pending) = &mut st.restore else {
        return;
    };
    let held = pending.backup.parts();
    let mut confirmed = false;
    let mut cancelled = false;
    let modal = crate::ui::widgets::Modal::new(egui::Id::unique("options_restore_settings")).show(ctx, |ui| {
        use crate::ui::widgets::{ActionKind, action_button, checkbox, primary_button_sized};
        use aurora_i18n::tr;
        ui.set_width(400.0);
        ui.label(
            RichText::new(tr!("ui-options-backup-restore-title"))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr!(
                "ui-options-backup-restore-from",
                date = pending.backup.created(),
                version = pending.backup.viewer().to_owned()
            ))
            .size(12.5)
            .color(p.muted),
        );
        ui.add_space(8.0);
        let parts = &mut pending.parts;
        if held.settings {
            checkbox(ui, p, &mut parts.settings, tr!("ui-options-backup-part-settings"));
        }
        if held.layout {
            checkbox(ui, p, &mut parts.layout, tr!("ui-options-backup-part-layout"));
        }
        if held.skin {
            checkbox(ui, p, &mut parts.skin, tr!("ui-options-backup-part-skin"));
        }
        if held.accounts {
            let count = pending.backup.accounts().count();
            checkbox(ui, p, &mut parts.accounts, tr!("ui-options-backup-part-accounts", count = count));
            if parts.accounts && pending.logged_in.is_some_and(|id| pending.backup.accounts().any(|a| *a == id)) {
                ui.label(RichText::new(tr!("ui-options-backup-account-kept")).size(11.5).color(p.muted));
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            let any = parts.settings || parts.layout || parts.skin || parts.accounts;
            ui.add_enabled_ui(any, |ui| {
                if primary_button_sized(ui, p, tr!("ui-options-backup-restore-confirm"), Vec2::new(96.0, 26.0)).clicked() {
                    confirmed = true;
                }
            });
            if action_button(ui, p, "x", tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                cancelled = true;
            }
        });
    });
    if confirmed && let Some(pending) = st.restore.take() {
        let mut parts = pending.parts;
        // a reset's undo would now bring back what came before the restore
        st.undo = None;
        // the settings swap here, as the undo of a reset does
        if parts.settings {
            match pending.backup.settings(s) {
                Some(restored) => {
                    *s = restored;
                    for page in Page::ALL {
                        st.after_reset(page, s, r);
                    }
                }
                None => parts.settings = false,
            }
        }
        r.restored = Some(Restored {
            backup: pending.backup,
            parts,
        });
    } else if cancelled || modal.should_close() {
        st.restore = None;
    }
}
