//! World map floater (LLFloaterWorldMap + LLWorldMapView): map server tiles
//! by mip level with coarser levels shown while the current one loads,
//! region names and access, people, hubs, land for sale and events,
//! home, our position and camera cone, the tracked location; wheel zoom
//! around the pointer, drag pan, click to track, double-click to teleport.
//! Two layouts (Preferences › Aurora › World map): the map filling the
//! window with its controls floating over it (`overlay.rs`, the default),
//! or Firestorm's right panel with the legend, landmarks, region search,
//! the coordinates and the teleport / SLurl buttons.
//!
//! Ported from Firestorm's llworldmapview.cpp / llfloaterworldmap.cpp
//! (originally LGPL 2.1, Linden Research, Inc.).

mod overlay;

use super::map_tiles::{MAP_LEVELS, MapTiles, scale_to_level, tile_origin};
use super::minimap::{MapCamera, TRACK_COLOR, View, draw_directions, draw_self, draw_tracking, text_shadow, to_global};
use super::widgets::{self, Floater};
use crate::settings::{MapSettings, WorldMapStyle};
use crate::theme::Palette;
use crate::world::World;
use crate::world::place_details::Source;
use crate::world::worldmap::{Track, access_label, location_label, teleport_command};
use aurora_net::{NetCommand, map_item};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Ocean under the tiles (#1D475F).
const OCEAN: Color32 = Color32::from_rgb(0x1D, 0x47, 0x5F);
const DOWN_FILL: Color32 = Color32::from_rgba_premultiplied(20, 0, 0, 102);
/// Labels from this zoom (DRAW_TEXT_THRESHOLD, px per region).
const DRAW_TEXT_THRESHOLD: f32 = 96.0;
/// Region info, people and items up to this mip level (DRAW_SIMINFO_THRESHOLD).
const DRAW_SIMINFO_THRESHOLD: u32 = 3;
/// Zoom slider: log2(scale / 256), up to 4096 px per region.
const ZOOM_MAX: f32 = 4.0;
const ZOOM_MIN: f32 = -8.0;
/// Zoom animation (MAP_ZOOM_ACCELERATION_TIME / MAP_ZOOM_MAX_INTERP).
const ZOOM_TIME: f32 = 0.3;
const ZOOM_MAX_INTERP: f32 = 0.5;
/// Pan smoothing time constants (MAP_ITERP_TIME_CONSTANT, centering).
const PAN_TAU: f32 = 0.75;
const PAN_TAU_CENTER: f32 = 0.1;
/// Items clickable within this many points.
const ITEM_PICK: f32 = 6.0;

/// Kinds of map items drawn as icons.
#[derive(Clone, Copy, PartialEq)]
enum ItemKind {
    Infohub,
    Telehub,
    ForSale,
    ForSaleAdult,
    EventPg,
    EventMature,
    EventAdult,
}

pub struct WorldMapUi {
    /// Pan in points (y down) and its smoothed target.
    pan: Vec2,
    target_pan: Vec2,
    pan_tau: f32,
    /// log2(pixels per region / 256), animated towards `target_zoom`.
    zoom: f32,
    target_zoom: f32,
    /// Pointer the zoom keeps fixed, relative to the view center.
    zoom_pivot: Option<Vec2>,
    was_open: bool,
    tiles_complete: bool,
    search: String,
    selected: Option<(u32, u32)>,
    /// Inventory landmark picked in the list (item id, asset id, name).
    landmark: Option<(Uuid, Uuid, String)>,
    last_scale: f32,
    /// The floating panels of the full-frame layout.
    ov: overlay::Overlay,
    /// Inventory thumbnails, avatar pictures and landmark folders to load
    /// (drained by the app).
    pub wanted_images: HashSet<Uuid>,
    pub wanted_pics: HashSet<Uuid>,
    pub want_folders: HashSet<Uuid>,
    /// AURORA_DEMO_MAP=legacy: the side panel without touching the setting.
    pub demo_style: Option<WorldMapStyle>,
}

impl Default for WorldMapUi {
    fn default() -> Self {
        WorldMapUi {
            pan: Vec2::ZERO,
            target_pan: Vec2::ZERO,
            pan_tau: PAN_TAU,
            zoom: -1.0,
            target_zoom: -1.0,
            zoom_pivot: None,
            was_open: false,
            tiles_complete: false,
            search: String::new(),
            selected: None,
            landmark: None,
            last_scale: 0.0,
            ov: Default::default(),
            wanted_images: HashSet::new(),
            wanted_pics: HashSet::new(),
            want_folders: HashSet::new(),
            demo_style: None,
        }
    }
}

pub enum WorldMapAction {
    TeleportHome,
    TeleportLandmark(Uuid),
    /// The place card's "Info": the profile of the place.
    OpenPlace(Source),
}

/// Pictures the full-frame map shows, by id: inventory thumbnails
/// (`wanted_images`) and avatar pictures (`wanted_pics`).
pub struct MapPictures<'a> {
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub avatars: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// What the resident did on the map itself this frame.
#[derive(Default)]
struct MapInput {
    /// A click, a drag or the wheel: the search lets go of its suggestions.
    touched: bool,
    dragging: bool,
    /// A landmark pin clicked (its index in the pins), double-clicked.
    pin: Option<(usize, bool)>,
}

impl WorldMapUi {
    /// AURORA_DEMO_MAP=search:<text>: the region search, as if typed.
    pub fn demo_search(&mut self, world: &mut World, text: &str) {
        self.search = text.to_owned();
        self.selected = None;
        world.map.search_region(text);
        self.ov.demo_search(text);
    }

    /// AURORA_DEMO_MAP=drawer:<landmarks|history|friends>: the drawer open
    /// on that tab.
    pub fn demo_drawer(&mut self, tab: &str) {
        self.ov.demo_drawer(tab);
    }

    /// Center the map on a global position (centerOnTarget).
    fn center_on(&mut self, world: &World, cam: &MapCamera, x: f64, y: f64) {
        let (cx, cy) = to_global(world, cam.pos);
        let s = 256.0 * self.zoom.exp2() / 256.0;
        self.target_pan = vec2(-((x - cx) as f32 * s).floor(), ((y - cy) as f32 * s).floor());
        self.pan_tau = PAN_TAU_CENTER;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        tiles: &mut MapTiles,
        cam: &MapCamera,
        opts: &mut MapSettings,
        style: WorldMapStyle,
        pictures: &MapPictures,
        open: &mut bool,
    ) -> Vec<WorldMapAction> {
        let mut actions = Vec::new();
        if !*open {
            self.was_open = false;
            return actions;
        }
        if !self.was_open {
            // LLFloaterWorldMap::onOpen: back on our position, items reloaded
            self.was_open = true;
            self.pan = Vec2::ZERO;
            self.target_pan = Vec2::ZERO;
            self.zoom = (opts.world_scale.max(1.0) / 256.0).log2().clamp(ZOOM_MIN, ZOOM_MAX);
            self.target_zoom = self.zoom;
            world.map.reload_items(false);
        }
        // llMapDestination's BEACON_FOCUS_MAP: centre on the place as soon as
        // its region is known (LLFloaterWorldMap autofocus, centerOnTarget)
        if world.map.focus_track
            && let Some((x, y)) = world.map.track.as_ref().filter(|t| !t.pending).map(|t| (t.x, t.y))
        {
            world.map.focus_track = false;
            self.center_on(world, cam, x, y);
        }
        let screen = ctx.content_rect();
        if self.demo_style.unwrap_or(style) == WorldMapStyle::Aurora {
            // the mockup's window: a map twice the side panel layout's
            let size = vec2(1040.0, 640.0);
            let default_pos = pos2((screen.width() - size.x).max(0.0) * 0.5, 60.0);
            Floater::new("worldmap", aurora_i18n::tr!("common-world-map"), default_pos, size)
                .help(aurora_i18n::tr!(
                    "ui-worldmap-click-place-marker-double-click-teleport-wheel-zoom-drag"
                ))
                .show(ctx, p, open, |ui| {
                    self.full_frame(ui, p, world, tiles, cam, opts, pictures, &mut actions);
                });
            return actions;
        }
        let default_pos = pos2((screen.width() - 700.0).max(0.0) * 0.5, 60.0);
        Floater::new("worldmap", aurora_i18n::tr!("common-world-map"), default_pos, vec2(700.0, 600.0))
            .help(aurora_i18n::tr!(
                "ui-worldmap-click-place-marker-double-click-teleport-wheel-zoom-drag"
            ))
            .show(ctx, p, open, |ui| {
                let avail = ui.available_size();
                let side = 236.0;
                // exact fit: anything wider would grow the window every frame
                let gap = ui.spacing().item_spacing.x;
                ui.horizontal_top(|ui| {
                    let map_size = vec2((avail.x - side - gap).max(120.0).floor(), avail.y.max(160.0).floor());
                    self.map(ui, p, world, tiles, cam, opts, map_size, None);
                    ui.vertical(|ui| {
                        ui.set_width(side);
                        ui.set_max_width(side);
                        ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                        let footer_id = egui::Id::unique("worldmap-position-height");
                        let footer_height = ui.ctx().data(|d| d.get_temp::<f32>(footer_id)).unwrap_or(180.0);
                        let scroll_height = (avail.y - footer_height - ui.spacing().item_spacing.y).max(0.0);
                        egui::ScrollArea::vertical()
                            .id_salt("worldmap-side")
                            .auto_shrink([false, false])
                            .max_height(scroll_height)
                            .show(ui, |ui| {
                                self.side_panel(ui, p, world, cam, opts, &mut actions);
                            });
                        let footer = ui.vertical(|ui| self.position_panel(ui, p, world, cam, &mut actions));
                        ui.ctx().data_mut(|d| d.insert_temp(footer_id, footer.response.rect.height()));
                    });
                });
            });
        actions
    }

    #[allow(clippy::too_many_arguments)]
    fn map(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        tiles: &mut MapTiles,
        cam: &MapCamera,
        opts: &mut MapSettings,
        size: Vec2,
        full_frame: Option<&[overlay::Pin<'_>]>,
    ) -> MapInput {
        // the full-frame layout: its labels, and the landmarks as pins
        let aurora = full_frame.is_some();
        let pins = full_frame.unwrap_or(&[]);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);

        // --- zoom (wheel around the pointer, animated) and pan ---
        let zoom_min = {
            // adjustZoomSliderBounds: at most 512 regions across
            let ppr = ((rect.width().min(rect.height()) / 512.0 / 0.2).floor() * 0.2).clamp(1.0, 128.0);
            (ppr / 256.0).log2().max(ZOOM_MIN)
        };
        let mut input = MapInput {
            touched: resp.drag_started() || resp.clicked(),
            dragging: resp.dragged(),
            pin: None,
        };
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                input.touched = true;
                self.target_zoom = (self.target_zoom + scroll / 50.0 / 3.0).clamp(zoom_min, ZOOM_MAX);
                self.zoom_pivot = resp.hover_pos().map(|m| m - rect.center());
            }
        }
        self.target_zoom = self.target_zoom.clamp(zoom_min, ZOOM_MAX);
        let old_scale = 256.0 * self.zoom.exp2();
        if (self.zoom - self.target_zoom).abs() > 0.005 {
            let k = (dt / ZOOM_TIME).min(ZOOM_MAX_INTERP);
            self.zoom += (self.target_zoom - self.zoom) * k;
        } else {
            self.zoom = self.target_zoom;
            self.zoom_pivot = None;
        }
        let scale = 256.0 * self.zoom.exp2();
        if scale != old_scale {
            // LLWorldMapView::setScale: the point under the pivot stays put
            let kk = scale / old_scale;
            let rel = self.zoom_pivot.unwrap_or(Vec2::ZERO);
            self.pan = self.pan * kk + rel * (1.0 - kk);
            self.target_pan = self.target_pan * kk + rel * (1.0 - kk);
        }
        opts.world_scale = scale;
        if resp.dragged() {
            self.pan += resp.drag_delta();
            self.target_pan = self.pan;
            self.pan_tau = PAN_TAU;
        } else {
            let k = 1.0 - (-dt / self.pan_tau).exp2();
            self.pan += (self.target_pan - self.pan) * k;
        }
        if scale != self.last_scale {
            self.tiles_complete = false;
            self.last_scale = scale;
        }

        let ppm = scale / 256.0;
        let view = View::new(rect.center() + self.pan, to_global(world, cam.pos), ppm, 0.0);
        let level = scale_to_level(scale);
        let info = level <= DRAW_SIMINFO_THRESHOLD;

        // --- ocean + tiles (coarser levels under the loading one) ---
        painter.rect_filled(rect, 0.0, OCEAN);
        let gmin = view.to_global(rect.left_bottom());
        let gmax = view.to_global(rect.right_top());
        if !self.tiles_complete {
            for l in (level + 1..=MAP_LEVELS).rev() {
                draw_level(&painter, tiles, &view, gmin, gmax, l, false);
            }
            if level > 1 {
                draw_level(&painter, tiles, &view, gmin, gmax, level - 1, false);
            }
        }
        self.tiles_complete = draw_level(&painter, tiles, &view, gmin, gmax, level, true);

        // --- regions: down ones, labels, people counts ---
        let my_sim = {
            let g = to_global(world, world.agent.position);
            world.map.sim_at_global(g.0, g.1).map(|(k, _)| k)
        };
        let mut visible = Vec::new();
        for (key, s) in &world.map.sims {
            let x0 = key.0 as f64 * 256.0;
            let y0 = key.1 as f64 * 256.0;
            let r = Rect::from_two_pos(view.to_screen(x0, y0), view.to_screen(x0 + s.size_x as f64, y0 + s.size_y as f64));
            if !r.intersects(rect) {
                continue;
            }
            visible.push(*key);
            if s.is_down() {
                painter.rect_filled(r, 0.0, DOWN_FILL);
            }
            if scale >= DRAW_TEXT_THRESHOLD && aurora && !world.rlva.has("showloc") {
                let n: i32 = world.map.agents.get(key).map(|v| v.iter().map(|a| a.2).sum()).unwrap_or(0) + i32::from(my_sim == Some(*key));
                let coords = opts.world_grid_coords.then_some(*key);
                region_label(&painter.with_clip_rect(r.intersect(rect)), p, r, s, n, coords);
            } else if scale >= DRAW_TEXT_THRESHOLD && !world.rlva.has("showloc") {
                let lp = painter.with_clip_rect(r.intersect(rect));
                let mut y = r.bottom() - 2.0;
                if opts.world_grid_coords {
                    text_shadow(
                        &lp,
                        pos2(r.left() + 3.0, y - 28.0),
                        egui::Align2::LEFT_BOTTOM,
                        &format!("({}, {})", key.0, key.1),
                        11.0,
                        Color32::WHITE,
                    );
                }
                let n: i32 = world.map.agents.get(key).map(|v| v.iter().map(|a| a.2).sum()).unwrap_or(0) + i32::from(my_sim == Some(*key));
                let second = if s.is_down() || n == 0 {
                    format!("({})", s.access_label())
                } else {
                    format!("({n} - {})", s.access_label())
                };
                text_shadow(
                    &lp,
                    pos2(r.left() + 3.0, y),
                    egui::Align2::LEFT_BOTTOM,
                    &second,
                    11.0,
                    Color32::WHITE,
                );
                y -= 14.0;
                lp.text(
                    pos2(r.left() + 4.0, y + 1.0),
                    egui::Align2::LEFT_BOTTOM,
                    &s.name,
                    egui::FontId::proportional(12.5),
                    Color32::from_black_alpha(200),
                );
                lp.text(
                    pos2(r.left() + 3.0, y),
                    egui::Align2::LEFT_BOTTOM,
                    &s.name,
                    egui::FontId::proportional(12.5),
                    Color32::WHITE,
                );
            }
        }
        if info && opts.world_people {
            for key in &visible {
                if !world.map.sims.get(key).is_some_and(|s| s.is_down()) {
                    world.map.want_agents(*key);
                }
            }
        }

        // --- items: hubs, land for sale, events ---
        let mut hits: Vec<(Pos2, ItemKind, usize)> = Vec::new();
        if info {
            for (kind, ty, on) in [
                (ItemKind::ForSale, map_item::LAND_FOR_SALE, opts.world_land_for_sale),
                (ItemKind::ForSaleAdult, map_item::LAND_FOR_SALE_ADULT, opts.world_land_for_sale),
                (ItemKind::EventPg, map_item::PG_EVENT, opts.world_events),
                (
                    ItemKind::EventMature,
                    map_item::MATURE_EVENT,
                    opts.world_events && opts.world_mature_events,
                ),
                (
                    ItemKind::EventAdult,
                    map_item::ADULT_EVENT,
                    opts.world_events && opts.world_adult_events,
                ),
                (ItemKind::Telehub, map_item::TELEHUB, true),
            ] {
                if !on {
                    continue;
                }
                let Some(list) = world.map.items.get(&ty) else {
                    continue;
                };
                for (i, it) in list.iter().enumerate() {
                    let kind = if ty == map_item::TELEHUB {
                        if it.extra2 != 0 { ItemKind::Infohub } else { ItemKind::Telehub }
                    } else {
                        kind
                    };
                    if (kind == ItemKind::Infohub && !opts.world_infohubs) || (kind == ItemKind::Telehub && !opts.world_telehubs) {
                        continue;
                    }
                    let sp = view.to_screen(it.x as f64, it.y as f64);
                    if !rect.contains(sp) {
                        continue;
                    }
                    let (icon, col) = item_icon(kind, p);
                    draw_icon(&painter, icon, sp, 16.0, col);
                    hits.push((sp, kind, i));
                }
            }
        }

        // --- landmarks of the inventory (the full-frame layout's layer) ---
        let mut pin_hits: Vec<(Pos2, usize)> = Vec::new();
        for (i, pin) in pins.iter().enumerate() {
            let sp = view.to_screen(pin.x, pin.y);
            if rect.expand(8.0).contains(sp) {
                pin_hits.push((draw_pin(&painter, p, sp), i));
            }
        }

        // --- home ---
        if let Some((hx, hy, _)) = world.map.home {
            let sp = view.to_screen(hx, hy);
            if rect.contains(sp) {
                draw_icon(&painter, "home-filled", sp, 16.0, Color32::from_rgb(255, 214, 102));
            }
        }

        // --- camera cone: white, fading out ---
        let dir = cam.dir.truncate();
        let dir = if dir.length() < 1e-3 { glam::Vec2::X } else { dir.normalize() };
        let apex = view.to_screen(view.cam.0, view.cam.1);
        let len = cam.far * ppm;
        let half = cam.far * (cam.hfov * 0.5).tan() * ppm;
        let fwd = vec2(dir.x, -dir.y);
        let left = vec2(-fwd.y, fwd.x);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(apex, Color32::from_white_alpha(64));
        mesh.colored_vertex(apex + fwd * len + left * half, Color32::from_white_alpha(5));
        mesh.colored_vertex(apex + fwd * len - left * half, Color32::from_white_alpha(5));
        mesh.add_triangle(0, 1, 2);
        painter.add(egui::Shape::mesh(mesh));

        // --- people (MAP_ITEM_AGENT_LOCATIONS, stacked dots) ---
        if info && opts.world_people {
            let k = super::colors::get();
            let col = super::colors::c(k.map_other);
            for key in &visible {
                for (x, y, n) in world.map.agents.get(key).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let sp = view.to_screen(*x as f64, *y as f64);
                    for i in 0..(*n).clamp(1, 20) {
                        painter.circle(
                            sp - vec2(0.0, 3.0 * i as f32),
                            3.0,
                            col,
                            Stroke::new(1.0, Color32::from_black_alpha(160)),
                        );
                    }
                }
            }
        }

        // --- ourselves ---
        let me = to_global(world, world.agent.position);
        let sp = view.to_screen(me.0, me.1);
        let me_col = super::colors::c(super::colors::get().map_me);
        if aurora && rect.shrink(2.0).contains(sp) {
            draw_heading(&painter, p, sp, dir);
        } else if rect.shrink(2.0).contains(sp) {
            draw_self(&painter, sp, 4.5, me_col);
        } else {
            draw_tracking(
                &painter,
                rect,
                sp,
                Color32::from_rgb(255, 204, 0),
                0.0,
                Some(aurora_i18n::tr!("ui-worldmap-you-are-here")),
            );
        }

        // --- tracking ---
        if let Some(t) = &world.map.track {
            let sp = view.to_screen(t.x, t.y);
            if aurora && rect.shrink(2.0).contains(sp) {
                // the full-frame layout's pin: its card says the rest
                let color = if t.pending {
                    ui.ctx().request_repaint();
                    let v = 0.5 + 0.5 * (std::f32::consts::PI * (ui.input(|i| i.time) as f32 % 2.0)).cos();
                    p.danger.gamma_multiply(0.4 + 0.6 * v)
                } else if t.invalid {
                    p.muted
                } else {
                    p.danger
                };
                draw_destination(&painter, sp, color);
            } else if t.pending {
                let v = 0.5 + 0.5 * (std::f32::consts::PI * (ui.input(|i| i.time) as f32 % 2.0)).cos();
                let col = Color32::from_rgb(0, (v * 127.0) as u8, (v * 255.0) as u8);
                draw_tracking(&painter, rect, sp, col, 0.0, Some(aurora_i18n::tr!("common-loading-2")));
                ui.ctx().request_repaint();
            } else if t.invalid {
                draw_tracking(
                    &painter,
                    rect,
                    sp,
                    Color32::from_rgb(0, 128, 255),
                    0.0,
                    Some(aurora_i18n::tr!("ui-worldmap-invalid-location")),
                );
            } else {
                draw_tracking(&painter, rect, sp, TRACK_COLOR, 0.0, Some(&t.label));
                // the rating of the region there (a llMapDestination, a
                // place link), left of its label as in the navigation bar
                if rect.shrink(2.0).contains(sp)
                    && let Some((_, sim)) = world.map.sim_at_global(t.x, t.y)
                {
                    let font = egui::FontId::proportional(11.0);
                    let label = painter.layout_no_wrap(t.label.clone(), font.clone(), Color32::WHITE);
                    let row = ui.fonts_mut(|f| f.row_height(&font));
                    let side = 12.0;
                    let at = pos2(sp.x - label.size().x * 0.5 - side - 4.0, sp.y + 9.0 + (row - side) * 0.5);
                    let badge = Rect::from_min_size(at, Vec2::splat(side));
                    widgets::paint_maturity_badge(&painter, p, super::bars::maturity_name(sim.access), badge, 2);
                }
            }
        }

        draw_directions(&painter, rect, 0.0);
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);

        // --- ask the regions on screen (updateVisibleBlocks, level <= 3) ---
        if info {
            let c = view.to_global(rect.center());
            let (cx, cy) = (c.0 / 256.0, c.1 / 256.0);
            let hw = (rect.width() * 0.5 / scale) as f64;
            let hh = (rect.height() * 0.5 / scale) as f64;
            world.map.want_regions(
                (cx - hw - 1.0) as i64,
                (cy - hh - 1.0) as i64,
                (cx + hw + 1.0) as i64,
                (cy + hh + 1.0) as i64,
            );
        }

        // --- tooltip ---
        let pointer = resp.hover_pos();
        let pin_at = |m: Pos2| pin_hits.iter().rev().find(|h| h.0.distance(m) < ITEM_PICK + 2.0).map(|h| h.1);
        if let Some(m) = pointer.filter(|_| !resp.dragged()) {
            let item = hits.iter().find(|h| h.0.distance(m) < ITEM_PICK).copied();
            let text = if let Some(i) = pin_at(m) {
                Some(pins[i].name.to_owned())
            } else if let Some((_, kind, i)) = item {
                world_ref_items(&world.map.items, item_type(kind), i).map(|it| item_tooltip_of(it, kind))
            } else {
                let g = view.to_global(m);
                world.map.sim_at_global(g.0, g.1).map(|(key, s)| {
                    let name = if world.rlva.has("showloc") {
                        aurora_i18n::tr!("rlva-hidden-region")
                    } else {
                        &s.name
                    };
                    let mut t = format!("{} ({})", name, s.access_label());
                    if !s.is_down() {
                        let n: i32 =
                            world.map.agents.get(&key).map(|v| v.iter().map(|a| a.2).sum()).unwrap_or(0) + i32::from(my_sim == Some(key));
                        if n > 0 {
                            t.push('\n');
                            t.push_str(&aurora_i18n::tr!("person-count", count = n));
                        }
                    }
                    for f in s.flag_labels() {
                        t.push('\n');
                        t.push_str(f);
                    }
                    t
                })
            };
            if let Some(t) = text {
                resp.clone().on_hover_text_at_pointer(t);
            }
        }

        // --- click: track an item or the spot; double-click: teleport ---
        let picked = resp.interact_pointer_pos().filter(|_| resp.clicked() || resp.double_clicked());
        if let Some(i) = picked.and_then(pin_at) {
            input.pin = Some((i, resp.double_clicked()));
        } else if let Some(m) = picked {
            self.landmark = None;
            let item = if info {
                hits.iter().find(|h| h.0.distance(m) < ITEM_PICK).copied()
            } else {
                None
            };
            let teleport = resp.double_clicked() && opts.world_double_click_tp;
            let z = world.agent.position.z;
            match item.filter(|h| !matches!(h.1, ItemKind::Infohub | ItemKind::Telehub)) {
                Some((_, kind, i)) => {
                    let ty = item_type(kind);
                    if let Some(it) = world.map.items.get(&ty).and_then(|l| l.get(i)).cloned() {
                        let z = if matches!(kind, ItemKind::EventPg | ItemKind::EventMature | ItemKind::EventAdult) {
                            it.extra2 as f32
                        } else {
                            z
                        };
                        world.map.track_location(it.x as f64, it.y as f64, z, teleport);
                        if let Some(t) = world.map.track.as_mut() {
                            t.label = it.name.clone();
                            t.tooltip = item_tooltip_of(&it, kind);
                        }
                    }
                }
                None => {
                    let (x, y) = view.to_global(m);
                    world.map.track_location(x, y, z, teleport);
                }
            }
        }
        input
    }

    fn clear_button(&mut self, ui: &mut egui::Ui, world: &mut World) -> egui::Response {
        let response = ui
            .add_enabled(
                world.map.track.is_some() || self.landmark.is_some(),
                egui::Button::new(aurora_i18n::tr!("ui-waypoint-remove")),
            )
            .on_hover_text(aurora_i18n::tr!("ui-waypoint-remove-help"));
        if response.clicked() {
            world.map.clear_track();
            // Cancel the old search too: clearing its selection alone lets
            // the exact-match result create the beacon again next frame.
            world.map.search = None;
            self.search.clear();
            self.landmark = None;
            self.selected = None;
            self.target_pan = Vec2::ZERO;
            self.pan_tau = PAN_TAU_CENTER;
        }
        response
    }

    /// LLFloaterWorldMap::onCoordinatesCommit: editing a position creates
    /// a destination. Without tracking, use our current region / position
    /// (onClearBtn's mSetToUserPosition), with editable fields throughout.
    fn coordinates(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World) -> Option<[egui::Response; 3]> {
        if world.rlva.has("showloc") {
            return None;
        }
        let tracked = world.map.track.clone();
        let (x, y, z) = if let Some(t) = &tracked {
            (t.x, t.y, t.z)
        } else {
            let (ox, oy) = world.main_origin()?;
            let pos = world.agent.position;
            (ox as f64 + pos.x as f64, oy as f64 + pos.y as f64, pos.z)
        };
        let ((ox, oy), name) = if let Some(((sx, sy), sim)) = world.map.sim_at_global(x, y) {
            ((sx as f64 * 256.0, sy as f64 * 256.0), sim.name.clone())
        } else {
            match &tracked {
                Some(t) => (
                    ((x / 256.0).floor() * 256.0, (y / 256.0).floor() * 256.0),
                    if t.pending {
                        aurora_i18n::tr!("common-loading-2")
                    } else {
                        aurora_i18n::tr!("ui-worldmap-invalid-location")
                    }
                    .to_owned(),
                ),
                None => {
                    let (ox, oy) = world.main_origin()?;
                    ((ox as f64, oy as f64), world.region_name())
                }
            }
        };
        ui.label(RichText::new(&name).size(12.5).color(p.ink));
        let (mut lx, mut ly, mut lz) = ((x - ox) as f32, (y - oy) as f32, z);
        let before = (lx, ly, lz);
        let responses = ui
            .horizontal(|ui| {
                [("X", &mut lx, 8191.0), ("Y", &mut ly, 8191.0), ("Z", &mut lz, 10000.0)].map(|(label, value, max)| {
                    ui.label(RichText::new(label).size(12.0).color(p.muted));
                    ui.add(
                        egui::DragValue::new(value)
                            .custom_parser(aurora_i18n::parse_number)
                            .range(0.0..=max)
                            .speed(1.0)
                            .max_decimals(0)
                            .update_while_editing(false),
                    )
                })
            })
            .inner;
        // TextEdit reports a change while a DragValue is still typing.
        // Track only a committed numeric change (or a drag of the value).
        if (lx, ly, lz) != before {
            self.landmark = None;
            world.map.track_location(ox + lx as f64, oy + ly as f64, lz, false);
            // Named events keep their title; a coordinate destination gets
            // the freshly formatted region / coordinates from track_location.
            if let Some(old) = tracked.filter(|t| !t.label.is_empty() && !t.label.starts_with(&name))
                && let Some(t) = &mut world.map.track
            {
                t.label = old.label;
            }
        }
        Some(responses)
    }

    fn friend_altitude_hint(ui: &mut egui::Ui, p: &Palette, world: &World) {
        if !world.rlva.has("showloc") && world.map.avatar.as_ref().is_some_and(|a| a.found && !a.altitude_known) {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-worldmap-friend-altitude-unknown"))
                    .size(12.0)
                    .color(p.muted),
            );
        }
    }

    fn side_panel(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        cam: &MapCamera,
        opts: &mut MapSettings,
        actions: &mut Vec<WorldMapAction>,
    ) {
        let heading = |ui: &mut egui::Ui, t: &str| {
            ui.add_space(4.0);
            ui.label(RichText::new(t).size(12.0).strong().color(p.violet_light));
            widgets::divider(ui, p);
        };
        let level = scale_to_level(256.0 * self.zoom.exp2());
        heading(ui, aurora_i18n::tr!("ui-worldmap-legend"));
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-me"))
                .on_hover_text(aurora_i18n::tr!("ui-worldmap-center-map-on-my-position"))
                .clicked()
            {
                self.target_pan = Vec2::ZERO;
                self.pan_tau = PAN_TAU_CENTER;
            }
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-home"))
                .on_hover_text(aurora_i18n::tr!("common-go-home"))
                .clicked()
            {
                actions.push(WorldMapAction::TeleportHome);
            }
        });
        ui.add_enabled_ui(level <= DRAW_SIMINFO_THRESHOLD, |ui| {
            legend_check(
                ui,
                p,
                &mut opts.world_people,
                aurora_i18n::tr!("common-people"),
                None,
                super::colors::c(super::colors::get().map_other),
            );
            legend_check(
                ui,
                p,
                &mut opts.world_infohubs,
                aurora_i18n::tr!("ui-worldmap-infohubs"),
                Some("info-circle-filled"),
                item_icon(ItemKind::Infohub, p).1,
            );
            legend_check(
                ui,
                p,
                &mut opts.world_telehubs,
                aurora_i18n::tr!("ui-worldmap-telehubs"),
                Some("building-broadcast-tower-filled"),
                item_icon(ItemKind::Telehub, p).1,
            );
            legend_check(
                ui,
                p,
                &mut opts.world_land_for_sale,
                aurora_i18n::tr!("common-land-for-sale"),
                Some("tag-filled"),
                item_icon(ItemKind::ForSale, p).1,
            );
            legend_check(
                ui,
                p,
                &mut opts.world_events,
                aurora_i18n::tr!("ui-worldmap-events"),
                Some("calendar-event-filled"),
                item_icon(ItemKind::EventPg, p).1,
            );
            ui.add_enabled_ui(opts.world_events, |ui| {
                ui.indent("wm-ev", |ui| {
                    legend_check(
                        ui,
                        p,
                        &mut opts.world_mature_events,
                        aurora_i18n::tr!("ui-worldmap-moderate"),
                        None,
                        item_icon(ItemKind::EventMature, p).1,
                    );
                    legend_check(
                        ui,
                        p,
                        &mut opts.world_adult_events,
                        aurora_i18n::tr!("ui-worldmap-adult"),
                        None,
                        item_icon(ItemKind::EventAdult, p).1,
                    );
                });
            });
        });
        if level > DRAW_SIMINFO_THRESHOLD {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-worldmap-zoom-in-to-see-details"))
                    .size(11.0)
                    .color(p.muted),
            );
        }

        heading(ui, aurora_i18n::tr!("ui-worldmap-find-on-map"));
        // landmarks of the inventory, sorted
        let mut lms: Vec<(Uuid, Uuid, String)> = world
            .inventory
            .items
            .values()
            .filter(|i| i.asset_type == 3 && !i.asset_id.is_nil())
            .map(|i| (i.id, i.asset_id, i.name.clone()))
            .collect();
        lms.sort_by_key(|l| l.2.to_lowercase());
        let current = self
            .landmark
            .as_ref()
            .map(|l| l.2.clone())
            .unwrap_or_else(|| aurora_i18n::tr!("ui-worldmap-my-landmarks").into());
        egui::ComboBox::from_id_salt("worldmap-landmarks")
            .selected_text(current)
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                if lms.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-worldmap-no-landmarks-in-inventory")).color(p.muted));
                }
                for l in lms {
                    let sel = self.landmark.as_ref().is_some_and(|c| c.0 == l.0);
                    if ui.selectable_label(sel, &l.2).clicked() {
                        world.map.clear_track();
                        self.landmark = Some(l);
                    }
                }
            });
        ui.add_space(4.0);
        let mut go = false;
        ui.horizontal(|ui| {
            let r = widgets::search_field(
                ui,
                &mut self.search,
                aurora_i18n::tr!("ui-worldmap-regions-by-name"),
                ui.available_width() - 84.0,
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-worldmap-search")).clicked() {
                go = true;
            }
        });
        if go && !self.search.trim().is_empty() {
            self.selected = None;
            world.map.search_region(&self.search);
        }
        if let Some((s, sent)) = world.map.search.clone() {
            let results: Vec<((u32, u32), String, u8)> = world
                .map
                .search_results()
                .into_iter()
                .map(|(k, i)| (k, i.name.clone(), i.access))
                .collect();
            if results.is_empty() {
                if sent.elapsed().as_secs_f32() > 3.0 {
                    ui.label(RichText::new(aurora_i18n::tr!("common-no-results")).size(12.0).color(p.muted));
                } else {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-worldmap-searching")).size(12.0).color(p.muted));
                }
            }
            // an exact name match is selected at once (updateSims)
            if self.selected.is_none()
                && let Some(r) = results.iter().find(|r| r.1.to_lowercase() == s).or(results.first())
            {
                let exact = r.1.to_lowercase() == s;
                self.selected = Some(r.0);
                if exact {
                    self.select_region(world, cam, r.0);
                }
            }
            egui::ScrollArea::vertical()
                .id_salt("worldmap-results")
                .max_height(120.0)
                .show(ui, |ui| {
                    for (key, name, access) in &results {
                        let sel = self.selected == Some(*key);
                        // the navigation bar's badge, else the state in text
                        // (offline, unknown)
                        let clicked = ui
                            .horizontal(|ui| {
                                let badge = super::widgets::maturity_badge_of(ui, p, *access, 14.0).is_some();
                                let text = if badge {
                                    name.to_string()
                                } else {
                                    format!("{name}  ·  {}", access_label(*access))
                                };
                                ui.selectable_label(sel, RichText::new(text).size(12.5).color(p.ink)).clicked()
                            })
                            .inner;
                        if clicked {
                            self.selected = Some(*key);
                            self.select_region(world, cam, *key);
                        }
                    }
                });
        }

        heading(ui, aurora_i18n::tr!("common-zoom"));
        let mut z = self.target_zoom;
        if ui
            .add(
                egui::Slider::new(&mut z, ZOOM_MIN..=ZOOM_MAX)
                    .custom_parser(aurora_i18n::parse_number)
                    .step_by(0.2)
                    .show_value(false),
            )
            .changed()
        {
            self.target_zoom = z;
            self.zoom_pivot = None;
        }
    }

    fn position_panel(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World, cam: &MapCamera, actions: &mut Vec<WorldMapAction>) {
        ui.add_space(4.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-worldmap-position"))
                .size(12.0)
                .strong()
                .color(p.violet_light),
        );
        widgets::divider(ui, p);
        if let Some((_, _, name)) = &self.landmark {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-worldmap-landmark", name = name.to_string()))
                    .size(12.0)
                    .color(p.ink),
            );
        }
        if self.coordinates(ui, p, world).is_none() {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-worldmap-click-on-the-map-to-choose-a-location"))
                    .size(12.0)
                    .color(p.muted),
            );
        }
        Self::friend_altitude_hint(ui, p, world);
        let tp_target = if let Some((_, asset, _)) = &self.landmark {
            Some(NetCommand::TeleportLandmark(*asset))
        } else {
            world
                .map
                .track
                .as_ref()
                .filter(|t| !t.pending && !t.invalid)
                .map(|t| teleport_command(t.x, t.y, t.z))
        };
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(tp_target.is_some(), egui::Button::new(aurora_i18n::tr!("common-teleport-3")))
                .clicked()
                && let Some(c) = tp_target.clone()
            {
                match c {
                    NetCommand::TeleportLandmark(a) => actions.push(WorldMapAction::TeleportLandmark(a)),
                    c => world.map.push(c),
                }
            }
            let slurl = world.map.track.as_ref().and_then(|t| {
                let ((sx, sy), s) = world.map.sim_at_global(t.x, t.y)?;
                Some(crate::slurl::make(
                    &s.name,
                    glam::Vec3::new((t.x - sx as f64 * 256.0) as f32, (t.y - sy as f64 * 256.0) as f32, t.z),
                ))
            });
            if ui
                .add_enabled(
                    slurl.is_some() && !world.rlva.has("showloc"),
                    egui::Button::new(aurora_i18n::tr!("common-copy-slurl")),
                )
                .clicked()
                && !world.rlva.has("showloc")
                && let Some(u) = slurl
            {
                ui.ctx().copy_text(u);
            }
            let tracked = world.map.track.as_ref().map(|t| (t.x, t.y));
            if ui
                .add_enabled(tracked.is_some(), egui::Button::new(aurora_i18n::tr!("ui-worldmap-show-selection")))
                .clicked()
                && let Some((x, y)) = tracked
            {
                self.center_on(world, cam, x, y);
            }
            self.clear_button(ui, world);
        });
    }

    /// A search result: track its middle and center the map on it.
    fn select_region(&mut self, world: &mut World, cam: &MapCamera, key: (u32, u32)) {
        let Some(s) = world.map.sims.get(&key).cloned() else {
            return;
        };
        self.landmark = None;
        let x = key.0 as f64 * 256.0 + s.size_x as f64 * 0.5;
        let y = key.1 as f64 * 256.0 + s.size_y as f64 * 0.5;
        world.map.set_track(Track {
            x,
            y,
            z: 0.0,
            label: location_label(&s.name, s.size_x as f64 * 0.5, s.size_y as f64 * 0.5, 0.0),
            tooltip: String::new(),
            pending: false,
            teleport: false,
            invalid: s.is_down(),
        });
        if self.target_zoom < -1.0 {
            self.target_zoom = -1.0;
        }
        self.center_on(world, cam, x, y);
    }
}

/// A region's label in the full-frame layout, at its bottom left: the
/// rating's badge before its name, then its people or its state, the grid
/// position above when asked.
fn region_label(
    painter: &egui::Painter,
    p: &Palette,
    r: Rect,
    s: &crate::world::worldmap::SimInfo,
    people: i32,
    coords: Option<(u32, u32)>,
) {
    let mid = r.bottom() - 12.0;
    let mut x = r.left() + 5.0;
    let badge = Rect::from_center_size(pos2(x + 6.5, mid), Vec2::splat(13.0));
    if !s.is_down() && widgets::paint_maturity_badge(painter, p, super::bars::maturity_name(s.access), badge, 2) {
        x = badge.right() + 4.0;
    }
    let font = egui::FontId::new(11.5, super::fonts::semibold(painter.ctx()));
    let name = painter.layout_no_wrap(s.name.clone(), font, Color32::WHITE);
    let top = mid - name.size().y / 2.0;
    painter.galley_with_override_text_color(pos2(x, top + 1.0), name.clone(), Color32::from_black_alpha(200));
    let end = x + name.size().x;
    painter.galley(pos2(x, top), name, Color32::WHITE);
    let state = if s.is_down() {
        format!(" · {}", s.access_label())
    } else if people > 0 {
        format!(" · {people}")
    } else {
        String::new()
    };
    if !state.is_empty() {
        text_shadow(
            painter,
            pos2(end, mid),
            egui::Align2::LEFT_CENTER,
            &state,
            10.5,
            Color32::from_white_alpha(190),
        );
    }
    if let Some((gx, gy)) = coords {
        text_shadow(
            painter,
            pos2(r.left() + 5.0, mid - 16.0),
            egui::Align2::LEFT_CENTER,
            &format!("({gx}, {gy})"),
            10.5,
            Color32::from_white_alpha(190),
        );
    }
}

/// Ourselves on the full-frame map: an arrow pointing where we look (the
/// camera cone's direction), in the navigation icon's shape.
fn draw_heading(painter: &egui::Painter, p: &Palette, at: Pos2, dir: glam::Vec2) {
    // screen axes: forward as the cone's, and its right
    let fwd = vec2(dir.x, -dir.y);
    let right = vec2(-fwd.y, fwd.x);
    let arrow = |scale: f32, color: Color32| {
        let pt = |f: f32, r: f32| at + fwd * f * scale + right * r * scale;
        // tip, right wing, notch, left wing
        let pts = [pt(9.0, 0.0), pt(-7.0, 6.5), pt(-3.5, 0.0), pt(-7.0, -6.5)];
        let mut mesh = egui::Mesh::default();
        for q in pts {
            mesh.colored_vertex(q, color);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        egui::Shape::mesh(mesh)
    };
    painter.add(arrow(1.25, Color32::from_black_alpha(150)));
    painter.add(arrow(1.0, p.teal));
}

/// The tracked spot on the full-frame map: a red pin whose tip is on it.
fn draw_destination(painter: &egui::Painter, at: Pos2, color: Color32) {
    let size = 24.0;
    let center = at - vec2(0.0, size * 0.42);
    if let Some(t) = super::icons::global("map-pin-filled") {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(
            t.id(),
            Rect::from_center_size(center + vec2(0.0, 1.5), Vec2::splat(size + 2.0)),
            uv,
            Color32::from_black_alpha(130),
        );
        painter.image(t.id(), Rect::from_center_size(center, Vec2::splat(size)), uv, color);
    }
}

/// A landmark of the inventory on the full-frame map: a pin whose tip is
/// on the spot. Returns the middle of the pin (where it is picked).
fn draw_pin(painter: &egui::Painter, p: &Palette, at: Pos2) -> Pos2 {
    let size = 14.0;
    let center = at - vec2(0.0, size * 0.45);
    if let Some(t) = super::icons::global("map-pin-filled") {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        // a dark edge, so that it reads over any tile
        painter.image(
            t.id(),
            Rect::from_center_size(center, Vec2::splat(size + 3.0)),
            uv,
            Color32::from_black_alpha(150),
        );
        painter.image(t.id(), Rect::from_center_size(center, Vec2::splat(size)), uv, p.rose);
    }
    center
}

/// Draw the tiles of one mip level over the view; true when every one is
/// loaded or known missing (LLWorldMapView::drawMipmapLevel).
fn draw_level(
    painter: &egui::Painter,
    tiles: &mut MapTiles,
    view: &View,
    gmin: (f64, f64),
    gmax: (f64, f64),
    level: u32,
    load: bool,
) -> bool {
    let tile_m = 256.0 * (1u32 << (level - 1)) as f64;
    let mut complete = true;
    let x0 = (gmin.0.max(0.0) / tile_m).floor() * tile_m;
    let y0 = (gmin.1.max(0.0) / tile_m).floor() * tile_m;
    let mut y = y0;
    while y < gmax.1 {
        let mut x = x0;
        while x < gmax.0 {
            let (gx, gy) = tile_origin((x / 256.0) as u32, (y / 256.0) as u32, level);
            match tiles.get(level, gx, gy, load) {
                Ok(t) => {
                    let r = Rect::from_two_pos(view.to_screen(x, y + tile_m), view.to_screen(x + tile_m, y));
                    painter.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                Err(missing) => complete &= missing,
            }
            x += tile_m;
        }
        y += tile_m;
    }
    complete
}

fn item_type(kind: ItemKind) -> u32 {
    match kind {
        ItemKind::Infohub | ItemKind::Telehub => map_item::TELEHUB,
        ItemKind::ForSale => map_item::LAND_FOR_SALE,
        ItemKind::ForSaleAdult => map_item::LAND_FOR_SALE_ADULT,
        ItemKind::EventPg => map_item::PG_EVENT,
        ItemKind::EventMature => map_item::MATURE_EVENT,
        ItemKind::EventAdult => map_item::ADULT_EVENT,
    }
}

fn item_icon(kind: ItemKind, p: &Palette) -> (&'static str, Color32) {
    match kind {
        ItemKind::Infohub => ("info-circle-filled", Color32::from_rgb(120, 200, 255)),
        ItemKind::Telehub => ("building-broadcast-tower-filled", Color32::from_rgb(200, 200, 255)),
        ItemKind::ForSale => ("tag-filled", Color32::from_rgb(255, 230, 0)),
        ItemKind::ForSaleAdult => ("tag-filled", Color32::from_rgb(255, 140, 0)),
        ItemKind::EventPg => ("calendar-event-filled", p.success),
        ItemKind::EventMature => ("calendar-event-filled", p.amber),
        ItemKind::EventAdult => ("calendar-event-filled", p.rose),
    }
}

fn draw_icon(painter: &egui::Painter, name: &str, at: Pos2, size: f32, tint: Color32) {
    let r = Rect::from_center_size(at, Vec2::splat(size));
    painter.circle_filled(at, size * 0.62, Color32::from_black_alpha(150));
    match super::icons::global(name) {
        Some(t) => {
            painter.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        None => {
            painter.circle_filled(at, size * 0.35, tint);
        }
    }
}

fn world_ref_items(items: &std::collections::HashMap<u32, Vec<aurora_net::MapItem>>, ty: u32, i: usize) -> Option<&aurora_net::MapItem> {
    items.get(&ty).and_then(|l| l.get(i))
}

/// Item tooltips (LLWorldMapView::handleToolTip).
fn item_tooltip_of(it: &aurora_net::MapItem, kind: ItemKind) -> String {
    match kind {
        ItemKind::ForSale | ItemKind::ForSaleAdult => {
            let area = it.extra;
            let price = it.extra2;
            if area > 0 {
                aurora_i18n::tr!(
                    "status-area-m-l-price-l-m",
                    value0 = it.name.to_string(),
                    area = format!("{}", area),
                    price = format!("{}", price),
                    value1 = aurora_i18n::number_text(&format!("{:.1}", price as f32 / area as f32))
                )
            } else {
                aurora_i18n::tr!(
                    "ui-worldmap-l-unknown-area",
                    value0 = it.name.to_string(),
                    price = format!("{}", price)
                )
            }
        }
        ItemKind::EventPg | ItemKind::EventMature | ItemKind::EventAdult => {
            // Extra = start time (Unix, UTC), shown in local 24 h time
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(it.extra.max(0) as u64);
            let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let (h, m) = ((secs / 3600) % 24, (secs / 60) % 60);
            aurora_i18n::tr!(
                "ui-worldmap-at-utc",
                value0 = it.name.to_string(),
                h = format!("{:02}", h),
                m = format!("{:02}", m)
            )
        }
        ItemKind::Infohub => aurora_i18n::tr!("ui-worldmap-infohub", value0 = it.name.to_string()),
        ItemKind::Telehub => aurora_i18n::tr!("ui-worldmap-telehub", value0 = it.name.to_string()),
    }
}

fn legend_check(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: &str, icon: Option<&str>, col: Color32) {
    ui.horizontal(|ui| {
        super::widgets::checkbox(ui, p, on, "");
        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        match icon {
            Some(name) => draw_icon(ui.painter(), name, r.center(), 14.0, col),
            None => {
                ui.painter()
                    .circle(r.center(), 4.0, col, Stroke::new(1.0, Color32::from_black_alpha(160)));
            }
        }
        ui.label(RichText::new(label).size(12.5).color(p.ink));
    });
}

#[cfg(test)]
mod coordinate_tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;

    struct Bench {
        ctx: egui::Context,
        map: WorldMapUi,
        world: World,
        fields: [Rect; 3],
        enabled: [bool; 3],
        remove: Rect,
        time: f64,
    }

    impl Bench {
        fn new() -> Self {
            let mut world = World::new(Arc::new(AvatarLibrary::load()));
            world.main_region = Some(aurora_net::origin_to_handle(256000, 256000));
            world.agent.position = glam::Vec3::new(32.0, 48.0, 70.0);
            world.map.apply_blocks(vec![aurora_net::MapBlock {
                x: 1000,
                y: 1000,
                name: "Coordinate Region".into(),
                access: 13,
                region_flags: 0,
                water_height: 20,
                agents: 0,
                map_image_id: Uuid::nil(),
                size_x: 256,
                size_y: 256,
            }]);
            let mut bench = Self {
                ctx: egui::Context::default(),
                map: WorldMapUi::default(),
                world,
                fields: [Rect::NOTHING; 3],
                enabled: [false; 3],
                remove: Rect::NOTHING,
                time: 0.0,
            };
            bench.frame(vec![]);
            bench
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            let p = crate::theme::Theme::default().palette();
            let ctx = self.ctx.clone();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let fields = self.map.coordinates(ui, &p, &mut self.world).expect("in-world coordinates");
                    self.fields = std::array::from_fn(|i| fields[i].rect);
                    self.enabled = std::array::from_fn(|i| fields[i].enabled());
                    self.remove = self.map.clear_button(ui, &mut self.world).rect;
                },
            );
            // This headless bench intentionally has no GPU texture consumer.
            output.textures_delta.clear();
        }

        fn type_value(&mut self, field: usize, value: &str) {
            let pos = self.fields[field].center();
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            self.frame(vec![]);
            self.frame(vec![egui::Event::Text(value.into())]);
        }

        fn commit(&mut self) {
            self.frame(vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]);
            self.frame(vec![]);
        }

        fn clear(&mut self) {
            let pos = self.remove.center();
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            self.frame(vec![]);
        }
    }

    #[test]
    fn coordinates_are_editable_without_a_beacon_and_commit_without_teleporting() {
        let mut b = Bench::new();
        assert!(b.enabled.into_iter().all(|enabled| enabled));
        assert!(b.world.map.track.is_none());
        b.type_value(2, "120");
        assert!(b.world.map.track.is_none(), "typing does not track partial input");
        b.commit();
        let t = b.world.map.track.as_ref().expect("typed destination");
        assert_eq!((t.x, t.y, t.z), (256032.0, 256048.0, 120.0));
        assert!(!t.pending && !t.invalid);
        assert!(b.world.map.take_commands().is_empty(), "coordinate entry does not teleport");
    }

    #[test]
    fn removing_the_beacon_cancels_search_and_allows_coordinate_entry_again() {
        let mut b = Bench::new();
        b.world.map.track_location(256100.0, 256110.0, 90.0, false);
        b.map.search = "Coordinate Region".into();
        b.map.selected = Some((1000, 1000));
        b.world.map.search = Some(("coordinate region".into(), std::time::Instant::now()));
        b.frame(vec![]);
        b.clear();
        assert!(b.world.map.track.is_none());
        assert!(b.world.map.search.is_none() && b.map.search.is_empty());
        assert!(b.enabled.into_iter().all(|enabled| enabled));
        b.type_value(0, "64");
        b.commit();
        let t = b.world.map.track.as_ref().expect("new typed destination");
        assert_eq!((t.x, t.y, t.z), (256064.0, 256048.0, 70.0));
        assert!(b.world.map.take_commands().is_empty());
    }

    #[test]
    fn coordinates_keep_the_main_region_origin_before_map_metadata_arrives() {
        let mut b = Bench::new();
        b.world.map.sims.clear();
        b.world.agent.position = glam::Vec3::new(500.0, 700.0, 30.0);
        b.frame(vec![]);
        b.type_value(0, "600");
        b.commit();
        let t = b.world.map.track.as_ref().expect("typed destination awaiting metadata");
        assert_eq!((t.x, t.y, t.z), (256600.0, 256700.0, 30.0));
        assert!(t.pending && !t.teleport);
    }
}
