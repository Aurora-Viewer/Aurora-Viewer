//! Markers over contact-set members (`world/contact_markers.rs` says who
//! and where): a glass disc or diamond with a border and a symbol in the
//! set's colour, seen through everything in the world. Near a resident it
//! lands on the badge of its set on their name plate, or fades where it is
//! without one, leaving the plate alone: nothing shows through what stands
//! in front of a plate. Off screen or behind the camera it stops at the edge of the room the bars leave and
//! gains a chevron toward the resident, like the destination beacon
//! (`ui/minimap.rs`).

use super::hud::{NameTag, Projector};
use crate::scene::Scene;
use crate::settings::MarkerAnchor;
use crate::theme::Palette;
use crate::world::World;
use crate::world::contact_markers::{Entry, Sighting, Tracker, ease_direction, in_scope, landing_alpha, proximity_alpha};
use crate::world::contact_sets::appearance::ICONS;
use crate::world::contact_sets::{Marker, MarkerShape};
use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use glam::{DVec3, Vec3, Vec4};
use std::collections::HashSet;
use std::time::Instant;
use uuid::Uuid;

/// The disc: larger than a plate's badge, which it shrinks to on landing.
const RADIUS: f32 = 12.0;
/// Half the diagonal of the diamond: the same surface as near as it reads.
const DIAMOND_RADIUS: f32 = 14.0;
/// The dot of a set without a symbol, and its diamond.
const DOT: f32 = 3.5;
const DIAMOND_DOT: f32 = 5.5;
/// Symbol size at full size.
const GLYPH: f32 = 13.0;
/// Points of a diamond's outline, in fours: its corners are among them.
const DIAMOND_POINTS: usize = 32;
/// The width of a plate's border.
const BORDER: f32 = 1.0;
/// The glass of a world label (`widgets::paint_world_label`).
const GLASS_TINT: f32 = 0.65;
/// Opacity of the disc once landed, as a plate's badge.
const BADGE_TINT: f32 = 0.9;
const GLASS_SHADE: f32 = 0.65;
/// Where the chest is without a skeleton (a resident known by their coarse
/// location), as a share of the way from the root to the name tag.
const CHEST: f32 = 0.4;
/// Markers that blur the world behind them, nearest first; the others only
/// tint it. Far apart on the screen, each widens the area blurred.
const MAX_GLASS: usize = 12;
/// Depth and half width of the chevron. A stroke, not the Tabler glyph: a
/// texture turned to any angle loses its thin lines.
const CHEVRON: f32 = 6.0;
const CHEVRON_WING: f32 = 9.0;
const CHEVRON_STROKE: f32 = 2.0;
/// Between the disc and its chevron.
const CHEVRON_GAP: f32 = 5.0;
/// How far from the room's edge a pinned marker stands: its chevron stays
/// in the room, whichever way it points.
const EDGE_MARGIN: f32 = DIAMOND_RADIUS + CHEVRON_GAP + CHEVRON + CHEVRON_STROKE;
/// Points between the head and the disc.
const GAP: f32 = 6.0;
/// Points between the top of a name plate and the disc: the plate's
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
    for (id, position, altitude_known) in crate::world::radar::coarse_in(world, region) {
        if seen.contains(&id) {
            continue;
        }
        let Some(marker) = marked(world, &id) else { continue };
        if !allowed(&id, position.distance(world.agent.position)) {
            continue;
        }
        out.push(Sighting {
            id,
            position,
            altitude_known,
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
    anchor: MarkerAnchor,
    shown: bool,
) {
    // with every set's marker off (the default), nothing to look for: no
    // walk through the avatars and the coarse locations
    let wanted = shown
        && world.contact_sets.has_markers()
        // what says where residents are stays hidden with the mini-map
        && !(world.rlva.has("shownearby") || world.rlva.has("showminimap"));
    let (true, Some(region)) = (wanted, world.main_region) else {
        tracker.clear();
        return;
    };
    let now = Instant::now();
    let seen = sightings(world, region, now);
    let dt = tracker.update(region, ctx.input(|i| i.time), &seen, world.agent.position.z, |id| {
        marked(world, id).is_some()
    });
    let screen = ctx.content_rect();
    let Some(inner) = room(super::layout::work_area(ctx), screen) else {
        return;
    };
    let mut markers: Vec<_> = tracker
        .iter_mut()
        .filter_map(|(id, entry)| {
            let alpha = entry.presence * proximity_alpha(entry.position.distance(world.agent.position), entry.marker.fade);
            let distance = entry.position.distance(eye);
            let placed = place(
                id,
                entry,
                world,
                scene,
                now,
                proj,
                screen.center(),
                inner,
                tags,
                anchor,
                distance,
                dt,
            )?;
            let alpha = alpha * placed.fade;
            (alpha > 0.0).then_some((distance, placed, entry.marker, alpha))
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
    for (i, (_, placed, marker, alpha)) in markers.into_iter().enumerate() {
        paint(&painter, p, &placed, &marker, alpha, i >= plain);
    }
}

/// Where markers may stand: the room the bars leave, else the whole screen,
/// `EDGE_MARGIN` inside. None on a window too small for one (`edge_point`
/// needs a rectangle that is not inverted).
fn room(work: Rect, screen: Rect) -> Option<Rect> {
    [work.intersect(screen), screen]
        .into_iter()
        .map(|rect| rect.shrink(EDGE_MARGIN))
        .find(|rect| rect.is_positive())
}

fn full_radius(shape: MarkerShape) -> f32 {
    match shape {
        MarkerShape::Circle => RADIUS,
        MarkerShape::Diamond => DIAMOND_RADIUS,
    }
}

/// Where a marker stands this frame.
struct Placed {
    at: Pos2,
    radius: f32,
    /// 0 above the plate, 1 landed on it.
    merge: f32,
    /// The way toward its resident, pinned to the edge of the room.
    edge: Option<Vec2>,
    /// Its opacity, as it goes near its plate.
    fade: f32,
}

/// The chest bone, kept relative to `root`: while a marker eases in from its
/// coarse position, it stays with the marker and not with the avatar.
fn chest(world: &World, scene: &Scene, id: &Uuid, now: Instant, root: Vec3, tag_anchor: Vec3) -> Vec3 {
    let avatar = world
        .objects
        .index_of_uuid(id)
        .and_then(|index| Scene::object_transform(world, index, now, 0));
    match avatar.zip(scene.joint_world(world, id, "mChest", now)) {
        Some(((position, ..), bone)) => root + bone - position,
        None => root.lerp(tag_anchor, CHEST),
    }
}

#[allow(clippy::too_many_arguments)]
fn place(
    id: &Uuid,
    entry: &mut Entry,
    world: &World,
    scene: &Scene,
    now: Instant,
    proj: &Projector,
    center: Pos2,
    inner: Rect,
    tags: &[NameTag],
    anchor: MarkerAnchor,
    distance: f32,
    dt: f32,
) -> Option<Placed> {
    let root = proj.view_proj * entry.position.extend(1.0);
    let tag_anchor = scene.name_tag_anchor(id, entry.position);
    let head = proj.view_proj * tag_anchor.extend(1.0);
    let above = anchor == MarkerAnchor::AboveName;
    // as `Projector::project_name_tag`: above the root, not above a head
    // that perspective moves sideways
    let clip = if !above {
        proj.view_proj * chest(world, scene, id, now, entry.position, tag_anchor).extend(1.0)
    } else if root.w > 0.05 && head.w > 0.05 {
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
        let tag = tags.iter().find(|tag| tag.id == *id);
        let full = full_radius(entry.marker.shape);
        let rest = GAP + full;
        let lift = match tag {
            _ if !above => 0.0,
            Some(tag) => (on_screen.y - tag.rect.top() + PLATE_GAP + full).max(rest),
            None => rest,
        };
        let at = on_screen - vec2(0.0, entry.ease_lift(lift, dt));
        let way = (at - center).normalized();
        if inner.contains(at) {
            entry.direction = Some(glam::Vec2::new(way.x, way.y));
            let slot = tag.and_then(|tag| tag.marker_slot);
            if let (true, Some(tag), Some(slot)) = (entry.landing(tag.is_some(), distance), tag, slot) {
                let offset = slot.center() - tag.rect.center_bottom();
                entry.badge = Some((glam::Vec2::new(offset.x, offset.y), slot.width()));
            }
            let merge = entry.ease_merge(tag.is_some(), distance, dt);
            let plate = tag.filter(|_| merge > 0.0);
            let Some(plate) = plate else {
                entry.drop_merge();
                return Some(Placed {
                    at,
                    radius: full,
                    merge: 0.0,
                    edge: None,
                    fade: 1.0,
                });
            };
            // a plate with no badge of its set: nowhere to land, it fades where it is
            let Some((offset, width)) = entry.badge else {
                return Some(Placed {
                    at,
                    radius: full,
                    merge: 0.0,
                    edge: None,
                    fade: 1.0 - merge,
                });
            };
            let badge = plate.rect.center_bottom() + vec2(offset.x, offset.y);
            return Some(Placed {
                at: at + (badge - at) * merge,
                radius: full + (width * 0.5 - full) * merge,
                merge,
                edge: None,
                fade: landing_alpha(merge),
            });
        }
        way
    } else {
        super::minimap::edge_direction(clip, proj)
    };
    let way = glam::Vec2::new(way.x, way.y);
    let way = entry.direction.map_or(way, |current| ease_direction(current, way, dt));
    entry.direction = Some(way);
    entry.drop_merge();
    let way = vec2(way.x, way.y);
    Some(Placed {
        at: super::minimap::edge_point(center, inner, way),
        radius: full_radius(entry.marker.shape),
        merge: 0.0,
        edge: Some(way),
        fade: 1.0,
    })
}

fn symbol(painter: &egui::Painter, icon: &str, at: Pos2, size: f32, color: Color32) {
    let Some(texture) = super::icons::global(icon) else { return };
    let mut mesh = egui::Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(
        Rect::from_center_size(at, Vec2::splat(size)),
        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
        color,
    );
    painter.add(mesh);
}

/// A 45° square, clockwise from the right as `Shape::convex_polygon` wants,
/// rounding into a circle of the same reach as `round` goes from 0 to 1: a
/// diamond lands on a plate's round badge.
fn diamond(at: Pos2, radius: f32, round: f32) -> Vec<Pos2> {
    (0..DIAMOND_POINTS)
        .map(|i| {
            let way = Vec2::angled(std::f32::consts::TAU * i as f32 / DIAMOND_POINTS as f32);
            let square = 1.0 / (way.x.abs() + way.y.abs());
            at + way * radius * (square + (1.0 - square) * round)
        })
        .collect()
}

/// The chevron beside a pinned marker, pointing along `way`.
fn chevron(at: Pos2, radius: f32, way: Vec2) -> Vec<Pos2> {
    let base = at + way * (radius + CHEVRON_GAP);
    let tip = base + way * CHEVRON;
    vec![base + way.rot90() * CHEVRON_WING, tip, base - way.rot90() * CHEVRON_WING]
}

/// `blur`: the world shows through the glass; off in a window, which has
/// its own background behind the preview.
fn paint(painter: &egui::Painter, p: &Palette, placed: &Placed, marker: &Marker, alpha: f32, blur: bool) {
    let &Placed {
        at, radius, merge, edge, ..
    } = placed;
    let color = super::colors::set_color(marker.color).to_opaque();
    // the resident's plate background; a plate's badge keeps the plain one
    let surface = marker
        .background
        .map_or(p.panel, |background| super::nameplate::set_background(p, background))
        .to_opaque();
    let surface = surface.lerp_to_gamma(p.panel.to_opaque(), merge);
    // the set's colour, moved in lightness to read on the panel's, then
    // toward the text if a tint of that same colour still hides it
    let ink = crate::interface_colors::on_panel(color);
    let ink = super::nameplate::readable_on(ink, surface, p.ink).gamma_multiply(alpha);
    let blur = blur && p.panel_alpha < 255;
    let tint = if blur {
        let rect = Rect::from_center_size(at, Vec2::splat(radius * 2.0));
        let opacity = alpha * (1.0 - merge);
        painter.add(match marker.shape {
            MarkerShape::Circle => aurora_render::glass_region(rect, radius, opacity, GLASS_SHADE),
            MarkerShape::Diamond => aurora_render::glass_diamond(rect, opacity, GLASS_SHADE),
        });
        f32::from(p.panel_alpha) / 255.0 * GLASS_TINT
    } else {
        1.0
    };
    let tint = tint + (BADGE_TINT - tint) * merge;
    let panel = surface.gamma_multiply(tint * alpha);
    // the resident's plate border, the plain one without a set's; a plate's
    // badge has none: it goes as the marker lands
    let border = marker
        .border
        .map_or_else(|| crate::ui::nameplate::plain_border(p), super::colors::set_color);
    let border = Stroke::new(BORDER, border.gamma_multiply(alpha * (1.0 - merge)));
    match marker.shape {
        MarkerShape::Circle => {
            painter.add(Shape::circle_filled(at, radius, panel));
            painter.add(Shape::circle_stroke(at, radius, border));
        }
        MarkerShape::Diamond => {
            painter.add(Shape::convex_polygon(diamond(at, radius, merge), panel, border));
        }
    }
    let scale = radius / full_radius(marker.shape);
    match (marker.icon.and_then(|i| ICONS.get(i)), marker.shape) {
        (Some(icon), _) => symbol(painter, icon, at, GLYPH * scale, ink),
        (None, MarkerShape::Circle) => {
            painter.add(Shape::circle_filled(at, DOT * scale, ink));
        }
        (None, MarkerShape::Diamond) => {
            painter.add(Shape::convex_polygon(diamond(at, DIAMOND_DOT * scale, merge), ink, Stroke::NONE));
        }
    }
    if let Some(way) = edge {
        let points = chevron(at, radius, way);
        let backing = p.bar.to_opaque().gamma_multiply(0.55 * alpha);
        painter.add(Shape::line(points.clone(), Stroke::new(CHEVRON_STROKE + 2.0, backing)));
        painter.add(Shape::line(points, Stroke::new(CHEVRON_STROKE, color.gamma_multiply(alpha))));
    }
}

/// The marker of a set in its editor: on screen, then pinned to an edge.
/// One marker at full size at `at`, as it stands over a resident seen from
/// afar (the set editor's preview).
pub fn paint_preview_at(painter: &egui::Painter, p: &Palette, at: Pos2, marker: &Marker) {
    let placed = Placed {
        at,
        radius: full_radius(marker.shape),
        merge: 0.0,
        fade: 1.0,
        edge: None,
    };
    paint(painter, p, &placed, marker, 1.0, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_falls_back_to_the_screen_then_to_none_on_a_tiny_window() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let work = Rect::from_min_max(pos2(0.0, 40.0), pos2(800.0, 560.0));
        assert_eq!(room(work, screen), Some(work.shrink(EDGE_MARGIN)));
        // bars that leave no room: the whole screen
        let squeezed = Rect::from_min_max(pos2(0.0, 290.0), pos2(800.0, 310.0));
        assert_eq!(room(squeezed, screen), Some(screen.shrink(EDGE_MARGIN)));
        // smaller than two margins: nowhere, rather than an inverted rectangle
        let tiny = Rect::from_min_size(Pos2::ZERO, vec2(40.0, 40.0));
        assert_eq!(room(tiny, tiny), None);
    }

    #[test]
    fn a_diamond_stands_on_its_corners_and_rounds_into_a_circle() {
        let at = pos2(100.0, 50.0);
        let square = diamond(at, 10.0, 0.0);
        assert_eq!(square.len() % 4, 0);
        for (i, corner) in [vec2(10.0, 0.0), vec2(0.0, 10.0), vec2(-10.0, 0.0), vec2(0.0, -10.0)]
            .into_iter()
            .enumerate()
        {
            assert!((square[i * DIAMOND_POINTS / 4] - (at + corner)).length() < 1e-3);
        }
        // the middle of a side, nearer than the corners
        let side = square[DIAMOND_POINTS / 8] - at;
        assert!((side.length() - 10.0 / 2.0_f32.sqrt()).abs() < 1e-3);
        assert!(diamond(at, 10.0, 1.0).iter().all(|point| (point.distance(at) - 10.0).abs() < 1e-3));
    }
}
