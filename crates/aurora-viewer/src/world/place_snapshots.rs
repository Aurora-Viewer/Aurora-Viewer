//! The snapshots of the parcels the landmarks and the teleport history point
//! at, for the bars' favourites and the rows of the Places window drawn as
//! pictures while their item has no thumbnail (most landmarks: item
//! thumbnails are recent; a history entry has none). The Legacy Viewer's
//! favourites bar and history list show no picture; its place profile reads
//! the parcel snapshot with the same two requests as `place_details.rs`:
//! RemoteParcelRequest for the parcel at the point, then ParcelInfoRequest
//! for its details (LLPanelPlaceInfo::displayParcelInfo,
//! LLRemoteParcelInfoProcessor; indra/newview/llpanelplaceinfo.cpp,
//! llremoteparcelrequest.cpp, originally LGPL 2.1). The ParcelInfoReply lands
//! in `World::profiles.parcels`. Each spot is asked once a session, a few at
//! a time.

use super::place_details::remote_request_at;
use aurora_net::land::RemoteParcelError;
use aurora_net::{NetCommand, ParcelSummary, RegionHandle};
use glam::{DVec3, Vec3};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// RemoteParcelRequests in flight at once: a long Favorites folder opened
/// in the menu does not post one capability request per landmark at once.
const IN_FLIGHT: usize = 4;
/// The agent's region had no RemoteParcelRequest capability (before its
/// seed answered, during a teleport, or a grid without it): the spots are
/// asked again after this pause, not every frame.
const RETRY: Duration = Duration::from_secs(10);

/// What a snapshot is kept for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Spot {
    /// A landmark, by its asset: its copies share one picture.
    Landmark(Uuid),
    /// A point of the world, to the meter: an entry of the teleport history
    /// by where it arrived, not by its index, which moves when an entry is
    /// removed or merged (#798).
    Point(i64, i64, i64),
}

impl Spot {
    pub fn at(global: DVec3) -> Spot {
        let g = global.round();
        Spot::Point(g.x as i64, g.y as i64, g.z as i64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// Waiting for its turn: global point, region id (nil when unknown).
    Queued(DVec3, Uuid),
    /// RemoteParcelRequest sent for (slot handle, position in the region).
    ParcelId(RegionHandle, Vec3),
    /// The parcel; its snapshot comes with its ParcelInfoReply.
    Parcel(Uuid),
    /// No parcel there, or the request failed: the pin stays.
    Failed,
}

#[derive(Debug, Default)]
pub struct PlaceSnapshots {
    spots: HashMap<Spot, Stage>,
    queue: VecDeque<Spot>,
    /// Parcels asked with ParcelInfoRequest this session.
    asked: HashSet<Uuid>,
    /// No capability to ask with: nothing new is asked before then.
    paused_until: Option<Instant>,
    out: Vec<NetCommand>,
}

impl PlaceSnapshots {
    /// The spot is at `global` (region `region_id`, nil when unknown) and
    /// on screen: ask its parcel's snapshot once.
    pub fn want(&mut self, spot: Spot, global: DVec3, region_id: Uuid, now: Instant) {
        if spot == Spot::Landmark(Uuid::nil()) || self.spots.contains_key(&spot) || self.paused_until.is_some_and(|t| now < t) {
            return;
        }
        self.spots.insert(spot, Stage::Queued(global, region_id));
        self.queue.push_back(spot);
        self.pump();
    }

    /// The snapshot of the spot's parcel, once its details are known.
    pub fn snapshot(&self, spot: Spot, parcels: &HashMap<Uuid, ParcelSummary>) -> Option<Uuid> {
        match self.spots.get(&spot) {
            Some(Stage::Parcel(id)) => parcels.get(id).map(|p| p.snapshot).filter(|s| !s.is_nil()),
            _ => None,
        }
    }

    /// RemoteParcelRequest answer (the same handle and position): the
    /// parcel's details are asked, unless they are known or asked already.
    pub fn apply_remote_parcel(
        &mut self,
        handle: RegionHandle,
        position: Vec3,
        result: Result<Uuid, RemoteParcelError>,
        parcels: &HashMap<Uuid, ParcelSummary>,
    ) {
        if result == Err(RemoteParcelError::NoCapability) {
            // forgotten with the queue: asked again once the pause is over
            let answered = Stage::ParcelId(handle, position);
            self.spots.retain(|_, s| *s != answered && !matches!(s, Stage::Queued(..)));
            self.queue.clear();
            self.paused_until = Some(Instant::now() + RETRY);
            return;
        }
        let mut answered = false;
        for stage in self.spots.values_mut() {
            if *stage != Stage::ParcelId(handle, position) {
                continue;
            }
            answered = true;
            *stage = match result {
                Ok(id) => {
                    if !parcels.contains_key(&id) && self.asked.insert(id) {
                        self.out.push(NetCommand::ParcelInfoRequest(id));
                    }
                    Stage::Parcel(id)
                }
                Err(_) => Stage::Failed,
            };
        }
        if answered {
            self.pump();
        }
    }

    /// The next queued spots while fewer than IN_FLIGHT wait.
    fn pump(&mut self) {
        let mut flying = self.spots.values().filter(|s| matches!(s, Stage::ParcelId(..))).count();
        while flying < IN_FLIGHT {
            let Some(spot) = self.queue.pop_front() else {
                break;
            };
            let Some(stage) = self.spots.get_mut(&spot) else {
                continue;
            };
            let Stage::Queued(global, region_id) = *stage else {
                continue;
            };
            let (handle, position) = remote_request_at(global);
            *stage = Stage::ParcelId(handle, position);
            self.out.push(NetCommand::RemoteParcelRequest {
                handle,
                position,
                region_id,
            });
            flying += 1;
        }
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: Uuid, snapshot: Uuid) -> ParcelSummary {
        ParcelSummary {
            id,
            snapshot,
            ..Default::default()
        }
    }

    fn lm(n: u128) -> Spot {
        Spot::Landmark(Uuid::from_u128(n))
    }

    fn global(n: u32) -> DVec3 {
        DVec3::new(256_000.0 + n as f64 * 256.0 + 40.0, 256_000.0 + 60.0, 22.0)
    }

    fn sent(s: &mut PlaceSnapshots) -> Vec<(RegionHandle, Vec3)> {
        s.take_commands()
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::RemoteParcelRequest { handle, position, .. } => Some((handle, position)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_landmark_gets_its_parcel_snapshot() {
        let mut s = PlaceSnapshots::default();
        let (asset, parcel, snap) = (lm(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let mut parcels = HashMap::new();
        s.want(asset, global(0), Uuid::nil(), Instant::now());
        let [(handle, position)] = sent(&mut s)[..] else {
            panic!("one RemoteParcelRequest");
        };
        assert_eq!(position, Vec3::new(40.0, 60.0, 22.0));
        // asked once, however many frames show it
        s.want(asset, global(0), Uuid::nil(), Instant::now());
        assert!(s.take_commands().is_empty());
        s.apply_remote_parcel(handle, position, Ok(parcel), &parcels);
        assert!(matches!(&s.take_commands()[..], [NetCommand::ParcelInfoRequest(id)] if *id == parcel));
        assert_eq!(s.snapshot(asset, &parcels), None);
        parcels.insert(parcel, summary(parcel, snap));
        assert_eq!(s.snapshot(asset, &parcels), Some(snap));
    }

    #[test]
    fn a_known_or_asked_parcel_is_not_asked_again() {
        let mut s = PlaceSnapshots::default();
        let parcel = Uuid::from_u128(9);
        let parcels = HashMap::from([(parcel, summary(parcel, Uuid::from_u128(10)))]);
        // two landmarks of the same point: one request, both answered
        s.want(lm(1), global(0), Uuid::nil(), Instant::now());
        s.want(lm(2), global(0), Uuid::nil(), Instant::now());
        let asked = sent(&mut s);
        assert_eq!(asked.len(), 2);
        s.apply_remote_parcel(asked[0].0, asked[0].1, Ok(parcel), &parcels);
        assert!(s.take_commands().is_empty(), "details already known");
        assert_eq!(s.snapshot(lm(2), &parcels), Some(Uuid::from_u128(10)));
    }

    #[test]
    fn a_failed_point_or_a_parcel_without_snapshot_keeps_the_pin() {
        let mut s = PlaceSnapshots::default();
        let mut parcels = HashMap::new();
        s.want(lm(1), global(0), Uuid::nil(), Instant::now());
        s.want(lm(2), global(1), Uuid::nil(), Instant::now());
        let asked = sent(&mut s);
        s.apply_remote_parcel(asked[0].0, asked[0].1, Err(RemoteParcelError::Status(404)), &parcels);
        s.apply_remote_parcel(asked[1].0, asked[1].1, Ok(Uuid::from_u128(5)), &parcels);
        parcels.insert(Uuid::from_u128(5), summary(Uuid::from_u128(5), Uuid::nil()));
        assert_eq!(s.snapshot(lm(1), &parcels), None);
        assert_eq!(s.snapshot(lm(2), &parcels), None);
        // failed for the session: not asked again
        s.want(lm(1), global(0), Uuid::nil(), Instant::now());
        assert!(sent(&mut s).is_empty());
    }

    #[test]
    fn history_points_are_their_own_spots() {
        let mut s = PlaceSnapshots::default();
        let (parcel, snap) = (Uuid::from_u128(7), Uuid::from_u128(8));
        let parcels = HashMap::from([(parcel, summary(parcel, snap))]);
        // the same arrival read back from the file, a few centimetres off
        let at = global(2) + DVec3::new(0.2, -0.3, 0.1);
        assert_eq!(Spot::at(at), Spot::at(global(2)));
        s.want(Spot::at(at), at, Uuid::nil(), Instant::now());
        s.want(Spot::at(global(2)), global(2), Uuid::nil(), Instant::now());
        let [(handle, position)] = sent(&mut s)[..] else {
            panic!("one RemoteParcelRequest");
        };
        s.apply_remote_parcel(handle, position, Ok(parcel), &parcels);
        assert_eq!(s.snapshot(Spot::at(global(2)), &parcels), Some(snap));
        // another point, or a landmark there, is not that entry
        assert_eq!(s.snapshot(Spot::at(global(3)), &parcels), None);
        assert_eq!(s.snapshot(lm(2), &parcels), None);
    }

    #[test]
    fn a_long_folder_is_asked_a_few_at_a_time() {
        let mut s = PlaceSnapshots::default();
        let parcels = HashMap::new();
        for n in 0..10 {
            s.want(lm(100 + n), global(n as u32), Uuid::nil(), Instant::now());
        }
        let mut flying: VecDeque<_> = sent(&mut s).into();
        assert_eq!(flying.len(), IN_FLIGHT);
        let mut total = flying.len();
        // each answer lets the next one go
        while let Some((handle, position)) = flying.pop_front() {
            s.apply_remote_parcel(handle, position, Err(RemoteParcelError::NoParcel), &parcels);
            let next = sent(&mut s);
            assert!(next.len() <= 1);
            total += next.len();
            flying.extend(next);
        }
        assert_eq!(total, 10);
    }

    #[test]
    fn no_capability_pauses_then_asks_again() {
        let mut s = PlaceSnapshots::default();
        let parcels = HashMap::new();
        let now = Instant::now();
        for n in 0..6 {
            s.want(lm(1 + n), global(n as u32), Uuid::nil(), now);
        }
        let asked = sent(&mut s);
        assert_eq!(asked.len(), IN_FLIGHT);
        s.apply_remote_parcel(asked[0].0, asked[0].1, Err(RemoteParcelError::NoCapability), &parcels);
        // the queue waits with the pause: nothing sent, nothing new taken
        assert!(sent(&mut s).is_empty());
        s.want(lm(1), global(0), Uuid::nil(), Instant::now());
        assert!(sent(&mut s).is_empty());
        // after it, the forgotten landmarks are asked again
        s.want(lm(1), global(0), Uuid::nil(), Instant::now() + RETRY);
        assert_eq!(sent(&mut s), vec![asked[0]]);
    }
}
