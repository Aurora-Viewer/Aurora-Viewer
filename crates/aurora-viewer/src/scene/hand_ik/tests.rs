use super::*;

#[test]
fn hand_ik_two_segment_solution_reaches_both_sides_and_matches_orientation() {
    for sign in [-1.0, 1.0] {
        let target = Target {
            position: Vec3::new(0.3, sign * 0.2, -0.1),
            rotation: Some(Quat::from_rotation_x(0.6)),
            contact: Vec3::ZERO,
        };
        let bones = [Vec3::ZERO, Vec3::Y * sign * 0.3, Vec3::Y * sign * 0.55];
        let [a, b, c] = solve(bones, [Quat::IDENTITY; 3], Quat::IDENTITY, target, Vec3::new(0.0, sign, -0.5)).expect("solve");
        let elbow = a * (bones[1] - bones[0]);
        let wrist = elbow + a * b * (bones[2] - bones[1]);
        assert!(wrist.distance(target.position) < 1e-5);
        assert!((a * b * c).abs_diff_eq(target.rotation.expect("orientation"), 1e-5));
        assert!((elbow.length() - 0.3).abs() < 1e-5);
        assert!((wrist.distance(elbow) - 0.25).abs() < 1e-5);
    }
}

#[test]
fn hand_ik_position_only_preserves_animated_wrist_articulation_for_both_arms() {
    for sign in [-1.0, 1.0] {
        let parent = Quat::from_rotation_x(0.4);
        let shoulder = parent * Quat::from_rotation_z(-sign * 1.2);
        let elbow = shoulder * Quat::from_rotation_x(0.3) * Quat::from_rotation_z(sign * 0.5);
        let animated_wrist = Quat::from_rotation_y(-0.2) * Quat::from_rotation_x(0.15);
        let points = [Vec3::ZERO, shoulder * Vec3::Y * sign * 0.3, Vec3::ZERO];
        let points = [points[0], points[1], points[1] + elbow * Vec3::Y * sign * 0.25];
        for position in [Vec3::new(0.35, sign * 0.15, -0.1), Vec3::new(0.2, sign * 0.25, 0.2)] {
            let [a, b, c] = solve(
                points,
                [shoulder, elbow, elbow * animated_wrist],
                parent,
                Target {
                    position,
                    rotation: None,
                    contact: Vec3::ZERO,
                },
                Vec3::new(0.0, sign, -0.5),
            )
            .expect("position-only solve");
            let solved_shoulder = parent * a;
            let solved_elbow = solved_shoulder * b;
            let wrist = solved_shoulder * (shoulder.inverse() * (points[1] - points[0]))
                + solved_elbow * (elbow.inverse() * (points[2] - points[1]));
            assert!(wrist.distance(position) < 1e-5);
            assert!(c.dot(animated_wrist).abs() > 0.99999, "no extra wrist bend or roll");
        }
    }
}

#[test]
fn hand_ik_unreachable_folded_and_singular_goals_remain_finite_without_stretch() {
    for position in [
        Vec3::ZERO,
        Vec3::Y * 0.55,
        Vec3::Y * 100.0,
        -Vec3::Y * 100.0,
        Vec3::new(0.0, 1.0, -0.5),
    ] {
        let points = [Vec3::ZERO, Vec3::Y * 0.3, Vec3::Y * 0.55];
        let goal = Target {
            position,
            rotation: Some(Quat::IDENTITY),
            contact: Vec3::ZERO,
        };
        let [a, b, c] = solve(points, [Quat::IDENTITY; 3], Quat::IDENTITY, goal, Vec3::new(0.0, 1.0, -0.5)).expect("finite solve");
        let elbow = a * Vec3::Y * 0.3;
        let wrist = elbow + a * b * Vec3::Y * 0.25;
        assert!(a.is_finite() && b.is_finite() && c.is_finite() && wrist.is_finite());
        assert!(wrist.length() <= 0.55001);
        assert!((wrist.distance(elbow) - 0.25).abs() < 1e-5);
    }
    assert!(
        solve(
            [Vec3::ZERO; 3],
            [Quat::IDENTITY; 3],
            Quat::IDENTITY,
            Target {
                position: Vec3::X,
                rotation: Some(Quat::IDENTITY),
                contact: Vec3::ZERO,
            },
            Vec3::Z
        )
        .is_none()
    );
}

#[test]
fn hand_ik_unreachable_palm_goals_do_not_stretch_the_real_forearm() {
    for sign in [-1.0, 1.0] {
        let bones = [Vec3::ZERO, Vec3::Y * sign * 0.3, Vec3::Y * sign * 0.55];
        let contact = Vec3::new(0.0, sign * 0.055, -0.015);
        for rotation in [None, Some(Quat::from_rotation_x(0.6))] {
            for position in [Vec3::ZERO, Vec3::X * 10.0, -Vec3::Y * sign * 10.0] {
                let [a, b, c] = solve(
                    bones,
                    [Quat::IDENTITY; 3],
                    Quat::IDENTITY,
                    Target {
                        position,
                        rotation,
                        contact,
                    },
                    Vec3::new(0.0, sign, -0.5),
                )
                .expect("finite palm solve");
                let elbow = a * bones[1];
                let wrist = elbow + a * b * (bones[2] - bones[1]);
                let palm = wrist + a * b * c * contact;
                assert!(palm.is_finite());
                assert!((elbow.length() - 0.3).abs() < 1e-5);
                assert!((wrist.distance(elbow) - 0.25).abs() < 1e-5);
                assert!(palm.length() <= 0.55 + contact.length() + 1e-5);
            }
        }
    }
    let bad = solve(
        [Vec3::ZERO, Vec3::Y * 0.3, Vec3::Y * 0.55],
        [Quat::IDENTITY, Quat::IDENTITY, Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)],
        Quat::IDENTITY,
        Target {
            position: Vec3::X * 0.3,
            rotation: None,
            contact: Vec3::Y * 0.055,
        },
        Vec3::Z,
    );
    assert!(bad.is_none(), "an invalid source wrist must not enter the virtual-segment clamp");
}
