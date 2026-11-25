use super::layout::{self, Bounds};
use super::{Group, Input, Place, Standing};
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
    let frame = |joints: &[Mat4]| model.frame(None, &model.bones(joints));
    let start = frame(&rest);
    let moved = |by: Vec3, sway: f32| -> Vec<Mat4> {
        rest.iter()
            .enumerate()
            .map(|(joint, m)| Mat4::from_translation(by + Vec3::Y * (sway * (joint as f32).sin())) * *m)
            .collect()
    };
    for sway in [0.05, 0.15] {
        let followed = start.follow(frame(&moved(Vec3::ZERO, sway)));
        assert_eq!(followed.center, start.center, "swaying by {sway}");
    }
    let followed = start.follow(frame(&moved(Vec3::Y * 5.0, 0.0)));
    let away = followed.center.y - start.center.y;
    assert!(away > 3.0 && away < 5.0, "a body elsewhere is followed: {away}");
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
