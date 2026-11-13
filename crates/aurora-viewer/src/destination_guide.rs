//! The Destination Guide's places, read for Aurora's own Destinations
//! window (`ui/destinations/aurora.rs`). The data is the public feed the
//! official guide page reads (`guide.js` of the v3.0 viewer web
//! components, the page LLFloaterDestinations shows): the categories of the
//! `viewer_dg` channel, then the places of each category sorted by
//! population, with "What's Hot Now" first (`busiest`: 20 to 1000 people,
//! General and Moderate, 32 at most), as guide.js asks them. The guide page
//! of both Second Life grids reads the main grid's feed; so does this.
//!
//! Everything read is untrusted: the sizes are bounded, the texts cut, a
//! category's name checked before it goes into an address, a place kept
//! only with a location the teleport links read. Pictures come over https
//! from public hosts only, at most `MAX_IMAGE_BYTES`, decoded off the frame
//! within `DECODE_LIMIT` pixels a side and kept at the size they are drawn.
//! No cookie, session or account goes with a request, and the User-Agent
//! is the viewer's. While the window is closed nothing is asked and the
//! pictures are let go.

use crate::demo::destinations as demo;
use aurora_net::search::AgentAccess;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The feed's address (guide.js `world_api`).
pub const API: &str = "https://worldaping.agni.lindenlab.com/v2/destinations/";
/// "What's Hot Now", the feed guide.js adds before the server's categories.
pub const HOT: &str = "busiest";
/// The window asks again this often while it is open.
pub const REFRESH: Duration = Duration::from_secs(300);
/// The refresh button is not a way to hammer the feed.
const MIN_MANUAL: Duration = Duration::from_secs(10);

const MAX_LIST_BYTES: usize = 4 * 1024 * 1024;
const MAX_CATEGORIES: usize = 40;
const MAX_PLACES: usize = 250;
const MAX_NAME: usize = 120;
const MAX_DESCRIPTION: usize = 1500;
const MAX_KEY: usize = 64;
const MAX_LOCATION: usize = 300;
const MAX_URL: usize = 600;
const MAX_IMAGE_BYTES: u64 = 3 * 1024 * 1024;
/// Largest picture decoded, in pixels per side.
const DECODE_LIMIT: u32 = 4096;
/// The longest side kept of a card's picture (the feed's `midsize` is 250
/// x 150: kept whole) and of the large photo of the wide window's preview.
pub const CARD_SIDE: u32 = 320;
pub const PHOTO_SIDE: u32 = 700;
const MAX_IN_FLIGHT: usize = 6;
/// Pictures kept as textures; the least recently drawn go first.
const MAX_TEXTURES: usize = 72;

/// A category's place list as last asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Waiting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    /// The feed's `simple_name` (`HOT` for What's Hot Now).
    pub key: String,
    /// As the server names it (in English); empty for `HOT`, which the
    /// window names itself.
    pub name: String,
    pub places: Vec<Place>,
    pub feed: Feed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub region: String,
    pub pos: glam::Vec3,
    /// SIM_ACCESS_PG, _MATURE or _ADULT.
    pub rating: u8,
    pub population: u32,
    /// `publish_date` as given ("YYYY-MM-DD HH:MM:SS", which sorts as
    /// text), empty when unreadable.
    pub published: String,
    /// The listing's last day (year, month, day) when it ends at all: the
    /// feed writes 2099 for "never".
    pub until: Option<(i32, u32, u32)>,
    /// The `midsize` picture, and the `fullsize` one (else `midsize`).
    pub card: Option<String>,
    pub photo: Option<String>,
}

impl Place {
    /// The link the guide's page gives this place: the same handling.
    pub fn teleport_link(&self) -> String {
        format!(
            "secondlife:///app/teleport/{}/{}/{}/{}",
            crate::slurl::encode(&self.region),
            self.pos.x.round(),
            self.pos.y.round(),
            self.pos.z.round()
        )
    }
}

/// Where the window stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    Idle,
    Loading,
    Ready,
    Failed,
}

/// Where the feed comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Live,
    /// The offline fixture of `AURORA_DEMO_DESTINATIONS`.
    Demo(demo::Fixture),
}

/// The maturity letters asked: never above what the agent may enter
/// (LLAgentAccess::canAccessMature / canAccessAdult, which include the
/// content preference).
pub fn access_letters(access: AgentAccess) -> &'static str {
    if access.can_access_adult() {
        "gma"
    } else if access.can_access_mature() {
        "gm"
    } else {
        "g"
    }
}

pub fn categories_url() -> String {
    format!("{API}categories/bychan/viewer_dg/?format=json")
}

/// guide.js's `fetchCategoryData`: sorted by population, the maturity
/// asked within what the agent may enter (What's Hot Now never above
/// Moderate), What's Hot Now with its own limits.
pub fn places_url(key: &str, letters: &str) -> String {
    let base = format!("{API}{key}/?format=json&sort_by=population&sort_dir=desc&flatten");
    if key == HOT {
        let letters = if letters == "g" { "g" } else { "gm" };
        format!("{base}&maturity={letters}&pagesize=32&maxpop=1000&minpop=20")
    } else {
        format!("{base}&maturity={letters}")
    }
}

/// A category name that may go into an address.
fn key_ok(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_KEY && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A text of the feed: control characters out (line breaks kept when
/// `lines`), trimmed, at most `max` characters.
fn clean(s: &str, max: usize, lines: bool) -> String {
    let s = s.replace("\r\n", "\n");
    let mut out: String = s
        .chars()
        .filter(|c| !c.is_control() || (lines && *c == '\n'))
        .map(|c| if c == '\t' { ' ' } else { c })
        .collect();
    out = out.trim().to_owned();
    if out.chars().count() > max {
        out = out.chars().take(max.saturating_sub(1)).collect::<String>().trim_end().to_owned();
        out.push('…');
    }
    out
}

fn text(v: &serde_json::Value, max: usize, lines: bool) -> String {
    v.as_str().map(|s| clean(s, max, lines)).unwrap_or_default()
}

/// The server's categories of the guide, in its order: (key, name).
pub fn parse_categories(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    if bytes.len() > MAX_LIST_BYTES {
        return Err("categories too large".into());
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let list = v.as_array().ok_or("categories: not a list")?;
    let mut out: Vec<(String, String)> = Vec::new();
    for c in list.iter().take(MAX_CATEGORIES) {
        let key = c["simple_name"].as_str().unwrap_or("");
        let name = text(&c["name"], MAX_NAME, false);
        if key_ok(key) && key != HOT && !name.is_empty() && !out.iter().any(|(k, _)| k == key) {
            out.push((key.to_owned(), name));
        }
    }
    Ok(out)
}

/// A place's `location` (url-encoded "Region/x/y/z"), read as the guide's
/// teleport link `secondlife:///app/teleport/<location>` is.
fn location(raw: &str) -> Option<(String, glam::Vec3)> {
    if raw.is_empty() || raw.len() > MAX_LOCATION || raw.chars().any(|c| c.is_whitespace() || c.is_control() || "?#".contains(c)) {
        return None;
    }
    let link = crate::slurl::place_link(&format!("secondlife:///app/teleport/{raw}"))?;
    let l = link.location;
    let region = clean(&l.region, MAX_NAME, false);
    (link.teleport && l.grid.is_none() && !region.is_empty() && l.pos.is_finite()).then_some((region, l.pos))
}

/// A picture's address: https, a host on the internet, nothing more.
fn image_url(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_URL {
        return None;
    }
    let url = reqwest::Url::parse(s).ok()?;
    let ok = url.scheme() == "https"
        && url.host_str().is_some_and(|h| !h.is_empty())
        && url.username().is_empty()
        && url.password().is_none()
        && !crate::link_trust::leads_to_local_network(s);
    ok.then(|| url.to_string())
}

/// "YYYY-MM-DD…" as (year, month, day).
fn day_of(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.get(..10)?.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
    let d = it.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
    Some((y, m, d))
}

/// One category's places: those with a name and a location the teleport
/// links read, the first of each slug.
pub fn parse_places(bytes: &[u8]) -> Result<Vec<Place>, String> {
    if bytes.len() > MAX_LIST_BYTES {
        return Err("places too large".into());
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let list = v.as_array().ok_or("places: not a list")?;
    let mut out: Vec<Place> = Vec::new();
    for e in list.iter().take(MAX_PLACES * 2) {
        if out.len() >= MAX_PLACES {
            break;
        }
        let name = text(&e["name"], MAX_NAME, false);
        let Some((region, pos)) = e["location"].as_str().and_then(location) else {
            continue;
        };
        let slug = match text(&e["slug"], MAX_NAME, false) {
            s if s.is_empty() => format!("{region}/{}/{}/{}", pos.x, pos.y, pos.z),
            s => s,
        };
        if name.is_empty() || out.iter().any(|p| p.slug == slug) {
            continue;
        }
        let rating = match e["maturity"].as_str().unwrap_or("").trim() {
            "A" | "a" => aurora_net::search::SIM_ACCESS_ADULT,
            "M" | "m" => aurora_net::search::SIM_ACCESS_MATURE,
            _ => aurora_net::search::SIM_ACCESS_PG,
        };
        let population = e["population"]["current"]
            .as_u64()
            .or_else(|| e["population"]["current"].as_str().and_then(|s| s.trim().parse().ok()))
            .unwrap_or(0)
            .min(100_000) as u32;
        let asset = |kind: &str| {
            e["assets"].as_array().and_then(|a| {
                a.iter()
                    .take(16)
                    .find(|x| x["type"].as_str() == Some(kind))
                    .and_then(|x| x["data"].as_str())
                    .and_then(image_url)
            })
        };
        let card = asset("midsize");
        let photo = asset("fullsize").or_else(|| card.clone());
        let published = e["publish_date"].as_str().filter(|s| day_of(s).is_some()).unwrap_or("");
        out.push(Place {
            slug,
            name,
            description: text(&e["description"], MAX_DESCRIPTION, true),
            region,
            pos,
            rating,
            population,
            published: published.chars().take(19).collect(),
            until: e["end_date"].as_str().and_then(day_of).filter(|(y, ..)| *y < 2090),
            card,
            photo,
        });
    }
    Ok(out)
}

/// Decode a picture of the feed (PNG, JPEG, WebP), kept within `side`.
pub fn decode(bytes: &[u8], side: u32) -> Option<egui::ColorImage> {
    let format = image::guess_format(bytes).ok()?;
    if !matches!(
        format,
        image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
    ) {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(DECODE_LIMIT);
    limits.max_image_height = Some(DECODE_LIMIT);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| log::debug!("destination picture: {e}")).ok()?;
    Some(fit(&img, side))
}

pub(crate) fn fit(img: &image::DynamicImage, side: u32) -> egui::ColorImage {
    let (w, h) = (img.width(), img.height());
    let img = if w.max(h) > side {
        let k = side as f32 / w.max(h) as f32;
        let (nw, nh) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
        std::borrow::Cow::Owned(img.resize_exact(nw, nh, image::imageops::FilterType::Triangle))
    } else {
        std::borrow::Cow::Borrowed(img)
    };
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

enum Msg {
    Categories(u64, Result<Vec<(String, String)>, String>),
    Places(u64, String, Result<Vec<Place>, String>),
    Image(String, Option<egui::ColorImage>),
}

enum Slot {
    Loading,
    Ready(egui::TextureHandle, u64),
    Failed,
}

/// The pictures drawn while the window is open, by address.
#[derive(Default)]
pub struct Pictures {
    map: HashMap<String, Slot>,
    /// Asked this frame and not loading yet: (address, longest side kept).
    wanted: Vec<(String, u32)>,
    in_flight: usize,
    frame: u64,
}

impl Pictures {
    /// The texture of a picture drawn this frame, None until it came; the
    /// first ask starts its download.
    pub fn texture(&mut self, url: &str, side: u32) -> Option<egui::TextureHandle> {
        let frame = self.frame;
        match self.map.get_mut(url) {
            Some(Slot::Ready(t, used)) => {
                *used = frame;
                Some(t.clone())
            }
            Some(_) => None,
            None => {
                if !self.wanted.iter().any(|(u, _)| u == url) {
                    self.wanted.push((url.to_owned(), side));
                }
                None
            }
        }
    }

    /// Pixels kept as textures, for the measures.
    pub fn bytes(&self) -> usize {
        self.map
            .values()
            .map(|s| match s {
                Slot::Ready(t, _) => t.size()[0] * t.size()[1] * 4,
                _ => 0,
            })
            .sum()
    }
}

/// The guide's data while the window is open, and its pictures.
pub struct DestinationGuide {
    pub categories: Vec<Category>,
    pub load: Load,
    /// Bumps at every change of `categories`: the window's lists are built
    /// again only then.
    pub generation: u64,
    /// When the last full answer came.
    pub updated: Option<Instant>,
    pub pictures: Pictures,
    source: Option<Source>,
    letters: &'static str,
    asked: u64,
    pending: usize,
    next: Option<Instant>,
    started: Option<Instant>,
    http: Option<reqwest::Client>,
    tx: crossbeam_channel::Sender<Msg>,
    rx: crossbeam_channel::Receiver<Msg>,
}

impl Default for DestinationGuide {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            categories: Vec::new(),
            load: Load::Idle,
            generation: 0,
            updated: None,
            pictures: Pictures::default(),
            source: None,
            letters: "g",
            asked: 0,
            pending: 0,
            next: None,
            started: None,
            http: None,
            tx,
            rx,
        }
    }
}

impl DestinationGuide {
    /// Whether an answer is awaited.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// The refresh button: at once, unless an answer is awaited or the last
    /// one was asked moments ago.
    pub fn refresh(&mut self, now: Instant) {
        if self.pending == 0 && self.started.is_none_or(|at| now.duration_since(at) >= MIN_MANUAL) {
            self.next = None;
        }
    }

    /// Each frame. `wanted`: the window is open (nothing else happens while
    /// it is closed, and its pictures are let go); `source` None on a grid
    /// without the feed.
    pub fn poll(
        &mut self,
        ctx: &egui::Context,
        rt: Option<&tokio::runtime::Handle>,
        source: Option<Source>,
        access: AgentAccess,
        wanted: bool,
        now: Instant,
    ) {
        if !wanted {
            if !self.pictures.map.is_empty() || !self.pictures.wanted.is_empty() {
                log::info!(
                    "destination guide: window closed, {} pictures let go ({} KiB of textures)",
                    self.pictures.map.len(),
                    self.pictures.bytes() / 1024
                );
                self.pictures = Pictures::default();
            }
            return;
        }
        self.pictures.frame += 1;
        let letters = access_letters(access);
        if self.source != source || self.letters != letters {
            // another grid, or another content rating: what was read says
            // nothing of what to show now
            let keep = (self.tx.clone(), self.rx.clone(), self.http.take(), self.asked + 1);
            *self = Self {
                source,
                letters,
                tx: keep.0,
                rx: keep.1,
                http: keep.2,
                asked: keep.3,
                ..Self::default()
            };
        }
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Image(url, image) => self.image_came(ctx, url, image),
                msg => changed |= self.apply(msg, rt, now),
            }
        }
        if let Some(source) = self.source
            && self.pending == 0
            && self.next.is_none_or(|at| now >= at)
        {
            self.start(source, rt, now);
            changed = true;
        }
        self.load_images(rt);
        if changed {
            self.generation += 1;
            ctx.request_repaint();
        }
    }

    fn start(&mut self, source: Source, rt: Option<&tokio::runtime::Handle>, now: Instant) {
        self.asked += 1;
        self.pending = 1;
        self.next = Some(now + REFRESH);
        self.started = Some(now);
        if self.categories.is_empty() {
            self.load = Load::Loading;
        }
        let (tx, asked) = (self.tx.clone(), self.asked);
        match source {
            Source::Demo(fixture) => {
                let _ = tx.send(Msg::Categories(asked, parse_categories(demo::categories_json(fixture).as_bytes())));
            }
            Source::Live => {
                let (Some(rt), Some(http)) = (rt, self.client()) else {
                    let _ = tx.send(Msg::Categories(asked, Err("no network".into())));
                    return;
                };
                rt.spawn(async move {
                    let result = get(&http, &categories_url(), MAX_LIST_BYTES as u64, "application/json")
                        .await
                        .and_then(|b| parse_categories(&b));
                    let _ = tx.send(Msg::Categories(asked, result));
                });
            }
        }
    }

    fn client(&mut self) -> Option<reqwest::Client> {
        if self.http.is_none() {
            self.http = client();
        }
        self.http.clone()
    }

    /// An answer; true when the data changed.
    fn apply(&mut self, msg: Msg, rt: Option<&tokio::runtime::Handle>, now: Instant) -> bool {
        match msg {
            Msg::Categories(asked, result) if asked == self.asked => {
                let list = match result {
                    Ok(list) => list,
                    Err(e) => {
                        log::warn!("destination guide: categories not read: {e}");
                        self.pending = 0;
                        if self.categories.is_empty() {
                            self.load = Load::Failed;
                        }
                        return true;
                    }
                };
                // the places shown stay until their category's new answer
                let mut old = std::mem::take(&mut self.categories);
                let mut take = |key: &str| {
                    old.iter()
                        .position(|c| c.key == key)
                        .map(|i| old.swap_remove(i).places)
                        .unwrap_or_default()
                };
                let mut next = vec![Category {
                    key: HOT.into(),
                    name: String::new(),
                    places: take(HOT),
                    feed: Feed::Waiting,
                }];
                for (key, name) in list {
                    let places = take(&key);
                    next.push(Category {
                        key,
                        name,
                        places,
                        feed: Feed::Waiting,
                    });
                }
                self.categories = next;
                self.pending = self.categories.len();
                let keys: Vec<String> = self.categories.iter().map(|c| c.key.clone()).collect();
                for key in keys {
                    self.ask_places(key, rt);
                }
                true
            }
            Msg::Places(asked, key, result) if asked == self.asked => {
                self.pending = self.pending.saturating_sub(1);
                if let Some(c) = self.categories.iter_mut().find(|c| c.key == key) {
                    match result {
                        Ok(places) => {
                            c.places = places;
                            c.feed = Feed::Ready;
                        }
                        Err(e) => {
                            log::warn!("destination guide: category {key} not read: {e}");
                            c.feed = Feed::Failed;
                        }
                    }
                }
                if self.pending == 0 {
                    let any = self.categories.iter().any(|c| c.feed == Feed::Ready);
                    self.load = if any || (!self.categories.is_empty() && self.updated.is_some()) {
                        Load::Ready
                    } else {
                        Load::Failed
                    };
                    if any {
                        self.updated = Some(now);
                    }
                }
                true
            }
            // an answer to an older request, or a picture (`poll`)
            _ => false,
        }
    }

    fn ask_places(&mut self, key: String, rt: Option<&tokio::runtime::Handle>) {
        let (tx, asked) = (self.tx.clone(), self.asked);
        match self.source {
            Some(Source::Demo(fixture)) => {
                let result = parse_places(demo::places_json(fixture, &key).as_bytes());
                let _ = tx.send(Msg::Places(asked, key, result));
            }
            _ => {
                let (Some(rt), Some(http)) = (rt, self.client()) else {
                    let _ = tx.send(Msg::Places(asked, key, Err("no network".into())));
                    return;
                };
                let url = places_url(&key, self.letters);
                rt.spawn(async move {
                    let result = get(&http, &url, MAX_LIST_BYTES as u64, "application/json")
                        .await
                        .and_then(|b| parse_places(&b));
                    let _ = tx.send(Msg::Places(asked, key, result));
                });
            }
        }
    }

    fn image_came(&mut self, ctx: &egui::Context, url: String, image: Option<egui::ColorImage>) {
        let pics = &mut self.pictures;
        pics.in_flight = pics.in_flight.saturating_sub(1);
        // a picture of a window closed since, or let go: dropped
        let Some(slot) = pics.map.get_mut(&url) else { return };
        *slot = match image {
            Some(img) => {
                let name = format!("destination-picture-{}", pics.frame);
                Slot::Ready(ctx.load_texture(name, img, egui::TextureOptions::LINEAR), pics.frame)
            }
            None => Slot::Failed,
        };
        ctx.request_repaint();
    }

    /// The pictures asked this frame start loading, a few at a time; the
    /// least recently drawn go once there are too many.
    fn load_images(&mut self, rt: Option<&tokio::runtime::Handle>) {
        let wanted = std::mem::take(&mut self.pictures.wanted);
        for (url, side) in wanted {
            if self.pictures.in_flight >= MAX_IN_FLIGHT {
                break;
            }
            if self.pictures.map.contains_key(&url) {
                continue;
            }
            let Some(rt) = rt else { break };
            let tx = self.tx.clone();
            match self.source {
                Some(Source::Demo(_)) => {
                    let key = url.clone();
                    rt.spawn_blocking(move || {
                        let _ = tx.send(Msg::Image(key.clone(), demo::picture(&key, side)));
                    });
                }
                Some(Source::Live) => {
                    let Some(http) = self.client() else { break };
                    let key = url.clone();
                    rt.spawn(async move {
                        let image = match get(&http, &key, MAX_IMAGE_BYTES, "image/webp,image/png,image/jpeg").await {
                            Ok(bytes) => tokio::task::spawn_blocking(move || decode(&bytes, side)).await.ok().flatten(),
                            Err(e) => {
                                log::debug!("destination picture {key}: {e}");
                                None
                            }
                        };
                        let _ = tx.send(Msg::Image(key, image));
                    });
                }
                None => break,
            }
            self.pictures.in_flight += 1;
            self.pictures.map.insert(url, Slot::Loading);
        }
        let ready = self.pictures.map.values().filter(|s| matches!(s, Slot::Ready(..))).count();
        if ready > MAX_TEXTURES {
            let mut used: Vec<(u64, String)> = self
                .pictures
                .map
                .iter()
                .filter_map(|(u, s)| match s {
                    Slot::Ready(_, at) => Some((*at, u.clone())),
                    _ => None,
                })
                .collect();
            used.sort_unstable();
            for (_, url) in used.into_iter().take(ready - MAX_TEXTURES) {
                self.pictures.map.remove(&url);
            }
        }
    }
}

/// The feed's client: the viewer's User-Agent, https only, no cookie
/// store, names resolved to public addresses only, and a redirect never to
/// this computer or the local network.
fn client() -> Option<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 3 || crate::link_trust::leads_to_local_network(attempt.url().as_str()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .user_agent(aurora_version::user_agent())
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .retry(reqwest::retry::never())
        .redirect(policy)
        .dns_resolver(crate::link_preview::PublicOnly)
        .build()
        .map_err(|e| log::warn!("destination guide: no HTTP client: {e}"))
        .ok()
}

/// The body of `url`, at most `max` bytes.
async fn get(http: &reqwest::Client, url: &str, max: u64, accept: &str) -> Result<Vec<u8>, String> {
    let mut resp = http
        .get(url)
        .header(reqwest::header::ACCEPT, accept)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(resp.status().to_string());
    }
    if resp.content_length().is_some_and(|n| n > max) {
        return Err("too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > max {
            return Err("too large".into());
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests;
