//! Where an account's transcripts go, and what a folder picked in the
//! preferences is for this account. Both read the folders (a directory
//! listing, the head of a file): on OneDrive or a network share that can
//! take seconds, so they run on the `aurora-chatlog` thread
//! (`store::Worker::place`, `store::Worker::check`), never on the frame,
//! and the answer is applied when it comes back.
//!
//! The account's folder follows LLDir::setPerAccountChatLogsDir
//! (indra/llfilesystem/lldir.cpp, originally LGPL 2.1) through
//! `legacy_profile::Account`.

use super::{import, names, store};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// What a placement is asked for.
#[derive(Debug, Clone)]
pub struct Request {
    /// The chosen folder of the accounts (InstantMessageLogPath); None: the
    /// default one.
    pub accounts: Option<PathBuf>,
    /// The default folder of the accounts when it is not the settings'
    /// (tests).
    pub default_accounts: Option<PathBuf>,
    pub identity: store::Identity,
    /// The account in the Legacy layout, when the login gave its name.
    pub account: Option<import::Account>,
    /// The user said yes to sharing the Legacy Viewer's folder
    /// (`log_legacy_layout`).
    pub share: bool,
}

/// Where the account's files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub dir: PathBuf,
    /// `dir` is a Legacy Viewer's account folder, shared with it: Aurora
    /// only appends lines there.
    pub shared: bool,
    /// The 1:1 file names of `dir` when shared.
    pub scheme: names::NamingScheme,
    /// The Legacy Viewer's transcripts found in `dir`.
    pub legacy_count: usize,
}

/// The account's folder under the chosen folder of the accounts. Shared
/// only with the user's yes; a folder that belongs to another account, or
/// on the beta grid to the account's main-grid files, falls back to the
/// default folder.
pub fn place(req: &Request) -> Placement {
    let account = req.account.as_ref();
    let accounts = req.accounts.as_deref().filter(|root| acceptable(root, req.identity.agent, account));
    if req.share
        && let Some(root) = accounts
        && let Some((dir, viewer_root)) = account.and_then(|a| a.resolve_chat_dir(root))
    {
        let (scheme, legacy_count) = names::probe_shared_dir(&dir, Some(&viewer_root));
        return Placement {
            dir,
            shared: true,
            scheme,
            legacy_count,
        };
    }
    let root = accounts
        .map(Path::to_path_buf)
        .or_else(|| req.default_accounts.clone())
        .unwrap_or_else(crate::settings::accounts_dir);
    Placement {
        dir: store::folder(&root, &req.identity),
        shared: false,
        scheme: names::NamingScheme::Legacy,
        legacy_count: 0,
    }
}

/// Neither another account's folder nor, on the beta grid, the account's
/// main-grid folder.
fn acceptable(root: &Path, agent: Uuid, account: Option<&import::Account>) -> bool {
    if let Some(label) = account.and_then(|a| a.is_own_main_grid_dir(root)) {
        log::warn!(
            "conversation transcripts: refusing to use main-grid folder {} ({label}) on this grid; falling back to default folder",
            root.display()
        );
        return false;
    }
    if foreign_dir_label(root, agent, account).is_none() {
        return true;
    }
    // neither the path nor the folder's name: both name another account
    log::warn!("conversation transcripts: refusing another account's folder; falling back to default folder");
    false
}

/// Why a picked folder is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// On the beta grid, the account's main-grid Legacy folder.
    MainGrid,
    /// Another account's folder (Aurora's or a Legacy Viewer's).
    OtherAccount,
}

/// What a folder picked in the preferences is for the account logged in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderCheck {
    /// Not usable: the folder's name, and this account's own folder beside
    /// it, `(viewer root, folder name)`, to offer instead.
    Refused {
        why: Refusal,
        folder: String,
        offer: Option<(PathBuf, String)>,
    },
    /// A Legacy Viewer's folder for this account: its root, the account
    /// folder's name and the transcripts in it. Sharing it needs the user's
    /// yes.
    Legacy { root: PathBuf, folder: String, count: usize },
    /// Any other folder: Aurora's own layout under `root`.
    Plain { root: PathBuf },
}

/// The checks of a picked folder (the account's own subfolder normalized to
/// its root).
pub fn check(picked: &Path, agent: Uuid, account: Option<&import::Account>) -> FolderCheck {
    let refused = account
        .and_then(|a| a.is_own_main_grid_dir(picked))
        .map(|folder| (Refusal::MainGrid, folder))
        .or_else(|| foreign_dir_label(picked, agent, account).map(|folder| (Refusal::OtherAccount, folder)));
    if let Some((why, folder)) = refused {
        let offer = picked.parent().and_then(|parent| matching_subfolder_in(parent, agent, account));
        return FolderCheck::Refused { why, folder, offer };
    }
    let Some((dir, viewer_root)) = account.and_then(|a| a.resolve_chat_dir(picked)) else {
        return FolderCheck::Plain {
            root: picked.to_path_buf(),
        };
    };
    let count = names::legacy_log_count(&dir);
    if (count > 0 || is_legacy_chat_root(picked, &dir, &viewer_root))
        && let Some(folder) = dir.file_name().and_then(|n| n.to_str())
    {
        return FolderCheck::Legacy {
            root: viewer_root,
            folder: folder.to_owned(),
            count,
        };
    }
    FolderCheck::Plain { root: viewer_root }
}

/// This account's own folder in `root` (the parent of a refused folder):
/// one holding transcripts, or one that exists, `(viewer root, folder
/// name)`, when that root is itself acceptable.
fn matching_subfolder_in(root: &Path, agent: Uuid, account: Option<&import::Account>) -> Option<(PathBuf, String)> {
    let (dir, viewer_root) = account?.resolve_chat_dir(root)?;
    if names::legacy_log_count(&dir) == 0 && !(dir != root && dir.is_dir()) {
        return None;
    }
    let folder = dir.file_name()?.to_str()?.to_owned();
    let refused =
        account.and_then(|a| a.is_own_main_grid_dir(&viewer_root)).is_some() || foreign_dir_label(&viewer_root, agent, account).is_some();
    (!refused).then_some((viewer_root, folder))
}

/// A Legacy Viewer's folder: a known viewer's root, an account folder that
/// already exists, or one holding its transcripts or account files.
fn is_legacy_chat_root(root: &Path, dir: &Path, viewer_root: &Path) -> bool {
    crate::legacy_profile::is_viewer_root(viewer_root) || (dir != root && dir.is_dir()) || names::has_legacy_account_files(dir, false)
}

/// The name of `dir` when it is another account's folder: another
/// `<agent>` folder of Aurora's, or a Legacy Viewer account folder that is
/// not this account's.
fn foreign_dir_label(dir: &Path, me: Uuid, account: Option<&import::Account>) -> Option<String> {
    if account.is_some_and(|a| a.is_account_dir(dir) || a.is_own_main_grid_dir(dir).is_some()) {
        return None;
    }
    if let Some((agent, label)) = aurora_account_uuid(dir)
        && agent != me
    {
        return Some(label);
    }
    let under_viewer_root = dir.parent().is_some_and(crate::legacy_profile::is_viewer_root);
    if names::has_legacy_account_files(dir, under_viewer_root) {
        return dir.file_name().and_then(|n| n.to_str()).map(str::to_owned);
    }
    None
}

/// The account of an Aurora folder: `<agent>`, `<agent>/chat_logs` or
/// `<agent>/chat_logs/<grid>`.
fn aurora_account_uuid(path: &Path) -> Option<(Uuid, String)> {
    let leaf = path.file_name()?.to_str()?;
    if let Ok(id) = Uuid::parse_str(leaf)
        && path.join("chat_logs").is_dir()
    {
        return Some((id, leaf.to_owned()));
    }
    if leaf.eq_ignore_ascii_case("chat_logs")
        && let Some(agent_dir) = path.parent()
        && let Some(label) = agent_dir.file_name().and_then(|n| n.to_str())
        && let Ok(id) = Uuid::parse_str(label)
    {
        return Some((id, label.to_owned()));
    }
    if let Some(chat_logs) = path.parent()
        && chat_logs
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("chat_logs"))
        && let Some(agent_dir) = chat_logs.parent()
        && let Some(label) = agent_dir.file_name().and_then(|n| n.to_str())
        && let Ok(id) = Uuid::parse_str(label)
    {
        return Some((id, label.to_owned()));
    }
    None
}
