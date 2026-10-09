//! Avatar display names, ported from LLAvatarName / LLAvatarNameCache
//! (Second Life viewer and Firestorm, originally LGPL 2.1, Linden Research, Inc.):
//! People API names through the GetDisplayNames capability, legacy names
//! (UUIDNameRequest) as fallback, cache kept between sessions like
//! avatar_name_cache.xml.

use aurora_llsd::Llsd;
use aurora_net::AvatarNameData;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Time-to-live of a temporary entry (legacy name, failed lookup).
const TEMP_CACHE_ENTRY_LIFETIME: f64 = 60.0;
/// Maximum time an unrefreshed entry is kept.
const MAX_UNREFRESHED_TIME: f64 = 20.0 * 60.0;
/// Expiration when the reply has no Cache-Control max-age.
const DEFAULT_EXPIRES: f64 = 60.0 * 60.0;
/// A request still unanswered after this is sent again.
const PENDING_TIMEOUT: f64 = 5.0 * 60.0;

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// How names are written (Firestorm defaults).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NameOptions {
    /// UseDisplayNames.
    pub use_display_names: bool,
    /// NameTagShowUsernames: the username next to / under the display name.
    pub show_usernames: bool,
    /// FSNameTagShowLegacyUsernames: "Firstname Lastname" instead of the
    /// account name ("firstname.lastname").
    pub legacy_format: bool,
    /// FSTrimLegacyNames: hide the "Resident" last name.
    pub trim_resident: bool,
}

impl Default for NameOptions {
    fn default() -> Self {
        Self {
            use_display_names: true,
            show_usernames: true,
            legacy_format: true,
            trim_resident: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AvatarName {
    /// Account name ("jane.doe", "janedoe").
    pub username: String,
    pub display_name: String,
    pub legacy_first: String,
    pub legacy_last: String,
    /// The display name is only the username (none chosen).
    pub is_default: bool,
    /// Built from legacy data: not saved, refreshed soon.
    pub temporary: bool,
    /// Unix time after which the name is requested again.
    pub expires: f64,
    /// Unix time before which the display name cannot change again.
    pub next_update: f64,
}

impl AvatarName {
    pub fn from_data(d: &AvatarNameData, expires: f64) -> AvatarName {
        AvatarName {
            username: d.username.clone(),
            display_name: d.display_name.clone(),
            legacy_first: d.legacy_first.clone(),
            legacy_last: d.legacy_last.clone(),
            is_default: d.is_default,
            temporary: false,
            expires,
            next_update: d.next_update,
        }
    }

    /// LLAvatarName::fromString: a temporary name from the legacy service.
    pub fn from_legacy(first: &str, last: &str, lifetime: f64) -> AvatarName {
        let (first, last) = (first.trim(), last.trim());
        let resident = last.is_empty() || last.eq_ignore_ascii_case("resident");
        let (username, display_name) = if resident {
            // very old names have a dummy "Resident" last name: hidden
            (first.to_owned(), first.to_owned())
        } else {
            (format!("{first}.{last}").to_lowercase(), format!("{first} {last}"))
        };
        AvatarName {
            username,
            display_name,
            legacy_first: first.to_owned(),
            legacy_last: last.to_owned(),
            is_default: true,
            temporary: true,
            expires: now() + lifetime,
            next_update: 0.0,
        }
    }

    /// LLAvatarName::getUserNameForDisplay: "Firstname Lastname", or
    /// "Firstname" for Resident names when trimmed.
    pub fn user_name_for_display(&self, o: &NameOptions) -> String {
        let (f, l) = (&self.legacy_first, &self.legacy_last);
        if f.is_empty() && l.is_empty() {
            self.display_name.clone()
        } else if (l.is_empty() || l == "Resident") && o.trim_resident {
            f.clone()
        } else if l.is_empty() {
            format!("{f} Resident")
        } else {
            format!("{f} {l}")
        }
    }

    /// LLAvatarName::getDisplayName.
    pub fn display(&self, o: &NameOptions) -> String {
        if o.use_display_names {
            self.display_name.clone()
        } else {
            self.user_name_for_display(o)
        }
    }

    /// LLAvatarName::getCompleteName (with parentheses): "Jane Doe (janedoe)".
    pub fn complete(&self, o: &NameOptions) -> String {
        if !o.use_display_names {
            return self.user_name_for_display(o);
        }
        let (f, l) = (&self.legacy_first, &self.legacy_last);
        if self.username.is_empty() || self.is_default {
            // defaulted display name: only the easier to read form
            if o.trim_resident || f.is_empty() {
                self.display_name.clone()
            } else if l.is_empty() {
                format!("{f} Resident")
            } else {
                format!("{f} {l}")
            }
        } else if o.legacy_format && !f.is_empty() {
            let dn = &self.display_name;
            if o.trim_resident && (l == "Resident" || l.is_empty()) {
                format!("{dn} ({f})")
            } else if l.is_empty() {
                format!("{dn} ({f} Resident)")
            } else {
                format!("{dn} ({f} {l})")
            }
        } else if o.show_usernames {
            format!("{} ({})", self.display_name, self.username)
        } else {
            self.display_name.clone()
        }
    }

    /// Name tag lines (LLVOAvatar::idleUpdateNameTagText): the name, then
    /// the username in small when a display name was chosen.
    pub fn tag_lines(&self, o: &NameOptions) -> (String, Option<String>) {
        if !o.use_display_names {
            return (self.user_name_for_display(o), None);
        }
        let main = if self.is_default {
            self.user_name_for_display(o)
        } else {
            self.display_name.clone()
        };
        let second = (o.show_usernames && !self.is_default).then(|| {
            if o.legacy_format && !self.legacy_first.is_empty() {
                self.user_name_for_display(o)
            } else {
                self.username.clone()
            }
        });
        (main, second)
    }

    /// Username shown after the display name in chat headers
    /// (FSChatHistoryHeader: " - username" when a display name was chosen).
    pub fn chat_username(&self, o: &NameOptions) -> Option<String> {
        (o.use_display_names && o.show_usernames && !self.is_default).then(|| self.user_name_for_display(o))
    }

    fn to_llsd(&self) -> Llsd {
        let mut m = Llsd::new_map();
        m.insert("username", self.username.clone());
        m.insert("display_name", self.display_name.clone());
        m.insert("legacy_first_name", self.legacy_first.clone());
        m.insert("legacy_last_name", self.legacy_last.clone());
        m.insert("is_display_name_default", self.is_default);
        m.insert("display_name_expires", Llsd::Date(self.expires));
        m.insert("display_name_next_update", Llsd::Date(self.next_update));
        m
    }

    fn from_llsd(v: &Llsd) -> AvatarName {
        let mut d = AvatarNameData::from_llsd(v);
        d.id = Uuid::nil();
        AvatarName::from_data(&d, v["display_name_expires"].as_f64())
    }
}

/// LLAvatarNameCache.
#[derive(Default)]
pub struct AvatarNames {
    cache: HashMap<Uuid, AvatarName>,
    /// Names looked up while missing or expired: requested at the next poll.
    ask: Mutex<HashSet<Uuid>>,
    /// Requests in flight (sent at, unix time).
    pending: HashMap<Uuid, f64>,
    last_expire_check: f64,
    /// The region has no GetDisplayNames: legacy names only, kept longer.
    pub legacy_protocol: bool,
    /// Ids to ask the legacy service for (UUIDNameRequest).
    pub legacy_asks: Vec<Uuid>,
    /// Tell in the chat when someone changes their display name
    /// (FSShowDisplayNameUpdateNotification).
    pub notify_changes: bool,
    pub options: NameOptions,
    /// Bumped whenever a name changes.
    pub generation: u64,
    /// Contact set aliases (LGGContactSets pseudonyms, unquoted; "--- ---"
    /// = display name removed) and the cached names they rewrite, which
    /// `get` returns instead (the CustomNameCheckCallback of Firestorm's
    /// LLAvatarNameCache).
    aliases: HashMap<Uuid, String>,
    aliased: HashMap<Uuid, AvatarName>,
}

/// Name with a contact set alias: the alias in quotes as display name, or
/// the legacy name when the display name is removed.
fn with_alias(n: &AvatarName, alias: &str) -> AvatarName {
    let mut a = n.clone();
    if alias == super::contact_sets::DN_REMOVED {
        a.is_default = true;
        a.display_name = if a.legacy_last.is_empty() || a.legacy_last == "Resident" {
            a.legacy_first.clone()
        } else {
            format!("{} {}", a.legacy_first, a.legacy_last)
        };
        if a.display_name.is_empty() {
            a.display_name = n.display_name.clone();
        }
    } else {
        a.is_default = false;
        a.display_name = format!("'{alias}'");
    }
    a
}

impl AvatarNames {
    /// The cached name (even expired, which queues a refresh); a missing
    /// one is queued for the next request.
    pub fn get(&self, id: &Uuid) -> Option<&AvatarName> {
        if id.is_nil() {
            return None;
        }
        let n = self.cache.get(id);
        if n.is_none_or(|n| n.expires < now()) && !self.is_pending(id) {
            self.ask.lock().insert(*id);
        }
        n.map(|n| self.aliased.get(id).unwrap_or(n))
    }

    /// New contact set aliases: every name is rewritten again.
    pub fn set_aliases(&mut self, aliases: HashMap<Uuid, String>) {
        if aliases == self.aliases {
            return;
        }
        self.aliases = aliases;
        self.aliased = self
            .aliases
            .iter()
            .filter_map(|(id, a)| Some((*id, with_alias(self.cache.get(id)?, a))))
            .collect();
        self.generation += 1;
    }

    fn realias(&mut self, id: &Uuid) {
        match (self.cache.get(id), self.aliases.get(id)) {
            (Some(n), Some(a)) => {
                let n = with_alias(n, a);
                self.aliased.insert(*id, n);
            }
            _ => {
                self.aliased.remove(id);
            }
        }
    }

    /// Queue a lookup if the name is missing or expired.
    pub fn want(&self, id: &Uuid) {
        let _ = self.get(id);
    }

    /// Complete name ("Jane Doe (janedoe)") if known.
    pub fn complete(&self, id: &Uuid) -> Option<String> {
        self.get(id).map(|n| n.complete(&self.options))
    }

    fn is_pending(&self, id: &Uuid) -> bool {
        self.pending.get(id).is_some_and(|t| *t > now() - PENDING_TIMEOUT)
    }

    /// Ids to request now (marked in flight).
    pub fn take_asks(&mut self) -> Vec<Uuid> {
        let t = now();
        let asks: Vec<Uuid> = self.ask.lock().drain().collect();
        let v: Vec<Uuid> = asks.into_iter().filter(|id| !self.is_pending(id)).collect();
        for id in &v {
            self.pending.insert(*id, t);
        }
        v
    }

    /// LLAvatarNameCache::processName. Returns the previous entry.
    fn process(&mut self, id: Uuid, name: AvatarName) -> Option<AvatarName> {
        if id.is_nil() {
            return None;
        }
        self.pending.remove(&id);
        self.generation += 1;
        let old = self.cache.insert(id, name);
        self.realias(&id);
        old
    }

    /// GetDisplayNames reply (handleAvNameCacheSuccess).
    pub fn on_display_names(&mut self, names: &[AvatarNameData], bad_ids: &[Uuid], max_age: Option<u64>) {
        let expires = now() + max_age.map(|s| s as f64).unwrap_or(DEFAULT_EXPIRES);
        for d in names {
            self.process(d.id, AvatarName::from_data(d, expires));
        }
        if !bad_ids.is_empty() {
            log::warn!("GetDisplayNames: {} unresolved ids", bad_ids.len());
        }
        self.on_failed(bad_ids);
    }

    /// handleAgentError: a cached (expired) name is kept a minute more,
    /// otherwise the legacy name is fetched.
    pub fn on_failed(&mut self, ids: &[Uuid]) {
        for id in ids {
            if id.is_nil() {
                continue;
            }
            match self.cache.get_mut(id) {
                Some(n) => {
                    self.pending.remove(id);
                    n.expires = now() + TEMP_CACHE_ENTRY_LIFETIME;
                }
                None => self.legacy_asks.push(*id),
            }
        }
    }

    /// Legacy name (UUIDNameReply): stands in until the People API answers
    /// (legacyNameFetch), or for good without the capability
    /// (legacyNameCallback, 20 minutes).
    pub fn on_legacy(&mut self, id: Uuid, first: &str, last: &str) {
        if self.cache.get(&id).is_some_and(|n| !n.temporary) {
            return;
        }
        let lifetime = if self.legacy_protocol {
            MAX_UNREFRESHED_TIME
        } else {
            TEMP_CACHE_ENTRY_LIFETIME
        };
        self.process(id, AvatarName::from_legacy(first, last, lifetime));
    }

    /// DisplayNameUpdate: the new name replaces the cached one. Returns the
    /// notice to show ("[OLD_NAME] ([SLID]) a désormais le nom [NEW_NAME].").
    pub fn on_update(&mut self, d: &AvatarNameData, old_display_name: &str) -> String {
        let n = AvatarName::from_data(d, now() + DEFAULT_EXPIRES);
        // getUserName(): "Firstname Lastname" or "Firstname"
        let slid = if n.legacy_first.is_empty() {
            n.display_name.clone()
        } else if n.legacy_last.is_empty() || n.legacy_last == "Resident" {
            n.legacy_first.clone()
        } else {
            format!("{} {}", n.legacy_first, n.legacy_last)
        };
        let msg = format!("{old_display_name} ({slid}) a désormais le nom {}.", n.display_name);
        self.process(d.id, n);
        msg
    }

    /// Forget a name (our own, refused as out of date) and ask it again.
    pub fn refetch(&mut self, id: &Uuid) {
        self.cache.remove(id);
        self.aliased.remove(id);
        self.pending.remove(id);
        self.generation += 1;
        self.want(id);
    }

    /// LLAvatarNameCache::eraseUnrefreshed, every 20 minutes.
    pub fn idle(&mut self) {
        let t = now();
        let max_unrefreshed = t - MAX_UNREFRESHED_TIME;
        if self.last_expire_check == 0.0 || self.last_expire_check < max_unrefreshed {
            self.last_expire_check = t;
            let before = self.cache.len();
            self.cache.retain(|_, n| n.expires >= max_unrefreshed);
            let cache = &self.cache;
            self.aliased.retain(|id, _| cache.contains_key(id));
            let expired = before - self.cache.len();
            if expired > 0 {
                self.generation += 1;
            }
            log::info!("avatar names: {expired} expired, {} cached", self.cache.len());
        }
    }

    /// importFile.
    pub fn load(&mut self, path: &Path) {
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let Ok(v) = aurora_llsd::from_xml(&bytes) else {
            log::warn!("avatar name cache unreadable, removed: {}", path.display());
            let _ = std::fs::remove_file(path);
            return;
        };
        if let Some(m) = v["agents"].as_map() {
            for (k, e) in m.iter() {
                if let Ok(id) = Uuid::parse_str(k) {
                    self.cache.insert(id, AvatarName::from_llsd(e));
                    self.realias(&id);
                }
            }
        }
        self.generation += 1;
        log::info!("avatar name cache: {} names loaded", self.cache.len());
    }

    /// exportFile: temporary and long expired names are left out.
    pub fn save(&self, path: &Path) {
        if self.cache.is_empty() {
            return;
        }
        let max_unrefreshed = now() - MAX_UNREFRESHED_TIME;
        let mut agents = Llsd::new_map();
        for (id, n) in &self.cache {
            if !n.temporary && n.expires >= max_unrefreshed {
                agents.insert(id.to_string(), n.to_llsd());
            }
        }
        let mut data = Llsd::new_map();
        data.insert("agents", agents);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&data)) {
            log::warn!("avatar name cache not saved: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(dn: &str, user: &str, first: &str, last: &str, default: bool) -> AvatarName {
        AvatarName {
            username: user.into(),
            display_name: dn.into(),
            legacy_first: first.into(),
            legacy_last: last.into(),
            is_default: default,
            temporary: false,
            expires: 0.0,
            next_update: 0.0,
        }
    }

    #[test]
    fn complete_names_like_firestorm() {
        let o = NameOptions::default();
        let a = named("Jane Doe", "janedoe", "janedoe", "Resident", false);
        assert_eq!(a.complete(&o), "Jane Doe (janedoe)");
        assert_eq!(a.tag_lines(&o), ("Jane Doe".into(), Some("janedoe".into())));
        let b = named("Bob", "bob.smith", "Bob", "Smith", false);
        assert_eq!(b.complete(&o), "Bob (Bob Smith)");
        let c = named("janedoe", "janedoe", "janedoe", "Resident", true);
        assert_eq!(c.complete(&o), "janedoe");
        assert_eq!(c.tag_lines(&o), ("janedoe".into(), None));
        let plain = NameOptions { legacy_format: false, ..o };
        assert_eq!(b.complete(&plain), "Bob (bob.smith)");
        let off = NameOptions {
            use_display_names: false,
            ..o
        };
        assert_eq!(b.complete(&off), "Bob Smith");
        let untrimmed = NameOptions { trim_resident: false, ..o };
        assert_eq!(a.complete(&untrimmed), "Jane Doe (janedoe Resident)");
    }

    #[test]
    fn people_api_reply() {
        let xml = br#"<?xml version="1.0"?><llsd><map><key>agents</key><array><map>
            <key>display_name_next_update</key><date>2010-04-16T21:34:02+00:00Z</date>
            <key>display_name_expires</key><date>2010-04-16T21:32:26.142178+00:00Z</date>
            <key>display_name</key><string>MickBot390 LLQABot</string>
            <key>sl_id</key><string>mickbot390.llqabot</string>
            <key>id</key><string>0012809d-7d2d-4c24-9609-af1230a37715</string>
            <key>is_display_name_default</key><boolean>false</boolean>
            </map></array></map></llsd>"#;
        let v = aurora_llsd::from_xml(xml).unwrap();
        let d = AvatarNameData::from_llsd(&v["agents"][0]);
        assert_eq!(d.username, "mickbot390.llqabot");
        assert_eq!(d.display_name, "MickBot390 LLQABot");
        assert!(!d.is_default);
        assert_eq!(d.next_update, 1_271_453_642.0);
    }

    #[test]
    fn legacy_names() {
        let n = AvatarName::from_legacy("janedoe", "Resident", 60.0);
        assert_eq!((n.username.as_str(), n.display_name.as_str()), ("janedoe", "janedoe"));
        let n = AvatarName::from_legacy("Bob", "Smith", 60.0);
        assert_eq!((n.username.as_str(), n.display_name.as_str()), ("bob.smith", "Bob Smith"));
        assert!(n.temporary && n.is_default);
    }

    #[test]
    fn contact_set_aliases() {
        let (jane, bob) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut names = AvatarNames::default();
        names.process(jane, named("Jane Doe", "janedoe", "janedoe", "Resident", false));
        names.set_aliases(HashMap::from([
            (jane, "Janou".to_owned()),
            (bob, crate::world::contact_sets::DN_REMOVED.to_owned()),
        ]));
        let o = NameOptions::default();
        assert_eq!(names.complete(&jane).as_deref(), Some("'Janou' (janedoe)"));
        // a name arriving later gets its alias too: the display name removed
        names.process(bob, named("Bobby", "bob.smith", "Bob", "Smith", false));
        let b = names.get(&bob).expect("cached");
        assert!(b.is_default);
        assert_eq!(b.display(&o), "Bob Smith");
        // the alias gone, the real names come back
        names.set_aliases(HashMap::new());
        assert_eq!(names.get(&jane).map(|n| n.display(&o)).as_deref(), Some("Jane Doe"));
    }
}
