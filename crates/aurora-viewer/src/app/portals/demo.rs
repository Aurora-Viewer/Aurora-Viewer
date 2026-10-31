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
    pub(in crate::app) fn arrival_portal_demo(&mut self, mode: &str) -> bool {
        if !mode.starts_with("arrival-") {
            return false;
        }
        let point = self.world.agent.position + self.world.agent.forward() * 4.0;
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
            self.settings.reduce_motion = mode == "arrival-static";
            self.world.portals.reduced_motion = self.settings.reduce_motion;
            let floor = self.scene.preview_floor(&self.world, point, terrain + 4.0).unwrap_or(terrain);
            let mut peer = crate::demo::poser::peer();
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
            let event = aurora_net::bridge::portal::Arrival {
                id: uuid::Uuid::new_v4(),
                region: handle,
                position: root.to_array(),
                floor,
                height,
                starts: crate::world::portals::arrival::unix_millis() + 250,
            };
            let opened = event.valid();
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
            log::info!("portal arrival demo: mode {mode}, opened {opened}, floor {floor}, terrain {terrain}, root {root:?}");
        }
        if self.portal_ui.demo_seeded {
            let root = self
                .world
                .objects
                .index_of_uuid(&crate::demo::poser::PEER)
                .and_then(|i| self.world.objects.get(i))
                .map_or(point, |o| o.position);
            self.camera.set_view(
                root + Vec3::new(-4.0, -5.0, 3.0),
                root + Vec3::Z * 1.1,
                &mut self.world.agent,
                &self.settings.camera,
            );
            if [700, 740, 840].contains(&self.frame_count) {
                log::info!(
                    "portal arrival lifecycle: frame {}, effects {}, entries {}, offset {}",
                    self.frame_count,
                    self.world.portals.arrivals.len(),
                    self.world.portals.entries.len(),
                    self.world.portals.arrival_offset(crate::demo::poser::PEER, root, Instant::now())
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
            if self.panels.inventory {
                ui::widgets::place_floater(&self.egui_ctx, "inventory", egui::pos2(20.0, 85.0), egui::vec2(460.0, 560.0));
            }
            let (item, asset) = crate::demo::place::landmark_ids(0);
            self.world.inventory.add_items(vec![aurora_net::inventory::InvItem {
                id: item,
                parent: self.world.inventory.root,
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
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_PORTALS")
            && mode.starts_with("private-")
            && (mode.contains("-chat-") || mode.contains("-keyboard-"))
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
            egui::DragAndDrop::set_payload(&self.egui_ctx, ui::inventory::InvDrag(crate::demo::place::landmark_ids(0).0));
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
                "portal keyboard demo: frame {}, chat {}, keyboard focus {}, forward {}, position {:?}, authorized {}, journey {}",
                self.frame_count,
                self.chat_ui.has_focus,
                self.egui_ctx.egui_wants_keyboard_input(),
                self.input.forward,
                self.world.agent.position,
                self.world.portals.entries.iter().any(|entry| entry.allowed(&self.world)),
                self.portal_journey.is_some()
            );
        }
    }

    pub(in crate::app) fn seed_portal_peer_demo(&mut self, mode: &str, mut portal: Portal, owner: uuid::Uuid) {
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
        state.create(portal, world.agent.position, now, unix_time());
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
