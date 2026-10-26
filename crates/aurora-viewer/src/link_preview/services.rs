//! The pages of image services whose links get the picture they show
//! (#923): a link to a Gyazo, Flickr, Imgur, Lightshot or Reddit page, or to
//! a Bluesky or X post (#1051), not to the picture's file, still gets a
//! picture card. An Aurora feature, like the rest of the previews.
//!
//! Each page leads to its picture by the cleanest way its service offers:
//! * Gyazo and Flickr: their oEmbed endpoints (documented, no key); a Gyazo
//!   video capture leads to its file instead, played in a video card (#983);
//! * Imgur and Lightshot: the page's `og:image`, read from the first part of
//!   the page only (Imgur's API needs a registered key, Lightshot has none);
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
    /// Reddit's Atom feed of a post.
    Feed(String),
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
                (
                    Service::Reddit,
                    Step::Feed(format!("https://www.reddit.com/comments/{id}/.rss?limit=1")),
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
        _ => site_page(&parsed),
    }
}

/// The servers each service keeps its pictures on.
fn picture_hosts(service: Service) -> &'static [&'static str] {
    match service {
        // pictures and the thumbnails of video captures
        Service::Gyazo => &["i.gyazo.com", "thumb.gyazo.com"],
        Service::Flickr => &["staticflickr.com"],
        Service::Imgur => &["i.imgur.com"],
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
    if parsed.scheme() != "https" || !parsed.username().is_empty() {
        return false;
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

/// The clip of a Gyazo video capture, from its page's oEmbed answer.
pub(super) struct FoundClip {
    pub clip: super::Clip,
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
    Some(FoundClip { clip, posters, size })
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

/// The picture a page announces for itself: its `og:image`, or
/// `twitter:image` when it has none.
pub(super) fn from_head(service: Service, html: &str) -> Option<String> {
    let mut twitter = None;
    for tag in html.split('<').skip(1) {
        let tag = tag.split('>').next().unwrap_or("");
        let is_meta =
            tag.get(..4).is_some_and(|t| t.eq_ignore_ascii_case("meta")) && tag[4..].starts_with(|c: char| c.is_ascii_whitespace());
        if !is_meta {
            continue;
        }
        let key = attribute(tag, "property").or_else(|| attribute(tag, "name")).unwrap_or("");
        let Some(content) = attribute(tag, "content") else {
            continue;
        };
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
        Step::Feed(_) => plain(from_feed(if url.contains("demo02") { REDDIT_TEXT } else { REDDIT_POST })),
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
        .filter(|found| found.clip.file.contains("9f8e7d6c5b4a39281706f5e4d3c2b1a0"))
}

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
