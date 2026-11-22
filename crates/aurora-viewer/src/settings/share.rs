//! The settings as a share code: plain text to hand to a friend (pasted in
//! a notecard, a chat, a forum), so that a new user can take someone's
//! looks or setup, page by page, after a preview (Preferences › Language
//! and updates › Share; the windows are `ui/options/share.rs`).
//!
//! ```text
//! # Aurora settings v1
//! # Aurora 0.9.11
//! name: Cozy evening
//! pages: graphics, appearance
//! # graphics
//! exposure = 1.2
//! shadow_quality = 2
//! # appearance
//! colors.chat_direct = [200, 180, 255, 255]
//! ```
//!
//! Only the settings that differ from their defaults are written, and only
//! those of the pages chosen: a code is short and says what it changes.
//! The Legacy Viewer has no such code (its presets are files of one kind:
//! graphics, camera; LLPresetsManager); this one carries any page of
//! Preferences.
//!
//! What a code never carries, and what an import refuses even when someone
//! wrote it in by hand (stricter on both ends):
//! - the settings of the login and of this PC, and the service keys
//!   (`backup::is_this_pc`), with the settings Debug Settings hides
//!   (`registry`);
//! - the pages that guard the user ([`NEVER_SHARED`]): privacy, protections,
//!   RLVa, network and proxy, language and updates, confirmations, chat
//!   commands and game controls; and on the other pages the settings that protect the user or
//!   send something out ([`PROTECTIVE`]: auto-accepting inventory, media
//!   that plays by itself or skips the site filter, announces on a chat
//!   channel, the scripted bridge). A code from someone else never lowers a
//!   protection nor redirects the network;
//! - a setting no page of Preferences shows (Debug Settings only);
//! - a value holding a UUID (someone's asset, AGENTS.md "Creators'
//!   permissions"), a web address, a path, an e-mail address, a line break
//!   or a long text;
//! - lists and per-account maps (the registry lists single values only).
//!
//! Reading is defensive: the text and its lines are capped, unknown or
//! broken lines are skipped and counted, and every value goes through the
//! settings' own deserialisation and bounds (`registry::set_many`).

use super::Settings;
use super::registry::{self, Locks};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub const HEADER: &str = "# Aurora settings v1";
/// The longest text read, in bytes (a notecard holds 64 KB).
pub const MAX_TEXT: usize = 64 * 1024;
/// The most setting lines read.
pub const MAX_LINES: usize = 3000;
/// The longest text value shared: names of fonts, of styles, short words.
const MAX_TEXT_VALUE: usize = 120;
/// The longest name of a code.
pub const MAX_NAME: usize = 60;

/// The pages of Preferences a code may carry, by `Page::key`.
pub const SHARED_PAGES: &[&str] = &[
    "aurora",
    "graphics",
    "camera",
    "vr",
    "avatars",
    "build",
    "chat",
    "sound",
    "media",
    "layout",
    "appearance",
    "accessibility",
    "inventory",
    "controls",
    "gamepad",
    "shortcuts",
    "fun",
];

/// The pages a code never carries, and whose settings an import refuses:
/// they guard the user, or belong to this PC and its network. Confirmations
/// guard payments, deletions and teleports; Commands holds the chat command
/// words, which a code could rename to an everyday word, and their
/// announces on a chat channel. Game Controls sends a controller's input
/// to the region's scripts and maps this PC's devices.
pub const NEVER_SHARED: &[&str] = &[
    "privacy",
    "protections",
    "rlva",
    "network",
    "system",
    "confirmations",
    "commands",
    "game-controls",
];

/// The pages "Looks only" ticks: what the world and the windows look like.
pub const LOOKS: &[&str] = &["graphics", "appearance", "layout", "camera"];

/// Settings on a shared page that are still the user's own business: a
/// text they wrote, or a choice of this PC's hardware.
const PERSONAL: &[&str] = &[
    // the replies a user wrote, and the words their chat alerts on
    "autoresponse",
    "keywords",
    // the place whose night sky is drawn: where the user may live
    "night_sky_place",
    // this PC's graphics card, controllers, media plugins and the notices
    // it dismissed
    "gpu_adapter",
    "gamepad.device",
    "joystick.device",
    "media.plugin_dir",
    "gpu_driver_dismissed",
    "ignored_update",
];

/// Settings on a shared page that protect the user or send something out:
/// a code never carries them and an import refuses them, so that a code
/// from someone else never lowers a protection (a new resident would tick
/// "Apply" without recognising them in the preview).
const PROTECTIVE: &[&str] = &[
    // items given to the user land without asking
    "auto_accept_inventory",
    "auto_accept_inventory_chat",
    "auto_accept_inventory_notify",
    // media that plays by itself or a site not asked about reveals the
    // user's IP address to whoever hosts it
    "audio.media_autoplay",
    "media.autoplay_huds",
    "media.allow_scripted",
    "media.filter",
    "media.filter_trust_own_worn",
    // posts on a chat channel, channel 0 included
    "audio.stream_title_announce",
    "audio.stream_title_channel",
    // the scripted bridge worn by the viewer
    "use_bridge",
];

/// Which page of Preferences shows each setting, by its path. Built from
/// the pages themselves (`ui::options::setting_pages`), so a new row is
/// shared with its page without a line here.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    pages: HashMap<String, &'static str>,
}

impl Catalog {
    pub fn new(rows: impl IntoIterator<Item = (&'static str, String)>) -> Catalog {
        let mut pages: HashMap<String, &'static str> = HashMap::new();
        for (page, path) in rows {
            // A setting keeps one page: the first that declares it, unless
            // a page never shared declares it too. Then that one, whatever
            // the order of the pages, so that a protection shown on a shared
            // page as well (the no-transfer confirmation on Graphics, the
            // delete confirmation on Inventory) never travels in a code.
            let at = pages.entry(path).or_insert(page);
            if NEVER_SHARED.contains(&page) && !NEVER_SHARED.contains(at) {
                *at = page;
            }
        }
        Catalog { pages }
    }

    /// The page of a setting: its own row, or the row that edits the map or
    /// the section it is in (`audio.ui_sounds` for `audio.ui_sounds.chat`).
    pub fn page(&self, key: &str) -> Option<&'static str> {
        let mut at = key;
        loop {
            if let Some(page) = self.pages.get(at) {
                return Some(page);
            }
            at = at.rsplit_once('.')?.0;
        }
    }

    /// The page of a setting a code may carry.
    fn shared_page(&self, key: &str) -> Option<&'static str> {
        self.page(key)
            .filter(|page| SHARED_PAGES.contains(page) && !NEVER_SHARED.contains(page))
    }
}

/// Whether `key` is one of `list` or inside one of them (`keywords.words`).
fn listed(list: &[&str], key: &str) -> bool {
    list.iter()
        .any(|p| key == *p || key.strip_prefix(p).is_some_and(|rest| rest.starts_with('.')))
}

/// A setting no code carries and no import takes, whatever its page: the
/// user's own business, or a protection.
fn kept_out(key: &str) -> bool {
    listed(PERSONAL, key) || listed(PROTECTIVE, key)
}

/// A value that may travel in a code (both ways).
fn safe_value(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            text.len() <= MAX_TEXT_VALUE
                && !registry::holds_uuid(text)
                && !text.contains("://")
                && !text.contains(['/', '\\', '@', '\n', '\r'])
        }
        Value::Bool(_) | Value::Number(_) => true,
        Value::Array(items) => items.iter().all(Value::is_number),
        _ => false,
    }
}

/// The settings of `pages` that differ from their defaults, each with its
/// page, in the order of the settings.
pub fn shareable(settings: &Settings, catalog: &Catalog, pages: &[&str]) -> Vec<(&'static str, String, Value)> {
    registry::entries(settings, &registry::defaults())
        .into_iter()
        .filter(|e| e.changed() && !kept_out(&e.key) && safe_value(&e.value))
        .filter_map(|e| {
            let page = catalog.shared_page(&e.key)?;
            pages.contains(&page).then_some((page, e.key, e.value))
        })
        .collect()
}

/// How many settings each shared page would carry (the Share window's
/// counts).
pub fn counts(settings: &Settings, catalog: &Catalog) -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    for (page, _, _) in shareable(settings, catalog, SHARED_PAGES) {
        *out.entry(page).or_insert(0) += 1;
    }
    out
}

/// A name as one short line.
fn clean_name(name: &str) -> String {
    let one_line: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    one_line.trim().chars().take(MAX_NAME).collect()
}

/// The code of `pages` (in the order of [`SHARED_PAGES`]), ready for the
/// clipboard. A page with nothing changed is still named: an import can
/// put its settings back to their defaults.
pub fn export(settings: &Settings, catalog: &Catalog, pages: &[&str], name: &str) -> String {
    let pages: Vec<&str> = SHARED_PAGES.iter().copied().filter(|p| pages.contains(p)).collect();
    let values = shareable(settings, catalog, &pages);
    let mut out = vec![HEADER.to_owned(), format!("# Aurora {}", aurora_version::version())];
    let name = clean_name(name);
    if !name.is_empty() {
        out.push(format!("name: {name}"));
    }
    out.push(format!("pages: {}", pages.join(", ")));
    for page in &pages {
        let mut lines = values.iter().filter(|(p, _, _)| p == page).peekable();
        if lines.peek().is_none() {
            continue;
        }
        out.push(format!("# {page}"));
        for (_, key, value) in lines {
            out.push(format!("{key} = {value}"));
        }
    }
    out.join("\n") + "\n"
}

/// Why a pasted text is not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// No header: not a code of Aurora's settings.
    NotACode,
    TooLong,
}

/// What a pasted code holds, before it is compared with the user's
/// settings.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Code {
    pub name: String,
    /// The pages the sender chose, shared ones only.
    pub pages: Vec<&'static str>,
    pub values: Vec<(String, Value)>,
    /// Lines that could not be read (comments and blank lines are not
    /// counted).
    pub skipped: usize,
}

impl Code {
    pub fn parse(text: &str) -> Result<Code, Refused> {
        if text.len() > MAX_TEXT {
            return Err(Refused::TooLong);
        }
        // whatever came before the header (a chat's time stamp and name, a
        // message's greeting) is not the code's
        let Some(start) = text.find(HEADER) else {
            return Err(Refused::NotACode);
        };
        let mut code = Code::default();
        for line in text[start + HEADER.len()..].lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix("name:") {
                code.name = clean_name(name);
            } else if let Some(pages) = line.strip_prefix("pages:") {
                code.pages = pages
                    .split(',')
                    .filter_map(|p| SHARED_PAGES.iter().copied().find(|s| *s == p.trim()))
                    .collect();
                // each page once, wherever it repeats
                let mut seen = std::collections::HashSet::new();
                code.pages.retain(|page| seen.insert(*page));
            } else if let Some((key, value)) = line.split_once(" = ")
                && code.values.len() < MAX_LINES
                && let Ok(value) = serde_json::from_str::<Value>(value.trim())
                && valid_key(key.trim())
            {
                let key = key.trim().to_owned();
                code.values.retain(|(k, _)| *k != key);
                code.values.push((key, value));
            } else {
                code.skipped += 1;
            }
        }
        Ok(code)
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// One setting an import changes.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub page: &'static str,
    pub key: String,
    pub from: Value,
    pub to: Value,
    /// Not in the code: back to its default ("reset the other settings of
    /// these pages").
    pub reset: bool,
}

/// What an import would do to `settings`, and what it leaves out.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Plan {
    pub changes: Vec<Change>,
    /// Values the code holds that this viewer refuses: an unknown setting
    /// (a newer or another viewer's), one of a page never shared, a value
    /// of the wrong type, a value that may not travel, a setting RLVa holds.
    pub refused: usize,
    /// Values the user already has.
    pub same: usize,
}

impl Plan {
    /// Compares `code` with `settings`. With `reset_others`, the settings of
    /// the code's pages that it does not name go back to their defaults, so
    /// that those pages end up as the sender's.
    pub fn new(code: &Code, settings: &Settings, catalog: &Catalog, locks: &Locks, reset_others: bool) -> Plan {
        let defaults = registry::defaults();
        let entries: BTreeMap<String, registry::Entry> = registry::entries(settings, &defaults)
            .into_iter()
            .map(|e| (e.key.clone(), e))
            .collect();
        let mut plan = Plan::default();
        for (key, value) in &code.values {
            let page = catalog.shared_page(key);
            let entry = entries.get(key);
            let (Some(page), Some(entry)) = (page, entry) else {
                plan.refused += 1;
                continue;
            };
            if kept_out(key) || locks.locked(key) || !safe_value(value) || !entry.accepts(value) {
                plan.refused += 1;
                continue;
            }
            if entry.value == *value || same_number(&entry.value, value) {
                plan.same += 1;
                continue;
            }
            plan.changes.push(Change {
                page,
                key: key.clone(),
                from: entry.value.clone(),
                to: value.clone(),
                reset: false,
            });
        }
        if reset_others {
            for entry in entries.values() {
                let Some(page) = catalog.shared_page(&entry.key) else {
                    continue;
                };
                if code.pages.contains(&page)
                    && entry.changed()
                    && !kept_out(&entry.key)
                    && !locks.locked(&entry.key)
                    && !code.values.iter().any(|(k, _)| *k == entry.key)
                {
                    plan.changes.push(Change {
                        page,
                        key: entry.key.clone(),
                        from: entry.value.clone(),
                        to: entry.default.clone(),
                        reset: true,
                    });
                }
            }
        }
        // the Import window lists them by page, in the order of the sidebar
        plan.changes.sort_by_key(|c| SHARED_PAGES.iter().position(|p| *p == c.page));
        plan
    }

    /// The pages the plan changes, in the order of the sidebar.
    pub fn pages(&self) -> Vec<&'static str> {
        let mut pages: Vec<&'static str> = self.changes.iter().map(|c| c.page).collect();
        pages.dedup();
        pages
    }

    /// Applies the changes `keep` says yes to (the settings ticked).
    pub fn apply_if(&self, settings: &mut Settings, keep: impl Fn(&Change) -> bool) -> usize {
        let values: Vec<(&str, Value)> = self
            .changes
            .iter()
            .filter(|c| keep(c))
            .map(|c| (c.key.as_str(), c.to.clone()))
            .collect();
        registry::set_many(settings, &values)
    }
}

/// 2 and 2.0 are one value.
fn same_number(a: &Value, b: &Value) -> bool {
    matches!((a.as_f64(), b.as_f64()), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
#[path = "share_tests.rs"]
mod tests;
