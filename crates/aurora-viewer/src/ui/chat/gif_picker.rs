//! The GIF picker of the Conversations field (#932), the emoji picker's
//! sibling: a tab per service (GIPHY, KLIPY: their terms keep their
//! results apart), a search that waits for the typing to pause, the
//! service's categories, the trending GIFs while nothing is typed, and the
//! favourites. A click sends the GIF's link at once, as a message of the
//! conversation; Aurora's link previews play it (`previews.rs`).
//!
//! The tiles load only while they show, as small animations within a
//! memory budget, and everything is freed when the picker closes.
//! Requests and media go straight to the services (`gif_search.rs`).

use crate::gif_search::{self as gs, Category, Error, Gif, Page, Provider, Query, Rating};
use crate::theme::Palette;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::time::Duration;

/// The typing pauses this long before a search leaves: a test key allows
/// about 100 requests an hour.
const DEBOUNCE: f64 = 0.45;
pub(super) const WIDTH: f32 = 340.0;
const COLS: usize = 3;
const TILE_H: f32 = 84.0;
const GAP: f32 = 4.0;
/// The grid's height, the same whatever it shows: the popup never moves.
const GRID_H: f32 = 3.0 * (TILE_H + GAP) + 2.0;
/// Tiles downloading at once.
const MAX_THUMBS_IN_FLIGHT: usize = 6;
/// Tiles kept with their frames; those out of view go first.
const MAX_THUMBS: usize = 36;

/// What the picker asks a service for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Request {
    provider: Provider,
    query: Query,
    rating: Rating,
    key: String,
}

enum Answer {
    Page {
        generation: u64,
        page: u32,
        result: Result<Page, Error>,
    },
    Thumb {
        generation: u64,
        url: String,
        result: Result<crate::link_preview::Picture, Error>,
    },
    Categories {
        provider: Provider,
        list: Vec<Category>,
    },
}

/// A tile's animation, as textures.
struct Frames {
    frames: Vec<(egui::TextureHandle, Duration)>,
    period: Duration,
}

impl Frames {
    fn new(ctx: &egui::Context, name: &str, picture: crate::link_preview::Picture) -> Frames {
        let period = picture.frames.iter().map(|f| f.1).sum();
        let frames = picture
            .frames
            .into_iter()
            .enumerate()
            .map(|(i, (img, d))| {
                (
                    ctx.load_texture(format!("gif-tile:{name}#{i}"), img, egui::TextureOptions::LINEAR),
                    d,
                )
            })
            .collect();
        Frames { frames, period }
    }

    /// The frame at `time`, and how long until the next.
    fn at(&self, time: f64) -> Option<(&egui::TextureHandle, Option<Duration>)> {
        let first = self.frames.first()?;
        if self.frames.len() < 2 || self.period.is_zero() {
            return Some((&first.0, None));
        }
        let mut at = Duration::from_secs_f64(time.max(0.0)).as_millis() % self.period.as_millis().max(1);
        for (t, d) in &self.frames {
            let ms = d.as_millis();
            if at < ms {
                return Some((t, Some(Duration::from_millis((ms - at) as u64))));
            }
            at -= ms;
        }
        Some((&first.0, Some(first.1)))
    }
}

enum Thumb {
    Queued,
    Loading,
    Ready(Frames),
    Failed,
}

struct Tile {
    thumb: Thumb,
    /// The pass that last drew it.
    used: u64,
}

/// What the picker did this pass.
#[derive(Default)]
pub(super) struct Out {
    /// A GIF clicked: its link is sent.
    pub send: Option<Gif>,
    pub escape: bool,
}

pub struct GifPicker {
    /// Preferences › Chat, given every frame by the app.
    pub settings: gs::Settings,
    /// The favourites or the random id changed: the app saves them.
    pub changed: bool,
    /// "Preferences" was clicked (no key): the app opens the GIF settings.
    pub open_settings: bool,
    /// Demo mode: answers and tiles made here.
    pub demo: bool,
    /// The tab shown (None: the settings' last one, on opening).
    tab: Option<Provider>,
    favorites: bool,
    pub query: String,
    typed_at: Option<f64>,
    /// What the shown results answer.
    asked: Option<Request>,
    gifs: Vec<Gif>,
    next: Option<u32>,
    loading: bool,
    loading_more: bool,
    error: Option<Error>,
    /// Answers of an older request are dropped.
    generation: u64,
    tiles: HashMap<String, Tile>,
    queue: VecDeque<(String, Provider)>,
    in_flight: usize,
    /// GIFs whose "shown" was told to GIPHY.
    seen: HashSet<String>,
    categories: HashMap<Provider, Vec<Category>>,
    categories_asked: HashSet<Provider>,
    /// The pass that last drew the picker.
    shown_pass: Option<u64>,
    /// The picker drew since it was last closed.
    active: bool,
    /// The tiles of the last pass and their GIFs' ids.
    drawn: Vec<(egui::Rect, String)>,
    /// `AURORA_DEMO_GIFS=sent`: the first GIF is sent once its tile shows.
    demo_send: bool,
    /// `AURORA_DEMO_GIFS=menu`: the button's right-click menu opens once.
    demo_menu: bool,
    tx: mpsc::Sender<Answer>,
    rx: mpsc::Receiver<Answer>,
    runtime: Option<tokio::runtime::Runtime>,
    client: Option<reqwest::Client>,
}

impl Default for GifPicker {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        GifPicker {
            settings: gs::Settings::default(),
            changed: false,
            open_settings: false,
            demo: false,
            tab: None,
            favorites: false,
            query: String::new(),
            typed_at: None,
            asked: None,
            gifs: Vec::new(),
            next: None,
            loading: false,
            loading_more: false,
            error: None,
            generation: 0,
            tiles: HashMap::new(),
            queue: VecDeque::new(),
            in_flight: 0,
            seen: HashSet::new(),
            categories: HashMap::new(),
            categories_asked: HashSet::new(),
            shown_pass: None,
            active: false,
            drawn: Vec::new(),
            demo_send: false,
            demo_menu: false,
            tx,
            rx,
            runtime: None,
            client: None,
        }
    }
}

impl GifPicker {
    /// `AURORA_DEMO_GIFS`: where the picker opens, what it does.
    pub fn demo_scene(&mut self, mode: crate::demo::Gifs) {
        match mode {
            crate::demo::Gifs::Klipy => {
                self.tab = Some(Provider::Klipy);
                self.query = "dance".into();
            }
            crate::demo::Gifs::Sent => self.demo_send = true,
            crate::demo::Gifs::Menu => self.demo_menu = true,
            _ => {}
        }
    }

    /// The GIF button shows: turned on and a service with a key (both
    /// services and no key in the demo's scenarios).
    pub fn usable(&self) -> bool {
        self.demo || self.settings.usable()
    }

    fn key(&self, provider: Provider) -> Option<String> {
        if self.demo {
            return Some("demo".to_owned());
        }
        self.settings.key(provider)
    }

    /// The services with a key: one tab each.
    fn providers(&self) -> Vec<Provider> {
        if self.demo {
            return Provider::ALL.to_vec();
        }
        self.settings.providers()
    }

    /// The tab shown: the one chosen, else the settings' last one, when
    /// they have a key; else the first with one.
    fn tab(&self) -> Provider {
        let providers = self.providers();
        let last = self.settings.provider;
        self.tab
            .filter(|t| providers.contains(t))
            .or_else(|| providers.contains(&last).then_some(last))
            .or_else(|| providers.first().copied())
            .unwrap_or(last)
    }

    /// The request the picker should show at `now`, when it is not the one
    /// shown: None while the typing has not paused, or nothing changed.
    pub(super) fn wanted(&self, now: f64) -> Option<Request> {
        let provider = self.tab();
        let key = self.key(provider)?;
        let query = if self.favorites {
            Query::Favorites(self.settings.favorite_ids(provider))
        } else {
            let text = self.query.trim();
            if text.is_empty() {
                Query::Trending
            } else if self.typed_at.is_some_and(|t| now - t < DEBOUNCE) {
                return None;
            } else {
                Query::Search(text.to_owned())
            }
        };
        let request = Request {
            provider,
            query,
            rating: self.settings.rating,
            key,
        };
        (self.asked.as_ref() != Some(&request)).then_some(request)
    }

    /// Show `request`'s answer from now on: the results and their tiles of
    /// before go.
    pub(super) fn begin(&mut self, request: Request) {
        self.generation += 1;
        self.gifs.clear();
        self.tiles.clear();
        self.queue.clear();
        self.next = None;
        self.error = None;
        self.loading_more = false;
        // no favourite: nothing to ask
        self.loading = !matches!(&request.query, Query::Favorites(ids) if ids.is_empty());
        self.asked = Some(request);
    }

    /// The next page, when the last row shows and there is one.
    pub(super) fn more(&mut self) -> Option<(Request, u32)> {
        if self.loading || self.loading_more || self.error.is_some() {
            return None;
        }
        let page = self.next?;
        self.loading_more = true;
        Some((self.asked.clone()?, page))
    }

    fn receive(&mut self, ctx: Option<&egui::Context>, answer: Answer) {
        match answer {
            Answer::Page { generation, page, result } if generation == self.generation => {
                let first = self.asked.as_ref().is_some_and(|r| page == gs::first_page(r.provider));
                self.loading = false;
                self.loading_more = false;
                match result {
                    Ok(answer) => {
                        if first {
                            self.gifs = answer.gifs;
                        } else {
                            // a GIF of the page before stays once
                            let known: HashSet<String> = self.gifs.iter().map(|g| g.id.clone()).collect();
                            self.gifs.extend(answer.gifs.into_iter().filter(|g| !known.contains(&g.id)));
                        }
                        self.next = answer.next;
                    }
                    Err(e) => {
                        self.next = None;
                        if first {
                            self.error = Some(e);
                        }
                    }
                }
            }
            Answer::Thumb { generation, url, result } if generation == self.generation => {
                self.in_flight = self.in_flight.saturating_sub(1);
                if let Some(tile) = self.tiles.get_mut(&url) {
                    tile.thumb = match (result, ctx) {
                        (Ok(picture), Some(ctx)) => Thumb::Ready(Frames::new(ctx, &url, picture)),
                        _ => Thumb::Failed,
                    };
                }
            }
            Answer::Thumb { .. } => self.in_flight = self.in_flight.saturating_sub(1),
            Answer::Categories { provider, list } => {
                self.categories.insert(provider, list);
            }
            Answer::Page { .. } => {}
        }
    }

    /// Everything goes: results, tiles, the search. The categories (a few
    /// words) stay for the session. Nothing happens when already closed.
    pub(super) fn close(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        self.generation += 1;
        self.asked = None;
        self.gifs = Vec::new();
        self.tiles = HashMap::new();
        self.queue.clear();
        self.next = None;
        self.loading = false;
        self.loading_more = false;
        self.error = None;
        self.query.clear();
        self.typed_at = None;
        self.favorites = false;
        self.tab = None;
        self.seen.clear();
        self.shown_pass = None;
    }

    /// Its window closed or hid it (the Conversations window closed, a
    /// window minimised): the tiles go, as on closing. Shown again, it
    /// starts afresh.
    pub(super) fn close_if_hidden(&mut self, pass: u64) {
        if self.shown_pass.is_some_and(|last| last + 2 < pass) {
            self.close();
        }
    }

    fn runtime(&mut self) -> Option<(&tokio::runtime::Runtime, reqwest::Client)> {
        if self.runtime.is_none() {
            match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("gif-search")
                .enable_all()
                .build()
            {
                Ok(rt) => self.runtime = Some(rt),
                Err(e) => log::warn!("GIF search: no runtime: {e}"),
            }
        }
        if self.client.is_none() {
            self.client = gs::client();
        }
        Some((self.runtime.as_ref()?, self.client.clone()?))
    }

    fn customer(&mut self) -> String {
        if self.settings.ensure_customer_id() {
            self.changed = true;
        }
        self.settings.customer_id.clone()
    }

    /// Ask `request`'s `page`.
    fn fetch(&mut self, ctx: &egui::Context, request: &Request, page: u32) {
        let generation = self.generation;
        if let Query::Favorites(ids) = &request.query
            && ids.is_empty()
        {
            return;
        }
        if self.demo {
            let result = gs::demo::page(request.provider, &request.query, page);
            let _ = self.tx.send(Answer::Page { generation, page, result });
            return;
        }
        let customer = self.customer();
        let url = gs::page_url(
            request.provider,
            &request.key,
            &request.query,
            page,
            request.rating,
            aurora_i18n::language(),
            &customer,
        );
        let provider = request.provider;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        let Some((rt, client)) = self.runtime() else {
            let _ = self.tx.send(Answer::Page {
                generation,
                page,
                result: Err(Error::Unavailable),
            });
            return;
        };
        rt.spawn(async move {
            let result = gs::fetch_page(&client, provider, &url).await;
            let _ = tx.send(Answer::Page { generation, page, result });
            wake.request_repaint();
        });
    }

    fn fetch_categories(&mut self, ctx: &egui::Context, provider: Provider) {
        if !self.categories_asked.insert(provider) {
            return;
        }
        if self.demo {
            self.categories.insert(provider, gs::demo::categories());
            return;
        }
        let Some(key) = self.key(provider) else {
            self.categories_asked.remove(&provider);
            return;
        };
        let url = gs::categories_url(provider, &key);
        let tx = self.tx.clone();
        let wake = ctx.clone();
        if let Some((rt, client)) = self.runtime() {
            rt.spawn(async move {
                let list = gs::fetch_categories(&client, provider, &url).await;
                let _ = tx.send(Answer::Categories { provider, list });
                wake.request_repaint();
            });
        }
    }

    /// A tile wants its animation.
    fn want_thumb(&mut self, url: &str, provider: Provider, pass: u64) {
        if let Some(tile) = self.tiles.get_mut(url) {
            tile.used = pass;
            return;
        }
        self.tiles.insert(
            url.to_owned(),
            Tile {
                thumb: Thumb::Queued,
                used: pass,
            },
        );
        self.queue.push_back((url.to_owned(), provider));
    }

    /// Answers in, tile downloads out, the tiles out of view freed.
    fn pump(&mut self, ctx: &egui::Context, pass: u64) {
        while let Ok(answer) = self.rx.try_recv() {
            self.receive(Some(ctx), answer);
        }
        while self.in_flight < MAX_THUMBS_IN_FLIGHT
            && let Some((url, provider)) = self.queue.pop_front()
        {
            // a tile scrolled away before its turn waits to show again
            if self.tiles.get(&url).is_none_or(|t| t.used + 1 < pass) {
                self.tiles.remove(&url);
                continue;
            }
            if let Some(tile) = self.tiles.get_mut(&url) {
                tile.thumb = Thumb::Loading;
            }
            self.in_flight += 1;
            let generation = self.generation;
            if self.demo {
                let _ = self.tx.send(Answer::Thumb {
                    generation,
                    result: Ok(gs::demo::thumb(&url)),
                    url,
                });
                continue;
            }
            let tx = self.tx.clone();
            let wake = ctx.clone();
            let Some((rt, client)) = self.runtime() else {
                self.in_flight -= 1;
                continue;
            };
            rt.spawn(async move {
                let result = gs::fetch_thumb(&client, provider, &url).await;
                let _ = tx.send(Answer::Thumb { generation, url, result });
                wake.request_repaint();
            });
        }
        let ready = self.tiles.values().filter(|t| matches!(t.thumb, Thumb::Ready(_))).count();
        if ready > MAX_THUMBS {
            let mut idle: Vec<(u64, String)> = self
                .tiles
                .iter()
                .filter(|(_, t)| matches!(t.thumb, Thumb::Ready(_)) && t.used + 1 < pass)
                .map(|(u, t)| (t.used, u.clone()))
                .collect();
            idle.sort();
            for (_, url) in idle.into_iter().take(ready - MAX_THUMBS) {
                self.tiles.remove(&url);
            }
        }
    }

    /// Tell GIPHY a GIF showed, was clicked or was sent (its terms ask
    /// for it); tell KLIPY a GIF was sent.
    fn ping(&mut self, url: Option<String>) {
        if self.demo {
            return;
        }
        let customer = self.customer();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let Some(url) = url.and_then(|u| gs::ping_url(&u, &customer, now)) else {
            return;
        };
        if let Some((rt, client)) = self.runtime() {
            rt.spawn(async move { gs::ping(&client, url).await });
        }
    }

    /// A GIF was sent: its service counts it.
    pub(super) fn sent(&mut self, gif: &Gif) {
        if self.demo {
            return;
        }
        match gif.provider {
            Provider::Giphy => self.ping(gif.pings.onsent.clone()),
            Provider::Klipy => {
                let Some(key) = self.key(Provider::Klipy) else { return };
                let customer = self.customer();
                let query = match self.asked.as_ref().map(|r| &r.query) {
                    Some(Query::Search(q)) => Some(q.clone()),
                    _ => None,
                };
                let (url, body) = gs::klipy_share(&key, &gif.id, &customer, query.as_deref());
                if let Some((rt, client)) = self.runtime() {
                    rt.spawn(async move { gs::share(&client, url, body).await });
                }
            }
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, p: &Palette, icons: &crate::ui::icons::Icons) -> Out {
        let ctx = ui.ctx().clone();
        let pass = ctx.cumulative_pass_nr();
        let now = ui.input(|i| i.time);
        let opened = self.shown_pass.is_none_or(|last| last + 1 < pass);
        self.shown_pass = Some(pass);
        self.active = true;
        crate::ui::emoji::note_picker_shown(&ctx);
        let mut out = Out {
            escape: ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)),
            ..Out::default()
        };
        self.pump(&ctx, pass);
        ui.set_width(WIDTH);
        let tab = self.tab();
        // ---- the services, then the favourites
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // a tab for each service with a key
            for provider in self.providers() {
                if tab_button(ui, p, provider.name(), tab == provider && !self.favorites).clicked() {
                    self.tab = Some(provider);
                    self.favorites = false;
                    if self.settings.provider != provider {
                        self.settings.provider = provider;
                        self.changed = true;
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let icon = if self.favorites { "star-filled" } else { "star" };
                let resp = icon_tab(ui, p, icons, icon, self.favorites).on_hover_text(aurora_i18n::tr!("ui-chat-gif-favorites"));
                if resp.clicked() {
                    self.favorites = !self.favorites;
                }
            });
        });
        // ---- the search ("Search KLIPY" is the placeholder KLIPY asks for)
        let search_id = egui::Id::unique("gif_picker_search");
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(search_id)
                .hint_text(aurora_i18n::tr!("ui-chat-gif-search", service = tab.name()))
                .desired_width(f32::INFINITY),
        );
        if search.changed() {
            self.typed_at = Some(now);
            self.favorites = false;
        }
        if opened || ctx.memory(|m| m.focused().is_none()) {
            search.request_focus();
        }
        // ---- the categories, while nothing is typed
        self.fetch_categories(&ctx, tab);
        let chips: Vec<Category> = self.categories.get(&tab).cloned().unwrap_or_default();
        egui::ScrollArea::horizontal()
            .id_salt("gif_picker_categories")
            .max_height(26.0)
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if chips.is_empty() {
                        // the row keeps its height
                        ui.add_space(22.0);
                    }
                    for c in &chips {
                        let on = !self.favorites && self.query.trim().eq_ignore_ascii_case(c.query.trim());
                        if chip(ui, p, &c.label, on).clicked() {
                            self.query = c.query.clone();
                            self.typed_at = None;
                            self.favorites = false;
                        }
                    }
                });
            });
        crate::ui::widgets::divider(ui, p);
        // ---- what to show, then the grid
        if let Some(request) = self.wanted(now) {
            self.begin(request.clone());
            self.fetch(&ctx, &request, gs::first_page(request.provider));
        } else if let Some(t) = self.typed_at
            && now - t < DEBOUNCE
        {
            ctx.request_repaint_after(Duration::from_secs_f64(DEBOUNCE - (now - t) + 0.01));
        }
        let (grid, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), GRID_H), egui::Sense::hover());
        let mut grid_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(grid)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        if let Some(gif) = self.grid(&mut grid_ui, p, tab, pass, now) {
            out.send = Some(gif);
        }
        // ---- the attribution, the settings
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let mark = egui::RichText::new(aurora_i18n::tr!("ui-chat-gif-powered-by", service = tab.name()))
                .size(10.5)
                .color(p.muted);
            ui.label(mark)
                .on_hover_text(aurora_i18n::tr!("ui-chat-gif-privacy", service = tab.name()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_tab(ui, p, icons, "settings-filled", false)
                    .on_hover_text(aurora_i18n::tr!("ui-chat-gif-settings"))
                    .clicked()
                {
                    self.open_settings = true;
                }
            });
        });
        out
    }

    /// The grid of tiles, or why there is none. A tile clicked is returned.
    fn grid(&mut self, ui: &mut egui::Ui, p: &Palette, tab: Provider, pass: u64, now: f64) -> Option<Gif> {
        self.drawn.clear();
        let note = |ui: &mut egui::Ui, text: &str| {
            ui.add_space(GRID_H * 0.35);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(text).size(12.0).color(p.muted));
            });
        };
        if self.key(tab).is_none() {
            note(ui, aurora_i18n::tr!("ui-chat-gif-no-key", service = tab.name()).as_str());
            ui.vertical_centered(|ui| {
                if ui.button(aurora_i18n::tr!("ui-chat-gif-open-settings")).clicked() {
                    self.open_settings = true;
                }
            });
            return None;
        }
        if let Some(e) = self.error {
            let text = match e {
                Error::Key => aurora_i18n::tr!("ui-chat-gif-key-refused", service = tab.name()),
                Error::RateLimited => aurora_i18n::tr!("ui-chat-gif-rate-limited", service = tab.name()),
                Error::Unavailable => aurora_i18n::tr!("ui-chat-gif-unavailable", service = tab.name()),
            };
            note(ui, &text);
            return None;
        }
        if self.gifs.is_empty() {
            let text = if self.loading {
                aurora_i18n::tr!("ui-chat-gif-loading").to_owned()
            } else if self.favorites {
                aurora_i18n::tr!("ui-chat-gif-no-favorites").to_owned()
            } else {
                aurora_i18n::tr!("ui-chat-gif-none").to_owned()
            };
            note(ui, &text);
            return None;
        }
        // the scroll bar keeps its own strip on the right
        let room = ui.available_width() - ui.spacing().scroll.allocated_width();
        let tile_w = ((room - GAP * (COLS - 1) as f32) / COLS as f32).floor();
        let rows = self.gifs.len().div_ceil(COLS);
        let mut clicked = None;
        let mut star = None;
        let mut last_row_shown = false;
        let mut next_frame: Option<Duration> = None;
        ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
        egui::ScrollArea::vertical()
            .id_salt(("gif_picker_grid", tab, self.favorites))
            .max_height(GRID_H)
            .min_scrolled_height(GRID_H)
            .auto_shrink([false, false])
            .show_rows(ui, TILE_H, rows, |ui, range| {
                last_row_shown = range.end >= rows;
                for r in range {
                    ui.horizontal(|ui| {
                        for i in r * COLS..((r + 1) * COLS).min(self.gifs.len()) {
                            let gif = self.gifs[i].clone();
                            self.want_thumb(&gif.thumb, gif.provider, pass);
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(tile_w, TILE_H), egui::Sense::click());
                            self.drawn.push((rect, gif.id.clone()));
                            let radius = ui.visuals().widgets.inactive.corner_radius;
                            ui.painter().rect_filled(rect, radius, p.field);
                            let mut shown = false;
                            if let Some(Tile {
                                thumb: Thumb::Ready(frames),
                                ..
                            }) = self.tiles.get(&gif.thumb)
                                && let Some((texture, wait)) = frames.at(now)
                            {
                                let uv = cover_uv(texture.size_vec2(), rect.size());
                                ui.painter().image(texture.id(), rect, uv, crate::ui::widgets::PICTURE_TINT);
                                if let Some(w) = wait {
                                    next_frame = Some(next_frame.map_or(w, |n| n.min(w)));
                                }
                                shown = true;
                            }
                            if shown && gif.provider == Provider::Giphy && self.seen.insert(gif.id.clone()) {
                                self.ping(gif.pings.onload.clone());
                            }
                            let favorite = self.settings.is_favorite(gif.provider, &gif.id);
                            let star_rect = egui::Rect::from_min_size(rect.right_top() + egui::vec2(-24.0, 4.0), egui::Vec2::splat(20.0));
                            let hovered = resp.hovered();
                            if hovered || favorite {
                                let on_star = ui.rect_contains_pointer(star_rect);
                                ui.painter()
                                    .rect_filled(star_rect, radius, p.panel.gamma_multiply(if on_star { 0.95 } else { 0.75 }));
                                super::thread::paint_icon(
                                    ui.painter(),
                                    if favorite { "star-filled" } else { "star" },
                                    star_rect.shrink(3.0),
                                    if favorite { p.amber } else { p.ink },
                                );
                            }
                            if hovered {
                                ui.painter()
                                    .rect_stroke(rect, radius, ui.visuals().widgets.hovered.bg_stroke, egui::StrokeKind::Inside);
                            }
                            let resp = if gif.title.is_empty() {
                                resp
                            } else {
                                resp.on_hover_text(gif.title.as_str())
                            };
                            if resp.clicked() {
                                let on_star = resp.interact_pointer_pos().is_some_and(|pos| star_rect.contains(pos));
                                if on_star {
                                    star = Some(gif);
                                } else {
                                    clicked = Some(gif);
                                }
                            }
                        }
                    });
                }
            });
        if let Some(d) = next_frame {
            ui.ctx().request_repaint_after(d);
        }
        if let Some(gif) = star {
            self.settings.toggle_favorite(gif.provider, &gif.id);
            self.changed = true;
        }
        if last_row_shown && let Some((request, page)) = self.more() {
            self.fetch(&ui.ctx().clone(), &request, page);
        }
        if self.demo_send
            && clicked.is_none()
            && let Some(first) = self.gifs.first()
            && matches!(
                self.tiles.get(&first.thumb),
                Some(Tile {
                    thumb: Thumb::Ready(_),
                    ..
                })
            )
        {
            self.demo_send = false;
            clicked = Some(first.clone());
        }
        if let Some(gif) = &clicked {
            self.ping(gif.pings.onclick.clone());
        }
        clicked
    }
}

/// Where `AURORA_DEMO_GIFS=menu` finds the GIF button.
const DEMO_BUTTON: &str = "gif_button_demo";

/// `AURORA_DEMO_GIFS=menu`: a right click on the GIF button at frame 700.
pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if !(700..=702).contains(&frame) {
        return Vec::new();
    }
    let Some(rect) = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(DEMO_BUTTON))) else {
        return Vec::new();
    };
    let pos = rect.center();
    let press = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    match frame {
        700 => vec![egui::Event::PointerMoved(pos)],
        701 => vec![press(true)],
        _ => vec![press(false)],
    }
}

/// The GIF button of a conversation's field: a click opens the picker (and
/// closes the emoji one); its right-click menu hides it (Preferences shows
/// it again) or opens its settings.
pub(super) fn button(resp: &egui::Response, p: &Palette, st: &mut super::ChatUi) {
    if resp.clicked() {
        st.gif_open = !st.gif_open;
        st.emoji_open = false;
    }
    if st.gifs.demo_menu {
        resp.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(DEMO_BUTTON), resp.rect));
    }
    crate::ui::menu::context_menu_of_button(resp, p, |ui| {
        if crate::ui::menu::item(ui, p, "eye-off", aurora_i18n::tr!("ui-chat-gif-hide")) {
            st.gifs.settings.hide();
            st.gifs.changed = true;
            st.gif_open = false;
        }
        if crate::ui::menu::item(ui, p, "settings-filled", aurora_i18n::tr!("ui-chat-gif-settings-menu")) {
            st.gifs.open_settings = true;
        }
    });
}

/// The GIF picker of a conversation's field, above `anchor` (its left
/// bottom corner `gap` points over it), when it is open. A GIF clicked is
/// sent at once as the conversation's message (`chat_type` in the nearby
/// chat); then, or on Escape, the picker closes and `field` gets the
/// keyboard back.
#[allow(clippy::too_many_arguments)]
pub(super) fn popup(
    ctx: &egui::Context,
    p: &Palette,
    icons: &crate::ui::icons::Icons,
    st: &mut super::ChatUi,
    anchor: egui::Pos2,
    gap: f32,
    field: egui::Id,
    chat_type: aurora_net::ChatType,
    actions: &mut Vec<super::ConvAction>,
) {
    if !st.gif_open || !super::popout::claim_picker(ctx) {
        return;
    }
    let mut out = Out::default();
    // one per conversation: one in each window (`popout.rs`)
    egui::Area::new(egui::Id::unique(("gif_picker", st.selected)))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(anchor - egui::vec2(0.0, gap))
        .show(ctx, |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                out = st.gifs.show(ui, p, icons);
            });
        });
    if let Some(gif) = out.send {
        actions.push(match st.selected {
            None => super::ConvAction::Local(super::OutgoingChat {
                message: gif.send.clone(),
                channel: 0,
                chat_type,
            }),
            Some(to) => super::ConvAction::Im {
                to,
                text: gif.send.clone(),
            },
        });
        st.gifs.sent(&gif);
        out.escape = true;
    }
    if out.escape {
        st.gif_open = false;
        ctx.memory_mut(|m| m.request_focus(field));
    }
}

/// The part of a picture of `size` that fills a tile of `tile` without
/// stretching it: its middle.
fn cover_uv(size: egui::Vec2, tile: egui::Vec2) -> egui::Rect {
    if size.x <= 0.0 || size.y <= 0.0 || tile.x <= 0.0 || tile.y <= 0.0 {
        return egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    let (pic, cell) = (size.x / size.y, tile.x / tile.y);
    if pic > cell {
        let w = cell / pic;
        egui::Rect::from_min_max(egui::pos2((1.0 - w) / 2.0, 0.0), egui::pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = pic / cell;
        egui::Rect::from_min_max(egui::pos2(0.0, (1.0 - h) / 2.0), egui::pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// A service's tab: its name, lit while chosen (the emoji picker's tabs).
fn tab_button(ui: &mut egui::Ui, p: &Palette, text: &str, selected: bool) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.5), egui::Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 16.0, 24.0), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.3));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter()
        .galley(pos, galley, if selected || resp.hovered() { p.ink } else { p.muted });
    resp
}

/// An icon tab of the picker's header and footer.
fn icon_tab(ui: &mut egui::Ui, p: &Palette, icons: &crate::ui::icons::Icons, icon: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.3));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    if let Some(t) = icons.get(icon) {
        let r = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if selected || resp.hovered() { p.ink } else { p.muted },
        );
    }
    resp
}

/// A category: a small pill, lit while it is the search.
fn chip(ui: &mut egui::Ui, p: &Palette, text: &str, on: bool) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), egui::Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 14.0, 22.0), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    let fill = if on {
        p.violet.gamma_multiply(0.3)
    } else if resp.hovered() {
        p.raised
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, radius, fill);
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley, if on || resp.hovered() { p.ink } else { p.muted });
    resp
}

#[cfg(test)]
mod tests;
