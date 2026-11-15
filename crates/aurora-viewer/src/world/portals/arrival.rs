//! One-way presentation only: no invitation, permission grant or crossing gate.
use super::*;
use aurora_net::bridge::portal::Arrival;

/// The horizontal mesh has its width on X and its height on Y.
pub fn fits_region(center: Vec3, size: [u32; 2]) -> bool {
    center.is_finite() && (WIDTH..=size[0] as f32 - WIDTH).contains(&center.x) && (HEIGHT..=size[1] as f32 - HEIGHT).contains(&center.y)
}

pub struct Effect {
    pub event: Arrival,
    at: Instant,
    age: f32,
    awaiting_land: bool,
}

impl Effect {
    pub fn new(event: Arrival, at: Instant, unix_ms: u64) -> Self {
        let age = if unix_ms >= event.starts {
            (unix_ms - event.starts) as f32 * 0.001
        } else {
            -((event.starts - unix_ms) as f32) * 0.001
        };
        Self {
            event,
            at,
            age,
            awaiting_land: false,
        }
    }
    pub fn age(&self, now: Instant) -> f32 {
        self.age + now.saturating_duration_since(self.at).as_secs_f32()
    }
    pub fn finished(&self, now: Instant) -> bool {
        self.age(now) >= self.event.duration()
    }
    pub fn center(&self) -> Vec3 {
        Vec3::from(self.event.position) + Vec3::Z * (self.event.drop_height() - self.event.height * 0.5)
    }
    pub fn matches(&self, root: Vec3) -> bool {
        let delta = root - Vec3::from(self.event.position);
        root.is_finite() && delta.truncate().length_squared() <= 1.5 * 1.5 && delta.z.abs() <= 2.0
    }
    pub fn offset(&self, now: Instant) -> f32 {
        let t = (self.age(now) - 0.25).max(0.0);
        (self.event.drop_height() - 0.5 * 9.81 * t * t).max(0.0)
    }
    pub fn scale(&self, now: Instant) -> f32 {
        let smooth = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        smooth(self.age(now) / 0.25) * (1.0 - smooth((self.age(now) - self.event.fall_end() - 0.1) / 0.35))
    }
    pub fn clip(&self, now: Instant) -> Option<aurora_render::PortalClip> {
        if self.finished(now) {
            None
        } else if self.age(now) < 0.25 {
            Some(aurora_render::PortalClip::hidden())
        } else {
            aurora_render::PortalClip::new(self.center(), Vec3::NEG_Z)
        }
    }
}

pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

impl State {
    pub(super) fn sync_arrivals(&mut self, world: &World, now: Instant) {
        self.arrivals.retain(|traveler, effect| {
            !effect.finished(now)
                && if *traveler == world.agent_id {
                    !world.mutes.is_muted_id(&effect.event.creator)
                } else {
                    source_shown(world, *traveler) && source_shown(world, effect.event.creator)
                }
        });
        if self.waiting_arrivals.is_empty() {
            return;
        }
        let unix_ms = unix_millis();
        self.waiting_arrivals.retain(|traveler, event| {
            if !event.live(unix_ms) || !source_kept(world, *traveler) || !source_kept(world, event.creator) {
                return false;
            }
            if source_shown(world, *traveler) && source_shown(world, event.creator) {
                self.arrivals.insert(*traveler, Effect::new(event.clone(), now, unix_ms));
                return false;
            }
            true
        });
    }

    pub fn begin_arrival(&mut self, creator: Uuid, position: Vec3, floor: f32, height: f32, now: Instant) -> bool {
        let Some(region) = self.region else { return false };
        if self.agent.is_nil() {
            return false;
        }
        let unix_ms = unix_millis();
        let event = Arrival {
            id: Uuid::new_v4(),
            creator,
            region,
            position: position.to_array(),
            floor,
            height,
            starts: unix_ms + 250,
        };
        if !event.valid() {
            return false;
        }
        let mut effect = Effect::new(event, now, unix_ms);
        effect.awaiting_land = true;
        self.arrivals.insert(self.agent, effect);
        true
    }

    pub fn awaiting_arrival_land(&self) -> Option<Vec3> {
        self.arrivals
            .get(&self.agent)
            .filter(|effect| effect.awaiting_land)
            .map(Effect::center)
    }

    /// Only the traveler's presentation starts immediately. Share the same
    /// event, with its original clock, if this parcel permits rez in time.
    pub fn arrival_land_known(&mut self, build: BuildRight, now: Instant) {
        let Some(effect) = self.arrivals.get_mut(&self.agent) else { return };
        if !effect.awaiting_land || build == BuildRight::Unknown {
            return;
        }
        effect.awaiting_land = false;
        if build == BuildRight::Allowed && !effect.finished(now) && self.out.len() < LIMIT * 2 {
            self.out.push_front((Uuid::nil(), Message::Arrival(effect.event.clone())));
        }
    }

    pub(super) fn receive_arrival(&mut self, world: &World, owner: Uuid, event: Arrival, now: Instant) {
        let unix_ms = unix_millis();
        if !event.live(unix_ms)
            || world
                .main()
                .is_some_and(|region| !fits_region(Vec3::from(event.position), [region.heightmap.size_x, region.heightmap.size_y]))
            || !source_kept(world, owner)
            || !source_kept(world, event.creator)
            || (self.arrivals.len() + self.waiting_arrivals.len() >= LIMIT
                && !self.arrivals.contains_key(&owner)
                && !self.waiting_arrivals.contains_key(&owner))
            || self.arrivals.get(&owner).is_some_and(|old| old.event.starts >= event.starts)
            || self.waiting_arrivals.get(&owner).is_some_and(|old| old.starts >= event.starts)
        {
            return;
        }
        if source_shown(world, owner) && source_shown(world, event.creator) {
            self.waiting_arrivals.remove(&owner);
            self.arrivals.insert(owner, Effect::new(event, now, unix_ms));
        } else {
            self.want_profile(world, owner);
            self.want_profile(world, event.creator);
            self.arrivals.remove(&owner);
            self.waiting_arrivals.insert(owner, event);
        }
    }

    pub fn arrival_offset(&self, owner: Uuid, root: Vec3, now: Instant) -> f32 {
        if self.reduced_motion {
            return 0.0;
        }
        self.arrivals.get(&owner).filter(|e| e.matches(root)).map_or(0.0, |e| e.offset(now))
    }

    pub fn arrival_hold(&self, owner: Uuid, root: Vec3, now: Instant) -> bool {
        !self.reduced_motion
            && self
                .arrivals
                .get(&owner)
                .is_some_and(|e| e.matches(root) && e.age(now) < e.event.fall_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> Arrival {
        Arrival {
            id: Uuid::from_u128(7),
            creator: Uuid::from_u128(30),
            region: 256,
            position: [10.0, 10.0, 25.0],
            floor: 24.0,
            height: 1.8,
            starts: 1000,
        }
    }

    #[test]
    fn floor_is_required_within_two_metres_and_invalid_inputs_never_open() {
        let base = event();
        for floor in [25.0, 24.0, 23.0] {
            assert!(Arrival { floor, ..base.clone() }.valid());
        }
        for floor in [25.01, 22.99, f32::NAN, f32::INFINITY] {
            assert!(!Arrival { floor, ..base.clone() }.valid());
        }
        assert!(
            !Arrival {
                height: f32::NAN,
                ..base.clone()
            }
            .valid()
        );
        assert!(
            !Arrival {
                position: [f32::NAN, 0.0, 0.0],
                ..base
            }
            .valid()
        );
    }

    #[test]
    fn the_shared_horizontal_aperture_must_fit_between_the_region_edges() {
        let size = [256, 512];
        assert!(fits_region(Vec3::new(WIDTH, HEIGHT, 25.0), size));
        assert!(fits_region(Vec3::new(256.0 - WIDTH, 512.0 - HEIGHT, 25.0), size));
        assert!(!fits_region(Vec3::new(WIDTH - 0.1, 256.0, 25.0), size));
        assert!(!fits_region(Vec3::new(128.0, 512.0 - HEIGHT + 0.1, 25.0), size));
    }

    #[test]
    fn the_body_falls_with_gravity_then_fully_clears_the_horizontal_front_before_closing() {
        let now = Instant::now();
        let e = Effect::new(event(), now, 1000);
        assert!(e.clip(now).unwrap().is_hidden());
        assert_eq!(e.scale(now), 0.0);
        assert_eq!(e.scale(now + Duration::from_millis(250)), 1.0);
        let root = Vec3::from(e.event.position);
        let mut previous = e.offset(now);
        for frame in 1..=120 {
            let at = now + Duration::from_secs_f32(frame as f32 / 60.0);
            let offset = e.offset(at);
            assert!(offset <= previous);
            previous = offset;
        }
        let falling = e.clip(now + Duration::from_millis(500)).unwrap();
        assert!(falling.normal().dot(Vec3::NEG_Z) > 0.99);
        let end = now + Duration::from_secs_f32(e.event.fall_end());
        assert!(e.offset(end) < 0.0001);
        assert!(e.clip(end).unwrap().keeps_point(root + Vec3::Z * (e.event.height * 0.5)));
        assert_eq!(e.scale(now + Duration::from_secs_f32(e.event.duration())), 0.0);
        assert!(e.clip(now + Duration::from_secs(4)).is_none());
    }

    #[test]
    fn arrival_is_subject_scoped_finite_and_never_creates_a_return_gate() {
        let (mut world, mut state) = super::super::tests::fixture();
        let now = Instant::now();
        let root = world.agent.position;
        assert!(state.begin_arrival(world.agent_id, root, root.z - 0.84, 1.8, now));
        assert_eq!(state.arrivals.len(), 1);
        assert!(state.entries.is_empty());
        assert!(state.arrival_offset(world.agent_id, root, now) > 2.0);
        assert_eq!(state.arrival_offset(Uuid::from_u128(300), root, now), 0.0);
        assert_eq!(state.arrival_offset(world.agent_id, root + Vec3::X * 10.0, now), 0.0);
        assert!(state.arrival_hold(world.agent_id, root, now));
        for step in 0..10 {
            world.agent.position.x += 0.1;
            assert!(state.update(&world, Vec3::X, now + Duration::from_millis(step * 100)).is_none());
        }
        state.reduced_motion = true;
        assert_eq!(state.arrival_offset(world.agent_id, root, now), 0.0);
        assert!(!state.arrival_hold(world.agent_id, root, now));
        let effect = state.arrivals.get_mut(&world.agent_id).unwrap();
        effect.at -= Duration::from_secs(5);
        state.sync(&world, None, 1000);
        assert!(state.arrivals.is_empty());
        assert!(state.entries.is_empty());
    }

    #[test]
    fn source_preferences_filter_remote_emergence_without_hiding_our_own() {
        let (mut world, mut state) = super::super::tests::fixture();
        let now = Instant::now();
        world.portal_sources = Sources::Nobody;
        let owner = Uuid::from_u128(20);
        let mut event = event();
        let creator = event.creator;
        event.starts = unix_millis() + 250;
        let wire = super::super::tests::wire(owner, Message::Arrival(event));
        state.receive(&world, &wire, now, unix_time());
        assert!(!state.arrivals.contains_key(&owner));
        world.portal_sources = Sources::Friends;
        world.social.form_friendship(owner);
        state.receive(&world, &wire, now, unix_time());
        assert!(!state.arrivals.contains_key(&owner), "the traveler cannot vouch for the creator");
        world.social.form_friendship(creator);
        world.social.end_friendship(&owner);
        state.receive(&world, &wire, now, unix_time());
        assert!(
            !state.arrivals.contains_key(&owner),
            "knowing the creator cannot vouch for the traveler"
        );
        world.social.form_friendship(owner);
        state.receive(&world, &wire, now, unix_time());
        assert!(state.arrivals.contains_key(&owner));
        world.social.end_friendship(&creator);
        state.sync(&world, None, unix_time());
        assert!(!state.arrivals.contains_key(&owner), "recheck the source during a running effect");
        let root = world.agent.position;
        assert!(state.begin_arrival(creator, root, root.z - 0.84, 1.8, now));
        world.portal_sources = Sources::Nobody;
        state.sync(&world, None, unix_time());
        assert!(!state.arrivals.contains_key(&owner));
        assert!(state.arrivals.contains_key(&world.agent_id));
    }

    #[test]
    fn the_arrival_stays_personal_until_its_own_parcel_allows_sharing() {
        let (world, mut state) = super::super::tests::fixture();
        let now = Instant::now();
        let root = world.agent.position;
        for (right, elapsed, should_share) in [
            (BuildRight::Denied, 100, false),
            (BuildRight::Allowed, 100, true),
            (BuildRight::Allowed, 5000, false),
        ] {
            assert!(state.begin_arrival(world.agent_id, root, root.z - 0.84, 1.8, now));
            let event = state.arrivals[&world.agent_id].event.clone();
            assert_eq!(state.awaiting_arrival_land(), Some(state.arrivals[&world.agent_id].center()));
            assert!(state.arrival_hold(world.agent_id, root, now));
            state.arrival_land_known(BuildRight::Unknown, now);
            assert!(state.awaiting_arrival_land().is_some());
            assert!(state.out.is_empty());
            let at = now + Duration::from_millis(elapsed);
            state.arrival_land_known(right, at);
            assert!(state.awaiting_arrival_land().is_none());
            state.arrival_land_known(BuildRight::Allowed, at);
            if should_share {
                assert_eq!(state.out.pop_front(), Some((Uuid::nil(), Message::Arrival(event))));
            }
            assert!(state.out.is_empty());
        }
    }

    #[test]
    fn group_resolution_may_reveal_only_a_still_live_arrival() {
        let (mut world, mut state) = super::super::tests::fixture();
        world.portal_sources = Sources::FriendsAndGroups;
        let now = Instant::now();
        let owner = Uuid::from_u128(20);
        let mut event = event();
        event.starts = unix_millis() + 250;
        let creator = event.creator;
        let wire = super::super::tests::wire(owner, Message::Arrival(event));
        state.receive(&world, &wire, now, unix_time());
        assert_eq!((state.take_profile_want(), state.take_profile_want()), (Some(owner), Some(creator)));
        assert!(state.arrivals.is_empty());
        world.social.form_friendship(owner);
        world.social.form_friendship(creator);
        state.sync(&world, None, unix_time());
        assert!(state.arrivals.contains_key(&owner));
        assert!(state.waiting_arrivals.is_empty());
        state.arrivals.clear();
        world.social.end_friendship(&creator);
        state.receive(&world, &wire, now, unix_time());
        state.waiting_arrivals.get_mut(&owner).unwrap().starts = unix_millis() - 6000;
        world.social.form_friendship(creator);
        state.sync(&world, None, unix_time());
        assert!(state.arrivals.is_empty());
        assert!(state.waiting_arrivals.is_empty());
    }

    #[test]
    fn authenticated_arrival_replays_and_stale_messages_cannot_restart_the_drop() {
        let (world, mut state) = super::super::tests::fixture();
        let now = Instant::now();
        let owner = Uuid::from_u128(20);
        let mut event = event();
        event.starts = unix_millis() + 250;
        let wire = super::super::tests::wire(owner, Message::Arrival(event.clone()));
        state.receive(&world, &wire, now, unix_time());
        assert!(state.arrivals.contains_key(&owner));
        assert!(!state.arrivals.contains_key(&world.agent_id));
        let anchor = state.arrivals[&owner].at;
        state.receive(&world, &wire, now + Duration::from_secs(1), unix_time());
        assert_eq!(state.arrivals[&owner].at, anchor);
        event.id = Uuid::new_v4();
        event.starts = unix_millis() - 6000;
        state.receive(&world, &super::super::tests::wire(owner, Message::Arrival(event)), now, unix_time());
        assert_eq!(state.arrivals[&owner].at, anchor);
        assert!(state.entries.is_empty());
    }
}
