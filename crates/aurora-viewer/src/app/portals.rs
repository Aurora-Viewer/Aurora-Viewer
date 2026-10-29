use super::*;
use crate::world::portals::unix_time;
use aurora_net::bridge::portal::{Access, LIFETIME, Portal};
pub(super) mod demo;
pub(super) mod landmark;
const RETURN_DISTANCE: f32 = 4.0;

pub(super) struct Journey {
    creator: uuid::Uuid,
    agent: uuid::Uuid,
    portal: Portal,
    source: Portal,
    arrival_region: String,
    started: Instant,
    pub(super) arrived: bool,
}

impl Journey {
    fn valid(&self, world: &World, now: Instant, unix: u64) -> bool {
        self.agent == world.agent_id
            && now.saturating_duration_since(self.started) <= Duration::from_secs(30)
            && self.portal.live(unix)
            && !world.tp_failed
    }

    fn new(world: &World, creator: uuid::Uuid, source: &Portal, now: Instant) -> Self {
        let normal = Vec3::new(source.yaw.cos(), source.yaw.sin(), 0.0);
        Self {
            creator,
            agent: world.agent_id,
            portal: Portal {
                // Paired endpoint IDs alternate on every crossing. Travelers
                // from one source all derive the same return, also within a sim.
                id: uuid::Uuid::from_u128(source.id.as_u128() ^ 1),
                region: source.region,
                // Emerge in front of the return, facing away from it. Holding
                // forward through arrival must not immediately send us back.
                center: (Vec3::from(source.position) - normal * RETURN_DISTANCE).to_array(),
                yaw: (-normal.y).atan2(-normal.x),
                destination: world.region_name(),
                position: (Vec3::from(source.center) - normal * RETURN_DISTANCE).to_array(),
                access: source.access,
                expires: source.expires,
            },
            source: source.clone(),
            arrival_region: source.destination.clone(),
            started: now,
            arrived: false,
        }
    }
}

impl App {
    pub(super) fn tick_portals(&mut self) {
        let now = Instant::now();
        let unix = unix_time();
        let endpoint = self.bridge_runtime.query_endpoint();
        let mut state = std::mem::take(&mut self.world.portals);
        state.sync(&self.world, endpoint.as_ref().map(|endpoint| endpoint.token), unix);
        if self
            .portal_journey
            .as_ref()
            .is_some_and(|journey| !journey.valid(&self.world, now, unix))
        {
            self.portal_journey = None;
        }
        if self.portal_journey.as_ref().is_some_and(|journey| journey.arrived)
            && !self.world.teleporting
            && !self.world.region_name().is_empty()
            && let Some(mut journey) = self.portal_journey.take()
            && journey.arrival_region.eq_ignore_ascii_case(&self.world.region_name())
            && let Some(region) = self.world.main_region
        {
            journey.portal.region = region;
            if journey.source.region == region {
                state.share_return(journey.source, journey.creator, self.world.agent.position, now, unix);
            }
            // Landing points can redirect a request within the same region.
            // The first shared aperture wins when other travelers arrive.
            if Vec3::from(journey.portal.center).distance(self.world.agent.position) > 10.0 {
                let normal = Vec3::new(journey.portal.yaw.cos(), journey.portal.yaw.sin(), 0.0);
                journey.portal.center = (self.world.agent.position + normal * RETURN_DISTANCE).to_array();
            }
            if self.world.main().is_some_and(|r| {
                crate::world::portals::fits_region(Vec3::from(journey.portal.center), [r.heightmap.size_x, r.heightmap.size_y])
            }) {
                state.share_return(journey.portal, journey.creator, self.world.agent.position, now, unix);
            }
        }
        if let Some(group) = state.take_group_refresh() {
            self.world.group_data.request_members(group);
        }
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
        if let Some((creator, portal)) = crossed {
            // Ordinary location teleports retain RLVa, maturity, map lookup,
            // cancellation and simulator access checks (LLAgent::teleportViaLocation).
            let journey = Journey::new(&self.world, creator, &portal, now);
            let look_at = Vec3::new(portal.yaw.cos(), portal.yaw.sin(), 0.0);
            self.teleport_to_location_looking(portal.destination, Vec3::from(portal.position), look_at);
            if self.world.teleporting {
                self.portal_journey = Some(journey);
            }
        }
        self.update_landmark_portal();
        self.exchange_portal_demo();
        if self.seed_portal_drop_demo() {
            return;
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
            let center = if mode == "near" {
                position
            } else if mode.starts_with("return") {
                position - ahead * RETURN_DISTANCE
            } else {
                position + ahead * 4.0
            };
            let portal = Portal {
                id: uuid::Uuid::new_v4(),
                region: self.world.main_region.unwrap_or(256),
                center: center.to_array(),
                yaw: if mode.starts_with("return") {
                    (-ahead.y).atan2(-ahead.x)
                } else {
                    self.world.agent.yaw
                },
                destination: if mode.starts_with("return") {
                    "Demo origin"
                } else {
                    "Demo destination"
                }
                .into(),
                position: [128.0, 128.0, 25.0],
                access: if mode == "friends" || mode == "return-private" || mode.starts_with("private-") {
                    Access::Friends
                } else {
                    Access::Everyone
                },
                expires: unix + LIFETIME,
            };
            let mut state = std::mem::take(&mut self.world.portals);
            if mode.starts_with("return") {
                let creator = uuid::Uuid::from_u128(0xA770);
                self.portal_ui.open = false;
                self.panels.chat = false;
                let tangent = Vec3::new(-ahead.y, ahead.x, 0.0);
                self.camera.set_view(
                    center + ahead * 7.0 + tangent * 4.0 + Vec3::Z * 2.0,
                    center,
                    &mut self.world.agent,
                    &self.settings.camera,
                );
                state.share_return(portal.clone(), creator, position, now, unix);
                let mut announcement = portal.announcement();
                announcement.creator = Some(creator);
                for peer in [0xA771, 0xA772] {
                    let peer = uuid::Uuid::from_u128(peer);
                    if let Some(wire) = aurora_net::bridge::portal::Message::Open(announcement.clone()).encode() {
                        state.receive(&self.world, &format!("<auroraPortal>{peer}|{wire}</auroraPortal>"), now, unix);
                    }
                }
            } else if mode == "friends" || mode.starts_with("private-") {
                self.portal_ui.open = false;
                self.portal_ui.destination.clear();
                let owner = uuid::Uuid::from_u128(0xA770);
                if mode.starts_with("private-") {
                    self.seed_portal_peer_demo(&mode, portal.clone(), owner);
                }
                if let Some(wire) = aurora_net::bridge::portal::Message::Open(portal.announcement()).encode() {
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
            "slice-still" | "slice-large" | "slice-wide" | "slice-wide-reverse" => 0.12,
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
            worn.rotation = if mode.starts_with("slice-wide") {
                glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
            } else {
                glam::Quat::IDENTITY
            };
            worn.scale = Vec3::new(
                if mode.starts_with("slice-wide") {
                    4.2
                } else if mode == "slice-large" {
                    3.0
                } else {
                    1.2
                },
                0.12,
                0.12,
            );
            self.world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![worn],
            });
            self.camera.set_view(
                center + tangent * 5.0 + normal * if mode.ends_with("reverse") { -4.0 } else { 4.0 } + Vec3::Z * 2.2,
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
            self.portal_drop = None;
            self.world.portals.close(unix_time());
        }
        if !actions.create {
            return;
        }
        self.portal_ui.notice = None;
        self.portal_drop = None;
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

    pub(super) fn create_portal_at(&mut self, destination: String, position: Vec3, access: Access, center: Vec3, yaw: f32) {
        let Some(region) = self.world.main_region else { return };
        let valid = self
            .world
            .main()
            .is_some_and(|region| crate::world::portals::fits_region(center, [region.heightmap.size_x, region.heightmap.size_y]));
        let destination_text = if self.world.rlva.has("showloc") {
            String::new()
        } else {
            crate::slurl::make(&destination, position)
        };
        let portal = Portal {
            id: uuid::Uuid::new_v4(),
            region,
            center: center.to_array(),
            yaw,
            destination,
            position: position.to_array(),
            access,
            expires: unix_time() + LIFETIME,
        };
        if !valid
            || !self
                .world
                .portals
                .create(portal, self.world.agent.position, Instant::now(), unix_time())
        {
            self.portal_drop_notice("portal-invalid");
        } else {
            self.portal_ui.destination = destination_text;
            self.portal_ui.access = access;
            self.portal_ui.notice = None;
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
    fn portal_journey_restores_the_original_endpoint_and_keeps_a_bounded_return_intent() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = uuid::Uuid::from_u128(1);
        let source = Portal {
            id: uuid::Uuid::from_u128(7),
            region: 256,
            center: [10.0, 10.0, 25.0],
            yaw: 0.0,
            destination: "Demo destination".into(),
            position: [128.0, 128.0, 25.0],
            access: Access::Friends,
            expires: 1120,
        };
        let now = Instant::now();
        let mut outward = Journey::new(&world, world.agent_id, &source, now);
        // Supply the region name normally provided by the region handshake.
        outward.portal.destination = "Demo source".into();
        assert!(!outward.arrived, "a teleport request is not an arrival");
        assert!(outward.valid(&world, now, 1000));
        assert_ne!(outward.portal.id, source.id, "same-region endpoints remain distinct");
        assert!(
            Vec3::from(outward.portal.center).x < source.position[0],
            "the return is behind the arrival"
        );
        assert_eq!(outward.portal.expires, source.expires);
        assert_eq!(outward.portal.access, source.access);
        let back = Journey::new(&world, world.agent_id, &outward.portal, now);
        assert_eq!(back.portal.id, source.id);
        assert!(Vec3::from(back.portal.center).distance(Vec3::from(source.center)) < 1e-5);
        assert_eq!(back.portal.position, source.position);
        assert!(!outward.valid(&world, now + Duration::from_secs(31), 1031));
        assert!(!outward.valid(&world, now, 1120));
        world.tp_failed = true;
        assert!(!outward.valid(&world, now, 1000));
        world.tp_failed = false;
        world.agent_id = uuid::Uuid::from_u128(2);
        assert!(!outward.valid(&world, now, 1000));
    }

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
