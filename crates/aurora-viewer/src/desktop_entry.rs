//! Aurora's menu entry and icon on Linux, installed by the viewer itself.
//!
//! A Wayland window has no icon of its own: the desktop finds it through the
//! window's app_id (`cli::DESKTOP_ID`), the name of an installed desktop entry
//! whose `Icon=` it shows in the panel, the dock and the task switcher. The
//! archive ships the entry and the icon, but nobody installed them, so the
//! window showed the desktop's default icon. At each start the viewer writes
//! them to the user's own folders (`$XDG_DATA_HOME`, `~/.local/share`):
//! `applications/aurora-viewer.desktop`, with this viewer's path in `Exec`,
//! and `icons/hicolor/scalable/apps/aurora-viewer.svg`, the colour logo.
//!
//! What the viewer did not write stays as it is: an entry the user installed
//! by hand (docs/LINUX.md before this, or one with `Hidden=true` to keep the
//! viewer out of the menu) or a distribution's package's, in a system
//! folder. An entry it wrote carries `X-Aurora-Managed=true`; it follows the
//! viewer when the viewer moves, its icon is put back when missing or old,
//! and it goes when a package installed later brings its own. A development
//! build never takes it from an installed viewer that is still there. Inside Flatpak or Snap
//! the package brings its own. The Legacy Viewer writes its entry from an
//! install script (`linux_tools/refresh_desktop_app_entry.sh`) instead.
use std::path::{Path, PathBuf};

/// The shipped entry, its `Exec` replaced with this viewer's path.
const ENTRY: &str = include_str!("../../../packaging/linux/aurora-viewer.desktop");
/// The colour logo (docs/BRANDING.md), shipped as `aurora-viewer.svg`.
const ICON: &[u8] = include_bytes!("../../../assets/branding/aurora-logo.svg");
/// The key that marks an entry this viewer wrote and may rewrite.
const MANAGED: &str = "X-Aurora-Managed=true";

/// What to do with the user's entry.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Write (or rewrite) the entry, and the icon.
    Write,
    /// The entry is ours and right: only the icon is checked.
    Keep,
    /// Ours, but a package installed since has its own: ours goes, so the
    /// package's answers for the app_id (the user's entry hides the
    /// system one).
    Remove,
    /// Someone else's (installed by hand, hidden on purpose) or a
    /// package's: nothing is touched.
    Leave,
}

/// What the start knows about the entries in place.
struct Seen<'a> {
    /// The user's `aurora-viewer.desktop`, when there is one.
    user: Option<&'a str>,
    /// An `aurora-viewer.desktop` in a system folder (a package's).
    system: bool,
    /// Whether the program an existing managed entry starts still exists.
    recorded_exists: bool,
    /// A release build (not `dev`, `ci` nor `debugging`).
    release: bool,
}

fn plan(seen: &Seen, wanted: &str) -> Plan {
    match seen.user {
        // a package's entry answers for the app_id: nothing to add
        None if seen.system => Plan::Leave,
        None => Plan::Write,
        // installed by hand (or `Hidden=true` to keep the viewer out of
        // the menu): not ours to change
        Some(text) if !managed(text) => Plan::Leave,
        Some(_) if seen.system => Plan::Remove,
        Some(text) if text == wanted => Plan::Keep,
        // another viewer of ours: a release takes the entry over, a
        // development build only from one that is gone
        Some(_) if seen.release || !seen.recorded_exists => Plan::Write,
        Some(_) => Plan::Keep,
    }
}

fn managed(text: &str) -> bool {
    text.lines().any(|l| l.trim() == MANAGED)
}

/// The entry for `exe`: the shipped one, with its `Exec` and the mark.
fn entry_for(exe: &str) -> String {
    let mut out = String::new();
    for line in ENTRY.lines() {
        if line.starts_with("Exec=") {
            out.push_str("Exec=");
            out.push_str(&crate::url_scheme::exec_command(exe));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out.push_str(MANAGED);
    out.push('\n');
    out
}

/// The program an entry starts, its `Exec` unquoted.
fn recorded_program(text: &str) -> Option<String> {
    let exec = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
    crate::url_scheme::exec_program(exec)
}

/// The path the entry must start: an AppImage's own file, not the folder
/// it is mounted on for this run (`$APPIMAGE`, set by its runtime). Only
/// when this viewer really runs from that mount (`$APPDIR`): a shell can
/// keep the variables of another AppImage started from it.
fn viewer_path(appimage: Option<PathBuf>, appdir: Option<PathBuf>, exe: PathBuf) -> PathBuf {
    match (appimage, appdir) {
        (Some(image), Some(dir)) if image.is_absolute() && dir.is_absolute() && exe.starts_with(&dir) && image.is_file() => image,
        _ => exe,
    }
}

/// Inside a sandbox the package brings its entry, and the user's folders
/// are not the desktop's.
fn sandboxed() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("SNAP").is_some()
}

/// At start, before the first window: a desktop that reads the icon when
/// the window maps then finds it. A start with everything in place only
/// reads two small files and looks for a package's entry; the first start,
/// or one after the folder moved, writes them (a few milliseconds). Not in
/// demo mode (the agents' and testers' runs leave the desktop as it is) nor
/// in a sandbox.
///
/// Runs before the log is set up: what it did comes back as lines for
/// [`report`].
pub fn install() -> Result<Vec<String>, String> {
    if sandboxed() || std::env::var_os("AURORA_DEMO").is_some() {
        return Ok(Vec::new());
    }
    install_now()
}

/// Logs what [`install`] did, once the log is up.
pub fn report(done: Result<Vec<String>, String>) {
    match done {
        Ok(lines) => lines.iter().for_each(|line| log::info!("desktop entry: {line}")),
        Err(error) => log::warn!("desktop entry: not installed: {error}"),
    }
}

fn install_now() -> Result<Vec<String>, String> {
    let data = crate::url_scheme::data_home().ok_or("no home folder")?;
    let exe = crate::url_scheme::live_exe(std::env::current_exe().map_err(|e| e.to_string())?);
    let exe = viewer_path(
        std::env::var_os("APPIMAGE").map(PathBuf::from),
        std::env::var_os("APPDIR").map(PathBuf::from),
        exe,
    );
    let id = format!("{}.desktop", crate::cli::DESKTOP_ID);
    let system = crate::url_scheme::application_dirs()
        .into_iter()
        .filter(|dir| !dir.starts_with(&data))
        .any(|dir| dir.join(&id).is_file());
    apply(&data, &exe, system, env!("AURORA_BUILD_PROFILE") == "release")
}

/// The entry and the icon under `data` (`$XDG_DATA_HOME`) for the viewer at
/// `exe`; `system`: a package's entry exists in a system folder.
fn apply(data: &Path, exe: &Path, system: bool, release: bool) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let wanted = entry_for(&exe.to_string_lossy());
    let path = data.join("applications").join(format!("{}.desktop", crate::cli::DESKTOP_ID));
    let user = std::fs::read_to_string(&path).ok();
    let recorded_exists = user
        .as_deref()
        .and_then(recorded_program)
        .is_some_and(|program| Path::new(&program).is_file());
    let seen = Seen {
        user: user.as_deref(),
        system,
        recorded_exists,
        release,
    };
    let plan = plan(&seen, &wanted);
    match plan {
        Plan::Leave => return Ok(done),
        Plan::Remove => {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            done.push(format!("{} removed, a package's entry answers now", path.display()));
            return Ok(done);
        }
        Plan::Write | Plan::Keep => {}
    }
    // the icon first, and while the entry is ours, at every start: an entry
    // is never shown without it, and a deleted or updated one comes back
    let icon = data
        .join("icons/hicolor/scalable/apps")
        .join(format!("{}.svg", crate::cli::DESKTOP_ID));
    if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
        write(&icon, ICON)?;
        done.push(format!("icon {} written", icon.display()));
    }
    if plan == Plan::Write {
        write(&path, wanted.as_bytes())?;
        done.push(format!("{} starts {}", path.display(), exe.display()));
    }
    Ok(done)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::atomic_file::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(user: Option<&str>) -> Seen<'_> {
        Seen {
            user,
            system: false,
            recorded_exists: true,
            release: true,
        }
    }

    #[test]
    fn the_entry_is_the_shipped_one_with_this_viewer_and_the_mark() {
        let text = entry_for("/opt/Aurora Viewer/aurora-viewer");
        assert!(text.contains("\nExec=\"/opt/Aurora Viewer/aurora-viewer\"\n"), "{text}");
        assert!(text.contains("\nIcon=aurora-viewer\n"));
        assert!(text.contains("\nStartupWMClass=aurora-viewer\n"));
        assert!(managed(&text));
        assert_eq!(text.matches("Exec=").count(), 1);
        assert_eq!(recorded_program(&text).as_deref(), Some("/opt/Aurora Viewer/aurora-viewer"));
        // a plain path stays bare, as the menus' launchers read it best
        assert!(entry_for("/opt/aurora/aurora-viewer").contains("\nExec=/opt/aurora/aurora-viewer\n"));
    }

    #[test]
    fn a_missing_entry_is_written_unless_a_package_has_one() {
        let wanted = entry_for("/opt/aurora/aurora-viewer");
        assert_eq!(plan(&seen(None), &wanted), Plan::Write);
        let packaged = Seen {
            system: true,
            ..seen(None)
        };
        assert_eq!(plan(&packaged, &wanted), Plan::Leave);
    }

    #[test]
    fn an_entry_the_viewer_did_not_write_is_left_alone() {
        let wanted = entry_for("/opt/aurora/aurora-viewer");
        let by_hand = ENTRY.replace("Exec=aurora-viewer", "Exec=\"/home/me/Aurora/aurora-viewer\"");
        assert_eq!(plan(&seen(Some(&by_hand)), &wanted), Plan::Leave);
        let gone = Seen {
            recorded_exists: false,
            ..seen(Some(&by_hand))
        };
        assert_eq!(plan(&gone, &wanted), Plan::Leave);
    }

    #[test]
    fn a_managed_entry_follows_a_release_and_a_development_build_takes_only_a_dead_one() {
        let installed = entry_for("/opt/aurora/aurora-viewer");
        let wanted = entry_for("/home/me/git/target/debug/aurora-viewer");
        assert_eq!(
            plan(&seen(Some(&installed)), &installed),
            Plan::Keep,
            "already right: the icon is checked"
        );
        assert_eq!(plan(&seen(Some(&installed)), &wanted), Plan::Write, "a release moved");
        let dev = Seen {
            release: false,
            ..seen(Some(&installed))
        };
        assert_eq!(plan(&dev, &wanted), Plan::Keep, "the installed viewer keeps it");
        let dev_gone = Seen {
            recorded_exists: false,
            ..dev
        };
        assert_eq!(plan(&dev_gone, &wanted), Plan::Write, "its viewer was removed");
    }

    #[test]
    fn an_appimage_records_its_own_file_only_when_running_from_it() {
        let dir = std::env::temp_dir().join(format!("aurora-appimage-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let own = dir.join("Aurora.AppImage");
        std::fs::write(&own, b"").unwrap();
        let mount = PathBuf::from("/tmp/.mount_Aurora");
        let mounted = mount.join("usr/bin/aurora-viewer");
        assert_eq!(viewer_path(Some(own.clone()), Some(mount.clone()), mounted.clone()), own);
        assert_eq!(viewer_path(None, Some(mount.clone()), mounted.clone()), mounted);
        assert_eq!(
            viewer_path(Some(PathBuf::from("relative")), Some(mount.clone()), mounted.clone()),
            mounted
        );
        // a shell that kept another AppImage's variables: this viewer is not in its mount
        let extracted = PathBuf::from("/opt/aurora/aurora-viewer");
        assert_eq!(viewer_path(Some(own.clone()), Some(mount.clone()), extracted.clone()), extracted);
        assert_eq!(viewer_path(Some(own.clone()), None, mounted.clone()), mounted);
        // the AppImage moved or deleted since
        let gone = dir.join("Moved.AppImage");
        assert_eq!(viewer_path(Some(gone), Some(mount), mounted.clone()), mounted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_package_installed_later_takes_over_from_our_entry() {
        let ours = entry_for("/opt/aurora/aurora-viewer");
        let both = Seen {
            system: true,
            ..seen(Some(&ours))
        };
        assert_eq!(plan(&both, &ours), Plan::Remove);
        // a hand-made entry (Hidden=true to stay out of the menu) is kept
        let hidden = "[Desktop Entry]\nType=Application\nName=Aurora Viewer\nHidden=true\n";
        assert_eq!(
            plan(
                &Seen {
                    system: true,
                    ..seen(Some(hidden))
                },
                &ours
            ),
            Plan::Leave
        );
        assert_eq!(plan(&seen(Some(hidden)), &ours), Plan::Leave);
    }

    /// On disk: the first start writes both; a start with the entry right
    /// puts a deleted or different icon back; a package installed later
    /// takes over; an entry of the user's own is never touched.
    #[test]
    fn the_files_are_kept_right_from_start_to_start() {
        let data = std::env::temp_dir().join(format!("aurora-desktop-entry-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let exe = data.join("Aurora Viewer/aurora-viewer");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"").unwrap();
        let entry = data.join("applications/aurora-viewer.desktop");
        let icon = data.join("icons/hicolor/scalable/apps/aurora-viewer.svg");
        // first start
        assert_eq!(apply(&data, &exe, false, true).unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), entry_for(&exe.to_string_lossy()));
        assert_eq!(std::fs::read(&icon).unwrap(), ICON);
        // nothing to do
        assert!(apply(&data, &exe, false, true).unwrap().is_empty());
        // the icon deleted, then an older one: put back, the entry untouched
        std::fs::remove_file(&icon).unwrap();
        assert_eq!(apply(&data, &exe, false, true).unwrap().len(), 1);
        std::fs::write(&icon, b"<svg>old</svg>").unwrap();
        assert_eq!(apply(&data, &exe, false, false).unwrap().len(), 1);
        assert_eq!(std::fs::read(&icon).unwrap(), ICON);
        // a package installed since: ours goes
        apply(&data, &exe, true, true).unwrap();
        assert!(!entry.exists());
        // the user's own entry (hidden on purpose): left as it is
        let own = "[Desktop Entry]\nType=Application\nName=Aurora Viewer\nHidden=true\n";
        std::fs::write(&entry, own).unwrap();
        std::fs::remove_file(&icon).unwrap();
        assert!(apply(&data, &exe, false, true).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), own);
        assert!(!icon.exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn the_icon_is_the_svg_logo() {
        assert!(std::str::from_utf8(ICON).is_ok_and(|s| s.contains("<svg")));
    }
}
