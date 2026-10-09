//! OpenID cookie of the Second Life web pages (web profiles), as
//! LLViewerMedia::openIDSetupCoro / openIDCookieResponse / parseRawCookie
//! (`newview/llviewermedia.cpp`, originally LGPL 2.1): the login answer
//! carries `openid_url` and `openid_token`; the token is posted to the URL
//! and the `Set-Cookie` of the answer is given to the browsers that show
//! my.secondlife.com, which then know who we are.
//!
//! The token and the cookie are session secrets: never logged.

use aurora_media::Cookie;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct OpenId {
    cookie: Arc<Mutex<Option<Cookie>>>,
}

impl OpenId {
    /// Post the login token (once per session).
    pub fn start(&self, runtime: &tokio::runtime::Handle, http: reqwest::Client, url: String, token: String) {
        *self.cookie.lock() = None;
        let Some((scheme, authority)) = url
            .split_once("://")
            .map(|(s, rest)| (s.to_owned(), rest.split('/').next().unwrap_or("").to_owned()))
        else {
            return;
        };
        let slot = self.cookie.clone();
        runtime.spawn(async move {
            let resp = http
                .post(&url)
                .header("Accept", "*/*")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(token)
                .send()
                .await;
            let raw = match resp {
                Ok(r) if r.status().is_success() => r
                    .headers()
                    .get(reqwest::header::SET_COOKIE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned),
                Ok(r) => {
                    log::warn!("OpenID: HTTP {}", r.status().as_u16());
                    None
                }
                Err(e) => {
                    log::warn!("OpenID: {e}");
                    None
                }
            };
            let Some(raw) = raw else {
                log::warn!("OpenID: no cookie in the answer");
                return;
            };
            match parse_raw_cookie(&raw, &scheme, &authority) {
                Some(c) => {
                    log::info!("OpenID: cookie received for {}", c.domain);
                    *slot.lock() = Some(c);
                }
                None => log::warn!("OpenID: unreadable cookie"),
            }
        });
    }

    pub fn cookie(&self) -> Option<Cookie> {
        self.cookie.lock().clone()
    }

    pub fn clear(&self) {
        *self.cookie.lock() = None;
    }
}

/// parseRawCookie + the host / CEF URL of getOpenIDCookie: "name=value;..."
/// for `scheme://authority`; path "/", http-only and secure like Firestorm.
fn parse_raw_cookie(raw: &str, scheme: &str, authority: &str) -> Option<Cookie> {
    let (name, rest) = raw.split_once('=')?;
    let (value, _) = rest.split_once(';')?;
    // [user[:password]@]host[:port]
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    Some(Cookie {
        uri: format!("{scheme}://{authority}"),
        name: name.to_owned(),
        value: value.to_owned(),
        domain: host.to_owned(),
        path: "/".into(),
        httponly: true,
        secure: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_cookie() {
        let c = parse_raw_cookie("session=abc123; Path=/; HttpOnly", "https", "id.example.com:443").expect("cookie");
        assert_eq!((c.name.as_str(), c.value.as_str()), ("session", "abc123"));
        assert_eq!(
            (c.domain.as_str(), c.uri.as_str()),
            ("id.example.com", "https://id.example.com:443")
        );
        // LL needs the ';' after the value
        assert!(parse_raw_cookie("session=abc123", "https", "id.example.com").is_none());
    }
}
