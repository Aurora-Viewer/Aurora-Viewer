//! Help › Grid status: the grid's status feed, read off the frame.
//!
//! Port of LLFloaterGridStatus::startGridStatusTimer / getGridStatusRSSCoro
//! (indra/newview/llfloatergridstatus.cpp, originally LGPL 2.1) and of the
//! feed's choice in LFSimFeatureHandler::setSupportedFeatures
//! (lfsimfeaturehandler.cpp): Second Life's Atom feed (GridStatusRSS), an
//! OpenSim grid's own through its SimulatorFeatures' OpenSimExtras, none
//! otherwise. The feed is fetched every GridStatusUpdateDelay (60 s) while
//! the Grid status command stands on a toolbar (LLToolBarView::hasCommand);
//! an entry that is new, or whose `updated` changed, after the first
//! answer flashes that command's button (sItemsMap, flashCommand).
//!
//! The Legacy Viewer's window is the status page in a web browser; Aurora
//! lists the feed's entries itself (`ui/grid_status.rs`), so it also polls
//! while that window is open, and the page stays one click away.

use crate::ui::chat::ButtonFlash;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// GridStatusRSS's default (settings.xml).
pub const SECOND_LIFE_FEED: &str = "https://status.secondlifegrid.net/history.atom";
/// DEFAULT_GRID_STATUS_URL (llfloatergridstatus.h), over https.
pub const SECOND_LIFE_PAGE: &str = "https://status.secondlifegrid.net/";
/// GridStatusUpdateDelay (settings.xml).
pub const UPDATE_DELAY: Duration = Duration::from_secs(60);
/// The `id` of the demo's feed (`AURORA_DEMO_GRID_STATUS`): read from
/// memory, never fetched.
pub const DEMO_FEED: &str = "demo:grid-status";

/// The status page keeps about 25 entries; anything far larger is not a
/// status feed.
const MAX_FEED: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 50;
const MAX_UPDATES: usize = 30;
const MAX_TITLE: usize = 200;
const MAX_TEXT: usize = 4000;
const MAX_URL: usize = 512;

/// One incident or maintenance of the feed (an Atom `entry`, an RSS `item`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// `id` (Atom) or `guid` (RSS), else the link or the title.
    pub id: String,
    pub title: String,
    /// `updated` as given, compared as text like sItemsMap does.
    pub updated_raw: String,
    /// `updated` (or `published`, `pubDate`) in seconds since the epoch, 0
    /// when unreadable.
    pub updated: i64,
    /// Its page on the status site (http or https only).
    pub link: Option<String>,
    /// Its updates, newest first as the status page writes them.
    pub updates: Vec<Update>,
}

/// One paragraph of an entry: on the status page, "<small>when</small><br>
/// <strong>Status</strong> - text".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Update {
    pub when: String,
    /// "Investigating", "Resolved", "Scheduled"…; empty for a plain note.
    pub status: String,
    pub text: String,
}

/// Where an entry stands, from its newest update's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Investigating, Identified, Monitoring, Update.
    Ongoing,
    /// Scheduled, In progress, Verifying: a maintenance.
    Maintenance,
    /// Resolved, Completed, Postmortem.
    Over,
    /// A status the page does not use, or none.
    Unknown,
}

impl Entry {
    pub fn phase(&self) -> Phase {
        let Some(status) = self.updates.iter().find(|u| !u.status.is_empty()) else {
            return Phase::Unknown;
        };
        match status.status.to_lowercase().as_str() {
            "investigating" | "identified" | "monitoring" | "update" => Phase::Ongoing,
            "scheduled" | "in progress" | "verifying" => Phase::Maintenance,
            "resolved" | "completed" | "postmortem" => Phase::Over,
            _ => Phase::Unknown,
        }
    }
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_owned(),
    }
}

fn web_link(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    ((lower.starts_with("https://") || lower.starts_with("http://")) && url.len() <= MAX_URL).then(|| url.to_owned())
}

fn seconds(date: &str) -> i64 {
    let date = date.trim();
    chrono::DateTime::parse_from_rfc3339(date)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(date))
        .map_or(0, |d| d.timestamp())
}

/// The entries of an Atom or RSS 2.0 document, most recently updated first.
pub fn parse(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    use aurora_llsd::xml::Element;
    let root = aurora_llsd::xml::parse(bytes).map_err(|e| e.to_string())?;
    let text = |e: &Element, name: &str| e.child(name).map(|c| c.text().trim().to_owned()).unwrap_or_default();
    let mut entries: Vec<Entry> = match root.name.as_str() {
        // Atom: what getGridStatusRSSCoro reads (entry, id, updated)
        "feed" => root
            .elements()
            .filter(|e| e.name == "entry")
            .map(|e| {
                let updated_raw = text(e, "updated");
                let date = if updated_raw.is_empty() {
                    text(e, "published")
                } else {
                    updated_raw.clone()
                };
                // the page of the entry: rel="alternate", the default rel
                let link = e
                    .elements()
                    .filter(|l| l.name == "link" && l.attr("rel").is_none_or(|r| r == "alternate"))
                    .find_map(|l| l.attr("href").and_then(web_link));
                let content = e.child("content").or_else(|| e.child("summary"));
                let updates = content.map(|c| updates(&c.text())).unwrap_or_default();
                Entry {
                    id: text(e, "id"),
                    title: text(e, "title"),
                    updated_raw,
                    updated: seconds(&date),
                    link,
                    updates,
                }
            })
            .collect(),
        // RSS 2.0, which an OpenSim grid may publish instead
        "rss" => root
            .child("channel")
            .into_iter()
            .flat_map(|c| c.elements().filter(|e| e.name == "item"))
            .map(|e| {
                let updated_raw = text(e, "pubDate");
                Entry {
                    id: text(e, "guid"),
                    title: text(e, "title"),
                    updated: seconds(&updated_raw),
                    updated_raw,
                    link: web_link(&text(e, "link")),
                    updates: updates(&text(e, "description")),
                }
            })
            .collect(),
        other => return Err(format!("<{other}> is neither an Atom feed nor an RSS one")),
    };
    for e in &mut entries {
        if e.id.is_empty() {
            e.id = e.link.clone().unwrap_or_else(|| e.title.clone());
        }
        e.title = clip(&e.title, MAX_TITLE);
        e.updates.truncate(MAX_UPDATES);
    }
    entries.retain(|e| !e.id.is_empty());
    // the newest change first; the feed's order between equals
    entries.sort_by_key(|e| std::cmp::Reverse(e.updated));
    entries.truncate(MAX_ENTRIES);
    Ok(entries)
}

/// One piece of an HTML paragraph.
struct Piece {
    text: String,
    small: bool,
    strong: bool,
}

/// The updates of an entry's HTML content: one per paragraph, its
/// `<small>` the time, a leading `<strong>` followed by " - " the status.
/// Anything else is kept as plain text.
fn updates(html: &str) -> Vec<Update> {
    let mut out = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    let (mut small, mut strong) = (0u32, 0u32);
    let mut flush = |pieces: &mut Vec<Piece>| {
        if let Some(u) = update(std::mem::take(pieces)) {
            out.push(u);
        }
    };
    let mut rest = html;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            push_text(&mut pieces, rest, small > 0, strong > 0);
            break;
        };
        push_text(&mut pieces, &rest[..open], small > 0, strong > 0);
        let Some(close) = rest[open..].find('>') else { break };
        let tag = rest[open + 1..open + close].trim();
        rest = &rest[open + close + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let count = |n: &mut u32| if closing { *n = n.saturating_sub(1) } else { *n += 1 };
        match name.as_str() {
            "small" => count(&mut small),
            "strong" | "b" => count(&mut strong),
            "br" => pieces.push(Piece {
                text: "\n".into(),
                small: false,
                strong: false,
            }),
            "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" => {
                flush(&mut pieces);
                (small, strong) = (0, 0);
            }
            _ => {}
        }
    }
    flush(&mut pieces);
    out
}

fn push_text(pieces: &mut Vec<Piece>, raw: &str, small: bool, strong: bool) {
    if raw.is_empty() {
        return;
    }
    // HTML collapses its white space; the line breaks are the <br>s
    let decoded = decode_entities(raw);
    let mut text = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if decoded.starts_with(char::is_whitespace) {
        text.insert(0, ' ');
    }
    if decoded.ends_with(char::is_whitespace) && !text.ends_with(' ') {
        text.push(' ');
    }
    if !text.is_empty() {
        pieces.push(Piece { text, small, strong });
    }
}

fn joined(pieces: &[&Piece]) -> String {
    pieces.iter().map(|p| p.text.as_str()).collect()
}

fn update(pieces: Vec<Piece>) -> Option<Update> {
    let when = tidy(&pieces.iter().filter(|p| p.small).map(|p| p.text.as_str()).collect::<String>());
    let body: Vec<&Piece> = pieces
        .iter()
        .filter(|p| !p.small)
        .skip_while(|p| p.text.trim().is_empty())
        .collect();
    // "<strong>Resolved</strong> - text": a status only when the dash follows
    let lead = body.iter().take_while(|p| p.strong).count();
    let rest = joined(&body[lead..]);
    let (status, text) = match rest.trim_start().strip_prefix('-') {
        Some(after) if lead > 0 => (tidy(&joined(&body[..lead])), after.to_owned()),
        _ => (String::new(), joined(&body)),
    };
    let text = clip(&tidy(&text), MAX_TEXT);
    (!when.is_empty() || !text.is_empty()).then_some(Update { when, status, text })
}

/// Trimmed lines, without the spaces the tags left around the breaks.
fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').map(str::trim).collect();
    let mut out = lines.join("\n");
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_owned()
}

/// The HTML entities the status page writes (the XML ones are already
/// read by the XML parser, which leaves the escaped HTML's own).
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "rdquo" => Some('”'),
            "ldquo" => Some('“'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// GET the feed, at most MAX_FEED bytes.
async fn fetch(url: &str) -> Result<Vec<Entry>, String> {
    let http = reqwest::Client::builder()
        .user_agent(aurora_version::user_agent())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let response = http.get(url).send().await.map_err(|e| e.to_string())?;
    let mut response = response.error_for_status().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_FEED {
            return Err(format!("feed larger than {MAX_FEED} bytes"));
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes)
}

/// What the window says while it has no entries to show.
#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    /// The grid publishes no status feed.
    NoFeed,
    Loading,
    Loaded,
    Failed(String),
}

/// The feed of the grid, kept up to date while it is followed.
pub struct GridStatus {
    pub entries: Vec<Entry>,
    pub load: Load,
    /// The feed's entries that came or changed since the window was last
    /// shown, after the first answer.
    pub fresh: HashSet<String>,
    /// The Grid status button's flash (flashCommand "gridstatus").
    pub flash: ButtonFlash,
    /// The feed followed and its status page.
    feed: Option<String>,
    page: Option<String>,
    /// sItemsMap: `updated` per entry id, and mIsFirstUpdate.
    seen: HashMap<String, String>,
    first: bool,
    next: Option<Instant>,
    pending: bool,
    tx: crossbeam_channel::Sender<(String, Result<Vec<Entry>, String>)>,
    rx: crossbeam_channel::Receiver<(String, Result<Vec<Entry>, String>)>,
}

impl Default for GridStatus {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            entries: Vec::new(),
            load: Load::NoFeed,
            fresh: HashSet::new(),
            flash: ButtonFlash::default(),
            feed: None,
            page: None,
            seen: HashMap::new(),
            first: true,
            next: None,
            pending: false,
            tx,
            rx,
        }
    }
}

impl GridStatus {
    /// The status page to open in the browser.
    pub fn page(&self) -> Option<&str> {
        self.page.as_deref()
    }

    /// Fetch again now (the window's Refresh button, or its opening).
    pub fn refresh(&mut self) {
        self.next = None;
    }

    /// Each frame: `feed` and `page` the grid's (None: it has none),
    /// `wanted` whether the feed is followed. Returns whether an entry came
    /// or changed (after the first answer): the button flashes.
    pub fn poll(&mut self, rt: &tokio::runtime::Handle, feed: Option<&str>, page: Option<&str>, wanted: bool, now: Instant) -> bool {
        if self.feed.as_deref() != feed {
            // another grid, or a region of an OpenSim grid naming another
            // feed: what was seen of the old one says nothing of this one
            let (tx, rx) = (self.tx.clone(), self.rx.clone());
            let flash = self.flash;
            *self = Self {
                feed: feed.map(str::to_owned),
                load: if feed.is_some() { Load::Loading } else { Load::NoFeed },
                flash,
                tx,
                rx,
                ..Self::default()
            };
        }
        if self.page.as_deref() != page {
            self.page = page.map(str::to_owned);
        }
        let mut news = false;
        while let Ok((url, result)) = self.rx.try_recv() {
            if self.feed.as_deref() != Some(url.as_str()) {
                continue;
            }
            self.pending = false;
            match result {
                Ok(entries) => news |= self.apply(entries),
                // offline or the site down: keep what is shown
                Err(e) => {
                    log::warn!("Grid status feed not updated: {e}");
                    if self.entries.is_empty() {
                        self.load = Load::Failed(e);
                    }
                }
            }
        }
        let Some(url) = self.feed.clone() else { return news };
        if wanted && !self.pending && self.next.is_none_or(|at| now >= at) {
            self.next = Some(now + UPDATE_DELAY);
            self.pending = true;
            if self.entries.is_empty() {
                self.load = Load::Loading;
            }
            let tx = self.tx.clone();
            if url == DEMO_FEED {
                let _ = tx.send((url, parse(crate::demo::grid_status::document().as_bytes())));
            } else {
                rt.spawn(async move {
                    let result = fetch(&url).await;
                    let _ = tx.send((url, result));
                });
            }
        }
        news
    }

    /// A new answer: whether an entry is new or changed, as
    /// getGridStatusRSSCoro compares sItemsMap (never on the first answer).
    fn apply(&mut self, entries: Vec<Entry>) -> bool {
        let mut news = false;
        for e in &entries {
            let changed = self.seen.get(&e.id) != Some(&e.updated_raw);
            if changed && !self.first {
                news = true;
                self.fresh.insert(e.id.clone());
            }
            self.seen.insert(e.id.clone(), e.updated_raw.clone());
        }
        self.first = false;
        self.entries = entries;
        self.load = Load::Loaded;
        news
    }
}

/// The feed and the status page of the grid: Second Life's, else what the
/// region's OpenSimExtras give (`GridStatusRSS`, `GridStatus`).
pub fn sources(second_life: bool, extras: Option<(&str, &str)>) -> (Option<String>, Option<String>) {
    if second_life {
        return (Some(SECOND_LIFE_FEED.into()), Some(SECOND_LIFE_PAGE.into()));
    }
    let (feed, page) = extras.unwrap_or_default();
    (web_link(feed), web_link(page))
}

#[cfg(test)]
mod tests;
