//! Scheme libcurl guesses for a URL that has none.
//!
//! The Legacy Viewer passes a capability string to `CURLOPT_URL`
//! (`LLCore::HttpOpRequest::prepareRequest`, indra/llcorehttp/_httpoprequest.cpp)
//! and does not set `CURLOPT_DEFAULT_PROTOCOL`. libcurl then treats a leading
//! `scheme:/` as absolute (`Curl_is_absolute_url`, guessing mode, lib/urlapi.c).
//! Otherwise `guess_scheme` uses the host prefix `ftp.`, `dict.`, `ldap.`,
//! `imap.`, `smtp.` or `pop3.`, and `http` for every other host. A value that
//! starts with `/` is left alone: libcurl still has no host for it.

use std::borrow::Cow;

/// `MAX_SCHEME_LEN` in libcurl's urlapi.
const MAX_SCHEME_LEN: usize = 40;

/// The URL libcurl would request. Already-absolute strings and path-only
/// strings are returned unchanged, without a copy.
pub(crate) fn ensure_scheme(url: &str) -> Cow<'_, str> {
    if is_left_alone(url) {
        Cow::Borrowed(url)
    } else {
        Cow::Owned(with_guessed_scheme(url))
    }
}

/// [`ensure_scheme`] for a string already owned: no copy when it is kept.
pub(crate) fn ensure_scheme_owned(url: String) -> String {
    if is_left_alone(&url) { url } else { with_guessed_scheme(&url) }
}

fn is_left_alone(url: &str) -> bool {
    url.is_empty() || url.starts_with('/') || has_scheme_slash(url)
}

fn with_guessed_scheme(url: &str) -> String {
    let mut out = String::with_capacity(url.len() + 8);
    out.push_str(guessed_scheme(host_of(url)));
    out.push_str("://");
    out.push_str(url);
    out
}

/// Guessing mode: the scheme is the leading token and the next character is `/`.
/// `a.example:8003/cap` is a host and a port, not a scheme.
fn has_scheme_slash(url: &str) -> bool {
    let bytes = url.as_bytes();
    if !bytes.first().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() && i < MAX_SCHEME_LEN {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
            i += 1;
            continue;
        }
        break;
    }
    bytes.get(i) == Some(&b':') && bytes.get(i + 1) == Some(&b'/')
}

/// Host inside the authority, after any `user:password@`.
fn host_of(url: &str) -> &str {
    let authority_end = url.find(['/', '?', '#']).unwrap_or(url.len());
    let authority = &url[..authority_end];
    authority.rsplit_once('@').map_or(authority, |(_, host)| host)
}

fn guessed_scheme(host: &str) -> &'static str {
    // `checkprefix` (lib/strcase.c) compares without regard to case.
    const PREFIXES: &[(&str, &str)] = &[
        ("ftp.", "ftp"),
        ("dict.", "dict"),
        ("ldap.", "ldap"),
        ("imap.", "imap"),
        ("smtp.", "smtp"),
        ("pop3.", "pop3"),
    ];
    PREFIXES
        .iter()
        // (bytes: the host comes from the grid and may cut a character in two)
        .find(|(prefix, _)| {
            host.as_bytes()
                .get(..prefix.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(prefix.as_bytes()))
        })
        .map_or("http", |(_, scheme)| *scheme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schemeless_url_gets_the_scheme_libcurl_would_guess() {
        assert_eq!(ensure_scheme("a.example/cap"), "http://a.example/cap");
        // reqwest would read `a.example` as the scheme. libcurl does not:
        // guessing mode requires `scheme:/`.
        assert_eq!(ensure_scheme("a.example:8003/cap"), "http://a.example:8003/cap");
        assert_eq!(ensure_scheme("https://a.example/cap"), "https://a.example/cap");
        assert_eq!(ensure_scheme("HTTP://a.example/cap"), "HTTP://a.example/cap");
        assert_eq!(ensure_scheme("/cap/x"), "/cap/x");
        assert_eq!(ensure_scheme(""), "");
        assert_eq!(ensure_scheme("FTP.example.com/file"), "ftp://FTP.example.com/file");
        assert_eq!(
            ensure_scheme("user:secret@ftp.example.com/readme"),
            "ftp://user:secret@ftp.example.com/readme"
        );
        assert_eq!(ensure_scheme("dict.example.org:2628"), "dict://dict.example.org:2628");
        assert_eq!(ensure_scheme("imap.example/inbox"), "imap://imap.example/inbox");
        assert_eq!(ensure_scheme("smtp.example/"), "smtp://smtp.example/");
        assert_eq!(ensure_scheme("pop3.example"), "pop3://pop3.example");
        assert_eq!(ensure_scheme("ldap.example/dc=example"), "ldap://ldap.example/dc=example");
    }

    #[test]
    fn a_kept_url_is_not_copied() {
        assert!(matches!(ensure_scheme("https://a.example/cap"), Cow::Borrowed(_)));
        assert!(matches!(ensure_scheme("/cap/x"), Cow::Borrowed(_)));
        assert_eq!(ensure_scheme_owned("a.example/cap".into()), "http://a.example/cap");
        assert_eq!(ensure_scheme_owned("https://a.example/cap".into()), "https://a.example/cap");
    }

    /// Grid data: the prefix length may fall inside a multi-byte character.
    #[test]
    fn a_multi_byte_host_does_not_panic() {
        assert_eq!(ensure_scheme("dicté.grid/cap"), "http://dicté.grid/cap");
        assert_eq!(ensure_scheme("ftp€x/cap"), "http://ftp€x/cap");
        assert_eq!(ensure_scheme("pop€/cap"), "http://pop€/cap");
        assert_eq!(ensure_scheme("é"), "http://é");
        assert_eq!(ensure_scheme("u@ïmap./x"), "http://u@ïmap./x");
    }
}
