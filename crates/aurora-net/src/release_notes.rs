//! Help › Server release notes: the ServerReleaseNotes capability of the
//! agent's region.
//!
//! Port of LLFloaterAbout::startFetchServerReleaseNotes /
//! fetchServerReleaseNotesCoro / handleServerReleaseNotes
//! (indra/newview/llfloaterabout.cpp, originally LGPL 2.1): the capability
//! itself is not opened in the browser (its certificate is the simulator's),
//! it is fetched without following redirects and its `Location` header is
//! the page to open.

use std::time::Duration;

/// The page the capability redirects to, if it is a web address.
pub fn page_of(location: Option<&str>) -> Option<String> {
    let url = location?.trim();
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| url.to_owned())
}

/// GET the capability without following its redirect; the page it points
/// to, or why there is none (ErrorFetchingServerReleaseNotesURL).
pub async fn fetch(cap_url: &str) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .user_agent(aurora_version::user_agent())
        .redirect(reqwest::redirect::Policy::none())
        .tls_certs_only(crate::simulator_roots())
        .tls_danger_accept_invalid_hostnames(true)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = http.get(cap_url).send().await.map_err(|e| e.to_string())?;
    let location = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok());
    page_of(location).ok_or_else(|| format!("status {} without a page", resp.status().as_u16()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_web_page_is_opened() {
        assert_eq!(
            page_of(Some(" https://releasenotes.example/simulator/2026.html ")).as_deref(),
            Some("https://releasenotes.example/simulator/2026.html")
        );
        assert_eq!(
            page_of(Some("HTTP://example.org/notes")).as_deref(),
            Some("HTTP://example.org/notes")
        );
        assert_eq!(page_of(None), None);
        assert_eq!(page_of(Some("")), None);
        assert_eq!(page_of(Some("file:///C:/Windows")), None);
        assert_eq!(page_of(Some("secondlife:///app/teleport")), None);
    }
}
