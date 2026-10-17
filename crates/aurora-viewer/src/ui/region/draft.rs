//! The settings of a "Manage" section while they are changed: Firestorm's
//! panels send a whole tab with "Apply" (`setregioninfo`,
//! `estatechangeinfo`, the terrain's four messages), so a section keeps its
//! changes until its bar applies or cancels them. Once applied, the sent
//! values stay shown until the simulator's next answer moves away from what
//! was there before, as Firestorm's controls keep them until
//! processRegionInfo or a new RegionHandshake.

use aurora_net::estate::{EstateInfo, RegionDebug, RegionRules, TerrainSettings, estate_flags as ef};

/// How many settings differ between two states of a section (the count of
/// "N changes not sent yet").
pub(crate) trait Changes {
    fn changes(&self, other: &Self) -> usize;
}

/// The estate's access flags of the Estate panel.
pub(crate) const ACCESS_FLAGS: u64 = ef::EXTERNALLY_VISIBLE
    | ef::DENY_AGEUNVERIFIED
    | ef::DENY_ANONYMOUS
    | ef::DENY_BOTS
    | ef::ALLOW_ACCESS_OVERRIDE
    | ef::ALLOW_DIRECT_TELEPORT
    | ef::ALLOW_VOICE;

/// "Who can come": the estate's access flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Access(pub u64);

impl Access {
    pub fn of(e: &EstateInfo) -> Self {
        Access(e.flags & ACCESS_FLAGS)
    }

    pub fn has(self, flag: u64) -> bool {
        self.0 & flag != 0
    }

    /// One switch; restricting the estate to its lists clears the three
    /// limits of public access (LLPanelEstateInfo::refresh).
    pub fn set(&mut self, flag: u64, on: bool) {
        if on {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
        if !self.has(ef::EXTERNALLY_VISIBLE) {
            self.0 &= !(ef::DENY_AGEUNVERIFIED | ef::DENY_ANONYMOUS | ef::DENY_BOTS);
        }
    }

    /// The estate as callbackChangeLindenEstate sends it: these flags in its
    /// own, and no fixed sun any more ("we don't support fixed sun estates").
    pub fn applied_to(self, e: &EstateInfo) -> EstateInfo {
        EstateInfo {
            flags: (e.flags & !ACCESS_FLAGS & !ef::SUN_FIXED) | self.0,
            ..e.clone()
        }
    }
}

impl Changes for Access {
    fn changes(&self, other: &Self) -> usize {
        (self.0 ^ other.0).count_ones() as usize
    }
}

impl Changes for RegionRules {
    fn changes(&self, o: &Self) -> usize {
        [
            self.block_terraform != o.block_terraform,
            self.block_fly != o.block_fly,
            self.block_fly_over != o.block_fly_over,
            self.allow_damage != o.allow_damage,
            self.restrict_pushobject != o.restrict_pushobject,
            self.allow_land_resell != o.allow_land_resell,
            self.allow_parcel_changes != o.allow_parcel_changes,
            self.block_parcel_search != o.block_parcel_search,
            self.agent_limit != o.agent_limit,
            self.object_bonus != o.object_bonus,
            self.sim_access != o.sim_access,
        ]
        .into_iter()
        .filter(|c| *c)
        .count()
    }
}

impl Changes for RegionDebug {
    fn changes(&self, o: &Self) -> usize {
        [
            self.skip_scripts != o.skip_scripts,
            self.skip_collisions != o.skip_collisions,
            self.skip_physics != o.skip_physics,
        ]
        .into_iter()
        .filter(|c| *c)
        .count()
    }
}

impl Changes for TerrainSettings {
    fn changes(&self, o: &Self) -> usize {
        let scalars = [
            self.water_height != o.water_height,
            self.raise_limit != o.raise_limit,
            self.lower_limit != o.lower_limit,
        ];
        let corners = (0..4).flat_map(|i| [self.start_height[i] != o.start_height[i], self.height_range[i] != o.height_range[i]]);
        let textures = (0..4).map(|i| self.textures[i] != o.textures[i]);
        scalars.into_iter().chain(corners).chain(textures).filter(|c| *c).count()
    }
}

/// A section's changes, and what its last "Apply" sent.
#[derive(Debug, Clone)]
pub(crate) struct Draft<T> {
    edit: Option<T>,
    /// (the simulator's values when it was sent, the values sent)
    sent: Option<(T, T)>,
}

impl<T> Default for Draft<T> {
    fn default() -> Self {
        Draft { edit: None, sent: None }
    }
}

impl<T: Clone + PartialEq + Changes> Draft<T> {
    /// What the section shows: the changes, else what was sent while the
    /// simulator has not answered otherwise, else the simulator's values.
    pub fn shown(&mut self, server: &T) -> T {
        if let Some(e) = &self.edit {
            return e.clone();
        }
        if let Some((before, sent)) = &self.sent {
            if before == server {
                return sent.clone();
            }
            self.sent = None;
        }
        server.clone()
    }

    /// A control changed: `now` is the whole section after it.
    pub fn set(&mut self, server: &T, now: T) {
        let base = self.base(server);
        self.edit = (now.changes(&base) > 0).then_some(now);
    }

    /// The count of the bar (0: no bar).
    pub fn pending(&mut self, server: &T) -> usize {
        let base = self.base(server);
        self.edit.as_ref().map_or(0, |e| e.changes(&base))
    }

    /// What the changes are measured against: the simulator's values, or
    /// what was sent and not answered yet.
    fn base(&mut self, server: &T) -> T {
        let edit = self.edit.take();
        let base = self.shown(server);
        self.edit = edit;
        base
    }

    /// "Cancel": back to what the simulator (or the last "Apply") says.
    pub fn cancel(&mut self) {
        self.edit = None;
    }

    /// "Apply": the values to send, kept shown until the simulator answers.
    pub fn apply(&mut self, server: &T) -> Option<T> {
        let now = self.edit.take()?;
        self.sent = Some((server.clone(), now.clone()));
        Some(now)
    }

    /// The changes being made, if any.
    pub fn editing(&self) -> Option<&T> {
        self.edit.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::estate::RegionDetails;
    use uuid::Uuid;

    fn rules() -> RegionRules {
        RegionRules::of(&RegionDetails {
            max_agents: 40,
            object_bonus: 1.0,
            sim_access: 13,
            ..Default::default()
        })
    }

    #[test]
    fn a_draft_counts_its_changes_and_drops_them_when_undone() {
        let server = rules();
        let mut d = Draft::default();
        assert_eq!(d.pending(&server), 0);
        let mut now = d.shown(&server);
        now.block_fly = true;
        now.agent_limit = 50.0;
        d.set(&server, now);
        assert_eq!(d.pending(&server), 2);
        assert!(d.shown(&server).block_fly);
        // switched back: nothing to send any more
        now.block_fly = false;
        now.agent_limit = 40.0;
        d.set(&server, now);
        assert_eq!(d.pending(&server), 0);
        assert!(d.editing().is_none());
    }

    #[test]
    fn applied_values_stay_until_the_simulator_answers() {
        let server = rules();
        let mut d = Draft::default();
        let mut now = server;
        now.object_bonus = 1.5;
        d.set(&server, now);
        let sent = d.apply(&server).expect("a change");
        assert_eq!(sent.object_bonus, 1.5);
        assert_eq!(d.pending(&server), 0, "no bar once sent");
        // the simulator has not answered: the sent value is shown
        assert_eq!(d.shown(&server).object_bonus, 1.5);
        // it answered: its values from now on
        let mut answered = server;
        answered.object_bonus = 1.5;
        assert_eq!(d.shown(&answered).object_bonus, 1.5);
        assert_eq!(d.shown(&server).object_bonus, 1.0, "the overlay is gone");
    }

    #[test]
    fn cancel_goes_back_to_the_simulator() {
        let server = rules();
        let mut d = Draft::default();
        let mut now = server;
        now.sim_access = 21;
        d.set(&server, now);
        d.cancel();
        assert_eq!(d.shown(&server).sim_access, 13);
        assert!(d.apply(&server).is_none());
    }

    /// LLPanelEstateInfo::refresh: an estate limited to its lists has no
    /// public access limits.
    #[test]
    fn restricting_the_estate_clears_the_public_limits() {
        let mut a = Access(ef::EXTERNALLY_VISIBLE | ef::DENY_BOTS | ef::DENY_ANONYMOUS | ef::ALLOW_VOICE);
        a.set(ef::EXTERNALLY_VISIBLE, false);
        assert_eq!(a, Access(ef::ALLOW_VOICE));
        let e = EstateInfo {
            flags: ef::SUN_FIXED | ef::ALLOW_ENVIRONMENT_OVERRIDE | ef::EXTERNALLY_VISIBLE,
            ..Default::default()
        };
        let sent = a.applied_to(&e);
        assert_eq!(
            sent.flags,
            ef::ALLOW_ENVIRONMENT_OVERRIDE | ef::ALLOW_VOICE,
            "other flags kept, fixed sun off"
        );
    }

    #[test]
    fn terrain_changes_count_each_setting() {
        let t = TerrainSettings {
            water_height: 20.0,
            raise_limit: 4.0,
            lower_limit: -4.0,
            textures: [Uuid::nil(); 4],
            start_height: [10.0; 4],
            height_range: [60.0; 4],
        };
        let mut o = t;
        o.textures[2] = Uuid::from_u128(5);
        o.start_height[1] = 12.0;
        o.water_height = 21.0;
        assert_eq!(t.changes(&o), 3);
    }
}
