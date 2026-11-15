//! Grounded third-person framing for seeing our own portal emergence.
//! Body placement remains in the scene; camera preferences are never edited.
use super::*;

pub(super) struct Framing {
    pub focus: Vec3,
    pub offset: Vec3,
    pub weight: f32,
}

pub(super) fn framing(world: &World, anchor: Vec3, heading: Vec3, now: Instant, fov: f32, aspect: f32) -> Option<Framing> {
    if world.portals.reduced_motion || !anchor.is_finite() || !fov.is_finite() || !aspect.is_finite() || aspect <= 0.0 {
        return None;
    }
    let effect = world.portals.arrivals.get(&world.agent_id)?;
    if effect.finished(now) || !effect.matches(world.agent.position) {
        return None;
    }
    // The portal cloud in scene/portals.rs has horizontal radii 1.5/2 m
    // and 0.6 m of vertical depth after rotating the aperture horizontally.
    let bottom = effect.event.floor;
    let plane = effect.center().z;
    let top = plane + 0.6;
    let focus = anchor.with_z((bottom + top) * 0.5);
    let tan = (fov.clamp(0.01, PI - 0.1) * 0.5).tan();
    let distance = 2.5 + ((top - bottom) * 0.6 / tan).max(2.5 / (tan * aspect.max(0.1)));
    let eye_height = (bottom + effect.event.height * 0.6).min(plane - 0.8).max(bottom + 0.5);
    let forward = heading.with_z(0.0).normalize_or(Vec3::X);
    Some(Framing {
        focus,
        offset: -forward * distance + Vec3::Z * (eye_height - focus.z),
        // Keep the emergence framed until the whole body is out, then blend
        // back as the aperture closes, without changing the saved zoom.
        weight: 1.0 - smoothstep((effect.age(now) - effect.event.fall_end()) / 0.45),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    fn fixture() -> (World, Instant) {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.main_region = Some(256);
        world.agent.position = Vec3::new(128.0, 128.0, 25.0);
        let mut portals = std::mem::take(&mut world.portals);
        portals.sync(&world, None, crate::world::portals::unix_time());
        let now = Instant::now();
        assert!(portals.begin_arrival(world.agent_id, world.agent.position, 24.16, 1.8, now));
        world.portals = portals;
        (world, now)
    }

    #[test]
    fn the_emergence_fits_below_the_aperture_across_headings_lenses_aspects_and_frame_rates() {
        let (mut world, now) = fixture();
        let physical = world.agent.position;
        let effect = &world.portals.arrivals[&world.agent_id];
        let (plane, top, bottom, fall_end) = (
            effect.center().z,
            effect.center().z + 0.6,
            effect.event.floor,
            effect.event.fall_end(),
        );
        for (yaw, fov, aspect, fps) in [(0.0, 30.0_f32, 0.6, 30), (1.0, 60.0, 1.55, 60), (2.6, 100.0, 2.0, 144)] {
            world.agent.yaw = yaw;
            let settings = CameraSettings {
                fov: fov.to_radians(),
                ..Default::default()
            };
            let mut camera = Camera::default();
            for i in 0..=fps * 3 {
                let time = i as f32 / fps as f32;
                let input = FrameInput {
                    dt: 1.0 / fps as f32,
                    now: now + Duration::from_secs_f32(time),
                    steering: true,
                    build_mode: false,
                    edit_tracking: None,
                    eye_offset: None,
                    follow_joint_offset: None,
                    draw_distance: 128.0,
                    region_width: REGION_WIDTH,
                    aspect,
                };
                camera.update(&mut world, &settings, &input);
                if (0.55..fall_end + 0.25).contains(&time) {
                    assert!(camera.position.z < plane - 0.6, "the eye must remain under the cloud");
                    assert!((camera.position - physical).dot(world.agent.forward()) < 0.0);
                    let view = camera.proj(aspect) * camera.view();
                    for z in [bottom, top] {
                        for step in 0..8 {
                            let angle = step as f32 * TAU / 8.0;
                            let point = physical.with_z(z) + Vec3::new(angle.cos(), angle.sin(), 0.0) * 2.5;
                            let clip = view * point.extend(1.0);
                            assert!(clip.w > 0.0);
                            let ndc = clip.truncate() / clip.w;
                            assert!(ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0, "{ndc:?}, {fov}, {aspect}, {time}");
                        }
                    }
                }
            }
            assert!(
                framing(
                    &world,
                    physical,
                    world.agent.forward(),
                    now + Duration::from_secs(3),
                    settings.fov,
                    aspect
                )
                .is_none()
            );
            assert!(
                camera.position.z > physical.z + 1.5,
                "ordinary third-person height returns after closure"
            );
            assert_eq!(world.agent.position, physical);
            assert_eq!(world.agent.yaw, yaw);
        }
    }

    #[test]
    fn free_and_first_person_views_keep_their_own_placement_and_reduced_motion_skips_framing() {
        let (mut world, now) = fixture();
        let settings = CameraSettings::default();
        let input = FrameInput {
            dt: 1.0 / 60.0,
            now: now + Duration::from_millis(650),
            steering: false,
            build_mode: false,
            edit_tracking: None,
            eye_offset: None,
            follow_joint_offset: None,
            draw_distance: 128.0,
            region_width: REGION_WIDTH,
            aspect: 1.55,
        };
        let mut camera = Camera::default();
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        camera.update(&mut world, &settings, &input);
        let drop = world.portals.arrival_offset(world.agent_id, world.agent.position, input.now);
        assert!(camera.mouselook());
        assert!(camera.position.z > world.agent.position.z + drop);
        camera.start_free(&mut world.agent, &settings);
        let free = camera.free().unwrap().position;
        camera.update(&mut world, &settings, &input);
        assert_eq!(camera.position, free);
        world.portals.reduced_motion = true;
        assert!(framing(&world, world.agent.position, Vec3::X, input.now, settings.fov, input.aspect).is_none());
    }

    #[test]
    fn arrival_framing_still_obeys_the_simulators_camera_plane() {
        let (mut world, now) = fixture();
        let settings = CameraSettings {
            disable_constraints: false,
            ignore_sim_constraints: false,
            ..Default::default()
        };
        let mut camera = Camera::default();
        camera.set_collide_plane(Vec4::new(1.0, 0.0, 0.0, world.agent.position.x - 1.0), &settings);
        for i in 0..30 {
            camera.update(
                &mut world,
                &settings,
                &FrameInput {
                    dt: 1.0 / 60.0,
                    now: now + Duration::from_millis(i * 16),
                    steering: false,
                    build_mode: false,
                    edit_tracking: None,
                    eye_offset: None,
                    follow_joint_offset: None,
                    draw_distance: 128.0,
                    region_width: REGION_WIDTH,
                    aspect: 1.55,
                },
            );
        }
        assert!(camera.position.x >= world.agent.position.x - 1.0 + CAMERA_COLLIDE_EPSILON - 0.001);
    }
}
