//! Radar of the People window: the avatars around us, swept once a second
//! even while the window is closed, with what an entry gathers over time
//! (first sighting, previous range) and the crossings it reports. Port of
//! FSRadar::updateRadarList and FSRadarEntry (indra/newview/fsradar.cpp,
//! fsradarentry.cpp, originally LGPL 2.1).

use super::World;
use aurora_net::RegionHandle;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Range of the local chat (CHAT_NORMAL_RADIUS).
pub const CHAT_RANGE: f32 = 20.0;
/// Range of a shout (CHAT_SHOUT_RADIUS).
pub const SHOUT_RANGE: f32 = 100.0;
/// FS_RADAR_LIST_UPDATE_INTERVAL.
const SWEEP_INTERVAL: f64 = 1.0;
/// Profiles asked per sweep: a crowded region does not send a hundred
/// requests at once (Firestorm asks for all of them as the entries appear).
const PROFILES_PER_SWEEP: usize = 4;

const ANIM_AGENT_TYPE: Uuid = Uuid::from_u128(0xc541c47f_e0c0_058b_ad1a_d6ae3a4584d9);
const ANIM_AGENT_SIT_GROUND: Uuid = Uuid::from_u128(0x1c7600d6_661f_b87b_efe2_d7421eb93c86);

/// An avatar as one sweep sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    pub id: Uuid,
    pub region: RegionHandle,
    /// Position in the frame of `World::region_offset`.
    pub position: Vec3,
    /// False for a coarse location without altitude: its range is unknown.
    pub altitude_known: bool,
    /// The avatar object is loaded (exact position, typing and sitting known).
    pub loaded: bool,
    pub sitting: bool,
    pub typing: bool,
}

/// An avatar of the radar list.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: Uuid,
    pub region: RegionHandle,
    pub position: Vec3,
    /// Distance to our avatar; None when unknown.
    pub range: Option<f32>,
    pub same_region: bool,
    pub loaded: bool,
    pub sitting: bool,
    pub typing: bool,
    /// Time of the first sweep that saw it; it starts again when the avatar
    /// leaves and comes back.
    pub first_seen: f64,
    profile_asked: bool,
    age_checked: bool,
}

/// A crossing reported by a sweep. The distances are the new ranges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    ChatEnter(Uuid, f32),
    ChatLeave(Uuid),
    DrawEnter(Uuid, f32),
    DrawLeave(Uuid),
    RegionEnter(Uuid, Option<f32>),
    RegionLeave(Uuid),
    /// Age in days, at most the alert's threshold.
    AgeAlert(Uuid, i64),
}

impl Event {
    pub fn avatar(&self) -> Uuid {
        match *self {
            Event::ChatEnter(id, _)
            | Event::ChatLeave(id)
            | Event::DrawEnter(id, _)
            | Event::DrawLeave(id)
            | Event::RegionEnter(id, _)
            | Event::RegionLeave(id)
            | Event::AgeAlert(id, _) => id,
        }
    }
}

#[derive(Default)]
pub struct Radar {
    entries: HashMap<Uuid, Entry>,
    last_sweep: Option<f64>,
    /// Avatars of the last sweep, hidden ones included (the "total" of the
    /// list's title).
    pub total: usize,
}

/// True when `range` is known and within `limit`.
fn within(range: Option<f32>, limit: f32) -> bool {
    range.is_some_and(|d| d <= limit)
}

impl Radar {
    /// A sweep is due (one per second).
    pub fn due(&self, now: f64) -> bool {
        self.last_sweep.is_none_or(|t| now - t >= SWEEP_INTERVAL)
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn get(&self, id: &Uuid) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Take the avatars seen now and report who crossed the chat range, the
    /// draw distance and the border of our region since the last sweep
    /// (FSRadar::updateRadarList).
    pub fn sweep(&mut self, seen: Vec<Sighting>, me: Vec3, my_region: Option<RegionHandle>, draw_distance: f32, now: f64) -> Vec<Event> {
        self.last_sweep = Some(now);
        self.total = seen.len();
        let mut events = Vec::new();
        let mut present = HashSet::with_capacity(seen.len());
        for s in seen {
            present.insert(s.id);
            let range = s.altitude_known.then(|| s.position.distance(me));
            let same_region = Some(s.region) == my_region;
            let old = self.entries.get(&s.id);
            let (was_chat, was_draw) = match old {
                Some(e) => (within(e.range, CHAT_RANGE), within(e.range, draw_distance)),
                None => (false, false),
            };
            if let Some(d) = range {
                if d <= CHAT_RANGE && !was_chat {
                    events.push(Event::ChatEnter(s.id, d));
                }
                if d <= draw_distance && !was_draw {
                    events.push(Event::DrawEnter(s.id, d));
                }
            }
            if was_chat && !within(range, CHAT_RANGE) {
                events.push(Event::ChatLeave(s.id));
            }
            if was_draw && !within(range, draw_distance) {
                events.push(Event::DrawLeave(s.id));
            }
            match old {
                None if same_region => events.push(Event::RegionEnter(s.id, range)),
                Some(e) if e.region != s.region => {
                    if same_region {
                        events.push(Event::RegionEnter(s.id, range));
                    } else if e.same_region {
                        events.push(Event::RegionLeave(s.id));
                    }
                }
                _ => {}
            }
            let (first_seen, profile_asked, age_checked) = match old {
                Some(e) => (e.first_seen, e.profile_asked, e.age_checked),
                None => (now, false, false),
            };
            self.entries.insert(
                s.id,
                Entry {
                    id: s.id,
                    region: s.region,
                    position: s.position,
                    range,
                    same_region,
                    loaded: s.loaded,
                    sitting: s.sitting,
                    typing: s.typing,
                    first_seen,
                    profile_asked,
                    age_checked,
                },
            );
        }
        // the ones that went away leave what they were in
        self.entries.retain(|id, e| {
            if present.contains(id) {
                return true;
            }
            if within(e.range, CHAT_RANGE) {
                events.push(Event::ChatLeave(*id));
            }
            if within(e.range, draw_distance) {
                events.push(Event::DrawLeave(*id));
            }
            if e.same_region {
                events.push(Event::RegionLeave(*id));
            }
            false
        });
        events
    }

    /// The next entries whose profile (age, payment info, notes) is to be
    /// asked for, nearest first.
    pub fn take_profile_wants(&mut self) -> Vec<Uuid> {
        let mut waiting: Vec<&mut Entry> = self.entries.values_mut().filter(|e| !e.profile_asked).collect();
        waiting.sort_by(|a, b| a.range.unwrap_or(f32::MAX).total_cmp(&b.range.unwrap_or(f32::MAX)));
        waiting
            .into_iter()
            .take(PROFILES_PER_SWEEP)
            .map(|e| {
                e.profile_asked = true;
                e.id
            })
            .collect()
    }

    /// The age alert, once per entry, when its age gets known
    /// (FSRadarEntry::checkAge): `age` gives the days of an avatar whose
    /// profile is there and shows its age.
    pub fn age_alerts(&mut self, threshold: i64, age: impl Fn(&Uuid) -> Option<i64>) -> Vec<Event> {
        let mut events = Vec::new();
        for e in self.entries.values_mut().filter(|e| !e.age_checked) {
            if let Some(days) = age(&e.id) {
                e.age_checked = true;
                if (0..=threshold).contains(&days) {
                    events.push(Event::AgeAlert(e.id, days));
                }
            }
        }
        events
    }
}

/// The avatars around us: the loaded ones first (exact position), then the
/// coarse locations of every region we see, neighbors included. A coarse
/// altitude of 0 or 1020 (the byte 0 or 255) means the server does not know
/// it.
pub fn sightings(world: &World) -> Vec<Sighting> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (_, o) in world.objects.avatars() {
        if o.full_id == world.agent_id || seen.contains(&o.full_id) {
            continue;
        }
        let Some(offset) = world.region_offset(o.key.region) else {
            continue;
        };
        // a seated avatar is placed relative to its seat, itself maybe a
        // child of its object
        let mut local = o.position;
        let mut parent = world.objects.parent_of(o).and_then(|i| world.objects.get(i));
        while let Some(up) = parent {
            local = up.position + up.rotation * local;
            parent = world.objects.parent_of(up).and_then(|i| world.objects.get(i));
        }
        let animations = world.animations_of(&o.full_id);
        let playing = |id: Uuid| animations.iter().any(|a| a.id == id);
        seen.insert(o.full_id);
        out.push(Sighting {
            id: o.full_id,
            region: o.key.region,
            position: offset + local,
            altitude_known: true,
            loaded: true,
            sitting: o.parent_id != 0 || playing(ANIM_AGENT_SIT_GROUND) || playing(super::body::ANIM_SIT_GROUND_CONSTRAINED),
            typing: playing(ANIM_AGENT_TYPE),
        });
    }
    for (region, list) in &world.coarse {
        let Some(offset) = world.region_offset(*region) else {
            continue;
        };
        for (id, position) in list {
            if *id == world.agent_id || !seen.insert(*id) {
                continue;
            }
            out.push(Sighting {
                id: *id,
                region: *region,
                position: offset + *position,
                altitude_known: position.z != 0.0 && position.z != 1020.0,
                loaded: false,
                sitting: false,
                typing: false,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: RegionHandle = 1;
    const THERE: RegionHandle = 2;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn at(n: u128, region: RegionHandle, x: f32) -> Sighting {
        Sighting {
            id: id(n),
            region,
            position: Vec3::new(x, 0.0, 0.0),
            altitude_known: true,
            loaded: true,
            sitting: false,
            typing: false,
        }
    }

    fn sweep(radar: &mut Radar, seen: Vec<Sighting>, now: f64) -> Vec<Event> {
        radar.sweep(seen, Vec3::ZERO, Some(HERE), 96.0, now)
    }

    #[test]
    fn crossing_the_chat_range_and_the_draw_distance() {
        let mut radar = Radar::default();
        // appears far: nothing but the region
        assert_eq!(
            sweep(&mut radar, vec![at(1, HERE, 150.0)], 0.0),
            vec![Event::RegionEnter(id(1), Some(150.0))]
        );
        assert_eq!(sweep(&mut radar, vec![at(1, HERE, 60.0)], 1.0), vec![Event::DrawEnter(id(1), 60.0)]);
        assert_eq!(sweep(&mut radar, vec![at(1, HERE, 12.0)], 2.0), vec![Event::ChatEnter(id(1), 12.0)]);
        // staying inside says nothing
        assert_eq!(sweep(&mut radar, vec![at(1, HERE, 8.0)], 3.0), vec![]);
        assert_eq!(sweep(&mut radar, vec![at(1, HERE, 40.0)], 4.0), vec![Event::ChatLeave(id(1))]);
        assert_eq!(sweep(&mut radar, vec![at(1, HERE, 120.0)], 5.0), vec![Event::DrawLeave(id(1))]);
    }

    #[test]
    fn a_new_avatar_within_range_enters_everything_at_once() {
        let mut radar = Radar::default();
        assert_eq!(
            sweep(&mut radar, vec![at(1, HERE, 5.0)], 0.0),
            vec![
                Event::ChatEnter(id(1), 5.0),
                Event::DrawEnter(id(1), 5.0),
                Event::RegionEnter(id(1), Some(5.0))
            ]
        );
        // and leaves everything when it disappears
        assert_eq!(
            sweep(&mut radar, vec![], 1.0),
            vec![Event::ChatLeave(id(1)), Event::DrawLeave(id(1)), Event::RegionLeave(id(1))]
        );
        assert_eq!(radar.entries().count(), 0);
    }

    #[test]
    fn an_unknown_range_is_outside_every_range() {
        let mut radar = Radar::default();
        sweep(&mut radar, vec![at(1, HERE, 10.0)], 0.0);
        let mut lost = at(1, HERE, 10.0);
        lost.altitude_known = false;
        assert_eq!(
            sweep(&mut radar, vec![lost.clone()], 1.0),
            vec![Event::ChatLeave(id(1)), Event::DrawLeave(id(1))]
        );
        assert_eq!(radar.get(&id(1)).and_then(|e| e.range), None);
        // unknown then known again: it enters again
        assert_eq!(sweep(&mut radar, vec![lost], 2.0), vec![]);
        assert_eq!(
            sweep(&mut radar, vec![at(1, HERE, 10.0)], 3.0),
            vec![Event::ChatEnter(id(1), 10.0), Event::DrawEnter(id(1), 10.0)]
        );
    }

    #[test]
    fn region_borders() {
        let mut radar = Radar::default();
        // seen in a neighbor region: not ours
        assert_eq!(sweep(&mut radar, vec![at(1, THERE, 200.0)], 0.0), vec![]);
        assert_eq!(
            sweep(&mut radar, vec![at(1, HERE, 200.0)], 1.0),
            vec![Event::RegionEnter(id(1), Some(200.0))]
        );
        assert_eq!(sweep(&mut radar, vec![at(1, THERE, 200.0)], 2.0), vec![Event::RegionLeave(id(1))]);
        // gone from the neighbor region: it had left ours already
        assert_eq!(sweep(&mut radar, vec![], 3.0), vec![]);
    }

    #[test]
    fn the_first_sighting_starts_again_after_a_return() {
        let mut radar = Radar::default();
        sweep(&mut radar, vec![at(1, HERE, 200.0)], 10.0);
        sweep(&mut radar, vec![at(1, HERE, 200.0)], 25.0);
        assert_eq!(radar.get(&id(1)).map(|e| e.first_seen), Some(10.0));
        sweep(&mut radar, vec![], 26.0);
        sweep(&mut radar, vec![at(1, HERE, 200.0)], 40.0);
        assert_eq!(radar.get(&id(1)).map(|e| e.first_seen), Some(40.0));
    }

    #[test]
    fn one_sweep_per_second() {
        let mut radar = Radar::default();
        assert!(radar.due(5.0));
        sweep(&mut radar, vec![], 5.0);
        assert!(!radar.due(5.6));
        assert!(radar.due(6.0));
    }

    #[test]
    fn profiles_are_asked_once_nearest_first() {
        let mut radar = Radar::default();
        let seen: Vec<Sighting> = (1..=6).map(|n| at(n, HERE, 300.0 - n as f32 * 10.0)).collect();
        sweep(&mut radar, seen.clone(), 0.0);
        assert_eq!(radar.take_profile_wants(), vec![id(6), id(5), id(4), id(3)]);
        sweep(&mut radar, seen, 1.0);
        assert_eq!(radar.take_profile_wants(), vec![id(2), id(1)]);
        assert!(radar.take_profile_wants().is_empty());
    }

    #[test]
    fn the_age_alert_fires_once_when_the_age_gets_known() {
        let mut radar = Radar::default();
        sweep(&mut radar, vec![at(1, HERE, 200.0), at(2, HERE, 210.0)], 0.0);
        // no profile yet
        assert_eq!(radar.age_alerts(7, |_| None), vec![]);
        let age = |who: &Uuid| if *who == id(1) { Some(3) } else { Some(4000) };
        assert_eq!(radar.age_alerts(7, age), vec![Event::AgeAlert(id(1), 3)]);
        assert_eq!(radar.age_alerts(7, age), vec![]);
    }
}
