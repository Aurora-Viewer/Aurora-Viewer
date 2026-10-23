//! Track regional bridge announcements and validate live simulator parentage.
//! Offline fixtures can still negotiate direct simulator snapshots.
use super::*;

const NETWORK_SPACING: Duration = Duration::from_millis(250);
const SCAN_INTERVAL: Duration = Duration::from_secs(1);
const MAX_SOURCES: usize = 256;

impl Contacts {
    pub fn for_avatar(&self, local: Uuid, subject: Uuid) -> Option<&Self> {
        if subject == local {
            Some(self)
        } else {
            self.observers.get(&subject)
        }
    }

    pub(super) fn receive_observer(&mut self, subject: Uuid, source: Seat, chat: &ChatMessage, now: Instant, allowance: usize) -> bool {
        self.observers
            .get_mut(&subject)
            .is_some_and(|c| c.receive(source, chat, now, allowance))
    }

    fn reset_world(&mut self) {
        let relay = self.relay.take();
        let required = self.relay_required;
        *self = Self::default();
        self.relay = relay;
        self.relay_required = required;
    }

    pub(super) fn update_world(&mut self, world: &World, now: Instant) -> Option<NetCommand> {
        let region = world.main_region.or_else(|| {
            world
                .objects
                .index_of_uuid(&world.agent_id)
                .and_then(|i| world.objects.get(i))
                .map(|o| o.key.region)
        });
        if world.teleporting || self.observed_region != region {
            self.reset_world();
            self.observed_region = region;
        }
        if world.teleporting {
            return None;
        }
        let editor_source = self.editor.seat.filter(|s| object_source(world, s.root) == Some(*s));
        self.editor.track(editor_source, now);
        if self
            .editor
            .reload
            .as_ref()
            .is_some_and(|r| now.saturating_duration_since(r.at) >= reload::TIMEOUT)
        {
            self.editor.graph = graph_wire::State::default();
        }
        self.subject = Some(world.agent_id);
        let push = self.relay.as_ref().is_some_and(|relay| {
            Some(relay.region) == region
                && world
                    .objects
                    .index_of_uuid(&relay.root)
                    .and_then(|i| world.objects.get(i))
                    .is_some_and(|root| {
                        sources::eligible_for(
                            world,
                            world.agent_id,
                            Seat {
                                root: root.full_id,
                                region: root.key.region,
                            },
                        )
                    })
        });
        if push || self.relay_required {
            let seat = self
                .primary
                .seat
                .filter(|source| sources::eligible_for(world, world.agent_id, *source));
            let worn: Vec<_> = self
                .worn
                .values()
                .filter_map(|session| session.seat)
                .filter(|source| sources::eligible_for(world, world.agent_id, *source))
                .collect();
            self.sync_sources(seat, &worn, now);
        } else {
            self.sync_sources(own_seat(world), &attachment_sources(world), now);
        }
        for session in std::iter::once(&mut self.primary).chain(self.worn.values_mut()) {
            session.push = push || self.relay_required;
        }
        if !push && !self.relay_required && self.scan_at.is_none_or(|at| now.saturating_duration_since(at) >= SCAN_INTERVAL) {
            self.scan_at = Some(now);
            let mut subjects: Vec<_> = world
                .objects
                .avatars()
                .filter(|(_, a)| a.full_id != world.agent_id && Some(a.key.region) == region)
                .map(|(_, a)| a.full_id)
                .collect();
            subjects.sort();
            let budget = MAX_SOURCES.saturating_sub(self.sources().len());
            subjects.truncate(budget.min(128));
            let mut candidates: Vec<_> = subjects.into_iter().map(|id| (id, avatar_seat(world, id), Vec::new())).collect();
            let mut remaining = budget.saturating_sub(candidates.iter().filter(|(_, s, _)| s.is_some()).count());
            for (id, _, worn) in &mut candidates {
                *worn = sources::attachments_for(world, *id).into_iter().take(remaining).collect();
                remaining -= worn.len();
            }
            self.observers
                .retain(|id, _| candidates.iter().any(|(a, s, w)| a == id && (s.is_some() || !w.is_empty())));
            for (id, seat, worn) in candidates {
                if seat.is_none() && worn.is_empty() {
                    continue;
                }
                let contacts = self.observers.entry(id).or_default();
                contacts.subject = Some(id);
                contacts.sync_sources(seat, &worn, now);
            }
        }
        // Parentage is checked every frame, even between discovery scans.
        // A quick stand/sit cycle must retire its old nonce as well.
        for (id, contacts) in &mut self.observers {
            if !contacts.needs_sync(world, *id, now) {
                continue;
            }
            let seat = contacts.primary.seat.filter(|s| sources::eligible_for(world, *id, *s));
            let worn: Vec<_> = contacts
                .sources()
                .into_iter()
                .filter(|s| Some(*s) != contacts.primary.seat && sources::eligible_for(world, *id, *s))
                .collect();
            contacts.sync_sources(seat, &worn, now);
        }
        if self.relay_required {
            self.observers.retain(|_, contacts| !contacts.sources().is_empty());
        }
        if self.relay_required && !push {
            // No recurring probes replace an absent relay. Lifecycle cleanup
            // above still runs, and corrections cannot outlive the worn relay.
            self.primary.graph = graph_wire::State::default();
            for contacts in self.observers.values_mut() {
                contacts.primary.graph = graph_wire::State::default();
                for session in contacts.worn.values_mut() {
                    session.graph = graph_wire::State::default();
                }
                contacts.merge();
            }
            for session in self.worn.values_mut() {
                session.graph = graph_wire::State::default();
            }
            self.merge();
            return None;
        }
        if self.network_at.is_some_and(|at| now < at) {
            return None;
        }
        if push
            && std::iter::once(&*self)
                .chain(self.observers.values())
                .flat_map(|contacts| contacts.sessions())
                .chain(std::iter::once(&self.editor))
                .filter(|session| session.graph.transferring(now))
                .count()
                >= 4
        {
            // Bound simultaneous fragment streams as well as request rate.
            // Large rigs cannot turn a single regional arrival into a flood.
            return None;
        }
        // One explicit selected-object request has priority without bypassing
        // the shared four-per-second budget or interrupting a live transfer.
        if self.editor.reloading(now)
            && self.editor.due(now)
            && let Some(command) = self.editor.update(self.editor.seat, world.agent_id, now)
        {
            self.network_at = Some(now + NETWORK_SPACING);
            return Some(command);
        }
        // Renew known leases before probing unrecognised creations. Every
        // avatar shares the same global traffic budget, including ourselves.
        let refresh = std::iter::once((world.agent_id, &*self))
            .chain(self.observers.iter().map(|(id, c)| (*id, c)))
            .filter_map(|(id, c)| c.refresh_due(now).map(|at| (id, at)))
            .min_by_key(|(_, at)| *at)
            .map(|(id, _)| id);
        let mut command = if let Some(id) = refresh {
            if id == world.agent_id {
                self.probe(self.primary.seat, world.agent_id, now)
            } else {
                self.observers
                    .get_mut(&id)
                    .and_then(|c| c.probe(c.primary.seat, world.agent_id, now))
            }
        } else {
            None
        };
        if command.is_none() {
            let mut ids: Vec<_> = self.observers.keys().copied().collect();
            ids.sort();
            ids.insert(0, world.agent_id);
            for delta in 0..ids.len() {
                let index = (self.observer_cursor + delta) % ids.len();
                let c = if ids[index] == world.agent_id {
                    &mut *self
                } else {
                    let Some(c) = self.observers.get_mut(&ids[index]) else { continue };
                    c
                };
                command = c.probe(c.primary.seat, world.agent_id, now);
                if command.is_some() {
                    self.observer_cursor = (index + 1) % ids.len();
                    break;
                }
            }
        }
        if command.is_some() {
            self.network_at = Some(now + NETWORK_SPACING);
        }
        command
    }
}

#[cfg(test)]
mod tests;
