//! The full-frame world map (Preferences › Aurora › World map, the
//! default): the map fills the window and everything else floats over it,
//! in the panels of the theme (`widgets::overlay_frame`: the map is behind
//! them, not the world, so no glass). Top left, one search for the regions,
//! the landmarks, the online friends and the teleport history, with its
//! suggestions grouped, and under it a drawer with the landmarks by folder,
//! the history and the friends; top right, the layers as pills; bottom
//! left, the card of the selected place; bottom right, the zoom, our
//! position and home. Every control of Firestorm's side panel
//! (`WorldMapUi::side_panel`, the legacy layout) has its place here. The
//! panels fade while the map is dragged, and those of the last frame keep
//! the map from taking their clicks.

use super::{DRAW_SIMINFO_THRESHOLD, ItemKind, MapPictures, PAN_TAU_CENTER, WorldMapAction, WorldMapUi, ZOOM_MAX, ZOOM_MIN, item_icon};
use crate::settings::MapSettings;
use crate::theme::Palette;
use crate::ui::map_tiles::{MapTiles, scale_to_level};
use crate::ui::minimap::{MapCamera, to_global};
use crate::ui::widgets::{self, ActionKind};
use crate::world::World;
use crate::world::inventory::{FetchState, Inventory};
use crate::world::landmarks::{Landmarks, Resolved};
use crate::world::place_details::Source;
use crate::world::social::rights;
use crate::world::tphistory::{self, HistoryItem};
use crate::world::worldmap::location_label;
use aurora_net::map_item;
use egui::{Align, Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Room between the panels and the edges of the map, and between panels.
const EDGE: f32 = 12.0;
const GAP: f32 = 8.0;
/// Inner margin of the panels.
const PAD: i8 = 8;
/// The search and its drawer.
const SEARCH_W: f32 = 330.0;
/// The place card, wide enough for its five buttons on one line.
const CARD_W: f32 = 440.0;
/// Suggestions of each group.
const SUGGESTIONS: usize = 4;
/// The regions are asked once the text rests this long (seconds): one
/// MapNameRequest per pause in the typing, not one per key.
const REGION_ASK_DELAY: f64 = 0.35;
/// Suggestions rebuilt at least this often (people counts arriving).
const SUGGEST_REFRESH: f64 = 0.5;
const SUGGEST_ROW: f32 = 26.0;
/// Lines of the drawer: folder and section heads, landmarks, the others.
const ROW_HEAD: f32 = 24.0;
const ROW_CARD: f32 = 46.0;
const ROW_LINE: f32 = 32.0;
const ROW_GAP: f32 = 4.0;
const CARD_THUMB: Vec2 = Vec2::new(74.0, 56.0);
const ROW_THUMB: Vec2 = Vec2::new(44.0, 32.0);
const PILL_H: f32 = 24.0;
const BUTTON_H: f32 = 26.0;
const NAV_BUTTON: f32 = 28.0;
/// Widest row of layer pills.
const LAYERS_MAX_W: f32 = 520.0;
/// One click of the zoom buttons: twice or half the pixels per region.
const ZOOM_STEP: f32 = 1.0;
const ASSET_LANDMARK: i32 = 3;
const FOLDER_LANDMARKS: i32 = 3;
const FOLDER_TRASH: i32 = 14;
const FOLDER_FAVORITES: i32 = 23;

fn search_id() -> egui::Id {
    egui::Id::unique("worldmap-search")
}

/// A landmark placed on the map.
pub(super) struct Pin<'a> {
    pub x: f64,
    pub y: f64,
    pub name: &'a str,
    /// Its line in `LandmarkList::items`.
    index: usize,
}

/// One landmark of the inventory.
struct Lm {
    item: Uuid,
    asset: Uuid,
    name: String,
    lower: String,
    thumbnail: Uuid,
}

/// A folder holding landmarks, and the range of its landmarks in `items`.
struct LmFolder {
    id: Uuid,
    name: String,
    items: std::ops::Range<usize>,
}

/// The landmarks of our inventory out of the trash
/// (LLFloaterWorldMap::buildLandmarkIDLists), by folder: Favorites, then
/// Landmarks, then the other folders by name; rebuilt when the inventory
/// changes.
#[derive(Default)]
pub(super) struct LandmarkList {
    generation: Option<u64>,
    items: Vec<Lm>,
    folders: Vec<LmFolder>,
}

impl LandmarkList {
    /// True when it was rebuilt.
    fn refresh(&mut self, inv: &Inventory) -> bool {
        if self.generation == Some(inv.generation) {
            return false;
        }
        self.generation = Some(inv.generation);
        let mut by_folder: HashMap<Uuid, Vec<&aurora_net::inventory::InvItem>> = HashMap::new();
        for it in inv.items.values() {
            if it.asset_type != ASSET_LANDMARK || it.asset_id.is_nil() {
                continue;
            }
            let Some(f) = inv.folders.get(&it.parent) else {
                continue;
            };
            if f.library || inv.in_folder_type(it.id, FOLDER_TRASH) {
                continue;
            }
            by_folder.entry(it.parent).or_default().push(it);
        }
        let rank = |id: &Uuid| match inv.folders.get(id).map(|f| f.info.type_default) {
            Some(FOLDER_FAVORITES) => 0,
            Some(FOLDER_LANDMARKS) => 1,
            _ => 2,
        };
        let name_of = |id: &Uuid| inv.folders.get(id).map(|f| f.info.name.clone()).unwrap_or_default();
        let mut folders: Vec<Uuid> = by_folder.keys().copied().collect();
        folders.sort_by_key(|id| (rank(id), name_of(id).to_lowercase(), *id));
        self.items.clear();
        self.folders.clear();
        for id in folders {
            let mut list = by_folder.remove(&id).unwrap_or_default();
            list.sort_by_key(|i| (i.name.to_lowercase(), i.id));
            let start = self.items.len();
            self.items.extend(list.into_iter().map(|i| Lm {
                item: i.id,
                asset: i.asset_id,
                name: i.name.clone(),
                lower: i.name.to_lowercase(),
                thumbnail: i.thumbnail,
            }));
            self.folders.push(LmFolder {
                id,
                name: name_of(&id),
                items: start..self.items.len(),
            });
        }
        true
    }

    /// The landmarks whose place is known, asking the others
    /// (LLLandmarkList::getAsset, then their region's handle).
    fn pins<'a>(&'a self, landmarks: &mut Landmarks) -> Vec<Pin<'a>> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, l)| match landmarks.resolve(l.asset) {
                Resolved::Ready { global, .. } => Some(Pin {
                    x: global.x,
                    y: global.y,
                    name: &l.name,
                    index,
                }),
                _ => None,
            })
            .collect()
    }
}

/// The tabs of the drawer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Drawer {
    Landmarks,
    History,
    Friends,
}

/// The groups of suggestions, in their order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Group {
    Regions,
    Landmarks,
    Friends,
    History,
}

/// What a suggestion or a line of the drawer selects.
#[derive(Clone, Debug, PartialEq)]
enum Pick {
    Region((u32, u32)),
    /// A line of `LandmarkList::items`.
    Landmark(usize),
    Friend(Uuid, String),
    /// An entry of the saved teleport history.
    History(usize),
}

#[derive(Clone, Debug)]
struct Suggestion {
    group: Group,
    pick: Pick,
    label: String,
    /// Grey, at the right: the people of a region, where a landmark or a
    /// friend is.
    detail: String,
    rating: Option<u8>,
}

/// What the card is about; a card closed with its cross stays closed for
/// that selection only.
#[derive(Clone, Debug, PartialEq)]
enum Selection {
    Landmark(Uuid),
    Avatar(Uuid),
    Spot(i64, i64),
}

/// The floating panels' state.
#[derive(Default)]
pub(super) struct Overlay {
    query: String,
    /// The suggestions show (typing, focus); a touch of the map closes them.
    suggest: bool,
    highlight: usize,
    /// When the text last changed, and the text whose regions were asked.
    typed_at: f64,
    asked: String,
    /// The suggestions, what they were built for and when.
    suggestions: Vec<Suggestion>,
    built_for: Option<(String, usize, u64, usize, usize)>,
    built_at: f64,
    drawer: Option<Drawer>,
    closed_folders: HashSet<Uuid>,
    card_closed: Option<Selection>,
    /// Where the selected landmark was tracked, once its asset was read.
    landmark_at: Option<(f64, f64)>,
    /// Each panel's size last frame (they are placed from it), and their
    /// rectangles, which keep the map from taking their clicks.
    sizes: HashMap<&'static str, Vec2>,
    rects: Vec<(&'static str, Rect)>,
    landmarks: LandmarkList,
}

/// A panel being filled: its frame's content is `frame.content_ui`.
struct Panel {
    key: &'static str,
    child: egui::Ui,
    frame: egui::containers::frame::Prepared,
}

impl Overlay {
    pub(super) fn demo_search(&mut self, text: &str) {
        self.query = text.to_owned();
        self.asked = text.trim().to_lowercase();
        self.suggest = true;
        self.drawer = None;
    }

    pub(super) fn demo_drawer(&mut self, tab: &str) {
        self.drawer = Some(match tab {
            "history" | "historique" => Drawer::History,
            "friends" | "amis" => Drawer::Friends,
            _ => Drawer::Landmarks,
        });
    }

    /// Open a panel at `anchor`, on the side `align` says (its size is the
    /// last frame's), `width` wide or as wide as its content.
    #[allow(clippy::too_many_arguments)]
    fn open(&self, ui: &mut egui::Ui, p: &Palette, key: &'static str, anchor: Pos2, align: Align2, width: Option<f32>, fade: f32) -> Panel {
        let last = self.sizes.get(key).copied().unwrap_or(Vec2::new(width.unwrap_or(0.0), 0.0));
        let x = match align.x() {
            Align::Min => anchor.x,
            Align::Center => anchor.x - last.x / 2.0,
            Align::Max => anchor.x - last.x,
        };
        let y = match align.y() {
            Align::Min => anchor.y,
            Align::Center => anchor.y - last.y / 2.0,
            Align::Max => anchor.y - last.y,
        };
        let room = ui.max_rect();
        let w = width.unwrap_or(room.width());
        let max = Rect::from_min_max(pos2(x, y), pos2(x + w, room.bottom().max(y + 1.0)));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("worldmap-panel", key))
                .max_rect(max)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        child.multiply_opacity(fade);
        let mut frame = widgets::overlay_frame(ui.ctx(), p)
            .inner_margin(egui::Margin::same(PAD))
            .begin(&mut child);
        if let Some(w) = width {
            frame.content_ui.set_width(w - 2.0 * f32::from(PAD) - 2.0);
        }
        Panel { key, child, frame }
    }

    fn close(&mut self, panel: Panel) -> Rect {
        let Panel { key, mut child, frame } = panel;
        let rect = frame.end(&mut child).rect;
        if self.sizes.insert(key, rect.size()) != Some(rect.size()) {
            child.ctx().request_repaint();
        }
        self.rects.push((key, rect));
        rect
    }
}

/// The suggestions for `query` (lowercase), at most `SUGGESTIONS` a group:
/// the known regions whose name holds it (those starting with it first),
/// the landmarks, the online friends who let us see them on the map
/// (Firestorm's friends of the map), the teleport history.
fn build_suggestions(world: &World, list: &LandmarkList, query: &str) -> Vec<Suggestion> {
    let q = query.trim().to_lowercase();
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    let me = {
        let g = to_global(world, world.agent.position);
        world.map.sim_at_global(g.0, g.1).map(|(k, _)| k)
    };
    let mut regions: Vec<(&(u32, u32), &crate::world::worldmap::SimInfo, String)> = world
        .map
        .sims
        .iter()
        .filter_map(|(k, s)| {
            let lower = s.name.to_lowercase();
            lower.contains(&q).then_some((k, s, lower))
        })
        .collect();
    regions.sort_by(|a, b| (!a.2.starts_with(&q), &a.2).cmp(&(!b.2.starts_with(&q), &b.2)));
    for (key, s, _) in regions.into_iter().take(SUGGESTIONS) {
        let detail = if s.is_down() {
            s.access_label().to_owned()
        } else {
            match world
                .map
                .agents
                .get(key)
                .map(|v| v.iter().map(|a| a.2).sum::<i32>() + i32::from(me == Some(*key)))
            {
                Some(n) if n > 0 => aurora_i18n::tr!("person-count", count = n).to_string(),
                _ => String::new(),
            }
        };
        out.push(Suggestion {
            group: Group::Regions,
            pick: Pick::Region(*key),
            label: s.name.clone(),
            detail,
            rating: (!s.is_down()).then_some(s.access),
        });
    }
    let folder_of = |i: usize| list.folders.iter().find(|f| f.items.contains(&i)).map_or("", |f| f.name.as_str());
    for (i, l) in list
        .items
        .iter()
        .enumerate()
        .filter(|(_, l)| l.lower.contains(&q))
        .take(SUGGESTIONS)
    {
        let place = landmark_region(world, l.asset);
        out.push(Suggestion {
            group: Group::Landmarks,
            pick: Pick::Landmark(i),
            label: l.name.clone(),
            detail: place.as_ref().map_or_else(|| folder_of(i).to_owned(), |(n, _)| n.clone()),
            rating: place.map(|(_, a)| a),
        });
    }
    let mut friends: Vec<(Uuid, String)> = world
        .social
        .friends
        .iter()
        .filter(|f| f.online && f.rights_has & rights::MAP_LOCATION != 0)
        .map(|f| (f.id, world.social.name_of(&f.id)))
        .filter(|(_, n)| n.to_lowercase().contains(&q))
        .collect();
    friends.sort_by_key(|f| f.1.to_lowercase());
    for (id, name) in friends.into_iter().take(SUGGESTIONS) {
        out.push(Suggestion {
            group: Group::Friends,
            detail: followed_region(world, id).unwrap_or_default(),
            pick: Pick::Friend(id, name.clone()),
            label: name,
            rating: None,
        });
    }
    for (i, h) in world
        .tp_storage
        .items
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, h)| tphistory::history_matches(&h.title, &q))
        .take(SUGGESTIONS)
    {
        out.push(Suggestion {
            group: Group::History,
            pick: Pick::History(i),
            label: h.title.clone(),
            detail: String::new(),
            rating: world.map.sim_at_global(h.global.x, h.global.y).map(|(_, s)| s.access),
        });
    }
    out
}

/// The region a landmark points into, once known: its name and rating.
fn landmark_region(world: &World, asset: Uuid) -> Option<(String, u8)> {
    match world.landmarks.peek(asset) {
        Resolved::Ready { global, .. } => world.map.sim_at_global(global.x, global.y).map(|(_, s)| (s.name.clone(), s.access)),
        _ => None,
    }
}

/// The region of the friend the map follows, once the grid answered.
fn followed_region(world: &World, id: Uuid) -> Option<String> {
    world.map.avatar.as_ref().filter(|a| a.id == id && a.found)?;
    let t = world.map.track.as_ref()?;
    world.map.sim_at_global(t.x, t.y).map(|(_, s)| s.name.clone())
}

/// "350 m away", "1.2 km away".
fn distance_text(meters: f64) -> String {
    let d = if meters < 1000.0 {
        format!("{:.0} m", meters)
    } else {
        aurora_i18n::number_text(&format!("{:.1} km", meters / 1000.0))
    };
    aurora_i18n::tr!("ui-worldmap-distance", distance = d).to_string()
}

/// Map events (PG, mature, adult) inside a region.
fn events_in(world: &World, key: (u32, u32), s: &crate::world::worldmap::SimInfo) -> usize {
    let (x0, y0) = (key.0 * 256, key.1 * 256);
    [map_item::PG_EVENT, map_item::MATURE_EVENT, map_item::ADULT_EVENT]
        .iter()
        .filter_map(|t| world.map.items.get(t))
        .flatten()
        .filter(|it| it.x >= x0 && it.y >= y0 && it.x < x0 + s.size_x && it.y < y0 + s.size_y)
        .count()
}

/// A texture laid over `rect`, cropped to fill it.
fn cover(painter: &egui::Painter, t: &egui::TextureHandle, rect: Rect) {
    let [w, h] = t.size();
    let (tw, th) = (w.max(1) as f32, h.max(1) as f32);
    let k = (rect.width() / tw).max(rect.height() / th);
    let (u, v) = (rect.width() / (tw * k), rect.height() / (th * k));
    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(u, v));
    painter.image(t.id(), rect, uv, widgets::PICTURE_TINT);
}

/// A picture of a place: its own (an inventory thumbnail) or its region's
/// map tile, else the placeholder of the pictures.
fn place_picture(ui: &mut egui::Ui, p: &Palette, rect: Rect, tex: Option<&egui::TextureHandle>) {
    let radius = ui.visuals().window_corner_radius;
    match tex {
        Some(t) => cover(&ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())), t, rect),
        None => {
            ui.painter().rect_filled(rect, radius, p.raised);
            icon(
                ui.painter(),
                "mountain-filled",
                rect.center(),
                rect.height().min(22.0) * 0.7,
                p.muted_dim,
            );
        }
    }
}

fn icon(painter: &egui::Painter, name: &str, center: Pos2, size: f32, tint: Color32) {
    if let Some(t) = crate::ui::icons::global(name) {
        painter.image(
            t.id(),
            Rect::from_center_size(center, Vec2::splat(size)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// The face of a layer or tab pill.
struct PillFace<'a> {
    icon: &'a str,
    tint: Color32,
    label: &'a str,
    /// Maturity names whose badges follow the label (the events' ratings).
    badges: Vec<&'static str>,
    on: bool,
    enabled: bool,
}

impl PillFace<'_> {
    fn width(&self, ui: &egui::Ui) -> f32 {
        let text = if self.label.is_empty() {
            0.0
        } else {
            5.0 + ui
                .painter()
                .layout_no_wrap(self.label.to_owned(), pill_font(ui), Color32::PLACEHOLDER)
                .size()
                .x
        };
        let badges = self.badges.len() as f32 * 15.0 + if self.badges.is_empty() { 0.0 } else { 3.0 };
        9.0 + 14.0 + text + badges + 9.0
    }

    fn show(&self, ui: &mut egui::Ui, p: &Palette, rect: Rect, id: egui::Id) -> egui::Response {
        let resp = ui.interact(rect, id, if self.enabled { Sense::click() } else { Sense::hover() });
        let hot = self.enabled && resp.hovered();
        let radius = ui.visuals().window_corner_radius;
        let fade = if self.enabled { 1.0 } else { 0.45 };
        let fill = match (self.on, hot) {
            (true, false) => p.violet.gamma_multiply(0.28),
            (true, true) => p.violet.gamma_multiply(0.4),
            (false, true) => p.raised,
            (false, false) => p.bar,
        };
        let painter = ui.painter();
        painter.rect_filled(rect, radius, fill.gamma_multiply(fade));
        if self.on {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, p.violet_light.gamma_multiply(0.5 * fade)),
                egui::StrokeKind::Inside,
            );
        }
        let ink = if self.on { p.ink } else { p.muted }.gamma_multiply(fade);
        let tint = if self.on { self.tint } else { p.muted }.gamma_multiply(fade);
        let mut x = rect.left() + 9.0;
        icon(painter, self.icon, pos2(x + 7.0, rect.center().y), 14.0, tint);
        x += 14.0;
        if !self.label.is_empty() {
            let galley = painter.layout_no_wrap(self.label.to_owned(), pill_font(ui), ink);
            x += 5.0;
            let w = galley.size().x;
            let at = crate::ui::widgets::row_text_origin(painter, &galley, x, rect.center().y);
            painter.galley(at, galley, ink);
            x += w;
        }
        if !self.badges.is_empty() {
            x += 3.0;
            for b in &self.badges {
                let r = Rect::from_min_size(pos2(x + 2.0, rect.center().y - 6.5), Vec2::splat(13.0));
                widgets::paint_maturity_badge(painter, p, b, r, widgets::inner_radius(ui.visuals()));
                x += 15.0;
            }
        }
        if hot {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, self.enabled, self.on, self.label));
        resp
    }
}

fn pill_font(ui: &egui::Ui) -> egui::FontId {
    egui::FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()))
}

/// The lines of the drawer.
enum Row {
    Folder { index: usize, open: bool },
    Landmark(usize),
    Section(usize),
    History(usize),
    Friend { id: Uuid, name: String, mappable: bool },
    Note(String),
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Row::Folder { .. } | Row::Section(_) => ROW_HEAD,
            Row::Landmark(_) => ROW_CARD,
            Row::History(_) | Row::Friend { .. } | Row::Note(_) => ROW_LINE,
        }
    }
}

/// What a click in a panel asks, carried out once the panel is closed.
enum Ask {
    Pick(Pick),
    Teleport(Pick),
    ToggleFolder(Uuid),
}

impl WorldMapUi {
    /// The whole window of the full-frame layout: the map, then the panels
    /// over it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn full_frame(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        tiles: &mut MapTiles,
        cam: &MapCamera,
        opts: &mut MapSettings,
        pictures: &MapPictures,
        actions: &mut Vec<WorldMapAction>,
    ) {
        let now = ui.input(|i| i.time);
        let area = ui.available_rect_before_wrap();
        let size = vec2(area.width().max(160.0).floor(), area.height().max(160.0).floor());
        let map = Rect::from_min_size(area.min, size);
        if self.ov.landmarks.refresh(&world.inventory) || (self.ov.landmarks.items.is_empty() && self.want_folders.is_empty()) {
            self.want_landmark_folders(&world.inventory);
        }
        let list = std::mem::take(&mut self.ov.landmarks);
        let input = {
            let pins = if opts.world_landmarks {
                list.pins(&mut world.landmarks)
            } else {
                Vec::new()
            };
            let input = self.map(ui, p, world, tiles, cam, opts, size, Some(&pins));
            (input.touched, input.dragging, input.pin.map(|(i, double)| (pins[i].index, double)))
        };
        self.ov.landmarks = list;
        let (touched, dragging, pin) = input;
        if let Some((index, double)) = pin {
            self.select_landmark(world, index);
            if double
                && opts.world_double_click_tp
                && let Some(l) = self.ov.landmarks.items.get(index)
            {
                actions.push(WorldMapAction::TeleportLandmark(l.asset));
            }
        }
        if touched {
            self.ov.suggest = false;
            ui.memory_mut(|m| m.surrender_focus(search_id()));
        }
        self.follow_landmark(world, cam);
        // the panels of the last frame keep the map's clicks and drags
        for (key, r) in std::mem::take(&mut self.ov.rects) {
            ui.interact(r, egui::Id::unique(("worldmap-panel-blocker", key)), Sense::click_and_drag());
        }
        let fade = if dragging { 0.55 } else { 1.0 };
        let card = self.place_card(ui, p, world, tiles, cam, pictures, map, fade, actions);
        self.nav(ui, p, map, fade, actions);
        self.layers(ui, p, opts, map, fade);
        if let Some(tab) = self.ov.drawer {
            let top = map.top() + EDGE + self.ov.sizes.get("search").map_or(40.0, |s| s.y) + GAP;
            let bottom = card.map_or(map.bottom() - EDGE, |c| c.top() - GAP);
            self.drawer(ui, p, world, tiles, cam, pictures, map, tab, top, bottom, fade, actions);
        }
        self.search_panel(ui, p, world, cam, pictures, map, fade, now);
    }

    /// Ask the folders of Landmarks and Favorites not fetched yet.
    fn want_landmark_folders(&mut self, inv: &Inventory) {
        fn walk(inv: &Inventory, folder: Uuid, out: &mut HashSet<Uuid>, depth: usize) {
            let Some(f) = inv.folders.get(&folder) else {
                return;
            };
            if matches!(f.state, FetchState::Unknown | FetchState::Failed) {
                out.insert(folder);
            }
            if depth < 32 {
                for c in &f.children {
                    walk(inv, *c, out, depth + 1);
                }
            }
        }
        for kind in [FOLDER_LANDMARKS, FOLDER_FAVORITES] {
            if let Some(root) = crate::ui::places::system_folder(inv, kind) {
                walk(inv, root, &mut self.want_folders, 0);
            }
        }
    }

    /// Select a landmark of the list: the card names it, and the map tracks
    /// its place once its asset is read (`follow_landmark`).
    fn select_landmark(&mut self, world: &mut World, index: usize) {
        let Some(l) = self.ov.landmarks.items.get(index) else {
            return;
        };
        self.landmark = Some((l.item, l.asset, l.name.clone()));
        self.ov.landmark_at = None;
        world.map.clear_track();
        world.landmarks.resolve(l.asset);
    }

    /// The selected landmark tracked where it points once known; dropped
    /// when something else is tracked since.
    fn follow_landmark(&mut self, world: &mut World, cam: &MapCamera) {
        let Some((_, asset, name)) = self.landmark.clone() else {
            self.ov.landmark_at = None;
            return;
        };
        match self.ov.landmark_at {
            None if world.map.track.is_some() || world.map.avatar.is_some() => {
                self.landmark = None;
            }
            None => {
                if let Resolved::Ready { global, .. } = world.landmarks.resolve(asset) {
                    world.map.track_location(global.x, global.y, global.z as f32, false);
                    if let Some(t) = world.map.track.as_mut() {
                        t.label = name;
                    }
                    self.ov.landmark_at = Some((global.x, global.y));
                    self.center_on(world, cam, global.x, global.y);
                }
            }
            Some((x, y)) => {
                let moved = world.map.avatar.is_some()
                    || world
                        .map
                        .track
                        .as_ref()
                        .is_none_or(|t| (t.x - x).abs() > 0.01 || (t.y - y).abs() > 0.01);
                if moved {
                    self.landmark = None;
                    self.ov.landmark_at = None;
                }
            }
        }
    }

    fn selection(&self, world: &World) -> Option<Selection> {
        if let Some((item, ..)) = &self.landmark {
            Some(Selection::Landmark(*item))
        } else if let Some(a) = &world.map.avatar {
            Some(Selection::Avatar(a.id))
        } else {
            world
                .map
                .track
                .as_ref()
                .map(|t| Selection::Spot((t.x * 10.0) as i64, (t.y * 10.0) as i64))
        }
    }

    /// Carry out a suggestion or a line of the drawer.
    fn pick(&mut self, world: &mut World, cam: &MapCamera, pick: Pick) {
        match pick {
            Pick::Region(key) => self.select_region(world, cam, key),
            Pick::Landmark(i) => self.select_landmark(world, i),
            Pick::Friend(id, name) => {
                self.landmark = None;
                world.map.track_avatar(id, &name);
            }
            Pick::History(i) => {
                if let Some(h) = world.tp_storage.items.get(i).cloned() {
                    self.landmark = None;
                    world.map.track_location(h.global.x, h.global.y, h.global.z as f32, false);
                    if let Some(t) = world.map.track.as_mut() {
                        t.label = h.title;
                    }
                    self.center_on(world, cam, h.global.x, h.global.y);
                }
            }
        }
    }

    /// A line's teleport button: a landmark goes through its asset
    /// (TeleportLandmarkRequest), a history entry to its place.
    fn teleport(&mut self, world: &mut World, pick: Pick, actions: &mut Vec<WorldMapAction>) {
        match pick {
            Pick::Landmark(i) => {
                if let Some(l) = self.ov.landmarks.items.get(i) {
                    actions.push(WorldMapAction::TeleportLandmark(l.asset));
                }
            }
            Pick::History(i) => {
                if let Some(h) = world.tp_storage.items.get(i).cloned() {
                    world.map.track_location(h.global.x, h.global.y, h.global.z as f32, true);
                    if let Some(t) = world.map.track.as_mut() {
                        t.label = h.title;
                    }
                }
            }
            Pick::Region(_) | Pick::Friend(..) => {}
        }
    }

    /// Top left: the search, its suggestions under it while typing (the
    /// drawer closed), and the drawer's button.
    #[allow(clippy::too_many_arguments)]
    fn search_panel(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        cam: &MapCamera,
        pictures: &MapPictures,
        map: Rect,
        fade: f32,
        now: f64,
    ) {
        let mut panel = self.ov.open(
            ui,
            p,
            "search",
            pos2(map.left() + EDGE, map.top() + EDGE),
            Align2::LEFT_TOP,
            Some(SEARCH_W),
            fade,
        );
        let ui = &mut panel.frame.content_ui;
        let id = search_id();
        let focused = ui.memory(|m| m.has_focus(id));
        let (mut down, mut up) = (false, false);
        if focused {
            down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
            up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        }
        let mut toggle = false;
        let field = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (r, _) = ui.allocate_exact_size(Vec2::new(16.0, 22.0), Sense::hover());
                icon(ui.painter(), "search-filled", r.center(), 15.0, p.muted);
                let hint_w = if self.ov.query.is_empty() { 0.0 } else { 44.0 };
                let w = (ui.available_width() - 26.0 - hint_w).max(60.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.ov.query)
                        .id(id)
                        .frame(egui::Frame::NONE.inner_margin(crate::ui::widgets::field_text_margin(
                            ui.ctx(),
                            &egui::TextStyle::Body.resolve(ui.style()),
                        )))
                        .hint_text(aurora_i18n::tr!("ui-worldmap-search-anything"))
                        .desired_width(w),
                );
                if !self.ov.query.is_empty() {
                    let (hr, _) = ui.allocate_exact_size(Vec2::new(hint_w - 6.0, 22.0), Sense::hover());
                    ui.painter().text(
                        hr.right_center(),
                        Align2::RIGHT_CENTER,
                        aurora_i18n::tr!("ui-worldmap-escape"),
                        egui::FontId::proportional(11.0),
                        p.muted_dim,
                    );
                }
                toggle = widgets::icon_toggle(
                    ui,
                    p,
                    "list-details-filled",
                    aurora_i18n::tr!("ui-worldmap-drawer"),
                    self.ov.drawer.is_some(),
                );
                r
            })
            .inner;
        if field.changed() {
            self.ov.suggest = true;
            self.ov.highlight = 0;
            self.ov.typed_at = now;
        }
        if field.gained_focus() && !self.ov.query.trim().is_empty() {
            self.ov.suggest = true;
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
        // the regions of the text, asked once it rests (LLFloaterWorldMap's
        // search, without its button)
        let q = self.ov.query.trim().to_lowercase();
        if !q.is_empty() && q != self.ov.asked {
            let wait = REGION_ASK_DELAY - (now - self.ov.typed_at);
            if wait <= 0.0 {
                world.map.search_region(&q);
                self.ov.asked = q.clone();
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
            }
        }
        let mut chosen: Option<Pick> = None;
        if self.ov.suggest && self.ov.drawer.is_none() && !q.is_empty() {
            let key = (
                q.clone(),
                world.map.sims.len(),
                world.inventory.generation,
                world.social.friends.iter().filter(|f| f.online).count(),
                world.tp_storage.items.len(),
            );
            if self.ov.built_for.as_ref() != Some(&key) || now - self.ov.built_at > SUGGEST_REFRESH {
                self.ov.suggestions = build_suggestions(world, &self.ov.landmarks, &q);
                self.ov.built_for = Some(key);
                self.ov.built_at = now;
                // the people dots of the regions shown (MAP_ITEM_AGENT_LOCATIONS)
                for s in &self.ov.suggestions {
                    if let Pick::Region(k) = s.pick
                        && s.rating.is_some()
                    {
                        world.map.want_agents(k);
                    }
                }
            }
            let n = self.ov.suggestions.len();
            if n > 0 {
                if down {
                    self.ov.highlight = (self.ov.highlight + 1) % n;
                }
                if up {
                    self.ov.highlight = (self.ov.highlight + n - 1) % n;
                }
                self.ov.highlight = self.ov.highlight.min(n - 1);
            }
            ui.add_space(4.0);
            widgets::divider(ui, p);
            let searching = world
                .map
                .search
                .as_ref()
                .is_some_and(|(_, sent)| sent.elapsed().as_secs_f32() < 3.0);
            let mut group = None;
            for (i, s) in self.ov.suggestions.iter().enumerate() {
                if group != Some(s.group) {
                    group = Some(s.group);
                    section_head(
                        ui,
                        p,
                        match s.group {
                            Group::Regions => aurora_i18n::tr!("ui-worldmap-regions"),
                            Group::Landmarks => aurora_i18n::tr!("common-landmarks"),
                            Group::Friends => aurora_i18n::tr!("ui-worldmap-online-friends"),
                            Group::History => aurora_i18n::tr!("ui-worldmap-history"),
                        },
                    );
                }
                let r = suggestion_row(ui, p, world, s, i == self.ov.highlight, pictures, &mut self.wanted_pics);
                if r.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                    self.ov.highlight = i;
                }
                if r.clicked() {
                    chosen = Some(s.pick.clone());
                }
            }
            if !self.ov.suggestions.iter().any(|s| s.group == Group::Regions) && searching {
                note(ui, p, aurora_i18n::tr!("ui-worldmap-searching"));
            } else if n == 0 {
                note(ui, p, aurora_i18n::tr!("common-no-results"));
            }
            if enter && n > 0 {
                chosen = Some(self.ov.suggestions[self.ov.highlight.min(n - 1)].pick.clone());
            }
        }
        self.ov.close(panel);
        if escape {
            self.ov.query.clear();
            self.ov.suggest = false;
        }
        if toggle {
            self.ov.drawer = if self.ov.drawer.is_some() { None } else { Some(Drawer::Landmarks) };
            self.ov.suggest = false;
        }
        if let Some(pick) = chosen {
            self.ov.query.clear();
            self.ov.suggest = false;
            self.pick(world, cam, pick);
        }
    }

    /// Under the search: the landmarks by folder, the teleport history and
    /// the friends, filtered by the search's text.
    #[allow(clippy::too_many_arguments)]
    fn drawer(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        tiles: &mut MapTiles,
        cam: &MapCamera,
        pictures: &MapPictures,
        map: Rect,
        tab: Drawer,
        top: f32,
        bottom: f32,
        fade: f32,
        actions: &mut Vec<WorldMapAction>,
    ) {
        let height = (bottom - top).max(140.0);
        let mut panel = self.ov.open(
            ui,
            p,
            "drawer",
            pos2(map.left() + EDGE, top),
            Align2::LEFT_TOP,
            Some(SEARCH_W),
            fade,
        );
        let ui = &mut panel.frame.content_ui;
        ui.set_height(height - 2.0 * f32::from(PAD) - 2.0);
        let online = world.social.friends.iter().filter(|f| f.online).count();
        // the tabs
        let count = self.ov.landmarks.items.len().to_string();
        let friends = online.to_string();
        let faces = [
            (
                Drawer::Landmarks,
                "map-pin-filled",
                p.rose,
                aurora_i18n::tr!("common-landmarks"),
                Some(count),
            ),
            (Drawer::History, "history", p.ink, aurora_i18n::tr!("ui-worldmap-history"), None),
            (Drawer::Friends, "users", p.ink, aurora_i18n::tr!("common-friends"), Some(friends)),
        ];
        let mut new_tab = None;
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), PILL_H), Sense::hover());
        let mut x = row.left();
        for (t, icon_name, tint, label, n) in faces {
            let text = match n {
                Some(n) => format!("{label} {n}"),
                None => label.to_owned(),
            };
            let face = PillFace {
                icon: icon_name,
                tint,
                label: &text,
                badges: Vec::new(),
                on: tab == t,
                enabled: true,
            };
            let w = face.width(ui);
            let r = Rect::from_min_size(pos2(x, row.top()), Vec2::new(w, PILL_H));
            let id = ui.scope_id().with(("drawer-tab", t as u8));
            if face.show(ui, p, r, id).clicked() {
                new_tab = Some(t);
            }
            x += w + 5.0;
        }
        ui.add_space(6.0);
        // the lines
        let filter = self.ov.query.trim().to_lowercase();
        let rows = self.drawer_rows(world, tab, &filter);
        let mut ask: Option<Ask> = None;
        let total: f32 = rows.iter().map(|r| r.height() + ROW_GAP).sum();
        let avail = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt(("worldmap-drawer", tab as u8))
            .auto_shrink([false, false])
            .max_height(avail)
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(total);
                let width = ui.available_width();
                let origin = ui.max_rect().min;
                let mut y = 0.0;
                for row in &rows {
                    let h = row.height();
                    if y + h >= viewport.min.y && y <= viewport.max.y {
                        let rect = Rect::from_min_size(origin + vec2(0.0, y), vec2(width, h));
                        if let Some(a) = self.drawer_row(ui, p, world, tiles, pictures, row, rect) {
                            ask = Some(a);
                        }
                    }
                    y += h + ROW_GAP;
                }
            });
        self.ov.close(panel);
        if let Some(t) = new_tab {
            self.ov.drawer = Some(t);
        }
        match ask {
            Some(Ask::Pick(pick)) => self.pick(world, cam, pick),
            Some(Ask::Teleport(pick)) => self.teleport(world, pick, actions),
            Some(Ask::ToggleFolder(id)) if !self.ov.closed_folders.remove(&id) => {
                self.ov.closed_folders.insert(id);
            }
            Some(Ask::ToggleFolder(_)) => {}
            None => {}
        }
    }

    fn drawer_rows(&self, world: &World, tab: Drawer, filter: &str) -> Vec<Row> {
        let mut rows = Vec::new();
        match tab {
            Drawer::Landmarks => {
                let list = &self.ov.landmarks;
                for (index, f) in list.folders.iter().enumerate() {
                    let items: Vec<usize> = f.items.clone().filter(|i| list.items[*i].lower.contains(filter)).collect();
                    if items.is_empty() {
                        continue;
                    }
                    // a search opens every folder holding a match
                    let open = !filter.is_empty() || !self.ov.closed_folders.contains(&f.id);
                    rows.push(Row::Folder { index, open });
                    if open {
                        rows.extend(items.into_iter().map(Row::Landmark));
                    }
                }
                if rows.is_empty() {
                    let fetching = list.items.is_empty()
                        && crate::ui::places::system_folder(&world.inventory, FOLDER_LANDMARKS)
                            .and_then(|f| world.inventory.folders.get(&f))
                            .is_none_or(|f| f.state != FetchState::Fetched);
                    rows.push(Row::Note(
                        if fetching {
                            aurora_i18n::tr!("common-loading-2")
                        } else if list.items.is_empty() {
                            aurora_i18n::tr!("ui-worldmap-no-landmarks-in-inventory")
                        } else {
                            aurora_i18n::tr!("common-no-results")
                        }
                        .to_owned(),
                    ));
                }
            }
            Drawer::History => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0.0, |d| d.as_secs_f64());
                for (section, items) in crate::ui::places::history_sections(&world.tp_storage.items, filter, now) {
                    rows.push(Row::Section(section));
                    rows.extend(items.into_iter().map(Row::History));
                }
                if rows.is_empty() {
                    rows.push(Row::Note(
                        if world.tp_storage.items.is_empty() {
                            aurora_i18n::tr!("ui-worldmap-history-empty")
                        } else {
                            aurora_i18n::tr!("common-no-results")
                        }
                        .to_owned(),
                    ));
                }
            }
            Drawer::Friends => {
                let mut friends: Vec<(Uuid, String, bool)> = world
                    .social
                    .friends
                    .iter()
                    .filter(|f| f.online)
                    .map(|f| (f.id, world.social.name_of(&f.id), f.rights_has & rights::MAP_LOCATION != 0))
                    .filter(|(_, n, _)| n.to_lowercase().contains(filter))
                    .collect();
                // those we can find on the map first
                friends.sort_by_key(|f| (!f.2, f.1.to_lowercase()));
                rows.extend(friends.into_iter().map(|(id, name, mappable)| Row::Friend { id, name, mappable }));
                if rows.is_empty() {
                    rows.push(Row::Note(
                        if filter.is_empty() {
                            aurora_i18n::tr!("ui-worldmap-no-friends-online")
                        } else {
                            aurora_i18n::tr!("common-no-results")
                        }
                        .to_owned(),
                    ));
                }
            }
        }
        rows
    }

    /// One line of the drawer, painted in `rect`.
    #[allow(clippy::too_many_arguments)]
    fn drawer_row(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        tiles: &mut MapTiles,
        pictures: &MapPictures,
        row: &Row,
        rect: Rect,
    ) -> Option<Ask> {
        let radius = ui.visuals().window_corner_radius;
        let body = egui::FontId::proportional(12.5);
        let small = egui::FontId::proportional(11.5);
        match row {
            Row::Folder { index, open } => {
                let f = &self.ov.landmarks.folders[*index];
                let id = ui.scope_id().with(("folder", f.id));
                let resp = ui.interact(rect, id, Sense::click());
                let ink = if resp.hovered() { p.muted } else { p.muted_dim };
                let caret = if *open { "caret-down-filled" } else { "caret-right-filled" };
                icon(ui.painter(), caret, pos2(rect.left() + 7.0, rect.center().y), 11.0, ink);
                let text = format!("{} · {}", f.name.to_uppercase(), f.items.len());
                let font = egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx()));
                widgets::paint_elided(ui, &text, pos2(rect.left() + 16.0, rect.center().y), rect.width() - 18.0, font, ink);
                resp.clicked().then_some(Ask::ToggleFolder(f.id))
            }
            Row::Section(i) => {
                let font = egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx()));
                let name = tphistory::history_sections()[(*i).min(8)].to_uppercase();
                widgets::paint_elided(ui, &name, pos2(rect.left() + 2.0, rect.center().y), rect.width(), font, p.muted_dim);
                None
            }
            Row::Note(text) => {
                widgets::paint_elided(
                    ui,
                    text,
                    pos2(rect.left() + 4.0, rect.center().y),
                    rect.width() - 8.0,
                    small,
                    p.muted,
                );
                None
            }
            Row::Landmark(i) => {
                let l = &self.ov.landmarks.items[*i];
                let (asset, item, thumb_id, name) = (l.asset, l.item, l.thumbnail, l.name.clone());
                let id = ui.scope_id().with(("landmark", item));
                let resp = ui.interact(rect, id, Sense::click());
                let go = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(24.0));
                let go_resp = ui.interact(go, id.with("go"), Sense::click());
                let selected = self.landmark.as_ref().is_some_and(|s| s.0 == item);
                let fill = if selected {
                    p.violet.gamma_multiply(0.22)
                } else if resp.hovered() {
                    p.raised
                } else {
                    p.field
                };
                ui.painter().rect_filled(rect, radius, fill);
                // where it points: its region's rating and name, and its map tile
                world.landmarks.resolve(asset);
                let place = match world.landmarks.peek(asset) {
                    Resolved::Ready { global, .. } => world
                        .map
                        .sim_at_global(global.x, global.y)
                        .map(|(k, s)| (k, s.name.clone(), s.access)),
                    _ => None,
                };
                let thumb = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - ROW_THUMB.y / 2.0), ROW_THUMB);
                let tex = self.thumbnail(tiles, pictures, thumb_id, place.as_ref().map(|p| p.0));
                place_picture(ui, p, thumb, tex.as_ref());
                let x = thumb.right() + 9.0;
                let w = go.left() - 6.0 - x;
                let mut name_x = x;
                if let Some((_, _, access)) = &place {
                    let badge = Rect::from_min_size(pos2(x, rect.top() + 8.0), Vec2::splat(13.0));
                    if widgets::paint_maturity_badge(
                        ui.painter(),
                        p,
                        crate::ui::bars::maturity_name(*access),
                        badge,
                        widgets::inner_radius(ui.visuals()),
                    ) {
                        name_x = badge.right() + 5.0;
                    }
                }
                let bold = egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
                widgets::paint_elided(ui, &name, pos2(name_x, rect.top() + 14.5), w - (name_x - x), bold, p.ink);
                let region = place.map_or_else(|| aurora_i18n::tr!("common-loading-2").to_owned(), |p| p.1);
                widgets::paint_elided(ui, &region, pos2(x, rect.bottom() - 12.5), w, small, p.muted);
                if go_resp.hovered() {
                    ui.painter().rect_filled(go, ui.visuals().widgets.inactive.corner_radius, p.raised);
                }
                let go_ink = if go_resp.hovered() { p.ink } else { p.muted };
                icon(ui.painter(), "location-filled", go.center(), 14.0, go_ink);
                let go_resp = go_resp.on_hover_text(aurora_i18n::tr!("common-teleport-3"));
                if go_resp.clicked() {
                    Some(Ask::Teleport(Pick::Landmark(*i)))
                } else if resp.clicked() {
                    Some(Ask::Pick(Pick::Landmark(*i)))
                } else {
                    None
                }
            }
            Row::History(i) => {
                let h: &HistoryItem = world.tp_storage.items.get(*i)?;
                let title = h.title.clone();
                let id = ui.scope_id().with(("history", *i));
                let resp = ui.interact(rect, id, Sense::click());
                let go = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(24.0));
                let go_resp = ui.interact(go, id.with("go"), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, radius, p.raised);
                }
                icon(ui.painter(), "history", pos2(rect.left() + 12.0, rect.center().y), 14.0, p.muted);
                widgets::paint_elided(
                    ui,
                    &title,
                    pos2(rect.left() + 28.0, rect.center().y),
                    go.left() - 6.0 - rect.left() - 28.0,
                    body,
                    p.ink,
                );
                if go_resp.hovered() {
                    ui.painter().rect_filled(go, ui.visuals().widgets.inactive.corner_radius, p.raised);
                }
                icon(
                    ui.painter(),
                    "location-filled",
                    go.center(),
                    14.0,
                    if go_resp.hovered() { p.ink } else { p.muted },
                );
                let go_resp = go_resp.on_hover_text(aurora_i18n::tr!("common-teleport-3"));
                if go_resp.clicked() {
                    Some(Ask::Teleport(Pick::History(*i)))
                } else if resp.clicked() {
                    Some(Ask::Pick(Pick::History(*i)))
                } else {
                    None
                }
            }
            Row::Friend { id, name, mappable } => {
                let sense = if *mappable { Sense::click() } else { Sense::hover() };
                let row_id = ui.scope_id().with(("friend", *id));
                let resp = ui.interact(rect, row_id, sense);
                let followed = world.map.avatar.as_ref().is_some_and(|a| a.id == *id);
                if followed {
                    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.22));
                } else if resp.hovered() && *mappable {
                    ui.painter().rect_filled(rect, radius, p.raised);
                }
                let disc = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), Vec2::splat(22.0));
                self.wanted_pics.insert(*id);
                widgets::avatar_disc(ui, p, disc, pictures.avatars.get(id), name, !*mappable);
                crate::ui::presence::paint_dot(ui.painter(), p, disc, crate::ui::presence::of(world, id), p.panel);
                let detail = if !*mappable {
                    aurora_i18n::tr!("ui-worldmap-not-sharing").to_owned()
                } else if followed {
                    followed_region(world, *id).unwrap_or_else(|| aurora_i18n::tr!("common-loading-2").to_owned())
                } else {
                    String::new()
                };
                let dw = if detail.is_empty() {
                    0.0
                } else {
                    let g = ui.painter().layout_no_wrap(detail.clone(), small.clone(), p.muted_dim);
                    let w = g.size().x.min(rect.width() * 0.5);
                    widgets::paint_elided(ui, &detail, pos2(rect.right() - 6.0 - w, rect.center().y), w, small, p.muted_dim);
                    w + 8.0
                };
                let ink = if *mappable { p.ink } else { p.muted };
                widgets::paint_elided(
                    ui,
                    name,
                    pos2(disc.right() + 8.0, rect.center().y),
                    rect.right() - 6.0 - dw - disc.right() - 8.0,
                    body,
                    ink,
                );
                if *mappable {
                    let resp = resp.on_hover_text(aurora_i18n::tr!("common-show-on-map"));
                    resp.clicked().then(|| Ask::Pick(Pick::Friend(*id, name.clone())))
                } else {
                    resp.on_hover_text(aurora_i18n::tr!("ui-worldmap-not-sharing"));
                    None
                }
            }
        }
    }

    /// The picture of a place: an inventory thumbnail, else its region's
    /// tile of the map.
    fn thumbnail(
        &mut self,
        tiles: &mut MapTiles,
        pictures: &MapPictures,
        thumbnail: Uuid,
        region: Option<(u32, u32)>,
    ) -> Option<egui::TextureHandle> {
        if !thumbnail.is_nil() {
            if let Some(t) = pictures.images.get(&thumbnail) {
                return Some(t.clone());
            }
            self.wanted_images.insert(thumbnail);
        }
        let (gx, gy) = region?;
        tiles.get(1, gx, gy, true).ok()
    }

    /// Top right: the layers, as pills; the fine settings behind the last.
    fn layers(&mut self, ui: &mut egui::Ui, p: &Palette, opts: &mut MapSettings, map: Rect, fade: f32) {
        // hubs, people, land and events show from this zoom (the side
        // panel's legend greys out too)
        let detail = scale_to_level(256.0 * self.zoom.exp2()) <= DRAW_SIMINFO_THRESHOLD;
        let mut ratings: Vec<&'static str> = vec![aurora_i18n::tr!("common-general")];
        if opts.world_mature_events {
            ratings.push(aurora_i18n::tr!("common-moderate"));
        }
        if opts.world_adult_events {
            ratings.push(aurora_i18n::tr!("common-adult"));
        }
        let faces = [
            PillFace {
                icon: "users",
                tint: crate::ui::colors::c(crate::ui::colors::get().map_other),
                label: aurora_i18n::tr!("common-people"),
                badges: Vec::new(),
                on: opts.world_people,
                enabled: detail,
            },
            PillFace {
                icon: "tag-filled",
                tint: item_icon(ItemKind::ForSale, p).1,
                label: aurora_i18n::tr!("ui-worldmap-for-sale"),
                badges: Vec::new(),
                on: opts.world_land_for_sale,
                enabled: detail,
            },
            PillFace {
                icon: "calendar-event-filled",
                tint: item_icon(ItemKind::EventPg, p).1,
                label: aurora_i18n::tr!("ui-worldmap-events"),
                badges: if opts.world_events { ratings } else { Vec::new() },
                on: opts.world_events,
                enabled: detail,
            },
            PillFace {
                icon: "map-pin-filled",
                tint: p.rose,
                label: aurora_i18n::tr!("common-landmarks"),
                badges: Vec::new(),
                on: opts.world_landmarks,
                enabled: true,
            },
            PillFace {
                icon: "info-circle-filled",
                tint: item_icon(ItemKind::Infohub, p).1,
                label: aurora_i18n::tr!("ui-worldmap-infohubs"),
                badges: Vec::new(),
                on: opts.world_infohubs,
                enabled: detail,
            },
            PillFace {
                icon: "building-broadcast-tower-filled",
                tint: item_icon(ItemKind::Telehub, p).1,
                label: aurora_i18n::tr!("ui-worldmap-telehubs"),
                badges: Vec::new(),
                on: opts.world_telehubs,
                enabled: detail,
            },
            PillFace {
                icon: "adjustments-horizontal-filled",
                tint: p.ink,
                label: "",
                badges: Vec::new(),
                on: false,
                enabled: true,
            },
        ];
        let tips = [
            aurora_i18n::tr!("common-people"),
            aurora_i18n::tr!("common-land-for-sale"),
            aurora_i18n::tr!("ui-worldmap-events"),
            aurora_i18n::tr!("ui-worldmap-landmarks-layer"),
            aurora_i18n::tr!("ui-worldmap-infohubs"),
            aurora_i18n::tr!("ui-worldmap-telehubs"),
            aurora_i18n::tr!("ui-worldmap-layer-options"),
        ];
        // rows filled from the left, each pushed against the right edge
        let max_w = (map.width() - SEARCH_W - 3.0 * EDGE - 2.0 * f32::from(PAD)).clamp(140.0, LAYERS_MAX_W);
        let widths: Vec<f32> = faces.iter().map(|f| f.width(ui)).collect();
        let mut rows: Vec<(std::ops::Range<usize>, f32)> = Vec::new();
        let (mut start, mut w) = (0, 0.0);
        for (i, fw) in widths.iter().enumerate() {
            let next = if i == start { *fw } else { w + 5.0 + fw };
            if next > max_w && i > start {
                rows.push((start..i, w));
                start = i;
                w = *fw;
            } else {
                w = next;
            }
        }
        rows.push((start..faces.len(), w));
        let content_w = rows.iter().map(|r| r.1).fold(0.0, f32::max);
        let content_h = rows.len() as f32 * PILL_H + (rows.len() as f32 - 1.0) * 5.0;
        let mut panel = self.ov.open(
            ui,
            p,
            "layers",
            pos2(map.right() - EDGE, map.top() + EDGE),
            Align2::RIGHT_TOP,
            None,
            fade,
        );
        let ui = &mut panel.frame.content_ui;
        let (area, _) = ui.allocate_exact_size(Vec2::new(content_w, content_h), Sense::hover());
        let mut clicked = None;
        for (r, (range, row_w)) in rows.iter().enumerate() {
            let mut x = area.right() - row_w;
            let y = area.top() + r as f32 * (PILL_H + 5.0);
            for i in range.clone() {
                let rect = Rect::from_min_size(pos2(x, y), Vec2::new(widths[i], PILL_H));
                let id = ui.scope_id().with(("layer", i));
                let mut resp = faces[i].show(ui, p, rect, id);
                resp = if faces[i].enabled {
                    resp.on_hover_text(tips[i])
                } else {
                    resp.on_hover_text(aurora_i18n::tr!("ui-worldmap-zoom-in-to-see-details"))
                };
                if i == faces.len() - 1 {
                    crate::ui::menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                        crate::ui::menu::toggle(
                            ui,
                            p,
                            "calendar-event-filled",
                            aurora_i18n::tr!("ui-worldmap-moderate-events"),
                            &mut opts.world_mature_events,
                        );
                        crate::ui::menu::toggle(
                            ui,
                            p,
                            "calendar-event-filled",
                            aurora_i18n::tr!("ui-worldmap-adult-events"),
                            &mut opts.world_adult_events,
                        );
                        crate::ui::menu::separator(ui, p);
                        crate::ui::menu::toggle(
                            ui,
                            p,
                            "grid-pattern-filled",
                            aurora_i18n::tr!("ui-worldmap-grid-coordinates"),
                            &mut opts.world_grid_coords,
                        );
                        crate::ui::menu::toggle(
                            ui,
                            p,
                            "location-filled",
                            aurora_i18n::tr!("ui-worldmap-double-click-teleport"),
                            &mut opts.world_double_click_tp,
                        );
                    });
                } else if resp.clicked() {
                    clicked = Some(i);
                }
                x += widths[i] + 5.0;
            }
        }
        self.ov.close(panel);
        match clicked {
            Some(0) => opts.world_people ^= true,
            Some(1) => opts.world_land_for_sale ^= true,
            Some(2) => opts.world_events ^= true,
            Some(3) => opts.world_landmarks ^= true,
            Some(4) => opts.world_infohubs ^= true,
            Some(5) => opts.world_telehubs ^= true,
            _ => {}
        }
    }

    /// Bottom right: zoom in and out, back to our position, home.
    fn nav(&mut self, ui: &mut egui::Ui, p: &Palette, map: Rect, fade: f32, actions: &mut Vec<WorldMapAction>) {
        let mut panel = self.ov.open(
            ui,
            p,
            "nav",
            pos2(map.right() - EDGE, map.bottom() - EDGE),
            Align2::RIGHT_BOTTOM,
            None,
            fade,
        );
        let ui = &mut panel.frame.content_ui;
        ui.spacing_mut().item_spacing.y = 4.0;
        let size = Vec2::splat(NAV_BUTTON);
        let zin = widgets::action_button_if(ui, p, "plus", "", ActionKind::Flat, size, self.target_zoom < ZOOM_MAX)
            .on_hover_text(aurora_i18n::tr!("ui-worldmap-zoom-in"))
            .clicked();
        let zout = widgets::action_button_if(ui, p, "minus", "", ActionKind::Flat, size, self.target_zoom > ZOOM_MIN)
            .on_hover_text(aurora_i18n::tr!("ui-worldmap-zoom-out"))
            .clicked();
        let me = widgets::action_button(ui, p, "current-location-filled", "", ActionKind::Flat, size)
            .on_hover_text(aurora_i18n::tr!("ui-worldmap-center-map-on-my-position"))
            .clicked();
        let home = widgets::action_button(ui, p, "home-filled", "", ActionKind::Flat, size)
            .on_hover_text(aurora_i18n::tr!("common-go-home"))
            .clicked();
        self.ov.close(panel);
        if zin || zout {
            let step = if zin { ZOOM_STEP } else { -ZOOM_STEP };
            self.target_zoom = (self.target_zoom + step).clamp(ZOOM_MIN, ZOOM_MAX);
            self.zoom_pivot = None;
        }
        if me {
            self.target_pan = Vec2::ZERO;
            self.pan_tau = PAN_TAU_CENTER;
        }
        if home {
            actions.push(WorldMapAction::TeleportHome);
        }
    }

    /// Bottom left: the selected place: its picture, its name and rating,
    /// its people and events, the position (editable) and how far it is,
    /// then teleport, SLurl, its profile, centring and clearing. Returns
    /// its rectangle when shown.
    #[allow(clippy::too_many_arguments)]
    fn place_card(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        tiles: &mut MapTiles,
        cam: &MapCamera,
        pictures: &MapPictures,
        map: Rect,
        fade: f32,
        actions: &mut Vec<WorldMapAction>,
    ) -> Option<Rect> {
        let selection = self.selection(world)?;
        if self.ov.card_closed.as_ref() == Some(&selection) {
            return None;
        }
        self.ov.card_closed = None;
        let track = world.map.track.clone();
        let avatar = world.map.avatar.clone();
        let region = track
            .as_ref()
            .and_then(|t| world.map.sim_at_global(t.x, t.y))
            .map(|(k, s)| (k, s.clone()));
        let landmark = self.landmark.clone();
        let title = if world.rlva.has("showloc") && landmark.is_none() && avatar.is_none() {
            aurora_i18n::tr!("rlva-hidden-region").to_owned()
        } else if let Some((_, _, n)) = &landmark {
            n.clone()
        } else if let Some(a) = &avatar {
            if world.rlva_can_name(a.id) {
                a.name.clone()
            } else {
                world.social.name_of(&a.id)
            }
        } else if let Some((_, s)) = &region {
            s.name.clone()
        } else if track.as_ref().is_some_and(|t| t.pending) {
            aurora_i18n::tr!("common-loading-2").to_owned()
        } else {
            aurora_i18n::tr!("ui-worldmap-invalid-location").to_owned()
        };
        let rating = region.as_ref().filter(|(_, s)| !s.is_down()).map(|(_, s)| s.access);
        // the line under the name
        let mut parts: Vec<String> = Vec::new();
        if avatar.as_ref().is_some_and(|a| !a.found) {
            parts.push(aurora_i18n::tr!("common-loading-2").to_owned());
        }
        if let Some((key, s)) = &region {
            if s.name != title {
                parts.push(s.name.clone());
            }
            if s.is_down() {
                parts.push(s.access_label().to_owned());
            } else {
                let me = {
                    let g = to_global(world, world.agent.position);
                    world.map.sim_at_global(g.0, g.1).map(|(k, _)| k)
                };
                let n: i32 = world.map.agents.get(key).map(|v| v.iter().map(|a| a.2).sum()).unwrap_or(0) + i32::from(me == Some(*key));
                if n > 0 {
                    parts.push(aurora_i18n::tr!("person-count", count = n).to_string());
                }
                let events = events_in(world, *key, s);
                if events > 0 {
                    parts.push(aurora_i18n::tr!("ui-worldmap-event-count", count = events as i64).to_string());
                }
            }
        } else if landmark.is_some() && self.ov.landmark_at.is_none() {
            parts.push(aurora_i18n::tr!("common-loading-2").to_owned());
        }
        let detail = parts.join(" · ");
        // the parcel there, as Firestorm's map asks it for a spot
        if landmark.is_none() && avatar.is_none() && !world.rlva.has("showloc") {
            world.map.want_track_parcel();
        }
        let parcel = world
            .map
            .track_parcel_id()
            .filter(|_| landmark.is_none() && avatar.is_none() && !world.rlva.has("showloc"))
            .and_then(|id| world.profiles.parcels.get(&id))
            .map(|p| p.name.trim().to_owned())
            .filter(|n| !n.is_empty())
            .map(|name| aurora_i18n::tr!("ui-worldmap-parcel", name = name).to_string());
        let tooltip = track
            .as_ref()
            .map(|t| t.tooltip.clone())
            .filter(|t| !t.is_empty() && !world.rlva.has("showloc"));
        let detail = if world.rlva.has("showloc") { String::new() } else { detail };
        // its picture
        let thumb_id = landmark
            .as_ref()
            .and_then(|(item, ..)| world.inventory.items.get(item))
            .map_or(Uuid::nil(), |i| i.thumbnail);
        let tex = self.thumbnail(tiles, pictures, thumb_id, region.as_ref().map(|r| r.0));
        // what its buttons do
        let can_teleport = landmark.is_some() || track.as_ref().is_some_and(|t| !t.pending && !t.invalid);
        let local = |t: &crate::world::worldmap::Track, (sx, sy): (u32, u32)| {
            glam::Vec3::new((t.x - sx as f64 * 256.0) as f32, (t.y - sy as f64 * 256.0) as f32, t.z)
        };
        let slurl = track
            .as_ref()
            .zip(region.as_ref())
            .map(|(t, (k, s))| crate::slurl::make(&s.name, local(t, *k)));
        let source = match (&landmark, &track, &region) {
            (Some((item, asset, _)), ..) => Some(Source::Landmark {
                item: *item,
                asset: *asset,
            }),
            (None, Some(t), Some((k, s))) => Some(Source::Link {
                region: s.name.clone(),
                pos: local(t, *k),
            }),
            _ => None,
        };
        let me = to_global(world, world.agent.position);

        let width = (map.width() - 3.0 * EDGE - NAV_BUTTON - 2.0 * f32::from(PAD)).clamp(260.0, CARD_W);
        let mut panel = self.ov.open(
            ui,
            p,
            "card",
            pos2(map.left() + EDGE, map.bottom() - EDGE),
            Align2::LEFT_BOTTOM,
            Some(width),
            fade,
        );
        let ui = &mut panel.frame.content_ui;
        let mut close = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (thumb, _) = ui.allocate_exact_size(CARD_THUMB, Sense::hover());
            place_picture(ui, p, thumb, tex.as_ref());
            let text_w = ui.available_width() - 30.0;
            ui.vertical(|ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if let Some(a) = rating {
                        widgets::maturity_badge_of(ui, p, a, 15.0);
                    }
                    let font = egui::FontId::new(14.0, crate::ui::fonts::semibold(ui.ctx()));
                    ui.add(egui::Label::new(egui::RichText::new(&title).font(font).color(p.ink)).truncate());
                });
                if !detail.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(&detail).size(12.0).color(p.muted)).wrap());
                }
                if let Some(t) = &parcel {
                    ui.add(egui::Label::new(egui::RichText::new(t).size(12.0).color(p.muted)).wrap());
                }
                if let Some(t) = &tooltip {
                    ui.add(egui::Label::new(egui::RichText::new(t).size(12.0).color(p.muted)).wrap());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| {
                close = widgets::icon_button(ui, p, "x", aurora_i18n::tr!("ui-worldmap-hide-card"), true);
            });
        });
        // the position in the region, and how far it is
        let mut edited: Option<(f64, f64, f32, bool)> = None;
        if let (Some(t), Some((k, _))) = (&track, &region)
            && !world.rlva.has("showloc")
        {
            ui.add_space(8.0);
            let l = local(t, *k);
            let (mut lx, mut ly, mut lz) = (l.x, l.y, l.z);
            ui.horizontal(|ui| {
                ui.spacing_mut().interact_size.x = 46.0;
                for (label, v, max) in [("X", &mut lx, 8191.0), ("Y", &mut ly, 8191.0), ("Z", &mut lz, 10000.0)] {
                    ui.label(egui::RichText::new(label).size(12.0).color(p.muted));
                    ui.add(
                        egui::DragValue::new(v)
                            .custom_parser(aurora_i18n::parse_number)
                            .range(0.0..=max)
                            .speed(1.0)
                            .max_decimals(0)
                            .update_while_editing(false),
                    );
                }
                let moved = (lx, ly) != (l.x, l.y);
                let lifted = lz != l.z;
                if moved || lifted {
                    edited = Some((k.0 as f64 * 256.0 + lx as f64, k.1 as f64 * 256.0 + ly as f64, lz, moved));
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let d = (t.x - me.0).hypot(t.y - me.1);
                    ui.label(egui::RichText::new(distance_text(d)).size(12.0).color(p.muted));
                });
            });
        }
        Self::friend_altitude_hint(ui, p, world);
        ui.add_space(8.0);
        let (mut go, mut copy, mut info, mut show, mut clear) = (false, false, false, false, false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let h = Vec2::new(0.0, BUTTON_H);
            let icon_only = Vec2::new(30.0, BUTTON_H);
            go = widgets::action_button_if(
                ui,
                p,
                "location-filled",
                aurora_i18n::tr!("common-teleport-3"),
                ActionKind::Primary,
                h,
                can_teleport,
            )
            .clicked();
            copy = widgets::action_button_if(
                ui,
                p,
                "link-filled",
                aurora_i18n::tr!("common-copy-slurl"),
                ActionKind::Flat,
                h,
                slurl.is_some() && !world.rlva.has("showloc"),
            )
            .clicked();
            info = widgets::action_button_if(
                ui,
                p,
                "info-circle-filled",
                aurora_i18n::tr!("ui-worldmap-info"),
                ActionKind::Flat,
                h,
                source.is_some() && !world.rlva.has("showloc"),
            )
            .on_hover_text(aurora_i18n::tr!("ui-worldmap-info-help"))
            .clicked();
            show = widgets::action_button_if(ui, p, "focus-centered", "", ActionKind::Flat, icon_only, track.is_some())
                .on_hover_text(aurora_i18n::tr!("ui-worldmap-show-selection"))
                .clicked();
            clear = widgets::action_button(ui, p, "backspace-filled", "", ActionKind::Flat, icon_only)
                .on_hover_text(format!(
                    "{}\n{}",
                    aurora_i18n::tr!("ui-waypoint-remove"),
                    aurora_i18n::tr!("ui-waypoint-remove-help")
                ))
                .clicked();
        });
        let ctx = ui.ctx().clone();
        let rect = self.ov.close(panel);

        if let Some((x, y, z, moved)) = edited {
            // another spot: no landmark nor friend any more; a height only:
            // the same spot
            self.landmark = None;
            self.ov.landmark_at = None;
            if moved {
                let label = track.as_ref().map(|t| t.label.clone()).unwrap_or_default();
                world.map.track_location(x, y, z, false);
                let name = world.map.sim_at_global(x, y).map(|(_, s)| s.name.clone());
                if let (Some(t), Some(n)) = (world.map.track.as_mut(), name) {
                    let (lx, ly) = (x.rem_euclid(256.0), y.rem_euclid(256.0));
                    t.label = if label.starts_with(&n) {
                        location_label(&n, lx, ly, z)
                    } else {
                        label
                    };
                }
            } else {
                world.map.set_track_altitude(z);
            }
        }
        if go {
            match (&self.landmark, &world.map.track) {
                (Some((_, asset, _)), _) => actions.push(WorldMapAction::TeleportLandmark(*asset)),
                (None, Some(t)) if !t.pending && !t.invalid => {
                    world.map.push(crate::world::worldmap::teleport_command(t.x, t.y, t.z));
                }
                _ => {}
            }
        }
        if copy
            && !world.rlva.has("showloc")
            && let Some(u) = slurl
        {
            ctx.copy_text(u);
        }
        if info
            && !world.rlva.has("showloc")
            && let Some(s) = source
        {
            actions.push(WorldMapAction::OpenPlace(s));
        }
        if show && let Some(t) = &track {
            self.center_on(world, cam, t.x, t.y);
        }
        if clear {
            world.map.clear_track();
            world.map.search = None;
            self.search.clear();
            self.ov.query.clear();
            self.landmark = None;
            self.ov.landmark_at = None;
            self.selected = None;
            self.target_pan = Vec2::ZERO;
            self.pan_tau = PAN_TAU_CENTER;
        }
        if close {
            self.ov.card_closed = Some(selection);
        }
        Some(rect)
    }
}

/// The grey capitals over a group of suggestions.
fn section_head(ui: &mut egui::Ui, p: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
    let font = egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx()));
    widgets::paint_elided(
        ui,
        &text.to_uppercase(),
        pos2(rect.left() + 8.0, rect.center().y + 2.0),
        rect.width() - 16.0,
        font,
        p.muted_dim,
    );
}

fn note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SUGGEST_ROW), Sense::hover());
    widgets::paint_elided(
        ui,
        text,
        pos2(rect.left() + 8.0, rect.center().y),
        rect.width() - 16.0,
        egui::FontId::proportional(12.0),
        p.muted,
    );
}

/// One suggestion: what it is (the rating's badge, a pin, the friend's
/// picture, the history's icon), its name, and grey at the right what it
/// holds or where it is.
#[allow(clippy::too_many_arguments)]
fn suggestion_row(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    s: &Suggestion,
    lit: bool,
    pictures: &MapPictures,
    wanted_pics: &mut HashSet<Uuid>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SUGGEST_ROW), Sense::click());
    if lit || resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.violet.gamma_multiply(0.22));
    }
    let lead = pos2(rect.left() + 15.0, rect.center().y);
    match &s.pick {
        Pick::Region(_) => {
            if let Some(a) = s.rating {
                let r = Rect::from_center_size(lead, Vec2::splat(15.0));
                widgets::paint_maturity_badge(
                    ui.painter(),
                    p,
                    crate::ui::bars::maturity_name(a),
                    r,
                    widgets::inner_radius(ui.visuals()),
                );
            }
        }
        Pick::Landmark(_) => icon(ui.painter(), "map-pin-filled", lead, 14.0, p.rose),
        Pick::Friend(id, name) => {
            let disc = Rect::from_center_size(lead, Vec2::splat(20.0));
            wanted_pics.insert(*id);
            widgets::avatar_disc(ui, p, disc, pictures.avatars.get(id), name, false);
            crate::ui::presence::paint_dot(ui.painter(), p, disc, crate::ui::presence::of(world, id), p.panel);
        }
        Pick::History(_) => icon(ui.painter(), "history", lead, 14.0, p.muted),
    }
    let small = egui::FontId::proportional(11.0);
    let dw = if s.detail.is_empty() {
        0.0
    } else {
        let w = ui
            .painter()
            .layout_no_wrap(s.detail.clone(), small.clone(), p.muted_dim)
            .size()
            .x
            .min(rect.width() * 0.45);
        widgets::paint_elided(ui, &s.detail, pos2(rect.right() - 8.0 - w, rect.center().y), w, small, p.muted_dim);
        w + 10.0
    };
    let x = rect.left() + 30.0;
    widgets::paint_elided(
        ui,
        &s.label,
        pos2(x, rect.center().y),
        rect.right() - 8.0 - dw - x,
        egui::FontId::proportional(12.5),
        p.ink,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use aurora_net::MapBlock;
    use aurora_net::inventory::{FolderContents, InvFolder, InvItem};
    use std::sync::Arc;

    fn folder(id: u128, parent: u128, name: &str, t: i32) -> InvFolder {
        InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            type_default: t,
            version: 1,
            favorite: false,
            favorite_known: false,
            thumbnail: Uuid::nil(),
        }
    }

    fn landmark(id: u128, parent: u128, name: &str) -> InvItem {
        InvItem {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            asset_type: ASSET_LANDMARK,
            inv_type: 3,
            asset_id: Uuid::from_u128(id + 1000),
            ..Default::default()
        }
    }

    fn contents(f: &InvFolder, folders: Vec<InvFolder>, items: Vec<InvItem>) -> FolderContents {
        FolderContents {
            folder_id: f.id,
            owner_id: Uuid::nil(),
            version: 2,
            folders,
            items,
        }
    }

    /// Landmarks in Favorites, Landmarks, a sub-folder and the trash.
    fn inventory(inv: &mut Inventory) {
        let root = folder(1, 0, "My Inventory", 8);
        let lms = folder(10, 1, "Landmarks", FOLDER_LANDMARKS);
        let favs = folder(11, 1, "Favorites", FOLDER_FAVORITES);
        let trash = folder(12, 1, "Trash", FOLDER_TRASH);
        let clubs = folder(20, 10, "Clubs", -1);
        inv.apply(vec![contents(&root, vec![lms.clone(), favs.clone(), trash.clone()], Vec::new())]);
        inv.apply(vec![contents(
            &lms,
            vec![clubs.clone()],
            vec![landmark(100, 10, "Plage de la lagune"), landmark(101, 10, "Arbre")],
        )]);
        inv.apply(vec![contents(&clubs, Vec::new(), vec![landmark(200, 20, "Club des Lucioles")])]);
        inv.apply(vec![contents(&favs, Vec::new(), vec![landmark(300, 11, "Lagon")])]);
        inv.apply(vec![contents(&trash, Vec::new(), vec![landmark(400, 12, "Lagune jetée")])]);
    }

    fn block(x: u16, y: u16, name: &str, access: u8) -> MapBlock {
        MapBlock {
            x,
            y,
            name: name.into(),
            access,
            region_flags: 0,
            water_height: 20,
            agents: 0,
            map_image_id: Uuid::nil(),
            size_x: 256,
            size_y: 256,
        }
    }

    fn world() -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        inventory(&mut w.inventory);
        w.map.apply_blocks(vec![
            block(1000, 1000, "Aurora Demo", 13),
            block(1001, 1000, "Lagune Boréale", 21),
            block(1002, 1000, "Lagoon Nights", 42),
            block(1003, 1000, "Faille lagunaire", 254),
        ]);
        w.map
            .agents
            .insert((1001, 1000), vec![(256_300, 256_100, 2), (256_310, 256_120, 1)]);
        let friend = |id: u128, online: bool, rights_has: i32| crate::world::social::Friend {
            id: Uuid::from_u128(id),
            online,
            rights_given: 0,
            rights_has,
        };
        w.social.friends = vec![friend(50, true, 3), friend(51, true, 1), friend(52, false, 3)];
        w.social.names.insert(Uuid::from_u128(50), "Nova Lagune".into());
        w.social.names.insert(Uuid::from_u128(51), "Laguna Sans Carte".into());
        w.social.names.insert(Uuid::from_u128(52), "Lagune Hors Ligne".into());
        w.tp_storage = tphistory::HistoryStorage::in_memory(vec![HistoryItem {
            title: "Plage, Lagune Boréale".into(),
            global: glam::DVec3::new(256_300.0, 256_100.0, 22.0),
            date: 0.0,
            slurl: String::new(),
        }]);
        w
    }

    #[test]
    fn the_landmarks_are_listed_by_folder_favorites_first_without_the_trash() {
        let w = world();
        let mut list = LandmarkList::default();
        assert!(list.refresh(&w.inventory));
        assert!(!list.refresh(&w.inventory), "rebuilt only when the inventory changes");
        let folders: Vec<&str> = list.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(folders, ["Favorites", "Landmarks", "Clubs"]);
        let names: Vec<&str> = list.items.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Lagon", "Arbre", "Plage de la lagune", "Club des Lucioles"]);
    }

    #[test]
    fn one_search_suggests_regions_landmarks_friends_and_history() {
        let mut w = world();
        let mut list = LandmarkList::default();
        list.refresh(&w.inventory);
        let s = build_suggestions(&w, &list, "LAG");
        let groups: Vec<Group> = s.iter().map(|s| s.group).collect();
        let labels: Vec<&str> = s.iter().map(|s| s.label.as_str()).collect();
        // regions starting with the text first, then those holding it; the
        // landmarks out of the trash; the online friends who can be mapped
        assert_eq!(
            labels,
            [
                "Lagoon Nights",
                "Lagune Boréale",
                "Faille lagunaire",
                "Lagon",
                "Plage de la lagune",
                "Nova Lagune",
                "Plage, Lagune Boréale"
            ]
        );
        assert_eq!(
            groups,
            [
                Group::Regions,
                Group::Regions,
                Group::Regions,
                Group::Landmarks,
                Group::Landmarks,
                Group::Friends,
                Group::History
            ]
        );
        // the rating's badge and the people of a region; a region down says so
        assert_eq!(s[1].rating, Some(21));
        assert_eq!(s[1].detail, aurora_i18n::tr!("person-count", count = 3));
        assert_eq!(s[2].rating, None);
        // a landmark names its region once its asset is read, else its folder
        assert_eq!(s[4].detail, "Landmarks");
        w.landmarks.on_data(
            Uuid::from_u128(1100),
            Some(b"Landmark version 1\nposition 256300 256100 22\n".as_slice()),
        );
        let s = build_suggestions(&w, &list, "plage de");
        assert_eq!(s[0].detail, "Lagune Boréale");
        assert_eq!(s[0].rating, Some(21));
        assert!(build_suggestions(&w, &list, "  ").is_empty());
    }

    #[test]
    fn distances_read_in_metres_then_kilometres() {
        assert!(distance_text(350.4).contains("350 m"));
        assert!(distance_text(1234.0).contains(&aurora_i18n::number_text("1.2 km")));
    }
}
