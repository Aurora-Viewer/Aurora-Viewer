//! Place drops reuse landmark assets or immutable history arrivals with the
//! ordinary region-map resolver. No inventory item is created or consumed.
use super::*;
use crate::world::{
    inventory::{Inventory, actions},
    landmarks::Resolved,
};
use aurora_net::inventory::InvItem;

pub(in crate::app) struct Pending {
    destination: DropDestination,
    agent: uuid::Uuid,
    region: u64,
    center: Vec3,
    yaw: f32,
    access: Access,
    started: Instant,
}

pub(in crate::app) fn cargo(inv: &Inventory, item: uuid::Uuid) -> Option<&InvItem> {
    let id = actions::original(inv, item)?;
    inv.items
        .get(&id)
        .filter(|item| item.asset_type == 3 && !inv.in_folder_type(id, 53))
}

pub(in crate::app) fn access(shift: bool) -> Access {
    if shift { Access::Everyone } else { Access::Friends }
}

#[derive(Clone, Copy)]
enum DropDestination {
    Landmark { item: uuid::Uuid, asset: uuid::Uuid },
    History(glam::DVec3),
}

impl DropDestination {
    fn error_key(self) -> &'static str {
        match self {
            Self::Landmark { .. } => "portal-landmark-unavailable",
            Self::History(_) => "portal-place-unavailable",
        }
    }

    fn resolve(self, world: &mut World) -> Result<Option<aurora_net::bridge::portal::Destination>, ()> {
        match self {
            Self::Landmark { asset, .. } => destination(world, asset),
            Self::History(global) => destination_at(world, global),
        }
    }
}

fn destination(world: &mut World, asset: uuid::Uuid) -> Result<Option<aurora_net::bridge::portal::Destination>, ()> {
    match world.landmarks.resolve(asset) {
        Resolved::Failed => Err(()),
        Resolved::Pending => Ok(None),
        Resolved::Ready { global, .. } => destination_at(world, global),
    }
}

fn destination_at(world: &mut World, global: glam::DVec3) -> Result<Option<aurora_net::bridge::portal::Destination>, ()> {
    if !global.is_finite() || global.x < 0.0 || global.y < 0.0 {
        return Err(());
    }
    if let Some(((x, y), sim)) = world.map.sim_at_global(global.x, global.y) {
        if sim.is_down() {
            return Err(());
        }
        // Modulo 256 loses the internal coordinates of variable regions.
        Ok(Some(aurora_net::bridge::portal::Destination {
            region: sim.name.clone(),
            position: [
                (global.x - f64::from(x) * 256.0) as f32,
                (global.y - f64::from(y) * 256.0) as f32,
                global.z as f32,
            ],
        }))
    } else {
        let (x, y) = ((global.x / 256.0) as i64, (global.y / 256.0) as i64);
        world.map.want_regions(x, y, x, y);
        Ok(None)
    }
}

impl Pending {
    fn valid(&self, world: &World, now: Instant) -> bool {
        !world.teleporting
            && self.agent == world.agent_id
            && Some(self.region) == world.main_region
            && now.saturating_duration_since(self.started) <= Duration::from_secs(30)
            && match self.destination {
                DropDestination::Landmark { item, asset } => cargo(&world.inventory, item).is_some_and(|item| item.asset_id == asset),
                DropDestination::History(global) => global.is_finite() && global.x >= 0.0 && global.y >= 0.0,
            }
    }
}

impl App {
    pub(in crate::app) fn queue_landmark_portal(&mut self, item: uuid::Uuid, point: Vec3, shift: bool) {
        let Some(cargo) = cargo(&self.world.inventory, item).filter(|item| !item.asset_id.is_nil()) else {
            self.portal_drop_notice("portal-landmark-unavailable");
            return;
        };
        let destination = DropDestination::Landmark {
            item: cargo.id,
            asset: cargo.asset_id,
        };
        self.queue_place_portal(destination, point, shift);
    }

    pub(in crate::app) fn queue_history_portal(&mut self, global: glam::DVec3, point: Vec3, shift: bool) {
        self.queue_place_portal(DropDestination::History(global), point, shift);
    }

    fn queue_place_portal(&mut self, destination: DropDestination, point: Vec3, shift: bool) {
        let Some(region) = self.world.main_region else { return };
        self.portal_drop = Some(Pending {
            destination,
            agent: self.world.agent_id,
            region,
            center: point + Vec3::Z * crate::world::portals::HEIGHT,
            yaw: self.world.agent.yaw,
            access: access(shift),
            started: Instant::now(),
        });
        self.portal_ui.notice = None;
    }

    pub(in crate::app) fn portal_drop_notice(&mut self, key: &str) {
        let message = match key {
            "portal-invalid" => aurora_i18n::tr!("portal-invalid"),
            "portal-bridge-required" => aurora_i18n::tr!("portal-bridge-required"),
            "portal-landmark-unavailable" => aurora_i18n::tr!("portal-landmark-unavailable"),
            "portal-place-unavailable" => aurora_i18n::tr!("portal-place-unavailable"),
            _ => aurora_i18n::tr!("rez-no-surface"),
        };
        self.portal_ui.notice = Some(message.into());
        self.world.system_notice(message, crate::world::notice_topics::Topic::Items);
    }

    pub(in crate::app) fn update_place_portal(&mut self) {
        let Some(pending) = self.portal_drop.take() else { return };
        if self.world.teleporting || pending.agent != self.world.agent_id || Some(pending.region) != self.world.main_region {
            return;
        }
        if !pending.valid(&self.world, Instant::now()) {
            self.portal_drop_notice(pending.destination.error_key());
            return;
        }
        match pending.destination.resolve(&mut self.world) {
            Err(()) => self.portal_drop_notice(pending.destination.error_key()),
            Ok(None) => self.portal_drop = Some(pending),
            Ok(Some(destination)) => self.create_portal_at(
                destination.region,
                Vec3::from(destination.position),
                pending.access,
                pending.center,
                pending.yaw,
            ),
        }
    }
}

pub(in crate::app) fn preview(ctx: &egui::Context, p: &Palette, point: egui::Pos2, shift: bool, ready: bool) {
    let offset = if egui::DragAndDrop::has_payload_of_type::<ui::places::drag::PlaceDrag>(ctx) {
        68.0
    } else {
        20.0
    };
    egui::Area::new(egui::Id::unique("portal_landmark_preview"))
        .order(egui::Order::Tooltip)
        .fixed_pos(point + egui::vec2(16.0, offset))
        .interactable(false)
        .show(ctx, |ui| {
            ui::widgets::overlay(ui, p, egui::Margin::same(8), |ui| {
                ui.label(if !ready {
                    aurora_i18n::tr!("portal-bridge-required")
                } else if shift {
                    aurora_i18n::tr!("portal-drop-public")
                } else {
                    aurora_i18n::tr!("portal-drop-friends")
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use crate::world::worldmap::SimInfo;

    #[test]
    fn portal_drag_preview_leaves_world_pointer_available_and_reads_shift_at_release() {
        let ctx = egui::Context::default();
        let point = egui::pos2(700.0, 400.0);
        let frame = |shift: bool, pressed: Option<bool>| {
            let modifiers = egui::Modifiers {
                shift,
                ..Default::default()
            };
            let mut read = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                    events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(point)]
                        .into_iter()
                        .chain(pressed.map(|pressed| egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        }))
                        .collect(),
                    ..Default::default()
                },
                |ui| {
                    read = ui::inventory::world_drag(ui.ctx());
                    preview(ui.ctx(), &crate::theme::Theme::default().palette(), point, shift, true);
                },
            );
            output.textures_delta.clear();
            read
        };
        // Stabilize the tooltip sizing pass, then use an actual held drag
        // before releasing it, as a resident's inventory interaction does.
        for _ in 0..3 {
            frame(false, None);
        }
        frame(false, Some(true));
        egui::DragAndDrop::set_payload(&ctx, ui::inventory::InvDrag(uuid::Uuid::from_u128(2)));
        for shift in [false, true, false, true] {
            let drag = frame(shift, None).expect("preview never intercepts the world pointer");
            assert_eq!(drag.shift, shift);
            assert_eq!(access(drag.shift), if shift { Access::Everyone } else { Access::Friends });
        }
        let drag = frame(true, Some(false)).expect("release reaches the world");
        assert!(drag.released && drag.shift);
    }

    #[test]
    fn landmark_portal_resolves_links_and_both_landmark_formats_without_consuming_inventory_or_losing_variable_region_offsets() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = uuid::Uuid::from_u128(1);
        world.main_region = Some(256);
        let id = uuid::Uuid::from_u128(2);
        let asset = uuid::Uuid::from_u128(3);
        let link = uuid::Uuid::from_u128(4);
        world.inventory.add_items(vec![
            InvItem {
                id,
                asset_id: asset,
                asset_type: 3,
                ..Default::default()
            },
            InvItem {
                id: link,
                asset_id: id,
                asset_type: 24,
                ..Default::default()
            },
        ]);
        assert_eq!(cargo(&world.inventory, link).unwrap().asset_id, asset);
        assert_eq!(access(false), Access::Friends);
        assert_eq!(access(true), Access::Everyone);
        assert_eq!(destination(&mut world, asset), Ok(None));
        assert_eq!(world.landmarks.take_wanted().len(), 1);
        world.landmarks.on_data(asset, Some(b"Landmark version 1\nposition 700 900 30\n"));
        assert_eq!(destination(&mut world, asset), Ok(None));
        assert_eq!(
            world
                .map
                .take_commands()
                .iter()
                .filter(|c| matches!(c, NetCommand::MapBlockRequest { .. }))
                .count(),
            1
        );
        world.map.sims.insert(
            (1, 2),
            SimInfo {
                name: "Demo wide region".into(),
                access: 13,
                flags: 0,
                image: uuid::Uuid::nil(),
                size_x: 512,
                size_y: 512,
            },
        );
        let expected = Some(aurora_net::bridge::portal::Destination {
            region: "Demo wide region".into(),
            position: [444.0, 388.0, 30.0],
        });
        assert_eq!(destination(&mut world, asset), Ok(expected.clone()));
        let region_id = uuid::Uuid::from_u128(5);
        let data = format!("Landmark version 2\nregion_id {region_id}\nlocal_pos 444 388 30\n");
        world.landmarks.on_data(asset, Some(data.as_bytes()));
        world.landmarks.set_local_region(region_id, (256u64 << 32) | 512);
        assert_eq!(destination(&mut world, asset), Ok(expected));
        assert_eq!(world.inventory.items.len(), 2);
        let now = Instant::now();
        let pending = Pending {
            destination: DropDestination::Landmark { item: id, asset },
            agent: world.agent_id,
            region: 256,
            center: Vec3::ONE,
            yaw: 0.0,
            access: Access::Friends,
            started: now,
        };
        assert!(pending.valid(&world, now));
        assert!(!pending.valid(&world, now + Duration::from_secs(31)));
        world.main_region = Some(512);
        assert!(!pending.valid(&world, now));
        world.main_region = Some(256);
        world.inventory.items.get_mut(&id).unwrap().asset_id = uuid::Uuid::from_u128(6);
        assert!(!pending.valid(&world, now));
        world.landmarks.on_data(asset, None);
        assert_eq!(destination(&mut world, asset), Err(()));
    }

    #[test]
    fn history_portal_resolves_the_arrival_without_inventory_and_retains_session_guards() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = uuid::Uuid::from_u128(1);
        world.main_region = Some(256);
        let global = glam::DVec3::new(701.0, 901.0, 32.0);
        let source = DropDestination::History(global);
        assert_eq!(source.resolve(&mut world), Ok(None));
        assert!(world.landmarks.take_wanted().is_empty());
        assert!(world.inventory.items.is_empty());
        assert!(
            world
                .map
                .take_commands()
                .iter()
                .any(|c| matches!(c, NetCommand::MapBlockRequest { .. }))
        );
        world.map.sims.insert(
            (1, 2),
            SimInfo {
                name: "Demo wide region".into(),
                access: 13,
                flags: 0,
                image: uuid::Uuid::nil(),
                size_x: 512,
                size_y: 512,
            },
        );
        let destination = source.resolve(&mut world).unwrap().unwrap();
        assert_eq!(destination.region, "Demo wide region");
        assert_eq!(destination.position, [445.0, 389.0, 32.0]);
        assert!(DropDestination::History(glam::DVec3::NAN).resolve(&mut world).is_err());
        let now = Instant::now();
        let pending = Pending {
            destination: source,
            agent: world.agent_id,
            region: 256,
            center: Vec3::ONE,
            yaw: 0.0,
            access: access(true),
            started: now,
        };
        world.tp_storage.items.clear();
        assert!(pending.valid(&world, now), "the drag retains its immutable arrival snapshot");
        assert!(!pending.valid(&world, now + Duration::from_secs(31)));
        world.teleporting = true;
        assert!(!pending.valid(&world, now));
        world.teleporting = false;
        world.main_region = Some(512);
        assert!(!pending.valid(&world, now));
        world.main_region = Some(256);
        world.agent_id = uuid::Uuid::from_u128(2);
        assert!(!pending.valid(&world, now));
        assert!(world.inventory.items.is_empty());
    }
}
