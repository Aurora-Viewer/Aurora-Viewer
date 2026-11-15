//! Real inventory world-drop input and two in-memory bridge peers. All message
//! exchange follows the production codec and creator authorization boundary.
use super::*;
use aurora_net::bridge::{
    Endpoint,
    query::{Query, Reply},
};

pub(in crate::app) struct Peer {
    world: World,
    state: crate::world::portals::State,
    endpoint: Endpoint,
}

fn drop_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_PORTALS").ok().filter(|mode| mode.starts_with("drop-")))
        .as_deref()
}

impl App {
    pub(in crate::app) fn repeat_portal_demo(&mut self, mode: &str) -> bool {
        if !mode.starts_with("repeat-local") {
            return false;
        }
        if !self.portal_ui.demo_seeded && self.frame_count >= 420 {
            self.portal_ui.demo_seeded = true;
            self.portal_ui.open = false;
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.inventory = false;
            self.panels.appearance = false;
            self.settings.reduce_motion = false;
            self.settings.portal_returns = mode.contains("return");
            self.always_run = true;
            let position = self.world.agent.position;
            let ahead = self.world.agent.forward();
            let portal = Portal {
                id: uuid::Uuid::new_v4(),
                region: self.world.main_region.unwrap_or(256),
                center: (position + ahead * if mode.ends_with("blocked") { 0.1 } else { 4.0 }).to_array(),
                yaw: self.world.agent.yaw,
                destination: self.world.region_name(),
                // Land before the same opening. Holding run must repeatedly
                // use this single invitation, including after delayed status.
                position: position.to_array(),
                access: Access::Everyone,
                expires: unix_time() + LIFETIME,
            };
            let build = self.build_right_at(Vec3::from(portal.center));
            if let Err(refusal) = self.world.portals.create(portal, build, position, Instant::now(), unix_time()) {
                log::warn!("portal repeat demo create refused: {refusal:?}");
            }
            if mode.ends_with("busy") {
                let occupied = Portal {
                    id: uuid::Uuid::new_v4(),
                    region: self.world.main_region.unwrap_or(256),
                    center: position.to_array(),
                    yaw: self.world.agent.yaw,
                    destination: "Occupied destination".into(),
                    position: [128.0, 128.0, position.z],
                    access: Access::Friends,
                    expires: unix_time() + LIFETIME,
                };
                if let Some(wire) = aurora_net::bridge::portal::Message::Open(occupied.announcement()).encode() {
                    let mut state = std::mem::take(&mut self.world.portals);
                    state.receive(
                        &self.world,
                        &format!("<auroraPortal>{}|{wire}</auroraPortal>", uuid::Uuid::from_u128(19)),
                        Instant::now(),
                        unix_time(),
                    );
                    self.world.portals = state;
                }
            }
        }
        if mode.ends_with("late")
            && self.world.portals.arrivals.contains_key(&self.world.agent_id)
            && self.frame_count.is_multiple_of(30)
            && self
                .move_complete_at
                .is_some_and(|at| (0.1..1.5).contains(&at.elapsed().as_secs_f32()))
        {
            self.world.apply(NetEvent::TeleportStarted { flags: 0 });
            self.world.apply(NetEvent::TeleportProgress {
                message: "arriving".into(),
                flags: 0,
            });
            log::info!(
                "portal repeat late status: frame {}, teleporting {}",
                self.frame_count,
                self.world.teleporting
            );
        }
        if self.frame_count >= 600 && self.frame_count.is_multiple_of(120) {
            let clipped = self
                .world
                .objects
                .index_of_uuid(&self.world.agent_id)
                .is_some_and(|i| self.scene.portals.planes.contains_key(&i));
            log::info!(
                "portal repeat lifecycle: frame {}, teleporting {}, entries {}, arrivals {}, clipped {}, position {:?}",
                self.frame_count,
                self.world.teleporting,
                self.world.portals.entries.len(),
                self.world.portals.arrivals.len(),
                clipped,
                self.world.agent.position
            );
        }
        true
    }

    pub(in crate::app) fn crowded_return_demo(&mut self, mode: &str) -> bool {
        if mode != "returns-crowd" {
            return false;
        }
        if !self.portal_ui.demo_seeded && self.frame_count >= 660 {
            self.portal_ui.demo_seeded = true;
            self.portal_ui.open = false;
            self.panels.people = false;
            self.panels.inventory = false;
            self.panels.appearance = false;
            self.settings.portal_returns = true;
            self.settings.portal_sources = crate::world::portals::Sources::Everyone;
            self.world.portal_sources = self.settings.portal_sources;
            let root = self.world.agent.position + self.world.agent.forward() * 4.0;
            let portal = Portal {
                id: uuid::Uuid::from_u128(40),
                region: self.world.main_region.unwrap_or(256),
                center: root.to_array(),
                yaw: self.world.agent.yaw,
                destination: "Demo departure".into(),
                position: [128.0, 128.0, root.z],
                access: Access::Everyone,
                expires: unix_time() + LIFETIME,
            };
            let now = Instant::now();
            let unix = unix_time();
            let mut state = std::mem::take(&mut self.world.portals);
            let creator = uuid::Uuid::from_u128(20);
            for i in 0..20 {
                let mut next = portal.clone();
                next.id = uuid::Uuid::from_u128(40 + i);
                state.keep_return(&self.world, next, creator, now, unix);
            }
            for i in 0..20 {
                let mut next = portal.announcement();
                next.id = uuid::Uuid::from_u128(100 + i);
                next.creator = Some(uuid::Uuid::from_u128(200 + i));
                if let Some(destination) = &mut next.destination {
                    destination.position[0] += 20.0;
                }
                let Some(wire) = aurora_net::bridge::portal::Message::Open(next).encode() else {
                    continue;
                };
                state.receive(
                    &self.world,
                    &format!("<auroraPortal>{}|{wire}</auroraPortal>", uuid::Uuid::from_u128(300 + i)),
                    now,
                    unix,
                );
            }
            log::info!(
                "portal crowded return demo: entries {}, apertures {}",
                state.entries.len(),
                state.shown(&self.world).count()
            );
            self.world.portals = state;
            self.camera.set_view(
                root + Vec3::new(-4.0, -5.0, 2.0),
                root,
                &mut self.world.agent,
                &self.settings.camera,
            );
        }
        true
    }

    pub(in crate::app) fn arrival_portal_demo(&mut self, mode: &str) -> bool {
        if !mode.starts_with("arrival-") {
            return false;
        }
        let own = mode.starts_with("arrival-own");
        let actor = if own { self.world.agent_id } else { crate::demo::poser::PEER };
        let point = self.world.agent.position + self.world.agent.forward() * if own { 0.0 } else { 4.0 };
        let terrain = self.world.ground_height(point).unwrap_or(point.z - 0.84);
        let handle = self.world.main_region.unwrap_or(256);
        if mode == "arrival-platform" && self.frame_count == 420 {
            let mut platform = crate::demo::action_avatar(false);
            platform.full_id = uuid::Uuid::from_u128(77701);
            platform.local_id = 77701;
            platform.pcode = aurora_prim::params::LL_PCODE_VOLUME;
            platform.volume = aurora_prim::VolumeParams::default();
            platform.parent_id = 0;
            platform.state = 0;
            platform.position = point.with_z(terrain + 2.5);
            platform.scale = Vec3::new(4.0, 4.0, 1.0);
            platform.rotation = glam::Quat::IDENTITY;
            platform.update_flags = 0;
            platform.extra = Default::default();
            self.world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![platform],
            });
        }
        if !self.portal_ui.demo_seeded && self.frame_count >= 660 {
            self.portal_ui.demo_seeded = true;
            self.portal_ui.open = false;
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.inventory = false;
            self.panels.appearance = false;
            self.settings.portal_sources = crate::world::portals::Sources::Everyone;
            self.world.portal_sources = self.settings.portal_sources;
            self.settings.reduce_motion = mode.ends_with("static");
            self.world.portals.reduced_motion = self.settings.reduce_motion;
            let floor = self.scene.preview_floor(&self.world, point, terrain + 4.0).unwrap_or(terrain);
            let mut peer = crate::demo::poser::peer();
            if own {
                peer.full_id = actor;
                if let Some(object) = self.world.objects.index_of_uuid(&actor).and_then(|i| self.world.objects.get(i)) {
                    peer.local_id = object.key.local_id;
                }
            }
            peer.position = point.with_z(floor + if mode == "arrival-sky" { 3.01 } else { 0.84 });
            peer.velocity = Vec3::ZERO;
            peer.acceleration = Vec3::ZERO;
            let (id, local, root, height) = (peer.full_id, peer.local_id, peer.position, peer.scale.z.clamp(0.5, 8.0));
            self.world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![peer],
            });
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
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: id,
                anims: vec![(crate::own_motion::ANIM_STAND, 1)],
                sources: Vec::new(),
            });
            if let Some(gfx) = &mut self.gfx {
                self.scene.install_demo_animesh(&mut gfx.renderer);
            }
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
            worn.scale = Vec3::new(1.2, 0.12, 0.12);
            self.world.apply(NetEvent::ObjectUpdates {
                handle,
                objects: vec![worn],
            });
            let mut state = std::mem::take(&mut self.world.portals);
            let opened = if own {
                state.begin_arrival(id, root, floor, height, Instant::now())
            } else {
                let event = aurora_net::bridge::portal::Arrival {
                    id: uuid::Uuid::new_v4(),
                    creator: id,
                    region: handle,
                    position: root.to_array(),
                    floor,
                    height,
                    starts: crate::world::portals::arrival::unix_millis() + 250,
                };
                let opened = event.valid();
                if let Some(wire) = aurora_net::bridge::portal::Message::Arrival(event).encode() {
                    state.receive(
                        &self.world,
                        &format!("<auroraPortal>{id}|{wire}</auroraPortal>"),
                        Instant::now(),
                        unix_time(),
                    );
                }
                opened
            };
            self.world.portals = state;
            if own {
                self.camera.on_portal_arrival(&self.world.agent, &self.settings.camera);
            }
            log::info!("portal arrival demo: mode {mode}, opened {opened}, floor {floor}, terrain {terrain}, root {root:?}");
        }
        if self.portal_ui.demo_seeded {
            let root = self
                .world
                .objects
                .index_of_uuid(&actor)
                .and_then(|i| self.world.objects.get(i))
                .map_or(point, |o| o.position);
            if !own {
                self.camera.set_view(
                    root + Vec3::new(-4.0, -5.0, 3.0),
                    root + Vec3::Z * 1.1,
                    &mut self.world.agent,
                    &self.settings.camera,
                );
            }
            if [700, 740, 840].contains(&self.frame_count) {
                log::info!(
                    "portal arrival lifecycle: frame {}, effects {}, entries {}, offset {}",
                    self.frame_count,
                    self.world.portals.arrivals.len(),
                    self.world.portals.entries.len(),
                    self.world.portals.arrival_offset(actor, root, Instant::now())
                );
            }
            if own && [700, 710, 720, 730, 740].contains(&self.frame_count) {
                let below = self
                    .world
                    .portals
                    .arrivals
                    .get(&actor)
                    .is_some_and(|effect| self.camera.position.z < effect.center().z - 0.6);
                log::info!(
                    "portal own reveal: frame {}, below {}, camera {:?}, root {:?}",
                    self.frame_count,
                    below,
                    self.camera.position,
                    self.world.agent.position
                );
            }
        }
        true
    }

    pub(in crate::app) fn seed_portal_drop_demo(&mut self) -> bool {
        let Some(mode) = drop_mode().filter(|_| self.demo) else {
            return false;
        };
        if self.frame_count >= 420 && !self.portal_ui.demo_seeded {
            self.portal_ui.demo_seeded = true;
            self.portal_ui.open = false;
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.inventory = mode == "drop-focus";
            self.panels.places = mode.contains("places") || mode.contains("favorites") || mode.contains("history");
            self.panels.search = mode.contains("search");
            if self.panels.search {
                let tab = crate::world::search::Tab::Places;
                self.search_ui.open_tab(tab);
                self.search_ui.set_text(tab, "aurora");
                let requests = if self.settings.classic_search {
                    self.search_ui.request(tab, &self.settings.search).into_iter().collect()
                } else {
                    self.search_ui.aurora_requests(&self.settings.search)
                };
                let access = self.world.agent_access();
                for request in requests {
                    self.world.search.find(request, access, Instant::now());
                }
                ui::widgets::place_floater(&self.egui_ctx, ui::search::WINDOW, egui::pos2(20.0, 85.0), egui::vec2(720.0, 560.0));
                return true;
            }
            if self.panels.places {
                self.places_ui.open_tab(if mode.contains("history") {
                    2
                } else {
                    usize::from(!mode.contains("favorites"))
                });
                ui::widgets::place_floater(&self.egui_ctx, "places", egui::pos2(20.0, 85.0), egui::vec2(420.0, 560.0));
            }
            if self.panels.inventory {
                ui::widgets::place_floater(&self.egui_ctx, "inventory", egui::pos2(20.0, 85.0), egui::vec2(460.0, 560.0));
            }
            if mode.contains("history") {
                let region = self.world.main_region.unwrap_or_default();
                self.world.tp_storage.items.push(crate::world::tphistory::HistoryItem {
                    title: "Portal history destination".into(),
                    global: glam::DVec3::new((region >> 32) as f64 + 140.0, f64::from(region as u32) + 120.0, 25.0),
                    date: crate::world::portals::unix_time() as f64,
                    slurl: String::new(),
                });
                return true;
            }
            let (item, asset) = crate::demo::place::landmark_ids(0);
            self.world.inventory.add_items(vec![aurora_net::inventory::InvItem {
                id: item,
                parent: ui::places::system_folder(&self.world.inventory, if mode.contains("favorites") { 23 } else { 3 })
                    .unwrap_or(self.world.inventory.root),
                asset_id: if mode == "drop-invalid" {
                    uuid::Uuid::from_u128(0x0A77_0BAD)
                } else {
                    asset
                },
                asset_type: 3,
                inv_type: 3,
                owner: self.world.agent_id,
                name: aurora_i18n::tr!("common-landmark").into(),
                ..Default::default()
            }]);
        }
        if mode == "drop-focus" && self.frame_count == 562 {
            log::info!(
                "portal drop focus: inventory open {}, keyboard focus {}",
                self.panels.inventory,
                self.egui_ctx.egui_wants_keyboard_input()
            );
        }
        true
    }

    pub(in crate::app) fn portal_demo_input(&mut self, raw: &mut egui::RawInput) {
        if self.demo && std::env::var("AURORA_DEMO_PORTALS").is_ok_and(|mode| mode.starts_with("repeat-local")) {
            raw.focused = true;
            raw.events.retain(|event| !matches!(event, egui::Event::WindowFocused(false)));
            if self.frame_count == 420 {
                self.demo_release_keyboard();
            }
            let forward = Input::key(KeyCode::ArrowUp);
            if self.frame_count == 600 {
                self.down.insert(forward.clone());
            }
            if self.frame_count == 1500 {
                self.down.remove(&forward);
            }
            return;
        }
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_PORTALS")
            && (mode.starts_with("private-") && (mode.contains("-chat-") || mode.contains("-keyboard-")) || mode.starts_with("stranger"))
        {
            self.portal_keyboard_demo_input(raw, mode.contains("-chat-"));
            return;
        }
        let Some(mode) = drop_mode() else { return };
        if !(480..=560).contains(&self.frame_count) {
            return;
        }
        let bounds = self.egui_ctx.content_rect();
        let point = bounds.min + egui::vec2(bounds.width() * 0.65, bounds.height() * 0.62);
        raw.focused = true;
        raw.events.retain(|event| !matches!(event, egui::Event::WindowFocused(false)));
        let modifiers = egui::Modifiers {
            shift: mode.contains("public") && self.frame_count >= 500,
            ..Default::default()
        };
        raw.events.push(egui::Event::ModifiersChanged(modifiers));
        raw.events.push(egui::Event::PointerMoved(point));
        if self.frame_count == 480 || self.frame_count == 560 && !mode.ends_with("hover") {
            raw.events.push(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: self.frame_count == 480,
                modifiers,
            });
        }
        if self.frame_count == 482 {
            let item = crate::demo::place::landmark_ids(0).0;
            if mode.contains("search") {
                if let Some(row) = self
                    .world
                    .search
                    .list(crate::world::search::Tab::Places)
                    .and_then(|list| list.rows.first())
                    && let crate::world::search::Key::Id(id) = row.key
                {
                    egui::DragAndDrop::set_payload(
                        &self.egui_ctx,
                        ui::places::drag::PlaceDrag {
                            target: ui::places::drag::Target::Parcel(id),
                            favorite: None,
                            name: row.name.clone(),
                            thumbnail: None,
                        },
                    );
                }
            } else if mode.contains("history") {
                if let Some(place) = self.world.tp_storage.items.last() {
                    egui::DragAndDrop::set_payload(
                        &self.egui_ctx,
                        ui::places::drag::PlaceDrag {
                            target: ui::places::drag::Target::History(place.global),
                            favorite: None,
                            name: place.title.clone(),
                            thumbnail: None,
                        },
                    );
                }
            } else if mode.contains("places") || mode.contains("favorites") {
                egui::DragAndDrop::set_payload(
                    &self.egui_ctx,
                    ui::places::drag::PlaceDrag {
                        target: ui::places::drag::Target::Landmark(item),
                        favorite: mode.contains("favorites").then_some(item),
                        name: self
                            .world
                            .inventory
                            .items
                            .get(&item)
                            .map_or_else(String::new, |item| item.name.clone()),
                        thumbnail: None,
                    },
                );
            } else {
                egui::DragAndDrop::set_payload(&self.egui_ctx, ui::inventory::InvDrag(item));
            }
            if mode == "drop-focus" {
                self.egui_ctx
                    .memory_mut(|memory| memory.request_focus(egui::Id::unique(("inventory_clipboard_focus", self.inventory_ui.ui_id))));
            }
        }
    }

    fn portal_keyboard_demo_input(&mut self, raw: &mut egui::RawInput, chat: bool) {
        raw.focused = true;
        raw.events.retain(|event| !matches!(event, egui::Event::WindowFocused(false)));
        if self.frame_count == 420 {
            self.panels.chat = false;
            self.chat_ui.input.clear();
            if chat {
                self.chat_ui.focus_request = true;
            } else {
                self.demo_release_keyboard();
            }
            if std::env::var("AURORA_DEMO_PORTALS").is_ok_and(|mode| mode.contains("-camera-")) {
                // Keep an opposing orbit through approach, with both ordinary
                // reset preferences off. Arrival alone must put it behind us.
                self.settings.camera.reset_on_teleport = false;
                self.settings.camera.reset_on_movement = false;
                self.camera.demo_orbit(std::f32::consts::PI, 0.0, None, &self.settings.camera);
                self.always_run = true;
            }
        }
        let forward = Input::key(KeyCode::ArrowUp);
        if self.frame_count == 600 {
            self.down.insert(forward.clone());
        }
        if self.frame_count == 810 {
            self.down.remove(&forward);
        }
        if (600..=840).contains(&self.frame_count) && self.frame_count.is_multiple_of(60) {
            log::info!(
                "portal keyboard demo: frame {}, chat {}, keyboard focus {}, forward {}, position {:?}, authorized {}, journey {}, question {}",
                self.frame_count,
                self.chat_ui.has_focus,
                self.egui_ctx.egui_wants_keyboard_input(),
                self.input.forward,
                self.world.agent.position,
                self.world.portals.entries.iter().any(|entry| entry.allowed(&self.world)),
                self.portal_journey.is_some(),
                self.portal_ask.is_some()
            );
        }
    }

    pub(in crate::app) fn seed_portal_peer_demo(&mut self, mode: &str, mut portal: Portal, owner: uuid::Uuid) {
        // The near-keyboard scenarios exercise the one-way emergence and camera.
        if mode.contains("-near-keyboard-") {
            self.settings.portal_returns = false;
        }
        let mut world = World::new(self.world.avatar_lib.clone());
        world.agent_id = owner;
        world.main_region = self.world.main_region;
        world.agent.position = self.world.agent.position;
        let granted = mode.ends_with("allowed");
        if mode.contains("group") {
            let group = uuid::Uuid::from_u128(0xA770_6000);
            portal.access = Access::Group(group);
            let membership = || aurora_net::GroupMembership {
                id: group,
                name: "Demo group".into(),
                insignia: uuid::Uuid::nil(),
                powers: 0,
                accept_notices: true,
                list_in_profile: true,
                contribution: 0,
            };
            world.groups.groups.push(membership());
            if granted {
                self.world.groups.groups.push(membership());
            }
        } else if granted {
            world.social.form_friendship(self.world.agent_id);
            self.world.social.form_friendship(owner);
        }
        let mut state = crate::world::portals::State::default();
        let now = Instant::now();
        state.sync(&world, None, unix_time());
        // the peer stands on our plaza: the same land test as ours
        world.parcel = self.world.parcel.clone();
        let build = world.build_right_at(world.main_region.unwrap_or(256), Vec3::from(portal.center), None);
        if let Err(refusal) = state.create(portal, build, world.agent.position, now, unix_time()) {
            log::warn!("portal peer demo: {refusal:?}");
        }
        if let Some(group) = state.take_group_refresh() {
            world.group_data.request_members(group);
            for command in world.group_data.take_commands() {
                if let NetCommand::GroupMembersRequest { request, .. } = command {
                    let members = std::iter::once(owner)
                        .chain(granted.then_some(self.world.agent_id))
                        .map(|id| aurora_net::groups::GroupMember {
                            id,
                            contribution: 0,
                            powers: 0,
                            title: String::new(),
                            last_seen: aurora_net::groups::LastSeen::Online,
                            is_owner: id == owner,
                        })
                        .collect();
                    world
                        .group_data
                        .on_event(&NetEvent::GroupMembers(Box::new(aurora_net::groups::MembersPage {
                            group,
                            request,
                            total: if granted { 2 } else { 1 },
                            members,
                            whole: true,
                        })));
                }
            }
        }
        self.portal_demo_peer = Some(Box::new(Peer {
            world,
            state,
            endpoint: Endpoint {
                url: String::new(),
                token: uuid::Uuid::from_u128(0x00A7_70E0),
            },
        }));
    }

    pub(in crate::app) fn exchange_portal_demo(&mut self) {
        let Some(peer) = &mut self.portal_demo_peer else { return };
        let peer = peer.as_mut();
        let now = Instant::now();
        let departing = std::env::var("AURORA_DEMO_PORTALS").is_ok_and(|mode| mode.contains("-departing-"));
        peer.state.sync(&peer.world, None, unix_time());
        if let Some(NetCommand::BridgeQuery {
            request,
            query: Query::Portal { target, message },
            ..
        }) = self.world.portals.command(Some(&peer.endpoint), now)
        {
            self.world.portals.reply(request, &Ok(Reply::Portal), now);
            if (target.is_nil() || target == peer.world.agent_id)
                && let Some(wire) = message.encode()
            {
                if departing && matches!(&message, aurora_net::bridge::portal::Message::Request { .. }) {
                    peer.world.teleporting = true;
                    peer.state.sync(&peer.world, None, unix_time());
                }
                peer.state.receive(
                    &peer.world,
                    &format!("<auroraPortal>{}|{wire}</auroraPortal>", self.world.agent_id),
                    now,
                    unix_time(),
                );
            }
        }
        peer.state.update(&peer.world, Vec3::ZERO, now);
        if let Some(NetCommand::BridgeQuery {
            request,
            query: Query::Portal { target, message },
            ..
        }) = peer.state.command(Some(&peer.endpoint), now)
        {
            peer.state.reply(request, &Ok(Reply::Portal), now);
            if (target.is_nil() || target == self.world.agent_id)
                && let Some(wire) = message.encode()
            {
                let mut state = std::mem::take(&mut self.world.portals);
                state.receive(
                    &self.world,
                    &format!("<auroraPortal>{}|{wire}</auroraPortal>", peer.world.agent_id),
                    now,
                    unix_time(),
                );
                self.world.portals = state;
                if departing && matches!(&message, aurora_net::bridge::portal::Message::Details { .. }) {
                    peer.world.main_region = Some(512);
                }
            }
        }
    }
}
