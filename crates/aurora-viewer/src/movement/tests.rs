use super::*;

#[test]
fn passage_feedback_requires_the_current_action_and_an_actual_terminal_report() {
    use aurora_net::bridge::PassageReport;
    let now = Instant::now();
    let mut feedback = passage::Feedback::default();
    let mut report = PassageReport {
        action: 123,
        state: 2,
        progress_cm: 10,
        remaining_cm: 140,
        speed_cm: 50,
        pushes: 2,
    };
    feedback.start(456, now);
    assert!(feedback.update(Some(report), now).is_none(), "ignore a previous action");
    report.action = 456;
    let started = feedback.update(Some(report), now).expect("motor started");
    assert!(started.contains("0.10") && started.contains("1.40"));
    assert!(
        feedback.update(Some(report), now).is_none(),
        "unchanged reports produce no new message"
    );
    report.state = 4;
    assert_ne!(feedback.update(Some(report), now).expect("target not reached"), started);
    report.state = 2;
    assert!(feedback.update(Some(report), now).is_none(), "a completed action cannot regress");
    feedback.start(789, now);
    assert!(feedback.update(Some(report), now).is_none());
    assert!(
        feedback.update(None, now + Duration::from_secs(5)).is_some(),
        "missing final result is explicit"
    );
    assert!(feedback.update(None, now + Duration::from_secs(6)).is_none());
}

#[test]
fn obstacle_heading_follows_the_camera_instead_of_the_avatar_and_discards_pitch() {
    for pitch in [-1.0_f32, 0.0, 1.0] {
        let camera_forward = Vec3::new(0.0, pitch.cos(), pitch.sin());
        assert!(horizontal_heading(camera_forward, Vec3::X).abs_diff_eq(Vec3::Y, 1.0e-6));
        assert!(horizontal_heading(-camera_forward, Vec3::X).abs_diff_eq(-Vec3::Y, 1.0e-6));
    }
}

#[test]
fn a_vertical_camera_keeps_a_finite_horizontal_body_heading_for_obstacle_checks() {
    for camera_forward in [Vec3::Z, -Vec3::Z, Vec3::ZERO] {
        assert_eq!(horizontal_heading(camera_forward, Vec3::Y), Vec3::Y);
        assert_eq!(horizontal_heading(camera_forward, Vec3::ZERO), Vec3::X);
    }
}

fn frame() -> Frame {
    Frame {
        position: Vec3::new(128.0, 128.0, 19.8),
        velocity: Vec3::ZERO,
        height: 1.7,
        water: Some(20.0),
        water_contact: None,
        ground: Some(10.0),
        flying: false,
        can_fly: true,
        seated: false,
        suspended: false,
        locked: false,
        bridge_ready: true,
        running: false,
        obstacle: false,
        landing: None,
        on_ground: true,
        jumping: false,
    }
}

#[test]
fn water_walk_descent_keeps_support_and_uses_ground_crouch_states() {
    let now = Instant::now();
    let prefs = Preferences {
        water_walk: true,
        ..Default::default()
    };
    let mut r = Runtime::default();
    let mut f = frame();
    f.position.z = 20.85;
    f.on_ground = false;
    assert_eq!(r.update(&prefs, &f, &MoveInput::default(), now).motion.mode, 13);
    assert_eq!(r.state, State::WaterWalking);
    let input = MoveInput {
        down: true,
        ..Default::default()
    };
    let step = r.update(&prefs, &f, &input, now + Duration::from_millis(50));
    assert_eq!(step.motion.mode, 13);
    assert_eq!(step.motion.parameters[0], 1.2);
    assert_eq!(r.state, State::WaterWalking);
    assert!(!r.state.swimming());
    assert_eq!(r.water_walk.signal(now).unwrap().0, crate::ao::State::Crouching);
    let moving = MoveInput {
        forward: true,
        run: true,
        ..input
    };
    let step = r.update(&prefs, &f, &moving, now + Duration::from_millis(100));
    assert_eq!(step.motion.mode, 13);
    assert_eq!(step.motion.parameters[0], 1.2, "crouching takes priority over run");
    assert_eq!(r.water_walk.signal(now).unwrap().0, crate::ao::State::CrouchWalk);
    r.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(150));
    assert_eq!(r.water_walk.signal(now).unwrap().0, crate::ao::State::Standing);
    let disabled = Preferences {
        water_walk: false,
        ..prefs
    };
    assert_eq!(r.update(&disabled, &f, &input, now + Duration::from_millis(200)).motion.mode, 0);
    assert!(r.water_walk.signal(now).is_none(), "the mode switch still releases support");
}

#[test]
fn water_walk_jump_keeps_motor_ownership_and_suppresses_automatic_flight_until_landing() {
    let now = Instant::now();
    let prefs = Preferences {
        water_walk: true,
        ..Default::default()
    };
    let mut r = Runtime::default();
    let mut f = frame();
    f.position.z = 20.85;
    f.on_ground = false;
    f.can_fly = false;
    r.update(&prefs, &f, &MoveInput::default(), now);
    let up = MoveInput {
        up: true,
        forward: true,
        run: true,
        ..Default::default()
    };
    let jump = r.update(&prefs, &f, &up, now).motion;
    assert!(jump.action > 0 && jump.valid());
    for (ms, z, vz, pose) in [
        (100, 21.2, 2.0, crate::ao::State::Jumping),
        (450, 21.6, 0.0, crate::ao::State::Jumping),
        (700, 21.2, -2.0, crate::ao::State::Jumping),
    ] {
        f.position.z = z;
        f.velocity.z = vz;
        let step = r.update(&prefs, &f, &up, now + Duration::from_millis(ms));
        assert_eq!(step.motion, jump);
        assert_eq!(step.flying, Some(false));
        assert_eq!(r.state, State::WaterWalking);
        assert_eq!(r.water_walk.pose(), Some(pose));
        assert!(!r.native_fly_allowed() && !r.allows_idle_anchor());
    }
    f.position.z = 20.85;
    f.velocity.z = 0.0;
    let landed = r.update(&prefs, &f, &up, now + Duration::from_millis(1000));
    assert_eq!(landed.motion, Motion::motor(13, 5.13, f.height));
    assert_eq!(r.water_walk.pose(), Some(crate::ao::State::Running));
    assert!(!r.native_fly_allowed());
}

#[test]
fn water_walk_intermediate_height_jump_keeps_its_pose_until_the_arc_lands() {
    let now = Instant::now();
    let prefs = Preferences {
        water_walk: true,
        ..Default::default()
    };
    let mut runtime = Runtime::default();
    let mut f = frame();
    let surface = 20.85;
    f.position.z = surface;
    f.on_ground = false;
    runtime.update(&prefs, &f, &MoveInput::default(), now);
    let input = MoveInput {
        up: true,
        forward: true,
        run: true,
        ..Default::default()
    };
    let jump = runtime.update(&prefs, &f, &input, now).motion;
    assert_eq!(jump.parameters, [5.13, f.height, 2.5]);
    // Prescribed samples of a 2.5 m / 1.25 s arc, not a Havok simulation.
    for (ms, lift, velocity, pose) in [
        (125, 0.9, 6.4, crate::ao::State::Jumping),
        (625, 2.5, 0.0, crate::ao::State::Jumping),
        (1000, 1.6, -4.8, crate::ao::State::Jumping),
        (1125, 0.9, -6.4, crate::ao::State::Jumping),
    ] {
        f.position.z = surface + lift;
        f.velocity.z = velocity;
        let step = runtime.update(&prefs, &f, &input, now + Duration::from_millis(ms));
        assert_eq!(step.motion, jump);
        assert_eq!(step.flying, Some(false));
        assert_eq!(runtime.water_walk.pose(), Some(pose));
        assert!(!runtime.native_fly_allowed() && !runtime.allows_idle_anchor());
    }
    f.position.z = surface;
    f.velocity.z = 0.0;
    let landed = runtime.update(&prefs, &f, &input, now + Duration::from_millis(1400));
    assert_eq!(landed.motion, Motion::motor(13, 5.13, f.height));
    assert_eq!(runtime.water_walk.pose(), Some(crate::ao::State::Running));
}

#[test]
fn water_walking_follows_visual_waves_only_while_supported() {
    let mut f = frame();
    f.position.z = 20.85;
    let mut follower = water_float::Follower::default();
    for pose in [
        crate::ao::State::Standing,
        crate::ao::State::Walking,
        crate::ao::State::Running,
        crate::ao::State::Crouching,
        crate::ao::State::CrouchWalk,
    ] {
        assert!(follower.at_surface(&f, State::WaterWalking, Some(pose), false));
        assert!(!follower.at_surface(&f, State::Ground, Some(pose), false));
        assert!(!follower.at_surface(&f, State::WaterWalking, Some(pose), true));
    }
}

#[test]
fn surface_float_accepts_idle_and_all_surface_strokes_without_terrain_bridge_or_speed_requirements() {
    use crate::ao::State as Pose;
    for height in [0.6, 1.7, 3.0] {
        for bias in [-0.4, 0.0, 0.4] {
            for pose in [Pose::Floating, Pose::SwimmingForward] {
                for speed in [0.0, 0.3, 2.5, 6.0] {
                    let mut frame = frame();
                    frame.height = height;
                    frame.position.z = 20.0 - (height * 0.5 - 0.4).clamp(0.25, 0.8) + bias;
                    frame.velocity = Vec3::new(speed, 0.0, 1.1);
                    frame.ground = None;
                    frame.bridge_ready = false;
                    let mut follower = water_float::Follower::default();
                    for i in 0..60 {
                        // The resolved AO can remain in a surface swim pose
                        // while the physical motor is unavailable/suspended.
                        assert!(
                            follower.at_surface(&frame, State::Ground, Some(pose), false),
                            "{height} {bias} {pose:?} {speed}"
                        );
                        follower.update(1, frame.position, i as f32 / 60.0, 0.25, false);
                    }
                    assert!((follower.offset() - 0.25).abs() < 0.001);
                }
            }
        }
    }
}

#[test]
fn surface_float_uses_live_surface_intent_after_the_simulator_root_drifts_from_the_flat_target() {
    use crate::ao::State as Pose;
    let now = Instant::now();
    for bias in [-0.4, 0.4] {
        let mut runtime = Runtime::default();
        let mut frame = frame();
        frame.position.z = 20.0 - (frame.height * 0.5 - 0.4).clamp(0.25, 0.8);
        let mut follower = water_float::Follower::default();
        let mut input = MoveInput::default();
        runtime.update(&prefs(), &frame, &input, now);
        assert_eq!(runtime.state, State::Surface);
        frame.position.z += bias;
        for (i, expected) in [Pose::Floating, Pose::SwimmingForward, Pose::SwimmingForward, Pose::SwimmingForward]
            .into_iter()
            .enumerate()
        {
            input.forward = i == 1 || i == 2;
            input.back = i == 3;
            input.run = i == 2;
            frame.velocity = Vec3::X * i as f32;
            let now = now + Duration::from_millis(i as u64 * 16 + 16);
            runtime.update(&prefs(), &frame, &input, now);
            let (pose, _) = runtime.swim_signal(now, &input).expect("live swim intent");
            assert_eq!(runtime.state, State::Surface);
            assert_eq!(pose, expected);
            assert!(follower.at_surface(&frame, runtime.state, Some(pose), false));
        }
    }
}

#[test]
fn surface_float_inspection_keeps_an_existing_hold_without_starting_one_for_underwater_hover() {
    use crate::ao::State as Pose;
    let mut frame = frame();
    let mut follower = water_float::Follower::default();
    assert!(follower.at_surface(&frame, State::Surface, Some(Pose::Floating), false));
    frame.suspended = true;
    assert!(follower.at_surface(&frame, State::Ground, Some(Pose::UnderwaterFloating), false));
    assert!(!water_float::Follower::default().at_surface(&frame, State::Ground, Some(Pose::UnderwaterFloating), false));
    assert!(!follower.at_surface(&frame, State::Ground, Some(Pose::SwimmingDown), false));
    assert!(!follower.at_surface(&frame, State::Ground, Some(Pose::UnderwaterFloating), false));
}

#[test]
fn surface_float_intent_queries_cannot_restart_the_aos_current_stroke() {
    let now = Instant::now();
    let mut runtime = Runtime {
        state: State::Surface,
        ..Default::default()
    };
    let input = MoveInput {
        forward: true,
        run: true,
        ..Default::default()
    };
    let (_, first) = runtime.swim_signal(now, &input).expect("crawl signal");
    for _ in 0..10 {
        assert_eq!(runtime.swim_intent(&MoveInput::default()), Some(crate::ao::State::Floating));
    }
    let (_, next) = runtime
        .swim_signal(now + Duration::from_millis(160), &input)
        .expect("continued crawl signal");
    assert_eq!(first.continuous_start, next.continuous_start);
}

#[test]
fn surface_float_rejects_wading_diving_submerged_strokes_exits_seats_and_missing_water() {
    use crate::ao::State as Pose;
    let mut follower = water_float::Follower::default();
    let mut frame = frame();
    for pose in [
        Pose::Standing,
        Pose::Walking,
        Pose::Running,
        Pose::UnderwaterFloating,
        Pose::UnderwaterSwimming,
        Pose::SwimmingUp,
        Pose::SwimmingDown,
        Pose::WaterEntry,
        Pose::WaterDiveJump,
        Pose::WaterDiveEntry,
        Pose::WaterCannonball,
        Pose::WaterShoreExit,
        Pose::WaterFlightExit,
    ] {
        assert!(!follower.at_surface(&frame, State::Surface, Some(pose), false), "{pose:?}");
    }
    for pose in [
        Pose::Floating,
        Pose::SwimmingForward,
        Pose::LegacySwimmingCrawl,
        Pose::LegacySwimmingBackward,
    ] {
        assert!(!follower.at_surface(&frame, State::Submerged, Some(pose), false));
        assert!(!follower.at_surface(&frame, State::Surface, Some(pose), true));
    }
    frame.seated = true;
    assert!(!follower.at_surface(&frame, State::Surface, Some(Pose::Floating), false));
    frame.seated = false;
    frame.jumping = true;
    assert!(!follower.at_surface(&frame, State::Surface, Some(Pose::Floating), false));
    frame.jumping = false;
    for z in [16.0, 24.0, f32::NAN] {
        frame.position.z = z;
        assert!(!follower.at_surface(&frame, State::Surface, Some(Pose::Floating), false));
    }
    frame.position.z = 19.55;
    frame.water = None;
    assert!(!follower.at_surface(&frame, State::Surface, Some(Pose::Floating), false));
}

fn prefs() -> Preferences {
    Preferences {
        running: true,
        fly_assist: true,
        fly_multiplier: 2.0,
        swimming: true,
        run_multiplier: 2.0,
        swim_speed: 2.0,
        ..Default::default()
    }
}

#[test]
fn camera_inspection_releases_the_motor_but_preserves_surface_and_underwater_animation_context() {
    let now = Instant::now();
    for (depth, z, expected) in [
        (State::Surface, 19.55, crate::ao::State::SwimmingForward),
        (State::Submerged, 18.0, crate::ao::State::UnderwaterSwimming),
    ] {
        let mut runtime = Runtime {
            state: depth,
            ..Default::default()
        };
        let mut f = frame();
        f.position.z = z;
        f.suspended = true;
        let input = MoveInput {
            forward: true,
            ..Default::default()
        };
        for i in 0..5 {
            let step = runtime.update(&prefs(), &f, &input, now + Duration::from_millis(i * 16));
            assert_eq!(step.motion.mode, 0);
            assert_eq!(runtime.swim_intent(&input), Some(expected));
        }
        f.seated = true;
        runtime.update(&prefs(), &f, &input, now + Duration::from_secs(1));
        assert!(runtime.swim_intent(&input).is_none());
    }
}
#[test]
fn swimming_speed_defaults_migrate_once() {
    let defaults = Preferences::default();
    assert_eq!(defaults.swim_speed, 5.0);
    for speed in [2.0, 3.5, 7.0] {
        let mut old: Preferences = serde_json::from_value(serde_json::json!({
            "controller_version": 4, "swim_speed": speed,
            "running": true, "fly_assist": false, "swimming": true,
            "run_multiplier": 4.0, "float_height": -0.25
        }))
        .expect("previous movement preferences");
        old.sanitize();
        assert_eq!(old.swim_speed, if speed == 2.0 { 5.0 } else { speed });
        assert!(old.running && old.swimming && !old.fly_assist);
        assert_eq!(old.run_multiplier, 4.0);
        assert_eq!(old.float_height, -0.25);
        // A deliberate choice after migration survives both sanitizing and reload.
        old.swim_speed = 2.0;
        let mut reloaded: Preferences = serde_json::from_value(serde_json::to_value(&old).expect("save")).expect("reload");
        reloaded.sanitize();
        assert_eq!(reloaded, old);
    }
    let mut invalid = Preferences {
        swim_speed: f32::NAN,
        ..defaults
    };
    invalid.sanitize();
    assert_eq!(invalid.swim_speed, 5.0);
}
#[test]
fn movement_water_hysteresis_dive_and_shallow_beach_restore_flight() {
    let mut runtime = Runtime::default();
    let mut f = frame();
    let now = Instant::now();
    let step = runtime.update(&prefs(), &f, &MoveInput::default(), now);
    assert_eq!(runtime.state, State::Surface);
    assert_ne!(
        step.flying,
        Some(true),
        "swimming and exit assistance must not enable native flight"
    );
    assert_eq!(step.motion.mode, 3);
    f.flying = true;
    f.position.z = 20.2;
    runtime.update(&prefs(), &f, &MoveInput::default(), now);
    assert_eq!(runtime.state, State::Surface, "waterline jitter does not alternate modes");
    f.position.z = 17.0;
    let down = MoveInput {
        down: true,
        ..Default::default()
    };
    let step = runtime.update(&prefs(), &f, &down, now);
    assert_eq!(runtime.state, State::Submerged);
    assert_eq!(step.motion, Motion::motor(6, 2.0, 1.7));
    let idle = runtime.update(&prefs(), &f, &MoveInput::default(), now);
    assert_eq!(
        idle.motion,
        Motion::motor(6, 2.0, 1.7),
        "the underwater depth motor stays distinct from surface buoyancy"
    );
    f.ground = Some(19.5);
    let dry = runtime.update(&prefs(), &f, &MoveInput::default(), now);
    assert_eq!(dry.flying, Some(false));
    assert!(!runtime.state.swimming());
}
#[test]
fn falling_into_water_returns_to_configured_flotation_after_entry_overshoot() {
    let now = Instant::now();
    for offset in [-0.5, 0.0, 0.5] {
        let prefs = Preferences {
            float_height: offset,
            ..prefs()
        };
        let mut runtime = Runtime::default();
        let mut f = frame();
        f.position.z = 23.0;
        f.on_ground = false;
        f.velocity.z = -12.0;
        let dry = runtime.update(&prefs, &f, &MoveInput::default(), now);
        assert_eq!(dry.motion.mode, 0);
        f.position.z = 19.9;
        runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(100));
        // The simulator may report the entry near the waterline before the
        // existing buoyancy motor can absorb a fast fall's remaining momentum.
        f.position.z = 16.0;
        let rising = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(200));
        let surface_offset = -0.45 + offset;
        assert_eq!(rising.motion.mode, 8);
        assert!((rising.motion.parameters[2] - surface_offset).abs() < 0.0001);
        assert!(rising.motion.valid());
        assert_eq!(runtime.state, State::Submerged);
        assert_ne!(rising.flying, Some(true));
        assert_eq!(
            runtime.swim_signal(now, &MoveInput::default()).expect("fall cushioning signal").0,
            crate::ao::State::WaterEntry
        );
        let continuing = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_secs(1));
        assert_eq!(continuing.motion, rising.motion, "return must outlive the entry frame");
        f.position.z = 20.0 + surface_offset;
        f.velocity.z = 0.0;
        let arrived = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_secs(2));
        assert_eq!(runtime.state, State::Surface);
        assert_eq!(arrived.motion.mode, 3);
        assert_eq!(arrived.motion.parameters[2], offset);
        assert_ne!(arrived.flying, Some(true));
    }
}

#[test]
fn surface_return_survives_crossing_flotation_while_falling_and_a_slow_ascent() {
    let now = Instant::now();
    let prefs = prefs();
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.on_ground = false;
    f.ground = Some(-50.0);
    f.velocity.z = -10.0;
    f.position.z = 19.9;
    assert_eq!(runtime.update(&prefs, &f, &MoveInput::default(), now).motion.mode, 8);
    f.position.z = 19.55;
    let falling = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(35));
    assert_eq!(falling.motion.mode, 8, "crossing flotation downwards is not arrival");
    assert_eq!(runtime.state, State::Submerged);
    // Fall another 20 m, then rise more slowly than the configured motor
    // speed. The initial shallow-entry estimate cannot end this ascent.
    for tick in 1..=350 {
        let elapsed = tick as f32 * 0.1;
        if elapsed <= 2.0 {
            f.position.z = 19.9 - 10.0 * elapsed;
            f.velocity.z = -10.0;
        } else {
            f.position.z = (-0.1 + 0.6 * (elapsed - 2.0)).min(19.55);
            f.velocity.z = if f.position.z < 19.55 { 0.6 } else { 0.0 };
        }
        let step = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(tick * 100));
        assert_eq!(
            step.motion.mode,
            if f.position.z < 19.55 - 0.18 { 8 } else { 3 },
            "elapsed={elapsed}"
        );
        assert_ne!(step.flying, Some(true));
    }
    assert_eq!(runtime.state, State::Surface);
}

#[test]
fn automatic_surface_return_uses_the_same_goal_as_a_short_underwater_ascent_tap() {
    let now = Instant::now();
    let prefs = prefs();
    let mut automatic = Runtime::default();
    let mut manual = Runtime::default();
    let mut f = frame();
    f.position.z = 16.0;
    let down = MoveInput {
        down: true,
        ..Default::default()
    };
    let up = MoveInput {
        up: true,
        ..Default::default()
    };
    manual.update(&prefs, &f, &down, now);
    let auto_goal = automatic.update(&prefs, &f, &MoveInput::default(), now);
    let manual_goal = manual.update(&prefs, &f, &up, now + Duration::from_millis(10));
    assert_eq!(auto_goal.motion.mode, 8);
    assert_eq!(manual_goal.motion, auto_goal.motion);
    for (millis, height) in [(90, 16.0), (1000, 17.0), (6000, 18.0), (9000, 19.55)] {
        f.position.z = height;
        let at = now + Duration::from_millis(millis);
        let auto_step = automatic.update(&prefs, &f, &MoveInput::default(), at);
        let manual_step = manual.update(&prefs, &f, &MoveInput::default(), at);
        assert_eq!(auto_step.motion, manual_step.motion);
        assert_eq!(automatic.state, manual.state);
    }
    assert_eq!(automatic.state, State::Surface);
}

#[test]
fn descent_cancels_a_surface_return_while_repeated_ascent_keeps_returning() {
    let now = Instant::now();
    let prefs = prefs();
    for manual in [false, true] {
        for up in [false, true] {
            let mut runtime = Runtime::default();
            let mut f = frame();
            f.position.z = 16.0;
            if manual {
                runtime.update(
                    &prefs,
                    &f,
                    &MoveInput {
                        down: true,
                        ..Default::default()
                    },
                    now,
                );
                runtime.update(
                    &prefs,
                    &f,
                    &MoveInput {
                        up: true,
                        ..Default::default()
                    },
                    now + Duration::from_millis(10),
                );
            }
            let returning = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(90));
            assert_eq!(returning.motion.mode, 8);
            let input = MoveInput {
                up,
                down: !up,
                ..Default::default()
            };
            let interrupted = runtime.update(&prefs, &f, &input, now + Duration::from_millis(500));
            assert_eq!(interrupted.motion.mode, if up { 8 } else { 6 }, "manual={manual}, up={up}");
            let released = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(590));
            assert_eq!(
                released.motion.mode,
                if up { 8 } else { 6 },
                "release retains the requested destination"
            );
            assert_ne!(released.flying, Some(true));
            let idle = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_secs(5));
            assert_eq!(idle.motion.mode, if up { 8 } else { 6 });
            let restarted = runtime.update(
                &prefs,
                &f,
                &MoveInput {
                    up: true,
                    ..Default::default()
                },
                now + Duration::from_secs(6),
            );
            assert_eq!(restarted.motion.mode, 8, "a later tap can request the surface again");
        }
    }
}

#[test]
fn entry_surface_return_yields_to_deliberate_descent_and_does_not_rearm_underwater() {
    let now = Instant::now();
    let down = MoveInput {
        down: true,
        ..Default::default()
    };
    for descend_on_entry in [false, true] {
        let mut runtime = Runtime::default();
        let mut f = frame();
        f.position.z = 16.0;
        let input = if descend_on_entry { down } else { MoveInput::default() };
        let entry = runtime.update(&prefs(), &f, &input, now);
        assert_eq!(entry.motion.mode, if descend_on_entry { 6 } else { 8 });
        let diving = runtime.update(&prefs(), &f, &down, now + Duration::from_millis(100));
        assert_eq!(diving.motion.mode, 6);
        let resting = runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_secs(1));
        assert_eq!(resting.motion.mode, 6, "idle divers retain their chosen depth");
        assert!(runtime.water_target.is_none());
        f.suspended = true;
        assert_eq!(
            runtime
                .update(&prefs(), &f, &MoveInput::default(), now + Duration::from_secs(2))
                .motion
                .mode,
            0
        );
        f.suspended = false;
        let resumed = runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_secs(3));
        assert_eq!(
            resumed.motion.mode, 6,
            "resuming an underwater session is not a new fall into water"
        );
    }
}

#[test]
fn entry_surface_return_expires_or_releases_when_swimming_cannot_continue() {
    let now = Instant::now();
    for reason in 0..7 {
        let mut runtime = Runtime::default();
        let mut f = frame();
        let mut prefs = prefs();
        f.position.z = 16.0;
        assert_eq!(runtime.update(&prefs, &f, &MoveInput::default(), now).motion.mode, 8);
        match reason {
            0 => prefs.swimming = false,
            1 => f.seated = true,
            2 => f.suspended = true,
            3 => f.bridge_ready = false,
            4 => f.water = None,
            5 => f.ground = Some(19.5),
            _ => {}
        }
        let step = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_secs(10));
        assert_eq!(step.motion.mode, if reason == 6 { 6 } else { 0 }, "reason={reason}");
        assert!(runtime.water_target.is_none());
        assert!(!runtime.surface_on_entry);
    }
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.position.z = 16.0;
    f.locked = true;
    assert_eq!(runtime.update(&prefs(), &f, &MoveInput::default(), now).motion.mode, 6);
    assert!(
        runtime.water_target.is_none(),
        "automatic buoyancy must not steal normal Movelock ownership"
    );
}

#[test]
fn movement_jumps_only_at_a_stalled_steep_surface_obstacle_with_cooldown() {
    let mut runtime = Runtime::default();
    let mut f = frame();
    let now = Instant::now();
    let input = MoveInput {
        forward: true,
        ..Default::default()
    };
    runtime.update(&prefs(), &f, &input, now);
    assert_eq!(runtime.state, State::Surface);
    f.obstacle = true;
    runtime.update(&prefs(), &f, &input, now + Duration::from_secs(1));
    let step = runtime.update(&prefs(), &f, &input, now + Duration::from_millis(1900));
    assert_ne!(
        step.flying,
        Some(true),
        "swimming and exit assistance must not enable native flight"
    );
    assert_eq!(step.motion.mode, 4);
    assert_eq!(step.motion.parameters[2], 4.5);
    assert_eq!(runtime.state, State::Leaving);
    f.position.z = 17.0;
    let returning = runtime.update(&prefs(), &f, &input, now + Duration::from_secs(3));
    assert_eq!(returning.motion.mode, 8, "falling back after the exit also requests flotation");
    runtime.update(&prefs(), &f, &MoveInput { down: true, ..input }, now + Duration::from_secs(4));
    let step = runtime.update(&prefs(), &f, &input, now + Duration::from_secs(5));
    assert_eq!(runtime.state, State::Submerged);
    assert_eq!(step.motion.mode, 6, "underwater walls do not eject a diver");
}
#[test]
fn movement_seating_suspension_and_disable_release_forces() {
    let now = Instant::now();
    for field in 0..3 {
        let mut runtime = Runtime::default();
        let mut f = frame();
        let mut prefs = prefs();
        let input = MoveInput {
            forward: true,
            ..Default::default()
        };
        let step = runtime.update(&prefs, &f, &input, now);
        assert_ne!(step.motion.mode, 0);
        assert!(runtime.state.swimming());
        match field {
            0 => f.seated = true,
            1 => f.suspended = true,
            2 => {
                prefs.running = false;
                prefs.fly_assist = false;
                prefs.swimming = false;
            }
            _ => unreachable!(),
        }
        let step = runtime.update(&prefs, &f, &input, now);
        assert_eq!(step.motion, Motion::default());
        assert!(!runtime.state.swimming());
    }
}
#[test]
fn movement_motor_settings_do_not_encode_direction_or_idle_braking_vectors() {
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.water = None;
    f.flying = true;
    let input = MoveInput {
        forward: true,
        strafe_left: true,
        up: true,
        ..Default::default()
    };
    let step = runtime.update(&prefs(), &f, &input, Instant::now());
    assert_eq!(step.motion.mode, 2);
    assert_eq!(step.motion, Motion::motor(2, 32.0, 1.7));
    let step = runtime.update(&prefs(), &f, &MoveInput::default(), Instant::now());
    assert_eq!(
        step.motion,
        Motion::motor(2, 32.0, 1.7),
        "LSL detects idle and captures one hover anchor"
    );
    f.flying = false;
    f.running = true;
    let step = runtime.update(&prefs(), &f, &MoveInput { up: false, ..input }, Instant::now());
    assert_eq!(step.motion.mode, 1);
    assert_eq!(step.motion.parameters[2], 0.0);
}

#[test]
fn running_turns_and_direction_reversals_send_the_same_motor_settings() {
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.water = None;
    f.running = true;
    for input in [
        MoveInput {
            forward: true,
            ..Default::default()
        },
        MoveInput {
            back: true,
            ..Default::default()
        },
        MoveInput {
            strafe_left: true,
            ..Default::default()
        },
        MoveInput {
            forward: true,
            strafe_right: true,
            ..Default::default()
        },
    ] {
        let step = runtime.update(&prefs(), &f, &input, Instant::now());
        assert_eq!(
            step.motion,
            Motion::motor(1, 2.0, 1.7),
            "the script reads direction directly from control events"
        );
    }
    let stopped = runtime.update(&prefs(), &f, &MoveInput::default(), Instant::now());
    assert_eq!(stopped.motion, Motion::default());
}

#[test]
fn legacy_flight_speeds_migrate_to_a_neutral_default_or_a_custom_multiplier() {
    for (previous, expected) in [(10.0, 1.0), (25.0, 1.0), (18.0, 1.125), (32.0, 2.0), (8.0, 1.0)] {
        let mut prefs: Preferences = serde_json::from_value(serde_json::json!({
            "fly_speed": previous, "controller_version": 1
        }))
        .expect("legacy movement preferences");
        prefs.sanitize();
        assert_eq!(prefs.fly_multiplier, expected);
        assert_eq!(prefs.controller_version, 6);
    }
}

#[test]
fn passage_keeps_one_action_and_offset_until_its_deadline_without_enabling_flight() {
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let f = frame();
    let motion = runtime.start_passage(Vec3::X * 3.0, now).expect("bounded physical passage");
    assert_eq!(motion.mode, 5);
    assert!(motion.action > 0);
    for millis in [0, 400, 800, 1200] {
        let step = runtime.update(
            &Preferences {
                pass_wall: true,
                ..Default::default()
            },
            &f,
            &MoveInput::default(),
            now + Duration::from_millis(millis),
        );
        assert_eq!(step.motion, motion, "repeated frames must retain the same one-shot action");
        assert_eq!(step.flying, None);
    }
    let end = runtime.update(
        &Preferences {
            pass_wall: true,
            ..Default::default()
        },
        &f,
        &MoveInput::default(),
        now + Duration::from_millis(1550),
    );
    assert_eq!(end.motion, Motion::default());
    assert!(runtime.start_passage(Vec3::X * 9.0, now).is_none());
}

#[test]
fn passage_cancels_on_sitting_focus_loss_or_a_movelock_hold() {
    let now = Instant::now();
    for reason in 0..3 {
        let mut runtime = Runtime::default();
        runtime.start_passage(Vec3::X * 3.0, now).expect("passage");
        let mut f = frame();
        match reason {
            0 => f.seated = true,
            1 => f.suspended = true,
            _ => f.locked = true,
        }
        let step = runtime.update(
            &Preferences {
                pass_wall: true,
                ..Default::default()
            },
            &f,
            &MoveInput::default(),
            now,
        );
        assert_eq!(step.motion, Motion::default());
        assert!(runtime.passage.is_none());
    }
}

#[test]
fn passage_cancels_pending_water_gestures_so_releasing_the_shortcut_cannot_take_off() {
    let now = Instant::now();
    let p = Preferences {
        pass_wall: true,
        ..prefs()
    };
    for depth in [19.8, 17.0] {
        let mut runtime = Runtime::default();
        let mut f = frame();
        f.position.z = depth;
        runtime.update(&p, &f, &MoveInput::default(), now);
        runtime.update(
            &p,
            &f,
            &MoveInput {
                up: true,
                ..Default::default()
            },
            now + Duration::from_millis(10),
        );
        let pass = runtime
            .start_passage(Vec3::X * 3.0, now + Duration::from_millis(20))
            .expect("passage");
        assert!(runtime.water_target.is_none());
        let active = runtime.update(&p, &f, &MoveInput::default(), now + Duration::from_millis(100));
        assert_eq!(active.motion, pass);
        let done = runtime.update(&p, &f, &MoveInput::default(), now + Duration::from_millis(1600));
        assert_eq!(done.motion.mode, if depth < 19.0 { 6 } else { 3 });
        assert_ne!(done.flying, Some(true));
    }
}
#[test]
fn movement_strokes_are_distinct_valid_looping_network_assets() {
    let mut encoded = Vec::new();
    for style in Style::ALL {
        let animation = strokes::animation(style);
        let pelvis = animation.joints[0].rot_keys[0].rotation;
        assert!((pelvis * Vec3::Z).x > 0.85, "the head points along the avatar's forward axis");
        let face = (pelvis * Vec3::X).z;
        assert!(if style == Style::Backstroke { face > 0.85 } else { face < -0.85 });
        let bytes = aurora_assets::import_animation::encode(&animation).expect("bundled stroke");
        let decoded = aurora_assets::anim::parse_animation_upload(&bytes).expect("network asset");
        assert!(decoded.looping && decoded.base_priority == 4 && decoded.joints.len() > 10);
        for joint in &decoded.joints {
            let first = joint.rot_keys.first().expect("first");
            let last = joint.rot_keys.last().expect("last");
            assert!(first.rotation.dot(last.rotation).abs() > 0.999);
        }
        encoded.push(bytes);
    }
    assert!(encoded[0] != encoded[1] && encoded[1] != encoded[2]);
    let mut prefs = prefs();
    prefs.run_multiplier = f32::NAN;
    prefs.fly_multiplier = f32::INFINITY;
    prefs.swim_speed = -3.0;
    prefs.sanitize();
    assert_eq!((prefs.run_multiplier, prefs.fly_multiplier, prefs.swim_speed), (1.0, 1.0, 0.5));
}

#[test]
fn running_flight_and_swimming_switches_are_independent_in_every_combination() {
    let input = MoveInput {
        forward: true,
        ..Default::default()
    };
    for switches in 0..8 {
        let prefs = Preferences {
            running: switches & 1 != 0,
            fly_assist: switches & 2 != 0,
            swimming: switches & 4 != 0,
            ..prefs()
        };
        for kind in 0..3 {
            let mut f = frame();
            f.running = true;
            f.flying = kind == 1;
            if kind != 2 {
                f.water = None;
            }
            let step = Runtime::default().update(&prefs, &f, &input, Instant::now());
            let expected = match kind {
                0 if prefs.running => 1,
                1 if prefs.fly_assist => 2,
                2 if prefs.swimming => 3,
                // With swimming off, a running avatar in water still uses
                // its running enhancement, independently of swimming.
                2 if prefs.running => 1,
                _ => 0,
            };
            assert_eq!(step.motion.mode, expected, "switches={switches}, kind={kind}");
            if !prefs.swimming {}
        }
    }
}

#[test]
fn water_modes_switch_exclusively_and_disable_without_reactivating_the_other() {
    let mut prefs = Preferences {
        running: true,
        fly_assist: true,
        ..Default::default()
    };
    for walking in [true, false, true, false] {
        prefs.set_water_mode(walking, true, true);
        assert_eq!((prefs.water_walk, prefs.swimming), (walking, !walking));
        prefs.set_water_mode(walking, false, true);
        assert!(!prefs.water_walk && !prefs.swimming);
        assert!(prefs.running && prefs.fly_assist);
    }
    prefs.set_water_mode(true, true, false);
    prefs.set_water_mode(false, true, false);
    assert!(
        prefs.water_walk && !prefs.swimming,
        "missing AO swim slots must not disable the working mode"
    );
    prefs.set_water_mode(true, false, false);
    assert!(!prefs.water_walk && !prefs.swimming);
    prefs.swimming = true;
    prefs.water_walk = true;
    prefs.sanitize();
    assert!(prefs.water_walk && !prefs.swimming, "old both-on preferences migrate consistently");
}

#[test]
fn running_at_x1_is_native_and_the_multiplier_cannot_request_a_slowdown() {
    let mut prefs = Preferences {
        running: true,
        ..Default::default()
    };
    assert_eq!(prefs.run_multiplier, 1.0);
    let mut f = frame();
    f.water = None;
    f.running = true;
    let input = MoveInput {
        forward: true,
        ..Default::default()
    };
    for (value, expected) in [(-1.0, 1.0), (1.0, 1.0), (3.0, 3.0), (5.0, 5.0), (9.0, 5.0), (f32::NAN, 1.0)] {
        prefs.run_multiplier = value;
        prefs.sanitize();
        assert_eq!(prefs.run_multiplier, expected);
        let step = Runtime::default().update(&prefs, &f, &input, Instant::now());
        assert_eq!(
            step.motion,
            if expected == 1.0 {
                Motion::default()
            } else {
                Motion::motor(1, expected, f.height)
            }
        );
    }
    prefs.run_multiplier = 5.0;
    f.running = false;
    assert_eq!(
        Runtime::default().update(&prefs, &f, &input, Instant::now()).motion,
        Motion::default()
    );
}

#[test]
fn flotation_offset_preserves_surface_classification_and_is_sent_for_diving_too() {
    let now = Instant::now();
    for height in [0.6_f32, 1.7, 3.0] {
        for offset in [-0.5, 0.0, 0.5] {
            let prefs = Preferences {
                float_height: offset,
                ..prefs()
            };
            let mut runtime = Runtime::default();
            let mut f = frame();
            f.height = height;
            runtime.update(&prefs, &f, &MoveInput::default(), now);
            f.position.z = 20.0 - (height * 0.5 - 0.4).clamp(0.25, 0.8) + offset;
            let step = runtime.update(&prefs, &f, &MoveInput::default(), now);
            assert_eq!(runtime.state, State::Surface, "height={height}, offset={offset}");
            assert_eq!(step.motion.parameters, [2.0, height, offset]);
            let step = runtime.update(
                &prefs,
                &f,
                &MoveInput {
                    down: true,
                    ..Default::default()
                },
                now,
            );
            assert_eq!(step.motion.mode, 7);
            assert_eq!(step.motion.parameters[2], -(height * 0.5 + 1.1).max(1.8));
        }
    }
}

#[test]
fn adjusting_flotation_while_idle_does_not_trigger_a_dive_before_the_motor_reaches_the_new_height() {
    let now = Instant::now();
    let mut prefs = prefs();
    let mut runtime = Runtime::default();
    let mut f = frame();
    runtime.update(&prefs, &f, &MoveInput::default(), now);
    for offset in [-0.5, 0.5, -0.5, 0.0] {
        prefs.float_height = offset;
        // Several HTTP/physics frames can pass before the new target is reached.
        for _ in 0..3 {
            let step = runtime.update(&prefs, &f, &MoveInput::default(), now);
            assert_eq!(step.motion.mode, 3, "offset={offset}");
        }
        f.position.z = 20.0 - 0.45 + offset;
        runtime.update(&prefs, &f, &MoveInput::default(), now);
        assert_eq!(runtime.state, State::Surface);
    }
}

#[test]
fn disabling_swimming_cancels_an_exit_pulse_and_releases_ao_state() {
    let now = Instant::now();
    let mut runtime = Runtime::default();
    runtime.update(&prefs(), &frame(), &MoveInput::default(), now);
    runtime.jump_until = Some(now + Duration::from_millis(350));
    let prefs = Preferences {
        swimming: false,
        ..prefs()
    };
    let step = runtime.update(&prefs, &frame(), &MoveInput::default(), now);
    assert_eq!(step.motion, Motion::default());
    assert_eq!(step.flying, Some(false));
    assert!(runtime.swim_signal(now, &MoveInput::default()).is_none());
    assert!(runtime.jump_until.is_none());
}

#[test]
fn movement_preferences_migrate_old_run_speeds_and_preserve_independent_settings() {
    for (old, expected) in [(4.5, 1.0), (8.0, 1.0), (2.0, 1.0), (10.26, 2.0)] {
        let mut prefs: Preferences =
            serde_json::from_value(serde_json::json!({"run_speed": old, "controller_version": 2, "enabled": true, "running": true, "fly_assist": true, "swimming": true})).expect("old prefs");
        prefs.sanitize();
        assert!((prefs.run_multiplier - expected).abs() < 1.0e-5);
        assert!(prefs.running && prefs.fly_assist && prefs.swimming);
        assert!(prefs.legacy_run_speed.is_none());
    }
    let prefs = Preferences {
        running: false,
        fly_assist: false,
        run_multiplier: 5.0,
        float_height: -0.3,
        ..prefs()
    };
    let encoded = serde_json::to_string(&prefs).expect("save");
    assert!(!encoded.contains("run_speed"));
    let mut restored: Preferences = serde_json::from_str(&encoded).expect("restore");
    restored.sanitize();
    assert_eq!(restored, prefs);
    restored.float_height = f32::INFINITY;
    restored.sanitize();
    assert_eq!(restored.float_height, 0.0);
    for value in [-3.0, 3.0] {
        restored.float_height = value;
        restored.sanitize();
        assert_eq!(restored.float_height, value.clamp(-0.5, 0.5));
    }
}
#[test]
fn removing_the_global_switch_preserves_effective_legacy_activation() {
    assert!(!Preferences::default().any_enabled());
    for enabled in [false, true] {
        let mut prefs: Preferences = serde_json::from_value(serde_json::json!({
            "controller_version": 3, "enabled": enabled,
            "running": true, "fly_assist": false, "swimming": true
        }))
        .expect("legacy global switch");
        prefs.sanitize();
        assert_eq!((prefs.running, prefs.fly_assist, prefs.swimming), (enabled, false, enabled));
        assert_eq!(prefs.any_enabled(), enabled);
        let saved = serde_json::to_value(&prefs).expect("save independent switches");
        assert!(saved.get("enabled").is_none());
        assert_eq!(prefs.controller_version, 6);
    }
}

#[test]
fn enhanced_run_holds_only_a_brief_small_loss_of_ground_contact() {
    let now = Instant::now();
    let prefs = prefs();
    let mut f = frame();
    f.water = None;
    f.running = true;
    let input = MoveInput {
        forward: true,
        ..Default::default()
    };
    let mut runtime = Runtime::default();
    runtime.update(&prefs, &f, &input, now);
    assert_eq!(runtime.run_start, Some(now));
    f.on_ground = false;
    f.position.z -= 0.1;
    f.velocity.z = -0.5;
    runtime.update(&prefs, &f, &input, now + Duration::from_millis(100));
    assert_eq!(runtime.run_start, Some(now));
    runtime.update(&prefs, &f, &input, now + Duration::from_millis(300));
    assert!(runtime.run_start.is_none(), "a real fall cannot renew its own grace");
}

#[test]
fn enhanced_run_pose_releases_on_jump_large_drop_and_other_movement_modes() {
    let now = Instant::now();
    for reason in 0..8 {
        let mut f = frame();
        f.water = None;
        f.running = true;
        let mut prefs = prefs();
        let mut input = MoveInput {
            forward: true,
            ..Default::default()
        };
        let mut runtime = Runtime::default();
        runtime.update(&prefs, &f, &input, now);
        assert!(runtime.run_start.is_some());
        f.on_ground = false;
        match reason {
            0 => input.up = true,
            1 => f.jumping = true,
            2 => f.velocity.z = -2.0,
            3 => f.position.z -= 0.5,
            4 => f.flying = true,
            5 => f.seated = true,
            6 => f.suspended = true,
            _ => prefs.running = false,
        }
        runtime.update(&prefs, &f, &input, now + Duration::from_millis(50));
        assert!(runtime.run_start.is_none(), "reason={reason}");
    }
}

#[test]
fn running_support_uses_the_avatar_contact_plane_on_platforms_and_respects_airborne_clears() {
    let position = Vec3::new(100.0, 100.0, 1000.85);
    let plane = glam::Vec4::new(0.0, 0.0, 1.0, 1000.0);
    assert!(run::on_ground(position, 1.7, Some(plane), Some(20.0), false));
    assert!(!run::on_ground(position, 1.7, Some(glam::Vec4::ZERO), Some(1000.0), false));
    assert!(!run::on_ground(position, 1.7, Some(plane), Some(20.0), true));
    assert!(!run::on_ground(position, 1.7, None, Some(20.0), false));
    assert!(run::on_ground(position, 1.7, None, Some(1000.0), false));
}

#[test]
fn movement_focus_pause_keeps_the_original_flight_preference() {
    let mut runtime = Runtime::default();
    let mut f = frame();
    let now = Instant::now();
    runtime.update(&prefs(), &f, &MoveInput::default(), now);
    f.flying = true;
    f.suspended = true;
    let paused = runtime.update(&prefs(), &f, &MoveInput::default(), now);
    assert_eq!(paused.flying, None);
    assert_eq!(paused.motion, Motion::default());
    f.suspended = false;
    runtime.update(&prefs(), &f, &MoveInput::default(), now);
    f.ground = Some(19.5);
    let dry = runtime.update(&prefs(), &f, &MoveInput::default(), now);
    assert_eq!(dry.flying, Some(false), "focus loss must not turn a swimmer into a flyer on shore");
}

#[test]
fn movement_swimming_controls_never_send_a_native_jump_even_during_exit() {
    use aurora_net::control as c;
    let flags = c::UP_POS | c::UP_NEG | c::FAST_UP | c::AT_POS | c::FLY | c::MOUSELOOK;
    for state in [State::Surface, State::Submerged, State::Leaving] {
        assert_eq!(controls(flags, state, false), c::AT_POS | c::FLY | c::MOUSELOOK);
        assert_eq!(
            controls(flags, state, true),
            flags & !c::FAST_UP,
            "an acknowledged script capture must receive vertical input"
        );
    }
    let flags = flags | c::NUDGE_AT_POS | c::NUDGE_LEFT_NEG | c::FAST_AT;
    for state in [State::Ground, State::Flight] {
        assert_eq!(
            controls(flags, state, false),
            flags,
            "Movelock must receive native movement, including nudges"
        );
    }
}

#[test]
fn movement_arms_swim_capture_while_locked_so_lsl_can_detect_a_voluntary_dive() {
    let mut runtime = Runtime::default();
    let mut frame = frame();
    frame.locked = true;
    let step = runtime.update(
        &prefs(),
        &frame,
        &MoveInput {
            down: true,
            ..Default::default()
        },
        Instant::now(),
    );
    assert_eq!(step.motion.mode, 7);
    let mut expected = Motion::motor(7, 2.0, 1.7);
    expected.parameters[2] = -1.95;
    assert_eq!(step.motion, expected);
}

#[test]
fn surface_forward_ascent_starts_immediately_and_lost_bridge_releases_the_motor() {
    let now = Instant::now();
    let mut f = frame();
    let input = MoveInput {
        forward: true,
        up: true,
        ..Default::default()
    };
    let mut runtime = Runtime::default();
    runtime.update(&prefs(), &f, &MoveInput::default(), now);
    let jump = runtime.update(&prefs(), &f, &input, now + Duration::from_millis(20));
    assert_eq!(jump.motion.mode, 4);
    assert_eq!(jump.flying, Some(false));
    assert_eq!(jump.motion.parameters[2], 4.5);
    f.bridge_ready = false;
    let stopped = runtime.update(&prefs(), &f, &input, now + Duration::from_millis(100));
    assert_eq!(stopped.motion, Motion::default());
    assert!(!runtime.checking_water_landing());
}

#[test]
fn movement_exit_releases_the_climb_after_650ms_and_restores_ground_controls() {
    let now = Instant::now();
    let mut f = frame();
    let mut runtime = Runtime::default();
    let input = MoveInput {
        forward: true,
        ..Default::default()
    };
    runtime.update(&prefs(), &f, &input, now);
    f.flying = true;
    f.obstacle = true;
    runtime.update(&prefs(), &f, &input, now + Duration::from_secs(1));
    let step = runtime.update(&prefs(), &f, &input, now + Duration::from_secs(2));
    assert_eq!(step.motion.mode, 4);
    f.position.z = 20.5;
    let end = runtime.update(&prefs(), &f, &input, now + Duration::from_millis(2800));
    assert_eq!(end.flying, Some(false));
    assert_eq!(end.motion.mode, 0);
    assert_eq!(runtime.state, State::Ground);
}

#[test]
fn movement_bvh_swimming_removes_root_translation_and_closes_loop() {
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml")).expect("skeleton");
    let bvh = b"HIERARCHY ROOT hip { OFFSET 0 0 0 CHANNELS 6 Xposition Yposition Zposition Xrotation Zrotation Yrotation End Site { OFFSET 0 1 0 } } MOTION Frames: 4 Frame Time: 0.1 0 0 0 0 0 0 0 43 0 74 0 0 0 45 0 75 0 0 0 46 0 76 0 0";
    let animation = strokes::from_bvh(bvh, &skeleton).expect("QAvimator stroke");
    assert!(animation.looping && animation.base_priority == 4);
    assert!(animation.joints.iter().all(|joint| joint.pos_keys.is_empty()));
    assert!(
        animation.joints[0]
            .rot_keys
            .first()
            .expect("first")
            .rotation
            .dot(animation.joints[0].rot_keys.last().expect("last").rotation)
            .abs()
            > 0.999
    );
    assert!(aurora_assets::import_animation::encode(&animation).is_ok());
}

#[test]
fn movement_bundled_swim_pack_converts_all_six_clips() {
    for animation in strokes::pack() {
        let bytes = aurora_assets::import_animation::encode(animation).expect("network encode");
        let decoded = aurora_assets::anim::parse_animation_upload(&bytes).expect("network asset");
        assert!(decoded.looping && decoded.duration > 0.5 && decoded.duration < 5.0);
        assert!(
            decoded
                .joints
                .iter()
                .find(|joint| joint.joint_name == "mPelvis")
                .is_none_or(|joint| joint.pos_keys.is_empty())
        );
    }
}

#[test]
fn swimming_state_signal_is_stable_and_released_on_exit() {
    let now = Instant::now();
    let mut runtime = Runtime {
        state: State::Surface,
        ..Default::default()
    };
    let forward = MoveInput {
        forward: true,
        ..Default::default()
    };
    let start = runtime.swim_signal(now, &forward).expect("swim state");
    assert_eq!(start.0, crate::ao::State::SwimmingForward);
    assert_eq!(
        runtime
            .swim_signal(now + Duration::from_secs(1), &forward)
            .expect("swim state")
            .1
            .continuous_start,
        start.1.continuous_start
    );
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("idle state").0,
        crate::ao::State::Floating
    );
    runtime.state = State::Ground;
    assert!(runtime.swim_signal(now, &forward).is_none());
}

#[test]
fn disabling_passage_cancels_its_motor_and_activates_the_toolbar_independently() {
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut prefs = Preferences {
        pass_wall: true,
        ..Default::default()
    };
    assert!(prefs.any_enabled());
    runtime.start_passage(Vec3::X * 3.0, now).expect("passage");
    assert_eq!(runtime.update(&prefs, &frame(), &MoveInput::default(), now).motion.mode, 5);
    prefs.pass_wall = false;
    assert_eq!(
        runtime.update(&prefs, &frame(), &MoveInput::default(), now).motion,
        Motion::default()
    );
    assert!(!prefs.any_enabled());
}

#[test]
fn flight_x1_is_native_and_larger_factors_scale_nominal_flight_without_changing_the_bridge_protocol() {
    let now = Instant::now();
    let mut prefs = Preferences {
        fly_assist: true,
        ..Default::default()
    };
    let mut f = frame();
    f.flying = true;
    f.water = None;
    let mut runtime = Runtime::default();
    for input in [
        MoveInput::default(),
        MoveInput {
            forward: true,
            ..Default::default()
        },
        MoveInput {
            down: true,
            ..Default::default()
        },
    ] {
        assert_eq!(runtime.update(&prefs, &f, &input, now).motion, Motion::default());
    }
    for factor in [1.1, 2.0, 5.0] {
        prefs.fly_multiplier = factor;
        for down in [false, true] {
            let input = MoveInput {
                forward: !down,
                down,
                ..Default::default()
            };
            let step = runtime.update(&prefs, &f, &input, now);
            assert_eq!(step.motion.mode, 2);
            assert_eq!(
                step.motion.parameters[0],
                factor * if down { STANDARD_FLY_DOWN_SPEED } else { STANDARD_FLY_SPEED }
            );
            assert!(step.motion.valid());
        }
    }
    for (value, expected) in [(0.5, 1.0), (6.0, 5.0), (f32::NAN, 1.0)] {
        prefs.fly_multiplier = value;
        prefs.sanitize();
        assert_eq!(prefs.fly_multiplier, expected);
    }
    let saved = serde_json::to_value(&prefs).expect("serialize new preferences");
    assert!(saved.get("fly_speed").is_none());
    assert!(saved.get("fly_multiplier").is_some());
}

#[test]
fn short_down_tap_completes_a_dive_after_release_and_keeps_the_new_depth() {
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame();
    let prefs = prefs();
    runtime.update(&prefs, &f, &MoveInput::default(), now);
    let down = MoveInput {
        down: true,
        ..Default::default()
    };
    let dive = runtime.update(&prefs, &f, &down, now + Duration::from_millis(10));
    assert_eq!(dive.motion.mode, 7);
    assert!(dive.motion.valid());
    assert_eq!(runtime.state, State::Submerged);
    let released = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(90));
    assert_eq!(released.motion, dive.motion);
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("dive pose").0,
        crate::ao::State::SwimmingDown
    );
    f.position.z = 18.05;
    let arrived = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(500));
    assert_eq!(arrived.motion.mode, 6);
    assert_eq!(runtime.state, State::Submerged);
}

#[test]
fn short_up_tap_underwater_returns_to_flotation_without_starting_flight() {
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.position.z = 17.0;
    let prefs = Preferences {
        float_height: 0.2,
        ..prefs()
    };
    runtime.update(
        &prefs,
        &f,
        &MoveInput {
            down: true,
            ..Default::default()
        },
        now,
    );
    let up = MoveInput {
        up: true,
        ..Default::default()
    };
    let rising = runtime.update(&prefs, &f, &up, now + Duration::from_millis(10));
    assert_eq!(rising.motion.mode, 8);
    assert_eq!(runtime.state, State::Submerged, "resurfacing cannot declare arrival before moving");
    assert!(rising.motion.valid());
    let released = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(90));
    assert_eq!(released.motion, rising.motion);
    assert_ne!(released.flying, Some(true));
    f.position.z = 20.0 - (f.height * 0.5 - 0.4) + prefs.float_height;
    let arrived = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_secs(1));
    assert_eq!(arrived.motion.mode, 3);
    assert_eq!(runtime.state, State::Surface);
    assert_ne!(arrived.flying, Some(true));
}

#[test]
fn double_surface_ascent_takes_off_then_hands_back_native_or_enhanced_flight() {
    let now = Instant::now();
    let up = MoveInput {
        up: true,
        ..Default::default()
    };
    for factor in [1.0, 2.0, 5.0] {
        let prefs = Preferences {
            fly_multiplier: factor,
            ..prefs()
        };
        let mut runtime = Runtime::default();
        let mut f = frame();
        f.on_ground = false;
        f.position.z = 20.0 - (f.height * 0.5 - 0.4);
        runtime.update(&prefs, &f, &MoveInput::default(), now);
        assert_eq!(runtime.update(&prefs, &f, &up, now + Duration::from_millis(10)).motion.mode, 4);
        runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(60));
        let takeoff = runtime.update(&prefs, &f, &up, now + Duration::from_millis(150));
        assert_eq!(takeoff.flying, Some(true));
        assert_eq!(takeoff.motion.mode, 10);
        assert!(takeoff.motion.valid());
        f.flying = true;
        let waiting = runtime.update(&prefs, &f, &up, now + Duration::from_millis(650));
        assert_eq!(waiting.motion.mode, 10, "holding the second tap must keep the same flight exit");
        f.position.z = 20.0 + f.height * 0.5 + 1.25;
        let flight = runtime.update(&prefs, &f, &up, now + Duration::from_millis(900));
        assert_eq!(runtime.state, State::Flight);
        assert_eq!(flight.motion.mode, if factor == 1.0 { 0 } else { 2 });
        assert!(flight.motion.valid());
    }
}

#[test]
fn forward_surface_press_owns_the_whole_platform_exit_and_clears_late_flight() {
    let now = Instant::now();
    let mut f = frame();
    f.obstacle = true;
    f.landing = Some(Vec3::new(129.0, 128.0, 21.0));
    let mut runtime = Runtime::default();
    runtime.update(&prefs(), &f, &MoveInput::default(), now);
    let up = MoveInput {
        forward: true,
        up: true,
        ..Default::default()
    };
    let jump = runtime.update(&prefs(), &f, &up, now + Duration::from_millis(10));
    assert_eq!(jump.motion.mode, 9);
    assert_eq!(jump.flying, Some(false));
    let crossing = runtime.update(&prefs(), &f, &up, now + Duration::from_millis(1500));
    assert_eq!(crossing.motion, jump.motion, "allow enough time to clear the lip and cross");
    assert_eq!(runtime.swim_intent(&up), Some(crate::ao::State::WaterShoreExit));
    f.position = f.landing.unwrap();
    f.on_ground = true;
    for millis in [1900, 2300, 2800] {
        f.flying = true; // delayed/repeated native flag after reaching the top
        let landed = runtime.update(&prefs(), &f, &up, now + Duration::from_millis(millis));
        assert_eq!(landed.flying, Some(false));
        assert_eq!(landed.motion, Motion::default());
        assert_eq!(runtime.state, State::Ground);
        assert!(!runtime.native_fly_allowed());
    }
    runtime.cancel_water_exit(); // an explicit Fly button remains authoritative
    let flight = runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_secs(3));
    assert_ne!(flight.flying, Some(false));
    assert_eq!(runtime.state, State::Flight);
}

#[test]
fn short_up_at_a_shore_uses_one_bounded_physical_jump_and_never_enables_flight() {
    let now = Instant::now();
    let prefs = Preferences {
        jump_obstacles: false,
        ..prefs()
    };
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.obstacle = true;
    f.landing = Some(Vec3::new(129.0, 128.0, 21.0));
    runtime.update(&prefs, &f, &MoveInput::default(), now);
    runtime.update(
        &prefs,
        &f,
        &MoveInput {
            up: true,
            forward: true,
            ..Default::default()
        },
        now + Duration::from_millis(10),
    );
    let jump = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(90));
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("obstacle leap pose").0,
        crate::ao::State::WaterShoreExit
    );
    assert_eq!(jump.motion.mode, 9);
    assert_eq!(jump.flying, Some(false));
    assert!(jump.motion.valid());
    assert_ne!(jump.motion.action, 0);
    f.position.x += 0.1;
    let repeated = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(400));
    assert_eq!(
        repeated.motion, jump.motion,
        "a repeated frame cannot move the jump goal or renew its nonce"
    );
    assert_ne!(repeated.flying, Some(true));
    f.position = f.landing.expect("shore");
    let landed = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(900));
    assert_eq!(landed.motion, Motion::default());
    assert_eq!(runtime.state, State::Ground);
}

#[test]
fn an_automatic_water_exit_drops_the_old_flight_preference_before_platform_arrival() {
    let now = Instant::now();
    let mut r = Runtime::default();
    let mut f = frame();
    f.flying = true;
    let input = MoveInput {
        forward: true,
        ..Default::default()
    };
    assert_eq!(r.update(&prefs(), &f, &input, now).flying, Some(false));
    f.flying = false;
    f.obstacle = true;
    r.update(&prefs(), &f, &input, now + Duration::from_secs(1));
    let jump = r.update(&prefs(), &f, &input, now + Duration::from_millis(1900));
    assert_eq!(jump.motion.mode, 4);
    assert_eq!(jump.flying, Some(false));
    assert_eq!(r.previous_flight, Some(false));
    f.position.z = 21.2;
    f.on_ground = true;
    let landed = r.update(&prefs(), &f, &input, now + Duration::from_millis(2400));
    assert_ne!(landed.flying, Some(true));
    assert_eq!(r.state, State::Ground);
}

#[test]
fn a_water_origin_up_hold_cannot_become_native_flight_after_reaching_land() {
    let now = Instant::now();
    for state in [State::Surface, State::Submerged, State::Leaving, State::WaterWalking] {
        let mut r = Runtime {
            state,
            ..Default::default()
        };
        let mut f = frame();
        let input = MoveInput {
            up: true,
            forward: true,
            ..Default::default()
        };
        r.update(&prefs(), &f, &input, now);
        assert!(!r.native_fly_allowed());
        f.position.z = 22.0;
        f.ground = Some(21.15);
        r.update(&prefs(), &f, &input, now + Duration::from_secs(1));
        assert!(
            !r.native_fly_allowed(),
            "the original press is still owned after arrival from {state:?}"
        );
        r.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_millis(1100));
        assert!(r.native_fly_allowed(), "a fresh ground press keeps ordinary AutomaticFly");
    }
}

#[test]
fn walking_out_of_water_onto_support_never_restores_an_old_flight_flag() {
    let now = Instant::now();
    let mut r = Runtime::default();
    let mut f = frame();
    f.flying = true;
    r.update(&prefs(), &f, &MoveInput::default(), now);
    f.flying = false;
    f.position.z = 21.0;
    f.on_ground = true;
    assert_eq!(r.update(&prefs(), &f, &MoveInput::default(), now).flying, Some(false));
    assert_eq!(r.state, State::Ground);
}

#[test]
fn a_fresh_ground_ascent_after_water_landing_keeps_ordinary_automatic_flight() {
    let now = Instant::now();
    let mut runtime = Runtime {
        state: State::Ground,
        water_exit_until: Some(now + Duration::from_secs(4)),
        water_exit_jump: true,
        ..Default::default()
    };
    let mut f = frame();
    f.position.z = 21.0;
    f.on_ground = true;
    f.flying = true;
    let step = runtime.update(
        &prefs(),
        &f,
        &MoveInput {
            up: true,
            ..Default::default()
        },
        now,
    );
    assert!(!runtime.checking_water_landing());
    assert!(runtime.native_fly_allowed());
    assert_ne!(step.flying, Some(false));
}

#[test]
fn typing_or_suspension_cancels_a_pending_water_tap_without_starting_flight() {
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame();
    runtime.update(&prefs(), &f, &MoveInput::default(), now);
    f.suspended = true;
    runtime.update(
        &prefs(),
        &f,
        &MoveInput {
            up: true,
            ..Default::default()
        },
        now + Duration::from_millis(10),
    );
    runtime.cancel_water_gesture();
    let idle = runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_millis(90));
    assert_ne!(idle.flying, Some(true));
    f.suspended = true;
    let idle = runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_millis(110));
    assert_eq!(idle.motion, Motion::default());
}

#[test]
fn an_unreachable_water_target_expires_and_opposite_controls_can_cancel_takeoff() {
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.position.z = 17.0;
    let prefs = prefs();
    let up = MoveInput {
        up: true,
        ..Default::default()
    };
    runtime.update(
        &prefs,
        &f,
        &MoveInput {
            down: true,
            ..Default::default()
        },
        now,
    );
    runtime.update(&prefs, &f, &up, now + Duration::from_millis(10));
    let rising = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(90));
    assert_eq!(rising.motion.mode, 8);
    let blocked = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_secs(10));
    assert_eq!(blocked.motion.mode, 6, "a ceiling must not leave a pending ascent forever");
    f.position.z = 19.8;
    runtime.update(&prefs, &f, &up, now + Duration::from_secs(11));
    runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(11060));
    let takeoff = runtime.update(&prefs, &f, &up, now + Duration::from_millis(11150));
    assert_eq!(takeoff.motion.mode, 10);
    f.flying = true;
    let down = MoveInput {
        down: true,
        ..Default::default()
    };
    let reversed = runtime.update(&prefs, &f, &down, now + Duration::from_millis(11700));
    assert_eq!(reversed.motion.mode, 7);
    assert_eq!(reversed.flying, Some(false));
}

#[test]
fn no_fly_parcels_allow_single_surface_jumps_but_reject_the_second_tap_takeoff() {
    let now = Instant::now();
    for forward in [false, true] {
        let mut runtime = Runtime::default();
        let mut f = frame();
        f.can_fly = false;
        runtime.update(&prefs(), &f, &MoveInput::default(), now);
        let up = MoveInput {
            up: true,
            forward,
            ..Default::default()
        };
        let jump = runtime.update(&prefs(), &f, &up, now + Duration::from_millis(10));
        assert_eq!(jump.motion.mode, 4);
        assert_eq!(jump.flying, Some(false));
        runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_millis(60));
        let blocked = runtime.update(&prefs(), &f, &up, now + Duration::from_millis(150));
        assert_eq!(blocked.motion, jump.motion);
        assert_eq!(blocked.flying, Some(false));
    }
}

#[test]
fn surface_recognition_uses_body_height_and_flotation_with_stable_depth_hysteresis() {
    use crate::ao::State as Pose;
    let now = Instant::now();
    let forward = MoveInput {
        forward: true,
        ..Default::default()
    };
    for height in [0.6_f32, 1.7, 3.0] {
        for offset in [-0.5, 0.0, 0.5] {
            let prefs = Preferences {
                float_height: offset,
                ..prefs()
            };
            let mut f = frame();
            f.height = height;
            let target = 20.0 - (height * 0.5 - 0.4).clamp(0.25, 0.8) + offset;
            let mut runtime = Runtime {
                state: State::Submerged,
                previous_flight: Some(false),
                ..Default::default()
            };
            f.position.z = target;
            assert_eq!(runtime.update(&prefs, &f, &forward, now).motion.mode, 3);
            assert_eq!(runtime.swim_intent(&forward), Some(Pose::SwimmingForward));
            for below in [0.20, 0.35, 0.24, 0.40] {
                f.position.z = target - below;
                runtime.update(&prefs, &f, &forward, now);
                assert_eq!(
                    runtime.swim_intent(&forward),
                    Some(Pose::SwimmingForward),
                    "height={height}, offset={offset}"
                );
            }
            f.position.z = target - 0.5;
            assert_eq!(runtime.update(&prefs, &f, &forward, now).motion.mode, 6);
            assert_eq!(runtime.swim_intent(&forward), Some(Pose::UnderwaterSwimming));
            f.position.z = target - 0.3;
            runtime.update(&prefs, &f, &forward, now);
            assert_eq!(runtime.state, State::Submerged);
            f.position.z = target - 0.15;
            runtime.update(&prefs, &f, &forward, now);
            assert_eq!(runtime.swim_intent(&forward), Some(Pose::SwimmingForward));
        }
    }
}

#[test]
fn an_underwater_up_hold_arrives_in_surface_swimming_without_flight_jump_or_a_second_ascent() {
    use crate::ao::State as Pose;
    let now = Instant::now();
    let mut runtime = Runtime::default();
    let mut f = frame();
    f.position.z = 16.0;
    let prefs = prefs();
    let held = MoveInput {
        up: true,
        forward: true,
        ..Default::default()
    };
    runtime.update(
        &prefs,
        &f,
        &MoveInput {
            down: true,
            ..Default::default()
        },
        now,
    );
    runtime.update(&prefs, &f, &held, now + Duration::from_millis(20));
    let rising = runtime.update(&prefs, &f, &held, now + Duration::from_secs(1));
    assert_eq!(rising.motion.mode, 8);
    assert_ne!(rising.flying, Some(true));
    f.position.z = 19.4;
    f.velocity.z = 0.8;
    let arrived = runtime.update(&prefs, &f, &held, now + Duration::from_secs(2));
    assert_eq!(arrived.motion.mode, 3);
    assert_eq!(runtime.swim_intent(&held), Some(Pose::SwimmingForward));
    assert_ne!(arrived.flying, Some(true));
    let released = runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(2100));
    assert_eq!(released.motion.mode, 3);
    assert_eq!(runtime.swim_intent(&MoveInput::default()), Some(Pose::Floating));
    runtime.update(&prefs, &f, &held, now + Duration::from_secs(3));
    let flight = runtime.update(&prefs, &f, &held, now + Duration::from_millis(3550));
    assert_eq!(flight.motion.mode, 4, "a fresh surface forward press requests a jump");
    assert_eq!(flight.flying, Some(false));

    // A press coinciding with a falling entry belongs to its automatic
    // return too, even before mode 8 has been installed in that frame.
    let mut entering = Runtime::default();
    f.position.z = 19.55;
    f.velocity.z = -3.0;
    f.on_ground = false;
    assert_eq!(entering.update(&prefs, &f, &held, now).motion.mode, 8);
    f.velocity.z = 0.0;
    let arrived = entering.update(&prefs, &f, &held, now + Duration::from_secs(1));
    assert_eq!(arrived.motion.mode, 3);
    assert_ne!(arrived.flying, Some(true));
    assert_eq!(entering.swim_intent(&held), Some(Pose::SwimmingForward));
}

#[test]
fn one_stationary_surface_tap_jumps_and_a_second_tap_takes_flight_above_the_ledge() {
    let now = Instant::now();
    for height in [0.6, 1.7, 3.0] {
        let mut f = frame();
        f.height = height;
        f.on_ground = false;
        f.position.z = 20.0 - (height * 0.5 - 0.4).clamp(0.25, 0.8);
        f.obstacle = true;
        f.landing = Some(Vec3::new(129.0, 128.0, 21.0));
        let mut runtime = Runtime::default();
        runtime.update(&prefs(), &f, &MoveInput::default(), now);
        let up = MoveInput {
            up: true,
            ..Default::default()
        };
        let jump = runtime.update(&prefs(), &f, &up, now + Duration::from_millis(20));
        assert_eq!(jump.motion.mode, 9, "stationary presses use the same safe ledge target");
        assert_eq!(jump.flying, Some(false));
        runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_millis(60));
        let flight = runtime.update(&prefs(), &f, &up, now + Duration::from_millis(150));
        assert!(runtime.shore_jump.is_none(), "second tap must stop steering toward the ledge");
        assert_eq!(flight.motion.mode, 10);
        assert_eq!(flight.flying, Some(true));
        assert!(flight.motion.valid());
        assert!((flight.motion.parameters[2] - height * 0.5 - 1.25).abs() < 0.001);
        f.flying = true;
        runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_millis(200));
        f.position.z = 20.0 + height * 0.5 + 0.3;
        assert_eq!(
            runtime
                .update(&prefs(), &f, &MoveInput::default(), now + Duration::from_millis(500))
                .motion
                .mode,
            10,
            "do not end takeoff with the feet barely above water"
        );
        f.position.z = 20.0 + flight.motion.parameters[2];
        assert_eq!(
            runtime
                .update(&prefs(), &f, &MoveInput::default(), now + Duration::from_secs(1))
                .motion
                .mode,
            2
        );
        assert_eq!(runtime.state, State::Flight);
        f.on_ground = true;
        let landed = runtime.update(&prefs(), &f, &MoveInput::default(), now + Duration::from_secs(2));
        assert_eq!(landed.flying, Some(false), "water takeoff also stops flying on real support");
    }
}

#[test]
fn a_low_platform_landing_takes_priority_over_surface_hysteresis_for_small_avatars() {
    let now = Instant::now();
    let mut runtime = Runtime {
        water_exit_until: Some(now + Duration::from_secs(2)),
        water_exit_jump: true,
        state: State::Leaving,
        ..Default::default()
    };
    let mut f = frame();
    f.height = 0.6;
    f.position.z = 20.3;
    f.on_ground = true;
    f.flying = true;
    let landed = runtime.update(&prefs(), &f, &MoveInput::default(), now);
    assert_eq!(landed.flying, Some(false));
    assert_eq!(landed.motion, Motion::default());
    assert_eq!(runtime.state, State::Ground);
}

#[test]
fn water_entry_pose_only_covers_the_cushioned_downward_fall_and_can_be_cancelled() {
    use crate::ao::State as Pose;
    let now = Instant::now();
    let prefs = Preferences {
        swimming: true,
        ..Default::default()
    };
    let mut f = frame();
    f.position.z = 19.5;
    f.velocity.z = -8.0;
    f.on_ground = false;
    let mut runtime = Runtime::default();
    runtime.update(&prefs, &f, &MoveInput::default(), now);
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("entry pose").0,
        Pose::WaterEntry
    );
    f.velocity.z = -2.0;
    runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(100));
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("still falling").0,
        Pose::WaterEntry
    );
    f.position.z = 17.0;
    f.velocity.z = 0.0;
    runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(200));
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("surface return").0,
        Pose::SwimmingUp
    );
    f.velocity.z = -2.0;
    runtime.update(&prefs, &f, &MoveInput::default(), now + Duration::from_millis(300));
    assert_ne!(
        runtime.swim_signal(now, &MoveInput::default()).expect("not a new entry").0,
        Pose::WaterEntry
    );
    runtime.update(&Preferences::default(), &f, &MoveInput::default(), now + Duration::from_millis(400));
    assert!(runtime.swim_signal(now, &MoveInput::default()).is_none());
}

#[test]
fn underwater_idle_is_distinct_from_surface_idle_and_exit_types_do_not_mix() {
    use crate::ao::State as Pose;
    let now = Instant::now();
    let mut runtime = Runtime {
        state: State::Surface,
        ..Default::default()
    };
    let surface = runtime.swim_signal(now, &MoveInput::default()).expect("surface");
    assert_eq!(surface.0, Pose::Floating);
    runtime.state = State::Submerged;
    let submerged = runtime
        .swim_signal(now + Duration::from_secs(1), &MoveInput::default())
        .expect("underwater");
    assert_eq!(submerged.0, Pose::UnderwaterFloating);
    assert_ne!(surface.1.continuous_start, submerged.1.continuous_start);
    runtime.state = State::Leaving;
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("obstacle leap").0,
        Pose::WaterShoreExit
    );
    runtime.state = State::Flight;
    runtime.water_target = Some((10, 1.0));
    runtime.flight_exit_pose = false;
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("deep ascent before exit").0,
        Pose::SwimmingUp
    );
    runtime.flight_exit_pose = true;
    assert_eq!(
        runtime.swim_signal(now, &MoveInput::default()).expect("flight leap").0,
        Pose::WaterFlightExit
    );
    runtime.water_target = None;
    assert!(runtime.swim_signal(now, &MoveInput::default()).is_none());
}

#[test]
fn surface_double_ascent_requires_two_edges_and_expires_or_cancels_with_context() {
    use swim::Action;
    let now = Instant::now();
    let f = frame();
    let up = MoveInput {
        up: true,
        ..Default::default()
    };
    let idle = MoveInput::default();
    for second_ms in [100, 400, 401, 700] {
        let mut controls = swim::Controls::default();
        assert_eq!(controls.update(true, State::Surface, false, &f, &up, now), Some(Action::Jump));
        assert_eq!(
            controls.update(true, State::Leaving, false, &f, &up, now + Duration::from_millis(20)),
            None
        );
        controls.update(true, State::Leaving, false, &f, &idle, now + Duration::from_millis(50));
        let second = controls.update(true, State::Leaving, false, &f, &up, now + Duration::from_millis(second_ms));
        assert_eq!(second, (second_ms <= 400).then_some(Action::Fly));
    }
    for interruption in 0..6 {
        let mut controls = swim::Controls::default();
        controls.update(true, State::Surface, false, &f, &up, now);
        let mut interrupted = frame();
        interrupted.seated = interruption == 1;
        interrupted.suspended = interruption == 2;
        let state = if interruption == 3 {
            State::Submerged
        } else if interruption == 4 {
            State::Ground
        } else {
            State::Surface
        };
        let input = MoveInput {
            down: interruption == 5,
            ..Default::default()
        };
        controls.update(
            interruption != 0,
            state,
            false,
            &interrupted,
            &input,
            now + Duration::from_millis(50),
        );
        assert_eq!(
            controls.update(true, State::Surface, false, &f, &up, now + Duration::from_millis(150)),
            Some(Action::Jump),
            "interruption {interruption} cannot leave a stale flight gesture"
        );
    }
    let mut controls = swim::Controls::default();
    assert_eq!(controls.update(true, State::Submerged, true, &f, &up, now), Some(Action::Surface));
    controls.update(true, State::Surface, false, &f, &idle, now + Duration::from_millis(50));
    assert_eq!(
        controls.update(true, State::Surface, false, &f, &up, now + Duration::from_millis(100)),
        Some(Action::Jump),
        "an underwater return must not count as the first surface tap"
    );
}
