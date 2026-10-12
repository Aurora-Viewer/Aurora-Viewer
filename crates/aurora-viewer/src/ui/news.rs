//! Login news. Release checks are owned by the update controller so the login
//! panel and the modal always show the same result.
//!
//! The news are the hand-edited Markdown files of the repository's `news/`
//! folder, one per UI language (format: `news/markdown.rs`). The login screen
//! shows at once the last download kept in the cache, or else the files
//! compiled into the viewer; the current file of `main`, fetched in the
//! background, then replaces them. Any failure leaves the screen as it is.
mod markdown;

use anyhow::ensure;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// The panel scrolls, but older news are no longer news.
const MAX_ITEMS: usize = 20;
/// In characters; longer texts are cut with an ellipsis.
const MAX_TITLE: usize = 120;
const MAX_SUMMARY: usize = 600;
/// In bytes; a longer link is dropped.
const MAX_URL: usize = 512;
/// A file with every item at its limits and the editing instructions stays
/// well under this.
const MAX_DOWNLOAD: usize = 64 * 1024;
/// The cached JSON escapes what the Markdown file writes plainly.
const MAX_CACHE: u64 = 4 * MAX_DOWNLOAD as u64;
/// Language of the file shown when the UI language has none.
const FALLBACK: &str = "en";
/// The news files of the repository when this viewer was built.
const BUILTIN: &[(&str, &str)] = &[
    ("en", include_str!("../../../../news/en.md")),
    ("fr", include_str!("../../../../news/fr.md")),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewsItem {
    #[serde(default)]
    pub date: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub url: String,
}

impl NewsItem {
    /// Limits of every item, wherever it comes from (download, cache or
    /// built-in file). `None` without a title: there is nothing to show.
    fn bounded(self) -> Option<Self> {
        let title = clip(&self.title.replace('\n', " "), MAX_TITLE);
        if title.is_empty() {
            return None;
        }
        Some(Self {
            date: if markdown::is_date(&self.date) { self.date } else { String::new() },
            title,
            summary: clip(&self.summary, MAX_SUMMARY),
            url: if is_https(&self.url) { self.url } else { String::new() },
        })
    }
}

/// Printable text of at most `max` characters.
fn clip(text: &str, max: usize) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control() || *c == '\n').collect();
    let clean = clean.trim();
    if clean.chars().count() <= max {
        return clean.to_owned();
    }
    let mut cut: String = clean.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// The only links the login screen opens: plain `https://host/…` addresses.
fn is_https(url: &str) -> bool {
    url.len() <= MAX_URL
        && url.starts_with("https://")
        && !url.contains(|c: char| c.is_whitespace() || c.is_control())
        && reqwest::Url::parse(url).is_ok_and(|url| url.has_host() && url.username().is_empty() && url.password().is_none())
}

fn bounded(items: Vec<NewsItem>) -> Vec<NewsItem> {
    items.into_iter().filter_map(NewsItem::bounded).take(MAX_ITEMS).collect()
}

/// Files tried for a UI language: its own, its base language (`pt` for
/// `pt-BR`), then English.
fn candidates(language: &str) -> Vec<&str> {
    let mut codes = vec![language];
    for code in [language.split('-').next().unwrap_or(language), FALLBACK] {
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

fn builtin(language: &str) -> Vec<NewsItem> {
    candidates(language)
        .into_iter()
        .find_map(|code| {
            let (_, text) = BUILTIN.iter().find(|(builtin, _)| *builtin == code)?;
            Some(markdown::parse(text)).filter(|news| !news.is_empty())
        })
        .unwrap_or_default()
}

/// Where the news files are downloaded from.
#[derive(Clone)]
struct Source {
    /// Folder address, ending with `/`; it holds one `<language>.md` per language.
    base: String,
    /// Test server of this machine: plain HTTP, never through a proxy.
    loopback: bool,
    /// Folder keeping the downloads for the next start. `None` for a test
    /// source: an ordinary run must never show what a test downloaded.
    cache: Option<PathBuf>,
}

impl Source {
    /// `None` where the login screen stays offline: demo and updater test
    /// installation.
    fn from_environment() -> Option<Self> {
        if std::env::var_os("AURORA_DEMO").is_some() || aurora_updater::test_environment().is_some() {
            return None;
        }
        if let Ok(address) = std::env::var("AURORA_NEWS_URL") {
            if let Some(source) = Self::test(&address) {
                log::info!("Login news read from {} (AURORA_NEWS_URL), not kept in the cache", source.base);
                return Some(source);
            }
            log::warn!("AURORA_NEWS_URL ignored: neither a server of this machine nor a folder of the Aurora Viewer repository");
        }
        let base = crate::links::NEWS_FEED;
        (!base.is_empty()).then(|| Self {
            base: base.into(),
            loopback: false,
            cache: Some(crate::settings::cache_dir()),
        })
    }

    /// `AURORA_NEWS_URL`: a server of this machine, or another branch or
    /// folder of the project's repository (to read a news file before it is
    /// merged). Nothing else: the variable must not be a way to make the
    /// login screen show somebody else's text.
    fn test(address: &str) -> Option<Self> {
        let url = reqwest::Url::parse(address.trim()).ok()?;
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return None;
        }
        let loopback = url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        // The parsed address has no `..` left to climb out of the repository.
        let mut base = String::from(url);
        if !loopback && !base.starts_with(crate::links::RAW_FILES) {
            return None;
        }
        if !base.ends_with('/') {
            base.push('/');
        }
        Some(Self {
            base,
            loopback,
            cache: None,
        })
    }

    async fn fetch(&self, language: &str) -> anyhow::Result<Vec<NewsItem>> {
        let builder = reqwest::Client::builder()
            .user_agent(aurora_version::user_agent())
            .https_only(!self.loopback)
            .timeout(Duration::from_secs(8));
        let http = if self.loopback { builder.no_proxy() } else { builder }.build()?;
        for code in candidates(language) {
            if let Some(text) = download(&http, &format!("{}{code}.md", self.base)).await? {
                let news = markdown::parse(&text);
                ensure!(!news.is_empty(), "no news item in {code}.md");
                return Ok(news);
            }
        }
        anyhow::bail!("no news file")
    }

    fn cache_file(&self, language: &str) -> Option<PathBuf> {
        Some(self.cache.as_ref()?.join(format!("news-{language}.json")))
    }

    /// Last download for this language. The file was written by the viewer,
    /// but it is a file all the same: read with the limits of a download.
    fn cached(&self, language: &str) -> Option<Vec<NewsItem>> {
        let path = self.cache_file(language)?;
        if std::fs::metadata(&path).ok()?.len() > MAX_CACHE {
            return None;
        }
        let news = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        Some(bounded(news)).filter(|news| !news.is_empty())
    }

    fn store(&self, language: &str, news: &[NewsItem]) {
        let Some(path) = self.cache_file(language) else { return };
        let written = serde_json::to_vec(news).map_err(std::io::Error::from).and_then(|bytes| {
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder)?;
            }
            std::fs::write(&path, bytes)
        });
        if let Err(e) = written {
            log::warn!("Login news not kept for the next start: {e}");
        }
    }
}

/// `Ok(None)`: no such file, this language has no news yet.
async fn download(http: &reqwest::Client, url: &str) -> anyhow::Result<Option<String>> {
    let response = http.get(url).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut response = response.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_DOWNLOAD,
            "news file larger than {MAX_DOWNLOAD} bytes"
        );
        bytes.extend_from_slice(&chunk);
    }
    // A file saved in another encoding still shows its unaccented text.
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

#[derive(Debug, Clone, PartialEq)]
pub enum VersionState {
    NotConfigured,
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Failed,
}

pub struct LoginInfo {
    pub news: Vec<NewsItem>,
    pub version: VersionState,
    pub can_check_updates: bool,
    /// UI language of `news`: they follow a change made in the preferences.
    language: &'static str,
    /// `None` where nothing is downloaded: demo, updater test, previews.
    feed: Option<Feed>,
}

/// Background downloads; each reply carries the language it was asked for.
struct Feed {
    source: Source,
    rt: tokio::runtime::Handle,
    tx: crossbeam_channel::Sender<(&'static str, Vec<NewsItem>)>,
    rx: crossbeam_channel::Receiver<(&'static str, Vec<NewsItem>)>,
}

impl Feed {
    fn request(&self, language: &'static str) {
        let (source, tx) = (self.source.clone(), self.tx.clone());
        self.rt.spawn(async move {
            match source.fetch(language).await {
                Ok(news) => {
                    source.store(language, &news);
                    let _ = tx.send((language, news));
                }
                // Being offline is ordinary: the login screen keeps what it shows.
                Err(e) => log::warn!("Login news not updated ({language}): {e:#}"),
            }
        });
    }
}

impl LoginInfo {
    pub fn preview_release(version: &str) -> Self {
        let language = aurora_i18n::language();
        Self {
            news: builtin(language),
            version: VersionState::Available {
                version: version.into(),
                url: String::new(),
            },
            can_check_updates: true,
            language,
            feed: None,
        }
    }

    pub fn start(rt: &tokio::runtime::Handle) -> Self {
        Self::with_source(rt, Source::from_environment(), aurora_i18n::language())
    }

    fn with_source(rt: &tokio::runtime::Handle, source: Option<Source>, language: &'static str) -> Self {
        let feed = source.map(|source| {
            let (tx, rx) = crossbeam_channel::unbounded();
            Feed {
                source,
                rt: rt.clone(),
                tx,
                rx,
            }
        });
        let mut info = Self {
            news: Vec::new(),
            version: VersionState::NotConfigured,
            can_check_updates: false,
            language,
            feed,
        };
        info.load();
        info
    }

    /// Shows what this machine has for the current language, then asks for
    /// the published file.
    fn load(&mut self) {
        let language = self.language;
        self.news = self
            .feed
            .as_ref()
            .and_then(|feed| feed.source.cached(language))
            .unwrap_or_else(|| builtin(language));
        if let Some(feed) = &self.feed {
            feed.request(language);
        }
    }

    pub fn poll(&mut self) {
        self.poll_in(aurora_i18n::language());
    }

    fn poll_in(&mut self, language: &'static str) {
        if language != self.language {
            self.language = language;
            self.load();
        }
        let Some(feed) = &self.feed else { return };
        while let Ok((language, news)) = feed.rx.try_recv() {
            // A reply for the language the user just left only fills its cache.
            if language == self.language {
                self.news = news;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Loopback server answering `routes` (path, body), 404 for the rest.
    fn serve(routes: Vec<(&'static str, String)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback port");
        let base = format!("http://{}/", listener.local_addr().expect("address"));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut request = [0; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]);
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let (status, body) = routes
                    .iter()
                    .find(|(route, _)| *route == path)
                    .map_or(("404 Not Found", ""), |(_, body)| ("200 OK", body.as_str()));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        base
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime")
    }

    fn source(base: String, cache: Option<PathBuf>) -> Source {
        Source {
            base,
            loopback: true,
            cache,
        }
    }

    fn titles(news: &[NewsItem]) -> Vec<&str> {
        news.iter().map(|item| item.title.as_str()).collect()
    }

    /// Lets the background download run until `done`.
    fn wait(rt: &tokio::runtime::Runtime, info: &mut LoginInfo, language: &'static str, done: impl Fn(&LoginInfo) -> bool) {
        rt.block_on(async {
            for _ in 0..1000 {
                info.poll_in(language);
                if done(info) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("no reply from the loopback server");
        });
    }

    #[test]
    fn builtin_news_in_each_language() {
        for (code, text) in BUILTIN {
            let news = markdown::parse(text);
            assert!(!news.is_empty(), "news/{code}.md has no news item");
            assert_eq!(builtin(code), news);
            // A mistake of the editor must not go unnoticed: every heading
            // written after the instructions gives an item, and no text is cut.
            let headings = text
                .rsplit("-->")
                .next()
                .unwrap_or(text)
                .lines()
                .filter(|line| line.starts_with("## "))
                .count();
            assert_eq!(news.len(), headings.min(MAX_ITEMS), "news/{code}.md: a heading has no title");
            for item in &news {
                assert!(
                    item.title.chars().count() < MAX_TITLE,
                    "news/{code}.md: title too long: {}",
                    item.title
                );
                assert!(
                    item.summary.chars().count() < MAX_SUMMARY,
                    "news/{code}.md: text too long: {}",
                    item.title
                );
            }
        }
        assert_ne!(builtin("en"), builtin("fr"));
        // A language without a file shows English.
        assert_eq!(builtin("de"), builtin("en"));
        assert_eq!(builtin("fr-CA"), builtin("fr"));
        assert_eq!(candidates("pt-BR"), ["pt-BR", "pt", "en"]);
        assert_eq!(candidates("en"), ["en"]);
    }

    #[test]
    fn items_from_the_cache_get_the_same_limits() {
        let item = |title: &str, url: &str| NewsItem {
            date: "yesterday".into(),
            title: title.into(),
            summary: "x".repeat(5000),
            url: url.into(),
        };
        let news = bounded(
            std::iter::once(item(" ", "https://example.org/"))
                .chain((0..100).map(|i| item(&format!("Title\n{i}"), "http://example.org/")))
                .collect(),
        );
        assert_eq!(news.len(), MAX_ITEMS);
        assert_eq!(news[0].title, "Title 0");
        assert!(news.iter().all(|item| item.date.is_empty() && item.url.is_empty()));
        assert_eq!(news[0].summary.chars().count(), MAX_SUMMARY);
        assert_eq!(
            item("Kept", "https://example.org/a").bounded().map(|item| item.url).as_deref(),
            Some("https://example.org/a")
        );
    }

    #[test]
    fn test_sources_stay_on_this_machine_or_in_the_repository() {
        for (address, base, loopback) in [
            ("http://127.0.0.1:8123", "http://127.0.0.1:8123/", true),
            ("http://localhost:8123/news/", "http://localhost:8123/news/", true),
            ("http://[::1]:8123/news", "http://[::1]:8123/news/", true),
            (
                " https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/feat/actualites-md/news ",
                "https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/feat/actualites-md/news/",
                false,
            ),
        ] {
            let source = Source::test(address).unwrap_or_else(|| panic!("{address}"));
            assert_eq!((source.base.as_str(), source.loopback), (base, loopback));
            assert!(source.cache.is_none());
        }
        for address in [
            "",
            "news",
            "http://example.org/news/",
            "https://example.org/news/",
            "https://127.0.0.1/news/",
            "http://127.0.0.1.example.org/",
            "http://user:secret@127.0.0.1/",
            "http://127.0.0.1/?next=https://example.org",
            "http://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/news/",
            "https://raw.githubusercontent.com/Somebody/Aurora-Viewer/main/news/",
            "https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/../../Somebody/Else/main/",
            "https://raw.githubusercontent.com.example.org/Aurora-Viewer/Aurora-Viewer/main/news/",
            "file:///C:/news/",
        ] {
            assert!(Source::test(address).is_none(), "{address}");
        }
        assert!(crate::links::NEWS_FEED.starts_with(crate::links::RAW_FILES) && crate::links::NEWS_FEED.ends_with('/'));
    }

    #[test]
    fn missing_language_falls_back_and_bad_files_are_errors() {
        let rt = runtime();
        let english = "## 2026-11-02 — Published\nText.\n".to_owned();
        let fetch = |routes, language| rt.block_on(source(serve(routes), None).fetch(language));

        let news = fetch(vec![("/en.md", english.clone()), ("/fr.md", "## Publié\r\n".into())], "fr").expect("French file");
        assert_eq!(titles(&news), ["Publié"]);
        let news = fetch(vec![("/en.md", english.clone())], "fr").expect("English fallback");
        assert_eq!(titles(&news), ["Published"]);
        let news = fetch(vec![("/en.md", english.clone()), ("/pt.md", "## Publicado".into())], "pt-BR").expect("base language");
        assert_eq!(titles(&news), ["Publicado"]);

        assert!(fetch(vec![], "fr").is_err(), "no file at all");
        // The language has a file: its mistakes are not hidden behind English.
        assert!(
            fetch(vec![("/en.md", english.clone()), ("/fr.md", "Rien.".into())], "fr").is_err(),
            "no item"
        );
        let huge = format!("{english}{}", "x".repeat(MAX_DOWNLOAD));
        assert!(fetch(vec![("/fr.md", huge)], "fr").is_err(), "too large");
        // Nothing listens on a port that was just released.
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback port");
            format!("http://{}/", listener.local_addr().expect("address"))
        };
        assert!(rt.block_on(source(closed, None).fetch("fr")).is_err(), "offline");
    }

    #[test]
    fn download_replaces_the_news_and_is_kept_for_the_next_start() {
        let rt = runtime();
        let cache = std::env::temp_dir().join(format!("aurora-news-test-{}", uuid::Uuid::new_v4()));
        let base = serve(vec![
            ("/fr.md", "## 2026-11-02 — Publié\nTexte.\n[Suite](https://example.org/fr)\n".into()),
            ("/en.md", "## 2026-11-02 — Published\nText.\n".into()),
        ]);

        // First start: the built-in file, then the published one.
        let mut info = LoginInfo::with_source(rt.handle(), Some(source(base.clone(), Some(cache.clone()))), "fr");
        assert_eq!(info.news, builtin("fr"));
        wait(&rt, &mut info, "fr", |info| titles(&info.news) == ["Publié"]);
        assert_eq!(info.news[0].url, "https://example.org/fr");

        // The language changes in the preferences: built-in English at once, then the download.
        info.poll_in("en");
        assert_eq!(info.news, builtin("en"));
        wait(&rt, &mut info, "en", |info| titles(&info.news) == ["Published"]);

        // Next start without a network: the last download of each language.
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback port");
            format!("http://{}/", listener.local_addr().expect("address"))
        };
        let mut offline = LoginInfo::with_source(rt.handle(), Some(source(closed.clone(), Some(cache.clone()))), "fr");
        assert_eq!(titles(&offline.news), ["Publié"]);
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(200)).await });
        offline.poll_in("fr");
        assert_eq!(titles(&offline.news), ["Publié"]);
        offline.poll_in("en");
        assert_eq!(titles(&offline.news), ["Published"]);

        // A damaged cache file is ignored, and a test source has no cache.
        std::fs::write(cache.join("news-fr.json"), b"[{\"title\": 3}").expect("cache file");
        assert_eq!(
            LoginInfo::with_source(rt.handle(), Some(source(closed.clone(), Some(cache.clone()))), "fr").news,
            builtin("fr")
        );
        std::fs::write(cache.join("news-fr.json"), vec![b' '; MAX_CACHE as usize + 1]).expect("cache file");
        assert_eq!(
            LoginInfo::with_source(rt.handle(), Some(source(closed, Some(cache.clone()))), "fr").news,
            builtin("fr")
        );
        let mut preview = LoginInfo::with_source(rt.handle(), Some(source(base, None)), "en");
        assert_eq!(preview.news, builtin("en"));
        wait(&rt, &mut preview, "en", |info| titles(&info.news) == ["Published"]);

        // Nothing is downloaded without a source (demo, updater test).
        let mut demo = LoginInfo::with_source(rt.handle(), None, "fr");
        demo.poll_in("en");
        assert_eq!(demo.news, builtin("en"));
        let _ = std::fs::remove_dir_all(cache);
    }
}
