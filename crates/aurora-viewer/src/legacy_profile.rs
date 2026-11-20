//! Where a Legacy Viewer keeps an account's files on this computer, for what
//! is imported from it: the folder of each viewer, and the account's folder
//! in it. Nothing here writes; these folders are another viewer's.
//!
//! The folders follow LLDir_Linux, LLDir_Win32 and LLDir_Mac::initAppDirs and
//! LLDir::setLindenUserDir (indra/llfilesystem, originally LGPL 2.1).

use std::path::{Path, PathBuf};

/// The per-user folders read, by application name: the `initAppDirs` name
/// of each viewer of the Third Party Viewer Directory whose transcripts use
/// the LL line format, the 64-bit name first, then the macOS and former one.
/// `SecondLife` is also the folder of the RLV viewer, and of the Lovense
/// viewer on macOS; Aperture shares `Firestorm` there. A name a system does
/// not use costs one failed directory read. Viewers with another line
/// format (Singularity, Genesis, Cool VL) and any other folder go through
/// the folder dialog.
const APPS: [&str; 14] = [
    "Firestorm_x64",
    "Firestorm",
    "SecondLife",
    "AlchemyNext",
    // Alchemy's 2025-2026 test and develop builds
    "AlchemyViewer",
    "Alchemy",
    "Aperture Viewer_x64",
    "BlackDragon",
    // the DirectX builds, then the OpenGL ones (Windows only)
    "Kirstens S24 DX",
    "Kirstens S24",
    "Kokua",
    "Catznip",
    "Megapahit",
    "SLLovenseViewer",
];

/// Alchemy's Linux folder since it follows the XDG layout
/// (`~/.local/share/alchemynext`, LLDir_Linux of the Alchemy sources); older
/// builds keep `~/.alchemynext`, which `APPS` covers.
const XDG_APPS: [&str; 1] = ["alchemynext"];

/// Label of the beta grid in the folder names (SECOND_LIFE_BETA_LABEL): the
/// account's beta-grid folder when none exists yet.
const BETA_SUFFIX: &str = ".second_life_beta";

/// Whether `label`, after `<first_last>.`, names the beta grid: a viewer
/// writes its grid label there, lower-cased with spaces as `_`
/// (LLDir::setPerAccountChatLogsDir), and the beta grid's labels in the
/// viewers' grid lists name it "Second Life Beta" or "Aditi". Another
/// grid's label (an OpenSim grid, the demo) does not, so its files never mix
/// with the beta grid's.
fn is_beta_label(label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    label.contains("beta") || label.contains("aditi")
}

/// The account's beta-grid folders in `root` (`<base>.<beta label>`, any
/// case), the most recently changed first.
fn beta_folders(root: &Path, base: &str) -> Vec<String> {
    let found = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            beta_label_of(&name, base)?;
            let changed = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            Some((changed, name))
        })
        .collect();
    newest_first(found)
}

/// Folder names by the time they last changed, the most recent first (then
/// by name, for a stable choice).
fn newest_first(mut found: Vec<(std::time::SystemTime, String)>) -> Vec<String> {
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, name)| name).collect()
}

/// The beta-grid label of an account folder's name `<base>.<label>`.
fn beta_label_of<'a>(name: &'a str, base: &str) -> Option<&'a str> {
    let head = name.get(..base.len())?;
    let label = name.get(base.len()..)?.strip_prefix('.')?;
    (head.eq_ignore_ascii_case(base) && !label.is_empty() && is_beta_label(label)).then_some(label)
}

/// Which viewer family owns a chat log root folder (`APPS`, `XDG_APPS`), or
/// `Other` when the user chose another folder: what its new 1:1 files are
/// named like when the folder has none yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootViewer {
    /// The Legacy Viewer and Aperture.
    Legacy,
    Alchemy,
    SecondLife,
    BlackDragon,
    Catznip,
    /// Other known third-party viewers listed in `APPS` (`Kokua`, `Kirstens S24`,
    /// `Kirstens S24 DX`, `Megapahit`, `SLLovenseViewer`).
    ThirdParty,
    Other,
}

impl RootViewer {
    fn of(root: &Path) -> Self {
        let Some(raw) = root.file_name().and_then(|n| n.to_str()) else {
            return Self::Other;
        };
        let name = raw.trim_start_matches('.').to_ascii_lowercase();
        if name.starts_with("firestorm") || name.starts_with("aperture") {
            Self::Legacy
        } else if name.starts_with("alchemy") {
            Self::Alchemy
        } else if name == "secondlife" {
            Self::SecondLife
        } else if name == "blackdragon" {
            Self::BlackDragon
        } else if name == "catznip" {
            Self::Catznip
        } else if matches!(
            name.as_str(),
            "kokua" | "kirstens s24 dx" | "kirstens s24" | "megapahit" | "sllovenseviewer"
        ) {
            Self::ThirdParty
        } else {
            Self::Other
        }
    }
}

/// Whether `root` belongs to a viewer whose default 1:1 transcript naming
/// scheme is the username form (`jane_doe.txt`: SecondLife, Alchemy,
/// BlackDragon) rather than the Legacy form (`Jane Doe.txt`: the Legacy Viewer,
/// Catznip, other TPVs or an unknown folder).
pub fn uses_username_log_names(root: Option<&Path>) -> bool {
    matches!(
        root.map(RootViewer::of),
        Some(RootViewer::SecondLife | RootViewer::Alchemy | RootViewer::BlackDragon)
    )
}

/// Whether `root`'s name is a known Second Life or third-party viewer
/// application folder (`APPS` / `XDG_APPS`).
pub fn is_viewer_root(root: &Path) -> bool {
    RootViewer::of(root) != RootViewer::Other
}

fn has_subfolder(root: &Path, name: &str) -> bool {
    if root.join(name).is_dir() {
        return true;
    }
    std::fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(name)) && e.file_type().is_ok_and(|t| t.is_dir()))
}

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

    /// The account's folder in a viewer's chat log root `root`
    /// (LLDir::setPerAccountChatLogsDir, indra/llfilesystem/lldir.cpp,
    /// originally LGPL 2.1: `<first_last>`, then `.` and the grid's label,
    /// lower-cased with spaces as `_`, unless that label is `second_life`):
    /// `first_last` on the main grid, `.demo` for the demo. On the beta grid
    /// each viewer writes its own label for it, so a beta-grid folder that
    /// already exists is taken (the most recently changed when there are
    /// several), else `.second_life_beta`; never the bare main-grid folder,
    /// even where a viewer writes the beta grid's files there. Lists `root`
    /// on the beta grid: placement only, off the frame. None on another
    /// grid, whose label only the other viewer knows (`grid_known`).
    pub fn chat_folder(&self, root: &Path) -> Option<String> {
        let base = self.folder();
        match self.grid.as_str() {
            "agni" => Some(base),
            "aditi" => Some(
                beta_folders(root, &base)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| format!("{base}{BETA_SUFFIX}")),
            ),
            grid if self.grid_known() => Some(format!("{base}.{grid}")),
            _ => None,
        }
    }

    /// Whether `dir` is itself this account's Legacy Viewer subfolder (for
    /// example when the user picked `<root>/<first_last>` directly instead
    /// of `<root>`). A bare `<custom>/<first_last>` outside a known viewer
    /// root only counts when it already holds Legacy Viewer files, so a custom
    /// root whose folder name happens to match `first_last` stays a root.
    pub fn is_account_dir(&self, dir: &Path) -> bool {
        let Some(leaf) = dir.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        let base = self.folder();
        let in_account_layout =
            || dir.parent().is_some_and(is_viewer_root) || crate::world::chat_log::names::has_legacy_account_files(dir, false);
        match self.grid.as_str() {
            "agni" => leaf.eq_ignore_ascii_case(&base) && !has_subfolder(dir, &base) && in_account_layout(),
            // a beta-grid folder only, never the bare main-grid one
            "aditi" => beta_label_of(leaf, &base).is_some(),
            grid if self.grid_known() => leaf.eq_ignore_ascii_case(&format!("{base}.{grid}")),
            _ => false,
        }
    }

    /// When logged into Aditi (`aditi`) and `dir` is this account's own
    /// main-grid folder (`<first_last>` under a viewer root or holding
    /// transcripts) rather than a subfolder accepted by `is_account_dir`,
    /// returns the leaf name so UI can warn about the grid mismatch instead
    /// of blaming another account.
    pub fn is_own_main_grid_dir(&self, dir: &Path) -> Option<String> {
        if self.grid != "aditi" {
            return None;
        }
        let leaf = dir.file_name()?.to_str()?;
        let base = self.folder();
        if !leaf.eq_ignore_ascii_case(&base) || self.is_account_dir(dir) {
            return None;
        }
        let looks_like_account =
            dir.parent().is_some_and(is_viewer_root) || crate::world::chat_log::names::has_legacy_account_files(dir, false);
        looks_like_account.then(|| leaf.to_owned())
    }

    /// Resolve the account's shared chat folder and the viewer root it lives
    /// under for a user-chosen `root` (which may be either `<viewer_root>`
    /// or the account subfolder `<viewer_root>/<account>` itself).
    pub fn resolve_chat_dir(&self, root: &Path) -> Option<(PathBuf, PathBuf)> {
        if self.is_account_dir(root) {
            let viewer_root = root.parent().unwrap_or(root).to_path_buf();
            return Some((root.to_path_buf(), viewer_root));
        }
        let name = self.chat_folder(root)?;
        Some((root.join(name), root.to_path_buf()))
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
            "aditi" => beta_label_of(name, &base).is_some(),
            _ if !self.grid_known() => name
                .strip_prefix(base.as_str())
                .and_then(|rest| rest.strip_prefix('.'))
                .is_some_and(|label| !label.is_empty() && !is_beta_label(label)),
            _ => name == base,
        }
    }
}

/// `<user folder>/<application>` for a home folder and the system's folder
/// of application data: `~/.firestorm_x64` on Linux, `%APPDATA%\\Firestorm_x64`
/// on Windows and `~/Library/Application Support/Firestorm` on macOS. On
/// Linux `data` is `~/.local/share`, for the XDG folders.
fn app_folders_in(home: &Path, data: &Path, dotted: bool) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = APPS
        .iter()
        .map(|app| {
            if dotted {
                home.join(format!(".{}", app.to_lowercase()))
            } else {
                data.join(app)
            }
        })
        .collect();
    if dotted {
        folders.extend(XDG_APPS.iter().map(|app| data.join(app)));
    }
    folders
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
        // the macOS build's folder
        assert_eq!(app_folders_in(home, data, false)[1], Path::new("/data/Firestorm"));
    }

    #[test]
    fn the_other_viewers_folders() {
        let (home, data) = (Path::new("/home/j"), Path::new("/data"));
        let windows = app_folders_in(home, data, false);
        for app in [
            "BlackDragon",
            "Kokua",
            "Catznip",
            "Megapahit",
            "Aperture Viewer_x64",
            "Kirstens S24 DX",
        ] {
            assert!(windows.contains(&data.join(app)), "{app}");
        }
        // no XDG folder off Linux
        assert_eq!(windows.len(), APPS.len());
        let linux = app_folders_in(home, data, true);
        assert!(linux.contains(&home.join(".blackdragon")));
        assert!(linux.contains(&home.join(".aperture viewer_x64")));
        assert_eq!(linux.last(), Some(&data.join("alchemynext")));
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

    #[test]
    fn the_chat_folder_written_like_the_legacy_viewer() {
        let root = Path::new("/data/Firestorm_x64");
        assert_eq!(account("agni").chat_folder(root).as_deref(), Some("jane_doe"));
        assert_eq!(account("aditi").chat_folder(root).as_deref(), Some("jane_doe.second_life_beta"));
        // never the main grid's folder
        assert_eq!(account("demo").chat_folder(root).as_deref(), Some("jane_doe.demo"));
        assert_eq!(account("custom_osgrid_org").chat_folder(root), None);
        // the folder it writes is the one the import reads
        for grid in ["agni", "aditi"] {
            let a = account(grid);
            assert!(a.owns(&a.chat_folder(root).expect("folder")), "{grid}");
        }
    }

    #[test]
    fn the_beta_grid_folder_by_the_general_rule() {
        let aditi = account("aditi");
        // no beta-grid folder yet: `.second_life_beta` under every root, a
        // known viewer's or not
        for root in [
            "/data/Firestorm_x64",
            "/data/AlchemyNext",
            "/data/SecondLife",
            "/data/BlackDragon",
            "/data/Catznip",
            "/home/j/.local/share/alchemynext",
        ] {
            assert_eq!(
                aditi.chat_folder(Path::new(root)).as_deref(),
                Some("jane_doe.second_life_beta"),
                "{root}"
            );
        }
        // the beta grid's labels, never another grid's nor the main grid's
        for (name, beta) in [
            ("jane_doe.second_life_beta", true),
            ("Jane_Doe.Aditi", true),
            ("jane_doe.second_life_beta_(aditi)", true),
            ("jane_doe", false),
            ("jane_doe.", false),
            ("jane_doe.osgrid", false),
            ("jane_doe.demo", false),
            ("john_doe.aditi", false),
        ] {
            assert_eq!(beta_label_of(name, "jane_doe").is_some(), beta, "{name}");
            assert_eq!(aditi.owns(name), beta, "{name}");
        }
        // several: the most recently changed, then by name
        let at = |s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s);
        assert_eq!(
            newest_first(vec![
                (at(10), "jane_doe.second_life_beta".into()),
                (at(20), "jane_doe.aditi".into()),
                (at(20), "jane_doe.aditi_beta".into()),
            ]),
            ["jane_doe.aditi", "jane_doe.aditi_beta", "jane_doe.second_life_beta"]
        );

        // an existing beta-grid folder is taken, whichever viewer's root;
        // the bare main-grid one and another grid's never
        let custom = std::env::temp_dir().join(format!("aurora-legacy-profile-{}", uuid::Uuid::new_v4()));
        for root in [custom.join("Firestorm_x64"), custom.join("SecondLife"), custom.join("archive")] {
            std::fs::create_dir_all(root.join("jane_doe")).expect("main-grid folder");
            std::fs::create_dir_all(root.join("jane_doe.osgrid")).expect("another grid's folder");
            assert_eq!(aditi.chat_folder(&root).as_deref(), Some("jane_doe.second_life_beta"), "{root:?}");
            std::fs::create_dir_all(root.join("jane_doe.aditi")).expect("beta-grid folder");
            assert_eq!(aditi.chat_folder(&root).as_deref(), Some("jane_doe.aditi"), "{root:?}");
            // the folder it writes is the one the import reads
            assert!(aditi.owns("jane_doe.aditi"));
            assert!(aditi.is_account_dir(&root.join("jane_doe.aditi")));
            assert!(!aditi.is_account_dir(&root.join("jane_doe")));
            assert_eq!(
                aditi.resolve_chat_dir(&root.join("jane_doe.aditi")),
                Some((root.join("jane_doe.aditi"), root.clone()))
            );
        }
        // on the beta grid, the bare folder is the account's main-grid one,
        // under any viewer's root
        for root in ["Firestorm_x64", "SecondLife"] {
            let bare = custom.join(root).join("jane_doe");
            assert_eq!(aditi.is_own_main_grid_dir(&bare).as_deref(), Some("jane_doe"), "{root}");
        }

        // Selecting an empty custom root named `<first_last>` stays a root, while
        // `<viewer_root>/<first_last>` or a custom folder holding transcripts normalizes.
        let agni = account("agni");
        let fs_root = custom.join("Firestorm_x64");
        let empty_custom_named_root = custom.join("nested_empty").join("jane_doe");
        std::fs::create_dir_all(&empty_custom_named_root).expect("empty custom root");
        assert!(!agni.is_account_dir(&empty_custom_named_root));
        assert_eq!(
            agni.resolve_chat_dir(&empty_custom_named_root),
            Some((empty_custom_named_root.join("jane_doe"), empty_custom_named_root.clone()))
        );
        let populated_custom_sub = custom.join("transcripts").join("jane_doe");
        std::fs::create_dir_all(&populated_custom_sub).expect("populated custom account");
        std::fs::write(populated_custom_sub.join("Bob.txt"), "[2026/10/12 14:00]  Bob: hi\n").unwrap();
        assert_eq!(
            agni.resolve_chat_dir(&populated_custom_sub),
            Some((populated_custom_sub.clone(), custom.join("transcripts")))
        );
        assert_eq!(
            agni.resolve_chat_dir(&fs_root.join("jane_doe")),
            Some((fs_root.join("jane_doe"), fs_root.clone()))
        );
        assert_eq!(aditi.is_own_main_grid_dir(&empty_custom_named_root), None);
        assert_eq!(aditi.is_own_main_grid_dir(&fs_root.join("jane_doe.aditi")), None);
        for viewer in [
            "Firestorm_x64",
            "SecondLife",
            "AlchemyNext",
            "Kokua",
            "Kirstens S24",
            "Kirstens S24 DX",
            "Megapahit",
            "SLLovenseViewer",
        ] {
            assert!(is_viewer_root(&custom.join(viewer)), "{viewer}");
        }
        assert!(!is_viewer_root(&custom));
        std::fs::remove_dir_all(&custom).expect("cleanup");
    }
}
