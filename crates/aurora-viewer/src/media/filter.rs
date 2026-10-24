//! Media filter: asks before a parcel media, a media on a prim or the parcel
//! music reaches a site it has no rule for, and keeps the account's allow /
//! deny list. Port of the media filter of LLViewerParcelMedia
//! (filterMediaUrl, filterAudioUrl, the callback_media_alert* answers,
//! extractDomain, saveDomainFilterList / loadDomainFilterList) and of
//! FloaterMediaLists (indra/newview/llviewerparcelmedia.cpp,
//! floatermedialists.cpp, originally LGPL 2.1).
//!
//! The list is the account's `medialist.xml`, in the Legacy Viewer's format
//! (an LLSD array of `{domain, action}` maps): a list made there can be
//! copied over.
//!
//! Deliberate differences, each for the resident:
//! * media on a prim is filtered too (the Legacy Viewer filters the parcel
//!   media and music only), and the check comes before anything reaches the
//!   site, the request that discovers the media type included; media on our
//!   own worn items (attachments, HUDs) is not asked about by default (the
//!   list's refusals still apply), closer to the Legacy Viewer, which asks
//!   about no prim media at all;
//! * a domain rule matches that domain and its subdomains at a dot
//!   (`example.com` no longer matches `badexample.com`), and the most
//!   precise rule wins (the URL, then the longest domain) instead of the
//!   first one added;
//! * "only this time" holds for that URL until logout, instead of until
//!   another URL is asked about;
//! * a URL without a host (`data:`, `about:`) is not asked about: by
//!   itself it contacts no site (what a page loads afterwards is never
//!   filtered, there as here);
//! * one question that offers to remember the answer, instead of two in a
//!   row, with the first use's explanation in it (FirstMediaFilter) instead
//!   of a dialog before it.

use aurora_llsd::{Llsd, Map};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The account's file, as the Legacy Viewer names it (LL_PATH_PER_SL_ACCOUNT).
pub const FILE: &str = "medialist.xml";

/// A question nothing asks for any more (the media went away, the music
/// stopped) is dropped after this.
const STALE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Allow,
    Deny,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Deny => "deny",
        }
    }
}

/// One entry of the list: a domain (with its subdomains) or a whole URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub pattern: String,
    pub action: Action,
}

impl Rule {
    pub fn is_url(&self) -> bool {
        self.pattern.contains("://")
    }
}

/// What asks to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ParcelMedia,
    PrimMedia,
    Music,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
    /// No rule: the question is asked, nothing loads until it is answered.
    Ask,
}

/// What an answer is kept for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Remember {
    /// This URL, until logout.
    #[default]
    Once,
    /// The domain, in the list.
    Domain,
    /// The URL, in the list.
    Url,
}

#[derive(Debug, Clone)]
pub struct Question {
    pub url: String,
    pub domain: String,
    pub source: Source,
    last_seen: Instant,
}

/// A change made in the preferences' list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// A domain (or a URL) typed by the resident, allowed or denied; an
    /// entry for the same domain changes.
    Set(String, Action),
    Remove(String),
}

#[derive(Default)]
pub struct MediaFilter {
    rules: Vec<Rule>,
    /// The account's file (none in the demo: nothing is written).
    path: Option<PathBuf>,
    /// "Only this time" answers, by URL.
    session: HashMap<String, Action>,
    questions: VecDeque<Question>,
    /// URLs just denied from the question: the refusal that follows is
    /// not told in the chat a second time.
    just_denied: HashSet<String>,
    /// Lines for the local chat (FSCommon::report_to_nearby_chat), drained
    /// by the app.
    pub messages: Vec<String>,
}

/// Schemes whose URLs name no site.
const HOSTLESS: [&str; 5] = ["data", "about", "javascript", "blob", "mailto"];

fn is_scheme(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
}

/// LLViewerParcelMedia::extractDomain: the host of a URL, lower case,
/// without the scheme, the user, the port nor the path; a URL without a
/// scheme starts with its host. `None` for a URL without a host (`data:`,
/// `about:`, `file:///`).
pub fn domain(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = match url.find("://") {
        Some(i) if is_scheme(&url[..i]) => &url[i + 3..],
        _ => {
            let scheme = url.split(':').next().unwrap_or("");
            if url.contains(':') && HOSTLESS.iter().any(|s| scheme.eq_ignore_ascii_case(s)) {
                return None;
            }
            url
        }
    };
    let host = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = if let Some(v6) = host.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The list's answer for a URL: its own entry, else the most precise
/// domain.
fn rule_for(rules: &[Rule], url: &str, host: &str) -> Option<Action> {
    if let Some(r) = rules.iter().find(|r| r.is_url() && r.pattern == url) {
        return Some(r.action);
    }
    rules
        .iter()
        .filter(|r| !r.is_url() && covers(&r.pattern, host))
        .max_by_key(|r| r.pattern.len())
        .map(|r| r.action)
}

/// A domain rule covers the domain itself and its subdomains.
fn covers(pattern: &str, host: &str) -> bool {
    host == pattern || host.strip_suffix(pattern).is_some_and(|sub| sub.ends_with('.'))
}

/// What the preferences' field makes of a typed entry (onWhitelistAdd /
/// handleAddDomainCallback): a URL stays whole when it has a path,
/// anything else becomes its domain.
fn pattern_of(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.contains("://") {
        let host = domain(typed)?;
        let path = typed.split_once("://").map_or("", |(_, r)| r.trim_start_matches(|c| c != '/'));
        return Some(if path.trim_matches('/').is_empty() {
            host
        } else {
            typed.to_string()
        });
    }
    domain(&format!("http://{typed}"))
}

impl MediaFilter {
    /// loadDomainFilterList: the account's list.
    pub fn load(path: PathBuf) -> Self {
        let mut filter = MediaFilter {
            path: Some(path.clone()),
            ..MediaFilter::default()
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("media filter list not read: {e}");
                }
                return filter;
            }
        };
        let data = match aurora_llsd::from_xml(&bytes) {
            Ok(data) => data,
            Err(e) => {
                log::warn!("media filter list unreadable: {e:?}");
                return filter;
            }
        };
        for entry in data.as_array() {
            let action = match entry.get("action").as_str() {
                "allow" => Action::Allow,
                "deny" => Action::Deny,
                _ => continue,
            };
            if let Some(pattern) = pattern_of(entry.get("domain").as_str()) {
                filter.put(pattern, action);
            }
        }
        log::info!("media filter: {} entries", filter.rules.len());
        filter
    }

    /// saveDomainFilterList.
    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let list = self
            .rules
            .iter()
            .map(|r| {
                let mut m = Map::new();
                m.insert("domain".into(), Llsd::from(r.pattern.as_str()));
                m.insert("action".into(), Llsd::from(r.action.as_str()));
                Llsd::Map(m)
            })
            .collect();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&Llsd::Array(list))) {
            log::warn!("media filter list not saved: {e}");
        }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// filterMediaUrl / filterAudioUrl, called every frame for what wants
    /// to load: the list, then this session's answers, else the question.
    /// A refusal is told in the local chat.
    pub fn check(&mut self, url: &str, source: Source) -> Verdict {
        self.decide(url, source, true)
    }

    /// The same for a media the resident trusts without being asked (on
    /// their own worn items): a site the list or this session refused stays
    /// refused, any other loads, and no question is queued.
    pub fn check_trusted(&mut self, url: &str, source: Source) -> Verdict {
        self.decide(url, source, false)
    }

    fn decide(&mut self, url: &str, source: Source, ask: bool) -> Verdict {
        let url = url.trim();
        let Some(host) = domain(url) else {
            return Verdict::Allow;
        };
        match rule_for(&self.rules, url, &host).or_else(|| self.session.get(url).copied()) {
            Some(Action::Allow) => Verdict::Allow,
            Some(Action::Deny) => {
                if !self.just_denied.remove(url) {
                    log::info!("media filter: {host} blocked");
                    self.messages.push(match source {
                        Source::Music => aurora_i18n::tr!("media-filter-music-blocked", domain = host),
                        _ => aurora_i18n::tr!("media-filter-media-blocked", domain = host),
                    });
                }
                Verdict::Deny
            }
            None if !ask => Verdict::Allow,
            None => {
                let now = Instant::now();
                match self.questions.iter_mut().find(|q| q.url == url) {
                    Some(q) => q.last_seen = now,
                    None => {
                        log::info!("media filter: asking about {host}");
                        self.questions.push_back(Question {
                            url: url.to_string(),
                            domain: host,
                            source,
                            last_seen: now,
                        });
                    }
                }
                Verdict::Ask
            }
        }
    }

    /// The question to show (one at a time, the oldest first); those
    /// nothing asks for any more are dropped.
    pub fn question(&mut self) -> Option<&Question> {
        self.questions.retain(|q| q.last_seen.elapsed() < STALE);
        self.questions.front()
    }

    /// callback_media_alert2 / callback_media_alert_single: the answer to
    /// the question about `url`, kept as `remember` says.
    pub fn answer(&mut self, url: &str, allow: bool, remember: Remember) {
        let Some(i) = self.questions.iter().position(|q| q.url == url) else {
            return;
        };
        let Some(q) = self.questions.remove(i) else {
            return;
        };
        let action = if allow { Action::Allow } else { Action::Deny };
        let music = q.source == Source::Music;
        let told = match (remember, allow, music) {
            (Remember::Once, ..) => None,
            (Remember::Domain, true, false) => Some(aurora_i18n::tr!("media-filter-media-domain-allowed", domain = q.domain.clone())),
            (Remember::Domain, false, false) => Some(aurora_i18n::tr!("media-filter-media-domain-blocked", domain = q.domain.clone())),
            (Remember::Domain, true, true) => Some(aurora_i18n::tr!("media-filter-music-domain-allowed", domain = q.domain.clone())),
            (Remember::Domain, false, true) => Some(aurora_i18n::tr!("media-filter-music-domain-blocked", domain = q.domain.clone())),
            (Remember::Url, true, false) => Some(aurora_i18n::tr!("media-filter-media-url-allowed", url = q.url.clone())),
            (Remember::Url, false, false) => Some(aurora_i18n::tr!("media-filter-media-url-blocked", url = q.url.clone())),
            (Remember::Url, true, true) => Some(aurora_i18n::tr!("media-filter-music-url-allowed", url = q.url.clone())),
            (Remember::Url, false, true) => Some(aurora_i18n::tr!("media-filter-music-url-blocked", url = q.url.clone())),
        };
        self.messages.extend(told);
        match remember {
            Remember::Once => {
                self.session.insert(q.url.clone(), action);
            }
            Remember::Domain => self.set(q.domain.clone(), action),
            Remember::Url => self.set(q.url.clone(), action),
        }
        if !allow {
            self.just_denied.insert(q.url.clone());
        }
        // the questions this answer settles go too
        let (rules, session) = (&self.rules, &self.session);
        self.questions
            .retain(|o| !session.contains_key(&o.url) && rule_for(rules, &o.url, &o.domain).is_none());
    }

    /// A change from the preferences' list.
    pub fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Set(typed, action) => {
                if let Some(pattern) = pattern_of(&typed) {
                    self.set(pattern, action);
                }
            }
            Edit::Remove(pattern) => {
                let before = self.rules.len();
                self.rules.retain(|r| r.pattern != pattern);
                if self.rules.len() != before {
                    self.save();
                }
            }
        }
    }

    /// An entry added, or changed when the list has it already.
    fn set(&mut self, pattern: String, action: Action) {
        self.put(pattern, action);
        self.save();
    }

    fn put(&mut self, pattern: String, action: Action) {
        match self.rules.iter_mut().find(|r| r.pattern == pattern) {
            Some(r) => r.action = action,
            None => self.rules.push(Rule { pattern, action }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_is_the_host_alone() {
        assert_eq!(domain("http://Example.COM/video.mp4").as_deref(), Some("example.com"));
        assert_eq!(
            domain("https://user:pa@ss@media.example.com:8443/x?y#z").as_deref(),
            Some("media.example.com")
        );
        assert_eq!(domain("rtsp://stream.example.net:554").as_deref(), Some("stream.example.net"));
        assert_eq!(domain("http://example.com?tracking=1").as_deref(), Some("example.com"));
        assert_eq!(domain("http://[2001:db8::1]:8000/live").as_deref(), Some("2001:db8::1"));
        assert_eq!(domain("http://example.com./").as_deref(), Some("example.com"));
        // no scheme: the host comes first
        assert_eq!(domain("radio.example.org:8000/live").as_deref(), Some("radio.example.org"));
        // no host: nothing to ask about, whatever the page holds
        assert_eq!(domain("data:text/html,<p>hello</p>"), None);
        assert_eq!(domain("data:text/html,<img src='http://tracker.example/p'>"), None);
        assert_eq!(domain("about:blank"), None);
        assert_eq!(domain("file:///C:/page.html"), None);
    }

    #[test]
    fn a_domain_rule_covers_its_subdomains_at_a_dot() {
        assert!(covers("example.com", "example.com"));
        assert!(covers("example.com", "media.example.com"));
        assert!(!covers("example.com", "badexample.com"));
        assert!(!covers("media.example.com", "example.com"));
    }

    #[test]
    fn the_most_precise_rule_wins() {
        let mut f = MediaFilter::default();
        f.set("example.com".into(), Action::Allow);
        f.set("ads.example.com".into(), Action::Deny);
        f.set("http://ads.example.com/ok.mp4".into(), Action::Allow);
        assert_eq!(f.check("http://www.example.com/", Source::ParcelMedia), Verdict::Allow);
        assert_eq!(f.check("http://ads.example.com/", Source::ParcelMedia), Verdict::Deny);
        assert_eq!(f.check("http://ads.example.com/ok.mp4", Source::ParcelMedia), Verdict::Allow);
        assert_eq!(f.messages.len(), 1, "the refusal is told");
    }

    #[test]
    fn an_unknown_site_is_asked_about_once() {
        let mut f = MediaFilter::default();
        assert_eq!(f.check("data:text/html,x", Source::PrimMedia), Verdict::Allow);
        assert_eq!(f.check("http://a.example/1", Source::PrimMedia), Verdict::Ask);
        assert_eq!(f.check("http://a.example/1", Source::PrimMedia), Verdict::Ask);
        assert_eq!(f.check("http://b.example/", Source::Music), Verdict::Ask);
        assert_eq!(f.question().map(|q| q.domain.as_str()), Some("a.example"));
        assert_eq!(f.questions.len(), 2);
    }

    #[test]
    fn answers_are_kept_as_asked() {
        let mut f = MediaFilter::default();
        // only this time: this URL, not the domain
        f.check("http://a.example/1", Source::ParcelMedia);
        f.answer("http://a.example/1", true, Remember::Once);
        assert_eq!(f.check("http://a.example/1", Source::ParcelMedia), Verdict::Allow);
        assert_eq!(f.check("http://a.example/2", Source::ParcelMedia), Verdict::Ask);
        assert!(f.rules.is_empty());
        // the domain: every URL of it, and the other questions about it go
        f.check("http://media.b.example/3", Source::PrimMedia);
        f.check("http://b.example/4", Source::PrimMedia);
        f.answer("http://b.example/4", false, Remember::Domain);
        assert!(f.questions.iter().all(|q| !q.domain.ends_with("b.example")));
        assert_eq!(
            f.rules,
            vec![Rule {
                pattern: "b.example".into(),
                action: Action::Deny
            }]
        );
        // the refusal right after the answer is not told twice
        let told = f.messages.len();
        assert_eq!(f.check("http://b.example/4", Source::PrimMedia), Verdict::Deny);
        assert_eq!(f.messages.len(), told);
        assert_eq!(f.check("http://b.example/4", Source::PrimMedia), Verdict::Deny);
        assert_eq!(f.messages.len(), told + 1);
        // the URL alone
        f.check("http://c.example/5", Source::Music);
        f.answer("http://c.example/5", true, Remember::Url);
        assert_eq!(f.check("http://c.example/6", Source::Music), Verdict::Ask);
        // a rule outweighs an earlier answer of the session
        f.apply(Edit::Set("a.example".into(), Action::Deny));
        assert_eq!(f.check("http://a.example/1", Source::ParcelMedia), Verdict::Deny);
    }

    #[test]
    fn trusted_media_is_never_asked_about_but_refusals_hold() {
        let mut f = MediaFilter::default();
        // no rule: loads, no question
        assert_eq!(f.check_trusted("http://a.example/1", Source::PrimMedia), Verdict::Allow);
        assert!(f.question().is_none());
        // the list's refusal, for the domain or the exact address
        f.apply(Edit::Set("b.example".into(), Action::Deny));
        f.apply(Edit::Set("http://c.example/x".into(), Action::Deny));
        assert_eq!(f.check_trusted("http://media.b.example/", Source::PrimMedia), Verdict::Deny);
        assert_eq!(f.check_trusted("http://c.example/x", Source::PrimMedia), Verdict::Deny);
        assert_eq!(f.messages.len(), 2, "the refusals are told");
        // a refusal of this session too
        f.check("http://d.example/", Source::PrimMedia);
        f.answer("http://d.example/", false, Remember::Once);
        assert_eq!(f.check_trusted("http://d.example/", Source::PrimMedia), Verdict::Deny);
        // the same site elsewhere is still asked about
        assert_eq!(f.check("http://a.example/1", Source::PrimMedia), Verdict::Ask);
    }

    #[test]
    fn questions_nothing_asks_for_go_away() {
        let mut f = MediaFilter::default();
        f.check("http://a.example/", Source::ParcelMedia);
        f.questions[0].last_seen = Instant::now() - STALE;
        assert!(f.question().is_none());
    }

    #[test]
    fn typed_entries_become_domains_unless_they_have_a_path() {
        assert_eq!(pattern_of(" Example.com ").as_deref(), Some("example.com"));
        assert_eq!(pattern_of("example.com/some/page").as_deref(), Some("example.com"));
        assert_eq!(pattern_of("https://example.com/").as_deref(), Some("example.com"));
        assert_eq!(
            pattern_of("https://example.com/live.mp3").as_deref(),
            Some("https://example.com/live.mp3")
        );
        assert_eq!(pattern_of(""), None);
        let mut f = MediaFilter::default();
        f.apply(Edit::Set("example.com".into(), Action::Allow));
        f.apply(Edit::Set("EXAMPLE.com".into(), Action::Deny));
        assert_eq!(f.rules.len(), 1);
        assert_eq!(f.rules[0].action, Action::Deny);
        f.apply(Edit::Remove("example.com".into()));
        assert!(f.rules.is_empty());
    }

    #[test]
    fn the_list_reads_and_writes_the_legacy_file() {
        let dir = std::env::temp_dir().join(format!("aurora-media-filter-{}", uuid::Uuid::new_v4()));
        let path = dir.join(FILE);
        std::fs::create_dir_all(&dir).expect("dir");
        // as the Legacy Viewer writes it
        std::fs::write(
            &path,
            "<?xml version=\"1.0\" ?>\n<llsd>\n<array>\n<map><key>action</key><string>allow</string><key>domain</key><string>example.com</string></map>\n<map><key>action</key><string>deny</string><key>domain</key><string>http://tracker.example/pixel</string></map>\n<map><key>action</key><string>maybe</string><key>domain</key><string>odd.example</string></map>\n</array>\n</llsd>\n",
        )
        .expect("write");
        let mut f = MediaFilter::load(path.clone());
        assert_eq!(f.rules.len(), 2);
        assert_eq!(f.check("http://www.example.com/", Source::ParcelMedia), Verdict::Allow);
        assert_eq!(f.check("http://tracker.example/pixel", Source::ParcelMedia), Verdict::Deny);
        f.apply(Edit::Set("radio.example".into(), Action::Deny));
        let again = MediaFilter::load(path);
        assert_eq!(again.rules, f.rules);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
