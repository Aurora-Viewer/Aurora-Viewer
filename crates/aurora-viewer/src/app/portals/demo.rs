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

    pub(in crate::app) fn portal_drop_demo_input(&mut self, raw: &mut egui::RawInput) {
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
            }
        }
    }
}
