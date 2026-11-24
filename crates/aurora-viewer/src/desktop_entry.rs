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
//! by hand (docs/LINUX.md before this) or a distribution's package's, in a
//! system folder. An entry it wrote carries `X-Aurora-Managed=true`, and it
//! follows the viewer when the viewer moves; a development build never takes
//! it from an installed viewer that is still there. Inside Flatpak or Snap
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
    /// Write (or rewrite) the entry.
    Write,
    /// Leave it: someone else's, a package's, or already right.
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
        // installed by hand, or by another tool: not ours to change
        Some(text) if !managed(text) => Plan::Leave,
        Some(text) if text == wanted => Plan::Leave,
        // another viewer of ours: a release takes the entry over, a
        // development build only from one that is gone
        Some(_) if seen.release || !seen.recorded_exists => Plan::Write,
        Some(_) => Plan::Leave,
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
/// it is mounted on for this run (`$APPIMAGE`, set by its runtime).
fn viewer_path(appimage: Option<PathBuf>, exe: PathBuf) -> PathBuf {
    appimage.filter(|p| p.is_absolute()).unwrap_or(exe)
}

/// Inside a sandbox the package brings its entry, and the user's folders
/// are not the desktop's.
fn sandboxed() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("SNAP").is_some()
}

/// At start, on a thread of its own: a few small files, never the window's
/// wait. Not in demo mode (the agents' and testers' runs leave the desktop
/// as it is) nor in a sandbox.
pub fn install() {
    if sandboxed() || std::env::var_os("AURORA_DEMO").is_some() {
        return;
    }
    let spawned = std::thread::Builder::new().name("aurora-desktop-entry".into()).spawn(|| {
        if let Err(error) = install_now() {
            log::warn!("desktop entry: not installed: {error}");
        }
    });
    if let Err(error) = spawned {
        log::warn!("desktop entry: not installed: {error}");
    }
}

fn install_now() -> Result<(), String> {
    let data = crate::url_scheme::data_home().ok_or("no home folder")?;
    let exe = crate::url_scheme::live_exe(std::env::current_exe().map_err(|e| e.to_string())?);
    let exe = viewer_path(std::env::var_os("APPIMAGE").map(PathBuf::from), exe);
    let wanted = entry_for(&exe.to_string_lossy());
    let id = format!("{}.desktop", crate::cli::DESKTOP_ID);
    let path = data.join("applications").join(&id);
    let user = std::fs::read_to_string(&path).ok();
    let system = crate::url_scheme::application_dirs()
        .into_iter()
        .filter(|dir| !dir.starts_with(&data))
        .any(|dir| dir.join(&id).is_file());
    let recorded_exists = user
        .as_deref()
        .and_then(recorded_program)
        .is_some_and(|program| Path::new(&program).is_file());
    let seen = Seen {
        user: user.as_deref(),
        system,
        recorded_exists,
        release: env!("AURORA_BUILD_PROFILE") == "release",
    };
    if plan(&seen, &wanted) == Plan::Leave {
        return Ok(());
    }
    // the icon first: an entry is never shown without it
    let icon = data
        .join("icons/hicolor/scalable/apps")
        .join(format!("{}.svg", crate::cli::DESKTOP_ID));
    if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
        write(&icon, ICON)?;
    }
    write(&path, wanted.as_bytes())?;
    log::info!("desktop entry: {} starts {}", path.display(), exe.display());
    Ok(())
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
        assert_eq!(plan(&seen(Some(&installed)), &installed), Plan::Leave, "already right");
        assert_eq!(plan(&seen(Some(&installed)), &wanted), Plan::Write, "a release moved");
        let dev = Seen {
            release: false,
            ..seen(Some(&installed))
        };
        assert_eq!(plan(&dev, &wanted), Plan::Leave, "the installed viewer keeps it");
        let dev_gone = Seen {
            recorded_exists: false,
            ..dev
        };
        assert_eq!(plan(&dev_gone, &wanted), Plan::Write, "its viewer was removed");
    }

    #[test]
    fn an_appimage_records_its_own_file() {
        let mounted = PathBuf::from("/tmp/.mount_Aurora/usr/bin/aurora-viewer");
        let own = PathBuf::from("/home/me/Apps/Aurora.AppImage");
        assert_eq!(viewer_path(Some(own.clone()), mounted.clone()), own);
        assert_eq!(viewer_path(None, mounted.clone()), mounted);
        assert_eq!(viewer_path(Some(PathBuf::from("relative")), mounted.clone()), mounted);
    }

    #[test]
    fn the_icon_is_the_svg_logo() {
        assert!(std::str::from_utf8(ICON).is_ok_and(|s| s.contains("<svg")));
    }
}
