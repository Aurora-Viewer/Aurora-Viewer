//! Hosted video cards use a provider's own player and public oEmbed metadata.
//! Build both addresses from the video ID, never from returned HTML.

use reqwest::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Hosted {
    Streamable(String),
    Vimeo { id: String, hash: Option<String> },
    TikTok { account: String, id: String },
    Bilibili { id: String, part: u32 },
    Framatube(String),
    InstagramReel(String),
    FacebookReel(String),
    TwitchClip(String),
    TwitchVideo(String),
}

fn id_ok(id: &str, max: usize) -> bool {
    !id.is_empty() && id.len() <= max && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn digits(id: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

fn tiktok_account(account: &str) -> bool {
    !account.is_empty() && account.len() <= 24 && account.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.'))
}

fn bilibili_id(id: &str) -> bool {
    (id.starts_with("BV") && id.len() == 12 && id_ok(id, 12)) || id.strip_prefix("av").is_some_and(|aid| digits(aid, 1, 20))
}

fn instagram_id(id: &str) -> bool {
    (5..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn peertube_id(id: &str) -> bool {
    (id.len() == 22 && id_ok(id, 22)) || (id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
}

fn twitch_clip_id(id: &str) -> bool {
    (8..=100).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        && !matches!(id, "dashboard" | "directory")
}

impl Hosted {
    pub(super) fn requires_player_click(&self) -> bool {
        matches!(self, Self::InstagramReel(_) | Self::FacebookReel(_))
    }
    pub(super) fn minimum_width(&self) -> f32 {
        match self {
            // Twitch requires at least 400 × 300 CSS pixels.
            Self::TwitchClip(_) | Self::TwitchVideo(_) => 534.0,
            _ => 0.0,
        }
    }
    pub(super) fn aspect(&self) -> Option<f32> {
        match self {
            Self::TikTok { .. } | Self::InstagramReel(_) | Self::FacebookReel(_) => Some(9.0 / 16.0),
            _ => None,
        }
    }

    pub(super) fn of(url: &Url) -> Option<Self> {
        let parts: Vec<_> = url.path().trim_end_matches('/').strip_prefix('/')?.split('/').collect();
        let host = url.host_str()?;
        match (host, parts.as_slice()) {
            ("instagram.com" | "www.instagram.com", ["reel", id]) if instagram_id(id) => {
                return Some(Self::InstagramReel((*id).to_owned()));
            }
            ("facebook.com" | "www.facebook.com" | "m.facebook.com", ["reel", id]) if digits(id, 8, 20) => {
                return Some(Self::FacebookReel((*id).to_owned()));
            }
            ("framatube.org", ["w", id] | ["videos", "watch" | "embed", id]) if peertube_id(id) => {
                return Some(Self::Framatube((*id).to_owned()));
            }
            ("clips.twitch.tv", [id]) | ("www.twitch.tv" | "twitch.tv" | "m.twitch.tv", [_, "clip", id]) if twitch_clip_id(id) => {
                return Some(Self::TwitchClip((*id).to_owned()));
            }
            ("www.twitch.tv" | "twitch.tv" | "m.twitch.tv", ["videos", id]) if digits(id, 1, 20) => {
                return Some(Self::TwitchVideo((*id).to_owned()));
            }
            ("tiktok.com" | "www.tiktok.com" | "m.tiktok.com", [account, "video", id])
                if account.strip_prefix('@').is_some_and(tiktok_account) && digits(id, 15, 20) =>
            {
                return Some(Self::TikTok {
                    account: (*account).to_owned(),
                    id: (*id).to_owned(),
                });
            }
            ("www.bilibili.com" | "bilibili.com" | "m.bilibili.com", ["video", id]) if bilibili_id(id) => {
                let part = url
                    .query_pairs()
                    .find(|(key, _)| key == "p")
                    .and_then(|(_, value)| value.parse::<u32>().ok())
                    .filter(|&part| part > 0)
                    .unwrap_or(1);
                return Some(Self::Bilibili {
                    id: (*id).to_owned(),
                    part,
                });
            }
            _ => {}
        }
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
            Self::InstagramReel(id) => format!("https://www.instagram.com/reel/{id}/"),
            Self::FacebookReel(id) => format!("https://www.facebook.com/reel/{id}/"),
            Self::Framatube(id) => format!("https://framatube.org/w/{id}"),
            Self::TwitchClip(id) => format!("https://clips.twitch.tv/{id}"),
            Self::TwitchVideo(id) => format!("https://www.twitch.tv/videos/{id}"),
            Self::TikTok { account, id } => format!("https://www.tiktok.com/{account}/video/{id}"),
            Self::Bilibili { id, part } => format!("https://www.bilibili.com/video/{id}?p={part}"),
            Self::Streamable(id) => format!("https://streamable.com/{id}"),
            Self::Vimeo { id, hash: Some(hash) } => format!("https://vimeo.com/{id}/{hash}"),
            Self::Vimeo { id, hash: None } => format!("https://vimeo.com/{id}"),
        }
    }

    pub(super) fn player(&self) -> String {
        match self {
            Self::InstagramReel(id) => format!("https://www.instagram.com/reel/{id}/embed/"),
            Self::FacebookReel(id) => {
                format!("https://www.facebook.com/plugins/video.php?href=https%3A%2F%2Fwww.facebook.com%2Freel%2F{id}%2F&autoplay=false")
            }
            Self::Framatube(id) => format!("https://framatube.org/videos/embed/{id}?autoplay=1&muted=0&p2p=0"),
            Self::TwitchClip(id) => format!("https://clips.twitch.tv/embed?clip={id}&parent=127.0.0.1&autoplay=true&muted=false"),
            Self::TwitchVideo(id) => format!("https://player.twitch.tv/?video=v{id}&parent=127.0.0.1&autoplay=true&muted=false"),
            Self::TikTok { id, .. } => format!("https://www.tiktok.com/player/v1/{id}?autoplay=1&force_muted=0&rel=0"),
            Self::Bilibili { id, part } => {
                let id = id
                    .strip_prefix("av")
                    .map_or_else(|| format!("bvid={id}"), |aid| format!("aid={aid}"));
                format!("https://player.bilibili.com/player.html?{id}&p={part}&autoplay=1&muted=0")
            }
            Self::Streamable(id) => format!("https://streamable.com/e/{id}?autoplay=1"),
            Self::Vimeo { id, hash } => {
                let hash = hash.as_ref().map(|h| format!("h={h}&")).unwrap_or_default();
                format!("https://player.vimeo.com/video/{id}?{hash}autoplay=1&muted=0")
            }
        }
    }

    pub(super) fn oembed(&self) -> Option<String> {
        let endpoint = match self {
            Self::Streamable(_) => "https://api.streamable.com/oembed.json",
            Self::Vimeo { .. } => "https://vimeo.com/api/oembed.json",
            Self::TikTok { .. } => "https://www.tiktok.com/oembed",
            Self::Framatube(_) => "https://framatube.org/services/oembed",
            Self::Bilibili { .. } | Self::TwitchClip(_) | Self::TwitchVideo(_) | Self::InstagramReel(_) | Self::FacebookReel(_) => {
                return None;
            }
        };
        Url::parse_with_params(endpoint, [("url", self.page())]).ok().map(String::from)
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
            Self::Framatube(_) => host == "framatube.org" && parsed.path().starts_with("/static/"),
            Self::TikTok { .. } => host.ends_with(".tiktokcdn.com") || host.ends_with(".tiktokcdn-us.com"),
            Self::Bilibili { .. } | Self::TwitchClip(_) | Self::TwitchVideo(_) | Self::InstagramReel(_) | Self::FacebookReel(_) => false,
        };
        permitted.then_some(url)
    }
}
