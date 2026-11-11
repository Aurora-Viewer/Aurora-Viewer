use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::NetEvent;

#[test]
fn animation_playback_hides_bone_overlays_and_pause_restores_them() {
    let mut editor = Editor::default();
    editor.clip.active = true;
    assert!(editor.bone_overlays_visible());

    editor.clip.playing = true;
    assert!(!editor.bone_overlays_visible());

    editor.clip.playing = false;
    assert!(editor.bone_overlays_visible());
}

#[test]
fn timeline_opens_at_the_lower_right_and_fits_the_available_width() {
    let content = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(1536.0, 864.0));
    let size = timeline::timeline_default_size(content);
    let pos = timeline::timeline_default_position(content, size);

    assert_eq!(size, egui::vec2(836.0, 440.0));
    assert_eq!(pos.x, 694.0);
    assert_eq!(pos.y, 428.0);
    assert!(pos.x >= 10.0 + 24.0 + 530.0);
    assert_eq!(pos.x + size.x, content.right() - 16.0);
    assert_eq!(pos.y + size.y, content.bottom() - 16.0);

    let compact = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
    assert_eq!(timeline::timeline_default_size(compact), egui::vec2(700.0, 440.0));
}

#[test]
fn timeline_zoom_keeps_a_clamped_visible_frame_window() {
    assert_eq!(timeline::timeline_view(0, 1800, 1.0, 900), (0, 1800));
    let (start, end) = timeline::timeline_view(0, 1800, 64.0, 900);
    assert!(start <= 900 && 900 <= end);
    assert_eq!(end - start, 28);
    assert_eq!(timeline::timeline_view(10, 70, 8.0, u32::MAX), (62, 70));
    assert_eq!(timeline::timeline_view(0, 3600, 64.0, 0), (0, 56));
}

#[test]
fn timeline_zoom_centers_on_the_playhead_and_pan_can_reach_the_ends() {
    let start = timeline::timeline_start_around(0, 1800, 8.0, 900);
    let (left, right) = timeline::timeline_view(0, 1800, 8.0, start);
    assert_eq!((left + right) / 2, 900);
    assert_eq!(timeline::timeline_view(0, 1800, 8.0, u32::MAX).1, 1800);
}

#[test]
fn timeline_navigation_reveals_remote_seeks_but_keeps_visible_seeks_stable() {
    assert_eq!(timeline::reveal_timeline_frame(0, 1800, 8.0, 0, 100), 0);
    let start = timeline::reveal_timeline_frame(0, 1800, 8.0, 0, 1200);
    let (left, right) = timeline::timeline_view(0, 1800, 8.0, start);
    assert!(left <= 1200 && 1200 <= right);
    assert_eq!(timeline::reveal_timeline_frame(0, 1800, 8.0, 1600, 0), 0);
}

#[test]
fn timeline_follows_playback_without_moving_the_view_during_scrubbing() {
    assert_eq!(timeline::follow_timeline_playhead(0, 1800, 8.0, 0, 500, false), 0);
    assert_eq!(timeline::follow_timeline_playhead(0, 1800, 8.0, 0, 100, true), 0);
    let start = timeline::follow_timeline_playhead(0, 1800, 8.0, 0, 300, true);
    let (left, right) = timeline::timeline_view(0, 1800, 8.0, start);
    assert!(left < 300 && 300 <= right);
    assert_eq!(right - 300, 23, "keep the playhead near the right edge while advancing");
}

#[test]
fn changing_auto_advance_step_retimes_all_joint_keys_and_extends_the_timeline() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bone = rig.names["mShoulderLeft"];
    let mut document = Document::new();
    document.range(0, 30);
    document.tracks.push(document::Track {
        bone,
        enabled: true,
        authored: true,
        motion: aurora_assets::anim::JointMotion {
            joint_name: rig.canonical_names[bone].clone(),
            priority: 3,
            rot_keys: [0.0, 0.5, 1.0]
                .into_iter()
                .map(|time| aurora_assets::anim::RotKey {
                    time,
                    rotation: Quat::IDENTITY,
                })
                .collect(),
            pos_keys: [0.0, 0.5, 1.0]
                .into_iter()
                .map(|time| aurora_assets::anim::PosKey {
                    time,
                    position: Vec3::ZERO,
                })
                .collect(),
        },
        interpolation: document::Interpolation::Linear,
        curve: document::Curve::default(),
        base_position: rig.local_pos[bone],
    });

    assert!(document.retime_keys_to_step(0.5, 1.5));
    let track = &document.tracks[0];
    assert_eq!(
        track.motion.rot_keys.iter().map(|key| key.time).collect::<Vec<_>>(),
        [0.0, 1.5, 3.0]
    );
    assert_eq!(
        track.motion.pos_keys.iter().map(|key| key.time).collect::<Vec<_>>(),
        [0.0, 1.5, 3.0]
    );
    assert_eq!(document.end, 90);
}

#[test]
fn retiming_rejects_collapsed_keys_without_partial_changes() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bone = rig.names["mHead"];
    let mut document = Document::new();
    document.tracks.push(document::Track {
        bone,
        enabled: true,
        authored: true,
        motion: aurora_assets::anim::JointMotion {
            joint_name: rig.canonical_names[bone].clone(),
            priority: 3,
            rot_keys: [0.0, 1.0 / 30.0]
                .into_iter()
                .map(|time| aurora_assets::anim::RotKey {
                    time,
                    rotation: Quat::IDENTITY,
                })
                .collect(),
            pos_keys: Vec::new(),
        },
        interpolation: document::Interpolation::Linear,
        curve: document::Curve::default(),
        base_position: rig.local_pos[bone],
    });
    let before = document.tracks[0].motion.rot_keys.clone();

    assert!(!document.retime_keys_to_step(1.5, 0.1));
    assert_eq!(document.tracks[0].motion.rot_keys, before);
}

#[test]
fn studio_animation_timeline_stays_inside_narrow_viewports() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let palette = crate::theme::Theme::default().palette();
    let mut editor = Editor {
        open: true,
        clip: State {
            active: true,
            document: Some(Arc::new(Document::new())),
            ..Default::default()
        },
        ..Default::default()
    };
    let id = egui::Id::unique("avatar-studio-animation-timeline-window-v3");
    for (width, height, zoom) in [
        (360.0, 720.0, 1.0),
        (640.0, 720.0, 1.0),
        (1280.0, 720.0, 1.0),
        (640.0, 720.0, 1.25),
        (800.0, 720.0, 1.5),
        (640.0, 480.0, 1.0),
        (640.0, 360.0, 1.0),
    ] {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        let mut input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        if let Some(viewport) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            viewport.inner_rect = Some(screen);
        }
        let ctx = egui::Context::default();
        let mut run_frame = || ctx.run_ui(input.clone(), |ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world));
        let _ = run_frame();
        ctx.set_zoom_factor(zoom);
        let _ = run_frame();
        let timeline = ctx.memory(|memory| memory.area_rect(id)).expect("timeline overlay");
        let visible = ctx.content_rect();
        assert!(
            timeline.left() >= visible.left(),
            "left edge at {width}px at {zoom}x (context {visible:?}): {timeline:?}"
        );
        assert!(
            timeline.right() <= visible.right(),
            "right edge at {width}px at {zoom}x (context {visible:?}): {timeline:?}"
        );
        assert!(
            timeline.top() >= visible.top(),
            "top edge in {height}px-high window at {zoom}x (context {visible:?}): {timeline:?}"
        );
        assert!(
            timeline.bottom() <= visible.bottom(),
            "bottom edge in {height}px-high window at {zoom}x (context {visible:?}): {timeline:?}"
        );
    }
}

#[test]
fn closing_studio_timeline_releases_its_egui_hit_layer() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let palette = crate::theme::Theme::default().palette();
    let mut editor = Editor {
        open: true,
        clip: State {
            active: true,
            document: Some(Arc::new(Document::new())),
            ..Default::default()
        },
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
    let input = egui::RawInput {
        screen_rect: Some(screen),
        ..Default::default()
    };
    let id = egui::Id::unique("avatar-studio-animation-timeline-window-v3");
    let _ = ctx.run_ui(input.clone(), |ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world));
    assert!(ctx.memory(|memory| memory.areas().visible_layer_ids().iter().any(|layer| layer.id == id)));

    editor.clip.timeline_open = false;
    let _ = ctx.run_ui(input.clone(), |ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world));
    let _ = ctx.run_ui(input, |ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world));
    assert!(!ctx.memory(|memory| memory.areas().visible_layer_ids().iter().any(|layer| layer.id == id)));
}

#[test]
fn studio_timeline_key_hover_and_drag_move_the_key_under_the_pointer() {
    use crate::ui::widgets::elide_tests::Frames;
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let world = World::new(lib.clone());
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document.range(0, 30);
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    let mut rotations = rig.local_rot.clone();
    rotations[head] = Quat::from_rotation_z(0.4) * rotations[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    let mut editor = Editor {
        open: true,
        selected: Some(head),
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            ..Default::default()
        },
        ..Default::default()
    };
    let palette = crate::theme::Theme::default().palette();
    let mut frames = Frames::new();
    let mut draw = |ui: &mut egui::Ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world);
    frames.run(Vec::new(), &mut draw);
    let shapes = frames.shapes(Vec::new(), &mut draw);
    let baseline = shapes
        .iter()
        .filter_map(|shape| {
            let egui::Shape::LineSegment { points, .. } = shape else {
                return None;
            };
            ((points[0].y - points[1].y).abs() < 0.1 && (points[1].x - points[0].x) > 80.0).then_some(*points)
        })
        .max_by(|a, b| a[0].y.total_cmp(&b[0].y))
        .expect("timeline track baseline");
    let key = egui::pos2(baseline[0].x + 1.0, baseline[0].y);
    let (_, hover) = frames.run_with_output(vec![egui::Event::PointerMoved(key)], &mut draw);
    assert_eq!(hover.cursor_icon, egui::CursorIcon::Grab);
    let texts = frames.hover(key, &mut draw);
    assert!(texts.iter().any(|text| text.source.contains("Keyframe at frame 0")));
    let stable_shapes = frames.shapes(Vec::new(), &mut draw);
    let stable_baseline = stable_shapes
        .iter()
        .filter_map(|shape| {
            let egui::Shape::LineSegment { points, .. } = shape else {
                return None;
            };
            ((points[0].y - points[1].y).abs() < 0.1 && (points[1].x - points[0].x) > 80.0).then_some(*points)
        })
        .max_by(|a, b| a[0].y.total_cmp(&b[0].y))
        .expect("stable timeline track baseline");
    let destination = egui::pos2((stable_baseline[0].x + stable_baseline[1].x) * 0.5, stable_baseline[0].y);

    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frames.run(vec![egui::Event::PointerMoved(key), button(key, true)], &mut draw);
    frames.run(vec![egui::Event::PointerMoved(key + egui::vec2(7.0, 0.0))], &mut draw);
    drop(draw);
    assert_eq!(editor.clip.drag_key, Some((head, 0, 0)));
    let mut draw = |ui: &mut egui::Ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world);
    let (_, dragging) = frames.run_with_output(vec![egui::Event::PointerMoved(destination)], &mut draw);
    assert_eq!(dragging.cursor_icon, egui::CursorIcon::Grabbing);
    frames.run(vec![button(destination, false)], &mut draw);

    let document = editor.clip.document.as_ref().unwrap();
    assert_eq!(document.frames(Some(head)), vec![15, 30]);
    assert_eq!(editor.clip.drag_key, None);
}

#[test]
fn studio_timeline_scrolls_when_the_animation_has_many_tracks() {
    use crate::ui::widgets::elide_tests::Frames;
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let world = World::new(lib.clone());
    let bones = (0..rig.len()).collect::<Vec<_>>();
    let mut document = Document::new();
    document.range(0, 30);
    let pose = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);
    document.record(rig, &pose, 0, &bones, &[]).unwrap();
    document.record(rig, &pose, 30, &bones, &[]).unwrap();
    let track_names = document
        .tracks
        .iter()
        .map(|track| track.motion.joint_name.clone())
        .collect::<std::collections::HashSet<_>>();
    let first_bone = bones[0];
    let mut editor = Editor {
        open: true,
        selected: Some(first_bone),
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            ..Default::default()
        },
        ..Default::default()
    };
    let palette = crate::theme::Theme::default().palette();
    let mut frames = Frames::new();
    let mut draw = |ui: &mut egui::Ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world);
    for _ in 0..3 {
        frames.run(Vec::new(), &mut draw);
    }
    let before = frames.run(Vec::new(), &mut draw);
    let visible_before = before.iter().filter(|text| track_names.contains(&text.source)).collect::<Vec<_>>();
    assert!(visible_before.len() < track_names.len(), "the timeline should clip excess tracks");
    let first_visible = visible_before.first().expect("a visible animation track");
    let pointer = first_visible.rect.center();
    let scroll = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -800.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    };
    frames.run(vec![egui::Event::PointerMoved(pointer), scroll], &mut draw);
    let mut after = Vec::new();
    for _ in 0..6 {
        after = frames.run(Vec::new(), &mut draw);
    }
    let visible_after = after.iter().filter(|text| track_names.contains(&text.source)).collect::<Vec<_>>();
    assert_ne!(
        visible_after.first().map(|text| text.source.as_str()),
        visible_before.first().map(|text| text.source.as_str()),
        "scrolling over the timeline tracks should reveal later bones"
    );
}

#[test]
fn selected_avatar_frame_values_stay_still_until_the_playhead_moves() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let pelvis = rig.pelvis;
    let mut editor = Editor {
        selected: Some(pelvis),
        clip: State {
            active: true,
            document: Some(Arc::new(Document::new())),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut live = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);

    editor.sync_clip_selection(&live);
    let paused_position = editor.position;
    let (scale, rotation, position) = live.joints[pelvis].to_scale_rotation_translation();
    live.joints[pelvis] = Mat4::from_scale_rotation_translation(scale, rotation, position + live.model.transform_vector3(Vec3::Z * 0.4));
    editor.sync_clip_selection(&live);
    assert_eq!(
        editor.position, paused_position,
        "live idle motion must not overwrite a paused edit value"
    );

    editor.clip.cursor = 15.0;
    editor.clip.frame = 15;
    editor.sync_clip_selection(&live);
    assert!(
        (editor.position.z - paused_position.z - 0.4).abs() < 0.0001,
        "seeking the playhead refreshes the selected joint's position"
    );

    editor.clip.playing = true;
    live.joints[pelvis] = Mat4::from_scale_rotation_translation(scale, rotation, position + live.model.transform_vector3(Vec3::Z * 0.8));
    editor.sync_clip_selection(&live);
    assert!(
        (editor.position.z - paused_position.z - 0.8).abs() < 0.0001,
        "playing preview continues to refresh the selected joint"
    );
}

#[test]
fn pelvis_height_keys_lift_the_whole_avatar_in_the_native_animation() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let pelvis = rig.pelvis;
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document.range(0, 30);
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &[pelvis], &[])
        .unwrap();
    let mut jumping = rig.local_pos.clone();
    jumping[pelvis].z += 0.7;
    document
        .record(rig, &frame(&lib, &rig.local_rot, &jumping, &rig.scale), 15, &[pelvis], &[pelvis])
        .unwrap();
    document
        .record(
            rig,
            &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale),
            30,
            &[pelvis],
            &[pelvis],
        )
        .unwrap();

    let encoded = aurora_assets::import_animation::encode(&document.animation().unwrap()).unwrap();
    let animation = aurora_assets::anim::parse_animation_upload(&encoded).unwrap();
    let sample = crate::scene::ik::animations::Sample {
        animation: Arc::new(crate::scene::anim::BoundAnim::bind(animation, rig)),
        frame: 15,
        time_seconds: Some(0.5),
    };
    let mut pose = ik::LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    };
    crate::scene::ik::animations::apply(rig, &mut pose, Vec3::ZERO, &[sample]);
    let baseline = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    let lifted = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &rig.scale);
    assert!((lifted[head].w_axis.z - baseline[head].w_axis.z - 0.7).abs() < 0.001);
}

#[test]
fn animation_timeline_shortcuts_match_the_documented_commands() {
    use timeline::TimelineShortcut as Shortcut;
    let none = egui::Modifiers::NONE;
    let ctrl = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let ctrl_shift = egui::Modifiers {
        ctrl: true,
        shift: true,
        ..Default::default()
    };
    let ctrl_alt = egui::Modifiers {
        ctrl: true,
        alt: true,
        ..Default::default()
    };

    assert_eq!(timeline::timeline_shortcut(egui::Key::Space, none), Some(Shortcut::PlayPause));
    assert_eq!(timeline::timeline_shortcut(egui::Key::Home, none), Some(Shortcut::Rewind));
    assert_eq!(timeline::timeline_shortcut(egui::Key::Delete, none), Some(Shortcut::DeleteKey));
    assert_eq!(timeline::timeline_shortcut(egui::Key::Z, ctrl), Some(Shortcut::Undo));
    assert_eq!(timeline::timeline_shortcut(egui::Key::Y, ctrl), Some(Shortcut::Redo));
    assert_eq!(timeline::timeline_shortcut(egui::Key::Z, ctrl_shift), Some(Shortcut::Redo));
    assert_eq!(timeline::timeline_shortcut(egui::Key::C, ctrl), Some(Shortcut::CopyPose));
    assert_eq!(timeline::timeline_shortcut(egui::Key::V, ctrl), Some(Shortcut::PastePose));
    assert_eq!(timeline::timeline_shortcut(egui::Key::S, ctrl), Some(Shortcut::SaveProject));
    assert_eq!(
        timeline::timeline_shortcut(egui::Key::ArrowLeft, none),
        Some(Shortcut::PreviousFrame)
    );
    assert_eq!(timeline::timeline_shortcut(egui::Key::ArrowRight, none), Some(Shortcut::NextFrame));
    assert_eq!(timeline::timeline_shortcut(egui::Key::K, none), Some(Shortcut::KeyBone));
    assert_eq!(timeline::timeline_shortcut(egui::Key::Z, ctrl_alt), None);
    assert_eq!(timeline::timeline_shortcut(egui::Key::V, none), None);
    assert_eq!(timeline::timeline_shortcut(egui::Key::K, ctrl), None);
    assert_eq!(timeline::timeline_shortcut(egui::Key::ArrowLeft, ctrl), None);
}

#[test]
fn animation_timeline_shortcut_keys_follow_the_typed_character() {
    use winit::keyboard::{Key, NamedKey};

    for (typed, expected) in [
        ("z", egui::Key::Z),
        ("Z", egui::Key::Z),
        ("y", egui::Key::Y),
        ("Y", egui::Key::Y),
        ("c", egui::Key::C),
        ("v", egui::Key::V),
        ("s", egui::Key::S),
        ("k", egui::Key::K),
        ("K", egui::Key::K),
    ] {
        assert_eq!(timeline::timeline_key(&Key::Character(typed.into())), Some(expected));
    }
    assert_eq!(timeline::timeline_key(&Key::Named(NamedKey::Space)), Some(egui::Key::Space));
    assert_eq!(timeline::timeline_key(&Key::Named(NamedKey::Home)), Some(egui::Key::Home));
    assert_eq!(timeline::timeline_key(&Key::Named(NamedKey::Delete)), Some(egui::Key::Delete));
    assert_eq!(timeline::timeline_key(&Key::Named(NamedKey::ArrowLeft)), Some(egui::Key::ArrowLeft));
    assert_eq!(
        timeline::timeline_key(&Key::Named(NamedKey::ArrowRight)),
        Some(egui::Key::ArrowRight)
    );
    assert_eq!(timeline::timeline_key(&Key::Character("ß".into())), None);
}

#[test]
fn arrow_keys_step_one_frame_within_the_range_and_k_keys_the_selected_bone() {
    let mut document = Document::new();
    document.range(0, 30);
    let mut editor = Editor {
        open: true,
        selected: Some(7),
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            ..Default::default()
        },
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let press = |editor: &mut Editor, key| assert!(editor.handle_timeline_shortcut(&ctx, key, egui::Modifiers::NONE));
    editor.clip_seek(10);

    press(&mut editor, egui::Key::ArrowRight);
    assert_eq!(editor.clip.frame, 11);
    press(&mut editor, egui::Key::ArrowLeft);
    press(&mut editor, egui::Key::ArrowLeft);
    assert_eq!(editor.clip.frame, 9);
    assert_eq!(editor.clip.cursor, 9.0, "the continuous playhead follows the step");

    editor.clip_seek(30);
    press(&mut editor, egui::Key::ArrowRight);
    assert_eq!(editor.clip.frame, 30, "stepping stops at the end of the range");
    editor.clip_seek(0);
    press(&mut editor, egui::Key::ArrowLeft);
    assert_eq!(editor.clip.frame, 0, "and at its start");
    assert_eq!(editor.clip.document.as_ref().unwrap().end, 30, "stepping never extends the range");

    editor.studio_toggle_clip_playback();
    press(&mut editor, egui::Key::ArrowRight);
    assert!(!editor.clip.playing, "stepping pauses playback");

    assert!(editor.clip.capture.is_none());
    press(&mut editor, egui::Key::K);
    assert!(matches!(editor.clip.capture, Some(Capture::Bone(7))));
    editor.clip.capture = None;
    editor.selected = None;
    press(&mut editor, egui::Key::K);
    assert!(editor.clip.capture.is_none(), "K keys nothing without a selected bone");
}

#[test]
fn timeline_ruler_labels_stay_apart_at_every_zoom() {
    let steps = [1, 5, 10, 30, 60, 150, 300, 600];
    assert_eq!(timeline::ruler_tick_step(60.0), 1);
    assert_eq!(timeline::ruler_tick_step(20.0), 5);
    assert_eq!(timeline::ruler_tick_step(1.0), 60);
    assert_eq!(timeline::ruler_tick_step(0.01), 6000, "past the longest step, a multiple of it");
    assert_eq!(timeline::ruler_tick_step(0.0), 600);
    assert_eq!(timeline::ruler_tick_step(f32::NAN), 600);
    for points_per_frame in [0.03, 0.1, 0.17, 0.5, 2.0, 7.3, 16.6, 33.0, 80.0] {
        let step = timeline::ruler_tick_step(points_per_frame);
        assert!(step as f32 * points_per_frame >= 60.0, "{step} frames at {points_per_frame} points");
        if let Some(index) = steps.iter().position(|candidate| *candidate == step)
            && index > 0
        {
            assert!(
                steps[index - 1] as f32 * points_per_frame < 60.0,
                "the closest step that fits at {points_per_frame} points"
            );
        }
    }
    assert_eq!(timeline::ruler_minor_step(1), None);
    for step in steps.into_iter().skip(1).chain([1200, 6000]) {
        let minor = timeline::ruler_minor_step(step).expect("a minor step");
        assert!(minor < step && step % minor == 0, "{minor} divides {step}");
    }
}

#[test]
fn ctrl_wheel_zoom_keeps_the_frame_under_the_pointer() {
    let (zoom, start) = timeline::zoom_timeline_around(0, 1800, 1.0, 0, 0.25, 2.0);
    assert_eq!(zoom, 2.0);
    let (left, right) = timeline::timeline_view(0, 1800, zoom, start);
    let under_pointer = left as f32 + 0.25 * (right - left) as f32;
    assert!(
        (under_pointer - 450.0).abs() <= 1.0,
        "frame 450 stays under the pointer: {under_pointer}"
    );

    // frame 1035 sits at 90 % of the view 225..1125
    let (zoom, start) = timeline::zoom_timeline_around(0, 1800, zoom, start, 0.9, 1.5);
    assert_eq!(zoom, 3.0);
    let (left, right) = timeline::timeline_view(0, 1800, zoom, start);
    assert!((left as f32 + 0.9 * (right - left) as f32 - 1035.0).abs() <= 1.0);

    assert_eq!(timeline::zoom_timeline_around(0, 1800, 64.0, 0, 0.5, 4.0).0, 64.0);
    assert_eq!(timeline::zoom_timeline_around(0, 1800, 1.0, 0, 0.5, 0.25), (1.0, 0));
    assert_eq!(timeline::zoom_timeline_around(0, 1800, 2.0, 100, 0.5, f32::NAN), (2.0, 100));
    let (_, edge) = timeline::zoom_timeline_around(0, 1800, 1.0, 0, 1.0, 8.0);
    assert_eq!(
        timeline::timeline_view(0, 1800, 8.0, edge).1,
        1800,
        "zooming at the end keeps the end in view"
    );
}

#[test]
fn shift_wheel_pans_within_the_animation() {
    assert_eq!(timeline::pan_timeline(0, 1800, 4.0, 0, 100), 100);
    assert_eq!(timeline::pan_timeline(0, 1800, 4.0, 100, -500), 0);
    assert_eq!(timeline::pan_timeline(0, 1800, 4.0, 1000, 5000), 1350, "the view stops at the end");
    assert_eq!(timeline::pan_timeline(0, 1800, 1.0, 0, 300), 0, "nothing to pan at full view");
}

#[test]
fn ruler_and_lanes_share_one_frame_scale() {
    let lane = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(314.0, 20.0));
    let scale = timeline::FrameScale::new(lane, 0, 30);
    assert_eq!(scale.x(0.0), 107.0, "the first frame sits inside the lane's padding");
    assert_eq!(scale.x(30.0), 407.0);
    assert_eq!(scale.points_per_frame(), 10.0);
    assert!((scale.x(12.5) - 232.0).abs() < 1e-3, "a playing playhead sits between frames");
    assert_eq!(scale.frame_at(151.0), 4);
    assert_eq!(scale.frame_at(0.0), 0);
    assert_eq!(scale.frame_at(1000.0), 30);

    let keys = [0, 10, 20, 30];
    assert_eq!(timeline::key_under(&keys, &scale, 108.0), Some(0));
    assert_eq!(timeline::key_under(&keys, &scale, 215.0), Some(10));
    assert_eq!(timeline::key_under(&keys, &scale, 216.0), None, "beyond the grab distance");
    assert_eq!(timeline::key_under(&keys, &scale, 157.0), None);
    let zoomed = timeline::FrameScale::new(lane, 10, 20);
    assert_eq!(timeline::key_under(&keys, &zoomed, 100.0), Some(10), "only the visible keys");
    assert_eq!(timeline::key_under(&[0, 30], &zoomed, 107.0), None);
}

#[test]
fn previous_and_next_key_follow_the_selected_bone_or_every_keyed_bone() {
    let track = |bone, authored, keys: &[u32]| TimelineTrack {
        bone,
        name: format!("bone {bone}"),
        enabled: true,
        authored,
        keys: keys.to_vec(),
    };
    let tracks = [track(1, true, &[0, 30, 60]), track(2, true, &[0, 45]), track(3, false, &[0, 10])];
    assert_eq!(timeline::neighbour_key(&tracks, Some(1), 30, true), Some(60));
    assert_eq!(timeline::neighbour_key(&tracks, Some(1), 30, false), Some(0));
    assert_eq!(timeline::neighbour_key(&tracks, Some(1), 60, true), None);
    assert_eq!(
        timeline::neighbour_key(&tracks, Some(3), 0, true),
        Some(10),
        "a selected bone's own track"
    );
    assert_eq!(timeline::neighbour_key(&tracks, None, 30, true), Some(45));
    assert_eq!(timeline::neighbour_key(&tracks, None, 50, false), Some(45));
    assert_eq!(
        timeline::neighbour_key(&tracks, None, 5, true),
        Some(30),
        "the hidden rest key of an untouched bone is skipped"
    );
    assert_eq!(
        timeline::neighbour_key(&tracks, Some(9), 5, true),
        Some(30),
        "a bone without a track"
    );
}

#[test]
fn frame_keys_ignore_the_hidden_rest_pose_of_a_fresh_draft() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let bones = (0..rig.len()).collect::<Vec<_>>();
    let mut document = Document::new();
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &bones, &[])
        .unwrap();
    for track in &mut document.tracks {
        track.authored = false;
    }
    let mut editor = Editor {
        open: true,
        selected: Some(head),
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!editor.clip.has_key(None, 0), "no key of the frame on a fresh draft");
    assert!(
        editor.clip.has_key(Some(head), 0),
        "the selected bone's lane still shows its rest key"
    );

    let tracks = editor.clip.document.as_ref().unwrap().tracks.len();
    editor.studio_delete_clip_keyframe();
    let document = editor.clip.document.as_ref().unwrap();
    assert_eq!(document.tracks.len(), tracks);
    assert!(
        document.tracks.iter().all(|track| !track.motion.rot_keys.is_empty()),
        "deleting the frame's keys keeps the rest pose of the untouched bones"
    );
    assert_eq!(document.frames(None), vec![0]);
}

#[test]
fn releasing_a_key_drag_commits_the_frame_it_reached() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document.range(0, 30);
    let mut rotations = rig.local_rot.clone();
    for at in [0, 30] {
        rotations[head] = Quat::from_rotation_z(at as f32 / 60.0) * rig.local_rot[head];
        document
            .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), at, &[head], &[])
            .unwrap();
    }
    let mut editor = Editor {
        open: true,
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            drag_key: Some((head, 0, 15)),
            ..Default::default()
        },
        ..Default::default()
    };

    editor.finish_timeline_key_drag(true);
    assert_eq!(
        editor.clip.drag_key,
        Some((head, 0, 15)),
        "the drag goes on while the button is down"
    );
    assert!(editor.clip.undo.is_empty());

    // no pointer position is needed at the release: the drag's frame is kept
    editor.finish_timeline_key_drag(false);
    assert_eq!(editor.clip.document.as_ref().unwrap().frames(Some(head)), vec![15, 30]);
    assert_eq!(editor.clip.drag_key, None);
    assert_eq!(editor.clip.undo.len(), 1);
    assert_eq!(editor.clip.frame, 15);
}

#[test]
fn dragging_the_auto_key_step_fills_one_undo_entry() {
    use crate::ui::widgets::elide_tests::Frames;
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let mut document = Document::new();
    document.auto_keyframe = true;
    let mut editor = Editor {
        open: true,
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            ..Default::default()
        },
        ..Default::default()
    };
    let palette = crate::theme::Theme::default().palette();
    let mut frames = Frames::new();
    let mut draw = |ui: &mut egui::Ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world);
    frames.run(Vec::new(), &mut draw);
    let texts = frames.run(Vec::new(), &mut draw);
    let step = texts
        .iter()
        .find(|text| text.source.starts_with("0.5"))
        .expect("the auto-key step value")
        .rect
        .center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frames.run(vec![egui::Event::PointerMoved(step), button(step, true)], &mut draw);
    for offset in 1..=6 {
        frames.run(
            vec![egui::Event::PointerMoved(step + egui::vec2(8.0 * offset as f32, 0.0))],
            &mut draw,
        );
    }
    frames.run(vec![button(step + egui::vec2(48.0, 0.0), false)], &mut draw);
    drop(draw);

    let seconds = editor.clip.document.as_ref().unwrap().auto_key_advance_seconds;
    assert!(seconds > 0.5, "the drag changed the step: {seconds}");
    assert_eq!(editor.clip.undo.len(), 1, "one undo entry for the whole drag");
}

#[test]
fn the_playhead_crosses_the_ruler_and_the_lanes_at_the_frame_scale_of_both() {
    use crate::ui::widgets::elide_tests::Frames;
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let world = World::new(lib.clone());
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document.range(0, 30);
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    let mut editor = Editor {
        open: true,
        selected: Some(head),
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            frame: 10,
            cursor: 10.5,
            ..Default::default()
        },
        ..Default::default()
    };
    let palette = crate::theme::Theme::default().palette();
    let mut frames = Frames::new();
    let mut draw = |ui: &mut egui::Ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world);
    frames.run(Vec::new(), &mut draw);
    let shapes = frames.shapes(Vec::new(), &mut draw);
    let segments = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::LineSegment { points, stroke } => Some((*points, *stroke)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let baseline = segments
        .iter()
        .map(|(points, _)| *points)
        .filter(|points| (points[0].y - points[1].y).abs() < 0.1 && (points[1].x - points[0].x) > 80.0)
        .max_by(|a, b| a[0].y.total_cmp(&b[0].y))
        .expect("the head lane's baseline");
    let playhead = segments
        .iter()
        .find(|(points, stroke)| stroke.color == palette.teal && (points[0].x - points[1].x).abs() < 0.1)
        .map(|(points, _)| *points)
        .expect("the playhead line");
    let expected = baseline[0].x + (baseline[1].x - baseline[0].x) * 10.5 / 30.0;
    assert!(
        (playhead[0].x - expected).abs() < 0.5,
        "the playhead sits at the continuous cursor on the lane's scale: {} vs {expected}",
        playhead[0].x
    );
    let (top, bottom) = (playhead[0].y.min(playhead[1].y), playhead[0].y.max(playhead[1].y));
    assert!(
        top < baseline[0].y - 20.0 && bottom > baseline[0].y,
        "one line from the ruler down through the lane"
    );

    let texts = frames.run(Vec::new(), &mut draw);
    let zero = texts
        .iter()
        .find(|text| text.source == "0")
        .expect("the ruler's first frame number");
    assert!(
        (zero.rect.center().x - baseline[0].x).abs() < 1.0,
        "the ruler's frame 0 stands over the lane's frame 0"
    );
}

#[test]
fn automatic_key_capture_waits_for_release_and_scrubbing_cancels_pending_capture() {
    let mut clip = State {
        auto_key_pending: true,
        ..Default::default()
    };
    clip.modified.insert(17);

    clip.finish_auto_key_on_release(true);
    assert!(clip.auto_key_pending, "hold the edit until the drag ends");
    assert!(clip.capture.is_none(), "do not key intermediate drag samples");

    clip.finish_auto_key_on_release(false);
    assert!(!clip.auto_key_pending);
    assert!(matches!(clip.capture, Some(Capture::AutoKeyModified)));

    clip.seek(30);
    assert!(!clip.auto_key_pending, "moving the playhead cancels stale auto-key work");
    assert!(clip.modified.is_empty());
    assert!(clip.capture.is_none());
}

#[test]
fn leaving_the_clip_editor_discards_pending_auto_key_work() {
    let mut editor = Editor {
        clip: State {
            active: true,
            auto_key_pending: true,
            ..Default::default()
        },
        ..Default::default()
    };
    editor.clip.modified.insert(17);
    editor.clip.translations.insert(17);
    editor.clip.capture = Some(Capture::Modified);

    editor.leave_clip();

    assert!(!editor.clip.active);
    assert!(!editor.clip.auto_key_pending);
    assert!(editor.clip.modified.is_empty());
    assert!(editor.clip.translations.is_empty());
    assert!(editor.clip.capture.is_none());
}

#[test]
fn animation_editor_disables_pose_only_scale_mode_and_restores_it_on_exit() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = lib.rig.clone();
    let world = World::new(lib.clone());
    let frame = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);
    let mut editor = Editor::default();
    editor.toggle();
    editor.poser.transform = 2;

    editor.enter_clip(&world, &frame);

    assert!(editor.clip.active);
    assert_eq!(editor.poser.transform, 0, "native clips cannot key joint scale");
    assert!(matches!(editor.tool, Tool::Rotate));
    assert_eq!(editor.clip.saved_pose_transform, Some(2));

    editor.leave_clip();

    assert_eq!(editor.poser.transform, 2, "restore the static Poser tool the user selected");
    assert!(matches!(editor.tool, Tool::Move));
    assert_eq!(editor.clip.saved_pose_transform, None);
}

#[test]
fn timeline_shortcuts_yield_to_focused_controls_and_run_when_the_editor_owns_input() {
    let mut editor = Editor {
        open: true,
        clip: State {
            active: true,
            document: Some(Arc::new(Document::new())),
            ..Default::default()
        },
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let text_field = egui::Id::unique("animation-name-field");
    ctx.memory_mut(|memory| memory.request_focus(text_field));

    assert!(
        !editor.handle_timeline_shortcut(&ctx, egui::Key::Space, egui::Modifiers::NONE),
        "a focused editor field owns Space"
    );
    assert!(!editor.clip.playing);

    ctx.memory_mut(|memory| memory.surrender_focus(text_field));
    assert!(editor.handle_timeline_shortcut(&ctx, egui::Key::Space, egui::Modifiers::NONE));
    assert!(editor.clip.playing, "the editor handles Space when no control owns focus");
}

#[test]
fn undo_and_redo_history_is_scoped_to_the_current_animation_draft() {
    let mut state = State {
        document: Some(Arc::new(Document::new())),
        ..Default::default()
    };
    state.checkpoint();
    if let Some(document) = &mut state.document {
        Arc::make_mut(document).name = "Edited draft".into();
    }
    assert!(state.undo());
    assert_eq!(state.document.as_ref().unwrap().name, aurora_i18n::tr!("anim-edit-new-name"));
    assert!(state.redo());
    assert_eq!(state.document.as_ref().unwrap().name, "Edited draft");

    state.document = Some(Arc::new(Document::new()));
    state.clear_undo_history();
    assert!(!state.undo(), "a new draft must not restore the previous draft");
    assert!(!state.redo(), "a new draft must not replay the previous draft");
}

#[test]
fn cancelling_a_file_dialog_keeps_the_active_draft_and_clears_busy_state() {
    let lib = Arc::new(AvatarLibrary::load());
    let document = Arc::new(Document::new());
    let name = document.name.clone();
    let mut state = State {
        document: Some(document.clone()),
        busy: true,
        preview_revision: Some(0),
        ..Default::default()
    };
    state.tx.send(files::Output::Cancelled).unwrap();

    let (jobs, _) = Jobs::new();
    let context = egui::Context::default();
    state.tick(&jobs, &context, lib.rig.clone(), lib.skeleton.as_ref());

    assert!(!state.busy);
    assert!(
        Arc::ptr_eq(state.document.as_ref().unwrap(), &document),
        "cancelling load or save leaves the current draft untouched"
    );
    assert_eq!(state.document.as_ref().unwrap().name, name);
}

#[test]
fn starting_a_file_action_clears_feedback_from_the_previous_operation() {
    let mut editor = Editor::default();
    editor.clip.message = Some("Saved old project".into());
    editor.clip.error = Some("Old error".into());

    editor.queue_studio_file_action(files::Action::LoadProject);

    assert!(editor.clip.busy);
    assert!(editor.clip.export.is_some());
    assert!(editor.clip.message.is_none());
    assert!(editor.clip.error.is_none());
}

#[test]
fn timeline_interpolation_modes_are_distinct_and_native_easing_is_baked_at_sixty_hz() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut doc = Document::new();
    doc.range(0, 60);
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(1.0) * rotations[head];
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 60, &[head], &[])
        .unwrap();
    {
        let track = &mut doc.tracks[0];
        track.interpolation = document::Interpolation::Smooth;
        let quarter = track.rotation_at(0.5).expect("quarter sample");
        assert!(quarter.dot(Quat::from_rotation_z(0.15625) * rig.local_rot[head]).abs() > 0.9999);
        track.interpolation = document::Interpolation::Linear;
        let quarter = track.rotation_at(0.5).expect("linear quarter sample");
        assert!(quarter.dot(Quat::from_rotation_z(0.25) * rig.local_rot[head]).abs() > 0.9999);
        track.interpolation = document::Interpolation::Stepped;
        let quarter = track.rotation_at(0.5).expect("stepped quarter sample");
        assert!(quarter.dot(rig.local_rot[head]).abs() > 0.99999);
        track.interpolation = document::Interpolation::Curve;
        track.curve = document::Curve { start: 0.1, end: 0.2 };
        let quarter = track.rotation_at(0.5).expect("custom curve quarter sample");
        assert!(quarter.dot(Quat::from_rotation_z(0.0859375) * rig.local_rot[head]).abs() > 0.9999);
    }
    let custom_curve = doc.animation().unwrap();
    assert_eq!(custom_curve.joints[0].rot_keys.len(), 121);
    assert!(
        custom_curve.joints[0].rot_keys[30]
            .rotation
            .dot(Quat::from_rotation_z(0.0859375) * rig.local_rot[head])
            .abs()
            > 0.9999
    );
    let encoded = aurora_assets::import_animation::encode(&custom_curve).unwrap();
    let exported = aurora_assets::anim::parse_animation_upload(&encoded).unwrap();
    let exported_joint = exported
        .joints
        .iter()
        .find(|joint| joint.joint_name == rig.canonical_names[head])
        .expect("exported head track");
    assert_eq!(exported_joint.rot_keys.len(), 121);
    assert!(
        exported_joint.rot_keys[30]
            .rotation
            .dot(Quat::from_rotation_z(0.0859375) * rig.local_rot[head])
            .abs()
            > 0.999
    );
    doc.tracks[0].interpolation = document::Interpolation::Smooth;
    let baked = doc.animation().unwrap();
    assert_eq!(baked.joints[0].rot_keys.len(), 121);
    assert_eq!(baked.joints[0].rot_keys[30].time, 0.5);
}

#[test]
fn animation_preview_samples_the_fractional_playhead_between_timeline_frames() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(1.0) * rig.local_rot[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    let bound = Arc::new(BoundAnim::bind(document.preview().unwrap(), rig));
    let mut state = State {
        active: true,
        document: Some(Arc::new(document)),
        preview: Some(Preview {
            revision: 0,
            start: 0,
            end: 900,
            bound,
        }),
        ..Default::default()
    };
    state.frame = 0;
    state.cursor = 0.5;

    let sample = state.sample().pop().unwrap();
    assert_eq!(sample.frame, 1);
    assert!((sample.time_seconds.unwrap() - (0.5 / FPS)).abs() < 1e-6);
}

#[test]
fn timeline_playback_loop_supports_full_range_or_last_authored_key() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    document.fps = 10;
    document.start = 10;
    document.end = 40;
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 10, &[head], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(0.5) * rotations[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 25, &[head], &[])
        .unwrap();

    let (cursor, playing) = advance_playhead(15.0, 4.0, &document, true, true);
    assert!(playing);
    assert!(
        (cursor - 23.0).abs() < 1e-9,
        "playback wraps after the last key, not the timeline end"
    );

    let (cursor, playing) = advance_playhead(35.0, 0.7, &document, true, false);
    assert!(playing);
    assert!((cursor - 12.0).abs() < 1e-9, "full-timeline mode keeps the existing loop range");

    let (cursor, playing) = advance_playhead(35.0, 0.7, &document, false, false);
    assert!(!playing);
    assert_eq!(cursor, 40.0);
}

#[test]
fn auto_key_advance_uses_the_selected_step_and_extends_the_timeline_when_needed() {
    let mut document = Document::new();
    document.fps = 30;
    document.range(0, 30);
    assert_eq!(document.auto_key_advance_seconds, 0.5);
    assert_eq!(next_auto_key_frame(&mut document, 0), Some(15));
    assert_eq!(document.end, 30);

    document.auto_key_advance_seconds = 0.2;
    assert_eq!(next_auto_key_frame(&mut document, 15), Some(21));
    assert_eq!(document.end, 30, "the next pose remains inside the visible timeline range");

    let mut at_limit = Document::new();
    at_limit.fps = 30;
    let max_frame = at_limit.max_frame();
    assert_eq!(next_auto_key_frame(&mut at_limit, max_frame), None);
}

#[test]
fn loop_end_uses_the_last_enabled_authored_rotation_or_position_key() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let pelvis = rig.pelvis;
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    document.end = 1800;
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(0.8) * rotations[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 75, &[pelvis], &[pelvis])
        .unwrap();

    assert_eq!(document.last_authored_keyframe(), Some(75));
    document.tracks.iter_mut().find(|track| track.bone == pelvis).unwrap().enabled = false;
    assert_eq!(document.last_authored_keyframe(), Some(30));
    document.tracks.iter_mut().find(|track| track.bone == head).unwrap().authored = false;
    assert_eq!(
        document.last_authored_keyframe(),
        None,
        "untouched baseline tracks do not shorten the loop"
    );
}

#[test]
fn upload_export_ends_at_the_last_authored_key_when_last_key_looping_is_selected() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document.range(0, 30 * 30);
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    let mut rotations = rig.local_rot.clone();
    rotations[head] = Quat::from_rotation_z(0.6) * rotations[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();

    let full = document.animation_for_export(false).unwrap();
    assert_eq!(full.duration, 30.0);
    assert_eq!(document.end, 900, "export preparation must not shorten the editable project");

    let last_key_loop = document.animation_for_export(true).unwrap();
    let expected_duration = 31.0 / 30.0;
    assert!((last_key_loop.duration - expected_duration).abs() < 0.0001);
    assert_eq!(last_key_loop.loop_out, last_key_loop.duration);
    assert!(last_key_loop.looping);
    let encoded = aurora_assets::import_animation::encode(&last_key_loop).unwrap();
    let uploaded = aurora_assets::anim::parse_animation_upload(&encoded).unwrap();
    assert!((uploaded.duration - expected_duration).abs() < 0.0001);
    assert_eq!(document.end, 900, "the editor timeline remains at its original length");
}

#[test]
fn playback_pause_resumes_fractional_time_and_restarts_from_clip_start_at_the_end() {
    let mut document = Document::new();
    document.range(10, 40);
    let mut editor = Editor {
        clip: State {
            active: true,
            document: Some(Arc::new(document)),
            frame: 15,
            cursor: 15.25,
            ..Default::default()
        },
        ..Default::default()
    };

    editor.studio_toggle_clip_playback();
    assert!(editor.clip.playing);
    assert_eq!(editor.clip.cursor, 15.25, "play starts from the selected sub-frame time");

    editor.clip.cursor = 17.5;
    editor.clip.frame = 18;
    editor.studio_toggle_clip_playback();
    assert!(!editor.clip.playing);
    assert_eq!(editor.clip.cursor, 17.5, "pause preserves sub-frame progress");
    editor.studio_toggle_clip_playback();
    assert!(editor.clip.playing);
    assert_eq!(editor.clip.cursor, 17.5, "resume continues from the same sub-frame time");

    editor.clip.playing = false;
    editor.clip.frame = 40;
    editor.clip.cursor = 39.75;
    editor.studio_toggle_clip_playback();
    assert!(editor.clip.playing);
    assert_eq!(editor.clip.frame, 40, "the rounded display may reach the end early");
    assert_eq!(editor.clip.cursor, 39.75, "playback must preserve the actual pre-end cursor");
    editor.studio_toggle_clip_playback();

    editor.clip.playing = false;
    editor.clip.frame = 40;
    editor.clip.cursor = 40.0;
    editor.studio_toggle_clip_playback();
    assert!(editor.clip.playing);
    assert_eq!(editor.clip.frame, 10);
    assert_eq!(editor.clip.cursor, 10.0, "a completed clip restarts at its trimmed start");
}

#[test]
fn edited_loop_metadata_keeps_a_valid_loop_window() {
    let mut document = Document::new();
    document.range(0, 30);
    document.meta.looping = true;
    document.meta.loop_in = 0.8;
    document.meta.loop_out = 0.4;
    document.timing();

    let animation = document.preview().expect("an empty preview is valid");
    let (start, end) = animation.loop_range().expect("looping needs a valid window");
    assert_eq!(start, 0.8);
    assert!(end > start);
    assert!(end - start + 1.0e-6 >= 1.0 / document.fps as f32);
}

#[test]
fn animation_file_reads_stop_at_the_configured_size_limit() {
    assert_eq!(files::read_bounded(&b"1234"[..], 4).unwrap(), b"1234");
    assert_eq!(
        files::read_bounded(&b"12345"[..], 4).unwrap_err(),
        aurora_i18n::text("anim-edit-file-too-large")
    );
}

#[test]
fn timeline_track_rows_are_reused_until_the_document_changes() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    for (at, angle) in [(0, 0.0), (30, 0.4)] {
        rotations[head] = Quat::from_rotation_z(angle) * rig.local_rot[head];
        document
            .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), at, &[head], &[])
            .unwrap();
    }
    let mut state = State {
        document: Some(Arc::new(document)),
        ..Default::default()
    };
    let first = state.timeline_tracks().unwrap();
    let second = state.timeline_tracks().unwrap();
    assert!(Arc::ptr_eq(&first, &second), "unchanged timeline rows reuse the same snapshot");
    assert_eq!(first[0].keys, [0, 30]);
    assert!(state.has_key(Some(head), 30));
    assert!(!state.has_key(None, 15));

    rotations[head] = Quat::from_rotation_z(-0.7) * rig.local_rot[head];
    Arc::make_mut(state.document.as_mut().unwrap())
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 60, &[head], &[])
        .unwrap();
    state.changed();
    let updated = state.timeline_tracks().unwrap();
    assert!(!Arc::ptr_eq(&first, &updated), "an edit invalidates the cached snapshot");
    assert_eq!(updated[0].keys, [0, 30, 60]);
    assert!(state.has_key(None, 60));
}

#[test]
fn binary_timeline_sampling_keeps_first_duplicate_key_and_uses_the_last_before_the_next_time() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    document.tracks[0].motion.rot_keys = vec![
        aurora_assets::anim::RotKey {
            time: 0.0,
            rotation: Quat::IDENTITY,
        },
        aurora_assets::anim::RotKey {
            time: 0.5,
            rotation: Quat::from_rotation_z(0.3),
        },
        aurora_assets::anim::RotKey {
            time: 0.5,
            rotation: Quat::from_rotation_z(0.6),
        },
        aurora_assets::anim::RotKey {
            time: 1.0,
            rotation: Quat::from_rotation_z(1.0),
        },
    ];

    let at_duplicate = document.tracks[0].rotation_at(0.5).unwrap();
    let after_duplicate = document.tracks[0].rotation_at(0.75).unwrap();
    assert!(at_duplicate.dot(Quat::from_rotation_z(0.3)).abs() > 0.9999);
    assert!(after_duplicate.dot(Quat::from_rotation_z(0.8)).abs() > 0.9999);
}

#[test]
fn custom_curve_parameters_persist_and_older_projects_keep_the_default_curve() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(0.8) * rotations[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    document.tracks[0].interpolation = document::Interpolation::Curve;
    document.tracks[0].curve = document::Curve { start: 0.12, end: 0.86 };

    let encoded = document.project_json().unwrap();
    let reopened = Document::from_project_json(&encoded, rig).unwrap();
    assert_eq!(reopened.tracks[0].interpolation, document::Interpolation::Curve);
    assert_eq!(reopened.tracks[0].curve, document::Curve { start: 0.12, end: 0.86 });

    let mut older: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    older["tracks"][0].as_object_mut().unwrap().remove("curve");
    older["tracks"][0].as_object_mut().unwrap().remove("authored");
    older.as_object_mut().unwrap().remove("auto_key_advance_seconds");
    let older = serde_json::to_vec(&older).unwrap();
    let reopened = Document::from_project_json(&older, rig).unwrap();
    assert_eq!(reopened.tracks[0].curve, document::Curve::default());
    assert_eq!(
        reopened.auto_key_advance_seconds, 0.5,
        "older projects use the new default advance step"
    );
    assert!(reopened.tracks[0].authored, "older projects keep their existing export behavior");
}

#[test]
fn fresh_draft_preview_keeps_the_rest_pose_but_export_includes_only_authored_tracks() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bones: Vec<_> = (0..rig.len()).collect();
    let mut document = Document::new();
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &bones, &[])
        .unwrap();
    for track in &mut document.tracks {
        track.authored = false;
    }
    let baseline_track_count = document.tracks.len();

    assert_eq!(
        document.preview().unwrap().joints.len(),
        baseline_track_count,
        "the live preview stays isolated at rest"
    );
    assert_eq!(
        document.animation().unwrap_err(),
        "anim-edit-no-bones",
        "an untouched baseline is not an animation track"
    );
    assert!(
        !document.has_exportable_tracks(),
        "a rest-pose preview cannot be uploaded as an animation"
    );

    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    rotations[head] = Quat::from_rotation_z(0.6) * rotations[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    let encoded = document.project_json().unwrap();
    let reopened = Document::from_project_json(&encoded, rig).unwrap();
    assert_eq!(
        reopened.preview().unwrap().joints.len(),
        baseline_track_count,
        "the project retains its rest-pose preview"
    );
    let exported = reopened.animation().unwrap();
    assert!(reopened.has_exportable_tracks());
    assert_eq!(exported.joints.len(), 1, "unchanged baseline bones are omitted from upload");
    assert_eq!(exported.joints[0].joint_name, rig.canonical_names[head]);
    assert!(
        reopened
            .tracks
            .iter()
            .filter(|track| track.bone != head)
            .all(|track| !track.authored)
    );
}

#[test]
fn whole_pose_keyframes_copy_paste_and_delete_across_bone_tracks() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let wrist = rig.names["mWristRight"];
    let mut rotations = rig.local_rot.clone();
    let mut doc = Document::new();
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head, wrist], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(0.5) * rotations[head];
    rotations[wrist] = Quat::from_rotation_x(-0.4) * rotations[wrist];
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head, wrist], &[])
        .unwrap();
    let copied = doc.copy_keyframe(30);
    assert_eq!(copied.len(), 2);
    doc.paste_keyframe(45, &copied);
    assert_eq!(doc.frames(None), vec![0, 30, 45]);
    assert_eq!(doc.copy_keyframe(45).len(), 2);
    doc.move_track_keyframe(head, 45, 60);
    doc.move_track_keyframe(wrist, 45, 60);
    assert_eq!(doc.frames(None), vec![0, 30, 60]);
    doc.remove_keyframe(60);
    assert_eq!(doc.frames(None), vec![0, 30]);
}

#[test]
fn dragging_a_timeline_key_moves_only_the_bone_track_that_was_grabbed() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let wrist = rig.names["mWristRight"];
    let mut rotations = rig.local_rot.clone();
    let mut doc = Document::new();
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head, wrist], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(0.5) * rotations[head];
    rotations[wrist] = Quat::from_rotation_x(-0.4) * rotations[wrist];
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head, wrist], &[])
        .unwrap();

    doc.move_track_keyframe(head, 30, 45);

    assert_eq!(doc.frames(Some(head)), vec![0, 45]);
    assert_eq!(doc.frames(Some(wrist)), vec![0, 30]);
    assert_eq!(doc.frames(None), vec![0, 30, 45]);
}

#[test]
fn moving_onto_an_occupied_frame_replaces_it_in_either_drag_direction() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut document = Document::new();
    for (frame_number, angle) in [(0, 0.0), (30, 0.4), (60, -0.8)] {
        rotations[head] = Quat::from_rotation_z(angle) * rig.local_rot[head];
        document
            .record(
                rig,
                &frame(&lib, &rotations, &rig.local_pos, &rig.scale),
                frame_number,
                &[head],
                &[],
            )
            .unwrap();
    }
    document.tracks[0].motion.pos_keys = [0.0, 1.0, 2.0]
        .into_iter()
        .enumerate()
        .map(|(frame, x)| aurora_assets::anim::PosKey {
            time: frame as f32,
            position: Vec3::new(x, 0.0, 0.0),
        })
        .collect();
    let middle = document.tracks[0].rotation_at(1.0).unwrap();
    let late = document.tracks[0].rotation_at(2.0).unwrap();

    let mut forward = document.clone();
    forward.move_track_keyframe(head, 30, 60);
    let track = forward.tracks.iter().find(|track| track.bone == head).unwrap();
    assert!(
        track.rotation_at(2.0).unwrap().dot(middle).abs() > 0.99999,
        "the dragged key replaces its target"
    );
    assert_eq!(track.position_at(2.0).unwrap(), Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(forward.frames(Some(head)), vec![0, 60]);

    let mut backward = document;
    backward.move_track_keyframe(head, 60, 30);
    let track = backward.tracks.iter().find(|track| track.bone == head).unwrap();
    assert!(
        track.rotation_at(1.0).unwrap().dot(late).abs() > 0.99999,
        "the result is independent of drag direction"
    );
    assert_eq!(track.position_at(1.0).unwrap(), Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(backward.frames(Some(head)), vec![0, 30]);
}

#[test]
fn deleting_a_timeline_key_from_one_bone_keeps_other_tracks_at_that_frame() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let wrist = rig.names["mWristRight"];
    let mut rotations = rig.local_rot.clone();
    let mut doc = Document::new();
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head, wrist], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(0.5) * rotations[head];
    rotations[wrist] = Quat::from_rotation_x(-0.4) * rotations[wrist];
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head, wrist], &[])
        .unwrap();

    doc.remove_track_keyframe(head, 30);

    assert_eq!(doc.frames(Some(head)), vec![0]);
    assert_eq!(doc.frames(Some(wrist)), vec![0, 30]);
    assert_eq!(doc.frames(None), vec![0, 30]);
}

#[test]
fn frame_rate_changes_preserve_timeline_seconds_and_imported_native_animations_become_editable_tracks() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut rotations = rig.local_rot.clone();
    let mut doc = Document::new();
    doc.range(0, 60);
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    rotations[head] = Quat::from_rotation_z(0.7) * rotations[head];
    doc.record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    doc.set_fps(60);
    assert_eq!(doc.frames(Some(head)), vec![0, 60]);
    assert_eq!(doc.tracks[0].motion.rot_keys[1].time, 1.0);
    assert_eq!(doc.meta.duration, 2.0);

    doc.tracks[0].interpolation = document::Interpolation::Linear;
    doc.auto_keyframe = true;
    doc.auto_key_advance_seconds = 0.3;
    let project = doc.project_json().unwrap();
    let reopened = Document::from_project_json(&project, rig).unwrap();
    assert_eq!(reopened.fps, 60);
    assert!(reopened.auto_keyframe);
    assert_eq!(reopened.auto_key_advance_seconds, 0.3);
    assert_eq!(reopened.frames(Some(head)), vec![0, 60]);
    assert_eq!(reopened.tracks[0].interpolation, document::Interpolation::Linear);
    assert!(Document::from_project_json(b"{invalid", rig).is_err());
    let encoded = aurora_assets::import_animation::encode(&doc.animation().unwrap()).unwrap();
    let imported = aurora_assets::anim::parse_animation_upload(&encoded).unwrap();
    let editable = Document::from_animation(imported, rig).unwrap();
    assert_eq!(editable.tracks.len(), 1);
    assert_eq!(editable.tracks[0].bone, head);
    assert_eq!(editable.tracks[0].interpolation, document::Interpolation::Linear);
    assert_eq!(editable.frames(Some(head)), vec![0, 30, 60]);
}

#[test]
fn zero_duration_native_pose_imports_as_one_editable_frame() {
    let lib = AvatarLibrary::load();
    let rotation = Quat::from_rotation_z(0.4);
    let animation = aurora_assets::Animation {
        base_priority: 3,
        duration: 0.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 0.0,
        looping: false,
        ends_after: None,
        ease_in: 0.0,
        ease_out: 0.0,
        hand_pose: 0,
        joints: vec![aurora_assets::anim::JointMotion {
            joint_name: "mHead".into(),
            priority: 3,
            rot_keys: vec![aurora_assets::anim::RotKey { time: 0.0, rotation }],
            pos_keys: Vec::new(),
        }],
    };

    let document = Document::from_animation(animation, &lib.rig).expect("static native pose is editable");
    assert_eq!((document.start, document.end), (0, 1));
    assert!((document.meta.duration - 1.0 / FPS).abs() < 0.0001);
    assert!(document.tracks[0].rotation_at(0.0).unwrap().dot(rotation).abs() > 0.999);
    assert!((document.animation().unwrap().duration - 1.0 / FPS).abs() < 0.0001);
}

#[test]
fn native_animation_import_rejects_unknown_bones_instead_of_silently_dropping_them() {
    let lib = AvatarLibrary::load();
    let mut document = Document::new();
    let head = lib.rig.names["mHead"];
    document
        .record(
            &lib.rig,
            &frame(&lib, &lib.rig.local_rot, &lib.rig.local_pos, &lib.rig.scale),
            0,
            &[head],
            &[],
        )
        .unwrap();
    let mut animation = document.animation().unwrap();
    animation.joints.push(aurora_assets::anim::JointMotion {
        joint_name: "unsupported_custom_joint".into(),
        priority: 3,
        rot_keys: vec![aurora_assets::anim::RotKey {
            time: 0.0,
            rotation: Quat::IDENTITY,
        }],
        pos_keys: Vec::new(),
    });

    assert!(matches!(
        Document::from_animation(animation, &lib.rig),
        Err("anim-edit-unsupported-bones")
    ));
}

#[test]
fn project_import_rejects_unknown_bones_instead_of_silently_dropping_tracks() {
    let lib = AvatarLibrary::load();
    let mut document = Document::new();
    let head = lib.rig.names["mHead"];
    document
        .record(
            &lib.rig,
            &frame(&lib, &lib.rig.local_rot, &lib.rig.local_pos, &lib.rig.scale),
            0,
            &[head],
            &[],
        )
        .unwrap();
    let mut project: serde_json::Value = serde_json::from_slice(&document.project_json().unwrap()).unwrap();
    project["tracks"][0]["joint_name"] = serde_json::Value::String("unsupported_custom_joint".into());
    let bytes = serde_json::to_vec(&project).unwrap();

    assert!(matches!(
        Document::from_project_json(&bytes, &lib.rig),
        Err("anim-edit-unsupported-bones")
    ));
}

#[test]
fn project_import_rejects_duplicate_tracks_for_the_same_bone() {
    let lib = AvatarLibrary::load();
    let mut document = Document::new();
    let head = lib.rig.names["mHead"];
    document
        .record(
            &lib.rig,
            &frame(&lib, &lib.rig.local_rot, &lib.rig.local_pos, &lib.rig.scale),
            0,
            &[head],
            &[],
        )
        .unwrap();
    let mut project: serde_json::Value = serde_json::from_slice(&document.project_json().unwrap()).unwrap();
    let duplicate = project["tracks"][0].clone();
    project["tracks"].as_array_mut().unwrap().push(duplicate);
    let bytes = serde_json::to_vec(&project).unwrap();

    assert!(matches!(Document::from_project_json(&bytes, &lib.rig), Err("anim-edit-invalid")));
}

#[test]
fn dense_sixty_second_projects_round_trip_beyond_the_old_sixteen_mib_limit() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let mut document = Document::new();
    document.fps = 60;
    document.range(0, 3600);
    for bone in 0..20 {
        let keys = (0..16_384)
            .map(|index| {
                let time = index as f32 * (60.0 / 16_383.0);
                (time, Quat::from_rotation_z((index % 360) as f32 * 0.01))
            })
            .collect::<Vec<_>>();
        document.tracks.push(document::Track {
            bone,
            enabled: true,
            authored: true,
            motion: aurora_assets::anim::JointMotion {
                joint_name: rig.canonical_names[bone].clone(),
                priority: 3,
                rot_keys: keys
                    .iter()
                    .map(|(time, rotation)| aurora_assets::anim::RotKey {
                        time: *time,
                        rotation: *rotation,
                    })
                    .collect(),
                pos_keys: keys
                    .iter()
                    .map(|(time, _)| aurora_assets::anim::PosKey {
                        time: *time,
                        position: Vec3::ZERO,
                    })
                    .collect(),
            },
            interpolation: document::Interpolation::Linear,
            curve: document::Curve::default(),
            base_position: rig.local_pos[bone],
        });
    }

    let bytes = document.project_json().expect("dense project fits the supported file limit");
    assert!(bytes.len() > 16 * 1024 * 1024, "the fixture exceeds the previous limit");
    assert!(bytes.len() <= document::MAX_PROJECT_BYTES);
    let reopened = Document::from_project_json(&bytes, rig).expect("saved project opens again");
    assert_eq!(reopened.tracks.len(), 20);
    assert_eq!(reopened.tracks[0].motion.rot_keys.len(), 16_384);
    assert_eq!(reopened.tracks[0].motion.pos_keys.len(), 16_384);
    assert_eq!(reopened.meta.duration, 60.0);
}

#[test]
fn project_save_stops_serializing_at_its_size_limit() {
    let large = "x".repeat(1_024);
    assert!(matches!(
        document::serialize_project(&large, 32),
        Err("anim-edit-project-too-large")
    ));
    assert_eq!(document::serialize_project(&"small", 32).unwrap(), br#""small""#);
}

#[test]
fn bvh_file_import_creates_editable_tracks_for_the_native_rig() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bvh = br#"HIERARCHY
ROOT hip {
  OFFSET 0 0 0
  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation
  JOINT mHead {
    OFFSET 0 0 1
    CHANNELS 3 Zrotation Xrotation Yrotation
    End Site { OFFSET 0 1 0 }
  }
}
MOTION
Frames: 3
Frame Time: 0.5
0 0 0 0 0 0 0 0 0
0 0 0 0 0 0 0 0 0
0 0 0 0 0 0 20 0 0"#;
    let document = files::load_document(bvh, "bvh", rig, lib.skeleton.as_ref()).expect("BVH tracks");
    assert_eq!(document.meta.duration, 0.5);
    let head = rig.names["mHead"];
    assert_eq!(document.tracks.len(), 1);
    assert_eq!(document.tracks[0].bone, head);
    assert_eq!(document.frames(Some(head)), vec![0, 15]);
    assert!(files::load_document(bvh, "bvh", rig, None).is_err());
}

#[test]
fn animation_editor_bvh_import_rejects_unmapped_joint_tracks() {
    let lib = AvatarLibrary::load();
    let bvh = br#"HIERARCHY
ROOT hip {
  OFFSET 0 0 0
  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation
  JOINT mHead {
    OFFSET 0 0 1
    CHANNELS 3 Zrotation Xrotation Yrotation
  }
  JOINT customJoint {
    OFFSET 0 0 1
    CHANNELS 3 Zrotation Xrotation Yrotation
  }
}
MOTION
Frames: 2
Frame Time: 0.5
0 0 0 0 0 0 0 0 0 0 0 0
0 0 0 0 0 0 10 0 0 10 0 0"#;

    let error = match files::load_document(bvh, "bvh", &lib.rig, lib.skeleton.as_ref()) {
        Ok(_) => panic!("the editor must reject unmapped joint tracks"),
        Err(error) => error,
    };
    assert!(error.contains("unknown to the avatar rig"));
}

#[test]
fn returning_from_upload_preparation_resumes_the_same_studio_draft() {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![demo::action_avatar(false)],
    });
    let rig = &lib.rig;
    world.avatar_poses.insert(
        world.agent_id,
        rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)
            .iter()
            .zip(&rig.default_world_inv)
            .map(|(pose, inverse)| *pose * *inverse)
            .collect(),
    );
    let head = rig.names["mHead"];
    let mut editor = Editor::default();
    let mut draft = Document::new();
    let rest = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);
    draft.record(rig, &rest, 0, &[head], &[]).unwrap();
    let mut rotations = rig.local_rot.clone();
    rotations[head] = Quat::from_rotation_z(0.5) * rotations[head];
    draft
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[head], &[])
        .unwrap();
    editor.clip.document = Some(Arc::new(draft));
    editor.clip.active = true;
    editor.open = true;
    editor.clip.from_avatar_studio = true;
    let original_frames = editor.clip.document.as_ref().unwrap().frames(Some(head));
    editor.clip.prepared.push(Prepared {
        name: "Wave".into(),
        bytes: Arc::new(Vec::new()),
        animation: Arc::new(editor.clip.document.as_ref().unwrap().animation().unwrap()),
    });

    assert_eq!(editor.take_clip_imports().len(), 1);
    assert!(!editor.clip.active);
    assert!(editor.animation_editor_return_pending());
    editor.poser.active = true;
    assert!(editor.resume_after_upload(&world));

    assert!(editor.clip.active);
    assert!(
        !editor.poser.active,
        "the animation editor uses its own posing and key capture path"
    );
    assert_eq!(editor.clip.document.as_ref().unwrap().frames(Some(head)), original_frames);
    assert!(
        editor.clip.history.is_empty(),
        "reopening does not hide the draft behind a fresh one"
    );
}

#[test]
fn sixty_second_timeline_limit_is_preserved_at_every_supported_fps() {
    let mut document = Document::new();
    document.range(0, MAX_FRAME);
    assert_eq!(document.end, 1800);
    assert_eq!(document.meta.duration, 60.0);

    document.set_fps(60);
    assert_eq!(document.end, 3600);
    assert_eq!(document.max_frame(), 3600);
    assert_eq!(document.meta.duration, 60.0);

    document.range(0, MAX_FRAME);
    assert_eq!(document.end, 3600);
    assert_eq!(document.meta.duration, 60.0);

    document.set_fps(1);
    assert_eq!(document.end, 60);
    assert_eq!(document.max_frame(), 60);
    assert_eq!(document.meta.duration, 60.0);
}

#[test]
fn bento_finger_joint_names_survive_animation_import_and_export() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let finger = rig.names["mHandIndex3Left"];
    let head = rig.names["mHead"];
    let animation = Animation {
        base_priority: 3,
        duration: 1.0,
        emote_name: "Finger test".into(),
        loop_in: 0.0,
        loop_out: 1.0,
        looping: true,
        ends_after: None,
        ease_in: 0.0,
        ease_out: 0.0,
        hand_pose: 0,
        joints: vec![aurora_assets::anim::JointMotion {
            joint_name: "mHandIndex3Left".into(),
            priority: 3,
            rot_keys: vec![aurora_assets::anim::RotKey {
                time: 0.0,
                rotation: rig.local_rot[finger],
            }],
            pos_keys: Vec::new(),
        }],
    };
    let mut document = Document::from_animation(animation, rig).unwrap();
    assert_eq!(document.tracks[0].bone, finger);

    let mut rotations = rig.local_rot.clone();
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    rotations[finger] = Quat::from_rotation_x(0.35) * rotations[finger];
    rotations[head] = Quat::from_rotation_z(0.45) * rotations[head];
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &[finger, head], &[])
        .unwrap();

    let bytes = aurora_assets::import_animation::encode(&document.animation().unwrap()).unwrap();
    let exported = aurora_assets::anim::parse_animation_upload(&bytes).unwrap();
    let finger_track = exported
        .joints
        .iter()
        .find(|track| track.joint_name == "mHandIndex3Left")
        .expect("Bento finger track");
    let head_track = exported
        .joints
        .iter()
        .find(|track| track.joint_name == "mHead")
        .expect("head track");
    assert_eq!(finger_track.rot_keys.len(), 2);
    assert_eq!(head_track.rot_keys.len(), 2);
    assert!(finger_track.rot_keys[1].rotation.dot(rotations[finger]).abs() > 0.999);
    assert!(head_track.rot_keys[1].rotation.dot(rotations[head]).abs() > 0.999);
}

#[test]
fn avatar_studio_wave_fixture_survives_project_save_and_anim_export() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bones = wave_demo_bones(rig).expect("the avatar rig includes the wave's Bento joints");
    let names: Vec<_> = bones.iter().map(|bone| rig.canonical_names[*bone].as_str()).collect();
    assert_eq!(
        names,
        [
            "mShoulderRight",
            "mElbowRight",
            "mWristRight",
            "mHandIndex1Right",
            "mHandMiddle1Right",
            "mHandRing1Right",
            "mHandPinky1Right",
            "mHandThumb1Right",
        ]
    );

    let mut document = Document::new();
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &bones, &[])
        .unwrap();
    let mut rotations = rig.local_rot.clone();
    for (bone, axis, angle) in [
        (bones[0], Vec3::X, 1.15),
        (bones[1], Vec3::X, -0.25),
        (bones[2], Vec3::Z, 0.65),
        (bones[3], Vec3::X, -0.25),
        (bones[4], Vec3::X, -0.225),
        (bones[5], Vec3::X, -0.2),
        (bones[6], Vec3::X, -0.175),
        (bones[7], Vec3::Y, 0.1125),
    ] {
        rotations[bone] = Quat::from_axis_angle(axis, angle) * rotations[bone];
    }
    document
        .record(rig, &frame(&lib, &rotations, &rig.local_pos, &rig.scale), 30, &bones, &[])
        .unwrap();

    let project = document.project_json().unwrap();
    document = Document::from_project_json(&project, rig).unwrap();
    assert_eq!(
        document.tracks.len(),
        bones.len(),
        "the saved project retains every authored bone track"
    );
    let exported =
        aurora_assets::anim::parse_animation_upload(&aurora_assets::import_animation::encode(&document.animation().unwrap()).unwrap())
            .unwrap();
    for bone in bones {
        let track = exported
            .joints
            .iter()
            .find(|track| track.joint_name == rig.canonical_names[bone])
            .expect("every wave joint is exported");
        assert_eq!(
            track.rot_keys.len(),
            3,
            "{} has both wave poses and the clip boundary",
            track.joint_name
        );
        assert!(
            track.rot_keys[1].rotation.dot(rotations[bone]).abs() > 0.999,
            "{} keeps its authored pose",
            track.joint_name
        );
        let editor_track = document.tracks.iter().find(|track| track.bone == bone).unwrap();
        for time in [0.25, 0.5, 0.75] {
            let preview = editor_track.rotation_at(time).unwrap();
            let uploaded = track.rotation_at(time).unwrap();
            assert!(
                preview.dot(uploaded).abs() > 0.99999,
                "{} matches the saved editor preview at {time}s",
                track.joint_name
            );
        }
    }
}

#[test]
fn encoded_pose_reapplies_captured_hierarchy_and_pelvis_to_the_rest_rig() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bones = [
        rig.pelvis,
        rig.names["mTorso"],
        rig.names["mShoulderRight"],
        rig.names["mElbowRight"],
        rig.names["mWristRight"],
        rig.names["mHandIndex1Right"],
        rig.names["mHead"],
    ];
    let mut document = Document::new();
    document.range(0, 30);
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &bones, &[])
        .unwrap();

    let mut rotations = rig.local_rot.clone();
    for (bone, axis, angle) in [
        (bones[0], Vec3::Z, 0.14),
        (bones[1], Vec3::X, 0.2),
        (bones[2], Vec3::Y, 0.45),
        (bones[3], Vec3::X, -0.3),
        (bones[4], Vec3::Z, 0.35),
        (bones[5], Vec3::Y, 0.22),
        (bones[6], Vec3::X, -0.18),
    ] {
        rotations[bone] = Quat::from_axis_angle(axis, angle) * rotations[bone];
    }
    let mut positions = rig.local_pos.clone();
    positions[rig.pelvis] += Vec3::new(0.02, -0.03, 0.08);
    document
        .record(rig, &frame(&lib, &rotations, &positions, &rig.scale), 30, &bones, &[])
        .unwrap();

    let bytes = aurora_assets::import_animation::encode(&document.animation().unwrap()).unwrap();
    let animation = aurora_assets::anim::parse_animation_upload(&bytes).unwrap();
    let sample = crate::scene::ik::animations::Sample {
        animation: Arc::new(crate::scene::anim::BoundAnim::bind(animation, rig)),
        frame: 30,
        time_seconds: Some(1.0),
    };
    let mut pose = ik::LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    };
    crate::scene::ik::animations::apply(rig, &mut pose, Vec3::ZERO, &[sample]);

    for bone in bones {
        assert!(
            pose.rotations[bone].dot(rotations[bone]).abs() > 0.9999,
            "{} rotation survives capture, native encoding, and avatar playback",
            rig.canonical_names[bone]
        );
    }
    assert!(pose.positions[rig.pelvis].abs_diff_eq(positions[rig.pelvis], 0.0002));
}

#[test]
fn exported_animation_matches_editor_preview_between_keyframes() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let bones = [head, rig.pelvis];
    for interpolation in [
        document::Interpolation::Linear,
        document::Interpolation::Smooth,
        document::Interpolation::Stepped,
        document::Interpolation::Curve,
    ] {
        let mut document = Document::new();
        document.range(0, 45);
        document
            .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &bones, &[])
            .unwrap();
        let mut rotations = rig.local_rot.clone();
        rotations[head] = Quat::from_rotation_z(0.5) * rotations[head];
        let mut positions = rig.local_pos.clone();
        positions[rig.pelvis].z += 0.2;
        document
            .record(rig, &frame(&lib, &rotations, &positions, &rig.scale), 15, &bones, &[rig.pelvis])
            .unwrap();
        rotations[head] = Quat::from_rotation_z(1.2) * rig.local_rot[head];
        positions[rig.pelvis] = rig.local_pos[rig.pelvis];
        document
            .record(rig, &frame(&lib, &rotations, &positions, &rig.scale), 45, &bones, &[rig.pelvis])
            .unwrap();
        for track in &mut document.tracks {
            track.interpolation = interpolation;
            track.curve = document::Curve { start: 0.25, end: 0.8 };
        }
        let expected: Vec<_> = document.tracks.clone();
        let exported = aurora_assets::import_animation::encode(&document.animation().unwrap()).unwrap();
        let animation = aurora_assets::anim::parse_animation_upload(&exported).unwrap();

        for time in [0.0, 0.05, 0.25, 0.49, 0.5, 0.75, 1.0, 1.49, 1.5] {
            let preview = expected.iter().find(|track| track.bone == head).unwrap().rotation_at(time).unwrap();
            let uploaded = animation
                .joints
                .iter()
                .find(|joint| joint.joint_name == "mHead")
                .unwrap()
                .rotation_at(time)
                .unwrap();
            let difference = 2.0 * preview.dot(uploaded).abs().clamp(0.0, 1.0).acos();
            assert!(difference < 0.002, "{interpolation:?} differs by {difference} radians at {time}s");

            let preview = expected
                .iter()
                .find(|track| track.bone == rig.pelvis)
                .unwrap()
                .position_at(time)
                .unwrap();
            let uploaded = animation
                .joints
                .iter()
                .find(|joint| joint.joint_name == "mPelvis")
                .unwrap()
                .position_at(time)
                .unwrap();
            assert!(
                preview.distance(uploaded) < 0.001,
                "{interpolation:?} pelvis motion differs by {} metres at {time}s",
                preview.distance(uploaded)
            );
        }
    }
}

#[test]
fn smooth_export_matches_preview_between_low_fps_timeline_keys() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let bones = [head, rig.pelvis];
    for interpolation in [document::Interpolation::Smooth, document::Interpolation::Curve] {
        let mut document = Document::new();
        document.set_fps(1);
        document.range(0, 2);
        document
            .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &bones, &[])
            .unwrap();
        let mut rotations = rig.local_rot.clone();
        rotations[head] = Quat::from_rotation_z(0.8) * rotations[head];
        let mut positions = rig.local_pos.clone();
        positions[rig.pelvis].z += 0.15;
        document
            .record(rig, &frame(&lib, &rotations, &positions, &rig.scale), 2, &bones, &[rig.pelvis])
            .unwrap();
        for track in &mut document.tracks {
            track.interpolation = interpolation;
            track.curve = document::Curve { start: 0.2, end: 0.75 };
            if track.bone == head {
                track.motion.rot_keys.insert(
                    1,
                    aurora_assets::anim::RotKey {
                        time: 0.7,
                        rotation: Quat::from_rotation_z(0.3) * rig.local_rot[head],
                    },
                );
            } else if track.bone == rig.pelvis {
                track.motion.pos_keys.insert(
                    1,
                    aurora_assets::anim::PosKey {
                        time: 0.7,
                        position: Vec3::new(0.0, 0.0, 0.05),
                    },
                );
            }
        }
        let expected = document.tracks.clone();
        let bytes = aurora_assets::import_animation::encode(&document.animation().unwrap()).unwrap();
        let animation = aurora_assets::anim::parse_animation_upload(&bytes).unwrap();

        for time in [0.1, 0.25, 0.5, 0.65, 0.7, 0.71, 0.75, 1.0, 1.25, 1.5, 1.75, 1.9] {
            let preview = expected.iter().find(|track| track.bone == head).unwrap().rotation_at(time).unwrap();
            let uploaded = animation
                .joints
                .iter()
                .find(|joint| joint.joint_name == "mHead")
                .unwrap()
                .rotation_at(time)
                .unwrap();
            let difference = 2.0 * preview.dot(uploaded).abs().clamp(0.0, 1.0).acos();
            assert!(
                difference < 0.002,
                "{interpolation:?} rotation differs by {difference} radians at {time}s"
            );

            let preview = expected
                .iter()
                .find(|track| track.bone == rig.pelvis)
                .unwrap()
                .position_at(time)
                .unwrap();
            let uploaded = animation
                .joints
                .iter()
                .find(|joint| joint.joint_name == "mPelvis")
                .unwrap()
                .position_at(time)
                .unwrap();
            assert!(
                preview.distance(uploaded) < 0.001,
                "{interpolation:?} pelvis motion differs by {} metres at {time}s",
                preview.distance(uploaded)
            );
        }
    }
}

#[test]
fn eased_export_keeps_single_pose_tracks_compact() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let pelvis = rig.pelvis;
    let bones = [head, pelvis];
    let mut document = Document::new();
    document.range(0, 60);
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &bones, &[pelvis])
        .unwrap();
    for track in &mut document.tracks {
        track.interpolation = document::Interpolation::Curve;
    }

    let animation = document.animation().unwrap();
    assert!(animation.joints.iter().all(|joint| joint.rot_keys.len() == 2));
    assert!(animation.joints.iter().all(|joint| joint.pos_keys.len() <= 2));
}

#[test]
fn eased_export_reserves_the_native_key_limit_for_dense_authored_tracks() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let head = rig.names["mHead"];
    let mut document = Document::new();
    document.set_fps(60);
    document.range(0, 60 * 60);
    document
        .record(rig, &frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale), 0, &[head], &[])
        .unwrap();
    let track = document.tracks.iter_mut().find(|track| track.bone == head).unwrap();
    track.interpolation = document::Interpolation::Curve;
    track.motion.rot_keys = (0..document::MAX_ANIM_KEYS)
        .map(|index| aurora_assets::anim::RotKey {
            time: index as f32 * 60.0 / (document::MAX_ANIM_KEYS - 1) as f32,
            rotation: rig.local_rot[head],
        })
        .collect();

    let animation = document.animation().unwrap();
    assert_eq!(animation.joints[0].rot_keys.len(), document::MAX_ANIM_KEYS);
}

#[test]
fn walking_cycle_round_trips_pelvis_bob_and_opposing_hip_swings() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let pelvis = rig.pelvis;
    let left_hip = rig.names["mHipLeft"];
    let right_hip = rig.names["mHipRight"];
    let bones = [pelvis, left_hip, right_hip];
    let mut document = Document::new();
    document.range(0, 30);

    let mut rotations = rig.local_rot.clone();
    let mut positions = rig.local_pos.clone();
    document
        .record(rig, &frame(&lib, &rotations, &positions, &rig.scale), 0, &bones, &[])
        .unwrap();

    rotations[left_hip] = Quat::from_rotation_x(0.3) * rig.local_rot[left_hip];
    rotations[right_hip] = Quat::from_rotation_x(-0.3) * rig.local_rot[right_hip];
    positions[pelvis].z += 0.05;
    document
        .record(rig, &frame(&lib, &rotations, &positions, &rig.scale), 15, &bones, &[])
        .unwrap();

    rotations[left_hip] = rig.local_rot[left_hip];
    rotations[right_hip] = rig.local_rot[right_hip];
    positions[pelvis] = rig.local_pos[pelvis];
    document
        .record(rig, &frame(&lib, &rotations, &positions, &rig.scale), 30, &bones, &[])
        .unwrap();

    let bytes = aurora_assets::import_animation::encode(&document.animation().unwrap()).unwrap();
    let exported = aurora_assets::anim::parse_animation_upload(&bytes).unwrap();
    assert!(exported.looping);
    assert_eq!(exported.duration, 1.0);
    let track = |name: &str| {
        exported
            .joints
            .iter()
            .find(|joint| joint.joint_name == name)
            .expect("walking track")
    };
    let pelvis_track = track("mPelvis");
    let left_track = track("mHipLeft");
    let right_track = track("mHipRight");
    assert_eq!(pelvis_track.pos_keys.len(), 3);
    assert_eq!(left_track.rot_keys.len(), 3);
    assert_eq!(right_track.rot_keys.len(), 3);
    assert!(pelvis_track.pos_keys[1].position.abs_diff_eq(Vec3::new(0.0, 0.0, 0.05), 0.0001));
    assert!(left_track.rot_keys[1].rotation.dot(rotations[left_hip]).abs() < 0.999);
    assert!(right_track.rot_keys[1].rotation.dot(rotations[right_hip]).abs() < 0.999);
}

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
fn keying_a_manually_rotated_bone_keeps_the_pose_after_preview_rebuilds() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = lib.rig.clone();
    let world = World::new(lib.clone());
    let original = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);
    let head = rig.names["mHead"];
    let mut editor = Editor::default();
    editor.toggle();
    editor.clip.active = true;
    editor.clip.rig = Some(rig.clone());
    editor.clip.document = Some(Arc::new(Document::new()));
    editor.mode = Mode::Direct;
    editor.select(head, &rig, &original);
    editor.orient = true;
    editor.degrees.z += 35.0;
    let expected = editor.rotation();
    editor.commit_manipulation(&rig, &original, true);
    assert!(editor.authoring.owns(head), "manual bone edits are retained as authored channels");

    editor.clip.capture = Some(Capture::Bone(head));
    editor.capture_clip(&world, &original);
    let keyed = editor
        .clip
        .document
        .as_ref()
        .unwrap()
        .tracks
        .iter()
        .find(|track| track.bone == head)
        .unwrap()
        .motion
        .rot_keys
        .iter()
        .find(|key| key.time == 0.0)
        .unwrap()
        .rotation;
    assert!(keyed.dot(expected).abs() > 0.9999, "the key records the edited rotation");

    let ctx = egui::Context::default();
    let (jobs, _) = Jobs::new();
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    editor.tick_clip(&jobs, &ctx, &world);
    editor.tick_clip(&jobs, &ctx, &world);
    assert!(!editor.authoring.owns(head), "the rebuilt preview replaces its temporary pose edit");
    let keyed_after_rebuild = editor
        .clip
        .document
        .as_ref()
        .unwrap()
        .tracks
        .iter()
        .find(|track| track.bone == head)
        .unwrap()
        .motion
        .rot_keys
        .iter()
        .find(|key| key.time == 0.0)
        .unwrap()
        .rotation;
    assert!(
        keyed_after_rebuild.dot(expected).abs() > 0.9999,
        "the preview retains the pose after the temporary edit is cleared"
    );
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
    editor.tick_clip(&jobs, &ctx, &world);
    editor.tick_clip(&jobs, &ctx, &world);
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
    editor.clip.capture = Some(Capture::Bone(wrist.bone));
    editor.capture_clip(&world, &original);
    assert_eq!(editor.goals.len(), 2, "hold the pose while the new preview is building");
    let wrist_key = editor
        .clip
        .document
        .as_ref()
        .unwrap()
        .tracks
        .iter()
        .find(|track| track.bone == wrist.bone)
        .unwrap()
        .motion
        .rot_keys
        .iter()
        .find(|key| key.time == 1.0)
        .unwrap();
    assert!(wrist_key.rotation.dot(solved.rotations[wrist.bone]).abs() > 0.9999);
    for bone in &path {
        assert!(editor.clip.document.as_ref().unwrap().frames(Some(*bone)).contains(&30));
    }
    editor.tick_clip(&jobs, &ctx, &world);
    editor.tick_clip(&jobs, &ctx, &world);
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
    editor.tick_clip(&jobs, &ctx, &world);
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

#[test]
fn creation_records_the_rest_skeleton_never_the_animation_playing_on_the_avatar() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = lib.rig.clone();
    let world = World::new(lib.clone());
    let elbow = rig.names["mElbowLeft"];
    let mut rotations = rig.local_rot.clone();
    rotations[elbow] = Quat::from_euler(EulerRot::ZYX, 0.9, -0.4, 0.7);
    let mut positions = rig.local_pos.clone();
    positions[rig.pelvis] += Vec3::new(0.1, -0.2, 0.3);
    let playing = frame(&lib, &rotations, &positions, &rig.scale);
    let at_rest = |editor: &Editor, time: f32| {
        let document = editor.clip.document.as_ref().unwrap();
        let track = document.tracks.iter().find(|t| t.bone == elbow).unwrap();
        track.motion.rotation_at(time).unwrap().dot(rig.local_rot[elbow]).abs() > 0.9999
    };
    let mut editor = Editor::default();
    editor.toggle();
    editor.new_clip(&world, &playing);
    assert!(at_rest(&editor, 0.0), "a new animation");
    let document = editor.clip.document.as_ref().unwrap();
    let pelvis = document.tracks.iter().find(|t| t.bone == rig.pelvis).unwrap();
    assert!(pelvis.motion.position_at(0.0).unwrap().abs_diff_eq(Vec3::ZERO, 1e-4));
    editor.leave_clip();
    editor.clip.frame = 30;
    editor.capture_ik_clip(&world, &playing);
    assert!(at_rest(&editor, 1.0), "the IK capture");
    editor.leave_clip();
    editor.clip.document = None;
    editor.enter_clip(&world, &playing);
    assert!(at_rest(&editor, 0.0), "entering without a draft");

    // while the first preview builds the avatar still shows its source, and
    // a key cannot read it
    assert!(editor.clip.waiting_source());
    assert!(!editor.request().suppress_live);
    editor.clip.frame = 45;
    editor.clip.capture = Some(Capture::Pose);
    editor.capture_clip(&world, &playing);
    assert_eq!(editor.clip.document.as_ref().unwrap().frames(Some(elbow)), vec![0]);
    let ctx = egui::Context::default();
    let (jobs, _) = Jobs::new();
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    editor.tick_clip(&jobs, &ctx, &world);
    editor.tick_clip(&jobs, &ctx, &world);
    assert!(!editor.clip.waiting_source());
    assert!(
        editor.request().suppress_live,
        "disabled tracks show the rest skeleton, not the animation"
    );
}

#[test]
fn creation_bakes_the_contacts_the_user_set_on_the_rest_skeleton() {
    let lib = Arc::new(AvatarLibrary::load());
    let rig = lib.rig.clone();
    let world = World::new(lib.clone());
    let wrist = rig.names["mWristLeft"];
    let original = frame(&lib, &rig.local_rot, &rig.local_pos, &rig.scale);
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(wrist, &rig, &original);
    editor.position += Vec3::new(0.04, 0.02, -0.01);
    editor.commit();
    let goal = editor.request().goals[0].clone();
    let mut solved = ik::LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    };
    assert!(ik::apply_goal(&rig, &mut solved, Vec3::ZERO, &rig.scale, &goal));
    let mut rotations = rig.local_rot.clone();
    rotations[rig.names["mHead"]] = Quat::from_rotation_x(0.8);
    editor.new_clip(&world, &frame(&lib, &rotations, &rig.local_pos, &rig.scale));
    let document = editor.clip.document.as_ref().unwrap();
    let key = |bone: usize| {
        let track = document.tracks.iter().find(|t| t.bone == bone).unwrap();
        track.motion.rotation_at(0.0).unwrap()
    };
    let elbow = rig.names["mElbowLeft"];
    assert!(
        solved.rotations[elbow].dot(rig.local_rot[elbow]).abs() < 0.99999,
        "the contact moves the arm"
    );
    assert!(key(elbow).dot(solved.rotations[elbow]).abs() > 0.9999, "and is in the key");
    let head = rig.names["mHead"];
    assert!(key(head).dot(rig.local_rot[head]).abs() > 0.9999, "the played head is not");
}
