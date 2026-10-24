use super::*;
use crate::{ao::State, scene::avatar::AvatarLibrary};
use std::cell::Cell;

struct Fixture {
    rig: Arc<Rig>,
    base: SkeletonBase,
    frame: Frame,
    at: Instant,
}

impl Fixture {
    fn new() -> Self {
        let library = AvatarLibrary::load();
        let rig = library.rig.clone();
        let base = SkeletonBase {
            local_pos: rig.local_pos.clone(),
            scale: rig.scale.clone(),
        };
        Self {
            rig,
            base,
            at: Instant::now(),
            frame: Frame {
                to_world: Mat4::IDENTITY,
                contact: None,
                velocity_z: 0.0,
                sole_height: None,
                max_height: 0.4,
                standing_anchor: false,
            },
        }
    }

    fn controller(&self, clips: &[(Uuid, aurora_assets::Animation)]) -> Controller {
        let mut controller = Controller {
            legacy_ground: Some(self.frame),
            ..Default::default()
        };
        let signals: Vec<_> = clips
            .iter()
            .map(|(id, _)| PlayingAnimation {
                id: *id,
                sequence: 1,
                sequence_start: self.at,
                continuous_start: self.at,
            })
            .collect();
        controller.sync(&signals, self.at, |id| {
            clips
                .iter()
                .find(|(bound, _)| bound == id)
                .map(|(_, clip)| Arc::new(BoundAnim::bind(clip.clone(), &self.rig)))
        });
        controller
    }

    fn render(&self, controller: &mut Controller, elapsed: f32, native: &dyn Fn(Vec3) -> Option<Surface>) -> Vec<Mat4> {
        let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); self.rig.len()];
        controller.evaluate_with_support(
            &self.rig,
            self.at + Duration::from_secs_f32(elapsed),
            Some(&self.base),
            &mut output,
            |_| panic!("native placement must never trace scene geometry"),
            native,
        );
        output.iter().map(Mat4::from_cols_array_2d).collect()
    }
}

fn stand() -> aurora_assets::Animation {
    let mut clip = crate::demo::grounding::animation(false);
    clip.ease_in = 0.0;
    clip.base_priority = 0;
    clip
}

fn support(point: Vec3) -> Option<Surface> {
    Some(Surface {
        position: point.with_z(0.18),
        normal: Vec3::Z,
    })
}

fn no_support(_: Vec3) -> Option<Surface> {
    panic!("an excluded or replaced native motion queried support")
}

#[test]
fn native_stands_and_crouch_correct_feet_without_baking_the_held_animation() {
    let f = Fixture::new();
    for id in crate::ao::STAND_VARIANTS
        .into_iter()
        .chain([State::Standing.motion(), State::Crouching.motion()])
    {
        let mut controller = f.controller(&[(id, stand())]);
        for elapsed in [0.1, 0.2, 0.2, 1.0] {
            let posed = f.render(&mut controller, elapsed, &support);
            for side in ["Left", "Right"] {
                let [hip, knee, ankle] = ["Hip", "Knee", "Ankle"].map(|bone| f.rig.names[&format!("m{bone}{side}")]);
                assert!((posed[ankle].w_axis.z - 0.18).abs() < 0.0005, "{side} did not reach support");
                for (a, b) in [(hip, knee), (knee, ankle)] {
                    let rest = f.rig.world(&f.rig.local_rot, Vec3::ZERO);
                    assert!((posed[a].w_axis.distance(posed[b].w_axis) - rest[a].w_axis.distance(rest[b].w_axis)).abs() < 1e-5);
                }
                assert!(
                    controller.rot[knee].abs_diff_eq(Quat::IDENTITY, 1e-5),
                    "native IK entered the held source"
                );
            }
        }
    }
}

#[test]
fn native_ankles_follow_support_normals_and_shaped_segments() {
    let mut f = Fixture::new();
    for name in ["mHipLeft", "mHipRight"] {
        f.base.scale[f.rig.names[name]] = Vec3::new(1.0, 1.1, 0.85);
    }
    let normal = Vec3::new(0.0, -0.2, 1.0).normalize();
    let mut controller = f.controller(&[(State::Standing.motion(), stand())]);
    let output = f.render(&mut controller, 1.0, &|p| {
        Some(Surface {
            position: p.with_z(0.23),
            normal,
        })
    });
    for name in ["mAnkleLeft", "mAnkleRight"] {
        let matrix = output[f.rig.names[name]];
        assert!((matrix.w_axis.z - 0.23).abs() < 0.0005);
        assert!(matrix.z_axis.truncate().normalize().dot(normal) > 0.9999);
    }
}

#[test]
fn custom_ao_replacement_and_native_walk_or_land_do_not_acquire_stand_ik() {
    let f = Fixture::new();
    for id in [
        Uuid::from_u128(123),
        State::Walking.motion(),
        State::Landing.motion(),
        State::SoftLanding.motion(),
    ] {
        let mut controller = f.controller(&[(id, stand())]);
        f.render(&mut controller, 1.0, &no_support);
    }
}

#[test]
fn higher_priority_ao_tracks_win_over_corrected_native_tracks() {
    let f = Fixture::new();
    let mut custom = stand();
    custom.base_priority = 4;
    for track in &mut custom.joints {
        for key in &mut track.rot_keys {
            key.rotation = Quat::from_rotation_y(0.08);
        }
    }
    let id = Uuid::from_u128(123);
    let mut mixed = f.controller(&[(State::Standing.motion(), stand()), (id, custom.clone())]);
    let mut reference = f.controller(&[(id, custom)]);
    let output = f.render(&mut mixed, 1.0, &support);
    let expected = f.render(&mut reference, 1.0, &no_support);
    for name in [
        "mPelvis",
        "mHipLeft",
        "mKneeLeft",
        "mAnkleLeft",
        "mHipRight",
        "mKneeRight",
        "mAnkleRight",
    ] {
        let j = f.rig.names[name];
        assert!(output[j].abs_diff_eq(expected[j], 1e-5), "AO lost {name}");
    }
}

#[test]
fn native_recovery_snapshots_support_and_preserves_leg_animation() {
    let f = Fixture::new();
    let mut clip = crate::demo::grounding::contact_animation(&f.rig, &f.base, true);
    clip.ease_in = 0.0;
    let mut controller = f.controller(&[(State::StandingUp.motion(), clip)]);
    let queries = Cell::new(0);
    let normal = Vec3::new(-0.25, 0.0, 1.0).normalize();
    let output = f.render(&mut controller, 0.2, &|p| {
        queries.set(queries.get() + 1);
        Some(Surface {
            position: p.with_z(0.0),
            normal,
        })
    });
    assert_eq!(queries.get(), 1);
    assert!(
        (controller.motions[0].weight - 0.5).abs() < 1e-5,
        "grounded recovery must ease in over 0.4 s"
    );
    let pelvis = f.rig.pelvis;
    assert!(output[pelvis].z_axis.truncate().normalize().dot(normal) > 0.9999);
    for name in ["mHipLeft", "mKneeLeft", "mHipRight", "mKneeRight"] {
        let j = f.rig.names[name];
        let parent = f.rig.parent[j].expect("leg parent");
        let local = (output[parent].inverse() * output[j]).to_scale_rotation_translation().1;
        assert!(local.dot(controller.rot[j]).abs() > 0.99999, "recovery replaced {name}");
    }
    let output = f.render(&mut controller, 1.6, &no_support);
    assert!(output[pelvis].z_axis.truncate().normalize().dot(Vec3::Z) > 0.9999);
}

#[test]
fn recovery_ease_uses_descent_speed_and_has_a_finite_grounded_fallback() {
    let f = Fixture::new();
    for (velocity, expected) in [(-4.0, 0.2), (-2.0, 0.4), (0.0, 0.4), (1.0, 0.4)] {
        let mut frame = f.frame;
        frame.velocity_z = velocity;
        let recovery = Recovery::new(frame, support(Vec3::ZERO).expect("support"), 0.8);
        assert!((recovery.ease_in - expected).abs() < 1e-6);
    }
}

#[test]
fn aurora_owns_legs_even_without_support_or_during_movement_exclusions() {
    let f = Fixture::new();
    for id in [State::Standing.motion(), State::StandingUp.motion()] {
        let mut controller = f.controller(&[(id, stand())]);
        controller.aurora_feet = true;
        assert!(controller.ground.is_none());
        f.render(&mut controller, 0.2, &no_support);
        assert!(controller.motions[0].recovery.is_none(), "replaced recovery still ran");
    }
}

#[test]
fn editing_preview_control_avatars_and_airborne_states_exclude_native_placement() {
    let f = Fixture::new();
    for mode in 0..4 {
        let mut controller = f.controller(&[(State::Standing.motion(), stand())]);
        match mode {
            0 => controller.ik.edit.active = true,
            1 => controller.set_preview_clock(Some((None, f.at))),
            2 => controller.control = true,
            _ => {
                let clip = Arc::new(BoundAnim::bind(stand(), &f.rig));
                controller.sync(
                    &[PlayingAnimation {
                        id: State::Flying.motion(),
                        sequence: 2,
                        sequence_start: f.at,
                        continuous_start: f.at,
                    }],
                    f.at,
                    |_| Some(clip.clone()),
                );
            }
        }
        f.render(&mut controller, 1.0, &no_support);
    }
}

#[test]
fn paused_native_pose_keeps_its_correction_without_querying_or_baking_it() {
    let f = Fixture::new();
    let mut controller = f.controller(&[(State::Standing.motion(), stand())]);
    let before = f.render(&mut controller, 0.2, &support);
    controller.set_paused(true, f.at + Duration::from_secs_f32(0.2));
    controller.sync(&[], f.at + Duration::from_secs(1), |_| None);
    let paused = f.render(&mut controller, 2.0, &no_support);
    for name in ["mPelvis", "mHipLeft", "mKneeLeft", "mAnkleLeft"] {
        let j = f.rig.names[name];
        assert!(before[j].abs_diff_eq(paused[j], 1e-5));
    }
    controller.ik.edit.active = true;
    let edited = f.render(&mut controller, 2.0, &no_support);
    assert!(edited[f.rig.names["mAnkleLeft"]].w_axis.z < 0.15);
}

#[test]
fn toggles_interpolate_independent_corrections_and_allow_immediate_reversal() {
    let f = Fixture::new();
    let ankle = f.rig.names["mAnkleLeft"];
    let mut handoff = Handoff::default();
    let source = f.rig.local_rot.clone();
    let mut native = source.clone();
    native[ankle] = Quat::from_rotation_y(0.4);
    let mut aurora = source.clone();
    aurora[ankle] = Quat::from_rotation_y(-0.6);
    assert!(
        handoff
            .apply(&f.rig, &source, &native, Vec3::ZERO, false, [true; 2], f.at)
            .is_none()
    );
    let at = f.at + Duration::from_secs(1);
    let begin = handoff
        .apply(&f.rig, &source, &aurora, Vec3::Z * 0.2, true, [true; 2], at)
        .expect("handoff");
    assert!(begin.rotations[ankle].abs_diff_eq(native[ankle], 1e-6));
    assert_eq!(begin.pelvis, Vec3::ZERO);
    let halfway = handoff
        .apply(
            &f.rig,
            &source,
            &aurora,
            Vec3::Z * 0.2,
            true,
            [true; 2],
            at + Duration::from_millis(250),
        )
        .expect("handoff");
    assert!(halfway.rotations[ankle].abs_diff_eq(Quat::from_rotation_y(-0.1), 1e-5));
    assert!((halfway.pelvis.z - 0.1).abs() < 1e-6);
    let reverse = handoff
        .apply(
            &f.rig,
            &source,
            &native,
            Vec3::ZERO,
            false,
            [true; 2],
            at + Duration::from_millis(250),
        )
        .expect("reverse");
    assert!(reverse.rotations[ankle].abs_diff_eq(halfway.rotations[ankle], 1e-5));
    assert!(reverse.pelvis.abs_diff_eq(halfway.pelvis, 1e-6));
    assert!(
        handoff
            .apply(
                &f.rig,
                &source,
                &native,
                Vec3::ZERO,
                false,
                [true; 2],
                at + Duration::from_millis(750)
            )
            .is_none()
    );
}

#[test]
fn native_to_aurora_and_back_never_feed_one_solver_into_the_other() {
    let f = Fixture::new();
    let mut controller = f.controller(&[(State::Standing.motion(), stand())]);
    let native = f.render(&mut controller, 0.1, &support);
    controller.aurora_feet = true;
    controller.ground = Some(f.frame);
    let mut output = vec![Mat4::IDENTITY.to_cols_array_2d(); f.rig.len()];
    let mut previous = native;
    for (step, aurora) in [(0, true), (30, true), (31, false), (61, false)] {
        controller.aurora_feet = aurora;
        controller.evaluate_with_support(
            &f.rig,
            f.at + Duration::from_millis(200 + step * 20),
            Some(&f.base),
            &mut output,
            support,
            |p| {
                assert!(!aurora, "Aurora fell back to native support");
                support(p)
            },
        );
        let posed: Vec<_> = output.iter().map(Mat4::from_cols_array_2d).collect();
        if step == 0 || step == 31 {
            for name in ["mPelvis", "mAnkleLeft", "mAnkleRight"] {
                let j = f.rig.names[name];
                assert!(posed[j].abs_diff_eq(previous[j], 1e-4), "toggle jumped {name}");
            }
        }
        previous = posed;
    }
    let reference = f.render(&mut f.controller(&[(State::Standing.motion(), stand())]), 1.0, &support);
    for name in ["mPelvis", "mAnkleLeft", "mAnkleRight"] {
        let j = f.rig.names[name];
        assert!(previous[j].abs_diff_eq(reference[j], 1e-4), "return did not restore native {name}");
    }
}

#[test]
fn explicit_leg_contact_wins_over_native_placement_and_releases_to_it() {
    use crate::scene::ik::{Goal, Mode};
    let f = Fixture::new();
    let mut controller = f.controller(&[(State::Standing.motion(), stand())]);
    let native = f.render(&mut controller, 0.1, &support);
    let ankle = f.rig.names["mAnkleLeft"];
    let target = native[ankle].w_axis.truncate() + Vec3::new(0.06, 0.0, 0.08);
    controller.ik.targets.push(Goal {
        bone: ankle,
        root: f.rig.names["mHipLeft"],
        position: target,
        rotation: Some(Quat::IDENTITY),
        contact: Vec3::ZERO,
        pole: Some(Vec3::X),
        mode: Mode::Ik,
        weight: 1.0,
        follow: None,
    });
    for step in 1..=12 {
        f.render(&mut controller, 0.1 + step as f32 * 0.04, &support);
    }
    let contacted = f.render(&mut controller, 0.6, &support);
    assert!(contacted[ankle].w_axis.truncate().distance(target) < 0.001);
    controller.ik.targets.clear();
    for step in 1..=12 {
        f.render(&mut controller, 0.6 + step as f32 * 0.04, &support);
    }
    let released = f.render(&mut controller, 1.2, &support);
    assert!(released[ankle].abs_diff_eq(native[ankle], 1e-4));
}
