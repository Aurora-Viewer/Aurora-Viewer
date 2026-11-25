use super::layout::{self, Bounds};
use glam::Vec3;

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

/// Arms down along the body, the two sides alike, feet on the same ground.
#[test]
fn the_stance_is_relaxed_and_symmetrical() {
    let lib = crate::scene::avatar::AvatarLibrary::load();
    let joints = super::build::stance(&lib, &[], None, true);
    let at = |name: &str| joints[lib.rig.names[name]].w_axis.truncate();
    for side in ["Left", "Right"] {
        let (shoulder, wrist) = (at(&format!("mShoulder{side}")), at(&format!("mWrist{side}")));
        assert!(shoulder.z - wrist.z > 0.35, "{side}: the hand hangs, {shoulder:?} {wrist:?}");
        let out = (wrist.y - shoulder.y).abs();
        assert!(out < 0.3 && wrist.y.abs() > 0.15, "{side}: beside the body, {wrist:?}");
        assert!(wrist.x > shoulder.x - 0.02, "{side}: a little forward");
    }
    for name in ["mWrist", "mElbow", "mKnee", "mAnkle"] {
        let (l, r) = (at(&format!("{name}Left")), at(&format!("{name}Right")));
        assert!((l - Vec3::new(r.x, -r.y, r.z)).length() < 0.01, "{name}: {l:?} {r:?}");
    }
}
