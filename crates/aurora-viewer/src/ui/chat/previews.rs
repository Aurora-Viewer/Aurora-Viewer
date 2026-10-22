//! The cards under the links of a conversation (`link_preview.rs` decides,
//! downloads and decodes): a picture, an animated GIF, a YouTube video that
//! plays in its card or in a window of its own, or a card that asks before
//! anything reaches the site. Drawn by the Aurora, Compact and Bubbles
//! styles of the Conversations window; the Legacy lines, the History window
//! and the chat lines at the bottom of the screen show links as text, as the
//! Legacy Viewer does.
//!
//! A picture's card: hovered, buttons to open it in the browser, copy it or
//! hide the preview; a click opens the link (through the trust warning of
//! links); a right click also hides every preview of its site. A group's
//! pictures are blurred until clicked (Preferences › Chat). A preview hidden,
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
/// The player window's default size: a 16:9 picture under its title bar.
const WINDOW_SIZE: Vec2 = Vec2::new(480.0, 300.0);

/// A picture made into textures.
struct Shown {
    frames: Vec<(egui::TextureHandle, Duration)>,
    /// One loop of the frames.
    period: Duration,
    blurred: egui::TextureHandle,
    /// As sent, in pixels.
    size: [u32; 2],
    /// The first frame, for the clipboard.
    copy: Arc<egui::ColorImage>,
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
    /// Answers in, downloads out; the player of a card no longer drawn
    /// stops. Once per frame, before the lines are drawn.
    pub fn update(&mut self, ctx: &egui::Context) {
        while let Ok((url, result)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let entry = match result {
                Ok(Content::Picture(p)) => Entry::Picture(Shown::new(ctx, &url, p)),
                Ok(Content::Video(v)) => Entry::Video {
                    title: v.title,
                    channel: v.channel,
                    thumbnail: v.thumbnail.map(|p| Shown::new(ctx, &url, p)),
                },
                Err(f) => Entry::Failed(f),
            };
            if let Some(c) = self.cache.get_mut(&url) {
                c.entry = entry;
                ctx.request_repaint();
            }
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
                .filter(|(_, c)| !matches!(c.entry, Entry::Queued | Entry::Loading))
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
            .filter_map(|url| lp::decide(&self.settings, self.place, url))
            .map(|(target, _)| match target {
                Target::Picture => w,
                Target::Video(_) => w + VIDEO_EXTRA,
            })
            .fold(0.0, f32::max)
    }

    /// The video playing in a window: its window; the one of a card no
    /// longer drawn (conversation closed or changed, scrolled away) stops.
    /// Every frame, from the app.
    pub fn player_window(&mut self, ctx: &egui::Context, p: &Palette) {
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
        let mut browser = false;
        let work = crate::ui::layout::work_area(ctx);
        let pos = work.center() - WINDOW_SIZE * 0.5;
        crate::ui::widgets::Floater::new("link_preview_player", title, pos, WINDOW_SIZE)
            .action("external-link-filled", aurora_i18n::tr!("ui-chat-preview-open"), &mut browser)
            .show(ctx, p, &mut open, |ui| {
                let size = ui.available_size().max(Vec2::splat(64.0));
                web.show(&mut v.web, ui, p, size);
            });
        if browser {
            super::open_web_link(ctx, &v.link);
        }
        if !open {
            self.playing = None;
        }
    }
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
    let blur = pv.place == Place::Group && pv.settings.blur_groups && !pv.revealed.contains(&k);
    let Some(cached) = pv.cache.get(url) else {
        return;
    };
    let resp = match (&cached.entry, target) {
        (Entry::Queued | Entry::Loading, Target::Picture) => placeholder(ui, p, Vec2::new(width, (width * 0.75).round())),
        (Entry::Queued | Entry::Loading, Target::Video(_)) => {
            let w = width + VIDEO_EXTRA;
            placeholder(ui, p, Vec2::new(w, (w * 9.0 / 16.0).round()))
        }
        (Entry::Picture(shown), _) => {
            let out = picture(ui, p, shown, width, blur);
            match out.action {
                Some(Action::Reveal) => {
                    pv.revealed.insert(k);
                }
                Some(Action::Hide) => {
                    pv.hidden.insert(k);
                }
                Some(Action::Copy) => ui.ctx().copy_image((*shown.copy).clone()),
                Some(Action::Open) => super::open_web_link(ui.ctx(), url),
                None => {}
            }
            out.resp
        }
        (Entry::Video { .. }, Target::Video(video)) => {
            let video = video.clone();
            video_card(ui, p, pv, k, url, &video, width + VIDEO_EXTRA);
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
    card_menu(&resp, p, pv, k, url);
}

/// What the resident did on a picture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    Copy,
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
fn picture(ui: &mut egui::Ui, p: &Palette, shown: &Shown, width: f32, blur: bool) -> PictureOut {
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
    if hovered && rect.width() >= BUTTON * 3.0 + 24.0 {
        let mut x = rect.right() - 8.0 - BUTTON;
        let y = rect.top() + 8.0;
        for (icon, tip, act) in [
            ("eye-off", aurora_i18n::tr!("ui-chat-preview-hide"), Action::Hide),
            ("copy-filled", aurora_i18n::tr!("ui-chat-preview-copy"), Action::Copy),
            ("external-link-filled", aurora_i18n::tr!("ui-chat-preview-open"), Action::Open),
        ] {
            let b = egui::Rect::from_min_size(egui::pos2(x, y), Vec2::splat(BUTTON));
            if overlay_button(ui, p, b, icon, tip) {
                action = Some(act);
            }
            x -= BUTTON + 4.0;
        }
    }
    if action.is_none() && resp.clicked() {
        action = Some(Action::Open);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    PictureOut { resp, action }
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
        Target::Picture => (
            "photo-filled",
            aurora_i18n::tr!("ui-chat-preview-picture-from", site = site.as_str()),
        ),
        Target::Video(_) => (
            "brand-youtube-filled",
            aurora_i18n::tr!("ui-chat-preview-video-from", site = site.as_str()),
        ),
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

/// A YouTube video: its picture with the play button, its title and
/// channel; playing, the player in place of the picture with its buttons
/// under it.
fn video_card(ui: &mut egui::Ui, p: &Palette, pv: &mut Previews, k: u64, url: &str, video: &lp::Video, width: f32) {
    let pass = ui.ctx().cumulative_pass_nr();
    let Some(Cached {
        entry: Entry::Video { title, channel, thumbnail },
        ..
    }) = pv.cache.get(url)
    else {
        return;
    };
    let title = if title.is_empty() {
        aurora_i18n::tr!("ui-chat-preview-video").to_owned()
    } else {
        title.clone()
    };
    let channel = if channel.is_empty() {
        "YouTube".to_owned()
    } else {
        format!("{channel} · YouTube")
    };
    let here = pv.playing.as_ref().filter(|v| v.key == k).map(|v| v.window);
    let width = if here == Some(false) {
        (width * PLAYING_SCALE).min(ui.available_width().max(width))
    } else {
        width
    };
    let mut play = false;
    let (mut detach, mut stop) = (false, false);
    let resp = egui::Frame::new()
        .fill(p.raised)
        .corner_radius(ui.visuals().window_corner_radius)
        .show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = Vec2::new(width, (width * 9.0 / 16.0).round());
            match (here, pv.playing.as_mut(), pv.web.as_ref()) {
                (Some(false), Some(v), Some(web)) => {
                    v.drawn = pass;
                    web.show(&mut v.web, ui, p, size);
                }
                _ => {
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    let radius = ui.visuals().window_corner_radius;
                    match thumbnail {
                        Some(t) => {
                            let corners = egui::CornerRadius {
                                nw: radius.nw,
                                ne: radius.ne,
                                sw: 0,
                                se: 0,
                            };
                            egui::Image::new(&t.frames[0].0).corner_radius(corners).paint_at(ui, rect);
                        }
                        None => {
                            ui.painter().rect_filled(rect, radius, p.field);
                            paint_icon(
                                ui.painter(),
                                "brand-youtube-filled",
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
                        let buttons = if here == Some(false) { 2.0 * (20.0 + 6.0) } else { 0.0 };
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - buttons).max(60.0));
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(egui::Label::new(RichText::new(&title).size(13.5).strong().color(p.ink)).truncate());
                            ui.add(egui::Label::new(RichText::new(&channel).size(11.5).color(p.muted)).truncate());
                        });
                        if here == Some(false) {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                            });
                        }
                    });
                });
        })
        .response;
    let demo = pv.playing.is_none().then(|| pv.demo_play.take()).flatten();
    if play || demo.is_some() {
        pv.playing = Some(Playing {
            key: k,
            link: url.to_owned(),
            title: title.clone(),
            web: {
                let mut web = WebView::default();
                web.navigate(&video.player_page());
                web
            },
            window: demo.unwrap_or(pv.settings.player) == lp::Player::Window,
            drawn: pass,
        });
    }
    if detach && let Some(v) = pv.playing.as_mut() {
        v.window = true;
    }
    if stop {
        pv.playing = None;
    }
    card_menu(&resp, p, pv, k, url);
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
