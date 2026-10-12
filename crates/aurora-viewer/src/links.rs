//! Project links shown on the login screen. Empty = not configured yet: the
//! button is shown disabled and nothing is fetched.

/// Source code repository (GitHub page).
pub const GITHUB: &str = "";
/// Community Discord invite.
pub const DISCORD: &str = "";
/// Project website.
pub const WEBSITE: &str = "";
/// News feed: JSON array of `{"date", "title", "summary", "url"}`.
/// That array is what the viewer keeps in its cache. The feed itself is this
/// folder of `main`: hand-edited Markdown files, one `<language>.md` per UI
/// language (`news/` at the repository root).
pub const NEWS_FEED: &str = "https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/news/";
/// Files of the source repository as GitHub serves them unrendered; a branch
/// name and a path follow.
pub const RAW_FILES: &str = "https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/";
