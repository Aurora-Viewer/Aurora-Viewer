//! Login screen information: news and the latest released version. Built-in
//! news are shown at once; the feed and the release check (`crate::links`)
//! are fetched in the background when configured, and cached on disk.

use serde::{Deserialize, Serialize};
use std::time::Duration;

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

#[derive(Debug, Clone, PartialEq)]
pub enum VersionState {
    /// No release source configured.
    NotConfigured,
    Checking,
    UpToDate,
    Available {
        version: String,
        url: String,
    },
    Failed,
}

pub struct LoginInfo {
    pub news: Vec<NewsItem>,
    pub version: VersionState,
    rx: Option<crossbeam_channel::Receiver<Update>>,
}

enum Update {
    News(Vec<NewsItem>),
    Version(VersionState),
}

const BUILTIN_NEWS: &str = include_str!("../../assets/news.json");

fn cache_path() -> std::path::PathBuf {
    crate::settings::cache_dir().join("news.json")
}

/// "v1.2.3" / "1.2.3-beta" → [1, 2, 3].
fn version_numbers(v: &str) -> Vec<u32> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .map_while(|p| p.parse::<u32>().ok())
        .collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (a, b) = (version_numbers(latest), version_numbers(current));
    if a.is_empty() {
        return false;
    }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

impl LoginInfo {
    /// Built-in / cached news now; network updates in the background.
    pub fn start(rt: &tokio::runtime::Handle) -> LoginInfo {
        let cached = std::fs::read(cache_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<NewsItem>>(&b).ok());
        let news = cached
            .filter(|n| !n.is_empty())
            .or_else(|| serde_json::from_str(BUILTIN_NEWS).ok())
            .unwrap_or_default();
        let fetch_news = !crate::links::NEWS_FEED.is_empty();
        let fetch_version = !crate::links::RELEASES_API.is_empty();
        let version = if fetch_version {
            VersionState::Checking
        } else {
            VersionState::NotConfigured
        };
        let mut info = LoginInfo { news, version, rx: None };
        if !fetch_news && !fetch_version {
            return info;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        info.rx = Some(rx);
        rt.spawn(async move {
            let Ok(http) = reqwest::Client::builder()
                .user_agent(concat!("AuroraViewer/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(8))
                .build()
            else {
                return;
            };
            if fetch_news
                && let Ok(r) = http.get(crate::links::NEWS_FEED).send().await
                && let Some(items) = r.bytes().await.ok().and_then(|b| serde_json::from_slice::<Vec<NewsItem>>(&b).ok())
            {
                let items: Vec<NewsItem> = items.into_iter().take(20).collect();
                if let Ok(b) = serde_json::to_vec(&items) {
                    let _ = std::fs::write(cache_path(), b);
                }
                let _ = tx.send(Update::News(items));
            }
            if fetch_version {
                let state = match http
                    .get(crate::links::RELEASES_API)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                {
                    Ok(r) => match r
                        .bytes()
                        .await
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    {
                        Some(v) => {
                            let tag = v["tag_name"].as_str().unwrap_or_default().to_owned();
                            let url = v["html_url"].as_str().unwrap_or_default().to_owned();
                            if is_newer(&tag, env!("CARGO_PKG_VERSION")) {
                                VersionState::Available {
                                    version: tag.trim_start_matches('v').to_owned(),
                                    url,
                                }
                            } else if tag.is_empty() {
                                VersionState::Failed
                            } else {
                                VersionState::UpToDate
                            }
                        }
                        None => VersionState::Failed,
                    },
                    Err(_) => VersionState::Failed,
                };
                let _ = tx.send(Update::Version(state));
            }
        });
        info
    }

    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        while let Ok(u) = rx.try_recv() {
            match u {
                Update::News(n) if !n.is_empty() => self.news = n,
                Update::News(_) => {}
                Update::Version(v) => self.version = v,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_and_builtin_news() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        let n: Vec<NewsItem> = serde_json::from_str(BUILTIN_NEWS).expect("built-in news");
        assert!(!n.is_empty());
    }
}
