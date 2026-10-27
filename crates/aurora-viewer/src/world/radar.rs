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

/// Range of a whisper (CHAT_WHISPER_RADIUS).
pub const WHISPER_RANGE: f32 = 10.0;
/// Range of the local chat (CHAT_NORMAL_RADIUS).
pub const CHAT_RANGE: f32 = 20.0;
/// Range of a shout (CHAT_SHOUT_RADIUS).
pub const SHOUT_RANGE: f32 = 100.0;
/// FS_RADAR_LIST_UPDATE_INTERVAL.
const SWEEP_INTERVAL: f64 = 1.0;
/// Seconds the avatars of a region left behind by a teleport stay in the
/// list, faded, before they leave it (asked by the human: like Firestorm,
/// we hear of them until the old simulator disables itself or its circuit
/// times out, a minute later, and they looked as if they were still around).
pub const LEFT_BEHIND_SECS: f64 = 10.0;
/// Profiles asked per sweep: a crowded region does not send a hundred
/// requests at once (Firestorm asks for all of them as the entries appear).
const PROFILES_PER_SWEEP: usize = 4;
/// Avatars per alert to scripts: what fits in the 255 bytes of a button
/// label (FSRADAR_MAX_AVATARS_PER_ALERT).
const AVATARS_PER_SCRIPT_ALERT: usize = 6;
/// Default channel of the alerts to scripts (RadarAlertChannel).
pub const SCRIPT_ALERT_CHANNEL: i32 = -777_777_777;
/// A sound our own object triggers to ask for every avatar on that channel
/// (RadarLegacyChannelAlertRefreshUUID, the Phoenix radar scripts' request).
pub const SCRIPT_ALERT_SYNC_SOUND: Uuid = Uuid::from_u128(0x76c78607_93f9_f55a_5238_e19b1a181389);

/// The typing animation (ANIM_AGENT_TYPE).
pub const ANIM_AGENT_TYPE: Uuid = Uuid::from_u128(0xc541c47f_e0c0_058b_ad1a_d6ae3a4584d9);
pub const ANIM_AGENT_SIT_GROUND: Uuid = Uuid::from_u128(0x1c7600d6_661f_b87b_efe2_d7421eb93c86);

/// An avatar as one sweep sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    pub id: Uuid,
    pub region: RegionHandle,
    /// Position in the frame of `World::region_offset`.
    pub position: Vec3,
    /// False for a coarse location without altitude: its range is unknown.
    pub altitude_known: bool,
    /// The avatar object is loaded, with an exact native position.
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
    /// Its region was left behind by a teleport: it leaves the list within
    /// `LEFT_BEHIND_SECS`.
    pub left_behind: bool,
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

    /// The interface sound of the alert (UISndRadar*).
    pub fn sound(&self) -> crate::ui_sound::UiSound {
        use crate::ui_sound::UiSound as S;
        match self {
            Event::ChatEnter(..) => S::RadarChatEnter,
            Event::ChatLeave(_) => S::RadarChatLeave,
            Event::DrawEnter(..) => S::RadarDrawEnter,
            Event::DrawLeave(_) => S::RadarDrawLeave,
            Event::RegionEnter(..) => S::RadarRegionEnter,
            Event::RegionLeave(_) => S::RadarRegionLeave,
            Event::AgeAlert(..) => S::RadarAgeAlert,
        }
    }
}

#[derive(Default)]
pub struct Radar {
    entries: HashMap<Uuid, Entry>,
    last_sweep: Option<f64>,
    /// Our region at the last sweep.
    region: Option<RegionHandle>,
    /// Regions we were hearing from when we changed region and that the new
    /// place leaves out of reach, with the time of the change.
    left: HashMap<RegionHandle, f64>,
    /// Avatars of the last sweep, hidden ones included (the "total" of the
    /// list's title).
    pub total: usize,
    /// New in the list at the last sweep, with whether they are in our
    /// region; gone from it at the last sweep.
    arrived: Vec<(Uuid, bool)>,
    departed: Vec<Uuid>,
    /// A script asked for every avatar (`SCRIPT_ALERT_SYNC_SOUND`).
    sync_asked: bool,
    /// Number of the last alert to scripts (mRadarFrameCount).
    script_frame: u32,
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

    /// The regions a change of region leaves behind. `gaps` gives how far
    /// each region we hear from is (`region_gaps`): one within the draw
    /// distance, or touching ours, is a neighbor the simulator keeps us in,
    /// so a border crossing leaves nothing behind; neither does a region we
    /// come back to, or get near again.
    fn leave_regions(&mut self, my_region: Option<RegionHandle>, gaps: &HashMap<RegionHandle, f32>, draw_distance: f32, now: f64) {
        if my_region != self.region {
            if self.region.is_some() {
                for e in self.entries.values() {
                    self.left.entry(e.region).or_insert(now);
                }
            }
            self.region = my_region;
        }
        self.left
            .retain(|region, _| Some(*region) != my_region && gaps.get(region).is_some_and(|gap| *gap > draw_distance));
    }

    /// Take the avatars seen now and report who crossed the chat range, the
    /// draw distance and the border of our region since the last sweep
    /// (FSRadar::updateRadarList). The avatars of a region a teleport left
    /// behind are marked, then dropped after `LEFT_BEHIND_SECS`.
    pub fn sweep(
        &mut self,
        seen: Vec<Sighting>,
        me: Vec3,
        my_region: Option<RegionHandle>,
        gaps: &HashMap<RegionHandle, f32>,
        draw_distance: f32,
        now: f64,
    ) -> Vec<Event> {
        self.last_sweep = Some(now);
        self.leave_regions(my_region, gaps, draw_distance, now);
        self.arrived.clear();
        self.departed.clear();
        let mut events = Vec::new();
        let mut present = HashSet::with_capacity(seen.len());
        for s in seen {
            let left_behind = match self.left.get(&s.region) {
                Some(since) if now - since >= LEFT_BEHIND_SECS => continue,
                since => since.is_some(),
            };
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
                None => {
                    self.arrived.push((s.id, same_region));
                    (now, false, false)
                }
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
                    left_behind,
                    profile_asked,
                    age_checked,
                },
            );
        }
        self.total = present.len();
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
            self.departed.push(*id);
            false
        });
        events
    }

    /// A script of ours asked for every avatar around
    /// (FSRadar::requestRadarChannelAlertSync): the next alert to scripts
    /// lists all of them as entering.
    pub fn ask_script_sync(&mut self) {
        self.sync_asked = true;
    }

    /// The button labels of the alerts to scripts for the last sweep
    /// (FSRadar::updateRadarList, steps 2b and 3b): `frame,count,key,…` for
    /// the arrivals, `frame,-count,key,…` for the departures, six keys a
    /// message. Without the exit alerts, only arrivals in our region are
    /// reported (the Phoenix scripts' convention). A sync asked by a script
    /// adds every avatar already there, whatever the settings.
    pub fn script_alerts(&mut self, enter: bool, leave: bool) -> Vec<String> {
        let sync = std::mem::take(&mut self.sync_asked);
        let mut entering: Vec<Uuid> = Vec::new();
        if enter || sync {
            entering.extend(self.arrived.iter().filter(|(_, here)| leave || *here).map(|(id, _)| *id));
        }
        if sync {
            let new: HashSet<Uuid> = self.arrived.iter().map(|(id, _)| *id).collect();
            entering.extend(self.entries.keys().filter(|id| !new.contains(id)));
        }
        let mut labels = Vec::new();
        for (ids, sign) in [(entering, ""), (if leave { self.departed.clone() } else { Vec::new() }, "-")] {
            if ids.is_empty() {
                continue;
            }
            self.script_frame = self.script_frame.wrapping_add(1);
            for chunk in ids.chunks(AVATARS_PER_SCRIPT_ALERT) {
                let mut label = format!("{},{sign}{}", self.script_frame, chunk.len());
                for id in chunk {
                    label.push(',');
                    label.push_str(&id.to_string());
                }
                labels.push(label);
            }
        }
        labels
    }

    /// An avatar seen since `at` at least (the demo's residents arrived
    /// before the session): its first sighting goes back to it.
    pub fn backdate(&mut self, id: &Uuid, at: f64) {
        if let Some(e) = self.entries.get_mut(id) {
            e.first_seen = e.first_seen.min(at);
        }
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

/// Distance from our avatar to each region we hear from, in meters on the
/// ground: 0 inside ours and for the regions touching it, corners included
/// (they stay our neighbors whatever the draw distance), else to the nearest
/// point of the region.
pub fn region_gaps(world: &World) -> HashMap<RegionHandle, f32> {
    let Some(main) = world.main_region else {
        return HashMap::new();
    };
    let size = |h: &RegionHandle| {
        world.regions.get(h).map_or(glam::Vec2::splat(256.0), |r| {
            glam::Vec2::new(r.heightmap.size_x as f32, r.heightmap.size_y as f32)
        })
    };
    let me = world.agent.position.truncate();
    let main_size = size(&main);
    world
        .regions
        .keys()
        .chain(world.coarse.keys())
        .filter_map(|h| Some((*h, region_gap(me, main_size, world.region_offset(*h)?.truncate(), size(h)))))
        .collect()
}

/// `region_gaps` for one region `size` wide at `offset` from ours, itself
/// `main` wide, both in the frame `me` is in.
fn region_gap(me: glam::Vec2, main: glam::Vec2, offset: glam::Vec2, size: glam::Vec2) -> f32 {
    // how far apart two spans are along one axis
    let apart = |a: (f32, f32), b: (f32, f32)| (b.0 - a.1).max(a.0 - b.1).max(0.0);
    let far = offset + size;
    let between = glam::Vec2::new(apart((0.0, main.x), (offset.x, far.x)), apart((0.0, main.y), (offset.y, far.y)));
    // half a meter of slack: region origins are whole meters
    if between.max_element() < 0.5 {
        return 0.0;
    }
    glam::Vec2::new(apart((me.x, me.x), (offset.x, far.x)), apart((me.y, me.y), (offset.y, far.y))).length()
}

/// Resolve the complete parent chain, as LLWorld::getAvatars excludes orphaned
/// avatars. A seat-relative position is never a usable region position.
fn native_position<'a>(world: &'a World, mut object: &'a super::objects::Object) -> Option<Vec3> {
    let mut position = object.position;
    for _ in 0..64 {
        if !position.is_finite() {
            return None;
        }
        if object.parent_id == 0 {
            return Some(world.region_offset(object.key.region)? + position);
        }
        object = world.objects.parent_of(object).and_then(|i| world.objects.get(i))?;
        position = object.position + object.rotation * position;
    }
    None
}

/// The avatars around us: the loaded ones first (exact position), then the
/// coarse locations of every region we see, neighbors included. A coarse
/// altitude of 0 or 1020 (the byte 0 or 255) means the server does not know
/// it.
pub fn sightings(world: &World) -> Vec<Sighting> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (_, o) in world.objects.avatars() {
        if o.full_id.is_nil() || o.full_id == world.agent_id || seen.contains(&o.full_id) {
            continue;
        }
        let Some(position) = native_position(world, o) else {
            continue;
        };
        let animations = world.animations_of(&o.full_id);
        let playing = |id: Uuid| animations.iter().any(|a| a.id == id);
        seen.insert(o.full_id);
        out.push(Sighting {
            id: o.full_id,
            region: o.key.region,
            position,
            altitude_known: true,
            loaded: true,
            sitting: o.parent_id != 0 || playing(ANIM_AGENT_SIT_GROUND) || playing(super::body::ANIM_SIT_GROUND_CONSTRAINED),
            typing: world.is_typing_nearby(&o.full_id),
        });
    }
    for (region, list) in &world.coarse {
        let Some(offset) = world.region_offset(*region) else {
            continue;
        };
        for (id, position) in list {
            if id.is_nil() || *id == world.agent_id || !seen.insert(*id) {
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
        radar.sweep(seen, Vec3::ZERO, Some(HERE), &HashMap::new(), 96.0, now)
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
    fn a_teleport_fades_then_drops_the_avatars_left_behind() {
        const FAR: RegionHandle = 3;
        let mut radar = Radar::default();
        let near = HashMap::from([(HERE, 0.0), (THERE, 0.0)]);
        let seen = || vec![at(1, HERE, 30.0), at(2, THERE, 300.0)];
        radar.sweep(seen(), Vec3::ZERO, Some(HERE), &near, 96.0, 0.0);
        // a border crossing: the region we come from still touches ours
        radar.sweep(seen(), Vec3::ZERO, Some(THERE), &near, 96.0, 1.0);
        assert!(radar.entries().all(|e| !e.left_behind));
        // a teleport far away: both regions are out of reach, and still heard
        let gone = HashMap::from([(FAR, 0.0), (HERE, 50_000.0), (THERE, 50_000.0)]);
        let seen = || vec![at(1, HERE, 50_030.0), at(2, THERE, 50_300.0), at(3, FAR, 10.0)];
        radar.sweep(seen(), Vec3::ZERO, Some(FAR), &gone, 96.0, 2.0);
        let left = |radar: &Radar| -> Vec<bool> { (1..=3).map(|n| radar.get(&id(n)).is_some_and(|e| e.left_behind)).collect() };
        assert_eq!(left(&radar), vec![true, true, false]);
        assert_eq!(radar.total, 3);
        radar.sweep(seen(), Vec3::ZERO, Some(FAR), &gone, 96.0, 2.0 + LEFT_BEHIND_SECS - 0.5);
        assert_eq!(left(&radar), vec![true, true, false]);
        // ten seconds after the arrival they leave the list, whatever the
        // old simulators still send
        radar.sweep(seen(), Vec3::ZERO, Some(FAR), &gone, 96.0, 2.0 + LEFT_BEHIND_SECS);
        assert_eq!(radar.entries().map(|e| e.id).collect::<Vec<_>>(), vec![id(3)]);
        assert_eq!(radar.total, 1);
        // a region that comes within reach again is heard again
        let back = HashMap::from([(FAR, 0.0), (HERE, 60.0), (THERE, 50_000.0)]);
        radar.sweep(seen(), Vec3::ZERO, Some(FAR), &back, 96.0, 20.0);
        assert_eq!(radar.get(&id(1)).map(|e| e.left_behind), Some(false));
        assert!(radar.get(&id(2)).is_none());
    }

    #[test]
    fn a_region_arriving_after_the_teleport_is_never_left_behind() {
        let mut radar = Radar::default();
        let gaps = HashMap::from([(HERE, 0.0), (THERE, 700.0)]);
        // the first sweep of the session, then a region out of the draw
        // distance that we start hearing from: nothing was left
        radar.sweep(vec![at(1, HERE, 10.0)], Vec3::ZERO, Some(HERE), &gaps, 96.0, 0.0);
        radar.sweep(
            vec![at(1, HERE, 10.0), at(2, THERE, 700.0)],
            Vec3::ZERO,
            Some(HERE),
            &gaps,
            96.0,
            30.0,
        );
        assert!(radar.entries().all(|e| !e.left_behind));
    }

    #[test]
    fn gap_to_a_region() {
        let v = glam::Vec2::new;
        let (me, size) = (v(200.0, 100.0), v(256.0, 256.0));
        // ours, the ones touching it by a side or a corner
        assert_eq!(region_gap(me, size, v(0.0, 0.0), size), 0.0);
        assert_eq!(region_gap(me, size, v(256.0, 0.0), size), 0.0);
        assert_eq!(region_gap(me, size, v(-256.0, 256.0), size), 0.0);
        // one region between us: from our avatar to its nearest edge
        assert_eq!(region_gap(me, size, v(512.0, 0.0), size), 312.0);
        assert_eq!(region_gap(me, size, v(0.0, -512.0), size), 356.0);
        assert!((region_gap(me, size, v(512.0, 512.0), size) - 312.0_f32.hypot(412.0)).abs() < 0.01);
        // a large region (OpenSim) touches what its far side reaches
        assert_eq!(region_gap(me, v(1024.0, 1024.0), v(1024.0, 768.0), size), 0.0);
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

    #[test]
    fn alerts_to_scripts() {
        let key = |n: u128| id(n).to_string();
        let mut radar = Radar::default();
        // off: nothing, even for an arrival
        sweep(&mut radar, vec![at(1, HERE, 30.0), at(2, THERE, 300.0)], 0.0);
        assert!(radar.script_alerts(false, false).is_empty());
        // entries only: the arrivals in our region
        sweep(
            &mut radar,
            vec![at(1, HERE, 30.0), at(2, THERE, 300.0), at(3, HERE, 50.0), at(4, THERE, 400.0)],
            1.0,
        );
        assert_eq!(radar.script_alerts(true, false), vec![format!("1,1,{}", key(3))]);
        // with the exits: every arrival, then the departures, each list
        // with its own number
        sweep(&mut radar, vec![at(1, HERE, 30.0), at(5, THERE, 500.0)], 2.0);
        let labels = radar.script_alerts(true, true);
        assert_eq!(labels[0], format!("2,1,{}", key(5)));
        assert!(labels[1].starts_with("3,-3,"));
        for gone in [2, 3, 4] {
            assert!(labels[1].contains(&key(gone)));
        }
        // nothing moved: nothing sent
        sweep(&mut radar, vec![at(1, HERE, 30.0), at(5, THERE, 500.0)], 3.0);
        assert!(radar.script_alerts(true, true).is_empty());
    }

    #[test]
    fn a_script_sync_lists_everyone_six_at_a_time() {
        let mut radar = Radar::default();
        let crowd: Vec<Sighting> = (1..=8).map(|n| at(n, HERE, n as f32)).collect();
        sweep(&mut radar, crowd.clone(), 0.0);
        assert!(radar.script_alerts(false, false).is_empty());
        radar.ask_script_sync();
        sweep(&mut radar, crowd.clone(), 1.0);
        let labels = radar.script_alerts(false, false);
        assert_eq!(labels.len(), 2);
        assert!(labels[0].starts_with("1,6,") && labels[1].starts_with("1,2,"));
        assert_eq!(labels.iter().map(|l| l.split(',').count() - 2).sum::<usize>(), 8);
        // one sync per request
        sweep(&mut radar, crowd, 2.0);
        assert!(radar.script_alerts(false, false).is_empty());
    }

    #[test]
    fn the_demo_walker_crosses_every_range_in_and_out() {
        use crate::demo::{RADAR_WALKER, radar_walk};
        let me = Vec3::new(132.0, 126.0, 33.0);
        let mut radar = Radar::default();
        let mut crossings = Vec::new();
        for t in 0..=31 {
            let aurora_net::NetEvent::CoarseLocations { positions, .. } = radar_walk(me, f64::from(t)) else {
                panic!("the walk is a list of coarse locations");
            };
            let seen = positions
                .into_iter()
                .map(|(id, position)| Sighting {
                    id,
                    position,
                    ..at(0, HERE, 0.0)
                })
                .collect();
            let events = radar.sweep(seen, me, Some(HERE), &HashMap::new(), 96.0, f64::from(t));
            crossings.extend(events.into_iter().filter(|e| e.avatar() == RADAR_WALKER).map(|e| match e {
                Event::RegionEnter(..) => "region in",
                Event::DrawEnter(..) => "draw in",
                Event::ChatEnter(..) => "chat in",
                Event::ChatLeave(_) => "chat out",
                Event::DrawLeave(_) => "draw out",
                Event::RegionLeave(_) => "region out",
                Event::AgeAlert(..) => "age",
            }));
        }
        assert_eq!(crossings, ["draw in", "region in", "chat in", "chat out", "draw out", "region out"]);
    }

    #[test]
    fn every_event_has_its_sound() {
        use crate::ui_sound::UiSound as S;
        assert_eq!(Event::RegionEnter(id(1), None).sound(), S::RadarRegionEnter);
        assert_eq!(Event::ChatLeave(id(1)).sound(), S::RadarChatLeave);
        assert_eq!(Event::AgeAlert(id(1), 2).sound(), S::RadarAgeAlert);
    }
}
