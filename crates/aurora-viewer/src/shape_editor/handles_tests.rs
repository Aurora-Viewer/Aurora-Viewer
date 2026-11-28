use super::*;

fn library() -> AvatarLibrary {
    AvatarLibrary::load()
}

fn name(lib: &AvatarLibrary, anchor: Anchor) -> String {
    format!("{}{}", lib.rig.canonical_names[anchor.joint], anchor.local)
}

fn grip(index: usize, handle: &Handle) -> Grip {
    Grip {
        handle: index,
        mirrored: false,
        anchor: handle.anchor,
        params: handle.params.clone(),
        resize: handle.resize,
    }
}

fn handle_of(layout: &Layout, id: i32) -> &Handle {
    layout
        .handles
        .iter()
        .find(|h| h.params.contains(&id))
        .unwrap_or_else(|| panic!("slider {id} has a handle"))
}

/// Front view, 500 points per metre: the avatar faces +X, its left (+Y)
/// on the screen's right as seen from the front, up is up.
fn front(p: Vec3) -> Option<Vec2> {
    Some(Vec2::new(p.y * -500.0, -p.z * 500.0))
}

/// Side view from the avatar's left.
fn side(p: Vec3) -> Option<Vec2> {
    Some(Vec2::new(p.x * 500.0, -p.z * 500.0))
}

fn rest_body(lib: &AvatarLibrary) -> Body<'_> {
    Body::new(
        lib,
        &[],
        Pose {
            rot: lib.rig.local_rot.clone(),
            model: Mat4::IDENTITY,
        },
    )
}

/// Screen points a slider moves the dot over its whole range.
fn reach(
    body: &Body,
    sliders: &BTreeMap<i32, f32>,
    anchor: Anchor,
    params: &[(i32, f32, f32)],
    project: impl Fn(Vec3) -> Option<Vec2>,
) -> f32 {
    params
        .iter()
        .filter_map(|(id, min, max)| {
            let mut lo = sliders.clone();
            lo.insert(*id, *min);
            let mut hi = sliders.clone();
            hi.insert(*id, *max);
            Some(project(body.point(&lo, anchor)?)?.distance(project(body.point(&hi, anchor)?)?))
        })
        .fold(0.0, f32::max)
}

fn defaults(male: bool) -> BTreeMap<i32, f32> {
    let mut sliders: BTreeMap<i32, f32> = catalog::PARAMS.iter().map(|p| (p.id, p.default)).collect();
    sliders.insert(PARAM_MALE, f32::from(u8::from(male)));
    sliders
}

#[test]
fn the_body_sliders_find_handles_on_the_volumes_they_shape() {
    let lib = library();
    for male in [false, true] {
        let layout = derive(&lib, male, Part::Body, false);
        for handle in &layout.handles {
            let labels: Vec<_> = handle
                .params
                .iter()
                .filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == *id))
                .map(|p| p.labels().0)
                .collect();
            eprintln!(
                "{} {}{} (twin {:?}): {labels:?}",
                if male { "male" } else { "female" },
                if handle.resize { "resize " } else { "" },
                name(&lib, handle.anchor),
                handle.mirror.map(|m| name(&lib, m))
            );
            assert!((1..=2).contains(&handle.params.len()));
            // canonical side: never a right-side volume
            assert!(!is_right(&lib.rig.canonical_names[handle.anchor.joint]));
        }
        let ids: Vec<i32> = layout.handles.iter().flat_map(|h| h.params.clone()).collect();
        let unique: HashSet<i32> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "a slider has one handle");
        for left_out in LEFT_OUT {
            assert!(!unique.contains(&left_out));
        }
        let on = |id: i32| lib.rig.canonical_names[handle_of(&layout, id).anchor.joint].clone();
        // Butt Size drives the butt and the pelvis volumes
        assert!(["BUTT", "PELVIS"].contains(&on(795).as_str()), "Butt Size: {}", on(795));
        assert!(on(693).contains("ARM") || on(693).contains("HAND"), "Arm Length: {}", on(693));
        // standing on the floor: the top of the body rises with Height, and
        // no dot of a length slider sits on the feet, which stay down
        assert_eq!(on(33), "HEAD", "Height");
        assert!(
            !on(692).contains("FOOT") && !on(692).contains("LOWER_LEG"),
            "Leg Length: {}",
            on(692)
        );
        // wider hips shift the whole leg: the dot sits on the thigh
        assert!(on(37).contains("UPPER_LEG") || on(37) == "PELVIS", "Hip Width: {}", on(37));
        if !male {
            assert_eq!(on(105), "LEFT_PEC", "Breast Size");
            // the breasts' lift and spacing have a dot of their own
            for id in [507, 684] {
                assert!(on(id).contains("PEC"), "slider {id}: {}", on(id));
            }
        }
        // two sliders that push the same point the same way each get one:
        // Saddle Bags beside Hip Width, on the pelvis or the thigh
        assert!(
            on(753).contains("PELVIS") || on(753).contains("UPPER_LEG") || on(753) == "BUTT",
            "Saddle Bags: {}",
            on(753)
        );
        assert_ne!(handle_of(&layout, 753).anchor, handle_of(&layout, 37).anchor);
        // a side volume shows on both sides
        let legs = handle_of(&layout, 692);
        assert!(legs.mirror.is_some_and(|m| is_right(&lib.rig.canonical_names[m.joint])));
        // most body sliders get a handle
        assert!(unique.len() >= 15, "{} sliders placed", unique.len());
    }
}

#[test]
fn the_layout_is_the_same_each_time() {
    let lib = library();
    assert_eq!(
        derive(&lib, false, Part::Body, false).handles,
        derive(&lib, false, Part::Body, false).handles
    );
}

#[test]
fn twins_mirror_across_the_avatar() {
    assert_eq!(mirror_name("L_UPPER_LEG").as_deref(), Some("R_UPPER_LEG"));
    assert_eq!(mirror_name("RIGHT_PEC").as_deref(), Some("LEFT_PEC"));
    assert_eq!(mirror_name("mHipLeft").as_deref(), Some("mHipRight"));
    assert_eq!(mirror_name("BELLY"), None);
    let lib = library();
    let pelvis = lib.rig.names["PELVIS"];
    let side = Anchor::on(pelvis, Vec3::NEG_Y);
    let (canon, twin) = canonical(&lib.rig, side);
    assert_eq!(canon.local, Vec3::Y);
    assert_eq!(twin, Some(side));
    let front = Anchor::on(pelvis, Vec3::X);
    assert_eq!(canonical(&lib.rig, front), (front, None));
    let right = Anchor::on(lib.rig.names["R_UPPER_LEG"], Vec3::NEG_Y);
    let (canon, twin) = canonical(&lib.rig, right);
    assert_eq!(lib.rig.canonical_names[canon.joint], "L_UPPER_LEG");
    assert_eq!(canon.local, Vec3::Y);
    assert_eq!(twin, Some(right));
}

#[test]
fn a_mesh_body_keeps_only_the_sliders_its_skin_follows() {
    let lib = library();
    let layout = derive(&lib, false, Part::Body, false);
    let butt = handle_of(&layout, 795);
    let all: Vec<i32> = shown_params(&layout, butt, true, &HashSet::new()).collect();
    assert_eq!(all, butt.params);
    // a mesh skinned to the bones alone does not follow a volume morph
    let bones: HashSet<usize> = (0..lib.rig.len())
        .filter(|j| lib.rig.canonical_names[*j].starts_with('m'))
        .collect();
    assert!(!shown_params(&layout, butt, false, &bones).any(|id| id == 795));
    // a fitted mesh does
    let fitted: HashSet<usize> = [lib.rig.names["BUTT"]].into();
    assert!(shown_params(&layout, butt, false, &fitted).any(|id| id == 795));
    // a skeleton slider moves the bones a classic rigged mesh follows
    let leg = handle_of(&layout, 692);
    assert!(shown_params(&layout, leg, false, &bones).any(|id| id == 692));
}

#[test]
fn damped_least_squares_matches_the_cursor_and_stays_bounded() {
    // one slider moving the point 100 points per unit, sideways
    let step = damped_step(&[Vec2::new(100.0, 0.0)], Vec2::new(50.0, 0.0), 0.0);
    assert!((step[0] - 0.5).abs() < 1e-5);
    // the part of the move it cannot make is left out
    let step = damped_step(&[Vec2::new(100.0, 0.0)], Vec2::new(0.0, 50.0), 0.0);
    assert!(step[0].abs() < 1e-5);
    // two sliders, two directions: both solved
    let step = damped_step(&[Vec2::new(100.0, 0.0), Vec2::new(0.0, 20.0)], Vec2::new(10.0, 10.0), 0.0);
    assert!((step[0] - 0.1).abs() < 1e-4 && (step[1] - 0.5).abs() < 1e-4);
    // seen along its motion: a tiny column gets a damped step, not a huge one
    let step = damped_step(&[Vec2::new(0.01, 0.0)], Vec2::new(50.0, 0.0), 5.0);
    assert!(step[0].abs() < 0.05, "{step:?}");
    assert_eq!(damped_step(&[Vec2::ZERO], Vec2::new(5.0, 5.0), 0.0), vec![0.0]);
}

#[test]
fn a_slider_at_its_limit_hands_the_rest_of_the_move_to_the_other() {
    let columns = [Vec2::new(100.0, 0.0), Vec2::new(100.0, 0.0)];
    let step = clamped_step(&columns, Vec2::new(100.0, 0.0), 1e-3, &[(0.9, 0.0, 1.0), (0.0, 0.0, 1.0)]);
    assert!((step[0] - 0.1).abs() < 1e-4, "{step:?}");
    assert!((step[1] - 0.9).abs() < 1e-4, "{step:?}");
}

#[test]
fn the_pose_read_back_from_the_drawn_joints_draws_them_again() {
    let lib = library();
    let rig = &lib.rig;
    let rot: Vec<Quat> = (0..rig.len())
        .map(|j| {
            Quat::from_euler(
                glam::EulerRot::XYZ,
                0.1 * (j % 3) as f32,
                0.05 * (j % 5) as f32,
                -0.07 * (j % 2) as f32,
            )
        })
        .collect();
    let model = Mat4::from_rotation_translation(Quat::from_rotation_z(0.7), Vec3::new(128.0, 64.0, 22.0));
    let shape = lib.shape_params.shape(&[], rig);
    let joints: Vec<Mat4> = rig
        .world_with(&rot, Vec3::ZERO, &shape.local_pos, &shape.scale)
        .into_iter()
        .map(|m| model * m)
        .collect();
    let pose = Pose::from_joints(rig, model, &joints).expect("pose");
    let again = rig.world_with(&pose.rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
    for (a, b) in joints.iter().zip(&again) {
        assert!(a.w_axis.distance(model * b.w_axis) < 1e-3);
    }
}

#[test]
fn a_drag_moves_the_sliders_until_the_dot_follows_the_pointer() {
    let lib = library();
    let layout = derive(&lib, false, Part::Body, false);
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    for (id, view, pull) in [
        // Butt Size seen from the side: back and out
        (795, side as fn(Vec3) -> Option<Vec2>, Vec2::new(-12.0, 0.0)),
        // Hip Width from the front
        (37, front, Vec2::new(-10.0, 0.0)),
        // Leg Length from the front: the feet stay down, the body rises
        (692, front, Vec2::new(0.0, -15.0)),
    ] {
        let sliders = defaults(false);
        let index = layout.handles.iter().position(|h| h.params.contains(&id)).expect("handle");
        let handle = &layout.handles[index];
        let start = body.point(&sliders, handle.anchor).expect("point");
        let reach = reach(
            &body,
            &sliders,
            handle.anchor,
            &[(id, ranges(id).unwrap().0, ranges(id).unwrap().1)],
            view,
        );
        assert!(reach > 10.0, "slider {id} moves its dot {reach} points");
        let pointer = view(start).unwrap();
        let drag = Drag::start(&body, &sliders, grip(index, handle), start, looking(view), view, pointer).expect("drag");
        let before = sliders.clone();
        // the pull's sign is unknown here: try both ways, one grows the slider
        let mut moved_any = false;
        for sign in [1.0, -1.0] {
            let mut trial = before.clone();
            let target = pointer + pull * sign;
            for _ in 0..10 {
                drag.step(&body, &mut trial, ranges, view, target);
            }
            let at = view(body.point(&trial, handle.anchor).unwrap()).unwrap();
            let (min, max) = ranges(id).unwrap();
            assert!((min..=max).contains(&trial[&id]));
            if trial != before {
                moved_any = true;
                // within the range, the dot comes as near the pointer as the
                // slider's own direction allows: the rest of the pull is
                // across that direction, or the slider is at its limit
                let limited = trial[&id] <= min + 1e-4 || trial[&id] >= max - 1e-4;
                let moved = at - pointer;
                let along = if moved.length() > 1e-3 { moved.normalize() } else { Vec2::ZERO };
                let off = (target - at) - along * (target - at).dot(along);
                assert!(
                    at.distance(target) < 2.0 || limited || (target - at).dot(along).abs() < 1.0,
                    "slider {id}: dot at {at}, target {target}, across {off}"
                );
                assert!(at.distance(target) < pull.length(), "slider {id}: the dot comes nearer");
            }
        }
        assert!(moved_any, "slider {id} follows a drag");
    }
}

#[test]
fn a_drag_past_the_range_stops_at_the_limit() {
    let lib = library();
    let layout = derive(&lib, false, Part::Body, false);
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    let index = layout.handles.iter().position(|h| h.params.contains(&795)).expect("butt");
    let handle = &layout.handles[index];
    let mut sliders = defaults(false);
    let start = body.point(&sliders, handle.anchor).unwrap();
    let pointer = side(start).unwrap();
    let drag = Drag::start(&body, &sliders, grip(index, handle), start, looking(side), side, pointer).unwrap();
    for sign in [1.0, -1.0] {
        let mut trial = sliders.clone();
        for _ in 0..20 {
            drag.step(&body, &mut trial, ranges, side, pointer + Vec2::new(5000.0 * sign, 0.0));
        }
        let (min, max) = ranges(795).unwrap();
        assert!((min..=max).contains(&trial[&795]));
        sliders = trial;
    }
}

#[test]
fn the_model_point_matches_the_drawn_skeleton_of_the_same_shape() {
    let lib = library();
    let body = rest_body(&lib);
    let sliders = defaults(false);
    let layout = derive(&lib, false, Part::Body, false);
    let shape = lib
        .shape_params
        .shape_weights(lib.shape_params.weights_with(&[], &sliders), &lib.rig);
    let joints = lib.rig.world_with(&lib.rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
    let ground = ["mFootLeft", "mFootRight", "mToeLeft", "mToeRight"]
        .iter()
        .map(|n| joints[lib.rig.names[*n]].w_axis.z)
        .fold(f32::INFINITY, f32::min);
    for handle in &layout.handles {
        let drawn = handle.anchor.at(&joints).unwrap() - Vec3::Z * ground;
        assert!(body.point(&sliders, handle.anchor).unwrap().distance(drawn) < 1e-4);
    }
}

#[test]
fn a_track_runs_from_the_slider_minimum_to_its_maximum_through_the_dot() {
    let lib = library();
    let layout = derive(&lib, false, Part::Body, false);
    let body = rest_body(&lib);
    let sliders = defaults(false);
    for id in [33, 795, 105] {
        let handle = handle_of(&layout, id);
        let param = catalog::PARAMS.iter().find(|p| p.id == id).expect("slider");
        let path = track(&body, &sliders, handle.anchor, id, (param.min, param.max), 12);
        assert_eq!(path.len(), 12);
        let at = |w: f32| {
            let mut values = sliders.clone();
            values.insert(id, w);
            body.point(&values, handle.anchor).expect("point")
        };
        assert!(path[0].distance(at(param.min)) < 1e-5, "slider {id} starts at its minimum");
        assert!(path[11].distance(at(param.max)) < 1e-5, "slider {id} ends at its maximum");
        // the dot, at its current value, lies on the track
        let here = at(sliders[&id]);
        let nearest = path
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let t = ((here - a).dot(b - a) / (b - a).length_squared().max(1e-12)).clamp(0.0, 1.0);
                here.distance(a + (b - a) * t)
            })
            .fold(f32::INFINITY, f32::min);
        assert!(nearest < 0.002, "slider {id}: the dot is {nearest} m off its track");
    }
}

#[test]
fn hands_and_feet_resize_where_legs_and_hips_slide() {
    let lib = library();
    for male in [false, true] {
        let layout = derive(&lib, male, Part::Body, false);
        for handle in &layout.handles {
            eprintln!(
                "{} {}: {:?} {}",
                if male { "male" } else { "female" },
                name(&lib, handle.anchor),
                handle.params,
                if handle.resize { "resize" } else { "slide" }
            );
        }
        assert!(handle_of(&layout, 675).resize, "Hand Size resizes the hand");
        for slide in [692, 37, 33, 693] {
            assert!(!handle_of(&layout, slide).resize, "slider {slide} slides");
        }
    }
}

#[test]
fn a_part_outline_is_its_ellipsoid_seen_across_the_view() {
    let volume = Mat4::from_scale_rotation_translation(Vec3::new(2.0, 2.0, 0.5), Quat::IDENTITY, Vec3::new(1.0, 0.0, 0.0));
    let ring = outline(volume, Vec3::Z, 16);
    assert_eq!(ring.len(), 16);
    for p in &ring {
        // seen from above: the circle of radius 2 in the XY plane
        assert!(p.z.abs() < 1e-5);
        assert!(((*p - Vec3::new(1.0, 0.0, 0.0)).length() - 2.0).abs() < 1e-4);
    }
    // seen from the side, the flat axis shows
    let side = outline(volume, Vec3::X, 32);
    let height = side.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    assert!((height - 0.5).abs() < 0.05, "{height}");
}

#[test]
fn a_resize_follows_the_pointer_distance_from_the_part() {
    // a hand pulled away from its centre on another side than its dot's,
    // seen from the front (its dot over its centre) or the side, grows;
    // pushed back towards the centre, it shrinks
    let lib = library();
    let layout = derive(&lib, false, Part::Body, false);
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    let index = layout.handles.iter().position(|h| h.params.contains(&675)).expect("hand");
    let handle = &layout.handles[index];
    let sliders = defaults(false);
    let (min, max) = ranges(675).unwrap();
    let start = body.point(&sliders, handle.anchor).unwrap();
    let pull = |drag: &Drag, view: fn(Vec3) -> Option<Vec2>, target: Vec2| {
        let mut trial = sliders.clone();
        for _ in 0..12 {
            drag.step(&body, &mut trial, ranges, view, target);
        }
        trial[&675]
    };
    // pressed on the outline (the hand's tip, seen from the front): along
    // the way out, it grows; pushed in, even past the centre, it shrinks
    // all the way to the smallest hand
    let centre = front(body.point(&sliders, Anchor::on(handle.anchor.joint, Vec3::ZERO)).unwrap()).unwrap();
    let pointer = front(start).unwrap();
    let out = (pointer - centre).normalize();
    let drag = Drag::start(&body, &sliders, grip(index, handle), start, looking(front), front, pointer).unwrap();
    assert!(
        pull(&drag, front, pointer + out * 30.0) > sliders[&675],
        "pulled out, the hand grows"
    );
    // any way away from the centre counts, not only the way it was pressed
    let across = Vec2::new(-out.y, out.x);
    assert!(
        pull(&drag, front, pointer + across * 40.0) > sliders[&675],
        "pulled away sideways, the hand grows"
    );
    assert!(
        pull(&drag, front, pointer - out * 8.0) < sliders[&675],
        "pushed in, the hand shrinks"
    );
    let smallest = pull(&drag, front, centre - out * 400.0);
    assert!(
        smallest - min < (max - min) * 0.02,
        "past the centre it reaches the smallest: {smallest}"
    );
    // pressed on the centre (the tip seen end on, from the side): no way
    // leads out, so it scrubs: up and right grow, down and left shrink
    let pointer = side(start).unwrap();
    let drag = Drag::start(&body, &sliders, grip(index, handle), start, looking(side), side, pointer).unwrap();
    assert!(
        pull(&drag, side, pointer + Vec2::new(20.0, -20.0)) > sliders[&675],
        "up and right, it grows"
    );
    assert!(
        pull(&drag, side, pointer + Vec2::new(-20.0, 20.0)) < sliders[&675],
        "down and left, it shrinks"
    );
}

/// The way a test projection looks along: the axis it flattens.
fn looking(view: fn(Vec3) -> Option<Vec2>) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .find(|axis| view(*axis).is_some_and(|p| p.length() < 1e-6))
        .unwrap_or(Vec3::X)
}

#[test]
fn face_sliders_find_handles_on_the_face_bones() {
    let lib = library();
    // the face sliders that only reshape the system head's mesh
    let mesh_only = [5, 12, 13, 22, 518, 664];
    for male in [false, true] {
        let layout = derive(&lib, male, Part::Head, false);
        assert_eq!(layout.part, Part::Head);
        for handle in &layout.handles {
            let labels: Vec<_> = handle
                .params
                .iter()
                .filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == *id))
                .map(|p| (p.id, p.labels().0))
                .collect();
            eprintln!(
                "{} head {}: {labels:?} {}",
                if male { "male" } else { "female" },
                name(&lib, handle.anchor),
                if handle.resize { "resize" } else { "slide" }
            );
            assert!((1..=2).contains(&handle.params.len()));
            assert!(!is_right(&lib.rig.canonical_names[handle.anchor.joint]));
        }
        let ids: Vec<i32> = layout.handles.iter().flat_map(|h| h.params.clone()).collect();
        let unique: HashSet<i32> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "a slider has one handle");
        for id in mesh_only {
            assert!(!unique.contains(&id), "mesh-only slider {id} has no bone to hold");
        }
        for id in &unique {
            let group = catalog::PARAMS.iter().find(|p| p.id == *id).expect("slider").group;
            assert!(Part::Head.groups().contains(&group), "slider {id} is a face slider");
        }
        let on = |id: i32| lib.rig.canonical_names[handle_of(&layout, id).anchor.joint].clone();
        assert!(on(517).contains("Nose"), "Nose Width: {}", on(517));
        assert!(on(196).contains("Eye"), "Eye Spacing: {}", on(196));
        assert!(on(35).contains("Ear"), "Ear Size: {}", on(35));
        assert!(
            handle_of(&layout, 35)
                .mirror
                .is_some_and(|m| is_right(&lib.rig.canonical_names[m.joint])),
            "both ears"
        );
        // most face sliders get a handle
        assert!(unique.len() >= 30, "{} face sliders placed", unique.len());
        // no dot inside the mouth
        for handle in &layout.handles {
            let bone = &lib.rig.canonical_names[handle.anchor.joint];
            assert!(!bone.contains("Tongue") && !bone.contains("Teeth"), "{bone}");
        }
    }
}

/// On the system head the face dots sit on its mesh, where each slider's
/// morph shows, near the part the slider names.
#[test]
fn on_the_system_head_face_dots_sit_where_the_face_changes() {
    let lib = library();
    let rig = &lib.rig;
    let head = head_part(&lib).expect("system head");
    for male in [false, true] {
        let started = std::time::Instant::now();
        let layout = derive(&lib, male, Part::Head, true);
        eprintln!("derived in {:?}", started.elapsed());
        assert!(layout.system_head);
        let sex = BTreeMap::from([(PARAM_MALE, f32::from(u8::from(male)))]);
        let weights = lib.shape_params.weights_with(&[], &sex);
        let joints = rest_joints(rig, &lib.shape_params.shape_weights(weights.clone(), rig));
        let at = |anchor: Anchor| match anchor.vertex {
            Some(v) => surface(rig, head, &weights, &joints, v).expect("vertex"),
            None => anchor.at(&joints).expect("point"),
        };
        let bone = |name: &str| joints[rig.names[name]].w_axis.truncate();
        let dot = |id: i32| at(handle_of(&layout, id).anchor);
        for handle in &layout.handles {
            let labels: Vec<_> = handle
                .params
                .iter()
                .filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == *id))
                .map(|p| (p.id, p.labels().0))
                .collect();
            eprintln!(
                "{} {:?} {} at {:.3} twin {}: {labels:?}",
                if male { "male" } else { "female" },
                handle.anchor.vertex,
                lib.rig.canonical_names[handle.anchor.joint],
                at(handle.anchor),
                handle.mirror.is_some()
            );
            assert!((1..=2).contains(&handle.params.len()));
        }
        let ids: HashSet<i32> = layout.handles.iter().flat_map(|h| h.params.clone()).collect();
        // the morph-only sliders get a dot too: their change shows here
        // (not Eyelash Length, 518, whose morph is on the eyelashes' mesh)
        let sex_bit = if male { 2 } else { 1 };
        for id in [5, 12, 13, 22, 664] {
            if catalog::PARAMS.iter().any(|p| p.id == id && p.sex & sex_bit != 0) {
                assert!(ids.contains(&id), "mesh slider {id} has a dot on the system head");
            }
        }
        assert!(ids.len() >= 40, "{} face sliders placed", ids.len());
        // near what they name
        let near = |id: i32, part: &str, within: f32| {
            let d = dot(id).distance(bone(part));
            assert!(d < within, "slider {id}: {d:.3} m from {part}");
        };
        near(35, "mFaceEar1Left", 0.04);
        near(517, "mFaceNoseBase", 0.03);
        near(196, "mEyeLeft", 0.04);
        assert!(handle_of(&layout, 35).mirror.is_some(), "both ears");
        // Egg Head reshapes the crown, above the eyes
        assert!(dot(646).z > bone("mEyeLeft").z, "Egg Head at {:.3}", dot(646));
        // two dots on the mesh never crowd each other
        let spots: Vec<Vec3> = layout
            .handles
            .iter()
            .filter(|h| h.anchor.vertex.is_some())
            .map(|h| at(h.anchor))
            .collect();
        for (i, a) in spots.iter().enumerate() {
            for b in &spots[i + 1..] {
                assert!(a.distance(*b) >= MESH_GAP * 0.99, "{a:.3} and {b:.3}");
            }
        }
    }
}

/// A drag on a system head dot follows the morph: the dot moves with the
/// slider, and dragging it moves the slider.
#[test]
fn a_system_head_dot_drags_its_morph() {
    let lib = library();
    let layout = derive(&lib, false, Part::Head, true);
    let body = rest_body(&lib);
    let sliders = defaults(false);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    let mut checked = 0;
    for (index, handle) in layout.handles.iter().enumerate() {
        if handle.anchor.vertex.is_none() || handle.params.len() != 1 {
            continue;
        }
        let id = handle.params[0];
        let Some(param) = catalog::PARAMS.iter().find(|p| p.id == id) else {
            continue;
        };
        let mut hi = sliders.clone();
        hi.insert(id, param.max);
        let (start, end) = (
            body.point(&sliders, handle.anchor).unwrap(),
            body.point(&hi, handle.anchor).unwrap(),
        );
        // seen from where the slider moves the dot most
        let way = end - start;
        let view: fn(Vec3) -> Option<Vec2> = if way.x.abs() > way.y.abs() { side } else { front };
        let pointer = view(start).unwrap();
        let goal = view(end).unwrap();
        // (a dot that hardly moves scrubs instead: `a_dot_too_small_to_follow_scrubs`)
        if pointer.distance(goal) < SMALL_ON_SCREEN {
            continue;
        }
        let drag = Drag::start(&body, &sliders, grip(index, handle), start, looking(view), view, pointer).expect("drag");
        let mut trial = sliders.clone();
        for i in 1..=20 {
            drag.step(&body, &mut trial, ranges, view, pointer + (goal - pointer) * (i as f32 / 20.0));
        }
        let wanted = param.max - sliders[&id];
        assert!(
            trial[&id] - sliders[&id] > wanted * 0.5,
            "slider {id} went {} → {} of {}",
            sliders[&id],
            trial[&id],
            param.max
        );
        checked += 1;
    }
    assert!(checked >= 10, "{checked} face dots dragged");
}

/// A drag never pushes the dot away from the pointer it follows: a slider
/// that bends in its middle (Leg Muscles, Belly Size…) settled by
/// overshooting its bend back and forth, a jitter.
#[test]
fn a_drag_settles_without_going_back_and_forth() {
    let lib = library();
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    for part in [Part::Body, Part::Head] {
        let layout = derive(&lib, false, part, part == Part::Head);
        // a slide's point is what follows the pointer (a resize drives its
        // outline instead)
        for (index, handle) in layout.handles.iter().enumerate().filter(|(_, h)| !h.resize) {
            let sliders = defaults(false);
            let start = body.point(&sliders, handle.anchor).unwrap();
            for view in [front as fn(Vec3) -> Option<Vec2>, side] {
                let pointer = view(start).unwrap();
                let Some(drag) = Drag::start(&body, &sliders, grip(index, handle), start, looking(view), view, pointer) else {
                    continue;
                };
                // (a scrub follows no point)
                if drag.scrubs() {
                    continue;
                }
                for push in [Vec2::new(25.0, 10.0), Vec2::new(-15.0, -30.0)] {
                    let target = pointer + push;
                    let mut trial = sliders.clone();
                    let mut last = pointer.distance(target);
                    for _ in 0..12 {
                        drag.step(&body, &mut trial, ranges, view, target);
                        let now = body.point(&trial, handle.anchor).and_then(view).unwrap().distance(target);
                        assert!(now <= last + 1e-3, "slider {:?}: {last} then {now}", handle.params);
                        last = now;
                    }
                }
            }
        }
    }
}

/// The body's dots fall into areas by where they sit on the skeleton; the
/// head's by the shape editor's face tabs; a dot never mixes two areas.
#[test]
fn every_dot_belongs_to_one_area_of_the_avatar() {
    let lib = library();
    for male in [false, true] {
        let body = derive(&lib, male, Part::Body, false);
        let in_area = |id: i32| handle_of(&body, id).area;
        assert_eq!(in_area(33), AREA_HEAD_NECK, "Height, on the head");
        assert_eq!(in_area(683), AREA_HEAD_NECK, "Neck Thickness");
        assert_eq!(in_area(693), AREA_ARMS, "Arm Length");
        assert_eq!(in_area(675), AREA_ARMS, "Hand Size");
        assert_eq!(in_area(157), AREA_TORSO, "Belly Size");
        assert_eq!(in_area(37), AREA_LEGS, "Hip Width");
        assert_eq!(in_area(795), AREA_LEGS, "Butt Size");
        assert_eq!(in_area(841), AREA_LEGS, "Knee Angle");
        assert_eq!(in_area(515), AREA_LEGS, "Foot Size");
        if !male {
            assert_eq!(in_area(105), AREA_TORSO, "Breast Size");
        }
        let areas: HashSet<usize> = body.handles.iter().map(|h| h.area).collect();
        assert_eq!(areas.len(), 4, "head and neck, torso, arms, legs");
        for system_head in [false, true] {
            let head = derive(&lib, male, Part::Head, system_head);
            for handle in &head.handles {
                for id in &handle.params {
                    let group = catalog::PARAMS.iter().find(|p| p.id == *id).expect("slider").group;
                    assert_eq!(handle.area, group, "slider {id} in its face tab's area");
                }
            }
        }
    }
}

#[test]
fn the_body_and_the_head_keep_their_own_sliders() {
    let lib = library();
    let body: HashSet<i32> = derive(&lib, false, Part::Body, false)
        .handles
        .iter()
        .flat_map(|h| h.params.clone())
        .collect();
    let head: HashSet<i32> = derive(&lib, false, Part::Head, false)
        .handles
        .iter()
        .flat_map(|h| h.params.clone())
        .collect();
    assert!(body.is_disjoint(&head));
}

/// A slider the shape does not list (an older shape asset) is drawn at the
/// body's own weight: a drag on it moves it exactly as if the shape listed
/// that weight. It started from the slider's minimum instead, and the body
/// jumped there at the first step.
#[test]
fn a_drag_starts_from_the_drawn_value_of_a_slider_the_shape_lacks() {
    let lib = library();
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    let layout = derive(&lib, false, Part::Body, false);
    let mut checked = 0;
    for (index, handle) in layout.handles.iter().enumerate() {
        if handle.resize || handle.params.len() != 1 {
            continue;
        }
        let id = handle.params[0];
        let Some(param) = catalog::PARAMS.iter().find(|p| p.id == id) else {
            continue;
        };
        if param.default - param.min < (param.max - param.min) * 0.3 {
            continue;
        }
        let mut lacking = defaults(false);
        lacking.remove(&id);
        let mut listed = lacking.clone();
        listed.insert(id, body.base[&id]);
        let start = body.point(&lacking, handle.anchor).unwrap();
        for view in [front as fn(Vec3) -> Option<Vec2>, side] {
            let pointer = view(start).unwrap();
            let Some(drag) = Drag::start(&body, &lacking, grip(index, handle), start, looking(view), view, pointer) else {
                continue;
            };
            let (mut from_lacking, mut from_listed) = (lacking.clone(), listed.clone());
            let target = pointer + Vec2::new(2.0, 2.0);
            drag.step(&body, &mut from_lacking, ranges, view, target);
            drag.step(&body, &mut from_listed, ranges, view, target);
            let (a, b) = (from_lacking.get(&id).copied().unwrap_or(body.base[&id]), from_listed[&id]);
            assert!((a - b).abs() < 1e-4, "slider {id}: {a} lacking, {b} listed (min {})", param.min);
            checked += 1;
        }
    }
    assert!(checked >= 3, "{checked} drags checked");
}

/// Knee Angle moves the legs sideways: seen from the side its dot cannot
/// follow the pointer, and the drag scrubs the slider like a number instead
/// of doing nothing; from the front it follows.
#[test]
fn a_dot_too_small_to_follow_scrubs() {
    let lib = library();
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    let layout = derive(&lib, false, Part::Body, false);
    let (index, handle) = layout
        .handles
        .iter()
        .enumerate()
        .find(|(_, h)| h.params.contains(&841))
        .expect("Knee Angle has a handle");
    let sliders = defaults(false);
    let start = body.point(&sliders, handle.anchor).unwrap();
    let from_side = Drag::start(
        &body,
        &sliders,
        grip(index, handle),
        start,
        looking(side),
        side,
        side(start).unwrap(),
    )
    .unwrap();
    assert!(from_side.scrubs());
    // it grows the way the slider moves the dot, however little: the left
    // dot of a pair to the left, the right one to the right
    let (min, max) = ranges(841).unwrap();
    let at = |value: f32| {
        let mut values = sliders.clone();
        values.insert(841, value);
        side(body.point(&values, handle.anchor).unwrap()).unwrap()
    };
    let motion = at(max) - at(min);
    // seen straight along its motion: out from its part's centre on screen,
    // the way the slider takes the point from that centre
    let point = |value: f32| {
        let mut values = sliders.clone();
        values.insert(841, value);
        body.point(&values, handle.anchor).unwrap()
    };
    let centre = body.joint(&sliders, handle.anchor.joint).unwrap().w_axis.truncate();
    let out = side(start).unwrap() - side(centre).unwrap();
    let way = if motion.length() > SCRUB_FOLLOWS {
        motion.normalize()
    } else if out.length() > SMALL_ON_SCREEN * 0.5 {
        out.normalize() * (point(max) - point(min)).dot(start - centre).signum()
    } else {
        Vec2::new(1.0, -1.0).normalize()
    };
    let mut trial = sliders.clone();
    assert!(from_side.step(&body, &mut trial, ranges, side, side(start).unwrap() + way * 60.0));
    let wanted = sliders[&841] + 60.0 * (max - min) / SCRUB_RANGE;
    assert!((trial[&841] - wanted).abs() < 0.01, "{} for {wanted}", trial[&841]);
    // and back the other way
    let mut trial = sliders.clone();
    from_side.step(&body, &mut trial, ranges, side, side(start).unwrap() - way * 60.0);
    assert!(trial[&841] < sliders[&841], "{}", trial[&841]);
    let from_front = Drag::start(
        &body,
        &sliders,
        grip(index, handle),
        start,
        looking(front),
        front,
        front(start).unwrap(),
    )
    .unwrap();
    assert!(!from_front.scrubs());
    // a face bone's resize under a mesh head has no outline: it scrubs
    let head = derive(&lib, false, Part::Head, false);
    let (index, handle) = head
        .handles
        .iter()
        .enumerate()
        .find(|(_, h)| h.params.contains(&18))
        .expect("Upper Cheeks");
    assert!(handle.resize);
    let start = body.point(&sliders, handle.anchor).unwrap();
    let drag = Drag::start(
        &body,
        &sliders,
        grip(index, handle),
        start,
        looking(front),
        front,
        front(start).unwrap(),
    )
    .unwrap();
    assert!(drag.scrubs());
}

/// Under a mesh head the face dots sit where the face's surface is, out
/// from the bones inside it: the lips' and the cheeks' clear of their bone
/// (spread apart across the face, [`spread_face`], some end nearer it than
/// the surface's centimetre), every dot still on its own bone.
#[test]
fn mesh_head_dots_sit_on_the_face_surface() {
    let lib = library();
    let layout = derive(&lib, false, Part::Head, false);
    for id in [659, 18, 653] {
        let handle = handle_of(&layout, id);
        assert!(handle.anchor.vertex.is_none());
        assert!(
            handle.anchor.local.length() > 0.004,
            "slider {id}: {} m off {}",
            handle.anchor.local.length(),
            lib.rig.canonical_names[handle.anchor.joint]
        );
        // its twin, mirrored the same way
        if let Some(twin) = handle.mirror {
            assert!((twin.local.length() - handle.anchor.local.length()).abs() < 0.004, "slider {id}");
        }
    }
}

/// Both dots of a pair grow their slider pulled outward, from the front and
/// from the back: Saddle Bags' left dot pulled to the left, its right one
/// to the right, whether the drag follows the dot or scrubs.
#[test]
fn both_dots_of_a_pair_grow_pulled_outward() {
    let lib = library();
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    let layout = derive(&lib, false, Part::Body, false);
    let (index, handle) = layout
        .handles
        .iter()
        .enumerate()
        .find(|(_, h)| h.params.contains(&753) && h.mirror.is_some())
        .expect("Saddle Bags has a pair of dots");
    let sliders = as_worn(&lib, defaults(false));
    let back = |p: Vec3| Some(Vec2::new(p.y * 500.0, -p.z * 500.0));
    for view in [front as fn(Vec3) -> Option<Vec2>, back] {
        for mirrored in [false, true] {
            let anchor = handle.anchor(mirrored);
            let grip = Grip {
                mirrored,
                anchor,
                ..grip(index, handle)
            };
            let start = body.point(&sliders, anchor).unwrap();
            let at = view(start).unwrap();
            // outward: away from the body's middle line on screen
            let out = Vec2::new(at.x.signum(), 0.0);
            let drag = Drag::start(&body, &sliders, grip, start, looking(view), view, at).unwrap();
            let mut trial = sliders.clone();
            for step in 1..=8 {
                drag.step(&body, &mut trial, ranges, view, at + out * (5.0 * step as f32));
            }
            assert!(
                trial[&753] > sliders[&753] + 0.01,
                "mirrored {mirrored}: {} from {}",
                trial[&753],
                sliders[&753]
            );
        }
    }
}

/// Under a mesh head, the dots of one area are apart across the face: the
/// upper lip's sliders (Lip Thickness, Lip Cleft Depth, Lip Ratio, Lip
/// Fullness, Mouth Position) lifted to one spot, and a dot on the midline
/// had its twin on top of it.
#[test]
fn mesh_head_dots_of_an_area_are_apart() {
    let lib = library();
    let joints = base_rest_joints(&lib, false);
    let centre = joints[lib.rig.names["HEAD"]].w_axis.truncate();
    let layout = derive(&lib, false, Part::Head, false);
    for area in 1..=6 {
        let dots: Vec<(Vec<i32>, Vec3)> = layout
            .handles
            .iter()
            .filter(|h| h.area == area)
            .flat_map(|h| {
                std::iter::once(h.anchor)
                    .chain(h.mirror)
                    .map(|a| (h.params.clone(), a.at(&joints).unwrap()))
            })
            .collect();
        for (i, (a, p)) in dots.iter().enumerate() {
            for (b, q) in &dots[i + 1..] {
                let out = ((*p + *q) * 0.5 - centre).normalize_or_zero();
                let apart = (*p - *q).reject_from_normalized(out).length();
                assert!(apart > FACE_GAP * 0.9, "area {area}: {a:?} and {b:?} {apart} m apart");
            }
        }
    }
}

/// A dot seen straight along its motion (Love Handles from the front) scrubs
/// outward: pulled away from its part's centre on screen it grows, pushed
/// in it shrinks, on either side. Up and to the right shrank the dot on the
/// left of the screen.
#[test]
fn a_dot_seen_along_its_motion_grows_pulled_outward() {
    let lib = library();
    let body = rest_body(&lib);
    let ranges = |id: i32| catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max));
    let layout = derive(&lib, false, Part::Body, false);
    let (index, handle) = layout
        .handles
        .iter()
        .enumerate()
        .find(|(_, h)| h.params.contains(&676) && h.mirror.is_some())
        .expect("Love Handles has a pair of dots");
    let sliders = as_worn(&lib, defaults(false));
    let (min, max) = ranges(676).unwrap();
    for mirrored in [false, true] {
        let anchor = handle.anchor(mirrored);
        let point = |value: f32| {
            let mut values = sliders.clone();
            values.insert(676, value);
            body.point(&values, anchor).unwrap()
        };
        // looking along the dot's own motion
        let along = (point(max) - point(min)).normalize();
        let right = along.cross(Vec3::Z).normalize();
        let up = right.cross(along);
        let view = move |p: Vec3| Some(Vec2::new(p.dot(right) * 500.0, -p.dot(up) * 500.0));
        let start = point(sliders[&676]);
        let at = view(start).unwrap();
        let centre = view(body.joint(&sliders, anchor.joint).unwrap().w_axis.truncate()).unwrap();
        let out = (at - centre).normalize();
        for (way, grows) in [(out, true), (-out, false)] {
            let grip = Grip {
                mirrored,
                anchor,
                ..grip(index, handle)
            };
            let drag = Drag::start(&body, &sliders, grip, start, along, view, at).unwrap();
            assert!(drag.scrubs(), "mirrored {mirrored}: seen along its motion");
            let mut trial = sliders.clone();
            drag.step(&body, &mut trial, ranges, view, at + way * 40.0);
            assert_eq!(
                trial[&676] > sliders[&676],
                grows,
                "mirrored {mirrored}, outward {grows}: {}",
                trial[&676]
            );
        }
    }
}

/// The sliders as a worn shape's draft holds them: the driven parameters
/// too (850 and 854 that Saddle Bags drives, 855 and 856 Love Handles'),
/// which a wearable lists with the rest.
fn as_worn(lib: &AvatarLibrary, sliders: BTreeMap<i32, f32>) -> BTreeMap<i32, f32> {
    let mut worn: BTreeMap<i32, f32> = lib.shape_params.weights(&[]).into_iter().collect();
    worn.extend(sliders);
    worn
}

/// The mesh head's surface read arriving mid-drag moves the dot's anchor:
/// the drag keeps its dot, and the other dot of the pair stays another.
#[test]
fn a_moved_anchor_is_still_the_dragged_dot() {
    let held = Grip {
        handle: 3,
        mirrored: false,
        anchor: Anchor::on(7, Vec3::ZERO),
        params: vec![1, 2],
        resize: false,
    };
    let lifted = Grip {
        anchor: Anchor::on(7, Vec3::new(0.0, 0.0, 0.01)),
        ..held.clone()
    };
    assert!(held.same_dot(&lifted));
    assert!(!held.same_dot(&Grip {
        mirrored: true,
        ..lifted.clone()
    }));
    assert!(!held.same_dot(&Grip {
        handle: 4,
        ..lifted.clone()
    }));
    assert!(!held.same_dot(&Grip { params: vec![1], ..lifted }));
}
