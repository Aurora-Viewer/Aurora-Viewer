use super::layout::{self, Bounds};
use glam::Vec3;

fn cube(center: Vec3, size: f32) -> Bounds {
    let h = Vec3::splat(size / 2.0);
    Bounds {
        lo: center - h,
        hi: center + h,
    }
}

fn apart(boxes: &[Bounds], moves: &[Vec3]) {
    let moved: Vec<Bounds> = boxes.iter().zip(moves).map(|(b, m)| b.moved(*m)).collect();
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
        assert!(b.moved(*m).lo.z.abs() < 1e-4);
    }
    assert!(layout::side_by_side(&[]).is_empty());
}

#[test]
fn worn_items_come_apart_around_the_largest() {
    // a body, and items worn on it, some at the same place
    let mut boxes = vec![Bounds {
        lo: Vec3::new(-0.2, -0.4, 0.0),
        hi: Vec3::new(0.2, 0.4, 1.8),
    }];
    boxes.extend((0..9).map(|i| cube(Vec3::new(0.0, 0.0, 0.2 * i as f32), 0.25)));
    let moves = layout::worn_apart(&boxes);
    assert_eq!(moves[0], Vec3::ZERO, "the body stays");
    apart(&boxes, &moves);
    assert!(layout::worn_apart(&[]).is_empty());
}

#[test]
fn prims_that_share_their_middle_fan_out() {
    let boxes = [cube(Vec3::ZERO, 1.0), cube(Vec3::ZERO, 1.0), cube(Vec3::new(0.0, 3.0, 0.0), 1.0)];
    let moves = layout::parts_apart(&boxes);
    assert!(moves.iter().all(|m| m.is_finite()));
    assert!(moves[2].y > 0.0, "away from the middle");
    assert_eq!(layout::parts_apart(&boxes[..1]), [Vec3::ZERO], "one part has nothing to leave");
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
