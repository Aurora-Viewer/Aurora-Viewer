//! Enforce outgoing actions at the application's common send boundary.

use super::super::World;
use aurora_net::{NetCommand, control};
use glam::Vec3;
use uuid::Uuid;

impl World {
    pub fn rlva_allows_im(&self, behaviour: &str, target: Uuid) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let distance = self
            .objects
            .index_of_uuid(&target)
            .and_then(|i| crate::scene::Scene::object_transform_unshifted(self, i, std::time::Instant::now()))
            .map(|(p, _, _)| p.distance(self.agent.position));
        distance.map_or_else(
            || self.rlva.allows_im(behaviour, target),
            |d| self.rlva.allows_im_at(behaviour, target, Some(d)),
        )
    }
    pub fn rlva_can_touch(&self, handle: aurora_net::RegionHandle, local_id: u32) -> bool {
        self.rlva_can_touch_at(handle, local_id, None)
    }

    /// `position` is in the viewer's region-relative world, like a picking
    /// ray. Measure the clicked surface, not the object's bounding-box centre.
    pub fn rlva_can_touch_at(&self, handle: aurora_net::RegionHandle, local_id: u32, position: Option<Vec3>) -> bool {
        let r = &self.rlva;
        if !r.enabled() {
            return true;
        }
        let Some(index) = self.objects.index_of(&super::super::objects::ObjKey { region: handle, local_id }) else {
            return false;
        };
        if !self.rlva_object_visible(index) {
            return false;
        }
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
        if r.rules_for("touchme", Uuid::nil()).any(|(source, _)| {
            let Some(mut i) = self.objects.index_of_uuid(&source) else {
                return false;
            };
            for _ in 0..64 {
                if i == root {
                    return true;
                }
                let Some(parent) = self.objects.get(i).and_then(|o| self.objects.parent_of(o)) else {
                    break;
                };
                i = parent;
            }
            false
        }) {
            return true;
        }
        if !hud && r.has("interact") {
            return false;
        }
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
                && !position
                    .filter(|p| p.is_finite())
                    .or_else(|| crate::scene::Scene::object_transform_unshifted(self, index, std::time::Instant::now()).map(|(p, _, _)| p))
                    .is_some_and(|p| p.distance(self.agent.position) <= max)
            {
                return false;
            }
        }
        true
    }

    fn rlva_teleport_delta_allowed(&self, delta: Vec3, actor: Uuid) -> bool {
        let r = &self.rlva;
        if self.agent.is_sitting() && r.has_except("unsit", actor) {
            return false;
        }
        if delta.truncate().length_squared() < 256.0 * 256.0 {
            delta.length() < r.distance_limit("sittp", 1.5, actor)
                && (!r.has_except("tplocal", actor) || delta.truncate().length() < r.distance_limit("tplocal", 256.0, actor).min(256.0))
        } else {
            !r.has_except("tploc", actor)
        }
    }
    pub fn rlva_allows_command(&self, command: &NetCommand, actor: Uuid) -> bool {
        let r = &self.rlva;
        if !r.enabled() {
            return true;
        }
        if !actor.is_nil() && r.controllers.blocked(actor) {
            return false;
        }
        let blocked = |name| r.has_except(name, actor);
        match command {
            NetCommand::AgentAnimation { anim, start: true } if *anim == super::super::radar::ANIM_AGENT_TYPE => r.can_send_typing_start(),
            NetCommand::CreateInventoryItem {
                kind: aurora_net::inventory::operations::NewItem::Landmark,
                ..
            } => !r.has("showloc"),
            NetCommand::OfferTeleport { to, .. } => to.iter().all(|id| self.rlva_can_offer_teleport(*id)),
            NetCommand::PickUpdate(pick) if r.has("showloc") => self
                .profiles
                .picks
                .get(&pick.id)
                .is_some_and(|old| old.pos_global == pick.pos_global),
            NetCommand::ClassifiedUpdate(classified) if r.has("showloc") => self
                .profiles
                .classifieds
                .get(&classified.id)
                .is_some_and(|old| old.pos_global == classified.pos_global),
            NetCommand::EditInventory { request, change } => {
                if actor == super::offers::ACTOR {
                    r.offers.permits(*request, change)
                } else {
                    self.rlva_inventory_mutation_allowed(change, actor)
                }
            }
            NetCommand::CopyInventoryItems(copies) => copies
                .iter()
                .all(|copy| self.rlva_inventory_move_allowed(copy.item, copy.parent, actor)),
            NetCommand::PreviewInventoryItem(item) => match item.asset_type {
                0 => !r.has("viewtexture"),
                7 => !r.has("viewnote"),
                10 => !r.has("viewscript"),
                _ => true,
            },
            NetCommand::PreviewTaskNotecard { .. } => !r.has("viewnote"),
            NetCommand::LoadScript { .. } => !r.has("viewscript"),
            NetCommand::SendIm { to, .. } => self.rlva_allows_im("sendim", *to),
            NetCommand::SendImDialog { to, dialog: 0, .. } => self.rlva_allows_im("sendim", *to),
            NetCommand::SendImDialog { to, dialog: 17, .. } => self.rlva_can_send_session(*to),
            NetCommand::SendImDialog { to, dialog: 4, .. } => r.allows("share", &to.to_string()),
            NetCommand::ActivateGroup(group) => {
                !blocked("setgroup") || (actor == super::groups::ACTOR && r.has("setgroup") && r.groups.restore_allowed(*group))
            }
            NetCommand::GroupTitleUpdate { .. } => !blocked("setgroup"),
            NetCommand::TeleportHome | NetCommand::TeleportLandmark(_) => {
                !blocked("tplm") && !(self.agent.is_sitting() && blocked("unsit"))
            }
            NetCommand::TeleportTo { handle, position, .. } => {
                if r.stand_return == Some((*handle, *position)) {
                    return true;
                }
                let Some(main) = self.main_region else {
                    return !blocked("tploc") && !(self.agent.is_sitting() && blocked("unsit"));
                };
                let (x, y) = aurora_net::handle_to_origin(*handle);
                let (ox, oy) = aurora_net::handle_to_origin(main);
                let shift = Vec3::new((x as f64 - ox as f64) as f32, (y as f64 - oy as f64) as f32, 0.0);
                self.rlva_teleport_delta_allowed(*position + shift - self.agent.position, actor)
            }
            NetCommand::TeleportToRegion { name, position, .. } => {
                let handle = self
                    .regions
                    .values()
                    .find(|r| r.name.eq_ignore_ascii_case(name))
                    .map(|r| r.handle)
                    .or_else(|| {
                        self.map
                            .sims
                            .iter()
                            .find(|(_, r)| r.name.eq_ignore_ascii_case(name))
                            .map(|(&(x, y), _)| aurora_net::origin_to_handle(x * 256, y * 256))
                    });
                if let Some(handle) = handle
                    && let Some(main) = self.main_region
                {
                    let (x, y) = aurora_net::handle_to_origin(handle);
                    let (ox, oy) = aurora_net::handle_to_origin(main);
                    self.rlva_teleport_delta_allowed(
                        *position + Vec3::new((x as f64 - ox as f64) as f32, (y as f64 - oy as f64) as f32, 0.0) - self.agent.position,
                        actor,
                    )
                } else {
                    !blocked("tploc") && !(self.agent.is_sitting() && blocked("unsit"))
                }
            }
            NetCommand::RequestSit { target, offset, .. } => {
                let limit = r.distance_limit("sittp", 1.5, actor).min(r.distance_limit("fartouch", 1.5, actor));
                !r.has("sit")
                    && self
                        .objects
                        .index_of_uuid(target)
                        .and_then(|i| self.objects.get(i))
                        .is_some_and(|o| o.pcode == aurora_prim::params::LL_PCODE_VOLUME)
                    && !(self.agent.is_sitting() && r.has("standtp"))
                    && !(self.agent.is_sitting() && r.has("unsit"))
                    && (!actor.is_nil()
                        || !limit.is_finite()
                        || self
                            .objects
                            .index_of_uuid(target)
                            .and_then(|i| crate::scene::Scene::object_transform_unshifted(self, i, std::time::Instant::now()))
                            .is_some_and(|(pos, rot, _)| (pos + rot * *offset).distance(self.agent.position) < limit))
            }
            NetCommand::OneShotControl(flags) => {
                (flags & control::STAND_UP == 0 || !blocked("unsit"))
                    && (flags & control::SIT_ON_GROUND == 0 || (!blocked("sit") && !(self.agent.is_sitting() && blocked("unsit"))))
            }
            NetCommand::SetAlwaysRun(true) => !blocked("alwaysrun"),
            NetCommand::BuyObject { .. } => !blocked("buy"),
            NetCommand::PayObject { .. } => !blocked("buy"),
            NetCommand::PayResident { .. } => !blocked("pay"),
            NetCommand::Build(
                aurora_net::build::BuildCmd::Add(_)
                | aurora_net::build::BuildCmd::RezInventory(_)
                | aurora_net::build::BuildCmd::Duplicate { .. },
            ) => !blocked("rez"),
            NetCommand::Build(command) => self.rlva_allows_build(command),
            NetCommand::ScriptAnswer { task_id, questions, .. } => self.rlva_script_permissions(*task_id, *questions).0 == *questions,
            NetCommand::AcceptLure { lure_id, godlike } => {
                *godlike
                    || self.teleport_invites.list.iter().any(|v| {
                        matches!(v.kind, super::super::teleport_invites::InviteKind::Offer { lure, .. } if lure == *lure_id)
                            && self.rlva_can_accept_teleport(v.from, false)
                    })
            }
            NetCommand::DetachAttachments(items) => items.iter().all(|item| self.rlva_can_remove(*item, actor)),
            NetCommand::DetachObject { handle, local_id } => self
                .objects
                .index_of(&super::super::objects::ObjKey {
                    region: *handle,
                    local_id: *local_id,
                })
                .and_then(|i| self.objects.get(i))
                .is_some_and(|object| {
                    self.rlva_own_attachment_root(object.full_id) == Some(object.full_id)
                        && self.rlva_can_remove(
                            object.attachment_item_id().filter(|id| !id.is_nil()).unwrap_or(object.full_id),
                            actor,
                        )
                }),
            // Rez is sent as ADD while RLVa is enabled. The watchdog replaces
            // only removable predecessors once the actual point is known.
            NetCommand::RezAttachments(items) => {
                let worn = self.rlva_worn_attachment_points();
                items.iter().all(|item| {
                    let at = if item.point == 0 {
                        self.inventory.items.get(&item.item_id).map_or(0, |it| (it.flags & 0xff) as u8)
                    } else {
                        item.point
                    };
                    self.rlva_can_add(item.item_id, item.point, actor)
                        && (item.add
                            || r.preferences.replace_unlocked
                            || actor == super::watchdog::ACTOR
                            || !worn
                                .iter()
                                .any(|(old, point)| *old != item.item_id && at != 0 && *point == at && !self.rlva_can_remove(*old, actor)))
                        && (actor == super::watchdog::ACTOR
                            || worn
                                .get(&item.item_id)
                                .is_none_or(|old| at == 0 || at == *old || self.rlva_can_remove(item.item_id, actor)))
                })
            }
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
            NetCommand::ObjectGrab {
                handle, local_id, surface, ..
            } => self.rlva_can_touch_at(
                *handle,
                *local_id,
                (surface.face >= 0)
                    .then(|| self.region_offset(*handle).map(|shift| surface.position + shift))
                    .flatten(),
            ),
            NetCommand::ObjectGrabUpdate { object, surface, .. } => self
                .objects
                .index_of_uuid(object)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|o| {
                    self.rlva_can_touch_at(
                        o.key.region,
                        o.key.local_id,
                        (surface.face >= 0)
                            .then(|| self.region_offset(o.key.region).map(|shift| surface.position + shift))
                            .flatten(),
                    )
                }),
            _ => true,
        }
    }

    /// Queries requiring world state and force commands are consumed in their
    /// original order. The issuing object may bypass only its own locks.
    /// Outfit transactions only stall inventory forces and worn-state replies;
    /// sit, teleport, camera and status replies keep draining so activity HUDs
    /// and pose relays are not frozen behind COF sync.
    pub fn rlva_pending(&mut self) -> Vec<(Uuid, NetCommand)> {
        self.rlva_relative_tick(std::time::Instant::now());
        let mut commands = Vec::new();
        let mut appearance_blocked = self.rlva.appearance_busy || !self.rlva.appearance.is_empty();
        let mut deferred = Vec::new();
        let mut pending = std::mem::take(&mut self.rlva.pending).into_iter();
        while let Some((source, c)) = pending.next() {
            if !self.rlva.enabled() {
                break;
            }
            if !self.rlva.controllers.accepts(source, c.param) {
                continue;
            }
            if appearance_blocked && super::inventory::appearance_bound(&c) {
                deferred.push((source, c));
                continue;
            }
            if self.rlva_environment_command(source, &c) {
                continue;
            }
            if c.behaviour.starts_with("getcam_")
                || c.behaviour.starts_with("setcam_")
                || c.behaviour.starts_with("getdebug_")
                || c.behaviour.starts_with("setdebug_")
                || matches!(c.behaviour.as_str(), "adjustheight" | "getheightoffset")
            {
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
                    "getsitid" if c.option.is_empty() => Some(self.rlva_sit_id().to_string()),
                    name if super::QUERIES.contains(&name) => self.rlva_inventory_query(&c, source),
                    _ => Some(String::new()),
                };
                if let Some(text) = text {
                    self.rlva.reply(channel, text);
                } else {
                    // Folders still loading: keep this reply ahead of later work.
                    deferred.push((source, c));
                    deferred.extend(pending);
                    self.rlva.pending = deferred;
                    return commands;
                }
                continue;
            }
            if c.behaviour == "detachme"
                && c.option.is_empty()
                && let Some(item) = self.rlva_attachment_item(source)
                && let Some(key) = self.rlva_temporary_attachment(item)
            {
                if self.rlva_can_remove(item, source) {
                    commands.push((
                        source,
                        NetCommand::DetachObject {
                            handle: key.region,
                            local_id: key.local_id,
                        },
                    ));
                }
                continue;
            }
            if super::inventory::force_kind(&c.behaviour).is_some() {
                if !self.rlva_inventory_force(source, &c) {
                    deferred.push((source, c));
                    appearance_blocked = true;
                    continue;
                }
                if !self.rlva.appearance.is_empty() {
                    appearance_blocked = true;
                }
                continue;
            }
            if c.behaviour == "setgroup" {
                commands.extend(self.rlva_group_force(source, &c, std::time::Instant::now()));
                continue;
            }
            let command = match c.behaviour.as_str() {
                "unsit" if c.option.is_empty() && self.agent.is_sitting() => Some(NetCommand::OneShotControl(control::STAND_UP)),
                "sitground" if c.option.is_empty() => self.rlva_force_ground_sit(source),
                "sit" => match Uuid::parse_str(&c.option) {
                    Ok(id) if id.is_nil() => self.rlva_force_ground_sit(source),
                    Ok(id) => match self.objects.index_of_uuid(&id).and_then(|i| self.objects.get(i)) {
                        Some(o) => Some(NetCommand::RequestSit {
                            handle: o.key.region,
                            target: o.full_id,
                            offset: Vec3::ZERO,
                        }),
                        None => {
                            // Pose balls often arrive after the relay OwnerSay.
                            self.rlva.relative.wait(source, c.clone(), std::time::Instant::now());
                            None
                        }
                    },
                    Err(_) => None,
                },
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
            if let Some(mut command) = command
                && self.rlva_normalize_teleport(&mut command)
                && self.rlva_allows_command(&command, source)
            {
                if matches!(command, NetCommand::RequestSit { .. }) && self.rlva.has("standtp") {
                    self.rlva.stand_origin = self.main_region.map(|h| (h, self.agent.position));
                }
                commands.push((source, command));
            }
        }
        deferred.extend(pending);
        self.rlva.pending = deferred;
        commands
    }
}

fn teleport(option: &str) -> Option<NetCommand> {
    // RlvForceHandler<TPTO> retains double precision until the region origin
    // is removed and rotates the look vector by the optional radian angle.
    let mut options = option.split(';');
    let destination = options.next()?;
    let look_at = match options.next() {
        Some(angle) => {
            let angle = angle.parse::<f32>().ok()?;
            if !angle.is_finite() {
                return None;
            }
            let (sin, cos) = angle.sin_cos();
            Vec3::new(cos, sin, 0.0)
        }
        None => Vec3::ZERO,
    };
    let parts: Vec<_> = destination.split('/').collect();
    let (name, coordinates) = match parts.as_slice() {
        [x, y, z] => (None, [*x, *y, *z]),
        [region, x, y, z] if !region.is_empty() => (Some(*region), [*x, *y, *z]),
        _ => return None,
    };
    let p = glam::DVec3::new(
        coordinates[0].parse().ok()?,
        coordinates[1].parse().ok()?,
        coordinates[2].parse().ok()?,
    );
    if !p.is_finite() {
        return None;
    }
    if let Some(name) = name {
        Some(NetCommand::TeleportToRegion {
            request: Uuid::new_v4(),
            name: name.to_owned(),
            position: p.as_vec3().is_finite().then_some(p.as_vec3())?,
            look_at,
        })
    } else {
        if p.x < 0.0 || p.y < 0.0 || p.x >= u32::MAX as f64 || p.y >= u32::MAX as f64 || p.z.abs() > f32::MAX as f64 {
            return None;
        }
        let x = (p.x as u32 / 256) * 256;
        let y = (p.y as u32 / 256) * 256;
        Some(NetCommand::TeleportTo {
            handle: aurora_net::origin_to_handle(x, y),
            position: Vec3::new((p.x - x as f64) as f32, (p.y - y as f64) as f32, p.z as f32),
            look_at,
        })
    }
}

#[cfg(test)]
mod teleport_tests {
    use super::*;

    #[test]
    fn rlva_tpto_preserves_global_fractions_and_named_region_heading() {
        let Some(NetCommand::TeleportTo { handle, position, look_at }) = teleport("10000000.125/20000000.75/42.5;1.57079632679") else {
            panic!("global teleport")
        };
        let (x, y) = aurora_net::handle_to_origin(handle);
        assert_eq!(x as f64 + position.x as f64, 10000000.125);
        assert_eq!(y as f64 + position.y as f64, 20000000.75);
        assert_eq!(position.z, 42.5);
        assert!(look_at.abs_diff_eq(Vec3::Y, 1e-6));
        assert!(
            matches!(teleport("region/1.25/2.5/40;-1.57079632679"), Some(NetCommand::TeleportToRegion { position, look_at, .. }) if position == Vec3::new(1.25, 2.5, 40.0) && look_at.abs_diff_eq(Vec3::NEG_Y, 1e-6))
        );
        assert!(matches!(teleport("1/2/3"), Some(NetCommand::TeleportTo { look_at, .. }) if look_at == Vec3::ZERO));
        assert!(
            matches!(teleport("region/-1.25/300.5/40"), Some(NetCommand::TeleportToRegion { position, .. }) if position == Vec3::new(-1.25, 300.5, 40.0))
        );
        for invalid in [
            "1/2/3;bad",
            "1/2/3;",
            "1/2/3;nan",
            "region/1/2/3;inf",
            "nan/2/3",
            "1/inf/3",
            "4294967296/2/3",
            "region/1/2/1e100",
            "region/1/2",
        ] {
            assert!(teleport(invalid).is_none(), "{invalid}");
        }
    }
}
