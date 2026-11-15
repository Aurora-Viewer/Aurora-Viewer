//! One-way presentation only: no invitation, permission grant or crossing gate.
use super::*;
use aurora_net::bridge::portal::Arrival;

pub struct Effect {
    pub event: Arrival,
    at: Instant,
    age: f32,
}

impl Effect {
    pub fn new(event: Arrival, at: Instant, unix_ms: u64) -> Self {
        let age = if unix_ms >= event.starts {
            (unix_ms - event.starts) as f32 * 0.001
        } else {
            -((event.starts - unix_ms) as f32) * 0.001
        };
        Self { event, at, age }
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
    pub fn begin_arrival(&mut self, position: Vec3, floor: f32, height: f32, now: Instant) -> bool {
        let Some(region) = self.region else { return false };
        if self.agent.is_nil() {
            return false;
        }
        let unix_ms = unix_millis();
        let event = Arrival {
            id: Uuid::new_v4(),
            region,
            position: position.to_array(),
            floor,
            height,
            starts: unix_ms + 250,
        };
        if !event.valid() {
            return false;
        }
        self.arrivals.insert(self.agent, Effect::new(event.clone(), now, unix_ms));
        if self.out.len() < LIMIT * 2 {
            self.out.push_front((Uuid::nil(), Message::Arrival(event)));
        }
        true
    }

    pub(super) fn receive_arrival(&mut self, owner: Uuid, event: Arrival, now: Instant) {
        let unix_ms = unix_millis();
        if !event.live(unix_ms)
            || self.arrivals.len() >= LIMIT && !self.arrivals.contains_key(&owner)
            || self.arrivals.get(&owner).is_some_and(|old| old.event.starts >= event.starts)
        {
            return;
        }
        self.arrivals.insert(owner, Effect::new(event, now, unix_ms));
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
        assert!(state.begin_arrival(root, root.z - 0.84, 1.8, now));
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
        event.starts = unix_millis() + 250;
        let wire = super::super::tests::wire(owner, Message::Arrival(event));
        state.receive(&world, &wire, now, unix_time());
        assert!(!state.arrivals.contains_key(&owner));
        world.portal_sources = Sources::Friends;
        world.social.form_friendship(owner);
        state.receive(&world, &wire, now, unix_time());
        assert!(state.arrivals.contains_key(&owner));
        let root = world.agent.position;
        assert!(state.begin_arrival(root, root.z - 0.84, 1.8, now));
        world.portal_sources = Sources::Nobody;
        state.sync(&world, None, unix_time());
        assert!(!state.arrivals.contains_key(&owner));
        assert!(state.arrivals.contains_key(&world.agent_id));
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
