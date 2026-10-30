//! RlvCommandOptionGetPathCallback (rlvhelper.cpp, LGPL 2.1): relative wear
//! from an object not received yet waits up to thirty seconds for its attach.
//! These callbacks do not hold up commands from other controllers.
use super::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct Waiting {
    source: Uuid,
    command: Command,
    since: Instant,
}

#[derive(Default)]
pub struct RelativeWear {
    waiting: Vec<Waiting>,
}

impl RelativeWear {
    pub fn wait(&mut self, source: Uuid, command: Command, now: Instant) {
        if self.waiting.len() < 256 {
            self.waiting.push(Waiting {
                source,
                command,
                since: now,
            });
        }
    }
    pub fn forget(&mut self, source: Uuid) {
        self.waiting.retain(|waiting| waiting.source != source);
    }

    pub fn update(&mut self, now: Instant, capacity: usize, ready_for: impl Fn(Uuid, &Command) -> bool) -> Vec<(Uuid, Command)> {
        let mut ready = Vec::new();
        self.waiting.retain(|waiting| {
            if now.saturating_duration_since(waiting.since) > Duration::from_secs(30) {
                return false;
            }
            if ready.len() < capacity && ready_for(waiting.source, &waiting.command) {
                ready.push((waiting.source, waiting.command.clone()));
                false
            } else {
                true
            }
        });
        ready
    }
}

impl crate::world::World {
    pub(crate) fn rlva_relative_tick(&mut self, now: Instant) {
        if !self.rlva.enabled() {
            return;
        }
        let mut waiting = std::mem::take(&mut self.rlva.relative);
        // Child prim issuers resolve to their worn linkset root; waiting for
        // the exact source UUID alone stranded relative wears from HUD children.
        // Pose `@sit:<uuid>=force` waits the same window for the seat object.
        let ready = waiting.update(now, 256usize.saturating_sub(self.rlva.pending.len()), |source, command| {
            if command.behaviour == "sit" {
                Uuid::parse_str(&command.option)
                    .ok()
                    .filter(|id| !id.is_nil())
                    .is_some_and(|id| self.objects.index_of_uuid(&id).is_some())
            } else {
                self.rlva_own_attachment_root(source).is_some()
            }
        });
        self.rlva.relative = waiting;
        self.rlva.pending.extend(
            ready
                .into_iter()
                .filter(|(source, command)| self.rlva.controllers.accepts(*source, command.param)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rlva_relative_wear_waits_for_the_exact_attach_expires_and_cancels_each_issuer() {
        let now = Instant::now();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let command = Command::parse("attachthis=force").unwrap();
        let mut waiting = RelativeWear::default();
        waiting.wait(a, command.clone(), now);
        waiting.wait(b, command.clone(), now);
        assert!(waiting.update(now, 256, |_, _| false).is_empty());
        assert_eq!(
            waiting.update(now + Duration::from_secs(5), 256, |source, _| source == b),
            [(b, command.clone())]
        );
        assert!(waiting.update(now + Duration::from_secs(31), 256, |_, _| true).is_empty());
        assert!(waiting.waiting.is_empty());
        waiting.wait(a, command.clone(), now);
        waiting.forget(a);
        assert!(waiting.update(now, 256, |_, _| true).is_empty());
        waiting.wait(b, command.clone(), now);
        assert!(waiting.update(now, 0, |_, _| true).is_empty());
        assert_eq!(waiting.update(now, 1, |_, _| true), [(b, command)]);
    }

    #[test]
    fn rlva_relative_wear_never_blocks_an_unrelated_query_and_global_disable_discards_callbacks() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.rlva_configure(true);
        let source = Uuid::from_u128(1);
        world.rlva.pending.push((source, Command::parse("attachthis=force").unwrap()));
        world
            .rlva
            .pending
            .push((Uuid::from_u128(2), Command::parse("getsitid=123").unwrap()));
        assert!(world.rlva_pending().is_empty());
        assert!(
            world
                .rlva
                .out
                .iter()
                .any(|command| matches!(command, aurora_net::NetCommand::ChatReply { channel: 123, .. }))
        );
        assert_eq!(world.rlva.relative.waiting.len(), 1);
        world.rlva.forget(source);
        assert!(world.rlva.relative.waiting.is_empty());
        world
            .rlva
            .relative
            .wait(source, Command::parse("attachthis=force").unwrap(), Instant::now());
        world.rlva_configure(false);
        assert!(world.rlva.relative.waiting.is_empty());
    }
}
