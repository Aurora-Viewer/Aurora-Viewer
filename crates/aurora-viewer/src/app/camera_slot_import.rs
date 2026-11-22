//! On-demand, read-only Aperture-compatible camera slot import. The AO import's
//! picker/preview/confirm flow is the UI precedent (ui/ao/migration.rs).

use super::App;
use crate::camera::{CameraShot, account_slots::State, legacy_slots};
use crate::theme::Palette;
use crate::ui::widgets;
use std::sync::mpsc::{self, Receiver};
use uuid::Uuid;

enum Outcome {
    Read(legacy_slots::Imported, String),
    BackedUp(Vec<CameraShot>),
    Failed,
    Cancelled,
}

#[derive(Default)]
pub(super) struct Job {
    receiver: Option<Receiver<Outcome>>,
    preview: Option<legacy_slots::Imported>,
    source_folder: Option<String>,
    selected: [bool; 8],
    account: Option<Uuid>,
    applying: bool,
    failed: bool,
}

impl Job {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn spawn(work: impl FnOnce() -> Outcome + Send + 'static) -> Option<Receiver<Outcome>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("aurora-legacy-camera-import".into())
        .spawn(move || {
            let _ = tx.send(work());
        })
        .ok()?;
    Some(rx)
}

fn source_dialog() -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new()
        .set_title(aurora_i18n::tr!("camera-aperture-import"))
        .add_filter(aurora_i18n::tr!("camera-aperture-xml"), &["xml"])
        .set_file_name("settings_per_account.xml");
    // LLDir platform conventions are shared with the other importers.
    // Prefer Aperture, then compatible Legacy forks; macOS shares their folder.
    let mut folders = crate::legacy_profile::app_folders();
    folders.sort_by_key(|path| {
        !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().to_lowercase().contains("aperture"))
    });
    if let Some(folder) = folders.into_iter().find(|path| path.is_dir()) {
        dialog = dialog.set_directory(folder);
    }
    dialog
}

impl App {
    pub(super) fn demo_legacy_slot_preview(&mut self) {
        use crate::camera::presets::Anchor;
        if self.camera_slots.state != State::Ready {
            return;
        }
        let mut current = CameraShot::at_avatar("Aurora slot".into(), [0.0, -4.0, 2.0], [0.0, 0.0, 1.0]);
        current.quick_slot = Some(0);
        crate::camera::presets::store(&mut self.camera_slots.slots, current);
        self.camera_slots.save();
        self.refresh_camera_shots();
        let slots = [(0, "Legacy slot 1"), (7, "Legacy cinema view")]
            .into_iter()
            .map(|(slot, name)| {
                let mut shot = CameraShot::at_avatar(name.into(), [0.0, -4.0, 2.0], [0.0, 0.0, 1.0]);
                shot.quick_slot = Some(slot);
                shot.roll = Some(0.3);
                shot.anchor = Some(Anchor::World {
                    camera: [1000.0, 2000.0, 30.0],
                    focus: [1001.0, 2000.0, 30.0],
                });
                shot
            })
            .collect();
        self.slot_import.account = self.camera_slots.account;
        self.slot_import.preview = Some(legacy_slots::Imported {
            slots,
            invalid: Vec::new(),
        });
        self.slot_import.source_folder = Some("Synthetic account".into());
        self.slot_import.selected = [false, false, false, false, false, false, false, true];
    }

    pub(super) fn start_legacy_slot_import(&mut self) {
        if self.camera_slots.state != State::Ready || self.slot_import.receiver.is_some() {
            return;
        }
        self.slot_import.reset();
        self.slot_import.account = self.camera_slots.account;
        self.slot_import.receiver = spawn(|| {
            let Some(path) = source_dialog().pick_file() else {
                return Outcome::Cancelled;
            };
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| !name.eq_ignore_ascii_case("settings_per_account.xml"))
            {
                return Outcome::Failed;
            }
            match legacy_slots::read(&path) {
                Ok(imported) => {
                    let folder = path
                        .parent()
                        .and_then(|parent| parent.file_name())
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    Outcome::Read(imported, folder)
                }
                Err(e) => {
                    log::warn!("legacy camera slots could not be read: {e}");
                    Outcome::Failed
                }
            }
        });
        if self.slot_import.receiver.is_none() {
            self.slot_import.failed = true;
        }
    }

    pub(super) fn poll_legacy_slot_import(&mut self) {
        if self.slot_import.account != self.camera_slots.account {
            self.slot_import.reset();
            return;
        }
        let Some(result) = self.slot_import.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Outcome::Failed),
            Err(mpsc::TryRecvError::Empty) => None,
        }) else {
            return;
        };
        self.slot_import.receiver = None;
        match result {
            Outcome::Read(imported, folder) => {
                self.slot_import.source_folder = Some(folder);
                self.slot_import.selected = [false; 8];
                for shot in &imported.slots {
                    if let Some(slot) = shot.quick_slot {
                        self.slot_import.selected[usize::from(slot)] = !self.camera_slots.slots.iter().any(|s| s.quick_slot == Some(slot));
                    }
                }
                self.slot_import.preview = Some(imported);
            }
            Outcome::BackedUp(shots) => {
                for shot in shots {
                    crate::camera::presets::store(&mut self.camera_slots.slots, shot);
                }
                let count = self.slot_import.selected.iter().filter(|selected| **selected).count();
                self.camera_slots.save();
                self.refresh_camera_shots();
                self.controls_ui.reset_quick_names();
                self.slot_import.reset();
                self.camera_preset_message(aurora_i18n::tr!("camera-legacy-done", count = count as i64));
            }
            Outcome::Failed => {
                self.slot_import.applying = false;
                self.slot_import.failed = true;
            }
            Outcome::Cancelled => self.slot_import.reset(),
        }
    }

    pub(super) fn confirm_legacy_slot_import(&mut self) {
        if self.camera_slots.state != State::Ready || self.slot_import.applying {
            return;
        }
        if self.camera_slots.account.is_none() {
            return;
        }
        let Some(preview) = &self.slot_import.preview else { return };
        let selected: Vec<CameraShot> = preview
            .slots
            .iter()
            .filter(|shot| shot.quick_slot.is_some_and(|slot| self.slot_import.selected[usize::from(slot)]))
            .cloned()
            .collect();
        if selected.is_empty() {
            return;
        }
        let barrier = self.camera_slots.barrier();
        let path = self.camera_slots.path().to_path_buf();
        self.slot_import.applying = true;
        self.slot_import.failed = false;
        self.slot_import.receiver = spawn(move || {
            if barrier.recv_timeout(std::time::Duration::from_secs(5)).is_err() {
                return Outcome::Failed;
            }
            if path.exists() {
                let Ok(bytes) = std::fs::read(&path) else { return Outcome::Failed };
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_nanos())
                    .unwrap_or_default();
                let backup = path.with_file_name(format!("camera_quick_views.before-legacy-import-{stamp}.json"));
                if crate::atomic_file::write(&backup, &bytes).is_err() {
                    return Outcome::Failed;
                }
            }
            Outcome::BackedUp(selected)
        });
        if self.slot_import.receiver.is_none() {
            self.slot_import.applying = false;
            self.slot_import.failed = true;
        }
    }

    pub(super) fn legacy_slot_import_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if self.slot_import.preview.is_none() && !self.slot_import.failed {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        let preview = self.slot_import.preview.as_ref();
        let selected = &mut self.slot_import.selected;
        let failed = self.slot_import.failed;
        let applying = self.slot_import.applying;
        let modal = widgets::Modal::new(egui::Id::unique("legacy_camera_slots_import")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("camera-aperture-import"))
                    .strong()
                    .color(p.ink),
            );
            ui.label(aurora_i18n::tr!("camera-aperture-help"));
            if let Some(folder) = &self.slot_import.source_folder {
                ui.label(aurora_i18n::tr!("camera-legacy-source", folder = folder.as_str()));
            }
            if failed {
                ui.label(egui::RichText::new(aurora_i18n::tr!("camera-legacy-failed")).color(p.warn));
            }
            if let Some(preview) = preview {
                if preview.slots.is_empty() {
                    ui.label(aurora_i18n::tr!("camera-aperture-none"));
                }
                if !preview.invalid.is_empty() {
                    ui.label(aurora_i18n::tr!("camera-legacy-invalid", count = preview.invalid.len() as i64));
                }
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for shot in &preview.slots {
                        let Some(slot) = shot.quick_slot else { continue };
                        let index = usize::from(slot);
                        let previous = self.camera_slots.slots.iter().find(|s| s.quick_slot == Some(slot));
                        let label = if let Some(previous) = previous {
                            aurora_i18n::tr!(
                                "camera-legacy-replace",
                                slot = i64::from(slot) + 1,
                                name = shot.name.as_str(),
                                old = previous.name.as_str()
                            )
                        } else {
                            aurora_i18n::tr!("camera-legacy-slot", slot = i64::from(slot) + 1, name = shot.name.as_str())
                        };
                        widgets::checkbox(ui, p, &mut selected[index], label);
                    }
                });
            }
            if applying {
                ui.label(aurora_i18n::tr!("camera-legacy-applying"));
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!applying && selected.iter().any(|value| *value), |ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("camera-legacy-confirm")).clicked() {
                        confirm = true;
                    }
                });
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel || modal.should_close() {
            self.slot_import.reset();
        } else if confirm {
            self.confirm_legacy_slot_import();
        }
    }
}
