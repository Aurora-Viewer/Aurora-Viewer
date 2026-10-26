//! How far a web link of a chat or profile text can be trusted: a green
//! check for well-known sites, a red cross for links that hide or fake their
//! destination, an amber triangle for the rest. Judged locally on the URL
//! alone: nothing is sent anywhere.
//!
//! The trusted domains start from Firestorm's (LLUrlEntrySecondlifeURL and
//! LLUrlEntryFirestormURL, indra/llui/llurlentry.cpp); the red cross is an
//! Aurora addition.

/// Trusted domains, their subdomains included. Hosts of pages or files
/// anyone can publish (github.io, githubusercontent.com, sites.google.com,
/// dropbox.com...) stay out: a fake login page can live there.
const TRUSTED: &[&str] = &[
    // Second Life and Linden Lab
    "secondlife.com",
    "lindenlab.com",
    "tilia-inc.com",
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

/// The host of an http(s) URL, lowercased, as the browser will read it:
/// what follows a user info ("https://secondlife.com@evil.example" goes to
/// evil.example), a backslash read as "/", no port, no trailing dot.
/// Second value: the URL hides something before an "@".
fn host_of(url: &str) -> Option<(String, bool)> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or("");
    let user_info = authority.contains('@');
    let host = authority.rsplit('@').next().unwrap_or("");
    // [IPv6]:port
    let host = if let Some(v6) = host.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    Some((host.trim_end_matches('.').to_lowercase(), user_info))
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
    if host.is_empty() {
        return Trust::Unknown;
    }
    if in_list(&host, sites.domains()) {
        return Trust::Trusted;
    }
    if !host.is_ascii() || host.split('.').any(|label| label.starts_with("xn--")) {
        return Trust::Dangerous(aurora_i18n::tr!(
            "link-trust-the-site-name-contains-characters-that-may-imitate-another"
        ));
    }
    if host.contains(':') || host.split('.').all(|label| label.chars().all(|c| c.is_ascii_digit())) {
        return Trust::Dangerous(aurora_i18n::tr!("link-trust-the-link-leads-to-an-ip-address-instead-of"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted() {
        for url in [
            "https://secondlife.com/destinations",
            "http://community.secondlife.com/forums",
            "https://marketplace.secondlife.com:443/p/x",
            "https://WWW.LindenLab.com",
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
}
