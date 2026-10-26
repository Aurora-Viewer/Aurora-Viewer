use super::*;
use crate::world::portals::unix_time;
use aurora_net::bridge::portal::{Access, LIFETIME, Portal};

impl App {
    pub(super) fn tick_portals(&mut self) {
        let now = Instant::now();
        let unix = unix_time();
        let endpoint = self.bridge_runtime.query_endpoint();
        let mut state = std::mem::take(&mut self.world.portals);
        state.sync(&self.world, endpoint.as_ref().map(|endpoint| endpoint.token), unix);
        let mut input = self.input;
        if self.camera.free().is_none() && !self.joystick.active {
            self.joystick.motion.apply_avatar(&mut input);
        }
        let voluntary = self.focused
            && !self.world.teleporting
            && !self.world.frozen
            && !self.world.agent.seated
            && !self.world.agent.ground_sit
            && self.autopilot.is_none()
            && !self.egui_ctx.egui_wants_keyboard_input();
        let vertical = self.world.agent.flying || self.world.agent.position.z < self.world.main_water_height();
        let intent = if voluntary {
            movement_intent(&input, self.world.agent.yaw, vertical)
        } else {
            Vec3::ZERO
        };
        let crossed = state.update(&self.world, intent, now);
        let command = state.command(endpoint.as_ref(), now);
        self.world.portals = state;
        if std::mem::take(&mut self.world.portals.failed) {
            let message = aurora_i18n::tr!("portal-share-failed");
            self.portal_ui.notice = Some(message.into());
            self.world.system_message(message);
        }
        if let Some(command) = command {
            self.send(command);
        }
        if let Some(portal) = crossed {
            // Ordinary location teleports retain RLVa, maturity, map lookup,
            // cancellation and simulator access checks (LLAgent::teleportViaLocation).
            self.teleport_to_location(portal.destination, Vec3::from(portal.position));
        }
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_PORTALS")
            && mode.starts_with("slice-")
        {
            self.portal_slice_demo(&mode);
        }
        if self.demo
            && !self.portal_ui.demo_seeded
            && let Ok(mode) = std::env::var("AURORA_DEMO_PORTALS")
        {
            self.portal_ui.demo_seeded = true;
            self.portal_ui.open = true;
            self.portal_ui.destination = "Demo destination/128/128/25".into();
            let position = self.world.agent.position;
            let ahead = Vec3::new(self.world.agent.yaw.cos(), self.world.agent.yaw.sin(), 0.0);
            let center = if mode == "near" { position } else { position + ahead * 4.0 };
            let portal = Portal {
                id: uuid::Uuid::new_v4(),
                region: self.world.main_region.unwrap_or(256),
                center: center.to_array(),
                yaw: self.world.agent.yaw,
                destination: "Demo destination".into(),
                position: [128.0, 128.0, 25.0],
                access: if mode == "friends" { Access::Friends } else { Access::Everyone },
                expires: unix + LIFETIME,
            };
            let mut state = std::mem::take(&mut self.world.portals);
            if mode == "friends" {
                let owner = uuid::Uuid::from_u128(0xA770);
                if let Some(wire) = aurora_net::bridge::portal::Message::Open(portal).encode() {
                    state.receive(&self.world, &format!("<auroraPortal>{owner}|{wire}</auroraPortal>"), now, unix);
                }
            } else {
                state.create(portal, position, now, unix);
            }
            self.world.portals = state;
        }
    }

    fn portal_slice_demo(&mut self, mode: &str) {
        if self.frame_count < 420 {
            return;
        }
        let Some(entry) = self.world.portals.own() else { return };
        let portal = entry.portal.clone();
        let Some(handle) = self.world.main_region else { return };
        let normal = Vec3::new(portal.yaw.cos(), portal.yaw.sin(), 0.0);
        let tangent = Vec3::new(-normal.y, normal.x, 0.0);
        let center = Vec3::from(portal.center);
        let phase = (self.frame_count.saturating_sub(420) % 360) as f32 / 60.0;
        let distance = match mode {
            "slice-still" | "slice-large" => 0.12,
            "slice-run" => 2.0 - phase * 0.9,
            _ => 1.2 - phase * 0.4,
        };
        let mut peer = crate::demo::poser::peer();
        peer.position = center + normal * distance;
        peer.rotation = glam::Quat::from_rotation_z(portal.yaw + std::f32::consts::PI);
        peer.velocity = Vec3::ZERO;
        let id = peer.full_id;
        let local = peer.local_id;
        let first = self.world.objects.index_of_uuid(&id).is_none();
        self.world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![peer],
        });
        if first {
            if let Some(gfx) = &mut self.gfx {
                self.scene.install_demo_animesh(&mut gfx.renderer);
            }
            for (animation, clip) in crate::demo::link::animations() {
                self.scene.anims.insert(
                    animation,
                    Arc::new(crate::scene::anim::BoundAnim::bind(clip, &self.world.avatar_lib.rig)),
                );
            }
            if let Some(NetEvent::Appearance(mut appearance)) =
                crate::demo::classic_events(&self.world.avatar_lib.shape_params).into_iter().next()
            {
                appearance.avatar_id = id;
                self.world.apply(NetEvent::Appearance(appearance));
            }
            self.portal_ui.open = false;
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.inventory = false;
            let animation = match mode {
                "slice-fly" => crate::own_motion::ANIM_FLY,
                "slice-run" => crate::own_motion::ANIM_RUN,
                _ => crate::own_motion::ANIM_WALK,
            };
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: id,
                anims: vec![(animation, 1)],
                sources: Vec::new(),
            });
            // Both a skinned mesh and a long rigid object share the peer's cut.
            for event in crate::demo::own_rigged_events(local) {
                self.world.apply(event);
            }
            let mut worn = crate::demo::action_avatar(false);
            worn.full_id = uuid::Uuid::from_u128(0xA077_A11A);
            worn.local_id = 9728;
            worn.pcode = aurora_prim::params::LL_PCODE_VOLUME;
            worn.parent_id = local;
            worn.state = 0x60;
            worn.position = Vec3::ZERO;
            worn.rotation = glam::Quat::IDENTITY;
            worn.scale = Vec3::new(if mode == "slice-large" { 3.0 } else { 1.2 }, 0.12, 0.12);
            self.world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![worn],
            });
            self.camera.set_view(
                center + tangent * 5.0 + normal * if mode == "slice-reverse" { -4.0 } else { 4.0 } + Vec3::Z * 2.2,
                center,
                &mut self.world.agent,
                &self.settings.camera,
            );
        }
    }

    pub(super) fn portals_window(&mut self, ctx: &egui::Context, palette: &Palette) {
        let ready = self.demo || self.bridge_runtime.query_endpoint().is_some();
        let actions = self.portal_ui.show(ctx, palette, &self.world, ready);
        if actions.close {
            self.world.portals.close(unix_time());
        }
        if !actions.create {
            return;
        }
        self.portal_ui.notice = None;
        let Some(location) = crate::slurl::parse(&self.portal_ui.destination).filter(|location| location.grid.is_none()) else {
            return;
        };
        if let Access::Group(id) = self.portal_ui.access
            && !self.world.groups.groups.iter().any(|group| group.id == id)
        {
            return;
        }
        let Some(region) = self.world.main_region else { return };
        let yaw = self.world.agent.yaw;
        let mut center = self.world.agent.position + Vec3::new(yaw.cos(), yaw.sin(), 0.0) * 4.0;
        if self
            .world
            .main()
            .is_none_or(|region| !crate::world::portals::fits_region(center, [region.heightmap.size_x, region.heightmap.size_y]))
        {
            self.portal_ui.notice = Some(aurora_i18n::tr!("portal-invalid").into());
            return;
        }
        // Support elevated floors through the same scene picker as local rez.
        if let Some(floor) = self.scene.preview_floor(&self.world, center, self.world.agent.position.z + 1.0) {
            center.z = floor + crate::world::portals::HEIGHT;
        }
        let portal = Portal {
            id: uuid::Uuid::new_v4(),
            region,
            center: center.to_array(),
            yaw,
            destination: location.region,
            position: location.pos.to_array(),
            access: self.portal_ui.access,
            expires: unix_time() + LIFETIME,
        };
        if !self
            .world
            .portals
            .create(portal, self.world.agent.position, Instant::now(), unix_time())
        {
            self.portal_ui.notice = Some(aurora_i18n::tr!("portal-invalid").into());
        }
    }
}

fn movement_intent(input: &MoveInput, yaw: f32, vertical: bool) -> Vec3 {
    let axis = |positive: bool, negative: bool| f32::from(u8::from(positive)) - f32::from(u8::from(negative));
    let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
    let left = Vec3::new(-forward.y, forward.x, 0.0);
    (forward * axis(input.forward, input.back)
        + left * axis(input.strafe_left, input.strafe_right)
        + Vec3::Z * if vertical { axis(input.up, input.down) } else { 0.0 })
    .normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portal_intent_excludes_turning_and_ground_crouching_and_tracks_avatar_axes() {
        let input = MoveInput {
            turn_left: true,
            down: true,
            ..Default::default()
        };
        assert_eq!(movement_intent(&input, 0.0, false), Vec3::ZERO);
        assert_eq!(movement_intent(&input, 0.0, true), Vec3::NEG_Z);
        let input = MoveInput {
            forward: true,
            ..Default::default()
        };
        assert!(movement_intent(&input, std::f32::consts::FRAC_PI_2, false).distance(Vec3::Y) < 1e-5);
    }
}
