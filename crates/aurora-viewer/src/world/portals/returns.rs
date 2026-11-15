//! Optional return invitations: reuse an immutable nearby route, never stack
//! apertures or grant a relay's friends access on the creator's behalf.
use super::*;

/// Include the cloudy rim, irrespective of either doorway's heading.
pub(super) fn overlaps(a: &Announcement, b: &Announcement) -> bool {
    let delta = Vec3::from(a.center) - Vec3::from(b.center);
    a.region == b.region && delta.truncate().length_squared() < 3.2_f32.powi(2) && delta.z.abs() < 4.2
}

impl State {
    fn obscured(&self, world: &World, entry: &Entry) -> bool {
        // Shown sources first, then our return. Two relayed returns can
        // become visible together after a profile or preference changes;
        // their immutable identities break that tie on every frame.
        let priority = |e: &Entry| (e.portal.creator.is_some(), !e.return_pass, e.portal.id, e.owner);
        entry.portal.creator.is_some()
            && self
                .entries
                .iter()
                .any(|other| priority(other) < priority(entry) && other.shown(world) && overlaps(&other.portal, &entry.portal))
    }

    pub(super) fn shown_entry(&self, world: &World, entry: &Entry) -> bool {
        entry.shown(world) && !self.obscured(world, entry)
    }

    /// A local trip keeps its departure pass. Start sampling again from
    /// the landing position, even when it is right next to the departure.
    pub fn keep_departure(&mut self, world: &World, portal: Portal, creator: Uuid, now: Instant, unix: u64) {
        self.keep_endpoint(world, portal, creator, now, unix, false);
    }

    /// The accepted journey grants only this traveler a return pass. Other
    /// residents still need the original creator's Friends/Group permission.
    /// `awaiting_land` then checks the parcel at the retained aperture's
    /// exact center before announcing an invitation, even on a reused route.
    pub fn keep_return(&mut self, world: &World, portal: Portal, creator: Uuid, now: Instant, unix: u64) -> bool {
        self.keep_endpoint(world, portal, creator, now, unix, true)
    }

    fn keep_endpoint(&mut self, world: &World, portal: Portal, creator: Uuid, now: Instant, unix: u64, emerge: bool) -> bool {
        let position = world.agent.position;
        let mut announcement = portal.announcement();
        announcement.creator = Some(creator);
        if creator.is_nil()
            || world.mutes.is_muted_id(&creator)
            || !position.is_finite()
            || Some(portal.region) != self.region
            || !portal.live(unix)
            || Message::Open(announcement.clone()).encode().is_none()
            || (portal.access != Access::Everyone
                && Message::Details {
                    region: portal.region,
                    id: portal.id,
                    nonce: portal.id,
                    details: portal.details(),
                }
                .encode()
                .is_none())
        {
            return false;
        }
        self.entries.retain(|entry| entry.portal.live(unix) && now < entry.expires_at);
        let same_route = |entry: &Entry| {
            entry.owner == creator
                && overlaps(&entry.portal, &announcement)
                && entry.details.as_ref().map_or_else(
                    || entry.shared_announcement() == announcement,
                    |details| {
                        details.access == portal.access
                            && details.destination.region.eq_ignore_ascii_case(&portal.destination)
                            && Vec3::from(details.destination.position).distance(Vec3::from(portal.position)) <= 0.5
                    },
                )
        };
        // Provenance is a place, not the departure UUID: recreating a source
        // must reuse the first aperture, its destination and its original TTL.
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| same_route(entry) && !self.obscured(world, entry))
        {
            let entry = &mut self.entries[index];
            entry.details.get_or_insert_with(|| portal.details());
            entry.awaiting_land = !entry.shared;
            entry.return_pass = true;
            entry.gate = Gate::after_teleport(Vec3::from(entry.portal.center), position, now, emerge);
            if entry.shared {
                self.refresh_access_group(portal.access);
            }
        } else {
            if self.entries.len() >= LIMIT
                || self
                    .entries
                    .iter()
                    .any(|entry| entry.portal.id == portal.id || (entry.shown(world) && overlaps(&entry.portal, &announcement)))
            {
                return false;
            }
            self.entries.push(Entry {
                owner: creator,
                gate: Gate::after_teleport(Vec3::from(portal.center), position, now, emerge),
                expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
                portal: announcement,
                details: Some(portal.details()),
                relays: Vec::new(),
                shared: false,
                created: false,
                return_pass: true,
                awaiting_land: true,
            });
        }
        true
    }
}

#[cfg(test)]
mod tests;
