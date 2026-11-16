//! The pathfinding requests of a region: the linkset and character lists,
//! linkset edits, the navmesh status, the agent's right to rebake and the
//! rebake itself. Port of LLPathfindingManager (requestGetLinksets,
//! requestSetLinksets, requestGetCharacters, requestGetNavMeshForRegion with
//! the status only, requestGetAgentState, requestRebakeNavMesh;
//! indra/newview/llpathfindingmanager.cpp, originally LGPL 2.1).
//!
//! Difference: the Legacy Viewer calls a region "pathfinding enabled" when it
//! gives RetrieveNavMeshSrc (isPathfindingEnabledForRegion). Aurora never
//! asks for that Havok blob, so NavMeshGenerationStatus, which the same
//! regions give, stands for it.
use super::Session;
use crate::RegionHandle;
use crate::caps;
use crate::pathfinding::linkset::{self, Edit};
use crate::pathfinding::{self, Error, Kind, Row, navmesh};
use crate::types::NetEvent;
use aurora_llsd::Llsd;
use std::time::Duration;
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(20);

/// A GET, tried a second time when it never reached the region (a pooled
/// connection the region had closed fails at once with "error sending
/// request"): a GET changes nothing, so the second try is safe. The Legacy
/// Viewer shows the error at once; retrying spares the user a Refresh.
async fn get(http: &reqwest::Client, url: &str, what: &str) -> Result<Llsd, Error> {
    for attempt in 1..=2 {
        match tokio::time::timeout(TIMEOUT, caps::get_llsd(http, url)).await {
            Ok(Ok(data)) => return Ok(data),
            Ok(Err(caps::CapsError::Http(e))) if attempt == 1 => {
                log::info!("pathfinding: {what} not sent ({e}), trying again");
            }
            Ok(Err(e)) => {
                log::warn!("pathfinding: {what} failed: {e}");
                return Err(Error::Failed);
            }
            Err(_) => {
                log::warn!("pathfinding: {what}: no reply in {} s", TIMEOUT.as_secs());
                return Err(Error::Failed);
            }
        }
    }
    Err(Error::Failed)
}

/// The PUT of linksetObjectsCoro / linksetTerrainCoro, or the POST of
/// navMeshRebakeCoro (never sent twice).
async fn send(http: &reqwest::Client, url: &str, body: &Llsd, put: bool, what: &str) -> Result<Llsd, Error> {
    match tokio::time::timeout(TIMEOUT, caps::request_llsd(http, url, body, put)).await {
        Ok(Ok(data)) => Ok(data),
        Ok(Err(e)) => {
            log::warn!("pathfinding: {what} failed: {e}");
            Err(Error::Failed)
        }
        Err(_) => {
            log::warn!("pathfinding: {what}: no reply in {} s", TIMEOUT.as_secs());
            Err(Error::Failed)
        }
    }
}

/// LinksetsResponder::sendCallback: an error as soon as one of the two
/// requests failed; the terrain row, when there is one, comes first.
fn combine(objects: Option<Result<Vec<Row>, Error>>, terrain: Option<Result<Row, Error>>) -> Result<Vec<Row>, Error> {
    let mut rows = Vec::new();
    if let Some(terrain) = terrain {
        rows.push(terrain?);
    }
    if let Some(objects) = objects {
        rows.extend(objects?);
    }
    Ok(rows)
}

impl Session<'_> {
    fn region_cap(&self, region: RegionHandle, name: &str) -> Option<String> {
        self.sims
            .values()
            .find(|sim| sim.handle == region)
            .and_then(|sim| sim.caps.get(name))
            .cloned()
    }

    /// requestGetLinksets / requestGetCharacters. The linksets need both
    /// RegionObjects and TerrainNavMeshProperties, as in the Legacy Viewer;
    /// the terrain is read only when `terrain` (a god or an estate manager).
    pub(super) fn fetch_pathfinding(&mut self, request: Uuid, region: RegionHandle, kind: Kind, terrain: bool) {
        let url = self.region_cap(region, kind.capability());
        let terrain_url = self.region_cap(region, "TerrainNavMeshProperties");
        let terrain_needed = kind == Kind::Objects;
        let events = self.sh.events.clone();
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let result = match (url, terrain_url) {
                (Some(url), _) if !terrain_needed => get(&http, &url, kind.capability())
                    .await
                    .and_then(|data| pathfinding::parse(&data, kind)),
                (Some(url), Some(terrain_url)) => {
                    let objects = async {
                        get(&http, &url, kind.capability())
                            .await
                            .and_then(|data| pathfinding::parse(&data, kind))
                    };
                    let ground = async {
                        if terrain {
                            Some(
                                get(&http, &terrain_url, "TerrainNavMeshProperties")
                                    .await
                                    .and_then(|data| pathfinding::parse_terrain(&data)),
                            )
                        } else {
                            None
                        }
                    };
                    let (objects, ground) = tokio::join!(objects, ground);
                    combine(Some(objects), ground)
                }
                _ => Err(Error::Unsupported),
            };
            let _ = events.send(NetEvent::Pathfinding {
                request,
                region,
                kind,
                result,
            });
        });
    }

    /// requestSetLinksets: the objects' changes and the terrain's, each to its
    /// own capability, in parallel; nothing sent when nothing changes.
    pub(super) fn set_linksets(&mut self, request: Uuid, region: RegionHandle, objects: Vec<(Uuid, Edit)>, terrain: Option<Edit>) {
        let urls = self
            .region_cap(region, "ObjectNavMeshProperties")
            .zip(self.region_cap(region, "TerrainNavMeshProperties"));
        let events = self.sh.events.clone();
        let reply = move |result| {
            let _ = events.send(NetEvent::LinksetsUpdated { request, region, result });
        };
        let Some((objects_url, terrain_url)) = urls else {
            reply(Err(Error::Unsupported));
            return;
        };
        let objects_body = linkset::objects_body(&objects);
        let terrain_body = terrain.filter(|edit| !edit.is_empty()).map(|edit| edit.to_llsd());
        if objects_body.is_none() && terrain_body.is_none() {
            reply(Ok(Vec::new()));
            return;
        }
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let objects = async {
                match &objects_body {
                    Some(body) => Some(
                        send(&http, &objects_url, body, true, "ObjectNavMeshProperties")
                            .await
                            .and_then(|data| pathfinding::parse(&data, Kind::Objects)),
                    ),
                    None => None,
                }
            };
            let ground = async {
                match &terrain_body {
                    Some(body) => Some(
                        send(&http, &terrain_url, body, true, "TerrainNavMeshProperties")
                            .await
                            .and_then(|data| pathfinding::parse_terrain(&data)),
                    ),
                    None => None,
                }
            };
            let (objects, ground) = tokio::join!(objects, ground);
            reply(combine(objects, ground));
        });
    }

    /// navMeshStatusRequestCoro with isGetStatusOnly: nothing is asked of a
    /// region without pathfinding (the windows read that from its
    /// capabilities).
    pub(super) fn fetch_navmesh_status(&mut self, region: RegionHandle) {
        let Some(url) = self.region_cap(region, "NavMeshGenerationStatus") else {
            return;
        };
        let events = self.sh.events.clone();
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let status = get(&http, &url, "NavMeshGenerationStatus")
                .await
                .ok()
                .and_then(|data| navmesh::Status::parse(&data));
            let _ = events.send(NetEvent::NavMeshStatus { region, status });
        });
    }

    /// navMeshRebakeCoro: `{ "command": "rebuild" }` posted to
    /// NavMeshGenerationStatus.
    pub(super) fn rebake_navmesh(&mut self, region: RegionHandle) {
        let events = self.sh.events.clone();
        let Some(url) = self.region_cap(region, "NavMeshGenerationStatus") else {
            let _ = events.send(NetEvent::NavMeshRebake { region, ok: false });
            return;
        };
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let ok = send(&http, &url, &navmesh::rebake_body(), false, "NavMeshGenerationStatus rebake")
                .await
                .is_ok();
            if !ok {
                log::warn!("pathfinding: navmesh rebake refused or failed");
            }
            let _ = events.send(NetEvent::NavMeshRebake { region, ok });
        });
    }

    /// On arrival in a region, once its capabilities are known
    /// (LLMenuOptionPathfindingRebakeNavmesh::handleRegionBoundaryCrossed):
    /// its navmesh status, and whether the agent may rebake it
    /// (requestGetAgentState; false without pathfinding or on an error).
    pub(super) fn request_pathfinding_state(&mut self) {
        let Some(region) = self.main.and_then(|addr| self.sims.get(&addr)).map(|sim| sim.handle) else {
            return;
        };
        self.fetch_navmesh_status(region);
        let events = self.sh.events.clone();
        let url = self
            .region_cap(region, "NavMeshGenerationStatus")
            .and_then(|_| self.region_cap(region, "AgentState"));
        let Some(url) = url else {
            let _ = events.send(NetEvent::PathfindingAgentState { region, can_rebake: false });
            return;
        };
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let can_rebake = get(&http, &url, "AgentState").await.is_ok_and(|data| navmesh::can_rebake(&data));
            let _ = events.send(NetEvent::PathfindingAgentState { region, can_rebake });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfinding::Detail;
    use crate::pathfinding::linkset::{Linkset, Use};

    fn row(id: u128) -> Row {
        Row {
            id: Uuid::from_u128(id),
            name: String::new(),
            description: String::new(),
            owner: Uuid::nil(),
            group_owned: false,
            position: [0.0; 3],
            detail: Detail::Object {
                land_impact: 1,
                scripted: None,
                linkset: Linkset {
                    use_: Use::Walkable,
                    walkability: [100; 4],
                    modifiable: true,
                    can_be_volume: true,
                    terrain: id == 0,
                },
            },
        }
    }

    #[test]
    fn replies_combine_like_the_linksets_responder() {
        assert_eq!(combine(None, None), Ok(Vec::new()));
        let both = combine(Some(Ok(vec![row(1), row(2)])), Some(Ok(row(0)))).unwrap();
        assert_eq!(both.iter().map(|r| r.id.as_u128()).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(combine(Some(Ok(vec![row(1)])), Some(Err(Error::Failed))), Err(Error::Failed));
        assert_eq!(combine(Some(Err(Error::InvalidReply)), Some(Ok(row(0)))), Err(Error::InvalidReply));
        assert_eq!(combine(None, Some(Ok(row(0)))).unwrap().len(), 1);
    }
}
