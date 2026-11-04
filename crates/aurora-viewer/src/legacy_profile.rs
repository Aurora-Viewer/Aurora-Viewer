//! Where a Legacy Viewer keeps an account's files on this computer, for what
//! is imported from it: the folder of each viewer, and the account's folder
//! in it. Nothing here writes; these folders are another viewer's.
//!
//! The folders follow LLDir_Linux, LLDir_Win32 and LLDir_Mac::initAppDirs and
//! LLDir::setLindenUserDir (indra/llfilesystem, originally LGPL 2.1).

use std::path::{Path, PathBuf};

/// The per-user folders of the viewers read, by application name: Firestorm
/// (the 64-bit name first, then the macOS and former one), the official
/// viewer and Alchemy. Any other goes through a folder dialog.
const APPS: [&str; 5] = ["Firestorm_x64", "Firestorm", "SecondLife", "AlchemyNext", "Alchemy"];

/// Label of the beta grid in the folder names (SECOND_LIFE_BETA_LABEL).
const BETA_SUFFIX: &str = ".second_life_beta";

/// Who to look for in a Legacy Viewer's folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub first: String,
    pub last: String,
    /// `chat_log::grid_name`.
    pub grid: String,
}

impl Account {
    /// LLSecAPIBasicCredential::userID: `first_last` in lower case; the last
    /// name of a one-word name is "Resident".
    pub fn folder(&self) -> String {
        format!("{}_{}", self.first, self.last).to_lowercase().replace(' ', "_")
    }

    /// False on another grid than Second Life's: its label comes from the
    /// viewer's grid list (`LLGridManager::getGridLabel`), so its folder
    /// cannot be told from another grid's, and only the user can choose it.
    pub fn grid_known(&self) -> bool {
        !self.grid.starts_with("custom_")
    }

    /// Whether a folder in an application's folder is this account's.
    /// Another grid's label is unknown (see `grid_known`), so any label but
    /// the beta grid's will do there.
    pub fn owns(&self, name: &str) -> bool {
        let base = self.folder();
        match self.grid.as_str() {
            "aditi" => name == format!("{base}{BETA_SUFFIX}"),
            _ if !self.grid_known() => name
                .strip_prefix(base.as_str())
                .is_some_and(|rest| rest.starts_with('.') && rest != BETA_SUFFIX),
            _ => name == base,
        }
    }
}

/// `<user folder>/<application>` for a home folder and the system's folder
/// of application data: `~/.firestorm_x64` on Linux, `%APPDATA%\\Firestorm_x64`
/// on Windows and `~/Library/Application Support/Firestorm` on macOS.
fn app_folders_in(home: &Path, data: &Path, dotted: bool) -> Vec<PathBuf> {
    APPS.iter()
        .map(|app| {
            if dotted {
                home.join(format!(".{}", app.to_lowercase()))
            } else {
                data.join(app)
            }
        })
        .collect()
}

/// The application folders of this system.
pub fn app_folders() -> Vec<PathBuf> {
    match directories::BaseDirs::new() {
        Some(dirs) => app_folders_in(dirs.home_dir(), dirs.data_dir(), cfg!(not(any(windows, target_os = "macos")))),
        None => Vec::new(),
    }
}

/// The account's folders under these application folders (`app_folders`, or
/// the demo's and the tests'), the viewers' order kept.
pub fn account_folders(apps: &[PathBuf], account: &Account) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for app in apps {
        let Ok(entries) = std::fs::read_dir(app) else {
            continue;
        };
        let mut here: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_name().to_str().is_some_and(|n| account.owns(n)))
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        here.sort();
        found.append(&mut here);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(grid: &str) -> Account {
        Account {
            first: "Jane".into(),
            last: "Doe".into(),
            grid: grid.into(),
        }
    }

    #[test]
    fn the_folders_of_each_system() {
        let (home, data) = (Path::new("/home/j"), Path::new("/data"));
        assert_eq!(app_folders_in(home, data, true)[0], Path::new("/home/j/.firestorm_x64"));
        assert_eq!(app_folders_in(home, data, true)[3], Path::new("/home/j/.alchemynext"));
        assert_eq!(app_folders_in(home, data, false)[0], Path::new("/data/Firestorm_x64"));
        // the macOS build's folder is "Firestorm"
        assert_eq!(app_folders_in(home, data, false)[1], Path::new("/data/Firestorm"));
    }

    #[test]
    fn an_accounts_folder_on_each_grid() {
        assert!(account("agni").owns("jane_doe"));
        assert!(!account("agni").owns("jane_doe.second_life_beta"));
        assert!(account("aditi").owns("jane_doe.second_life_beta"));
        assert!(!account("aditi").owns("jane_doe"));
        assert!(account("custom_x").owns("jane_doe.osgrid"));
        assert!(!account("custom_x").owns("jane_doe"));
        assert!(!account("custom_x").owns("jane_doe.second_life_beta"));
        assert!(!account("custom_x").grid_known());
        assert!(account("agni").grid_known() && account("aditi").grid_known());
        assert!(!account("agni").owns("jane_doe2"));
        let resident = Account {
            first: "JaneDoe".into(),
            last: "Resident".into(),
            grid: "agni".into(),
        };
        assert!(resident.owns("janedoe_resident"));
    }
}
