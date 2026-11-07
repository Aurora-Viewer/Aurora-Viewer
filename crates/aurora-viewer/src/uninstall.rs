//! Preferences › Language and updates › "Remove Aurora from this computer",
//! and the `--uninstall` command line: removes what the viewer registered
//! outside its own folders, then lists those folders for the user to delete
//! with the viewer.
//!
//! The Legacy Viewer's uninstaller (Section Uninstall and
//! un.UserSettingsFiles, indra/newview/installers/windows/
//! installer_template.nsi) deletes the link classes `secondlife`, `hop` and
//! `x-grid-location-info` whoever wrote them, and asks whether to delete the
//! settings folders. Aurora has no installer, so the viewer does it itself,
//! and only for what is its own: a link class that opens another program
//! stays. What it removes:
//!
//! - the link handler (`url_scheme.rs`), when it opens this executable;
//! - Windows' notification identity (`system_notify.rs`);
//! - the credential store's entries, deleted without being read: the cache
//!   key (`cache_key.rs`), each saved login's password hash and
//!   second-factor authorization (`credentials.rs`), the crash reports'
//!   GitHub authorization (`crash/github.rs`);
//! - Aurora's add-on in each Blender that has it (`blender_live/installer.rs`).
//!
//! The folders stay: the running viewer uses them, and deleting a folder
//! after the exit would need a process outliving the viewer, deleting
//! whatever path the settings name. The user deletes them, from the list
//! the result shows (`remaining`).
//!
//! The rules are pure functions over [`System`]: the computer itself
//! (`system::Computer`), or the demo's dry run (`system::DryRun`), which
//! only logs. Unit tests and the demo never get the first (`system::for_run`).

pub mod system;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// What the removal acts on: the computer, or a stand-in.
pub trait System {
    /// The link handler's places (registry classes, desktop files) that
    /// open this executable.
    fn links(&self) -> Vec<String>;
    fn remove_links(&mut self) -> Result<(), String>;
    /// Windows' notification identity (its registry key), when written.
    fn notification_id(&self) -> Option<String>;
    fn remove_notification_id(&mut self) -> Result<(), String>;
    /// Deletes one credential store entry, never reading it: false when
    /// there was none.
    fn delete_secret(&mut self, service: &str, account: &str) -> Result<bool, String>;
    /// Removes Aurora's add-on from each Blender found that has it.
    fn remove_blender_addon(&mut self) -> Vec<AddonRemoval>;
    /// Nothing is really removed (the demo).
    fn dry_run(&self) -> bool {
        false
    }
}

/// One Blender looked at (see `blender_live::installer::Removal`).
pub(crate) use crate::blender_live::AddonRemoval;

/// A saved login: its grid's login URI and its user name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub grid: String,
    pub user: String,
}

/// A credential store entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Secret {
    CacheKey,
    Password(Login),
    SecondFactor(Login),
    CrashReports,
}

impl Secret {
    /// (service, account) in the store.
    fn entry(&self) -> (&'static str, String) {
        match self {
            Secret::CacheKey => (crate::cache_key::STORE_ENTRY.0, crate::cache_key::STORE_ENTRY.1.to_owned()),
            Secret::CrashReports => (crate::crash::github::STORE_ENTRY.0, crate::crash::github::STORE_ENTRY.1.to_owned()),
            Secret::Password(login) => {
                let [password, _] = crate::credentials::store_entries(&login.grid, &login.user);
                password
            }
            Secret::SecondFactor(login) => {
                let [_, mfa] = crate::credentials::store_entries(&login.grid, &login.user);
                mfa
            }
        }
    }
}

/// One thing the removal acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// The link handler, at these places.
    Links(Vec<String>),
    /// Windows' notification identity, at this registry key.
    NotificationId(String),
    Secret(Secret),
    /// Aurora's add-on in the Blenders found.
    BlenderAddon,
}

/// What a removal will do, shown before it is confirmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub parts: Vec<Part>,
}

/// What became of one part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    Removed,
    /// Nothing to remove (a credential never saved, no Blender with the
    /// add-on).
    Absent,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Report {
    pub steps: Vec<(Part, Done)>,
    /// The Blenders the add-on was removed from, or not.
    pub blender: Vec<AddonRemoval>,
    /// What stays for the user to delete.
    pub remaining: Vec<Remaining>,
    pub dry_run: bool,
}

impl Report {
    pub fn failed(&self) -> bool {
        self.steps.iter().any(|(_, done)| matches!(done, Done::Failed(_)))
    }
}

/// What the removal will act on. The credentials are listed whether saved
/// or not: knowing it would mean reading the store.
pub fn plan(system: &dyn System, logins: &[Login]) -> Plan {
    let mut parts = Vec::new();
    let links = system.links();
    if !links.is_empty() {
        parts.push(Part::Links(links));
    }
    if let Some(key) = system.notification_id() {
        parts.push(Part::NotificationId(key));
    }
    parts.push(Part::Secret(Secret::CacheKey));
    let mut seen = std::collections::HashSet::new();
    for login in logins {
        // the store's account name: one entry per grid and user, whatever
        // the case of the name
        let (_, account) = Secret::Password(login.clone()).entry();
        if login.user.trim().is_empty() || !seen.insert(account) {
            continue;
        }
        parts.push(Part::Secret(Secret::Password(login.clone())));
        parts.push(Part::Secret(Secret::SecondFactor(login.clone())));
    }
    parts.push(Part::Secret(Secret::CrashReports));
    parts.push(Part::BlenderAddon);
    Plan { parts }
}

/// Carries the plan out, every part even after a failure.
pub fn run(plan: &Plan, system: &mut dyn System) -> (Vec<(Part, Done)>, Vec<AddonRemoval>) {
    let mut blender = Vec::new();
    let steps = plan
        .parts
        .iter()
        .map(|part| {
            let done = match part {
                Part::Links(_) => removed(system.remove_links()),
                Part::NotificationId(_) => removed(system.remove_notification_id()),
                Part::Secret(secret) => {
                    let (service, account) = secret.entry();
                    match system.delete_secret(service, &account) {
                        Ok(true) => Done::Removed,
                        Ok(false) => Done::Absent,
                        Err(e) => Done::Failed(e),
                    }
                }
                Part::BlenderAddon => {
                    blender = system.remove_blender_addon();
                    addon_done(&blender)
                }
            };
            (part.clone(), done)
        })
        .collect();
    (steps, blender)
}

fn removed(result: Result<(), String>) -> Done {
    match result {
        Ok(()) => Done::Removed,
        Err(e) => Done::Failed(e),
    }
}

/// The add-on's step: failed if one Blender failed, removed if one had it.
fn addon_done(blender: &[AddonRemoval]) -> Done {
    let errors: Vec<String> = blender
        .iter()
        .filter_map(|b| b.error.as_ref().map(|e| format!("{}: {e}", b.executable.display())))
        .collect();
    if !errors.is_empty() {
        Done::Failed(errors.join("; "))
    } else if blender.iter().any(|b| !b.removed.is_empty()) {
        Done::Removed
    } else {
        Done::Absent
    }
}

/// What a folder left behind holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Settings,
    Cache,
    ChosenCache,
    Logs,
    MediaPlugins,
    Transcripts,
    BlenderBackups,
    /// The desktop entry and icon a Linux user installed from the archive.
    MenuEntry,
    /// The viewer's own executable (its bundle on macOS).
    Viewer,
}

/// A folder (or a file) left for the user to delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remaining {
    pub kinds: Vec<Kind>,
    pub path: PathBuf,
}

impl Remaining {
    /// The folder to open: the path itself, or the one that holds it.
    pub fn folder(&self, is_dir: impl Fn(&Path) -> bool) -> PathBuf {
        if is_dir(&self.path) {
            self.path.clone()
        } else {
            self.path.parent().map_or_else(|| self.path.clone(), Path::to_path_buf)
        }
    }
}

/// The places that exist, each once: one inside another joins it (macOS
/// keeps the logs in the settings folder), in the order given.
pub fn remaining(candidates: Vec<(Kind, PathBuf)>, exists: impl Fn(&Path) -> bool) -> Vec<Remaining> {
    let mut out: Vec<Remaining> = Vec::new();
    for (kind, path) in candidates {
        if path.as_os_str().is_empty() || !exists(&path) {
            continue;
        }
        if let Some(outer) = out.iter_mut().find(|r| path.starts_with(&r.path)) {
            if !outer.kinds.contains(&kind) {
                outer.kinds.push(kind);
            }
            continue;
        }
        // the new place holds some listed earlier: they join it, where the
        // first of them was
        let mut kinds = vec![kind];
        let mut at = None;
        let mut i = 0;
        while i < out.len() {
            if out[i].path.starts_with(&path) {
                at.get_or_insert(i);
                for inner in out.remove(i).kinds {
                    if !kinds.contains(&inner) {
                        kinds.push(inner);
                    }
                }
            } else {
                i += 1;
            }
        }
        out.insert(at.unwrap_or(out.len()), Remaining { kinds, path });
    }
    out
}

/// The application to delete: the bundle that holds the executable on
/// macOS, else the executable.
pub fn viewer_location(exe: &Path) -> PathBuf {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
        .unwrap_or(exe)
        .to_path_buf()
}

/// A path as the user can type it: the longest known folder it is in
/// replaced by its name (`%LOCALAPPDATA%`, `~`), without the user name.
pub fn shown(path: &Path, bases: &[(&str, PathBuf)]) -> String {
    let best = bases
        .iter()
        .filter(|(_, base)| !base.as_os_str().is_empty())
        .filter_map(|(name, base)| path.strip_prefix(base).ok().map(|rest| (name, base, rest)))
        .max_by_key(|(_, base, _)| base.components().count());
    match best {
        Some((name, _, rest)) if rest.as_os_str().is_empty() => (*name).to_owned(),
        Some((name, _, rest)) => format!("{name}{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

/// The folders `shown` names, from the environment.
pub fn known_bases() -> Vec<(&'static str, PathBuf)> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from).unwrap_or_default();
    if cfg!(windows) {
        vec![
            ("%LOCALAPPDATA%", var("LOCALAPPDATA")),
            ("%APPDATA%", var("APPDATA")),
            ("%USERPROFILE%", var("USERPROFILE")),
        ]
    } else {
        vec![("~", var("HOME"))]
    }
}

/// Where the viewer keeps its files, from the path helpers the viewer
/// itself uses (the demo's subfolders are inside them). `chosen_cache` and
/// `transcripts`: the folders the user chose in the preferences.
pub fn folder_candidates(chosen_cache: &[PathBuf], transcripts: Option<PathBuf>, blender: &[AddonRemoval]) -> Vec<(Kind, PathBuf)> {
    let mut out = vec![
        (Kind::Settings, crate::settings::default_config_root()),
        (Kind::Cache, crate::settings::default_cache_root()),
    ];
    out.extend(chosen_cache.iter().map(|dir| (Kind::ChosenCache, dir.clone())));
    out.push((Kind::Logs, crate::logging::logs_dir()));
    out.push((Kind::MediaPlugins, crate::settings::media_plugins_dir()));
    out.extend(transcripts.map(|dir| (Kind::Transcripts, dir)));
    out.extend(
        blender
            .iter()
            .filter_map(|b| b.backups.clone().map(|dir| (Kind::BlenderBackups, dir))),
    );
    // docs/LINUX.md, "Start the viewer": installed by the user, from the archive
    if cfg!(target_os = "linux")
        && let Some(base) = directories::BaseDirs::new()
    {
        let data = base.data_dir();
        out.push((Kind::MenuEntry, data.join("applications").join("aurora-viewer.desktop")));
        out.push((Kind::MenuEntry, data.join("icons/hicolor/scalable/apps/aurora-viewer.svg")));
    }
    if let Ok(exe) = std::env::current_exe() {
        out.push((Kind::Viewer, viewer_location(&exe)));
    }
    out
}

/// The interface's name of a part.
pub fn part_label(part: &Part) -> String {
    use aurora_i18n::tr;
    match part {
        Part::Links(_) => tr!("uninstall-links").to_owned(),
        Part::NotificationId(_) => tr!("uninstall-notification-id").to_owned(),
        Part::Secret(Secret::CacheKey) => tr!("uninstall-cache-key").to_owned(),
        Part::Secret(Secret::CrashReports) => tr!("uninstall-crash-reports").to_owned(),
        Part::Secret(Secret::Password(login)) => {
            tr!("uninstall-password", user = login.user.clone(), grid = grid_label(&login.grid)).to_owned()
        }
        Part::Secret(Secret::SecondFactor(login)) => {
            tr!("uninstall-second-factor", user = login.user.clone(), grid = grid_label(&login.grid)).to_owned()
        }
        Part::BlenderAddon => tr!("uninstall-blender").to_owned(),
    }
}

/// Where a part is on this system (registry keys, files), shown under it.
pub fn part_places(part: &Part) -> Vec<String> {
    match part {
        Part::Links(places) => places.clone(),
        Part::NotificationId(key) => vec![key.clone()],
        _ => Vec::new(),
    }
}

/// A grid as the login screen names it.
pub fn grid_label(uri: &str) -> String {
    match uri.trim() {
        aurora_net::login::SL_MAIN_GRID => "Second Life".to_owned(),
        aurora_net::login::SL_BETA_GRID => "Second Life Beta".to_owned(),
        other => other.to_owned(),
    }
}

pub fn kind_label(kind: Kind) -> &'static str {
    use aurora_i18n::tr;
    match kind {
        Kind::Settings => tr!("uninstall-kind-settings"),
        Kind::Cache => tr!("uninstall-kind-cache"),
        Kind::ChosenCache => tr!("uninstall-kind-chosen-cache"),
        Kind::Logs => tr!("uninstall-kind-logs"),
        Kind::MediaPlugins => tr!("uninstall-kind-media-plugins"),
        Kind::Transcripts => tr!("uninstall-kind-transcripts"),
        Kind::BlenderBackups => tr!("uninstall-kind-blender-backups"),
        Kind::MenuEntry => tr!("uninstall-kind-menu-entry"),
        Kind::Viewer => tr!("uninstall-kind-viewer"),
    }
}

pub fn kinds_label(kinds: &[Kind]) -> String {
    kinds.iter().map(|k| kind_label(*k)).collect::<Vec<_>>().join(" · ")
}

pub fn done_label(done: &Done) -> String {
    use aurora_i18n::tr;
    match done {
        Done::Removed => tr!("uninstall-removed").to_owned(),
        Done::Absent => tr!("uninstall-absent").to_owned(),
        Done::Failed(error) => tr!("uninstall-failed", error = error.clone()).to_owned(),
    }
}

/// The saved logins whose credentials may be in the store: the account
/// catalogue, and the one of the settings.
pub fn saved_logins(current: Option<Login>) -> Vec<Login> {
    crate::login_accounts::load()
        .into_iter()
        .map(|account| Login {
            grid: account.uri,
            user: account.username,
        })
        .chain(current)
        .collect()
}

/// `--uninstall`: the removal without a window, what it did printed. The
/// exit code is 1 when a part could not be removed.
pub fn command_line() -> i32 {
    #[cfg(windows)]
    attach_console();
    let settings = crate::settings::Settings::load();
    // the saved language, unless the command line chose one
    aurora_i18n::select(&settings.language, crate::cli::get().language.as_deref());
    let mut system = system::for_run();
    let current = Login {
        grid: settings.login_uri(),
        user: settings.username.clone(),
    };
    let logins = if system.dry_run() {
        system::demo_logins()
    } else {
        saved_logins(Some(current))
    };
    let plan = plan(system.as_ref(), &logins);
    let (steps, blender) = run(&plan, system.as_mut());
    let chosen_cache: Vec<PathBuf> = crate::cache_location::root_for(&settings.cache_location).into_iter().collect();
    let transcripts = Some(PathBuf::from(settings.chat_log_location.trim())).filter(|p| !p.as_os_str().is_empty());
    let report = Report {
        remaining: remaining(folder_candidates(&chosen_cache, transcripts, &blender), Path::exists),
        steps,
        blender,
        dry_run: system.dry_run(),
    };
    println!("{}", printed(&report));
    i32::from(report.failed())
}

/// The command line's text of a report.
pub fn printed(report: &Report) -> String {
    use aurora_i18n::tr;
    let mut lines = vec![tr!("uninstall-window").to_owned()];
    if report.dry_run {
        lines.push(tr!("uninstall-dry-run").to_owned());
    }
    lines.push(String::new());
    for (part, done) in &report.steps {
        lines.push(format!("  {} — {}", part_label(part), done_label(done)));
        for place in part_places(part) {
            lines.push(format!("      {place}"));
        }
        if *part == Part::BlenderAddon {
            for blender in &report.blender {
                for path in &blender.removed {
                    lines.push(format!("      {}", path.display()));
                }
            }
        }
    }
    lines.push(String::new());
    lines.push(tr!("uninstall-left").to_owned());
    lines.push(tr!("uninstall-left-text").to_owned());
    for place in &report.remaining {
        lines.push(format!("  {}: {}", kinds_label(&place.kinds), place.path.display()));
    }
    lines.join("\n")
}

/// The release build has no console of its own: its text goes to the
/// terminal that started it, when its output is not already redirected.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // SAFETY: no pointers; a failure (no parent console) leaves the process
    // as it was.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
