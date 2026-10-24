//! The GIF search of the Conversations field's GIF picker (#932): GIPHY and
//! KLIPY, their addresses, their answers, the small animations of the
//! picker's tiles. The picker itself is `ui/chat/gif_picker.rs`.
//!
//! An Aurora feature: the Legacy Viewer has no GIF search. What a GIF sends
//! is its link, a normal chat message: other viewers show the link, and
//! Aurora's link previews (`link_preview.rs`) play it.
//!
//! Both services ask, in their terms of use:
//! * the requests leave the user's own application, never a proxy or a
//!   server of ours (there is none);
//! * one key per application, a test key limited to about 100 requests an
//!   hour until the service grants a production key;
//! * their results shown as they come, in their order, never mixed with
//!   another service's in one grid: the picker has a tab per service;
//! * their media loaded from the addresses they give, unchanged, and not
//!   kept: the favourites keep the GIF's id, and ask the service again;
//! * their attribution: "Powered by GIPHY"; "Search KLIPY" as the search
//!   field's placeholder.
//!
//! Each user brings their own key (Preferences › Chat › GIFs): a key built
//! into an open viewer would be shared by everyone, and a free key allows
//! about 100 requests an hour in all. The button is hidden by default; a
//! key entered shows it, unless the user hid it themselves.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[cfg(test)]
mod tests;

/// A GIF service.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Provider {
    #[default]
    Giphy,
    Klipy,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Giphy, Provider::Klipy];

    /// The service's name, as its brand writes it.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Giphy => "GIPHY",
            Provider::Klipy => "KLIPY",
        }
    }

    /// The domain of its media servers: a picture from anywhere else is
    /// not loaded.
    fn media_domain(self) -> &'static str {
        match self {
            Provider::Giphy => "giphy.com",
            Provider::Klipy => "klipy.com",
        }
    }
}

/// How strict the content filter is (GIPHY's `rating`, KLIPY's
/// `content_filter`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Rating {
    /// GIPHY "g", KLIPY "high".
    Strict,
    /// GIPHY "pg-13", KLIPY "medium".
    #[default]
    Moderate,
    /// GIPHY "r", KLIPY "low".
    Relaxed,
}

impl Rating {
    fn giphy(self) -> &'static str {
        match self {
            Rating::Strict => "g",
            Rating::Moderate => "pg-13",
            Rating::Relaxed => "r",
        }
    }

    fn klipy(self) -> &'static str {
        match self {
            Rating::Strict => "high",
            Rating::Moderate => "medium",
            Rating::Relaxed => "low",
        }
    }
}

/// A GIF kept as a favourite: its service and its id (GIPHY's id, KLIPY's
/// slug), never its media.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Favorite {
    pub provider: Provider,
    pub id: String,
}

/// Most favourites kept, per service: one request fetches them all.
pub const MAX_FAVORITES: usize = 48;

/// The GIF picker of Preferences › Chat.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// The GIF button of the Conversations field (off by default; shown
    /// only with a key).
    pub show_button: bool,
    /// The user turned the button off themselves: a key entered later no
    /// longer turns it on.
    pub hidden_by_user: bool,
    /// The service of the picker's tab, the last one used.
    pub provider: Provider,
    pub rating: Rating,
    /// The user's own keys, kept in the settings file in plain text.
    pub giphy_key: String,
    pub klipy_key: String,
    /// A random id, the same for every search of this viewer: both
    /// services ask for one to count what is sent. Nothing of the account.
    pub customer_id: String,
    pub favorites: Vec<Favorite>,
}

/// A key that is no key: empty, or not made of URL-safe characters.
fn usable(key: &str) -> Option<String> {
    let key = key.trim();
    (!key.is_empty() && key.len() <= 128 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .then(|| key.to_owned())
}

impl Settings {
    /// The user's key of `provider`.
    pub fn key(&self, provider: Provider) -> Option<String> {
        usable(match provider {
            Provider::Giphy => &self.giphy_key,
            Provider::Klipy => &self.klipy_key,
        })
    }

    /// The services with a key, in the tabs' order.
    pub fn providers(&self) -> Vec<Provider> {
        Provider::ALL.into_iter().filter(|p| self.key(*p).is_some()).collect()
    }

    /// The GIF button shows: turned on, and a service has a key.
    pub fn usable(&self) -> bool {
        self.show_button && !self.providers().is_empty()
    }

    /// A key was typed: a usable one shows the button, unless the user hid
    /// it themselves.
    pub fn key_changed(&mut self) {
        if !self.hidden_by_user && !self.providers().is_empty() {
            self.show_button = true;
        }
    }

    /// The user hides the button (its right-click menu, or Preferences).
    pub fn hide(&mut self) {
        self.show_button = false;
        self.hidden_by_user = true;
    }

    /// The random id, made on first use. True when it was just made (the
    /// settings are saved).
    pub fn ensure_customer_id(&mut self) -> bool {
        if self.customer_id.is_empty() {
            self.customer_id = uuid::Uuid::new_v4().simple().to_string();
            return true;
        }
        false
    }

    pub fn is_favorite(&self, provider: Provider, id: &str) -> bool {
        self.favorites.iter().any(|f| f.provider == provider && f.id == id)
    }

    /// Add or remove a favourite; the newest first, `MAX_FAVORITES` per
    /// service at most.
    pub fn toggle_favorite(&mut self, provider: Provider, id: &str) {
        if self.is_favorite(provider, id) {
            self.favorites.retain(|f| !(f.provider == provider && f.id == id));
            return;
        }
        self.favorites.insert(
            0,
            Favorite {
                provider,
                id: id.to_owned(),
            },
        );
        let mut kept = 0;
        self.favorites.retain(|f| {
            if f.provider != provider {
                return true;
            }
            kept += 1;
            kept <= MAX_FAVORITES
        });
    }

    /// The ids of `provider`'s favourites, newest first.
    pub fn favorite_ids(&self, provider: Provider) -> Vec<String> {
        self.favorites
            .iter()
            .filter(|f| f.provider == provider)
            .map(|f| f.id.clone())
            .collect()
    }
}

/// What a request asks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Query {
    Trending,
    Search(String),
    /// Ids, newest first.
    Favorites(Vec<String>),
}

/// GIFs per page: GIPHY's test keys allow 50 at most, KLIPY 8 to 50.
pub const PER_PAGE: u32 = 24;
/// Longest search GIPHY takes, in characters.
const GIPHY_MAX_QUERY: usize = 50;
/// GIPHY's last offset: its trending stops at 499, its search at 4999.
const GIPHY_TRENDING_END: u32 = 499;
const GIPHY_SEARCH_END: u32 = 4999;

/// The two letters of a language code ("fr", "pt-BR" → "pt").
fn language(code: &str) -> Option<String> {
    let two: String = code.chars().take(2).collect::<String>().to_ascii_lowercase();
    (two.len() == 2 && two.bytes().all(|b| b.is_ascii_lowercase())).then_some(two)
}

/// The address of a page of results. `page`: GIPHY's offset, KLIPY's page
/// number (from 1); `language`: the interface's.
pub fn page_url(provider: Provider, key: &str, query: &Query, page: u32, rating: Rating, language_code: &str, customer: &str) -> String {
    match provider {
        Provider::Giphy => {
            let (path, max_end) = match query {
                Query::Trending => ("trending", Some(GIPHY_TRENDING_END)),
                Query::Search(_) => ("search", Some(GIPHY_SEARCH_END)),
                Query::Favorites(_) => ("", None),
            };
            let mut url = reqwest::Url::parse("https://api.giphy.com/v1/gifs").expect("a fixed address");
            if !path.is_empty()
                && let Ok(mut segments) = url.path_segments_mut()
            {
                segments.push(path);
            }
            {
                let mut q = url.query_pairs_mut();
                q.append_pair("api_key", key);
                match query {
                    Query::Favorites(ids) => {
                        q.append_pair("ids", &ids.join(","));
                    }
                    Query::Search(text) => {
                        let text: String = text.trim().chars().take(GIPHY_MAX_QUERY).collect();
                        q.append_pair("q", &text);
                    }
                    Query::Trending => {}
                }
                if !matches!(query, Query::Favorites(_)) {
                    let offset = page.min(max_end.unwrap_or(0));
                    q.append_pair("limit", &PER_PAGE.to_string());
                    q.append_pair("offset", &offset.to_string());
                    q.append_pair("rating", rating.giphy());
                    if matches!(query, Query::Search(_))
                        && let Some(lang) = language(language_code)
                    {
                        q.append_pair("lang", &lang);
                    }
                    // the renditions of a messaging app, without clips
                    q.append_pair("bundle", "messaging_non_clips");
                }
                if !customer.is_empty() {
                    q.append_pair("customer_id", customer);
                }
            }
            url.into()
        }
        Provider::Klipy => {
            let mut url = reqwest::Url::parse("https://api.klipy.com").expect("a fixed address");
            if let Ok(mut segments) = url.path_segments_mut() {
                segments.extend(["api", "v1", key, "gifs"]);
                segments.push(match query {
                    Query::Trending => "trending",
                    Query::Search(_) => "search",
                    Query::Favorites(_) => "items",
                });
            }
            {
                let mut q = url.query_pairs_mut();
                match query {
                    Query::Favorites(ids) => {
                        q.append_pair("slugs", &ids.join(","));
                    }
                    Query::Search(text) => {
                        q.append_pair("q", text.trim());
                    }
                    Query::Trending => {}
                }
                if !matches!(query, Query::Favorites(_)) {
                    q.append_pair("page", &page.max(1).to_string());
                    q.append_pair("per_page", &PER_PAGE.to_string());
                    q.append_pair("content_filter", rating.klipy());
                    // the picker plays GIFs only
                    q.append_pair("format_filter", "gif");
                    if !customer.is_empty() {
                        q.append_pair("customer_id", customer);
                    }
                }
            }
            url.into()
        }
    }
}

/// The first page's number: GIPHY's offset 0, KLIPY's page 1.
pub fn first_page(provider: Provider) -> u32 {
    match provider {
        Provider::Giphy => 0,
        Provider::Klipy => 1,
    }
}

/// The address of a service's categories.
pub fn categories_url(provider: Provider, key: &str) -> String {
    match provider {
        Provider::Giphy => {
            let mut url = reqwest::Url::parse("https://api.giphy.com/v1/gifs/categories").expect("a fixed address");
            url.query_pairs_mut().append_pair("api_key", key);
            url.into()
        }
        Provider::Klipy => {
            let mut url = reqwest::Url::parse("https://api.klipy.com").expect("a fixed address");
            if let Ok(mut segments) = url.path_segments_mut() {
                segments.extend(["api", "v1", key, "gifs", "categories"]);
            }
            url.into()
        }
    }
}

/// The links GIPHY asks to be called when a GIF is shown, clicked and
/// sent (its "analytics" object).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pings {
    pub onload: Option<String>,
    pub onclick: Option<String>,
    pub onsent: Option<String>,
}

/// One GIF of a page of results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gif {
    pub provider: Provider,
    /// GIPHY's id, KLIPY's slug.
    pub id: String,
    pub title: String,
    /// The small animation of its tile.
    pub thumb: String,
    /// Its size, in pixels.
    pub thumb_size: [u32; 2],
    /// The link sent in the chat.
    pub send: String,
    pub pings: Pings,
}

/// A page of results.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub gifs: Vec<Gif>,
    /// The next page's number, when there is one.
    pub next: Option<u32>,
}

/// A category: what it shows and what it searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub label: String,
    pub query: String,
}

/// Why a request gave nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The service refused the key (401, 403).
    Key,
    /// Too many requests: a test key allows about 100 an hour.
    RateLimited,
    /// No answer, an error, an answer that is not one.
    Unavailable,
}

/// A number GIPHY may write as a string ("100").
fn number(v: &serde_json::Value) -> u32 {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .map_or(0, |n| n.min(u32::MAX as u64) as u32)
}

/// An https address on `provider`'s media servers.
fn media_ok(provider: Provider, url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let domain = provider.media_domain();
    parsed.scheme() == "https"
        && parsed
            .host_str()
            .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
            .is_some_and(|h| h == domain || h.ends_with(&format!(".{domain}")))
}

/// A short text: titles are shown in a tooltip.
fn short(s: &str) -> String {
    s.chars().take(120).collect()
}

/// A GIPHY answer (search, trending, or by ids).
pub fn parse_giphy(body: &[u8]) -> Result<Page, Error> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::Unavailable)?;
    let data = v["data"].as_array().ok_or(Error::Unavailable)?;
    let gifs = data
        .iter()
        .filter_map(|g| {
            let images = &g["images"];
            let thumb = &images["fixed_width_small"];
            let thumb_url = thumb["url"].as_str()?;
            // a link of at most 2 MB, else the original
            let send = images["downsized"]["url"]
                .as_str()
                .filter(|u| !u.is_empty())
                .or_else(|| images["original"]["url"].as_str())?;
            if !media_ok(Provider::Giphy, thumb_url) || !media_ok(Provider::Giphy, send) {
                return None;
            }
            let ping = |name: &str| {
                g["analytics"][name]["url"]
                    .as_str()
                    .filter(|u| u.starts_with("https://"))
                    .map(str::to_owned)
            };
            Some(Gif {
                provider: Provider::Giphy,
                id: g["id"].as_str()?.to_owned(),
                title: short(g["title"].as_str().unwrap_or("")),
                thumb: thumb_url.to_owned(),
                thumb_size: [number(&thumb["width"]), number(&thumb["height"])],
                send: send.to_owned(),
                pings: Pings {
                    onload: ping("onload"),
                    onclick: ping("onclick"),
                    onsent: ping("onsent"),
                },
            })
        })
        .collect();
    let pagination = &v["pagination"];
    let next = (!pagination.is_null()).then(|| {
        let (offset, count, total) = (
            number(&pagination["offset"]),
            number(&pagination["count"]),
            number(&pagination["total_count"]),
        );
        let next = offset.saturating_add(count);
        (count > 0 && next < total).then_some(next)
    });
    Ok(Page {
        gifs,
        next: next.flatten(),
    })
}

/// A KLIPY answer (search, trending, or items).
pub fn parse_klipy(body: &[u8]) -> Result<Page, Error> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::Unavailable)?;
    if v["result"].as_bool() == Some(false) {
        return Err(Error::Unavailable);
    }
    let data = &v["data"];
    let items = data["data"].as_array().or_else(|| data.as_array()).ok_or(Error::Unavailable)?;
    let gifs = items
        .iter()
        .filter_map(|g| {
            let file = &g["file"];
            let rendition = |size: &str| {
                let r = &file[size]["gif"];
                r["url"].as_str().map(|u| (u, [number(&r["width"]), number(&r["height"])]))
            };
            let (thumb, thumb_size) = rendition("xs").or_else(|| rendition("sm"))?;
            // a link of about 2 MB, else the largest
            let (send, _) = rendition("md").or_else(|| rendition("hd")).or_else(|| rendition("sm"))?;
            if !media_ok(Provider::Klipy, thumb) || !media_ok(Provider::Klipy, send) {
                return None;
            }
            Some(Gif {
                provider: Provider::Klipy,
                id: g["slug"].as_str()?.to_owned(),
                title: short(g["title"].as_str().unwrap_or("")),
                thumb: thumb.to_owned(),
                thumb_size,
                send: send.to_owned(),
                pings: Pings::default(),
            })
        })
        .collect();
    let next = (data["has_next"].as_bool() == Some(true)).then(|| number(&data["current_page"]).max(1) + 1);
    Ok(Page { gifs, next })
}

/// A service's categories.
pub fn parse_categories(provider: Provider, body: &[u8]) -> Vec<Category> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let list = match provider {
        Provider::Giphy => v["data"].as_array(),
        Provider::Klipy => v["data"]["categories"].as_array(),
    };
    list.into_iter()
        .flatten()
        .filter_map(|c| {
            let (label, query) = match provider {
                Provider::Giphy => (c["name"].as_str()?, c["name"].as_str()?),
                Provider::Klipy => (c["category"].as_str()?, c["query"].as_str().or_else(|| c["category"].as_str())?),
            };
            let label = label.trim();
            (!label.is_empty()).then(|| Category {
                label: short(label),
                query: query.trim().to_owned(),
            })
        })
        .take(30)
        .collect()
}

/// Largest answer of a service, and largest tile animation.
const MAX_ANSWER: u64 = 1024 * 1024;
const MAX_THUMB: u64 = 1536 * 1024;

/// The client the picker asks with: no cookie, no redirect off the
/// services.
pub fn client() -> Option<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let ok = Provider::ALL.iter().any(|p| media_ok(*p, attempt.url().as_str()))
            || attempt
                .url()
                .host_str()
                .is_some_and(|h| h == "api.giphy.com" || h == "api.klipy.com");
        if attempt.previous().len() >= 3 || !ok {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .user_agent(concat!("AuroraViewer/", env!("CARGO_PKG_VERSION"), " (GIF search)"))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .retry(reqwest::retry::never())
        .redirect(policy)
        .build()
        .map_err(|e| log::warn!("GIF search: no HTTP client: {e}"))
        .ok()
}

/// The body of `url`, at most `max` bytes.
async fn download(client: &reqwest::Client, url: &str, max: u64) -> Result<Vec<u8>, Error> {
    let mut resp = client.get(url).send().await.map_err(|e| {
        log::debug!("GIF search: {}: {e}", without_key(url));
        Error::Unavailable
    })?;
    match resp.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err(Error::Key),
        429 => return Err(Error::RateLimited),
        status => {
            log::debug!("GIF search: {}: {status}", without_key(url));
            return Err(Error::Unavailable);
        }
    }
    if resp.content_length().is_some_and(|len| len > max) {
        return Err(Error::Unavailable);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| Error::Unavailable)? {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > max {
            return Err(Error::Unavailable);
        }
    }
    Ok(body)
}

/// An address for the log: the key never goes there.
fn without_key(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return String::new();
    };
    parsed.set_query(None);
    let host = parsed.host_str().unwrap_or("");
    if host == "api.klipy.com" {
        // the key is the path's third segment
        let segments: Vec<String> = parsed
            .path_segments()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, s)| if i == 2 { "…".to_owned() } else { s.to_owned() })
            .collect();
        parsed.set_path(&segments.join("/"));
    }
    parsed.into()
}

/// A page of results.
pub async fn fetch_page(client: &reqwest::Client, provider: Provider, url: &str) -> Result<Page, Error> {
    let body = download(client, url, MAX_ANSWER).await?;
    match provider {
        Provider::Giphy => parse_giphy(&body),
        Provider::Klipy => parse_klipy(&body),
    }
}

/// A service's categories (none when it does not answer).
pub async fn fetch_categories(client: &reqwest::Client, provider: Provider, url: &str) -> Vec<Category> {
    match download(client, url, MAX_ANSWER).await {
        Ok(body) => parse_categories(provider, &body),
        Err(_) => Vec::new(),
    }
}

/// Side, frames and memory of a tile's animation: a tile is about 110
/// points wide, twice that for dense screens.
const THUMB_SIDE: u32 = 220;
const THUMB_FRAMES: usize = 60;
const THUMB_BUDGET: usize = 6 * 1024 * 1024;

/// A tile's animation, decoded off the frame.
pub async fn fetch_thumb(client: &reqwest::Client, provider: Provider, url: &str) -> Result<crate::link_preview::Picture, Error> {
    if !media_ok(provider, url) {
        return Err(Error::Unavailable);
    }
    let body = download(client, url, MAX_THUMB).await?;
    tokio::task::spawn_blocking(move || crate::link_preview::decode_animation(&body, THUMB_SIDE, THUMB_FRAMES, THUMB_BUDGET))
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::Unavailable)
}

/// A GIPHY analytics link, with the time and the id it asks for.
pub fn ping_url(url: &str, customer: &str, now_ms: u128) -> Option<String> {
    let mut parsed = reqwest::Url::parse(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    parsed.query_pairs_mut().append_pair("ts", &now_ms.to_string());
    if !customer.is_empty() {
        parsed.query_pairs_mut().append_pair("customer_id", customer);
    }
    Some(parsed.into())
}

/// Call a GIPHY analytics link (its answer is not used).
pub async fn ping(client: &reqwest::Client, url: String) {
    if let Err(e) = client.get(&url).send().await {
        log::debug!("GIF search: analytics: {e}");
    }
}

/// The address KLIPY counts a sent GIF at, and its body.
pub fn klipy_share(key: &str, slug: &str, customer: &str, query: Option<&str>) -> (String, String) {
    let mut url = reqwest::Url::parse("https://api.klipy.com").expect("a fixed address");
    if let Ok(mut segments) = url.path_segments_mut() {
        segments.extend(["api", "v1", key, "gifs", "share", slug]);
    }
    let mut body = serde_json::json!({ "customer_id": customer });
    if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
        body["q"] = serde_json::Value::String(q.trim().to_owned());
    }
    (url.into(), body.to_string())
}

/// Tell KLIPY a GIF was sent (its answer is not used).
pub async fn share(client: &reqwest::Client, url: String, body: String) {
    let sent = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;
    if let Err(e) = sent {
        log::debug!("GIF search: share: {e}");
    }
}

/// Demo mode: answers made here, without network or key. The GIFs are
/// drawn shapes going round, as many as a page.
pub mod demo {
    use super::*;

    /// Made-up media addresses: their names say what to draw.
    fn media(provider: Provider, n: u32, size: &str) -> String {
        match provider {
            Provider::Giphy => format!("https://media.giphy.com/media/demo{n}/{size}.gif"),
            Provider::Klipy => format!("https://static.klipy.com/ii/demo/{n}/{size}.gif"),
        }
    }

    /// A page of `query` (two pages at most, then none).
    pub fn page(provider: Provider, query: &Query, page: u32) -> Result<Page, Error> {
        let index = page.saturating_sub(first_page(provider)) / if provider == Provider::Giphy { PER_PAGE } else { 1 };
        let seed = match query {
            Query::Trending => 0,
            Query::Search(q) => q.bytes().map(u32::from).sum::<u32>(),
            Query::Favorites(_) => 7,
        };
        let count = match query {
            Query::Favorites(ids) => ids.len() as u32,
            Query::Search(q) if q.trim().eq_ignore_ascii_case("none") => 0,
            _ => PER_PAGE,
        };
        let gifs = (0..count)
            .map(|i| {
                let n = seed.wrapping_add(index * PER_PAGE + i);
                let id = match query {
                    Query::Favorites(ids) => ids[i as usize].clone(),
                    _ => format!("demo-{}-{n}", provider.name().to_lowercase()),
                };
                Gif {
                    provider,
                    id,
                    title: format!("Aurora demo {}", n % 100),
                    thumb: media(provider, n, "small"),
                    thumb_size: [100, 75 + (n % 3) * 15],
                    send: media(provider, n, "send"),
                    pings: Pings::default(),
                }
            })
            .collect();
        let second = match provider {
            Provider::Giphy => PER_PAGE,
            Provider::Klipy => 2,
        };
        let next = (index == 0 && count == PER_PAGE).then_some(second);
        Ok(Page { gifs, next })
    }

    pub fn categories() -> Vec<Category> {
        ["Hello", "Thank you", "Dance", "Love", "Applause", "Sleepy"]
            .iter()
            .map(|c| Category {
                label: (*c).to_owned(),
                query: c.to_lowercase(),
            })
            .collect()
    }

    /// The tile of a demo address: dots going round, their colours from
    /// the address.
    pub fn thumb(url: &str) -> crate::link_preview::Picture {
        let n: u32 = url
            .split('/')
            .find_map(|s| s.strip_prefix("demo").and_then(|d| d.parse().ok()).or_else(|| s.parse().ok()))
            .unwrap_or(0);
        let (w, h) = (110usize, 84usize);
        let palette = [[244, 114, 182], [94, 234, 212], [252, 211, 77], [129, 140, 248], [134, 239, 172]];
        let base = [[38, 42, 64], [52, 36, 70], [30, 52, 60], [60, 44, 38]][(n % 4) as usize];
        let frames = (0..10)
            .map(|f| {
                let a = f as f32 / 10.0 * std::f32::consts::TAU;
                let mut rgba = Vec::with_capacity(w * h * 4);
                for y in 0..h {
                    for x in 0..w {
                        let mut rgb = base;
                        for k in 0..3usize {
                            let phase = k as f32 * 2.1 + n as f32 * 0.7;
                            let (cx, cy) = (55.0 + 26.0 * (a + phase).cos(), 42.0 + 20.0 * (a + phase).sin());
                            if (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2) < 100.0 {
                                rgb = palette[(n as usize + k) % palette.len()];
                            }
                        }
                        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                    }
                }
                (egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba), Duration::from_millis(90))
            })
            .collect::<Vec<_>>();
        let blurred = frames[0].0.clone();
        crate::link_preview::Picture {
            frames,
            size: [w as u32, h as u32],
            blurred,
        }
    }
}
