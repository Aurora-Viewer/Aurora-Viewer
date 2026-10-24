//! Link previews of the conversations (issues #538, #739): which links of a
//! message get a picture under them, the download of that picture and its
//! decoding, off the frame. The cards are drawn by `ui/chat/previews.rs`.
//!
//! An Aurora feature: the Legacy Viewer shows links as text only, and the
//! Legacy style of the Conversations window keeps it that way.
//!
//! What keeps it safe, in order:
//! * nothing is fetched before the rules allow it (`decide`): by default
//!   only links to the trusted sites of `link_trust.rs` load by themselves;
//!   the others wait for a click, as loading a picture tells its site our
//!   address;
//! * a link that hides or fakes its destination never gets a preview;
//! * no cookie and no referrer are sent; when the trust of the site is what
//!   allowed the load, redirects must stay on trusted sites;
//! * at most `MAX_BYTES`, refused before the body when the server announces
//!   more; the decoding runs on a blocking thread, within pixel limits;
//! * pictures are kept at the size they are drawn at, the frames of an
//!   animated GIF within a memory budget.

use crate::link_trust::Trust;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Largest download.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Longest side kept of a picture: twice the largest card, for dense screens.
const MAX_SIDE: u32 = 800;
/// Longest side kept of the frames of an animated GIF.
const GIF_MAX_SIDE: u32 = 480;
const GIF_MAX_FRAMES: usize = 150;
/// Memory the frames of one GIF may take.
const GIF_BUDGET: usize = 48 * 1024 * 1024;
/// Largest picture decoded, in pixels per side.
const DECODE_LIMIT: u32 = 8192;
/// Most redirects followed.
const MAX_REDIRECTS: usize = 5;

/// Which links load by themselves (Preferences › Chat).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Load {
    /// None: every preview waits for a click.
    Never,
    /// Links to the trusted sites (`link_trust.rs`).
    #[default]
    Trusted,
    /// Any link that does not hide its destination.
    All,
}

/// Where a YouTube video plays.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Player {
    /// In its card, which can send it to a window.
    #[default]
    InCard,
    /// In a window of its own.
    Window,
}

/// Width of the cards.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

impl Size {
    /// Width of a picture's card, in points.
    pub fn width(self) -> f32 {
        match self {
            Size::Small => 240.0,
            Size::Medium => 320.0,
            Size::Large => 400.0,
        }
    }
}

/// The link previews of Preferences › Chat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// In the conversations with a resident or an object.
    pub ims: bool,
    /// In group chats and conferences.
    pub groups: bool,
    /// In the nearby chat of the Conversations window (never in the chat
    /// lines at the bottom of the screen).
    pub nearby: bool,
    /// The pictures of group chats and conferences are blurred until
    /// clicked.
    pub blur_groups: bool,
    pub load: Load,
    pub player: Player,
    pub size: Size,
    /// Sites whose links get no preview (a link's menu adds them).
    pub hidden_sites: Vec<String>,
    /// The level of the player's own volume, 0 to 1, kept from one video
    /// to the next (#848): set by the player's slider or Preferences, and
    /// still scaled by the master and media volumes.
    pub video_volume: f32,
    /// A video starts with its subtitles shown.
    pub subtitles: bool,
    /// The subtitles' language, a code such as "fr" ("" = the viewer's).
    pub subtitle_language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ims: true,
            groups: true,
            nearby: false,
            blur_groups: true,
            load: Load::default(),
            player: Player::default(),
            size: Size::default(),
            hidden_sites: Vec::new(),
            video_volume: 1.0,
            subtitles: false,
            subtitle_language: String::new(),
        }
    }
}

impl Settings {
    /// What the player starts with, in the viewer's `language`.
    pub fn player_options(&self, language: &str) -> PlayerOptions {
        let language = language_code(language).unwrap_or_else(|| "en".to_owned());
        PlayerOptions {
            volume: (self.video_volume.clamp(0.0, 1.0) * 100.0).round() as u8,
            subtitles: self
                .subtitles
                .then(|| language_code(&self.subtitle_language).unwrap_or_else(|| language.clone())),
            language,
        }
    }
}

/// How a video starts playing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerOptions {
    /// The player's own volume, 0 to 100.
    pub volume: u8,
    /// The subtitles' language when they are shown.
    pub subtitles: Option<String>,
    /// The language of the player's interface.
    pub language: String,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        PlayerOptions {
            volume: 100,
            subtitles: None,
            language: "en".to_owned(),
        }
    }
}

/// A language code fit for the player's address: letters and `-`, 2 to 8
/// long ("fr", "pt-BR"); anything else is refused.
fn language_code(s: &str) -> Option<String> {
    let s = s.trim();
    ((2..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-') && !s.starts_with('-')).then(|| s.to_owned())
}

/// The conversation a link is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Im,
    Group,
    Nearby,
}

/// What a link shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Picture,
    Video(Video),
}

/// A YouTube video.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Video {
    pub id: String,
    /// Where it starts, in seconds (`t=` of the link).
    pub start: u32,
}

impl Video {
    /// Its picture on YouTube's image server.
    pub fn thumbnail(&self) -> String {
        format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", self.id)
    }

    /// Its title and channel (oEmbed: no key, no account).
    pub fn oembed(&self) -> String {
        format!(
            "https://www.youtube.com/oembed?format=json&url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{}",
            self.id
        )
    }

    /// The player alone, on YouTube's domain that sets no cookie: its
    /// interface in the viewer's language, the subtitles shown in theirs
    /// when asked (`cc_load_policy`, `cc_lang_pref`), and the page around it
    /// allowed to set and read its volume (`enablejsapi`).
    pub fn player(&self, options: &PlayerOptions) -> String {
        let start = if self.start > 0 {
            format!("&start={}", self.start)
        } else {
            String::new()
        };
        let subtitles = match &options.subtitles {
            Some(lang) => format!("&cc_load_policy=1&cc_lang_pref={lang}"),
            None => String::new(),
        };
        format!(
            "https://www.youtube-nocookie.com/embed/{}?autoplay=1&rel=0&enablejsapi=1&hl={}{subtitles}{start}",
            self.id, options.language
        )
    }

    /// The page the browser opens to play it: YouTube's player refuses a
    /// page that names no referrer (its error 153), which an address
    /// opened directly never does; so the player sits in a page of our
    /// own, served on this machine (`player_page`). The player's address
    /// alone when that page cannot be served.
    pub fn player_page(&self, options: &PlayerOptions) -> String {
        match player_server() {
            Some(port) => {
                let cc = options.subtitles.as_ref().map(|l| format!("&cc={l}")).unwrap_or_default();
                format!(
                    "http://127.0.0.1:{port}/youtube/{}?start={}&vol={}&hl={}{cc}",
                    self.id, self.start, options.volume, options.language
                )
            }
            None => self.player(options),
        }
    }
}

/// The page holding a video's player, for `id` (checked) from `start`.
/// Its script speaks YouTube's embed protocol by `postMessage` (what
/// YouTube's iframe API does, without loading that script): once the
/// player is ready it gets the volume kept, and each change made with the
/// player's own slider goes back to the viewer (`/volume`), so the next
/// video starts at the same level (#848).
fn player_page(id: &str, start: u32, options: &PlayerOptions, port: u16) -> String {
    let player = Video { id: id.to_owned(), start }.player(options);
    let volume = options.volume.min(100);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"referrer\" content=\"strict-origin-when-cross-origin\">\
         <style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}\
         iframe{{border:0;width:100%;height:100%}}</style></head><body>\
         <iframe src=\"{player}&amp;origin=http%3A%2F%2F127.0.0.1%3A{port}\" \
         allow=\"autoplay; encrypted-media; picture-in-picture; fullscreen\" allowfullscreen \
         referrerpolicy=\"strict-origin-when-cross-origin\"></iframe>\
         <script>(function(){{var f=document.querySelector('iframe'),o='https://www.youtube-nocookie.com',\
         v={volume},since=0;\
         function post(m){{f.contentWindow.postMessage(JSON.stringify(m),o);}}\
         f.addEventListener('load',function(){{post({{event:'listening',id:1,channel:'widget'}});}});\
         window.addEventListener('message',function(e){{if(e.origin!==o)return;var d;\
         try{{d=JSON.parse(e.data);}}catch(x){{return;}}\
         if(d.event==='onReady'){{post({{event:'command',func:'setVolume',args:[v]}});since=Date.now()+1500;return;}}\
         if(!since||Date.now()<since||d.event!=='infoDelivery'||!d.info||typeof d.info.volume!=='number')return;\
         var n=Math.round(d.info.volume);if(n!==v){{v=n;new Image().src='/volume?v='+n;}}}});}})();</script>\
         </body></html>"
    )
}

/// The answer to a request line of the player server: the page of a
/// video, or nothing for anything else.
fn player_answer(request: &str, port: u16) -> Option<String> {
    let path = request.strip_prefix("GET /youtube/")?.split(' ').next()?;
    let (id, query) = path.split_once('?').unwrap_or((path, ""));
    let id = video_id(id)?;
    let param = |name: &str| {
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix(name).and_then(|v| v.strip_prefix('=')))
    };
    let start = param("start").and_then(|v| v.parse().ok()).unwrap_or(0);
    let options = PlayerOptions {
        volume: param("vol").and_then(|v| v.parse::<u8>().ok()).unwrap_or(100).min(100),
        subtitles: param("cc").and_then(language_code),
        language: param("hl").and_then(language_code).unwrap_or_else(|| "en".to_owned()),
    };
    Some(player_page(&id, start, &options, port))
}

/// A level the player's page reports (`GET /volume?v=<0-100>`).
fn volume_answer(request: &str) -> Option<u8> {
    let path = request.strip_prefix("GET /volume?v=")?.split(' ').next()?;
    path.parse::<u8>().ok().filter(|v| *v <= 100)
}

/// The last level the player reported, not yet taken by the app (-1: none).
static REPORTED_VOLUME: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// The player's volume changed with its own slider, 0 to 1: the app keeps
/// it in the settings. Once per change.
pub fn take_video_volume() -> Option<f32> {
    let v = REPORTED_VOLUME.swap(-1, std::sync::atomic::Ordering::Relaxed);
    (v >= 0).then(|| v as f32 / 100.0)
}

/// The loopback server of the player pages, started once; its port.
fn player_server() -> Option<u16> {
    static PORT: std::sync::OnceLock<Option<u16>> = std::sync::OnceLock::new();
    *PORT.get_or_init(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| log::warn!("link previews: no player server: {e}"))
            .ok()?;
        let port = listener.local_addr().ok()?.port();
        let spawned = std::thread::Builder::new().name("youtube-page".into()).spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let line = request.lines().next().unwrap_or("");
                if let Some(volume) = volume_answer(line) {
                    REPORTED_VOLUME.store(i32::from(volume), std::sync::atomic::Ordering::Relaxed);
                    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let answer = match player_answer(line, port) {
                    Some(page) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
                        page.len()
                    ),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                };
                let _ = stream.write_all(answer.as_bytes());
            }
        });
        spawned.map_err(|e| log::warn!("link previews: no player server: {e}")).ok()?;
        Some(port)
    })
}

/// Whether a preview loads at once or waits for a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Load,
    Ask,
}

/// The host of an http(s) link, lowercased, without "www." nor a final dot.
pub fn host(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.trim_end_matches('.').to_lowercase();
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

/// `host` is `domain` or one of its subdomains.
fn in_domain(host: &str, domain: &str) -> bool {
    let domain = domain.trim().trim_start_matches("www.").to_lowercase();
    !domain.is_empty() && (host == domain || host.strip_suffix(&domain).is_some_and(|sub| sub.ends_with('.')))
}

/// A YouTube video id: 11 letters, digits, `-` or `_`.
fn video_id(s: &str) -> Option<String> {
    (s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).then(|| s.to_owned())
}

/// `t=` / `start=` of a link: "90", "90s", "1m30s", "1h2m3s".
fn start_seconds(v: &str) -> u32 {
    if let Ok(s) = v.parse::<u32>() {
        return s;
    }
    let (mut total, mut n) = (0u32, 0u32);
    for c in v.chars() {
        match c {
            '0'..='9' => n = n.saturating_mul(10).saturating_add(c as u32 - '0' as u32),
            'h' => total = total.saturating_add(n.saturating_mul(3600)),
            'm' => total = total.saturating_add(n.saturating_mul(60)),
            's' => total = total.saturating_add(n),
            _ => return 0,
        }
        if !c.is_ascii_digit() {
            n = 0;
        }
    }
    total
}

/// What a link shows, if anything: a YouTube video, or a picture (a file
/// whose name says so; the answer's type decides in the end).
pub fn target(url: &str) -> Option<Target> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = host(url)?;
    let host = host.strip_prefix("m.").or_else(|| host.strip_prefix("music.")).unwrap_or(&host);
    let query = |key: &str| parsed.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
    let start = query("t").or_else(|| query("start")).map(|v| start_seconds(&v)).unwrap_or(0);
    let mut segments = parsed.path_segments().into_iter().flatten().filter(|s| !s.is_empty());
    let video = match host {
        "youtu.be" => segments.next().and_then(video_id),
        "youtube.com" | "youtube-nocookie.com" => match segments.next() {
            Some("watch") => query("v").as_deref().and_then(video_id),
            Some("shorts" | "embed" | "live" | "v") => segments.next().and_then(video_id),
            _ => None,
        },
        _ => None,
    };
    if let Some(id) = video {
        return Some(Target::Video(Video { id, start }));
    }
    let path = parsed.path().to_lowercase();
    let picture = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|ext| path.ends_with(ext));
    picture.then_some(Target::Picture)
}

/// The preview a link of a message gets in a conversation: none, or what it
/// shows and whether it loads by itself.
pub fn decide(s: &Settings, place: Place, url: &str) -> Option<(Target, Start)> {
    let on = match place {
        Place::Im => s.ims,
        Place::Group => s.groups,
        Place::Nearby => s.nearby,
    };
    if !on {
        return None;
    }
    let target = target(url)?;
    let host = host(url)?;
    if s.hidden_sites.iter().any(|d| in_domain(&host, d)) {
        return None;
    }
    let start = match crate::link_trust::classify(url) {
        Trust::Dangerous(_) => return None,
        Trust::Trusted if s.load != Load::Never => Start::Load,
        Trust::Unknown if s.load == Load::All => Start::Load,
        _ => Start::Ask,
    };
    Some((target, start))
}

/// A picture decoded for its card.
pub struct Picture {
    /// One frame, or those of an animated GIF with how long each shows.
    pub frames: Vec<(egui::ColorImage, Duration)>,
    /// Its size as sent, in pixels.
    pub size: [u32; 2],
    /// A small blurred copy (group chats, until clicked).
    pub blurred: egui::ColorImage,
}

/// What YouTube says of a video.
pub struct VideoInfo {
    pub title: String,
    pub channel: String,
    pub thumbnail: Option<Picture>,
    /// Width over height of the video itself (oEmbed's player size), when
    /// YouTube says it: a portrait short, an old 4:3 clip.
    pub aspect: Option<f32>,
}

pub enum Content {
    Picture(Picture),
    Video(VideoInfo),
}

/// Why a link has no picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Larger than `MAX_BYTES` (its size when the server said it).
    TooLarge(Option<u64>),
    /// The answer is not a picture (a web page, an unknown format).
    NotPicture,
    /// No answer, an error, a redirect off the trusted sites.
    Unavailable,
}

/// The clients previews download with: `trusted_only`, redirects must stay
/// on the trusted sites.
pub fn client(trusted_only: bool) -> Option<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        let off_trusted = trusted_only && crate::link_trust::classify(attempt.url().as_str()) != Trust::Trusted;
        if attempt.previous().len() >= MAX_REDIRECTS || off_trusted {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .user_agent(concat!("AuroraViewer/", env!("CARGO_PKG_VERSION"), " (link preview)"))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .retry(reqwest::retry::never())
        .redirect(policy)
        .build()
        .map_err(|e| log::warn!("link previews: no HTTP client: {e}"))
        .ok()
}

/// The body of `url`, at most `max` bytes, and its content type.
async fn download(client: &reqwest::Client, url: &str, accept: &str, max: u64) -> Result<(Vec<u8>, String), Failure> {
    let mut resp = client.get(url).header(reqwest::header::ACCEPT, accept).send().await.map_err(|e| {
        log::debug!("link preview {url}: {e}");
        Failure::Unavailable
    })?;
    if !resp.status().is_success() {
        log::debug!("link preview {url}: {}", resp.status());
        return Err(Failure::Unavailable);
    }
    let kind = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if let Some(len) = resp.content_length()
        && len > max
    {
        return Err(Failure::TooLarge(Some(len)));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| Failure::Unavailable)? {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > max {
            return Err(Failure::TooLarge(None));
        }
    }
    Ok((body, kind))
}

/// Download and decode the picture of `url`.
pub async fn fetch_picture(client: &reqwest::Client, url: &str) -> Result<Picture, Failure> {
    fetch_picture_sized(client, url, MAX_SIDE).await
}

/// Only the single open enlargement needs this resolution (at most 16 MiB
/// of pixels). Cards and animated GIFs retain their existing memory budgets.
pub async fn fetch_enlarged_picture(client: &reqwest::Client, url: &str) -> Result<Picture, Failure> {
    fetch_picture_sized(client, url, 2048).await
}

async fn fetch_picture_sized(client: &reqwest::Client, url: &str, side: u32) -> Result<Picture, Failure> {
    let (body, kind) = download(client, url, "image/webp,image/png,image/*;q=0.8", MAX_BYTES).await?;
    // a picture says so, or says nothing precise about itself
    if !(kind.starts_with("image/") || kind.is_empty() || kind.starts_with("application/octet-stream")) {
        return Err(Failure::NotPicture);
    }
    tokio::task::spawn_blocking(move || {
        if side == MAX_SIDE {
            decode(&body)
        } else {
            decode_sized(&body, side)
        }
    })
    .await
    .unwrap_or(Err(Failure::NotPicture))
}

/// Title, channel and picture of a video.
pub async fn fetch_video(client: &reqwest::Client, video: &Video) -> Result<VideoInfo, Failure> {
    // a private or removed video has no oEmbed: its card still opens it
    let about = match download(client, &video.oembed(), "application/json", 64 * 1024).await {
        Ok((body, _)) => serde_json::from_slice::<serde_json::Value>(&body).ok(),
        Err(_) => None,
    };
    let text = |key: &str| {
        about
            .as_ref()
            .and_then(|v| v[key].as_str())
            .map(|s| s.chars().take(200).collect::<String>())
            .unwrap_or_default()
    };
    let thumbnail = fetch_picture(client, &video.thumbnail()).await.ok();
    if about.is_none() && thumbnail.is_none() {
        return Err(Failure::Unavailable);
    }
    let aspect = about.as_ref().and_then(|v| aspect(v["width"].as_f64()?, v["height"].as_f64()?));
    Ok(VideoInfo {
        title: text("title"),
        channel: text("author_name"),
        thumbnail,
        aspect,
    })
}

/// A video's width over height, from sizes that make sense only.
fn aspect(w: f64, h: f64) -> Option<f32> {
    let a = (w / h) as f32;
    (w > 0.0 && h > 0.0 && a.is_finite() && (0.2..=5.0).contains(&a)).then_some(a)
}

fn color_image(img: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

/// The blurred copy: small, blurred again, drawn stretched.
fn blurred(img: &image::DynamicImage) -> egui::ColorImage {
    let small = img.resize(48, 48, image::imageops::FilterType::Triangle).to_rgba8();
    color_image(&image::imageops::blur(&small, 3.0))
}

/// Fit `(w, h)` within `side` on its longest side.
fn fit(w: u32, h: u32, side: u32) -> (u32, u32) {
    if w.max(h) <= side {
        return (w, h);
    }
    let scale = side as f32 / w.max(h) as f32;
    (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}

/// Decode a picture (PNG, JPEG, GIF, WebP, BMP), its frames when animated.
pub fn decode(bytes: &[u8]) -> Result<Picture, Failure> {
    decode_sized(bytes, MAX_SIDE)
}

fn decode_sized(bytes: &[u8], side: u32) -> Result<Picture, Failure> {
    let format = image::guess_format(bytes).map_err(|_| Failure::NotPicture)?;
    if format == image::ImageFormat::Gif {
        return decode_gif(bytes);
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(DECODE_LIMIT);
    limits.max_image_height = Some(DECODE_LIMIT);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| {
        log::debug!("link preview: cannot decode: {e}");
        Failure::NotPicture
    })?;
    let size = [img.width(), img.height()];
    let (w, h) = fit(img.width(), img.height(), side);
    let kept = if (w, h) == (img.width(), img.height()) {
        img
    } else {
        img.resize(w, h, image::imageops::FilterType::Triangle)
    };
    Ok(Picture {
        frames: vec![(color_image(&kept.to_rgba8()), Duration::ZERO)],
        size,
        blurred: blurred(&kept),
    })
}

/// How long a GIF frame shows: browsers draw a delay under 20 ms as 100.
fn frame_delay(delay: image::Delay) -> Duration {
    let (n, d) = delay.numer_denom_ms();
    let ms = n.checked_div(d).unwrap_or(0);
    Duration::from_millis(if ms < 20 { 100 } else { ms as u64 })
}

fn decode_gif(bytes: &[u8]) -> Result<Picture, Failure> {
    use image::{AnimationDecoder, ImageDecoder};
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|_| Failure::NotPicture)?;
    let (gw, gh) = decoder.dimensions();
    if gw == 0 || gh == 0 || gw > DECODE_LIMIT || gh > DECODE_LIMIT {
        return Err(Failure::NotPicture);
    }
    let (w, h) = fit(gw, gh, GIF_MAX_SIDE);
    let mut frames = Vec::new();
    let mut used = 0usize;
    for frame in decoder.into_frames() {
        let Ok(frame) = frame else { break };
        let delay = frame_delay(frame.delay());
        let buffer = frame.into_buffer();
        let buffer = if (w, h) == buffer.dimensions() {
            buffer
        } else {
            image::imageops::resize(&buffer, w, h, image::imageops::FilterType::Triangle)
        };
        used += (w * h * 4) as usize;
        if used > GIF_BUDGET || frames.len() >= GIF_MAX_FRAMES {
            break;
        }
        frames.push((color_image(&buffer), delay));
    }
    let first = frames.first().ok_or(Failure::NotPicture)?;
    let first = image::RgbaImage::from_raw(w, h, first.0.pixels.iter().flat_map(|c| c.to_array()).collect())
        .map(image::DynamicImage::ImageRgba8)
        .ok_or(Failure::NotPicture)?;
    Ok(Picture {
        blurred: blurred(&first),
        frames,
        size: [gw, gh],
    })
}

/// Demo mode: the content of a link made here, without network. A link
/// whose file is named `large…` is too large, `missing…` unavailable; a
/// `.gif` is animated; a video is the one of the demo's line.
pub fn demo(target: &Target, url: &str) -> Result<Content, Failure> {
    let file = url.rsplit('/').next().unwrap_or("").to_lowercase();
    if file.starts_with("large") {
        return Err(Failure::TooLarge(Some(24 * 1024 * 1024)));
    }
    if file.starts_with("missing") {
        return Err(Failure::Unavailable);
    }
    match target {
        // a short is upright; the zoo clip is 4:3, as on YouTube
        Target::Video(_) if url.contains("/shorts/") => Ok(Content::Video(VideoInfo {
            title: "Aurora short".into(),
            channel: "Aurora".into(),
            thumbnail: Some(demo_still(480, 360, 1)),
            aspect: Some(9.0 / 16.0),
        })),
        Target::Video(_) => Ok(Content::Video(VideoInfo {
            title: "Me at the zoo".into(),
            channel: "jawed".into(),
            thumbnail: Some(demo_still(480, 360, 1)),
            aspect: Some(4.0 / 3.0),
        })),
        Target::Picture if file.ends_with(".gif") => Ok(Content::Picture(demo_gif())),
        Target::Picture => Ok(Content::Picture(demo_still(640, 480, 0))),
    }
}

/// A landscape drawn here: a sky, a sun, two ridges.
fn demo_still(w: u32, h: u32, variant: u32) -> Picture {
    let sky = if variant == 0 { [42, 47, 74] } else { [29, 33, 53] };
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let sun = ((fx - 0.75).powi(2) + (fy - 0.28).powi(2)).sqrt() < 0.09;
        let ridge = fy > 0.75 - 0.25 * (fx * 7.0).sin().abs() * (1.0 - fx * 0.5);
        let front = fy > 0.86 - 0.08 * (fx * 4.0 + 1.0).sin();
        let rgb = if front {
            [79, 70, 229]
        } else if ridge {
            [61, 58, 110]
        } else if sun {
            [252, 211, 77]
        } else {
            let t = 1.0 - fy * 0.4;
            [(sky[0] as f32 * t) as u8, (sky[1] as f32 * t) as u8, (sky[2] as f32 * t) as u8]
        };
        image::Rgba([rgb[0], rgb[1], rgb[2], 255])
    });
    let img = image::DynamicImage::ImageRgba8(img);
    Picture {
        frames: vec![(color_image(&img.to_rgba8()), Duration::ZERO)],
        size: [w, h],
        blurred: blurred(&img),
    }
}

/// Three dots going round: an animated GIF of the demo.
fn demo_gif() -> Picture {
    let (w, h) = (240, 180);
    let frames: Vec<(egui::ColorImage, Duration)> = (0..12)
        .map(|i| {
            let a = i as f32 / 12.0 * std::f32::consts::TAU;
            let img = image::RgbaImage::from_fn(w, h, |x, y| {
                let dots = [([244, 114, 182], 0.0), ([94, 234, 212], 2.1), ([252, 211, 77], 4.2)];
                for (rgb, phase) in dots {
                    let (cx, cy) = (120.0 + 50.0 * (a + phase).cos(), 90.0 + 40.0 * (a + phase).sin());
                    if (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2) < 22.0f32.powi(2) {
                        return image::Rgba([rgb[0], rgb[1], rgb[2], 255]);
                    }
                }
                image::Rgba([38, 42, 64, 255])
            });
            (color_image(&img), Duration::from_millis(80))
        })
        .collect();
    let first = image::DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(w, h, frames[0].0.pixels.iter().flat_map(|c| c.to_array()).collect()).unwrap_or_default(),
    );
    Picture {
        blurred: blurred(&first),
        frames,
        size: [w, h],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str, start: u32) -> Option<Target> {
        Some(Target::Video(Video { id: id.into(), start }))
    }

    #[test]
    fn youtube_links_are_videos_with_their_start() {
        assert_eq!(target("https://www.youtube.com/watch?v=jNQXAC9IVRw"), video("jNQXAC9IVRw", 0));
        assert_eq!(target("https://youtu.be/jNQXAC9IVRw?t=90"), video("jNQXAC9IVRw", 90));
        assert_eq!(
            target("https://m.youtube.com/watch?v=jNQXAC9IVRw&t=1m30s"),
            video("jNQXAC9IVRw", 90)
        );
        assert_eq!(target("https://youtube.com/shorts/jNQXAC9IVRw"), video("jNQXAC9IVRw", 0));
        assert_eq!(
            target("https://www.youtube.com/live/jNQXAC9IVRw?t=1h2m3s"),
            video("jNQXAC9IVRw", 3723)
        );
        assert_eq!(target("https://music.youtube.com/watch?v=jNQXAC9IVRw"), video("jNQXAC9IVRw", 0));
        // not a video: a channel, a short id, another site's "watch"
        assert_eq!(target("https://www.youtube.com/@channel"), None);
        assert_eq!(target("https://www.youtube.com/watch?v=short"), None);
        assert_eq!(target("https://example.com/watch?v=jNQXAC9IVRw"), None);
    }

    #[test]
    fn pictures_are_named_by_their_file() {
        assert_eq!(target("https://i.imgur.com/k3Xr9Qa.JPG"), Some(Target::Picture));
        assert_eq!(target("https://i.gyazo.com/5f2c0e.gif?x=1"), Some(Target::Picture));
        assert_eq!(target("http://pics.example.net/scene.webp"), Some(Target::Picture));
        assert_eq!(target("https://imgur.com/gallery/abc"), None);
        assert_eq!(target("ftp://example.net/scene.png"), None);
        assert_eq!(target("https://example.net/scene.png.html"), None);
    }

    #[test]
    fn the_rules_decide_what_loads_by_itself() {
        let s = Settings::default();
        let trusted = "https://i.imgur.com/a.png";
        let unknown = "https://pics.example.net/a.png";
        assert_eq!(decide(&s, Place::Im, trusted), Some((Target::Picture, Start::Load)));
        assert_eq!(decide(&s, Place::Im, unknown), Some((Target::Picture, Start::Ask)));
        // the nearby chat is off by default, groups on
        assert_eq!(decide(&s, Place::Nearby, trusted), None);
        assert_eq!(decide(&s, Place::Group, trusted), Some((Target::Picture, Start::Load)));
        // a link that hides its destination never gets one
        assert_eq!(decide(&s, Place::Im, "https://bit.ly/a.png"), None);
        assert_eq!(decide(&s, Place::Im, "http://192.168.1.2/a.png"), None);
        let never = Settings {
            load: Load::Never,
            ..s.clone()
        };
        assert_eq!(decide(&never, Place::Im, trusted), Some((Target::Picture, Start::Ask)));
        let all = Settings {
            load: Load::All,
            ..s.clone()
        };
        assert_eq!(decide(&all, Place::Im, unknown), Some((Target::Picture, Start::Load)));
        // a hidden site, its subdomains included, not a look-alike
        let hidden = Settings {
            hidden_sites: vec!["imgur.com".into()],
            ..s
        };
        assert_eq!(decide(&hidden, Place::Im, trusted), None);
        assert!(decide(&hidden, Place::Im, "https://notimgur.com/a.png").is_some());
    }

    #[test]
    fn the_player_starts_where_the_link_said() {
        let v = Video {
            id: "jNQXAC9IVRw".into(),
            start: 90,
        };
        assert_eq!(
            v.player(&PlayerOptions::default()),
            "https://www.youtube-nocookie.com/embed/jNQXAC9IVRw?autoplay=1&rel=0&enablejsapi=1&hl=en&start=90"
        );
        assert!(v.oembed().ends_with("v%3DjNQXAC9IVRw"));
    }

    /// The settings become the player's options: its volume in YouTube's
    /// 0-100, the interface in the viewer's language, the subtitles in the
    /// one chosen or the viewer's.
    #[test]
    fn the_settings_set_the_players_volume_language_and_subtitles() {
        let mut s = Settings::default();
        assert_eq!(
            s.player_options("fr"),
            PlayerOptions {
                volume: 100,
                subtitles: None,
                language: "fr".into()
            }
        );
        s.video_volume = 0.304;
        s.subtitles = true;
        assert_eq!(s.player_options("fr").volume, 30);
        assert_eq!(s.player_options("fr").subtitles.as_deref(), Some("fr"));
        s.subtitle_language = "pt-BR".into();
        assert_eq!(s.player_options("fr").subtitles.as_deref(), Some("pt-BR"));
        // a code that could break the address is ignored
        s.subtitle_language = "fr&x=1".into();
        assert_eq!(s.player_options("de").subtitles.as_deref(), Some("de"));
        assert_eq!(s.player_options("<b>").language, "en");
        s.video_volume = 7.0;
        assert_eq!(s.player_options("en").volume, 100);
        let v = Video {
            id: "jNQXAC9IVRw".into(),
            start: 0,
        };
        assert!(
            v.player(&s.player_options("de"))
                .ends_with("enablejsapi=1&hl=de&cc_load_policy=1&cc_lang_pref=de")
        );
    }

    #[test]
    fn a_video_s_aspect_comes_from_sizes_that_make_sense() {
        assert_eq!(aspect(200.0, 150.0), Some(4.0 / 3.0));
        assert_eq!(aspect(113.0, 200.0), Some(113.0 / 200.0));
        assert_eq!(aspect(0.0, 150.0), None);
        assert_eq!(aspect(200.0, 0.0), None);
        assert_eq!(aspect(5000.0, 10.0), None);
    }

    /// The local page serves a checked video only: nothing of the request
    /// reaches the page but an id and a number.
    #[test]
    fn the_player_page_holds_a_checked_video_only() {
        let page = player_answer("GET /youtube/jNQXAC9IVRw?start=5&vol=30&hl=fr&cc=de HTTP/1.1", 4321).unwrap();
        assert!(page.contains(concat!(
            "src=\"https://www.youtube-nocookie.com/embed/jNQXAC9IVRw?autoplay=1&rel=0&enablejsapi=1&hl=fr",
            "&cc_load_policy=1&cc_lang_pref=de&start=5&amp;origin="
        )));
        // the volume kept reaches the page's script
        assert!(page.contains("v=30,"));
        // options that could break the page fall back to safe values
        let page = player_answer("GET /youtube/jNQXAC9IVRw?vol=900&hl=%3Cb%3E&cc=x\" HTTP/1.1", 4321).unwrap();
        assert!(page.contains("v=100,"));
        assert!(page.contains("enablejsapi=1&hl=en&amp;origin"));
        assert!(page.contains("referrerpolicy=\"strict-origin-when-cross-origin\""));
        assert!(player_answer("GET /youtube/<script>x</s HTTP/1.1", 4321).is_none());
        assert!(player_answer("GET /youtube/jNQXAC9IVRw\"onload HTTP/1.1", 4321).is_none());
        assert!(player_answer("GET /etc/passwd HTTP/1.1", 4321).is_none());
        assert!(player_answer("POST /youtube/jNQXAC9IVRw HTTP/1.1", 4321).is_none());
        // a start that is not a number is ignored
        assert!(
            player_answer("GET /youtube/jNQXAC9IVRw?start=x\" HTTP/1.1", 4321)
                .unwrap()
                .contains("hl=en&amp;origin")
        );
    }

    /// The page reports the player's volume to `/volume`, a number from 0
    /// to 100 and nothing else.
    #[test]
    fn the_player_reports_its_volume_as_a_number_only() {
        assert_eq!(volume_answer("GET /volume?v=30 HTTP/1.1"), Some(30));
        assert_eq!(volume_answer("GET /volume?v=0 HTTP/1.1"), Some(0));
        assert_eq!(volume_answer("GET /volume?v=101 HTTP/1.1"), None);
        assert_eq!(volume_answer("GET /volume?v=-1 HTTP/1.1"), None);
        assert_eq!(volume_answer("POST /volume?v=30 HTTP/1.1"), None);
        assert_eq!(volume_answer("GET /youtube/jNQXAC9IVRw HTTP/1.1"), None);
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn pictures_are_kept_at_the_size_they_are_drawn() {
        let p = decode(&png(1600, 1200)).unwrap();
        assert_eq!(p.size, [1600, 1200]);
        assert_eq!(p.frames.len(), 1);
        assert_eq!(p.frames[0].0.size, [800, 600]);
        assert!(p.blurred.size[0] <= 48);
        let detail = decode_sized(&png(2400, 1200), 2048).unwrap();
        assert_eq!(detail.size, [2400, 1200]);
        assert_eq!(detail.frames[0].0.size, [2048, 1024]);
        assert_eq!(decode(b"<html>not a picture</html>").err(), Some(Failure::NotPicture));
    }

    #[test]
    fn an_animated_gif_keeps_its_frames_and_delays() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            enc.set_repeat(Repeat::Infinite).unwrap();
            for (i, ms) in [(0u8, 50u32), (120, 0)] {
                let img = image::RgbaImage::from_pixel(600, 300, image::Rgba([i, 0, 0, 255]));
                let delay = image::Delay::from_numer_denom_ms(ms, 1);
                enc.encode_frame(image::Frame::from_parts(img, 0, 0, delay)).unwrap();
            }
        }
        let p = decode(&out).unwrap();
        assert_eq!(p.size, [600, 300]);
        assert_eq!(p.frames.len(), 2);
        assert_eq!(p.frames[0].0.size, [480, 240]);
        assert_eq!(p.frames[0].1, Duration::from_millis(50));
        // no delay: drawn as browsers do
        assert_eq!(p.frames[1].1, Duration::from_millis(100));
    }

    /// A server answering each connection with one of `answers`.
    fn server(answers: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            for answer in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(&answer);
            }
        });
        (base, thread)
    }

    /// A client for the test server: one connection per request.
    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[test]
    fn downloads_stop_at_the_limit_and_refuse_what_is_not_a_picture() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let client = test_client();
        let (base, thread) = server(vec![
            format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
                MAX_BYTES + 1
            )
            .into_bytes(),
            b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/html\r\nContent-Length: 4\r\n\r\nhtml".to_vec(),
            b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ]);
        let get = |path: &str| rt.block_on(fetch_picture(&client, &format!("{base}/{path}"))).err();
        assert_eq!(get("big.png"), Some(Failure::TooLarge(Some(MAX_BYTES + 1))));
        assert_eq!(get("page.png"), Some(Failure::NotPicture));
        assert_eq!(get("gone.png"), Some(Failure::Unavailable));
        thread.join().unwrap();
    }

    #[test]
    fn a_picture_downloads_and_decodes() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let client = test_client();
        let body = png(64, 32);
        let mut answer = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        answer.extend_from_slice(&body);
        let (base, thread) = server(vec![answer]);
        let p = rt.block_on(fetch_picture(&client, &format!("{base}/a.png"))).ok().unwrap();
        assert_eq!(p.size, [64, 32]);
        assert_eq!(p.frames[0].0.size, [64, 32]);
        thread.join().unwrap();
    }

    #[test]
    fn demo_links_show_each_state() {
        let pic = Target::Picture;
        assert!(matches!(demo(&pic, "https://i.imgur.com/demo.jpg"), Ok(Content::Picture(p)) if p.frames.len() == 1));
        assert!(matches!(demo(&pic, "https://i.gyazo.com/demo.gif"), Ok(Content::Picture(p)) if p.frames.len() == 12));
        assert_eq!(
            demo(&pic, "https://i.imgur.com/large.png").err(),
            Some(Failure::TooLarge(Some(24 * 1024 * 1024)))
        );
    }
}
