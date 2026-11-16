//! Snap to objects through the move handles, on the AURORA_DEMO_OBJECT_SNAP boxes.

use super::*;
use crate::demo::object_snap::FIRST_ID;
use aurora_net::NetEvent;

const SLAB: u32 = FIRST_ID;
const BLUE: u32 = FIRST_ID + 1;
const ORANGE: u32 = FIRST_ID + 2;
const GREEN: u32 = FIRST_ID + 3;
const PLANK: u32 = FIRST_ID + 4;

/// The demo world with the six snap boxes on open land; `only` keeps just
/// these of them.
fn world(only: Option<&[u32]>) -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for ev in crate::demo::events() {
        world.apply(ev);
    }
    for ev in crate::demo::object_snap::events_at(Vec3::new(80.0, 100.0, 0.0)) {
        let ev = match ev {
            NetEvent::ObjectUpdates { handle, mut objects } => {
                // the room and the cylinder are surfaces, which need the scene
                objects.retain(|o| only.map_or(o.local_id < FIRST_ID + 6, |only| only.contains(&o.local_id)));
                NetEvent::ObjectUpdates { handle, objects }
            }
            ev => ev,
        };
        world.apply(ev);
    }
    world
}

fn key(world: &World, id: u32) -> (usize, ObjKey) {
    let (idx, o) = world.objects.iter().find(|(_, o)| o.key.local_id == id).expect("demo box");
    (idx, o.key)
}

fn camera(at: Vec3) -> Cam {
    let eye = at + Vec3::new(4.0, -6.0, 3.0);
    let fov = 60f32.to_radians();
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 1280.0 / 720.0, 0.01);
    Cam::new(
        eye,
        at,
        projection * glam::camera::rh::view::look_at_mat4(eye, at, Vec3::Z),
        fov,
        1280.0,
        720.0,
        1.0,
    )
}

/// Grab `part` of the move handles of `sel`; the cursor where it was
/// grabbed, or None when the view hides that handle (an arrow pointing at
/// the camera).
fn grab(manip: &mut Manip, world: &mut World, s: &BuildSettings, cam: &Cam, sel: ObjKey, part: Part) -> Option<(f32, f32)> {
    let now = Instant::now();
    let f = frame(world, s, &[sel], false, now, manip.grid_ref.as_ref()).expect("frame");
    let g = f.grid.rotation;
    let len = arrow_length(cam, &f, f.pivot);
    // along an arrow, the first point not hidden by another handle
    let points: Vec<(f32, f32)> = match part {
        Part::Arrow(i) => [0.9, 0.75, 1.0, 0.6]
            .iter()
            .filter_map(|k| cam.project_px(f.pivot + g * unit(i) * len * k))
            .collect(),
        Part::Plane(i) => {
            let q = plane_handles(cam, &f)[i]?;
            let p = q[0].lerp(q[2], 0.5);
            vec![(p.x * cam.ppp, p.y * cam.ppp)]
        }
        _ => unreachable!("move handles only"),
    };
    let cursor = points.into_iter().find(|c| manip.pick(cam, &f, EditMode::Move, *c) == Some(part))?;
    assert!(manip.try_grab(world, s, cam, &[sel], false, EditMode::Move, cursor, now));
    assert_eq!(manip.drag.as_ref().map(|d| d.part), Some(part));
    Some(cursor)
}

fn drag(manip: &mut Manip, world: &mut World, s: &BuildSettings, cam: &Cam, sel: ObjKey, cursor: (f32, f32)) {
    let mut out = Vec::new();
    manip.update(
        world,
        s,
        cam,
        &[sel],
        false,
        EditMode::Move,
        cursor,
        false,
        Mods::default(),
        Instant::now(),
        &mut out,
        None,
    );
}

/// The cursor that drags the grabbed handle by `by` metres (in its plane).
fn along(cam: &Cam, manip: &Manip, by: Vec3) -> (f32, f32) {
    let drag = manip.drag.as_ref().unwrap();
    let p = cam.project_px(drag.start_hit + by).unwrap();
    if matches!(drag.part, Part::Plane(_)) {
        let offset = plane_cursor_offset(&drag.camera, drag.pivot, drag.start_cursor);
        (p.0 + offset.0, p.1 + offset.1)
    } else {
        p
    }
}

#[test]
fn a_plane_anchor_beyond_the_ray_horizon_drags_without_a_jump() {
    let mut w = world(Some(&[BLUE]));
    let s = BuildSettings {
        gizmo_scale: 3.0,
        snap: false,
        object_snap: false,
        ..Default::default()
    };
    let (blue, sel) = key(&w, BLUE);
    let start = w.objects.get(blue).unwrap().position;
    let eye = start + Vec3::new(4.0, -8.0, 8.0);
    let fov = 60f32.to_radians();
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 1280.0 / 720.0, 0.01);
    let cam = Cam::new(
        eye,
        start,
        projection * glam::camera::rh::view::look_at_mat4(eye, start, Vec3::Z),
        fov,
        1280.0,
        720.0,
        1.0,
    );
    let mut manip = Manip::default();
    let cursor = grab(&mut manip, &mut w, &s, &cam, sel, Part::Plane(0)).expect("displaced YZ anchor");
    let (origin, direction) = cam.ray(cursor.0, cursor.1);
    assert!(
        ray_plane(origin, direction, start, Vec3::X).is_none(),
        "the visible anchor is beyond the world-plane horizon"
    );
    drag(&mut manip, &mut w, &s, &cam, sel, cursor);
    assert_eq!(w.objects.get(blue).unwrap().position, start, "a held cursor never jumps");
    let by = Vec3::new(0.0, 0.4, -0.3);
    let cursor = along(&cam, &manip, by);
    drag(&mut manip, &mut w, &s, &cam, sel, cursor);
    assert!(w.objects.get(blue).unwrap().position.abs_diff_eq(start + by, 2e-3));
}

fn snapping() -> BuildSettings {
    BuildSettings {
        snap: false,
        object_snap: true,
        ..Default::default()
    }
}

#[test]
fn the_z_arrow_sits_the_cube_on_the_slab_within_a_few_pixels_only() {
    let mut w = world(None);
    let s = snapping();
    let (blue, sel) = key(&w, BLUE);
    let (slab, _) = key(&w, SLAB);
    let top = w.objects.get(slab).unwrap().position.z + 0.1;
    let start = w.objects.get(blue).unwrap().position;
    let cam = camera(start);
    // the cube floats 0.3 m over the slab
    assert!((start.z - 0.5 - top - 0.3).abs() < 1e-4);
    for (pixels, snaps) in [(3.0, true), (6.0, true), (20.0, false)] {
        let mut manip = Manip::default();
        grab(&mut manip, &mut w, &s, &cam, sel, Part::Arrow(2)).expect("handle in view");
        let short = 0.3 - cam.meters_for_pixels(start, pixels);
        let cursor = along(&cam, &manip, -Vec3::Z * short);
        drag(&mut manip, &mut w, &s, &cam, sel, cursor);
        let now = w.objects.get(blue).unwrap().position;
        assert_eq!((now.x, now.y), (start.x, start.y), "the arrow keeps its axis");
        let guides = &manip.drag.as_ref().unwrap().snap_guides;
        if snaps {
            assert!((now.z - 0.5 - top).abs() < 1e-4, "{pixels} px: resting on the slab, {now}");
            assert!(guides.iter().any(|g| g.contact && g.kind == object_snap::Kind::Touch));
        } else {
            assert!(
                (now.z - (start.z - short)).abs() < 2e-3,
                "{pixels} px: free, {now} vs {}: {guides:?}",
                start.z - short
            );
            assert!(guides.is_empty());
        }
        manip.cancel();
        w.objects.get_mut(blue).unwrap().position = start;
    }
}

#[test]
fn the_x_arrow_touches_the_neighbour_and_stops_there() {
    // short of it, then far past it
    for (past, pixels) in [(false, 4.0), (true, 0.0)] {
        let mut w = world(None);
        let s = snapping();
        let (blue, sel) = key(&w, BLUE);
        let (orange, _) = key(&w, ORANGE);
        let start = w.objects.get(blue).unwrap().position;
        let face = w.objects.get(orange).unwrap().position.x - 0.5;
        let cam = camera(start);
        let mut manip = Manip::default();
        grab(&mut manip, &mut w, &s, &cam, sel, Part::Arrow(0)).expect("handle in view");
        let gap = face - (start.x + 0.5);
        let by = if past {
            gap + 3.0
        } else {
            gap - cam.meters_for_pixels(start, pixels)
        };
        let cursor = along(&cam, &manip, Vec3::X * by);
        drag(&mut manip, &mut w, &s, &cam, sel, cursor);
        let now = w.objects.get(blue).unwrap().position;
        assert!((now.x + 0.5 - face).abs() < 1e-4, "past {past}: touching, {now}");
        assert_eq!((now.y, now.z), (start.y, start.z));
        assert!(manip.drag.as_ref().unwrap().snap_guides.iter().any(|g| g.contact));
        // back out again
        let cursor = along(&cam, &manip, Vec3::X * -0.5);
        drag(&mut manip, &mut w, &s, &cam, sel, cursor);
        let back = w.objects.get(blue).unwrap().position;
        assert!((back.x - (start.x - 0.5)).abs() < 2e-3, "past {past}: free again, {back}");
    }
}

#[test]
fn a_plane_handle_puts_the_cube_edge_to_edge() {
    {
        let mut w = world(Some(&[SLAB, BLUE, ORANGE]));
        let s = snapping();
        let (blue, sel) = key(&w, BLUE);
        let (orange, _) = key(&w, ORANGE);
        let o = w.objects.get(orange).unwrap().position;
        let start = w.objects.get(blue).unwrap().position;
        let cam = camera(start);
        let mut manip = Manip::default();
        grab(&mut manip, &mut w, &s, &cam, sel, Part::Plane(2)).expect("handle in view");
        // 3 px short of touching, 3 px off flush sides
        let near = cam.meters_for_pixels(start, 3.0);
        let by = Vec3::new(o.x - 1.0 - start.x - near, o.y - start.y + near, 0.0);
        let cursor = along(&cam, &manip, by);
        drag(&mut manip, &mut w, &s, &cam, sel, cursor);
        let now = w.objects.get(blue).unwrap().position;
        assert!((now.x - (o.x - 1.0)).abs() < 1e-4, "touching, {now}");
        assert!((now.y - o.y).abs() < 1e-4, "flush sides, {now}");
        assert_eq!(now.z, start.z, "in the handle's plane");
        let d = manip.drag.as_ref().unwrap();
        assert_eq!(d.snap_guides.len(), 2);
        assert!(!d.in_snap, "the object snap, not the grid");
    }
}

#[test]
fn the_z_arrow_never_pushes_the_cube_through_the_slab() {
    let mut w = world(None);
    let s = snapping();
    let (blue, sel) = key(&w, BLUE);
    let (slab, _) = key(&w, SLAB);
    let top = w.objects.get(slab).unwrap().position.z + 0.1;
    let start = w.objects.get(blue).unwrap().position;
    let cam = camera(start);
    let mut manip = Manip::default();
    grab(&mut manip, &mut w, &s, &cam, sel, Part::Arrow(2)).expect("handle in view");
    // well into the slab, then right through and under it
    for down in [0.6, 1.5, 4.0] {
        let cursor = along(&cam, &manip, -Vec3::Z * down);
        drag(&mut manip, &mut w, &s, &cam, sel, cursor);
        let now = w.objects.get(blue).unwrap().position;
        assert!((now.z - 0.5 - top).abs() < 1e-4, "{down} m down: on the slab, {now}");
    }
    // Grid or no snap: the Legacy Viewer's move goes through
    for mode in [super::super::SnapMode::Grid, super::super::SnapMode::Off] {
        let mut s = s.clone();
        s.set_snap_mode(mode);
        w.objects.get_mut(blue).unwrap().position = start;
        let mut manip = Manip::default();
        grab(&mut manip, &mut w, &s, &cam, sel, Part::Arrow(2)).expect("handle in view");
        let cursor = along(&cam, &manip, -Vec3::Z * 1.5);
        drag(&mut manip, &mut w, &s, &cam, sel, cursor);
        assert!(w.objects.get(blue).unwrap().position.z < top, "{mode:?}: through the slab");
    }
}

#[test]
fn grid_and_object_snap_are_one_choice() {
    use super::super::SnapMode;
    let mut s = BuildSettings::default();
    assert_eq!(s.snap_mode(), SnapMode::Grid, "the Legacy Viewer's default");
    for mode in [SnapMode::Objects, SnapMode::Off, SnapMode::Grid, SnapMode::Objects] {
        s.set_snap_mode(mode);
        assert_eq!(s.snap_mode(), mode);
        assert!(!(s.snap && s.object_snap));
    }
    for choice in [SnapMode::Objects, SnapMode::Grid] {
        s.set_snap_mode(choice);
        s.set_snap_enabled(false);
        assert_eq!(s.snap_mode(), SnapMode::Off);
        assert_eq!(s.snap_choice(), choice);
        let text = serde_json::to_string(&s).unwrap();
        let mut restored: BuildSettings = serde_json::from_str(&text).unwrap();
        restored.set_snap_enabled(true);
        assert_eq!(restored.snap_mode(), choice);
    }
    // a settings file with both on snaps to the grid, never to both
    s.snap = true;
    s.object_snap = true;
    assert_eq!(s.snap_mode(), SnapMode::Grid);
}

/// Setting off, nothing in reach, or the HUD: every handle, grid mode and
/// grid snap state moves exactly as without the snap boxes around.
#[test]
fn without_a_target_in_reach_every_move_is_unchanged() {
    let parts = [
        Part::Arrow(0),
        Part::Arrow(1),
        Part::Arrow(2),
        Part::Plane(0),
        Part::Plane(1),
        Part::Plane(2),
    ];
    let offsets = [
        (30.0, 0.0),
        (0.0, 30.0),
        (-40.0, 25.0),
        (60.0, -45.0),
        (5.0, 160.0),
        (-120.0, -90.0),
    ];
    let mut checked = 0;
    for selected in [BLUE, GREEN] {
        let mut full = world(None);
        let mut alone = world(Some(&[selected]));
        for grid_mode in [0u8, 1, 2] {
            for grid in [false, true] {
                for part in parts {
                    for (dx, dy) in offsets {
                        let mut positions = Vec::new();
                        for (in_full, object_snap) in [(true, false), (false, false), (false, true)] {
                            let w = if in_full { &mut full } else { &mut alone };
                            let s = BuildSettings {
                                snap: grid,
                                object_snap,
                                grid_mode,
                                ..Default::default()
                            };
                            let (idx, sel) = key(w, selected);
                            let start = w.objects.get(idx).unwrap().position;
                            let cam = camera(start);
                            let mut manip = Manip {
                                // "Reference": the grid of a box turned like the green one
                                grid_ref: (grid_mode == 2).then_some(Bounds {
                                    center: start + Vec3::new(3.0, 1.0, 0.5),
                                    rotation: Quat::from_rotation_z(0.5),
                                    half: Vec3::splat(0.5),
                                }),
                                ..Default::default()
                            };
                            let Some(at) = grab(&mut manip, w, &s, &cam, sel, part) else {
                                continue;
                            };
                            drag(&mut manip, w, &s, &cam, sel, (at.0 + dx, at.1 + dy));
                            let d = manip.drag.as_ref().unwrap();
                            assert!(d.snap_guides.is_empty());
                            positions.push((w.objects.get(idx).unwrap().position, d.in_snap));
                            manip.cancel();
                            w.objects.get_mut(idx).unwrap().position = start;
                        }
                        if positions.is_empty() {
                            continue;
                        }
                        assert_eq!(positions.len(), 3, "the same handles in every world");
                        // The same move up to float rounding: the cursor ray is cast
                        // in a different world, and on Apple's CPU the result drifts
                        // by up to a tenth of a millimetre from run to run. A snap
                        // moves the box by centimetres, far above this 1 mm.
                        for pair in positions.windows(2) {
                            let ((a, a_snap), (b, b_snap)) = (pair[0], pair[1]);
                            assert!(
                                a.abs_diff_eq(b, 1e-3) && a_snap == b_snap,
                                "{selected} {part:?} grid {grid} mode {grid_mode} ({dx}, {dy}): {a} {a_snap} vs {b} {b_snap}"
                            );
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    // only the arrow a turned grid points at the camera is out of reach
    assert!(checked >= 2 * 3 * 2 * 6 * 6 * 4 / 5, "{checked} moves checked");
}

#[test]
fn the_moved_selection_and_worn_objects_are_never_targets() {
    let w = world(None);
    let (blue, sel) = key(&w, BLUE);
    let (plank, _) = key(&w, PLANK);
    let now = Instant::now();
    let around = bounds_of(&w, &[sel], false, now).unwrap();
    let moving: HashSet<usize> = [blue].into();
    let targets = object_snap::collect(&w, None, &moving, &around, now, None);
    let has = |idx: usize| {
        let p = w.objects.get(idx).unwrap().position;
        targets.boxes.iter().any(|b| b.center == p)
    };
    assert!(!has(blue));
    assert!(has(plank));
    assert!(targets.boxes.len() <= object_snap::MAX_TARGETS);
    for (idx, _) in w.objects.iter() {
        if super::super::select_type(&w, idx) != super::super::SelectType::World && has(idx) {
            panic!("worn object {idx} is a target");
        }
    }
}

/// A drag collects its targets again as it goes: a prim that has not moved
/// keeps what was read of it (its triangles are not read again), a prim
/// that moved is read again.
#[test]
fn collected_again_a_prim_that_has_not_moved_is_not_read_again() {
    let (mut w, mut scene, front, back) = crate::scene::picking::tests::scene_with_two_objects();
    let now = Instant::now();
    let around = Bounds {
        center: Vec3::new(5.0, 0.0, 50.0),
        rotation: Quat::IDENTITY,
        half: Vec3::splat(0.5),
    };
    let none = HashSet::new();
    let count = |t: &object_snap::Targets| t.boxes.len() + t.surfaces.len();
    let first = object_snap::collect(&w, Some(&scene), &none, &around, now, None);
    assert_eq!(count(&first), 2, "both prims show a face");
    // their faces no longer drawn: unmoved, they are kept as they were read
    for idx in [front, back] {
        for f in &mut scene.gpu[idx].faces {
            f.pass = crate::scene::Pass::Hidden;
        }
    }
    let again = object_snap::collect(&w, Some(&scene), &none, &around, now, Some(first.clone()));
    assert_eq!(count(&again), 2);
    // moved, a prim is read again (and shows nothing now)
    w.objects.get_mut(front).unwrap().position.y += 0.5;
    let moved = object_snap::collect(&w, Some(&scene), &none, &around, now, Some(again));
    assert_eq!(count(&moved), 1);
    // and with nothing before, nothing is assumed
    assert_eq!(count(&object_snap::collect(&w, Some(&scene), &none, &around, now, None)), 0);
}

/// The move hologram is off by default (a move looks as the Legacy
/// Viewer's), and once a move has started it has the prims moved, the box
/// where it began and how far it went.
#[test]
fn a_move_gives_its_hologram_what_it_moves_and_from_where() {
    let s = BuildSettings::default();
    assert_eq!(s.move_hologram, super::super::MoveHologram::Off);
    assert_eq!(serde_json::to_string(&super::super::MoveHologram::Start).unwrap(), "\"start\"");
    let mut w = world(None);
    let (blue, sel) = key(&w, BLUE);
    let start = w.objects.get(blue).unwrap().position;
    let cam = camera(start);
    let mut manip = Manip::default();
    assert!(manip.move_ghost().is_none(), "no move");
    let Some(_) = grab(&mut manip, &mut w, &s, &cam, sel, Part::Arrow(0)) else {
        panic!("the X arrow is in view");
    };
    let cursor = along(&cam, &manip, Vec3::X * 1.5);
    drag(&mut manip, &mut w, &s, &cam, sel, cursor);
    let ghost = manip.move_ghost().expect("a move under way");
    assert_eq!(ghost.prims, vec![blue]);
    assert!(ghost.start.center.abs_diff_eq(start, 1e-4), "the box where it began");
    let now = w.objects.get(blue).unwrap().position;
    assert!((now - start).abs_diff_eq(ghost.delta, 1e-4), "how far it went");
    assert!(ghost.delta.x > 1.0);
}
