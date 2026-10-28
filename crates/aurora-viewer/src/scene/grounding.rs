//! Render-space collision queries for pose-space foot placement.
//! Exact solid geometry and terrain, with streamed contact as a fallback.

use super::anim::grounding::{Frame, Surface};
use super::*;

impl Scene {
    pub fn set_foot_scope_limit(&mut self, maximum: u32) {
        self.foot_scope.configure(maximum);
    }

    pub fn update_foot_scope(&mut self, world: &World, view: CullView, now: Instant) {
        if !self.ground_avatar_feet || crate::settings::foot_height_limit(self.ground_feet_max_height) == 0.0 {
            self.foot_scope.clear_selection();
            return;
        }
        let origin = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|index| Self::object_transform(world, index, now, 0))
            .map_or(world.agent.position, |transform| transform.0);
        let hidden = &self.too_complex;
        let candidates = world.objects.avatars().filter_map(|(index, avatar)| {
            if avatar.full_id == world.agent_id || avatar.parent_id != 0 || hidden.contains(&index) {
                return None;
            }
            let position = Self::object_transform(world, index, now, 0)?.0;
            Some((avatar.full_id, position, (avatar.scale.z.abs() * 0.5).clamp(0.25, 3.0)))
        });
        self.foot_scope.select(candidates, origin, view);
    }

    pub(super) fn grounding_frame(&self, world: &World, idx: usize, now: Instant) -> Option<Frame> {
        let object = world.objects.get(idx)?;
        let own = object.full_id == world.agent_id;
        let standing_anchor = own && self.ground_feet_native_stand && world.agent.ground_sit && !world.agent.seated;
        if !self.ground_avatar_feet
            || crate::settings::foot_height_limit(self.ground_feet_max_height) == 0.0
            || !object.is_avatar()
            || object.parent_id != 0
            || (own && (world.agent.flying || (world.agent.is_sitting() && !standing_anchor) || world.animation_preview_clock.is_some()))
            || (!own && self.grounding_sole_height(world, object.full_id).is_none())
            || (!own && !self.foot_scope.allows(object.full_id))
        {
            return None;
        }
        if !self.ground_feet_while_moving {
            let moving_clip = |id| {
                matches!(
                    crate::ao::State::from_motion(id),
                    Some(crate::ao::State::Walking | crate::ao::State::Running | crate::ao::State::CrouchWalk)
                )
            };
            // Stock clips cover walking in place; velocity also covers custom
            // AOs and pushes. Include the drawn stop glide, with a small noise
            // tolerance. An owned standing anchor ignores stale raw locomotion.
            let signals = if own {
                world.own_signals()
            } else {
                world.animations_of(&object.full_id)
            };
            let velocity = if own { world.agent.velocity } else { object.velocity };
            let moving = (own && self.ground_feet_own_moving)
                || (!standing_anchor
                    && (signals.iter().any(|a| moving_clip(a.id))
                        || velocity.truncate().length_squared() > 0.05f32.powi(2)
                        || (own && world.agent.settling_velocity().truncate().length_squared() > 0.05f32.powi(2))));
            if moving {
                return None;
            }
        }
        self.support_frame(world, idx, now, standing_anchor)
    }

    pub(super) fn legacy_grounding_frame(&self, world: &World, idx: usize, now: Instant) -> Option<Frame> {
        let object = world.objects.get(idx)?;
        let own = object.full_id == world.agent_id;
        let standing_anchor = own && self.ground_feet_native_stand && world.agent.ground_sit && !world.agent.seated;
        if !object.is_avatar()
            || object.parent_id != 0
            || (own && (world.agent.flying || (world.agent.is_sitting() && !standing_anchor) || world.animation_preview_clock.is_some()))
        {
            return None;
        }
        self.support_frame(world, idx, now, standing_anchor)
    }

    fn support_frame(&self, world: &World, idx: usize, now: Instant, standing_anchor: bool) -> Option<Frame> {
        let object = world.objects.get(idx)?;
        let (position, rotation, _) = Self::object_transform(world, idx, now, 0)?;
        let velocity_z = if object.full_id == world.agent_id {
            world.agent.velocity.z
        } else {
            object.velocity.z
        };
        let contact = object.foot_plane.filter(|p| p.is_finite() && p.z > 0.1).map(|mut plane| {
            // Simulator planes are region-relative; queries use render space.
            plane.w += plane.truncate().dot(world.region_offset(object.key.region).unwrap_or(Vec3::ZERO));
            plane
        });
        let to_world = Mat4::from_rotation_translation(rotation, position) * Mat4::from_translation(-self.avatar_lib.pelvis);
        (to_world.is_finite() && velocity_z.is_finite()).then_some(Frame {
            to_world,
            contact,
            velocity_z,
            sole_height: self.grounding_sole_height(world, object.full_id),
            max_height: crate::settings::foot_height_limit(self.ground_feet_max_height),
            standing_anchor,
        })
    }

    /// LLWorld::resolveStepHeightGlobal: terrain and the simulator's shared
    /// foot plane only. No object candidates, triangle traces, or shoe probes.
    pub(super) fn legacy_ground_surface(&self, world: &World, frame: Frame, point: Vec3) -> Option<Surface> {
        let terrain = world.ground_height(point).filter(|z| z.is_finite()).map(|z| Surface {
            position: point.with_z(z),
            normal: terrain_normal(world, point),
        });
        let contact = frame.contact.and_then(|plane| {
            let z = (plane.w - plane.x * point.x - plane.y * point.y - 0.05) / plane.z;
            z.is_finite().then_some(Surface {
                position: point.with_z(z.clamp(point.z - 1.0, point.z + 1.0)),
                normal: plane.truncate().normalize_or(Vec3::Z),
            })
        });
        match (terrain, contact) {
            (Some(a), Some(b)) => Some(if a.position.z >= b.position.z { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    pub(super) fn ground_surface(&self, world: &World, frame: Frame, ankle: Vec3, now: Instant) -> Option<Surface> {
        let reach = crate::settings::MAX_GROUND_FEET_HEIGHT + 0.002;
        let ray = (ankle + Vec3::Z * reach, -Vec3::Z);
        let mut candidates = Vec::new();
        self.pick.ray_candidates(ray, &mut candidates);
        let hit = candidates
            .iter()
            .filter_map(|&idx| {
                let object = world.objects.get(idx)?;
                let gpu = self.gpu.get(idx)?;
                if !Self::foot_support_candidate(gpu, object) {
                    return None;
                }
                // (the highest level: the drawn one follows the camera, #1281)
                self.support_hit(world, idx, ray, now)
                    .filter(|hit| hit.t.is_finite() && (0.0..=reach * 2.0).contains(&hit.t) && hit.normal.is_finite() && hit.normal.z > 0.1)
            })
            .min_by(|a, b| a.t.total_cmp(&b.t));
        let terrain = world
            .ground_height(ankle)
            .filter(|z| z.is_finite() && (ankle.z - z).abs() <= reach)
            .map(|z| Surface {
                position: ankle.with_z(z),
                normal: terrain_normal(world, ankle),
            });
        let object = hit.map(|hit| Surface {
            position: ray.0 + ray.1 * hit.t,
            normal: hit.normal.normalize_or(Vec3::Z),
        });
        // Exact steps take precedence over extending one simulator contact
        // plane across both feet. Contact is useful while geometry is loading.
        let support = match (object, terrain) {
            (Some(a), Some(b)) => Some(if a.position.z >= b.position.z { a } else { b }),
            (a, b) => a.or(b),
        };
        if object.is_some() {
            return support;
        }
        let contact = frame.contact.and_then(|p| {
            let z = (p.w - p.x * ankle.x - p.y * ankle.y - 0.05) / p.z;
            (z.is_finite() && (ankle.z - z).abs() <= reach).then_some(Surface {
                position: ankle.with_z(z),
                normal: p.truncate().normalize_or(Vec3::Z),
            })
        });
        match (support, contact) {
            (Some(a), Some(b)) => Some(if a.position.z >= b.position.z { a } else { b }),
            (a, b) => a.or(b),
        }
    }
}

fn terrain_normal(world: &World, p: Vec3) -> Vec3 {
    let height = world.ground_height(p).unwrap_or(p.z);
    let sample = |offset| world.ground_height(p + offset).unwrap_or(height);
    Vec3::new(
        sample(-Vec3::X * 0.1) - sample(Vec3::X * 0.1),
        sample(-Vec3::Y * 0.1) - sample(Vec3::Y * 0.1),
        0.2,
    )
    .normalize_or(Vec3::Z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    #[test]
    fn grounding_support_uses_exact_steps_and_excludes_phantom_or_worn_geometry() {
        let (mut world, mut scene, front, _) = super::super::picking::tests::scene_with_two_objects();
        let object = world.objects.get_mut(front).expect("platform");
        object.update_flags = 0;
        object.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        scene.gpu[front].center = Vec3::new(3.0, 0.0, 50.0);
        scene.gpu[front].radius = 1.0;
        scene.pick.set(front, hover::PickEntry::of(&scene.gpu[front]));
        let mut frame = Frame {
            to_world: Mat4::IDENTITY,
            contact: Some(Vec4::new(0.0, 0.0, 1.0, 50.9)),
            velocity_z: 0.0,
            sole_height: None,
            max_height: crate::settings::DEFAULT_GROUND_FEET_HEIGHT,
            standing_anchor: false,
        };
        let ankle = Vec3::new(3.0, 0.0, 50.7);
        let now = Instant::now();
        let support = scene.ground_surface(&world, frame, ankle, now).expect("platform top");
        let native = scene.legacy_ground_surface(&world, frame, ankle).expect("native contact");
        assert!(
            (native.position.z - 50.85).abs() < 0.001,
            "native support must ignore exact object geometry"
        );
        assert!(
            (support.position.z - 50.5).abs() < 0.001,
            "exact step overrides one higher contact plane"
        );
        assert!(support.normal.z > 0.99);
        world.objects.get_mut(front).unwrap().update_flags = crate::build::edits::FLAGS_PHANTOM;
        let contact = scene.ground_surface(&world, frame, ankle, now).expect("contact fallback");
        assert!((contact.position.z - 50.85).abs() < 0.001);
        frame.contact = None;
        assert!(scene.ground_surface(&world, frame, ankle, now).is_none());
        world.objects.get_mut(front).unwrap().update_flags = 0;
        scene.gpu[front].owner_avatar = Some(front);
        assert!(scene.ground_surface(&world, frame, ankle, now).is_none());
    }

    #[test]
    fn grounding_movement_scope_covers_signals_custom_motion_and_immediate_input() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, _, _) = super::super::picking::tests::scene_with_two_objects();
        scene.foot_scope = super::super::foot_scope::Scope::from_mode("all");
        let now = Instant::now();
        world.agent.flying = false;
        scene.ground_avatar_feet = true;
        let own = world.objects.index_of_uuid(&world.agent_id).expect("own avatar");
        let remote = world
            .objects
            .iter()
            .find(|(_, o)| o.is_avatar() && o.full_id != world.agent_id)
            .map(|(i, _)| i)
            .expect("other avatar");
        let remote_id = world.objects.get(remote).expect("remote").full_id;
        for id in [world.agent_id, remote_id] {
            scene.palette_base(id);
        }
        scene.update_poses(&mut world, Vec3::ZERO, now);
        for delay in [0, 250, 500] {
            scene.update_auto_hover(&mut world, false, 0.0, true, now + std::time::Duration::from_millis(delay));
        }
        for (idx, id) in [(own, world.agent_id), (remote, remote_id)] {
            for state in [crate::ao::State::Walking, crate::ao::State::Running, crate::ao::State::CrouchWalk] {
                world
                    .animations
                    .insert(id, vec![crate::world::PlayingAnimation::from_signal(state.motion(), 1, now, None)]);
                assert!(scene.grounding_frame(&world, idx, now).is_none(), "locomotion should be opt-in");
                scene.ground_feet_while_moving = true;
                assert!(scene.grounding_frame(&world, idx, now).is_some());
                scene.ground_feet_while_moving = false;
            }
            world.animations.insert(id, Vec::new());
            if idx == own {
                world.agent.velocity = Vec3::X * 0.2;
            } else {
                world.objects.get_mut(idx).expect("remote").velocity = Vec3::X * 0.2;
            }
            assert!(scene.grounding_frame(&world, idx, now).is_none(), "custom AO or a push still moves");
            if idx == own {
                world.agent.velocity = Vec3::ZERO;
            } else {
                world.objects.get_mut(idx).expect("remote").velocity = Vec3::ZERO;
            }
            world.animations.insert(
                id,
                vec![crate::world::PlayingAnimation::from_signal(
                    crate::ao::State::TurningLeft.motion(),
                    1,
                    now,
                    None,
                )],
            );
            assert!(
                scene.grounding_frame(&world, idx, now).is_some(),
                "turning in place retains support"
            );
        }
        scene.ground_feet_own_moving = true;
        assert!(scene.grounding_frame(&world, own, now).is_none(), "input precedes velocity");
        assert!(
            scene.grounding_frame(&world, remote, now).is_some(),
            "own input does not suppress others"
        );
        scene.ground_feet_own_moving = false;
        assert!(scene.grounding_frame(&world, own, now).is_some(), "support returns at rest");
    }

    #[test]
    fn grounding_native_standing_anchor_is_not_a_manual_or_object_seat() {
        let (mut world, mut scene, _, _) = super::super::picking::tests::scene_with_two_objects();
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("own avatar");
        let now = Instant::now();
        scene.ground_avatar_feet = true;
        world.agent.flying = false;
        world.agent.ground_sit = true;
        assert!(scene.grounding_frame(&world, idx, now).is_none(), "manual ground sit");
        scene.ground_feet_native_stand = true;
        world.agent.velocity = Vec3::new(0.2, 0.0, -2.0);
        assert!(
            scene
                .grounding_frame(&world, idx, now)
                .expect("owned standing anchor")
                .standing_anchor
        );
        scene.ground_feet_own_moving = true;
        assert!(
            scene.grounding_frame(&world, idx, now).is_none(),
            "voluntary departure still releases IK"
        );
        scene.ground_feet_own_moving = false;
        world.agent.seated = true;
        assert!(scene.grounding_frame(&world, idx, now).is_none(), "an actual seat always wins");
        world.agent.seated = false;
        world.objects.get_mut(idx).expect("own").parent_id = 970;
        assert!(scene.grounding_frame(&world, idx, now).is_none());
        world.objects.get_mut(idx).expect("own").parent_id = 0;
        world.agent.flying = true;
        assert!(scene.grounding_frame(&world, idx, now).is_none());
        world.agent.flying = false;
        world.agent.ground_sit = false;
        world.agent.velocity = Vec3::ZERO;
        assert!(
            !scene.grounding_frame(&world, idx, now).expect("standing normally").standing_anchor,
            "stale anchor flag must not escape ground sit"
        );
    }

    #[test]
    fn grounding_frame_respects_disable_seating_flight_and_preview() {
        let (mut world, mut scene, _, _) = super::super::picking::tests::scene_with_two_objects();
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("own avatar");
        let now = Instant::now();
        world.agent.flying = false;
        assert!(scene.grounding_frame(&world, idx, now).is_none(), "placement defaults off");
        assert!(
            scene.legacy_grounding_frame(&world, idx, now).is_some(),
            "native placement defaults on"
        );
        scene.ground_avatar_feet = true;
        assert!(scene.grounding_frame(&world, idx, now).is_some());
        scene.ground_feet_max_height = 1.0;
        assert_eq!(scene.grounding_frame(&world, idx, now).expect("enabled").max_height, 1.0);
        scene.ground_feet_max_height = 0.0;
        assert!(scene.grounding_frame(&world, idx, now).is_none());
        assert!(
            scene.legacy_grounding_frame(&world, idx, now).is_some(),
            "the Aurora height limit does not gate native context"
        );
        scene.ground_feet_max_height = 0.4;
        scene.ground_avatar_feet = false;
        assert!(scene.grounding_frame(&world, idx, now).is_none());
        scene.ground_avatar_feet = true;
        world.agent.flying = true;
        assert!(scene.grounding_frame(&world, idx, now).is_none());
        assert!(scene.legacy_grounding_frame(&world, idx, now).is_none());
        world.agent.flying = false;
        world.objects.get_mut(idx).unwrap().parent_id = 970;
        assert!(scene.grounding_frame(&world, idx, now).is_none());
        assert!(scene.legacy_grounding_frame(&world, idx, now).is_none());
        world.objects.get_mut(idx).unwrap().parent_id = 0;
        world.animation_preview_clock = Some((Uuid::nil(), now, now));
        assert!(scene.grounding_frame(&world, idx, now).is_none());
        assert!(scene.legacy_grounding_frame(&world, idx, now).is_none());
    }
}
