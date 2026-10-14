//! Firestorm's Windlight presets (`indra/newview/app_settings/windlight`:
//! `days`, `skies`, `water`), built into the executable as one deflated
//! archive (`assets/windlight/windlight.zip`, the XML files unchanged, their
//! escaped file names kept). The Windlight day import reads its skies and
//! water from them when the folders beside the imported day are missing, as
//! LLSettingsVODay::buildFromLegacyPreset falls back on LL_PATH_APP_SETTINGS
//! (newview/llsettingsvo.cpp, originally LGPL 2.1; the presets' origin and
//! licence are in NOTICE.md).

use super::env_day_edit::LegacyFolder;
use std::io::Read;

static ARCHIVE: &[u8] = include_bytes!("../../assets/windlight/windlight.zip");

fn folder_name(folder: LegacyFolder) -> &'static str {
    match folder {
        LegacyFolder::Skies => "skies",
        LegacyFolder::Water => "water",
    }
}

/// A built-in preset file of `folder` by its file name (`Default.xml`,
/// `A%2D12AM.xml`…).
pub fn file(folder: LegacyFolder, file_name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(ARCHIVE))
        .map_err(|e| log::warn!("built-in windlight presets: {e}"))
        .ok()?;
    let mut entry = zip.by_name(&format!("{}/{file_name}", folder_name(folder))).ok()?;
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut data)
        .map_err(|e| log::warn!("built-in windlight preset {file_name}: {e}"))
        .ok()?;
    Some(data)
}

/// The path a built-in preset is reported under (Firestorm's own).
pub fn shown_path(folder: LegacyFolder, file_name: &str) -> String {
    format!("app_settings/windlight/{}/{file_name}", folder_name(folder))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::env_day_edit::{DayDoc, legacy_file_names};

    #[test]
    fn firestorm_presets_are_built_in() {
        let water = file(LegacyFolder::Water, "Default.xml").expect("the default water");
        assert!(crate::world::env_edit::SettingsDoc::import_legacy_water(&water, "Default").is_some());
        // a sky named with a dash is found under its escaped file name
        let names = legacy_file_names("A-12AM");
        assert!(names.iter().any(|n| file(LegacyFolder::Skies, n).is_some()));
        assert!(file(LegacyFolder::Skies, "No such sky.xml").is_none());
        // a whole day of Firestorm's: its skies and the default water
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(ARCHIVE)).expect("archive");
        let mut day = Vec::new();
        zip.by_name("days/Default.xml")
            .expect("the default day")
            .read_to_end(&mut day)
            .expect("read");
        let mut read = |folder: LegacyFolder, preset: &str| {
            legacy_file_names(preset)
                .iter()
                .find_map(|n| file(folder, n).map(|d| (d, shown_path(folder, n))))
                .ok_or_else(|| preset.to_owned())
        };
        let d = DayDoc::import_legacy(&day, "days/Default.xml", "Default", &mut read).expect("imported");
        assert!(d.tracks[1].len() >= 4);
    }
}
