//! The photo's filters editor (issue #969, redesigned on 2026-11-17 from
//! the mockup the human chose), in the viewfinder's filter panel and the
//! photo window's Filters section, the Director's Filters tab and the
//! World filters window (`ui/world_look.rs`, the world's own stack).
//!
//! The active effects are cards, the first one drawn first, each with its
//! eye (drawn, or kept in its place undrawn), name, family and strength,
//! opened to its settings. They are dragged to reorder them
//! (`widgets::reorder`: the card lifted under the pointer, the others
//! gliding out of its way, the places they take shown on them instead of
//! their eye while one is held, a number that changes turning over), moved
//! with Alt + ↑ ↓ on a focused card or with the card's ⋯ actions; a card
//! dropped on the catalogue is taken out.
//!
//! The catalogue lists the looks in their three families (the Legacy
//! Viewer's filters, Aurora's by section, the LUTs) with a search over all
//! three, each tile a picture of the scene through its look
//! (`Options::thumb`). A tile pointed at is tried on the live view
//! (`snapshot::looks::tried`); clicked, it is added at the end, or swaps the
//! chosen card while Replace is on; dragged into the cards, it takes the
//! place it is dropped at.

use super::snapshot::{Action, chip};
use super::widgets::{self, ActionKind, TrackFace, TrackLit, reorder};
use crate::snapshot::Settings;
use crate::snapshot::knobs::{self, Kind, Knob};
use crate::snapshot::looks::{self, Entry, Layer, MAX_LAYERS, Tab, WorldLook};
use crate::theme::Palette;
use aurora_i18n::UiUpper;
use aurora_render::looks::AuroraSection;
use egui::{Align, Layout, Rect, RichText, Sense, UiBuilder, Vec2};
use std::collections::{HashMap, HashSet};

/// The stack the editor changes, with the settings of its Aurora looks:
/// the photos' and films' (`photo`) or the world's (`world`). The stars are
/// the user's, one list for every editor.
pub struct Stack<'a> {
    pub looks: &'a mut Vec<Layer>,
    pub look_settings: &'a mut looks::Settings,
    pub favorites: &'a mut Vec<String>,
}

impl<'a> Stack<'a> {
    pub fn photo(s: &'a mut Settings) -> Self {
        Stack {
            looks: &mut s.looks,
            look_settings: &mut s.look_settings,
            favorites: &mut s.look_favorites,
        }
    }

    pub fn world(w: &'a mut WorldLook, favorites: &'a mut Vec<String>) -> Self {
        Stack {
            looks: &mut w.looks,
            look_settings: &mut w.look_settings,
            favorites,
        }
    }
}

/// Height of a card's head (a closed card).
const HEAD: f32 = 40.0;
/// Space between two cards.
const GAP: f32 = 4.0;
/// Height of the catalogue's tabs (the poser's sliding track).
const TABS_H: f32 = 30.0;
/// Tiles a row of the catalogue, and the space between them.
const COLUMNS: usize = 3;
const TILE_GAP: f32 = 6.0;
/// How long a place number takes to turn over (the contact sets' tiles).
const NUMBER_TURN: f64 = 0.16;
/// How long a card's settings or the catalogue take to unroll (seconds).
const UNROLL: f32 = 0.12;
/// How long a tile's outline, "+" and star take to fade in or out.
const FADE: f32 = 0.08;
/// How long a tile takes to glide to a new place in its list, how much the
/// one that travels is lifted, and how long a star's pop lasts.
const GLIDE: f64 = 0.32;
const LIFT: f32 = 0.05;
const POP: f64 = 0.25;

/// A tile on its way to a new place in its list (from the list's corner).
#[derive(Debug, Clone, Copy)]
struct Glide {
    from: Vec2,
    to: Vec2,
    start: f64,
}

/// Where a gliding tile is at `now`, and how far along (0 to 1): it starts
/// fast, slows, and settles past its place by a hair before landing (an
/// ease-out with a light overshoot, a spring rather than a slide).
fn glide_at(g: Glide, now: f64) -> (Vec2, f32) {
    let t = (((now - g.start) / GLIDE) as f32).clamp(0.0, 1.0);
    const C1: f32 = 1.2;
    const C3: f32 = C1 + 1.0;
    let u = t - 1.0;
    let eased = 1.0 + C3 * u * u * u + C1 * u * u;
    (g.from + (g.to - g.from) * eased, t)
}
/// How long the numbers stay after a move from the keyboard or ⋯.
const FLASH: f64 = 0.9;
/// Aurora's sections, in the catalogue's order.
const SECTIONS: [AuroraSection; 5] = [
    AuroraSection::Film,
    AuroraSection::Color,
    AuroraSection::Lens,
    AuroraSection::Creative,
    AuroraSection::Depth,
];

/// A card's key in the reorder list: its look and which copy of it.
type Key = (String, usize);

/// A tile carried from the catalogue (`egui::DragAndDrop`).
#[derive(Clone, Debug)]
struct Carried(String);

/// The editor's state for the session.
#[derive(Debug, Default)]
pub struct LooksUi {
    pub tab: Tab,
    pub query: String,
    /// Aurora's section shown alone (None: all of them).
    pub section: Option<AuroraSection>,
    /// Replace: a click in the catalogue swaps the chosen card instead of
    /// adding one (the old "Replace the chosen one", no longer the default).
    pub replace: bool,
    /// The chosen card: the one Replace swaps; the viewfinder opens on its
    /// tab.
    pub selected: usize,
    /// The look pointed at in the catalogue, tried on the live view until
    /// the next frame of the interface (Some(None): "No filter").
    pub hover: Option<Option<String>>,
    /// The before / after comparison of the live view: the share of the
    /// photo's width left as it is.
    pub compare: Option<f32>,
    /// The photo window's settings scroll to the filters once
    /// (AURORA_DEMO_PHOTO_LOOK).
    pub reveal: bool,
    /// The catalogue folded away.
    pub folded: bool,
    /// The ★ list shown instead of a family: every starred look.
    pub favorites: bool,
    /// The catalogue's tiles drawn this frame, for their pictures.
    pub shown: Vec<String>,
    /// The cards open to their settings, by look (two copies of a look
    /// share its settings, so they open together).
    open: HashSet<String>,
    /// The card whose ⋯ actions are shown.
    menu: Option<usize>,
    /// The places shown on the cards while one is held: the number, the one
    /// before and when it changed (ui time).
    numbers: HashMap<Key, (usize, Option<usize>, f64)>,
    /// Until when a move from the keyboard or ⋯ shows the numbers.
    flash: f64,
    /// Each card's height, measured when it was last drawn.
    heights: HashMap<Key, f32>,
    /// Each open card's settings, as tall as they were last drawn (what
    /// unrolls).
    bodies: HashMap<Key, f32>,
    /// The card being carried (from its press to its release), and whether
    /// it is out of its list (`reorder::Layout::out`: over the take-out
    /// zone, a drop takes it out).
    carrying: Option<usize>,
    over_zone: bool,
    /// Where the cards and the take-out zone were drawn last frame: what a
    /// carried tile or card is dropped on. The zone is the dashed box only,
    /// never the catalogue's row above it (feedback 2026-11-21: a card
    /// touching the row's title was already taken out).
    cards_rect: Option<Rect>,
    drawn: Vec<Rect>,
    zone_rect: Option<Rect>,
    /// The catalogue list's corner, and the view it shows (its tiles'
    /// places are kept per view).
    grid_origin: egui::Pos2,
    view: Option<egui::Id>,
    /// The tiles gliding to a new place, and the stars popping (when
    /// clicked).
    glides: HashMap<egui::Id, Glide>,
    pops: HashMap<String, f64>,
    /// The catalogue's height under its row (its spacing included), kept by
    /// the zone that takes a card out; its row's, for a folded catalogue.
    catalogue_height: f32,
    bar_height: f32,
    /// Everything but the cards, as tall as it was last frame: what the
    /// cards leave of `Options::height`.
    rest: f32,
    /// The photo window is in its 360° format (`Options::pano`).
    pano: bool,
    /// What this editor's ids are made from: the photo window's and the
    /// Director's both show one, at the same time maybe.
    scope: Option<egui::Id>,
    /// A tile was taken out of this editor's catalogue: it draws the card
    /// under the pointer (the other editor does not).
    tile_out: bool,
}

/// What the editor shows besides the looks.
pub struct Options<'a> {
    /// The catalogue's height before it scrolls.
    pub list_height: f32,
    /// The editor's height at most (the viewfinder's panel): past it the
    /// cards scroll, the catalogue keeping its height. None: the cards take
    /// their own room (the photo window's section, in its own scroll).
    pub height: Option<f32>,
    /// The before / after button (the viewfinder).
    pub compare: bool,
    /// The user's tables, by file name.
    pub luts: &'a [String],
    /// The photo window is in its 360° format: only the colour looks (the
    /// ones a face can take alone) are drawn; the others stay in the stack,
    /// greyed.
    pub pano: bool,
    /// The slot of a tile's picture, by look (`snapshot::look_thumbs`).
    pub thumb: &'a dyn Fn(&str) -> Option<usize>,
}

/// A change asked from a card, applied once the cards are drawn.
enum Edit {
    Move(usize, usize),
    Remove(usize),
    Replace(usize),
    Choose(usize),
    Open(String),
    Menu(usize),
    /// An Aurora look's settings back to their defaults.
    Reset(String),
}

fn keep_keys(resp: &egui::Response) {
    if resp.clicked() || resp.drag_stopped() {
        resp.surrender_focus();
    }
}

/// A small text button: muted, lit when pointed at (the header's and the
/// Replace strip's, one style for all).
fn text_button(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.5), p.muted);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(8.0, 6.0), sense);
    let lit = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("lit"), enabled && resp.hovered(), 0.12);
    let ink = if enabled { p.muted.lerp_to_gamma(p.ink, lit) } else { p.muted_dim };
    if lit > 0.0 {
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            p.raised.gamma_multiply(0.5 * lit),
        );
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    resp
}

/// A short title: small, muted, in capitals.
fn caps(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.label(RichText::new(text.ui_upper()).size(10.5).strong().color(p.muted))
}

/// The keys of the cards: each look with which copy of it it is.
fn keys_of(stack: &[looks::Layer]) -> Vec<Key> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    stack
        .iter()
        .map(|l| {
            let n = seen.entry(l.id.as_str()).or_default();
            *n += 1;
            (l.id.clone(), *n - 1)
        })
        .collect()
}

/// The places (from 1) the cards take while something is held: the card
/// carried from `from` over place `to`, or a tile from the catalogue making
/// room before `gap`; None when nothing is held.
fn places(n: usize, held: Option<(usize, usize)>, gap: Option<usize>) -> Option<Vec<usize>> {
    match (held, gap) {
        (Some((from, to)), _) => Some((0..n).map(|i| reorder::place_in(i, from, to) + 1).collect()),
        (None, Some(g)) => Some((0..n).map(|i| i + 1 + usize::from(i >= g)).collect()),
        _ => None,
    }
}

/// The family icon drawn on a tile whose picture is not ready yet.
fn family_icon(id: &str) -> &'static str {
    match looks::tab_of(id) {
        Tab::Legacy => "filter-filled",
        Tab::Aurora => "color-filter",
        Tab::Luts => "photo-filled",
    }
}

/// A look's family, as a card's second line shows it.
fn family(id: &str) -> String {
    match looks::tab_of(id) {
        Tab::Aurora => {
            let section = id
                .strip_prefix(looks::AURORA)
                .and_then(aurora_render::looks::aurora_look)
                .map(|l| looks::section_name(l.section));
            match section {
                Some(section) => format!("{} · {section}", Tab::Aurora.label()),
                None => Tab::Aurora.label().to_owned(),
            }
        }
        tab => tab.label().to_owned(),
    }
}

impl LooksUi {
    /// Opens a look's cards to their settings (AURORA_DEMO_PHOTO_LOOK).
    pub fn open_card(&mut self, id: &str) {
        self.open.insert(id.to_owned());
    }

    /// Shows a card's ⋯ actions (AURORA_DEMO_PHOTO_LOOK).
    pub fn show_menu(&mut self, i: usize) {
        self.menu = Some(i);
    }

    /// The card a click swaps while Replace is on, over a stack of `n`.
    pub fn replace_target(&self, n: usize) -> Option<usize> {
        (self.replace && n > 0).then(|| self.selected.min(n - 1))
    }

    /// A look clicked in the catalogue (or imported): it swaps the chosen
    /// card while Replace is on, or goes on at the end; nothing when the
    /// stack is full.
    pub fn pick(&mut self, s: &mut Stack<'_>, id: &str) -> bool {
        let target = self.replace_target(s.looks.len());
        match looks::choose(s.looks, id, target) {
            Some(at) => {
                self.selected = at;
                true
            }
            None => false,
        }
    }

    /// The pointer is over this editor's cards, and they are not under
    /// another window there: with two editors open (the photo window and
    /// the World filters window, say), a tile goes to the cards it is seen
    /// dropped on, whichever editor is drawn first.
    fn over_own_cards(&self, ctx: &egui::Context, layer: egui::LayerId, at: egui::Pos2) -> bool {
        self.cards_rect.is_some_and(|r| r.contains(at)) && ctx.layer_id_at(at).is_none_or(|top| top == layer)
    }

    /// One of this editor's ids.
    fn id(&self, source: impl egui::AsIdSalt) -> egui::Id {
        self.scope.unwrap_or(egui::Id::NULL).with(source)
    }

    /// The whole editor.
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, mut s: Stack<'_>, opts: Options, out: &mut Vec<Action>) {
        let s = &mut s;
        let n = s.looks.len();
        self.selected = self.selected.min(n.saturating_sub(1));
        self.menu = self.menu.filter(|&m| m < n);
        self.pano = opts.pano;
        self.shown.clear();
        self.scope = Some(ui.make_persistent_id("looks_editor"));
        // what was kept for cards no longer in the stack, and the stars
        // done popping
        let keys = keys_of(s.looks);
        self.heights.retain(|k, _| keys.contains(k));
        self.bodies.retain(|k, _| keys.contains(k));
        let now = ui.input(|i| i.time);
        self.pops.retain(|_, at| now - *at < POP);
        // a card carried when the editor last showed, with the button up
        // since: its release happened while the panel was closed
        if !ui.input(|i| i.pointer.primary_down() || i.pointer.primary_released()) {
            self.carrying = None;
        }
        let start = ui.cursor().top();
        self.header(ui, p, s);
        // the cards take what the rest left last frame (a first guess
        // before it is measured)
        let rest = if self.rest > 0.0 { self.rest } else { 330.0 };
        let room = opts.height.map(|h| (h - rest).max(2.0 * (HEAD + GAP)));
        let cards_top = ui.cursor().top();
        // the panel takes its content's height, the whole height at most
        // (past it the cards scroll); it follows the cards and the
        // catalogue as they unroll, so that it never jumps
        let unrolled = ui.ctx().animate_bool_with_time(self.id("catalogue_open"), !self.folded, UNROLL);
        self.cards(ui, p, s, room);
        let cards = ui.cursor().top() - cards_top;
        widgets::divider(ui, p);
        // while a card is carried, the zone that takes it out stands in the
        // catalogue's place at its height (in its row's when it is folded):
        // nothing around it moves
        let spacing = ui.spacing().item_spacing.y;
        let zone = match (self.carrying.is_some(), self.folded) {
            (true, true) => Some(self.remove_zone(ui, p, self.bar_height - spacing)),
            (true, false) => {
                self.catalogue_bar(ui, p, s, &opts);
                Some(self.remove_zone(ui, p, self.catalogue_height - spacing))
            }
            (false, _) => {
                let start = ui.cursor().top();
                self.catalogue_bar(ui, p, s, &opts);
                self.bar_height = ui.cursor().top() - start;
                if unrolled > 0.0 {
                    // drawn whole in a ui of its own, shown down to the part
                    // unrolled; the page takes that part
                    let at = ui.cursor().min;
                    let width = ui.available_width();
                    let full = self.catalogue_height;
                    let shown = Rect::from_min_size(at, Vec2::new(width, full * unrolled));
                    let mut catalogue = ui.new_child(
                        UiBuilder::new()
                            .id_salt("looks_catalogue")
                            .max_rect(Rect::from_min_size(at, Vec2::new(width, f32::INFINITY)))
                            .layout(Layout::top_down(Align::Min)),
                    );
                    catalogue.set_clip_rect(shown.intersect(ui.clip_rect()));
                    self.catalogue(&mut catalogue, p, s, &opts, out);
                    let measured = catalogue.min_rect().height() + spacing;
                    if (measured - full).abs() > 0.5 {
                        self.catalogue_height = measured;
                        ui.ctx().request_repaint();
                    }
                    ui.allocate_exact_size(Vec2::new(width, (measured * unrolled - spacing).max(0.0)), Sense::hover());
                }
                None
            }
        };
        // only what shows of it: a zone taller than its window's view (the
        // World filters window, the photo window's scrolled section) takes
        // nothing out past the window's edge
        self.zone_rect = zone.map(|z| z.intersect(ui.clip_rect())).filter(|z| z.is_positive());
        let rest = ui.cursor().top() - start - cards;
        if (rest - self.rest).abs() > 0.5 {
            self.rest = rest;
            ui.ctx().request_repaint();
        }
        self.carried_tile(ui, p, s);
    }

    /// "Active", the count, Expand / Collapse all and "No filter".
    fn header(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Stack<'_>) {
        ui.horizontal(|ui| {
            // how the cards work, where it is asked for (no line of
            // instructions kept on screen)
            let hint = aurora_i18n::tr!("photo-looks-hint");
            caps(ui, p, aurora_i18n::tr!("photo-looks-active")).on_hover_text(hint);
            ui.label(
                RichText::new(aurora_i18n::tr!("photo-looks-count", count = s.looks.len(), max = MAX_LAYERS))
                    .size(11.0)
                    .color(p.muted),
            )
            .on_hover_text(hint);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let none = text_button(ui, p, aurora_i18n::tr!("photo-look-none"), !s.looks.is_empty());
                keep_keys(&none);
                if none.hovered() {
                    self.hover = Some(None);
                }
                if none.clicked() {
                    s.looks.clear();
                    self.selected = 0;
                }
                if !s.looks.is_empty() {
                    let any_open = s.looks.iter().any(|l| self.open.contains(&l.id));
                    let label = if any_open {
                        aurora_i18n::tr!("photo-looks-collapse-all")
                    } else {
                        aurora_i18n::tr!("photo-looks-expand-all")
                    };
                    let all = text_button(ui, p, label, true);
                    keep_keys(&all);
                    if all.clicked() {
                        if any_open {
                            self.open.clear();
                        } else {
                            self.open.extend(s.looks.iter().map(|l| l.id.clone()));
                        }
                    }
                }
            });
        });
    }

    /// The cards: dragged to reorder them, a tile dropped among them.
    fn cards(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Stack<'_>, room: Option<f32>) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let released = ui.input(|i| i.pointer.primary_released());
        let keys = keys_of(s.looks);
        let n = keys.len();
        // a tile carried over the cards opens room where it would go
        let carried_tile = egui::DragAndDrop::payload::<Carried>(&ctx);
        let over_cards = pointer.is_some_and(|at| self.over_own_cards(&ctx, ui.layer_id(), at));
        let gap = (carried_tile.is_some() && over_cards && n < MAX_LAYERS)
            .then(|| pointer.map(|at| reorder::insert_place(at.y, &self.drawn)))
            .flatten();
        let mut edits = Vec::new();
        let before = ui.cursor().top();
        // (an area's content only gets the room the area had last frame: a
        // list that scrolls takes its own height, or it could never grow)
        let scroll = egui::ScrollArea::vertical()
            .id_salt("looks_cards")
            .max_height(room.unwrap_or(f32::INFINITY))
            .min_scrolled_height(room.unwrap_or(4.0 * (HEAD + GAP)))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = GAP;
                let width = ui.available_width();
                let rows: Vec<egui::Response> = keys
                    .iter()
                    .map(|k| {
                        let h = self.heights.get(k).copied().unwrap_or(HEAD);
                        ui.allocate_exact_size(Vec2::new(width, h), Sense::click_and_drag()).1
                    })
                    .collect();
                let id = ui.make_persistent_id("looks_reorder");
                let layout = reorder::update_sized(ui, id, &keys, &rows, true, gap.map(|g| (g, HEAD + GAP)), true);
                if let Some(i) = layout.started {
                    self.carrying = Some(i);
                    self.menu = None;
                }
                // the numbers: the places while something is held, or for a
                // moment after a move from the keyboard or ⋯
                let held = places(n, layout.held, gap).or_else(|| (now < self.flash).then(|| (1..=n).collect()));
                match held {
                    Some(places) => {
                        for (key, place) in keys.iter().zip(places) {
                            let entry = self.numbers.entry(key.clone()).or_insert((place, None, now));
                            if entry.0 != place {
                                *entry = (place, Some(entry.0), now);
                            }
                            if now - entry.2 < NUMBER_TURN {
                                ui.ctx().request_repaint();
                            }
                        }
                        if now < self.flash {
                            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(self.flash - now));
                        }
                    }
                    None => self.numbers.clear(),
                }
                self.drawn.clear();
                for (i, key) in keys.iter().enumerate() {
                    let rect = layout.rect(i, rows[i].rect);
                    self.drawn.push(rect);
                    let number = self
                        .numbers
                        .get(key)
                        .map(|&(shown, before, changed)| (shown, before, ((now - changed) / NUMBER_TURN) as f32));
                    let height = if layout.lifted == Some(i) {
                        let mut h = rect.height();
                        reorder::lifted_card(ui, p, id.with(i), rect, |card| {
                            h = self.card(card, p, s, i, rect, &rows[i], number, true, layout.busy(), &mut edits);
                        });
                        h
                    } else {
                        self.card(ui, p, s, i, rect, &rows[i], number, false, layout.busy(), &mut edits)
                    };
                    if (self.heights.get(key).copied().unwrap_or(HEAD) - height).abs() > 0.5 {
                        self.heights.insert(key.clone(), height);
                        ui.ctx().request_repaint();
                    }
                }
                // a card out of its list and touching the take-out zone is
                // taken out when let go; anywhere else it takes the place it
                // was let go at
                let over_catalogue = layout.out
                    && self
                        .carrying
                        .and_then(|i| self.drawn.get(i))
                        .is_some_and(|card| self.zone_rect.is_some_and(|zone| zone.intersects(*card)));
                self.over_zone = over_catalogue;
                match (released.then(|| self.carrying.take()).flatten(), layout.moved) {
                    (Some(from), _) if over_catalogue => edits.push(Edit::Remove(from)),
                    (_, Some((from, to))) => edits.push(Edit::Move(from, to)),
                    _ => {}
                }
                self.end_zone(ui, p, n, gap, carried_tile.is_some(), over_cards);
            });
        // where a tile may be dropped: what the list took of the page (its
        // `inner_rect` is the room it was allowed)
        self.cards_rect = Some(
            Rect::from_x_y_ranges(
                scroll.inner_rect.x_range(),
                before..=ui.cursor().top() - ui.spacing().item_spacing.y,
            )
            .intersect(ui.clip_rect()),
        );
        // a tile let go over the cards takes the place it opened
        if released
            && let Some(tile) = carried_tile
            && let Some(at) = gap
        {
            if let Some(at) = looks::insert(s.looks, at, &tile.0) {
                self.selected = at;
            }
            egui::DragAndDrop::clear_payload(&ctx);
        }
        for edit in edits {
            self.apply(s, edit);
        }
    }

    /// A change asked from a card.
    fn apply(&mut self, s: &mut Stack<'_>, edit: Edit) {
        match edit {
            Edit::Move(from, to) => {
                if from < s.looks.len() && to < s.looks.len() {
                    let layer = s.looks.remove(from);
                    s.looks.insert(to, layer);
                    self.selected = reorder::place_in(self.selected, from, to);
                    self.menu = self.menu.map(|m| reorder::place_in(m, from, to));
                }
            }
            Edit::Remove(i) => {
                if i < s.looks.len() {
                    s.looks.remove(i);
                    if self.selected > i || self.selected >= s.looks.len() {
                        self.selected = self.selected.saturating_sub(1);
                    }
                    self.menu = None;
                }
            }
            Edit::Replace(i) => {
                self.replace = true;
                self.selected = i;
                self.menu = None;
                self.folded = false;
                if let Some(layer) = s.looks.get(i) {
                    self.tab = looks::tab_of(&layer.id);
                }
            }
            Edit::Choose(i) => self.selected = i,
            Edit::Open(id) => {
                if !self.open.remove(&id) {
                    self.open.insert(id);
                }
            }
            Edit::Menu(i) => self.menu = if self.menu == Some(i) { None } else { Some(i) },
            // (drawn again at their defaults: `settings` puts them back)
            Edit::Reset(id) => {
                s.look_settings.remove(&id);
            }
        }
    }

    /// Where a tile can be dropped, under the cards: shown with no card, or
    /// while a tile is carried (unrolling then, as Blizzard's empty slots
    /// show only with something in the hand); the note once full.
    fn end_zone(&mut self, ui: &mut egui::Ui, p: &Palette, n: usize, gap: Option<usize>, carrying: bool, over: bool) {
        if n >= MAX_LAYERS {
            ui.label(
                RichText::new(aurora_i18n::tr!("photo-looks-full", max = MAX_LAYERS))
                    .size(11.5)
                    .color(p.muted),
            );
            return;
        }
        let text = if n == 0 {
            aurora_i18n::tr!("photo-looks-empty")
        } else {
            aurora_i18n::tr!("photo-looks-drop")
        };
        let shown = ui.ctx().animate_bool_with_time(self.id("drop_zone"), n == 0 || carrying, UNROLL);
        if shown <= 0.0 {
            return;
        }
        let width = ui.available_width();
        let galley = ui
            .painter()
            .layout(text.to_owned(), egui::FontId::proportional(11.5), p.muted, (width - 24.0).max(10.0));
        let full = (galley.size().y + 18.0).max(HEAD);
        let (mut rect, _) = ui.allocate_exact_size(Vec2::new(width, full * shown), Sense::hover());
        // the cards slid down to make room: the zone goes with them
        if gap.is_some_and(|g| g < n) {
            rect = rect.translate(Vec2::new(0.0, HEAD + GAP));
        }
        let hot = carrying && over && gap == Some(n);
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let (fill, stroke) = if hot {
            (p.violet.gamma_multiply(0.14), p.violet)
        } else {
            (egui::Color32::TRANSPARENT, p.muted.gamma_multiply(0.35))
        };
        ui.painter().rect_filled(rect, radius, fill);
        widgets::dashed_outline(ui.painter(), rect, stroke);
        let color = if hot { p.ink } else { p.muted };
        let at = rect.center() - galley.size() / 2.0;
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .galley(at, galley, color.gamma_multiply(shown));
    }

    /// One card at `rect`: its head, its ⋯ actions and its settings when
    /// open. Returns the height it takes.
    #[allow(clippy::too_many_arguments)]
    fn card(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Stack<'_>,
        i: usize,
        rect: Rect,
        resp: &egui::Response,
        number: Option<(usize, Option<usize>, f32)>,
        lifted: bool,
        busy: bool,
        edits: &mut Vec<Edit>,
    ) -> f32 {
        let n = s.looks.len();
        let Some(layer) = s.looks.get(i).cloned() else {
            return HEAD;
        };
        let name = looks::label(Some(&layer.id));
        let drawn = !self.pano || looks::is_pointwise(&layer.id);
        let open = self.open.contains(&layer.id);
        let chosen = self.replace && i == self.selected;
        let radius = ui.visuals().widgets.inactive.corner_radius;
        if !lifted {
            let hovered = !busy && resp.hovered();
            let fill = if hovered { p.field.lerp_to_gamma(p.raised, 0.45) } else { p.field };
            ui.painter().rect_filled(rect, radius, fill);
            if chosen {
                ui.painter()
                    .rect_stroke(rect, radius, egui::Stroke::new(1.0, p.violet), egui::StrokeKind::Inside);
            }
        }
        // a click on the head opens or closes the card and makes it the
        // chosen one (the one Replace swaps); Alt + ↑ ↓ on a focused card
        // moves it
        let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEAD));
        if resp.clicked() && resp.interact_pointer_pos().is_none_or(|at| head.contains(at)) {
            edits.push(Edit::Choose(i));
            edits.push(Edit::Open(layer.id.clone()));
        }
        // a right click takes it out
        if resp.secondary_clicked() {
            edits.push(Edit::Remove(i));
        }
        keep_keys(resp);
        if resp.has_focus() {
            let (up, down) = ui.input(|inp| {
                (
                    inp.modifiers.alt && inp.key_pressed(egui::Key::ArrowUp),
                    inp.modifiers.alt && inp.key_pressed(egui::Key::ArrowDown),
                )
            });
            if up && i > 0 {
                self.flash_move(ui, s, i, i - 1, edits);
            } else if down && i + 1 < n {
                self.flash_move(ui, s, i, i + 1, edits);
            }
        }
        let label = aurora_i18n::tr!("photo-looks-card", name = name.as_ref(), place = i + 1, count = n);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
                .layout(Layout::top_down(Align::Min)),
        );
        let width = child.available_width();
        child.allocate_ui_with_layout(Vec2::new(width, HEAD), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            match number {
                Some((shown, before, turn)) => {
                    let (slot, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    // a little lift as the number turns over
                    let pop = (turn.clamp(0.0, 1.0) * std::f32::consts::PI).sin() * 1.5;
                    let slot = slot.expand(pop);
                    let (face, ink) = if lifted {
                        (p.violet, p.on_accent)
                    } else {
                        (p.violet.gamma_multiply(0.3), p.ink)
                    };
                    ui.painter().rect_filled(slot, radius, face);
                    widgets::turning_number(ui, slot, shown, before, turn, ink);
                }
                None => {
                    let (icon, tip) = if layer.on {
                        ("eye-filled", aurora_i18n::tr!("photo-looks-hide", name = name.as_ref()))
                    } else {
                        ("eye-off", aurora_i18n::tr!("photo-looks-show", name = name.as_ref()))
                    };
                    if widgets::icon_button(ui, p, icon, &tip, true)
                        && let Some(l) = s.looks.get_mut(i)
                    {
                        l.on = !l.on;
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::icon_toggle(ui, p, "dots-filled", aurora_i18n::tr!("photo-looks-more"), self.menu == Some(i)) {
                    edits.push(Edit::Menu(i));
                }
                let chevron = if open { "chevron-up" } else { "chevron-down" };
                let tip = aurora_i18n::tr!("photo-looks-settings", name = name.as_ref());
                if widgets::icon_button(ui, p, chevron, &tip, true) {
                    edits.push(Edit::Open(layer.id.clone()));
                }
                if drawn {
                    ui.label(RichText::new(format!("{} %", layer.strength)).size(11.5).color(p.muted));
                } else {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("photo-looks-not-in-360-tag"))
                            .size(10.5)
                            .color(p.warn),
                    )
                    .on_hover_text(aurora_i18n::tr!("photo-looks-not-in-360"));
                }
                // the name and family in what is left
                let text = ui.available_rect_before_wrap();
                let ink = if layer.on && drawn { p.ink } else { p.muted_dim };
                let mut job = egui::text::LayoutJob::single_section(
                    name.to_string(),
                    crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 13.0, ink),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(text.width().max(0.0));
                let g = ui.painter().layout_job(job);
                let mut job = egui::text::LayoutJob::single_section(
                    family(&layer.id),
                    egui::TextFormat::simple(egui::FontId::proportional(10.5), p.muted),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(text.width().max(0.0));
                let fam = ui.painter().layout_job(job);
                let top = text.center().y - (g.size().y + fam.size().y) / 2.0;
                let gh = g.size().y;
                ui.painter().galley(egui::pos2(text.left(), top), g, ink);
                ui.painter().galley(egui::pos2(text.left(), top + gh), fam, p.muted);
            });
        });
        if self.menu == Some(i) {
            child.horizontal(|ui| {
                ui.add_space(26.0);
                ui.horizontal_wrapped(|ui| self.card_actions(ui, p, s, i, n, edits));
            });
            child.add_space(6.0);
        }
        // the settings unroll as the card opens and roll up as it closes:
        // the card's height follows, and the cards under it glide with it
        let copy = s.looks[..i].iter().filter(|l| l.id == layer.id).count();
        let key = (layer.id.clone(), copy);
        let t = ui
            .ctx()
            .animate_bool_with_time(self.id(("card_open", key.0.as_str(), copy)), open, UNROLL);
        if t <= 0.0 {
            return child.min_rect().height().max(HEAD);
        }
        let top = child.min_rect().bottom() + child.spacing().item_spacing.y;
        let full = self.bodies.get(&key).copied().unwrap_or(0.0);
        let shown = Rect::from_min_max(egui::pos2(rect.left(), top), egui::pos2(rect.right(), top + full * t));
        let mut body = child.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    egui::pos2(child.min_rect().left(), top),
                    Vec2::new(width, f32::INFINITY),
                ))
                .layout(Layout::top_down(Align::Min)),
        );
        body.set_clip_rect(shown.intersect(child.clip_rect()));
        body.horizontal(|ui| {
            ui.add_space(26.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_enabled_ui(drawn, |ui| settings(ui, p, s, i));
            });
        });
        body.add_space(8.0);
        let measured = body.min_rect().height();
        if (measured - full).abs() > 0.5 {
            self.bodies.insert(key, measured);
            ui.ctx().request_repaint();
        }
        (top - rect.top() + measured * t).max(HEAD)
    }

    /// A card's ⋯ actions: the same as dragging, for who does not drag.
    fn card_actions(&mut self, ui: &mut egui::Ui, p: &Palette, s: &Stack<'_>, i: usize, n: usize, edits: &mut Vec<Edit>) {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let size = Vec2::new(0.0, 24.0);
        let up = widgets::action_button_if(
            ui,
            p,
            "arrow-up",
            aurora_i18n::tr!("photo-looks-move-up"),
            ActionKind::Flat,
            size,
            i > 0,
        );
        if up.clicked() {
            self.flash_move(ui, s, i, i - 1, edits);
        }
        let down = widgets::action_button_if(
            ui,
            p,
            "arrow-down",
            aurora_i18n::tr!("photo-looks-move-down"),
            ActionKind::Flat,
            size,
            i + 1 < n,
        );
        if down.clicked() {
            self.flash_move(ui, s, i, i + 1, edits);
        }
        let replace = widgets::action_button(
            ui,
            p,
            "replace-filled",
            aurora_i18n::tr!("photo-looks-replace-one"),
            ActionKind::Flat,
            size,
        );
        if replace.clicked() {
            edits.push(Edit::Replace(i));
        }
        // an Aurora look's settings back to their defaults (the others have
        // none), once one is changed
        if let Some(layer) = s.looks.get(i).filter(|l| l.id.starts_with(looks::AURORA)) {
            let reset = widgets::action_button_if(
                ui,
                p,
                "restore",
                aurora_i18n::tr!("photo-looks-reset"),
                ActionKind::Flat,
                size,
                changed(s, &layer.id),
            );
            if reset.clicked() {
                edits.push(Edit::Reset(layer.id.clone()));
            }
        }
        let out = widgets::action_button(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("photo-looks-take-out"),
            ActionKind::FlatDanger,
            size,
        );
        if out.clicked() {
            edits.push(Edit::Remove(i));
        }
    }

    /// A move from the keyboard or ⋯: the numbers show, and turn over.
    fn flash_move(&mut self, ui: &egui::Ui, s: &Stack<'_>, from: usize, to: usize, edits: &mut Vec<Edit>) {
        let now = ui.input(|i| i.time);
        for (place, key) in keys_of(s.looks).into_iter().enumerate() {
            self.numbers.insert(key, (place + 1, None, now - NUMBER_TURN));
        }
        self.flash = now + FLASH;
        edits.push(Edit::Move(from, to));
    }

    /// The catalogue's row: its title (folding it), Replace and before /
    /// after.
    fn catalogue_bar(&mut self, ui: &mut egui::Ui, p: &Palette, s: &Stack<'_>, opts: &Options) {
        let n = s.looks.len();
        // folded, the row is as tall as a closed card, and so is the zone
        // that stands in its place while a card is carried (feedback
        // 2026-11-21: the row's own height made a tiny target); it eases to
        // the buttons' height as the catalogue unrolls
        let unrolled = ui.ctx().animate_bool_with_time(self.id("catalogue_open"), !self.folded, UNROLL);
        let row = egui::lerp(HEAD..=ui.spacing().interact_size.y, unrolled);
        let width = ui.available_width();
        ui.allocate_ui_with_layout(Vec2::new(width, row), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_size(Vec2::new(width, row));
            let chevron = if self.folded { "chevron-right" } else { "chevron-down" };
            if widgets::icon_button(ui, p, chevron, aurora_i18n::tr!("photo-looks-catalogue-fold"), true) {
                self.folded = !self.folded;
            }
            caps(ui, p, aurora_i18n::tr!("photo-looks-catalogue")).on_hover_text(aurora_i18n::tr!("photo-looks-catalogue-tip"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if opts.compare {
                    ui.add_enabled_ui(n > 0, |ui| {
                        if widgets::icon_toggle(
                            ui,
                            p,
                            "columns-2-filled",
                            aurora_i18n::tr!("photo-look-compare-tip"),
                            self.compare.is_some(),
                        ) {
                            self.compare = if self.compare.is_some() { None } else { Some(0.5) };
                        }
                    });
                }
                let kind = if self.replace { ActionKind::Primary } else { ActionKind::Flat };
                // (always there: with no effect, Replace adds the first)
                let replace = widgets::action_button(
                    ui,
                    p,
                    "replace-filled",
                    aurora_i18n::tr!("photo-looks-replace"),
                    kind,
                    Vec2::new(0.0, 24.0),
                )
                .on_hover_text(aurora_i18n::tr!("photo-looks-replace-tip"));
                keep_keys(&replace);
                if replace.clicked() {
                    self.replace = !self.replace;
                    self.folded = false;
                }
            });
        });
        if let Some(at) = self.replace_target(n) {
            // one strip: what a click swaps, and Done at its end
            let name = looks::label(s.looks.get(at).map(|l| l.id.as_str()));
            let text = aurora_i18n::tr!("photo-looks-replacing", name = name.as_ref());
            let (strip, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
            ui.painter()
                .rect_filled(strip, ui.visuals().widgets.inactive.corner_radius, p.violet.gamma_multiply(0.16));
            ui.scope_builder(UiBuilder::new().max_rect(strip.shrink2(Vec2::new(8.0, 0.0))), |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let done = text_button(ui, p, aurora_i18n::tr!("photo-looks-done"), true);
                    keep_keys(&done);
                    if done.clicked() {
                        self.replace = false;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.ink)).truncate());
                    });
                });
            });
        }
    }

    /// While a card is carried, the catalogue takes it out: the zone drawn.
    fn remove_zone(&mut self, ui: &mut egui::Ui, p: &Palette, height: f32) -> Rect {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height.max(24.0)), Sense::hover());
        let hot = self.over_zone;
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let fill = if hot {
            p.danger.gamma_multiply(0.14)
        } else {
            egui::Color32::TRANSPARENT
        };
        let edge = p.danger.gamma_multiply(if hot { 1.0 } else { 0.5 });
        ui.painter().rect_filled(rect, radius, fill);
        widgets::dashed_outline(ui.painter(), rect, edge);
        // the bin above the words, or beside them in a low zone (the row of
        // a folded catalogue)
        let text = ui.painter().layout_no_wrap(
            aurora_i18n::tr!("photo-looks-take-out-zone").to_owned(),
            egui::FontId::proportional(12.0),
            p.danger,
        );
        let (icon, words) = if rect.height() >= 64.0 {
            (rect.center() - Vec2::new(0.0, 10.0), rect.center() + Vec2::new(0.0, 14.0))
        } else {
            let half = (16.0 + 6.0 + text.size().x) / 2.0;
            (
                egui::pos2(rect.center().x - half + 8.0, rect.center().y),
                egui::pos2(rect.center().x - half + 22.0 + text.size().x / 2.0, rect.center().y),
            )
        };
        let size = if rect.height() >= 64.0 { 20.0 } else { 16.0 };
        crate::ui::sheet::icon_at(ui, "trash-filled", icon, size, p.danger);
        ui.painter().galley(words - text.size() / 2.0, text, p.danger);
        rect
    }

    /// The tabs, the search, Aurora's sections and the tiles.
    fn catalogue(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Stack<'_>, opts: &Options, out: &mut Vec<Action>) {
        // the three families, then the ★ list, in one sliding track (the
        // poser's Body / Face / Hands / Other)
        let tip = aurora_i18n::tr!("photo-looks-favorites");
        let cells = [
            (TrackFace::Letter(Tab::Legacy.label()), "", Tab::Legacy.label()),
            (TrackFace::Letter(Tab::Aurora.label()), "", Tab::Aurora.label()),
            (TrackFace::Letter(Tab::Luts.label()), "", Tab::Luts.label()),
            (TrackFace::Icon("star-filled"), "", tip),
        ];
        let chosen = match self.favorites {
            true => Tab::ALL.len(),
            false => Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0),
        };
        let cell = Vec2::new(
            (ui.available_width() - widgets::TRACK_INSET * 2.0) / cells.len() as f32,
            TABS_H - widgets::TRACK_INSET * 2.0,
        );
        if let Some(picked) = widgets::icon_track(ui, p, "looks_tabs", cell, chosen, TrackLit::Accent, &cells) {
            self.favorites = picked == Tab::ALL.len();
            if let Some(tab) = Tab::ALL.get(picked) {
                self.tab = *tab;
            }
            self.query.clear();
        }
        // in a row: the field centres itself in the height it is given
        ui.horizontal(|ui| {
            widgets::clearable_search_box(ui, p, &mut self.query, aurora_i18n::tr!("photo-looks-search"), ui.available_width());
        });
        let query = self.query.trim().to_owned();
        let before = ui.cursor().top();
        egui::ScrollArea::vertical()
            .id_salt("looks_list")
            .max_height(opts.list_height)
            .min_scrolled_height(opts.list_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // the tiles' places are counted from the list's corner (a
                // scroll moves them all, not one) and in this view (another
                // tab, section or search lays them out afresh)
                self.grid_origin = ui.min_rect().min;
                let view = self.id((
                    "view",
                    self.favorites,
                    self.tab.label(),
                    self.section.map(looks::section_name),
                    query.as_str(),
                ));
                if self.view != Some(view) {
                    // another view lays its tiles out afresh
                    self.glides.clear();
                    self.view = Some(view);
                }
                // Aurora's sections scroll with its tiles: every tab keeps
                // the catalogue's height
                if query.is_empty() && !self.favorites && self.tab == Tab::Aurora {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                        let all = chip(ui, p, aurora_i18n::tr!("photo-looks-all"), None, self.section.is_none(), true);
                        keep_keys(&all);
                        if all.clicked() {
                            self.section = None;
                        }
                        for section in SECTIONS {
                            let c = chip(ui, p, looks::section_name(section), None, self.section == Some(section), true);
                            keep_keys(&c);
                            if c.clicked() {
                                self.section = Some(section);
                            }
                        }
                    });
                }
                if !query.is_empty() {
                    let found = looks::search(&query, opts.luts);
                    if found.is_empty() {
                        ui.label(RichText::new(aurora_i18n::tr!("photo-looks-no-result")).color(p.muted));
                    } else {
                        self.tiles(ui, p, s, &found, opts, false, out);
                    }
                    return;
                }
                if self.favorites {
                    let list = looks::favorite_list(s.favorites, opts.luts);
                    if list.is_empty() {
                        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-looks-favorites-empty")).color(p.muted)).wrap());
                    }
                    for (family, entries) in list {
                        ui.add_space(4.0);
                        caps(ui, p, family.label());
                        self.tiles(ui, p, s, &entries, opts, false, out);
                    }
                    return;
                }
                // in each list, the starred looks first
                match self.tab {
                    Tab::Legacy => {
                        let entries = looks::favorites_first(looks::legacy_list(), s.favorites);
                        self.tiles(ui, p, s, &entries, opts, false, out);
                    }
                    Tab::Aurora => {
                        for (section, entries) in looks::aurora_list() {
                            if self.section.is_some_and(|only| only != section) {
                                continue;
                            }
                            if self.section.is_none() {
                                ui.add_space(4.0);
                                caps(ui, p, looks::section_name(section));
                            }
                            let entries = looks::favorites_first(entries, s.favorites);
                            self.tiles(ui, p, s, &entries, opts, false, out);
                        }
                    }
                    Tab::Luts => self.luts(ui, p, s, opts, out),
                }
            });
        // a short list (the ★ list, a family searched) keeps the
        // catalogue's height: the window does not change size between tabs
        let short = opts.list_height - (ui.cursor().top() - before - ui.spacing().item_spacing.y);
        if short > 0.5 && opts.list_height.is_finite() {
            ui.add_space(short);
        }
    }

    /// The creators' tables under their name, then the user's (each with
    /// its bin), the import and the folder.
    fn luts(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Stack<'_>, opts: &Options, out: &mut Vec<Action>) {
        for (creator, entries) in looks::pack_list() {
            ui.add_space(4.0);
            caps(ui, p, &aurora_i18n::tr!("photo-lut-pack", creator = creator))
                .on_hover_text(aurora_i18n::tr!("photo-lut-pack-tip", creator = creator));
            let entries = looks::favorites_first(entries, s.favorites);
            self.tiles(ui, p, s, &entries, opts, false, out);
        }
        ui.add_space(4.0);
        caps(ui, p, aurora_i18n::tr!("photo-lut-mine"));
        let entries = looks::favorites_first(looks::lut_list(opts.luts), s.favorites);
        if entries.is_empty() {
            ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-lut-empty")).color(p.muted)).wrap());
        }
        self.tiles(ui, p, s, &entries, opts, true, out);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let import = widgets::action_button(
                ui,
                p,
                "file-import",
                aurora_i18n::tr!("photo-lut-import"),
                ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            );
            keep_keys(&import);
            if import.clicked() {
                out.push(Action::ImportLut);
            }
            let folder = widgets::action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("photo-lut-folder"),
                ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            );
            keep_keys(&folder);
            if folder.clicked() {
                out.push(Action::OpenLutFolder);
            }
        });
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-lut-hint")).size(11.0).color(p.muted)).wrap());
    }

    /// A grid of tiles; `mine`: the user's own tables, each with its bin.
    #[allow(clippy::too_many_arguments)]
    fn tiles(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Stack<'_>,
        entries: &[Entry],
        opts: &Options,
        mine: bool,
        out: &mut Vec<Action>,
    ) {
        let width = ((ui.available_width() - TILE_GAP * (COLUMNS - 1) as f32) / COLUMNS as f32)
            .floor()
            .max(40.0);
        for row in entries.chunks(COLUMNS) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = TILE_GAP;
                for e in row {
                    self.tile(ui, p, s, e, width, opts, mine, out);
                }
            });
            ui.add_space((TILE_GAP - ui.spacing().item_spacing.y).max(0.0));
        }
    }

    /// One tile: the scene through its look (its family's icon until the
    /// picture is ready) and its name.
    #[allow(clippy::too_many_arguments)]
    fn tile(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Stack<'_>,
        e: &Entry,
        width: f32,
        opts: &Options,
        mine: bool,
        out: &mut Vec<Action>,
    ) {
        let drawn = !self.pano || looks::is_pointwise(&e.id);
        let full = s.looks.len() >= MAX_LAYERS && !self.replace;
        let thumb_h = (width * 0.75).round();
        let sense = if drawn { Sense::click_and_drag() } else { Sense::hover() };
        // the name on up to two lines under the picture
        let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(width, thumb_h + 32.0), sense);
        if !ui.is_rect_visible(rect) {
            return;
        }
        self.shown.push(e.id.clone());
        // a tile that changes place (starred to the front of its list, or
        // back) glides there instead of jumping
        let at = rect.min - self.grid_origin;
        let key = self.view.unwrap_or(egui::Id::NULL).with(("tile", e.id.as_str()));
        let now = ui.input(|i| i.time);
        let glide = self.glides.entry(key).or_insert(Glide {
            from: at,
            to: at,
            start: now - GLIDE,
        });
        if (glide.to - at).length() > 0.5 {
            // from where it is drawn now, even mid-flight
            let (drawn, _) = glide_at(*glide, now);
            *glide = Glide {
                from: drawn,
                to: at,
                start: now,
            };
        }
        let g = *glide;
        let (pos, t) = glide_at(g, now);
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        // the tile that travels (the one starred or not any more) is lifted
        // over the others on its way; those it passes only slide over
        let lift = if t < 1.0 && (g.to - g.from).length() > width * 1.5 {
            (t * std::f32::consts::PI).sin()
        } else {
            0.0
        };
        let placed = Rect::from_min_size(self.grid_origin + pos, rect.size());
        let rect = Rect::from_center_size(placed.center(), placed.size() * (1.0 + LIFT * lift));
        let mut layer_ui;
        let pui: &egui::Ui = if lift > 0.0 {
            layer_ui = ui.new_child(
                UiBuilder::new()
                    .layer_id(egui::LayerId::new(egui::Order::Tooltip, key.with("lifted")))
                    .max_rect(rect),
            );
            layer_ui.set_clip_rect(ui.clip_rect());
            &layer_ui
        } else {
            ui
        };
        let thumb = Rect::from_min_size(rect.min, Vec2::new(width, thumb_h));
        // the picture's corners rounded a little, as the cards'
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let small = ui.visuals().widgets.inactive.corner_radius;
        let dim = if drawn { 1.0 } else { 0.35 };
        if lift > 0.0 {
            pui.painter().rect_filled(
                thumb.translate(Vec2::new(0.0, 4.0)).expand(2.0),
                radius,
                p.navy.gamma_multiply(0.45 * lift),
            );
        }
        // the family's icon, under the picture until it is drawn
        pui.painter().rect_filled(thumb, radius, p.bar);
        crate::ui::sheet::icon_at(pui, family_icon(&e.id), thumb.center(), 18.0, p.muted.gamma_multiply(dim));
        if let Some(slot) = (opts.thumb)(&e.id) {
            // the picture in its own colours, dimmed when greyed
            aurora_render::look_thumbs::paint(
                pui.painter(),
                thumb,
                slot,
                widgets::PICTURE_TINT.gamma_multiply(dim),
                f32::from(radius.nw),
            );
        }
        let used = s.looks.iter().any(|l| l.id == e.id);
        let dragging = egui::DragAndDrop::has_any_payload(ui.ctx());
        let hovered = drawn && !dragging && resp.hovered();
        // the outline and the "+" fade in and out, as everything else moves
        let tile_id = self.id(("tile_widget", e.id.as_str()));
        let lit = ui.ctx().animate_bool_with_time(tile_id, hovered, FADE);
        if lit > 0.0 {
            // inside the picture: the list's edges would cut an outline
            // around the first column and row
            pui.painter().rect_stroke(
                thumb,
                radius,
                egui::Stroke::new(2.0, p.violet.gamma_multiply(lit)),
                egui::StrokeKind::Inside,
            );
            if !full && !mine {
                let badge = Rect::from_min_size(egui::pos2(thumb.right() - 22.0, thumb.bottom() - 22.0), Vec2::splat(18.0));
                pui.painter().rect_filled(badge, small, p.violet.gamma_multiply(lit));
                crate::ui::sheet::icon_at(pui, "plus", badge.center(), 12.0, p.on_accent.gamma_multiply(lit));
            }
        }
        if used {
            let badge = Rect::from_min_size(thumb.min + Vec2::splat(4.0), Vec2::splat(16.0));
            pui.painter().rect_filled(badge, small, p.navy.gamma_multiply(0.8));
            crate::ui::sheet::icon_at(pui, "check", badge.center(), 11.0, p.on_accent);
        }
        let ink = if drawn { p.ink } else { p.muted_dim };
        // the name as the cards' (semibold), a little smaller
        let mut job = egui::text::LayoutJob::single_section(
            e.name.to_string(),
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 12.0, ink),
        );
        job.wrap = egui::text::TextWrapping {
            max_width: width,
            max_rows: 2,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let g = pui.painter().layout_job(job);
        pui.painter().galley(egui::pos2(rect.left(), thumb.bottom() + 3.0), g, ink);
        // the star and the bin show on the tile pointed at, or reached with
        // Tab (the tile, then its bin and its star, take the focus in turn)
        let focused = ui.memory(|m| {
            [resp.id, resp.id.with("bin"), resp.id.with("star")]
                .iter()
                .any(|id| m.has_focus(*id))
        });
        let near = (resp.contains_pointer() || focused) && !dragging;
        // the user's own table: its bin over the picture
        if mine && near {
            let bin = Rect::from_min_size(egui::pos2(thumb.right() - 24.0, thumb.bottom() - 24.0), Vec2::splat(20.0));
            let del = ui.interact(bin, resp.id.with("bin"), Sense::click());
            let (face, ink) = if del.hovered() {
                (p.danger, p.on_accent)
            } else {
                (p.navy.gamma_multiply(0.8), p.danger)
            };
            pui.painter().rect_filled(bin, small, face);
            crate::ui::sheet::icon_at(pui, "trash-filled", bin.center(), 13.0, ink);
            let del = del.on_hover_text(aurora_i18n::tr!("photo-lut-remove"));
            if del.clicked()
                && let Some(file) = e.id.strip_prefix(looks::LUT)
            {
                out.push(Action::RemoveLut(file.to_owned()));
                return;
            }
        }
        // its star: lit on a favourite, offered on the tile pointed at
        let starred = s.favorites.contains(&e.id);
        let shown = ui.ctx().animate_bool_with_time(tile_id.with("star"), near, FADE);
        let alpha = if starred { 1.0 } else { shown };
        // a star clicked swells and settles
        let pop = match self.pops.get(&e.id).map(|t0| ((now - t0) / POP) as f32) {
            Some(k) if k < 1.0 => {
                ui.ctx().request_repaint();
                1.0 + 0.45 * (k * std::f32::consts::PI).sin()
            }
            Some(_) => {
                self.pops.remove(&e.id);
                1.0
            }
            None => 1.0,
        };
        if alpha > 0.0 && !dragging {
            let at = Rect::from_min_size(egui::pos2(thumb.right() - 22.0, thumb.top() + 4.0), Vec2::splat(18.0));
            let star = ui.interact(at, resp.id.with("star"), Sense::click());
            let ink = if starred {
                p.amber
            } else if star.hovered() {
                p.ink
            } else {
                p.muted
            };
            pui.painter().rect_filled(at, small, p.navy.gamma_multiply(0.8 * alpha));
            crate::ui::sheet::icon_at(
                pui,
                if starred { "star-filled" } else { "star" },
                at.center(),
                12.0 * pop,
                ink.gamma_multiply(alpha),
            );
            let tip = if starred {
                aurora_i18n::tr!("photo-looks-unfavorite")
            } else {
                aurora_i18n::tr!("photo-looks-favorite")
            };
            let star = star.on_hover_text(tip);
            keep_keys(&star);
            if star.clicked() {
                looks::toggle_favorite(s.favorites, &e.id);
                self.pops.insert(e.id.clone(), now);
                return;
            }
        }
        keep_keys(&resp);
        if hovered {
            self.hover = Some(Some(e.id.clone()));
        }
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), Carried(e.id.clone()));
            self.tile_out = true;
            self.hover = None;
        }
        let tip = match (e.tip, drawn) {
            (Some(tip), true) => tip.to_owned(),
            (Some(tip), false) => format!("{tip}\n{}", aurora_i18n::tr!("photo-looks-not-in-360")),
            (None, true) => e.name.to_string(),
            (None, false) => format!("{}\n{}", e.name, aurora_i18n::tr!("photo-looks-not-in-360")),
        };
        let tip = match used {
            true => format!("{tip}\n{}", aurora_i18n::tr!("photo-looks-right-click-out")),
            false => tip,
        };
        resp = resp.on_hover_text(tip);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, drawn, used, e.name.as_ref()));
        if resp.clicked() {
            self.pick(s, &e.id);
        }
        // a right click takes an active effect out (the copy added last)
        if resp.secondary_clicked()
            && let Some(at) = s.looks.iter().rposition(|l| l.id == e.id)
        {
            self.apply(s, Edit::Remove(at));
        }
    }

    /// A tile in the hand: drawn as the card it will become, with the place
    /// it would take over the cards.
    fn carried_tile(&mut self, ui: &mut egui::Ui, p: &Palette, s: &Stack<'_>) {
        let Some(tile) = egui::DragAndDrop::payload::<Carried>(ui.ctx()) else {
            self.tile_out = false;
            return;
        };
        if !self.tile_out {
            return;
        }
        let Some(at) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let width = self.cards_rect.map_or(300.0, |r| r.width());
        let rect = Rect::from_min_size(at - Vec2::new(36.0, HEAD / 2.0), Vec2::new(width, HEAD));
        let over = self.over_own_cards(ui.ctx(), ui.layer_id(), at) && s.looks.len() < MAX_LAYERS;
        let place = over.then(|| reorder::insert_place(at.y, &self.drawn) + 1);
        let name = looks::label(Some(&tile.0));
        let radius = ui.visuals().widgets.inactive.corner_radius;
        reorder::lifted_card(ui, p, ui.make_persistent_id("looks_carried"), rect, |card| {
            let slot = Rect::from_min_size(rect.min + Vec2::new(8.0, (HEAD - 20.0) / 2.0), Vec2::splat(20.0));
            match place {
                Some(place) => {
                    card.painter().rect_filled(slot, radius, p.violet);
                    widgets::turning_number(card, slot, place, None, 1.0, p.on_accent);
                }
                None => {
                    card.painter().rect_filled(slot, radius, p.field);
                    crate::ui::sheet::icon_at(card, "plus", slot.center(), 12.0, p.muted);
                }
            }
            let left = slot.right() + 10.0;
            let font = crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(card.ctx()), 13.0, p.ink).font_id;
            let g = card.painter().layout_no_wrap(name.to_string(), font, p.ink);
            let fam = card
                .painter()
                .layout_no_wrap(family(&tile.0), egui::FontId::proportional(10.5), p.muted);
            let top = rect.center().y - (g.size().y + fam.size().y) / 2.0;
            let gh = g.size().y;
            card.painter().galley(egui::pos2(left, top), g, p.ink);
            card.painter().galley(egui::pos2(left, top + gh), fam, p.muted);
        });
        // let go anywhere but over the cards: nothing changes (egui lets
        // the payload go at the end of the frame; clearing it here would
        // take it from the other editor, the Director's or the photo
        // window's, before it sees the drop)
    }
}

/// Width of a setting's name, and of its value, in a card's settings: the
/// three columns line up in every card.
const NAME_W: f32 = 92.0;
const VALUE_W: f32 = 58.0;
const ROW_H: f32 = 22.0;

/// A card's strength and settings, in three columns: the name, the slider,
/// the value (dragged or typed).
fn settings(ui: &mut egui::Ui, p: &Palette, s: &mut Stack<'_>, i: usize) {
    let Some(layer) = s.looks.get_mut(i) else {
        return;
    };
    let id = layer.id.clone();
    value_row(
        ui,
        p,
        aurora_i18n::tr!("photo-look-strength"),
        &mut layer.strength,
        0..=100,
        0,
        " %",
        false,
    );
    let Some(aurora) = id.strip_prefix(looks::AURORA) else {
        let note = if id.starts_with(looks::LUT) || id.starts_with(looks::PACK) {
            aurora_i18n::tr!("photo-looks-lut-note")
        } else {
            aurora_i18n::tr!("photo-looks-legacy-note")
        };
        card_note(ui, p, note);
        return;
    };
    let Some(look) = aurora_render::looks::aurora_look(aurora) else {
        return;
    };
    let list = knobs::knobs(aurora);
    let defaults = knobs::defaults(list, &look.ops);
    let values = s.look_settings.entry(id.clone()).or_insert_with(|| defaults.clone());
    if values.len() != defaults.len() {
        *values = defaults.clone();
    }
    let mut at = 0;
    for knob in list {
        let width = knob.width();
        knob_row(ui, p, knob, &mut values[at..at + width], &id);
        at += width;
    }
    // what the look does, under its settings (as the others' note)
    if let Some(about) = looks::tip(&id) {
        card_note(ui, p, about);
    }
    // (their Default values is in the card's ⋯ actions)
}

/// Whether an Aurora look's settings differ from its defaults.
fn changed(s: &Stack<'_>, id: &str) -> bool {
    let Some(look) = id.strip_prefix(looks::AURORA).and_then(aurora_render::looks::aurora_look) else {
        return false;
    };
    let defaults = knobs::defaults(knobs::knobs(look.id), &look.ops);
    s.look_settings.get(id).is_some_and(|values| *values != defaults)
}

/// The line at the end of a card's settings: what the look does, or why it
/// has no other setting; small and faint, under the controls.
fn card_note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(text).size(10.5).color(p.muted_dim)).wrap());
}

/// A setting's row, cut into its three columns: the name, the control,
/// the value (each at the same place in every row and every card).
fn row_columns(ui: &mut egui::Ui) -> (Rect, Rect, Rect) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    let name = Rect::from_min_size(row.min, Vec2::new(NAME_W, ROW_H));
    let value = Rect::from_min_max(egui::pos2(row.right() - VALUE_W, row.top()), row.max);
    let control = Rect::from_min_max(
        egui::pos2(name.right() + 8.0, row.top()),
        egui::pos2(value.left() - 10.0, row.bottom()),
    );
    (name, control, value)
}

/// A setting's name in its column, cut with "…" when too long (whole in
/// its tooltip).
fn setting_name(ui: &mut egui::Ui, p: &Palette, rect: Rect, label: &str) {
    let mut job = egui::text::LayoutJob::single_section(
        label.to_owned(),
        egui::TextFormat::simple(egui::FontId::proportional(12.0), p.muted),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width());
    let galley = ui.painter().layout_job(job);
    let cut = galley.elided;
    widgets::paint_row_text(ui.painter(), galley, rect.left(), rect.center().y, p.muted);
    if cut {
        ui.interact(rect, ui.scope_id().with(("setting", label)), Sense::hover())
            .on_hover_text(label);
    }
}

/// A setting with a value: its name, a slider filling the middle, and the
/// value in its own column, dragged or typed (`parse`: in the viewer's
/// language). Returns whether it changed.
#[allow(clippy::too_many_arguments)]
fn value_row<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    decimals: usize,
    suffix: &str,
    parse: bool,
) -> bool {
    let (name, control, cell) = row_columns(ui);
    setting_name(ui, p, name, label);
    // the slider's handle stays inside its column
    ui.spacing_mut().slider_width = (control.width() - ui.spacing().interact_size.y).max(24.0);
    let slider = ui.put(
        control,
        egui::Slider::new(value, range.clone()).show_value(false).trailing_fill(true),
    );
    let span = range.end().to_f64() - range.start().to_f64();
    let mut field = egui::DragValue::new(value)
        .range(range)
        .fixed_decimals(decimals)
        .suffix(suffix)
        .speed(span / 200.0);
    if parse {
        field = field.custom_parser(aurora_i18n::parse_number);
    }
    let field = ui.put(cell, field);
    keep_keys(&slider);
    keep_keys(&field);
    slider.changed() || field.changed()
}

/// One setting of an Aurora look.
fn knob_row(ui: &mut egui::Ui, p: &Palette, knob: &Knob, values: &mut [f32], id: &str) {
    let label = aurora_i18n::text(knob.label);
    match knob.kind {
        Kind::Range { min, max } => {
            let mut percent = ((values[0] - min) / (max - min) * 100.0).clamp(0.0, 100.0);
            if value_row(ui, p, label, &mut percent, 0.0..=100.0, 0, " %", false) {
                values[0] = min + percent / 100.0 * (max - min);
            }
        }
        Kind::Meters { min, max } => {
            value_row(ui, p, label, &mut values[0], min..=max, 0, " m", false);
        }
        Kind::Ratio { min, max } => {
            value_row(ui, p, label, &mut values[0], min..=max, 2, ":1", true);
        }
        Kind::Hue => {
            value_row(ui, p, label, &mut values[0], 0.0..=360.0, 0, "°", false);
        }
        Kind::Seed => {
            let mut seed = values[0].round() as i32;
            if value_row(ui, p, label, &mut seed, 0..=99, 0, "", false) {
                values[0] = seed as f32;
            }
        }
        Kind::Color => {
            let (name, control, _) = row_columns(ui);
            setting_name(ui, p, name, label);
            // the swatch at the start of the control's column
            ui.scope_builder(
                UiBuilder::new().max_rect(control).layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    // the look's colours are in the photo's gamma-encoded levels
                    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let mut rgba = [byte(values[0]), byte(values[1]), byte(values[2]), 255];
                    let resp = widgets::color_button(ui, p, (id, knob.label, knob.op), &mut rgba, false);
                    if resp.changed() {
                        for (v, b) in values.iter_mut().zip(rgba) {
                            *v = f32::from(b) / 255.0;
                        }
                    }
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_of_a_look_have_their_own_keys() {
        let stack = [
            looks::Layer::new("Sepia"),
            looks::Layer::new("aurora:vhs"),
            looks::Layer::new("Sepia"),
        ];
        let keys = keys_of(&stack);
        assert_eq!(keys[0], ("Sepia".to_owned(), 0));
        assert_eq!(keys[2], ("Sepia".to_owned(), 1), "the second Sepia is another card");
    }

    /// The numbers taking the eyes' place: the card carried shows where it
    /// goes and the others where they make way to; a tile from the
    /// catalogue pushes the cards after its place down by one.
    #[test]
    fn the_places_shown_follow_what_is_held() {
        assert_eq!(places(4, Some((0, 2)), None), Some(vec![3, 1, 2, 4]));
        assert_eq!(places(3, None, Some(1)), Some(vec![1, 3, 4]));
        assert_eq!(places(3, None, Some(3)), Some(vec![1, 2, 3]), "dropped at the end: nobody moves");
        assert_eq!(places(3, None, None), None, "nothing held: the eyes come back");
    }

    /// The editor in a test frame, with no pictures.
    fn frame(frames: &mut crate::ui::widgets::elide_tests::Frames, ed: &mut LooksUi, s: &mut Settings, events: Vec<egui::Event>) {
        let p = crate::theme::Theme::default().skin_palette();
        let thumb = |_: &str| None;
        frames.run(events, |ui| {
            let opts = Options {
                list_height: 200.0,
                height: Some(700.0),
                compare: true,
                luts: &[],
                pano: false,
                thumb: &thumb,
            };
            ed.show(ui, &p, Stack::photo(s), opts, &mut Vec::new());
        });
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(at), button(true), button(false)]
    }

    /// A click on a card's name opens it (not only its chevron), with
    /// Replace on too, and makes it the card Replace swaps.
    #[test]
    fn a_click_on_a_cards_name_opens_it_and_chooses_it() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let mut ed = LooksUi {
            replace: true,
            selected: 1,
            ..Default::default()
        };
        let mut s = Settings {
            looks: vec![looks::Layer::new("Sepia"), looks::Layer::new("aurora:vhs")],
            ..Default::default()
        };
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        let name = ed.drawn[0].left_center() + Vec2::new(60.0, 0.0);
        frame(&mut frames, &mut ed, &mut s, click(name));
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        assert!(ed.open.contains("Sepia"), "the name opens the card");
        assert_eq!(ed.selected, 0, "and makes it the chosen one");
        assert!(ed.replace, "Replace stays on");
    }

    /// A card carried when the editor stops showing (its panel closed with
    /// a shortcut mid-drag) is let go: the release is never seen, and the
    /// catalogue would stay the take-out zone once the panel is back.
    #[test]
    fn a_card_carried_while_the_panel_closes_is_let_go() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let mut ed = LooksUi::default();
        let mut s = Settings {
            looks: vec![looks::Layer::new("Sepia"), looks::Layer::new("aurora:vhs")],
            ..Default::default()
        };
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        let at = ed.drawn[0].left_center() + Vec2::new(60.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut frames, &mut ed, &mut s, vec![egui::Event::PointerMoved(at), button(at, true)]);
        for dy in [10.0, 20.0, 30.0] {
            frame(
                &mut frames,
                &mut ed,
                &mut s,
                vec![egui::Event::PointerMoved(at + Vec2::new(0.0, dy))],
            );
        }
        assert!(ed.carrying.is_some(), "the card is carried");
        // the panel is closed: the release lands elsewhere
        let end = at + Vec2::new(0.0, 30.0);
        frames.run(vec![button(end, false)], |_| {});
        frames.run(Vec::new(), |_| {});
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        assert!(ed.carrying.is_none(), "the panel comes back with its catalogue");
        assert_eq!(s.looks.len(), 2, "and nothing was taken out");
    }

    /// A card carried down onto the catalogue's row (its title, Replace)
    /// stays in the stack when let go: only the dashed zone under the row
    /// takes it out (feedback 2026-11-21).
    #[test]
    fn only_the_dashed_zone_takes_a_card_out_not_the_catalogues_row() {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // carries the last card until its bottom edge sits `past` points
        // below the zone's top, then lets it go
        let drop_at = |past: f32| {
            let mut frames = crate::ui::widgets::elide_tests::Frames::new();
            let mut ed = LooksUi::default();
            let mut s = Settings {
                looks: vec![looks::Layer::new("Sepia"), looks::Layer::new("aurora:vhs")],
                ..Default::default()
            };
            frame(&mut frames, &mut ed, &mut s, Vec::new());
            frame(&mut frames, &mut ed, &mut s, Vec::new());
            let mut at = ed.drawn[1].left_center() + Vec2::new(60.0, 0.0);
            frame(&mut frames, &mut ed, &mut s, vec![egui::Event::PointerMoved(at), button(at, true)]);
            for _ in 0..3 {
                at.y += 10.0;
                frame(&mut frames, &mut ed, &mut s, vec![egui::Event::PointerMoved(at)]);
            }
            assert!(ed.carrying.is_some(), "the card is carried");
            // the dashed box from its own height (the catalogue's, under
            // the row), not from the area tested
            let zone = ed.zone_rect.expect("the zone shows while a card is carried");
            let spacing = egui::Style::default().spacing.item_spacing.y;
            let box_top = zone.bottom() - (ed.catalogue_height - spacing).max(24.0);
            for _ in 0..12 {
                at.y += box_top + past - ed.drawn[1].bottom();
                frame(&mut frames, &mut ed, &mut s, vec![egui::Event::PointerMoved(at)]);
            }
            frame(&mut frames, &mut ed, &mut s, vec![button(at, false)]);
            frame(&mut frames, &mut ed, &mut s, Vec::new());
            s.looks.len()
        };
        assert_eq!(drop_at(-4.0), 2, "over the catalogue's row only: kept");
        assert_eq!(drop_at(6.0), 1, "into the dashed zone: taken out");
    }

    /// In a window that shows only part of the editor (the World filters
    /// window, the photo window's scrolled section), the take-out zone ends
    /// at the window's edge: a card let go below it, over the world, stays.
    #[test]
    fn a_card_let_go_past_the_windows_edge_stays() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let p = crate::theme::Theme::default().skin_palette();
        let thumb = |_: &str| None;
        let mut ed = LooksUi::default();
        let mut s = Settings {
            looks: vec![looks::Layer::new("Sepia"), looks::Layer::new("aurora:vhs")],
            ..Default::default()
        };
        let edge = 260.0;
        let mut frame = |frames: &mut crate::ui::widgets::elide_tests::Frames, ed: &mut LooksUi, events| {
            frames.run(events, |ui| {
                egui::ScrollArea::vertical().max_height(edge).show(ui, |ui| {
                    let opts = Options {
                        list_height: 200.0,
                        height: None,
                        compare: false,
                        luts: &[],
                        pano: false,
                        thumb: &thumb,
                    };
                    ed.show(ui, &p, Stack::photo(&mut s), opts, &mut Vec::new());
                });
            });
        };
        frame(&mut frames, &mut ed, Vec::new());
        frame(&mut frames, &mut ed, Vec::new());
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut at = ed.drawn[1].left_center() + Vec2::new(60.0, 0.0);
        frame(&mut frames, &mut ed, vec![egui::Event::PointerMoved(at), button(at, true)]);
        for _ in 0..3 {
            at.y += 10.0;
            frame(&mut frames, &mut ed, vec![egui::Event::PointerMoved(at)]);
        }
        assert!(ed.carrying.is_some(), "the card is carried");
        // well below the window's edge, where the zone would go on unseen
        for _ in 0..12 {
            at.y += edge + 60.0 - ed.drawn[1].top();
            frame(&mut frames, &mut ed, vec![egui::Event::PointerMoved(at)]);
        }
        assert!(ed.drawn[1].top() > edge + 30.0, "the card is past the edge");
        frame(&mut frames, &mut ed, vec![button(at, false)]);
        frame(&mut frames, &mut ed, Vec::new());
        assert_eq!(s.looks.len(), 2, "nothing taken out");
    }

    /// Two editors in windows that overlap (the photo window and the World
    /// filters window): a tile let go over both goes to the cards seen
    /// there, the window on top, not to the editor drawn first.
    #[test]
    fn a_tile_goes_to_the_window_on_top() {
        type Frames = crate::ui::widgets::elide_tests::Frames;
        // the first window drawn is under the second
        fn run(frames: &mut Frames, eds: [(&mut LooksUi, &mut Settings); 2], events: Vec<egui::Event>) {
            let p = crate::theme::Theme::default().skin_palette();
            let thumb = |_: &str| None;
            let [(ed_under, s_under), (ed_over, s_over)] = eds;
            frames.run(events, |ui| {
                let ctx = ui.ctx().clone();
                let windows = [
                    ("under", 0.0, &mut *ed_under, &mut *s_under),
                    ("over", 40.0, &mut *ed_over, &mut *s_over),
                ];
                for (name, x, ed, s) in windows {
                    egui::Area::new(egui::Id::unique(name))
                        .fixed_pos(egui::pos2(x, 0.0))
                        .show(&ctx, |ui| {
                            ui.set_width(360.0);
                            let opts = Options {
                                list_height: 200.0,
                                height: Some(700.0),
                                compare: false,
                                luts: &[],
                                pano: false,
                                thumb: &thumb,
                            };
                            ed.show(ui, &p, Stack::photo(s), opts, &mut Vec::new());
                        });
                }
            });
        }
        let mut frames = Frames::new();
        let (mut under, mut over) = (LooksUi::default(), LooksUi::default());
        let two = || Settings {
            looks: vec![looks::Layer::new("Sepia"), looks::Layer::new("aurora:vhs")],
            ..Default::default()
        };
        let (mut photo, mut world) = (two(), two());
        macro_rules! frame {
            ($events:expr) => {
                run(&mut frames, [(&mut under, &mut photo), (&mut over, &mut world)], $events)
            };
        }
        frame!(Vec::new());
        frame!(Vec::new());
        // over the first card of both editors
        let at = egui::pos2(200.0, over.drawn[0].center().y);
        assert!(under.cards_rect.is_some_and(|r| r.contains(at)), "the windows overlap there");
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // a tile taken (away from the cards, as from a catalogue), carried
        // over them, let go
        let start = egui::pos2(1000.0, 800.0);
        frame!(vec![egui::Event::PointerMoved(start), button(start, true)]);
        egui::DragAndDrop::set_payload(&frames.ctx, Carried("Negative".into()));
        frame!(vec![egui::Event::PointerMoved(at)]);
        frame!(vec![egui::Event::PointerMoved(at + Vec2::new(0.0, 2.0))]);
        frame!(vec![button(at, false)]);
        frame!(Vec::new());
        assert_eq!(photo.looks.len(), 2, "the window underneath takes nothing");
        assert_eq!(world.looks.len(), 3, "the window on top takes the tile");
    }

    /// Replace stays on with no effect: the first click then adds.
    #[test]
    fn replace_stays_on_and_adds_the_first_effect() {
        let mut ed = LooksUi {
            replace: true,
            ..Default::default()
        };
        let mut s = Settings::default();
        assert!(ed.pick(&mut Stack::photo(&mut s), "Sepia"));
        assert_eq!(s.looks.len(), 1);
        assert!(ed.pick(&mut Stack::photo(&mut s), "Negative"));
        assert_eq!(s.looks.len(), 1, "then it swaps");
        assert_eq!(s.looks[0].id, "Negative");
        // the editor drawn with no effect leaves it on
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        s.looks.clear();
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        assert!(ed.replace);
    }

    fn right_click(at: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(at), button(true), button(false)]
    }

    /// A right click on an active card takes it out.
    #[test]
    fn a_right_click_on_a_card_takes_it_out() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let mut ed = LooksUi::default();
        let mut s = Settings {
            looks: vec![looks::Layer::new("Sepia"), looks::Layer::new("aurora:vhs")],
            ..Default::default()
        };
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        let name = ed.drawn[0].left_center() + Vec2::new(60.0, 0.0);
        frame(&mut frames, &mut ed, &mut s, right_click(name));
        frame(&mut frames, &mut ed, &mut s, Vec::new());
        let ids: Vec<&str> = s.looks.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["aurora:vhs"]);
    }

    /// Default values (in the ⋯ actions) puts an Aurora look's settings
    /// back, and is offered once one is changed.
    #[test]
    fn default_values_put_a_looks_settings_back() {
        let mut ed = LooksUi::default();
        let mut s = Settings {
            looks: vec![looks::Layer::new("aurora:glitch")],
            ..Default::default()
        };
        assert!(!changed(&Stack::photo(&mut s), "aurora:glitch"), "nothing changed yet");
        s.look_settings.insert("aurora:glitch".into(), vec![0.9, 0.9, 1.0]);
        assert!(changed(&Stack::photo(&mut s), "aurora:glitch"));
        ed.apply(&mut Stack::photo(&mut s), Edit::Reset("aurora:glitch".into()));
        assert!(!changed(&Stack::photo(&mut s), "aurora:glitch"));
        assert!(!changed(&Stack::photo(&mut s), "Sepia"), "a Legacy filter has no settings");
    }

    /// A glide starts where the tile was, settles a hair past its place,
    /// and lands on it.
    #[test]
    fn a_tile_glides_like_a_spring() {
        let g = Glide {
            from: Vec2::ZERO,
            to: Vec2::new(100.0, 0.0),
            start: 0.0,
        };
        assert_eq!(glide_at(g, 0.0).0, Vec2::ZERO);
        let (end, t) = glide_at(g, GLIDE);
        assert!((end.x - 100.0).abs() < 0.01 && t == 1.0, "lands on its place");
        let peak = (1..100).map(|k| glide_at(g, GLIDE * k as f64 / 100.0).0.x).fold(0.0, f32::max);
        assert!(peak > 100.5 && peak < 110.0, "a light overshoot: {peak}");
        assert!(glide_at(g, GLIDE * 0.25).0.x > 50.0, "fast at first");
    }

    #[test]
    fn replace_swaps_the_chosen_card_and_adding_is_the_default() {
        let mut ui = LooksUi::default();
        let mut s = Settings::default();
        assert!(ui.pick(&mut Stack::photo(&mut s), "Sepia"));
        assert!(ui.pick(&mut Stack::photo(&mut s), "aurora:vhs"));
        assert_eq!(s.looks.len(), 2, "a click adds");
        ui.replace = true;
        ui.selected = 0;
        assert!(ui.pick(&mut Stack::photo(&mut s), "Negative"));
        let ids: Vec<&str> = s.looks.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["Negative", "aurora:vhs"], "Replace swaps the chosen card");
        assert_eq!(ui.replace_target(0), None, "nothing to replace in an empty stack");
        // full: a click adds nothing until Replace is on
        ui.replace = false;
        while s.looks.len() < MAX_LAYERS {
            assert!(ui.pick(&mut Stack::photo(&mut s), "Sepia"));
        }
        assert!(!ui.pick(&mut Stack::photo(&mut s), "Antique"));
        ui.replace = true;
        ui.selected = 3;
        assert!(ui.pick(&mut Stack::photo(&mut s), "Antique"));
        assert_eq!(s.looks[3].id, "Antique");
    }
}
