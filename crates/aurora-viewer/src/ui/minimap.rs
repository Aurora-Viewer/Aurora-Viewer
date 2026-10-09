//! Mini-map (LLNetMap): the map server tile under each connected region,
//! rasterized objects and parcel lines refreshed a few times a second,
//! avatars with above / below markers, the camera cone, chat rings,
//! tracking beacon and cardinal letters; wheel zoom, drag pan with
//! auto-centering, north-up or camera-up rotation, double-click teleport
//! and Firestorm's context menu.
//!
//! Ported from Firestorm's llnetmap.cpp, llfloatermap.cpp and
//! LLViewerObjectList::renderObjectsForMap (originally LGPL 2.1, Linden Research, Inc.).

use super::context::{self, AvatarList, CtxAction};
use super::map_tiles::MapTiles;
use super::menu;
use crate::scene::Scene;
use crate::settings::MapSettings;
use crate::theme::Palette;
use crate::world::World;
use aurora_net::RegionHandle;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use glam::Vec3;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const MAP_SCALE_MIN: f32 = 32.0;
pub const MAP_SCALE_MAX: f32 = 4096.0;
const MAP_SCALE_FAR: f32 = 32.0;
const MAP_SCALE_MEDIUM: f32 = 128.0;
const MAP_SCALE_CLOSE: f32 = 256.0;
const MAP_SCALE_VERY_CLOSE: f32 = 1024.0;
/// Zoom per wheel click (4 %).
const MAP_SCALE_ZOOM_FACTOR: f32 = 1.04;
const MIN_DOT_RADIUS: f32 = 3.5;
const DOT_SCALE: f32 = 0.75;
/// FSMinimapPickScale.
const PICK_SCALE: f32 = 3.0;
/// MiniMapPrimMaxRadius / MiniMapPrimMaxVertDistance.
const PRIM_MAX_RADIUS: f32 = 16.0;
const PRIM_MAX_VERT_DISTANCE: f32 = 256.0;
/// Above / below markers past this height difference (LLWorldMapView::drawAvatar).
const HEIGHT_THRESHOLD: f32 = 7.0;
/// Objects image refresh period; parcel image re-centered past 3 m.
const OBJECTS_REFRESH: Duration = Duration::from_millis(500);
const PARCEL_RECENTER_M: f64 = 3.0;
/// Coarse locations at this height or above are unknown (COARSEUPDATE_MAX_Z).
const COARSE_MAX_Z: f32 = 1020.0;

// object colors (colors.xml, Firestorm Aurora theme; unset ones use the
// code defaults of renderObjectsForMap)
const OTHER_ABOVE: Color32 = Color32::from_rgb(61, 61, 61);
const OTHER_BELOW: Color32 = Color32::from_rgb(31, 31, 31);
const YOU_ABOVE: Color32 = Color32::from_rgb(0, 255, 255);
const YOU_BELOW: Color32 = Color32::from_rgb(0, 199, 199);
const GROUP_ABOVE: Color32 = Color32::from_rgb(255, 0, 255);
const GROUP_BELOW: Color32 = Color32::from_rgb(199, 0, 199);
const SCRIPTED: Color32 = Color32::from_rgb(255, 128, 0);
const TEMP_ON_REZ: Color32 = Color32::from_rgb(255, 128, 0);
const YOU_PHYSICAL: Color32 = Color32::from_rgb(255, 0, 0);
const OTHER_PHYSICAL: Color32 = Color32::from_rgb(0, 255, 0);
/// FSNetMapPhantomOpacity (used as a raw alpha byte).
const PHANTOM_ALPHA: u8 = 90;
/// MapTrackColor (Red).
pub const TRACK_COLOR: Color32 = Color32::from_rgb(186, 0, 31);
/// MapFrustumColor (White_10).
const FRUSTUM: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26);
/// Cardinal letters (1, 1, 1, 0.7).
const LETTERS: Color32 = Color32::from_rgba_premultiplied(179, 179, 179, 179);
/// MapParcelOutlineColor.
const PARCEL_LINE: [u8; 4] = [255, 255, 255, 255];
const PARCEL_FOR_SALE: [u8; 4] = [255, 255, 128, 192];
const PARCEL_AUCTION: [u8; 4] = [128, 0, 255, 102];
/// Cells of the collision parcel (ban lines), MiniMapCollisionParcels.
const PARCEL_COLLISION: [u8; 4] = [255, 128, 128, 192];

// object flags (object_flags.h)
const FLAGS_USE_PHYSICS: u32 = 1 << 0;
const FLAGS_OBJECT_YOU_OWNER: u32 = 1 << 5;
const FLAGS_SCRIPTED: u32 = 1 << 6;
const FLAGS_PHANTOM: u32 = 1 << 10;
const FLAGS_OBJECT_GROUP_OWNED: u32 = 1 << 18;
const FLAGS_TEMPORARY_ON_REZ: u32 = 1 << 29;

/// Camera seen by the maps (render space).
pub struct MapCamera {
    pub pos: Vec3,
    pub dir: Vec3,
    /// Horizontal field of view (radians) and draw distance.
    pub hfov: f32,
    pub far: f32,
}

pub enum MiniMapAction {
    /// Teleport to a global position (tracking it first).
    Teleport {
        x: f64,
        y: f64,
        z: f32,
    },
    /// Track a global position.
    Track {
        x: f64,
        y: f64,
        z: f32,
    },
    StopTracking,
    OpenWorldMap,
    /// "Voir le profil" (LLNetMap::handleShowProfile).
    Profile(Uuid),
    /// "À propos du terrain" on the parcel under the click (popupShowAboutLand).
    AboutLand(f64, f64),
    /// Chat ring options changed (saved with the color settings).
    Colors(crate::settings::ColorSettings),
    /// "Fermer la mini-carte" (the floater has no close button).
    Close,
}

/// A CPU raster drawn as a rotated quad (objects, parcel lines).
struct Raster {
    tex: egui::TextureHandle,
    /// Global position of the image center.
    center: (f64, f64),
    /// Texels per meter and image side.
    tpm: f32,
    size: usize,
    built: Instant,
    scale: f32,
    generation: u64,
}

/// Height-shaded terrain shown under a region until (or instead of) its
/// map server tile (offline demo, grids without a map server).
struct LocalTerrain {
    tex: egui::TextureHandle,
    received: u32,
    built: Instant,
}

#[derive(Default)]
pub struct MiniMap {
    /// Pan in screen points (y down).
    pan: Vec2,
    recenter: bool,
    objects: Option<Raster>,
    parcels: Option<Raster>,
    terrain: HashMap<RegionHandle, LocalTerrain>,
    /// Avatars marked from the context menu (Mark ▸ color).
    marks: HashMap<Uuid, Color32>,
    /// Right click: global position and avatars under the pointer.
    popup: Option<((f64, f64), Vec<Uuid>)>,
    /// Shift + drag pan in progress.
    dragging: bool,
}

/// Map frame: global meters <-> screen points around the camera.
pub struct View {
    pub center: Pos2,
    pub cam: (f64, f64),
    pub ppm: f32,
    cos: f32,
    sin: f32,
}

impl View {
    /// `rot`: map rotation (radians, counter-clockwise).
    pub fn new(center: Pos2, cam: (f64, f64), ppm: f32, rot: f32) -> View {
        View {
            center,
            cam,
            ppm,
            cos: rot.cos(),
            sin: rot.sin(),
        }
    }

    pub fn to_screen(&self, gx: f64, gy: f64) -> Pos2 {
        let dx = (gx - self.cam.0) as f32 * self.ppm;
        let dy = (gy - self.cam.1) as f32 * self.ppm;
        let rx = dx * self.cos - dy * self.sin;
        let ry = dx * self.sin + dy * self.cos;
        pos2(self.center.x + rx, self.center.y - ry)
    }

    pub fn to_global(&self, p: Pos2) -> (f64, f64) {
        let rx = p.x - self.center.x;
        let ry = self.center.y - p.y;
        let dx = rx * self.cos + ry * self.sin;
        let dy = -rx * self.sin + ry * self.cos;
        (self.cam.0 + (dx / self.ppm) as f64, self.cam.1 + (dy / self.ppm) as f64)
    }
}

/// Render-space position -> global meters.
pub fn to_global(world: &World, p: Vec3) -> (f64, f64) {
    let (ox, oy) = world.main_origin().unwrap_or((0, 0));
    (ox as f64 + p.x as f64, oy as f64 + p.y as f64)
}

/// Draw a texture on a quad given by its NW, NE, SE, SW corners.
pub fn textured_quad(painter: &egui::Painter, tex: egui::TextureId, corners: [Pos2; 4], tint: Color32) {
    let mut m = egui::Mesh::with_texture(tex);
    for (p, uv) in corners.iter().zip([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)]) {
        m.vertices.push(egui::epaint::Vertex { pos: *p, uv, color: tint });
    }
    m.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(m));
}

/// Avatars to show: loaded ones at their exact place, the others from the
/// coarse locations. (id, render position, height known).
pub fn avatars(world: &World) -> Vec<(Uuid, Vec3, bool)> {
    let now = Instant::now();
    let mut out: Vec<(Uuid, Vec3, bool)> = Vec::new();
    for (idx, o) in world.objects.iter() {
        if o.is_avatar()
            && o.full_id != world.agent_id
            && let Some((p, _, _)) = Scene::object_transform(world, idx, now, 0)
        {
            out.push((o.full_id, p, true));
        }
    }
    for (h, list) in &world.coarse {
        let Some(off) = world.region_offset(*h) else {
            continue;
        };
        for (id, p) in list {
            if *id == world.agent_id || out.iter().any(|e| e.0 == *id) {
                continue;
            }
            let known = p.z > 0.0 && p.z < COARSE_MAX_Z;
            out.push((*id, off + *p, known));
        }
    }
    out
}

pub fn avatar_name(world: &World, id: &Uuid) -> String {
    world
        .avatar_name(id)
        .or_else(|| world.social.names.get(id).cloned())
        .unwrap_or_else(|| "…".into())
}

/// LLWorldMapView::drawAvatar: a dot, with a cap above / below when the
/// avatar is more than 7 m higher / lower than the camera.
pub fn draw_avatar(painter: &egui::Painter, c: Pos2, r: f32, color: Color32, dz: Option<f32>) {
    let outline = Stroke::new(1.0, Color32::from_black_alpha(160));
    match dz {
        Some(d) if d > HEIGHT_THRESHOLD => {
            painter.circle(c, r * 0.8, color, outline);
            let t = [
                pos2(c.x - r, c.y - r * 0.55),
                pos2(c.x + r, c.y - r * 0.55),
                pos2(c.x, c.y - r * 1.75),
            ];
            painter.add(egui::Shape::convex_polygon(t.to_vec(), color, outline));
        }
        Some(d) if d < -HEIGHT_THRESHOLD => {
            painter.circle(c, r * 0.8, color, outline);
            let t = [
                pos2(c.x - r, c.y + r * 0.55),
                pos2(c.x, c.y + r * 1.75),
                pos2(c.x + r, c.y + r * 0.55),
            ];
            painter.add(egui::Shape::convex_polygon(t.to_vec(), color, outline));
        }
        Some(_) => {
            painter.circle(c, r, color, outline);
        }
        None => {
            painter.circle(c, r, color.gamma_multiply(0.55), Stroke::new(1.0, color));
        }
    }
}

/// Our own marker (map_avatar_you_32): ringed dot.
pub fn draw_self(painter: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    painter.circle(c, r + 1.0, color, Stroke::new(1.5, Color32::WHITE));
}

/// LLWorldMapView::drawTrackingDot / drawTracking: the beacon in the frame,
/// or an arrow on the border pointing at it.
pub fn draw_tracking(painter: &egui::Painter, frame: Rect, at: Pos2, color: Color32, dz: f32, label: Option<&str>) {
    if frame.shrink(2.0).contains(at) {
        if dz.abs() <= HEIGHT_THRESHOLD {
            painter.circle_stroke(at, 6.0, Stroke::new(2.0, color));
            painter.circle_filled(at, 2.5, color);
        } else {
            // chevron pointing up (target above) or down
            let s = if dz > 0.0 { -1.0 } else { 1.0 };
            let pts = vec![
                pos2(at.x - 5.0, at.y - 2.5 * s),
                pos2(at.x, at.y + 2.5 * s),
                pos2(at.x + 5.0, at.y - 2.5 * s),
            ];
            painter.add(egui::Shape::line(pts, Stroke::new(3.0, color)));
        }
        if let Some(l) = label {
            text_shadow(painter, pos2(at.x, at.y + 9.0), egui::Align2::CENTER_TOP, l, 11.0, Color32::WHITE);
        }
        return;
    }
    // off frame: arrow on the border towards the target
    let c = frame.center();
    let d = at - c;
    if d.length() < 1.0 {
        return;
    }
    let inner = frame.shrink(9.0);
    let t = ((inner.width() * 0.5) / d.x.abs().max(1e-3)).min((inner.height() * 0.5) / d.y.abs().max(1e-3));
    let tip = c + d * t;
    let dir = d.normalized();
    let side = vec2(-dir.y, dir.x);
    let pts = [
        tip + dir * 7.0,
        tip - dir * 5.0 + side * 6.0,
        tip - dir * 2.0,
        tip - dir * 5.0 - side * 6.0,
    ];
    painter.add(egui::Shape::convex_polygon(pts[..3].to_vec(), color, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(vec![pts[0], pts[2], pts[3]], color, Stroke::NONE));
    if let Some(l) = label {
        let pos = (tip - dir * 14.0).clamp(frame.min + vec2(30.0, 8.0), frame.max - vec2(30.0, 18.0));
        text_shadow(painter, pos, egui::Align2::CENTER_CENTER, l, 11.0, Color32::WHITE);
    }
}

/// White label with a drop shadow (map labels).
pub fn text_shadow(painter: &egui::Painter, pos: Pos2, align: egui::Align2, text: &str, size: f32, color: Color32) -> Rect {
    let font = egui::FontId::proportional(size);
    painter.text(pos + vec2(1.0, 1.0), align, text, font.clone(), Color32::from_black_alpha(200));
    painter.text(pos, align, text, font, color)
}

/// Cardinal letters on the map border (LLFloaterMap::draw), `rot` = angle
/// of east (radians, counter-clockwise).
pub fn draw_directions(painter: &egui::Painter, rect: Rect, rot: f32) {
    let font = egui::FontId::proportional(12.0);
    let pad = painter.layout_no_wrap("N".into(), font.clone(), LETTERS).size().x * 0.5;
    let minor = 12.0 < 0.07 * rect.width().min(rect.height());
    let dirs: [(&str, f32, bool); 8] = [
        ("E", 0.0, true),
        ("NE", FRAC_PI_4, false),
        ("N", FRAC_PI_2, true),
        ("NO", 3.0 * FRAC_PI_4, false),
        ("O", PI, true),
        ("SO", 5.0 * FRAC_PI_4, false),
        ("S", 3.0 * FRAC_PI_2, true),
        ("SE", 7.0 * FRAC_PI_4, false),
    ];
    let c = rect.center();
    for (label, a, major) in dirs {
        if !major && !minor {
            continue;
        }
        let g = painter.layout_no_wrap(label.into(), font.clone(), LETTERS);
        let half_w = rect.width() * 0.5 - g.size().x * 0.5 - pad;
        let half_h = rect.height() * 0.5 - g.size().y * 0.5 - pad;
        let ang = (a + rot).rem_euclid(TAU);
        let (s, co) = ang.sin_cos();
        // ray from the center to the shrunk border
        let tx = if co.abs() > 1e-4 { half_w / co.abs() } else { f32::MAX };
        let ty = if s.abs() > 1e-4 { half_h / s.abs() } else { f32::MAX };
        let t = tx.min(ty);
        let p = pos2(c.x + co * t, c.y - s * t);
        painter.galley(p - g.size() * 0.5, g, LETTERS);
    }
}

impl MiniMap {
    /// The floater (Ctrl+Shift+M).
    #[allow(clippy::too_many_arguments)]
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        tiles: &mut MapTiles,
        cam: &MapCamera,
        opts: &mut MapSettings,
        open: &mut bool,
    ) -> Vec<MiniMapAction> {
        let mut actions = Vec::new();
        if !*open {
            return actions;
        }
        super::sound_cues::floater_shown(ctx, egui::Id::new("minimap"));
        // floater_map.xml: no header nor border, DkGray background at
        // FSMiniMapOpacity (0.66); dragged anywhere, resized by its corner
        let screen = ctx.content_rect();
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(32, 32, 32, 168))
            .corner_radius(egui::CornerRadius::same(2))
            .inner_margin(egui::Margin::same(2));
        egui::Window::new("Mini-carte")
            .id(egui::Id::new("minimap"))
            .title_bar(false)
            .frame(frame)
            .fade_in(false)
            .fade_out(false)
            .default_pos(pos2(screen.right() - 216.0, 56.0))
            .default_size(vec2(200.0, 200.0))
            .min_size(vec2(64.0, 64.0))
            .resizable(true)
            .constrain(true)
            .show(ctx, |ui| {
                let size = ui.available_size().max(vec2(60.0, 60.0));
                actions = self.draw(ui, p, world, tiles, cam, opts, size, true);
            });
        if actions.iter().any(|a| matches!(a, MiniMapAction::Close)) {
            *open = false;
        }
        actions
    }

    /// Draw the map in `size` (`interactive`: wheel, drag, menus).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        tiles: &mut MapTiles,
        cam: &MapCamera,
        opts: &mut MapSettings,
        size: Vec2,
        interactive: bool,
    ) -> Vec<MiniMapAction> {
        let mut actions = Vec::new();
        // FIRE-32339: a plain drag moves the floater, Shift + drag pans the map
        let shift = ui.input(|i| i.modifiers.shift);
        let sense = if !interactive {
            Sense::hover()
        } else if shift || self.dragging {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };
        let (rect, resp) = ui.allocate_exact_size(size, sense);
        let painter = ui.painter_at(rect);
        let now = Instant::now();
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        opts.mini_scale = opts.mini_scale.clamp(MAP_SCALE_MIN, MAP_SCALE_MAX);

        // --- input: wheel zoom, drag pan, auto-center ---
        if interactive && resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old = opts.mini_scale;
                let clicks = -scroll / 50.0;
                opts.mini_scale = (old * MAP_SCALE_ZOOM_FACTOR.powf(-clicks)).clamp(MAP_SCALE_MIN, MAP_SCALE_MAX);
                // keep the pan on the same spot; zoom around the pointer
                // when the map does not re-center itself
                self.pan *= opts.mini_scale / old;
                if !opts.mini_auto_center
                    && let Some(m) = resp.hover_pos()
                {
                    let off = m - rect.center();
                    self.pan -= off * (opts.mini_scale / old) - off;
                }
            }
        }
        self.dragging = interactive && resp.dragged();
        if interactive && resp.hovered() && shift {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if self.dragging {
            self.pan += resp.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if opts.mini_auto_center || self.recenter {
            // LLNetMap::draw: pan = lerp(pan, 0, 1 - 2^(-dt / 0.1))
            let k = 1.0 - (-dt / 0.1f32).exp2();
            self.pan *= 1.0 - k;
            if self.pan.x.abs() < 0.5 && self.pan.y.abs() < 0.5 {
                self.pan = Vec2::ZERO;
                self.recenter = false;
            }
        }

        let scale = opts.mini_scale;
        let ppm = scale / 256.0;
        // MiniMapRotate: atan2(at.x, at.y) turns the camera direction up
        let rot = if opts.mini_rotate { cam.dir.x.atan2(cam.dir.y) } else { 0.0 };
        let view = View {
            center: rect.center() + self.pan,
            cam: to_global(world, cam.pos),
            ppm,
            cos: rot.cos(),
            sin: rot.sin(),
        };

        // --- region tiles (the floater's background shows elsewhere) ---
        let main = world.main_region;
        let mut regions: Vec<_> = world.regions.values().collect();
        regions.sort_by_key(|r| Some(r.handle) == main);
        for r in regions {
            let (ox, oy) = aurora_net::handle_to_origin(r.handle);
            let (w, h) = (r.heightmap.size_x.max(256), r.heightmap.size_y.max(256));
            let tint = if Some(r.handle) == main {
                Color32::WHITE
            } else {
                Color32::from_gray(204)
            };
            let corners = |x0: f64, y0: f64, sx: f64, sy: f64| {
                [
                    view.to_screen(x0, y0 + sy),
                    view.to_screen(x0 + sx, y0 + sy),
                    view.to_screen(x0 + sx, y0),
                    view.to_screen(x0, y0),
                ]
            };
            let bounds = Rect::from_points(&corners(ox as f64, oy as f64, w as f64, h as f64));
            if !bounds.intersects(rect) {
                continue;
            }
            let mut all = true;
            let mut ready = Vec::new();
            for ty in 0..h / 256 {
                for tx in 0..w / 256 {
                    let (gx, gy) = (ox / 256 + tx, oy / 256 + ty);
                    match tiles.get(1, gx, gy, true) {
                        Ok(t) => ready.push((t, gx, gy)),
                        Err(_) => all = false,
                    }
                }
            }
            if !all && let Some(t) = self.local_terrain(ui.ctx(), world, r, now) {
                textured_quad(&painter, t, corners(ox as f64, oy as f64, w as f64, h as f64), tint);
            }
            for (t, gx, gy) in ready {
                textured_quad(&painter, t.id(), corners(gx as f64 * 256.0, gy as f64 * 256.0, 256.0, 256.0), tint);
            }
        }

        // --- objects and parcel lines (CPU rasters, drawn rotated) ---
        let diag = rect.width().hypot(rect.height());
        let img = ((diag * 0.5).max(1.0).log2().ceil().exp2() as usize).clamp(64, 512);
        let cam_g = view.to_global(rect.center());
        if opts.mini_objects {
            let stale = self
                .objects
                .as_ref()
                .is_none_or(|r| r.scale != scale || r.size != img || now.duration_since(r.built) >= OBJECTS_REFRESH);
            if stale {
                let tpm = img as f32 / (diag / ppm);
                let ci = rasterize_objects(world, cam_g, img, tpm, opts);
                self.objects = Some(store_raster(
                    ui.ctx(),
                    self.objects.take(),
                    "minimap-objects",
                    ci,
                    cam_g,
                    tpm,
                    img,
                    scale,
                    0,
                ));
            }
            if let Some(r) = &self.objects {
                draw_raster(&painter, &view, r);
            }
        } else {
            self.objects = None;
        }
        if opts.mini_property_lines {
            let overlay_gen = world.map.overlay_generation;
            let stale = self.parcels.as_ref().is_none_or(|r| {
                r.scale != scale
                    || r.size != img
                    || r.generation != overlay_gen
                    || (r.center.0 - cam_g.0).hypot(r.center.1 - cam_g.1) > PARCEL_RECENTER_M
            });
            if stale {
                let tpm = img as f32 / (diag / ppm);
                let ci = rasterize_parcels(world, cam_g, img, tpm, opts.mini_for_sale, opts.mini_collision);
                self.parcels = Some(store_raster(
                    ui.ctx(),
                    self.parcels.take(),
                    "minimap-parcels",
                    ci,
                    cam_g,
                    tpm,
                    img,
                    scale,
                    overlay_gen,
                ));
            }
            if let Some(r) = &self.parcels {
                draw_raster(&painter, &view, r);
            }
        } else {
            self.parcels = None;
        }

        // --- avatars ---
        let dot_r = (DOT_SCALE * ppm).max(MIN_DOT_RADIUS);
        let pointer = resp.hover_pos();
        let mut closest: Option<(f32, Uuid, Vec3, bool)> = None;
        let mut under: Vec<Uuid> = Vec::new();
        for (id, pos, known) in avatars(world) {
            let g = to_global(world, pos);
            let sp = view.to_screen(g.0, g.1);
            if !rect.expand(dot_r).contains(sp) {
                continue;
            }
            let name = avatar_name(world, &id);
            let col = self
                .marks
                .get(&id)
                .copied()
                .unwrap_or_else(|| super::colors::map_color(world, &id, &name));
            // unknown height: shown above while the camera is under 1020 m
            let dz = if known {
                Some(pos.z - cam.pos.z)
            } else if cam.pos.z < COARSE_MAX_Z {
                Some(f32::MAX)
            } else {
                None
            };
            draw_avatar(&painter, sp, dot_r, col, dz);
            if let Some(m) = pointer {
                let d = sp.distance(m);
                if d < dot_r * PICK_SCALE {
                    under.push(id);
                    if closest.as_ref().is_none_or(|c| d < c.0) {
                        closest = Some((d, id, pos, known));
                    }
                }
            }
        }

        // --- tracking ---
        if let Some(t) = &world.map.track {
            let sp = view.to_screen(t.x, t.y);
            draw_tracking(&painter, rect, sp, TRACK_COLOR, t.z - cam.pos.z, None);
        }

        // --- self ---
        let k = super::colors::get();
        let me = world.agent.position;
        let gme = to_global(world, me);
        draw_self(&painter, view.to_screen(gme.0, gme.1), dot_r, super::colors::c(k.map_me));

        // --- chat rings: whisper 10 m, say 20 m, shout 100 m ---
        if k.map_ranges {
            let c = view.to_screen(gme.0, gme.1);
            for (on, r, col) in [
                (k.map_whisper_on, 10.0, k.map_whisper),
                (k.map_say_on, 20.0, k.map_say),
                (k.map_shout_on, 100.0, k.map_shout),
            ] {
                if on {
                    painter.circle_stroke(c, r * ppm - 1.0, Stroke::new(2.0, super::colors::c(col)));
                }
            }
        }

        // --- pick disk ---
        if interactive && let Some(m) = pointer {
            painter.circle_filled(m, dot_r * PICK_SCALE, FRUSTUM);
        }

        // --- camera cone: points up when rotating, else along the camera ---
        let hfov = cam.hfov.clamp(0.1, PI * 0.95);
        let radius = cam.far * ppm;
        let heading = if opts.mini_rotate { FRAC_PI_2 } else { cam.dir.y.atan2(cam.dir.x) };
        let steps = ((hfov * 40.0 / TAU).round() as usize).max(1);
        let apex = view.center;
        let mut pts = vec![apex];
        for i in 0..=steps {
            let a = heading - hfov * 0.5 + hfov * i as f32 / steps as f32;
            pts.push(pos2(apex.x + a.cos() * radius, apex.y - a.sin() * radius));
        }
        // a fan of triangles (the sector is convex only up to 180°)
        for w in pts[1..].windows(2) {
            painter.add(egui::Shape::convex_polygon(vec![apex, w[0], w[1]], FRUSTUM, Stroke::NONE));
        }

        draw_directions(&painter, rect, rot);

        if !interactive {
            return actions;
        }

        // --- tooltip ---
        if let Some((_, id, pos, known)) = closest.filter(|_| resp.hovered() && !resp.dragged()) {
            let d = if known {
                pos.distance(me)
            } else {
                pos.truncate().distance(me.truncate())
            };
            let name = avatar_name(world, &id);
            resp.clone().on_hover_text_at_pointer(format!("{name}\n(Distance : {d:.2} m)"));
        } else if let Some(m) = pointer.filter(|_| resp.hovered() && !resp.dragged()) {
            let g = view.to_global(m);
            let mut lines = Vec::new();
            let name = world
                .regions
                .values()
                .find(|r| {
                    let (ox, oy) = aurora_net::handle_to_origin(r.handle);
                    g.0 >= ox as f64 && g.1 >= oy as f64 && g.0 < (ox + r.heightmap.size_x) as f64 && g.1 < (oy + r.heightmap.size_y) as f64
                })
                .map(|r| r.name.clone())
                .or_else(|| world.map.sim_at_global(g.0, g.1).map(|(_, s)| s.name.clone()));
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                lines.push(format!("Région : {n}"));
            }
            match opts.mini_double_click {
                1 => lines.push("Double-cliquez pour ouvrir la carte".into()),
                2 => lines.push("Double-cliquez pour vous téléporter".into()),
                _ => {}
            }
            if !lines.is_empty() {
                resp.clone().on_hover_text_at_pointer(lines.join("\n"));
            }
        }

        // --- double click (FSNetMapDoubleClickAction) ---
        if resp.double_clicked()
            && let Some(m) = resp.interact_pointer_pos()
        {
            let (x, y) = view.to_global(m);
            let z = to_global_z(world, x, y);
            match opts.mini_double_click {
                1 => {
                    if world.map.track.is_none() {
                        actions.push(MiniMapAction::Track { x, y, z });
                    }
                    actions.push(MiniMapAction::OpenWorldMap);
                }
                2 => actions.push(MiniMapAction::Teleport { x, y, z }),
                _ => {}
            }
        }

        // --- context menu (menu_mini_map.xml) ---
        if resp.secondary_clicked()
            && let Some(m) = resp.interact_pointer_pos()
        {
            self.popup = Some((view.to_global(m), under.clone()));
        }
        menu::context_menu(&resp, p, |ui| {
            let Some(((px, py), avs)) = self.popup.clone() else {
                ui.close();
                return;
            };
            if let Some(&first) = avs.first() {
                if avs.len() == 1 {
                    if menu::item(ui, p, "user-circle", "Voir le profil") {
                        actions.push(MiniMapAction::Profile(first));
                    }
                } else {
                    // « Voir le profil ▸ » lists the avatars under the pointer
                    menu::submenu(ui, p, "user-circle", "Voir le profil", true, |ui| {
                        for id in &avs {
                            if menu::item(ui, p, "user", &avatar_name(world, id)) {
                                actions.push(MiniMapAction::Profile(*id));
                            }
                        }
                    });
                }
                menu::todo(ui, p, "users-three", "Ajouter au cercle");
                if menu::item(ui, p, "magnifying-glass-plus", "Zoomer") {
                    context::request(ui.ctx(), CtxAction::ZoomAvatar(first));
                }
                menu::todo(ui, p, "eye", "Regard vers l'avatar");
                menu::submenu(ui, p, "tag", "Marques", true, |ui| {
                    for (name, col) in [
                        ("Rouge", Color32::from_rgb(186, 0, 31)),
                        ("Vert", Color32::from_rgb(0, 255, 0)),
                        ("Bleu", Color32::from_rgb(0, 0, 255)),
                        ("Mauve", Color32::from_rgb(255, 0, 255)),
                        ("Jaune", Color32::from_rgb(255, 255, 201)),
                    ] {
                        if menu::swatch(ui, p, col, name) {
                            for id in &avs {
                                self.marks.insert(*id, col);
                            }
                        }
                    }
                    menu::separator(ui, p);
                    if menu::item(ui, p, "eraser", "Effacer la marque") {
                        for id in &avs {
                            self.marks.remove(id);
                        }
                    }
                    menu::separator(ui, p);
                    if menu::item(ui, p, "eraser", "Effacer toutes les marques") {
                        self.marks.clear();
                    }
                });
                // « Plus… »: the radar menu of the nearest avatar
                menu::submenu(ui, p, "list", "Plus…", true, |ui| {
                    context::avatar_list_menu(ui, p, world, first, AvatarList::Nearby)
                });
                menu::separator(ui, p);
            }
            if menu::item(ui, p, "target", "Suivre") {
                actions.push(MiniMapAction::Track {
                    x: px,
                    y: py,
                    z: to_global_z(world, px, py),
                });
            }
            if menu::item_if(ui, p, "x-circle", "Arrêter de suivre", world.map.track.is_some()) {
                actions.push(MiniMapAction::StopTracking);
            }
            menu::separator(ui, p);
            menu::submenu(ui, p, "magnifying-glass", "Zoom", true, |ui| {
                for (label, s) in [
                    ("Très proche", MAP_SCALE_VERY_CLOSE),
                    ("Proche", MAP_SCALE_CLOSE),
                    ("Moyen", MAP_SCALE_MEDIUM),
                    ("Distant", MAP_SCALE_FAR),
                ] {
                    if menu::check(ui, p, "magnifying-glass", label, opts.mini_scale == s, true) {
                        self.pan *= s / opts.mini_scale;
                        opts.mini_scale = s;
                    }
                }
            });
            menu::submenu(ui, p, "eye", "Afficher", true, |ui| {
                menu::toggle(ui, p, "cube", "Objets", &mut opts.mini_objects);
                menu::toggle(ui, p, "cube", "Objets physiques", &mut opts.mini_physical);
                menu::toggle(ui, p, "code", "Objets scriptés", &mut opts.mini_scripted);
                menu::toggle(ui, p, "clock-counter-clockwise", "Objets temporaires", &mut opts.mini_temp_on_rez);
                menu::separator(ui, p);
                menu::toggle(ui, p, "squares-four", "Limites de terrain", &mut opts.mini_property_lines);
                menu::toggle(ui, p, "tag", "Terrains à vendre", &mut opts.mini_for_sale);
                menu::toggle(ui, p, "prohibit", "Parcelles interdites", &mut opts.mini_collision);
            });
            if menu::check(ui, p, "compass", "Nord en haut", !opts.mini_rotate, true) {
                opts.mini_rotate = false;
            }
            if menu::check(ui, p, "camera", "Caméra en haut", opts.mini_rotate, true) {
                opts.mini_rotate = true;
            }
            if menu::check(ui, p, "crosshair-simple", "Centrage automatique", opts.mini_auto_center, true) {
                opts.mini_auto_center = !opts.mini_auto_center;
            }
            if menu::item_if(
                ui,
                p,
                "arrows-in",
                "Recentrer la carte",
                !opts.mini_auto_center && self.pan != Vec2::ZERO,
            ) {
                self.recenter = true;
            }
            menu::submenu(ui, p, "chats-circle", "Portée des discussions", true, |ui| {
                let mut k = super::colors::get();
                let before = k;
                menu::toggle(ui, p, "chats-circle", "Montrer la portée des discussions", &mut k.map_ranges);
                menu::separator(ui, p);
                let ranges = k.map_ranges;
                let rings = [
                    ("Montrer la portée des murmures (10 m)", &mut k.map_whisper_on),
                    ("Montrer la portée des discussions (20 m)", &mut k.map_say_on),
                    ("Montrer la portée des cris (100 m)", &mut k.map_shout_on),
                ];
                for (label, on) in rings {
                    if ranges {
                        menu::toggle(ui, p, "chat-text", label, on);
                    } else {
                        menu::item_if(ui, p, "chat-text", label, false);
                    }
                }
                if k != before {
                    super::colors::set(&k);
                    actions.push(MiniMapAction::Colors(k));
                }
            });
            // Aurora: what a double click does
            menu::submenu(ui, p, "mouse", "Double-clic", true, |ui| {
                for (v, icon, label) in [
                    (0u8, "x-circle", "Ne rien faire"),
                    (1, "map-trifold", "Ouvrir la carte du monde"),
                    (2, "navigation-arrow", "Se téléporter"),
                ] {
                    if menu::check(ui, p, icon, label, opts.mini_double_click == v, true) {
                        opts.mini_double_click = v;
                    }
                }
            });
            menu::separator(ui, p);
            if menu::item(ui, p, "info", "À propos du terrain") {
                actions.push(MiniMapAction::AboutLand(px, py));
            }
            menu::todo(ui, p, "map-pin", "Profil du lieu");
            if menu::item(ui, p, "map-trifold", "Carte du monde") {
                actions.push(MiniMapAction::OpenWorldMap);
            }
            menu::separator(ui, p);
            if menu::item(ui, p, "x", "Fermer la mini-carte") {
                actions.push(MiniMapAction::Close);
            }
        });
        actions
    }

    /// Height-shaded terrain of a region, rebuilt while patches arrive.
    fn local_terrain(&mut self, ctx: &egui::Context, world: &World, r: &crate::world::Region, now: Instant) -> Option<egui::TextureId> {
        let received = (r.heightmap.received_fraction() * 1000.0) as u32;
        if received == 0 {
            return None;
        }
        if let Some(t) = self.terrain.get(&r.handle)
            && (t.received == received || now.duration_since(t.built) < Duration::from_secs(1))
        {
            return Some(t.tex.id());
        }
        let water = r.info.as_ref().map(|i| i.water_height).unwrap_or(20.0);
        let (w, h) = (r.heightmap.size_x / 2, r.heightmap.size_y / 2);
        let mut px = Vec::with_capacity((w * h) as usize);
        for j in 0..h {
            for i in 0..w {
                // image row 0 = north
                let z = r.heightmap.sample(i as f32 * 2.0 + 1.0, (h - 1 - j) as f32 * 2.0 + 1.0);
                px.push(terrain_color(z, water));
            }
        }
        let ci = egui::ColorImage::new([w as usize, h as usize], px);
        let _ = world;
        let tex = match self.terrain.remove(&r.handle) {
            Some(mut t) => {
                t.tex.set(ci, egui::TextureOptions::LINEAR);
                t.tex
            }
            None => ctx.load_texture(format!("minimap-terrain-{}", r.handle), ci, egui::TextureOptions::LINEAR),
        };
        let id = tex.id();
        self.terrain.insert(r.handle, LocalTerrain { tex, received, built: now });
        Some(id)
    }

    /// Forget per-region data of regions that are gone.
    pub fn prune(&mut self, world: &World) {
        self.terrain.retain(|h, _| world.regions.contains_key(h));
    }
}

/// Ground under a global position, else our own height (teleport target).
fn to_global_z(world: &World, x: f64, y: f64) -> f32 {
    let (ox, oy) = world.main_origin().unwrap_or((0, 0));
    let p = Vec3::new((x - ox as f64) as f32, (y - oy as f64) as f32, 0.0);
    world
        .ground_height(p)
        .map(|h| h.max(world.main_water_height()) + 1.0)
        .unwrap_or(world.agent.position.z)
}

fn terrain_color(h: f32, water: f32) -> Color32 {
    if h < water {
        let d = ((water - h) / 12.0).clamp(0.0, 1.0);
        Color32::from_rgb((58.0 - 30.0 * d) as u8, (98.0 - 40.0 * d) as u8, (128.0 - 30.0 * d) as u8)
    } else {
        let t = ((h - water) / 30.0).clamp(0.0, 1.0);
        Color32::from_rgb((96.0 + 40.0 * t) as u8, (118.0 + 20.0 * t) as u8, (78.0 + 30.0 * t) as u8)
    }
}

#[allow(clippy::too_many_arguments)]
fn store_raster(
    ctx: &egui::Context,
    old: Option<Raster>,
    name: &str,
    ci: egui::ColorImage,
    center: (f64, f64),
    tpm: f32,
    size: usize,
    scale: f32,
    generation: u64,
) -> Raster {
    let tex = match old {
        Some(mut r) => {
            r.tex.set(ci, egui::TextureOptions::NEAREST);
            r.tex
        }
        None => ctx.load_texture(name, ci, egui::TextureOptions::NEAREST),
    };
    Raster {
        tex,
        center,
        tpm,
        size,
        built: Instant::now(),
        scale,
        generation,
    }
}

fn draw_raster(painter: &egui::Painter, view: &View, r: &Raster) {
    let half = (r.size as f32 / r.tpm * 0.5) as f64;
    let (cx, cy) = r.center;
    let corners = [
        view.to_screen(cx - half, cy + half),
        view.to_screen(cx + half, cy + half),
        view.to_screen(cx + half, cy - half),
        view.to_screen(cx - half, cy - half),
    ];
    textured_quad(painter, r.tex.id(), corners, Color32::WHITE);
}

/// LLViewerObjectList::renderObjectsForMap into a `size`² image centered on
/// `center` (global), `tpm` texels per meter.
fn rasterize_objects(world: &World, center: (f64, f64), size: usize, tpm: f32, opts: &MapSettings) -> egui::ColorImage {
    let mut img = egui::ColorImage::filled([size, size], Color32::TRANSPARENT);
    let Some((ox, oy)) = world.main_origin() else {
        return img;
    };
    let agent_z = world.agent.position.z;
    let water: HashMap<RegionHandle, f32> = world
        .regions
        .iter()
        .map(|(h, r)| (*h, r.info.as_ref().map(|i| i.water_height).unwrap_or(20.0)))
        .collect();
    let half_m = size as f64 / tpm as f64 * 0.5;
    for (_, o) in world.objects.iter() {
        if o.pcode != aurora_prim::params::LL_PCODE_VOLUME {
            continue;
        }
        let f = o.update_flags;
        let you = f & FLAGS_OBJECT_YOU_OWNER != 0;
        let physical = f & FLAGS_USE_PHYSICS != 0;
        let scripted = f & FLAGS_SCRIPTED != 0;
        let temp = f & FLAGS_TEMPORARY_ON_REZ != 0;
        let accent = (opts.mini_physical && physical) || (opts.mini_scripted && scripted) || (opts.mini_temp_on_rez && temp);
        // mMapObjects: own objects, big ones, highlighted kinds
        if !(you || o.scale.length() > 7.5 || accent) {
            continue;
        }
        // children follow their root; attachments are left out
        let mut local = o.position;
        if o.parent_id != 0 {
            let Some(par) = world.objects.parent_of(o).and_then(|i| world.objects.get(i)) else {
                continue;
            };
            if par.is_avatar() || par.parent_id != 0 {
                continue;
            }
            local = par.position + par.rotation * o.position;
        }
        if (local.z - agent_z).abs() > PRIM_MAX_VERT_DISTANCE {
            continue;
        }
        let Some(off) = world.region_offset(o.key.region) else {
            continue;
        };
        let gx = ox as f64 + (off.x + local.x) as f64;
        let gy = oy as f64 + (off.y + local.y) as f64;
        if (gx - center.0).abs() > half_m + 16.0 || (gy - center.1).abs() > half_m + 16.0 {
            continue;
        }
        let mut r = ((o.scale.x + o.scale.y) * 0.5 * 0.5 * 1.3).min(PRIM_MAX_RADIUS);
        if you || accent {
            r = r.max(2.0);
        }
        let below = local.z < water.get(&o.key.region).copied().unwrap_or(20.0);
        let mut col = if you {
            if f & FLAGS_OBJECT_GROUP_OWNED != 0 {
                if below { GROUP_BELOW } else { GROUP_ABOVE }
            } else if below {
                YOU_BELOW
            } else {
                YOU_ABOVE
            }
        } else if below {
            OTHER_BELOW
        } else {
            OTHER_ABOVE
        };
        if opts.mini_scripted && scripted {
            col = SCRIPTED;
        }
        if opts.mini_physical && physical {
            col = if you { YOU_PHYSICAL } else { OTHER_PHYSICAL };
        }
        if opts.mini_temp_on_rez && temp {
            col = TEMP_ON_REZ;
        }
        if f & FLAGS_PHANTOM != 0 {
            col = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), PHANTOM_ALPHA);
        }
        // renderPoint: a filled square of round(2 r tpm) texels
        let d = ((2.0 * r * tpm).round() as i64).max(1);
        let tx = ((gx - center.0) * tpm as f64 + size as f64 * 0.5) as i64;
        let ty = (size as f64 * 0.5 - (gy - center.1) * tpm as f64) as i64;
        let (neg, pos) = (d / 2, d - d / 2);
        let n = size as i64;
        for y in (ty - neg).max(0)..(ty + pos).min(n) {
            for x in (tx - neg).max(0)..(tx + pos).min(n) {
                img.pixels[(y * n + x) as usize] = col;
            }
        }
    }
    img
}

/// LLNetMap::renderPropertyLinesForRegion: south / west parcel lines of
/// every 4 m cell, north and east region borders, for sale cells filled.
fn rasterize_parcels(world: &World, center: (f64, f64), size: usize, tpm: f32, for_sale: bool, collision: bool) -> egui::ColorImage {
    let mut img = egui::ColorImage::filled([size, size], Color32::TRANSPARENT);
    let n = size as i64;
    let to_tex = |gx: f64, gy: f64| -> (i64, i64) {
        (
            ((gx - center.0) * tpm as f64 + size as f64 * 0.5).floor() as i64,
            (size as f64 * 0.5 - (gy - center.1) * tpm as f64).floor() as i64,
        )
    };
    let mut put = |x: i64, y: i64, c: [u8; 4]| {
        if x >= 0 && y >= 0 && x < n && y < n {
            img.pixels[(y * n + x) as usize] = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
        }
    };
    let half_m = size as f64 / tpm as f64 * 0.5;
    for (h, r) in &world.regions {
        let (ox, oy) = aurora_net::handle_to_origin(*h);
        let (w, hh) = (r.heightmap.size_x as f64, r.heightmap.size_y as f64);
        let (x0, y0) = (ox as f64, oy as f64);
        if x0 > center.0 + half_m || y0 > center.1 + half_m || x0 + w < center.0 - half_m || y0 + hh < center.1 - half_m {
            continue;
        }
        let banned = world.map.collision.as_ref().filter(|c| collision && c.handle == *h);
        if let Some((cx, cy, data)) = world.map.overlays.get(h) {
            for j in 0..*cy {
                for i in 0..*cx {
                    let b = data[(j * cx + i) as usize];
                    let gx = x0 + i as f64 * 4.0;
                    let gy = y0 + j as f64 * 4.0;
                    if gx > center.0 + half_m || gy > center.1 + half_m || gx + 4.0 < center.0 - half_m || gy + 4.0 < center.1 - half_m {
                        continue;
                    }
                    let (ax, ay) = to_tex(gx, gy + 4.0);
                    let (bx, by) = to_tex(gx + 4.0, gy);
                    let kind = b & 0x07;
                    let sale = for_sale && (kind == 4 || kind == 5);
                    if sale || banned.is_some_and(|c| c.cell(i, j, *cx)) {
                        let c = match kind {
                            4 if sale => PARCEL_FOR_SALE,
                            5 if sale => PARCEL_AUCTION,
                            _ => PARCEL_COLLISION,
                        };
                        for y in ay..by {
                            for x in ax..bx {
                                put(x, y, c);
                            }
                        }
                    }
                    if b & 0x80 != 0 {
                        // south line
                        for x in ax..=bx {
                            put(x, by, PARCEL_LINE);
                        }
                    }
                    if b & 0x40 != 0 {
                        // west line
                        for y in ay..=by {
                            put(ax, y, PARCEL_LINE);
                        }
                    }
                }
            }
        }
        // north and east region borders
        let (ax, ay) = to_tex(x0, y0 + hh);
        let (bx, by) = to_tex(x0 + w, y0);
        for x in ax..=bx {
            put(x, ay, PARCEL_LINE);
        }
        for y in ay..=by {
            put(bx, y, PARCEL_LINE);
        }
    }
    img
}

/// 3D beacon of the tracked location (LLTracker::renderBeacon): a red
/// column with its label and distance; tracking stops within 3 m.
pub fn draw_beacon(ctx: &egui::Context, world: &World, proj: &super::hud::Projector) {
    let Some(t) = &world.map.track else {
        return;
    };
    let Some((ox, oy)) = world.main_origin() else {
        return;
    };
    let base = Vec3::new((t.x - ox as f64) as f32, (t.y - oy as f64) as f32, t.z);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("track-beacon")));
    let d = base.distance(world.agent.position);
    let label = format!("{} ({:.0} m)", if t.label.is_empty() { "Destination" } else { &t.label }, d);
    let col = TRACK_COLOR;
    match (proj.project(base), proj.project(base + Vec3::Z * 200.0)) {
        (Some((a, _)), Some((b, _))) => {
            painter.line_segment([a, b], Stroke::new(4.0, col.gamma_multiply(0.55)));
            painter.line_segment([a, b], Stroke::new(1.5, col));
            text_shadow(
                &painter,
                a + vec2(0.0, -14.0),
                egui::Align2::CENTER_BOTTOM,
                &label,
                13.0,
                Color32::WHITE,
            );
        }
        (Some((a, _)), None) | (None, Some((a, _))) => {
            painter.circle_stroke(a, 8.0, Stroke::new(2.0, col));
            text_shadow(
                &painter,
                a + vec2(0.0, -12.0),
                egui::Align2::CENTER_BOTTOM,
                &label,
                13.0,
                Color32::WHITE,
            );
        }
        (None, None) => {
            // off screen or behind the camera: arrow at the screen border
            let screen = ctx.content_rect();
            let c = proj.view_proj * base.extend(1.0);
            let mut v = vec2(c.x, -c.y);
            if c.w < 0.0 {
                v = -v;
            }
            if v.length() > 1e-4 {
                let at = screen.center() + v.normalized() * screen.width() * 2.0;
                draw_tracking(&painter, screen.shrink(40.0), at, col, 0.0, Some(&label));
            }
        }
    }
}
