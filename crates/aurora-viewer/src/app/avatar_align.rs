//! Face nearest avatar and the Compass, applied: the agent's heading set at
//! once, the drawn bodies snapped and an AgentUpdate forced, as
//! FSAvatarAlignBase::applyRotation does (LLAgent::resetAxes,
//! send_agent_update(true, false), snapAvatarBody, snapRemoteAvatarBody;
//! indra/newview/fsfloateravataralign.cpp, originally LGPL 2.1). The rules
//! are `crate::avatar_align`, the window `ui/avatar_align.rs`.
use super::*;
use crate::avatar_align as align;

/// snapAvatarBody for an avatar sitting on an object: it is drawn facing
/// `yaw` (upright) now, through its seat, until the simulator reports its
/// own rotation. Returns whether `id` sits on a seat that is known.
fn snap_seated(world: &mut World, id: uuid::Uuid, yaw: f32) -> bool {
    let now = Instant::now();
    let Some(idx) = world.objects.index_of_uuid(&id) else {
        return false;
    };
    let Some((parent_id, reported, seat)) = world
        .objects
        .get(idx)
        .filter(|o| o.parent_id != 0)
        .and_then(|o| Some((o.parent_id, o.rotation, world.objects.parent_of(o)?)))
    else {
        return false;
    };
    let Some((_, seat_rotation, _)) = crate::scene::Scene::object_transform_unshifted(world, seat, now) else {
        return false;
    };
    world.seat_snaps.insert(
        id,
        crate::world::body::SeatSnap {
            parent_id,
            reported,
            local: align::seat_local_rotation(seat_rotation, yaw),
        },
    );
    true
}

impl App {
    /// Each frame: the Compass, while open.
    pub(super) fn avatar_align_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo {
            self.demo_avatar_align();
        }
        self.panels.avatar_align_mini = self.settings.avatar_align_mini;
        let mini = self.settings.avatar_align_mini;
        let Some(action) = ui::avatar_align::show(ctx, p, &mut self.panels.avatar_align, mini, self.world.agent.yaw) else {
            return;
        };
        self.avatar_align_action(action);
    }

    fn avatar_align_action(&mut self, action: ui::avatar_align::Action) {
        use ui::avatar_align::Action;
        let yaw = self.world.agent.yaw;
        match action {
            Action::Bearing(bearing) => self.turn_agent(align::yaw_of_bearing(bearing), None),
            Action::Rotate(delta) => self.turn_agent(align::rotated(yaw, delta), None),
            Action::NearestCardinal => self.turn_agent(align::nearest_cardinal(yaw), None),
            Action::FaceNearest => self.face_nearest_avatar(),
            Action::ToggleMode => {
                self.settings.avatar_align_mini = !self.settings.avatar_align_mini;
                self.panels.avatar_align_mini = self.settings.avatar_align_mini;
                self.panels.avatar_align = true;
                self.settings.save();
            }
        }
    }

    /// onClickFaceNearestAvatar: turn to the nearest avatar within 20 m,
    /// animesh and ourselves left out; nothing but a log line without one.
    pub(super) fn face_nearest_avatar(&mut self) {
        if !self.in_world() {
            return;
        }
        let world = &self.world;
        let avatars = world
            .objects
            .avatars()
            .filter(|(_, o)| !o.full_id.is_nil() && o.full_id != world.agent_id)
            // where each is drawn now, seat chain included: the stored
            // position of a seated avatar is relative to its seat
            .filter_map(|(_, o)| crate::autopilot::avatar_position(world, o.full_id).map(|at| (o.full_id, at)));
        let me = world.agent.position;
        let Some((id, at)) = align::nearest(me, avatars) else {
            log::warn!("avatar align: no nearby avatar found to face");
            return;
        };
        let Some(yaw) = align::yaw_towards(me, at) else {
            return;
        };
        self.turn_agent(yaw, Some(id));
    }

    /// applyRotation: the agent faces `yaw` now, our drawn body with it
    /// (no turn animation), `facing`'s body snapped to its own rotation,
    /// and the simulator told at once.
    fn turn_agent(&mut self, yaw: f32, facing: Option<uuid::Uuid>) {
        let before = align::bearing(self.world.agent.yaw);
        self.world.agent.yaw = yaw;
        let agent = self.world.agent_id;
        let forward = self.world.agent.forward();
        if let Some(body) = self.world.bodies.get_mut(&agent) {
            body.snap(forward);
        } else {
            // seated: parented avatars have no standing body, the drawn
            // rotation is the seat's through the avatar's own
            snap_seated(&mut self.world, agent, yaw);
        }
        if let Some(id) = facing {
            let now = Instant::now();
            let heading = self
                .world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| self.world.objects.get(i))
                .map(|o| self.world.objects.predicted(o, now).1 * Vec3::X);
            if let (Some(heading), Some(body)) = (heading, self.world.bodies.get_mut(&id)) {
                body.snap(heading);
            }
        }
        self.force_agent_update = true;
        log::info!(
            "avatar align: bearing {before:.1}° -> {:.1}°{}",
            align::bearing(yaw),
            if facing.is_some() { " (facing the nearest avatar)" } else { "" }
        );
    }

    /// AURORA_DEMO_AVATAR_ALIGN: the commands run one after the other, each
    /// logging the bearing before and after (`avatar align:` lines).
    fn demo_avatar_align(&mut self) {
        let Some(mode) = crate::demo::avatar_align::mode() else {
            return;
        };
        let frame = self.frame_count;
        if frame == crate::demo::avatar_align::OPEN_FRAME && crate::demo::avatar_align::opens(&mode) {
            if mode == "mini" {
                self.settings.avatar_align_mini = true;
            }
            let window = ui::avatar_align::window(self.settings.avatar_align_mini);
            ui::widgets::place_floater(
                &self.egui_ctx,
                window,
                egui::pos2(40.0, 120.0),
                if mode == "mini" {
                    egui::vec2(150.0, 210.0)
                } else {
                    egui::vec2(240.0, 420.0)
                },
            );
            ui::widgets::open_floater(&self.egui_ctx, window, &mut self.panels.avatar_align);
        }
        if let Some(step) = crate::demo::avatar_align::step(&mode, frame) {
            log::info!("demo: avatar align step {step:?} at frame {frame}");
            match step {
                crate::demo::avatar_align::Step::Toolbar(command) => {
                    let action = ui::bars::run_command(&self.egui_ctx, command, &mut self.panels);
                    if matches!(action, ui::bars::BarAction::FaceNearestAvatar) {
                        self.face_nearest_avatar();
                    }
                }
                crate::demo::avatar_align::Step::Window(action) => self.avatar_align_action(action),
                crate::demo::avatar_align::Step::Yaw(yaw) => self.world.agent.yaw = yaw,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo, scene::Scene, scene::avatar::AvatarLibrary};
    use aurora_net::NetEvent;
    use glam::{Quat, Vec3};
    use std::sync::Arc;

    /// Our avatar seated on a seat turned 90° and tilted: after a turn it is
    /// drawn upright, facing the asked heading, and a new report from the
    /// simulator takes over.
    #[test]
    fn a_turn_on_a_rotated_seat_shows_on_the_drawn_body() {
        let main = aurora_net::origin_to_handle(256000, 256000);
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = demo::DEMO_AGENT;
        world.main_region = Some(main);
        let mut seat = demo::action_avatar(false);
        seat.local_id = 970;
        seat.full_id = uuid::Uuid::from_u128(970);
        seat.pcode = 9;
        seat.position = Vec3::new(10.0, 20.0, 30.0);
        seat.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(0.2);
        let me = demo::action_avatar(true);
        world.apply(NetEvent::ObjectUpdates {
            handle: main,
            objects: vec![seat, me],
        });
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("avatar");
        let drawn_forward = |world: &World| {
            let (_, r, _) = Scene::object_transform_unshifted(world, idx, Instant::now()).expect("transform");
            r * Vec3::X
        };
        let yaw = 2.2_f32;
        assert!(snap_seated(&mut world, demo::DEMO_AGENT, yaw));
        let forward = drawn_forward(&world);
        let wanted = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        assert!(forward.abs_diff_eq(wanted, 1e-4), "{forward:?} vs {wanted:?}");
        // a standing avatar has no seat to turn on
        let mut standing = World::new(Arc::new(AvatarLibrary::load()));
        standing.agent_id = demo::DEMO_AGENT;
        assert!(!snap_seated(&mut standing, demo::DEMO_AGENT, yaw));
        // standing up ends the snap; sitting again on the same seat, with the
        // same sit rotation reported, does not bring the old turn back
        let (seat_id, sit_rotation) = {
            let o = world.objects.get(idx).expect("avatar");
            (o.parent_id, o.rotation)
        };
        let frame = |world: &mut World| world.update_bodies(Instant::now(), 0.016, Vec3::ZERO, false, false, 1.0);
        world.objects.get_mut(idx).expect("avatar").parent_id = 0;
        frame(&mut world);
        assert!(world.seat_snaps.is_empty());
        let o = world.objects.get_mut(idx).expect("avatar");
        o.parent_id = seat_id;
        o.rotation = sit_rotation;
        frame(&mut world);
        assert!(!drawn_forward(&world).abs_diff_eq(wanted, 1e-3));
        // the simulator reports a new rotation: its word wins, and the snap
        // is dropped
        assert!(snap_seated(&mut world, demo::DEMO_AGENT, yaw));
        assert!(drawn_forward(&world).abs_diff_eq(wanted, 1e-4));
        world.objects.get_mut(idx).expect("avatar").rotation = Quat::from_rotation_z(1.0);
        assert!(!drawn_forward(&world).abs_diff_eq(wanted, 1e-3));
        frame(&mut world);
        assert!(world.seat_snaps.is_empty());
    }
}
