//! Scheme libcurl guesses for a URL that has none.
//!
//! The Legacy Viewer passes a capability string to `CURLOPT_URL`
//! (`LLCore::HttpOpRequest::prepareRequest`, indra/llcorehttp/_httpoprequest.cpp)
//! and does not set `CURLOPT_DEFAULT_PROTOCOL`. libcurl then treats a leading
//! `scheme:/` as absolute (`Curl_is_absolute_url`, guessing mode, lib/urlapi.c).
//! Otherwise `guess_scheme` uses the host prefix `ftp.`, `dict.`, `ldap.`,
//! `imap.`, `smtp.` or `pop3.`, and `http` for every other host. A value that
//! starts with `/` is left alone: libcurl still has no host for it.

/// `MAX_SCHEME_LEN` in libcurl's urlapi.
const MAX_SCHEME_LEN: usize = 40;

/// The URL libcurl would request. Already-absolute strings and path-only
/// strings are returned unchanged.
pub(crate) fn ensure_scheme(url: &str) -> String {
    if url.is_empty() || url.starts_with('/') || has_scheme_slash(url) {
        return url.to_owned();
    }
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
        .find(|(prefix, _)| host.len() >= prefix.len() && host[..prefix.len()].eq_ignore_ascii_case(prefix))
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
}
