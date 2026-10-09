//! Place profiles: what the Places window (LLPanelPlaces) shows for a place
//! link, a teleport history entry or a landmark, and the standalone
//! "Détails de l'emplacement" windows (FSFloaterPlaceDetails, used instead
//! when FSUseStandalonePlaceDetailsFloater is on). A place link resolves its
//! region by name (MapNameRequest, like LLWorldMapMessage::
//! sendNamedRegionRequest), a landmark its asset and region handle
//! (`landmarks.rs`); then the parcel at the point is asked with the
//! RemoteParcelRequest capability of the agent's region and its details
//! with ParcelInfoRequest (LLPanelPlaceInfo::displayParcelInfo,
//! LLRemoteParcelInfoProcessor; indra/newview/llpanelplaces.cpp,
//! fsfloaterplacedetails.cpp, llpanelplaceinfo.cpp, llremoteparcelrequest.cpp,
//! llurldispatcher.cpp, originally LGPL 2.1). The ParcelInfoReply lands in
//! `World::profiles.parcels`, shared with the picks of the profiles.

use super::landmarks::{Landmarks, Resolved};
use super::worldmap::WorldMap;
use aurora_net::land::RemoteParcelError;
use aurora_net::{NetCommand, RegionHandle};
use glam::{DVec3, Vec3};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a region name may stay unanswered before the profile gives up.
/// Firestorm opens nothing until the name is resolved and nothing at all for
/// an unknown region; Aurora shows the profile at once (loading texts) and
/// needs an end to the wait.
pub const REGION_TIMEOUT: Duration = Duration::from_secs(10);

/// What a profile is about (the "key" of LLPanelPlaces::onOpen).
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// "remote_place": a place link, region by name and position in it.
    Link { region: String, pos: Vec3 },
    /// "teleport_history": an entry of the saved history.
    History { title: String, global: DVec3 },
    /// "landmark": an inventory landmark (item, asset).
    Landmark { item: Uuid, asset: Uuid },
}

impl Source {
    fn same(&self, other: &Source) -> bool {
        match (self, other) {
            (Source::Link { region: a, pos: p }, Source::Link { region: b, pos: q }) => {
                a.trim().to_lowercase() == b.trim().to_lowercase() && p.round() == q.round()
            }
            (Source::Landmark { item: a, .. }, Source::Landmark { item: b, .. }) => a == b,
            (a, b) => a == b,
        }
    }
}

/// Why a profile shows no parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceError {
    /// No region of that name answered MapNameRequest in time.
    UnknownRegion,
    /// The landmark asset could not be loaded or read.
    LandmarkUnavailable,
    /// RemoteParcelRequest failed (LLPanelPlaceInfo::setErrorStatus).
    Remote(RemoteParcelError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// A place link waiting for its region's grid position (MapNameRequest
    /// sent then).
    Region {
        since: Instant,
    },
    /// A landmark waiting for its asset or its region's handle.
    Landmark,
    /// RemoteParcelRequest sent for (slot handle, position in the region).
    ParcelId {
        handle: RegionHandle,
        position: Vec3,
    },
    /// ParcelInfoRequest sent; the answer is `World::profiles.parcels[id]`.
    Info(Uuid),
    Failed(PlaceError),
}

/// One place profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// Stable id (egui ids of the standalone windows, raise / close).
    pub serial: u64,
    pub source: Source,
    /// mPosGlobal: where Téléporter / Carte go, once known.
    pub global: Option<DVec3>,
    /// mPosRegion: the position shown after the region name.
    pub region_pos: Vec3,
    pub stage: Stage,
}

impl Place {
    /// The parcel asked with ParcelInfoRequest, if any.
    pub fn parcel(&self) -> Option<Uuid> {
        match self.stage {
            Stage::Info(id) => Some(id),
            _ => None,
        }
    }

    /// Region name to build a SLURL from when the parcel did not say it.
    pub fn region_hint(&self) -> &str {
        match &self.source {
            Source::Link { region, .. } => region,
            _ => "",
        }
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
    (slot_handle(gx, gy), pos)
}

fn slot_handle(gx: f64, gy: f64) -> RegionHandle {
    let slot = |g: f64| ((g.max(0.0) / 256.0).floor() as u32) * 256;
    aurora_net::origin_to_handle(slot(gx), slot(gy))
}

/// LLPanelPlaceInfo::displayParcelInfo(region_id, pos_global): the position
/// in the region is the global one modulo the region width.
fn region_pos_of(global: DVec3) -> Vec3 {
    Vec3::new(
        global.x.rem_euclid(256.0) as f32,
        global.y.rem_euclid(256.0) as f32,
        global.z as f32,
    )
}

#[derive(Debug, Default)]
pub struct PlaceDetails {
    /// Standalone windows (FSFloaterPlaceDetails, one per place).
    pub windows: Vec<Place>,
    /// The profile shown in the Places window, if any.
    pub panel: Option<Place>,
    next_serial: u64,
    out: Vec<NetCommand>,
}

impl PlaceDetails {
    /// FSFloaterPlaceDetails::showPlaceDetails: one window per place
    /// (LLFloaterReg instance per key); opening the same place again reloads
    /// it (onOpen: resetLocation, displayParcelInfo). Returns its serial.
    pub fn open_window(&mut self, source: Source, map: &WorldMap, landmarks: &mut Landmarks, now: Instant) -> u64 {
        let i = match self.windows.iter().position(|p| p.source.same(&source)) {
            Some(i) => i,
            None => {
                let place = self.new_place(source);
                self.windows.push(place);
                self.windows.len() - 1
            }
        };
        let serial = self.windows[i].serial;
        Self::start(&mut self.windows[i], map, landmarks, now, &mut self.out);
        serial
    }

    /// LLPanelPlaces::onOpen with a place key: the Places window's profile.
    pub fn open_panel(&mut self, source: Source, map: &WorldMap, landmarks: &mut Landmarks, now: Instant) {
        let mut place = self.new_place(source);
        Self::start(&mut place, map, landmarks, now, &mut self.out);
        self.panel = Some(place);
    }

    fn new_place(&mut self, source: Source) -> Place {
        self.next_serial += 1;
        Place {
            serial: self.next_serial,
            source,
            global: None,
            region_pos: Vec3::ZERO,
            stage: Stage::Landmark,
        }
    }

    pub fn close_window(&mut self, serial: u64) {
        self.windows.retain(|p| p.serial != serial);
    }

    /// LLPanelPlaces::onBackButtonClicked.
    pub fn close_panel(&mut self) {
        self.panel = None;
    }

    fn all(&mut self) -> impl Iterator<Item = &mut Place> {
        self.windows.iter_mut().chain(self.panel.iter_mut())
    }

    /// resetLocation, then what each source needs first.
    fn start(place: &mut Place, map: &WorldMap, landmarks: &mut Landmarks, now: Instant, out: &mut Vec<NetCommand>) {
        place.global = None;
        place.region_pos = Vec3::ZERO;
        match place.source.clone() {
            Source::Link { region, pos } => {
                place.stage = Stage::Region { since: now };
                place.region_pos = pos;
                if !Self::resolve_link(place, map, out) {
                    let name = region.trim().to_lowercase();
                    if !name.is_empty() {
                        out.push(NetCommand::MapNameRequest { name });
                    }
                }
            }
            Source::History { global, .. } => Self::request_at(place, global, Uuid::nil(), out),
            Source::Landmark { asset, .. } => {
                place.stage = Stage::Landmark;
                Self::resolve_landmark(place, asset, landmarks, out);
            }
        }
    }

    /// RemoteParcelRequest for a known global position (history entries and
    /// landmarks, displayParcelInfo without a region handle).
    fn request_at(place: &mut Place, global: DVec3, region_id: Uuid, out: &mut Vec<NetCommand>) {
        let handle = slot_handle(global.x, global.y);
        let position = region_pos_of(global);
        place.global = Some(global);
        place.region_pos = position;
        place.stage = Stage::ParcelId { handle, position };
        out.push(NetCommand::RemoteParcelRequest {
            handle,
            position,
            region_id,
        });
    }

    /// The region of a waiting place link is known: send RemoteParcelRequest.
    fn resolve_link(place: &mut Place, map: &WorldMap, out: &mut Vec<NetCommand>) -> bool {
        let Source::Link { region, pos } = &place.source else {
            return false;
        };
        let Some((gx, gy)) = map.region_by_name(region) else {
            return false;
        };
        let origin = (gx * 256, gy * 256);
        let (handle, position) = remote_request(origin, *pos);
        place.global = Some(DVec3::new(
            origin.0 as f64 + pos.x as f64,
            origin.1 as f64 + pos.y as f64,
            pos.z as f64,
        ));
        place.stage = Stage::ParcelId { handle, position };
        out.push(NetCommand::RemoteParcelRequest {
            handle,
            position,
            region_id: Uuid::nil(),
        });
        true
    }

    /// LLPanelPlaces::onLandmarkLoaded once the asset and its region are known.
    fn resolve_landmark(place: &mut Place, asset: Uuid, landmarks: &mut Landmarks, out: &mut Vec<NetCommand>) {
        match landmarks.resolve(asset) {
            Resolved::Pending => {}
            Resolved::Failed => place.stage = Stage::Failed(PlaceError::LandmarkUnavailable),
            Resolved::Ready { region_id, global, .. } => Self::request_at(place, global, region_id, out),
        }
    }

    /// Regions and landmarks resolved since the last frame start their parcel
    /// request; unanswered names give up after REGION_TIMEOUT.
    pub fn update(&mut self, map: &WorldMap, landmarks: &mut Landmarks, now: Instant) {
        let mut out = std::mem::take(&mut self.out);
        for place in self.all() {
            match (&place.stage, place.source.clone()) {
                (Stage::Region { since }, _) => {
                    let since = *since;
                    if !Self::resolve_link(place, map, &mut out) && now.duration_since(since) >= REGION_TIMEOUT {
                        place.stage = Stage::Failed(PlaceError::UnknownRegion);
                    }
                }
                (Stage::Landmark, Source::Landmark { asset, .. }) => Self::resolve_landmark(place, asset, landmarks, &mut out),
                _ => {}
            }
        }
        self.out = out;
    }

    /// RemoteParcelRequest answer: ParcelInfoRequest for the parcel
    /// (LLPanelPlaceInfo::setParcelID → sendParcelInfoRequest), or the error.
    pub fn apply_remote_parcel(&mut self, handle: RegionHandle, position: Vec3, result: Result<Uuid, RemoteParcelError>) {
        let mut asked = false;
        let mut out = std::mem::take(&mut self.out);
        for place in self.all() {
            if place.stage != (Stage::ParcelId { handle, position }) {
                continue;
            }
            match result {
                Ok(id) => {
                    place.stage = Stage::Info(id);
                    // several profiles of one parcel: a single request
                    if !asked {
                        out.push(NetCommand::ParcelInfoRequest(id));
                        asked = true;
                    }
                }
                Err(e) => place.stage = Stage::Failed(PlaceError::Remote(e)),
            }
        }
        self.out = out;
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

    fn link(region: &str, pos: Vec3) -> Source {
        Source::Link {
            region: region.into(),
            pos,
        }
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
        let mut lm = Landmarks::default();
        let mut d = PlaceDetails::default();
        let s = d.open_window(
            link("Aurora Démo", Vec3::new(140.0, 120.0, 25.0)),
            &WorldMap::default(),
            &mut lm,
            now,
        );
        assert!(matches!(&d.take_commands()[..], [NetCommand::MapNameRequest { name }] if name == "aurora démo"));
        assert!(d.windows[0].global.is_none());
        let map = map_with("Aurora Démo", 1000, 1000);
        d.update(&map, &mut lm, now);
        let place = &d.windows[0];
        assert_eq!(place.serial, s);
        assert_eq!(place.global, Some(DVec3::new(256_140.0, 256_120.0, 25.0)));
        let handle = aurora_net::origin_to_handle(256_000, 256_000);
        let cmds = d.take_commands();
        assert!(matches!(&cmds[..], [NetCommand::RemoteParcelRequest { handle: h, region_id, .. }] if *h == handle && region_id.is_nil()));
        let parcel = Uuid::from_u128(0x9A2C);
        d.apply_remote_parcel(handle, Vec3::new(140.0, 120.0, 25.0), Ok(parcel));
        assert_eq!(d.windows[0].parcel(), Some(parcel));
        assert!(matches!(&d.take_commands()[..], [NetCommand::ParcelInfoRequest(id)] if *id == parcel));
    }

    #[test]
    fn known_region_requests_at_once_and_reopening_reloads() {
        let now = Instant::now();
        let mut lm = Landmarks::default();
        let map = map_with("Lagune Boréale", 1001, 1000);
        let mut d = PlaceDetails::default();
        let pos = Vec3::new(60.0, 200.0, 22.0);
        let a = d.open_window(link("lagune boréale", pos), &map, &mut lm, now);
        assert!(matches!(&d.take_commands()[..], [NetCommand::RemoteParcelRequest { .. }]));
        let b = d.open_window(link("Lagune Boréale", pos + Vec3::splat(0.2)), &map, &mut lm, now);
        assert_eq!(a, b, "same place, same window");
        assert_eq!(d.windows.len(), 1);
        assert!(matches!(&d.take_commands()[..], [NetCommand::RemoteParcelRequest { .. }]));
        let c = d.open_window(link("Lagune Boréale", Vec3::new(10.0, 10.0, 10.0)), &map, &mut lm, now);
        assert_ne!(a, c);
        d.close_window(a);
        assert_eq!(d.windows.len(), 1);
    }

    #[test]
    fn errors_end_the_wait() {
        let now = Instant::now();
        let mut lm = Landmarks::default();
        let mut d = PlaceDetails::default();
        d.open_panel(link("Nulle Part", Vec3::new(1.0, 2.0, 3.0)), &WorldMap::default(), &mut lm, now);
        d.update(&WorldMap::default(), &mut lm, now + Duration::from_secs(2));
        assert!(matches!(d.panel.as_ref().map(|p| &p.stage), Some(Stage::Region { .. })));
        d.update(&WorldMap::default(), &mut lm, now + REGION_TIMEOUT);
        assert_eq!(
            d.panel.as_ref().map(|p| p.stage.clone()),
            Some(Stage::Failed(PlaceError::UnknownRegion))
        );

        let map = map_with("Faille", 1001, 1001);
        let mut d = PlaceDetails::default();
        d.open_window(link("Faille", Vec3::new(128.0, 128.0, 30.0)), &map, &mut lm, now);
        let Some(NetCommand::RemoteParcelRequest { handle, position, .. }) = d.take_commands().pop() else {
            panic!("no request");
        };
        // an answer for another point changes nothing
        d.apply_remote_parcel(handle, Vec3::ZERO, Err(RemoteParcelError::Status(404)));
        assert!(matches!(d.windows[0].stage, Stage::ParcelId { .. }));
        d.apply_remote_parcel(handle, position, Err(RemoteParcelError::Status(404)));
        assert_eq!(
            d.windows[0].stage,
            Stage::Failed(PlaceError::Remote(RemoteParcelError::Status(404)))
        );
        assert!(d.take_commands().is_empty());
    }

    #[test]
    fn history_entry_asks_at_its_global_position() {
        let mut lm = Landmarks::default();
        let mut d = PlaceDetails::default();
        let global = DVec3::new(256_316.4, 256_200.0, 22.0);
        let source = Source::History {
            title: "Lagune des aurores, Lagune Boréale".into(),
            global,
        };
        d.open_panel(source, &WorldMap::default(), &mut lm, Instant::now());
        let p = d.panel.as_ref().expect("profile");
        assert_eq!(p.global, Some(global));
        assert!((p.region_pos.x - 60.4).abs() < 1e-3);
        let cmds = d.take_commands();
        let handle = aurora_net::origin_to_handle(256_256, 256_000);
        assert!(matches!(&cmds[..], [NetCommand::RemoteParcelRequest { handle: h, .. }] if *h == handle));
    }

    #[test]
    fn landmark_waits_for_its_asset_and_region() {
        let region = Uuid::from_u128(0x5E61);
        let asset = Uuid::from_u128(0x1A4D);
        let mut lm = Landmarks::default();
        let mut d = PlaceDetails::default();
        let map = WorldMap::default();
        let now = Instant::now();
        d.open_panel(
            Source::Landmark {
                item: Uuid::from_u128(1),
                asset,
            },
            &map,
            &mut lm,
            now,
        );
        assert!(d.take_commands().is_empty());
        assert_eq!(lm.take_wanted().len(), 1);
        lm.on_data(
            asset,
            Some(format!("Landmark version 2\nregion_id {region}\nlocal_pos 148 195 24\n").as_bytes()),
        );
        d.update(&map, &mut lm, now);
        assert!(matches!(&lm.take_commands()[..], [NetCommand::RegionHandleRequest(r)] if *r == region));
        lm.on_region_handle(region, aurora_net::origin_to_handle(256_512, 256_000));
        d.update(&map, &mut lm, now);
        let cmds = d.take_commands();
        assert!(matches!(&cmds[..], [NetCommand::RemoteParcelRequest { region_id, .. }] if *region_id == region));
        assert_eq!(
            d.panel.as_ref().and_then(|p| p.global),
            Some(DVec3::new(256_660.0, 256_195.0, 24.0))
        );

        let broken = Uuid::from_u128(0xBAD);
        d.open_panel(
            Source::Landmark {
                item: Uuid::from_u128(2),
                asset: broken,
            },
            &map,
            &mut lm,
            now,
        );
        lm.on_data(broken, None);
        d.update(&map, &mut lm, now);
        assert_eq!(
            d.panel.as_ref().map(|p| p.stage.clone()),
            Some(Stage::Failed(PlaceError::LandmarkUnavailable))
        );
    }
}
