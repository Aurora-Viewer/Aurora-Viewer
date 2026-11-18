//! The interface language file the user chose (Preferences › Language and
//! updates, "Use a language file"): a Fluent catalog like the bundled ones
//! (`crates/aurora-i18n/locales/`), read once at startup, so a new file
//! applies at the next launch. A file that cannot be read leaves the
//! language of the list. The Legacy Viewer has no such choice: it is an
//! Aurora addition, for translators and the languages Aurora does not ship.
//!
//! Only the file's name reaches the log: its folder holds the Windows user
//! name.

use aurora_i18n::{UserCatalog, UserCatalogError};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Why the chosen file is not in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Missing,
    Unreadable,
    TooLarge,
    /// Not UTF-8 text.
    NotText,
    /// Not one Fluent message in it.
    Empty,
}

/// The file read at startup, and what came of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub path: PathBuf,
    pub result: Result<UserCatalog, Failure>,
}

static LOADED: OnceLock<Loaded> = OnceLock::new();

/// The file of the settings, when its use is chosen.
pub fn chosen(enabled: bool, file: &str) -> Option<&str> {
    let file = file.trim();
    (enabled && !file.is_empty()).then_some(file)
}

/// Reads and installs the chosen file, once per process; the later calls
/// change nothing (the file applies at the next launch). The first read
/// comes before the log starts (crash recovery loads the settings): `report`
/// writes its outcome once the log is there.
pub fn start(file: Option<&str>) {
    let Some(file) = file else {
        return;
    };
    LOADED.get_or_init(|| {
        let path = PathBuf::from(file);
        let result = read(&path);
        Loaded { path, result }
    });
}

/// Writes what came of the file read at startup to the log, by its name.
pub fn report() {
    let Some(loaded) = loaded() else {
        return;
    };
    let name = display_name(&loaded.path);
    match &loaded.result {
        Ok(info) if info.errors > 0 => log::warn!(
            "Interface language file {name}: {} messages ({}), {} entries not readable, left out",
            info.messages,
            info.code,
            info.errors
        ),
        Ok(info) => log::info!("Interface language file {name}: {} messages ({})", info.messages, info.code),
        Err(failure) => log::warn!("Interface language file {name} not used: {failure:?}"),
    }
}

fn read(path: &Path) -> Result<UserCatalog, Failure> {
    let size = std::fs::metadata(path).map_err(io_failure)?.len();
    if size > aurora_i18n::USER_CATALOG_MAX_BYTES {
        return Err(Failure::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(io_failure)?;
    let source = String::from_utf8(bytes).map_err(|_| Failure::NotText)?;
    let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
    aurora_i18n::install_user_catalog(stem, &source).map_err(|error| match error {
        UserCatalogError::Empty => Failure::Empty,
        // installed by an earlier read: never, the read is once
        UserCatalogError::AlreadyInstalled => Failure::Unreadable,
    })
}

fn io_failure(error: std::io::Error) -> Failure {
    match error.kind() {
        std::io::ErrorKind::NotFound => Failure::Missing,
        _ => Failure::Unreadable,
    }
}

/// The file read at startup, if any.
pub fn loaded() -> Option<&'static Loaded> {
    LOADED.get()
}

/// The interface language from the settings: the list's choice, or the
/// language file when its use is chosen and it was read at startup; a
/// `--language` on the command line wins, as for the list.
pub fn select(language: &str, enabled: bool) -> bool {
    aurora_i18n::select_with(language, enabled, crate::cli::get().language.as_deref())
}

/// A path's file name, for the log and the preferences.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// What the preferences say of the file in the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// No file chosen yet.
    NoFile,
    /// The file in use since startup.
    InUse { name: String, info: UserCatalog },
    /// Read at startup and not usable.
    Failed { name: String, failure: Failure },
    /// Another file than the one read at startup (or none was): it applies
    /// at the next launch.
    NextLaunch { name: String },
}

pub fn shown(file: &str, loaded: Option<&Loaded>) -> Shown {
    let file = file.trim();
    if file.is_empty() {
        return Shown::NoFile;
    }
    let name = display_name(Path::new(file));
    match loaded {
        Some(loaded) if loaded.path == Path::new(file) => match loaded.result {
            Ok(info) => Shown::InUse { name, info },
            Err(failure) => Shown::Failed { name, failure },
        },
        _ => Shown::NextLaunch { name },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> UserCatalog {
        UserCatalog {
            code: "de",
            named: true,
            messages: 12,
            errors: 0,
        }
    }

    #[test]
    fn the_file_is_used_only_when_its_use_is_chosen() {
        assert_eq!(chosen(true, r"C:\t\de.ftl"), Some(r"C:\t\de.ftl"));
        assert_eq!(chosen(true, "  "), None);
        assert_eq!(chosen(false, r"C:\t\de.ftl"), None);
    }

    /// A file of a folder, written as this system writes paths: the name
    /// shown must come out of it on every system, not only on Windows.
    fn file(name: &str) -> String {
        Path::new("t").join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn the_preferences_tell_the_file_in_use_from_the_next_one() {
        let de = file("de.ftl");
        let ok = Loaded {
            path: PathBuf::from(&de),
            result: Ok(info()),
        };
        assert_eq!(shown("", Some(&ok)), Shown::NoFile);
        assert_eq!(
            shown(&de, Some(&ok)),
            Shown::InUse {
                name: "de.ftl".into(),
                info: info()
            }
        );
        assert_eq!(shown(&file("es.ftl"), Some(&ok)), Shown::NextLaunch { name: "es.ftl".into() });
        assert_eq!(shown(&de, None), Shown::NextLaunch { name: "de.ftl".into() });
        let failed = Loaded {
            path: PathBuf::from(&de),
            result: Err(Failure::NotText),
        };
        assert_eq!(
            shown(&de, Some(&failed)),
            Shown::Failed {
                name: "de.ftl".into(),
                failure: Failure::NotText
            }
        );
    }

    #[test]
    fn unreadable_files_are_refused_before_the_catalog() {
        let dir = std::env::temp_dir().join(format!("aurora-language-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temporary folder");
        assert_eq!(read(&dir.join("missing.ftl")), Err(Failure::Missing));
        let latin1 = dir.join("de.ftl");
        std::fs::write(&latin1, b"language-name = Fran\xe7ais\n").expect("write test file");
        assert_eq!(read(&latin1), Err(Failure::NotText));
        let empty = dir.join("fr.ftl");
        std::fs::write(&empty, "# nothing\n").expect("write test file");
        assert_eq!(read(&empty), Err(Failure::Empty));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
