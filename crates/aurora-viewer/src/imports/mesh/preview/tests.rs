use super::*;
use aurora_assets::import_mesh::Skin;
use glam::Vec3;
use std::sync::Arc;
use std::time::Duration;

fn draft() -> Draft {
    let mut draft = crate::imports::tests::mesh_fixture();
    let model = &mut Arc::make_mut(&mut draft.scene).models[0];
    model.skin = Some(Skin {
        names: vec!["mPelvis".into()],
        inverse_bind: vec![Mat4::IDENTITY],
        alternate_bind: vec![Mat4::from_translation(Vec3::new(0.0, 0.0, 1.4))],
        bind_shape: Mat4::IDENTITY,
        pelvis_offset: 0.0,
    });
    draft.preview_weights = true;
    draft
}

#[test]
fn mesh_import_parity_preview_plays_standard_stand_on_an_independent_clock() {
    let avatars = crate::scene::avatar::AvatarLibrary::load();
    let rig = &avatars.rig;
    let draft = draft();
    let mut streamer = AnimStreamer::new(crate::settings::cache_dir());
    streamer.insert(
        crate::own_motion::ANIM_STAND,
        Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::idle_animation(), rig)),
    );
    let mut first = Player::default();
    let mut second = Player::default();
    let now = Instant::now();
    assert!(first.tick(&draft, rig, &mut streamer, now));
    second.tick(&draft, rig, &mut streamer, now + Duration::from_secs(1));
    let later = now + Duration::from_millis(1500);
    first.tick(&draft, rig, &mut streamer, later);
    second.tick(&draft, rig, &mut streamer, later);
    let wrist = rig.names["mWristLeft"];
    let first_pose = first.pose.as_ref().expect("standing pose");
    let rest = rig.default_world_inv[wrist].inverse();
    assert!(first_pose[wrist][3][2] < rest.w_axis.z - 0.2, "standing lowers the arm");
    let head = rig.names["mHead"];
    assert!(
        !Mat4::from_cols_array_2d(&first_pose[head])
            .abs_diff_eq(Mat4::from_cols_array_2d(&second.pose.as_ref().expect("second pose")[head]), 0.0001),
        "opening another preview must not restart an existing preview"
    );
    assert_eq!(first.controller.motion_count(), 1);
    assert_eq!(
        draft.scene.models[0].faces[0].vertices[0].weights, [0.0; 4],
        "the draft is not edited"
    );
}

#[test]
fn mesh_import_parity_preview_overrides_are_separate_from_upload_and_marker_flags() {
    let avatars = crate::scene::avatar::AvatarLibrary::load();
    let rig = &avatars.rig;
    let mut draft = draft();
    let mut streamer = AnimStreamer::new(crate::settings::cache_dir());
    let mut player = Player::default();
    let now = Instant::now();
    draft.options.joints = true;
    player.tick(&draft, rig, &mut streamer, now);
    let ordinary = player.pose.as_ref().expect("ordinary pose")[rig.pelvis][3][2];
    assert!((ordinary - rig.local_pos[rig.pelvis].z).abs() < 0.0001);
    draft.show_overrides = true;
    draft.options.pelvis = 0.2;
    player.tick(&draft, rig, &mut streamer, now + Duration::from_millis(20));
    assert!((player.pose.as_ref().expect("override pose")[rig.pelvis][3][2] - 1.6).abs() < 0.0001);
    assert!(!draft.show_joints);
    draft.preview_physics = true;
    assert!(!player.tick(&draft, rig, &mut streamer, now + Duration::from_millis(40)));
    assert!(player.pose.is_none());
}

#[test]
fn mesh_import_parity_animated_pose_keeps_the_bend_diagnostic_and_pelvis_offset() {
    let avatars = crate::scene::avatar::AvatarLibrary::load();
    let rig = &avatars.rig;
    let world = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    let mut pose: Vec<_> = world.iter().map(Mat4::to_cols_array_2d).collect();
    finish_pose(rig, &mut pose, 0.2, 60.0);
    assert!((pose[rig.pelvis][3][2] - world[rig.pelvis].w_axis.z - 0.2).abs() < 0.0001);
    for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
        let joint = rig.names[name];
        let expected = world[joint].w_axis.truncate() + Vec3::Z * 0.2;
        assert!(Mat4::from_cols_array_2d(&pose[joint]).w_axis.truncate().distance(expected) > 0.05);
    }
    assert!(pose.iter().all(|matrix| Mat4::from_cols_array_2d(matrix).is_finite()));
}
