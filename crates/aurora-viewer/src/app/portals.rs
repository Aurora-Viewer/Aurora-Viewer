use super::*;
use crate::world::portals::unix_time;
use aurora_net::bridge::portal::{Access, LIFETIME, Portal};
pub(super) mod demo;
pub(super) mod landmark;
pub(super) struct Journey {
    creator: uuid::Uuid,
    origin: aurora_net::bridge::portal::Destination,
    agent: uuid::Uuid,
    source: Portal,
    arrival_region: String,
    started: Instant,
    landing: Option<(Vec3, Instant)>,
    pub(super) arrived: bool,
}

impl Journey {
    pub(super) fn arrival_pending(&self, now: Instant) -> bool {
        self.arrived
            && self
                .landing
                .is_none_or(|(_, at)| now.saturating_duration_since(at) < Duration::from_secs(1))
    }
    pub(super) fn arrives_locally(&self, world: &World) -> bool {
        Some(self.source.region) == world.main_region
            && self.arrival_region.eq_ignore_ascii_case(&world.region_name())
            && self.valid(world, Instant::now(), unix_time())
    }
    fn valid(&self, world: &World, now: Instant, unix: u64) -> bool {
        self.agent == world.agent_id
            && !world.mutes.is_muted_id(&self.creator)
            && now.saturating_duration_since(self.started) <= Duration::from_secs(30)
            && self.source.live(unix)
            && !world.tp_failed
    }
    /// Its time counted from `now`: the crossing waited for a question.
    pub(super) fn restarted(mut self, now: Instant) -> Self {
        self.started = now;
        self
    }
    fn new(world: &World, creator: uuid::Uuid, source: &Portal, now: Instant) -> Self {
        let normal = Vec3::new(source.yaw.cos(), source.yaw.sin(), 0.0);
        Self {
            creator,
            origin: aurora_net::bridge::portal::Destination {
                region: world.region_name(),
                position: (Vec3::from(source.center) - normal * 0.15)
                    .with_z(world.agent.position.z)
                    .to_array(),
            },
            agent: world.agent_id,
            source: source.clone(),
            arrival_region: source.destination.clone(),
            started: now,
            arrived: false,
            landing: None,
        }
    }

    fn return_portal(&self, region: u64, position: Vec3) -> Portal {
        let normal = Vec3::new(self.source.yaw.cos(), self.source.yaw.sin(), 0.0);
        Portal {
            id: uuid::Uuid::from_u128(self.source.id.as_u128() ^ 1),
            region,
            center: (position - normal * 0.15).to_array(),
            yaw: (-normal.y).atan2(-normal.x),
            destination: self.origin.region.clone(),
            position: self.origin.position,
            access: self.source.access,
            expires: self.source.expires,
        }
    }

    fn keep_returns(&self, world: &World, state: &mut crate::world::portals::State, now: Instant, unix: u64) -> bool {
        let Some(region) = world.main() else { return false };
        // On a local trip the departure is still in this region. Keep the
        // traveler's pass there even if the original relay leaves or closes.
        if self.source.region == region.handle && self.landing.is_none() {
            state.keep_departure(world, self.source.clone(), self.creator, now, unix);
        }
        let exit = self.return_portal(region.handle, world.agent.position);
        crate::world::portals::fits_region(Vec3::from(exit.center), [region.heightmap.size_x, region.heightmap.size_y])
            && state.keep_return(world, exit, self.creator, now, unix)
    }
}
impl App {
    pub(super) fn tick_portals(&mut self) {
        let now = Instant::now();
        let unix = unix_time();
        let endpoint = self.bridge_runtime.query_endpoint();
        let mut state = std::mem::take(&mut self.world.portals);
        state.sync(&self.world, endpoint.as_ref().map(|endpoint| endpoint.token), unix);
        state.reduced_motion = self.settings.reduce_motion;
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
            let returned = self.settings.portal_returns && journey.keep_returns(&self.world, &mut state, now, unix);
            if returned {
                if self.demo {
                    log::info!("portal demo persistent arrival: entries {}, region {region}", state.entries.len());
                }
            } else {
                let (anchor, at) = *journey.landing.get_or_insert((self.world.agent.position, now));
                let floor = self.scene.preview_floor(&self.world, anchor, anchor.z);
                let terrain_ready = self.world.main().is_some_and(|r| {
                    anchor.x >= 0.0
                        && anchor.y >= 0.0
                        && anchor.x < r.heightmap.size_x as f32
                        && anchor.y < r.heightmap.size_y as f32
                        && r.heightmap
                            .patch_received
                            .get((anchor.y as u32 / 16 * r.heightmap.patches_x + anchor.x as u32 / 16) as usize)
                            .copied()
                            .unwrap_or(false)
                });
                // An unreceived terrain patch is initialized to zero, not a
                // verified floor. A distinct solid-object hit is already usable.
                let floor = floor.filter(|height| {
                    terrain_ready
                        || self
                            .world
                            .ground_height(anchor)
                            .is_none_or(|terrain| (height - terrain).abs() > 0.001)
                });
                let close = floor.is_some_and(|floor| (0.0..=2.0).contains(&(anchor.z - floor)));
                if !close && !terrain_ready && now.saturating_duration_since(at) < Duration::from_secs(1) {
                    self.portal_journey = Some(journey);
                } else {
                    let root = self.world.agent.position;
                    let height = self
                        .world
                        .objects
                        .index_of_uuid(&self.world.agent_id)
                        .and_then(|i| self.world.objects.get(i))
                        .map_or(1.7, |o| o.scale.z.clamp(0.5, 8.0));
                    let opened = close && floor.is_some_and(|floor| state.begin_arrival(journey.creator, root, floor, height, now));
                    if opened {
                        self.world.agent.flying = false;
                    }
                    if self.demo {
                        log::info!("portal demo one-way arrival: region {region}, opened {opened}, floor {floor:?}, root {root:?}");
                    }
                }
            }
        }
        if let Some(center) = state.awaiting_arrival_land() {
            let fits = self.world.main().is_some_and(|region| {
                crate::world::portals::arrival::fits_region(center, [region.heightmap.size_x, region.heightmap.size_y])
            });
            let build = if fits {
                self.build_right_at(center)
            } else {
                crate::world::land::BuildRight::Denied
            };
            state.arrival_land_known(build, now);
        }
        // a return kept ours while its land was unknown: asked again until known
        for (creator, id, center) in state.awaiting_land() {
            let build = self.build_right_at(center);
            state.land_known(creator, id, build, unix);
        }
        if let Some(group) = state.take_group_refresh() {
            self.world.group_data.request_members(group);
        }
        state.awaiting_floor = !state.reduced_motion && self.portal_journey.as_ref().is_some_and(|journey| journey.arrived);
        if let Some(id) = state.take_profile_want() {
            self.world.want_profile(id);
        }
        if self
            .portal_ask
            .as_ref()
            .is_some_and(|ask| state.crossable(&self.world, ask.owner, ask.portal.id).is_none())
        {
            // Closed, expired, muted or left out meanwhile: nothing to answer.
            self.portal_ask = None;
        }
        let mut input = self.input;
        if self.camera.free().is_none() && !self.joystick.active {
            self.joystick.motion.apply_avatar(&mut input);
        }
        // update_move_input already filters text-owned keys. Nearby chat can
        // retain focus while passing arrows to the avatar (LLViewerWindow::handleKey,
        // indra/newview/llviewerwindow.cpp); UI focus must not veto those controls.
        let voluntary = self.focused
            && !self.world.teleporting
            && !self.world.frozen
            && !self.world.agent.seated
            && !self.world.agent.ground_sit
            && self.autopilot.is_none();
        let vertical = self.world.agent.flying || self.world.agent.position.z < self.world.main_water_height();
        let intent = if voluntary {
            movement_intent(&input, self.world.agent.yaw, vertical)
        } else {
            Vec3::ZERO
        };
        let crossed = state.update(&self.world, intent, now);
        let command = state.command(endpoint.as_ref(), now);
        self.world.portals = state;
        let arriving: Vec<_> = self.world.portals.arrivals.keys().copied().collect();
        for id in arriving {
            if let Some(index) = self.world.objects.index_of_uuid(&id)
                && let Some(object) = self.world.objects.get_mut(index)
            {
                object.render.needs_records = true;
            }
        }
        if std::mem::take(&mut self.world.portals.failed) {
            let message = aurora_i18n::tr!("portal-share-failed");
            self.portal_ui.notice = Some(message.into());
            self.world.system_message(message);
        }
        if let Some(command) = command {
            self.send(command);
        }
        if let Some((creator, portal)) = crossed {
            if !self.world.portals.ask_before_crossing(&self.world, creator, portal.id) {
                self.cross_portal(creator, portal, now);
            } else if self.portal_ask.is_none()
                && let Some(entry) = self.world.portals.crossable(&self.world, creator, portal.id)
            {
                // One question per portal; the walk through is consumed.
                self.portal_ask = Some(ui::portals::CrossingAsk::new(&self.world, entry, portal));
            }
        }
        self.world.portals.departure = self
            .portal_journey
            .as_ref()
            .filter(|journey| self.world.teleporting && !journey.arrived)
            .map(|journey| journey.source.id);
        self.update_place_portal();
        self.exchange_portal_demo();
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_PORTALS")
            && (self.arrival_portal_demo(&mode) || self.repeat_portal_demo(&mode) || self.crowded_return_demo(&mode))
        {
            return;
        }
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
            // Frame the rift before it opens so the first wisp can be
            // compared from a stationary camera.
            let rift = mode.starts_with("rift");
            if rift && self.frame_count < 660 && self.frame_count != 600 {
                return;
            }
            let position = self.world.agent.position;
            let ahead = Vec3::new(self.world.agent.yaw.cos(), self.world.agent.yaw.sin(), 0.0);
            let mut center = if mode.contains("-near-") {
                position + ahead * 0.3
            } else if mode == "near" {
                position
            } else {
                position + ahead * 4.0
            };
            if rift {
                // Match placement from the portal window, including on a slope.
                if let Some(floor) = self.scene.preview_floor(&self.world, center, position.z + 1.0) {
                    center.z = floor + crate::world::portals::HEIGHT;
                }
                if self.frame_count == 600 {
                    self.panels.chat = false;
                    self.panels.people = false;
                    self.panels.inventory = false;
                    self.settings.reduce_motion = mode == "rift-static";
                    let tangent = Vec3::new(-ahead.y, ahead.x, 0.0);
                    let (side, facing) = match mode.as_str() {
                        "rift-profile" => (4.5, 0.6),
                        "rift-reverse" => (3.0, -4.0),
                        _ => (3.0, 4.0),
                    };
                    self.camera.set_view(
                        center + tangent * side + ahead * facing + Vec3::Z * 1.2,
                        center,
                        &mut self.world.agent,
                        &self.settings.camera,
                    );
                    return;
                }
            }
            self.portal_ui.demo_seeded = true;
            self.portal_ui.open = !rift;
            self.portal_ui.destination = "Demo destination/128/128/25".into();
            let portal = Portal {
                // The rift ID seeds the currents. Use the same pattern when
                // comparing the front, reverse, grazing and still views.
                id: if rift {
                    uuid::Uuid::from_u128(0xA772)
                } else {
                    uuid::Uuid::new_v4()
                },
                region: self.world.main_region.unwrap_or(256),
                center: center.to_array(),
                yaw: self.world.agent.yaw,
                destination: if mode.starts_with("private-") || mode.starts_with("slice-local-teleport") {
                    // The offline teleport stays in the plaza. Keep the real
                    // arrival-region check active so it also creates the exit.
                    self.world.region_name()
                } else {
                    "Demo destination".into()
                },
                position: if mode.starts_with("slice-local-teleport") {
                    let tangent = Vec3::new(-ahead.y, ahead.x, 0.0);
                    let point = center + tangent * 3.0 + ahead * 2.65;
                    point.with_z(crate::demo::floor_at(point.x, point.y) + 0.84).to_array()
                } else if mode.starts_with("private-") {
                    // Land on open terrain, away from the raised plaza's edge.
                    // 0.84 m is the offline simulator's standing root height.
                    [116.0, 108.0, crate::demo::floor_at(116.0, 108.0) + 0.84]
                } else {
                    [128.0, 128.0, 25.0]
                },
                access: if mode == "friends" || mode.starts_with("private-") {
                    Access::Friends
                } else {
                    Access::Everyone
                },
                expires: unix + LIFETIME,
            };
            if mode.ends_with("no-build") {
                self.demo_close_land();
            }
            let build = self.build_right_at(center);
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
                state.keep_return(&self.world, portal.clone(), creator, now, unix);
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
            } else if mode.starts_with("stranger") {
                // A public portal from a resident who is not our friend; the
                // walk of `portal_demo_input` crosses it from frame 600.
                self.portal_ui.open = false;
                self.portal_ui.destination.clear();
                let owner = uuid::Uuid::from_u128(0xA770);
                if mode == "stranger-groups" {
                    let group = uuid::Uuid::from_u128(0xA770_6000);
                    self.world.groups.groups.push(aurora_net::GroupMembership {
                        id: group,
                        name: "Demo group".into(),
                        insignia: uuid::Uuid::nil(),
                        powers: 0,
                        accept_notices: true,
                        list_in_profile: true,
                        contribution: 0,
                    });
                    self.world.profiles.merge_groups(
                        owner,
                        vec![aurora_net::profile::ProfileGroup {
                            id: group,
                            name: "Demo group".into(),
                            insignia: uuid::Uuid::nil(),
                        }],
                    );
                }
                if let Some(wire) = aurora_net::bridge::portal::Message::Open(portal.announcement()).encode() {
                    state.receive(&self.world, &format!("<auroraPortal>{owner}|{wire}</auroraPortal>"), now, unix);
                }
            } else if let Err(refusal) = state.create(portal, build, position, now, unix) {
                self.portal_ui.notice = Some(refusal.message().into());
            }
            self.world.portals = state;
        }
    }

    fn cross_portal(&mut self, creator: uuid::Uuid, portal: Portal, now: Instant) {
        // Ordinary location teleports retain RLVa, maturity, map lookup,
        // cancellation and simulator access checks (LLAgent::teleportViaLocation).
        let journey = Journey::new(&self.world, creator, &portal, now);
        let look_at = Vec3::new(portal.yaw.cos(), portal.yaw.sin(), 0.0);
        self.teleport_to_location_looking_with_journey(portal.destination, Vec3::from(portal.position), look_at, Some(journey));
    }

    /// The question before a non-friend's portal is first crossed. Go checks
    /// again that it can still be crossed, then travels like a crossing.
    pub(super) fn portal_crossing_question(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(ask) = &self.portal_ask else { return };
        let Some(go) = ui::portals::crossing_confirm(ctx, p, ask) else {
            return;
        };
        let Some(ask) = self.portal_ask.take() else { return };
        if !go
            || self.world.teleporting
            || !ask.portal.live(unix_time())
            || self.world.portals.crossable(&self.world, ask.owner, ask.portal.id).is_none()
        {
            return;
        }
        self.world.portals.confirm(ask.owner, ask.portal.id);
        self.cross_portal(ask.owner, ask.portal, Instant::now());
    }

    /// The setting, or the one an `AURORA_DEMO_PORTALS` scenario tests (never
    /// saved). The earlier scenarios show a non-friend's portal.
    pub(super) fn portal_sources(&self) -> crate::world::portals::Sources {
        use crate::world::portals::Sources;
        if !self.demo {
            return self.settings.portal_sources;
        }
        match std::env::var("AURORA_DEMO_PORTALS").as_deref() {
            Ok("stranger") => Sources::Everyone,
            Ok("stranger-groups") => Sources::FriendsAndGroups,
            Ok("stranger-hidden") => Sources::Friends,
            Ok(mode)
                if mode == "friends" || mode.starts_with("private-") || mode.starts_with("return") || mode.starts_with("repeat-local") =>
            {
                Sources::Everyone
            }
            _ => self.settings.portal_sources,
        }
    }

    fn portal_slice_demo(&mut self, mode: &str) {
        if self.frame_count < 420 {
            return;
        }
        let Some(entry) = self.world.portals.own().or_else(|| self.world.portals.entries.first()) else {
            return;
        };
        let portal = entry.portal.clone();
        let creator = entry.owner;
        let Some(handle) = self.world.main_region else { return };
        let normal = Vec3::new(portal.yaw.cos(), portal.yaw.sin(), 0.0);
        let tangent = Vec3::new(-normal.y, normal.x, 0.0);
        let center = Vec3::from(portal.center);
        let sideways = mode == "slice-sideways";
        let complete_sideways = mode == "slice-complete-sideways";
        let local_teleport = mode.starts_with("slice-local-teleport");
        // Let the stationary pose and attachments settle before crossing.
        let phase = (self.frame_count.saturating_sub(if sideways || complete_sideways || local_teleport {
            780
        } else {
            420
        }) % 360) as f32
            / 60.0;
        let distance = match mode {
            "slice-local-teleport" | "slice-local-teleport-terse" => 1.2 - phase * 0.9,
            "slice-sideways" => 0.15,
            "slice-complete-sideways" => 0.6 - phase.min(2.0) * 0.8,
            "slice-still" | "slice-large" | "slice-wide" | "slice-wide-reverse" => 0.12,
            "slice-profile" | "slice-profile-reverse" => 0.18,
            "slice-turnback" | "slice-turnback-reverse" => 1.2 - phase.min(6.0 - phase) * 0.7,
            "slice-run" => 2.0 - phase * 0.9,
            _ => 1.2 - phase * 0.4,
        };
        let mut peer = crate::demo::poser::peer();
        peer.position = center
            + normal * distance
            + tangent
                * if sideways {
                    phase.min(6.0 - phase) * 1.2
                } else if complete_sideways {
                    (phase - 2.0).clamp(0.0, 2.0) * 1.2
                } else {
                    0.0
                };
        if local_teleport
            && self.frame_count >= 900
            && let Some(destination) = &portal.destination
        {
            peer.position = Vec3::from(destination.position) + normal * (self.frame_count - 900) as f32 / 60.0 * 0.35;
        }
        let turning_back = (mode.starts_with("slice-turnback") && phase >= 3.0) || (local_teleport && self.frame_count >= 900);
        peer.rotation = glam::Quat::from_rotation_z(portal.yaw + if turning_back { 0.0 } else { std::f32::consts::PI });
        peer.velocity = Vec3::ZERO;
        let id = peer.full_id;
        let local = peer.local_id;
        let first = self.world.objects.index_of_uuid(&id).is_none();
        let target = peer.position;
        let height = peer.scale.z.clamp(0.5, 8.0);
        if local_teleport && mode.ends_with("terse") && !first {
            self.world.apply(NetEvent::TerseUpdates {
                handle,
                updates: vec![aurora_net::objects::TerseUpdate {
                    local_id: local,
                    state: peer.state,
                    is_avatar: true,
                    foot_plane: peer.foot_plane,
                    position: peer.position,
                    rotation: peer.rotation,
                    velocity: peer.velocity,
                    acceleration: peer.acceleration,
                    angular_velocity: peer.angular_velocity,
                    texture_entry: None,
                }],
            });
        } else {
            self.world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![peer],
            });
        }
        if local_teleport
            && self.frame_count == 900
            && let Some(object) = self.world.objects.index_of_uuid(&id).and_then(|idx| self.world.objects.get(idx))
        {
            log::info!(
                "portal observer demo: placement serial {}, arrival gap {} m",
                object.placement_serial,
                self.world.objects.predicted(object, Instant::now()).0.distance(target)
            );
        }
        if local_teleport
            && self.frame_count == 900
            && let Some(floor) = self.scene.preview_floor(&self.world, target, target.z)
        {
            let event = aurora_net::bridge::portal::Arrival {
                id: uuid::Uuid::new_v4(),
                creator,
                region: handle,
                position: target.to_array(),
                floor,
                height,
                starts: crate::world::portals::arrival::unix_millis() + 250,
            };
            if let Some(wire) = aurora_net::bridge::portal::Message::Arrival(event).encode() {
                let mut state = std::mem::take(&mut self.world.portals);
                state.receive(
                    &self.world,
                    &format!("<auroraPortal>{id}|{wire}</auroraPortal>"),
                    Instant::now(),
                    unix_time(),
                );
                self.world.portals = state;
            }
        }
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
            worn.rotation = if mode.starts_with("slice-wide") || sideways {
                glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
            } else {
                glam::Quat::IDENTITY
            };
            worn.scale = Vec3::new(
                if mode.starts_with("slice-wide") || sideways {
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
        }
        // Keep the automation view deterministic even if startup/movement
        // resets camera focus after the peer's first geometry update.
        let profile = mode.starts_with("slice-profile");
        let close = mode.starts_with("slice-turnback") || profile;
        let across = if profile {
            3.8
        } else if close {
            2.5
        } else {
            5.0
        };
        let ahead = if profile {
            0.35
        } else if close {
            2.0
        } else {
            4.0
        };
        let focus = center
            + tangent
                * if local_teleport {
                    1.5
                } else if sideways || complete_sideways {
                    1.2
                } else {
                    0.0
                }
            + normal * if local_teleport { 1.2 } else { 0.0 };
        self.camera.set_view(
            focus
                + tangent * across
                + normal * if mode.ends_with("reverse") { -ahead } else { ahead }
                + Vec3::Z * if close { 1.2 } else { 2.2 },
            focus,
            &mut self.world.agent,
            &self.settings.camera,
        );
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
        let build = self.build_right_at(center);
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
        if let Err(refusal) = self
            .world
            .portals
            .create(portal, build, self.world.agent.position, Instant::now(), unix_time())
        {
            self.portal_ui.notice = Some(refusal.message().into());
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
        let build = self.build_right_at(center);
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
        let created = if valid {
            self.world
                .portals
                .create(portal, build, self.world.agent.position, Instant::now(), unix_time())
        } else {
            Err(crate::world::portals::Refusal::Invalid)
        };
        if let Err(refusal) = created {
            self.portal_notice(refusal.message());
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
    fn return_journeys_keep_the_departure_place_height_permissions_and_expiry() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.agent.position.z = 26.4;
        let source = Portal {
            id: uuid::Uuid::from_u128(7),
            region: world.main_region.unwrap(),
            center: [10.0, 10.0, 27.0],
            yaw: 0.0,
            destination: "Other destination".into(),
            position: [128.0, 128.0, 30.0],
            access: Access::Friends,
            expires: unix_time() + LIFETIME,
        };
        let creator = uuid::Uuid::from_u128(9);
        let journey = Journey::new(&world, creator, &source, Instant::now());
        let exit = journey.return_portal(512, Vec3::new(130.0, 131.0, 31.0));
        assert_eq!(journey.creator, creator);
        assert_eq!(exit.region, 512);
        assert_eq!(exit.destination, world.region_name());
        assert_eq!(exit.position, [9.85, 10.0, 26.4]);
        assert_eq!(exit.center, [129.85, 131.0, 31.0]);
        assert_eq!(exit.access, source.access);
        assert_eq!(exit.expires, source.expires);
        assert!((Vec3::new(exit.yaw.cos(), exit.yaw.sin(), 0.0) + Vec3::X).length() < 1e-5);
        assert!(journey.valid(&world, Instant::now(), unix_time()));
        use crate::world::mutes::{Mute, MuteType};
        world
            .mutes
            .add(
                Mute {
                    id: creator,
                    name: "Muted Resident".into(),
                    kind: MuteType::Agent,
                    flags: 0,
                },
                0,
            )
            .unwrap();
        assert!(
            !journey.valid(&world, Instant::now(), unix_time()),
            "muted during transit: no arrival is opened or shared"
        );
    }

    #[test]
    fn a_persistent_journey_keeps_both_personal_passes_only_on_a_local_trip() {
        use crate::world::portals::{Sources, State};

        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let region = world.main_region.unwrap();
        let creator = uuid::Uuid::from_u128(9);
        let now = Instant::now();
        let unix = unix_time();
        let mut source = Portal {
            id: uuid::Uuid::from_u128(7),
            region,
            center: [10.0, 10.0, 25.0],
            yaw: 0.0,
            destination: world.region_name(),
            position: [128.0, 128.0, 25.0],
            access: Access::Friends,
            expires: unix + LIFETIME,
        };
        world.portal_sources = Sources::Nobody;
        for local in [true, false] {
            if !local {
                source.destination = "Next region".into();
            }
            world.agent.position = Vec3::from(source.center);
            let journey = Journey::new(&world, creator, &source, now);
            let destination = if local { region } else { region ^ 256 };
            if !local {
                let mut info = world.main().unwrap().info.as_deref().unwrap().clone();
                info.handle = destination;
                info.name = source.destination.clone();
                world.apply(NetEvent::MainRegionChanged { handle: destination });
                world.apply(NetEvent::RegionHandshake(Arc::new(info)));
            }
            world.agent.position = Vec3::from(source.position);
            let exit = journey.return_portal(destination, world.agent.position);
            // The source relay is no longer present when the teleport finishes.
            let mut state = State::default();
            state.sync(&world, None, unix);
            assert!(journey.keep_returns(&world, &mut state, now, unix));
            let returned = state.crossable(&world, creator, exit.id).unwrap();
            assert_eq!(returned.details.as_ref(), Some(&exit.details()));
            assert!(returned.waiting_for_clearance());
            let departure = state.crossable(&world, creator, source.id);
            if local {
                let departure = departure.unwrap();
                assert!(departure.ready_to_cross(now));
                assert!(!departure.reveals_arrival());
                assert_eq!(departure.portal.center, source.center);
            } else {
                assert!(departure.is_none());
            }
            let waiting = state.awaiting_land();
            assert!(waiting.contains(&(creator, exit.id, Vec3::from(exit.center))));
            assert_eq!(waiting.contains(&(creator, source.id, Vec3::from(source.center))), local);
            assert_eq!(state.visible(&world).count(), if local { 2 } else { 1 });
        }
    }

    #[test]
    fn only_the_live_same_region_portal_journey_requests_the_arrival_camera() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let region = world.main_region.unwrap();
        let source = Portal {
            id: uuid::Uuid::from_u128(7),
            region,
            center: [10.0, 10.0, 25.0],
            yaw: 0.0,
            destination: world.region_name(),
            position: [128.0, 128.0, 25.0],
            access: Access::Everyone,
            expires: unix_time() + LIFETIME,
        };
        let mut journey = Journey::new(&world, world.agent_id, &source, Instant::now());
        assert!(journey.arrives_locally(&world));
        assert!(!journey.arrival_pending(Instant::now()));
        journey.arrived = true;
        assert!(journey.arrival_pending(Instant::now()));
        journey.landing = Some((world.agent.position, Instant::now() - Duration::from_secs(2)));
        assert!(!journey.arrival_pending(Instant::now()));
        world.main_region = Some(region ^ 256);
        assert!(!journey.arrives_locally(&world));
        world.main_region = Some(region);
        journey.arrival_region = "Other destination".into();
        assert!(!journey.arrives_locally(&world));
        journey.arrival_region = world.region_name();
        journey.started -= Duration::from_secs(31);
        assert!(!journey.arrives_locally(&world));
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
