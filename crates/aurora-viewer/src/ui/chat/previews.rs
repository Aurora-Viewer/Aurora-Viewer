//! The cards under the links of a conversation (`link_preview.rs` decides,
//! downloads and decodes): a picture, an animated GIF or WebP, a YouTube
//! video or a video file of a trusted site (#983) that plays in its card or
//! in a window of its own, or a card that asks before anything reaches the
//! site. Drawn by all four styles of the Conversations
//! window and its detached conversations. The History window, transcript
//! history and chat lines at the bottom of the screen show links as text.
//!
//! A picture's card: hovered, buttons to open it in the browser, copy it or
//! hide the preview; a click enlarges it over the conversation (`lightbox`);
//! a right click also hides every preview of its site. A group's
//! pictures are blurred until clicked (Preferences › Chat), as are Reddit's
//! and those of a post its network marks sensitive. A preview hidden,
//! asked for or unblurred stays so until the viewer closes; a site hidden
//! from the menu is saved in the settings.

use super::{LineEnv, Seg, segments, web_target};
use crate::link_preview::{self as lp, Content, Failure, Place, Start, Target};
use crate::theme::Palette;
use crate::ui::web_view::WebView;
use crate::world::{ChatLine, Origin};
use egui::{Color32, RichText, Vec2};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

mod clipboard;
mod lightbox;
use lightbox::Lightbox;

/// Flush copied pictures after egui's final UI pass.
pub fn clipboard_update(ctx: &egui::Context, output: &mut egui::PlatformOutput) {
    clipboard::update(ctx, output);
}

/// Cards under one message at most.
const MAX_CARDS: usize = 3;
/// Links kept with their pictures; the least recently drawn go first.
const MAX_ENTRIES: usize = 48;
/// Downloads at once.
const MAX_IN_FLIGHT: usize = 4;
/// A video's card is wider than a picture's by this.
const VIDEO_EXTRA: f32 = 20.0;
/// Width of a video playing in its card, against the card's.
const PLAYING_SCALE: f32 = 1.375;
const BUTTON: f32 = 26.0;
const PLAY: f32 = 56.0;
/// The player window's default width, and the height of its title bar
/// until the window has been measured.
const WINDOW_WIDTH: f32 = 480.0;
const TITLE_BAR: f32 = 30.0;
/// A video's shape when YouTube does not say it.
const DEFAULT_ASPECT: f32 = 16.0 / 9.0;
/// A card never narrower than this under an upright video: its title and
/// buttons keep room.
const MIN_VIDEO_CARD: f32 = 200.0;
/// The player window's id (its place and size are remembered).
const PLAYER_WINDOW: &str = "link_preview_player";

/// A picture made into textures.
#[derive(Clone)]
struct Shown {
    frames: Vec<(egui::TextureHandle, Duration)>,
    /// One loop of the frames.
    period: Duration,
    blurred: egui::TextureHandle,
    /// As sent, in pixels.
    size: [u32; 2],
    /// The first frame, for the clipboard.
    copy: Arc<egui::ColorImage>,
    /// Where the picture came from when it is not the link itself (the
    /// page of an image service): the enlargement asks it again.
    source: Option<String>,
    /// A post its network marks sensitive: blurred until clicked.
    sensitive: bool,
}

impl Shown {
    fn new(ctx: &egui::Context, name: &str, p: lp::Picture) -> Shown {
        let copy = Arc::new(p.frames[0].0.clone());
        let period = p.frames.iter().map(|f| f.1).sum();
        let frames = p
            .frames
            .into_iter()
            .enumerate()
            .map(|(i, (img, d))| (ctx.load_texture(format!("{name}#{i}"), img, egui::TextureOptions::LINEAR), d))
            .collect();
        Shown {
            frames,
            period,
            blurred: ctx.load_texture(format!("{name}#blur"), p.blurred, egui::TextureOptions::LINEAR),
            size: p.size,
            copy,
            source: None,
            sensitive: false,
        }
    }

    fn animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// The frame shown at `time`, and how long until the next one.
    fn frame(&self, time: f64) -> (&egui::TextureHandle, Option<Duration>) {
        if !self.animated() || self.period.is_zero() {
            return (&self.frames[0].0, None);
        }
        let mut at = Duration::from_secs_f64(time.max(0.0)).as_millis() % self.period.as_millis().max(1);
        for (t, d) in &self.frames {
            let ms = d.as_millis();
            if at < ms {
                return (t, Some(Duration::from_millis((ms - at) as u64)));
            }
            at -= ms;
        }
        (&self.frames[0].0, Some(self.frames[0].1))
    }

    /// Its size in a card `width` wide: within a square of that side, never
    /// larger than sent.
    fn fit(&self, width: f32) -> Vec2 {
        let (w, h) = (self.size[0].max(1) as f32, self.size[1].max(1) as f32);
        let scale = (width / w).min(width / h).min(1.0);
        Vec2::new((w * scale).round(), (h * scale).round())
    }
}

enum Entry {
    /// Asked for, waiting for a download slot.
    Queued,
    Loading,
    Picture(Shown),
    Video {
        title: String,
        channel: String,
        thumbnail: Option<Shown>,
        /// Width over height of the video.
        aspect: f32,
        /// A video file, not a YouTube video.
        clip: Option<lp::Clip>,
    },
    Failed(Failure),
}

struct Cached {
    entry: Entry,
    /// The pass it was last drawn in.
    used: u64,
}

/// A video playing, in its card or in the player window.
struct Playing {
    key: u64,
    link: String,
    title: String,
    web: WebView,
    window: bool,
    /// The pass its card last drew it in.
    drawn: u64,
    /// Width over height of the video: the card and the window keep it.
    aspect: f32,
    /// The window was given the video's shape since this video started
    /// playing in it.
    fitted: bool,
    /// What the player window adds around the picture (its title bar), as
    /// last measured: its edges keep the picture's shape while dragged.
    chrome: Vec2,
}

/// What the player needs of the app: the media plugins, the browser's
/// settings and the media volume.
pub struct WebEnv {
    pub paths: Option<aurora_media::PluginPaths>,
    pub browser: aurora_media::BrowserSettings,
    /// Master × media channel volume (0 when muted), as for the media on
    /// faces.
    pub volume: f32,
}

impl WebEnv {
    /// Draw the player. The plugin's browser starts muted
    /// (MediaPluginCEF::mCurVolume), so it gets the volume every frame: the
    /// plugin sends it only when it changes.
    fn show(&self, web: &mut WebView, ui: &mut egui::Ui, p: &Palette, size: Vec2) {
        web.show(ui, p, self.paths.as_ref(), &self.browser, None, size);
        if let Some(plugin) = web.plugin_mut() {
            plugin.set_volume(self.volume);
        }
    }
}

type Answer = (String, Result<Content, Failure>);

/// The previews of the Conversations window (part of `ChatUi`).
pub struct Previews {
    /// Preferences › Chat, copied by the app every frame.
    pub settings: lp::Settings,
    /// The conversation drawn (set before its lines are).
    pub place: Place,
    /// Demo mode: the pictures are made here (`link_preview::demo`).
    pub demo: bool,
    /// Set by the app every frame; no player without it.
    pub web: Option<WebEnv>,
    /// A site whose previews the resident hid from a card's menu, for the
    /// app to add to the settings.
    pub hide_site: Option<String>,
    /// `AURORA_DEMO_PREVIEW_PLAY=card|window`: the first video card drawn
    /// plays there by itself.
    pub demo_play: Option<lp::Player>,
    /// A player's cog was clicked: the app opens the video settings.
    pub open_settings: bool,
    cache: HashMap<String, Cached>,
    queue: VecDeque<(String, Target, bool)>,
    in_flight: usize,
    tx: mpsc::Sender<Answer>,
    rx: mpsc::Receiver<Answer>,
    runtime: Option<tokio::runtime::Runtime>,
    /// Clients whose redirects stay on the trusted sites, and the others.
    clients: Option<(reqwest::Client, reqwest::Client)>,
    /// Messages whose preview was asked for (their "Show" clicked).
    asked: HashSet<u64>,
    hidden: HashSet<u64>,
    /// Blurred pictures clicked.
    revealed: HashSet<u64>,
    playing: Option<Playing>,
    lightbox: Option<Lightbox>,
    image_window: Option<Lightbox>,
    /// Offline capture: enlarge the first still/GIF, optionally zoomed/panned.
    demo_zoom: Option<String>,
}

impl Default for Previews {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Previews {
            settings: lp::Settings::default(),
            place: Place::Im,
            demo: false,
            web: None,
            hide_site: None,
            demo_play: None,
            open_settings: false,
            cache: HashMap::new(),
            queue: VecDeque::new(),
            in_flight: 0,
            tx,
            rx,
            runtime: None,
            clients: None,
            asked: HashSet::new(),
            hidden: HashSet::new(),
            revealed: HashSet::new(),
            playing: None,
            lightbox: None,
            image_window: None,
            demo_zoom: std::env::var("AURORA_DEMO_PREVIEW_ZOOM").ok(),
        }
    }
}

/// The web links of a message, each once, at most `MAX_CARDS`.
fn links(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in segments(text) {
        let url = match seg {
            Seg::Url(u) => u,
            Seg::Labelled(_, inner) => match *inner {
                Seg::Url(u) => u,
                _ => continue,
            },
            _ => continue,
        };
        let url = web_target(url).into_owned();
        if !out.contains(&url) {
            out.push(url);
        }
        if out.len() == MAX_CARDS {
            break;
        }
    }
    out
}

/// One preview of one message.
fn key(line: &ChatLine, url: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (line.time, line.source, &line.text, url).hash(&mut h);
    h.finish()
}

/// Size of a file, for a card.
fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    // "24 MB", "8 MB", "1.5 MB"
    let whole = mb >= 10.0 || (mb - mb.round()).abs() < 0.05;
    let n = aurora_i18n::number(mb, if whole { 0 } else { 1 });
    aurora_i18n::tr!("ui-chat-preview-megabytes", n = n.as_str())
}

impl Previews {
    pub fn enlarged(&self) -> bool {
        self.lightbox.is_some()
    }

    pub fn image_window_open(&self) -> bool {
        self.image_window.is_some()
    }

    /// Before the conversation's editor and shortcuts receive keys.
    pub(super) fn lightbox_keys(&mut self, ctx: &egui::Context) {
        let owner = egui::Id::unique("conversations");
        if self.enlarged()
            && ctx.memory(|m| m.top_modal_layer()).is_none()
            && (crate::ui::close_key::take(ctx, egui::Id::unique("conversations"))
                || (ctx
                    .top_layer_id()
                    .is_none_or(|layer| layer.id == owner || layer.id == owner.with("image_enlargement"))
                    && ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)
                    })))
        {
            self.close_lightbox(ctx);
        }
    }

    fn close_lightbox(&mut self, ctx: &egui::Context) {
        if let Some(view) = self.lightbox.take()
            && let Some(field) = view.previous_focus
        {
            ctx.memory_mut(|m| m.request_focus(field));
        }
    }

    /// Keep the image within its owning conversation, including when resized.
    pub(super) fn lightbox(&mut self, ctx: &egui::Context, p: &Palette, bounds: Option<egui::Rect>) {
        let Some(bounds) = bounds else {
            self.close_lightbox(ctx);
            return;
        };
        if self.lightbox.as_mut().is_some_and(|view| view.show(ctx, p, bounds)) {
            self.close_lightbox(ctx);
        }
    }

    fn enlarge(&mut self, ctx: &egui::Context, url: &str, trusted_only: bool, window: bool) {
        let Some(Cached {
            entry: Entry::Picture(shown),
            ..
        }) = self.cache.get(url)
        else {
            return;
        };
        let mut view = Lightbox::new(ctx, url.to_owned(), shown.clone());
        if self.demo {
            if let Some(mode) = self.demo_zoom.take() {
                view.demo(&mode);
            }
        } else if !shown.animated()
            && shown.size.iter().copied().max().unwrap_or(0) > shown.frames[0].0.size().into_iter().max().unwrap_or(0) as u32
            && let (Some(rt), Some((trusted, any))) = (&self.runtime, &self.clients)
        {
            let client = if trusted_only { trusted.clone() } else { any.clone() };
            let url = shown.source.clone().unwrap_or_else(|| url.to_owned());
            let (tx, rx) = mpsc::channel();
            view.pending = Some(rx);
            let wake = ctx.clone();
            view.task = Some(rt.spawn(async move {
                let _ = tx.send(lp::fetch_enlarged_picture(&client, &url).await);
                wake.request_repaint();
            }));
        }
        if window {
            self.image_window = Some(view);
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("link_preview_image")));
        } else {
            self.lightbox = Some(view);
        }
    }

    /// Answers in, downloads out; the player of a card no longer drawn
    /// stops. Once per frame, before the lines are drawn.
    pub fn update(&mut self, ctx: &egui::Context) {
        while let Ok((url, result)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let entry = match result {
                Ok(Content::Picture(p)) => Entry::Picture(Shown::new(ctx, &url, p)),
                Ok(Content::Page {
                    picture,
                    source,
                    sensitive,
                }) => Entry::Picture(Shown {
                    source: Some(source),
                    sensitive,
                    ..Shown::new(ctx, &url, picture)
                }),
                Ok(Content::Video(v)) => Entry::Video {
                    title: v.title,
                    channel: v.channel,
                    thumbnail: v.thumbnail.map(|p| Shown::new(ctx, &url, p)),
                    aspect: v.aspect.unwrap_or(DEFAULT_ASPECT),
                    clip: None,
                },
                Ok(Content::Clip(c)) => Entry::Video {
                    title: String::new(),
                    channel: String::new(),
                    thumbnail: c.thumbnail.map(|p| Shown::new(ctx, &url, p)),
                    aspect: c.aspect.unwrap_or(DEFAULT_ASPECT),
                    clip: Some(c.clip),
                },
                Err(f) => Entry::Failed(f),
            };
            if let Some(c) = self.cache.get_mut(&url) {
                c.entry = entry;
                ctx.request_repaint();
            }
        }
        // a clip's page tells its shape once it plays: its card and the
        // player window take it
        if let Some((key, aspect)) = lp::take_clip_aspect()
            && let Some(v) = self.playing.as_mut().filter(|v| v.key == key)
        {
            if (v.aspect - aspect).abs() > 0.01 {
                v.aspect = aspect;
                v.fitted = false;
            }
            if let Some(Cached {
                entry: Entry::Video { aspect: kept, .. },
                ..
            }) = self.cache.get_mut(&v.link)
            {
                *kept = aspect;
            }
            ctx.request_repaint();
        }
        while self.in_flight < MAX_IN_FLIGHT
            && let Some((url, target, trusted_only)) = self.queue.pop_front()
        {
            self.start(url, target, trusted_only);
        }
        if self.cache.len() > MAX_ENTRIES {
            let mut idle: Vec<(u64, String)> = self
                .cache
                .iter()
                .filter(|(url, c)| {
                    !matches!(c.entry, Entry::Queued | Entry::Loading)
                        && self.lightbox.as_ref().is_none_or(|view| &view.url != *url)
                        && self.image_window.as_ref().is_none_or(|view| &view.url != *url)
                })
                .map(|(u, c)| (c.used, u.clone()))
                .collect();
            idle.sort();
            let extra = self.cache.len() - MAX_ENTRIES;
            for (_, url) in idle.into_iter().take(extra) {
                self.cache.remove(&url);
            }
        }
    }

    /// Ask for `url`'s content (once).
    fn request(&mut self, url: &str, target: &Target, trusted_only: bool, pass: u64) {
        if let Some(c) = self.cache.get_mut(url) {
            c.used = pass;
            return;
        }
        self.cache.insert(
            url.to_owned(),
            Cached {
                entry: Entry::Queued,
                used: pass,
            },
        );
        self.queue.push_back((url.to_owned(), target.clone(), trusted_only));
    }

    fn start(&mut self, url: String, target: Target, trusted_only: bool) {
        if let Some(c) = self.cache.get_mut(&url) {
            c.entry = Entry::Loading;
        }
        if self.demo {
            // made here, no network; a "loading…" file never arrives
            if !url.rsplit('/').next().unwrap_or("").starts_with("loading") {
                self.in_flight += 1;
                let _ = self.tx.send((url.clone(), lp::demo(&target, &url)));
            }
            return;
        }
        if self.runtime.is_none() {
            match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("link-preview")
                .enable_all()
                .build()
            {
                Ok(rt) => self.runtime = Some(rt),
                Err(e) => log::warn!("link previews: no runtime: {e}"),
            }
        }
        if self.clients.is_none() {
            self.clients = lp::client(true).zip(lp::client(false));
        }
        let (Some(rt), Some((trusted, any))) = (&self.runtime, &self.clients) else {
            if let Some(c) = self.cache.get_mut(&url) {
                c.entry = Entry::Failed(Failure::Unavailable);
            }
            return;
        };
        let client = if trusted_only { trusted.clone() } else { any.clone() };
        let tx = self.tx.clone();
        self.in_flight += 1;
        rt.spawn(async move {
            let result = match &target {
                Target::Picture => lp::fetch_picture(&client, &url).await.map(Content::Picture),
                Target::Video(v) => lp::fetch_video(&client, v).await.map(Content::Video),
                // nothing reaches the site before it plays
                Target::Clip(clip) => Ok(Content::Clip(lp::ClipInfo {
                    clip: clip.clone(),
                    thumbnail: None,
                    aspect: None,
                })),
                Target::Page(_) => lp::fetch_page(&client, &url, trusted_only).await,
            };
            let _ = tx.send((url, result));
        });
    }

    /// A video plays: the app gives the player its browser (`web`).
    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// The widest card a message gets, for a bubble to make room (0: none).
    pub(super) fn width(&self, line: &ChatLine, text: &str) -> f32 {
        if line.origin != Origin::Live {
            return 0.0;
        }
        let w = self.settings.size.width();
        links(text)
            .iter()
            .filter_map(|url| lp::decide(&self.settings, self.place, url).map(|(target, _)| (url, target)))
            .map(|(url, target)| match target {
                // a Gyazo page may lead to a video capture
                Target::Picture | Target::Page(_) if !matches!(self.cache.get(url).map(|c| &c.entry), Some(Entry::Video { .. })) => w,
                _ => w + VIDEO_EXTRA,
            })
            .fold(0.0, f32::max)
    }

    /// The video playing in a window: its window; the one of a card no
    /// longer drawn (conversation closed or changed, scrolled away) stops.
    /// Every frame, from the app.
    pub fn player_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if self.image_window.as_mut().is_some_and(|view| view.window(ctx, p)) {
            self.image_window = None;
        }
        let pass = ctx.cumulative_pass_nr();
        if self.playing.as_ref().is_some_and(|v| !v.window && pass > v.drawn + 2) {
            self.playing = None;
        }
        let Some(v) = self.playing.as_mut().filter(|v| v.window) else {
            return;
        };
        let Some(web) = &self.web else {
            return;
        };
        let title = if v.title.is_empty() {
            aurora_i18n::tr!("ui-chat-preview-player").to_owned()
        } else {
            v.title.clone()
        };
        let mut open = true;
        let (mut browser, mut copy_link, mut settings) = (false, false, false);
        let work = crate::ui::layout::work_area(ctx);
        let default = window_size(WINDOW_WIDTH, v.chrome, v.aspect, work.height() * 0.8);
        let pos = work.center() - default * 0.5;
        let body = crate::ui::widgets::Floater::new(PLAYER_WINDOW, title, pos, default)
            .action("settings-filled", aurora_i18n::tr!("ui-chat-preview-settings"), &mut settings)
            .action("external-link-filled", &link_tip(&v.link, false), &mut browser)
            .action("link-filled", &link_tip(&v.link, true), &mut copy_link)
            .flush_body()
            .keep_aspect(v.aspect, v.chrome)
            .show(ctx, p, &mut open, |ui| {
                let size = ui.available_size().max(Vec2::splat(64.0));
                web.show(&mut v.web, ui, p, size);
                size
            });
        // the window keeps the video's shape (#848): its edges hold it at
        // every frame of a drag (#941); outside a drag its height follows
        // its width when another video starts in it, or when the title
        // bar measured differently
        let outer = ctx.memory(|m| m.area_rect(egui::Id::unique(PLAYER_WINDOW)));
        if let (Some(body), Some(outer)) = (body, outer)
            && !crate::ui::widgets::floater_minimized(ctx, PLAYER_WINDOW)
        {
            v.chrome = (outer.size() - body).max(Vec2::ZERO);
        }
        if let (Some(_), Some(outer)) = (body, outer)
            && !ctx.input(|i| i.pointer.any_down())
            && !crate::ui::widgets::floater_minimized(ctx, PLAYER_WINDOW)
        {
            // a window the user sized may be as tall as the room; a video
            // starting in it opens at most at 80 % of it
            let (width, max_height) = if v.fitted {
                (outer.width(), work.height())
            } else {
                (outer.width().max(160.0), work.height() * 0.8)
            };
            let want = window_size(width, v.chrome, v.aspect, max_height);
            if (want - outer.size()).abs().max_elem() > 1.5 {
                crate::ui::widgets::size_floater(ctx, PLAYER_WINDOW, want);
            }
            v.fitted = true;
        }
        if browser {
            super::open_web_link(ctx, &v.link);
        }
        if copy_link {
            ctx.copy_text(v.link.clone());
        }
        if settings {
            self.open_settings = true;
        }
        if !open {
            self.playing = None;
        }
    }
}

/// The player window's size for a video of `aspect` (width over height):
/// `width` wide, `chrome` around the picture (its title bar above it), and
/// no taller than `max_height` (an upright video narrows instead). The same
/// shape its edges keep while dragged (`Floater::keep_aspect`).
fn window_size(width: f32, chrome: Vec2, aspect: f32, max_height: f32) -> Vec2 {
    let aspect = aspect.clamp(0.2, 5.0);
    let mut picture = width.max(160.0) - chrome.x;
    if chrome.y + picture / aspect > max_height {
        picture = ((max_height - chrome.y).max(90.0) * aspect).max(160.0 - chrome.x);
    }
    Vec2::new((picture + chrome.x).round(), (chrome.y + picture / aspect).round())
}

/// A video's size in a card `width` wide: within a square of that side, as
/// a picture's, so an upright short stays as tall as a landscape card is
/// wide (#848).
fn video_size(width: f32, aspect: f32) -> Vec2 {
    let aspect = aspect.clamp(0.2, 5.0);
    if aspect >= 1.0 {
        Vec2::new(width.round(), (width / aspect).round())
    } else {
        Vec2::new((width * aspect).round(), width.round())
    }
}

/// The part of a thumbnail that holds the picture: YouTube's thumbnails
/// are 4:3 with black bands around a picture of another shape.
fn thumbnail_uv(size: [u32; 2], aspect: f32) -> egui::Rect {
    let thumb = size[0].max(1) as f32 / size[1].max(1) as f32;
    let aspect = aspect.clamp(0.2, 5.0);
    if aspect > thumb {
        let h = thumb / aspect;
        egui::Rect::from_min_max(egui::pos2(0.0, (1.0 - h) * 0.5), egui::pos2(1.0, (1.0 + h) * 0.5))
    } else {
        let w = aspect / thumb;
        egui::Rect::from_min_max(egui::pos2((1.0 - w) * 0.5, 0.0), egui::pos2((1.0 + w) * 0.5, 1.0))
    }
}

/// Only a ready card replaces its raw URL. Keep labelled links as authored,
/// and retain the original message for history, search and message copying.
pub(super) fn text<'a>(pv: Option<&Previews>, line: &ChatLine, text: &'a str, width: f32) -> std::borrow::Cow<'a, str> {
    let Some(pv) = pv.filter(|_| line.origin == Origin::Live) else {
        return text.into();
    };
    let ready: Vec<String> = links(text)
        .into_iter()
        .filter(|url| {
            let Some((_, start)) = lp::decide(&pv.settings, pv.place, url) else {
                return false;
            };
            let k = key(line, url);
            if pv.hidden.contains(&k) || (start == Start::Ask && !pv.asked.contains(&k)) {
                return false;
            }
            match pv.cache.get(url).map(|c| &c.entry) {
                Some(Entry::Picture(shown)) => {
                    let blurred =
                        (pv.place == Place::Group && pv.settings.blur_groups) || lp::blurred_until_clicked(url) || shown.sensitive;
                    let size = shown.fit(pv.settings.size.width().min(width.max(80.0)));
                    // Leave tiny images' links in the text: their hover toolbar
                    // cannot fit all actions without covering the image.
                    size.x >= 122.0 && size.y >= BUTTON + 16.0 && (!blurred || pv.revealed.contains(&k))
                }
                Some(Entry::Video { .. }) => true,
                _ => false,
            }
        })
        .collect();
    if ready.is_empty() {
        return text.into();
    }
    let mut rest = text;
    let mut out = String::with_capacity(text.len());
    while let Some((at, len, seg)) = super::next_link(rest) {
        out.push_str(&rest[..at]);
        if !matches!(seg, Seg::Url(url) if ready.iter().any(|u| u == web_target(url).as_ref())) {
            out.push_str(&rest[at..at + len]);
        }
        rest = &rest[at + len..];
    }
    out.push_str(rest);
    out.trim().to_owned().into()
}

/// The cards of a message, under its text: one per link that gets one.
pub(super) fn cards(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, text: &str) {
    let Some(pv) = env.previews.as_deref_mut() else {
        return;
    };
    // the history of a transcript stays text: opening a conversation does
    // not fetch the links of its past
    if line.origin != Origin::Live {
        return;
    }
    let p = env.p;
    for url in links(text) {
        let Some((target, start)) = lp::decide(&pv.settings, pv.place, &url) else {
            continue;
        };
        let k = key(line, &url);
        ui.add_space(4.0);
        card(ui, p, pv, k, &url, &target, start);
    }
}

fn card(ui: &mut egui::Ui, p: &Palette, pv: &mut Previews, k: u64, url: &str, target: &Target, start: Start) {
    let pass = ui.ctx().cumulative_pass_nr();
    let width = pv.settings.size.width().min(ui.available_width().max(80.0));
    if pv.hidden.contains(&k) {
        hidden_line(ui, p, pv, k);
        return;
    }
    if start == Start::Ask && !pv.asked.contains(&k) {
        if ask_card(ui, p, url, target, width + VIDEO_EXTRA) {
            pv.asked.insert(k);
        }
        return;
    }
    // a load allowed by the trust of the site stays on trusted sites
    let trusted_only = start == Start::Load && pv.settings.load == lp::Load::Trusted;
    pv.request(url, target, trusted_only, pass);
    let Some(cached) = pv.cache.get(url) else {
        return;
    };
    // a group's pictures as set in Preferences, Reddit's always, a post its
    // network marks sensitive
    let marked = matches!(&cached.entry, Entry::Picture(shown) if shown.sensitive);
    let blur =
        ((pv.place == Place::Group && pv.settings.blur_groups) || lp::blurred_until_clicked(url) || marked) && !pv.revealed.contains(&k);
    let mut enlarge = false;
    let mut window = false;
    let resp = match (&cached.entry, target) {
        (Entry::Queued | Entry::Loading, Target::Picture | Target::Page(_)) => placeholder(ui, p, Vec2::new(width, (width * 0.75).round())),
        (Entry::Queued | Entry::Loading, Target::Video(_) | Target::Clip(_)) => {
            let w = width + VIDEO_EXTRA;
            placeholder(ui, p, Vec2::new(w, (w * 9.0 / 16.0).round()))
        }
        (Entry::Picture(shown), _) => {
            let out = picture(ui, p, shown, url, width, blur);
            if !blur && pv.demo && pv.demo_zoom.as_deref() == Some("links") {
                let button = BUTTON.min((out.resp.rect.width() - 32.0) / 5.0);
                note_link_hover(
                    ui,
                    out.resp.rect,
                    egui::pos2(
                        out.resp.rect.right() - 8.0 - button * 0.5 - 4.0 * (button + 4.0),
                        out.resp.rect.top() + 8.0 + button * 0.5,
                    ),
                );
            }
            if !blur && pv.demo && pv.demo_zoom.as_deref() == Some("copy") {
                let visible = out.resp.rect.intersect(ui.clip_rect());
                if visible.is_positive() {
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(egui::Id::unique("preview_copy_demo"), (out.resp.rect, visible)));
                }
            }
            match out.action {
                Some(Action::Reveal) => {
                    pv.revealed.insert(k);
                }
                Some(Action::Hide) => {
                    pv.hidden.insert(k);
                }
                Some(Action::Copy) => clipboard::request(ui.ctx(), &shown.copy),
                Some(Action::CopyLink) => ui.ctx().copy_text(url.to_owned()),
                Some(Action::Open) => super::open_web_link(ui.ctx(), url),
                Some(Action::Enlarge) => enlarge = true,
                Some(Action::Window) => window = true,
                None => {}
            }
            if !blur
                && pv.demo
                && pv
                    .demo_zoom
                    .as_ref()
                    .is_some_and(|mode| !matches!(mode.as_str(), "copy" | "links") && (mode == "gif") == shown.animated())
            {
                window |= pv.demo_zoom.as_deref() == Some("window");
                enlarge |= !window;
            }
            out.resp
        }
        // a YouTube video, a clip, or the clip a Gyazo page leads to
        (Entry::Video { .. }, _) => {
            let video = match target {
                Target::Video(video) => Some(video.clone()),
                _ => None,
            };
            video_card(ui, p, pv, k, url, video.as_ref(), width + VIDEO_EXTRA);
            return;
        }
        (Entry::Failed(Failure::TooLarge(size)), _) => {
            let size = *size;
            too_large(ui, p, url, size, width + VIDEO_EXTRA);
            return;
        }
        // not a picture, unavailable: the link stays a link
        _ => return,
    };
    if enlarge || window {
        pv.enlarge(ui.ctx(), url, trusted_only, window);
    }
    card_menu(&resp, p, pv, k, url);
}

/// Exercise the actual hovered Copy button, including platform clipboard delivery.
pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    static LINKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *LINKS.get_or_init(|| std::env::var("AURORA_DEMO_PREVIEW_ZOOM").is_ok_and(|mode| mode == "links")) && frame >= 620 {
        return ctx
            .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("preview_links_demo")))
            .filter(|at| ctx.input(|i| i.pointer.hover_pos()) != Some(*at))
            .map(|at| vec![egui::Event::PointerMoved(at)])
            .unwrap_or_default();
    }
    static COPY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*COPY.get_or_init(|| std::env::var("AURORA_DEMO_PREVIEW_ZOOM").is_ok_and(|mode| mode == "copy")) || !(620..=760).contains(&frame) {
        return Vec::new();
    }
    let id = egui::Id::unique("preview_copy_demo");
    let Some((rect, visible)) = ctx.data(|d| d.get_temp::<(egui::Rect, egui::Rect)>(id)) else {
        return Vec::new();
    };
    let at = egui::pos2(rect.right() - 8.0 - BUTTON * 1.5 - 4.0, rect.top() + 8.0 + BUTTON * 0.5);
    if !visible.contains(at) {
        return if frame.is_multiple_of(30) {
            ctx.data_mut(|d| d.insert_temp(id.with("scrolled"), frame));
            vec![
                egui::Event::PointerMoved(visible.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 260.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        } else {
            Vec::new()
        };
    }
    if ctx
        .data(|d| d.get_temp::<u64>(id.with("scrolled")))
        .is_some_and(|at| frame < at + 30)
    {
        return Vec::new();
    }
    let step = ctx.data(|d| d.get_temp::<usize>(id.with("step"))).unwrap_or(0);
    if step >= 3 {
        return Vec::new();
    }
    ctx.data_mut(|d| d.insert_temp(id.with("step"), step + 1));
    let mut events = vec![egui::Event::PointerMoved(at)];
    if step > 0 {
        events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: step == 1,
            modifiers: egui::Modifiers::NONE,
        });
    }
    events
}

fn note_link_hover(ui: &egui::Ui, rect: egui::Rect, at: egui::Pos2) {
    if rect.intersect(ui.clip_rect()).contains(at) {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("preview_links_demo"), at));
    }
}

/// What the resident did on a picture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Enlarge,
    Window,
    Open,
    Copy,
    CopyLink,
    Hide,
    Reveal,
}

struct PictureOut {
    resp: egui::Response,
    action: Option<Action>,
}

/// The empty card a preview takes while it loads.
fn placeholder(ui: &mut egui::Ui, p: &Palette, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    paint_icon(
        ui.painter(),
        "photo-filled",
        egui::Rect::from_center_size(rect.center(), Vec2::splat(28.0)),
        p.muted_dim,
    );
    resp
}

fn paint_icon(painter: &egui::Painter, icon: &str, rect: egui::Rect, color: Color32) {
    super::thread::paint_icon(painter, icon, rect, color);
}

/// A square button over a picture; true when clicked.
fn overlay_button(ui: &mut egui::Ui, p: &Palette, rect: egui::Rect, icon: &str, tip: &str) -> bool {
    let resp = ui.interact(
        rect,
        ui.scope_id().with(("preview_button", icon, rect.min.x as i32, rect.min.y as i32)),
        egui::Sense::click(),
    );
    let fill = if resp.hovered() { p.raised } else { p.field };
    ui.painter()
        .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill.gamma_multiply(0.9));
    paint_icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), Vec2::splat(15.0)),
        p.ink,
    );
    resp.on_hover_text(tip).clicked()
}

/// A picture's card: the picture (or its blur), the GIF mark, and the
/// buttons while hovered.
fn picture(ui: &mut egui::Ui, p: &Palette, shown: &Shown, url: &str, width: f32, blur: bool) -> PictureOut {
    let size = shown.fit(width);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter().with_clip_rect(rect);
    let mut action = None;
    if blur {
        egui::Image::new(&shown.blurred).corner_radius(radius).paint_at(ui, rect);
        let label = aurora_i18n::tr!("ui-chat-preview-see");
        let galley = painter.layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), p.ink);
        let pill = egui::Rect::from_center_size(rect.center(), galley.size() + Vec2::new(36.0, 14.0));
        let hover = resp.hovered();
        ui.painter().rect_filled(
            pill,
            ui.visuals().widgets.inactive.corner_radius,
            if hover { p.raised } else { p.field }.gamma_multiply(0.92),
        );
        paint_icon(
            ui.painter(),
            "eye-filled",
            egui::Rect::from_center_size(egui::pos2(pill.left() + 15.0, pill.center().y), Vec2::splat(14.0)),
            p.ink,
        );
        ui.painter().galley(
            egui::pos2(pill.left() + 26.0, pill.center().y - galley.size().y * 0.5),
            galley,
            p.ink,
        );
        if resp.clicked() {
            action = Some(Action::Reveal);
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        return PictureOut { resp, action };
    }
    let (frame, next) = shown.frame(ui.input(|i| i.time));
    egui::Image::new(frame).corner_radius(radius).paint_at(ui, rect);
    if let Some(next) = next {
        ui.ctx().request_repaint_after(next);
    }
    if shown.animated() {
        let galley = painter.layout_no_wrap("GIF".to_owned(), egui::FontId::proportional(10.5), p.ink);
        let mark = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.bottom() - 8.0 - galley.size().y - 4.0),
            galley.size() + Vec2::new(12.0, 4.0),
        );
        painter.rect_filled(mark, radius, p.field.gamma_multiply(0.85));
        painter.galley(mark.min + Vec2::new(6.0, 2.0), galley, p.ink);
    }
    // the buttons over the picture while the pointer is on it
    let hovered = ui.rect_contains_pointer(rect);
    let button = BUTTON.min((rect.width() - 32.0) / 5.0);
    if hovered && button >= 18.0 {
        let mut x = rect.right() - 8.0 - button;
        let y = rect.top() + 8.0;
        for (icon, tip, act) in [
            ("eye-off", aurora_i18n::tr!("ui-chat-preview-hide").to_owned(), Action::Hide),
            ("copy-filled", clipboard::tip(ui.ctx(), &shown.copy).to_owned(), Action::Copy),
            (
                "photo-filled",
                aurora_i18n::tr!("ui-chat-preview-detach-image").to_owned(),
                Action::Window,
            ),
            ("external-link-filled", link_tip(url, false), Action::Open),
            ("link-filled", link_tip(url, true), Action::CopyLink),
        ] {
            let b = egui::Rect::from_min_size(egui::pos2(x, y), Vec2::splat(button));
            if overlay_button(ui, p, b, icon, &tip) {
                action = Some(act);
            }
            x -= button + 4.0;
        }
    }
    if action.is_none() && resp.clicked() {
        action = Some(Action::Enlarge);
    }
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::ZoomIn)
        .on_hover_text(aurora_i18n::tr!("ui-chat-preview-enlarge"));
    PictureOut { resp, action }
}

fn link_tip(url: &str, copy: bool) -> String {
    let action = if copy {
        aurora_i18n::tr!("ui-chat-preview-copy-link")
    } else {
        aurora_i18n::tr!("ui-chat-preview-open")
    };
    format!("{action}\n{url}")
}

/// "Preview hidden · Show".
fn hidden_line(ui: &mut egui::Ui, p: &Palette, pv: &mut Previews, k: u64) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
        paint_icon(ui.painter(), "eye-off", r, p.muted_dim);
        ui.label(
            RichText::new(format!("{} ·", aurora_i18n::tr!("ui-chat-preview-hidden")))
                .size(12.0)
                .color(p.muted),
        );
        let show = ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("ui-chat-preview-show"))
                    .size(12.0)
                    .color(p.violet_light),
            )
            .sense(egui::Sense::click()),
        );
        if show.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            pv.hidden.remove(&k);
        }
    });
}

/// A compact card: an icon, a title and a line under it, and what goes at
/// its end.
fn compact_card<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    width: f32,
    icon: &str,
    title: &str,
    note: &str,
    end: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(width - 24.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (r, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
                ui.painter().rect_filled(r, ui.visuals().window_corner_radius, p.panel);
                paint_icon(
                    ui.painter(),
                    icon,
                    egui::Rect::from_center_size(r.center(), Vec2::splat(20.0)),
                    p.muted,
                );
                let end_w = 96.0;
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - end_w).max(60.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(title).size(13.0).strong().color(p.ink)).truncate());
                    ui.add(egui::Label::new(RichText::new(note).size(11.5).color(p.muted)).wrap());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), end).inner
            })
            .inner
        })
        .inner
}

/// The card of a preview that waits for a click; true when "Show" is
/// clicked.
fn ask_card(ui: &mut egui::Ui, p: &Palette, url: &str, target: &Target, width: f32) -> bool {
    let site = lp::host(url).unwrap_or_default();
    let (icon, title) = match target {
        Target::Picture | Target::Page(_) => (
            "photo-filled",
            aurora_i18n::tr!("ui-chat-preview-picture-from", site = site.as_str()),
        ),
        Target::Video(_) => (
            "brand-youtube-filled",
            aurora_i18n::tr!("ui-chat-preview-video-from", site = site.as_str()),
        ),
        // no filled icon of a film strip
        Target::Clip(_) => ("movie", aurora_i18n::tr!("ui-chat-preview-video-from", site = site.as_str())),
    };
    compact_card(ui, p, width, icon, &title, aurora_i18n::tr!("ui-chat-preview-not-loaded"), |ui| {
        crate::ui::widgets::primary_button_sized(ui, p, aurora_i18n::tr!("ui-chat-preview-show"), Vec2::new(84.0, 28.0)).clicked()
    })
}

/// A picture too large to download: its size, and the link to open it.
fn too_large(ui: &mut egui::Ui, p: &Palette, url: &str, size: Option<u64>, width: f32) {
    let limit = megabytes(lp::MAX_BYTES);
    let title = match size {
        Some(n) => aurora_i18n::tr!("ui-chat-preview-too-large", size = megabytes(n)),
        None => aurora_i18n::tr!("ui-chat-preview-too-large-unknown", limit = limit.as_str()),
    };
    let note = aurora_i18n::tr!("ui-chat-preview-over-limit", limit = limit.as_str());
    let open = compact_card(ui, p, width, "photo-filled", &title, &note, |ui| {
        crate::ui::widgets::icon_button(ui, p, "external-link-filled", aurora_i18n::tr!("ui-chat-preview-open"), true)
    });
    if open {
        super::open_web_link(ui.ctx(), url);
    }
}

/// A YouTube video (`video`) or a clip: its picture with the play button,
/// its title and channel (a clip: its site); playing, the player in place of
/// the picture with its buttons under it.
fn video_card(ui: &mut egui::Ui, p: &Palette, pv: &mut Previews, k: u64, url: &str, video: Option<&lp::Video>, width: f32) {
    let pass = ui.ctx().cumulative_pass_nr();
    let Some(Cached {
        entry: Entry::Video {
            title,
            channel,
            thumbnail,
            aspect,
            clip,
        },
        ..
    }) = pv.cache.get(url)
    else {
        return;
    };
    let aspect = *aspect;
    let clip = clip.clone();
    if clip.is_none() && video.is_none() {
        return;
    }
    let title = if title.is_empty() {
        if clip.is_some() {
            aurora_i18n::tr!("ui-chat-preview-clip")
        } else {
            aurora_i18n::tr!("ui-chat-preview-video")
        }
        .to_owned()
    } else {
        title.clone()
    };
    let channel = match (&clip, channel.is_empty()) {
        (Some(_), _) => lp::host(url).unwrap_or_default(),
        (None, true) => "YouTube".to_owned(),
        (None, false) => format!("{channel} · YouTube"),
    };
    let here = pv.playing.as_ref().filter(|v| v.key == k).map(|v| v.window);
    let width = if here == Some(false) {
        (width * PLAYING_SCALE).min(ui.available_width().max(width))
    } else {
        width
    };
    // the video in its own shape; the card no narrower than its text needs
    let size = video_size(width, aspect);
    let card_width = size.x.max(MIN_VIDEO_CARD.min(width));
    // the player's buttons show while the pointer is over the card (#848)
    let hover_id = ui.scope_id().with(("video_card_hover", k));
    let last = ui.ctx().data(|d| d.get_temp::<egui::Rect>(hover_id));
    let over = last.is_some_and(|r| ui.rect_contains_pointer(r));
    let shown = ui.ctx().animate_bool(hover_id, over);
    let mut play = false;
    let (mut detach, mut stop, mut settings) = (false, false, false);
    let resp = egui::Frame::new()
        .fill(p.raised)
        .corner_radius(ui.visuals().window_corner_radius)
        .show(ui, |ui| {
            ui.set_width(card_width);
            ui.spacing_mut().item_spacing.y = 0.0;
            let (row, _) = ui.allocate_exact_size(Vec2::new(card_width, size.y), egui::Sense::hover());
            let area = egui::Rect::from_center_size(row.center(), size);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            match (here, pv.playing.as_mut(), pv.web.as_ref()) {
                (Some(false), Some(v), Some(web)) => {
                    v.drawn = pass;
                    web.show(&mut v.web, &mut child, p, size);
                }
                _ => {
                    let ui = &mut child;
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    let radius = ui.visuals().window_corner_radius;
                    match thumbnail {
                        Some(t) => {
                            let corners = egui::CornerRadius {
                                nw: if area.left() <= row.left() { radius.nw } else { 0 },
                                ne: if area.right() >= row.right() { radius.ne } else { 0 },
                                sw: 0,
                                se: 0,
                            };
                            egui::Image::new(&t.frames[0].0)
                                .uv(thumbnail_uv(t.size, aspect))
                                .corner_radius(corners)
                                .paint_at(ui, rect);
                        }
                        None => {
                            ui.painter().rect_filled(rect, radius, p.field);
                            paint_icon(
                                ui.painter(),
                                if clip.is_some() { "movie" } else { "brand-youtube-filled" },
                                egui::Rect::from_center_size(rect.center(), Vec2::splat(40.0)),
                                p.muted_dim,
                            );
                        }
                    }
                    if here == Some(true) {
                        // playing in the window
                        ui.painter().rect_filled(rect, 0.0, p.navy.gamma_multiply(0.6));
                        let text = aurora_i18n::tr!("ui-chat-preview-playing-window");
                        let galley = ui
                            .painter()
                            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.0), p.violet_pale);
                        let x = rect.center().x - (galley.size().x + 22.0) * 0.5;
                        paint_icon(
                            ui.painter(),
                            "picture-in-picture-filled",
                            egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), Vec2::splat(16.0)),
                            p.violet_pale,
                        );
                        ui.painter()
                            .galley(egui::pos2(x + 22.0, rect.center().y - galley.size().y * 0.5), galley, p.violet_pale);
                    } else {
                        let disc = egui::Rect::from_center_size(rect.center(), Vec2::splat(PLAY));
                        let hover = resp.hovered();
                        ui.painter().circle_filled(
                            disc.center(),
                            PLAY * 0.5,
                            if hover { p.violet } else { p.navy.gamma_multiply(0.75) },
                        );
                        paint_icon(
                            ui.painter(),
                            "player-play-filled",
                            egui::Rect::from_center_size(disc.center() + Vec2::new(2.0, 0.0), Vec2::splat(24.0)),
                            if hover { p.on_accent } else { p.ink },
                        );
                        if resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(aurora_i18n::tr!("ui-chat-preview-play"))
                            .clicked()
                        {
                            play = true;
                        }
                    }
                }
            }
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 10,
                    right: 8,
                    top: 8,
                    bottom: 10,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // room kept for the buttons while it plays here, so
                        // the title does not move when they appear
                        let buttons = if here == Some(false) { 5.0 } else { 2.0 } * (20.0 + 6.0);
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - buttons).max(60.0));
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(egui::Label::new(RichText::new(&title).size(13.5).strong().color(p.ink)).truncate());
                            ui.add(egui::Label::new(RichText::new(&channel).size(11.5).color(p.muted)).truncate());
                        });
                        if shown > 0.01 {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.multiply_opacity(shown);
                                if crate::ui::widgets::icon_button(ui, p, "external-link-filled", &link_tip(url, false), true) {
                                    super::open_web_link(ui.ctx(), url);
                                }
                                if crate::ui::widgets::icon_button(ui, p, "link-filled", &link_tip(url, true), true) {
                                    ui.ctx().copy_text(url.to_owned());
                                }
                                if here != Some(false) {
                                    return;
                                }
                                stop = crate::ui::widgets::icon_button(
                                    ui,
                                    p,
                                    "player-stop-filled",
                                    aurora_i18n::tr!("ui-chat-preview-stop"),
                                    true,
                                );
                                detach = crate::ui::widgets::icon_button(
                                    ui,
                                    p,
                                    "picture-in-picture-filled",
                                    aurora_i18n::tr!("ui-chat-preview-detach"),
                                    true,
                                );
                                settings = crate::ui::widgets::icon_button(
                                    ui,
                                    p,
                                    "settings-filled",
                                    aurora_i18n::tr!("ui-chat-preview-settings"),
                                    true,
                                );
                            });
                        }
                    });
                });
        })
        .response;
    ui.ctx().data_mut(|d| d.insert_temp(hover_id, resp.rect));
    if pv.demo && pv.demo_zoom.as_deref() == Some("links") {
        note_link_hover(ui, resp.rect, egui::pos2(resp.rect.right() - 18.0, resp.rect.bottom() - 26.0));
    }
    let demo = pv.playing.is_none().then(|| pv.demo_play.take()).flatten();
    if play || demo.is_some() {
        let mut options = pv.settings.player_options(aurora_i18n::language());
        // AURORA_DEMO_VIDEO_VOLUME=<0-100>: the level a video starts at,
        // as if kept from an earlier one
        if let Some(v) = demo_video_volume() {
            options.volume = v;
        }
        pv.playing = Some(Playing {
            key: k,
            link: url.to_owned(),
            title: title.clone(),
            web: {
                let mut web = WebView::default();
                match (&clip, video) {
                    (Some(clip), _) => web.navigate(&clip.player_page(&options, k)),
                    (None, Some(video)) => web.navigate(&video.player_page(&options)),
                    (None, None) => {}
                }
                web
            },
            window: demo.unwrap_or(pv.settings.player) == lp::Player::Window,
            drawn: pass,
            aspect,
            fitted: false,
            chrome: Vec2::new(0.0, TITLE_BAR),
        });
    }
    if detach && let Some(v) = pv.playing.as_mut() {
        v.window = true;
        v.fitted = false;
    }
    if stop {
        pv.playing = None;
    }
    if settings {
        pv.open_settings = true;
    }
    card_menu(&resp, p, pv, k, url);
}

/// `AURORA_DEMO_VIDEO_VOLUME=<0-100>` (demo only): the player's starting level.
fn demo_video_volume() -> Option<u8> {
    static LEVEL: std::sync::OnceLock<Option<u8>> = std::sync::OnceLock::new();
    *LEVEL.get_or_init(|| {
        std::env::var_os("AURORA_DEMO")
            .is_some()
            .then(|| std::env::var("AURORA_DEMO_VIDEO_VOLUME").ok()?.trim().parse::<u8>().ok())
            .flatten()
            .map(|v| v.min(100))
    })
}

/// Right click on a card: open, copy the link, hide this preview, hide the
/// previews of its site.
fn card_menu(resp: &egui::Response, p: &Palette, pv: &mut Previews, k: u64, url: &str) {
    let site = lp::host(url).unwrap_or_default();
    crate::ui::menu::context_menu(resp, p, |ui| {
        if crate::ui::menu::item(ui, p, "external-link-filled", aurora_i18n::tr!("ui-chat-preview-open")) {
            super::open_web_link(ui.ctx(), url);
        }
        if crate::ui::menu::item(ui, p, "copy-filled", aurora_i18n::tr!("ui-chat-preview-copy-link")) {
            ui.ctx().copy_text(url.to_owned());
        }
        crate::ui::menu::separator(ui, p);
        if crate::ui::menu::item(ui, p, "eye-off", aurora_i18n::tr!("ui-chat-preview-hide")) {
            pv.hidden.insert(k);
        }
        if !site.is_empty()
            && crate::ui::menu::item(
                ui,
                p,
                "eye-off",
                &aurora_i18n::tr!("ui-chat-preview-hide-site", site = site.as_str()),
            )
        {
            pv.hide_site = Some(site.clone());
        }
    });
    super::line_menu::inline(resp);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_media_replace_only_their_raw_links_and_restore_them_when_unavailable() {
        let ctx = egui::Context::default();
        let url = "https://i.imgur.com/demo.gif";
        let mut line = ChatLine {
            time: std::time::SystemTime::UNIX_EPOCH,
            from: "Demo".into(),
            text: url.into(),
            kind: crate::world::ChatKind::Im,
            source: uuid::Uuid::nil(),
            origin: Origin::Live,
        };
        let mut pv = Previews::default();
        let visible = |pv: &Previews, line: &ChatLine| text(Some(pv), line, &line.text, 320.0).into_owned();
        assert_eq!(visible(&pv, &line), url, "not requested");
        pv.cache.insert(
            url.into(),
            Cached {
                entry: Entry::Loading,
                used: 0,
            },
        );
        assert_eq!(visible(&pv, &line), url, "loading");
        let shown = Shown::new(
            &ctx,
            url,
            lp::Picture {
                frames: vec![(egui::ColorImage::filled([1, 1], Color32::GRAY), Duration::ZERO)],
                size: [640, 480],
                blurred: egui::ColorImage::filled([1, 1], Color32::GRAY),
            },
        );
        pv.cache.get_mut(url).unwrap().entry = Entry::Picture(shown);
        assert_eq!(visible(&pv, &line), "", "ready URL-only messages have no text row");
        assert_eq!(
            text(Some(&pv), &line, url, 100.0),
            url,
            "a narrow card keeps its accessible text link"
        );
        let authored = format!("Look: {url}! [https://example.org More] [https://i.imgur.com/demo.gif My caption]");
        assert_eq!(
            text(Some(&pv), &line, &authored, 320.0),
            "Look: ! [https://example.org More] [https://i.imgur.com/demo.gif My caption]"
        );
        assert_eq!(line.text, url, "the stored message is unchanged");
        pv.hidden.insert(key(&line, url));
        assert_eq!(visible(&pv, &line), url, "hidden preview");
        pv.hidden.clear();
        pv.settings.ims = false;
        assert_eq!(visible(&pv, &line), url, "disabled previews");
        pv.settings.ims = true;
        pv.settings.load = lp::Load::Never;
        assert_eq!(visible(&pv, &line), url, "cached media still requires consent");
        pv.asked.insert(key(&line, url));
        assert_eq!(visible(&pv, &line), "");
        pv.settings.load = lp::Load::Trusted;
        pv.place = Place::Group;
        assert_eq!(visible(&pv, &line), url, "blurred picture");
        pv.revealed.insert(key(&line, url));
        assert_eq!(visible(&pv, &line), "");
        pv.place = Place::Im;
        for origin in [Origin::Log, Origin::ServerLog] {
            line.origin = origin;
            assert_eq!(visible(&pv, &line), url, "history stays text");
        }
        line.origin = Origin::Live;
        for failure in [Failure::Unavailable, Failure::TooLarge(Some(99_000_000))] {
            pv.cache.get_mut(url).unwrap().entry = Entry::Failed(failure);
            assert_eq!(visible(&pv, &line), url);
        }
    }

    #[test]
    fn media_beyond_the_card_limit_keep_their_links() {
        let mut pv = Previews::default();
        let urls: Vec<_> = (0..4).map(|i| format!("https://youtu.be/DemoVideo{i:02}")).collect();
        for url in &urls {
            pv.cache.insert(
                url.clone(),
                Cached {
                    entry: Entry::Video {
                        title: String::new(),
                        channel: String::new(),
                        thumbnail: None,
                        aspect: DEFAULT_ASPECT,
                        clip: None,
                    },
                    used: 0,
                },
            );
        }
        let line = ChatLine {
            time: std::time::SystemTime::UNIX_EPOCH,
            from: "Demo".into(),
            text: urls.join("\n"),
            kind: crate::world::ChatKind::Im,
            source: uuid::Uuid::nil(),
            origin: Origin::Live,
        };
        assert_eq!(text(Some(&pv), &line, &line.text, 340.0), urls[3]);
        assert_eq!(
            link_tip(&urls[0], true),
            format!("{}\n{}", aurora_i18n::tr!("ui-chat-preview-copy-link"), urls[0])
        );
        assert_eq!(
            link_tip(&urls[0], false),
            format!("{}\n{}", aurora_i18n::tr!("ui-chat-preview-open"), urls[0])
        );
    }

    #[test]
    fn a_message_gets_a_card_per_link_once_and_three_at_most() {
        let text = "look https://i.imgur.com/a.png and https://i.imgur.com/a.png, \
                    [https://youtu.be/jNQXAC9IVRw the zoo] www.example.com/b.gif https://e.net/c.png https://e.net/d.png";
        assert_eq!(
            links(text),
            vec![
                "https://i.imgur.com/a.png".to_owned(),
                "https://youtu.be/jNQXAC9IVRw".to_owned(),
                "http://www.example.com/b.gif".to_owned(),
            ]
        );
    }

    #[test]
    fn a_gif_shows_each_frame_for_its_delay() {
        let ctx = egui::Context::default();
        let img = |v: u8| egui::ColorImage::new([2, 2], vec![Color32::from_gray(v); 4]);
        let shown = Shown::new(
            &ctx,
            "test",
            lp::Picture {
                frames: vec![(img(1), Duration::from_millis(100)), (img(2), Duration::from_millis(300))],
                size: [2, 2],
                blurred: img(0),
            },
        );
        assert_eq!(shown.period, Duration::from_millis(400));
        let (t, next) = shown.frame(0.05);
        assert_eq!(t.id(), shown.frames[0].0.id());
        assert_eq!(next, Some(Duration::from_millis(50)));
        let (t, next) = shown.frame(0.25);
        assert_eq!(t.id(), shown.frames[1].0.id());
        assert_eq!(next, Some(Duration::from_millis(150)));
        // the loop starts again
        assert_eq!(shown.frame(0.45).0.id(), shown.frames[0].0.id());
    }

    #[test]
    fn pictures_fit_their_card_without_growing() {
        let ctx = egui::Context::default();
        let make = |w: u32, h: u32| {
            Shown::new(
                &ctx,
                "fit",
                lp::Picture {
                    frames: vec![(egui::ColorImage::new([1, 1], vec![Color32::BLACK]), Duration::ZERO)],
                    size: [w, h],
                    blurred: egui::ColorImage::new([1, 1], vec![Color32::BLACK]),
                },
            )
        };
        assert_eq!(make(640, 480).fit(320.0), Vec2::new(320.0, 240.0));
        assert_eq!(make(300, 900).fit(320.0), Vec2::new(107.0, 320.0));
        assert_eq!(make(64, 32).fit(320.0), Vec2::new(64.0, 32.0));
    }

    /// A video keeps its shape in its card (#848): a landscape one as wide
    /// as the card, an upright one as tall as the card is wide.
    #[test]
    fn videos_keep_their_shape_in_their_card() {
        assert_eq!(video_size(320.0, 16.0 / 9.0), Vec2::new(320.0, 180.0));
        assert_eq!(video_size(320.0, 4.0 / 3.0), Vec2::new(320.0, 240.0));
        assert_eq!(video_size(320.0, 9.0 / 16.0), Vec2::new(180.0, 320.0));
        // nonsense from a site is held within bounds
        assert_eq!(video_size(320.0, 0.0), Vec2::new(64.0, 320.0));
    }

    /// A 4:3 thumbnail shows only the part that holds the picture.
    #[test]
    fn thumbnails_lose_their_black_bands() {
        let r = thumbnail_uv([480, 360], 16.0 / 9.0);
        assert!((r.top() - 0.125).abs() < 1e-4 && (r.bottom() - 0.875).abs() < 1e-4);
        assert_eq!((r.left(), r.right()), (0.0, 1.0));
        let r = thumbnail_uv([480, 360], 9.0 / 16.0);
        assert!((r.width() - 0.421_875).abs() < 1e-4 && (r.center().x - 0.5).abs() < 1e-4);
        assert_eq!(
            thumbnail_uv([480, 360], 4.0 / 3.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
        );
    }

    /// The player window is the video's shape under its title bar, and an
    /// upright video narrows the window rather than outgrowing the screen.
    #[test]
    fn the_player_window_takes_the_video_s_shape() {
        let bar = |y: f32| Vec2::new(0.0, y);
        assert_eq!(window_size(480.0, bar(30.0), 16.0 / 9.0, 900.0), Vec2::new(480.0, 300.0));
        assert_eq!(window_size(400.0, bar(30.0), 4.0 / 3.0, 900.0), Vec2::new(400.0, 330.0));
        let tall = window_size(480.0, bar(30.0), 9.0 / 16.0, 700.0);
        assert!(tall.y <= 700.0 && (tall.x - 377.0).abs() <= 1.0);
        // a width the user gave keeps, the height follows it
        assert_eq!(window_size(640.0, bar(32.0), 16.0 / 9.0, 2000.0), Vec2::new(640.0, 392.0));
        // a frame on the sides: the picture, not the window, has the shape
        assert_eq!(window_size(482.0, Vec2::new(2.0, 30.0), 16.0 / 9.0, 900.0), Vec2::new(482.0, 300.0));
    }
}
