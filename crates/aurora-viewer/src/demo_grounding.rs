//! Offline standing on a slope or two steps, and landing recovery (#667).
use super::*;
use crate::ao::State;
use crate::scene::{
    Scene,
    anim::{BoundAnim, Rig, SkeletonBase},
};
use aurora_assets::anim::{JointMotion, PosKey, RotKey};

fn motion(state: State, mode: &str) -> Uuid {
    if state == State::Standing && mode.contains("-ao") {
        Uuid::from_u128(0x6670_0000_0000_0000_0000_0000_0000_0001)
    } else {
        state.motion()
    }
}

pub fn animation(landing: bool) -> aurora_assets::Animation {
    let mut anim = super::idle_animation();
    for name in ["mAnkleLeft", "mAnkleRight", "mPelvis"] {
        anim.joints.push(JointMotion {
            joint_name: name.into(),
            priority: -1,
            rot_keys: vec![RotKey {
                time: 0.0,
                rotation: Quat::IDENTITY,
            }],
            pos_keys: Vec::new(),
        });
    }
    if landing {
        anim.duration = 2.0;
        anim.loop_out = 2.0;
        anim.looping = false;
        for joint in &mut anim.joints {
            let bend = if joint.joint_name.starts_with("mHip") {
                -0.45
            } else if joint.joint_name.starts_with("mKnee") {
                0.9
            } else {
                continue;
            };
            joint.rot_keys = vec![
                RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_y(bend),
                },
                RotKey {
                    time: 2.0,
                    rotation: Quat::IDENTITY,
                },
            ];
        }
        if let Some(pelvis) = anim.joints.iter_mut().find(|joint| joint.joint_name == "mPelvis") {
            pelvis.pos_keys = vec![
                PosKey {
                    time: 0.0,
                    position: Vec3::new(0.0, 0.0, -0.12),
                },
                PosKey {
                    time: 2.0,
                    position: Vec3::ZERO,
                },
            ];
        }
    }
    anim
}

/// Author the fixture from explicit flat-floor foot paths, independently of
/// the runtime terrain solver. Stance is level; only swing lifts the foot.
pub fn contact_animation(rig: &Rig, base: &SkeletonBase, recovery: bool) -> aurora_assets::Animation {
    let mut clip = animation(false);
    clip.duration = if recovery { 2.0 } else { 1.0 };
    clip.loop_out = clip.duration;
    clip.looping = !recovery;
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
    let pelvis_rest = base.local_pos[rig.pelvis] - rig.local_pos[rig.pelvis];
    let frames = (clip.duration * 60.0) as usize;
    for joint in &mut clip.joints {
        joint.rot_keys.clear();
        joint.pos_keys.clear();
        if joint.joint_name == "mPelvis" {
            // Native recovery tilts this authored channel before blending.
            joint.rot_keys.push(RotKey {
                time: 0.0,
                rotation: Quat::IDENTITY,
            });
        }
    }
    for i in 0..=frames {
        let time = i as f32 / 60.0;
        let fraction = time / clip.duration;
        let pelvis = if recovery {
            -0.12 * (1.0 - fraction).powi(2)
        } else {
            -0.045 - 0.008 * (fraction * std::f32::consts::TAU).sin().powi(2)
        };
        if let Some(joint) = clip.joints.iter_mut().find(|joint| joint.joint_name == "mPelvis") {
            joint.pos_keys.push(PosKey {
                time,
                position: pelvis_rest + Vec3::Z * pelvis,
            });
        }
        for (side, phase) in [("Left", 0.0), ("Right", 0.5)] {
            let [Some(hip), Some(knee), Some(ankle)] =
                ["Hip", "Knee", "Ankle"].map(|bone| rig.names.get(&format!("m{bone}{side}")).copied())
            else {
                continue;
            };
            let p = |j: usize| rest[j].w_axis.truncate();
            let upper = p(knee) - p(hip);
            let lower = p(ankle) - p(knee);
            let (a, b) = (upper.x.hypot(upper.z), lower.x.hypot(lower.z));
            let phase = (fraction + phase) % 1.0;
            let (stride, lift) = if recovery {
                (0.0, 0.0)
            } else if phase < 0.6 {
                (0.21 - 0.42 * phase / 0.6, 0.0)
            } else {
                let flight = (phase - 0.6) / 0.4;
                // Hermite endpoint velocities match the backward-moving
                // stance foot; both position and velocity survive toe-off.
                let tangent = -0.28 * (2.0 * flight.powi(3) - 3.0 * flight * flight + flight);
                (
                    -0.21 + 0.42 * flight * flight * (3.0 - 2.0 * flight) + tangent,
                    0.12 * (std::f32::consts::PI * flight).sin().powi(2),
                )
            };
            let target = p(ankle) + Vec3::new(stride, 0.0, lift) - p(hip) - Vec3::Z * pelvis;
            let distance = target.x.hypot(target.z).clamp((a - b).abs() + 1e-5, a + b - 1e-5);
            let bend = ((distance * distance - a * a - b * b) / (2.0 * a * b)).clamp(-1.0, 1.0).acos();
            let hip_angle = (-target.x).atan2(-target.z)
                - ((a * a + distance * distance - b * b) / (2.0 * a * distance))
                    .clamp(-1.0, 1.0)
                    .acos()
                - (-upper.x).atan2(-upper.z);
            let knee_angle = bend - ((-lower.x).atan2(-lower.z) - (-upper.x).atan2(-upper.z));
            for (bone, angle) in [("Hip", hip_angle), ("Knee", knee_angle), ("Ankle", -hip_angle - knee_angle)] {
                if let Some(joint) = clip.joints.iter_mut().find(|joint| joint.joint_name == format!("m{bone}{side}")) {
                    joint.rot_keys.push(RotKey {
                        time,
                        rotation: Quat::from_rotation_y(angle),
                    });
                }
            }
        }
        for (name, sign) in [("mShoulderLeft", 1.0), ("mShoulderRight", -1.0)] {
            if let Some(joint) = clip.joints.iter_mut().find(|joint| joint.joint_name == name) {
                let swing = if recovery {
                    0.0
                } else {
                    0.22 * (fraction * std::f32::consts::TAU).sin() * sign
                };
                joint.rot_keys.push(RotKey {
                    time,
                    rotation: Quat::from_rotation_x(-1.3 * sign) * Quat::from_rotation_y(swing),
                });
            }
        }
    }
    clip
}

/// Alternate authored foot lifts while standing, independently of scene IK.
pub fn lifting_animation() -> aurora_assets::Animation {
    let mut anim = animation(false);
    anim.duration = 4.0;
    anim.loop_out = 4.0;
    for (side, offset) in [("Left", 0.0), ("Right", 2.0)] {
        for (bone, angle) in [("Hip", -0.55), ("Knee", 1.1), ("Ankle", -0.55)] {
            if let Some(joint) = anim.joints.iter_mut().find(|joint| joint.joint_name == format!("m{bone}{side}")) {
                joint.rot_keys = [
                    (0.0, 0.0),
                    (offset + 0.6, 0.0),
                    (offset + 1.0, angle),
                    (offset + 1.6, 0.0),
                    (4.0, 0.0),
                ]
                .into_iter()
                .map(|(time, angle)| RotKey {
                    time,
                    rotation: Quat::from_rotation_y(angle),
                })
                .collect();
            }
        }
    }
    anim
}

fn origin(mode: &str) -> Vec3 {
    // A frontal camera east of the usual fixture sits inside the plaza deck.
    let x = if mode.contains("-front") { 122.0 } else { 128.0 };
    Vec3::new(x, 128.0, super::floor_at(x, 128.0))
}

pub fn tick(world: &mut crate::world::World, scene: &mut Scene, frame: u64, mode: &str) {
    if frame != 600 {
        return;
    }
    let landing = mode.starts_with("landing");
    let p = origin(mode);
    if frame == 600 {
        let rig = scene.avatar_lib.rig.clone();
        let shape = scene
            .avatar_lib
            .shape_params
            .shape(world.visual_params(&world.agent_id).unwrap_or(&[]), &rig);
        let base = SkeletonBase {
            local_pos: shape.local_pos,
            scale: shape.scale,
        };
        for (id, mut clip) in super::link::animations() {
            for joint in animation(false)
                .joints
                .into_iter()
                .filter(|j| matches!(j.joint_name.as_str(), "mAnkleLeft" | "mAnkleRight" | "mPelvis"))
            {
                if !clip.joints.iter().any(|j| j.joint_name == joint.joint_name) {
                    clip.joints.push(joint);
                }
            }
            scene.anims.insert(id, Arc::new(BoundAnim::bind(clip, &rig)));
        }
        let walk = contact_animation(&rig, &base, false);
        for state in [State::Walking, State::Running, State::CrouchWalk] {
            scene.anims.insert(state.motion(), Arc::new(BoundAnim::bind(walk.clone(), &rig)));
        }
        for state in [State::Standing, State::StandingUp] {
            let clip = if state == State::Standing && mode.contains("-lift") {
                lifting_animation()
            } else if state == State::Standing && mode.contains("-walk") {
                contact_animation(&rig, &base, false)
            } else {
                if state == State::StandingUp {
                    contact_animation(&rig, &base, true)
                } else {
                    animation(false)
                }
            };
            scene.anims.insert(motion(state, mode), Arc::new(BoundAnim::bind(clip, &rig)));
        }
        for state in [State::Jumping, State::Falling, State::Crouching] {
            let mut clip = contact_animation(&rig, &base, true);
            clip.looping = true;
            if state == State::Crouching {
                clip.loop_out = 0.0;
            }
            scene.anims.insert(state.motion(), Arc::new(BoundAnim::bind(clip, &rig)));
        }
        let top = |id, y, height, rotation| {
            prim(
                id,
                p + Vec3::new(0.0, y, height * 0.5),
                rotation,
                Vec3::new(3.0, 1.5, height),
                VolumeParams::default(),
                te([0.4, 0.5, 0.6, 1.0], 0, false, 0.0),
                ExtraParams::default(),
                "",
            )
        };
        let objects = if mode.starts_with("steps") && mode.contains("-rise") {
            // A narrow groove leaves the physical centre lower while both
            // complete soles have higher supports: both knees need a body lift.
            let mut base = top(9380, 0.0, 0.18, Quat::IDENTITY);
            base.scale.y = 3.0;
            let mut sides = [-0.76, 0.76].map(|y| top(if y < 0.0 { 9381 } else { 9382 }, y, 0.4, Quat::IDENTITY));
            for side in &mut sides {
                side.scale.y = 1.48;
            }
            vec![base, sides[0].clone(), sides[1].clone()]
        } else if mode.starts_with("steps") {
            let high = if mode.contains("-range") { 0.9 } else { 0.4 };
            vec![top(9380, -0.75, 0.18, Quat::IDENTITY), top(9381, 0.75, high, Quat::IDENTITY)]
        } else {
            let mut slope = top(9380, 0.0, 0.3, Quat::from_rotation_x(0.3));
            slope.scale.y = 3.0;
            vec![slope]
        };
        world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
    }
    // Straddle the seam from the lower step, so one knee can bend up while
    // the other leg reaches its support without stretching the skeleton.
    let spawn = if mode.starts_with("steps") {
        p + Vec3::Y * if mode.contains("-reach") { 0.005 } else { -0.005 }
    } else {
        p
    };
    let position = spawn.with_z(surface_height(mode, spawn) + half_height(world, scene));
    world
        .agent
        .on_server_update_at(position, Vec3::ZERO, Vec3::ZERO, false, true, std::time::Instant::now());
    world.agent.position = position;
    world.agent.yaw = 0.0;
    world.agent.velocity = Vec3::ZERO;
    // Standing bodies retain a small heading difference from the agent.
    // Seed the rendered heading too, so both sole footprints straddle the
    // seam. Subsequent frames keep ordinary movement and body turning.
    world.bodies.insert(world.agent_id, crate::world::body::Body::new(Vec3::X));
    if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
        avatar.position = world.agent.position;
        avatar.rotation = Quat::IDENTITY;
        avatar.velocity = Vec3::ZERO;
        avatar.foot_plane = Some(contact_plane(mode, spawn));
        if let Some(te) = &mut avatar.te
            && let Some(skirt) = Arc::make_mut(te).faces.get_mut(19)
        {
            // Expose both knees and ankles in the offline comparison fixture.
            skirt.texture = crate::scene::avatar::IMG_INVISIBLE;
            avatar.shape_dirty = true;
        }
    }
    // AvatarAppearance overrides the object TE after the demo's appearance
    // setup. Update that rendered bake source as well, preserving the shape.
    if let Some(mut appearance) = world.server_appearance().cloned() {
        if let Some(te) = &mut appearance.texture_entry
            && let Some(skirt) = Arc::make_mut(te).faces.get_mut(19)
        {
            skirt.texture = crate::scene::avatar::IMG_INVISIBLE;
        }
        // Own appearances at the current COF version are deliberately ignored.
        // Publish a newer simulated appearance, as a real outfit change does.
        appearance.cof_version = appearance.cof_version.max(world.outfit.stepped_cof_version()).saturating_add(1);
        world.apply(NetEvent::Appearance(appearance));
    }
    let agent = world.agent_id;
    world.refresh_bakes(&agent);
    // Combining this terrain fixture with native Movelock must retain the
    // confirmed ground sit. Replacing its raw signal would stand the avatar
    // up behind the anchor controller's back and invalidate the scenario.
    if !world.agent.ground_sit {
        let state = if landing { State::StandingUp } else { State::Standing };
        // Restart the fixture's stand after replacing its tracks. A sequence
        // change alone preserves an uninterrupted loop and its loaded asset.
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: Vec::new(),
            sources: Vec::new(),
        });
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(motion(state, mode), frame as i32)],
            sources: Vec::new(),
        });
    }
}

fn half_height(world: &crate::world::World, scene: &Scene) -> f32 {
    scene.automatic_hover_box_height(world).map_or(0.84, |height| height * 0.5)
}

/// Exact top planes of this fixture's two boxes or rotated box. Movement
/// samples the same geometry as IK; the rest of the demo keeps its own floor.
pub fn surface_height(mode: &str, position: Vec3) -> f32 {
    let origin = origin(mode);
    let d = position - origin;
    let terrain = super::floor_at(position.x, position.y);
    if d.x.abs() > 1.5 {
        return terrain;
    }
    if mode.starts_with("steps") {
        if d.y.abs() <= 1.5 {
            let low = if mode.contains("-rise") { d.y.abs() < 0.02 } else { d.y < 0.0 };
            let high = if mode.contains("-range") { 0.9 } else { 0.4 };
            terrain.max(origin.z + if low { 0.18 } else { high })
        } else {
            terrain
        }
    } else {
        let (sin, cos) = 0.3_f32.sin_cos();
        let z = 0.15 + (0.15 + sin * d.y) / cos;
        let local_y = cos * d.y + sin * (z - 0.15);
        if local_y.abs() <= 1.5 { terrain.max(origin.z + z) } else { terrain }
    }
}

fn contact_plane(mode: &str, position: Vec3) -> glam::Vec4 {
    let height = surface_height(mode, position);
    let normal = if !mode.starts_with("steps") && height > super::floor_at(position.x, position.y) + 0.001 {
        Vec3::new(0.0, -0.3_f32.sin(), 0.3_f32.cos())
    } else {
        Vec3::Z
    };
    // Emulate the simulator's common foot plane, including the documented
    // 5 cm contact bias. It intentionally cannot describe two separate steps.
    normal.extend(normal.dot(position.with_z(height)) + 0.05)
}

/// Compensate only this fixture for simulate_locally's fixed 0.84 m half
/// height, so its shaped physical centre and both support traces agree.
pub fn simulation_floor(world: &crate::world::World, scene: &Scene, mode: &str) -> f32 {
    surface_height(mode, world.agent.server_pos) + half_height(world, scene) - 0.84
}

pub fn update_motion(world: &mut crate::world::World, scene: &Scene, frame: u64, mode: &str, input: &crate::agent::MoveInput) {
    if frame < 600 || world.agent.is_sitting() {
        return;
    }
    let position = world.agent.server_pos;
    let velocity = world.agent.velocity;
    let moving = velocity.truncate().length_squared() > 0.01;
    let airborne = position.z - half_height(world, scene) > surface_height(mode, position) + 0.05;
    if let Some(avatar) = world
        .objects
        .index_of_uuid(&world.agent_id)
        .and_then(|idx| world.objects.get_mut(idx))
    {
        avatar.foot_plane = (!airborne && !world.agent.flying).then(|| contact_plane(mode, position));
    }
    let current = world.animations.get(&world.agent_id).and_then(|motions| motions.first()).copied();
    let recovering =
        current.is_some_and(|motion| motion.id == State::StandingUp.motion() && motion.sequence_start.elapsed().as_secs_f32() < 2.0);
    let landed = current.is_some_and(|motion| [State::Jumping.motion(), State::Falling.motion()].contains(&motion.id));
    let state = if world.agent.flying {
        if velocity.z > 0.1 {
            State::FlyingUp
        } else if velocity.z < -0.1 {
            State::FlyingDown
        } else if moving {
            State::Flying
        } else {
            State::Hovering
        }
    } else if airborne {
        if velocity.z > 0.0 { State::Jumping } else { State::Falling }
    } else if moving {
        if input.down {
            State::CrouchWalk
        } else if velocity.truncate().length() > 4.0 {
            State::Running
        } else {
            State::Walking
        }
    } else if input.down {
        State::Crouching
    } else if landed || recovering {
        State::StandingUp
    } else {
        State::Standing
    };
    if current.is_none_or(|playing| playing.id != motion(state, mode)) {
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(motion(state, mode), frame as i32)],
            sources: Vec::new(),
        });
    }
}

/// Explicit offline input fixture; ordinary modes always keep human controls.
pub fn scripted_input(mode: &str, frame: u64) -> Option<crate::agent::MoveInput> {
    mode.contains("-motion").then(|| crate::agent::MoveInput {
        strafe_left: (720..744).contains(&frame),
        strafe_right: (880..904).contains(&frame),
        up: (1040..1042).contains(&frame),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::MoveInput;
    use crate::world::World;

    fn fixture() -> (World, Scene) {
        let lib = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in super::super::events() {
            world.apply(event);
        }
        let mut scene = Scene::new(std::path::PathBuf::new(), lib);
        tick(&mut world, &mut scene, 600, "steps");
        (world, scene)
    }

    fn advance(world: &mut World, scene: &mut Scene, frame: u64, input: MoveInput) {
        let floor = simulation_floor(world, scene, "steps");
        world.agent.simulate_locally(&input, false, Some(floor), 1.0 / 60.0);
        world.agent.update(&input, false, 1.0 / 60.0, 4.0);
        tick(world, scene, frame, "steps");
        update_motion(world, scene, frame, "steps", &input);
    }

    #[test]
    fn grounding_comparison_exposes_legs_with_an_appearance_bake_override() {
        let (mut world, mut scene) = fixture();
        let params = scene.avatar_lib.shape_params.values_with(&[]);
        world.apply(NetEvent::Appearance(aurora_net::AvatarAppearance {
            avatar_id: world.agent_id,
            texture_entry: Some(Arc::new(aurora_prim::te::TextureEntry::default())),
            visual_params: params.clone(),
            cof_version: 7,
            appearance_version: 1,
            hover_height: 0.03,
            attachments: 0,
        }));
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("our avatar");
        assert_ne!(world.bake_texture(idx, 19), Some(crate::scene::avatar::IMG_INVISIBLE));
        tick(&mut world, &mut scene, 600, "steps");
        assert_eq!(world.bake_texture(idx, 19), Some(crate::scene::avatar::IMG_INVISIBLE));
        assert_eq!(world.visual_params(&world.agent_id), Some(params.as_slice()));
        assert_eq!(world.appearance_cof_version(&world.agent_id), 8);
        assert_eq!(world.server_appearance().expect("received appearance").hover_height, 0.03);
    }

    #[test]
    fn grounding_demo_keeps_human_position_yaw_and_movement_after_initial_spawn() {
        let (mut world, mut scene) = fixture();
        world
            .bodies
            .insert(world.agent_id, crate::world::body::Body::new(Vec3::new(1.0, 0.5, 0.0)));
        tick(&mut world, &mut scene, 600, "steps");
        assert!(world.bodies[&world.agent_id].rotation.abs_diff_eq(Quat::IDENTITY, 1e-6));
        let spawn = world.agent.position;
        world.agent.yaw = 0.6;
        for frame in 601..621 {
            advance(
                &mut world,
                &mut scene,
                frame,
                MoveInput {
                    forward: true,
                    ..Default::default()
                },
            );
        }
        assert!(world.agent.position.distance(spawn) > 0.5, "walking must survive the fixture tick");
        assert!((world.agent.yaw - 0.6).abs() < 0.001, "the fixture must not reset orientation");
        assert_eq!(world.animations[&world.agent_id][0].id, State::Walking.motion());
        advance(&mut world, &mut scene, 622, MoveInput::default());
        let stopped = world.animations[&world.agent_id][0];
        assert_eq!(stopped.id, State::Standing.motion());
        advance(&mut world, &mut scene, 623, MoveInput::default());
        assert_eq!(
            world.animations[&world.agent_id][0], stopped,
            "a stable state must not restart every frame"
        );
        assert!(scripted_input("steps", 725).is_none());
        assert!(scripted_input("steps-motion", 725).unwrap().strafe_left);
    }

    #[test]
    fn grounding_demo_uses_the_rendered_top_planes_and_releases_their_edges() {
        let p = Vec3::new(128.0, 128.0, super::super::floor_at(128.0, 128.0));
        assert!((surface_height("steps", p - Vec3::Y * 0.4) - p.z - 0.18).abs() < 1e-4);
        assert!((surface_height("steps", p + Vec3::Y * 0.4) - p.z - 0.4).abs() < 1e-4);
        let outside = p + Vec3::X * 2.0;
        assert_eq!(surface_height("steps", outside), super::super::floor_at(outside.x, outside.y));
        let at = p + Vec3::Y * 0.4;
        let top = at.with_z(surface_height("slope", at));
        let normal = Quat::from_rotation_x(0.3) * Vec3::Z;
        assert!(((top - p - Vec3::Z * 0.15).dot(normal) - 0.15).abs() < 1e-4);
    }

    #[test]
    fn grounding_demo_front_camera_fixture_matches_its_rendered_platforms_and_physics() {
        let (mut world, mut scene) = fixture();
        for mode in ["steps-reach-front", "steps-rise-front"] {
            tick(&mut world, &mut scene, 600, mode);
            assert!(
                (world.agent.position.x - 122.0).abs() < 1e-4,
                "frontal camera must stay clear of the plaza deck"
            );
            let ids: &[u32] = if mode.contains("-rise") {
                &[9380, 9381, 9382]
            } else {
                &[9380, 9381]
            };
            for local_id in ids {
                let platform = world
                    .objects
                    .iter()
                    .find(|(_, object)| object.key.local_id == *local_id)
                    .map(|(_, object)| object)
                    .expect("step");
                let top = platform.position.z + platform.scale.z * 0.5;
                assert!(
                    (surface_height(mode, platform.position) - top).abs() < 1e-4,
                    "local physics missed the moved platform"
                );
            }
            let floor = surface_height(mode, world.agent.position) + half_height(&world, &scene);
            assert!((world.agent.position.z - floor).abs() < 1e-4);
        }
    }

    #[test]
    fn grounding_demo_jump_returns_to_known_support_and_triggers_recovery() {
        let (mut world, mut scene) = fixture();
        let start = world.agent.server_pos.z;
        let mut peak = start;
        let mut states = Vec::new();
        for frame in 601..721 {
            advance(
                &mut world,
                &mut scene,
                frame,
                MoveInput {
                    up: frame == 601,
                    ..Default::default()
                },
            );
            peak = peak.max(world.agent.server_pos.z);
            states.push(world.animations[&world.agent_id][0].id);
        }
        assert!(peak > start + 0.8);
        assert!(states.contains(&State::Jumping.motion()));
        assert!(states.contains(&State::Falling.motion()));
        assert!(states.contains(&State::StandingUp.motion()));
        let support = surface_height("steps", world.agent.server_pos) + half_height(&world, &scene);
        // The existing local simulator considers the last 5 cm grounded.
        assert!((world.agent.server_pos.z - support).abs() <= 0.05);
    }
}
