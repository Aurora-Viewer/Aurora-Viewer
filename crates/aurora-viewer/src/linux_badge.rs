//! The launcher badge of the Linux taskbar: the number of unread items on
//! Aurora's icon, and the KDE flash that goes with it. Opt-in
//! (`Settings::linux_taskbar_badge`, off).
//!
//! The badge is the Unity launcher API, `com.canonical.Unity.LauncherEntry`:
//! a broadcast signal `Update(s app_uri, a{sv} properties)` on the session
//! bus with the properties `count` (i64) and `count-visible` (bool). KDE
//! Plasma's task manager, Dash to Dock, Plank and others listen for it; a
//! desktop with no listener ignores the signal. The path is
//! `/com/canonical/unity/launcherentry/<number>`, the number a hash of the
//! `app_uri` as libunity makes it (listeners match the interface and the
//! `app_uri`, not the path). The `app_uri` is `application://<desktop
//! file>`, here the `aurora-viewer.desktop` of `packaging/linux/`.
//!
//! The Legacy Viewer has no launcher badge on Linux (LLWindowSDL::flashIcon,
//! indra/llwindow/llwindowsdl.cpp, only flashes a minimized window); this is
//! Aurora's own, behind its flag, and nothing of it leaves the machine.
//!
//! Everything that talks to the bus runs on a worker thread, never in the
//! frame loop; every failure (no bus, no listener) is logged once at debug
//! and then ignored. The pure helpers are tested on every system.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// The desktop entry of the viewer (`packaging/linux/aurora-viewer.desktop`).
const APP_URI: &str = "application://aurora-viewer.desktop";
const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const PATH_PREFIX: &str = "/com/canonical/unity/launcherentry/";

/// What the badge counts: the unread conversations (IMs, group chats,
/// conferences, the number of the Conversations button) and the pending
/// items of the notification center (offers, group notices, alerts).
pub fn badge_count(unread_conversations: usize, pending_notices: usize) -> usize {
    unread_conversations.saturating_add(pending_notices)
}

/// Whether the desktop is KDE: `XDG_CURRENT_DESKTOP` is a colon-separated
/// list ("KDE", "ubuntu:GNOME", "KDE:Plasma"), compared without case.
pub fn is_kde(xdg_current_desktop: Option<&str>) -> bool {
    xdg_current_desktop.is_some_and(|list| list.split(':').any(|d| d.trim().eq_ignore_ascii_case("KDE")))
}

/// The unfocused flash is asked on this desktop: the flag on and KDE.
pub fn kde_flash_enabled(flag: bool, xdg_current_desktop: Option<&str>) -> bool {
    cfg!(target_os = "linux") && flag && is_kde(xdg_current_desktop)
}

/// libunity's number for an application: the djb2 hash of its `app_uri`.
fn path_number(app_uri: &str) -> u32 {
    app_uri
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_shl(5).wrapping_add(h).wrapping_add(u32::from(b)))
}

/// The object path the signal is emitted from.
fn object_path(app_uri: &str) -> String {
    format!("{PATH_PREFIX}{}", path_number(app_uri))
}

/// The `count` and `count-visible` properties for a number.
fn properties(count: usize) -> (i64, bool) {
    (i64::try_from(count).unwrap_or(i64::MAX), count > 0)
}

/// The worker's mail.
#[cfg(target_os = "linux")]
enum Mail {
    Count(usize),
    /// Clear the badge, tell `done` and stop.
    Stop(std::sync::mpsc::Sender<()>),
}

/// The badge: the number last asked, and the worker that sends it.
#[derive(Default)]
pub struct Badge {
    /// What was last sent (0 at start: nothing to clear yet).
    sent: usize,
    #[cfg(target_os = "linux")]
    worker: Option<std::sync::mpsc::Sender<Mail>>,
    /// The "worker gone" line was written.
    #[cfg(target_os = "linux")]
    gone_logged: bool,
}

impl Badge {
    /// Show `count` on the launcher (0 clears it); sent only when it
    /// changes. Never blocks.
    pub fn set(&mut self, count: usize) {
        if count == self.sent {
            return;
        }
        self.sent = count;
        #[cfg(target_os = "linux")]
        if self.worker.get_or_insert_with(start_worker).send(Mail::Count(count)).is_err() && !std::mem::replace(&mut self.gone_logged, true)
        {
            // logged once, like the other errors of the badge
            log::debug!("launcher badge: worker gone");
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Badge {
    /// Clears the badge before the viewer goes, waiting at most half a
    /// second for the bus.
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        if self.sent > 0 && worker.send(Mail::Stop(tx)).is_ok() {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
        }
    }
}

#[cfg(target_os = "linux")]
fn start_worker() -> std::sync::mpsc::Sender<Mail> {
    let (tx, rx) = std::sync::mpsc::channel();
    if let Err(e) = std::thread::Builder::new().name("aurora-badge".into()).spawn(move || run(rx)) {
        log::debug!("launcher badge: no worker thread ({e})");
    }
    tx
}

#[cfg(target_os = "linux")]
fn run(rx: std::sync::mpsc::Receiver<Mail>) {
    let mut bus: Option<zbus::blocking::Connection> = None;
    let mut failed = false;
    for mail in rx {
        let (count, done) = match mail {
            Mail::Count(n) => (n, None),
            Mail::Stop(done) => (0, Some(done)),
        };
        if !failed {
            if bus.is_none() {
                match zbus::blocking::Connection::session() {
                    Ok(b) => bus = Some(b),
                    Err(e) => {
                        log::debug!("launcher badge: no session bus ({e})");
                        failed = true;
                    }
                }
            }
            if let Some(b) = &bus
                && let Err(e) = emit(b, count)
            {
                log::debug!("launcher badge: not sent ({e})");
                failed = true;
            }
        }
        if let Some(done) = done {
            let _ = done.send(());
            return;
        }
    }
}

#[cfg(target_os = "linux")]
fn emit(bus: &zbus::blocking::Connection, count: usize) -> zbus::Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;
    let (n, visible) = properties(count);
    let props: HashMap<&str, Value<'_>> = HashMap::from([("count", Value::from(n)), ("count-visible", Value::from(visible))]);
    bus.emit_signal(None::<&str>, object_path(APP_URI).as_str(), INTERFACE, "Update", &(APP_URI, props))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_adds_conversations_and_notices() {
        assert_eq!(badge_count(0, 0), 0);
        assert_eq!(badge_count(3, 2), 5);
        assert_eq!(badge_count(usize::MAX, 1), usize::MAX);
    }

    #[test]
    fn kde_is_found_in_the_colon_separated_list() {
        assert!(is_kde(Some("KDE")));
        assert!(is_kde(Some("kde")));
        assert!(is_kde(Some("ubuntu:KDE")));
        assert!(is_kde(Some("KDE:Plasma")));
        assert!(!is_kde(Some("GNOME")));
        assert!(!is_kde(Some("ubuntu:GNOME")));
        assert!(!is_kde(Some("KDEish")));
        assert!(!is_kde(Some("")));
        assert!(!is_kde(None));
    }

    #[test]
    fn the_kde_flash_needs_the_flag() {
        assert!(!kde_flash_enabled(false, Some("KDE")));
        assert!(!kde_flash_enabled(true, Some("GNOME")));
        assert_eq!(kde_flash_enabled(true, Some("KDE")), cfg!(target_os = "linux"));
    }

    #[test]
    fn the_properties_hide_a_zero_count() {
        assert_eq!(properties(0), (0, false));
        assert_eq!(properties(7), (7, true));
        assert_eq!(properties(usize::MAX), (i64::MAX, true));
    }

    #[test]
    fn the_path_is_a_valid_object_path_of_the_launcher_entry() {
        let path = object_path(APP_URI);
        assert!(path.starts_with(PATH_PREFIX));
        assert!(path[PATH_PREFIX.len()..].bytes().all(|b| b.is_ascii_digit()));
        // the same number every time, libunity's djb2 ("a" is 177670)
        assert_eq!(path, object_path(APP_URI));
        assert_eq!(path_number("a"), 5381 * 33 + 97);
    }
}
