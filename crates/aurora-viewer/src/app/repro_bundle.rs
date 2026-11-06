//! Saving the privacy-filtered GPU repro archive from Preferences.

use super::*;

impl App {
    pub(super) fn save_repro_bundle(&mut self) {
        let confirmed = rfd::MessageDialog::new()
            .set_title(aurora_i18n::tr!("deep-diagnostics-bundle-confirm-title"))
            .set_description(crate::deep_diagnostics::confirmation_text())
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if confirmed != rfd::MessageDialogResult::Yes {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title(aurora_i18n::tr!("deep-diagnostics-bundle-save"))
            .add_filter("ZIP", &["zip"])
            .set_file_name("aurora-repro.zip")
            .save_file()
        else {
            return;
        };
        let bytes = crate::deep_diagnostics::build_bundle(
            &Settings::path(),
            &crate::logging::log_path(),
            crate::crash::current_session_dir().as_deref(),
        );
        self.options_ui.repro_bundle_status = match bytes.and_then(|bytes| crate::atomic_file::write(&path, &bytes)) {
            Ok(()) => aurora_i18n::tr!("deep-diagnostics-bundle-saved").to_owned(),
            Err(error) => {
                log::warn!("diagnostic bundle could not be saved: {error}");
                aurora_i18n::tr!("deep-diagnostics-bundle-failed").to_owned()
            }
        };
    }
}
