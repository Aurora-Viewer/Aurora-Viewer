//! Markers over contact-set members (`world/contact_markers.rs` says who
//! and where): a glass diamond with a border and a symbol in the set's
//! colour, seen through everything in the world. Off screen or behind the camera it
//! stops at the edge of the room the bars leave and gains a chevron toward
//! the resident, like the destination beacon (`ui/minimap.rs`).

use super::hud::{NameTag, Projector};
use crate::scene::Scene;
use crate::theme::Palette;
use crate::world::World;
use crate::world::contact_markers::{Entry, Sighting, Tracker, ease_direction, in_scope, proximity_alpha};
use crate::world::contact_sets::Marker;
use crate::world::contact_sets::appearance::ICONS;
use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use glam::{DVec3, Vec3, Vec4};
use std::collections::HashSet;
use std::time::Instant;
use uuid::Uuid;

/// Half the diagonal of the diamond: a 45° square about 20 points wide.
const RADIUS: f32 = 14.0;
const INNER_RADIUS: f32 = 5.5;
const GLYPH: f32 = 14.0;
const BORDER: f32 = 1.25;
/// Opacity of the border over the glass.
const BORDER_ALPHA: f32 = 0.85;
/// The glass of a world label (`widgets::paint_world_label`).
const GLASS_TINT: f32 = 0.65;
const GLASS_SHADE: f32 = 0.65;
/// Markers that blur the world behind them, nearest first; the others only
/// tint it. Far apart on the screen, each widens the area blurred.
const MAX_GLASS: usize = 12;
/// Depth and half width of the chevron. A stroke, not the Tabler glyph: a
/// texture turned to any angle loses its thin lines.
const CHEVRON: f32 = 6.0;
const CHEVRON_WING: f32 = 9.0;
const CHEVRON_STROKE: f32 = 2.0;
/// Between the diamond and its chevron.
const CHEVRON_GAP: f32 = 5.0;
/// How far from the room's edge a pinned marker stands: its chevron stays
/// in the room, whichever way it points.
const EDGE_MARGIN: f32 = RADIUS + CHEVRON_GAP + CHEVRON + CHEVRON_STROKE;
/// Points between the head and the diamond.
const GAP: f32 = 6.0;
/// Points between the top of a name plate and the diamond: the plate's
/// badges float over that edge.
const PLATE_GAP: f32 = 12.0;

/// Ours, seated, loaded or coarse: the residents with a marker, in the
/// region we are in. A loaded avatar answers for its coarse location even
/// when it is filtered out, so that it does not come back through it.
fn sightings(world: &World, region: u64, now: Instant) -> Vec<Sighting> {
    let origin = world.main_origin();
    let global = |p: Vec3| {
        let (x, y) = origin.unwrap_or_default();
        p.as_dvec3() + DVec3::new(f64::from(x), f64::from(y), 0.0)
    };
    // a bitmap that does not hold us is the parcel we just left
    let parcel = world
        .parcel
        .as_ref()
        .zip(origin)
        .map(|(parcel, origin)| (parcel.bitmap.as_slice(), origin))
        .filter(|(bitmap, origin)| crate::world::landmarks::in_parcel(bitmap, *origin, global(world.agent.position)));
    let allowed = |id: &Uuid, distance: f32| world.rlva.nametag_visible(*id, world.agent_id, distance);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (idx, o) in world.objects.avatars() {
        if o.full_id.is_nil() || o.full_id == world.agent_id {
            continue;
        }
        let Some(marker) = marked(world, &o.full_id) else { continue };
        seen.insert(o.full_id);
        if !world.parcel_object_visible(idx) || !crate::snapshot::isolate::object_visible(world, idx) {
            continue;
        }
        let Some((position, _, _)) = Scene::object_transform(world, idx, now, 0) else {
            continue;
        };
        if !allowed(&o.full_id, position.distance(world.agent.position)) {
            continue;
        }
        out.push(Sighting {
            id: o.full_id,
            position,
            altitude_known: true,
            exact: true,
            in_scope: in_scope(marker.scope, o.key.region == region, parcel, global(position)),
            marker,
        });
    }
    let Some(list) = world.coarse.get(&region) else { return out };
    let offset = world.region_offset(region).unwrap_or(Vec3::ZERO);
    for (id, position) in list {
        if *id == world.agent_id || seen.contains(id) {
            continue;
        }
        let Some(marker) = marked(world, id) else { continue };
        let position = offset + *position;
        if !allowed(id, position.distance(world.agent.position)) {
            continue;
        }
        out.push(Sighting {
            id: *id,
            position,
            altitude_known: position.z != offset.z && position.z != offset.z + 1020.0,
            exact: false,
            in_scope: in_scope(marker.scope, true, parcel, global(position)),
            marker,
        });
    }
    out
}

fn marked(world: &World, id: &Uuid) -> Option<Marker> {
    world.contact_sets.appearance(id).marker.filter(|_| !world.is_avatar_blocked(id))
}

/// One frame of markers. `shown` is off with the setting, in immersive mode
/// without name tags, and over a held photo.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    scene: &Scene,
    proj: &Projector,
    eye: Vec3,
    tags: &[NameTag],
    tracker: &mut Tracker,
    shown: bool,
) {
    // what says where residents are stays hidden with the mini-map
    let hidden = world.rlva.has("shownearby") || world.rlva.has("showminimap");
    let (true, false, Some(region)) = (shown, hidden, world.main_region) else {
        tracker.clear();
        return;
    };
    let seen = sightings(world, region, Instant::now());
    let dt = tracker.update(region, ctx.input(|i| i.time), &seen, world.agent.position.z, |id| {
        marked(world, id).is_some()
    });
    let screen = ctx.content_rect();
    let room = super::layout::work_area(ctx).intersect(screen).shrink(EDGE_MARGIN);
    let inner = if room.is_positive() { room } else { screen.shrink(EDGE_MARGIN) };
    let mut markers: Vec<_> = tracker
        .iter_mut()
        .filter_map(|(id, entry)| {
            let alpha = entry.presence * proximity_alpha(entry.position.distance(world.agent.position), entry.marker.fade);
            let (at, edge) = place(id, entry, scene, proj, screen.center(), inner, tags, dt)?;
            (alpha > 0.0).then_some((entry.position.distance(eye), at, edge, entry.marker, alpha))
        })
        .collect();
    if markers.is_empty() {
        return;
    }
    // the nearest on top
    markers.sort_by(|a, b| b.0.total_cmp(&a.0));
    // never hidden by the scene
    aurora_render::world_label_at(ctx, None);
    let painter = super::hud::world_painter(ctx);
    let plain = markers.len().saturating_sub(MAX_GLASS);
    for (i, (_, at, edge, marker, alpha)) in markers.into_iter().enumerate() {
        paint(&painter, p, at, &marker, alpha, edge, i >= plain);
    }
}

/// Where a marker stands, and the way toward its resident when it is pinned
/// to the edge of `inner`.
#[allow(clippy::too_many_arguments)]
fn place(
    id: &Uuid,
    entry: &mut Entry,
    scene: &Scene,
    proj: &Projector,
    center: Pos2,
    inner: Rect,
    tags: &[NameTag],
    dt: f32,
) -> Option<(Pos2, Option<Vec2>)> {
    let root = proj.view_proj * entry.position.extend(1.0);
    let head = proj.view_proj * scene.name_tag_anchor(id, entry.position).extend(1.0);
    // as `Projector::project_name_tag`: above the root, not above a head
    // that perspective moves sideways
    let clip = if root.w > 0.05 && head.w > 0.05 {
        Vec4::new(root.x / root.w * head.w, head.y, head.z, head.w)
    } else {
        head
    };
    if !clip.is_finite() {
        return None;
    }
    let way = if clip.w > 0.05 {
        let on_screen = pos2(
            (clip.x / clip.w * 0.5 + 0.5) * proj.width / proj.ppp,
            (0.5 - clip.y / clip.w * 0.5) * proj.height / proj.ppp,
        );
        let rest = GAP + RADIUS;
        let lift = tags
            .iter()
            .find(|tag| tag.id == *id)
            .map_or(rest, |tag| (on_screen.y - tag.rect.top() + PLATE_GAP + RADIUS).max(rest));
        // no slide up from the head on the frame it appears
        if entry.presence == 0.0 {
            entry.lift = lift;
        }
        entry.ease_lift(lift, dt);
        let at = on_screen - vec2(0.0, entry.lift);
        let way = (at - center).normalized();
        if inner.contains(at) {
            entry.direction = Some(glam::Vec2::new(way.x, way.y));
            return Some((at, None));
        }
        way
    } else {
        super::minimap::edge_direction(clip, proj)
    };
    let way = glam::Vec2::new(way.x, way.y);
    let way = entry.direction.map_or(way, |current| ease_direction(current, way, dt));
    entry.direction = Some(way);
    let way = vec2(way.x, way.y);
    Some((super::minimap::edge_point(center, inner, way), Some(way)))
}

/// A 45° square, clockwise from the top, as `Shape::convex_polygon` wants.
fn diamond(at: Pos2, radius: f32) -> Vec<Pos2> {
    vec![
        at - vec2(0.0, radius),
        at + vec2(radius, 0.0),
        at + vec2(0.0, radius),
        at - vec2(radius, 0.0),
    ]
}

fn symbol(painter: &egui::Painter, icon: &str, at: Pos2, color: Color32) {
    let Some(texture) = super::icons::global(icon) else { return };
    let mut mesh = egui::Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(
        Rect::from_center_size(at, Vec2::splat(GLYPH)),
        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
        color,
    );
    painter.add(mesh);
}

/// The chevron beside a pinned diamond, pointing along `way`.
fn chevron(at: Pos2, way: Vec2) -> Vec<Pos2> {
    let base = at + way * (RADIUS + CHEVRON_GAP);
    let tip = base + way * CHEVRON;
    vec![base + way.rot90() * CHEVRON_WING, tip, base - way.rot90() * CHEVRON_WING]
}

/// `blur`: the world shows through the glass; off in a window, which has
/// its own background behind the preview.
fn paint(painter: &egui::Painter, p: &Palette, at: Pos2, marker: &Marker, alpha: f32, edge: Option<Vec2>, blur: bool) {
    let color = super::colors::set_color(marker.color).to_opaque();
    // the set's colour, moved in lightness to read on the panel's
    let ink = crate::interface_colors::on_panel(color).gamma_multiply(alpha);
    let blur = blur && p.panel_alpha < 255;
    let tint = if blur {
        let rect = Rect::from_center_size(at, Vec2::splat(RADIUS * 2.0));
        painter.add(aurora_render::glass_diamond(rect, alpha, GLASS_SHADE));
        f32::from(p.panel_alpha) / 255.0 * GLASS_TINT
    } else {
        1.0
    };
    painter.add(Shape::convex_polygon(
        diamond(at, RADIUS),
        p.panel.to_opaque().gamma_multiply(tint * alpha),
        Stroke::new(BORDER, ink.gamma_multiply(BORDER_ALPHA)),
    ));
    match marker.icon.and_then(|i| ICONS.get(i)) {
        Some(icon) => symbol(painter, icon, at, ink),
        None => {
            painter.add(Shape::convex_polygon(diamond(at, INNER_RADIUS), ink, Stroke::NONE));
        }
    }
    if let Some(way) = edge {
        let points = chevron(at, way);
        let backing = p.bar.to_opaque().gamma_multiply(0.55 * alpha);
        painter.add(Shape::line(points.clone(), Stroke::new(CHEVRON_STROKE + 2.0, backing)));
        painter.add(Shape::line(points, Stroke::new(CHEVRON_STROKE, color.gamma_multiply(alpha))));
    }
}

/// The marker of a set in its editor: on screen, then pinned to an edge.
pub fn preview(ui: &mut egui::Ui, p: &Palette, marker: &Marker) {
    let pinned_width = RADIUS * 2.0 + CHEVRON_GAP + CHEVRON + CHEVRON_STROKE;
    let size = vec2(RADIUS * 3.0 + pinned_width, RADIUS * 2.0 + 4.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let y = rect.center().y;
    paint(&painter, p, pos2(rect.left() + RADIUS, y), marker, 1.0, None, false);
    let pinned = pos2(rect.left() + RADIUS * 4.0, y);
    paint(&painter, p, pinned, marker, 1.0, Some(Vec2::RIGHT), false);
}
