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
    "auroraviewer.com",
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

fn in_list(host: &str, list: &[&str]) -> bool {
    list.iter()
        .any(|d| host == *d || host.strip_suffix(d).is_some_and(|sub| sub.ends_with('.')))
}

/// Judge a web link (see the module documentation).
pub fn classify(url: &str) -> Trust {
    let Some((host, user_info)) = host_of(url) else {
        return Trust::Unknown;
    };
    if user_info {
        return Trust::Dangerous("L'adresse cache sa vraie destination derrière un « @ ».");
    }
    if host.is_empty() {
        return Trust::Unknown;
    }
    if in_list(&host, TRUSTED) {
        return Trust::Trusted;
    }
    if !host.is_ascii() || host.split('.').any(|label| label.starts_with("xn--")) {
        return Trust::Dangerous("Le nom du site contient des caractères qui peuvent en imiter un autre.");
    }
    if host.contains(':') || host.split('.').all(|label| label.chars().all(|c| c.is_ascii_digit())) {
        return Trust::Dangerous("Le lien mène à une adresse IP au lieu d'un nom de site.");
    }
    if in_list(&host, HIDDEN_DESTINATION) {
        return Trust::Dangerous("Raccourcisseur de lien : la vraie destination est cachée.");
    }
    if in_list(&host, THROWAWAY_HOSTING) {
        return Trust::Dangerous("Hébergement jetable, très utilisé pour les fausses pages de connexion.");
    }
    // "youtube.com.evil.example": a trusted name used as a mere subdomain
    if TRUSTED
        .iter()
        .any(|d| host.starts_with(&format!("{d}.")) || host.contains(&format!(".{d}.")))
    {
        return Trust::Dangerous("Le nom commence comme un site de confiance, mais le site est un autre.");
    }
    let squashed: String = host.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if IMPERSONATED.iter().any(|name| squashed.contains(name)) {
        return Trust::Dangerous("Ce site reprend le nom d'un site officiel sans en être un.");
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
            "https://auroraviewer.com",
            "https://github.com/Aurora-Viewer/Aurora-Viewer/pulls",
            "https://gist.github.com/x",
            "https://secondlife.com./x",
            "https://www.youtube.com/watch?v=x",
            "https://youtu.be/x",
            "https://fr.wikipedia.org/wiki/Second_Life",
            "https://www.firestormviewer.org/downloads",
            "https://discord.gg/abc",
            "https://www.flickr.com/photos/x",
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
}
