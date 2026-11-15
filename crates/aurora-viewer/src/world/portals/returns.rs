//! Optional return invitations: reuse an immutable nearby route, never stack
//! apertures or grant a relay's friends access on the creator's behalf.
use super::*;

/// Include the cloudy rim, irrespective of either doorway's heading.
pub(super) fn overlaps(a: &Announcement, b: &Announcement) -> bool {
    let delta = Vec3::from(a.center) - Vec3::from(b.center);
    a.region == b.region && delta.truncate().length_squared() < 3.2_f32.powi(2) && delta.z.abs() < 4.2
}

impl State {
    pub(super) fn materialized(&self, entry: &Entry) -> bool {
        // A manually placed source has priority if it appears after a return.
        // Keep the return invitation intact, but never draw or cross both.
        entry.portal.creator.is_none()
            || !self
                .entries
                .iter()
                .any(|other| other.portal.creator.is_none() && overlaps(&other.portal, &entry.portal))
    }

    pub fn apertures(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| self.materialized(entry))
    }

    pub(super) fn shown_entry(&self, world: &World, entry: &Entry) -> bool {
        entry.shown(world)
            && (entry.portal.creator.is_none()
                || !self
                    .entries
                    .iter()
                    .any(|other| other.portal.creator.is_none() && other.shown(world) && overlaps(&other.portal, &entry.portal)))
    }

    /// The accepted journey grants only this traveler a return pass. Other
    /// residents still need the original creator's Friends/Group permission.
    pub fn share_return(&mut self, portal: Portal, creator: Uuid, build: BuildRight, position: Vec3, now: Instant, unix: u64) -> bool {
        let share = build == BuildRight::Allowed;
        let unknown = build == BuildRight::Unknown;
        let mut announcement = portal.announcement();
        announcement.creator = Some(creator);
        if creator.is_nil()
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
                && entry.details.as_ref().is_some_and(|details| {
                    details.access == portal.access
                        && details.destination.region.eq_ignore_ascii_case(&portal.destination)
                        && Vec3::from(details.destination.position).distance(Vec3::from(portal.position)) <= 0.5
                })
        };
        // Provenance is a place, not the departure UUID: recreating a source
        // must reuse the first aperture, its destination and its original TTL.
        if let Some(index) = self.entries.iter().position(|entry| same_route(entry) && self.materialized(entry)) {
            let entry = &self.entries[index];
            announcement = entry.portal.clone();
            announcement.creator = Some(creator);
            if share && !entry.shared && !self.queue(Uuid::nil(), Message::Open(announcement.clone())) {
                return false;
            }
            let entry = &mut self.entries[index];
            // Retain a normal source's local priority. Only its relayed wire
            // needs the creator hint; converting its stored kind would expose
            // an older overlapping return that the source currently suppresses.
            entry.shared |= share;
            entry.awaiting_land = unknown && !entry.shared;
            entry.return_pass = true;
            entry.gate = Gate::arrival(Vec3::from(entry.portal.center), position, now);
        } else {
            if self.entries.len() >= LIMIT
                || self
                    .entries
                    .iter()
                    .any(|entry| entry.portal.id == portal.id || overlaps(&entry.portal, &announcement))
                || (share && !self.queue(Uuid::nil(), Message::Open(announcement.clone())))
            {
                return false;
            }
            self.entries.push(Entry {
                owner: creator,
                gate: Gate::arrival(Vec3::from(portal.center), position, now),
                expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
                portal: announcement,
                details: Some(portal.details()),
                relays: Vec::new(),
                shared: share,
                created: false,
                return_pass: true,
                awaiting_land: unknown,
            });
        }
        if share
            && let Access::Group(group) = portal.access
            && !self.group_refresh.contains(&group)
        {
            self.group_refresh.push_back(group);
        }
        true
    }
}

#[cfg(test)]
mod tests;
