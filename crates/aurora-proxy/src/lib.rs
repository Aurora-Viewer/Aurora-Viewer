//! The one rule every outgoing connection of the viewer follows: the proxy
//! settings of Preferences › Network (the Legacy Viewer's
//! floater_preferences_proxy.xml), read once at startup and installed for
//! the whole process (`install`), like LLProxy's singleton.
//!
//! - Every reqwest client of every crate is built by `http_client`; a guard
//!   test (`tests/no_bypass.rs`) fails when production code builds one
//!   another way.
//! - The circuits' UDP goes through a SOCKS 5 relay (`udp`); the radio's
//!   own TCP through `connect::tcp_connect`.
//! - Fail closed: a proxy that is set up but wrong (bad address, missing
//!   credentials) blocks every request instead of letting it go direct, and
//!   the login says why (`Problem`). The Legacy Viewer lets the requests made
//!   before the login go direct when its startup check fails.
//! - Loopback stays direct (the demo, a local grid, the media plugins' IPC).
//! - The proxy resolves the names (`socks5h`, HTTP CONNECT): no DNS request
//!   for a visited host leaves this computer. The Legacy Viewer gives libcurl
//!   CURLPROXY_SOCKS5, which resolves them locally.
//! - TLS checks are the clients' own and do not change with the proxy.
//!
//! Port of LLStartUp::startLLProxy (indra/newview/llstartup.cpp) and
//! LLProxy::applyProxySettings (indra/llmessage/llproxy.cpp), originally
//! LGPL 2.1.

pub mod connect;
pub mod socks5;
pub mod udp;

use std::net::IpAddr;
use std::sync::OnceLock;

pub use socks5::{Credentials, SocksError};

/// SOCKS 5 authentication (Socks5AuthType: "None" or "UserPass").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SocksAuth {
    #[default]
    None,
    UserPass,
}

/// The proxy of the viewer's other HTTP traffic (HttpProxyType: "None",
/// "Web", "Socks"). The Legacy Viewer's default is Socks: it only acts once
/// the SOCKS 5 proxy is on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HttpProxyType {
    None,
    Web,
    #[default]
    Socks,
}

/// The settings, as saved in settings.json (`Settings::proxy`). The user name
/// and password are not here: they live in the OS credential store, like the
/// Legacy Viewer's "SOCKS5" credential.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProxySettings {
    /// Use a SOCKS 5 proxy for the UDP traffic (Socks5ProxyEnabled).
    pub socks_enabled: bool,
    /// Socks5ProxyHost.
    pub socks_host: String,
    /// Socks5ProxyPort, 1080.
    pub socks_port: u16,
    /// Socks5AuthType.
    pub socks_auth: SocksAuth,
    /// HttpProxyType.
    pub http_type: HttpProxyType,
    /// Use an HTTP proxy for web pages (BrowserProxyEnabled).
    pub web_enabled: bool,
    /// BrowserProxyAddress.
    pub web_host: String,
    /// BrowserProxyPort, 3128.
    pub web_port: u16,
    /// Let voice connect directly while a proxy is on. Voice (WebRTC) cannot
    /// go through these proxies and shows the real address to the voice
    /// servers; the Legacy Viewer always lets it go direct. Aurora turns
    /// voice off while a proxy is on, unless the user turns this on after a
    /// warning. Off.
    pub direct_voice: bool,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            socks_enabled: false,
            socks_host: String::new(),
            socks_port: 1080,
            socks_auth: SocksAuth::None,
            http_type: HttpProxyType::Socks,
            web_enabled: false,
            web_host: String::new(),
            web_port: 3128,
            direct_voice: false,
        }
    }
}

impl std::fmt::Debug for ProxySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // the proxies' addresses can name the user's network: never printed
        f.debug_struct("ProxySettings")
            .field("socks_enabled", &self.socks_enabled)
            .field("socks_auth", &self.socks_auth)
            .field("http_type", &self.http_type)
            .field("web_enabled", &self.web_enabled)
            .field("direct_voice", &self.direct_voice)
            .finish_non_exhaustive()
    }
}

impl ProxySettings {
    /// Whether any proxy is on: the user wants their address hidden.
    pub fn any_proxy(&self) -> bool {
        self.socks_enabled || self.web_enabled
    }

    /// The HTTP choice the proxies allow: a choice whose proxy is off falls
    /// back to None (LLFloaterPreferenceProxy::onChangeSocksSettings).
    pub fn effective_http_type(&self) -> HttpProxyType {
        match self.http_type {
            HttpProxyType::Socks if self.socks_enabled => HttpProxyType::Socks,
            HttpProxyType::Web if self.web_enabled => HttpProxyType::Web,
            _ => HttpProxyType::None,
        }
    }
}

/// A proxy's host and port, checked.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Endpoint(<redacted>)")
    }
}

impl Endpoint {
    /// A host name or IP literal, and a port other than 0. Anything that
    /// could change the meaning of an URL (`@`, `/`, a space, a scheme) is
    /// refused: the host becomes part of the proxy's URL.
    pub fn new(host: &str, port: u16) -> Option<Self> {
        let host = host.trim();
        let bare = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
        let valid = if bare.parse::<IpAddr>().is_ok() {
            true
        } else {
            (1..=253).contains(&bare.len())
                && bare.split('.').all(|label| {
                    (1..=63).contains(&label.len())
                        && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                })
        };
        (valid && port != 0).then(|| Self {
            host: bare.to_owned(),
            port,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `host:port` with IPv6 literals in brackets, for an URL.
    fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// A SOCKS 5 server and its credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocksServer {
    pub endpoint: Endpoint,
    pub credentials: Option<Credentials>,
}

/// How HTTP requests leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpRoute {
    /// No proxy of the viewer: the environment's proxies (`http_proxy`,
    /// `https_proxy`, `all_proxy`, `no_proxy`), as libcurl reads them for
    /// the Legacy Viewer; loopback always direct.
    System,
    Socks(SocksServer),
    Web(Endpoint),
    /// The settings name a proxy that cannot be used: nothing leaves.
    Blocked,
}

/// Why the proxy settings cannot be used. Each one has the Legacy Viewer's
/// notification; the login refuses to start while there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// SOCKS_INVALID_HOST / PROXY_INVALID_SOCKS_HOST.
    InvalidSocksHost,
    /// PROXY_INVALID_HTTP_HOST.
    InvalidWebHost,
    /// SOCKS_BAD_CREDS: user name and password asked for, missing or invalid.
    BadCredentials,
}

/// The installed rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub http: HttpRoute,
    /// The SOCKS 5 server for the circuits, when on and valid.
    pub udp: Option<SocksServer>,
    /// The HTTP proxy given to the web browser plugin (BrowserProxyEnabled).
    pub browser: Option<Endpoint>,
    pub problem: Option<Problem>,
    /// Some proxy is on (`ProxySettings::any_proxy`).
    pub any_proxy: bool,
    pub direct_voice: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self::direct()
    }
}

impl Rule {
    /// No proxy configured.
    pub fn direct() -> Self {
        Self {
            http: HttpRoute::System,
            udp: None,
            browser: None,
            problem: None,
            any_proxy: false,
            direct_voice: false,
        }
    }

    /// The rule of `settings`. `credentials` is what the credential store
    /// gave for the SOCKS 5 proxy (only read when the settings ask for
    /// UserPass). Port of LLStartUp::startLLProxy's checks.
    pub fn from_settings(settings: &ProxySettings, credentials: Option<Credentials>) -> Self {
        let mut rule = Self {
            any_proxy: settings.any_proxy(),
            direct_voice: settings.direct_voice,
            ..Self::direct()
        };
        let socks = if settings.socks_enabled {
            let credentials = match settings.socks_auth {
                SocksAuth::None => None,
                SocksAuth::UserPass => match credentials {
                    Some(c) => Some(c),
                    None => {
                        rule.problem = Some(Problem::BadCredentials);
                        None
                    }
                },
            };
            match Endpoint::new(&settings.socks_host, settings.socks_port) {
                Some(endpoint) => Some(SocksServer { endpoint, credentials }),
                None => {
                    rule.problem = Some(Problem::InvalidSocksHost);
                    None
                }
            }
        } else {
            None
        };
        let web = if settings.web_enabled {
            let endpoint = Endpoint::new(&settings.web_host, settings.web_port);
            if endpoint.is_none() {
                rule.problem = rule.problem.or(Some(Problem::InvalidWebHost));
            }
            endpoint
        } else {
            None
        };
        if rule.problem.is_some() {
            rule.http = HttpRoute::Blocked;
            return rule;
        }
        rule.udp = socks.clone();
        rule.browser = web.clone();
        rule.http = match settings.effective_http_type() {
            HttpProxyType::Socks => socks.map_or(HttpRoute::Blocked, HttpRoute::Socks),
            HttpProxyType::Web => web.map_or(HttpRoute::Blocked, HttpRoute::Web),
            HttpProxyType::None => HttpRoute::System,
        };
        rule
    }

    /// Whether voice may connect: no proxy, or the user chose direct voice.
    pub fn voice_allowed(&self) -> bool {
        !self.any_proxy || self.direct_voice
    }

    /// Whether HTTP requests go through a proxy of the viewer (and so its
    /// DNS): the public-address checks of the link previews then cannot run
    /// on resolved addresses.
    pub fn proxies_http(&self) -> bool {
        matches!(self.http, HttpRoute::Socks(_) | HttpRoute::Web(_) | HttpRoute::Blocked)
    }

    /// One line for the log, without addresses or names.
    pub fn summary(&self) -> &'static str {
        match (&self.http, self.udp.is_some(), self.problem.is_some()) {
            (_, _, true) => "proxy settings invalid: network blocked",
            (HttpRoute::Socks(_), true, _) => "SOCKS 5 proxy for UDP and HTTP",
            (HttpRoute::Web(_), true, _) => "SOCKS 5 proxy for UDP, HTTP proxy for HTTP",
            (HttpRoute::System, true, _) => "SOCKS 5 proxy for UDP, HTTP not proxied",
            (HttpRoute::Web(_), false, _) => "HTTP proxy for HTTP",
            _ if self.browser.is_some() => "HTTP proxy for web pages only",
            _ => "no proxy",
        }
    }

    /// Adds this rule's proxies to a reqwest builder, replacing any the
    /// caller set and the environment reading.
    fn apply(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        let builder = builder.no_proxy();
        let bypass = || reqwest::NoProxy::from_string(LOOPBACK);
        match &self.http {
            HttpRoute::System => environment_proxies(|name| std::env::var(name).ok())
                .into_iter()
                .fold(builder, |b, proxy| b.proxy(proxy)),
            HttpRoute::Socks(server) => match socks_proxy(server) {
                Some(proxy) => builder.proxy(proxy.no_proxy(bypass())),
                None => builder.proxy(blocked()),
            },
            HttpRoute::Web(endpoint) => match reqwest::Proxy::all(format!("http://{}", endpoint.authority())) {
                Ok(proxy) => builder.proxy(proxy.no_proxy(bypass())),
                Err(_) => builder.proxy(blocked()),
            },
            HttpRoute::Blocked => builder.proxy(blocked()),
        }
    }
}

/// Never proxied.
const LOOPBACK: &str = "localhost,.localhost,127.0.0.0/8,::1";

fn socks_proxy(server: &SocksServer) -> Option<reqwest::Proxy> {
    let proxy = reqwest::Proxy::all(format!("socks5h://{}", server.endpoint.authority())).ok()?;
    Some(match &server.credentials {
        Some(c) => proxy.basic_auth(c.username(), c.password()),
        None => proxy,
    })
}

/// The proxy of a blocked rule: port 0 of loopback, which no connection can
/// reach, so a request fails on this computer and never leaves it. (reqwest's
/// `Proxy::custom` would turn an error into a direct connection.)
fn blocked() -> reqwest::Proxy {
    #[allow(clippy::expect_used)]
    reqwest::Proxy::all("socks5h://127.0.0.1:0").expect("a constant, valid proxy URL")
}

/// The environment's proxies, as reqwest's own reading of them
/// (hyper_util::client::proxy::matcher::Builder::from_env), with loopback
/// always direct (issue #3903). `REQUEST_METHOD` (a CGI) disables
/// `HTTP_PROXY`, as there.
fn environment_proxies(var: impl Fn(&str) -> Option<String>) -> Vec<reqwest::Proxy> {
    let first = |names: [&str; 2]| names.iter().find_map(|n| var(n).filter(|v| !v.trim().is_empty()));
    let mut bypass = LOOPBACK.to_owned();
    if let Some(no) = first(["NO_PROXY", "no_proxy"]) {
        bypass.push(',');
        bypass.push_str(&no);
    }
    let bypass = || reqwest::NoProxy::from_string(&bypass);
    let mut out = Vec::new();
    let cgi = var("REQUEST_METHOD").is_some();
    if !cgi && let Some(url) = first(["HTTP_PROXY", "http_proxy"]) {
        out.extend(reqwest::Proxy::http(url).ok().map(|p| p.no_proxy(bypass())));
    }
    if let Some(url) = first(["HTTPS_PROXY", "https_proxy"]) {
        out.extend(reqwest::Proxy::https(url).ok().map(|p| p.no_proxy(bypass())));
    }
    if let Some(url) = first(["ALL_PROXY", "all_proxy"]) {
        out.extend(reqwest::Proxy::all(url).ok().map(|p| p.no_proxy(bypass())));
    }
    out
}

/// Whether a URL's host is this computer (always direct).
pub fn is_loopback_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
        };
    }
    let lower = bare.trim_end_matches('.').to_ascii_lowercase();
    lower == "localhost" || lower.ends_with(".localhost")
}

static RULE: OnceLock<Rule> = OnceLock::new();

/// Installs the process's rule, once, before the first client is built.
/// Returns false when a rule was already installed (the first one stays:
/// changed settings take effect at the next start, as the Legacy Viewer's
/// "Proxy settings take effect after you restart").
pub fn install(rule: Rule) -> bool {
    log::info!("network: {}", rule.summary());
    RULE.set(rule).is_ok()
}

/// The installed rule; no proxy when none was installed (the tests, the
/// tools).
pub fn rule() -> &'static Rule {
    static DIRECT: OnceLock<Rule> = OnceLock::new();
    RULE.get().unwrap_or_else(|| DIRECT.get_or_init(Rule::direct))
}

/// Builds a reqwest client that follows the rule: `configure` sets the
/// client's own options (timeouts, TLS, redirects…) on a fresh builder, then
/// the rule's proxies replace any proxy it set. The only way production code
/// builds a client (`tests/no_bypass.rs`).
pub fn http_client(configure: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder) -> reqwest::Result<reqwest::Client> {
    // the one construction of a reqwest builder outside the tests
    let builder = reqwest::Client::builder(); // proxy-guard: the rule's own builder
    rule().apply(configure(builder)).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ProxySettings {
        ProxySettings {
            socks_enabled: true,
            socks_host: "proxy.example".into(),
            ..Default::default()
        }
    }

    #[test]
    fn defaults_like_the_legacy_viewer() {
        let s = ProxySettings::default();
        assert_eq!((s.socks_port, s.web_port), (1080, 3128));
        assert_eq!(s.http_type, HttpProxyType::Socks);
        assert_eq!(s.socks_auth, SocksAuth::None);
        assert!(!s.socks_enabled && !s.web_enabled && !s.direct_voice);
        assert_eq!(Rule::from_settings(&s, None), Rule::direct());
    }

    #[test]
    fn socks_for_udp_and_http() {
        let rule = Rule::from_settings(&settings(), None);
        assert!(rule.problem.is_none());
        let server = rule.udp.clone().expect("udp");
        assert_eq!(rule.http, HttpRoute::Socks(server));
        assert!(rule.proxies_http() && !rule.voice_allowed());
    }

    #[test]
    fn http_choice_falls_back_when_its_proxy_is_off() {
        let mut s = settings();
        s.http_type = HttpProxyType::Web; // web proxy off
        assert_eq!(Rule::from_settings(&s, None).http, HttpRoute::System);
        s.http_type = HttpProxyType::None;
        let rule = Rule::from_settings(&s, None);
        assert_eq!(rule.http, HttpRoute::System);
        assert!(rule.udp.is_some());
        s.web_enabled = true;
        s.web_host = "10.0.0.2".into();
        s.http_type = HttpProxyType::Web;
        let rule = Rule::from_settings(&s, None);
        assert_eq!(rule.http, HttpRoute::Web(Endpoint::new("10.0.0.2", 3128).unwrap()));
        assert_eq!(rule.browser, Endpoint::new("10.0.0.2", 3128));
    }

    #[test]
    fn problems_block_everything() {
        let mut s = settings();
        s.socks_auth = SocksAuth::UserPass;
        let rule = Rule::from_settings(&s, None);
        assert_eq!((rule.problem, &rule.http), (Some(Problem::BadCredentials), &HttpRoute::Blocked));
        assert!(rule.udp.is_none());
        let creds = Credentials::new("u".into(), "p".into()).unwrap();
        assert!(Rule::from_settings(&s, Some(creds)).problem.is_none());
        s.socks_auth = SocksAuth::None;
        s.socks_host = "user@evil.example".into();
        assert_eq!(Rule::from_settings(&s, None).problem, Some(Problem::InvalidSocksHost));
        s.socks_host = String::new();
        assert_eq!(Rule::from_settings(&s, None).http, HttpRoute::Blocked);
        // a bad web proxy blocks too, even when HTTP was going through SOCKS
        let mut s = settings();
        s.web_enabled = true;
        s.web_host = "http://x".into();
        let rule = Rule::from_settings(&s, None);
        assert_eq!((rule.problem, &rule.http), (Some(Problem::InvalidWebHost), &HttpRoute::Blocked));
        assert!(rule.proxies_http());
    }

    #[test]
    fn endpoint_checks() {
        for good in ["proxy", "proxy.example", "10.1.2.3", "::1", "[2001:db8::1]", " a-b.c_d "] {
            assert!(Endpoint::new(good, 1080).is_some(), "{good}");
        }
        for bad in ["", "a b", "a/b", "u@h", "socks5://h", "h:1", "-a", "a-", "a..b", &"a".repeat(254)] {
            assert!(Endpoint::new(bad, 1080).is_none(), "{bad}");
        }
        assert!(Endpoint::new("proxy", 0).is_none());
        assert_eq!(Endpoint::new("[::1]", 9).unwrap().authority(), "[::1]:9");
    }

    #[test]
    fn voice_rule() {
        let mut s = settings();
        assert!(!Rule::from_settings(&s, None).voice_allowed());
        s.direct_voice = true;
        assert!(Rule::from_settings(&s, None).voice_allowed());
        let web_only = ProxySettings {
            web_enabled: true,
            web_host: "h".into(),
            ..Default::default()
        };
        assert!(!Rule::from_settings(&web_only, None).voice_allowed());
        assert!(Rule::direct().voice_allowed());
    }

    #[test]
    fn loopback_hosts() {
        for host in [
            "localhost",
            "LOCALHOST.",
            "a.localhost",
            "127.0.0.1",
            "127.9.9.9",
            "::1",
            "[::1]",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["localhost.example", "10.0.0.1", "example.org", "::2", "0.0.0.0"] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn nothing_secret_in_debug_or_summary() {
        let mut s = settings();
        s.socks_host = "secret-host.example".into();
        s.socks_auth = SocksAuth::UserPass;
        let creds = Credentials::new("secret-user".into(), "secret-pass".into()).unwrap();
        let rule = Rule::from_settings(&s, Some(creds));
        let shown = format!("{s:?} {rule:?} {}", rule.summary());
        assert!(!shown.contains("secret"), "{shown}");
        // a built client's Debug lists its proxies without their credentials
        // (hyper-util keeps them apart from the URI). reqwest's `Proxy`
        // itself does print them: it never leaves `socks_proxy`.
        let client = rule.apply(reqwest::Client::builder()).build().unwrap(); // proxy-guard: test
        let shown = format!("{client:?}");
        assert!(shown.contains("proxies"), "{shown}");
        assert!(!shown.contains("secret-pass") && !shown.contains("secret-user"), "{shown}");
    }

    /// An HTTP request through a SOCKS 5 server on loopback: the name goes
    /// to the proxy (no local DNS), with the credentials of RFC 1929.
    #[tokio::test]
    async fn http_through_socks_resolves_at_the_proxy() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut greet = [0u8; 3];
            s.read_exact(&mut greet).await.unwrap();
            assert_eq!(greet[0], 5);
            assert!(greet[2..].contains(&2), "{greet:?}");
            s.write_all(&[5, 2]).await.unwrap();
            let mut head = [0u8; 2];
            s.read_exact(&mut head).await.unwrap();
            let mut user = vec![0u8; usize::from(head[1])];
            s.read_exact(&mut user).await.unwrap();
            let mut plen = [0u8; 1];
            s.read_exact(&mut plen).await.unwrap();
            let mut pass = vec![0u8; usize::from(plen[0])];
            s.read_exact(&mut pass).await.unwrap();
            assert_eq!((user.as_slice(), pass.as_slice()), (&b"bob"[..], &b"p@ss:w%rd"[..]));
            s.write_all(&[1, 0]).await.unwrap();
            let mut req = [0u8; 5];
            s.read_exact(&mut req).await.unwrap();
            assert_eq!(&req[..4], &[5, 1, 0, 3], "a name, not an address");
            let mut name = vec![0u8; usize::from(req[4]) + 2];
            s.read_exact(&mut name).await.unwrap();
            assert_eq!(&name[..name.len() - 2], b"grid.example.invalid");
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
            let mut request = vec![0u8; 1024];
            let n = s.read(&mut request).await.unwrap();
            assert!(request[..n].starts_with(b"GET /status HTTP/1.1"));
            s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let mut s = settings();
        s.socks_host = "127.0.0.1".into();
        s.socks_port = port;
        s.socks_auth = SocksAuth::UserPass;
        let rule = Rule::from_settings(&s, Some(Credentials::new("bob".into(), "p@ss:w%rd".into()).unwrap()));
        let client = rule.apply(reqwest::Client::builder()).build().unwrap(); // proxy-guard: test
        let body = client
            .get("http://grid.example.invalid/status")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "ok");
        server.await.unwrap();
    }

    #[test]
    fn environment_reading() {
        let env =
            |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        assert!(environment_proxies(env(&[])).is_empty());
        assert_eq!(
            environment_proxies(env(&[("http_proxy", "http://p:1"), ("HTTPS_PROXY", "http://p:2")])).len(),
            2
        );
        // a CGI's HTTP_PROXY is a request header, not a proxy
        assert_eq!(
            environment_proxies(env(&[("HTTP_PROXY", "http://p:1"), ("REQUEST_METHOD", "GET")])).len(),
            0
        );
        assert_eq!(
            environment_proxies(env(&[("ALL_PROXY", "socks5h://p:1"), ("NO_PROXY", "example.org")])).len(),
            1
        );
    }

    /// Requests of a blocked rule fail on this computer: no connection to
    /// the destination, no direct fallback.
    #[tokio::test]
    async fn blocked_rule_never_connects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = Rule {
            http: HttpRoute::Blocked,
            ..Rule::direct()
        }
        .apply(reqwest::Client::builder()) // proxy-guard: test
        .build()
        .unwrap();
        // a non-loopback destination: refused before any connection
        let error = client
            .get("http://192.0.2.1/")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
        assert!(error.is_err());
        // loopback stays direct, as everywhere
        let accept = tokio::spawn(async move { listener.accept().await.is_ok() });
        let _ = client
            .get(format!("http://127.0.0.1:{port}/"))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await;
        assert!(accept.await.unwrap());
    }
}
