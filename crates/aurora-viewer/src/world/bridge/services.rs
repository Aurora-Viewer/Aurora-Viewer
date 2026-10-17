//! Optional read-only bridge services. FSRadar bulk height lookup and
//! handle_script_info, independent of the confirmed movement controller.
use crate::world::radar::Sighting;
use aurora_net::{
    NetCommand, RegionHandle,
    bridge::{
        Endpoint,
        query::{Altitude, MAX_AVATARS, ObjectInfo, Query, Reply, ScriptInfo},
    },
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

const REFRESH: Duration = Duration::from_secs(7);
const CACHE_AGE: Duration = Duration::from_secs(14);
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

struct Height {
    region: RegionHandle,
    z: f32,
    at: Instant,
}
struct RadarRequest {
    request: Uuid,
    avatars: HashMap<Uuid, RegionHandle>,
    at: Instant,
}
struct Inspection {
    query: Query,
    attachment_names: HashMap<u8, String>,
    request: Option<Uuid>,
    at: Instant,
}

#[derive(Default)]
pub struct Services {
    agent: Uuid,
    region: Option<RegionHandle>,
    // In-memory epoch only; never format, log or persist endpoint credentials.
    session: Option<Uuid>,
    heights: HashMap<Uuid, Height>,
    attempts: HashMap<Uuid, Instant>,
    radar: Option<RadarRequest>,
    last_batch: Option<Instant>,
    inspection: Option<Inspection>,
}

impl Services {
    /// Reject late replies after teleport, relog, repair or endpoint rotation.
    /// A queued inspection survives initial provisioning, not a region change.
    pub fn sync(&mut self, agent: Uuid, region: Option<RegionHandle>, endpoint: Option<&Endpoint>) -> Option<String> {
        let session = endpoint.map(|e| e.token);
        let place_changed = self.agent != agent || self.region != region;
        let changed = place_changed || self.session != session;
        let cancel = place_changed || (self.session.is_some() && self.inspection.as_ref().is_some_and(|p| p.request.is_some()) && changed);
        let message = if cancel && self.inspection.take().is_some() {
            Some(aurora_i18n::tr!("bridge-query-interrupted").into())
        } else {
            None
        };
        if changed {
            self.heights.clear();
            self.attempts.clear();
            self.radar = None;
            self.last_batch = None;
        }
        self.agent = agent;
        self.region = region;
        self.session = session;
        message
    }

    pub fn inspect(&mut self, object: Uuid, now: Instant) -> bool {
        self.inspect_query(Query::ScriptInfo(object), now)
    }

    pub fn inspect_object(&mut self, object: Uuid, now: Instant, attachment_names: &HashMap<u8, String>) -> bool {
        if !self.inspect_query(Query::ObjectInfo(object), now) {
            return false;
        }
        if let Some(pending) = &mut self.inspection {
            pending.attachment_names.clone_from(attachment_names);
        }
        true
    }

    fn inspect_query(&mut self, query: Query, now: Instant) -> bool {
        if !query.valid() || self.inspection.as_ref().is_some_and(|p| p.query == query) {
            return false;
        }
        self.inspection = Some(Inspection {
            query,
            attachment_names: HashMap::new(),
            request: None,
            at: now,
        });
        true
    }

    pub fn update(&mut self, endpoint: Option<&Endpoint>, now: Instant) -> (Option<NetCommand>, Option<String>) {
        let Some(pending) = &mut self.inspection else { return (None, None) };
        let timeout = if pending.request.is_some() {
            QUERY_TIMEOUT
        } else {
            Duration::from_secs(60)
        };
        if now.duration_since(pending.at) >= timeout {
            self.inspection = None;
            return (None, Some(aurora_i18n::tr!("bridge-query-timeout").into()));
        }
        if pending.request.is_some() {
            return (None, None);
        }
        let Some(endpoint) = endpoint else { return (None, None) };
        let request = Uuid::new_v4();
        pending.request = Some(request);
        pending.at = now;
        (
            Some(NetCommand::BridgeQuery {
                request,
                endpoint: endpoint.clone(),
                query: pending.query.clone(),
            }),
            None,
        )
    }

    pub fn radar(&mut self, seen: &mut [Sighting], enabled: bool, endpoint: Option<&Endpoint>, now: Instant) -> Option<NetCommand> {
        if !enabled {
            self.heights.clear();
            self.attempts.clear();
            self.radar = None;
            return None;
        }
        let present: HashSet<_> = seen.iter().map(|s| s.id).collect();
        self.heights
            .retain(|id, h| present.contains(id) && now.duration_since(h.at) < CACHE_AGE);
        self.attempts.retain(|id, _| present.contains(id));
        let mut wanted = Vec::new();
        for sighting in seen {
            // Firestorm requests only missing heights. A native or known
            // coarse height wins over both cached and delayed bridge replies.
            if sighting.id.is_nil() || sighting.altitude_known {
                self.heights.remove(&sighting.id);
                continue;
            }
            if self.attempts.get(&sighting.id).is_none_or(|at| now.duration_since(*at) >= REFRESH) {
                wanted.push((sighting.id, sighting.region));
            }
            if let Some(height) = self.heights.get(&sighting.id).filter(|h| h.region == sighting.region) {
                sighting.position.z = height.z;
                sighting.altitude_known = true;
            } else {
                self.heights.remove(&sighting.id);
            }
        }
        if self.radar.as_ref().is_some_and(|p| now.duration_since(p.at) >= QUERY_TIMEOUT) {
            self.radar = None;
        }
        if self.radar.is_some()
            || self.inspection.is_some()
            || self.last_batch.is_some_and(|at| now.duration_since(at) < Duration::from_secs(1))
        {
            return None;
        }
        let endpoint = endpoint?;
        // Never let refreshed entries at the front starve unqueried avatars
        // when the visible crowd spans more than seven batches.
        wanted.sort_unstable_by_key(|(id, _)| self.attempts.get(id).copied());
        wanted.truncate(MAX_AVATARS);
        if wanted.is_empty() {
            return None;
        }
        let request = Uuid::new_v4();
        let ids = wanted.iter().map(|(id, _)| *id).collect();
        for (id, _) in &wanted {
            self.attempts.insert(*id, now);
        }
        self.radar = Some(RadarRequest {
            request,
            avatars: wanted.into_iter().collect(),
            at: now,
        });
        self.last_batch = Some(now);
        Some(NetCommand::BridgeQuery {
            request,
            endpoint: endpoint.clone(),
            query: Query::Altitudes(ids),
        })
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.radar.as_ref().is_some_and(|p| p.request == request) || self.inspection.as_ref().is_some_and(|p| p.request == Some(request))
    }

    pub fn reply(&mut self, request: Uuid, result: Result<Reply, String>, now: Instant) -> Option<String> {
        if self.inspection.as_ref().is_some_and(|p| p.request == Some(request)) {
            let pending = self.inspection.take()?;
            if now.duration_since(pending.at) >= QUERY_TIMEOUT {
                return Some(aurora_i18n::tr!("bridge-query-timeout").into());
            }
            return Some(match (pending.query, result) {
                (Query::ScriptInfo(object), Ok(Reply::ScriptInfo(info))) if info.object == object => script_report(&info),
                (Query::ObjectInfo(object), Ok(Reply::ObjectInfo(info))) if info.scripts.object == object => {
                    object_report(&info, &pending.attachment_names)
                }
                (Query::ObjectInfo(_), Err(error)) => aurora_i18n::tr!("bridge-object-info-failed", reason = error),
                (_, Err(error)) => aurora_i18n::tr!("bridge-script-info-failed", reason = error),
                _ => aurora_i18n::tr!("bridge-request-response").into(),
            });
        }
        if self.radar.as_ref().is_some_and(|p| p.request == request) {
            let pending = self.radar.take()?;
            if now.duration_since(pending.at) >= QUERY_TIMEOUT {
                return None;
            }
            if let Ok(Reply::Altitudes(heights)) = result {
                for Altitude { avatar, z } in heights {
                    if z.is_finite()
                        && z.abs() <= 1_000_000.0
                        && let Some(region) = pending.avatars.get(&avatar)
                    {
                        self.heights.insert(
                            avatar,
                            Height {
                                region: *region,
                                z,
                                at: now,
                            },
                        );
                    }
                }
            }
        }
        None
    }
}

fn script_report(info: &ScriptInfo) -> String {
    let pathfinding = if info.character_seconds > 0.0 {
        aurora_i18n::tr!(
            "bridge-script-info-pathfinding",
            time = aurora_i18n::number(info.character_seconds * 1000.0, 3)
        )
    } else {
        String::new()
    };
    aurora_i18n::tr!(
        "bridge-script-info-result",
        name = info.name.clone(),
        running = info.running.to_string(),
        total = info.total.to_string(),
        memory = aurora_i18n::number(f64::from(info.memory) / 1024.0, 2),
        time = aurora_i18n::number(info.cpu_seconds * 1000.0, 3),
        pathfinding = pathfinding
    )
}

/// FSLSLBridge::lslToViewer's extended report, exposed as its own action.
fn object_report(info: &ObjectInfo, attachment_names: &HashMap<u8, String>) -> String {
    let vector = |values: &[f64]| {
        format!(
            "<{}>",
            values.iter().map(|v| aurora_i18n::number(*v, 3)).collect::<Vec<_>>().join(", ")
        )
    };
    let identity = |id: Uuid, kind: &str| {
        if id.is_nil() {
            "---".into()
        } else {
            format!("secondlife:///app/{kind}/{id}/about")
        }
    };
    let distance = info
        .position
        .iter()
        .zip(info.observer_position)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt();
    let details = aurora_i18n::tr!(
        "bridge-object-info-result",
        name = info.scripts.name.clone(),
        object = info.scripts.object.to_string(),
        description = info.description.clone(),
        root = info.root.to_string(),
        prims = info.prims.to_string(),
        impact = info.land_impact.to_string(),
        inventory = info.inventory.to_string(),
        velocity = vector(&info.velocity),
        position = vector(&info.position),
        distance = aurora_i18n::number(distance, 3),
        rotation = vector(&info.rotation),
        degrees = vector(&info.rotation_degrees),
        angular = vector(&info.angular_velocity),
        creator = identity(info.creator, "agent"),
        owner = if info.owner.is_nil() {
            aurora_i18n::tr!("bridge-object-info-group-owned").to_owned()
        } else {
            identity(info.owner, "agent")
        },
        previous = identity(info.previous_owner, "agent"),
        rezzer = info.rezzer.to_string(),
        group = identity(info.group, "group"),
        created = if info.created.is_empty() {
            "---".into()
        } else {
            info.created.clone()
        },
        rezzed = if info.rezzed.is_empty() {
            "---".into()
        } else {
            info.rezzed.clone()
        },
        pathfinding = info.pathfinding.to_string(),
        attachment = if let Some(name) = attachment_names.get(&info.attachment_point) {
            crate::ui::appearance::items::point_name(name).to_owned()
        } else if info.attachment_point == 0 {
            "---".into()
        } else {
            info.attachment_point.to_string()
        },
        temporary = if info.temporary_attachment {
            aurora_i18n::tr!("common-yes")
        } else {
            aurora_i18n::tr!("common-no")
        }
        .to_owned(),
        observer = vector(&info.observer_position)
    );
    format!("{}\n{details}", script_report(&info.scripts))
}

#[cfg(test)]
mod tests;
