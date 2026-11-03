//! "Place on surface": with the button (or P) on, the surface under the
//! cursor shows where the selection would go, and a click puts it there, a
//! chosen side against the surface, centred on the point clicked; the tool
//! then turns off.
//!
//! Creators turn their root prims any way, so a side is not one of the
//! object's own axes: it is read from how the selection sits when the tool
//! turns on (`Reference`). Bottom is the side pointing down then, Front the
//! side facing the camera, and they stay those sides while it turns.
//!
//! An Aurora feature, beside its snap to objects (`object_snap.rs`); no
//! Legacy Viewer has it. The surfaces are those the snap uses: what objects
//! show (the triangles picking uses), and the ground. The move is sent as
//! the Align tool's (`TransformUpdate` per linkset root), so only the
//! selection's move right counts.

use super::object_snap::{self, support};
use super::{Bounds, BuildSettings, BuildTool, EditMode, SelectType, Tool, draw::Painter3d, draw::fade};
use crate::scene::Scene;
use crate::world::World;
use crate::world::objects::ObjKey;
use aurora_net::build::{BuildCmd, TransformUpdate};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// The side of the selection put against the surface, as it sat when the
/// tool turned on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// As it is: nothing turns, the side facing the surface meets it.
    #[default]
    Auto,
    Bottom,
    Back,
    Top,
    Front,
    Left,
    Right,
}

impl Side {
    pub const ALL: [Side; 7] = [
        Side::Auto,
        Side::Bottom,
        Side::Back,
        Side::Top,
        Side::Front,
        Side::Left,
        Side::Right,
    ];

    /// Where it pointed when it was read: `forward` is the camera's view
    /// along the ground.
    fn seen(self, forward: Vec3) -> Option<Vec3> {
        let left = Vec3::Z.cross(forward);
        Some(match self {
            Side::Auto => return None,
            Side::Bottom => -Vec3::Z,
            Side::Top => Vec3::Z,
            Side::Front => -forward,
            Side::Back => forward,
            Side::Left => left,
            Side::Right => -left,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Auto => aurora_i18n::tr!("build-place-side-auto"),
            Side::Bottom => aurora_i18n::tr!("build-place-side-bottom"),
            Side::Back => aurora_i18n::tr!("build-place-side-back"),
            Side::Top => aurora_i18n::tr!("build-place-side-top"),
            Side::Front => aurora_i18n::tr!("build-place-side-front"),
            Side::Left => aurora_i18n::tr!("build-place-side-left"),
            Side::Right => aurora_i18n::tr!("build-place-side-right"),
        }
    }
}

/// How the selection sat when the tool turned on: its box axis for `side`
/// (in the box's frame), the one that pointed most that way.
pub fn side_axis(box_rotation: Quat, camera_at: Vec3, side: Side) -> Option<Vec3> {
    let forward = Vec3::new(camera_at.x, camera_at.y, 0.0).normalize_or(Vec3::X);
    let want = side.seen(forward)?;
    (0..3)
        .flat_map(|i| [1.0, -1.0].map(|s| unit(i) * s))
        .max_by(|a, b| (box_rotation * *a).dot(want).total_cmp(&(box_rotation * *b).dot(want)))
}

/// The selection and the view the sides were read with.
#[derive(Debug, Clone)]
struct Reference {
    selection: Vec<ObjKey>,
    rotation: Quat,
    camera_at: Vec3,
}

/// Where a click would put the selection.
#[derive(Debug, Clone, Copy)]
pub struct Landing {
    /// The point under the cursor and its surface's normal (toward the camera).
    pub point: Vec3,
    pub normal: Vec3,
    /// The selection's shown box now, and where it lands.
    pub from: Bounds,
    pub to: Bounds,
    /// The turn about the box's centre (identity when it only moves).
    pub turn: Quat,
}

/// The button's state, for the session.
#[derive(Debug, Default)]
pub struct Place {
    pub active: bool,
    pub preview: Option<Landing>,
    reference: Option<Reference>,
}

/// Where the selection's box `sel` goes on the surface at `point`: the side
/// `against` (an axis of the box's frame) turned flat against it, or, with
/// None, as it is, the side facing the surface on it; resting on the plane,
/// its centre over the point.
pub fn landing(sel: &Bounds, point: Vec3, normal: Vec3, against: Option<Vec3>) -> Landing {
    let n = normal.normalize_or(Vec3::Z);
    let turn = against.map_or(Quat::IDENTITY, |a| Quat::from_rotation_arc(sel.rotation * a, -n));
    let mut to = Bounds {
        center: point,
        rotation: (turn * sel.rotation).normalize(),
        half: sel.half,
    };
    to.center = point + n * support(&to, n);
    Landing {
        point,
        normal: n,
        from: *sel,
        to,
        turn,
    }
}

/// The box axis nearest `v` (a turned axis, read back in the box's frame).
fn snap_axis(v: Vec3) -> Vec3 {
    (0..3)
        .flat_map(|i| [1.0, -1.0].map(|s| unit(i) * s))
        .max_by(|a, b| a.dot(v).total_cmp(&b.dot(v)))
        .unwrap_or(-Vec3::Z)
}

fn unit(i: usize) -> Vec3 {
    let mut v = Vec3::ZERO;
    v[i] = 1.0;
    v
}

/// The ground's normal at `p`, from the height map around it.
fn ground_normal(world: &World, p: Vec3) -> Vec3 {
    let e = 0.5;
    let h = |dx: f32, dy: f32| world.ground_height(p + Vec3::new(dx, dy, 0.0)).unwrap_or(p.z);
    Vec3::new(h(-e, 0.0) - h(e, 0.0), h(0.0, -e) - h(0.0, e), 2.0 * e).normalize_or(Vec3::Z)
}

impl BuildTool {
    /// Whether the selection can be placed: whole objects in the world that
    /// may all be moved, with the Edit tool.
    pub fn can_place(&self, world: &World) -> bool {
        self.tool == Tool::Edit
            && !self.selection.is_empty()
            && !self.selection_linked
            && self.select_type(world) == SelectType::World
            && super::manip::can_transform_selection(world, &self.selection, false, EditMode::Move)
    }

    /// The button and P: on (the sides read again), or off.
    pub fn toggle_place(&mut self, world: &World) {
        self.place.active = !self.place.active && self.can_place(world);
        self.place.preview = None;
        self.place.reference = None;
    }

    /// The landing under the cursor, every frame while the button is on.
    pub(super) fn place_hover(&mut self, world: &World, s: &BuildSettings, scene: Option<&Scene>, cursor: (f32, f32), over_ui: bool) {
        self.place.preview = None;
        if !self.can_place(world) {
            self.place.active = false;
            return;
        }
        if over_ui {
            return;
        }
        let now = Instant::now();
        let Some(edit_box) = super::bounds_of(world, &self.selection, false, now) else {
            return;
        };
        let moving: HashSet<usize> = super::highlighted_of(world, &self.selection, false)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        let sel = scene
            .and_then(|scene| object_snap::shown_bounds(world, scene, &moving, &edit_box, now))
            .unwrap_or(edit_box);
        // the sides as the selection sits now, kept while it stays selected
        if self.place.reference.as_ref().is_none_or(|r| r.selection != self.selection) {
            self.place.reference = Some(Reference {
                selection: self.selection.clone(),
                rotation: edit_box.rotation,
                camera_at: self.cam.at,
            });
        }
        let ray = self.cam.ray(cursor.0, cursor.1);
        // the first object surface along the ray, past the selection
        let object = scene.and_then(|scene| {
            scene
                .left_edit_ray_hits(world, ray, false, s.select_probes, s.select_limit())
                .into_iter()
                .filter(|i| !moving.contains(i) && world.objects.get(*i).is_some_and(|o| !o.is_avatar()))
                .find_map(|i| {
                    let o = world.objects.get(i)?;
                    let t = scene.left_touch_surface(world, i, ray)?;
                    Some((t.position + world.region_offset(o.key.region)?, t.normal))
                })
        });
        let ground = super::land::terrain_hit(world, ray.0, ray.1).map(|p| (p, ground_normal(world, p)));
        let nearest = [object, ground]
            .into_iter()
            .flatten()
            .min_by(|a, b| a.0.distance_squared(ray.0).total_cmp(&b.0.distance_squared(ray.0)));
        let Some((point, normal)) = nearest else { return };
        // the side the camera sees
        let normal = if normal.dot(ray.1) > 0.0 { -normal } else { normal };
        // the side read then, in the box's frame now: the box turns with the
        // selection, so it is the same side after a turn
        let against = self.place.reference.as_ref().and_then(|r| {
            let axis = side_axis(r.rotation, r.camera_at, s.place_side)?;
            Some(sel.rotation.inverse() * (r.rotation * axis))
        });
        self.place.preview = Some(landing(&sel, point, normal, against.map(|a| snap_axis(a))));
    }

    /// A click while the button is on: put the selection where the preview
    /// shows, root by root (the Align tool's way); the button turns off (P
    /// places again).
    pub(super) fn place_click(&mut self, world: &mut World) {
        let Some(l) = self.place.preview.take() else { return };
        let now = Instant::now();
        let mut by_region: HashMap<aurora_net::RegionHandle, Vec<TransformUpdate>> = HashMap::new();
        for root in self.roots(world) {
            let Some((at, _, _)) = Scene::object_transform(world, root, now, 0) else {
                continue;
            };
            let delta = l.to.center + l.turn * (at - l.from.center) - at;
            let Some(o) = world.objects.get_mut(root) else { continue };
            o.position += delta;
            let turned = l.turn != Quat::IDENTITY;
            if turned {
                o.rotation = (l.turn * o.rotation).normalize();
            }
            o.velocity = Vec3::ZERO;
            o.render.needs_records = true;
            by_region.entry(o.key.region).or_default().push(TransformUpdate {
                local_id: o.key.local_id,
                position: Some(o.position),
                rotation: turned.then_some(o.rotation),
                scale: None,
                linked: true,
                uniform: false,
            });
        }
        for (handle, updates) in by_region {
            self.send(BuildCmd::Transform { handle, updates });
        }
        self.place.active = false;
    }

    /// The preview: a pad on the surface under the landing, and the
    /// selection's box where it would go.
    pub(super) fn place_draw(&self, p: &mut Painter3d) {
        let Some(l) = self.place.preview else { return };
        let pal = p.pal;
        let (n, to) = (l.normal, l.to);
        // the box's two axes across the surface, flattened onto it
        let mut across: Vec<Vec3> = (0..3).map(|i| to.rotation * unit(i) * to.half[i]).collect();
        across.sort_by(|a, b| a.normalize_or_zero().dot(n).abs().total_cmp(&b.normalize_or_zero().dot(n).abs()));
        let flat = |v: Vec3| (v - n * v.dot(n)) * 1.2;
        let (a, b) = (flat(across[0]), flat(across[1]));
        let c = l.point + n * 0.002;
        let pad = [c - a - b, c + a - b, c + a + b, c - a + b];
        p.tri_plain(pad[0], pad[1], pad[2], fade(pal.violet, 0.25));
        p.tri_plain(pad[0], pad[2], pad[3], fade(pal.violet, 0.25));
        p.flush();
        for k in 0..4 {
            p.line_cased(pad[k], pad[(k + 1) % 4], pal.violet_light, 2.0);
        }
        // the ghost of the selection, edge by edge
        let corner = |m: u8| {
            let s = Vec3::new(
                if m & 1 != 0 { 1.0 } else { -1.0 },
                if m & 2 != 0 { 1.0 } else { -1.0 },
                if m & 4 != 0 { 1.0 } else { -1.0 },
            );
            to.center + to.rotation * (to.half * s)
        };
        for m in 0u8..8 {
            for bit in [1u8, 2, 4] {
                if m & bit == 0 {
                    p.line(corner(m), corner(m | bit), fade(pal.violet_light, 0.85), 1.5);
                }
            }
        }
        p.square(l.point, 6.0, pal.violet_light, egui::Stroke::new(1.0, fade(pal.navy, 0.5)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(center: Vec3, half: Vec3, rotation: Quat) -> Bounds {
        Bounds { center, rotation, half }
    }

    #[test]
    fn as_it_is_puts_the_side_facing_the_surface_on_it_without_turning() {
        let sel = cube(Vec3::new(5.0, 5.0, 9.0), Vec3::new(0.5, 0.3, 0.2), Quat::from_rotation_z(0.4));
        // on a floor: its bottom, centred over the point
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), Vec3::Z, None);
        assert_eq!(l.turn, Quat::IDENTITY);
        assert!(l.to.center.abs_diff_eq(Vec3::new(1.0, 2.0, 3.2), 1e-5), "{:?}", l.to);
        // on a wall facing +X: its side toward the wall
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), Vec3::X, None);
        assert!((l.to.center.x - (1.0 + support(&sel, Vec3::X))).abs() < 1e-5);
        assert_eq!((l.to.center.y, l.to.center.z), (2.0, 3.0));
        // a tilted one rests on its corner
        let tilted = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::from_rotation_x(0.3));
        let l = landing(&tilted, Vec3::ZERO, Vec3::Z, None);
        assert_eq!(l.to.rotation, tilted.rotation);
        assert!(l.to.center.z > 0.5);
    }

    #[test]
    fn sides_are_read_from_how_it_sits_not_from_the_creators_axes() {
        // a root prim turned on its side by its creator: its local +Y points down
        let rot = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert!((rot * Vec3::Y).abs_diff_eq(Vec3::Z, 1e-5));
        let at = Vec3::new(0.0, 1.0, -0.3); // the camera looks along +Y, a little down
        let axis = |side| side_axis(rot, at, side).unwrap();
        assert!((rot * axis(Side::Bottom)).abs_diff_eq(-Vec3::Z, 1e-5));
        assert!((rot * axis(Side::Top)).abs_diff_eq(Vec3::Z, 1e-5));
        assert!((rot * axis(Side::Back)).abs_diff_eq(Vec3::Y, 1e-5), "away from the camera");
        assert!((rot * axis(Side::Front)).abs_diff_eq(-Vec3::Y, 1e-5), "toward it");
        assert!((rot * axis(Side::Left)).abs_diff_eq(-Vec3::X, 1e-5), "on the camera's left");
        assert_eq!(side_axis(rot, at, Side::Auto), None);
    }

    #[test]
    fn a_chosen_side_turns_flat_against_the_surface() {
        let sel = cube(Vec3::ZERO, Vec3::new(0.1, 0.6, 0.4), Quat::IDENTITY);
        // a picture: its back (-X here) against a wall facing -Y, slanted 10°
        let wall = (Quat::from_rotation_x(0.17) * -Vec3::Y).normalize();
        let l = landing(&sel, Vec3::new(3.0, 8.0, 2.0), wall, Some(-Vec3::X));
        assert!((l.to.rotation * -Vec3::X).abs_diff_eq(-wall, 1e-5), "its back on the wall");
        assert!((l.to.center - Vec3::new(3.0, 8.0, 2.0)).abs_diff_eq(wall * 0.1, 1e-5));
        // its top against a ceiling facing down
        let up = landing(&sel, Vec3::new(0.0, 0.0, 4.0), -Vec3::Z, Some(Vec3::Z));
        assert!((up.to.rotation * Vec3::Z).abs_diff_eq(Vec3::Z, 1e-5));
        assert!((up.to.center.z - 3.6).abs() < 1e-5);
    }

    #[test]
    fn the_side_read_stays_that_side_after_the_object_turns() {
        let first = Quat::from_rotation_y(0.7);
        let bottom = side_axis(first, Vec3::X, Side::Bottom).unwrap();
        // placed on a wall, turned a quarter: the same box axis is its bottom
        let turned = Quat::from_rotation_z(1.2) * first;
        let again = snap_axis(turned.inverse() * (first * bottom));
        assert!((turned.inverse() * (turned * again)).abs_diff_eq(bottom, 1e-5));
    }

    #[test]
    fn sides_keep_their_names() {
        assert_eq!(serde_json::to_string(&Side::Bottom).unwrap(), "\"bottom\"");
        let s: BuildSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.place_side, Side::Auto);
    }
}
