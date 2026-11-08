//! A SLURL opened while Aurora already runs goes to that viewer instead of
//! starting a second one, as LLAppViewer::initConfiguration hands the
//! startup SLURL to the other instance and quits
//! (LLAppViewerWin32::sendURLToOtherInstance, FindWindow + WM_COPYDATA;
//! LLViewerWindow::handleDataCopy; indra/newview/llappviewer.cpp,
//! llappviewerwin32.cpp, llviewerwindow.cpp, originally LGPL 2.1).
//!
//! Aurora uses a named pipe of the user's session instead of a window
//! message (winit owns the window procedure): every viewer creates an
//! endpoint and publishes it through properties of its main window. Actual
//! focus events record which receiver was used last. A launch with a link
//! selects that receiver and stops before opening a window. The pipe's
//! security permits only the same interactive logon to write requests, at
//! medium integrity even when the viewer is elevated; remote clients are
//! refused. Offline demos and explicitly titled senders are isolated. An
//! untitled online launch (the browser's) reaches the last focused online
//! window even if that receiver has a test title. Shared names remain as
//! relays for older clients. No registry or per-frame polling is used.
//!
//! On Linux the hand-over goes over the session bus, as the Legacy Viewer's
//! (LLAppViewerLinux::sendURLToOtherInstance and initSLURLHandler, method
//! `GoSLURL(s)`; indra/newview/llappviewerlinux.cpp): the first viewer owns
//! a well-known name, the bus gives it up when that viewer exits (even on a
//! crash), and only the user's own processes reach the session bus. The
//! name is Aurora's and not the Legacy Viewer's `com.secondlife.ViewerAppAPIService`,
//! which every viewer of that family owns: a link would otherwise go to
//! whichever of them started first. Demo mode and `--title` windows add
//! their own part to the name.

use crossbeam_channel::Receiver;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{focused, hand_off, listen, release, window_ready};

/// Longest link received (a SLURL is far shorter).
#[cfg(any(windows, target_os = "linux"))]
const MAX_URL: usize = 8 * 1024;

#[cfg(target_os = "linux")]
pub use bus::hand_off;

#[cfg(not(any(windows, target_os = "linux")))]
pub fn hand_off(_url: &str) -> bool {
    false
}

#[cfg(target_os = "linux")]
pub use bus::listen;

/// Stops receiving links as the viewer closes, so that a later launch
/// starts a viewer of its own instead of handing its link to this one.
#[cfg(target_os = "linux")]
pub use bus::release;

#[cfg(not(any(windows, target_os = "linux")))]
pub fn release() {}

#[cfg(not(windows))]
pub fn window_ready(_window: &std::sync::Arc<winit::window::Window>) {}

#[cfg(not(windows))]
pub fn focused() {}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn listen() -> Option<Receiver<String>> {
    None
}

#[cfg(target_os = "linux")]
mod bus {
    use super::{MAX_URL, Receiver};
    use crossbeam_channel::Sender;
    use zbus::blocking::Connection;
    use zbus::fdo::{RequestNameFlags, RequestNameReply};

    const PATH: &str = "/org/aurora_viewer/AuroraViewer";
    const INTERFACE: &str = "org.aurora_viewer.AuroraViewer";
    /// Past it, the launch starts a viewer of its own with the link.
    const HAND_OFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    /// The viewer that receives the links: its connection keeps the name
    /// until `release`.
    static OWNER: std::sync::Mutex<Option<(Connection, String)>> = std::sync::Mutex::new(None);

    /// `org.aurora_viewer.AuroraViewer[.Demo][.T_<title>]`.
    fn bus_name() -> String {
        name_for(std::env::var_os("AURORA_DEMO").is_some(), crate::cli::title_slug().as_deref())
    }

    /// A bus name's elements hold only `[A-Za-z0-9_]` and do not start with
    /// a digit, and the whole name stays under 255 bytes.
    pub(super) fn name_for(demo: bool, title_slug: Option<&str>) -> String {
        let mut name = String::from(INTERFACE);
        if demo {
            name.push_str(".Demo");
        }
        if let Some(slug) = title_slug {
            name.push_str(".T_");
            name.extend(slug.chars().take(100).map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }));
        }
        name
    }

    struct Links(Sender<String>);

    #[zbus::interface(name = "org.aurora_viewer.AuroraViewer")]
    impl Links {
        /// A link of a later launch (the Legacy Viewer's method name).
        /// An error when the viewer no longer reads them (closing): the
        /// launch then starts a viewer of its own with the link.
        #[zbus(name = "GoSLURL")]
        fn go_slurl(&self, slurl: String) -> zbus::fdo::Result<()> {
            let url = slurl.trim();
            if !url.is_empty() && url.len() <= MAX_URL {
                self.0
                    .send(url.to_owned())
                    .map_err(|_| zbus::fdo::Error::Failed("the viewer is closing".into()))?;
            }
            Ok(())
        }
    }

    /// Sends the link to the viewer already running; false when there is
    /// none (no owner of the name: this process then starts with it).
    /// Runs before the log is open, so failures go to stderr.
    pub fn hand_off(url: &str) -> bool {
        hand_off_to(&bus_name(), url, HAND_OFF_TIMEOUT)
    }

    /// `timeout`: how long an owner that holds the name may take to answer.
    /// An owner that does not answer (stopped, frozen, shutting down) must
    /// not keep a launch waiting; the tests give a loaded CI runner longer.
    pub(super) fn hand_off_to(name: &str, url: &str, timeout: std::time::Duration) -> bool {
        let bus = match zbus::blocking::connection::Builder::session().and_then(|b| b.method_timeout(timeout).build()) {
            Ok(bus) => bus,
            Err(e) => {
                eprintln!("startup links: no session bus ({e})");
                return false;
            }
        };
        // no service file names it: without an owner the call fails at once
        match bus.call_method(Some(name), PATH, Some(INTERFACE), "GoSLURL", &(url,)) {
            Ok(_) => true,
            Err(zbus::Error::MethodError(error, ..)) if error.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown" => false,
            Err(e) => {
                eprintln!("startup links: not handed over ({e})");
                false
            }
        }
    }

    /// Owns the name when no other viewer of this session (same demo mode
    /// and title) does; the object is served before the name is taken, so
    /// a launch that finds the name finds the method too.
    pub fn listen() -> Option<Receiver<String>> {
        listen_as(bus_name())
    }

    pub(super) fn listen_as(name: String) -> Option<Receiver<String>> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let bus = match Connection::session() {
            Ok(bus) => bus,
            Err(e) => {
                log::warn!("startup links: no session bus ({e})");
                return None;
            }
        };
        if let Err(e) = bus.object_server().at(PATH, Links(tx)) {
            log::warn!("startup links: {e}");
            return None;
        }
        match bus.request_name_with_flags(name.as_str(), RequestNameFlags::DoNotQueue.into()) {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                *OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((bus, name));
                Some(rx)
            }
            Ok(_) => {
                log::info!("startup links: received by another viewer");
                None
            }
            Err(e) => {
                log::warn!("startup links: {e}");
                None
            }
        }
    }

    /// Gives the name back before the viewer's slow shutdown (browsers,
    /// plugins): the bus would only free it once the process exits, and
    /// meanwhile a launch would hand its link to a viewer that drops it.
    pub fn release() {
        let Some((bus, name)) = OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() else {
            return;
        };
        if let Err(e) = bus.release_name(name.as_str()) {
            log::warn!("startup links: name not released ({e})");
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn bus_names_are_valid() {
        use super::bus::name_for;
        assert_eq!(name_for(false, None), "org.aurora_viewer.AuroraViewer");
        assert_eq!(
            name_for(true, Some("test-linux-slurl")),
            "org.aurora_viewer.AuroraViewer.Demo.T_test_linux_slurl"
        );
        // non-ASCII letters of a title, a leading digit, a very long title
        for slug in ["été-2", "9lives", &"a".repeat(400)] {
            let name = name_for(false, Some(slug));
            assert!(zbus::names::WellKnownName::try_from(name.as_str()).is_ok(), "{name}");
        }
    }

    /// A link sent to a viewer that no longer reads them (closing) is
    /// refused, so the launch opens it itself; once the name is released
    /// the launch does not even try. Needs a session bus (skipped without).
    #[test]
    fn closing_viewer_refuses_links() {
        use super::bus::{hand_off_to, listen_as, release};
        if zbus::blocking::Connection::session().is_err() {
            eprintln!("no session bus: skipped");
            return;
        }
        let name = super::bus::name_for(false, Some(&format!("unit-test-{}", std::process::id())));
        let url = "secondlife://Test%20Region/128/128/20";
        // the viewer's 5 s are for a frozen owner; this one answers, only
        // late on a CI runner loaded by the rest of the tests
        let hand_off = |name: &str| hand_off_to(name, url, std::time::Duration::from_secs(60));
        let rx = listen_as(name.clone()).expect("owns a name unique to this process");
        assert!(hand_off(&name));
        assert_eq!(rx.try_recv().as_deref(), Ok(url));
        // the app (and its receiver) dropped, the name still held
        drop(rx);
        assert!(!hand_off(&name));
        release();
        assert!(!hand_off(&name));
        assert!(listen_as(name).is_some(), "the name is free again");
        release();
    }
}
