//! Named teleports return through the common action boundary after map lookup.
//! RlvHandler::onTeleportCallback retains the issuing object around the delayed
//! teleport. Tickets also discard replies superseded by another destination.
use aurora_net::{NetCommand, RegionHandle};
use glam::Vec3;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct NamedTeleport {
    pending: Option<Pending>,
}

struct Pending {
    request: Uuid,
    actor: Uuid,
    position: Vec3,
    look_at: Vec3,
}

impl NamedTeleport {
    pub fn observe(&mut self, command: &NetCommand, actor: Uuid) {
        match command {
            NetCommand::TeleportToRegion {
                request,
                position,
                look_at,
                ..
            } => {
                self.pending = Some(Pending {
                    request: *request,
                    actor,
                    position: *position,
                    look_at: *look_at,
                });
            }
            NetCommand::TeleportTo { .. } | NetCommand::TeleportHome | NetCommand::TeleportLandmark(_) | NetCommand::AcceptLure { .. } => {
                self.pending = None;
            }
            _ => {}
        }
    }

    pub fn cancel_scripts(&mut self) {
        if self.pending.as_ref().is_some_and(|p| !p.actor.is_nil()) {
            self.pending = None;
        }
    }

    pub fn resolve(&mut self, request: Uuid, result: Result<RegionHandle, String>) -> Option<(Uuid, Result<NetCommand, String>)> {
        if self.pending.as_ref()?.request != request {
            return None;
        }
        let pending = self.pending.take()?;
        Some((
            pending.actor,
            result.map(|handle| NetCommand::TeleportTo {
                handle,
                position: pending.position,
                look_at: pending.look_at,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(request: Uuid) -> NetCommand {
        NetCommand::TeleportToRegion {
            request,
            name: "region".into(),
            position: Vec3::new(7.25, 3.5, 20.0),
            look_at: Vec3::Y,
        }
    }

    #[test]
    fn rlva_named_teleports_retain_actor_heading_and_only_accept_the_current_reply_once() {
        let mut pending = NamedTeleport::default();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let actor = Uuid::from_u128(3);
        pending.observe(&lookup(a), actor);
        pending.observe(&lookup(b), actor);
        assert!(pending.resolve(a, Err("stale".into())).is_none());
        let (source, command) = pending.resolve(b, Ok(256)).unwrap();
        assert_eq!(source, actor);
        assert!(
            matches!(command, Ok(NetCommand::TeleportTo { handle: 256, position, look_at }) if position == Vec3::new(7.25, 3.5, 20.0) && look_at == Vec3::Y)
        );
        assert!(pending.resolve(b, Ok(256)).is_none());
        pending.observe(&lookup(a), actor);
        pending.cancel_scripts();
        assert!(pending.resolve(a, Ok(256)).is_none());
        pending.observe(&lookup(a), Uuid::nil());
        pending.cancel_scripts();
        assert!(
            pending
                .resolve(a, Err("not found".into()))
                .is_some_and(|(source, result)| source.is_nil() && result.is_err())
        );
        pending.observe(&lookup(a), actor);
        pending.observe(&NetCommand::TeleportHome, Uuid::nil());
        assert!(pending.resolve(a, Ok(256)).is_none());
    }
}
