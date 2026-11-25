use super::layout::{self, Bounds};
use super::{Group, Input, Place, Standing, Steady};
use glam::{Mat4, Vec3};

fn cube(center: Vec3, size: f32) -> Bounds {
    let h = Vec3::splat(size / 2.0);
    Bounds {
        lo: center - h,
        hi: center + h,
    }
}

fn moved(b: &Bounds, by: Vec3) -> Bounds {
    Bounds {
        lo: b.lo + by,
        hi: b.hi + by,
    }
}

fn apart(boxes: &[Bounds], moves: &[Vec3]) {
    let moved: Vec<Bounds> = boxes.iter().zip(moves).map(|(b, m)| moved(b, *m)).collect();
    for (i, a) in moved.iter().enumerate() {
        for b in &moved[i + 1..] {
            let over = (a.hi.min(b.hi) - a.lo.max(b.lo)).min_element();
            assert!(over <= 1e-4, "{a:?} over {b:?}");
        }
    }
}

#[test]
fn objects_of_the_world_stand_side_by_side_on_the_same_ground() {
    let boxes = [
        cube(Vec3::new(3.0, 8.0, 20.0), 2.0),
        cube(Vec3::ZERO, 0.5),
        cube(Vec3::new(-4.0, 1.0, 1.0), 4.0),
    ];
    let moves = layout::side_by_side(&boxes);
    apart(&boxes, &moves);
    for (b, m) in boxes.iter().zip(&moves) {
        assert!(moved(b, *m).lo.z.abs() < 1e-4);
    }
    assert!(layout::side_by_side(&[]).is_empty());
}

#[test]
fn a_ray_finds_a_box_in_front_of_it_only() {
    let b = cube(Vec3::new(5.0, 0.0, 0.0), 2.0);
    let hit = b.hit(Vec3::ZERO, Vec3::X).expect("in front");
    assert!((hit - 4.0).abs() < 1e-4);
    assert!(b.hit(Vec3::ZERO, -Vec3::X).is_none());
    assert!(b.hit(Vec3::ZERO, Vec3::Y).is_none());
    assert_eq!(b.hit(Vec3::new(5.0, 0.0, 0.0), Vec3::X), Some(0.0), "from inside");
    assert!(Bounds::default().empty());
}

/// What an avatar of the demo wears, as a model.
fn worn_model() -> (crate::world::World, super::StageModel) {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let wearer = world.objects.index_of_uuid(&crate::demo::DEMO_LOUP).expect("the wolf");
    let key = world.objects.get(wearer).expect("avatar").key;
    let groups: Vec<Group> = world
        .objects
        .children_of(&key)
        .iter()
        .filter_map(|i| Some((*i, world.objects.get(*i)?)))
        .filter(|(_, o)| !o.is_avatar())
        .map(|(i, o)| Group {
            root: o.full_id,
            prims: crate::build::family(&world, i),
        })
        .collect();
    let scene = crate::scene::Scene::new(
        std::env::temp_dir().join(format!("aurora-stage-{}", uuid::Uuid::new_v4())),
        world.avatar_lib.clone(),
    );
    let input = Input::capture(&world, &scene, &groups, Place::Worn(wearer));
    let model = input.prepare(&world.avatar_lib, None).expect("something worn");
    (world, model)
}

/// Limbs, tails and small sways never move the picture; a body that leaves its
/// place does.
#[test]
fn a_moving_avatar_does_not_glide_in_the_view() {
    let (world, model) = worn_model();
    let rest = super::build::rest(&world.avatar_lib, &[], None);
    let pelvis = world.avatar_lib.rig.pelvis;
    let moved = |by: Vec3, sway: f32| -> Vec<Mat4> {
        rest.iter()
            .enumerate()
            .map(|(joint, m)| {
                // all but the body sways, up to several metres
                let sway = if joint == pelvis { 0.0 } else { sway };
                Mat4::from_translation(by + Vec3::Y * sway) * *m
            })
            .collect()
    };
    let bounds = |joints: &[Mat4]| model.bounds(&model.bones(joints));
    let start = Steady::of(&bounds(&rest), model.body(&rest));
    for sway in [0.05, 0.4, 2.0, -2.0] {
        let joints = moved(Vec3::ZERO, sway);
        let mut steady = start;
        steady.follow(&bounds(&joints), model.body(&joints), 1.0);
        assert_eq!(steady.frame.center, start.frame.center, "swaying by {sway}");
        assert_eq!(steady.frame.radius, start.frame.radius, "swaying by {sway}");
    }
    let joints = moved(Vec3::Y * 5.0, 0.0);
    let mut steady = start;
    steady.follow(&bounds(&joints), model.body(&joints), 1.0);
    let away = steady.frame.center.y - start.frame.center.y;
    assert!(away > 3.0 && away < 5.0, "a body elsewhere is followed: {away}");
}

/// A worn animesh follows the joints of its own skeleton, from the place of its
/// attachment; on the ground too.
#[test]
fn an_animesh_stands_on_its_own_skeleton() {
    for (local, worn) in [(9203, true), (9200, false)] {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::animesh_events()) {
            world.apply(event);
        }
        let root = world.objects.iter().find(|(_, o)| o.key.local_id == local).expect("animesh").0;
        let group = Group {
            root: world.objects.get(root).expect("root").full_id,
            prims: crate::build::family(&world, root),
        };
        let mut scene = crate::scene::Scene::new(
            std::env::temp_dir().join(format!("aurora-stage-{}", uuid::Uuid::new_v4())),
            world.avatar_lib.clone(),
        );
        // a head, rigged to mHead
        let mesh = crate::shape_editor::demo::asset(&world.avatar_lib, true).expect("head");
        scene
            .meshes
            .insert_local(crate::demo::ANIMESH_MESH, mesh.data, mesh.header, mesh.skin);
        let wearer = worn
            .then(|| world.objects.parent_of(world.objects.get(root).expect("root")))
            .flatten();
        assert_eq!(wearer.is_some(), worn);
        let place = wearer.map_or(Place::World, Place::Worn);
        let input = Input::capture(&world, &scene, std::slice::from_ref(&group), place);
        let model = input.prepare(&world.avatar_lib, None).expect("the head");
        let own: Vec<(uuid::Uuid, Vec<Mat4>)> = model.own().map(|(id, still)| (*id, still.to_vec())).collect();
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].0, group.root);
        let rest = super::build::rest(&world.avatar_lib, &[], None);
        let middle = |skeleton: &[Mat4]| model.bounds(&model.bones(skeleton)).center();
        let mut joints = rest.clone();
        joints.extend(&own[0].1);
        let start = middle(&joints);
        assert!((middle(&rest) - start).length() < 1e-4, "at rest without its joints");
        let by = Mat4::from_translation(Vec3::Z * 0.5);
        let head = world.avatar_lib.rig.names["mHead"];
        // the wearer's head: not its own
        let mut other = joints.clone();
        other[head] = by * other[head];
        assert!((middle(&other) - start).length() < 1e-4, "not on the wearer's skeleton");
        other = joints.clone();
        other[rest.len() + head] = by * other[rest.len() + head];
        let moved = middle(&other) - start;
        assert!((moved.length() - 0.5).abs() < 1e-3, "on its own: {moved}");
        if worn {
            // the hand that holds it
            other = joints
                .iter()
                .enumerate()
                .map(|(j, m)| if j < rest.len() { by * *m } else { *m })
                .collect();
            assert!((middle(&other) - start - Vec3::Z * 0.5).length() < 1e-3, "held by the wearer");
        }
    }
}

/// The parts of the avatar's own body show unless a bake hides them.
#[test]
fn the_avatars_own_eyes_show_until_a_bake_hides_them() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let scene = crate::scene::Scene::new(
        std::env::temp_dir().join(format!("aurora-stage-{}", uuid::Uuid::new_v4())),
        world.avatar_lib.clone(),
    );
    let wearer = world.objects.index_of_uuid(&world.agent_id).expect("avatar");
    let names = |world: &crate::world::World| -> Vec<&'static str> {
        super::own::body_parts(world, &scene, wearer)
            .into_iter()
            .map(|(i, _)| world.avatar_lib.parts[i].name)
            .collect()
    };
    assert!(names(&world).contains(&"eye_left"));
    world.preview_bakes([(11, crate::scene::avatar::IMG_INVISIBLE)].into());
    assert!(!names(&world).iter().any(|n| n.starts_with("eye_")));
    for part in [0, 1] {
        let faces = super::own::body_faces(&world.avatar_lib, &[part], &[]);
        assert!(faces[0].0.iter().all(|v| {
            let sum: f32 = v.weights.iter().sum();
            (sum - 1.0).abs() < 0.01 && v.joints.iter().all(|j| *j as usize <= world.avatar_lib.rig.len())
        }));
    }
}

/// A ray finds the prim it meets whatever the bones do, in their own space.
#[test]
fn picking_follows_the_bones() {
    let (world, model) = worn_model();
    let standing = Standing::default();
    for shift in [Vec3::ZERO, Vec3::new(0.4, -0.7, 0.2)] {
        let joints: Vec<Mat4> = super::build::rest(&world.avatar_lib, &[], None)
            .into_iter()
            .map(|m| Mat4::from_translation(shift) * m)
            .collect();
        let bones = model.bones(&joints);
        let frame = model.frame(None, &bones);
        for unit in &model.units {
            // the middle of the unit, looked at from far in front
            let group = model.groups[unit.group].root;
            let at = model
                .frame(model.groups.iter().position(|g| g.root == group).map(|g| (g, 1.0)), &bones)
                .center;
            let target = (at - frame.center) / frame.radius;
            let from = target + Vec3::X * 5.0;
            let hit = model.pick(standing, &frame, &bones, from, Vec3::NEG_X);
            assert!(hit.is_some(), "{group}: something is there");
        }
        assert!(
            model
                .pick(standing, &frame, &bones, Vec3::new(5.0, 40.0, 0.0), Vec3::NEG_X)
                .is_none()
        );
    }
}
