//! World map floater (LLFloaterWorldMap + LLWorldMapView): map server tiles
//! by mip level with coarser levels shown while the current one loads,
//! region names and access, people, hubs, land for sale and events,
//! home, our position and camera cone, the tracked location; wheel zoom
//! around the pointer, drag pan, click to track, double-click to teleport.
//! The right panel holds the legend, landmarks, region search, the
//! coordinates and the teleport / SLurl buttons.
//!
//! Ported from Firestorm's llworldmapview.cpp / llfloaterworldmap.cpp
//! (originally LGPL 2.1, Linden Research, Inc.).

use super::map_tiles::{MAP_LEVELS, MapTiles, scale_to_level, tile_origin};
use super::minimap::{MapCamera, TRACK_COLOR, View, draw_directions, draw_self, draw_tracking, text_shadow, to_global};
use super::widgets::{self, Floater};
use crate::settings::MapSettings;
use crate::theme::Palette;
use crate::world::World;
use crate::world::worldmap::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, Track, access_label, location_label, teleport_command};
use aurora_net::{NetCommand, map_item};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};
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
        }
    }
}

pub enum WorldMapAction {
    TeleportHome,
    TeleportLandmark(Uuid),
}

impl WorldMapUi {
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
        let screen = ctx.content_rect();
        let default_pos = pos2((screen.width() - 700.0).max(0.0) * 0.5, 60.0);
        Floater::new("worldmap", "Carte du monde", default_pos, vec2(700.0, 600.0))
            .help("Clic : placer un repère · double-clic : se téléporter · molette : zoom · glisser : déplacer")
            .show(ctx, p, open, |ui| {
                let avail = ui.available_size();
                let side = 236.0;
                // exact fit: anything wider would grow the window every frame
                let gap = ui.spacing().item_spacing.x;
                ui.horizontal_top(|ui| {
                    let map_size = vec2((avail.x - side - gap).max(120.0).floor(), avail.y.max(160.0).floor());
                    self.map(ui, p, world, tiles, cam, opts, map_size);
                    ui.vertical(|ui| {
                        ui.set_width(side);
                        ui.set_max_width(side);
                        ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                        egui::ScrollArea::vertical()
                            .id_salt("worldmap-side")
                            .max_height(avail.y)
                            .show(ui, |ui| {
                                self.side_panel(ui, p, world, cam, opts, &mut actions);
                            });
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
    ) {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);

        // --- zoom (wheel around the pointer, animated) and pan ---
        let zoom_min = {
            // adjustZoomSliderBounds: at most 512 regions across
            let ppr = ((rect.width().min(rect.height()) / 512.0 / 0.2).floor() * 0.2).clamp(1.0, 128.0);
            (ppr / 256.0).log2().max(ZOOM_MIN)
        };
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
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
            if scale >= DRAW_TEXT_THRESHOLD {
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

        // --- home ---
        if let Some((hx, hy, _)) = world.map.home {
            let sp = view.to_screen(hx, hy);
            if rect.contains(sp) {
                draw_icon(&painter, "house", sp, 16.0, Color32::from_rgb(255, 214, 102));
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
        if rect.shrink(2.0).contains(sp) {
            draw_self(&painter, sp, 4.5, me_col);
        } else {
            draw_tracking(&painter, rect, sp, Color32::from_rgb(255, 204, 0), 0.0, Some("Vous êtes ici"));
        }

        // --- tracking ---
        if let Some(t) = &world.map.track {
            let sp = view.to_screen(t.x, t.y);
            if t.pending {
                let v = 0.5 + 0.5 * (std::f32::consts::PI * (ui.input(|i| i.time) as f32 % 2.0)).cos();
                let col = Color32::from_rgb(0, (v * 127.0) as u8, (v * 255.0) as u8);
                draw_tracking(&painter, rect, sp, col, 0.0, Some("Chargement…"));
                ui.ctx().request_repaint();
            } else if t.invalid {
                draw_tracking(&painter, rect, sp, Color32::from_rgb(0, 128, 255), 0.0, Some("Lieu invalide"));
            } else {
                draw_tracking(&painter, rect, sp, TRACK_COLOR, 0.0, Some(&t.label));
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
        if let Some(m) = pointer.filter(|_| !resp.dragged()) {
            let item = hits.iter().find(|h| h.0.distance(m) < ITEM_PICK).copied();
            let text = if let Some((_, kind, i)) = item {
                world_ref_items(&world.map.items, item_type(kind), i).map(|it| item_tooltip_of(it, kind))
            } else {
                let g = view.to_global(m);
                world.map.sim_at_global(g.0, g.1).map(|(key, s)| {
                    let mut t = format!("{} ({})", s.name, s.access_label());
                    if !s.is_down() {
                        let n: i32 =
                            world.map.agents.get(&key).map(|v| v.iter().map(|a| a.2).sum()).unwrap_or(0) + i32::from(my_sim == Some(key));
                        if n == 1 {
                            t.push_str("\n1 personne");
                        } else if n > 1 {
                            t.push_str(&format!("\n{n} personnes"));
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
        if (resp.clicked() || resp.double_clicked())
            && let Some(m) = resp.interact_pointer_pos()
        {
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
            ui.separator();
        };
        let level = scale_to_level(256.0 * self.zoom.exp2());
        heading(ui, "Légende");
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, p, "Moi")
                .on_hover_text("Centrer la carte sur ma position")
                .clicked()
            {
                self.target_pan = Vec2::ZERO;
                self.pan_tau = PAN_TAU_CENTER;
            }
            if widgets::flat_button(ui, p, "Domicile").on_hover_text("Rentrer chez moi").clicked() {
                actions.push(WorldMapAction::TeleportHome);
            }
        });
        ui.add_enabled_ui(level <= DRAW_SIMINFO_THRESHOLD, |ui| {
            legend_check(
                ui,
                p,
                &mut opts.world_people,
                "Personnes",
                None,
                super::colors::c(super::colors::get().map_other),
            );
            legend_check(
                ui,
                p,
                &mut opts.world_infohubs,
                "Infohubs",
                Some("info"),
                item_icon(ItemKind::Infohub, p).1,
            );
            legend_check(
                ui,
                p,
                &mut opts.world_telehubs,
                "Telehubs",
                Some("broadcast"),
                item_icon(ItemKind::Telehub, p).1,
            );
            legend_check(
                ui,
                p,
                &mut opts.world_land_for_sale,
                "Terrains à vendre",
                Some("tag"),
                item_icon(ItemKind::ForSale, p).1,
            );
            legend_check(
                ui,
                p,
                &mut opts.world_events,
                "Événements",
                Some("calendar-dots"),
                item_icon(ItemKind::EventPg, p).1,
            );
            ui.add_enabled_ui(opts.world_events, |ui| {
                ui.indent("wm-ev", |ui| {
                    legend_check(
                        ui,
                        p,
                        &mut opts.world_mature_events,
                        "Modérés",
                        None,
                        item_icon(ItemKind::EventMature, p).1,
                    );
                    legend_check(
                        ui,
                        p,
                        &mut opts.world_adult_events,
                        "Adultes",
                        None,
                        item_icon(ItemKind::EventAdult, p).1,
                    );
                });
            });
        });
        if level > DRAW_SIMINFO_THRESHOLD {
            ui.label(RichText::new("Zoomez pour voir les détails.").size(11.0).color(p.muted));
        }

        heading(ui, "Trouver sur la carte");
        // landmarks of the inventory, sorted
        let mut lms: Vec<(Uuid, Uuid, String)> = world
            .inventory
            .items
            .values()
            .filter(|i| i.asset_type == 3 && !i.asset_id.is_nil())
            .map(|i| (i.id, i.asset_id, i.name.clone()))
            .collect();
        lms.sort_by_key(|l| l.2.to_lowercase());
        let current = self.landmark.as_ref().map(|l| l.2.clone()).unwrap_or_else(|| "Mes repères".into());
        egui::ComboBox::from_id_salt("worldmap-landmarks")
            .selected_text(current)
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                if lms.is_empty() {
                    ui.label(RichText::new("Aucun repère dans l'inventaire").color(p.muted));
                }
                for l in lms {
                    let sel = self.landmark.as_ref().is_some_and(|c| c.0 == l.0);
                    if ui.selectable_label(sel, &l.2).clicked() {
                        world.map.track = None;
                        self.landmark = Some(l);
                    }
                }
            });
        ui.add_space(4.0);
        let mut go = false;
        ui.horizontal(|ui| {
            let r = widgets::search_field(ui, &mut self.search, "Régions par nom", ui.available_width() - 84.0);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if widgets::flat_button(ui, p, "Chercher").clicked() {
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
                    ui.label(RichText::new("Aucun résultat.").size(12.0).color(p.muted));
                } else {
                    ui.label(RichText::new("Recherche…").size(12.0).color(p.muted));
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
                        let text = RichText::new(format!("{name}  ·  {}", access_label(*access)))
                            .size(12.5)
                            .color(match *access {
                                SIM_ACCESS_ADULT => p.rose,
                                SIM_ACCESS_MATURE => p.amber,
                                _ => p.ink,
                            });
                        if ui.selectable_label(sel, text).clicked() {
                            self.selected = Some(*key);
                            self.select_region(world, cam, *key);
                        }
                    }
                });
        }

        heading(ui, "Position");
        let mut tp_target: Option<NetCommand> = None;
        if let Some((_, asset, name)) = &self.landmark {
            ui.label(RichText::new(format!("Repère : {name}")).size(12.0).color(p.ink));
            tp_target = Some(NetCommand::TeleportLandmark(*asset));
        } else if let Some(t) = world.map.track.clone() {
            let region = world.map.sim_at_global(t.x, t.y).map(|(k, s)| (k, s.name.clone()));
            match &region {
                Some((_, n)) => ui.label(RichText::new(n).size(12.5).color(p.ink)),
                None => ui.label(
                    RichText::new(if t.pending { "Chargement…" } else { "Lieu invalide" })
                        .size(12.0)
                        .color(p.muted),
                ),
            };
            if let Some(((sx, sy), _)) = region {
                let (mut lx, mut ly, mut lz) = ((t.x - sx as f64 * 256.0) as f32, (t.y - sy as f64 * 256.0) as f32, t.z);
                let mut changed = false;
                ui.horizontal(|ui| {
                    for (label, v, max) in [("X", &mut lx, 8191.0), ("Y", &mut ly, 8191.0), ("Z", &mut lz, 10000.0)] {
                        ui.label(RichText::new(label).size(12.0).color(p.muted));
                        changed |= ui
                            .add(egui::DragValue::new(v).range(0.0..=max).speed(1.0).max_decimals(0))
                            .changed();
                    }
                });
                if changed {
                    let (x, y) = (sx as f64 * 256.0 + lx as f64, sy as f64 * 256.0 + ly as f64);
                    let label = t.label.clone();
                    world.map.track_location(x, y, lz, false);
                    let region = world.map.sim_at_global(x, y).map(|(_, s)| s.name.clone());
                    if let (Some(nt), Some(n)) = (world.map.track.as_mut(), region) {
                        nt.label = if label.starts_with(&n) {
                            location_label(&n, lx as f64, ly as f64, lz)
                        } else {
                            label
                        };
                    }
                }
                if !t.invalid {
                    tp_target = Some(teleport_command(t.x, t.y, t.z));
                }
            }
        } else {
            ui.label(
                RichText::new("Cliquez sur la carte pour choisir un lieu.")
                    .size(12.0)
                    .color(p.muted),
            );
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(tp_target.is_some(), egui::Button::new("Téléporter")).clicked()
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
            if ui.add_enabled(slurl.is_some(), egui::Button::new("Copier la SLurl")).clicked()
                && let Some(u) = slurl
            {
                ui.ctx().copy_text(u);
            }
            let tracked = world.map.track.as_ref().map(|t| (t.x, t.y));
            if ui
                .add_enabled(tracked.is_some(), egui::Button::new("Montrer la sélection"))
                .clicked()
                && let Some((x, y)) = tracked
            {
                self.center_on(world, cam, x, y);
            }
            if ui
                .add_enabled(tracked.is_some() || self.landmark.is_some(), egui::Button::new("Effacer"))
                .clicked()
            {
                world.map.track = None;
                self.landmark = None;
                self.selected = None;
                self.target_pan = Vec2::ZERO;
                self.pan_tau = PAN_TAU_CENTER;
            }
        });

        heading(ui, "Zoom");
        let mut z = self.target_zoom;
        if ui
            .add(egui::Slider::new(&mut z, ZOOM_MIN..=ZOOM_MAX).step_by(0.2).show_value(false))
            .changed()
        {
            self.target_zoom = z;
            self.zoom_pivot = None;
        }
    }

    /// A search result: track its middle and center the map on it.
    fn select_region(&mut self, world: &mut World, cam: &MapCamera, key: (u32, u32)) {
        let Some(s) = world.map.sims.get(&key).cloned() else {
            return;
        };
        self.landmark = None;
        let x = key.0 as f64 * 256.0 + s.size_x as f64 * 0.5;
        let y = key.1 as f64 * 256.0 + s.size_y as f64 * 0.5;
        world.map.track = Some(Track {
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
        ItemKind::Infohub => ("info", Color32::from_rgb(120, 200, 255)),
        ItemKind::Telehub => ("broadcast", Color32::from_rgb(200, 200, 255)),
        ItemKind::ForSale => ("tag", Color32::from_rgb(255, 230, 0)),
        ItemKind::ForSaleAdult => ("tag", Color32::from_rgb(255, 140, 0)),
        ItemKind::EventPg => ("calendar-dots", p.success),
        ItemKind::EventMature => ("calendar-dots", p.amber),
        ItemKind::EventAdult => ("calendar-dots", p.rose),
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
                format!("{}\n{area} m² L$ {price} ({:.1} L$/m²)", it.name, price as f32 / area as f32)
            } else {
                format!("{}\nL$ {price} (surface inconnue)", it.name)
            }
        }
        ItemKind::EventPg | ItemKind::EventMature | ItemKind::EventAdult => {
            // Extra = start time (Unix, UTC), shown in local 24 h time
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(it.extra.max(0) as u64);
            let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let (h, m) = ((secs / 3600) % 24, (secs / 60) % 60);
            format!("{}\nÀ {h:02}:{m:02} UTC", it.name)
        }
        ItemKind::Infohub => format!("Infohub : {}", it.name),
        ItemKind::Telehub => format!("Telehub : {}", it.name),
    }
}

fn legend_check(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: &str, icon: Option<&str>, col: Color32) {
    ui.horizontal(|ui| {
        ui.checkbox(on, "");
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
