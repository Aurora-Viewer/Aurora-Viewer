//! Project links shown on the login screen. Empty = not configured yet: the
//! button is shown disabled and nothing is fetched.

/// Source code repository (GitHub page).
pub const GITHUB: &str = "";
/// Community Discord invite.
pub const DISCORD: &str = "";
/// Project website.
pub const WEBSITE: &str = "";
/// Latest release: GitHub API `https://api.github.com/repos/OWNER/REPO/releases/latest`
/// (`tag_name`, `html_url`).
pub const RELEASES_API: &str = "";
/// News feed: JSON array of `{"date", "title", "summary", "url"}`.
pub const NEWS_FEED: &str = "";
