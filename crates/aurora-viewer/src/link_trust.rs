//! How far a web link of a chat or profile text can be trusted: a green
//! check for well-known sites, a red cross for links that hide or fake their
//! destination, an amber triangle for the rest. Judged locally on the URL
//! alone: nothing is sent anywhere.
//!
//! The trusted domains start from Firestorm's (LLUrlEntrySecondlifeURL and
//! LLUrlEntryFirestormURL, indra/llui/llurlentry.cpp); the red cross is an
//! Aurora addition. The host is read as a browser reads it (the WHATWG URL
//! rules of the `url` crate): an address of this computer or of the local
//! network gets the red cross however it is written (`127.1`,
//! `0x7f.0.0.1`, `2130706433`, `[::ffff:127.0.0.1]`, `localhost`,
//! `nas.local`), and link previews never fetch it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Where a web link comes from, for the browser choice
/// (`settings::BrowserBehavior`, the Legacy Viewer's PreferredBrowserBehavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserEntryPoint {
    /// A link of a text, a notification, a script or a server reply
    /// (LLWeb::loadURL).
    Link,
    /// A Marketplace page opened from outside the Marketplace window.
    MarketplaceLink,
    /// The Marketplace listings' open action: the Marketplace window in
    /// every mode.
    MarketplaceWindow,
    /// An "Open in browser" button (LLWeb::loadURLExternal): the system
    /// browser in every mode.
    ExplicitExternal,
}

/// Whether a web link leaves Aurora for the system browser, like
/// LLWeb::useExternalBrowser (indra/newview/llweb.cpp, originally LGPL 2.1).
/// The toolbar's Web browser button is not routed here: toggleWebBrowser
/// (llviewermenu.cpp) always opens the built-in window.
pub fn should_use_system_browser(url: &str, behavior: crate::settings::BrowserBehavior, entry: BrowserEntryPoint) -> bool {
    use crate::settings::BrowserBehavior;
    match (entry, behavior) {
        (BrowserEntryPoint::ExplicitExternal, _) => true,
        (BrowserEntryPoint::MarketplaceWindow, _) => false,
        (_, BrowserBehavior::SystemForAll) => true,
        (_, BrowserBehavior::BuiltInForAll) => false,
        // the grid's Marketplace (its host can be Aditi's) keeps its window
        (BrowserEntryPoint::MarketplaceLink, BrowserBehavior::BuiltInForSecondLife) => false,
        (BrowserEntryPoint::Link, BrowserBehavior::BuiltInForSecondLife) => !second_life_host(url),
    }
}

/// The hosts of LLWeb::useExternalBrowser's "built-in for Second Life links"
/// choice, their subdomains included. The Legacy Viewer's pattern
/// (`\b(lindenlab.com|…)$`) also takes `my-secondlife.com`; only a dot before
/// the domain counts here, so a look-alike host leaves Aurora.
const SECOND_LIFE_HOSTS: &[&str] = &[
    "lindenlab.com",
    "secondlife.com",
    "secondlife.io",
    "secondlifegrid.net",
    "secondlife-status.statuspage.io",
];

/// Whether `url` is a page of the Second Life sites. An address with no host
/// the parser can read is not one, so it leaves Aurora, as in the Legacy
/// Viewer (an empty host matches no pattern).
pub fn second_life_host(url: &str) -> bool {
    let Some(host) = reqwest::Url::parse(url.trim())
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
    else {
        return false;
    };
    SECOND_LIFE_HOSTS
        .iter()
        .any(|domain| host == *domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.')))
}

/// Trusted domains, their subdomains included. Hosts of pages or files
/// anyone can publish (github.io, githubusercontent.com, sites.google.com,
/// dropbox.com...) stay out: a fake login page can live there.
const TRUSTED: &[&str] = &[
    // Second Life and Linden Lab
    "secondlife.com",
    "lindenlab.com",
    "tilia-inc.com",
    // CasperTech (CasperVend, CasperLet), a Linden Lab company since 2022
    "casperdns.com",
    "casperpanel.com",
    "casperlet.com",
    "secondlifegrid.net",
    "secondlife.io",
    "secondlife-status.statuspage.io",
    // viewers
    "firestormviewer.org",
    "phoenixviewer.com",
    // the project's official domain; the former auroraviewer.com is no
    // longer trusted (once released, anyone could own it)
    "aurora-viewer.org",
    // OpenSimulator
    "opensimulator.org",
    "osgrid.org",
    // Second Life community
    "flickr.com",
    "primfeed.com",
    "discord.com",
    "discord.gg",
    // well-known sites
    "github.com",
    "youtube.com",
    "youtu.be",
    "twitch.tv",
    "vimeo.com",
    "wikipedia.org",
    "reddit.com",
    "bsky.app",
    "x.com",
    "twitter.com",
    "instagram.com",
    "facebook.com",
    "imgur.com",
    "gyazo.com",
    // the short links and picture servers of the image services whose
    // pages get a picture preview (#923, link_preview/services.rs)
    "flic.kr",
    "staticflickr.com",
    "redd.it",
    "prnt.sc",
    "prntscr.com",
    // X's picture servers (pbs.twimg.com) and its embed widget's API
    // (cdn.syndication.twimg.com), for the previews of X posts (#1051)
    "twimg.com",
    // the GIF services of the Conversations field's GIF picker (#932,
    // gif_search.rs): what it sends is a link to their media servers
    // (media*.giphy.com, static*.klipy.com)
    "giphy.com",
    "klipy.com",
    // the other GIF service most shared in chat (#1551): its pages
    // (tenor.com/view/...) and media servers (media.tenor.com, c.tenor.com)
    "tenor.com",
];

/// URL shorteners and link hubs: the real destination is hidden.
const HIDDEN_DESTINATION: &[&str] = &[
    "bit.ly",
    "bitly.com",
    "tinyurl.com",
    "t.co",
    "goo.gl",
    "ow.ly",
    "is.gd",
    "v.gd",
    "buff.ly",
    "rebrand.ly",
    "cutt.ly",
    "shorturl.at",
    "tiny.cc",
    "rb.gy",
    "t.ly",
    "s.id",
    "bl.ink",
    "short.io",
    "shorte.st",
    "adf.ly",
    "linktr.ee",
    "lnk.bio",
    "grabify.link",
    "iplogger.org",
    "iplogger.com",
];

/// Throwaway tunnels and free instant hosting, where phishing pages are put
/// up and taken down within hours.
const THROWAWAY_HOSTING: &[&str] = &[
    "ngrok.io",
    "ngrok-free.app",
    "ngrok.app",
    "trycloudflare.com",
    "loca.lt",
    "serveo.net",
    "000webhostapp.com",
    "glitch.me",
    "repl.co",
    "workers.dev",
    "herokuapp.com",
];

/// Suffixes of names that only exist on a local network (RFC 6761, 6762,
/// 8375 and the common router defaults).
const LOCAL_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "localdomain",
    "lan",
    "home",
    "home.arpa",
    "internal",
    "intranet",
    "corp",
    "private",
];

/// Names a fake site borrows to look official (compared without dots and
/// dashes, so "second-life-login.example" is caught).
const IMPERSONATED: &[&str] = &["secondlife", "lindenlab", "firestorm", "auroraviewer", "tiliainc"];

/// Verdict on a web link.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Trust {
    /// A trusted domain: opens at once.
    Trusted,
    /// Anything else: opens after a warning (that can be turned off).
    Unknown,
    /// Hides or fakes its destination: always warns, with the reason.
    Dangerous(&'static str),
}

/// The host of an http(s) URL.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Host {
    /// Lowercase, IDNA names as their `xn--` form, no trailing dot.
    Name(String),
    Ip(IpAddr),
}

/// The host of an http(s) URL as the browser will read it (WHATWG URL
/// parsing): what follows a user info ("https://secondlife.com@evil.example"
/// goes to evil.example), a backslash read as "/", percent escapes decoded,
/// an IPv4 address in any of its notations, no port. Second value: the URL
/// hides something before an "@". `None` for another scheme or a URL the
/// browser would not open.
fn host_of(url: &str) -> Option<(Host, bool)> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let user_info = !parsed.username().is_empty() || parsed.password().is_some();
    let host = parsed.host_str()?;
    let host = if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Host::Ip(IpAddr::V6(v6.parse().ok()?))
    } else if let Ok(v4) = host.parse::<Ipv4Addr>() {
        Host::Ip(IpAddr::V4(v4))
    } else {
        Host::Name(host.trim_end_matches('.').to_owned())
    };
    Some((host, user_info))
}

/// Whether an address is on the internet: not this computer, a private or
/// link-local network, a shared (carrier) range, nor a reserved,
/// documentation or multicast one; an IPv4 address carried in IPv6
/// (mapped, compatible, NAT64, 6to4) is judged as that IPv4 address.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        // shared address space (RFC 6598)
        || (a == 100 && (64..128).contains(&b))
        // IETF protocol assignments, benchmarking
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19))
        || ip.is_documentation()
        || ip.is_multicast()
        // reserved, broadcast
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let embedded = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    // IPv4-compatible (::a.b.c.d) and NAT64 (64:ff9b::a.b.c.d)
    if s[..6] == [0; 6] || s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return is_public_v4(embedded(s[6], s[7]));
    }
    // 6to4 (2002:aabb:ccdd::)
    if s[0] == 0x2002 {
        return is_public_v4(embedded(s[1], s[2]));
    }
    !((s[0] & 0xfe00) == 0xfc00 // unique local
        || (s[0] & 0xffc0) == 0xfe80 // link-local
        || (s[0] & 0xffc0) == 0xfec0 // site-local (deprecated)
        || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentation
}

/// A name only a local network knows: a single label ("router"), or one
/// of `LOCAL_SUFFIXES`.
fn is_local_name(name: &str) -> bool {
    !name.contains('.') || in_list(name, LOCAL_SUFFIXES.iter().copied())
}

fn is_local(host: &Host) -> bool {
    match host {
        Host::Ip(ip) => !is_public_ip(*ip),
        Host::Name(name) => is_local_name(name),
    }
}

/// Whether an http(s) link leads to this computer or the local network
/// (see `is_public_ip`), as the browser reads its host. Link previews
/// never fetch such a link, nor follow a redirect to one.
pub fn leads_to_local_network(url: &str) -> bool {
    host_of(url).is_some_and(|(host, _)| is_local(&host))
}

fn in_list<'a>(host: &str, list: impl IntoIterator<Item = &'a str>) -> bool {
    list.into_iter()
        .any(|d| host == d || host.strip_suffix(d).is_some_and(|sub| sub.ends_with('.')))
}

/// Preferences › Protections › Trusted sites: domains the user trusts too, and
/// built-in ones they no longer trust (Aurora choice; the Legacy Viewer's
/// list is fixed).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TrustedSites {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl TrustedSites {
    /// The built-in list minus the removed domains, then the added ones.
    fn domains(&self) -> impl Iterator<Item = &str> {
        TRUSTED
            .iter()
            .copied()
            .filter(|d| !self.removed.iter().any(|r| r == d))
            .chain(self.added.iter().map(String::as_str))
    }

    /// Trusts a normalized domain: a built-in one the user had removed is
    /// trusted again, one already trusted is not added twice. False when
    /// nothing changes.
    pub fn add(&mut self, domain: &str) -> bool {
        if let Some(i) = self.removed.iter().position(|r| r == domain) {
            self.removed.remove(i);
            return true;
        }
        if TRUSTED.contains(&domain) || self.added.iter().any(|a| a == domain) {
            return false;
        }
        self.added.push(domain.to_owned());
        true
    }

    /// Stops or resumes trusting a built-in domain.
    pub fn set_built_in(&mut self, domain: &str, trusted: bool) {
        self.removed.retain(|r| r != domain);
        if !trusted {
            self.removed.push(domain.to_owned());
        }
    }
}

/// The built-in trusted domains, for Preferences.
pub fn built_in() -> &'static [&'static str] {
    TRUSTED
}

/// What the user typed, as a domain the list can hold: no scheme, path,
/// port or leading "www." / "*.", lowercase. None for anything that is not
/// a plain domain name (an IP address, spaces, a single label...).
pub fn normalize(input: &str) -> Option<String> {
    let mut domain = input.trim().to_ascii_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = domain.strip_prefix(scheme) {
            domain = rest.to_owned();
        }
    }
    let domain = domain.split(['/', '?', '#', ':']).next().unwrap_or("");
    let domain = domain.trim_end_matches('.');
    let domain = domain.strip_prefix("*.").unwrap_or(domain);
    let domain = domain.strip_prefix("www.").unwrap_or(domain);
    let labels: Vec<&str> = domain.split('.').collect();
    let valid = domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && !labels.last().is_some_and(|tld| tld.chars().all(|c| c.is_ascii_digit()));
    valid.then(|| domain.to_owned())
}

/// The Preferences list in use; read by every judge, from any thread.
static USER: std::sync::RwLock<Option<TrustedSites>> = std::sync::RwLock::new(None);

/// At start and after each change of Preferences.
pub fn configure(sites: &TrustedSites) {
    if let Ok(mut user) = USER.write() {
        *user = Some(sites.clone());
    }
}

static BUILT_IN_ONLY: TrustedSites = TrustedSites {
    added: Vec::new(),
    removed: Vec::new(),
};

/// Judge a web link (see the module documentation). Called for every link
/// drawn: the list is read in place, never copied.
pub fn classify(url: &str) -> Trust {
    match USER.read() {
        Ok(user) => classify_with(url, user.as_ref().unwrap_or(&BUILT_IN_ONLY)),
        Err(_) => classify_with(url, &BUILT_IN_ONLY),
    }
}

fn classify_with(url: &str, sites: &TrustedSites) -> Trust {
    let Some((host, user_info)) = host_of(url) else {
        return Trust::Unknown;
    };
    if user_info {
        return Trust::Dangerous(aurora_i18n::tr!("link-trust-the-address-hides-its-real-destination-behind-an"));
    }
    // before the trusted list: a site the user added cannot open the way
    // to the local network either
    if is_local(&host) {
        return Trust::Dangerous(aurora_i18n::tr!("link-trust-the-link-leads-to-this-computer-or-local-network"));
    }
    let host = match host {
        Host::Ip(_) => return Trust::Dangerous(aurora_i18n::tr!("link-trust-the-link-leads-to-an-ip-address-instead-of")),
        Host::Name(name) if name.is_empty() => return Trust::Unknown,
        Host::Name(name) => name,
    };
    if in_list(&host, sites.domains()) {
        return Trust::Trusted;
    }
    if !host.is_ascii() || host.split('.').any(|label| label.starts_with("xn--")) {
        return Trust::Dangerous(aurora_i18n::tr!(
            "link-trust-the-site-name-contains-characters-that-may-imitate-another"
        ));
    }
    if in_list(&host, HIDDEN_DESTINATION.iter().copied()) {
        return Trust::Dangerous(aurora_i18n::tr!("link-trust-shortened-link-the-real-destination-is-hidden"));
    }
    if in_list(&host, THROWAWAY_HOSTING.iter().copied()) {
        return Trust::Dangerous(aurora_i18n::tr!("link-trust-disposable-hosting-often-used-for-fake-login-pages"));
    }
    // "youtube.com.evil.example": a trusted name used as a mere subdomain
    if sites
        .domains()
        .any(|d| host.starts_with(&format!("{d}.")) || host.contains(&format!(".{d}.")))
    {
        return Trust::Dangerous(aurora_i18n::tr!("link-trust-the-name-starts-like-a-trusted-site-but-leads"));
    }
    let squashed: String = host.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if IMPERSONATED.iter().any(|name| squashed.contains(name)) {
        return Trust::Dangerous(aurora_i18n::tr!("link-trust-this-site-imitates-the-name-of-an-official-site"));
    }
    Trust::Unknown
}

/// The scheme of a link the viewer must not hand to the system, or `None`
/// for an http(s) one. Only web pages leave the viewer, as the links of a
/// notification: any other scheme (or none, read by the system as a file
/// path) could start a local program. `secondlife://` and app links have
/// their own handling and never get here. The scheme is returned in a form
/// fit for a log (letters, digits, `+-.`, at most 16 characters), never the
/// rest of the link.
pub fn refused_scheme(url: &str) -> Option<String> {
    let url = url.trim();
    match url.split_once("://") {
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") => None,
        _ => {
            let scheme = url.split(':').next().unwrap_or("");
            let fit = !scheme.is_empty()
                && scheme.len() <= 16
                && scheme.len() < url.len()
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
            Some(if fit { scheme.to_ascii_lowercase() } else { "(none)".to_owned() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_behavior_routes_the_same_urls_by_preference() {
        use crate::settings::BrowserBehavior::{BuiltInForAll, BuiltInForSecondLife, SystemForAll};

        // [system for all, Second Life links built in, built in for all]
        let cases = [
            ("chat link", "https://example.org/chat", [true, true, false]),
            ("Second Life page", "https://secondlife.com/destinations", [true, false, false]),
            ("Second Life subdomain", "https://my.secondlife.com/someone", [true, false, false]),
            ("grid status", "https://secondlife-status.statuspage.io/", [true, false, false]),
            ("upper-case host", "https://WWW.LindenLab.com/", [true, false, false]),
            ("other site", "https://video.example.net/watch", [true, true, false]),
            ("host look-alike", "https://secondlife.com.evil.example/", [true, true, false]),
            ("name ending alike", "https://notsecondlife.com/", [true, true, false]),
            ("unreadable address", "https://[bad", [true, true, false]),
        ];
        let behaviors = [SystemForAll, BuiltInForSecondLife, BuiltInForAll];
        for (name, url, expected) in cases {
            for (index, behavior) in behaviors.into_iter().enumerate() {
                assert_eq!(
                    should_use_system_browser(url, behavior, BrowserEntryPoint::Link),
                    expected[index],
                    "{name}, {behavior:?}"
                );
            }
        }
    }

    #[test]
    fn marketplace_links_follow_the_choice_and_its_window_stays_inside() {
        use crate::settings::BrowserBehavior::{BuiltInForAll, BuiltInForSecondLife, SystemForAll};
        let page = "https://marketplace.secondlife.com/p/item";
        // FSLoadSLMarketplace and loadURL: outside only with "system for all"
        for (behavior, outside) in [(SystemForAll, true), (BuiltInForSecondLife, false), (BuiltInForAll, false)] {
            assert_eq!(
                should_use_system_browser(page, behavior, BrowserEntryPoint::MarketplaceLink),
                outside,
                "{behavior:?}"
            );
            // the listings' open action, even off the Marketplace host
            assert!(!should_use_system_browser(
                "https://not-the-marketplace.example/",
                behavior,
                BrowserEntryPoint::MarketplaceWindow
            ));
        }
    }

    #[test]
    fn open_in_browser_buttons_always_leave_the_viewer() {
        use crate::settings::BrowserBehavior::{BuiltInForAll, BuiltInForSecondLife, SystemForAll};
        for behavior in [SystemForAll, BuiltInForSecondLife, BuiltInForAll] {
            for url in [
                "https://example.org",
                "https://secondlife.com/",
                "https://marketplace.secondlife.com/p/item",
            ] {
                assert!(
                    should_use_system_browser(url, behavior, BrowserEntryPoint::ExplicitExternal),
                    "{behavior:?}: {url}"
                );
            }
        }
    }

    #[test]
    fn trusted() {
        for url in [
            "https://secondlife.com/destinations",
            "http://community.secondlife.com/forums",
            "https://marketplace.secondlife.com:443/p/x",
            "https://WWW.LindenLab.com",
            "https://www.casperdns.com/",
            "https://www.casperpanel.com/",
            "https://casperlet.com/",
            "https://aurora-viewer.org",
            "https://www.aurora-viewer.org/downloads",
            "https://github.com/Aurora-Viewer/Aurora-Viewer/pulls",
            "https://gist.github.com/x",
            "https://secondlife.com./x",
            "https://www.youtube.com/watch?v=x",
            "https://youtu.be/x",
            "https://fr.wikipedia.org/wiki/Second_Life",
            "https://www.firestormviewer.org/downloads",
            "https://discord.gg/abc",
            "https://www.flickr.com/photos/x",
            // the image services of the previews (#923)
            "https://flic.kr/p/2nDemoAb",
            "https://live.staticflickr.com/65535/1_2_c.jpg",
            "https://i.redd.it/demo01picture.jpeg",
            "https://preview.redd.it/demo01picture.jpg?width=640",
            "https://prnt.sc/demo42",
            "https://image.prntscr.com/image/Dm0Aur0raExample.png",
            "https://i.gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9.png",
            "https://i.imgur.com/DemoAb1.jpeg",
            // the GIF services (#932, #1551)
            "https://media.giphy.com/media/demo/giphy.gif",
            "https://tenor.com/view/demo-wave-gif-12345678",
            "https://media.tenor.com/DemoAb1cD2eAAAAC/demo-wave.gif",
        ] {
            assert_eq!(classify(url), Trust::Trusted, "{url}");
        }
    }

    #[test]
    fn unknown() {
        for url in [
            "https://example.com",
            "https://github.io",
            "https://raw.githubusercontent.com/x",
            "https://notgithub.com",
            "https://nottenor.com/view/x",
            "https://docs.google.com/forms/x",
            "https://www.dropbox.com/s/x",
            "ftp://secondlife.com/x",
            "secondlife.com",
        ] {
            assert_eq!(classify(url), Trust::Unknown, "{url}");
        }
    }

    #[test]
    fn dangerous() {
        for url in [
            "https://secondlife.com@evil.example/x",
            "https://secondlife.com.evil.example/x",
            "https://evilsecondlife.com",
            "https://second-life-login.example",
            "https://firestorm-viewer.download",
            "https://auroraviewer.com",
            "https://aurora-viewer.org.evil.example",
            "https://aurora-viewer-login.example",
            "https://youtube.com.evil.example/watch",
            "https://bit.ly/abc",
            "https://linktr.ee/x",
            "https://abc.ngrok-free.app/login",
            "https://sl-login-demo.herokuapp.com/",
            "http://192.168.1.10/login",
            "http://[2001:db8::1]/x",
            "https://xn--secndlife-x9a.com",
            "https://sеcondlife.com",
        ] {
            assert!(matches!(classify(url), Trust::Dangerous(_)), "{url}");
        }
        // "https://evil.example\@secondlife.com": the browser goes to evil.example
        assert_eq!(classify("https://evil.example\\@secondlife.com"), Trust::Unknown);
    }

    #[test]
    fn this_computer_and_the_local_network_are_flagged_however_written() {
        let local = aurora_i18n::tr!("link-trust-the-link-leads-to-this-computer-or-local-network");
        for url in [
            "http://127.0.0.1/",
            "http://127.1/",
            "http://0x7f.0.0.1/",
            "http://0x7f000001/",
            "http://2130706433/",
            "http://017700000001/",
            "http://0177.0.0.1:8080/x",
            "http://0.0.0.0/",
            "http://10.0.0.1/",
            "http://172.16.5.4/",
            "http://192.168.1.1/admin",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/",
            "http://%31%32%37.0.0.1/",
            "http://[::1]/",
            "http://[0:0:0:0:0:0:0:1]:8080/",
            "http://[::]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:7f00:1]/",
            "http://[::ffff:192.168.1.1]/",
            "http://[64:ff9b::a00:1]/",
            "http://[2002:c0a8:101::]/",
            "http://[fe80::1]/",
            "http://[fd12:3456::1]/",
            "http://localhost/",
            "http://LOCALHOST:631/",
            "http://localhost./",
            "http://app.localhost/",
            "http://nas.local/",
            "http://router.lan/",
            "http://grafana.internal/",
            "http://printer.home.arpa/",
            "http://router/",
            "https://secondlife.com@127.0.0.1/",
        ] {
            assert!(matches!(classify(url), Trust::Dangerous(_)), "{url}");
            assert!(leads_to_local_network(url), "{url}");
            if !url.contains('@') {
                assert_eq!(classify(url), Trust::Dangerous(local), "{url}");
            }
        }
        // a site the user added cannot lift it
        let sites = TrustedSites {
            added: vec!["nas.lan".into()],
            removed: Vec::new(),
        };
        assert_eq!(classify_with("http://nas.lan/", &sites), Trust::Dangerous(local));
        // a public address is still flagged, as an IP address
        for url in ["http://8.8.8.8/", "http://[2606:4700::1111]/", "http://[::ffff:8.8.8.8]/"] {
            assert!(!leads_to_local_network(url), "{url}");
            assert_eq!(
                classify(url),
                Trust::Dangerous(aurora_i18n::tr!("link-trust-the-link-leads-to-an-ip-address-instead-of")),
                "{url}"
            );
        }
        for url in [
            "https://example.com/",
            "https://secondlife.com/",
            "https://localhost.example.com/",
            "https://local.example/",
            "ftp://127.0.0.1/",
            "not a link",
        ] {
            assert!(!leads_to_local_network(url), "{url}");
        }
    }

    #[test]
    fn public_addresses() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700::1111",
            "2a00:1450::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.31.255.255",
            "192.168.0.1",
            "169.254.1.1",
            "100.127.0.1",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "198.18.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fec0::1",
            "ff02::1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "2002:7f00:1::",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn the_users_list_adds_and_removes_trusted_sites_without_lifting_the_traps() {
        let sites = TrustedSites {
            added: vec!["my-club.example".into()],
            removed: vec!["x.com".into()],
        };
        assert_eq!(classify_with("https://my-club.example/events", &sites), Trust::Trusted);
        assert_eq!(classify_with("https://shop.my-club.example", &sites), Trust::Trusted);
        assert_eq!(classify_with("https://x.com/someone", &sites), Trust::Unknown);
        assert_eq!(classify_with("https://secondlife.com", &sites), Trust::Trusted);
        // An added site cannot be borrowed as a subdomain or behind an "@".
        for url in ["https://my-club.example.evil.test/login", "https://my-club.example@evil.test"] {
            assert!(matches!(classify_with(url, &sites), Trust::Dangerous(_)), "{url}");
        }
        assert_eq!(classify_with("https://my-club.example", &TrustedSites::default()), Trust::Unknown);
    }

    /// The list saved in Preferences holds what the user changed, not the
    /// whole list: domains built in later are trusted for everyone at once,
    /// except one the user had already removed.
    #[test]
    fn domains_built_in_later_reach_a_list_saved_before() {
        let saved: TrustedSites = serde_json::from_str(r#"{"added":["my-club.example"],"removed":["x.com"]}"#).unwrap();
        for url in ["https://prnt.sc/demo42", "https://i.redd.it/a.jpg", "https://flic.kr/p/2nDemoAb"] {
            assert_eq!(classify_with(url, &saved), Trust::Trusted, "{url}");
        }
        assert_eq!(classify_with("https://my-club.example", &saved), Trust::Trusted);
        assert_eq!(classify_with("https://x.com/someone", &saved), Trust::Unknown);
        let removed = TrustedSites {
            removed: vec!["prnt.sc".into()],
            ..TrustedSites::default()
        };
        assert_eq!(classify_with("https://prnt.sc/demo42", &removed), Trust::Unknown);
    }

    #[test]
    fn adding_a_site_never_duplicates_and_brings_back_a_removed_built_in_one() {
        let mut sites = TrustedSites::default();
        assert!(sites.add("my-club.example"));
        assert!(!sites.add("my-club.example"));
        assert!(!sites.add("secondlife.com"));
        sites.set_built_in("x.com", false);
        sites.set_built_in("x.com", false);
        assert_eq!(sites.removed, ["x.com"]);
        assert!(sites.add("x.com"));
        assert!(sites.removed.is_empty());
        assert_eq!(sites.added, ["my-club.example"]);
    }

    #[test]
    fn typed_sites_become_plain_domains() {
        for (typed, expected) in [
            ("My-Club.example", Some("my-club.example")),
            ("  https://www.my-club.example/events?x=1 ", Some("my-club.example")),
            ("http://shop.example:8080/", Some("shop.example")),
            ("*.example.org", Some("example.org")),
            ("example.org.", Some("example.org")),
            ("example", None),
            ("192.168.1.10", None),
            ("exa mple.org", None),
            ("-bad.example", None),
            ("ex@mple.org", None),
            ("", None),
        ] {
            assert_eq!(normalize(typed).as_deref(), expected, "{typed:?}");
        }
    }

    #[test]
    fn only_web_pages_leave_the_viewer() {
        for url in [
            "https://example.org/a?b",
            "http://example.org",
            "HTTPS://Example.org",
            " https://example.org ",
        ] {
            assert_eq!(refused_scheme(url), None, "{url:?}");
        }
        for (url, scheme) in [
            ("file:///C:/Windows/System32/calc.exe", "file"),
            ("FILE://server/share/x.exe", "file"),
            ("ms-settings:privacy", "ms-settings"),
            ("javascript:alert(1)", "javascript"),
            ("mailto:someone@example.org", "mailto"),
            ("search-ms:query=x", "search-ms"),
            ("C:\\Windows\\notepad.exe", "c"),
            // no scheme at all: the system would read a path
            ("example.org/page", "(none)"),
            ("\\\\server\\share\\x.exe", "(none)"),
            ("", "(none)"),
            // nothing of the rest of the link reaches the log
            ("weird scheme://secret/token", "(none)"),
            ("averyveryverylongscheme://x", "(none)"),
        ] {
            assert_eq!(refused_scheme(url).as_deref(), Some(scheme), "{url:?}");
        }
    }
}
