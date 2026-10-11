//! Login news. Release checks are owned by the update controller so the login
//! panel and the modal always show the same result.
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
    rx: Option<crossbeam_channel::Receiver<Vec<NewsItem>>>,
}
const BUILTIN_NEWS: &str = include_str!("../../assets/news.json");
fn cache_path() -> std::path::PathBuf {
    crate::settings::cache_dir().join("news.json")
}
impl LoginInfo {
    pub fn preview_release(version: &str) -> Self {
        Self {
            news: serde_json::from_str(BUILTIN_NEWS).unwrap_or_default(),
            version: VersionState::Available {
                version: version.into(),
                url: String::new(),
            },
            can_check_updates: true,
            rx: None,
        }
    }
    pub fn start(rt: &tokio::runtime::Handle) -> Self {
        let news = std::fs::read(cache_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<NewsItem>>(&b).ok())
            .filter(|n| !n.is_empty())
            .or_else(|| serde_json::from_str(BUILTIN_NEWS).ok())
            .unwrap_or_default();
        let mut info = Self {
            news,
            version: VersionState::NotConfigured,
            can_check_updates: false,
            rx: None,
        };
        if crate::links::NEWS_FEED.is_empty() || std::env::var_os("AURORA_DEMO").is_some() || aurora_updater::test_environment().is_some() {
            return info;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        info.rx = Some(rx);
        rt.spawn(async move {
            let Ok(http) = reqwest::Client::builder()
                .user_agent(aurora_version::user_agent())
                .timeout(Duration::from_secs(8))
                .build()
            else {
                return;
            };
            if let Ok(r) = http.get(crate::links::NEWS_FEED).send().await
                && let Some(items) = r.bytes().await.ok().and_then(|b| serde_json::from_slice::<Vec<NewsItem>>(&b).ok())
            {
                let items: Vec<NewsItem> = items.into_iter().take(20).collect();
                if let Ok(b) = serde_json::to_vec(&items) {
                    let _ = std::fs::write(cache_path(), b);
                }
                let _ = tx.send(items);
            }
        });
        info
    }
    pub fn poll(&mut self) {
        if let Some(rx) = &self.rx {
            while let Ok(n) = rx.try_recv() {
                if !n.is_empty() {
                    self.news = n;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn builtin_news_valid() {
        let n: Vec<super::NewsItem> = serde_json::from_str(super::BUILTIN_NEWS).expect("built-in news");
        assert!(!n.is_empty());
    }
}
