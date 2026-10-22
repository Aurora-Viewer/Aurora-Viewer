//! A SLURL that comes from outside the viewer: the startup argument, or a
//! link a web page opened in another process that handed it over
//! (`slurl_handoff.rs`). Read as LLURLDispatcher reads it with
//! NAV_TYPE_EXTERNAL and no trusted browser (LLViewerWindow::handleDataCopy,
//! LLURLDispatcherImpl::dispatchCore, LLCommandHandlerRegistry::dispatch;
//! indra/newview/llurldispatcher.cpp, llcommandhandler.cpp, originally
//! LGPL 2.1):
//!
//! - a place (LLSLURL::LOCATION) is where the login screen starts, or once
//!   in the world its details (dispatchRegion, SLURLTeleportDirectly off);
//! - an app command runs when its handler allows untrusted links
//!   (UNTRUSTED_THROTTLE: at most one every 5 s), and is refused when it
//!   only allows clicks or nothing from outside (UNTRUSTED_CLICK_ONLY,
//!   UNTRUSTED_BLOCK).
//!
//! Deliberate difference: `secondlife:///app/teleport/…` (a click-only
//! command, refused from outside by the Legacy Viewer) opens the place like
//! a location link, whose window has the Teleport button: the link of a web
//! page still never teleports without a click in the viewer.

use crate::slapp::AppLink;
use crate::slurl::Location;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Place(Location),
    /// `agent/<id>/about` or `inspect`: the resident's profile
    /// (LLAgentHandler, the verbs `slapp.rs` leaves to the name labels).
    Profile(uuid::Uuid),
    App(AppLink),
    /// A command the Legacy Viewer refuses from outside.
    Blocked,
    /// Not a SLURL: a web address or any other text is never opened.
    Ignored,
}

/// What an outside link asks for.
pub fn classify(url: &str) -> Request {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    // `secondlife://app/…`: LLSLURL reads the host "app" as an app command
    let app = if let Some(rest) = lower.strip_prefix("secondlife://app/") {
        Some(format!("secondlife:///app/{}", &url[url.len() - rest.len()..]))
    } else if lower.starts_with("secondlife:///app/") {
        Some(url.to_owned())
    } else {
        None
    };
    if let Some(app) = app {
        let command = app["secondlife:///app/".len()..].split(['/', '?', '#']).next().unwrap_or("");
        if command.eq_ignore_ascii_case("teleport") || command.eq_ignore_ascii_case("region") {
            return crate::slurl::parse(&app).map_or(Request::Ignored, Request::Place);
        }
        if command.eq_ignore_ascii_case("agent") {
            let mut tokens = app["secondlife:///app/agent/".len()..]
                .split(['?', '#'])
                .next()
                .unwrap_or("")
                .split('/');
            if let (Some(Ok(id)), Some("about" | "inspect")) = (tokens.next().map(uuid::Uuid::parse_str), tokens.next()) {
                return Request::Profile(id);
            }
        }
        return match crate::slapp::parse(&app) {
            // UNTRUSTED_BLOCK
            Some(AppLink::Blocked(_)) => Request::Blocked,
            // UNTRUSTED_CLICK_ONLY: LLSearchHandler, LLChatCommandHandler
            // (llfloaterimnearbychat.cpp; the other registration of "chat"
            // allows a throttled one, the stricter is kept)
            Some(AppLink::Search { .. } | AppLink::Chat { .. }) => Request::Blocked,
            Some(link) => Request::App(link),
            None => Request::App(AppLink::Unsupported),
        };
    }
    let place = [
        "secondlife://",
        "hop://",
        "x-grid-location-info://",
        "http://maps.secondlife.com/secondlife/",
        "https://maps.secondlife.com/secondlife/",
        "http://slurl.com/secondlife/",
        "https://slurl.com/secondlife/",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix));
    if !place || (lower.starts_with("secondlife:///") && !lower.starts_with("secondlife:///secondlife/")) {
        return Request::Ignored;
    }
    crate::slurl::parse(url).map_or(Request::Ignored, Request::Place)
}

/// LLCommandHandler's THROTTLE_PERIOD: one outside command every 5 s.
const THROTTLE: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    /// Whether a command from outside may run now (the refused ones do not
    /// restart the wait, as in LLCommandHandlerRegistry::dispatch).
    pub fn allow(&mut self, now: Instant) -> bool {
        if self.last.is_some_and(|last| now < last + THROTTLE) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// "Region (x, y, z)", as the login screen shows a start SLURL.
pub fn place_label(location: &Location) -> String {
    format!(
        "{} ({}, {}, {})",
        location.region,
        location.pos.x.round() as i32,
        location.pos.y.round() as i32,
        location.pos.z.round() as i32
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn place(url: &str) -> (String, [i32; 3]) {
        match classify(url) {
            Request::Place(l) => (l.region, [l.pos.x as i32, l.pos.y as i32, l.pos.z as i32]),
            other => panic!("{url}: {other:?}"),
        }
    }

    #[test]
    fn hypergrid_external_link_opens_details_without_teleporting() {
        for scheme in ["hop", "x-grid-location-info"] {
            let Request::Place(location) = classify(&format!("{scheme}://grid.example:8002/Region/12/34/56")) else {
                panic!("place");
            };
            assert_eq!(location.grid.as_deref(), Some("grid.example:8002"));
            assert_eq!(location.pos, Vec3::new(12.0, 34.0, 56.0));
        }
    }

    #[test]
    fn places() {
        let want = ("Aurora Démo".to_owned(), [12, 34, 56]);
        assert_eq!(place("secondlife://Aurora%20D%C3%A9mo/12/34/56"), want);
        assert_eq!(place("secondlife://Aurora%20D%C3%A9mo/12/34/56/"), want);
        assert_eq!(place(" http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56 "), want);
        assert_eq!(place("https://slurl.com/secondlife/Aurora%20D%C3%A9mo/12/34/56?title=x"), want);
        assert_eq!(place("secondlife:///secondlife/Aurora%20D%C3%A9mo/12/34/56"), want);
        assert_eq!(place("secondlife:///app/teleport/Aurora%20D%C3%A9mo/12/34/56"), want);
        assert_eq!(place("secondlife://app/teleport/Aurora%20D%C3%A9mo/12/34/56"), want);
        assert_eq!(place("secondlife:///app/region/Aurora%20D%C3%A9mo/12/34/56"), want);
        // a region alone: its centre
        assert_eq!(place("secondlife://Ahern"), ("Ahern".to_owned(), [128, 128, 0]));
    }

    #[test]
    fn app_commands_from_outside() {
        let id = uuid::Uuid::parse_str("0e346d8b-4433-4d66-a6b0-fd37083abc4c").expect("id");
        assert_eq!(classify(&format!("secondlife:///app/agent/{id}/about")), Request::Profile(id));
        assert_eq!(classify(&format!("secondlife://app/agent/{id}/inspect/")), Request::Profile(id));
        assert!(matches!(
            classify(&format!("secondlife:///app/agent/{id}/im")),
            Request::App(AppLink::Agent { .. })
        ));
        assert!(matches!(
            classify("secondlife://app/group/0e346d8b-4433-4d66-a6b0-fd37083abc4c/about"),
            Request::App(AppLink::Group(_))
        ));
        assert!(matches!(
            classify("secondlife:///app/worldmap/Ahern/1/2/3"),
            Request::App(AppLink::WorldMap { .. })
        ));
        assert_eq!(classify("secondlife:///app/chat/5/hello"), Request::Blocked);
        assert_eq!(classify("secondlife:///app/search/all/shoes"), Request::Blocked);
        assert_eq!(
            classify("secondlife:///app/wear_folder/?folder_id=0e346d8b-4433-4d66-a6b0-fd37083abc4c"),
            Request::Blocked
        );
        assert_eq!(classify("secondlife:///app/nothing/here"), Request::App(AppLink::Unsupported));
    }

    #[test]
    fn other_text_is_ignored() {
        for url in [
            "https://example.com/secondlife/Ahern/1/2/3",
            "Ahern/1/2/3",
            "C:\\some\\file.txt",
            "secondlife:///unknown/thing",
            "",
        ] {
            assert_eq!(classify(url), Request::Ignored, "{url}");
        }
    }

    #[test]
    fn throttle_of_five_seconds() {
        let mut t = Throttle::default();
        let now = Instant::now();
        assert!(t.allow(now));
        assert!(!t.allow(now + Duration::from_secs(4)));
        assert!(t.allow(now + Duration::from_secs(5)));
    }

    #[test]
    fn label() {
        let l = Location {
            grid: None,
            region: "Ahern".into(),
            pos: Vec3::new(12.4, 34.6, 56.0),
        };
        assert_eq!(place_label(&l), "Ahern (12, 35, 56)");
    }
}
