//! Hosted video cards use a provider's own player and public oEmbed metadata.
//! Build both addresses from the video ID, never from returned HTML.

use reqwest::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Hosted {
    Streamable(String),
    Vimeo { id: String, hash: Option<String> },
}

fn id_ok(id: &str, max: usize) -> bool {
    !id.is_empty() && id.len() <= max && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl Hosted {
    pub(super) fn of(url: &Url) -> Option<Self> {
        let parts: Vec<_> = url.path().trim_end_matches('/').strip_prefix('/')?.split('/').collect();
        let host = url.host_str()?;
        let (id, path_hash) = match (host, parts.as_slice()) {
            ("streamable.com" | "www.streamable.com", [id] | ["e", id] | ["o", id]) if id_ok(id, 16) => {
                // A video shortcode is at least three characters. Reject site navigation.
                if id.len() < 3
                    || matches!(
                        *id,
                        "login"
                            | "signup"
                            | "upload"
                            | "pricing"
                            | "blog"
                            | "about"
                            | "terms"
                            | "privacy"
                            | "dashboard"
                            | "videos"
                            | "features"
                            | "help"
                            | "tools"
                            | "contact"
                            | "register"
                    )
                {
                    return None;
                }
                return Some(Self::Streamable((*id).to_owned()));
            }
            ("vimeo.com" | "www.vimeo.com", [id] | ["channels", _, id] | ["groups", _, "videos", id])
            | ("player.vimeo.com", ["video", id]) => (*id, None),
            ("vimeo.com" | "www.vimeo.com", [id, hash]) => (*id, Some(*hash)),
            _ => return None,
        };
        if !id_ok(id, 12) || !id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let hash = path_hash
            .map(str::to_owned)
            .or_else(|| url.query_pairs().find(|(k, _)| k == "h").map(|(_, v)| v.into_owned()));
        if hash.as_deref().is_some_and(|h| !id_ok(h, 32)) {
            return None;
        }
        Some(Self::Vimeo { id: id.to_owned(), hash })
    }

    pub(super) fn page(&self) -> String {
        match self {
            Self::Streamable(id) => format!("https://streamable.com/{id}"),
            Self::Vimeo { id, hash: Some(hash) } => format!("https://vimeo.com/{id}/{hash}"),
            Self::Vimeo { id, hash: None } => format!("https://vimeo.com/{id}"),
        }
    }

    pub(super) fn player(&self) -> String {
        match self {
            Self::Streamable(id) => format!("https://streamable.com/e/{id}?autoplay=1"),
            Self::Vimeo { id, hash } => {
                let hash = hash.as_ref().map(|h| format!("h={h}&")).unwrap_or_default();
                format!("https://player.vimeo.com/video/{id}?{hash}autoplay=1&muted=0")
            }
        }
    }

    pub(super) fn oembed(&self) -> String {
        let endpoint = match self {
            Self::Streamable(_) => "https://api.streamable.com/oembed.json",
            Self::Vimeo { .. } => "https://vimeo.com/api/oembed.json",
        };
        Url::parse_with_params(endpoint, [("url", self.page())])
            .expect("valid provider endpoint")
            .into()
    }

    pub(super) fn poster(&self, value: &str) -> Option<String> {
        // Streamable also returns protocol-relative thumbnails.
        let url = if value.starts_with("//") {
            format!("https:{value}")
        } else {
            value.to_owned()
        };
        let parsed = Url::parse(&url).ok()?;
        if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() || parsed.port().is_some() {
            return None;
        }
        let host = parsed.host_str()?;
        let permitted = match self {
            Self::Streamable(_) => matches!(
                host,
                "images.streamable.com" | "cdn-cf-east.streamable.com" | "cdn-cf-west.streamable.com"
            ),
            Self::Vimeo { .. } => host == "i.vimeocdn.com",
        };
        permitted.then_some(url)
    }
}
