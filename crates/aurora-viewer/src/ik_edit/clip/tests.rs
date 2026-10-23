use super::*;
use crate::scene::avatar::AvatarLibrary;

fn frame(lib: &AvatarLibrary, rot: &[Quat], positions: &[Vec3], scale: &[Vec3]) -> Frame {
    let model = Mat4::from_rotation_translation(Quat::from_euler(EulerRot::ZYX, 0.4, -0.2, 0.1), Vec3::new(132.0, 126.0, 40.0));
    Frame {
        model,
        joints: lib
            .rig
            .world_with(rot, Vec3::ZERO, positions, scale)
            .iter()
            .map(|m| model * *m)
            .collect(),
        at: Instant::now(),
    }
}
#[test]
fn native_capture_preserves_local_rotations_scaled_offsets_and_pelvis_position_without_locking_other_shape_positions() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let shoulder = rig.names["mShoulderLeft"];
    let elbow = rig.names["mElbowLeft"];
    let mut rot = rig.local_rot.clone();
    rot[shoulder] = Quat::from_rotation_x(0.4);
    rot[elbow] = Quat::from_rotation_z(-0.6);
    let mut positions = rig.local_pos.clone();
    positions[rig.pelvis] += Vec3::new(0.1, -0.2, 0.3);
    let mut scale = rig.scale.clone();
    scale[shoulder] = Vec3::new(1.2, 0.8, 1.4);
    let f = frame(&lib, &rot, &positions, &scale);
    let mut doc = Document::new();
    doc.record(rig, &f, 0, &[shoulder, elbow, rig.pelvis], &[]).unwrap();
    let animation = doc.animation().unwrap();
    let track = animation.joints.iter().find(|j| j.joint_name == "mElbowLeft").unwrap();
    assert!(track.rot_keys[0].rotation.dot(rot[elbow]).abs() > 0.99999);
    assert!(track.pos_keys.is_empty());
    let pelvis = animation.joints.iter().find(|j| j.joint_name == "mPelvis").unwrap();
    assert!(pelvis.pos_keys[0].position.abs_diff_eq(Vec3::new(0.1, -0.2, 0.3), 3e-5));
    rot[elbow] = Quat::from_rotation_z(-0.8);
    doc.record(rig, &frame(&lib, &rot, &positions, &scale), 60, &[elbow], &[elbow])
        .unwrap();
    assert!(
        doc.tracks.iter().find(|t| t.bone == elbow).unwrap().motion.pos_keys.is_empty(),
        "direct rotation alone must not freeze this shape's joint positions"
    );
    positions[elbow] += Vec3::new(0.02, 0.0, -0.01);
    let f = frame(&lib, &rot, &positions, &scale);
    doc.record(rig, &f, 30, &[elbow], &[elbow]).unwrap();
    let track = &doc.tracks.iter().find(|t| t.bone == elbow).unwrap().motion;
    assert_eq!(track.pos_keys.len(), 2);
    assert!(track.pos_keys[0].position.abs_diff_eq(rig.local_pos[elbow], 4e-5));
    assert!(track.pos_keys[1].position.abs_diff_eq(positions[elbow], 4e-5));
}
#[test]
fn crop_and_native_codec_round_trip_priorities_timing_keys_and_classic_metadata() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bone = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut doc = Document::new();
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[bone], &[])
        .unwrap();
    rotations[bone] = Quat::from_rotation_z(1.0);
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 60, &[bone], &[])
        .unwrap();
    doc.tracks[0].motion.priority = 5;
    doc.meta.base_priority = 3;
    doc.meta.ease_in = 0.2;
    doc.meta.ease_out = 0.3;
    doc.meta.hand_pose = 2;
    doc.meta.emote_name = "express_smile".into();
    doc.range(15, 45);
    doc.meta.loop_in = 0.1;
    doc.meta.loop_out = 0.9;
    let animation = doc.animation().unwrap();
    assert_eq!(animation.duration, 1.0);
    assert!(animation.joints[0].rot_keys[0].rotation.dot(Quat::from_rotation_z(0.25)).abs() > 0.9999);
    let bytes = aurora_assets::import_animation::encode(&animation).unwrap();
    let decoded = aurora_assets::anim::parse_animation_upload(&bytes).unwrap();
    assert_eq!(decoded.base_priority, 3);
    assert_eq!(decoded.joints[0].priority, 5);
    assert_eq!(decoded.duration, 1.0);
    assert_eq!(decoded.hand_pose, 2);
    assert_eq!(decoded.emote_name, "express_smile");
    assert_eq!(decoded.loop_in, 0.1);
    assert_eq!(decoded.loop_out, 0.9);
    assert_eq!(decoded.ease_in, 0.2);
    assert_eq!(decoded.ease_out, 0.3);
    assert_eq!(doc.frames(Some(bone)), vec![0, 60]);
    assert!(decoded.joints[0].rot_keys[1].rotation.dot(Quat::from_rotation_z(0.75)).abs() > 0.9999);
    doc.tracks[0].enabled = false;
    assert!(doc.animation().is_err());
    assert!(
        doc.preview().unwrap().joints.is_empty(),
        "the last disabled track releases its preview channel"
    );
}
#[test]
fn native_capture_rejects_out_of_range_positions_atomically() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let mut positions = rig.local_pos.clone();
    positions[rig.pelvis].x += 5.1;
    let mut doc = Document::new();
    assert!(
        doc.record(
            rig,
            &frame(&lib, &rig.local_rot, &positions, &rig.scale),
            0,
            &[rig.names["mHead"], rig.pelvis],
            &[]
        )
        .is_err()
    );
    assert!(doc.tracks.is_empty());
}

#[test]
fn recorded_ik_chains_release_only_their_pose_goals_after_the_new_sample_and_leaving_restores_contacts() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = lib.rig.clone();
    let world = World::new(lib.clone());
    let original = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(rig.names["mWristLeft"], &rig, &original);
    editor.commit();
    let saved = editor.goals[0].clone();
    editor.enter_clip(&world, &original);
    let ctx = egui::Context::default();
    let (jobs, _) = Jobs::new();
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    editor.tick_clip(&jobs, &ctx, rig.clone());
    editor.tick_clip(&jobs, &ctx, rig.clone());
    assert!(editor.goals.is_empty());
    editor.clip_seek(30);
    editor.select(rig.names["mWristLeft"], &rig, &original);
    editor.position += Vec3::new(0.04, 0.02, -0.01);
    editor.weight = 0.5;
    editor.commit();
    let wrist = editor.goals[0].clone();
    let path = ik::chain(&rig, wrist.bone, wrist.root).unwrap();
    editor.select(rig.names["mHead"], &rig, &original);
    editor.mode = Mode::Direct;
    editor.degrees.z += 20.0;
    editor.orient = true;
    editor.commit();
    let mut solved = ik::LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    };
    assert!(ik::apply_goal(&rig, &mut solved, Vec3::ZERO, &rig.scale, &wrist));
    let posed = frame(&lib, &solved.rotations, &solved.positions, &rig.scale);
    editor.clip.capture = Some(Capture::Bone(wrist.bone));
    editor.capture_clip(&world, &posed);
    assert_eq!(editor.goals.len(), 2, "hold the pose while the new preview is building");
    for bone in &path {
        assert!(editor.clip.document.as_ref().unwrap().frames(Some(*bone)).contains(&30));
    }
    editor.tick_clip(&jobs, &ctx, rig.clone());
    editor.tick_clip(&jobs, &ctx, rig.clone());
    assert!(editor.goals.iter().all(|g| g.bone != wrist.bone));
    assert!(editor.goals.iter().any(|g| g.bone == rig.names["mHead"]));
    editor.leave_clip();
    assert_eq!(editor.goals.len(), 1);
    assert_eq!(editor.goals[0].bone, saved.bone);
    assert_eq!(editor.goals[0].position, saved.position);
    editor.clip.clear_after = Some(editor.clip.revision);
    editor.clip.clear_bones = vec![saved.bone];
    let document = editor.clip.document.as_ref().unwrap();
    editor
        .clip
        .tx
        .send(files::Output::Preview {
            revision: editor.clip.revision,
            start: document.start,
            end: document.end,
            result: Ok(Arc::new(BoundAnim::bind(document.animation().unwrap(), &rig))),
        })
        .unwrap();
    editor.tick_clip(&jobs, &ctx, rig.clone());
    assert_eq!(editor.goals.len(), 1, "inactive creation cannot remove restored contacts");
    editor.toggle();
    editor.toggle();
    assert!(editor.clip.document.is_some());
    assert!(!editor.clip.active);
}

#[test]
fn current_ik_pose_can_be_added_to_an_existing_draft_and_new_drafts_preserve_previous_keys() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = lib.rig.clone();
    let world = World::new(lib.clone());
    let original = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(rig.names["mHead"], &rig, &original);
    editor.new_clip(&world, &original);
    editor.leave_clip();
    let old = editor.clip.document.as_ref().unwrap().clone();
    editor.clip.frame = 30;
    let mut rotations = rig.local_rot.clone();
    rotations[rig.names["mHead"]] = Quat::from_rotation_z(0.6);
    let posed = frame(&lib, &rotations, &rig.local_pos, &rig.scale);
    editor.capture_ik_clip(&world, &posed);
    assert!(editor.clip.active);
    assert!(editor.clip.waiting_source());
    assert_eq!(editor.clip.document.as_ref().unwrap().frames(Some(rig.names["mHead"])), vec![0, 30]);
    assert_eq!(
        old.frames(Some(rig.names["mHead"])),
        vec![0],
        "worker snapshots and retained drafts are immutable"
    );
    editor.leave_clip();
    editor.new_clip(&world, &original);
    assert_eq!(editor.clip.history.len(), 1);
    assert_eq!(editor.clip.history[0].frames(Some(rig.names["mHead"])), vec![0, 30]);
    assert_eq!(editor.clip.document.as_ref().unwrap().frames(Some(rig.names["mHead"])), vec![0]);
}
