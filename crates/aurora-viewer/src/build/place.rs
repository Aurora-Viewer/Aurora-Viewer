//! "Place on surface": with the button (or P) on, the surface under the
//! cursor shows where the selection would go, a hologram of it there, and a
//! click puts it there, centred on the point clicked; the tool then turns
//! off. "Keep rotation" only moves it, the side facing the surface on it;
//! "Auto rotate" turns a chosen side flat against the surface.
//!
//! Creators turn their root prims any way, so a side is not one of the
//! object's own axes: it is read from how the selection sat the first time
//! the tool was turned on for it (`Truth`), Bottom the side pointing down
//! then, Front the side facing the camera. That rotation is the ground
//! truth: every placement turns the selection from it, not from where the
//! last placement left it, so Bottom on a floor always stands it back the
//! way it was. A turn by anything else (the rotate handles, a script) makes
//! the rotation it then has the new truth.
//!
//! An Aurora feature, beside its snap to objects (`object_snap.rs`); no
//! Legacy Viewer has it. The surfaces are those the snap uses: what objects
//! show (the triangles picking uses), and the ground. The move is sent as
//! the Align tool's (`TransformUpdate` per linkset root), so only the
//! selection's move right counts.

use super::object_snap::{self, same_box, support};
use super::{Bounds, BuildSettings, BuildTool, EditMode, SelectType, Tool, draw::Painter3d, draw::fade};
use crate::scene::Scene;
use crate::world::World;
use crate::world::objects::ObjKey;
use aurora_net::build::{BuildCmd, TransformUpdate};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// A rotation further than this from the one the tool left (radians) was
/// made by something else: the network's rounding of a placement stays
/// well under it.
const TURNED: f32 = 0.02;
/// A turn smaller than this (radians) is f32 noise.
const NOISE: f32 = 1e-3;
/// Selections whose truth is kept, for the session.
const MAX_TRUTHS: usize = 64;

/// The side of the selection put against the surface, as it sat the first
/// time the tool turned on for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// (`auto`: the "As it is" of the first test builds, now Keep rotation)
    #[default]
    #[serde(alias = "auto")]
    Bottom,
    Back,
    Top,
    Front,
    Left,
    Right,
}

impl Side {
    pub const ALL: [Side; 6] = [Side::Bottom, Side::Back, Side::Top, Side::Front, Side::Left, Side::Right];

    /// Where it pointed when it was read: `forward` is the camera's view
    /// along the ground.
    fn seen(self, forward: Vec3) -> Vec3 {
        let left = Vec3::Z.cross(forward);
        match self {
            Side::Bottom => -Vec3::Z,
            Side::Top => Vec3::Z,
            Side::Front => -forward,
            Side::Back => forward,
            Side::Left => left,
            Side::Right => -left,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Bottom => aurora_i18n::tr!("build-place-side-bottom"),
            Side::Back => aurora_i18n::tr!("build-place-side-back"),
            Side::Top => aurora_i18n::tr!("build-place-side-top"),
            Side::Front => aurora_i18n::tr!("build-place-side-front"),
            Side::Left => aurora_i18n::tr!("build-place-side-left"),
            Side::Right => aurora_i18n::tr!("build-place-side-right"),
        }
    }
}

/// The camera's view along the ground.
fn flat_forward(camera_at: Vec3) -> Vec3 {
    Vec3::new(camera_at.x, camera_at.y, 0.0).normalize_or(Vec3::X)
}

/// How the selection sat when it was read: its box axis for `side` (in the
/// box's frame), the one that pointed most that way.
pub fn side_axis(box_rotation: Quat, camera_at: Vec3, side: Side) -> Vec3 {
    let want = side.seen(flat_forward(camera_at));
    (0..3)
        .flat_map(|i| [1.0, -1.0].map(|s| unit(i) * s))
        .max_by(|a, b| (box_rotation * *a).dot(want).total_cmp(&(box_rotation * *b).dot(want)))
        .unwrap_or(-Vec3::Z)
}

/// The turn, from the box's rotation `now`, that puts `side` of the box as
/// it sat (`truth`, read with the camera at `camera_at`) flat against a
/// surface facing `normal`: the shortest turn from the truth, so a side
/// already flat on it leaves the truth exactly. Turned right over (a top
/// onto a floor), it rolls about the camera's view, so its front stays in
/// front.
pub fn turn_to(truth: Quat, camera_at: Vec3, now: Quat, side: Side, normal: Vec3) -> Quat {
    let n = normal.normalize_or(Vec3::Z);
    let from = truth * side_axis(truth, camera_at, side);
    let to = -n;
    let arc = if from.dot(to) < -0.9999 {
        let forward = flat_forward(camera_at);
        let axis = (forward - from * forward.dot(from))
            .try_normalize()
            .unwrap_or_else(|| from.any_orthonormal_vector());
        Quat::from_axis_angle(axis, std::f32::consts::PI)
    } else {
        Quat::from_rotation_arc(from, to)
    };
    // (under a twentieth of a degree is float noise: the truth exactly)
    let arc = if arc.angle_between(Quat::IDENTITY) < NOISE {
        Quat::IDENTITY
    } else {
        arc
    };
    let turn = (arc * truth * now.inverse()).normalize();
    if turn.angle_between(Quat::IDENTITY) < NOISE {
        Quat::IDENTITY
    } else {
        turn
    }
}

/// How the selection sat the first time the tool turned on for it.
#[derive(Debug, Clone, Copy)]
struct Truth {
    rotation: Quat,
    camera_at: Vec3,
    /// The rotation the tool last saw or left it at.
    last: Quat,
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

impl Landing {
    /// Where a point of the selection goes.
    pub fn apply(&self, p: Vec3) -> Vec3 {
        self.to.center + self.turn * (p - self.from.center)
    }
}

/// The button's state, for the session.
#[derive(Debug, Default)]
pub struct Place {
    pub active: bool,
    pub preview: Option<Landing>,
    /// The prims of the preview, for its hologram.
    pub moving: Vec<usize>,
    truths: HashMap<Vec<ObjKey>, Truth>,
    /// The selection's shown box, kept while its edit box stays put: it
    /// goes through every visible triangle (a complex mesh's many).
    shown: Option<(Vec<ObjKey>, Bounds, Bounds)>,
}

/// Where the selection's box `sel` goes on the surface at `point`, turned by
/// `turn` about its centre: resting on the plane, `offset` off it (metres
/// along its normal; below it when negative), its centre over the point.
pub fn landing(sel: &Bounds, point: Vec3, normal: Vec3, turn: Quat, offset: f32) -> Landing {
    let n = normal.normalize_or(Vec3::Z);
    let mut to = Bounds {
        center: point,
        rotation: (turn * sel.rotation).normalize(),
        half: sel.half,
    };
    to.center = point + n * (support(&to, n) + offset);
    Landing {
        point,
        normal: n,
        from: *sel,
        to,
        turn,
    }
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

impl Place {
    /// Off, its preview gone (the tools closed, another tool).
    pub fn stop(&mut self) {
        self.active = false;
        self.preview = None;
        self.moving.clear();
    }

    /// The truth of `selection`, its box now at `now`: read now the first
    /// time, and again when something else turned it.
    fn truth(&mut self, selection: &[ObjKey], now: Quat, camera_at: Vec3) -> Truth {
        if self.truths.len() >= MAX_TRUTHS && !self.truths.contains_key(selection) {
            self.truths.clear();
        }
        let fresh = Truth {
            rotation: now,
            camera_at,
            last: now,
        };
        // (no key copied while it is known: this runs every frame)
        let t = match self.truths.get_mut(selection) {
            Some(t) => t,
            None => self.truths.entry(selection.to_vec()).or_insert(fresh),
        };
        if t.last.angle_between(now) > TURNED {
            *t = fresh;
        }
        t.last = now;
        *t
    }
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

    /// The button and P: on, or off.
    pub fn toggle_place(&mut self, world: &World) {
        self.place.active = !self.place.active && self.can_place(world);
        self.place.preview = None;
        self.place.shown = None;
    }

    /// The landing under the cursor, every frame while the button is on.
    pub(super) fn place_hover(&mut self, world: &World, s: &BuildSettings, scene: Option<&Scene>, cursor: (f32, f32), over_ui: bool) {
        self.place.preview = None;
        // off with the selection or the tool it needs, and with the object
        // snap its button goes with
        if !self.can_place(world) || s.snap_mode() != super::SnapMode::Objects {
            self.place.active = false;
            return;
        }
        let now = Instant::now();
        // (`edit_box` turns with the first prim selected)
        let Some(edit_box) = super::bounds_of(world, &self.selection, false, now) else {
            return;
        };
        let truth = self.place.truth(&self.selection, edit_box.rotation, self.cam.at);
        if over_ui {
            return;
        }
        let moving: HashSet<usize> = super::highlighted_of(world, &self.selection, false)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        let sel = match &self.place.shown {
            Some((keys, at, shown)) if *keys == self.selection && same_box(at, &edit_box) => *shown,
            _ => {
                let shown = scene
                    .and_then(|scene| object_snap::shown_bounds(world, scene, &moving, &edit_box, now))
                    .unwrap_or(edit_box);
                self.place.shown = Some((self.selection.clone(), edit_box, shown));
                shown
            }
        };
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
        let turn = if s.place_rotate {
            turn_to(truth.rotation, truth.camera_at, edit_box.rotation, s.place_side, normal)
        } else {
            Quat::IDENTITY
        };
        self.place.preview = Some(landing(&sel, point, normal, turn, s.place_offset));
        // (in a steady order: the hologram's faces are drawn in it, and a
        // set's order changes from one frame to the next)
        self.place.moving = moving.into_iter().collect();
        self.place.moving.sort_unstable();
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
            let delta = l.apply(at) - at;
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
        // its own turn keeps the truth
        if let Some(t) = self.place.truths.get_mut(&self.selection) {
            t.last = (l.turn * t.last).normalize();
        }
        self.place.stop();
    }

    /// The preview's pad on the surface, in the accent; the selection itself
    /// shows there as a hologram (`scene/build_hologram.rs`).
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
            p.line_cased(pad[k], pad[(k + 1) % 4], pal.violet, 2.0);
        }
        p.square(l.point, 6.0, pal.violet, egui::Stroke::new(1.0, fade(pal.navy, 0.5)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::EulerRot;

    fn cube(center: Vec3, half: Vec3, rotation: Quat) -> Bounds {
        Bounds { center, rotation, half }
    }

    /// The build floater's rotation (degrees, X then Y then Z).
    fn euler(x: f32, y: f32, z: f32) -> Quat {
        Quat::from_euler(EulerRot::ZYX, z.to_radians(), y.to_radians(), x.to_radians())
    }

    #[test]
    fn keep_rotation_puts_the_side_facing_the_surface_on_it_without_turning() {
        let sel = cube(Vec3::new(5.0, 5.0, 9.0), Vec3::new(0.5, 0.3, 0.2), Quat::from_rotation_z(0.4));
        // on a floor: its bottom, centred over the point
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), Vec3::Z, Quat::IDENTITY, 0.0);
        assert_eq!(l.turn, Quat::IDENTITY);
        assert!(l.to.center.abs_diff_eq(Vec3::new(1.0, 2.0, 3.2), 1e-5), "{:?}", l.to);
        // an offset lifts it off the surface, or sinks it in
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), Vec3::Z, Quat::IDENTITY, 0.25);
        assert!(l.to.center.abs_diff_eq(Vec3::new(1.0, 2.0, 3.45), 1e-5));
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), -Vec3::X, Quat::IDENTITY, -0.1);
        assert!((l.to.center.x - (1.0 - support(&sel, Vec3::X) + 0.1)).abs() < 1e-5);
        // on a wall facing +X: its side toward the wall
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), Vec3::X, Quat::IDENTITY, 0.0);
        assert!((l.to.center.x - (1.0 + support(&sel, Vec3::X))).abs() < 1e-5);
        assert_eq!((l.to.center.y, l.to.center.z), (2.0, 3.0));
        // a tilted one rests on its corner
        let tilted = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::from_rotation_x(0.3));
        let l = landing(&tilted, Vec3::ZERO, Vec3::Z, Quat::IDENTITY, 0.0);
        assert_eq!(l.to.rotation, tilted.rotation);
        assert!(l.to.center.z > 0.5);
    }

    #[test]
    fn sides_are_read_from_how_it_sits_not_from_the_creators_axes() {
        // a root prim turned on its side by its creator: its local +Y points down
        let rot = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert!((rot * Vec3::Y).abs_diff_eq(Vec3::Z, 1e-5));
        let at = Vec3::new(0.0, 1.0, -0.3); // the camera looks along +Y, a little down
        let axis = |side| side_axis(rot, at, side);
        assert!((rot * axis(Side::Bottom)).abs_diff_eq(-Vec3::Z, 1e-5));
        assert!((rot * axis(Side::Top)).abs_diff_eq(Vec3::Z, 1e-5));
        assert!((rot * axis(Side::Back)).abs_diff_eq(Vec3::Y, 1e-5), "away from the camera");
        assert!((rot * axis(Side::Front)).abs_diff_eq(-Vec3::Y, 1e-5), "toward it");
        assert!((rot * axis(Side::Left)).abs_diff_eq(-Vec3::X, 1e-5), "on the camera's left");
    }

    #[test]
    fn bottom_on_a_floor_leaves_a_creator_rotated_object_exactly_as_it_was() {
        // the object of the report: 90, 0, 180 in the build floater
        let truth = euler(90.0, 0.0, 180.0);
        for at in [Vec3::X, Vec3::new(0.3, -1.0, -0.2), -Vec3::Y] {
            assert_eq!(turn_to(truth, at, truth, Side::Bottom, Vec3::Z), Quat::IDENTITY, "camera {at}");
            // on a slight slope, only the slope's tilt
            let slope = Quat::from_rotation_x(0.1) * Vec3::Z;
            let t = turn_to(truth, at, truth, Side::Bottom, slope);
            assert!(t.angle_between(Quat::IDENTITY) < 0.1 + NOISE);
        }
    }

    #[test]
    fn every_placement_turns_from_the_truth_not_from_the_last_one() {
        let truth = euler(90.0, 0.0, 180.0);
        let at = Vec3::Y;
        // its back against a wall on its left, facing +X: a quarter turn
        let t1 = turn_to(truth, at, truth, Side::Back, Vec3::X);
        let on_wall = t1 * truth;
        assert!(
            (on_wall * side_axis(truth, at, Side::Back)).abs_diff_eq(-Vec3::X, 1e-5),
            "its back on the wall"
        );
        assert!(
            (on_wall * side_axis(truth, at, Side::Top)).abs_diff_eq(Vec3::Z, 1e-5),
            "still upright"
        );
        // then its bottom on the floor: it stands as it first did, not
        // turned as the wall left it
        let t2 = turn_to(truth, at, on_wall, Side::Bottom, Vec3::Z);
        assert!((t2 * on_wall).angle_between(truth) < NOISE);
        // the top onto the floor rolls about the view: its front stays in front
        let flipped = turn_to(truth, at, truth, Side::Top, Vec3::Z) * truth;
        assert!((flipped * side_axis(truth, at, Side::Top)).abs_diff_eq(-Vec3::Z, 1e-5));
        assert!((flipped * side_axis(truth, at, Side::Front)).abs_diff_eq(-Vec3::Y, 1e-5));
    }

    #[test]
    fn the_truth_is_kept_through_its_own_turns_and_read_again_after_others() {
        let mut place = Place::default();
        let sel = vec![ObjKey {
            region: Default::default(),
            local_id: 7,
        }];
        let first = euler(90.0, 0.0, 180.0);
        assert_eq!(place.truth(&sel, first, Vec3::X).rotation, first);
        // the tool turned it (and the network rounded it a little)
        let placed = Quat::from_rotation_z(1.0) * first;
        place.truths.get_mut(&sel).unwrap().last = placed;
        let echoed = (Quat::from_rotation_x(0.001) * placed).normalize();
        assert_eq!(place.truth(&sel, echoed, Vec3::Y).rotation, first);
        // the rotate handles turned it: that is the new truth, with the view then
        let by_hand = Quat::from_rotation_y(0.5) * echoed;
        let t = place.truth(&sel, by_hand, Vec3::Y);
        assert_eq!((t.rotation, t.camera_at), (by_hand, Vec3::Y));
    }

    #[test]
    fn a_chosen_side_turns_flat_against_the_surface() {
        let sel = cube(Vec3::ZERO, Vec3::new(0.1, 0.6, 0.4), Quat::IDENTITY);
        // a picture: its back (+X, away from a camera looking along +X)
        // against a wall facing -X, slanted 10°
        let wall = (Quat::from_rotation_y(0.17) * -Vec3::X).normalize();
        let t = turn_to(Quat::IDENTITY, Vec3::X, Quat::IDENTITY, Side::Back, wall);
        let l = landing(&sel, Vec3::new(3.0, 8.0, 2.0), wall, t, 0.0);
        assert!((l.to.rotation * Vec3::X).abs_diff_eq(-wall, 1e-5), "its back on the wall");
        assert!((l.to.center - Vec3::new(3.0, 8.0, 2.0)).abs_diff_eq(wall * 0.1, 1e-5));
        // its top against a ceiling facing down
        let t = turn_to(Quat::IDENTITY, Vec3::X, Quat::IDENTITY, Side::Top, -Vec3::Z);
        let up = landing(&sel, Vec3::new(0.0, 0.0, 4.0), -Vec3::Z, t, 0.0);
        assert!((up.to.rotation * Vec3::Z).abs_diff_eq(Vec3::Z, 1e-5));
        assert!((up.to.center.z - 3.6).abs() < 1e-5);
    }

    #[test]
    fn closing_the_tools_or_changing_tool_ends_the_preview() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut s = BuildSettings::default();
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let preview = || Some(landing(&sel, Vec3::Z, Vec3::Z, Quat::IDENTITY, 0.0));
        let mut tool = BuildTool {
            open: true,
            ..Default::default()
        };
        tool.place.active = true;
        tool.place.preview = preview();
        tool.close(&mut world, &mut s);
        assert!(!tool.place.active && tool.place.preview.is_none(), "closed");
        // another tool of the open window
        let mut tool = BuildTool {
            open: true,
            tool: Tool::Create,
            ..Default::default()
        };
        tool.place.active = true;
        tool.place.preview = preview();
        tool.update(&mut world, &s, None, (0.0, 0.0), false, Default::default());
        assert!(!tool.place.active && tool.place.preview.is_none(), "the Create tool");
    }

    #[test]
    fn sides_keep_their_names() {
        assert_eq!(serde_json::to_string(&Side::Bottom).unwrap(), "\"bottom\"");
        let s: BuildSettings = serde_json::from_str("{}").unwrap();
        assert_eq!((s.place_rotate, s.place_side, s.place_offset), (false, Side::Bottom, 0.0));
        // the first test builds' "As it is"
        let s: BuildSettings = serde_json::from_str(r#"{"place_side":"auto"}"#).unwrap();
        assert_eq!(s.place_side, Side::Bottom);
    }
}
