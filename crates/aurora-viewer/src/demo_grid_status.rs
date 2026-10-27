//! Demo of Help › Grid status (#1123), without a grid or a network:
//! `AURORA_DEMO_GRID_STATUS=<mode>` puts the Grid status button at the end
//! of the bottom toolbar (for this run only) and serves a feed from memory
//! (`grid_status::DEMO_FEED`): an incident in progress, a maintenance
//! scheduled and two incidents resolved.
//!
//! - `1` or `open`: the window opens at frame 600.
//! - `new`: at frame 700 a new incident comes in the feed: the button
//!   flashes eight times, then stays tinted until the window is opened.
//! - `none`: the grid publishes no feed (an OpenSim grid without
//!   GridStatusRSS); the window opens at frame 600.
//! - `error`: the feed cannot be read; the window opens at frame 600.

use chrono::{Duration, FixedOffset, Utc};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The frame the window opens (`open`, `none`, `error`).
pub const OPEN_FRAME: u64 = 600;
/// The frame the new incident comes (`new`).
pub const NEWS_FRAME: u64 = 700;

/// Whether the new incident came.
static NEWS: AtomicBool = AtomicBool::new(false);

pub fn mode() -> Option<&'static str> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| {
        let mode = std::env::var("AURORA_DEMO_GRID_STATUS").ok()?;
        let mode = mode.trim();
        match mode {
            "" | "0" => None,
            "1" => Some("open".into()),
            _ => Some(mode.to_owned()),
        }
    })
    .as_deref()
}

/// The window opens by itself in this mode.
pub fn opens() -> bool {
    matches!(mode(), Some("open" | "none" | "error"))
}

/// The grid has a feed (all modes but `none`).
pub fn has_feed() -> bool {
    mode().is_some_and(|m| m != "none")
}

/// The new incident is in the feed from now on (`new`, at NEWS_FRAME).
pub fn announce() {
    NEWS.store(true, Ordering::Relaxed);
}

/// One entry of the feed: (id, title, minutes ago it was updated, its
/// updates as (minutes ago, status, text), newest first).
type Demo = (&'static str, &'static str, i64, &'static [(i64, &'static str, &'static str)]);

const NEW: Demo = (
    "demo-5",
    "Group chat messages delayed",
    0,
    &[(
        0,
        "Investigating",
        "Messages in some group chats arrive several minutes late. We are looking into it.",
    )],
);

const ENTRIES: &[Demo] = &[
    (
        "demo-4",
        "Some residents cannot teleport between regions",
        25,
        &[
            (
                25,
                "Identified",
                "The cause is found; a fix is rolling out to the affected simulators.",
            ),
            (
                70,
                "Investigating",
                "Teleports fail for some residents with \"Teleport failed\". Logging in again may help.",
            ),
        ],
    ),
    (
        "demo-3",
        "Scheduled maintenance: rolling region restarts",
        180,
        &[
            (-1, "", "THIS IS A SCHEDULED EVENT: Thursday, 07:00 - 10:00 PDT"),
            (
                180,
                "Scheduled",
                "Regions restart in turn for a server update. Each restart takes a few minutes; please avoid rezzing \
                 no-copy objects during the window.",
            ),
        ],
    ),
    (
        "demo-2",
        "Delays in inventory deliveries",
        1500,
        &[
            (1500, "Resolved", "Deliveries are back to normal."),
            (1560, "Monitoring", "A fix is in place and we are watching the results."),
            (1680, "Investigating", "Some Marketplace deliveries arrive late."),
        ],
    ),
    (
        "demo-1",
        "Login difficulties",
        4400,
        &[
            (4400, "Resolved", "Logins work again for everyone."),
            (4460, "Investigating", "Some residents cannot log in."),
        ],
    ),
];

/// The feed as the status page writes it (escaped HTML, `<small>` times
/// in PDT), dated from now. `error` gives a page that is not a feed.
pub fn document() -> String {
    if mode() == Some("error") {
        return "<html><body>503 Service Unavailable</body></html>".into();
    }
    // dated from the first reading: the entries must not look updated at
    // each poll
    static START: OnceLock<chrono::DateTime<Utc>> = OnceLock::new();
    let now = *START.get_or_init(Utc::now);
    let pacific = FixedOffset::west_opt(7 * 3600).expect("a valid offset");
    let news = NEWS.load(Ordering::Relaxed).then_some(&NEW);
    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom">"#);
    for (id, title, ago, updates) in news.into_iter().chain(ENTRIES) {
        // the new incident is dated when it comes
        static CAME: OnceLock<chrono::DateTime<Utc>> = OnceLock::new();
        let now = if *id == NEW.0 { *CAME.get_or_init(Utc::now) } else { now };
        let mut html = String::new();
        for (minutes, status, text) in *updates {
            if *minutes < 0 {
                html.push_str(&format!("<p><strong>{text}</strong></p>"));
                continue;
            }
            let when = (now - Duration::minutes(*minutes))
                .with_timezone(&pacific)
                .format("%b %d, %H:%M PDT");
            html.push_str(&format!("<p><small>{when}</small><br><strong>{status}</strong> - {text}</p>"));
        }
        let mut content = String::new();
        aurora_llsd::xml::escape(&html, &mut content);
        let updated = (now - Duration::minutes(*ago)).to_rfc3339();
        xml.push_str(&format!(
            "<entry><id>{id}</id><updated>{updated}</updated>\
             <link rel=\"alternate\" type=\"text/html\" href=\"https://status.secondlifegrid.net/\"/>\
             <title>{title}</title><content type=\"html\">{content}</content></entry>"
        ));
    }
    xml.push_str("</feed>");
    xml
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_demo_feed_reads_like_the_status_page() {
        let entries = crate::grid_status::parse(super::document().as_bytes()).expect("demo feed");
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].title, "Some residents cannot teleport between regions");
        assert_eq!(entries[0].phase(), crate::grid_status::Phase::Ongoing);
        assert_eq!(entries[1].phase(), crate::grid_status::Phase::Maintenance);
        assert_eq!(entries[1].updates[0].status, "");
        assert_eq!(entries[2].updates.len(), 3);
    }
}
