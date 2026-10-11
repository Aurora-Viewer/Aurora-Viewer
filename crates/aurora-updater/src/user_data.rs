//! Maintenance happens after the old process exits. Backups live beside each
//! data path, so portable installations and profiles may be on different disks.
//! Selective settings reset follows llappviewer.cpp's startup purge (Firestorm,
//! originally LGPL 2.1): preserve unrelated profile files. Release thresholds
//! and recoverable backups are Aurora's explicit publication policy.

use crate::{
    Maintenance,
    install::{plain_dir, validate_tree},
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Entry {
    original: PathBuf,
    saved: PathBuf,
    directory: bool,
    existed: bool,
}

pub(crate) struct Backup {
    entries: Vec<Entry>,
}

fn paths(install: &Path) -> Result<(PathBuf, PathBuf)> {
    if let Some(root) = std::env::var_os("AURORA_UPDATE_TEST_ROOT") {
        let root = PathBuf::from(root).canonicalize()?;
        ensure!(root.join("install") == install, "test data outside isolated installation");
        let marker: serde_json::Value = serde_json::from_slice(&fs::read(root.join("aurora-update-test.json"))?)?;
        ensure!(marker["schema"].as_u64() == Some(1), "invalid test data marker");
        crate::feed::Source::local(marker["feed"].as_str().context("test feed")?)?;
        return Ok((root.join("cache"), root.join("profile/settings.json")));
    }
    let dirs = directories::ProjectDirs::from("org", "Aurora", "AuroraViewer").context("user data directories unavailable")?;
    Ok((dirs.cache_dir().into(), dirs.config_dir().join("settings.json")))
}

fn validate(path: &Path, directory: bool) -> Result<()> {
    if directory {
        validate_tree(path)
    } else {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "settings is not a regular file"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(metadata.file_attributes() & 0x400 == 0, "settings is a reparse point");
        }
        Ok(())
    }
}

impl Backup {
    pub(crate) fn prepare(install: &Path, token: &str, maintenance: Maintenance) -> Result<Self> {
        let (cache, settings) = paths(install)?;
        Self::from_paths(&cache, &settings, token, maintenance)
    }

    fn from_paths(cache: &Path, settings: &Path, token: &str, maintenance: Maintenance) -> Result<Self> {
        ensure!(uuid::Uuid::parse_str(token).is_ok(), "invalid data backup token");
        let mut backup = Self { entries: Vec::new() };
        for (original, requested, directory) in [
            (cache, maintenance.clear_cache, true),
            (settings, maintenance.reset_settings, false),
        ] {
            if !requested {
                continue;
            }
            let existed = original.try_exists()?;
            if existed {
                validate(original, directory)?;
            }
            // Resolve the parent, keeping the final component un-followed. A
            // junction in the data itself was rejected above before any rename.
            let parent = original.parent().context("data parent")?;
            fs::create_dir_all(parent)?;
            let parent = parent.canonicalize()?;
            plain_dir(&parent)?;
            let name = original.file_name().context("data name")?;
            let original = parent.join(name);
            let saved = parent.join(format!("{}.aurora-before-update-{token}", name.to_string_lossy()));
            ensure!(!saved.try_exists()?, "data backup already exists");
            if existed {
                fs::rename(&original, &saved).context("cannot back up user data")?;
            }
            backup.entries.push(Entry {
                original,
                saved,
                directory,
                existed,
            });
        }
        Ok(backup)
    }

    pub(crate) fn commit(&mut self) {
        for entry in self.entries.drain(..) {
            if !entry.existed {
                continue;
            }
            if let Err(e) = validate(&entry.saved, entry.directory).and_then(|()| {
                if entry.directory {
                    fs::remove_dir_all(&entry.saved)
                } else {
                    fs::remove_file(&entry.saved)
                }
                .map_err(Into::into)
            }) {
                // The new viewer is running: retain the recoverable backup,
                // never turn a cleanup failure into an installation rollback.
                log::warn!("update user data backup cleanup: {e:#}");
            }
        }
    }

    fn restore(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        for entry in self.entries.drain(..).rev() {
            let result = (|| -> Result<()> {
                if entry.existed {
                    validate(&entry.saved, entry.directory)?;
                }
                if entry.original.try_exists()? {
                    validate(&entry.original, entry.directory)?;
                    if entry.directory {
                        fs::remove_dir_all(&entry.original)?;
                    } else {
                        fs::remove_file(&entry.original)?;
                    }
                }
                if entry.existed {
                    fs::rename(&entry.saved, &entry.original)?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                failures.push(format!("{e:#}"));
            }
        }
        ensure!(failures.is_empty(), "user data restoration: {}", failures.join("; "));
        Ok(())
    }
}

impl Drop for Backup {
    fn drop(&mut self) {
        if let Err(e) = self.restore() {
            log::error!("update retained user data backup after restoration failure: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("aurora-data-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).expect("fixture");
            Self(path)
        }
        fn paths(&self) -> (PathBuf, PathBuf) {
            let cache = self.0.join("cache");
            let profile = self.0.join("profile");
            fs::create_dir(&cache).expect("cache");
            fs::create_dir(&profile).expect("profile");
            fs::write(cache.join("old-texture"), b"cache").expect("fixture");
            fs::write(profile.join("settings.json"), b"old settings").expect("fixture");
            fs::write(profile.join("contacts.json"), b"contacts").expect("fixture");
            (cache, profile.join("settings.json"))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn successful_update_applies_each_choice_and_preserves_other_profile_files() {
        for clear_cache in [false, true] {
            for reset_settings in [false, true] {
                let tmp = Temp::new();
                let (cache, settings) = tmp.paths();
                let mut backup = Backup::from_paths(
                    &cache,
                    &settings,
                    &uuid::Uuid::new_v4().to_string(),
                    Maintenance {
                        clear_cache,
                        reset_settings,
                    },
                )
                .expect("backup");
                assert_eq!(cache.exists(), !clear_cache);
                assert_eq!(settings.exists(), !reset_settings);
                assert_eq!(fs::read(settings.with_file_name("contacts.json")).expect("contacts"), b"contacts");
                backup.commit();
                assert!(
                    !fs::read_dir(&tmp.0)
                        .expect("root")
                        .any(|e| e.expect("entry").file_name().to_string_lossy().contains("aurora-before"))
                );
                assert!(
                    !fs::read_dir(settings.parent().expect("profile")).expect("profile").any(|e| e
                        .expect("entry")
                        .file_name()
                        .to_string_lossy()
                        .contains("aurora-before"))
                );
            }
        }
    }

    #[test]
    fn failed_start_restores_cache_and_settings_even_after_new_viewer_writes() {
        let tmp = Temp::new();
        let (cache, settings) = tmp.paths();
        let backup = Backup::from_paths(
            &cache,
            &settings,
            &uuid::Uuid::new_v4().to_string(),
            Maintenance {
                clear_cache: true,
                reset_settings: true,
            },
        )
        .expect("backup");
        fs::create_dir(&cache).expect("new cache");
        fs::write(cache.join("new-texture"), b"new").expect("new cache");
        fs::write(&settings, b"new settings").expect("new settings");
        drop(backup);
        assert_eq!(fs::read(cache.join("old-texture")).expect("restored cache"), b"cache");
        assert!(!cache.join("new-texture").exists());
        assert_eq!(fs::read(&settings).expect("restored settings"), b"old settings");
    }

    #[test]
    fn rollback_restores_absence_when_the_profile_had_no_cache_or_settings() {
        let tmp = Temp::new();
        let cache = tmp.0.join("cache");
        let settings = tmp.0.join("profile/settings.json");
        let backup = Backup::from_paths(
            &cache,
            &settings,
            &uuid::Uuid::new_v4().to_string(),
            Maintenance {
                clear_cache: true,
                reset_settings: true,
            },
        )
        .expect("empty data backup");
        fs::create_dir(&cache).expect("new cache");
        fs::write(cache.join("new-texture"), b"new").expect("fixture");
        fs::write(&settings, b"new settings").expect("fixture");
        drop(backup);
        assert!(!cache.exists());
        assert!(!settings.exists());
    }

    #[test]
    fn partial_preparation_failure_restores_the_already_moved_cache() {
        let tmp = Temp::new();
        let (cache, settings) = tmp.paths();
        fs::remove_file(&settings).expect("fixture");
        fs::create_dir(&settings).expect("invalid settings");
        assert!(
            Backup::from_paths(
                &cache,
                &settings,
                &uuid::Uuid::new_v4().to_string(),
                Maintenance {
                    clear_cache: true,
                    reset_settings: true
                }
            )
            .is_err()
        );
        assert!(cache.join("old-texture").exists());
        assert!(settings.is_dir());
    }
}
