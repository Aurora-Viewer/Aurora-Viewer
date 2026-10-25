//! The safe graphics after the graphics card stopped answering. When the
//! last session ended with the GPU in trouble (stalled, lost, or the window
//! frozen: `crash::take_gpu_trouble`), the login screen offers to start
//! with the Potato level. Nothing changes without the user's click: the
//! settings it replaces are kept aside (`graphics-before-safe.json`, the
//! level's values only) and the same card then offers them back. The
//! Legacy Viewer's equivalent is its safe start (`--safe`:
//! `LLFeatureManager::setSafe`, the "safe" feature mask applied in
//! indra/newview/llfeaturemanager.cpp, and the DisplaySetToSafe notice of
//! llstartup.cpp); it is only chosen by hand, never offered after a
//! stall, and leaves no way back but setting each option again.

use super::*;
use crate::graphics_presets::{Preset, Values};
use crate::ui::login::safe_graphics_notice::{Choice, SafeGraphics};

const FILE: &str = "graphics-before-safe.json";

fn backup_path() -> std::path::PathBuf {
    crate::settings::config_dir().join(FILE)
}

fn read_backup(path: &std::path::Path) -> Option<Values> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(values) => Some(values),
        Err(e) => {
            // a later version may have changed the level's values: the
            // safe graphics stay, nothing to restore
            log::warn!("{FILE} not read ({e}): nothing to restore");
            None
        }
    }
}

/// The card to show at start: an offer after GPU trouble, else the way
/// back while settings are kept aside. `AURORA_DEMO_SAFE_GRAPHICS=offer|active`
/// shows either in demo.
pub(super) fn at_start(trouble: bool, kept: bool, demo: Option<&str>) -> Option<SafeGraphics> {
    match demo {
        Some("offer") => return Some(SafeGraphics::Offer),
        Some("active") => return Some(SafeGraphics::Active),
        _ => {}
    }
    if trouble {
        Some(SafeGraphics::Offer)
    } else {
        kept.then_some(SafeGraphics::Active)
    }
}

impl App {
    pub(super) fn safe_graphics_at_start(&mut self) {
        let trouble = crate::crash::take_gpu_trouble();
        if trouble {
            log::warn!("the last session ended with the graphics card in trouble: safe graphics offered");
        }
        let demo = std::env::var_os("AURORA_DEMO").and_then(|_| std::env::var("AURORA_DEMO_SAFE_GRAPHICS").ok());
        self.login_form.safe_graphics = at_start(trouble, backup_path().exists(), demo.as_deref());
    }

    pub(super) fn safe_graphics_choice(&mut self, choice: Choice) {
        let path = backup_path();
        match choice {
            Choice::Use => {
                let before = Values::read(&self.settings);
                match serde_json::to_vec_pretty(&before)
                    .map_err(std::io::Error::other)
                    .and_then(|b| std::fs::write(&path, b))
                {
                    Ok(()) => {
                        Preset::Potato.apply(&mut self.settings);
                        log::info!("safe graphics on: Potato level, the settings before kept in {FILE}");
                        self.login_form.safe_graphics = Some(SafeGraphics::Active);
                    }
                    Err(e) => {
                        // never replace settings that could not be kept
                        log::warn!("safe graphics not applied: {FILE} not written ({e})");
                        self.login_form.safe_graphics = None;
                        return;
                    }
                }
            }
            Choice::Keep => {
                log::info!("safe graphics declined");
                self.login_form.safe_graphics = None;
                return;
            }
            Choice::Restore => {
                if let Some(values) = read_backup(&path) {
                    values.write(&mut self.settings);
                    log::info!("graphics settings restored from {FILE}");
                }
                let _ = std::fs::remove_file(&path);
                self.login_form.safe_graphics = None;
            }
            Choice::Stay => {
                let _ = std::fs::remove_file(&path);
                log::info!("safe graphics kept; the settings kept aside dropped");
                self.login_form.safe_graphics = None;
                return;
            }
        }
        self.settings.save();
        self.snapshots.settings_changed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_offers_after_trouble_then_the_way_back() {
        assert_eq!(at_start(false, false, None), None);
        assert_eq!(at_start(true, false, None), Some(SafeGraphics::Offer));
        // new trouble while already on the safe graphics: offered again
        assert_eq!(at_start(true, true, None), Some(SafeGraphics::Offer));
        assert_eq!(at_start(false, true, None), Some(SafeGraphics::Active));
        assert_eq!(at_start(false, false, Some("active")), Some(SafeGraphics::Active));
        assert_eq!(at_start(false, false, Some("other")), None);
    }

    #[test]
    fn the_settings_kept_aside_come_back_as_they_were() {
        let mut settings = crate::settings::Settings::default();
        Preset::Deluxe.apply(&mut settings);
        let before = Values::read(&settings);
        let dir = std::env::temp_dir().join(format!("aurora-safe-graphics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join(FILE);
        std::fs::write(&path, serde_json::to_vec(&before).unwrap()).unwrap();
        Preset::Potato.apply(&mut settings);
        assert_eq!(Preset::matching(&settings), Some(Preset::Potato));
        read_backup(&path).expect("kept values").write(&mut settings);
        assert_eq!(Values::read(&settings), before);
        // a file a later version cannot read restores nothing
        std::fs::write(&path, b"{\"draw_distance\": \"far\"}").unwrap();
        assert_eq!(read_backup(&path), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
