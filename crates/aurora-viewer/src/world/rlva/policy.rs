//! Enforce outgoing actions at the application's common send boundary.

use super::super::World;
use aurora_net::{NetCommand, control};
use glam::Vec3;
use uuid::Uuid;

impl World {
    pub fn rlva_can_touch(&self, handle: aurora_net::RegionHandle, local_id: u32) -> bool {
        let r = &self.rlva;
        if !r.enabled() {
            return true;
        }
        if r.has("interact") {
            return false;
        }
        let Some(index) = self.objects.index_of(&super::super::objects::ObjKey { region: handle, local_id }) else {
            return false;
        };
        let mut root = index;
        let mut wearer = None;
        for _ in 0..64 {
            let Some(object) = self.objects.get(root) else {
                return false;
            };
            let Some(parent) = self.objects.parent_of(object).and_then(|i| self.objects.get(i).map(|o| (i, o))) else {
                break;
            };
            if parent.1.is_avatar() {
                wearer = Some(parent.1.full_id);
                break;
            }
            root = parent.0;
        }
        let Some(object) = self.objects.get(root) else {
            return false;
        };
        let id = object.full_id.to_string();
        let hud = wearer == Some(self.agent_id) && self.avatar_lib.attach_points.get(&object.attachment_point()).is_some_and(|p| p.hud);
        if r.targets("touchthis", &id) || (!hud && r.has("touchall")) {
            return false;
        }
        if hud {
            return r.allows("touchhud", &id);
        }
        match wearer {
            None if !r.allows("touchworld", &id) => return false,
            Some(w) if w == self.agent_id => {
                if !r.allows("touchattach", &id) || (r.has("touchattachself") && !r.targets("touchattach", &id)) {
                    return false;
                }
            }
            Some(w)
                if ((!r.allows("touchattach", &id) || r.has("touchattachother"))
                    && !r.targets("touchattach", &w.to_string())
                    && !r.targets("touchattach", &id))
                    || r.targets("touchattachother", &w.to_string()) =>
            {
                return false;
            }
            _ => {}
        }
        if wearer != Some(self.agent_id) {
            let max = r.distance_limit("fartouch", 1.5, Uuid::nil());
            if max.is_finite()
                && !crate::scene::Scene::object_transform_unshifted(self, index, std::time::Instant::now())
                    .is_some_and(|(p, _, _)| p.distance(self.agent.position) <= max)
            {
                return false;
            }
        }
        true
    }
    pub fn rlva_allows_command(&self, command: &NetCommand, actor: Uuid) -> bool {
        let r = &self.rlva;
        if !r.enabled() {
            return true;
        }
        let blocked = |name| r.has_except(name, actor);
        match command {
            NetCommand::SendIm { to, .. } => r.allows_im("sendim", *to),
            NetCommand::SendImDialog {
                to,
                dialog: 0 | 17 | 41 | 42,
                ..
            } => r.allows_im("sendim", *to),
            NetCommand::SendImDialog { to, dialog: 4, .. } => r.allows("share", &to.to_string()),
            NetCommand::ActivateGroup(_) => !blocked("setgroup"),
            NetCommand::TeleportHome | NetCommand::TeleportLandmark(_) => {
                !blocked("tplm") && !(self.agent.is_sitting() && blocked("unsit"))
            }
            NetCommand::TeleportTo { handle, position, .. } => {
                let local = self.main_region == Some(*handle) && position.distance(self.agent.position) <= 256.0;
                !blocked(if local { "tplocal" } else { "tploc" }) && !(self.agent.is_sitting() && blocked("unsit"))
            }
            NetCommand::TeleportToRegion { name, position } => {
                let local = self.main().is_some_and(|region| region.name.eq_ignore_ascii_case(name))
                    && position.distance(self.agent.position) <= 256.0;
                !blocked(if local { "tplocal" } else { "tploc" }) && !(self.agent.is_sitting() && blocked("unsit"))
            }
            NetCommand::RequestSit { target, .. } => {
                let limit = r.distance_limit("sittp", 1.5, actor);
                !blocked("sit")
                    && !(self.agent.is_sitting() && blocked("unsit"))
                    && (!limit.is_finite()
                        || self
                            .objects
                            .index_of_uuid(target)
                            .and_then(|i| crate::scene::Scene::object_transform_unshifted(self, i, std::time::Instant::now()))
                            .is_some_and(|(pos, _, _)| pos.distance(self.agent.position) <= limit))
            }
            NetCommand::OneShotControl(flags) => {
                (flags & control::STAND_UP == 0 || !blocked("unsit")) && (flags & control::SIT_ON_GROUND == 0 || !blocked("sitground"))
            }
            NetCommand::SetAlwaysRun(true) => !blocked("alwaysrun"),
            NetCommand::BuyObject { .. } => !blocked("buy"),
            NetCommand::PayObject { .. } | NetCommand::PayResident { .. } => !blocked("pay"),
            NetCommand::Build(
                aurora_net::build::BuildCmd::Add(_)
                | aurora_net::build::BuildCmd::RezInventory(_)
                | aurora_net::build::BuildCmd::Duplicate { .. },
            ) => !blocked("rez"),
            NetCommand::DetachAttachments(items) => items.iter().all(|item| self.rlva_can_remove(*item, actor)),
            NetCommand::RezAttachments(items) => items.iter().all(|item| {
                let at = if item.point == 0 {
                    self.inventory.items.get(&item.item_id).map_or(0, |it| (it.flags & 0xff) as u8)
                } else {
                    item.point
                };
                self.rlva_can_add(item.item_id, item.point, actor)
                    && (item.add
                        || self
                            .worn_attachment_points()
                            .iter()
                            .filter(|(_, p)| **p == at && at != 0)
                            .all(|(id, _)| self.rlva_can_remove(*id, actor)))
            }),
            NetCommand::UpdateOutfit { change, .. } => {
                let Some(cof) = super::super::appearance::cof(&self.inventory) else {
                    return true;
                };
                let old = super::super::appearance::folder_links(&self.inventory, cof);
                if change.folder == cof {
                    self.rlva_outfit_allowed(&old, &change.links, actor)
                } else {
                    change
                        .cof
                        .as_ref()
                        .is_none_or(|(_, links)| self.rlva_outfit_allowed(&old, links, actor))
                }
            }
            NetCommand::Touch { local_id } => self.main_region.is_some_and(|h| self.rlva_can_touch(h, *local_id)),
            NetCommand::ObjectGrab { handle, local_id, .. } => self.rlva_can_touch(*handle, *local_id),
            _ => true,
        }
    }

    /// Queries requiring world state and force commands are consumed in their
    /// original order. The issuing object may bypass only its own locks.
    pub fn rlva_pending(&mut self) -> Vec<(Uuid, NetCommand)> {
        let mut commands = Vec::new();
        for (source, c) in std::mem::take(&mut self.rlva.pending) {
            if !self.rlva.enabled() {
                break;
            }
            if c.behaviour.starts_with("getcam_") || c.behaviour.starts_with("setcam_") {
                if self.rlva.viewer.len() < 256 {
                    self.rlva.viewer.push((source, c));
                }
                continue;
            }
            if let super::Param::Reply(channel) = c.param {
                let text = match c.behaviour.as_str() {
                    "getgroup" if c.option.is_empty() => Some(
                        self.groups
                            .groups
                            .iter()
                            .find(|g| g.id == self.groups.active)
                            .map_or("none", |g| g.name.as_str())
                            .to_owned(),
                    ),
                    "getsitid" if c.option.is_empty() => Some(
                        self.objects
                            .index_of_uuid(&self.agent_id)
                            .and_then(|i| self.objects.get(i))
                            .and_then(|o| self.objects.parent_of(o))
                            .and_then(|i| self.objects.get(i))
                            .map_or(Uuid::nil(), |o| o.full_id)
                            .to_string(),
                    ),
                    name if super::QUERIES.contains(&name) => self.rlva_inventory_query(&c),
                    _ => Some(String::new()),
                };
                if let Some(text) = text {
                    self.rlva.reply(channel, text);
                } else {
                    self.rlva.pending.push((source, c));
                }
                continue;
            }
            if matches!(
                c.behaviour.as_str(),
                "attach" | "attachover" | "attachall" | "attachallover" | "detach" | "detachall" | "detachme" | "remattach" | "remoutfit"
            ) {
                if !self.rlva_inventory_force(source, &c) {
                    self.rlva.pending.push((source, c));
                }
                continue;
            }
            let command = match c.behaviour.as_str() {
                "unsit" if c.option.is_empty() => Some(NetCommand::OneShotControl(control::STAND_UP)),
                "sitground" if c.option.is_empty() => Some(NetCommand::OneShotControl(control::SIT_ON_GROUND)),
                "sit" => Uuid::parse_str(&c.option)
                    .ok()
                    .and_then(|id| self.objects.index_of_uuid(&id))
                    .and_then(|i| self.objects.get(i))
                    .map(|o| NetCommand::RequestSit {
                        handle: o.key.region,
                        target: o.full_id,
                        offset: Vec3::ZERO,
                    }),
                "setgroup" => Uuid::parse_str(&c.option).ok().map(NetCommand::ActivateGroup),
                "setrot" => {
                    if let Ok(angle) = c.option.parse::<f32>()
                        && angle.is_finite()
                    {
                        self.agent.yaw = std::f32::consts::FRAC_PI_2 - angle;
                    }
                    None
                }
                "fly" => {
                    let flying = match c.option.as_str() {
                        "" | "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    };
                    if let Some(flying) = flying
                        && (!flying || !self.rlva.has_except("fly", source))
                    {
                        self.agent.flying = flying;
                    }
                    None
                }
                "tpto" => teleport(&c.option),
                _ => None,
            };
            if let Some(command) = command
                && self.rlva_allows_command(&command, source)
            {
                commands.push((source, command));
            }
        }
        commands
    }
}

fn teleport(option: &str) -> Option<NetCommand> {
    let destination = option.split(';').next()?;
    let parts: Vec<_> = destination.split('/').collect();
    let (name, coordinates) = match parts.as_slice() {
        [x, y, z] => (None, [*x, *y, *z]),
        [region, x, y, z] if !region.is_empty() => (Some(*region), [*x, *y, *z]),
        _ => return None,
    };
    let p = Vec3::new(
        coordinates[0].parse().ok()?,
        coordinates[1].parse().ok()?,
        coordinates[2].parse().ok()?,
    );
    if !p.is_finite() || p.x < 0.0 || p.y < 0.0 {
        return None;
    }
    if let Some(name) = name {
        Some(NetCommand::TeleportToRegion {
            name: name.to_owned(),
            position: p,
        })
    } else {
        if p.x >= u32::MAX as f32 || p.y >= u32::MAX as f32 {
            return None;
        }
        let x = (p.x as u32 / 256) * 256;
        let y = (p.y as u32 / 256) * 256;
        Some(NetCommand::TeleportTo {
            handle: aurora_net::origin_to_handle(x, y),
            position: Vec3::new(p.x - x as f32, p.y - y as f32, p.z),
            look_at: Vec3::X,
        })
    }
}
