//! The pages of image services whose links get the picture they show
//! (#923): a link to a Gyazo, Flickr, Imgur, Lightshot or Reddit page, or to
//! a Bluesky or X post (#1051), not to the picture's file, still gets a
//! picture card. An Aurora feature, like the rest of the previews.
//!
//! Each page leads to its picture by the cleanest way its service offers:
//! * Gyazo and Flickr: their oEmbed endpoints (documented, no key); a Gyazo
//!   video capture leads to its file instead, played in a video card (#983);
//! * Imgur and Lightshot: the page's `og:image`, with video metadata preferred
//!   on an Imgur single post. GIPHY and Tenor use animated image renditions
//!   on their own CDNs. Only the page head is read and no API key is needed;
//! * Reddit: the post's Atom feed, its first entry's `media:thumbnail`
//!   (Reddit's JSON answers 403 to clients without an account, its post
//!   pages a script challenge; the feed is public). A `reddit.com/media`
//!   link names its picture itself.
//! * Bluesky: the post from its public API (`app.bsky.feed.getPostThread`,
//!   documented, no account), its first picture or its video's thumbnail;
//! * X: the post as X's own embed widget reads it (`tweet-result` of
//!   `cdn.syndication.twimg.com`): X's documented oEmbed gives no picture
//!   and its API needs a paid key. Undocumented, so it may stop answering:
//!   the link then stays a link.
//!
//! * the user's own picture sites (#1063, Preferences › Chat): a
//!   self-hosted cloud or gallery (Nextcloud, Immich, Pixelfed…) whose
//!   share pages name their picture in `og:image`, as Nextcloud's do
//!   (`/s/<token>` names `/s/<token>/preview`, DefaultPublicShareTemplateProvider
//!   of its files_sharing app). Listing a site does not trust it: its links
//!   still load on a click unless it is a trusted site too.
//!
//! The picture found must be on the service's own image servers
//! (`picture_host_ok`), or on the listed site: a page cannot send the viewer
//! anywhere else. A post its network marks sensitive (X's
//! `possibly_sensitive`, Bluesky's adult and graphic labels) is blurred
//! until clicked.

use reqwest::Url;

/// A service whose pages lead to a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Gyazo,
    Flickr,
    Imgur,
    Giphy,
    Tenor,
    Lightshot,
    Reddit,
    Bluesky,
    X,
    /// A site of the user's list of picture sites (`set_sites`).
    Site,
}

/// The user's picture sites, normalized domains (`set_sites`); read by
/// every link judged, from any thread.
static SITES: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

/// At start and after each change of Preferences.
pub(super) fn set_sites(sites: Vec<String>) {
    if let Ok(mut list) = SITES.write() {
        *list = sites;
    }
}

/// `host` is one of the user's picture sites or one of their subdomains.
fn on_site(host: &str) -> bool {
    SITES.read().is_ok_and(|sites| {
        sites
            .iter()
            .any(|d| host == d || host.strip_suffix(d.as_str()).is_some_and(|sub| sub.ends_with('.')))
    })
}

/// A page of one of the user's picture sites: https, not the site's root,
/// read for its `og:image`.
fn site_page(parsed: &Url) -> Option<(Service, Step)> {
    let host = parsed.host_str()?.trim_end_matches('.').to_lowercase();
    if parsed.scheme() != "https" || !parsed.username().is_empty() || segments(parsed).is_empty() || !on_site(&host) {
        return None;
    }
    let mut page = parsed.clone();
    page.set_fragment(None);
    Some((Service::Site, Step::Head(page.into())))
}

/// How a page leads to its picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    /// The link names its picture (`reddit.com/media?url=`): no request.
    Direct(String),
    /// A JSON oEmbed answer, its `url` when its `type` is `photo`.
    OEmbed(String),
    /// The page itself, its `og:image`.
    Head(String),
    /// Public sharing metadata from a recognized video publisher.
    Publisher(String),
    /// Reddit's Atom feed of a post.
    Feed { feed: String, page: String, share_image: String },
    /// A Bluesky or X post as JSON (`from_post`).
    Post(String),
}

/// The host of a link without the prefixes of the services' mobile and
/// former sites.
fn bare_host(parsed: &Url) -> Option<String> {
    let host = parsed.host_str()?.trim_end_matches('.').to_lowercase();
    let mut host = host.as_str();
    for prefix in ["www.", "m.", "mobile.", "old.", "new.", "np."] {
        host = host.strip_prefix(prefix).unwrap_or(host);
    }
    Some(host.to_owned())
}

fn segments(parsed: &Url) -> Vec<&str> {
    parsed.path_segments().into_iter().flatten().filter(|s| !s.is_empty()).collect()
}

/// A Gyazo picture's id: 32 hexadecimal digits.
fn gyazo_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// An id of letters and digits, `min` to `max` long.
fn alnum(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A Reddit post's id: base 36, lowercase.
fn reddit_id(s: &str) -> bool {
    (4..=10).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// A Bluesky account in a post's link: its handle (a domain name) or its
/// DID.
fn bluesky_actor(s: &str) -> bool {
    let handle = (3..=253).contains(&s.len()) && s.contains('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let did = s.len() <= 2048
        && s.strip_prefix("did:")
            .is_some_and(|rest| rest.contains(':') && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b".:_-%".contains(&b)));
    handle || did
}

/// A Bluesky record key (a post's id, 13 characters in practice).
fn bluesky_key(s: &str) -> bool {
    (1..=512).contains(&s.len()) && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:~-".contains(&b))
}

/// An X post's id: a number.
fn x_id(s: &str) -> Option<u64> {
    s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse().ok())?
}

/// The token X's embed widget sends with a post's id (as react-tweet
/// computes it): `(id / 1e15 * π).toString(36)` without its zeros and its
/// dot. X asks for one; its value is not checked today, but it may be.
pub(super) fn x_token(id: u64) -> String {
    js_base36(id as f64 / 1e15 * std::f64::consts::PI)
        .chars()
        .filter(|&c| c != '0' && c != '.')
        .collect()
}

/// A positive number written in base 36 as JavaScript writes it
/// (`Number.prototype.toString(36)`): the fraction's digits stop as soon as
/// they name the number (V8's DoubleToRadixCString, src/numbers/conversions.cc).
fn js_base36(value: f64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut integer = value.floor();
    let mut fraction = value - integer;
    let mut delta = (0.5 * (f64::from_bits(value.to_bits() + 1) - value)).max(f64::from_bits(1));
    let mut digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= 36.0;
            delta *= 36.0;
            let digit = fraction as usize;
            digits.push(digit as u8);
            fraction -= digit as f64;
            if (fraction > 0.5 || (fraction == 0.5 && digit & 1 == 1)) && fraction + delta > 1.0 {
                // round up: the last digit that is not 'z' gains one
                loop {
                    match digits.pop() {
                        None => {
                            integer += 1.0;
                            break;
                        }
                        Some(d) if d + 1 < 36 => {
                            digits.push(d + 1);
                            break;
                        }
                        Some(_) => {}
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    let mut whole = Vec::new();
    let mut n = integer as u64;
    loop {
        whole.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    whole.reverse();
    let mut out = String::from_utf8(whole).unwrap_or_default();
    if !digits.is_empty() {
        out.push('.');
        out.extend(digits.iter().map(|&d| DIGITS[d as usize] as char));
    }
    out
}

/// Imgur's own pages, whose names look like a picture's id.
const IMGUR_PAGES: &[&str] = &[
    "upload", "signin", "register", "about", "apps", "blog", "rules", "emerald", "privacy", "search", "random", "hot", "new", "top",
    "user", "account", "settings", "terms", "help", "jobs", "press", "ads", "tos", "vidgif", "memegen",
];

/// The service whose page `url` is, with how it leads to its picture.
pub(super) fn recognize(url: &str) -> Option<(Service, Step)> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = bare_host(&parsed)?;
    let segs = segments(&parsed);
    let with = |base: &str, pairs: &[(&str, &str)]| Url::parse_with_params(base, pairs).ok().map(String::from);
    match host.as_str() {
        "gyazo.com" => {
            let id = segs.first().filter(|id| gyazo_id(id) && segs.len() == 1)?;
            let page = format!("https://gyazo.com/{}", id.to_lowercase());
            Some((
                Service::Gyazo,
                Step::OEmbed(with("https://api.gyazo.com/api/oembed", &[("url", &page)])?),
            ))
        }
        "flickr.com" | "flic.kr" => {
            let photo = match (host.as_str(), segs.as_slice()) {
                // flickr.com/photos/<user>/<id>, with /in/<album> or /sizes/… after
                ("flickr.com", ["photos", user, id, ..]) if !user.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => {
                    format!("https://www.flickr.com/photos/{user}/{id}/")
                }
                ("flic.kr", ["p", id]) if alnum(id, 4, 16) => format!("https://flic.kr/p/{id}"),
                _ => return None,
            };
            let endpoint = with(
                "https://www.flickr.com/services/oembed",
                &[("format", "json"), ("maxwidth", "800"), ("maxheight", "800"), ("url", &photo)],
            )?;
            Some((Service::Flickr, Step::OEmbed(endpoint)))
        }
        "imgur.com" => {
            let page = match segs.as_slice() {
                [id] if alnum(id, 5, 8) && !IMGUR_PAGES.contains(&id.to_lowercase().as_str()) => format!("https://imgur.com/{id}"),
                ["a" | "gallery", slug] if slug.len() <= 120 && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') => {
                    format!("https://imgur.com/{}/{slug}", segs[0])
                }
                ["t", tag, id] if !tag.is_empty() && alnum(id, 5, 8) => format!("https://imgur.com/t/{tag}/{id}"),
                _ => return None,
            };
            Some((Service::Imgur, Step::Head(page)))
        }
        "giphy.com" => {
            let ["gifs" | "stickers", slug] = segs.as_slice() else {
                return None;
            };
            let id = slug.rsplit('-').next()?;
            if !alnum(id, 6, 32) || slug.len() > 240 {
                return None;
            }
            Some((Service::Giphy, Step::Head(format!("https://giphy.com/{}/{slug}", segs[0]))))
        }
        "tenor.com" => {
            let ["view", slug] = segs.as_slice() else {
                return None;
            };
            let (_, id) = slug.rsplit_once('-')?;
            if !(1..=16).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_digit()) || slug.len() > 240 {
                return None;
            }
            Some((Service::Tenor, Step::Head(format!("https://tenor.com/view/{slug}"))))
        }
        "prnt.sc" | "prntscr.com" => {
            let id = segs.first().filter(|id| segs.len() == 1 && alnum(id, 5, 12))?;
            Some((Service::Lightshot, Step::Head(format!("https://prnt.sc/{}", id.to_lowercase()))))
        }
        "reddit.com" | "redd.it" => {
            if host == "reddit.com" && segs.as_slice() == ["media"] {
                let inner = parsed.query_pairs().find(|(k, _)| k == "url").map(|(_, v)| v.into_owned())?;
                return picture_host_ok(Service::Reddit, &inner).then_some((Service::Reddit, Step::Direct(inner)));
            }
            let id = match (host.as_str(), segs.as_slice()) {
                ("redd.it", [id]) => id,
                ("reddit.com", ["r" | "user" | "u", _, "comments", id, ..]) => id,
                ("reddit.com", ["comments", id, ..]) => id,
                _ => return None,
            };
            reddit_id(id).then(|| {
                let page = if host == "redd.it" {
                    format!("https://www.reddit.com/comments/{id}/")
                } else {
                    format!("https://www.reddit.com{}", parsed.path())
                };
                (
                    Service::Reddit,
                    Step::Feed {
                        feed: format!("https://www.reddit.com/comments/{id}/.rss?limit=1"),
                        page,
                        share_image: format!("https://share.redd.it/preview/post/{id}?format=webp"),
                    },
                )
            })
        }
        "bsky.app" => {
            // bsky.app/profile/<handle or DID>/post/<key>; getPostThread
            // takes a handle where getPosts wants a DID
            let ["profile", actor, "post", key] = segs.as_slice() else {
                return None;
            };
            if !bluesky_actor(actor) || !bluesky_key(key) {
                return None;
            }
            let uri = format!("at://{actor}/app.bsky.feed.post/{key}");
            let endpoint = with(
                "https://public.api.bsky.app/xrpc/app.bsky.feed.getPostThread",
                &[("uri", &uri), ("depth", "0"), ("parentHeight", "0")],
            )?;
            Some((Service::Bluesky, Step::Post(endpoint)))
        }
        "x.com" | "twitter.com" => {
            // <user>/status/<id>, with /photo/1 or /video/1 after;
            // i/status/<id> and i/web/status/<id> have no user
            let id = match segs.as_slice() {
                ["i", "web", "status", id, ..] => id,
                [user, "status" | "statuses", id, ..]
                    if (1..=15).contains(&user.len()) && user.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') =>
                {
                    id
                }
                _ => return None,
            };
            let id = x_id(id)?;
            Some((
                Service::X,
                Step::Post(format!(
                    "https://cdn.syndication.twimg.com/tweet-result?id={id}&lang=en&token={}",
                    x_token(id)
                )),
            ))
        }
        _ => publisher_page(&parsed).or_else(|| site_page(&parsed)),
    }
}

#[test]
fn publisher_shares_use_public_pages_and_publisher_images() {
    for url in [
        "https://www.snapchat.com/spotlight/1234aBcDe",
        "https://www.dailymotion.com/video/x84sh87",
        "https://www.arte.tv/en/videos/123456-000-A/example/",
        "https://www.bbc.co.uk/iplayer/episode/m001jjq0/example",
        "https://tver.jp/episodes/ep12345",
        "https://www.nicovideo.jp/watch/sm9",
        "https://tv.naver.com/v/12345678",
        "https://chzzk.naver.com/video/12345",
        "https://vod.sooplive.co.kr/player/12345",
        "https://vod.sooplive.com/player/12345",
        "https://tv.kakao.com/channel/123/cliplink/456",
        "https://www.douyin.com/video/12345678",
        "https://www.kuaishou.com/short-video/Abc_123",
        "https://www.xiaohongshu.com/explore/123abc",
        "https://v.qq.com/x/cover/series123/video456.html",
        "https://www.iqiyi.com/v_example.html",
        "https://v.youku.com/v_show/id_XYZ=.html",
        "https://www.mgtv.com/b/123456/7890123.html",
    ] {
        let input = format!("{url}?tracking=example#fragment");
        assert!(
            matches!(recognize(&input), Some((Service::Site, Step::Publisher(page))) if page == url),
            "{url}"
        );
    }
    for url in [
        "https://www.dailymotion.com/",
        "https://dailymotion.com.example.org/video/x84sh87",
        "https://www.dailymotion.com/video/x84sh87%2Fextra",
        "http://www.nicovideo.jp/watch/sm9",
    ] {
        assert!(!matches!(recognize(url), Some((_, Step::Publisher(_)))), "{url}");
    }
    let html = r#"<meta property="og:image" content="https://dmcdn.net.example.org/bad"><meta name="twitter:image" content="https://s1.dmcdn.net/good">"#;
    assert_eq!(
        publisher_poster("https://www.dailymotion.com/video/x84sh87", html).as_deref(),
        Some("https://s1.dmcdn.net/good"),
    );
}

const VIEWER_CONFIRMATION_DOMAINS: &[&str] = &["pornhub.com", "xvideos.com", "onlyfans.com", "redgifs.com"];

pub(super) fn adult_media_site(host: &str) -> bool {
    VIEWER_CONFIRMATION_DOMAINS.iter().any(|domain| super::in_domain(host, domain))
}

#[test]
fn adult_media_pages_keep_only_the_parameters_needed_to_share_the_item() {
    let cases = [
        (
            "https://www.pornhub.com/view_video.php?opaque=example&viewkey=phabcdef#fragment",
            "https://www.pornhub.com/view_video.php?viewkey=phabcdef",
        ),
        (
            "https://www.xvideos.com/video.example/title?tracking=example",
            "https://www.xvideos.com/video.example/title",
        ),
        (
            "https://onlyfans.com/123456789/example?tracking=example",
            "https://onlyfans.com/123456789/example",
        ),
        (
            "https://www.redgifs.com/watch/sample?tracking=example",
            "https://www.redgifs.com/watch/sample",
        ),
    ];
    for (url, expected) in cases {
        assert!(matches!(recognize(url), Some((Service::Site, Step::Publisher(page))) if page == expected));
    }
    assert!(!adult_media_site("redgifs.com.example.org"));
    assert!(adult_media_site("media.redgifs.com"));
    assert!(recognize("https://onlyfans.com/example").is_none());
}

#[test]
fn mainland_mobile_links_map_to_the_public_watch_page() {
    let cases = [
        (
            "https://www.iesdouyin.com/share/video/12345678/?tracking=1",
            "https://www.douyin.com/video/12345678",
        ),
        (
            "https://m.v.qq.com/x/m/play?vid=v3308e31l25&tracking=1",
            "https://v.qq.com/x/page/v3308e31l25.html",
        ),
        (
            "https://v.youku.com/video?vid=XNjMzNzY4MjYxNg%3D%3D&tracking=1",
            "https://v.youku.com/v_show/id_XNjMzNzY4MjYxNg==.html",
        ),
    ];
    for (url, expected) in cases {
        assert!(
            matches!(recognize(url), Some((Service::Site, Step::Publisher(page))) if page == expected),
            "{url}"
        );
    }
}

fn publisher_page(url: &Url) -> Option<(Service, Step)> {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    let host = bare_host(url)?;
    let parts = segments(url);
    let slug = |id: &str| (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'='));
    let digits = |id: &str| (1..=24).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit());
    let html_id = |id: &str| id.strip_suffix(".html").is_some_and(slug);
    let video_id = url.query_pairs().find(|(key, _)| key == "vid").map(|(_, value)| value);
    let viewkey = url.query_pairs().find(|(key, _)| key == "viewkey").map(|(_, value)| value);
    let safe = match (host.as_str(), parts.as_slice()) {
        ("pornhub.com", ["view_video.php"]) => viewkey.as_deref().is_some_and(slug),
        ("xvideos.com", [video, ..]) => video.strip_prefix("video.").is_some_and(slug) || video.strip_prefix("video").is_some_and(digits),
        ("onlyfans.com", [post, creator]) => digits(post) && slug(creator),
        ("redgifs.com", ["watch", id]) => slug(id),
        ("snapchat.com", ["spotlight", id] | [_, "spotlight", id]) => {
            (8..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        }
        ("dailymotion.com", ["video", id]) | ("dai.ly", [id]) => id.starts_with('x') && alnum(id, 2, 24),
        ("arte.tv", [lang, "videos", id, ..]) => {
            lang.len() == 2
                && id.rsplit_once('-').is_some_and(|(body, variant)| {
                    variant.len() == 1
                        && variant.bytes().all(|b| b.is_ascii_uppercase())
                        && body
                            .split_once('-')
                            .is_some_and(|(programme, part)| digits(programme) && digits(part))
                })
        }
        ("bbc.com" | "bbc.co.uk", ["iplayer", "episode", id, ..]) => slug(id),
        ("tver.jp", ["episodes", id]) => slug(id),
        ("nicovideo.jp" | "sp.nicovideo.jp", ["watch", id]) | ("nico.ms", [id]) => {
            ["sm", "so", "nm"].iter().any(|prefix| id.strip_prefix(prefix).is_some_and(digits))
        }
        ("tv.naver.com", ["v", id]) | ("chzzk.naver.com", ["video", id]) => digits(id),
        ("vod.sooplive.co.kr" | "vod.sooplive.com", ["player", id]) => digits(id),
        ("tv.kakao.com", ["channel", channel, "cliplink", id]) => digits(channel) && digits(id),
        ("douyin.com", ["video", id]) | ("iesdouyin.com", ["share", "video", id]) => digits(id),
        ("kuaishou.com", ["short-video", id]) => slug(id),
        ("xiaohongshu.com", ["explore", id] | ["discovery", "item", id]) => slug(id),
        ("v.qq.com", ["x", "cover", id]) => slug(id) || html_id(id),
        ("v.qq.com", ["x", "cover", series, id]) => slug(series) && html_id(id),
        ("v.qq.com", ["x", "page", id]) => html_id(id),
        ("v.qq.com", ["x", "m", "play"]) => video_id.as_deref().is_some_and(|id| alnum(id, 5, 24)),
        ("iqiyi.com", [id]) => id.strip_prefix("v_").is_some_and(html_id),
        ("v.youku.com", ["v_show", id]) => id.strip_prefix("id_").is_some_and(html_id),
        ("v.youku.com", ["video"]) => video_id.as_deref().is_some_and(slug),
        ("mgtv.com", ["b", series, id]) => digits(series) && html_id(id),
        _ => false,
    };
    if !safe {
        return None;
    }
    let mut page = match (host.as_str(), parts.as_slice()) {
        ("dai.ly", [id]) => Url::parse(&format!("https://www.dailymotion.com/video/{id}")).ok()?,
        ("nico.ms", [id]) | ("sp.nicovideo.jp", ["watch", id]) => Url::parse(&format!("https://www.nicovideo.jp/watch/{id}")).ok()?,
        ("iesdouyin.com", ["share", "video", id]) => Url::parse(&format!("https://www.douyin.com/video/{id}")).ok()?,
        ("v.qq.com", ["x", "m", "play"]) => Url::parse(&format!("https://v.qq.com/x/page/{}.html", video_id?)).ok()?,
        ("v.youku.com", ["video"]) => Url::parse(&format!("https://v.youku.com/v_show/id_{}.html", video_id?)).ok()?,
        _ => url.clone(),
    };
    page.set_query(None);
    page.set_fragment(None);
    if host == "pornhub.com"
        && let Some(key) = viewkey
    {
        page.query_pairs_mut().append_pair("viewkey", &key);
    }
    Some((Service::Site, Step::Publisher(page.into())))
}

/// Use an ARTE player only when the programme advertises its own embed URL.
pub(super) fn arte_clip(page: &str, html: &str) -> Option<FoundClip> {
    let page_url = Url::parse(page).ok()?;
    if bare_host(&page_url).as_deref() != Some("arte.tv") {
        return None;
    }
    let parts = segments(&page_url);
    let [language, "videos", id, ..] = parts.as_slice() else {
        return None;
    };
    let embed = metadata(html, &["og:video:secure_url", "og:video", "twitter:player"])?;
    if embed != format!("https://www.arte.tv/embeds/{language}/{id}") {
        return None;
    }
    let clip = super::Clip::of(&embed)?;
    let width = metadata(html, &["og:video:width"]).and_then(|s| s.parse().ok());
    let height = metadata(html, &["og:video:height"]).and_then(|s| s.parse().ok());
    Some(FoundClip {
        clip,
        title: metadata(html, &["og:title", "twitter:title"])?,
        channel: "ARTE".into(),
        posters: publisher_poster(page, html).into_iter().collect(),
        size: width.zip(height),
    })
}

#[test]
fn arte_plays_only_the_embed_advertised_for_that_programme() {
    let page = "https://www.arte.tv/en/videos/135503-000-A/episode/";
    let html = r#"<meta property="og:video" content="https://www.arte.tv/embeds/en/135503-000-A">
        <meta property="og:title" content="An episode">"#;
    assert_eq!(arte_clip(page, html).unwrap().title, "An episode");
    assert!(arte_clip(page, &html.replace("/embeds/en/", "/embeds/fr/")).is_none());
    assert!(arte_clip(page, &html.replace("www.arte.tv", "example.org")).is_none());
}

/// A public poster at the publisher or one of its own media hosts.
pub(super) fn publisher_poster(page: &str, html: &str) -> Option<String> {
    let page = Url::parse(page).ok()?;
    let owner = bare_host(&page)?;
    let allowed: &[&str] = match owner.as_str() {
        "pornhub.com" => &["phncdn.com"],
        "xvideos.com" => &["xvideos-cdn.com", "xvideos-cdn.net"],
        "onlyfans.com" => &["ofstatic.com"],
        "redgifs.com" => &["redgifs.com"],
        "snapchat.com" => &["sc-cdn.net", "snapcdn.com"],
        "dailymotion.com" => &["dmcdn.net"],
        "bbc.com" | "bbc.co.uk" => &["bbci.co.uk"],
        "tver.jp" => &["tver.jp", "tver-static.com"],
        "nicovideo.jp" => &["nimg.jp", "nicovideo.jp"],
        "tv.naver.com" | "chzzk.naver.com" => &["pstatic.net"],
        "vod.sooplive.co.kr" | "vod.sooplive.com" => &["sooplive.co.kr", "sooplive.com", "afreecatv.com"],
        "tv.kakao.com" => &["kakaocdn.net"],
        "douyin.com" => &["douyinpic.com", "pstatp.com"],
        "kuaishou.com" => &["kwaicdn.com", "gifshow.com"],
        "xiaohongshu.com" => &["xhscdn.com"],
        "v.qq.com" => &["qpic.cn", "gtimg.cn"],
        "iqiyi.com" => &["qiyi.com", "iqiyipic.com"],
        "v.youku.com" => &["ykimg.com"],
        "mgtv.com" => &["mgtv.com", "hunantv.com"],
        _ => &[],
    };
    head_meta(html).find_map(|(key, value)| {
        matches!(key.to_ascii_lowercase().as_str(), "og:image" | "og:image:url" | "twitter:image")
            .then(|| publisher_image(&page, &owner, allowed, &unescape(value)))
            .flatten()
    })
}

fn publisher_image(page: &Url, owner: &str, allowed: &[&str], value: &str) -> Option<String> {
    if value.len() > 2048 {
        return None;
    }
    let url = page.join(value).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    let host = url.host_str()?;
    (bare_host(&url).as_deref() == Some(owner)
        || allowed
            .iter()
            .any(|domain| host == *domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.'))))
    .then(|| url.into())
}

pub(super) fn dailymotion_oembed(page: &str) -> Option<Url> {
    let parsed = Url::parse(page).ok()?;
    (bare_host(&parsed).as_deref() == Some("dailymotion.com")).then_some(())?;
    Url::parse_with_params("https://www.dailymotion.com/services/oembed", [("url", page)]).ok()
}

pub(super) fn from_dailymotion_oembed(page: &str, body: &[u8]) -> Option<(super::PostInfo, Option<String>)> {
    let parsed = Url::parse(page).ok()?;
    let data: serde_json::Value = serde_json::from_slice(body).ok()?;
    let title: String = data["title"].as_str()?.trim().chars().take(200).collect();
    if title.is_empty() {
        return None;
    }
    let channel = data["author_name"].as_str().unwrap_or("Dailymotion").chars().take(80).collect();
    let poster = data["thumbnail_url"]
        .as_str()
        .and_then(|url| publisher_image(&parsed, "dailymotion.com", &["dmcdn.net"], url));
    Some((
        super::PostInfo {
            title,
            channel,
            summary: String::new(),
        },
        poster,
    ))
}

/// The servers each service keeps its pictures on.
fn picture_hosts(service: Service) -> &'static [&'static str] {
    match service {
        // pictures and the thumbnails of video captures
        Service::Gyazo => &["i.gyazo.com", "thumb.gyazo.com"],
        Service::Flickr => &["staticflickr.com"],
        Service::Imgur => &["i.imgur.com"],
        Service::Giphy | Service::Tenor => &[],
        // Lightshot's former screenshots are kept on Imgur
        Service::Lightshot => &["image.prntscr.com", "img.prnt.sc", "i.imgur.com"],
        Service::Reddit => &["i.redd.it", "preview.redd.it", "external-preview.redd.it"],
        // pictures and video thumbnails
        Service::Bluesky => &["cdn.bsky.app", "video.bsky.app"],
        Service::X => &["pbs.twimg.com"],
        // the listed sites themselves (`on_site`)
        Service::Site => &[],
    }
}

/// `url` is an https picture on `service`'s own servers, or on one of the
/// user's picture sites, where a port is allowed (a self-hosted server may
/// answer on another one).
pub(super) fn picture_host_ok(service: Service, url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str().map(|h| h.trim_end_matches('.').to_lowercase()) else {
        return false;
    };
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    if matches!(service, Service::Giphy | Service::Tenor) {
        let domain = if service == Service::Giphy { ".giphy.com" } else { ".tenor.com" };
        return parsed.port().is_none()
            && host.strip_suffix(domain).is_some_and(|h| {
                h.strip_prefix("media").is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())) || (service == Service::Tenor && h == "c")
            })
            && [".gif", ".webp"]
                .iter()
                .any(|ext| parsed.path().to_ascii_lowercase().ends_with(ext));
    }
    if service == Service::Site {
        return on_site(&host);
    }
    parsed.port().is_none()
        && picture_hosts(service)
            .iter()
            .any(|d| host == *d || host.strip_suffix(d).is_some_and(|sub| sub.ends_with('.')))
}

/// The site's own icon a page names when it has no picture: Nextcloud's
/// share of a folder or of a file it makes no preview of names
/// `core/img/favicon-fb.png` (or its theming app's `favicon`).
fn site_icon(url: &str) -> bool {
    Url::parse(url).ok().is_some_and(|u| {
        segments(&u)
            .last()
            .is_some_and(|name| name.to_ascii_lowercase().starts_with("favicon"))
    })
}

/// Imgur's `og:image` asks for a copy padded for social cards (`?fb`); the
/// picture itself has the same address without it.
fn cleaned(service: Service, url: String) -> String {
    match (service, url.split_once('?')) {
        (Service::Imgur | Service::Lightshot, Some((bare, _))) => bare.to_owned(),
        _ => url,
    }
}

/// The picture of an oEmbed answer: a photo's `url`, nothing for a video or
/// a rich embed.
pub(super) fn from_oembed(service: Service, body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let url = (v["type"].as_str() == Some("photo")).then(|| v["url"].as_str())??.to_owned();
    picture_host_ok(service, &url).then_some(url)
}

/// A clip identified by its provider's metadata.
pub(super) struct FoundClip {
    pub clip: super::Clip,
    pub title: String,
    pub channel: String,
    /// Its thumbnail's addresses, the larger first.
    pub posters: Vec<String>,
    /// Its size in pixels, when the answer gives it.
    pub size: Option<(f64, f64)>,
}

/// A Gyazo video capture (#983): its oEmbed answer has the type `video` and
/// an embed of Gyazo's own player, not the file, which Gyazo serves as
/// `i.gyazo.com/<id>.mp4` (the id of the page the link names). Its
/// `thumbnail_url` is a small copy (`thumb/116_w/…`) whose width is part of
/// the address: a 640 wide copy is asked for first.
pub(super) fn clip_from_oembed(service: Service, page: &str, body: &[u8]) -> Option<FoundClip> {
    if service != Service::Gyazo {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    if v["type"].as_str() != Some("video") {
        return None;
    }
    let parsed = Url::parse(page).ok()?;
    let id = segments(&parsed).first().copied().filter(|id| gyazo_id(id))?.to_lowercase();
    let clip = super::Clip::of(&format!("https://i.gyazo.com/{id}.mp4"))?;
    let posters = match v["thumbnail_url"].as_str().filter(|t| picture_host_ok(service, t)) {
        Some(small) => {
            let large = small
                .split('/')
                .map(|seg| match seg.strip_suffix("_w") {
                    Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => "640_w",
                    _ => seg,
                })
                .collect::<Vec<_>>()
                .join("/");
            if large == small {
                vec![large]
            } else {
                vec![large, small.to_owned()]
            }
        }
        None => Vec::new(),
    };
    let size = v["width"].as_f64().zip(v["height"].as_f64());
    Some(FoundClip {
        clip,
        title: String::new(),
        channel: String::new(),
        posters,
        size,
    })
}

/// HTML's character references the services put in an attribute.
fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x2F;", "/")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

/// The value of attribute `name` in the inside of a tag, quoted or not.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        from = start + name.len();
        // a whole attribute name: preceded by a space, followed by "="
        if !lower[..start].ends_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        return Some(match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
            _ => value
                .split(|c: char| c.is_ascii_whitespace() || c == '>')
                .next()
                .unwrap_or("")
                .trim_end_matches('/'),
        });
    }
    None
}

pub(super) fn metadata(html: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let (_, value) = head_meta(html).find(|(name, _)| name.eq_ignore_ascii_case(key))?;
        let value: String = unescape(value)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(200)
            .collect();
        (!value.is_empty()).then_some(value)
    })
}

fn head_meta(html: &str) -> impl Iterator<Item = (&str, &str)> {
    html.split('<')
        .skip(1)
        .take_while(|tag| {
            !tag.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("/head")) && !tag.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("body"))
        })
        .filter_map(|tag| {
            let tag = tag.split('>').next().unwrap_or("");
            let is_meta =
                tag.get(..4).is_some_and(|t| t.eq_ignore_ascii_case("meta")) && tag[4..].starts_with(|c: char| c.is_ascii_whitespace());
            is_meta.then_some(())?;
            Some((
                attribute(tag, "property").or_else(|| attribute(tag, "name"))?,
                attribute(tag, "content")?,
            ))
        })
}

/// The picture a page announces for itself: its `og:image`, or
/// `twitter:image` when it has none.
pub(super) fn from_head(service: Service, html: &str) -> Option<String> {
    let mut twitter = None;
    for (key, content) in head_meta(html) {
        let url = cleaned(service, unescape(content));
        if !picture_host_ok(service, &url) || (service == Service::Site && site_icon(&url)) {
            continue;
        }
        if key.eq_ignore_ascii_case("og:image") || key.eq_ignore_ascii_case("og:image:url") {
            return Some(url);
        }
        if key.eq_ignore_ascii_case("twitter:image") && twitter.is_none() {
            twitter = Some(url);
        }
    }
    twitter
}

/// Imgur share pages advertise a cover and, for animated posts, a stream.
/// Keep galleries and albums as cover cards even when their first item moves.
pub(super) fn clip_from_head(service: Service, page: &str, html: &str) -> Option<FoundClip> {
    if service == Service::Site {
        return declared_video(page, html);
    }
    if service != Service::Imgur {
        return None;
    }
    let page = Url::parse(page).ok()?;
    let parts = segments(&page);
    let [id] = parts.as_slice() else {
        return None;
    };
    let mut width = None;
    let mut height = None;
    let mut stream = None;
    let mut video = None;
    for (key, value) in head_meta(html) {
        match key.to_ascii_lowercase().as_str() {
            "og:video:width" => width = value.parse().ok(),
            "og:video:height" => height = value.parse().ok(),
            "twitter:player:stream" | "og:video" | "og:video:url" | "og:video:secure_url" | "twitter:player" => {
                let value = unescape(value);
                let parsed = Url::parse(&value).ok();
                let Some(clip) = parsed
                    .filter(|url| {
                        url.host_str() == Some("i.imgur.com")
                            && url
                                .path()
                                .strip_prefix('/')
                                .and_then(|path| path.rsplit_once('.'))
                                .is_some_and(|(stem, _)| stem == *id)
                    })
                    .and_then(|_| super::Clip::of(&value))
                else {
                    continue;
                };
                if key.eq_ignore_ascii_case("twitter:player:stream") {
                    stream.get_or_insert(clip);
                } else {
                    video.get_or_insert(clip);
                }
            }
            _ => {}
        }
    }
    Some(FoundClip {
        clip: stream.or(video)?,
        title: metadata(html, &["og:title"]).unwrap_or_default(),
        channel: metadata(html, &["author"]).unwrap_or_default(),
        posters: from_head(service, html).into_iter().collect(),
        size: width.zip(height),
    })
}

/// A configured site can name a direct video in its sharing metadata.
fn declared_video(page: &str, html: &str) -> Option<FoundClip> {
    let page = Url::parse(page).ok()?;
    let clip = head_meta(html).find_map(|(name, value)| {
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "og:video:secure_url" | "og:video:url" | "og:video" | "twitter:player:stream"
        ) || value.len() > 2048
        {
            return None;
        }
        let media = page.join(&unescape(value)).ok()?;
        if media.origin() != page.origin() {
            return None;
        }
        let clip = super::Clip::of(media.as_str())?;
        clip.download_url().is_some().then_some(clip)
    })?;
    let width = metadata(html, &["og:video:width"]).and_then(|s| s.parse().ok());
    let height = metadata(html, &["og:video:height"]).and_then(|s| s.parse().ok());
    Some(FoundClip {
        clip,
        title: metadata(html, &["og:title", "twitter:title"]).unwrap_or_default(),
        channel: metadata(html, &["author"]).unwrap_or_default(),
        posters: from_head(Service::Site, html).into_iter().collect(),
        size: width.zip(height),
    })
}

#[test]
fn configured_pages_promote_same_origin_direct_media_only() {
    let page = "https://i.imgur.com/shares/example";
    let html = r#"<meta property="og:video" content="https://streamable.com/e/Demo42">
        <meta property="og:video:secure_url" content="/sample.mp4">
        <meta property="og:title" content="A vertical video">
        <meta property="og:video:width" content="360">
        <meta property="og:video:height" content="640">"#;
    let result = declared_video(page, html).unwrap();
    assert_eq!(result.clip.download_url(), Some("https://i.imgur.com/sample.mp4"));
    assert_eq!(result.title, "A vertical video");
    assert_eq!(result.size, Some((360.0, 640.0)));
    let cross_site = r#"<meta property="og:video" content="https://video.twimg.com/example.mp4">"#;
    assert!(declared_video(page, cross_site).is_none());
}

pub(super) fn reddit_feed(xml: &str) -> Option<super::PostInfo> {
    let entry = xml.split("<entry").nth(1)?.split("</entry>").next()?;
    let field = |name: &str| {
        let value = entry
            .split_once(&format!("<{name}"))?
            .1
            .split_once('>')?
            .1
            .split_once(&format!("</{name}>"))?
            .0;
        Some(unescape(value))
    };
    let title: String = field("title")?.trim().chars().take(200).collect();
    if title.is_empty() {
        return None;
    }
    let text = field("content").unwrap_or_default();
    let summary = text
        .split("<!-- SC_ON -->")
        .nth(1)
        .and_then(|body| body.split("<!-- SC_OFF -->").next())
        .map_or_else(String::new, |body| {
            body.split('<')
                .flat_map(|part| part.split_once('>').map(|(_, text)| text))
                .flat_map(str::split_whitespace)
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(280)
                .collect()
        });
    let community = entry
        .split_once("<category")
        .and_then(|(_, tag)| tag.split_once('>'))
        .and_then(|(tag, _)| attribute(tag, "term"))
        .map(|term| format!("r/{}", unescape(term)));
    let channel = [
        community,
        field("name"),
        field("published").map(|date| date.chars().take(10).collect()),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    Some(super::PostInfo { title, channel, summary })
}

#[test]
fn reddit_feed_keeps_the_post_title_byline_and_plain_text() {
    let xml = r#"<feed><entry><title>Example title</title><author><name>/u/example</name></author>
        <category term="demo"/><published>2026-03-01T15:51:00Z</published>
        <content type="html">&lt;!-- SC_ON --&gt;&lt;p&gt;A short &lt;b&gt;summary&lt;/b&gt;.&lt;/p&gt;
        &lt;!-- SC_OFF --&gt;</content></entry></feed>"#;
    let post = reddit_feed(xml).unwrap();
    assert_eq!(post.title, "Example title");
    assert_eq!(post.channel, "r/demo · /u/example · 2026-03-01");
    assert_eq!(post.summary, "A short summary .");
}

/// The first entry may link to the Reddit-hosted video, separately from its poster.
pub(super) fn clip_from_feed(xml: &str) -> Option<FoundClip> {
    let entry = xml.split("<entry").nth(1)?.split("</entry>").next()?;
    let content = unescape(entry.split("<content").nth(1)?.split("</content>").next()?);
    let url = content.split_once("href=\"https://v.redd.it/")?.1.split('"').next()?;
    let clip = super::Clip::of(&format!("https://v.redd.it/{url}")).filter(super::Clip::native_video)?;
    let post = reddit_feed(xml)?;
    Some(FoundClip {
        clip,
        title: post.title,
        channel: post.channel,
        posters: from_feed(xml).into_iter().collect(),
        size: None,
    })
}

/// A GIF or animated WebP linked from the post itself. Reddit can publish a
/// still `media:thumbnail` for these, so try the original image first.
pub(super) fn reddit_animation(xml: &str) -> Option<String> {
    let entry = xml.split("<entry").nth(1)?.split("</entry>").next()?;
    let html = unescape(entry.split("<content").nth(1)?.split("</content>").next()?);
    html.split('<').find_map(|tag| {
        let tag = tag.split_once('>')?.0.strip_prefix('a')?;
        if !tag.starts_with(|c: char| c.is_ascii_whitespace()) {
            return None;
        }
        let url = Url::parse(&unescape(attribute(tag, "href")?)).ok()?;
        let path = url.path().to_ascii_lowercase();
        (url.host_str() == Some("i.redd.it")
            && (path.ends_with(".gif") || path.ends_with(".webp"))
            && picture_host_ok(Service::Reddit, url.as_str()))
        .then(|| url.into())
    })
}

#[test]
fn reddit_prefers_its_animated_image_to_the_still_thumbnail() {
    let xml = concat!(
        r#"<feed><entry><media:thumbnail url="https://preview.redd.it/still.jpg"/>"#,
        r#"<content type="html">&lt;a href=&quot;https://i.redd.it.example.org/untrusted.gif&quot;&gt;other&lt;/a&gt;"#,
        r#"&lt;a href=&quot;https://i.redd.it/example.gif&quot;&gt;image&lt;/a&gt;</content></entry></feed>"#,
    );
    assert_eq!(reddit_animation(xml).as_deref(), Some("https://i.redd.it/example.gif"));
    assert_eq!(from_feed(xml).as_deref(), Some("https://preview.redd.it/still.jpg"));
}

/// The picture of a Reddit post's feed: its entry's `media:thumbnail`
/// (Reddit's preview, at most 640 wide). A text post has none.
pub(super) fn from_feed(xml: &str) -> Option<String> {
    let entry = xml.split("<entry").nth(1)?;
    let entry = entry.split("</entry>").next().unwrap_or(entry);
    let tag = entry.split("<media:thumbnail").nth(1)?.split('>').next()?;
    let url = unescape(attribute(&format!(" {tag}"), "url")?);
    picture_host_ok(Service::Reddit, &url).then_some(url)
}

/// Bluesky's labels that hide a post's media behind a warning in its own
/// app: adult content and graphic violence (`porn`, `sexual`, `nudity`,
/// `graphic-media`, and the older `gore`), any warning (`!warn`, `!hide`).
const BLUESKY_SENSITIVE: &[&str] = &[
    "porn",
    "sexual",
    "sexual-figurative",
    "nudity",
    "graphic-media",
    "gore",
    "!warn",
    "!hide",
];

/// Select a public MP4 rendition from X's post metadata. Sensitive posts
/// keep the existing blurred picture and browser path.
pub(super) fn clip_from_post(service: Service, body: &[u8]) -> Option<FoundClip> {
    if service != Service::X {
        return None;
    }
    let post: serde_json::Value = serde_json::from_slice(body).ok()?;
    if post["possibly_sensitive"].as_bool() != Some(false) {
        return None;
    }
    let detail = &post["mediaDetails"][0];
    let variants = detail["video_info"]["variants"]
        .as_array()
        .or_else(|| post["video"]["variants"].as_array())?;
    let clip = variants
        .iter()
        .filter_map(|variant| {
            let url = variant["url"].as_str().or_else(|| variant["src"].as_str())?;
            let media = Url::parse(url).ok()?;
            if media.host_str()? != "video.twimg.com" || !media.path().ends_with(".mp4") {
                return None;
            }
            Some((variant["bitrate"].as_u64().unwrap_or(0), super::Clip::of(url)?))
        })
        .min_by_key(|(bitrate, _)| bitrate.abs_diff(1_200_000))?
        .1;
    let poster = detail["media_url_https"].as_str().or_else(|| post["video"]["poster"].as_str());
    let size = &detail["original_info"];
    Some(FoundClip {
        clip,
        title: post["text"]
            .as_str()
            .unwrap_or_default()
            .split(" https://t.co/")
            .next()
            .unwrap_or("")
            .chars()
            .take(160)
            .collect(),
        channel: post["user"]["name"].as_str().unwrap_or_default().chars().take(80).collect(),
        posters: poster
            .filter(|url| picture_host_ok(service, url))
            .into_iter()
            .map(str::to_owned)
            .collect(),
        size: size["width"].as_f64().zip(size["height"].as_f64()),
    })
}

#[test]
fn x_clips_require_a_public_mp4_on_the_official_video_host() {
    let mut post = serde_json::json!({
        "possibly_sensitive": false,
        "video": {
            "poster": "https://pbs.twimg.com/media/Preview.jpg",
            "variants": [
                {"src": "https://other.example/video.mp4"},
                {"src": "https://video.twimg.com/test/master.m3u8"},
                {"src": "https://video.twimg.com/test/clip.mp4"}
            ]
        }
    });
    let body = serde_json::to_vec(&post).unwrap();
    let clip = clip_from_post(Service::X, &body).expect("public MP4 rendition");
    assert_eq!(clip.clip.source(), "https://video.twimg.com/test/clip.mp4");
    assert_eq!(clip.posters, ["https://pbs.twimg.com/media/Preview.jpg"]);
    post["possibly_sensitive"] = true.into();
    assert!(clip_from_post(Service::X, &serde_json::to_vec(&post).unwrap()).is_none());
}

/// The picture of a post's JSON answer and whether its network marks it
/// sensitive: its first picture, or its video's thumbnail. A text post has
/// none.
pub(super) fn from_post(service: Service, body: &[u8]) -> Option<(String, bool)> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let (url, sensitive) = match service {
        Service::Bluesky => {
            let post = &v["thread"]["post"];
            let embed = &post["embed"];
            // a quote with pictures keeps them in `media`
            let media = if embed["$type"] == "app.bsky.embed.recordWithMedia#view" {
                &embed["media"]
            } else {
                embed
            };
            let url = match media["$type"].as_str()? {
                "app.bsky.embed.images#view" => media["images"][0]["fullsize"].as_str(),
                "app.bsky.embed.video#view" => media["thumbnail"].as_str(),
                _ => None,
            }?;
            // the labels of the post (its author's own included) and of its
            // author's account
            let labels = [&post["labels"], &post["author"]["labels"], &post["record"]["labels"]["values"]];
            let sensitive = labels
                .iter()
                .filter_map(|l| l.as_array())
                .flatten()
                .any(|l| l["val"].as_str().is_some_and(|val| BLUESKY_SENSITIVE.contains(&val)) && l["neg"].as_bool() != Some(true));
            (url, sensitive)
        }
        Service::X => {
            let url = v["mediaDetails"][0]["media_url_https"]
                .as_str()
                .or_else(|| v["photos"][0]["url"].as_str())
                .or_else(|| v["video"]["poster"].as_str())?;
            (url, v["possibly_sensitive"].as_bool() == Some(true))
        }
        _ => return None,
    };
    picture_host_ok(service, url).then(|| (url.to_owned(), sensitive))
}

/// Pictures from Reddit are blurred until clicked, in every conversation:
/// Reddit hosts adult communities and tells a viewer without an account
/// nothing of a post being adult (its feed has no such mark).
pub fn blurred_until_clicked(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    bare_host(&parsed).is_some_and(|host| {
        ["reddit.com", "redd.it", "redditmedia.com"]
            .iter()
            .any(|d| host == *d || host.strip_suffix(d).is_some_and(|sub| sub.ends_with('.')))
    })
}

// Saved answers of each service, with made-up ids: the demo's links and the
// tests read them as the real answers are read.
const GYAZO: &str = include_str!("fixtures/gyazo_oembed.json");
const GYAZO_VIDEO: &str = include_str!("fixtures/gyazo_video.json");
const FLICKR: &str = include_str!("fixtures/flickr_oembed.json");
const IMGUR_ALBUM: &str = include_str!("fixtures/imgur_album.html");
const LIGHTSHOT: &str = include_str!("fixtures/lightshot.html");
const LIGHTSHOT_REMOVED: &str = include_str!("fixtures/lightshot_removed.html");
const REDDIT_POST: &str = include_str!("fixtures/reddit_post.rss");
const REDDIT_TEXT: &str = include_str!("fixtures/reddit_text.rss");
const BLUESKY_POST: &str = include_str!("fixtures/bluesky_post.json");
const BLUESKY_LABELLED: &str = include_str!("fixtures/bluesky_labelled.json");
const X_POST: &str = include_str!("fixtures/x_post.json");
const NEXTCLOUD_PICTURE: &str = include_str!("fixtures/nextcloud_picture.html");
const NEXTCLOUD_FOLDER: &str = include_str!("fixtures/nextcloud_folder.html");

/// Demo mode: the picture a service's link leads to, from the saved
/// answers, and whether it is marked sensitive. A Lightshot link whose id
/// starts with `removed` is a removed screenshot, the Reddit post `demo02` a
/// text post: no picture for either. A Bluesky post whose key holds
/// `labelled` is labelled `nudity`. A picture site's page is a Nextcloud
/// share, of a folder (no picture) when its link holds `Folder`.
pub(super) fn demo_source(url: &str) -> Option<(String, bool)> {
    let (service, step) = recognize(url)?;
    let plain = |source: Option<String>| source.map(|s| (s, false));
    match step {
        Step::Direct(source) => Some((source, false)),
        Step::OEmbed(_) => plain(from_oembed(
            service,
            if service == Service::Gyazo { GYAZO } else { FLICKR }.as_bytes(),
        )),
        Step::Head(_) if service == Service::Lightshot => plain(from_head(
            service,
            if url.contains("/removed") { LIGHTSHOT_REMOVED } else { LIGHTSHOT },
        )),
        Step::Head(_) if service == Service::Site => plain(from_head(
            service,
            if url.contains("Folder") {
                NEXTCLOUD_FOLDER
            } else {
                NEXTCLOUD_PICTURE
            },
        )),
        Step::Head(_) => plain(from_head(service, IMGUR_ALBUM)),
        Step::Publisher(_) => None,
        Step::Feed { .. } => plain(from_feed(if url.contains("demo02") { REDDIT_TEXT } else { REDDIT_POST })),
        Step::Post(_) if service == Service::X => from_post(service, X_POST.as_bytes()),
        Step::Post(_) => from_post(
            service,
            if url.contains("labelled") { BLUESKY_LABELLED } else { BLUESKY_POST }.as_bytes(),
        ),
    }
}

/// Demo mode: the clip of a Gyazo link to the video capture of the saved
/// answer (`9f8e7d6c…`).
pub(super) fn demo_clip(url: &str) -> Option<FoundClip> {
    clip_from_oembed(recognize(url)?.0, url, GYAZO_VIDEO.as_bytes())
        .filter(|found| found.clip.source().contains("9f8e7d6c5b4a39281706f5e4d3c2b1a0"))
}

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
