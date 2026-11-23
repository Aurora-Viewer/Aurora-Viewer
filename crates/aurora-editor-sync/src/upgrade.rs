//! Which WebSocket upgrade requests are let in.
//!
//! Port of LLWebsocketMgr::WSServer::acceptOrigin and Server_impl::onValidate
//! (indra/llcorehttp/llwebsocketmgr.{h,cpp}, originally LGPL 2.1): a request
//! carrying an `Origin` comes from a browser (a web page, or media on a prim
//! in the viewer's own browser) and is answered 403. An editor's extension
//! is a program and sends none.
//!
//! Stricter than the Legacy Viewer, which checks the origin only:
//! - the `Host` must name the loopback address with our port (or no port),
//!   so a page that rebinds its own name to 127.0.0.1 is refused even if a
//!   browser ever left the origin out;
//! - the `Sec-Fetch-*` headers that only browsers send are refused too.

/// Why an upgrade request was refused. Logged once per kind, never with the
/// header's value (a page chooses it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Origin,
    Browser,
    Host,
}

impl Refusal {
    pub fn describe(self) -> &'static str {
        match self {
            Refusal::Origin => "a browser origin",
            Refusal::Browser => "a browser fetch",
            Refusal::Host => "a host other than this computer",
        }
    }
}

/// The headers of the request that matter here, read by the caller.
#[derive(Debug, Default, Clone, Copy)]
pub struct Headers<'a> {
    pub origin: Option<&'a str>,
    pub host: Option<&'a str>,
    /// Any `Sec-Fetch-Site`, `Sec-Fetch-Mode` or `Sec-Fetch-Dest`.
    pub sec_fetch: bool,
}

pub fn check(headers: Headers<'_>, port: u16) -> Result<(), Refusal> {
    // acceptOrigin: an origin of any value, even "null" or empty, is a page
    if headers.origin.is_some() {
        return Err(Refusal::Origin);
    }
    if headers.sec_fetch {
        return Err(Refusal::Browser);
    }
    match headers.host {
        Some(host) if loopback_host(host, port) => Ok(()),
        // HTTP/1.1 requires a Host: one without is not an editor's
        _ => Err(Refusal::Host),
    }
}

/// `127.0.0.1`, `localhost` or `[::1]`, with our port or none.
fn loopback_host(host: &str, port: u16) -> bool {
    let host = host.trim();
    let (name, given_port) = if let Some(rest) = host.strip_prefix('[') {
        let Some((inside, after)) = rest.split_once(']') else {
            return false;
        };
        match after {
            "" => (inside, None),
            _ => match after.strip_prefix(':') {
                Some(p) => (inside, Some(p)),
                None => return false,
            },
        }
    } else {
        match host.split_once(':') {
            Some((name, p)) => (name, Some(p)),
            None => (host, None),
        }
    };
    if let Some(p) = given_port
        && p.parse::<u16>().ok() != Some(port)
    {
        return false;
    }
    name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost") || name == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(h: &str) -> Result<(), Refusal> {
        check(
            Headers {
                host: Some(h),
                ..Headers::default()
            },
            9020,
        )
    }

    #[test]
    fn an_editor_on_this_computer_is_let_in() {
        for h in [
            "127.0.0.1:9020",
            "localhost:9020",
            "LOCALHOST:9020",
            "[::1]:9020",
            "127.0.0.1",
            "localhost",
            "[::1]",
            " 127.0.0.1:9020 ",
        ] {
            assert_eq!(host(h), Ok(()), "{h}");
        }
    }

    #[test]
    fn any_browser_origin_is_refused_as_the_legacy_viewer_does() {
        for origin in ["https://evil.example", "null", "", "http://127.0.0.1:9020", "vscode-webview://abc"] {
            let headers = Headers {
                origin: Some(origin),
                host: Some("127.0.0.1:9020"),
                sec_fetch: false,
            };
            assert_eq!(check(headers, 9020), Err(Refusal::Origin), "{origin}");
        }
        let fetch = Headers {
            origin: None,
            host: Some("127.0.0.1:9020"),
            sec_fetch: true,
        };
        assert_eq!(check(fetch, 9020), Err(Refusal::Browser));
    }

    #[test]
    fn rebound_names_and_other_ports_are_refused() {
        for h in [
            "evil.example:9020",
            "evil.example",
            "127.0.0.1.evil.example:9020",
            "localhost.evil.example:9020",
            "127.0.0.2:9020",
            "0.0.0.0:9020",
            "192.168.1.10:9020",
            "127.0.0.1:9021",
            "127.0.0.1:",
            "127.0.0.1:abc",
            "127.0.0.1:9020:9020",
            "[::1]9020",
            "[::1",
            "[::2]:9020",
            "",
            "localhost@evil.example",
        ] {
            assert_eq!(host(h), Err(Refusal::Host), "{h}");
        }
        assert_eq!(check(Headers::default(), 9020), Err(Refusal::Host));
    }
}
