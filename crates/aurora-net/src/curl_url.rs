//! Scheme libcurl guesses for a URL that has none.
//!
//! The Legacy Viewer passes a capability string to `CURLOPT_URL`
//! (`LLCore::HttpOpRequest::prepareRequest`, indra/llcorehttp/_httpoprequest.cpp)
//! and does not set `CURLOPT_DEFAULT_PROTOCOL`. It links libcurl 7.54.1
//! (`curl` in its autobuild.xml, secondlife/3p-curl v7.54.1-r4), where
//! `parseurlandfillconn` (lib/url.c) reads a scheme of at most 15 characters
//! other than `/`, `:` and a line end, followed by `:` and one to three
//! slashes (`%15[^\n/:]:%3[/]`). Without one, the authority up to `/`, `?`
//! or `#` is the host, user name included, and its prefix `FTP.`, `DICT.`,
//! `LDAP.`, `IMAP.`, `SMTP.` or `POP3.` (any case, `checkprefix`) picks that
//! scheme; every other host gets `http`. A value that starts with `/` is left
//! alone: libcurl has no host for it either and refuses it.
//!
//! Stricter than libcurl on purpose: a string the HTTP client already reads
//! as `http` or `https` (`https:@host/…`, a backslash, a tab, a leading line
//! end) is left as written, so a guess never turns the grid's `https` into
//! `http` or sends the capability to another host.

use std::borrow::Cow;

/// The `%15[^\n/:]` of libcurl 7.54.1's scheme.
const MAX_SCHEME_LEN: usize = 15;

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
    url.is_empty()
        || url.starts_with('/')
        || has_scheme_slash(url)
        // The grid wrote http(s): a guess never replaces it (only reached for
        // strings that would otherwise be guessed).
        || reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

fn with_guessed_scheme(url: &str) -> String {
    let mut out = String::with_capacity(url.len() + 8);
    out.push_str(guessed_scheme(host_of(url)));
    out.push_str("://");
    out.push_str(url);
    out
}

/// `%15[^\n/:]:%3[/]`: 1 to 15 bytes other than a line end, `/` and `:`,
/// then `:` and a slash. `a.example:8003/cap` is a host and a port, not a
/// scheme; a longer token is no scheme either.
fn has_scheme_slash(url: &str) -> bool {
    let bytes = url.as_bytes();
    let end = bytes.iter().take(MAX_SCHEME_LEN + 1).position(|c| matches!(c, b'\n' | b'/' | b':'));
    matches!(end, Some(i) if i > 0 && bytes[i] == b':' && bytes.get(i + 1) == Some(&b'/'))
}

/// `%[^\n/?#]`: the whole authority, a `user:password@` included (libcurl
/// 7.54.1 removes it only after the guess).
fn host_of(url: &str) -> &str {
    let authority_end = url.find(['\n', '/', '?', '#']).unwrap_or(url.len());
    &url[..authority_end]
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
        // 7.54.1 checks the prefix before it removes the user name
        assert_eq!(
            ensure_scheme("user:secret@ftp.example.com/readme"),
            "http://user:secret@ftp.example.com/readme"
        );
        assert_eq!(ensure_scheme("ftp.example.com@x/readme"), "ftp://ftp.example.com@x/readme");
        assert_eq!(ensure_scheme("dict.example.org:2628"), "dict://dict.example.org:2628");
        assert_eq!(ensure_scheme("imap.example/inbox"), "imap://imap.example/inbox");
        assert_eq!(ensure_scheme("smtp.example/"), "smtp://smtp.example/");
        assert_eq!(ensure_scheme("pop3.example"), "pop3://pop3.example");
        assert_eq!(ensure_scheme("ldap.example/dc=example"), "ldap://ldap.example/dc=example");
    }

    /// libcurl 7.54.1's `%15[^\n/:]:%3[/]`.
    #[test]
    fn a_scheme_is_at_most_fifteen_bytes_before_a_colon_and_a_slash() {
        assert_eq!(ensure_scheme("abcdefghijklmno://h/x"), "abcdefghijklmno://h/x");
        assert_eq!(ensure_scheme("abcdefghijklmnop://h/x"), "http://abcdefghijklmnop://h/x");
        assert_eq!(ensure_scheme("https:/a.example/cap"), "https:/a.example/cap");
        assert_eq!(ensure_scheme("1http://a.example/cap"), "1http://a.example/cap");
    }

    /// Strings the HTTP client reads as https are never guessed into http
    /// or onto another host (stricter than libcurl, see the module comment).
    #[test]
    fn a_scheme_the_grid_wrote_as_https_is_never_replaced() {
        for url in [
            "https:@a.example/cap",
            "https:\\\\a.example\\cap",
            "https:\t//a.example/cap",
            "\nhttps://a.example/cap",
        ] {
            assert_eq!(ensure_scheme(url), url, "{url:?}");
            assert_eq!(ensure_scheme_owned(url.to_owned()), url, "{url:?}");
        }
        // real schemeless capabilities are still guessed
        assert_eq!(ensure_scheme("a.example:8003/cap"), "http://a.example:8003/cap");
        assert_eq!(ensure_scheme("user:pw@a.example/cap"), "http://user:pw@a.example/cap");
        assert_eq!(ensure_scheme("a.example/cap"), "http://a.example/cap");
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
