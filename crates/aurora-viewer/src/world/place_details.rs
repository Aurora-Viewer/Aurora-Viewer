//! Place details of a place link (FSFloaterPlaceDetails in its
//! "remote_place" mode): what a click on a SLURL opens in Firestorm
//! (LLURLDispatcherImpl::regionHandleCallback with SLURLTeleportDirectly
//! off). Each window resolves its region by name (MapNameRequest, like
//! LLWorldMapMessage::sendNamedRegionRequest), then asks the parcel at the
//! point with the RemoteParcelRequest capability of the agent's region and
//! its details with ParcelInfoRequest (LLPanelPlaceInfo::displayParcelInfo,
//! LLRemoteParcelInfoProcessor; indra/newview/fsfloaterplacedetails.cpp,
//! llpanelplaceinfo.cpp, llremoteparcelrequest.cpp, llurldispatcher.cpp,
//! originally LGPL 2.1). The ParcelInfoReply lands in
//! `World::profiles.parcels`, shared with the picks of the profiles.

use super::worldmap::WorldMap;
use aurora_net::land::RemoteParcelError;
use aurora_net::{NetCommand, RegionHandle};
use glam::{DVec3, Vec3};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a region name may stay unanswered before the window gives up.
/// Firestorm opens its floater only once the name is resolved and shows
/// nothing for an unknown region; Aurora opens at once (loading texts) and
/// needs an end to the wait.
pub const REGION_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a window shows no parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceError {
    /// No region of that name answered MapNameRequest in time.
    UnknownRegion,
    /// RemoteParcelRequest failed (LLPanelPlaceInfo::setErrorStatus).
    Remote(RemoteParcelError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Waiting for the region's grid position (MapNameRequest sent then).
    Region {
        since: Instant,
    },
    /// RemoteParcelRequest sent for (slot handle, position in the region).
    ParcelId {
        handle: RegionHandle,
        position: Vec3,
    },
    /// ParcelInfoRequest sent; the answer is `World::profiles.parcels[id]`.
    Info(Uuid),
    Failed(PlaceError),
}

/// One place details window.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// Stable id of the window (egui id, close / raise).
    pub serial: u64,
    /// Region name as written in the link.
    pub region: String,
    /// Position in the region (meters).
    pub pos: Vec3,
    /// South-west corner of the region, global meters, once known.
    pub origin: Option<(u32, u32)>,
    pub stage: Stage,
}

impl Place {
    /// Global position of the point (FSFloaterPlaceDetails::mGlobalPos:
    /// region origin + SLURL position), once the region is known.
    pub fn global(&self) -> Option<DVec3> {
        self.origin
            .map(|(x, y)| DVec3::new(x as f64 + self.pos.x as f64, y as f64 + self.pos.y as f64, self.pos.z as f64))
    }

    /// The parcel asked with ParcelInfoRequest, if any.
    pub fn parcel(&self) -> Option<Uuid> {
        match self.stage {
            Stage::Info(id) => Some(id),
            _ => None,
        }
    }

    fn same_place(&self, region: &str, pos: Vec3) -> bool {
        self.region.to_lowercase() == region.trim().to_lowercase() && self.pos.round() == pos.round()
    }
}

/// What a RemoteParcelRequest for a point carries
/// (LLRemoteParcelInfoProcessor::regionParcelInfoCoro): the handle of the
/// 256 m slot holding the global point ("leave this to_region_handle at 256
/// grid cell resolution") and the point relative to its region's origin
/// (FS:Beq variable regions: pos_global - region_origin).
pub fn remote_request(origin: (u32, u32), pos: Vec3) -> (RegionHandle, Vec3) {
    let gx = origin.0 as f64 + pos.x as f64;
    let gy = origin.1 as f64 + pos.y as f64;
    let slot = |g: f64| ((g.max(0.0) / 256.0).floor() as u32) * 256;
    (aurora_net::origin_to_handle(slot(gx), slot(gy)), pos)
}

#[derive(Debug, Default)]
pub struct PlaceDetails {
    pub places: Vec<Place>,
    next_serial: u64,
    out: Vec<NetCommand>,
}

impl PlaceDetails {
    /// FSFloaterPlaceDetails::showPlaceDetails with a "remote_place" key:
    /// one window per place (LLFloaterReg instance per key); opening the same
    /// place again reloads it (onOpen: resetLocation, displayParcelInfo).
    /// Returns the window's serial.
    pub fn open(&mut self, map: &WorldMap, region: &str, pos: Vec3, now: Instant) -> u64 {
        let i = match self.places.iter().position(|p| p.same_place(region, pos)) {
            Some(i) => i,
            None => {
                self.next_serial += 1;
                self.places.push(Place {
                    serial: self.next_serial,
                    region: region.trim().to_owned(),
                    pos,
                    origin: None,
                    stage: Stage::Region { since: now },
                });
                self.places.len() - 1
            }
        };
        let place = &mut self.places[i];
        place.origin = None;
        place.stage = Stage::Region { since: now };
        let serial = place.serial;
        if !Self::resolve(place, map, &mut self.out) {
            let name = place.region.to_lowercase();
            if !name.is_empty() {
                self.out.push(NetCommand::MapNameRequest { name });
            }
        }
        serial
    }

    pub fn close(&mut self, serial: u64) {
        self.places.retain(|p| p.serial != serial);
    }

    /// Regions resolved since the last frame start their parcel request;
    /// unanswered names give up after REGION_TIMEOUT.
    pub fn update(&mut self, map: &WorldMap, now: Instant) {
        for place in &mut self.places {
            if let Stage::Region { since } = place.stage
                && !Self::resolve(place, map, &mut self.out)
                && now.duration_since(since) >= REGION_TIMEOUT
            {
                place.stage = Stage::Failed(PlaceError::UnknownRegion);
            }
        }
    }

    /// The region of a waiting window is known: send RemoteParcelRequest.
    fn resolve(place: &mut Place, map: &WorldMap, out: &mut Vec<NetCommand>) -> bool {
        let Some((gx, gy)) = map.region_by_name(&place.region) else {
            return false;
        };
        let origin = (gx * 256, gy * 256);
        let (handle, position) = remote_request(origin, place.pos);
        place.origin = Some(origin);
        place.stage = Stage::ParcelId { handle, position };
        out.push(NetCommand::RemoteParcelRequest { handle, position });
        true
    }

    /// RemoteParcelRequest answer: ParcelInfoRequest for the parcel
    /// (LLPanelPlaceInfo::setParcelID → sendParcelInfoRequest), or the error.
    pub fn apply_remote_parcel(&mut self, handle: RegionHandle, position: Vec3, result: Result<Uuid, RemoteParcelError>) {
        let mut asked = false;
        for place in &mut self.places {
            if place.stage != (Stage::ParcelId { handle, position }) {
                continue;
            }
            match result {
                Ok(id) => {
                    place.stage = Stage::Info(id);
                    // several windows on one parcel: a single request
                    if !asked {
                        self.out.push(NetCommand::ParcelInfoRequest(id));
                        asked = true;
                    }
                }
                Err(e) => place.stage = Stage::Failed(PlaceError::Remote(e)),
            }
        }
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::MapBlock;

    fn map_with(name: &str, x: u16, y: u16) -> WorldMap {
        let mut m = WorldMap::default();
        m.apply_blocks(vec![MapBlock {
            x,
            y,
            name: name.into(),
            access: 13,
            region_flags: 0,
            water_height: 20,
            agents: 0,
            map_image_id: Uuid::nil(),
            size_x: 256,
            size_y: 256,
        }]);
        m
    }

    #[test]
    fn slot_handle_and_local_position() {
        let (h, p) = remote_request((256_000, 256_000), Vec3::new(140.0, 120.0, 25.0));
        assert_eq!(h, aurora_net::origin_to_handle(256_000, 256_000));
        assert_eq!(p, Vec3::new(140.0, 120.0, 25.0));
        // a point of a variable region beyond its first 256 m slot
        let (h, p) = remote_request((256_000, 256_000), Vec3::new(300.0, 520.0, 30.0));
        assert_eq!(h, aurora_net::origin_to_handle(256_256, 256_512));
        assert_eq!(p, Vec3::new(300.0, 520.0, 30.0));
    }

    #[test]
    fn unknown_region_is_asked_then_requested() {
        let now = Instant::now();
        let mut d = PlaceDetails::default();
        let s = d.open(&WorldMap::default(), "Aurora Démo", Vec3::new(140.0, 120.0, 25.0), now);
        assert!(matches!(&d.take_commands()[..], [NetCommand::MapNameRequest { name }] if name == "aurora démo"));
        assert!(d.places[0].global().is_none());
        let map = map_with("Aurora Démo", 1000, 1000);
        d.update(&map, now);
        let place = &d.places[0];
        assert_eq!(place.serial, s);
        assert_eq!(place.global(), Some(DVec3::new(256_140.0, 256_120.0, 25.0)));
        let handle = aurora_net::origin_to_handle(256_000, 256_000);
        let cmds = d.take_commands();
        assert!(matches!(&cmds[..], [NetCommand::RemoteParcelRequest { handle: h, .. }] if *h == handle));
        let parcel = Uuid::from_u128(0x9A2C);
        d.apply_remote_parcel(handle, Vec3::new(140.0, 120.0, 25.0), Ok(parcel));
        assert_eq!(d.places[0].parcel(), Some(parcel));
        assert!(matches!(&d.take_commands()[..], [NetCommand::ParcelInfoRequest(id)] if *id == parcel));
    }

    #[test]
    fn known_region_requests_at_once_and_reopening_reloads() {
        let now = Instant::now();
        let map = map_with("Lagune Boréale", 1001, 1000);
        let mut d = PlaceDetails::default();
        let pos = Vec3::new(60.0, 200.0, 22.0);
        let a = d.open(&map, "lagune boréale", pos, now);
        assert!(matches!(&d.take_commands()[..], [NetCommand::RemoteParcelRequest { .. }]));
        let b = d.open(&map, "Lagune Boréale", pos + Vec3::splat(0.2), now);
        assert_eq!(a, b, "same place, same window");
        assert_eq!(d.places.len(), 1);
        assert!(matches!(&d.take_commands()[..], [NetCommand::RemoteParcelRequest { .. }]));
        let c = d.open(&map, "Lagune Boréale", Vec3::new(10.0, 10.0, 10.0), now);
        assert_ne!(a, c);
        d.close(a);
        assert_eq!(d.places.len(), 1);
    }

    #[test]
    fn errors_end_the_wait() {
        let now = Instant::now();
        let mut d = PlaceDetails::default();
        d.open(&WorldMap::default(), "Nulle Part", Vec3::new(1.0, 2.0, 3.0), now);
        d.update(&WorldMap::default(), now + Duration::from_secs(2));
        assert!(matches!(d.places[0].stage, Stage::Region { .. }));
        d.update(&WorldMap::default(), now + REGION_TIMEOUT);
        assert_eq!(d.places[0].stage, Stage::Failed(PlaceError::UnknownRegion));

        let map = map_with("Faille", 1001, 1001);
        let mut d = PlaceDetails::default();
        d.open(&map, "Faille", Vec3::new(128.0, 128.0, 30.0), now);
        let Some(NetCommand::RemoteParcelRequest { handle, position }) = d.take_commands().pop() else {
            panic!("no request");
        };
        // an answer for another point changes nothing
        d.apply_remote_parcel(handle, Vec3::ZERO, Err(RemoteParcelError::Status(404)));
        assert!(matches!(d.places[0].stage, Stage::ParcelId { .. }));
        d.apply_remote_parcel(handle, position, Err(RemoteParcelError::Status(404)));
        assert_eq!(d.places[0].stage, Stage::Failed(PlaceError::Remote(RemoteParcelError::Status(404))));
        assert!(d.take_commands().is_empty());
    }
}
