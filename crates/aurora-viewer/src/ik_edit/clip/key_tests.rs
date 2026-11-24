//! Tests of the keyframe document rules (frame matching, copy, paste, delete).
use super::*;
use crate::scene::avatar::AvatarLibrary;
use aurora_assets::anim::{JointMotion, PosKey, RotKey};

fn frame(lib: &AvatarLibrary, rot: &[Quat]) -> Frame {
    let rig = &lib.rig;
    Frame {
        model: Mat4::IDENTITY,
        joints: rig.world_with(rot, Vec3::ZERO, &rig.local_pos, &rig.scale),
        at: Instant::now(),
    }
}

/// A head track whose keys sit at the given times, as an imported file (or an
/// FPS change) leaves them: not on the 30 fps frame grid.
fn imported_head(rig: &Rig, times: &[f32]) -> Document {
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document.tracks.push(document::Track {
        bone: head,
        enabled: true,
        authored: true,
        base_position: rig.local_pos[head],
        motion: JointMotion {
            joint_name: "mHead".into(),
            priority: -1,
            rot_keys: times
                .iter()
                .enumerate()
                .map(|(index, &time)| RotKey {
                    time,
                    rotation: Quat::from_rotation_z(index as f32 * 0.2),
                })
                .collect(),
            pos_keys: times
                .iter()
                .map(|&time| PosKey {
                    time,
                    position: Vec3::new(0.0, 0.0, time),
                })
                .collect(),
        },
        interpolation: document::Interpolation::Linear,
        curve: document::Curve::default(),
    });
    document
}

fn head_track(document: &Document, rig: &Rig) -> document::Track {
    let head = rig.names["mHead"];
    document
        .tracks
        .iter()
        .find(|track| track.bone == head)
        .cloned()
        .expect("the head track exists")
}

#[test]
fn a_key_belongs_to_the_frame_the_timeline_draws_it_on() {
    assert!(document::key_on_frame(0.5, 15, 30));
    assert!(document::key_on_frame(0.504, 15, 30), "rounds onto frame 15");
    assert!(document::key_on_frame(0.49, 15, 30));
    assert!(!document::key_on_frame(0.52, 15, 30), "15.6 rounds onto frame 16");
    assert!(!document::key_on_frame(f32::NAN, 0, 30));
    assert!(document::key_on_frame(0.2333, 6, 24), "a 30 fps key after a change to 24 fps");
}

#[test]
fn an_off_grid_key_can_be_deleted_from_the_frame_that_shows_it() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = imported_head(rig, &[0.0, 0.504, 1.0]);
    assert_eq!(document.frames(Some(head)), vec![0, 15, 30]);

    document.remove_track_keyframe(head, 15);

    assert_eq!(document.frames(Some(head)), vec![0, 30]);
    let track = head_track(&document, rig);
    assert_eq!(track.motion.rot_keys.len(), 2);
    assert_eq!(track.motion.pos_keys.len(), 2, "its position key goes with it");
}

#[test]
fn dragging_an_off_grid_key_snaps_it_to_the_destination_frame() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = imported_head(rig, &[0.0, 0.504, 1.0]);
    let moved = head_track(&document, rig).motion.rot_keys[1].rotation;

    document.move_track_keyframe(head, 15, 20);

    assert_eq!(document.frames(Some(head)), vec![0, 20, 30]);
    let track = head_track(&document, rig);
    assert_eq!(track.motion.rot_keys[1].time, 20.0 / 30.0, "on the frame's exact time");
    assert_eq!(track.motion.pos_keys[1].time, 20.0 / 30.0);
    assert_eq!(track.motion.rot_keys[1].rotation, moved, "the dragged key keeps its value");

    // dropped onto an off-grid key, the dragged key replaces it
    document.move_track_keyframe(head, 20, 30);
    let track = head_track(&document, rig);
    assert_eq!(document.frames(Some(head)), vec![0, 30]);
    assert_eq!(track.motion.rot_keys.len(), 2);
    assert_eq!(track.motion.rot_keys[1].rotation, moved);
}

#[test]
fn recording_on_an_off_grid_key_replaces_it_instead_of_adding_a_second_key() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    // 0.49 and 0.504 both round onto frame 15, as a denser import can
    let mut document = imported_head(rig, &[0.0, 0.49, 0.504, 1.0]);
    let mut rotations = rig.local_rot.clone();
    rotations[head] = Quat::from_rotation_y(0.4) * rotations[head];

    document.record(rig, &frame(&lib, &rotations), 15, &[head], &[]).unwrap();

    let track = head_track(&document, rig);
    let in_frame: Vec<_> = track
        .motion
        .rot_keys
        .iter()
        .filter(|key| document::key_on_frame(key.time, 15, 30))
        .collect();
    assert_eq!(in_frame.len(), 1, "one key per frame");
    assert_eq!(in_frame[0].time, 0.5);
    assert!(in_frame[0].rotation.dot(rotations[head]).abs() > 0.9999, "the recorded pose");
    let positions: Vec<_> = track
        .motion
        .pos_keys
        .iter()
        .filter(|key| document::key_on_frame(key.time, 15, 30))
        .collect();
    assert_eq!(positions.len(), 1, "an existing position channel is recorded once too");
    assert_eq!(document.frames(Some(head)), vec![0, 15, 30]);
}

#[test]
fn keys_left_between_frames_by_an_fps_change_stay_editable() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    document.record(rig, &frame(&lib, &rotations), 0, &[head], &[]).unwrap();
    rotations[head] = Quat::from_rotation_z(0.5) * rotations[head];
    document.record(rig, &frame(&lib, &rotations), 7, &[head], &[]).unwrap();

    document.set_fps(24);
    // 7/30 s is frame 5.6 at 24 fps: shown on frame 6
    assert_eq!(document.frames(Some(head)), vec![0, 6]);
    let mut pasted = document.clone();
    pasted.paste_keyframe(6, &document.copy_keyframe(0));
    assert_eq!(pasted.frames(Some(head)), vec![0, 6]);
    assert_eq!(head_track(&pasted, rig).motion.rot_keys.len(), 2, "the paste replaced the key");

    document.remove_track_keyframe(head, 6);
    assert_eq!(document.frames(Some(head)), vec![0]);
}

/// A fresh draft: every bone seeded at rest on frame 0 but not authored, as
/// `Editor::new_clip` leaves it, then the head keyed at frames 0 and 30.
fn draft_with_a_head_key(lib: &AvatarLibrary) -> Document {
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let bones: Vec<_> = (0..rig.len()).collect();
    let mut document = Document::new();
    document.record(rig, &frame(lib, &rig.local_rot), 0, &bones, &[]).unwrap();
    for track in &mut document.tracks {
        track.authored = false;
    }
    document.record(rig, &frame(lib, &rig.local_rot), 0, &[head], &[]).unwrap();
    let mut rotations = rig.local_rot.clone();
    rotations[head] = Quat::from_rotation_z(0.6) * rotations[head];
    document.record(rig, &frame(lib, &rotations), 30, &[head], &[]).unwrap();
    document
}

#[test]
fn copying_a_pose_leaves_the_untouched_baseline_tracks_out() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = draft_with_a_head_key(&lib);
    let baseline = document.tracks.len();
    assert!(baseline > 100, "a fresh draft seeds every bone");

    let copied = document.copy_keyframe(30);
    assert_eq!(copied.len(), 1, "only the authored track");
    assert_eq!(copied[0].bone, head);

    document.paste_keyframe(45, &copied);

    assert_eq!(document.frames(Some(head)), vec![0, 30, 45]);
    let authored: Vec<_> = document
        .tracks
        .iter()
        .filter(|track| track.authored)
        .map(|track| track.bone)
        .collect();
    assert_eq!(authored, vec![head], "untouched joints stay unauthored");
    assert_eq!(
        document.animation().unwrap().joints.len(),
        1,
        "the export still holds the head alone"
    );
    assert_eq!(
        document.preview().unwrap().joints.len(),
        baseline,
        "the preview keeps its rest baseline"
    );
}

#[test]
fn copying_a_pose_skips_disabled_tracks() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let wrist = rig.names["mWristRight"];
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    rotations[head] = Quat::from_rotation_z(0.3) * rotations[head];
    rotations[wrist] = Quat::from_rotation_x(0.3) * rotations[wrist];
    document.record(rig, &frame(&lib, &rotations), 10, &[head, wrist], &[]).unwrap();
    for track in &mut document.tracks {
        track.enabled = track.bone != wrist;
    }

    let copied = document.copy_keyframe(10);

    assert_eq!(copied.iter().map(|key| key.bone).collect::<Vec<_>>(), vec![head]);
}

#[test]
fn deleting_the_frame_keys_keeps_the_hidden_baseline() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = draft_with_a_head_key(&lib);
    let baseline = document.tracks.len();

    document.remove_keyframe(0);

    assert_eq!(document.frames(Some(head)), vec![30], "the authored key at frame 0 is gone");
    for track in document.tracks.iter().filter(|track| track.bone != head) {
        assert!(!track.authored);
        assert_eq!(track.motion.rot_keys.len(), 1, "{} keeps its rest key", track.motion.joint_name);
    }
    assert_eq!(document.preview().unwrap().joints.len(), baseline, "the preview stays at rest");
}

#[test]
fn picked_keys_move_together_and_keep_their_spacing() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = imported_head(rig, &[0.0, 10.0 / 30.0, 20.0 / 30.0, 1.0]);
    let values: Vec<Quat> = head_track(&document, rig).motion.rot_keys.iter().map(|key| key.rotation).collect();
    let picked = std::collections::BTreeSet::from([(head, 10), (head, 20)]);

    // moved onto each other's frames, neither replaces the other
    assert_eq!(document.move_keys(&picked, 10), 10);

    assert_eq!(document.frames(Some(head)), vec![0, 20, 30]);
    let track = head_track(&document, rig);
    assert_eq!(track.motion.rot_keys[1].rotation, values[1], "frame 10's key now on 20");
    assert_eq!(
        track.motion.rot_keys[2].rotation, values[2],
        "frame 20's key replaced the one on 30"
    );
    assert_eq!(track.motion.pos_keys.len(), 3, "the position keys move with them");
}

#[test]
fn a_group_move_stops_at_frame_zero_for_every_key() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = imported_head(rig, &[0.0, 5.0 / 30.0, 15.0 / 30.0, 1.0]);
    let picked = std::collections::BTreeSet::from([(head, 5), (head, 15)]);

    assert_eq!(document.move_keys(&picked, -12), -5, "the first key stops on frame 0");

    assert_eq!(document.frames(Some(head)), vec![0, 10, 30]);
    assert_eq!(document::clamp_shift([3, 40].into_iter(), 2000, 1800), Some(1760));
    assert_eq!(document::clamp_shift(std::iter::empty(), 4, 1800), None);
}

#[test]
fn a_group_move_past_the_end_grows_the_range() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = imported_head(rig, &[0.0, 1.0]);
    document.range(0, 30);

    document.move_keys(&std::collections::BTreeSet::from([(head, 30)]), 15);

    assert_eq!(document.end, 45);
    assert_eq!(document.frames(Some(head)), vec![0, 45]);
}

#[test]
fn deleting_picked_keys_removes_them_all_and_keeps_the_hidden_rest_keys() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let neck = rig.names["mNeck"];
    let mut document = imported_head(rig, &[0.0, 10.0 / 30.0, 20.0 / 30.0, 1.0]);
    let mut rest = head_track(&document, rig);
    rest.bone = neck;
    rest.authored = false;
    rest.motion.joint_name = "mNeck".into();
    document.tracks.push(rest);

    document.remove_selected_keys(&std::collections::BTreeSet::from([(head, 10), (head, 30), (neck, 10)]));

    assert_eq!(document.frames(Some(head)), vec![0, 20]);
    assert_eq!(document.frames(Some(neck)), vec![0, 10, 20, 30], "a draft's rest track stays whole");
}

#[test]
fn copied_keys_paste_at_the_playhead_with_their_spacing() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = imported_head(rig, &[0.0, 10.0 / 30.0, 0.5, 1.0]);
    document.range(0, 30);
    let source = head_track(&document, rig);
    let copied = document.copy_keys(&std::collections::BTreeSet::from([(head, 10), (head, 15)]));
    assert_eq!(copied.len(), 2);
    assert_eq!((copied[0].offset, copied[1].offset), (0, 5));
    assert_eq!(
        copied[0].rotation,
        Some(source.motion.rot_keys[1].rotation),
        "the key itself, not a sample"
    );

    let pasted = document.paste_keys(28, &copied);

    assert_eq!(pasted, std::collections::BTreeSet::from([(head, 28), (head, 33)]));
    assert_eq!(document.frames(Some(head)), vec![0, 10, 15, 28, 30, 33]);
    assert_eq!(document.end, 33, "the range grows to the last pasted key");
    let track = head_track(&document, rig);
    assert_eq!(track.motion.rot_keys[3].rotation, source.motion.rot_keys[1].rotation);
    assert_eq!(track.motion.pos_keys[5].position, source.motion.pos_keys[2].position);
    // past the length limit, a key is left out instead of piling up on the last frame
    let max = document.max_frame();
    assert_eq!(
        document.paste_keys(max - 2, &copied),
        std::collections::BTreeSet::from([(head, max - 2)])
    );
}
